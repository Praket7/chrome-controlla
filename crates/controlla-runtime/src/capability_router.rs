use crate::metrics::RouteClass;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteAvailability {
    pub native_app: bool,
    pub qualified_skill: bool,
    pub semantic: bool,
    pub visual_probe: bool,
    pub strict_input: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RouteDecision {
    pub selected: RouteClass,
    pub attempted: Vec<RouteClass>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTier {
    QualifiedSkill,
    WebMcpOrSiteTool,
    SpeculativeRead,
    SemanticProgram,
    SemanticPlanner,
    TargetedVisual,
    FullVisionFallback,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionCapabilities {
    pub qualified_skill: bool,
    pub site_tool: bool,
    pub speculative_read: bool,
    pub semantic_program: bool,
    pub semantic_confidence: f32,
    pub targeted_visual: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionRouteDecision {
    pub selected: ExecutionTier,
    pub reason: &'static str,
    pub predicted_model_calls: u8,
    pub expected_cost: ExpectedRouteCost,
}

/// Bounded, route-level cost estimate. Millisecond-valued terms are kept
/// separate from work counts so receipts show which parts are measured versus
/// policy estimates. Non-elapsed terms use conservative default conversion
/// weights in `policy_overhead_ms`; they are not claimed as empirical data.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExpectedRouteCost {
    pub model_turns: u8,
    pub broad_observations: u8,
    pub visual_processing_units: u8,
    pub cold_start: u8,
    pub schema_setup: u8,
    pub transport_round_trips: u8,
    pub verification_passes: u8,
    pub elapsed_ms: u64,
    pub policy_overhead_ms: u64,
    pub total_expected_ms: u64,
}

impl ExpectedRouteCost {
    pub fn for_tier(tier: ExecutionTier, cold_browser: bool, measured_elapsed_ms: u64) -> Self {
        let (
            model_turns,
            broad_observations,
            visual_processing_units,
            schema_setup,
            transport_round_trips,
            verification_passes,
        ) = match tier {
            ExecutionTier::QualifiedSkill => (0, 1, 0, 0, 2, 1),
            ExecutionTier::WebMcpOrSiteTool => (0, 1, 0, 1, 2, 1),
            ExecutionTier::SpeculativeRead => (0, 1, 0, 0, 3, 1),
            ExecutionTier::SemanticProgram => (0, 1, 0, 0, 3, 1),
            ExecutionTier::SemanticPlanner => (1, 1, 0, 0, 4, 1),
            ExecutionTier::TargetedVisual => (1, 1, 1, 0, 4, 1),
            ExecutionTier::FullVisionFallback => (1, 2, 2, 0, 5, 1),
        };
        // `measured_elapsed_ms` is the MCP task route's elapsed interval. It
        // already includes browser startup/attach when they occur, schema work,
        // transport, visual capture, and verification inside that route. Only
        // caller-side model turns and model-visible observation handling sit
        // outside this interval, so only those estimates are added. The values
        // remain explicit heuristics until matched Phase 11 measurements can
        // calibrate them per provider and cohort.
        let cold_start = u8::from(cold_browser);
        let policy_overhead_ms = u64::from(model_turns) * 1_500
            + u64::from(broad_observations) * 250
            + u64::from(visual_processing_units) * 500;
        Self {
            model_turns,
            broad_observations,
            visual_processing_units,
            cold_start,
            schema_setup,
            transport_round_trips,
            verification_passes,
            elapsed_ms: measured_elapsed_ms,
            policy_overhead_ms,
            total_expected_ms: measured_elapsed_ms.saturating_add(policy_overhead_ms),
        }
    }
}

/// Per-route invocation budget. Call `begin_*` immediately before dispatching
/// an invocation; rejected work is not counted as dispatched work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteBudgetLimits {
    pub internal_models: u32,
    pub visible_observations: u32,
    pub browser_commands: u32,
    pub wall_deadline_ms: u64,
    pub fallback_count: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteBudgetUsage {
    pub internal_models: u32,
    pub visible_observations: u32,
    pub browser_commands: u32,
    pub elapsed_ms: u64,
    pub fallback_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteBudgetExceeded {
    InternalModels,
    VisibleObservations,
    BrowserCommands,
    WallDeadline,
    Fallbacks,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteBudget {
    pub limits: RouteBudgetLimits,
    pub usage: RouteBudgetUsage,
}

impl RouteBudget {
    pub const fn new(limits: RouteBudgetLimits) -> Self {
        Self {
            limits,
            usage: RouteBudgetUsage {
                internal_models: 0,
                visible_observations: 0,
                browser_commands: 0,
                elapsed_ms: 0,
                fallback_count: 0,
            },
        }
    }

    pub const fn usage(&self) -> RouteBudgetUsage {
        self.usage
    }

    pub fn begin_internal_model(&mut self) -> Result<(), RouteBudgetExceeded> {
        Self::increment(
            &mut self.usage.internal_models,
            self.limits.internal_models,
            RouteBudgetExceeded::InternalModels,
        )
    }

    pub fn begin_visible_observation(&mut self) -> Result<(), RouteBudgetExceeded> {
        Self::increment(
            &mut self.usage.visible_observations,
            self.limits.visible_observations,
            RouteBudgetExceeded::VisibleObservations,
        )
    }

    pub fn begin_browser_command(&mut self) -> Result<(), RouteBudgetExceeded> {
        Self::increment(
            &mut self.usage.browser_commands,
            self.limits.browser_commands,
            RouteBudgetExceeded::BrowserCommands,
        )
    }

    pub fn begin_fallback(&mut self) -> Result<(), RouteBudgetExceeded> {
        Self::increment(
            &mut self.usage.fallback_count,
            self.limits.fallback_count,
            RouteBudgetExceeded::Fallbacks,
        )
    }

    pub fn check_deadline(&mut self, elapsed_ms: u64) -> Result<(), RouteBudgetExceeded> {
        self.usage.elapsed_ms = elapsed_ms;
        if elapsed_ms >= self.limits.wall_deadline_ms {
            Err(RouteBudgetExceeded::WallDeadline)
        } else {
            Ok(())
        }
    }

    fn increment(
        value: &mut u32,
        limit: u32,
        exceeded: RouteBudgetExceeded,
    ) -> Result<(), RouteBudgetExceeded> {
        if *value >= limit {
            return Err(exceeded);
        }
        *value += 1;
        Ok(())
    }
}

pub fn route_execution(
    capabilities: ExecutionCapabilities,
    read_only: bool,
) -> ExecutionRouteDecision {
    let (selected, reason, predicted_model_calls) = if read_only && capabilities.qualified_skill {
        (
            ExecutionTier::QualifiedSkill,
            "qualified workflow preconditions are available",
            0,
        )
    } else if capabilities.site_tool {
        (
            ExecutionTier::WebMcpOrSiteTool,
            "a runtime-qualified site tool is available",
            0,
        )
    } else if read_only && capabilities.speculative_read {
        (
            ExecutionTier::SpeculativeRead,
            "verified read-only site profile is available",
            0,
        )
    } else if capabilities.semantic_program {
        (
            ExecutionTier::SemanticProgram,
            "a deterministic semantic program is available",
            0,
        )
    } else if capabilities.semantic_confidence >= 0.8 {
        (
            ExecutionTier::SemanticPlanner,
            "semantic target confidence meets the routing threshold",
            1,
        )
    } else if capabilities.targeted_visual {
        (
            ExecutionTier::TargetedVisual,
            "semantic confidence is low and a bounded visual probe is available",
            1,
        )
    } else {
        (
            ExecutionTier::FullVisionFallback,
            "no cheaper qualified route is available",
            1,
        )
    };
    ExecutionRouteDecision {
        selected,
        reason,
        predicted_model_calls,
        expected_cost: ExpectedRouteCost::for_tier(selected, false, 0),
    }
}

pub fn select_route(availability: RouteAvailability) -> Option<RouteDecision> {
    let ordered = [
        (RouteClass::NativeApp, availability.native_app),
        (RouteClass::QualifiedSkill, availability.qualified_skill),
        (RouteClass::Semantic, availability.semantic),
        (RouteClass::VisualProbe, availability.visual_probe),
        (RouteClass::StrictInput, availability.strict_input),
    ];
    let mut attempted = Vec::new();
    for (route, available) in ordered {
        attempted.push(route);
        if available {
            return Some(RouteDecision {
                selected: route,
                attempted,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_budget_rejects_invocations_before_exceeding_any_configured_limit() {
        let mut budget = RouteBudget::new(RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 1,
            browser_commands: 2,
            wall_deadline_ms: 50,
            fallback_count: 1,
        });

        assert_eq!(
            budget.begin_internal_model(),
            Err(RouteBudgetExceeded::InternalModels)
        );
        budget.begin_visible_observation().unwrap();
        assert_eq!(
            budget.begin_visible_observation(),
            Err(RouteBudgetExceeded::VisibleObservations)
        );
        budget.begin_browser_command().unwrap();
        budget.begin_browser_command().unwrap();
        assert_eq!(
            budget.begin_browser_command(),
            Err(RouteBudgetExceeded::BrowserCommands)
        );
        budget.begin_fallback().unwrap();
        assert_eq!(budget.begin_fallback(), Err(RouteBudgetExceeded::Fallbacks));
        assert_eq!(
            budget.check_deadline(51),
            Err(RouteBudgetExceeded::WallDeadline)
        );
        assert_eq!(
            budget.usage(),
            RouteBudgetUsage {
                internal_models: 0,
                visible_observations: 1,
                browser_commands: 2,
                elapsed_ms: 51,
                fallback_count: 1,
            }
        );
    }

    #[test]
    fn route_priority_prefers_cheaper_more_structured_capabilities() {
        let decision = select_route(RouteAvailability {
            native_app: false,
            qualified_skill: true,
            semantic: true,
            visual_probe: true,
            strict_input: true,
        })
        .unwrap();
        assert_eq!(decision.selected, RouteClass::QualifiedSkill);
        assert_eq!(
            decision.attempted,
            vec![RouteClass::NativeApp, RouteClass::QualifiedSkill]
        );
    }

    #[test]
    fn router_fails_closed_when_no_route_is_qualified() {
        assert_eq!(select_route(RouteAvailability::default()), None);
    }
}
