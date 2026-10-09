use super::{
    AppV2, BrowserActArgs, BrowserClickOutcome, BrowserExtractArgs, BrowserFindArgs, BrowserPredicate,
    BrowserSessionArgs, BrowserSnapshotArgs, BrowserVerifyArgs, find_impl, resolve_reference,
    retained_mutation, snapshot_impl, tab_key, verify_impl,
};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::{Json, Parameters},
    tool, tool_handler, tool_router,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_WORKFLOW_STEPS: usize = 20;
const MAX_TEXT: usize = 16_384;
const MAX_SELECTOR: usize = 2_048;
const MAX_PROBE_DIMENSION: f64 = 4_096.0;
const MAX_PROBE_AREA: f64 = 16_777_216.0;
const STRICT_TYPE_DELAY: Duration = Duration::from_millis(60);

#[derive(Clone)]
struct AppFull {
    core: AppV2,
    skills: Arc<crate::skill_runtime::SkillRuntime>,
}

impl AppFull {
    fn new(core: AppV2, skills: crate::skill_runtime::SkillRuntime) -> Self {
        Self {
            core,
            skills: Arc::new(skills),
        }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FullActArgs {
    session_id: String,
    chrome_tab_id: String,
    action: String,
    reference: Option<String>,
    expected_value: Option<String>,
    value: Option<String>,
    key: Option<String>,
    outcome: Option<BrowserClickOutcome>,
    timeout_ms: Option<u64>,
}

#[derive(Clone, Copy, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProbeRegionArgs {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FullProbeArgs {
    session_id: String,
    chrome_tab_id: String,
    reference: Option<String>,
    selector: Option<String>,
    region: Option<ProbeRegionArgs>,
    max_bytes: Option<usize>,
    timeout_ms: Option<u64>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum FullWorkflowStep {
    Snapshot,
    Navigate {
        url: String,
    },
    Find {
        query: String,
        role: Option<String>,
        save_as: String,
    },
    Click {
        reference: String,
        outcome: BrowserClickOutcome,
    },
    Fill {
        reference: String,
        expected_value: String,
        value: String,
    },
    Type {
        reference: String,
        expected_value: String,
        value: String,
    },
    Press {
        reference: String,
        key: String,
    },
    Select {
        reference: String,
        expected_value: String,
        value: String,
    },
    WaitFor {
        selector: String,
        timeout_ms: u64,
    },
    Observe {
        selector: String,
        max_bytes: usize,
    },
    Extract {
        selector: String,
        max_items: usize,
        max_bytes: usize,
    },
    Assert {
        predicate: BrowserPredicate,
    },
    Verify {
        predicate: BrowserPredicate,
    },
    Checkpoint,
    Script {
        source: String,
    },
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FullWorkflowArgs {
    session_id: String,
    chrome_tab_id: String,
    steps: Vec<FullWorkflowStep>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FullSkillArgs {
    action: String,
    skill_id: Option<String>,
    definition: Option<Value>,
    run_id: Option<String>,
    session_id: Option<String>,
    chrome_tab_id: Option<String>,
    predicate: Option<BrowserPredicate>,
    expires_at_ms: Option<u64>,
    severe_safety_failure: Option<bool>,
}

fn invalid(message: impl Into<String>) -> rmcp::ErrorData {
    super::super::invalid(message.into())
}

fn now_ms() -> Result<u64, rmcp::ErrorData> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| invalid(error.to_string()))?
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX))
}

fn require_ref(reference: &Option<String>) -> Result<&str, rmcp::ErrorData> {
    reference
        .as_deref()
        .ok_or_else(|| invalid("reference is required"))
}

