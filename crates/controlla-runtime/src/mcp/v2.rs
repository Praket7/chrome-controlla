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
    next_ref: u32,
    key_to_short: BTreeMap<String, String>,
    short_to_legacy: BTreeMap<String, String>,
    items: BTreeMap<String, Value>,
    snapshot_id: String,
}

#[derive(Clone)]
struct AppV2 {
    legacy: App,
    semantic: Arc<Mutex<BTreeMap<String, SemanticTabState>>>,
}

impl AppV2 {
    fn new(legacy: App) -> Self {
        Self {
            legacy,
            semantic: Arc::default(),
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
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserFindArgs {
    session_id: String,
    chrome_tab_id: String,
    query: String,
    role: Option<String>,
    limit: Option<usize>,
}

#[derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserClickOutcome {
    Navigation { url: Option<String> },
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

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserPredicate {
    Text { reference: String, equals: String },
    Value { reference: String, equals: String },
    Exists { reference: String },
    Url { equals: String },
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

fn stable_item_keys(items: &[Value]) -> Vec<String> {
    let mut seen = BTreeMap::<String, usize>::new();
    items
        .iter()
        .map(|item| {
            let role = item.get("role").and_then(Value::as_str).unwrap_or_default();
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let href = item.get("href").and_then(Value::as_str).unwrap_or_default();
            let base = format!("{role}\u{1f}{name}\u{1f}{href}");
            let ordinal = seen.entry(base.clone()).or_default();
            let key = format!("{base}\u{1f}{ordinal}");
            *ordinal += 1;
            key
        })
        .collect()
}

fn changed_fields(previous: &Value, current: &Value) -> bool {
    ["role", "name", "raw_value", "href", "expanded", "selected", "disabled", "requires_trusted_events"]
        .into_iter()
        .any(|field| previous.get(field) != current.get(field))
}

async fn snapshot_impl(app: &AppV2, args: &BrowserSnapshotArgs) -> Result<Value, rmcp::ErrorData> {
    let legacy = app
        .legacy
        .shared_snapshot(Parameters(SharedSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: args.selector.clone().unwrap_or_else(snapshot_scope),
            max_items: args.max_items.unwrap_or_else(snapshot_items).clamp(1, 200),
            max_text_chars: args.max_text_chars.unwrap_or_else(snapshot_chars).min(6000),
            max_bytes: args.max_bytes.unwrap_or_else(snapshot_bytes).clamp(4096, 100_000),
        }))
        .await?;
    let Json(mut snapshot) = legacy;
    let items = snapshot
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let item_keys = stable_item_keys(&items);
    let key = tab_key(&args.session_id, &args.chrome_tab_id);
    let doc_key = document_key(&snapshot);
    let snapshot_id = snapshot
        .get("snapshot_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut semantic = app.semantic.lock().await;
    let state = semantic.entry(key).or_default();
    let document_changed = !state.document_key.is_empty() && state.document_key != doc_key;
    if state.document_key != doc_key {
        *state = SemanticTabState {
            document_key: doc_key.clone(),
            next_ref: 1,
            ..Default::default()
        };
    }
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
    Ok(snapshot)
}

async fn resolve_reference(app: &AppV2, session_id: &str, tab_id: &str, reference: &str) -> Result<String, rmcp::ErrorData> {
    if !reference.starts_with("@c") || reference[2..].parse::<u32>().is_err() {
        return Err(invalid("reference must be an exact @cN value returned by browser_snapshot/browser_find"));
    }
    app.semantic
        .lock()
        .await
        .get(&tab_key(session_id, tab_id))
        .and_then(|state| state.short_to_legacy.get(reference))
        .cloned()
        .ok_or_else(|| invalid("reference is stale or unknown; take a fresh browser_snapshot"))
}

async fn retained_mutation(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
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
    if token != snapshot.token || index >= snapshot.count || snapshot.consumed {
        return Err(invalid("stale or consumed reference; take one fresh browser_snapshot"));
    }
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(10_000).clamp(1_000, 30_000));
    let deadline = tokio::time::Instant::now() + timeout;
    let before = {
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
        .map_err(|_| invalid("action preflight timed out; no mutation dispatched"))?
        .map_err(invalid)?
    };
    if before != snapshot.identity {
        return Err(invalid("document changed since snapshot; take a fresh browser_snapshot"));
    }
    let function = if action == "fill" {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)||['password','hidden','file','checkbox','radio','button','submit','reset','image'].includes(e.type||'')||e.matches(':disabled')||e.readOnly||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};const setter=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:value}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,value:e.value};}"#
    } else {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLSelectElement)||e.matches(':disabled')||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};if(![...e.options].some(o=>o.value===value))return {ok:false,reason:'option_missing'};e.value=value;e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,value:e.value};}"#
    };
    // Consume before dispatch so a lost response cannot permit duplicate mutation via the same reference.
    shared
        .snapshots
        .get_mut(&args.chrome_tab_id)
        .ok_or_else(|| invalid("snapshot disappeared before dispatch"))?
        .consumed = true;
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
    if !matches!(after, Ok(Ok(ref identity)) if identity == &before) && result["status"] == "verified" {
        result = json!({
            "status":"unknown","dispatch_acknowledged":true,
            "reason":"document changed after mutation; local readback cannot establish the final page state; inspect before retrying"
        });
    }
    drop(shared);
    if result["status"] == "verified" {
        let fresh = snapshot_impl(app, &BrowserSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: None,
            max_items: None,
            max_text_chars: None,
            max_bytes: None,
        }).await;
        if let Ok(snapshot) = fresh {
            result["snapshot"] = snapshot;
        }
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
        }).await?;
    }
    let query = args.query.to_lowercase();
    let role = args.role.as_deref().map(str::to_lowercase);
    let state = app.semantic.lock().await;
    let state = state.get(&key).ok_or_else(|| invalid("semantic snapshot state unavailable"))?;
    let mut matches = state.items.values().filter_map(|item| {
        let item_role = item.get("role").and_then(Value::as_str).unwrap_or_default();
        if role.as_deref().is_some_and(|wanted| item_role.to_lowercase() != wanted) {
            return None;
        }
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        let raw = item.get("raw_value").and_then(Value::as_str).unwrap_or_default();
        let href = item.get("href").and_then(Value::as_str).unwrap_or_default();
        let haystack = format!("{item_role} {name} {raw} {href}").to_lowercase();
        let score = if name.eq_ignore_ascii_case(&args.query) {
            4
        } else if name.to_lowercase().contains(&query) {
            3
        } else if haystack.contains(&query) {
            2
        } else {
            0
        };
        (score > 0).then(|| (score, item.clone()))
    }).collect::<Vec<_>>();
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let limit = args.limit.unwrap_or(10).clamp(1, 50);
    let matches = matches.into_iter().take(limit).map(|(_, item)| item).collect::<Vec<_>>();
    let count = matches.len();
    Ok(json!({"matches":matches,"count":count,"semantic_revision":state.snapshot_id}))
}

