use sha2::{Digest, Sha256};

/// A configured credential is bound to one principal and one endpoint audience.
/// The token itself is never retained after construction.
#[derive(Clone)]
pub struct Credential {
    token_digest: [u8; 32],
    pub principal: String,
    pub audience: String,
}

impl Credential {
    pub fn configured(token: &str, principal: &str, audience: &str) -> Option<Self> {
        if token.is_empty() || principal.is_empty() || audience.is_empty() {
            return None;
        }
        Some(Self {
            token_digest: Sha256::digest(token.as_bytes()).into(),
            principal: principal.to_owned(),
            audience: audience.to_owned(),
        })
    }

    pub fn authorize(
        &self,
        presented: Option<&str>,
        audience: &str,
        target_principal: &str,
    ) -> bool {
        let Some(token) = presented else { return false };
        let candidate: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let difference = self
            .token_digest
            .iter()
            .zip(candidate)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        difference == 0 && self.audience == audience && self.principal == target_principal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_requires_exact_configured_principal_and_audience() {
        let credential =
            Credential::configured("offline-test-token", "tenant-a", "mcp://local").unwrap();
        assert!(!credential.authorize(None, "mcp://local", "tenant-a"));
        assert!(!credential.authorize(Some("wrong"), "mcp://local", "tenant-a"));
        assert!(!credential.authorize(Some("offline-test-token"), "mcp://other", "tenant-a"));
        assert!(!credential.authorize(Some("offline-test-token"), "mcp://local", "tenant-b"));
        assert!(credential.authorize(Some("offline-test-token"), "mcp://local", "tenant-a"));
        assert!(Credential::configured("", "tenant-a", "mcp://local").is_none());
    }
}