fn validate_url(url: &str) -> Result<(), rmcp::ErrorData> {
    if url.len() > 4_096 || url.chars().any(char::is_control) {
        return Err(invalid("URL exceeds bounds"));
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| invalid("URL must use HTTP(S)"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(invalid("URL must contain a host and cannot contain credentials"));
    }
    Ok(())
}

fn validate_reference_expr(reference: &str, vars: &BTreeSet<String>) -> Result<(), rmcp::ErrorData> {
    if let Some(name) = reference.strip_prefix('$') {
        if vars.contains(name) {
            return Ok(());
        }
        return Err(invalid(format!("workflow variable ${name} is unresolved")));
    }
    if let Some(number) = reference.strip_prefix("@c")
        && !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && number.parse::<u32>().is_ok_and(|value| value > 0)
    {
        return Ok(());
    }
    Err(invalid("reference must be an exact @cN value or an earlier $variable"))
}

fn predicate_references(predicate: &BrowserPredicate) -> Option<&str> {
    match predicate {
        BrowserPredicate::Text { reference, .. }
        | BrowserPredicate::Value { reference, .. }
        | BrowserPredicate::Exists { reference } => Some(reference),
        BrowserPredicate::Url { .. } => None,
    }
}

fn validate_workflow(steps: &[FullWorkflowStep]) -> Result<(), rmcp::ErrorData> {
    if steps.is_empty() || steps.len() > MAX_WORKFLOW_STEPS {
        return Err(invalid("workflow must contain 1..20 steps"));
    }
    let script_count = steps
        .iter()
        .filter(|step| matches!(step, FullWorkflowStep::Script { .. }))
        .count();
    if script_count > 0 {
        if script_count != 1 || steps.len() != 1 {
            return Err(invalid("script must be the only workflow step"));
        }
        return Err(invalid(
            "script execution is unavailable on the shared-tab v2 route; use the legacy direct-CDP workflow with explicit trusted-script opt-in",
        ));
    }
    let mut vars = BTreeSet::new();
    for (index, step) in steps.iter().enumerate() {
        match step {
            FullWorkflowStep::Snapshot | FullWorkflowStep::Checkpoint => {}
            FullWorkflowStep::Navigate { url } => {
                validate_url(url)?;
                vars.clear();
            }
            FullWorkflowStep::Find {
                query,
                role,
                save_as,
            } => {
                if query.trim().is_empty() || query.len() > 512 {
                    return Err(invalid("find query must contain 1..512 bytes"));
                }
                if role.as_deref().is_some_and(|role| role.is_empty() || role.len() > 128) {
                    return Err(invalid("find role must contain 1..128 bytes when supplied"));
                }
                if save_as.is_empty()
                    || save_as.len() > 64
                    || !save_as
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    || !vars.insert(save_as.clone())
                {
                    return Err(invalid(
                        "save_as must be a unique 1..64 ASCII alphanumeric/underscore variable",
                    ));
                }
            }
            FullWorkflowStep::Click { reference, .. }
            | FullWorkflowStep::Fill { reference, .. }
            | FullWorkflowStep::Type { reference, .. }
            | FullWorkflowStep::Press { reference, .. }
            | FullWorkflowStep::Select { reference, .. } => {
                validate_reference_expr(reference, &vars)?;
                if matches!(step, FullWorkflowStep::Press { .. })
                    && !matches!(steps.get(index + 1), Some(FullWorkflowStep::Verify { .. }))
                {
                    return Err(invalid(
                        "press must be followed immediately by verify because dispatch acknowledgement is not a semantic postcondition",
                    ));
                }
            }
            FullWorkflowStep::WaitFor {
                selector,
                timeout_ms,
            } => {
                if selector.trim().is_empty()
                    || selector.len() > MAX_SELECTOR
                    || !(1..=30_000).contains(timeout_ms)
                {
                    return Err(invalid("wait_for selector or timeout is out of bounds"));
                }
            }
            FullWorkflowStep::Observe {
                selector,
                max_bytes,
            } => {
                if selector.trim().is_empty()
                    || selector.len() > MAX_SELECTOR
                    || !(4_096..=1_000_000).contains(max_bytes)
                {
                    return Err(invalid("observe bounds are invalid"));
                }
            }
            FullWorkflowStep::Extract {
                selector,
                max_items,
                max_bytes,
            } => {
                if selector.trim().is_empty()
                    || selector.len() > MAX_SELECTOR
                    || !(1..=500).contains(max_items)
                    || !(4_096..=1_000_000).contains(max_bytes)
                {
                    return Err(invalid("extract bounds are invalid"));
                }
            }
            FullWorkflowStep::Assert { predicate } | FullWorkflowStep::Verify { predicate } => {
                if let Some(reference) = predicate_references(predicate) {
                    validate_reference_expr(reference, &vars)?;
                }
            }
            FullWorkflowStep::Script { .. } => unreachable!(),
        }
        match step {
            FullWorkflowStep::Fill {
                expected_value,
                value,
                ..
            }
            | FullWorkflowStep::Type {
                expected_value,
                value,
                ..
            }
            | FullWorkflowStep::Select {
                expected_value,
                value,
                ..
            } if expected_value.len() > MAX_TEXT || value.len() > MAX_TEXT => {
                return Err(invalid("workflow text value exceeds 16384 bytes"));
            }
            FullWorkflowStep::Press { key, .. } if key.is_empty() || key.len() > 64 => {
                return Err(invalid("press key must contain 1..64 bytes"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn substitute(reference: &str, vars: &BTreeMap<String, String>) -> Result<String, rmcp::ErrorData> {
    if let Some(name) = reference.strip_prefix('$') {
        vars.get(name)
            .cloned()
            .ok_or_else(|| invalid(format!("workflow variable ${name} is unresolved")))
    } else {
        Ok(reference.to_owned())
    }
}

fn substitute_predicate(
    predicate: BrowserPredicate,
    vars: &BTreeMap<String, String>,
) -> Result<BrowserPredicate, rmcp::ErrorData> {
    Ok(match predicate {
        BrowserPredicate::Text { reference, equals } => BrowserPredicate::Text {
            reference: substitute(&reference, vars)?,
            equals,
        },
        BrowserPredicate::Value { reference, equals } => BrowserPredicate::Value {
            reference: substitute(&reference, vars)?,
            equals,
        },
        BrowserPredicate::Exists { reference } => BrowserPredicate::Exists {
            reference: substitute(&reference, vars)?,
        },
        BrowserPredicate::Url { equals } => BrowserPredicate::Url { equals },
    })
}

async fn strict_type(app: &AppFull, args: &FullActArgs) -> Result<Value, rmcp::ErrorData> {
    let reference = require_ref(&args.reference)?;
    let expected = args.expected_value.as_deref().unwrap_or_default();
    let text = args.value.as_deref().unwrap_or_default();
    if text.is_empty()
        || expected.len() > MAX_TEXT
        || text.len() > MAX_TEXT
        || !text.chars().all(|character| character.is_ascii_graphic() || character == ' ')
    {
        return Err(invalid(
            "strict type requires nonempty ASCII text and values no longer than 16384 bytes",
        ));
    }
    let legacy_reference = resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
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
    if shared.handle.principal != app.core.legacy.principal.as_ref() {
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
    if token != snapshot.token || index >= snapshot.count || snapshot.consumed {
        return Err(invalid(
            "stale or consumed reference; take one fresh browser_snapshot",
        ));
    }
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(30_000).clamp(1_000, 60_000));
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
    if before != snapshot.identity {
        return Err(invalid(
            "document changed since snapshot; take a fresh browser_snapshot",
        ));
    }
    let guard_function = r#"function(index,expected){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type))||e.matches(':disabled')||e.readOnly||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};if(document.activeElement!==e||e.selectionStart!==e.selectionEnd||e.selectionEnd!==e.value.length)return {ok:false,reason:'typing_requires_focused_end_caret'};return {ok:true,value:e.value};}"#;
    let preflight = {
        let connection = shared.connection.as_ref().unwrap();
        let response = tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId":snapshot.object_id,
                    "functionDeclaration":guard_function,
                    "arguments":[{"value":index},{"value":expected}],
                    "returnByValue":true
                }),
            ),
        )
        .await
        .map_err(|_| invalid("type preflight timed out; no key dispatched"))?
        .map_err(|error| invalid(error.to_string()))?;
        super::super::shared_value(&response).map_err(invalid)?
    };
    if preflight["ok"] != true {
        return Ok(json!({
            "status":"not_dispatched",
            "action":"type",
            "reason":preflight["reason"],
            "readback":preflight
        }));
    }
    shared
        .snapshots
        .get_mut(&args.chrome_tab_id)
        .ok_or_else(|| invalid("snapshot disappeared before dispatch"))?
        .consumed = true;

    let mut expected_progress = expected.to_owned();
    let mut dispatch_count = 0_u64;
    let characters = text.chars().collect::<Vec<_>>();
    for (position, character) in characters.iter().enumerate() {
        let guard = {
            let connection = shared.connection.as_ref().unwrap();
            let response = tokio::time::timeout_at(
                deadline,
                connection.command(
                    shared.registry()?,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId":snapshot.object_id,
                        "functionDeclaration":guard_function,
                        "arguments":[{"value":index},{"value":expected_progress}],
                        "returnByValue":true
                    }),
                ),
            )
            .await;
            match response {
                Ok(Ok(response)) => super::super::shared_value(&response).ok(),
                _ => None,
            }
        };
        if guard.as_ref().is_none_or(|value| value["ok"] != true) {
            return Ok(json!({
                "status":"unknown",
                "action":"type",
                "dispatch_acknowledged":dispatch_count>0,
                "reason":"typing guard changed after at least one retained-state check; inspect current state and do not automatically retry"
            }));
        }
        let key = character.to_string();
        let key_up = json!({"type":"keyUp","key":key});
        for event in [
            json!({"type":"keyDown","key":key}),
            json!({"type":"char","text":key,"unmodifiedText":key}),
        ] {
            let result = {
                let connection = shared.connection.as_ref().unwrap();
                tokio::time::timeout_at(
                    deadline,
                    connection.command(
                        shared.registry()?,
                        &shared.handle,
                        &args.chrome_tab_id,
                        "Input.dispatchKeyEvent",
                        event,
                    ),
                )
                .await
            };
            if !matches!(result, Ok(Ok(_))) {
                if let Some(connection) = shared.connection.as_ref() {
                    let _ = tokio::time::timeout_at(
                        deadline,
                        connection.command(
                            shared.registry()?,
                            &shared.handle,
                            &args.chrome_tab_id,
                            "Input.dispatchKeyEvent",
                            key_up.clone(),
                        ),
                    )
                    .await;
                }
                return Ok(json!({
                    "status":"unknown",
                    "action":"type",
                    "dispatch_acknowledged":dispatch_count>0,
                    "reason":"key dispatch became uncertain; key release was attempted; inspect state and do not automatically retry"
                }));
            }
            dispatch_count = dispatch_count.saturating_add(1);
        }
        let released = {
            let connection = shared.connection.as_ref().unwrap();
            tokio::time::timeout_at(
                deadline,
                connection.command(
                    shared.registry()?,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Input.dispatchKeyEvent",
                    key_up,
                ),
            )
            .await
        };
        if !matches!(released, Ok(Ok(_))) {
            return Ok(json!({
                "status":"unknown",
                "action":"type",
                "dispatch_acknowledged":true,
                "reason":"key release became uncertain; inspect state and do not automatically retry"
            }));
        }
        dispatch_count = dispatch_count.saturating_add(1);
        expected_progress.push(*character);
        if position + 1 < characters.len() {
            if tokio::time::timeout_at(deadline, tokio::time::sleep(STRICT_TYPE_DELAY))
                .await
                .is_err()
            {
                return Ok(json!({
                    "status":"unknown",
                    "action":"type",
                    "dispatch_acknowledged":true,
                    "reason":"typing deadline expired after a partial dispatch; inspect state and do not automatically retry"
                }));
            }
        }
    }
    let readback = {
        let connection = shared.connection.as_ref().unwrap();
        let response = tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId":snapshot.object_id,
                    "functionDeclaration":guard_function,
                    "arguments":[{"value":index},{"value":expected_progress}],
                    "returnByValue":true
                }),
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
    drop(shared);
    if !verified {
        return Ok(json!({
            "status":"unknown",
            "action":"type",
            "dispatch_acknowledged":true,
            "reason":"post-typing retained-node or document readback did not verify final state; inspect before retrying"
        }));
    }
    let mut result = json!({
        "status":"verified",
        "action":"type",
        "dispatch_acknowledged":true,
        "readback":expected_progress,
        "cdp_key_events":dispatch_count
    });
    if let Ok(snapshot) = snapshot_impl(
        &app.core,
        &BrowserSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: None,
            max_items: None,
            max_text_chars: None,
            max_bytes: None,
        },
    )
    .await
    {
        result["snapshot"] = snapshot;
    }
    Ok(result)
}

