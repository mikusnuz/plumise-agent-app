//! Auto-download llama.cpp pre-built binaries for CLI usage.
//! The GUI ships llama-server as a Tauri sidecar; the CLI downloads it on first run.

use std::path::{Path, PathBuf};

/// Pinned llama.cpp release version.
pub const LLAMA_CPP_VERSION: &str = "b4722";

/// Detect the appropriate llama.cpp release asset name for this platform.
pub fn detect_asset_name() -> Result<String, String> {
    let gpu = crate::system::detect_gpu();
    let has_nvidia = gpu
        .as_ref()
        .map(|(name, _)| name.to_lowercase().contains("nvidia") || name.to_lowercase().contains("geforce") || name.to_lowercase().contains("rtx"))
        .unwrap_or(false);
    let has_metal = gpu
        .as_ref()
        .map(|(name, _)| name.to_lowercase().contains("metal") || name.to_lowercase().contains("apple"))
        .unwrap_or(false);

    #[cfg(target_os = "linux")]
    {
        if has_nvidia {
            return Ok(format!("llama-{}-bin-ubuntu-x64-cuda-cu12.4.zip", LLAMA_CPP_VERSION));
        }
        return Ok(format!("llama-{}-bin-ubuntu-x64.zip", LLAMA_CPP_VERSION));
    }

    #[cfg(target_os = "macos")]
    {
        let _ = has_nvidia;
        let _ = has_metal;
        return Ok(format!("llama-{}-bin-macos-arm64.zip", LLAMA_CPP_VERSION));
    }

    #[cfg(target_os = "windows")]
    {
        if has_nvidia {
            return Ok(format!("llama-{}-bin-win-cuda-cu12.4-x64.zip", LLAMA_CPP_VERSION));
        }
        return Ok(format!("llama-{}-bin-win-avx2-x64.zip", LLAMA_CPP_VERSION));
    }

    #[allow(unreachable_code)]
    Err("Unsupported platform".into())
}

/// Ensure llama-server binary exists in the given directory.
/// Downloads from GitHub releases if missing.
/// Returns path to the llama-server executable.
pub async fn ensure_llama_server(bin_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(bin_dir)
        .map_err(|e| format!("Failed to create bin dir: {}", e))?;

    let exe_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    let server_path = bin_dir.join(exe_name);

    // Check version marker
    let version_file = bin_dir.join(".llama-version");
    let current_version = std::fs::read_to_string(&version_file).unwrap_or_default();

    if server_path.exists() && current_version.trim() == LLAMA_CPP_VERSION {
        log::info!("llama-server {} already installed", LLAMA_CPP_VERSION);
        return Ok(server_path);
    }

    let asset_name = detect_asset_name()?;
    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{}/{}",
        LLAMA_CPP_VERSION, asset_name
    );

    log::info!("Downloading llama-server from {}", url);
    println!("Downloading llama-server {}...", LLAMA_CPP_VERSION);

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Download failed with HTTP {}", resp.status()));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download: {}", e))?;

    // Extract zip
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Failed to open zip: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Zip entry error: {}", e))?;

        let name = file.name().to_string();
        // Extract files to bin_dir (flatten directory structure)
        let file_name = Path::new(&name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if file_name.is_empty() || file.is_dir() {
            continue;
        }

        // Only extract executables and shared libraries
        let dominated = file_name.ends_with(".exe")
            || file_name.ends_with(".dll")
            || file_name.ends_with(".so")
            || file_name.ends_with(".dylib")
            || file_name.ends_with(".metal")
            || file_name == "llama-server"
            || file_name == "rpc-server"
            || file_name.starts_with("llama-")
            || file_name.starts_with("rpc-server");

        if !dominated {
            continue;
        }

        let out_path = bin_dir.join(&file_name);
        let mut out_file = std::fs::File::create(&out_path)
            .map_err(|e| format!("Failed to create {}: {}", file_name, e))?;

        std::io::copy(&mut file, &mut out_file)
            .map_err(|e| format!("Failed to write {}: {}", file_name, e))?;

        // Set executable permission on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file_name == "llama-server" || file_name == "rpc-server" {
                let _ = std::fs::set_permissions(
                    &out_path,
                    std::fs::Permissions::from_mode(0o755),
                );
            }
        }
    }

    // Write version marker
    let _ = std::fs::write(&version_file, LLAMA_CPP_VERSION);

    if !server_path.exists() {
        return Err("llama-server not found in downloaded archive".into());
    }

    log::info!("llama-server {} installed to {}", LLAMA_CPP_VERSION, server_path.display());
    println!("llama-server {} installed.", LLAMA_CPP_VERSION);

    Ok(server_path)
}
