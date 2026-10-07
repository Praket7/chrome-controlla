use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const TEST_OBSERVER_KEY: &[u8] = b"controlla-phase7-test-observer-only";

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
    if !matches_binding(binding, now_ms, max_age_ms, predicate, evidence) {
        return Verification::Inconclusive;
    }
    // Caller-supplied evidence cannot prove which observer produced it.
    Verification::Inconclusive
}

fn matches_binding(
    binding: &EvidenceBinding,
    now_ms: u64,
    max_age_ms: u64,
    predicate: &Predicate,
    evidence: &Evidence,
) -> bool {
    evidence.operation_id == binding.operation_id
        && evidence.provenance_id == binding.provenance_id
        && evidence.principal == binding.principal
        && evidence.session == binding.session
        && evidence.target == binding.target
        && evidence.revision == binding.revision
        && evidence.app_signature == binding.app_signature
        && evidence.account_id == binding.account_id
        && evidence.scope == EvidenceScope::IndependentState
        && !evidence.observer.is_empty()
        && evidence.observed_at_ms <= now_ms
        && now_ms.saturating_sub(evidence.observed_at_ms) <= max_age_ms
        && evidence.predicate_hash == predicate.fingerprint()
}

/// Evidence received from a runtime-owned observer boundary. The private receipt prevents
/// callers from turning a page claim into independently observed state.
pub struct ObserverEvidence {
    evidence: Evidence,
    receipt: String,
}

impl ObserverEvidence {
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

pub trait TrustedObserver {
    fn observe(&self, binding: &EvidenceBinding, predicate: &Predicate)
    -> Option<ObserverEvidence>;
}

pub fn verify_observed(
    observer: &impl TrustedObserver,
    binding: &EvidenceBinding,
    now_ms: u64,
    max_age_ms: u64,
    predicate: &Predicate,
) -> Verification {
    let Some(observed) = observer.observe(binding, predicate) else {
        return Verification::Inconclusive;
    };
    if !valid_receipt(&observed) {
        return Verification::Inconclusive;
    }
    if !matches_binding(binding, now_ms, max_age_ms, predicate, &observed.evidence) {
        return Verification::Inconclusive;
    }
    evaluate(predicate, &observed.evidence.state)
}

fn valid_receipt(observed: &ObserverEvidence) -> bool {
    let bytes = serde_json::to_vec(&observed.evidence).expect("evidence serializes");
    observed.receipt == receipt(&bytes)
}

fn receipt(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(TEST_OBSERVER_KEY);
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn evaluate(predicate: &Predicate, state: &Value) -> Verification {
    let matches = match predicate {
        Predicate::FieldEq { path, expected } => {
            state.pointer(&json_pointer(path)) == Some(expected)
        }
        Predicate::ObjectExists {
            path,
            field,
            expected,
        } => state
            .pointer(&json_pointer(path))
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.get(field) == Some(expected))),
        Predicate::StateEq { expected } => state == expected,
        Predicate::Visual => return Verification::Inconclusive,
    };
    if matches {
        Verification::Passed
    } else {
        Verification::Failed
    }
}

