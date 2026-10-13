//! Conservative generic widget recognition. A match is a routing hint; the
//! normal semantic target guard and fresh postcondition remain mandatory.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    DatePicker,
    DateRange,
    TimePicker,
    MultiSelect,
    SearchableCombobox,
    Slider,
    TreeSelect,
    RichText,
    FileChooser,
    Disclosure,
    TabSet,
    HoverReveal,
    Map,
    DragDrop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WidgetAdapter {
    pub kind: WidgetKind,
    pub live_postcondition: &'static str,
    pub fallback: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WidgetCommand {
    Focus,
    TypeText,
    ClickOption,
    SetNativeValue,
    ReadSelected,
    ReadValue,
    SetRangeStart,
    SetRangeEnd,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WidgetPlan {
    pub kind: WidgetKind,
    pub commands: Vec<WidgetCommand>,
    pub expected_postcondition: String,
    pub max_commands: usize,
}

/// Build a small generic sequence from observed widget semantics. The runtime
/// must route every command through its existing fresh-reference guard.
pub fn plan_widget(item: &Value, desired: &str) -> Option<WidgetPlan> {
    if desired.is_empty() || desired.len() > 4096 {
        return None;
    }
    let adapter = resolve_widget(item)?;
    let (commands, expected) = match adapter.kind {
        WidgetKind::DatePicker | WidgetKind::TimePicker | WidgetKind::RichText => (
            vec![
                WidgetCommand::Focus,
                WidgetCommand::SetNativeValue,
                WidgetCommand::ReadValue,
            ],
            desired.to_owned(),
        ),
        WidgetKind::SearchableCombobox => (
            vec![
                WidgetCommand::Focus,
                WidgetCommand::TypeText,
                WidgetCommand::ClickOption,
                WidgetCommand::ReadSelected,
            ],
            desired.to_owned(),
        ),
        WidgetKind::MultiSelect
        | WidgetKind::TreeSelect
        | WidgetKind::Disclosure
        | WidgetKind::TabSet => (
            vec![WidgetCommand::ClickOption, WidgetCommand::ReadSelected],
            desired.to_owned(),
        ),
        WidgetKind::Slider => (
            vec![WidgetCommand::SetNativeValue, WidgetCommand::ReadValue],
            desired.to_owned(),
        ),
        WidgetKind::DateRange => {
            parse_date_range(desired)?;
            (
                vec![
                    WidgetCommand::SetRangeStart,
                    WidgetCommand::SetRangeEnd,
                    WidgetCommand::ReadValue,
                ],
                desired.to_owned(),
            )
        }
        WidgetKind::FileChooser => return None,
        WidgetKind::HoverReveal | WidgetKind::Map | WidgetKind::DragDrop => return None,
    };
    let max_commands = commands.len();
    Some(WidgetPlan {
        kind: adapter.kind,
        max_commands,
        commands,
        expected_postcondition: expected,
    })
}

pub fn parse_date_range(value: &str) -> Option<(&str, &str)> {
    let (start, end) = value.split_once('/')?;
    let valid = |date: &str| {
        let bytes = date.as_bytes();
        if !(bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(i, byte)| i == 4 || i == 7 || byte.is_ascii_digit()))
        {
            return false;
        }
        let year = date[..4].parse::<u16>().ok().unwrap_or(0);
        let month = date[5..7].parse::<u8>().ok().unwrap_or(0);
        let day = date[8..10].parse::<u8>().ok().unwrap_or(0);
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return false,
        };
        year != 0 && day >= 1 && day <= days
    };
    (valid(start) && valid(end) && start <= end).then_some((start, end))
}

pub fn postcondition_matches(
    plan: &WidgetPlan,
    observed_value: &str,
    exact_selection: bool,
) -> bool {
    exact_selection && observed_value == plan.expected_postcondition
}

/// Containers with explicit `expanded`/`selected` outcomes are verified by the
/// action's fresh semantic readback, not by their own text/value.
pub fn uses_action_outcome_verification(kind: WidgetKind) -> bool {
    matches!(kind, WidgetKind::Disclosure | WidgetKind::TabSet)
}

