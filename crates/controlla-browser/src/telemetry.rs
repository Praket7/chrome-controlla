//! Low-level browser execution telemetry used by higher-level benchmark and routing receipts.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

tokio::task_local! {
    static CURRENT_OPERATION_METRICS: Arc<Mutex<OperationMetrics>>;
    static CURRENT_OPERATION_STARTED: Instant;
    static CURRENT_OPERATION_ID: String;
    static CURRENT_COMMAND_BUDGET: Arc<Mutex<CommandBudgetUsage>>;
}

static NEXT_OPERATION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
const MAX_CDP_METHODS: usize = 64;
const CDP_METHOD_OVERFLOW: &str = "__method_cardinality_overflow__";

#[derive(Default)]
struct CommandBudgetUsage {
    limit: u64,
    used: u64,
    cdp_limit: u64,
    cdp_used: u64,
    stop_reason: Option<&'static str>,
    started: Option<Instant>,
    wall_limit: Duration,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct OperationMetrics {
    /// Process-local correlation key for the MCP operation receipt. It is not
    /// a cross-process trace ID or evidence of client prompt submission.
    pub operation_id: Option<String>,
    /// Legacy count of top-level telemetry scopes; this does not assert that a
    /// caller model inspected the result.
    pub model_visible_calls: u64,
    /// Page-evidence response envelopes emitted at an MCP return boundary.
    pub returned_observation_packets: u64,
    pub host_round_trips: u64,
    /// Native-host wire messages observed by the provider.
    pub native_host_messages: u64,
    pub native_host_messages_sent: u64,
    pub native_host_messages_received: u64,
    /// Dedicated Chrome launch-to-DevTools endpoint time when measured.
    pub startup_us: Option<u64>,
    /// V3 shared-session attachment time when measured.
    pub attach_us: Option<u64>,
    /// Elapsed time from operation-scope entry to first/last provider dispatch.
    /// None means no browser operation was observed in this scope.
    pub first_browser_op_us: Option<u64>,
    pub last_browser_op_us: Option<u64>,
    /// Dispatched CDP commands grouped into a bounded purpose taxonomy.
    pub command_purposes: BTreeMap<String, u64>,
    /// Method-level counts for command attribution. Cardinality is bounded;
    /// additional distinct methods are retained in the overflow bucket.
    pub cdp_methods: BTreeMap<String, u64>,
    pub cdp_commands: u64,
    pub cdp_commands_partial: bool,
    pub cdp_bytes: u64,
    pub queue_us: u64,
    pub execution_us: u64,
    pub dom_nodes_scanned: u64,
    pub observation_bytes: u64,
    pub screenshot_bytes: u64,
    pub retries: u64,
    pub unknown_deliveries: u64,
    pub browser_command_budget: u64,
    pub browser_commands_used: u64,
    pub browser_commands_remaining: u64,
    pub cdp_command_budget: u64,
    pub cdp_commands_charged: u64,
    pub cdp_commands_remaining: u64,
    pub budget_stop_reason: Option<String>,
    pub elapsed_us: u64,
    pub phases: OperationPhaseMetrics,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct OperationPhaseMetrics {
    pub discovery_us: u64,
    pub pair_us: u64,
    pub readiness_us: u64,
    pub snapshot_us: u64,
    pub grounding_us: u64,
    pub dispatch_us: u64,
    pub verification_us: u64,
    pub enrichment_us: u64,
    pub artifact_transfer_us: u64,
    /// Time spent assembling/serializing the structured MCP receipt locally.
    pub serialization_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationPhase {
    Discovery,
    Pair,
    Readiness,
    Snapshot,
    Grounding,
    Dispatch,
    Verification,
    Enrichment,
    ArtifactTransfer,
    Serialization,
}

impl OperationMetrics {
    pub fn merge(&mut self, other: &Self) {
        self.model_visible_calls = self
            .model_visible_calls
            .saturating_add(other.model_visible_calls);
        self.returned_observation_packets = self
            .returned_observation_packets
            .saturating_add(other.returned_observation_packets);
        self.host_round_trips = self.host_round_trips.saturating_add(other.host_round_trips);
        self.native_host_messages = self
            .native_host_messages
            .saturating_add(other.native_host_messages);
        self.native_host_messages_sent = self
            .native_host_messages_sent
            .saturating_add(other.native_host_messages_sent);
        self.native_host_messages_received = self
            .native_host_messages_received
            .saturating_add(other.native_host_messages_received);
        if let Some(value) = other.first_browser_op_us {
            self.first_browser_op_us = Some(
                self.first_browser_op_us
                    .map_or(value, |current| current.min(value)),
            );
        }
        if let Some(value) = other.last_browser_op_us {
            self.last_browser_op_us = Some(
                self.last_browser_op_us
                    .map_or(value, |current| current.max(value)),
            );
        }
        for (purpose, count) in &other.command_purposes {
            let current = self.command_purposes.entry(purpose.clone()).or_default();
            *current = current.saturating_add(*count);
        }
        for (method, count) in &other.cdp_methods {
            let key = if self.cdp_methods.contains_key(method)
                || (method != CDP_METHOD_OVERFLOW && self.cdp_methods.len() < MAX_CDP_METHODS - 1)
                || method == CDP_METHOD_OVERFLOW
            {
                method.as_str()
            } else {
                CDP_METHOD_OVERFLOW
            };
            let current = self.cdp_methods.entry(key.to_owned()).or_default();
            *current = current.saturating_add(*count);
        }
        self.cdp_commands = self.cdp_commands.saturating_add(other.cdp_commands);
        self.cdp_commands_partial |= other.cdp_commands_partial;
        self.cdp_bytes = self.cdp_bytes.saturating_add(other.cdp_bytes);
        self.queue_us = self.queue_us.saturating_add(other.queue_us);
        self.execution_us = self.execution_us.saturating_add(other.execution_us);
        self.dom_nodes_scanned = self
            .dom_nodes_scanned
            .saturating_add(other.dom_nodes_scanned);
        self.observation_bytes = self
            .observation_bytes
            .saturating_add(other.observation_bytes);
        self.screenshot_bytes = self.screenshot_bytes.saturating_add(other.screenshot_bytes);
        self.retries = self.retries.saturating_add(other.retries);
        self.unknown_deliveries = self
            .unknown_deliveries
            .saturating_add(other.unknown_deliveries);
        self.browser_command_budget = self
            .browser_command_budget
            .max(other.browser_command_budget);
        self.browser_commands_used = self
            .browser_commands_used
            .saturating_add(other.browser_commands_used);
        self.browser_commands_remaining = other.browser_commands_remaining;
        self.cdp_command_budget = self.cdp_command_budget.max(other.cdp_command_budget);
        self.cdp_commands_charged = self
            .cdp_commands_charged
            .saturating_add(other.cdp_commands_charged);
        self.cdp_commands_remaining = other.cdp_commands_remaining;
        if other.budget_stop_reason.is_some() {
            self.budget_stop_reason = other.budget_stop_reason.clone();
        }
        self.elapsed_us = self.elapsed_us.saturating_add(other.elapsed_us);
        self.phases.merge(&other.phases);
    }
}

impl OperationPhaseMetrics {
    fn merge(&mut self, other: &Self) {
        self.discovery_us = self.discovery_us.saturating_add(other.discovery_us);
        self.pair_us = self.pair_us.saturating_add(other.pair_us);
        self.readiness_us = self.readiness_us.saturating_add(other.readiness_us);
        self.snapshot_us = self.snapshot_us.saturating_add(other.snapshot_us);
        self.grounding_us = self.grounding_us.saturating_add(other.grounding_us);
        self.dispatch_us = self.dispatch_us.saturating_add(other.dispatch_us);
        self.verification_us = self.verification_us.saturating_add(other.verification_us);
        self.enrichment_us = self.enrichment_us.saturating_add(other.enrichment_us);
        self.artifact_transfer_us = self
            .artifact_transfer_us
            .saturating_add(other.artifact_transfer_us);
        self.serialization_us = self.serialization_us.saturating_add(other.serialization_us);
    }
}

/// Run one MCP operation with task-local metrics; nested calls reuse the same span.
pub async fn collect_operation_metrics<T>(
    future: impl Future<Output = T>,
) -> (T, OperationMetrics) {
    if let Ok(metrics) = CURRENT_OPERATION_METRICS.try_with(Arc::clone) {
        let result = future.await;
        return (result, metrics.lock().await.clone());
    }

    let sequence = NEXT_OPERATION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let operation_id = format!("mcp:{}:{sequence}", std::process::id());
    let metrics = Arc::new(Mutex::new(OperationMetrics {
        model_visible_calls: 1,
        operation_id: Some(operation_id.clone()),
        ..OperationMetrics::default()
    }));
    let started = Instant::now();
    let result = CURRENT_OPERATION_ID
        .scope(
            operation_id,
            CURRENT_OPERATION_STARTED.scope(
                started,
                CURRENT_OPERATION_METRICS.scope(Arc::clone(&metrics), future),
            ),
        )
        .await;
    metrics.lock().await.elapsed_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    let collected = metrics.lock().await.clone();
    (result, collected)
}

pub async fn with_browser_command_budget<T>(
    max_commands: u64,
    max_cdp_commands: u64,
    wall_limit: Duration,
    future: impl Future<Output = T>,
) -> T {
    if CURRENT_COMMAND_BUDGET.try_with(|_| ()).is_ok() {
        return future.await;
    }
    let usage = Arc::new(Mutex::new(CommandBudgetUsage {
        limit: max_commands,
        used: 0,
        cdp_limit: max_cdp_commands,
        cdp_used: 0,
        stop_reason: None,
        started: Some(Instant::now()),
        wall_limit,
    }));
    let result = CURRENT_COMMAND_BUDGET
        .scope(Arc::clone(&usage), future)
        .await;
    let usage = usage.lock().await;
    update_current_metrics(|metrics| {
        metrics.browser_command_budget = max_commands;
        metrics.browser_commands_used = usage.used;
        metrics.browser_commands_remaining = max_commands.saturating_sub(usage.used);
        metrics.cdp_command_budget = max_cdp_commands;
        metrics.cdp_commands_charged = usage.cdp_used;
        metrics.cdp_commands_remaining = max_cdp_commands.saturating_sub(usage.cdp_used);
    })
    .await;
    result
}

/// Reserve bounded CDP work before dispatch, including bridge-local microprograms.
pub async fn admit_cdp_commands(count: u64) -> Result<(), &'static str> {
    let Ok(usage) = CURRENT_COMMAND_BUDGET.try_with(Arc::clone) else {
        return Ok(());
    };
    let mut usage = usage.lock().await;
    let reason = if usage
        .started
        .is_some_and(|started| started.elapsed() > usage.wall_limit)
    {
        Some("workflow_wall_time_exceeded")
    } else if usage.cdp_used.saturating_add(count) > usage.cdp_limit {
        Some("workflow_cdp_command_budget_exceeded")
    } else {
        None
    };
    if let Some(reason) = reason {
        usage.stop_reason = Some(reason);
        drop(usage);
        update_current_metrics(|metrics| metrics.budget_stop_reason = Some(reason.into())).await;
        return Err(reason);
    }
    usage.cdp_used = usage.cdp_used.saturating_add(count);
    let used = usage.cdp_used;
    drop(usage);
    update_current_metrics(|metrics| metrics.cdp_commands_charged = used).await;
    Ok(())
}

/// Return the typed reason for a command rejected before it reached the transport.
pub async fn command_budget_stop_reason() -> Option<&'static str> {
    let usage = CURRENT_COMMAND_BUDGET.try_with(Arc::clone).ok()?;
    usage.lock().await.stop_reason
}

/// Charge before sending so an over-budget command is never dispatched.
pub async fn admit_browser_commands(count: u64) -> Result<(), &'static str> {
    let Ok(usage) = CURRENT_COMMAND_BUDGET.try_with(Arc::clone) else {
        return Ok(());
    };
    let mut usage = usage.lock().await;
    let reason = if usage
        .started
        .is_some_and(|started| started.elapsed() > usage.wall_limit)
    {
        Some("workflow_wall_time_exceeded")
    } else if usage.used.saturating_add(count) > usage.limit {
        Some("workflow_browser_command_budget_exceeded")
    } else {
        None
    };
    if let Some(reason) = reason {
        usage.stop_reason = Some(reason);
        drop(usage);
        update_current_metrics(|metrics| metrics.budget_stop_reason = Some(reason.into())).await;
        return Err(reason);
    }
    usage.used = usage.used.saturating_add(count);
    let used = usage.used;
    drop(usage);
    update_current_metrics(|metrics| metrics.browser_commands_used = used).await;
    Ok(())
}

