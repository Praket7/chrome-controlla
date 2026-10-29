//! Typed pre-dispatch guard diagnostics.
//!
//! The page guard (`shared_page/guard.js`) reports exactly which predicate
//! failed. This module is the single Rust-side contract for those codes, their
//! retry classification, and the structured refusal that MCP routes return.
//! A generic `blocked` result is never produced.
//!
//! ## Backend mismatch is not a boundary
//!
//! Most guard codes describe a *routing* problem: the requested element needs a
//! trusted CDP backend, a frame-local session, an editor adapter, a dialog
//! route, or one rebind. Under the default [`ExecutionPolicy::ExecutionFirst`]
//! those reasons are internal recoveries and never become a model-visible
//! refusal. Only genuinely external conditions — a Chrome-owned surface, an
//! unauthorized secret entry, or a policy that forbids the route — stay
//! terminal. [`ExecutionPolicy::Strict`] restores the older conservative
//! classification for callers that ask for it.

use serde::Serialize;
use serde_json::{Value, json};

/// Machine-readable cause of a refused pre-dispatch guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardReason {
    /// The control lives in a child frame and must use the frame-local route.
    FrameLocalRouteRequired,
    /// A protected browser surface (permissions, chooser, native dialog).
    ProtectedBrowserSurface,
    /// The renderer surface is not supported for this operation.
    UnsupportedRendererSurface,
    /// The frame exists but its document cannot be resolved cross-origin.
    CrossOriginFrameUnresolved,
    /// A native dialog needs its own interaction route.
    NativeDialogRouteRequired,
    StaleReference,
    ChangedTarget,
    Hidden,
    Disabled,
    AriaDisabled,
    MaskedBoundary,
    TrustedBoundary,
    PointerEventsNone,
    OutsideViewport,
    NoHitTarget,
    Obscured,
    Detached,
    DocumentChanged,
    /// Control type is not supported by this operation (for example filling a checkbox).
    UnsupportedControl,
    /// Control is read-only.
    ReadOnly,
    /// Typed-value or caret precondition no longer holds.
    StaleValue,
}

/// How much the caller has authorized the runtime to resolve internally.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPolicy {
    /// Resolve every route, backend, frame, and rebind problem inside the
    /// runtime. A refusal is returned only for a real boundary.
    #[default]
    ExecutionFirst,
    /// Surface route and backend problems to the caller instead of resolving
    /// them. Retained for callers that want the pre-execution-first contract.
    Strict,
}

impl ExecutionPolicy {
    pub fn code(self) -> &'static str {
        match self {
            Self::ExecutionFirst => "execution_first",
            Self::Strict => "strict",
        }
    }
}

/// Per-action authorization context. It never widens what the user asked for;
/// it only decides whether a route the caller already authorized may be used.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GuardContext {
    pub policy: ExecutionPolicy,
    /// The caller explicitly authorized secret entry for this action.
    pub secret_authorized: bool,
}

impl GuardContext {
    pub const fn execution_first() -> Self {
        Self {
            policy: ExecutionPolicy::ExecutionFirst,
            secret_authorized: false,
        }
    }

    pub const fn strict() -> Self {
        Self {
            policy: ExecutionPolicy::Strict,
            secret_authorized: false,
        }
    }

    pub const fn with_secret_authorization(mut self, authorized: bool) -> Self {
        self.secret_authorized = authorized;
        self
    }
}

/// What a caller (or the runtime) may do after a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryClass {
    /// The runtime may repair this itself without another model turn (bounded, non-mutating).
    SafeInternal,
    /// Take one fresh snapshot, then decide.
    RefreshSnapshot,
    /// A person must act; Controlla must not bypass the boundary.
    RequiresUser,
    /// Do not retry; the boundary is permanent for this target.
    DoNotRetry,
}

impl GuardReason {
    pub const ALL: [GuardReason; 21] = [
        GuardReason::FrameLocalRouteRequired,
        GuardReason::ProtectedBrowserSurface,
        GuardReason::UnsupportedRendererSurface,
        GuardReason::CrossOriginFrameUnresolved,
        GuardReason::NativeDialogRouteRequired,
        GuardReason::StaleReference,
        GuardReason::ChangedTarget,
        GuardReason::Hidden,
        GuardReason::Disabled,
        GuardReason::AriaDisabled,
        GuardReason::MaskedBoundary,
        GuardReason::TrustedBoundary,
        GuardReason::PointerEventsNone,
        GuardReason::OutsideViewport,
        GuardReason::NoHitTarget,
        GuardReason::Obscured,
        GuardReason::Detached,
        GuardReason::DocumentChanged,
        GuardReason::UnsupportedControl,
        GuardReason::ReadOnly,
        GuardReason::StaleValue,
    ];

