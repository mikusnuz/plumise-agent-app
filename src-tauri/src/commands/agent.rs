use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

use crate::chain;
use crate::oracle;

// ---- Types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub private_key: String,
    pub model: String,
    #[serde(default = "default_model_file")]
    pub model_file: String,
    pub device: String,
    pub oracle_url: String,
    pub chain_rpc: String,
    pub http_port: u16,
    #[serde(default = "default_gpu_layers")]
    pub gpu_layers: i32,
    #[serde(default = "default_ctx_size")]
    pub ctx_size: u32,
    #[serde(default = "default_parallel_slots")]
    pub parallel_slots: u32,
    #[serde(default = "default_ram_limit_gb")]
    pub ram_limit_gb: u32,
    // inference_api_url is auto-derived from oracle_url, no config needed
}

fn default_model_file() -> String {
    "gpt-oss-20b-mxfp4.gguf".to_string()
}
fn default_gpu_layers() -> i32 {
    99
}
fn default_ctx_size() -> u32 {
    32768
}
fn default_parallel_slots() -> u32 {
    1
}
fn default_ram_limit_gb() -> u32 {
    0 // 0 = auto (no limit)
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusResponse {
    pub status: AgentStatus,
    pub http_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMetricsResponse {
    pub status: String,
    pub model: String,
    pub address: String,
    pub uptime: f64,
    pub total_tokens: u64,
    pub total_requests: u64,
    pub tps: f64,
}

// ---- State ----

pub struct AgentState {
    pub pid: Option<u32>,
    pub status: AgentStatus,
    pub http_port: u16,
    pub start_time: Option<std::time::Instant>,
    pub background_tasks: Vec<tokio::task::JoinHandle<()>>,
    pub model_path: Option<PathBuf>,
    pub agent_address: Option<String>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            pid: None,
            status: AgentStatus::Stopped,
            http_port: 18920,
            start_time: None,
            background_tasks: Vec::new(),
            model_path: None,
            agent_address: None,
        }
    }
}

pub type SharedAgentState = Arc<Mutex<AgentState>>;

// ---- Commands ----

