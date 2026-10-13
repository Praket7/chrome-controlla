use super::*;
use rmcp::{
    ServerHandler, ServiceExt, handler::server::wrapper::{Json, Parameters}, tool, tool_handler,
    tool_router,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const MAX_WORKFLOW_STEPS: usize = 20;

#[derive(Clone, Default)]
struct SemanticTabState {
    document_key: String,
    document: Option<crate::semantic_state::SemanticDocumentKey>,
    next_ref: u32,
    key_to_short: BTreeMap<String, String>,
    short_to_legacy: BTreeMap<String, String>,
    identities: BTreeMap<String, crate::reference_reacquire::SemanticIdentity>,
    items: BTreeMap<String, Value>,
    snapshot_id: String,
    scope_complete: bool,
}

impl SemanticTabState {
    fn page_ordered_items(&self) -> Vec<&Value> {
        let mut items = self.items.values().collect::<Vec<_>>();
        items.sort_by_key(|item| item["index"].as_u64().unwrap_or(u64::MAX));
        items
    }
}

fn reset_semantic_document(state: &mut SemanticTabState, doc_key: &str, snapshot: &Value) {
    // Keep public references monotonic for this paired tab even when the
    // document changes. This prevents an old @cN from aliasing a new item.
    let next_ref = state.next_ref.max(1);
    *state = SemanticTabState {
        document_key: doc_key.to_owned(),
        document: Some(crate::semantic_state::SemanticDocumentKey::from_snapshot(snapshot)),
        next_ref,
        ..Default::default()
    };
}

#[derive(Clone)]
struct AppV2 {
    legacy: App,
    semantic: Arc<Mutex<BTreeMap<String, SemanticTabState>>>,
    observation_history: Arc<std::sync::Mutex<crate::observation_router::ObservationHistory>>,
}

impl AppV2 {
    fn new(legacy: App) -> Self {
        Self {
            legacy,
            semantic: Arc::default(),
            observation_history: Arc::new(std::sync::Mutex::new(crate::observation_router::ObservationHistory::open(
                super::state_directory().join("observation-costs-v1.json"),
            ))),
        }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserSessionArgs {
    action: String,
    provider: Option<String>,
    session_id: Option<String>,
    target_ids: Option<Vec<String>>,
    host_id: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserSnapshotArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: Option<String>,
    max_items: Option<usize>,
    max_text_chars: Option<usize>,
    max_bytes: Option<usize>,
    href_hint: Option<String>,
    frame_id: Option<String>,
    #[schemars(description = "Optional targeted search. A match is a bounded candidate, not proof that no better or duplicate candidate exists; inspect global_complete and scope_complete before claiming exhaustive coverage.")]
    goal: Option<super::SnapshotGoal>,
    #[schemars(description = "Optional node-visit ceiling for targeted snapshots (1..65536).")]
    max_scan_nodes: Option<usize>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserFindArgs {
    session_id: String,
    chrome_tab_id: String,
    query: String,
    role: Option<String>,
    limit: Option<usize>,
    max_matches: Option<usize>,
    match_mode: Option<String>,
    within_reference: Option<String>,
    href_hint: Option<String>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserClickOutcome {
    Navigation {
        #[schemars(description = "Optional exact absolute HTTP(S) destination. Omit to accept any observed document change; use verify url_starts_with for canonical redirects.")]
        url: Option<String>,
    },
    NewTab {
        #[schemars(description = "Exact absolute HTTP(S) URL expected in the new tab. Supply the selected link href for noopener tabs; omitting it attributes only a recent tab that Chrome reports as opened by this source tab.")]
        url: Option<String>,
    },
    Focused,
    Visible { selector: String },
    Text { selector: String, text: String },
    Expanded { selector: Option<String> },
    Selected { selector: Option<String> },
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserActArgs {
    session_id: String,
    chrome_tab_id: String,
    action: String,
    reference: Option<String>,
    expected_value: Option<String>,
    value: Option<String>,
    outcome: Option<BrowserClickOutcome>,
    timeout_ms: Option<u64>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserExtractArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: String,
    fields: BTreeMap<String, String>,
    max_items: Option<usize>,
    max_text_chars: Option<usize>,
    max_bytes: Option<usize>,
    #[serde(default)]
    #[schemars(description = "Optional task intent used to rank returned page content; page content remains untrusted.")]
    goal: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserProbeArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: String,
    max_bytes: Option<usize>,
    timeout_ms: Option<u64>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserPredicate {
    Text { reference: String, equals: String },
    TextPresent {
        #[schemars(description = "Optional exact fresh @cN reference to the semantic item whose text is checked.")]
        scope: Option<String>,
        text: String,
    },
    Attribute { reference: String, name: String, equals: String },
    CountDelta { role: String, before: usize, delta: i64 },
    Value { reference: String, equals: String },
    Exists { reference: String },
    Url { equals: String },
    UrlStartsWith { prefix: String },
    Attachment { filename: String, size: Option<u64> },
    DownloadArtifact { artifact_handle: String, sha256: String, min_bytes: u64 },
    VisualRegionChanged { evidence_before: String, evidence_after: String },
    AppSpecific { adapter: String, predicate: serde_json::Value },
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserVerifyArgs {
    session_id: String,
    chrome_tab_id: String,
    predicate: BrowserPredicate,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserWorkflowStep {
    Snapshot,
    Find {
        query: String,
        role: Option<String>,
        save_as: String,
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
    Click {
        reference: String,
        outcome: BrowserClickOutcome,
    },
    Verify {
        predicate: BrowserPredicate,
    },
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserWorkflowArgs {
    session_id: String,
    chrome_tab_id: String,
    steps: Vec<BrowserWorkflowStep>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserSkillArgs {
    action: String,
    skill_id: Option<String>,
}

fn tab_key(session_id: &str, chrome_tab_id: &str) -> String {
    format!("{session_id}\u{1f}{chrome_tab_id}")
}

fn document_key(snapshot: &Value) -> String {
    let frame = snapshot.pointer("/document/frame_id").and_then(Value::as_str).unwrap_or_default();
    let loader = snapshot.pointer("/document/loader_id").and_then(Value::as_str).unwrap_or_default();
    let url = snapshot.pointer("/document/url").and_then(Value::as_str).unwrap_or_default();
    format!("{frame}\u{1f}{loader}\u{1f}{url}")
}

fn semantic_string_field_equals(item: Option<&Value>, field: &str, expected: &str) -> bool {
    let Some(actual) = item.and_then(|item| item.get(field)) else { return false; };
    actual.as_str().is_some_and(|actual| actual == expected)
        || (matches!(field, "selected" | "expanded")
            && actual.as_bool().is_some_and(|actual| actual.to_string() == expected))
}

fn semantic_text_is_authoritative(item: &Value) -> bool {
    item["feedback_region"] != true
        && !matches!(item["role"].as_str(), Some("alert" | "status" | "log"))
}

fn semantic_count_delta(items: &BTreeMap<String, Value>, role: &str, before: usize, delta: i64) -> (bool, usize) {
    let actual = items.values().filter(|item| item["role"].as_str() == Some(role)).count();
    let expected = before as i128 + delta as i128;
    (expected >= 0 && expected <= usize::MAX as i128 && actual as i128 == expected, actual)
}

fn changed_fields(previous: &Value, current: &Value) -> bool {
    ["role", "name", "raw_value", "href", "expanded", "selected", "disabled", "requires_trusted_events", "present", "rendered", "in_viewport", "pointer_events_none", "actionability", "editable_kind", "multiline"]
        .into_iter()
        .any(|field| previous.get(field) != current.get(field))
}

pub(super) fn update_observation_bytes(value: &mut Value) -> u64 {
    if value.get("observation").is_none() { return 0; }
    let mut bytes = 0;
    for _ in 0..3 {
        bytes = serde_json::to_vec(value).map_or(0, |encoded| encoded.len() as u64);
        if value["observation"]["bytes"] == json!(bytes) { break; }
        value["observation"]["bytes"] = json!(bytes);
    }
    bytes
}

fn attach_observation_metadata(value: &mut Value, decision: crate::observation_router::ObservationDecision) -> u64 {
    value["observation"] = json!({"mode":decision.mode,"reason":decision.reason,"bytes":0});
    update_observation_bytes(value)
}

fn query_needs_layout(query: &str) -> bool {
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .collect::<std::collections::BTreeSet<_>>();
    ["layout", "chart", "graph", "map", "canvas", "diagram", "overlap", "position"]
        .iter()
        .any(|term| terms.contains(*term))
}

fn observation_task_class(query: Option<&str>) -> crate::observation_router::TaskClass {
    let words = query.unwrap_or_default().split(|c: char| !c.is_alphanumeric())
        .map(str::to_ascii_lowercase).collect::<std::collections::BTreeSet<_>>();
    if ["dashboard", "filter", "sort", "chart", "table"].iter().any(|word| words.contains(*word)) {
        crate::observation_router::TaskClass::FilterSortDashboard
    } else if ["fill", "submit", "save", "delete", "send", "purchase", "edit"].iter().any(|word| words.contains(*word)) {
        crate::observation_router::TaskClass::Mutation
    } else if query.is_some() {
        crate::observation_router::TaskClass::Research
    } else {
        crate::observation_router::TaskClass::General
    }
}

fn scoped_html_expression(selector: &str, max_chars: usize) -> String {
    format!(
        r#"(()=>{{let root;try{{const matches=[...document.querySelectorAll({selector})];if(matches.length!==1)return {{ok:false,reason:matches.length===0?'no_match':'ambiguous',count:matches.length}};root=matches[0].cloneNode(true)}}catch(_){{return {{ok:false,reason:'invalid_selector'}}}}root.querySelectorAll('script,style,noscript,iframe,object,embed,svg,input,textarea,select,button').forEach(node=>node.remove());for(const node of [root,...root.querySelectorAll('*')]){{for(const attr of [...node.attributes]){{if(!['id','class','role','aria-label','aria-expanded','aria-selected','aria-controls','data-state'].includes(attr.name))node.removeAttribute(attr.name)}}}}return {{ok:true,html:root.outerHTML.slice(0,{max_chars}),truncated:root.outerHTML.length>{max_chars}}}}})()"#,
        selector = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
        max_chars = max_chars.clamp(1, 32_768),
    )
}

async fn scoped_html_view(
    app: &AppV2,
    session_id: &str,
    tab_id: &str,
    selector: &str,
    max_bytes: usize,
) -> Result<Value, rmcp::ErrorData> {
    if selector.trim().is_empty() || selector.len() > 2048 || selector.contains('\0') {
        return Err(invalid("scoped HTML requires one bounded CSS selector"));
    }
    let shared = app.legacy.shared_sessions.lock().await
        .get(session_id).cloned().ok_or_else(|| invalid("unknown shared session_id"))?;
    let shared = shared.lock().await;
    if shared.handle.principal != app.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let connection = shared.connection.as_ref().ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let registry = shared.registry()?;
    let before = super::shared_frame_identity(connection, registry, &shared.handle, tab_id).await.map_err(invalid)?;
    let expression = scoped_html_expression(selector, max_bytes.min(32_768));
    let response = connection.command(registry, &shared.handle, tab_id, "Runtime.evaluate", serde_json::json!({"expression":expression,"returnByValue":true})).await.map_err(|error| invalid(error.to_string()))?;
    let mut result = super::shared_value(&response).map_err(invalid)?;
    let after = super::shared_frame_identity(connection, registry, &shared.handle, tab_id).await.map_err(invalid)?;
    if before != after { return Err(invalid("document changed during scoped HTML observation")); }
    if result["ok"] != true { return Err(invalid(format!("scoped HTML unavailable: {}", result["reason"].as_str().unwrap_or("unknown")))); }
    let html = result["html"].as_str().unwrap_or_default().to_owned();
    let limit = max_bytes.clamp(4096, 32_768);
    let mut bounded = String::new();
    for character in html.chars() {
        if bounded.len() + character.len_utf8() > limit { break; }
        bounded.push(character);
    }
    result["html"] = json!(bounded);
    result["truncated"] = json!(result["truncated"] == true || bounded.len() < html.len());
    result["document"] = json!({"frame_id":before.0,"loader_id":before.1,"url":before.2});
    result["page_content_is_untrusted"] = json!(true);
    Ok(result)
}

async fn snapshot_impl(app: &AppV2, args: &BrowserSnapshotArgs) -> Result<Value, rmcp::ErrorData> {
    let started = std::time::Instant::now();
    // Reuse the last observed candidate set to choose the next snapshot's
    // detail level. This avoids taking a second snapshot just to estimate
    // ambiguity, while still allowing a compact first read to trigger a rich
    // follow-up when it exposed several indistinguishable controls.
    let prior_ambiguity = app.semantic.lock().await
        .get(&tab_key(&args.session_id, &args.chrome_tab_id))
        .map(|state| semantic_ambiguity(state.items.values()))
        .unwrap_or(0.0);
    let request = crate::observation_router::ObservationRequest {
        task_class: observation_task_class(args.goal.as_ref().and_then(|goal| goal.query.as_deref())),
        model_profile: args.goal.as_ref().and_then(|goal| goal.model_profile).map(|profile| match profile {
            super::ObservationModelProfile::RichContext => crate::observation_router::ModelProfile::RichContext,
            super::ObservationModelProfile::CompactContext => crate::observation_router::ModelProfile::CompactContext,
        }).or_else(|| args.max_text_chars.map(|limit| if limit > 2_000 { crate::observation_router::ModelProfile::RichContext } else { crate::observation_router::ModelProfile::CompactContext })),
        exhaustive: args.goal.as_ref().is_some_and(|goal| goal.exhaustive),
        needs_layout: args.goal.as_ref().is_some_and(|goal| goal.needs_layout)
            || args.goal.as_ref().and_then(|goal| goal.query.as_deref()).is_some_and(query_needs_layout),
        ambiguity: prior_ambiguity,
        rich_context: args.max_text_chars.is_some_and(|limit| limit > 2_000),
        frame_id: args.frame_id.is_some(),
        scoped_selector: args.selector.is_some(),
    };
    let decision = app.observation_history.lock().map(|history|
        crate::observation_router::choose_observation_with_history(request, &history)
    ).unwrap_or_else(|_| crate::observation_router::choose_observation(request));
    if let Some(frame_id) = args.frame_id.as_deref() {
        let Json(mut value) = app.legacy.shared_accessibility(Parameters(SharedAccessibilityArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: None,
            frame_id: Some(frame_id.to_owned()),
            max_bytes: args.max_bytes.unwrap_or(64 * 1024).clamp(4096, 1_000_000),
            timeout_ms: Some(10_000),
        })).await?;
        let bytes = attach_observation_metadata(&mut value, decision);
        record_observation_feedback(app, request, decision, started, &value, bytes);
        controlla_browser::scheduler::telemetry::record_observation(0, bytes).await;
        controlla_browser::scheduler::telemetry::record_phase(
            controlla_browser::scheduler::telemetry::OperationPhase::Snapshot,
            started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        ).await;
        return Ok(value);
    }
    let legacy = app
        .legacy
        .shared_snapshot(Parameters(SharedSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: args.selector.clone().unwrap_or_else(snapshot_scope),
            max_items: args.max_items.unwrap_or_else(snapshot_items).clamp(1, 200),
            max_text_chars: args.max_text_chars.unwrap_or_else(|| if decision.mode == crate::observation_router::ObservationMode::SemanticRich { 6_000 } else { snapshot_chars() }).min(6000),
            max_bytes: args.max_bytes.unwrap_or_else(snapshot_bytes).clamp(4096, 100_000),
            href_hint: args.href_hint.clone(),
            goal: args.goal.clone(),
            max_scan_nodes: args.max_scan_nodes,
        }))
        .await?;
    let Json(mut snapshot) = legacy;
    let items = snapshot
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let intent_query = args.goal.as_ref().and_then(|goal| goal.query.as_deref());
    let items = crate::relevance::filter_items(&items, intent_query, args.max_items.unwrap_or(200));
    let item_keys = crate::semantic_state::stable_item_keys(&items, snapshot["snapshot_id"].as_str().unwrap_or_default());
    let key = tab_key(&args.session_id, &args.chrome_tab_id);
    let doc_key = document_key(&snapshot);
    let snapshot_id = snapshot
        .get("snapshot_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let scope_complete = snapshot["scope_complete"] == true;
    let mut semantic = app.semantic.lock().await;
    let state = semantic.entry(key).or_default();
    let document_changed = !state.document_key.is_empty() && state.document_key != doc_key;
    if state.document_key != doc_key {
        reset_semantic_document(state, &doc_key, &snapshot);
    }
    state.document = Some(crate::semantic_state::SemanticDocumentKey::from_snapshot(&snapshot));
    state.scope_complete = scope_complete;
    if state.next_ref == 0 {
        state.next_ref = 1;
    }
    let previous_items = state.items.clone();
    state.short_to_legacy.clear();
    state.items.clear();
    let mut public_items = Vec::with_capacity(items.len());
    for (index, (mut item, semantic_key)) in items.into_iter().zip(item_keys).enumerate() {
        let short = if let Some(existing) = state.key_to_short.get(&semantic_key) {
            existing.clone()
        } else {
            let new_ref = format!("@c{}", state.next_ref);
            state.next_ref = state.next_ref.saturating_add(1);
            state.key_to_short.insert(semantic_key, new_ref.clone());
            new_ref
        };
        let legacy_ref = item
            .get("reference")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("shared snapshot item omitted its retained reference"))?
            .to_owned();
        item["reference"] = json!(short);
        item["index"] = json!(index);
        if let Some(adapter) = crate::widget_adapters::resolve_widget(&item) {
            item["widget_adapter"] = json!({
                "kind": adapter.kind,
                "postcondition": adapter.live_postcondition,
                "fallback": adapter.fallback
            });
        }
        let identity = crate::reference_reacquire::SemanticIdentity::from_item(
            &item,
            state.document.as_ref().expect("snapshot document was set"),
        );
        state.identities.insert(short.clone(), identity);
        state.short_to_legacy.insert(short.clone(), legacy_ref);
        state.items.insert(short, item.clone());
        public_items.push(item);
    }
    let mut upsert = Vec::new();
    for (reference, item) in &state.items {
        match previous_items.get(reference) {
            Some(previous) if !changed_fields(previous, item) => {}
            _ => upsert.push(item.clone()),
        }
    }
    let removed = previous_items
        .keys()
        .filter(|reference| !state.items.contains_key(*reference))
        .cloned()
        .collect::<Vec<_>>();
    state.snapshot_id = snapshot_id.clone();
    snapshot["items"] = json!(public_items);
    snapshot["semantic_revision"] = json!(snapshot_id);
    snapshot["delta"] = json!({
        "full": previous_items.is_empty() || document_changed,
        "document_changed": document_changed,
        "upsert": upsert,
        "removed": removed
    });
    drop(semantic);
    if decision.mode == crate::observation_router::ObservationMode::HtmlScoped {
        if let Some(selector) = args.selector.as_deref() {
            snapshot["scoped_html"] = scoped_html_view(
                app, &args.session_id, &args.chrome_tab_id, selector,
                args.max_bytes.unwrap_or(32_768).min(32_768),
            ).await?;
        }
        snapshot["page_content_is_untrusted"] = json!(true);
    } else if decision.mode == crate::observation_router::ObservationMode::VisualTargeted {
        if let Some(selector) = args.selector.as_deref() {
            let visual = full::visual_probe(app, full::FullProbeArgs {
                session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(),
                reference: None, selector: Some(selector.to_owned()), region: None,
                max_bytes: Some(args.max_bytes.unwrap_or(262_144).min(262_144)), timeout_ms: Some(10_000),
            }).await?;
            if visual["visual_ref"]["frame_id"] == snapshot["document"]["frame_id"]
                && visual["visual_ref"]["loader_id"] == snapshot["document"]["loader_id"]
            {
                snapshot["visual_probe"] = visual;
            } else {
                return Err(invalid("document changed between semantic and targeted visual observations"));
            }
        }
        snapshot["page_content_is_untrusted"] = json!(true);
    }
    let bytes = attach_observation_metadata(&mut snapshot, decision);
    record_observation_feedback(app, request, decision, started, &snapshot, bytes);
    controlla_browser::scheduler::telemetry::record_observation(
        snapshot["scanned_nodes"].as_u64().unwrap_or(0),
        bytes,
    ).await;
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Snapshot,
        started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    ).await;
    Ok(snapshot)
}

fn record_observation_feedback(
    app: &AppV2,
    request: crate::observation_router::ObservationRequest,
    decision: crate::observation_router::ObservationDecision,
    started: std::time::Instant,
    value: &Value,
    bytes: u64,
) {
    let has_evidence = value["items"].as_array().is_some_and(|items| !items.is_empty())
        || value["nodes"].as_array().is_some_and(|nodes| !nodes.is_empty())
        || value["global_complete"] == true;
    let explicitly_failed = matches!(value["status"].as_str(), Some("failed" | "unknown" | "incomplete"));
    if let Ok(mut history) = app.observation_history.lock() {
        history.record(request, decision, started.elapsed().as_millis().clamp(1, u64::MAX as u128) as u64, bytes, has_evidence && !explicitly_failed);
    }
}

fn semantic_ambiguity<'a>(items: impl Iterator<Item = &'a Value>) -> f32 {
    let mut counts = std::collections::BTreeMap::<(String, String), usize>::new();
    let mut total = 0usize;
    for item in items {
        let role = item.get("role").and_then(Value::as_str).unwrap_or_default().trim();
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default().trim();
        if role.is_empty() || name.is_empty() { continue; }
        total += 1;
        *counts.entry((role.to_ascii_lowercase(), name.to_ascii_lowercase())).or_default() += 1;
    }
    if total == 0 { return 0.0; }
    let collisions = counts.values().map(|count| count.saturating_sub(1)).sum::<usize>();
    (collisions as f32 / total as f32).clamp(0.0, 1.0)
}

async fn resolve_reference(app: &AppV2, session_id: &str, tab_id: &str, reference: &str) -> Result<String, rmcp::ErrorData> {
    if !reference.starts_with("@c") || reference[2..].parse::<u32>().is_err() {
        return Err(invalid("reference must be an exact @cN value returned by browser_snapshot/browser_find"));
    }
    let mut semantic = app.semantic.lock().await;
    let state = semantic.get_mut(&tab_key(session_id, tab_id))
        .ok_or_else(|| invalid("reference is stale or unknown; take a fresh browser_snapshot"))?;
    if let Some(legacy) = state.short_to_legacy.get(reference) {
        return Ok(legacy.clone());
    }
    let expected = state.identities.get(reference)
        .ok_or_else(|| invalid("reference is stale or unknown; take a fresh browser_snapshot"))?;
    let current_document = state.document.as_ref()
        .ok_or_else(|| invalid("semantic document identity is unavailable; take a fresh browser_snapshot"))?;
    let items = state.items.values().cloned().collect::<Vec<_>>();
    let candidate = crate::reference_reacquire::reacquire_reference(expected, current_document, &items)
        .map_err(|error| invalid(format!("reference could not be rebound safely: {error}")))?;
    let current = state.items.get(&candidate.reference)
        .ok_or_else(|| invalid("reacquired candidate is no longer in the current snapshot"))?;
    let index = current["index"].as_u64().ok_or_else(|| invalid("reacquired candidate omitted its index"))?;
    let legacy = format!("{}:{index}", state.snapshot_id);
    state.short_to_legacy.insert(reference.to_owned(), legacy.clone());
    Ok(legacy)
}

async fn retained_mutation(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    allow_password: bool,
    action: &str,
) -> Result<Value, rmcp::ErrorData> {
    let first = retained_mutation_once(app, args, reference, allow_password, action).await?;
    if first["status"] != "not_dispatched" || first["readback"]["reason"] != "target_replaced" {
        return Ok(first);
    }

    let (expected, ancestor) = {
        let semantic = app.semantic.lock().await;
        let Some(state) = semantic.get(&tab_key(&args.session_id, &args.chrome_tab_id)) else { return Ok(first); };
        let Some(expected) = state.identities.get(reference).cloned() else { return Ok(first); };
        let ancestor = state.items.get(reference).and_then(|item| {
            Some(SnapshotSemanticAncestor {
                tag: item.pointer("/semantic_ancestor/tag")?.as_str()?.to_owned(),
                id: item.pointer("/semantic_ancestor/id")?.as_str()?.to_owned(),
            })
        });
        (expected, ancestor)
    };
    if expected.name.is_empty() || expected.role.is_empty() {
        let mut result = first;
        result["reacquisition"] = json!({"status":"not_rebound","reason":"target identity lacks an exact role or name"});
        return Ok(result);
    }
    let fresh = snapshot_impl(app, &BrowserSnapshotArgs {
        session_id: args.session_id.clone(),
        chrome_tab_id: args.chrome_tab_id.clone(),
        selector: None,
        max_items: Some(200),
        max_text_chars: Some(0),
        max_bytes: Some(100_000),
        href_hint: None,
        frame_id: None,
        goal: Some(SnapshotGoal {
            query: Some(expected.name.clone()),
            roles: Some(vec![expected.role.clone()]),
            href_hint: expected.href.clone(),
            within: ancestor,
            exhaustive: true,
            model_profile: None,
            needs_layout: false,
        }),
        max_scan_nodes: Some(65_536),
    }).await;
    let fresh = match fresh {
        Ok(snapshot) if snapshot["scope_complete"] == true => snapshot,
        Ok(snapshot) => {
            let mut result = first;
            result["reacquisition"] = json!({"status":"not_rebound","reason":"targeted semantic scope was incomplete","scope_complete":snapshot["scope_complete"]});
            return Ok(result);
        }
        Err(error) => {
            let mut result = first;
            result["reacquisition"] = json!({"status":"not_rebound","reason":error.to_string()});
            return Ok(result);
        }
    };
    let candidate = {
        let semantic = app.semantic.lock().await;
        let Some(state) = semantic.get(&tab_key(&args.session_id, &args.chrome_tab_id)) else { return Ok(first); };
        let Some(document) = state.document.as_ref() else { return Ok(first); };
        let items = state.items.values().cloned().collect::<Vec<_>>();
        crate::reference_reacquire::reacquire_reference(&expected, document, &items)
    };
    let candidate = match candidate {
        Ok(candidate) => candidate.reference,
        Err(error) => {
            let mut result = first;
            result["reacquisition"] = json!({"status":"not_rebound","reason":error.to_string()});
            return Ok(result);
        }
    };
    let mut rebound = retained_mutation_once(app, args, &candidate, allow_password, action).await?;
    rebound["reacquisition"] = json!({"status":"rebound","from":reference,"to":candidate,"semantic_revision":fresh["semantic_revision"]});
    Ok(rebound)
}

async fn retained_click(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    outcome: BrowserClickOutcome,
) -> Result<Value, rmcp::ErrorData> {
    let first = retained_click_once(app, args, reference, &outcome).await?;
    if !click_needs_reacquisition(&first) {
        return Ok(first);
    }
    let expected = {
        let semantic = app.semantic.lock().await;
        semantic.get(&tab_key(&args.session_id, &args.chrome_tab_id))
            .and_then(|state| state.identities.get(reference)).cloned()
    };
    let Some(expected) = expected.filter(|identity| !identity.name.is_empty() && !identity.role.is_empty()) else {
        return Ok(first);
    };
    let fresh = snapshot_impl(app, &BrowserSnapshotArgs {
        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(),
        selector: None, max_items: Some(200), max_text_chars: Some(0), max_bytes: Some(100_000),
        href_hint: None, frame_id: None,
        goal: Some(SnapshotGoal { query: Some(expected.name.clone()), roles: Some(vec![expected.role.clone()]),
            href_hint: expected.href.clone(), within: None, exhaustive: true, model_profile: None, needs_layout: false }),
        max_scan_nodes: Some(65_536),
    }).await?;
    if fresh["scope_complete"] != true { return Ok(first); }
    let candidate = {
        let semantic = app.semantic.lock().await;
        let Some(state) = semantic.get(&tab_key(&args.session_id, &args.chrome_tab_id)) else { return Ok(first); };
        let Some(document) = state.document.as_ref() else { return Ok(first); };
        crate::reference_reacquire::reacquire_reference(&expected, document, &state.items.values().cloned().collect::<Vec<_>>())
    };
    let candidate = match candidate { Ok(candidate) => candidate.reference, Err(_) => return Ok(first) };
    let mut rebound = retained_click_once(app, args, &candidate, &outcome).await?;
    rebound["reacquisition"] = json!({"status":"rebound_after_not_dispatched","from":reference,"to":candidate,"semantic_revision":fresh["semantic_revision"]});
    Ok(rebound)
}

fn click_needs_reacquisition(receipt: &Value) -> bool {
    receipt["status"] == "not_dispatched" && receipt["reason_code"] == "changed_target"
}

async fn retained_click_once(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    outcome: &BrowserClickOutcome,
) -> Result<Value, rmcp::ErrorData> {
    let legacy_reference = resolve_reference(app, &args.session_id, &args.chrome_tab_id, reference).await?;
    let shared_outcome: SharedClickOutcome = serde_json::from_value(serde_json::to_value(outcome).map_err(|e| invalid(e.to_string()))?)
        .map_err(|e| invalid(e.to_string()))?;
    let Json(value) = app.legacy.shared_click(Parameters(SharedClickArgs {
        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(),
        reference: legacy_reference, outcome: shared_outcome, timeout_ms: args.timeout_ms,
    })).await?;
    Ok(value)
}

async fn retained_mutation_once(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    allow_password: bool,
    action: &str,
) -> Result<Value, rmcp::ErrorData> {
    let legacy_reference = resolve_reference(app, &args.session_id, &args.chrome_tab_id, reference).await?;
    let expected = args.expected_value.clone().unwrap_or_default();
    let value = args.value.clone().unwrap_or_default();
    if expected.len() > 16_384 || value.len() > 16_384 {
        return Err(invalid("expected_value/value exceed the 16384 character input bound"));
    }
    let shared = app
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = shared.lock().await;
    if shared.handle.principal != app.legacy.principal.as_ref() {
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
    let index = index.parse::<usize>().map_err(|_| invalid("invalid retained reference index"))?;
    let unrestricted = crate::guard_reason::unrestricted_mode();
    if index >= snapshot.count || (!unrestricted && (token != snapshot.token || snapshot.consumed && !super::internal_workflow_snapshot_allowed(&snapshot.token))) {
        return Err(invalid("stale or consumed reference; take one fresh browser_snapshot"));
    }
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(10_000).clamp(1_000, 30_000));
    let deadline = tokio::time::Instant::now() + timeout;
    let (before, interaction) = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        let before = tokio::time::timeout_at(
            deadline,
            shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id),
        )
        .await
        .map_err(|_| invalid("action preflight timed out; no mutation dispatched"))?
        .map_err(invalid)?;
        let interaction = tokio::time::timeout_at(
            deadline,
            super::shared_interaction_epoch(connection, registry, &shared.handle, &args.chrome_tab_id, &snapshot.object_id),
        ).await.map_err(|_| invalid("action epoch check timed out; no mutation dispatched"))?
          .map_err(invalid)?;
        (before, interaction)
    };
    let epoch_use=if action=="slider" {crate::v3::EpochUse::PointerMutation}else{crate::v3::EpochUse::TextMutation};
    if !unrestricted && !super::shared_epoch_matches(&snapshot, &args.chrome_tab_id, &before, &interaction, epoch_use) {
        return Err(invalid("document changed since snapshot; take a fresh browser_snapshot"));
    }
    if allow_password && action != "fill" {
        return Err(invalid("allow_password is supported only for action=fill"));
    }
    let function = if action == "fill" && allow_password {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLInputElement)||e.type!=='password'||e.matches(':disabled')||e.readOnly||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};const r=e.getBoundingClientRect(),s=getComputedStyle(e);if(r.width<=0||r.height<=0||r.left<0||r.top<0||r.right>innerWidth||r.bottom>innerHeight||s.visibility==='hidden'||s.display==='none'||s.pointerEvents==='none'||document.elementFromPoint(r.left+r.width/2,r.top+r.height/2)!==e)return {ok:false,reason:'blocked'};const setter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:null}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,credential_filled:e.value===value,dispatched:true};}"#
    } else if action == "fill" {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)||['password','hidden','file','checkbox','radio','button','submit','reset','image'].includes(e.type||'')||e.matches(':disabled')||e.readOnly||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};const setter=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:value}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,value:e.value,dispatched:true};}"#
    } else if action == "slider" {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLInputElement)||e.type!=='range'||e.matches(':disabled')||e.getAttribute('aria-disabled')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};const n=Number(value),min=e.min===''?0:Number(e.min),max=e.max===''?100:Number(e.max),step=e.step==='any'?0:Number(e.step);if(!Number.isFinite(n)||n<min||n>max||(step>0&&Math.abs((n-min)/step-Math.round((n-min)/step))>1e-6))return {ok:false,reason:'invalid_range_value'};const setter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,String(n));e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertReplacementText',data:String(n)}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===String(n),value:e.value,aria_value:e.getAttribute('aria-valuenow'),dispatched:true};}"#
    } else {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLSelectElement)||e.matches(':disabled')||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};if(![...e.options].some(o=>o.value===value))return {ok:false,reason:'option_missing'};e.value=value;e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,value:e.value};}"#
    };
    // Consume before dispatch so a lost response cannot permit duplicate mutation via the same reference.
    if !unrestricted {
        shared.snapshots.get_mut(&args.chrome_tab_id)
            .ok_or_else(|| invalid("snapshot disappeared before dispatch"))?
            .consumed = true;
    }
    let verification_started = std::time::Instant::now();
    let response = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        tokio::time::timeout_at(
            deadline,
            connection.command(
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId": snapshot.object_id,
                    "functionDeclaration": function,
                    "arguments":[{"value":index},{"value":expected},{"value":value}],
                    "returnByValue":true
                }),
            ),
        )
        .await
    };
    let mut result = match response {
        Ok(Ok(response)) => match shared_value(&response) {
            Ok(value) if value["ok"] == true => {
                json!({"status":"verified","dispatch_acknowledged":true,"action":action,"readback":value})
            }
            Ok(value) if value["dispatched"] == true => {
                json!({"status":"unknown","dispatch_acknowledged":true,"dispatch_may_have_occurred":true,"action":action,"reason":"fill readback changed after input/change dispatch; inspect state before retrying","readback":value})
            }
            Ok(value) => {
                json!({"status":"not_dispatched","dispatch_acknowledged":true,"action":action,"reason":value["reason"],"readback":value})
            }
            Err(error) => json!({
                "status":"unknown","dispatch_acknowledged":true,
                "reason":format!("browser mutation reply could not be interpreted after dispatch: {error}; inspect state and do not automatically retry")
            }),
        },
        Ok(Err(error)) => json!({
            "status":"unknown","dispatch_acknowledged":false,
            "reason":format!("browser mutation delivery is uncertain: {error}; inspect state and do not automatically retry")
        }),
        Err(_) => json!({
            "status":"unknown","dispatch_acknowledged":false,
            "reason":"browser mutation deadline exceeded; effect may have occurred; inspect state and do not automatically retry"
        }),
    };
    let after = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        tokio::time::timeout_at(
            deadline,
            shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id),
        )
        .await
    };
    if !unrestricted && !matches!(after, Ok(Ok(ref identity)) if identity == &before) && result["status"] == "verified" {
        result = json!({
            "status":"unknown","dispatch_acknowledged":true,
            "reason":"document changed after mutation; local readback cannot establish the final page state; inspect before retrying"
        });
    }
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Verification,
        verification_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    ).await;
    drop(shared);
    if allow_password && result["status"] == "verified" {
        result["readback"] = json!({"credential_filled":true,"ok":true});
    }
    if result["status"] == "verified" {
        super::authorize_internal_workflow_snapshot(&snapshot.token);
        result["delta"] = json!({
            "reference": reference,
            "before": if allow_password { Value::Null } else { json!(expected) },
            "after": if allow_password { Value::Null } else { result["readback"].get("value").cloned().unwrap_or(json!(value)) },
            "credential_filled": allow_password,
            "document_changed": false,
            "verified": true
        });
    }
    Ok(result)
}

