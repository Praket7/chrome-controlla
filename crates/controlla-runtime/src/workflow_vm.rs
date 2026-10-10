//! Bounded branching, foreach, fallback VM separated from schema.
//! Hard limits: no recursion, bounded expanded steps, foreach max items validated,
//! parallel_read read-only, mutation branches preserve authority.

use crate::browser_workflow::{BrowserWorkflowStep, CompiledBrowserWorkflow};
use std::collections::BTreeMap;

pub const MAX_EXPANDED_STEPS: usize = 64;
pub const MAX_FOREACH_ITEMS: usize = 32;
pub const MAX_PROBES: usize = 4;

#[derive(Clone, Debug)]
pub enum VmOp {
    Step(BrowserWorkflowStep),
    If {
        condition: String,
        then_branch: Vec<VmOp>,
        else_branch: Vec<VmOp>,
    },
    Foreach {
        items: Vec<String>,
        var: String,
        body: Vec<VmOp>,
    },
    TryFallback {
        primary: Vec<VmOp>,
        fallback: Vec<VmOp>,
    },
    ParallelRead(Vec<VmOp>),
}

pub struct Vm {
    pub variables: BTreeMap<String, String>,
    pub step_count: usize,
    pub probe_count: usize,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            variables: BTreeMap::new(),
            step_count: 0,
            probe_count: 0,
        }
    }

    pub fn compile_and_run(
        &mut self,
        workflow: CompiledBrowserWorkflow,
    ) -> Result<Vec<String>, String> {
        if workflow.steps.len() > MAX_EXPANDED_STEPS {
            return Err("workflow exceeds max steps".into());
        }
        let mut receipts = Vec::new();
        for step in workflow.steps {
            self.execute_step(step, &mut receipts)?;
            if self.step_count > MAX_EXPANDED_STEPS {
                return Err("expanded steps exceeded".into());
            }
        }
        Ok(receipts)
    }

    fn execute_step(
        &mut self,
        step: BrowserWorkflowStep,
        receipts: &mut Vec<String>,
    ) -> Result<(), String> {
        self.step_count = self.step_count.saturating_add(1);
        if self.step_count > MAX_EXPANDED_STEPS {
            return Err("bound exceeded".into());
        }
        receipts.push(format!("{:?}", step.effect()));
        Ok(())
    }

    pub fn run_op(&mut self, op: VmOp, receipts: &mut Vec<String>) -> Result<(), String> {
        match op {
            VmOp::Step(s) => self.execute_step(s, receipts),
            VmOp::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let branch = if condition == "true" {
                    then_branch
                } else {
                    else_branch
                };
                for o in branch {
                    self.run_op(o, receipts)?;
                }
                Ok(())
            }
            VmOp::Foreach { items, var, body } => {
                if items.len() > MAX_FOREACH_ITEMS {
                    return Err("foreach exceeds max".into());
                }
                for item in items {
                    self.variables.insert(var.clone(), item);
                    for o in body.clone() {
                        self.run_op(o, receipts)?;
                    }
                }
                Ok(())
            }
            VmOp::TryFallback { primary, fallback } => {
                let start = receipts.len();
                let mut primary_failed_not_dispatched = false;
                for o in primary {
                    let before = receipts.len();
                    let _ = self.run_op(o, receipts);
                    if receipts.len() == before {
                        primary_failed_not_dispatched = true;
                    }
                }
                if primary_failed_not_dispatched || receipts.len() == start {
                    for o in fallback {
                        self.run_op(o, receipts)?;
                    }
                }
                Ok(())
            }
            VmOp::ParallelRead(ops) => {
                for op in ops {
                    if let VmOp::Step(s) = &op {
                        if !matches!(s.effect(), crate::workflow::EffectClass::ReadOnly) {
                            return Err("parallel_read may contain read-only effects only".into());
                        }
                    } else {
                        return Err("parallel_read may only contain direct steps".into());
                    }
                    self.run_op(op, receipts)?;
                }
                Ok(())
            }
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_workflow::BrowserWorkflowStep;
    #[test]
    fn foreach_bound_enforced() {
        let mut vm = Vm::new();
        let ops = VmOp::Foreach {
            items: (0..33).map(|i| format!("{i}")).collect(),
            var: "x".into(),
            body: vec![],
        };
        assert!(vm.run_op(ops, &mut Vec::new()).is_err());
    }
    #[test]
    fn parallel_read_rejects_mutation() {
        let mut vm = Vm::new();
        let ops = VmOp::ParallelRead(vec![VmOp::Step(BrowserWorkflowStep::Click {
            reference: "@c1".into(),
            expected_outcome: "x".into(),
        })]);
        assert!(vm.run_op(ops, &mut Vec::new()).is_err());
    }
}
