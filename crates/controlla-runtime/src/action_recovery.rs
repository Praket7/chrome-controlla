//! Internal recovery routing for refused pre-dispatch guards.
//!
//! Guard codes describe *why* an action was refused, not what the model should
//! do next. This module turns a guard into a bounded, non-mutating recovery
//! ladder that the runtime executes before any refusal becomes model-visible.
//!
//! A ladder names concrete internal routes (`trusted_cdp`, `frame_local_cdp`,
//! `semantic_reacquire`, `editable_descendant`, ...) so a backend mismatch is
//! resolved as a routing decision rather than reported to the caller as a
//! capability limit. Every rung is non-mutating: recovery may reobserve, wait,
//! scroll, rebind, or switch transport, but it never re-dispatches the effect
//! whose delivery is uncertain.

use crate::guard_reason::{ExecutionPolicy, GuardContext, GuardReason, RetryClass};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryClass {
    /// Take one fresh scoped observation.
    Reobserve,
    /// Scroll the nearest necessary container and revalidate geometry.
    ScrollIntoView,
    /// Rebind the stale reference through the target registry.
    RebindTarget,
    /// Switch to a frame-local, trusted, editor, or dialog backend.
    SwitchBackend,
    /// Wait for the page to settle, then revalidate.
    WaitForSettling,
    /// The caller must take one fresh observation (strict policy only).
    CallerRefresh,
    /// Only a person can cross this boundary.
    UserRequired,
    /// The surface or control is permanently unsupported for this operation.
    TerminalPlatform,
}

impl RecoveryClass {
    pub fn code(self) -> &'static str {
        match self {
            Self::Reobserve => "reobserve",
            Self::ScrollIntoView => "scroll_into_view",
            Self::RebindTarget => "rebind_target",
            Self::SwitchBackend => "switch_backend",
            Self::WaitForSettling => "wait_for_settling",
            Self::CallerRefresh => "caller_refresh",
            Self::UserRequired => "user_required",
            Self::TerminalPlatform => "terminal_platform",
        }
    }

    /// Whether the runtime may execute this recovery itself. Internal steps
    /// never mutate: they observe, scroll, rebind, or switch transport.
    pub fn is_internal(self) -> bool {
        matches!(
            self,
            Self::Reobserve
                | Self::ScrollIntoView
                | Self::RebindTarget
                | Self::SwitchBackend
                | Self::WaitForSettling
        )
    }
}

/// One rung of a recovery ladder.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RecoveryDirective {
    pub class: RecoveryClass,
    /// The concrete internal route this rung selects, if any. Receipts name it
    /// so a failed ladder is diagnosable without re-deriving the decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<&'static str>,
    pub max_attempts: u8,
    /// A recovery must never repeat an operation whose delivery is uncertain.
    pub idempotence_required: bool,
}

impl RecoveryDirective {
    const fn internal(
        class: RecoveryClass,
        backend: &'static str,
        max_attempts: u8,
    ) -> RecoveryDirective {
        RecoveryDirective {
            class,
            backend: Some(backend),
            max_attempts,
            idempotence_required: true,
        }
    }

    const fn terminal(class: RecoveryClass) -> RecoveryDirective {
        RecoveryDirective {
            class,
            backend: None,
            max_attempts: 0,
            idempotence_required: true,
        }
    }
}

/// Ordered recovery ladder for a guard. The first internal rung is the primary
/// recovery; later rungs are tried only if the earlier one does not produce a
/// dispatchable target. The list is bounded at four rungs.
pub fn plan_for(reason: GuardReason, context: &GuardContext) -> Vec<RecoveryDirective> {
    use GuardReason as G;
    use RecoveryClass as R;
    if context.policy == ExecutionPolicy::Strict {
        // Strict callers asked to see route problems. Only the two conditions
        // the old contract already treated as internal keep their ladder.
        if matches!(
            reason,
            G::OutsideViewport
                | G::Obscured
                | G::FrameLocalRouteRequired
                | G::CrossOriginFrameUnresolved
        ) {
            return execution_first_plan(reason, context);
        }
        return vec![match reason.retry_class_with(context) {
            RetryClass::RequiresUser => RecoveryDirective::terminal(R::UserRequired),
            RetryClass::RefreshSnapshot => RecoveryDirective::terminal(R::CallerRefresh),
            _ => RecoveryDirective::terminal(R::TerminalPlatform),
        }];
    }
    execution_first_plan(reason, context)
}

