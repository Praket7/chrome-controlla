//! Compact action receipts. The receipt keeps uncertainty explicit and never
//! invites replay of a mutation whose dispatch may have occurred.

use serde_json::{Value, json};

pub fn act_receipt(
    action: &str,
    status: &str,
    verified: bool,
    dispatch_may_have_occurred: bool,
    reason_code: Option<&str>,
) -> Value {
    let next_action = if status == "unknown" {
        Some("Inspect the requested postcondition read-only; do not replay the mutation.")
    } else if status == "not_dispatched" {
        Some("Correct the target or precondition, then retry once.")
    } else {
        None
    };
    json!({
        "status": status,
        "action": action,
        "verified": verified,
        "dispatch_may_have_occurred": dispatch_may_have_occurred,
        "reason_code": reason_code,
        "next_action": next_action,
    })
}

pub fn status_from(verified: bool, dispatch_may_have_occurred: bool) -> &'static str {
    match (verified, dispatch_may_have_occurred) {
        (true, _) => "verified",
        (false, true) => "unknown",
        (false, false) => "not_dispatched",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_mutations_never_suggest_replay() {
        let receipt = act_receipt("click", "unknown", false, true, Some("focus_changed"));
        assert_eq!(receipt["status"], "unknown");
        assert!(
            receipt["next_action"]
                .as_str()
                .unwrap()
                .contains("do not replay")
        );
    }

    #[test]
    fn status_mapping_matches_execution_semantics() {
        assert_eq!(status_from(true, true), "verified");
        assert_eq!(status_from(false, true), "unknown");
        assert_eq!(status_from(false, false), "not_dispatched");
    }
}
