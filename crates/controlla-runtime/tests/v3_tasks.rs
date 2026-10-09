use controlla_runtime::v3_tasks::{
    EvidenceReceipt, ExpandedStep, FormField, PageToolDescriptor, PageToolEffect, PageToolRoute,
    RepeatAction, TaskPrimitive, route_page_tool, validate_page_tool_result,
};
use serde_json::json;

#[test]
fn research_expands_without_an_autonomous_loop() {
    let task = TaskPrimitive::Research {
        urls: vec!["https://example.com/a".into(), "https://example.com/b".into()],
        selector: Some("main".into()),
    };
    let steps = task.expand().unwrap();
    assert_eq!(steps.len(), 4);
    assert!(matches!(steps[0], ExpandedStep::Open { .. }));
    assert!(matches!(steps[1], ExpandedStep::Extract { capture_url: true, .. }));
}

#[test]
fn form_draft_can_save_without_submitting() {
    let task = TaskPrimitive::FormDraft {
        fields: vec![FormField { reference: "@c1".into(), expected_value: "".into(), value: "Praket".into() }],
        save_without_submit: true,
    };
    let steps = task.expand().unwrap();
    assert!(matches!(steps.last(), Some(ExpandedStep::SaveDraft)));
    assert!(!steps.iter().any(|step| matches!(step, ExpandedStep::Click { .. })));
}

#[test]
fn repeated_actions_are_bounded() {
    let task = TaskPrimitive::Repeat { references: vec!["@c1".into(), "@c2".into()], action: RepeatAction::VerifyVisible };
    assert_eq!(task.expand().unwrap().len(), 2);
    let too_many = TaskPrimitive::Repeat { references: (0..65).map(|i| format!("@c{i}")).collect(), action: RepeatAction::Click };
    assert!(too_many.expand().is_err());
}

#[test]
fn page_tools_precede_ui_only_when_valid_and_authorized() {
    let tools = vec![PageToolDescriptor { name: "search".into(), effect: PageToolEffect::Read, input_schema: json!({"type":"object"}) }];
    assert_eq!(route_page_tool(&tools, "search", false), PageToolRoute::NativeTool("search".into()));
    assert_eq!(route_page_tool(&tools, "missing", true), PageToolRoute::SemanticUi);
    let mutating = vec![PageToolDescriptor { name: "save".into(), effect: PageToolEffect::Mutate, input_schema: json!({"type":"object"}) }];
    assert_eq!(route_page_tool(&mutating, "save", false), PageToolRoute::SemanticUi);
    assert_eq!(route_page_tool(&mutating, "save", true), PageToolRoute::NativeTool("save".into()));
}

#[test]
fn page_tool_output_and_evidence_are_bounded() {
    assert!(validate_page_tool_result(&json!({"ok":true}), 1024));
    assert!(!validate_page_tool_result(&json!({"ok":true}), 300_000));
    let receipt = EvidenceReceipt { url: "u".repeat(3000), snippet: "x".repeat(3000), affected_controls: (0..100).map(|i| i.to_string()).collect(), verified: true }.bounded();
    assert!(receipt.url.len() <= 2048);
    assert_eq!(receipt.snippet.chars().count(), 2000);
    assert_eq!(receipt.affected_controls.len(), 64);
}
