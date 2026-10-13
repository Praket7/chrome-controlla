use controlla_runtime::widget_adapters::{
    WidgetKind, classify_widget, plan_widget, postcondition_matches,
};
use serde_json::json;

#[test]
fn adapters_match_generic_semantics_and_fall_back_on_ambiguous_shapes() {
    assert_eq!(
        classify_widget(
            &json!({"tag":"input","role":"combobox","name":"Search","expanded":"false"})
        ),
        Some(WidgetKind::SearchableCombobox)
    );
    assert_eq!(
        classify_widget(&json!({"role":"slider","name":"Volume"})),
        Some(WidgetKind::Slider)
    );
    assert_eq!(
        classify_widget(&json!({"editable_kind":"rich_text","role":"textbox","name":"Editor"})),
        Some(WidgetKind::RichText)
    );
    assert_eq!(
        classify_widget(&json!({"role":"treeitem","name":"Documents"})),
        Some(WidgetKind::TreeSelect)
    );
    assert_eq!(
        classify_widget(&json!({"tag":"input","role":"combobox","name":""})),
        None
    );
    assert_eq!(
        classify_widget(&json!({"role":"combobox","name":"Search"})),
        None
    );
    assert_eq!(
        classify_widget(&json!({"role":"control","name":"Custom"})),
        None
    );
}

#[test]
fn plans_are_bounded_and_require_exact_live_postconditions() {
    let item = json!({"tag":"input","role":"combobox","name":"Search"});
    let plan = plan_widget(&item, "Bills").unwrap();
    assert_eq!(plan.kind, WidgetKind::SearchableCombobox);
    assert_eq!(plan.commands.len(), plan.max_commands);
    assert!(postcondition_matches(&plan, "Bills", true));
    assert!(!postcondition_matches(&plan, "Bill", true));
    assert!(!postcondition_matches(&plan, "Bills", false));
    assert!(plan_widget(&item, &"x".repeat(4097)).is_none());
}

#[test]
fn adapter_matrix_declares_execution_or_fail_closed_fallback_for_each_hard_widget() {
    let matrix = [
        (
            json!({"tag":"input","control_type":"date","role":"textbox","name":"Date"}),
            WidgetKind::DatePicker,
            true,
        ),
        (
            json!({"role":"group","name":"Date range"}),
            WidgetKind::DateRange,
            true,
        ),
        (
            json!({"tag":"input","control_type":"time","role":"textbox","name":"Time"}),
            WidgetKind::TimePicker,
            true,
        ),
        (
            json!({"tag":"div","role":"combobox","multiple":true,"name":"Categories"}),
            WidgetKind::MultiSelect,
            true,
        ),
        (
            json!({"tag":"input","role":"combobox","name":"Search"}),
            WidgetKind::SearchableCombobox,
            true,
        ),
        (
            json!({"role":"slider","name":"Volume"}),
            WidgetKind::Slider,
            true,
        ),
        (
            json!({"role":"treeitem","name":"Folder"}),
            WidgetKind::TreeSelect,
            true,
        ),
        (
            json!({"role":"textbox","editable_kind":"rich_text","name":"Editor"}),
            WidgetKind::RichText,
            true,
        ),
        (
            json!({"tag":"input","control_type":"file","role":"button","name":"Choose file"}),
            WidgetKind::FileChooser,
            false,
        ),
        (
            json!({"role":"button","name":"More","expanded":false}),
            WidgetKind::Disclosure,
            true,
        ),
        (
            json!({"role":"tab","name":"Details"}),
            WidgetKind::TabSet,
            true,
        ),
        (
            json!({"role":"button","name":"More","hover_reveals":true}),
            WidgetKind::HoverReveal,
            false,
        ),
        (
            json!({"role":"application","name":"Map","widget_kind":"map"}),
            WidgetKind::Map,
            false,
        ),
        (
            json!({"role":"listitem","name":"Card","draggable":true}),
            WidgetKind::DragDrop,
            false,
        ),
    ];
    for (item, kind, executable) in matrix {
        assert_eq!(classify_widget(&item), Some(kind), "{item}");
        let desired = if kind == WidgetKind::DateRange {
            "2026-01-01/2026-01-02"
        } else {
            "target"
        };
        assert_eq!(
            plan_widget(&item, desired).is_some(),
            executable,
            "{kind:?}"
        );
    }
}

#[test]
fn unsupported_hard_widgets_have_named_fail_closed_fallbacks() {
    for item in [
        json!({"role":"button","name":"More","hover_reveals":true}),
        json!({"role":"application","name":"Map","widget_kind":"map"}),
        json!({"role":"listitem","name":"Card","draggable":true}),
    ] {
        let adapter = controlla_runtime::widget_adapters::resolve_widget(&item).unwrap();
        assert!(!adapter.fallback.is_empty());
        assert!(plan_widget(&item, "target").is_none());
    }
}

#[test]
fn date_range_adapter_requires_two_ordered_iso_dates() {
    let item =
        json!({"reference":"@c1","role":"group","name":"Date range","widget_kind":"date_range"});
    let plan =
        controlla_runtime::widget_adapters::plan_widget(&item, "2026-01-02/2026-01-31").unwrap();
    assert_eq!(
        plan.kind,
        controlla_runtime::widget_adapters::WidgetKind::DateRange
    );
    assert_eq!(plan.expected_postcondition, "2026-01-02/2026-01-31");
    assert!(
        controlla_runtime::widget_adapters::plan_widget(&item, "2026-02-01/2026-01-31").is_none()
    );
    assert!(
        controlla_runtime::widget_adapters::plan_widget(&item, "2026-1-2/2026-01-31").is_none()
    );
    assert!(
        controlla_runtime::widget_adapters::plan_widget(&item, "2026-02-01/2026-02-31").is_none()
    );
}

#[test]
fn disclosure_and_tabs_use_fresh_action_outcome_as_their_postcondition() {
    use controlla_runtime::widget_adapters::{WidgetKind, uses_action_outcome_verification};
    assert!(uses_action_outcome_verification(WidgetKind::Disclosure));
    assert!(uses_action_outcome_verification(WidgetKind::TabSet));
    assert!(!uses_action_outcome_verification(WidgetKind::MultiSelect));
}