async fn retained_press(app: &AppFull, args: &FullActArgs) -> Result<Value, rmcp::ErrorData> {
    let reference = require_ref(&args.reference)?;
    let key = args
        .key
        .as_deref()
        .filter(|key| !key.is_empty() && key.len() <= 64 && !key.chars().any(char::is_control))
        .ok_or_else(|| invalid("press requires a bounded non-control key"))?;
    let legacy_reference = resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
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
    if shared.handle.principal != app.core.legacy.principal.as_ref() {
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
        super::super::shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
        .map_err(invalid)?
    };
    if before != snapshot.identity {
        return Err(invalid("document changed since snapshot; take a fresh browser_snapshot"));
    }
    let preflight = {
        let connection = shared.connection.as_ref().unwrap();
        let response = tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId":snapshot.object_id,
                    "functionDeclaration":"function(index){const e=this.nodes[index];return {ok:!!e&&e.isConnected&&document.activeElement===e};}",
                    "arguments":[{"value":index}],
                    "returnByValue":true
                }),
            ),
        )
        .await
        .map_err(|_| invalid("press preflight timed out; no key dispatched"))?
        .map_err(|error| invalid(error.to_string()))?;
        super::super::shared_value(&response).map_err(invalid)?
    };
    if preflight["ok"] != true {
        return Ok(json!({
            "status":"not_dispatched",
            "action":"press",
            "reason":"referenced element is no longer connected and focused"
        }));
    }
    shared
        .snapshots
        .get_mut(&args.chrome_tab_id)
        .ok_or_else(|| invalid("snapshot disappeared before dispatch"))?
        .consumed = true;
    let down = {
        let connection = shared.connection.as_ref().unwrap();
        tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Input.dispatchKeyEvent",
                json!({"type":"keyDown","key":key}),
            ),
        )
        .await
    };
    let up = {
        let connection = shared.connection.as_ref().unwrap();
        tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Input.dispatchKeyEvent",
                json!({"type":"keyUp","key":key}),
            ),
        )
        .await
    };
    if !matches!(down, Ok(Ok(_))) || !matches!(up, Ok(Ok(_))) {
        return Ok(json!({
            "status":"unknown",
            "action":"press",
            "dispatch_acknowledged":false,
            "reason":"key press or release delivery is uncertain; inspect state and do not automatically retry"
        }));
    }
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
    let navigation_observed = matches!(after, Ok(Ok(ref identity)) if identity != &before);
    drop(shared);
    Ok(json!({
        "status":"dispatched",
        "action":"press",
        "dispatch_acknowledged":true,
        "navigation_observed":navigation_observed,
        "verification_required":true
    }))
}

