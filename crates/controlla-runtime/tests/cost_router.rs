use controlla_runtime::capability_router::{ExecutionCapabilities, ExecutionTier, route_execution};

#[test]
fn cost_router_selects_cheapest_qualified_tier_and_keeps_visual_as_fallback() {
    let decision = route_execution(
        ExecutionCapabilities {
            qualified_skill: true,
            site_tool: true,
            speculative_read: true,
            semantic_program: true,
            semantic_confidence: 1.0,
            targeted_visual: true,
        },
        true,
    );
    assert_eq!(decision.selected, ExecutionTier::QualifiedSkill);
    assert!(decision.predicted_model_calls == 0);
}

#[test]
fn speculative_routing_is_never_selected_for_mutations() {
    let decision = route_execution(
        ExecutionCapabilities {
            speculative_read: true,
            semantic_confidence: 0.9,
            ..Default::default()
        },
        false,
    );
    assert_ne!(decision.selected, ExecutionTier::SpeculativeRead);
}

#[test]
fn read_only_skill_tier_is_never_selected_for_a_mutating_task() {
    let decision = route_execution(
        ExecutionCapabilities {
            qualified_skill: true,
            ..Default::default()
        },
        false,
    );
    assert_ne!(decision.selected, ExecutionTier::QualifiedSkill);
}

#[test]
fn expected_cost_vector_can_choose_higher_observed_latency_with_lower_total_cost() {
    let mut history = controlla_runtime::execution_cost_router::ExecutionCostRouter::default();
    let context = controlla_runtime::execution_cost_router::RouteCostContext {
        origin: "https://example.test",
        intent: "read-story",
        provider: "chrome",
        warmth: "warm_qualified_route",
        task_size: "small",
    };
    for _ in 0..16 {
        history
            .record(&context, ExecutionTier::TargetedVisual, 100, true, true)
            .unwrap();
        history
            .record(&context, ExecutionTier::QualifiedSkill, 1_000, true, true)
            .unwrap();
    }
    let candidates = [ExecutionTier::TargetedVisual, ExecutionTier::QualifiedSkill];
    assert_eq!(
        history.select(&context, &candidates),
        Some(ExecutionTier::TargetedVisual)
    );
    let selected = history.select_with_overhead(&context, &candidates, |tier| {
        controlla_runtime::capability_router::ExpectedRouteCost::for_tier(tier, false, 0)
            .policy_overhead_ms
    });
    assert_eq!(selected, Some(ExecutionTier::QualifiedSkill));
}

#[test]
fn expected_cost_receipt_accounts_for_each_stage_three_dimension() {
    let cost = controlla_runtime::capability_router::ExpectedRouteCost::for_tier(
        ExecutionTier::TargetedVisual,
        true,
        700,
    );
    assert_eq!(cost.model_turns, 1);
    assert_eq!(cost.broad_observations, 1);
    assert_eq!(cost.visual_processing_units, 1);
    assert_eq!(cost.cold_start, 1);
    assert_eq!(cost.schema_setup, 0);
    assert_eq!(cost.transport_round_trips, 4);
    assert_eq!(cost.verification_passes, 1);
    assert_eq!(cost.elapsed_ms, 700);
    assert_eq!(
        cost.policy_overhead_ms, 2_250,
        "only caller-side model/observation/visual interpretation estimates sit outside the measured MCP interval"
    );
    assert_eq!(cost.total_expected_ms, 2_950);
}
