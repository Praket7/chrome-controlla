//! Execution of a recovery ladder against a live target.
//!
//! [`crate::action_recovery`] decides *which* internal route a guard mismatch
//! needs. This module turns a rung into the concrete step a dispatch site can
//! run, and records what ran so a receipt proves the recovery happened instead
//! of merely being proposed.
//!
//! Two rules keep execution honest:
//!
//! * every step is non-mutating except a route that reaches the same target by
//!   a different transport, and that transport is used at most once;
//! * a step that cannot resolve the target consumes its rung and the ladder
//!   continues, so a stuck page can never loop.
//!
//! The page-side scripts live in `shared_page/recovery_*.js` and are injected
//! verbatim, which keeps them testable as text without a browser.

use crate::action_recovery::{RecoveryDirective, RecoveryLedger};
use crate::guard_reason::{GuardReason, RetryClass};
use serde_json::{Value, json};

/// Upper bound on steps a single action may execute. Mirrors the ladder bound.
pub const MAX_LADDER_STEPS: usize = 4;

/// Page-side scripts, injected verbatim by the dispatch sites.
pub const ANCHOR_SCRIPT: &str = include_str!("shared_page/recovery_anchor.js");
pub const REACQUIRE_SCRIPT: &str = include_str!("shared_page/recovery_reacquire.js");
pub const EDITABLE_SCRIPT: &str = include_str!("shared_page/recovery_editable.js");

/// One executable step derived from a recovery rung.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RungOperation {
    /// Scroll the nearest container with the given alignment, then revalidate.
    Rescroll { block: &'static str },
    /// Wait for the page to settle, then revalidate.
    Settle { delay_ms: u64 },
    /// Wait for a control to leave its disabled state, then revalidate.
    WaitForEnable,
    /// Rebind `nodes[index]` by semantic identity, then revalidate.
    Reacquire,
    /// Reach the same control through an ancestor that accepts pointer events.
    AncestorPointer,
    /// Dispatch trusted pointer events at the control's own geometry.
    TrustedPointer,
    /// Bind the editable descendant of a wrapper control.
    EditableDescendant,
    /// Re-read the value precondition before deciding.
    ValueRecheck,
    /// Re-observe the target or document and revalidate.
    Revalidate,
    /// The route needs a dispatch site this operation does not own.
    Unsupported,
}

impl RungOperation {
    pub fn code(self) -> &'static str {
        match self {
            Self::Rescroll { .. } => "rescroll",
            Self::Settle { .. } => "settle",
            Self::WaitForEnable => "wait_for_enable",
            Self::Reacquire => "reacquire",
            Self::AncestorPointer => "ancestor_pointer",
            Self::TrustedPointer => "trusted_pointer",
            Self::EditableDescendant => "editable_descendant",
            Self::ValueRecheck => "value_recheck",
            Self::Revalidate => "revalidate",
            Self::Unsupported => "unsupported",
        }
    }

    /// Whether the step changes what the next guard evaluation sees.
    pub fn is_executable(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

/// Map a rung's named backend to the step that implements it. The mapping is
/// exhaustive over the backends `action_recovery::plan_for` can emit; an
/// unknown backend is [`RungOperation::Unsupported`] rather than a silent
/// success.
pub fn operation_for(directive: RecoveryDirective) -> RungOperation {
    match directive.backend {
        Some("nearest_scroll_container") => RungOperation::Rescroll { block: "center" },
        Some("settle_then_reobserve") => RungOperation::Settle { delay_ms: 40 },
        Some("layout_settle_hit_recheck") => RungOperation::Settle { delay_ms: 60 },
        Some("wait_for_enable") => RungOperation::WaitForEnable,
        Some("semantic_reacquire") => RungOperation::Reacquire,
        Some("ancestor_pointer_target") => RungOperation::AncestorPointer,
        Some("trusted_cdp" | "trusted_pointer") => RungOperation::TrustedPointer,
        Some("editable_descendant" | "editor_adapter") => RungOperation::EditableDescendant,
        // A control the requested operation does not support is still
        // reachable by a trusted pointer when the control itself is a valid
        // click target (checkbox, radio, slider thumb).
        Some("semantic_control_route") => RungOperation::TrustedPointer,
        Some("value_precondition_recheck") => RungOperation::ValueRecheck,
        Some("scoped_subtree" | "document_reobserve" | "shadow_or_same_process_context") => {
            RungOperation::Revalidate
        }
        _ => RungOperation::Unsupported,
    }
}

/// Ledger of what an action actually executed.
#[derive(Clone, Debug, Default)]
pub struct Executed {
    steps: Vec<&'static str>,
    resolved: Option<&'static str>,
}

impl Executed {
    pub fn record(&mut self, operation: RungOperation) {
        if self.steps.len() < MAX_LADDER_STEPS {
            self.steps.push(operation.code());
        }
    }

    /// Record the transport that resolved the guard and revalidated.
    pub fn resolve(&mut self, via: &'static str) {
        self.resolved = Some(via);
    }

    /// Mark a ladder that produced a dispatchable revalidation. A no-op when
    /// no step ran, so a plain guard pass never claims a recovery.
    pub fn mark_resolved(&mut self) {
        if !self.steps.is_empty() && self.resolved.is_none() {
            self.resolved = Some("revalidated");
        }
    }

    pub fn steps(&self) -> &[&'static str] {
        &self.steps
    }

    pub fn resolved_via(&self) -> Option<&'static str> {
        self.resolved
    }

    pub fn ran_any(&self) -> bool {
        !self.steps.is_empty()
    }

    /// Attach execution evidence to a receipt. A resolved recovery is reported
    /// as automatic so no caller retry is implied.
    pub fn attach(&self, receipt: &mut Value) {
        if !self.ran_any() && self.resolved.is_none() {
            return;
        }
        if !receipt.get("recovery").is_some_and(Value::is_object) {
            receipt["recovery"] = json!({});
        }
        receipt["recovery"]["executed"] = json!(self.steps);
        receipt["recovery"]["resolved"] = json!(self.resolved.is_some());
        receipt["recovery"]["exhausted"] = json!(self.resolved.is_none());
        if let Some(via) = self.resolved {
            receipt["recovery"]["resolved_via"] = json!(via);
            receipt["automatic_recovery"] = json!(true);
            receipt["model_action_required"] = json!(false);
            receipt["next_action"] = Value::Null;
        }
        if let Some(recovery) = receipt["recovery"].as_object_mut() {
            recovery.remove("next_action");
        }
    }
}

