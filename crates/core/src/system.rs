use serde::{Deserialize, Serialize};

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

/// Cross-platform GPU detection.
pub fn detect_gpu() -> Option<(String, u64)> {
    #[cfg(target_os = "macos")]
    return detect_metal_gpu();

    #[cfg(not(target_os = "macos"))]
    return detect_nvidia_gpu();
}

/// Detect Metal GPU on macOS (Apple Silicon or discrete).
#[cfg(target_os = "macos")]
fn detect_metal_gpu() -> Option<(String, u64)> {
    let output = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return Some(("Apple GPU (Metal)".to_string(), 0));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let name = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("Chipset Model:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Apple GPU".to_string());

    // Apple Silicon uses unified memory — report total system RAM
    let sys = sysinfo::System::new_all();
    let unified_ram_mb = sys.total_memory() / (1024 * 1024);

    Some((format!("{} (Metal)", name), unified_ram_mb))
}

#[cfg(not(target_os = "macos"))]
fn detect_nvidia_gpu() -> Option<(String, u64)> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?.trim().to_string();
    let mut parts = line.splitn(2, ',');
    let name = parts.next()?.trim().to_string();
    let vram: u64 = parts.next()?.trim().parse().unwrap_or(0);

    if name.is_empty() {
        return None;
    }
    Some((name, vram))
}

/// Discover the machine's LAN IP by connecting a UDP socket to a remote address.
pub fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip().to_string();
    if ip == "127.0.0.1" || ip == "::1" {
        return None;
    }
    Some(ip)
}

/// Get total system RAM in megabytes.
pub fn total_ram_mb() -> u64 {
    let sys = sysinfo::System::new_all();
    sys.total_memory() / (1024 * 1024)
}

/// Kill a process by PID (cross-platform).
pub fn kill_pid(pid: u32) {
    log::info!("Killing process PID: {}", pid);
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/F", "/PID", &pid.to_string()]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
        let _ = cmd.output();
    }
}

/// Kill any llama-server process listening on the given port.
pub fn kill_process_on_port(port: u16) -> Option<String> {
    #[cfg(unix)]
    {
        let output = std::process::Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
            .ok()?;

        let pids_str = String::from_utf8_lossy(&output.stdout);
        let pids: Vec<&str> = pids_str.trim().lines().collect();
        if pids.is_empty() {
            return None;
        }

        let mut killed = Vec::new();
        for pid in &pids {
            let pid = pid.trim();
            if pid.is_empty() {
                continue;
            }
            let ps_out = std::process::Command::new("ps")
                .args(["-p", pid, "-o", "comm="])
                .output()
                .ok();
            let proc_name = ps_out
                .as_ref()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();

            if proc_name.contains("llama-server") || proc_name.contains("llama_server") {
                let _ = std::process::Command::new("kill")
                    .args(["-9", pid])
                    .output();
                killed.push(format!("PID {} ({})", pid, proc_name));
            } else {
                log::warn!(
                    "Port {} held by non-llama process: PID {} ({}), skipping",
                    port, pid, proc_name
                );
            }
        }

        if killed.is_empty() {
            None
        } else {
            Some(killed.join(", "))
        }
    }

    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", &format!("netstat -ano | findstr :{}", port)])
            .output()
            .ok()?;

        let text = String::from_utf8_lossy(&output.stdout);
        let mut killed = Vec::new();

        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 && parts[1].contains(&format!(":{}", port)) {
                let pid = parts[4];
                let tasklist = std::process::Command::new("tasklist")
                    .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
                    .output()
                    .ok();
                let proc_name = tasklist
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();

                if proc_name.to_lowercase().contains("llama-server") || proc_name.to_lowercase().contains("llama_server") {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/PID", pid])
                        .output();
                    killed.push(format!("PID {}", pid));
                } else {
                    log::warn!("Port {} held by non-llama process: PID {}, skipping", port, pid);
                }
            }
        }

        if killed.is_empty() {
            None
        } else {
            Some(killed.join(", "))
        }
    }
}

/// Convert wei (as string) to human-readable display (e.g., "1.2345").
pub fn wei_to_display(wei_str: &str) -> String {
    let padded = format!("{:0>19}", wei_str);
    let split_pos = padded.len() - 18;
    let integer_part = &padded[..split_pos];
    let decimal_part = &padded[split_pos..];
    let trimmed = decimal_part.trim_end_matches('0');
    let decimal_display = if trimmed.len() < 4 {
        &decimal_part[..4]
    } else {
        trimmed
    };
    format!("{}.{}", integer_part, decimal_display)
}

/// Check wallet balance via RPC.
pub async fn check_wallet_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    private_key: &str,
) -> Result<(String, String), String> {
    let signing_key = crate::chain::crypto::parse_private_key(private_key)?;
    let address = crate::chain::crypto::address_from_key(&signing_key);

    let resp = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBalance",
            "params": [&address, "latest"],
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| format!("RPC failed: {}", e))?;

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
    let balance_hex = json["result"].as_str().ok_or("No balance")?;
    let balance_hex = balance_hex.strip_prefix("0x").unwrap_or(balance_hex);
    let balance_wei =
        u128::from_str_radix(balance_hex, 16).map_err(|e| format!("Invalid balance: {}", e))?;
    Ok((wei_to_display(&balance_wei.to_string()), address))
}

/// Describe Windows exit codes for common llama-server failures.
pub fn describe_exit_code(code: Option<i32>) -> String {
    match code {
        Some(-1073741515) => {
            "llama-server failed: Required DLL not found. \
             Please install NVIDIA CUDA Toolkit 12.x or ensure CUDA drivers are up to date."
                .to_string()
        }
        Some(-1073741819) => {
            "llama-server crashed: Access violation. \
             Try reducing GPU layers (gpu_layers=0 for CPU-only mode)."
                .to_string()
        }
        Some(c) => format!("llama-server exited with code {}", c),
        None => "llama-server was terminated by signal".to_string(),
    }
}

/// Parse log level from a llama-server log line.
pub fn parse_log_level(line: &str) -> &str {
    let upper = line.to_uppercase();
    if upper.contains("ERROR") || upper.contains("CRITICAL") || upper.contains("FATAL") {
        "ERROR"
    } else if upper.contains("WARNING") || upper.contains("WARN") {
        "WARNING"
    } else if upper.contains("DEBUG") || upper.contains("TRACE") {
        "DEBUG"
    } else {
        "INFO"
    }
}

/// Mask long hex strings (private keys, etc.) in log lines.
pub fn mask_sensitive_data(line: &str) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        if i + 66 <= len && bytes[i] == b'0' && bytes[i + 1] == b'x' {
            let candidate = &line[i + 2..i + 66];
            if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                result.push_str(&line[i..i + 6]);
                result.push_str("****...****");
                result.push_str(&line[i + 62..i + 66]);
                i += 66;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}
