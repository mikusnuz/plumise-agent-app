use k256::ecdsa::SigningKey;
use reqwest::Client;
use super::crypto::{address_from_key, keccak256};

pub struct TxSender {
    pub rpc_url: String,
    pub chain_id: u64,
    pub signing_key: SigningKey,
    pub address: String,
    pub client: Client,
}

impl TxSender {
    pub fn new(rpc_url: String, chain_id: u64, signing_key: SigningKey) -> Self {
        let address = address_from_key(&signing_key);
        Self {
            rpc_url,
            chain_id,
            signing_key,
            address,
            client: Client::new(),
        }
    }

    /// Send a transaction and return the tx hash
    pub async fn send_tx(&self, to: [u8; 20], data: Vec<u8>, gas: u64) -> Result<String, String> {
        let nonce = self.get_nonce().await?;
        let gas_price = self.get_gas_price().await?;

        // RLP encode for signing: [nonce, gasPrice, gasLimit, to, value=0, data, chainId, 0, 0]
        let sign_payload = rlp_encode_list(&[
            rlp_encode_u64(nonce),
            rlp_encode_u64(gas_price),
            rlp_encode_u64(gas),
            rlp_encode_bytes(&to),
            rlp_encode_u64(0), // value = 0
            rlp_encode_bytes(&data),
            rlp_encode_u64(self.chain_id),
            rlp_encode_u64(0),
            rlp_encode_u64(0),
        ]);

        let tx_hash = keccak256(&sign_payload);

        let (sig, rec_id) = self
            .signing_key
            .sign_prehash_recoverable(&tx_hash)
            .map_err(|e| format!("Signing failed: {}", e))?;

        let sig_bytes = sig.to_bytes();
        let r = strip_leading_zeros(&sig_bytes[..32]);
        let s = strip_leading_zeros(&sig_bytes[32..64]);
        let v = self.chain_id * 2 + 35 + rec_id.to_byte() as u64;

        // RLP encode signed tx: [nonce, gasPrice, gasLimit, to, value=0, data, v, r, s]
        let signed_tx = rlp_encode_list(&[
            rlp_encode_u64(nonce),
            rlp_encode_u64(gas_price),
            rlp_encode_u64(gas),
            rlp_encode_bytes(&to),
            rlp_encode_u64(0),
            rlp_encode_bytes(&data),
            rlp_encode_u64(v),
            rlp_encode_bytes(&r),
            rlp_encode_bytes(&s),
        ]);

        let raw_tx = format!("0x{}", hex::encode(&signed_tx));

        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_sendRawTransaction",
                "params": [raw_tx],
                "id": 1
            }))
            .send()
            .await
            .map_err(|e| format!("RPC request failed: {}", e))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("RPC response parse error: {}", e))?;

        if let Some(error) = json.get("error") {
            return Err(format!("RPC error: {}", error));
        }

        json["result"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "No result in sendRawTransaction response".to_string())
    }

    /// Wait for a transaction receipt, return true if status=1
    pub async fn wait_receipt(&self, tx_hash: &str, timeout_secs: u64) -> Result<bool, String> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed().as_secs() > timeout_secs {
                return Err("Transaction receipt timeout".to_string());
            }

            let resp = self
                .client
                .post(&self.rpc_url)
                .json(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "eth_getTransactionReceipt",
                    "params": [tx_hash],
                    "id": 1
                }))
                .send()
                .await
                .map_err(|e| format!("RPC request failed: {}", e))?;

            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| format!("RPC response parse error: {}", e))?;

            if let Some(result) = json.get("result") {
                if !result.is_null() {
                    let status = result["status"].as_str().unwrap_or("0x0");
                    return Ok(status == "0x1");
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    async fn get_nonce(&self) -> Result<u64, String> {
        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionCount",
                "params": [&self.address, "latest"],
                "id": 1
            }))
            .send()
            .await
            .map_err(|e| format!("RPC request failed: {}", e))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("RPC response parse error: {}", e))?;

        parse_hex_u64(&json["result"])
    }

    async fn get_gas_price(&self) -> Result<u64, String> {
        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_gasPrice",
                "params": [],
                "id": 1
            }))
            .send()
            .await
            .map_err(|e| format!("RPC request failed: {}", e))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("RPC response parse error: {}", e))?;

        parse_hex_u64(&json["result"])
    }
}

fn parse_hex_u64(value: &serde_json::Value) -> Result<u64, String> {
    let hex_str = value.as_str().ok_or("Expected hex string in RPC response")?;
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    if hex_str.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(hex_str, 16).map_err(|e| format!("Invalid hex u64: {}", e))
}

// ---- Manual RLP encoding (no external crate needed) ----

fn strip_leading_zeros(data: &[u8]) -> Vec<u8> {
    let start = data.iter().position(|&b| b != 0).unwrap_or(data.len());
    if start == data.len() {
        vec![]
    } else {
        data[start..].to_vec()
    }
}

fn u64_to_be_trimmed(value: u64) -> Vec<u8> {
    if value == 0 {
        return vec![];
    }
    let bytes = value.to_be_bytes();
    strip_leading_zeros(&bytes)
}

/// RLP-encode a u64 integer
fn rlp_encode_u64(value: u64) -> Vec<u8> {
    rlp_encode_bytes(&u64_to_be_trimmed(value))
}

/// RLP-encode a byte string
fn rlp_encode_bytes(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return vec![0x80];
    }
    if data.len() == 1 && data[0] < 0x80 {
        return vec![data[0]];
    }
    if data.len() <= 55 {
        let mut result = vec![0x80 + data.len() as u8];
        result.extend_from_slice(data);
        result
    } else {
        let len_bytes = u64_to_be_trimmed(data.len() as u64);
        let mut result = vec![0xb7 + len_bytes.len() as u8];
        result.extend_from_slice(&len_bytes);
        result.extend_from_slice(data);
        result
    }
}

/// RLP-encode a list of already-encoded items
fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(item);
    }
    if payload.len() <= 55 {
        let mut result = vec![0xc0 + payload.len() as u8];
        result.extend_from_slice(&payload);
        result
    } else {
        let len_bytes = u64_to_be_trimmed(payload.len() as u64);
        let mut result = vec![0xf7 + len_bytes.len() as u8];
        result.extend_from_slice(&len_bytes);
        result.extend_from_slice(&payload);
        result
    }
}