async fn find_impl(app: &AppV2, args: &BrowserFindArgs) -> Result<Value, rmcp::ErrorData> {
    if args.query.trim().is_empty() || args.query.len() > 512 {
        return Err(invalid("query must contain 1..512 characters"));
    }
    let key = tab_key(&args.session_id, &args.chrome_tab_id);
    if !app.semantic.lock().await.contains_key(&key) {
        let _ = snapshot_impl(app, &BrowserSnapshotArgs {
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
        }).await?;
    }
    let mode = args.match_mode.as_deref().unwrap_or("all");
    if !matches!(mode, "all" | "best" | "exact") || args.href_hint.as_ref().is_some_and(|hint| hint.is_empty() || hint.len() > 512) {
        return Err(invalid("match_mode must be all, best, or exact and href_hint must be bounded"));
    }
    let state = app.semantic.lock().await;
    let state = state.get(&key).ok_or_else(|| invalid("semantic snapshot state unavailable"))?;
    let ancestor_id = if let Some(reference) = args.within_reference.as_deref() {
        Some(state.items.get(reference).and_then(|item| item.pointer("/semantic_ancestor/id")).and_then(Value::as_str).ok_or_else(|| invalid("within_reference must identify a stable semantic ancestor"))?)
    } else { None };
    let ranked = crate::semantic_state::rank_candidates(state.items.values().cloned(), &args.query, args.role.as_deref(), args.href_hint.as_deref(), ancestor_id, mode == "exact");
    let ambiguous = find_is_ambiguous(mode, state.scope_complete, &ranked);
    let limit = args.max_matches.or(args.limit).unwrap_or(10).clamp(1, 50);
    let matches = ranked.into_iter().take(if mode == "best" && !ambiguous { 1 } else { limit }).collect::<Vec<_>>();
    let count = matches.len();
    Ok(json!({"matches":matches,"count":count,"ambiguous":ambiguous,"scope_complete":state.scope_complete,"semantic_revision":state.snapshot_id}))
}

