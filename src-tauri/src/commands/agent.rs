use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

// --- Types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub private_key: String,
    pub model: String,
    pub device: String,
    pub oracle_url: String,
    pub chain_rpc: String,
    pub http_port: u16,
    pub grpc_port: u16,
    pub ram_limit_mb: u64,
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
pub struct AgentHealthResponse {
    pub status: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub uptime: f64,
    #[serde(default)]
    pub layers: Option<LayerRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerRange {
    pub start: u32,
    pub end: u32,
    pub total: u32,
}

// --- State ---

pub struct AgentState {
    pub process: Option<CommandChild>,
    pub fallback_pid: Option<u32>,
    pub status: AgentStatus,
    pub http_port: u16,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            process: None,
            fallback_pid: None,
            status: AgentStatus::Stopped,
            http_port: 18920,
        }
    }
}

pub type SharedAgentState = Arc<Mutex<AgentState>>;

// --- Commands ---

#[tauri::command]
pub async fn start_agent(config: AgentConfig, app: AppHandle) -> Result<(), String> {
    let state = app.state::<SharedAgentState>();
    let mut guard = state.lock().await;

    if guard.status == AgentStatus::Running || guard.status == AgentStatus::Starting {
        return Err("Agent is already running or starting".into());
    }

    guard.status = AgentStatus::Starting;
    guard.http_port = config.http_port;

    // Pre-flight validation
    if config.private_key.is_empty() {
        guard.status = AgentStatus::Stopped;
        return Err("Private key is required. Go to Settings to configure it.".into());
    }
    if !config.private_key.starts_with("0x") || config.private_key.len() != 66 {
        guard.status = AgentStatus::Stopped;
        return Err("Invalid private key format. Must be 0x-prefixed hex (66 chars).".into());
    }

    // Build environment variables for the agent process
    let mut envs = vec![
        ("PLUMISE_PRIVATE_KEY", config.private_key.clone()),
        ("MODEL_NAME", config.model.clone()),
        ("DEVICE", config.device.clone()),
        ("ORACLE_API_URL", config.oracle_url.clone()),
        ("PLUMISE_RPC_URL", config.chain_rpc.clone()),
        ("API_PORT", config.http_port.to_string()),
    ];

    // Only set gRPC port if > 0 (0 = disabled for standalone)
    if config.grpc_port > 0 {
        envs.push(("GRPC_PORT", config.grpc_port.to_string()));
    }

    if config.ram_limit_mb > 0 {
        envs.push(("RAM_MB", config.ram_limit_mb.to_string()));
    }

    // Try to use sidecar first, fallback to system PATH for dev mode
    let spawn_result = app
        .shell()
        .sidecar("plumise-agent")
        .and_then(|cmd| {
            let mut cmd = cmd.args(["start"]);
            for (key, val) in &envs {
                cmd = cmd.env(key, val);
            }
            Ok(cmd)
        })
        .and_then(|cmd| cmd.spawn());

    let (mut rx, child) = match spawn_result {
        Ok((rx, child)) => {
            log::info!("Agent spawned via sidecar, PID: {}", child.pid());
            (rx, child)
        }
        Err(e) => {
            log::warn!("Sidecar spawn failed ({}), trying system PATH", e);
            // Fallback to system PATH (dev mode)
            use tokio::process::Command;
            let mut cmd = Command::new("plumise-agent");
            cmd.arg("start");
            for (key, val) in &envs {
                cmd.env(key, val);
            }
            #[cfg(target_os = "windows")]
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            let mut tokio_child = cmd
                .spawn()
                .map_err(|e| format!("Failed to spawn agent process: {}", e))?;

            // Convert tokio child to CommandChild-like behavior
            let pid = tokio_child.id();
            log::info!("Agent spawned via system PATH, PID: {:?}", pid);

            // Capture stdout/stderr for tokio fallback
            let stdout = tokio_child.stdout.take();
            let stderr = tokio_child.stderr.take();
            let app_handle_stdout = app.clone();
            let app_handle_stderr = app.clone();

            if let Some(stdout) = stdout {
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    let reader = BufReader::new(stdout);
                    let mut lines = reader.lines();
                    let mut last_progress_pct: i32 = -1;
                    while let Ok(Some(line)) = lines.next_line().await {
                        // Filter loading progress lines → emit structured event
                        if let Some(progress) = parse_loading_progress(&line) {
                            let pct = progress.percent as i32;
                            if pct != last_progress_pct {
                                last_progress_pct = pct;
                                let _ = app_handle_stdout.emit("agent-loading-progress", progress);
                            }
                            continue;
                        }
                        let level = parse_log_level(&line);
                        let masked_line = mask_sensitive_data(&line);
                        let _ = app_handle_stdout.emit("agent-log", LogEvent {
                            level: level.to_string(),
                            message: masked_line,
                        });
                    }
                });
            }

            if let Some(stderr) = stderr {
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    let reader = BufReader::new(stderr);
                    let mut lines = reader.lines();
                    let mut last_progress_pct: i32 = -1;
                    while let Ok(Some(line)) = lines.next_line().await {
                        // Filter loading progress lines → emit structured event
                        if let Some(progress) = parse_loading_progress(&line) {
                            let pct = progress.percent as i32;
                            if pct != last_progress_pct {
                                last_progress_pct = pct;
                                let _ = app_handle_stderr.emit("agent-loading-progress", progress);
                            }
                            continue;
                        }
                        let level = parse_log_level(&line);
                        let masked_line = mask_sensitive_data(&line);
                        let _ = app_handle_stderr.emit("agent-log", LogEvent {
                            level: level.to_string(),
                            message: masked_line,
                        });
                    }
                });
            }

            // Store the fallback process PID so stop_agent can kill it
            guard.fallback_pid = pid;
            drop(guard);

            // Spawn exit watcher for fallback process
            let state_exit = Arc::clone(&state.inner());
            let app_exit = app.clone();
            tokio::spawn(async move {
                match tokio_child.wait().await {
                    Ok(exit_status) => {
                        log::warn!("Fallback agent process exited: {:?}", exit_status);
                        let mut guard = state_exit.lock().await;
                        if guard.status != AgentStatus::Stopped && guard.status != AgentStatus::Stopping {
                            guard.status = AgentStatus::Error;
                            guard.fallback_pid = None;
                            let _ = app_exit.emit("agent-status", AgentStatusEvent {
                                status: AgentStatus::Error,
                            });
                            let _ = app_exit.emit("agent-log", LogEvent {
                                level: "ERROR".to_string(),
                                message: format!("Agent process exited: {:?}", exit_status),
                            });
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to wait on fallback process: {}", e);
                    }
                }
            });

            // Spawn health polling
            let state_clone = Arc::clone(&state.inner());
            let app_clone = app.clone();
            let http_port = config.http_port;
            tokio::spawn(async move {
                poll_agent_health(state_clone, app_clone, http_port).await;
            });

            return Ok(());
        }
    };

    // Store the sidecar child process
    guard.process = Some(child);
    drop(guard);

    // Spawn a task to handle sidecar events (stdout, stderr, terminated)
    let state_clone = Arc::clone(&state.inner());
    let app_clone = app.clone();
    let http_port = config.http_port;

    tokio::spawn(async move {
        let mut last_progress_pct: i32 = -1;
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    if let Ok(line) = String::from_utf8(bytes) {
                        // Filter loading progress lines → emit structured event
                        if let Some(progress) = parse_loading_progress(&line) {
                            let pct = progress.percent as i32;
                            if pct != last_progress_pct {
                                last_progress_pct = pct;
                                let _ = app_clone.emit("agent-loading-progress", progress);
                            }
                            continue;
                        }
                        let level = parse_log_level(&line);
                        let masked_line = mask_sensitive_data(&line);
                        let _ = app_clone.emit("agent-log", LogEvent {
                            level: level.to_string(),
                            message: masked_line,
                        });
                    }
                }
                CommandEvent::Stderr(bytes) => {
                    if let Ok(line) = String::from_utf8(bytes) {
                        // Filter loading progress lines → emit structured event
                        if let Some(progress) = parse_loading_progress(&line) {
                            let pct = progress.percent as i32;
                            if pct != last_progress_pct {
                                last_progress_pct = pct;
                                let _ = app_clone.emit("agent-loading-progress", progress);
                            }
                            continue;
                        }
                        let level = parse_log_level(&line);
                        let masked_line = mask_sensitive_data(&line);
                        let _ = app_clone.emit("agent-log", LogEvent {
                            level: level.to_string(),
                            message: masked_line,
                        });
                    }
                }
                CommandEvent::Terminated(payload) => {
                    log::warn!("Agent process terminated: {:?}", payload);
                    let mut guard = state_clone.lock().await;
                    guard.status = AgentStatus::Error;
                    guard.process = None;
                    let _ = app_clone.emit("agent-status", AgentStatusEvent {
                        status: AgentStatus::Error,
                    });
                    let _ = app_clone.emit("agent-log", LogEvent {
                        level: "ERROR".to_string(),
                        message: format!("Agent process terminated: code={:?}", payload.code),
                    });
                    break;
                }
                _ => {}
            }
        }
    });

    // Spawn a background task to poll /health and detect readiness
    let state_clone = Arc::clone(&state.inner());
    let app_clone = app.clone();
    tokio::spawn(async move {
        poll_agent_health(state_clone, app_clone, http_port).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn stop_agent(app: AppHandle) -> Result<(), String> {
    let state = app.state::<SharedAgentState>();

    // Phase 1: Read state and mark as stopping, then release the lock
    let http_port;
    {
        let mut guard = state.lock().await;
        if guard.status == AgentStatus::Stopped {
            return Err("Agent is not running".into());
        }
        guard.status = AgentStatus::Stopping;
        http_port = guard.http_port;
    } // lock released here

    // Phase 2: Attempt graceful HTTP shutdown without holding the lock
    let shutdown_url = format!("http://127.0.0.1:{}/shutdown", http_port);
    let client = reqwest::Client::new();
    let graceful = client
        .post(&shutdown_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    if graceful.is_ok() {
        // Wait a bit for graceful shutdown (without holding the lock)
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    } else {
        log::warn!("Graceful shutdown failed, will force kill");
    }

    // Phase 3: Re-acquire lock and force kill any remaining processes
    {
        let mut guard = state.lock().await;
        if let Some(child) = guard.process.take() {
            let _ = child.kill();
        }
        if let Some(pid) = guard.fallback_pid.take() {
            kill_pid(pid);
        }
        guard.process = None;
        guard.fallback_pid = None;
        guard.status = AgentStatus::Stopped;
    } // lock released here

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

// --- Health Polling ---

async fn poll_agent_health(state: SharedAgentState, app: AppHandle, http_port: u16) {
    let client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", http_port);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    let mut ready_detected = false;

    loop {
        interval.tick().await;

        // Check if we should stop polling
        {
            let guard = state.lock().await;
            match guard.status {
                AgentStatus::Stopped | AgentStatus::Stopping | AgentStatus::Error => {
                    log::info!("Stopping health poll (agent stopped/stopping/error)");
                    return;
                }
                _ => {}
            }
        }

        // Poll the health endpoint
        match client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(health) = resp.json::<AgentHealthResponse>().await {
                    if (health.status == "ok" || health.status == "ready") && !ready_detected {
                        ready_detected = true;
                        let mut guard = state.lock().await;
                        guard.status = AgentStatus::Running;
                        let _ = app.emit("agent-status", AgentStatusEvent {
                            status: AgentStatus::Running,
                        });
                        let _ = app.emit("agent-log", LogEvent {
                            level: "INFO".to_string(),
                            message: "Agent is ready and serving requests".to_string(),
                        });
                        log::info!("Agent is ready (model: {})", health.model);
                    }
                }
            }
            _ => {
                // Agent not ready yet, keep polling
            }
        }
    }
}

// --- Pre-flight Check ---

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
pub async fn preflight_check(config: AgentConfig) -> Result<PreflightResult, String> {
    let mut checks = Vec::new();

    // 1. Private key validation
    let pk_valid = config.private_key.starts_with("0x") && config.private_key.len() == 66;
    checks.push(PreflightCheck {
        name: "Wallet".to_string(),
        passed: pk_valid,
        message: if pk_valid {
            "Private key configured".to_string()
        } else {
            "Invalid or missing private key".to_string()
        },
    });

    // 2. Oracle URL reachability — must return HTTP 200 with valid JSON body
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let oracle_check = async {
        let oracle_health_url = format!("{}/api/stats", config.oracle_url.trim_end_matches('/'));
        let resp = client.get(&oracle_health_url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let body = resp.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str::<serde_json::Value>(&body)
            .map_err(|_| "Response is not valid JSON".to_string())?;
        Ok::<(), String>(())
    }.await;
    let oracle_ok = oracle_check.is_ok();
    checks.push(PreflightCheck {
        name: "Oracle".to_string(),
        passed: oracle_ok,
        message: if oracle_ok {
            format!("Connected to {}", config.oracle_url)
        } else {
            format!("Cannot reach {}: {}", config.oracle_url, oracle_check.unwrap_err())
        },
    });

    // 3. Chain RPC reachability — must return HTTP 200 with JSON containing "result" field
    let rpc_check = async {
        let resp = client
            .post(&config.chain_rpc)
            .json(&serde_json::json!({"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if json.get("result").is_none() {
            return Err("RPC response missing 'result' field".to_string());
        }
        Ok::<(), String>(())
    }.await;
    let rpc_ok = rpc_check.is_ok();
    checks.push(PreflightCheck {
        name: "Chain RPC".to_string(),
        passed: rpc_ok,
        message: if rpc_ok {
            format!("Connected to {}", config.chain_rpc)
        } else {
            format!("Cannot reach {}: {}", config.chain_rpc, rpc_check.unwrap_err())
        },
    });

    // 4. Wallet address & balance info (gas is Oracle-sponsored, 0 PLM is OK)
    if pk_valid {
        let balance_result = check_wallet_balance(&client, &config.chain_rpc, &config.private_key).await;
        match balance_result {
            Ok((balance_display, address)) => {
                // Check if balance is effectively zero (all zeros after decimal)
                let is_zero = balance_display.trim_start_matches('0').trim_start_matches('.').is_empty()
                    || balance_display == "0.0000";
                checks.push(PreflightCheck {
                    name: "Wallet".to_string(),
                    passed: true,
                    message: if !is_zero {
                        format!("{}: {} PLM", address, balance_display)
                    } else {
                        format!("{}: 0 PLM (OK — gas is sponsored by Oracle)", address)
                    },
                });
            }
            Err(e) => {
                checks.push(PreflightCheck {
                    name: "Wallet".to_string(),
                    passed: true,
                    message: format!("Address derived, balance check skipped: {}", e),
                });
            }
        }
    }

    // 5. HTTP port availability
    let http_port_free = std::net::TcpListener::bind(format!("127.0.0.1:{}", config.http_port)).is_ok();
    checks.push(PreflightCheck {
        name: "HTTP Port".to_string(),
        passed: http_port_free,
        message: if http_port_free {
            format!("HTTP port {} is available", config.http_port)
        } else {
            format!("HTTP port {} is already in use", config.http_port)
        },
    });

    // 6. gRPC port availability (only if enabled)
    if config.grpc_port > 0 {
        let grpc_port_free = std::net::TcpListener::bind(format!("127.0.0.1:{}", config.grpc_port)).is_ok();
        checks.push(PreflightCheck {
            name: "gRPC Port".to_string(),
            passed: grpc_port_free,
            message: if grpc_port_free {
                format!("gRPC port {} is available", config.grpc_port)
            } else {
                format!("gRPC port {} is already in use", config.grpc_port)
            },
        });
    }

    let passed = checks.iter().all(|c| c.passed);

    Ok(PreflightResult { passed, checks })
}

// --- Helpers ---

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

/// Convert wei (as decimal string) to ETH-like display string using pure string manipulation.
/// Avoids f64 precision loss for large balances.
fn wei_to_display(wei_str: &str) -> String {
    // Pad to at least 19 chars so we can split integer/decimal parts (18 decimals)
    let padded = format!("{:0>19}", wei_str);
    let split_pos = padded.len() - 18;
    let integer_part = &padded[..split_pos];
    let decimal_part = &padded[split_pos..];
    // Trim trailing zeros from decimal, keep at least 4 digits
    let trimmed = decimal_part.trim_end_matches('0');
    let decimal_display = if trimmed.len() < 4 {
        &decimal_part[..4]
    } else {
        trimmed
    };
    format!("{}.{}", integer_part, decimal_display)
}

/// Derive Ethereum address from private key and check on-chain balance
async fn check_wallet_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    private_key: &str,
) -> Result<(String, String), String> {
    // Derive address from private key using keccak256
    let pk_hex = private_key.strip_prefix("0x").unwrap_or(private_key);
    let pk_bytes = hex::decode(pk_hex).map_err(|e| format!("Invalid private key hex: {}", e))?;

    let secret_key = k256::ecdsa::SigningKey::from_bytes((&pk_bytes[..]).into())
        .map_err(|e| format!("Invalid private key: {}", e))?;
    let public_key = secret_key.verifying_key();
    let public_key_bytes = public_key.to_encoded_point(false);
    // Skip the 0x04 prefix byte, hash the remaining 64 bytes
    let hash = sha3_keccak256(&public_key_bytes.as_bytes()[1..]);
    let address = format!("0x{}", hex::encode(&hash[12..]));

    // eth_getBalance RPC call
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_getBalance",
        "params": [&address, "latest"],
        "id": 1
    });

    let resp = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {}", e))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("RPC response parse error: {}", e))?;

    let balance_hex = json["result"]
        .as_str()
        .ok_or("No balance in RPC response")?;

    let balance_hex = balance_hex.strip_prefix("0x").unwrap_or(balance_hex);
    let balance_wei = u128::from_str_radix(balance_hex, 16)
        .map_err(|e| format!("Failed to parse balance hex '{}': {}", balance_hex, e))?;
    let balance_display = wei_to_display(&balance_wei.to_string());

    Ok((balance_display, address))
}

/// Simple Keccak-256 hash (no external crate dependency, uses sha3)
fn sha3_keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut output = [0u8; 32];
    output.copy_from_slice(&result);
    output
}

/// Kill a process by PID (platform-specific)
fn kill_pid(pid: u32) {
    log::info!("Killing fallback process PID: {}", pid);
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/F", "/PID", &pid.to_string()]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let _ = cmd.output();
    }
}

/// Parse loading/download progress from tqdm-style output lines.
/// Returns Some(LoadingProgressEvent) if the line is a progress indicator.
fn parse_loading_progress(line: &str) -> Option<LoadingProgressEvent> {
    let is_loading = line.contains("Loading weights");
    let is_download = line.contains("Downloading");

    if !is_loading && !is_download {
        return None;
    }

    // Find "XX%" pattern - look for '%' and extract the number before it
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
            let phase = if is_download { "downloading" } else { "loading" };
            return Some(LoadingProgressEvent {
                percent: pct,
                phase: phase.to_string(),
            });
        }
    }

    None
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

fn mask_sensitive_data(line: &str) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        // Look for "0x" prefix with at least 64 hex chars following
        if i + 66 <= len && bytes[i] == b'0' && bytes[i + 1] == b'x' {
            let candidate = &line[i + 2..i + 66];
            if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                // Mask: show first 6 chars + last 4 chars
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
