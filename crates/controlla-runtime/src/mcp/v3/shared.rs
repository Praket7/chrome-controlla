//! Shared V3 receipt and bound helpers.

use serde_json::{Value, json};

pub fn bounded_string(value: &Value, max_chars: usize) -> Value {
    let Some(text) = value.as_str() else {
        return Value::Null;
    };
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        json!({ "prefix": prefix, "truncated": true, "chars": text.chars().count() })
    } else {
        Value::String(prefix)
    }
}

pub fn receipt_status(verified: bool, dispatch_may_have_occurred: bool) -> &'static str {
    match (verified, dispatch_may_have_occurred) {
        (true, _) => "verified",
        (false, true) => "unknown",
        (false, false) => "not_dispatched",
    }
}

pub fn compact_success(context: Value, result: Value) -> Value {
    json!({ "status": "completed", "context": context, "result": result, "verified": true })
}

pub fn recovery_summary(automatic: u32, user_visible: u32) -> Option<Value> {
    (automatic > 0 || user_visible > 0).then(|| {
        json!({
            "automatic_recoveries": automatic,
            "user_visible_recoveries": user_visible,
        })
    })
}

pub fn value_has_unknown(value: &Value) -> bool {
    // `dispatched_unverified` is a finished transaction with acknowledged
    // dispatch, not an ambiguous delivery: the mutation is complete and the
    // receipt tells the caller to inspect before retrying. Retaining the target
    // lock for two minutes after it made a typed document unusable.
    let resolved = matches!(
        value["status"].as_str(),
        Some(
            "verified"
                | "complete"
                | "completed"
                | "passed"
                | "not_dispatched"
                | "dispatched_unverified"
        )
    );
    value["status"] == "unknown"
        || (!resolved
            && (value["dispatch_may_have_occurred"] == true
                || value["dispatch_may_have_occurred"] == "unknown"))
        || value
            .as_object()
            .is_some_and(|object| object.values().any(value_has_unknown))
        || value
            .as_array()
            .is_some_and(|items| items.iter().any(value_has_unknown))
}

pub fn value_may_have_dispatched(value: &Value) -> bool {
    value["dispatch_may_have_occurred"] == true
        || value["dispatch_may_have_occurred"] == "unknown"
        || value["status"] == "unknown"
        || value
            .as_object()
            .is_some_and(|object| object.values().any(value_may_have_dispatched))
        || value
            .as_array()
            .is_some_and(|items| items.iter().any(value_may_have_dispatched))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_preserves_uncertainty() {
        assert_eq!(receipt_status(true, false), "verified");
        assert_eq!(receipt_status(false, true), "unknown");
        assert_eq!(receipt_status(false, false), "not_dispatched");
    }

    #[test]
    fn finished_transactions_are_not_treated_as_ambiguous() {
        assert!(!value_has_unknown(&json!({
            "status": "dispatched_unverified",
            "dispatch_may_have_occurred": true
        })));
        assert!(value_has_unknown(&json!({"status": "unknown"})));
        assert!(!value_has_unknown(&json!({"status": "verified"})));
    }

    #[test]
    fn bounded_strings_never_truncate_silently() {
        assert_eq!(bounded_string(&json!("short"), 10), json!("short"));
        let bounded = bounded_string(&json!("0123456789"), 4);
        assert_eq!(bounded["prefix"], "0123");
        assert_eq!(bounded["truncated"], true);
        assert_eq!(bounded_string(&json!(null), 4), Value::Null);
    }
}