fn find_is_ambiguous(mode: &str, scope_complete: bool, ranked: &[Value]) -> bool {
    match mode {
        "best" => !scope_complete || ranked.first().is_some_and(|item| item["confidence"] != "high"),
        "exact" => !scope_complete || ranked.len() > 1,
        _ => false,
    }
}

async fn verify_impl(app: &AppV2, args: &BrowserVerifyArgs) -> Result<Value, rmcp::ErrorData> {
    let started = std::time::Instant::now();
    let result = verify_impl_unmeasured(app, args).await;
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Verification,
        started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    )
    .await;
    result
}

async fn verify_impl_unmeasured(app: &AppV2, args: &BrowserVerifyArgs) -> Result<Value, rmcp::ErrorData> {
    let snapshot = snapshot_impl(app, &BrowserSnapshotArgs {
        session_id: args.session_id.clone(),
        chrome_tab_id: args.chrome_tab_id.clone(),
        selector: None,
        max_items: None,
        max_text_chars: Some(6000),
        max_bytes: Some(60_000),
        href_hint: None,
        frame_id: None,
            goal: None,
            max_scan_nodes: None,
    }).await?;
    if let BrowserPredicate::DownloadArtifact { artifact_handle, sha256, min_bytes } = &args.predicate {
        if artifact_handle.is_empty() || artifact_handle.len() > 256 || !valid_sha256(sha256) {
            return Err(invalid("download verification requires a bounded artifact handle and lowercase SHA-256"));
        }
        let live = app.legacy.shared_sessions.lock().await.get(&args.session_id).cloned()
            .ok_or_else(|| invalid("unknown shared session for download verification"))?;
        let live = live.lock().await;
        if live.handle.principal != app.legacy.principal.as_ref() {
            return Err(invalid("artifact session is not owned by this server principal"));
        }
        let handle: controlla_browser::sessions::ArtifactHandle = serde_json::from_value(json!(artifact_handle))
            .map_err(|_| invalid("invalid opaque artifact_handle"))?;
        let registry = live.registry()?;
        let metadata = registry.artifact_metadata(&live.handle, &handle)
            .map_err(|_| invalid("download artifact is unavailable in this session"))?;
        let path = registry.artifact_path_for(&live.handle.id, &live.handle.principal, &handle)
            .map_err(|_| invalid("download artifact path is unavailable"))?;
        let bytes = tokio::fs::read(path).await.map_err(|_| invalid("download artifact could not be read"))?;
        let digest = crate::artifacts::ArtifactExpectation::sha256(&bytes);
        let passed = bytes.len() as u64 >= *min_bytes && digest == *sha256 && metadata.size as u64 == bytes.len() as u64;
        return Ok(json!({"status":if passed {"passed"} else {"failed"},"observer":"runtime_owned_artifact_readback",
            "observed":{"sha256":digest,"bytes":bytes.len()},"document":snapshot["document"]}));
    }
    let state = app.semantic.lock().await;
    let state = state.get(&tab_key(&args.session_id, &args.chrome_tab_id)).ok_or_else(|| invalid("semantic snapshot state unavailable"))?;
    let (passed, observed, complete) = match &args.predicate {
        BrowserPredicate::Exists { reference } => (state.items.contains_key(reference), json!(state.items.get(reference)), true),
        BrowserPredicate::Text { reference, equals } => {
            let item = state.items.get(reference).filter(|item| semantic_text_is_authoritative(item));
            let actual = item.and_then(|v| v.get("name")).and_then(Value::as_str);
            (semantic_string_field_equals(item, "name", equals), json!(actual), true)
        }
        BrowserPredicate::TextPresent { scope, text } => {
            if text.is_empty() || text.len() > 4096 { return Err(invalid("verification text must contain 1..4096 bytes")); }
            let matched = if let Some(reference) = scope {
                state.items.get(reference).is_some_and(|item| semantic_text_is_authoritative(item) && item.get("text").or_else(|| item.get("name")).and_then(Value::as_str).is_some_and(|value| value.contains(text)))
            } else {
                state.items.values().any(|item| semantic_text_is_authoritative(item) && item.get("text").or_else(|| item.get("name")).and_then(Value::as_str).is_some_and(|value| value.contains(text)))
            };
            let complete = scope.is_some() || snapshot["scope_complete"] == true;
            (matched, json!({"text_present":matched,"scope":scope}), complete)
        }
        BrowserPredicate::Attribute { reference, name, equals } => {
            if !matches!(name.as_str(), "href" | "datetime" | "role" | "selected" | "expanded") {
                return Err(invalid("attribute verification is limited to href, datetime, role, selected, or expanded"));
            }
            let item = state.items.get(reference);
            let actual = item.and_then(|item| item.get(name)).cloned().unwrap_or(Value::Null);
            (semantic_string_field_equals(item, name, equals), actual, true)
        }
        BrowserPredicate::CountDelta { role, before, delta } => {
            if role.is_empty() || role.len() > 64 { return Err(invalid("count_delta role must be 1..64 bytes")); }
            let (matched, actual) = semantic_count_delta(&state.items, role, *before, *delta);
            (matched, json!({"role":role,"count":actual,"before":before,"delta":delta}), snapshot["scope_complete"] == true)
        }
        BrowserPredicate::Value { reference, equals } => {
            let item = state.items.get(reference);
            let actual = item.and_then(|v| v.get("raw_value")).and_then(Value::as_str);
            (semantic_string_field_equals(item, "raw_value", equals), json!(actual), true)
        }
        BrowserPredicate::Url { equals } => {
            let actual = snapshot.get("url").and_then(Value::as_str).unwrap_or_default();
            (actual == equals, json!(actual), true)
        }
        BrowserPredicate::UrlStartsWith { prefix } => {
            if prefix.is_empty() { return Err(invalid("URL prefix cannot be empty")); }
            let actual = snapshot.get("url").and_then(Value::as_str).unwrap_or_default();
            (actual.starts_with(prefix), json!(actual), true)
        }
        BrowserPredicate::Attachment { filename, size } => {
            if filename.is_empty() || filename.len() > 512 {
                return Err(invalid("attachment filename must contain 1..512 bytes"));
            }
            let found = state.items.values().find(|item| {
                semantic_text_is_authoritative(item)
                    && matches!(item.get("role").and_then(Value::as_str), Some("attachment" | "listitem" | "option"))
                    && (item.get("filename").and_then(Value::as_str) == Some(filename)
                        || item.get("name").and_then(Value::as_str) == Some(filename))
                    && size.is_none_or(|expected| item.get("size").and_then(Value::as_u64) == Some(expected))
            });
            (found.is_some(), json!({"filename":filename,"size":found.and_then(|item|item.get("size")).cloned()}), snapshot["scope_complete"] == true)
        }
        BrowserPredicate::DownloadArtifact { .. } => unreachable!("handled before the semantic snapshot borrow"),
        BrowserPredicate::VisualRegionChanged { .. } => {
            return Ok(json!({"status":"inconclusive","observer":"fresh_shared_page_snapshot","reason":"visual_region_changed requires two same-document runtime probe results supplied to workflow verify","document":snapshot["document"]}));
        }
        BrowserPredicate::AppSpecific { adapter, predicate } => {
            if adapter != "semantic_field_equals" { return Err(invalid("app_specific adapter is not registered")); }
            let reference = predicate["reference"].as_str().ok_or_else(|| invalid("semantic_field_equals requires reference"))?;
            let field = predicate["field"].as_str().ok_or_else(|| invalid("semantic_field_equals requires field"))?;
            let expected = predicate.get("equals").and_then(Value::as_str).ok_or_else(|| invalid("semantic_field_equals requires string equals"))?;
            if !matches!(field, "name" | "role" | "href" | "datetime" | "selected" | "expanded" | "raw_value") || expected.len() > 4096 {
                return Err(invalid("semantic_field_equals field or value is outside its allowlist"));
            }
            let item = state.items.get(reference).filter(|item| semantic_text_is_authoritative(item));
            let actual = item.and_then(|item| item.get(field)).and_then(Value::as_str);
            (actual == Some(expected), json!({"field":field,"value":actual}), true)
        }
    };
    Ok(json!({
        "status": if passed { "passed" } else if complete { "failed" } else { "incomplete" },
        "observer":"fresh_shared_page_snapshot",
        "observed":observed,
        "semantic_revision":snapshot["semantic_revision"],
        "document":snapshot["document"]
    }))
}

