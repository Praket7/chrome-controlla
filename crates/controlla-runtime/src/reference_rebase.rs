//! Typed reference-rebase classes.
//!
//! A snapshot's stable key is deliberately conservative: when the DOM changes
//! structure, a reference can disappear while the control still exists. A
//! rebase class tells the caller what happened without turning every soft
//! rerender into a new observation:
//!
//! * `stable` — the same reference is present in the new snapshot.
//! * `rebound` — a high-confidence, unique replacement exists.
//! * `ambiguous` — one or more candidates exist, but identity is not unique.
//! * `removed` — the control is gone from the current document.
//! * `document_replaced` — frame/loader identity changed, so no rebase is safe.
//!
//! This module is pure. It never dispatches and never mutates semantic state.

use crate::reference_reacquire::{
    ReacquireError, ReacquiredReference, SemanticIdentity, reacquire_reference,
};
use crate::semantic_state::SemanticDocumentKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseClass {
    Stable,
    Rebound,
    Ambiguous,
    Removed,
    DocumentReplaced,
}

impl RebaseClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Rebound => "rebound",
            Self::Ambiguous => "ambiguous",
            Self::Removed => "removed",
            Self::DocumentReplaced => "document_replaced",
        }
    }

    pub fn is_safe_for_mutation(self) -> bool {
        matches!(self, Self::Stable | Self::Rebound)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RebaseDecision {
    pub from: String,
    pub to: Option<String>,
    pub class: RebaseClass,
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl RebaseDecision {
    pub fn stable(reference: impl Into<String>) -> Self {
        Self {
            from: reference.into(),
            to: None,
            class: RebaseClass::Stable,
            evidence: vec!["same_public_reference".into()],
        }
    }

    pub fn removed(reference: impl Into<String>) -> Self {
        Self {
            from: reference.into(),
            to: None,
            class: RebaseClass::Removed,
            evidence: vec!["absent_from_current_snapshot".into()],
        }
    }

    pub fn document_replaced(reference: impl Into<String>) -> Self {
        Self {
            from: reference.into(),
            to: None,
            class: RebaseClass::DocumentReplaced,
            evidence: vec!["frame_or_loader_changed".into()],
        }
    }

    pub fn ambiguous(reference: impl Into<String>) -> Self {
        Self {
            from: reference.into(),
            to: None,
            class: RebaseClass::Ambiguous,
            evidence: vec!["no_unique_high_confidence_identity".into()],
        }
    }

    pub fn receipt(&self) -> Value {
        json!({
            "from": self.from,
            "to": self.to,
            "class": self.class,
            "safe_for_mutation": self.class.is_safe_for_mutation(),
            "evidence": self.evidence,
        })
    }
}

/// Classify a lost reference against the current document and snapshot items.
///
/// This is the one bounded rebind attempt. Callers must not loop: `Ambiguous`
/// and `DocumentReplaced` are terminal for the current observation.
pub fn classify_rebase(
    from: &str,
    expected: &SemanticIdentity,
    current_document: Option<&SemanticDocumentKey>,
    current_items: &[Value],
) -> RebaseDecision {
    let Some(current) = current_document else {
        return RebaseDecision::document_replaced(from);
    };
    if !expected.document.same_document(current) {
        return RebaseDecision::document_replaced(from);
    }
    match reacquire_reference(expected, current, current_items) {
        Ok(candidate) => rebound(from, candidate),
        Err(ReacquireError::DocumentChanged) => RebaseDecision::document_replaced(from),
        Err(ReacquireError::Stale) | Err(ReacquireError::Ambiguous) => {
            if current_items.iter().any(|item| {
                item.get("reference").and_then(Value::as_str) == Some(from)
                    || item.get("retained_reference").and_then(Value::as_str) == Some(from)
            }) {
                RebaseDecision::stable(from)
            } else {
                RebaseDecision::ambiguous(from)
            }
        }
    }
}

fn rebound(from: &str, candidate: ReacquiredReference) -> RebaseDecision {
    let mut evidence = vec!["unique_high_confidence_identity".to_owned()];
    if candidate.confidence == crate::reference_reacquire::IdentityConfidence::High {
        evidence.push("high_confidence".to_owned());
    }
    RebaseDecision {
        from: from.to_owned(),
        to: Some(candidate.reference),
        class: RebaseClass::Rebound,
        evidence,
    }
}

/// Build a bounded rebase map for every public reference that disappeared from
/// the new snapshot. The output is intentionally small: callers need the
/// transition and its class, not a second copy of the page.
pub fn rebase_map(
    identities: impl IntoIterator<Item = (String, SemanticIdentity)>,
    current_references: impl IntoIterator<Item = String>,
    current_document: Option<&SemanticDocumentKey>,
    current_items: &[Value],
    limit: usize,
) -> Vec<RebaseDecision> {
    let current = current_references
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    identities
        .into_iter()
        .filter(|(reference, _)| !current.contains(reference))
        .take(limit)
        .map(|(reference, identity)| {
            classify_rebase(&reference, &identity, current_document, current_items)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_reacquire::SemanticIdentity;
    use serde_json::json;

    fn document(loader: &str) -> SemanticDocumentKey {
        SemanticDocumentKey {
            frame_id: "frame".into(),
            loader_id: loader.into(),
            url: "https://example.test/app".into(),
            frame_revision: None,
            document_revision: None,
        }
    }

    fn identity(loader: &str) -> SemanticIdentity {
        SemanticIdentity {
            document: document(loader),
            frame_id: "frame".into(),
            frame_revision: String::new(),
            backend_node_id: Some(7),
            role: "button".into(),
            name: "Save".into(),
            href: None,
            dom_path_fingerprint: "form:0:button:0".into(),
            rect_bucket: Some("\"10,10,20,20\"".into()),
            ancestor_id: Some("editor".into()),
            test_id: None,
            section: None,
        }
    }

    #[test]
    fn a_unique_replacement_is_rebound() {
        let items = [json!({
            "reference":"@c9",
            "retained_reference":"session:9",
            "source_frame":"frame",
            "backend_node_id":9,
            "role":"button",
            "name":"Save",
            "dom_path_fingerprint":"form:0:button:0",
            "rect_bucket":"10,10,20,20",
            "semantic_ancestor":{"id":"editor"}
        })];
        let decision = classify_rebase(
            "@c3",
            &identity("loader-1"),
            Some(&document("loader-1")),
            &items,
        );
        assert_eq!(decision.class, RebaseClass::Rebound);
        assert_eq!(decision.to.as_deref(), Some("@c9"));
        assert!(decision.class.is_safe_for_mutation());
    }

    #[test]
    fn a_replaced_document_never_rebinds() {
        let decision = classify_rebase(
            "@c3",
            &identity("loader-1"),
            Some(&document("loader-2")),
            &[],
        );
        assert_eq!(decision.class, RebaseClass::DocumentReplaced);
        assert!(!decision.class.is_safe_for_mutation());
        assert_eq!(decision.to, None);
    }

    #[test]
    fn duplicate_candidates_are_ambiguous_not_a_guess() {
        let candidate = json!({
            "reference":"@c9",
            "source_frame":"frame",
            "role":"button",
            "name":"Save",
            "dom_path_fingerprint":"form:0:button:0",
            "rect_bucket":"10,10,20,20",
            "semantic_ancestor":{"id":"editor"}
        });
        let items = [candidate.clone(), candidate];
        let decision = classify_rebase(
            "@c3",
            &identity("loader-1"),
            Some(&document("loader-1")),
            &items,
        );
        assert_eq!(decision.class, RebaseClass::Ambiguous);
        assert!(!decision.class.is_safe_for_mutation());
    }

    #[test]
    fn the_rebase_map_is_bounded_and_skips_live_references() {
        let identities = vec![
            ("@c1".to_owned(), identity("loader-1")),
            ("@c2".to_owned(), identity("loader-1")),
        ];
        let decisions = rebase_map(
            identities,
            ["@c1".to_owned()],
            Some(&document("loader-1")),
            &[],
            8,
        );
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].from, "@c2");
        assert_eq!(decisions[0].class, RebaseClass::Ambiguous);
    }

    #[test]
    fn receipts_are_explicit_and_do_not_contain_page_text() {
        let decision = RebaseDecision {
            from: "@c4".into(),
            to: Some("@c11".into()),
            class: RebaseClass::Rebound,
            evidence: vec!["unique_high_confidence_identity".into()],
        };
        let receipt = decision.receipt();
        assert_eq!(receipt["class"], "rebound");
        assert_eq!(receipt["safe_for_mutation"], true);
        assert!(!receipt.to_string().contains("Save"));
    }
}
