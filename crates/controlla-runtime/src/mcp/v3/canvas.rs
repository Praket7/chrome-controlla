//! Canvas-editor presentation policy.
//!
//! An editor that paints its own text (Google Docs) ignores scripted
//! insertion, and Chrome defers trusted key input for a tab it does not
//! consider visible. Dispatching anyway yields `dispatched_unverified` over an
//! empty document, which a caller cannot tell apart from success, so a hidden
//! tab is refused before anything is sent.
//!
//! The presentation that works is an unfocused window: the tab is visible to
//! Chrome, the window never takes focus, and trusted keys and document copy
//! readback both work.

use serde_json::{Value, json};

pub fn hidden_canvas_editor_refusal(
    editor_value_unreadable: bool,
    visibility_state: Option<&str>,
) -> Option<Value> {
    (editor_value_unreadable && visibility_state != Some("visible")).then(receipt)
}

fn receipt() -> Value {
    json!({
        "status": "not_dispatched",
        "action": "type",
        "reason_code": "hidden_canvas_editor_requires_visible_tab",
        "dispatch_may_have_occurred": false,
        "reason": "this editor paints its own text, and Chrome defers trusted key input for a hidden tab; scripted insertion is ignored, so the document would stay empty",
        "next_action": "open the same URL with mode=background and target_policy=create_background_window (or move the owned tab into that unfocused window), then type again; the tab stays visible to Chrome without taking focus"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_canvas_editor_refusal_never_claims_a_dispatch() {
        let receipt = hidden_canvas_editor_refusal(true, Some("hidden")).unwrap();
        assert_eq!(receipt["status"], "not_dispatched");
        assert_eq!(receipt["dispatch_may_have_occurred"], false);
        assert_eq!(
            receipt["reason_code"],
            "hidden_canvas_editor_requires_visible_tab"
        );
        assert!(
            receipt["next_action"]
                .as_str()
                .unwrap_or_default()
                .contains("create_background_window")
        );
    }

    #[test]
    fn visible_or_readable_editors_are_never_refused() {
        assert!(hidden_canvas_editor_refusal(true, Some("visible")).is_none());
        assert!(hidden_canvas_editor_refusal(false, Some("hidden")).is_none());
        assert!(hidden_canvas_editor_refusal(false, None).is_none());
    }
}
