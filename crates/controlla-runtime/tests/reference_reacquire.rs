#![cfg(target_os = "macos")]
use controlla_runtime::reference_reacquire::{IdentityConfidence, SemanticIdentity};
use controlla_runtime::semantic_state::SemanticDocumentKey;
use serde_json::json;

#[test]
fn lost_cdp_object_reacquires_with_high_confidence() {
    let id = SemanticIdentity {
        frame_id: "f".into(),
        backend_node_id: Some(123),
        role: "link".into(),
        name: "Go".into(),
        href: Some("https://example.test/a".into()),
        dom_path_fingerprint: "a:0".into(),
        rect_bucket: None,
        ancestor_id: None,
    };
    let doc = SemanticDocumentKey {
        frame_id: "f".into(),
        loader_id: "l".into(),
        url: "https://example.test".into(),
        frame_revision: None,
        document_revision: None,
    };
    let items = vec![
        json!({"reference":"@c1","role":"link","name":"Go","href":"https://example.test/a","dom_path_fingerprint":"a:0","backend_node_id":123}),
    ];
    let r = controlla_runtime::reference_reacquire::reacquire_reference(&id, &doc, &items).unwrap();
    assert_eq!(r.confidence, IdentityConfidence::High);
}

#[test]
fn reordered_duplicates_do_not_alias() {
    let id = SemanticIdentity {
        frame_id: "f".into(),
        backend_node_id: None,
        role: "link".into(),
        name: "Item".into(),
        href: None,
        dom_path_fingerprint: "a:0".into(),
        rect_bucket: None,
        ancestor_id: None,
    };
    let doc = SemanticDocumentKey {
        frame_id: "f".into(),
        loader_id: "l".into(),
        url: "https://example.test".into(),
        frame_revision: None,
        document_revision: None,
    };
    let items = vec![
        json!({"reference":"@c1","role":"link","name":"Item","href":null,"dom_path_fingerprint":"a:1"}),
        json!({"reference":"@c2","role":"link","name":"Item","href":null,"dom_path_fingerprint":"a:0"}),
    ];
    // High confidence still finds the correct fingerprint, not ordinal
    let r = controlla_runtime::reference_reacquire::reacquire_reference(&id, &doc, &items).unwrap();
    assert_eq!(r.reference, "@c2");
}
