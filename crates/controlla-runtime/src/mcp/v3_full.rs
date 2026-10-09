use super::{
    AppV2, BrowserActArgs, BrowserClickOutcome, BrowserExtractArgs, BrowserFindArgs, BrowserPredicate,
    BrowserSessionArgs, BrowserSnapshotArgs, BrowserVerifyArgs, find_impl, resolve_reference,
    retained_mutation, snapshot_impl, verify_impl,
};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::{Json, Parameters},
    tool, tool_handler, tool_router,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio::sync::Mutex;

#[derive(Clone)]
struct AppV3 {
    core: AppV2,
    leases: Arc<Mutex<crate::v3::LeaseTable>>,
}

impl AppV3 {
    fn new(core: AppV2) -> Self {
        Self { core, leases: Arc::new(Mutex::new(crate::v3::LeaseTable::default())) }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3BrowserArgs {
    action: String,
    provider: Option<String>,
    session_id: Option<String>,
    target_ids: Option<Vec<String>>,
    host_id: Option<String>,
    mode: Option<String>,
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
    key: Option<String>,
    outcome: Option<BrowserClickOutcome>,
    timeout_ms: Option<u64>,
    typing_mode: Option<String>,
    delay_ms: Option<u64>,
    client_id: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3WorkflowArgs {
    session_id: String,
    chrome_tab_id: String,
    steps: Vec<V3WorkflowStep>,
    client_id: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum V3WorkflowStep {
    Snapshot,
    Find { query: String, role: Option<String>, save_as: String },
    Click { reference: String, outcome: Option<BrowserClickOutcome> },
    Fill { reference: String, expected_value: String, value: String },
    Type { reference: String, expected_value: String, value: String, typing_mode: Option<String>, delay_ms: Option<u64> },
    Press { reference: String, key: String },
    Extract { selector: String },
    Verify { predicate: BrowserPredicate },
}

fn invalid(message: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(message.into(), None)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn fast_key_batch_failure(
    error: &str,
    completed_actions: u64,
    prior_key_events: u64,
) -> (&'static str, bool) {
    let definitely_not_dispatched = error.contains("read guard rejected")
        && completed_actions == 1
        && prior_key_events == 0;
    if definitely_not_dispatched {
        ("not_dispatched", false)
    } else {
        ("unknown", prior_key_events > 0 || completed_actions > 1)
    }
}

fn workflow_should_stop(value: &Value) -> bool {
    matches!(value.get("status").and_then(Value::as_str), Some("unknown" | "not_dispatched"))
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

fn parse_typing_mode(mode: Option<&str>) -> Result<crate::v3::TypingMode, rmcp::ErrorData> {
    Ok(match mode.unwrap_or("block") {
        "block" => crate::v3::TypingMode::Block,
        "fast_keys" => crate::v3::TypingMode::FastKeys,
        "human_keys" => crate::v3::TypingMode::HumanKeys,
        "ime" => crate::v3::TypingMode::Ime,
        _ => return Err(invalid("typing_mode must be block, fast_keys, human_keys, or ime")),
    })
}

async fn acquire_lease(app: &AppV3, args: &V3ActArgs) -> Result<crate::v3::TargetKey, rmcp::ErrorData> {
    let owner = args.client_id.as_deref().unwrap_or("local-mcp");
    let key = lease_target_key(&args.session_id, &args.chrome_tab_id);
    app.leases.lock().await.acquire(key.clone(), owner, now_ms(), 120_000)
        .map_err(|lease| invalid(format!("target is leased by {}; retry after observing fresh state", lease.owner)))?;
    Ok(key)
}

async fn release_lease(app: &AppV3, args: &V3ActArgs, key: &crate::v3::TargetKey) {
    let owner = args.client_id.as_deref().unwrap_or("local-mcp");
    let _ = app.leases.lock().await.release(key, owner);
}

async fn block_type(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
    let reference = args.reference.as_deref().ok_or_else(|| invalid("type requires reference"))?;
    let expected = args.expected_value.clone().unwrap_or_default();
    let appended = args.value.clone().unwrap_or_default();
    if appended.is_empty() || expected.len().saturating_add(appended.len()) > 16_384 {
        return Err(invalid("type value must be nonempty and final value <= 16384 bytes"));
    }
    let mut final_value = expected.clone();
    final_value.push_str(&appended);
    retained_mutation(&app.core, &BrowserActArgs {
        session_id: args.session_id.clone(),
        chrome_tab_id: args.chrome_tab_id.clone(),
        action: "fill".into(),
        reference: Some(reference.to_owned()),
        expected_value: Some(expected),
        value: Some(final_value),
        outcome: None,
        timeout_ms: args.timeout_ms,
    }, reference, "fill").await.map(|mut value| {
        value["action"] = json!("type");
        value["typing_mode"] = json!("block");
        value
    })
}

async fn ime_type(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
    let reference = args.reference.as_deref().ok_or_else(|| invalid("type requires reference"))?;
    let expected = args.expected_value.as_deref().unwrap_or_default();
    let text = args.value.as_deref().unwrap_or_default();
    if text.is_empty() || expected.len().saturating_add(text.len()) > 16_384 {
        return Err(invalid("IME text must be nonempty and final value <= 16384 bytes"));
    }
    let legacy_reference = resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
    let live = app.core.legacy.shared_sessions.lock().await.get(&args.session_id).cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = live.lock().await;
    if shared.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let snapshot = shared.snapshots.get(&args.chrome_tab_id).cloned()
        .ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token, index) = legacy_reference.rsplit_once(':').ok_or_else(|| invalid("invalid retained reference"))?;
    let index = index.parse::<usize>().map_err(|_| invalid("invalid retained reference index"))?;
    if token != snapshot.token || index >= snapshot.count || snapshot.consumed {
        return Err(invalid("stale or consumed reference; take one fresh browser_snapshot"));
    }
    let identity = {
        let connection = shared.connection.as_ref().ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        super::super::shared_frame_identity(connection, shared.registry()?, &shared.handle, &args.chrome_tab_id)
            .await.map_err(invalid)?
    };
    if identity != snapshot.identity {
        return Err(invalid("document changed since snapshot; take a fresh browser_snapshot"));
    }
    let guard = r#"function(index,expected){const e=this.nodes[index];return !!e&&e.isConnected&&(e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type))&&!e.disabled&&!e.readOnly&&document.activeElement===e&&e.value===expected&&e.selectionStart===e.selectionEnd&&e.selectionEnd===e.value.length;}"#;
    let initial = {
        let connection = shared.connection.as_ref().unwrap();
        connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
            "objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":expected}],"returnByValue":true
        })).await.map_err(|error| invalid(error.to_string()))?
    };
    if super::super::shared_value(&initial).map_err(invalid)? != json!(true) {
        return Ok(json!({"status":"not_dispatched","action":"type","typing_mode":"ime","reason":"field, focus, or caret changed before composition"}));
    }
    shared.snapshots.get_mut(&args.chrome_tab_id).unwrap().consumed = true;
    let connection = shared.connection.as_ref().unwrap();
    let final_value = format!("{expected}{text}");
    let composition_len = text.encode_utf16().count();
    if let Err(error) = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.imeSetComposition", json!({
        "text":text,"selectionStart":composition_len,"selectionEnd":composition_len
    })).await {
        return Ok(json!({"status":"unknown","action":"type","typing_mode":"ime","reason":format!("IME composition delivery is unknown; inspect state and do not retry: {error}")}));
    }
    let composed = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
        "objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":final_value}],"returnByValue":true
    })).await;
    if !matches!(composed, Ok(ref value) if super::super::shared_value(value).is_ok_and(|value| value == true)) {
        let _ = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.imeSetComposition", json!({"text":"","selectionStart":0,"selectionEnd":0})).await;
        return Ok(json!({"status":"unknown","action":"type","typing_mode":"ime","dispatch_acknowledged":true,
            "reason":"IME composition changed target or document state; composition cancellation was attempted; inspect before retrying"}));
    }
    if let Err(error) = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.insertText", json!({"text":text})).await {
        return Ok(json!({"status":"unknown","action":"type","typing_mode":"ime","dispatch_acknowledged":true,
            "reason":format!("IME commit delivery is unknown; inspect state and do not retry: {error}")}));
    }
    let readback = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
        "objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":final_value}],"returnByValue":true
    })).await;
    let after = super::super::shared_frame_identity(connection, shared.registry()?, &shared.handle, &args.chrome_tab_id).await;
    let verified = matches!(readback, Ok(ref value) if super::super::shared_value(value).is_ok_and(|value| value == json!(true)))
        && matches!(after, Ok(ref current) if current == &identity);
    drop(shared);
    if !verified {
        return Ok(json!({"status":"unknown","action":"type","typing_mode":"ime","dispatch_acknowledged":true,
            "reason":"IME commit did not pass retained-node and document readback; inspect before retrying"}));
    }
    Ok(json!({"status":"verified","action":"type","typing_mode":"ime","dispatch_acknowledged":true,"readback":final_value}))
}