#[tauri::command]
pub async fn start_agent(config: AgentConfig, app: AppHandle) -> Result<(), String> {
    let state = app.state::<SharedAgentState>();

    {
        let mut guard = state.lock().await;
        if guard.status == AgentStatus::Running || guard.status == AgentStatus::Starting {
            return Err("Agent is already running or starting".into());
        }
        guard.status = AgentStatus::Starting;
        guard.http_port = config.http_port;
    }

    // Validate private key
    if config.private_key.is_empty() {
        state.lock().await.status = AgentStatus::Stopped;
        return Err("Private key is required. Go to Settings to configure it.".into());
    }
    if !config.private_key.starts_with("0x") || config.private_key.len() != 66 {
        state.lock().await.status = AgentStatus::Stopped;
        return Err("Invalid private key format. Must be 0x-prefixed hex (66 chars).".into());
    }

    // Derive agent address
    let signing_key = match chain::crypto::parse_private_key(&config.private_key) {
        Ok(k) => k,
        Err(e) => {
            state.lock().await.status = AgentStatus::Stopped;
            return Err(e);
        }
    };
    let agent_address = chain::crypto::address_from_key(&signing_key);

    // Resolve model path
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let models_dir = app_data_dir.join("models");

    // Ensure model is downloaded
    let _ = app.emit("agent-log", LogEvent {
        level: "INFO".to_string(),
        message: "Checking model file...".to_string(),
    });

    let model_path = match crate::model::download::ensure_model(
        &models_dir,
        &config.model,
        &config.model_file,
        &app,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            state.lock().await.status = AgentStatus::Stopped;
            return Err(format!("Model download failed: {}", e));
        }
    };

    // Kill any leftover llama-server on our port (e.g. from force-quit)
    if let Some(killed) = kill_process_on_port(config.http_port) {
        log::info!("Killed leftover process on port {}: {}", config.http_port, killed);
        let _ = app.emit("agent-log", LogEvent {
            level: "WARNING".to_string(),
            message: format!("Killed leftover process on port {}: {}", config.http_port, killed),
        });
        // Brief pause so the OS releases the port
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Auto-adjust parallel_slots to ensure each slot gets at least 4096 tokens
    let mut effective_slots = config.parallel_slots;
    if effective_slots > 1 {
        let per_slot = config.ctx_size / effective_slots;
        if per_slot < 4096 {
            effective_slots = (config.ctx_size / 4096).max(1);
            log::warn!(
                "Reduced parallel_slots from {} to {} (per-slot context {} < 4096)",
                config.parallel_slots, effective_slots, per_slot,
            );
            let _ = app.emit("agent-log", LogEvent {
                level: "WARNING".to_string(),
                message: format!(
                    "Auto-adjusted parallel slots: {} → {} (need ≥4096 tokens per slot, ctx={})",
                    config.parallel_slots, effective_slots, config.ctx_size,
                ),
            });
        }
    }

    // Build llama-server arguments
    let mut args: Vec<String> = vec![
        "-m".into(),
        model_path.to_string_lossy().to_string(),
        "--host".into(),
        "0.0.0.0".into(),
        "--port".into(),
        config.http_port.to_string(),
        "-ngl".into(),
        config.gpu_layers.to_string(),
        "--ctx-size".into(),
        config.ctx_size.to_string(),
        "-np".into(),
        effective_slots.to_string(),
        "--jinja".into(),
    ];


    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Resolve DLL/backend directories for llama.cpp
    // DLLs are now bundled as resources in the install root (same dir as sidecar exe).
    // llama-server's default behavior: search for backend DLLs in its own exe directory.
    // Belt-and-suspenders: also set GGML_BACKEND_DIR and PATH.
    let backend_search_dirs = {
        let mut dirs = Vec::new();
        if let Ok(resource_dir) = app.path().resource_dir() {
            dirs.push(resource_dir.to_string_lossy().to_string());
            let bin_dir = resource_dir.join("binaries");
            if bin_dir.is_dir() {
                dirs.push(bin_dir.to_string_lossy().to_string());
            }
        }
        // Also check exe directory
        if let Ok(exe_dir) = std::env::current_exe() {
            if let Some(parent) = exe_dir.parent() {
                let s = parent.to_string_lossy().to_string();
                if !dirs.contains(&s) {
                    dirs.push(s);
                }
            }
        }
        dirs
    };
    let backend_path = backend_search_dirs.join(if cfg!(windows) { ";" } else { ":" });
    let dll_path_env = {
        let mut paths = backend_search_dirs.clone();
        if let Ok(sys_path) = std::env::var("PATH") {
            paths.push(sys_path);
        }
        paths.join(if cfg!(windows) { ";" } else { ":" })
    };

    // Diagnostic: log paths and check for backend DLLs
    log::info!("GGML_BACKEND_DIR={}", backend_path);
    for dir in &backend_search_dirs {
        let path = std::path::Path::new(dir);
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                let dlls: Vec<String> = entries
                    .flatten()
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map(|ext| ext == "dll")
                            .unwrap_or(false)
                    })
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                log::info!("Dir {} has {} DLLs: {:?}", dir, dlls.len(), dlls);
            }
        } else {
            log::warn!("Dir {} does not exist", dir);
        }
    }

    let _ = app.emit("agent-log", LogEvent {
        level: "INFO".to_string(),
        message: format!("Backend search dirs: {:?}", backend_search_dirs),
    });

    // Try sidecar first (Tauri handles import DLL resolution)
    let spawn_result = app
        .shell()
        .sidecar("llama-server")
        .and_then(|cmd| {
            Ok(cmd.args(&args_ref).envs([
                ("PATH".to_string(), dll_path_env.clone()),
                ("GGML_BACKEND_DIR".to_string(), backend_path.clone()),
            ]))
        })
        .and_then(|cmd| cmd.spawn());

    match spawn_result {
        Ok((rx, child)) => {
            let pid = child.pid();
            log::info!("llama-server spawned via sidecar, PID: {}", pid);

            let mut guard = state.lock().await;
            guard.pid = Some(pid);
            guard.start_time = Some(std::time::Instant::now());
            guard.model_path = Some(model_path.clone());
            guard.agent_address = Some(agent_address.clone());
            drop(guard);

            // Handle sidecar events (stdout/stderr + termination)
            let state_ev = Arc::clone(&state.inner());
            let app_ev = app.clone();
            tokio::spawn(async move {
                handle_sidecar_events(rx, state_ev, app_ev).await;
            });
        }
        Err(e) => {
            log::warn!("Sidecar spawn failed ({}), trying system PATH", e);

            // Fallback: direct spawn from resource dir or PATH
            let (exe_path, exe_dir) = find_llama_server(&app)?;
            log::info!("Fallback binary: {}", exe_path.display());

            use tokio::process::Command;
            let mut cmd = Command::new(&exe_path);
            cmd.args(&args_ref);
            cmd.current_dir(&exe_dir);
            cmd.env("PATH", &dll_path_env);
            cmd.env("GGML_BACKEND_DIR", &exe_dir);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            let mut tokio_child = cmd
                .spawn()
                .map_err(|e| format!("Failed to spawn llama-server: {}", e))?;

            let pid = tokio_child.id();
            log::info!("llama-server spawned via fallback, PID: {:?}", pid);

            // Stream stdout/stderr
            for stream in [
                tokio_child.stdout.take().map(StreamKind::Out),
                tokio_child.stderr.take().map(StreamKind::Err),
            ] {
                if let Some(kind) = stream {
                    let app_h = app.clone();
                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, BufReader};
                        let mut last_pct: i32 = -1;
                        match kind {
                            StreamKind::Out(s) => {
                                let mut lines = BufReader::new(s).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    handle_log_line(&line, &app_h, &mut last_pct);
                                }
                            }
                            StreamKind::Err(s) => {
                                let mut lines = BufReader::new(s).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    handle_log_line(&line, &app_h, &mut last_pct);
                                }
                            }
                        }
                    });
                }
            }

            {
                let mut guard = state.lock().await;
                guard.pid = pid;
                guard.start_time = Some(std::time::Instant::now());
                guard.model_path = Some(model_path.clone());
                guard.agent_address = Some(agent_address.clone());
            }

            // Exit watcher
            let state_exit = Arc::clone(&state.inner());
            let app_exit = app.clone();
            tokio::spawn(async move {
                if let Ok(exit_status) = tokio_child.wait().await {
                    log::warn!("llama-server exited: {:?}", exit_status);
                    let mut guard = state_exit.lock().await;
                    if guard.status != AgentStatus::Stopped
                        && guard.status != AgentStatus::Stopping
                    {
                        guard.status = AgentStatus::Error;
                        guard.pid = None;
                        let _ = app_exit.emit("agent-status", AgentStatusEvent {
                            status: AgentStatus::Error,
                        });
                        let _ = app_exit.emit("agent-log", LogEvent {
                            level: "ERROR".to_string(),
                            message: describe_exit_code(exit_status.code()),
                        });
                    }
                }
            });
        }
    }

    // Spawn health polling (triggers chain/oracle registration when ready)
    let state_poll = Arc::clone(&state.inner());
    let app_poll = app.clone();
    tokio::spawn(async move {
        poll_agent_health(state_poll, app_poll, config).await;
    });

    Ok(())
}

