use std::sync::Arc;
use tokio::sync::Mutex;

mod commands;
mod inference;
mod model;

use commands::agent::{AgentState, SharedAgentState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let agent_state: SharedAgentState = Arc::new(Mutex::new(AgentState::default()));

    tauri::Builder::default()
        .manage(agent_state)
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agent::start_agent,
            commands::agent::stop_agent,
            commands::agent::get_agent_status,
            commands::agent::get_agent_metrics,
            commands::agent::check_model,
            commands::agent::preflight_check,
            commands::system::get_system_info,
            commands::config::save_config,
            commands::config::load_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
