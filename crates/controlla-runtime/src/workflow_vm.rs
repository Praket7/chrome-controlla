//! Bounded async workflow executor. Browser hosts own dispatch, guards, budgets,
//! telemetry, and receipt classification; the VM owns control flow and retry rules.

use crate::browser_workflow::{BrowserWorkflowStep, CompiledBrowserWorkflow};
use crate::workflow::EffectClass;
use std::time::Instant;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

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
    RouteBudgetExceeded(crate::capability_router::RouteBudgetExceeded),
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

pub struct Vm {
    pub variables: BTreeMap<String, String>,
    pub step_count: usize,
    pub probe_count: usize,
    pub receipts: Vec<StepReceipt>,
    successful_mutations: usize,
    control_work: usize,
    route_budget: crate::capability_router::RouteBudget,
    route_started: Instant,
}

impl Default for Vm {
    fn default() -> Self {
        Self {
            variables: BTreeMap::new(),
            step_count: 0,
            probe_count: 0,
            receipts: Vec::new(),
            successful_mutations: 0,
            control_work: 0,
            route_budget: crate::capability_router::RouteBudget::new(
                crate::capability_router::RouteBudgetLimits {
                    internal_models: 0,
                    visible_observations: 0,
                    browser_commands: MAX_EXPANDED_STEPS as u32,
                    wall_deadline_ms: 120_000,
                    fallback_count: 1,
                },
            ),
            route_started: Instant::now(),
        }
    }
}

