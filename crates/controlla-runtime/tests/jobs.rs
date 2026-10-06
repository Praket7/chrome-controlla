use controlla_runtime::jobs::{Delivery, DispatchClaim, JobStatus, Journal};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp_db() -> std::path::PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "controlla-jobs-{}-{}.sqlite",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn cleanup(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(name);
    }
}

#[derive(Default)]
struct FixtureEndpoint {
    effects: AtomicUsize,
}

impl FixtureEndpoint {
    fn send(&self, claim: DispatchClaim) {
        if claim.acquired {
            self.effects.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
fn admission_is_scoped_durable_and_conflicts_on_changed_request() {
    let path = temp_db();
    cleanup(&path);
    let journal = Journal::open(&path).unwrap();
    assert!(journal.admit("", "s1", "k", &json!({})).is_err());
    let endpoint = FixtureEndpoint::default();
    let first = journal
        .admit(
            "p1",
            "s1",
            "k",
            &json!({"effect":"save","target":"t1","grant":4}),
        )
        .unwrap();
    assert!(!first.replayed);
    let claim = journal
        .record_dispatch("p1", "s1", &first.operation.id, "first-send")
        .unwrap();
    endpoint.send(claim);
    assert!(
        journal
            .acknowledge_dispatch("p1", "s1", &first.operation.id, "first-send")
            .unwrap()
    );
    journal
        .complete("p1", "s1", &first.operation.id, json!({"saved":true}))
        .unwrap();
    assert!(
        journal
            .get("p2", "s1", &first.operation.id)
            .unwrap()
            .is_none()
    );
    assert!(
        journal
            .cancel("p2", "s1", &first.operation.id)
            .is_ok_and(|changed| !changed)
    );
    drop(journal);
    let journal = Journal::open(&path).unwrap();
    let replay = journal
        .admit(
            "p1",
            "s1",
            "k",
            &json!({"grant":4,"target":"t1","effect":"save"}),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.operation.id, replay.operation.id);
    assert_eq!(replay.operation.status, JobStatus::Completed);
    assert!(
        journal
            .record_dispatch("p1", "s1", &replay.operation.id, "replay-send")
            .is_err()
    );
    assert_eq!(endpoint.effects.load(Ordering::SeqCst), 1);
    assert!(
        journal
            .admit(
                "p1",
                "s1",
                "k",
                &json!({"effect":"delete","target":"t1","grant":4})
            )
            .is_err()
    );
    let isolated = journal
        .admit(
            "p2",
            "s1",
            "k",
            &json!({"effect":"save","target":"t1","grant":4}),
        )
        .unwrap();
    assert_ne!(first.operation.id, isolated.operation.id);
    cleanup(&path);
}

#[test]
fn dispatch_crash_is_unknown_and_cannot_be_blindly_replayed() {
    let path = temp_db();
    let _ = std::fs::remove_file(&path);
    let journal = Journal::open(&path).unwrap();
    let admission = journal
        .admit("p", "s", "lost", &json!({"effect":"save"}))
        .unwrap();
    let endpoint = FixtureEndpoint::default();
    endpoint.send(
        journal
            .record_dispatch("p", "s", &admission.operation.id, "request-1")
            .unwrap(),
    );
    assert!(
        journal
            .acknowledge_dispatch("p", "s", &admission.operation.id, "request-1")
            .unwrap()
    );
    drop(journal);
    let journal = Journal::open(&path).unwrap();
    assert_eq!(journal.recover_uncertain("p", "s").unwrap(), 1);
    let recovered = journal
        .get("p", "s", &admission.operation.id)
        .unwrap()
        .unwrap();
    assert!(
        journal
            .get("other", "s", &admission.operation.id)
            .unwrap()
            .is_none()
    );
    assert_eq!(recovered.status, JobStatus::Unknown);
    assert_eq!(recovered.delivery, Delivery::Sent);
    assert!(
        journal
            .admit("p", "s", "lost", &json!({"effect":"save"}))
            .unwrap()
            .replayed
    );
    assert_eq!(endpoint.effects.load(Ordering::SeqCst), 1);
    assert!(
        journal
            .reconcile("p", "s", &admission.operation.id)
            .unwrap()
            .unwrap()
            .result
            .is_none()
    );
    assert!(
        journal
            .record_dispatch("p", "s", &admission.operation.id, "request-1")
            .is_err()
    );
    assert_eq!(endpoint.effects.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn crash_after_claim_before_transport_is_unknown_and_not_retried() {
    let path = temp_db();
    let journal = Journal::open(&path).unwrap();
    let endpoint = FixtureEndpoint::default();
    let admitted = journal
        .admit("p", "s", "claim-crash", &json!({"effect":"save"}))
        .unwrap();
    let claim = journal
        .record_dispatch("p", "s", &admitted.operation.id, "wire-claim")
        .unwrap();
    assert!(claim.acquired);
    drop(journal); // crash before FixtureEndpoint::send

    let recovered = Journal::open(&path).unwrap();
    assert_eq!(recovered.recover_uncertain("p", "s").unwrap(), 1);
    let operation = recovered
        .get("p", "s", &admitted.operation.id)
        .unwrap()
        .unwrap();
    assert_eq!(operation.status, JobStatus::Unknown);
    assert_eq!(operation.delivery, Delivery::Unknown);
    assert!(
        recovered
            .record_dispatch("p", "s", &operation.id, "retry")
            .is_err()
    );
    assert_eq!(endpoint.effects.load(Ordering::SeqCst), 0);
    cleanup(&path);
}

#[test]
fn confirmed_pre_send_failure_is_failed_and_not_sent() {
    let path = temp_db();
    let journal = Journal::open(&path).unwrap();
    let admitted = journal
        .admit("p", "s", "pre-send-fail", &json!({"effect":"save"}))
        .unwrap();
    let claim = journal
        .record_dispatch("p", "s", &admitted.operation.id, "wire-1")
        .unwrap();
    assert!(claim.acquired);
    assert!(
        journal
            .complete("p", "s", &admitted.operation.id, json!({"saved":true}))
            .is_err()
    );
    let uncertain = journal
        .get("p", "s", &admitted.operation.id)
        .unwrap()
        .unwrap();
    assert_eq!(uncertain.status, JobStatus::Running);
    assert_eq!(uncertain.delivery, Delivery::Unknown);
    let failed = journal
        .fail_before_send(
            "p",
            "s",
            &admitted.operation.id,
            json!({"code":"connect_failed_before_send"}),
        )
        .unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.delivery, Delivery::NotSent);
    assert!(
        !journal
            .acknowledge_dispatch("p", "s", &failed.id, "wire-1")
            .unwrap()
    );
    assert!(
        journal
            .record_dispatch("p", "s", &failed.id, "retry")
            .is_err()
    );
    cleanup(&path);
}

#[test]
fn concurrent_identical_admission_shares_one_operation_and_cancel_only_blocks_dispatch() {
    let path = temp_db();
    let journal = Journal::open(&path).unwrap();
    let other = Journal::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for handle in [journal.clone(), other] {
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            handle
                .admit("p", "s", "same", &json!({"effect":"save"}))
                .unwrap()
        }));
    }
    barrier.wait();
    let a = threads.remove(0).join().unwrap();
    let b = threads.remove(0).join().unwrap();
    assert_eq!(a.operation.id, b.operation.id);
    let queued = journal
        .admit("p", "s", "cancel", &json!({"effect":"save"}))
        .unwrap();
    assert!(journal.cancel("p", "s", &queued.operation.id).unwrap());
    assert_eq!(
        journal
            .get("p", "s", &queued.operation.id)
            .unwrap()
            .unwrap()
            .status,
        JobStatus::Cancelled
    );
    let dispatched = journal
        .admit("p", "s", "sent", &json!({"effect":"save"}))
        .unwrap();
    let claim = journal
        .record_dispatch("p", "s", &dispatched.operation.id, "wire-1")
        .unwrap();
    assert!(claim.acquired);
    assert!(!journal.cancel("p", "s", &dispatched.operation.id).unwrap());
    assert_eq!(
        journal
            .get("p", "s", &dispatched.operation.id)
            .unwrap()
            .unwrap()
            .status,
        JobStatus::Running
    );
    cleanup(&path);
}

#[test]
fn concurrent_dispatch_claims_allow_only_one_remote_send() {
    let path = temp_db();
    let journal = Journal::open(&path).unwrap();
    let other = Journal::open(&path).unwrap();
    let admitted = journal
        .admit("p", "s", "claim", &json!({"effect":"save"}))
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for handle in [journal.clone(), other] {
        let barrier = barrier.clone();
        let id = admitted.operation.id.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            handle
                .record_dispatch("p", "s", &id, "one-wire-request")
                .unwrap()
                .acquired
        }));
    }
    barrier.wait();
    let acquisitions = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .filter(|v| *v)
        .count();
    assert_eq!(acquisitions, 1);
    cleanup(&path);
}

