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
    },
    FormDraft {
        fields: Vec<FormField>,
        save_without_submit: bool,
    },
    Upload {
        reference: String,
        path: String,
    },
    Download {
        reference: String,
    },
    Repeat {
        references: Vec<String>,
        action: RepeatAction,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormField {
    pub reference: String,
    pub expected_value: String,
    pub value: String,
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
        reference: String,
        path: String,
    },
    Download {
        reference: String,
    },
    Click {
        reference: String,
    },
    VerifyVisible {
        reference: String,
    },
    SaveDraft,
}

impl TaskPrimitive {
    pub fn expand(&self) -> Result<Vec<ExpandedStep>, &'static str> {
        let steps = match self {
            Self::Research { urls, selector } => {
                if urls.is_empty() || urls.len() > MAX_RESEARCH_PAGES {
                    return Err("research requires 1..32 URLs");
                }
                if urls.iter().any(|url| !valid_http_url(url)) {
                    return Err("research URLs must be bounded HTTP(S) URLs without credentials");
                }
                let selector = selector.clone().unwrap_or_else(|| "body".to_owned());
                if selector.is_empty() || selector.len() > 512 {
                    return Err("selector must be 1..512 bytes");
                }
                urls.iter()
                    .flat_map(|url| {
                        [
                            ExpandedStep::Open { url: url.clone() },
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
                    steps.push(ExpandedStep::SaveDraft);
                }
                steps
            }
            Self::Upload { reference, path } => {
                if reference.is_empty() || path.is_empty() || path.len() > 4_096 {
                    return Err("upload requires a bounded reference and path");
                }
                vec![ExpandedStep::Upload {
                    reference: reference.clone(),
                    path: path.clone(),
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
            Self::Repeat { references, action } => {
                if references.is_empty()
                    || references.len() > 64
                    || references.iter().any(String::is_empty)
                {
                    return Err("repeat requires 1..64 references");
                }
                references
                    .iter()
                    .map(|reference| match action {
                        RepeatAction::Click => ExpandedStep::Click {
                            reference: reference.clone(),
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
    if url.len() > 2_048 {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !url.contains('@')
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
    input.iter().all(|(name, value)| {
        let Some(property_schema) = properties.get(name).and_then(Value::as_object) else {
            return false;
        };
        if property_schema
            .keys()
            .any(|key| !matches!(key.as_str(), "type" | "minLength" | "maxLength"))
            || property_schema.get("type") != Some(&Value::String("string".into()))
        {
            return false;
        }
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

fn valid_descriptor(tool: &PageToolDescriptor) -> bool {
    !tool.name.is_empty() && tool.name.len() <= 128 && tool.input_schema.is_object()
}
