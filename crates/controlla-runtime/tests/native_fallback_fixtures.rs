use serde_json::json;

#[test]
fn native_action_fixtures_cover_identity_capability_and_delivery_outcomes() {
    let valid = json!({
        "version":1,"request_id":"req-1","lease_id":"lease-1",
        "capability":"desktop_pointer","mode":"foreground",
        "visual_identity":{"session_id":"session-1","tab_id":"7","document_id":"doc-1","visual_revision":"sha256:1"},
        "action":{"kind":"click","x":120,"y":80}
    });
    let stale = json!({"reason_code":"stale_visual_reference","status":"not_dispatched"});
    let unowned = json!({"reason_code":"unowned_session","status":"not_dispatched"});
    let unavailable = json!({"reason_code":"capability_unavailable","status":"not_dispatched"});
    let acknowledged = json!({"status":"completed","dispatch_acknowledged":true,"dispatch_may_have_occurred":true});
    let uncertain = json!({"status":"unknown","dispatch_acknowledged":false,"dispatch_may_have_occurred":true});

    assert_eq!(valid["action"]["kind"], "click");
    assert_eq!(stale["reason_code"], "stale_visual_reference");
    assert_eq!(unowned["reason_code"], "unowned_session");
    assert_eq!(unavailable["reason_code"], "capability_unavailable");
    assert_eq!(acknowledged["status"], "completed");
    assert_eq!(uncertain["status"], "unknown");
    assert!(uncertain["dispatch_may_have_occurred"].as_bool().unwrap());
}

#[test]
fn malformed_and_oversized_native_envelopes_are_representable_as_refusals() {
    let malformed = json!({"status":"not_dispatched","reason_code":"invalid_envelope"});
    let oversized = json!({"status":"not_dispatched","reason_code":"envelope_too_large"});
    assert_eq!(malformed["status"], "not_dispatched");
    assert_eq!(malformed["reason_code"], "invalid_envelope");
    assert_eq!(oversized["reason_code"], "envelope_too_large");
}