#[tokio::test]
async fn work_can_finish_after_the_response_wait_budget() {
    let path = temp_db();
    let _ = std::fs::remove_file(&path);
    let journal = Journal::open(&path).unwrap();
    let accepted = journal
        .admit("p", "s", "slow", &json!({"effect":"read"}))
        .unwrap();
    journal
        .record_dispatch("p", "s", &accepted.operation.id, "fixture-1")
        .unwrap();
    assert!(
        journal
            .acknowledge_dispatch("p", "s", &accepted.operation.id, "fixture-1")
            .unwrap()
    );
    let worker = journal.clone();
    let id = accepted.operation.id.clone();
    let worker_id = id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        worker
            .complete("p", "s", &worker_id, json!({"value":12}))
            .unwrap();
    });
    assert_eq!(
        journal
            .wait("p", "s", &id, accepted.operation.revision + 1, 10)
            .await
            .unwrap()
            .status,
        JobStatus::Running
    );
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    assert_eq!(
        journal.get("p", "s", &id).unwrap().unwrap().status,
        JobStatus::Completed
    );
    cleanup(&path);
}

#[tokio::test]
async fn never_resolving_dispatch_times_out_as_running_and_remains_reconcilable() {
    let path = temp_db();
    let journal = Journal::open(&path).unwrap();
    let accepted = journal
        .admit("p", "s", "never", &json!({"effect":"save"}))
        .unwrap();
    let running = journal
        .record_dispatch("p", "s", &accepted.operation.id, "fixture-never")
        .unwrap();
    assert!(running.acquired);
    let timed_out = journal
        .wait(
            "p",
            "s",
            &running.operation.id,
            running.operation.revision,
            5,
        )
        .await
        .unwrap();
    assert_eq!(timed_out.status, JobStatus::Running);
    assert_eq!(timed_out.delivery, Delivery::Unknown);
    assert!(timed_out.result.is_none());
    assert_eq!(
        journal
            .reconcile("p", "s", &running.operation.id)
            .unwrap()
            .unwrap()
            .status,
        JobStatus::Running
    );
    cleanup(&path);
}