    pub fn code(self) -> &'static str {
        match self {
            GuardReason::FrameLocalRouteRequired => "frame_local_route_required",
            GuardReason::ProtectedBrowserSurface => "protected_browser_surface",
            GuardReason::UnsupportedRendererSurface => "unsupported_renderer_surface",
            GuardReason::CrossOriginFrameUnresolved => "cross_origin_frame_unresolved",
            GuardReason::NativeDialogRouteRequired => "native_dialog_route_required",
            GuardReason::StaleReference => "stale_reference",
            GuardReason::ChangedTarget => "changed_target",
            GuardReason::Hidden => "hidden",
            GuardReason::Disabled => "disabled",
            GuardReason::AriaDisabled => "aria_disabled",
            GuardReason::MaskedBoundary => "masked_boundary",
            GuardReason::TrustedBoundary => "trusted_boundary",
            GuardReason::PointerEventsNone => "pointer_events_none",
            GuardReason::OutsideViewport => "outside_viewport",
            GuardReason::NoHitTarget => "no_hit_target",
            GuardReason::Obscured => "obscured",
            GuardReason::Detached => "detached",
            GuardReason::DocumentChanged => "document_changed",
            GuardReason::UnsupportedControl => "unsupported_control",
            GuardReason::ReadOnly => "read_only",
            GuardReason::StaleValue => "stale_value",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        // `unsafe_page` was a catch-all in older receipts. Map it to the
        // concrete unsupported-surface reason for one compatibility release.
        if code == "unsafe_page" {
            return Some(GuardReason::UnsupportedRendererSurface);
        }
        Self::ALL.into_iter().find(|reason| reason.code() == code)
    }

    /// Canonical classification under the default (execution-first) context.
    /// Rust is authoritative; the JS table is advisory.
    pub fn retry_class(self) -> RetryClass {
        self.retry_class_with(&GuardContext::execution_first())
    }

    /// Classification for a specific authorization context.
    pub fn retry_class_with(self, context: &GuardContext) -> RetryClass {
        match context.policy {
            ExecutionPolicy::Strict => self.strict_retry_class(context),
            ExecutionPolicy::ExecutionFirst => self.execution_first_retry_class(context),
        }
    }

    fn execution_first_retry_class(self, context: &GuardContext) -> RetryClass {
        match self {
            // The route exists but is not the one already tried. The runtime
            // switches backend/frame/dialog/editor routes itself.
            GuardReason::FrameLocalRouteRequired
            | GuardReason::CrossOriginFrameUnresolved
            | GuardReason::UnsupportedRendererSurface
            | GuardReason::NativeDialogRouteRequired
            | GuardReason::UnsupportedControl
            | GuardReason::ReadOnly
            | GuardReason::TrustedBoundary
            | GuardReason::PointerEventsNone
            | GuardReason::OutsideViewport
            | GuardReason::NoHitTarget
            | GuardReason::Obscured
            | GuardReason::StaleReference
            | GuardReason::ChangedTarget
            | GuardReason::Detached
            | GuardReason::StaleValue => RetryClass::SafeInternal,
            // A disabled control often becomes enabled after an async settle;
            // one bounded wait is internal, and a still-disabled control is
            // then reported as an application state, not a capability limit.
            GuardReason::Disabled | GuardReason::AriaDisabled => RetryClass::SafeInternal,
            GuardReason::Hidden | GuardReason::DocumentChanged => RetryClass::RefreshSnapshot,
            // Genuine boundaries: a person must authorize or change state.
            GuardReason::MaskedBoundary if !context.secret_authorized => RetryClass::RequiresUser,
            GuardReason::MaskedBoundary => RetryClass::SafeInternal,
            GuardReason::ProtectedBrowserSurface => RetryClass::DoNotRetry,
        }
    }