pub async fn record_browser_command(
    host_round_trips: u64,
    cdp_commands: u64,
    cdp_bytes: u64,
    queue_us: u64,
    execution_us: u64,
) {
    update_current_metrics(|metrics| {
        metrics.host_round_trips = metrics.host_round_trips.saturating_add(host_round_trips);
        metrics.cdp_commands = metrics.cdp_commands.saturating_add(cdp_commands);
        metrics.cdp_bytes = metrics.cdp_bytes.saturating_add(cdp_bytes);
        metrics.queue_us = metrics.queue_us.saturating_add(queue_us);
        metrics.execution_us = metrics.execution_us.saturating_add(execution_us);
    })
    .await;
}

/// Record a native-host wire message at the send or response boundary.
pub async fn record_native_host_message(received: bool) {
    update_current_metrics(|metrics| {
        metrics.native_host_messages = metrics.native_host_messages.saturating_add(1);
        if received {
            metrics.native_host_messages_received =
                metrics.native_host_messages_received.saturating_add(1);
        } else {
            metrics.native_host_messages_sent = metrics.native_host_messages_sent.saturating_add(1);
        }
    })
    .await;
}

/// Record elapsed dedicated-browser startup through DevTools endpoint readiness.
pub async fn record_startup_duration_us(elapsed_us: u64) {
    update_current_metrics(|metrics| {
        metrics.startup_us = Some(
            metrics
                .startup_us
                .unwrap_or_default()
                .saturating_add(elapsed_us),
        );
    })
    .await;
}

