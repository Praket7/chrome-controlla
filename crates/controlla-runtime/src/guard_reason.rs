//! Typed pre-dispatch guard diagnostics.
//!
//! The page guard (`shared_page/guard.js`) reports exactly which predicate
//! failed. This module is the single Rust-side contract for those codes, their
//! retry classification, and the structured refusal that MCP routes return.
//! A generic `blocked` result is never produced.

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

    /// Canonical classification. Rust is authoritative; the JS table is advisory.
    pub fn retry_class(self) -> RetryClass {
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
        matches!(
            self,
            GuardReason::UnsupportedRendererSurface
                | GuardReason::ProtectedBrowserSurface
                | GuardReason::NativeDialogRouteRequired
                | GuardReason::MaskedBoundary
                | GuardReason::TrustedBoundary
                | GuardReason::Hidden
                | GuardReason::Disabled
                | GuardReason::AriaDisabled
        )
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
    let raw = guard_result["reason"].as_str().unwrap_or("unclassified");
    let (code, class) = match GuardReason::from_code(raw) {
        Some(reason) => (reason.code(), reason.retry_class()),
        None => ("unclassified", RetryClass::DoNotRetry),
    };
    let mut detail = guard_result.get("detail").cloned().unwrap_or(Value::Null);
    if detail.to_string().len() > MAX_DETAIL_BYTES {
        detail = json!({"truncated": true});
    }
    let mut out = json!({
        "status": "not_dispatched",
        "dispatch_acknowledged": false,
        "reason": code,
        "reason_code": code,
        "retry_class": class.code(),
        "detail": detail,
    });
    if code == "obscured"
        && let Some(blocker) = detail.get("blocker")
    {
        out["blocker"] = blocker.clone();
    }
    if let Some(reason) = GuardReason::from_code(raw) {
        crate::action_recovery::attach_to_refusal(&mut out, reason);
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
    fn only_repairable_guards_are_safe_internal_and_boundaries_never_are() {
        for reason in GuardReason::ALL {
            let safe = reason.retry_class() == RetryClass::SafeInternal;
            assert_eq!(
                safe,
                matches!(
                    reason,
                    GuardReason::OutsideViewport
                        | GuardReason::Obscured
                        | GuardReason::FrameLocalRouteRequired
                        | GuardReason::CrossOriginFrameUnresolved
                ),
                "{reason:?}"
            );
            if reason.is_boundary() {
                assert_ne!(reason.retry_class(), RetryClass::SafeInternal);
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
            RetryClass::DoNotRetry
        );
        assert_eq!(
            GuardReason::ReadOnly.retry_class(),
            RetryClass::RequiresUser
        );
    }

    #[test]
    fn refusal_is_specific_not_dispatched_and_bounded() {
        let value =
            refusal(&json!({"ok":false,"reason":"obscured","detail":{"blocker":{"tag":"div"}}}));
        assert_eq!(value["status"], "not_dispatched");
        assert_eq!(value["reason_code"], "obscured");
        assert_eq!(value["retry_class"], "safe_internal");
        assert_eq!(value["blocker"]["tag"], "div");
        let unknown = refusal(&json!({"ok":false,"reason":"blocked"}));
        assert_eq!(unknown["reason_code"], "unclassified");
        assert_eq!(unknown["retry_class"], "do_not_retry");
        let huge = refusal(&json!({"ok":false,"reason":"hidden","detail":{"x":"y".repeat(4000)}}));
        assert_eq!(huge["detail"]["truncated"], true);
    }
}
