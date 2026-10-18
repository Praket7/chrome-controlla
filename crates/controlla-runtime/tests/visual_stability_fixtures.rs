use serde_json::{Value, json};

#[test]
fn deterministic_frame_fixture_has_one_change_then_a_quiet_window() {
    let frames = json!([
        {"digest":"frame-a","document_id":"doc-1","target_lifecycle":"active"},
        {"digest":"frame-b","document_id":"doc-1","target_lifecycle":"active"},
        {"digest":"frame-b","document_id":"doc-1","target_lifecycle":"active"},
        {"digest":"frame-b","document_id":"doc-1","target_lifecycle":"active"}
    ]);
    assert_eq!(frames.as_array().unwrap().len(), 4);
    assert_ne!(frames[0]["digest"], frames[1]["digest"]);
    assert_eq!(frames[1]["digest"], frames[2]["digest"]);
    assert_eq!(frames[2]["digest"], frames[3]["digest"]);
}

#[test]
fn deterministic_frame_fixture_can_represent_a_perpetually_changing_region() {
    let frames = json!(["frame-a", "frame-b", "frame-c", "frame-d"]);
    let digests: Vec<_> = frames
        .as_array()
        .unwrap()
        .iter()
        .map(Value::as_str)
        .collect();
    assert!(digests.windows(2).all(|pair| pair[0] != pair[1]));
}
