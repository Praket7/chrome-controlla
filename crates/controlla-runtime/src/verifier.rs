use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Predicate {
    FieldEq {
        path: String,
        expected: Value,
    },
    ObjectExists {
        path: String,
        field: String,
        expected: Value,
    },
    StateEq {
        expected: Value,
    },
    Visual,
}

impl Predicate {
    pub fn field_eq(path: impl Into<String>, expected: Value) -> Self {
        Self::FieldEq {
            path: path.into(),
            expected,
        }
    }

    pub fn object_exists(
        path: impl Into<String>,
        field: impl Into<String>,
        expected: Value,
    ) -> Self {
        Self::ObjectExists {
            path: path.into(),
            field: field.into(),
            expected,
        }
    }

    pub fn state_eq(expected: Value) -> Self {
        Self::StateEq { expected }
    }

    pub fn fingerprint(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("predicate serializes");
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScope {
    IndependentState,
    PageClaim,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Evidence {
    pub operation_id: String,
    pub provenance_id: String,
    pub principal: String,
    pub session: String,
    pub target: String,
    pub revision: String,
    pub app_signature: String,
    pub account_id: String,
    pub observer: String,
    pub observed_at_ms: u64,
    pub predicate_hash: String,
    pub scope: EvidenceScope,
    pub state: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceBinding {
    pub operation_id: String,
    pub provenance_id: String,
    pub principal: String,
    pub session: String,
    pub target: String,
    pub revision: String,
    pub app_signature: String,
    pub account_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    Passed,
    Failed,
    Inconclusive,
}

pub fn verify(
    binding: &EvidenceBinding,
    now_ms: u64,
    max_age_ms: u64,
    predicate: &Predicate,
    evidence: &Evidence,
) -> Verification {
    if evidence.operation_id != binding.operation_id
        || evidence.provenance_id != binding.provenance_id
        || evidence.principal != binding.principal
        || evidence.session != binding.session
        || evidence.target != binding.target
        || evidence.revision != binding.revision
        || evidence.app_signature != binding.app_signature
        || evidence.account_id != binding.account_id
        || evidence.scope != EvidenceScope::IndependentState
        || evidence.observer.is_empty()
        || evidence.observed_at_ms > now_ms
        || now_ms.saturating_sub(evidence.observed_at_ms) > max_age_ms
        || evidence.predicate_hash != predicate.fingerprint()
    {
        return Verification::Inconclusive;
    }
    // No runtime-controlled app observer exists yet. A caller-provided scope or observer label
    // cannot establish that evidence is independent, so no external receipt may pass here.
    Verification::Inconclusive
}
