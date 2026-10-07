pub mod apps;
pub mod artifacts;
pub mod cache;
pub mod canva;
pub mod capability;
pub mod capcut_recipe;
pub mod doctor;
pub mod http_auth;
pub mod jobs;
pub mod mcp;
pub mod slides_deck;
pub mod verifier;
pub mod workflow;

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
