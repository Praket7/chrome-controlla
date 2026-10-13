use controlla_runtime::{evidence_store::PersistentEvidenceStore, visual_probe::EvidenceTrail};
use std::time::{SystemTime, UNIX_EPOCH};

fn path() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("controlla-evidence-{nonce}/evidence.json"))
}

#[test]
fn verified_task_evidence_persists_across_store_instances_and_stays_scoped() {
    let path = path();
    let mut trail = EvidenceTrail::default();
    trail.record(
        "commit",
        Some("@c1".into()),
        None,
        "invoice total is 42",
        true,
    );
    trail.record("failure", None, None, "reason=stale_target", false);
    let store = PersistentEvidenceStore::new(&path);
    store.replace("scope-a", &trail).unwrap();
    let reopened = PersistentEvidenceStore::new(&path);
    let restored = reopened.recent_relevant("scope-a", "invoice total", 8);
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].fact, "invoice total is 42");
    assert!(restored[0].verified);
    assert!(
        reopened
            .recent_relevant("scope-b", "invoice total", 8)
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
