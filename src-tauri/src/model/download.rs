use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

/// Thin wrapper around core's ensure_model that bridges progress events to Tauri.
pub async fn ensure_model(
    models_dir: &Path,
    repo: &str,
    filename: &str,
    app: &AppHandle,
) -> Result<PathBuf, String> {
    let app_clone = app.clone();

    plumise_agent_core::model::download::ensure_model(
        models_dir,
        repo,
        filename,
        move |progress| {
            let _ = app_clone.emit(
                "agent-loading-progress",
                serde_json::json!({
                    "percent": progress.percent,
                    "phase": progress.phase,
                    "downloadedBytes": progress.downloaded_bytes,
                    "totalBytes": progress.total_bytes,
                }),
            );
        },
    )
    .await
}
