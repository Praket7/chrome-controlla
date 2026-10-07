use controlla_runtime::{artifacts::ArtifactExpectation, verifier::*};
use serde_json::json;

fn evidence(predicate: &Predicate) -> Evidence {
    Evidence {
        operation_id: "op-1".into(),
        provenance_id: "observer-proof-1".into(),
        principal: "alice".into(),
        session: "session-1".into(),
        target: "tab-1".into(),
        revision: "rev-7".into(),
        app_signature: "fixture-app-v3".into(),
        account_id: "account-alice".into(),
        observer: "independent-readback".into(),
        observed_at_ms: 1_000,
        predicate_hash: predicate.fingerprint(),
        scope: EvidenceScope::IndependentState,
        state: json!({"document":{"title":"Saved"},"objects":[{"id":"shape-1"}]}),
    }
}

fn verify_for(
    target: &str,
    revision: &str,
    now_ms: u64,
    predicate: &Predicate,
    evidence: &Evidence,
) -> Verification {
    verify(
        &EvidenceBinding {
            operation_id: "op-1".into(),
            provenance_id: "observer-proof-1".into(),
            principal: "alice".into(),
            session: "session-1".into(),
            target: target.into(),
            revision: revision.into(),
            app_signature: "fixture-app-v3".into(),
            account_id: "account-alice".into(),
        },
        now_ms,
        500,
        predicate,
        evidence,
    )
}

#[test]
fn rejects_fake_success_and_stale_or_unbound_evidence() {
    let predicate = Predicate::field_eq("document.title", json!("Saved"));
    let mut claim = evidence(&predicate);
    claim.scope = EvidenceScope::PageClaim;
    assert_eq!(
        verify_for("tab-1", "rev-7", 1_100, &predicate, &claim),
        Verification::Inconclusive
    );

    let current = evidence(&predicate);
    assert_eq!(
        verify_for("tab-2", "rev-7", 1_100, &predicate, &current),
        Verification::Inconclusive
    );
    assert_eq!(
        verify_for("tab-1", "rev-8", 1_100, &predicate, &current),
        Verification::Inconclusive
    );
    assert_eq!(
        verify_for("tab-1", "rev-7", 2_000, &predicate, &current),
        Verification::Inconclusive
    );
    let altered = Predicate::field_eq("document.title", json!("Draft"));
    assert_eq!(
        verify_for("tab-1", "rev-7", 1_100, &altered, &current),
        Verification::Inconclusive
    );
    assert_eq!(
        verify_for("tab-1", "rev-7", 1_100, &altered, &evidence(&altered)),
        Verification::Inconclusive
    );
}

#[test]
fn caller_cannot_relabel_page_evidence_or_omit_operation_provenance() {
    let predicate = Predicate::field_eq("document.title", json!("Saved"));
    let mut forged = evidence(&predicate);
    forged.scope = EvidenceScope::IndependentState;
    assert_eq!(
        verify_for("tab-1", "rev-7", 1_100, &predicate, &forged),
        Verification::Inconclusive
    );
    let serialized = serde_json::to_value(&forged).unwrap();
    assert!(serialized.get("operation_id").is_some());
    assert!(serialized.get("provenance_id").is_some());
}

#[test]
fn caller_supplied_state_never_claims_verified_success() {
    for predicate in [
        Predicate::field_eq("document.title", json!("Saved")),
        Predicate::object_exists("objects", "id", json!("shape-1")),
        Predicate::state_eq(json!({"document":{"title":"Saved"},"objects":[{"id":"shape-1"}]})),
    ] {
        assert_eq!(
            verify_for("tab-1", "rev-7", 1_100, &predicate, &evidence(&predicate)),
            Verification::Inconclusive
        );
    }
    let visual = Predicate::Visual;
    assert_eq!(
        verify_for("tab-1", "rev-7", 1_100, &visual, &evidence(&visual)),
        Verification::Inconclusive
    );
}

#[test]
fn artifact_validation_rejects_truncated_download() {
    let correct = ArtifactExpectation {
        byte_length: 7,
        sha256: "15a596e3c98c407e043751ff3b21ff0358a1bdfdf3fe948b1523893a8e5de2e8".into(),
    };
    assert!(correct.matches(b"correct"));
    assert!(!correct.matches(b"corrupt"));
    assert!(!correct.matches(b"corre"));
    let registered = ArtifactExpectation::from_bytes(b"correct");
    assert_eq!(registered, correct);
    assert!(!registered.matches(b"corrupt"));
    assert!(!registered.matches(b"corre"));
}
