use controlla_runtime::v3::{BrowserMode, InteractionEpoch, RecoveryClass, TypingMode};
use controlla_runtime::v3_runtime::{
    BatchAction, BatchActionKind, BrowserLaunchPlan, ClientKind, ClientQualification,
    DEFAULT_AGENT_TOOLS, PageToolAuthority, PageToolCall, ReconnectController, ReconnectDecision,
    TypingPlan, compact_tool_surface_valid, execute_guarded_batch, result_within_budget,
};
use serde_json::json;
use std::collections::BTreeSet;

fn epoch() -> InteractionEpoch {
    InteractionEpoch {
        browser_generation: 1,
        target_id: "tab-1".into(),
        target_revision: "target-1".into(),
        frame_id: "frame-1".into(),
        loader_id: "loader-1".into(),
        document_id: "doc-1".into(),
        focused_backend_node_id: Some(10),
        selection_fingerprint: Some("10:0:0".into()),
        semantic_revision: "sem-1".into(),
        viewport_revision: 1,
    }
}

#[test]
fn browser_modes_produce_explicit_launch_plans() {
    let foreground = BrowserLaunchPlan::for_mode(BrowserMode::Foreground);
    let background = BrowserLaunchPlan::for_mode(BrowserMode::Background);
    let headless = BrowserLaunchPlan::for_mode(BrowserMode::Headless);
    assert!(foreground.may_activate_window);
    assert!(!background.may_activate_window);
    assert!(background.chrome_args.contains(&"--start-minimized"));
    assert!(headless.chrome_args.contains(&"--headless=new"));
    assert_eq!(headless.provider, "dedicated_headless");
}

#[test]
fn guarded_batch_is_one_host_round_trip_and_stops_before_stale_mutation() {
    let grounded = epoch();
    let actions = vec![
        BatchAction {
            id: "read".into(),
            kind: BatchActionKind::Read,
        },
        BatchAction {
            id: "fill".into(),
            kind: BatchActionKind::TextMutation,
        },
        BatchAction {
            id: "click".into(),
            kind: BatchActionKind::PointerMutation,
        },
    ];
    let mut stale = grounded.clone();
    stale.selection_fingerprint = Some("10:1:1".into());
    let receipt = execute_guarded_batch(
        &grounded,
        &[grounded.clone(), stale, grounded.clone()],
        &actions,
    )
    .unwrap();
    assert_eq!(receipt.host_round_trips, 1);
    assert_eq!(receipt.completed, vec!["read"]);
    assert_eq!(receipt.stopped_before.as_deref(), Some("fill"));
    assert!(receipt.stale_transition);
}

#[test]
fn block_typing_is_constant_protocol_work_and_fast_keys_have_no_delay_floor() {
    let payload = "a".repeat(1_000);
    let block = TypingPlan::build(&payload, TypingMode::Block, None).unwrap();
    assert_eq!(block.protocol_batches, 1);
    assert_eq!(block.events.len(), 1);

    let fast = TypingPlan::build(&payload, TypingMode::FastKeys, Some(60)).unwrap();
    assert_eq!(fast.policy.delay_ms, 0);
    assert!(fast.protocol_batches <= 63);
    assert!(fast.events.iter().all(|event| event.delay_after_ms == 0));
    assert!(fast.requires_focus_revalidation);

    let human = TypingPlan::build("abc", TypingMode::HumanKeys, Some(24)).unwrap();
    assert_eq!(human.policy.delay_ms, 24);
    assert_eq!(human.protocol_batches, 3);
}

#[test]
fn ime_typing_uses_composition_events() {
    let plan = TypingPlan::build("こんにちは", TypingMode::Ime, None).unwrap();
    assert_eq!(plan.protocol_batches, 1);
    assert_eq!(plan.events.len(), 4);
    assert_eq!(plan.events[1].text, "こんにちは");
}

#[test]
fn page_tools_are_allowlisted_bounded_and_typed() {
    let allowed = BTreeSet::from(["search".to_string()]);
    let call = PageToolCall {
        name: "search".into(),
        authority: PageToolAuthority::ReadOnly,
        input: json!({"query":"rust"}),
        timeout_ms: 1_000,
        max_output_bytes: 16_384,
    };
    assert!(call.validate(&allowed).is_ok());
    let mut oversized = call.clone();
    oversized.max_output_bytes = 300_000;
    assert!(oversized.validate(&allowed).is_err());
}

#[test]
fn reconnect_never_blindly_replays_mutations_and_is_bounded() {
    let mut controller = ReconnectController::new(2).unwrap();
    assert_eq!(
        controller.on_disconnect(RecoveryClass::SideEffecting),
        ReconnectDecision::StopUnknownMutation
    );
    assert_eq!(
        controller.on_disconnect(RecoveryClass::UnknownDelivery),
        ReconnectDecision::ObserveBeforeDecision
    );
    assert_eq!(
        controller.on_disconnect(RecoveryClass::IdempotentRead),
        ReconnectDecision::Terminal
    );
    assert_eq!(controller.attempts(), 2);
}

#[test]
fn default_agent_surface_is_exactly_six_compact_tools() {
    assert_eq!(
        DEFAULT_AGENT_TOOLS,
        [
            "browser", "snapshot", "act", "workflow", "extract", "verify"
        ]
    );
    assert!(compact_tool_surface_valid(47 * 1024));
    assert!(!compact_tool_surface_valid(49 * 1024));
    assert!(result_within_budget(&json!({"ok":true,"delta":[1,2,3]})));
}

#[test]
fn client_qualification_fails_closed() {
    let mut q = ClientQualification {
        client: ClientKind::Codex,
        startup: true,
        persistent_session: true,
        foreground: true,
        background: true,
        headless: true,
        typed_input: true,
        multi_tab: true,
        interference_safe: true,
        upload_download: true,
        research_extract: true,
        verified_mutations: false,
    };
    assert!(!q.qualified());
    q.verified_mutations = true;
    assert!(q.qualified());
}