/// Record elapsed provider/session attachment for the current MCP operation.
pub async fn record_attach_duration_us(elapsed_us: u64) {
    update_current_metrics(|metrics| {
        metrics.attach_us = Some(
            metrics
                .attach_us
                .unwrap_or_default()
                .saturating_add(elapsed_us),
        );
    })
    .await;
}

/// Measure dedicated-browser startup and record duration on both success and failure.
pub async fn measure_startup<T>(operation: impl Future<Output = T>) -> (T, u64) {
    let started = Instant::now();
    let result = operation.await;
    let elapsed_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    record_startup_duration_us(elapsed_us).await;
    (result, elapsed_us)
}

/// Measure provider/session attachment and record duration on both success and failure.
pub async fn measure_attach<T>(operation: impl Future<Output = T>) -> (T, u64) {
    let started = Instant::now();
    let result = operation.await;
    let elapsed_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    record_attach_duration_us(elapsed_us).await;
    (result, elapsed_us)
}

/// Record the provider send boundary, before waiting for the extension reply.
pub async fn record_browser_operation_dispatch() {
    let Some(offset_us) = CURRENT_OPERATION_STARTED
        .try_with(|started| started.elapsed().as_micros().min(u64::MAX as u128) as u64)
        .ok()
    else {
        return;
    };
    update_current_metrics(|metrics| {
        metrics.first_browser_op_us = Some(
            metrics
                .first_browser_op_us
                .map_or(offset_us, |value| value.min(offset_us)),
        );
        metrics.last_browser_op_us = Some(
            metrics
                .last_browser_op_us
                .map_or(offset_us, |value| value.max(offset_us)),
        );
    })
    .await;
}

