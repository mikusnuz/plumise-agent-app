use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    percent: f32,
    phase: String,
}

/// Ensure the GGUF model file exists. Downloads from HuggingFace if missing.
/// Returns the full path to the model file.
pub async fn ensure_model(
    models_dir: &Path,
    repo: &str,
    filename: &str,
    app: &AppHandle,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(models_dir)
        .map_err(|e| format!("Failed to create models dir: {}", e))?;

    let model_path = models_dir.join(filename);

    if model_path.exists() {
        let metadata = std::fs::metadata(&model_path)
            .map_err(|e| format!("Failed to read model file metadata: {}", e))?;
        // Sanity check: GGUF should be at least 100MB
        if metadata.len() > 100 * 1024 * 1024 {
            log::info!("Model already downloaded: {:?} ({:.1} GB)", model_path, metadata.len() as f64 / 1e9);
            return Ok(model_path);
        }
        // File exists but too small → likely incomplete, re-download
        log::warn!("Model file too small ({} bytes), re-downloading", metadata.len());
        let _ = std::fs::remove_file(&model_path);
    }

    // Check for partial download
    let partial_path = models_dir.join(format!("{}.partial", filename));

    download_model(repo, filename, &model_path, &partial_path, app).await?;

    Ok(model_path)
}

async fn download_model(
    repo: &str,
    filename: &str,
    target: &Path,
    partial: &Path,
    app: &AppHandle,
) -> Result<(), String> {
    let url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        repo, filename
    );

    log::info!("Downloading model from {}", url);

    let client = reqwest::Client::new();

    // Check if partial file exists for resume
    let existing_size = if partial.exists() {
        std::fs::metadata(partial).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    let mut req = client.get(&url);
    if existing_size > 0 {
        log::info!("Resuming download from byte {}", existing_size);
        req = req.header("Range", format!("bytes={}-", existing_size));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !resp.status().is_success() && resp.status().as_u16() != 206 {
        return Err(format!("Download failed with HTTP {}", resp.status()));
    }

    let total_size = if resp.status().as_u16() == 206 {
        // Partial content — total size from Content-Range header
        resp.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        resp.content_length().unwrap_or(0)
    };

    // Open file for append (resume) or create
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(existing_size > 0)
        .write(true)
        .truncate(existing_size == 0)
        .open(partial)
        .map_err(|e| format!("Failed to open partial file: {}", e))?;

    let mut downloaded = existing_size;
    let mut last_pct: i32 = -1;

    // Stream download using chunk()
    let mut response = resp;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Download stream error: {}", e))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("Write error: {}", e))?;
        downloaded += chunk.len() as u64;

        if total_size > 0 {
            let pct = ((downloaded as f64 / total_size as f64) * 100.0) as i32;
            if pct != last_pct {
                last_pct = pct;
                let _ = app.emit(
                    "agent-loading-progress",
                    DownloadProgress {
                        percent: pct as f32,
                        phase: "downloading".to_string(),
                    },
                );
            }
        }
    }

    file.flush().map_err(|e| format!("Flush error: {}", e))?;
    drop(file);

    // Rename partial → final
    std::fs::rename(partial, target)
        .map_err(|e| format!("Failed to finalize model file: {}", e))?;

    log::info!(
        "Model download complete: {:.1} GB",
        downloaded as f64 / 1e9
    );

    Ok(())
}
