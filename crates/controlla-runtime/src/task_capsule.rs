//! A bounded, model-facing execution capsule.
//!
//! The capsule is deliberately data-only. Planning stays with the caller;
//! execution can be compiled into the existing workflow VM without another
//! model round trip between obvious browser operations.

use serde::{Deserialize, Serialize};

pub const MAX_CAPSULE_STEPS: usize = 128;
pub const MAX_REQUIRED_OUTPUTS: usize = 32;
pub const MAX_OUTPUT_NAME: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCapsule {
    pub session_id: String,
    pub target_id: String,
    #[serde(default)]
    pub observation: ObservationRequirements,
    pub steps: Vec<CapsuleStep>,
    #[serde(default)]
    pub required_outputs: Vec<String>,
    #[serde(default)]
    pub budget: CapsuleBudget,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRequirements {
    #[serde(default)]
    pub semantic: bool,
    #[serde(default)]
    pub visual: bool,
    #[serde(default)]
    pub max_items: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapsuleStep {
    Find {
        reference: String,
    },
    Click {
        reference: String,
        verify: String,
    },
    Fill {
        reference: String,
        value: String,
        verify: String,
    },
    Press {
        key: String,
    },
    Scroll {
        delta_x: i32,
        delta_y: i32,
    },
    WaitFor {
        predicate: String,
    },
    Extract {
        selector: String,
        output: String,
    },
    Download {
        reference: String,
        output: String,
    },
    Upload {
        artifact: String,
        reference: String,
        verify: String,
    },
    Assert {
        predicate: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleBudget {
    #[serde(default = "default_steps")]
    pub max_steps: usize,
    #[serde(default = "default_commands")]
    pub max_browser_commands: usize,
    #[serde(default = "default_wall_ms")]
    pub wall_time_ms: u64,
}

impl Default for CapsuleBudget {
    fn default() -> Self {
        Self {
            max_steps: default_steps(),
            max_browser_commands: default_commands(),
            wall_time_ms: default_wall_ms(),
        }
    }
}

fn default_steps() -> usize {
    128
}
fn default_commands() -> usize {
    256
}
fn default_wall_ms() -> u64 {
    30_000
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapsuleValidationError(pub String);

impl TaskCapsule {
    pub fn validate(&self) -> Result<(), CapsuleValidationError> {
        if self.session_id.trim().is_empty() || self.target_id.trim().is_empty() {
            return Err(CapsuleValidationError(
                "session_id and target_id are required".into(),
            ));
        }
        if self.steps.is_empty() || self.steps.len() > MAX_CAPSULE_STEPS {
            return Err(CapsuleValidationError(
                "capsule steps must be nonempty and bounded".into(),
            ));
        }
        if self.budget.max_steps == 0
            || self.budget.max_steps > MAX_CAPSULE_STEPS
            || self.steps.len() > self.budget.max_steps
        {
            return Err(CapsuleValidationError("capsule exceeds max_steps".into()));
        }
        if self.budget.max_browser_commands == 0 || self.budget.wall_time_ms == 0 {
            return Err(CapsuleValidationError(
                "capsule budget must be positive".into(),
            ));
        }
        if self.required_outputs.len() > MAX_REQUIRED_OUTPUTS {
            return Err(CapsuleValidationError(
                "required_outputs is too large".into(),
            ));
        }
        let mut outputs = std::collections::HashSet::new();
        for output in &self.required_outputs {
            if output.trim().is_empty() || output.len() > MAX_OUTPUT_NAME || !outputs.insert(output)
            {
                return Err(CapsuleValidationError(
                    "required_outputs must be unique, bounded names".into(),
                ));
            }
        }
        for step in &self.steps {
            match step {
                CapsuleStep::Click { verify, .. }
                | CapsuleStep::Fill { verify, .. }
                | CapsuleStep::Upload { verify, .. } => {
                    if verify.trim().is_empty() {
                        return Err(CapsuleValidationError(
                            "mutations require a verification predicate".into(),
                        ));
                    }
                }
                CapsuleStep::Extract { output, .. } | CapsuleStep::Download { output, .. } => {
                    if output.trim().is_empty() || output.len() > MAX_OUTPUT_NAME {
                        return Err(CapsuleValidationError(
                            "artifact outputs must be named and bounded".into(),
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capsule() -> TaskCapsule {
        TaskCapsule {
            session_id: "s".into(),
            target_id: "t".into(),
            observation: ObservationRequirements {
                semantic: true,
                visual: false,
                max_items: 20,
            },
            steps: vec![CapsuleStep::Click {
                reference: "@c1".into(),
                verify: "visible".into(),
            }],
            required_outputs: vec!["result".into()],
            budget: CapsuleBudget::default(),
        }
    }

    #[test]
    fn bounded_capsule_requires_postconditions_for_mutations() {
        assert!(capsule().validate().is_ok());
    }

    #[test]
    fn invalid_capsules_fail_before_dispatch() {
        let mut value = capsule();
        value.steps = vec![CapsuleStep::Click {
            reference: "@c1".into(),
            verify: String::new(),
        }];
        assert!(value.validate().is_err());
    }
}
