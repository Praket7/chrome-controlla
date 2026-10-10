use controlla_runtime::v3_tasks::{
    EvidenceReceipt, ExpandedStep, FormField, PageToolDescriptor, PageToolEffect, PageToolRoute,
    RepeatAction, SaveControl, TaskPrimitive, route_page_tool, validate_page_tool_input,
    validate_page_tool_result,
};
use serde_json::json;

#[test]
fn research_expands_without_an_autonomous_loop() {
    let task = TaskPrimitive::Research {
        urls: vec![
            "https://example.com/a".into(),
            "https://example.com/b".into(),
        ],
        selector: Some("main".into()),
        tab_ids: None,
    };
    let steps = task.expand().unwrap();
    assert_eq!(steps.len(), 4);
    assert!(matches!(steps[0], ExpandedStep::Open { .. }));
    assert!(matches!(
        steps[1],
        ExpandedStep::Extract {
            capture_url: true,
            ..
        }
    ));
}

#[test]
fn ten_page_research_is_bounded_and_keeps_each_citation_url() {
    let task = TaskPrimitive::Research {
        urls: (0..10)
            .map(|n| format!("https://example.com/{n}"))
            .collect(),
        selector: Some("article".into()),
        tab_ids: Some((0..10).map(|n| format!("tab-{n}")).collect()),
    };
    let steps = task.expand().unwrap();
    assert_eq!(steps.len(), 20);
    for (index, pair) in steps.as_chunks::<2>().0.iter().enumerate() {
        assert!(
            matches!(&pair[0], ExpandedStep::Open { tab_id: Some(id), .. } if id == &format!("tab-{index}"))
        );
        assert!(matches!(
            pair[1],
            ExpandedStep::Extract {
                capture_url: true,
                ..
            }
        ));
    }
}

#[test]
fn research_rejects_missing_host_and_mismatched_tab_list() {
    for (url, tab_ids) in [
        ("https:///missing", None),
        (
            "https://example.test/a",
            Some(vec!["tab-1".into(), "tab-2".into()]),
        ),
    ] {
        let task = TaskPrimitive::Research {
            urls: vec![url.into()],
            selector: None,
            tab_ids,
        };
        assert!(task.expand().is_err());
    }
}

#[test]
fn upload_does_not_accept_raw_filesystem_paths() {
    let task = serde_json::from_value::<TaskPrimitive>(json!({
        "kind":"upload","reference":"@c1","path":"/Users/person/private.txt"
    }));
    assert!(task.is_err());
}

#[test]
fn form_draft_can_save_without_submitting() {
    let task = TaskPrimitive::FormDraft {
        fields: vec![FormField {
            reference: "@c1".into(),
            expected_value: "".into(),
            value: "Praket".into(),
        }],
        save_without_submit: true,
        save_control: Some(SaveControl {
            reference: "@c2".into(),
            confirmation_selector: "#saved".into(),
            confirmation_text: "Saved".into(),
        }),
    };
    let steps = task.expand().unwrap();
    assert!(matches!(steps.last(), Some(ExpandedStep::SaveDraft { .. })));
    assert!(
        !steps
            .iter()
            .any(|step| matches!(step, ExpandedStep::Click { .. }))
    );
}

#[test]
fn repeated_actions_are_bounded() {
    let task = TaskPrimitive::Repeat {
        references: vec!["@c1".into(), "@c2".into()],
        action: RepeatAction::VerifyVisible,
        click_outcome: None,
    };
    assert_eq!(task.expand().unwrap().len(), 2);
    let too_many = TaskPrimitive::Repeat {
        references: (0..65).map(|i| format!("@c{i}")).collect(),
        action: RepeatAction::Click,
        click_outcome: Some(json!({"kind":"visible","selector":"#done"})),
    };
    assert!(too_many.expand().is_err());
}

#[test]
fn page_tools_precede_ui_only_when_valid_and_authorized() {
    let tools = vec![PageToolDescriptor {
        name: "search".into(),
        effect: PageToolEffect::Read,
        input_schema: json!({"type":"object"}),
    }];
    assert_eq!(
        route_page_tool(&tools, "search", false),
        PageToolRoute::NativeTool("search".into())
    );
    assert_eq!(
        route_page_tool(&tools, "missing", true),
        PageToolRoute::SemanticUi
    );
    let mutating = vec![PageToolDescriptor {
        name: "save".into(),
        effect: PageToolEffect::Mutate,
        input_schema: json!({"type":"object"}),
    }];
    assert_eq!(
        route_page_tool(&mutating, "save", false),
        PageToolRoute::SemanticUi
    );
    assert_eq!(
        route_page_tool(&mutating, "save", true),
        PageToolRoute::NativeTool("save".into())
    );
    let invalid_name = vec![PageToolDescriptor {
        name: "save();window.location='https://bad.example'".into(),
        effect: PageToolEffect::Read,
        input_schema: json!({"type":"object"}),
    }];
    assert_eq!(
        route_page_tool(
            &invalid_name,
            "save();window.location='https://bad.example'",
            true
        ),
        PageToolRoute::SemanticUi
    );
}

#[test]
fn page_tool_output_and_evidence_are_bounded() {
    assert!(validate_page_tool_result(&json!({"ok":true}), 1024));
    assert!(!validate_page_tool_result(&json!({"ok":true}), 300_000));
    let receipt = EvidenceReceipt {
        url: "u".repeat(3000),
        snippet: "x".repeat(3000),
        affected_controls: (0..100).map(|i| i.to_string()).collect(),
        verified: true,
    }
    .bounded();
    assert!(receipt.url.len() <= 2048);
    assert_eq!(receipt.snippet.chars().count(), 2000);
    assert_eq!(receipt.affected_controls.len(), 64);
}

#[test]
fn page_tool_input_is_checked_against_a_bounded_schema_subset() {
    let schema = json!({
        "type":"object",
        "properties":{"query":{"type":"string","minLength":1,"maxLength":40}},
        "required":["query"],
        "additionalProperties":false
    });
    assert!(validate_page_tool_input(
        &schema,
        &json!({"query":"school closures"})
    ));
    assert!(!validate_page_tool_input(&schema, &json!({})));
    assert!(!validate_page_tool_input(&schema, &json!({"query":3})));
    assert!(!validate_page_tool_input(
        &schema,
        &json!({"query":"x","admin":true})
    ));
    assert!(!validate_page_tool_input(
        &json!({"type":"object","anyOf":[]}),
        &json!({"query":"x"})
    ));
    assert!(!validate_page_tool_input(
        &json!({"type":"object","properties":{"query":{"type":"string","maxLength":"40"}},"required":["query"],"additionalProperties":false}),
        &json!({"query":"x"})
    ));
    assert!(!validate_page_tool_input(
        &json!({"type":"object","properties":{"query":{"type":"string"},"count":{"type":"integer"}},"required":["query"],"additionalProperties":false}),
        &json!({"query":"x"})
    ));
}
