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
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

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

    async fn require_v3_session(&self, session_id: &str) -> Result<(), rmcp::ErrorData> {
        if self.browser_modes.lock().await.contains_key(session_id) {
            Ok(())
        } else {
            Err(invalid("session was not created through the V3 browser route"))
        }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3BrowserArgs {
    action: String,
    provider: Option<String>,
    session_id: Option<String>,
    chrome_tab_id: Option<String>,
    target_ids: Option<Vec<String>>,
    host_id: Option<String>,
    mode: Option<String>,
    url: Option<String>,
    task: Option<Value>,
    filename: Option<String>,
    bytes: Option<Vec<u8>>,
    artifact_handle: Option<String>,
    client_id: Option<String>,
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
    page_tool_name: Option<String>,
    page_tool_input: Option<Value>,
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
#[serde(deny_unknown_fields)]
struct V3TaskArgs {
    session_id: String,
    chrome_tab_id: String,
    task: Value,
    client_id: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3ArtifactRegisterArgs {
    session_id: String,
    filename: String,
    bytes: Vec<u8>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct V3ArtifactReadArgs {
    session_id: String,
    artifact_handle: String,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum V3WorkflowStep {
    Snapshot,
    Find {
        query: String,
        role: Option<String>,
        save_as: String,
    },
    Click {
        reference: String,
        outcome: Option<BrowserClickOutcome>,
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
        typing_mode: Option<String>,
        delay_ms: Option<u64>,
    },
    Press {
        reference: String,
        key: String,
    },
    Extract {
        selector: String,
    },
    PageTool {
        name: String,
        input: Value,
    },
    Verify {
        predicate: BrowserPredicate,
    },
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
        "artifact_handle": if kind == "download" { value["artifact_handle"].clone() } else { Value::Null },
        "filename": if matches!(kind, "download" | "upload") { value["filename"].clone() } else { Value::Null },
        "size": if matches!(kind, "download" | "upload") { value["size"].clone() } else { Value::Null },
        "sha256": if kind == "download" { value["sha256"].clone() } else { Value::Null },
        "item_count": if kind == "extract" { value["items"].as_array().map(Vec::len) } else { None },
        "limited": if kind == "extract" { value["truncated"].as_bool() } else { None }})
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
        json!({"status":"selected","filename":evidence.filename,"size":evidence.size,
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
                }))
                .await
                .map(|Json(value)| value)
                .map_err(|error| error.to_string())
        }
    };
    if let Err(error) = navigation {
        return json!({"status":"unknown","reason":format!("navigation delivery uncertain; inspect before retrying: {error}")});
    }
    app.core
        .semantic
        .lock()
        .await
        .remove(&super::tab_key(session_id, tab_id));
    for _ in 0..20 {
        match verify_impl(
            &app.core,
            &BrowserVerifyArgs {
                session_id: session_id.to_owned(),
                chrome_tab_id: tab_id.to_owned(),
                predicate: BrowserPredicate::Url {
                    equals: url.to_owned(),
                },
            },
        )
        .await
        {
            Ok(value) if value["status"] == "passed" => return value,
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    json!({"status":"unknown","reason":"navigation was dispatched but the requested URL was not reobserved within the bounded wait"})
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
    let guard = include_str!("../shared_page/guard.js")
        .replace("__UNSAFE__", super::super::SHARED_CLICK_UNSAFE_PREDICATE);
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
        return Ok(json!({"status":"not_dispatched","reason":first["reason"]}));
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
        return Err(invalid("workflow find must resolve exactly one control"));
    }
    matches[0]["reference"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("workflow find omitted its reference"))
}

fn validate_workflow_steps(steps: &[V3WorkflowStep]) -> Result<(), rmcp::ErrorData> {
    if steps.iter().enumerate().any(|(index, step)| {
        matches!(
            step,
            V3WorkflowStep::PageTool { .. } | V3WorkflowStep::Press { .. }
        ) && !matches!(steps.get(index + 1), Some(V3WorkflowStep::Verify { .. }))
    }) {
        return Err(invalid(
            "press and page_tool steps must be followed immediately by runtime verification",
        ));
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

async fn acquire_lease(
    app: &AppV3,
    args: &V3ActArgs,
) -> Result<crate::v3::TargetKey, rmcp::ErrorData> {
    let owner = args.client_id.as_deref().unwrap_or("local-mcp");
    let key = lease_target_key(&args.session_id, &args.chrome_tab_id);
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
        "fill",
    )
    .await
    .map(|mut value| {
        value["action"] = json!("type");
        value["typing_mode"] = json!("block");
        value
    })
}

async fn ime_type(app: &AppV3, args: &V3ActArgs) -> Result<Value, rmcp::ErrorData> {
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
    if identity != snapshot.identity {
        return Err(invalid(
            "document changed since snapshot; take a fresh browser_snapshot",
        ));
    }
    let guard = r#"function(index,expected){const e=this.nodes[index];return !!e&&e.isConnected&&(e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type))&&!e.disabled&&!e.readOnly&&document.activeElement===e&&e.value===expected&&e.selectionStart===e.selectionEnd&&e.selectionEnd===e.value.length;}"#;
    let initial = {
        let connection = shared.connection.as_ref().unwrap();
        connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
            "objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":expected}],"returnByValue":true
        })).await.map_err(|error| invalid(error.to_string()))?
    };
    if super::super::shared_value(&initial).map_err(invalid)? != json!(true) {
        return Ok(
            json!({"status":"not_dispatched","action":"type","typing_mode":"ime","reason":"field, focus, or caret changed before composition"}),
        );
    }
    shared
        .snapshots
        .get_mut(&args.chrome_tab_id)
        .unwrap()
        .consumed = true;
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
    if !matches!(composed, Ok(ref value) if super::super::shared_value(value).is_ok_and(|value| value == true))
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
    let verified = matches!(readback, Ok(ref value) if super::super::shared_value(value).is_ok_and(|value| value == json!(true)))
        && matches!(after, Ok(ref current) if current == &identity);
    drop(shared);
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

