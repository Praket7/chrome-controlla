//! Shared, bounded task-program representation.
//!
//! `browser(action="task")` and public workflows compile to this program shape,
//! which then lowers into the existing workflow VM. No second executor lives
//! here; this module owns validation and program-level evidence only.

use serde::{Deserialize, Serialize};

pub const MAX_TASK_PROGRAM_STEPS: usize = 128;
pub const MAX_TASK_PROGRAM_NESTING: usize = 16;
pub const MAX_TASK_FOREACH_ITEMS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskProgramLimits {
    pub max_steps: usize,
    pub max_nesting: usize,
    pub max_foreach_items: usize,
}

impl Default for TaskProgramLimits {
    fn default() -> Self {
        Self {
            max_steps: MAX_TASK_PROGRAM_STEPS,
            max_nesting: MAX_TASK_PROGRAM_NESTING,
            max_foreach_items: MAX_TASK_FOREACH_ITEMS,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStepKind {
    Navigate,
    Observe,
    Find,
    Click,
    Fill,
    Type,
    Press,
    Select,
    Scroll,
    ScrollTo,
    Hover,
    Drag,
    WaitFor,
    Extract,
    Assert,
    Verify,
    If,
    Foreach,
    TryFallback,
    ParallelRead,
}

impl TaskStepKind {
    pub fn is_read_only(self) -> bool {
        matches!(
            self,
            Self::Observe
                | Self::Find
                | Self::WaitFor
                | Self::Extract
                | Self::Assert
                | Self::Verify
                | Self::ParallelRead
        )
    }

    pub fn is_control_flow(self) -> bool {
        matches!(self, Self::If | Self::Foreach | Self::TryFallback | Self::ParallelRead)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStep {
    pub kind: TaskStepKind,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub expected_value: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub save_as: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub var: Option<String>,
    #[serde(default)]
    pub steps: Vec<TaskStep>,
    #[serde(default)]
    pub fallback: Vec<TaskStep>,
    #[serde(default)]
    pub predicate: Option<serde_json::Value>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl Default for TaskStepKind {
    fn default() -> Self {
        Self::Observe
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProgram {
    #[serde(default)]
    pub steps: Vec<TaskStep>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledTaskProgram {
    pub steps: Vec<TaskStep>,
    pub expanded_steps: usize,
    pub max_depth: usize,
    pub has_mutation: bool,
    pub has_control_flow: bool,
}

pub fn compile_task_program(
    program: &TaskProgram,
    limits: TaskProgramLimits,
) -> Result<CompiledTaskProgram, String> {
    if program.steps.is_empty() {
        return Err("task program must contain at least one step".into());
    }
    let limits = TaskProgramLimits {
        max_steps: limits.max_steps.clamp(1, MAX_TASK_PROGRAM_STEPS),
        max_nesting: limits.max_nesting.clamp(1, MAX_TASK_PROGRAM_NESTING),
        max_foreach_items: limits.max_foreach_items.clamp(1, MAX_TASK_FOREACH_ITEMS),
    };
    let mut expanded = 0usize;
    let mut max_depth = 0usize;
    let mut has_mutation = false;
    let mut has_control_flow = false;
    let mut variables = std::collections::BTreeSet::new();
    validate_steps(
        &program.steps,
        1,
        limits,
        &mut expanded,
        &mut max_depth,
        &mut has_mutation,
        &mut has_control_flow,
        &mut variables,
    )?;
    if expanded > limits.max_steps {
        return Err(format!(
            "task program expands to {expanded} steps, above the {} step bound",
            limits.max_steps
        ));
    }
    Ok(CompiledTaskProgram {
        steps: program.steps.clone(),
        expanded_steps: expanded,
        max_depth,
        has_mutation,
        has_control_flow,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_steps(
    steps: &[TaskStep],
    depth: usize,
    limits: TaskProgramLimits,
    expanded: &mut usize,
    max_depth: &mut usize,
    has_mutation: &mut bool,
    has_control_flow: &mut bool,
    variables: &mut std::collections::BTreeSet<String>,
) -> Result<(), String> {
    if depth > limits.max_nesting {
        return Err("task program nesting exceeds the bound".into());
    }
    *max_depth = (*max_depth).max(depth);
    for step in steps {
        *expanded = expanded.saturating_add(1);
        if *expanded > limits.max_steps {
            return Err("task program exceeds the step bound".into());
        }
        if !step.kind.is_read_only() {
            *has_mutation = true;
        }
        if step.kind.is_control_flow() {
            *has_control_flow = true;
        }
        if let Some(name) = &step.save_as {
            validate_variable(name)?;
            if !variables.insert(name.clone()) {
                return Err(format!("task variable {name} is defined more than once"));
            }
        }
        match step.kind {
            TaskStepKind::Navigate => {
                let url = step
                    .url
                    .as_deref()
                    .ok_or_else(|| "navigate requires url".to_owned())?;
                validate_http_url(url)?;
            }
            TaskStepKind::Find => {
                if step.query.as_deref().unwrap_or_default().is_empty() {
                    return Err("find requires a query".into());
                }
            }
            TaskStepKind::If => {
                if step.condition.as_deref().unwrap_or_default().is_empty() {
                    return Err("if requires a bounded condition".into());
                }
                if step.steps.is_empty() {
                    return Err("if requires a then branch".into());
                }
                validate_steps(
                    &step.steps,
                    depth + 1,
                    limits,
                    expanded,
                    max_depth,
                    has_mutation,
                    has_control_flow,
                    variables,
                )?;
                validate_steps(
                    &step.fallback,
                    depth + 1,
                    limits,
                    expanded,
                    max_depth,
                    has_mutation,
                    has_control_flow,
                    variables,
                )?;
            }
            TaskStepKind::Foreach => {
                if step.items.is_empty() || step.items.len() > limits.max_foreach_items {
                    return Err("foreach requires 1..=32 concrete items".into());
                }
                let var = step
                    .var
                    .as_deref()
                    .ok_or_else(|| "foreach requires a variable name".to_owned())?;
                validate_variable(var)?;
                if step.steps.is_empty() {
                    return Err("foreach requires a body".into());
                }
                validate_steps(
                    &step.steps,
                    depth + 1,
                    limits,
                    expanded,
                    max_depth,
                    has_mutation,
                    has_control_flow,
                    variables,
                )?;
            }
            TaskStepKind::TryFallback => {
                if step.steps.is_empty() || step.fallback.is_empty() {
                    return Err("try_fallback requires both branches".into());
                }
                validate_steps(
                    &step.steps,
                    depth + 1,
                    limits,
                    expanded,
                    max_depth,
                    has_mutation,
                    has_control_flow,
                    variables,
                )?;
                validate_steps(
                    &step.fallback,
                    depth + 1,
                    limits,
                    expanded,
                    max_depth,
                    has_mutation,
                    has_control_flow,
                    variables,
                )?;
            }
            TaskStepKind::ParallelRead => {
                if step.steps.is_empty() {
                    return Err("parallel_read requires at least one read step".into());
                }
                if !step.steps.iter().all(|child| child.kind.is_read_only()) {
                    return Err("parallel_read may contain only read-only steps".into());
                }
                validate_steps(
                    &step.steps,
                    depth + 1,
                    limits,
                    expanded,
                    max_depth,
                    has_mutation,
                    has_control_flow,
                    variables,
                )?;
            }
            TaskStepKind::Click
            | TaskStepKind::Fill
            | TaskStepKind::Type
            | TaskStepKind::Press
            | TaskStepKind::Select
            | TaskStepKind::Scroll
            | TaskStepKind::ScrollTo
            | TaskStepKind::Hover
            | TaskStepKind::Drag => {
                if step.reference.is_none() && step.target.is_none() && step.query.is_none() {
                    return Err(format!("{:?} requires a target, reference, or query", step.kind));
                }
            }
            TaskStepKind::WaitFor => {
                if step.selector.as_deref().unwrap_or_default().is_empty() {
                    return Err("wait_for requires a selector".into());
                }
                let timeout = step.timeout_ms.unwrap_or(30_000);
                if !(1..=30_000).contains(&timeout) {
                    return Err("wait_for timeout must be in 1..=30000".into());
                }
            }
            TaskStepKind::Extract => {
                if step.selector.as_deref().unwrap_or_default().is_empty() {
                    return Err("extract requires a selector".into());
                }
            }
            TaskStepKind::Assert | TaskStepKind::Verify => {
                if step.predicate.is_none() {
                    return Err("verify requires a typed predicate".into());
                }
            }
            TaskStepKind::Observe => {}
        }
    }
    Ok(())
}

fn validate_variable(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Err("task variables must be 1..=64 ASCII alphanumeric/underscore characters".into())
    } else {
        Ok(())
    }
}

fn validate_http_url(url: &str) -> Result<(), String> {
    if url.is_empty() || url.len() > 4096 {
        return Err("navigate url must be a bounded HTTP(S) URL".into());
    }
    let remainder = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| "navigate requires an HTTP(S) URL".to_owned())?;
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err("navigate URL credentials are forbidden and a host is required".into());
    }
    Ok(())
}

/// Lower a compiled program to JSON operations consumed by the existing VM
/// host. This is intentionally a lossless representation, not a second runtime.
pub fn lower_to_vm_ops(program: &CompiledTaskProgram) -> Vec<serde_json::Value> {
    program
        .steps
        .iter()
        .map(|step| {
            let mut value = serde_json::to_value(step).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("kind".into(), serde_json::Value::String(
                    serde_json::to_string(&step.kind)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_owned(),
                ));
            }
            value
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: TaskStepKind) -> TaskStep {
        TaskStep {
            kind,
            ..TaskStep::default()
        }
    }

    #[test]
    fn empty_and_oversized_programs_fail_closed() {
        assert!(compile_task_program(&TaskProgram::default(), TaskProgramLimits::default()).is_err());
        let program = TaskProgram {
            steps: vec![step(TaskStepKind::Observe); 200],
        };
        assert!(compile_task_program(&program, TaskProgramLimits::default()).is_err());
    }

    #[test]
    fn control_flow_is_bounded_and_typed() {
        let program = TaskProgram {
            steps: vec![TaskStep {
                kind: TaskStepKind::Foreach,
                items: vec!["a".into(), "b".into()],
                var: Some("item".into()),
                steps: vec![TaskStep {
                    kind: TaskStepKind::Click,
                    reference: Some("@c1".into()),
                    ..TaskStep::default()
                }],
                ..TaskStep::default()
            }],
        };
        let compiled = compile_task_program(&program, TaskProgramLimits::default()).unwrap();
        assert_eq!(compiled.expanded_steps, 2);
        assert!(compiled.has_control_flow);
        assert!(compiled.has_mutation);
        let unbounded = TaskProgram {
            steps: vec![TaskStep {
                kind: TaskStepKind::Foreach,
                items: (0..40).map(|index| index.to_string()).collect(),
                var: Some("item".into()),
                steps: vec![step(TaskStepKind::Observe)],
                ..TaskStep::default()
            }],
        };
        assert!(compile_task_program(&unbounded, TaskProgramLimits::default()).is_err());
    }

    #[test]
    fn parallel_reads_reject_mutations() {
        let program = TaskProgram {
            steps: vec![TaskStep {
                kind: TaskStepKind::ParallelRead,
                steps: vec![TaskStep {
                    kind: TaskStepKind::Click,
                    reference: Some("@c1".into()),
                    ..TaskStep::default()
                }],
                ..TaskStep::default()
            }],
        };
        assert!(compile_task_program(&program, TaskProgramLimits::default()).is_err());
    }

    #[test]
    fn navigation_rejects_credentials() {
        let program = TaskProgram {
            steps: vec![TaskStep {
                kind: TaskStepKind::Navigate,
                url: Some("https://user:pass@example.test/".into()),
                ..TaskStep::default()
            }],
        };
        assert!(compile_task_program(&program, TaskProgramLimits::default()).is_err());
    }
}
