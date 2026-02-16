use super::tx::TxSender;

const fn precompile_addr(id: u8) -> [u8; 20] {
    let mut addr = [0u8; 20];
    addr[19] = id;
    addr
}

const PRECOMPILE_REGISTER: [u8; 20] = precompile_addr(0x21);
const PRECOMPILE_HEARTBEAT: [u8; 20] = precompile_addr(0x22);

/// Register agent on-chain via precompile 0x21
/// Input: name(32B) + modelHash(32B) + capCount(32B)
pub async fn register_agent(
    tx_sender: &TxSender,
    name: &str,
    model_hash: [u8; 32],
) -> Result<String, String> {
    let mut name_bytes = [0u8; 32];
    let name_raw = name.as_bytes();
    let copy_len = name_raw.len().min(32);
    name_bytes[..copy_len].copy_from_slice(&name_raw[..copy_len]);

    let cap_count = [0u8; 32]; // no capabilities

    let mut data = Vec::with_capacity(96);
    data.extend_from_slice(&name_bytes);
    data.extend_from_slice(&model_hash);
    data.extend_from_slice(&cap_count);

    let tx_hash = tx_sender.send_tx(PRECOMPILE_REGISTER, data, 300_000).await?;
    let success = tx_sender.wait_receipt(&tx_hash, 30).await?;
    if success {
        log::info!("Agent registration tx succeeded: {}", tx_hash);
        Ok(tx_hash)
    } else {
        Err("Registration transaction failed (status=0)".to_string())
    }
}

/// Send heartbeat via precompile 0x22 (empty data, uses msg.sender)
pub async fn heartbeat(tx_sender: &TxSender) -> Result<String, String> {
    let tx_hash = tx_sender.send_tx(PRECOMPILE_HEARTBEAT, vec![], 100_000).await?;
    let success = tx_sender.wait_receipt(&tx_hash, 30).await?;
    if success {
        log::debug!("Heartbeat tx succeeded: {}", tx_hash);
        Ok(tx_hash)
    } else {
        Err("Heartbeat transaction failed (status=0)".to_string())
    }
}