async fn key_type(app: &AppV3, args: &V3ActArgs, policy: crate::v3::TypingPolicy) -> Result<Value, rmcp::ErrorData> {
    let reference = args.reference.as_deref().ok_or_else(|| invalid("type requires reference"))?;
    let expected = args.expected_value.as_deref().unwrap_or_default();
    let text = args.value.as_deref().unwrap_or_default();
    if text.is_empty() || expected.len().saturating_add(text.len()) > 16_384 || !text.is_ascii() {
        return Err(invalid("key typing requires nonempty ASCII text and final value <= 16384 bytes"));
    }
    let legacy_reference = resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
    let shared = app.core.legacy.shared_sessions.lock().await.get(&args.session_id).cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = shared.lock().await;
    if shared.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let snapshot = shared.snapshots.get(&args.chrome_tab_id).cloned()
        .ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token, index) = legacy_reference.rsplit_once(':').ok_or_else(|| invalid("invalid retained reference"))?;
    let index = index.parse::<usize>().map_err(|_| invalid("invalid retained reference index"))?;
    if token != snapshot.token || index >= snapshot.count || snapshot.consumed {
        return Err(invalid("stale or consumed reference; take one fresh browser_snapshot"));
    }
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(30_000).clamp(1_000, 60_000));
    let deadline = tokio::time::Instant::now() + timeout;
    let before = {
        let connection = shared.connection.as_ref().ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        tokio::time::timeout_at(deadline, super::super::shared_frame_identity(connection, shared.registry()?, &shared.handle, &args.chrome_tab_id))
            .await.map_err(|_| invalid("type preflight timed out; no key dispatched"))?.map_err(invalid)?
    };
    if before != snapshot.identity {
        return Err(invalid("document changed since snapshot; take a fresh browser_snapshot"));
    }
    let guard_function = r#"function(index,expected){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type))||e.matches(':disabled')||e.readOnly||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};if(document.activeElement!==e||e.selectionStart!==e.selectionEnd||e.selectionEnd!==e.value.length)return {ok:false,reason:'focus_or_selection_changed'};return {ok:true,value:e.value};}"#;
    let mut progress = expected.to_owned();
    let preflight = {
        let connection = shared.connection.as_ref().unwrap();
        let response = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
            "objectId":snapshot.object_id,"functionDeclaration":guard_function,
            "arguments":[{"value":index},{"value":progress}],"returnByValue":true
        })).await.map_err(|error| invalid(error.to_string()))?;
        super::super::shared_value(&response).map_err(invalid)?
    };
    if preflight["ok"] != true {
        return Ok(json!({"status":"not_dispatched","action":"type","reason":preflight["reason"],"readback":preflight}));
    }
    shared.snapshots.get_mut(&args.chrome_tab_id).ok_or_else(|| invalid("snapshot disappeared before dispatch"))?.consumed = true;
    let mut dispatch_count = 0_u64;
    if policy.mode == crate::v3::TypingMode::FastKeys {
        let characters = text.chars().collect::<Vec<_>>();
        for chunk in characters.chunks(16) {
            let mut actions = Vec::with_capacity(chunk.len() * 4);
            for character in chunk {
                actions.push(controlla_browser::providers::SharedBatchAction {
                    method: "Runtime.callFunctionOn".into(),
                    params: json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                        "arguments":[{"value":index},{"value":progress}],"returnByValue":true}),
                    stop_on_not_ok: true,
                });
                let key = character.to_string();
                for params in [
                    json!({"type":"keyDown","key":key}),
                    json!({"type":"char","key":key,"text":key,"unmodifiedText":key}),
                    json!({"type":"keyUp","key":key}),
                ] {
                    actions.push(controlla_browser::providers::SharedBatchAction {
                        method: "Input.dispatchKeyEvent".into(),
                        params,
                        stop_on_not_ok: false,
                    });
                }
                progress.push(*character);
            }
            let connection = shared.connection.as_ref().unwrap();
            let result = connection
                .command_batch(
                    shared.registry()?,
                    &shared.handle,
                    &args.chrome_tab_id,
                    actions,
                )
                .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count > 0,
                    "reason":format!("fast-key batch delivery is unknown; inspect state and do not automatically retry: {error}")})),
            };
            if let Some(error) = result["batch_error"].as_str() {
                let completed = result["completed"].as_u64().unwrap_or(0);
                let (status, acknowledged) =
                    fast_key_batch_failure(error, completed, dispatch_count);
                return Ok(json!({"status":status,"action":"type","dispatch_acknowledged":acknowledged,
                    "batch_receipt":result,"reason":format!("fast-key batch stopped; inspect current state and do not automatically retry: {error}")}));
            }
            if result["completed"].as_u64() != Some((chunk.len() * 4) as u64) {
                return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count > 0,
                    "batch_receipt":result,"reason":"fast-key batch returned an incomplete receipt; inspect state and do not automatically retry"}));
            }
            dispatch_count = dispatch_count.saturating_add((chunk.len() * 3) as u64);
        }
    } else {
      for character in text.chars() {
        let guard = {
            let connection = shared.connection.as_ref().unwrap();
            let response = tokio::time::timeout_at(deadline, connection.command(
                shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn",
                json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                    "arguments":[{"value":index},{"value":progress}],"returnByValue":true})
            )).await;
            match response { Ok(Ok(response)) => super::super::shared_value(&response).ok(), _ => None }
        };
        if guard.as_ref().is_none_or(|value| value["ok"] != true) {
            return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                "reason":"focus, value, selection, target, or document changed during typing; inspect before retrying"}));
        }
        let key = character.to_string();
        let events = [
            json!({"type":"keyDown","key":key}),
            json!({"type":"char","key":key,"text":key,"unmodifiedText":key}),
            json!({"type":"keyUp","key":key}),
        ];
        for event in events {
            let connection = shared.connection.as_ref().unwrap();
            let sent = tokio::time::timeout_at(deadline, connection.command(
                shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.dispatchKeyEvent", event
            )).await;
            if !matches!(sent, Ok(Ok(_))) {
                return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                    "reason":"key dispatch became uncertain; inspect state and do not automatically retry"}));
            }
            dispatch_count = dispatch_count.saturating_add(1);
        }
        progress.push(character);
        if policy.delay_ms > 0
            && tokio::time::timeout_at(
                deadline,
                tokio::time::sleep(Duration::from_millis(policy.delay_ms)),
            )
            .await
            .is_err()
        {
            return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
                "reason":"typing deadline expired after partial dispatch; inspect before retrying"}));
        }
      }
    }
    let readback = {
        let connection = shared.connection.as_ref().unwrap();
        let response = tokio::time::timeout_at(deadline, connection.command(
            shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn",
            json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                "arguments":[{"value":index},{"value":progress}],"returnByValue":true})
        )).await;
        match response { Ok(Ok(response)) => super::super::shared_value(&response).ok(), _ => None }
    };
    let after = {
        let connection = shared.connection.as_ref().unwrap();
        tokio::time::timeout_at(deadline, super::super::shared_frame_identity(connection, shared.registry()?, &shared.handle, &args.chrome_tab_id)).await
    };
    let verified = readback.as_ref().is_some_and(|value| value["ok"] == true)
        && matches!(after, Ok(Ok(ref identity)) if identity == &before);
    drop(shared);
    if !verified {
        return Ok(json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
            "reason":"post-typing readback or document identity did not verify final state; inspect before retrying"}));
    }
    let mut result = json!({"status":"verified","action":"type","dispatch_acknowledged":true,
        "typing_mode":match policy.mode { crate::v3::TypingMode::FastKeys=>"fast_keys", _=>"human_keys" },
        "readback":progress,"cdp_key_events":dispatch_count,"intentional_delay_ms":policy.delay_ms});
    if let Ok(snapshot) = snapshot_impl(&app.core, &BrowserSnapshotArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), selector: None, max_items: None, max_text_chars: None, max_bytes: None }).await {
        result["snapshot"] = snapshot;
    }
    Ok(result)
}

