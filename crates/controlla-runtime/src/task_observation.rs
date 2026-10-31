//! Task-conditioned observation filters.
//!
//! The filter is derived only from the bounded task primitive, never from page
//! text. It keeps controls a task must see (roles) ahead of query ranking and
//! gives a deterministic receipt explaining why the observation was shaped.

use crate::v3_tasks::{MutationStep, RepeatAction, TaskPrimitive};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const MAX_TASK_FILTER_ROLES: usize = 12;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskObservationFilter {
    pub query: Option<String>,
    pub required_roles: Vec<String>,
    pub exhaustive: bool,
    pub source: &'static str,
}

impl TaskObservationFilter {
    pub fn for_task(task: &TaskPrimitive) -> Self {
        match task {
            TaskPrimitive::Research { goal, .. } => Self {
                query: goal
                    .as_deref()
                    .map(str::trim)
                    .filter(|goal| !goal.is_empty())
                    .map(|goal| goal.chars().take(512).collect()),
                required_roles: Vec::new(),
                exhaustive: false,
                source: "research_goal",
            },
            TaskPrimitive::FormDraft { .. } => Self::roles(
                &[
                    "textbox", "combobox", "checkbox", "radio", "switch", "button",
                ],
                None,
                "form_draft_controls",
            ),
            TaskPrimitive::Mutation { steps } => {
                let mut roles = BTreeSet::new();
                let mut query = None;
                for step in steps {
                    match step {
                        MutationStep::Fill { .. } => {
                            roles.insert("textbox");
                            roles.insert("combobox");
                        }
                        MutationStep::Click { outcome, .. } => {
                            roles.extend([
                                "button", "link", "tab", "menuitem", "option", "checkbox", "radio",
                            ]);
                            query = query.or_else(|| outcome_text(outcome));
                        }
                        MutationStep::VerifyVisible { .. } => {
                            roles.extend(["button", "link", "alert", "status"]);
                        }
                    }
                }
                Self {
                    query,
                    required_roles: bounded_roles(roles),
                    exhaustive: false,
                    source: "mutation_step_targets",
                }
            }
            TaskPrimitive::Upload { .. } => Self::roles(
                &["button", "link", "textbox"],
                Some("upload file".into()),
                "upload_controls",
            ),
            TaskPrimitive::Download { .. } => Self::roles(
                &["button", "link"],
                Some("download".into()),
                "download_controls",
            ),
            TaskPrimitive::DownloadUpload { .. } => Self::roles(
                &["button", "link", "textbox"],
                Some("upload file".into()),
                "download_upload_controls",
            ),
            TaskPrimitive::Repeat {
                action,
                click_outcome,
                ..
            } => {
                let roles: &[&str] = match action {
                    RepeatAction::Click => &["button", "link", "tab", "menuitem"],
                    RepeatAction::VerifyVisible => &["alert", "status"],
                };
                Self::roles(
                    roles,
                    click_outcome.as_ref().and_then(outcome_text),
                    "repeat_controls",
                )
            }
        }
    }

    fn roles(roles: &[&str], query: Option<String>, source: &'static str) -> Self {
        Self {
            query: query
                .map(|query| query.trim().to_owned())
                .filter(|query| !query.is_empty())
                .map(|query| query.chars().take(512).collect()),
            required_roles: bounded_roles(roles.iter().copied().collect()),
            exhaustive: false,
            source,
        }
    }

    pub fn receipt(&self) -> Value {
        json!({
            "source": self.source,
            "query_present": self.query.is_some(),
            "required_roles": self.required_roles,
            "exhaustive": self.exhaustive,
            "policy": "required_roles_and_feedback_survive_pruning_then_query_rank",
            "derived_from_task_only": true,
        })
    }
}

fn outcome_text(outcome: &Value) -> Option<String> {
    let kind = outcome.get("kind").and_then(Value::as_str)?;
    let text = match kind {
        "text" => outcome.get("text").and_then(Value::as_str),
        "navigation" => outcome.get("url").and_then(Value::as_str),
        _ => None,
    }?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(512).collect())
}

fn bounded_roles(roles: BTreeSet<&str>) -> Vec<String> {
    roles
        .into_iter()
        .take(MAX_TASK_FILTER_ROLES)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3_tasks::MutationStep;
    use serde_json::json;

    #[test]
    fn mutation_filter_keeps_fill_and_click_roles_and_uses_only_outcome_text() {
        let task = TaskPrimitive::Mutation {
            steps: vec![
                MutationStep::Fill {
                    reference: "@c1".into(),
                    expected_value: "old".into(),
                    value: "person@example.test".into(),
                },
                MutationStep::Click {
                    reference: "@c2".into(),
                    outcome: json!({"kind":"text","selector":"#notice","text":"Saved"}),
                },
            ],
        };
        let filter = TaskObservationFilter::for_task(&task);
        assert_eq!(filter.query.as_deref(), Some("Saved"));
        assert!(filter.required_roles.iter().any(|role| role == "textbox"));
        assert!(filter.required_roles.iter().any(|role| role == "button"));
        assert!(!filter.receipt().to_string().contains("person@example.test"));
        assert!(
            filter.receipt()["derived_from_task_only"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn form_and_repeat_filters_are_bounded_and_deterministic() {
        let form = TaskPrimitive::FormDraft {
            fields: vec![],
            save_without_submit: false,
            save_control: None,
        };
        let first = TaskObservationFilter::for_task(&form);
        let second = TaskObservationFilter::for_task(&form);
        assert_eq!(first, second);
        assert!(first.required_roles.contains(&"textbox".into()));
        assert!(first.required_roles.len() <= MAX_TASK_FILTER_ROLES);

        let repeat = TaskPrimitive::Repeat {
            references: vec!["@c1".into()],
            action: RepeatAction::Click,
            click_outcome: Some(json!({"kind":"navigation","url":"https://example.test/next"})),
        };
        let filter = TaskObservationFilter::for_task(&repeat);
        assert_eq!(filter.query.as_deref(), Some("https://example.test/next"));
        assert!(filter.required_roles.contains(&"button".into()));
    }

    #[test]
    fn research_filter_carries_goal_without_page_roles() {
        let task = TaskPrimitive::Research {
            urls: vec!["https://example.test".into()],
            selector: None,
            goal: Some("search for buffalo bills".into()),
            tab_ids: None,
        };
        let filter = TaskObservationFilter::for_task(&task);
        assert_eq!(filter.query.as_deref(), Some("search for buffalo bills"));
        assert!(filter.required_roles.is_empty());
    }
}
