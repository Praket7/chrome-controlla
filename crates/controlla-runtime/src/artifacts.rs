use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactExpectation {
    pub byte_length: u64,
    pub sha256: String,
}

impl ArtifactExpectation {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            byte_length: bytes.len() as u64,
            sha256: Self::sha256(bytes),
        }
    }

    pub fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.byte_length && Self::sha256(bytes) == self.sha256
    }
}