/// Record dispatched CDP work by a fixed, low-cardinality purpose category.
pub async fn record_command_purpose(method: &str, count: u64) {
    let purpose = match method {
        "Page.navigate" | "Page.reload" | "Page.stopLoading" => "navigation",
        "Page.getFrameTree"
        | "Page.getNavigationHistory"
        | "Controlla.checkIdentityMarker"
        | "Controlla.presentationState" => "document_identity",
        "Page.captureScreenshot" | "Controlla.captureScreenshot" => "visual_capture",
        "DOMSnapshot.captureSnapshot"
        | "Accessibility.getPartialAXTree"
        | "Controlla.frameAccessibility"
        | "Controlla.snapshot"
        | "Controlla.find"
        | "Controlla.fileInputSnapshot"
        | "Controlla.findFileInput" => "observation",
        "Runtime.evaluate" => "page_script_evaluation",
        "Input.dispatchMouseEvent"
        | "Input.dispatchKeyEvent"
        | "Input.insertText"
        | "Controlla.click"
        | "Controlla.fill"
        | "Controlla.drag"
        | "Controlla.frameClick"
        | "Controlla.pointerClick"
        | "Controlla.pointerScroll"
        | "Controlla.pointerSwipe"
        | "Controlla.frameScroll"
        | "Controlla.scrollToTarget"
        | "Controlla.fastType"
        | "Controlla.frameFill"
        | "Controlla.humanTypeCharacter"
        | "Controlla.securePasswordFill" => "mutation",
        "Page.setFileInputFiles"
        | "Controlla.upload"
        | "Controlla.waitForFileChooser"
        | "Controlla.frameSelectFile"
        | "Controlla.setFrameFileInputFiles"
        | "Controlla.downloadLifecycle"
        | "IO.read"
        | "IO.close" => "file_transfer",
        "Controlla.activateOwnedTab"
        | "Controlla.closeOwnedTab"
        | "Controlla.moveOwnedTab"
        | "Controlla.manageTabs"
        | "Controlla.openTab"
        | "Controlla.navigate"
        | "Controlla.newTabs" => "target_lifecycle",
        _ => "other",
    };
    update_current_metrics(|metrics| {
        let current = metrics
            .command_purposes
            .entry(purpose.to_owned())
            .or_default();
        *current = current.saturating_add(count);
        let method_key = if metrics.cdp_methods.contains_key(method)
            || (method != CDP_METHOD_OVERFLOW && metrics.cdp_methods.len() < MAX_CDP_METHODS - 1)
            || method == CDP_METHOD_OVERFLOW
        {
            method
        } else {
            CDP_METHOD_OVERFLOW
        };
        let method_count = metrics
            .cdp_methods
            .entry(method_key.to_owned())
            .or_default();
        *method_count = method_count.saturating_add(count);
    })
    .await;
}