fn substitute_reference(reference: &str, vars: &BTreeMap<String, String>) -> Result<String, rmcp::ErrorData> {
    if let Some(name) = reference.strip_prefix('$') {
        vars.get(name).cloned().ok_or_else(|| invalid(format!("workflow variable ${name} is unresolved")))
    } else {
        Ok(reference.to_owned())
    }
}

fn frame_click_receipt(response: Value) -> Value {
    response.pointer("/result/value").or_else(|| response.get("result")).cloned().unwrap_or(response)
}

#[tool_router]
impl AppV2 {
    #[tool(name = "browser_session", description = "Discover, pair, connect, list, or release explicitly selected Chrome targets. This is a compact alias over Controlla's existing authority-preserving session broker.")]
    async fn browser_session(&self, Parameters(args): Parameters<BrowserSessionArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        self.legacy.session(Parameters(SessionArgs {
            action: args.action,
            provider: args.provider,
            session_id: args.session_id,
            target_ids: args.target_ids,
            host_id: args.host_id,
        })).await
    }

    #[tool(name = "browser_snapshot", description = "Return a compact semantic snapshot for an explicitly paired Chrome tab. Controls report present/rendered/in_viewport separately; in_viewport is only a candidate, and mutations still require actionability checks. Completeness reports text, controls, frames, and limits. Set frame_id to request a bounded frame-aware accessibility snapshot for a child frame; returned @f refs carry frame identity.")]
    async fn browser_snapshot(&self, Parameters(args): Parameters<BrowserSnapshotArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(snapshot_impl(self, &args).await?))
    }

    #[tool(name = "browser_find", description = "Find controls in the current compact semantic snapshot by name, role, value, or observed href. Returns exact @cN refs; if no snapshot exists, one is captured first.")]
    async fn browser_find(&self, Parameters(args): Parameters<BrowserFindArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(find_impl(self, &args).await?))
    }

    #[tool(name = "browser_act", description = "Act on an exact @cN reference. click reuses Controlla's single-use guarded click semantics. fill/select use the retained DOM identity directly, consume the reference before dispatch, verify local readback, and return unknown rather than retry when delivery or final document state is uncertain.")]
    async fn browser_act(&self, Parameters(args): Parameters<BrowserActArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        let reference = args.reference.as_deref().ok_or_else(|| invalid("reference is required"))?;
        let result = match args.action.as_str() {
            "click" => {
                if let Some(BrowserClickOutcome::Navigation { url: Some(url) } | BrowserClickOutcome::NewTab { url: Some(url) }) = args.outcome.as_ref() {
                    super::validate_shared_url(url)?;
                }
                if let Some(frame_ref) = reference.strip_prefix("@f:") {
                    let mut parts = frame_ref.splitn(3, ':');
                    let frame_id = parts.next().unwrap_or_default();
                    let loader_id = parts.next().unwrap_or_default();
                    let backend = parts.next().unwrap_or_default();
                    let backend_node_id = backend.parse::<i64>().ok().filter(|id| *id > 0).ok_or_else(|| invalid("frame reference has an invalid backend node id"))?;
                    if frame_id.is_empty() || frame_id.len() > 256 || loader_id.is_empty() || loader_id.len() > 256 || args.outcome.is_none() {
                        return Err(invalid("frame click requires a valid frame identity and explicit outcome"));
                    }
                    let outcome = args.outcome.as_ref().ok_or_else(|| invalid("frame click requires an explicit outcome"))?;
                    let shared = self.legacy.shared_sessions.lock().await.get(&args.session_id).cloned().ok_or_else(||invalid("unknown shared session_id"))?;
                    let shared = shared.lock().await;
                    if shared.handle.principal != self.legacy.principal.as_ref() { return Err(invalid("session is not owned by this server principal")); }
                    let connection = shared.connection.as_ref().ok_or_else(||invalid("shared extension session has not been accepted"))?;
                    let response = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Controlla.frameClick", serde_json::json!({"frame_id":frame_id,"loader_id":loader_id,"backend_node_id":backend_node_id,"outcome":outcome,"timeout_ms":args.timeout_ms.unwrap_or(5000).clamp(250,10000)})).await;
                    match response {
                        Ok(response) => {
                            let value = frame_click_receipt(response);
                            let status = if value["verified"] == true { "verified" } else if value["dispatch_may_have_occurred"] == false { "not_dispatched" } else { "unknown" };
                            json!({"status":status,"dispatch_acknowledged":value["dispatch_acknowledged"],"dispatch_may_have_occurred":value["dispatch_may_have_occurred"],"verified":value["verified"] == true,"reference":reference,"source_frame_id":frame_id,"requested_outcome":outcome,"reason":value["reason"],"dispatch":value})
                        }
                        Err(error) => json!({"status":"unknown","dispatch_acknowledged":null,"dispatch_may_have_occurred":true,"verified":false,"reference":reference,"source_frame_id":frame_id,"requested_outcome":outcome,"reason":format!("frame click delivery or readback became uncertain; inspect the frame before retrying: {error}")}),
                    }
                } else {
                let outcome = args.outcome.clone().ok_or_else(|| invalid("click requires an explicit bounded outcome"))?;
                retained_click(self, &args, reference, outcome).await?
                }
            }
            "fill" | "select" | "slider" => retained_mutation(self, &args, reference, false, &args.action).await?,
            _ => return Err(invalid("action must be click, fill, select, or slider")),
        };
        Ok(Json(result))
    }

    #[tool(name = "browser_extract", description = "Run a bounded structured extraction from one explicitly paired Chrome tab while requiring the root frame, loader, and URL to remain stable across the read.")]
    async fn browser_extract(&self, Parameters(args): Parameters<BrowserExtractArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
    let relevance_goal = args.goal.clone();
    let result = self.legacy.shared_observe(Parameters(SharedObserveArgs {
            session_id: args.session_id,
            chrome_tab_id: args.chrome_tab_id,
            spec: ObserveInput {
                selector: args.selector,
                fields: args.fields,
                max_items: args.max_items.unwrap_or(100).clamp(1, 500),
                max_text_chars: args.max_text_chars.unwrap_or(10_000).clamp(1, 10_000),
                max_bytes: args.max_bytes.unwrap_or(200_000).clamp(4096, 1_000_000),
                cursor: None,
            },
        })).await;
        match result {
            Ok(value) => {
                let mut value = value.0;
                if let Some(items) = value["items"].as_array().cloned() {
                    value["items"] = json!(crate::relevance::filter_items(&items, relevance_goal.as_deref(), args.max_items.unwrap_or(100)));
                }
                crate::relevance::mark_untrusted(&mut value);
                Ok(Json(value))
            },
            Err(error) => match selector_error_receipt(&error.message) {
                Some(receipt) => Ok(Json(receipt)),
                None => Err(error),
            },
        }
    }

    #[tool(name = "browser_probe", description = "Perform a targeted accessibility probe on one CSS-selected node in an explicitly paired Chrome tab. This is a scoped fallback, not a full-page screenshot loop.")]
    async fn browser_probe(&self, Parameters(args): Parameters<BrowserProbeArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        self.legacy.shared_accessibility(Parameters(SharedAccessibilityArgs {
            session_id: args.session_id,
            chrome_tab_id: args.chrome_tab_id,
            selector: Some(args.selector),
            frame_id: None,
            max_bytes: args.max_bytes.unwrap_or(64 * 1024).clamp(4096, 262_144),
            timeout_ms: args.timeout_ms,
        })).await
    }

    #[tool(name = "browser_verify", description = "Independently re-observe the selected tab with a fresh snapshot and evaluate a bounded value/text/existence/URL predicate. Caller-supplied prior evidence is never treated as proof.")]
    async fn browser_verify(&self, Parameters(args): Parameters<BrowserVerifyArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(verify_impl(self, &args).await?))
    }

    #[tool(name = "browser_workflow", description = "Execute up to 20 deterministic snapshot/find/fill/select/click/verify steps with no internal model calls and no loops. Unknown mutation outcomes stop the workflow and are never automatically retried.")]
    async fn browser_workflow(&self, Parameters(args): Parameters<BrowserWorkflowArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        if args.steps.is_empty() || args.steps.len() > MAX_WORKFLOW_STEPS {
            return Err(invalid("workflow must contain 1..20 steps"));
        }
        let mut vars = BTreeMap::<String, String>::new();
        let mut receipts = Vec::new();
        for step in args.steps {
            let receipt = match step {
                BrowserWorkflowStep::Snapshot => snapshot_impl(self, &BrowserSnapshotArgs {
                    session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), selector: None,
                    max_items: None, max_text_chars: None, max_bytes: None, href_hint: None, frame_id: None,
            goal: None,
            max_scan_nodes: None,
                }).await?,
                BrowserWorkflowStep::Find { query, role, save_as } => {
                    if save_as.is_empty() || save_as.len() > 64 || !save_as.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        return Err(invalid("workflow save_as must be 1..64 ASCII alphanumeric/underscore characters"));
                    }
                    let found = find_impl(self, &BrowserFindArgs {
                        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), query, role, limit: Some(2), max_matches: None, match_mode: None, within_reference: None, href_hint: None,
                    }).await?;
                    let matches = found["matches"].as_array().cloned().unwrap_or_default();
                    if matches.len() != 1 {
                        return Err(invalid(format!("workflow find for {save_as} must resolve exactly one control")));
                    }
                    let reference = matches[0]["reference"].as_str().ok_or_else(|| invalid("workflow find omitted reference"))?.to_owned();
                    vars.insert(save_as.clone(), reference.clone());
                    json!({"kind":"find","save_as":save_as,"reference":reference,"match":matches[0]})
                }
                BrowserWorkflowStep::Fill { reference, expected_value, value } => {
                    let reference = substitute_reference(&reference, &vars)?;
                    retained_mutation(self, &BrowserActArgs {
                        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action: "fill".into(),
                        reference: Some(reference.clone()), expected_value: Some(expected_value), value: Some(value), outcome: None, timeout_ms: None,
                    }, &reference, false, "fill").await?
                }
                BrowserWorkflowStep::Select { reference, expected_value, value } => {
                    let reference = substitute_reference(&reference, &vars)?;
                    retained_mutation(self, &BrowserActArgs {
                        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action: "select".into(),
                        reference: Some(reference.clone()), expected_value: Some(expected_value), value: Some(value), outcome: None, timeout_ms: None,
                    }, &reference, false, "select").await?
                }
                BrowserWorkflowStep::Click { reference, outcome } => {
                    let reference = substitute_reference(&reference, &vars)?;
                    let legacy_reference = resolve_reference(self, &args.session_id, &args.chrome_tab_id, &reference).await?;
                    let outcome: SharedClickOutcome = serde_json::from_value(serde_json::to_value(outcome).map_err(|e| invalid(e.to_string()))?)
                        .map_err(|e| invalid(e.to_string()))?;
                    let Json(value) = self.legacy.shared_click(Parameters(SharedClickArgs {
                        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), reference: legacy_reference, outcome, timeout_ms: None,
                    })).await?;
                    value
                }
                BrowserWorkflowStep::Verify { predicate } => verify_impl(self, &BrowserVerifyArgs {
                    session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), predicate,
                }).await?,
            };
            let status = receipt
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned);
            receipts.push(receipt);
            if matches!(
                status.as_deref(),
                Some("unknown" | "failed" | "not_dispatched")
            ) {
                break;
            }
        }
        let terminal = receipts.last().and_then(|r| r.get("status")).and_then(Value::as_str).unwrap_or("completed");
        Ok(Json(json!({
            "status": if terminal == "unknown" { "unknown" } else if terminal == "failed" || terminal == "not_dispatched" { "failed" } else { "completed" },
            "completed_steps": receipts.len(),
            "internal_model_calls": 0,
            "steps": receipts
        })))
    }

    #[tool(name = "browser_skill", description = "Describe the reusable-skill contract exposed by the v2 engine. Skill execution remains fail-closed until a candidate has independent training/validation qualification in the production skill store.")]
    async fn browser_skill(&self, Parameters(args): Parameters<BrowserSkillArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        match args.action.as_str() {
            "describe" => Ok(Json(json!({
                "status":"available",
                "qualification":["distinct_training_success","distinct_validation_success","independent_runtime_verification"],
                "replay_policy":"zero internal model calls on a current qualified hit; quarantine on drift or failed verification",
                "safety":"skill replay never bypasses principal/session/target/document/authority checks"
            }))),
            "status" => Ok(Json(json!({
                "status":"not_found",
                "skill_id":args.skill_id,
                "reason":"no v2 persistent skill store has been registered for this process yet"
            }))),
            _ => Err(invalid("browser_skill action must be describe or status")),
        }
    }
}