async fn press(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
    let reference = args.reference.as_deref().ok_or_else(|| invalid("press requires reference"))?;
    let key = args.key.as_deref().filter(|key| !key.is_empty() && key.len() <= 64 && !key.chars().any(char::is_control))
        .ok_or_else(|| invalid("press requires a bounded non-control key"))?;
    let legacy_reference = resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
    let shared = app.core.legacy.shared_sessions.lock().await.get(&args.session_id).cloned().ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = shared.lock().await;
    let snapshot = shared.snapshots.get(&args.chrome_tab_id).cloned().ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token, index) = legacy_reference.rsplit_once(':').ok_or_else(|| invalid("invalid retained reference"))?;
    let index = index.parse::<usize>().map_err(|_| invalid("invalid retained reference index"))?;
    if token != snapshot.token || index >= snapshot.count || snapshot.consumed { return Err(invalid("stale or consumed reference")); }
    let connection = shared.connection.as_ref().ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let preflight = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
        "objectId":snapshot.object_id,"functionDeclaration":"function(index){const e=this.nodes[index];return {ok:!!e&&e.isConnected&&document.activeElement===e};}",
        "arguments":[{"value":index}],"returnByValue":true
    })).await.map_err(|error| invalid(error.to_string()))?;
    if super::super::shared_value(&preflight).map_err(invalid)?["ok"] != true {
        return Ok(json!({"status":"not_dispatched","action":"press","reason":"referenced element is not connected and focused"}));
    }
    shared.snapshots.get_mut(&args.chrome_tab_id).unwrap().consumed = true;
    for (index, event) in [json!({"type":"keyDown","key":key}), json!({"type":"keyUp","key":key})].into_iter().enumerate() {
        let connection = shared.connection.as_ref().unwrap();
        if let Err(error) = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Input.dispatchKeyEvent", event).await {
            return Ok(json!({"status":"unknown","action":"press","dispatch_acknowledged":index > 0,
                "reason":format!("press delivery is uncertain; inspect before retrying: {error}")}));
        }
    }
    Ok(json!({"status":"dispatched","action":"press","dispatch_acknowledged":true,"requires_verify":true}))
}