async fn handle_sidecar_events(
    mut rx: tauri::async_runtime::Receiver<CommandEvent>,
    state: SharedAgentState,
    app: AppHandle,
) {
    let mut last_pct: i32 = -1;
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                if let Ok(line) = String::from_utf8(bytes) {
                    handle_log_line(&line, &app, &mut last_pct);
                }
            }
            CommandEvent::Terminated(payload) => {
                log::warn!("llama-server terminated: code={:?}", payload.code);
                let mut guard = state.lock().await;
                if guard.status != AgentStatus::Stopped && guard.status != AgentStatus::Stopping {
                    guard.status = AgentStatus::Error;
                    guard.pid = None;
                    let _ = app.emit("agent-status", AgentStatusEvent {
                        status: AgentStatus::Error,
                    });
                    let _ = app.emit("agent-log", LogEvent {
                        level: "ERROR".to_string(),
                        message: describe_exit_code(payload.code),
                    });
                }
                break;
            }
            _ => {}
        }
    }
}

enum StreamKind {
    Out(tokio::process::ChildStdout),
    Err(tokio::process::ChildStderr),
}

#[tauri::command]
pub async fn stop_agent(app: AppHandle) -> Result<(), String> {
    let state = app.state::<SharedAgentState>();

    {
        let mut guard = state.lock().await;
        if guard.status == AgentStatus::Stopped {
            return Err("Agent is not running".into());
        }
        guard.status = AgentStatus::Stopping;

        // Abort background tasks
        for task in guard.background_tasks.drain(..) {
            task.abort();
        }
    }

    // Force kill process
    {
        let mut guard = state.lock().await;
        if let Some(pid) = guard.pid.take() {
            kill_pid(pid);
        }
        guard.status = AgentStatus::Stopped;
        guard.start_time = None;
    }

    let _ = app.emit("agent-status", AgentStatusEvent {
        status: AgentStatus::Stopped,
    });

    log::info!("Agent stopped");
    Ok(())
}

#[tauri::command]
pub async fn get_agent_status(
    state: tauri::State<'_, SharedAgentState>,
) -> Result<AgentStatusResponse, String> {
    let guard = state.lock().await;
    Ok(AgentStatusResponse {
        status: guard.status.clone(),
        http_port: guard.http_port,
    })
}

