use controlla_browser::{
    BrowserManager, ExpansionControl, ExtractionSpec, GuardSnapshot, GuardedFileSelection,
    ObserveSpec, ScreenshotCrop, SessionProvider, connect_permissioned_auto_connect,
    identity_marker_command, list_sessions, observation_command, parse_observation,
    sessions::{
        IdentityRevisions, ProviderGrants, SessionMode, SessionRegistry, SessionSpec, TargetRef,
    },
};
use rmcp::{ServiceExt, handler::server::wrapper::Parameters, tool, tool_router};
use serde_json::{Value, json};
use std::{collections::BTreeMap, net::IpAddr, sync::Arc};
use tokio::sync::Mutex;

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct SessionArgs {
    action: String,
    provider: Option<String>,
    session_id: Option<String>,
    target_ids: Option<Vec<String>>,
}
#[derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
struct TargetRefInput {
    session_id: String,
    principal: String,
    capability_revision: u64,
    browser_instance_id: String,
    browser_generation: u64,
    target_id: String,
    target_revision: String,
    frame_id: String,
    frame_revision: u64,
    account_revision: u64,
    document_revision: u64,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ObserveInput {
    selector: String,
    fields: BTreeMap<String, String>,
    max_items: usize,
    max_text_chars: usize,
    max_bytes: usize,
    cursor: Option<String>,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct SectionInput {
    section_id: String,
    spec: ExtractionInput,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ExtractionInput {
    container: String,
    record: String,
    fields: BTreeMap<String, String>,
    id_field: String,
    max_steps: usize,
    max_records: usize,
    max_text_chars: usize,
    max_bytes: usize,
    expected_count: Option<usize>,
    account_marker: Option<(String, String)>,
    terminal_selector: Option<String>,
    #[serde(default)]
    expand: Vec<ExpansionInput>,
    cursor: Option<String>,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ExpansionInput {
    selector: String,
    content_selector: String,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ObserveArgs {
    session_id: String,
    target_ref: TargetRefInput,
    spec: ObserveInput,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct SharedObserveArgs {
    session_id: String,
    chrome_tab_id: String,
    spec: ObserveInput,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ExtractArgs {
    session_id: String,
    target_ref: TargetRefInput,
    sections: Vec<SectionInput>,
    max_records: usize,
    max_bytes: usize,
    timeout_ms: Option<u64>,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct AccessibilityArgs {
    session_id: String,
    target_ref: TargetRefInput,
    selector: String,
    max_bytes: usize,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ScreenshotArgs {
    session_id: String,
    target_ref: TargetRefInput,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
    max_bytes: usize,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ArtifactRegisterArgs {
    session_id: String,
    filename: String,
    bytes: Vec<u8>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct FileSelectArgs {
    session_id: String,
    target_ref: TargetRefInput,
    locator: Value,
    artifact_handle: String,
    account_marker: (String, String),
}

struct LiveSession {
    connection: Arc<controlla_browser::BrowserConnection>,
    registry: Mutex<SessionRegistry>,
    handle: controlla_browser::sessions::SessionHandle,
}

struct SharedLiveSession {
    provider: Option<controlla_browser::providers::SharedExtensionProvider>,
    connection: Option<controlla_browser::providers::SharedExtensionSession>,
    registry: SessionRegistry,
    handle: controlla_browser::sessions::SessionHandle,
}

#[derive(Clone, Default)]
struct App {
    manager: BrowserManager,
    sessions: Arc<Mutex<BTreeMap<String, Arc<LiveSession>>>>,
    shared_sessions: Arc<Mutex<BTreeMap<String, Arc<Mutex<SharedLiveSession>>>>>,
}

async fn run_file_select(s: Arc<LiveSession>, args: FileSelectArgs) -> Result<Value, String> {
    let reference: TargetRef =
        serde_json::from_value(serde_json::to_value(args.target_ref).map_err(|e| e.to_string())?)
            .map_err(|_| "target_ref must be a complete current TargetRef".to_owned())?;
    if reference.session_id != args.session_id {
        return Err("target_ref session does not match session_id".into());
    }
    if s.handle.mode != SessionMode::DirectCdp || reference.principal != s.handle.principal {
        return Err("file selection requires the matching direct CDP session".into());
    }
    let locator =
        controlla_browser::Locator::from_value(&args.locator).map_err(|e| e.to_string())?;
    let handle: controlla_browser::sessions::ArtifactHandle =
        serde_json::from_value(json!(args.artifact_handle))
            .map_err(|_| "artifact_handle is invalid".to_owned())?;
    let registry = s.registry.lock().await;
    if !registry.contains_target(&s.handle, &reference.target_id) {
        return Err("target_ref is not bound to this session".into());
    }
    let revisions = IdentityRevisions {
        account: reference.account_revision,
        document: reference.document_revision,
    };
    let resolved = s
        .connection
        .resolve_target_ref(
            &registry,
            &reference,
            &s.handle.principal,
            revisions.account,
            revisions.document,
        )
        .await
        .map_err(|e| format!("target_ref is stale: {e:?}"))?;
    let marker_check = identity_marker_command(&args.account_marker.0, &args.account_marker.1)
        .map_err(|e| format!("invalid account marker: {e}"))?;
    verify_file_select_marker(&s, &registry, &reference, revisions, &marker_check)
        .await
        .map_err(|error| format!("file selection withheld: {error}"))?;
    let expected = GuardSnapshot {
        navigation: reference.frame_revision,
        account: reference.account_revision,
        document: reference.document_revision,
        dependencies: Default::default(),
        strict_background: false,
        requires_native: false,
    };
    let current = GuardSnapshot {
        navigation: resolved.frame.revision,
        ..expected.clone()
    };
    let evidence = s
        .connection
        .select_file_input_artifact(
            &registry,
            &reference,
            &s.handle.principal,
            revisions,
            GuardedFileSelection {
                expected: &expected,
                current: &current,
                locator: &locator,
                handle: &handle,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    verify_file_select_marker(&s, &registry, &reference, revisions, &marker_check)
        .await
        .map_err(|error| {
            format!("file may already be selected; post-selection identity check failed: {error}")
        })?;
    let mut result = serde_json::to_value(evidence).map_err(|e| e.to_string())?;
    result["identity_guard"] = json!({
        "selector": args.account_marker.0,
        "expected_text": args.account_marker.1,
        "evidence": "caller-declared DOM marker matched in preselection and postselection samples; no atomic account binding or app acceptance is established"
    });
    Ok(result)
}

async fn verify_file_select_marker(
    s: &LiveSession,
    registry: &SessionRegistry,
    reference: &TargetRef,
    revisions: IdentityRevisions,
    command: &Value,
) -> Result<(), String> {
    let response = s
        .connection
        .target_ref_command(
            registry,
            reference,
            &s.handle.principal,
            revisions,
            "Runtime.evaluate",
            command.clone(),
        )
        .await
        .map_err(|error| format!("identity marker check failed: {error}"))?;
    if response.get("exceptionDetails").is_some() {
        return Err("identity marker check threw; file selection withheld or unverifiable".into());
    }
    match response.pointer("/result/value") {
        Some(Value::Bool(true)) => Ok(()),
        Some(Value::Bool(false)) => {
            Err("caller-declared identity marker is missing or mismatched".into())
        }
        _ => Err(
            "identity marker check returned no boolean; file selection withheld or unverifiable"
                .into(),
        ),
    }
}

async fn shared_frame_identity(
    connection: &controlla_browser::providers::SharedExtensionSession,
    registry: &SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
) -> Result<(String, String, String), String> {
    let response = connection
        .command(registry, handle, tab_id, "Page.getFrameTree", Value::Null)
        .await
        .map_err(|e| e.to_string())?;
    let frame = response
        .get("frameTree")
        .and_then(|tree| tree.get("frame"))
        .ok_or_else(|| "Page.getFrameTree omitted root frame".to_owned())?;
    let id = frame["id"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Page.getFrameTree omitted root frame ID".to_owned())?;
    let loader = frame["loaderId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Page.getFrameTree omitted root loader ID".to_owned())?;
    let url = frame["url"]
        .as_str()
        .ok_or_else(|| "Page.getFrameTree omitted root frame URL".to_owned())?;
    Ok((id.to_owned(), loader.to_owned(), url.to_owned()))
}

#[tool_router(server_handler)]
impl App {
    #[tool(
        name = "artifact_register",
        description = "Register bounded bytes under an opaque artifact handle scoped to this direct CDP session."
    )]
    async fn artifact_register(
        &self,
        Parameters(args): Parameters<ArtifactRegisterArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        if args.bytes.is_empty() || args.bytes.len() > 10 * 1024 * 1024 {
            return Err(invalid(
                "artifact bytes must be between 1 and 10485760 bytes",
            ));
        }
        let s = self
            .sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        if s.handle.mode != SessionMode::DirectCdp {
            return Err(invalid(
                "artifact registration is currently available only for direct CDP sessions",
            ));
        }
        let metadata = s
            .registry
            .lock()
            .await
            .put_artifact_bytes(&s.handle, &args.filename, &args.bytes)
            .map_err(|e| invalid(format!("artifact registration failed: {e:?}")))?;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "handle": metadata.handle, "filename": metadata.filename, "size": metadata.size,
            "evidence": "registered in the ephemeral principal/session-scoped store"
        })))
    }

    #[tool(
        name = "file_select",
        description = "Select a registered artifact after sampling a caller-declared account marker before and after selection. The samples are not atomic with selection; success proves Chrome selection/readback, not app acceptance."
    )]
    async fn file_select(
        &self,
        Parameters(args): Parameters<FileSelectArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let s = self
            .sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        Ok(rmcp::handler::server::wrapper::Json(
            run_file_select(s, args).await.map_err(invalid)?,
        ))
    }

    #[tool(
        name = "accessibility",
        description = "Return a bounded Chrome accessibility tree for an explicit target reference."
    )]
    async fn accessibility(
        &self,
        Parameters(args): Parameters<AccessibilityArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let session_id = args.session_id.as_str();
        let reference: TargetRef = serde_json::from_value(
            serde_json::to_value(args.target_ref).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|_| invalid("target_ref must be a complete current TargetRef"))?;
        let s = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        let registry = s.registry.lock().await;
        if reference.session_id != session_id
            || reference.principal != s.handle.principal
            || !registry.contains_target(&s.handle, &reference.target_id)
        {
            return Err(invalid("target_ref is not bound to this session"));
        }
        let result = s
            .connection
            .observe_accessibility(
                &registry,
                &reference,
                &s.handle.principal,
                IdentityRevisions {
                    account: reference.account_revision,
                    document: reference.document_revision,
                },
                &args.selector,
                args.max_bytes,
            )
            .await
            .map_err(|e| invalid(e.to_string()))?;
        Ok(rmcp::handler::server::wrapper::Json(
            serde_json::to_value(result).map_err(|e| invalid(e.to_string()))?,
        ))
    }

    #[tool(
        name = "screenshot_crop",
        description = "Capture a bounded PNG crop from an explicit Chrome target reference."
    )]
    async fn screenshot_crop(
        &self,
        Parameters(args): Parameters<ScreenshotArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let session_id = args.session_id.as_str();
        let reference: TargetRef = serde_json::from_value(
            serde_json::to_value(args.target_ref).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|_| invalid("target_ref must be a complete current TargetRef"))?;
        let s = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        let registry = s.registry.lock().await;
        if reference.session_id != session_id
            || reference.principal != s.handle.principal
            || !registry.contains_target(&s.handle, &reference.target_id)
        {
            return Err(invalid("target_ref is not bound to this session"));
        }
        let result = s
            .connection
            .observe_screenshot(
                &registry,
                &reference,
                &s.handle.principal,
                IdentityRevisions {
                    account: reference.account_revision,
                    document: reference.document_revision,
                },
                ScreenshotCrop {
                    x: args.x,
                    y: args.y,
                    width: args.width,
                    height: args.height,
                    scale: args.scale,
                },
                args.max_bytes,
            )
            .await
            .map_err(|e| invalid(e.to_string()))?;
        Ok(rmcp::handler::server::wrapper::Json(
            serde_json::to_value(result).map_err(|e| invalid(e.to_string()))?,
        ))
    }

    #[tool(
        name = "session",
        description = "Discover/connect to configured Chrome and list explicit targets."
    )]
    async fn session(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let action = args.action.as_str();
        match action {
            "pair_shared" => {
                let target_ids = args.target_ids.ok_or_else(|| invalid(
                    "target_ids must explicitly select decimal Chrome tab IDs from the extension popup",
                ))?;
                if target_ids.is_empty()
                    || target_ids.iter().any(|id| {
                        !id.parse::<u32>()
                            .is_ok_and(|parsed| parsed.to_string() == *id)
                    })
                    || target_ids
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != target_ids.len()
                {
                    return Err(invalid(
                        "target_ids must be unique, non-empty decimal Chrome tab IDs",
                    ));
                }
                let mut registry = SessionRegistry::new(ProviderGrants {
                    shared_extension: true,
                    ..Default::default()
                });
                let handle = registry
                    .create_session(
                        SessionSpec {
                            mode: SessionMode::Shared,
                            selected_target_ids: target_ids.clone(),
                        },
                        "local-stdio",
                    )
                    .map_err(|e| invalid(format!("shared session denied: {e:?}")))?;
                let provider = controlla_browser::providers::SharedExtensionProvider::bind(
                    &mut registry,
                    &handle,
                )
                .await
                .map_err(|e| invalid(e.to_string()))?;
                let pairing = provider.pairing().clone();
                let session_id = handle.id.clone();
                self.shared_sessions.lock().await.insert(
                    session_id.clone(),
                    Arc::new(Mutex::new(SharedLiveSession {
                        provider: Some(provider),
                        connection: None,
                        registry,
                        handle,
                    })),
                );
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "session_id":session_id, "target_ids":target_ids,
                    "endpoint":pairing.endpoint, "one_session_token":pairing.token,
                    "next":"Call session action accept_shared, then use the unpacked Chrome extension popup to check exactly these tab IDs, enter endpoint and token, and pair. This extension route uses chrome.debugger and does not require Chrome DevTools remote debugging to be enabled.",
                    "identity":"Chrome tab IDs are not Direct CDP target references"
                })))
            }
            "accept_shared" => {
                let id = args
                    .session_id
                    .as_deref()
                    .ok_or_else(|| invalid("session_id is required"))?;
                let shared = self
                    .shared_sessions
                    .lock()
                    .await
                    .get(id)
                    .cloned()
                    .ok_or_else(|| invalid("unknown shared session_id"))?;
                let mut shared = shared.lock().await;
                if shared.connection.is_some() {
                    return Err(invalid("shared extension session is already accepted"));
                }
                let mut provider = shared
                    .provider
                    .take()
                    .ok_or_else(|| invalid("shared pairing is no longer pending"))?;
                let connection = match provider.accept(&mut shared.registry).await {
                    Ok(connection) => connection,
                    Err(error) => {
                        shared.provider = Some(provider);
                        return Err(invalid(error.to_string()));
                    }
                };
                shared.connection = Some(connection);
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "session_id":id, "accepted":true,
                    "target_ids":shared.connection.as_ref().unwrap().selected_targets(),
                    "next":"Call list_shared_targets to inventory selected tabs or shared_observe for a bounded read. Extraction and input remain Direct CDP only."
                })))
            }
            "list_shared_targets" => {
                let id = args
                    .session_id
                    .as_deref()
                    .ok_or_else(|| invalid("session_id is required"))?;
                let shared = self
                    .shared_sessions
                    .lock()
                    .await
                    .get(id)
                    .cloned()
                    .ok_or_else(|| invalid("unknown shared session_id"))?;
                let shared = shared.lock().await;
                let connection = shared
                    .connection
                    .as_ref()
                    .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
                let mut targets = Vec::new();
                for target_id in connection.selected_targets() {
                    let frame_tree = connection
                        .command(
                            &shared.registry,
                            &shared.handle,
                            target_id,
                            "Page.getFrameTree",
                            Value::Null,
                        )
                        .await
                        .map_err(|e| invalid(e.to_string()))?;
                    targets.push(json!({"chrome_tab_id":target_id,"frame_tree":frame_tree}));
                }
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "session_id":id,"targets":targets,
                    "identity":"point-in-time extension tab inventory; no BrowserConnection TargetRef"
                })))
            }
            "release_shared" => {
                let id = args
                    .session_id
                    .as_deref()
                    .ok_or_else(|| invalid("session_id is required"))?;
                let shared = self
                    .shared_sessions
                    .lock()
                    .await
                    .get(id)
                    .cloned()
                    .ok_or_else(|| invalid("unknown shared session_id"))?;
                let mut shared = shared.lock().await;
                if let Some(connection) = shared.connection.as_mut() {
                    connection
                        .release()
                        .await
                        .map_err(|e| invalid(e.to_string()))?;
                }
                shared.connection = None;
                shared.provider = None;
                drop(shared);
                self.shared_sessions.lock().await.remove(id);
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "session_id":id,"released":true,"effect":"released this provider's debugger attachments"
                })))
            }
            "discover" => {
                let mut providers = list_sessions();
                if let Some(explicit) = providers
                    .iter_mut()
                    .find(|p| p.provider == SessionProvider::ExplicitCdp)
                {
                    let configured = std::env::var("COMPTROL_CDP_ENDPOINT")
                        .ok()
                        .filter(|url| validate_loopback_ws(url).is_ok());
                    explicit.available = std::env::var("COMPTROL_ALLOW_DIRECT_CDP").as_deref()
                        == Ok("1")
                        && configured.is_some();
                    explicit.reason = if explicit.available {
                        "explicit loopback endpoint is configured and Direct CDP opt-in is enabled"
                    } else {
                        "requires a loopback COMPTROL_CDP_ENDPOINT and COMPTROL_ALLOW_DIRECT_CDP=1"
                    }
                    .to_owned();
                }
                Ok(rmcp::handler::server::wrapper::Json(
                    json!({"providers":providers}),
                ))
            }
            "targets" => {
                let endpoint =
                    configured_endpoint(args.provider.as_deref().unwrap_or("explicit_cdp"))
                        .map_err(invalid)?;
                validate_loopback_ws(&endpoint).map_err(invalid)?;
                let connection = self
                    .manager
                    .connect(&endpoint)
                    .await
                    .map_err(|e| invalid(e.to_string()))?;
                let (generation, targets) = connection.target_snapshot().await;
                Ok(rmcp::handler::server::wrapper::Json(
                    json!({"browser_generation":generation,"targets":targets.into_iter().filter(|target| target.target_type == "page").collect::<Vec<_>>()}),
                ))
            }
            "connect" => {
                let provider = args.provider.as_deref().unwrap_or("explicit_cdp");
                let endpoint = configured_endpoint(provider).map_err(invalid)?;
                validate_loopback_ws(&endpoint).map_err(|e| invalid(&e))?;
                let connection = self
                    .manager
                    .connect(&endpoint)
                    .await
                    .map_err(|e| invalid(e.to_string()))?;
                let target_ids = args.target_ids.ok_or_else(|| {
                    invalid("target_ids must explicitly select one or more listed targets")
                })?;
                let (_generation, targets) = connection.target_snapshot().await;
                if target_ids.is_empty()
                    || target_ids.iter().any(|id| {
                        !targets
                            .iter()
                            .any(|t| &t.id == id && t.target_type == "page")
                    })
                {
                    return Err(invalid(format!(
                        "every selected target_id must match a current page target; requested={target_ids:?}, current={:?}",
                        targets
                            .iter()
                            .filter(|t| t.target_type == "page")
                            .map(|t| (&t.id, &t.target_type))
                            .collect::<Vec<_>>()
                    )));
                }
                let grants = ProviderGrants {
                    direct_cdp: true,
                    ..Default::default()
                };
                let mut registry = SessionRegistry::new(grants);
                let handle = registry
                    .create_session(
                        SessionSpec {
                            mode: SessionMode::DirectCdp,
                            selected_target_ids: target_ids.clone(),
                        },
                        "local-stdio",
                    )
                    .map_err(|e| invalid(format!("session denied: {e:?}")))?;
                registry
                    .bind_session_to_browser(&handle, connection.instance_id())
                    .map_err(|e| invalid(format!("browser binding failed: {e:?}")))?;
                let id = handle.id.clone();
                self.sessions.lock().await.insert(
                    id.clone(),
                    Arc::new(LiveSession {
                        connection,
                        registry: Mutex::new(registry),
                        handle,
                    }),
                );
                Ok(rmcp::handler::server::wrapper::Json(
                    json!({"session_id":id,"targets":target_ids,"provider":provider,"consent":"Chrome may show its native Allow prompt; this server does not bypass it"}),
                ))
            }
            "list_targets" => {
                let id = args
                    .session_id
                    .as_deref()
                    .ok_or_else(|| invalid("session_id is required"))?;
                let s = self
                    .sessions
                    .lock()
                    .await
                    .get(id)
                    .cloned()
                    .ok_or_else(|| invalid("unknown session_id"))?;
                let (_, targets) = s.connection.target_snapshot().await;
                let frames = s.connection.frames.read().await;
                let registry = s.registry.lock().await;
                let mut listed = Vec::new();
                for target in targets.into_iter().filter(|t| {
                    t.target_type == "page" && registry.contains_target(&s.handle, &t.id)
                }) {
                    let frame = frames
                        .frames
                        .values()
                        .find(|f| f.target_id == target.id && f.parent_id.is_none());
                    if let Some(frame) = frame
                        && let Ok(reference) = s
                            .connection
                            .capture_target_ref(&registry, &s.handle, &target.id, &frame.id, 0, 0)
                            .await
                    {
                        listed.push(json!({"target":target,"target_ref":reference}));
                    }
                }
                Ok(rmcp::handler::server::wrapper::Json(
                    json!({"session_id":id,"targets":listed}),
                ))
            }
            _ => Err(invalid(
                "action must be pair_shared, accept_shared, list_shared_targets, release_shared, discover, targets, connect, or list_targets",
            )),
        }
    }

    #[tool(
        name = "observe",
        description = "Read a bounded DOM observation from an explicit target reference."
    )]
    async fn observe(
        &self,
        Parameters(args): Parameters<ObserveArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let session_id = args.session_id.as_str();
        let reference: TargetRef = serde_json::from_value(
            serde_json::to_value(args.target_ref).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|_| invalid("target_ref must be a complete current TargetRef"))?;
        let spec = ObserveSpec {
            selector: args.spec.selector,
            fields: args.spec.fields,
            max_items: args.spec.max_items,
            max_text_chars: args.spec.max_text_chars,
            max_bytes: args.spec.max_bytes,
            cursor: args.spec.cursor,
        };
        let s = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        let registry = s.registry.lock().await;
        if reference.session_id != session_id
            || reference.principal != s.handle.principal
            || !registry.contains_target(&s.handle, &reference.target_id)
        {
            return Err(invalid("target_ref is not bound to this session"));
        }
        let result = s
            .connection
            .observe(
                &registry,
                &reference,
                &s.handle.principal,
                IdentityRevisions {
                    account: reference.account_revision,
                    document: reference.document_revision,
                },
                &spec,
            )
            .await
            .map_err(|e| invalid(e.to_string()))?;
        Ok(rmcp::handler::server::wrapper::Json(
            serde_json::to_value(result).unwrap_or(Value::Null),
        ))
    }

    #[tool(
        name = "shared_observe",
        description = "Read a bounded observation from one explicitly paired Chrome tab. Freshness is checked by matching root frame, loader, and URL before and after the read; this route does not create a Direct CDP TargetRef."
    )]
    async fn shared_observe(
        &self,
        Parameters(args): Parameters<SharedObserveArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let spec = ObserveSpec {
            selector: args.spec.selector,
            fields: args.spec.fields,
            max_items: args.spec.max_items,
            max_text_chars: args.spec.max_text_chars,
            max_bytes: args.spec.max_bytes,
            cursor: args.spec.cursor,
        };
        let shared = self
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let shared = shared.lock().await;
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let before = shared_frame_identity(
            connection,
            &shared.registry,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
        .map_err(invalid)?;
        let params = observation_command(&spec).map_err(|e| invalid(e.to_string()))?;
        let response = connection
            .command(
                &shared.registry,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.evaluate",
                params,
            )
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let after = shared_frame_identity(
            connection,
            &shared.registry,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
        .map_err(invalid)?;
        if before != after {
            return Err(invalid(
                "shared target navigated during observation; result discarded as stale",
            ));
        }
        let mut result = serde_json::to_value(
            parse_observation(&args.chrome_tab_id, 0, &spec, &response)
                .map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| invalid(e.to_string()))?;
        result["navigation_epoch"] = Value::Null;
        result["shared_frame_identity"] = json!({
            "frame_id":before.0,"loader_id":before.1,
            "freshness":"same root frame, loader, and URL before and after this read"
        });
        if serde_json::to_vec(&result)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX)
            > spec.max_bytes
        {
            return Err(invalid(
                "byte budget is too small for shared frame identity metadata",
            ));
        }
        Ok(rmcp::handler::server::wrapper::Json(result))
    }

    #[tool(
        name = "extract",
        description = "Bounded extraction across caller-declared sections; every section reports completeness independently."
    )]
    async fn extract(
        &self,
        Parameters(args): Parameters<ExtractArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let session_id = args.session_id.as_str();
        let reference: TargetRef = serde_json::from_value(
            serde_json::to_value(args.target_ref).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|_| invalid("target_ref must be a complete current TargetRef"))?;
        let sections = args.sections;
        // ponytail: eight sections keeps coverage metadata inside the minimum 4 KiB aggregate budget; raise with a matching metadata-budget test.
        if sections.is_empty() || sections.len() > 8 {
            return Err(invalid("sections must contain 1..=8 entries"));
        }
        let global_records = args.max_records;
        if !(1..=1000).contains(&global_records) {
            return Err(invalid("max_records must be 1..=1000"));
        }
        let global_bytes = args.max_bytes;
        if !(4096..=1_000_000).contains(&global_bytes) {
            return Err(invalid("max_bytes must be 4096..=1000000"));
        }
        let timeout_ms = args.timeout_ms.unwrap_or(30_000);
        if !(1..=30_000).contains(&timeout_ms) {
            return Err(invalid("timeout_ms must be 1..=30000"));
        }
        let s = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        let registry = s.registry.lock().await;
        if reference.session_id != session_id
            || reference.principal != s.handle.principal
            || !registry.contains_target(&s.handle, &reference.target_id)
        {
            return Err(invalid("target_ref is not bound to this session"));
        }
        let mut records = BTreeMap::new();
        let mut results = Vec::new();
        let section_count = sections.len();
        let base_record_budget = global_records / section_count;
        let record_remainder = global_records % section_count;
        let section_byte_budget = global_bytes / section_count;
        let mut all_complete = true;
        let mut any_unknown = false;
        let started = tokio::time::Instant::now();
        for (section_index, section) in sections.iter().enumerate() {
            let name = section.section_id.as_str();
            if name.is_empty() {
                return Err(invalid("each section requires section_id"));
            }
            let mut spec = ExtractionSpec {
                container: section.spec.container.clone(),
                record: section.spec.record.clone(),
                fields: section.spec.fields.clone(),
                id_field: section.spec.id_field.clone(),
                max_steps: section.spec.max_steps,
                max_records: section.spec.max_records,
                max_text_chars: section.spec.max_text_chars,
                max_bytes: section.spec.max_bytes,
                expected_count: section.spec.expected_count,
                account_marker: section.spec.account_marker.clone(),
                terminal_selector: section.spec.terminal_selector.clone(),
                expand: section
                    .spec
                    .expand
                    .iter()
                    .map(|e| ExpansionControl {
                        selector: e.selector.clone(),
                        content_selector: e.content_selector.clone(),
                    })
                    .collect(),
                cursor: section.spec.cursor.clone(),
            };
            let left_ms = timeout_ms.saturating_sub(started.elapsed().as_millis() as u64);
            let section_records =
                base_record_budget + usize::from(section_index < record_remainder);
            let section_bytes = section_byte_budget.saturating_sub(512);
            if section_records == 0 || left_ms == 0 || section_bytes < 4096 {
                let reason = if section_records == 0 {
                    "fixed per-section record share is zero"
                } else if left_ms == 0 {
                    "global extraction deadline exhausted"
                } else {
                    "fixed per-section byte share cannot fit the 4096-byte page minimum and response metadata"
                };
                results
                    .push(json!({"section_id":name,"completeness":"partial","missing":[reason]}));
                all_complete = false;
                continue;
            }
            spec.max_records = spec.max_records.min(section_records);
            spec.max_bytes = spec.max_bytes.min(section_bytes);
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(left_ms),
                s.connection.extract(
                    &registry,
                    &reference,
                    &s.handle.principal,
                    IdentityRevisions {
                        account: reference.account_revision,
                        document: reference.document_revision,
                    },
                    &spec,
                ),
            )
            .await;
            let result = match result {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => {
                    let message = error.to_string().chars().take(240).collect::<String>();
                    results.push(
                        json!({"section_id":name,"completeness":"unknown","missing":[message]}),
                    );
                    all_complete = false;
                    any_unknown = true;
                    continue;
                }
                Err(_) => {
                    results.push(json!({"section_id":name,"completeness":"unknown","missing":["global extraction deadline exceeded"]}));
                    all_complete = false;
                    any_unknown = true;
                    for pending in sections.iter().skip(results.len()) {
                        results.push(json!({"section_id":pending.section_id,"completeness":"unknown","missing":["not visited because the global extraction deadline was exceeded"]}));
                    }
                    break;
                }
            };
            all_complete &= result.completeness == controlla_browser::Completeness::Complete;
            any_unknown |= result.completeness == controlla_browser::Completeness::Unknown;
            for record in &result.records {
                if let Some(id) = record.get(&spec.id_field) {
                    records
                        .entry((name.to_owned(), id.clone()))
                        .or_insert_with(|| json!({"section_id":name,"fields":record}));
                }
            }
            results.push(json!({"section_id":name,"completeness":result.completeness,"unique_count":result.unique_count,"expected_count":result.expected_count,"cursor":result.cursor,"cursor_is_resumable":result.cursor_is_resumable,"terminal_evidence":result.terminal_evidence,"missing":result.missing,"truncated":result.truncated,"navigation_epoch":result.navigation_epoch}));
        }
        let unique_count = records.len();
        let overall = if all_complete {
            "complete"
        } else if any_unknown || unique_count == 0 {
            "unknown"
        } else {
            "partial"
        };
        let output = json!({"records":records.into_values().collect::<Vec<_>>(),"unique_count":unique_count,"completeness":overall,"sections":results,"global_record_limit":global_records,"global_byte_limit":global_bytes,"timeout_ms":timeout_ms});
        let output = fit_aggregate(output, global_bytes).map_err(invalid)?;
        Ok(rmcp::handler::server::wrapper::Json(output))
    }
}

