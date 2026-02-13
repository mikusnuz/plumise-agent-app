use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};

/// Compute keccak256 hash
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut output = [0u8; 32];
    output.copy_from_slice(&result);
    output
}

/// EIP-191 personal_sign — returns "0x"-prefixed hex signature (65 bytes = r+s+v)
pub fn personal_sign(message: &str, signing_key: &SigningKey) -> Result<String, String> {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut data = Vec::new();
    data.extend_from_slice(prefix.as_bytes());
    data.extend_from_slice(message.as_bytes());
    let hash = keccak256(&data);

    let (sig, rec_id) = signing_key
        .sign_prehash_recoverable(&hash)
        .map_err(|e| format!("Signing failed: {}", e))?;

    let mut result = [0u8; 65];
    let sig_bytes = sig.to_bytes();
    result[..64].copy_from_slice(&sig_bytes);
    result[64] = rec_id.to_byte() + 27;

    Ok(format!("0x{}", hex::encode(result)))
}

/// Derive Ethereum address from signing key (lowercase, 0x-prefixed)
pub fn address_from_key(signing_key: &SigningKey) -> String {
    let public_key = signing_key.verifying_key();
    let public_key_bytes = public_key.to_encoded_point(false);
    let hash = keccak256(&public_key_bytes.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}

/// Parse a hex private key string ("0x"-prefixed or raw) to SigningKey
pub fn parse_private_key(hex_key: &str) -> Result<SigningKey, String> {
    let hex_str = hex_key.strip_prefix("0x").unwrap_or(hex_key);
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid hex: {}", e))?;
    SigningKey::from_bytes((&bytes[..]).into())
        .map_err(|e| format!("Invalid private key: {}", e))
}