#[tauri::command]
pub async fn get_agent_metrics(
    state: tauri::State<'_, SharedAgentState>,
) -> Result<AgentMetricsResponse, String> {
    let (http_port, model_path, agent_address, uptime, status) = {
        let guard = state.lock().await;
        (
            guard.http_port,
            guard.model_path.clone(),
            guard.agent_address.clone().unwrap_or_default(),
            guard.start_time.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0),
            guard.status.clone(),
        )
    };

    if status != AgentStatus::Running {
        return Ok(AgentMetricsResponse {
            status: "stopped".to_string(),
            model: String::new(),
            address: agent_address,
            uptime: 0.0,
            total_tokens: 0,
            total_requests: 0,
            tps: 0.0,
        });
    }

    let client = reqwest::Client::new();
    let metrics = crate::inference::metrics::fetch_metrics(&client, http_port)
        .await
        .unwrap_or_default();

    let model_name = model_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(AgentMetricsResponse {
        status: "ok".to_string(),
        model: model_name,
        address: agent_address,
        uptime,
        total_tokens: metrics.total_tokens,
        total_requests: metrics.total_requests,
        tps: metrics.tps,
    })
}

#[tauri::command]
pub async fn check_model(config: AgentConfig, app: AppHandle) -> Result<bool, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let model_path = app_data_dir.join("models").join(&config.model_file);
    Ok(model_path.exists()
        && std::fs::metadata(&model_path)
            .map(|m| m.len() > 100 * 1024 * 1024)
            .unwrap_or(false))
}

// ---- Health Polling ----

async fn poll_agent_health(state: SharedAgentState, app: AppHandle, config: AgentConfig) {
    let client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", config.http_port);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    let mut ready_detected = false;

    loop {
        interval.tick().await;

        {
            let guard = state.lock().await;
            match guard.status {
                AgentStatus::Stopped | AgentStatus::Stopping | AgentStatus::Error => {
                    log::info!("Stopping health poll");
                    return;
                }
                _ => {}
            }
        }

        // llama-server: 200 when ready, 503 when loading
        let resp = match client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(r) => r,
            _ => continue,
        };

        let body: serde_json::Value = match resp.json().await {
            Ok(b) => b,
            _ => continue,
        };

        let llama_status = body["status"].as_str().unwrap_or("");

        // Emit loading progress
        if llama_status == "loading model" {
            if let Some(progress) = body["progress"].as_f64() {
                let _ = app.emit(
                    "agent-loading-progress",
                    LoadingProgressEvent {
                        percent: (progress * 100.0) as f32,
                        phase: "loading".to_string(),
                    },
                );
            }
        }

        // Detect readiness
        if llama_status == "ok" && !ready_detected {
            ready_detected = true;
            {
                let mut guard = state.lock().await;
                guard.status = AgentStatus::Running;
            }
            let _ = app.emit("agent-status", AgentStatusEvent {
                status: AgentStatus::Running,
            });
            let _ = app.emit("agent-log", LogEvent {
                level: "INFO".to_string(),
                message: "llama-server is ready and serving requests".to_string(),
            });
            log::info!("llama-server is ready");

            on_agent_ready(&state, &app, &config, &client).await;
        }
    }
}

