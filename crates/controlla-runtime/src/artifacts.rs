use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactExpectation {
    pub byte_length: u64,
    pub sha256: String,
}

impl ArtifactExpectation {
    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.byte_length
            && Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
                == self.sha256
    }
}
