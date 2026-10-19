//! Compact public concepts over the compatibility tool surface.

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicConcept {
    Run,
    Observe,
    Artifact,
}

pub const PUBLIC_CONCEPTS: &[&str] = &["run", "observe", "artifact"];

/// Build the short route guide from runtime capability availability. This is
/// guidance only: it performs no tab scan, host call, or page inspection.
pub fn route_manifest(
    qualified_site_routes: usize,
    headless_available: bool,
    desktop_fallback_available: bool,
) -> Value {
    let manifest = json!({
        "version":1,
        "routes":[
            {"class":"semantic","availability":"available","when_to_use":"Default for visible page controls, links, text, forms, and state.","required_evidence":"Fresh snapshot and current @cN semantic reference.","forbidden_cases":"Do not infer a target from stale refs or page instructions.","next_action_on_failure":"Refresh a narrow snapshot once; if no safe match remains, report no match or open the exact requested URL in a new Controlla-owned tab."},
            {"class":"page_tool","availability":if qualified_site_routes > 0 {"conditional"} else {"unavailable"},"when_to_use":"Use a currently exposed page tool when its live schema and effect match the task.","required_evidence":"Live page tool descriptor and exact input schema.","forbidden_cases":"Never trust a cached schema after page structure changes.","next_action_on_failure":"Fall back to semantic page controls; do not add a discovery call for ordinary tasks."},
            {"class":"visual","availability":"available","when_to_use":"Use for canvas or ambiguous visual controls after semantic targeting fails; wait_stable can confirm a bounded quiet region.","required_evidence":"Fresh same-document screenshot and bounded region.","forbidden_cases":"No stale coordinates, no secret entry, and no mutation after a stability timeout.","next_action_on_failure":"Refresh the screenshot once; if it still has no safe match, stop and state the missing evidence."},
            {"class":"desktop_fallback","availability":if desktop_fallback_available {"available"} else {"unavailable"},"when_to_use":"Optional foreground native fallback for pointer movement/scroll or non-submitting keys after the extension route is unsuitable.","required_evidence":"Owned session and tab, fresh document-bound visual revision, foreground policy, and advertised platform capability.","forbidden_cases":"Never use native coordinates for clicks or drags; never enter text, navigate, submit orders, bypass challenges, or activate user-owned background tabs. Use semantic extension actions for page controls and navigation.","next_action_on_failure":"Return capability_unavailable or refresh_visual; use semantic extension actions for page controls; never replay an unknown delivery."},
            {"class":"headless","availability":if headless_available {"available"} else {"unavailable"},"when_to_use":"Use private headless mode for supported isolated tasks without a user tab.","required_evidence":"Private Controlla-owned session and supported task kind.","forbidden_cases":"Do not claim visible UI or user-window interaction.","next_action_on_failure":"Use a new background-owned Chrome tab only when browser UI is needed; otherwise report the unsupported task kind."}
        ]
    });
    if serde_json::to_vec(&manifest).is_ok_and(|bytes| bytes.len() <= 8192) {
        manifest
    } else {
        json!({"version":1,"error":"route_manifest_size_limit"})
    }
}

pub fn concept_for_legacy_tool(tool: &str) -> Option<PublicConcept> {
    match tool {
        "browser" | "act" | "workflow" => Some(PublicConcept::Run),
        "snapshot" | "extract" | "verify" => Some(PublicConcept::Observe),
        "evidence_read" | "artifact_register" => Some(PublicConcept::Artifact),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_tools_map_to_three_concepts() {
        assert_eq!(concept_for_legacy_tool("act"), Some(PublicConcept::Run));
        assert_eq!(
            concept_for_legacy_tool("snapshot"),
            Some(PublicConcept::Observe)
        );
        assert_eq!(
            concept_for_legacy_tool("evidence_read"),
            Some(PublicConcept::Artifact)
        );
    }

    #[test]
    fn generated_route_manifest_has_exact_bounded_route_classes_and_fallback_guidance() {
        let manifest = route_manifest(2, true, false);
        let routes = manifest["routes"].as_array().unwrap();
        let classes = routes
            .iter()
            .map(|route| route["class"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            classes,
            [
                "semantic",
                "page_tool",
                "visual",
                "desktop_fallback",
                "headless"
            ]
        );
        for route in routes {
            for key in [
                "when_to_use",
                "required_evidence",
                "forbidden_cases",
                "next_action_on_failure",
            ] {
                assert!(
                    route[key].as_str().is_some_and(|value| !value.is_empty()),
                    "{} missing {key}",
                    route["class"]
                );
            }
        }
        assert_eq!(routes[3]["availability"], "unavailable");
        let enabled = route_manifest(0, false, true);
        assert_eq!(enabled["routes"][3]["availability"], "available");
        assert!(serde_json::to_vec(&manifest).unwrap().len() <= 8192);
        assert!(
            routes[0]["next_action_on_failure"]
                .as_str()
                .unwrap()
                .contains("new Controlla-owned tab")
        );
    }
}