fn json_pointer(path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!(
            "/{}",
            path.split('.')
                .map(|part| part.replace('~', "~0").replace('/', "~1"))
                .collect::<Vec<_>>()
                .join("/")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FixtureObserver(Evidence);
    impl TrustedObserver for FixtureObserver {
        fn observe(&self, _: &EvidenceBinding, _: &Predicate) -> Option<ObserverEvidence> {
            let bytes = serde_json::to_vec(&self.0).ok()?;
            Some(ObserverEvidence {
                evidence: self.0.clone(),
                receipt: receipt(&bytes),
            })
        }
    }

    // Ground truth is held separately from the candidate's page claim. This observer is
    // test-only; production still has no trusted app-state or persistence observer.
    struct GroundTruthFixture {
        state: Value,
        revision: String,
        observed_at_ms: u64,
    }

    impl TrustedObserver for GroundTruthFixture {
        fn observe(
            &self,
            binding: &EvidenceBinding,
            predicate: &Predicate,
        ) -> Option<ObserverEvidence> {
            let evidence = Evidence {
                operation_id: binding.operation_id.clone(),
                provenance_id: binding.provenance_id.clone(),
                principal: binding.principal.clone(),
                session: binding.session.clone(),
                target: binding.target.clone(),
                revision: self.revision.clone(),
                app_signature: binding.app_signature.clone(),
                account_id: binding.account_id.clone(),
                observer: "independent-offline-fixture".into(),
                observed_at_ms: self.observed_at_ms,
                predicate_hash: predicate.fingerprint(),
                scope: EvidenceScope::IndependentState,
                state: self.state.clone(),
            };
            let bytes = serde_json::to_vec(&evidence).ok()?;
            Some(ObserverEvidence {
                evidence,
                receipt: receipt(&bytes),
            })
        }
    }

    fn binding() -> EvidenceBinding {
        EvidenceBinding {
            operation_id: "op".into(),
            provenance_id: "proof".into(),
            principal: "alice".into(),
            session: "s".into(),
            target: "tab".into(),
            revision: "r1".into(),
            app_signature: "app".into(),
            account_id: "acct".into(),
        }
    }
    fn evidence(predicate: &Predicate) -> Evidence {
        Evidence {
            operation_id: "op".into(),
            provenance_id: "proof".into(),
            principal: "alice".into(),
            session: "s".into(),
            target: "tab".into(),
            revision: "r1".into(),
            app_signature: "app".into(),
            account_id: "acct".into(),
            observer: "fixture".into(),
            observed_at_ms: 100,
            predicate_hash: predicate.fingerprint(),
            scope: EvidenceScope::IndependentState,
            state: json!({"document":{"title":"Saved"},"objects":[{"id":"shape"}]}),
        }
    }

    #[test]
    fn runtime_observer_receipt_gates_predicate_evaluation() {
        for predicate in [
            Predicate::field_eq("document.title", json!("Saved")),
            Predicate::object_exists("objects", "id", json!("shape")),
            Predicate::state_eq(json!({"document":{"title":"Saved"},"objects":[{"id":"shape"}]})),
        ] {
            assert_eq!(
                verify_observed(
                    &FixtureObserver(evidence(&predicate)),
                    &binding(),
                    150,
                    100,
                    &predicate
                ),
                Verification::Passed
            );
        }
        let changed = Predicate::field_eq("document.title", json!("Draft"));
        assert_eq!(
            verify_observed(
                &FixtureObserver(evidence(&changed)),
                &binding(),
                150,
                100,
                &changed
            ),
            Verification::Failed
        );
    }

    #[test]
    fn stale_wrong_binding_and_tampered_receipts_are_inconclusive() {
        let predicate = Predicate::field_eq("document.title", json!("Saved"));
        for (target, revision, account, observed_at_ms) in [
            ("other", "r1", "acct", 100),
            ("tab", "r2", "acct", 100),
            ("tab", "r1", "other", 100),
            ("tab", "r1", "acct", 0),
        ] {
            let mut item = evidence(&predicate);
            item.target = target.into();
            item.revision = revision.into();
            item.account_id = account.into();
            item.observed_at_ms = observed_at_ms;
            assert_eq!(
                verify_observed(&FixtureObserver(item), &binding(), 150, 100, &predicate),
                Verification::Inconclusive
            );
        }
        struct Tampered(Evidence);
        impl TrustedObserver for Tampered {
            fn observe(&self, _: &EvidenceBinding, _: &Predicate) -> Option<ObserverEvidence> {
                let bytes = serde_json::to_vec(&self.0).ok()?;
                Some(ObserverEvidence {
                    evidence: self.0.clone(),
                    receipt: receipt(&bytes).replace('a', "b"),
                })
            }
        }
        assert_eq!(
            verify_observed(
                &Tampered(evidence(&predicate)),
                &binding(),
                150,
                100,
                &predicate
            ),
            Verification::Inconclusive
        );
        assert_eq!(
            verify(&binding(), 150, 100, &predicate, &evidence(&predicate)),
            Verification::Inconclusive
        );
    }

    #[test]
    fn offline_ground_truth_catches_fake_save_and_control_semantic_drift() {
        let saved = Predicate::field_eq("document.title", json!("Saved"));
        let binding = binding();
        let mut persuasive_claim = evidence(&saved);
        persuasive_claim.state = json!({"document":{"title":"Saved"}});
        assert_eq!(
            verify(&binding, 150, 100, &saved, &persuasive_claim),
            Verification::Inconclusive
        );
        assert_eq!(
            verify_observed(
                &GroundTruthFixture {
                    state: json!({"document":{"title":"Draft"}}),
                    revision: "r1".into(),
                    observed_at_ms: 140,
                },
                &binding,
                150,
                100,
                &saved,
            ),
            Verification::Failed
        );

        let publish = Predicate::field_eq("controls.publish.action", json!("publish"));
        assert_eq!(
            verify_observed(
                &GroundTruthFixture {
                    state: json!({"controls":{"publish":{"label":"Publish","action":"delete"}}}),
                    revision: "r1".into(),
                    observed_at_ms: 140,
                },
                &binding,
                150,
                100,
                &publish,
            ),
            Verification::Failed
        );
    }

    #[test]
    fn offline_ground_truth_rejects_old_evidence_and_revision_drift() {
        let predicate = Predicate::field_eq("document.title", json!("Saved"));
        let binding = binding();
        let old_revision = GroundTruthFixture {
            state: json!({"document":{"title":"Saved"}}),
            revision: "r0".into(),
            observed_at_ms: 140,
        };
        assert_eq!(
            verify_observed(&old_revision, &binding, 150, 100, &predicate),
            Verification::Inconclusive
        );
        let stale_observation = GroundTruthFixture {
            revision: "r1".into(),
            observed_at_ms: 0,
            ..old_revision
        };
        assert_eq!(
            verify_observed(&stale_observation, &binding, 150, 100, &predicate),
            Verification::Inconclusive
        );
    }
}
