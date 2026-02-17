use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use plumise_agent_core as core;

#[derive(Parser)]
#[command(name = "plumise-agent", about = "Plumise distributed inference agent (CLI)")]
enum Cli {
    /// Start the agent
    Start {
        /// Path to config file (default: ~/.plumise-agent/agent-config.json)
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Override HTTP port
        #[arg(long)]
        port: Option<u16>,

        /// Override model HuggingFace repo
        #[arg(long)]
        model: Option<String>,

        /// Override GPU layers (-1 or 99 for all, 0 for CPU-only)
        #[arg(long)]
        gpu_layers: Option<i32>,

        /// Path to llama-server binary (auto-downloaded if omitted)
        #[arg(long)]
        llama_path: Option<PathBuf>,
    },

    /// Stop the running agent (via PID file)
    Stop,

    /// Show agent status
    Status {
        /// Agent HTTP port to query (default: 18920)
        #[arg(long, default_value = "18920")]
        port: u16,
    },

    /// Interactive setup wizard
    Init,
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    match Cli::parse() {
        Cli::Start {
            config,
            port,
            model,
            gpu_layers,
            llama_path,
        } => {
            if let Err(e) = cmd_start(config, port, model, gpu_layers, llama_path).await {
                log::error!("{}", e);
                std::process::exit(1);
            }
        }
        Cli::Stop => {
            if let Err(e) = cmd_stop() {
                log::error!("{}", e);
                std::process::exit(1);
            }
        }
        Cli::Status { port } => {
            if let Err(e) = cmd_status(port).await {
                log::error!("{}", e);
                std::process::exit(1);
            }
        }
        Cli::Init => {
            if let Err(e) = cmd_init() {
                log::error!("{}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn cmd_start(
    config_path: Option<PathBuf>,
    port_override: Option<u16>,
    model_override: Option<String>,
    gpu_layers_override: Option<i32>,
    llama_path_override: Option<PathBuf>,
) -> Result<(), String> {
    let config_dir = config_path
        .clone()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| core::config::default_config_dir());

    // Load config
    let mut config = core::config::load_config(&config_dir)?;

    // Apply overrides
    if let Some(port) = port_override {
        config.http_port = port;
    }
    if let Some(model) = model_override {
        config.model = model;
    }
    if let Some(ngl) = gpu_layers_override {
        config.gpu_layers = ngl;
    }

    // Validate private key
    if config.private_key.is_empty() {
        return Err(
            "Private key not configured. Run `plumise-agent init` to set up.".into(),
        );
    }
    if !config.private_key.starts_with("0x") || config.private_key.len() != 66 {
        return Err("Invalid private key format. Must be 0x-prefixed hex (66 chars).".into());
    }

    // Derive agent address
    let signing_key = core::chain::crypto::parse_private_key(&config.private_key)?;
    let agent_address = core::chain::crypto::address_from_key(&signing_key);
    log::info!("Agent address: {}", agent_address);

    // Ensure model
    let models_dir = config_dir.join("models");
    log::info!("Checking model file...");
    let model_path = core::model::download::ensure_model(
        &models_dir,
        &config.model,
        &config.model_file,
        |progress| {
            if progress.total_bytes > 0 {
                print!(
                    "\rDownloading model: {:.0}% ({:.1} / {:.1} GB)",
                    progress.percent,
                    progress.downloaded_bytes as f64 / 1e9,
                    progress.total_bytes as f64 / 1e9,
                );
                use std::io::Write;
                let _ = std::io::stdout().flush();
                if progress.percent >= 100.0 {
                    println!();
                }
            }
        },
    )
    .await?;

    // Ensure llama-server binary
    let bin_dir = config_dir.join("bin");
    let llama_server_path = if let Some(ref path) = llama_path_override {
        if !path.exists() {
            return Err(format!("Specified llama-server not found: {}", path.display()));
        }
        path.clone()
    } else {
        core::llama_bin::ensure_llama_server(&bin_dir).await?
    };

    // Kill leftover process on our port
    if let Some(killed) = core::system::kill_process_on_port(config.http_port) {
        log::warn!("Killed leftover process on port {}: {}", config.http_port, killed);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Auto-adjust parallel slots
    let effective_slots = core::process::adjust_parallel_slots(config.ctx_size, config.parallel_slots);

    // Build env vars
    let mut env_vars = std::collections::HashMap::new();
    // Set GGML_BACKEND_DIR to the bin directory (where DLLs/dylibs live)
    env_vars.insert(
        "GGML_BACKEND_DIR".to_string(),
        bin_dir.to_string_lossy().to_string(),
    );

    // Spawn llama-server
    let llama_config = core::process::LlamaServerConfig {
        binary_path: llama_server_path,
        model_path,
        host: "0.0.0.0".into(),
        port: config.http_port,
        gpu_layers: config.gpu_layers,
        ctx_size: config.ctx_size,
        parallel_slots: effective_slots,
        env_vars,
        rpc_peers: None,
    };

    let mut llama = core::process::LlamaProcess::spawn(&llama_config)?;
    let pid = llama.pid;

    // Write PID file
    let pid_file = config_dir.join("agent.pid");
    let _ = std::fs::write(&pid_file, pid.to_string());

    // Stream stdout/stderr to logger
    if let Some(stdout) = llama.take_stdout() {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let level = core::system::parse_log_level(&line);
                let masked = core::system::mask_sensitive_data(&line);
                match level {
                    "ERROR" => log::error!("[llama] {}", masked),
                    "WARNING" => log::warn!("[llama] {}", masked),
                    "DEBUG" => log::debug!("[llama] {}", masked),
                    _ => log::info!("[llama] {}", masked),
                }
            }
        });
    }
    if let Some(stderr) = llama.take_stderr() {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let masked = core::system::mask_sensitive_data(&line);
                log::warn!("[llama:err] {}", masked);
            }
        });
    }

    log::info!("Waiting for llama-server to become ready...");
    llama.wait_ready(300, config.http_port).await?;

    // Agent is ready — register with Oracle
    let client = reqwest::Client::new();

    let local_ip = core::system::get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    log::info!("Local endpoint: http://{}:{}", local_ip, config.http_port);

    let oracle_model = core::config::oracle_model_name(&config.model);

    // RAM
    let ram_mb = if config.ram_limit_gb > 0 {
        (config.ram_limit_gb as u64) * 1024
    } else {
        core::system::total_ram_mb()
    };

    // Benchmark
    let benchmark_tps = match core::inference::benchmark::run_benchmark(&client, config.http_port).await {
        Ok(tps) => {
            log::info!("Benchmark: {:.2} tok/s", tps);
            tps
        }
        Err(e) => {
            log::warn!("Benchmark skipped: {}", e);
            0.0
        }
    };

    let can_distribute = config.distributed_mode != "disabled";

    // Register with Oracle
    match core::oracle::registry::register(
        &client,
        &config.oracle_url,
        &signing_key,
        oracle_model,
        config.http_port,
        ram_mb,
        0,
        &config.device,
        &local_ip,
        benchmark_tps,
        can_distribute,
        &local_ip,
    )
    .await
    {
        Ok(_assignment) => {
            log::info!("Registered with Oracle");
        }
        Err(e) => {
            log::warn!("Oracle registration failed (non-fatal): {}", e);
        }
    }

    // Start background reporter
    let reporter_handle = core::oracle::reporter::start_reporter(
        client.clone(),
        config.oracle_url.clone(),
        signing_key.clone(),
        config.http_port,
        core::oracle::reporter::RegistrationParams {
            model: oracle_model.to_string(),
            http_port: config.http_port,
            ram_mb,
            vram_mb: 0,
            device: config.device.clone(),
            external_ip: local_ip.clone(),
            benchmark_tok_per_sec: benchmark_tps,
            can_distribute,
            lan_ip: local_ip.clone(),
        },
    );

    // Start WS relay
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
    log::info!("Connecting to inference relay: {}", ws_url);

    let relay_handle = core::relay::client::start_relay(
        ws_url,
        signing_key.clone(),
        oracle_model.to_string(),
        config.http_port,
    );

    log::info!("Agent is running. Press Ctrl+C to stop.");

    // Setup Ctrl+C handler using std::sync for signal handler compatibility
    let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_flag = Arc::clone(&shutdown);
    ctrlc::set_handler(move || {
        shutdown_flag.store(true, std::sync::atomic::Ordering::SeqCst);
    })
    .map_err(|e| format!("Failed to set Ctrl+C handler: {}", e))?;

    // Poll for shutdown signal
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
    }

    // Graceful shutdown
    log::info!("Shutting down...");
    reporter_handle.abort();
    relay_handle.abort();
    llama.kill();

    // Remove PID file
    let _ = std::fs::remove_file(&pid_file);

    log::info!("Agent stopped.");
    Ok(())
}