#[tool_router]
impl AppV3 {
    #[tool(description = "Manage one browser/session surface. Supports foreground, background, and headless modes; background never requires window activation. action=describe returns the v3 contract.")]
    async fn browser(&self, Parameters(args): Parameters<V3BrowserArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        if args.action == "describe" {
            return Ok(Json(json!({
                "version":"v3","tools":["browser","snapshot","act","workflow","extract","verify"],
                "modes":{"foreground":crate::v3::BrowserMode::Foreground.policy(),"background":crate::v3::BrowserMode::Background.policy(),"headless":crate::v3::BrowserMode::Headless.policy()},
                "typing_modes":["block","fast_keys","human_keys","ime"],
                "safety":"revision-bound refs, target leases, fresh-state verification, no blind mutation retry"
            })));
        }
        let mode = parse_mode(args.mode.as_deref())?;
        let provider = args.provider.or_else(|| Some(mode.default_provider().to_owned()));
        self.core.browser_session(Parameters(BrowserSessionArgs { action: args.action, provider, session_id: args.session_id, target_ids: args.target_ids, host_id: args.host_id })).await
    }

    #[tool(description = "Return a compact semantic snapshot/delta with revision-bound @cN refs. Use before mutations and again after user/page changes.")]
    async fn snapshot(&self, Parameters(args): Parameters<BrowserSnapshotArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        snapshot_impl(&self.core, &args).await.map(Json)
    }

