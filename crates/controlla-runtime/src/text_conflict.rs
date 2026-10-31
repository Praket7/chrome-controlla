//! Typed text-conflict semantics for fill and type.
//!
//! Text mutations are conditional: the caller names the value it expects to
//! replace. A different live value is not a generic guard failure; it is either
//! a benign retry (the requested value is already present), a human/other-agent
//! edit, or an unreadable surface. The distinction changes the safe next action,
//! so it is computed once and carried as a typed receipt.
//!
//! Values are never included in the receipt. Only lengths and booleans leave
//! this module.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextConflictClass {
    /// The live value is the caller's declared precondition; dispatch is safe.
    None,
    /// The requested value is already present; no write is needed.
    AlreadyAtRequestedValue,
    /// The live value differs from both the precondition and the request.
    HumanEdited,
    /// The surface exposes no readable DOM value (for example a canvas editor).
    Unreadable,
}

impl TextConflictClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::AlreadyAtRequestedValue => "already_at_requested_value",
            Self::HumanEdited => "human_edited",
            Self::Unreadable => "unreadable",
        }
    }

    /// Only a clean precondition may dispatch. An already-satisfied request is
    /// successful without dispatch. The other classes are yields.
    pub fn dispatch_allowed(self) -> bool {
        self == Self::None
    }

    pub fn is_success(self) -> bool {
        matches!(self, Self::None | Self::AlreadyAtRequestedValue)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextConflict {
    pub class: TextConflictClass,
    pub expected_supplied: bool,
    pub observed_matches_expected: bool,
    pub observed_matches_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_length: Option<usize>,
    pub requested_length: usize,
}

impl TextConflict {
    pub fn receipt(&self) -> Value {
        json!({
            "class": self.class,
            "expected_supplied": self.expected_supplied,
            "observed_matches_expected": self.observed_matches_expected,
            "observed_matches_requested": self.observed_matches_requested,
            "observed_length": self.observed_length,
            "requested_length": self.requested_length,
            "dispatch_allowed": self.class.dispatch_allowed(),
            "resolution": match self.class {
                TextConflictClass::None => "dispatch_requested_value",
                TextConflictClass::AlreadyAtRequestedValue => "no_write_needed",
                TextConflictClass::HumanEdited => "yield_for_fresh_observation",
                TextConflictClass::Unreadable => "use_surface_specific_verification",
            },
        })
    }
}

/// Classify the live value against the caller's precondition and request.
///
/// `observed` is `None` when the surface cannot expose a trustworthy DOM value.
/// The caller must not treat `Unreadable` as permission to overwrite.
pub fn classify_text_conflict(
    expected: &str,
    requested: &str,
    observed: Option<&str>,
) -> TextConflict {
    let requested_length = requested.chars().count();
    let Some(observed) = observed else {
        return TextConflict {
            class: TextConflictClass::Unreadable,
            expected_supplied: !expected.is_empty(),
            observed_matches_expected: false,
            observed_matches_requested: false,
            observed_length: None,
            requested_length,
        };
    };
    let observed_matches_expected = observed == expected;
    let observed_matches_requested = observed == requested;
    let class = if observed_matches_requested {
        TextConflictClass::AlreadyAtRequestedValue
    } else if observed_matches_expected {
        TextConflictClass::None
    } else {
        TextConflictClass::HumanEdited
    };
    TextConflict {
        class,
        expected_supplied: !expected.is_empty(),
        observed_matches_expected,
        observed_matches_requested,
        observed_length: Some(observed.chars().count()),
        requested_length,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_precondition_dispatches() {
        let conflict = classify_text_conflict("old", "new", Some("old"));
        assert_eq!(conflict.class, TextConflictClass::None);
        assert!(conflict.class.dispatch_allowed());
        assert!(conflict.class.is_success());
    }

    #[test]
    fn an_existing_requested_value_is_success_without_a_write() {
        let conflict = classify_text_conflict("old", "new", Some("new"));
        assert_eq!(conflict.class, TextConflictClass::AlreadyAtRequestedValue);
        assert!(!conflict.class.dispatch_allowed());
        assert!(conflict.class.is_success());
        assert_eq!(conflict.receipt()["resolution"], "no_write_needed");
    }

    #[test]
    fn a_third_value_yields_instead_of_overwriting() {
        let conflict = classify_text_conflict("old", "new", Some("human edit"));
        assert_eq!(conflict.class, TextConflictClass::HumanEdited);
        assert!(!conflict.class.dispatch_allowed());
        assert!(!conflict.class.is_success());
        assert_eq!(
            conflict.receipt()["resolution"],
            "yield_for_fresh_observation"
        );
    }

    #[test]
    fn unreadable_surfaces_are_not_permission_to_overwrite() {
        let conflict = classify_text_conflict("", "new", None);
        assert_eq!(conflict.class, TextConflictClass::Unreadable);
        assert!(!conflict.class.dispatch_allowed());
        assert_eq!(conflict.observed_length, None);
    }

    #[test]
    fn receipts_never_include_the_values() {
        let conflict = classify_text_conflict("old-secret", "new-secret", Some("human-secret"));
        let receipt = conflict.receipt().to_string();
        assert!(!receipt.contains("secret"));
        assert!(receipt.contains("observed_length"));
    }
}