pub async fn mark_cdp_commands_partial() {
    update_current_metrics(|metrics| metrics.cdp_commands_partial = true).await;
}

pub async fn record_observation(nodes: u64, bytes: u64) {
    update_current_metrics(|metrics| {
        metrics.dom_nodes_scanned = metrics.dom_nodes_scanned.saturating_add(nodes);
        metrics.observation_bytes = metrics.observation_bytes.saturating_add(bytes);
    })
    .await;
}

pub async fn record_unknown_delivery() {
    update_current_metrics(|metrics| {
        metrics.unknown_deliveries = metrics.unknown_deliveries.saturating_add(1);
    })
    .await;
}

pub async fn record_phase(phase: OperationPhase, elapsed_us: u64) {
    update_current_metrics(|metrics| {
        let value = match phase {
            OperationPhase::Discovery => &mut metrics.phases.discovery_us,
            OperationPhase::Pair => &mut metrics.phases.pair_us,
            OperationPhase::Readiness => &mut metrics.phases.readiness_us,
            OperationPhase::Snapshot => &mut metrics.phases.snapshot_us,
            OperationPhase::Grounding => &mut metrics.phases.grounding_us,
            OperationPhase::Dispatch => &mut metrics.phases.dispatch_us,
            OperationPhase::Verification => &mut metrics.phases.verification_us,
            OperationPhase::Enrichment => &mut metrics.phases.enrichment_us,
            OperationPhase::ArtifactTransfer => &mut metrics.phases.artifact_transfer_us,
            OperationPhase::Serialization => &mut metrics.phases.serialization_us,
        };
        *value = value.saturating_add(elapsed_us);
    })
    .await;
}