fn cmd_stop() -> Result<(), String> {
    let config_dir = core::config::default_config_dir();
    let pid_file = config_dir.join("agent.pid");

    if !pid_file.exists() {
        return Err("No PID file found. Agent may not be running.".into());
    }

    let pid_str = std::fs::read_to_string(&pid_file)
        .map_err(|e| format!("Failed to read PID file: {}", e))?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|e| format!("Invalid PID: {}", e))?;

    log::info!("Stopping agent PID: {}", pid);
    core::system::kill_pid(pid);

    let _ = std::fs::remove_file(&pid_file);
    log::info!("Agent stopped.");
    Ok(())
}

async fn cmd_status(port: u16) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let health_url = format!("http://127.0.0.1:{}/health", port);
    match client.get(&health_url).send().await {
        Ok(resp) => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let status = body["status"].as_str().unwrap_or("unknown");
                println!("Status: {}", status);

                if status == "ok" {
                    // Fetch metrics
                    let metrics =
                        core::inference::metrics::fetch_metrics(&client, port)
                            .await
                            .unwrap_or_default();
                    println!("Tokens processed: {}", metrics.total_tokens);
                    println!("Requests served: {}", metrics.total_requests);
                    println!("Throughput: {:.2} tok/s", metrics.tps);
                }
            } else {
                println!("Status: running (could not parse health response)");
            }
        }
        Err(_) => {
            println!("Status: not running (port {} unreachable)", port);

            // Check PID file
            let config_dir = core::config::default_config_dir();
            let pid_file = config_dir.join("agent.pid");
            if pid_file.exists() {
                if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                    println!("Stale PID file found: {}. Run `plumise-agent stop` to clean up.", pid.trim());
                }
            }
        }
    }

    Ok(())
}