/// Parse the guard payload returned by the page into a typed reason.
pub fn reason_of(checked: &Value) -> Option<GuardReason> {
    checked["reason"].as_str().and_then(GuardReason::from_code)
}

/// Build the refusal for an exhausted ladder. The primary reason is preserved;
/// execution evidence is added so the caller can tell "recovered" from
/// "recovery attempted and failed".
pub fn exhausted_refusal(checked: &Value, executed: &Executed) -> Value {
    let mut refusal = crate::guard_reason::refusal(checked);
    executed.attach(&mut refusal);
    if let Some(reason) = reason_of(checked)
        && reason.retry_class() == RetryClass::SafeInternal
    {
        // Every internal rung ran and the target still could not be reached.
        // That is a real observable state, not a capability refusal: keep the
        // reason concrete and let `recovery.exhausted` say recovery ran.
        refusal["recovery"]["stage"] = json!("internal_ladder_exhausted");
    }
    refusal
}

/// Whether a guard payload is a successful, dispatchable check.
pub fn dispatchable(checked: &Value) -> bool {
    checked["ok"] == true
}

/// Map an editor-preflight reason to the guard taxonomy. The editor guard
/// names its predicate differently from the pointer guard, so dispatch sites
/// translate once instead of inventing a second recovery table.
pub fn guard_reason_for_preflight(reason: &str) -> Option<GuardReason> {
    match reason {
        "not_editable" | "blocked" | "guard_rejected" => Some(GuardReason::ReadOnly),
        "target_replaced" | "detached" => Some(GuardReason::StaleReference),
        "focus_or_selection_changed" => Some(GuardReason::TrustedBoundary),
        "stale_value" | "option_not_found" | "same_field_conflict" => Some(GuardReason::StaleValue),
        "interaction_epoch_changed" => Some(GuardReason::DocumentChanged),
        "unsupported_control" => Some(GuardReason::UnsupportedControl),
        "not_visible" => Some(GuardReason::Hidden),
        _ => GuardReason::from_code(reason),
    }
}

