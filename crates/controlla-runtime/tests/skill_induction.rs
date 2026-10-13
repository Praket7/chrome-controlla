use controlla_runtime::skill_induction::{
    FailureClass, SkillEvidenceSet, SkillTrace, TraceStep, induce_candidate,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

fn success(id: &str, input: &str) -> SkillTrace {
    SkillTrace {
        run_id: id.into(),
        site_scope: "https://example.test".into(),
        intent_fingerprint: "draft-form".into(),
        structural_signature: "form:v1".into(),
        input_fingerprint: input.into(),
        verified: true,
        preconditions: BTreeSet::from(["form_present".into()]),
        postconditions: BTreeSet::from(["draft_saved".into()]),
        steps: vec![TraceStep {
            fingerprint: "verified-template:form_fill_save:2".into(),
            action: json!({"kind":"form_fill_save","fields":[{"slot":0},{"slot":1}],"confirmation":{"selector":"#draft-status","text":"Draft saved"}}),
        }],
        failure: None,
    }
}

#[test]
fn induction_requires_three_verified_varied_successes_and_known_failure_guards() {
    let evidence = SkillEvidenceSet {
        traces: vec![
            success("r1", "a"),
            success("r2", "b"),
            success("r3", "c"),
            SkillTrace::failure(
                "f1",
                "https://example.test",
                "draft-form",
                "form:v1",
                FailureClass::ReadFailure,
                "save_control_missing",
            ),
        ],
    };
    let candidate = induce_candidate(&evidence).unwrap();
    assert_eq!(candidate.definition.workflow["kind"], "form_fill_save");
    assert_eq!(
        candidate.definition.workflow["fields"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        candidate.definition.parameter_names,
        vec!["value_0", "value_1"]
    );
    assert_eq!(candidate.definition.structural_signatures, vec!["form:v1"]);
    assert_eq!(
        candidate.failure_guards,
        vec!["read_failure:save_control_missing"]
    );
    assert!(
        candidate
            .definition
            .preconditions
            .contains(&"form_present".into())
    );
}

#[test]
fn generic_receipt_summaries_cannot_become_unexecutable_skill_candidates() {
    let mut traces = vec![success("r1", "a"), success("r2", "b"), success("r3", "c")];
    for trace in &mut traces {
        trace.steps[0].action =
            json!({"kind":"fill","target_count":1,"verified":true,"route":"semantic"});
    }
    traces.push(SkillTrace::failure(
        "f1",
        "https://example.test",
        "draft-form",
        "form:v1",
        FailureClass::ReadFailure,
        "save_control_missing",
    ));
    assert!(induce_candidate(&SkillEvidenceSet { traces }).is_none());
}

#[test]
fn induction_refuses_unknown_mutation_traces_and_insufficient_evidence() {
    let mut evidence = SkillEvidenceSet {
        traces: vec![
            success("r1", "a"),
            success("r2", "b"),
            success("r3", "c"),
            SkillTrace::failure(
                "f1",
                "https://example.test",
                "draft-form",
                "form:v1",
                FailureClass::UnknownMutation,
                "unknown",
            ),
        ],
    };
    assert!(induce_candidate(&evidence).is_none());
    evidence.traces.pop();
    assert!(induce_candidate(&evidence).is_none());
}

#[test]
fn induction_rejects_failure_guards_from_an_unrelated_site_or_intent() {
    let mut unrelated_site = SkillTrace::failure(
        "f1",
        "https://attacker.test",
        "draft-form",
        "form:v1",
        FailureClass::ReadFailure,
        "save_control_missing",
    );
    unrelated_site.structural_signature = "form:v1".into();
    let evidence = SkillEvidenceSet {
        traces: vec![
            success("r1", "a"),
            success("r2", "b"),
            success("r3", "c"),
            unrelated_site,
        ],
    };
    assert!(induce_candidate(&evidence).is_none());

    let mut unrelated_intent = SkillTrace::failure(
        "f2",
        "https://example.test",
        "transfer-funds",
        "form:v1",
        FailureClass::NotDispatched,
        "save_control_missing",
    );
    unrelated_intent.structural_signature = "form:v1".into();
    let evidence = SkillEvidenceSet {
        traces: vec![
            success("r1", "a"),
            success("r2", "b"),
            success("r3", "c"),
            unrelated_intent,
        ],
    };
    assert!(induce_candidate(&evidence).is_none());

    let mut unrelated_structure = SkillTrace::failure(
        "f3",
        "https://example.test",
        "draft-form",
        "form:v2",
        FailureClass::ReadFailure,
        "save_control_missing",
    );
    unrelated_structure.structural_signature = "form:v2".into();
    let evidence = SkillEvidenceSet {
        traces: vec![
            success("r1", "a"),
            success("r2", "b"),
            success("r3", "c"),
            unrelated_structure,
        ],
    };
    assert!(induce_candidate(&evidence).is_none());
}

#[test]
fn structured_traces_persist_and_runtime_induction_stays_a_candidate() {
    let directory = std::env::temp_dir().join(format!(
        "controlla-trace-induction-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let trace_path = directory.join("traces.json");
    let skills_path = directory.join("skills.json");
    let traces = controlla_runtime::skill_induction::SkillTraceStore::open(&trace_path).unwrap();
    for trace in [success("r1", "a"), success("r2", "b"), success("r3", "c")] {
        traces.record(trace).unwrap();
    }
    traces
        .record(SkillTrace::failure(
            "f1",
            "https://example.test",
            "draft-form",
            "form:v1",
            FailureClass::ReadFailure,
            "save_control_missing",
        ))
        .unwrap();

    let restarted = controlla_runtime::skill_induction::SkillTraceStore::open(&trace_path).unwrap();
    let runtime = controlla_runtime::skill_runtime::SkillRuntime::new(&skills_path);
    let record = runtime
        .induce_from_traces(&restarted, "https://example.test", "draft-form")
        .unwrap()
        .unwrap();
    assert_eq!(
        record.status,
        controlla_runtime::skills::SkillStatus::Candidate
    );
    assert!(
        runtime
            .store()
            .get(&record.definition.skill_id)
            .unwrap()
            .is_some()
    );
    runtime
        .record_training_example(&record.definition.skill_id, "train-a", "input-a")
        .unwrap();
    runtime
        .record_training_example(&record.definition.skill_id, "train-b", "input-b")
        .unwrap();
    runtime
        .record_validation_example(&record.definition.skill_id, "validation-c", "input-c")
        .unwrap();
    runtime
        .qualify(
            &record.definition.skill_id,
            controlla_runtime::verifier::Verification::Passed,
            100,
            200,
        )
        .unwrap();
    let replay = runtime
        .replay(
            &record.definition.skill_id,
            "https://example.test",
            "form:v1",
            101,
        )
        .unwrap();
    assert_eq!(replay.workflow["kind"], "form_fill_save");
    assert_eq!(replay.workflow["fields"], json!([{"slot":0},{"slot":1}]));
    assert_eq!(
        replay.failure_guards,
        vec!["read_failure:save_control_missing"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}
