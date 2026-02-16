use super::crypto::keccak256;
use super::tx::TxSender;

const REWARD_POOL_ADDRESS: &str = "0000000000000000000000000000000000001000";

fn pool_address() -> [u8; 20] {
    let mut addr = [0u8; 20];
    addr[18] = 0x10;
    addr[19] = 0x00;
    addr
}

/// Submit claimReward() transaction to RewardPool
pub async fn claim_reward(tx_sender: &TxSender) -> Result<String, String> {
    let selector = &keccak256(b"claimReward()")[..4];
    let data = selector.to_vec();

    let tx_hash = tx_sender.send_tx(pool_address(), data, 200_000).await?;
    let success = tx_sender.wait_receipt(&tx_hash, 30).await?;
    if success {
        log::info!("Claim reward tx succeeded: {}", tx_hash);
        Ok(tx_hash)
    } else {
        Err("Claim transaction failed (status=0)".to_string())
    }
}

/// Read pending reward via eth_call (read-only)
pub async fn get_pending_reward(
    client: &reqwest::Client,
    rpc_url: &str,
    agent_address: &str,
) -> Result<u128, String> {
    let selector = &keccak256(b"getPendingReward(address)")[..4];
    let addr_hex = agent_address
        .strip_prefix("0x")
        .unwrap_or(agent_address);
    let addr_bytes =
        hex::decode(addr_hex).map_err(|e| format!("Invalid address hex: {}", e))?;

    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(selector);
    data.extend_from_slice(&[0u8; 12]); // left-pad to 32 bytes
    data.extend_from_slice(&addr_bytes);

    let call_data = format!("0x{}", hex::encode(&data));
    let pool_addr = format!("0x{}", REWARD_POOL_ADDRESS);

    let resp = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [{"to": pool_addr, "data": call_data}, "latest"],
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {}", e))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("RPC response parse error: {}", e))?;

    let result_hex = json["result"]
        .as_str()
        .ok_or("No result in eth_call response")?;
    let result_hex = result_hex.strip_prefix("0x").unwrap_or(result_hex);

    if result_hex.is_empty() || result_hex.chars().all(|c| c == '0') {
        return Ok(0);
    }

    u128::from_str_radix(result_hex, 16)
        .map_err(|e| format!("Failed to parse reward: {}", e))
}
