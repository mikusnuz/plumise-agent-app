use std::path::PathBuf;
use tauri::Manager;

use super::agent::AgentConfig;

const CONFIG_FILE_NAME: &str = "agent-config.json";

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
            model: "openai/gpt-oss-20b".to_string(),
            device: "auto".to_string(),
            oracle_url: "https://node-1.plumise.com/oracle".to_string(),
            chain_rpc: "https://node-1.plumise.com/rpc".to_string(),
            http_port: 8080,
            grpc_port: 50051,
            ram_limit_mb: 0,
        });
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let config: AgentConfig = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    log::info!("Config loaded from {:?}", path);
    Ok(config)
}