    fn strict_retry_class(self, _context: &GuardContext) -> RetryClass {
        match self {
            GuardReason::OutsideViewport | GuardReason::Obscured => RetryClass::SafeInternal,
            GuardReason::StaleReference
            | GuardReason::ChangedTarget
            | GuardReason::Detached
            | GuardReason::DocumentChanged
            | GuardReason::Hidden
            | GuardReason::NoHitTarget
            | GuardReason::PointerEventsNone
            | GuardReason::Disabled
            | GuardReason::AriaDisabled
            | GuardReason::StaleValue => RetryClass::RefreshSnapshot,
            GuardReason::MaskedBoundary | GuardReason::TrustedBoundary | GuardReason::ReadOnly => {
                RetryClass::RequiresUser
            }
            GuardReason::UnsupportedControl
            | GuardReason::UnsupportedRendererSurface
            | GuardReason::ProtectedBrowserSurface
            | GuardReason::NativeDialogRouteRequired => RetryClass::DoNotRetry,
            GuardReason::FrameLocalRouteRequired | GuardReason::CrossOriginFrameUnresolved => {
                RetryClass::SafeInternal
            }
        }
    }

    /// Reasons Controlla must never bypass or "repair" around.
    pub fn is_boundary(self) -> bool {
        self.is_boundary_with(&GuardContext::execution_first())
    }

    /// Boundary test for a specific authorization context. A masked boundary
    /// stops being a boundary once the caller authorized that secret entry.
    pub fn is_boundary_with(self, context: &GuardContext) -> bool {
        match self {
            GuardReason::ProtectedBrowserSurface => true,
            GuardReason::MaskedBoundary => !context.secret_authorized,
            _ => false,
        }
    }
}

impl RetryClass {
    pub fn code(self) -> &'static str {
        match self {
            RetryClass::SafeInternal => "safe_internal",
            RetryClass::RefreshSnapshot => "refresh_snapshot",
            RetryClass::RequiresUser => "requires_user",
            RetryClass::DoNotRetry => "do_not_retry",
        }
    }

    /// Whether the model must do something before this action can proceed.
    /// A fresh snapshot is still a caller-visible step.
    pub fn needs_caller_action(self) -> bool {
        !matches!(self, RetryClass::SafeInternal)
    }
}

/// Maximum serialized size of the `detail` object returned to callers.
const MAX_DETAIL_BYTES: usize = 1024;

/// Build the structured, compact `not_dispatched` result for a refused guard.
///
/// `guard_result` is the object returned by the page guard
/// (`{ok:false, reason, retry_class?, detail?}`). Unknown reason codes are
/// reported verbatim under `reason_code = "unclassified"` (never `blocked`) and
/// fail closed (`do_not_retry`).
pub fn refusal(guard_result: &Value) -> Value {
    refusal_with(guard_result, &GuardContext::execution_first())
}

/// Build a refusal for a specific authorization context.
pub fn refusal_with(guard_result: &Value, context: &GuardContext) -> Value {
    let raw = guard_result["reason"].as_str().unwrap_or("unclassified");
    let (code, class) = match GuardReason::from_code(raw) {
        Some(reason) => (reason.code(), reason.retry_class_with(context)),
        None => ("unclassified", RetryClass::DoNotRetry),
    };
    let mut detail = guard_result.get("detail").cloned().unwrap_or(Value::Null);
    if detail.to_string().len() > MAX_DETAIL_BYTES {
        detail = json!({"truncated": true});
    }
    let mut out = json!({
        "status": "not_dispatched",
        "dispatch_acknowledged": false,
        // A refused pre-dispatch guard never reached the page: the side effect
        // is known not to have happened.
        "dispatch_may_have_occurred": false,
        "reason": code,
        "reason_code": code,
        "retry_class": class.code(),
        "policy": context.policy.code(),
        "model_action_required": class.needs_caller_action(),
        "detail": detail,
    });
    if code == "obscured"
        && let Some(blocker) = detail.get("blocker")
    {
        out["blocker"] = blocker.clone();
    }
    if let Some(reason) = GuardReason::from_code(raw) {
        crate::action_recovery::attach_to_refusal_with(&mut out, reason, context);
    }
    out
}

const SHARED_CLICK_GUARD_JS: &str = include_str!("shared_page/guard.js");

/// The shared click guard with the unsafe-control predicate substituted in.
pub fn click_guard_script(unsafe_predicate: &str) -> String {
    if unrestricted_mode() {
        return "function(index){const e=this.nodes[index];if(!e)return {ok:false,reason:'stale_reference'};e.scrollIntoView({block:'nearest',inline:'nearest'});const b=e.getBoundingClientRect();return {ok:true,x:b.left+b.width/2,y:b.top+b.height/2};}".to_owned();
    }
    SHARED_CLICK_GUARD_JS.replace("__UNSAFE__", unsafe_predicate)
}

