use controlla_runtime::capability::{
    AvailabilitySource, CapabilityRegistry, PolicySnapshot, ProviderSnapshot,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn catalog_and_dispatch_read_registry_owned_revisions_and_refuse_revoked_origin() {
    let mut policy = PolicySnapshot::default();
    policy.allow_origin("https://example.test");
    policy.grant("principal-a", "https://example.test");
    let registry = CapabilityRegistry::new(ProviderSnapshot::fixture(true, false), policy);
    let catalog = registry.catalog("principal-a", "https://example.test", &["click"]);
    assert_eq!(catalog[0].provider_source, AvailabilitySource::Fixture);
    assert!(catalog[0].qualified);
    let revision = catalog[0].policy_revision;

    registry.revoke_origin("https://example.test");
    let called = AtomicBool::new(false);
    let outcome = registry.dispatch("principal-a", "https://example.test", "click", || {
        called.store(true, Ordering::SeqCst);
    });
    assert!(!called.load(Ordering::SeqCst));
    assert!(!outcome.decision.qualified);
    assert!(outcome.decision.policy_revision > revision);
}

#[test]
fn registry_reuses_evaluator_and_dispatch_runs_authorized_handler() {
    let mut policy = PolicySnapshot::default();
    policy.allow_origin("https://example.test");
    policy.grant("principal-a", "https://example.test");
    let registry = CapabilityRegistry::new(ProviderSnapshot::fixture(false, true), policy);
    let catalog = registry.catalog("principal-a", "https://example.test", &["observe"]);
    let outcome = registry.dispatch("principal-a", "https://example.test", "observe", || 42);
    assert_eq!(catalog[0], outcome.decision);
    assert_eq!(outcome.result, Some(42));
}

#[test]
fn revocation_waits_for_admitted_primitive_then_blocks_next_dispatch() {
    let mut policy = PolicySnapshot::default();
    policy.allow_origin("https://example.test");
    policy.grant("principal-a", "https://example.test");
    let registry = std::sync::Arc::new(CapabilityRegistry::new(
        ProviderSnapshot::fixture(true, false),
        policy,
    ));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let dispatch_registry = registry.clone();
    let dispatch = std::thread::spawn(move || {
        dispatch_registry.dispatch("principal-a", "https://example.test", "click", || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
    });
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let (revoked_tx, revoked_rx) = mpsc::channel();
    let revoke_registry = registry.clone();
    let revoke = std::thread::spawn(move || {
        let revision = revoke_registry.revoke_origin("https://example.test");
        revoked_tx.send(revision).unwrap();
    });
    assert!(matches!(
        revoked_rx.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release_tx.send(()).unwrap();
    assert!(dispatch.join().unwrap().decision.qualified);
    let revision = revoked_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    revoke.join().unwrap();
    let called = AtomicBool::new(false);
    let next = registry.dispatch("principal-a", "https://example.test", "click", || {
        called.store(true, Ordering::SeqCst)
    });
    assert!(!next.decision.qualified);
    assert_eq!(next.decision.policy_revision, revision);
    assert!(!called.load(Ordering::SeqCst));
}