    #[tool(description = "Perform a guarded mutation. type supports block, fast_keys (real zero-delay CDP key events), human_keys, and ime. Mutations lease the exact target and fail closed on drift.")]
    async fn act(&self, Parameters(mut args): Parameters<V3ActArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        let lease = acquire_lease(self, &args).await?;
        let result = match tokio::time::timeout(Duration::from_secs(60), async {
        match args.action.as_str() {
            "click" => self.core.browser_act(Parameters(BrowserActArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action: "click".into(), reference: args.reference.clone(), expected_value: None, value: None, outcome: args.outcome.take(), timeout_ms: args.timeout_ms })).await.map(|Json(v)| v),
            "fill" | "select" => {
                let reference = args.reference.as_deref().ok_or_else(|| invalid(format!("{} requires reference", args.action)))?;
                retained_mutation(&self.core, &BrowserActArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action: args.action.clone(), reference: args.reference.clone(), expected_value: args.expected_value.clone(), value: args.value.clone(), outcome: None, timeout_ms: args.timeout_ms }, reference, &args.action).await
            }
            "type" => {
                let mode = parse_typing_mode(args.typing_mode.as_deref())?;
                let policy = crate::v3::TypingPolicy::new(mode, args.delay_ms).map_err(invalid)?;
                match mode {
                    crate::v3::TypingMode::Block => block_type(self, &args).await,
                    crate::v3::TypingMode::Ime => ime_type(self, &args).await,
                    crate::v3::TypingMode::FastKeys | crate::v3::TypingMode::HumanKeys => key_type(self, &args, policy).await,
                }
            }
            "press" => press(self, &args).await,
            _ => Err(invalid("action must be click, fill, type, press, or select")),
        }
        }).await {
            Ok(result) => result,
            Err(_) => Ok(json!({"status":"unknown","action":args.action,"dispatch_acknowledged":false,
                "reason":"action exceeded its 60-second bound; inspect state before retrying"})),
        };
        if !matches!(&result, Ok(value) if value["status"] == "unknown") {
            release_lease(self, &args, &lease).await;
        }
        result.map(Json)
    }

    #[tool(description = "Run a bounded deterministic browser program in one MCP call. Supports snapshot/find/click/fill/type/press/extract/verify and rejects unresolved refs or >40 steps.")]
    async fn workflow(&self, Parameters(args): Parameters<V3WorkflowArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        if args.steps.is_empty() || args.steps.len() > 40 { return Err(invalid("workflow requires 1..40 steps")); }
        let mut vars = std::collections::BTreeMap::<String,String>::new();
        let mut receipts = Vec::with_capacity(args.steps.len());
        for step in args.steps {
            let resolve = |reference: String, vars: &std::collections::BTreeMap<String,String>| -> Result<String,rmcp::ErrorData> {
                if let Some(name) = reference.strip_prefix('$') { vars.get(name).cloned().ok_or_else(|| invalid(format!("unresolved workflow variable ${name}"))) } else { Ok(reference) }
            };
            let value = match step {
                V3WorkflowStep::Snapshot => snapshot_impl(&self.core, &BrowserSnapshotArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), selector: None, max_items: None, max_text_chars: None, max_bytes: None }).await?,
                V3WorkflowStep::Find { query, role, save_as } => {
                    let found = find_impl(&self.core, &BrowserFindArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), query, role, limit: Some(10) }).await?;
                    let reference = found.pointer("/matches/0/reference").and_then(Value::as_str).ok_or_else(|| invalid("find returned no reference"))?.to_owned();
                    if save_as.is_empty() || vars.insert(save_as, reference).is_some() { return Err(invalid("save_as must be unique and nonempty")); }
                    found
                }
                V3WorkflowStep::Click { reference, outcome } => {
                    let reference = resolve(reference, &vars)?;
                    self.act(Parameters(V3ActArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action:"click".into(), reference:Some(reference), expected_value:None, value:None, key:None, outcome, timeout_ms:None, typing_mode:None, delay_ms:None, client_id:args.client_id.clone() })).await?.0
                }
                V3WorkflowStep::Fill { reference, expected_value, value } => {
                    let reference = resolve(reference, &vars)?;
                    self.act(Parameters(V3ActArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action:"fill".into(), reference:Some(reference), expected_value:Some(expected_value), value:Some(value), key:None, outcome:None, timeout_ms:None, typing_mode:None, delay_ms:None, client_id:args.client_id.clone() })).await?.0
                }
                V3WorkflowStep::Type { reference, expected_value, value, typing_mode, delay_ms } => {
                    let reference = resolve(reference, &vars)?;
                    self.act(Parameters(V3ActArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action:"type".into(), reference:Some(reference), expected_value:Some(expected_value), value:Some(value), key:None, outcome:None, timeout_ms:None, typing_mode, delay_ms, client_id:args.client_id.clone() })).await?.0
                }
                V3WorkflowStep::Press { reference, key } => {
                    let reference = resolve(reference, &vars)?;
                    self.act(Parameters(V3ActArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), action:"press".into(), reference:Some(reference), expected_value:None, value:None, key:Some(key), outcome:None, timeout_ms:None, typing_mode:None, delay_ms:None, client_id:args.client_id.clone() })).await?.0
                }
                V3WorkflowStep::Extract { selector } => self.core.browser_extract(Parameters(BrowserExtractArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), selector, fields: std::collections::BTreeMap::new(), max_items: Some(100), max_text_chars: Some(6000), max_bytes: Some(100_000) })).await?.0,
                V3WorkflowStep::Verify { predicate } => verify_impl(&self.core, &BrowserVerifyArgs { session_id: args.session_id.clone(), chrome_tab_id: args.chrome_tab_id.clone(), predicate }).await?,
            };
            let stop_status = workflow_should_stop(&value)
                .then(|| value["status"].as_str().unwrap_or("unknown").to_owned());
            receipts.push(value);
            if let Some(status) = stop_status {
                return Ok(Json(json!({"status":status,"steps":receipts.len(),"receipts":receipts})));
            }
        }
        Ok(Json(json!({"status":"completed","steps":receipts.len(),"receipts":receipts})))
    }

    #[tool(description = "Extract bounded structured page data without screenshots.")]
    async fn extract(&self, Parameters(args): Parameters<BrowserExtractArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        self.core.browser_extract(Parameters(args)).await
    }

    #[tool(description = "Independently reobserve the browser and verify a semantic postcondition.")]
    async fn verify(&self, Parameters(args): Parameters<BrowserVerifyArgs>) -> Result<Json<Value>, rmcp::ErrorData> {
        verify_impl(&self.core, &args).await.map(Json)
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
        .with_instructions("Controlla v3 exposes six browser tools. Prefer workflow for multi-step automation, block typing for ordinary text, fast_keys only when real key events are required, and verify after consequential mutations. Background mode must not steal focus. Reobserve after user/page drift; never blindly retry an unknown mutation outcome.")
    }
}