const ADAPTERS: &[WidgetAdapter] = &[
    WidgetAdapter {
        kind: WidgetKind::DatePicker,
        live_postcondition: "exact selected date/value",
        fallback: "semantic or targeted visual inspection",
    },
    WidgetAdapter {
        kind: WidgetKind::DateRange,
        live_postcondition: "both exact range endpoints",
        fallback: "require unique scoped start date and end date textboxes",
    },
    WidgetAdapter {
        kind: WidgetKind::TimePicker,
        live_postcondition: "exact selected time/value",
        fallback: "semantic or targeted visual inspection",
    },
    WidgetAdapter {
        kind: WidgetKind::MultiSelect,
        live_postcondition: "selected option set",
        fallback: "semantic option inspection",
    },
    WidgetAdapter {
        kind: WidgetKind::SearchableCombobox,
        live_postcondition: "unique selected option and collapsed/expanded state",
        fallback: "semantic option inspection",
    },
    WidgetAdapter {
        kind: WidgetKind::Slider,
        live_postcondition: "live value and aria-valuenow",
        fallback: "targeted visual inspection",
    },
    WidgetAdapter {
        kind: WidgetKind::TreeSelect,
        live_postcondition: "selected tree node and checked state",
        fallback: "semantic subtree inspection",
    },
    WidgetAdapter {
        kind: WidgetKind::RichText,
        live_postcondition: "fresh editor text readback",
        fallback: "verified rich-text typing",
    },
    WidgetAdapter {
        kind: WidgetKind::FileChooser,
        live_postcondition: "application acceptance after observed chooser event",
        fallback: "use the guarded workflow upload adapter with artifact handle and acceptance predicate",
    },
    WidgetAdapter {
        kind: WidgetKind::Disclosure,
        live_postcondition: "expanded state",
        fallback: "semantic click with explicit outcome",
    },
    WidgetAdapter {
        kind: WidgetKind::TabSet,
        live_postcondition: "selected tab and associated panel",
        fallback: "semantic click and fresh panel snapshot",
    },
    WidgetAdapter {
        kind: WidgetKind::HoverReveal,
        live_postcondition: "revealed control and exact target identity",
        fallback: "targeted visual or semantic inspection; provider hover action is not available",
    },
    WidgetAdapter {
        kind: WidgetKind::Map,
        live_postcondition: "exact map viewport or selected feature state",
        fallback: "use a site-specific qualified map capability or targeted visual reasoning",
    },
    WidgetAdapter {
        kind: WidgetKind::DragDrop,
        live_postcondition: "source removal and exact destination membership",
        fallback: "require a provider-supported guarded drag action and explicit destination verification",
    },
];

pub fn resolve_widget(item: &Value) -> Option<&'static WidgetAdapter> {
    let role = item.get("role")?.as_str()?;
    let name = item.get("name")?.as_str()?;
    if name.trim().is_empty() {
        return None;
    }
    let tag = item.get("tag").and_then(Value::as_str).unwrap_or_default();
    let declared_kind = item
        .get("widget_kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let control_type = item
        .get("control_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let kind = if declared_kind == "date_range"
        || (role == "group" && name.to_ascii_lowercase().contains("date range"))
    {
        WidgetKind::DateRange
    } else if control_type == "file" {
        WidgetKind::FileChooser
    } else if item.get("editable_kind").and_then(Value::as_str) == Some("rich_text") {
        WidgetKind::RichText
    } else if role == "slider" || control_type == "range" {
        WidgetKind::Slider
    } else if role == "treeitem" {
        WidgetKind::TreeSelect
    } else if role == "tab" {
        WidgetKind::TabSet
    } else if role == "combobox" && item.get("multiple").and_then(Value::as_bool) == Some(true) {
        WidgetKind::MultiSelect
    } else if role == "combobox" && tag == "input" {
        WidgetKind::SearchableCombobox
    } else if control_type == "date" {
        WidgetKind::DatePicker
    } else if control_type == "time" {
        WidgetKind::TimePicker
    } else if role == "button" && item.get("expanded").is_some() {
        WidgetKind::Disclosure
    } else if item.get("hover_reveals").and_then(Value::as_bool) == Some(true)
        || item.get("data_hover_reveals").and_then(Value::as_bool) == Some(true)
    {
        WidgetKind::HoverReveal
    } else if item.get("widget_kind").and_then(Value::as_str) == Some("map") {
        WidgetKind::Map
    } else if item.get("draggable").and_then(Value::as_bool) == Some(true)
        || item.get("widget_kind").and_then(Value::as_str) == Some("drag_drop")
    {
        WidgetKind::DragDrop
    } else {
        return None;
    };
    ADAPTERS.iter().find(|adapter| adapter.kind == kind)
}

pub fn classify_widget(item: &Value) -> Option<WidgetKind> {
    resolve_widget(item).map(|adapter| adapter.kind)
}