async fn verify_impl(app: &AppV2, args: &BrowserVerifyArgs) -> Result<Value, rmcp::ErrorData> {
    let snapshot = snapshot_impl(app, &BrowserSnapshotArgs {
        session_id: args.session_id.clone(),
        chrome_tab_id: args.chrome_tab_id.clone(),
        selector: None,
        max_items: None,
        max_text_chars: Some(6000),
        max_bytes: Some(60_000),
    }).await?;
    let state = app.semantic.lock().await;
    let state = state.get(&tab_key(&args.session_id, &args.chrome_tab_id)).ok_or_else(|| invalid("semantic snapshot state unavailable"))?;
    let (passed, observed) = match &args.predicate {
        BrowserPredicate::Exists { reference } => (state.items.contains_key(reference), json!(state.items.get(reference))),
        BrowserPredicate::Text { reference, equals } => {
            let item = state.items.get(reference);
            let actual = item.and_then(|v| v.get("name")).and_then(Value::as_str).unwrap_or_default();
            (actual == equals, json!(actual))
        }
        BrowserPredicate::Value { reference, equals } => {
            let item = state.items.get(reference);
            let actual = item.and_then(|v| v.get("raw_value")).and_then(Value::as_str).unwrap_or_default();
            (actual == equals, json!(actual))
        }
        BrowserPredicate::Url { equals } => {
            let actual = snapshot.get("url").and_then(Value::as_str).unwrap_or_default();
            (actual == equals, json!(actual))
        }
    };
    Ok(json!({
        "status": if passed { "passed" } else { "failed" },
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

    #[tool(name = "browser_snapshot", description = "Return a compact visible-page snapshot for an explicitly paired Chrome tab. Actionable controls receive stable short refs such as @c1 for the current document, plus a changed-only delta from the previous snapshot. Refs still resolve to the broker's single-use retained DOM identities.")]
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
                let legacy_reference = resolve_reference(self, &args.session_id, &args.chrome_tab_id, reference).await?;
                let outcome = args.outcome.ok_or_else(|| invalid("click requires an explicit bounded outcome"))?;
                let outcome: SharedClickOutcome = serde_json::from_value(serde_json::to_value(outcome).map_err(|e| invalid(e.to_string()))?)
                    .map_err(|e| invalid(e.to_string()))?;
                let Json(value) = self.legacy.shared_click(Parameters(SharedClickArgs {
                    session_id: args.session_id.clone(),
                    chrome_tab_id: args.chrome_tab_id.clone(),
                    reference: legacy_reference,
                    outcome,
                    timeout_ms: args.timeout_ms,
                })).await?;
                value
            }
            "fill" | "select" => retained_mutation(self, &args, reference, &args.action).await?,
            _ => return Err(invalid("action must be click, fill, or select")),
        };
        Ok(Json(result))
    }

    #[tool(name = "browser_extract", description = "Run a bounded structured extraction from one explicitly paired Chrome tab while requiring the root frame, loader, and URL to remain stable across the read.")]
    async fn browser_extract(&self, Parameters(args): Parameters<BrowserExtractArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        self.legacy.shared_observe(Parameters(SharedObserveArgs {
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
        })).await
    }

    #[tool(name = "browser_probe", description = "Perform a targeted accessibility probe on one CSS-selected node in an explicitly paired Chrome tab. This is a scoped fallback, not a full-page screenshot loop.")]
    async fn browser_probe(&self, Parameters(args): Parameters<BrowserProbeArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        self.legacy.shared_accessibility(Parameters(SharedAccessibilityArgs {
            session_id: args.session_id,
            chrome_tab_id: args.chrome_tab_id,
            selector: args.selector,
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
                    max_items: None, max_text_chars: None, max_bytes: None,
                }).await?,
                BrowserWorkflowStep::Find { query, role, save_as } => {
                    if save_as.is_empty() || save_as.len() > 64 || !save_as.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        return Err(invalid("workflow save_as must be 1..64 ASCII alphanumeric/underscore characters"));
                    }
                    let found = find_impl(self, &BrowserFindArgs {
                        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), query, role, limit: Some(2),
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
                    }, &reference, "fill").await?
                }
                BrowserWorkflowStep::Select { reference, expected_value, value } => {
                    let reference = substitute_reference(&reference, &vars)?;
                    retained_mutation(self, &BrowserActArgs {
                        session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action: "select".into(),
                        reference: Some(reference.clone()), expected_value: Some(expected_value), value: Some(value), outcome: None, timeout_ms: None,
                    }, &reference, "select").await?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_keys_disambiguate_duplicate_controls_without_using_mutable_values() {
        let items = vec![
            json!({"role":"textbox","name":"Email","raw_value":"a"}),
            json!({"role":"textbox","name":"Email","raw_value":"b"}),
        ];
        let keys = stable_item_keys(&items);
        assert_ne!(keys[0], keys[1]);
        let changed = vec![
            json!({"role":"textbox","name":"Email","raw_value":"new-a"}),
            json!({"role":"textbox","name":"Email","raw_value":"new-b"}),
        ];
        assert_eq!(keys, stable_item_keys(&changed));
    }

    #[test]
    fn workflow_variable_substitution_fails_closed() {
        let vars = BTreeMap::from([("email".to_owned(), "@c2".to_owned())]);
        assert_eq!(substitute_reference("$email", &vars).unwrap(), "@c2");
        assert!(substitute_reference("$missing", &vars).is_err());
        assert_eq!(substitute_reference("@c1", &vars).unwrap(), "@c1");
    }
}