#[tool_handler]
impl ServerHandler for AppV2 {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "controlla-browser-v2",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Pair only explicitly selected tabs. Prefer browser_snapshot/browser_find/browser_act. Treat unknown effects as non-retriable until a fresh observation resolves state. Use browser_probe only as a scoped fallback.".to_owned(),
        )
    }
}

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let state_dir = state_directory();
    std::fs::create_dir_all(&state_dir)?;
    let _state_lock = acquire_state_directory_lock(&state_dir)?;
    let journal = crate::jobs::Journal::open(state_dir.join("operations.sqlite"))?;
    journal.recover_after_restart()?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async {
        AppV2::new(App::with_journal(journal))
            .serve(rmcp::transport::io::stdio())
            .await?
            .waiting()
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

fn selector_error_receipt(message: &str) -> Option<Value> {
    let (field, reason) = message.split_once("selector_error:")?.1.split_once(':')?;
    if field.is_empty() || !matches!(reason, "invalid_selector" | "no_match" | "ambiguous_match" | "unsupported_target_type") {
        return None;
    }
    Some(json!({"status":"not_dispatched","field":field,"reason_code":reason}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_verifier_predicates_are_exposed_with_typed_inputs() {
        let download: BrowserVerifyArgs = serde_json::from_value(json!({"session_id":"s","chrome_tab_id":"t","predicate":{"kind":"download_artifact","artifact_handle":"opaque","sha256":"a".repeat(64),"min_bytes":12}})).unwrap();
        assert!(matches!(download.predicate, BrowserPredicate::DownloadArtifact { min_bytes: 12, .. }));
        let attachment: BrowserVerifyArgs = serde_json::from_value(json!({"session_id":"s","chrome_tab_id":"t","predicate":{"kind":"attachment","filename":"report.pdf","size":42}})).unwrap();
        assert!(matches!(attachment.predicate, BrowserPredicate::Attachment { size: Some(42), .. }));
        let app_specific: BrowserVerifyArgs = serde_json::from_value(json!({"session_id":"s","chrome_tab_id":"t","predicate":{"kind":"app_specific","adapter":"semantic_field_equals","predicate":{"reference":"@c1","field":"selected","equals":"true"}}})).unwrap();
        assert!(matches!(app_specific.predicate, BrowserPredicate::AppSpecific { .. }));
    }

    #[test]
    fn document_reset_never_reuses_prior_semantic_reference_numbers() {
        let mut state = SemanticTabState {
            document_key: "old-document".into(),
            next_ref: 9,
            ..Default::default()
        };
        reset_semantic_document(&mut state, "new-document", &json!({
            "document":{"frame_id":"frame-2","loader_id":"loader-2","url":"https://example.test/new"}
        }));
        assert_eq!(state.document_key, "new-document");
        assert_eq!(state.next_ref, 9);
        assert!(state.key_to_short.is_empty());
    }

    #[test]
    fn extraction_selector_failures_are_typed_and_field_bound() {
        let receipt = selector_error_receipt("browser response was invalid: selector_error:announcement:invalid_selector").unwrap();
        assert_eq!(receipt["status"], "not_dispatched");
        assert_eq!(receipt["field"], "announcement");
        assert_eq!(receipt["reason_code"], "invalid_selector");
        assert!(selector_error_receipt("selector_error:body:browser_crash").is_none());
        assert!(selector_error_receipt("unrelated failure").is_none());
    }

    #[test]
    fn missing_or_non_string_semantic_fields_never_match_empty_expected_values() {
        assert!(!semantic_string_field_equals(None, "name", ""));
        assert!(!semantic_string_field_equals(Some(&json!({"name": null})), "name", ""));
        assert!(semantic_string_field_equals(Some(&json!({"name": ""})), "name", ""));
    }

    #[test]
    fn boolean_semantic_attributes_match_string_predicates() {
        assert!(semantic_string_field_equals(Some(&json!({"selected": true})), "selected", "true"));
        assert!(semantic_string_field_equals(Some(&json!({"expanded": false})), "expanded", "false"));
        assert!(!semantic_string_field_equals(Some(&json!({"selected": true})), "selected", "false"));
        assert!(!semantic_string_field_equals(Some(&json!({"selected": null})), "selected", "true"));
    }

    #[test]
    fn repeated_semantic_controls_raise_observation_ambiguity() {
        let unique = [json!({"role":"button","name":"Submit"}), json!({"role":"link","name":"Help"})];
        let ambiguous = [json!({"role":"button","name":"Open"}), json!({"role":"button","name":"Open"}), json!({"role":"button","name":"Save"})];
        assert_eq!(semantic_ambiguity(unique.iter()), 0.0);
        let score = semantic_ambiguity(ambiguous.iter());
        assert!((score - 1.0 / 3.0).abs() < f32::EPSILON);
        assert_eq!(
            crate::observation_router::choose_observation(
                crate::observation_router::ObservationRequest { ambiguity: score, ..Default::default() }
            ).mode,
            crate::observation_router::ObservationMode::SemanticRich,
        );
        assert_eq!(semantic_ambiguity(std::iter::empty()), 0.0);
    }

    #[test]
    fn scoped_html_expression_quotes_scope_and_redacts_executable_or_input_content() {
        let expression = scoped_html_expression("main[data-x=\"a'\"]", 50_000);
        assert!(expression.contains("main[data-x=\\\"a'\\\"]"));
        assert!(expression.contains("script,style,noscript,iframe,object,embed,svg,input,textarea,select,button"));
        assert!(expression.contains("32_768") || expression.contains("32768"));
        assert!(query_needs_layout("read the chart labels"));
        assert!(!query_needs_layout("summarize this paragraph"));
    }

    #[test]
    fn observation_metadata_reports_the_serialized_payload_size() {
        let mut value = json!({"items":[{"name":"fixture"}]});
        let bytes = attach_observation_metadata(
            &mut value,
            crate::observation_router::ObservationDecision {
                mode: crate::observation_router::ObservationMode::SemanticCompact,
                reason: "test",
            },
        );
        assert_eq!(bytes as usize, serde_json::to_vec(&value).unwrap().len());
        assert_eq!(value["observation"]["bytes"], json!(bytes));
    }

    #[test]
    fn semantic_count_delta_uses_live_counts_and_rejects_underflow() {
        let items = BTreeMap::from([
            ("a".into(), json!({"role":"row"})),
            ("b".into(), json!({"role":"row"})),
            ("c".into(), json!({"role":"button"})),
        ]);
        assert_eq!(semantic_count_delta(&items, "row", 1, 1), (true, 2));
        assert_eq!(semantic_count_delta(&items, "row", 1, -1), (false, 2));
        assert_eq!(semantic_count_delta(&items, "row", 0, -1), (false, 2));
    }

    #[test]
    fn success_toast_and_live_regions_cannot_satisfy_page_outcome_text() {
        assert!(!semantic_text_is_authoritative(&json!({"role":"status","text":"Saved successfully"})));
        assert!(!semantic_text_is_authoritative(&json!({"role":"generic","feedback_region":true,"text":"Saved successfully"})));
        assert!(semantic_text_is_authoritative(&json!({"role":"heading","feedback_region":false,"text":"Saved record"})));
    }

    #[test]
    fn frame_click_keeps_the_native_receipt_when_transport_has_unwrapped_it() {
        let receipt = json!({"dispatch_acknowledged":true,"dispatch_may_have_occurred":true,"verified":true});
        assert_eq!(frame_click_receipt(receipt.clone()), receipt);
        assert_eq!(frame_click_receipt(json!({"result":{"value":receipt}})), receipt);
    }

    #[test]
    fn page_order_survives_two_digit_reference_keys() {
        let mut state = SemanticTabState::default();
        state.items.insert("@c10".into(), json!({"reference":"@c10","index":9}));
        state.items.insert("@c2".into(), json!({"reference":"@c2","index":1}));
        assert_eq!(state.page_ordered_items()[0]["reference"], "@c2");
    }

    #[test]
    fn stable_keys_disambiguate_duplicate_controls_without_using_mutable_values() {
        let items = vec![
            json!({"role":"textbox","name":"Email","raw_value":"a"}),
            json!({"role":"textbox","name":"Email","raw_value":"b"}),
        ];
        let keys = crate::semantic_state::stable_item_keys(&items, "s1");
        assert_ne!(keys[0], keys[1]);
        let changed = vec![
            json!({"role":"textbox","name":"Email","raw_value":"new-a"}),
            json!({"role":"textbox","name":"Email","raw_value":"new-b"}),
        ];
        assert_ne!(keys, crate::semantic_state::stable_item_keys(&changed, "s2"));
    }

    #[test]
    fn workflow_variable_substitution_fails_closed() {
        let vars = BTreeMap::from([("email".to_owned(), "@c2".to_owned())]);
        assert_eq!(substitute_reference("$email", &vars).unwrap(), "@c2");
        assert!(substitute_reference("$missing", &vars).is_err());
        assert_eq!(substitute_reference("@c1", &vars).unwrap(), "@c1");
    }

    #[test]
    fn exact_find_does_not_claim_uniqueness_from_partial_scope_or_duplicates() {
        let unique = vec![json!({"confidence":"high"})];
        assert!(find_is_ambiguous("exact", false, &unique));
        assert!(!find_is_ambiguous("exact", true, &unique));
        assert!(find_is_ambiguous("exact", true, &[unique[0].clone(), unique[0].clone()]));
        assert!(!find_is_ambiguous("all", false, &unique));
    }

    #[test]
    fn click_reacquisition_only_follows_a_typed_pre_dispatch_target_change() {
        assert!(click_needs_reacquisition(&json!({"status":"not_dispatched","reason_code":"changed_target"})));
        assert!(!click_needs_reacquisition(&json!({"status":"unknown","reason_code":"changed_target"})));
        assert!(!click_needs_reacquisition(&json!({"status":"not_dispatched","reason_code":"other"})));
    }
}
