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
    pub principal: String,
    pub session: String,
    pub target: String,
    pub revision: String,
    pub observer: String,
    pub observed_at_ms: u64,
    pub predicate_hash: String,
    pub scope: EvidenceScope,
    pub state: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceBinding {
    pub principal: String,
    pub session: String,
    pub target: String,
    pub revision: String,
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
    if evidence.principal != binding.principal
        || evidence.session != binding.session
        || evidence.target != binding.target
        || evidence.revision != binding.revision
        || evidence.scope != EvidenceScope::IndependentState
        || evidence.observer.is_empty()
        || evidence.observed_at_ms > now_ms
        || now_ms.saturating_sub(evidence.observed_at_ms) > max_age_ms
        || evidence.predicate_hash != predicate.fingerprint()
        || matches!(predicate, Predicate::Visual)
    {
        return Verification::Inconclusive;
    }

    let matched = match predicate {
        Predicate::FieldEq { path, expected } => {
            path_value(&evidence.state, path) == Some(expected)
        }
        Predicate::ObjectExists {
            path,
            field,
            expected,
        } => path_value(&evidence.state, path)
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.get(field) == Some(expected))),
        Predicate::StateEq { expected } => &evidence.state == expected,
        Predicate::Visual => return Verification::Inconclusive,
    };
    if matched {
        Verification::Passed
    } else {
        Verification::Failed
    }
}

fn path_value<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return None;
    }
    path.split('.').try_fold(value, |node, part| node.get(part))
}
