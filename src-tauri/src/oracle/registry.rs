use k256::ecdsa::SigningKey;
use serde::Serialize;
use crate::chain::crypto::{address_from_key, personal_sign};

/// Register this node with the Oracle as a standalone inference node.
/// Uses /api/nodes/register (NOT /api/v1/pipeline/register) because
/// agent-app is a standalone llama-server node, not a gRPC pipeline participant.
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
) -> Result<(), String> {
    let address = address_from_key(signing_key);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs();

    let endpoint = format!("http://{}:{}", external_ip, http_port);
    let capabilities = vec!["inference".to_string(), model.to_string()];

    // Field order MUST match Oracle's NodesService.verifyRegistrationSignature
    // which signs: JSON.stringify({ address, endpoint, capabilities, timestamp })
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

    // Compact JSON — matches Oracle's JSON.stringify()
    let message = serde_json::to_string(&sign_data)
        .map_err(|e| format!("JSON serialize error: {}", e))?;
    let signature = personal_sign(&message, signing_key)?;

    // Build payload
    #[derive(Serialize)]
    struct RegisterPayload {
        address: String,
        endpoint: String,
        capabilities: Vec<String>,
        timestamp: u64,
        signature: String,
    }

    let payload = RegisterPayload {
        address,
        endpoint,
        capabilities,
        timestamp,
        signature,
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
        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Oracle response parse error: {}", e))?;
        if data.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            log::info!("Registered with Oracle as standalone node");
            Ok(())
        } else {
            let msg = data["message"].as_str().unwrap_or("unknown");
            Err(format!("Oracle registration rejected: {}", msg))
        }
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Oracle register failed ({}): {}", status, &text[..text.len().min(300)]))
    }
}
