use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

use crate::commands::agent::{AgentState, AgentStatus, NodeMode, SharedAgentState};

/// Start the ggml-rpc server sidecar on the specified port.
/// Returns the PID of the spawned process.
pub async fn start_rpc_server(
    app: &AppHandle,
    state: &SharedAgentState,
    port: u16,
    gpu_layers: i32,
) -> Result<u32, String> {
    log::info!("Starting rpc-server on port {} (gpu_layers={})", port, gpu_layers);

    let _ = app.emit("agent-log", serde_json::json!({
        "level": "INFO",
        "message": format!("Starting RPC server on port {} for distributed inference", port),
    }));

    // Resolve backend directories (same logic as llama-server)
    let backend_path = {
        let mut dirs = Vec::new();
        if let Ok(resource_dir) = app.path().resource_dir() {
            dirs.push(resource_dir.to_string_lossy().to_string());
            let bin_dir = resource_dir.join("binaries");
            if bin_dir.is_dir() {
                dirs.push(bin_dir.to_string_lossy().to_string());
            }
        }
        if let Ok(exe_dir) = std::env::current_exe() {
            if let Some(parent) = exe_dir.parent() {
                let s = parent.to_string_lossy().to_string();
                if !dirs.contains(&s) {
                    dirs.push(s);
                }
            }
        }
        dirs.join(if cfg!(windows) { ";" } else { ":" })
    };

    let args = vec![
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Try Tauri sidecar first
    let spawn_result = app
        .shell()
        .sidecar("rpc-server")
        .and_then(|cmd| {
            Ok(cmd.args(&args_ref).envs([
                ("GGML_BACKEND_DIR".to_string(), backend_path.clone()),
            ]))
        })
        .and_then(|cmd| cmd.spawn());

    match spawn_result {
        Ok((mut rx, child)) => {
            let pid = child.pid();
            log::info!("rpc-server spawned via sidecar, PID: {}", pid);

            // Log stdout/stderr + watchdog
            let app_ev = app.clone();
            let state_watch = Arc::clone(state);
            tokio::spawn(async move {
                use tauri_plugin_shell::process::CommandEvent;
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                            if let Ok(line) = String::from_utf8(bytes) {
                                let trimmed = line.trim();
                                if !trimmed.is_empty() {
                                    log::debug!("[rpc-server] {}", trimmed);
                                }
                            }
                        }
                        CommandEvent::Terminated(payload) => {
                            log::warn!("rpc-server terminated: code={:?}", payload.code);
                            let mut guard = state_watch.lock().await;
                            // Only handle if we were in RPC server mode (not manually stopping)
                            if guard.node_mode == NodeMode::RpcServer
                                && guard.status != AgentStatus::Stopped
                                && guard.status != AgentStatus::Stopping
                            {
                                guard.status = AgentStatus::Error;
                                guard.rpc_server_pid = None;
                                guard.node_mode = NodeMode::Standalone;
                                guard.cluster_id = None;
                                let _ = app_ev.emit("agent-status", serde_json::json!({
                                    "status": "error",
                                }));
                                let _ = app_ev.emit("agent-log", serde_json::json!({
                                    "level": "ERROR",
                                    "message": format!("RPC server crashed (code: {:?}). Agent stopped.", payload.code),
                                }));
                            } else {
                                let _ = app_ev.emit("agent-log", serde_json::json!({
                                    "level": "WARNING",
                                    "message": format!("RPC server terminated (code: {:?})", payload.code),
                                }));
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            });

            Ok(pid)
        }
        Err(e) => {
            log::error!("Failed to spawn rpc-server sidecar: {}", e);
            Err(format!("Failed to start rpc-server: {}", e))
        }
    }
}

/// Stop the rpc-server process.
pub fn stop_rpc_server(pid: u32) {
    log::info!("Stopping rpc-server PID: {}", pid);
    plumise_agent_core::system::kill_pid(pid);
}
