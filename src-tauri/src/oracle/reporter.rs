use k256::ecdsa::SigningKey;
use serde::Serialize;
use crate::chain::crypto::{address_from_key, personal_sign};
use crate::inference::metrics::InferenceMetrics;

/// Registration parameters needed for periodic re-registration
#[derive(Clone)]
pub struct RegistrationParams {
    pub model: String,
    pub http_port: u16,
    pub ram_mb: u64,
    pub vram_mb: u64,
    pub device: String,
    pub external_ip: String,
}

/// Start a background metrics reporter task (60s interval).
///
/// - Reports metrics every 60s as keepalive for Oracle node tracking.
/// - Re-registers every 5 minutes at /api/nodes/register (standalone node,
///   NOT /api/v1/pipeline/register — agent-app is not a gRPC pipeline participant).
pub fn start_reporter(
    client: reqwest::Client,
    oracle_url: String,
    signing_key: SigningKey,
    llama_port: u16,
    registration: RegistrationParams,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        // Skip the first immediate tick
        interval.tick().await;

        let mut tick_count = 0u64;

        loop {
            interval.tick().await;
            tick_count += 1;

            // Every 5 minutes (every 5th tick), re-register as standalone node
            // to keep node entry alive in Oracle (no pipeline ready needed).
            if tick_count % 5 == 0 {
                match crate::oracle::registry::register(
                    &client,
                    &oracle_url,
                    &signing_key,
                    &registration.model,
                    registration.http_port,
                    registration.ram_mb,
                    registration.vram_mb,
                    &registration.device,
                    &registration.external_ip,
                )
                .await
                {
                    Ok(()) => {
                        log::debug!("Periodic re-registration successful");
                    }
                    Err(e) => {
                        log::warn!("Periodic re-registration failed: {}", e);
                    }
                }
            }

            // Fetch metrics from local llama-server, fall back to zeroed metrics.
            // Even zeroed metrics keep the pipeline_assignments.updatedAt fresh,
            // preventing the Oracle's stale node cleanup from removing us.
            let metrics = match crate::inference::metrics::fetch_metrics(&client, llama_port).await
            {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("Failed to fetch metrics, sending keepalive: {}", e);
                    InferenceMetrics::default()
                }
            };

            if let Err(e) =
                report_metrics(&client, &oracle_url, &signing_key, &metrics).await
            {
                log::warn!("Failed to report metrics: {}", e);
            }
        }
    })
}

async fn report_metrics(
    client: &reqwest::Client,
    oracle_url: &str,
    signing_key: &SigningKey,
    metrics: &InferenceMetrics,
) -> Result<(), String> {
    let address = address_from_key(signing_key);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs();

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ReportPayload {
        address: String,
        total_tokens: u64,
        total_requests: u64,
        avg_latency: f64,
        tps: f64,
        uptime: u64,
        timestamp: u64,
        signature: String,
    }

    // Sign the metrics data
    let sign_msg = format!(
        "{}:{}:{}:{}",
        address, metrics.total_tokens, metrics.total_requests, timestamp
    );
    let signature = personal_sign(&sign_msg, signing_key)?;

    let payload = ReportPayload {
        address,
        total_tokens: metrics.total_tokens,
        total_requests: metrics.total_requests,
        avg_latency: metrics.avg_latency,
        tps: metrics.tps,
        uptime: metrics.uptime,
        timestamp,
        signature,
    };

    let url = format!(
        "{}/api/v1/metrics/report",
        oracle_url.trim_end_matches('/')
    );

    let resp = client
        .post(&url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Metrics report failed: {}", e))?;

    if resp.status().is_success() {
        log::debug!("Metrics reported successfully");
        Ok(())
    } else {
        Err(format!("Metrics report HTTP {}", resp.status()))
    }
}
