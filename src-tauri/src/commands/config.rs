use tauri::Manager;
use plumise_agent_core::config::AgentConfig;

#[tauri::command]
pub async fn save_config(config: AgentConfig, app: tauri::AppHandle) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    plumise_agent_core::config::save_config(&config, &app_data_dir)
}

#[tauri::command]
pub async fn load_config(app: tauri::AppHandle) -> Result<AgentConfig, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    plumise_agent_core::config::load_config(&app_data_dir)
}
