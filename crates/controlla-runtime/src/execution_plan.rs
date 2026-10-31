//! One bounded execution-plan envelope shared by task programs, workflows,
//! qualified skills and learned operations.
//!
//! The plan is a planning and evidence contract, not a second executor.
//! `TaskProgram` still lowers into the existing workflow VM, and a skill
//! workflow still uses the public browser-workflow compiler. What becomes
//! shared is the operation contract (effect, preconditions, postconditions,
//! cleanup, budget), the routing decision, and the read schedule attached to
//! every producer's receipt.

use crate::browser_workflow::{
    BrowserWorkflowDefinition, CompiledBrowserWorkflow, compile_browser_workflow,
};
use crate::capability_router::{
    ExecutionCapabilities, ExecutionRouteDecision, ExecutionTier, ExpectedRouteCost,
    route_execution,
};
use crate::page_action_memory::{ActionMemoryEntry, ReplayPlan};
use crate::skills::SkillDefinition;
use crate::task_program::{
    CompiledTaskProgram, TaskProgram, TaskProgramLimits, TaskStep, TaskStepKind,
    compile_task_program,
};
use crate::v3_tasks::TaskPrimitive;
use crate::workflow::EffectClass;
use crate::workflow_dag::parallel_wave_is_worthwhile;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_CONTRACT_CONDITIONS: usize = 32;
pub const DEFAULT_WALL_DEADLINE_MS: u64 = 30_000;
/// Conservative policy estimate, not a measured site latency. The gate only
/// groups independent reads when projected serial savings exceed setup cost.
pub const POLICY_READ_STEP_MS: u64 = 120;
pub const POLICY_PARALLEL_SETUP_MS: u64 = 250;
pub const MIN_PARALLEL_READS: usize = 4;
pub const MAX_PARALLEL_READS_PER_GROUP: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSource {
    TaskProgram,
    BrowserTask,
    Workflow,
    Skill,
    LearnedOperation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEffect {
    ReadOnly,
    Reversible,
    External,
    AuthorityBoundary,
    Unknown,
}

impl PlanEffect {
    /// Navigation changes the document but does not mutate user data, so it is
    /// not a mutation for routing, verification, or cleanup purposes.
    pub fn is_mutation(self) -> bool {
        matches!(self, Self::Reversible | Self::External | Self::Unknown)
    }

    pub fn is_read_only(self) -> bool {
        !self.is_mutation()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRequirement {
    None,
    Postcondition,
    IndependentObservation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRoutingPolicy {
    QualifiedFirst,
    WebMcpFirst,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanBudget {
    pub max_steps: usize,
    pub max_browser_commands: u32,
    pub max_parallel_reads: usize,
    pub max_speculative_reads: usize,
    pub wall_deadline_ms: u64,
}

impl PlanBudget {
    pub fn for_steps(steps: usize) -> Self {
        let steps = steps.max(1);
        Self {
            max_steps: steps,
            max_browser_commands: u32::try_from(steps.saturating_mul(4))
                .unwrap_or(u32::MAX)
                .clamp(8, 256),
            max_parallel_reads: MAX_PARALLEL_READS_PER_GROUP,
            max_speculative_reads: 4,
            wall_deadline_ms: DEFAULT_WALL_DEADLINE_MS,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationContract {
    pub effect: PlanEffect,
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
    pub cleanup: Vec<String>,
    pub verification: VerificationRequirement,
    pub budget: PlanBudget,
}

impl OperationContract {
    pub fn complete(&self) -> bool {
        if !self.effect.is_mutation() {
            return true;
        }
        self.verification != VerificationRequirement::None && !self.postconditions.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanBody {
    TaskProgram(CompiledTaskProgram),
    BrowserWorkflow(CompiledBrowserWorkflow),
    Primitive(TaskPrimitive),
    LearnedOperation(LearnedOperationPlan),
}

impl PlanBody {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TaskProgram(_) => "task_program",
            Self::BrowserWorkflow(_) => "browser_workflow",
            Self::Primitive(_) => "task_primitive",
            Self::LearnedOperation(_) => "learned_operation",
        }
    }

    pub fn expanded_steps(&self) -> usize {
        match self {
            Self::TaskProgram(program) => program.expanded_steps,
            Self::BrowserWorkflow(workflow) => workflow.steps.len(),
            Self::Primitive(primitive) => primitive.expand().map(|steps| steps.len()).unwrap_or(0),
            Self::LearnedOperation(_) => 1,
        }
    }

    pub fn has_control_flow(&self) -> bool {
        matches!(self, Self::TaskProgram(program) if program.has_control_flow)
    }

    pub fn effect(&self) -> PlanEffect {
        match self {
            Self::TaskProgram(program) => task_program_effect(program),
            Self::BrowserWorkflow(workflow) => browser_workflow_effect(workflow),
            Self::Primitive(primitive) => primitive_effect(primitive),
            Self::LearnedOperation(operation) => operation.effect,
        }
    }

    pub fn has_verification(&self) -> bool {
        match self {
            Self::TaskProgram(program) => program_has_verification(&program.steps),
            Self::BrowserWorkflow(workflow) => workflow.steps.iter().any(|step| {
                matches!(
                    step,
                    crate::browser_workflow::BrowserWorkflowStep::Assert { .. }
                        | crate::browser_workflow::BrowserWorkflowStep::Verify { .. }
                )
            }),
            Self::Primitive(primitive) => match primitive {
                TaskPrimitive::FormDraft { .. }
                | TaskPrimitive::Mutation { .. }
                | TaskPrimitive::Download { .. } => true,
                TaskPrimitive::Upload {
                    acceptance,
                    persisted,
                    ..
                }
                | TaskPrimitive::DownloadUpload {
                    acceptance,
                    persisted,
                    ..
                } => acceptance.is_some() || persisted.is_some(),
                TaskPrimitive::Repeat { .. } => true,
                TaskPrimitive::Research { .. } => false,
            },
            Self::LearnedOperation(operation) => operation.has_postcondition,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRoute {
    pub tier: ExecutionTier,
    pub reason: &'static str,
    pub predicted_model_calls: u8,
    pub expected_cost: ExpectedRouteCost,
    #[serde(default)]
    pub webmcp_first: bool,
    #[serde(default)]
    pub available_tiers: Vec<ExecutionTier>,
    #[serde(default)]
    pub selected_by_history: bool,
}

impl PlanRoute {
    fn from_decision(
        decision: ExecutionRouteDecision,
        webmcp_first: bool,
        available_tiers: Vec<ExecutionTier>,
    ) -> Self {
        Self {
            tier: decision.selected,
            reason: decision.reason,
            predicted_model_calls: decision.predicted_model_calls,
            expected_cost: decision.expected_cost,
            webmcp_first,
            available_tiers,
            selected_by_history: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSchedule {
    pub read_groups: usize,
    pub read_steps: usize,
    pub scheduled_groups: usize,
    pub estimated_saved_ms: u64,
}

/// A learned operation is a bounded, non-secret replay description. It never
/// authorizes a blind dispatch: the live target is re-resolved and the stored
/// postcondition is observed again.
#[derive(Clone, Debug, PartialEq)]
pub struct LearnedOperationPlan {
    pub action: String,
    pub replay: &'static str,
    pub effect: PlanEffect,
    pub has_postcondition: bool,
}

impl LearnedOperationPlan {
    fn from_entry(entry: &ActionMemoryEntry) -> Self {
        let (replay, effect) = match entry.replay.as_ref() {
            Some(ReplayPlan::Extract { .. }) => ("extract", PlanEffect::ReadOnly),
            Some(ReplayPlan::Navigate { .. }) => ("navigate", PlanEffect::AuthorityBoundary),
            Some(ReplayPlan::Click { .. }) => ("click", PlanEffect::External),
            Some(ReplayPlan::ClickHint { .. }) => ("click_hint", PlanEffect::External),
            Some(ReplayPlan::Fill { .. }) => ("fill", PlanEffect::External),
            Some(ReplayPlan::FillHint { .. }) => ("fill_hint", PlanEffect::External),
            Some(ReplayPlan::Type { .. }) => ("type", PlanEffect::External),
            Some(ReplayPlan::Select { .. }) => ("select", PlanEffect::External),
            Some(ReplayPlan::SelectHint { .. }) => ("select_hint", PlanEffect::External),
            Some(ReplayPlan::Tool { .. }) => ("tool", PlanEffect::Unknown),
            None => ("none", PlanEffect::Unknown),
        };
        Self {
            action: entry.action.clone(),
            replay,
            effect,
            has_postcondition: !entry.expected_postcondition.is_null(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionPlan {
    pub source: PlanSource,
    pub body: PlanBody,
    pub contract: OperationContract,
    pub route: PlanRoute,
    pub schedule: PlanSchedule,
}

impl ExecutionPlan {
    pub fn from_task_program(
        program: &TaskProgram,
        source: PlanSource,
        capabilities: ExecutionCapabilities,
        policy: PlanRoutingPolicy,
    ) -> Result<Self, String> {
        let compiled = compile_task_program(program, TaskProgramLimits::default())?;
        let contract = contract_for_task_program(&compiled);
        let route = plan_route(&contract, capabilities, policy);
        Ok(Self {
            source,
            body: PlanBody::TaskProgram(compiled),
            contract,
            route,
            schedule: PlanSchedule::default(),
        })
    }

    pub fn from_browser_workflow(
        definition: &BrowserWorkflowDefinition,
        source: PlanSource,
        capabilities: ExecutionCapabilities,
        policy: PlanRoutingPolicy,
    ) -> Result<Self, String> {
        let compiled = compile_browser_workflow(definition.clone())?;
        let body = PlanBody::BrowserWorkflow(compiled);
        let contract = contract_for_body(&body, Vec::new(), Vec::new());
        let route = plan_route(&contract, capabilities, policy);
        Ok(Self {
            source,
            body,
            contract,
            route,
            schedule: PlanSchedule::default(),
        })
    }

    /// Skills persist their executable workflow as JSON. Read-only research
    /// skills use the public browser-workflow shape; bounded task programs are
    /// also accepted so a learned operation can share the same envelope.
    pub fn from_skill_definition(
        skill: &SkillDefinition,
        source: PlanSource,
        capabilities: ExecutionCapabilities,
        policy: PlanRoutingPolicy,
    ) -> Result<Self, String> {
        let body = if let Ok(definition) =
            serde_json::from_value::<BrowserWorkflowDefinition>(skill.workflow.clone())
        {
            PlanBody::BrowserWorkflow(
                compile_browser_workflow(definition)
                    .map_err(|error| format!("unsafe skill workflow: {error}"))?,
            )
        } else {
            let program: TaskProgram = serde_json::from_value(skill.workflow.clone())
                .map_err(|error| format!("skill workflow is not a supported plan body: {error}"))?;
            PlanBody::TaskProgram(
                compile_task_program(&program, TaskProgramLimits::default())
                    .map_err(|error| format!("invalid skill task program: {error}"))?,
            )
        };
        let contract = contract_for_body(
            &body,
            bounded_conditions(&skill.preconditions),
            bounded_conditions(&skill.postconditions),
        );
        let route = plan_route(&contract, capabilities, policy);
        Ok(Self {
            source,
            body,
            contract,
            route,
            schedule: PlanSchedule::default(),
        })
    }

    /// Learned operation memory is a hint source, not a trusted executor. The
    /// plan records the recorded replay shape and the re-verification obligation
    /// so it uses the same contract as programs, workflows, and skills.
    pub fn from_learned_operation(entry: &ActionMemoryEntry) -> Self {
        let body = PlanBody::LearnedOperation(LearnedOperationPlan::from_entry(entry));
        let contract = contract_for_body(&body, Vec::new(), Vec::new());
        let route = plan_route(
            &contract,
            ExecutionCapabilities {
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            PlanRoutingPolicy::QualifiedFirst,
        );
        Self {
            source: PlanSource::LearnedOperation,
            body,
            contract,
            route,
            schedule: PlanSchedule::default(),
        }
    }

    /// A task primitive is already compiled by its own bounded expander. This
    /// constructor wraps that executable capsule in the shared contract and
    /// route so primitive, program, workflow, and skill receipts agree on the
    /// same effect, verification, budget, and route fields.
    pub fn from_primitive(
        primitive: TaskPrimitive,
        decision: ExecutionRouteDecision,
        available_tiers: Vec<ExecutionTier>,
    ) -> Self {
        let body = PlanBody::Primitive(primitive);
        let effect = body.effect();
        let read_steps = if effect == PlanEffect::ReadOnly {
            body.expanded_steps()
        } else {
            0
        };
        let contract = contract_for_body(&body, Vec::new(), Vec::new());
        let mut route = PlanRoute::from_decision(decision, false, available_tiers);
        route.webmcp_first = route.tier == ExecutionTier::WebMcpOrSiteTool;
        Self {
            source: PlanSource::BrowserTask,
            body,
            contract,
            route,
            schedule: PlanSchedule {
                read_groups: 0,
                read_steps,
                scheduled_groups: 0,
                estimated_saved_ms: 0,
            },
        }
    }

    /// Compile a caller task program and return both the scheduled executable
    /// program and its shared plan envelope. This keeps the scheduling policy
    /// in the module that owns it instead of the MCP facade.
    pub fn for_task_program(program: &TaskProgram) -> Result<(TaskProgram, Self), String> {
        let (scheduled, schedule) = schedule_task_program(program);
        let plan = Self::from_task_program(
            &scheduled,
            PlanSource::TaskProgram,
            ExecutionCapabilities {
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            PlanRoutingPolicy::QualifiedFirst,
        )?
        .with_schedule(schedule);
        Ok((scheduled, plan))
    }

    /// Compile a skill's read-only browser workflow into the shared envelope.
    /// Skill registration and replay use the same constructor so a learned
    /// operation cannot quietly diverge from a manually registered workflow.
    pub fn for_skill_workflow(workflow: &BrowserWorkflowDefinition) -> Result<Self, String> {
        Self::from_browser_workflow(
            workflow,
            PlanSource::Skill,
            ExecutionCapabilities {
                qualified_skill: true,
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            PlanRoutingPolicy::QualifiedFirst,
        )
    }

    /// Rebuild a skill replay plan from the persisted workflow JSON. The
    /// constructor fails closed when the stored body is no longer supported.
    pub fn for_skill_replay(workflow: &Value) -> Option<Self> {
        serde_json::from_value::<BrowserWorkflowDefinition>(workflow.clone())
            .ok()
            .and_then(|workflow| Self::for_skill_workflow(&workflow).ok())
    }

    /// Attach the scheduled-read decision produced by `schedule_task_program`.
    pub fn with_schedule(mut self, schedule: PlanSchedule) -> Self {
        self.schedule = schedule;
        self
    }

    /// Record the route that actually served the call when a qualified replay
    /// replaced the statically selected tier.
    pub fn set_selected_tier(
        &mut self,
        tier: ExecutionTier,
        reason: &'static str,
        predicted_model_calls: u8,
    ) {
        self.route.tier = tier;
        self.route.reason = reason;
        self.route.predicted_model_calls = predicted_model_calls;
        self.route.webmcp_first = tier == ExecutionTier::WebMcpOrSiteTool;
        self.route.expected_cost = ExpectedRouteCost::for_tier(
            tier,
            self.route.expected_cost.cold_start > 0,
            self.route.expected_cost.elapsed_ms,
        );
    }

    pub fn with_selected_tier(
        mut self,
        tier: ExecutionTier,
        reason: &'static str,
        predicted_model_calls: u8,
    ) -> Self {
        self.set_selected_tier(tier, reason, predicted_model_calls);
        self
    }

    /// Mark a route chosen from a qualified empirical cohort rather than the
    /// static ladder. The caller must have already verified the cohort.
    pub fn with_empirical_route(
        mut self,
        tier: ExecutionTier,
        expected_cost: ExpectedRouteCost,
    ) -> Self {
        self.route.tier = tier;
        self.route.webmcp_first = tier == ExecutionTier::WebMcpOrSiteTool;
        self.route.expected_cost = expected_cost;
        self.route.reason = "qualified empirical p95 plus explicit route overhead";
        self.route.predicted_model_calls = if matches!(
            tier,
            ExecutionTier::QualifiedSkill
                | ExecutionTier::WebMcpOrSiteTool
                | ExecutionTier::SpeculativeRead
                | ExecutionTier::SemanticProgram
        ) {
            0
        } else {
            1
        };
        self.route.selected_by_history = true;
        self
    }

    pub fn receipt(&self) -> Value {
        json!({
            "source": self.source,
            "body": self.body.kind(),
            "steps": self.body.expanded_steps(),
            "effect": self.contract.effect,
            "contract": self.contract,
            "contract_complete": self.contract.complete(),
            "route": self.route,
            "schedule": self.schedule,
        })
    }
}

fn bounded_conditions(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter(|value| !value.trim().is_empty() && value.len() <= 256)
        .take(MAX_CONTRACT_CONDITIONS)
        .cloned()
        .collect()
}

fn contract_for_task_program(program: &CompiledTaskProgram) -> OperationContract {
    let body = PlanBody::TaskProgram(program.clone());
    contract_for_body(&body, Vec::new(), Vec::new())
}

fn contract_for_body(
    body: &PlanBody,
    preconditions: Vec<String>,
    postconditions: Vec<String>,
) -> OperationContract {
    let effect = body.effect();
    let has_verification = body.has_verification();
    let mut preconditions = if preconditions.is_empty() {
        vec![
            "live_session_and_document_identity".to_owned(),
            "bounded_program".to_owned(),
        ]
    } else {
        preconditions
    };
    if effect == PlanEffect::External {
        preconditions.push("fresh_target_reference_before_dispatch".to_owned());
    }
    let mut postconditions = postconditions;
    if has_verification && postconditions.is_empty() {
        postconditions.push("typed_postcondition_observed".to_owned());
    }
    let verification = if has_verification || !postconditions.is_empty() {
        VerificationRequirement::Postcondition
    } else if effect == PlanEffect::External {
        VerificationRequirement::IndependentObservation
    } else {
        VerificationRequirement::None
    };
    let cleanup = if effect.is_mutation() {
        vec!["release_dispatch_lease".to_owned()]
    } else {
        Vec::new()
    };
    OperationContract {
        effect,
        preconditions,
        postconditions,
        cleanup,
        verification,
        budget: PlanBudget::for_steps(body.expanded_steps()),
    }
}

fn plan_route(
    contract: &OperationContract,
    capabilities: ExecutionCapabilities,
    policy: PlanRoutingPolicy,
) -> PlanRoute {
    let read_only = !contract.effect.is_mutation();
    let available = route_candidates(capabilities, read_only);
    let webmcp_first =
        policy == PlanRoutingPolicy::WebMcpFirst && read_only && capabilities.site_tool;
    let decision = if webmcp_first {
        ExecutionRouteDecision {
            selected: ExecutionTier::WebMcpOrSiteTool,
            reason: "a runtime-qualified WebMCP/site tool is preferred for this read-only plan",
            predicted_model_calls: 0,
            expected_cost: ExpectedRouteCost::for_tier(ExecutionTier::WebMcpOrSiteTool, false, 0),
        }
    } else {
        route_execution(capabilities, read_only)
    };
    PlanRoute::from_decision(decision, webmcp_first, available)
}

/// Map a bounded task primitive to the capabilities the static router can
/// consume. This is shared by the executor and by tests so both see the same
/// route decision.
pub fn task_route_signals(
    task: &TaskPrimitive,
    expanded_step_count: usize,
    qualified_skill: bool,
    site_tool: bool,
    speculative_read: bool,
) -> ExecutionCapabilities {
    let visual_required = match task {
        TaskPrimitive::Research { goal, .. } => {
            goal.as_deref().is_some_and(task_needs_visual_evidence)
        }
        _ => false,
    };
    // TaskPrimitive::expand admits only bounded operations supported by the V3
    // executor. A zero-step program has no semantic implementation and must
    // fall through instead of claiming a deterministic route.
    let semantic_program = expanded_step_count > 0 && !visual_required;
    ExecutionCapabilities {
        qualified_skill: qualified_skill && !visual_required,
        site_tool: site_tool && !visual_required,
        speculative_read: speculative_read && !visual_required,
        semantic_program,
        semantic_confidence: 0.0,
        targeted_visual: visual_required,
    }
}

pub fn route_candidates(
    capabilities: ExecutionCapabilities,
    read_only: bool,
) -> Vec<ExecutionTier> {
    let mut tiers = Vec::new();
    if read_only && capabilities.qualified_skill {
        tiers.push(ExecutionTier::QualifiedSkill);
    }
    if capabilities.site_tool {
        tiers.push(ExecutionTier::WebMcpOrSiteTool);
    }
    if read_only && capabilities.speculative_read {
        tiers.push(ExecutionTier::SpeculativeRead);
    }
    if capabilities.semantic_program {
        tiers.push(ExecutionTier::SemanticProgram);
    }
    if capabilities.semantic_confidence >= 0.8 {
        tiers.push(ExecutionTier::SemanticPlanner);
    }
    if capabilities.targeted_visual {
        tiers.push(ExecutionTier::TargetedVisual);
    }
    if tiers.is_empty() {
        tiers.push(ExecutionTier::FullVisionFallback);
    }
    tiers
}

/// A task goal that needs visual evidence must stay on the targeted-visual
/// path even when cheaper read routes are available.
pub fn task_needs_visual_evidence(goal: &str) -> bool {
    let terms = goal
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .collect::<std::collections::BTreeSet<_>>();
    [
        "layout", "chart", "graph", "map", "canvas", "diagram", "overlap", "position", "visual",
    ]
    .iter()
    .any(|term| terms.contains(*term))
}

fn task_program_effect(program: &CompiledTaskProgram) -> PlanEffect {
    let mut effect = PlanEffect::ReadOnly;
    collect_task_effect(&program.steps, &mut effect);
    effect
}

fn collect_task_effect(steps: &[TaskStep], effect: &mut PlanEffect) {
    for step in steps {
        let next = match step.kind {
            TaskStepKind::Navigate => PlanEffect::AuthorityBoundary,
            TaskStepKind::Click | TaskStepKind::Press => PlanEffect::External,
            TaskStepKind::Fill
            | TaskStepKind::Type
            | TaskStepKind::Select
            | TaskStepKind::Scroll
            | TaskStepKind::ScrollTo
            | TaskStepKind::Hover
            | TaskStepKind::Drag => PlanEffect::Reversible,
            TaskStepKind::Observe
            | TaskStepKind::Find
            | TaskStepKind::WaitFor
            | TaskStepKind::Extract
            | TaskStepKind::Assert
            | TaskStepKind::Verify => PlanEffect::ReadOnly,
            TaskStepKind::If
            | TaskStepKind::Foreach
            | TaskStepKind::TryFallback
            | TaskStepKind::ParallelRead => PlanEffect::ReadOnly,
        };
        *effect = effect.max_by_risk(next);
        collect_task_effect(&step.steps, effect);
        collect_task_effect(&step.fallback, effect);
    }
}

impl PlanEffect {
    fn max_by_risk(self, next: Self) -> Self {
        let rank = |effect: Self| match effect {
            Self::ReadOnly => 0,
            Self::Reversible => 1,
            Self::AuthorityBoundary => 2,
            Self::External => 3,
            Self::Unknown => 4,
        };
        if rank(next) > rank(self) { next } else { self }
    }
}

fn browser_workflow_effect(workflow: &CompiledBrowserWorkflow) -> PlanEffect {
    let mut effect = PlanEffect::ReadOnly;
    for step_effect in &workflow.effects {
        let next = match step_effect {
            EffectClass::ReadOnly => PlanEffect::ReadOnly,
            EffectClass::Reversible => PlanEffect::Reversible,
            EffectClass::External => PlanEffect::External,
            EffectClass::AuthorityBoundary => PlanEffect::AuthorityBoundary,
            EffectClass::Unknown => PlanEffect::Unknown,
        };
        effect = effect.max_by_risk(next);
    }
    effect
}

fn primitive_effect(primitive: &TaskPrimitive) -> PlanEffect {
    match primitive {
        TaskPrimitive::Research { .. } => PlanEffect::ReadOnly,
        TaskPrimitive::FormDraft { .. } => PlanEffect::Reversible,
        TaskPrimitive::Mutation { .. }
        | TaskPrimitive::Upload { .. }
        | TaskPrimitive::Download { .. }
        | TaskPrimitive::DownloadUpload { .. } => PlanEffect::External,
        TaskPrimitive::Repeat { action, .. } => match action {
            crate::v3_tasks::RepeatAction::VerifyVisible => PlanEffect::ReadOnly,
            crate::v3_tasks::RepeatAction::Click => PlanEffect::External,
        },
    }
}

fn program_has_verification(steps: &[TaskStep]) -> bool {
    steps.iter().any(|step| {
        matches!(step.kind, TaskStepKind::Assert | TaskStepKind::Verify)
            || matches!(step.kind, TaskStepKind::Click | TaskStepKind::Press)
                && step.predicate.is_some()
            || matches!(
                step.kind,
                TaskStepKind::Fill | TaskStepKind::Type | TaskStepKind::Select
            ) && step
                .expected_value
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            || program_has_verification(&step.steps)
            || program_has_verification(&step.fallback)
    })
}

/// Group consecutive independent observation/extraction steps into a bounded
/// parallel-read wave when the policy estimate says serial work costs more than
/// the setup. Steps that save a variable are never grouped because their order
/// can affect later control flow.
pub fn schedule_task_program(program: &TaskProgram) -> (TaskProgram, PlanSchedule) {
    let mut schedule = PlanSchedule::default();
    let mut steps = Vec::with_capacity(program.steps.len());
    let mut pending = Vec::new();
    for step in &program.steps {
        if is_independent_read(step) {
            pending.push(step.clone());
            continue;
        }
        flush_read_group(&mut pending, &mut steps, &mut schedule);
        steps.push(step.clone());
    }
    flush_read_group(&mut pending, &mut steps, &mut schedule);
    (TaskProgram { steps }, schedule)
}

fn is_independent_read(step: &TaskStep) -> bool {
    matches!(step.kind, TaskStepKind::Observe | TaskStepKind::Extract)
        && step.save_as.is_none()
        && step.reference.is_none()
        && step.target.is_none()
}

fn flush_read_group(
    pending: &mut Vec<TaskStep>,
    steps: &mut Vec<TaskStep>,
    schedule: &mut PlanSchedule,
) {
    while pending.len() >= MIN_PARALLEL_READS {
        let width = pending.len().min(MAX_PARALLEL_READS_PER_GROUP);
        let group = pending.drain(..width).collect::<Vec<_>>();
        schedule.read_groups += 1;
        schedule.read_steps += group.len();
        let estimates = vec![POLICY_READ_STEP_MS; group.len()];
        if parallel_wave_is_worthwhile(&estimates, POLICY_PARALLEL_SETUP_MS) {
            schedule.scheduled_groups += 1;
            schedule.estimated_saved_ms = schedule
                .estimated_saved_ms
                .saturating_add(estimates.iter().copied().sum::<u64>() - POLICY_READ_STEP_MS);
            steps.push(TaskStep {
                kind: TaskStepKind::ParallelRead,
                steps: group,
                ..TaskStep::default()
            });
        } else {
            steps.extend(group);
        }
    }
    steps.append(pending);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_workflow::BrowserWorkflowStep;
    use crate::page_action_memory::{ActionMemoryEntry, ReplayPlan, VerifyPredicate};
    use crate::skills::SKILL_SCHEMA_VERSION;
    use crate::task_program::TaskStep;

    fn observe() -> TaskStep {
        TaskStep {
            kind: TaskStepKind::Observe,
            ..TaskStep::default()
        }
    }

    fn extract(selector: &str) -> TaskStep {
        TaskStep {
            kind: TaskStepKind::Extract,
            selector: Some(selector.into()),
            ..TaskStep::default()
        }
    }

    fn task_program(steps: Vec<TaskStep>) -> TaskProgram {
        TaskProgram { steps }
    }

    fn capabilities() -> ExecutionCapabilities {
        ExecutionCapabilities {
            semantic_program: true,
            ..ExecutionCapabilities::default()
        }
    }

    #[test]
    fn task_program_contract_reports_mutation_and_verification() {
        let program = task_program(vec![
            observe(),
            TaskStep {
                kind: TaskStepKind::Click,
                reference: Some("@c1".into()),
                ..TaskStep::default()
            },
            TaskStep {
                kind: TaskStepKind::Verify,
                predicate: Some(json!({"kind":"text","reference":"@c1","equals":"Saved"})),
                ..TaskStep::default()
            },
        ]);
        let plan = ExecutionPlan::from_task_program(
            &program,
            PlanSource::TaskProgram,
            capabilities(),
            PlanRoutingPolicy::QualifiedFirst,
        )
        .unwrap();
        assert_eq!(plan.contract.effect, PlanEffect::External);
        assert_eq!(
            plan.contract.verification,
            VerificationRequirement::Postcondition
        );
        assert!(plan.contract.complete());
        assert_eq!(plan.route.tier, ExecutionTier::SemanticProgram);
        assert_eq!(plan.route.predicted_model_calls, 0);
    }

    #[test]
    fn mutations_without_a_postcondition_are_reported_incomplete() {
        let program = task_program(vec![TaskStep {
            kind: TaskStepKind::Click,
            reference: Some("@c1".into()),
            ..TaskStep::default()
        }]);
        let plan = ExecutionPlan::from_task_program(
            &program,
            PlanSource::TaskProgram,
            capabilities(),
            PlanRoutingPolicy::QualifiedFirst,
        )
        .unwrap();
        assert_eq!(plan.contract.effect, PlanEffect::External);
        assert_eq!(
            plan.contract.verification,
            VerificationRequirement::IndependentObservation
        );
        assert!(!plan.contract.complete());

        let verified = ExecutionPlan::from_task_program(
            &task_program(vec![TaskStep {
                kind: TaskStepKind::Click,
                reference: Some("@c1".into()),
                predicate: Some(json!({"kind":"text","reference":"@c1","equals":"Saved"})),
                ..TaskStep::default()
            }]),
            PlanSource::TaskProgram,
            capabilities(),
            PlanRoutingPolicy::QualifiedFirst,
        )
        .unwrap();
        assert!(verified.contract.complete());
    }

    #[test]
    fn read_only_programs_do_not_require_mutation_cleanup() {
        let plan = ExecutionPlan::from_task_program(
            &task_program(vec![observe(), extract("main")]),
            PlanSource::TaskProgram,
            capabilities(),
            PlanRoutingPolicy::QualifiedFirst,
        )
        .unwrap();
        assert_eq!(plan.contract.effect, PlanEffect::ReadOnly);
        assert!(plan.contract.cleanup.is_empty());
        assert!(plan.contract.complete());
    }

    #[test]
    fn browser_workflow_and_skill_share_the_same_contract() {
        let workflow = BrowserWorkflowDefinition {
            steps: vec![
                BrowserWorkflowStep::Navigate {
                    url: "https://example.test/".into(),
                },
                BrowserWorkflowStep::Extract {
                    selector: "main".into(),
                    max_items: 8,
                    max_bytes: 4096,
                },
                BrowserWorkflowStep::Verify {
                    predicate: crate::browser_workflow::BrowserWorkflowPredicate::Exists {
                        reference: "@c1".into(),
                    },
                },
            ],
        };
        let plan = ExecutionPlan::from_browser_workflow(
            &workflow,
            PlanSource::Workflow,
            capabilities(),
            PlanRoutingPolicy::QualifiedFirst,
        )
        .unwrap();
        assert_eq!(plan.contract.effect, PlanEffect::AuthorityBoundary);
        assert_eq!(
            plan.contract.verification,
            VerificationRequirement::Postcondition
        );
        assert_eq!(plan.route.tier, ExecutionTier::SemanticProgram);

        let skill = SkillDefinition {
            skill_id: "skill-1".into(),
            schema_version: SKILL_SCHEMA_VERSION,
            site_scope: "https://example.test".into(),
            intent_fingerprint: "intent".into(),
            account_identity_fingerprint: None,
            parameter_names: vec![],
            preconditions: vec!["signed_in".into()],
            workflow: serde_json::to_value(workflow).unwrap(),
            postconditions: vec!["url_unchanged".into()],
            failure_guards: vec![],
            structural_signatures: vec!["signature".into()],
            version: 1,
        };
        let plan = ExecutionPlan::from_skill_definition(
            &skill,
            PlanSource::Skill,
            ExecutionCapabilities {
                qualified_skill: true,
                ..capabilities()
            },
            PlanRoutingPolicy::QualifiedFirst,
        )
        .unwrap();
        assert_eq!(plan.source, PlanSource::Skill);
        assert_eq!(plan.contract.preconditions, vec!["signed_in"]);
        assert_eq!(plan.contract.postconditions, vec!["url_unchanged"]);
    }

    #[test]
    fn webmcp_first_prefers_a_verified_site_tool_over_other_read_routes() {
        let plan = ExecutionPlan::from_task_program(
            &task_program(vec![observe(), extract("main"), observe(), extract("body")]),
            PlanSource::TaskProgram,
            ExecutionCapabilities {
                qualified_skill: true,
                site_tool: true,
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            PlanRoutingPolicy::WebMcpFirst,
        )
        .unwrap();
        assert_eq!(plan.route.tier, ExecutionTier::WebMcpOrSiteTool);
        assert!(plan.route.webmcp_first);
        assert_eq!(plan.route.predicted_model_calls, 0);
    }

    #[test]
    fn scheduled_reads_group_only_independent_observations() {
        let program = task_program(vec![
            observe(),
            extract("main"),
            observe(),
            extract("body"),
            observe(),
        ]);
        let (scheduled, report) = schedule_task_program(&program);
        assert_eq!(scheduled.steps.len(), 1);
        assert_eq!(scheduled.steps[0].kind, TaskStepKind::ParallelRead);
        assert_eq!(scheduled.steps[0].steps.len(), 5);
        assert_eq!(report.read_groups, 1);
        assert_eq!(report.scheduled_groups, 1);
        assert!(report.estimated_saved_ms > 0);
        compile_task_program(&scheduled, TaskProgramLimits::default()).unwrap();
    }

    #[test]
    fn short_or_stateful_read_runs_stay_serial() {
        let short = task_program(vec![observe(), extract("main"), observe()]);
        let (scheduled, report) = schedule_task_program(&short);
        assert_eq!(scheduled.steps.len(), 3);
        assert_eq!(report.scheduled_groups, 0);

        let stateful = task_program(vec![
            TaskStep {
                kind: TaskStepKind::Observe,
                save_as: Some("first".into()),
                ..TaskStep::default()
            },
            observe(),
            observe(),
            observe(),
        ]);
        let (scheduled, report) = schedule_task_program(&stateful);
        assert_eq!(scheduled.steps.len(), 4);
        assert_eq!(report.scheduled_groups, 0);
    }

    fn learned_entry(replay: ReplayPlan, action: &str) -> ActionMemoryEntry {
        ActionMemoryEntry {
            semantic_target: json!({"role":"button","name":"Save"}),
            action: action.into(),
            expected_postcondition: json!({"kind":"text_present","text":"Saved"}),
            replay: Some(replay),
            operation: None,
            preconditions: None,
            repair: None,
            last_verified_ms: 1,
        }
    }

    #[test]
    fn learned_operations_share_the_plan_contract() {
        let plan = ExecutionPlan::from_learned_operation(&learned_entry(
            ReplayPlan::Extract {
                selector: "main".into(),
            },
            "extract",
        ));
        assert_eq!(plan.source, PlanSource::LearnedOperation);
        assert_eq!(plan.body.kind(), "learned_operation");
        assert_eq!(plan.contract.effect, PlanEffect::ReadOnly);
        assert_eq!(
            plan.contract.verification,
            VerificationRequirement::Postcondition
        );
        assert!(plan.contract.cleanup.is_empty());
        assert!(plan.contract.complete());
        assert_eq!(plan.route.tier, ExecutionTier::SemanticProgram);

        let click = ExecutionPlan::from_learned_operation(&learned_entry(
            ReplayPlan::Click {
                target: crate::page_action_memory::TargetTemplate {
                    role: Some("button".into()),
                    name: Some("Save".into()),
                    href: None,
                    ancestor: None,
                    test_id: None,
                },
                outcome: VerifyPredicate::TextPresent {
                    text: "Saved".into(),
                },
            },
            "click",
        ));
        assert_eq!(click.contract.effect, PlanEffect::External);
        assert_eq!(click.contract.cleanup, vec!["release_dispatch_lease"]);
        assert!(click.contract.complete());
    }

    #[test]
    fn empirical_route_replaces_the_static_tier_without_hiding_history() {
        let plan = ExecutionPlan::from_task_program(
            &task_program(vec![observe(), extract("main")]),
            PlanSource::TaskProgram,
            capabilities(),
            PlanRoutingPolicy::QualifiedFirst,
        )
        .unwrap();
        let cost = ExpectedRouteCost::for_tier(ExecutionTier::QualifiedSkill, true, 900);
        let plan = plan.with_empirical_route(ExecutionTier::QualifiedSkill, cost);
        assert_eq!(plan.route.tier, ExecutionTier::QualifiedSkill);
        assert!(plan.route.selected_by_history);
        assert_eq!(plan.route.predicted_model_calls, 0);
        assert_eq!(plan.route.expected_cost, cost);
        assert_eq!(
            plan.receipt()["route"]["selected_by_history"].as_bool(),
            Some(true)
        );
        let webmcp = plan.with_empirical_route(ExecutionTier::WebMcpOrSiteTool, cost);
        assert!(webmcp.route.webmcp_first);
    }

    #[test]
    fn skill_replay_rebuilds_only_supported_workflows() {
        let workflow = BrowserWorkflowDefinition {
            steps: vec![BrowserWorkflowStep::Navigate {
                url: "https://example.test/".into(),
            }],
        };
        let plan =
            ExecutionPlan::for_skill_replay(&serde_json::to_value(workflow).unwrap()).unwrap();
        assert_eq!(plan.source, PlanSource::Skill);
        assert_eq!(plan.body.kind(), "browser_workflow");
        assert!(
            ExecutionPlan::for_skill_replay(&json!({"kind":"mutation"})).is_none(),
            "unsupported persisted bodies must fail closed"
        );
    }

    #[test]
    fn speculative_read_can_be_the_selected_primitive_route() {
        let decision = route_execution(
            ExecutionCapabilities {
                speculative_read: true,
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            true,
        );
        assert_eq!(decision.selected, ExecutionTier::SpeculativeRead);
        let plan = ExecutionPlan::from_primitive(
            TaskPrimitive::Research {
                urls: vec!["https://example.test/search?q=one".into()],
                selector: Some("main".into()),
                goal: Some("find the latest story".into()),
                tab_ids: None,
            },
            decision,
            vec![ExecutionTier::SpeculativeRead],
        );
        assert_eq!(plan.route.tier, ExecutionTier::SpeculativeRead);
        assert_eq!(plan.route.predicted_model_calls, 0);
        assert_eq!(plan.contract.effect, PlanEffect::ReadOnly);
    }

    #[test]
    fn route_candidates_fail_closed_to_full_vision() {
        assert_eq!(
            route_candidates(ExecutionCapabilities::default(), true),
            vec![ExecutionTier::FullVisionFallback]
        );
        let candidates = route_candidates(
            ExecutionCapabilities {
                qualified_skill: true,
                site_tool: true,
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            true,
        );
        assert_eq!(candidates[0], ExecutionTier::QualifiedSkill);
        assert!(candidates.contains(&ExecutionTier::WebMcpOrSiteTool));
        assert!(candidates.contains(&ExecutionTier::SemanticProgram));
    }

    #[test]
    fn upload_without_acceptance_is_reported_incomplete() {
        let decision = route_execution(
            ExecutionCapabilities {
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            false,
        );
        let upload = |acceptance: Option<Value>| TaskPrimitive::Upload {
            artifact_handle: Some("artifact-1".into()),
            artifact_handles: None,
            selector: Some("input[type=file]".into()),
            trigger_reference: None,
            acceptance,
            persisted: None,
            account_marker: ("@c1".into(), "signed-in".into()),
        };
        let plan = ExecutionPlan::from_primitive(upload(None), decision, Vec::new());
        assert_eq!(plan.contract.effect, PlanEffect::External);
        assert_eq!(
            plan.contract.verification,
            VerificationRequirement::IndependentObservation
        );
        assert!(!plan.contract.complete());

        let plan = ExecutionPlan::from_primitive(
            upload(Some(json!({"kind":"text_present","text":"Uploaded"}))),
            route_execution(
                ExecutionCapabilities {
                    semantic_program: true,
                    ..ExecutionCapabilities::default()
                },
                false,
            ),
            Vec::new(),
        );
        assert!(plan.contract.complete());
    }

    #[test]
    fn primitive_plan_uses_the_selected_route_and_effect() {
        let decision = route_execution(
            ExecutionCapabilities {
                semantic_program: true,
                ..ExecutionCapabilities::default()
            },
            true,
        );
        let plan = ExecutionPlan::from_primitive(
            TaskPrimitive::Research {
                urls: vec!["https://example.test/".into()],
                selector: None,
                goal: None,
                tab_ids: None,
            },
            decision,
            vec![ExecutionTier::SemanticProgram],
        );
        assert_eq!(plan.contract.effect, PlanEffect::ReadOnly);
        assert_eq!(plan.route.tier, ExecutionTier::SemanticProgram);
        assert_eq!(plan.body.expanded_steps(), 2);
    }
}