pub(super) async fn run() -> Result<(), String> {
    let state_dir = super::super::state_directory();
    std::fs::create_dir_all(&state_dir).map_err(|error| error.to_string())?;
    let _state_lock = super::super::acquire_state_directory_lock(&state_dir)
        .map_err(|error| error.to_string())?;
    let journal = crate::jobs::Journal::open(state_dir.join("operations.sqlite"))
        .map_err(|error| error.to_string())?;
    journal.recover_after_restart().map_err(|error| error.to_string())?;
    let legacy = super::super::App::with_journal(journal);
    let app = AppV3::new(AppV2::new(legacy));
    let service = app.serve(rmcp::transport::io::stdio()).await.map_err(|error| error.to_string())?;
    service.waiting().await.map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod v3_batch_delivery_tests {
    use super::{fast_key_batch_failure, lease_target_key, workflow_should_stop};
    use serde_json::json;

    #[test]
    fn a_late_guard_failure_is_unknown_after_keys_were_sent() {
        assert_eq!(fast_key_batch_failure("Batch read guard rejected", 1, 0), ("not_dispatched", false));
        assert_eq!(fast_key_batch_failure("Batch read guard rejected", 5, 0), ("unknown", true));
        assert_eq!(fast_key_batch_failure("Batch read guard rejected", 1, 48), ("unknown", true));
    }

    #[test]
    fn workflow_stops_after_uncertain_or_undispatched_actions() {
        assert!(workflow_should_stop(&json!({"status":"unknown"})));
        assert!(workflow_should_stop(&json!({"status":"not_dispatched"})));
        assert!(!workflow_should_stop(&json!({"status":"verified"})));
    }

    #[test]
    fn session_aliases_share_the_same_selected_chrome_tab_lease() {
        assert_eq!(lease_target_key("session-a", "17"), lease_target_key("session-b", "17"));
        assert_ne!(lease_target_key("session-a", "17"), lease_target_key("session-a", "18"));
    }
}
