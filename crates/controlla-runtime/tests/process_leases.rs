use controlla_runtime::process_lease::{LeaseCoordinator, LeaseKey};
use std::process::Command;
use std::time::Duration;

#[test]
fn two_process_mutation_race_serializes() {
    let coord = test_coordinator();
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
    let coord = test_coordinator();
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
    let coord = test_coordinator();
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
    coord.retain_on_unknown(&key, "p1");
    // still held
    assert!(
        coord
            .try_acquire(key, "p2", Duration::from_secs(10), false)
            .is_err()
    );
}

#[test]
fn file_lock_excludes_a_separate_process() {
    let dir = std::env::temp_dir().join(format!(
        "controlla-process-lock-child-{}-{}",
        std::process::id(),
        getrandom::u32().unwrap()
    ));
    let coordinator = LeaseCoordinator::with_lock_dir(&dir);
    let key = key();
    let _lease = coordinator
        .try_acquire(key, "parent", Duration::from_secs(30), false)
        .unwrap();

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cross_process_lock_child", "--nocapture"])
        .env("CONTROLLA_LOCK_CHILD_DIR", &dir)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn separate_processes_can_mutate_independent_tabs_concurrently() {
    let dir = std::env::temp_dir().join(format!(
        "controlla-independent-process-lock-child-{}-{}",
        std::process::id(),
        getrandom::u32().unwrap()
    ));
    let coordinator = LeaseCoordinator::with_lock_dir(&dir);
    let _lease = coordinator
        .try_acquire(key(), "parent", Duration::from_secs(30), false)
        .unwrap();

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "independent_tab_lock_child", "--nocapture"])
        .env("CONTROLLA_LOCK_CHILD_DIR", &dir)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn cross_process_lock_child() {
    let Ok(dir) = std::env::var("CONTROLLA_LOCK_CHILD_DIR") else {
        return;
    };
    let coordinator = LeaseCoordinator::with_lock_dir(dir);
    assert!(
        coordinator
            .try_acquire(key(), "child", Duration::from_secs(30), false)
            .is_err()
    );
}

#[test]
fn independent_tab_lock_child() {
    let Ok(dir) = std::env::var("CONTROLLA_LOCK_CHILD_DIR") else {
        return;
    };
    let coordinator = LeaseCoordinator::with_lock_dir(dir);
    assert!(
        coordinator
            .try_acquire(
                independent_tab_key(),
                "child",
                Duration::from_secs(30),
                false
            )
            .is_ok()
    );
}

fn test_coordinator() -> LeaseCoordinator {
    let dir = std::env::temp_dir().join(format!(
        "controlla-process-leases-{}-{}",
        std::process::id(),
        getrandom::u32().unwrap()
    ));
    LeaseCoordinator::with_lock_dir(dir)
}

fn key() -> LeaseKey {
    LeaseKey {
        browser_instance: "browser".into(),
        target_id: "target".into(),
        document_epoch: "document".into(),
        account_epoch: "account".into(),
        effect_class: "mutation".into(),
    }
}

fn independent_tab_key() -> LeaseKey {
    LeaseKey {
        browser_instance: "browser".into(),
        target_id: "independent-tab".into(),
        document_epoch: "document".into(),
        account_epoch: "account".into(),
        effect_class: "mutation".into(),
    }
}
