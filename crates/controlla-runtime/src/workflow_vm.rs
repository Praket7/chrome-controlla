//! Bounded async workflow executor. Browser hosts own dispatch, guards, budgets,
//! telemetry, and receipt classification; the VM owns control flow and retry rules.

use crate::browser_workflow::{BrowserWorkflowStep, CompiledBrowserWorkflow};
use crate::workflow::EffectClass;
use std::{collections::BTreeMap, future::Future, pin::Pin};

pub const MAX_EXPANDED_STEPS: usize = 64;
pub const MAX_FOREACH_ITEMS: usize = 32;
pub const MAX_NESTING: usize = 16;
pub const MAX_PROBES: usize = 4;
const MAX_CONTROL_WORK: usize = 256;

pub trait VmOperation: Clone + Send + Sync {
    fn effect(&self) -> EffectClass;
    fn is_verification(&self) -> bool {
        false
    }
}

impl VmOperation for BrowserWorkflowStep {
    fn effect(&self) -> EffectClass {
        BrowserWorkflowStep::effect(self)
    }
    fn is_verification(&self) -> bool {
        matches!(self, BrowserWorkflowStep::Verify { .. })
    }
}

#[derive(Clone, Debug)]
pub enum VmOp<S = BrowserWorkflowStep> {
    Step(S),
    If {
        condition: String,
        then_branch: Vec<VmOp<S>>,
        else_branch: Vec<VmOp<S>>,
    },
    Foreach {
        items: Vec<String>,
        var: String,
        body: Vec<VmOp<S>>,
    },
    TryFallback {
        primary: Vec<VmOp<S>>,
        fallback: Vec<VmOp<S>>,
    },
    ParallelRead(Vec<VmOp<S>>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepReceipt {
    pub status: String,
    pub value: String,
    pub variable_update: Option<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmError {
    NotDispatched(String),
    ReadFailure(String),
    UnknownMutation(String),
    Failed(String),
    UnsafeParallelEffect,
    StepLimitExceeded,
    ForeachLimitExceeded,
    WorkLimitExceeded,
    NestingLimitExceeded,
    InvalidCondition(String),
}

impl VmError {
    fn permits_fallback(&self) -> bool {
        matches!(self, Self::NotDispatched(_) | Self::ReadFailure(_))
    }
}

pub trait BrowserVmHost<S: VmOperation = BrowserWorkflowStep>: Sync {
    fn execute<'a>(
        &'a self,
        step: S,
        variables: &'a BTreeMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = Result<StepReceipt, VmError>> + Send + 'a>>;
}

#[derive(Default)]
pub struct Vm {
    pub variables: BTreeMap<String, String>,
    pub step_count: usize,
    pub probe_count: usize,
    pub receipts: Vec<StepReceipt>,
    successful_mutations: usize,
    control_work: usize,
}

impl Vm {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn compile_and_run<H: BrowserVmHost>(
        &mut self,
        workflow: CompiledBrowserWorkflow,
        host: &H,
    ) -> Result<&[StepReceipt], VmError> {
        if workflow.steps.len() > MAX_EXPANDED_STEPS {
            return Err(VmError::StepLimitExceeded);
        }
        self.run_sequence(workflow.steps.into_iter().map(VmOp::Step).collect(), host)
            .await?;
        Ok(&self.receipts)
    }

    pub async fn run_op<S: VmOperation, H: BrowserVmHost<S>>(
        &mut self,
        op: VmOp<S>,
        host: &H,
    ) -> Result<(), VmError> {
        self.run_op_at_depth(op, host, 0).await
    }

    fn run_op_at_depth<'a, S: VmOperation + 'a, H: BrowserVmHost<S> + 'a>(
        &'a mut self,
        op: VmOp<S>,
        host: &'a H,
        depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<(), VmError>> + Send + 'a>> {
        Box::pin(async move {
            if depth > MAX_NESTING {
                return Err(VmError::NestingLimitExceeded);
            }
            self.charge_control_work()?;
            match op {
                VmOp::Step(step) => self.execute_step(host, step).await,
                VmOp::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    let branch = if evaluate_condition(&condition, &self.variables)? {
                        then_branch
                    } else {
                        else_branch
                    };
                    self.run_sequence_at_depth(branch, host, depth + 1).await
                }
                VmOp::Foreach { items, var, body } => {
                    if items.len() > MAX_FOREACH_ITEMS {
                        return Err(VmError::ForeachLimitExceeded);
                    }
                    for item in items {
                        self.charge_control_work()?;
                        self.variables.insert(var.clone(), item);
                        self.run_sequence_at_depth(body.clone(), host, depth + 1)
                            .await?;
                    }
                    Ok(())
                }
                VmOp::TryFallback { primary, fallback } => {
                    let mutations_before = self.successful_mutations;
                    if let Err(error) = self.run_sequence_at_depth(primary, host, depth + 1).await {
                        if !error.permits_fallback()
                            || self.successful_mutations != mutations_before
                        {
                            return Err(error);
                        }
                        return self.run_sequence_at_depth(fallback, host, depth + 1).await;
                    }
                    Ok(())
                }
                VmOp::ParallelRead(operations) => {
                    for _ in &operations {
                        self.charge_control_work()?;
                    }
                    let steps = operations
                        .into_iter()
                        .map(|operation| match operation {
                            VmOp::Step(step) if step.effect() == EffectClass::ReadOnly => Ok(step),
                            _ => Err(VmError::UnsafeParallelEffect),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    if self.step_count.saturating_add(steps.len()) > MAX_EXPANDED_STEPS {
                        return Err(VmError::StepLimitExceeded);
                    }
                    self.step_count += steps.len();
                    let results = futures_util::future::try_join_all(
                        steps
                            .into_iter()
                            .map(|step| host.execute(step, &self.variables)),
                    )
                    .await?;
                    for receipt in results {
                        self.apply_receipt(receipt);
                    }
                    Ok(())
                }
            }
        })
    }

    fn run_sequence_at_depth<'a, S: VmOperation + 'a, H: BrowserVmHost<S> + 'a>(
        &'a mut self,
        operations: Vec<VmOp<S>>,
        host: &'a H,
        depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<(), VmError>> + Send + 'a>> {
        Box::pin(async move {
            let mut operations = operations.into_iter().peekable();
            while let Some(operation) = operations.next() {
                let follows_with_verify = matches!(
                    (operation_is_mutation(&operation), operations.peek()),
                    (true, Some(VmOp::Step(step))) if step.is_verification()
                );
                match self.run_op_at_depth(operation, host, depth).await {
                    Err(error @ VmError::UnknownMutation(_)) if follows_with_verify => {
                        let verify = operations.next().expect("peeked verify operation");
                        match self.run_op_at_depth(verify, host, depth).await {
                            Ok(())
                                if self.receipts.last().is_some_and(|r| r.status == "passed") =>
                            {
                                self.successful_mutations =
                                    self.successful_mutations.saturating_add(1);
                            }
                            _ => return Err(error),
                        }
                    }
                    Err(error) => return Err(error),
                    Ok(()) => {}
                }
            }
            Ok(())
        })
    }

    pub async fn run_sequence<S: VmOperation, H: BrowserVmHost<S>>(
        &mut self,
        operations: Vec<VmOp<S>>,
        host: &H,
    ) -> Result<(), VmError> {
        self.run_sequence_at_depth(operations, host, 0).await
    }

    fn charge_control_work(&mut self) -> Result<(), VmError> {
        if self.control_work >= MAX_CONTROL_WORK {
            return Err(VmError::WorkLimitExceeded);
        }
        self.control_work += 1;
        Ok(())
    }

    async fn execute_step<S: VmOperation, H: BrowserVmHost<S>>(
        &mut self,
        host: &H,
        step: S,
    ) -> Result<(), VmError> {
        if self.step_count >= MAX_EXPANDED_STEPS {
            return Err(VmError::StepLimitExceeded);
        }
        self.step_count += 1;
        let effect = step.effect();
        let receipt = match host.execute(step, &self.variables).await {
            Ok(receipt) => receipt,
            Err(error) => {
                self.receipts.push(StepReceipt {
                    status: match &error {
                        VmError::NotDispatched(_) => "not_dispatched",
                        VmError::ReadFailure(_) | VmError::Failed(_) => "failed",
                        VmError::UnknownMutation(_) => "unknown",
                        _ => "failed",
                    }
                    .into(),
                    value: error.detail().unwrap_or_else(|| format!("{error:?}")),
                    variable_update: None,
                });
                return Err(error);
            }
        };
        if effect != EffectClass::ReadOnly {
            self.successful_mutations = self.successful_mutations.saturating_add(1);
        }
        self.apply_receipt(receipt);
        Ok(())
    }

    fn apply_receipt(&mut self, receipt: StepReceipt) {
        if let Some((name, value)) = &receipt.variable_update {
            self.variables.insert(name.clone(), value.clone());
        }
        self.receipts.push(receipt);
    }
}

fn operation_is_mutation<S: VmOperation>(operation: &VmOp<S>) -> bool {
    matches!(operation, VmOp::Step(step) if step.effect() != EffectClass::ReadOnly)
}

pub fn condition_is_supported(condition: &str) -> bool {
    if condition.len() > 512 {
        return false;
    }
    if matches!(condition, "true" | "false") {
        return true;
    }
    if let Some(name) = condition.strip_prefix("var_present:") {
        return valid_variable_name(name);
    }
    condition
        .strip_prefix("var_equals:")
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(name, _)| valid_variable_name(name))
}

fn valid_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn evaluate_condition(
    condition: &str,
    variables: &BTreeMap<String, String>,
) -> Result<bool, VmError> {
    if condition == "true" {
        return Ok(true);
    }
    if condition == "false" {
        return Ok(false);
    }
    if let Some(name) = condition.strip_prefix("var_present:") {
        if !valid_variable_name(name) {
            return Err(VmError::InvalidCondition(condition.into()));
        }
        return Ok(variables.contains_key(name));
    }
    if let Some((name, expected)) = condition
        .strip_prefix("var_equals:")
        .and_then(|rest| rest.split_once(':'))
    {
        if !valid_variable_name(name) {
            return Err(VmError::InvalidCondition(condition.into()));
        }
        return variables
            .get(name)
            .map(|value| value == expected)
            .ok_or_else(|| {
                VmError::InvalidCondition(format!("workflow variable {name} is not defined"))
            });
    }
    Err(VmError::InvalidCondition(condition.into()))
}

impl VmError {
    fn detail(&self) -> Option<String> {
        match self {
            Self::NotDispatched(detail)
            | Self::ReadFailure(detail)
            | Self::UnknownMutation(detail)
            | Self::Failed(detail) => Some(detail.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_workflow::BrowserWorkflowStep;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct FakeHost {
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        fail_as: Option<VmError>,
        calls: Arc<AtomicUsize>,
        fail_after: usize,
    }

    impl BrowserVmHost for FakeHost {
        fn execute<'a>(
            &'a self,
            step: BrowserWorkflowStep,
            _: &'a BTreeMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = Result<StepReceipt, VmError>> + Send + 'a>> {
            Box::pin(async move {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                if let Some(error) = self.fail_as.clone()
                    && call == self.fail_after
                {
                    return Err(error);
                }
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                tokio::task::yield_now().await;
                self.active.fetch_sub(1, Ordering::SeqCst);
                let variable_update = match step {
                    BrowserWorkflowStep::Find { save_as, .. } => Some((save_as, "@c1".into())),
                    _ => None,
                };
                Ok(StepReceipt {
                    status: "passed".into(),
                    value: "ok".into(),
                    variable_update,
                })
            })
        }
    }

    fn host(fail_as: Option<VmError>) -> FakeHost {
        FakeHost {
            active: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::new(AtomicUsize::new(0)),
            fail_as,
            calls: Arc::new(AtomicUsize::new(0)),
            fail_after: 0,
        }
    }

    fn read_step(name: &str) -> BrowserWorkflowStep {
        BrowserWorkflowStep::Find {
            query: name.into(),
            role: None,
            save_as: name.into(),
        }
    }

    #[tokio::test]
    async fn parallel_reads_run_concurrently_and_update_workflow_variables() {
        let host = host(None);
        let mut vm = Vm::new();
        vm.run_op(
            VmOp::ParallelRead(vec![VmOp::Step(read_step("a")), VmOp::Step(read_step("b"))]),
            &host,
        )
        .await
        .unwrap();
        assert_eq!(host.max_active.load(Ordering::SeqCst), 2);
        assert_eq!(vm.variables.get("a").map(String::as_str), Some("@c1"));
        assert_eq!(vm.step_count, 2);
    }

    #[tokio::test]
    async fn fallback_only_runs_after_known_not_dispatched_or_read_failures() {
        let host = host(Some(VmError::UnknownMutation("delivery uncertain".into())));
        let mut vm = Vm::new();
        let op = VmOp::TryFallback {
            primary: vec![VmOp::Step(BrowserWorkflowStep::Click {
                reference: "@c1".into(),
                expected_outcome: "done".into(),
            })],
            fallback: vec![VmOp::Step(read_step("fallback"))],
        };
        assert!(matches!(
            vm.run_op(op, &host).await,
            Err(VmError::UnknownMutation(_))
        ));
        assert_eq!(vm.step_count, 1);
        assert_eq!(vm.receipts.len(), 1);
        assert_eq!(vm.receipts[0].status, "unknown");
    }

    #[tokio::test]
    async fn unknown_mutation_can_only_continue_to_an_immediate_passing_verify() {
        let host = host(Some(VmError::UnknownMutation("delivery uncertain".into())));
        let mut vm = Vm::new();
        let operations = vec![
            VmOp::Step(BrowserWorkflowStep::Click {
                reference: "@c1".into(),
                expected_outcome: "done".into(),
            }),
            VmOp::Step(BrowserWorkflowStep::Verify {
                predicate: crate::browser_workflow::BrowserWorkflowPredicate::Exists {
                    reference: "@c2".into(),
                },
            }),
        ];
        assert!(vm.run_sequence(operations, &host).await.is_ok());
        assert_eq!(host.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            vm.receipts
                .iter()
                .map(|r| r.status.as_str())
                .collect::<Vec<_>>(),
            ["unknown", "passed"]
        );
        assert_eq!(vm.step_count, 2);
    }

    #[tokio::test]
    async fn fallback_runs_for_known_pre_dispatch_failure() {
        let host = host(Some(VmError::NotDispatched("guard refused".into())));
        let mut vm = Vm::new();
        let op = VmOp::TryFallback {
            primary: vec![VmOp::Step(BrowserWorkflowStep::Click {
                reference: "@c1".into(),
                expected_outcome: "done".into(),
            })],
            fallback: vec![VmOp::Step(read_step("fallback"))],
        };
        assert!(vm.run_op(op, &host).await.is_ok());
        assert_eq!(vm.step_count, 2);
        assert_eq!(vm.receipts.len(), 2);
        assert_eq!(vm.receipts[0].status, "not_dispatched");
        assert_eq!(vm.receipts[1].status, "passed");
    }

    #[tokio::test]
    async fn fallback_is_refused_after_a_successful_primary_mutation() {
        let mut host = host(Some(VmError::ReadFailure("later read failed".into())));
        host.fail_after = 1;
        let mut vm = Vm::new();
        let op = VmOp::TryFallback {
            primary: vec![
                VmOp::Step(BrowserWorkflowStep::Click {
                    reference: "@c1".into(),
                    expected_outcome: "done".into(),
                }),
                VmOp::Step(read_step("later")),
            ],
            fallback: vec![VmOp::Step(read_step("fallback"))],
        };
        assert!(matches!(
            vm.run_op(op, &host).await,
            Err(VmError::ReadFailure(_))
        ));
        assert_eq!(
            host.calls.load(Ordering::SeqCst),
            2,
            "fallback must not execute after a completed mutation"
        );
        assert_eq!(vm.receipts.len(), 2);
        assert_eq!(vm.receipts[0].status, "passed");
        assert_eq!(vm.receipts[1].value, "later read failed");
    }

    #[tokio::test]
    async fn if_conditions_can_read_bounded_workflow_variables() {
        let host = host(None);
        let mut vm = Vm::new();
        vm.variables.insert("choice".into(), "@c1".into());
        vm.variables.insert("empty".into(), String::new());
        vm.run_op(
            VmOp::If {
                condition: "var_equals:choice:@c1".into(),
                then_branch: vec![VmOp::Step(read_step("chosen"))],
                else_branch: vec![VmOp::Step(read_step("other"))],
            },
            &host,
        )
        .await
        .unwrap();
        assert_eq!(vm.variables.get("chosen").map(String::as_str), Some("@c1"));
        assert!(condition_is_supported("var_present:choice"));
        assert!(evaluate_condition("var_present:empty", &vm.variables).unwrap());
        assert!(!evaluate_condition("var_present:missing", &vm.variables).unwrap());
        assert!(!condition_is_supported("arbitrary expression"));
    }

    #[tokio::test]
    async fn bounds_foreach_expansion_and_parallel_effects() {
        let host = host(None);
        let mut vm = Vm::new();
        assert_eq!(
            vm.run_op(
                VmOp::Foreach {
                    items: (0..33).map(|i| i.to_string()).collect(),
                    var: "x".into(),
                    body: vec![]
                },
                &host
            )
            .await,
            Err(VmError::ForeachLimitExceeded)
        );
        assert_eq!(
            vm.run_op(
                VmOp::ParallelRead(vec![VmOp::Step(BrowserWorkflowStep::Click {
                    reference: "@c1".into(),
                    expected_outcome: "done".into()
                })]),
                &host
            )
            .await,
            Err(VmError::UnsafeParallelEffect)
        );
    }

    #[tokio::test]
    async fn nested_empty_foreach_loops_consume_bounded_control_work() {
        let host = host(None);
        let mut vm = Vm::new();
        let op = VmOp::Foreach {
            items: (0..32).map(|i| i.to_string()).collect(),
            var: "outer".into(),
            body: vec![VmOp::Foreach {
                items: (0..32).map(|i| i.to_string()).collect(),
                var: "inner".into(),
                body: vec![VmOp::Foreach {
                    items: (0..32).map(|i| i.to_string()).collect(),
                    var: "leaf".into(),
                    body: vec![],
                }],
            }],
        };

        assert_eq!(vm.run_op(op, &host).await, Err(VmError::WorkLimitExceeded));
        assert_eq!(vm.step_count, 0);
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }
}
