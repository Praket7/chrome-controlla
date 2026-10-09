//! Deterministic, bounded browser-workflow contract used by the compact MCP surface.
//!
//! This module deliberately separates workflow validation from execution. A valid
//! workflow cannot contain loops, unresolved symbolic references, implicit
//! authority crossings, or unbounded observations. Executors still re-check live
//! target/document authority immediately before every browser side effect.

use crate::workflow::{EffectClass, MAX_SCRIPT_BYTES, MAX_STEPS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_ACTION_TEXT: usize = 16_384;
pub const MAX_QUERY_CHARS: usize = 512;
pub const MAX_SELECTOR_CHARS: usize = 2048;
pub const MAX_KEY_CHARS: usize = 64;
pub const MAX_TIMEOUT_MS: u64 = 30_000;
pub const MAX_STEP_BYTES: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserWorkflowPredicate {
    Exists { reference: String },
    Text { reference: String, equals: String },
    Value { reference: String, equals: String },
    Url { equals: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserWorkflowStep {
    Navigate {
        url: String,
    },
    Find {
        query: String,
        role: Option<String>,
        save_as: String,
    },
    Click {
        reference: String,
        expected_outcome: String,
    },
    Fill {
        reference: String,
        expected_value: String,
        value: String,
    },
    Type {
        reference: String,
        expected_value: String,
        value: String,
    },
    Press {
        reference: String,
        key: String,
    },
    Select {
        reference: String,
        expected_value: String,
        value: String,
    },
    WaitFor {
        selector: String,
        timeout_ms: u64,
    },
    Observe {
        selector: String,
        max_bytes: usize,
    },
    Extract {
        selector: String,
        max_items: usize,
        max_bytes: usize,
    },
    Assert {
        predicate: BrowserWorkflowPredicate,
    },
    Verify {
        predicate: BrowserWorkflowPredicate,
    },
    Checkpoint,
    Script {
        source: String,
    },
}

impl BrowserWorkflowStep {
    pub fn effect(&self) -> EffectClass {
        match self {
            Self::Find { .. }
            | Self::WaitFor { .. }
            | Self::Observe { .. }
            | Self::Extract { .. }
            | Self::Assert { .. }
            | Self::Verify { .. }
            | Self::Checkpoint => EffectClass::ReadOnly,
            Self::Fill { .. } | Self::Type { .. } | Self::Select { .. } => EffectClass::Reversible,
            Self::Click { .. } | Self::Press { .. } => EffectClass::External,
            Self::Navigate { .. } => EffectClass::AuthorityBoundary,
            Self::Script { .. } => EffectClass::Unknown,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserWorkflowDefinition {
    pub steps: Vec<BrowserWorkflowStep>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledBrowserWorkflow {
    pub steps: Vec<BrowserWorkflowStep>,
    pub effects: Vec<EffectClass>,
}

pub fn compile_browser_workflow(
    definition: BrowserWorkflowDefinition,
) -> Result<CompiledBrowserWorkflow, String> {
    if definition.steps.is_empty() || definition.steps.len() > MAX_STEPS {
        return Err(format!("workflow must contain 1..={MAX_STEPS} steps"));
    }
    let script_count = definition
        .steps
        .iter()
        .filter(|step| matches!(step, BrowserWorkflowStep::Script { .. }))
        .count();
    if script_count > 0 && (script_count != 1 || definition.steps.len() != 1) {
        return Err("script must be the only workflow step".into());
    }

    let mut variables = BTreeSet::<String>::new();
    for step in &definition.steps {
        validate_step(step, &variables)?;
        if let BrowserWorkflowStep::Find { save_as, .. } = step {
            if !variables.insert(save_as.clone()) {
                return Err(format!(
                    "workflow variable ${save_as} is defined more than once"
                ));
            }
        }
    }

    let effects = definition
        .steps
        .iter()
        .map(BrowserWorkflowStep::effect)
        .collect();
    Ok(CompiledBrowserWorkflow {
        steps: definition.steps,
        effects,
    })
}

fn validate_step(step: &BrowserWorkflowStep, variables: &BTreeSet<String>) -> Result<(), String> {
    match step {
        BrowserWorkflowStep::Navigate { url } => validate_http_url(url),
        BrowserWorkflowStep::Find {
            query,
            role,
            save_as,
        } => {
            bounded_nonempty(query, MAX_QUERY_CHARS, "find query")?;
            if let Some(role) = role {
                bounded_nonempty(role, 128, "find role")?;
            }
            validate_variable_name(save_as)
        }
        BrowserWorkflowStep::Click {
            reference,
            expected_outcome,
        } => {
            validate_reference(reference, variables)?;
            bounded_nonempty(expected_outcome, 1024, "click expected_outcome")
        }
        BrowserWorkflowStep::Fill {
            reference,
            expected_value,
            value,
        }
        | BrowserWorkflowStep::Type {
            reference,
            expected_value,
            value,
        }
        | BrowserWorkflowStep::Select {
            reference,
            expected_value,
            value,
        } => {
            validate_reference(reference, variables)?;
            bounded(expected_value, MAX_ACTION_TEXT, "expected_value")?;
            bounded(value, MAX_ACTION_TEXT, "value")
        }
        BrowserWorkflowStep::Press { reference, key } => {
            validate_reference(reference, variables)?;
            bounded_nonempty(key, MAX_KEY_CHARS, "key")
        }
        BrowserWorkflowStep::WaitFor {
            selector,
            timeout_ms,
        } => {
            bounded_nonempty(selector, MAX_SELECTOR_CHARS, "wait selector")?;
            validate_timeout(*timeout_ms)
        }
        BrowserWorkflowStep::Observe {
            selector,
            max_bytes,
        } => {
            bounded_nonempty(selector, MAX_SELECTOR_CHARS, "observe selector")?;
            validate_bytes(*max_bytes)
        }
        BrowserWorkflowStep::Extract {
            selector,
            max_items,
            max_bytes,
        } => {
            bounded_nonempty(selector, MAX_SELECTOR_CHARS, "extract selector")?;
            if !(1..=500).contains(max_items) {
                return Err("extract max_items must be in 1..=500".into());
            }
            validate_bytes(*max_bytes)
        }
        BrowserWorkflowStep::Assert { predicate } | BrowserWorkflowStep::Verify { predicate } => {
            validate_predicate(predicate, variables)
        }
        BrowserWorkflowStep::Checkpoint => Ok(()),
        BrowserWorkflowStep::Script { source } => {
            bounded_nonempty(source, MAX_SCRIPT_BYTES, "script source")
        }
    }
}

fn validate_predicate(
    predicate: &BrowserWorkflowPredicate,
    variables: &BTreeSet<String>,
) -> Result<(), String> {
    match predicate {
        BrowserWorkflowPredicate::Exists { reference } => validate_reference(reference, variables),
        BrowserWorkflowPredicate::Text { reference, equals }
        | BrowserWorkflowPredicate::Value { reference, equals } => {
            validate_reference(reference, variables)?;
            bounded(equals, MAX_ACTION_TEXT, "predicate value")
        }
        BrowserWorkflowPredicate::Url { equals } => validate_http_url(equals),
    }
}

fn validate_reference(reference: &str, variables: &BTreeSet<String>) -> Result<(), String> {
    if let Some(name) = reference.strip_prefix('$') {
        validate_variable_name(name)?;
        if variables.contains(name) {
            return Ok(());
        }
        return Err(format!("workflow variable ${name} is unresolved"));
    }
    if let Some(number) = reference.strip_prefix("@c")
        && !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && number.parse::<u32>().is_ok_and(|value| value > 0)
    {
        return Ok(());
    }
    Err("reference must be an exact @cN value or an earlier $variable".into())
}

fn validate_variable_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Err(
            "workflow variable names must be 1..=64 ASCII alphanumeric/underscore characters"
                .into(),
        )
    } else {
        Ok(())
    }
}

fn validate_http_url(url: &str) -> Result<(), String> {
    bounded_nonempty(url, 4096, "url")?;
    let remainder = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| "navigation/URL predicates require http(s) URLs".to_owned())?;
    if remainder.is_empty() {
        return Err("URL host is required".into());
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err("URL credentials are forbidden and a host is required".into());
    }
    Ok(())
}

fn validate_timeout(timeout_ms: u64) -> Result<(), String> {
    if (1..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
        Ok(())
    } else {
        Err(format!("timeout_ms must be in 1..={MAX_TIMEOUT_MS}"))
    }
}

fn validate_bytes(max_bytes: usize) -> Result<(), String> {
    if (4096..=MAX_STEP_BYTES).contains(&max_bytes) {
        Ok(())
    } else {
        Err(format!("max_bytes must be in 4096..={MAX_STEP_BYTES}"))
    }
}

fn bounded(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.len() <= max {
        Ok(())
    } else {
        Err(format!("{label} exceeds {max} bytes"))
    }
}

fn bounded_nonempty(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > max {
        Err(format!("{label} must contain 1..={max} bytes"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(steps: Vec<BrowserWorkflowStep>) -> Result<CompiledBrowserWorkflow, String> {
        compile_browser_workflow(BrowserWorkflowDefinition { steps })
    }

    #[test]
    fn complete_workflow_vocabulary_compiles_with_ordered_refs() {
        let result = compile(vec![
            BrowserWorkflowStep::Navigate {
                url: "https://example.test/form".into(),
            },
            BrowserWorkflowStep::Find {
                query: "Email".into(),
                role: Some("textbox".into()),
                save_as: "email".into(),
            },
            BrowserWorkflowStep::Fill {
                reference: "$email".into(),
                expected_value: String::new(),
                value: "person@example.test".into(),
            },
            BrowserWorkflowStep::Verify {
                predicate: BrowserWorkflowPredicate::Value {
                    reference: "$email".into(),
                    equals: "person@example.test".into(),
                },
            },
        ])
        .unwrap();
        assert_eq!(result.steps.len(), 4);
        assert_eq!(result.effects[0], EffectClass::AuthorityBoundary);
        assert_eq!(result.effects[2], EffectClass::Reversible);
        assert_eq!(result.effects[3], EffectClass::ReadOnly);
    }

    #[test]
    fn unresolved_and_duplicate_variables_fail_closed() {
        assert!(
            compile(vec![BrowserWorkflowStep::Click {
                reference: "$missing".into(),
                expected_outcome: "dialog visible".into(),
            }])
            .is_err()
        );
        assert!(
            compile(vec![
                BrowserWorkflowStep::Find {
                    query: "one".into(),
                    role: None,
                    save_as: "same".into(),
                },
                BrowserWorkflowStep::Find {
                    query: "two".into(),
                    role: None,
                    save_as: "same".into(),
                },
            ])
            .is_err()
        );
    }

    #[test]
    fn navigation_rejects_credentials_and_non_http_schemes() {
        for url in [
            "file:///tmp/a",
            "javascript:alert(1)",
            "https://user:pass@example.test/",
        ] {
            assert!(compile(vec![BrowserWorkflowStep::Navigate { url: url.into() }]).is_err());
        }
    }

    #[test]
    fn script_isolated_and_bounds_are_enforced() {
        assert!(
            compile(vec![
                BrowserWorkflowStep::Script {
                    source: "return 1".into()
                },
                BrowserWorkflowStep::Checkpoint,
            ])
            .is_err()
        );
        assert!(
            compile(vec![BrowserWorkflowStep::WaitFor {
                selector: "body".into(),
                timeout_ms: MAX_TIMEOUT_MS + 1,
            }])
            .is_err()
        );
        assert!(
            compile(vec![BrowserWorkflowStep::Extract {
                selector: "body".into(),
                max_items: 501,
                max_bytes: 4096,
            }])
            .is_err()
        );
    }
}
