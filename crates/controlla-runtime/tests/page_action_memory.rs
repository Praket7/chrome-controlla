use controlla_runtime::page_action_memory::{ActionMemory, ActionMemoryEntry, ActionMemoryKey};
use controlla_runtime::verifier::Verification;
use serde_json::json;

#[test]
fn only_verified_actions_are_recalled_for_an_exact_origin_structure_and_intent() {
    let key = ActionMemoryKey {
        origin: "https://example.test".into(),
        structural_signature: "search:v1".into(),
        intent: "search".into(),
    };
    let entry = ActionMemoryEntry {
        semantic_target: json!({"role":"button","name":"Search"}),
        action: "click".into(),
        expected_postcondition: json!({"kind":"text_present","text":"Results"}),
        last_verified_ms: 100,
    };
    let mut memory = ActionMemory::default();
    assert!(!memory.record(key.clone(), entry.clone(), Verification::Inconclusive));
    assert!(memory.lookup(&key, 200).is_none());
    assert!(memory.record(key.clone(), entry, Verification::Passed));
    assert!(memory.lookup(&key, 200).is_some());
    assert_eq!(
        memory
            .lookup_page("https://example.test", "search:v1", 200, 4)
            .len(),
        1
    );
    assert!(
        memory
            .lookup_page("https://other.test", "search:v1", 200, 4)
            .is_empty()
    );
    assert!(
        memory
            .lookup_page("https://example.test", "search:v2", 200, 4)
            .is_empty()
    );
    assert!(
        memory
            .lookup(
                &ActionMemoryKey {
                    origin: "https://other.test".into(),
                    ..key.clone()
                },
                200
            )
            .is_none()
    );
    assert!(
        memory
            .lookup(
                &ActionMemoryKey {
                    structural_signature: "search:v2".into(),
                    ..key.clone()
                },
                200
            )
            .is_none()
    );
    assert!(
        memory
            .lookup(
                &ActionMemoryKey {
                    intent: "delete".into(),
                    ..key
                },
                200
            )
            .is_none()
    );
}

#[test]
fn stale_memory_is_a_hint_that_expires_and_never_executes_by_itself() {
    let key = ActionMemoryKey {
        origin: "https://example.test".into(),
        structural_signature: "v1".into(),
        intent: "search".into(),
    };
    let entry = ActionMemoryEntry {
        semantic_target: json!({"name":"Search"}),
        action: "click".into(),
        expected_postcondition: json!({}),
        last_verified_ms: 10,
    };
    let mut memory = ActionMemory::default();
    memory.record(key.clone(), entry, Verification::Passed);
    assert!(
        memory
            .lookup(
                &key,
                10 + controlla_runtime::page_action_memory::MAX_AGE_MS + 1
            )
            .is_none()
    );
}

#[test]
fn verified_affordance_memory_survives_restart_but_unverified_changes_do_not() {
    let path = std::env::temp_dir().join(format!(
        "controlla-action-memory-{}.json",
        std::process::id()
    ));
    let key = ActionMemoryKey {
        origin: "https://example.test".into(),
        structural_signature: "search:v1".into(),
        intent: "search".into(),
    };
    let entry = ActionMemoryEntry {
        semantic_target: json!({"role":"textbox","name":"Search"}),
        action: "fill".into(),
        expected_postcondition: json!({"kind":"text_present","text":"results"}),
        last_verified_ms: 100,
    };
    let mut first = ActionMemory::open(&path).unwrap();
    assert!(first.record(key.clone(), entry.clone(), Verification::Passed));
    drop(first);
    let mut restarted = ActionMemory::open(&path).unwrap();
    assert!(restarted.lookup(&key, 101).is_some());
    assert!(!restarted.record(key.clone(), entry, Verification::Failed));
    assert!(
        ActionMemory::open(&path)
            .unwrap()
            .lookup(&key, 101)
            .is_none()
    );
    std::fs::remove_file(path).unwrap();
}
