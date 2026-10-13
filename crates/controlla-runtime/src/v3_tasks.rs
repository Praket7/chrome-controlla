//! Deterministic v3 task expansions and native page-tool routing.
//! No primitive in this module starts an autonomous reasoning loop.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_RESEARCH_PAGES: usize = 32;
pub const MAX_TASK_STEPS: usize = 128;
pub const MAX_EVIDENCE_SNIPPET_CHARS: usize = 2_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskPrimitive {
    Research {
        urls: Vec<String>,
        selector: Option<String>,
        /// User/task intent used to rank observations and verify read results.
        #[serde(default)]
        goal: Option<String>,
        #[serde(default)]
        tab_ids: Option<Vec<String>>,
    },
    FormDraft {
        fields: Vec<FormField>,
        save_without_submit: bool,
        #[serde(default)]
        save_control: Option<SaveControl>,
    },
    /// Bounded reusable mutation workflow. Every click carries an explicit
    /// runtime-verifiable outcome; only fills and clicks are admitted.
    Mutation {
        steps: Vec<MutationStep>,
    },
    Upload {
        artifact_handle: String,
        selector: Option<String>,
        trigger_reference: Option<String>,
        acceptance: Option<Value>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct FormField {
    pub reference: String,
    pub expected_value: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveControl {
    pub reference: String,
    pub confirmation_selector: String,
    pub confirmation_text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationStep {
    Fill {
        reference: String,
        expected_value: String,
        value: String,
    },
    Click {
        reference: String,
        outcome: Value,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
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
        selector: Option<String>,
        trigger_reference: Option<String>,
        acceptance: Option<Value>,
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
    /// Stable opaque key for task-shape lookup. Research URLs are bound at
    /// replay time by exact site scope and live structure, allowing safe
    /// same-site input variation without persisting a user's target URL.
    pub fn intent_fingerprint(&self) -> String {
        let mut canonical = serde_json::to_value(self).expect("task primitive serializes");
        if let Self::Research { .. } = self
            && let Some(object) = canonical.as_object_mut()
        {
            object.remove("tab_ids");
            object.remove("urls");
            object.remove("goal");
        }
        if let Self::FormDraft {
            fields,
            save_without_submit,
            save_control,
        } = self
        {
            // Field references and values are run inputs. The reusable skill
            // shape is the bounded field count plus whether an external save
            // action was requested; evidence_fingerprint retains the values.
            canonical = serde_json::json!({
                "kind":"form_draft",
                "field_count":fields.len(),
                "save_without_submit":save_without_submit,
                "save_confirmation":save_control.as_ref().map(|control| serde_json::json!({
                    "selector":control.confirmation_selector,
                    "text":control.confirmation_text,
                })),
            });
        }
        if let Self::Repeat {
            references,
            action,
            click_outcome,
        } = self
        {
            canonical = serde_json::json!({
                "kind":"repeat",
                "count":references.len(),
                "action":action,
                "click_outcome":click_outcome,
            });
        }
        if let Self::Mutation { steps } = self {
            canonical = serde_json::json!({
                "kind":"mutation",
                "steps":steps.iter().enumerate().map(|(index, step)| match step {
                    MutationStep::Fill { .. } => serde_json::json!({"action":"fill","slot":index}),
                    MutationStep::Click { outcome, .. } => serde_json::json!({"action":"click","slot":index,"outcome":outcome}),
                }).collect::<Vec<_>>(),
            });
        }
        use sha2::Digest as _;
        sha2::Sha256::digest(serde_json::to_vec(&canonical).expect("JSON value serializes"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Evidence history is narrower than the reusable skill shape: retain the
    /// URL and goal (but not ephemeral tab IDs) so distinct reads never share
    /// prior task evidence merely because their workflow shape matches.
    pub fn evidence_fingerprint(&self) -> String {
        let mut canonical = serde_json::to_value(self).expect("task primitive serializes");
        if let Some(object) = canonical.as_object_mut() {
            object.remove("tab_ids");
        }
        use sha2::Digest as _;
        sha2::Sha256::digest(serde_json::to_vec(&canonical).expect("JSON value serializes"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn expand(&self) -> Result<Vec<ExpandedStep>, &'static str> {
        let steps = match self {
            Self::Research {
                urls,
                selector,
                goal,
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
                if goal
                    .as_ref()
                    .is_some_and(|goal| goal.trim().is_empty() || goal.len() > 512)
                {
                    return Err("research goal must be 1..512 bytes when provided");
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
                trigger_reference,
                acceptance,
                account_marker,
            } => {
                let exactly_one_target = selector.is_some() ^ trigger_reference.is_some();
                let valid_selector = selector
                    .as_ref()
                    .is_none_or(|selector| !selector.is_empty() && selector.len() <= 512);
                let valid_reference = trigger_reference
                    .as_ref()
                    .is_none_or(|reference| !reference.is_empty() && reference.len() <= 1024);
                let valid_acceptance = acceptance.as_ref().is_none_or(|value| {
                    serde_json::to_vec(value).is_ok_and(|encoded| encoded.len() <= 8192)
                });
                if artifact_handle.is_empty()
                    || artifact_handle.len() > 256
                    || artifact_handle.contains('/')
                    || artifact_handle.contains('\\')
                    || !exactly_one_target
                    || !valid_selector
                    || !valid_reference
                    || !valid_acceptance
                    || account_marker.0.is_empty()
                    || account_marker.0.len() > 512
                    || account_marker.1.is_empty()
                    || account_marker.1.len() > 512
                {
                    return Err(
                        "upload requires an opaque artifact handle, exactly one bounded file selector or trigger reference, and account marker",
                    );
                }
                vec![ExpandedStep::Upload {
                    artifact_handle: artifact_handle.clone(),
                    selector: selector.clone(),
                    trigger_reference: trigger_reference.clone(),
                    acceptance: acceptance.clone(),
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
            Self::Mutation { steps } => {
                if steps.is_empty() || steps.len() > 32 {
                    return Err("mutation requires 1..32 bounded steps");
                }
                steps.iter().map(|step| match step {
                    MutationStep::Fill { reference, expected_value, value } => {
                        if !reference.starts_with("@c") || expected_value.len() > 16_384 || value.len() > 16_384 {
                            return Err("mutation fill requires a fresh semantic ref and bounded values");
                        }
                        Ok(ExpandedStep::Fill { reference: reference.clone(), expected_value: expected_value.clone(), value: value.clone() })
                    }
                    MutationStep::Click { reference, outcome } => {
                        if !reference.starts_with("@c") || outcome.as_object().is_none() || serde_json::to_vec(outcome).map_or(true, |bytes| bytes.len() > 8_192) {
                            return Err("mutation click requires a fresh semantic ref and bounded explicit outcome");
                        }
                        Ok(ExpandedStep::Click { reference: reference.clone(), outcome: outcome.clone() })
                    }
                }).collect::<Result<Vec<_>, _>>()?
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

#[cfg(test)]
mod task_expansion_tests {
    use super::{ExpandedStep, FormField, MutationStep, RepeatAction, SaveControl, TaskPrimitive};
    use serde_json::Value;

    #[test]
    fn research_expansion_keeps_the_explicit_target_with_its_url() {
        let task = TaskPrimitive::Research {
            urls: vec!["https://example.test/story".into()],
            selector: Some("article".into()),
            goal: Some("latest story".into()),
            tab_ids: Some(vec!["tab-7".into()]),
        };
        let steps = task.expand().unwrap();
        assert!(matches!(&steps[0], ExpandedStep::Open { tab_id: Some(id), .. } if id == "tab-7"));
        assert!(
            matches!(&steps[1], ExpandedStep::Extract { selector, .. } if selector == "article")
        );
    }

    #[test]
    fn research_intent_fingerprint_ignores_target_url_and_tab_id_for_scoped_replay() {
        let task = |tab_id: &str| TaskPrimitive::Research {
            urls: vec!["https://example.test/story".into()],
            selector: Some("article".into()),
            goal: None,
            tab_ids: Some(vec![tab_id.into()]),
        };
        assert_eq!(
            task("1").intent_fingerprint(),
            task("2").intent_fingerprint()
        );
        assert_eq!(
            task("1").intent_fingerprint(),
            TaskPrimitive::Research {
                urls: vec!["https://example.test/other".into()],
                selector: Some("article".into()),
                goal: None,
                tab_ids: Some(vec!["1".into()]),
            }
            .intent_fingerprint()
        );
        let same_shape_different_goal = TaskPrimitive::Research {
            urls: vec!["https://example.test/search?q=another".into()],
            selector: Some("article".into()),
            goal: Some("find a different current topic".into()),
            tab_ids: None,
        };
        assert_eq!(
            task("1").intent_fingerprint(),
            same_shape_different_goal.intent_fingerprint()
        );
        assert_ne!(
            task("1").evidence_fingerprint(),
            same_shape_different_goal.evidence_fingerprint()
        );
    }

    #[test]
    fn form_draft_skill_shape_ignores_values_and_ephemeral_references_but_evidence_does_not() {
        let make = |reference: &str, expected: &str, value: &str| TaskPrimitive::FormDraft {
            fields: vec![FormField {
                reference: reference.into(),
                expected_value: expected.into(),
                value: value.into(),
            }],
            save_without_submit: false,
            save_control: None,
        };
        let first = make("@c1", "old", "alpha");
        let second = make("@c9", "changed", "beta");
        assert_eq!(first.intent_fingerprint(), second.intent_fingerprint());
        assert_ne!(first.evidence_fingerprint(), second.evidence_fingerprint());
    }

    #[test]
    fn form_draft_intent_binds_the_explicit_save_confirmation_contract() {
        let make = |selector: &str, text: &str| TaskPrimitive::FormDraft {
            fields: vec![FormField {
                reference: "@c1".into(),
                expected_value: "old".into(),
                value: "new".into(),
            }],
            save_without_submit: true,
            save_control: Some(SaveControl {
                reference: "@c2".into(),
                confirmation_selector: selector.into(),
                confirmation_text: text.into(),
            }),
        };
        assert_eq!(
            make("#status", "Saved").intent_fingerprint(),
            make("#status", "Saved").intent_fingerprint()
        );
        assert_ne!(
            make("#status", "Saved").intent_fingerprint(),
            make("#status", "Published").intent_fingerprint()
        );
        assert_ne!(
            make("#status", "Saved").intent_fingerprint(),
            make("#notice", "Saved").intent_fingerprint()
        );
    }

    #[test]
    fn mutation_intent_rebinds_current_refs_and_values_but_keeps_action_contract() {
        let task = |reference: &str, expected: &str, value: &str| TaskPrimitive::Mutation {
            steps: vec![
                MutationStep::Fill {
                    reference: reference.into(),
                    expected_value: expected.into(),
                    value: value.into(),
                },
                MutationStep::Click {
                    reference: "@c-save".into(),
                    outcome: serde_json::json!({"kind":"text","selector":"#saved","text":"Draft saved"}),
                },
            ],
        };
        assert_eq!(
            task("@c1", "old", "one").intent_fingerprint(),
            task("@c9", "other", "two").intent_fingerprint()
        );
        let TaskPrimitive::Mutation { steps } = task("@c1", "old", "one") else {
            unreachable!()
        };
        assert!(matches!(
            TaskPrimitive::Mutation { steps }
                .expand()
                .unwrap()
                .as_slice(),
            [ExpandedStep::Fill { .. }, ExpandedStep::Click { .. }]
        ));
        let changed = TaskPrimitive::Mutation {
            steps: vec![MutationStep::Click {
                reference: "@c-save".into(),
                outcome: serde_json::json!({"kind":"text","selector":"#saved","text":"Different"}),
            }],
        };
        let original = task("@c1", "old", "one");
        assert_ne!(original.intent_fingerprint(), changed.intent_fingerprint());
    }

    #[test]
    fn repeat_skill_shape_ignores_ephemeral_refs_but_keeps_action_and_outcome() {
        let make = |reference: &str, outcome: Value| TaskPrimitive::Repeat {
            references: vec![reference.to_owned()],
            action: RepeatAction::Click,
            click_outcome: Some(outcome),
        };
        let first = make("@c1", serde_json::json!({"kind":"selected"}));
        let second = make("@c9", serde_json::json!({"kind":"selected"}));
        assert_eq!(first.intent_fingerprint(), second.intent_fingerprint());
        assert_ne!(first.evidence_fingerprint(), second.evidence_fingerprint());
        assert_ne!(
            first.intent_fingerprint(),
            make("@c2", serde_json::json!({"kind":"expanded"})).intent_fingerprint()
        );
    }
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
