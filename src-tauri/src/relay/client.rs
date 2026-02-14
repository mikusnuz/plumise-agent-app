use futures_util::{SinkExt, StreamExt};
use k256::ecdsa::SigningKey;
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::chain::crypto::{address_from_key, personal_sign};

#[derive(Debug, Deserialize)]
struct ServerMessage {
    r#type: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    messages: Vec<ChatMessage>,
    #[serde(default, rename = "maxTokens")]
    max_tokens: u32,
    #[serde(default)]
    temperature: f64,
    #[serde(default, rename = "topP")]
    top_p: f64,
    #[serde(default)]
    stream: bool,
    #[allow(dead_code)]
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// Start the WebSocket relay client as a background task.
/// Connects to the inference API, authenticates, and proxies inference requests
/// to the local llama-server.
pub fn start_relay(
    ws_url: String,
    signing_key: SigningKey,
    model: String,
    llama_port: u16,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut backoff = 1u64;

        loop {
            log::info!("Connecting to relay: {}", ws_url);

            match run_relay(&ws_url, &signing_key, &model, llama_port, &client).await {
                Ok(()) => {
                    log::info!("Relay connection closed normally");
                    backoff = 1;
                }
                Err(e) => {
                    log::warn!("Relay connection error: {}", e);
                }
            }

            // Exponential backoff: 1s, 2s, 4s, 8s, ..., 60s max
            log::info!("Reconnecting in {}s...", backoff);
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(60);
        }
    })
}

async fn run_relay(
    ws_url: &str,
    signing_key: &SigningKey,
    model: &str,
    llama_port: u16,
    http_client: &reqwest::Client,
) -> Result<(), String> {
    // Connect
    let (ws_stream, _) = connect_async(ws_url)
        .await
        .map_err(|e| format!("WebSocket connect failed: {}", e))?;

    let (mut write, mut read) = ws_stream.split();

    // Authenticate
    let address = address_from_key(signing_key);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("Time error: {}", e))?
        .as_secs();

    let sign_message = serde_json::json!({
        "address": address,
        "model": model,
        "timestamp": timestamp,
    })
    .to_string();

    let signature = personal_sign(&sign_message, signing_key)?;

    let auth_msg = serde_json::json!({
        "type": "auth",
        "address": address,
        "model": model,
        "timestamp": timestamp,
        "signature": signature,
    });

    write
        .send(Message::Text(auth_msg.to_string()))
        .await
        .map_err(|e| format!("Auth send failed: {}", e))?;

    // Wait for auth response (with timeout)
    let auth_response = tokio::time::timeout(std::time::Duration::from_secs(10), read.next())
        .await
        .map_err(|_| "Auth response timeout".to_string())?
        .ok_or("Connection closed during auth")?
        .map_err(|e| format!("Auth read error: {}", e))?;

    let auth_text = auth_response
        .to_text()
        .map_err(|e| format!("Auth response not text: {}", e))?;

    let auth_resp: serde_json::Value =
        serde_json::from_str(auth_text).map_err(|e| format!("Auth response parse error: {}", e))?;

    if auth_resp["type"].as_str() != Some("auth_ok") {
        let msg = auth_resp["message"].as_str().unwrap_or("unknown error");
        return Err(format!("Auth rejected: {}", msg));
    }

    log::info!("Relay authenticated as {}", address);

    // Spawn ping task
    let ping_write = std::sync::Arc::new(tokio::sync::Mutex::new(write));
    let ping_writer = ping_write.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let msg = serde_json::json!({"type": "ping"}).to_string();
            let mut w = ping_writer.lock().await;
            if w.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Message loop
    while let Some(msg_result) = read.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                log::warn!("WS read error: {}", e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let server_msg: ServerMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        log::warn!("Failed to parse server message: {}", e);
                        continue;
                    }
                };

                match server_msg.r#type.as_str() {
                    "request" => {
                        let req_id = server_msg.id.clone();
                        let writer = ping_write.clone();
                        let client = http_client.clone();
                        let port = llama_port;
                        let messages = server_msg.messages;
                        let max_tokens = server_msg.max_tokens;
                        let temperature = server_msg.temperature;
                        let top_p = server_msg.top_p;
                        let stream = server_msg.stream;

                        // Process request in background to not block message loop
                        tokio::spawn(async move {
                            if stream {
                                handle_stream_request(
                                    &client, port, &req_id, &messages, max_tokens,
                                    temperature, top_p, &writer,
                                )
                                .await;
                            } else {
                                handle_request(
                                    &client, port, &req_id, &messages, max_tokens,
                                    temperature, top_p, &writer,
                                )
                                .await;
                            }
                        });
                    }
                    "pong" => {
                        // Heartbeat response, ignore
                    }
                    other => {
                        log::debug!("Unknown server message type: {}", other);
                    }
                }
            }
            Message::Close(_) => {
                log::info!("Server closed connection");
                break;
            }
            Message::Ping(data) => {
                let mut w = ping_write.lock().await;
                let _ = w.send(Message::Pong(data)).await;
            }
            _ => {}
        }
    }

    ping_task.abort();
    Ok(())
}

