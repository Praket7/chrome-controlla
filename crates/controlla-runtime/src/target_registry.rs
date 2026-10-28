//! Document-scoped target registry.
//!
//! `@cN` references remain the public handle, but each one is backed by a
//! semantic identity for the lifetime of its document. Hard boundaries (tab,
//! frame or document replacement) invalidate the identity permanently. Soft
//! drift (rerender, row insertion, layout shift) gets exactly one bounded
//! rebind attempt before ambiguity is surfaced to the caller.

use crate::reference_reacquire::{
    ReacquireError, ReacquiredReference, SemanticIdentity, reacquire_reference,
};
use crate::semantic_state::SemanticDocumentKey;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Invalidation {
    /// Same document; an existing identity remains usable.
    None,
    /// Same tab and frame lineage, but the document was replaced.
    Hard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardBoundary {
    TabReplaced,
    FrameReplaced,
    DocumentReplaced,
}

impl HardBoundary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TabReplaced => "tab_replaced",
            Self::FrameReplaced => "frame_replaced",
            Self::DocumentReplaced => "document_replaced",
        }
    }
}

/// Classify a stale reference against the current document identity. A missing
/// document means the tab or session is gone. URL changes count as document
/// replacement even when Chrome reuses the loader id.
pub fn classify_invalidation(
    expected: &SemanticDocumentKey,
    current: Option<&SemanticDocumentKey>,
) -> Invalidation {
    match current {
        Some(current)
            if current.frame_id == expected.frame_id
                && current.loader_id == expected.loader_id
                && current.url == expected.url =>
        {
            Invalidation::None
        }
        _ => Invalidation::Hard,
    }
}

pub fn hard_boundary(
    expected: &SemanticDocumentKey,
    current: Option<&SemanticDocumentKey>,
) -> Option<HardBoundary> {
    let current = current?;
    if current.frame_id != expected.frame_id {
        Some(HardBoundary::FrameReplaced)
    } else if current.url != expected.url || current.loader_id != expected.loader_id {
        Some(HardBoundary::DocumentReplaced)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub enum RebindOutcome {
    /// Exactly one uniquely identifiable candidate in the same document.
    Rebound(ReacquiredReference),
    /// A hard boundary: identity cannot be carried across documents.
    Invalidated(HardBoundary),
    /// Soft drift, but zero or several candidates satisfy the identity.
    Ambiguous(ReacquireError),
}

impl RebindOutcome {
    pub fn as_receipt(&self) -> Value {
        match self {
            Self::Rebound(candidate) => serde_json::json!({
                "status": "rebound",
                "reference": candidate.reference,
                "confidence": "high",
                "attempts": 1
            }),
            Self::Invalidated(boundary) => serde_json::json!({
                "status": "invalidated",
                "boundary": boundary.as_str(),
                "attempts": 0
            }),
            Self::Ambiguous(error) => serde_json::json!({
                "status": "ambiguous",
                "reason": error.to_string(),
                "attempts": 1
            }),
        }
    }
}

/// One bounded rebind attempt. Callers must not loop: an ambiguous result is a
/// terminal observation for this action, not an invitation to retry mutations.
pub fn rebind_in_document(
    expected: &SemanticIdentity,
    current_document: Option<&SemanticDocumentKey>,
    items: &[Value],
) -> RebindOutcome {
    match classify_invalidation(&expected.document, current_document) {
        Invalidation::Hard => RebindOutcome::Invalidated(
            hard_boundary(&expected.document, current_document)
                .unwrap_or(HardBoundary::TabReplaced),
        ),
        Invalidation::None => {
            match reacquire_reference(
                expected,
                current_document.expect("same document implies a current document"),
                items,
            ) {
                Ok(candidate) => RebindOutcome::Rebound(candidate),
                Err(error) => RebindOutcome::Ambiguous(error),
            }
        }
    }
}

/// Convenience wrapper for callers that only need a candidate or a single
/// bounded explanation for why rebinding was refused.
pub fn rebind_result(
    expected: &SemanticIdentity,
    current_document: Option<&SemanticDocumentKey>,
    items: &[Value],
) -> Result<ReacquiredReference, String> {
    match rebind_in_document(expected, current_document, items) {
        RebindOutcome::Rebound(candidate) => Ok(candidate),
        RebindOutcome::Invalidated(boundary) => {
            Err(format!("document boundary changed ({})", boundary.as_str()))
        }
        RebindOutcome::Ambiguous(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn identity(document: SemanticDocumentKey) -> SemanticIdentity {
        SemanticIdentity {
            document,
            frame_id: "frame".into(),
            frame_revision: String::new(),
            backend_node_id: Some(1),
            role: "button".into(),
            name: "Save".into(),
            href: None,
            dom_path_fingerprint: "form:0:button:0".into(),
            rect_bucket: Some("\"10,10,20,20\"".into()),
            ancestor_id: Some("dialog-editor".into()),
            test_id: None,
            section: None,
        }
    }

    #[test]
    fn document_replacement_is_a_hard_boundary() {
        let expected = identity(document("loader-1"));
        let current = document("loader-2");
        assert_eq!(
            classify_invalidation(&expected.document, Some(&current)),
            Invalidation::Hard
        );
        assert_eq!(
            rebind_in_document(&identity(document("loader-1")), Some(&current), &[]).as_receipt()["boundary"],
            "document_replaced"
        );
    }

    #[test]
    fn missing_tab_is_a_hard_boundary() {
        let expected = identity(document("loader-1"));
        assert!(matches!(
            rebind_in_document(&expected, None, &[]),
            RebindOutcome::Invalidated(HardBoundary::TabReplaced)
        ));
    }

    #[test]
    fn same_document_react_rerender_rebinds_once() {
        let expected = identity(document("loader-1"));
        let items = [json!({
            "reference": "@c9",
            "retained_reference": "session:9",
            "source_frame": "frame",
            "backend_node_id": 77,
            "role": "button",
            "name": "Save",
            "dom_path_fingerprint": "form:0:button:0",
            "rect_bucket": "10,10,20,20",
            "semantic_ancestor": {"id": "dialog-editor"}
        })];
        let outcome = rebind_in_document(&expected, Some(&document("loader-1")), &items);
        assert!(matches!(outcome, RebindOutcome::Rebound(_)));
        assert_eq!(outcome.as_receipt()["attempts"], 1);
    }

    #[test]
    fn ambiguous_duplicates_stay_terminal() {
        let expected = identity(document("loader-1"));
        let duplicate = json!({
            "reference": "@c9",
            "source_frame": "frame",
            "role": "button",
            "name": "Save",
            "dom_path_fingerprint": "form:0:button:0",
            "rect_bucket": "10,10,20,20",
            "semantic_ancestor": {"id": "dialog-editor"}
        });
        let items = [duplicate.clone(), duplicate];
        assert!(matches!(
            rebind_in_document(&expected, Some(&document("loader-1")), &items),
            RebindOutcome::Ambiguous(ReacquireError::Ambiguous)
        ));
    }
}