pub fn unrestricted_mode() -> bool {
    // Local fixture mode bypasses page interaction guards only; native
    // transport, identity, lease, and postcondition checks remain enforced.
    std::env::var_os("CONTROLLA_UNRESTRICTED").is_some_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_never_use_blocked() {
        for reason in GuardReason::ALL {
            assert_eq!(GuardReason::from_code(reason.code()), Some(reason));
            assert_ne!(reason.code(), "blocked");
        }
        assert_eq!(GuardReason::from_code("blocked"), None);
    }

    #[test]
    fn backend_and_route_mismatch_resolve_inside_the_runtime() {
        // Every reason whose only problem is "this route cannot dispatch here"
        // must be repairable without a model turn.
        for reason in [
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
        ] {
            assert_eq!(
                reason.retry_class(),
                RetryClass::SafeInternal,
                "{reason:?} must resolve internally under execution_first"
            );
            assert!(
                !reason.is_boundary(),
                "{reason:?} must not be reported as a boundary"
            );
        }
    }

    #[test]
    fn genuine_boundaries_stay_terminal() {
        let strict = GuardContext::strict();
        // A Chrome-owned surface is terminal in both policies.
        assert_eq!(
            GuardReason::ProtectedBrowserSurface.retry_class(),
            RetryClass::DoNotRetry
        );
        assert!(GuardReason::ProtectedBrowserSurface.is_boundary());
        // Secret entry without authorization is a person's decision.
        let masked = GuardReason::MaskedBoundary;
        assert_eq!(masked.retry_class(), RetryClass::RequiresUser);
        assert!(masked.is_boundary());
        assert_eq!(
            masked
                .retry_class_with(&GuardContext::execution_first().with_secret_authorization(true)),
            RetryClass::SafeInternal
        );
        assert!(
            !masked
                .is_boundary_with(&GuardContext::execution_first().with_secret_authorization(true))
        );
        // The strict policy restores the conservative classification.
        assert_eq!(
            GuardReason::TrustedBoundary.retry_class_with(&strict),
            RetryClass::RequiresUser
        );
        assert_eq!(
            GuardReason::UnsupportedRendererSurface.retry_class_with(&strict),
            RetryClass::DoNotRetry
        );
    }

    #[test]
    fn every_boundary_requires_a_caller_action() {
        for reason in GuardReason::ALL {
            if reason.is_boundary() {
                assert!(
                    reason.retry_class().needs_caller_action(),
                    "{reason:?} is a boundary but claims no caller action"
                );
            }
        }
    }

    #[test]
    fn child_frame_controls_are_not_blanket_unsafe_pages() {
        let script = click_guard_script("false");
        assert!(
            !script.contains("frame && e.tagName !== 'A'"),
            "non-link child-frame controls must reach the frame-local route"
        );
        assert!(
            script.contains("const frame = e.ownerDocument === document"),
            "frame ownership remains checked"
        );
        assert!(
            script.contains("unsupported_renderer_surface"),
            "the actual unsafe-control predicate stays enforced with a precise reason"
        );
        assert_eq!(
            GuardReason::UnsupportedControl.retry_class(),
            RetryClass::SafeInternal
        );
        assert_eq!(
            GuardReason::ReadOnly.retry_class(),
            RetryClass::SafeInternal
        );
    }

    #[test]
    fn refusal_is_specific_not_dispatched_and_bounded() {
        let value =
            refusal(&json!({"ok":false,"reason":"obscured","detail":{"blocker":{"tag":"div"}}}));
        assert_eq!(value["status"], "not_dispatched");
        assert_eq!(value["reason_code"], "obscured");
        assert_eq!(value["retry_class"], "safe_internal");
        assert_eq!(value["model_action_required"], false);
        assert_eq!(value["policy"], "execution_first");
        assert_eq!(value["blocker"]["tag"], "div");
        let unknown = refusal(&json!({"ok":false,"reason":"blocked"}));
        assert_eq!(unknown["reason_code"], "unclassified");
        assert_eq!(unknown["retry_class"], "do_not_retry");
        assert_eq!(unknown["model_action_required"], true);
        let huge = refusal(&json!({"ok":false,"reason":"hidden","detail":{"x":"y".repeat(4000)}}));
        assert_eq!(huge["detail"]["truncated"], true);
    }

    #[test]
    fn strict_policy_receipt_reports_the_policy_it_used() {
        let value = refusal_with(
            &json!({"ok":false,"reason":"trusted_boundary"}),
            &GuardContext::strict(),
        );
        assert_eq!(value["policy"], "strict");
        assert_eq!(value["retry_class"], "requires_user");
        assert_eq!(value["model_action_required"], true);
    }
}
