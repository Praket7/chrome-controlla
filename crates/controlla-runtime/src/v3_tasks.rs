//! Deterministic v3 task expansions and native page-tool routing.
//! No primitive in this module starts an autonomous reasoning loop.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_RESEARCH_PAGES: usize = 32;
pub const MAX_TASK_STEPS: usize = 128;
pub const MAX_EVIDENCE_SNIPPET_CHARS: usize = 2_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskPrimitive {
    Research {
        urls: Vec<String>,
        selector: Option<String>,
        #[serde(default)]
        tab_ids: Option<Vec<String>>,
    },
    FormDraft {
        fields: Vec<FormField>,
        save_without_submit: bool,
        #[serde(default)]
        save_control: Option<SaveControl>,
    },
    Upload {
        artifact_handle: String,
        selector: String,
        account_marker: (String, String),
    },
    Download {
        reference: String,
    },
    Repeat {
        references: Vec<String>,
        action: RepeatAction,
        #[serde(default)]
        click_outcome: Option<Value>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormField {
    pub reference: String,
    pub expected_value: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveControl {
    pub reference: String,
    pub confirmation_selector: String,
    pub confirmation_text: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatAction {
    Click,
    VerifyVisible,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExpandedStep {
    Open {
        url: String,
        tab_id: Option<String>,
    },
    Extract {
        selector: String,
        capture_url: bool,
    },
    Fill {
        reference: String,
        expected_value: String,
        value: String,
    },
    Upload {
        artifact_handle: String,
        selector: String,
        account_marker: (String, String),
    },
    Download {
        reference: String,
    },
    Click {
        reference: String,
        outcome: Value,
    },
    VerifyVisible {
        reference: String,
    },
    SaveDraft {
        control: SaveControl,
    },
}

impl TaskPrimitive {
    pub fn expand(&self) -> Result<Vec<ExpandedStep>, &'static str> {
        let steps = match self {
            Self::Research {
                urls,
                selector,
                tab_ids,
            } => {
                if urls.is_empty() || urls.len() > MAX_RESEARCH_PAGES {
                    return Err("research requires 1..32 URLs");
                }
                if urls.iter().any(|url| !valid_http_url(url)) {
                    return Err("research URLs must be bounded HTTP(S) URLs without credentials");
                }
                if tab_ids.as_ref().is_some_and(|ids| {
                    ids.len() != urls.len() || ids.iter().any(|id| id.is_empty() || id.len() > 128)
                }) {
                    return Err(
                        "research tab_ids must provide one bounded selected target per URL",
                    );
                }
                let selector = selector.clone().unwrap_or_else(|| "body".to_owned());
                if selector.is_empty() || selector.len() > 512 {
                    return Err("selector must be 1..512 bytes");
                }
                urls.iter()
                    .enumerate()
                    .flat_map(|(index, url)| {
                        [
                            ExpandedStep::Open {
                                url: url.clone(),
                                tab_id: tab_ids.as_ref().map(|ids| ids[index].clone()),
                            },
                            ExpandedStep::Extract {
                                selector: selector.clone(),
                                capture_url: true,
                            },
                        ]
                    })
                    .collect()
            }
            Self::FormDraft {
                fields,
                save_without_submit,
                save_control,
            } => {
                if fields.is_empty() || fields.len() > 64 {
                    return Err("form draft requires 1..64 fields");
                }
                let mut steps =
                    Vec::with_capacity(fields.len() + usize::from(*save_without_submit));
                for field in fields {
                    if field.reference.is_empty() || field.value.len() > 16_384 {
                        return Err("form field is invalid or oversized");
                    }
                    steps.push(ExpandedStep::Fill {
                        reference: field.reference.clone(),
                        expected_value: field.expected_value.clone(),
                        value: field.value.clone(),
                    });
                }
                if *save_without_submit {
                    let control = save_control
                        .as_ref()
                        .ok_or("save draft requires an explicit save control and confirmation")?;
                    if control.reference.is_empty()
                        || control.confirmation_selector.is_empty()
                        || control.confirmation_selector.len() > 512
                        || control.confirmation_text.is_empty()
                        || control.confirmation_text.len() > 512
                    {
                        return Err("save control or confirmation is invalid or oversized");
                    }
                    steps.push(ExpandedStep::SaveDraft {
                        control: control.clone(),
                    });
                }
                steps
            }
            Self::Upload {
                artifact_handle,
                selector,
                account_marker,
            } => {
                if artifact_handle.is_empty()
                    || artifact_handle.len() > 256
                    || artifact_handle.contains('/')
                    || artifact_handle.contains('\\')
                    || selector.is_empty()
                    || selector.len() > 512
                    || account_marker.0.is_empty()
                    || account_marker.0.len() > 512
                    || account_marker.1.is_empty()
                    || account_marker.1.len() > 512
                {
                    return Err(
                        "upload requires an opaque artifact handle, bounded file selector, and account marker",
                    );
                }
                vec![ExpandedStep::Upload {
                    artifact_handle: artifact_handle.clone(),
                    selector: selector.clone(),
                    account_marker: account_marker.clone(),
                }]
            }
            Self::Download { reference } => {
                if reference.is_empty() {
                    return Err("download requires a reference");
                }
                vec![ExpandedStep::Download {
                    reference: reference.clone(),
                }]
            }
            Self::Repeat {
                references,
                action,
                click_outcome,
            } => {
                if references.is_empty()
                    || references.len() > 64
                    || references.iter().any(String::is_empty)
                {
                    return Err("repeat requires 1..64 references");
                }
                if matches!(action, RepeatAction::Click) && click_outcome.is_none() {
                    return Err("repeat click requires an explicit bounded outcome");
                }
                references
                    .iter()
                    .map(|reference| match action {
                        RepeatAction::Click => ExpandedStep::Click {
                            reference: reference.clone(),
                            outcome: click_outcome.clone().unwrap(),
                        },
                        RepeatAction::VerifyVisible => ExpandedStep::VerifyVisible {
                            reference: reference.clone(),
                        },
                    })
                    .collect()
            }
        };
        if steps.len() > MAX_TASK_STEPS {
            return Err("expanded task exceeds 128 deterministic steps");
        }
        Ok(steps)
    }
}

fn valid_http_url(url: &str) -> bool {
    if url.len() > 2_048 || url.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let authority = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .unwrap_or_default();
    !authority.is_empty() && !authority.contains('@')
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceReceipt {
    pub url: String,
    pub snippet: String,
    pub affected_controls: Vec<String>,
    pub verified: bool,
}

impl EvidenceReceipt {
    pub fn bounded(mut self) -> Self {
        self.url.truncate(2_048);
        self.snippet = self
            .snippet
            .chars()
            .take(MAX_EVIDENCE_SNIPPET_CHARS)
            .collect();
        self.affected_controls.truncate(64);
        self
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageToolEffect {
    Read,
    Mutate,
    Consequential,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageToolDescriptor {
    pub name: String,
    pub effect: PageToolEffect,
    pub input_schema: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum PageToolRoute {
    NativeTool(String),
    SemanticUi,
}

pub fn route_page_tool(
    tools: &[PageToolDescriptor],
    requested: &str,
    mutation_authorized: bool,
) -> PageToolRoute {
    let Some(tool) = tools
        .iter()
        .find(|tool| tool.name == requested && valid_descriptor(tool))
    else {
        return PageToolRoute::SemanticUi;
    };
    if !mutation_authorized && !matches!(tool.effect, PageToolEffect::Read) {
        return PageToolRoute::SemanticUi;
    }
    PageToolRoute::NativeTool(tool.name.clone())
}

pub fn validate_page_tool_result(value: &Value, max_bytes: usize) -> bool {
    max_bytes > 0
        && max_bytes <= 256 * 1024
        && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= max_bytes)
}

/// Validates the bounded JSON Schema subset accepted for native page tools.
/// Unsupported keywords or malformed schemas fail closed to semantic UI routing.
pub fn validate_page_tool_input(schema: &Value, input: &Value) -> bool {
    let Some(schema) = schema.as_object() else {
        return false;
    };
    if schema.keys().any(|key| {
        !matches!(
            key.as_str(),
            "type" | "properties" | "required" | "additionalProperties"
        )
    }) || schema.get("type") != Some(&Value::String("object".into()))
    {
        return false;
    }
    let (Some(properties), Some(input)) = (
        schema.get("properties").and_then(Value::as_object),
        input.as_object(),
    ) else {
        return false;
    };
    let Some(required) = schema.get("required").and_then(Value::as_array) else {
        return false;
    };
    if required.iter().any(|name| {
        name.as_str()
            .is_none_or(|name| !input.contains_key(name) || !properties.contains_key(name))
    }) || schema.get("additionalProperties") != Some(&Value::Bool(false))
    {
        return false;
    }
    if !properties.values().all(valid_string_property_schema) {
        return false;
    }
    input.iter().all(|(name, value)| {
        let Some(property_schema) = properties.get(name).and_then(Value::as_object) else {
            return false;
        };
        let parse_length = |key: &str, default| match property_schema.get(key) {
            None => Some(default),
            Some(value) => value
                .as_u64()
                .and_then(|length| usize::try_from(length).ok())
                .filter(|length| *length <= 16_384),
        };
        let (Some(min), Some(max)) = (
            parse_length("minLength", 0),
            parse_length("maxLength", 16_384),
        ) else {
            return false;
        };
        let Some(value) = value.as_str() else {
            return false;
        };
        min <= max && value.chars().count() >= min && value.chars().count() <= max
    })
}

fn valid_string_property_schema(value: &Value) -> bool {
    let Some(schema) = value.as_object() else {
        return false;
    };
    if schema
        .keys()
        .any(|key| !matches!(key.as_str(), "type" | "minLength" | "maxLength"))
        || schema.get("type") != Some(&Value::String("string".into()))
    {
        return false;
    }
    let length = |key: &str, default| match schema.get(key) {
        None => Some(default),
        Some(value) => value
            .as_u64()
            .and_then(|length| usize::try_from(length).ok())
            .filter(|length| *length <= 16_384),
    };
    matches!((length("minLength", 0), length("maxLength", 16_384)), (Some(min), Some(max)) if min <= max)
}

fn valid_descriptor(tool: &PageToolDescriptor) -> bool {
    !tool.name.is_empty()
        && tool.name.len() <= 128
        && tool
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        && tool.input_schema.is_object()
}
