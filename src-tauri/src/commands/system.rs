use serde::{Deserialize, Serialize};
use sysinfo::System;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub cpu_usage: f32,
    pub ram_total: u64,
    pub ram_used: u64,
    pub vram_total: u64,
    pub vram_used: u64,
    pub gpu_name: String,
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, String> {
    let mut sys = System::new();

    // Refresh CPU usage - need two samples with delay for accurate reading
    sys.refresh_cpu_usage();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sys.refresh_cpu_usage();

    // Refresh memory info
    sys.refresh_memory();

    let cpu_usage = sys.global_cpu_usage();
    let ram_total = sys.total_memory(); // in bytes
    let ram_used = sys.used_memory(); // in bytes

    // Try to get GPU info via nvidia-smi
    let (vram_total, vram_used, gpu_name) = get_gpu_info().await;

    Ok(SystemInfo {
        cpu_usage,
        ram_total,
        ram_used,
        vram_total,
        vram_used,
        gpu_name,
    })
}

async fn get_gpu_info() -> (u64, u64, String) {
    // Try nvidia-smi for NVIDIA GPUs (with hidden console window on Windows)
    let mut cmd = Command::new("nvidia-smi");
    cmd.args([
        "--query-gpu=memory.total,memory.used,name",
        "--format=csv,noheader,nounits",
    ]);

    // Prevent console window from flashing on Windows
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let result = cmd.output().await;

    if let Ok(output) = result {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Output format: "24576, 1234, NVIDIA GeForce RTX 5090" (in MiB)
            if let Some(line) = stdout.lines().next() {
                let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 3 {
                    let total = parts[0].parse::<u64>().unwrap_or(0) * 1024 * 1024; // MiB to bytes
                    let used = parts[1].parse::<u64>().unwrap_or(0) * 1024 * 1024;
                    let name = parts[2].to_string();
                    return (total, used, name);
                } else if parts.len() >= 2 {
                    let total = parts[0].parse::<u64>().unwrap_or(0) * 1024 * 1024;
                    let used = parts[1].parse::<u64>().unwrap_or(0) * 1024 * 1024;
                    return (total, used, "NVIDIA GPU".to_string());
                }
            }
        }
    }

    // No GPU info available
    (0, 0, String::new())
}
