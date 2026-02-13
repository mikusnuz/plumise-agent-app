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

    // Save private_key to OS keyring
    if !config.private_key.is_empty() {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
        entry.set_password(&config.private_key)
            .map_err(|e| format!("Failed to save private key to keyring: {}", e))?;
        log::info!("Private key saved to OS keyring");
    }

    // Create a config copy without private_key for JSON storage
    let mut config_to_save = config.clone();
    config_to_save.private_key = String::new();

    let json = serde_json::to_string_pretty(&config_to_save)
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
            model_file: "gpt-oss-20b-MXFP4.gguf".to_string(),
            device: "auto".to_string(),
            oracle_url: "https://node-1.plumise.com/oracle".to_string(),
            chain_rpc: "https://node-1.plumise.com/rpc".to_string(),
            http_port: 18920,
            gpu_layers: 99,
            ctx_size: 8192,
            parallel_slots: 4,
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

    // Migration: if JSON has private_key, move it to keyring and remove from JSON
    if !config.private_key.is_empty() {
        log::info!("Migrating private key from JSON to keyring");
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
        entry.set_password(&config.private_key)
            .map_err(|e| format!("Failed to migrate private key to keyring: {}", e))?;

        // Clear from config and save back to JSON
        config.private_key = String::new();
        let json = serde_json::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to update config file: {}", e))?;
        log::info!("Private key migrated to keyring, removed from JSON");
    }

    // Load private_key from keyring
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;

    match entry.get_password() {
        Ok(pk) => {
            config.private_key = pk;
            log::info!("Private key loaded from OS keyring");
        }
        Err(keyring::Error::NoEntry) => {
            // No private key in keyring (new user or not set yet)
            log::info!("No private key found in keyring");
        }
        Err(e) => {
            log::warn!("Failed to read private key from keyring: {}", e);
        }
    }

    log::info!("Config loaded from {:?}", path);
    Ok(config)
}
