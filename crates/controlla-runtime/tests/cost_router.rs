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
