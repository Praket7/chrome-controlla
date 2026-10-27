use controlla_runtime::v3_tasks::{
    EvidenceReceipt, ExpandedStep, FormField, PageToolDescriptor, PageToolEffect, PageToolRoute,
    RepeatAction, SaveControl, TaskPrimitive, route_page_tool, validate_page_tool_input,
    validate_page_tool_result,
};
use serde_json::json;

#[test]
fn mutation_can_verify_visibility_without_becoming_resumable() {
    let task: TaskPrimitive = serde_json::from_value(json!({"kind":"mutation","steps":[
        {"action":"verify_visible","reference":"@c1"}
    ]})).unwrap();
    assert!(matches!(&task.expand().unwrap()[0], ExpandedStep::VerifyVisible { reference } if reference == "@c1"));
    assert!(!task.is_resumable_fill_only());
    assert!(!task.is_resumable_with_postconditions());
    let invalid: TaskPrimitive = serde_json::from_value(json!({"kind":"mutation","steps":[
        {"action":"verify_visible","reference":"#selector"}
    ]})).unwrap();
    assert!(invalid.expand().is_err());
}

#[test]
fn research_expands_without_an_autonomous_loop() {
    let task = TaskPrimitive::Research {
        urls: vec![
            "https://example.com/a".into(),
            "https://example.com/b".into(),
        ],
        selector: Some("main".into()),
        goal: None,
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
        goal: None,
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
            goal: None,
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
fn upload_requires_one_bounded_target_and_acceptance_is_explicit() {
    let selector = serde_json::from_value::<TaskPrimitive>(json!({
        "kind":"upload","artifact_handle":"opaque-handle","selector":"input[type=file]",
        "account_marker":["#account","school@example.test"]
    }))
    .unwrap();
    assert!(matches!(
        selector.expand().unwrap().as_slice(),
        [ExpandedStep::Upload {
            selector: Some(_),
            trigger_reference: None,
            ..
        }]
    ));
    let trigger = serde_json::from_value::<TaskPrimitive>(json!({
        "kind":"upload","artifact_handle":"opaque-handle","trigger_reference":"@c4",
        "acceptance":{"kind":"text","reference":"@c5","equals":"attached"},
        "persisted":{"kind":"text","reference":"@c6","equals":"saved"},
        "account_marker":["#account","school@example.test"]
    }))
    .unwrap();
    assert!(matches!(
        trigger.expand().unwrap().as_slice(),
        [ExpandedStep::Upload {
            selector: None,
            trigger_reference: Some(_),
            acceptance: Some(_),
            persisted: Some(_),
            ..
        }]
    ));
    let frame_trigger = serde_json::from_value::<TaskPrimitive>(json!({
        "kind":"upload","artifact_handle":"opaque-handle",
        "trigger_reference":"@f:32C95EE3B095097A0638B9FBD18DC854:952ABCB690EC70ABEA7A73F866B1F49D:5673",
        "account_marker":["#account","school@example.test"]
    })).unwrap();
    assert!(
        frame_trigger.expand().is_ok(),
        "frame-bound Browse references must survive task validation"
    );
    for target in [
        json!({}),
        json!({"selector":"#file","trigger_reference":"@c1"}),
        json!({"trigger_reference":""}),
    ] {
        let mut args = json!({"kind":"upload","artifact_handle":"opaque-handle","account_marker":["#account","school@example.test"]});
        args.as_object_mut()
            .unwrap()
            .extend(target.as_object().unwrap().clone());
        assert!(serde_json::from_value::<TaskPrimitive>(args.clone()).is_ok());
        assert!(
            serde_json::from_value::<TaskPrimitive>(args)
                .unwrap()
                .expand()
                .is_err()
        );
    }
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
fn five_field_form_is_one_bounded_task_with_per_field_preconditions() {
    let task = TaskPrimitive::FormDraft {
        fields: (0..5)
            .map(|index| FormField {
                reference: format!("@c{index}"),
                expected_value: String::new(),
                value: format!("value-{index}"),
            })
            .collect(),
        save_without_submit: false,
        save_control: None,
    };
    let steps = task.expand().unwrap();
    assert_eq!(steps.len(), 5);
    for (index, step) in steps.iter().enumerate() {
        assert!(
            matches!(step, ExpandedStep::Fill { reference, expected_value, value }
            if reference == &format!("@c{index}") && expected_value.is_empty() && value == &format!("value-{index}"))
        );
    }
}

#[test]
fn download_upload_is_one_bounded_task_with_explicit_artifact_dataflow() {
    let task: TaskPrimitive = serde_json::from_value(json!({
        "kind":"download_upload",
        "download_reference":"@c1",
        "source":"page_fetch",
        "trigger_reference":"@c2",
        "acceptance":{"kind":"text","reference":"@c3","equals":"Accepted"},
        "persisted":{"kind":"text","reference":"@c4","equals":"Saved"},
        "account_marker":["#account","fixture-account"]
    }))
    .unwrap();
    let steps = task.expand().unwrap();
    assert!(matches!(steps.as_slice(), [
        ExpandedStep::Download { reference, .. },
        ExpandedStep::UploadPreviousDownload { trigger_reference: Some(trigger), acceptance: Some(_), persisted: Some(_), .. }
    ] if reference == "@c1" && trigger == "@c2"));

    let invalid: TaskPrimitive = serde_json::from_value(json!({
        "kind":"download_upload", "download_reference":"@c1", "source":"page_fetch",
        "trigger_reference":"@c2", "selector":"#file",
        "account_marker":["#account","fixture-account"]
    }))
    .unwrap();
    assert!(
        invalid.expand().is_err(),
        "exactly one upload target is required"
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