impl Vm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_route_budget(limits: crate::capability_router::RouteBudgetLimits) -> Self {
        Self {
            route_budget: crate::capability_router::RouteBudget::new(limits),
            route_started: Instant::now(),
            variables: BTreeMap::new(),
            step_count: 0,
            probe_count: 0,
            receipts: Vec::new(),
            successful_mutations: 0,
            control_work: 0,
        }
    }

    pub fn route_budget_usage(&self) -> crate::capability_router::RouteBudgetUsage {
        let mut usage = self.route_budget.usage();
        usage.elapsed_ms = self
            .route_started
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        usage
    }

    fn reserve_browser_command(&mut self) -> Result<(), VmError> {
        self.route_budget
            .check_deadline(
                self.route_started
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
            )
            .map_err(VmError::RouteBudgetExceeded)?;
        self.route_budget
            .begin_browser_command()
            .map_err(VmError::RouteBudgetExceeded)
    }

    fn ensure_browser_command_capacity(&mut self, count: usize) -> Result<(), VmError> {
        self.route_budget
            .check_deadline(
                self.route_started
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
            )
            .map_err(VmError::RouteBudgetExceeded)?;
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        if count
            > self
                .route_budget
                .limits
                .browser_commands
                .saturating_sub(self.route_budget.usage.browser_commands)
        {
            return Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::BrowserCommands,
            ));
        }
        Ok(())
    }

    fn reserve_fallback(&mut self) -> Result<(), VmError> {
        self.route_budget
            .check_deadline(
                self.route_started
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
            )
            .map_err(VmError::RouteBudgetExceeded)?;
        self.route_budget
            .begin_fallback()
            .map_err(VmError::RouteBudgetExceeded)
    }

    fn remaining_wall_budget(&self) -> std::time::Duration {
        let elapsed = self.route_started.elapsed();
        std::time::Duration::from_millis(self.route_budget.limits.wall_deadline_ms)
            .saturating_sub(elapsed)
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
                        self.reserve_fallback()?;
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
                    self.ensure_browser_command_capacity(steps.len())?;
                    let remaining = self.remaining_wall_budget();
                    let initial_dispatch_count = self.route_budget.usage.browser_commands;
                    let dispatch_count = Arc::new(AtomicU32::new(initial_dispatch_count));
                    let command_limit = self.route_budget.limits.browser_commands;
                    let variables = &self.variables;
                    let execution = tokio::time::timeout(
                        remaining,
                        futures_util::future::try_join_all(
                            steps
                                .into_iter()
                                .map(|step| {
                                    let dispatch_count = Arc::clone(&dispatch_count);
                                    async move {
                                        let mut current = dispatch_count.load(Ordering::SeqCst);
                                        loop {
                                            if current >= command_limit {
                                                return Err(VmError::RouteBudgetExceeded(
                                                    crate::capability_router::RouteBudgetExceeded::BrowserCommands,
                                                ));
                                            }
                                            match dispatch_count.compare_exchange(
                                                current,
                                                current + 1,
                                                Ordering::SeqCst,
                                                Ordering::SeqCst,
                                            ) {
                                                Ok(_) => break,
                                                Err(observed) => current = observed,
                                            }
                                        }
                                        host.execute(step, variables).await
                                    }
                                }),
                        ),
                    )
                    .await;
                    self.route_budget.usage.browser_commands =
                        dispatch_count.load(Ordering::SeqCst);
                    self.step_count += (self.route_budget.usage.browser_commands
                        - initial_dispatch_count) as usize;
                    let results = match execution {
                        Ok(Ok(results)) => results,
                        Ok(Err(error)) => return Err(error),
                        Err(_) => {
                            return Err(VmError::RouteBudgetExceeded(
                                crate::capability_router::RouteBudgetExceeded::WallDeadline,
                            ));
                        }
                    };
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
        self.reserve_browser_command()?;
        self.step_count += 1;
        let effect = step.effect();
        let remaining = self.remaining_wall_budget();
        let outcome = tokio::time::timeout(remaining, host.execute(step, &self.variables)).await;
        let receipt = match outcome {
            Ok(Ok(receipt)) => receipt,
            Ok(Err(error)) => {
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
            Err(_) => {
                let error = if effect == EffectClass::ReadOnly {
                    VmError::RouteBudgetExceeded(
                        crate::capability_router::RouteBudgetExceeded::WallDeadline,
                    )
                } else {
                    VmError::UnknownMutation(
                        "workflow wall deadline elapsed while command outcome was unknown".into(),
                    )
                };
                self.receipts.push(StepReceipt {
                    status: if effect == EffectClass::ReadOnly {
                        "failed"
                    } else {
                        "unknown"
                    }
                    .into(),
                    value: format!("{error:?}"),
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
        assert_eq!(vm.route_budget_usage().browser_commands, 2);
    }

    #[tokio::test]
    async fn route_budget_stops_before_dispatch_and_reports_only_invoked_commands() {
        let initial_host = host(None);
        let mut vm = Vm::with_route_budget(crate::capability_router::RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 0,
            browser_commands: 1,
            wall_deadline_ms: 120_000,
            fallback_count: 0,
        });
        let result = vm
            .run_sequence(
                vec![
                    VmOp::Step(read_step("first")),
                    VmOp::Step(read_step("second")),
                ],
                &initial_host,
            )
            .await;
        assert_eq!(
            result,
            Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::BrowserCommands
            ))
        );
        assert_eq!(initial_host.calls.load(Ordering::SeqCst), 1);
        assert_eq!(vm.route_budget_usage().browser_commands, 1);
        assert_eq!(vm.step_count, 1);

        let zero_budget_host = host(None);
        let mut blocked_vm = Vm::with_route_budget(crate::capability_router::RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 0,
            browser_commands: 0,
            wall_deadline_ms: 120_000,
            fallback_count: 0,
        });
        let blocked = blocked_vm
            .run_op(VmOp::Step(read_step("blocked")), &zero_budget_host)
            .await;
        assert_eq!(
            blocked,
            Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::BrowserCommands
            ))
        );
        assert_eq!(zero_budget_host.calls.load(Ordering::SeqCst), 0);
        assert_eq!(blocked_vm.route_budget_usage().browser_commands, 0);
        assert_eq!(blocked_vm.step_count, 0);
    }

    #[tokio::test]
    async fn parallel_budget_preflight_does_not_count_undispatched_steps() {
        let host = host(None);
        let mut vm = Vm::with_route_budget(crate::capability_router::RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 0,
            browser_commands: 1,
            wall_deadline_ms: 120_000,
            fallback_count: 0,
        });
        let result = vm
            .run_op(
                VmOp::ParallelRead(vec![
                    VmOp::Step(read_step("first")),
                    VmOp::Step(read_step("second")),
                ]),
                &host,
            )
            .await;
        assert_eq!(
            result,
            Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::BrowserCommands
            ))
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
        assert_eq!(vm.route_budget_usage().browser_commands, 0);
        assert_eq!(vm.step_count, 0);
    }

    #[tokio::test]
    async fn parallel_read_failure_counts_only_started_host_invocations() {
        let mut host = host(Some(VmError::ReadFailure("fixture read failed".into())));
        host.fail_after = 0;
        let mut vm = Vm::new();
        let result = vm
            .run_op(
                VmOp::ParallelRead(vec![
                    VmOp::Step(read_step("first")),
                    VmOp::Step(read_step("second")),
                ]),
                &host,
            )
            .await;
        assert_eq!(
            result,
            Err(VmError::ReadFailure("fixture read failed".into()))
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
        assert_eq!(vm.route_budget_usage().browser_commands, 1);
    }

    #[tokio::test]
    async fn parallel_failure_after_multiple_starts_counts_each_started_read() {
        let mut host = host(Some(VmError::ReadFailure("second read failed".into())));
        host.fail_after = 1;
        let mut vm = Vm::new();
        let result = vm
            .run_op(
                VmOp::ParallelRead(vec![
                    VmOp::Step(read_step("first")),
                    VmOp::Step(read_step("second")),
                ]),
                &host,
            )
            .await;
        assert_eq!(
            result,
            Err(VmError::ReadFailure("second read failed".into()))
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 2);
        assert_eq!(vm.route_budget_usage().browser_commands, 2);
        assert_eq!(vm.step_count, 2);
    }

    #[tokio::test]
    async fn parallel_timeout_counts_started_reads_and_steps() {
        struct SlowHost {
            calls: AtomicUsize,
        }
        impl BrowserVmHost for SlowHost {
            fn execute<'a>(
                &'a self,
                _step: BrowserWorkflowStep,
                _: &'a BTreeMap<String, String>,
            ) -> Pin<Box<dyn Future<Output = Result<StepReceipt, VmError>> + Send + 'a>>
            {
                Box::pin(async move {
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    Ok(StepReceipt {
                        status: "passed".into(),
                        value: "ok".into(),
                        variable_update: None,
                    })
                })
            }
        }

        let host = SlowHost {
            calls: AtomicUsize::new(0),
        };
        let mut vm = Vm::with_route_budget(crate::capability_router::RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 0,
            browser_commands: 2,
            wall_deadline_ms: 20,
            fallback_count: 0,
        });
        let result = vm
            .run_op(
                VmOp::ParallelRead(vec![
                    VmOp::Step(read_step("first")),
                    VmOp::Step(read_step("second")),
                ]),
                &host,
            )
            .await;
        assert_eq!(
            result,
            Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::WallDeadline
            ))
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 2);
        assert_eq!(vm.route_budget_usage().browser_commands, 2);
        assert_eq!(vm.step_count, 2);
    }

    #[tokio::test]
    async fn route_budget_usage_reports_elapsed_time_at_readout() {
        let vm = Vm::new();
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;
        assert!(vm.route_budget_usage().elapsed_ms >= 1);
    }

    #[tokio::test]
    async fn route_budget_bounds_fallback_before_running_fallback_steps() {
        let mut host = host(Some(VmError::NotDispatched(
            "known pre-dispatch failure".into(),
        )));
        host.fail_after = 0;
        let mut vm = Vm::with_route_budget(crate::capability_router::RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 0,
            browser_commands: 8,
            wall_deadline_ms: 120_000,
            fallback_count: 0,
        });
        let result = vm
            .run_op(
                VmOp::TryFallback {
                    primary: vec![VmOp::Step(read_step("primary"))],
                    fallback: vec![VmOp::Step(read_step("fallback"))],
                },
                &host,
            )
            .await;
        assert_eq!(
            result,
            Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::Fallbacks
            ))
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
        assert_eq!(vm.route_budget_usage().fallback_count, 0);
        assert_eq!(vm.route_budget_usage().browser_commands, 1);
    }

    #[tokio::test]
    async fn nested_fallback_shares_invocation_budget_with_enclosing_workflow() {
        struct FailTwiceHost {
            calls: AtomicUsize,
        }
        impl BrowserVmHost for FailTwiceHost {
            fn execute<'a>(
                &'a self,
                _step: BrowserWorkflowStep,
                _: &'a BTreeMap<String, String>,
            ) -> Pin<Box<dyn Future<Output = Result<StepReceipt, VmError>> + Send + 'a>>
            {
                Box::pin(async move {
                    let call = self.calls.fetch_add(1, Ordering::SeqCst);
                    if call == 0 || call == 2 {
                        return Err(VmError::NotDispatched("known pre-dispatch failure".into()));
                    }
                    Ok(StepReceipt {
                        status: "passed".into(),
                        value: "ok".into(),
                        variable_update: None,
                    })
                })
            }
        }
        let host = FailTwiceHost {
            calls: AtomicUsize::new(0),
        };
        let mut vm = Vm::with_route_budget(crate::capability_router::RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 0,
            browser_commands: 3,
            wall_deadline_ms: 120_000,
            fallback_count: 1,
        });

        let result = vm
            .run_sequence(
                vec![
                    VmOp::TryFallback {
                        primary: vec![VmOp::Step(read_step("primary"))],
                        fallback: vec![VmOp::Step(read_step("fallback"))],
                    },
                    VmOp::TryFallback {
                        primary: vec![VmOp::Step(read_step("second-primary"))],
                        fallback: vec![VmOp::Step(read_step("second-fallback"))],
                    },
                ],
                &host,
            )
            .await;

        assert_eq!(
            result,
            Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::Fallbacks
            ))
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 3);
        assert_eq!(vm.route_budget_usage().browser_commands, 3);
        assert_eq!(vm.route_budget_usage().fallback_count, 1);
        assert_eq!(vm.route_budget_usage().internal_models, 0);
        assert_eq!(vm.route_budget_usage().visible_observations, 0);
    }

    #[tokio::test]
    async fn expired_wall_budget_prevents_the_next_dispatch() {
        let host = host(None);
        let mut vm = Vm::with_route_budget(crate::capability_router::RouteBudgetLimits {
            internal_models: 0,
            visible_observations: 0,
            browser_commands: 8,
            wall_deadline_ms: 1,
            fallback_count: 0,
        });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let result = vm.run_op(VmOp::Step(read_step("expired")), &host).await;
        assert_eq!(
            result,
            Err(VmError::RouteBudgetExceeded(
                crate::capability_router::RouteBudgetExceeded::WallDeadline
            ))
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
        assert_eq!(vm.route_budget_usage().browser_commands, 0);
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
