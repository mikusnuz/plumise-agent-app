use std::path::PathBuf;
use tauri::Manager;
use keyring::Entry;

use super::agent::AgentConfig;

const CONFIG_FILE_NAME: &str = "agent-config.json";
const KEYRING_SERVICE: &str = "com.plumise.agent";
const KEYRING_USER: &str = "plumise-agent-private-key";

fn get_config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    // Ensure the directory exists
    std::fs::create_dir_all(&app_data_dir)
        .map_err(|e| format!("Failed to create app data directory: {}", e))?;

    Ok(app_data_dir.join(CONFIG_FILE_NAME))
}

#[tauri::command]
pub async fn save_config(config: AgentConfig, app: tauri::AppHandle) -> Result<(), String> {
    let path = get_config_path(&app)?;

    // Try to save private_key to OS keyring (best-effort, may fail on macOS with ad-hoc signing)
    if !config.private_key.is_empty() {
        match Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            Ok(entry) => {
                if let Err(e) = entry.set_password(&config.private_key) {
                    log::warn!("Keyring save failed (non-fatal): {}", e);
                } else {
                    log::info!("Private key saved to OS keyring");
                }
            }
            Err(e) => {
                log::warn!("Keyring not available (non-fatal): {}", e);
            }
        }
    }

    // Always save full config including private_key to JSON
    // (JSON is in app_data_dir which is user-private)
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    log::info!("Config saved to {:?}", path);
    Ok(())
}

#[tauri::command]
pub async fn load_config(app: tauri::AppHandle) -> Result<AgentConfig, String> {
    let path = get_config_path(&app)?;

    if !path.exists() {
        // Return default config if no saved config exists
        return Ok(AgentConfig {
            private_key: String::new(),
            model: "ggml-org/gpt-oss-20b-GGUF".to_string(),
            model_file: "gpt-oss-20b-mxfp4.gguf".to_string(),
            device: "auto".to_string(),
            oracle_url: "https://plug.plumise.com/oracle".to_string(),
            chain_rpc: "https://plug.plumise.com/rpc/plug_live_w9mS7DOAqMGlhyYwhLa8MOE-7UZfbKwCT34ib8JLZL0".to_string(),
            http_port: 18920,
            gpu_layers: 99,
            ctx_size: 8192,
            parallel_slots: 1,
            ram_limit_gb: 0,
        });
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let mut config: AgentConfig = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    // Migration: fix old default port 8080 -> 18920
    if config.http_port == 8080 {
        log::info!("Migrating http_port from old default 8080 to 18920");
        config.http_port = 18920;
    }

    // Migration: old model name -> new GGUF repo
    if config.model == "openai/gpt-oss-20b" {
        log::info!("Migrating model from openai/gpt-oss-20b to ggml-org/gpt-oss-20b-GGUF");
        config.model = "ggml-org/gpt-oss-20b-GGUF".to_string();
    }

    // Migration: fix model_file case (MXFP4 -> mxfp4, HuggingFace is case-sensitive)
    if config.model_file == "gpt-oss-20b-MXFP4.gguf" {
        log::info!("Migrating model_file case: MXFP4 -> mxfp4");
        config.model_file = "gpt-oss-20b-mxfp4.gguf".to_string();
    }

    // Migration: parallelSlots 4 was old default, too high for 8192 ctx (only 2048/slot)
    if config.parallel_slots == 4 && config.ctx_size <= 8192 {
        log::info!("Migrating parallel_slots from 4 to 1 (was causing context overflow)");
        config.parallel_slots = 1;
    }

    // Migration: node-1.plumise.com → plug.plumise.com
    if config.oracle_url.contains("node-1.plumise.com") {
        log::info!("Migrating oracle_url from node-1 to plug.plumise.com");
        config.oracle_url = "https://plug.plumise.com/oracle".to_string();
    }
    if config.chain_rpc.contains("node-1.plumise.com") {
        log::info!("Migrating chain_rpc from node-1 to plug.plumise.com");
        config.chain_rpc = "https://plug.plumise.com/rpc/plug_live_w9mS7DOAqMGlhyYwhLa8MOE-7UZfbKwCT34ib8JLZL0".to_string();
    }

    // Private key loading: try keyring first (more secure), fall back to JSON value.
    // JSON now always stores the key as backup for platforms where keyring is unreliable.
    let json_private_key = config.private_key.clone();

    match Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        Ok(entry) => match entry.get_password() {
            Ok(pk) if !pk.is_empty() => {
                config.private_key = pk;
                log::info!("Private key loaded from OS keyring");
            }
            _ => {
                // Keyring empty or error — keep JSON value
                if !json_private_key.is_empty() {
                    config.private_key = json_private_key;
                    log::info!("Private key loaded from config JSON (keyring fallback)");
                } else {
                    log::info!("No private key found");
                }
            }
        },
        Err(e) => {
            log::warn!("Keyring not available: {}", e);
            // Keep JSON value as fallback
            if !json_private_key.is_empty() {
                config.private_key = json_private_key;
                log::info!("Private key loaded from config JSON (keyring unavailable)");
            }
        }
    }

    log::info!("Config loaded from {:?}", path);
    Ok(config)
}
