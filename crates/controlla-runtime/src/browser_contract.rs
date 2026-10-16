//! Small executable contracts shared by MCP validation and regression tests.
//! Tool schemas remain generated from typed arguments; these rows constrain
//! the cross-field rules that JSON Schema cannot express.

use serde::{Deserialize, Serialize};

/// Browser presentation evidence. This describes independent Chrome and page
/// signals; `document_has_focus` is intentionally not a substitute for window
/// or tab focus, and lifecycle `unknown` never implies a live renderer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PresentationState {
    pub window_focused: bool,
    pub tab_active: bool,
    #[serde(rename = "visibility_state")]
    pub visibility: PageVisibility,
    pub lifecycle: TargetLifecycle,
    pub viewport: Option<PresentationViewport>,
    pub activation_epoch: u64,
    pub focus_epoch: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageVisibility {
    Visible,
    Hidden,
    Prerender,
    Unloaded,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetLifecycle {
    Active,
    Loading,
    Frozen,
    Discarded,
    Crashed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationViewport {
    pub width: f64,
    pub height: f64,
    pub device_scale_factor: f64,
}

/// Per-operation policy, supplied independently of the target's current
/// presentation evidence. Background policies never grant permission to
/// focus or activate a target as a side effect.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub forbid_focus: bool,
    pub forbid_activation: bool,
    pub allow_scroll: bool,
    pub allow_owned_window: bool,
    pub deadline_ms: u64,
}

impl ExecutionPolicy {
    pub const fn strict_background(allow_scroll: bool, deadline_ms: u64) -> Self {
        Self {
            forbid_focus: true,
            forbid_activation: true,
            allow_scroll,
            allow_owned_window: false,
            deadline_ms,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueRequirement {
    Forbidden,
    Optional,
    Required,
}

pub const BROWSER_ACTION_CONTRACT_VERSION: u16 = 1;

pub const SHARED_EXTENSION_AUTHORIZED_METHODS: &[&str] = &[
    "Page.getFrameTree",
    "Page.getLayoutMetrics",
    "Page.captureScreenshot",
    "Runtime.evaluate",
    "Runtime.callFunctionOn",
    "Runtime.releaseObject",
    "Input.dispatchMouseEvent",
    "Input.dispatchDragEvent",
    "Input.dispatchTouchEvent",
    "Input.dispatchKeyEvent",
    "Input.imeSetComposition",
    "Input.insertText",
    "DOM.getDocument",
    "DOM.querySelector",
    "DOM.enable",
    "DOM.describeNode",
    "DOM.resolveNode",
    "DOM.setFileInputFiles",
    "Page.enable",
    "Page.setInterceptFileChooserDialog",
    "Accessibility.getPartialAXTree",
    "Accessibility.getFullAXTree",
    "Controlla.waitForFileChooser",
    "Controlla.downloadLifecycle",
    "Controlla.scrollToTarget",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityKey {
    pub provider: &'static str,
    pub mode: &'static str,
    pub action: &'static str,
    pub version: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserActionCapability {
    pub key: CapabilityKey,
    pub value: ValueRequirement,
    pub allow_password: bool,
    pub effect: &'static str,
    pub evidence: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityResolution {
    Supported(&'static BrowserActionCapability),
    Unsupported { reason: &'static str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserOperationCapability {
    pub provider: &'static str,
    pub mode: &'static str,
    pub action: &'static str,
    pub effect: &'static str,
    pub evidence: &'static str,
    pub dispatch: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationResolution {
    Supported(&'static BrowserOperationCapability),
    Unsupported { reason: &'static str },
}

// Single public browser-operation inventory used by routing and parity tests.
// `dispatch` names the implementation boundary expected to own the operation.
pub const BROWSER_OPERATIONS: &[BrowserOperationCapability] = &[
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "discover_shared_tabs",
        effect: "observation",
        evidence: "fresh_inventory",
        dispatch: "discover_shared_tabs",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "discover_shared_tabs",
        effect: "observation",
        evidence: "fresh_inventory",
        dispatch: "discover_shared_tabs",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "pair_shared",
        effect: "session_attach",
        evidence: "paired_target_identity",
        dispatch: "pair_shared",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "pair_shared",
        effect: "session_attach",
        evidence: "paired_target_identity",
        dispatch: "pair_shared",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "accept_shared",
        effect: "session_attach",
        evidence: "paired_target_identity",
        dispatch: "accept_shared",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "accept_shared",
        effect: "session_attach",
        evidence: "paired_target_identity",
        dispatch: "accept_shared",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "list",
        effect: "observation",
        evidence: "target_inventory",
        dispatch: "list",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "list",
        effect: "observation",
        evidence: "target_inventory",
        dispatch: "list",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "list",
        effect: "observation",
        evidence: "target_inventory",
        dispatch: "list",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "open",
        effect: "navigation",
        evidence: "event_and_document_readiness",
        dispatch: "open",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "open",
        effect: "navigation",
        evidence: "event_and_document_readiness",
        dispatch: "open",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "adopt_popup",
        effect: "session_attach",
        evidence: "unique_opener_creation_url_and_visible_account_marker",
        dispatch: "adopt_popup",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "adopt_popup",
        effect: "session_attach",
        evidence: "unique_opener_creation_url_and_visible_account_marker",
        dispatch: "adopt_popup",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "transition_presentation",
        effect: "presentation_policy_transition",
        evidence: "same_target_before_and_after_presentation_state",
        dispatch: "transition_presentation",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "transition_presentation",
        effect: "presentation_policy_transition",
        evidence: "same_target_before_and_after_presentation_state",
        dispatch: "transition_presentation",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "close_owned",
        effect: "close_owned_target",
        evidence: "ownership_and_absence",
        dispatch: "close_owned",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "close_owned",
        effect: "close_owned_target",
        evidence: "ownership_and_absence",
        dispatch: "close_owned",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "move_owned",
        effect: "move_owned_target",
        evidence: "ownership_and_topology",
        dispatch: "move_owned",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "activate",
        effect: "activate_owned_target",
        evidence: "ownership_and_focus_topology",
        dispatch: "activate_owned",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "release",
        effect: "session_release",
        evidence: "lease_released",
        dispatch: "release",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "release",
        effect: "session_release",
        evidence: "lease_released",
        dispatch: "release",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "launch",
        effect: "private_session_create",
        evidence: "session_and_target_identity",
        dispatch: "launch",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "release",
        effect: "session_release",
        evidence: "lease_released",
        dispatch: "release",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "artifact_register",
        effect: "artifact_stage",
        evidence: "content_hash_and_handle",
        dispatch: "artifact_register",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "artifact_read",
        effect: "artifact_read",
        evidence: "content_hash_and_size",
        dispatch: "artifact_read",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "artifact_register",
        effect: "artifact_stage",
        evidence: "content_hash_and_handle",
        dispatch: "artifact_register",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "artifact_register",
        effect: "artifact_stage",
        evidence: "content_hash_and_handle",
        dispatch: "artifact_register",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "artifact_read",
        effect: "artifact_read",
        evidence: "content_hash_and_size",
        dispatch: "artifact_read",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "artifact_read",
        effect: "artifact_read",
        evidence: "content_hash_and_size",
        dispatch: "artifact_read",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "task",
        effect: "task_execution",
        evidence: "task_receipts",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "task",
        effect: "task_execution",
        evidence: "task_receipts",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "task",
        effect: "task_execution",
        evidence: "task_receipts",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "snapshot",
        effect: "observation",
        evidence: "snapshot_and_optional_png",
        dispatch: "snapshot",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "snapshot",
        effect: "observation",
        evidence: "snapshot_and_optional_png",
        dispatch: "snapshot",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "snapshot",
        effect: "observation",
        evidence: "snapshot_and_optional_png",
        dispatch: "snapshot",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "snapshot:visual_probe",
        effect: "visual_observation",
        evidence: "png_payload_integrity_and_visual_interpretation",
        dispatch: "visual_probe",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "snapshot:visual_probe",
        effect: "visual_observation",
        evidence: "png_payload_integrity_and_visual_interpretation",
        dispatch: "visual_probe",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "snapshot:visual_probe",
        effect: "visual_observation",
        evidence: "png_payload_integrity_and_visual_interpretation",
        dispatch: "visual_probe",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "task:upload",
        effect: "file_upload",
        evidence: "application_acceptance",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "task:upload",
        effect: "file_upload",
        evidence: "application_acceptance",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "task:upload",
        effect: "file_upload",
        evidence: "application_acceptance",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "task:visual_probe",
        effect: "visual_observation",
        evidence: "png_payload_integrity_and_visual_interpretation",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "task:visual_probe",
        effect: "visual_observation",
        evidence: "png_payload_integrity_and_visual_interpretation",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "task:visual_probe",
        effect: "visual_observation",
        evidence: "png_payload_integrity_and_visual_interpretation",
        dispatch: "task",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "foreground",
        action: "skill",
        effect: "skill_management",
        evidence: "qualification_receipt",
        dispatch: "skill",
    },
    BrowserOperationCapability {
        provider: "shared_extension",
        mode: "background",
        action: "skill",
        effect: "skill_management",
        evidence: "qualification_receipt",
        dispatch: "skill",
    },
    BrowserOperationCapability {
        provider: "dedicated_headless",
        mode: "headless",
        action: "skill",
        effect: "skill_management",
        evidence: "qualification_receipt",
        dispatch: "skill",
    },
];

pub fn resolve_browser_operation(provider: &str, mode: &str, action: &str) -> OperationResolution {
    BROWSER_OPERATIONS
        .iter()
        .find(|cap| cap.provider == provider && cap.mode == mode && cap.action == action)
        .map(OperationResolution::Supported)
        .unwrap_or(OperationResolution::Unsupported {
            reason: "operation is not declared for this provider and mode",
        })
}

pub fn browser_action_inventory() -> serde_json::Value {
    let actions = ["foreground", "background", "headless"].map(|mode| {
        let provider = if mode == "headless" {
            "dedicated_headless"
        } else {
            "shared_extension"
        };
        let mut names = BROWSER_OPERATIONS
            .iter()
            .filter(|cap| cap.mode == mode && cap.provider == provider)
            .map(|cap| cap.action.split(':').next().unwrap_or(cap.action))
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        (mode, serde_json::json!(names))
    });
    serde_json::json!({"foreground":actions[0].1,"background":actions[1].1,"headless":actions[2].1})
}

// Public act routes are enumerated here before target acquisition. Keep the
// provider/mode split explicit so a schema-valid action cannot reach an
// implementation that has never qualified that transport combination.
const fn capability(
    provider: &'static str,
    mode: &'static str,
    action: &'static str,
    value: ValueRequirement,
    allow_password: bool,
    effect: &'static str,
    evidence: &'static str,
) -> BrowserActionCapability {
    BrowserActionCapability {
        key: CapabilityKey {
            provider,
            mode,
            action,
            version: BROWSER_ACTION_CONTRACT_VERSION,
        },
        value,
        allow_password,
        effect,
        evidence,
    }
}

const SHARED: &[BrowserActionCapability] = &[
    capability(
        "shared_extension",
        "foreground",
        "click",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "double_click",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "context_click",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "hover",
        ValueRequirement::Forbidden,
        false,
        "input",
        "exact_css_hover_state",
    ),
    capability(
        "shared_extension",
        "foreground",
        "scroll",
        ValueRequirement::Forbidden,
        false,
        "input",
        "same_container_scroll_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "scroll_to",
        ValueRequirement::Forbidden,
        false,
        "input",
        "target_visible_center_hit",
    ),
    capability(
        "shared_extension",
        "foreground",
        "fill",
        ValueRequirement::Required,
        true,
        "input",
        "value_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "secure_fill",
        ValueRequirement::Optional,
        true,
        "credential_input",
        "redacted_value_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "type",
        ValueRequirement::Required,
        false,
        "input",
        "value_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "press",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "select",
        ValueRequirement::Required,
        false,
        "input",
        "native_select_exact_value_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "slider",
        ValueRequirement::Required,
        false,
        "input",
        "native_range_min_max_step_value_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "touch",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "swipe",
        ValueRequirement::Forbidden,
        false,
        "input",
        "guarded_touch_movement_readback",
    ),
    capability(
        "shared_extension",
        "foreground",
        "multi_touch",
        ValueRequirement::Forbidden,
        false,
        "input",
        "guarded_trusted_two_finger_pinch",
    ),
    capability(
        "shared_extension",
        "foreground",
        "page_tool",
        ValueRequirement::Forbidden,
        false,
        "page_tool",
        "page_tool_receipt",
    ),
    capability(
        "shared_extension",
        "background",
        "click",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "double_click",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "context_click",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "hover",
        ValueRequirement::Forbidden,
        false,
        "input",
        "exact_css_hover_state",
    ),
    capability(
        "shared_extension",
        "background",
        "scroll",
        ValueRequirement::Forbidden,
        false,
        "input",
        "same_container_scroll_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "scroll_to",
        ValueRequirement::Forbidden,
        false,
        "input",
        "target_visible_center_hit",
    ),
    capability(
        "shared_extension",
        "background",
        "fill",
        ValueRequirement::Required,
        true,
        "input",
        "value_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "type",
        ValueRequirement::Required,
        false,
        "input",
        "value_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "press",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "select",
        ValueRequirement::Required,
        false,
        "input",
        "native_select_exact_value_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "slider",
        ValueRequirement::Required,
        false,
        "input",
        "native_range_min_max_step_value_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "touch",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "swipe",
        ValueRequirement::Forbidden,
        false,
        "input",
        "guarded_touch_movement_readback",
    ),
    capability(
        "shared_extension",
        "background",
        "multi_touch",
        ValueRequirement::Forbidden,
        false,
        "input",
        "guarded_trusted_two_finger_pinch",
    ),
    capability(
        "shared_extension",
        "background",
        "page_tool",
        ValueRequirement::Forbidden,
        false,
        "page_tool",
        "page_tool_receipt",
    ),
];

const HEADLESS: &[BrowserActionCapability] = &[
    capability(
        "dedicated_headless",
        "headless",
        "click",
        ValueRequirement::Forbidden,
        false,
        "input",
        "application_readback",
    ),
    capability(
        "dedicated_headless",
        "headless",
        "fill",
        ValueRequirement::Required,
        true,
        "input",
        "value_readback",
    ),
    capability(
        "dedicated_headless",
        "headless",
        "type",
        ValueRequirement::Required,
        false,
        "input",
        "value_readback",
    ),
    capability(
        "dedicated_headless",
        "headless",
        "select",
        ValueRequirement::Required,
        false,
        "input",
        "native_select_exact_value_readback",
    ),
    capability(
        "dedicated_headless",
        "headless",
        "slider",
        ValueRequirement::Required,
        false,
        "input",
        "native_range_min_max_step_value_readback",
    ),
    capability(
        "dedicated_headless",
        "headless",
        "page_tool",
        ValueRequirement::Forbidden,
        false,
        "page_tool",
        "page_tool_receipt",
    ),
];

pub fn resolve_action_capability(
    provider: &str,
    mode: &str,
    action: &str,
    version: u16,
) -> CapabilityResolution {
    if version != BROWSER_ACTION_CONTRACT_VERSION {
        return CapabilityResolution::Unsupported {
            reason: "unsupported capability contract version",
        };
    }
    let rows = match (provider, mode) {
        ("shared_extension", "foreground" | "background") => SHARED,
        ("dedicated_headless", "headless") => HEADLESS,
        _ => {
            return CapabilityResolution::Unsupported {
                reason: "unsupported provider and mode combination",
            };
        }
    };
    rows.iter()
        .find(|row| row.key.action == action && row.key.mode == mode)
        .map(CapabilityResolution::Supported)
        .unwrap_or(CapabilityResolution::Unsupported {
            reason: "action is unsupported by this provider and mode",
        })
}

pub fn validate_action_value(
    action: &str,
    value: Option<&str>,
    allow_password: Option<bool>,
) -> Result<(), &'static str> {
    let capability = SHARED
        .iter()
        .chain(HEADLESS.iter())
        .find(|contract| contract.key.action == action)
        .ok_or("unsupported action")?;
    validate_capability_value(capability, value, allow_password)
}

pub fn validate_capability_value(
    capability: &BrowserActionCapability,
    value: Option<&str>,
    allow_password: Option<bool>,
) -> Result<(), &'static str> {
    if allow_password == Some(true) && !capability.allow_password {
        return Err("allow_password is not supported for this action");
    }
    match (capability.value, value.is_some()) {
        (ValueRequirement::Forbidden, true) => Err("value is not accepted for this action"),
        (ValueRequirement::Required, false) => Err("value is required for this action"),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_and_execution_contracts_keep_focus_and_activation_separate() {
        let state: PresentationState = serde_json::from_value(serde_json::json!({
            "window_focused":false,"tab_active":true,"visibility_state":"hidden",
            "lifecycle":"unknown","viewport":{"width":1280.0,"height":720.0,"device_scale_factor":2.0},
            "activation_epoch":7,"focus_epoch":11
        })).unwrap();
        assert!(!state.window_focused);
        assert!(state.tab_active);
        assert_eq!(state.visibility, PageVisibility::Hidden);
        assert_eq!(state.lifecycle, TargetLifecycle::Unknown);
        assert_eq!(state.activation_epoch, 7);
        assert_eq!(state.focus_epoch, 11);
        let policy = ExecutionPolicy::strict_background(true, 3_000);
        assert!(policy.forbid_focus);
        assert!(policy.forbid_activation);
        assert!(policy.allow_scroll);
        assert!(!policy.allow_owned_window);
        assert_eq!(policy.deadline_ms, 3_000);
    }

    #[test]
    fn browser_operation_inventory_matches_dispatch_and_agent_instructions() {
        let source = include_str!("mcp/v3_full.rs");
        let instructions = include_str!(
            "../../../packages/chrome-controlla/plugin/skills/browser-control/SKILL.md"
        );
        let extension = include_str!("../../../extensions/chrome-controlla/background.js");
        let browser_body = source
            .split_once("async fn browser_inner")
            .expect("V3 browser route")
            .1;
        let browser_body = &browser_body[..browser_body
            .find("async fn snapshot(")
            .expect("browser route end")];
        let dispatch_anchors = [
            ("task", "self\n                .task(Parameters(V3TaskArgs"),
            ("skill", "self.browser_skill(args)"),
            (
                "artifact_register",
                "self\n                .artifact_register(",
            ),
            ("artifact_read", "self\n                .artifact_read("),
            (
                "describe",
                "\"browser_actions\":crate::browser_contract::browser_action_inventory()",
            ),
            ("launch", "launch_private_headless("),
            ("close_owned", "action: \"close_owned\".into()"),
            ("move_owned", "action: \"move_owned\".into()"),
            ("activate", "action: \"activate_owned\".into()"),
            ("open", "action: \"open\".into()"),
        ];
        for (action, anchor) in dispatch_anchors {
            assert!(
                browser_body.contains(anchor),
                "public action {action} is not wired to its concrete browser route {anchor}"
            );
        }
        assert!(
            browser_body.contains("shared_session_dispatch_action(&args.action)"),
            "pair/accept/discover/list/release must use the explicit action mapping"
        );
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../extensions/chrome-controlla/manifest.json"
        ))
        .unwrap();
        assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
        assert!(extension.contains("CONTROLLA_EXTENSION_BUILD_ID_PROMISE"));
        assert!(extension.contains("CONTROLLA_EXTENSION_BUILD_ASSETS"));
        assert!(extension.contains("chrome.runtime.getURL(name)"));
        assert!(extension.contains("crypto.subtle.digest(\"SHA-256\", input)"));
        assert!(env!("CONTROLLA_EXTENSION_BUILD_ID").starts_with(&format!(
            "controlla-chrome-extension:{}:",
            env!("CARGO_PKG_VERSION")
        )));
        let allowlist_start = extension
            .find("const SHARED_COMMAND_METHODS = new Set([")
            .unwrap();
        let allowlist_end = allowlist_start
            + extension[allowlist_start..]
                .find("]);")
                .expect("extension command allowlist terminator");
        let allowlist_source = &extension[allowlist_start..allowlist_end];
        let mut extension_methods = allowlist_source
            .split('"')
            .skip(1)
            .step_by(2)
            .collect::<Vec<_>>();
        let mut contract_methods = SHARED_EXTENSION_AUTHORIZED_METHODS.to_vec();
        extension_methods.sort_unstable();
        contract_methods.sort_unstable();
        assert_eq!(
            extension_methods, contract_methods,
            "Rust and extension CDP authorization allowlists drifted"
        );
        for capability in BROWSER_OPERATIONS {
            assert!(
                source.contains(capability.dispatch),
                "dispatch {} missing from V3 implementation",
                capability.dispatch
            );
            let public_action = capability
                .action
                .split(':')
                .next()
                .unwrap_or(capability.action);
            assert!(
                instructions.contains(public_action),
                "agent instructions omit browser operation {public_action}"
            );
            assert!(
                browser_action_inventory()[capability.mode]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|action| action == public_action),
                "describe inventory omits {} for {}",
                public_action,
                capability.mode
            );
        }
        assert!(
            resolve_browser_operation("shared_extension", "headless", "open")
                == OperationResolution::Unsupported {
                    reason: "operation is not declared for this provider and mode"
                }
        );
    }

    #[test]
    fn cross_field_action_contract_is_single_and_password_scoped() {
        for action in ["fill", "type", "select", "slider"] {
            let capability = match resolve_action_capability(
                "shared_extension",
                "foreground",
                action,
                BROWSER_ACTION_CONTRACT_VERSION,
            ) {
                CapabilityResolution::Supported(capability) => capability,
                CapabilityResolution::Unsupported { .. } => {
                    panic!("missing capability for {action}")
                }
            };
            assert_eq!(capability.value, ValueRequirement::Required);
            if action == "select" {
                assert_eq!(capability.evidence, "native_select_exact_value_readback");
            }
            if action == "slider" {
                assert_eq!(
                    capability.evidence,
                    "native_range_min_max_step_value_readback"
                );
            }
            assert!(validate_action_value(action, Some("x"), None).is_ok());
            assert!(validate_action_value(action, None, None).is_err());
        }
        assert!(validate_action_value("secure_fill", None, Some(true)).is_ok());
        assert!(validate_action_value("click", Some("x"), None).is_err());
        assert!(validate_action_value("type", Some("x"), Some(true)).is_err());
        assert!(matches!(
            resolve_action_capability("shared_extension", "foreground", "mystery", 1),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "foreground", "scroll", BROWSER_ACTION_CONTRACT_VERSION),
            CapabilityResolution::Supported(capability) if capability.evidence == "same_container_scroll_readback"
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "foreground", "scroll_to", BROWSER_ACTION_CONTRACT_VERSION),
            CapabilityResolution::Supported(capability) if capability.evidence == "target_visible_center_hit"
        ));
        assert!(matches!(
            resolve_action_capability(
                "shared_extension",
                "background",
                "scroll_to",
                BROWSER_ACTION_CONTRACT_VERSION
            ),
            CapabilityResolution::Supported(_)
        ));
        assert!(matches!(
            resolve_action_capability(
                "dedicated_headless",
                "headless",
                "scroll_to",
                BROWSER_ACTION_CONTRACT_VERSION
            ),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(matches!(
            resolve_action_capability(
                "shared_extension",
                "background",
                "scroll",
                BROWSER_ACTION_CONTRACT_VERSION
            ),
            CapabilityResolution::Supported(_)
        ));
        assert!(matches!(
            resolve_action_capability(
                "dedicated_headless",
                "headless",
                "scroll",
                BROWSER_ACTION_CONTRACT_VERSION
            ),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "foreground", "swipe", BROWSER_ACTION_CONTRACT_VERSION),
            CapabilityResolution::Supported(capability) if capability.evidence == "guarded_touch_movement_readback"
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "foreground", "multi_touch", BROWSER_ACTION_CONTRACT_VERSION),
            CapabilityResolution::Supported(capability) if capability.evidence == "guarded_trusted_two_finger_pinch"
        ));
        assert!(matches!(
            resolve_action_capability(
                "dedicated_headless",
                "headless",
                "multi_touch",
                BROWSER_ACTION_CONTRACT_VERSION
            ),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(matches!(
            resolve_action_capability(
                "shared_extension",
                "background",
                "swipe",
                BROWSER_ACTION_CONTRACT_VERSION
            ),
            CapabilityResolution::Supported(_)
        ));
        assert!(matches!(
            resolve_action_capability(
                "dedicated_headless",
                "headless",
                "swipe",
                BROWSER_ACTION_CONTRACT_VERSION
            ),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "foreground", "hover", 1),
            CapabilityResolution::Supported(capability) if capability.evidence == "exact_css_hover_state"
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "background", "hover", 1),
            CapabilityResolution::Supported(_)
        ));
        assert!(matches!(
            resolve_action_capability("dedicated_headless", "headless", "hover", 1),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(
            matches!(
                resolve_action_capability("dedicated_headless", "headless", "press", 1),
                CapabilityResolution::Unsupported { .. }
            ),
            "press requires the shared guarded key-event route and is not advertised for headless"
        );
    }

    #[test]
    fn capability_key_rejects_unsupported_provider_mode_action_and_version_before_dispatch() {
        let CapabilityResolution::Supported(fill) = resolve_action_capability(
            "shared_extension",
            "background",
            "fill",
            BROWSER_ACTION_CONTRACT_VERSION,
        ) else {
            panic!("shared background fill should be supported")
        };
        assert_eq!(fill.key.provider, "shared_extension");
        assert_eq!(fill.key.mode, "background");
        assert_eq!(fill.effect, "input");
        assert_eq!(fill.evidence, "value_readback");
        assert!(matches!(
            resolve_action_capability("dedicated_headless", "headless", "secure_fill", 1),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "headless", "click", 1),
            CapabilityResolution::Unsupported { .. }
        ));
        assert!(matches!(
            resolve_action_capability("shared_extension", "foreground", "click", 2),
            CapabilityResolution::Unsupported { .. }
        ));
    }
}
