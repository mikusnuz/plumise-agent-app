use serde::Deserialize;

#[derive(Deserialize)]
struct ChatCompletionResponse {
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    completion_tokens: Option<u64>,
}

/// Run a quick benchmark against local llama-server.
/// Sends a short prompt, measures tokens/second from the response.
pub async fn run_benchmark(client: &reqwest::Client, port: u16) -> Result<f64, String> {
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", port);

    let body = serde_json::json!({
        "model": "default",
        "messages": [{"role": "user", "content": "Count from 1 to 50, one number per line."}],
        "max_tokens": 256,
        "temperature": 0.0,
        "stream": false,
    });

    let start = std::time::Instant::now();

    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("Benchmark request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Benchmark HTTP {}", resp.status()));
    }

    let elapsed = start.elapsed().as_secs_f64();
    let data: ChatCompletionResponse = resp.json().await
        .map_err(|e| format!("Benchmark parse error: {}", e))?;

    let tokens = data.usage
        .and_then(|u| u.completion_tokens)
        .unwrap_or(0);

    if tokens == 0 || elapsed < 0.1 {
        return Err("Benchmark produced no tokens or too fast".to_string());
    }

    let tok_per_sec = tokens as f64 / elapsed;
    log::info!("Benchmark: {} tokens in {:.2}s = {:.2} tok/s", tokens, elapsed, tok_per_sec);

    Ok(tok_per_sec)
}