fn probe_region_valid(region: ProbeRegionArgs) -> bool {
    region.x.is_finite()
        && region.y.is_finite()
        && region.width.is_finite()
        && region.height.is_finite()
        && region.x >= 0.0
        && region.y >= 0.0
        && region.width > 0.0
        && region.height > 0.0
        && region.width <= MAX_PROBE_DIMENSION
        && region.height <= MAX_PROBE_DIMENSION
        && region.width * region.height <= MAX_PROBE_AREA
}

fn base64_decoded_len(data: &str) -> Option<usize> {
    if data.is_empty() || !data.len().is_multiple_of(4) {
        return None;
    }
    let padding = usize::from(data.ends_with('=')) + usize::from(data.ends_with("=="));
    data.len().checked_div(4)?.checked_mul(3)?.checked_sub(padding)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

async fn visual_probe(app: &AppFull, args: FullProbeArgs) -> Result<Value, rmcp::ErrorData> {
    let modes = usize::from(args.reference.is_some())
        + usize::from(args.selector.is_some())
        + usize::from(args.region.is_some());
    if modes != 1 {
        return Err(invalid(
            "browser_probe requires exactly one of reference, selector, or region",
        ));
    }
    if args.selector.as_deref().is_some_and(|selector| {
        selector.trim().is_empty() || selector.len() > MAX_SELECTOR || selector.contains('\0')
    }) {
        return Err(invalid("probe selector is out of bounds"));
    }
    if args.region.is_some_and(|region| !probe_region_valid(region)) {
        return Err(invalid("probe region is out of bounds"));
    }
    let max_bytes = args.max_bytes.unwrap_or(262_144).clamp(4_096, 1_000_000);
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(10_000).clamp(1_000, 30_000));
    let deadline = tokio::time::Instant::now() + timeout;
    let shared = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let shared = shared.lock().await;
    if shared.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let before = tokio::time::timeout_at(
        deadline,
        super::super::shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        ),
    )
    .await
    .map_err(|_| invalid("probe preflight timed out"))?
    .map_err(invalid)?;

    let region = if let Some(region) = args.region {
        region
    } else if let Some(reference) = args.reference.as_deref() {
        let legacy_reference = resolve_reference(&app.core, &args.session_id, &args.chrome_tab_id, reference).await?;
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
        if token != snapshot.token || index >= snapshot.count || snapshot.consumed || snapshot.identity != before {
            return Err(invalid("probe reference is stale; take a fresh browser_snapshot"));
        }
        let response = tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId":snapshot.object_id,
                    "functionDeclaration":"function(index){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false};e.scrollIntoView({block:'center',inline:'center'});const r=e.getBoundingClientRect();return {ok:r.width>0&&r.height>0,x:r.left+scrollX,y:r.top+scrollY,width:r.width,height:r.height};}",
                    "arguments":[{"value":index}],
                    "returnByValue":true
                }),
            ),
        )
        .await
        .map_err(|_| invalid("probe target resolution timed out"))?
        .map_err(|error| invalid(error.to_string()))?;
        let value = super::super::shared_value(&response).map_err(invalid)?;
        if value["ok"] != true {
            return Err(invalid("probe reference no longer resolves to a visible element"));
        }
        ProbeRegionArgs {
            x: value["x"].as_f64().unwrap_or(-1.0),
            y: value["y"].as_f64().unwrap_or(-1.0),
            width: value["width"].as_f64().unwrap_or(0.0),
            height: value["height"].as_f64().unwrap_or(0.0),
        }
    } else {
        let selector = args.selector.as_deref().unwrap();
        let selector_json = serde_json::to_string(selector).map_err(|error| invalid(error.to_string()))?;
        let response = tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.evaluate",
                json!({
                    "expression":format!("(()=>{{let es;try{{es=[...document.querySelectorAll({selector_json})]}}catch(_){{return {{ok:false}}}};if(es.length!==1)return {{ok:false}};const e=es[0];e.scrollIntoView({{block:'center',inline:'center'}});const r=e.getBoundingClientRect();return {{ok:e.isConnected&&r.width>0&&r.height>0,x:r.left+scrollX,y:r.top+scrollY,width:r.width,height:r.height}};}})()"),
                    "returnByValue":true
                }),
            ),
        )
        .await
        .map_err(|_| invalid("probe selector resolution timed out"))?
        .map_err(|error| invalid(error.to_string()))?;
        let value = super::super::shared_value(&response).map_err(invalid)?;
        if value["ok"] != true {
            return Err(invalid("probe selector must uniquely resolve to one visible element"));
        }
        ProbeRegionArgs {
            x: value["x"].as_f64().unwrap_or(-1.0),
            y: value["y"].as_f64().unwrap_or(-1.0),
            width: value["width"].as_f64().unwrap_or(0.0),
            height: value["height"].as_f64().unwrap_or(0.0),
        }
    };
    if !probe_region_valid(region) {
        return Err(invalid("resolved probe region exceeds the bounded visual scope"));
    }
    let screenshot = tokio::time::timeout_at(
        deadline,
        connection.command(
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
            "Page.captureScreenshot",
            json!({
                "format":"png",
                "fromSurface":true,
                "captureBeyondViewport":true,
                "clip":{"x":region.x,"y":region.y,"width":region.width,"height":region.height,"scale":1.0}
            }),
        ),
    )
    .await
    .map_err(|_| invalid("targeted screenshot timed out"))?
    .map_err(|error| invalid(error.to_string()))?;
    let data = screenshot["data"]
        .as_str()
        .filter(|data| !data.is_empty())
        .ok_or_else(|| invalid("Chrome omitted targeted screenshot data"))?;
    let byte_length = base64_decoded_len(data)
        .ok_or_else(|| invalid("Chrome returned invalid targeted screenshot encoding"))?;
    if byte_length > max_bytes {
        return Err(invalid(format!(
            "targeted screenshot is {byte_length} bytes, exceeding max_bytes={max_bytes}"
        )));
    }
    let after = tokio::time::timeout_at(
        deadline,
        super::super::shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        ),
    )
    .await
    .map_err(|_| invalid("probe identity readback timed out"))?
    .map_err(invalid)?;
    if before != after {
        return Err(invalid(
            "document changed during targeted visual probe; screenshot discarded as stale",
        ));
    }
    let observed_at_ms = now_ms()?;
    Ok(json!({
        "status":"verified_read",
        "media_type":"image/png",
        "data_base64":data,
        "byte_length":byte_length,
        "crop":{"x":region.x,"y":region.y,"width":region.width,"height":region.height},
        "visual_ref":{
            "chrome_tab_id":args.chrome_tab_id,
            "frame_id":before.0,
            "loader_id":before.1,
            "url":before.2,
            "content_hash":sha256_hex(data.as_bytes()),
            "observed_at_ms":observed_at_ms
        }
    }))
}