fn fit_aggregate(mut output: Value, max_bytes: usize) -> Result<Value, String> {
    while serde_json::to_vec(&output)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
        > max_bytes
    {
        let Some(rows) = output["records"].as_array_mut() else {
            break;
        };
        let Some(removed) = rows.pop() else { break };
        let section_id = removed["section_id"].as_str().map(str::to_owned);
        output["unique_count"] = json!(rows.len());
        output["completeness"] = json!("partial");
        if let Some(section_id) = section_id
            && let Some(section) = output["sections"]
                .as_array_mut()
                .and_then(|all| all.iter_mut().find(|s| s["section_id"] == section_id))
        {
            section["completeness"] = json!("partial");
            section["truncated"] = json!(true);
            if let Some(missing) = section["missing"].as_array_mut() && !missing.iter().any(|m| {
                m == "global aggregate byte budget omitted one or more records from this section"
            }) {
                missing.push(json!(
                    "global aggregate byte budget omitted one or more records from this section"
                ));
            }
        }
    }
    if serde_json::to_vec(&output)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
        > max_bytes
    {
        return Err("global byte budget is too small for explicit coverage metadata".into());
    }
    Ok(output)
}

fn invalid(message: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(message.into(), None)
}
fn configured_endpoint(provider: &str) -> Result<String, String> {
    match provider {
        "explicit_cdp" => {
            if std::env::var("COMPTROL_ALLOW_DIRECT_CDP").as_deref() != Ok("1") {
                return Err("explicit CDP is disabled; set COMPTROL_ALLOW_DIRECT_CDP=1".into());
            }
            std::env::var("COMPTROL_CDP_ENDPOINT")
                .map_err(|_| "COMPTROL_CDP_ENDPOINT is not configured".into())
        }
        "permissioned_auto_connect" => {
            if std::env::var("COMPTROL_CHROME_AUTO_CONNECT").as_deref() != Ok("1") {
                return Err("auto-connect is not armed; set COMPTROL_CHROME_AUTO_CONNECT=1".into());
            }
            connect_permissioned_auto_connect()
        }
        _ => Err("provider must be explicit_cdp or permissioned_auto_connect".into()),
    }
}
fn validate_loopback_ws(endpoint: &str) -> Result<(), String> {
    let u = endpoint
        .strip_prefix("ws://")
        .ok_or("only ws:// loopback endpoints are accepted")?;
    if u.contains('@') || u.contains('#') || u.contains('?') {
        return Err("credentials, fragments, and query strings are rejected".into());
    }
    let (authority, path) = u.split_once('/').unwrap_or((u, ""));
    if !path.starts_with("devtools/") || path.contains("..") {
        return Err("expected a Chrome DevTools WebSocket path".into());
    }
    let (host, port) = if authority.starts_with('[') {
        let (host, rest) = authority
            .split_once("]:")
            .ok_or("endpoint requires a port")?;
        (host.trim_start_matches('['), rest)
    } else {
        authority
            .rsplit_once(':')
            .ok_or("endpoint requires a port")?
    };
    let port = port
        .parse::<u16>()
        .map_err(|_| "endpoint port must be numeric")?;
    if port == 0 {
        return Err("endpoint port must be nonzero".into());
    }
    let ip: IpAddr = host
        .parse()
        .map_err(|_| "endpoint host must be a loopback IP literal")?;
    if !ip.is_loopback() {
        return Err("endpoint must be loopback".into());
    }
    Ok(())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        App::default()
            .serve(rmcp::transport::io::stdio())
            .await?
            .waiting()
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

#[cfg(test)]
mod tests {
    use super::{App, validate_loopback_ws};
    use rmcp::{RoleServer, ServiceExt, model::CallToolRequestParams, service::serve_directly};
    use serde_json::{Value, json};

    #[test]
    fn only_loopback_devtools_websockets_are_accepted() {
        assert!(validate_loopback_ws("ws://127.0.0.1:9222/devtools/browser/test").is_ok());
        assert!(validate_loopback_ws("ws://[::1]:9222/devtools/browser/test").is_ok());
        for denied in [
            "ws://example.test:9222/devtools/browser/test",
            "ws://192.168.1.3:9222/devtools/browser/test",
            "ws://user@127.0.0.1:9222/devtools/browser/test",
            "ws://127.0.0.1:9222/devtools/browser/../other",
            "ws://127.0.0.1:nope/devtools/browser/test",
            "ws://127.0.0.1:0/devtools/browser/test",
            "http://127.0.0.1:9222/devtools/browser/test",
        ] {
            assert!(validate_loopback_ws(denied).is_err(), "accepted {denied}");
        }
    }

    #[test]
    fn aggregate_byte_truncation_marks_the_removed_record_section_partial() {
        let output = json!({
            "records":[{"section_id":"alpha","fields":{"value":"x".repeat(10_000)}}],
            "unique_count":1,"completeness":"complete",
            "sections":[
                {"section_id":"alpha","completeness":"complete","truncated":false,"missing":[]},
                {"section_id":"beta","completeness":"complete","truncated":false,"missing":[]}
            ]
        });
        let fitted = super::fit_aggregate(output, 4096).unwrap();
        assert_eq!(fitted["completeness"], "partial");
        assert_eq!(fitted["sections"][0]["completeness"], "partial");
        assert_eq!(fitted["sections"][0]["truncated"], true);
        assert_eq!(fitted["sections"][1]["completeness"], "complete");
        assert!(
            fitted["sections"][0]["missing"][0]
                .as_str()
                .unwrap()
                .contains("omitted")
        );
        assert!(serde_json::to_vec(&fitted).unwrap().len() <= 4096);
    }

    #[tokio::test]
    async fn stdio_server_handler_roundtrips_initialize_list_and_discovery_call() {
        let (server_io, client_io) = tokio::io::duplex(16_384);
        let server = serve_directly::<RoleServer, _, _, _, _>(App::default(), server_io, None);
        let server_task = tokio::spawn(async move { server.waiting().await });
        let client = ().serve(client_io).await.unwrap();
        let listed = client.list_tools(None).await.unwrap();
        let names = listed
            .tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert!(names.contains(&"session"));
        assert!(names.contains(&"observe"));
        assert!(names.contains(&"extract"));
        assert!(names.contains(&"accessibility"));
        assert!(names.contains(&"screenshot_crop"));
        assert!(names.contains(&"artifact_register"));
        assert!(names.contains(&"file_select"));
        assert!(names.contains(&"shared_observe"));
        for name in [
            "session",
            "observe",
            "extract",
            "accessibility",
            "screenshot_crop",
            "artifact_register",
            "file_select",
            "shared_observe",
        ] {
            let tool = listed.tools.iter().find(|tool| tool.name == name).unwrap();
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            assert_eq!(schema["type"], "object", "{name} schema root");
            assert!(schema["properties"].is_object(), "{name} schema properties");
            assert!(
                schema["required"]
                    .as_array()
                    .is_some_and(|required| !required.is_empty()),
                "{name} has required fields"
            );
        }
        let file_select_schema = listed
            .tools
            .iter()
            .find(|tool| tool.name == "file_select")
            .unwrap();
        let file_select_schema = serde_json::to_value(&file_select_schema.input_schema).unwrap();
        assert!(
            file_select_schema["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|field| field == "account_marker"))
        );
        let result = client
            .call_tool(
                CallToolRequestParams::new("session")
                    .with_arguments(json!({"action":"discover"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false));
        assert!(
            client
                .call_tool(
                    CallToolRequestParams::new("session")
                        .with_arguments(json!({"action":"unknown"}).as_object().unwrap().clone(),)
                )
                .await
                .is_err()
        );
        client.cancel().await.unwrap();
        let _ = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn observe_and_multisection_extract_calls_reach_mock_cdp() {
        use futures_util::{SinkExt, StreamExt};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tokio::net::TcpListener;
        use tokio_tungstenite::{accept_async, tungstenite::Message};
        static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _env_guard = ENV_LOCK.lock().await;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("ws://{address}/devtools/browser/mock");
        let full_ax_calls = Arc::new(AtomicUsize::new(0));
        let screenshot_calls = Arc::new(AtomicUsize::new(0));
        let ax_calls_for_server = full_ax_calls.clone();
        let screenshot_calls_for_server = screenshot_calls.clone();
        let earlier_calls = Arc::new(AtomicUsize::new(0));
        let earlier_calls_for_server = earlier_calls.clone();
        let file_select_calls = Arc::new(AtomicUsize::new(0));
        let file_select_calls_for_server = file_select_calls.clone();
        let cdp = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(Ok(message)) = ws.next().await {
                let Ok(request) = serde_json::from_str::<Value>(&message.to_string()) else {
                    continue;
                };
                let id = request["id"].as_u64();
                let method = request["method"].as_str().unwrap_or_default();
                let result = match method {
                    "Target.getTargets" => {
                        json!({"targetInfos":[{"targetId":"tab-1","type":"page","url":"https://fixture.test/","title":"Fixture"},{"targetId":"worker-1","type":"service_worker","url":"https://fixture.test/sw.js","title":"Fixture worker"}]})
                    }
                    "Target.attachToTarget" => json!({"sessionId":"session-1"}),
                    "Runtime.evaluate" => {
                        let expr = request["params"]["expression"].as_str().unwrap_or_default();
                        if expr.contains("const q=") {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                        }
                        let value = if expr.contains("const m=") {
                            json!(!expr.contains("missing-marker"))
                        } else if expr.contains("missingFields") {
                            let items = if expr.contains("#bucket-left")
                                || expr.contains("#bucket-right")
                                || expr.contains("#later")
                            {
                                vec![json!({"id":"duplicate"})]
                            } else if expr.contains("#earlier")
                                && earlier_calls_for_server.fetch_add(1, Ordering::SeqCst) > 0
                            {
                                vec![json!({"id":"changed-earlier"})]
                            } else {
                                vec![]
                            };
                            json!({"items":items,"end":true,"before":0,"after":0,"account":true,"limited":false,"missingFields":[],"clipped":false})
                        } else {
                            json!({"items":[],"count":0,"missing":[],"clipped":0,"limited":false})
                        };
                        json!({"result":{"type":"object","value":value}})
                    }
                    "DOM.getDocument" => json!({"root":{"nodeId":1}}),
                    "DOM.setFileInputFiles" => {
                        file_select_calls_for_server.fetch_add(1, Ordering::SeqCst);
                        json!({})
                    }
                    "DOM.querySelector" => {
                        assert_eq!(request["params"]["selector"], "#fixture");
                        json!({"nodeId":2})
                    }
                    "Accessibility.getPartialAXTree" => {
                        assert!(
                            request["params"]["nodeId"]
                                .as_i64()
                                .is_some_and(|id| id > 0)
                        );
                        assert_eq!(request["params"]["fetchRelatives"], false);
                        json!({"nodes":[{"nodeId":"1","role":{"value":"button"}}]})
                    }
                    "Accessibility.getFullAXTree" => {
                        ax_calls_for_server.fetch_add(1, Ordering::SeqCst);
                        json!({"nodes":[{"nodeId":"1","role":{"value":"RootWebArea"}}]})
                    }
                    "Page.captureScreenshot" => {
                        screenshot_calls_for_server.fetch_add(1, Ordering::SeqCst);
                        json!({"data":"aGVsbG8="})
                    }
                    _ => json!({}),
                };
                if let Some(id) = id {
                    ws.send(Message::Text(
                        json!({"id":id,"result":result}).to_string().into(),
                    ))
                    .await
                    .unwrap();
                }
                if method == "Page.enable" {
                    ws.send(Message::Text(json!({"sessionId":"session-1","method":"Page.frameNavigated","params":{"frame":{"id":"frame-1","loaderId":"load-1","url":"https://fixture.test/"}}}).to_string().into())).await.unwrap();
                }
            }
        });
        unsafe {
            std::env::set_var("COMPTROL_ALLOW_DIRECT_CDP", "1");
            std::env::set_var("COMPTROL_CDP_ENDPOINT", &endpoint);
        }

        let (server_io, client_io) = tokio::io::duplex(32_768);
        let server = serve_directly::<RoleServer, _, _, _, _>(App::default(), server_io, None);
        let server_task = tokio::spawn(async move { server.waiting().await });
        let client = ().serve(client_io).await.unwrap();
        let args = |v: Value| v.as_object().unwrap().clone();
        let non_page = client
            .call_tool(CallToolRequestParams::new("session").with_arguments(args(
                json!({"action":"connect","provider":"explicit_cdp","target_ids":["worker-1"]}),
            )))
            .await;
        assert!(non_page.is_err());
        let connected = client
            .call_tool(CallToolRequestParams::new("session").with_arguments(args(
                json!({"action":"connect","provider":"explicit_cdp","target_ids":["tab-1"]}),
            )))
            .await
            .unwrap();
        assert!(!connected.is_error.unwrap_or(false), "{connected:?}");
        let session_id = connected.structured_content.as_ref().unwrap()["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let artifact = client
            .call_tool(
                CallToolRequestParams::new("artifact_register").with_arguments(args(
                    json!({"session_id":session_id,"filename":"fixture.txt","bytes":[104,105]}),
                )),
            )
            .await
            .unwrap();
        assert!(!artifact.is_error.unwrap_or(false), "{artifact:?}");
        assert_eq!(artifact.structured_content.as_ref().unwrap()["size"], 2);
        assert_eq!(
            artifact.structured_content.as_ref().unwrap()["filename"],
            "fixture.txt"
        );
        let listed = client
            .call_tool(CallToolRequestParams::new("session").with_arguments(args(
                json!({"action":"list_targets","session_id":session_id}),
            )))
            .await
            .unwrap();
        assert!(!listed.is_error.unwrap_or(false), "{listed:?}");
        let target_ref =
            listed.structured_content.as_ref().unwrap()["targets"][0]["target_ref"].clone();
        let artifact_handle = artifact.structured_content.as_ref().unwrap()["handle"].clone();
        let before_missing_marker = client
            .call_tool(
                CallToolRequestParams::new("file_select").with_arguments(args(json!({
                    "session_id":session_id,
                    "target_ref":target_ref,
                    "locator":{"selector":"input[type=file]"},
                    "artifact_handle":artifact_handle
                }))),
            )
            .await;
        assert!(
            before_missing_marker.is_err()
                || before_missing_marker.unwrap().is_error.unwrap_or(false)
        );
        let mismatched_marker = client
            .call_tool(
                CallToolRequestParams::new("file_select").with_arguments(args(json!({
                    "session_id":session_id,
                    "target_ref":target_ref,
                    "locator":{"selector":"input[type=file]"},
                    "artifact_handle":artifact_handle,
                    "account_marker":["#account","missing-marker"]
                }))),
            )
            .await;
        assert!(mismatched_marker.is_err() || mismatched_marker.unwrap().is_error.unwrap_or(false));
        assert_eq!(
            file_select_calls.load(Ordering::SeqCst),
            0,
            "a mismatched marker must withhold DOM.setFileInputFiles"
        );
        let observed = client.call_tool(CallToolRequestParams::new("observe").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"spec":{"selector":"p","fields":{"text":"p"},"max_items":10,"max_text_chars":100,"max_bytes":4096,"cursor":null}})))).await.unwrap();
        assert!(!observed.is_error.unwrap_or(false), "{observed:?}");
        let ax = client.call_tool(CallToolRequestParams::new("accessibility").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"selector":"#fixture","max_bytes":8192})))).await.unwrap();
        assert!(!ax.is_error.unwrap_or(false), "{ax:?}");
        assert_eq!(
            ax.structured_content.as_ref().unwrap()["nodes"][0]["nodeId"],
            "1"
        );
        assert_eq!(
            full_ax_calls.load(Ordering::SeqCst),
            0,
            "AX request must use a per-node command"
        );
        let crop = client.call_tool(CallToolRequestParams::new("screenshot_crop").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"x":0.0,"y":0.0,"width":8.0,"height":8.0,"scale":1.0,"max_bytes":8192})))).await.unwrap();
        assert!(!crop.is_error.unwrap_or(false), "{crop:?}");
        assert_eq!(
            crop.structured_content.as_ref().unwrap()["data_base64"],
            "aGVsbG8="
        );
        let before_oversized_crop = screenshot_calls.load(Ordering::SeqCst);
        let oversized_crop = client.call_tool(CallToolRequestParams::new("screenshot_crop").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"x":0.0,"y":0.0,"width":32.0,"height":32.0,"scale":1.0,"max_bytes":4096})))).await;
        assert!(
            oversized_crop.is_err()
                || oversized_crop
                    .as_ref()
                    .is_ok_and(|r| r.is_error.unwrap_or(false)),
            "oversized crop was accepted: {oversized_crop:?}"
        );
        assert_eq!(
            screenshot_calls.load(Ordering::SeqCst),
            before_oversized_crop,
            "oversized crop reached CDP"
        );
        let extracted = client.call_tool(CallToolRequestParams::new("extract").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"sections":[{"section_id":"primary","spec":{"container":"main","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":10,"max_text_chars":100,"max_bytes":8192,"expected_count":0,"account_marker":["#account","me"],"terminal_selector":".end"}},{"section_id":"secondary","spec":{"container":"aside","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":10,"max_text_chars":100,"max_bytes":8192,"expected_count":0,"account_marker":["#account","me"],"terminal_selector":".end"}}],"max_records":20,"max_bytes":32768,"timeout_ms":5000})))).await.unwrap();
        assert!(!extracted.is_error.unwrap_or(false), "{extracted:?}");
        assert_eq!(
            extracted.structured_content.as_ref().unwrap()["sections"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            extracted.structured_content.as_ref().unwrap()["completeness"],
            "complete"
        );
        let overlapping = client.call_tool(CallToolRequestParams::new("extract").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"sections":[{"section_id":"left","spec":{"container":"#bucket-left","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":4,"max_text_chars":100,"max_bytes":8192,"expected_count":1,"account_marker":["#account","me"],"terminal_selector":".end"}},{"section_id":"right","spec":{"container":"#bucket-right","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":4,"max_text_chars":100,"max_bytes":8192,"expected_count":1,"account_marker":["#account","me"],"terminal_selector":".end"}}],"max_records":8,"max_bytes":32768,"timeout_ms":5000})))).await.unwrap();
        let overlapping = overlapping.structured_content.as_ref().unwrap();
        assert_eq!(
            overlapping["unique_count"], 2,
            "same IDs in different sections must remain distinct"
        );
        let overlapping_rows = overlapping["records"].as_array().unwrap();
        assert_eq!(overlapping_rows.len(), 2);
        assert!(
            overlapping_rows
                .iter()
                .any(|row| row["section_id"] == "left")
        );
        assert!(
            overlapping_rows
                .iter()
                .any(|row| row["section_id"] == "right")
        );

        let later_args = |later_cursor: Option<String>| json!({"session_id":session_id,"target_ref":target_ref,"sections":[{"section_id":"earlier","spec":{"container":"#earlier","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":1,"max_records":20,"max_text_chars":100,"max_bytes":8192,"expected_count":1,"account_marker":["#account","me"],"terminal_selector":".end"}},{"section_id":"later","spec":{"container":"#later","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":1,"max_records":20,"max_text_chars":100,"max_bytes":8192,"expected_count":2,"account_marker":["#account","me"],"terminal_selector":".end","cursor":later_cursor}}],"max_records":20,"max_bytes":32768,"timeout_ms":5000});
        let first_sections = client
            .call_tool(CallToolRequestParams::new("extract").with_arguments(args(later_args(None))))
            .await
            .unwrap();
        let first_later =
            first_sections.structured_content.as_ref().unwrap()["sections"][1]["cursor"]
                .as_str()
                .unwrap()
                .to_owned();
        assert!(
            first_sections.structured_content.as_ref().unwrap()["sections"][1]["cursor_is_resumable"]
                == true
        );
        let resumed_sections = client
            .call_tool(
                CallToolRequestParams::new("extract")
                    .with_arguments(args(later_args(Some(first_later)))),
            )
            .await
            .unwrap();
        let resumed_later = &resumed_sections.structured_content.as_ref().unwrap()["sections"][1];
        assert_ne!(
            resumed_later["completeness"], "unknown",
            "later section cursor must survive an earlier section returning a different record count: {resumed_later}"
        );
        let too_small = client.call_tool(CallToolRequestParams::new("extract").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"sections":[{"section_id":"primary","spec":{"container":"main","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":10,"max_text_chars":100,"max_bytes":8192,"expected_count":0,"account_marker":["#account","me"],"terminal_selector":".end"}},{"section_id":"secondary","spec":{"container":"aside","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":10,"max_text_chars":100,"max_bytes":8192,"expected_count":0,"account_marker":["#account","me"],"terminal_selector":".end"}}],"max_records":20,"max_bytes":4096,"timeout_ms":5000})))).await.unwrap();
        let partial = too_small.structured_content.as_ref().unwrap();
        assert_eq!(partial["completeness"], "unknown");
        assert!(
            partial["sections"]
                .as_array()
                .unwrap()
                .iter()
                .all(|section| section["missing"].is_array())
        );
        let timed_out = client.call_tool(CallToolRequestParams::new("extract").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"sections":[{"section_id":"primary","spec":{"container":"main","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":10,"max_text_chars":100,"max_bytes":8192,"expected_count":0,"account_marker":["#account","me"],"terminal_selector":".end"}},{"section_id":"secondary","spec":{"container":"aside","record":"article","fields":{"id":".id"},"id_field":"id","max_steps":3,"max_records":10,"max_text_chars":100,"max_bytes":8192,"expected_count":0,"account_marker":["#account","me"],"terminal_selector":".end"}}],"max_records":20,"max_bytes":32768,"timeout_ms":50})))).await.unwrap();
        let timed_out = timed_out.structured_content.as_ref().unwrap();
        assert_eq!(timed_out["sections"].as_array().unwrap().len(), 2);
        assert_eq!(timed_out["sections"][1]["completeness"], "unknown");
        assert!(
            timed_out["sections"][1]["missing"][0]
                .as_str()
                .unwrap()
                .contains("not visited")
        );
        client.cancel().await.unwrap();
        let _ = server_task.await.unwrap();
        cdp.abort();
        unsafe {
            std::env::remove_var("COMPTROL_ALLOW_DIRECT_CDP");
            std::env::remove_var("COMPTROL_CDP_ENDPOINT");
        }
    }

    #[tokio::test]
    async fn mcp_shared_observe_is_bounded_and_discards_navigation_races() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let (server_io, client_io) = tokio::io::duplex(16_384);
        let server = serve_directly::<RoleServer, _, _, _, _>(App::default(), server_io, None);
        let server_task = tokio::spawn(async move { server.waiting().await });
        let client = ().serve(client_io).await.unwrap();
        let args = |v: Value| v.as_object().unwrap().clone();
        let invalid_pair = client
            .call_tool(CallToolRequestParams::new("session").with_arguments(args(
                json!({"action":"pair_shared","target_ids":["../remote"]}),
            )))
            .await;
        assert!(invalid_pair.is_err() || invalid_pair.unwrap().is_error.unwrap_or(false));
        let noncanonical = client
            .call_tool(
                CallToolRequestParams::new("session")
                    .with_arguments(args(json!({"action":"pair_shared","target_ids":["0123"]}))),
            )
            .await;
        assert!(noncanonical.is_err() || noncanonical.unwrap().is_error.unwrap_or(false));

        let paired = client
            .call_tool(
                CallToolRequestParams::new("session")
                    .with_arguments(args(json!({"action":"pair_shared","target_ids":["123"]}))),
            )
            .await
            .unwrap();
        let paired = paired.structured_content.unwrap();
        let endpoint = paired["endpoint"].as_str().unwrap().to_owned();
        assert!(endpoint.starts_with("ws://127.0.0.1:"));
        assert_eq!(paired["target_ids"], json!(["123"]));
        assert_eq!(
            paired["identity"],
            "Chrome tab IDs are not Direct CDP target references"
        );
        let session_id = paired["session_id"].as_str().unwrap().to_owned();
        let token = paired["one_session_token"].as_str().unwrap().to_owned();
        assert_eq!(token.len(), 64);
        let accept = client.call_tool(CallToolRequestParams::new("session").with_arguments(args(
            json!({"action":"accept_shared","session_id":session_id}),
        )));
        let extension = async move {
            let (mut peer, _) = connect_async(endpoint).await.unwrap();
            peer.send(Message::Text(
                json!({
                    "type":"hello","token":token,"targets":["123"]
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
            assert_eq!(
                peer.next().await.unwrap().unwrap().to_text().unwrap(),
                "{\"type\":\"ready\"}"
            );
            peer
        };
        let (accepted, mut extension) = tokio::join!(accept, extension);
        assert_eq!(
            accepted.unwrap().structured_content.unwrap()["accepted"],
            true
        );

        let observe_args = || {
            args(json!({
                "session_id":session_id,"chrome_tab_id":"123",
                "spec":{"selector":"p","fields":{"text":"p"},"max_items":10,"max_text_chars":100,"max_bytes":4096,"cursor":null}
            }))
        };
        let observe = client
            .call_tool(CallToolRequestParams::new("shared_observe").with_arguments(observe_args()));
        let replies = async {
            for (method, loader) in [
                ("Page.getFrameTree", "doc-1"),
                ("Runtime.evaluate", ""),
                ("Page.getFrameTree", "doc-1"),
            ] {
                let request: Value = serde_json::from_str(
                    extension.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                assert_eq!(request["method"], method);
                let result = if method == "Page.getFrameTree" {
                    json!({"frameTree":{"frame":{"id":"root","loaderId":loader,"url":"https://fixture.test/"}}})
                } else {
                    json!({"result":{"type":"object","value":{"items":[{"text":"ok"}],"count":1,"missing":[],"clipped":0,"limited":false}}})
                };
                extension
                    .send(Message::Text(
                        json!({"type":"result","id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
        };
        let (observed, ()) = tokio::join!(observe, replies);
        let observed = observed.unwrap().structured_content.unwrap();
        assert_eq!(observed["items"][0]["text"], "ok");
        assert_eq!(observed["navigation_epoch"], Value::Null);
        assert_eq!(observed["shared_frame_identity"]["loader_id"], "doc-1");
        assert!(observed.get("target_ref").is_none());
        assert!(serde_json::to_vec(&observed).unwrap().len() <= 4096);

        let stale_observe = client
            .call_tool(CallToolRequestParams::new("shared_observe").with_arguments(observe_args()));
        let navigation_replies = async {
            for (method, loader) in [
                ("Page.getFrameTree", "doc-1"),
                ("Runtime.evaluate", ""),
                ("Page.getFrameTree", "doc-2"),
            ] {
                let request: Value = serde_json::from_str(
                    extension.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                assert_eq!(request["method"], method);
                let result = if method == "Page.getFrameTree" {
                    json!({"frameTree":{"frame":{"id":"root","loaderId":loader,"url":"https://fixture.test/"}}})
                } else {
                    json!({"result":{"type":"object","value":{"items":[],"count":0,"missing":[],"clipped":0,"limited":false}}})
                };
                extension
                    .send(Message::Text(
                        json!({"type":"result","id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
        };
        let (stale, ()) = tokio::join!(stale_observe, navigation_replies);
        assert!(stale.is_err() || stale.unwrap().is_error.unwrap_or(false));
        let released = client
            .call_tool(CallToolRequestParams::new("session").with_arguments(args(
                json!({"action":"release_shared","session_id":session_id}),
            )))
            .await
            .unwrap();
        assert_eq!(released.structured_content.unwrap()["released"], true);
        client.cancel().await.unwrap();
        let _ = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn separate_shared_sessions_keep_distinct_registry_ids_and_can_release_independently() {
        let (server_io, client_io) = tokio::io::duplex(16_384);
        let server = serve_directly::<RoleServer, _, _, _, _>(App::default(), server_io, None);
        let server_task = tokio::spawn(async move { server.waiting().await });
        let client = ().serve(client_io).await.unwrap();
        let args = |value: Value| value.as_object().unwrap().clone();
        let pair = |tab_id: &str| {
            CallToolRequestParams::new("session").with_arguments(args(json!({
                "action":"pair_shared","target_ids":[tab_id]
            })))
        };
        let first = client.call_tool(pair("123")).await.unwrap();
        let second = client.call_tool(pair("456")).await.unwrap();
        let first_id = first.structured_content.unwrap()["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second_id = second.structured_content.unwrap()["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(
            first_id, second_id,
            "independent registries must not collide"
        );
        for session_id in [&first_id, &second_id] {
            let released = client
                .call_tool(CallToolRequestParams::new("session").with_arguments(args(
                    json!({"action":"release_shared","session_id":session_id}),
                )))
                .await
                .unwrap();
            assert_eq!(released.structured_content.unwrap()["released"], true);
        }
        client.cancel().await.unwrap();
        let _ = server_task.await.unwrap();
    }
}
