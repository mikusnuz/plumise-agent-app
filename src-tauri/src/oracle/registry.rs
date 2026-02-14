use k256::ecdsa::SigningKey;
use serde::Serialize;
use crate::chain::crypto::{address_from_key, personal_sign};

/// Register this node with the Oracle
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

    let http_endpoint = format!("http://{}:{}", external_ip, http_port);

    // Field order MUST match Oracle's verifyRegistrationSignature
    // serde serializes struct fields in declaration order
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SignData {
        address: String,
        grpc_endpoint: String,
        http_endpoint: String,
        model: String,
        ram_mb: u64,
        device: String,
        vram_mb: u64,
        timestamp: u64,
    }

    let sign_data = SignData {
        address: address.clone(),
        grpc_endpoint: http_endpoint.clone(), // no gRPC, reuse HTTP endpoint
        http_endpoint,
        model: model.to_string(),
        ram_mb,
        device: device.to_string(),
        vram_mb,
        timestamp,
    };

    // Compact JSON (no spaces) — matches Python json.dumps(data, separators=(",",":"))
    let message = serde_json::to_string(&sign_data)
        .map_err(|e| format!("JSON serialize error: {}", e))?;
    let signature = personal_sign(&message, signing_key)?;

    // Build payload: sign_data fields + signature
    let mut payload = serde_json::to_value(&sign_data)
        .map_err(|e| format!("JSON value error: {}", e))?;
    payload
        .as_object_mut()
        .unwrap()
        .insert("signature".to_string(), serde_json::Value::String(signature));

    let url = format!("{}/api/v1/pipeline/register", oracle_url.trim_end_matches('/'));

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
            log::info!("Registered with Oracle successfully");
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

/// Report ready to the Oracle
pub async fn report_ready(
    client: &reqwest::Client,
    oracle_url: &str,
    signing_key: &SigningKey,
    model: &str,
) -> Result<(), String> {
    let address = address_from_key(signing_key);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs();

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SignData {
        address: String,
        model: String,
        timestamp: u64,
    }

    let sign_data = SignData {
        address: address.clone(),
        model: model.to_string(),
        timestamp,
    };

    let message = serde_json::to_string(&sign_data)
        .map_err(|e| format!("JSON serialize error: {}", e))?;
    let signature = personal_sign(&message, signing_key)?;

    let mut payload = serde_json::to_value(&sign_data)
        .map_err(|e| format!("JSON value error: {}", e))?;
    payload
        .as_object_mut()
        .unwrap()
        .insert("signature".to_string(), serde_json::Value::String(signature));

    let url = format!("{}/api/v1/pipeline/ready", oracle_url.trim_end_matches('/'));

    let resp = client
        .post(&url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Oracle ready request failed: {}", e))?;

    if resp.status().is_success() {
        log::info!("Reported ready to Oracle");
        Ok(())
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Oracle ready failed ({}): {}", status, &text[..text.len().min(300)]))
    }
}