/// Remaining rungs for diagnostics. Never more than [`MAX_LADDER_STEPS`].
pub fn remaining(ledger: &RecoveryLedger) -> usize {
    ledger.plan().len().saturating_sub(ledger.attempted())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_recovery::{RecoveryLedger, plan_for};
    use crate::guard_reason::{GuardContext, GuardReason};

    const BACKEND_BEARING: [GuardReason; 12] = [
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
    fn every_planned_rung_is_classified_and_owned_routes_are_executable() {
        let context = GuardContext::execution_first();
        let owned = [
            GuardReason::TrustedBoundary,
            GuardReason::ReadOnly,
            GuardReason::UnsupportedControl,
            GuardReason::StaleReference,
            GuardReason::Detached,
            GuardReason::OutsideViewport,
            GuardReason::Obscured,
            GuardReason::Disabled,
        ];
        for reason in BACKEND_BEARING {
            for rung in plan_for(reason, &context) {
                // Every rung must map somewhere. A route this site does not own
                // stays explicitly `Unsupported` so the executor skips it
                // instead of pretending it ran.
                let operation = operation_for(rung);
                if reason == GuardReason::ProtectedBrowserSurface {
                    assert_eq!(operation, RungOperation::Unsupported);
                }
            }
            if owned.contains(&reason) {
                let primary = operation_for(plan_for(reason, &context)[0]);
                assert!(
                    primary.is_executable(),
                    "{reason:?} must execute its primary rung, got {primary:?}"
                );
            }
        }
        // Routes owned by a frame or dialog dispatch site are visible as such.
        for backend in [
            "frame_local_cdp",
            "oopif_child_session",
            "dialog_route",
            "secure_trusted_input",
        ] {
            let directive = crate::action_recovery::RecoveryDirective {
                class: crate::action_recovery::RecoveryClass::SwitchBackend,
                backend: Some(backend),
                max_attempts: 1,
                idempotence_required: true,
            };
            assert_eq!(
                operation_for(directive),
                RungOperation::Unsupported,
                "{backend}"
            );
        }
    }

    #[test]
    fn viewport_and_pointer_recoveries_are_distinct_operations() {
        assert_eq!(
            operation_for(RecoveryDirective {
                class: crate::action_recovery::RecoveryClass::ScrollIntoView,
                backend: Some("nearest_scroll_container"),
                max_attempts: 2,
                idempotence_required: true,
            }),
            RungOperation::Rescroll { block: "center" }
        );
        assert_eq!(
            operation_for(RecoveryDirective {
                class: crate::action_recovery::RecoveryClass::SwitchBackend,
                backend: Some("ancestor_pointer_target"),
                max_attempts: 1,
                idempotence_required: true,
            }),
            RungOperation::AncestorPointer
        );
        assert_eq!(
            operation_for(RecoveryDirective {
                class: crate::action_recovery::RecoveryClass::SwitchBackend,
                backend: Some("frame_local_cdp"),
                max_attempts: 1,
                idempotence_required: true,
            }),
            RungOperation::Unsupported,
            "a frame route belongs to the frame dispatch site, not the click site"
        );
    }

    #[test]
    fn execution_is_bounded_and_records_at_most_four_steps() {
        let mut executed = Executed::default();
        for _ in 0..10 {
            executed.record(RungOperation::Revalidate);
        }
        assert_eq!(executed.steps().len(), MAX_LADDER_STEPS);
        executed.resolve("ancestor_pointer_target");
        let mut receipt = json!({"status":"not_dispatched","model_action_required":true});
        executed.attach(&mut receipt);
        assert_eq!(receipt["recovery"]["executed"].as_array().unwrap().len(), 4);
        assert_eq!(receipt["recovery"]["resolved"], true);
        assert_eq!(
            receipt["recovery"]["resolved_via"],
            "ancestor_pointer_target"
        );
        assert_eq!(receipt["model_action_required"], false);
        assert!(receipt["next_action"].is_null());
    }

    #[test]
    fn a_resolved_ladder_never_claims_exhaustion() {
        let mut executed = Executed::default();
        executed.mark_resolved();
        assert_eq!(
            executed.resolved_via(),
            None,
            "no step ran, no recovery claimed"
        );
        executed.record(RungOperation::WaitForEnable);
        executed.mark_resolved();
        assert_eq!(executed.resolved_via(), Some("revalidated"));
        let mut receipt = json!({"status":"verified"});
        executed.attach(&mut receipt);
        assert_eq!(receipt["recovery"]["exhausted"], false);
        assert_eq!(receipt["recovery"]["executed"], json!(["wait_for_enable"]));
    }

    #[test]
    fn an_exhausted_ladder_keeps_the_concrete_reason_and_records_the_attempt() {
        let checked = json!({"ok":false,"reason":"pointer_events_none"});
        let mut executed = Executed::default();
        executed.record(RungOperation::AncestorPointer);
        let refusal = exhausted_refusal(&checked, &executed);
        assert_eq!(refusal["status"], "not_dispatched");
        assert_eq!(refusal["reason_code"], "pointer_events_none");
        assert_eq!(refusal["recovery"]["executed"], json!(["ancestor_pointer"]));
        assert_eq!(refusal["recovery"]["exhausted"], true);
        assert_eq!(refusal["recovery"]["stage"], "internal_ladder_exhausted");
        assert!(refusal["recovery"].get("next_action").is_none());
    }

    #[test]
    fn a_ledger_never_reports_more_rungs_than_it_planned() {
        let mut ledger = RecoveryLedger::new(GuardReason::ReadOnly);
        assert_eq!(remaining(&ledger), 3);
        assert!(ledger.next_rung().is_some());
        assert_eq!(remaining(&ledger), 2);
        while ledger.next_rung().is_some() {}
        assert_eq!(remaining(&ledger), 0);
    }

    #[test]
    fn editor_preflight_reasons_reach_executable_rungs() {
        for (reason, expected) in [
            ("not_editable", RungOperation::EditableDescendant),
            ("blocked", RungOperation::EditableDescendant),
            ("target_replaced", RungOperation::Reacquire),
            ("focus_or_selection_changed", RungOperation::TrustedPointer),
            ("stale_value", RungOperation::ValueRecheck),
        ] {
            let guard = guard_reason_for_preflight(reason).expect(reason);
            let mut ledger = RecoveryLedger::new(guard);
            let operation = ledger.next_rung().map(operation_for).expect(reason);
            assert_eq!(operation, expected, "{reason}");
        }
        assert_eq!(guard_reason_for_preflight("never_a_reason"), None);
    }

    #[test]
    fn dispatchable_requires_an_explicit_ok() {
        assert!(dispatchable(&json!({"ok":true})));
        assert!(!dispatchable(&json!({"ok":false})));
        assert!(!dispatchable(&json!({})));
    }
}
