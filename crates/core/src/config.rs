use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub private_key: String,
    pub model: String,
    #[serde(default = "default_model_file")]
    pub model_file: String,
    pub device: String,
    pub oracle_url: String,
    pub chain_rpc: String,
    pub http_port: u16,
    #[serde(default = "default_gpu_layers")]
    pub gpu_layers: i32,
    #[serde(default = "default_ctx_size")]
    pub ctx_size: u32,
    #[serde(default = "default_parallel_slots")]
    pub parallel_slots: u32,
    #[serde(default = "default_ram_limit_gb")]
    pub ram_limit_gb: u32,
    #[serde(default = "default_distributed_mode")]
    pub distributed_mode: String,
    #[serde(default = "default_rpc_port")]
    pub rpc_port: u16,
}

fn default_model_file() -> String {
    "Qwen3-32B-Q4_K_M.gguf".to_string()
}
fn default_gpu_layers() -> i32 {
    99
}
fn default_ctx_size() -> u32 {
    32768
}
fn default_parallel_slots() -> u32 {
    1
}
fn default_ram_limit_gb() -> u32 {
    0
}
fn default_distributed_mode() -> String {
    "auto".to_string()
}
fn default_rpc_port() -> u16 {
    50052
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            private_key: String::new(),
            model: "Qwen/Qwen3-32B-GGUF".to_string(),
            model_file: default_model_file(),
            device: "auto".to_string(),
            oracle_url: "https://plug.plumise.com/oracle".to_string(),
            chain_rpc: "https://plug.plumise.com/rpc/plug_live_6VuDzRY1lNoA2noX0lSPGQlm9itOF9td4Jvvd4eAMzE".to_string(),
            http_port: 18920,
            gpu_layers: default_gpu_layers(),
            ctx_size: default_ctx_size(),
            parallel_slots: default_parallel_slots(),
            ram_limit_gb: default_ram_limit_gb(),
            distributed_mode: default_distributed_mode(),
            rpc_port: default_rpc_port(),
        }
    }
}

const CONFIG_FILE_NAME: &str = "agent-config.json";
const KEYRING_SERVICE: &str = "com.plumise.agent";
const KEYRING_USER: &str = "plumise-agent-private-key";

/// Save config to a directory (JSON file + optional keyring).
pub fn save_config(config: &AgentConfig, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;

    // Try to save private_key to OS keyring (best-effort)
    if !config.private_key.is_empty() {
        match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
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

    let path = dir.join(CONFIG_FILE_NAME);
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    log::info!("Config saved to {:?}", path);
    Ok(())
}

/// Load config from a directory.
pub fn load_config(dir: &Path) -> Result<AgentConfig, String> {
    let path = dir.join(CONFIG_FILE_NAME);

    if !path.exists() {
        return Ok(AgentConfig::default());
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let mut config: AgentConfig = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    // Migrations
    if config.http_port == 8080 {
        log::info!("Migrating http_port from old default 8080 to 18920");
        config.http_port = 18920;
    }
    // Migrate old gpt-oss-20b models to Qwen3-32B
    if config.model.contains("gpt-oss-20b") {
        log::info!("Migrating model from {} to Qwen/Qwen3-32B-GGUF", config.model);
        config.model = "Qwen/Qwen3-32B-GGUF".to_string();
        config.model_file = "Qwen3-32B-Q4_K_M.gguf".to_string();
    }
    if config.parallel_slots == 4 && config.ctx_size <= 8192 {
        log::info!("Migrating parallel_slots from 4 to 1");
        config.parallel_slots = 1;
    }
    if config.ctx_size == 8192 {
        log::info!("Migrating ctx_size from 8192 to 32768");
        config.ctx_size = 32768;
    }
    if config.oracle_url.contains("node-1.plumise.com") {
        log::info!("Migrating oracle_url to plug.plumise.com");
        config.oracle_url = "https://plug.plumise.com/oracle".to_string();
    }
    if config.chain_rpc.contains("node-1.plumise.com") || config.chain_rpc.contains("plug_live_w9mS7DOAqMGlhyYwhLa8MOE") {
        log::info!("Migrating chain_rpc to new Plug API key");
        config.chain_rpc = "https://plug.plumise.com/rpc/plug_live_6VuDzRY1lNoA2noX0lSPGQlm9itOF9td4Jvvd4eAMzE".to_string();
    }

    // Private key: try keyring first, fall back to JSON
    let json_private_key = config.private_key.clone();
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        Ok(entry) => match entry.get_password() {
            Ok(pk) if !pk.is_empty() => {
                config.private_key = pk;
                log::info!("Private key loaded from OS keyring");
            }
            _ => {
                if !json_private_key.is_empty() {
                    config.private_key = json_private_key;
                    log::info!("Private key loaded from config JSON (keyring fallback)");
                }
            }
        },
        Err(e) => {
            log::warn!("Keyring not available: {}", e);
            if !json_private_key.is_empty() {
                config.private_key = json_private_key;
            }
        }
    }

    log::info!("Config loaded from {:?}", path);
    Ok(config)
}

/// Map HuggingFace GGUF repo to oracle model ID.
pub fn oracle_model_name(gguf_repo: &str) -> &'static str {
    match gguf_repo {
        s if s.contains("Qwen3-32B") => "qwen/qwen3-32b",
        s if s.contains("Qwen3.5-397B") || s.contains("qwen3.5-397b") => "qwen/qwen3.5-397b-a17b",
        _ => "qwen/qwen3-32b", // fallback
    }
}

/// Get the default config directory for CLI usage.
pub fn default_config_dir() -> PathBuf {
    dirs_next().unwrap_or_else(|| PathBuf::from(".plumise-agent"))
}

fn dirs_next() -> Option<PathBuf> {
    // ~/.plumise-agent/
    #[cfg(unix)]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".plumise-agent"))
    }
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .ok()
            .map(|h| PathBuf::from(h).join("plumise-agent"))
    }
}