type WsWriter = std::sync::Arc<
    tokio::sync::Mutex<
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
    >,
>;

async fn handle_request(
    client: &reqwest::Client,
    llama_port: u16,
    req_id: &str,
    messages: &[ChatMessage],
    max_tokens: u32,
    temperature: f64,
    top_p: f64,
    writer: &WsWriter,
) {
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", llama_port);

    let openai_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content,
            })
        })
        .collect();

    let body = serde_json::json!({
        "messages": openai_messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "top_p": top_p,
        "stream": false,
    });

    match client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<serde_json::Value>().await {
                    Ok(data) => {
                        let response = serde_json::json!({
                            "type": "response",
                            "id": req_id,
                            "choices": data["choices"],
                            "usage": data["usage"],
                        });
                        let mut w = writer.lock().await;
                        let _ = w.send(Message::Text(response.to_string())).await;
                    }
                    Err(e) => {
                        send_error(writer, req_id, &format!("Response parse error: {}", e)).await;
                    }
                }
            } else {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                send_error(
                    writer,
                    req_id,
                    &format!("llama-server error {}: {}", status, &text[..text.len().min(200)]),
                )
                .await;
            }
        }
        Err(e) => {
            send_error(writer, req_id, &format!("llama-server request failed: {}", e)).await;
        }
    }
}

async fn handle_stream_request(
    client: &reqwest::Client,
    llama_port: u16,
    req_id: &str,
    messages: &[ChatMessage],
    max_tokens: u32,
    temperature: f64,
    top_p: f64,
    writer: &WsWriter,
) {
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", llama_port);

    let openai_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content,
            })
        })
        .collect();

    let body = serde_json::json!({
        "messages": openai_messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "top_p": top_p,
        "stream": true,
    });

    let resp = match client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            send_error(writer, req_id, &format!("llama-server stream failed: {}", e)).await;
            return;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        send_error(
            writer,
            req_id,
            &format!("llama-server stream error {}: {}", status, &text[..text.len().min(200)]),
        )
        .await;
        return;
    }

    // Parse SSE stream
    use tokio::io::AsyncBufReadExt;
    let stream = resp.bytes_stream();
    use futures_util::TryStreamExt;
    let reader = tokio_util::io::StreamReader::new(
        stream.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
    );
    let mut lines = tokio::io::BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }

        match serde_json::from_str::<serde_json::Value>(data) {
            Ok(parsed) => {
                if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                    if !content.is_empty() {
                        let chunk = serde_json::json!({
                            "type": "chunk",
                            "id": req_id,
                            "content": content,
                        });
                        let mut w = writer.lock().await;
                        if w.send(Message::Text(chunk.to_string())).await.is_err() {
                            return;
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }

    // Send done
    let done = serde_json::json!({
        "type": "done",
        "id": req_id,
    });
    let mut w = writer.lock().await;
    let _ = w.send(Message::Text(done.to_string())).await;
}

async fn send_error(writer: &WsWriter, req_id: &str, message: &str) {
    log::error!("Relay request {} error: {}", req_id, message);
    let error = serde_json::json!({
        "type": "error",
        "id": req_id,
        "message": message,
    });
    let mut w = writer.lock().await;
    let _ = w.send(Message::Text(error.to_string())).await;
}
