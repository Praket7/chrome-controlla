pub mod apps;
pub mod artifacts;
pub mod browser_contract;
pub mod browser_workflow;
pub mod cache;
pub mod canva;
pub mod capability;
pub mod capability_registry;
pub mod capability_router;
pub mod capcut_recipe;
pub mod compact_catalog;
pub mod desktop_fallback;
pub mod desktop_platform;
pub mod doctor;
pub mod evidence_store;
pub mod execution_cost_router;
pub mod execution_journal;
pub mod guard_reason;
pub mod http_auth;
pub mod http_server;
pub mod jobs;
pub mod latency_slo;
pub mod mcp;
pub mod metrics;
pub mod native_host;
pub mod native_setup;
pub mod network_acceleration;
pub mod observation_router;
pub mod page_action_memory;
pub mod page_tool_schema;
pub mod process_lease;
pub mod protocol_contract;
pub mod reference_reacquire;
pub mod relevance;
pub mod runtime_observer;
pub mod semantic_state;
pub mod site_profiles;
pub mod skill_induction;
pub mod skill_runtime;
pub mod skill_store;
pub mod skills;
pub mod slides_deck;
pub mod target_graph;
pub mod task_capsule;
pub mod task_checkpoints;
pub mod v3;
pub mod v3_runtime;
pub mod v3_tasks;
pub mod verifier;
pub mod visual_probe;
pub mod visual_stability;
pub mod widget_adapters;
pub mod workflow;
pub mod workflow_dag;
pub mod workflow_value;
pub mod workflow_vm;

#[cfg(test)]
mod tests {
    use super::capability::*;

    fn context() -> CapabilityContext {
        CapabilityContext {
            direct: true,
            bridge: false,
            configured: true,
            process_reachable: true,
            authenticated: true,
            heartbeat_fresh: true,
            origin_allowed: true,
            grant_active: true,
            policy_revision: 7,
            capability_revision: 3,
            provider_source: AvailabilitySource::Fixture,
        }
    }

    #[test]
    fn direct_only_and_bridge_only_routes_agree_for_catalog_and_dispatch() {
        for (direct, bridge, route) in [(true, false, Route::Direct), (false, true, Route::Bridge)]
        {
            let mut ctx = context();
            ctx.direct = direct;
            ctx.bridge = bridge;
            let listed = list_capabilities(&ctx, &["click"])[0].clone();
            let dispatched = authorize_dispatch(&ctx, "click");
            assert!(listed.supported && listed.configured && listed.authorized && listed.qualified);
            assert_eq!(listed.route, Some(route));
            assert_eq!(dispatched, listed);
        }
    }

    #[test]
    fn unconfigured_denied_and_revoked_are_distinct() {
        let mut ctx = context();
        ctx.configured = false;
        assert_eq!(
            evaluate_capability(&ctx, "click").reason,
            Reason::NotConfigured
        );
        ctx = context();
        ctx.origin_allowed = false;
        assert_eq!(
            evaluate_capability(&ctx, "click").reason,
            Reason::PolicyDenied
        );
        ctx = context();
        ctx.grant_active = false;
        assert_eq!(
            evaluate_capability(&ctx, "click").reason,
            Reason::GrantRevoked
        );
        ctx.policy_revision = 8;
        let fresh = authorize_dispatch(&ctx, "click");
        assert_eq!(fresh.reason, Reason::GrantRevoked);
        assert_eq!(fresh.policy_revision, 8);
    }
}