async fn wait_for_selector(
    app: &AppFull,
    session_id: &str,
    tab_id: &str,
    selector: &str,
    timeout_ms: u64,
) -> Result<Value, rmcp::ErrorData> {
    if selector.trim().is_empty() || selector.len() > MAX_SELECTOR || !(1..=30_000).contains(&timeout_ms) {
        return Err(invalid("wait_for selector or timeout is out of bounds"));
    }
    let selector_json = serde_json::to_string(selector).map_err(|error| invalid(error.to_string()))?;
    let shared = app
        .core
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let shared = shared.lock().await;
    if shared.handle.principal != app.core.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let before = super::super::shared_frame_identity(connection, shared.registry()?, &shared.handle, tab_id)
        .await
        .map_err(invalid)?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let response = tokio::time::timeout_at(
            deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression":format!("(()=>{{try{{const e=document.querySelector({selector_json});if(!e||!e.isConnected)return false;const r=e.getBoundingClientRect(),s=getComputedStyle(e);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'}}catch(_){{return false}}}})()"),
                    "returnByValue":true
                }),
            ),
        )
        .await;
        if matches!(response, Ok(Ok(ref value)) if value.pointer("/result/value") == Some(&Value::Bool(true))) {
            let after = super::super::shared_frame_identity(
                connection,
                shared.registry()?,
                &shared.handle,
                tab_id,
            )
            .await
            .map_err(invalid)?;
            if before != after {
                return Err(invalid("document changed while waiting for selector"));
            }
            return Ok(json!({"status":"verified","kind":"wait_for","selector":selector}));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(json!({"status":"failed","kind":"wait_for","selector":selector,"reason":"timeout"}));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn observe_selector(
    app: &AppFull,
    session_id: &str,
    tab_id: &str,
    selector: String,
    max_items: usize,
    max_bytes: usize,
) -> Result<Value, rmcp::ErrorData> {
    let mut fields = BTreeMap::new();
    fields.insert("text".to_owned(), ":scope".to_owned());
    let Json(value) = app
        .core
        .legacy
        .shared_observe(Parameters(super::super::SharedObserveArgs {
            session_id: session_id.to_owned(),
            chrome_tab_id: tab_id.to_owned(),
            spec: super::super::ObserveInput {
                selector,
                fields,
                max_items: max_items.clamp(1, 500),
                max_text_chars: 10_000,
                max_bytes: max_bytes.clamp(4_096, 1_000_000),
                cursor: None,
            },
        }))
        .await?;
    Ok(value)
}

async fn execute_workflow(app: &AppFull, args: FullWorkflowArgs) -> Result<Value, rmcp::ErrorData> {
    validate_workflow(&args.steps)?;
    let mut vars = BTreeMap::<String, String>::new();
    let mut receipts = Vec::new();
    for step in args.steps {
        let receipt = match step {
            FullWorkflowStep::Snapshot => {
                snapshot_impl(
                    &app.core,
                    &BrowserSnapshotArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        selector: None,
                        max_items: None,
                        max_text_chars: None,
                        max_bytes: None,
                    },
                )
                .await?
            }
            FullWorkflowStep::Navigate { url } => {
                let Json(value) = app
                    .core
                    .legacy
                    .shared_tab(Parameters(super::super::SharedTabArgs {
                        session_id: args.session_id.clone(),
                        action: "navigate".to_owned(),
                        chrome_tab_id: Some(args.chrome_tab_id.clone()),
                        url,
                    }))
                    .await?;
                app.core
                    .semantic
                    .lock()
                    .await
                    .remove(&tab_key(&args.session_id, &args.chrome_tab_id));
                vars.clear();
                json!({"status":"verified","kind":"navigate","result":value})
            }
            FullWorkflowStep::Find {
                query,
                role,
                save_as,
            } => {
                let found = find_impl(
                    &app.core,
                    &BrowserFindArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        query,
                        role,
                        limit: Some(2),
                    },
                )
                .await?;
                let matches = found["matches"].as_array().cloned().unwrap_or_default();
                if matches.len() != 1 {
                    return Err(invalid(format!(
                        "workflow find for {save_as} must resolve exactly one control"
                    )));
                }
                let reference = matches[0]["reference"]
                    .as_str()
                    .ok_or_else(|| invalid("workflow find omitted reference"))?
                    .to_owned();
                vars.insert(save_as.clone(), reference.clone());
                json!({"status":"verified","kind":"find","save_as":save_as,"reference":reference,"match":matches[0]})
            }
            FullWorkflowStep::Click { reference, outcome } => {
                let reference = substitute(&reference, &vars)?;
                let Json(value) = app
                    .core
                    .browser_act(Parameters(BrowserActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "click".into(),
                        reference: Some(reference),
                        expected_value: None,
                        value: None,
                        outcome: Some(outcome),
                        timeout_ms: None,
                    }))
                    .await?;
                value
            }
            FullWorkflowStep::Fill {
                reference,
                expected_value,
                value,
            } => {
                let reference = substitute(&reference, &vars)?;
                retained_mutation(
                    &app.core,
                    &BrowserActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "fill".into(),
                        reference: Some(reference.clone()),
                        expected_value: Some(expected_value),
                        value: Some(value),
                        outcome: None,
                        timeout_ms: None,
                    },
                    &reference,
                    "fill",
                )
                .await?
            }
            FullWorkflowStep::Type {
                reference,
                expected_value,
                value,
            } => {
                let reference = substitute(&reference, &vars)?;
                strict_type(
                    app,
                    &FullActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "type".into(),
                        reference: Some(reference),
                        expected_value: Some(expected_value),
                        value: Some(value),
                        key: None,
                        outcome: None,
                        timeout_ms: None,
                    },
                )
                .await?
            }
            FullWorkflowStep::Press { reference, key } => {
                let reference = substitute(&reference, &vars)?;
                retained_press(
                    app,
                    &FullActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "press".into(),
                        reference: Some(reference),
                        expected_value: None,
                        value: None,
                        key: Some(key),
                        outcome: None,
                        timeout_ms: None,
                    },
                )
                .await?
            }
            FullWorkflowStep::Select {
                reference,
                expected_value,
                value,
            } => {
                let reference = substitute(&reference, &vars)?;
                retained_mutation(
                    &app.core,
                    &BrowserActArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        action: "select".into(),
                        reference: Some(reference.clone()),
                        expected_value: Some(expected_value),
                        value: Some(value),
                        outcome: None,
                        timeout_ms: None,
                    },
                    &reference,
                    "select",
                )
                .await?
            }
            FullWorkflowStep::WaitFor {
                selector,
                timeout_ms,
            } => wait_for_selector(
                app,
                &args.session_id,
                &args.chrome_tab_id,
                &selector,
                timeout_ms,
            )
            .await?,
            FullWorkflowStep::Observe { selector, max_bytes } => {
                let value = observe_selector(
                    app,
                    &args.session_id,
                    &args.chrome_tab_id,
                    selector,
                    60,
                    max_bytes,
                )
                .await?;
                json!({"status":"verified","kind":"observe","observation":value})
            }
            FullWorkflowStep::Extract {
                selector,
                max_items,
                max_bytes,
            } => {
                let value = observe_selector(
                    app,
                    &args.session_id,
                    &args.chrome_tab_id,
                    selector,
                    max_items,
                    max_bytes,
                )
                .await?;
                json!({"status":"verified","kind":"extract","observation":value})
            }
            FullWorkflowStep::Assert { predicate } => {
                let predicate = substitute_predicate(predicate, &vars)?;
                let mut value = verify_impl(
                    &app.core,
                    &BrowserVerifyArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        predicate,
                    },
                )
                .await?;
                value["kind"] = json!("assert");
                value
            }
            FullWorkflowStep::Verify { predicate } => {
                let predicate = substitute_predicate(predicate, &vars)?;
                let mut value = verify_impl(
                    &app.core,
                    &BrowserVerifyArgs {
                        session_id: args.session_id.clone(),
                        chrome_tab_id: args.chrome_tab_id.clone(),
                        predicate,
                    },
                )
                .await?;
                value["kind"] = json!("verify");
                value
            }
            FullWorkflowStep::Checkpoint => {
                json!({"status":"verified","kind":"checkpoint","after_step":receipts.len()})
            }
            FullWorkflowStep::Script { .. } => unreachable!(),
        };
        let should_stop = receipt
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| matches!(status, "unknown" | "failed" | "not_dispatched"));
        receipts.push(receipt);
        if should_stop {
            break;
        }
    }
    let terminal = receipts
        .last()
        .and_then(|receipt| receipt.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("verified");
    Ok(json!({
        "status":if terminal == "unknown" { "unknown" } else if matches!(terminal,"failed"|"not_dispatched") { "failed" } else { "completed" },
        "completed_steps":receipts.len(),
        "internal_model_calls":0,
        "route":"semantic",
        "steps":receipts
    }))
}

