//! Internal recovery routing for refused pre-dispatch guards.
//!
//! Guard codes describe *why* an action was refused, not what the model should
//! do next. This module turns a guard into a bounded, non-mutating recovery
//! step that the runtime can execute before the refusal becomes model-visible.

use crate::guard_reason::{GuardReason, RetryClass};
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
    /// Switch to a frame-local or alternative input backend.
    SwitchBackend,
    /// Wait for the page to settle, then revalidate.
    WaitForSettling,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RecoveryDirective {
    pub class: RecoveryClass,
    pub max_attempts: u8,
    /// A recovery must never repeat an operation whose delivery is uncertain.
    pub idempotence_required: bool,
}

/// Route a guard to one bounded recovery ladder. Attempts count recovery
/// steps, not model turns; a mutation must not be replayed by recovery.
pub fn directive_for(reason: GuardReason) -> RecoveryDirective {
    use GuardReason as G;
    use RecoveryClass as R;
    match reason {
        G::OutsideViewport => RecoveryDirective {
            class: R::ScrollIntoView,
            max_attempts: 2,
            idempotence_required: true,
        },
        G::Obscured => RecoveryDirective {
            class: R::WaitForSettling,
            max_attempts: 2,
            idempotence_required: true,
        },
        G::StaleReference | G::Detached | G::ChangedTarget | G::DocumentChanged => {
            RecoveryDirective {
                class: R::RebindTarget,
                max_attempts: 1,
                idempotence_required: true,
            }
        }
        G::Hidden | G::NoHitTarget | G::PointerEventsNone | G::StaleValue => RecoveryDirective {
            class: R::Reobserve,
            max_attempts: 1,
            idempotence_required: true,
        },
        G::FrameLocalRouteRequired | G::CrossOriginFrameUnresolved => RecoveryDirective {
            class: R::SwitchBackend,
            max_attempts: 1,
            idempotence_required: true,
        },
        G::Disabled | G::AriaDisabled | G::ReadOnly | G::MaskedBoundary | G::TrustedBoundary => {
            RecoveryDirective {
                class: R::UserRequired,
                max_attempts: 0,
                idempotence_required: true,
            }
        }
        G::UnsupportedControl
        | G::UnsupportedRendererSurface
        | G::ProtectedBrowserSurface
        | G::NativeDialogRouteRequired => RecoveryDirective {
            class: R::TerminalPlatform,
            max_attempts: 0,
            idempotence_required: true,
        },
    }
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
    let directive = directive_for(reason);
    if directive.class.is_internal() && attempts < directive.max_attempts {
        RecoveryDecision::Retry(directive)
    } else {
        RecoveryDecision::Stop(directive.class)
    }
}

/// Attach routing metadata to a guard refusal. Internal classes keep the
/// model-visible receipt short; only terminal/user classes carry next_action.
pub fn attach_to_refusal(refusal: &mut serde_json::Value, reason: GuardReason) {
    let directive = directive_for(reason);
    let mut recovery = serde_json::json!({
        "class": directive.class.code(),
        "max_attempts": directive.max_attempts,
        "idempotence_required": directive.idempotence_required,
        "internal": directive.class.is_internal(),
    });
    if !directive.class.is_internal() {
        recovery["next_action"] = serde_json::json!(match reason.retry_class() {
            RetryClass::RequiresUser => "a person must cross this boundary; do not bypass it",
            _ => "this surface is unsupported for the requested operation; choose a supported path",
        });
    }
    refusal["recovery"] = recovery;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recoverable_guards_are_internal_and_bounded() {
        for reason in [
            GuardReason::OutsideViewport,
            GuardReason::Obscured,
            GuardReason::StaleReference,
            GuardReason::FrameLocalRouteRequired,
        ] {
            let directive = directive_for(reason);
            assert!(directive.class.is_internal(), "{reason:?}");
            assert!(directive.max_attempts >= 1, "{reason:?}");
            assert!(directive.idempotence_required);
            assert_eq!(decide(reason, 0), RecoveryDecision::Retry(directive));
            assert_eq!(
                decide(reason, directive.max_attempts),
                RecoveryDecision::Stop(directive.class)
            );
        }
    }

    #[test]
    fn safety_boundaries_never_retry() {
        for reason in [
            GuardReason::MaskedBoundary,
            GuardReason::TrustedBoundary,
            GuardReason::ReadOnly,
            GuardReason::UnsupportedRendererSurface,
            GuardReason::ProtectedBrowserSurface,
            GuardReason::NativeDialogRouteRequired,
        ] {
            let directive = directive_for(reason);
            assert!(!directive.class.is_internal(), "{reason:?}");
            assert_eq!(directive.max_attempts, 0);
            assert_eq!(
                decide(reason, 0),
                RecoveryDecision::Stop(directive.class)
            );
        }
    }

    #[test]
    fn refusal_receipt_reports_recovery_without_mutating() {
        let mut refusal = serde_json::json!({"status":"not_dispatched","dispatch_may_have_occurred":false});
        attach_to_refusal(&mut refusal, GuardReason::OutsideViewport);
        assert_eq!(refusal["recovery"]["class"], "scroll_into_view");
        assert_eq!(refusal["recovery"]["max_attempts"], 2);
        assert_eq!(refusal["dispatch_may_have_occurred"], false);
        assert!(refusal["recovery"].get("next_action").is_none());
        attach_to_refusal(&mut refusal, GuardReason::MaskedBoundary);
        assert_eq!(refusal["recovery"]["class"], "user_required");
        assert!(refusal["recovery"]["next_action"].is_string());
    }
}
