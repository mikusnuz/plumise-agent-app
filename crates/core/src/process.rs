use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for spawning a llama-server process.
#[derive(Debug, Clone)]
pub struct LlamaServerConfig {
    pub binary_path: PathBuf,
    pub model_path: PathBuf,
    pub host: String,
    pub port: u16,
    pub gpu_layers: i32,
    pub ctx_size: u32,
    pub parallel_slots: u32,
    pub env_vars: HashMap<String, String>,
    pub rpc_peers: Option<Vec<String>>,
}

/// Build llama-server command-line arguments from config.
pub fn build_llama_args(config: &LlamaServerConfig) -> Vec<String> {
    let mut args = vec![
        "-m".into(),
        config.model_path.to_string_lossy().to_string(),
        "--host".into(),
        config.host.clone(),
        "--port".into(),
        config.port.to_string(),
        "-ngl".into(),
        config.gpu_layers.to_string(),
        "--ctx-size".into(),
        config.ctx_size.to_string(),
        "-np".into(),
        config.parallel_slots.to_string(),
        "--jinja".into(),
    ];

    if let Some(ref peers) = config.rpc_peers {
        if !peers.is_empty() {
            args.push("--rpc".into());
            args.push(peers.join(","));
        }
    }

    args
}

/// Auto-adjust parallel slots so each slot gets at least 4096 tokens.
pub fn adjust_parallel_slots(ctx_size: u32, parallel_slots: u32) -> u32 {
    if parallel_slots <= 1 {
        return parallel_slots;
    }
    let per_slot = ctx_size / parallel_slots;
    if per_slot < 4096 {
        let adjusted = (ctx_size / 4096).max(1);
        log::warn!(
            "Reduced parallel_slots from {} to {} (per-slot context {} < 4096)",
            parallel_slots, adjusted, per_slot,
        );
        adjusted
    } else {
        parallel_slots
    }
}

/// Managed llama-server process spawned via tokio::process.
/// Used by CLI; GUI uses Tauri sidecar instead (for Windows DLL resolution).
pub struct LlamaProcess {
    child: tokio::process::Child,
    pub pid: u32,
}

impl LlamaProcess {
    /// Spawn llama-server as a child process.
    pub fn spawn(config: &LlamaServerConfig) -> Result<Self, String> {
        let args = build_llama_args(config);

        log::info!(
            "Spawning llama-server: {} {}",
            config.binary_path.display(),
            args.join(" ")
        );

        let mut cmd = tokio::process::Command::new(&config.binary_path);
        cmd.args(&args);

        // Set environment variables
        for (k, v) in &config.env_vars {
            cmd.env(k, v);
        }

        // Set working directory to binary's parent (for DLL discovery)
        if let Some(parent) = config.binary_path.parent() {
            cmd.current_dir(parent);
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn llama-server: {}", e))?;

        let pid = child.id().unwrap_or(0);
        log::info!("llama-server spawned, PID: {}", pid);

        Ok(Self { child, pid })
    }

    /// Poll /health endpoint until llama-server reports "ok" or timeout.
    pub async fn wait_ready(&self, timeout_secs: u64, port: u16) -> Result<(), String> {
        let client = reqwest::Client::new();
        let health_url = format!("http://127.0.0.1:{}/health", port);
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(timeout_secs);

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(format!(
                    "llama-server did not become ready within {} seconds",
                    timeout_secs
                ));
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            match client
                .get(&health_url)
                .timeout(std::time::Duration::from_secs(3))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let status = body["status"].as_str().unwrap_or("");
                        if status == "ok" {
                            log::info!("llama-server is ready");
                            return Ok(());
                        }
                        if status == "loading model" {
                            if let Some(progress) = body["progress"].as_f64() {
                                log::info!("Loading model: {:.0}%", progress * 100.0);
                            }
                        }
                    }
                }
                Err(_) => {
                    // Not ready yet
                }
            }
        }
    }

    /// Kill the llama-server process.
    pub fn kill(&mut self) {
        crate::system::kill_pid(self.pid);
    }

    /// Take stdout for log streaming.
    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    /// Take stderr for log streaming.
    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// Wait for the process to exit.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus, String> {
        self.child
            .wait()
            .await
            .map_err(|e| format!("Failed to wait for llama-server: {}", e))
    }
}
