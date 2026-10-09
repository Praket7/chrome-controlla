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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteDecision {
    pub selected: RouteClass,
    pub attempted: Vec<RouteClass>,
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
