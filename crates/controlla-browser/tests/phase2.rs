use controlla_browser::scheduler::TargetScheduler;
use controlla_browser::sessions::{
    Ownership, ProviderGrants, SessionError, SessionMode, SessionRegistry, SessionSpec,
    StaleTarget, TargetIdentity, TargetRef,
};
use controlla_browser::{FrameRecord, TargetRecord};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn all_test_grants() -> ProviderGrants {
    ProviderGrants {
        dedicated_headed: true,
        dedicated_headless: true,
        shared_extension: true,
    }
}

#[test]
fn session_modes_use_independent_provider_grants() {
    let mut registry = SessionRegistry::new(ProviderGrants {
        dedicated_headed: true,
        dedicated_headless: false,
        shared_extension: true,
    });
    let headed = registry
        .create_session(
            SessionSpec {
                mode: SessionMode::Headed,
                selected_target_ids: vec![],
            },
            "alice",
        )
        .unwrap();
    registry.bind_session_to_browser(&headed, 10).unwrap();
    registry
        .register_tab(&headed.id, "tab", Ownership::Borrowed)
        .unwrap();
    assert!(
        registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["chosen-tab".into()]
                },
                "alice"
            )
            .is_ok()
    );
    assert!(
        registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: vec![]
                },
                "alice"
            )
            .is_err()
    );
    assert_eq!(
        registry.create_session(
            SessionSpec {
                mode: SessionMode::Shared,
                selected_target_ids: vec![]
            },
            "alice"
        ),
        Err(SessionError::TargetSelectionRequired)
    );
    assert_eq!(
        registry.create_session(
            SessionSpec {
                mode: SessionMode::Headed,
                selected_target_ids: vec!["user-tab".into()],
            },
            "alice"
        ),
        Err(SessionError::UnexpectedTargetSelection)
    );
    let reference = TargetRef {
        session_id: headed.id.clone(),
        principal: headed.principal.clone(),
        capability_revision: headed.capability_revision,
        browser_instance_id: 10,
        browser_generation: 0,
        target_id: "tab".into(),
        target_revision: "r1".into(),
        frame_id: "root".into(),
        frame_revision: 1,
        account_revision: 1,
        document_revision: 1,
    };
    let identity = TargetIdentity::from(&reference);
    assert!(
        registry
            .resolve_active_target(&reference, "alice", &identity)
            .is_ok()
    );
    registry.revoke_provider(controlla_browser::sessions::ProviderKind::DedicatedHeaded);
    assert_eq!(
        registry.resolve_active_target(&reference, "alice", &identity),
        Err(StaleTarget::GrantRevoked)
    );
}

#[test]
fn target_reference_is_bound_to_principal_generation_target_frame_and_identity() {
    let ref_at_start = TargetRef {
        session_id: "s1".into(),
        principal: "alice".into(),
        capability_revision: 2,
        browser_instance_id: 10,
        browser_generation: 4,
        target_id: "tab-1".into(),
        target_revision: "target-r2".into(),
        frame_id: "frame-1".into(),
        frame_revision: 8,
        account_revision: 3,
        document_revision: 9,
    };
    let current = TargetIdentity::from(&ref_at_start);
    assert!(SessionRegistry::resolve_target(&ref_at_start, "alice", &current).is_ok());
    assert_eq!(
        SessionRegistry::resolve_target(&ref_at_start, "bob", &current),
        Err(StaleTarget::PrincipalChanged)
    );
    let mut reused = current;
    reused.target_revision = "target-r3".into();
    assert_eq!(
        SessionRegistry::resolve_target(&ref_at_start, "alice", &reused),
        Err(StaleTarget::TargetChanged)
    );
    let mut other_browser = TargetIdentity::from(&ref_at_start);
    other_browser.browser_instance_id = 11;
    assert_eq!(
        SessionRegistry::resolve_target(&ref_at_start, "alice", &other_browser),
        Err(StaleTarget::BrowserProfileChanged)
    );
}

