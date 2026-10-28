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
    fn bounded_strings_never_truncate_silently() {
        assert_eq!(bounded_string(&json!("short"), 10), json!("short"));
        let bounded = bounded_string(&json!("0123456789"), 4);
        assert_eq!(bounded["prefix"], "0123");
        assert_eq!(bounded["truncated"], true);
        assert_eq!(bounded_string(&json!(null), 4), Value::Null);
    }
}
