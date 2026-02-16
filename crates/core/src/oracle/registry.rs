use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use crate::chain::crypto::{address_from_key, personal_sign};

/// Cluster assignment returned by Oracle on registration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterAssignment {
    pub mode: String,            // "standalone" | "rpc-server" | "coordinator"
    pub cluster_id: Option<String>,
    pub rpc_port: u16,
    pub rpc_peers: Option<Vec<String>>, // coordinator only: ["192.168.0.101:50052", ...]
}

/// Registration response from Oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub success: bool,
    pub message: String,
    pub assignment: Option<ClusterAssignment>,
}

/// Register this node with the Oracle as a standalone inference node.
/// Returns the cluster assignment if Oracle assigns us to a cluster.
pub async fn register(
    client: &reqwest::Client,
    oracle_url: &str,
    signing_key: &SigningKey,
    model: &str,
    http_port: u16,
    ram_mb: u64,
    vram_mb: u64,
    device: &str,
    external_ip: &str,
    benchmark_tok_per_sec: f64,
    can_distribute: bool,
    lan_ip: &str,
) -> Result<Option<ClusterAssignment>, String> {
    let address = address_from_key(signing_key);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs();

    let endpoint = format!("http://{}:{}", external_ip, http_port);
    let capabilities = vec!["inference".to_string(), model.to_string()];

    // Field order MUST match Oracle's NodesService.verifyRegistrationSignature
    #[derive(Serialize)]
    struct SignData {
        address: String,
        endpoint: String,
        capabilities: Vec<String>,
        timestamp: u64,
    }

    let sign_data = SignData {
        address: address.clone(),
        endpoint: endpoint.clone(),
        capabilities: capabilities.clone(),
        timestamp,
    };

    let message = serde_json::to_string(&sign_data)
        .map_err(|e| format!("JSON serialize error: {}", e))?;
    let signature = personal_sign(&message, signing_key)?;

    // Build payload with distributed inference fields
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RegisterPayload {
        address: String,
        endpoint: String,
        capabilities: Vec<String>,
        timestamp: u64,
        signature: String,
        benchmark_tok_per_sec: f64,
        lan_ip: String,
        can_distribute: bool,
    }

    let payload = RegisterPayload {
        address,
        endpoint,
        capabilities,
        timestamp,
        signature,
        benchmark_tok_per_sec,
        lan_ip: lan_ip.to_string(),
        can_distribute,
    };

    let url = format!("{}/api/nodes/register", oracle_url.trim_end_matches('/'));

    let resp = client
        .post(&url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Oracle register request failed: {}", e))?;

    if resp.status().is_success() {
        let data: RegisterResponse = resp
            .json()
            .await
            .map_err(|e| format!("Oracle response parse error: {}", e))?;

        if data.success {
            log::info!("Registered with Oracle (assignment: {:?})", data.assignment);
            Ok(data.assignment)
        } else {
            Err(format!("Oracle registration rejected: {}", data.message))
        }
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Oracle register failed ({}): {}", status, &text[..text.len().min(300)]))
    }
}
