#![cfg(target_os = "macos")]
use controlla_runtime::reference_reacquire::{IdentityConfidence, SemanticIdentity};
use controlla_runtime::semantic_state::SemanticDocumentKey;
use serde_json::json;

#[test]
fn lost_cdp_object_reacquires_with_high_confidence() {
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
        backend_node_id: Some(123),
        role: "link".into(),
        name: "Go".into(),
        href: Some("https://example.test/a".into()),
        dom_path_fingerprint: "a:0".into(),
        rect_bucket: None,
        ancestor_id: None,
        test_id: None,
        section: None,
    };
    let items = vec![
        json!({"reference":"@c1","source_frame":"f","role":"link","name":"Go","href":"https://example.test/a","dom_path_fingerprint":"a:0","backend_node_id":123}),
    ];
    let r = controlla_runtime::reference_reacquire::reacquire_reference(&id, &doc, &items).unwrap();
    assert_eq!(r.confidence, IdentityConfidence::High);
}

#[test]
fn reordered_duplicates_do_not_alias() {
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
    // High confidence still finds the correct fingerprint, not ordinal
    let r = controlla_runtime::reference_reacquire::reacquire_reference(&id, &doc, &items).unwrap();
    assert_eq!(r.reference, "@c2");
}

#[test]
fn child_frame_navigation_invalidates_semantic_reacquisition() {
    let doc = SemanticDocumentKey {
        frame_id: "root".into(),
        loader_id: "root-loader".into(),
        url: "https://example.test".into(),
        frame_revision: None,
        document_revision: None,
    };
    let identity = SemanticIdentity {
        document: doc.clone(),
        frame_id: "main/iframe[1]".into(),
        frame_revision: "1000".into(),
        backend_node_id: None,
        role: "link".into(),
        name: "Continue".into(),
        href: None,
        dom_path_fingerprint: "a:0".into(),
        rect_bucket: None,
        ancestor_id: None,
        test_id: None,
        section: None,
    };
    let reloaded_frame = json!({"reference":"@c2","source_frame":"main/iframe[1]","source_frame_revision":"2000","role":"link","name":"Continue","href":null,"dom_path_fingerprint":"a:0"});
    assert!(
        controlla_runtime::reference_reacquire::reacquire_reference(
            &identity,
            &doc,
            &[reloaded_frame]
        )
        .is_err()
    );
}
