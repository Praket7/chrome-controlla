use super::{
    AppV2, BrowserActArgs, BrowserClickOutcome, BrowserExtractArgs, BrowserFindArgs,
    BrowserPredicate, BrowserSessionArgs, BrowserSnapshotArgs, BrowserVerifyArgs, find_impl,
    resolve_reference, retained_mutation, snapshot_impl, verify_impl,
};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::{Json, Parameters},
    tool, tool_handler, tool_router,
};
use serde_json::{Value, json};
use base64::Engine as _;
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const SHARED_TRIGGER_UPLOAD_DEADLINE: Duration = Duration::from_secs(45);

async fn measured_receipt(
    operation: impl Future<Output = Result<Json<Value>, rmcp::ErrorData>>,
) -> Result<Json<Value>, rmcp::ErrorData> {
    let (result, metrics) = controlla_browser::scheduler::telemetry::collect_operation_metrics(operation).await;
    result.map(|Json(mut receipt)| {
        receipt["metrics"] = serde_json::to_value(metrics).unwrap_or(Value::Null);
        Json(receipt)
    })
}

#[derive(Clone)]
struct AppV3 {
    core: AppV2,
    leases: Arc<Mutex<crate::v3::LeaseTable>>,
    browser_modes: Arc<Mutex<std::collections::BTreeMap<String, crate::v3::BrowserMode>>>,
    headless_chrome: Option<std::path::PathBuf>,
}

impl AppV3 {
    fn new(core: AppV2) -> Self {
        Self {
            core,
            leases: Arc::new(Mutex::new(crate::v3::LeaseTable::default())),
            browser_modes: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            headless_chrome: std::env::var_os("COMPTROL_CHROME_EXECUTABLE")
                .map(std::path::PathBuf::from),
        }
    }

    async fn readiness_url(&self, session_id: &str, tab_id: &str) -> Result<String, rmcp::ErrorData> {
        let live = self.core.legacy.shared_sessions.lock().await.get(session_id).cloned().ok_or_else(|| invalid("unknown shared session"))?;
        let live = live.lock().await;
        let connection = live.connection.as_ref().ok_or_else(|| invalid("shared session is not accepted"))?;
        let identity = super::super::shared_frame_identity(connection, live.registry()?, &live.handle, tab_id).await.map_err(invalid)?;
        Ok(identity.2)
    }

    async fn readiness_document(&self, session_id: &str, tab_id: &str) -> Result<(String, String, String), String> {
        let live = self.core.legacy.shared_sessions.lock().await.get(session_id).cloned().ok_or_else(|| "unknown shared session".to_owned())?;
        let live = live.lock().await;
        let connection = live.connection.as_ref().ok_or_else(|| "shared session is not accepted".to_owned())?;
        let registry = live.registry().map_err(|error| error.to_string())?;
        let identity = super::super::shared_frame_identity(connection, registry, &live.handle, tab_id).await?;
        let response = connection.command(registry, &live.handle, tab_id, "Runtime.evaluate", json!({
            "expression":"({url:location.href,ready_state:document.readyState})",
            "returnByValue":true
        })).await?;
        let state = super::super::shared_value(&response).map_err(|error| error.to_string())?;
        Ok((identity.1, state["url"].as_str().unwrap_or(&identity.2).to_owned(), state["ready_state"].as_str().unwrap_or("loading").to_owned()))
    }

    async fn readiness_selector(&self, session_id: &str, tab_id: &str, selector: &str) -> Result<bool, rmcp::ErrorData> {
        let live = self.core.legacy.shared_sessions.lock().await.get(session_id).cloned().ok_or_else(|| invalid("unknown shared session"))?;
        let live = live.lock().await;
        let connection = live.connection.as_ref().ok_or_else(|| invalid("shared session is not accepted"))?;
        let expression = format!("(()=>{{try{{return {{matched:!!document.querySelector({})}}}}catch(_){{return {{selector_error:true}}}}}})()", json!(selector));
        let response = connection.command(live.registry()?, &live.handle, tab_id, "Runtime.evaluate", json!({"expression":expression,"returnByValue":true})).await.map_err(|error| invalid(error.to_string()))?;
        let result = super::super::shared_value(&response).map_err(invalid)?;
        if result["selector_error"] == true { return Err(invalid("invalid readiness selector")); }
        Ok(result["matched"] == true)
    }

    async fn wait_for_open_readiness(
        &self,
        session_id: &str,
        tab_id: &str,
        ready_when: &V3ReadyWhen,
    ) -> Result<bool, rmcp::ErrorData> {
        if matches!(ready_when, V3ReadyWhen::DocumentCommitted | V3ReadyWhen::DomContentLoaded) {
            return Ok(true);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match ready_when {
                V3ReadyWhen::UrlPrefix { prefix } => {
                    if self.readiness_url(session_id, tab_id).await?.starts_with(prefix) { return Ok(true); }
                }
                V3ReadyWhen::Selector { selector }
                    if self.readiness_selector(session_id, tab_id, selector).await? =>
                {
                    return Ok(true);
                }
                _ => {}
            }
            if matches!(ready_when, V3ReadyWhen::UrlPrefix { .. } | V3ReadyWhen::Selector { .. }) {
                if tokio::time::Instant::now() >= deadline { return Ok(false); }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            let (selector, goal) = match ready_when {
                V3ReadyWhen::SemanticMatch { query, role } => (
                    None,
                    Some(super::SnapshotGoal {
                        query: Some(query.clone()),
                        roles: role.clone().map(|role| vec![role]),
                        href_hint: None,
                        within: None,
                        exhaustive: false,
                    }),
                ),
                _ => (None, None),
            };
            let snapshot = snapshot_impl(&self.core, &BrowserSnapshotArgs {
                session_id: session_id.to_owned(),
                chrome_tab_id: tab_id.to_owned(),
                selector,
                max_items: Some(1),
                max_text_chars: Some(0),
                max_bytes: Some(16_000),
                href_hint: None,
                frame_id: None,
                goal,
                max_scan_nodes: Some(65_536),
            }).await;
            match snapshot {
                Ok(snapshot) => {
                    let ready = match ready_when {
                        V3ReadyWhen::UrlPrefix { .. } | V3ReadyWhen::Selector { .. } => false,
                        V3ReadyWhen::SemanticMatch { .. } => snapshot["goal_satisfied"] == true,
                        V3ReadyWhen::DocumentCommitted | V3ReadyWhen::DomContentLoaded => true,
                    };
                    if ready { return Ok(true); }
                }
                Err(error) if matches!(ready_when, V3ReadyWhen::Selector { .. })
                    && error.to_string().contains("selector_error:selector:no_match") => {}
                Err(error) => return Err(error),
            }
            if tokio::time::Instant::now() >= deadline { return Ok(false); }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn require_v3_session(&self, session_id: &str) -> Result<(), rmcp::ErrorData> {
        if self.browser_modes.lock().await.contains_key(session_id) {
            Ok(())
        } else {
            Err(invalid("session was not created through the V3 browser route"))
        }
    }

    async fn release_cold_session(&self, session_id: &str) {
        let _ = self.core.browser_session(Parameters(BrowserSessionArgs {
            action: "release_shared".into(),
            provider: Some("shared_extension".into()),
            session_id: Some(session_id.to_owned()),
            target_ids: None,
            host_id: None,
        })).await;
        self.browser_modes.lock().await.remove(session_id);
    }

    async fn cold_open_session(&self, args: &V3BrowserArgs) -> Result<String, rmcp::ErrorData> {
        if !crate::native_setup::host_registered() {
            return Err(invalid("native Chrome bridge is not registered"));
        }
        let discovery_started = std::time::Instant::now();
        let target_ids = match args.target_ids.clone() {
            Some(ids) => ids,
            None if crate::guard_reason::unrestricted_mode() => {
                let snapshot = crate::native_setup::read_tabs().map_err(invalid)?;
                vec![crate::native_setup::bootstrap_tab_id(&snapshot)
                    .ok_or_else(|| invalid("no stable web tab is available to open a foreground tab"))?]
            }
            None => return Err(invalid("guarded cold open requires host_id and target_ids from discover_shared_tabs")),
        };
        if args.target_ids.is_none() {
            controlla_browser::scheduler::telemetry::record_phase(
                controlla_browser::scheduler::telemetry::OperationPhase::Discovery,
                discovery_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            ).await;
        }
        if !crate::guard_reason::unrestricted_mode() && args.host_id.is_none() {
            return Err(invalid("guarded cold open requires host_id from discover_shared_tabs"));
        }
        let pair_started = std::time::Instant::now();
        let Json(pair) = self.core.browser_session(Parameters(BrowserSessionArgs {
            action: "pair_shared".into(),
            provider: Some("shared_extension".into()),
            session_id: None,
            target_ids: Some(target_ids),
            host_id: args.host_id.clone(),
        })).await?;
        controlla_browser::scheduler::telemetry::record_phase(
            controlla_browser::scheduler::telemetry::OperationPhase::Pair,
            pair_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        ).await;
        let session_id = pair["session_id"].as_str()
            .ok_or_else(|| invalid("pairing omitted session_id"))?.to_owned();
        self.browser_modes.lock().await.insert(session_id.clone(), crate::v3::BrowserMode::Foreground);
        if pair["native_bridge"] != true {
            self.release_cold_session(&session_id).await;
            return Err(invalid("native Chrome bridge is unavailable for cold open"));
        }
        let ready_started = std::time::Instant::now();
        let accepted = self.core.browser_session(Parameters(BrowserSessionArgs {
            action: "accept_shared_wait".into(),
            provider: Some("shared_extension".into()),
            session_id: Some(session_id.clone()),
            target_ids: None,
            host_id: None,
        })).await;
        controlla_browser::scheduler::telemetry::record_phase(
            controlla_browser::scheduler::telemetry::OperationPhase::Readiness,
            ready_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        ).await;
        match accepted {
            Ok(Json(value)) if value["accepted"] == true => Ok(session_id),
            Ok(_) => {
                self.release_cold_session(&session_id).await;
                Err(invalid("native Chrome pairing timed out; inspect bridge status"))
            }
            Err(error) => {
                self.release_cold_session(&session_id).await;
                Err(error)
            }
        }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("allOf" = [{"if":{"properties":{"action":{"const":"open"}},"required":["action"]},"then":{"required":["idempotency_key"]}}]))]
struct V3BrowserArgs {
    action: String,
    provider: Option<String>,
    session_id: Option<String>,
    chrome_tab_id: Option<String>,
    target_ids: Option<Vec<String>>,
    host_id: Option<String>,
    mode: Option<String>,
    url: Option<String>,
    #[schemars(description = "Optional task readiness condition after DOMContentLoaded. Use semantic_match or selector when steps must not start before a target appears; timeout returns an incomplete open receipt and skips steps.")]
    ready_when: Option<V3ReadyWhen>,
    #[schemars(description = "For action=task, pass a tagged task object. Example: {\"kind\":\"research\",\"urls\":[\"https://example.test/\"]}. Research navigates the selected existing tab.")]
    task: Option<crate::v3_tasks::TaskPrimitive>,
    filename: Option<String>,
    #[schemars(description = "Inline artifact bytes as a JSON byte array. Prefer bytes_base64 for compact portable transfer.")]
    bytes: Option<Vec<u8>>,
    #[schemars(description = "Standard padded Base64 artifact bytes (1..10 MiB decoded). Provide exactly one of bytes, bytes_base64, or file_path.")]
    bytes_base64: Option<String>,
    file_path: Option<String>,
    artifact_handle: Option<String>,
    #[schemars(description = "For artifact_read, use compact base64 by default or legacy JSON byte arrays with bytes.")]
    encoding: Option<String>,
    client_id: Option<String>,
    #[schemars(description = "Required for action=open. Reuse the same key only when retrying the same logical open; choose a new key for a distinct new tab.")]
    idempotency_key: Option<String>,
    steps: Option<Vec<V3WorkflowStep>>,
}

#[derive(Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum V3ReadyWhen {
    DocumentCommitted,
    DomContentLoaded,
    UrlPrefix { prefix: String },
    Selector { selector: String },
    SemanticMatch { query: String, role: Option<String> },
}

impl V3ReadyWhen {
    fn label(&self) -> &'static str {
        match self {
            Self::DocumentCommitted => "document_committed",
            Self::DomContentLoaded => "dom_content_loaded",
            Self::UrlPrefix { .. } => "url_prefix",
            Self::Selector { .. } => "selector",
            Self::SemanticMatch { .. } => "semantic_match",
        }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3ActArgs {
    session_id: String,
    chrome_tab_id: String,
    action: String,
    reference: Option<String>,
    expected_value: Option<String>,
    value: Option<String>,
    #[schemars(description = "Explicitly allow filling a password input for this act only. The value is sent to the local MCP and browser and is never returned in snapshots or action results.")]
    allow_password: Option<bool>,
    key: Option<String>,
    outcome: Option<BrowserClickOutcome>,
    timeout_ms: Option<u64>,
    typing_mode: Option<String>,
    #[schemars(description = "For human_keys, delay after each character in milliseconds (0–250); defaults to 180 ms per character. Other typing modes ignore this field.")]
    delay_ms: Option<u64>,
    client_id: Option<String>,
    page_tool_name: Option<String>,
    page_tool_input: Option<Value>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3WorkflowArgs {
    session_id: String,
    chrome_tab_id: String,
    #[serde(default)]
    steps: Vec<V3WorkflowStep>,
    #[serde(default)]
    program: Option<Vec<V3WorkflowOp>>,
    client_id: Option<String>,
}

#[derive(Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3WorkflowFillField {
    reference: String,
    expected_value: String,
    value: String,
    #[serde(default)]
    #[schemars(description = "Set true for a guarded HTML select control; false fills an input or textarea.")]
    select: bool,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3TaskArgs {
    session_id: String,
    chrome_tab_id: String,
    task: crate::v3_tasks::TaskPrimitive,
    client_id: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3ArtifactRegisterArgs {
    session_id: String,
    filename: String,
    bytes: Option<Vec<u8>>,
    bytes_base64: Option<String>,
    file_path: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3ArtifactReadArgs {
    session_id: String,
    artifact_handle: String,
    #[serde(default)]
    #[schemars(description = "Read encoding: base64 (default, compact) or bytes (legacy JSON integer array).")]
    encoding: Option<String>,
}

#[derive(Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum V3WorkflowStep {
    Snapshot {
        href_hint: Option<String>,
        max_items: Option<usize>,
        max_text_chars: Option<usize>,
        goal: Option<super::SnapshotGoal>,
        max_scan_nodes: Option<usize>,
    },
    Find {
        query: String,
        role: Option<String>,
        match_mode: Option<String>,
        max_matches: Option<usize>,
        within_reference: Option<String>,
        href_hint: Option<String>,
        save_as: String,
    },
    FindAll {
        query: Option<String>,
        role: Option<String>,
        href_hint: Option<String>,
        max_matches: usize,
        save_as: String,
    },
    CollectVirtual {
        container: Option<String>,
        query: Option<String>,
        role: Option<String>,
        identity_fields: Vec<String>,
        max_scrolls: u8,
        max_items: usize,
        save_as: String,
    },
    Choose {
        from: String,
        key: String,
        save_as: String,
    },
    Click {
        reference: String,
        outcome: Option<BrowserClickOutcome>,
    },
    Upload {
        artifact_handle: String,
        trigger_reference: String,
        account_marker: (String, String),
        acceptance: Option<BrowserPredicate>,
    },
    Fill {
        reference: String,
        expected_value: String,
        value: String,
    },
    Select {
        reference: String,
        expected_value: String,
        value: String,
    },
    FillForm {
        fields: Vec<V3WorkflowFillField>,
    },
    Type {
        reference: String,
        expected_value: String,
        value: String,
        typing_mode: Option<String>,
        delay_ms: Option<u64>,
    },
    Press {
        reference: String,
        key: String,
    },
    Extract {
        selector: String,
        fields: Option<std::collections::BTreeMap<String, String>>,
    },
    PageTool {
        name: String,
        input: Value,
    },
    Verify {
        predicate: BrowserPredicate,
    },
}

#[derive(Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum V3WorkflowOp {
    Step { step: V3WorkflowStep },
    If { condition: String, then_branch: Vec<V3WorkflowOp>, else_branch: Vec<V3WorkflowOp> },
    Foreach { items: Vec<String>, var: String, body: Vec<V3WorkflowOp> },
    TryFallback { primary: Vec<V3WorkflowOp>, fallback: Vec<V3WorkflowOp> },
}

impl crate::workflow_vm::VmOperation for V3WorkflowStep {
    fn effect(&self) -> crate::workflow::EffectClass {
        use crate::workflow::EffectClass;
        match self {
            Self::Snapshot { .. } | Self::Find { .. } | Self::FindAll { .. }
            | Self::CollectVirtual { .. } | Self::Choose { .. } | Self::Extract { .. }
            | Self::Verify { .. } => EffectClass::ReadOnly,
            Self::Click { .. } | Self::Upload { .. } | Self::Fill { .. }
            | Self::Select { .. } | Self::FillForm { .. } | Self::Type { .. } | Self::Press { .. }
            | Self::PageTool { .. } => EffectClass::Unknown,
        }
    }
    fn is_verification(&self) -> bool { matches!(self, Self::Verify { .. }) }
}

#[derive(Default)]
struct V3WorkflowState {
    collections: std::collections::BTreeMap<String, (Vec<Value>, bool)>,
    selected: std::collections::BTreeMap<String, Value>,
    coverage_complete: bool,
    pending_readback: bool,
    unverified_effect: bool,
}

struct V3WorkflowHost<'a> {
    app: &'a AppV3,
    args: &'a V3WorkflowArgs,
    state: Mutex<V3WorkflowState>,
}

impl crate::workflow_vm::BrowserVmHost<V3WorkflowStep> for V3WorkflowHost<'_> {
    fn execute<'a>(
        &'a self,
        step: V3WorkflowStep,
        variables: &'a std::collections::BTreeMap<String, String>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<crate::workflow_vm::StepReceipt, crate::workflow_vm::VmError>> + Send + 'a>> {
        Box::pin(async move {
            let started = std::time::Instant::now();
            let step_name = workflow_step_name(&step);
            let mut state = self.state.lock().await;
            let effect = crate::workflow_vm::VmOperation::effect(&step);
            let is_verify = crate::workflow_vm::VmOperation::is_verification(&step);
            if state.unverified_effect && !is_verify {
                return Err(crate::workflow_vm::VmError::UnknownMutation(
                    "a prior mutation still requires immediate read-only verification".into(),
                ));
            }
            let (mut value, variable_update) = match self.app.execute_v3_workflow_step(self.args, &mut state, step, variables).await {
                Ok(result) => result,
                Err(error) => {
                    let detail = error.to_string();
                    return Err(if effect == crate::workflow::EffectClass::ReadOnly {
                        crate::workflow_vm::VmError::ReadFailure(detail)
                    } else {
                        state.pending_readback = true;
                        state.unverified_effect = true;
                        crate::workflow_vm::VmError::UnknownMutation(detail)
                    });
                }
            };
            value["step"] = json!(step_name);
            value["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
            let detail = value.to_string();
            let status = value["status"].as_str();
            if effect != crate::workflow::EffectClass::ReadOnly && matches!(status, Some("unknown" | "dispatched")) {
                state.pending_readback = true;
                state.unverified_effect = true;
            }
            if is_verify && status == Some("passed") && state.pending_readback {
                state.pending_readback = false;
                state.unverified_effect = false;
            }
            match value["status"].as_str() {
                Some("unknown") if effect != crate::workflow::EffectClass::ReadOnly => return Err(crate::workflow_vm::VmError::UnknownMutation(detail)),
                Some("unknown" | "dispatched") if effect == crate::workflow::EffectClass::ReadOnly => return Err(crate::workflow_vm::VmError::ReadFailure(detail)),
                Some("not_dispatched") => return Err(crate::workflow_vm::VmError::NotDispatched(detail)),
                Some("failed") if effect == crate::workflow::EffectClass::ReadOnly => return Err(crate::workflow_vm::VmError::ReadFailure(detail)),
                Some("failed") => return Err(crate::workflow_vm::VmError::Failed(detail)),
                _ => {}
            }
            Ok(crate::workflow_vm::StepReceipt {
                status: value["status"].as_str().unwrap_or("completed").to_owned(),
                value: detail,
                variable_update,
            })
        })
    }
}

fn invalid(message: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(message.into(), None)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn enrich_linked_dates(app: &AppV3, session_id: &str, tab_id: &str, items: &mut [Value]) -> Result<(), String> {
    if items.len() > 12 { return Err("too many linked articles to inspect".into()); }
    let live = app.core.legacy.shared_sessions.lock().await.get(session_id).cloned().ok_or("unknown session")?;
    let live = live.lock().await;
    let connection = live.connection.as_ref().ok_or("session not accepted")?;
    let registry = live.registry().map_err(|error| error.to_string())?;
    let before = super::super::shared_frame_identity(connection, registry, &live.handle, tab_id).await?;
    let hrefs = items.iter().map(|item| item["href"].as_str().unwrap_or_default()).collect::<Vec<_>>();
    let expression = format!("({})({})", include_str!("../shared_page/linked_dates.js"), json!(hrefs));
    let response = tokio::time::timeout(Duration::from_secs(8), connection.command(registry, &live.handle, tab_id, "Runtime.evaluate", json!({"expression":expression,"awaitPromise":true,"returnByValue":true}))).await.map_err(|_| "linked article metadata timed out")?.map_err(|error| error.to_string())?;
    let dates = super::super::shared_value(&response)?;
    let after = super::super::shared_frame_identity(connection, registry, &live.handle, tab_id).await?;
    if before != after { return Err("document changed during linked article metadata read".into()); }
    let dates = dates.as_array().ok_or("linked article metadata was not an array")?;
    if dates.len() != items.len() { return Err("linked article metadata count changed".into()); }
    for (item, date) in items.iter_mut().zip(dates) {
        if date["href"] == item["href"] && date["datetime_utc_ms"].as_i64().is_some() {
            item["datetime"] = date["datetime"].clone();
            item["datetime_utc_ms"] = date["datetime_utc_ms"].clone();
            item["freshness_source"] = json!(if date["source"] == "listing_json_ld" { "listing_json_ld" } else { "linked_article_metadata" });
        } else {
            item["freshness_error"] = date.get("error").cloned().unwrap_or(json!("invalid_metadata"));
        }
    }
    Ok(())
}

fn linked_date_frontier(items: &[Value], limit: usize) -> Vec<Value> {
    let mut frontier = items.iter()
        .filter(|item| freshness_unresolved(item) && item["href"].as_str().is_some_and(|href| !href.is_empty()))
        .cloned()
        .collect::<Vec<_>>();
    frontier.sort_by(|a, b| {
        b["freshness_upper_bound_ms"].as_i64().cmp(&a["freshness_upper_bound_ms"].as_i64())
            .then_with(|| a["collection_order"].as_u64().cmp(&b["collection_order"].as_u64()))
            .then_with(|| a["reference"].as_str().cmp(&b["reference"].as_str()))
            .then_with(|| a["href"].as_str().cmp(&b["href"].as_str()))
    });
    frontier.truncate(limit.min(12));
    frontier
}

fn unresolved_linked_date_count(items: &[Value]) -> usize {
    items.iter().filter(|item| {
        freshness_unresolved(item)
            && item["href"].as_str().is_some_and(|href| !href.is_empty())
    }).count()
}

fn freshness_unresolved(item: &Value) -> bool {
    item["datetime_precision"] == "date" || item["datetime_utc_ms"].as_i64().is_none()
}

fn choose_newest_with_bounds(items: &[Value], coverage_complete: bool) -> Option<Value> {
    if !coverage_complete || items.is_empty() { return None; }
    if items.iter().any(|item| item["href"].as_str().is_none_or(str::is_empty)) { return None; }
    if unresolved_linked_date_count(items) == 0 {
        return crate::semantic_state::choose_newest(items, true).ok();
    }
    let mut exact = items.iter().filter_map(|item| {
        if freshness_unresolved(item) { None } else { item["datetime_utc_ms"].as_i64().map(|time| (time, item)) }
    }).collect::<Vec<_>>();
    exact.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    let (time, selected) = *exact.first()?;
    let unresolved = items.iter().filter(|item| freshness_unresolved(item)).collect::<Vec<_>>();
    let upper_bounds = unresolved.iter().map(|item| item["freshness_upper_bound_ms"].as_i64()).collect::<Option<Vec<_>>>()?;
    let max_upper_bound = *upper_bounds.iter().max()?;
    if time <= max_upper_bound || unresolved.iter().any(|item| item["href"].as_str().is_none_or(str::is_empty)) { return None; }
    let ties = exact.iter().take_while(|(candidate_time, _)| *candidate_time == time).map(|(_, item)| item["href"].clone()).collect::<Vec<_>>();
    Some(json!({"selected":selected,"freshness_evidence":{"datetime":selected["datetime"],"utc_ms":time,"candidate_count":items.len(),"basis":"listing_timestamp_beats_all_unchecked_upper_bounds","unique_latest":ties.len()==1,"tie_count":ties.len(),"equally_newest_hrefs":ties,"unchecked_upper_bound_ms":max_upper_bound}}))
}

const FILL_FORM_STEP: &str = r#"function(index,expected,value,select){const e=this.nodes[index],saved=this.items[index];if(!e||!saved||!e.isConnected)return {ok:false,reason:'detached'};if(this.raw(e)!==saved.raw_value||this.name(e)!==saved.name)return {ok:false,reason:'changed_target'};if(select?!(e instanceof HTMLSelectElement):!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)||!select&&['password','hidden','file','checkbox','radio','button','submit','reset','image'].includes(e.type||''))return {ok:false,reason:'unsupported_control'};const trusted=e.closest('[data-requires-trusted]'),visible=(()=>{for(let n=e;n;n=n.parentElement||n.getRootNode()?.host){const s=getComputedStyle(n),r=n.getBoundingClientRect();if(n.hidden||n.inert||s.display==='none'||s.visibility==='hidden'||Number.parseFloat(s.opacity||'1')<=0||r.width<=0||r.height<=0)return false}return true})();if(!visible)return {ok:false,reason:'not_visible'};if(e.matches(':disabled')||e.readOnly||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||trusted)return {ok:false,reason:'guard_rejected'};if(e.value!==expected)return {ok:false,reason:'stale_value'};if(select){const option=Array.from(e.options).find(option=>option.value===value&&!option.disabled);if(!option)return {ok:false,reason:'option_not_found'};e.value=value}else{const setter=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,value)}e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:select?null:value}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,reason:e.value===value?null:'readback_mismatch'}}"#;

fn fill_form_known_pre_dispatch_failure(receipt: &Value) -> bool {
    let Some(result) = receipt
        .pointer("/result/result/value")
        .or_else(|| receipt.pointer("/result/value"))
    else {
        return false;
    };
    result["ok"] == false
        && matches!(
            result["reason"].as_str(),
            Some(
                "detached"
                    | "changed_target"
                    | "unsupported_control"
                    | "not_visible"
                    | "guard_rejected"
                    | "stale_value"
                    | "option_not_found"
                    | "missing_value_setter"
            )
        )
}

async fn workflow_fill_form(
    app: &AppV3,
    args: &V3WorkflowArgs,
    fields: Vec<V3WorkflowFillField>,
    vars: &std::collections::BTreeMap<String, String>,
) -> Result<Value, rmcp::ErrorData> {
    let mut references = Vec::with_capacity(fields.len());
    for field in fields {
        let reference = crate::browser_workflow::resolve_workflow_reference(&field.reference, vars).map_err(invalid)?;
        references.push((resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, &reference).await?, field));
    }
    let live = app.core.legacy.shared_sessions.lock().await.get(&args.session_id).cloned().ok_or_else(|| invalid("unknown session"))?;
    let mut live = live.lock().await;
    if live.handle.principal != app.core.legacy.principal.as_ref() { return Err(invalid("session is not owned by this server principal")); }
    let snapshot = live.snapshots.get(&args.chrome_tab_id).cloned().ok_or_else(|| invalid("take browser_snapshot first"))?;
    let before = {
        let connection = live.connection.as_ref().ok_or_else(|| invalid("session is not accepted"))?;
        super::super::shared_frame_identity(connection, live.registry()?, &live.handle, &args.chrome_tab_id).await.map_err(invalid)?
    };
    if !crate::guard_reason::unrestricted_mode() && before != snapshot.identity { return Err(invalid("document changed since snapshot; take a fresh snapshot")); }
    let mut actions = Vec::with_capacity(references.len());
    for (legacy, field) in &references {
        let (token, index) = legacy.rsplit_once(':').ok_or_else(|| invalid("invalid retained reference"))?;
        let index = index.parse::<usize>().map_err(|_| invalid("invalid retained reference index"))?;
        if index >= snapshot.count || (!crate::guard_reason::unrestricted_mode() && (token != snapshot.token || snapshot.consumed && !super::super::internal_workflow_snapshot_allowed(&snapshot.token))) {
            return Err(invalid("stale or consumed reference; take a fresh snapshot"));
        }
        actions.push(controlla_browser::providers::SharedBatchAction {
            method: "Runtime.callFunctionOn".into(),
            params: json!({"objectId":snapshot.object_id,"functionDeclaration":FILL_FORM_STEP,"arguments":[{"value":index},{"value":field.expected_value},{"value":field.value},{"value":field.select}],"returnByValue":true}),
            stop_on_not_ok: true,
        });
    }
    if !crate::guard_reason::unrestricted_mode() {
        live.snapshots.get_mut(&args.chrome_tab_id).ok_or_else(|| invalid("snapshot disappeared before dispatch"))?.consumed = true;
    }
    let batch = {
        let connection = live.connection.as_ref().ok_or_else(|| invalid("session is not accepted"))?;
        connection.command_batch(live.registry()?, &live.handle, &args.chrome_tab_id, actions, Duration::from_secs(30)).await
    };
    let (completed, receipts, batch_error) = match batch {
        Ok(value) => (value["completed"].as_u64().unwrap_or_default() as usize, value["receipts"].as_array().cloned().unwrap_or_default(), value["batch_error"].as_str().map(str::to_owned).or_else(|| value["error"].as_str().map(str::to_owned))),
        Err(error) => return Ok(json!({"status":"unknown","action":"fill_form","dispatch_may_have_occurred":true,"reason":format!("fill form delivery is uncertain; inspect state before retrying: {error}")})),
    };
    let verification_started = std::time::Instant::now();
    let final_readback = if completed == references.len() && batch_error.is_none() && receipts.len() == references.len() {
        let expected = references.iter().map(|(reference, field)| {
            let index = reference.rsplit_once(':').and_then(|(_, index)| index.parse::<usize>().ok()).unwrap_or(usize::MAX);
            json!({"index":index,"value":field.value})
        }).collect::<Vec<_>>();
        let connection = live.connection.as_ref().ok_or_else(|| invalid("session is not accepted"))?;
        match connection.command(live.registry()?, &live.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
            "objectId":snapshot.object_id,
            "functionDeclaration":"function(expected){const checks=expected.map(({index,value})=>{const e=this.nodes[index];return !!e&&e.isConnected&&(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement||e instanceof HTMLSelectElement)&&e.value===value});return {ok:checks.length===expected.length&&checks.every(Boolean),checks}}",
            "arguments":[{"value":expected}],"returnByValue":true
        })).await {
            Ok(response) => super::super::shared_value(&response).unwrap_or_else(|error| json!({"ok":false,"reason":error})),
            Err(error) => json!({"ok":false,"reason":error.to_string()}),
        }
    } else {
        json!({"ok":false,"reason":"batch did not complete every field"})
    };
    let after = {
        let connection = live.connection.as_ref().ok_or_else(|| invalid("session is not accepted"))?;
        super::super::shared_frame_identity(connection, live.registry()?, &live.handle, &args.chrome_tab_id).await
    };
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Verification,
        verification_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    ).await;
    let all_verified = completed == references.len() && batch_error.is_none() && receipts.len() == references.len() && receipts.iter().all(|receipt| receipt.pointer("/result/result/value/ok").and_then(Value::as_bool) == Some(true) || receipt.pointer("/result/value/ok").and_then(Value::as_bool) == Some(true)) && final_readback["ok"] == true;
    let same_document = matches!(after, Ok(ref identity) if identity == &before);
    let final_checks = final_readback["checks"].as_array();
    let deltas = references.iter().zip(receipts.iter()).enumerate().map(|(index, ((legacy, field), receipt))| {
        let result = receipt.pointer("/result/result/value").or_else(|| receipt.pointer("/result/value"));
        let immediate = result.and_then(|result|result["ok"].as_bool()).unwrap_or(false);
        let final_ok = final_checks.and_then(|checks|checks.get(index)).and_then(Value::as_bool).unwrap_or(false);
        json!({"reference":legacy,"field_index":index,"verified":immediate&&final_ok,"reason_code":result.and_then(|result|result["reason"].as_str()).or(if immediate&&!final_ok{Some("final_readback_mismatch")}else{None}),"expected_value_supplied":!field.expected_value.is_empty(),"value_length":field.value.len()})
    }).collect::<Vec<_>>();
    let verified_fields = deltas.iter().filter(|delta| delta["verified"] == true).count();
    if all_verified && same_document {
        super::super::authorize_internal_workflow_snapshot(&snapshot.token);
        Ok(json!({"status":"verified","action":"fill_form","dispatch_acknowledged":true,"completed_fields":completed,"deltas":deltas,"final_readback":{"ok":true,"checks":final_checks},"document_changed":false}))
    } else if completed == 1 && receipts.first().is_some_and(fill_form_known_pre_dispatch_failure) {
        Ok(json!({"status":"not_dispatched","action":"fill_form","dispatch_acknowledged":true,"completed_fields":0,"attempted_fields":completed,"reason_code":"fill_guard_rejected","deltas":deltas}))
    } else {
        Ok(json!({"status":"unknown","action":"fill_form","dispatch_acknowledged":true,"dispatch_may_have_occurred":true,"completed_fields":verified_fields,"attempted_fields":completed,"document_changed":!same_document,"batch_error":batch_error,"deltas":deltas,"final_readback":final_readback,"reason":"fill form stopped before every field passed its final readback; inspect current values before retrying"}))
    }
}

fn fast_key_batch_failure(result: &Value, prior_key_events: u64) -> (&'static str, bool, bool) {
    let acknowledged = result
        .get("receipts")
        .and_then(Value::as_array)
        .map(|receipts| {
            receipts
                .iter()
                .filter(|receipt| {
                    receipt["method"] == "Input.dispatchKeyEvent" && receipt.get("error").is_none()
                })
                .count() as u64
        })
        .unwrap_or_default()
        .saturating_add(prior_key_events)
        > 0;
    let may_have_occurred = result["dispatch_may_have_occurred"] == true || acknowledged;
    let read_guard_rejected = result["batch_error"]
        .as_str()
        .is_some_and(|error| error.contains("read guard rejected"));
    let failed_on_guard = result["failed_method"] == "Runtime.callFunctionOn"
        && result["failed_at"].is_number()
        && !acknowledged;
    if !may_have_occurred && (read_guard_rejected || failed_on_guard) {
        ("not_dispatched", false, false)
    } else {
        ("unknown", acknowledged, may_have_occurred)
    }
}

fn workflow_should_stop(value: &Value) -> bool {
    matches!(
        value.get("status").and_then(Value::as_str),
        Some("unknown" | "not_dispatched" | "failed")
    )
}

fn workflow_should_verify_next(value: &Value, next: Option<&V3WorkflowStep>) -> bool {
    matches!(value["status"].as_str(), Some("unknown" | "dispatched"))
        && matches!(next, Some(V3WorkflowStep::Verify { .. }))
}

fn workflow_should_stop_before_next(value: &Value, next: Option<&V3WorkflowStep>) -> bool {
    workflow_should_stop(value) && !workflow_should_verify_next(value, next)
}

fn compact_workflow_receipt(mut value: Value, intermediate_snapshot: bool, mutation: bool) -> Value {
    let collection = if value["items"].is_array() {
        Some("items")
    } else if value["records"].is_array() {
        Some("records")
    } else if value["matches"].is_array() {
        Some("matches")
    } else {
        None
    };
    if intermediate_snapshot
        && (value["status"].is_null() || value["status"] == "collected")
        && let Some(collection) = collection
    {
        let count = value[collection].as_array().map_or(0, Vec::len);
        let encoded = serde_json::to_vec(&value).unwrap_or_default();
        use sha2::Digest;
        let evidence_hash = sha2::Sha256::digest(encoded).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        if let Some(object) = value.as_object_mut() {
            object.insert("item_count".into(), json!(count));
            object.insert("evidence_hash".into(), json!(evidence_hash));
            object.remove(collection);
            object.remove("text");
            object.remove("delta");
        }
    }
    if mutation && let Some(object) = value.as_object_mut() {
        object.remove("snapshot");
    }
    value
}

fn compile_fill_runs(steps: Vec<V3WorkflowStep>) -> Vec<V3WorkflowStep> {
    let mut compiled = Vec::with_capacity(steps.len());
    let mut steps = steps.into_iter().peekable();
    while let Some(step) = steps.next() {
        let (reference, expected_value, value, select) = match step {
            V3WorkflowStep::Fill { reference, expected_value, value } => (reference, expected_value, value, false),
            V3WorkflowStep::Select { reference, expected_value, value } => (reference, expected_value, value, true),
            other => {
                compiled.push(other);
                continue;
            }
        };
        let mut fields = vec![V3WorkflowFillField { reference, expected_value, value, select }];
        while fields.len() < 16 {
            match steps.peek() {
                Some(V3WorkflowStep::Fill { .. } | V3WorkflowStep::Select { .. }) => {
                    let next = steps.next().unwrap();
                    let (reference, expected_value, value, select) = match next {
                        V3WorkflowStep::Fill { reference, expected_value, value } => (reference, expected_value, value, false),
                        V3WorkflowStep::Select { reference, expected_value, value } => (reference, expected_value, value, true),
                        _ => unreachable!(),
                    };
                    fields.push(V3WorkflowFillField { reference, expected_value, value, select });
                }
                _ => break,
            }
        }
        compiled.push(if fields.len() == 1 {
            let field = fields.pop().unwrap();
            if field.select {
                V3WorkflowStep::Select { reference: field.reference, expected_value: field.expected_value, value: field.value }
            } else {
                V3WorkflowStep::Fill { reference: field.reference, expected_value: field.expected_value, value: field.value }
            }
        } else {
            V3WorkflowStep::FillForm { fields }
        });
    }
    compiled
}

fn compile_workflow_ops(ops: Vec<V3WorkflowOp>) -> Result<Vec<crate::workflow_vm::VmOp<V3WorkflowStep>>, rmcp::ErrorData> {
    fn compile_sequence(ops: Vec<V3WorkflowOp>) -> Result<Vec<crate::workflow_vm::VmOp<V3WorkflowStep>>, rmcp::ErrorData> {
        let mut result = Vec::new();
        let mut pending_steps = Vec::new();
        let flush = |pending: &mut Vec<V3WorkflowStep>, result: &mut Vec<crate::workflow_vm::VmOp<V3WorkflowStep>>| {
            result.extend(compile_fill_runs(std::mem::take(pending)).into_iter().map(crate::workflow_vm::VmOp::Step));
        };
        for op in ops {
            match op {
                V3WorkflowOp::Step { step } => pending_steps.push(step),
                V3WorkflowOp::If { condition, then_branch, else_branch } => {
                    flush(&mut pending_steps, &mut result);
                    if !crate::workflow_vm::condition_is_supported(&condition) { return Err(invalid("workflow if condition must be true, false, var_present:<name>, or var_equals:<name>:<value>")); }
                    result.push(crate::workflow_vm::VmOp::If { condition,
                        then_branch: compile_sequence(then_branch)?, else_branch: compile_sequence(else_branch)? });
                }
                V3WorkflowOp::Foreach { items, var, body } => {
                    flush(&mut pending_steps, &mut result);
                    if var.is_empty() || var.len() > 64 || !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        return Err(invalid("workflow foreach var must be 1..64 ASCII alphanumeric/underscore characters"));
                    }
                    result.push(crate::workflow_vm::VmOp::Foreach { items, var, body: compile_sequence(body)? });
                }
                V3WorkflowOp::TryFallback { primary, fallback } => {
                    flush(&mut pending_steps, &mut result);
                    result.push(crate::workflow_vm::VmOp::TryFallback { primary: compile_sequence(primary)?, fallback: compile_sequence(fallback)? });
                }
            }
        }
        flush(&mut pending_steps, &mut result);
        Ok(result)
    }
    compile_sequence(ops)
}

fn validate_workflow_ops(ops: &[V3WorkflowOp], depth: usize) -> Result<(), rmcp::ErrorData> {
    if depth > crate::workflow_vm::MAX_NESTING { return Err(invalid("workflow nesting exceeds the VM limit")); }
    let mut consecutive_steps = Vec::new();
    for op in ops {
        match op {
            V3WorkflowOp::Step { step } => consecutive_steps.push(step.clone()),
            V3WorkflowOp::If { condition, then_branch, else_branch } => {
                if !consecutive_steps.is_empty() { validate_workflow_steps(&consecutive_steps)?; consecutive_steps.clear(); }
                if !crate::workflow_vm::condition_is_supported(condition) { return Err(invalid("workflow if condition must be true, false, var_present:<name>, or var_equals:<name>:<value>")); }
                validate_workflow_ops(then_branch, depth + 1)?;
                validate_workflow_ops(else_branch, depth + 1)?;
            }
            V3WorkflowOp::Foreach { items, var, body } => {
                if !consecutive_steps.is_empty() { validate_workflow_steps(&consecutive_steps)?; consecutive_steps.clear(); }
                if items.len() > crate::workflow_vm::MAX_FOREACH_ITEMS || var.is_empty() || var.len() > 64 {
                    return Err(invalid("workflow foreach exceeds its item or variable-name limit"));
                }
                validate_workflow_ops(body, depth + 1)?;
            }
            V3WorkflowOp::TryFallback { primary, fallback } => {
                if !consecutive_steps.is_empty() { validate_workflow_steps(&consecutive_steps)?; consecutive_steps.clear(); }
                validate_workflow_ops(primary, depth + 1)?;
                validate_workflow_ops(fallback, depth + 1)?;
            }
        }
    }
    if !consecutive_steps.is_empty() { validate_workflow_steps(&consecutive_steps)?; }
    Ok(())
}

fn workflow_step_name(step: &V3WorkflowStep) -> &'static str {
    match step {
        V3WorkflowStep::Snapshot { .. } => "snapshot",
        V3WorkflowStep::Find { .. } => "find",
        V3WorkflowStep::FindAll { .. } => "find_all",
        V3WorkflowStep::CollectVirtual { .. } => "collect_virtual",
        V3WorkflowStep::Choose { .. } => "choose",
        V3WorkflowStep::Click { .. } => "click",
        V3WorkflowStep::Upload { .. } => "upload",
        V3WorkflowStep::Fill { .. } => "fill",
        V3WorkflowStep::Select { .. } => "select",
        V3WorkflowStep::FillForm { .. } => "fill_form",
        V3WorkflowStep::Type { .. } => "type",
        V3WorkflowStep::Press { .. } => "press",
        V3WorkflowStep::Extract { .. } => "extract",
        V3WorkflowStep::PageTool { .. } => "page_tool",
        V3WorkflowStep::Verify { .. } => "verify",
    }
}

fn virtual_item_identity(item: &Value, fields: &[String]) -> Option<String> {
    let values = fields.iter().map(|field| item.get(field).cloned().unwrap_or(Value::Null)).collect::<Vec<_>>();
    values.iter().any(|value| value.as_str().is_some_and(|text| !text.is_empty()))
        .then(|| serde_json::to_string(&values).unwrap_or_default())
}

const VIRTUAL_SCROLL_WITH_RECORDS: &str = r#"async function(selector,rowLimit){let e;try{e=selector?document.querySelector(selector):document.scrollingElement}catch(_){return {ok:false,reason:'invalid_container_selector'}}if(!e||(selector&&document.querySelectorAll(selector).length!==1))return {ok:false,reason:'container_not_unique'};const root=selector?e:(document.body||document.documentElement),recordSelector='[role="row"],[role="listitem"],tr,li,article,[data-row],[data-record],a[href]',candidates=new Set(selector?Array.from(root.children):[]),walker=document.createTreeWalker(root,NodeFilter.SHOW_ELEMENT);let node,scanned=0,scanLimit=4096;while((node=walker.nextNode())){if(++scanned>scanLimit)break;if(node.matches(recordSelector))candidates.add(node)}for(const parent of Array.from(candidates)){const children=Array.from(parent.children);if(children.length<2||children.length>rowLimit+1)continue;const signature=child=>child.tagName+'|'+String(child.className||'');if(children.every(child=>signature(child)===signature(children[0]))){candidates.delete(parent);for(const child of children)candidates.add(child)}}const nested=new Set();for(const item of candidates)for(let p=item.parentElement;p&&p!==root;p=p.parentElement)if(candidates.has(p))nested.add(p);const records=[];for(const item of candidates){if(nested.has(item)||item.closest('[data-masked],[data-requires-trusted],[hidden],[inert],input[type="password"],input[type="file"]')||item.querySelector('[data-masked],[data-requires-trusted],input[type="password"],input[type="file"]'))continue;const s=getComputedStyle(item),b=item.getBoundingClientRect();if(s.display==='none'||s.visibility==='hidden'||Number.parseFloat(s.opacity||'1')<=0||b.width<=0||b.height<=0)continue;let base=item;for(let p=item.parentElement;p&&p!==root;p=p.parentElement)if(candidates.has(p)){base=p;break}const link=item.matches('a[href]')?item:base.querySelector('a[href]'),text=((link&&link.innerText)||base.innerText||'').replace(/\s+/g,' ').trim().slice(0,320),datetimeNode=base.matches('time[datetime],[datetime]')?base:base.querySelector('time[datetime],[datetime]'),datetime=datetimeNode?.dateTime||datetimeNode?.getAttribute('datetime')||datetimeNode?.getAttribute('data-published-at')||null;if(!text&&!link?.href)continue;const role=item.getAttribute('role')||(link?'link':'row'),record={role,name:text,text,href:link?.href||null,datetime,source_frame_url:location.href};if(datetime&&/^\d{4}-\d{2}-\d{2}$/.test(datetime)){const [y,m,d]=datetime.split('-').map(Number);record.datetime_precision='date';record.freshness_upper_bound_ms=Date.UTC(y,m-1,d+1,12)-1}else if(datetime){const parsed=Date.parse(datetime);if(Number.isFinite(parsed)){record.datetime_precision='instant';record.datetime_utc_ms=parsed}}records.push(record);if(records.length>rowLimit)break}const recordsTruncated=scanned>scanLimit||records.length>rowLimit,visibleRecords=records.slice(0,rowLimit),signature=()=>((root.innerText||'').slice(0,4096)+'|'+Array.from(root.querySelectorAll('a[href]'),a=>a.getAttribute('href')+' '+a.innerText).join('|').slice(0,8192)),before=signature(),top=e.scrollTop,height=e.clientHeight||innerHeight,max=e.scrollHeight,prior=top;let observer;const changed=new Promise(resolve=>{observer=new MutationObserver(()=>{if(signature()!==before){observer.disconnect();resolve(true)}});observer.observe(root,{subtree:true,childList:true,attributes:true,characterData:true})});if(top+height>=max-2){const semantic_delta=await Promise.race([changed,new Promise(resolve=>setTimeout(()=>resolve(false),350))]);observer.disconnect();return {ok:true,records:visibleRecords,records_truncated:recordsTruncated,at_end:!semantic_delta&&e.scrollTop+(e.clientHeight||innerHeight)>=e.scrollHeight-2,advanced:semantic_delta,semantic_delta}}e.scrollTop=Math.min(max,top+Math.max(100,height*.8));await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));let semantic_delta=signature()!==before;if(!semantic_delta)semantic_delta=await Promise.race([changed,new Promise(resolve=>setTimeout(()=>resolve(false),250))]);observer.disconnect();return {ok:true,records:visibleRecords,records_truncated:recordsTruncated,at_end:e.scrollTop+(e.clientHeight||innerHeight)>=e.scrollHeight-2,advanced:e.scrollTop>prior||semantic_delta,semantic_delta}}"#;

fn task_receipt(kind: &str, url: &str, reference: Option<&str>, value: &Value) -> Value {
    let snippet = if kind == "extract" {
        value["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    } else {
        value["reason"].as_str().unwrap_or_default().to_owned()
    };
    let bounded = crate::v3_tasks::EvidenceReceipt {
        url: url.to_owned(),
        snippet,
        affected_controls: reference.into_iter().map(str::to_owned).collect(),
        verified: matches!(value["status"].as_str(), Some("verified" | "passed"))
            || kind == "extract"
                && value["items"].is_array()
                && value["truncated"] == false
                && value["omissions"].as_array().is_some_and(Vec::is_empty),
    }
    .bounded();
    json!({"kind":kind,"status":value["status"],"url":bounded.url,"snippet":bounded.snippet,
        "affected_controls":bounded.affected_controls,"verified":bounded.verified,
        "reason_code":value["reason_code"],"dispatch_acknowledged":value["dispatch_acknowledged"],
        "dispatch_may_have_occurred":value["dispatch_may_have_occurred"],
        "root_document_matches":value["root_document_matches"],"account_marker_matches":value["account_marker_matches"],
        "artifact_handle": if kind == "download" { value["artifact_handle"].clone() } else { Value::Null },
        "filename": if matches!(kind, "download" | "upload") { value["filename"].clone() } else { Value::Null },
        "size": if matches!(kind, "download" | "upload") { value["size"].clone() } else { Value::Null },
        "chrome_selected_file": if kind == "upload" { value["chrome_selected_file"].clone() } else { Value::Null },
        "selection_readback_verified": if kind == "upload" { value["selection_readback_verified"].clone() } else { Value::Null },
        "file_input_consumed_by_page": if kind == "upload" { value["file_input_consumed_by_page"].clone() } else { Value::Null },
        "application_accepted": if kind == "upload" { value["application_accepted"].clone() } else { Value::Null },
        "network_events": if kind == "upload" { value["network_events"].clone() } else { Value::Null },
        "network_observation_window_ms": if kind == "upload" { value["network_observation_window_ms"].clone() } else { Value::Null },
        "persisted": if kind == "upload" { value["persisted"].clone() } else { Value::Null },
        "sha256": if kind == "download" { value["sha256"].clone() } else { Value::Null },
        "item_count": if kind == "extract" { value["items"].as_array().map(Vec::len) } else { None },
        "limited": if kind == "extract" { value["truncated"].as_bool() } else { None }})
}

async fn shared_file_select_trigger(
    app: &AppV3,
    session_id: &str,
    tab_id: &str,
    artifact_handle: &str,
    trigger_reference: &str,
    account_marker: &(String, String),
    acceptance: Option<Value>,
) -> Result<Value, rmcp::ErrorData> {
    let acceptance_predicate: Option<BrowserPredicate> = acceptance.map(serde_json::from_value)
        .transpose().map_err(|error|invalid(format!("invalid upload acceptance predicate: {error}")))?;
    let live=app.core.legacy.shared_sessions.lock().await.get(session_id).cloned().ok_or_else(||invalid("unknown session_id"))?;
    let (filename,size,path,before)= {
        let live=live.lock().await;
        if live.handle.principal!=app.core.legacy.principal.as_ref() || !crate::guard_reason::unrestricted_mode() || super::super::private_headless_cleanup_allowed(&live.handle) {
            return Ok(json!({"status":"unsupported","reason":"trigger-based upload requires the owning local unrestricted shared Chrome session"}));
        }
        let registry=live.registry()?;
        let handle:controlla_browser::sessions::ArtifactHandle=serde_json::from_value(json!(artifact_handle)).map_err(|error|invalid(format!("invalid artifact handle: {error}")))?;
        let metadata=registry.artifact_metadata(&live.handle,&handle).map_err(|error|invalid(format!("artifact lookup failed: {error:?}")))?;
        let path=registry.artifact_path_for(&live.handle.id,&live.handle.principal,&handle).map_err(|error|invalid(format!("artifact path lookup failed: {error:?}")))?.to_string_lossy().into_owned();
        let connection=live.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;
        let before=super::super::shared_frame_identity(connection,registry,&live.handle,tab_id).await.map_err(invalid)?;
        let marker_selector = shared_upload_account_marker_selector(&account_marker.0).map_err(invalid)?;
        controlla_browser::identity_marker_command(marker_selector,&account_marker.1).map_err(|error|invalid(format!("invalid account marker: {error}")))?;
        (metadata.filename,metadata.size,path,before)
    };
    if !shared_upload_account_marker_matches(app, session_id, tab_id, &account_marker.0, &account_marker.1).await.map_err(invalid)? {
        return Ok(json!({"status":"not_dispatched","reason_code":"account_marker_mismatch","reason":"account marker did not match before file selection"}));
    }
    let acceptance_previously_passed = if let Some(predicate)=&acceptance_predicate {
        verify_impl(&app.core,&BrowserVerifyArgs{session_id:session_id.to_owned(),chrome_tab_id:tab_id.to_owned(),predicate:predicate.clone()}).await?["status"]=="passed"
    } else { false };
    if let Some(frame_reference) = trigger_reference.strip_prefix("@f:") {
        let parts=frame_reference.split(':').collect::<Vec<_>>();
        let backend=parts.get(2).and_then(|value|value.parse::<u64>().ok());
        if parts.len()!=3 || parts[0].is_empty() || parts[1].is_empty() || backend.is_none() {
            return Ok(json!({"status":"not_dispatched","reason_code":"invalid_frame_reference","dispatch_may_have_occurred":false}));
        }
        let selection=async {
            let live=live.lock().await;
            let registry=live.registry()?;
            let connection=live.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;
            connection.command(registry,&live.handle,tab_id,"Controlla.frameSelectFile",json!({"frame_id":parts[0],"frame_loader_id":parts[1],"trigger_backend_node_id":backend.unwrap(),"file_path":path})).await.map_err(invalid)
        }.await;
        let selection=match selection {
            Ok(value)=>shared_extension_value(&value).map_err(invalid)?,
            Err(error)=>return Ok(json!({"status":"unknown","reason_code":"frame_upload_transport_uncertain","reason":error.to_string(),"dispatch_may_have_occurred":true})),
        };
        let input_consumed_by_page = upload_picker_consumed_input(&selection);
        if selection["status"]!="selected" && !input_consumed_by_page {
            let reason_code=upload_selection_reason_code(&selection["status"]);
            return Ok(json!({"status":selection["status"],"reason_code":reason_code,"reason":selection["reason"],"dispatch_acknowledged":selection["dispatch_acknowledged"],"network_events":selection["network_events"],"network_observation_window_ms":selection["network_observation_window_ms"]}));
        }
        let files=selection["files"].as_array();
        let readback_matches=files.is_some_and(|files|files.len()==1&&files[0]["name"]==filename&&files[0]["size"].as_u64()==Some(size as u64));
        let document_matches=async {
            let live=live.lock().await;
            let registry=live.registry()?;
            let connection=live.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;
            let current=super::super::shared_frame_identity(connection,registry,&live.handle,tab_id).await;
            Ok::<_,rmcp::ErrorData>(matches!(current,Ok(ref current) if current==&before))
        }.await?;
        let marker_matches=shared_upload_account_marker_matches(app, session_id, tab_id, &account_marker.0, &account_marker.1).await.is_ok_and(|matches| matches);
        if !document_matches || !marker_matches {
            return Ok(json!({"status":"unknown","reason_code":"file_selection_unverified","chrome_selected_file":readback_matches,"root_document_matches":document_matches,"account_marker_matches":marker_matches,"dispatch_acknowledged":true,"reason":"frame file readback, document identity, or account marker did not match"}));
        }
        let mut application_accepted=Value::Null;
        if let Some(predicate)=acceptance_predicate {
            for attempt in 0..10 {
                let observed=verify_impl(&app.core,&BrowserVerifyArgs{session_id:session_id.to_owned(),chrome_tab_id:tab_id.to_owned(),predicate:predicate.clone()}).await?;
                if upload_acceptance_transition(acceptance_previously_passed,observed["status"]=="passed") {application_accepted=Value::Bool(true);break;}
                if attempt<9 {tokio::time::sleep(Duration::from_millis(100)).await;}
            }
            if application_accepted!=Value::Bool(true) {application_accepted=Value::Bool(false);}
        }
        let accepted=application_accepted==Value::Bool(true);
        if !readback_matches {
            if upload_input_selection_confirmed(input_consumed_by_page, document_matches, marker_matches, accepted) {
                return Ok(json!({"status":"verified","verified":true,"dispatch_acknowledged":true,"chrome_selected_file":true,"selection_readback_verified":false,"file_input_consumed_by_page":true,"application_accepted":true,"persisted":null,"filename":filename,"size":size,"file_chooser_frame_id":parts[0],"upload_route":"frame_trigger","scope":"local_unrestricted_shared_session","network_events":selection["network_events"],"network_observation_window_ms":selection["network_observation_window_ms"],"reason":"The picker consumed the file input; the requested application acceptance was observed"}));
            }
            return Ok(json!({"status":"unknown","reason_code":"file_selection_unverified","chrome_selected_file":false,"selection_readback_verified":false,"file_input_consumed_by_page":input_consumed_by_page,"root_document_matches":document_matches,"account_marker_matches":marker_matches,"application_accepted":application_accepted,"dispatch_acknowledged":true,"reason":"file input readback was empty and application acceptance was not observed"}));
        }
        return Ok(json!({"status":if accepted {"verified"} else {"selected"},"verified":accepted,"dispatch_acknowledged":true,"chrome_selected_file":true,"application_accepted":application_accepted,"persisted":null,"filename":filename,"size":size,"file_chooser_frame_id":parts[0],"upload_route":"frame_trigger","scope":"local_unrestricted_shared_session"}));
    }
    let baseline = async {
        let live=live.lock().await;
        let registry=live.registry().map_err(|error|error.to_string())?;
        let connection=live.connection.as_ref().ok_or_else(||"shared extension session has not been accepted".to_owned())?;
        let response=connection.command(registry,&live.handle,tab_id,"Controlla.fileInputSnapshot",Value::Null).await?;
        shared_file_input_baseline(&response)
    }.await;
    let baseline = match baseline {
        Ok(baseline) => baseline,
        Err(reason) => return Ok(json!({"status":"not_dispatched","reason_code":"file_input_baseline_unavailable","dispatch_may_have_occurred":false,"reason":format!("file-input baseline failed before the upload trigger: {reason}")})),
    };
    let cleanup_runtime = tokio::runtime::Handle::current();
    let mut chooser_cleanup = FileChooserCleanupGuard::new({
        let live = live.clone();
        let tab_id = tab_id.to_owned();
        move || {
            cleanup_runtime.spawn(async move { disable_file_chooser_interception(&live, &tab_id).await; });
        }
    });
    {
        let live=live.lock().await;
        let registry=live.registry()?;
        let connection=live.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;
        connection.command(registry,&live.handle,tab_id,"Page.enable",Value::Null).await.map_err(invalid)?;
        connection.command(registry,&live.handle,tab_id,"Page.setInterceptFileChooserDialog",json!({"enabled":true})).await.map_err(invalid)?;
    }
    let clicked_result=app.act(Parameters(V3ActArgs {
        session_id:session_id.to_owned(),chrome_tab_id:tab_id.to_owned(),action:"click".into(),reference:Some(trigger_reference.to_owned()),expected_value:None,value:None,key:None,
        outcome:Some(BrowserClickOutcome::Focused),timeout_ms:Some(2_000),typing_mode:None,delay_ms:None,client_id:None,page_tool_name:None,page_tool_input:None,

        allow_password: None,
    })).await;
    let clicked=match clicked_result { Ok(result)=>result.0, Err(error)=>{chooser_cleanup.disable(disable_file_chooser_interception(&live,tab_id)).await;return Err(error)} };
    if !upload_trigger_may_have_dispatched(&clicked) {
        chooser_cleanup.disable(disable_file_chooser_interception(&live,tab_id)).await;
        return Ok(json!({"status":"not_dispatched","reason_code":clicked["reason_code"],"dispatch_acknowledged":false,"dispatch_may_have_occurred":false,"trigger_status":"not_dispatched","reason":clicked["reason"]}));
    }
    let event_result = wait_for_chooser_or_cleanup(
        async {
            let live=live.lock().await;
            match (live.connection.as_ref(), live.registry()) {
                (Some(connection), Ok(registry)) => connection.command(registry,&live.handle,tab_id,"Controlla.waitForFileChooser",json!({"timeout_ms":750})).await.map_err(|error|error.to_string()),
                (None, _) => Err("shared extension session has not been accepted".to_owned()),
                (_, Err(error)) => Err(error.to_string()),
            }
        },
        chooser_cleanup.disable(disable_file_chooser_interception(&live,tab_id)),
    ).await;
    let (event,upload_route)=match event_result {
        Ok(event)=>match shared_extension_value(&event) {
            Ok(event) if event["backend_node_id"].as_i64().is_some() && event["frame_id"].as_str().is_some() && event["document_id"].as_str().is_some() => (event,"chooser_event"),
            _ => {
                chooser_cleanup.disable(disable_file_chooser_interception(&live,tab_id)).await;
                return Ok(json!({"status":"unknown","reason_code":"invalid_file_chooser_event","dispatch_may_have_occurred":true,"trigger_status":clicked["status"],"reason":"chooser event identity or backend node was incomplete; inspect state before retrying"}));
            }
        },
        Err(error)=>{
            let fallback=async {
                let live=live.lock().await;
                let registry=live.registry()?;
                let connection=live.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;
                connection.command(registry,&live.handle,tab_id,"Controlla.findFileInput",json!({"baseline":baseline})).await.map_err(invalid)
            }.await;
            let fallback=match fallback.and_then(|value|shared_extension_value(&value).map_err(invalid)) {
                Ok(value) if value["backend_node_id"].as_i64().is_some() && value["frame_id"].as_str().is_some() && value["frame_loader_id"].as_str().is_some() && value["document_id"].as_str().is_some() => value,
                Ok(_) => return Ok(json!({"status":"unknown","reason_code":"file_chooser_not_observed","dispatch_may_have_occurred":upload_trigger_may_have_dispatched(&clicked),"trigger_status":clicked["status"],"chooser_wait_ms":750,"lazy_input_fallback":"invalid_result","reason":"no chooser event or valid lazy file input was observed; inspect the page before retrying"})),
                Err(fallback_error) => return Ok(json!({"status":"unknown","reason_code":"file_chooser_not_observed","dispatch_may_have_occurred":upload_trigger_may_have_dispatched(&clicked),"trigger_status":clicked["status"],"chooser_wait_ms":750,"lazy_input_fallback":"not_found_or_ambiguous","reason":format!("no chooser event ({error}); lazy-input fallback also failed ({fallback_error}); inspect the page before retrying")})),
            };
            (fallback,"lazy_input_fallback")
        }
    };
    let selected_result:Result<(bool,bool),rmcp::ErrorData>=async {
        let live=live.lock().await;
        let connection=live.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;
        let registry=live.registry()?;
        if event["document_id"].as_str().is_none_or(str::is_empty) || event["frame_id"].as_str().is_none_or(str::is_empty) || event["frame_loader_id"].as_str().is_none_or(str::is_empty) { return Err(invalid("chooser event omitted its paired document or frame identity")); }
        let tree=connection.command(registry,&live.handle,tab_id,"Page.getFrameTree",Value::Null).await.map_err(invalid)?;
        if super::super::find_cdp_frame(&tree,event["frame_id"].as_str().unwrap()).is_none_or(|(loader,_)|Some(loader.as_str())!=event["frame_loader_id"].as_str()) { return Err(invalid("chooser frame navigated before file selection")); }
        let readback=connection.command(registry,&live.handle,tab_id,"Controlla.setFrameFileInputFiles",json!({"frame_id":event["frame_id"],"frame_loader_id":event["frame_loader_id"],"backend_node_id":event["backend_node_id"],"file_path":path})).await.map_err(|error|invalid(format!("file selection delivery is uncertain: {error}")))?;
        let selection=shared_extension_value(&readback).map_err(invalid)?;
        let files=selection["files"].clone();
        let selected=files.as_array().is_some_and(|files|files.len()==1&&files[0]["name"]==filename&&files[0]["size"].as_u64()==Some(size as u64));
        let current=super::super::shared_frame_identity(connection,registry,&live.handle,tab_id).await;
        Ok((selected,matches!(current,Ok(ref current) if current==&before)))
    }.await;
    {
        chooser_cleanup.disable(disable_file_chooser_interception(&live,tab_id)).await;
    }
    let selected=match selected_result { Ok(value)=>value, Err(error)=>return Ok(json!({"status":"unknown","reason_code":"file_selection_unverified","dispatch_acknowledged":true,"reason":format!("file selection may have occurred but readback failed; inspect before retrying: {error}")})) };
    let marker_matches=shared_upload_account_marker_matches(app, session_id, tab_id, &account_marker.0, &account_marker.1).await.is_ok_and(|matches| matches);
    if !selected.0 || !selected.1 || !marker_matches { return Ok(json!({"status":"unknown","chrome_selected_file":selected.0,"reason_code":"file_selection_unverified","reason":"selected file readback, document identity, or account marker did not match; inspect before retrying","dispatch_acknowledged":true})); }
    let mut application_accepted=Value::Null;
    if let Some(predicate)=acceptance_predicate {
        for attempt in 0..10 {
            let observed=verify_impl(&app.core,&BrowserVerifyArgs{session_id:session_id.to_owned(),chrome_tab_id:tab_id.to_owned(),predicate:predicate.clone()}).await?;
            if upload_acceptance_transition(acceptance_previously_passed, observed["status"]=="passed") { application_accepted=Value::Bool(true);break; }
            if attempt<9 { tokio::time::sleep(Duration::from_millis(100)).await; }
        }
        if application_accepted!=Value::Bool(true) { application_accepted=Value::Bool(false); }
    }
    let accepted=application_accepted==Value::Bool(true);
    Ok(json!({"status":if accepted {"verified"} else {"selected"},"verified":accepted,"dispatch_acknowledged":true,"chrome_selected_file":true,"application_accepted":application_accepted,"persisted":null,"filename":filename,"size":size,"file_chooser_frame_id":event["frame_id"],"upload_route":upload_route,"scope":"local_unrestricted_shared_session","reason":if accepted {Value::Null} else {json!("Chrome selected and read back the staged file; application acceptance or persistence was not verified")}}))
}

async fn shared_account_marker_matches(
    connection: &super::super::PageSessionTransport,
    registry: &controlla_browser::sessions::SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
    selector: &str,
    expected: &str,
) -> Result<bool, String> {
    let value = connection.command(registry, handle, tab_id, "Controlla.checkIdentityMarker", json!({"selector":selector,"expected":expected})).await?;
    identity_marker_matches(&value)
}

async fn shared_upload_account_marker_matches(
    app: &AppV3,
    session_id: &str,
    tab_id: &str,
    selector: &str,
    expected: &str,
) -> Result<bool, String> {
    let selector = shared_upload_account_marker_selector(selector)?;
    let live = app.core.legacy.shared_sessions.lock().await.get(session_id).cloned()
        .ok_or_else(|| "unknown session_id".to_owned())?;
    let live = live.lock().await;
    let registry = live.registry().map_err(|error| error.to_string())?;
    let connection = live.connection.as_ref().ok_or_else(|| "shared extension session has not been accepted".to_owned())?;
    shared_account_marker_matches(connection, registry, &live.handle, tab_id, selector, expected).await
}

fn shared_upload_account_marker_selector(selector: &str) -> Result<&str, String> {
    match selector {
        "ax:button" => Ok("button,[role='button']"),
        value if value.starts_with("ax:") => Err("account role marker currently supports only ax:button".into()),
        value => Ok(value),
    }
}

fn identity_marker_matches(response: &Value) -> Result<bool, String> {
    let value = if response.get("matches").is_some() {
        response
    } else if response.get("exceptionDetails").is_some() {
        return Err("page evaluation failed; take a fresh snapshot".into());
    } else {
        response
            .pointer("/result/value")
            .ok_or_else(|| "identity marker check returned no result".to_owned())?
    };
    let matches = value["matches"]
        .as_bool()
        .ok_or_else(|| "identity marker check returned no boolean".to_owned())?;
    let count = value["match_count"]
        .as_u64()
        .ok_or_else(|| "identity marker check returned no match count".to_owned())?;
    Ok(matches && count == 1)
}

async fn wait_for_chooser_or_cleanup<T, E, C>(
    wait: impl std::future::Future<Output = Result<T, E>>,
    cleanup: C,
) -> Result<T, E>
where
    C: std::future::Future,
{
    let result = wait.await;
    if result.is_err() { cleanup.await; }
    result
}

async fn disable_file_chooser_interception(
    live: &Arc<Mutex<super::SharedLiveSession>>,
    tab_id: &str,
) -> bool {
    let live = live.lock().await;
    if let (Some(connection), Ok(registry)) = (live.connection.as_ref(), live.registry()) {
        connection.command(registry,&live.handle,tab_id,"Page.setInterceptFileChooserDialog",json!({"enabled":false})).await.is_ok()
    } else {
        false
    }
}

struct FileChooserCleanupGuard(Option<Box<dyn FnOnce() + Send>>);

impl FileChooserCleanupGuard {
    fn new(on_drop: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(on_drop)))
    }

    async fn disable(&mut self, result: impl std::future::Future<Output = bool>) {
        if result.await {
            self.0 = None;
        }
    }
}

impl Drop for FileChooserCleanupGuard {
    fn drop(&mut self) {
        if let Some(on_drop) = self.0.take() { on_drop(); }
    }
}

fn upload_acceptance_transition(was_passed: bool, is_passed: bool) -> bool {
    !was_passed && is_passed
}

fn upload_input_selection_confirmed(input_consumed_by_page: bool, document_matches: bool, marker_matches: bool, application_accepted: bool) -> bool {
    input_consumed_by_page && document_matches && marker_matches && application_accepted
}

fn upload_picker_consumed_input(selection: &Value) -> bool {
    selection["status"]=="unknown"
        && selection["dispatch_acknowledged"]==true
        && selection["reason"].as_str().is_some_and(|reason| reason.contains("File input did not retain one selected file for readback"))
}

fn upload_selection_reason_code(status: &Value) -> &'static str {
    if status=="unknown" { "file_selection_unverified" } else { "frame_upload_failed" }
}

fn upload_trigger_may_have_dispatched(click: &Value) -> bool {
    click["status"] != "not_dispatched"
}

fn shared_extension_value(response: &Value) -> Result<Value, String> {
    if response.get("exceptionDetails").is_some() {
        return Err("page evaluation failed; take a fresh snapshot".into());
    }
    if let Some(value) = response.pointer("/result/value") {
        return Ok(value.clone());
    }
    response
        .as_object()
        .map(|_| response.clone())
        .ok_or_else(|| "shared extension response omitted an object result".into())
}

fn shared_file_input_baseline(response: &Value) -> Result<Value, String> {
    let value = shared_extension_value(response)?;
    let inputs = value["inputs"]
        .as_array()
        .ok_or_else(|| "file-input snapshot omitted its inputs array".to_owned())?;
    if inputs.len() > 256 {
        return Err("file-input snapshot exceeded the 256-input limit".into());
    }
    Ok(Value::Array(inputs.clone()))
}

async fn headless_file_select(
    app: &AppV3,
    session_id: &str,
    tab_id: &str,
    artifact_handle: &str,
    selector: &str,
    account_marker: &(String, String),
) -> Result<Value, rmcp::ErrorData> {
    let live = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown session_id"))?;
    let live = live.lock().await;
    if live.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    if !super::super::private_headless_cleanup_allowed(&live.handle) && !crate::guard_reason::unrestricted_mode() {
        return Ok(json!({"status":"unsupported","reason":"file selection requires private headless or local unrestricted shared scope"}));
    }
    if selector.is_empty() || selector.len() > 512 || selector.chars().any(char::is_control) {
        return Ok(json!({"status":"not_dispatched","field":"selector","reason_code":"invalid_selector"}));
    }
    let connection = live.connection.as_ref().ok_or_else(|| invalid("browser connection unavailable"))?;
    let registry = live.registry()?;
    let preflight = format!("(()=>{{let n;try{{n=[...document.querySelectorAll({})]}}catch{{return {{reason:'invalid_selector'}}}}if(!n.length)return {{reason:'no_match'}};if(n.length!==1)return {{reason:'ambiguous_match',count:n.length}};const e=n[0];if(!(e instanceof HTMLInputElement)||e.type!=='file')return {{reason:'unsupported_target_type'}};return {{ok:true}}}})()",json!(selector));
    let checked=connection.command(registry,&live.handle,tab_id,"Runtime.evaluate",json!({"expression":preflight,"returnByValue":true})).await.map_err(invalid)?;
    let checked=super::super::shared_value(&checked).map_err(invalid)?;
    if let Some(reason)=checked["reason"].as_str() { return Ok(json!({"status":"not_dispatched","field":"selector","reason_code":reason,"count":checked["count"]})); }
    if crate::guard_reason::unrestricted_mode()
        && !super::super::private_headless_cleanup_allowed(&live.handle)
    {
        let handle: controlla_browser::sessions::ArtifactHandle = serde_json::from_value(json!(artifact_handle))
            .map_err(|error| invalid(format!("invalid artifact handle: {error}")))?;
        let metadata = registry.artifact_metadata(&live.handle, &handle)
            .map_err(|error| invalid(format!("artifact lookup failed: {error:?}")))?;
        let path = registry.artifact_path_for(&live.handle.id, &live.handle.principal, &handle)
            .map_err(|error| invalid(format!("artifact path lookup failed: {error:?}")))?
            .to_string_lossy().into_owned();
        let expression = format!("document.querySelector({})", json!(selector));
        let element=connection.command(registry,&live.handle,tab_id,"Runtime.evaluate",json!({"expression":expression,"returnByValue":false})).await.map_err(invalid)?;
        let object_id=element["result"]["objectId"].as_str().ok_or_else(||invalid("unsupported_target_type: selector did not resolve to a file input object"))?;
        let before=super::super::shared_frame_identity(connection,registry,&live.handle,tab_id).await.map_err(invalid)?;
        controlla_browser::identity_marker_command(&account_marker.0,&account_marker.1).map_err(|error| invalid(error.to_string()))?;
        if !shared_account_marker_matches(connection,registry,&live.handle,tab_id,&account_marker.0,&account_marker.1).await.map_err(invalid)? { return Ok(json!({"status":"not_dispatched","reason_code":"account_marker_mismatch","dispatch_acknowledged":false})); }
        connection.command(registry, &live.handle, tab_id, "DOM.setFileInputFiles",
            json!({"files":[path],"objectId":object_id})).await.map_err(invalid)?;
        let readback = connection.command(registry, &live.handle, tab_id, "Runtime.callFunctionOn",
            json!({"objectId":object_id,"functionDeclaration":"function(){return Array.from(this.files||[]).map(f=>({name:f.name,size:f.size}));}","returnByValue":true})).await.map_err(invalid)?;
        let files = super::super::shared_value(&readback).map_err(invalid)?;
        let selected = files.as_array().is_some_and(|items| items.len() == 1
            && items[0]["name"] == metadata.filename
            && items[0]["size"].as_u64() == Some(metadata.size as u64));
        let after=super::super::shared_frame_identity(connection,registry,&live.handle,tab_id).await;
        let marker_after=shared_account_marker_matches(connection,registry,&live.handle,tab_id,&account_marker.0,&account_marker.1).await;
        let identity_ok=matches!(after,Ok(ref identity) if identity==&before);
        let account_ok=matches!(marker_after,Ok(true));
        if !identity_ok||!account_ok{return Ok(json!({"status":"unknown","reason_code":"identity_changed_after_selection","dispatch_acknowledged":true,"chrome_selected_file":selected,"account_marker_checked":true}));}
        return Ok(if selected {
            json!({"status":"selected","verified":false,"chrome_selected_file":true,"application_accepted":null,"persisted":null,"filename":metadata.filename,"size":metadata.size,"account_marker_checked":true,"scope":metadata.scope})
        } else {
            json!({"status":"unknown","reason":"file selection was dispatched but readback did not match","dispatch_acknowledged":true})
        });
    }
    if live.handle.principal != app.core.legacy.principal.as_ref()
        || !super::super::private_headless_cleanup_allowed(&live.handle)
    {
        return Ok(
            json!({"status":"unsupported","reason":"file selection requires a dedicated private headless session"}),
        );
    }
    let Some(super::super::PageSessionTransport::Dedicated(Some(browser))) =
        live.connection.as_ref()
    else {
        return Ok(
            json!({"status":"unsupported","reason":"dedicated browser connection is unavailable"}),
        );
    };
    if browser.target_id() != tab_id {
        return Err(invalid("upload target is not the dedicated session target"));
    }
    let registry = live.registry()?;
    let before = super::super::shared_frame_identity(
        live.connection.as_ref().unwrap(),
        registry,
        &live.handle,
        tab_id,
    )
    .await
    .map_err(invalid)?;
    let reference = browser
        .connection()
        .capture_target_ref(registry, &live.handle, tab_id, &before.0, 1, 1)
        .await
        .map_err(|error| invalid(format!("upload target changed: {error:?}")))?;
    let revisions = controlla_browser::sessions::IdentityRevisions {
        account: 1,
        document: 1,
    };
    let marker = controlla_browser::identity_marker_command(&account_marker.0, &account_marker.1)
        .map_err(|error| invalid(format!("invalid account marker: {error}")))?;
    let marker_check = browser
        .connection()
        .target_ref_command(
            registry,
            &reference,
            &live.handle.principal,
            revisions,
            "Runtime.evaluate",
            marker.clone(),
        )
        .await;
    if !matches!(marker_check, Ok(ref response) if response.pointer("/result/value") == Some(&Value::Bool(true)))
    {
        return Ok(
            json!({"status":"not_dispatched","reason":"account marker did not match before file selection"}),
        );
    }
    let handle: controlla_browser::sessions::ArtifactHandle =
        serde_json::from_value(json!(artifact_handle))
            .map_err(|error| invalid(format!("invalid artifact handle: {error}")))?;
    let guard = controlla_browser::GuardSnapshot {
        navigation: reference.frame_revision,
        account: 1,
        document: 1,
        dependencies: Default::default(),
        strict_background: true,
        requires_native: false,
    };
    let locator = controlla_browser::SemanticLocator::Css(selector.to_owned());
    let selection = browser
        .connection()
        .select_file_input_artifact(
            registry,
            &reference,
            &live.handle.principal,
            revisions,
            controlla_browser::GuardedFileSelection {
                expected: &guard,
                current: &guard,
                locator: &locator,
                handle: &handle,
            },
        )
        .await;
    let evidence = match selection {
        Ok(evidence) => evidence,
        Err(error) => {
            return Ok(
                json!({"status":"unknown","reason":format!("file selection may have occurred; inspect state before retrying: {error}")}),
            );
        }
    };
    let after = super::super::shared_frame_identity(
        live.connection.as_ref().unwrap(),
        registry,
        &live.handle,
        tab_id,
    )
    .await;
    let marker_after = browser
        .connection()
        .target_ref_command(
            registry,
            &reference,
            &live.handle.principal,
            revisions,
            "Runtime.evaluate",
            marker,
        )
        .await;
    if !matches!(after, Ok(ref after) if after == &before)
        || !matches!(marker_after, Ok(ref response) if response.pointer("/result/value") == Some(&Value::Bool(true)))
    {
        return Ok(
            json!({"status":"unknown","reason":"file was selected but document or account marker changed; inspect before retrying"}),
        );
    }
    // shortcut: selection readback stops before app acceptance, upgrade when the app exposes a persistence verifier.
    Ok(
        json!({"status":"selected","verified":false,"chrome_selected_file":true,"application_accepted":null,"persisted":null,"filename":evidence.filename,"size":evidence.size,
        "reason":"Chrome selected and read back the staged file; application transfer and persistence are unverified"}),
    )
}

fn valid_download_guid(guid: &str) -> bool {
    !guid.is_empty()
        && guid.len() <= 128
        && guid
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

async fn task_navigate(app: &AppV3, session_id: &str, tab_id: &str, url: &str) -> Value {
    let (before_loader, before_url, before_ready_state) = match app.readiness_document(session_id, tab_id).await {
        Ok(state) => state,
        Err(error) => return json!({"status":"not_dispatched","reason":error}),
    };
    let live = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(session_id)
        .cloned();
    let Some(live) = live else {
        return json!({"status":"unknown","reason":"unknown browser session"});
    };
    let navigation = {
        let live = live.lock().await;
        if live.handle.principal != app.core.legacy.principal.as_ref() {
            return json!({"status":"not_dispatched","reason":"session principal changed"});
        }
        if super::super::private_headless_cleanup_allowed(&live.handle) {
            let Some(transport) = live.connection.as_ref() else {
                return json!({"status":"not_dispatched","reason":"headless connection unavailable"});
            };
            transport
                .command(
                    match live.registry() {
                        Ok(registry) => registry,
                        Err(error) => {
                            return json!({"status":"not_dispatched","reason":error.to_string()});
                        }
                    },
                    &live.handle,
                    tab_id,
                    "Page.navigate",
                    json!({"url":url}),
                )
                .await
                .map_err(|error| error.to_string())
        } else {
            drop(live);
            app.core
                .legacy
                .shared_tab(Parameters(super::SharedTabArgs {
                    session_id: session_id.to_owned(),
                    action: "navigate".into(),
                    chrome_tab_id: Some(tab_id.to_owned()),
                    url: url.to_owned(),
                    idempotency_key: None,
                    wait_until: None,
                }))
                .await
                .map(|Json(value)| value)
                .map_err(|error| error.to_string())
        }
    };
    if let Ok(value) = &navigation
        && let Some(error) = page_navigate_error(value)
    {
        return json!({"status":"failed","dispatch":"acknowledged","dispatch_acknowledged":true,
            "document_committed":false,"current_url":null,"destination_match":false,
            "timed_out_waiting_for":null,"reason":format!("Chrome rejected the navigation: {error}")});
    }
    let dispatch = if let Err(error) = navigation {
        let observed = verify_impl(&app.core, &BrowserVerifyArgs {
            session_id: session_id.to_owned(), chrome_tab_id: tab_id.to_owned(),
            predicate: BrowserPredicate::Url { equals: url.to_owned() },
        }).await.ok();
        return json!({"status":if observed.as_ref().is_some_and(|v| v["status"] == "passed") { "incomplete" } else { "unknown" },
            "dispatch":"unknown","document_committed":observed.as_ref().is_some_and(|v| v["status"] == "passed"),
            "current_url":observed.as_ref().and_then(|v| v["observed"].as_str()),
            "destination_match":observed.as_ref().is_some_and(|v| v["status"] == "passed"),
            "timed_out_waiting_for":"navigation_receipt","reason":format!("navigation delivery uncertain; inspect before retrying: {error}")});
    } else { "acknowledged" };
    app.core
        .semantic
        .lock()
        .await
        .remove(&super::tab_key(session_id, tab_id));
    for _ in 0..20 {
        if let Ok((loader, current_url, ready_state)) = app.readiness_document(session_id, tab_id).await
            && current_url == url && ready_state != "loading"
            && (loader != before_loader || (before_url == url && before_ready_state != "loading"))
        {
            return json!({"status":"verified","dispatch":dispatch,"dispatch_acknowledged":true,
                "document_committed":loader != before_loader,"current_url":current_url,
                "ready_state":ready_state,"destination_match":true,"timed_out_waiting_for":null,
                "postcondition":{"status":"passed","observed_url":current_url}});
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let state = app.readiness_document(session_id, tab_id).await.ok();
    let destination_match = state.as_ref().is_some_and(|(_, current_url, _)| current_url == url);
    json!({"status":if destination_match { "incomplete" } else { "unknown" },"dispatch":dispatch,
        "dispatch_acknowledged":true,"document_committed":state.as_ref().is_some_and(|(loader, _, _)| loader != &before_loader),"current_url":state.as_ref().map(|(_, url, _)| url),
        "ready_state":state.as_ref().map(|(_, _, ready)| ready),"destination_match":destination_match,
        "timed_out_waiting_for":if destination_match { "document_readiness" } else { "destination" },
        "reason":"navigation was acknowledged but its final state was not verified within the bounded wait"})
}

async fn presentation_state(app: &AppV3, session_id: &str, tab_id: &str) -> Option<Value> {
    let live = app.core.legacy.shared_sessions.lock().await.get(session_id).cloned()?;
    let live = live.lock().await;
    let connection = live.connection.as_ref()?;
    connection.command(live.registry().ok()?, &live.handle, tab_id, "Controlla.presentationState", Value::Null).await.ok()
}

fn focus_changed_during_operation(before: &Value, after: &Value) -> Value {
    if !before.is_object() || !after.is_object() {
        return json!("unknown");
    }
    let state_changed = before["tab_active"] != after["tab_active"]
        || before["window_focused"] != after["window_focused"];
    if state_changed {
        return Value::Bool(true);
    }
    match (
        before["activation_epoch"].as_u64(),
        after["activation_epoch"].as_u64(),
        before["focus_epoch"].as_u64(),
        after["focus_epoch"].as_u64(),
    ) {
        (Some(before_activation), Some(after_activation), Some(before_focus), Some(after_focus)) => {
            Value::Bool(before_activation != after_activation || before_focus != after_focus)
        }
        _ => json!("unknown"),
    }
}

fn action_timeout_receipt(action: &str) -> Value {
    json!({"status":"unknown","action":action,"dispatch_acknowledged":null,
        "dispatch_may_have_occurred":true,
        "reason":"action exceeded its 60-second bound; inspect state before retrying"})
}

fn decode_artifact_bytes(
    bytes: Option<Vec<u8>>,
    bytes_base64: Option<String>,
) -> Result<Option<Vec<u8>>, rmcp::ErrorData> {
    let decoded = match (bytes, bytes_base64) {
        (Some(bytes), None) => Some(bytes),
        (None, Some(encoded)) => Some(crate::artifacts::decode_base64(&encoded).map_err(invalid)?),
        (Some(_), Some(_)) => return Err(invalid("provide exactly one of bytes or bytes_base64")),
        (None, None) => None,
    };
    if decoded.as_ref().is_some_and(|data| data.is_empty() || data.len() > 10 * 1024 * 1024) {
        return Err(invalid("artifact bytes must be between 1 and 10485760 bytes"));
    }
    Ok(decoded)
}

fn encode_artifact_read(bytes: &[u8], encoding: &str) -> Result<Value, rmcp::ErrorData> {
    match encoding {
        "base64" => Ok(json!({"bytes_base64":base64::engine::general_purpose::STANDARD.encode(bytes)})),
        "bytes" => Ok(json!({"bytes":bytes})),
        _ => Err(invalid("artifact encoding must be base64 or bytes")),
    }
}

fn page_navigate_error(value: &Value) -> Option<&str> {
    value.get("errorText").and_then(Value::as_str)
        .or_else(|| value.pointer("/result/errorText").and_then(Value::as_str))
}

async fn headless_download(
    app: &AppV3,
    session_id: &str,
    tab_id: &str,
    short_ref: &str,
) -> Result<Value, rmcp::ErrorData> {
    use std::io::Read;
    let live = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown session_id"))?;
    {
        let preflight = live.lock().await;
        if preflight.handle.principal != app.core.legacy.principal.as_ref()
            || !super::super::private_headless_cleanup_allowed(&preflight.handle)
        {
            return Ok(
                json!({"status":"unsupported","reason":"download requires a dedicated private headless session"}),
            );
        }
    }
    let _ = snapshot_impl(
        &app.core,
        &BrowserSnapshotArgs {
            session_id: session_id.to_owned(),
            chrome_tab_id: tab_id.to_owned(),
            selector: None,
            max_items: None,
            max_text_chars: None,
            max_bytes: None,
            href_hint: None,
            frame_id: None,
            goal: None,
            max_scan_nodes: None,
        },
    )
    .await?;
    let retained = resolve_reference(&app.core, session_id, tab_id, short_ref).await?;
    let mut live = live.lock().await;
    if live.handle.principal != app.core.legacy.principal.as_ref()
        || !super::super::private_headless_cleanup_allowed(&live.handle)
    {
        return Ok(
            json!({"status":"unsupported","reason":"download session changed before dispatch"}),
        );
    }
    let Some(super::super::PageSessionTransport::Dedicated(Some(browser))) =
        live.connection.as_ref()
    else {
        return Ok(
            json!({"status":"unsupported","reason":"dedicated browser connection is unavailable"}),
        );
    };
    if browser.target_id() != tab_id {
        return Err(invalid(
            "download target is not the dedicated session target",
        ));
    }
    let snapshot = live
        .snapshots
        .get(tab_id)
        .cloned()
        .ok_or_else(|| invalid("take a fresh snapshot before download"))?;
    let (token, index) = retained
        .rsplit_once(':')
        .ok_or_else(|| invalid("invalid retained download reference"))?;
    let index = index
        .parse::<usize>()
        .map_err(|_| invalid("invalid retained download index"))?;
    if token != snapshot.token || index >= snapshot.count || snapshot.consumed {
        return Err(invalid("download reference was stale or consumed"));
    }
    let transport = live.connection.as_ref().unwrap();
    let registry = live.registry()?;
    let before = super::super::shared_frame_identity(transport, registry, &live.handle, tab_id)
        .await
        .map_err(invalid)?;
    if before != snapshot.identity {
        return Ok(json!({"status":"not_dispatched","reason":"document changed before download"}));
    }
    let guard = crate::guard_reason::click_guard_script(super::super::SHARED_CLICK_UNSAFE_PREDICATE);
    let check = |object_id: &str, index: usize| {
        json!({"objectId":object_id,
        "functionDeclaration":guard,"arguments":[{"value":index},{"value":{"kind":"download"}}],"returnByValue":true})
    };
    let first = transport
        .command(
            registry,
            &live.handle,
            tab_id,
            "Runtime.callFunctionOn",
            check(&snapshot.object_id, index),
        )
        .await
        .map_err(invalid)?;
    let first = super::super::shared_value(&first).map_err(invalid)?;
    if first["ok"] != true {
        return Ok(crate::guard_reason::refusal(&first));
    }
    let directory = browser.profile_directory().join("controlla-downloads");
    if directory.exists() {
        if !directory.is_dir()
            || directory
                .symlink_metadata()
                .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(invalid(
                "private download directory is not a regular directory",
            ));
        }
    } else {
        std::fs::create_dir(&directory).map_err(|error| {
            invalid(format!("cannot create private download directory: {error}"))
        })?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                invalid(format!(
                    "cannot restrict private download directory: {error}"
                ))
            },
        )?;
    }
    let browser_connection = browser.connection().clone();
    if let Err(error) = browser_connection
        .command(
            None,
            "Browser.setDownloadBehavior",
            json!({
                "behavior":"allowAndName","downloadPath":directory,"eventsEnabled":true
            }),
        )
        .await
    {
        return Ok(
            json!({"status":"unsupported","reason":format!("Chrome download behavior unavailable before dispatch: {error}")}),
        );
    }
    let cursor = browser_connection.latest_event_sequence();
    let recheck = transport
        .command(
            registry,
            &live.handle,
            tab_id,
            "Runtime.callFunctionOn",
            check(&snapshot.object_id, index),
        )
        .await;
    let recheck = recheck
        .ok()
        .and_then(|value| super::super::shared_value(&value).ok());
    let fresh =
        super::super::shared_frame_identity(transport, registry, &live.handle, tab_id).await;
    if !matches!(recheck, Some(ref value) if value["ok"] == true)
        || !matches!(fresh, Ok(ref identity) if identity == &before)
    {
        return Ok(
            json!({"status":"not_dispatched","reason":"download target or document changed before dispatch"}),
        );
    }
    let x = recheck
        .as_ref()
        .and_then(|value| value["x"].as_f64())
        .ok_or_else(|| invalid("download guard omitted x"))?;
    let y = recheck
        .as_ref()
        .and_then(|value| value["y"].as_f64())
        .ok_or_else(|| invalid("download guard omitted y"))?;
    live.snapshots.get_mut(tab_id).unwrap().consumed = true;
    let transport = live.connection.as_ref().unwrap();
    let registry = live.registry()?;
    let press = transport
        .command(
            registry,
            &live.handle,
            tab_id,
            "Input.dispatchMouseEvent",
            json!({"type":"mousePressed","button":"left","clickCount":1,"x":x,"y":y}),
        )
        .await;
    let release = transport
        .command(
            registry,
            &live.handle,
            tab_id,
            "Input.dispatchMouseEvent",
            json!({"type":"mouseReleased","button":"left","clickCount":1,"x":x,"y":y}),
        )
        .await;
    let dispatch_acknowledged = press.is_ok() && release.is_ok();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut event_cursor = cursor;
    let mut guid = None::<String>;
    let mut filename = None::<String>;
    let mut completed_bytes = None::<u64>;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = browser_connection
            .next_event_after(event_cursor, remaining)
            .await;
        let event = match event {
            Ok(event) => event,
            Err(_) => break,
        };
        event_cursor = event.sequence;
        match event.value["method"].as_str() {
            Some("Browser.downloadWillBegin") if event.value["params"]["frameId"] == before.0 => {
                let candidate = event.value["params"]["guid"].as_str().unwrap_or_default();
                if !valid_download_guid(candidate) {
                    return Ok(
                        json!({"status":"unknown","reason":"Chrome emitted an invalid download GUID"}),
                    );
                }
                guid = Some(candidate.to_owned());
                let suggested = event.value["params"]["suggestedFilename"]
                    .as_str()
                    .unwrap_or("download.bin");
                filename = Some(
                    if suggested.is_empty()
                        || suggested.len() > 255
                        || suggested.contains(['/', '\\', '\0'])
                        || suggested.chars().any(char::is_control)
                    {
                        "download.bin".to_owned()
                    } else {
                        suggested.to_owned()
                    },
                );
            }
            Some("Browser.downloadProgress")
                if guid.as_deref() == event.value["params"]["guid"].as_str() =>
            {
                if event.value["params"]["state"] == "canceled" {
                    return Ok(json!({"status":"failed","reason":"Chrome canceled the download"}));
                }
                if event.value["params"]["state"] == "completed" {
                    completed_bytes = event.value["params"]["receivedBytes"].as_u64();
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(guid) = guid else {
        return Ok(
            json!({"status":"unknown","dispatch_acknowledged":dispatch_acknowledged,
            "reason":"no download begin event matched the selected frame before the deadline; inspect before retrying"}),
        );
    };
    let Some(completed_bytes) = completed_bytes else {
        return Ok(
            json!({"status":"unknown","dispatch_acknowledged":dispatch_acknowledged,
            "reason":"download did not complete before the deadline; inspect before retrying"}),
        );
    };
    let path = directory.join(&guid);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return Ok(
                json!({"status":"failed","reason":"Chrome completed the download but its private-profile file is unavailable"}),
            );
        }
    };
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > 10 * 1024 * 1024
        || metadata.len() != completed_bytes
    {
        return Ok(
            json!({"status":"failed","reason":"completed download did not pass regular-file, size, or event-byte verification"}),
        );
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let Ok(file) = std::fs::File::open(&path) else {
        return Ok(
            json!({"status":"failed","reason":"verified download file could not be opened"}),
        );
    };
    if file
        .take(10 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Ok(json!({"status":"failed","reason":"verified download file could not be read"}));
    }
    if bytes.len() as u64 != completed_bytes {
        return Ok(json!({"status":"failed","reason":"download file changed during verification"}));
    }
    let sha256 = crate::artifacts::ArtifactExpectation::sha256(&bytes);
    let session_handle = live.handle.clone();
    let Some(registry) = live.registry.as_mut() else {
        return Ok(
            json!({"status":"incomplete","reason":"download completed but session staging is unavailable"}),
        );
    };
    let artifact = match registry.put_artifact_bytes(
        &session_handle,
        filename.as_deref().unwrap_or("download.bin"),
        &bytes,
    ) {
        Ok(artifact) => artifact,
        Err(_) => {
            return Ok(
                json!({"status":"incomplete","reason":"download completed but bounded artifact staging failed"}),
            );
        }
    };
    Ok(
        json!({"status":"verified","artifact_handle":artifact.handle,"filename":artifact.filename,
        "size":artifact.size,"sha256":sha256,"browser_guid":guid,"dispatch_acknowledged":dispatch_acknowledged,
        "reason":"Chrome reported completion and a bounded private-profile file matched the event byte count and SHA-256 readback"}),
    )
}

fn workflow_find_reference(found: &Value) -> Result<String, rmcp::ErrorData> {
    let matches = found["matches"]
        .as_array()
        .ok_or_else(|| invalid("workflow find returned invalid matches"))?;
    if matches.len() != 1 {
        return Err(invalid(format!("workflow find resolved {} controls; expected exactly one", matches.len())));
    }
    matches[0]["reference"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("workflow find omitted its reference"))
}

fn resolve_workflow_predicate(
    predicate: BrowserPredicate,
    vars: &std::collections::BTreeMap<String, String>,
    selected: &std::collections::BTreeMap<String, Value>,
) -> Result<BrowserPredicate, rmcp::ErrorData> {
    let resolve = |reference: String| {
        crate::browser_workflow::resolve_workflow_reference(&reference, vars).map_err(invalid)
    };
    Ok(match predicate {
        BrowserPredicate::Text { reference, equals } => BrowserPredicate::Text {
            reference: resolve(reference)?, equals,
        },
        BrowserPredicate::Value { reference, equals } => BrowserPredicate::Value {
            reference: resolve(reference)?, equals,
        },
        BrowserPredicate::Exists { reference } => BrowserPredicate::Exists {
            reference: resolve(reference)?,
        },
        BrowserPredicate::Url { equals } => BrowserPredicate::Url {
            equals: if let Some(name) = equals.strip_prefix('$').and_then(|value| value.strip_suffix(".href")) {
                selected.get(name).and_then(|item| item["href"].as_str())
                    .ok_or_else(|| invalid("selected href variable is unresolved"))?.to_owned()
            } else { equals },
        },
        BrowserPredicate::UrlStartsWith { prefix } => BrowserPredicate::UrlStartsWith { prefix },
    })
}

fn workflow_upload_requires_stop(value: &Value) -> bool {
    value["status"] != "verified" || value["application_accepted"] != true
}

fn validate_workflow_steps(steps: &[V3WorkflowStep]) -> Result<(), rmcp::ErrorData> {
    for (index, step) in steps.iter().enumerate() {
        if matches!(step, V3WorkflowStep::PageTool { .. } | V3WorkflowStep::Press { .. })
            && !matches!(steps.get(index + 1), Some(V3WorkflowStep::Verify { .. }))
        {
            return Err(invalid(
                "press and page_tool steps must be followed immediately by runtime verification",
            ));
        }
        if let V3WorkflowStep::Upload {
            artifact_handle,
            trigger_reference,
            account_marker,
            acceptance,
        } = step
        {
            let valid_acceptance = acceptance.as_ref().is_none_or(|predicate| {
                serde_json::to_vec(predicate).is_ok_and(|bytes| bytes.len() <= 8192)
            });
            if artifact_handle.is_empty()
                || artifact_handle.len() > 256
                || artifact_handle.contains('/')
                || artifact_handle.contains('\\')
                || trigger_reference.is_empty()
                || trigger_reference.len() > 1024
                || account_marker.0.is_empty()
                || account_marker.0.len() > 512
                || account_marker.1.is_empty()
                || account_marker.1.len() > 512
                || !valid_acceptance
            {
                return Err(invalid(
                    "upload requires an opaque artifact handle, bounded trigger reference and account marker, with an optional bounded acceptance predicate",
                ));
            }
        }
        if let V3WorkflowStep::CollectVirtual {
            container, query, role, identity_fields, max_scrolls, max_items, save_as,
        } = step
            && (container.as_ref().is_some_and(|value| value.is_empty() || value.len() > 512)
                || query.as_ref().is_some_and(|value| value.len() > 512)
                || role.as_ref().is_some_and(|value| value.is_empty() || value.len() > 64)
                || identity_fields.is_empty() || identity_fields.len() > 4
                || identity_fields.iter().any(|field| !matches!(field.as_str(), "href" | "name" | "text" | "datetime"))
                || *max_scrolls > 20 || *max_items == 0 || *max_items > 500
                || save_as.is_empty() || save_as.len() > 64)
        {
            return Err(invalid("collect_virtual requires bounded filters, 1..4 stable identity fields, at most 20 scrolls and 500 items, and a bounded save_as"));
        }
        if let V3WorkflowStep::Select { reference, expected_value, value } = step
            && (reference.is_empty() || reference.len() > 1024 || expected_value.len() > 16_384 || value.len() > 16_384)
        {
            return Err(invalid("select requires a bounded reference, expected value, and selected value"));
        }
        if let V3WorkflowStep::FillForm { fields } = step
            && (fields.is_empty() || fields.len() > 16 || fields.iter().any(|field| {
                field.reference.is_empty() || field.reference.len() > 1024
                    || field.expected_value.len() > 16_384 || field.value.len() > 16_384
            }))
        {
            return Err(invalid("fill_form requires 1..16 bounded references and values"));
        }
    }
    Ok(())
}

fn lease_target_key(_session_id: &str, tab_id: &str) -> crate::v3::TargetKey {
    crate::v3::TargetKey {
        // A tab may be paired through more than one MCP session; session IDs do not identify Chrome.
        browser_id: "shared-extension".into(),
        tab_id: tab_id.to_owned(),
    }
}

fn parse_mode(mode: Option<&str>) -> Result<crate::v3::BrowserMode, rmcp::ErrorData> {
    Ok(match mode.unwrap_or("foreground") {
        "foreground" => crate::v3::BrowserMode::Foreground,
        "background" => crate::v3::BrowserMode::Background,
        "headless" => crate::v3::BrowserMode::Headless,
        _ => return Err(invalid("mode must be foreground, background, or headless")),
    })
}

fn typing_policy(
    mode: Option<&str>,
    delay_ms: Option<u64>,
    text: &str,
    requires_key_events: bool,
) -> Result<crate::v3::TypingPolicy, &'static str> {
    let mode = match mode {
        None => crate::v3::TypingPolicy::choose(text, requires_key_events, false).mode,
        Some("block") => crate::v3::TypingMode::Block,
        Some("fast_keys") => crate::v3::TypingMode::FastKeys,
        Some("human_keys") => crate::v3::TypingMode::HumanKeys,
        Some("ime") => crate::v3::TypingMode::Ime,
        _ => return Err("typing_mode must be block, fast_keys, human_keys, or ime"),
    };
    crate::v3::TypingPolicy::new(mode, delay_ms)
}

async fn requires_key_events(app: &AppV3, args: &V3ActArgs) -> bool {
    let Some(reference) = args.reference.as_deref() else {
        return false;
    };
    app.core
        .semantic
        .lock()
        .await
        .get(&super::tab_key(&args.session_id, &args.chrome_tab_id))
        .and_then(|state| state.items.get(reference))
        .is_some_and(|item| item["requires_trusted_events"] == true)
}

// shortcut: rich key entry collapses Enter-created innerText runs in pairs; use structured block readback for editors with different semantics.
const EDITOR_GUARD: &str = r#"function(index,expected,prepare=false,normalizeParagraphs=false,requireFocus=true){let e=this.nodes[index],rebound=false;const valueOf=n=>{const rich=n instanceof HTMLElement&&!(n instanceof HTMLInputElement||n instanceof HTMLTextAreaElement)&&(n.isContentEditable||n.getAttribute('role')==='textbox');const raw=rich?(n.innerText??n.textContent??'').replace(/\r\n/g,'\n').replace(/\u00a0/g,' '):n.value;return rich?(raw==='\n'?'':normalizeParagraphs?raw.replace(/\n{2,}/g,breaks=>'\n'.repeat(Math.ceil(breaks.length/2))):raw):raw};if(!e||!e.isConnected){if(!prepare||!e)return {ok:false,reason:'target_replaced'};const candidates=Array.from(document.querySelectorAll('[contenteditable],[role=\"textbox\"]')).filter(n=>n.tagName===e.tagName&&n.className===e.className&&n.getAttribute('role')===e.getAttribute('role')&&n.getAttribute('aria-label')===e.getAttribute('aria-label')&&n.getAttribute('data-placeholder')===e.getAttribute('data-placeholder')&&valueOf(n)===(n instanceof HTMLElement&&!(n instanceof HTMLInputElement||n instanceof HTMLTextAreaElement)&&(n.isContentEditable||n.getAttribute('role')==='textbox')&&expected==='\n'?'':expected));if(candidates.length!==1)return {ok:false,reason:'target_replaced',candidates:candidates.length};this.nodes[index]=e=candidates[0];rebound=true}const rich=e instanceof HTMLElement&&!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)&&e.getAttribute('aria-readonly')!=='true'&&e.getAttribute('contenteditable')?.toLowerCase()!=='false'&&(e.isContentEditable||e.getAttribute('role')==='textbox'),plain=e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type);if(!rich&&!plain)return {ok:false,reason:'not_editable'};const trustedBoundary=e.closest('[data-requires-trusted]');if(e.matches(':disabled,[aria-disabled=\"true\"]')||e.readOnly||e.getAttribute('aria-readonly')==='true'||e.getAttribute('contenteditable')?.toLowerCase()==='false'||e.closest('[data-masked]')||trustedBoundary&&trustedBoundary!==e)return {ok:false,reason:'blocked'};const value=valueOf(e);if(value!==(rich&&expected==='\n'?'':expected))return {ok:false,reason:'stale_value',value};if(prepare){e.focus();if(rich){const s=e.ownerDocument.getSelection(),r=e.ownerDocument.createRange();r.selectNodeContents(e);r.collapse(false);s.removeAllRanges();s.addRange(r)}else e.setSelectionRange(e.value.length,e.value.length)}const s=e.ownerDocument.getSelection(),focused=e.ownerDocument.activeElement===e,atEnd=rich?s?.isCollapsed&&s.anchorNode&&e.contains(s.anchorNode)&&(()=>{const after=e.ownerDocument.createRange();after.selectNodeContents(e);after.setStart(s.anchorNode,s.anchorOffset);return after.toString()===''})():e.selectionStart===e.selectionEnd&&e.selectionEnd===e.value.length;if(requireFocus&&(!focused||!atEnd))return {ok:false,reason:'focus_or_selection_changed',value,focused,atEnd,rebound};return {ok:true,rich,value,focused,atEnd,rebound}}"#;

async fn rich_editor_reference(app: &AppV3, args: &V3ActArgs, reference: &str) -> bool {
    app.core.semantic.lock().await
        .get(&super::tab_key(&args.session_id, &args.chrome_tab_id))
        .and_then(|state| state.items.get(reference))
        .is_some_and(|item| item["editable_kind"] == "rich_text")
}

async fn acquire_lease(
    app: &AppV3,
    args: &V3ActArgs,
) -> Result<crate::v3::TargetKey, rmcp::ErrorData> {
    let owner = args.client_id.as_deref().unwrap_or("local-mcp");
    let key = lease_target_key(&args.session_id, &args.chrome_tab_id);
    if crate::guard_reason::unrestricted_mode() {
        return Ok(key);
    }
    app.leases
        .lock()
        .await
        .acquire(key.clone(), owner, now_ms(), 120_000)
        .map_err(|lease| {
            invalid(format!(
                "target is leased by {}; retry after observing fresh state",
                lease.owner
            ))
        })?;
    Ok(key)
}

async fn release_lease(app: &AppV3, args: &V3ActArgs, key: &crate::v3::TargetKey) {
    let owner = args.client_id.as_deref().unwrap_or("local-mcp");
    let _ = app.leases.lock().await.release(key, owner);
}

async fn block_type(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
    let reference = args
        .reference
        .as_deref()
        .ok_or_else(|| invalid("type requires reference"))?;
    let expected = args.expected_value.clone().unwrap_or_default();
    let appended = args.value.clone().unwrap_or_default();
    if appended.is_empty() || expected.len().saturating_add(appended.len()) > 16_384 {
        return Err(invalid(
            "type value must be nonempty and final value <= 16384 bytes",
        ));
    }
    let rich = rich_editor_reference(app,args,reference).await;
    if rich {
        return rich_block_type(app, args, reference, &expected, &appended, "block").await;
    }
    let mut final_value = expected.clone();
    final_value.push_str(&appended);
    retained_mutation(
        &app.core,
        &BrowserActArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            action: "fill".into(),
            reference: Some(reference.to_owned()),
            expected_value: Some(expected),
            value: Some(final_value),
            outcome: None,
            timeout_ms: args.timeout_ms,
        },
        reference,
        false,
        "fill",
    )
    .await
    .map(|mut value| {
        value["action"] = json!("type");
        value["typing_mode"] = json!("block");
        value
    })
}

async fn rich_block_type(app: &AppV3, args: &V3ActArgs, reference: &str, expected: &str, text: &str, typing_mode: &str) -> Result<Value, rmcp::ErrorData> {
    let legacy_reference = resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
    let live = app.core.legacy.shared_sessions.lock().await.get(&args.session_id).cloned().ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = live.lock().await;
    if shared.handle.principal != app.core.legacy.principal.as_ref() { return Err(invalid("session is not owned by this server principal")); }
    let snapshot = shared.snapshots.get(&args.chrome_tab_id).cloned().ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token,index)=legacy_reference.rsplit_once(':').ok_or_else(||invalid("invalid retained reference"))?;
    let index=index.parse::<usize>().map_err(|_|invalid("invalid retained reference index"))?;
    if index>=snapshot.count || token!=snapshot.token || snapshot.consumed { return Err(invalid("stale or consumed reference; take one fresh browser_snapshot")); }
    let identity={let connection=shared.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;super::super::shared_frame_identity(connection,shared.registry()?,&shared.handle,&args.chrome_tab_id).await.map_err(invalid)?};
    if identity!=snapshot.identity { return Err(invalid("document changed since snapshot; take a fresh browser_snapshot")); }
    let prepared={let connection=shared.connection.as_ref().unwrap();connection.command(shared.registry()?,&shared.handle,&args.chrome_tab_id,"Runtime.callFunctionOn",json!({"objectId":snapshot.object_id,"functionDeclaration":EDITOR_GUARD,"arguments":[{"value":index},{"value":expected},{"value":true}],"returnByValue":true})).await.map_err(|error|invalid(error.to_string()))?};
    let prepared=super::super::shared_value(&prepared).map_err(invalid)?;
    if prepared["ok"]!=true { return Ok(json!({"status":"not_dispatched","action":"type","typing_mode":"block","reason":prepared["reason"],"readback":prepared})); }
    shared.snapshots.get_mut(&args.chrome_tab_id).unwrap().consumed=true;
    let dispatched={let connection=shared.connection.as_ref().unwrap();connection.command(shared.registry()?,&shared.handle,&args.chrome_tab_id,"Input.insertText",json!({"text":text})).await};
    if let Err(error)=dispatched { return Ok(json!({"status":"unknown","action":"type","typing_mode":"block","dispatch_may_have_occurred":true,"reason":format!("text insertion delivery is unknown; inspect state and do not retry: {error}")})); }
    let final_value=format!("{expected}{text}");
    let readback={let connection=shared.connection.as_ref().unwrap();connection.command(shared.registry()?,&shared.handle,&args.chrome_tab_id,"Runtime.callFunctionOn",json!({"objectId":snapshot.object_id,"functionDeclaration":EDITOR_GUARD,"arguments":[{"value":index},{"value":final_value}],"returnByValue":true})).await};
    let after={let connection=shared.connection.as_ref().unwrap();super::super::shared_frame_identity(connection,shared.registry()?,&shared.handle,&args.chrome_tab_id).await};
    let readback_value=readback.as_ref().ok().and_then(|value|super::super::shared_value(value).ok());
    let guard_ok=readback_value.as_ref().is_some_and(|value|value["ok"]==true);
    let identity_same=matches!(after,Ok(ref current) if current==&identity);
    let verified=guard_ok&&identity_same;
    drop(shared);
    if verified { super::super::authorize_internal_workflow_snapshot(&snapshot.token); }
    Ok(if verified { json!({"status":"verified","action":"type","typing_mode":typing_mode,"dispatch_acknowledged":true,"readback":final_value}) } else { json!({"status":"unknown","action":"type","typing_mode":typing_mode,"dispatch_acknowledged":true,"reason":"inserted text did not pass editor readback and document identity; inspect before retrying","readback_guard":readback_value.as_ref().and_then(|value|value["reason"].as_str()),"identity_same":identity_same}) })
}

async fn ime_type(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
    let unrestricted = crate::guard_reason::unrestricted_mode();
    let reference = args
        .reference
        .as_deref()
        .ok_or_else(|| invalid("type requires reference"))?;
    let expected = args.expected_value.as_deref().unwrap_or_default();
    let text = args.value.as_deref().unwrap_or_default();
    if text.is_empty() || expected.len().saturating_add(text.len()) > 16_384 {
        return Err(invalid(
            "IME text must be nonempty and final value <= 16384 bytes",
        ));
    }
    let legacy_reference =
        resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
    let live = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = live.lock().await;
    if !unrestricted && shared.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let snapshot = shared
        .snapshots
        .get(&args.chrome_tab_id)
        .cloned()
        .ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token, index) = legacy_reference
        .rsplit_once(':')
        .ok_or_else(|| invalid("invalid retained reference"))?;
    let index = index
        .parse::<usize>()
        .map_err(|_| invalid("invalid retained reference index"))?;
    if index >= snapshot.count || (!unrestricted && (token != snapshot.token || snapshot.consumed && !super::super::internal_workflow_snapshot_allowed(&snapshot.token))) {
        return Err(invalid(
            "stale or consumed reference; take one fresh browser_snapshot",
        ));
    }
    let identity = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        super::super::shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
        .map_err(invalid)?
    };
    if !unrestricted && identity != snapshot.identity {
        return Err(invalid(
            "document changed since snapshot; take a fresh browser_snapshot",
        ));
    }
    let guard = EDITOR_GUARD;
    let initial = {
        let connection = shared.connection.as_ref().unwrap();
        connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
            "objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":expected},{"value":true}],"returnByValue":true
        })).await.map_err(|error| invalid(error.to_string()))?
    };
    let initial=super::super::shared_value(&initial).map_err(invalid)?;
    if initial!=json!(true)&&initial["ok"]!=true {
        return Ok(
            json!({"status":"not_dispatched","action":"type","typing_mode":"ime","reason":"field, focus, or caret changed before composition"}),
        );
    }
    if !unrestricted {
        shared.snapshots.get_mut(&args.chrome_tab_id).unwrap().consumed = true;
    }
    let connection = shared.connection.as_ref().unwrap();
    let final_value = format!("{expected}{text}");
    let composition_len = text.encode_utf16().count();
    if let Err(error) = connection
        .command(
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
            "Input.imeSetComposition",
            json!({
                "text":text,"selectionStart":composition_len,"selectionEnd":composition_len
            }),
        )
        .await
    {
        return Ok(
            json!({"status":"unknown","action":"type","typing_mode":"ime","reason":format!("IME composition delivery is unknown; inspect state and do not retry: {error}")}),
        );
    }
    let composed = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
        "objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":final_value}],"returnByValue":true
    })).await;
    if !matches!(composed, Ok(ref value) if super::super::shared_value(value).is_ok_and(|value| value==json!(true)||value["ok"]==true))
    {
        let _ = connection
            .command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Input.imeSetComposition",
                json!({"text":"","selectionStart":0,"selectionEnd":0}),
            )
            .await;
        return Ok(
            json!({"status":"unknown","action":"type","typing_mode":"ime","dispatch_acknowledged":true,
            "reason":"IME composition changed target or document state; composition cancellation was attempted; inspect before retrying"}),
        );
    }
    if let Err(error) = connection
        .command(
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
            "Input.insertText",
            json!({"text":text}),
        )
        .await
    {
        return Ok(
            json!({"status":"unknown","action":"type","typing_mode":"ime","dispatch_acknowledged":true,
            "reason":format!("IME commit delivery is unknown; inspect state and do not retry: {error}")}),
        );
    }
    let verification_started = std::time::Instant::now();
    let readback = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
        "objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":final_value}],"returnByValue":true
    })).await;
    let after = super::super::shared_frame_identity(
        connection,
        shared.registry()?,
        &shared.handle,
        &args.chrome_tab_id,
    )
    .await;
    let verified = matches!(readback, Ok(ref value) if super::super::shared_value(value).is_ok_and(|value| value==json!(true)||value["ok"]==true))
        && matches!(after, Ok(ref current) if current == &identity);
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Verification,
        verification_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    ).await;
    drop(shared);
    if verified { super::super::authorize_internal_workflow_snapshot(&snapshot.token); }
    if !verified {
        return Ok(
            json!({"status":"unknown","action":"type","typing_mode":"ime","dispatch_acknowledged":true,
            "reason":"IME commit did not pass retained-node and document readback; inspect before retrying"}),
        );
    }
    Ok(
        json!({"status":"verified","action":"type","typing_mode":"ime","dispatch_acknowledged":true,"readback":final_value}),
    )
}

fn cdp_character_events(character: char) -> Vec<(Value, bool)> {
    if character == '\n' {
        return vec![
            (json!({"type":"keyDown","key":"Enter","code":"Enter","windowsVirtualKeyCode":13,"nativeVirtualKeyCode":36,"text":"\r","unmodifiedText":"\r"}), true),
            (json!({"type":"keyUp","key":"Enter","code":"Enter","windowsVirtualKeyCode":13,"nativeVirtualKeyCode":36}), false),
        ];
    }
    let key = character.to_string();
    vec![
        (json!({"type":"keyDown","key":key}), false),
        (json!({"type":"char","key":key,"text":key,"unmodifiedText":key}), true),
        (json!({"type":"keyUp","key":key}), false),
    ]
}

fn normalize_key_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

async fn key_type(
    app: &AppV3,
    args: &V3ActArgs,
    policy: crate::v3::TypingPolicy,
) -> Result<Value, rmcp::ErrorData> {
    let unrestricted = crate::guard_reason::unrestricted_mode();
    let reference = args
        .reference
        .as_deref()
        .ok_or_else(|| invalid("type requires reference"))?;
    let expected = args.expected_value.as_deref().unwrap_or_default();
    let rich_editor = rich_editor_reference(app, args, reference).await;
    let text = normalize_key_text(args.value.as_deref().unwrap_or_default());
    if text.is_empty() || expected.len().saturating_add(text.len()) > 16_384 {
        return Err(invalid(
            "key typing requires nonempty text and final value <= 16384 bytes",
        ));
    }
    if rich_editor {
        if !text.is_ascii() { return Err(invalid("HumanKeys requires ASCII text; use IME for Unicode composition")); }
    } else if !text.is_ascii() {
        return Err(invalid("key typing requires ASCII text; use IME for Unicode input"));
    }
    let legacy_reference =
        resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
    let shared = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = shared.lock().await;
    if !unrestricted && shared.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let snapshot = shared
        .snapshots
        .get(&args.chrome_tab_id)
        .cloned()
        .ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token, index) = legacy_reference
        .rsplit_once(':')
        .ok_or_else(|| invalid("invalid retained reference"))?;
    let index = index
        .parse::<usize>()
        .map_err(|_| invalid("invalid retained reference index"))?;
    if index >= snapshot.count || (!unrestricted && (token != snapshot.token || snapshot.consumed && !super::super::internal_workflow_snapshot_allowed(&snapshot.token))) {
        return Err(invalid(
            "stale or consumed reference; take one fresh browser_snapshot",
        ));
    }
    let default_timeout = if policy.mode == crate::v3::TypingMode::HumanKeys {
        crate::v3::human_typing_timeout_ms(text.chars().count(), policy.delay_ms)
    } else {
        30_000
    };
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(default_timeout).clamp(1_000, 600_000));
    let deadline = tokio::time::Instant::now() + timeout;
    let before = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        tokio::time::timeout_at(
            deadline,
            super::super::shared_frame_identity(
                connection,
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
            ),
        )
        .await
        .map_err(|_| invalid("type preflight timed out; no key dispatched"))?
        .map_err(invalid)?
    };
    if !unrestricted && before != snapshot.identity {
        return Err(invalid(
            "document changed since snapshot; take a fresh browser_snapshot",
        ));
    }
    let guard_function = EDITOR_GUARD;
    let progress = expected.to_owned();
    let preflight = {
        let connection = shared.connection.as_ref().unwrap();
        let response = crate::v3_runtime::await_key_batch(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId":snapshot.object_id,"functionDeclaration":guard_function,
                    "arguments":[{"value":index},{"value":progress},{"value":true},{"value":true}],"returnByValue":true
                }),
            ),
        )
        .await
        .map_err(|_| invalid("type preflight timed out; no key dispatched"))?
        .map_err(|error| invalid(error.to_string()))?;
        super::super::shared_value(&response).map_err(invalid)?
    };
    if preflight["ok"] != true {
        return Ok(
            json!({"status":"not_dispatched","action":"type","reason":preflight["reason"],"readback":preflight}),
        );
    }
    let mut progress = preflight["value"].as_str().unwrap_or(&progress).to_owned();
    if !unrestricted {
        shared.snapshots.get_mut(&args.chrome_tab_id).ok_or_else(|| invalid("snapshot disappeared before dispatch"))?.consumed = true;
    }
    let mut dispatch_count = 0_u64;
    let bridge_fast_type = matches!(shared.connection.as_ref(), Some(super::super::PageSessionTransport::Shared(_)));
    if policy.mode == crate::v3::TypingMode::FastKeys && bridge_fast_type {
        let connection = shared.connection.as_ref().unwrap();
        let result = tokio::time::timeout_at(deadline, connection.command(
            shared.registry()?, &shared.handle, &args.chrome_tab_id, "Controlla.fastType",
            json!({"object_id":snapshot.object_id,"index":index,"expected":progress,"text":text,
                "normalize_paragraphs":rich_editor,
                "deadline_ms":deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis().clamp(1, 60_000) as u64}),
        )).await;
        let result = match result {
            Ok(Ok(value)) => shared_extension_value(&value).map_err(invalid)?,
            Ok(Err(error)) => {
                if let Some(reason_code) = controlla_browser::scheduler::telemetry::command_budget_stop_reason().await {
                    return Ok(json!({"status":"not_dispatched","action":"type","reason_code":reason_code,
                        "dispatch_acknowledged":false,"dispatch_may_have_occurred":false,
                        "reason":"the command budget refused the fast-key program before dispatch"}));
                }
                return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
                    "dispatch_may_have_occurred":true,"reason":format!("fast-key program delivery is uncertain; inspect before retrying: {error}")}));
            }
            Err(_) => return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
                "dispatch_may_have_occurred":true,"reason":"fast-key program exceeded its deadline; inspect before retrying"})),
        };
        dispatch_count = result["key_events"].as_u64().unwrap_or_default();
        progress = result["readback"].as_str().unwrap_or(&progress).to_owned();
        if result["status"] != "verified" {
            return Ok(json!({"status":result["status"],"action":"type","dispatch_acknowledged":dispatch_count > 0,
                "dispatch_may_have_occurred":dispatch_count > 0,"completed_chars":result["completed_chars"],
                "reason_code":result["reason"],"fast_type_receipt":result}));
        }
    } else {
        for character in text.chars() {
            let mut attempts = 0;
            'attempt: loop {
                let guard = {
                    let connection = shared.connection.as_ref().unwrap();
                    let response = tokio::time::timeout_at(deadline, connection.command(
                        shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn",
                        json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                            "arguments":[{"value":index},{"value":progress},{"value":true},{"value":true}],"returnByValue":true})
                    )).await;
                    match response {
                        Ok(Ok(response)) => super::super::shared_value(&response).ok(),
                        _ => None,
                    }
                };
                if guard.as_ref().is_none_or(|value| value["ok"] != true) {
                    return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                        "reason":"focus, value, selection, target, or document changed during typing; inspect before retrying","guard_failure":guard}));
                }
                if guard.as_ref().is_some_and(|value| value["rebound"] == true) {
                    let current = super::super::shared_frame_identity(
                        shared.connection.as_ref().unwrap(), shared.registry()?, &shared.handle, &args.chrome_tab_id,
                    ).await;
                    if !matches!(current, Ok(ref identity) if identity == &before) {
                        return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                            "reason":"editor was replaced after the document changed; no more keys dispatched"}));
                    }
                }
                let (events, inserted_on_keydown) = if character == '\n' {
                    let (down, _) = cdp_character_events(character).remove(0);
                    (vec![down], true)
                } else {
                    let events = cdp_character_events(character);
                    (vec![events[0].0.clone(), events[1].0.clone()], false)
                };
                for (event_index, event) in events.into_iter().enumerate() {
                    let connection = shared.connection.as_ref().unwrap();
                    let sent = tokio::time::timeout_at(deadline, connection.command(
                        shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.dispatchKeyEvent", event,
                    )).await;
                    if !matches!(sent, Ok(Ok(_))) {
                        return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                            "reason":"key dispatch became uncertain; inspect state and do not automatically retry"}));
                    }
                    dispatch_count = dispatch_count.saturating_add(1);
                    let mut candidate = progress.clone();
                    if inserted_on_keydown || event_index > 0 { candidate.push(character); }
                    // Check without preparing: preparation would refocus a control after keydown
                    // and conceal focus theft before the remaining trusted key events are sent.
                    let after_event = {
                        let connection = shared.connection.as_ref().unwrap();
                        let response = tokio::time::timeout_at(deadline, connection.command(
                            shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn",
                            json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                                "arguments":[{"value":index},{"value":candidate},{"value":false},{"value":rich_editor}],"returnByValue":true})
                        )).await;
                        match response {
                            Ok(Ok(response)) => super::super::shared_value(&response).ok(),
                            _ => None,
                        }
                    };
                    if after_event.as_ref().is_none_or(|value| value["ok"] != true) {
                        let unchanged = {
                            let connection = shared.connection.as_ref().unwrap();
                            let response = tokio::time::timeout_at(deadline, connection.command(
                                shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn",
                                json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                                    "arguments":[{"value":index},{"value":progress},{"value":false},{"value":rich_editor}],"returnByValue":true})
                            )).await;
                            match response {
                                Ok(Ok(response)) => super::super::shared_value(&response).ok(),
                                _ => None,
                            }
                        };
                        if attempts == 0 && unchanged.as_ref().is_some_and(|value| value["ok"] == true) {
                            let key_up = cdp_character_events(character).pop().unwrap().0;
                            let connection = shared.connection.as_ref().unwrap();
                            if matches!(tokio::time::timeout_at(deadline, connection.command(
                                shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.dispatchKeyEvent", key_up,
                            )).await, Ok(Ok(_))) {
                                dispatch_count = dispatch_count.saturating_add(1);
                                attempts += 1;
                                continue 'attempt;
                            }
                        }
                        return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
                            "reason":"focus, value, selection, target, or document changed during key dispatch; inspect before retrying",
                            "guard_failure":after_event,"unchanged_readback":unchanged}));
                    }
                    if inserted_on_keydown || event_index > 0 { progress = candidate; }
                }
                let key_up = cdp_character_events(character).pop().unwrap().0;
                let connection = shared.connection.as_ref().unwrap();
                let sent = tokio::time::timeout_at(deadline, connection.command(
                    shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.dispatchKeyEvent", key_up,
                )).await;
                if !matches!(sent, Ok(Ok(_))) {
                    return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
                        "reason":"key-up delivery became uncertain; inspect state and do not automatically retry"}));
                }
                dispatch_count = dispatch_count.saturating_add(1);
                break;
            }
            if policy.delay_ms > 0
                && tokio::time::timeout_at(
                    deadline,
                    tokio::time::sleep(Duration::from_millis(policy.delay_ms)),
                )
                .await
                .is_err()
            {
                return Ok(
                    json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
                "reason":"typing deadline expired after partial dispatch; inspect before retrying"}),
                );
            }
        }
    }
    let verification_started = std::time::Instant::now();
    let readback = {
        let connection = shared.connection.as_ref().unwrap();
        let response = tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                "arguments":[{"value":index},{"value":progress},{"value":false},{"value":true},{"value":false}],"returnByValue":true}),
            ),
        )
        .await;
        match response {
            Ok(Ok(response)) => super::super::shared_value(&response).ok(),
            _ => None,
        }
    };
    let after = {
        let connection = shared.connection.as_ref().unwrap();
        tokio::time::timeout_at(
            deadline,
            super::super::shared_frame_identity(
                connection,
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
            ),
        )
        .await
    };
    let verified = readback.as_ref().is_some_and(|value| value["ok"] == true)
        && matches!(after, Ok(Ok(ref identity)) if identity == &before);
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Verification,
        verification_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    ).await;
    drop(shared);
    if !verified {
        return Ok(
            json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
            "reason":"post-typing readback or document identity did not verify final state; inspect before retrying"}),
        );
    }
    super::super::authorize_internal_workflow_snapshot(&snapshot.token);
    let result = json!({"status":"verified","action":"type","dispatch_acknowledged":true,
        "typing_mode":match policy.mode { crate::v3::TypingMode::FastKeys=>"fast_keys", _=>"human_keys" },
        "readback":progress.clone(),"cdp_key_events":dispatch_count,"intentional_delay_ms":policy.delay_ms,
        "intentional_delay_total_ms":(text.chars().count() as u64).saturating_mul(policy.delay_ms),
        "delta":{"reference":reference,"before":expected,"after":progress,"document_changed":false,"verified":true}});
    Ok(result)
}

async fn press(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
    let reference = args
        .reference
        .as_deref()
        .ok_or_else(|| invalid("press requires reference"))?;
    let key = args
        .key
        .as_deref()
        .filter(|key| !key.is_empty() && key.len() <= 64 && !key.chars().any(char::is_control))
        .ok_or_else(|| invalid("press requires a bounded non-control key"))?;
    let legacy_reference =
        resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
    let shared = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = shared.lock().await;
    let snapshot = shared
        .snapshots
        .get(&args.chrome_tab_id)
        .cloned()
        .ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token, index) = legacy_reference
        .rsplit_once(':')
        .ok_or_else(|| invalid("invalid retained reference"))?;
    let index = index
        .parse::<usize>()
        .map_err(|_| invalid("invalid retained reference index"))?;
    let unrestricted = crate::guard_reason::unrestricted_mode();
    if index >= snapshot.count || (!unrestricted && (token != snapshot.token || snapshot.consumed)) {
        return Err(invalid("stale or consumed reference"));
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let preflight = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
        "objectId":snapshot.object_id,"functionDeclaration":if unrestricted {"function(index){const e=this.nodes[index];if(e)e.focus();return {ok:!!e};}"} else {"function(index){const e=this.nodes[index];return {ok:!!e&&e.isConnected&&document.activeElement===e};}"},
        "arguments":[{"value":index}],"returnByValue":true
    })).await.map_err(|error| invalid(error.to_string()))?;
    if super::super::shared_value(&preflight).map_err(invalid)?["ok"] != true {
        return Ok(
            json!({"status":"not_dispatched","action":"press","reason":"referenced element is not connected and focused"}),
        );
    }
    if !unrestricted {
        shared.snapshots.get_mut(&args.chrome_tab_id).unwrap().consumed = true;
    }
    for (index, event) in [
        json!({"type":"keyDown","key":key}),
        json!({"type":"keyUp","key":key}),
    ]
    .into_iter()
    .enumerate()
    {
        let connection = shared.connection.as_ref().unwrap();
        if let Err(error) = connection
            .command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Input.dispatchKeyEvent",
                event,
            )
            .await
        {
            return Ok(
                json!({"status":"unknown","action":"press","dispatch_acknowledged":index > 0,
                "reason":format!("press delivery is uncertain; inspect before retrying: {error}")}),
            );
        }
    }
    Ok(
        json!({"status":"dispatched","action":"press","dispatch_acknowledged":true,"requires_verify":true}),
    )
}

fn page_tool_completion(response: Result<Value, String>) -> Result<Value, Value> {
    response.map_err(|error| {
        json!({
            "status":"unknown","action":"page_tool","dispatch_acknowledged":true,
            "reason":format!("page-tool completion is uncertain; inspect before retrying: {error}")
        })
    })
}

fn page_tool_snapshot_fresh(consumed: bool, identity_matches: bool) -> bool {
    !consumed && identity_matches
}

fn page_tool_unavailable(reason: &str) -> Value {
    json!({"status":"not_dispatched","action":"page_tool","fallback":"semantic_ui","reason":reason})
}

async fn call_page_tool(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
    let name = args
        .page_tool_name
        .as_deref()
        .filter(|name| !name.is_empty() && name.len() <= 128);
    if args.action == "page_tool" && name.is_none() {
        return Err(invalid("page_tool requires a bounded page_tool_name"));
    }
    let input = args.page_tool_input.clone().unwrap_or_else(|| json!({}));
    let timeout_ms = args.timeout_ms.unwrap_or(10_000).clamp(1, 30_000);
    if !input.is_object() || serde_json::to_vec(&input).map_or(true, |value| value.len() > 16_384) {
        return Err(invalid(
            "page_tool_input must be an object within 16384 bytes",
        ));
    }
    let live = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = live.lock().await;
    if shared.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let snapshot = shared
        .snapshots
        .get(&args.chrome_tab_id)
        .cloned()
        .ok_or_else(|| invalid("take browser_snapshot before discovering page tools"))?;
    let (before, discovery) = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let before = super::super::shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
        .map_err(invalid)?;
        if !page_tool_snapshot_fresh(snapshot.consumed, before == snapshot.identity) {
            return Err(invalid(if snapshot.consumed {
                "snapshot was already consumed; take a fresh browser_snapshot"
            } else {
                "document changed since snapshot; take a fresh browser_snapshot"
            }));
        }
        let discovery = tokio::time::timeout(Duration::from_millis(timeout_ms), connection.command(
            shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.evaluate",
            json!({"expression":format!("({})()", crate::v3_runtime::PageToolProgram::discovery()),"awaitPromise":true,"returnByValue":true}),
        )).await.map_err(|_| invalid("page-tool discovery timed out"))?
            .map_err(|error| invalid(error.to_string()))?;
        (before, discovery)
    };
    let discovered = super::super::shared_value(&discovery).map_err(invalid)?;
    if discovered["available"] != true {
        return Ok(page_tool_unavailable("page does not expose WebMCP tools"));
    }
    if !crate::v3_tasks::validate_page_tool_result(
        &discovered,
        crate::v3_runtime::MAX_PAGE_TOOL_OUTPUT_BYTES,
    ) {
        return Ok(page_tool_unavailable(
            "page-tool discovery exceeded its output budget",
        ));
    }
    let tools = discovered["tools"]
        .as_array()
        .ok_or_else(|| invalid("page-tool discovery returned invalid tools"))?;
    if args.action == "page_tools" {
        let listed: Vec<_> = tools.iter().take(64).filter_map(|tool| {
            Some(json!({
                "name":tool.get("name")?.as_str()?.chars().take(128).collect::<String>(),
                "description":tool.get("description")?.as_str()?.chars().take(512).collect::<String>(),
                "input_schema":tool.get("input_schema")?.clone()
            }))
        }).collect();
        return Ok(
            json!({"status":"discovered","available":!listed.is_empty(),"tools":listed,"page_content_is_untrusted":true,
            "fallback":if listed.is_empty() { Some("semantic_ui") } else { None }}),
        );
    }
    let name = name.expect("page_tool name validated before discovery");
    let descriptor = tools
        .iter()
        .filter_map(|tool| {
            Some(crate::v3_tasks::PageToolDescriptor {
                name: tool.get("name")?.as_str()?.to_owned(),
                effect: crate::v3_tasks::PageToolEffect::Consequential,
                input_schema: tool.get("input_schema")?.clone(),
            })
        })
        .find(|tool| tool.name == name);
    let Some(descriptor) = descriptor else {
        return Ok(page_tool_unavailable(
            "requested page tool is not available",
        ));
    };
    if !matches!(
        crate::v3_tasks::route_page_tool(std::slice::from_ref(&descriptor), name, true),
        crate::v3_tasks::PageToolRoute::NativeTool(_)
    ) {
        return Ok(page_tool_unavailable(
            "page tool is not authorized for this action",
        ));
    }
    if !crate::v3_tasks::validate_page_tool_input(&descriptor.input_schema, &input) {
        return Ok(page_tool_unavailable(
            "page-tool input does not match the supported schema",
        ));
    }
    let call = crate::v3_runtime::PageToolCall {
        name: name.to_owned(),
        authority: crate::v3_runtime::PageToolAuthority::Consequential,
        input: input.clone(),
        timeout_ms,
        max_output_bytes: crate::v3_runtime::MAX_PAGE_TOOL_OUTPUT_BYTES,
    };
    call.validate(&std::collections::BTreeSet::from([descriptor.name.clone()]))
        .map_err(invalid)?;
    let object_id = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let global = connection
            .command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.evaluate",
                json!({"expression":"globalThis","returnByValue":false}),
            )
            .await
            .map_err(|error| invalid(error.to_string()))?;
        global
            .pointer("/result/objectId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| invalid("page context did not expose a callable global object"))?
    };
    let dispatch_identity = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        super::super::shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
        .map_err(invalid)?
    };
    if !page_tool_snapshot_fresh(snapshot.consumed, dispatch_identity == before) {
        return Ok(page_tool_unavailable(
            "document changed after tool discovery; take a fresh snapshot",
        ));
    }
    shared
        .snapshots
        .get_mut(&args.chrome_tab_id)
        .unwrap()
        .consumed = true;
    let execution = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        tokio::time::timeout(Duration::from_millis(timeout_ms), connection.command(
            shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
                "objectId":object_id,"functionDeclaration":crate::v3_runtime::PageToolProgram::invocation(),
                "arguments":[{"value":name},{"value":input},{"value":descriptor.input_schema},{"value":timeout_ms}],"awaitPromise":true,"returnByValue":true
            }),
        )).await
    };
    let result = match execution {
        Err(_) => page_tool_completion(Err("page tool exceeded its bound".into())),
        Ok(Err(error)) => page_tool_completion(Err(error.to_string())),
        Ok(Ok(result)) => page_tool_completion(super::super::shared_value(&result)),
    };
    let result = match result {
        Ok(result) => result,
        Err(unknown) => return Ok(unknown),
    };
    if result["available"] != true
        || result["not_found"] == true
        || result["schema_changed"] == true
    {
        return Ok(
            json!({"status":"not_dispatched","action":"page_tool","reason":"page tool disappeared or its schema changed after discovery; use semantic UI"}),
        );
    }
    if !crate::v3_tasks::validate_page_tool_result(&result["result"], call.max_output_bytes) {
        return Ok(
            json!({"status":"unknown","action":"page_tool","dispatch_acknowledged":true,"reason":"page tool returned an oversized result; inspect page state"}),
        );
    }
    let after = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        super::super::shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
    };
    if !matches!(after, Ok(ref identity) if identity == &before) {
        return Ok(
            json!({"status":"unknown","action":"page_tool","dispatch_acknowledged":true,"reason":"document identity changed during page-tool execution; inspect before retrying"}),
        );
    }
    Ok(
        json!({"status":"dispatched","action":"page_tool","tool":name,"result":result["result"],"page_content_is_untrusted":true,"requires_verify":true}),
    )
}

#[tool_router]
impl AppV3 {
    #[tool(
        description = "Manage browser sessions and bounded tasks. Each browser call, including cold open plus optional workflow steps, shares one 30-second budget of 256 browser commands and 20,000 CDP commands; usage and remaining budget appear in metrics. For action=task, provide a tagged task; valid Research example: {\"kind\":\"research\",\"urls\":[\"https://example.test/\"]}. Research navigates the selected existing tab. Foreground action=open requires a stable idempotency_key: reuse it only to recover the same open; use a new key for a distinct tab. It cold-starts a shared session and opens a tab; optional steps run in that same bounded call. Open attaches at document commit for DocumentCommitted, URL, selector, and semantic ready_when conditions; otherwise it waits for DOMContentLoaded by default. URL-prefix, CSS-selector, and exact semantic-match conditions gate steps for up to 10 seconds. Password controls appear only as redacted credential_input references. To fill one, use act action=fill with its fresh reference and allow_password=true; the supplied value is transmitted to the local MCP/browser but never returned in snapshots or action results. Do not use workflow fill for credentials. The visible-trigger workflow Upload and foreground artifact_register file_path are available only in local_unrestricted shared sessions; guarded uploads use the dedicated headless task upload route. Upload requires an artifact_handle registered in that same session, an observed trigger reference, an account marker, and (for continuation) an acceptance predicate. Use [\"ax:button\",\"exact visible accessible name\"] when a button lacks a unique CSS selector; this is checked against the root DOM's computed name, excluding picker/menu scopes. Files are limited to 10 MiB. If a trigger opens a child-frame picker, list_shared_targets, snapshot its frame, then pass the observed Browse @f reference.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<std::collections::BTreeMap<String, Value>>()
    )]
    async fn browser(
        &self,
        Parameters(args): Parameters<V3BrowserArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let budget = crate::v3::CommandBudget::interactive_default();
        let wall_limit = Duration::from_millis(budget.max_wall_ms);
        measured_receipt(controlla_browser::scheduler::telemetry::with_browser_command_budget(
            budget.max_browser_commands as u64,
            budget.max_cdp_commands as u64,
            wall_limit,
            async move {
                match tokio::time::timeout(wall_limit, Box::pin(self.browser_inner(args))).await {
                    Ok(result) => result,
                    Err(_) => Ok(Json(json!({"status":"unknown","reason_code":"browser_wall_time_exceeded",
                        "dispatch_may_have_occurred":true,"next_action":"inspect the selected tab; retry the same open only with its original idempotency_key"}))),
                }
            },
        )).await
    }

    async fn browser_inner(&self, args: V3BrowserArgs) -> Result<Json<Value>, rmcp::ErrorData> {
        if args.steps.is_some() && args.action != "open" {
            return Err(invalid("steps are supported only by browser action=open"));
        }
        if args.ready_when.is_some() && args.action != "open" {
            return Err(invalid("ready_when is supported only by browser action=open"));
        }
        if args.action == "task" {
            return self
                .task(Parameters(V3TaskArgs {
                    session_id: args
                        .session_id
                        .ok_or_else(|| invalid("session_id is required"))?,
                    chrome_tab_id: args
                        .chrome_tab_id
                        .ok_or_else(|| invalid("chrome_tab_id is required"))?,
                    task: args.task.ok_or_else(|| invalid("task is required"))?,
                    client_id: args.client_id,
                }))
                .await;
        }
        if args.action == "artifact_register" {
            return self
                .artifact_register(Parameters(V3ArtifactRegisterArgs {
                    session_id: args
                        .session_id
                        .ok_or_else(|| invalid("session_id is required"))?,
                    filename: args.filename.ok_or_else(|| invalid("filename is required"))?,
                    bytes: args.bytes,
                    bytes_base64: args.bytes_base64,
                    file_path: args.file_path,
                }))
                .await;
        }
        if args.action == "artifact_read" {
            return self
                .artifact_read(Parameters(V3ArtifactReadArgs {
                    session_id: args
                        .session_id
                        .ok_or_else(|| invalid("session_id is required"))?,
                    artifact_handle: args
                        .artifact_handle
                        .ok_or_else(|| invalid("artifact_handle is required"))?,
                    encoding: args.encoding,
                }))
                .await;
        }
        if args.action == "describe" {
            let tools = AppV3::tool_router().list_all();
            let mut surface = tools.iter().map(|tool| serde_json::to_value(tool).unwrap_or(Value::Null)).collect::<Vec<_>>();
            surface.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            let schema_sha256 = {
                use sha2::{Digest, Sha256};
                Sha256::digest(serde_json::to_vec(&surface).unwrap_or_default()).iter().map(|byte| format!("{byte:02x}")).collect::<String>()
            };
            return Ok(Json(json!({
                "version":"v3","tools":["browser","snapshot","act","workflow","extract","verify"],
                "surface":"v3-compact","server_version":env!("CARGO_PKG_VERSION"),"schema_sha256":schema_sha256,
                "modes":{"foreground":crate::v3::BrowserMode::Foreground.policy(),"background":crate::v3::BrowserMode::Background.policy(),"headless":crate::v3::BrowserMode::Headless.policy()},
                "browser_actions":{"foreground":["discover_shared_tabs","pair_shared","accept_shared","list","open","release","task"],"background":["discover_shared_tabs","pair_shared","accept_shared","list","release","task"],"headless":["launch","list","release","task","artifact_register","artifact_read"]},
                "foreground_open":{"session_id":"optional; omitting it pairs a shared Chrome tab before opening a new foreground tab","idempotency_key":"required; reuse only to retry the same logical open, and use a new key for a distinct tab","steps":"optional bounded workflow executed in the new tab; result is returned in workflow",
                "ready_when":"DocumentCommitted, DomContentLoaded, URL-prefix, CSS-selector, or semantic-match. Non-DOMContentLoaded conditions attach at document commit before gating workflow steps; timeout returns incomplete and skips workflow steps","guarded_cold_open":"requires host_id and target_ids from discovery","local_unrestricted_cold_open":"chooses a stable web tab from the native inventory"},
                "act_actions":["click","fill","type","press","select","page_tools","page_tool"],
                "artifact_scopes":{"guarded":["private_headless_session"],"local_unrestricted":["private_headless_session","local_unrestricted_shared_session"]},
                "client_loaded_tool_state":"unverified_until_client_acknowledges_surface_fingerprint",
                "typing_modes":["block","fast_keys","human_keys","ime"],
                "interaction_policy":if crate::guard_reason::unrestricted_mode() {"local_unrestricted"} else {"guarded"},
                "safety":if crate::guard_reason::unrestricted_mode() {"native pairing discovery checks, click/fill/select/press target checks, and target leases are disabled; explicit click outcomes are still observed before verified is returned; transport and request validity checks remain"} else {"revision-bound refs, target leases, fresh-state verification, no blind mutation retry"}
            })));
        }
        let mode = parse_mode(args.mode.as_deref())?;
        let provider = args
            .provider.clone()
            .or_else(|| Some(mode.default_provider().to_owned()));
        if provider.as_deref() != Some(mode.default_provider()) {
            return Err(invalid(format!(
                "mode {mode:?} requires provider {}",
                mode.default_provider()
            )));
        }
        if args.action == "open" {
            let open_started = std::time::Instant::now();
            if mode != crate::v3::BrowserMode::Foreground {
                return Err(invalid("browser action=open requires foreground mode"));
            }
            let url = args.url.as_deref().ok_or_else(|| invalid("url is required"))?;
            super::super::validate_shared_url(url)?;
            if args.ready_when.as_ref().is_some_and(|condition| match condition {
                V3ReadyWhen::UrlPrefix { prefix } => prefix.is_empty() || prefix.len() > 2048,
                V3ReadyWhen::Selector { selector } => selector.trim().is_empty() || selector.len() > 1024,
                V3ReadyWhen::SemanticMatch { query, .. } => query.trim().is_empty() || query.len() > 512,
                _ => false,
            }) {
                return Err(invalid("ready_when requires a bounded nonempty URL prefix, CSS selector, or semantic query"));
            }
            if let Some(steps) = args.steps.as_ref() {
                if steps.is_empty() || steps.len() > 40 {
                    return Err(invalid("workflow requires 1..40 steps"));
                }
                validate_workflow_steps(steps)?;
            }
            if args.idempotency_key.as_deref().is_none_or(str::is_empty) {
                return Err(invalid("browser action=open requires idempotency_key so retries reuse the same tab"));
            }
            let cold = args.session_id.is_none();
            let session_id = if let Some(session_id) = args.session_id.as_deref() {
                if self.browser_modes.lock().await.get(session_id) != Some(&mode) {
                    return Err(invalid("session was not created in foreground mode"));
                }
                session_id.to_owned()
            } else {
                self.cold_open_session(&args).await?
            };
            let opened = self.core.legacy.shared_tab(Parameters(super::SharedTabArgs {
                session_id: session_id.clone(),
                action: "open".into(),
                chrome_tab_id: None,
                url: url.to_owned(),
                idempotency_key: args.idempotency_key.as_deref().map(|key| format!("{}:{key}", args.client_id.as_deref().unwrap_or("local-stdio"))),
                wait_until: Some(if matches!(args.ready_when.as_ref(), Some(V3ReadyWhen::DomContentLoaded)) || args.ready_when.is_none() {
                    "dom_content_loaded".into()
                } else {
                    "document_committed".into()
                }),
            })).await;
            let Json(mut result) = match opened {
                Ok(result) => result,
                Err(error) => {
                    if cold { self.release_cold_session(&session_id).await; }
                    return Err(error);
                }
            };
            result["session_id"] = json!(session_id);
            result["mode"] = json!(mode);
            if let Some(ready_when) = args.ready_when.as_ref() {
                let ready_started = std::time::Instant::now();
                let ready = self.wait_for_open_readiness(
                    result["session_id"].as_str().unwrap_or_default(),
                    result["tab_id"].as_str().unwrap_or_default(),
                    ready_when,
                ).await?;
                controlla_browser::scheduler::telemetry::record_phase(
                    controlla_browser::scheduler::telemetry::OperationPhase::Readiness,
                    ready_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                ).await;
                result["readiness"] = json!({"condition":ready_when.label(),"ready":ready,"timeout_ms":10_000});
                if !ready {
                    result["status"] = json!("incomplete");
                    result["reason_code"] = json!("application_readiness_timeout");
                    result["next_action"] = json!("inspect the opened tab; workflow steps were not dispatched");
                    result["elapsed_ms"] = json!(open_started.elapsed().as_millis() as u64);
                    return Ok(Json(result));
                }
            }
            if let Some(steps) = args.steps {
                let tab_id = result["tab_id"].as_str().ok_or_else(|| invalid("open omitted tab_id"))?.to_owned();
                result["workflow"] = match self.workflow(Parameters(V3WorkflowArgs {
                        session_id,
                        chrome_tab_id: tab_id,
                        steps,
                        program: None,
                        client_id: args.client_id,
                    })).await {
                    Ok(Json(value)) => value,
                    Err(error) => json!({"status":"failed","stage":"workflow","reason":error.to_string()}),
                };
            }
            result["elapsed_ms"] = json!(open_started.elapsed().as_millis() as u64);
            return Ok(Json(result));
        }
        if mode == crate::v3::BrowserMode::Headless {
            return match args.action.as_str() {
                "launch" => {
                    let executable = self.headless_chrome.clone()
                        .ok_or_else(|| invalid("headless mode requires COMPTROL_CHROME_EXECUTABLE to name the Chrome executable"))?;
                    let url = args.url.as_deref().unwrap_or("about:blank");
                    if url.is_empty() || url.len() > 2048 || url.chars().any(char::is_control) {
                        return Err(invalid(
                            "url must contain 1..2048 characters without controls",
                        ));
                    }
                    let mut registry = controlla_browser::sessions::SessionRegistry::new(
                        controlla_browser::sessions::ProviderGrants {
                            dedicated_headless: true,
                            ..Default::default()
                        },
                    );
                    let handle = registry
                        .create_session(
                            controlla_browser::sessions::SessionSpec {
                                mode: controlla_browser::sessions::SessionMode::Headless,
                                selected_target_ids: Vec::new(),
                            },
                            self.core.legacy.principal.to_string(),
                        )
                        .map_err(|error| invalid(format!("headless session denied: {error:?}")))?;
                    let browser =
                        controlla_browser::providers::DedicatedChromeProvider::new(executable)
                            .launch(&mut registry, &handle, url)
                            .await
                            .map_err(|error| invalid(error.to_string()))?;
                    let session_id = handle.id.clone();
                    let target_id = browser.target_id().to_owned();
                    self.core.legacy.shared_sessions.lock().await.insert(
                        session_id.clone(),
                        Arc::new(Mutex::new(super::SharedLiveSession {
                            snapshots: std::collections::BTreeMap::new(),
                            pairing: None,
                            connection: Some(super::PageSessionTransport::Dedicated(Some(browser))),
                            registry: Some(registry),
                            handle,
                            native_pairing: None,
                        })),
                    );
                    self.browser_modes
                        .lock()
                        .await
                        .insert(session_id.clone(), mode);
                    Ok(Json(
                        json!({"session_id":session_id,"mode":"headless","provider":"dedicated_headless","target_ids":[target_id],"launched":true,"verified":false,"next":"Call browser action=list, then snapshot before acting."}),
                    ))
                }
                "list" | "release" => {
                    let session_id = args
                        .session_id
                        .as_deref()
                        .ok_or_else(|| invalid("session_id is required"))?;
                    let session = self
                        .core
                        .legacy
                        .shared_sessions
                        .lock()
                        .await
                        .get(session_id)
                        .cloned()
                        .ok_or_else(|| invalid("unknown headless session_id"))?;
                    let session = session.lock().await;
                    if session.handle.principal != self.core.legacy.principal.as_ref()
                        || session.handle.mode != controlla_browser::sessions::SessionMode::Headless
                        || session.handle.provider
                            != controlla_browser::sessions::ProviderKind::DedicatedHeadless
                    {
                        return Err(invalid(
                            "session does not belong to this server's dedicated headless provider",
                        ));
                    }
                    drop(session);
                    let result = self
                        .core
                        .browser_session(Parameters(BrowserSessionArgs {
                            action: if args.action == "list" {
                                "list_shared_targets"
                            } else {
                                "release_shared"
                            }
                            .into(),
                            provider,
                            session_id: Some(session_id.to_owned()),
                            target_ids: None,
                            host_id: None,
                        }))
                        .await?;
                    if args.action == "release" {
                        self.browser_modes.lock().await.remove(session_id);
                    }
                    let Json(mut value) = result;
                    value["mode"] = json!(mode);
                    value["provider"] = json!(mode.default_provider());
                    Ok(Json(value))
                }
                _ => {
                    return Err(invalid(
                        "headless browser action must be launch, list, release, or describe",
                    ));
                }
            };
        }
        let action = match args.action.as_str() {
            "list" => "list_shared_targets",
            "release" => "release_shared",
            action => action,
        };
        if matches!(
            action,
            "accept_shared" | "list_shared_targets" | "release_shared"
        ) {
            let session_id = args
                .session_id
                .as_deref()
                .ok_or_else(|| invalid("session_id is required"))?;
            if self.browser_modes.lock().await.get(session_id) != Some(&mode) {
                return Err(invalid("session was not created in this V3 browser mode"));
            }
        } else if !matches!(action, "pair_shared" | "discover_shared_tabs") {
            return Err(invalid(
                "foreground/background browser action must be pair_shared, accept_shared, list, release, discover_shared_tabs, or foreground open",
            ));
        }
        let result = self
            .core
            .browser_session(Parameters(BrowserSessionArgs {
                action: action.to_owned(),
                provider,
                session_id: args.session_id.clone(),
                target_ids: args.target_ids,
                host_id: args.host_id,
            }))
            .await?;
        if action == "pair_shared" {
            let Json(mut value) = result;
            let session_id = value["session_id"]
                .as_str()
                .ok_or_else(|| invalid("pair_shared response omitted session_id"))?
                .to_owned();
            self.browser_modes.lock().await.insert(session_id, mode);
            value["mode"] = json!(mode);
            value["provider"] = json!(mode.default_provider());
            Ok(Json(value))
        } else if action == "release_shared" {
            if let Some(session_id) = args.session_id {
                self.browser_modes.lock().await.remove(&session_id);
            }
            let Json(mut value) = result;
            value["mode"] = json!(mode);
            Ok(Json(value))
        } else {
            let Json(mut value) = result;
            value["mode"] = json!(mode);
            value["provider"] = json!(mode.default_provider());
            Ok(Json(value))
        }
    }

    #[tool(
        description = "Return a compact semantic snapshot/delta with revision-bound @cN refs. Visible password fields are included as credential_input references with raw_value=null and requires_user_input=true; their value is never returned. Set frame_id for bounded child-frame accessibility, which returns loader-bound @f refs usable by click. Use before mutations and again after user/page changes.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<std::collections::BTreeMap<String, Value>>()
    )]
    async fn snapshot(
        &self,
        Parameters(args): Parameters<BrowserSnapshotArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        measured_receipt(async {
            self.require_v3_session(&args.session_id).await?;
            snapshot_impl(&self.core, &args).await.map(Json)
        }).await
    }

    #[tool(
        description = "Perform one bounded action, discover selected page tools with action=page_tools, or explicitly invoke one with action=page_tool. Each standalone act call shares a 30-second budget of 256 browser commands and 20,000 CDP commands; usage and remaining budget appear in metrics. FastKeys reserves its full bounded CDP work before dispatch and returns not_dispatched when the budget cannot cover it. Password inputs remain redacted and can be filled only with action=fill plus allow_password=true and a fresh reference that resolves to an enabled visible password input. The value is sent to the local MCP/browser and never returned in snapshots or action results. Omit allow_password for normal fills; action=type and workflows do not accept password values. type supports block, fast_keys (zero-delay trusted CDP keys; rich-editor newline runs become Enter), human_keys (trusted keys with a default 180 ms per character, about 55 WPM; override with delay_ms), and ime. Human typing gets a text-length-based deadline capped by the 30-second call budget. Guarded mode leases targets and checks drift; local_unrestricted skips target checks. Click still waits for its explicit postcondition and reports unknown until observed.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<std::collections::BTreeMap<String, Value>>()
    )]
    async fn act(
        &self,
        Parameters(args): Parameters<V3ActArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let budget = crate::v3::CommandBudget::interactive_default();
        let wall_limit = Duration::from_millis(budget.max_wall_ms);
        measured_receipt(controlla_browser::scheduler::telemetry::with_browser_command_budget(
            budget.max_browser_commands as u64,
            budget.max_cdp_commands as u64,
            wall_limit,
            async move {
                match tokio::time::timeout(wall_limit, Box::pin(self.act_inner(args))).await {
                    Ok(result) => result,
                    Err(_) => Ok(Json(json!({"status":"unknown","reason_code":"act_wall_time_exceeded",
                        "dispatch_may_have_occurred":true,"next_action":"inspect current page state before retrying"}))),
                }
            },
        )).await
    }

    async fn act_inner(&self, mut args: V3ActArgs) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        if args.allow_password == Some(true) && args.action != "fill" {
            return Err(invalid("allow_password is supported only for action=fill"));
        }
        let background_mutation = self.browser_modes.lock().await.get(&args.session_id) == Some(&crate::v3::BrowserMode::Background)
            && matches!(args.action.as_str(), "click" | "fill" | "type" | "press" | "select" | "page_tool");
        let presentation_before = if background_mutation { presentation_state(self, &args.session_id, &args.chrome_tab_id).await } else { None };
        let lease = acquire_lease(self, &args).await?;
        let dispatch_started = std::time::Instant::now();
        let operation_timeout = if args.action == "type" && args.typing_mode.as_deref() == Some("human_keys") {
            let delay_ms = args.delay_ms.unwrap_or(180);
            let default = crate::v3::human_typing_timeout_ms(args.value.as_deref().unwrap_or_default().chars().count(), delay_ms);
            Duration::from_millis(args.timeout_ms.unwrap_or(default).clamp(1_000, 600_000))
        } else {
            Duration::from_secs(60)
        };
        let result = match tokio::time::timeout(operation_timeout, async {
            match args.action.as_str() {
                "click" => self
                    .core
                    .browser_act(Parameters(BrowserActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "click".into(),
                        reference: args.reference.clone(),
                        expected_value: None,
                        value: None,
                        outcome: args.outcome.take().or_else(|| crate::guard_reason::unrestricted_mode().then_some(BrowserClickOutcome::Focused)),
                        timeout_ms: args.timeout_ms,
                    }))
                    .await
                    .map(|Json(v)| v),
                "fill" | "select" => {
                    let reference = args
                        .reference
                        .as_deref()
                        .ok_or_else(|| invalid(format!("{} requires reference", args.action)))?;
                    retained_mutation(
                        &self.core,
                        &BrowserActArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            action: args.action.clone(),
                            reference: args.reference.clone(),
                            expected_value: args.expected_value.clone(),
                            value: args.value.clone(),
                            outcome: None,
                            timeout_ms: args.timeout_ms,
                        },
                        reference,
                        args.allow_password.unwrap_or(false),
                        &args.action,
                    )
                    .await
                }
                "type" => {
                    let required = requires_key_events(self, &args).await;
                    let policy = typing_policy(
                        args.typing_mode.as_deref(),
                        args.delay_ms,
                        args.value.as_deref().unwrap_or_default(),
                        required,
                    )
                    .map_err(invalid)?;
                    let mode = policy.mode;
                    match mode {
                        crate::v3::TypingMode::Block => block_type(self, &args).await,
                        crate::v3::TypingMode::Ime => ime_type(self, &args).await,
                        crate::v3::TypingMode::FastKeys | crate::v3::TypingMode::HumanKeys => {
                            key_type(self, &args, policy).await
                        }
                    }
                }
                "press" => press(self, &args).await,
                "page_tools" | "page_tool" => call_page_tool(self, &args).await,
                _ => Err(invalid(
                    "action must be click, fill, type, press, select, page_tools, or page_tool",
                )),
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Ok(action_timeout_receipt(&args.action)),
        };
        controlla_browser::scheduler::telemetry::record_phase(
            controlla_browser::scheduler::telemetry::OperationPhase::Dispatch,
            dispatch_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        ).await;
        let result = if background_mutation { match result {
            Ok(mut value) => {
                let before = presentation_before.unwrap_or(Value::Null);
                let after = presentation_state(self, &args.session_id, &args.chrome_tab_id).await.unwrap_or(Value::Null);
                value["mode"] = json!("background");
                value["presentation_before"] = before.clone();
                value["presentation_after"] = after.clone();
                value["focus_changed_during_operation"] = focus_changed_during_operation(&before, &after);
                Ok(value)
            }
            Err(error) => Err(error),
        }} else { result };
        if !matches!(&result, Ok(value) if value["status"] == "unknown") {
            release_lease(self, &args, &lease).await;
        }
        result.map(|value| Json(compact_workflow_receipt(value, false, true)))
    }

    #[tool(
        description = "Run a bounded browser program in one MCP call. It shares the active browser call's 30-second, 256 browser-command, and 20,000-CDP-command budget. Use either steps (linear) or program (VM control flow: step, if with true/false, var_present:<name>, or var_equals:<name>:<value>, bounded foreach, or safe try_fallback); fallback is refused after any primary mutation has succeeded, and unknown mutations never fall back. A dispatched or unknown mutation may continue only to an immediately following read-only Verify; it is never replayed. Parallel read operations are not exposed for a single-tab workflow because the bridge serializes commands to that tab. Snapshot steps accept the same bounded goal, max_scan_nodes, max_items, and max_text_chars fields as snapshot; text_requested:false means text was omitted. Supports snapshot, find, find_all, collect_virtual, choose newest by datetime, click, fill/select, fill_form (up to 16 guarded input/select fields in one bridge program), type, press, upload a previously registered artifact through an observed trigger with an explicit account marker and optional acceptance predicate, page_tool, extract, and verify; up to 40 top-level operations and 64 expanded steps. Adjacent deterministic fill/select steps are compiled into one guarded bridge program with per-field readbacks. Read-only failures can use try_fallback; a following read-only Verify may inspect an unknown mutation without replaying it. Visible-trigger Upload is available only in local_unrestricted shared sessions; guarded uploads use the dedicated headless task route. For upload account_marker, use [\"ax:button\",\"exact visible accessible name\"] when no unique CSS selector exists; it checks the visible root-DOM button name and excludes picker/menu scopes. Otherwise pass [\"<CSS selector>\",\"exact visible name\"]. Upload stops the workflow unless the receipt proves application acceptance.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<std::collections::BTreeMap<String, Value>>()
    )]
    async fn workflow(
        &self,
        Parameters(args): Parameters<V3WorkflowArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let wall_limit = Duration::from_millis(crate::v3::CommandBudget::interactive_default().max_wall_ms);
        measured_receipt(controlla_browser::scheduler::telemetry::with_browser_command_budget(
            crate::v3::CommandBudget::interactive_default().max_browser_commands as u64,
            crate::v3::CommandBudget::interactive_default().max_cdp_commands as u64,
            wall_limit,
            super::super::with_internal_workflow_state(async move {
                match tokio::time::timeout(wall_limit, Box::pin(self.workflow_inner(args))).await {
                    Ok(result) => result,
                    Err(_) => Ok(Json(json!({"status":"unknown","reason_code":"workflow_wall_time_exceeded",
                        "dispatch_may_have_occurred":true,"next_action":"inspect current page state before retrying"}))),
                }
            }),
        )).await
    }

    async fn execute_v3_workflow_step(
        &self,
        args: &V3WorkflowArgs,
        state: &mut V3WorkflowState,
        step: V3WorkflowStep,
        variables: &std::collections::BTreeMap<String, String>,
    ) -> Result<(Value, Option<(String, String)>), rmcp::ErrorData> {
        let vars = variables;
        let collections = &mut state.collections;
        let selected = &mut state.selected;
        let mut coverage_complete = state.coverage_complete;
        let pending_readback = state.pending_readback;
        let mut unverified_effect = state.unverified_effect;
        let mut variable_update = None;
        let resolve = |reference: String, variables: &std::collections::BTreeMap<String, String>| {
            crate::browser_workflow::resolve_workflow_reference(&reference, variables).map_err(invalid)
        };
        let value = match step {
                V3WorkflowStep::Snapshot { href_hint, max_items, max_text_chars, goal, max_scan_nodes } => {
                    let snapshot = snapshot_impl(&self.core, &BrowserSnapshotArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        selector: None,
                        max_items,
                        max_text_chars,
                        max_bytes: None,
                        href_hint: href_hint.clone(),
                        frame_id: None,
                        goal,
                        max_scan_nodes,
                    }).await?;
                    coverage_complete = snapshot["scope_complete"] == true;
                    snapshot
                }
                V3WorkflowStep::Find {
                    query,
                    role,
                    match_mode,
                    max_matches,
                    within_reference,
                    href_hint,
                    save_as,
                } => {
                    let find_args = BrowserFindArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        query,
                        role,
                        limit: Some(2),
                        max_matches,
                        match_mode,
                        within_reference,
                        href_hint,
                    };
                    let grounding_started = std::time::Instant::now();
                    let mut found = find_impl(&self.core, &find_args).await?;
                    if controlla_runtime::semantic_state::workflow_find_needs_refresh(&found) {
                        let refreshed = snapshot_impl(&self.core, &BrowserSnapshotArgs {
                            session_id: find_args.session_id.clone(),
                            chrome_tab_id: find_args.chrome_tab_id.clone(),
                            selector: None,
                            max_items: Some(200),
                            max_text_chars: Some(0),
                            max_bytes: Some(100_000),
                            href_hint: None,
                            frame_id: None,
                            goal: Some(super::SnapshotGoal {
                                query: Some(find_args.query.clone()),
                                roles: find_args.role.clone().map(|role| vec![role]),
                                href_hint: find_args.href_hint.clone(),
                                within: found["matches"].as_array().and_then(|matches| matches.first())
                                    .and_then(|item| item.get("semantic_ancestor"))
                                    .filter(|ancestor| ancestor["id"].as_str().is_some_and(|id| !id.is_empty()))
                                    .and_then(|ancestor| Some(super::SnapshotSemanticAncestor {
                                        tag: ancestor["tag"].as_str()?.to_owned(),
                                        id: ancestor["id"].as_str()?.to_owned(),
                                    })),
                                exhaustive: true,
                            }),
                            max_scan_nodes: Some(65_536),
                        }).await?;
                        if refreshed["scope_complete"] != true {
                            return Err(invalid("targeted semantic refresh was incomplete; inspect a fresh full snapshot before acting"));
                        }
                        found = find_impl(&self.core, &find_args).await?;
                    }
                    controlla_browser::scheduler::telemetry::record_phase(
                        controlla_browser::scheduler::telemetry::OperationPhase::Grounding,
                        grounding_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                    ).await;
                    let reference = workflow_find_reference(&found)?;
                    if save_as.is_empty() || vars.contains_key(&save_as) || collections.contains_key(&save_as) {
                        return Err(invalid("save_as must be unique and nonempty"));
                    }
                    variable_update = Some((save_as, reference));
                    found
                }
                V3WorkflowStep::FindAll { query, role, href_hint, max_matches, save_as } => {
                    if save_as.is_empty() || save_as.len() > 64 || collections.contains_key(&save_as) || vars.contains_key(&save_as) || max_matches == 0 || max_matches > 200 || query.as_ref().is_some_and(|q| q.len() > 512) || href_hint.as_ref().is_some_and(|h| h.is_empty() || h.len() > 512) {
                        return Err(invalid("find_all requires unique save_as and bounded filters/max_matches"));
                    }
                    let state = self.core.semantic.lock().await;
                    let state = state.get(&super::tab_key(&args.session_id, &args.chrome_tab_id)).ok_or_else(|| invalid("find_all requires a preceding snapshot"))?;
                    let matches = state.page_ordered_items().into_iter().filter(|item| {
                        role.as_ref().is_none_or(|wanted| item["role"].as_str() == Some(wanted))
                            && query.as_ref().is_none_or(|wanted| item["name"].as_str().is_some_and(|name| name.to_lowercase().contains(&wanted.to_lowercase())))
                            && href_hint.as_ref().is_none_or(|wanted| item["href"].as_str().is_some_and(|href| href.contains(wanted)))
                    }).take(max_matches + 1).cloned().collect::<Vec<_>>();
                    let limited = matches.len() > max_matches;
                    let matches = matches.into_iter().take(max_matches).collect::<Vec<_>>();
                    collections.insert(save_as, (matches.clone(), coverage_complete && !limited));
                    json!({"status":"collected","matches":matches,"count":matches.len(),"limited":limited,"coverage_complete":coverage_complete})
                }
                V3WorkflowStep::CollectVirtual { container, query, role, identity_fields, max_scrolls, max_items, save_as } => {
                    if collections.contains_key(&save_as) || vars.contains_key(&save_as) {
                        return Err(invalid("collect_virtual save_as must be unique"));
                    }
                    let mut by_identity = std::collections::BTreeMap::<String, Value>::new();
                    let mut encounter_order = Vec::new();
                    let mut missing_identity = false;
                    let mut end_of_list = false;
                    let mut reached_end = false;
                    let mut slice_incomplete = false;
                    let mut scrolls = 0_u8;
                    for page in 0..=max_scrolls {
                        if reached_end {
                            tokio::time::sleep(Duration::from_millis(350)).await;
                        }
                        let snapshot = snapshot_impl(&self.core, &BrowserSnapshotArgs {
                            session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(),
                            selector: container.clone(), max_items: Some(max_items.min(200)), max_text_chars: Some(0),
                            max_bytes: Some(32_000), href_hint: None, frame_id: None,
                            goal: None, max_scan_nodes: Some(65_536),
                        }).await?;
                        slice_incomplete |= snapshot["scope_complete"] != true;
                        let candidates = snapshot["items"].as_array().cloned().unwrap_or_default();
                        let mut added_this_page = 0;
                        for item in candidates.into_iter().filter(|item| {
                            role.as_ref().is_none_or(|wanted| item["role"].as_str() == Some(wanted))
                                && query.as_ref().is_none_or(|wanted| item["name"].as_str().is_some_and(|name| name.to_lowercase().contains(&wanted.to_lowercase())) || item["text"].as_str().is_some_and(|text| text.to_lowercase().contains(&wanted.to_lowercase())))
                        }) {
                            let Some(key) = virtual_item_identity(&item, &identity_fields) else { missing_identity = true; continue; };
                            if !by_identity.contains_key(&key) {
                                let mut item = item;
                                item["collection_order"] = json!(by_identity.len());
                                encounter_order.push(key.clone());
                                by_identity.insert(key, item);
                                added_this_page += 1;
                            }
                            if by_identity.len() >= max_items { break; }
                        }
                        if by_identity.len() >= max_items { break; }
                        if reached_end && added_this_page > 0 { reached_end = false; }
                        if reached_end { end_of_list = true; break; }
                        if page == max_scrolls { break; }
                        let selector = serde_json::to_string(container.as_deref().unwrap_or("")).unwrap_or_else(|_| "\"\"".into());
                        let expression = format!("({VIRTUAL_SCROLL_WITH_RECORDS})({selector},{})", max_items.min(50));
                        let live = self.core.legacy.shared_sessions.lock().await.get(&args.session_id).cloned().ok_or_else(|| invalid("unknown session"))?;
                        let live = live.lock().await;
                        let connection = live.connection.as_ref().ok_or_else(|| invalid("session is not accepted"))?;
                        let response = connection.command(live.registry()?, &live.handle, &args.chrome_tab_id, "Runtime.evaluate", json!({"expression":expression,"awaitPromise":true,"returnByValue":true})).await.map_err(|error| invalid(error.to_string()))?;
                        let scroll = super::super::shared_value(&response).map_err(invalid)?;
                        if scroll["ok"] != true { return Err(invalid(scroll["reason"].as_str().unwrap_or("virtual list scroll failed"))); }
                        slice_incomplete |= scroll["records_truncated"] == true;
                        for mut item in scroll["records"].as_array().cloned().unwrap_or_default().into_iter().filter(|item| {
                            role.as_ref().is_none_or(|wanted| item["role"].as_str() == Some(wanted))
                                && query.as_ref().is_none_or(|wanted| item["name"].as_str().is_some_and(|name| name.to_lowercase().contains(&wanted.to_lowercase())) || item["text"].as_str().is_some_and(|text| text.to_lowercase().contains(&wanted.to_lowercase())))
                        }) {
                            let Some(key) = virtual_item_identity(&item, &identity_fields) else { missing_identity = true; continue; };
                            if !by_identity.contains_key(&key) {
                                item["collection_order"] = json!(by_identity.len());
                                encounter_order.push(key.clone());
                                by_identity.insert(key, item);
                            }
                            if by_identity.len() >= max_items { break; }
                        }
                        if by_identity.len() >= max_items { break; }
                        if scroll["at_end"] == true { reached_end = true; }
                        else if scroll["advanced"] != true { break; }
                        scrolls = scrolls.saturating_add(1);
                    }
                    let matches = encounter_order.into_iter().filter_map(|key| by_identity.remove(&key)).take(max_items).collect::<Vec<_>>();
                    let complete = end_of_list && matches.len() < max_items && !missing_identity && !slice_incomplete;
                    coverage_complete = complete;
                    collections.insert(save_as, (matches.clone(), complete));
                    json!({"status":"collected","matches":matches,"count":matches.len(),"scrolls":scrolls,"end_of_list":end_of_list,"coverage_complete":complete,"slice_incomplete":slice_incomplete,"identity_incomplete":missing_identity,"deduplicated":true})
                }
                V3WorkflowStep::Choose { from, key, save_as } => {
                    if save_as.is_empty() || save_as.len() > 64 || vars.contains_key(&save_as) || collections.contains_key(&save_as) || key != "datetime" {
                        return Err(invalid("choose requires unique save_as and supported key datetime"));
                    }
                    let (items, complete) = collections.get(&from).ok_or_else(|| invalid("choose source was not collected"))?;
                    let mut items = items.clone();
                    let enrichment_started = std::time::Instant::now();
                    let metadata_frontier = if *complete { linked_date_frontier(&items, 12) } else { Vec::new() };
                    let mut metadata_error = None;
                    let mut inspected = 0;
                    for mut batch in metadata_frontier.chunks(4).map(<[Value]>::to_vec) {
                        if unresolved_linked_date_count(&items) == 0 { break; }
                        inspected += batch.len();
                        match enrich_linked_dates(self, &args.session_id, &args.chrome_tab_id, &mut batch).await {
                            Ok(()) => for candidate in batch {
                                if let Some(item) = items.iter_mut().find(|item| item["href"] == candidate["href"]) {
                                    if candidate["datetime_utc_ms"].as_i64().is_some() {
                                        item["datetime"] = candidate["datetime"].clone();
                                        item["datetime_utc_ms"] = candidate["datetime_utc_ms"].clone();
                                        item["datetime_precision"] = json!("instant");
                                        item["freshness_upper_bound_ms"] = Value::Null;
                                        item["freshness_source"] = candidate["freshness_source"].clone();
                                    } else if !candidate["freshness_error"].is_null() {
                                        item["freshness_error"] = candidate["freshness_error"].clone();
                                    }
                                }
                            },
                            Err(error) => { metadata_error = Some(error); break; }
                        }
                        if choose_newest_with_bounds(&items, *complete).is_some() { break; }
                    }
                    let unresolved = unresolved_linked_date_count(&items);
                    let proven_choice = choose_newest_with_bounds(&items, *complete);
                    let freshness_incomplete = *complete && unresolved > 0 && proven_choice.is_none();
                    if freshness_incomplete && metadata_error.is_none() {
                        metadata_error = Some(format!("staged linked-date enrichment inspected {inspected} candidates; {unresolved} remain unresolved, so newest selection is not proven"));
                    }
                    controlla_browser::scheduler::telemetry::record_phase(
                        controlla_browser::scheduler::telemetry::OperationPhase::Enrichment,
                        enrichment_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                    ).await;
                    match proven_choice.ok_or("unresolved_latest") {
                        Ok(choice) => {
                            let reference = choice["selected"]["reference"].as_str().ok_or_else(|| invalid("selected item omitted reference"))?.to_owned();
                            variable_update = Some((save_as.clone(), reference));
                            selected.insert(save_as, choice["selected"].clone());
                            json!({"status":"selected","selected":choice["selected"],"freshness_evidence":choice["freshness_evidence"]})
                        }
                        Err(reason) => json!({"status":"failed","reason_code":if freshness_incomplete { "candidate_freshness_incomplete" } else { reason },"retry_class":"do_not_retry","next_action":"inspect page coverage and freshness evidence","metadata_error":metadata_error,"checked_candidates":items.iter().filter(|item| item["datetime_utc_ms"].as_i64().is_some()).map(|item| json!({"href":item["href"],"utc_ms":item["datetime_utc_ms"],"source":item["freshness_source"],"collection_order":item["collection_order"]})).collect::<Vec<_>>(),"unchecked_candidates":items.iter().filter(|item| item["datetime_utc_ms"].as_i64().is_none() && item["href"].as_str().is_some_and(|href| !href.is_empty())).map(|item| json!({"href":item["href"],"error":item["freshness_error"],"precision":item["datetime_precision"],"upper_bound_ms":item["freshness_upper_bound_ms"],"collection_order":item["collection_order"]})).collect::<Vec<_>>(),"unresolved_candidates":unresolved_linked_date_count(&items),"candidate_freshness":items.iter().map(|item| json!({"href":item["href"],"datetime":item["datetime"],"utc_ms":item["datetime_utc_ms"],"precision":item["datetime_precision"],"upper_bound_ms":item["freshness_upper_bound_ms"],"source":item["freshness_source"],"collection_order":item["collection_order"]})).collect::<Vec<_>>()})
                    }
                }
                V3WorkflowStep::Click { reference, outcome } => {
                    let reference = resolve(reference, vars)?;
                    self.act(Parameters(V3ActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "click".into(),
                        reference: Some(reference),
                        expected_value: None,
                        value: None,
                        key: None,
                        outcome,
                        timeout_ms: None,
                        typing_mode: None,
                        delay_ms: None,
                        client_id: args.client_id.clone(),
                        page_tool_name: None,
                        page_tool_input: None,

                        allow_password: None,
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Upload { artifact_handle, trigger_reference, account_marker, acceptance } => {
                    let trigger_reference = resolve(trigger_reference, vars)?;
                    let acceptance = acceptance
                        .map(|predicate| resolve_workflow_predicate(predicate, vars, selected))
                        .transpose()?
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| invalid(format!("invalid upload acceptance predicate: {error}")))?;
                    match tokio::time::timeout(
                        SHARED_TRIGGER_UPLOAD_DEADLINE,
                        shared_file_select_trigger(
                            self,
                            &args.session_id,
                            &args.chrome_tab_id,
                            &artifact_handle,
                            &trigger_reference,
                            &account_marker,
                            acceptance,
                        ),
                    ).await {
                        Ok(result) => result?,
                        Err(_) => json!({"status":"unknown","reason_code":"upload_deadline_exceeded","dispatch_may_have_occurred":true,"reason":"Upload exceeded the 45-second operation deadline; inspect page state before retrying."}),
                    }
                }
                V3WorkflowStep::Fill {
                    reference,
                    expected_value,
                    value,
                } => {
                    let reference = resolve(reference, vars)?;
                    self.act(Parameters(V3ActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "fill".into(),
                        reference: Some(reference),
                        expected_value: Some(expected_value),
                        value: Some(value),
                        key: None,
                        outcome: None,
                        timeout_ms: None,
                        typing_mode: None,
                        delay_ms: None,
                        client_id: args.client_id.clone(),
                        page_tool_name: None,
                        page_tool_input: None,

                        allow_password: None,
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Select { reference, expected_value, value } => {
                    let reference = resolve(reference, vars)?;
                    self.act(Parameters(V3ActArgs {
                        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "select".into(), reference: Some(reference), expected_value: Some(expected_value), value: Some(value),
                        key: None, outcome: None, timeout_ms: None, typing_mode: None, delay_ms: None,
                        client_id: args.client_id.clone(), page_tool_name: None, page_tool_input: None, allow_password: None,
                    })).await?.0
                }
                V3WorkflowStep::FillForm { fields } => {
                    workflow_fill_form(self, args, fields, vars).await?
                }
                V3WorkflowStep::Type {
                    reference,
                    expected_value,
                    value,
                    typing_mode,
                    delay_ms,
                } => {
                    let reference = resolve(reference, vars)?;
                    self.act(Parameters(V3ActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "type".into(),
                        reference: Some(reference),
                        expected_value: Some(expected_value),
                        value: Some(value),
                        key: None,
                        outcome: None,
                        timeout_ms: None,
                        typing_mode,
                        delay_ms,
                        client_id: args.client_id.clone(),
                        page_tool_name: None,
                        page_tool_input: None,

                        allow_password: None,
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Press { reference, key } => {
                    let reference = resolve(reference, vars)?;
                    self.act(Parameters(V3ActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "press".into(),
                        reference: Some(reference),
                        expected_value: None,
                        value: None,
                        key: Some(key),
                        outcome: None,
                        timeout_ms: None,
                        typing_mode: None,
                        delay_ms: None,
                        client_id: args.client_id.clone(),
                        page_tool_name: None,
                        page_tool_input: None,

                        allow_password: None,
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Extract { selector, fields } => {
                    let fields = fields.unwrap_or_else(|| std::collections::BTreeMap::from([("text".to_owned(), selector.clone())]));
                    if fields.is_empty() { return Err(invalid("extract fields cannot be empty")); }
                    self.core
                        .browser_extract(Parameters(BrowserExtractArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            selector,
                            fields,
                            max_items: Some(100),
                            max_text_chars: Some(6000),
                            max_bytes: Some(100_000),
                        }))
                        .await?
                        .0
                }
                V3WorkflowStep::PageTool { name, input } => {
                    self.act(Parameters(V3ActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "page_tool".into(),
                        reference: None,
                        expected_value: None,
                        value: None,
                        key: None,
                        outcome: None,
                        timeout_ms: None,
                        typing_mode: None,
                        delay_ms: None,
                        client_id: args.client_id.clone(),
                        page_tool_name: Some(name),
                        page_tool_input: Some(input),

                        allow_password: None,
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Verify { predicate } => {
                    let predicate = resolve_workflow_predicate(predicate, vars, selected)?;
                    let mut observed = json!({"status":"unknown","reason":"verification did not complete"});
                    let mut verification_error = None;
                    for attempt in 0..if pending_readback { 20 } else { 1 } {
                        match verify_impl(&self.core, &BrowserVerifyArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            predicate: predicate.clone(),
                        }).await {
                            Ok(value) => observed = value,
                            Err(error) if !pending_readback => {
                                verification_error = Some(error);
                                break;
                            }
                            Err(error) => observed = json!({"status":"unknown","reason":error.to_string()}),
                        }
                        if !pending_readback || observed["status"] == "passed" || attempt == 19 { break; }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    if let Some(error) = verification_error { return Err(error); }
                    if pending_readback && observed["status"] == "passed" { unverified_effect = false; }
                    observed
                }
            };
        state.coverage_complete = coverage_complete;
        state.unverified_effect = unverified_effect;
        Ok((value, variable_update))
    }

    async fn workflow_inner(&self, args: V3WorkflowArgs) -> Result<Json<Value>, rmcp::ErrorData> {
        let mut args = args;
        self.require_v3_session(&args.session_id).await?;
        if let Some(program) = args.program.take() {
            if !args.steps.is_empty() { return Err(invalid("provide either steps or program, not both")); }
            if program.is_empty() || program.len() > 40 { return Err(invalid("workflow program requires 1..40 top-level operations")); }
            validate_workflow_ops(&program, 0)?;
            return self.execute_workflow_program(args, compile_workflow_ops(program)?).await;
        }
        if args.steps.is_empty() || args.steps.len() > 40 {
            return Err(invalid("workflow requires 1..40 steps"));
        }
        validate_workflow_steps(&args.steps)?;
        let started = std::time::Instant::now();
        let mut receipts: Vec<Value> = Vec::with_capacity(args.steps.len());
        let mut step_ms = Vec::with_capacity(args.steps.len());
        let mut workflow_vm = crate::workflow_vm::Vm::new();
        let mut steps = compile_fill_runs(std::mem::take(&mut args.steps)).into_iter().peekable();
        let workflow_host = V3WorkflowHost { app: self, args: &args, state: Mutex::new(V3WorkflowState::default()) };
        while let Some(step) = steps.next() {
            let step_name = workflow_step_name(&step);
            let compactable_read = matches!(&step, V3WorkflowStep::Snapshot { .. } | V3WorkflowStep::Extract { .. } | V3WorkflowStep::FindAll { .. } | V3WorkflowStep::CollectVirtual { .. });
            let is_upload = matches!(&step, V3WorkflowStep::Upload { .. });
            let is_mutation = matches!(&step, V3WorkflowStep::Click { .. } | V3WorkflowStep::Fill { .. } | V3WorkflowStep::Select { .. } | V3WorkflowStep::FillForm { .. } | V3WorkflowStep::Type { .. } | V3WorkflowStep::Press { .. } | V3WorkflowStep::PageTool { .. } | V3WorkflowStep::Upload { .. });
            let step_started = std::time::Instant::now();
            let execution = workflow_vm.run_op(
                crate::workflow_vm::VmOp::Step(step),
                &workflow_host,
            ).await;
            let value = match execution {
                Ok(()) => workflow_vm.receipts.last().ok_or_else(|| invalid("workflow VM returned no step receipt"))
                    .and_then(|receipt| serde_json::from_str::<Value>(&receipt.value).map_err(|error| invalid(format!("workflow VM receipt is invalid: {error}")))),
                Err(error) => {
                    let verify_unknown_next = matches!(&error, crate::workflow_vm::VmError::UnknownMutation(_))
                        && matches!(steps.peek(), Some(V3WorkflowStep::Verify { .. }));
                    let (default_status, detail) = match &error {
                        crate::workflow_vm::VmError::NotDispatched(detail) => ("not_dispatched", Some(detail.as_str())),
                        crate::workflow_vm::VmError::ReadFailure(detail) => ("failed", Some(detail.as_str())),
                        crate::workflow_vm::VmError::UnknownMutation(detail) => ("unknown", Some(detail.as_str())),
                        crate::workflow_vm::VmError::Failed(detail) => ("failed", Some(detail.as_str())),
                        _ => ("failed", None),
                    };
                    let mut failure = detail.and_then(|detail| serde_json::from_str::<Value>(detail).ok())
                        .unwrap_or_else(|| json!({"status":default_status,"reason":format!("workflow VM stopped: {error:?}")}));
                    failure["step"] = json!(step_name);
                    let status = failure["status"].as_str().unwrap_or(default_status).to_owned();
                    receipts.push(compact_workflow_receipt(failure, false, is_mutation));
                    step_ms.push(step_started.elapsed().as_millis() as u64);
                    if verify_unknown_next { continue; }
                    return Ok(Json(json!({"status":status,"failed_step":receipts.len()-1,
                        "steps":receipts.len(),"receipts":receipts,"step_ms":step_ms,
                        "elapsed_ms":started.elapsed().as_millis() as u64})));
                }
            };
            let value = match value {
                Ok(value) => value,
                Err(error) => {
                    let failure = json!({"status":"failed","reason":error.to_string(),"step":step_name});
                    receipts.push(compact_workflow_receipt(failure, false, is_mutation));
                    step_ms.push(step_started.elapsed().as_millis() as u64);
                    return Ok(Json(json!({"status":"failed","failed_step":receipts.len()-1,
                        "steps":receipts.len(),"receipts":receipts,"step_ms":step_ms,
                        "elapsed_ms":started.elapsed().as_millis() as u64})));
                }
            };
            let elapsed_ms = step_started.elapsed().as_millis() as u64;
            let mut value = value;
            value["step"] = json!(step_name);
            value["elapsed_ms"] = json!(elapsed_ms);
            let stop_status = (workflow_should_stop_before_next(&value, steps.peek())
                || (is_upload && workflow_upload_requires_stop(&value)))
                .then(|| value["status"].as_str().unwrap_or("unknown").to_owned());
            let mut state = workflow_host.state.lock().await;
            state.pending_readback = workflow_should_verify_next(&value, steps.peek()) && stop_status.is_none();
            state.unverified_effect |= matches!(value["status"].as_str(), Some("unknown" | "dispatched" | "not_dispatched" | "failed"));
            drop(state);
            step_ms.push(elapsed_ms);
            receipts.push(compact_workflow_receipt(value, compactable_read && steps.peek().is_some(), is_mutation));
            if let Some(status) = stop_status {
                return Ok(Json(
                    json!({"status":status,"steps":receipts.len(),"receipts":receipts,"step_ms":step_ms,"elapsed_ms":started.elapsed().as_millis() as u64}),
                ));
            }
        }
        Ok(Json(
            json!({"status":if workflow_host.state.lock().await.unverified_effect {"completed_unverified"} else {"completed"},"steps":receipts.len(),"receipts":receipts,"step_ms":step_ms,"elapsed_ms":started.elapsed().as_millis() as u64}),
        ))
    }

    async fn execute_workflow_program(
        &self,
        args: V3WorkflowArgs,
        program: Vec<crate::workflow_vm::VmOp<V3WorkflowStep>>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let started = std::time::Instant::now();
        let workflow_host = V3WorkflowHost { app: self, args: &args, state: Mutex::new(V3WorkflowState::default()) };
        let mut vm = crate::workflow_vm::Vm::new();
        let mut receipts = Vec::new();
        let mut op_ms = Vec::new();
        let operation_count = program.len();
        for (op_index, operation) in program.iter().cloned().enumerate() {
            let before = vm.receipts.len();
            let op_started = std::time::Instant::now();
            let execution = vm.run_op(operation, &workflow_host).await;
            let elapsed = op_started.elapsed().as_millis() as u64;
            op_ms.push(elapsed);
            let mut values = vm.receipts[before..].iter().filter_map(|receipt| serde_json::from_str::<Value>(&receipt.value).ok()).collect::<Vec<_>>();
            let verify_unknown_next = matches!(&execution, Err(crate::workflow_vm::VmError::UnknownMutation(_)))
                && matches!(program.get(op_index + 1), Some(crate::workflow_vm::VmOp::Step(step)) if crate::workflow_vm::VmOperation::is_verification(step));
            let failure = execution.err().map(|error| {
                let (status, detail) = match &error {
                    crate::workflow_vm::VmError::NotDispatched(detail) => ("not_dispatched", Some(detail.as_str())),
                    crate::workflow_vm::VmError::ReadFailure(detail) => ("failed", Some(detail.as_str())),
                    crate::workflow_vm::VmError::UnknownMutation(detail) => ("unknown", Some(detail.as_str())),
                    crate::workflow_vm::VmError::Failed(detail) => ("failed", Some(detail.as_str())),
                    _ => ("failed", None),
                };
                detail.and_then(|detail| serde_json::from_str::<Value>(detail).ok())
                    .unwrap_or_else(|| json!({"status":status,"reason":format!("workflow VM stopped: {error:?}")}))
            });
            let failure_already_recorded = failure.as_ref().is_some_and(|failure| values.iter().any(|value| value == failure));
            let has_more = op_index + 1 < operation_count;
            let nested_len = values.len();
            for (index, value) in values.drain(..).enumerate() {
                let is_read = matches!(value["step"].as_str(), Some("snapshot" | "extract" | "find_all" | "collect_virtual"));
                receipts.push(compact_workflow_receipt(value, is_read && (has_more || index + 1 < nested_len), false));
            }
            if let Some(mut failure) = failure {
                failure["failed_operation"] = json!(op_index);
                if failure_already_recorded {
                    if let Some(receipt) = receipts.last_mut() { receipt["failed_operation"] = json!(op_index); }
                } else {
                    receipts.push(failure);
                }
                if verify_unknown_next { continue; }
                return Ok(Json(json!({"status":receipts.last().and_then(|v|v["status"].as_str()).unwrap_or("failed"),"failed_operation":op_index,"steps":vm.step_count,"receipts":receipts,"operation_ms":op_ms,"elapsed_ms":started.elapsed().as_millis() as u64})));
            }
        }
        let unverified = workflow_host.state.lock().await.unverified_effect;
        Ok(Json(json!({"status":if unverified {"completed_unverified"} else {"completed"},"steps":vm.step_count,"receipts":receipts,"operation_ms":op_ms,"elapsed_ms":started.elapsed().as_millis() as u64})))
    }

    async fn artifact_register(
        &self,
        Parameters(args): Parameters<V3ArtifactRegisterArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let started = std::time::Instant::now();
        self.require_v3_session(&args.session_id).await?;
        let live = self
            .core
            .legacy
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        let mut live = live.lock().await;
        if live.handle.principal != self.core.legacy.principal.as_ref()
            || (!crate::guard_reason::unrestricted_mode() && !super::super::private_headless_cleanup_allowed(&live.handle))
        {
            return Err(invalid(
                "artifact registration requires a dedicated private headless session",
            ));
        }
        let inline = decode_artifact_bytes(args.bytes, args.bytes_base64)?;
        let bytes = match (inline, args.file_path) {
            (Some(bytes), None) => bytes,
            (None, Some(path)) if crate::guard_reason::unrestricted_mode() && !super::super::private_headless_cleanup_allowed(&live.handle) => crate::artifacts::read_local_file(&path).map_err(invalid)?,
            (None, Some(_)) => return Err(invalid("file_path staging requires a local unrestricted shared session")),
            (Some(_), Some(_)) => return Err(invalid("provide exactly one of bytes, bytes_base64, or file_path")),
            (None, None) => return Err(invalid("provide one of bytes, bytes_base64, or file_path")),
        };
        let handle = live.handle.clone();
        let private_headless = super::super::private_headless_cleanup_allowed(&handle);
        let metadata = live
            .registry
            .as_mut()
            .ok_or_else(|| invalid("session registry unavailable"))?
            .put_artifact_bytes_scoped(
                &handle,
                &args.filename,
                &bytes,
                if private_headless {
                    controlla_browser::sessions::ArtifactScope::PrivateHeadlessSession
                } else {
                    controlla_browser::sessions::ArtifactScope::LocalUnrestrictedSharedSession
                },
            )
            .map_err(|error| invalid(format!("artifact registration failed: {error:?}")))?;
        controlla_browser::scheduler::telemetry::record_phase(
            controlla_browser::scheduler::telemetry::OperationPhase::ArtifactTransfer,
            started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        ).await;
        Ok(Json(
            json!({"artifact_handle":metadata.handle,"filename":metadata.filename,"size":metadata.size,
            "scope":if super::super::private_headless_cleanup_allowed(&live.handle) {"private_headless_session"} else {"local_unrestricted_shared_session"},"verified":true,"elapsed_ms":started.elapsed().as_millis() as u64}),
        ))
    }

    async fn artifact_read(
        &self,
        Parameters(args): Parameters<V3ArtifactReadArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let started = std::time::Instant::now();
        self.require_v3_session(&args.session_id).await?;
        let live = self
            .core
            .legacy
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        let live = live.lock().await;
        if live.handle.principal != self.core.legacy.principal.as_ref()
            || (!crate::guard_reason::unrestricted_mode() && !super::super::private_headless_cleanup_allowed(&live.handle))
        {
            return Err(invalid(
                "artifact read requires the owning private headless session",
            ));
        }
        let handle: controlla_browser::sessions::ArtifactHandle =
            serde_json::from_value(json!(args.artifact_handle))
                .map_err(|error| invalid(format!("invalid artifact handle: {error}")))?;
        let bytes = live
            .registry()?
            .read_artifact_bytes(&live.handle, &handle)
            .map_err(|error| invalid(format!("artifact read failed: {error:?}")))?;
        let encoding = args.encoding.as_deref().unwrap_or("base64");
        let encoded = encode_artifact_read(&bytes, encoding)?;
        controlla_browser::scheduler::telemetry::record_phase(
            controlla_browser::scheduler::telemetry::OperationPhase::ArtifactTransfer,
            started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        ).await;
        let mut result = json!({"artifact_handle":handle,"size":bytes.len(),"elapsed_ms":started.elapsed().as_millis() as u64,
            "sha256":crate::artifacts::ArtifactExpectation::sha256(&bytes),
            "verified":true,"scope":if super::super::private_headless_cleanup_allowed(&live.handle) {"private_headless_session"} else {"local_unrestricted_shared_session"}});
        result.as_object_mut().unwrap().extend(encoded.as_object().unwrap().clone());
        Ok(Json(result))
    }

    async fn task(
        &self,
        Parameters(args): Parameters<V3TaskArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let budget = crate::v3::CommandBudget::interactive_default();
        let wall_limit = Duration::from_millis(budget.max_wall_ms);
        measured_receipt(controlla_browser::scheduler::telemetry::with_browser_command_budget(
            budget.max_browser_commands as u64,
            budget.max_cdp_commands as u64,
            wall_limit,
            super::super::with_internal_workflow_state(async move {
                match tokio::time::timeout(wall_limit, Box::pin(self.task_inner(args))).await {
                    Ok(result) => result,
                    Err(_) => Ok(Json(json!({"status":"unknown","reason_code":"task_wall_time_exceeded",
                        "dispatch_may_have_occurred":true,"next_action":"inspect current page state before retrying"}))),
                }
            }),
        )).await
    }

    async fn task_inner(
        &self,
        args: V3TaskArgs,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        use crate::v3_tasks::ExpandedStep;
        let steps = args.task.expand().map_err(invalid)?;
        let mut url = String::new();
        let mut active_tab_id = args.chrome_tab_id.clone();
        let mut receipts = Vec::with_capacity(steps.len());
        let mut unverified_effect = false;
        let mut retained_generation = false;
        let task_started = std::time::Instant::now();
        for step in steps {
            let step_started = std::time::Instant::now();
            let is_mutation = matches!(&step, ExpandedStep::Fill { .. } | ExpandedStep::Upload { .. } | ExpandedStep::Download { .. } | ExpandedStep::Click { .. } | ExpandedStep::SaveDraft { .. });
            let background_mode = self.browser_modes.lock().await.get(&args.session_id) == Some(&crate::v3::BrowserMode::Background);
            let presentation_before = if background_mode && is_mutation { presentation_state(self, &args.session_id, &active_tab_id).await } else { None };
            let (kind, reference, mut result) = match step {
                ExpandedStep::Open {
                    url: next_url,
                    tab_id,
                } => {
                    if let Some(tab_id) = tab_id {
                        active_tab_id = tab_id;
                    }
                    let value =
                        task_navigate(self, &args.session_id, &active_tab_id, &next_url).await;
                    url = next_url;
                    ("navigate", None, value)
                }
                ExpandedStep::Extract { selector, .. } => {
                    let value = self
                        .core
                        .browser_extract(Parameters(BrowserExtractArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: active_tab_id.clone(),
                            selector,
                            fields: std::collections::BTreeMap::from([(
                                "text".into(),
                                ":scope".into(),
                            )]),
                            max_items: Some(100),
                            max_text_chars: Some(2_000),
                            max_bytes: Some(20_000),
                        }))
                        .await.map(|Json(value)| value)
                        .unwrap_or_else(|error| json!({"status":"unknown","reason":format!("extraction failed after navigation; inspect page state: {error}")}));
                    ("extract", None, value)
                }
                ExpandedStep::Fill {
                    reference,
                    expected_value,
                    value,
                } => {
                    if !retained_generation {
                        snapshot_impl(&self.core, &BrowserSnapshotArgs {
                            session_id: args.session_id.clone(), chrome_tab_id: active_tab_id.clone(),
                            selector: None, max_items: None, max_text_chars: None, max_bytes: None,
                            href_hint: None, frame_id: None, goal: None, max_scan_nodes: None,
                        }).await?;
                    }
                    let action = self
                        .act(Parameters(V3ActArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: active_tab_id.clone(),
                            action: "fill".into(),
                            reference: Some(reference.clone()),
                            expected_value: Some(expected_value),
                            value: Some(value.clone()),
                            key: None,
                            outcome: None,
                            timeout_ms: None,
                            typing_mode: None,
                            delay_ms: None,
                            client_id: args.client_id.clone(),
                            page_tool_name: None,
                            page_tool_input: None,

                            allow_password: None,
                        }))
                        .await?
                        .0;
                    retained_generation = action["status"] == "verified" && action["delta"]["document_changed"] != true;
                    ("fill", Some(reference), action)
                }
                ExpandedStep::VerifyVisible { reference } => {
                    let value = verify_impl(
                        &self.core,
                        &BrowserVerifyArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            predicate: BrowserPredicate::Exists {
                                reference: reference.clone(),
                            },
                        },
                    )
                    .await?;
                    ("verify_visible", Some(reference), value)
                }
                ExpandedStep::Click { reference, outcome } => {
                    let outcome: BrowserClickOutcome =
                        serde_json::from_value(outcome).map_err(|error| {
                            invalid(format!("invalid repeat click outcome: {error}"))
                        })?;
                    let _ = snapshot_impl(
                        &self.core,
                        &BrowserSnapshotArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            selector: None,
                            max_items: None,
                            max_text_chars: None,
                            max_bytes: None,
                            href_hint: None,
                            frame_id: None,
            goal: None,
            max_scan_nodes: None,
                        },
                    )
                    .await?;
                    let value = self
                        .act(Parameters(V3ActArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            action: "click".into(),
                            reference: Some(reference.clone()),
                            expected_value: None,
                            value: None,
                            key: None,
                            outcome: Some(outcome),
                            timeout_ms: None,
                            typing_mode: None,
                            delay_ms: None,
                            client_id: args.client_id.clone(),
                            page_tool_name: None,
                            page_tool_input: None,

                            allow_password: None,
                        }))
                        .await?
                        .0;
                    ("click", Some(reference), value)
                }
                ExpandedStep::SaveDraft { control } => {
                    let _ = snapshot_impl(
                        &self.core,
                        &BrowserSnapshotArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            selector: None,
                            max_items: None,
                            max_text_chars: None,
                            max_bytes: None,
                            href_hint: None,
                            frame_id: None,
            goal: None,
            max_scan_nodes: None,
                        },
                    )
                    .await?;
                    let value = self
                        .act(Parameters(V3ActArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            action: "click".into(),
                            reference: Some(control.reference.clone()),
                            expected_value: None,
                            value: None,
                            key: None,
                            outcome: Some(BrowserClickOutcome::Text {
                                selector: control.confirmation_selector,
                                text: control.confirmation_text,
                            }),
                            timeout_ms: None,
                            typing_mode: None,
                            delay_ms: None,
                            client_id: args.client_id.clone(),
                            page_tool_name: None,
                            page_tool_input: None,

                            allow_password: None,
                        }))
                        .await?
                        .0;
                    ("save_draft", Some(control.reference), value)
                }
                ExpandedStep::Upload {
                    artifact_handle,
                    selector,
                    trigger_reference,
                    acceptance,
                    account_marker,
                } => {
                    let value = if let Some(reference)=trigger_reference {
                        match tokio::time::timeout(SHARED_TRIGGER_UPLOAD_DEADLINE, shared_file_select_trigger(self,&args.session_id,&args.chrome_tab_id,&artifact_handle,&reference,&account_marker,acceptance)).await {
                            Ok(result) => result?,
                            Err(_) => json!({"status":"unknown","reason_code":"upload_deadline_exceeded","dispatch_may_have_occurred":true,"reason":"Upload exceeded the 45-second operation deadline; inspect Classroom before retrying."}),
                        }
                    } else {
                        let mut value=headless_file_select(self,&args.session_id,&args.chrome_tab_id,&artifact_handle,selector.as_deref().unwrap(),&account_marker).await?;
                        if let Some(acceptance)=acceptance {
                            let predicate:BrowserPredicate=serde_json::from_value(acceptance).map_err(|error|invalid(format!("invalid upload acceptance predicate: {error}")))?;
                            let mut accepted=false;
                            for attempt in 0..10 {
                                if verify_impl(&self.core,&BrowserVerifyArgs{session_id:args.session_id.clone(),chrome_tab_id:args.chrome_tab_id.clone(),predicate:predicate.clone()}).await?["status"]=="passed" { accepted=true;break; }
                                if attempt<9 {tokio::time::sleep(Duration::from_millis(100)).await;}
                            }
                            value["application_accepted"]=json!(accepted);
                            if accepted {value["status"]=json!("verified");value["verified"]=json!(true);}
                        }
                        value
                    };
                    ("upload", None, value)
                }
                ExpandedStep::Download { reference } => {
                    let value =
                        headless_download(self, &args.session_id, &args.chrome_tab_id, &reference)
                            .await?;
                    ("download", Some(reference), value)
                }
            };
            if kind != "fill" { retained_generation = false; }
            if background_mode && is_mutation {
                let after = presentation_state(self, &args.session_id, &active_tab_id).await;
                let before = presentation_before.unwrap_or(Value::Null);
                let after_value = after.unwrap_or(Value::Null);
                let changed = focus_changed_during_operation(&before, &after_value);
                result["mode"] = json!("background");
                result["presentation_before"] = before;
                result["presentation_after"] = after_value;
                result["focus_changed_during_operation"] = changed;
            }
            let mut receipt = task_receipt(kind, &url, reference.as_deref(), &result);
            receipt["elapsed_ms"] = json!(step_started.elapsed().as_millis() as u64);
            let verified = receipt["verified"] == true;
            unverified_effect |= !verified;
            receipts.push(receipt);
            if (!verified && !crate::guard_reason::unrestricted_mode()) || workflow_should_stop(&result) {
                let status = if result["status"] == "selected" {
                    "incomplete"
                } else {
                    result["status"].as_str().unwrap_or("incomplete")
                };
                return Ok(Json(
                    json!({"status":status,"steps":receipts.len(),"receipts":receipts,"elapsed_ms":task_started.elapsed().as_millis() as u64}),
                ));
            }
        }
        Ok(Json(
            json!({"status":if unverified_effect {"completed_unverified"} else {"completed"},"steps":receipts.len(),"receipts":receipts,"elapsed_ms":task_started.elapsed().as_millis() as u64}),
        ))
    }

    #[tool(
        description = "Extract bounded structured page data without screenshots.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<std::collections::BTreeMap<String, Value>>()
    )]
    async fn extract(
        &self,
        Parameters(args): Parameters<BrowserExtractArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        measured_receipt(async {
            self.require_v3_session(&args.session_id).await?;
            self.core.browser_extract(Parameters(args)).await
        }).await
    }

    #[tool(
        description = "Independently reobserve the browser and verify a semantic postcondition.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<std::collections::BTreeMap<String, Value>>()
    )]
    async fn verify(
        &self,
        Parameters(args): Parameters<BrowserVerifyArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        measured_receipt(async {
            self.require_v3_session(&args.session_id).await?;
            verify_impl(&self.core, &args).await.map(Json)
        }).await
    }

}

#[tool_handler]
impl ServerHandler for AppV3 {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "controlla-v3",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions("Controlla V3 exposes browser, snapshot, act, workflow, extract, and verify. Read these tool schemas once per session; after an MCP reload, confirm browser schema exposes idempotency_key as required for action=open and a tagged task shape before calling it. If offered schema rejects a runtime-required field, stop and reload/repair the MCP contract instead of probing retries. For action=task use {\"kind\":\"research\",\"urls\":[\"https://example.test/\"]}; research navigates the selected tab. For a new foreground tab, call browser action=open with a unique logical-operation idempotency_key; reuse it only for that operation's retry, and omit open when the selected tab should be reused. Open waits for DOMContentLoaded by default; select ready_when=document_committed to attach earlier, or use URL prefix, CSS selector, or exact semantic match to gate dependent steps after commit. A readiness timeout returns incomplete and does not dispatch steps. Password controls appear only as redacted credential_input references with raw_value=null; fill one only with act action=fill, its fresh reference, and allow_password=true. This sends the value through the local MCP/browser, but snapshots and results remain redacted. Do not use credential values in workflows. Use one compact snapshot scoped by href_hint or visible query, then workflow find/find_all/choose(datetime) for ranked candidates. A click Navigation outcome with no URL means any document change; a supplied navigation URL is an exact absolute HTTP(S) destination. For canonical redirects, click with navigation-only and follow with verify url_starts_with. A click receipt of unknown is never retried: inspect once or verify read-only. Browser open/task, workflow, and standalone act each share a 30-second, 256-browser-command, 20,000-CDP-command budget; receipts report usage and remaining budget. FastKeys reserves its full bounded work before dispatch and returns not_dispatched if the budget cannot cover it. Prefer fast_keys for exact text; rich-editor newline runs become one Enter event each to preserve paragraph boundaries. Use human_keys only when requested or key-event pacing is required; it defaults to 180 ms per character (about 55 words per minute), accepts a delay_ms override, and scales its deadline with text length up to the 30-second act limit. For upload, the visible-trigger workflow Upload and foreground file_path artifact registration work only in local_unrestricted shared sessions; guarded uploads use the dedicated headless task upload route. Register the exact file once, pass the observed trigger reference, account marker, and bounded application acceptance predicate; an upload without verified application_accepted stops the workflow. Do not post after selected/unknown upload. A verified upload proves visible app acceptance only, not persistence. Measure elapsed time from first call to independently verified result. Release the session without closing user tabs.")
    }
}

pub(super) async fn run() -> Result<(), String> {
    let state_dir = super::super::state_directory();
    std::fs::create_dir_all(&state_dir).map_err(|error| error.to_string())?;
    let _state_lock = super::super::acquire_state_directory_lock(&state_dir)
        .map_err(|error| error.to_string())?;
    let journal = crate::jobs::Journal::open(state_dir.join("operations.sqlite"))
        .map_err(|error| error.to_string())?;
    journal
        .recover_after_restart()
        .map_err(|error| error.to_string())?;
    let legacy = super::super::App::with_journal(journal);
    let app = AppV3::new(AppV2::new(legacy));
    let service = app
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|error| error.to_string())?;
    service.waiting().await.map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod v3_batch_delivery_tests {
    use super::{
        AppV3, BrowserPredicate, V3WorkflowStep, V3WorkflowOp, SHARED_TRIGGER_UPLOAD_DEADLINE, cdp_character_events, normalize_key_text, fast_key_batch_failure, lease_target_key,
        page_tool_completion, page_tool_snapshot_fresh, page_tool_unavailable, task_receipt,
        typing_policy, validate_workflow_steps, workflow_find_reference, workflow_should_stop,
        workflow_should_stop_before_next, workflow_should_verify_next,
        compact_workflow_receipt, shared_extension_value, shared_file_input_baseline, upload_acceptance_transition, upload_input_selection_confirmed, upload_picker_consumed_input, upload_selection_reason_code, upload_trigger_may_have_dispatched,
        action_timeout_receipt, focus_changed_during_operation, identity_marker_matches, page_navigate_error,
        virtual_item_identity, linked_date_frontier, unresolved_linked_date_count, choose_newest_with_bounds,
        compile_fill_runs, compile_workflow_ops, validate_workflow_ops, workflow_step_name,
        wait_for_chooser_or_cleanup,
        FileChooserCleanupGuard,
        workflow_upload_requires_stop,
        decode_artifact_bytes, encode_artifact_read,
        fill_form_known_pre_dispatch_failure,
    };
    use serde_json::json;
    use serde_json::Value;
    use std::time::Duration;

    #[test]
    fn fill_form_only_reports_explicit_pre_dispatch_guards_as_not_dispatched() {
        let guarded = json!({"result":{"result":{"value":{"ok":false,"reason":"stale_value"}}}});
        let readback_mismatch = json!({"result":{"result":{"value":{"ok":false,"reason":"readback_mismatch"}}}});
        let missing_reason = json!({"result":{"value":{"ok":false}}});
        assert!(fill_form_known_pre_dispatch_failure(&guarded));
        assert!(!fill_form_known_pre_dispatch_failure(&readback_mismatch));
        assert!(!fill_form_known_pre_dispatch_failure(&missing_reason));
    }

    #[test]
    fn virtual_list_identity_uses_stable_fields_and_ignores_empty_identity() {
        let fields = vec!["href".to_owned(), "name".to_owned()];
        let first = json!({"href":"/story/1","name":"Story"});
        let recycled = json!({"href":"/story/1","name":"Story"});
        let second = json!({"href":"/story/2","name":"Story"});
        assert_eq!(virtual_item_identity(&first, &fields), virtual_item_identity(&recycled, &fields));
        assert_ne!(virtual_item_identity(&first, &fields), virtual_item_identity(&second, &fields));
        assert_eq!(virtual_item_identity(&json!({"href":"","name":""}), &fields), None);
    }

    #[test]
    fn v3_workflow_program_routes_control_flow_through_the_bounded_vm() {
        let program: Vec<V3WorkflowOp> = serde_json::from_value(json!([
            {"op":"if","condition":"true","then_branch":[{"op":"step","step":{"kind":"snapshot"}}],"else_branch":[]},
            {"op":"foreach","items":["a","b"],"var":"item","body":[{"op":"step","step":{"kind":"extract","selector":"main","fields":null}}]},
            {"op":"try_fallback","primary":[{"op":"step","step":{"kind":"find","query":"Continue","save_as":"continue"}}],"fallback":[]}
        ])).unwrap();
        validate_workflow_ops(&program, 0).unwrap();
        let compiled = compile_workflow_ops(program).unwrap();
        assert!(matches!(compiled[0], crate::workflow_vm::VmOp::If { .. }));
        assert!(matches!(compiled[1], crate::workflow_vm::VmOp::Foreach { .. }));
        assert!(matches!(compiled[2], crate::workflow_vm::VmOp::TryFallback { .. }));
    }

    #[test]
    fn public_workflow_schema_does_not_accept_unsupported_parallel_read() {
        let program = serde_json::from_value::<Vec<V3WorkflowOp>>(json!([
            {"op":"parallel_read","operations":[{"op":"step","step":{"kind":"snapshot"}}]}
        ]));
        assert!(program.is_err());
    }

    #[test]
    fn workflow_snapshot_accepts_bounded_goal_fields() {
        let step: V3WorkflowStep = serde_json::from_value(json!({
            "kind":"snapshot",
            "goal":{"query":"Open requested panel","roles":["button"],"exhaustive":false},
            "max_scan_nodes":4096,
            "max_text_chars":0,
            "max_items":1
        })).unwrap();
        assert!(matches!(step, V3WorkflowStep::Snapshot { max_scan_nodes: Some(4096), max_text_chars: Some(0), .. }));
    }

    #[test]
    fn linked_date_enrichment_proves_small_frontier_winners_and_reports_large_incomplete_feeds() {
        let mut items = (0..50).map(|index| json!({
            "reference":format!("@c{index}"), "href":format!("/story/{index}"),
            "datetime_utc_ms":if index < 3 { Value::Null } else { json!(index) }
        })).collect::<Vec<_>>();
        let frontier = linked_date_frontier(&items, 50);
        assert_eq!(frontier.len(), 3, "only three unresolved links need metadata reads");
        for (index, item) in items.iter_mut().take(3).enumerate() { item["datetime_utc_ms"] = json!(1 + index as i64); }
        assert_eq!(unresolved_linked_date_count(&items), 0);
        assert_eq!(crate::semantic_state::choose_newest(&items, true).unwrap()["selected"]["reference"], "@c49");

        let mut unresolved = (0..50).rev().map(|index| json!({
            "href":format!("/unknown/{index}"), "datetime_utc_ms":Value::Null,
            "collection_order":index
        })).collect::<Vec<_>>();
        let frontier = linked_date_frontier(&unresolved, 50);
        assert_eq!(frontier.len(), 12, "one metadata operation remains bounded");
        assert_eq!(frontier[0]["collection_order"],0,"enrich in observed feed order, not map-key order");
        assert_eq!(frontier[11]["collection_order"],11);
        for item in unresolved.iter_mut().filter(|item| item["collection_order"].as_u64().is_some_and(|order| order < frontier.len() as u64)) { item["datetime_utc_ms"] = json!(1); }
        assert_eq!(unresolved_linked_date_count(&unresolved), 38);
        assert!(crate::semantic_state::choose_newest(&unresolved, true).is_err(), "must not claim a winner while 38 timestamps remain unknown");
        assert!(linked_date_frontier(&unresolved, 0).is_empty());
    }

    #[test]
    fn newest_frontier_reads_highest_upper_bounds_and_stops_only_on_proof() {
        let mut items = (0..50).map(|index| json!({
            "reference":format!("@c{index}"),"href":format!("/story/{index}"),
            "datetime_utc_ms":if index == 0 {json!(1000)} else {Value::Null},
            "datetime_precision":if index == 0 {"instant"} else {"date"},
            "freshness_upper_bound_ms":if index == 0 {Value::Null} else {json!(index)} ,
            "collection_order":index
        })).collect::<Vec<_>>();
        let frontier = linked_date_frontier(&items, 12);
        assert_eq!(frontier[0]["collection_order"], 49, "inspect the strongest unresolved upper bounds first");
        let choice = choose_newest_with_bounds(&items, true).unwrap();
        assert_eq!(choice["selected"]["reference"], "@c0");
        assert_eq!(choice["freshness_evidence"]["basis"], "listing_timestamp_beats_all_unchecked_upper_bounds");
        items[49]["freshness_upper_bound_ms"] = Value::Null;
        assert!(choose_newest_with_bounds(&items, true).is_none(), "one unchecked candidate without a bound blocks proof");
    }

    #[test]
    fn date_only_freshness_bound_covers_utc_minus_twelve() {
        let items = vec![
            json!({"href":"/date-only","datetime_precision":"date","datetime_utc_ms":null,
                "freshness_upper_bound_ms":1_712_059_199_999_i64,"collection_order":0}),
            json!({"href":"/exact","datetime_precision":"instant","datetime_utc_ms":1_712_052_000_000_i64,
                "freshness_upper_bound_ms":null,"collection_order":1}),
        ];
        assert!(choose_newest_with_bounds(&items, true).is_none(), "an exact instant before the UTC-12 end of the listing date cannot prove newest");
    }

    #[test]
    fn virtual_list_workflow_is_strictly_bounded() {
        let step = V3WorkflowStep::CollectVirtual {
            container: None, query: None, role: None,
            identity_fields: vec!["href".into()], max_scrolls: 21, max_items: 100,
            save_as: "items".into(),
        };
        assert!(validate_workflow_steps(&[step]).is_err());
    }

    #[test]
    fn base64_artifact_decodes_compact_inline_payload_and_rejects_ambiguous_sources() {
        use base64::Engine;
        let original = vec![0, 1, 2, 127, 128, 255];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&original);
        assert!(encoded.len() < original.iter().map(|byte| byte.to_string().len() + 1).sum::<usize>());
        assert_eq!(decode_artifact_bytes(None, Some(encoded)).unwrap(), Some(original));
        assert!(decode_artifact_bytes(Some(vec![1]), Some("AQ==".into())).is_err());
        assert!(decode_artifact_bytes(None, Some("%%%".into())).is_err());
        let compact = encode_artifact_read(b"upload payload", "base64").unwrap();
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(compact["bytes_base64"].as_str().unwrap()).unwrap(), b"upload payload");
        assert!(compact.get("bytes").is_none());
        assert_eq!(encode_artifact_read(b"upload payload", "bytes").unwrap()["bytes"], json!(b"upload payload".to_vec()));
        assert!(encode_artifact_read(b"x", "invalid").is_err());
    }

    #[test]
    fn four_and_ten_megabyte_base64_payloads_preserve_digest_with_less_json() {
        use base64::Engine;
        for size in [4 * 1024 * 1024, 10 * 1024 * 1024] {
            let original = vec![0x5a; size];
            let encoded = base64::engine::general_purpose::STANDARD.encode(&original);
            let integer_array = serde_json::to_vec(&original).unwrap();
            assert!(encoded.len() < integer_array.len() / 2, "base64 transport should use less than half the JSON integer-array bytes for {size} bytes");
            let decoded = decode_artifact_bytes(None, Some(encoded)).unwrap().unwrap();
            assert_eq!(crate::artifacts::ArtifactExpectation::sha256(&decoded), crate::artifacts::ArtifactExpectation::sha256(&original));
        }
    }

    #[test]
    fn unknown_upload_receipt_preserves_the_failed_verification_dimension() {
        let receipt = task_receipt("upload", "https://classroom.google.com/c/test", None, &json!({
            "status":"unknown","reason_code":"file_selection_unverified",
            "reason":"file readback did not match","chrome_selected_file":false,
            "root_document_matches":true,"account_marker_matches":true,
            "dispatch_acknowledged":true,
            "network_events":[{"host":"www.googleapis.com","status":403}],
            "network_observation_window_ms":2500
        }));
        assert_eq!(receipt["status"], "unknown");
        assert_eq!(receipt["reason_code"], "file_selection_unverified");
        assert_eq!(receipt["chrome_selected_file"], false);
        assert_eq!(receipt["root_document_matches"], true);
        assert_eq!(receipt["account_marker_matches"], true);
        assert_eq!(receipt["dispatch_acknowledged"], true);
        assert_eq!(receipt["network_events"][0]["status"], 403);
        assert_eq!(receipt["network_observation_window_ms"], 2500);
    }

    #[test]
    fn upload_workflow_stops_without_visible_application_acceptance() {
        assert!(workflow_upload_requires_stop(&json!({"status":"selected","application_accepted":null})));
        assert!(workflow_upload_requires_stop(&json!({"status":"unknown","application_accepted":true})));
        assert!(!workflow_upload_requires_stop(&json!({"status":"verified","application_accepted":true})));
    }

    #[test]
    fn workflow_upload_schema_uses_explicit_artifact_trigger_and_acceptance() {
        let step: V3WorkflowStep = serde_json::from_value(json!({
            "kind":"upload",
            "artifact_handle":"opaque-handle",
            "trigger_reference":"@c7",
            "account_marker":["#account","fixture-account"],
            "acceptance":{"kind":"value","reference":"@c8","equals":"Accepted"}
        })).unwrap();
        assert!(matches!(step, V3WorkflowStep::Upload { .. }));
    }

    #[test]
    fn workflow_rejects_unbounded_upload_references_and_account_markers() {
        let step: V3WorkflowStep = serde_json::from_value(json!({
            "kind":"upload",
            "artifact_handle":"opaque-handle",
            "trigger_reference":"@c7",
            "account_marker":["#account","x".repeat(513)]
        })).unwrap();
        assert!(validate_workflow_steps(&[step]).is_err());
    }

    #[test]
    fn shared_trigger_upload_has_a_hard_subminute_deadline() {
        assert!(SHARED_TRIGGER_UPLOAD_DEADLINE < Duration::from_secs(60));
        assert_eq!(SHARED_TRIGGER_UPLOAD_DEADLINE, Duration::from_secs(45));
    }

    #[test]
    fn identity_marker_accepts_the_extension_reply_shape_and_cdp_test_envelope() {
        assert!(identity_marker_matches(&json!({"matches":true,"match_count":1})).unwrap());
        assert!(!identity_marker_matches(&json!({"matches":false,"match_count":0})).unwrap());
        assert!(identity_marker_matches(&json!({"result":{"value":{"matches":true,"match_count":1}}})).unwrap());
        assert!(!identity_marker_matches(&json!({"matches":true,"match_count":2})).unwrap());
        assert!(identity_marker_matches(&json!({"unrelated":true})).is_err());
    }


    #[test]
    fn shared_extension_results_accept_direct_objects_and_cdp_value_envelopes() {
        let direct = json!({"inputs": [], "document_id": "doc-1"});
        assert_eq!(shared_extension_value(&direct).unwrap(), direct);
        let fast_type = json!({"status":"verified","completed_chars":3,"key_events":9,"cdp_commands":15,"readback":"abc"});
        assert_eq!(shared_extension_value(&fast_type).unwrap(), fast_type);
        assert_eq!(
            shared_extension_value(&json!({"result":{"type":"object","value":{"files":[]}}})).unwrap(),
            json!({"files":[]})
        );
        assert!(shared_extension_value(&json!({"exceptionDetails":{}})).is_err());
        assert!(shared_extension_value(&json!(null)).is_err());
    }

    #[test]
    fn file_input_baseline_preserves_diagnostics_and_requires_a_bounded_array() {
        assert_eq!(
            shared_file_input_baseline(&json!({"inputs": [{"backend_node_id": 1}]})).unwrap(),
            json!([{"backend_node_id": 1}])
        );
        assert!(shared_file_input_baseline(&json!({"error":"DOM unavailable"})).is_err());
        assert!(shared_file_input_baseline(&json!({"inputs": "unavailable"})).is_err());
        assert!(shared_file_input_baseline(&json!({"inputs": vec![Value::Null; 257]})).is_err());
    }


    #[test]
    fn compact_surface_routes_tasks_and_receipts_do_not_dump_page_state() {
        let tools = AppV3::tool_router().list_all();
        assert_eq!(tools.len(), 6);
        assert!(tools.iter().all(|tool| {
            ["browser", "snapshot", "act", "workflow", "extract", "verify"]
                .contains(&tool.name.as_ref())
        }));
        assert!(tools.iter().all(|tool| {
            tool.output_schema
                .as_ref()
                .and_then(|schema| schema.get("type"))
                == Some(&serde_json::Value::String("object".into()))
        }));
        let receipt = task_receipt(
            "extract",
            "https://example.test/a",
            None,
            &json!({"items":[{"text":"x".repeat(5000),"secret":"do not return"}],"truncated":false,"omissions":[]}),
        );
        assert_eq!(receipt["verified"], true);
        assert_eq!(receipt["snippet"].as_str().unwrap().chars().count(), 2000);
        assert!(!receipt.to_string().contains("do not return"));
        assert_eq!(receipt["url"], "https://example.test/a");
    }

    #[tokio::test]
    async fn v3_actions_require_a_session_created_through_the_v3_browser_route() {
        let app = AppV3::new(super::super::AppV2::new(super::super::App::default()));
        assert!(app.require_v3_session("legacy-session").await.is_err());
    }

    #[test]
    fn upload_acceptance_requires_a_newly_satisfied_predicate() {
        assert!(!upload_acceptance_transition(true, true));
        assert!(!upload_acceptance_transition(false, false));
        assert!(upload_acceptance_transition(false, true));
    }

    #[test]
    fn consumed_picker_input_is_reported_as_accepted_only_after_app_confirmation() {
        let consumed = json!({"status":"unknown","dispatch_acknowledged":true,"reason":"Error: File input did not retain one selected file for readback."});
        assert!(upload_picker_consumed_input(&consumed));
        assert!(!upload_picker_consumed_input(&json!({"status":"unknown","dispatch_acknowledged":true,"reason":"frame changed during upload"})));
        assert!(!upload_picker_consumed_input(&json!({"status":"unknown","dispatch_acknowledged":false,"reason":"File input did not retain one selected file for readback."})));
        assert!(upload_input_selection_confirmed(true, true, true, true));
        assert!(!upload_input_selection_confirmed(true, true, true, false));
        assert!(!upload_input_selection_confirmed(true, false, true, true));
        assert!(!upload_input_selection_confirmed(true, true, false, true));
        assert_eq!(upload_selection_reason_code(&json!("unknown")), "file_selection_unverified");
        assert_eq!(upload_selection_reason_code(&json!("not_dispatched")), "frame_upload_failed");
    }

    #[test]
    fn upload_receipt_explains_app_accepted_file_when_picker_readback_was_consumed() {
        let receipt=task_receipt("upload","https://classroom.google.com/c/test",None,&json!({
            "status":"verified","verified":true,"chrome_selected_file":true,
            "selection_readback_verified":false,"file_input_consumed_by_page":true,
            "application_accepted":true,"persisted":null,
            "reason":"The picker consumed the file input; the requested application acceptance was observed"
        }));
        assert_eq!(receipt["status"],"verified");
        assert_eq!(receipt["verified"],true);
        assert_eq!(receipt["chrome_selected_file"],true);
        assert_eq!(receipt["selection_readback_verified"],false);
        assert_eq!(receipt["file_input_consumed_by_page"],true);
        assert_eq!(receipt["application_accepted"],true);
        assert!(receipt["snippet"].as_str().unwrap().contains("application acceptance was observed"));
    }

    #[tokio::test]
    async fn failed_chooser_wait_attempts_interception_cleanup() {
        let disabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cleanup_state = disabled.clone();
        let result: Result<(), &str> = wait_for_chooser_or_cleanup(
            async { Err("connection unavailable") },
            async move { cleanup_state.store(true, std::sync::atomic::Ordering::SeqCst); },
        ).await;
        assert_eq!(result, Err("connection unavailable"));
        assert!(disabled.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn failed_disable_keeps_upload_cleanup_guard_armed() {
        let scheduled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let scheduled_on_drop = scheduled.clone();
        let attempts_on_disable = attempts.clone();
        let mut guard = FileChooserCleanupGuard::new(move || {
            scheduled_on_drop.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        guard.disable(async move {
            attempts_on_disable.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        }).await;
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(guard);
        assert!(scheduled.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn acknowledged_disable_disarms_upload_cleanup_guard() {
        let scheduled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let scheduled_on_drop = scheduled.clone();
        let mut guard = FileChooserCleanupGuard::new(move || {
            scheduled_on_drop.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        guard.disable(async { true }).await;
        drop(guard);
        assert!(!scheduled.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn refused_upload_trigger_is_known_not_dispatched() {
        assert!(!upload_trigger_may_have_dispatched(&json!({"status":"not_dispatched"})));
        assert!(upload_trigger_may_have_dispatched(&json!({"status":"verified"})));
        assert!(upload_trigger_may_have_dispatched(&json!({"status":"unknown"})));
    }

    #[test]
    fn a_late_guard_failure_is_unknown_after_keys_were_sent() {
        assert_eq!(
            fast_key_batch_failure(
                &json!({"batch_error":"Batch read guard rejected","completed":1,"stopped_before":1,"receipts":[{"method":"Runtime.callFunctionOn"}]}),
                0,
            ),
            ("not_dispatched", false, false)
        );
        assert_eq!(
            fast_key_batch_failure(
                &json!({"batch_error":"Input.dispatchKeyEvent failed","completed":1,"failed_at":1,"dispatch_may_have_occurred":true,"receipts":[{"method":"Runtime.callFunctionOn"}]}),
                0,
            ),
            ("unknown", false, true)
        );
        assert_eq!(
            fast_key_batch_failure(
                &json!({"batch_error":"Input.dispatchKeyEvent failed","completed":2,"failed_at":2,"dispatch_may_have_occurred":true,"receipts":[{"method":"Runtime.callFunctionOn"},{"method":"Input.dispatchKeyEvent"}]}),
                3,
            ),
            ("unknown", true, true)
        );
        assert_eq!(
            fast_key_batch_failure(
                &json!({"batch_error":"Input.dispatchKeyEvent failed","completed":3,"failed_at":3,"dispatch_may_have_occurred":true,"receipts":[{"method":"Runtime.callFunctionOn"},{"method":"Input.dispatchKeyEvent"},{"method":"Input.dispatchKeyEvent"}]}),
                6,
            ),
            ("unknown", true, true)
        );
    }

    #[test]
    fn workflow_stops_after_uncertain_or_undispatched_actions() {
        assert!(workflow_should_stop(&json!({"status":"unknown"})));
        assert!(workflow_should_stop(&json!({"status":"not_dispatched"})));
        assert!(workflow_should_stop(&json!({"status":"failed"})));
        assert!(!workflow_should_stop(&json!({"status":"verified"})));
    }

    #[test]
    fn unrestricted_workflow_reports_failed_selection_before_resolving_its_reference() {
        assert!(workflow_should_stop(&json!({"status":"failed","reason_code":"missing_freshness"})));
    }

    #[test]
    fn uncertain_click_can_continue_to_read_only_verification() {
        let next = V3WorkflowStep::Verify { predicate: BrowserPredicate::Url { equals: "https://example.test/ok".into() } };
        assert!(!workflow_should_stop_before_next(&json!({"status":"unknown"}), Some(&next)));
        assert!(workflow_should_verify_next(&json!({"status":"dispatched"}), Some(&next)));
        assert!(workflow_should_stop_before_next(&json!({"status":"unknown"}), None));
        assert!(workflow_should_stop_before_next(&json!({"status":"failed"}), Some(&next)));
    }

    #[test]
    fn uncertain_post_mutation_stops_before_another_mutation() {
        let post = V3WorkflowStep::Click {
            reference: "@c7".into(),
            outcome: None,
        };
        assert!(workflow_should_stop_before_next(&json!({"status":"unknown"}), Some(&post)));
    }

    #[test]
    fn combined_workflow_receipts_omit_redundant_page_dumps() {
        let snapshot = json!({"url":"https://example.test/","items":[{"reference":"@c1"}],"text":"large","delta":{"upsert":[]}});
        let compact = compact_workflow_receipt(snapshot.clone(), true, false);
        assert_eq!(compact["item_count"], 1);
        assert!(compact.get("items").is_none());
        assert_eq!(compact_workflow_receipt(snapshot.clone(), false, false), snapshot);
        let extract = json!({"records":[{"title":"long page content"}],"unique_count":1,"completeness":"complete","shared_frame_identity":{"loader_id":"l1"}});
        let compact_extract = compact_workflow_receipt(extract, true, false);
        assert_eq!(compact_extract["item_count"], 1);
        assert!(compact_extract.get("records").is_none());
        assert_eq!(compact_extract["unique_count"], 1);
        assert_eq!(compact_extract["completeness"], "complete");
        assert!(compact_extract["evidence_hash"].as_str().is_some());
        let collected = compact_workflow_receipt(json!({"status":"collected","matches":[{"reference":"@c1"}],"count":1,"coverage_complete":true}), true, false);
        assert!(collected.get("matches").is_none());
        assert_eq!(collected["item_count"], 1);
        assert_eq!(collected["coverage_complete"], true);
        assert!(collected["evidence_hash"].as_str().is_some());
        let click = compact_workflow_receipt(json!({"status":"dispatched","snapshot":snapshot}), false, true);
        assert!(click.get("snapshot").is_none());
    }

    #[test]
    fn workflow_compiles_only_adjacent_bounded_fill_runs_into_one_program() {
        let fill = |reference: &str| V3WorkflowStep::Fill {
            reference: reference.into(), expected_value: String::new(), value: "x".into(),
        };
        let select = |reference: &str| V3WorkflowStep::Select {
            reference: reference.into(), expected_value: "old".into(), value: "new".into(),
        };
        let barrier = V3WorkflowStep::Snapshot { href_hint: None, max_items: None, max_text_chars: None, goal: None, max_scan_nodes: None };
        let compiled = compile_fill_runs(vec![fill("@c1"), select("@c2"), barrier.clone(), fill("@c3")]);
        assert!(matches!(&compiled[0], V3WorkflowStep::FillForm { fields } if fields.len() == 2 && fields[1].select));
        assert_eq!(workflow_step_name(&compiled[1]), "snapshot");
        assert_eq!(workflow_step_name(&compiled[2]), "fill");
        let overflow = compile_fill_runs((0..17).map(|index| if index % 2 == 0 { fill(&format!("@c{index}")) } else { select(&format!("@c{index}")) }).collect());
        assert!(matches!(&overflow[0], V3WorkflowStep::FillForm { fields } if fields.len() == 16));
        assert_eq!(overflow.len(), 2);
    }


    #[test]
    fn workflow_find_rejects_ambiguous_targets() {
        let ambiguous = json!({"matches":[{"reference":"@c1"},{"reference":"@c2"}]});
        assert!(workflow_find_reference(&ambiguous).is_err());
        let unique = json!({"matches":[{"reference":"@c1"}]});
        assert_eq!(workflow_find_reference(&unique).unwrap(), "@c1");
    }

    #[test]
    fn workflow_requires_runtime_verification_after_unverified_effects() {
        let page_tool = V3WorkflowStep::PageTool {
            name: "submit".into(),
            input: json!({}),
        };
        let fill = V3WorkflowStep::Fill {
            reference: "@c1".into(),
            expected_value: "".into(),
            value: "x".into(),
        };
        assert!(validate_workflow_steps(&[page_tool, fill]).is_err());
        let page_tool = V3WorkflowStep::PageTool {
            name: "submit".into(),
            input: json!({}),
        };
        let verify = V3WorkflowStep::Verify {
            predicate: BrowserPredicate::Url {
                equals: "https://example.test/ok".into(),
            },
        };
        assert!(validate_workflow_steps(&[page_tool, verify]).is_ok());
        let press = V3WorkflowStep::Press {
            reference: "@c1".into(),
            key: "Enter".into(),
        };
        let fill = V3WorkflowStep::Fill {
            reference: "@c1".into(),
            expected_value: "".into(),
            value: "x".into(),
        };
        assert!(validate_workflow_steps(&[press, fill]).is_err());
        let press = V3WorkflowStep::Press {
            reference: "@c1".into(),
            key: "Enter".into(),
        };
        let verify = V3WorkflowStep::Verify {
            predicate: BrowserPredicate::Url {
                equals: "https://example.test/ok".into(),
            },
        };
        assert!(validate_workflow_steps(&[press, verify]).is_ok());
    }

    #[test]
    fn page_tool_runtime_exceptions_are_unknown_mutation_delivery() {
        let failure = page_tool_completion(Err("page evaluation failed".into())).unwrap_err();
        assert_eq!(failure["status"], "unknown");
        assert_eq!(failure["dispatch_acknowledged"], true);
        assert!(workflow_should_stop(&failure));
    }

    #[test]
    fn page_tool_requires_an_unconsumed_matching_snapshot() {
        assert!(page_tool_snapshot_fresh(false, true));
        assert!(!page_tool_snapshot_fresh(true, true));
        assert!(!page_tool_snapshot_fresh(false, false));
    }

    #[test]
    fn unavailable_page_tool_stops_workflow_for_ui_fallback() {
        let unavailable = page_tool_unavailable("page tools are not exposed");
        assert_eq!(unavailable["status"], "not_dispatched");
        assert_eq!(unavailable["fallback"], "semantic_ui");
        assert!(workflow_should_stop(&unavailable));
    }

    #[test]
    fn session_aliases_share_the_same_selected_chrome_tab_lease() {
        assert_eq!(
            lease_target_key("session-a", "17"),
            lease_target_key("session-b", "17")
        );
        assert_ne!(
            lease_target_key("session-a", "17"),
            lease_target_key("session-a", "18")
        );
    }

    #[test]
    fn typing_policy_uses_required_events_and_composition_by_default_but_honors_explicit_mode() {
        use crate::v3::TypingMode;
        assert_eq!(
            typing_policy(None, None, "hello", true).unwrap().mode,
            TypingMode::FastKeys
        );
        assert_eq!(
            typing_policy(None, None, "hello", false).unwrap().mode,
            TypingMode::Block
        );
        assert_eq!(
            typing_policy(None, None, "你好", false).unwrap().mode,
            TypingMode::Ime
        );
        assert_eq!(
            typing_policy(Some("block"), None, "hello", true)
                .unwrap()
                .mode,
            TypingMode::Block
        );
        assert!(typing_policy(Some("fast_keys"), None, "你好", true).is_ok());
        assert!(typing_policy(Some("unknown"), None, "hello", false).is_err());
    }

    #[test]
    fn fast_and_human_newlines_dispatch_enter_and_advance_readback() {
        let events = cdp_character_events('\n');
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0["key"], "Enter");
        assert_eq!(events[0].0["code"], "Enter");
        assert_eq!(events[0].0["windowsVirtualKeyCode"], 13);
        assert!(events[0].1);
        assert_eq!(events[1].0["type"], "keyUp");
        assert!(!events[1].1);
        assert_eq!(cdp_character_events('a').len(), 3);
        assert_eq!(normalize_key_text("one\r\n\r\ntwo"), "one\n\ntwo");
    }

    #[test]
    fn focus_change_reports_transient_events_without_claiming_who_caused_them() {
        let before = json!({"tab_active":false,"window_focused":false,"activation_epoch":4,"focus_epoch":8});
        let after = json!({"tab_active":false,"window_focused":false,"activation_epoch":6,"focus_epoch":10});
        assert_eq!(focus_changed_during_operation(&before, &after), Value::Bool(true));
        assert_eq!(focus_changed_during_operation(&json!({"tab_active":false,"window_focused":false}), &json!({"tab_active":false,"window_focused":false})), json!("unknown"));
        assert_eq!(focus_changed_during_operation(&before, &json!({"tab_active":true,"window_focused":false})), Value::Bool(true));
    }

    #[test]
    fn action_timeout_does_not_claim_that_dispatch_was_not_acknowledged() {
        let receipt = action_timeout_receipt("type");
        assert_eq!(receipt["status"], "unknown");
        assert!(receipt["dispatch_acknowledged"].is_null());
        assert_eq!(receipt["dispatch_may_have_occurred"], true);
    }

    #[test]
    fn page_navigate_error_text_is_exposed_as_a_failed_command_result() {
        assert_eq!(page_navigate_error(&json!({"errorText":"net::ERR_NAME_NOT_RESOLVED"})), Some("net::ERR_NAME_NOT_RESOLVED"));
        assert_eq!(page_navigate_error(&json!({"result":{"errorText":"blocked"}})), Some("blocked"));
        assert_eq!(page_navigate_error(&json!({"frameId":"main"})), None);
    }

    #[test]
    fn headless_mode_rejects_shared_provider_instead_of_falling_back() {
        std::thread::Builder::new()
            .name("controlla-v3-provider-test".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        use rmcp::{ServiceExt, model::CallToolRequestParams};
                        let app = AppV3::new(super::super::AppV2::new(super::super::App::default()));
                        let (server_io, client_io) = tokio::io::duplex(65536);
                        let server = rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(app, server_io, None);
                        let server_task = tokio::spawn(async move { let _ = server.waiting().await; });
                        let client = ().serve(client_io).await.unwrap();
                        let params = CallToolRequestParams::new("browser".to_owned()).with_arguments(json!({
                            "action":"pair_shared", "mode":"headless", "provider":"shared_extension", "target_ids":["123"]
                        }).as_object().unwrap().clone());
                        let result = client.call_tool(params).await;
                        assert!(result.is_err() || result.is_ok_and(|result| result.is_error.unwrap_or(false)),
                            "headless must never silently attach to shared Chrome");
                        let _ = client.cancel().await;
                        server_task.abort();
                    });
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn foreground_and_background_pairing_return_the_selected_mode_and_enforce_it_on_release() {
        std::thread::Builder::new()
            .name("controlla-v3-pairing-test".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
        use rmcp::{ServiceExt, model::CallToolRequestParams};
        let app = AppV3::new(super::super::AppV2::new(super::super::App::default()));
        let (server_io, client_io) = tokio::io::duplex(65536);
        let server =
            rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(app, server_io, None);
        let server_task = tokio::spawn(async move {
            let _ = server.waiting().await;
        });
        let client = ().serve(client_io).await.unwrap();
        let call = |value: serde_json::Value| {
            CallToolRequestParams::new("browser".to_owned())
                .with_arguments(value.as_object().unwrap().clone())
        };
        for mode in ["foreground", "background"] {
            let paired = client
                .call_tool(call(
                    json!({"action":"pair_shared","mode":mode,"target_ids":["123"]}),
                ))
                .await
                .unwrap()
                .structured_content
                .unwrap();
            assert_eq!(paired["mode"], mode, "{paired}");
            assert_eq!(paired["provider"], "shared_extension", "{paired}");
            let session_id = paired["session_id"].as_str().unwrap();
            let wrong_mode = if mode == "foreground" {
                "background"
            } else {
                "foreground"
            };
            let wrong_release = client
                .call_tool(call(
                    json!({"action":"release","mode":wrong_mode,"session_id":session_id}),
                ))
                .await;
            assert!(
                wrong_release.is_err()
                    || wrong_release.is_ok_and(|result| result.is_error.unwrap_or(false)),
                "mode mismatch must not release another session"
            );
            let released = client
                .call_tool(call(
                    json!({"action":"release","mode":mode,"session_id":session_id}),
                ))
                .await
                .unwrap()
                .structured_content
                .unwrap();
            assert_eq!(released["released"], true, "{released}");
            assert_eq!(released["mode"], mode, "{released}");
        }
        let _ = client.cancel().await;
        server_task.abort();
                    });
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires installed Chrome; validates shared-bridge FastKeys and visible-trigger file upload"]
    async fn shared_extension_marked_control_defaults_to_verified_trusted_fast_keys() {
        use controlla_browser::{
            providers::DedicatedChromeProvider,
            sessions::{
                CleanupObservation, IndependentTargetObserver, ProviderGrants, SessionMode,
                SessionRegistry, SessionSpec,
            },
        };
        use futures_util::{FutureExt, SinkExt, StreamExt};
        use rmcp::{ServiceExt, model::CallToolRequestParams};
        use std::{panic::AssertUnwindSafe, path::Path};
        use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
        use std::time::Duration;
        use tokio_tungstenite::{connect_async, tungstenite::Message};
        let executable = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(executable.exists());
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..Default::default()
        });
        let handle = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: vec![],
                },
                "v3-fast-keys-fixture",
            )
            .unwrap();
        let browser = DedicatedChromeProvider::new(executable)
            .launch(&mut registry, &handle, "about:blank")
            .await
            .unwrap();
        let connection = browser.connection().clone();
        let target = browser.target_id().to_owned();
        let eval = |script: String| {
            let connection = connection.clone();
            let target = target.clone();
            async move {
                let (_, targets) = connection.target_snapshot().await;
                let target_state = targets.iter().find(|item| item.id == target).unwrap();
                connection
                    .target_command(
                        &target,
                        target_state.generation,
                        &target_state.revision,
                        "Runtime.evaluate",
                        json!({"expression":script,"returnByValue":true}),
                    )
                    .await
                    .unwrap()
            }
        };
        let result=AssertUnwindSafe(async {
            let unrestricted=crate::guard_reason::unrestricted_mode();
            eval(r#"document.body.innerHTML='<span id="account">fixture-account</span><input id="file" type="file" style="display:none"><button id="upload" aria-label="Upload file">Upload file</button><button id="status" aria-label="Upload status">Waiting</button><input id="trusted" aria-label="Trusted keys" type="text" data-requires-trusted value="before"><input id="other" aria-label="Other" type="text">';document.querySelector('#upload').addEventListener('click',()=>document.querySelector('#file').click());document.querySelector('#file').addEventListener('change',()=>setTimeout(()=>document.querySelector('#status').innerText='Accepted',300));window.events=[];window.otherEvents=[];const input=document.querySelector('#trusted');for(const type of ['keydown','beforeinput','input','keyup'])input.addEventListener(type,event=>{window.events.push({type,trusted:event.isTrusted});if(type==='keydown'&&window.blurOnKeydown){window.blurOnKeydown=false;input.blur();document.querySelector('#other').focus();}});document.querySelector('#other').addEventListener('keyup',event=>window.otherEvents.push({type:event.type,key:event.key,trusted:event.isTrusted}));"#.into()).await;
            let (server_io,client_io)=tokio::io::duplex(65536);
            let server=rmcp::service::serve_directly::<rmcp::RoleServer,_,_,_,_>(super::AppV3::new(super::AppV2::new(super::super::App::default())),server_io,None);
            let server_task=tokio::spawn(async move {let _=server.waiting().await;});
            let client=().serve(client_io).await.unwrap();
            let call=|name:&str,value:serde_json::Value| CallToolRequestParams::new(name.to_owned()).with_arguments(value.as_object().unwrap().clone());
            let pair=client.call_tool(call("browser",json!({"action":"pair_shared","target_ids":["123","124","125"]}))).await.unwrap().structured_content.unwrap();
            let session=pair["session_id"].as_str().unwrap().to_owned();
            let (mut socket,_)=connect_async(pair["endpoint"].as_str().unwrap()).await.unwrap();
            socket.send(Message::Text(json!({"type":"hello","token":pair["one_session_token"],"extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"batch_execution":true,"batch_deadline":true,"targets":["123","124","125"]}).to_string().into())).await.unwrap();
            let _=socket.next().await.unwrap();
            let c=connection.clone(); let t=target.clone();
            let chooser_cursor=Arc::new(AtomicU64::new(connection.latest_event_sequence()));
            let proxy_chooser_cursor=chooser_cursor.clone();
            let drop_next_mutation=Arc::new(AtomicBool::new(false));
            let proxy_drop_next_mutation=drop_next_mutation.clone();
            let proxy=tokio::spawn(async move {
                while let Some(Ok(message))=socket.next().await {
                    let request:serde_json::Value=serde_json::from_str(message.to_text().unwrap()).unwrap();
                    if request["type"]=="batch" {
                        let actions=request["actions"].as_array().unwrap();
                        let mut receipts=Vec::new(); let mut error=None; let mut completed=0;
                        for (index,action) in actions.iter().enumerate() {
                            let (_,targets)=c.target_snapshot().await; let state=targets.iter().find(|item|item.id==t).unwrap();
                            match c.target_command(&t,state.generation,&state.revision,action["method"].as_str().unwrap(),action["params"].clone()).await {
                                Ok(result)=>{
                                    let ok=result.pointer("/result/value/ok")==Some(&serde_json::Value::Bool(true));
                                    receipts.push(json!({"index":index,"method":action["method"],"result":result})); completed=index+1;
                                    if action["stop_on_not_ok"]==true&&!ok {error=Some("Batch read guard rejected before the next action.".to_owned());break;}
                                },
                                Err(problem)=>{error=Some(problem.to_string());break;}
                            }
                        }
                        let mut response=json!({"type":"batch_result","id":request["id"],"result":{"receipts":receipts,"completed":completed,"stopped_before":if error.is_some(){Some(completed)}else{None},"host_round_trips":1,"in_browser_actions":actions.len()}});
                        if let Some(problem)=error {response["error"]=json!(problem);}
                        if socket.send(Message::Text(response.to_string().into())).await.is_err(){break;}
                        continue;
                    }
                    let Some(method)=request["method"].as_str() else {continue};
                    let (_,targets)=c.target_snapshot().await; let state=targets.iter().find(|item|item.id==t).unwrap();
                    let response=if method=="Controlla.fastType" {
                        // This proxy stands in for background.js's extension-private command.
                        // Exercise the same guarded CDP path instead of forwarding its method
                        // name to Chrome, which correctly rejects non-CDP commands.
                        let params=&request["params"];
                        let object_id=params["object_id"].as_str().unwrap_or_default();
                        let index=params["index"].as_u64().unwrap_or(u64::MAX);
                        let text=params["text"].as_str().unwrap_or_default();
                        let mut progress=params["expected"].as_str().unwrap_or_default().to_owned();
                        let guard="function(index,expected,prepare){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};const rich=e.isContentEditable, value=()=>rich?(e.innerText||e.textContent||'').replace(/\\r\\n/g,'\\n'):e.value;if(value()!==expected)return {ok:false,reason:'stale_value',value:value()};if(prepare){e.focus();if(rich){const s=e.ownerDocument.getSelection(),r=e.ownerDocument.createRange();r.selectNodeContents(e);r.collapse(false);s.removeAllRanges();s.addRange(r)}else e.setSelectionRange(e.value.length,e.value.length)}const s=e.ownerDocument.getSelection(),focused=e.ownerDocument.activeElement===e,atEnd=rich?s?.isCollapsed&&s.anchorNode&&e.contains(s.anchorNode):e.selectionStart===e.selectionEnd&&e.selectionEnd===e.value.length;return focused&&atEnd?{ok:true,value:value()}:{ok:false,reason:'focus_or_selection_changed',value:value(),focused,atEnd}}";
                        let mut key_events=0usize;
                        let mut completed_chars=0usize;
                        let mut failure=None;
                        let guard_call=|expected:String,prepare:bool| {
                            let c=c.clone(); let t=t.clone(); let object_id=object_id.to_owned(); let guard=guard.to_owned();
                            async move {
                                let (_,targets)=c.target_snapshot().await;
                                let current=targets.iter().find(|item|item.id==t).unwrap();
                                c.target_command(&t,current.generation,&current.revision,"Runtime.callFunctionOn",json!({"objectId":object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":expected},{"value":prepare}],"returnByValue":true})).await
                            }
                        };
                        let initial=guard_call(progress.clone(),true).await;
                        let initial=initial.ok().and_then(|value|value.pointer("/result/value").cloned()).unwrap_or(json!({"ok":false,"reason":"guard_read_failed"}));
                        if initial["ok"]!=true { failure=Some(initial["reason"].as_str().unwrap_or("editor_guard_rejected").to_owned()); }
                        for character in text.chars().take(16384) {
                            if failure.is_some(){break;}
                            let enter=character=='\n';
                            let key=if enter{"Enter".to_owned()}else{character.to_string()};
                            let down=if enter {json!({"type":"keyDown","key":key,"code":"Enter","windowsVirtualKeyCode":13,"nativeVirtualKeyCode":36,"text":"\r","unmodifiedText":"\r"})} else {json!({"type":"keyDown","key":key})};
                            let send=|method:&'static str,params:serde_json::Value| {let c=c.clone();let t=t.clone();async move {let (_,targets)=c.target_snapshot().await;let current=targets.iter().find(|item|item.id==t).unwrap();c.target_command(&t,current.generation,&current.revision,method,params).await}};
                            if send("Input.dispatchKeyEvent",down).await.is_err(){failure=Some("key_down_delivery_unknown".to_owned());break;}
                            key_events+=1;
                            if enter { progress.push(character); }
                            let after_down=guard_call(progress.clone(),false).await.ok().and_then(|value|value.pointer("/result/value").cloned()).unwrap_or(json!({"ok":false,"reason":"guard_read_failed"}));
                            if after_down["ok"]!=true {failure=Some("focus_or_value_changed_after_keydown".to_owned());break;}
                            if !enter {
                                if send("Input.dispatchKeyEvent",json!({"type":"char","key":key,"text":character.to_string(),"unmodifiedText":character.to_string()})).await.is_err(){failure=Some("character_delivery_unknown".to_owned());break;}
                                key_events+=1;
                                progress.push(character);
                                let after_char=guard_call(progress.clone(),false).await.ok().and_then(|value|value.pointer("/result/value").cloned()).unwrap_or(json!({"ok":false,"reason":"guard_read_failed"}));
                                if after_char["ok"]!=true {failure=Some("focus_or_value_changed_after_character".to_owned());break;}
                            }
                            let up=if enter {json!({"type":"keyUp","key":key,"code":"Enter","windowsVirtualKeyCode":13,"nativeVirtualKeyCode":36})} else {json!({"type":"keyUp","key":key})};
                            if send("Input.dispatchKeyEvent",up).await.is_err(){failure=Some("key_up_delivery_unknown".to_owned());break;}
                            key_events+=1;
                            completed_chars+=1;
                        }
                        let final_guard=if failure.is_none(){guard_call(progress.clone(),false).await.ok().and_then(|value|value.pointer("/result/value").cloned()).unwrap_or(json!({"ok":false,"reason":"guard_read_failed"}))}else{json!({"ok":false})};
                        let status=if failure.is_none()&&final_guard["ok"]==true{"verified"}else if key_events==0{"not_dispatched"}else{"unknown"};
                        Ok(json!({"result":{"value":{"status":status,"completed_chars":completed_chars,"key_events":key_events,"cdp_commands":key_events+completed_chars*2+2,"readback":progress,"reason":failure}}}))
                    } else if method=="Controlla.fileInputSnapshot" {
                        Ok(json!({"inputs":[],"document_id":"fixture-document"}))
                    } else if method=="Controlla.waitForFileChooser" {
                        let cursor=proxy_chooser_cursor.load(Ordering::Acquire);
                        let event=async {
                            let timeout=Duration::from_millis(request["params"]["timeout_ms"].as_u64().unwrap_or(8_000).clamp(1,15_000));
                            let deadline=tokio::time::Instant::now()+timeout;
                            let mut sequence=cursor;
                            loop {
                                let remaining=deadline.saturating_duration_since(tokio::time::Instant::now());
                                if remaining.is_zero(){break;}
                                let event=c.next_event_after(sequence,remaining).await.map_err(|error|error.to_string())?;
                                sequence=event.sequence;
                                if event.value["method"]=="Page.fileChooserOpened" { return Ok(event.value); }
                            }
                            Err("timed out waiting for Chrome file chooser event".to_owned())
                        }.await;
                        match event {
                            Ok(event)=>{
                                let frame_id=event.pointer("/params/frameId").and_then(serde_json::Value::as_str).unwrap_or_default();
                                let (generation,revision)= {let (_,targets)=c.target_snapshot().await;let state=targets.iter().find(|item|item.id==t).unwrap();(state.generation,state.revision.clone())};
                                match c.target_command(&t,generation,&revision,"Page.getFrameTree",serde_json::Value::Null).await {
                                    Ok(tree)=>{
                                        fn frame_loader(node:&serde_json::Value,id:&str)->Option<String>{if node.pointer("/frame/id").and_then(serde_json::Value::as_str)==Some(id){return node.pointer("/frame/loaderId").and_then(serde_json::Value::as_str).map(str::to_owned);}node.get("childFrames")?.as_array()?.iter().find_map(|child|frame_loader(child,id))}
                                        let loader=frame_loader(&tree["frameTree"],frame_id).unwrap_or_default();
                                        Ok(json!({"result":{"value":{"backend_node_id":event["params"]["backendNodeId"],"frame_id":frame_id,"frame_loader_id":loader,"document_id":"fixture-document","root_document_id":"fixture-document"}}}))
                                    },Err(error)=>Err(error.to_string())
                                }
                            },Err(error)=>Err(error)
                        }
                    } else if method=="Controlla.setFrameFileInputFiles" {
                        let backend=request["params"]["backend_node_id"].clone();
                        let (generation,revision)={let (_,targets)=c.target_snapshot().await;let state=targets.iter().find(|item|item.id==t).unwrap();(state.generation,state.revision.clone())};
                        let set=c.target_command(&t,generation,&revision,"DOM.setFileInputFiles",json!({"files":[request["params"]["file_path"]],"backendNodeId":backend})).await;
                        match set {
                            Ok(_)=>{
                                let (generation,revision)={let (_,targets)=c.target_snapshot().await;let state=targets.iter().find(|item|item.id==t).unwrap();(state.generation,state.revision.clone())};
                                let resolved=c.target_command(&t,generation,&revision,"DOM.resolveNode",json!({"backendNodeId":backend})).await;
                                match resolved.and_then(|value|value.pointer("/object/objectId").and_then(serde_json::Value::as_str).map(str::to_owned).ok_or_else(||controlla_browser::BrowserError::InvalidResponse("missing file input object".into()))) {
                                    Ok(object_id)=>{
                                        let (generation,revision)={let (_,targets)=c.target_snapshot().await;let state=targets.iter().find(|item|item.id==t).unwrap();(state.generation,state.revision.clone())};
                                        match c.target_command(&t,generation,&revision,"Runtime.callFunctionOn",json!({"objectId":object_id,"functionDeclaration":"function(){return Array.from(this.files||[]).map(f=>({name:f.name,size:f.size}));}","returnByValue":true})).await {
                                            Ok(readback)=>Ok(json!({"result":{"value":{"files":readback.pointer("/result/value").cloned().unwrap_or(json!([]))}}})),Err(error)=>Err(error.to_string())
                                        }
                                    },Err(error)=>Err(error.to_string())
                                }
                            },Err(error)=>Err(error.to_string())
                        }
                    } else if method == "Controlla.checkIdentityMarker" {
                        let selector = request.pointer("/params/selector").and_then(serde_json::Value::as_str).unwrap_or_default();
                        let expected = request.pointer("/params/expected").and_then(serde_json::Value::as_str).unwrap_or_default();
                        match controlla_browser::identity_marker_command(selector, expected) {
                            Ok(params) => match c.target_command(&t,state.generation,&state.revision,"Runtime.evaluate",params).await {
                                Ok(value) => {
                                    let matches = value.pointer("/result/value").and_then(serde_json::Value::as_bool) == Some(true);
                                    Ok(json!({"result":{"value":{"matches":matches,"match_count":if matches {1} else {0},"route":"root_dom"}}}))
                                }
                                Err(error) => Err(error.to_string()),
                            },
                            Err(error) => Err(error.to_string()),
                        }
                    } else { c.target_command(&t,state.generation,&state.revision,method,request["params"].clone()).await.map_err(|error|error.to_string()) };
                    let response=match response {Ok(value)=>json!({"type":"result","id":request["id"],"result":value}),Err(error)=>json!({"type":"result","id":request["id"],"error":error})};
                    if method=="Input.dispatchKeyEvent" && request.pointer("/params/type").and_then(serde_json::Value::as_str)==Some("char") && proxy_drop_next_mutation.swap(false,Ordering::AcqRel) {
                        // Simulate the mutation reaching Chrome while its extension response is lost.
                        let lost=json!({"type":"result","id":request["id"],"error":"simulated acknowledgement loss after dispatch"});
                        if socket.send(Message::Text(lost.to_string().into())).await.is_err(){break;}
                        continue;
                    }
                    if socket.send(Message::Text(response.to_string().into())).await.is_err(){break;}
                }
            });
            let accepted=client.call_tool(call("browser",json!({"action":"accept_shared","session_id":session}))).await.unwrap().structured_content.unwrap();
            assert_eq!(accepted["accepted"],true);
            let snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"123"}))).await.unwrap().structured_content.unwrap();
            let target_ref=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            let clicked=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"123","action":"click","reference":target_ref,"outcome":{"kind":"focused"}}))).await.unwrap().structured_content.unwrap();
            assert_eq!(clicked["status"],"verified","{clicked}");
            let fresh=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"123"}))).await.unwrap().structured_content.unwrap();
            let target_ref=fresh["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            let payload="x".repeat(1000);
            let type_started=std::time::Instant::now();
            let typed=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"123","action":"type","reference":target_ref,"expected_value":"before","value":payload,"timeout_ms":60000}))).await.unwrap().structured_content.unwrap();
            let type_elapsed=type_started.elapsed();
            eprintln!("shared-extension V3 FastKeys 1000-character action: {:.1} ms",type_elapsed.as_secs_f64()*1000.0);
            assert!(type_elapsed<std::time::Duration::from_secs(30),"FastKeys must not impose the former 60ms-per-character floor: {type_elapsed:?}");
            assert_eq!(typed["typing_mode"],"fast_keys","{typed}");
            assert_eq!(typed["status"],"verified","{typed}");
            assert_eq!(typed["readback"],format!("before{payload}"));
            let oversized_snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"123"}))).await.unwrap().structured_content.unwrap();
            let oversized_reference=oversized_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            let oversized="x".repeat(3_334);
            let refused=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"123","action":"type","reference":oversized_reference,"expected_value":format!("before{payload}"),"value":oversized,"typing_mode":"fast_keys"}))).await.unwrap().structured_content.unwrap();
            assert_eq!(refused["status"],"not_dispatched","over-budget FastKeys must be typed as pre-dispatch refusal: {refused}");
            assert_eq!(refused["reason_code"],"workflow_cdp_command_budget_exceeded","{refused}");
            assert_eq!(refused["dispatch_may_have_occurred"],false,"{refused}");
            assert_eq!(refused["metrics"]["budget_stop_reason"],"workflow_cdp_command_budget_exceeded","{refused}");
            let unchanged=eval("document.querySelector('#trusted').value".into()).await["result"]["value"].as_str().unwrap().to_owned();
            assert_eq!(unchanged,format!("before{payload}"),"over-budget FastKeys changed the page: {refused}");
            let events=eval("JSON.stringify({value:document.querySelector('#trusted').value,events:window.events})".into()).await["result"]["value"].as_str().unwrap().to_owned();
            let events:serde_json::Value=serde_json::from_str(&events).unwrap();
            assert_eq!(events["value"],format!("before{payload}"));
            let observed=events["events"].as_array().unwrap();
            assert_eq!(observed.len(),4000);
            assert!(observed.as_chunks::<4>().0.iter().all(|events|events.iter().map(|event|event["type"].as_str().unwrap()).eq(["keydown","beforeinput","input","keyup"])));
            assert!(observed.iter().all(|event|event["trusted"]==true),"untrusted key event: {events}");
            let human_snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"123"}))).await.unwrap().structured_content.unwrap();
            let human_ref=human_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            let human=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"123","action":"type","reference":human_ref,"expected_value":format!("before{payload}"),"value":"h","typing_mode":"human_keys","timeout_ms":60000}))).await.unwrap().structured_content.unwrap();
            assert_eq!(human["status"],"verified","{human}");
            assert_eq!(human["readback"],format!("before{payload}h"));
            let fresh=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"123"}))).await.unwrap().structured_content.unwrap();
            let target_ref=fresh["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            if !unrestricted {
            eval("window.blurOnKeydown=true".into()).await;
            let interrupted=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"123","action":"type","reference":target_ref,"expected_value":format!("before{payload}h"),"value":"Z","timeout_ms":60000}))).await.unwrap().structured_content.unwrap();
            assert_eq!(interrupted["status"],"unknown","{interrupted}");
            let values=eval("JSON.stringify([document.querySelector('#trusted').value,document.querySelector('#other').value])".into()).await["result"]["value"].as_str().unwrap().to_owned();
            assert_eq!(serde_json::from_str::<serde_json::Value>(&values).unwrap(),json!([format!("before{payload}h"),""]));
            let key_events=eval("JSON.stringify({source:window.events.slice(-1),other:window.otherEvents})".into()).await["result"]["value"].as_str().unwrap().to_owned();
            let key_events:serde_json::Value=serde_json::from_str(&key_events).unwrap();
            assert_eq!(key_events["source"][0]["type"],"keydown");
            assert_eq!(key_events["other"],json!([]),"never release a stale key to the newly focused control: {key_events}");

            eval("document.querySelector('#trusted').focus();window.blurOnKeydown=true".into()).await;
            let fresh=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"124"}))).await.unwrap().structured_content.unwrap();
            let target_ref=fresh["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            let interrupted=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"124","action":"type","reference":target_ref,"expected_value":format!("before{payload}h"),"value":"Y","typing_mode":"human_keys","timeout_ms":60000}))).await.unwrap().structured_content.unwrap();
            assert_eq!(interrupted["status"],"unknown","{interrupted}");
            let key_events=eval("JSON.stringify({source:window.events.slice(-1),other:window.otherEvents})".into()).await["result"]["value"].as_str().unwrap().to_owned();
            let key_events:serde_json::Value=serde_json::from_str(&key_events).unwrap();
            assert_eq!(key_events["source"][0]["type"],"keydown");
            assert_eq!(key_events["other"],json!([]),"HumanKeys must also avoid retargeted keyup: {key_events}");
            let recovery_ref=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"125"}))).await.unwrap().structured_content.unwrap()["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            drop_next_mutation.store(true,Ordering::Release);
            let uncertain=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"125","action":"type","reference":recovery_ref,"expected_value":format!("before{payload}h"),"value":"q","typing_mode":"human_keys","delay_ms":0,"timeout_ms":20000}))).await.unwrap().structured_content.unwrap();
            assert_eq!(uncertain["status"],"unknown","{uncertain}");
            let observed=client.call_tool(call("verify",json!({"session_id":session,"chrome_tab_id":"125","predicate":{"kind":"value","reference":recovery_ref,"equals":format!("before{payload}hq")}}))).await.unwrap().structured_content.unwrap();
            assert_eq!(observed["status"],"passed","{observed}");
            }
            if unrestricted {
            let registered=client.call_tool(call("browser",json!({"action":"artifact_register","session_id":session,"filename":"stage1.txt","bytes":[115,116,97,103,101,49]}))).await.unwrap().structured_content.unwrap();
            let upload_snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"123"}))).await.unwrap().structured_content.unwrap();
            let upload_ref=upload_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Upload file").unwrap()["reference"].clone();
            let status_ref=upload_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Upload status").unwrap()["reference"].clone();
            chooser_cursor.store(connection.latest_event_sequence(),Ordering::Release);
            let uploaded=client.call_tool(call("workflow",json!({"session_id":session,"chrome_tab_id":"123","steps":[{"kind":"upload","artifact_handle":registered["artifact_handle"],"trigger_reference":upload_ref,"account_marker":["#account","fixture-account"],"acceptance":{"kind":"value","reference":status_ref,"equals":"Accepted"}}]}))).await.unwrap().structured_content.unwrap();
            assert_eq!(uploaded["status"],"completed","{uploaded}");
            assert_eq!(uploaded["receipts"][0]["status"],"verified","{uploaded}");
            assert_eq!(uploaded["receipts"][0]["chrome_selected_file"],true,"{uploaded}");
            assert_eq!(uploaded["receipts"][0]["application_accepted"],true,"{uploaded}");
            assert_eq!(uploaded["receipts"][0]["filename"],"stage1.txt");
            assert!(uploaded["step_ms"][0].as_u64().is_some(),"{uploaded}");
            }
            proxy.abort(); let _=client.cancel().await; server_task.abort();
        }).catch_unwind().await;
        struct Observer;
        impl IndependentTargetObserver for Observer {
            fn verify_unchanged(&self, _: &CleanupObservation) -> Result<(), String> {
                Ok(())
            }
        }
        let cleanup = browser.shutdown(&mut registry, Some(&Observer)).await;
        assert!(
            cleanup.cleanup_error.is_none(),
            "{:?}",
            cleanup.cleanup_error
        );
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires installed Google Chrome; exercises the complete V3 dedicated headless lifecycle"]
    fn v3_headless_launch_list_snapshot_fill_verify_and_release() {
        std::thread::Builder::new().name("controlla-v3-chrome-test".into()).stack_size(16 * 1024 * 1024).spawn(|| {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        use rmcp::{ServiceExt, model::CallToolRequestParams};
        let executable =
            std::path::Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(executable.is_file());
        let mut app = AppV3::new(super::super::AppV2::new(super::super::App::default()));
        app.headless_chrome = Some(executable.to_path_buf());
        let (server_io, client_io) = tokio::io::duplex(65536);
        let server = rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(
            app.clone(),
            server_io,
            None,
        );
        let server_task = tokio::spawn(async move {
            let _ = server.waiting().await;
        });
        let client = ().serve(client_io).await.unwrap();
        let call = |name: &str, value: serde_json::Value| {
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(value.as_object().unwrap().clone())
        };
        let launch = client.call_tool(call("browser", json!({
            "action":"launch", "mode":"headless", "url":"data:text/html,%3Cinput%20aria-label%3D%22Name%22%20value%3D%22before%22%3E%3Cinput%20aria-label%3D%22Next%22%20value%3D%22before%22%3E%3Cselect%20aria-label%3D%22Country%22%3E%3Coption%20value%3D%22a%22%3EAlpha%3C/option%3E%3Coption%20value%3D%22b%22%3EBeta%3C/option%3E%3C/select%3E"
        }))).await.unwrap().structured_content.unwrap();
        assert_eq!(launch["launched"], true, "{launch}");
        let session_id = launch["session_id"].as_str().unwrap().to_owned();
        let target_id = launch["target_ids"][0].as_str().unwrap().to_owned();
        assert!(app.wait_for_open_readiness(
            &session_id,
            &target_id,
            &super::V3ReadyWhen::SemanticMatch { query: "Name".into(), role: Some("input".into()) },
        ).await.unwrap(), "semantic readiness must observe the target before workflow dispatch");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.querySelector('[aria-label=Name]').remove();setTimeout(()=>document.body.insertAdjacentHTML('afterbegin','<input aria-label=\"Name\" value=\"before\">'),250);true",
                "returnByValue":true
            })).await.unwrap();
        }
        let readiness_started = tokio::time::Instant::now();
        assert!(app.wait_for_open_readiness(
            &session_id,
            &target_id,
            &super::V3ReadyWhen::SemanticMatch { query: "Name".into(), role: Some("input".into()) },
        ).await.unwrap(), "semantic readiness must wait for an exact match instead of accepting Username");
        assert!(readiness_started.elapsed() >= Duration::from_millis(180), "substring-only lookalike incorrectly satisfied semantic readiness early");
        assert!(app.wait_for_open_readiness(
            &session_id,
            &target_id,
            &super::V3ReadyWhen::Selector { selector: "input[aria-label='Name']".into() },
        ).await.unwrap(), "selector readiness must observe the requested element");
        let listed = client
            .call_tool(call(
                "browser",
                json!({"action":"list","mode":"headless","session_id":session_id}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(listed["targets"][0]["chrome_tab_id"], target_id, "{listed}");
        assert_eq!(listed["mode"], "headless", "{listed}");
        let snapshot = client
            .call_tool(call(
                "snapshot",
                json!({"session_id":session_id,"chrome_tab_id":target_id}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let item = snapshot["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Name")
            .unwrap();
        let reference = item["reference"].as_str().unwrap().to_owned();
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"const old=document.querySelector('[aria-label=Name]');old.replaceWith(old.cloneNode(true));true",
                "returnByValue":true
            })).await.unwrap();
        }
        let failed_find=super::find_impl(&app.core,&super::BrowserFindArgs{
            session_id:session_id.clone(),chrome_tab_id:target_id.clone(),query:"no control has this label".into(),
            role:None,limit:Some(10),max_matches:None,match_mode:Some("all".into()),within_reference:None,href_hint:None,
        }).await.unwrap();
        assert_eq!(failed_find["count"],0,"{failed_find}");
        let filled = client.call_tool(call("act", json!({"session_id":session_id,"chrome_tab_id":target_id,"action":"fill","reference":reference,"expected_value":"before","value":"after"}))).await.unwrap().structured_content.unwrap();
        assert_eq!(filled["status"], "verified", "{filled}");
        assert!(filled["snapshot"].is_null(), "a verified field mutation must not return another full-page snapshot: {filled}");
        assert_eq!(filled["delta"]["verified"], true, "the compact affected-control delta is the postcondition: {filled}");
        assert_eq!(filled["reacquisition"]["status"], "rebound", "same-document rerender should use one bounded fresh semantic read before retrying the known-not-dispatched fill: {filled}");
        let workflow = client.call_tool(call("workflow", json!({
            "session_id":session_id,"chrome_tab_id":target_id,
            "steps":[
                {"kind":"snapshot"},
                {"kind":"find","query":"Name","role":"input","save_as":"name"},
                {"kind":"fill","reference":"$name","expected_value":"after","value":"workflow"},
                {"kind":"find","query":"Next","role":"input","save_as":"next"},
                {"kind":"fill","reference":"$next","expected_value":"before","value":"done"}
            ]
        }))).await.unwrap().structured_content.unwrap();
        assert_eq!(workflow["status"], "completed", "a workflow-owned verified fill may advance the retained generation: {workflow}");
        assert_eq!(workflow["receipts"][2]["status"], "verified", "{workflow}");
        assert_eq!(workflow["receipts"][4]["status"], "verified", "{workflow}");
        let vm_program = client.call_tool(call("workflow", json!({
            "session_id":session_id,"chrome_tab_id":target_id,"steps":[],"program":[
                {"op":"step","step":{"kind":"snapshot"}},
                {"op":"step","step":{"kind":"find","query":"Name","role":"input","save_as":"vm_name"}},
                {"op":"if","condition":"var_present:vm_name","then_branch":[{"op":"step","step":{"kind":"snapshot"}}],"else_branch":[]},
                {"op":"foreach","items":["first","second"],"var":"pass","body":[{"op":"step","step":{"kind":"extract","selector":"input[aria-label='Name']","fields":null}}]}
            ]
        }))).await.unwrap().structured_content.unwrap();
        assert_eq!(vm_program["status"], "completed", "public V3 program must execute VM control flow: {vm_program}");
        assert_eq!(vm_program["steps"], 5, "a saved find result drives conditional execution and two foreach reads: {vm_program}");
        let read_fallback = client.call_tool(call("workflow", json!({
            "session_id":session_id,"chrome_tab_id":target_id,"steps":[],"program":[
                {"op":"try_fallback","primary":[{"op":"step","step":{"kind":"extract","selector":"["}}],
                    "fallback":[{"op":"step","step":{"kind":"find","query":"Name","role":"input","save_as":"fallback_name"}}]}
            ]
        }))).await.unwrap().structured_content.unwrap();
        assert_eq!(read_fallback["status"], "completed", "a failed public read should run the bounded read fallback: {read_fallback}");
        assert_eq!(read_fallback["receipts"].as_array().unwrap().len(), 2, "retain both the failed read and fallback evidence: {read_fallback}");
        assert_eq!(read_fallback["receipts"][0]["status"], "not_dispatched");
        assert!(read_fallback["receipts"][1]["matches"][0]["reference"].as_str().is_some());
        let values = client.call_tool(call("snapshot", json!({"session_id":session_id,"chrome_tab_id":target_id}))).await.unwrap().structured_content.unwrap();
        assert_eq!(values["items"].as_array().unwrap().iter().find(|item|item["name"]=="Name").unwrap()["raw_value"], "workflow", "{values}");
        assert_eq!(values["items"].as_array().unwrap().iter().find(|item|item["name"]=="Next").unwrap()["raw_value"], "done", "{values}");
        let name_ref = values["items"].as_array().unwrap().iter().find(|item|item["name"]=="Name").unwrap()["reference"].clone();
        let next_ref = values["items"].as_array().unwrap().iter().find(|item|item["name"]=="Next").unwrap()["reference"].clone();
        let filled_form = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[{"kind":"fill_form","fields":[
            {"reference":name_ref,"expected_value":"workflow","value":"form"},
            {"reference":next_ref,"expected_value":"done","value":"batch"}
        ]}]}))).await.unwrap().structured_content.unwrap();
        assert_eq!(filled_form["status"], "completed", "{filled_form}");
        assert_eq!(filled_form["receipts"][0]["status"], "verified", "{filled_form}");
        assert_eq!(filled_form["receipts"][0]["completed_fields"], 2, "{filled_form}");
        let stale_reuse = super::workflow_fill_form(&app, &super::V3WorkflowArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), steps:vec![], program:None, client_id:None,
        }, vec![super::V3WorkflowFillField { reference:name_ref.as_str().unwrap().into(), expected_value:"form".into(), value:"replayed".into(), select:false }], &std::collections::BTreeMap::new()).await;
        assert!(stale_reuse.is_err(), "a separate workflow must not reuse a consumed snapshot generation");
        let compiled_snapshot = client.call_tool(call("snapshot", json!({"session_id":session_id,"chrome_tab_id":target_id}))).await.unwrap().structured_content.unwrap();
        let compiled_name_ref = compiled_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Name").unwrap()["reference"].clone();
        let compiled_country_ref = compiled_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Country").unwrap()["reference"].clone();
        let compiled_next_ref = compiled_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Next").unwrap()["reference"].clone();
        let mixed_program = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[
            {"kind":"fill","reference":compiled_name_ref,"expected_value":"form","value":"compiled"},
            {"kind":"select","reference":compiled_country_ref,"expected_value":"a","value":"b"}
        ]}))).await.unwrap().structured_content.unwrap();
        assert_eq!(mixed_program["status"], "completed", "adjacent fill/select operations compile into one guarded bridge program: {mixed_program}");
        assert_eq!(mixed_program["receipts"][0]["action"], "fill_form", "{mixed_program}");
        assert_eq!(mixed_program["receipts"][0]["completed_fields"], 2, "{mixed_program}");
        let task_form = client.call_tool(call("browser", json!({"action":"task","session_id":session_id,"chrome_tab_id":target_id,"task":{
            "kind":"form_draft","fields":[
                {"reference":compiled_name_ref,"expected_value":"compiled","value":"tasked"},
                {"reference":compiled_next_ref,"expected_value":"batch","value":"tasked-next"}
            ],"save_without_submit":false
        }}))).await.unwrap().structured_content.unwrap();
        assert_eq!(task_form["status"], "completed", "bounded task form should share the guarded retained generation: {task_form}");
        assert_eq!(task_form["steps"], 2, "{task_form}");
        assert!(task_form["receipts"].as_array().unwrap().iter().all(|receipt|receipt["verified"]==true), "{task_form}");
        let verified_workflow = client.call_tool(call("workflow", json!({
            "session_id":session_id,"chrome_tab_id":target_id,
            "steps":[
                {"kind":"snapshot"},
                {"kind":"find","query":"Name","role":"input","save_as":"verify_name"},
                {"kind":"verify","predicate":{"kind":"value","reference":"$verify_name","equals":"tasked"}}
            ]
        }))).await.unwrap().structured_content.unwrap();
        assert_eq!(verified_workflow["status"], "completed", "{verified_workflow}");
        assert!(verified_workflow["metrics"]["phases"]["grounding_us"].as_u64().unwrap_or_default() > 0, "{verified_workflow}");
        assert!(verified_workflow["metrics"]["phases"]["verification_us"].as_u64().unwrap_or_default() > 0, "{verified_workflow}");
        let guarded_snapshot = super::snapshot_impl(&app.core, &super::BrowserSnapshotArgs {
            session_id: session_id.clone(), chrome_tab_id: target_id.clone(), selector: None,
            max_items: None, max_text_chars: None, max_bytes: None, href_hint: None,
            frame_id: None, goal: None, max_scan_nodes: None,
        }).await.unwrap();
        let guarded_ref = guarded_snapshot["items"].as_array().unwrap().iter().find(|item| item["name"] == "Name").unwrap()["reference"].as_str().unwrap().to_owned();
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.querySelector('[aria-label=Name]').style.visibility='hidden';true",
                "returnByValue":true
            })).await.unwrap();
        }
        let hidden_fill = super::workflow_fill_form(&app, &super::V3WorkflowArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), steps:vec![], program:None, client_id:None,
        }, vec![super::V3WorkflowFillField {
            reference:guarded_ref, expected_value:"tasked".into(), value:"must-not-fill".into(), select:false,
        }], &std::collections::BTreeMap::new()).await.unwrap();
        assert_eq!(hidden_fill["status"], "not_dispatched", "live visibility changes must stop batched fill before input: {hidden_fill}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.querySelector('[aria-label=Name]').style.visibility='visible';true",
                "returnByValue":true
            })).await.unwrap();
        }
        let post_guard_snapshot = super::snapshot_impl(&app.core, &super::BrowserSnapshotArgs {
            session_id: session_id.clone(), chrome_tab_id: target_id.clone(), selector: None,
            max_items: None, max_text_chars: None, max_bytes: None, href_hint: None,
            frame_id: None, goal: None, max_scan_nodes: None,
        }).await.unwrap();
        let post_guard_ref = post_guard_snapshot["items"].as_array().unwrap().iter().find(|item| item["name"] == "Name").unwrap()["reference"].as_str().unwrap().to_owned();
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"const e=document.querySelector('[aria-label=Name]');e.addEventListener('input',()=>{if(e.value==='rewrite-me')e.value='changed-by-listener'});true",
                "returnByValue":true
            })).await.unwrap();
        }
        let readback_mismatch = super::workflow_fill_form(&app, &super::V3WorkflowArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), steps:vec![], program:None, client_id:None,
        }, vec![super::V3WorkflowFillField {
            reference:post_guard_ref, expected_value:"tasked".into(), value:"rewrite-me".into(), select:false,
        }], &std::collections::BTreeMap::new()).await.unwrap();
        assert_eq!(readback_mismatch["status"], "unknown", "a value changed by the page after input dispatch must never be reported untouched: {readback_mismatch}");
        assert_eq!(readback_mismatch["completed_fields"], 0, "a readback-mismatched field is not verified complete: {readback_mismatch}");
        assert_eq!(readback_mismatch["attempted_fields"], 1, "the failed readback still records the dispatched field: {readback_mismatch}");
        assert_eq!(readback_mismatch["deltas"][0]["reason_code"], "readback_mismatch", "retain the concrete failure evidence: {readback_mismatch}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            let observed = live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.querySelector('[aria-label=Name]').value",
                "returnByValue":true
            })).await.unwrap();
            assert_eq!(super::super::shared_value(&observed).unwrap(), "changed-by-listener", "the app changed the field after dispatch; the receipt must remain unknown");
        }
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"const [a,b]=document.querySelectorAll('input');a.value='first-before';b.value='second-before';b.addEventListener('input',()=>{a.value='cleared-by-later-field'});true",
                "returnByValue":true
            })).await.unwrap();
        }
        let before_cross_field = super::super::snapshot_impl(&app.core, &super::super::BrowserSnapshotArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), selector:None,
            max_items:None, max_text_chars:Some(0), max_bytes:None, href_hint:None,
            frame_id:None, goal:None, max_scan_nodes:None,
        }).await.unwrap();
        let first_field_ref = before_cross_field["items"].as_array().unwrap().iter().find(|item|item["name"]=="Name").unwrap()["reference"].as_str().unwrap().to_owned();
        let second_field_ref = before_cross_field["items"].as_array().unwrap().iter().find(|item|item["name"]=="Next").unwrap()["reference"].as_str().unwrap().to_owned();
        let cross_field_change = super::workflow_fill_form(&app, &super::V3WorkflowArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), steps:vec![], program:None, client_id:None,
        }, vec![
            super::V3WorkflowFillField { reference:first_field_ref, expected_value:"first-before".into(), value:"first-after".into(), select:false },
            super::V3WorkflowFillField { reference:second_field_ref, expected_value:"second-before".into(), value:"second-after".into(), select:false },
        ], &std::collections::BTreeMap::new()).await.unwrap();
        assert_eq!(cross_field_change["status"], "unknown", "final batch readback must catch a later input listener changing an earlier field: {cross_field_change}");
        assert_eq!(cross_field_change["final_readback"]["checks"][0], false, "the earlier field did not retain its requested value: {cross_field_change}");
        assert_eq!(cross_field_change["deltas"][0]["verified"], false, "an immediate per-field readback cannot override failed final batch readback: {cross_field_change}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"const b=document.querySelectorAll('input')[1];b.replaceWith(b.cloneNode(true));true",
                "returnByValue":true
            })).await.unwrap();
        }
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.body.replaceChildren(Object.assign(document.createElement('button'),{textContent:'Open requested panel'}),...Array.from({length:50000},(_,i)=>Object.assign(document.createElement('div'),{textContent:'dashboard cell '+i})));true",
                "returnByValue":true
            })).await.unwrap();
        }
        let targeted = super::super::snapshot_impl(&app.core, &super::super::BrowserSnapshotArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), selector:None,
            max_items:Some(1), max_text_chars:Some(0), max_bytes:Some(8192), href_hint:None,
            frame_id:None, goal:Some(super::super::SnapshotGoal {
                query:Some("Open requested panel".into()), roles:Some(vec!["button".into()]),
                href_hint:None, within:None, exhaustive:false,
            }), max_scan_nodes:Some(50_000),
        }).await.unwrap();
        assert_eq!(targeted["goal_satisfied"], true, "{targeted}");
        assert_eq!(targeted["scope_complete"], false, "fast target reads remain explicitly partial: {targeted}");
        assert!(targeted["scanned_nodes"].as_u64().unwrap() < 100, "target search scanned unrelated nodes: {targeted}");
        let workflow_targeted = client.call_tool(call("workflow", json!({
            "session_id":session_id,"chrome_tab_id":target_id,"steps":[{"kind":"snapshot","goal":{"query":"Open requested panel","roles":["button"]},"max_scan_nodes":50000,"max_items":1,"max_text_chars":0}]
        }))).await.unwrap().structured_content.unwrap();
        assert_eq!(workflow_targeted["status"], "completed", "workflow snapshot must accept and run targeted goals: {workflow_targeted}");
        assert_eq!(workflow_targeted["receipts"][0]["goal_satisfied"], true, "{workflow_targeted}");
        assert!(workflow_targeted["receipts"][0]["scanned_nodes"].as_u64().unwrap() < 100, "workflow target snapshot visited unrelated nodes: {workflow_targeted}");
        assert_eq!(workflow_targeted["receipts"][0]["text_requested"], false, "control-only reads must declare that text was omitted: {workflow_targeted}");
        let roles_only = super::super::snapshot_impl(&app.core, &super::super::BrowserSnapshotArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), selector:None,
            max_items:Some(1), max_text_chars:Some(0), max_bytes:Some(8192), href_hint:None,
            frame_id:None, goal:Some(super::super::SnapshotGoal {
                query:None, roles:Some(vec!["button".into()]), href_hint:None, within:None, exhaustive:false,
            }), max_scan_nodes:Some(50_000),
        }).await.unwrap();
        assert_eq!(roles_only["goal_satisfied"], true, "role-only targeted reads must work: {roles_only}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.body.replaceChildren(Object.assign(document.createElement('a'),{href:'/wanted-story',textContent:'Wanted story'}));true",
                "returnByValue":true
            })).await.unwrap();
        }
        let href_only = super::super::snapshot_impl(&app.core, &super::super::BrowserSnapshotArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), selector:None,
            max_items:Some(1), max_text_chars:Some(0), max_bytes:Some(8192), href_hint:None,
            frame_id:None, goal:Some(super::super::SnapshotGoal {
                query:None, roles:None, href_hint:Some("/wanted-story".into()), within:None, exhaustive:false,
            }), max_scan_nodes:Some(50_000),
        }).await.unwrap();
        assert_eq!(href_only["goal_satisfied"], true, "href-only targeted reads must work: {href_only}");
        let partial_best = super::super::find_impl(&app.core, &super::super::BrowserFindArgs {
            session_id:session_id.clone(), chrome_tab_id:target_id.clone(), query:"Open requested panel".into(),
            role:Some("button".into()), limit:Some(5), max_matches:None,
            match_mode:Some("best".into()), within_reference:None, href_hint:None,
        }).await.unwrap();
        assert_eq!(partial_best["ambiguous"], true, "incomplete targeted scan cannot certify unique best: {partial_best}");
        assert_eq!(partial_best["scope_complete"], false);
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.body.innerHTML='<div id=\"list\" style=\"height:400px;overflow:auto\"><div id=\"rows\">'+Array.from({length:10},(_,i)=>`<div class=\"row\">Plain record ${i}</div>`).join('')+'</div></div>';true",
                "returnByValue":true
            })).await.unwrap();
        }
        let plain_rows = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[{
            "kind":"collect_virtual","container":"#list","query":"Plain record","identity_fields":["text"],"max_scrolls":3,"max_items":100,"save_as":"plain_rows"
        }]}))).await.unwrap().structured_content.unwrap();
        assert_eq!(plain_rows["status"], "completed", "text-only rows should be collected without requiring controls: {plain_rows}");
        assert_eq!(plain_rows["receipts"][0]["count"], 10, "text-only records should not silently disappear: {plain_rows}");
        assert_eq!(plain_rows["receipts"][0]["coverage_complete"], true, "the complete static text list should retain its coverage proof: {plain_rows}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.body.innerHTML='<div id=\"list\" style=\"height:400px;overflow:auto\">'+Array.from({length:201},(_,i)=>`<button>R${i}</button>`).join('')+'</div>';true",
                "returnByValue":true
            })).await.unwrap();
        }
        let static_collection = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[{
            "kind":"collect_virtual","container":"#list","role":"button","identity_fields":["text"],"max_scrolls":3,"max_items":500,"save_as":"static_rows"
        }]}))).await.unwrap().structured_content.unwrap();
        assert!(static_collection["receipts"][0]["count"].as_u64().is_some_and(|count| count > 0 && count < 201), "the byte-bounded snapshot slice must not claim all 201 rows: {static_collection}");
        assert_eq!(static_collection["receipts"][0]["coverage_complete"], false, "collector must propagate the snapshot's omitted-row signal instead of claiming complete coverage: {static_collection}");
        assert_eq!(static_collection["receipts"][0]["slice_incomplete"], true, "the receipt should expose why collection is partial: {static_collection}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            let connection = live.connection.as_ref().unwrap();
            let expression = r#"(()=>{document.body.innerHTML='<style>#list{height:480px;overflow:auto;position:relative}.row{height:24px}</style><div id="list"><div style="height:24000px"></div><div id="rows" style="position:absolute;top:0;left:0"></div></div>';const list=document.querySelector('#list'),rows=document.querySelector('#rows');const render=()=>{const first=Math.floor(list.scrollTop/24);rows.style.transform=`translateY(${list.scrollTop}px)`;rows.innerHTML=Array.from({length:Math.min(20,1000-first)},(_,offset)=>{const id=first+offset;return `<div class="row"><a href="/row/${id}">Record ${id}</a></div>`}).join('')};list.addEventListener('scroll',()=>setTimeout(render,100));render();return true})()"#;
            connection.command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({"expression":expression,"returnByValue":true})).await.unwrap();
        }
        let virtual_result = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[{
            "kind":"collect_virtual","container":"#list","query":"Record 75","role":"link",
            "identity_fields":["href"],"max_scrolls":6,"max_items":200,"save_as":"records"
        }]}))).await.unwrap().structured_content.unwrap();
        assert_eq!(virtual_result["status"], "completed", "{virtual_result}");
        let collected = &virtual_result["receipts"][0];
        assert!(collected["matches"].as_array().unwrap().iter().any(|item| item["name"] == "Record 75"), "1,000-row virtualized fixture lost a record before its configured continuation limit: {collected}");
        let hrefs = collected["matches"].as_array().unwrap().iter().map(|item| item["href"].as_str().unwrap()).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(hrefs.len(), collected["count"].as_u64().unwrap() as usize, "recycled rows produced duplicate identities: {collected}");
        assert_eq!(collected["coverage_complete"], false, "a bounded partial walk must not claim full coverage: {collected}");
        assert_eq!(collected["scrolls"], 6);
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"const l=document.querySelector('#list');Object.defineProperty(l,'scrollTop',{configurable:true,get(){return 0},set(_){}});true",
                "returnByValue":true
            })).await.unwrap();
        }
        let stalled = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[{
            "kind":"collect_virtual","container":"#list","query":"Record","role":"link",
            "identity_fields":["href"],"max_scrolls":2,"max_items":200,"save_as":"stalled"
        }]}))).await.unwrap().structured_content.unwrap();
        assert_eq!(stalled["status"], "completed", "a stalled scroll is a partial observation, not a tool error: {stalled}");
        assert_eq!(stalled["receipts"][0]["end_of_list"], false, "non-advancing scroll without bottom evidence must not certify end: {stalled}");
        assert_eq!(stalled["receipts"][0]["coverage_complete"], false);
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":r#"(()=>{document.body.innerHTML='<style>#list{height:480px;overflow:auto;position:relative}</style><div id="list"><div style="height:960px"></div><div id="rows" style="position:absolute;top:0;left:0"></div></div>';const l=document.querySelector('#list'),rows=l.querySelector('#rows');const render=()=>{const first=Math.floor(l.scrollTop/24);rows.style.transform=`translateY(${l.scrollTop}px)`;rows.innerHTML=Array.from({length:Math.min(20,40-first)},(_,offset)=>{const id=first+offset;return `<div><a href="/tail/${id}">Tail ${id}</a></div>`}).join('')};l.addEventListener('scroll',()=>setTimeout(render,100));render();true})()"#,
                "returnByValue":true
            })).await.unwrap();
        }
        let tail = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[{
            "kind":"collect_virtual","container":"#list","query":"Tail","role":"link",
            "identity_fields":["href"],"max_scrolls":5,"max_items":200,"save_as":"tail"
        }]}))).await.unwrap().structured_content.unwrap();
        assert_eq!(tail["receipts"][0]["end_of_list"], true, "only measured bottom plus a final viewport snapshot completes coverage: {tail}");
        assert_eq!(tail["receipts"][0]["coverage_complete"], true, "final bottom viewport must be included before complete coverage: {tail}");
        assert_eq!(tail["receipts"][0]["count"], 40, "the bottom viewport must not be skipped: {tail}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":r#"(()=>{document.body.innerHTML='<style>#list{height:480px;overflow:auto}</style><div id=list></div>';setTimeout(()=>document.querySelector('#list').innerHTML=Array.from({length:20},(_,i)=>`<a href='/late/${i}'>Late ${i}</a>`).join(''),500);true})()"#,
                "returnByValue":true
            })).await.unwrap();
        }
        let delayed_bottom = client.call_tool(call("workflow", json!({"session_id":session_id,"chrome_tab_id":target_id,"steps":[{
            "kind":"collect_virtual","container":"#list","query":"Late","role":"link",
            "identity_fields":["href"],"max_scrolls":3,"max_items":200,"save_as":"delayed"
        }]}))).await.unwrap().structured_content.unwrap();
        assert_eq!(delayed_bottom["status"], "completed", "delayed bottom content is a bounded partial/complete observation, not an error: {delayed_bottom}");
        assert_eq!(delayed_bottom["receipts"][0]["coverage_complete"], true, "the collector must re-snapshot after a transient bottom and include asynchronously appended rows: {delayed_bottom}");
        assert_eq!(delayed_bottom["receipts"][0]["count"], 20, "transient bottom returned before delayed rows arrived: {delayed_bottom}");
        {
            let live = app.core.legacy.shared_sessions.lock().await.get(&session_id).unwrap().clone();
            let live = live.lock().await;
            live.connection.as_ref().unwrap().command(live.registry().unwrap(), &live.handle, &target_id, "Runtime.evaluate", json!({
                "expression":"document.body.innerHTML='<input aria-label=Name value=after><input aria-label=Next value=done>'",
                "returnByValue":true
            })).await.unwrap();
        }
        let snapshot = client
            .call_tool(call(
                "snapshot",
                json!({"session_id":session_id,"chrome_tab_id":target_id}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let reference = snapshot["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Name")
            .unwrap()["reference"]
            .as_str()
            .unwrap()
            .to_owned();
        let focused = client.call_tool(call("act", json!({"session_id":session_id,"chrome_tab_id":target_id,"action":"click","reference":reference,"outcome":{"kind":"focused"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(focused["status"], "verified", "{focused}");
        let snapshot = client
            .call_tool(call(
                "snapshot",
                json!({"session_id":session_id,"chrome_tab_id":target_id}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let reference = snapshot["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Name")
            .unwrap()["reference"]
            .as_str()
            .unwrap()
            .to_owned();
        let typed = client.call_tool(call("act", json!({"session_id":session_id,"chrome_tab_id":target_id,"action":"type","reference":reference,"expected_value":"after","value":"!","typing_mode":"fast_keys","timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(typed["status"], "verified", "{typed}");
        let verified = client.call_tool(call("verify", json!({"session_id":session_id,"chrome_tab_id":target_id,"predicate":{"kind":"value","reference":reference,"equals":"after!"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(verified["status"], "passed", "{verified}");
        use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::TcpListener,sync::oneshot};
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address=listener.local_addr().unwrap();
        let (release_slow,release_receiver)=oneshot::channel::<()>();
        let release_receiver=std::sync::Arc::new(tokio::sync::Mutex::new(Some(release_receiver)));
        let local_server=tokio::spawn(async move {
            loop {
                let Ok((mut stream,_))=listener.accept().await else {break};
                let release_receiver=release_receiver.clone();
                tokio::spawn(async move {
                    let mut request=Vec::new();let mut byte=[0_u8;1];
                    while stream.read(&mut byte).await.unwrap_or(0)==1 {request.push(byte[0]);if request.ends_with(b"\r\n\r\n"){break;}}
                    let request=String::from_utf8_lossy(&request);
                    if request.starts_with("GET /slow.gif ") {
                        if let Some(receiver)=release_receiver.lock().await.take(){let _=receiver.await;}
                        let gif=[71,73,70,56,57,97,1,0,1,0,128,0,0,0,0,0,255,255,255,33,249,4,1,0,0,0,0,44,0,0,0,0,1,0,1,0,0,2,2,68,1,0,59];
                        let _=stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",gif.len()).as_bytes()).await;
                        let _=stream.write_all(&gif).await;
                    } else if request.starts_with("GET / ") {
                        let body=b"<!doctype html><title>Committed</title><img src='/slow.gif'>";
                        let _=stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await;
                        let _=stream.write_all(body).await;
                    } else {let _=stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;}
                });
            }
        });
        let destination=format!("http://{address}/");
        let navigation=super::task_navigate(&app,&session_id,&target_id,&destination).await;
        assert_eq!(navigation["status"],"verified","task navigation should finish at DOMContentLoaded rather than wait for unrelated slow media: {navigation}");
        assert_eq!(navigation["document_committed"],true,"{navigation}");
        assert_eq!(navigation["destination_match"],true,"{navigation}");
        assert_ne!(navigation["ready_state"],"complete","{navigation}");
        assert_eq!(navigation["timed_out_waiting_for"],Value::Null,"{navigation}");
        let _=release_slow.send(());
        let mut ready=false;
        for _ in 0..50 {
            let current=super::snapshot_impl(&app.core,&super::BrowserSnapshotArgs{session_id:session_id.clone(),chrome_tab_id:target_id.clone(),selector:None,max_items:Some(1),max_text_chars:Some(100),max_bytes:None,href_hint:None,frame_id:None,goal:None,max_scan_nodes:None}).await.unwrap();
            if current["ready_state"]=="complete" {ready=true;break;}
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        local_server.abort();
        assert!(ready,"committed navigation did not finish loading after the fixture released it");
        let profile = {
            let sessions = app.core.legacy.shared_sessions.lock().await;
            let session = sessions.get(&session_id).unwrap().lock().await;
            let Some(super::super::PageSessionTransport::Dedicated(Some(browser))) =
                session.connection.as_ref()
            else {
                panic!("V3 session did not retain its dedicated Chrome process")
            };
            browser.profile_directory().to_path_buf()
        };
        assert!(profile.is_dir());
        let released = client
            .call_tool(call(
                "browser",
                json!({"action":"release","mode":"headless","session_id":session_id}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(released["released"], true, "{released}");
        assert_eq!(released["mode"], "headless", "{released}");
        assert!(
            !profile.exists(),
            "dedicated headless profile was not removed after release"
        );
        let _ = client.cancel().await;
        server_task.abort();
        });
        }).unwrap().join().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires installed Google Chrome; verifies hidden file selection and delayed app acceptance"]
    async fn v3_hidden_file_upload_waits_for_delayed_application_acceptance() {
        use rmcp::{ServiceExt, model::CallToolRequestParams};
        let executable = std::path::Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(executable.is_file());
        let mut app = AppV3::new(super::super::AppV2::new(super::super::App::default()));
        app.headless_chrome = Some(executable.to_path_buf());
        let (server_io, client_io) = tokio::io::duplex(65_536);
        let server = rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(app.clone(), server_io, None);
        let server_task = tokio::spawn(async move { let _ = server.waiting().await; });
        let client = ().serve(client_io).await.unwrap();
        let call = |name: &str, value: serde_json::Value| CallToolRequestParams::new(name.to_owned()).with_arguments(value.as_object().unwrap().clone());
        let launch = client.call_tool(call("browser", json!({"action":"launch","mode":"headless","url":"data:text/html,%3Cspan%20id%3D%22account%22%3Efixture-account%3C%2Fspan%3E%3Cinput%20id%3D%22file%22%20type%3D%22file%22%20style%3D%22display%3Anone%22%3E%3Cbutton%20id%3D%22status%22%20aria-label%3D%22Upload%20status%22%3EWaiting%3C%2Fbutton%3E%3Cscript%3Edocument.querySelector%28%22%23file%22%29.addEventListener%28%22change%22%2C%28%29%3D%3EsetTimeout%28%28%29%3D%3Edocument.querySelector%28%22%23status%22%29.innerText%3D%22Accepted%22%2C300%29%29%3C%2Fscript%3E"}))).await.unwrap().structured_content.unwrap();
        let session = launch["session_id"].as_str().unwrap().to_owned();
        let tab = launch["target_ids"][0].as_str().unwrap().to_owned();
        let registered = client.call_tool(call("browser", json!({"action":"artifact_register","session_id":session,"filename":"stage1.txt","bytes":[115,116,97,103,101,49]}))).await.unwrap().structured_content.unwrap();
        let snapshot = client.call_tool(call("snapshot", json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let status_ref = snapshot["items"].as_array().unwrap().iter().find(|item| item["name"] == "Upload status").unwrap()["reference"].as_str().unwrap();
        let uploaded = client.call_tool(call("browser", json!({"action":"task","session_id":session,"chrome_tab_id":tab,"task":{"kind":"upload","artifact_handle":registered["artifact_handle"],"selector":"#file","account_marker":["#account","fixture-account"],"acceptance":{"kind":"value","reference":status_ref,"equals":"Accepted"}}}))).await.unwrap().structured_content.unwrap();
        assert!(uploaded["elapsed_ms"].as_u64().is_some(),"{uploaded}");
        let receipt = &uploaded["receipts"][0];
        assert_eq!(receipt["verified"], true, "{uploaded}");
        assert_eq!(receipt["chrome_selected_file"], true, "{uploaded}");
        assert_eq!(receipt["application_accepted"], true, "{uploaded}");
        assert_eq!(receipt["persisted"], serde_json::Value::Null, "the fixture does not claim app persistence");
        assert_eq!(receipt["filename"], "stage1.txt");
        let released = client.call_tool(call("browser", json!({"action":"release","mode":"headless","session_id":session}))).await.unwrap().structured_content.unwrap();
        assert_eq!(released["released"], true, "{released}");
        let _ = client.cancel().await;
        server_task.abort();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires installed Google Chrome; exercises all V3 rich-editor typing modes"]
    async fn v3_rich_editor_fast_paragraphs_and_human_pace_are_verified() {
        use rmcp::{ServiceExt, model::CallToolRequestParams};
        let executable=std::path::Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(executable.is_file());
        let mut app=AppV3::new(super::super::AppV2::new(super::super::App::default()));
        app.headless_chrome=Some(executable.to_path_buf());
        let (server_io,client_io)=tokio::io::duplex(65536);
        let server=rmcp::service::serve_directly::<rmcp::RoleServer,_,_,_,_>(app.clone(),server_io,None);
        let server_task=tokio::spawn(async move {let _=server.waiting().await;});
        let client=().serve(client_io).await.unwrap();
        let call=|name:&str,value:serde_json::Value|CallToolRequestParams::new(name.to_owned()).with_arguments(value.as_object().unwrap().clone());
        let launch=client.call_tool(call("browser",json!({"action":"launch","mode":"headless","url":"about:blank"}))).await.unwrap().structured_content.unwrap();
        let session=launch["session_id"].as_str().unwrap().to_owned();
        let tab=launch["target_ids"][0].as_str().unwrap().to_owned();
        let connected=app.core.legacy.shared_sessions.lock().await.get(&session).unwrap().clone();
        let connection={let state=connected.lock().await;let Some(super::super::PageSessionTransport::Dedicated(Some(browser)))=state.connection.as_ref() else {panic!("headless browser transport missing")};browser.connection().clone()};
        let (_,targets)=connection.target_snapshot().await;
        let target=targets.iter().find(|target|target.id==tab).unwrap();
        connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.body.innerHTML='<div id=editor contenteditable=true aria-label=Announcement style=\"width:400px;height:40px\"></div><div id=readonly contenteditable=\"false\" role=\"textbox\" aria-label=\"Read only\" style=\"width:400px;height:40px\"></div><input id=ariaReadonly type=text aria-readonly=true aria-label=\"ARIA readonly field\" value=locked style=\"width:100px;height:20px\"><input id=ariaDisabledInput type=text aria-disabled=true aria-label=\"ARIA disabled input\" value=locked style=\"width:100px;height:20px\"><select id=ariaDisabled aria-disabled=true aria-label=\"ARIA disabled select\" style=\"width:100px;height:20px\"><option value=one selected>One</option><option value=two>Two</option></select><select id=ariaReadonlySelect aria-readonly=true aria-label=\"ARIA readonly select\" style=\"width:100px;height:20px\"><option value=one selected>One</option><option value=two>Two</option></select>';window.events=[];for(const t of ['beforeinput','input','compositionstart','compositionend','keydown','keyup'])document.addEventListener(t,x=>{if(x.target.closest('#editor'))window.events.push({type:t,key:x.key,inputType:x.inputType,data:x.data,trusted:x.isTrusted})},true)","returnByValue":true})).await.unwrap();
        connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.querySelector('#editor').innerHTML='<div><br></div>'","returnByValue":true})).await.unwrap();
        let blank_snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let blank_reference=blank_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Announcement").unwrap()["reference"].as_str().unwrap();
        let blank_typed=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"type","reference":blank_reference,"expected_value":"","value":"Seed","typing_mode":"block","timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(blank_typed["status"],"verified","empty rich editor newline normalization: {blank_typed}");
        connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.querySelector('#editor').innerHTML=''","returnByValue":true})).await.unwrap();
        connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.querySelector('#editor').innerHTML='<div><br></div>'","returnByValue":true})).await.unwrap();
        let newline_snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let newline_reference=newline_snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Announcement").unwrap()["reference"].as_str().unwrap();
        let newline_typed=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"type","reference":newline_reference,"expected_value":"\n","value":"S","typing_mode":"human_keys","delay_ms":0,"timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(newline_typed["status"],"verified","human_keys must canonicalize the blank rich-editor newline before tracking progress: {newline_typed}");
        assert_eq!(newline_typed["readback"],"S","{newline_typed}");
        connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.querySelector('#editor').innerHTML='';window.events=[]","returnByValue":true})).await.unwrap();
        let mut snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let mut reference=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Announcement").unwrap()["reference"].as_str().unwrap().to_owned();
        assert_eq!(snapshot["items"].as_array().unwrap().iter().find(|item|item["reference"]==reference).unwrap()["editable_kind"],"rich_text");
        let readonly=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Read only").unwrap_or_else(||panic!("readonly textbox missing: {snapshot}"));
        assert_eq!(readonly["editable_kind"],serde_json::Value::Null);
        let blocked=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"type","reference":readonly["reference"],"expected_value":"","value":"wrong","typing_mode":"block","timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(blocked["status"],"not_dispatched","{blocked}");
        snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let aria_readonly=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="ARIA readonly field").unwrap();
        let blocked=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"fill","reference":aria_readonly["reference"],"expected_value":"locked","value":"changed","timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(blocked["status"],"not_dispatched","{blocked}");
        snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let aria_disabled=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="ARIA disabled select").unwrap();
        let blocked=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"select","reference":aria_disabled["reference"],"expected_value":"one","value":"two","timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(blocked["status"],"not_dispatched","{blocked}");
        snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let aria_disabled_input=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="ARIA disabled input").unwrap();
        let blocked=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"fill","reference":aria_disabled_input["reference"],"expected_value":"locked","value":"changed","timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(blocked["status"],"not_dispatched","{blocked}");
        snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let aria_readonly_select=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="ARIA readonly select").unwrap();
        let blocked=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"select","reference":aria_readonly_select["reference"],"expected_value":"one","value":"two","timeout_ms":10000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(blocked["status"],"not_dispatched","{blocked}");
        snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        reference=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Announcement").unwrap()["reference"].as_str().unwrap().to_owned();
        let steps=[("block","Block<b>"),("fast_keys"," Fast\n\nParagraphs"),("human_keys"," and paced")];
        let mut expected=String::new();
        let mut keyed_characters=0;
        for (mode,text) in steps {
            if mode=="human_keys" {
                connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.querySelector('#editor').addEventListener('keyup',event=>event.target.replaceWith(event.target.cloneNode(true)),{once:true});window.preventedOnce=false;document.addEventListener('beforeinput',event=>{if(event.data==='p'&&!window.preventedOnce){window.preventedOnce=true;event.preventDefault()}},true)","returnByValue":true})).await.unwrap();
            }
            let typed=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"type","reference":reference,"expected_value":expected,"value":text,"typing_mode":mode,"timeout_ms":30000}))).await.unwrap().structured_content.unwrap();
            let normalized=normalize_key_text(text);
            expected.push_str(&normalized);
            if mode!="block" { keyed_characters += normalized.chars().count(); }
            assert_eq!(typed["status"],"verified","{mode}: {typed}");
            assert_eq!(typed["readback"],expected,"{mode}: {typed}");
            if mode == "human_keys" {
                assert_eq!(typed["intentional_delay_ms"], 180, "{typed}");
                assert_eq!(typed["intentional_delay_total_ms"], text.chars().count() as u64 * 180, "{typed}");
            }
            snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
            let observed=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Announcement").unwrap()["raw_value"].as_str().unwrap();
            if mode=="fast_keys"&&text=="\n\n" { assert_eq!(observed,format!("{expected}\n\n")); } else { assert_eq!(observed,expected); }
        }
        let (_,targets)=connection.target_snapshot().await;let target=targets.iter().find(|target|target.id==tab).unwrap();
        let events=connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"JSON.stringify({text:document.querySelector('#editor').innerText,html:document.querySelector('#editor').innerHTML,events:window.events,preventedOnce:window.preventedOnce})","returnByValue":true})).await.unwrap()["result"]["value"].as_str().unwrap().to_owned();
        let events:serde_json::Value=serde_json::from_str(&events).unwrap();
        assert_eq!(events["text"],expected);
        assert!(!events["html"].as_str().unwrap().contains("<b>"),"markup-looking text must be inserted as text");
        assert!(events["events"].as_array().unwrap().iter().any(|event|event["type"]=="beforeinput"));
        assert!(events["events"].as_array().unwrap().iter().any(|event|event["type"]=="keydown"&&event["trusted"]==true),"FastKeys must dispatch trusted keydown: {events}");
        assert!(events["events"].as_array().unwrap().iter().any(|event|event["type"]=="keyup"&&event["trusted"]==true),"FastKeys must dispatch trusted keyup: {events}");
        assert_eq!(events["events"].as_array().unwrap().iter().filter(|event|event["type"]=="keydown").count(),keyed_characters+1);
        assert_eq!(events["preventedOnce"],true,"the missed character was retried only after readback proved it was absent: {events}");
        assert_eq!(events["events"].as_array().unwrap().iter().filter(|event|event["type"]=="keydown"&&event["key"]=="Enter").count(),1);
        let readonly_values=connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"JSON.stringify({textbox:document.querySelector('#readonly').innerText,readonly_input:document.querySelector('#ariaReadonly').value,disabled_input:document.querySelector('#ariaDisabledInput').value,disabled_select:document.querySelector('#ariaDisabled').value,readonly_select:document.querySelector('#ariaReadonlySelect').value})","returnByValue":true})).await.unwrap()["result"]["value"].as_str().unwrap().to_owned();
        let readonly_values:serde_json::Value=serde_json::from_str(&readonly_values).unwrap();
        assert_eq!(readonly_values,json!({"textbox":"","readonly_input":"locked","disabled_input":"locked","disabled_select":"one","readonly_select":"one"}));
        let released=client.call_tool(call("browser",json!({"action":"release","mode":"headless","session_id":session}))).await.unwrap().structured_content.unwrap();
        assert_eq!(released["released"],true,"{released}");
        let _=client.cancel().await;server_task.abort();
    }

    #[tokio::test]
    #[ignore = "requires installed Google Chrome; verifies explicit password fill and redaction"]
    async fn v3_password_fill_requires_opt_in_and_never_returns_the_value() {
        use rmcp::{ServiceExt, model::CallToolRequestParams};
        let executable=std::path::Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(executable.is_file());
        let mut app=AppV3::new(super::super::AppV2::new(super::super::App::default()));
        app.headless_chrome=Some(executable.to_path_buf());
        let (server_io,client_io)=tokio::io::duplex(65536);
        let server=rmcp::service::serve_directly::<rmcp::RoleServer,_,_,_,_>(app.clone(),server_io,None);
        let server_task=tokio::spawn(async move {let _=server.waiting().await;});
        let client=().serve(client_io).await.unwrap();
        let call=|name:&str,value:serde_json::Value|CallToolRequestParams::new(name.to_owned()).with_arguments(value.as_object().unwrap().clone());
        let launch=client.call_tool(call("browser",json!({"action":"launch","mode":"headless","url":"about:blank"}))).await.unwrap().structured_content.unwrap();
        let session=launch["session_id"].as_str().unwrap().to_owned();
        let tab=launch["target_ids"][0].as_str().unwrap().to_owned();
        let connected=app.core.legacy.shared_sessions.lock().await.get(&session).unwrap().clone();
        let connection={let state=connected.lock().await;let Some(super::super::PageSessionTransport::Dedicated(Some(browser)))=state.connection.as_ref() else {panic!("headless browser transport missing")};browser.connection().clone()};
        let (_,targets)=connection.target_snapshot().await;let target=targets.iter().find(|target|target.id==tab).unwrap();
        connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.body.innerHTML='<label for=password>Password</label><input id=password type=password style=\"width:200px;height:24px\">'","returnByValue":true})).await.unwrap();
        let snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let item=snapshot["items"].as_array().unwrap().iter().find(|item|item["credential_input"]==true).expect("password reference missing");
        assert!(item["raw_value"].is_null());
        let reference=item["reference"].as_str().unwrap();
        let sentinel="PASSWORD-TEST-SENTINEL";
        let blocked=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"fill","reference":reference,"value":sentinel}))).await.unwrap().structured_content.unwrap();
        assert_eq!(blocked["status"],"not_dispatched","ordinary fill must keep password protection: {blocked}");
        assert!(!blocked.to_string().contains(sentinel),"password leaked in blocked result: {blocked}");
        let snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let reference=snapshot["items"].as_array().unwrap().iter().find(|item|item["credential_input"]==true).unwrap()["reference"].as_str().unwrap();
        let result=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":tab,"action":"fill","reference":reference,"value":sentinel,"allow_password":true}))).await.unwrap().structured_content.unwrap();
        assert_eq!(result["status"],"verified","{result}");
        assert_eq!(result["readback"]["credential_filled"],true,"{result}");
        assert!(!result.to_string().contains(sentinel),"password leaked in result: {result}");
        let snapshot=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":tab}))).await.unwrap().structured_content.unwrap();
        let password=snapshot["items"].as_array().unwrap().iter().find(|item|item["credential_input"]==true).unwrap();
        assert!(password["raw_value"].is_null());
        assert!(!snapshot.to_string().contains(sentinel),"password leaked in snapshot: {snapshot}");
        let actual=connection.target_command(&tab,target.generation,&target.revision,"Runtime.evaluate",json!({"expression":"document.querySelector('#password').value","returnByValue":true})).await.unwrap()["result"]["value"].as_str().unwrap().to_owned();
        assert_eq!(actual,sentinel);
        let released=client.call_tool(call("browser",json!({"action":"release","mode":"headless","session_id":session}))).await.unwrap().structured_content.unwrap();
        assert_eq!(released["released"],true,"{released}");
        let _=client.cancel().await;server_task.abort();
    }
}