#[tokio::test]
async fn persisted_job_deadline_marks_unsent_and_dispatched_effects_differently() {
    let path = temp_db();
    let journal = Journal::open(&path).unwrap();
    let queued = journal
        .admit_with_deadline("p", "s", "queued-deadline", &json!({"effect":"save"}), 100)
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let queued = journal
        .get("p", "s", &queued.operation.id)
        .unwrap()
        .unwrap();
    assert_eq!(queued.status, JobStatus::Failed);
    assert_eq!(queued.delivery, Delivery::NotSent);
    assert!(
        queued.deadline_at_ms > 0 && queued.deadline_error.as_deref() == Some("deadline_exceeded")
    );
    assert!(
        journal
            .record_dispatch("p", "s", &queued.id, "too-late")
            .is_err()
    );

    let sent = journal
        .admit_with_deadline("p", "s", "sent-deadline", &json!({"effect":"save"}), 100)
        .unwrap();
    assert!(
        journal
            .record_dispatch("p", "s", &sent.operation.id, "sent")
            .unwrap()
            .acquired
    );
    assert!(
        journal
            .acknowledge_dispatch("p", "s", &sent.operation.id, "sent")
            .unwrap()
    );
    assert_eq!(
        journal
            .get("p", "s", &sent.operation.id)
            .unwrap()
            .unwrap()
            .delivery,
        Delivery::Sent
    );
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let expired = journal.get("p", "s", &sent.operation.id).unwrap().unwrap();
    assert_eq!(expired.status, JobStatus::Unknown);
    assert_eq!(expired.delivery, Delivery::Sent);
    assert!(
        journal
            .cancel("p", "s", &expired.id)
            .is_ok_and(|changed| !changed)
    );
    cleanup(&path);
}

#[tokio::test]
async fn deadline_is_persisted_across_journal_reopen() {
    let path = temp_db();
    let journal = Journal::open(&path).unwrap();
    let admitted = journal
        .admit_with_deadline("p", "s", "restart-deadline", &json!({"effect":"read"}), 100)
        .unwrap();
    let id = admitted.operation.id;
    drop(journal);
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let reopened = Journal::open(&path).unwrap();
    let operation = reopened.get("p", "s", &id).unwrap().unwrap();
    assert_eq!(operation.status, JobStatus::Failed);
    assert_eq!(operation.delivery, Delivery::NotSent);
    cleanup(&path);
}
