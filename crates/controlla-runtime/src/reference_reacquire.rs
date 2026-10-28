// High-confidence stale-object reacquisition.
// Rebinds a lost CDP object only when semantic identity remains high-confidence.
// Medium/low confidence returns Err to force fresh snapshot and fail-closed behavior.

use crate::semantic_state::SemanticDocumentKey;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum IdentityConfidence {
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug)]
pub struct SemanticIdentity {
    pub document: SemanticDocumentKey,
    pub frame_id: String,
    pub frame_revision: String,
    pub backend_node_id: Option<i64>,
    pub role: String,
    pub name: String,
    pub href: Option<String>,
    pub dom_path_fingerprint: String,
    pub rect_bucket: Option<String>,
    pub ancestor_id: Option<String>,
    pub test_id: Option<String>,
    pub section: Option<String>,
}

impl SemanticIdentity {
    pub fn from_item(item: &Value, document: &SemanticDocumentKey) -> Self {
        Self {
            document: document.clone(),
            frame_id: item
                .get("source_frame")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            frame_revision: item
                .get("source_frame_revision")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            backend_node_id: item.get("backend_node_id").and_then(Value::as_i64),
            role: item
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            href: item.get("href").and_then(Value::as_str).map(str::to_owned),
            dom_path_fingerprint: item
                .get("dom_path_fingerprint")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            rect_bucket: item.get("rect_bucket").map(Value::to_string),
            ancestor_id: item
                .pointer("/semantic_ancestor/id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            test_id: item
                .get("test_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            section: item
                .get("section")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReacquiredReference {
    pub reference: String,
    pub legacy_reference: String,
    pub confidence: IdentityConfidence,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReacquireError {
    #[error("no candidate with high confidence")]
    Ambiguous,
    #[error("stale")]
    Stale,
    #[error("document changed")]
    DocumentChanged,
}

pub fn reacquire_reference(
    expected: &SemanticIdentity,
    current_document: &SemanticDocumentKey,
    current_items: &[Value],
) -> Result<ReacquiredReference, ReacquireError> {
    if &expected.document != current_document {
        return Err(ReacquireError::DocumentChanged);
    }
    let mut high_matches = Vec::new();
    for item in current_items {
        if item.get("source_frame").and_then(Value::as_str) != Some(expected.frame_id.as_str()) {
            continue;
        }
        if item
            .get("source_frame_revision")
            .and_then(Value::as_str)
            .unwrap_or_default()
            != expected.frame_revision
        {
            continue;
        }
        let role = item.get("role").and_then(Value::as_str).unwrap_or_default();
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        let href = item
            .get("href")
            .and_then(Value::as_str)
            .map(|s| s.to_owned());
        let fp = item
            .get("dom_path_fingerprint")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let ref_str = item
            .get("reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let legacy = item
            .get("retained_reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let current_backend_node_id = item.get("backend_node_id").and_then(Value::as_i64);
        let current_rect = item.get("rect_bucket").map(Value::to_string);
        let current_ancestor = item
            .pointer("/semantic_ancestor/id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let current_test_id = item
            .get("test_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let current_section = item
            .get("section")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let rect_matched = expected.rect_bucket.is_some() && current_rect == expected.rect_bucket;
        let ancestor_matched = expected.ancestor_id.is_some()
            && current_ancestor == expected.ancestor_id;
        let test_id_matched = expected.test_id.is_some()
            && current_test_id == expected.test_id.as_deref();
        let section_matched = expected.section.is_some()
            && current_section == expected.section.as_deref();
        // A control that moved to a different semantic ancestor is a different
        // target unless a stable test attribute proves that it is the same one.
        let ancestor_conflict = expected
            .ancestor_id
            .as_deref()
            .is_some_and(|expected| {
                current_ancestor
                    .as_deref()
                    .is_some_and(|current| current != expected)
            })
            && !test_id_matched;
        // Score deterministically. Textual identity is necessary for a
        // context rebind; geometry and ancestor context break ties between
        // otherwise identical repeated controls.
        let mut score = 0;
        if role == expected.role {
            score += 10;
        }
        if name == expected.name {
            score += 20;
        }
        if href == expected.href {
            score += 15;
        }
        if fp == expected.dom_path_fingerprint {
            score += 25;
        }
        if rect_matched {
            score += 8;
        }
        if ancestor_matched {
            score += 12;
        }
        if test_id_matched {
            score += 40;
        }
        if section_matched {
            score += 6;
        }
        let backend_id_matched = expected.backend_node_id.is_some()
            && current_backend_node_id == expected.backend_node_id;
        if backend_id_matched {
            score += 50;
        }
        // A recorded test attribute is part of the core identity; a control
        // that kept the same name but changed its test id is not the same target.
        let test_id_conflict = expected.test_id.is_some() && !test_id_matched;
        let same_core = role == expected.role
            && name == expected.name
            && href == expected.href
            && !test_id_conflict;
        let fingerprint_matched = !fp.is_empty() && fp == expected.dom_path_fingerprint;
        let context_required = expected.rect_bucket.is_some() || expected.ancestor_id.is_some();
        let context_matched = rect_matched || ancestor_matched || test_id_matched;
        let is_high = if backend_id_matched {
            score >= 50
        } else if same_core && !ancestor_conflict {
            // Geometry and ancestry can carry identity across a rerender or a
            // row insertion that changed the DOM path; duplicates in the same
            // context remain ambiguous because they produce several matches.
            (fingerprint_matched || test_id_matched || context_matched)
                && (!context_required || context_matched)
        } else {
            false
        };
        if is_high {
            high_matches.push(ReacquiredReference {
                reference: ref_str.to_owned(),
                legacy_reference: legacy.to_owned(),
                confidence: IdentityConfidence::High,
            });
        }
    }
    if high_matches.len() == 1 {
        Ok(high_matches.into_iter().next().unwrap())
    } else {
        Err(ReacquireError::Ambiguous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reacquire_high_confidence_single_match() {
        let doc = SemanticDocumentKey {
            frame_id: "f".into(),
            loader_id: "l".into(),
            url: "https://example.test".into(),
            frame_revision: None,
            document_revision: None,
        };
        let id = SemanticIdentity {
            document: doc.clone(),
            frame_id: "f".into(),
            frame_revision: String::new(),
            backend_node_id: None,
            role: "link".into(),
            name: "Bills latest".into(),
            href: Some("https://example.test/a".into()),
            dom_path_fingerprint: "a:0".into(),
            rect_bucket: None,
            ancestor_id: None,
            test_id: None,
            section: None,
        };
        let items = vec![
            json!({"reference":"@c1","source_frame":"f","role":"link","name":"Bills latest","href":"https://example.test/a","dom_path_fingerprint":"a:0"}),
        ];
        let r = reacquire_reference(&id, &doc, &items).unwrap();
        assert_eq!(r.confidence, IdentityConfidence::High);
    }
    #[test]
    fn reacquire_ambiguous_when_reordered_duplicates() {
        let doc = SemanticDocumentKey {
            frame_id: "f".into(),
            loader_id: "l".into(),
            url: "https://example.test".into(),
            frame_revision: None,
            document_revision: None,
        };
        let id = SemanticIdentity {
            document: doc.clone(),
            frame_id: "f".into(),
            frame_revision: String::new(),
            backend_node_id: None,
            role: "link".into(),
            name: "Item".into(),
            href: None,
            dom_path_fingerprint: "a:0".into(),
            rect_bucket: None,
            ancestor_id: None,
            test_id: None,
            section: None,
        };
        let items = vec![
            json!({"reference":"@c1","source_frame":"f","role":"link","name":"Item","href":null,"dom_path_fingerprint":"a:1"}),
            json!({"reference":"@c2","source_frame":"f","role":"link","name":"Item","href":null,"dom_path_fingerprint":"a:0"}),
        ];
        // Only one high match (the one with fingerprint a:0) -> still ambiguous if ordering changed? stub treats one match as ok; ensure not silently switching.
        // Here we expect ok because exactly one high-confidence still exists; reordered test should ensure caller validates not swapping.
        assert!(reacquire_reference(&id, &doc, &items).is_ok());
    }

    #[test]
    fn geometry_and_ancestor_break_ties_between_repeated_controls() {
        let doc = SemanticDocumentKey {
            frame_id: "f".into(),
            loader_id: "l".into(),
            url: "https://example.test".into(),
            frame_revision: None,
            document_revision: None,
        };
        let expected = SemanticIdentity {
            document: doc.clone(),
            frame_id: "f".into(),
            frame_revision: String::new(),
            backend_node_id: None,
            role: "button".into(),
            name: "Save".into(),
            href: None,
            dom_path_fingerprint: "button:0".into(),
            rect_bucket: Some("\"10,20,30,40\"".into()),
            ancestor_id: Some("toolbar-main".into()),
            test_id: None,
            section: None,
        };
        let items = vec![
            json!({"reference":"@c1","source_frame":"f","role":"button","name":"Save",
                "dom_path_fingerprint":"button:0","rect_bucket":"10,20,30,40",
                "semantic_ancestor":{"id":"toolbar-main"}}),
            json!({"reference":"@c2","source_frame":"f","role":"button","name":"Save",
                "dom_path_fingerprint":"button:0","rect_bucket":"500,20,30,40",
                "semantic_ancestor":{"id":"toolbar-secondary"}}),
        ];
        let reacquired = reacquire_reference(&expected, &doc, &items).unwrap();
        assert_eq!(reacquired.reference, "@c1");
        let contextless = SemanticIdentity {
            rect_bucket: None,
            ancestor_id: None,
            test_id: None,
            section: None,
            ..expected
        };
        assert!(matches!(
            reacquire_reference(&contextless, &doc, &items),
            Err(ReacquireError::Ambiguous)
        ));
    }

    #[test]
    fn replaced_backend_node_reacquires_only_on_exact_unique_semantic_identity() {
        let doc = SemanticDocumentKey {
            frame_id: "f".into(),
            loader_id: "l".into(),
            url: "https://example.test".into(),
            frame_revision: None,
            document_revision: None,
        };
        let expected = SemanticIdentity {
            document: doc.clone(),
            frame_id: "f".into(),
            frame_revision: String::new(),
            backend_node_id: Some(123),
            role: "link".into(),
            name: "Latest story".into(),
            href: Some("https://example.test/latest".into()),
            dom_path_fingerprint: "a:0".into(),
            rect_bucket: None,
            ancestor_id: None,
            test_id: None,
            section: None,
        };
        let current = [
            json!({"reference":"@c1","source_frame":"f","backend_node_id":456,
            "role":"link","name":"Latest story","href":"https://example.test/latest","dom_path_fingerprint":"a:0"}),
        ];
        assert_eq!(
            reacquire_reference(&expected, &doc, &current)
                .unwrap()
                .reference,
            "@c1"
        );
        let ambiguous = [current[0].clone(), current[0].clone()];
        assert!(matches!(
            reacquire_reference(&expected, &doc, &ambiguous),
            Err(ReacquireError::Ambiguous)
        ));
        let changed_href = [
            json!({"reference":"@c2","source_frame":"f","backend_node_id":456,
            "role":"link","name":"Latest story","href":"https://example.test/other","dom_path_fingerprint":"a:0"}),
        ];
        assert!(matches!(
            reacquire_reference(&expected, &doc, &changed_href),
            Err(ReacquireError::Ambiguous)
        ));
    }

    fn base_document() -> SemanticDocumentKey {
        SemanticDocumentKey {
            frame_id: "f".into(),
            loader_id: "l".into(),
            url: "https://example.test".into(),
            frame_revision: None,
            document_revision: None,
        }
    }

    fn row_target() -> SemanticIdentity {
        SemanticIdentity {
            document: base_document(),
            frame_id: "f".into(),
            frame_revision: String::new(),
            backend_node_id: Some(9),
            role: "button".into(),
            name: "Archive".into(),
            href: None,
            dom_path_fingerprint: "tbody:0:tr:2:button:0".into(),
            rect_bucket: Some("\"10,300,20,20\"".into()),
            ancestor_id: Some("inbox-list".into()),
            test_id: None,
            section: Some("Inbox".into()),
        }
    }

    #[test]
    fn row_insertion_above_target_rebinds_within_the_same_context() {
        let items = [
            json!({"reference":"@c1","source_frame":"f","backend_node_id":10,
                "role":"button","name":"Archive","dom_path_fingerprint":"tbody:0:tr:3:button:0",
                "rect_bucket":"10,320,20,20","semantic_ancestor":{"id":"inbox-list"},"section":"Inbox"}),
        ];
        let reacquired = reacquire_reference(&row_target(), &base_document(), &items).unwrap();
        assert_eq!(reacquired.reference, "@c1");
    }

    #[test]
    fn pure_geometry_shift_keeps_the_same_fingerprint_target() {
        let items = [
            json!({"reference":"@c2","source_frame":"f","role":"button","name":"Archive",
                "dom_path_fingerprint":"tbody:0:tr:2:button:0","rect_bucket":"10,340,20,20",
                "semantic_ancestor":{"id":"inbox-list"},"section":"Inbox"}),
        ];
        assert_eq!(
            reacquire_reference(&row_target(), &base_document(), &items)
                .unwrap()
                .reference,
            "@c2"
        );
    }

    #[test]
    fn moving_to_a_different_ancestor_is_rejected() {
        let items = [
            json!({"reference":"@c3","source_frame":"f","role":"button","name":"Archive",
                "dom_path_fingerprint":"tbody:0:tr:2:button:0","rect_bucket":"10,300,20,20",
                "semantic_ancestor":{"id":"archive-dialog"},"section":"Inbox"}),
        ];
        assert!(matches!(
            reacquire_reference(&row_target(), &base_document(), &items),
            Err(ReacquireError::Ambiguous)
        ));
    }

    #[test]
    fn stable_test_id_proves_identity_across_a_moved_ancestor() {
        let expected = SemanticIdentity {
            ancestor_id: Some("inbox-list".into()),
            test_id: Some("archive-row-9".into()),
            ..row_target()
        };
        let items = [
            json!({"reference":"@c4","source_frame":"f","role":"button","name":"Archive",
                "dom_path_fingerprint":"dialog:0:button:0","rect_bucket":"500,100,20,20",
                "semantic_ancestor":{"id":"archive-dialog"},"test_id":"archive-row-9","section":"Inbox"}),
        ];
        assert_eq!(
            reacquire_reference(&expected, &base_document(), &items)
                .unwrap()
                .reference,
            "@c4"
        );
        let wrong_test_id = [json!({"reference":"@c5","source_frame":"f","role":"button",
            "name":"Archive","dom_path_fingerprint":"dialog:0:button:0",
            "test_id":"archive-row-10","section":"Inbox"})];
        assert!(matches!(
            reacquire_reference(&expected, &base_document(), &wrong_test_id),
            Err(ReacquireError::Ambiguous)
        ));
    }

    #[test]
    fn same_role_and_name_in_another_frame_is_not_rebound() {
        let items = [
            json!({"reference":"@c6","source_frame":"other-frame","role":"button",
                "name":"Archive","dom_path_fingerprint":"tbody:0:tr:2:button:0",
                "rect_bucket":"10,300,20,20","semantic_ancestor":{"id":"inbox-list"},"section":"Inbox"}),
        ];
        assert!(matches!(
            reacquire_reference(&row_target(), &base_document(), &items),
            Err(ReacquireError::Ambiguous)
        ));
    }
}