/// Called when llama-server health reports "ok" — registers with Oracle (which handles on-chain registration)
async fn on_agent_ready(
    state: &SharedAgentState,
    app: &AppHandle,
    config: &AgentConfig,
    client: &reqwest::Client,
) {
    let signing_key = match chain::crypto::parse_private_key(&config.private_key) {
        Ok(k) => k,
        Err(e) => {
            log::error!("Private key parse error: {}", e);
            return;
        }
    };

    // On-chain registration and heartbeats are handled by Oracle via sponsor flow.
    // Agent wallet does not need PLM balance.

    // Detect LAN IP for external access (Oracle/inference API need to reach us)
    let local_ip = get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    log::info!("Detected local IP: {}", local_ip);
    let _ = app.emit("agent-log", LogEvent {
        level: "INFO".to_string(),
        message: format!("Local endpoint: http://{}:{}", local_ip, config.http_port),
    });

    // Oracle model name must match what inference API queries (openai/gpt-oss-20b)
    // config.model is the HuggingFace repo for download (ggml-org/gpt-oss-20b-GGUF)
    let oracle_model = "openai/gpt-oss-20b";

    // 1. Oracle registration (Oracle will sponsor on-chain registration if needed)
    let ram_mb = if config.ram_limit_gb > 0 {
        (config.ram_limit_gb as u64) * 1024
    } else {
        let sys = sysinfo::System::new_all();
        sys.total_memory() / (1024 * 1024)
    };

    // Run benchmark
    let benchmark_tok_per_sec = match crate::inference::benchmark::run_benchmark(client, config.http_port).await {
        Ok(tps) => {
            log::info!("Benchmark result: {:.2} tok/s", tps);
            let _ = app.emit("agent-log", format!("Benchmark: {:.2} tok/s", tps));
            tps
        }
        Err(e) => {
            log::warn!("Benchmark failed (using default): {}", e);
            let _ = app.emit("agent-log", format!("Benchmark skipped: {}", e));
            0.0
        }
    };

    match oracle::registry::register(
        client,
        &config.oracle_url,
        &signing_key,
        oracle_model,
        config.http_port,
        ram_mb,
        0,
        &config.device,
        &local_ip,
        benchmark_tok_per_sec,
    )
    .await
    {
        Ok(()) => {
            let _ = app.emit("agent-log", LogEvent {
                level: "INFO".to_string(),
                message: "Registered with Oracle as standalone node".to_string(),
            });
        }
        Err(e) => {
            log::warn!("Oracle registration failed (non-fatal): {}", e);
            let _ = app.emit("agent-log", LogEvent {
                level: "WARNING".to_string(),
                message: format!("Oracle registration failed: {}", e),
            });
        }
    }

    // 2. Spawn background tasks (metrics reporter + periodic re-registration)
    let reporter_handle = crate::oracle::reporter::start_reporter(
        client.clone(),
        config.oracle_url.clone(),
        signing_key.clone(),
        config.http_port,
        crate::oracle::reporter::RegistrationParams {
            model: oracle_model.to_string(),
            http_port: config.http_port,
            ram_mb,
            vram_mb: 0,
            device: config.device.clone(),
            external_ip: local_ip.clone(),
            benchmark_tok_per_sec,
        },
    );

    let mut guard = state.lock().await;
    guard.background_tasks.push(reporter_handle);

    // 4. Start WebSocket relay — auto-derive from oracle URL
    //    e.g. https://plug.plumise.com/oracle → wss://plug.plumise.com/ws/agent-relay
    let relay_base = config.oracle_url.trim_end_matches('/');
    let relay_base = if relay_base.ends_with("/oracle") {
        relay_base.trim_end_matches("/oracle")
    } else {
        relay_base
    };
    let ws_base = relay_base
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let ws_url = format!("{}/ws/agent-relay", ws_base);

    let _ = app.emit("agent-log", LogEvent {
        level: "INFO".to_string(),
        message: format!("Connecting to inference relay: {}", ws_url),
    });

    let relay_handle = crate::relay::client::start_relay(
        ws_url,
        signing_key.clone(),
        oracle_model.to_string(),
        config.http_port,
    );
    guard.background_tasks.push(relay_handle);
}

// ---- Pre-flight Check ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightResult {
    pub passed: bool,
    pub checks: Vec<PreflightCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