#[test]
fn target_reference_capture_binds_target_frame_and_navigation_revision() {
    let mut registry = SessionRegistry::new(all_test_grants());
    let session = registry
        .create_session(
            SessionSpec {
                mode: SessionMode::Headed,
                selected_target_ids: vec![],
            },
            "alice",
        )
        .unwrap();
    let target = TargetRecord {
        id: "tab".into(),
        target_type: "page".into(),
        browser_context_id: Some("ctx".into()),
        session_id: Some("cdp-session".into()),
        url: Some("https://example.test".into()),
        title: None,
        opener_id: None,
        attached: true,
        generation: 3,
        revision: "target-revision-7".into(),
    };
    let frame = FrameRecord {
        id: "root-frame".into(),
        parent_id: None,
        target_id: "tab".into(),
        loader_id: Some("loader-1".into()),
        execution_context_ids: vec![12],
        generation: 3,
        revision: 11,
    };
    registry.bind_session_to_browser(&session, 10).unwrap();
    registry
        .register_tab(&session.id, "tab", Ownership::Borrowed)
        .unwrap();
    let reference = TargetRef::capture(&registry, &session, 10, &target, &frame, 5, 8).unwrap();
    let identity = TargetIdentity::from(&reference);
    assert!(
        registry
            .resolve_active_target(&reference, "alice", &identity)
            .is_ok()
    );
    let mut navigated = target;
    navigated.revision = "target-revision-8".into();
    assert_eq!(
        registry.resolve_active_target(
            &reference,
            "alice",
            &TargetIdentity::from_snapshot(10, &reference.session_id, &navigated, &frame, 5, 8)
                .unwrap()
        ),
        Err(StaleTarget::TargetChanged)
    );
}

#[tokio::test]
async fn blocked_tab_does_not_block_other_targets() {
    let scheduler = Arc::new(TargetScheduler::default());
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let blocked_scheduler = Arc::clone(&scheduler);
    let blocked = tokio::spawn(async move {
        blocked_scheduler
            .run_target("slow", async move {
                let _ = entered_tx.send(());
                std::future::pending::<()>().await
            })
            .await
    });
    entered_rx.await.unwrap();
    let healthy = ["healthy-1", "healthy-2", "healthy-3"].map(|target_id| {
        let scheduler = Arc::clone(&scheduler);
        async move {
            scheduler
                .run_target(target_id, async move { target_id })
                .await
        }
    });
    let healthy = tokio::time::timeout(
        Duration::from_millis(100),
        futures_util::future::join_all(healthy),
    )
    .await
    .expect("independent targets were blocked");
    assert_eq!(healthy, ["healthy-1", "healthy-2", "healthy-3"]);
    blocked.abort();
}

#[tokio::test]
async fn scheduler_preserves_identity_at_one_four_and_eight_tabs() {
    for count in [1usize, 4, 8] {
        let scheduler = Arc::new(TargetScheduler::default());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for index in 0..count {
            let scheduler = Arc::clone(&scheduler);
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            workers.push(tokio::spawn(async move {
                let target_id = format!("tab-{index}");
                let response_id = target_id.clone();
                scheduler
                    .run_target(&target_id, async move {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(current, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        response_id
                    })
                    .await
            }));
        }
        let mut identities = Vec::new();
        for worker in workers {
            identities.push(worker.await.unwrap());
        }
        identities.sort();
        let mut expected = (0..count)
            .map(|index| format!("tab-{index}"))
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(identities, expected, "identity mismatch at {count} tabs");
        assert_eq!(peak.load(Ordering::SeqCst), count.min(4));
        assert_eq!(scheduler.max_active_tabs(), 4);
    }
}

#[tokio::test]
async fn scheduler_active_tab_limit_is_configurable_and_nonzero() {
    use controlla_browser::scheduler::SchedulerError;

    assert_eq!(
        TargetScheduler::with_max_active_tabs(0).err(),
        Some(SchedulerError::ActiveTabLimitMustBePositive)
    );
    let scheduler = Arc::new(TargetScheduler::with_max_active_tabs(2).unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();
    for index in 0..6 {
        let scheduler = Arc::clone(&scheduler);
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        workers.push(tokio::spawn(async move {
            scheduler
                .run_target(&format!("tab-{index}"), async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .await
        }));
    }
    for worker in workers {
        worker.await.unwrap();
    }
    assert_eq!(scheduler.max_active_tabs(), 2);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn shared_document_mutations_are_serialized_across_tabs() {
    let scheduler = Arc::new(TargetScheduler::default());
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();
    for target in ["tab-a", "tab-b"] {
        let scheduler = Arc::clone(&scheduler);
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        workers.push(tokio::spawn(async move {
            scheduler
                .run_document_mutation(target, "document-1", async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    target
                })
                .await
        }));
    }
    for worker in workers {
        worker.await.unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}
