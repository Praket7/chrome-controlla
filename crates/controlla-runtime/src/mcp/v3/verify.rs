//! Typed verification predicates used by workflows and task programs.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VerifyPredicate {
    TextPresent { text: String, scope: Option<String> },
    ValueEquals { target: String, value: String },
    UrlMatches { pattern: String },
    Visible { target: String },
    Hidden { target: String },
    Selected { target: String },
    AppState { adapter: String, state: serde_json::Value },
}

impl VerifyPredicate {
    pub fn is_read_only(&self) -> bool {
        true
    }

    pub fn bounded(self) -> Result<Self, &'static str> {
        let too_long = |value: &str| value.chars().count() > 16_384;
        match &self {
            Self::TextPresent { text, scope } => {
                if too_long(text) || scope.as_ref().is_some_and(|value| too_long(value)) {
                    return Err("text predicate exceeds 16384 characters");
                }
            }
            Self::ValueEquals { value, target } => {
                if too_long(value) || too_long(target) {
                    return Err("predicate value exceeds 16384 characters");
                }
            }
            Self::UrlMatches { pattern } => {
                if too_long(pattern) || !(pattern.starts_with("http://") || pattern.starts_with("https://")) {
                    return Err("URL predicate must be a bounded http(s) pattern");
                }
            }
            Self::Visible { target } | Self::Hidden { target } | Self::Selected { target } => {
                if too_long(target) {
                    return Err("predicate target exceeds 16384 characters");
                }
            }
            Self::AppState { adapter, state } => {
                if too_long(adapter) || state.to_string().len() > 65_536 {
                    return Err("application state predicate exceeds bounds");
                }
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_predicates_are_http_only() {
        assert!(VerifyPredicate::UrlMatches {
            pattern: "https://example.test/".into()
        }
        .bounded()
        .is_ok());
        assert!(VerifyPredicate::UrlMatches {
            pattern: "javascript:alert(1)".into()
        }
        .bounded()
        .is_err());
    }

    #[test]
    fn app_state_predicates_are_bounded() {
        assert!(VerifyPredicate::AppState {
            adapter: "generic".into(),
            state: serde_json::json!({"saved": true}),
        }
        .bounded()
        .is_ok());
        assert!(VerifyPredicate::AppState {
            adapter: "generic".into(),
            state: serde_json::json!("x".repeat(70_000)),
        }
        .bounded()
        .is_err());
    }
}