#[tauri::command]
pub async fn preflight_check(
    config: AgentConfig,
    app: AppHandle,
) -> Result<PreflightResult, String> {
    let mut checks = Vec::new();

    // 1. Private key
    let pk_valid = config.private_key.starts_with("0x") && config.private_key.len() == 66;
    checks.push(PreflightCheck {
        name: "Wallet".to_string(),
        passed: pk_valid,
        message: if pk_valid {
            "Private key configured".into()
        } else {
            "Invalid or missing private key".into()
        },
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    // 2. Oracle
    let oracle_ok = client
        .get(format!(
            "{}/api/stats",
            config.oracle_url.trim_end_matches('/')
        ))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    checks.push(PreflightCheck {
        name: "Oracle".to_string(),
        passed: oracle_ok,
        message: if oracle_ok {
            format!("Connected to {}", config.oracle_url)
        } else {
            format!("Cannot reach {}", config.oracle_url)
        },
    });

    // 3. Chain RPC
    let rpc_ok = client
        .post(&config.chain_rpc)
        .json(&serde_json::json!({"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}))
        .send()
        .await
        .ok()
        .and_then(|r| if r.status().is_success() { Some(()) } else { None })
        .is_some();
    checks.push(PreflightCheck {
        name: "Chain RPC".to_string(),
        passed: rpc_ok,
        message: if rpc_ok {
            format!("Connected to {}", config.chain_rpc)
        } else {
            format!("Cannot reach {}", config.chain_rpc)
        },
    });

    // 4. Wallet balance
    if pk_valid {
        match check_wallet_balance(&client, &config.chain_rpc, &config.private_key).await {
            Ok((balance, addr)) => {
                let is_zero = balance == "0.0000"
                    || balance
                        .trim_start_matches('0')
                        .trim_start_matches('.')
                        .is_empty();
                checks.push(PreflightCheck {
                    name: "Wallet".to_string(),
                    passed: true,
                    message: if is_zero {
                        format!("{}: 0 PLM (OK — gas is sponsored)", addr)
                    } else {
                        format!("{}: {} PLM", addr, balance)
                    },
                });
            }
            Err(e) => {
                checks.push(PreflightCheck {
                    name: "Wallet".to_string(),
                    passed: true,
                    message: format!("Balance check skipped: {}", e),
                });
            }
        }
    }

    // 5. HTTP port — auto-kill leftover llama-server if port is occupied
    let mut port_free =
        std::net::TcpListener::bind(format!("127.0.0.1:{}", config.http_port)).is_ok();
    let port_message = if port_free {
        format!("Port {} available", config.http_port)
    } else {
        // Try to kill leftover process (likely a previous llama-server)
        if let Some(killed) = kill_process_on_port(config.http_port) {
            // Brief wait for OS to release the port
            std::thread::sleep(std::time::Duration::from_millis(500));
            port_free =
                std::net::TcpListener::bind(format!("127.0.0.1:{}", config.http_port)).is_ok();
            if port_free {
                format!("Port {} freed (killed leftover: {})", config.http_port, killed)
            } else {
                format!("Port {} still in use after killing {}", config.http_port, killed)
            }
        } else {
            format!("Port {} in use by unknown process", config.http_port)
        }
    };
    checks.push(PreflightCheck {
        name: "HTTP Port".to_string(),
        passed: port_free,
        message: port_message,
    });

    // 6. GPU detection (cross-platform)
    if config.gpu_layers > 0 {
        let gpu_info = detect_gpu();
        match gpu_info {
            Some((name, vram_mb)) => {
                let detail = if vram_mb > 0 {
                    format!("{} ({} MB)", name, vram_mb)
                } else {
                    name
                };
                checks.push(PreflightCheck {
                    name: "GPU".to_string(),
                    passed: true,
                    message: detail,
                });
            }
            None => {
                checks.push(PreflightCheck {
                    name: "GPU".to_string(),
                    passed: false,
                    message: "No GPU detected. Install GPU drivers or set GPU Layers to 0 for CPU mode.".into(),
                });
            }
        }
    }

    // 7. Model
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("App data dir: {}", e))?;
    let model_path = app_data_dir.join("models").join(&config.model_file);
    let model_exists = model_path.exists()
        && std::fs::metadata(&model_path)
            .map(|m| m.len() > 100 * 1024 * 1024)
            .unwrap_or(false);
    checks.push(PreflightCheck {
        name: "Model".to_string(),
        passed: true,
        message: if model_exists {
            let gb = std::fs::metadata(&model_path)
                .map(|m| m.len() as f64 / 1e9)
                .unwrap_or(0.0);
            format!("{} ({:.1} GB)", config.model_file, gb)
        } else {
            format!("{} (will download on start)", config.model_file)
        },
    });

    let passed = checks.iter().all(|c| c.passed);
    Ok(PreflightResult { passed, checks })
}

// ---- Helpers ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogEvent {
    level: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentStatusEvent {
    status: AgentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadingProgressEvent {
    percent: f32,
    phase: String,
}

fn handle_log_line(line: &str, app: &AppHandle, last_progress_pct: &mut i32) {
    if let Some(progress) = parse_loading_progress(line) {
        let pct = progress.percent as i32;
        if pct != *last_progress_pct {
            *last_progress_pct = pct;
            let _ = app.emit("agent-loading-progress", progress);
        }
        return;
    }
    let level = parse_log_level(line);
    let masked = mask_sensitive_data(line);
    let _ = app.emit("agent-log", LogEvent {
        level: level.to_string(),
        message: masked,
    });
}

fn parse_loading_progress(line: &str) -> Option<LoadingProgressEvent> {
    // llama-server: "llm_load_progress: 0.42"
    if let Some(idx) = line.find("llm_load_progress:") {
        let after = line[idx + 18..].trim();
        if let Ok(frac) = after.parse::<f64>() {
            return Some(LoadingProgressEvent {
                percent: (frac * 100.0) as f32,
                phase: "loading".to_string(),
            });
        }
    }
    // tqdm-style
    if line.contains("Loading") || line.contains("Downloading") {
        if let Some(pct_idx) = line.find('%') {
            let before = &line[..pct_idx];
            let num_str: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if let Ok(pct) = num_str.parse::<f32>() {
                let phase = if line.contains("Download") {
                    "downloading"
                } else {
                    "loading"
                };
                return Some(LoadingProgressEvent {
                    percent: pct,
                    phase: phase.to_string(),
                });
            }
        }
    }
    None
}

fn wei_to_display(wei_str: &str) -> String {
    let padded = format!("{:0>19}", wei_str);
    let split_pos = padded.len() - 18;
    let integer_part = &padded[..split_pos];
    let decimal_part = &padded[split_pos..];
    let trimmed = decimal_part.trim_end_matches('0');
    let decimal_display = if trimmed.len() < 4 {
        &decimal_part[..4]
    } else {
        trimmed
    };
    format!("{}.{}", integer_part, decimal_display)
}

async fn check_wallet_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    private_key: &str,
) -> Result<(String, String), String> {
    let signing_key = chain::crypto::parse_private_key(private_key)?;
    let address = chain::crypto::address_from_key(&signing_key);

    let resp = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBalance",
            "params": [&address, "latest"],
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| format!("RPC failed: {}", e))?;

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
    let balance_hex = json["result"].as_str().ok_or("No balance")?;
    let balance_hex = balance_hex.strip_prefix("0x").unwrap_or(balance_hex);
    let balance_wei =
        u128::from_str_radix(balance_hex, 16).map_err(|e| format!("Invalid balance: {}", e))?;
    Ok((wei_to_display(&balance_wei.to_string()), address))
}

fn kill_pid(pid: u32) {
    log::info!("Killing process PID: {}", pid);
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/F", "/PID", &pid.to_string()]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
        let _ = cmd.output();
    }
}

fn parse_log_level(line: &str) -> &str {
    let upper = line.to_uppercase();
    if upper.contains("ERROR") || upper.contains("CRITICAL") || upper.contains("FATAL") {
        "ERROR"
    } else if upper.contains("WARNING") || upper.contains("WARN") {
        "WARNING"
    } else if upper.contains("DEBUG") || upper.contains("TRACE") {
        "DEBUG"
    } else {
        "INFO"
    }
}

/// Cross-platform GPU detection.
fn detect_gpu() -> Option<(String, u64)> {
    #[cfg(target_os = "macos")]
    return detect_metal_gpu();

    #[cfg(not(target_os = "macos"))]
    return detect_nvidia_gpu();
}

/// Detect Metal GPU on macOS (Apple Silicon or discrete).
#[cfg(target_os = "macos")]
fn detect_metal_gpu() -> Option<(String, u64)> {
    let output = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        // Apple Silicon always has Metal — return generic info
        return Some(("Apple GPU (Metal)".to_string(), 0));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let name = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("Chipset Model:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Apple GPU".to_string());

    // Apple Silicon uses unified memory — report total system RAM
    let sys = sysinfo::System::new_all();
    let unified_ram_mb = sys.total_memory() / (1024 * 1024);

    Some((format!("{} (Metal)", name), unified_ram_mb))
}

/// Find the llama-server binary. Returns (exe_path, exe_directory).
/// Search order:
///   1. Tauri resource dir / binaries/ (installed MSI)
///   2. Tauri resource dir / (flat resource layout)
///   3. System PATH
fn find_llama_server(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let exe_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    // Search in Tauri resource directories
    if let Ok(resource_dir) = app.path().resource_dir() {
        // 1. Check binaries/ subdirectory (preserves externalBin structure)
        let bin_dir = resource_dir.join("binaries");
        if bin_dir.is_dir() {
            // Look for exact name or triple-suffixed name
            if let Ok(entries) = std::fs::read_dir(&bin_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("llama-server") && !name.ends_with(".dll") {
                        let path = entry.path();
                        if path.is_file() {
                            log::info!("Found sidecar in binaries/: {}", path.display());
                            return Ok((path, bin_dir));
                        }
                    }
                }
            }
        }

        // 2. Check resource dir root (flat layout)
        if let Ok(entries) = std::fs::read_dir(&resource_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("llama-server") && !name.ends_with(".dll") {
                    let path = entry.path();
                    if path.is_file() {
                        log::info!("Found sidecar in resource root: {}", path.display());
                        return Ok((path, resource_dir));
                    }
                }
            }
        }

        log::warn!(
            "llama-server not found in resource dir: {}",
            resource_dir.display()
        );
    }

    // 3. Fallback: system PATH
    let which_cmd = if cfg!(windows) { "where" } else { "which" };
    if let Ok(output) = std::process::Command::new(which_cmd)
        .arg(exe_name)
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !path_str.is_empty() {
                let path = PathBuf::from(&path_str);
                let dir = path.parent().unwrap_or(&path).to_path_buf();
                log::info!("Found llama-server in PATH: {}", path.display());
                return Ok((path, dir));
            }
        }
    }

    Err(
        "llama-server not found. Reinstall the app or add llama-server to your system PATH."
            .into(),
    )
}

