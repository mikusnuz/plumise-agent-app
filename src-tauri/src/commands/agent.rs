use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
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
    pub process: Option<Child>,
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
    let mut cmd = Command::new("plumise-agent");
    cmd.env("PRIVATE_KEY", &config.private_key)
        .env("MODEL_NAME", &config.model)
        .env("DEVICE", &config.device)
        .env("ORACLE_URL", &config.oracle_url)
        .env("CHAIN_RPC_URL", &config.chain_rpc)
        .env("HTTP_PORT", config.http_port.to_string())
        .env("MODE", "single"); // Force standalone mode

    // Only set gRPC port if > 0 (0 = disabled for standalone)
    if config.grpc_port > 0 {
        cmd.env("GRPC_PORT", config.grpc_port.to_string());
    }

    if config.ram_limit_mb > 0 {
        cmd.env("RAM_LIMIT_MB", config.ram_limit_mb.to_string());
    }

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn agent process: {}", e))?;

    // Capture stdout and emit log events
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let app_handle_stdout = app.clone();
    let app_handle_stderr = app.clone();

    if let Some(stdout) = stdout {
        tokio::spawn(async move {
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

    // Store the child process
    let pid = child.id();
    guard.process = Some(child);
    drop(guard);

    log::info!("Agent process spawned with PID: {:?}", pid);

    // Spawn a background task to poll /health and detect readiness/crash
    let state_clone = Arc::clone(&state.inner());
    let app_clone = app.clone();
    let http_port = config.http_port;

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
        if let Some(ref mut child) = guard.process {
            let _ = child.kill().await;
        }
    }

    // Wait for process to exit (with timeout)
    if let Some(ref mut child) = guard.process {
        let wait_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            child.wait(),
        )
        .await;

        if wait_result.is_err() {
            log::warn!("Process did not exit within timeout, force killing");
            let _ = child.kill().await;
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
                AgentStatus::Stopped | AgentStatus::Stopping => {
                    log::info!("Stopping health poll (agent stopped/stopping)");
                    return;
                }
                _ => {}
            }
        }

        // Check if the process is still alive
        {
            let mut guard = state.lock().await;
            if let Some(ref mut child) = guard.process {
                match child.try_wait() {
                    Ok(Some(exit_status)) => {
                        // Process has exited
                        log::warn!("Agent process exited with status: {}", exit_status);
                        guard.status = AgentStatus::Error;
                        guard.process = None;
                        let _ = app.emit("agent-status", AgentStatusEvent {
                            status: AgentStatus::Error,
                        });
                        let _ = app.emit("agent-log", LogEvent {
                            level: "ERROR".to_string(),
                            message: format!("Agent process exited with status: {}", exit_status),
                        });
                        return;
                    }
                    Ok(None) => {
                        // Still running
                    }
                    Err(e) => {
                        log::error!("Failed to check process status: {}", e);
                    }
                }
            } else if guard.status != AgentStatus::Stopped {
                // Process handle is gone but status isn't stopped
                guard.status = AgentStatus::Error;
                let _ = app.emit("agent-status", AgentStatusEvent {
                    status: AgentStatus::Error,
                });
                return;
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

    // 4. Port availability
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
