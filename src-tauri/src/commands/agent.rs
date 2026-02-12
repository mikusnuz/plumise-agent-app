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
    pub status: AgentStatus,
    pub http_port: u16,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            process: None,
            status: AgentStatus::Stopped,
            http_port: 8080,
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
    // Desktop app always runs in standalone mode (all layers locally)
    let mut envs = vec![
        ("PRIVATE_KEY", config.private_key.clone()),
        ("MODEL_NAME", config.model.clone()),
        ("DEVICE", config.device.clone()),
        ("ORACLE_URL", config.oracle_url.clone()),
        ("CHAIN_RPC_URL", config.chain_rpc.clone()),
        ("HTTP_PORT", config.http_port.to_string()),
        ("MODE", "single".to_string()), // Force standalone mode
    ];

    // Only set gRPC port if > 0 (0 = disabled for standalone)
    if config.grpc_port > 0 {
        envs.push(("GRPC_PORT", config.grpc_port.to_string()));
    }

    if config.ram_limit_mb > 0 {
        envs.push(("RAM_LIMIT_MB", config.ram_limit_mb.to_string()));
    }

    // Try to use sidecar first, fallback to system PATH for dev mode
    let spawn_result = app
        .shell()
        .sidecar("plumise-agent")
        .and_then(|cmd| {
            let mut cmd = cmd;
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
            for (key, val) in &envs {
                cmd.env(key, val);
            }
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
                    while let Ok(Some(line)) = lines.next_line().await {
                        let level = parse_log_level(&line);
                        let _ = app_handle_stdout.emit("agent-log", LogEvent {
                            level: level.to_string(),
                            message: line,
                        });
                    }
                });
            }

            if let Some(stderr) = stderr {
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    let reader = BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let _ = app_handle_stderr.emit("agent-log", LogEvent {
                            level: "ERROR".to_string(),
                            message: line,
                        });
                    }
                });
            }

            // For fallback mode, we need to wrap tokio::process::Child
            // Since CommandChild is not directly constructible, we'll store the tokio child
            // This means we need to handle this case separately in stop_agent
            // For now, we'll skip sidecar event handling for fallback
            guard.process = None; // Will be handled by health polling
            drop(guard);

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
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    if let Ok(line) = String::from_utf8(bytes) {
                        let level = parse_log_level(&line);
                        let _ = app_clone.emit("agent-log", LogEvent {
                            level: level.to_string(),
                            message: line,
                        });
                    }
                }
                CommandEvent::Stderr(bytes) => {
                    if let Ok(line) = String::from_utf8(bytes) {
                        let _ = app_clone.emit("agent-log", LogEvent {
                            level: "ERROR".to_string(),
                            message: line,
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
    let mut guard = state.lock().await;

    if guard.status == AgentStatus::Stopped {
        return Err("Agent is not running".into());
    }

    guard.status = AgentStatus::Stopping;
    let http_port = guard.http_port;

    // Try graceful shutdown via HTTP first
    let shutdown_url = format!("http://127.0.0.1:{}/shutdown", http_port);
    let client = reqwest::Client::new();
    let graceful = client
        .post(&shutdown_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    if graceful.is_err() {
        // Graceful shutdown failed, kill the process
        log::warn!("Graceful shutdown failed, killing process");
        if let Some(child) = guard.process.take() {
            let _ = child.kill();
        }
    } else {
        // Wait a bit for graceful shutdown
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        // Force kill if still alive
        if let Some(child) = guard.process.take() {
            let _ = child.kill();
        }
    }

    guard.process = None;
    guard.status = AgentStatus::Stopped;

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

    // 2. Oracle URL reachability
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let oracle_ok = client
        .get(&config.oracle_url)
        .send()
        .await
        .is_ok();
    checks.push(PreflightCheck {
        name: "Oracle".to_string(),
        passed: oracle_ok,
        message: if oracle_ok {
            format!("Connected to {}", config.oracle_url)
        } else {
            format!("Cannot reach {}", config.oracle_url)
        },
    });

    // 3. Chain RPC reachability
    let rpc_ok = client
        .post(&config.chain_rpc)
        .json(&serde_json::json!({"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}))
        .send()
        .await
        .is_ok();
    checks.push(PreflightCheck {
        name: "Chain RPC".to_string(),
        passed: rpc_ok,
        message: if rpc_ok {
            format!("Connected to {}", config.chain_rpc)
        } else {
            format!("Cannot reach {}", config.chain_rpc)
        },
    });

    // 4. Wallet address & balance info (gas is Oracle-sponsored, 0 PLM is OK)
    if pk_valid {
        let balance_result = check_wallet_balance(&client, &config.chain_rpc, &config.private_key).await;
        match balance_result {
            Ok((balance_plm, address)) => {
                checks.push(PreflightCheck {
                    name: "Wallet".to_string(),
                    passed: true,
                    message: if balance_plm > 0.0 {
                        format!("{}: {:.4} PLM", address, balance_plm)
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

    // 5. Port availability
    let port_free = std::net::TcpListener::bind(format!("127.0.0.1:{}", config.http_port)).is_ok();
    checks.push(PreflightCheck {
        name: "Port".to_string(),
        passed: port_free,
        message: if port_free {
            format!("Port {} is available", config.http_port)
        } else {
            format!("Port {} is already in use", config.http_port)
        },
    });

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

/// Derive Ethereum address from private key and check on-chain balance
async fn check_wallet_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    private_key: &str,
) -> Result<(f64, String), String> {
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
    let balance_wei = u128::from_str_radix(balance_hex, 16).unwrap_or(0);
    let balance_plm = balance_wei as f64 / 1e18;

    Ok((balance_plm, address))
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
