use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
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
}

fn default_model_file() -> String {
    "gpt-oss-20b-mxfp4.gguf".to_string()
}
fn default_gpu_layers() -> i32 {
    99
}
fn default_ctx_size() -> u32 {
    8192
}
fn default_parallel_slots() -> u32 {
    4
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
    pub process: Option<CommandChild>,
    pub fallback_pid: Option<u32>,
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
            process: None,
            fallback_pid: None,
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

    // Build llama-server arguments
    let args: Vec<String> = vec![
        "-m".into(),
        model_path.to_string_lossy().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        config.http_port.to_string(),
        "-ngl".into(),
        config.gpu_layers.to_string(),
        "--ctx-size".into(),
        config.ctx_size.to_string(),
        "-np".into(),
        config.parallel_slots.to_string(),
        "--jinja".into(),
    ];

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Resolve resource dir for DLL search path (Windows CUDA DLLs)
    let dll_path_env = {
        let mut paths = Vec::new();
        if let Ok(resource_dir) = app.path().resource_dir() {
            let bin_dir = resource_dir.join("binaries");
            if bin_dir.exists() {
                paths.push(bin_dir.to_string_lossy().to_string());
            }
            paths.push(resource_dir.to_string_lossy().to_string());
        }
        if let Ok(sys_path) = std::env::var("PATH") {
            paths.push(sys_path);
        }
        paths.join(if cfg!(windows) { ";" } else { ":" })
    };

    // Try sidecar first, fallback to system PATH
    let spawn_result = app
        .shell()
        .sidecar("llama-server")
        .and_then(|cmd| {
            Ok(cmd
                .args(&args_ref)
                .envs([("PATH".to_string(), dll_path_env.clone())]))
        })
        .and_then(|cmd| cmd.spawn());

    match spawn_result {
        Ok((rx, child)) => {
            log::info!("llama-server spawned via sidecar, PID: {}", child.pid());

            let mut guard = state.lock().await;
            guard.process = Some(child);
            guard.start_time = Some(std::time::Instant::now());
            guard.model_path = Some(model_path.clone());
            guard.agent_address = Some(agent_address.clone());
            drop(guard);

            // Handle sidecar events
            let state_ev = Arc::clone(&state.inner());
            let app_ev = app.clone();
            tokio::spawn(async move {
                handle_sidecar_events(rx, state_ev, app_ev).await;
            });
        }
        Err(e) => {
            log::warn!("Sidecar spawn failed ({}), trying system PATH", e);

            // Fallback: system PATH
            use tokio::process::Command;
            let mut cmd = Command::new("llama-server");
            cmd.args(&args_ref);
            cmd.env("PATH", &dll_path_env);
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
            log::info!("llama-server spawned via PATH, PID: {:?}", pid);

            // Stream stdout/stderr
            for stream in [tokio_child.stdout.take().map(StreamKind::Out), tokio_child.stderr.take().map(StreamKind::Err)] {
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
                guard.fallback_pid = pid;
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
                    if guard.status != AgentStatus::Stopped && guard.status != AgentStatus::Stopping
                    {
                        guard.status = AgentStatus::Error;
                        guard.fallback_pid = None;
                        let _ = app_exit.emit("agent-status", AgentStatusEvent {
                            status: AgentStatus::Error,
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

enum StreamKind {
    Out(tokio::process::ChildStdout),
    Err(tokio::process::ChildStderr),
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
                log::warn!("llama-server terminated: {:?}", payload);
                let mut guard = state.lock().await;
                if guard.status != AgentStatus::Stopped && guard.status != AgentStatus::Stopping {
                    guard.status = AgentStatus::Error;
                    guard.process = None;
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
        if let Some(child) = guard.process.take() {
            let _ = child.kill();
        }
        if let Some(pid) = guard.fallback_pid.take() {
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

/// Called when llama-server health reports "ok" — registers on-chain and with Oracle
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

    // 1. On-chain registration
    let model_hash = chain::crypto::keccak256(config.model.as_bytes());
    let tx_sender =
        chain::tx::TxSender::new(config.chain_rpc.clone(), 41956, signing_key.clone());

    match chain::precompiles::register_agent(&tx_sender, "plumise-agent-app", model_hash).await {
        Ok(hash) => {
            let _ = app.emit("agent-log", LogEvent {
                level: "INFO".to_string(),
                message: format!("On-chain registration: {}", hash),
            });
        }
        Err(e) => {
            log::warn!("On-chain registration failed (non-fatal): {}", e);
            let _ = app.emit("agent-log", LogEvent {
                level: "WARNING".to_string(),
                message: format!("On-chain registration skipped: {}", e),
            });
        }
    }

    // 2. Oracle registration
    let sys = sysinfo::System::new_all();
    let ram_mb = sys.total_memory() / (1024 * 1024);

    if let Err(e) = oracle::registry::register(
        client,
        &config.oracle_url,
        &signing_key,
        &config.model,
        config.http_port,
        ram_mb,
        0,
        &config.device,
    )
    .await
    {
        log::warn!("Oracle registration failed (non-fatal): {}", e);
        let _ = app.emit("agent-log", LogEvent {
            level: "WARNING".to_string(),
            message: format!("Oracle registration skipped: {}", e),
        });
    }

    // 3. Report ready
    if let Err(e) =
        oracle::registry::report_ready(client, &config.oracle_url, &signing_key, &config.model)
            .await
    {
        log::warn!("Oracle ready report failed: {}", e);
    }

    // 4. Spawn background tasks
    let heartbeat_handle = {
        let tx_sender =
            chain::tx::TxSender::new(config.chain_rpc.clone(), 41956, signing_key.clone());
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await;
            loop {
                interval.tick().await;
                match chain::precompiles::heartbeat(&tx_sender).await {
                    Ok(_) => log::debug!("Heartbeat sent"),
                    Err(e) => log::warn!("Heartbeat failed: {}", e),
                }
            }
        })
    };

    let reporter_handle = crate::oracle::reporter::start_reporter(
        client.clone(),
        config.oracle_url.clone(),
        signing_key.clone(),
        config.http_port,
    );

    let mut guard = state.lock().await;
    guard.background_tasks.push(heartbeat_handle);
    guard.background_tasks.push(reporter_handle);
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

    // 5. HTTP port
    let port_free =
        std::net::TcpListener::bind(format!("127.0.0.1:{}", config.http_port)).is_ok();
    checks.push(PreflightCheck {
        name: "HTTP Port".to_string(),
        passed: port_free,
        message: if port_free {
            format!("Port {} available", config.http_port)
        } else {
            format!("Port {} in use", config.http_port)
        },
    });

    // 6. Model
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
