//! Bounded native desktop fallback contract. Platform dispatch requires a live
//! adapter plus current ownership, permission, and target identity checks.

use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin};

pub const MAX_DESKTOP_ENVELOPE_BYTES: usize = 16 * 1024;
pub const MAX_DESKTOP_COORDINATE: u32 = 16_384;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesktopCapability {
    DesktopPointer,
    DesktopKeyboard,
    DesktopWindow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesktopMode {
    Foreground,
    OwnedAutomationWindow,
    Background,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DesktopVisualIdentity {
    pub session_id: String,
    pub tab_id: String,
    pub document_id: String,
    pub visual_revision: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesktopAction {
    Move {
        x: u32,
        y: u32,
    },
    Scroll {
        x: u32,
        y: u32,
        delta_x: i32,
        delta_y: i32,
    },
    Key {
        key: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DesktopActionEnvelope {
    pub version: u16,
    pub request_id: String,
    pub lease_id: String,
    pub capability: DesktopCapability,
    pub mode: DesktopMode,
    pub visual_identity: DesktopVisualIdentity,
    /// Exact URL of the tab observed immediately before native dispatch.
    pub expected_url: String,
    /// OS process ID reported by the native messaging host that paired this
    /// exact Chrome instance.
    pub expected_process_id: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub action: DesktopAction,
}

pub fn decode_desktop_envelope(
    bytes: &[u8],
) -> Result<DesktopActionEnvelope, DesktopValidationError> {
    if bytes.len() > MAX_DESKTOP_ENVELOPE_BYTES {
        return Err(DesktopValidationError {
            reason_code: "envelope_too_large",
            next_action: "reduce the native action payload",
        });
    }
    serde_json::from_slice(bytes).map_err(|_| DesktopValidationError {
        reason_code: "invalid_envelope",
        next_action: "correct the bounded native action envelope before retrying",
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesktopActionStatus {
    Completed,
    NotDispatched,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DesktopActionReceipt {
    pub status: DesktopActionStatus,
    pub capability: DesktopCapability,
    pub dispatch_acknowledged: bool,
    pub dispatch_may_have_occurred: bool,
    pub reason_code: Option<String>,
    pub retry_class: Option<String>,
    pub next_action: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopValidationError {
    pub reason_code: &'static str,
    pub next_action: &'static str,
}

pub fn validate_desktop_envelope(
    envelope: &DesktopActionEnvelope,
    owned_session: bool,
    expected_identity: &DesktopVisualIdentity,
    current_width: u32,
    current_height: u32,
) -> Result<(), DesktopValidationError> {
    let fail = |reason_code, next_action| {
        Err(DesktopValidationError {
            reason_code,
            next_action,
        })
    };
    if !owned_session {
        return fail(
            "unowned_session",
            "use a Controlla-owned session and refresh its visual evidence",
        );
    }
    if envelope.version != 1
        || envelope.request_id.is_empty()
        || envelope.request_id.len() > 128
        || envelope.lease_id.is_empty()
        || envelope.lease_id.len() > 128
    {
        return fail(
            "invalid_envelope",
            "correct the bounded native action envelope before retrying",
        );
    }
    let encoded = serde_json::to_vec(envelope).map_err(|_| DesktopValidationError {
        reason_code: "invalid_envelope",
        next_action: "correct the bounded native action envelope before retrying",
    })?;
    if encoded.len() > MAX_DESKTOP_ENVELOPE_BYTES {
        return fail("envelope_too_large", "reduce the native action payload");
    }
    if &envelope.visual_identity != expected_identity {
        return fail(
            "stale_visual_reference",
            "capture a fresh visual reference for this tab and document",
        );
    }
    if envelope.mode == DesktopMode::Background {
        return fail(
            "activation_forbidden",
            "use semantic page control or an owned foreground automation window",
        );
    }
    if envelope.viewport_width == 0
        || envelope.viewport_height == 0
        || envelope.viewport_width > MAX_DESKTOP_COORDINATE
        || envelope.viewport_height > MAX_DESKTOP_COORDINATE
        || envelope.viewport_width != current_width
        || envelope.viewport_height != current_height
    {
        return fail(
            "stale_visual_reference",
            "refresh visual evidence and current viewport geometry",
        );
    }
    let in_bounds = |x: u32, y: u32| x < current_width && y < current_height;
    let valid = match &envelope.action {
        DesktopAction::Move { x, y } => in_bounds(*x, *y),
        DesktopAction::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => {
            in_bounds(*x, *y)
                && (-1200..=1200).contains(delta_x)
                && (-1200..=1200).contains(delta_y)
                && (*delta_x != 0 || *delta_y != 0)
        }
        DesktopAction::Key { key } => [
            "Escape",
            "Tab",
            "Shift+Tab",
            "Control+L",
            "Meta+L",
            "ArrowUp",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
        ]
        .contains(&key.as_str()),
    };
    if !valid {
        return fail(
            "invalid_action_bounds",
            "refresh the visual reference or use a supported bounded action",
        );
    }
    let required_capability = match &envelope.action {
        DesktopAction::Move { .. } | DesktopAction::Scroll { .. } => {
            DesktopCapability::DesktopPointer
        }
        DesktopAction::Key { .. } => DesktopCapability::DesktopKeyboard,
    };
    if envelope.capability != required_capability {
        return fail(
            "capability_mismatch",
            "request the capability matching the bounded action",
        );
    }
    Ok(())
}

pub trait DesktopFallbackProvider: Send + Sync {
    fn capability_available(&self, capability: DesktopCapability) -> bool;
    fn dispatch<'a>(
        &'a self,
        envelope: &'a DesktopActionEnvelope,
    ) -> Pin<Box<dyn Future<Output = DesktopActionReceipt> + Send + 'a>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableDesktopProvider;

impl DesktopFallbackProvider for UnavailableDesktopProvider {
    fn capability_available(&self, _capability: DesktopCapability) -> bool {
        false
    }
    fn dispatch<'a>(
        &'a self,
        envelope: &'a DesktopActionEnvelope,
    ) -> Pin<Box<dyn Future<Output = DesktopActionReceipt> + Send + 'a>> {
        Box::pin(async move {
            DesktopActionReceipt {
            status: DesktopActionStatus::NotDispatched,
            capability: envelope.capability,
            dispatch_acknowledged: false,
            dispatch_may_have_occurred: false,
            reason_code: Some("capability_unavailable".into()),
            retry_class: Some("capability_unavailable".into()),
            next_action: Some("use semantic or page-tool control, or an explicit user-visible Chrome interaction".into()),
        }
        })
    }
}

pub fn uncertain_delivery_receipt(capability: DesktopCapability) -> DesktopActionReceipt {
    DesktopActionReceipt { status: DesktopActionStatus::Unknown, capability, dispatch_acknowledged: false,
        dispatch_may_have_occurred: true, reason_code: Some("desktop_delivery_uncertain".into()),
        retry_class: Some("read_only_reconciliation_required".into()),
        next_action: Some("inspect the same target once before considering any new action; never replay this delivery".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> DesktopVisualIdentity {
        DesktopVisualIdentity {
            session_id: "s".into(),
            tab_id: "7".into(),
            document_id: "doc".into(),
            visual_revision: "sha256:1".into(),
        }
    }
    fn envelope(action: DesktopAction) -> DesktopActionEnvelope {
        DesktopActionEnvelope {
            version: 1,
            request_id: "r".into(),
            lease_id: "l".into(),
            capability: match action {
                DesktopAction::Key { .. } => DesktopCapability::DesktopKeyboard,
                _ => DesktopCapability::DesktopPointer,
            },
            mode: DesktopMode::Foreground,
            visual_identity: identity(),
            expected_url: "https://example.com/".into(),
            expected_process_id: 1234,
            viewport_width: 800,
            viewport_height: 600,
            action,
        }
    }
    #[test]
    fn desktop_action_requires_owned_session_and_fresh_visual_identity() {
        let valid = envelope(DesktopAction::Move { x: 4, y: 5 });
        assert_eq!(
            validate_desktop_envelope(&valid, false, &identity(), 800, 600)
                .unwrap_err()
                .reason_code,
            "unowned_session"
        );
        let mut stale = valid.clone();
        stale.visual_identity.document_id = "new-doc".into();
        assert_eq!(
            validate_desktop_envelope(&stale, true, &identity(), 800, 600)
                .unwrap_err()
                .reason_code,
            "stale_visual_reference"
        );
    }
    #[test]
    fn desktop_action_rejects_user_owned_background_activation() {
        let mut action = envelope(DesktopAction::Move { x: 4, y: 5 });
        action.mode = DesktopMode::Background;
        assert_eq!(
            validate_desktop_envelope(&action, true, &identity(), 800, 600)
                .unwrap_err()
                .reason_code,
            "activation_forbidden"
        );
    }
    #[test]
    fn desktop_action_rejects_out_of_bounds_coordinates_and_payloads() {
        let action = envelope(DesktopAction::Move { x: 800, y: 5 });
        assert_eq!(
            validate_desktop_envelope(&action, true, &identity(), 800, 600)
                .unwrap_err()
                .reason_code,
            "invalid_action_bounds"
        );
    }
    #[test]
    fn native_schema_rejects_coordinate_click_and_drag_actions() {
        for action in [
            serde_json::json!({"kind":"click","x":10,"y":20,"target_class":"browser_viewport"}),
            serde_json::json!({"kind":"drag","start_x":10,"start_y":20,"end_x":40,"end_y":60,"target_class":"browser_viewport"}),
        ] {
            let mut value =
                serde_json::to_value(envelope(DesktopAction::Move { x: 10, y: 20 })).unwrap();
            value["action"] = action;
            assert!(serde_json::from_value::<DesktopActionEnvelope>(value).is_err());
        }
    }

    #[test]
    fn native_schema_rejects_all_text_entry() {
        let mut value =
            serde_json::to_value(envelope(DesktopAction::Key { key: "Tab".into() })).unwrap();
        value["action"] = serde_json::json!({"kind":"type","text":"opaque-secret-7f4a9e1c","target_class":"browser_viewport"});
        assert!(serde_json::from_value::<DesktopActionEnvelope>(value).is_err());
    }

    #[test]
    fn safe_native_pointer_motion_and_scrolling_are_supported() {
        for action in [
            envelope(DesktopAction::Move { x: 10, y: 20 }),
            envelope(DesktopAction::Scroll {
                x: 10,
                y: 20,
                delta_x: 0,
                delta_y: 120,
            }),
        ] {
            validate_desktop_envelope(&action, true, &identity(), 800, 600).unwrap();
        }
    }

    #[test]
    fn desktop_envelope_decoder_rejects_malformed_and_oversized_data() {
        assert_eq!(
            decode_desktop_envelope(b"{").unwrap_err().reason_code,
            "invalid_envelope"
        );
        assert_eq!(
            decode_desktop_envelope(&vec![b' '; MAX_DESKTOP_ENVELOPE_BYTES + 1])
                .unwrap_err()
                .reason_code,
            "envelope_too_large"
        );
    }
    #[test]
    fn uncertain_delivery_is_unknown_and_never_replayable() {
        let receipt = uncertain_delivery_receipt(DesktopCapability::DesktopPointer);
        assert_eq!(receipt.status, DesktopActionStatus::Unknown);
        assert!(receipt.dispatch_may_have_occurred);
        assert_eq!(
            receipt.retry_class.as_deref(),
            Some("read_only_reconciliation_required")
        );
    }
}
