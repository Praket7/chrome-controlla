use controlla_runtime::{
    runtime_observer::{
        RuntimeObservation, VerificationPolicy, VerificationStrength, validate_observation,
    },
    skill_runtime::{SkillRuntime, SkillRuntimeError},
    skills::{SKILL_SCHEMA_VERSION, SkillDefinition, SkillStatus},
    verifier::Verification,
};
use serde_json::json;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn runtime() -> SkillRuntime {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let unique = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    SkillRuntime::new(std::env::temp_dir().join(format!(
        "controlla-faults-{}-{nonce}-{unique}/skills.json",
        std::process::id()
    )))
}

fn definition() -> SkillDefinition {
    SkillDefinition {
        skill_id: "fault-skill".into(),
        schema_version: SKILL_SCHEMA_VERSION,
        site_scope: "https://example.test".into(),
        intent_fingerprint: "fill".into(),
        parameter_names: vec![],
        preconditions: vec!["ready".into()],
        workflow: json!({"steps":[]}),
        postconditions: vec!["saved".into()],
        structural_signatures: vec!["sig-v1".into()],
        version: 1,
    }
}

fn qualified(runtime: &SkillRuntime) {
    runtime.register_candidate(definition()).unwrap();
    runtime
        .record_training_success("fault-skill", "train")
        .unwrap();
    runtime
        .record_validation_success("fault-skill", "validate")
        .unwrap();
    runtime
        .qualify("fault-skill", Verification::Passed, 10, 10_000)
        .unwrap();
}

#[test]
fn structural_drift_is_quarantined_instead_of_blindly_replayed() {
    let runtime = runtime();
    qualified(&runtime);
    assert!(matches!(
        runtime.replay("fault-skill", "https://example.test", "sig-v2", 20),
        Err(SkillRuntimeError::NotQualified)
    ));
    assert_eq!(
        runtime.status("fault-skill", 20).unwrap().status,
        SkillStatus::Quarantined
    );
    let _ = std::fs::remove_dir_all(runtime.store().path().parent().unwrap());
}

#[test]
fn severe_safety_failure_quarantines_immediately() {
    let runtime = runtime();
    qualified(&runtime);
    let record = runtime
        .record_runtime_verification("fault-skill", Verification::Passed, true, false)
        .unwrap();
    assert_eq!(record.status, SkillStatus::Quarantined);
    assert!(
        runtime
            .replay("fault-skill", "https://example.test", "sig-v1", 20)
            .is_err()
    );
    let _ = std::fs::remove_dir_all(runtime.store().path().parent().unwrap());
}

#[test]
fn wrong_revision_or_stale_observer_evidence_is_inconclusive() {
    let policy = VerificationPolicy {
        max_age_ms: 50,
        minimum_strength: VerificationStrength::FreshPageState,
    };
    let wrong_revision = RuntimeObservation {
        binding_revision: "doc-6".into(),
        observer_id: "runtime".into(),
        observed_at_ms: 100,
        strength: VerificationStrength::ExternalState,
        state: json!({"saved":true}),
    };
    assert_eq!(
        validate_observation(policy, "doc-7", 120, &wrong_revision),
        Verification::Inconclusive
    );
    let stale = RuntimeObservation {
        binding_revision: "doc-7".into(),
        observer_id: "runtime".into(),
        observed_at_ms: 1,
        strength: VerificationStrength::ExternalState,
        state: json!({"saved":true}),
    };
    assert_eq!(
        validate_observation(policy, "doc-7", 120, &stale),
        Verification::Inconclusive
    );
}

#[test]
fn expiry_blocks_replay_without_retrying_mutation() {
    let runtime = runtime();
    qualified(&runtime);
    assert!(matches!(
        runtime.replay("fault-skill", "https://example.test", "sig-v1", 10_000),
        Err(SkillRuntimeError::NotQualified)
    ));
    assert_eq!(
        runtime.status("fault-skill", 10_000).unwrap().status,
        SkillStatus::Expired
    );
    let _ = std::fs::remove_dir_all(runtime.store().path().parent().unwrap());
}
