use controlla_runtime::{artifacts::ArtifactExpectation, cache::*, verifier::*};
use serde_json::json;

fn evidence(predicate: &Predicate) -> Evidence {
    Evidence {
        principal: "alice".into(),
        session: "session-1".into(),
        target: "tab-1".into(),
        revision: "rev-7".into(),
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
            principal: "alice".into(),
            session: "session-1".into(),
            target: target.into(),
            revision: revision.into(),
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
        Verification::Failed
    );
}

#[test]
fn verifies_field_object_and_exact_state_but_not_visual_criteria() {
    for predicate in [
        Predicate::field_eq("document.title", json!("Saved")),
        Predicate::object_exists("objects", "id", json!("shape-1")),
        Predicate::state_eq(json!({"document":{"title":"Saved"},"objects":[{"id":"shape-1"}]})),
    ] {
        assert_eq!(
            verify_for("tab-1", "rev-7", 1_100, &predicate, &evidence(&predicate)),
            Verification::Passed
        );
    }
    let visual = Predicate::Visual;
    assert_eq!(
        verify_for("tab-1", "rev-7", 1_100, &visual, &evidence(&visual)),
        Verification::Inconclusive
    );
}

#[test]
fn workflow_cache_is_bound_to_authority_target_revision_time_and_provenance() {
    let mut cache = WorkflowCache::default();
    let workflow = qualified_workflow();
    assert!(cache.insert(workflow.clone()));
    let context = LookupContext {
        principal: "alice".into(),
        session: "session-1".into(),
        target: "tab-1".into(),
        revision: "rev-7".into(),
        environment: "chrome-154/linux".into(),
        authority: "edit:fixture.test".into(),
        predicate_hash: "predicate-hash".into(),
        now_ms: 1_500,
    };
    assert!(matches!(
        cache.lookup(&workflow.signature, &context),
        CacheLookup::Hit(_)
    ));

    for drift in [
        LookupContext {
            target: "tab-2".into(),
            ..context.clone()
        },
        LookupContext {
            revision: "rev-8".into(),
            ..context.clone()
        },
        LookupContext {
            principal: "bob".into(),
            ..context.clone()
        },
        LookupContext {
            authority: "read:fixture.test".into(),
            ..context.clone()
        },
        LookupContext {
            session: "session-2".into(),
            ..context.clone()
        },
        LookupContext {
            environment: "chrome-155/linux".into(),
            ..context.clone()
        },
        LookupContext {
            predicate_hash: "changed-control-semantics".into(),
            ..context.clone()
        },
        LookupContext {
            now_ms: 2_001,
            ..context.clone()
        },
    ] {
        assert!(matches!(
            cache.lookup(&workflow.signature, &drift),
            CacheLookup::Miss(_)
        ));
    }
    cache.quarantine(&workflow.signature, "false success");
    assert!(matches!(
        cache.lookup(&workflow.signature, &context),
        CacheLookup::Quarantined(_)
    ));
    assert!(!cache.insert(QualifiedWorkflow {
        schema_version: 2,
        ..workflow
    }));
    let mut missing_provenance = qualified_workflow();
    missing_provenance.provenance.validation = None;
    assert!(!cache.insert(missing_provenance));
}

#[test]
fn artifact_validation_rejects_truncated_download() {
    let expected = ArtifactExpectation {
        byte_length: 8,
        sha256: "0000000000000000000000000000000000000000000000000000000000000000".into(),
    };
    assert!(!expected.matches(b"short"));
}

fn qualified_workflow() -> QualifiedWorkflow {
    QualifiedWorkflow {
        signature: "sig-v1".into(),
        schema_version: 1,
        principal: "alice".into(),
        session: "session-1".into(),
        target: "tab-1".into(),
        revision: "rev-7".into(),
        environment: "chrome-154/linux".into(),
        authority: "edit:fixture.test".into(),
        predicate_hash: "predicate-hash".into(),
        qualified_at_ms: 1_000,
        expires_at_ms: 2_000,
        provenance: Provenance {
            training: Some("train-fixture-1".into()),
            validation: Some("heldout-fixture-1".into()),
        },
    }
}