fn execution_first_plan(reason: GuardReason, context: &GuardContext) -> Vec<RecoveryDirective> {
    use GuardReason as G;
    use RecoveryClass as R;
    match reason {
        G::OutsideViewport => vec![RecoveryDirective::internal(
            R::ScrollIntoView,
            "nearest_scroll_container",
            2,
        )],
        G::Obscured => vec![
            RecoveryDirective::internal(R::WaitForSettling, "settle_then_reobserve", 2),
            RecoveryDirective::internal(R::Reobserve, "scoped_subtree", 1),
        ],
        G::NoHitTarget => vec![RecoveryDirective::internal(
            R::WaitForSettling,
            "layout_settle_hit_recheck",
            1,
        )],
        G::PointerEventsNone => vec![RecoveryDirective::internal(
            R::SwitchBackend,
            "ancestor_pointer_target",
            1,
        )],
        G::StaleReference | G::Detached | G::ChangedTarget => vec![RecoveryDirective::internal(
            R::RebindTarget,
            "semantic_reacquire",
            1,
        )],
        G::StaleValue => vec![RecoveryDirective::internal(
            R::Reobserve,
            "value_precondition_recheck",
            1,
        )],
        G::Hidden => vec![RecoveryDirective::internal(
            R::Reobserve,
            "scoped_subtree",
            1,
        )],
        G::DocumentChanged => vec![RecoveryDirective::internal(
            R::Reobserve,
            "document_reobserve",
            1,
        )],
        G::Disabled | G::AriaDisabled => vec![RecoveryDirective::internal(
            R::WaitForSettling,
            "wait_for_enable",
            1,
        )],
        G::FrameLocalRouteRequired | G::CrossOriginFrameUnresolved => vec![
            RecoveryDirective::internal(R::SwitchBackend, "frame_local_cdp", 1),
            RecoveryDirective::internal(R::SwitchBackend, "oopif_child_session", 1),
        ],
        G::TrustedBoundary => vec![RecoveryDirective::internal(
            R::SwitchBackend,
            "trusted_cdp",
            1,
        )],
        G::MaskedBoundary if context.secret_authorized => vec![RecoveryDirective::internal(
            R::SwitchBackend,
            "secure_trusted_input",
            1,
        )],
        G::MaskedBoundary => vec![RecoveryDirective::terminal(R::UserRequired)],
        G::ReadOnly => vec![
            RecoveryDirective::internal(R::SwitchBackend, "editable_descendant", 1),
            RecoveryDirective::internal(R::SwitchBackend, "editor_adapter", 1),
            RecoveryDirective::internal(R::Reobserve, "scoped_subtree", 1),
        ],
        G::UnsupportedControl => vec![RecoveryDirective::internal(
            R::SwitchBackend,
            "semantic_control_route",
            1,
        )],
        G::UnsupportedRendererSurface => vec![
            RecoveryDirective::internal(R::SwitchBackend, "oopif_child_session", 1),
            RecoveryDirective::internal(R::SwitchBackend, "shadow_or_same_process_context", 1),
            RecoveryDirective::internal(R::SwitchBackend, "trusted_pointer", 1),
        ],
        G::NativeDialogRouteRequired => vec![RecoveryDirective::internal(
            R::SwitchBackend,
            "dialog_route",
            1,
        )],
        G::ProtectedBrowserSurface => vec![RecoveryDirective::terminal(R::TerminalPlatform)],
    }
}

/// Primary recovery rung. Retained for callers that only need one step.
pub fn directive_for(reason: GuardReason) -> RecoveryDirective {
    plan_for(reason, &GuardContext::execution_first())
        .into_iter()
        .next()
        .expect("every guard has at least one recovery rung")
}

