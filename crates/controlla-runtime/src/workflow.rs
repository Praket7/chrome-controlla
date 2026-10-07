use controlla_browser::ObserveSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_STEPS: usize = 20;
pub const MAX_WAIT_MS: u64 = 10_000;
pub const MAX_TOTAL_WAIT_MS: u64 = 30_000;
pub const MAX_OUTPUT_BYTES: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowStep {
    Observe { spec: ObserveSpec },
    Wait { ms: u64 },
    Checkpoint,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRequest {
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug)]
pub struct WorkflowGraph {
    pub steps: Vec<WorkflowStep>,
}

pub fn validate_binding(
    request_session: &str,
    reference_session: &str,
    principal: &str,
    expected_principal: &str,
) -> Result<(), &'static str> {
    if request_session != reference_session || principal != expected_principal {
        Err("target_ref is not bound to this session")
    } else {
        Ok(())
    }
}

pub fn compile(request: Value) -> Result<WorkflowGraph, String> {
    fn only_keys(value: &Value, allowed: &[&str]) -> bool {
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
    }
    if !only_keys(&request, &["steps"]) {
        return Err("workflow accepts only steps".into());
    }
    let raw_steps = request["steps"]
        .as_array()
        .ok_or("steps must be an array")?;
    for raw in raw_steps {
        let kind = raw["kind"].as_str().ok_or("step kind is required")?;
        let keys: &[&str] = match kind {
            "observe" => &["kind", "spec"],
            "wait" => &["kind", "ms"],
            "checkpoint" => &["kind"],
            _ => return Err(format!("unsupported workflow node: {kind}")),
        };
        if !only_keys(raw, keys) {
            return Err(format!("unsupported fields in {kind} node"));
        }
        if kind == "observe" {
            let spec = &raw["spec"];
            if !only_keys(
                spec,
                &[
                    "selector",
                    "fields",
                    "max_items",
                    "max_text_chars",
                    "max_bytes",
                    "cursor",
                ],
            ) || !spec["fields"]
                .as_object()
                .is_some_and(|fields| fields.values().all(Value::is_string))
            {
                return Err("invalid observe spec fields".into());
            }
        }
    }
    let request: WorkflowRequest =
        serde_json::from_value(request).map_err(|error| format!("invalid workflow: {error}"))?;
    if request.steps.is_empty() || request.steps.len() > MAX_STEPS {
        return Err(format!("steps must contain 1..={MAX_STEPS} nodes"));
    }
    let mut waits = 0_u64;
    let mut declared_output = 0_usize;
    for step in &request.steps {
        match step {
            WorkflowStep::Wait { ms } if *ms == 0 || *ms > MAX_WAIT_MS => {
                return Err(format!("wait must be 1..={MAX_WAIT_MS} ms"));
            }
            WorkflowStep::Wait { ms } => waits = waits.saturating_add(*ms),
            WorkflowStep::Observe { spec } => {
                if spec.selector.is_empty()
                    || spec.selector.len() > 512
                    || spec.fields.is_empty()
                    || spec.fields.len() > 32
                    || spec.max_items == 0
                    || spec.max_items > 1000
                    || spec.max_text_chars == 0
                    || spec.max_text_chars > 100_000
                    || !(4096..=MAX_OUTPUT_BYTES).contains(&spec.max_bytes)
                    || spec
                        .cursor
                        .as_ref()
                        .is_some_and(|cursor| cursor.len() > 4096)
                    || spec.fields.iter().any(|(name, selector)| {
                        name.is_empty()
                            || name.len() > 64
                            || selector.is_empty()
                            || selector.len() > 512
                    })
                {
                    return Err("observe spec exceeds workflow bounds".into());
                }
                declared_output = declared_output.saturating_add(spec.max_bytes);
            }
            _ => {}
        }
    }
    if waits > MAX_TOTAL_WAIT_MS {
        return Err(format!("total wait exceeds {MAX_TOTAL_WAIT_MS} ms"));
    }
    if declared_output > MAX_OUTPUT_BYTES - 1024 {
        return Err("aggregate observe byte budget exceeds workflow output limit".into());
    }
    Ok(WorkflowGraph {
        steps: request.steps,
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkflowReceipt {
    pub status: &'static str,
    pub completed_steps: usize,
    pub browser_operations: usize,
    pub steps: Vec<Value>,
    pub artifacts: Vec<Value>,
    pub error: Option<String>,
}

impl WorkflowReceipt {
    pub fn partial(error: impl Into<String>, steps: Vec<Value>) -> Self {
        Self {
            status: "partial",
            completed_steps: steps.len(),
            browser_operations: steps.iter().filter(|s| s["kind"] == "observe").count(),
            steps,
            artifacts: vec![],
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bounded_workflow_compiles_and_rejects_escape_and_resource_limits() {
        let valid = json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"text":"p"},"max_items":2,"max_text_chars":40,"max_bytes":4096,"cursor":null}},{"kind":"wait","ms":10},{"kind":"checkpoint"}]});
        assert!(compile(valid).is_ok());
        for invalid in [
            json!({"steps":[{"kind":"script","source":"import('node:fs')"}]}),
            json!({"steps":(0..22).map(|_|json!({"kind":"checkpoint"})).collect::<Vec<_>>()}),
            json!({"steps":[{"kind":"wait","ms":10001}]}),
            json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":1001,"max_text_chars":40,"max_bytes":4096,"cursor":null}}]}),
            json!({"steps":[{"kind":"checkpoint","module":"node:fs"}]}),
            json!({"steps":[{"kind":"checkpoint","network":"fetch"}]}),
            json!({"steps":[{"kind":"checkpoint","process":"spawn"}]}),
            json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":2,"max_text_chars":40,"max_bytes":1000001,"cursor":null}}]}),
            json!({"steps":[{"kind":"wait","ms":10000},{"kind":"wait","ms":10000},{"kind":"wait","ms":10000},{"kind":"wait","ms":1}]}),
            json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":2,"max_text_chars":40,"max_bytes":600000,"cursor":null}},{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":2,"max_text_chars":40,"max_bytes":400000,"cursor":null}}]}),
        ] {
            assert!(compile(invalid.clone()).is_err(), "accepted: {invalid}");
        }
    }

    #[test]
    fn workflow_rejects_cross_session_and_cross_principal_handles() {
        assert!(validate_binding("s1", "s2", "p", "p").is_err());
        assert!(validate_binding("s1", "s1", "p1", "p2").is_err());
        assert!(validate_binding("s1", "s1", "p", "p").is_ok());
    }
}