fn cmd_init() -> Result<(), String> {
    let config_dir = core::config::default_config_dir();
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;

    let config = core::config::load_config(&config_dir)?;

    println!("Plumise Agent Setup");
    println!("===================");
    println!();
    println!("Config directory: {}", config_dir.display());
    println!();

    // Private key
    if config.private_key.is_empty() {
        println!("No private key configured.");
        println!("Edit {} and set your private key.", config_dir.join("agent-config.json").display());
    } else {
        let masked = format!(
            "{}...{}",
            &config.private_key[..6],
            &config.private_key[config.private_key.len() - 4..]
        );
        println!("Private key: {}", masked);
    }

    println!("Model: {} ({})", config.model, config.model_file);
    println!("Oracle: {}", config.oracle_url);
    println!("Chain RPC: {}", config.chain_rpc);
    println!("HTTP Port: {}", config.http_port);
    println!("GPU Layers: {}", config.gpu_layers);
    println!("Context Size: {}", config.ctx_size);
    println!();

    // Save default config if none exists
    let config_file = config_dir.join("agent-config.json");
    if !config_file.exists() {
        core::config::save_config(&config, &config_dir)?;
        println!("Default config saved to {}", config_file.display());
        println!("Edit it and run `plumise-agent start`.");
    } else {
        println!("Config file: {}", config_file.display());
    }

    // GPU detection
    if let Some((name, vram)) = core::system::detect_gpu() {
        if vram > 0 {
            println!("GPU: {} ({} MB)", name, vram);
        } else {
            println!("GPU: {}", name);
        }
    } else {
        println!("GPU: not detected (CPU-only mode)");
    }

    Ok(())
}