pub fn directive_for_with(reason: GuardReason, context: &GuardContext) -> RecoveryDirective {
    plan_for(reason, context)
        .into_iter()
        .next()
        .expect("every guard has at least one recovery rung")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryDecision {
    /// Run this internal step, then revalidate before dispatch.
    Retry(RecoveryDirective),
    /// Stop retrying; return one compact refusal to the caller.
    Stop(RecoveryClass),
}

/// Decide whether another attempt is allowed. `attempts` is the number of
/// recoveries already performed for this guard in this action.
pub fn decide(reason: GuardReason, attempts: u8) -> RecoveryDecision {
    decide_with(reason, attempts, &GuardContext::execution_first())
}

pub fn decide_with(reason: GuardReason, attempts: u8, context: &GuardContext) -> RecoveryDecision {
    let directive = directive_for_with(reason, context);
    if directive.class.is_internal() && attempts < directive.max_attempts {
        RecoveryDecision::Retry(directive)
    } else {
        RecoveryDecision::Stop(directive.class)
    }
}

/// Bounded executor-facing ledger for one guarded action.
///
/// The runtime calls [`RecoveryLedger::next`] before each revalidation. A rung
/// is consumed at most once, so a stuck page can never cause an unbounded
/// recovery loop, and a terminal ladder yields no steps at all.
#[derive(Clone, Debug)]
pub struct RecoveryLedger {
    plan: Vec<RecoveryDirective>,
    consumed: usize,
}

impl RecoveryLedger {
    pub fn new(reason: GuardReason) -> Self {
        Self::with_context(reason, &GuardContext::execution_first())
    }

    pub fn with_context(reason: GuardReason, context: &GuardContext) -> Self {
        Self {
            plan: plan_for(reason, context),
            consumed: 0,
        }
    }

    /// Next internal rung, or `None` when the ladder is terminal/exhausted.
    pub fn next_rung(&mut self) -> Option<RecoveryDirective> {
        while let Some(directive) = self.plan.get(self.consumed).copied() {
            self.consumed += 1;
            if directive.class.is_internal() {
                return Some(directive);
            }
        }
        None
    }

    pub fn attempted(&self) -> usize {
        self.consumed
    }

    /// True when every rung is internal, so no model turn is required.
    pub fn automatic(&self) -> bool {
        !self.plan.is_empty() && self.plan.iter().all(|rung| rung.class.is_internal())
    }

    /// The class the caller sees if every internal rung fails.
    pub fn terminal_class(&self) -> RecoveryClass {
        self.plan
            .last()
            .map(|rung| rung.class)
            .unwrap_or(RecoveryClass::TerminalPlatform)
    }

    pub fn plan(&self) -> &[RecoveryDirective] {
        &self.plan
    }
}

/// Attach routing metadata to a guard refusal. Internal classes keep the
/// model-visible receipt short and state that no caller action is needed; only
/// terminal/user classes carry `next_action`.
pub fn attach_to_refusal(refusal: &mut serde_json::Value, reason: GuardReason) {
    attach_to_refusal_with(refusal, reason, &GuardContext::execution_first());
}

pub fn attach_to_refusal_with(
    refusal: &mut serde_json::Value,
    reason: GuardReason,
    context: &GuardContext,
) {
    let plan = plan_for(reason, context);
    let primary = plan
        .first()
        .copied()
        .expect("every guard has at least one recovery rung");
    let automatic = plan.iter().all(|rung| rung.class.is_internal());
    // Keep the object small: a receipt must survive response-budget
    // compaction, so it names the route and the bound, not the whole planning
    // table. `automatic_recovery` and `model_action_required` are top-level.
    let mut recovery = serde_json::json!({
        "class": primary.class.code(),
        "max_attempts": primary.max_attempts,
        "internal": primary.class.is_internal(),
    });
    if let Some(backend) = primary.backend {
        recovery["backend"] = serde_json::json!(backend);
    }
    if !primary.class.is_internal() {
        recovery["next_action"] = serde_json::json!(next_action(reason, context));
    }
    refusal["recovery"] = recovery;
    refusal["automatic_recovery"] = serde_json::json!(automatic);
    refusal["model_action_required"] = serde_json::json!(!automatic);
}

fn next_action(reason: GuardReason, context: &GuardContext) -> &'static str {
    match reason {
        GuardReason::MaskedBoundary if !context.secret_authorized => {
            "a person must authorize secret entry; do not bypass this boundary"
        }
        GuardReason::ProtectedBrowserSurface => {
            "Chrome owns this surface; ask the person to complete it"
        }
        _ => match reason.retry_class_with(context) {
            RetryClass::RequiresUser => "a person must cross this boundary; do not bypass it",
            RetryClass::RefreshSnapshot => "take one fresh observation before retrying",
            RetryClass::SafeInternal => "one internal recovery step runs before redispatch",
            RetryClass::DoNotRetry => {
                "this surface is unsupported for the requested operation; choose a supported path"
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTING_REASONS: [GuardReason; 12] = [
        GuardReason::TrustedBoundary,
        GuardReason::ReadOnly,
        GuardReason::UnsupportedControl,
        GuardReason::UnsupportedRendererSurface,
        GuardReason::NativeDialogRouteRequired,
        GuardReason::FrameLocalRouteRequired,
        GuardReason::CrossOriginFrameUnresolved,
        GuardReason::StaleReference,
        GuardReason::Detached,
        GuardReason::OutsideViewport,
        GuardReason::Obscured,
        GuardReason::Disabled,
    ];

    #[test]
    fn routing_problems_have_automatic_ladders_naming_a_backend() {
        for reason in ROUTING_REASONS {
            let mut ledger = RecoveryLedger::new(reason);
            assert!(ledger.automatic(), "{reason:?}");
            assert!(
                (1..=4).contains(&ledger.plan().len()),
                "{reason:?} ladder must be bounded"
            );
            let mut rungs = 0;
            while let Some(rung) = ledger.next_rung() {
                rungs += 1;
                assert!(rung.class.is_internal(), "{reason:?}");
                assert!(rung.backend.is_some(), "{reason:?} must name its route");
                assert!(rung.idempotence_required);
                assert!(rung.max_attempts >= 1);
            }
            assert_eq!(rungs, ledger.plan().len(), "{reason:?}");
            assert_eq!(ledger.next_rung(), None, "{reason:?} ladder must terminate");
        }
    }

    #[test]
    fn recovery_never_replays_a_mutation_and_consumes_each_rung_once() {
        let mut ledger = RecoveryLedger::new(GuardReason::ReadOnly);
        let mut rungs = Vec::new();
        while let Some(rung) = ledger.next_rung() {
            rungs.push(rung.backend);
        }
        assert_eq!(
            rungs,
            vec![
                Some("editable_descendant"),
                Some("editor_adapter"),
                Some("scoped_subtree")
            ]
        );
        assert_eq!(ledger.next_rung(), None);
        assert_eq!(ledger.attempted(), 3);
    }

    #[test]
    fn secret_entry_is_a_boundary_only_without_authorization() {
        let unauthorized = GuardContext::execution_first();
        let mut ledger = RecoveryLedger::with_context(GuardReason::MaskedBoundary, &unauthorized);
        assert!(!ledger.automatic());
        assert_eq!(ledger.next_rung(), None);
        assert_eq!(ledger.terminal_class(), RecoveryClass::UserRequired);

        let authorized = unauthorized.with_secret_authorization(true);
        let mut ledger = RecoveryLedger::with_context(GuardReason::MaskedBoundary, &authorized);
        assert!(ledger.automatic());
        assert_eq!(
            ledger.next_rung().and_then(|rung| rung.backend),
            Some("secure_trusted_input")
        );
    }

    #[test]
    fn chrome_owned_surfaces_stay_terminal() {
        let mut ledger = RecoveryLedger::new(GuardReason::ProtectedBrowserSurface);
        assert!(!ledger.automatic());
        assert_eq!(ledger.next_rung(), None);
        assert_eq!(ledger.terminal_class(), RecoveryClass::TerminalPlatform);
    }

    #[test]
    fn strict_policy_keeps_the_conservative_ladder() {
        let strict = GuardContext::strict();
        let mut ledger = RecoveryLedger::with_context(GuardReason::TrustedBoundary, &strict);
        assert!(!ledger.automatic());
        assert_eq!(ledger.next_rung(), None);
        let ledger = RecoveryLedger::new(GuardReason::TrustedBoundary);
        assert!(ledger.automatic());
    }

    #[test]
    fn refusal_receipt_reports_recovery_without_mutating() {
        let mut refusal =
            serde_json::json!({"status":"not_dispatched","dispatch_may_have_occurred":false});
        attach_to_refusal(&mut refusal, GuardReason::OutsideViewport);
        assert_eq!(refusal["recovery"]["class"], "scroll_into_view");
        assert_eq!(refusal["recovery"]["backend"], "nearest_scroll_container");
        assert_eq!(refusal["recovery"]["max_attempts"], 2);
        assert_eq!(refusal["automatic_recovery"], true);
        assert_eq!(refusal["model_action_required"], false);
        assert_eq!(refusal["dispatch_may_have_occurred"], false);
        assert!(refusal["recovery"].get("next_action").is_none());
        attach_to_refusal(&mut refusal, GuardReason::MaskedBoundary);
        assert_eq!(refusal["recovery"]["class"], "user_required");
        assert_eq!(refusal["automatic_recovery"], false);
        assert!(refusal["recovery"]["next_action"].is_string());
    }

    #[test]
    fn backend_mismatch_is_never_a_user_refusal_under_execution_first() {
        for reason in ROUTING_REASONS {
            let mut refusal = serde_json::json!({"status":"not_dispatched"});
            attach_to_refusal(&mut refusal, reason);
            assert_eq!(
                refusal["model_action_required"], false,
                "{reason:?}: {refusal}"
            );
            assert_eq!(refusal["recovery"]["internal"], true, "{reason:?}");
        }
    }
}