async fn key_type(
    app: &AppV3,
    args: &V3ActArgs,
    policy: crate::v3::TypingPolicy,
) -> Result<Value, rmcp::ErrorData> {
    let reference = args
        .reference
        .as_deref()
        .ok_or_else(|| invalid("type requires reference"))?;
    let expected = args.expected_value.as_deref().unwrap_or_default();
    let text = args.value.as_deref().unwrap_or_default();
    if text.is_empty() || expected.len().saturating_add(text.len()) > 16_384 || !text.is_ascii() {
        return Err(invalid(
            "key typing requires nonempty ASCII text and final value <= 16384 bytes",
        ));
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
    let guard_function = r#"function(index,expected){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type))||e.matches(':disabled')||e.readOnly||e.hasAttribute('data-masked'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};if(document.activeElement!==e||e.selectionStart!==e.selectionEnd||e.selectionEnd!==e.value.length)return {ok:false,reason:'focus_or_selection_changed'};return {ok:true,value:e.value};}"#;
    let mut progress = expected.to_owned();
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
                    "arguments":[{"value":index},{"value":progress}],"returnByValue":true
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
    shared
        .snapshots
        .get_mut(&args.chrome_tab_id)
        .ok_or_else(|| invalid("snapshot disappeared before dispatch"))?
        .consumed = true;
    let mut dispatch_count = 0_u64;
    if policy.mode == crate::v3::TypingMode::FastKeys {
        let characters = text.chars().collect::<Vec<_>>();
        for chunk in characters.chunks(10) {
            let mut actions = Vec::with_capacity(chunk.len() * 6);
            let guard_action = |expected: &str| controlla_browser::providers::SharedBatchAction {
                method: "Runtime.callFunctionOn".into(),
                params: json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                    "arguments":[{"value":index},{"value":expected}],"returnByValue":true}),
                stop_on_not_ok: true,
            };
            for character in chunk {
                actions.push(guard_action(&progress));
                let key = character.to_string();
                for (event_index, params) in [
                    json!({"type":"keyDown","key":key}),
                    json!({"type":"char","key":key,"text":key,"unmodifiedText":key}),
                    json!({"type":"keyUp","key":key}),
                ]
                .into_iter()
                .enumerate()
                {
                    actions.push(controlla_browser::providers::SharedBatchAction {
                        method: "Input.dispatchKeyEvent".into(),
                        params,
                        stop_on_not_ok: false,
                    });
                    if event_index == 0 {
                        actions.push(guard_action(&progress));
                    } else if event_index == 1 {
                        progress.push(*character);
                        actions.push(guard_action(&progress));
                    }
                }
            }
            let connection = shared.connection.as_ref().unwrap();
            let result = match crate::v3_runtime::await_key_batch(
                deadline,
                connection.command_batch(
                    shared.registry()?,
                    &shared.handle,
                    &args.chrome_tab_id,
                    actions,
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    return Ok(
                        json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count > 0,
                    "dispatch_may_have_occurred":true,
                    "reason":"fast-key batch exceeded its deadline; inspect state and do not automatically retry"}),
                    );
                }
            };
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    return Ok(
                        json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count > 0,
                    "dispatch_may_have_occurred":true,
                    "reason":format!("fast-key batch delivery is unknown; inspect state and do not automatically retry: {error}")}),
                    );
                }
            };
            if let Some(error) = result["batch_error"].as_str() {
                let (status, acknowledged, may_have_occurred) =
                    fast_key_batch_failure(&result, dispatch_count);
                return Ok(
                    json!({"status":status,"action":"type","dispatch_acknowledged":acknowledged,
                    "dispatch_may_have_occurred":may_have_occurred,
                    "batch_receipt":result,"reason":format!("fast-key batch stopped; inspect current state and do not automatically retry: {error}")}),
                );
            }
            if result["completed"].as_u64() != Some((chunk.len() * 6) as u64) {
                return Ok(
                    json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count > 0,
                    "dispatch_may_have_occurred":true,
                    "batch_receipt":result,"reason":"fast-key batch returned an incomplete receipt; inspect state and do not automatically retry"}),
                );
            }
            dispatch_count = dispatch_count.saturating_add((chunk.len() * 3) as u64);
        }
    } else {
        for character in text.chars() {
            let guard =
                {
                    let connection = shared.connection.as_ref().unwrap();
                    let response = tokio::time::timeout_at(deadline, connection.command(
                shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn",
                json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                    "arguments":[{"value":index},{"value":progress}],"returnByValue":true})
            )).await;
                    match response {
                        Ok(Ok(response)) => super::super::shared_value(&response).ok(),
                        _ => None,
                    }
                };
            if guard.as_ref().is_none_or(|value| value["ok"] != true) {
                return Ok(
                    json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                "reason":"focus, value, selection, target, or document changed during typing; inspect before retrying"}),
                );
            }
            let key = character.to_string();
            let events = [
                json!({"type":"keyDown","key":key}),
                json!({"type":"char","key":key,"text":key,"unmodifiedText":key}),
                json!({"type":"keyUp","key":key}),
            ];
            for (event_index, event) in events.into_iter().enumerate() {
                if event_index > 0 {
                    if event_index == 2 {
                        progress.push(character);
                    }
                    let connection = shared.connection.as_ref().unwrap();
                    let guard = tokio::time::timeout_at(deadline, connection.command(
                    shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn",
                    json!({"objectId":snapshot.object_id,"functionDeclaration":guard_function,
                        "arguments":[{"value":index},{"value":progress}],"returnByValue":true})
                )).await;
                    let guard = match guard {
                        Ok(Ok(response)) => super::super::shared_value(&response).ok(),
                        _ => None,
                    };
                    if guard.as_ref().is_none_or(|value| value["ok"] != true) {
                        return Ok(
                            json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                        "reason":"focus, value, selection, target, or document changed between key events; inspect before retrying"}),
                        );
                    }
                }
                let connection = shared.connection.as_ref().unwrap();
                let sent = tokio::time::timeout_at(
                    deadline,
                    connection.command(
                        shared.registry()?,
                        &shared.handle,
                        &args.chrome_tab_id,
                        "Input.dispatchKeyEvent",
                        event,
                    ),
                )
                .await;
                if !matches!(sent, Ok(Ok(_))) {
                    return Ok(
                        json!({"status":"unknown","action":"type","dispatch_acknowledged":dispatch_count>0,
                    "reason":"key dispatch became uncertain; inspect state and do not automatically retry"}),
                    );
                }
                dispatch_count = dispatch_count.saturating_add(1);
                if event_index == 1 {
                    progress.push(character);
                }
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
                "arguments":[{"value":index},{"value":progress}],"returnByValue":true}),
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
        return Ok(
            json!({"status":"unknown","action":"type","dispatch_acknowledged":true,
            "reason":"post-typing readback or document identity did not verify final state; inspect before retrying"}),
        );
    }
    let mut result = json!({"status":"verified","action":"type","dispatch_acknowledged":true,
        "typing_mode":match policy.mode { crate::v3::TypingMode::FastKeys=>"fast_keys", _=>"human_keys" },
        "readback":progress,"cdp_key_events":dispatch_count,"intentional_delay_ms":policy.delay_ms});
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
    if token != snapshot.token || index >= snapshot.count || snapshot.consumed {
        return Err(invalid("stale or consumed reference"));
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let preflight = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
        "objectId":snapshot.object_id,"functionDeclaration":"function(index){const e=this.nodes[index];return {ok:!!e&&e.isConnected&&document.activeElement===e};}",
        "arguments":[{"value":index}],"returnByValue":true
    })).await.map_err(|error| invalid(error.to_string()))?;
    if super::super::shared_value(&preflight).map_err(invalid)?["ok"] != true {
        return Ok(
            json!({"status":"not_dispatched","action":"press","reason":"referenced element is not connected and focused"}),
        );
    }
    shared
        .snapshots
        .get_mut(&args.chrome_tab_id)
        .unwrap()
        .consumed = true;
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
        description = "Manage browser sessions and bounded task/file actions. Supports foreground, background, and headless modes; action=task runs bounded work; artifact_register/read use opaque private-headless handles."
    )]
    async fn browser(
        &self,
        Parameters(args): Parameters<V3BrowserArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
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
                    bytes: args.bytes.ok_or_else(|| invalid("bytes are required"))?,
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
                }))
                .await;
        }
        if args.action == "describe" {
            return Ok(Json(json!({
                "version":"v3","tools":["browser","snapshot","act","workflow","extract","verify"],
                "modes":{"foreground":crate::v3::BrowserMode::Foreground.policy(),"background":crate::v3::BrowserMode::Background.policy(),"headless":crate::v3::BrowserMode::Headless.policy()},
                "typing_modes":["block","fast_keys","human_keys","ime"],
                "safety":"revision-bound refs, target leases, fresh-state verification, no blind mutation retry"
            })));
        }
        let mode = parse_mode(args.mode.as_deref())?;
        let provider = args
            .provider
            .or_else(|| Some(mode.default_provider().to_owned()));
        if provider.as_deref() != Some(mode.default_provider()) {
            return Err(invalid(format!(
                "mode {mode:?} requires provider {}",
                mode.default_provider()
            )));
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
                "foreground/background browser action must be pair_shared, accept_shared, list, release, or discover_shared_tabs",
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
        description = "Return a compact semantic snapshot/delta with revision-bound @cN refs. Use before mutations and again after user/page changes."
    )]
    async fn snapshot(
        &self,
        Parameters(args): Parameters<BrowserSnapshotArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        snapshot_impl(&self.core, &args).await.map(Json)
    }

    #[tool(
        description = "Perform a guarded mutation, discover selected page tools with action=page_tools, or explicitly invoke one with action=page_tool. type supports block, fast_keys (real zero-delay CDP key events), human_keys, and ime. Mutations lease the exact target and fail closed on drift."
    )]
    async fn act(
        &self,
        Parameters(mut args): Parameters<V3ActArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        let lease = acquire_lease(self, &args).await?;
        let result = match tokio::time::timeout(Duration::from_secs(60), async {
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
                        outcome: args.outcome.take(),
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
            Err(_) => Ok(
                json!({"status":"unknown","action":args.action,"dispatch_acknowledged":false,
                "reason":"action exceeded its 60-second bound; inspect state before retrying"}),
            ),
        };
        if !matches!(&result, Ok(value) if value["status"] == "unknown") {
            release_lease(self, &args, &lease).await;
        }
        result.map(Json)
    }

    #[tool(
        description = "Run a bounded deterministic browser program in one MCP call. Supports snapshot/find/click/fill/type/press/page_tool/extract/verify and rejects unresolved refs or >40 steps."
    )]
    async fn workflow(
        &self,
        Parameters(args): Parameters<V3WorkflowArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        if args.steps.is_empty() || args.steps.len() > 40 {
            return Err(invalid("workflow requires 1..40 steps"));
        }
        validate_workflow_steps(&args.steps)?;
        let mut vars = std::collections::BTreeMap::<String, String>::new();
        let mut receipts = Vec::with_capacity(args.steps.len());
        for step in args.steps {
            let resolve = |reference: String,
                           vars: &std::collections::BTreeMap<String, String>|
             -> Result<String, rmcp::ErrorData> {
                if let Some(name) = reference.strip_prefix('$') {
                    vars.get(name)
                        .cloned()
                        .ok_or_else(|| invalid(format!("unresolved workflow variable ${name}")))
                } else {
                    Ok(reference)
                }
            };
            let value = match step {
                V3WorkflowStep::Snapshot => {
                    snapshot_impl(
                        &self.core,
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
                V3WorkflowStep::Find {
                    query,
                    role,
                    save_as,
                } => {
                    let found = find_impl(
                        &self.core,
                        &BrowserFindArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            query,
                            role,
                            limit: Some(2),
                        },
                    )
                    .await?;
                    let reference = workflow_find_reference(&found)?;
                    if save_as.is_empty() || vars.insert(save_as, reference).is_some() {
                        return Err(invalid("save_as must be unique and nonempty"));
                    }
                    found
                }
                V3WorkflowStep::Click { reference, outcome } => {
                    let reference = resolve(reference, &vars)?;
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
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Fill {
                    reference,
                    expected_value,
                    value,
                } => {
                    let reference = resolve(reference, &vars)?;
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
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Type {
                    reference,
                    expected_value,
                    value,
                    typing_mode,
                    delay_ms,
                } => {
                    let reference = resolve(reference, &vars)?;
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
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Press { reference, key } => {
                    let reference = resolve(reference, &vars)?;
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
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Extract { selector } => {
                    self.core
                        .browser_extract(Parameters(BrowserExtractArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            selector,
                            fields: std::collections::BTreeMap::new(),
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
                    }))
                    .await?
                    .0
                }
                V3WorkflowStep::Verify { predicate } => {
                    verify_impl(
                        &self.core,
                        &BrowserVerifyArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            predicate,
                        },
                    )
                    .await?
                }
            };
            let stop_status = workflow_should_stop(&value)
                .then(|| value["status"].as_str().unwrap_or("unknown").to_owned());
            receipts.push(value);
            if let Some(status) = stop_status {
                return Ok(Json(
                    json!({"status":status,"steps":receipts.len(),"receipts":receipts}),
                ));
            }
        }
        Ok(Json(
            json!({"status":"completed","steps":receipts.len(),"receipts":receipts}),
        ))
    }

    async fn artifact_register(
        &self,
        Parameters(args): Parameters<V3ArtifactRegisterArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        if args.bytes.is_empty() || args.bytes.len() > 10 * 1024 * 1024 {
            return Err(invalid(
                "artifact bytes must be between 1 and 10485760 bytes",
            ));
        }
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
            || !super::super::private_headless_cleanup_allowed(&live.handle)
        {
            return Err(invalid(
                "artifact registration requires a dedicated private headless session",
            ));
        }
        let handle = live.handle.clone();
        let metadata = live
            .registry
            .as_mut()
            .ok_or_else(|| invalid("session registry unavailable"))?
            .put_artifact_bytes(&handle, &args.filename, &args.bytes)
            .map_err(|error| invalid(format!("artifact registration failed: {error:?}")))?;
        Ok(Json(
            json!({"artifact_handle":metadata.handle,"filename":metadata.filename,"size":metadata.size,
            "scope":"private_headless_session","verified":true}),
        ))
    }

    async fn artifact_read(
        &self,
        Parameters(args): Parameters<V3ArtifactReadArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
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
            || !super::super::private_headless_cleanup_allowed(&live.handle)
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
        Ok(Json(json!({"artifact_handle":handle,"size":bytes.len(),
            "sha256":crate::artifacts::ArtifactExpectation::sha256(&bytes),"bytes":bytes,
            "verified":true,"scope":"private_headless_session"})))
    }

    async fn task(
        &self,
        Parameters(args): Parameters<V3TaskArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        use crate::v3_tasks::{ExpandedStep, TaskPrimitive};
        let primitive: TaskPrimitive =
            serde_json::from_value(args.task).map_err(|error| invalid(error.to_string()))?;
        let steps = primitive.expand().map_err(invalid)?;
        let mut url = String::new();
        let mut active_tab_id = args.chrome_tab_id.clone();
        let mut receipts = Vec::with_capacity(steps.len());
        for step in steps {
            let (kind, reference, result) = match step {
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
                    let _ = snapshot_impl(
                        &self.core,
                        &BrowserSnapshotArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            selector: None,
                            max_items: None,
                            max_text_chars: None,
                            max_bytes: None,
                        },
                    )
                    .await?;
                    let action = self
                        .act(Parameters(V3ActArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
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
                        }))
                        .await?
                        .0;
                    let verified = if action["status"] == "verified" {
                        verify_impl(
                            &self.core,
                            &BrowserVerifyArgs {
                                session_id: args.session_id.clone(),
                                chrome_tab_id: args.chrome_tab_id.clone(),
                                predicate: BrowserPredicate::Value {
                                    reference: reference.clone(),
                                    equals: value,
                                },
                            },
                        )
                        .await?
                    } else {
                        action
                    };
                    ("fill", Some(reference), verified)
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
                        }))
                        .await?
                        .0;
                    ("save_draft", Some(control.reference), value)
                }
                ExpandedStep::Upload {
                    artifact_handle,
                    selector,
                    account_marker,
                } => {
                    let value = headless_file_select(
                        self,
                        &args.session_id,
                        &args.chrome_tab_id,
                        &artifact_handle,
                        &selector,
                        &account_marker,
                    )
                    .await?;
                    ("upload", None, value)
                }
                ExpandedStep::Download { reference } => {
                    let value =
                        headless_download(self, &args.session_id, &args.chrome_tab_id, &reference)
                            .await?;
                    ("download", Some(reference), value)
                }
            };
            let receipt = task_receipt(kind, &url, reference.as_deref(), &result);
            let verified = receipt["verified"] == true;
            receipts.push(receipt);
            if !verified || workflow_should_stop(&result) {
                let status = if result["status"] == "selected" {
                    "incomplete"
                } else {
                    result["status"].as_str().unwrap_or("incomplete")
                };
                return Ok(Json(
                    json!({"status":status,"steps":receipts.len(),"receipts":receipts}),
                ));
            }
        }
        Ok(Json(
            json!({"status":"completed","steps":receipts.len(),"receipts":receipts}),
        ))
    }

    #[tool(description = "Extract bounded structured page data without screenshots.")]
    async fn extract(
        &self,
        Parameters(args): Parameters<BrowserExtractArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
        self.core.browser_extract(Parameters(args)).await
    }

    #[tool(
        description = "Independently reobserve the browser and verify a semantic postcondition."
    )]
    async fn verify(
        &self,
        Parameters(args): Parameters<BrowserVerifyArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.require_v3_session(&args.session_id).await?;
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
        .with_instructions("Controlla v3 exposes six compact tools. Use browser action=task for bounded research and form drafts, and browser action=artifact_register/read for opaque private-headless file handles. Use workflow for explicit multi-step automation, block typing for ordinary text, fast_keys only when real key events are required, and verify after consequential mutations. Background mode must not steal focus. Reobserve after user/page drift; never blindly retry an unknown mutation outcome.")
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
        AppV3, BrowserPredicate, V3WorkflowStep, fast_key_batch_failure, lease_target_key,
        page_tool_completion, page_tool_snapshot_fresh, page_tool_unavailable, task_receipt,
        typing_policy, validate_workflow_steps, workflow_find_reference, workflow_should_stop,
    };
    use serde_json::json;

    #[test]
    fn compact_surface_routes_tasks_and_receipts_do_not_dump_page_state() {
        let tools = AppV3::tool_router().list_all();
        assert_eq!(tools.len(), 6);
        assert!(tools.iter().all(|tool| {
            ["browser", "snapshot", "act", "workflow", "extract", "verify"]
                .contains(&tool.name.as_ref())
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

    #[tokio::test]
    async fn headless_mode_rejects_shared_provider_instead_of_falling_back() {
        use rmcp::{ServiceExt, model::CallToolRequestParams};
        let app = AppV3::new(super::super::AppV2::new(super::super::App::default()));
        let (server_io, client_io) = tokio::io::duplex(65536);
        let server =
            rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(app, server_io, None);
        let server_task = tokio::spawn(async move {
            let _ = server.waiting().await;
        });
        let client = ().serve(client_io).await.unwrap();
        let params = CallToolRequestParams::new("browser".to_owned()).with_arguments(json!({
            "action":"pair_shared", "mode":"headless", "provider":"shared_extension", "target_ids":["123"]
        }).as_object().unwrap().clone());
        let result = client.call_tool(params).await;
        assert!(
            result.is_err() || result.is_ok_and(|result| result.is_error.unwrap_or(false)),
            "headless must never silently attach to shared Chrome"
        );
        let _ = client.cancel().await;
        server_task.abort();
    }

    #[tokio::test]
    async fn foreground_and_background_pairing_return_the_selected_mode_and_enforce_it_on_release()
    {
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
    }

    #[tokio::test]
    #[ignore = "requires installed Chrome; validates V3 FastKeys through the shared extension bridge"]
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
            eval(r#"document.body.innerHTML='<input id="trusted" aria-label="Trusted keys" type="text" data-requires-trusted value="before"><input id="other" aria-label="Other" type="text"><script></script>';window.events=[];window.otherEvents=[];const input=document.querySelector('#trusted');for(const type of ['keydown','beforeinput','input','keyup'])input.addEventListener(type,event=>{window.events.push({type,trusted:event.isTrusted});if(type==='keydown'&&window.blurOnKeydown){window.blurOnKeydown=false;input.blur();document.querySelector('#other').focus();}});document.querySelector('#other').addEventListener('keyup',event=>window.otherEvents.push({type:event.type,key:event.key,trusted:event.isTrusted}));"#.into()).await;
            let (server_io,client_io)=tokio::io::duplex(65536);
            let server=rmcp::service::serve_directly::<rmcp::RoleServer,_,_,_,_>(super::AppV3::new(super::AppV2::new(super::super::App::default())),server_io,None);
            let server_task=tokio::spawn(async move {let _=server.waiting().await;});
            let client=().serve(client_io).await.unwrap();
            let call=|name:&str,value:serde_json::Value| CallToolRequestParams::new(name.to_owned()).with_arguments(value.as_object().unwrap().clone());
            let pair=client.call_tool(call("browser",json!({"action":"pair_shared","target_ids":["123","124"]}))).await.unwrap().structured_content.unwrap();
            let session=pair["session_id"].as_str().unwrap().to_owned();
            let (mut socket,_)=connect_async(pair["endpoint"].as_str().unwrap()).await.unwrap();
            socket.send(Message::Text(json!({"type":"hello","token":pair["one_session_token"],"extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"batch_execution":true,"batch_deadline":true,"targets":["123","124"]}).to_string().into())).await.unwrap();
            let _=socket.next().await.unwrap();
            let c=connection.clone(); let t=target.clone();
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
                    let response=c.target_command(&t,state.generation,&state.revision,method,request["params"].clone()).await;
                    let response=match response {Ok(value)=>json!({"type":"result","id":request["id"],"result":value}),Err(error)=>json!({"type":"result","id":request["id"],"error":error.to_string()})};
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
            let events=eval("JSON.stringify({value:document.querySelector('#trusted').value,events:window.events})".into()).await["result"]["value"].as_str().unwrap().to_owned();
            let events:serde_json::Value=serde_json::from_str(&events).unwrap();
            assert_eq!(events["value"],format!("before{payload}"));
            let observed=events["events"].as_array().unwrap();
            assert_eq!(observed.len(),4000);
            assert!(observed.as_chunks::<4>().0.iter().all(|events|events.iter().map(|event|event["type"].as_str().unwrap()).eq(["keydown","beforeinput","input","keyup"])));
            assert!(observed.iter().all(|event|event["trusted"]==true),"untrusted key event: {events}");
            let fresh=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"123"}))).await.unwrap().structured_content.unwrap();
            let target_ref=fresh["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            eval("window.blurOnKeydown=true".into()).await;
            let interrupted=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"123","action":"type","reference":target_ref,"expected_value":format!("before{payload}"),"value":"Z","timeout_ms":60000}))).await.unwrap().structured_content.unwrap();
            assert_eq!(interrupted["status"],"unknown","{interrupted}");
            let values=eval("JSON.stringify([document.querySelector('#trusted').value,document.querySelector('#other').value])".into()).await["result"]["value"].as_str().unwrap().to_owned();
            assert_eq!(serde_json::from_str::<serde_json::Value>(&values).unwrap(),json!([format!("before{payload}"),""]));
            let key_events=eval("JSON.stringify({source:window.events.slice(-1),other:window.otherEvents})".into()).await["result"]["value"].as_str().unwrap().to_owned();
            let key_events:serde_json::Value=serde_json::from_str(&key_events).unwrap();
            assert_eq!(key_events["source"][0]["type"],"keydown");
            assert_eq!(key_events["other"],json!([]),"never release a stale key to the newly focused control: {key_events}");

            eval("document.querySelector('#trusted').focus();window.blurOnKeydown=true".into()).await;
            let fresh=client.call_tool(call("snapshot",json!({"session_id":session,"chrome_tab_id":"124"}))).await.unwrap().structured_content.unwrap();
            let target_ref=fresh["items"].as_array().unwrap().iter().find(|item|item["name"]=="Trusted keys").unwrap()["reference"].clone();
            let interrupted=client.call_tool(call("act",json!({"session_id":session,"chrome_tab_id":"124","action":"type","reference":target_ref,"expected_value":format!("before{payload}"),"value":"Y","typing_mode":"human_keys","timeout_ms":60000}))).await.unwrap().structured_content.unwrap();
            assert_eq!(interrupted["status"],"unknown","{interrupted}");
            let key_events=eval("JSON.stringify({source:window.events.slice(-1),other:window.otherEvents})".into()).await["result"]["value"].as_str().unwrap().to_owned();
            let key_events:serde_json::Value=serde_json::from_str(&key_events).unwrap();
            assert_eq!(key_events["source"][0]["type"],"keydown");
            assert_eq!(key_events["other"],json!([]),"HumanKeys must also avoid retargeted keyup: {key_events}");
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
    #[tokio::test]
    #[ignore = "requires installed Google Chrome; exercises the complete V3 dedicated headless lifecycle"]
    async fn v3_headless_launch_list_snapshot_fill_verify_and_release() {
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
            "action":"launch", "mode":"headless", "url":"data:text/html,%3Cinput%20aria-label%3D%22Name%22%20value%3D%22before%22%3E"
        }))).await.unwrap().structured_content.unwrap();
        assert_eq!(launch["launched"], true, "{launch}");
        let session_id = launch["session_id"].as_str().unwrap().to_owned();
        let target_id = launch["target_ids"][0].as_str().unwrap().to_owned();
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
        let filled = client.call_tool(call("act", json!({"session_id":session_id,"chrome_tab_id":target_id,"action":"fill","reference":reference,"expected_value":"before","value":"after"}))).await.unwrap().structured_content.unwrap();
        assert_eq!(filled["status"], "verified", "{filled}");
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
    }
}