fn site_scope(url: &str) -> Result<String, rmcp::ErrorData> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| invalid("snapshot URL is not HTTP(S)"))?;
    if !matches!(scheme, "http" | "https") {
        return Err(invalid("snapshot URL is not HTTP(S)"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(invalid("snapshot URL has no safe authority"));
    }
    Ok(format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

async fn current_skill_context(
    app: &AppFull,
    session_id: &str,
    tab_id: &str,
) -> Result<(String, String), rmcp::ErrorData> {
    let snapshot = snapshot_impl(
        &app.core,
        &BrowserSnapshotArgs {
            session_id: session_id.to_owned(),
            chrome_tab_id: tab_id.to_owned(),
            selector: None,
            max_items: Some(200),
            max_text_chars: Some(6_000),
            max_bytes: Some(100_000),
        },
    )
    .await?;
    let url = snapshot
        .pointer("/document/url")
        .or_else(|| snapshot.get("url"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("snapshot omitted document URL"))?;
    let scope = site_scope(url)?;
    let semantic = app.core.semantic.lock().await;
    let state = semantic
        .get(&tab_key(session_id, tab_id))
        .ok_or_else(|| invalid("semantic snapshot state unavailable"))?;
    let mut items = state.items.values().collect::<Vec<_>>();
    items.sort_by_key(|item| item.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX));
    let mut hasher = Sha256::new();
    hasher.update(scope.as_bytes());
    for item in items {
        for field in ["role", "name", "href", "disabled"] {
            if let Some(value) = item.get(field) {
                hasher.update(field.as_bytes());
                hasher.update([0]);
                hasher.update(value.to_string().as_bytes());
                hasher.update([0xff]);
            }
        }
    }
    let digest = hasher.finalize();
    let mut signature = String::with_capacity(64);
    for byte in digest {
        let _ = write!(signature, "{byte:02x}");
    }
    Ok((scope, signature))
}

fn required<'a>(value: &'a Option<String>, label: &str) -> Result<&'a str, rmcp::ErrorData> {
    value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid(format!("{label} is required")))
}