async fn update_current_metrics(update: impl FnOnce(&mut OperationMetrics)) {
    if let Ok(metrics) = CURRENT_OPERATION_METRICS.try_with(Arc::clone) {
        let mut current = metrics.lock().await;
        update(&mut current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn operation_scope_collects_command_and_phase_metrics() {
        let (value, metrics) = collect_operation_metrics(async {
            record_browser_operation_dispatch().await;
            record_browser_command(1, 3, 120, 40, 7).await;
            record_native_host_message(false).await;
            record_native_host_message(true).await;
            record_command_purpose("Page.getFrameTree", 1).await;
            record_command_purpose("DOMSnapshot.captureSnapshot", 2).await;
            record_phase(OperationPhase::Snapshot, 9).await;
            record_browser_operation_dispatch().await;
            "observed"
        })
        .await;

        assert_eq!(value, "observed");
        assert_eq!(metrics.model_visible_calls, 1);
        assert!(
            metrics
                .operation_id
                .as_deref()
                .is_some_and(|id| id.starts_with("mcp:"))
        );
        assert_eq!(
            metrics.startup_us, None,
            "startup is unavailable in this process scope"
        );
        assert_eq!(
            metrics.attach_us, None,
            "attach is unavailable in this process scope"
        );
        assert_eq!(metrics.host_round_trips, 1);
        assert_eq!(metrics.native_host_messages, 2);
        assert_eq!(metrics.native_host_messages_sent, 1);
        assert_eq!(metrics.native_host_messages_received, 1);
        assert!(metrics.first_browser_op_us.is_some());
        assert!(metrics.last_browser_op_us >= metrics.first_browser_op_us);
        assert_eq!(metrics.cdp_commands, 3);
        assert_eq!(metrics.cdp_bytes, 120);
        assert_eq!(metrics.queue_us, 40);
        assert_eq!(metrics.execution_us, 7);
        assert_eq!(metrics.command_purposes["document_identity"], 1);
        assert_eq!(metrics.command_purposes["observation"], 2);
        assert_eq!(metrics.cdp_methods["Page.getFrameTree"], 1);
        assert_eq!(metrics.cdp_methods["DOMSnapshot.captureSnapshot"], 2);
        assert_eq!(metrics.cdp_methods.values().sum::<u64>(), 3);
        assert_eq!(metrics.phases.snapshot_us, 9);
    }

    #[tokio::test]
    async fn method_attribution_bounds_cardinality_and_preserves_total() {
        let (_, metrics) = collect_operation_metrics(async {
            for index in 0..80 {
                record_command_purpose(&format!("Test.method{index}"), 1).await;
            }
        })
        .await;

        assert_eq!(metrics.cdp_methods.len(), MAX_CDP_METHODS);
        assert_eq!(metrics.cdp_methods[CDP_METHOD_OVERFLOW], 17);
        assert_eq!(metrics.cdp_methods.values().sum::<u64>(), 80);
    }

    #[tokio::test]
    async fn startup_and_attach_lifecycle_spans_are_observed_when_measured() {
        let (_, metrics) = collect_operation_metrics(async {
            let (startup, _) = measure_startup(async { Err::<(), _>("startup failed") }).await;
            assert!(startup.is_err());
            let (attach, _) = measure_attach(async { Err::<(), _>("attach failed") }).await;
            assert!(attach.is_err());
        })
        .await;

        assert!(metrics.startup_us.is_some());
        assert!(metrics.attach_us.is_some());
    }

    #[tokio::test]
    async fn opaque_extension_programs_mark_cdp_totals_partial() {
        let (_, metrics) = collect_operation_metrics(async {
            record_browser_command(1, 1, 40, 0, 7).await;
            mark_cdp_commands_partial().await;
        })
        .await;
        assert_eq!(metrics.cdp_commands, 1);
        assert!(metrics.cdp_commands_partial);
    }

    #[tokio::test]
    async fn nested_operation_scope_reuses_parent_metrics() {
        let (_, metrics) = collect_operation_metrics(async {
            record_phase(OperationPhase::Pair, 5).await;
            let (_, nested) = collect_operation_metrics(async {
                record_phase(OperationPhase::Readiness, 6).await;
            })
            .await;
            assert_eq!(nested.model_visible_calls, 1);
        })
        .await;

        assert_eq!(metrics.model_visible_calls, 1);
        assert_eq!(metrics.phases.pair_us, 5);
        assert_eq!(metrics.phases.readiness_us, 6);
    }

    #[tokio::test]
    async fn command_budget_refuses_dispatch_before_limit_and_reports_usage() {
        let (_, metrics) = collect_operation_metrics(with_browser_command_budget(
            1,
            4,
            Duration::from_secs(1),
            async {
                assert!(admit_browser_commands(1).await.is_ok());
                assert_eq!(
                    admit_browser_commands(1).await,
                    Err("workflow_browser_command_budget_exceeded")
                );
                assert!(admit_cdp_commands(4).await.is_ok());
                assert_eq!(
                    admit_cdp_commands(1).await,
                    Err("workflow_cdp_command_budget_exceeded")
                );
                assert_eq!(
                    command_budget_stop_reason().await,
                    Some("workflow_cdp_command_budget_exceeded")
                );
            },
        ))
        .await;
        assert_eq!(metrics.browser_command_budget, 1);
        assert_eq!(metrics.browser_commands_used, 1);
        assert_eq!(metrics.browser_commands_remaining, 0);
        assert_eq!(metrics.cdp_commands_charged, 4);
        assert_eq!(metrics.cdp_commands_remaining, 0);
        assert_eq!(
            metrics.budget_stop_reason.as_deref(),
            Some("workflow_cdp_command_budget_exceeded")
        );
    }

    #[tokio::test]
    async fn nested_command_budget_uses_the_outer_open_operation_limit() {
        let (_, metrics) = collect_operation_metrics(with_browser_command_budget(
            2,
            3,
            Duration::from_secs(1),
            async {
                assert!(admit_browser_commands(1).await.is_ok());
                with_browser_command_budget(99, 99, Duration::from_secs(99), async {
                    assert!(admit_browser_commands(1).await.is_ok());
                    assert_eq!(
                        admit_browser_commands(1).await,
                        Err("workflow_browser_command_budget_exceeded")
                    );
                })
                .await;
            },
        ))
        .await;
        assert_eq!(metrics.browser_command_budget, 2);
        assert_eq!(metrics.browser_commands_used, 2);
        assert_eq!(metrics.browser_commands_remaining, 0);
    }

    #[test]
    fn metrics_merge_is_saturating() {
        let mut metrics = OperationMetrics {
            cdp_commands: u64::MAX,
            ..Default::default()
        };
        metrics.merge(&OperationMetrics {
            cdp_commands: 1,
            queue_us: 7,
            ..Default::default()
        });
        assert_eq!(metrics.cdp_commands, u64::MAX);
        assert_eq!(metrics.queue_us, 7);
    }
}
