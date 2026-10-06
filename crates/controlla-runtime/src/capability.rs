use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::RwLock;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum Route {
    Direct,
    Bridge,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum Reason {
    Ready,
    UnsupportedAction,
    NotConfigured,
    Unreachable,
    AuthenticationRequired,
    StaleHeartbeat,
    PolicyDenied,
    GrantRevoked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CapabilityDecision {
    pub action: String,
    pub supported: bool,
    pub configured: bool,
    pub reachable: bool,
    pub authorized: bool,
    pub qualified: bool,
    pub reason: Reason,
    pub route: Option<Route>,
    pub policy_revision: u64,
    pub capability_revision: u64,
    pub provider_source: AvailabilitySource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum AvailabilitySource {
    Fixture,
}

/// Provider availability is fixture-backed until a live provider is qualified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderSnapshot {
    pub configured: bool,
    pub direct: bool,
    pub bridge: bool,
    pub process_reachable: bool,
    pub authenticated: bool,
    pub heartbeat_fresh: bool,
    pub source: AvailabilitySource,
}

impl ProviderSnapshot {
    pub fn fixture(direct: bool, bridge: bool) -> Self {
        Self {
            configured: true,
            direct,
            bridge,
            process_reachable: true,
            authenticated: true,
            heartbeat_fresh: true,
            source: AvailabilitySource::Fixture,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PolicySnapshot {
    pub allowed_origins: BTreeSet<String>,
    pub active_grants: BTreeSet<(String, String)>,
}

impl PolicySnapshot {
    pub fn allow_origin(&mut self, origin: impl Into<String>) {
        self.allowed_origins.insert(origin.into());
    }

    pub fn grant(&mut self, principal: impl Into<String>, origin: impl Into<String>) {
        self.active_grants.insert((principal.into(), origin.into()));
    }
}

struct RegistryState {
    provider: ProviderSnapshot,
    policy: PolicySnapshot,
    capability_revision: u64,
    policy_revision: u64,
}

/// The authoritative catalog and dispatch decision source. Provider status is
/// fixture-backed; dispatch holds a read lock through handler admission so a
/// completed revocation cannot race past its final policy check.
pub struct CapabilityRegistry {
    state: RwLock<RegistryState>,
}

pub struct DispatchOutcome<T> {
    pub decision: CapabilityDecision,
    pub result: Option<T>,
}

impl CapabilityRegistry {
    pub fn new(provider: ProviderSnapshot, policy: PolicySnapshot) -> Self {
        Self {
            state: RwLock::new(RegistryState {
                provider,
                policy,
                capability_revision: 1,
                policy_revision: 1,
            }),
        }
    }

    pub fn catalog(
        &self,
        principal: &str,
        origin: &str,
        actions: &[&str],
    ) -> Vec<CapabilityDecision> {
        let state = self.state.read().expect("capability registry poisoned");
        let context = context(&state, principal, origin);
        actions
            .iter()
            .map(|action| evaluate_capability(&context, action))
            .collect()
    }

    pub fn revoke_origin(&self, origin: &str) -> u64 {
        let mut state = self.state.write().expect("capability registry poisoned");
        let prior = (
            state.policy.allowed_origins.len(),
            state.policy.active_grants.len(),
        );
        state.policy.allowed_origins.remove(origin);
        state
            .policy
            .active_grants
            .retain(|(_, grant_origin)| grant_origin != origin);
        let changed = prior
            != (
                state.policy.allowed_origins.len(),
                state.policy.active_grants.len(),
            );
        if changed {
            state.policy_revision += 1;
        }
        state.policy_revision
    }

    pub fn replace_provider(&self, provider: ProviderSnapshot) -> u64 {
        let mut state = self.state.write().expect("capability registry poisoned");
        state.provider = provider;
        state.capability_revision += 1;
        state.capability_revision
    }

    /// Invoke one bounded synchronous primitive while holding the registry
    /// read lock. Revocation linearizes after an admitted handler returns.
    /// Handlers must not reenter this registry; doing so can deadlock, and a
    /// long handler delays policy updates. A future async/multi-step executor
    /// needs reservation tokens or per-scope admission locks.
    pub fn dispatch<T>(
        &self,
        principal: &str,
        origin: &str,
        action: &str,
        handler: impl FnOnce() -> T,
    ) -> DispatchOutcome<T> {
        let state = self.state.read().expect("capability registry poisoned");
        let decision = evaluate_capability(&context(&state, principal, origin), action);
        let result = decision.qualified.then(handler);
        DispatchOutcome { decision, result }
    }
}

fn context(state: &RegistryState, principal: &str, origin: &str) -> CapabilityContext {
    CapabilityContext {
        direct: state.provider.direct,
        bridge: state.provider.bridge,
        configured: state.provider.configured,
        process_reachable: state.provider.process_reachable,
        authenticated: state.provider.authenticated,
        heartbeat_fresh: state.provider.heartbeat_fresh,
        origin_allowed: state.policy.allowed_origins.contains(origin),
        grant_active: state
            .policy
            .active_grants
            .contains(&(principal.to_owned(), origin.to_owned())),
        policy_revision: state.policy_revision,
        capability_revision: state.capability_revision,
        provider_source: state.provider.source,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityContext {
    pub direct: bool,
    pub bridge: bool,
    pub configured: bool,
    pub process_reachable: bool,
    pub authenticated: bool,
    pub heartbeat_fresh: bool,
    pub origin_allowed: bool,
    pub grant_active: bool,
    pub policy_revision: u64,
    pub capability_revision: u64,
    pub provider_source: AvailabilitySource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum DiagnosticState {
    Healthy,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    pub configured: DiagnosticState,
    pub process: DiagnosticState,
    pub binary_version: String,
    pub binary_hash: Option<String>,
    pub state_dir: String,
    pub principal_correlation: Option<String>,
    pub transport: DiagnosticState,
    pub authentication: DiagnosticState,
    pub heartbeat: DiagnosticState,
    pub heartbeat_age_seconds: Option<u64>,
    pub live_round_trip: DiagnosticState,
    pub policy_revision: Option<u64>,
    pub errors: Vec<String>,
}

pub fn evaluate_capability(ctx: &CapabilityContext, action: &str) -> CapabilityDecision {
    let supported = matches!(action, "observe" | "click" | "type" | "navigate");
    let route = if ctx.direct {
        Some(Route::Direct)
    } else if ctx.bridge {
        Some(Route::Bridge)
    } else {
        None
    };
    let reason = if !supported {
        Reason::UnsupportedAction
    } else if !ctx.configured {
        Reason::NotConfigured
    } else if route.is_none() || !ctx.process_reachable {
        Reason::Unreachable
    } else if !ctx.authenticated {
        Reason::AuthenticationRequired
    } else if !ctx.heartbeat_fresh {
        Reason::StaleHeartbeat
    } else if !ctx.origin_allowed {
        Reason::PolicyDenied
    } else if !ctx.grant_active {
        Reason::GrantRevoked
    } else {
        Reason::Ready
    };
    let reachable = route.is_some() && ctx.process_reachable;
    let authorized = ctx.origin_allowed && ctx.grant_active;
    let qualified = reason == Reason::Ready;
    CapabilityDecision {
        action: action.to_owned(),
        supported,
        configured: ctx.configured,
        reachable,
        authorized,
        qualified,
        reason,
        route,
        policy_revision: ctx.policy_revision,
        capability_revision: ctx.capability_revision,
        provider_source: ctx.provider_source,
    }
}

pub fn list_capabilities(ctx: &CapabilityContext, actions: &[&str]) -> Vec<CapabilityDecision> {
    actions
        .iter()
        .map(|action| evaluate_capability(ctx, action))
        .collect()
}

pub fn authorize_dispatch(ctx: &CapabilityContext, action: &str) -> CapabilityDecision {
    evaluate_capability(ctx, action)
}