async fn fresh_skill_verification(
    app: &AppFull,
    session_id: &str,
    tab_id: &str,
    predicate: BrowserPredicate,
) -> Result<crate::verifier::Verification, rmcp::ErrorData> {
    let result = verify_impl(
        &app.core,
        &BrowserVerifyArgs {
            session_id: session_id.to_owned(),
            chrome_tab_id: tab_id.to_owned(),
            predicate,
        },
    )
    .await?;
    Ok(if result["status"] == "passed" {
        crate::verifier::Verification::Passed
    } else {
        crate::verifier::Verification::Failed
    })
}

async fn browser_skill(app: &AppFull, args: FullSkillArgs) -> Result<Value, rmcp::ErrorData> {
    let now = now_ms()?;
    match args.action.as_str() {
        "describe" => Ok(json!({
            "status":"available",
            "actions":["inspect_context","register","training_success","validation_success","qualify","status","replay","verify_outcome"],
            "qualification":["distinct_training_success","distinct_validation_success","fresh_runtime_verification"],
            "replay_policy":"zero internal model calls on an unexpired exact site/structure match; structural drift quarantines before replay",
            "safety":"caller may choose a predicate but cannot supply verification evidence; runtime captures fresh page state"
        })),
        "inspect_context" => {
            let session_id = required(&args.session_id, "session_id")?;
            let tab_id = required(&args.chrome_tab_id, "chrome_tab_id")?;
            let (scope, signature) = current_skill_context(app, session_id, tab_id).await?;
            Ok(json!({"status":"observed","site_scope":scope,"structural_signature":signature}))
        }
        "register" => {
            let session_id = required(&args.session_id, "session_id")?;
            let tab_id = required(&args.chrome_tab_id, "chrome_tab_id")?;
            let definition_value = args
                .definition
                .ok_or_else(|| invalid("definition is required"))?;
            let definition: crate::skills::SkillDefinition = serde_json::from_value(definition_value)
                .map_err(|error| invalid(format!("invalid skill definition: {error}")))?;
            let compiled: crate::browser_workflow::BrowserWorkflowDefinition =
                serde_json::from_value(definition.workflow.clone())
                    .map_err(|error| invalid(format!("skill workflow schema is invalid: {error}")))?;
            crate::browser_workflow::compile_browser_workflow(compiled)
                .map_err(|error| invalid(format!("skill workflow is unsafe: {error}")))?;
            let (scope, signature) = current_skill_context(app, session_id, tab_id).await?;
            if definition.site_scope != scope
                || !definition
                    .structural_signatures
                    .iter()
                    .any(|candidate| candidate == &signature)
            {
                return Err(invalid(
                    "skill definition does not match the runtime-observed site scope and structural signature",
                ));
            }
            let record = app
                .skills
                .register_candidate(definition)
                .map_err(|error| invalid(error.to_string()))?;
            Ok(json!({"status":"candidate","record":record}))
        }
        "training_success" | "validation_success" | "qualify" | "verify_outcome" => {
            let skill_id = required(&args.skill_id, "skill_id")?;
            let session_id = required(&args.session_id, "session_id")?;
            let tab_id = required(&args.chrome_tab_id, "chrome_tab_id")?;
            let (scope, signature) = current_skill_context(app, session_id, tab_id).await?;
            let current = app
                .skills
                .status(skill_id, now)
                .map_err(|error| invalid(error.to_string()))?;
            let structural_match = current.definition.site_scope == scope
                && current
                    .definition
                    .structural_signatures
                    .iter()
                    .any(|candidate| candidate == &signature);
            if !structural_match {
                let record = app
                    .skills
                    .record_runtime_verification(
                        skill_id,
                        crate::verifier::Verification::Failed,
                        false,
                        true,
                    )
                    .map_err(|error| invalid(error.to_string()))?;
                return Ok(json!({"status":"quarantined","reason":"structural_drift","record":record}));
            }
            let predicate = args
                .predicate
                .ok_or_else(|| invalid("predicate is required for runtime-owned verification"))?;
            let verification = fresh_skill_verification(app, session_id, tab_id, predicate).await?;
            match args.action.as_str() {
                "training_success" => {
                    if verification != crate::verifier::Verification::Passed {
                        return Ok(json!({"status":"failed","reason":"fresh_runtime_verification_failed"}));
                    }
                    let run_id = required(&args.run_id, "run_id")?;
                    let record = app
                        .skills
                        .record_training_success(skill_id, run_id)
                        .map_err(|error| invalid(error.to_string()))?;
                    Ok(json!({"status":"recorded","kind":"training","record":record}))
                }
                "validation_success" => {
                    if verification != crate::verifier::Verification::Passed {
                        return Ok(json!({"status":"failed","reason":"fresh_runtime_verification_failed"}));
                    }
                    let run_id = required(&args.run_id, "run_id")?;
                    let record = app
                        .skills
                        .record_validation_success(skill_id, run_id)
                        .map_err(|error| invalid(error.to_string()))?;
                    Ok(json!({"status":"recorded","kind":"validation","record":record}))
                }
                "qualify" => {
                    if verification != crate::verifier::Verification::Passed {
                        return Ok(json!({"status":"failed","reason":"fresh_runtime_verification_failed"}));
                    }
                    let expires_at_ms = args
                        .expires_at_ms
                        .ok_or_else(|| invalid("expires_at_ms is required"))?;
                    let record = app
                        .skills
                        .qualify(
                            skill_id,
                            crate::verifier::Verification::Passed,
                            now,
                            expires_at_ms,
                        )
                        .map_err(|error| invalid(error.to_string()))?;
                    Ok(json!({"status":"qualified","record":record}))
                }
                "verify_outcome" => {
                    let record = app
                        .skills
                        .record_runtime_verification(
                            skill_id,
                            verification,
                            args.severe_safety_failure.unwrap_or(false),
                            false,
                        )
                        .map_err(|error| invalid(error.to_string()))?;
                    Ok(json!({"status":record.status,"record":record}))
                }
                _ => unreachable!(),
            }
        }
        "status" => {
            let skill_id = required(&args.skill_id, "skill_id")?;
            let record = app
                .skills
                .status(skill_id, now)
                .map_err(|error| invalid(error.to_string()))?;
            Ok(json!({"status":record.status,"record":record}))
        }
        "replay" => {
            let skill_id = required(&args.skill_id, "skill_id")?;
            let session_id = required(&args.session_id, "session_id")?;
            let tab_id = required(&args.chrome_tab_id, "chrome_tab_id")?;
            let (scope, signature) = current_skill_context(app, session_id, tab_id).await?;
            let replay = app
                .skills
                .replay(skill_id, &scope, &signature, now)
                .map_err(|error| invalid(error.to_string()))?;
            Ok(json!({
                "status":"qualified_hit",
                "route":"qualified_skill",
                "cache_status":"hit",
                "internal_model_calls":replay.internal_model_calls,
                "skill_id":replay.skill_id,
                "version":replay.version,
                "workflow":replay.workflow,
                "site_scope":scope,
                "structural_signature":signature
            }))
        }
        _ => Err(invalid(
            "browser_skill action must be describe, inspect_context, register, training_success, validation_success, qualify, status, replay, or verify_outcome",
        )),
    }
}

