use controlla_runtime::capability_registry::{
    CapabilityImplementation, CapabilityRegistry, SiteCapability,
};
use controlla_runtime::workflow::EffectClass;
use serde_json::json;

fn capability(implementation: CapabilityImplementation, effect: EffectClass) -> SiteCapability {
    SiteCapability {
        name: "search".into(),
        site_scope: "https://example.test".into(),
        structural_signature: "search:v1".into(),
        input_schema: json!({"type":"object"}),
        implementation,
        effect,
        independently_verified: true,
        version: 1,
    }
}

#[test]
fn operation_routing_is_bounded_unambiguous_and_exact_site_structure() {
    use controlla_runtime::capability_registry::{
        operation_from_goal, operation_input, read_only_webmcp_operation,
    };

    assert_eq!(
        operation_from_goal("Search the team site for recent scores"),
        Some("search")
    );
    assert_eq!(
        operation_from_goal("Filter results by date"),
        Some("filter")
    );
    assert_eq!(operation_from_goal("Search then sort by date"), None);
    assert_eq!(operation_from_goal("show the dashboard"), None);
    assert_eq!(operation_from_goal(&"x".repeat(513)), None);

    let schema = json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false});
    assert_eq!(
        operation_input("search", "Search for Bills scores", &schema),
        Some(json!({"query":"Bills scores"}))
    );
    assert_eq!(
        read_only_webmcp_operation("search", &schema),
        Some("search")
    );
    assert_eq!(read_only_webmcp_operation("create_draft", &schema), None);
    assert_eq!(
        read_only_webmcp_operation(
            "search",
            &json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":true})
        ),
        None
    );
    assert_eq!(
        operation_input(
            "search",
            "Search for Bills scores",
            &json!({"type":"object","properties":{"query":{"type":"string"},"account":{"type":"string"}},"required":["query","account"]})
        ),
        None
    );
    assert_eq!(
        operation_input(
            "search",
            "Search for Bills scores",
            &json!({"type":"object","properties":{"query":{"type":"string"},"amount":{"type":"number"}},"required":["query"]})
        ),
        None
    );

    let mut registry = CapabilityRegistry::default();
    registry
        .register(capability(
            CapabilityImplementation::QualifiedSkill {
                skill_id: "search-v1".into(),
            },
            EffectClass::ReadOnly,
        ))
        .unwrap();
    assert!(
        registry
            .resolve_operation("search", "https://example.test", "search:v1", true)
            .is_some()
    );
    assert!(
        registry
            .resolve_operation("search", "https://elsewhere.test", "search:v1", true)
            .is_none()
    );
    assert!(
        registry
            .resolve_operation("search", "https://example.test", "search:v2", true)
            .is_none()
    );
    assert!(
        registry
            .resolve_operation("create_draft", "https://example.test", "search:v1", true)
            .is_none()
    );
}

#[test]
fn registry_prefers_qualified_capabilities_and_requires_exact_live_identity() {
    let mut registry = CapabilityRegistry::default();
    registry
        .register(capability(
            CapabilityImplementation::SiteProfile {
                profile_id: "p1".into(),
            },
            EffectClass::ReadOnly,
        ))
        .unwrap();
    registry
        .register(capability(
            CapabilityImplementation::WebMcp {
                tool_name: "search".into(),
            },
            EffectClass::ReadOnly,
        ))
        .unwrap();
    let selected = registry
        .resolve("search", "https://example.test", "search:v1", true)
        .unwrap();
    assert!(matches!(
        selected.implementation,
        CapabilityImplementation::WebMcp { .. }
    ));
    assert!(
        registry
            .resolve("search", "https://example.test", "search:v2", true)
            .is_none()
    );
    assert!(
        registry
            .resolve("search", "https://other.test", "search:v1", true)
            .is_none()
    );
}

#[test]
fn unverified_or_mutating_capabilities_are_never_used_for_read_only_routing() {
    let mut registry = CapabilityRegistry::default();
    let mut capability = capability(
        CapabilityImplementation::QualifiedSkill {
            skill_id: "s1".into(),
        },
        EffectClass::External,
    );
    capability.independently_verified = false;
    registry.register(capability).unwrap();
    assert!(
        registry
            .resolve("search", "https://example.test", "search:v1", true)
            .is_none()
    );
    assert!(
        registry
            .resolve("search", "https://example.test", "search:v1", false)
            .is_none()
    );
}

#[test]
fn cached_webmcp_candidate_is_exact_site_and_requires_prior_verification() {
    let mut registry = CapabilityRegistry::default();
    let mut candidate = capability(
        CapabilityImplementation::WebMcp {
            tool_name: "search".into(),
        },
        EffectClass::External,
    );
    candidate.independently_verified = false;
    registry.register(candidate.clone()).unwrap();
    assert!(
        registry
            .resolve_webmcp_candidate("search", "https://example.test")
            .is_none()
    );
    candidate.independently_verified = true;
    registry.register(candidate).unwrap();
    assert!(
        registry
            .resolve_webmcp_candidate("search", "https://example.test")
            .is_some()
    );
    assert!(
        registry
            .resolve_webmcp_candidate("search", "https://other.test")
            .is_none()
    );
}

#[test]
fn inferred_webmcp_read_requires_verified_runtime_classification_and_exact_site() {
    let mut registry = CapabilityRegistry::default();
    let mut candidate = capability(
        CapabilityImplementation::WebMcp {
            tool_name: "search".into(),
        },
        EffectClass::ReadOnly,
    );
    candidate.structural_signature = "tool-schema-fingerprint".into();
    candidate.input_schema = json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false});
    assert!(
        registry
            .resolve_read_only_webmcp_operation("search", "https://example.test")
            .is_none()
    );
    candidate.independently_verified = true;
    registry.register(candidate.clone()).unwrap();
    assert_eq!(
        registry
            .resolve_read_only_webmcp_operation("search", "https://example.test")
            .map(|capability| capability.structural_signature.as_str()),
        Some("tool-schema-fingerprint")
    );
    assert!(
        registry
            .resolve_read_only_webmcp_operation("search", "https://other.test")
            .is_none()
    );
    candidate.effect = EffectClass::External;
    registry.register(candidate.clone()).unwrap();
    assert!(
        registry
            .resolve_read_only_webmcp_operation("search", "https://example.test")
            .is_none()
    );
    candidate.effect = EffectClass::ReadOnly;
    candidate.input_schema = json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":true});
    registry.register(candidate).unwrap();
    assert!(
        registry
            .resolve_read_only_webmcp_operation("search", "https://example.test")
            .is_none()
    );
}

#[test]
fn verified_capability_survives_runtime_restart() {
    let path = std::env::temp_dir().join(format!(
        "controlla-capability-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut registry = CapabilityRegistry::open(&path).unwrap();
    registry
        .register(capability(
            CapabilityImplementation::WebMcp {
                tool_name: "search".into(),
            },
            EffectClass::External,
        ))
        .unwrap();
    let loaded = CapabilityRegistry::open(&path).unwrap();
    assert!(
        loaded
            .resolve_webmcp_candidate("search", "https://example.test")
            .is_some()
    );
    std::fs::remove_file(path).unwrap();
}
