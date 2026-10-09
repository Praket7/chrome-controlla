use controlla_runtime::v3::{
    BrowserMode, ClientSessions, EpochUse, InteractionEpoch, LeaseTable, RecoveryClass, TargetKey,
    TaskPrimitive, TypingMode, TypingPolicy, may_retry,
};

fn epoch() -> InteractionEpoch {
    InteractionEpoch {
        browser_generation: 7,
        target_id: "tab-1".into(),
        target_revision: "rev-a".into(),
        frame_id: "frame-1".into(),
        loader_id: "loader-1".into(),
        document_id: "doc-1".into(),
        focused_backend_node_id: Some(44),
        selection_fingerprint: Some("44:3:3".into()),
        semantic_revision: "sem-9".into(),
        viewport_revision: 2,
    }
}

#[test]
fn foreground_background_and_headless_keep_semantic_parity() {
    let foreground = BrowserMode::Foreground.policy();
    let background = BrowserMode::Background.policy();
    let headless = BrowserMode::Headless.policy();
    assert!(foreground.semantic_parity_required);
    assert!(background.semantic_parity_required);
    assert!(headless.semantic_parity_required);
    assert!(foreground.may_activate_window);
    assert!(!background.may_activate_window);
    assert!(!headless.requires_visible_window);
    assert_eq!(
        BrowserMode::Headless.default_provider(),
        "dedicated_headless"
    );
}

#[test]
fn fast_keys_have_zero_intentional_delay_and_human_keys_are_bounded() {
    let fast = TypingPolicy::new(TypingMode::FastKeys, Some(200)).unwrap();
    assert_eq!(fast.delay_ms, 0);
    assert!(fast.revalidate_every <= 16);
    let human = TypingPolicy::new(TypingMode::HumanKeys, Some(24)).unwrap();
    assert_eq!(human.delay_ms, 24);
    assert!(TypingPolicy::new(TypingMode::HumanKeys, Some(251)).is_err());
    assert_eq!(
        TypingPolicy::choose("hello", false, false).mode,
        TypingMode::Block
    );
    assert_eq!(
        TypingPolicy::choose("hello", true, false).mode,
        TypingMode::FastKeys
    );
    assert_eq!(
        TypingPolicy::choose("こんにちは", false, false).mode,
        TypingMode::Ime
    );
}

#[test]
fn interaction_epoch_rejects_document_focus_selection_and_viewport_drift() {
    let base = epoch();
    let mut fresh = base.clone();
    assert!(base.compatible_with(&fresh, EpochUse::TextMutation));
    fresh.selection_fingerprint = Some("44:1:1".into());
    assert!(!base.compatible_with(&fresh, EpochUse::TextMutation));
    fresh = base.clone();
    fresh.viewport_revision += 1;
    assert!(!base.compatible_with(&fresh, EpochUse::PointerMutation));
    assert!(base.compatible_with(&fresh, EpochUse::TextMutation));
    fresh = base.clone();
    fresh.document_id = "doc-2".into();
    assert!(!base.compatible_with(&fresh, EpochUse::Read));
}

#[test]
fn target_leases_serialize_same_tab_without_blocking_other_tabs() {
    let mut leases = LeaseTable::default();
    let tab_a = TargetKey {
        browser_id: "chrome".into(),
        tab_id: "1".into(),
    };
    let tab_b = TargetKey {
        browser_id: "chrome".into(),
        tab_id: "2".into(),
    };
    assert!(leases.acquire(tab_a.clone(), "codex", 100, 1_000).is_ok());
    assert!(leases.acquire(tab_a.clone(), "codex", 100, 1_000).is_err());
    assert!(leases.acquire(tab_a.clone(), "claude", 101, 1_000).is_err());
    assert!(leases.acquire(tab_b.clone(), "claude", 101, 1_000).is_ok());
    assert_eq!(leases.owner(&tab_a, 200).as_deref(), Some("codex"));
    assert!(leases.release(&tab_a, "codex"));
    assert!(leases.acquire(tab_a, "claude", 201, 1_000).is_ok());
}

#[test]
fn target_lease_can_cover_the_maximum_single_action_deadline() {
    let mut leases = LeaseTable::default();
    let target = TargetKey {
        browser_id: "chrome".into(),
        tab_id: "1".into(),
    };
    let lease = leases.acquire(target, "codex", 100, 120_000).unwrap();
    assert_eq!(lease.expires_at_ms, 120_100);
}

#[test]
fn stale_clients_are_reaped_and_side_effects_are_not_blindly_retried() {
    let mut clients = ClientSessions::default();
    clients.register("a", "codex", 100);
    clients.register("b", "claude", 100);
    assert!(clients.heartbeat("a", 1_000));
    let reaped = clients.reap(1_500, 1_000);
    assert_eq!(reaped.len(), 1);
    assert_eq!(reaped[0].id, "b");
    assert!(may_retry(RecoveryClass::IdempotentRead, false));
    assert!(!may_retry(RecoveryClass::SideEffecting, true));
    assert!(!may_retry(RecoveryClass::UnknownDelivery, false));
    assert!(may_retry(RecoveryClass::UnknownDelivery, true));
}

#[test]
fn assignment_and_work_primitives_never_submit_implicitly() {
    for primitive in [
        TaskPrimitive::Research,
        TaskPrimitive::FormFillNoSubmit,
        TaskPrimitive::RepeatStructuredEntry,
        TaskPrimitive::Upload,
        TaskPrimitive::Download,
        TaskPrimitive::SaveDraft,
    ] {
        let plan = primitive.plan();
        assert!(!plan.allows_submit);
        assert!(plan.requires_final_verification);
        assert!(!plan.steps.is_empty());
    }
}