#[tool_router]
impl AppFull {
    #[tool(name = "browser_session", description = "Discover, pair, connect, list, or release explicitly selected Chrome targets through Controlla's authority-preserving session broker.")]
    async fn browser_session(
        &self,
        Parameters(args): Parameters<BrowserSessionArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.core.browser_session(Parameters(args)).await
    }

    #[tool(name = "browser_snapshot", description = "Return a compact semantic snapshot with stable short refs and a changed-only delta for the current document.")]
    async fn browser_snapshot(
        &self,
        Parameters(args): Parameters<BrowserSnapshotArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.core.browser_snapshot(Parameters(args)).await
    }

    #[tool(name = "browser_find", description = "Find current semantic controls by role/name/value/href and return exact revision-bound @cN refs.")]
    async fn browser_find(
        &self,
        Parameters(args): Parameters<BrowserFindArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.core.browser_find(Parameters(args)).await
    }

    #[tool(name = "browser_act", description = "Act on one exact @cN ref. Supports guarded click, verified O(1) fill/select, strict sequential ASCII type, and focused key press. Unknown effects are never automatically retried.")]
    async fn browser_act(
        &self,
        Parameters(args): Parameters<FullActArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let value = match args.action.as_str() {
            "click" | "fill" | "select" => {
                let Json(value) = self
                    .core
                    .browser_act(Parameters(BrowserActArgs {
                        session_id: args.session_id,
                        chrome_tab_id: args.chrome_tab_id,
                        action: args.action,
                        reference: args.reference,
                        expected_value: args.expected_value,
                        value: args.value,
                        outcome: args.outcome,
                        timeout_ms: args.timeout_ms,
                    }))
                    .await?;
                value
            }
            "type" => strict_type(self, &args).await?,
            "press" => retained_press(self, &args).await?,
            _ => return Err(invalid("action must be click, fill, select, type, or press")),
        };
        Ok(Json(value))
    }

    #[tool(name = "browser_extract", description = "Run bounded structured extraction from one explicitly paired tab with root frame/loader/URL freshness checks.")]
    async fn browser_extract(
        &self,
        Parameters(args): Parameters<BrowserExtractArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.core.browser_extract(Parameters(args)).await
    }

    #[tool(name = "browser_workflow", description = "Execute a bounded deterministic Chrome workflow. Supports navigate/find/click/fill/type/press/select/wait/observe/extract/assert/verify/checkpoint plus snapshots. Scripts fail closed on the shared route. Unknown mutation outcomes stop execution and are never automatically retried.")]
    async fn browser_workflow(
        &self,
        Parameters(args): Parameters<FullWorkflowArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(execute_workflow(self, args).await?))
    }

    #[tool(name = "browser_probe", description = "Capture a targeted PNG visual probe for exactly one @cN element, unique selector, or bounded region. The crop is document-bound and full-page screenshots are not the default path.")]
    async fn browser_probe(
        &self,
        Parameters(args): Parameters<FullProbeArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(visual_probe(self, args).await?))
    }

    #[tool(name = "browser_verify", description = "Independently re-observe the selected tab with fresh runtime-owned evidence and evaluate a bounded text/value/existence/URL predicate.")]
    async fn browser_verify(
        &self,
        Parameters(args): Parameters<BrowserVerifyArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.core.browser_verify(Parameters(args)).await
    }

    #[tool(name = "browser_skill", description = "Register, qualify, persist, inspect, verify, and replay reusable browser skills. Qualification requires distinct verified training/validation runs plus fresh runtime-owned verification; structural drift quarantines before replay.")]
    async fn browser_skill(
        &self,
        Parameters(args): Parameters<FullSkillArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(browser_skill(self, args).await?))
    }
}

#[tool_handler]
impl ServerHandler for AppFull {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "controlla-browser-v2",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Pair only explicitly selected tabs. Prefer semantic snapshot/find/act, then qualified skills, and use targeted visual probes only when semantic state is insufficient. Treat unknown effects as non-retriable until fresh observation resolves state. Qualification evidence is captured by the runtime, never supplied by the caller."
                .to_owned(),
        )
    }
}

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let state_dir = super::super::state_directory();
    std::fs::create_dir_all(&state_dir)?;
    let _state_lock = super::super::acquire_state_directory_lock(&state_dir)?;
    let journal = crate::jobs::Journal::open(state_dir.join("operations.sqlite"))?;
    journal.recover_after_restart()?;
    let skills = crate::skill_runtime::SkillRuntime::new(state_dir.join("skills-v2.json"));
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async {
        AppFull::new(
            AppV2::new(super::super::App::with_journal(journal)),
            skills,
        )
        .serve(rmcp::transport::io::stdio())
        .await?
        .waiting()
        .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
