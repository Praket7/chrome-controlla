use controlla_runtime::process_lease::{LeaseCoordinator, LeaseKey};
use std::time::Duration;

#[test]
fn two_process_mutation_race_serializes() {
    let coord = LeaseCoordinator::new();
    let key = LeaseKey {
        browser_instance: "b".into(),
        target_id: "t".into(),
        document_epoch: "d".into(),
        account_epoch: "a".into(),
        effect_class: "mutation".into(),
    };
    let _l1 = coord
        .try_acquire(key.clone(), "p1", Duration::from_secs(10), false)
        .unwrap();
    assert!(
        coord
            .try_acquire(key.clone(), "p2", Duration::from_secs(10), false)
            .is_err()
    );
}

#[test]
fn read_read_may_share() {
    let coord = LeaseCoordinator::new();
    let key = LeaseKey {
        browser_instance: "b".into(),
        target_id: "t".into(),
        document_epoch: "d".into(),
        account_epoch: "a".into(),
        effect_class: "mutation".into(),
    };
    let _r1 = coord
        .try_acquire(key.clone(), "p1", Duration::from_secs(10), true)
        .unwrap();
    assert!(
        coord
            .try_acquire(key, "p2", Duration::from_secs(10), true)
            .is_ok()
    );
}

#[test]
fn unknown_retains_lease() {
    let coord = LeaseCoordinator::new();
    let key = LeaseKey {
        browser_instance: "b".into(),
        target_id: "t".into(),
        document_epoch: "d".into(),
        account_epoch: "a".into(),
        effect_class: "mutation".into(),
    };
    let _l = coord
        .try_acquire(key.clone(), "p1", Duration::from_secs(60), false)
        .unwrap();
    coord.retain_on_unknown(&key);
    // still held
    assert!(
        coord
            .try_acquire(key, "p2", Duration::from_secs(10), false)
            .is_err()
    );
}
