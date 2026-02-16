use serde::Serialize;
use crate::chain::crypto::keccak256;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofData {
    pub model_hash: String,
    pub input_hash: String,
    pub output_hash: String,
    pub agent_address: String,
    pub token_count: u64,
    pub proof_hash: String,
}

pub struct InferenceProofGenerator {
    model_hash: [u8; 32],
    agent_address: String,
}

impl InferenceProofGenerator {
    pub fn new(model_name: &str, agent_address: &str) -> Self {
        let model_hash = keccak256(model_name.as_bytes());
        Self {
            model_hash,
            agent_address: agent_address.to_string(),
        }
    }

    pub fn generate_proof(
        &self,
        input_data: &str,
        output_data: &str,
        token_count: u64,
    ) -> ProofData {
        let input_hash = keccak256(input_data.as_bytes());
        let output_hash = keccak256(output_data.as_bytes());

        // proof_hash = keccak256(modelHash || inputHash || outputHash || agent_padded)
        let addr_hex = self
            .agent_address
            .strip_prefix("0x")
            .unwrap_or(&self.agent_address);
        let addr_bytes = hex::decode(addr_hex).unwrap_or_default();
        let mut addr_padded = [0u8; 32];
        if addr_bytes.len() == 20 {
            addr_padded[12..].copy_from_slice(&addr_bytes);
        }

        let mut composite = Vec::with_capacity(128);
        composite.extend_from_slice(&self.model_hash);
        composite.extend_from_slice(&input_hash);
        composite.extend_from_slice(&output_hash);
        composite.extend_from_slice(&addr_padded);

        let proof_hash = keccak256(&composite);

        ProofData {
            model_hash: format!("0x{}", hex::encode(self.model_hash)),
            input_hash: format!("0x{}", hex::encode(input_hash)),
            output_hash: format!("0x{}", hex::encode(output_hash)),
            agent_address: self.agent_address.clone(),
            token_count,
            proof_hash: format!("0x{}", hex::encode(proof_hash)),
        }
    }
}