#[cfg(not(target_os = "macos"))]
fn detect_nvidia_gpu() -> Option<(String, u64)> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?.trim().to_string();
    // Format: "NVIDIA GeForce RTX 4090, 24564"
    let mut parts = line.splitn(2, ',');
    let name = parts.next()?.trim().to_string();
    let vram: u64 = parts.next()?.trim().parse().unwrap_or(0);

    if name.is_empty() {
        return None;
    }
    Some((name, vram))
}

fn describe_exit_code(code: Option<i32>) -> String {
    match code {
        // 0xC0000135 = STATUS_DLL_NOT_FOUND (Windows)
        Some(-1073741515) => {
            "llama-server failed: Required DLL not found. \
             Please install NVIDIA CUDA Toolkit 12.x or ensure CUDA drivers are up to date."
                .to_string()
        }
        // 0xC0000005 = ACCESS_VIOLATION
        Some(-1073741819) => {
            "llama-server crashed: Access violation. \
             Try reducing GPU layers (gpu_layers=0 for CPU-only mode)."
                .to_string()
        }
        Some(c) => format!("llama-server exited with code {}", c),
        None => "llama-server was terminated by signal".to_string(),
    }
}

/// Kill any process listening on the given port. Returns a description of what was killed.
/// Only kills llama-server processes to avoid accidentally killing unrelated services.
fn kill_process_on_port(port: u16) -> Option<String> {
    #[cfg(unix)]
    {
        // lsof -ti :<port> returns PIDs listening on the port
        let output = std::process::Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
            .ok()?;

        let pids_str = String::from_utf8_lossy(&output.stdout);
        let pids: Vec<&str> = pids_str.trim().lines().collect();
        if pids.is_empty() {
            return None;
        }

        let mut killed = Vec::new();
        for pid in &pids {
            let pid = pid.trim();
            if pid.is_empty() {
                continue;
            }
            // Check process name before killing — only kill llama-server
            let ps_out = std::process::Command::new("ps")
                .args(["-p", pid, "-o", "comm="])
                .output()
                .ok();
            let proc_name = ps_out
                .as_ref()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();

            if proc_name.contains("llama-server") || proc_name.contains("llama_server") {
                let _ = std::process::Command::new("kill")
                    .args(["-9", pid])
                    .output();
                killed.push(format!("PID {} ({})", pid, proc_name));
            } else {
                log::warn!(
                    "Port {} held by non-llama process: PID {} ({}), skipping",
                    port, pid, proc_name
                );
            }
        }

        if killed.is_empty() {
            None
        } else {
            Some(killed.join(", "))
        }
    }

    #[cfg(windows)]
    {
        // netstat -ano | findstr :<port>
        let output = std::process::Command::new("cmd")
            .args(["/C", &format!("netstat -ano | findstr :{}", port)])
            .output()
            .ok()?;

        let text = String::from_utf8_lossy(&output.stdout);
        let mut killed = Vec::new();

        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 && parts[1].contains(&format!(":{}", port)) {
                let pid = parts[4];
                // Check if it's llama-server before killing
                let tasklist = std::process::Command::new("tasklist")
                    .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
                    .output()
                    .ok();
                let proc_name = tasklist
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();

                if proc_name.to_lowercase().contains("llama-server") || proc_name.to_lowercase().contains("llama_server") {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/PID", pid])
                        .output();
                    killed.push(format!("PID {}", pid));
                } else {
                    log::warn!("Port {} held by non-llama process: PID {}, skipping", port, pid);
                }
            }
        }

        if killed.is_empty() {
            None
        } else {
            Some(killed.join(", "))
        }
    }
}

/// Discover the machine's LAN IP by connecting a UDP socket to a remote address.
/// The OS picks the correct local interface without actually sending data.
fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // Connect to a public IP — no data is sent, just determines the route
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip().to_string();
    // Don't return loopback
    if ip == "127.0.0.1" || ip == "::1" {
        return None;
    }
    Some(ip)
}

fn mask_sensitive_data(line: &str) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        if i + 66 <= len && bytes[i] == b'0' && bytes[i + 1] == b'x' {
            let candidate = &line[i + 2..i + 66];
            if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                result.push_str(&line[i..i + 6]);
                result.push_str("****...****");
                result.push_str(&line[i + 62..i + 66]);
                i += 66;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}
