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
