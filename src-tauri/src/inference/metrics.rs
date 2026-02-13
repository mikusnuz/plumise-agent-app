use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMetrics {
    pub total_tokens: u64,
    pub total_requests: u64,
    pub prompt_tokens: u64,
    pub avg_latency: f64,
    pub tps: f64,
    pub uptime: u64,
    pub slots_processing: u64,
}

/// Fetch and parse llama-server /metrics (Prometheus text format)
pub async fn fetch_metrics(
    client: &reqwest::Client,
    port: u16,
) -> Result<InferenceMetrics, String> {
    let url = format!("http://127.0.0.1:{}/metrics", port);

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map_err(|e| format!("Metrics fetch failed: {}", e))?;

    let text = resp
        .text()
        .await
        .map_err(|e| format!("Metrics read failed: {}", e))?;

    parse_prometheus(&text)
}

/// Parse Prometheus text format from llama-server
fn parse_prometheus(text: &str) -> Result<InferenceMetrics, String> {
    let mut metrics = InferenceMetrics::default();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse "metric_name{labels} value" or "metric_name value"
        let (name, value_str) = if let Some(brace_end) = line.find('}') {
            let name_end = line.find('{').unwrap_or(0);
            let name = &line[..name_end];
            let value = line[brace_end + 1..].trim();
            (name, value)
        } else {
            let mut parts = line.splitn(2, ' ');
            let name = parts.next().unwrap_or("");
            let value = parts.next().unwrap_or("0");
            (name, value)
        };

        let value_f64: f64 = value_str.parse().unwrap_or(0.0);

        match name {
            "llamacpp:tokens_predicted_total" | "llamacpp_tokens_predicted_total" => {
                metrics.total_tokens = value_f64 as u64;
            }
            "llamacpp:prompt_tokens_processed_total"
            | "llamacpp_prompt_tokens_processed_total" => {
                metrics.prompt_tokens = value_f64 as u64;
            }
            "llamacpp:requests_processing" | "llamacpp_requests_processing" => {
                metrics.slots_processing = value_f64 as u64;
            }
            "llamacpp:prompt_seconds_total" | "llamacpp_prompt_seconds_total" => {
                // avg_latency approximation
                if metrics.total_requests > 0 {
                    metrics.avg_latency = value_f64 / metrics.total_requests as f64;
                }
            }
            "llamacpp:tokens_predicted_seconds_total"
            | "llamacpp_tokens_predicted_seconds_total" => {
                // tokens per second
                if value_f64 > 0.0 {
                    metrics.tps = metrics.total_tokens as f64 / value_f64;
                }
            }
            _ => {}
        }
    }

    // total_requests = prompt_tokens events (approximate)
    // llama-server doesn't have a direct request counter in all versions
    metrics.total_requests = metrics.total_requests.max(if metrics.total_tokens > 0 { 1 } else { 0 });

    Ok(metrics)
}
