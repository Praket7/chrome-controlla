use controlla_browser::{
    BrowserManager, ExpansionControl, ExtractionSpec, GuardSnapshot, GuardedFileSelection,
    ObserveSpec, ScreenshotCrop, SessionProvider, connect_permissioned_auto_connect,
    identity_marker_command, list_sessions, observation_command, parse_observation,
    sessions::{
        IdentityRevisions, Ownership, ProviderGrants, SessionMode, SessionRegistry, SessionSpec,
        TargetRef,
    },
};
use rmcp::{
    ServerHandler, ServiceExt, handler::server::wrapper::Parameters, tool, tool_handler,
    tool_router,
};
use serde_json::{Value, json};
use std::time::Duration;
use std::{collections::BTreeMap, net::IpAddr, sync::Arc};
use tokio::sync::Mutex;

const LOCAL_STDIO_PRINCIPAL: &str = "local-stdio";
pub const SHARED_CLICK_UNSAFE_PREDICATE: &str = "['password','hidden','file','image'].includes(t)||(e instanceof HTMLInputElement&&!['text','search','email','url','tel','submit','reset','button'].includes(e.type))||(e.hasAttribute('data-requires-trusted')&&!(e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(t)))||((e instanceof HTMLButtonElement||e instanceof HTMLInputElement)&&!!e.form&&['submit','reset'].includes(t))";
const SHARED_CLICK_VERIFY_INITIAL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const SHARED_CLICK_VERIFY_MAX_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Why a click was refused before any input was dispatched.
enum ClickRefusal {
    /// Typed guard failure; already shaped as the compact `not_dispatched` result.
    Guard(Value),
    Msg(String),
}
impl From<String> for ClickRefusal {
    fn from(message: String) -> Self {
        ClickRefusal::Msg(message)
    }
}
impl From<&str> for ClickRefusal {
    fn from(message: &str) -> Self {
        ClickRefusal::Msg(message.to_owned())
    }
}
const SHARED_TYPE_GUARD_FUNCTION: &str = r#"function(selector,expected,finalCheck){const e=this.node;let es;try{es=[...document.querySelectorAll(selector)]}catch(_){return {ok:false,reason:'invalid_selector'};}if(!e||!e.isConnected||es.length!==1||es[0]!==e)return {ok:false,reason:'target_replaced'};e.scrollIntoView({block:'nearest'});const s=getComputedStyle(e),b=e.getBoundingClientRect(),x=b.left+b.width/2,y=b.top+b.height/2,h=document.elementFromPoint(x,y);if(!(e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type))||e.matches(':disabled')||e.readOnly||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted')||b.width<=0||b.height<=0||b.left<0||b.top<0||b.right>innerWidth||b.bottom>innerHeight||s.visibility==='hidden'||s.display==='none'||s.pointerEvents==='none'||h!==e)return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'stale_value'};if(document.activeElement!==e||e.selectionStart!==e.selectionEnd||e.selectionEnd!==e.value.length)return {ok:false,reason:'typing_requires_focused_end_caret'};return finalCheck?{ok:e.value===expected,value:e.value}:{ok:true};}"#;

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct SessionArgs {
    action: String,
    provider: Option<String>,
    session_id: Option<String>,
    target_ids: Option<Vec<String>>,
    host_id: Option<String>,
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
    #[schemars(range(min = 1, max = 500))]
    max_items: usize,
    #[schemars(range(min = 1, max = 10_000))]
    max_text_chars: usize,
    #[schemars(range(min = 4096, max = 1_000_000))]
    max_bytes: usize,
    #[schemars(length(max = 256))]
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
struct SharedAccessibilityArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: Option<String>,
    frame_id: Option<String>,
    max_bytes: usize,
    timeout_ms: Option<u64>,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct SharedInputArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: String,
    action: String,
    expected_value: String,
    value: String,
    postcondition_selector: Option<String>,
    postcondition: Option<String>,
    timeout_ms: Option<u64>,
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("allOf" = [{"if":{"properties":{"action":{"const":"open"}},"required":["action"]},"then":{"required":["idempotency_key"]}}]))]
struct SharedTabArgs {
    session_id: String,
    action: String,
    chrome_tab_id: Option<String>,
    url: String,
    #[schemars(
        description = "Required only for action=open. Reuse the same key only when retrying the same logical open; choose a new key for a distinct tab."
    )]
    idempotency_key: Option<String>,
    #[schemars(
        description = "Whether a new tab becomes active. Defaults to true; set false to keep it in the background."
    )]
    active: Option<bool>,
    #[schemars(
        description = "Optional readiness gate for action=open: document_committed attaches at the navigation commit; dom_content_loaded waits for the root document readiness event. Defaults to dom_content_loaded."
    )]
    wait_until: Option<String>,
    create_background_window: Option<bool>,
    target_window_id: Option<u64>,
    #[schemars(
        description = "For action=adopt_popup, the exact tab ID returned by a verified new-tab outcome."
    )]
    candidate_tab_id: Option<String>,
    #[schemars(
        description = "For action=adopt_popup, the exact Chrome creation timestamp returned by the opener-bound candidate inventory."
    )]
    created_at_ms: Option<u64>,
    #[schemars(
        description = "For action=adopt_popup, the click-start timestamp used to bound the candidate inventory."
    )]
    since_ms: Option<u64>,
    #[schemars(
        description = "For action=adopt_popup, a CSS selector that scopes a fresh semantic snapshot to the visible account marker."
    )]
    account_marker_selector: Option<String>,
    #[schemars(
        description = "Exact caller-observed visible account marker text. This is UI evidence and does not prove authentication."
    )]
    account_marker_text: Option<String>,
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
struct WorkflowArgs {
    session_id: String,
    idempotency_key: String,
    target_ref: TargetRefInput,
    steps: Vec<crate::workflow::WorkflowStep>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct WorkflowStatusArgs {
    session_id: String,
    operation_id: String,
    #[serde(default)]
    cancel: bool,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ArtifactRegisterArgs {
    session_id: String,
    filename: String,
    bytes: Vec<u8>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct ArtifactVerifyArgs {
    session_id: String,
    artifact_handle: String,
}

#[derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(untagged)]
enum FileSelectLocator {
    RoleName(RoleNameLocator),
    Label(LabelLocator),
    Placeholder(PlaceholderLocator),
    Text(TextLocator),
    TestId(TestIdLocator),
    AltText(AltTextLocator),
    HrefContains(HrefContainsLocator),
    Selector(SelectorLocator),
    BackendNodeId(BackendNodeIdLocator),
}

macro_rules! locator_string_input {
    ($name:ident, $field:ident) => {
        #[derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
        #[serde(deny_unknown_fields)]
        struct $name {
            $field: String,
        }
    };
}

locator_string_input!(LabelLocator, label);
locator_string_input!(PlaceholderLocator, placeholder);
locator_string_input!(TextLocator, text);
locator_string_input!(TestIdLocator, test_id);
locator_string_input!(AltTextLocator, alt_text);
locator_string_input!(HrefContainsLocator, href_contains);
locator_string_input!(SelectorLocator, selector);

#[derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RoleNameLocator {
    role: String,
    name: String,
}

#[derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BackendNodeIdLocator {
    backend_node_id: i64,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct FileSelectArgs {
    session_id: String,
    target_ref: TargetRefInput,
    locator: FileSelectLocator,
    artifact_handle: String,
    account_marker: (String, String),
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct GuideArgs {
    topic: String,
    server_version: String,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct AppCapabilitiesArgs {
    app: String,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SlidesDeckPlanArgs {
    account_id: String,
    presentation_id: String,
    required_revision_id: String,
    asserted_existing_slide_ids: Vec<String>,
    asserted_existing_object_ids: Vec<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CanvaDesignPlanArgs {
    binding: CanvaPlanBindingArgs,
    session: CanvaSessionArgs,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CanvaIdentityArgs {
    account_id: String,
    workspace_id: String,
    design_id: String,
}

impl CanvaIdentityArgs {
    fn into_identity(self, principal: &str) -> crate::canva::CanvaIdentity {
        crate::canva::CanvaIdentity {
            principal: principal.into(),
            account_id: self.account_id,
            workspace_id: self.workspace_id,
            design_id: self.design_id,
        }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CanvaPlanBindingArgs {
    identity: CanvaIdentityArgs,
    session_id: String,
    expected_version: String,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CanvaSessionArgs {
    identity: CanvaIdentityArgs,
    session_id: String,
    current_version: String,
    opened_at_ms: u64,
    expires_at_ms: u64,
    page_type: crate::apps::CanvaPageType,
    locked: bool,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct CapCutRecipePlanArgs {
    recipe: crate::capcut_recipe::CapCutRecipe,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SlidesPlanArgs {
    account_id: String,
    presentation_id: String,
    required_revision_id: String,
    operation: String,
    expected_text: Option<String>,
    new_text: Option<String>,
    start_index: Option<usize>,
    end_index: Option<usize>,
    object_id: Option<String>,
    text: Option<String>,
    insertion_index: Option<usize>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CanvaPlanArgs {
    account_id: String,
    design_id: String,
    page_id: String,
    revision: String,
    page_type: String,
    locked: bool,
    opened_at_ms: u64,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
struct CapCutPlanArgs {
    operation: String,
}

struct LiveSession {
    connection: Arc<controlla_browser::BrowserConnection>,
    registry: Mutex<SessionRegistry>,
    handle: controlla_browser::sessions::SessionHandle,
    artifact_expectations: Mutex<BTreeMap<String, crate::artifacts::ArtifactExpectation>>,
}

struct SharedLiveSession {
    snapshots: BTreeMap<String, SharedPageSnapshot>,
    pairing: Option<
        tokio::task::JoinHandle<
            Result<
                (
                    SessionRegistry,
                    controlla_browser::providers::SharedExtensionSession,
                ),
                controlla_browser::providers::ProviderError,
            >,
        >,
    >,
    connection: Option<PageSessionTransport>,
    registry: Option<SessionRegistry>,
    handle: controlla_browser::sessions::SessionHandle,
    native_pairing: Option<(String, String)>,
}

tokio::task_local! {
    static INTERNAL_WORKFLOW_STATE: std::cell::RefCell<std::collections::HashSet<String>>;
}

#[allow(dead_code)] // Used by V3 workflow in the CLI binary.
pub(crate) async fn with_internal_workflow_state<T>(
    future: impl std::future::Future<Output = T>,
) -> T {
    INTERNAL_WORKFLOW_STATE
        .scope(
            std::cell::RefCell::new(std::collections::HashSet::new()),
            future,
        )
        .await
}

#[allow(dead_code)] // Used by V3 retained-mutation checks in the CLI binary.
pub(crate) fn internal_workflow_snapshot_allowed(token: &str) -> bool {
    INTERNAL_WORKFLOW_STATE
        .try_with(|tokens| tokens.borrow().contains(token))
        .unwrap_or(false)
}

#[allow(dead_code)] // Used by V2/V3 workflow success paths in the CLI binary.
pub(crate) fn authorize_internal_workflow_snapshot(token: &str) {
    let _ = INTERNAL_WORKFLOW_STATE.try_with(|tokens| {
        tokens.borrow_mut().insert(token.to_owned());
    });
}

/// The V2/V3 semantic layer uses the same guarded page protocol over either
/// the explicitly paired extension or a provider-owned dedicated Chrome.
#[allow(dead_code)] // V2/V3 transports are compiled into the CLI binary, not the library API.
enum PageSessionTransport {
    Shared(controlla_browser::providers::SharedExtensionSession),
    Dedicated(Option<controlla_browser::providers::DedicatedBrowserSession>),
}

fn dedicated_batch_guard_ok(response: &Value) -> bool {
    response.pointer("/result/value/ok") == Some(&Value::Bool(true))
}

fn private_headless_cleanup_allowed(handle: &controlla_browser::sessions::SessionHandle) -> bool {
    handle.mode == controlla_browser::sessions::SessionMode::Headless
        && handle.provider == controlla_browser::sessions::ProviderKind::DedicatedHeadless
}

impl PageSessionTransport {
    fn selected_targets(&self) -> Vec<String> {
        match self {
            Self::Shared(connection) => connection.selected_targets().iter().cloned().collect(),
            Self::Dedicated(Some(browser)) => vec![browser.target_id().to_owned()],
            Self::Dedicated(None) => Vec::new(),
        }
    }

    fn add_selected_target(&mut self, target_id: String) -> Result<(), String> {
        match self {
            Self::Shared(connection) => connection
                .add_selected_target(target_id)
                .map_err(|e| e.to_string()),
            Self::Dedicated(_) => Err("dedicated session target set is fixed at launch".into()),
        }
    }

    fn remove_selected_target(&mut self, target_id: &str) -> bool {
        match self {
            Self::Shared(connection) => connection.remove_selected_target(target_id),
            Self::Dedicated(_) => false,
        }
    }

    async fn command(
        &self,
        registry: &SessionRegistry,
        handle: &controlla_browser::sessions::SessionHandle,
        target_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        match self {
            Self::Shared(connection) => connection
                .command(registry, handle, target_id, method, params)
                .await
                .map_err(|error| error.to_string()),
            Self::Dedicated(Some(browser)) => {
                controlla_browser::scheduler::telemetry::admit_browser_commands(1)
                    .await
                    .map_err(str::to_owned)?;
                registry
                    .authorize_direct_cdp(&handle.id)
                    .map_err(|error| format!("session authorization failed: {error:?}"))?;
                if !registry.contains_target(handle, target_id) {
                    return Err("target is not owned by the selected dedicated session".into());
                }
                let (_, targets) = browser.connection().target_snapshot().await;
                let target = targets
                    .iter()
                    .find(|target| {
                        target.id == target_id && target.target_type == "page" && target.attached
                    })
                    .ok_or_else(|| {
                        "selected dedicated target is detached or no longer a page".to_owned()
                    })?;
                let request_bytes = method.len().saturating_add(params.to_string().len()) as u64;
                let started = std::time::Instant::now();
                let result = browser
                    .connection()
                    .target_command(
                        target_id,
                        target.generation,
                        &target.revision,
                        method,
                        params,
                    )
                    .await;
                let response_bytes =
                    result.as_ref().map_or(0, |value| value.to_string().len()) as u64;
                controlla_browser::scheduler::telemetry::record_browser_command(
                    1,
                    1,
                    request_bytes.saturating_add(response_bytes),
                    0,
                    started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                )
                .await;
                if result.is_err() {
                    controlla_browser::scheduler::telemetry::record_unknown_delivery().await;
                }
                result.map_err(|error| error.to_string())
            }
            Self::Dedicated(None) => Err("dedicated browser session is shutting down".into()),
        }
    }

    #[allow(dead_code)] // Used by V3 FastKeys in the CLI binary.
    async fn command_batch(
        &self,
        registry: &SessionRegistry,
        handle: &controlla_browser::sessions::SessionHandle,
        target_id: &str,
        actions: Vec<controlla_browser::providers::SharedBatchAction>,
        deadline: Duration,
    ) -> Result<Value, String> {
        if actions.is_empty() || actions.len() > 64 {
            return Err("batch requires 1..64 actions".into());
        }
        match self {
            Self::Shared(connection) => connection
                .command_batch(registry, handle, target_id, actions, deadline)
                .await
                .map_err(|error| error.to_string()),
            Self::Dedicated(_) => {
                let mut receipts = Vec::with_capacity(actions.len());
                let deadline_at = tokio::time::Instant::now() + deadline;
                for (index, action) in actions.iter().enumerate() {
                    let result = tokio::time::timeout_at(
                        deadline_at,
                        self.command(
                            registry,
                            handle,
                            target_id,
                            &action.method,
                            action.params.clone(),
                        ),
                    )
                    .await;
                    let result = match result {
                        Ok(Ok(result)) => result,
                        Ok(Err(error)) => {
                            let may_have_occurred = action.method == "Input.dispatchKeyEvent"
                                || receipts.iter().any(|receipt: &Value| {
                                    receipt["method"] == "Input.dispatchKeyEvent"
                                });
                            return Ok(
                                json!({"receipts":receipts,"completed":index,"failed_at":index,
                                "failed_method":action.method,"dispatch_may_have_occurred":may_have_occurred,
                                "batch_error":error}),
                            );
                        }
                        Err(_) => {
                            let may_have_occurred = action.method == "Input.dispatchKeyEvent"
                                || receipts.iter().any(|receipt: &Value| {
                                    receipt["method"] == "Input.dispatchKeyEvent"
                                });
                            return Ok(
                                json!({"receipts":receipts,"completed":index,"failed_at":index,
                                "failed_method":action.method,"dispatch_may_have_occurred":may_have_occurred,
                                "batch_error":"dedicated key batch deadline exceeded"}),
                            );
                        }
                    };
                    let ok = dedicated_batch_guard_ok(&result);
                    receipts.push(json!({"index":index,"method":action.method,"result":result}));
                    if action.stop_on_not_ok && !ok {
                        return Ok(
                            json!({"receipts":receipts,"completed":index+1,"stopped_before":index+1,"batch_error":"Batch read guard rejected before the next action."}),
                        );
                    }
                }
                Ok(json!({"receipts":receipts,"completed":actions.len()}))
            }
        }
    }

    async fn release(
        &mut self,
        registry: &mut SessionRegistry,
        handle: &controlla_browser::sessions::SessionHandle,
    ) -> Result<(), String> {
        match self {
            Self::Shared(connection) => connection
                .release()
                .await
                .map_err(|error| error.to_string()),
            Self::Dedicated(slot) => {
                if !private_headless_cleanup_allowed(handle) {
                    return Err(
                        "private-profile cleanup is restricted to dedicated headless sessions"
                            .into(),
                    );
                }
                let Some(browser) = slot.take() else {
                    return Ok(());
                };
                let expected_session = browser.session_id().to_owned();
                let expected_target = browser.target_id().to_owned();
                let expected_instance = browser.connection().instance_id();
                let (_, targets) = browser.connection().target_snapshot().await;
                let Some(target) = targets
                    .iter()
                    .find(|target| target.id == expected_target && target.attached)
                else {
                    *slot = Some(browser);
                    return Err(
                        "dedicated target changed before release; session retained for inspection"
                            .into(),
                    );
                };
                // This adapter does not inspect page contents independently. It is authorized
                // only for the provider-owned headless process with a unique private profile;
                // generic/shared/headed targets still require a real independent observer.
                let observer = DedicatedOwnedObserver {
                    expected_session,
                    expected_target,
                    expected_instance,
                    expected_generation: target.generation,
                    expected_revision: target.revision.clone(),
                };
                let outcome = browser.shutdown(registry, Some(&observer)).await;
                if let Some(recovery) = outcome.recovery {
                    *slot = Some(recovery);
                    return Err(outcome
                        .cleanup_error
                        .unwrap_or_else(|| "dedicated browser cleanup remains pending".into()));
                }
                if let Some(error) = outcome.cleanup_error {
                    return Err(error);
                }
                Ok(())
            }
        }
    }
}

struct DedicatedOwnedObserver {
    expected_session: String,
    expected_target: String,
    expected_instance: u128,
    expected_generation: u64,
    expected_revision: String,
}

impl controlla_browser::sessions::IndependentTargetObserver for DedicatedOwnedObserver {
    fn verify_unchanged(
        &self,
        observation: &controlla_browser::sessions::CleanupObservation,
    ) -> Result<(), String> {
        if observation.session_id == self.expected_session
            && observation.target_id == self.expected_target
            && observation.browser_instance_id == self.expected_instance
            && observation.browser_generation == self.expected_generation
            && observation.target_revision == self.expected_revision
        {
            Ok(())
        } else {
            Err("owned headless target identity changed".into())
        }
    }
}

impl SharedLiveSession {
    fn registry(&self) -> Result<&SessionRegistry, rmcp::ErrorData> {
        self.registry
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))
    }
}

impl Drop for SharedLiveSession {
    fn drop(&mut self) {
        if let Some(task) = self.pairing.take() {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct SharedPageSnapshot {
    token: String,
    object_id: String,
    identity: (String, String, String),
    interaction_epoch: Value,
    count: usize,
    consumed: bool,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SharedSnapshotArgs {
    session_id: String,
    chrome_tab_id: String,
    #[serde(default = "snapshot_scope")]
    selector: String,
    #[serde(default = "snapshot_items")]
    max_items: usize,
    #[serde(default = "snapshot_chars")]
    max_text_chars: usize,
    #[serde(default = "snapshot_bytes")]
    max_bytes: usize,
    href_hint: Option<String>,
    goal: Option<SnapshotGoal>,
    max_scan_nodes: Option<usize>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SnapshotGoal {
    query: Option<String>,
    roles: Option<Vec<String>>,
    href_hint: Option<String>,
    #[serde(default)]
    within: Option<SnapshotSemanticAncestor>,
    #[serde(default)]
    #[schemars(
        description = "Search all reachable controls instead of stopping at the first exact match. Results remain bounded by max_scan_nodes and max_items."
    )]
    exhaustive: bool,
    #[serde(default)]
    model_profile: Option<ObservationModelProfile>,
    #[serde(default)]
    needs_layout: bool,
}
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ObservationModelProfile {
    CompactContext,
    RichContext,
}
#[derive(Clone, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SnapshotSemanticAncestor {
    tag: String,
    id: String,
}
fn snapshot_scope() -> String {
    "body".into()
}
fn snapshot_items() -> usize {
    200
}
fn snapshot_chars() -> usize {
    2000
}
fn snapshot_bytes() -> usize {
    100000
}

#[derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SharedClickOutcome {
    Navigation {
        #[schemars(
            description = "Optional exact absolute HTTP(S) destination. Omit to accept any observed document change; use a separate URL prefix verification for canonical redirects."
        )]
        url: Option<String>,
    },
    NewTab {
        #[schemars(
            description = "Exact absolute HTTP(S) URL expected in the new tab. The runtime verifies attribution only when Chrome reports this source tab as opener; a matching URL alone cannot prove causality, so rel=noopener or missing opener identity remains unverified."
        )]
        url: Option<String>,
    },
    Focused,
    Visible {
        selector: String,
    },
    Text {
        selector: String,
        text: String,
    },
    Expanded {
        selector: Option<String>,
    },
    Selected {
        selector: Option<String>,
    },
}

/// Stable result code for an action whose input may already have reached the
/// page. These outcomes are deliberately terminal for mutation replay.
fn click_unknown_reason_code(dispatch_acknowledged: bool) -> &'static str {
    if dispatch_acknowledged {
        "outcome_verification_timeout"
    } else {
        "click_dispatch_uncertain"
    }
}
#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SharedClickArgs {
    session_id: String,
    chrome_tab_id: String,
    reference: String,
    outcome: SharedClickOutcome,
    timeout_ms: Option<u64>,
    #[serde(default)]
    input_mode: Option<String>,
    #[serde(default)]
    click_kind: Option<String>,
    /// Revision returned by the watcher-backed tab inventory. When supplied,
    /// the extension refuses dispatch if topology changed after discovery.
    expected_target_revision: Option<u64>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SharedScrollArgs {
    session_id: String,
    chrome_tab_id: String,
    reference: String,
    delta_x: i32,
    delta_y: i32,
    timeout_ms: Option<u64>,
    expected_activation_epoch: Option<u64>,
    expected_focus_epoch: Option<u64>,
    #[serde(default)]
    scroll_to_target: Option<bool>,
    expected_target_revision: Option<u64>,
}

#[allow(dead_code)] // Used by the V3 CLI binary.
struct SharedSwipeArgs {
    session_id: String,
    chrome_tab_id: String,
    reference: String,
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    timeout_ms: Option<u64>,
    expected_activation_epoch: Option<u64>,
    expected_focus_epoch: Option<u64>,
    expected_target_revision: Option<u64>,
}

#[allow(dead_code)]
struct SharedPinchArgs {
    session_id: String,
    chrome_tab_id: String,
    reference: String,
    center_x: f64,
    center_y: f64,
    start_distance: f64,
    end_distance: f64,
    outcome: SharedClickOutcome,
    timeout_ms: Option<u64>,
    expected_activation_epoch: Option<u64>,
    expected_focus_epoch: Option<u64>,
    expected_target_revision: Option<u64>,
}

fn validate_wheel_delta(delta_x: i32, delta_y: i32) -> Result<(), &'static str> {
    const MAX_DELTA: i32 = 1200;
    if (delta_x == 0) == (delta_y == 0) {
        return Err("exactly one of delta_x or delta_y must be nonzero");
    }
    if delta_x.unsigned_abs() > MAX_DELTA as u32 || delta_y.unsigned_abs() > MAX_DELTA as u32 {
        return Err("wheel delta must be within -1200..=1200");
    }
    Ok(())
}

#[allow(dead_code)] // Used by the V3 CLI binary.
fn validate_swipe_endpoints(
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
) -> Result<(), &'static str> {
    const MAX_COORD: f64 = 10_000.0;
    const MIN_DISTANCE: f64 = 20.0;
    const MAX_DISTANCE: f64 = 1_200.0;
    if [start_x, start_y, end_x, end_y]
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=MAX_COORD).contains(value))
    {
        return Err("swipe coordinates must be finite and within 0..=10000");
    }
    let distance = (end_x - start_x).hypot(end_y - start_y);
    if !(MIN_DISTANCE..=MAX_DISTANCE).contains(&distance) {
        return Err("swipe path length must be within 20..=1200 CSS pixels");
    }
    Ok(())
}

fn validate_pinch(
    center_x: f64,
    center_y: f64,
    start_distance: f64,
    end_distance: f64,
) -> Result<(), &'static str> {
    if [center_x, center_y, start_distance, end_distance]
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=10_000.0).contains(v))
    {
        return Err("pinch values must be finite and within 0..=10000");
    }
    if !(40.0..=600.0).contains(&start_distance)
        || !(20.0..=800.0).contains(&end_distance)
        || !(0.5..=2.0).contains(&(end_distance / start_distance))
    {
        return Err("pinch distances must be bounded and scale ratio must be 0.5..=2.0");
    }
    Ok(())
}

fn shared_pinch_guard_script() -> &'static str {
    r#"function(index,cx,cy,startDistance,endDistance,expectedViewport){
      const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'stale_reference'};
      const b=e.getBoundingClientRect(),vw=innerWidth,vh=innerHeight;
      const viewport=JSON.stringify([vw,vh,devicePixelRatio||1]);
      const points=d=>[{x:cx-d/2,y:cy},{x:cx+d/2,y:cy}];
      const start=points(startDistance),end=points(endDistance),inside=p=>p.x>=b.left&&p.x<=b.right&&p.y>=b.top&&p.y<=b.bottom;
      const valid=p=>p.x>=0&&p.y>=0&&p.x<vw&&p.y<vh;
      const style=getComputedStyle(e);
      if(expectedViewport&&viewport!==expectedViewport)return {ok:false,reason:'viewport_changed'};
      if(!start.every(p=>valid(p)&&inside(p)&&(document.elementFromPoint(p.x,p.y)===e||e.contains(document.elementFromPoint(p.x,p.y))))||!end.every(valid)||b.width<=0||b.height<=0||style.display==='none'||style.visibility==='hidden'||style.pointerEvents==='none')return {ok:false,reason:'pinch_target_or_endpoint_not_interactable'};
      return {ok:true,viewport,rect:{left:b.left,top:b.top,width:b.width,height:b.height}};
    }"#
}

fn shared_scroll_guard_script() -> &'static str {
    r#"function(index,dx,dy,expectedViewport){
      const expectedContainer=arguments[4];
      const e=this.nodes[index];
      if(!e||!e.isConnected)return {ok:false,reason:'stale_reference'};
      const d=e.ownerDocument,w=d.defaultView,s=w.getComputedStyle(e),b=e.getBoundingClientRect(),vw=w.innerWidth,vh=w.innerHeight;
      if(s.display==='none'||s.visibility==='hidden'||Number.parseFloat(s.opacity||'1')<=0||b.width<=0||b.height<=0)return {ok:false,reason:'not_visible'};
      const x=Math.max(0,Math.min(vw-1,b.left+b.width/2)),y=Math.max(0,Math.min(vh-1,b.top+b.height/2)),hit=d.elementFromPoint(x,y);
      if(!hit||!(hit===e||e.contains(hit)))return {ok:false,reason:'obscured'};
      if(hit.getRootNode()!==d||hit instanceof HTMLIFrameElement)return {ok:false,reason:'nested_document_scroll_unsupported'};
      const viewport=`${vw}x${vh}@${w.devicePixelRatio||1}`;
      if(expectedViewport&&viewport!==expectedViewport)return {ok:false,reason:'viewport_changed'};
      const axis=dy?'y':'x';
      const isScrollable=n=>{
        if(!(n instanceof Element))return false;
        const cs=w.getComputedStyle(n),overflow=axis==='y'?cs.overflowY:cs.overflowX;
        const extent=axis==='y'?n.scrollHeight:n.scrollWidth,client=axis==='y'?n.clientHeight:n.clientWidth;
        return ['auto','scroll','overlay'].includes(overflow)&&extent-client>1;
      };
      let container=null;
      for(let n=hit;n&&n instanceof Element;n=n.parentElement){if(isScrollable(n)){if(dx&&w.getComputedStyle(n).direction==='rtl')return {ok:false,reason:'rtl_horizontal_scroll_unsupported'};container=n;break}}
      if(!container){const root=d.scrollingElement;if(root&&isScrollable(root)){if(dx&&w.getComputedStyle(root).direction==='rtl')return {ok:false,reason:'rtl_horizontal_scroll_unsupported'};container=root}}
      if(!container)return {ok:false,reason:'no_scrollable_ancestor'};
      if(expectedContainer&&container!==expectedContainer)return {ok:false,reason:'scroll_container_changed'};
      const left=container.scrollLeft,top=container.scrollTop,maxLeft=Math.max(0,container.scrollWidth-container.clientWidth),maxTop=Math.max(0,container.scrollHeight-container.clientHeight);
      const pos=axis==='y'?top:left,max=axis==='y'?maxTop:maxLeft;
      if(dy<0?pos<=1:dy>0?pos>=max-1:dx<0?pos<=1:pos>=max-1)return {ok:false,reason:'nearest_scroll_container_at_boundary'};
      return {ok:true,x,y,viewport,container,left,top,maxLeft,maxTop};
    }"#
}

fn shared_scroll_to_target_guard_script() -> &'static str {
    r#"function(index,scrollRequest){
      const before=(reason)=>({ok:false,phase:'before',dispatch_may_have_occurred:false,reason});
      if(scrollRequest!=='scroll-to-target-v1')return before('invalid_scroll_request');
      const e=this.nodes[index],saved=this.items&&this.items[index];
      if(!e||!saved||!e.isConnected)return before('stale_reference');
      if(e.ownerDocument!==document||e.getRootNode()!==document)return before('non_root_document_unsupported');
      const identity=()=>e.isConnected&&this.raw(e)===saved.raw_value&&this.name(e)===saved.name;
      if(!identity())return before('semantic_identity_changed');
      const w=document.defaultView,s=w.getComputedStyle(e),r=e.getBoundingClientRect();
      if(s.display==='none'||s.visibility==='hidden'||s.pointerEvents==='none'||Number.parseFloat(s.opacity||'1')<=0||r.width<=0||r.height<=0)return before('target_not_visible');
      if(e.disabled||e.inert||e.matches('input[type=password],input[type=file]'))return before('target_not_scrollable');
      try{e.scrollIntoView({block:'center',inline:'center',behavior:'instant'});}catch(_){return {ok:false,phase:'after',dispatch_may_have_occurred:true,reason:'scroll_into_view_failed'};}
      if(!identity())return {ok:false,phase:'after',dispatch_may_have_occurred:true,reason:'semantic_identity_changed_after_scroll'};
      const after=e.getBoundingClientRect(),vw=w.innerWidth,vh=w.innerHeight;
      const x=after.left+after.width/2,y=after.top+after.height/2;
      const visible=after.width>0&&after.height>0&&x>=0&&y>=0&&x<vw&&y<vh;
      const hit=visible?document.elementFromPoint(x,y):null;
      const centerHit=!!hit&&(hit===e||e.contains(hit));
      return {ok:visible&&centerHit,phase:'after',dispatch_may_have_occurred:true,target_visible:visible,center_hit:centerHit,viewport:{width:vw,height:vh},center:{x,y},reason:visible&&centerHit?undefined:'target_visibility_not_verified'};
    }"#
}

#[allow(dead_code)] // Used by the V3 CLI binary.
fn shared_swipe_guard_script() -> &'static str {
    r#"function(index,sx,sy,ex,ey,expectedViewport,phase){
      const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'stale_reference'};
      const b=e.getBoundingClientRect(),vw=innerWidth,vh=innerHeight;
      const viewport=JSON.stringify([vw,vh,devicePixelRatio||1]);
      const point=(x,y)=>Number.isFinite(x)&&Number.isFinite(y)&&x>=0&&y>=0&&x<vw&&y<vh;
      const sxInside=sx>=b.left&&sx<=b.right&&sy>=b.top&&sy<=b.bottom;
      const hit=document.elementFromPoint(sx,sy);
      const style=getComputedStyle(e);
      if(expectedViewport&&viewport!==expectedViewport)return {ok:false,reason:'viewport_changed'};
      if(!point(sx,sy)||!point(ex,ey)||!sxInside||!(hit===e||e.contains(hit))||b.width<=0||b.height<=0||style.display==='none'||style.visibility==='hidden'||style.pointerEvents==='none')return {ok:false,reason:'swipe_target_or_endpoint_not_interactable'};
      let scroll=null;
      for(let n=e;n&&n instanceof Element;n=n.parentElement){const s=getComputedStyle(n);if((['auto','scroll','overlay'].includes(s.overflowY)&&n.scrollHeight>n.clientHeight+1)||(['auto','scroll','overlay'].includes(s.overflowX)&&n.scrollWidth>n.clientWidth+1)){scroll={left:n.scrollLeft,top:n.scrollTop};break}}
      return {ok:true,viewport,start:{x:sx,y:sy},end:{x:ex,y:ey},rect:{left:b.left,top:b.top,width:b.width,height:b.height},scroll};
    }"#
}

fn valid_scroll_readback(receipt: &Value, delta_x: i32, delta_y: i32) -> bool {
    if receipt["same_container"] != true || receipt["container_connected"] != true {
        return false;
    }
    let before_x = receipt["before"]["left"].as_f64();
    let after_x = receipt["after"]["left"].as_f64();
    let before_y = receipt["before"]["top"].as_f64();
    let after_y = receipt["after"]["top"].as_f64();
    match (delta_x, delta_y, before_x, after_x, before_y, after_y) {
        (dx, 0, Some(bx), Some(ax), _, _) if dx < 0 => ax < bx,
        (dx, 0, Some(bx), Some(ax), _, _) if dx > 0 => ax > bx,
        (0, dy, _, _, Some(by), Some(ay)) if dy < 0 => ay < by,
        (0, dy, _, _, Some(by), Some(ay)) if dy > 0 => ay > by,
        _ => false,
    }
}

fn shared_value(response: &Value) -> Result<Value, String> {
    if response.get("exceptionDetails").is_some() {
        let details = response
            .pointer("/exceptionDetails/text")
            .and_then(Value::as_str)
            .unwrap_or("page script exception");
        return Err(format!(
            "page evaluation failed: {}",
            details.chars().take(256).collect::<String>()
        ));
    }
    response.pointer("/result/value").cloned().ok_or_else(|| {
        format!(
            "page evaluation omitted value (remote type={}, subtype={}, response keys={})",
            response
                .pointer("/result/type")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            response
                .pointer("/result/subtype")
                .and_then(Value::as_str)
                .unwrap_or("none"),
            response
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_default()
        )
    })
}

fn shared_command_value(response: &Value) -> Value {
    if response["status"].is_string() {
        response.clone()
    } else {
        shared_value(response).unwrap_or(Value::Null)
    }
}

struct SnapshotBatchReads {
    value: Value,
    identity: (String, String, String),
    frame_handles: Vec<controlla_browser::FrameHandle>,
    frame_handles_truncated: bool,
}

fn snapshot_batch_reads(batch: &Value, target_id: &str) -> Result<SnapshotBatchReads, String> {
    if batch["completed"] != 2 || batch.get("batch_error").is_some() {
        return Err(format!(
            "snapshot read batch failed: {}",
            batch["batch_error"]
        ));
    }
    let receipts = batch["receipts"]
        .as_array()
        .filter(|receipts| receipts.len() == 2)
        .ok_or("snapshot read batch omitted receipts")?;
    if receipts[0]["method"] != "Runtime.callFunctionOn"
        || receipts[1]["method"] != "Page.getFrameTree"
        || receipts
            .iter()
            .any(|receipt| receipt.get("error").is_some())
    {
        return Err("snapshot read batch returned unexpected or failed commands".into());
    }
    let frame_tree = &receipts[1]["result"];
    let (frame_handles, frame_handles_truncated) = frame_handles_from_tree(frame_tree, target_id)?;
    Ok(SnapshotBatchReads {
        value: shared_value(&receipts[0]["result"])
            .map_err(|error| format!("snapshot public getter failed: {error}"))?,
        identity: frame_identity_from_tree(frame_tree)?,
        frame_handles,
        frame_handles_truncated,
    })
}

async fn capture_shared_snapshot(
    shared: &mut SharedLiveSession,
    args: &SharedSnapshotArgs,
) -> Result<Value, String> {
    if args.selector.is_empty()
        || args.selector.len() > 512
        || args.selector.chars().any(char::is_control)
        || !(1..=500).contains(&args.max_items)
        || args.max_text_chars > 6000
        || !(4096..=100000).contains(&args.max_bytes)
        || args.max_bytes < args.max_text_chars * 6 + 4096
        || args.href_hint.as_ref().is_some_and(|hint| {
            hint.is_empty() || hint.len() > 512 || hint.chars().any(char::is_control)
        })
    {
        return Err("snapshot limits: 1..500 items, 0..6000 text chars, 4096..100000 bytes; bytes must cover 6*text chars + 4096".into());
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or("shared session is not accepted")?;
    let registry = shared.registry().map_err(|e| e.to_string())?;
    let before =
        shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id).await?;
    if let Some(old) = shared.snapshots.get(&args.chrome_tab_id) {
        let _ = connection
            .command(
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.releaseObject",
                json!({"objectId":old.object_id}),
            )
            .await;
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let token = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if args
        .max_scan_nodes
        .is_some_and(|limit| !(1..=65536).contains(&limit))
        || args.goal.as_ref().is_some_and(|goal| {
            goal.query
                .as_ref()
                .is_some_and(|query| query.trim().is_empty() || query.len() > 256)
                || (goal.query.is_none() && goal.roles.is_none() && goal.href_hint.is_none())
                || goal.roles.as_ref().is_some_and(|roles| {
                    roles.is_empty() || roles.len() > 16 || roles.iter().any(|role| role.len() > 64)
                })
                || goal
                    .href_hint
                    .as_ref()
                    .is_some_and(|hint| hint.is_empty() || hint.len() > 512)
                || goal.within.as_ref().is_some_and(|scope| {
                    scope.tag.is_empty()
                        || scope.tag.len() > 32
                        || !scope.tag.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                        })
                        || scope.id.is_empty()
                        || scope.id.len() > 256
                })
        })
    {
        return Err(
            "targeted snapshot goal is invalid or exceeds its bounded search limits".into(),
        );
    }
    let config = json!({"selector":args.selector,"max_items":args.max_items,"max_text_chars":args.max_text_chars,"max_bytes":args.max_bytes,"token":token,"href_hint":args.href_hint,"goal":args.goal,"max_scan_nodes":args.max_scan_nodes});
    let expression = format!("({})({})", include_str!("shared_page/snapshot.js"), config);
    let evaluation = connection
        .command(
            registry,
            &shared.handle,
            &args.chrome_tab_id,
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":false}),
        )
        .await
        .map_err(|e| e.to_string())?;
    let object_id = evaluation
        .pointer("/result/objectId")
        .and_then(Value::as_str)
        .ok_or("snapshot failed to retain target identities")?
        .to_owned();
    let read = async {
        if matches!(connection, PageSessionTransport::Shared(shared) if shared.supports_batch_execution()) {
            let batch = connection.command_batch(
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                vec![
                    controlla_browser::providers::SharedBatchAction {
                        method: "Runtime.callFunctionOn".into(),
                        params: json!({"objectId":object_id,"functionDeclaration":"function(){return this.public}","returnByValue":true}),
                        stop_on_not_ok: false,
                    },
                    controlla_browser::providers::SharedBatchAction {
                        method: "Page.getFrameTree".into(),
                        params: Value::Null,
                        stop_on_not_ok: false,
                    },
                ],
                Duration::from_secs(10),
            ).await?;
            snapshot_batch_reads(&batch, &args.chrome_tab_id)
        } else {
            let response = connection.command(
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({"objectId":object_id,"functionDeclaration":"function(){return this.public}","returnByValue":true}),
            ).await?;
            let value = shared_value(&response)
                .map_err(|error| format!("snapshot public getter failed: {error}"))?;
            let tree = connection.command(registry, &shared.handle, &args.chrome_tab_id, "Page.getFrameTree", Value::Null).await?;
            let (frame_handles, frame_handles_truncated) = frame_handles_from_tree(&tree, &args.chrome_tab_id)?;
            Ok(SnapshotBatchReads {
                value,
                identity: frame_identity_from_tree(&tree)?,
                frame_handles,
                frame_handles_truncated,
            })
        }
    }.await;
    let result = async {
        let SnapshotBatchReads {
            mut value,
            identity: after,
            frame_handles,
            frame_handles_truncated,
        } = read?;
        if before != after {
            return Err("document changed during snapshot; discard and read once again".into());
        }
        let interaction_epoch = value
            .get("interaction_epoch")
            .cloned()
            .ok_or_else(|| "snapshot omitted interaction epoch".to_owned())?;
        value
            .as_object_mut()
            .ok_or_else(|| "snapshot result is not an object".to_owned())?
            .remove("interaction_epoch");
        value["snapshot_id"] = json!(token);
        value["chrome_tab_id"] = json!(args.chrome_tab_id);
        value["document"] = json!({"frame_id":before.0,"loader_id":before.1,"url":before.2});
        value["frame_handles"] = public_frame_handles(&frame_handles)?;
        value["frame_handles_truncated"] = json!(frame_handles_truncated);
        if frame_handles_truncated {
            value["truncated"] = json!(true);
            value["complete"] = json!(false);
            value["global_complete"] = json!(false);
            value["scope_complete"] = json!(false);
            value["completeness"]["frames"] = json!("partial");
            if !value["incomplete_reasons"].is_array() {
                value["incomplete_reasons"] = json!([]);
            }
            let reasons = value["incomplete_reasons"]
                .as_array_mut()
                .expect("array set above");
            if !reasons
                .iter()
                .any(|reason| reason.as_str() == Some("frame_handles_truncated"))
            {
                reasons.push(json!("frame_handles_truncated"));
            }
        }
        ensure_snapshot_byte_budget(&value, args.max_bytes)?;
        Ok((value, interaction_epoch))
    }
    .await;
    match result {
        Ok((value, interaction_epoch)) => {
            let count = value["items"]
                .as_array()
                .ok_or("snapshot omitted items")?
                .len();
            shared.snapshots.insert(
                args.chrome_tab_id.clone(),
                SharedPageSnapshot {
                    token,
                    object_id,
                    identity: before,
                    interaction_epoch,
                    count,
                    consumed: false,
                },
            );
            Ok(value)
        }
        Err(error) => {
            let _ = connection
                .command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Runtime.releaseObject",
                    json!({"objectId":object_id}),
                )
                .await;
            Err(error)
        }
    }
}

async fn evaluate_shared_outcome(
    shared: &SharedLiveSession,
    tab: &str,
    snapshot: &SharedPageSnapshot,
    index: usize,
    outcome: &SharedClickOutcome,
    identity: &(String, String, String),
    since_ms: u64,
) -> Result<bool, String> {
    if matches!(outcome, SharedClickOutcome::Navigation { .. }) {
        return shared_navigation_matches(outcome, &snapshot.identity, identity);
    }
    let connection = shared.connection.as_ref().ok_or("session not accepted")?;
    let registry = shared.registry().map_err(|e| e.to_string())?;
    if let SharedClickOutcome::NewTab { url } = outcome {
        let response = connection
            .command(
                registry,
                &shared.handle,
                tab,
                "Controlla.newTabs",
                json!({"since_ms":since_ms,"url":url}),
            )
            .await
            .map_err(|e| e.to_string())?;
        return Ok(shared_new_tab_matches(&response, url.as_deref()));
    }
    let function = include_str!("shared_page/outcome.js");
    let response=if identity==&snapshot.identity {
        connection.command(registry,&shared.handle,tab,"Runtime.callFunctionOn",json!({"objectId":snapshot.object_id,"functionDeclaration":function,"arguments":[{"value":outcome},{"value":index}],"returnByValue":true})).await
    } else {
        let expression=format!("({function})({},0)",serde_json::to_string(outcome).map_err(|e|e.to_string())?);
        connection.command(registry,&shared.handle,tab,"Runtime.evaluate",json!({"expression":expression,"returnByValue":true})).await
    }.map_err(|e|e.to_string())?;
    Ok(shared_value(&response)? == true)
}

fn shared_outcome_identity_allows_verification(
    outcome: &SharedClickOutcome,
    snapshot: &(String, String, String),
    current: &(String, String, String),
) -> bool {
    matches!(outcome, SharedClickOutcome::Navigation { .. }) || snapshot == current
}

fn shared_new_tab_matches(response: &Value, expected_url: Option<&str>) -> bool {
    shared_new_tab_candidate(response, expected_url).is_some()
}

fn shared_new_tab_candidate(response: &Value, expected_url: Option<&str>) -> Option<Value> {
    let tabs = response["tabs"].as_array()?;
    let mut matching = tabs.iter().filter(|candidate| {
        candidate["tab_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
            && candidate["url"].as_str().is_some_and(|url| {
                candidate["opener_match"] == true
                    && candidate["created_at_ms"].as_u64().is_some()
                    && expected_url.is_none_or(|expected| url == expected)
            })
    });
    let candidate = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(json!({
        "tab_id":candidate["tab_id"],
        "url":candidate["url"],
        "window_id":candidate["window_id"],
        "created_at_ms":candidate["created_at_ms"],
        "opener_match":true,
        "evidence_scope":"recent_opener_bound_tab_candidate_not_account_authentication"
    }))
}

fn popup_candidate_for_adoption(
    response: &Value,
    opener_tab_id: &str,
    candidate_tab_id: &str,
    expected_created_at_ms: u64,
    since_ms: u64,
    expected_url: &str,
    now_ms: u64,
) -> Result<Value, &'static str> {
    if candidate_tab_id.is_empty()
        || candidate_tab_id.parse::<u32>().is_err()
        || expected_created_at_ms < since_ms
        || expected_created_at_ms > now_ms
        || now_ms.saturating_sub(expected_created_at_ms) > 60_000
    {
        return Err("popup candidate identity or creation time is invalid or stale");
    }
    let tabs = response["tabs"]
        .as_array()
        .ok_or("popup candidate inventory is unavailable")?;
    if tabs.len() != 1 {
        return Err("popup adoption requires exactly one recent opener-bound candidate");
    }
    let candidate = &tabs[0];
    if candidate["tab_id"].as_str() != Some(candidate_tab_id)
        || candidate["created_at_ms"].as_u64() != Some(expected_created_at_ms)
        || candidate["created_at_ms"]
            .as_u64()
            .is_none_or(|created| created < since_ms || created > now_ms)
        || candidate["url"].as_str() != Some(expected_url)
        || candidate["opener_match"] != true
        || candidate["opener_tab_id"].as_str() != Some(opener_tab_id)
    {
        return Err(
            "popup candidate is stale, closed, unrelated, or not bound to the expected opener and URL",
        );
    }
    Ok(
        json!({"tab_id":candidate_tab_id,"created_at_ms":expected_created_at_ms,"url":expected_url,"window_id":candidate["window_id"],"opener_tab_id":candidate["opener_tab_id"],"opener_match":true}),
    )
}

fn popup_account_marker_matches(
    snapshot: &Value,
    expected_url: &str,
    expected_marker: &str,
) -> bool {
    if snapshot["url"].as_str() != Some(expected_url) || snapshot["scope_complete"] != true {
        return false;
    }
    let Some(items) = snapshot["items"].as_array() else {
        return false;
    };
    let mut matching = items.iter().filter(|item| {
        (item["name"].as_str() == Some(expected_marker)
            || item["text"].as_str() == Some(expected_marker))
            && item["rendered"] == true
            && item["in_viewport"] == true
    });
    matching.next().is_some() && matching.next().is_none()
}

async fn rollback_shared_popup_adoption(shared: &mut SharedLiveSession, target_id: &str) {
    if let Some(snapshot) = shared.snapshots.remove(target_id)
        && let (Some(connection), Ok(registry)) = (shared.connection.as_ref(), shared.registry())
    {
        let _ = connection
            .command(
                registry,
                &shared.handle,
                target_id,
                "Runtime.releaseObject",
                json!({"objectId":snapshot.object_id}),
            )
            .await;
    }
    if let Some(connection) = shared.connection.as_mut() {
        connection.remove_selected_target(target_id);
    }
    if let Some(registry) = shared.registry.as_mut() {
        let _ = registry.unregister_tab(&shared.handle.id, target_id);
    }
}

fn shared_navigation_matches(
    outcome: &SharedClickOutcome,
    before: &(String, String, String),
    after: &(String, String, String),
) -> Result<bool, String> {
    let SharedClickOutcome::Navigation { url } = outcome else {
        return Ok(false);
    };
    if let Some(url) = url {
        validate_shared_url(url).map_err(|error| error.message)?;
    }
    Ok(before != after
        && url.as_ref().is_none_or(|expected| {
            url::Url::parse(expected)
                .ok()
                .zip(url::Url::parse(&after.2).ok())
                .is_some_and(|(expected, actual)| expected == actual)
        }))
}

fn validate_shared_click_outcome(outcome: &SharedClickOutcome) -> Result<(), rmcp::ErrorData> {
    match outcome {
        SharedClickOutcome::Navigation { url: Some(url) }
        | SharedClickOutcome::NewTab { url: Some(url) } => validate_shared_url(url)?,
        _ => {}
    }
    Ok(())
}

fn click_mouse_events(x: f64, y: f64) -> [Value; 2] {
    [
        json!({"type":"mousePressed","button":"left","clickCount":1,"x":x,"y":y}),
        json!({"type":"mouseReleased","button":"left","clickCount":1,"x":x,"y":y}),
    ]
}

fn pointer_click_events(x: f64, y: f64, kind: &str) -> Result<Vec<Value>, &'static str> {
    let (button, count) = match kind {
        "click" => ("left", 1),
        "double_click" => ("left", 2),
        "context_click" => ("right", 1),
        _ => return Err("click_kind must be click, double_click, or context_click"),
    };
    let mut events = Vec::with_capacity(count as usize * 2);
    for click_index in 1..=count {
        events.push(
            json!({"type":"mousePressed","button":button,"clickCount":click_index,"x":x,"y":y}),
        );
        events.push(
            json!({"type":"mouseReleased","button":button,"clickCount":click_index,"x":x,"y":y}),
        );
    }
    Ok(events)
}

fn pointer_events(x: f64, y: f64, mode: &str) -> Result<(&'static str, [Value; 2]), &'static str> {
    match mode {
        "mouse" => Ok(("Input.dispatchMouseEvent", click_mouse_events(x, y))),
        "touch" => Ok((
            "Input.dispatchTouchEvent",
            [
                json!({"type":"touchStart","touchPoints":[{"id":1,"x":x,"y":y,"radiusX":1,"radiusY":1,"force":1}]}),
                json!({"type":"touchEnd","touchPoints":[]}),
            ],
        )),
        _ => Err("input_mode must be mouse or touch"),
    }
}

#[derive(Clone)]
pub(crate) struct App {
    manager: BrowserManager,
    principal: Arc<str>,
    sessions: Arc<Mutex<BTreeMap<String, Arc<LiveSession>>>>,
    shared_sessions: Arc<Mutex<BTreeMap<String, Arc<Mutex<SharedLiveSession>>>>>,
    jobs: crate::jobs::Journal,
}

impl Default for App {
    fn default() -> Self {
        Self::with_journal(crate::jobs::Journal::open(":memory:").expect("in-memory journal"))
    }
}

impl App {
    fn with_journal(jobs: crate::jobs::Journal) -> Self {
        Self::with_principal_and_journal(LOCAL_STDIO_PRINCIPAL, jobs)
    }

    pub(crate) fn with_principal_and_journal(
        principal: impl Into<Arc<str>>,
        jobs: crate::jobs::Journal,
    ) -> Self {
        Self {
            manager: BrowserManager::default(),
            principal: principal.into(),
            sessions: Arc::default(),
            shared_sessions: Arc::default(),
            jobs,
        }
    }

    pub(crate) fn principal(&self) -> &str {
        &self.principal
    }

    fn owns_session(&self, session: &LiveSession) -> bool {
        session.handle.principal == self.principal.as_ref()
    }
}

fn workflow_receipt(
    operation_id: &str,
    status: &str,
    session_id: &str,
    reference: &TargetRef,
    browser_operations: u64,
    steps: &[Value],
    error: Option<&str>,
) -> Value {
    json!({
        "schema_version":"1", "operation_id":operation_id, "status":status,
        "replayed":false,
        "target":{"session_id":session_id,"target_id":reference.target_id,
            "navigation_epoch":reference.frame_revision,"target_revision":reference.target_revision,
            "account_revision":reference.account_revision,"document_revision":reference.document_revision},
        "result":{"completed_steps":steps.len(),"browser_operations":browser_operations,"steps":steps},
        "metrics":{"browser_operations":browser_operations,"internal_model_calls":0},
        "evidence":[], "artifacts":[], "error":error
    })
}

async fn run_workflow_job(
    jobs: crate::jobs::Journal,
    principal: Arc<str>,
    session: Arc<LiveSession>,
    session_id: String,
    operation_id: String,
    reference: TargetRef,
    graph: crate::workflow::WorkflowGraph,
) {
    let principal_ref = principal.clone();
    if !jobs
        .start(principal_ref.as_ref(), &session_id, &operation_id)
        .unwrap_or(false)
    {
        return;
    }
    let Some(admitted) = jobs
        .get(principal_ref.as_ref(), &session_id, &operation_id)
        .ok()
        .flatten()
    else {
        return;
    };
    let remaining = admitted.deadline_at_ms.saturating_sub(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(remaining);
    let mut completed = Vec::new();
    if let Some(source) = graph.steps.first().and_then(|step| match step {
        crate::workflow::WorkflowStep::Script { source } => Some(source.clone()),
        _ => None,
    }) {
        let script_jobs = jobs.clone();
        let script_session = session.clone();
        let script_session_id = session_id.clone();
        let script_operation_id = operation_id.clone();
        let script_reference = reference.clone();
        let script_completed = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
        let callback_jobs = script_jobs.clone();
        let callback_session = script_session.clone();
        let callback_sid = script_session_id.clone();
        let callback_opid = script_operation_id.clone();
        let callback_ref = script_reference.clone();
        let callback_completed = script_completed.clone();
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let callback_count = call_count.clone();
        let callback_deadline = deadline;
        let callback_principal = principal_ref.clone();
        let broker: crate::workflow::ScriptBroker = Arc::new(move |raw| {
            let jobs = callback_jobs.clone();
            let session = callback_session.clone();
            let session_id = callback_sid.clone();
            let operation_id = callback_opid.clone();
            let reference = callback_ref.clone();
            let completed = callback_completed.clone();
            let call_count = callback_count.clone();
            let deadline = callback_deadline;
            let principal = callback_principal.clone();
            Box::pin(async move {
                if call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    >= crate::workflow::MAX_OBSERVE_CALLS
                {
                    return Err("script browser operation limit exceeded".into());
                }
                let spec: crate::workflow::WorkflowObserveSpec = serde_json::from_str(&raw)
                    .map_err(|_| "invalid brokered observe spec".to_owned())?;
                crate::workflow::compile_steps(vec![crate::workflow::WorkflowStep::Observe {
                    spec: spec.clone(),
                }])
                .map_err(|e| e.to_owned())?;
                let registry = session.registry.lock().await;
                if reference.session_id != session_id
                    || reference.principal != session.handle.principal
                    || !registry.contains_target(&session.handle, &reference.target_id)
                {
                    return Err("target_ref is not bound to this session".into());
                }
                let index = completed.lock().await.len();
                let correlation = format!("{operation_id}:script:{index}");
                let claim = jobs
                    .record_dispatch(principal.as_ref(), &session_id, &operation_id, &correlation)
                    .map_err(|e| e.to_string())?;
                if !claim.acquired {
                    return Err("operation dispatch could not be claimed".into());
                }
                let pending = workflow_receipt(
                    &operation_id,
                    "running",
                    &session_id,
                    &reference,
                    claim.operation.dispatch_count,
                    &completed.lock().await,
                    None,
                );
                jobs.checkpoint(principal.as_ref(), &session_id, &operation_id, pending)
                    .map_err(|e| e.to_string())?;
                let step_deadline = deadline
                    .min(tokio::time::Instant::now() + tokio::time::Duration::from_secs(10));
                match tokio::time::timeout_at(
                    step_deadline,
                    session.connection.observe(
                        &registry,
                        &reference,
                        &session.handle.principal,
                        IdentityRevisions {
                            account: reference.account_revision,
                            document: reference.document_revision,
                        },
                        &spec.clone().into_observe_spec(),
                    ),
                )
                .await
                {
                    Ok(Ok(observation)) => {
                        jobs.acknowledge_dispatch(
                            principal.as_ref(),
                            &session_id,
                            &operation_id,
                            &correlation,
                        )
                        .map_err(|e| e.to_string())?;
                        let value = json!({"kind":"observe","observation":observation});
                        let mut done = completed.lock().await;
                        done.push(value);
                        let receipt = workflow_receipt(
                            &operation_id,
                            "running",
                            &session_id,
                            &reference,
                            claim.operation.dispatch_count,
                            &done,
                            None,
                        );
                        jobs.checkpoint(principal.as_ref(), &session_id, &operation_id, receipt)
                            .map_err(|e| e.to_string())?;
                        serde_json::to_string(&observation).map_err(|e| e.to_string())
                    }
                    Ok(Err(error)) => {
                        let receipt = workflow_receipt(
                            &operation_id,
                            "unknown",
                            &session_id,
                            &reference,
                            claim.operation.dispatch_count,
                            &completed.lock().await,
                            Some(&error.to_string()),
                        );
                        let _ = jobs.mark_unknown(
                            principal.as_ref(),
                            &session_id,
                            &operation_id,
                            receipt,
                        );
                        Err("browser operation delivery is unknown".into())
                    }
                    Err(_) => {
                        let receipt = workflow_receipt(
                            &operation_id,
                            "unknown",
                            &session_id,
                            &reference,
                            claim.operation.dispatch_count,
                            &completed.lock().await,
                            Some("step outcome unknown after timeout"),
                        );
                        let _ = jobs.mark_unknown(
                            principal.as_ref(),
                            &session_id,
                            &operation_id,
                            receipt,
                        );
                        Err("browser operation delivery is unknown after timeout".into())
                    }
                }
            })
        });
        let timeout_ms = remaining.min(crate::workflow::MAX_JOB_DEADLINE_MS);
        let heap_limit = 8 * 1024 * 1024;
        let worker_executable = crate::workflow::script_worker_executable();
        let worker = match worker_executable {
            Ok(executable) => {
                crate::workflow::run_script_isolated(
                    &executable,
                    &source,
                    timeout_ms,
                    heap_limit,
                    broker,
                )
                .await
            }
            Err(error) => Err(format!(
                "could not resolve script worker executable: {error}"
            )),
        };
        match worker {
            Ok(output) => {
                let artifact = match crate::workflow::validate_script_artifact(
                    &output,
                    &script_operation_id,
                    principal_ref.as_ref(),
                    &script_session_id,
                ) {
                    Ok(artifact) => artifact,
                    Err(error) => {
                        let steps = script_completed.lock().await;
                        let count = script_jobs
                            .get(
                                principal_ref.as_ref(),
                                &script_session_id,
                                &script_operation_id,
                            )
                            .ok()
                            .flatten()
                            .map_or(0, |op| op.dispatch_count);
                        let receipt = workflow_receipt(
                            &script_operation_id,
                            "failed",
                            &script_session_id,
                            &script_reference,
                            count,
                            &steps,
                            Some(&error),
                        );
                        let _ = script_jobs.fail(
                            principal_ref.as_ref(),
                            &script_session_id,
                            &script_operation_id,
                            receipt,
                        );
                        return;
                    }
                };
                let mut steps = script_completed.lock().await;
                let script_output = artifact.as_ref().map_or_else(
                    || json!(output),
                    |artifact| json!({"artifact_id":artifact["artifact_id"]}),
                );
                steps.push(json!({"kind":"script","output":script_output}));
                let count = script_jobs
                    .get(
                        principal_ref.as_ref(),
                        &script_session_id,
                        &script_operation_id,
                    )
                    .ok()
                    .flatten()
                    .map_or(0, |op| op.dispatch_count);
                let mut receipt = workflow_receipt(
                    &script_operation_id,
                    "completed",
                    &script_session_id,
                    &script_reference,
                    count,
                    &steps,
                    None,
                );
                receipt["artifacts"] = json!(artifact.into_iter().collect::<Vec<_>>());
                let _ = script_jobs.complete(
                    principal_ref.as_ref(),
                    &script_session_id,
                    &script_operation_id,
                    receipt,
                );
            }
            Err(error) => {
                if script_jobs
                    .get(
                        principal_ref.as_ref(),
                        &script_session_id,
                        &script_operation_id,
                    )
                    .ok()
                    .flatten()
                    .is_some_and(|op| op.delivery == crate::jobs::Delivery::Unknown)
                {
                    let steps = script_completed.lock().await;
                    let count = script_jobs
                        .get(
                            principal_ref.as_ref(),
                            &script_session_id,
                            &script_operation_id,
                        )
                        .ok()
                        .flatten()
                        .map_or(0, |op| op.dispatch_count);
                    let receipt = workflow_receipt(
                        &script_operation_id,
                        "unknown",
                        &script_session_id,
                        &script_reference,
                        count,
                        &steps,
                        Some("script stopped while browser delivery remained unknown"),
                    );
                    let _ = script_jobs.mark_unknown(
                        principal_ref.as_ref(),
                        &script_session_id,
                        &script_operation_id,
                        receipt,
                    );
                    return;
                }
                let steps = script_completed.lock().await;
                let count = script_jobs
                    .get(
                        principal_ref.as_ref(),
                        &script_session_id,
                        &script_operation_id,
                    )
                    .ok()
                    .flatten()
                    .map_or(0, |op| op.dispatch_count);
                let receipt = workflow_receipt(
                    &script_operation_id,
                    "failed",
                    &script_session_id,
                    &script_reference,
                    count,
                    &steps,
                    Some(&error),
                );
                let _ = script_jobs.fail(
                    principal_ref.as_ref(),
                    &script_session_id,
                    &script_operation_id,
                    receipt,
                );
            }
        }
        return;
    }
    for (index, step) in graph.steps.iter().enumerate() {
        let outcome = match step {
            crate::workflow::WorkflowStep::Wait { ms } => {
                match tokio::time::timeout_at(
                    deadline,
                    tokio::time::sleep(std::time::Duration::from_millis(*ms)),
                )
                .await
                {
                    Ok(()) => Ok(json!({"kind":"wait","ms":ms})),
                    Err(_) => Err("job deadline exceeded".to_owned()),
                }
            }
            crate::workflow::WorkflowStep::Checkpoint => {
                Ok(json!({"kind":"checkpoint","after_step":index}))
            }
            crate::workflow::WorkflowStep::Script { .. } => {
                Err("script step must run as an isolated worker".into())
            }
            crate::workflow::WorkflowStep::Observe { spec } => {
                let registry = session.registry.lock().await;
                if reference.session_id != session_id
                    || reference.principal != session.handle.principal
                    || !registry.contains_target(&session.handle, &reference.target_id)
                {
                    Err("target_ref is not bound to this session".into())
                } else {
                    let correlation = format!("{operation_id}:step:{index}");
                    let claim = match jobs.record_dispatch(
                        principal_ref.as_ref(),
                        &session_id,
                        &operation_id,
                        &correlation,
                    ) {
                        Ok(claim) if claim.acquired => claim,
                        _ => return,
                    };
                    let pending = workflow_receipt(
                        &operation_id,
                        "running",
                        &session_id,
                        &reference,
                        claim.operation.dispatch_count,
                        &completed,
                        None,
                    );
                    if jobs
                        .checkpoint(principal_ref.as_ref(), &session_id, &operation_id, pending)
                        .is_err()
                    {
                        return;
                    }
                    let spec = spec.clone().into_observe_spec();
                    let step_deadline = deadline
                        .min(tokio::time::Instant::now() + std::time::Duration::from_secs(10));
                    match tokio::time::timeout_at(
                        step_deadline,
                        session.connection.observe(
                            &registry,
                            &reference,
                            &session.handle.principal,
                            IdentityRevisions {
                                account: reference.account_revision,
                                document: reference.document_revision,
                            },
                            &spec,
                        ),
                    )
                    .await
                    {
                        Ok(Ok(observation)) => {
                            if !jobs
                                .acknowledge_dispatch(
                                    principal_ref.as_ref(),
                                    &session_id,
                                    &operation_id,
                                    &correlation,
                                )
                                .unwrap_or(false)
                            {
                                return;
                            }
                            Ok(json!({"kind":"observe","observation":observation}))
                        }
                        Ok(Err(error)) => {
                            let receipt = workflow_receipt(
                                &operation_id,
                                "unknown",
                                &session_id,
                                &reference,
                                claim.operation.dispatch_count,
                                &completed,
                                Some(&error.to_string()),
                            );
                            let _ = jobs.mark_unknown(
                                principal_ref.as_ref(),
                                &session_id,
                                &operation_id,
                                receipt,
                            );
                            return;
                        }
                        Err(_) => {
                            let receipt = workflow_receipt(
                                &operation_id,
                                "unknown",
                                &session_id,
                                &reference,
                                claim.operation.dispatch_count,
                                &completed,
                                Some("step outcome unknown after timeout"),
                            );
                            let _ = jobs.mark_unknown(
                                principal_ref.as_ref(),
                                &session_id,
                                &operation_id,
                                receipt,
                            );
                            return;
                        }
                    }
                }
            }
        };
        match outcome {
            Ok(value) => completed.push(value),
            Err(error) => {
                let receipt = workflow_receipt(
                    &operation_id,
                    "failed",
                    &session_id,
                    &reference,
                    jobs.get(principal_ref.as_ref(), &session_id, &operation_id)
                        .ok()
                        .flatten()
                        .map_or(0, |op| op.dispatch_count),
                    &completed,
                    Some(&error),
                );
                let _ = jobs.fail(principal_ref.as_ref(), &session_id, &operation_id, receipt);
                return;
            }
        }
        let receipt = workflow_receipt(
            &operation_id,
            "running",
            &session_id,
            &reference,
            jobs.get(principal_ref.as_ref(), &session_id, &operation_id)
                .ok()
                .flatten()
                .map_or(0, |op| op.dispatch_count),
            &completed,
            None,
        );
        if jobs
            .checkpoint(principal_ref.as_ref(), &session_id, &operation_id, receipt)
            .is_err()
        {
            return;
        }
    }
    let receipt = workflow_receipt(
        &operation_id,
        "completed",
        &session_id,
        &reference,
        jobs.get(principal_ref.as_ref(), &session_id, &operation_id)
            .ok()
            .flatten()
            .map_or(0, |op| op.dispatch_count),
        &completed,
        None,
    );
    let _ = jobs.complete(principal_ref.as_ref(), &session_id, &operation_id, receipt);
}

fn operation_response(operation: crate::jobs::Operation, replayed: bool) -> Result<Value, String> {
    Ok(json!({
        "schema_version":"1", "operation_id":operation.id,
        "status":operation.status, "delivery":operation.delivery,
        "revision":operation.revision, "deadline_at_ms":operation.deadline_at_ms,
        "deadline_error":operation.deadline_error, "replayed":replayed,
        "metrics":{"browser_operations":operation.dispatch_count,"internal_model_calls":0},
        "result":operation.result
    }))
}

async fn run_file_select(
    s: Arc<LiveSession>,
    principal: &str,
    args: FileSelectArgs,
) -> Result<Value, String> {
    let reference: TargetRef =
        serde_json::from_value(serde_json::to_value(args.target_ref).map_err(|e| e.to_string())?)
            .map_err(|_| "target_ref must be a complete current TargetRef".to_owned())?;
    if reference.session_id != args.session_id {
        return Err("target_ref session does not match session_id".into());
    }
    if s.handle.mode != SessionMode::DirectCdp
        || s.handle.principal != principal
        || reference.principal != principal
    {
        return Err("file selection requires the matching direct CDP session".into());
    }
    let locator_value = serde_json::to_value(args.locator).map_err(|e| e.to_string())?;
    let locator =
        controlla_browser::Locator::from_value(&locator_value).map_err(|e| e.to_string())?;
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

struct SharedTypeGuard<'a> {
    tab_id: &'a str,
    object_id: &'a str,
    selector: &'a str,
    expected: &'a str,
    final_check: bool,
    deadline: tokio::time::Instant,
}

const SHARED_TYPE_INTER_CHARACTER_DELAY: Duration = Duration::from_millis(60);
const SHARED_CLICK_NAVIGATION_GRACE: Duration = Duration::from_secs(3);

fn shared_typing_minimum_duration(character_count: usize) -> Duration {
    SHARED_TYPE_INTER_CHARACTER_DELAY.saturating_mul(
        character_count
            .saturating_sub(1)
            .try_into()
            .unwrap_or(u32::MAX),
    )
}

async fn shared_type_guard(
    connection: &PageSessionTransport,
    registry: &SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    guard: SharedTypeGuard<'_>,
) -> Result<Value, String> {
    let response = tokio::time::timeout_at(
        guard.deadline,
        connection.command(
            registry,
            handle,
            guard.tab_id,
            "Runtime.callFunctionOn",
            json!({
                "objectId":guard.object_id,
                "functionDeclaration":SHARED_TYPE_GUARD_FUNCTION,
                "arguments":[{"value":guard.selector},{"value":guard.expected},{"value":guard.final_check}],
                "returnByValue":true
            }),
        ),
    )
    .await
    .map_err(|_| "typing guard deadline exceeded".to_owned())?
    ?;
    Ok(response
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

async fn shared_release_key(
    connection: &PageSessionTransport,
    registry: &SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
    key_up: &Value,
    deadline: tokio::time::Instant,
) -> Result<(), String> {
    let mut first_error = None;
    for attempt in 0..2 {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(
                first_error.unwrap_or_else(|| "key release recovery deadline expired".into())
            );
        }
        let attempt_deadline = std::cmp::min(now + Duration::from_millis(250), deadline);
        let result = tokio::time::timeout_at(
            attempt_deadline,
            connection.command(
                registry,
                handle,
                tab_id,
                "Input.dispatchKeyEvent",
                key_up.clone(),
            ),
        )
        .await
        .map_err(|_| "release acknowledgement timed out".to_owned())
        .and_then(|result| result);
        match (attempt, result) {
            (0, Ok(_)) => return Ok(()),
            (1, Ok(_)) => {
                return Err(format!(
                    "initial key release was uncertain; bounded retry was acknowledged: {}",
                    first_error.unwrap_or_default()
                ));
            }
            (0, Err(error)) => first_error = Some(error),
            (1, Err(error)) => {
                return Err(format!(
                    "key release remained uncertain after one bounded retry: {}; {error}",
                    first_error.unwrap_or_default()
                ));
            }
            _ => unreachable!(),
        }
    }
    unreachable!()
}

async fn shared_frame_identity(
    connection: &PageSessionTransport,
    registry: &SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
) -> Result<(String, String, String), String> {
    let response = connection
        .command(registry, handle, tab_id, "Page.getFrameTree", Value::Null)
        .await?;
    frame_identity_from_tree(&response)
}

async fn shared_interaction_epoch(
    connection: &PageSessionTransport,
    registry: &SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
    object_id: &str,
) -> Result<Value, String> {
    let response = connection.command(
        registry,
        handle,
        tab_id,
        "Runtime.callFunctionOn",
        json!({
            "objectId": object_id,
            "functionDeclaration": r#"function(){const s=window[Symbol.for('controlla.interactionEpoch.v1')];if(!s)return null;const a=document.activeElement;const focus=a?JSON.stringify([a.tagName,a.id||'',a.getAttribute('name')||'',a.getAttribute('role')||'',a.getAttribute('aria-label')||'']):'';let selection='';if(a&&typeof a.selectionStart==='number')selection=JSON.stringify([a.selectionStart,a.selectionEnd,a.selectionDirection||'none']);else{const x=window.getSelection?.();if(x)selection=JSON.stringify([x.anchorNode?.nodeType,x.anchorOffset,x.focusNode?.nodeType,x.focusOffset,x.isCollapsed])}const viewport=JSON.stringify([innerWidth,innerHeight,scrollX,scrollY,visualViewport?.width,visualViewport?.height,visualViewport?.offsetLeft,visualViewport?.offsetTop,visualViewport?.scale]);return {semantic_revision:s.semantic,focus,selection,viewport}}"#,
            "returnByValue": true
        }),
    ).await?;
    shared_value(&response)
}

async fn shared_identity_and_epoch(
    connection: &PageSessionTransport,
    registry: &SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
    object_id: &str,
    deadline: Duration,
) -> Result<((String, String, String), Value), String> {
    if matches!(connection, PageSessionTransport::Shared(shared) if shared.supports_batch_execution())
    {
        let batch = connection
            .command_batch(
                registry,
                handle,
                tab_id,
                vec![
                    controlla_browser::providers::SharedBatchAction {
                        method: "Runtime.callFunctionOn".into(),
                        params: json!({
                            "objectId": object_id,
                            "functionDeclaration": r#"function(){const s=window[Symbol.for('controlla.interactionEpoch.v1')];if(!s)return null;const a=document.activeElement;const focus=a?JSON.stringify([a.tagName,a.id||'',a.getAttribute('name')||'',a.getAttribute('role')||'',a.getAttribute('aria-label')||'']):'';let selection='';if(a&&typeof a.selectionStart==='number')selection=JSON.stringify([a.selectionStart,a.selectionEnd,a.selectionDirection||'none']);else{const x=window.getSelection?.();if(x)selection=JSON.stringify([x.anchorNode?.nodeType,x.anchorOffset,x.focusNode?.nodeType,x.focusOffset,x.isCollapsed])}const viewport=JSON.stringify([innerWidth,innerHeight,scrollX,scrollY,visualViewport?.width,visualViewport?.height,visualViewport?.offsetLeft,visualViewport?.offsetTop,visualViewport?.scale]);return {semantic_revision:s.semantic,focus,selection,viewport}}"#,
                            "returnByValue": true
                        }),
                        stop_on_not_ok: false,
                    },
                    controlla_browser::providers::SharedBatchAction {
                        method: "Page.getFrameTree".into(),
                        params: Value::Null,
                        stop_on_not_ok: false,
                    },
                ],
                deadline,
            )
            .await
            .map_err(|error| error.to_string())?;
        return shared_identity_epoch_batch(&batch);
    }
    let identity = shared_frame_identity(connection, registry, handle, tab_id).await?;
    let epoch = shared_interaction_epoch(connection, registry, handle, tab_id, object_id).await?;
    Ok((identity, epoch))
}

#[derive(Debug, Clone)]
struct SharedOperationRead {
    identity: (String, String, String),
    interaction_epoch: Value,
}

impl SharedOperationRead {
    fn new(identity: (String, String, String), interaction_epoch: Value) -> Self {
        Self {
            identity,
            interaction_epoch,
        }
    }

    fn remains_fresh(&self, identity: &(String, String, String), epoch: &Value) -> bool {
        &self.identity == identity && self.interaction_epoch == *epoch
    }
}

fn frame_identity_from_tree(response: &Value) -> Result<(String, String, String), String> {
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
    let fragment = frame["urlFragment"].as_str().unwrap_or("");
    Ok((id.to_owned(), loader.to_owned(), format!("{url}{fragment}")))
}

fn shared_identity_epoch_batch(batch: &Value) -> Result<((String, String, String), Value), String> {
    if batch["completed"] != 2 || batch.get("batch_error").is_some() {
        return Err("identity/epoch batch did not complete".into());
    }
    let receipts = batch["receipts"]
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or("identity/epoch batch omitted receipts")?;
    if receipts[0]["method"] != "Runtime.callFunctionOn"
        || receipts[1]["method"] != "Page.getFrameTree"
        || receipts.iter().any(|item| item.get("error").is_some())
    {
        return Err("identity/epoch batch returned unexpected receipts".into());
    }
    Ok((
        frame_identity_from_tree(&receipts[1]["result"])?,
        shared_value(&receipts[0]["result"])
            .map_err(|error| format!("interaction epoch read failed: {error}"))?,
    ))
}

fn frame_handles_from_tree(
    response: &Value,
    target_id: &str,
) -> Result<(Vec<controlla_browser::FrameHandle>, bool), String> {
    const MAX_FRAME_HANDLES: usize = 256;
    const MAX_FRAME_DEPTH: usize = 16;
    let root = response
        .get("frameTree")
        .ok_or_else(|| "Page.getFrameTree omitted frameTree".to_owned())?;
    let mut handles = Vec::new();
    let mut truncated = false;
    let mut pending = vec![(root, None::<String>, 0usize)];
    while let Some((node, inherited_parent, depth)) = pending.pop() {
        if depth > MAX_FRAME_DEPTH || handles.len() >= MAX_FRAME_HANDLES {
            truncated = true;
            continue;
        }
        let Some(frame) = node.get("frame") else {
            truncated = true;
            continue;
        };
        let Some(frame_id) = frame
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            truncated = true;
            continue;
        };
        let parent_id = frame
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(inherited_parent);
        handles.push(controlla_browser::FrameHandle {
            // Page.getFrameTree reports frame ancestry but not OOPIF target
            // ownership. Only the root is safely bound to the selected tab.
            target_id: (depth == 0).then(|| target_id.to_owned()),
            // The extension protocol exposes frame IDs, not its debugger session IDs.
            session_id: None,
            frame_id: frame_id.to_owned(),
            loader_id: frame
                .get("loaderId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            parent_id: parent_id.clone(),
            origin: frame
                .get("securityOrigin")
                .and_then(Value::as_str)
                .map(str::to_owned),
            // Shared extension frame-tree reads do not expose transform measurements.
            transform_revision: 0,
            accessibility_state: "not_probed".to_owned(),
        });
        if let Some(children) = node.get("childFrames").and_then(Value::as_array) {
            pending.extend(
                children
                    .iter()
                    .map(|child| (child, Some(frame_id.to_owned()), depth + 1)),
            );
        }
    }
    Ok((handles, truncated))
}

fn public_frame_handles(handles: &[controlla_browser::FrameHandle]) -> Result<Value, String> {
    let mut values = serde_json::to_value(handles).map_err(|error| error.to_string())?;
    if let Some(items) = values.as_array_mut() {
        for handle in items {
            // The shared extension exposes frame IDs, but not debugger
            // session IDs or measured renderer transforms.
            handle["route"] = json!("frame_id");
            handle["transform_revision"] = Value::Null;
            handle["transform_state"] = json!("unmeasured");
        }
    }
    Ok(values)
}

fn ensure_snapshot_byte_budget(value: &Value, max_bytes: usize) -> Result<(), String> {
    if serde_json::to_vec(value)
        .map_err(|error| error.to_string())?
        .len()
        > max_bytes
    {
        return Err("snapshot exceeded byte budget; reduce text/items".into());
    }
    Ok(())
}

/// Bind the existing single-use snapshot token and live root-frame identity to the
/// interaction-epoch contract. Element hit-test/readback guards still validate the
/// target-specific and viewport dimensions immediately before input dispatch.
fn shared_epoch_matches(
    snapshot: &SharedPageSnapshot,
    target_id: &str,
    current: &(String, String, String),
    current_interaction: &Value,
    use_case: crate::v3::EpochUse,
) -> bool {
    if !snapshot.interaction_epoch.is_object() || !current_interaction.is_object() {
        return false;
    }
    use sha2::Digest as _;
    let revision = |bytes: &[u8]| {
        sha2::Sha256::digest(bytes)
            .iter()
            .take(8)
            .fold(0u64, |n, b| (n << 8) | u64::from(*b))
    };
    let make = |identity: &(String, String, String), interaction: &Value| {
        let focus = interaction
            .get("focus")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let selection = interaction
            .get("selection")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let viewport = interaction
            .get("viewport")
            .and_then(Value::as_str)
            .unwrap_or_default();
        crate::v3::InteractionEpoch {
            browser_generation: 1,
            target_id: target_id.to_owned(),
            target_revision: revision(target_id.as_bytes()).to_string(),
            frame_id: identity.0.clone(),
            loader_id: identity.1.clone(),
            document_id: format!("{}:{}", identity.1, revision(identity.2.as_bytes())),
            focused_backend_node_id: (!focus.is_empty()).then(|| revision(focus.as_bytes()) as i64),
            selection_fingerprint: Some(selection.to_owned()),
            semantic_revision: interaction
                .get("semantic_revision")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                .to_string(),
            viewport_revision: revision(viewport.as_bytes()),
        }
    };
    let grounded = make(&snapshot.identity, &snapshot.interaction_epoch);
    let fresh = make(current, current_interaction);
    if use_case == crate::v3::EpochUse::GuardedClick {
        // Only guarded semantic clicks revalidate the exact node, its name/value,
        // visibility, hit target, and viewport in the browser-side guard immediately
        // before dispatch. Other pointer mutations keep the strict page semantic epoch.
        grounded.browser_generation == fresh.browser_generation
            && grounded.target_id == fresh.target_id
            && grounded.target_revision == fresh.target_revision
            && grounded.frame_id == fresh.frame_id
            && grounded.loader_id == fresh.loader_id
            && grounded.document_id == fresh.document_id
            && grounded.viewport_revision == fresh.viewport_revision
    } else {
        grounded.compatible_with(&fresh, use_case)
    }
}

fn final_click_guard_matches_viewport(result: &Value, prepared_viewport: Option<&str>) -> bool {
    result["ok"] == true
        && result["x"].as_f64().is_some_and(f64::is_finite)
        && result["y"].as_f64().is_some_and(f64::is_finite)
        && result["viewport"]
            .as_str()
            .is_some_and(|viewport| prepared_viewport.is_none_or(|prepared| viewport == prepared))
}

fn find_cdp_frame(response: &Value, frame_id: &str) -> Option<(String, String)> {
    fn walk(tree: &Value, id: &str) -> Option<(String, String)> {
        let frame = tree.get("frame")?;
        if frame.get("id").and_then(Value::as_str) == Some(id) {
            return Some((
                frame
                    .get("loaderId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                frame
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
        tree.get("childFrames")?
            .as_array()?
            .iter()
            .find_map(|child| walk(child, id))
    }
    walk(response.get("frameTree")?, frame_id)
}

async fn wait_for_shared_frame_change(
    connection: &PageSessionTransport,
    registry: &SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
    before: &(String, String, String),
    deadline: tokio::time::Instant,
) -> Result<(String, String, String), String> {
    let deadline = std::cmp::min(
        deadline,
        tokio::time::Instant::now() + SHARED_CLICK_NAVIGATION_GRACE,
    );
    let mut last_error = None;
    loop {
        match tokio::time::timeout_at(
            deadline,
            shared_frame_identity(connection, registry, handle, tab_id),
        )
        .await
        {
            Ok(Ok(identity)) if &identity != before => return Ok(identity),
            Ok(Ok(_)) => {}
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                return Err(last_error.unwrap_or_else(|| "tab navigation was not observed".into()));
            }
        }
        let next = tokio::time::Instant::now() + Duration::from_millis(50);
        if tokio::time::timeout_at(deadline, tokio::time::sleep_until(next))
            .await
            .is_err()
        {
            return Err(last_error.unwrap_or_else(|| "tab navigation was not observed".into()));
        }
    }
}

#[tool_router]
impl App {
    #[tool(
        name = "workflow",
        description = "Compile and run a bounded deterministic workflow graph. Trusted-local JavaScript is opt-in via CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS=1; QuickJS runs in a bounded child process with brokered reads only."
    )]
    async fn workflow(
        &self,
        Parameters(args): Parameters<WorkflowArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let graph = crate::workflow::compile_steps(args.steps).map_err(invalid)?;
        if graph
            .steps
            .iter()
            .any(|step| matches!(step, crate::workflow::WorkflowStep::Script { .. }))
            && std::env::var("CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS").as_deref() != Ok("1")
        {
            return Err(invalid(
                "trusted local scripts are disabled; set CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS=1 only for scripts from a trusted operator",
            ));
        }
        let reference: TargetRef = serde_json::from_value(
            serde_json::to_value(args.target_ref).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|_| invalid("target_ref must be a complete current TargetRef"))?;
        let s = self
            .sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
        crate::workflow::validate_binding(
            &args.session_id,
            &reference.session_id,
            &reference.principal,
            &self.principal,
        )
        .map_err(invalid)?;
        let request = json!({"target_ref":reference,"steps":graph.steps});
        let admission = self
            .jobs
            .admit_with_deadline(
                &self.principal,
                &args.session_id,
                &args.idempotency_key,
                &request,
                crate::workflow::MAX_JOB_DEADLINE_MS,
            )
            .map_err(|error| invalid(error.to_string()))?;
        if !admission.replayed || admission.operation.status == crate::jobs::JobStatus::Accepted {
            tokio::spawn(run_workflow_job(
                self.jobs.clone(),
                self.principal.clone(),
                s,
                args.session_id,
                admission.operation.id.clone(),
                reference,
                graph,
            ));
        }
        Ok(rmcp::handler::server::wrapper::Json(
            operation_response(admission.operation, admission.replayed).map_err(invalid)?,
        ))
    }

    #[tool(
        name = "workflow_status",
        description = "Read the durable status and latest checkpoint for a workflow operation. Set cancel=true to atomically stop an accepted or between-dispatch job; a possibly in-flight remote operation cannot be cancelled and must be reconciled."
    )]
    async fn workflow_status(
        &self,
        Parameters(args): Parameters<WorkflowStatusArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let cancellation_requested = if args.cancel {
            self.jobs
                .cancel(&self.principal, &args.session_id, &args.operation_id)
                .map_err(|error| invalid(error.to_string()))?
        } else {
            false
        };
        let operation = self
            .jobs
            .get(&self.principal, &args.session_id, &args.operation_id)
            .map_err(|error| invalid(error.to_string()))?
            .ok_or_else(|| invalid("unknown operation_id for this session"))?;
        let mut response = operation_response(operation, true).map_err(invalid)?;
        if args.cancel {
            response["cancellation_requested"] = json!(cancellation_requested);
            response["cancellation_boundary"] = json!(if cancellation_requested {
                "fenced_before_next_dispatch"
            } else {
                "operation_may_be_in_flight_or_already_terminal"
            });
        }
        Ok(rmcp::handler::server::wrapper::Json(response))
    }

    #[tool(
        name = "artifact_register",
        description = "Register bounded bytes under an opaque artifact handle scoped to this direct CDP session."
    )]
    async fn artifact_register(
        &self,
        Parameters(args): Parameters<ArtifactRegisterArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        if args.bytes.len() > 10 * 1024 * 1024 {
            return Err(invalid("artifact bytes must be at most 10485760 bytes"));
        }
        let s = self
            .sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
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
        let expectation = crate::artifacts::ArtifactExpectation::from_bytes(&args.bytes);
        s.artifact_expectations
            .lock()
            .await
            .insert(metadata.handle.opaque_id().to_owned(), expectation.clone());
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "handle": metadata.handle, "filename": metadata.filename, "size": metadata.size,
            "sha256": expectation.sha256,
            "evidence": "registered bytes and expected digest retained in the ephemeral principal/session-scoped store"
        })))
    }

    #[tool(
        name = "artifact_verify",
        description = "Re-read a registered artifact through its opaque principal/session-scoped handle and compare its bytes with the digest captured at registration. Verifies local staging integrity only; does not prove app acceptance, persistence, or download origin."
    )]
    async fn artifact_verify(
        &self,
        Parameters(args): Parameters<ArtifactVerifyArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let s = self
            .sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session_id"))?;
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
        let handle: controlla_browser::sessions::ArtifactHandle =
            serde_json::from_value(json!(args.artifact_handle))
                .map_err(|_| invalid("artifact_handle is invalid"))?;
        let expected = s
            .artifact_expectations
            .lock()
            .await
            .get(handle.opaque_id())
            .cloned()
            .ok_or_else(|| invalid("artifact expectation is unavailable for this session"))?;
        let registry = s.registry.lock().await;
        let bytes = registry
            .read_artifact_bytes(&s.handle, &handle)
            .map_err(|error| invalid(format!("artifact readback failed: {error:?}")))?;
        let actual = crate::artifacts::ArtifactExpectation::from_bytes(&bytes);
        let matched = actual == expected;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "status": if matched { "passed" } else { "failed" },
            "expected": {"byte_length": expected.byte_length, "sha256": expected.sha256},
            "actual": {"byte_length": actual.byte_length, "sha256": actual.sha256},
            "evidence": "runtime re-read the opaque artifact handle and computed its byte length and SHA-256",
            "scope": "local_artifact_integrity_only",
            "app_acceptance_or_persistence_verified": false
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
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
        Ok(rmcp::handler::server::wrapper::Json(
            run_file_select(s, &self.principal, args)
                .await
                .map_err(invalid)?,
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
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
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
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
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
            "discover_shared_tabs" => {
                let snapshot = crate::native_setup::read_tabs().map_err(invalid)?;
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "transport":"native_extension",
                    "snapshot":snapshot,
                    "next":"Pass this snapshot's host_id and exact numeric tab IDs to pair_shared. Listing does not attach to tabs."
                })))
            }
            "pair_shared" => {
                let target_ids = args.target_ids.ok_or_else(|| invalid(
                    "target_ids must explicitly select decimal Chrome tab IDs from discover_shared_tabs",
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
                        self.principal.to_string(),
                    )
                    .map_err(|e| invalid(format!("shared session denied: {e:?}")))?;
                let mut provider = controlla_browser::providers::SharedExtensionProvider::bind(
                    &mut registry,
                    &handle,
                )
                .await
                .map_err(|e| invalid(e.to_string()))?;
                let pairing = provider.pairing().clone();
                let session_id = handle.id.clone();
                let mut shared_sessions = self.shared_sessions.lock().await;
                let host_registered = crate::native_setup::host_registered();
                let unrestricted = crate::guard_reason::unrestricted_mode();
                if args.host_id.is_some() && !host_registered {
                    return Err(invalid(
                        "the native messaging host is not registered; register it before pairing",
                    ));
                }
                let native_snapshot = if host_registered {
                    if args.host_id.is_some() || unrestricted {
                        Some(crate::native_setup::read_tabs().map_err(invalid)?)
                    } else {
                        crate::native_setup::read_tabs().ok()
                    }
                } else {
                    None
                };
                if !unrestricted && let Some(requested_host) = args.host_id.as_deref() {
                    let current = native_snapshot.as_ref().ok_or_else(|| {
                        invalid(
                            "the selected native host has no fresh tab inventory; rediscover tabs",
                        )
                    })?;
                    if current["host_id"].as_str() != Some(requested_host) {
                        return Err(invalid(
                            "native host changed since discovery; rediscover tabs before pairing",
                        ));
                    }
                }
                let native_bridge =
                    native_snapshot.is_some() && (args.host_id.is_some() || unrestricted);
                let mut native_pairing = None;
                if let Some(snapshot) = native_snapshot.filter(|_| native_bridge) {
                    let tabs = snapshot["tabs"]
                        .as_array()
                        .ok_or_else(|| invalid("native tab inventory is invalid"))?;
                    if target_ids.iter().any(|id| {
                        !tabs.iter().any(|tab| {
                            tab["id"]
                                .as_u64()
                                .is_some_and(|tab_id| tab_id.to_string() == *id)
                        })
                    }) {
                        return Err(invalid(
                            "target_ids must be present in the fresh native tab inventory",
                        ));
                    }
                    let host_id = snapshot["host_id"]
                        .as_str()
                        .ok_or_else(|| invalid("native tab inventory has no host ID"))?;
                    // Bind the pairing to the page identity observed at
                    // discovery: the extension rechecks each tab's URL just
                    // before attach and detaches on later navigation.
                    let mut expected_urls = serde_json::Map::new();
                    let mut expected_document_ids = serde_json::Map::new();
                    for selected_id in &target_ids {
                        let mut matches = tabs.iter().filter(|tab| {
                            tab["id"]
                                .as_u64()
                                .is_some_and(|id| id.to_string() == *selected_id)
                        });
                        let tab = matches.next();
                        if !unrestricted {
                            let tab = tab.ok_or_else(|| {
                                invalid("selected tab is missing from the fresh native inventory")
                            })?;
                            if matches.next().is_some() {
                                return Err(invalid(
                                    "native inventory contains duplicate selected tab IDs",
                                ));
                            }
                            let url = tab["url"].as_str().ok_or_else(|| {
                                invalid("selected native tab has no discovery URL")
                            })?;
                            let document_id = tab["document_id"].as_str().filter(|id| !id.is_empty()).ok_or_else(|| invalid("selected native tab has no durable document ID; reload the extension and rediscover tabs"))?;
                            expected_urls.insert(selected_id.clone(), json!(url));
                            expected_document_ids.insert(selected_id.clone(), json!(document_id));
                        } else if let Some(tab) = tab {
                            // Preserve discovery receipts for extensions predating unrestricted pairing.
                            if let (Some(url), Some(document_id)) = (
                                tab["url"].as_str(),
                                tab["document_id"].as_str().filter(|id| !id.is_empty()),
                            ) {
                                expected_urls.insert(selected_id.clone(), json!(url));
                                expected_document_ids
                                    .insert(selected_id.clone(), json!(document_id));
                            }
                        }
                    }
                    let native_request_id =
                        crate::native_setup::new_pairing_request_id().map_err(invalid)?;
                    crate::native_setup::write_pairing(
                        host_id,
                        &pairing.endpoint,
                        &pairing.token,
                        &target_ids,
                        &native_request_id,
                        &expected_urls,
                        &expected_document_ids,
                        unrestricted,
                    )
                    .map_err(invalid)?;
                    native_pairing = Some((host_id.to_owned(), native_request_id));
                }
                shared_sessions.insert(
                    session_id.clone(),
                    Arc::new(Mutex::new(SharedLiveSession {
                        snapshots: BTreeMap::new(),
                        pairing: Some(tokio::spawn(async move {
                            let connection = provider
                                .accept_with_timeout(&mut registry, Duration::from_secs(300))
                                .await?;
                            Ok((registry, connection))
                        })),
                        connection: None,
                        registry: None,
                        handle,
                        native_pairing,
                    })),
                );
                drop(shared_sessions);
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "session_id":session_id, "target_ids":target_ids,
                    "endpoint":pairing.endpoint,
                    "one_session_token":pairing.token,
                    "native_bridge":native_bridge,
                    "pairing_expires_in_seconds":300,
                    "next":if native_bridge { "The native bridge is pairing the requested tab IDs. Call accept_shared to confirm; it returns accepted:false while waiting." } else { "Native pairing is unavailable; use the popup's manual endpoint and token, then call accept_shared." },
                    "identity":"Chrome tab IDs are not Direct CDP target references"
                })))
            }
            "accept_shared" | "accept_shared_wait" => {
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
                if shared.handle.principal != self.principal.as_ref() {
                    return Err(invalid("session is not owned by this server principal"));
                }
                if shared.connection.is_none() {
                    let task = shared.pairing.as_mut().ok_or_else(|| {
                        invalid("pairing ended; create a fresh pair_shared session")
                    })?;
                    let wait = if args.action == "accept_shared_wait" {
                        Duration::from_secs(15)
                    } else {
                        Duration::from_millis(100)
                    };
                    let outcome = match tokio::time::timeout(wait, task).await {
                        Ok(outcome) => outcome,
                        Err(_) => {
                            return Ok(rmcp::handler::server::wrapper::Json(json!({
                                "session_id":id, "accepted":false, "status":"waiting_for_extension",
                                "next":"Complete the native bridge or popup fallback shown by pair_shared, then call accept_shared again."
                            })));
                        }
                    };
                    shared.pairing = None;
                    let (registry, connection) = match outcome {
                        Ok(Ok(ready)) => ready,
                        Ok(Err(error)) => {
                            drop(shared);
                            self.shared_sessions.lock().await.remove(id);
                            let message = error.to_string();
                            let next = if message.contains("already leased")
                                || message.contains("already paired")
                                || message.contains("occupied")
                            {
                                "Release the session that owns the tab, then rediscover and pair its current document identity. Do not retry this lease blindly."
                            } else {
                                "Follow the pairing error, then rediscover the tab before creating a new pair."
                            };
                            return Err(invalid(format!("{message} Next action: {next}")));
                        }
                        Err(error) => {
                            drop(shared);
                            self.shared_sessions.lock().await.remove(id);
                            let message = error.to_string();
                            return Err(invalid(format!(
                                "{message} Next action: rediscover the tab and create a fresh pair_shared request."
                            )));
                        }
                    };
                    shared.registry = Some(registry);
                    shared.connection = Some(PageSessionTransport::Shared(connection));
                }
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "session_id":id, "accepted":true,
                    "target_ids":shared.connection.as_ref().unwrap().selected_targets(),
                    "next":"Use list_shared_targets and the guarded V3 snapshot/action surface. App save/persistence is not established by browser control."
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
                if shared.handle.principal != self.principal.as_ref() {
                    return Err(invalid("session is not owned by this server principal"));
                }
                let connection = shared
                    .connection
                    .as_ref()
                    .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
                let mut targets = Vec::new();
                for target_id in connection.selected_targets() {
                    let frame_tree = connection
                        .command(
                            shared.registry()?,
                            &shared.handle,
                            &target_id,
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
                if shared.handle.principal != self.principal.as_ref() {
                    return Err(invalid("session is not owned by this server principal"));
                }
                let dedicated = shared.connection.as_ref().is_some_and(|connection| {
                    matches!(connection, PageSessionTransport::Dedicated(_))
                });
                let handle = shared.handle.clone();
                let pairing_cleanup_error =
                    shared
                        .native_pairing
                        .take()
                        .and_then(|(host_id, request_id)| {
                            crate::native_setup::remove_pairing_request(&host_id, &request_id).err()
                        });
                let release_error = {
                    let SharedLiveSession {
                        connection,
                        registry,
                        ..
                    } = &mut *shared;
                    if let Some(connection) = connection.as_mut() {
                        connection
                            .release(
                                registry
                                    .as_mut()
                                    .ok_or_else(|| invalid("session registry is unavailable"))?,
                                &handle,
                            )
                            .await
                            .err()
                    } else {
                        None
                    }
                };
                if shared.connection.as_ref().is_some_and(|connection| {
                    matches!(connection, PageSessionTransport::Dedicated(Some(_)))
                }) && release_error.is_some()
                {
                    return Err(invalid(format!(
                        "dedicated Chrome session retained for cleanup retry: {}",
                        release_error.unwrap_or_default()
                    )));
                }
                shared.connection = None;
                if let Some(task) = shared.pairing.take() {
                    task.abort();
                    let _ = task.await;
                }
                drop(shared);
                self.shared_sessions.lock().await.remove(id);
                if let Some(error) = pairing_cleanup_error {
                    return Err(invalid(format!(
                        "shared session released but pending pairing cleanup failed: {error}"
                    )));
                }
                if let Some(error) = release_error {
                    return Err(invalid(format!(
                        "shared session retired after close failed: {error}"
                    )));
                }
                Ok(rmcp::handler::server::wrapper::Json(json!({
                    "session_id":id,"released":true,"effect":if dedicated { "closed the owned dedicated Chrome target and removed its private profile" } else { "released this provider's debugger attachments" }
                })))
            }
            "discover" => {
                let mut providers = list_sessions();
                if let Some(extension) = providers
                    .iter_mut()
                    .find(|p| p.provider == SessionProvider::CompanionExtension)
                {
                    extension.available = crate::native_setup::host_registered()
                        && crate::native_setup::read_tabs().is_ok();
                    if extension.available {
                        extension.reason = "native host and fresh extension tab inventory observed; select exact tab IDs with discover_shared_tabs".to_owned();
                    }
                }
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
                        self.principal.to_string(),
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
                        artifact_expectations: Mutex::new(BTreeMap::new()),
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
                if !self.owns_session(&s) {
                    return Err(invalid("session is not owned by this server principal"));
                }
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
                "action must be discover_shared_tabs, pair_shared, accept_shared, list_shared_targets, release_shared, discover, targets, connect, or list_targets",
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
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
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
        name = "shared_snapshot",
        description = "Read a compact visible-page snapshot with actionable references, names, raw values, observed link URLs, readiness, coverage, and bounded frame_handles. A frame handle identifies a frame in this selected tab and includes its loader and parent; use its frame_id with shared_accessibility to read that frame. Shared extension handles do not expose debugger session IDs or measured transforms, so those fields are explicitly unavailable. Default scope is body. References retain DOM identity and expire on the next snapshot, document change or click. Use shared_click with a returned reference; never invent selectors or URLs. Text is an excerpt, not proof that all tasks were listed."
    )]
    async fn shared_snapshot(
        &self,
        Parameters(args): Parameters<SharedSnapshotArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let shared = self
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let mut shared = shared.lock().await;
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            capture_shared_snapshot(&mut shared, &args),
        )
        .await
        .map_err(|_| invalid("snapshot timed out; no action dispatched"))?
        .map_err(invalid)?;
        Ok(rmcp::handler::server::wrapper::Json(result))
    }

    #[tool(
        name = "shared_scroll",
        description = "Apply one bounded trusted wheel event at the center of exactly one fresh shared_snapshot reference, or set scroll_to_target=true to scroll a fresh root-document semantic target into view and verify its center hit-test. Scroll-to-target refuses child frames and unknown delivery is never retried."
    )]
    async fn shared_scroll(
        &self,
        Parameters(args): Parameters<SharedScrollArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let scroll_to_target = args.scroll_to_target == Some(true);
        if scroll_to_target {
            if args.delta_x != 0 || args.delta_y != 0 {
                return Err(invalid("scroll_to_target requires zero wheel deltas"));
            }
            if args.reference.starts_with("@f:") {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"not_dispatched","action":"scroll_to","reason_code":"frame_scroll_to_unsupported","dispatch_may_have_occurred":false,"reason":"scroll_to_target supports root-document semantic references only."}),
                ));
            }
        } else {
            validate_wheel_delta(args.delta_x, args.delta_y).map_err(invalid)?;
        }
        let timeout = args.timeout_ms.unwrap_or(3000);
        if !(1000..=10000).contains(&timeout) {
            return Err(invalid("timeout_ms must be 1000..10000"));
        }
        let shared = self
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let mut shared = shared.lock().await;
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let snapshot = shared
            .snapshots
            .get(&args.chrome_tab_id)
            .cloned()
            .ok_or_else(|| invalid("take shared_snapshot first"))?;
        let frame_reference = if let Some(value) = args.reference.strip_prefix("@f:") {
            let parts = value.splitn(3, ':').collect::<Vec<_>>();
            if parts.len() != 3
                || parts[0].is_empty()
                || parts[0].len() > 256
                || parts[1].is_empty()
                || parts[1].len() > 256
            {
                return Err(invalid(
                    "frame scroll requires @f:<frame>:<loader>:<backend-node>",
                ));
            }
            let backend = parts[2]
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or_else(|| {
                    invalid("frame scroll reference has an invalid backend node identity")
                })?;
            Some((parts[0].to_owned(), parts[1].to_owned(), backend))
        } else {
            None
        };
        let root_index = if frame_reference.is_none() {
            let (token, index) = args.reference.rsplit_once(':').ok_or_else(|| {
                invalid("invalid reference; use the returned reference unchanged")
            })?;
            let index = index
                .parse::<usize>()
                .map_err(|_| invalid("invalid reference index"))?;
            if index >= snapshot.count
                || (!crate::guard_reason::unrestricted_mode()
                    && (token != snapshot.token || snapshot.consumed))
            {
                return Err(invalid(
                    "stale or consumed reference; take one fresh snapshot before acting",
                ));
            }
            Some(index)
        } else {
            if snapshot.consumed {
                return Err(invalid(
                    "stale or consumed frame reference; take one fresh shared snapshot before acting",
                ));
            }
            None
        };
        let unrestricted = crate::guard_reason::unrestricted_mode();
        if !unrestricted {
            shared
                .snapshots
                .get_mut(&args.chrome_tab_id)
                .unwrap()
                .consumed = true;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout);
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("session not accepted"))?;
        let registry = shared.registry()?;
        let fresh = tokio::time::timeout_at(
            deadline,
            shared_identity_and_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            ),
        )
        .await
        .map_err(|_| invalid("scroll deadline exceeded before dispatch"))?
        .map_err(invalid)?;
        let read = SharedOperationRead::new(fresh.0, fresh.1);
        let identity = &read.identity;
        let interaction = &read.interaction_epoch;
        if !read.remains_fresh(&snapshot.identity, &snapshot.interaction_epoch)
            || (!unrestricted
                && !shared_epoch_matches(
                    &snapshot,
                    &args.chrome_tab_id,
                    identity,
                    interaction,
                    crate::v3::EpochUse::GuardedClick,
                ))
        {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":if scroll_to_target{"scroll_to"}else{"scroll"},"reason_code":"stale_document","dispatch_may_have_occurred":false,"reason":"The target document or viewport changed; take a fresh snapshot."}),
            ));
        }
        if scroll_to_target {
            let index = root_index.expect("scroll-to-target rejects frame references");
            let guard = shared_scroll_to_target_guard_script();
            let response = tokio::time::timeout_at(
                deadline,
                connection.command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Controlla.scrollToTarget",
                    json!({
                        "object_id": snapshot.object_id, "index": index, "guard_function": guard,
                        "scroll_request": "scroll-to-target-v1", "timeout_ms": timeout,
                        "expected_activation_epoch": args.expected_activation_epoch,
                        "expected_focus_epoch": args.expected_focus_epoch,
                        "expected_target_revision": args.expected_target_revision
                    }),
                ),
            )
            .await;
            let mut receipt = match response {
                Ok(Ok(response)) => shared_command_value(&response),
                _ => {
                    json!({"status":"unknown","reason_code":"scroll_to_target_delivery_unknown","dispatch_acknowledged":false,"dispatch_may_have_occurred":true,"next_action":"inspect_current_state_once_before_retrying"})
                }
            };
            let identity_after = tokio::time::timeout_at(
                deadline,
                shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id),
            )
            .await;
            if !matches!(identity_after, Ok(Ok(ref current)) if *current == snapshot.identity)
                && receipt["dispatch_may_have_occurred"] != false
            {
                receipt["status"] = json!("unknown");
                receipt["reason_code"] = json!("document_changed_after_scroll_to_target");
            }
            receipt["action"] = json!("scroll_to");
            receipt["reference"] = json!(args.reference);
            return Ok(rmcp::handler::server::wrapper::Json(receipt));
        }
        if let Some((frame_id, frame_loader_id, backend_node_id)) = frame_reference {
            let tree = connection
                .command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Page.getFrameTree",
                    Value::Null,
                )
                .await
                .map_err(|_| invalid("frame tree is unavailable before child-frame scroll"))?;
            if find_cdp_frame(&tree, &frame_id).is_none_or(|(loader, _)| loader != frame_loader_id)
            {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"not_dispatched","action":"scroll","reason_code":"stale_frame_reference","dispatch_may_have_occurred":false,"reason":"Child frame navigated since its semantic reference was created."}),
                ));
            }
            let response=connection.command(registry,&shared.handle,&args.chrome_tab_id,"Controlla.frameScroll",json!({
                "frame_id":frame_id,"loader_id":frame_loader_id,"backend_node_id":backend_node_id,
                "delta_x":args.delta_x,"delta_y":args.delta_y,"timeout_ms":timeout,
                "expected_activation_epoch":args.expected_activation_epoch,"expected_focus_epoch":args.expected_focus_epoch,
                "expected_target_revision":args.expected_target_revision
            })).await;
            let mut receipt = match response {
                Ok(response) => shared_command_value(&response),
                Err(_) => {
                    json!({"status":"unknown","reason_code":"frame_wheel_delivery_unknown","dispatch_acknowledged":false,"dispatch_may_have_occurred":true,"next_action":"inspect_current_state_once_before_retrying"})
                }
            };
            let root_after =
                shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
                    .await;
            let frame_after = connection
                .command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Page.getFrameTree",
                    Value::Null,
                )
                .await
                .ok()
                .and_then(|tree| find_cdp_frame(&tree, &frame_id));
            if (!matches!(root_after,Ok(ref current) if *current==snapshot.identity)
                || frame_after.is_none_or(|(loader, _)| loader != frame_loader_id))
                && receipt["dispatch_may_have_occurred"] != false
            {
                receipt["status"] = json!("unknown");
                receipt["reason_code"] = json!("frame_identity_changed_after_scroll");
            }
            receipt["action"] = json!("scroll");
            receipt["reference"] = json!(args.reference);
            receipt["requested_delta_x"] = json!(args.delta_x);
            receipt["requested_delta_y"] = json!(args.delta_y);
            return Ok(rmcp::handler::server::wrapper::Json(receipt));
        }
        let index = root_index.expect("root scroll has a parsed item index");
        let guard = shared_scroll_guard_script();
        let preflight=tokio::time::timeout_at(deadline,connection.command(registry,&shared.handle,&args.chrome_tab_id,"Runtime.callFunctionOn",json!({
            "objectId":snapshot.object_id,"functionDeclaration":guard,
            "arguments":[{"value":index},{"value":args.delta_x},{"value":args.delta_y},{"value":Value::Null}],"returnByValue":true
        }))).await;
        let checked = match preflight {
            Ok(Ok(response)) => shared_value(&response).unwrap_or(Value::Null),
            _ => {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"not_dispatched","action":"scroll","reason_code":"scroll_preflight_failed","dispatch_may_have_occurred":false}),
                ));
            }
        };
        if checked["ok"] != true {
            let mut refusal = crate::guard_reason::refusal(&checked);
            refusal["action"] = json!("scroll");
            return Ok(rmcp::handler::server::wrapper::Json(refusal));
        }
        let Some(viewport) = checked["viewport"].as_str() else {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"scroll","reason_code":"invalid_scroll_guard","dispatch_may_have_occurred":false}),
            ));
        };
        let fresh = tokio::time::timeout_at(
            deadline,
            shared_identity_and_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            ),
        )
        .await;
        if !matches!(fresh,Ok(Ok((ref id,ref epoch))) if shared_epoch_matches(&snapshot,&args.chrome_tab_id,id,epoch,crate::v3::EpochUse::GuardedClick))
        {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"scroll","reason_code":"changed_target","dispatch_may_have_occurred":false,"reason":"The document or viewport changed before wheel dispatch."}),
            ));
        }
        let dispatch=tokio::time::timeout_at(deadline,connection.command(registry,&shared.handle,&args.chrome_tab_id,"Controlla.pointerScroll",json!({
            "object_id":snapshot.object_id,"index":index,"delta_x":args.delta_x,"delta_y":args.delta_y,
            "guard_function":guard,"viewport":viewport,"timeout_ms":timeout
            ,"expected_activation_epoch":args.expected_activation_epoch,"expected_focus_epoch":args.expected_focus_epoch,
            "expected_target_revision":args.expected_target_revision
        }))).await;
        let receipt = match dispatch {
            Ok(Ok(value)) => shared_command_value(&value),
            _ => {
                json!({"status":"unknown","reason_code":"wheel_delivery_unknown","dispatch_acknowledged":false,"dispatch_may_have_occurred":true,"next_action":"inspect_current_state_once_before_retrying"})
            }
        };
        if receipt["status"] == "verified"
            && !valid_scroll_readback(&receipt, args.delta_x, args.delta_y)
        {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"inconclusive","action":"scroll","reason_code":"invalid_scroll_readback","dispatch_acknowledged":true,"dispatch_may_have_occurred":true,"receipt":receipt}),
            ));
        }
        let identity_after = tokio::time::timeout_at(
            deadline,
            shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id),
        )
        .await;
        let mut result = receipt;
        if !matches!(identity_after,Ok(Ok(ref value)) if *value==snapshot.identity)
            && result["dispatch_may_have_occurred"] != false
        {
            result["status"] = json!("unknown");
            result["reason_code"] = json!("document_changed_after_scroll");
            result["reason"] = json!(
                "The target document changed after wheel dispatch; inspect state before retrying."
            );
        }
        result["action"] = json!("scroll");
        result["reference"] = json!(args.reference);
        result["requested_delta_x"] = json!(args.delta_x);
        result["requested_delta_y"] = json!(args.delta_y);
        Ok(rmcp::handler::server::wrapper::Json(result))
    }

    #[allow(dead_code)] // Used by the V3 CLI binary.
    async fn shared_swipe(
        &self,
        args: SharedSwipeArgs,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        validate_swipe_endpoints(args.start_x, args.start_y, args.end_x, args.end_y)
            .map_err(invalid)?;
        let timeout = args.timeout_ms.unwrap_or(3000);
        if !(1000..=10000).contains(&timeout) {
            return Err(invalid("timeout_ms must be 1000..10000"));
        }
        let shared = self
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let mut shared = shared.lock().await;
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let snapshot = shared
            .snapshots
            .get(&args.chrome_tab_id)
            .cloned()
            .ok_or_else(|| invalid("take shared_snapshot first"))?;
        let (token, index) = args
            .reference
            .rsplit_once(':')
            .ok_or_else(|| invalid("invalid reference; use the returned reference unchanged"))?;
        let index = index
            .parse::<usize>()
            .map_err(|_| invalid("invalid reference index"))?;
        let unrestricted = crate::guard_reason::unrestricted_mode();
        if index >= snapshot.count
            || (!unrestricted && (token != snapshot.token || snapshot.consumed))
        {
            return Err(invalid(
                "stale or consumed reference; take one fresh snapshot before acting",
            ));
        }
        let timeout = Duration::from_millis(timeout);
        let deadline = tokio::time::Instant::now() + timeout;
        // Consume before any potentially yielding browser call. A refused gesture
        // still requires a fresh snapshot, while an uncertain delivery can never
        // be replayed with the same semantic reference.
        if !unrestricted {
            shared
                .snapshots
                .get_mut(&args.chrome_tab_id)
                .unwrap()
                .consumed = true;
        }
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("session not accepted"))?;
        let registry = shared.registry()?;
        let fresh = tokio::time::timeout_at(
            deadline,
            shared_identity_and_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            ),
        )
        .await
        .map_err(|_| invalid("swipe deadline exceeded before dispatch"))?
        .map_err(invalid)?;
        let read = SharedOperationRead::new(fresh.0, fresh.1);
        let identity = &read.identity;
        let interaction = &read.interaction_epoch;
        if !read.remains_fresh(&snapshot.identity, &snapshot.interaction_epoch)
            || (!unrestricted
                && !shared_epoch_matches(
                    &snapshot,
                    &args.chrome_tab_id,
                    identity,
                    interaction,
                    crate::v3::EpochUse::GuardedClick,
                ))
        {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"swipe","reason_code":"stale_document","dispatch_may_have_occurred":false}),
            ));
        }
        let guard = shared_swipe_guard_script();
        let preflight=tokio::time::timeout_at(deadline,connection.command(registry,&shared.handle,&args.chrome_tab_id,"Runtime.callFunctionOn",json!({"objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":args.start_x},{"value":args.start_y},{"value":args.end_x},{"value":args.end_y},{"value":Value::Null},{"value":"before"}],"returnByValue":true}))).await;
        let checked = match preflight {
            Ok(Ok(response)) => shared_value(&response).unwrap_or(Value::Null),
            _ => {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"not_dispatched","action":"swipe","reason_code":"swipe_preflight_failed","dispatch_may_have_occurred":false}),
                ));
            }
        };
        if checked["ok"] != true {
            let mut refusal = crate::guard_reason::refusal(&checked);
            refusal["action"] = json!("swipe");
            return Ok(rmcp::handler::server::wrapper::Json(refusal));
        }
        let Some(viewport) = checked["viewport"].as_str() else {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"swipe","reason_code":"invalid_swipe_guard","dispatch_may_have_occurred":false}),
            ));
        };
        let fresh = tokio::time::timeout_at(deadline, async {
            let id =
                shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
                    .await?;
            let epoch = shared_interaction_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
            )
            .await?;
            Ok::<_, String>((id, epoch))
        })
        .await;
        if !matches!(fresh,Ok(Ok((ref id,ref epoch))) if shared_epoch_matches(&snapshot,&args.chrome_tab_id,id,epoch,crate::v3::EpochUse::GuardedClick))
        {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"swipe","reason_code":"changed_target","dispatch_may_have_occurred":false}),
            ));
        }
        let dispatch=tokio::time::timeout_at(deadline,connection.command(registry,&shared.handle,&args.chrome_tab_id,"Controlla.pointerSwipe",json!({
            "object_id":snapshot.object_id,"index":index,"guard_function":guard,"viewport":viewport,
            "start_x":args.start_x,"start_y":args.start_y,"end_x":args.end_x,"end_y":args.end_y,"timeout_ms":timeout.as_millis(),
            "expected_activation_epoch":args.expected_activation_epoch,"expected_focus_epoch":args.expected_focus_epoch,
            "expected_target_revision":args.expected_target_revision
        }))).await;
        let mut receipt = match dispatch {
            Ok(Ok(value)) => shared_command_value(&value),
            _ => {
                json!({"status":"unknown","reason_code":"touch_delivery_unknown","dispatch_acknowledged":false,"dispatch_may_have_occurred":true,"next_action":"inspect_current_state_once_before_retrying"})
            }
        };
        let identity_after = tokio::time::timeout_at(
            deadline,
            shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id),
        )
        .await;
        if !matches!(identity_after,Ok(Ok(ref value)) if *value==snapshot.identity)
            && receipt["dispatch_may_have_occurred"] != false
        {
            receipt["status"] = json!("unknown");
            receipt["reason_code"] = json!("document_changed_after_swipe");
        }
        receipt["action"] = json!("swipe");
        receipt["reference"] = json!(args.reference);
        Ok(rmcp::handler::server::wrapper::Json(receipt))
    }

    #[allow(dead_code)]
    async fn shared_multi_touch(
        &self,
        args: SharedPinchArgs,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        validate_pinch(
            args.center_x,
            args.center_y,
            args.start_distance,
            args.end_distance,
        )
        .map_err(invalid)?;
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(3000).clamp(1000, 10000));
        let deadline = tokio::time::Instant::now() + timeout;
        let shared = self
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let mut shared = shared.lock().await;
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let snapshot = shared
            .snapshots
            .get(&args.chrome_tab_id)
            .cloned()
            .ok_or_else(|| invalid("take shared_snapshot first"))?;
        let (token, index) = args
            .reference
            .rsplit_once(':')
            .ok_or_else(|| invalid("invalid reference; use the returned reference unchanged"))?;
        let index = index
            .parse::<usize>()
            .map_err(|_| invalid("invalid reference index"))?;
        let unrestricted = crate::guard_reason::unrestricted_mode();
        if index >= snapshot.count
            || (!unrestricted && (token != snapshot.token || snapshot.consumed))
        {
            return Err(invalid(
                "stale or consumed reference; take one fresh snapshot before acting",
            ));
        }
        if !unrestricted {
            shared
                .snapshots
                .get_mut(&args.chrome_tab_id)
                .unwrap()
                .consumed = true;
        }
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("session not accepted"))?;
        let registry = shared.registry()?;
        let fresh = tokio::time::timeout_at(
            deadline,
            shared_identity_and_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            ),
        )
        .await
        .map_err(|_| invalid("multi_touch deadline exceeded before dispatch"))?
        .map_err(invalid)?;
        let read = SharedOperationRead::new(fresh.0, fresh.1);
        let identity = &read.identity;
        let interaction = &read.interaction_epoch;
        if !read.remains_fresh(&snapshot.identity, &snapshot.interaction_epoch)
            || (!unrestricted
                && !shared_epoch_matches(
                    &snapshot,
                    &args.chrome_tab_id,
                    identity,
                    interaction,
                    crate::v3::EpochUse::GuardedClick,
                ))
        {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"multi_touch","reason_code":"stale_document","dispatch_may_have_occurred":false}),
            ));
        }
        let guard = shared_pinch_guard_script();
        let preflight=tokio::time::timeout_at(deadline,connection.command(registry,&shared.handle,&args.chrome_tab_id,"Runtime.callFunctionOn",json!({"objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":args.center_x},{"value":args.center_y},{"value":args.start_distance},{"value":args.end_distance},{"value":Value::Null}],"returnByValue":true}))).await;
        let checked = match preflight {
            Ok(Ok(response)) => shared_value(&response).unwrap_or(Value::Null),
            _ => {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"not_dispatched","action":"multi_touch","reason_code":"pinch_preflight_failed","dispatch_may_have_occurred":false}),
                ));
            }
        };
        if checked["ok"] != true {
            let mut refusal = crate::guard_reason::refusal(&checked);
            refusal["action"] = json!("multi_touch");
            return Ok(rmcp::handler::server::wrapper::Json(refusal));
        }
        let Some(viewport) = checked["viewport"].as_str() else {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"multi_touch","reason_code":"invalid_pinch_guard","dispatch_may_have_occurred":false}),
            ));
        };
        let fresh = tokio::time::timeout_at(deadline, async {
            let id =
                shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
                    .await?;
            let epoch = shared_interaction_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
            )
            .await?;
            Ok::<_, String>((id, epoch))
        })
        .await;
        if !matches!(fresh,Ok(Ok((ref id,ref epoch))) if shared_epoch_matches(&snapshot,&args.chrome_tab_id,id,epoch,crate::v3::EpochUse::GuardedClick))
        {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","action":"multi_touch","reason_code":"changed_target","dispatch_may_have_occurred":false}),
            ));
        }
        let dispatch_started_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();
        let mut receipt=match tokio::time::timeout_at(deadline,connection.command(registry,&shared.handle,&args.chrome_tab_id,"Controlla.pointerMultiTouch",json!({"object_id":snapshot.object_id,"index":index,"guard_function":guard,"viewport":viewport,"center_x":args.center_x,"center_y":args.center_y,"start_distance":args.start_distance,"end_distance":args.end_distance,"timeout_ms":timeout.as_millis(),"expected_activation_epoch":args.expected_activation_epoch,"expected_focus_epoch":args.expected_focus_epoch,"expected_target_revision":args.expected_target_revision}))).await {Ok(Ok(value))=>shared_command_value(&value),_=>json!({"status":"unknown","reason_code":"touch_delivery_unknown","dispatch_acknowledged":false,"dispatch_may_have_occurred":true,"next_action":"inspect_current_state_once_before_retrying"})};
        let acknowledged = matches!(receipt["status"].as_str(), Some("dispatched" | "verified"));
        if acknowledged {
            let mut poll = Duration::from_millis(20);
            while tokio::time::Instant::now() < deadline {
                let identity = match tokio::time::timeout_at(
                    deadline,
                    shared_frame_identity(
                        connection,
                        registry,
                        &shared.handle,
                        &args.chrome_tab_id,
                    ),
                )
                .await
                .ok()
                .and_then(Result::ok)
                {
                    Some(value) => value,
                    None => break,
                };
                if !shared_outcome_identity_allows_verification(
                    &args.outcome,
                    &snapshot.identity,
                    &identity,
                ) {
                    receipt["status"] = json!("unknown");
                    receipt["reason_code"] = json!("document_replaced_during_outcome");
                    receipt["dispatch_may_have_occurred"] = json!(true);
                    break;
                }
                let observed = tokio::time::timeout_at(
                    deadline,
                    evaluate_shared_outcome(
                        &shared,
                        &args.chrome_tab_id,
                        &snapshot,
                        index,
                        &args.outcome,
                        &identity,
                        dispatch_started_ms,
                    ),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false);
                let after = tokio::time::timeout_at(
                    deadline,
                    shared_frame_identity(
                        connection,
                        registry,
                        &shared.handle,
                        &args.chrome_tab_id,
                    ),
                )
                .await
                .ok()
                .and_then(Result::ok);
                if observed && matches!(after, Some(ref value) if *value == identity) {
                    receipt["status"] = json!("verified");
                    receipt["verified"] = json!(true);
                    receipt["outcome"] = serde_json::to_value(&args.outcome).unwrap_or(Value::Null);
                    break;
                }
                tokio::time::sleep(poll).await;
                poll = (poll * 2).min(Duration::from_millis(200));
            }
            if receipt["status"] != "verified" {
                receipt["status"] = json!("unknown");
                receipt["reason_code"] = json!("outcome_verification_timeout");
                receipt["dispatch_may_have_occurred"] = json!(true);
            }
        }
        receipt["action"] = json!("multi_touch");
        receipt["reference"] = json!(args.reference);
        Ok(rmcp::handler::server::wrapper::Json(receipt))
    }

    #[tool(
        name = "shared_click",
        description = "Apply one guarded pointer tap to exactly one shared_snapshot reference. input_mode defaults to mouse; touch uses CDP touchStart/touchEnd and never falls back to mouse. Mouse input requires the target tab to already be active; inactive user-owned tabs return not_dispatched and are never activated. Trusted background input is supported only for an active tab inside an explicitly Controlla-owned unfocused automation window. Declare a bounded outcome: navigation, visible/text selector, expanded/selected, or focused. Returns verified, not_dispatched, or unknown plus a fresh snapshot where available. Unknown is never automatically retried; inspect the returned state before deciding. A successful UI outcome does not prove save/persistence."
    )]
    async fn shared_click(
        &self,
        Parameters(args): Parameters<SharedClickArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        validate_shared_click_outcome(&args.outcome)?;
        let input_mode = args.input_mode.as_deref().unwrap_or("mouse");
        let click_kind = args.click_kind.as_deref().unwrap_or("click");
        if !matches!(input_mode, "mouse" | "touch") {
            return Err(invalid("input_mode must be mouse or touch"));
        }
        if !matches!(click_kind, "click" | "double_click" | "context_click") {
            return Err(invalid(
                "click_kind must be click, double_click, or context_click",
            ));
        }
        if input_mode == "touch" && click_kind != "click" {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"not_dispatched","reason_code":"unsupported_pointer_variant","dispatch_may_have_occurred":false,"reason":"double and context click are mouse-only; no touch input was dispatched"}),
            ));
        }
        let outcome = json!(args.outcome);
        if outcome
            .get("selector")
            .and_then(Value::as_str)
            .is_some_and(|s| s.is_empty() || s.len() > 512 || s.chars().any(char::is_control))
            || outcome
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|s| s.len() > 16384)
            || outcome
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|s| s.len() > 2048)
        {
            return Err(invalid("outcome selector/text/URL exceeds bounds"));
        }
        let timeout = args.timeout_ms.unwrap_or(3000);
        if !(1000..=10000).contains(&timeout) {
            return Err(invalid("timeout_ms must be 1000..10000"));
        }
        let shared = self
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let mut shared = shared.lock().await;
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let snapshot = shared
            .snapshots
            .get(&args.chrome_tab_id)
            .cloned()
            .ok_or_else(|| invalid("take shared_snapshot first"))?;
        let (token, index) = args
            .reference
            .rsplit_once(':')
            .ok_or_else(|| invalid("invalid reference; use the returned reference unchanged"))?;
        let index = index
            .parse::<usize>()
            .map_err(|_| invalid("invalid reference index"))?;
        let unrestricted = crate::guard_reason::unrestricted_mode();
        if index >= snapshot.count
            || (!unrestricted && (token != snapshot.token || snapshot.consumed))
        {
            return Err(invalid(
                "stale or consumed reference; take one fresh snapshot before acting",
            ));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout);
        let prepare = async {
            let connection = shared.connection.as_ref().ok_or("session not accepted")?;
            let registry = shared.registry().map_err(|e| e.to_string())?;
            let identity =
                shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
                    .await?;
            let interaction = shared_interaction_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
            )
            .await?;
            if !unrestricted
                && !shared_epoch_matches(
                    &snapshot,
                    &args.chrome_tab_id,
                    &identity,
                    &interaction,
                    crate::v3::EpochUse::GuardedClick,
                )
            {
                return Err(ClickRefusal::Guard(crate::guard_reason::refusal(
                    &json!({"ok":false,"reason":"document_changed"}),
                )));
            }
            let guard = crate::guard_reason::click_guard_script(SHARED_CLICK_UNSAFE_PREDICATE);
            let response=connection.command(registry,&shared.handle,&args.chrome_tab_id,"Runtime.callFunctionOn",json!({"objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":outcome}],"returnByValue":true})).await.map_err(|e|e.to_string())?;
            let value = shared_value(&response)?;
            if value["ok"] != true {
                return Err(ClickRefusal::Guard(crate::guard_reason::refusal(&value)));
            }
            if !unrestricted
                && !matches!(&args.outcome, SharedClickOutcome::NewTab { .. })
                && evaluate_shared_outcome(
                    &shared,
                    &args.chrome_tab_id,
                    &snapshot,
                    index,
                    &args.outcome,
                    &identity,
                    0,
                )
                .await?
            {
                return Err("outcome already satisfied; no click needed".into());
            }
            Ok((guard, value))
        };
        let guard = match tokio::time::timeout_at(deadline, prepare).await {
            Ok(Ok(guard_and_check)) => guard_and_check,
            result => {
                let refusal = match result {
                    Ok(Err(ClickRefusal::Guard(value))) => value,
                    Ok(Err(ClickRefusal::Msg(reason))) => {
                        json!({"status":"not_dispatched","reason":reason,"dispatch_acknowledged":false})
                    }
                    _ => {
                        json!({"status":"not_dispatched","reason":"preflight timed out","dispatch_acknowledged":false})
                    }
                };
                return Ok(rmcp::handler::server::wrapper::Json(refusal));
            }
        };
        let (guard, initial_checked) = guard;
        // Consume before dispatch: even a lost reply must not permit a duplicate click with this reference.
        if !unrestricted {
            shared
                .snapshots
                .get_mut(&args.chrome_tab_id)
                .unwrap()
                .consumed = true;
        }
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("session not accepted"))?;
        let registry = shared.registry()?;
        let prepared_viewport = initial_checked["viewport"].as_str().map(str::to_owned);
        if !unrestricted {
            let mut dispatch_snapshot = snapshot.clone();
            let Some(prepared_viewport) = prepared_viewport.as_deref() else {
                return Ok(rmcp::handler::server::wrapper::Json(json!({
                    "status":"not_dispatched",
                    "reason":"click guard omitted its prepared viewport; inspect current state",
                    "reason_code":"invalid_guard_receipt",
                    "dispatch_acknowledged":false
                })));
            };
            dispatch_snapshot.interaction_epoch["viewport"] = json!(prepared_viewport);
            let fresh = tokio::time::timeout_at(
                deadline,
                shared_identity_and_epoch(
                    connection,
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    &snapshot.object_id,
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                ),
            )
            .await;
            if !matches!(fresh, Ok(Ok((ref identity, ref interaction))) if shared_epoch_matches(&dispatch_snapshot, &args.chrome_tab_id, identity, interaction, crate::v3::EpochUse::GuardedClick))
            {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"not_dispatched","reason":"document changed before dispatch; take one fresh snapshot","dispatch_acknowledged":false}),
                ));
            }
        }
        // Revalidate the exact target and hit point after the last document/epoch
        // read. The browser-side guard is the final CDP read before pointer dispatch.
        let final_recheck = tokio::time::timeout_at(
            deadline,
            connection.command(
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({"objectId":snapshot.object_id,"functionDeclaration":guard,"arguments":[{"value":index},{"value":outcome}],"returnByValue":true}),
            ),
        )
        .await;
        let final_checked = match final_recheck {
            Ok(Ok(response)) => shared_value(&response).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        if !final_click_guard_matches_viewport(
            &final_checked,
            if unrestricted {
                None
            } else {
                prepared_viewport.as_deref()
            },
        ) {
            let mut refusal = if final_checked["reason"].is_string() {
                crate::guard_reason::refusal(&final_checked)
            } else {
                json!({"status":"not_dispatched","reason":"target or viewport changed during final revalidation","reason_code":"changed_target","retry_class":"refresh_snapshot","dispatch_acknowledged":false})
            };
            refusal["phase"] = json!("final_revalidation");
            return Ok(rmcp::handler::server::wrapper::Json(refusal));
        }
        let (x, y) = (
            final_checked["x"]
                .as_f64()
                .ok_or_else(|| invalid("missing x"))?,
            final_checked["y"]
                .as_f64()
                .ok_or_else(|| invalid("missing y"))?,
        );
        let click_started_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let mut pointer_activation = Value::Null;
        let mut pointer_terminal = None::<Value>;
        let (acknowledged, dispatch_failed) = if input_mode == "mouse"
            && shared.handle.mode == controlla_browser::sessions::SessionMode::Shared
        {
            let dispatch = tokio::time::timeout_at(
                deadline,
                connection.command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Controlla.pointerClick",
                    json!({"object_id":snapshot.object_id,"index":index,"outcome":outcome,
                    "function_declaration":guard,"viewport":prepared_viewport,"click_kind":click_kind,
                    "expected_target_revision":args.expected_target_revision}),
                ),
            )
            .await;
            match dispatch {
                Ok(Ok(response)) => {
                    let value = shared_command_value(&response);
                    if value["status"] == "not_dispatched" {
                        let mut value = value;
                        let next_action = match value["reason_code"].as_str() {
                            Some("activation_forbidden") => {
                                "use an active tab in a Controlla-owned unfocused automation window, or use an observed href navigation"
                            }
                            Some("final_target_revalidation_failed")
                            | Some("activation_revalidation_failed") => {
                                "take one fresh snapshot and reacquire the target reference"
                            }
                            Some("tab_topology_unstable") | Some("tab_topology_changed") => {
                                "wait for the tab move to settle, then rediscover and pair the tab"
                            }
                            _ => "take one fresh snapshot and follow the returned reason",
                        };
                        value["next_action"] = json!(next_action);
                        value["mutation_retry_policy"] = json!("correct_reason_then_reobserve");
                        return Ok(rmcp::handler::server::wrapper::Json(value));
                    }
                    pointer_activation = json!({
                        "temporary_activation":value["temporary_activation"],
                        "restored_previous_tab":value["restored_previous_tab"]
                    });
                    // An extension can report that the first half of a multi-click was
                    // delivered before it fenced the next click. Keep that receipt
                    // terminal: a postcondition such as focus may already be true after
                    // the first click and must not upgrade partial delivery to success.
                    if value["status"] == "unknown" && value["dispatch_may_have_occurred"] == true {
                        pointer_terminal = Some(value.clone());
                    }
                    (
                        value["dispatch_acknowledged"] == true,
                        value["status"] == "not_dispatched" || value["status"] == "unknown",
                    )
                }
                _ => (false, true),
            }
        } else {
            let events = if input_mode == "mouse" {
                pointer_click_events(x, y, click_kind).map_err(invalid)?
            } else {
                let (_, [press, release]) = pointer_events(x, y, input_mode).map_err(invalid)?;
                vec![press, release]
            };
            let mut acknowledged = true;
            let mut held_button = None::<(&str, u64)>;
            let mut touch_active = false;
            for event in &events {
                let press = event["type"] == "mousePressed";
                let touch_start = event["type"] == "touchStart";
                let button = event["button"].as_str().unwrap_or("left");
                let click_count = event["clickCount"].as_u64().unwrap_or(1);
                let dispatch = tokio::time::timeout_at(
                    deadline,
                    connection.command(
                        registry,
                        &shared.handle,
                        &args.chrome_tab_id,
                        if input_mode == "mouse" {
                            "Input.dispatchMouseEvent"
                        } else {
                            "Input.dispatchTouchEvent"
                        },
                        event.clone(),
                    ),
                )
                .await;
                if press {
                    held_button = Some((button, click_count));
                }
                if touch_start {
                    touch_active = true;
                }
                if dispatch.is_err() || !matches!(dispatch, Ok(Ok(_))) {
                    acknowledged = false;
                    break;
                }
                if event["type"] == "mouseReleased" || event["type"] == "touchEnd" {
                    held_button = None;
                }
                if event["type"] == "touchEnd" {
                    touch_active = false;
                }
            }
            if touch_active {
                let (_, [_, release]) = pointer_events(x, y, "touch").map_err(invalid)?;
                let cleaned = tokio::time::timeout(
                    Duration::from_secs(2),
                    connection.command(
                        registry,
                        &shared.handle,
                        &args.chrome_tab_id,
                        "Input.dispatchTouchEvent",
                        release,
                    ),
                )
                .await;
                acknowledged &= matches!(cleaned, Ok(Ok(_)));
            }
            if let Some((button, click_count)) = held_button {
                let release = json!({"type":"mouseReleased","button":button,"buttons":0,"clickCount":click_count,"x":x,"y":y});
                let cleaned = tokio::time::timeout(
                    Duration::from_secs(2),
                    connection.command(
                        registry,
                        &shared.handle,
                        &args.chrome_tab_id,
                        "Input.dispatchMouseEvent",
                        release,
                    ),
                )
                .await;
                acknowledged &= matches!(cleaned, Ok(Ok(_)));
            }
            (acknowledged, false)
        };
        let mut result = pointer_terminal.clone().unwrap_or_else(|| json!({
            "status":"unknown",
            "dispatch_acknowledged":acknowledged,
            "input_mode":input_mode,
            "reason_code":if dispatch_failed {"pointer_dispatch_unknown"} else {click_unknown_reason_code(acknowledged)},
            "mutation_retry_policy":"reconcile_read_only_first",
            "next_action":"inspect_current_state_once_before_retrying",
            "reason":"outcome not verified before deadline; inspect state, do not automatically retry"
        }));
        let mut verification_poll = SHARED_CLICK_VERIFY_INITIAL_POLL_INTERVAL;
        while pointer_terminal.is_none() && tokio::time::Instant::now() < deadline {
            let check = async {
                let identity = shared_frame_identity(
                    connection,
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                )
                .await?;
                if matches!(&args.outcome, SharedClickOutcome::Navigation { .. }) {
                    let verified = evaluate_shared_outcome(
                        &shared,
                        &args.chrome_tab_id,
                        &snapshot,
                        index,
                        &args.outcome,
                        &identity,
                        click_started_ms,
                    )
                    .await?;
                    return Ok::<_, String>((identity, verified));
                }
                let observed = evaluate_shared_outcome(
                    &shared,
                    &args.chrome_tab_id,
                    &snapshot,
                    index,
                    &args.outcome,
                    &identity,
                    click_started_ms,
                )
                .await?;
                let after = shared_frame_identity(
                    connection,
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                )
                .await?;
                let verified = observed && identity == after;
                Ok::<_, String>((identity, verified))
            };
            if let Ok(Ok((identity, true))) = tokio::time::timeout_at(deadline, check).await {
                let new_tab_candidate = if let SharedClickOutcome::NewTab { url } = &args.outcome {
                    connection
                        .command(
                            registry,
                            &shared.handle,
                            &args.chrome_tab_id,
                            "Controlla.newTabs",
                            json!({"since_ms":click_started_ms,"url":url}),
                        )
                        .await
                        .ok()
                        .and_then(|response| shared_new_tab_candidate(&response, url.as_deref()))
                } else {
                    None
                };
                result = json!({"status":"verified","dispatch_acknowledged":acknowledged,"input_mode":input_mode,"outcome":outcome,"url":identity.2,"navigation_observed":identity!=snapshot.identity,"new_tab_observed":matches!(&args.outcome, SharedClickOutcome::NewTab { .. }),"new_tab_candidate":new_tab_candidate});
                break;
            }
            let _ = tokio::time::timeout_at(deadline, tokio::time::sleep(verification_poll)).await;
            verification_poll = verification_poll
                .saturating_mul(2)
                .min(SHARED_CLICK_VERIFY_MAX_POLL_INTERVAL);
        }
        if result["status"] == "unknown" && !acknowledged {
            result["reason"] = json!(
                "mouse dispatch/release uncertain; inspect state before deciding any further action"
            );
        }
        if pointer_activation.is_object() {
            result["pointer_activation"] = pointer_activation;
        }
        let snapshot_args = SharedSnapshotArgs {
            session_id: args.session_id,
            chrome_tab_id: args.chrome_tab_id,
            selector: snapshot_scope(),
            max_items: snapshot_items(),
            max_text_chars: snapshot_chars(),
            max_bytes: snapshot_bytes(),
            href_hint: None,
            goal: None,
            max_scan_nodes: None,
        };
        match tokio::time::timeout(
            Duration::from_secs(5),
            capture_shared_snapshot(&mut shared, &snapshot_args),
        )
        .await
        {
            Ok(Ok(next)) => result["snapshot"] = next,
            Ok(Err(error)) => result["snapshot_error"] = json!(error),
            Err(_) => {
                result["snapshot_error"] = json!("snapshot timed out; outcome remains as reported")
            }
        }
        Ok(rmcp::handler::server::wrapper::Json(result))
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
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let before = shared_frame_identity(
            connection,
            shared.registry()?,
            &shared.handle,
            &args.chrome_tab_id,
        )
        .await
        .map_err(invalid)?;
        let params = observation_command(&spec).map_err(|e| invalid(e.to_string()))?;
        let response = connection
            .command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.evaluate",
                params,
            )
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let after = shared_frame_identity(
            connection,
            shared.registry()?,
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
        name = "shared_accessibility",
        description = "Read one CSS-selected node's partial AX data or a bounded full AX tree for an explicit child frame in an explicitly paired Chrome tab. Use frame_id from a shared_snapshot frame_handle. Child-frame reads use the same-target frameId path first and a bounded OOPIF child session fallback; returned items retain frame and loader identity for supported frame actions. Root and frame identity must remain unchanged."
    )]
    async fn shared_accessibility(
        &self,
        Parameters(args): Parameters<SharedAccessibilityArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        if args.selector.as_ref().is_some_and(|selector| {
            selector.trim().is_empty() || selector.len() > 512 || selector.contains('\0')
        }) || args.frame_id.as_ref().is_some_and(|frame| {
            frame.is_empty() || frame.len() > 256 || frame.chars().any(char::is_control)
        }) || args.selector.is_some() == args.frame_id.is_some()
            || !(4096..=1_000_000).contains(&args.max_bytes)
        {
            return Err(invalid(
                "provide exactly one bounded selector or frame_id and a valid accessibility byte budget",
            ));
        }
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(10_000).clamp(1_000, 60_000));
        let result = tokio::time::timeout(timeout, async {
            let shared = self
                .shared_sessions
                .lock()
                .await
                .get(&args.session_id)
                .cloned()
                .ok_or_else(|| invalid("unknown shared session_id"))?;
            let shared = shared.lock().await;
            if shared.handle.principal != self.principal.as_ref() {
                return Err(invalid("session is not owned by this server principal"));
            }
            let connection = shared
                .connection
                .as_ref()
                .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
            let registry = shared.registry()?;
            let before =
                shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
                    .await
                    .map_err(invalid)?;
            if let Some(frame_id) = args.frame_id.as_deref() {
                let before_tree = connection.command(registry, &shared.handle, &args.chrome_tab_id, "Page.getFrameTree", Value::Null).await.map_err(|error|invalid(error.to_string()))?;
                let before_frame = find_cdp_frame(&before_tree, frame_id).ok_or_else(||invalid("frame_id is detached or not part of the selected tab"))?;
                let response = match connection.command(registry, &shared.handle, &args.chrome_tab_id, "Accessibility.getFullAXTree", json!({"frameId":frame_id})).await {
                    Ok(value) => value,
                    Err(_) => connection.command(registry, &shared.handle, &args.chrome_tab_id, "Controlla.frameAccessibility", json!({"frame_id":frame_id})).await.map_err(|error|invalid(format!("frame accessibility unavailable in the attached target and OOPIF route: {error}")))?,
                };
                let ax = if response.get("nodes").is_some() {
                    response.clone()
                } else {
                    shared_value(&response).map_err(invalid)?
                };
                let raw_nodes=ax.get("nodes").and_then(Value::as_array).ok_or_else(||invalid("frame accessibility response omitted nodes"))?;
                let mut items=Vec::new();
                let mut truncated=false;
                for node in raw_nodes {
                    let Some(backend)=node.get("backendDOMNodeId").and_then(Value::as_i64) else { continue };
                    if items.len()>=200 { truncated=true; break; }
                    let role=node.pointer("/role/value").and_then(Value::as_str).unwrap_or("unknown");
                    let name=node.pointer("/name/value").and_then(Value::as_str).unwrap_or("");
                    let actionable = matches!(role, "button" | "link" | "checkbox" | "radio" | "option" | "menuitem" | "tab" | "switch");
                    items.push(json!({"reference":format!("@f:{frame_id}:{}:{backend}",before_frame.0),"source_frame_id":frame_id,"frame_loader_id":before_frame.0,"source_frame_url":before_frame.1,"role":role,"name":name,"backend_node_id":backend,"actionable":actionable}));
                }
                let after_tree=connection.command(registry,&shared.handle,&args.chrome_tab_id,"Page.getFrameTree",Value::Null).await.map_err(|error|invalid(error.to_string()))?;
                let after_frame=find_cdp_frame(&after_tree,frame_id).ok_or_else(||invalid("frame detached during accessibility read; result discarded"))?;
                if before!=shared_frame_identity(connection,registry,&shared.handle,&args.chrome_tab_id).await.map_err(invalid)? || before_frame!=after_frame { return Err(invalid("tab or frame navigated during accessibility read; result discarded as stale")); }
                let mut result=json!({"chrome_tab_id":args.chrome_tab_id,"source_frame_id":frame_id,"source_frame_url":before_frame.1,"frame_loader_id":before_frame.0,"routing":ax.get("routing").and_then(Value::as_str).unwrap_or("root_frame_id"),"items":items,"truncated":truncated,"complete":!truncated,"shared_frame_identity":{"frame_id":before.0,"loader_id":before.1,"url":before.2}});
                if serde_json::to_vec(&result).map(|bytes|bytes.len()).unwrap_or(usize::MAX)>args.max_bytes { result["items"]=json!([]);result["truncated"]=json!(true);result["complete"]=json!(false);result["missing"]=json!(["AX result exceeded the requested byte budget"]); }
                return Ok(result);
            }
            let selector_arg=args.selector.as_deref().expect("selector is required when frame_id is absent");
            let document = connection
                .command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "DOM.getDocument",
                    json!({"depth":0,"pierce":false}),
                )
                .await
                .map_err(|e| invalid(e.to_string()))?;
            let root = document
                .get("root")
                .and_then(|v| v["nodeId"].as_u64())
                .filter(|id| *id > 0)
                .ok_or_else(|| invalid("DOM.getDocument omitted root nodeId"))?;
            let selector = connection
                .command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "DOM.querySelector",
                    json!({"nodeId":root,"selector":selector_arg}),
                )
                .await
                .map_err(|_| invalid("selector_error:selector:invalid_selector"))?;
            let node_id = selector
                .get("nodeId")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or_else(|| invalid("selector_error:selector:no_match"))?;
            let response = connection
                .command(
                    registry,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Accessibility.getPartialAXTree",
                    json!({"nodeId":node_id,"fetchRelatives":false}),
                )
                .await
                .map_err(|e| invalid(e.to_string()))?;
            // Fail closed if the parsed CDP result already exceeds the caller's budget.
            if serde_json::to_vec(&response)
                .map(|bytes| bytes.len())
                .unwrap_or(usize::MAX)
                > args.max_bytes
            {
                return Err(invalid("AX response exceeds requested byte budget"));
            }
            let after =
                shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
                    .await
                    .map_err(invalid)?;
            if before != after {
                return Err(invalid(
                    "shared target navigated during accessibility read; result discarded as stale",
                ));
            }
            let nodes = response
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("accessibility response omitted nodes"))?;
            let result = json!({
                "chrome_tab_id":args.chrome_tab_id,
                "nodes":nodes.iter().take(1).cloned().collect::<Vec<_>>(),
                "truncated":nodes.len()>1,
                "missing":if nodes.len()>1 { vec!["AX response exceeded the selected-node bound and was truncated"] } else { vec![] },
                "shared_frame_identity":{"frame_id":before.0,"loader_id":before.1,"freshness":"same root frame, loader, and URL before and after this read"}
            });
            if serde_json::to_vec(&result)
                .map(|bytes| bytes.len())
                .unwrap_or(usize::MAX)
                > args.max_bytes
            {
                return Err(invalid("AX byte budget cannot fit selected-node metadata"));
            }
            Ok(result)
        })
        .await
        .map_err(|_| invalid("shared accessibility deadline exceeded"))??;
        Ok(rmcp::handler::server::wrapper::Json(result))
    }

    #[tool(
        name = "shared_input",
        description = "Perform guarded fill, nonempty sequential ASCII typing, or click on an explicitly paired Chrome tab. Requires a unique CSS match and exact current value; typing retains the matched DOM object, requires a focused ordinary text field with a collapsed caret at its end, rechecks object identity and value around each key dispatch and at readback, and waits a fixed 60 ms between characters. Typing is refused before dispatch if its minimum paced duration cannot fit the action budget. This sends CDP key events, not OS hardware input or IME, and must not be used to bypass app security or bot checks. A key release gets at most one bounded retry, but any uncertain dispatch/release is an error. Enforces a 6–60 second overall deadline; fill and typing require the same root frame, loader, and URL before and after, while click may return a verified new-document identity for navigation in the same selected tab. Readback proves only DOM state, not app save or persistence."
    )]
    async fn shared_input(
        &self,
        Parameters(args): Parameters<SharedInputArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        if args.selector.trim().is_empty()
            || args.selector.len() > 512
            || args.selector.contains('\0')
            || args.expected_value.len() > 16_384
            || args.value.len() > 16_384
        {
            return Err(invalid("selector and values exceed shared input bounds"));
        }
        if args.action != "fill" && args.action != "type" && args.action != "click" {
            return Err(invalid("shared input action must be fill, type, or click"));
        }
        if args.action == "type" && !args.value.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            return Err(invalid(
                "shared sequential typing supports ASCII only; use fill for Unicode text",
            ));
        }
        if args.action == "type" && args.value.is_empty() {
            return Err(invalid(
                "shared sequential typing requires nonempty ASCII text",
            ));
        }
        if args.action == "click"
            && (args.postcondition_selector.as_ref().is_none_or(|selector| {
                selector.trim().is_empty() || selector.len() > 512 || selector.contains('\0')
            }) || args
                .postcondition
                .as_ref()
                .is_none_or(|value| value.len() > 16_384))
        {
            return Err(invalid(
                "click requires a bounded postcondition selector and exact value",
            ));
        }
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(60_000).clamp(6_000, 60_000));
        if args.action == "type" {
            let minimum_duration = shared_typing_minimum_duration(args.value.chars().count());
            let action_budget = timeout.saturating_sub(Duration::from_secs(5));
            if minimum_duration >= action_budget {
                return Err(invalid(format!(
                    "paced sequential typing needs at least {} ms, exceeding the {} ms action budget",
                    minimum_duration.as_millis(),
                    action_budget.as_millis()
                )));
            }
        }
        let deadline = tokio::time::Instant::now() + timeout;
        let action_deadline = deadline - Duration::from_secs(5);
        let recovery_deadline = deadline - Duration::from_secs(2);
        let shared = tokio::time::timeout_at(deadline, self.shared_sessions.lock())
            .await
            .map_err(|_| invalid("shared input deadline exceeded waiting for session registry"))?
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let shared = tokio::time::timeout_at(deadline, shared.lock())
            .await
            .map_err(|_| invalid("shared input deadline exceeded waiting for session"))?;
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let before = tokio::time::timeout_at(
            action_deadline,
            shared_frame_identity(
                connection,
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
            ),
        )
        .await
        .map_err(|_| invalid("shared input deadline exceeded before dispatch"))?
        .map_err(invalid)?;
        let selector = serde_json::to_string(&args.selector).map_err(|e| invalid(e.to_string()))?;
        let expected =
            serde_json::to_string(&args.expected_value).map_err(|e| invalid(e.to_string()))?;
        let postcondition_selector =
            serde_json::to_string(args.postcondition_selector.as_deref().unwrap_or_default())
                .map_err(|e| invalid(e.to_string()))?;
        let expression = if args.action == "fill" {
            let value = serde_json::to_string(&args.value).map_err(|e| invalid(e.to_string()))?;
            format!(
                r#"(()=>{{let es;try{{es=[...document.querySelectorAll({selector})]}}catch(_){{return {{ok:false,reason:'invalid_selector'}}}};if(es.length!==1)return {{ok:false,reason:es.length?'ambiguous':'no_match'}};const e=es[0];e.scrollIntoView({{block:'nearest'}});const s=getComputedStyle(e),b=e.getBoundingClientRect(),x=b.left+b.width/2,y=b.top+b.height/2,h=document.elementFromPoint(x,y);if(!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)||['password','hidden','file','checkbox','radio','button','submit','reset','image'].includes(e.type||'')||e.matches(':disabled')||e.readOnly||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted')||b.width<=0||b.height<=0||b.left<0||b.top<0||b.right>innerWidth||b.bottom>innerHeight||s.visibility==='hidden'||s.display==='none'||s.pointerEvents==='none'||h!==e)return {{ok:false,reason:'blocked'}};if(e.value!=={expected})return {{ok:false,reason:'stale_value'}};const setter=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value')?.set;if(!setter)return {{ok:false,reason:'blocked'}};setter.call(e,{value});e.dispatchEvent(new InputEvent('input',{{bubbles:true,inputType:'insertText',data:{value}}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{ok:e.value==={value},value:e.value,dispatched:true}};}})()"#
            )
        } else if args.action == "type" {
            format!(
                r#"(()=>{{let es;try{{es=[...document.querySelectorAll({selector})]}}catch(_){{return {{node:null,identityRoute:'shared_type_handle'}}}};if(es.length!==1)return {{node:null,identityRoute:'shared_type_handle'}};const e=es[0];e.scrollIntoView({{block:'nearest'}});e.focus();return {{node:e,identityRoute:'shared_type_handle'}};}})()"#
            )
        } else {
            let unsafe_predicate = SHARED_CLICK_UNSAFE_PREDICATE;
            let postcondition = args.postcondition.as_deref().unwrap_or_default();
            let postcondition_json =
                serde_json::to_string(postcondition).map_err(|e| invalid(e.to_string()))?;
            format!(
                r#"(()=>{{let es,ps;try{{es=[...document.querySelectorAll({selector})];ps=[...document.querySelectorAll({postcondition_selector})]}}catch(_){{return {{ok:false,reason:'invalid_selector'}}}};if(es.length!==1||ps.length!==1)return {{ok:false,reason:es.length!==1?(es.length?'ambiguous':'no_match'):(ps.length?'postcondition_ambiguous':'postcondition_no_match')}};const e=es[0],p=ps[0];e.scrollIntoView({{block:'nearest'}});const s=getComputedStyle(e),b=e.getBoundingClientRect(),t=e.type||'',x=b.left+b.width/2,y=b.top+b.height/2,h=document.elementFromPoint(x,y),visible=b.width>0&&b.height>0&&b.left>=0&&b.top>=0&&b.right<=innerWidth&&b.bottom<=innerHeight&&s.visibility!=='hidden'&&s.display!=='none'&&s.pointerEvents!=='none',unsafe={unsafe_predicate},disabled=e.matches(':disabled'),current=(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement||e instanceof HTMLSelectElement)?e.value:(e.innerText??''),postcondition_before=(p instanceof HTMLInputElement||p instanceof HTMLTextAreaElement||p instanceof HTMLSelectElement)?p.value:(p.innerText??'');if(p===e||!visible||disabled||unsafe||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted')||p.hasAttribute('data-masked')||p.hasAttribute('data-requires-trusted')||!h||!(h===e||e.contains(h)))return {{ok:false,reason:'blocked',detail:{{p_is_e:p===e,visible,disabled,unsafe,masked:e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted')||p.hasAttribute('data-masked')||p.hasAttribute('data-requires-trusted'),covered:!h||!(h===e||e.contains(h)),rect:{{l:Math.round(b.left),t:Math.round(b.top),r:Math.round(b.right),b2:Math.round(b.bottom)}},iw:innerWidth,ih:innerHeight,pt:h?h.tagName:null}}}};if(typeof current!=='string'||current.length>16384)return {{ok:false,reason:'click_text_too_large'}};if(typeof postcondition_before!=='string'||postcondition_before.length>16384)return {{ok:false,reason:'postcondition_text_too_large'}};if(current!=={expected})return {{ok:false,reason:'stale_value'}};const postcondition_before_is_desired=postcondition_before==={postcondition_json};if(postcondition_before_is_desired)return {{ok:false,reason:'postcondition_already_satisfied'}};return {{ok:true,x,y,postcondition_before_is_desired}};}})()"#
            )
        };
        let mut typing_object_id = None;
        let (preflight, mut action_error) = match tokio::time::timeout_at(
            action_deadline,
            connection.command(
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.evaluate",
                json!({"expression":expression,"returnByValue":args.action!="type","awaitPromise":false}),
            ),
        )
        .await
        {
            Ok(Ok(response)) if args.action == "type" => {
                typing_object_id = response
                    .pointer("/result/objectId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if typing_object_id.is_some() {
                    (json!({"ok":true}), None)
                } else {
                    (Value::Null, Some("typing identity handle could not be created".into()))
                }
            }
            Ok(Ok(response)) => (
                response.pointer("/result/value").cloned().unwrap_or(Value::Null),
                None,
            ),
            Ok(Err(error)) => (
                Value::Null,
                Some(format!("input dispatch outcome is uncertain: {error}")),
            ),
            Err(_) => (
                Value::Null,
                Some("shared input deadline exceeded; effect may have occurred".into()),
            ),
        };
        let dispatch_may_have_occurred =
            args.action == "fill" && (preflight["dispatched"] == true || preflight["ok"] == true);
        let mut click_postcondition_error = None;
        if args.action != "type" && action_error.is_none() && preflight["ok"] != true {
            let detail = preflight["detail"]
                .as_object()
                .map(|d| serde_json::to_string(d).unwrap_or_default())
                .unwrap_or_default();
            action_error = Some(if dispatch_may_have_occurred {
                "shared fill readback changed after input/change dispatch; inspect state before retrying".into()
            } else {
                format!(
                    "shared input refused: {}{}",
                    preflight["reason"].as_str().unwrap_or("unverifiable"),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(" ({detail})")
                    }
                )
            });
        }
        let observed_value;
        if args.action == "fill" {
            let observed = preflight["value"].as_str().unwrap_or_default().to_owned();
            if action_error.is_none() {
                let readback = format!(
                    r#"(()=>{{let es;try{{es=[...document.querySelectorAll({selector})]}}catch(_){{return {{ok:false}}}};if(es.length!==1)return {{ok:false}};const e=es[0],v=e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement?e.value:null;return {{ok:e.isConnected&&v==={expected_value},value:v}};}})()"#,
                    expected_value =
                        serde_json::to_string(&args.value).map_err(|e| invalid(e.to_string()))?
                );
                match tokio::time::timeout_at(
                    action_deadline,
                    connection.command(
                        shared.registry()?,
                        &shared.handle,
                        &args.chrome_tab_id,
                        "Runtime.evaluate",
                        json!({"expression":readback,"returnByValue":true,"awaitPromise":false}),
                    ),
                )
                .await
                {
                    Ok(Ok(response)) => {
                        let value = response
                            .pointer("/result/value")
                            .cloned()
                            .unwrap_or(Value::Null);
                        if value["ok"] == true && value["value"].as_str() == Some(&args.value) {
                            observed_value = value["value"].as_str().unwrap_or_default().to_owned();
                        } else {
                            action_error = Some("shared fill post-event readback did not match; effect may have occurred".into());
                            observed_value = observed;
                        }
                    }
                    Ok(Err(error)) => {
                        action_error = Some(format!(
                            "shared fill post-event readback failed; effect may have occurred: {error}"
                        ));
                        observed_value = observed;
                    }
                    Err(_) => {
                        action_error = Some("shared fill post-event readback deadline exceeded; effect may have occurred".into());
                        observed_value = observed;
                    }
                }
            } else {
                observed_value = observed;
            }
        } else if args.action == "type" {
            let mut expected_value = args.expected_value.clone();
            let object_id = typing_object_id.as_deref();
            if action_error.is_none() && object_id.is_none() {
                action_error = Some("typing identity handle is unavailable".into());
            }
            if action_error.is_none() {
                let object_id = object_id.unwrap();
                for (index, character) in args.value.chars().enumerate() {
                    if index > 0 {
                        let pace_deadline =
                            tokio::time::Instant::now() + SHARED_TYPE_INTER_CHARACTER_DELAY;
                        if tokio::time::timeout_at(
                            std::cmp::min(pace_deadline, action_deadline),
                            tokio::time::sleep_until(pace_deadline),
                        )
                        .await
                        .is_err()
                        {
                            action_error = Some(
                                "typing pace deadline exceeded before next key dispatch; earlier characters may have been entered".into(),
                            );
                            break;
                        }
                    }
                    let checked = shared_type_guard(
                        connection,
                        shared.registry()?,
                        &shared.handle,
                        SharedTypeGuard {
                            tab_id: &args.chrome_tab_id,
                            object_id,
                            selector: &args.selector,
                            expected: &expected_value,
                            final_check: false,
                            deadline: action_deadline,
                        },
                    )
                    .await;
                    let checked = match checked {
                        Ok(value) => value,
                        Err(error) => {
                            action_error = Some(format!(
                                "typing guard failed; effect may have occurred: {error}"
                            ));
                            break;
                        }
                    };
                    if checked["ok"] != true {
                        action_error = Some(format!(
                            "sequential typing stopped: {}",
                            checked["reason"].as_str().unwrap_or("unverifiable")
                        ));
                        break;
                    }
                    let key = character.to_string();
                    let key_up = json!({"type":"keyUp","key":key});
                    let key_down = tokio::time::timeout_at(
                        action_deadline,
                        connection.command(
                            shared.registry()?,
                            &shared.handle,
                            &args.chrome_tab_id,
                            "Input.dispatchKeyEvent",
                            json!({"type":"keyDown","key":key}),
                        ),
                    )
                    .await
                    .map_err(|_| "key down deadline exceeded; effect may have occurred".to_owned())
                    .and_then(|result| result.map_err(|error| error.to_string()));
                    if let Err(error) = key_down {
                        let recovery = shared_release_key(
                            connection,
                            shared.registry()?,
                            &shared.handle,
                            &args.chrome_tab_id,
                            &key_up,
                            recovery_deadline,
                        )
                        .await
                        .err()
                        .map(|failure| format!("; key-up recovery uncertain: {failure}"))
                        .unwrap_or_default();
                        action_error = Some(format!(
                            "key down outcome uncertain; key release attempted: {error}{recovery}"
                        ));
                        break;
                    }
                    let checked_down = shared_type_guard(
                        connection,
                        shared.registry()?,
                        &shared.handle,
                        SharedTypeGuard {
                            tab_id: &args.chrome_tab_id,
                            object_id,
                            selector: &args.selector,
                            expected: &expected_value,
                            final_check: false,
                            deadline: action_deadline,
                        },
                    )
                    .await;
                    let checked_down = match checked_down {
                        Ok(value) => value,
                        Err(error) => {
                            let recovery = shared_release_key(
                                connection,
                                shared.registry()?,
                                &shared.handle,
                                &args.chrome_tab_id,
                                &key_up,
                                recovery_deadline,
                            )
                            .await
                            .err()
                            .map(|failure| format!("; key-up recovery uncertain: {failure}"))
                            .unwrap_or_default();
                            action_error = Some(format!(
                                "typing revalidation failed; key release attempted: {error}{recovery}"
                            ));
                            break;
                        }
                    };
                    if checked_down["ok"] != true {
                        let reason = checked_down["reason"].as_str().unwrap_or("unverifiable");
                        let recovery = shared_release_key(
                            connection,
                            shared.registry()?,
                            &shared.handle,
                            &args.chrome_tab_id,
                            &key_up,
                            recovery_deadline,
                        )
                        .await
                        .err()
                        .map(|failure| format!("; key-up recovery uncertain: {failure}"))
                        .unwrap_or_default();
                        action_error = Some(format!(
                            "sequential typing stopped before character dispatch: {reason}{recovery}"
                        ));
                        break;
                    }
                    let typed = tokio::time::timeout_at(
                        action_deadline,
                        connection.command(
                            shared.registry()?,
                            &shared.handle,
                            &args.chrome_tab_id,
                            "Input.dispatchKeyEvent",
                            json!({"type":"char","text":key,"unmodifiedText":key}),
                        ),
                    )
                    .await
                    .map_err(|_| {
                        "character event deadline exceeded; effect may have occurred".to_owned()
                    })
                    .and_then(|result| result.map_err(|error| error.to_string()));
                    if let Err(error) = typed {
                        let recovery = shared_release_key(
                            connection,
                            shared.registry()?,
                            &shared.handle,
                            &args.chrome_tab_id,
                            &key_up,
                            recovery_deadline,
                        )
                        .await
                        .err()
                        .map(|failure| format!("; key-up recovery uncertain: {failure}"))
                        .unwrap_or_default();
                        action_error = Some(format!(
                            "character event outcome uncertain; key release attempted: {error}{recovery}"
                        ));
                        break;
                    }
                    expected_value.push(character);
                    if let Err(error) = shared_release_key(
                        connection,
                        shared.registry()?,
                        &shared.handle,
                        &args.chrome_tab_id,
                        &key_up,
                        recovery_deadline,
                    )
                    .await
                    {
                        action_error = Some(format!(
                            "key release outcome uncertain after bounded retry; stop and reobserve: {error}"
                        ));
                        break;
                    }
                }
            }
            if action_error.is_none() {
                let object_id = object_id.unwrap();
                match shared_type_guard(
                    connection,
                    shared.registry()?,
                    &shared.handle,
                    SharedTypeGuard {
                        tab_id: &args.chrome_tab_id,
                        object_id,
                        selector: &args.selector,
                        expected: &expected_value,
                        final_check: true,
                        deadline: action_deadline,
                    },
                )
                .await
                {
                    Ok(value)
                        if value["ok"] == true
                            && value["value"].as_str() == Some(&expected_value) =>
                    {
                        observed_value = expected_value;
                    }
                    Ok(_) => {
                        action_error = Some("sequential typing post-event identity/value/caret readback did not match; effect may have occurred".into());
                        observed_value = String::new();
                    }
                    Err(error) => {
                        action_error = Some(format!(
                            "sequential typing post-event readback failed; effect may have occurred: {error}"
                        ));
                        observed_value = String::new();
                    }
                }
            } else {
                observed_value = String::new();
            }
            if let Some(object_id) = typing_object_id {
                let released = tokio::time::timeout_at(
                    recovery_deadline,
                    connection.command(
                        shared.registry()?,
                        &shared.handle,
                        &args.chrome_tab_id,
                        "Runtime.releaseObject",
                        json!({"objectId":object_id}),
                    ),
                )
                .await
                .map_err(|_| "typing handle cleanup timed out".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
                if let Err(error) = released {
                    let cleanup_error = format!("typing handle cleanup failed: {error}");
                    if let Some(action_error) = action_error.as_mut() {
                        action_error.push_str(&format!("; {cleanup_error}"));
                    } else {
                        action_error = Some(cleanup_error);
                    }
                }
            }
        } else if action_error.is_none() {
            let mut postcondition_before_is_desired = false;
            if let Some((_x, _y)) = preflight["x"].as_f64().zip(preflight["y"].as_f64()) {
                let refreshed = tokio::time::timeout_at(
                    action_deadline,
                    connection.command(
                        shared.registry()?,
                        &shared.handle,
                        &args.chrome_tab_id,
                        "Runtime.evaluate",
                        json!({"expression":expression,"returnByValue":true,"awaitPromise":false}),
                    ),
                )
                .await
                .map_err(|_| "click revalidation deadline exceeded".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
                let refreshed = match refreshed {
                    Ok(response) => response
                        .pointer("/result/value")
                        .cloned()
                        .unwrap_or(Value::Null),
                    Err(error) => {
                        action_error = Some(format!(
                            "click target revalidation failed before dispatch: {error}"
                        ));
                        Value::Null
                    }
                };
                if action_error.is_none() && refreshed["ok"] != true {
                    action_error = Some(format!(
                        "click target changed before dispatch: {}",
                        refreshed["reason"].as_str().unwrap_or("unverifiable")
                    ));
                }
                if action_error.is_none() && refreshed["postcondition_before_is_desired"] != false {
                    action_error = Some("click postcondition baseline was unverifiable".into());
                }
                if action_error.is_none() {
                    postcondition_before_is_desired =
                        refreshed["postcondition_before_is_desired"] == true;
                }
                if action_error.is_none() {
                    if let Some((x, y)) = refreshed["x"].as_f64().zip(refreshed["y"].as_f64()) {
                        let moved = tokio::time::timeout_at(
                            action_deadline,
                            connection.command(
                                shared.registry()?,
                                &shared.handle,
                                &args.chrome_tab_id,
                                "Input.dispatchMouseEvent",
                                json!({"type":"mouseMoved","x":x,"y":y,"button":"left"}),
                            ),
                        )
                        .await
                        .map_err(|_| "mouse move deadline exceeded".to_owned())
                        .and_then(|result| result.map_err(|error| error.to_string()));
                        let press = if let Err(error) = moved {
                            Err(error)
                        } else {
                            tokio::time::timeout_at(
                            action_deadline,
                            connection.command(
                                shared.registry()?,
                                &shared.handle,
                                &args.chrome_tab_id,
                                "Input.dispatchMouseEvent",
                                json!({"type":"mousePressed","x":x,"y":y,"button":"left","clickCount":1}),
                            ),
                            )
                            .await
                            .map_err(|_| "mouse press deadline exceeded".to_owned())
                            .and_then(|result| result.map_err(|error| error.to_string()))
                        };
                        let release = tokio::time::timeout_at(
                            recovery_deadline,
                            connection.command(
                                shared.registry()?,
                                &shared.handle,
                                &args.chrome_tab_id,
                                "Input.dispatchMouseEvent",
                                json!({"type":"mouseReleased","x":x,"y":y,"button":"left","clickCount":1}),
                            ),
                        )
                        .await
                        .map_err(|_| "mouse release deadline exceeded".to_owned())
                        .and_then(|result| result.map_err(|error| error.to_string()));
                        match (press, release) {
                            (Ok(_), Ok(_)) => {}
                            (Err(press_error), Ok(_)) => {
                                action_error = Some(format!(
                                    "mouse press outcome uncertain; release was attempted: {press_error}"
                                ))
                            }
                            (Ok(_), Err(release_error)) => {
                                action_error = Some(format!(
                                    "mouse release outcome uncertain; pointer button may remain pressed: {release_error}"
                                ))
                            }
                            (Err(press_error), Err(release_error)) => {
                                action_error = Some(format!(
                                    "mouse press outcome uncertain ({press_error}); release also uncertain and pointer button may remain pressed ({release_error})"
                                ))
                            }
                        }
                    } else {
                        action_error =
                            Some("click coordinates unavailable after revalidation".into());
                    }
                }
            } else {
                action_error = Some("click target coordinates unavailable".into());
            }
            let postcondition = args.postcondition.as_deref().unwrap_or_default();
            if action_error.is_none() {
                let postcondition_json =
                    serde_json::to_string(postcondition).map_err(|e| invalid(e.to_string()))?;
                let before_json = serde_json::to_string(&postcondition_before_is_desired)
                    .map_err(|e| invalid(e.to_string()))?;
                let readback_expression = format!(
                    r#"(()=>{{let es;try{{es=[...document.querySelectorAll({postcondition_selector})]}}catch(_){{return {{ok:false}}}};if(es.length!==1)return {{ok:false}};const e=es[0],current=(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement||e instanceof HTMLSelectElement)?e.value:(e.innerText??'');return {{ok:e.isConnected&&typeof current==='string'&&current.length<=16384&&current==={postcondition_json}&&{before_json}===false}};}})()"#
                );
                match tokio::time::timeout_at(action_deadline, connection.command(
                    shared.registry()?, &shared.handle, &args.chrome_tab_id, "Runtime.evaluate",
                    json!({"expression":readback_expression,"returnByValue":true,"awaitPromise":false}),
                )).await {
                    Ok(Ok(readback)) if readback.pointer("/result/value/ok") == Some(&Value::Bool(true)) => {}
                    Ok(Ok(_)) => click_postcondition_error = Some("click postcondition did not match; effect may have occurred".into()),
                    Ok(Err(error)) => click_postcondition_error = Some(format!("click postcondition readback failed; effect may have occurred: {error}")),
                    Err(_) => click_postcondition_error = Some("click postcondition deadline exceeded; effect may have occurred".into()),
                }
            }
            observed_value = postcondition.to_owned();
        } else {
            observed_value = String::new();
        }
        let first_after = tokio::time::timeout_at(
            deadline,
            shared_frame_identity(
                connection,
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
            ),
        )
        .await;
        let after = match first_after {
            Ok(Ok(after)) => after,
            Ok(Err(error)) if args.action == "click" && action_error.is_none() => {
                wait_for_shared_frame_change(
                    connection,
                    shared.registry()?,
                    &shared.handle,
                    &args.chrome_tab_id,
                    &before,
                    deadline,
                )
                .await
                .map_err(|navigation_error| {
                    invalid(format!(
                        "post-click identity could not be confirmed ({navigation_error}); effect may have occurred: {error}"
                    ))
                })?
            }
            Err(_) if args.action == "click" && action_error.is_none() => {
                wait_for_shared_frame_change(
                    connection,
                    shared.registry()?,
                    &shared.handle,
                    &args.chrome_tab_id,
                    &before,
                    deadline,
                )
                .await
                .map_err(|navigation_error| {
                    invalid(format!(
                        "post-click identity could not be confirmed ({navigation_error}); effect may have occurred"
                    ))
                })?
            }
            Ok(Err(error)) => {
                return Err(invalid(format!(
                    "post-action identity readback failed; effect may have occurred: {error}"
                )))
            }
            Err(_) => {
                return Err(invalid(
                    "post-action identity readback deadline exceeded; effect may have occurred",
                ))
            }
        };
        let should_wait_for_click_navigation =
            click_postcondition_error.as_deref().is_some_and(|error| {
                error.contains("readback failed") || error.contains("deadline exceeded")
            });
        let after = if args.action == "click"
            && action_error.is_none()
            && should_wait_for_click_navigation
            && before == after
        {
            wait_for_shared_frame_change(
                connection,
                shared.registry()?,
                &shared.handle,
                &args.chrome_tab_id,
                &before,
                deadline,
            )
            .await
            .unwrap_or(after)
        } else {
            after
        };
        if before != after {
            if args.action == "fill" && dispatch_may_have_occurred {
                return Ok(rmcp::handler::server::wrapper::Json(json!({
                    "status":"unknown","action":"fill","dispatch_acknowledged":true,
                    "dispatch_may_have_occurred":true,
                    "reason":"shared target changed after fill dispatch; inspect the new document before retrying",
                    "identity_before":{"frame_id":before.0,"loader_id":before.1,"url":before.2},
                    "identity_after":{"frame_id":after.0,"loader_id":after.1,"url":after.2}
                })));
            }
            if args.action == "click" && action_error.is_none() {
                return Ok(rmcp::handler::server::wrapper::Json(json!({
                    "action":"click","dispatch_acknowledged":true,"navigation_observed":true,
                    "postcondition":"unavailable_due_to_navigation",
                    "identity":"same explicitly paired tab; new document observed",
                    "navigation":{"frame_id":after.0,"loader_id":after.1,"url":after.2}
                })));
            }
            return Err(invalid(
                "shared target changed frame, loader, or URL during input; effect may have occurred",
            ));
        }
        if let Some(error) = click_postcondition_error {
            return Err(invalid(error));
        }
        if let Some(error) = action_error {
            if dispatch_may_have_occurred {
                return Ok(rmcp::handler::server::wrapper::Json(json!({
                    "status":"unknown","action":"fill","dispatch_acknowledged":true,
                    "dispatch_may_have_occurred":true,"reason":error
                })));
            }
            return Err(invalid(error));
        }
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "action":args.action,"observed_value":observed_value,"verified":true,
            "identity":"same root frame, loader, and URL before and after"
        })))
    }
    #[tool(
        name = "shared_tab",
        description = "Open or navigate a tab in the shared extension session, or adopt a recent popup only when its exact Chrome opener, creation timestamp, candidate ID, URL, and a fresh visible account marker all match. `adopt_popup` requires the click-start time and candidate creation time returned by the opener-bound tab evidence, plus a CSS scope and exact account-marker text. It reports visible UI evidence only and does not establish authentication."
    )]
    async fn shared_tab(
        &self,
        Parameters(args): Parameters<SharedTabArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        if matches!(args.action.as_str(), "open" | "navigate" | "adopt_popup") {
            validate_shared_url(&args.url)?;
        } else if !args.url.is_empty() {
            return Err(invalid("lifecycle actions do not accept url"));
        }
        if args.action == "adopt_popup" {
            let opener_tab_id = args.chrome_tab_id.as_deref().ok_or_else(|| {
                invalid("adopt_popup requires the already paired opener chrome_tab_id")
            })?;
            if opener_tab_id.parse::<u32>().is_err() {
                return Err(invalid(
                    "adopt_popup opener chrome_tab_id must be a decimal Chrome tab ID",
                ));
            }
            let candidate_tab_id = args.candidate_tab_id.as_deref().ok_or_else(|| {
                invalid("adopt_popup requires candidate_tab_id from a verified new-tab outcome")
            })?;
            let created_at_ms = args.created_at_ms.ok_or_else(|| {
                invalid("adopt_popup requires the observed candidate created_at_ms")
            })?;
            let since_ms = args
                .since_ms
                .ok_or_else(|| invalid("adopt_popup requires the originating click since_ms"))?;
            let marker_selector = args
                .account_marker_selector
                .as_deref()
                .filter(|selector| {
                    !selector.trim().is_empty()
                        && selector.len() <= 512
                        && !selector.chars().any(char::is_control)
                })
                .ok_or_else(|| {
                    invalid("adopt_popup requires a bounded CSS account_marker_selector")
                })?;
            let marker_text = args
                .account_marker_text
                .as_deref()
                .filter(|text| {
                    !text.trim().is_empty()
                        && text.len() <= 512
                        && !text.chars().any(char::is_control)
                })
                .ok_or_else(|| invalid("adopt_popup requires exact visible account_marker_text"))?;
            if args.idempotency_key.is_some()
                || args.active.is_some()
                || args.wait_until.is_some()
                || args.create_background_window.is_some()
                || args.target_window_id.is_some()
            {
                return Err(invalid(
                    "adopt_popup accepts only session_id, opener chrome_tab_id, url, candidate identity/timing, and account-marker fields",
                ));
            }
            let shared = self
                .shared_sessions
                .lock()
                .await
                .get(&args.session_id)
                .cloned()
                .ok_or_else(|| invalid("unknown shared session_id"))?;
            let mut shared = shared.lock().await;
            if shared.handle.principal != self.principal.as_ref() {
                return Err(invalid("session is not owned by this server principal"));
            }
            let connection = shared
                .connection
                .as_ref()
                .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
            if !connection
                .selected_targets()
                .iter()
                .any(|target| target == opener_tab_id)
            {
                return Err(invalid(
                    "popup opener is not part of this guarded shared session",
                ));
            }
            let inventory = connection
                .command(
                    shared.registry()?,
                    &shared.handle,
                    opener_tab_id,
                    "Controlla.newTabs",
                    json!({"since_ms":since_ms}),
                )
                .await
                .map_err(|error| invalid(error.to_string()))?;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| invalid("system clock is before Unix epoch"))?
                .as_millis()
                .min(u64::MAX as u128) as u64;
            let candidate = popup_candidate_for_adoption(
                &inventory,
                opener_tab_id,
                candidate_tab_id,
                created_at_ms,
                since_ms,
                &args.url,
                now_ms,
            )
            .map_err(invalid)?;
            if shared.connection.as_ref().is_some_and(|connection| {
                connection
                    .selected_targets()
                    .iter()
                    .any(|target| target == candidate_tab_id)
            }) {
                return Err(invalid(
                    "popup candidate is already attached to this shared session",
                ));
            }
            shared
                .registry
                .as_mut()
                .ok_or_else(|| invalid("shared registry is unavailable"))?
                .register_tab(
                    &args.session_id,
                    candidate_tab_id.to_owned(),
                    Ownership::Borrowed,
                )
                .map_err(|error| invalid(format!("cannot register popup candidate: {error:?}")))?;
            if let Err(error) = shared
                .connection
                .as_mut()
                .ok_or_else(|| invalid("shared extension session has not been accepted"))?
                .add_selected_target(candidate_tab_id.to_owned())
            {
                let _ = shared.registry.as_mut().and_then(|registry| {
                    registry
                        .unregister_tab(&args.session_id, candidate_tab_id)
                        .ok()
                });
                return Err(invalid(error));
            }
            let marker_snapshot = capture_shared_snapshot(
                &mut shared,
                &SharedSnapshotArgs {
                    session_id: args.session_id.clone(),
                    chrome_tab_id: candidate_tab_id.to_owned(),
                    selector: marker_selector.to_owned(),
                    max_items: 32,
                    max_text_chars: 1024,
                    max_bytes: 16384,
                    href_hint: None,
                    goal: None,
                    max_scan_nodes: Some(4096),
                },
            )
            .await;
            let snapshot = match marker_snapshot {
                Ok(snapshot) if popup_account_marker_matches(&snapshot, &args.url, marker_text) => {
                    snapshot
                }
                Ok(_) => {
                    rollback_shared_popup_adoption(&mut shared, candidate_tab_id).await;
                    return Err(invalid(
                        "popup was not adopted: exact destination URL and one fresh visible account marker were not verified",
                    ));
                }
                Err(error) => {
                    rollback_shared_popup_adoption(&mut shared, candidate_tab_id).await;
                    return Err(invalid(format!(
                        "popup was not adopted: fresh destination/account snapshot failed: {error}"
                    )));
                }
            };
            return Ok(rmcp::handler::server::wrapper::Json(json!({
                "status":"adopted","chrome_tab_id":candidate["tab_id"],"url":snapshot["url"],"created_at_ms":candidate["created_at_ms"],
                "opener_tab_id":candidate["opener_tab_id"],"account_marker_verified":true,
                "evidence_scope":"unique_recent_opener_bound_candidate_and_fresh_visible_account_marker_ui_only",
                "authentication":"not independently established; visible account marker only"
            })));
        }
        if args.action == "close_owned" {
            let target = args
                .chrome_tab_id
                .as_deref()
                .ok_or_else(|| invalid("chrome_tab_id is required to close an owned tab"))?;
            if target.parse::<u32>().is_err() {
                return Err(invalid("chrome_tab_id must be a decimal Chrome tab ID"));
            }
            let shared = self
                .shared_sessions
                .lock()
                .await
                .get(&args.session_id)
                .cloned()
                .ok_or_else(|| invalid("unknown shared session_id"))?;
            let shared = shared.lock().await;
            if shared.handle.principal != self.principal.as_ref() {
                return Err(invalid("session is not owned by this server principal"));
            }
            let connection = shared
                .connection
                .as_ref()
                .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
            let source = connection.selected_targets().iter().find(|candidate| candidate.as_str() != target)
                .cloned()
                .ok_or_else(|| invalid("close_owned needs another still-open tab in this session to verify the target lifecycle"))?;
            let receipt = connection
                .command(
                    shared.registry()?,
                    &shared.handle,
                    &source,
                    "Controlla.closeOwnedTab",
                    json!({"owned_tab_id":target}),
                )
                .await
                .map_err(|error| invalid(error.to_string()))?;
            if receipt["status"] != "closed"
                || receipt["topology_verified"] != true
                || receipt["tab_id"] != target
            {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"unknown","reason":"Chrome did not independently confirm closure of the exact Controlla-owned tab","receipt":receipt}),
                ));
            }
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"closed","verified":true,"receipt":receipt}),
            ));
        }
        if args.action == "move_owned" {
            if !args.url.is_empty()
                || args.idempotency_key.is_some()
                || args.active.is_some()
                || args.wait_until.is_some()
                || args.create_background_window == Some(true)
            {
                return Err(invalid(
                    "move_owned accepts only session_id, chrome_tab_id, and target_window_id",
                ));
            }
            let target = args
                .chrome_tab_id
                .as_deref()
                .ok_or_else(|| invalid("chrome_tab_id is required"))?;
            let window_id = args
                .target_window_id
                .ok_or_else(|| invalid("target_window_id is required"))?;
            let shared = self
                .shared_sessions
                .lock()
                .await
                .get(&args.session_id)
                .cloned()
                .ok_or_else(|| invalid("unknown shared session_id"))?;
            let shared = shared.lock().await;
            if shared.handle.principal != self.principal.as_ref() {
                return Err(invalid("session is not owned by this server principal"));
            }
            let connection = shared
                .connection
                .as_ref()
                .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
            let source = connection
                .selected_targets()
                .iter()
                .find(|candidate| candidate.as_str() != target)
                .cloned()
                .ok_or_else(|| {
                    invalid("move_owned needs another still-open source tab in this session")
                })?;
            let receipt = connection
                .command(
                    shared.registry()?,
                    &shared.handle,
                    &source,
                    "Controlla.moveOwnedTab",
                    json!({"owned_tab_id":target,"target_window_id":window_id}),
                )
                .await
                .map_err(|error| invalid(error.to_string()))?;
            if receipt["status"] != "moved"
                || receipt["topology_verified"] != true
                || receipt["tab_id"] != target
                || receipt["window_id"].as_u64() != Some(window_id)
            {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"unknown","reason":"Chrome did not independently confirm the exact moved tab and destination window","receipt":receipt}),
                ));
            }
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"moved","verified":true,"receipt":receipt}),
            ));
        }
        if args.action == "activate_owned" {
            if args.active.is_some()
                || args.wait_until.is_some()
                || args.create_background_window == Some(true)
                || args.target_window_id.is_some()
            {
                return Err(invalid(
                    "activate_owned accepts only session_id, chrome_tab_id, and idempotency_key",
                ));
            }
            let target = args
                .chrome_tab_id
                .as_deref()
                .ok_or_else(|| invalid("chrome_tab_id is required"))?;
            if target.parse::<u32>().is_err() {
                return Err(invalid("chrome_tab_id must be a decimal Chrome tab ID"));
            }
            let idempotency_key = args
                .idempotency_key
                .as_deref()
                .filter(|key| {
                    !key.is_empty() && key.len() <= 128 && key.chars().all(|c| c.is_ascii_graphic())
                })
                .ok_or_else(|| {
                    invalid(
                        "activate_owned requires a 1–128 character printable ASCII idempotency_key",
                    )
                })?;
            let shared = self
                .shared_sessions
                .lock()
                .await
                .get(&args.session_id)
                .cloned()
                .ok_or_else(|| invalid("unknown shared session_id"))?;
            let shared = shared.lock().await;
            if shared.handle.principal != self.principal.as_ref() {
                return Err(invalid("session is not owned by this server principal"));
            }
            let connection = shared
                .connection
                .as_ref()
                .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
            let source = connection.selected_targets().iter().find(|candidate| candidate.as_str() != target).cloned()
                .ok_or_else(|| invalid("activate_owned needs another still-open tab in this session to verify the target"))?;
            let receipt = connection
                .command(
                    shared.registry()?,
                    &shared.handle,
                    &source,
                    "Controlla.activateOwnedTab",
                    json!({"owned_tab_id":target,"idempotency_key":idempotency_key}),
                )
                .await
                .map_err(|error| invalid(error.to_string()))?;
            if receipt["status"] != "activated"
                || receipt["topology_verified"] != true
                || receipt["tab_id"] != target
                || receipt["tab_active"] != true
                || receipt["window_focused"] != true
            {
                return Ok(rmcp::handler::server::wrapper::Json(
                    json!({"status":"unknown","reason":"Chrome did not verify activation and focus of the exact Controlla-owned tab","receipt":receipt}),
                ));
            }
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"activated","verified":true,"receipt":receipt}),
            ));
        }
        if args.action != "open" && args.action != "navigate" {
            return Err(invalid(
                "shared_tab action must be open, navigate, close_owned, move_owned, or activate_owned",
            ));
        }
        if args
            .wait_until
            .as_deref()
            .is_some_and(|value| !matches!(value, "document_committed" | "dom_content_loaded"))
        {
            return Err(invalid(
                "wait_until must be document_committed or dom_content_loaded",
            ));
        }
        if args.action != "open" && args.wait_until.is_some() {
            return Err(invalid("wait_until is supported only for action=open"));
        }
        if args.action != "open" && args.active.is_some() {
            return Err(invalid("active is supported only for action=open"));
        }
        if args.action != "open" && args.create_background_window == Some(true) {
            return Err(invalid(
                "background window policy and placement are supported only for action=open",
            ));
        }
        if args.action != "open" && args.action != "move_owned" && args.target_window_id.is_some() {
            return Err(invalid(
                "target_window_id is supported only for action=open or move_owned",
            ));
        }
        if args.create_background_window == Some(true) && args.active.unwrap_or(true) {
            return Err(invalid(
                "create_background_window requires active=false so the created Chrome window remains unfocused",
            ));
        }
        if args.action == "open" && args.idempotency_key.as_deref().is_none_or(str::is_empty) {
            return Err(invalid(
                "shared_tab open requires idempotency_key so retries reuse the same tab",
            ));
        }
        let shared = self
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session_id"))?;
        let mut shared = shared.lock().await;
        if shared.handle.principal != self.principal.as_ref() {
            return Err(invalid("session is not owned by this server principal"));
        }
        let method = if args.action == "open" {
            "Controlla.openTab"
        } else {
            "Controlla.navigate"
        };
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let source = if args.action == "open" {
            let candidates = connection.selected_targets().clone();
            let registry = shared.registry()?;
            let probe = async {
                for target in candidates {
                    if connection
                        .command(
                            registry,
                            &shared.handle,
                            &target,
                            "Page.getFrameTree",
                            Value::Null,
                        )
                        .await
                        .is_ok()
                    {
                        return Some(target);
                    }
                }
                None
            };
            tokio::time::timeout(Duration::from_secs(5), probe)
                .await
                .map_err(|_| invalid("timed out finding an attached source tab"))?
                .ok_or_else(|| invalid("shared session has no currently attached source tab"))?
        } else {
            args.chrome_tab_id
                .clone()
                .ok_or_else(|| invalid("chrome_tab_id is required for navigation"))?
        };
        let result = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?
            .command(
                shared.registry()?,
                &shared.handle,
                &source,
                method,
                json!({"url":args.url,"idempotency_key":args.idempotency_key,"active":args.active.unwrap_or(true),"wait_until":args.wait_until.unwrap_or_else(|| "dom_content_loaded".into()),"create_background_window":args.create_background_window.unwrap_or(false),"target_window_id":args.target_window_id}),
            )
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let tab_id = result["tab_id"]
            .as_str()
            .filter(|id| id.parse::<u32>().is_ok())
            .ok_or_else(|| invalid("extension returned an invalid tab ID"))?
            .to_owned();
        if args.action == "open" {
            let session_id = shared.handle.id.clone();
            shared
                .registry
                .as_mut()
                .ok_or_else(|| invalid("shared registry is unavailable"))?
                .register_tab(&session_id, tab_id.clone(), Ownership::Borrowed)
                .map_err(|e| invalid(format!("cannot register opened tab: {e:?}")))?;
            shared
                .connection
                .as_mut()
                .ok_or_else(|| invalid("shared connection is unavailable"))?
                .add_selected_target(tab_id)
                .map_err(|e| invalid(e.to_string()))?;
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
        if !self.owns_session(&s) {
            return Err(invalid("session is not owned by this server principal"));
        }
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

    #[tool(
        name = "guide",
        description = "Return the version-matched local setup guide. This read-only preview supports clients and master topics."
    )]
    async fn guide(
        &self,
        Parameters(args): Parameters<GuideArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        if args.server_version != env!("CARGO_PKG_VERSION") {
            return Err(invalid(format!(
                "guide is available for server version {} only",
                env!("CARGO_PKG_VERSION")
            )));
        }
        let (guide_version, content) =
            guide_content(&args.topic).ok_or_else(|| invalid("topic must be clients or master"))?;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "server_version":env!("CARGO_PKG_VERSION"),
            "guide_version":guide_version,
            "topic":args.topic,
            "content":content
        })))
    }

    #[tool(
        name = "app_capabilities",
        description = "Read app-specific route gates for Google Slides, Canva, or CapCut Web. This is preflight only and does not connect to or edit an app."
    )]
    async fn app_capabilities(
        &self,
        Parameters(args): Parameters<AppCapabilitiesArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let app = match args.app.as_str() {
            "google_slides" => crate::apps::App::GoogleSlides,
            "canva" => crate::apps::App::Canva,
            "capcut_web" => crate::apps::App::CapCutWeb,
            _ => return Err(invalid("app must be google_slides, canva, or capcut_web")),
        };
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "app": app,
            "live_qualified": false,
            "capabilities": crate::apps::capabilities(app),
            "side_effects": false
        })))
    }

    #[tool(
        name = "slides_plan_text_edit",
        description = "Build a revision-bound Google Slides text-edit API request without dispatching it. OAuth, exact live account/document checks, journaling, independent readback, and persistence verification are not connected."
    )]
    async fn slides_plan_text_edit(
        &self,
        Parameters(args): Parameters<SlidesPlanArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let binding = crate::apps::SlidesBinding {
            principal: self.principal.to_string(),
            account_id: args.account_id,
            presentation_id: args.presentation_id,
            required_revision_id: args.required_revision_id,
        };
        let edit = match args.operation.as_str() {
            "replace_range" => crate::apps::SlidesTextEdit::ReplaceRange {
                object_id: args
                    .object_id
                    .ok_or_else(|| invalid("replace_range requires object_id"))?,
                start_index: args
                    .start_index
                    .ok_or_else(|| invalid("replace_range requires start_index"))?,
                end_index: args
                    .end_index
                    .ok_or_else(|| invalid("replace_range requires end_index"))?,
                expected_text: args.expected_text.ok_or_else(|| {
                    invalid("replace_range requires expected_text for precondition")
                })?,
                new_text: args
                    .new_text
                    .ok_or_else(|| invalid("replace_range requires new_text"))?,
            },
            "insert_into_object" => crate::apps::SlidesTextEdit::InsertIntoObject {
                object_id: args
                    .object_id
                    .ok_or_else(|| invalid("insert_into_object requires object_id"))?,
                text: args
                    .text
                    .ok_or_else(|| invalid("insert_into_object requires text"))?,
                insertion_index: args
                    .insertion_index
                    .ok_or_else(|| invalid("insert_into_object requires insertion_index"))?,
            },
            _ => {
                return Err(invalid(
                    "operation must be replace_range or insert_into_object",
                ));
            }
        };
        let plan = crate::apps::plan_slides_text_edit(&binding, edit).map_err(invalid)?;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "status":"planned_not_dispatched",
            "plan":plan,
            "evidence":[],
            "app_acceptance":"not_established"
        })))
    }

    #[tool(
        name = "canva_sync_preflight",
        description = "Validate a Canva app-session snapshot before a potential sync. Sync is treated as an external write; this tool never opens a Canva session or calls sync."
    )]
    async fn canva_sync_preflight(
        &self,
        Parameters(args): Parameters<CanvaPlanArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let page_type = match args.page_type.as_str() {
            "absolute" => crate::apps::CanvaPageType::Absolute,
            "unsupported" => crate::apps::CanvaPageType::Unsupported,
            _ => return Err(invalid("page_type must be absolute or unsupported")),
        };
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| invalid(error.to_string()))?
            .as_millis() as u64;
        let snapshot = crate::apps::CanvaSessionSnapshot {
            principal: self.principal.to_string(),
            account_id: args.account_id,
            design_id: args.design_id,
            page_id: args.page_id,
            revision: args.revision,
            page_type,
            locked: args.locked,
            opened_at_ms: args.opened_at_ms,
        };
        let plan = crate::apps::plan_canva_sync(&snapshot, now_ms).map_err(invalid)?;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "status":"preflight_only",
            "plan":plan,
            "evidence":[],
            "app_acceptance":"not_established"
        })))
    }

    #[tool(
        name = "capcut_web_plan",
        description = "Check whether a bounded CapCut Web recipe is currently available. Operations remain unsupported until a live versioned control map and independent verifier are qualified."
    )]
    async fn capcut_web_plan(
        &self,
        Parameters(args): Parameters<CapCutPlanArgs>,
    ) -> rmcp::handler::server::wrapper::Json<Value> {
        let result = crate::apps::capcut_web_plan(&args.operation);
        rmcp::handler::server::wrapper::Json(json!({
            "operation":args.operation,
            "status":"unsupported",
            "reason":result.err().unwrap_or("operation requires live qualification"),
            "side_effects":false
        }))
    }

    #[tool(
        name = "slides_deck_plan",
        description = "Compile a candidate 10-slide Google Slides API request from caller assertions. These assertions are not authoritative or authenticated; the candidate is not executable until a live route verifies the entire deck inventory and revision. No OAuth, network request, write, readback, or export occurs."
    )]
    async fn slides_deck_plan(
        &self,
        Parameters(args): Parameters<SlidesDeckPlanArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let binding = crate::apps::SlidesBinding {
            principal: self.principal.to_string(),
            account_id: args.account_id,
            presentation_id: args.presentation_id,
            required_revision_id: args.required_revision_id,
        };
        let start = crate::slides_deck::SlidesDeckStart {
            asserted_existing_slide_ids: args.asserted_existing_slide_ids,
            asserted_existing_object_ids: args.asserted_existing_object_ids,
        };
        let plan =
            crate::slides_deck::compile_urban_heat_deck(&binding, &start).map_err(invalid)?;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "status":"planned_not_dispatched",
            "plan":plan,
            "preconditions_authoritative":false,
            "side_effects":false,
            "app_acceptance":"not_established"
        })))
    }

    #[tool(
        name = "canva_design_plan",
        description = "Compile the five-page Heat-Ready Canva brief using unverified caller assertions for identity, session, version, expiry, and page state. These checks are advisory only and cannot authorize a sync. No Canva SDK/OAuth call or app mutation occurs."
    )]
    async fn canva_design_plan(
        &self,
        Parameters(args): Parameters<CanvaDesignPlanArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| invalid(error.to_string()))?
            .as_millis() as u64;
        let binding = crate::canva::CanvaPlanBinding {
            identity: args.binding.identity.into_identity(&self.principal),
            session_id: args.binding.session_id,
            expected_version: args.binding.expected_version,
        };
        let session = crate::canva::CanvaSession {
            identity: args.session.identity.into_identity(&self.principal),
            session_id: args.session.session_id,
            current_version: args.session.current_version,
            opened_at_ms: args.session.opened_at_ms,
            expires_at_ms: args.session.expires_at_ms,
            page_type: args.session.page_type,
            locked: args.session.locked,
        };
        let plan =
            crate::canva::plan_heat_ready_design(&binding, &session, now_ms).map_err(invalid)?;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "status":"planned_not_dispatched",
            "plan":plan,
            "preconditions_authoritative":false,
            "side_effects":false,
            "app_acceptance":"not_established"
        })))
    }

    #[tool(
        name = "capcut_recipe_plan",
        description = "Validate owned/licensed asset metadata and compile a bounded captioned-video timeline description. Offline planning only; no CapCut selectors, media transfer, or editor action."
    )]
    async fn capcut_recipe_plan(
        &self,
        Parameters(args): Parameters<CapCutRecipePlanArgs>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, rmcp::ErrorData> {
        let plan = crate::capcut_recipe::compile_capcut_recipe(&args.recipe).map_err(invalid)?;
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "status":"offline_plan_only",
            "plan":plan,
            "side_effects":false,
            "app_acceptance":"not_established"
        })))
    }
}

#[tool_handler]
impl ServerHandler for App {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(bootstrap_instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= rmcp::model::ProtocolVersion::V_2026_07_28);
        let mut tools = Self::tool_router().list_all();
        for tool in &mut tools {
            if let Some(schema) = &mut tool.output_schema {
                let schema = Arc::make_mut(schema);
                if !schema.contains_key("type") {
                    schema.insert("type".to_owned(), Value::String("object".to_owned()));
                }
            }
        }
        Ok(rmcp::model::ListToolsResult {
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(rmcp::model::CacheScope::Public),
        })
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, rmcp::ErrorData> {
        let resources = ["clients", "master"]
            .into_iter()
            .map(|topic| {
                let uri = guide_resource_uri(topic, env!("CARGO_PKG_VERSION"));
                let (guide_version, content) = guide_content(topic).expect("known guide topic");
                rmcp::model::Resource::new(uri, format!("Chrome Controlla {topic} guide"))
                    .with_title(format!("Chrome Controlla {topic} guide"))
                    .with_description(format!(
                        "Read-only {guide_version} instructions for server version {}",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .with_mime_type("text/markdown")
                    .with_size(content.len() as u64)
            })
            .collect();
        Ok(rmcp::model::ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResponse, rmcp::ErrorData> {
        let Some((topic, version)) = parse_guide_resource_uri(&request.uri) else {
            return Err(rmcp::ErrorData::resource_not_found(
                "guide resource is unavailable",
                None,
            ));
        };
        if version != env!("CARGO_PKG_VERSION") {
            return Err(rmcp::ErrorData::resource_not_found(
                "guide resource is unavailable for this server version",
                None,
            ));
        }
        let Some((_guide_version, content)) = guide_content(topic) else {
            return Err(rmcp::ErrorData::resource_not_found(
                "guide topic is unavailable",
                None,
            ));
        };
        let contents = rmcp::model::ResourceContents::text(content, request.uri)
            .with_mime_type("text/markdown");
        Ok(rmcp::model::ReadResourceResponse::Complete(
            rmcp::model::ReadResourceResult::new(vec![contents]),
        ))
    }
}

fn bootstrap_instructions() -> String {
    let registered = App::tool_router()
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Cache tools/list schemas. Pair selected tabs once; snapshot then click returned references. Never replay unknown effects. Read guide as needed. Tools: {registered}."
    )
}

fn guide_resource_uri(topic: &str, server_version: &str) -> String {
    format!("controlla://guide/{topic}/{server_version}")
}

fn parse_guide_resource_uri(uri: &str) -> Option<(&str, &str)> {
    let suffix = uri.strip_prefix("controlla://guide/")?;
    let (topic, version) = suffix.split_once('/')?;
    if topic.is_empty() || version.is_empty() || version.contains('/') {
        return None;
    }
    Some((topic, version))
}

fn guide_content(topic: &str) -> Option<(&'static str, &'static str)> {
    match topic {
        "clients" => Some((
            "clients-2026-10-06-v1",
            include_str!("../../../docs/clients.md"),
        )),
        "master" => Some((
            "master-2026-10-08-v6",
            include_str!("../../../docs/MASTER_GUIDE.md"),
        )),
        _ => None,
    }
}

fn fit_aggregate(mut output: Value, max_bytes: usize) -> Result<Value, String> {
    // Top-level unique_count describes records returned; each section's count
    // retains unique rows observed by that section before aggregate truncation.
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

fn validate_shared_url(url: &str) -> Result<(), rmcp::ErrorData> {
    let uri = url
        .parse::<axum::http::Uri>()
        .map_err(|_| invalid("URL must be an absolute HTTP(S) URL without credentials"))?;
    let authority = uri
        .authority()
        .ok_or_else(|| invalid("URL must include an HTTP(S) authority"))?;
    let scheme = uri.scheme_str().unwrap_or_default();
    let host = authority.host();
    let authority_suffix = authority
        .as_str()
        .strip_prefix(host)
        .unwrap_or(authority.as_str());
    let valid_host = match host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        Some(ipv6) => ipv6.parse::<std::net::Ipv6Addr>().is_ok(),
        None => !host.is_empty() && !host.contains('[') && !host.contains(']'),
    };
    let valid_port = authority_suffix.is_empty()
        || authority_suffix.strip_prefix(':').is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok()
        });
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || !valid_host
        || authority.as_str().contains('@')
        || !valid_port
        || url.len() > 2048
        || url.chars().any(char::is_whitespace)
    {
        return Err(invalid(
            "URL must be a bounded, absolute HTTP(S) URL without credentials",
        ));
    }
    Ok(())
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

pub(crate) fn acquire_state_directory_lock(
    state_dir: &std::path::Path,
) -> Result<std::fs::File, std::io::Error> {
    let lock_path = state_dir.join("server.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "state directory is already owned by another Chrome Controlla process; set CONTROLLA_STATE_DIR to a separate directory for concurrent clients ({})",
                state_dir.display()
            ),
        )),
        Err(std::fs::TryLockError::Error(error)) => Err(std::io::Error::new(
            error.kind(),
            format!(
                "unable to lock state directory {}: {error}",
                state_dir.display()
            ),
        )),
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let state_dir = state_directory();
    std::fs::create_dir_all(&state_dir)?;
    let _state_lock = acquire_state_directory_lock(&state_dir)?;
    let journal = crate::jobs::Journal::open(state_dir.join("operations.sqlite"))?;
    journal.recover_after_restart()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        App::with_journal(journal)
            .serve(rmcp::transport::io::stdio())
            .await?
            .waiting()
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

pub(crate) fn state_directory() -> std::path::PathBuf {
    std::env::var_os("CONTROLLA_STATE_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".chrome-controlla"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".chrome-controlla"))
}

#[cfg(test)]
mod tests {
    use super::{
        App, LOCAL_STDIO_PRINCIPAL, SHARED_CLICK_UNSAFE_PREDICATE, SHARED_TYPE_GUARD_FUNCTION,
        SharedOperationRead, SharedPageSnapshot, click_mouse_events,
        final_click_guard_matches_viewport, pointer_click_events, pointer_events,
        shared_command_value, shared_epoch_matches, shared_identity_epoch_batch,
        shared_outcome_identity_allows_verification, shared_typing_minimum_duration,
        validate_loopback_ws, validate_pinch, validate_swipe_endpoints, validate_wheel_delta,
    };
    use rmcp::{
        RoleServer, ServerHandler, ServiceExt, model::CallToolRequestParams,
        service::serve_directly,
    };
    use serde_json::{Value, json};

    #[test]
    fn wheel_delta_requires_bounded_nonzero_integer_movement() {
        assert!(validate_wheel_delta(0, 120).is_ok());
        assert!(validate_wheel_delta(-120, 0).is_ok());
        assert!(validate_wheel_delta(0, 0).is_err());
        assert!(validate_wheel_delta(1201, 0).is_err());
        assert!(validate_wheel_delta(0, -1201).is_err());
        assert!(validate_wheel_delta(i32::MIN, 0).is_err());
    }

    #[test]
    fn swipe_endpoints_require_finite_bounded_viewport_path_length() {
        assert!(validate_swipe_endpoints(20.0, 40.0, 20.0, 10.0).is_ok());
        assert!(validate_swipe_endpoints(0.0, 0.0, 20.0, 0.0).is_ok());
        assert!(validate_swipe_endpoints(1.0, 1.0, 10.0, 10.0).is_err());
        assert!(validate_swipe_endpoints(0.0, 0.0, 1201.0, 0.0).is_err());
        assert!(validate_swipe_endpoints(f64::NAN, 0.0, 100.0, 0.0).is_err());
        assert!(validate_swipe_endpoints(10_001.0, 1.0, 100.0, 1.0).is_err());
    }

    #[test]
    fn multi_touch_pinch_requires_bounded_geometry_and_scale_ratio() {
        assert!(validate_pinch(200.0, 300.0, 80.0, 120.0).is_ok());
        assert!(validate_pinch(f64::NAN, 300.0, 80.0, 120.0).is_err());
        assert!(validate_pinch(200.0, 300.0, 30.0, 60.0).is_err());
        assert!(validate_pinch(200.0, 300.0, 80.0, 170.0).is_err());
        assert!(validate_pinch(10_001.0, 300.0, 80.0, 120.0).is_err());
    }

    #[test]
    fn multi_touch_outcome_verification_rejects_replacement_documents() {
        let original = ("https://app.test".into(), "frame-a".into(), "doc-1".into());
        let same = original.clone();
        let replacement = ("https://app.test".into(), "frame-a".into(), "doc-2".into());
        let visible = super::SharedClickOutcome::Visible {
            selector: "#ready".into(),
        };
        let navigation = super::SharedClickOutcome::Navigation { url: None };
        assert!(shared_outcome_identity_allows_verification(
            &visible, &original, &same
        ));
        assert!(!shared_outcome_identity_allows_verification(
            &visible,
            &original,
            &replacement
        ));
        assert!(shared_outcome_identity_allows_verification(
            &navigation,
            &original,
            &replacement
        ));
    }

    #[test]
    fn coalesced_identity_epoch_batch_requires_ordered_complete_receipts() {
        let valid = json!({
            "completed": 2,
            "receipts": [
                {"method":"Runtime.callFunctionOn","result":{"result":{"value":{"semantic_revision":7}}}},
                {"method":"Page.getFrameTree","result":{"frameTree":{"frame":{"id":"frame-1","loaderId":"loader-1","url":"https://app.test"}}}}
            ]
        });
        let (identity, epoch) = shared_identity_epoch_batch(&valid).unwrap();
        assert_eq!(identity.1, "loader-1");
        assert_eq!(epoch["semantic_revision"], 7);
        let wrong_order = json!({"completed":2,"receipts":[valid["receipts"][1].clone(),valid["receipts"][0].clone()]});
        assert!(shared_identity_epoch_batch(&wrong_order).is_err());
    }

    #[test]
    fn operation_read_record_reuses_only_matching_identity_and_epoch() {
        let identity = (
            "frame-1".into(),
            "loader-1".into(),
            "https://app.test".into(),
        );
        let epoch = json!({"semantic_revision":7,"viewport":"800x600"});
        let read = SharedOperationRead::new(identity.clone(), epoch.clone());
        assert!(read.remains_fresh(&identity, &epoch));
        assert!(!read.remains_fresh(
            &(identity.0.clone(), "loader-2".into(), identity.2.clone()),
            &epoch
        ));
        assert!(!read.remains_fresh(
            &identity,
            &json!({"semantic_revision":8,"viewport":"800x600"})
        ));
    }

    #[test]
    fn scroll_receipt_requires_same_connected_container_and_expected_direction() {
        let receipt = json!({"same_container":true,"container_connected":true,
            "before":{"left":0.0,"top":20.0},"after":{"left":0.0,"top":80.0}});
        assert!(super::valid_scroll_readback(&receipt, 0, 60));
        assert!(!super::valid_scroll_readback(&receipt, 0, -60));
        let replaced = json!({"same_container":false,"container_connected":true,
            "before":{"left":0.0,"top":20.0},"after":{"left":0.0,"top":80.0}});
        assert!(!super::valid_scroll_readback(&replaced, 0, 60));
    }

    #[test]
    fn custom_shared_command_receipts_are_not_parsed_as_cdp_runtime_values() {
        let receipt = json!({"status":"dispatched","dispatch_acknowledged":true,"temporary_activation":true,"restored_previous_tab":true});
        assert_eq!(shared_command_value(&receipt), receipt);
        let cdp = json!({"result":{"value":{"status":"verified"}}});
        assert_eq!(shared_command_value(&cdp), json!({"status":"verified"}));
    }

    #[test]
    fn final_click_guard_requires_a_live_hit_test_and_stable_viewport() {
        let checked = json!({"ok":true,"x":120.5,"y":80.0,"viewport":"800x600@0,0"});
        assert!(final_click_guard_matches_viewport(
            &checked,
            Some("800x600@0,0")
        ));
        assert!(!final_click_guard_matches_viewport(
            &checked,
            Some("800x600@0,1")
        ));
        assert!(!final_click_guard_matches_viewport(
            &json!({"ok":false,"x":120.5,"y":80.0,"viewport":"800x600@0,0"}),
            Some("800x600@0,0")
        ));
        assert!(!final_click_guard_matches_viewport(
            &json!({"ok":true,"x":"120.5","y":80.0,"viewport":"800x600@0,0"}),
            Some("800x600@0,0")
        ));
    }

    #[test]
    fn semantic_click_dispatches_at_verified_point_without_a_separate_pointer_move() {
        let events = click_mouse_events(12.0, 34.0);
        assert_eq!(events[0]["type"], "mousePressed");
        assert_eq!(events[0]["x"], 12.0);
        assert_eq!(events[0]["y"], 34.0);
        assert_eq!(events[1]["type"], "mouseReleased");
        assert_eq!(events[1]["x"], 12.0);
        assert_eq!(events[1]["y"], 34.0);
    }

    #[test]
    fn double_and_context_clicks_emit_explicit_trusted_mouse_event_sequences() {
        let double = pointer_click_events(12.0, 34.0, "double_click").unwrap();
        assert_eq!(double.len(), 4);
        assert_eq!(
            double
                .iter()
                .map(|event| event["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "mousePressed",
                "mouseReleased",
                "mousePressed",
                "mouseReleased"
            ]
        );
        assert_eq!(double[0]["button"], "left");
        assert_eq!(double[0]["clickCount"], 1);
        assert_eq!(double[2]["clickCount"], 2);
        assert!(
            double
                .iter()
                .all(|event| event["x"] == 12.0 && event["y"] == 34.0)
        );
        let context = pointer_click_events(8.0, 9.0, "context_click").unwrap();
        assert_eq!(context.len(), 2);
        assert!(
            context
                .iter()
                .all(|event| event["button"] == "right" && event["clickCount"] == 1)
        );
        assert!(pointer_click_events(0.0, 0.0, "hover").is_err());
    }

    #[test]
    fn guarded_touch_tap_uses_touch_start_and_end_without_mouse_fallback() {
        let (method, events) = pointer_events(12.0, 34.0, "touch").unwrap();
        assert_eq!(method, "Input.dispatchTouchEvent");
        assert_eq!(events[0]["type"], "touchStart");
        assert_eq!(events[0]["touchPoints"][0]["x"], 12.0);
        assert_eq!(events[0]["touchPoints"][0]["y"], 34.0);
        assert_eq!(events[1]["type"], "touchEnd");
        assert_eq!(events[1]["touchPoints"].as_array().unwrap().len(), 0);
        assert!(pointer_events(12.0, 34.0, "keyboard").is_err());
    }

    #[tokio::test]
    async fn internal_workflow_reuse_is_scoped_to_its_successfully_consumed_generation() {
        assert!(!super::internal_workflow_snapshot_allowed("external"));
        super::with_internal_workflow_state(async {
            assert!(!super::internal_workflow_snapshot_allowed("external"));
            super::authorize_internal_workflow_snapshot("new");
            assert!(super::internal_workflow_snapshot_allowed("new"));
            assert!(!super::internal_workflow_snapshot_allowed("external"));
        })
        .await;
        assert!(!super::internal_workflow_snapshot_allowed("new"));
    }

    #[test]
    fn dedicated_batch_read_guard_accepts_only_explicit_true() {
        assert!(super::dedicated_batch_guard_ok(
            &json!({"result":{"value":{"ok":true}}})
        ));
        assert!(!super::dedicated_batch_guard_ok(
            &json!({"result":{"value":{}}})
        ));
        assert!(!super::dedicated_batch_guard_ok(
            &json!({"result":{"value":{"ok":null}}})
        ));
        assert!(!super::dedicated_batch_guard_ok(
            &json!({"result":{"value":{"ok":false}}})
        ));
        assert!(!super::dedicated_batch_guard_ok(
            &json!({"error":{"message":"failed"}})
        ));
    }

    #[test]
    fn navigation_outcomes_match_only_changed_documents_and_absolute_exact_urls() {
        use super::{SharedClickOutcome, shared_navigation_matches, validate_shared_click_outcome};

        let before = (
            "frame".into(),
            "old-loader".into(),
            "https://site.test/old".into(),
        );
        let target = (
            "frame".into(),
            "new-loader".into(),
            "https://site.test/story/1".into(),
        );
        let spa_route = (
            "frame".into(),
            "old-loader".into(),
            "https://site.test/story/1".into(),
        );
        assert!(
            shared_navigation_matches(
                &SharedClickOutcome::Navigation { url: None },
                &before,
                &target,
            )
            .unwrap()
        );
        assert!(
            shared_navigation_matches(
                &SharedClickOutcome::Navigation { url: None },
                &before,
                &spa_route,
            )
            .unwrap(),
            "same-document SPA URL changes count as navigation"
        );
        assert!(
            !shared_navigation_matches(
                &SharedClickOutcome::Navigation { url: None },
                &before,
                &before,
            )
            .unwrap()
        );
        assert!(
            shared_navigation_matches(
                &SharedClickOutcome::Navigation {
                    url: Some(target.2.clone())
                },
                &before,
                &target,
            )
            .unwrap()
        );
        for (expected, actual) in [
            ("https://site.test/story/1", "https://site.test/story/1"),
            ("HTTPS://SITE.TEST:443", "https://site.test/"),
            ("http://SITE.test:80/", "http://site.test/"),
        ] {
            let actual = ("frame".into(), "new-loader".into(), actual.into());
            assert!(
                shared_navigation_matches(
                    &SharedClickOutcome::Navigation {
                        url: Some(expected.into())
                    },
                    &before,
                    &actual,
                )
                .unwrap(),
                "browser-equivalent URLs must match: {expected} -> {}",
                actual.2
            );
        }
        assert!(
            !shared_navigation_matches(
                &SharedClickOutcome::Navigation {
                    url: Some("https://site.test/story/".into())
                },
                &before,
                &target,
            )
            .unwrap()
        );
        assert!(
            shared_navigation_matches(
                &SharedClickOutcome::Navigation {
                    url: Some("/story/1".into())
                },
                &before,
                &target,
            )
            .is_err()
        );
        assert!(
            validate_shared_click_outcome(&SharedClickOutcome::Navigation {
                url: Some("/story/1".into())
            })
            .is_err()
        );
        for malformed in [
            "https://example.test:bad",
            "https://example.test:65536",
            "https://example.test:",
            "https://[::1",
            "https://[:::]/",
            "https://[gggg::1]/",
            "http:///missing-host",
        ] {
            assert!(
                validate_shared_click_outcome(&SharedClickOutcome::Navigation {
                    url: Some(malformed.into())
                })
                .is_err(),
                "malformed exact destination must be rejected before dispatch: {malformed}"
            );
        }
    }

    #[test]
    fn click_unknown_reasons_are_machine_readable_and_never_retryable() {
        use super::click_unknown_reason_code;
        assert_eq!(
            click_unknown_reason_code(true),
            "outcome_verification_timeout"
        );
        assert_eq!(click_unknown_reason_code(false), "click_dispatch_uncertain");
    }

    #[test]
    fn new_tab_outcome_requires_a_recent_child_tab_and_optional_exact_url() {
        use super::{shared_new_tab_candidate, shared_new_tab_matches};
        let response = json!({"tabs":[{"tab_id":"900","url":"https://site.test/story","window_id":5,"created_at_ms":1200,"opener_match":true}]});
        assert!(shared_new_tab_matches(&response, None));
        assert!(shared_new_tab_matches(
            &response,
            Some("https://site.test/story")
        ));
        let candidate = shared_new_tab_candidate(&response, Some("https://site.test/story"))
            .expect("unique opener-bound candidate is exposed");
        assert_eq!(candidate["tab_id"], "900");
        assert_eq!(candidate["created_at_ms"], 1200);
        assert_eq!(
            candidate["evidence_scope"],
            "recent_opener_bound_tab_candidate_not_account_authentication"
        );
        assert!(!shared_new_tab_matches(
            &response,
            Some("https://site.test/other")
        ));
        assert!(!shared_new_tab_matches(&json!({"tabs":[]}), None));
        assert!(!shared_new_tab_matches(
            &json!({"tabs":[{"tab_id":"1","url":"https://site.test/story","opener_match":false}]}),
            None
        ));
        assert!(!shared_new_tab_matches(
            &json!({"tabs":[{"tab_id":"1","url":"https://site.test/story","opener_match":false}]}),
            Some("https://site.test/story")
        ));
        assert!(!shared_new_tab_matches(
            &json!({"tabs":[{"tab_id":"1","url":"https://site.test/story"}]}),
            Some("https://site.test/story")
        ));
        assert!(
            !shared_new_tab_matches(
                &json!({"tabs":[{"tab_id":"1","url":"https://site.test/story","opener_match":true}]}),
                Some("https://site.test/story")
            ),
            "candidate without creation-time evidence cannot be adopted"
        );
        assert!(
            !shared_new_tab_matches(
                &json!({"tabs":[
                    {"tab_id":"1","url":"https://site.test/story","created_at_ms":1200,"opener_match":true},
                    {"tab_id":"2","url":"https://site.test/story","created_at_ms":1201,"opener_match":true}
                ]}),
                Some("https://site.test/story")
            ),
            "ambiguous opener-bound candidates cannot be adopted"
        );
        assert!(!shared_new_tab_matches(
            &json!({"tabs":[{"tab_id":"1","url":"https://site.test/a"},{"tab_id":"2","url":"https://site.test/b"}]}),
            None
        ));
        assert!(!shared_new_tab_matches(
            &json!({"tabs":[{"url":"https://site.test/story"}]}),
            None
        ));
    }

    #[test]
    fn popup_adoption_requires_unique_fresh_candidate_and_visible_exact_account_marker() {
        use super::{popup_account_marker_matches, popup_candidate_for_adoption};
        let response = json!({"tabs":[{"tab_id":"900","url":"https://accounts.example.test/pick","window_id":5,"created_at_ms":1200,"opener_tab_id":"41","opener_match":true}]});
        let candidate = popup_candidate_for_adoption(
            &response,
            "41",
            "900",
            1200,
            1100,
            "https://accounts.example.test/pick",
            1250,
        )
        .unwrap();
        assert_eq!(candidate["tab_id"], "900");
        assert!(
            popup_candidate_for_adoption(
                &response,
                "41",
                "900",
                1200,
                1201,
                "https://accounts.example.test/pick",
                1250
            )
            .is_err(),
            "candidate predating the click is stale"
        );
        assert!(
            popup_candidate_for_adoption(
                &response,
                "41",
                "900",
                1200,
                1100,
                "https://accounts.example.test/pick",
                62_000
            )
            .is_err(),
            "candidate outside the short-lived extension event window is stale"
        );
        assert!(
            popup_candidate_for_adoption(
                &response,
                "41",
                "901",
                1200,
                1100,
                "https://accounts.example.test/pick",
                1250
            )
            .is_err(),
            "closed or unknown candidate is rejected"
        );
        assert!(
            popup_candidate_for_adoption(
                &response,
                "41",
                "900",
                1200,
                1100,
                "https://accounts.example.test/other",
                1250
            )
            .is_err(),
            "unrelated destination is rejected"
        );
        let ambiguous = json!({"tabs":[response["tabs"][0].clone(),{"tab_id":"901","url":"https://accounts.example.test/pick","created_at_ms":1201,"opener_tab_id":"41","opener_match":true}]});
        assert!(
            popup_candidate_for_adoption(
                &ambiguous,
                "41",
                "900",
                1200,
                1100,
                "https://accounts.example.test/pick",
                1250
            )
            .is_err(),
            "ambiguous popup set is rejected"
        );
        let wrong_opener = json!({"tabs":[{"tab_id":"900","url":"https://accounts.example.test/pick","created_at_ms":1200,"opener_tab_id":"99","opener_match":false}]});
        assert!(
            popup_candidate_for_adoption(
                &wrong_opener,
                "41",
                "900",
                1200,
                1100,
                "https://accounts.example.test/pick",
                1250
            )
            .is_err()
        );
        let snapshot = json!({"url":"https://accounts.example.test/pick","scope_complete":true,"items":[{"name":"Signed in as test@example.test","text":"Signed in as test@example.test","rendered":true,"in_viewport":true}]});
        assert!(popup_account_marker_matches(
            &snapshot,
            "https://accounts.example.test/pick",
            "Signed in as test@example.test"
        ));
        assert!(!popup_account_marker_matches(
            &snapshot,
            "https://accounts.example.test/other",
            "Signed in as test@example.test"
        ));
        assert!(!popup_account_marker_matches(
            &snapshot,
            "https://accounts.example.test/pick",
            "Signed in as other@example.test"
        ));
        let hidden = json!({"url":"https://accounts.example.test/pick","scope_complete":true,"items":[{"name":"Signed in as test@example.test","rendered":false,"in_viewport":false}]});
        assert!(!popup_account_marker_matches(
            &hidden,
            "https://accounts.example.test/pick",
            "Signed in as test@example.test"
        ));
        let truncated = json!({"url":"https://accounts.example.test/pick","scope_complete":false,"items":snapshot["items"].clone()});
        assert!(!popup_account_marker_matches(
            &truncated,
            "https://accounts.example.test/pick",
            "Signed in as test@example.test"
        ));
        let duplicate = json!({"url":"https://accounts.example.test/pick","scope_complete":true,"items":[snapshot["items"][0].clone(),snapshot["items"][0].clone()]});
        assert!(!popup_account_marker_matches(
            &duplicate,
            "https://accounts.example.test/pick",
            "Signed in as test@example.test"
        ));
    }

    #[test]
    fn shared_tab_schema_requires_idempotency_key_only_for_open() {
        let schema =
            serde_json::to_value(rmcp::schemars::schema_for!(super::SharedTabArgs)).unwrap();
        assert_eq!(
            schema["allOf"][0]["if"]["properties"]["action"]["const"],
            "open"
        );
        assert!(
            schema["allOf"][0]["then"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|property| property == "idempotency_key")
        );
        assert!(
            schema["properties"]["active"]["type"]
                .as_array()
                .unwrap()
                .iter()
                .any(|kind| kind == "boolean")
        );
        for field in [
            "candidate_tab_id",
            "created_at_ms",
            "since_ms",
            "account_marker_selector",
            "account_marker_text",
        ] {
            assert!(
                schema["properties"].get(field).is_some(),
                "shared_tab exposes the bounded {field} input for adopt_popup"
            );
            assert!(
                !schema["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|required| required == field),
                "adopt_popup-only input {field} stays optional for existing actions"
            );
        }
    }

    #[test]
    fn interaction_epoch_guard_binds_click_to_snapshot_and_document_identity() {
        let interaction = json!({"semantic_revision":4,"focus":"body","selection":"collapsed:0","viewport":"800x600@0,0"});
        let snapshot = SharedPageSnapshot {
            token: "snapshot-7".into(),
            object_id: "obj".into(),
            identity: (
                "frame".into(),
                "loader".into(),
                "https://example.test/a".into(),
            ),
            interaction_epoch: interaction.clone(),
            count: 1,
            consumed: false,
        };
        assert!(shared_epoch_matches(
            &snapshot,
            "tab-1",
            &snapshot.identity,
            &interaction,
            crate::v3::EpochUse::PointerMutation
        ));
        assert!(!shared_epoch_matches(
            &snapshot,
            "tab-1",
            &(
                "other-frame".into(),
                "loader".into(),
                "https://example.test/a".into()
            ),
            &interaction,
            crate::v3::EpochUse::PointerMutation
        ));
        assert!(!shared_epoch_matches(
            &snapshot,
            "tab-1",
            &(
                "frame".into(),
                "new-loader".into(),
                "https://example.test/a".into()
            ),
            &interaction,
            crate::v3::EpochUse::TextMutation
        ));
        assert!(!shared_epoch_matches(
            &snapshot,
            "tab-1",
            &snapshot.identity,
            &json!({"semantic_revision":5,"focus":"body","selection":"collapsed:0","viewport":"800x600@0,0"}),
            crate::v3::EpochUse::PointerMutation
        ));
        assert!(shared_epoch_matches(
            &snapshot,
            "tab-1",
            &snapshot.identity,
            &json!({"semantic_revision":5,"focus":"body","selection":"collapsed:0","viewport":"800x600@0,0"}),
            crate::v3::EpochUse::GuardedClick
        ));
        assert!(!shared_epoch_matches(
            &snapshot,
            "tab-1",
            &snapshot.identity,
            &json!({"semantic_revision":5,"focus":"body","selection":"collapsed:0","viewport":"900x600@0,0"}),
            crate::v3::EpochUse::GuardedClick
        ));
        assert!(!shared_epoch_matches(
            &snapshot,
            "tab-1",
            &snapshot.identity,
            &json!({"semantic_revision":4,"focus":"input#search","selection":"collapsed:0","viewport":"800x600@0,0"}),
            crate::v3::EpochUse::TextMutation
        ));
        assert!(!shared_epoch_matches(
            &snapshot,
            "tab-1",
            &snapshot.identity,
            &json!({"semantic_revision":4,"focus":"body","selection":"range:1-4","viewport":"800x600@0,0"}),
            crate::v3::EpochUse::TextMutation
        ));
        assert!(!shared_epoch_matches(
            &snapshot,
            "tab-1",
            &snapshot.identity,
            &json!({"semantic_revision":4,"focus":"body","selection":"collapsed:0","viewport":"900x600@0,0"}),
            crate::v3::EpochUse::PointerMutation
        ));
    }

    #[test]
    fn snapshot_batch_requires_both_successful_reads_and_a_root_identity() {
        let valid = json!({"completed":2,"receipts":[
            {"method":"Runtime.callFunctionOn","result":{"result":{"value":{"items":[]}}}},
            {"method":"Page.getFrameTree","result":{"frameTree":{"frame":{"id":"f","loaderId":"l","url":"https://example.test/","securityOrigin":"https://example.test"},"childFrames":[{"frame":{"id":"child","parentId":"f","loaderId":"cl","url":"https://child.example.test/","securityOrigin":"https://child.example.test"}}]}}}
        ]});
        let reads = super::snapshot_batch_reads(&valid, "tab-1").unwrap();
        assert_eq!(reads.identity.2, "https://example.test/");
        assert_eq!(reads.frame_handles.len(), 2);
        assert_eq!(reads.frame_handles[1].frame_id, "child");
        assert_eq!(reads.frame_handles[0].target_id.as_deref(), Some("tab-1"));
        assert_eq!(reads.frame_handles[1].target_id, None);
        assert_eq!(reads.frame_handles[1].parent_id.as_deref(), Some("f"));
        assert_eq!(reads.frame_handles[1].session_id, None);
        assert!(
            super::snapshot_batch_reads(
                &json!({"completed":1,"receipts":[valid["receipts"][0]]}),
                "tab-1"
            )
            .is_err()
        );
        assert!(
            super::snapshot_batch_reads(
                &json!({"completed":2,"batch_error":"navigation","receipts":valid["receipts"]}),
                "tab-1"
            )
            .is_err()
        );
    }

    #[test]
    fn frame_handles_are_bounded_and_do_not_invent_debugger_sessions_or_geometry() {
        let tree = json!({"frameTree":{"frame":{"id":"root","loaderId":"root-load","url":"https://example.test/"},"childFrames":[{"frame":{"id":"opaque","parentId":"root","loaderId":"opaque-load","securityOrigin":"://","url":"about:blank"},"childFrames":[{"frame":{"id":"deep","loaderId":"deep-load","securityOrigin":"null"}}]}]}});
        let (handles, truncated) = super::frame_handles_from_tree(&tree, "selected-tab").unwrap();
        assert!(!truncated);
        assert_eq!(handles.len(), 3);
        assert_eq!(handles[2].parent_id.as_deref(), Some("opaque"));
        assert_eq!(handles[1].origin.as_deref(), Some("://"));
        assert_eq!(handles[1].session_id, None);
        assert_eq!(handles[1].transform_revision, 0);
        assert_eq!(handles[1].accessibility_state, "not_probed");
        let public = super::public_frame_handles(&handles).unwrap();
        assert_eq!(public[0]["target_id"], "selected-tab");
        assert_eq!(
            public[1]["target_id"],
            Value::Null,
            "frame-tree ancestry does not prove OOPIF target ownership"
        );
        assert_eq!(public[1]["route"], "frame_id");
        assert_eq!(public[1]["session_id"], Value::Null);
        assert_eq!(public[1]["transform_revision"], Value::Null);
        assert_eq!(public[1]["transform_state"], "unmeasured");
        let snapshot = json!({"items":[],"frame_handles":public});
        super::ensure_snapshot_byte_budget(&snapshot, 4096).unwrap();
        let many = (0..256)
            .map(|index| controlla_browser::FrameHandle {
                target_id: Some("tab".to_owned()),
                session_id: None,
                frame_id: format!("frame-{index:03}-abcdefghijklmnopqrstuvwxyz"),
                loader_id: Some("loader-abcdefghijklmnopqrstuvwxyz".to_owned()),
                parent_id: None,
                origin: Some("https://example.test".to_owned()),
                transform_revision: 0,
                accessibility_state: "not_probed".to_owned(),
            })
            .collect::<Vec<_>>();
        let oversized =
            json!({"items":[],"frame_handles":super::public_frame_handles(&many).unwrap()});
        assert!(super::ensure_snapshot_byte_budget(&oversized, 4096).is_err());
    }

    #[test]
    fn frame_handle_collection_marks_missing_ids_depth_and_count_truncation() {
        let missing_id = json!({"frameTree":{"frame":{"id":"root","loaderId":"l"},"childFrames":[{"frame":{"loaderId":"missing"}}]}});
        let (handles, truncated) = super::frame_handles_from_tree(&missing_id, "tab").unwrap();
        assert_eq!(handles.len(), 1);
        assert!(truncated);

        let mut deep = json!({"frame":{"id":"frame-0","loaderId":"l"}});
        let mut cursor = &mut deep;
        for index in 1..=18 {
            cursor["childFrames"] =
                json!([{"frame":{"id":format!("frame-{index}"),"loaderId":"l"}}]);
            cursor = &mut cursor["childFrames"][0];
        }
        let (handles, truncated) =
            super::frame_handles_from_tree(&json!({"frameTree":deep}), "tab").unwrap();
        assert_eq!(handles.len(), 17);
        assert!(truncated);

        let children = (0..260)
            .map(|index| json!({"frame":{"id":format!("child-{index}"),"loaderId":"l"}}))
            .collect::<Vec<_>>();
        let wide =
            json!({"frameTree":{"frame":{"id":"root","loaderId":"l"},"childFrames":children}});
        let (handles, truncated) = super::frame_handles_from_tree(&wide, "tab").unwrap();
        assert_eq!(handles.len(), 256);
        assert!(truncated);
    }

    #[test]
    fn dedicated_cleanup_observer_is_limited_to_private_headless_sessions() {
        let headless = controlla_browser::sessions::SessionHandle {
            id: "headless".into(),
            principal: "test".into(),
            mode: controlla_browser::sessions::SessionMode::Headless,
            provider: controlla_browser::sessions::ProviderKind::DedicatedHeadless,
            capability_revision: 1,
        };
        let headed = controlla_browser::sessions::SessionHandle {
            mode: controlla_browser::sessions::SessionMode::Headed,
            provider: controlla_browser::sessions::ProviderKind::DedicatedHeaded,
            ..headless.clone()
        };
        let shared = controlla_browser::sessions::SessionHandle {
            mode: controlla_browser::sessions::SessionMode::Shared,
            provider: controlla_browser::sessions::ProviderKind::SharedExtension,
            ..headless.clone()
        };
        assert!(super::private_headless_cleanup_allowed(&headless));
        assert!(!super::private_headless_cleanup_allowed(&headed));
        assert!(!super::private_headless_cleanup_allowed(&shared));
    }

    #[test]
    fn shared_click_guard_allows_implicit_submit_only_outside_forms() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            ctx.eval::<(), _>(
                "globalThis.HTMLInputElement=class {hasAttribute(name){return name==='data-requires-trusted'&&this.trusted}}; globalThis.HTMLTextAreaElement=class {}; globalThis.HTMLButtonElement=class {hasAttribute(name){return name==='data-requires-trusted'&&this.trusted}};",
            )
            .unwrap();
            let unsafe_for = |tag: &str, kind: &str, in_form: bool, trusted: bool| {
                let form = if in_form { "{}" } else { "null" };
                let script = format!(
                    "(()=>{{const e=Object.assign(new {tag}(),{{type:{kind:?},form:{form},trusted:{trusted}}});const t=e.type||'';return {SHARED_CLICK_UNSAFE_PREDICATE};}})()"
                );
                ctx.eval::<bool, _>(script).unwrap()
            };
            assert!(!unsafe_for("HTMLButtonElement", "submit", false, false));
            assert!(unsafe_for("HTMLButtonElement", "submit", true, false));
            assert!(unsafe_for("HTMLButtonElement", "reset", true, false));
            assert!(unsafe_for("HTMLInputElement", "password", false, false));
            assert!(!unsafe_for("HTMLInputElement", "submit", false, false));
            assert!(!unsafe_for("HTMLInputElement", "text", false, true));
            assert!(unsafe_for("HTMLButtonElement", "button", false, true));
        });
    }

    #[test]
    fn shared_type_guard_is_valid_javascript() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            ctx.eval::<(), _>(format!(
                "globalThis.sharedTypeGuard = {SHARED_TYPE_GUARD_FUNCTION};"
            ))
            .unwrap();
        });
    }

    #[tokio::test]
    async fn configured_principal_scopes_planners_and_journal_reads() {
        let jobs = crate::jobs::Journal::open(":memory:").unwrap();
        let owned = jobs
            .admit("server-principal", "session", "key", &json!({"op":"x"}))
            .unwrap()
            .operation;
        let outsider = jobs
            .admit("other-principal", "session", "key", &json!({"op":"x"}))
            .unwrap()
            .operation;
        let app = App::with_principal_and_journal("server-principal", jobs);
        let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
        let server = serve_directly::<RoleServer, _, _, _, _>(app, server_io, None);
        let client = ().serve(client_io).await.unwrap();
        let plan = client
            .call_tool(CallToolRequestParams::new("slides_plan_text_edit").with_arguments(
                json!({"account_id":"a","presentation_id":"d","required_revision_id":"r","operation":"replace_range","object_id":"shape1","start_index":1,"end_index":2,"expected_text":"x","new_text":"y"})
                    .as_object().unwrap().clone(),
            ))
            .await.unwrap().structured_content.unwrap();
        assert_eq!(plan["plan"]["principal"], "server-principal");
        let owned_status = client
            .call_tool(
                CallToolRequestParams::new("workflow_status").with_arguments(
                    json!({"session_id":"session","operation_id":owned.id})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(owned_status["status"], "accepted");
        let status = client
            .call_tool(
                CallToolRequestParams::new("workflow_status").with_arguments(
                    json!({"session_id":"session","operation_id":outsider.id})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await;
        assert!(status.is_err());
        client.cancel().await.unwrap();
        server.waiting().await.unwrap();
    }

    #[tokio::test]
    async fn workflow_status_cancel_fences_a_running_job_before_next_dispatch() {
        let jobs = crate::jobs::Journal::open(":memory:").unwrap();
        let operation = jobs
            .admit(
                "server-principal",
                "session",
                "cancel-key",
                &json!({"op":"observe"}),
            )
            .unwrap()
            .operation;
        assert!(
            jobs.start("server-principal", "session", &operation.id)
                .unwrap()
        );
        let app = App::with_principal_and_journal("server-principal", jobs.clone());
        let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
        let server = serve_directly::<RoleServer, _, _, _, _>(app, server_io, None);
        let client = ().serve(client_io).await.unwrap();
        let response = client
            .call_tool(
                CallToolRequestParams::new("workflow_status").with_arguments(
                    json!({"session_id":"session","operation_id":operation.id,"cancel":true})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(response["status"], "cancelled");
        assert_eq!(response["cancellation_requested"], true);
        assert!(matches!(
            jobs.record_dispatch("server-principal", "session", &operation.id, "late"),
            Err(crate::jobs::JournalError::InvalidTransition("dispatch"))
        ));
        drop(client);
        server.cancel().await.unwrap();
    }

    #[test]
    fn bootstrap_instructions_include_the_runtime_tool_registry() {
        let instructions = App::default().get_info().instructions.unwrap();
        let tools = App::tool_router().list_all();
        for tool in &tools {
            assert!(
                instructions.contains(tool.name.as_ref()),
                "bootstrap instructions omit registered tool {}",
                tool.name
            );
        }
        assert!(
            instructions.len() <= 600,
            "bootstrap instructions grew too long"
        );
    }

    #[test]
    fn state_directory_lock_has_one_owner_for_its_lifetime() {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "chrome-controlla-state-lock-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let first = super::acquire_state_directory_lock(&dir).unwrap();
        let second = super::acquire_state_directory_lock(&dir);
        let error = second.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("CONTROLLA_STATE_DIR"));
        drop(first);
        let third = super::acquire_state_directory_lock(&dir).unwrap();
        drop(third);
        std::fs::remove_file(dir.join("server.lock")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

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
                {"section_id":"alpha","unique_count":1,"completeness":"complete","truncated":false,"missing":[]},
                {"section_id":"beta","unique_count":0,"completeness":"complete","truncated":false,"missing":[]}
            ]
        });
        let fitted = super::fit_aggregate(output, 4096).unwrap();
        assert_eq!(fitted["completeness"], "partial");
        assert_eq!(
            fitted["unique_count"], 0,
            "count is returned aggregate records"
        );
        assert_eq!(fitted["sections"][0]["completeness"], "partial");
        assert_eq!(
            fitted["sections"][0]["unique_count"], 1,
            "count is source rows observed before aggregate truncation"
        );
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
        let shared_tab = listed
            .tools
            .iter()
            .find(|tool| tool.name == "shared_tab")
            .expect("shared tab open/navigation tool is registered");
        let shared_tab_schema = serde_json::to_value(&shared_tab.input_schema).unwrap();
        assert!(shared_tab_schema["properties"]["action"].is_object());
        assert!(shared_tab_schema["properties"]["url"].is_object());
        for name in ["observe", "shared_observe"] {
            let tool = listed.tools.iter().find(|tool| tool.name == name).unwrap();
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            let spec_ref = schema["properties"]["spec"]["$ref"]
                .as_str()
                .expect("observe spec is described by a schema definition");
            let spec_name = spec_ref.rsplit('/').next().unwrap();
            assert_eq!(
                schema["$defs"][spec_name]["properties"]["max_items"]["minimum"], 1,
                "{name} schema must reject an empty observation item budget"
            );
            assert_eq!(
                schema["$defs"][spec_name]["properties"]["max_items"]["maximum"], 500,
                "{name} schema must match the browser observation item limit"
            );
            assert_eq!(
                schema["$defs"][spec_name]["properties"]["max_text_chars"]["minimum"], 1,
                "{name} schema must reject an empty observation text budget"
            );
            assert_eq!(
                schema["$defs"][spec_name]["properties"]["max_text_chars"]["maximum"], 10_000,
                "{name} schema must match the browser observation text limit"
            );
            assert_eq!(
                schema["$defs"][spec_name]["properties"]["max_bytes"]["minimum"], 4096,
                "{name} schema must reject byte budgets below the runtime minimum"
            );
            assert_eq!(
                schema["$defs"][spec_name]["properties"]["max_bytes"]["maximum"], 1_000_000,
                "{name} schema must reject byte budgets above the runtime maximum"
            );
            assert_eq!(
                schema["$defs"][spec_name]["properties"]["cursor"]["maxLength"], 256,
                "{name} schema must match the browser cursor limit"
            );
        }
        let workflow = listed
            .tools
            .iter()
            .find(|tool| tool.name == "workflow")
            .unwrap();
        let workflow_schema = serde_json::to_value(&workflow.input_schema).unwrap();
        let workflow_observe_variant = workflow_schema["$defs"]["WorkflowStep"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|variant| variant["properties"]["kind"]["const"] == "observe")
            .expect("workflow schema includes an observe variant");
        let workflow_spec_ref = workflow_observe_variant["properties"]["spec"]["$ref"]
            .as_str()
            .expect("workflow observe spec is described by a schema definition");
        let workflow_spec_name = workflow_spec_ref.rsplit('/').next().unwrap();
        assert_eq!(
            workflow_schema["$defs"][workflow_spec_name]["properties"]["max_items"]["maximum"], 500,
            "workflow schema must match the browser observation item limit"
        );
        assert_eq!(
            workflow_schema["$defs"][workflow_spec_name]["properties"]["max_text_chars"]["maximum"],
            10_000,
            "workflow schema must match the browser observation text limit"
        );
        assert_eq!(
            workflow_schema["$defs"][workflow_spec_name]["properties"]["max_items"]["minimum"], 1,
            "workflow schema must reject an empty observation item budget"
        );
        assert_eq!(
            workflow_schema["$defs"][workflow_spec_name]["properties"]["max_text_chars"]["minimum"],
            1,
            "workflow schema must reject an empty observation text budget"
        );
        assert_eq!(
            workflow_schema["$defs"][workflow_spec_name]["properties"]["cursor"]["maxLength"], 256,
            "workflow schema must match the browser cursor limit"
        );
        assert_eq!(
            workflow_schema["$defs"][workflow_spec_name]["properties"]["max_bytes"]["minimum"],
            4096,
            "workflow schema must match observation's runtime minimum"
        );
        assert_eq!(
            workflow_schema["$defs"][workflow_spec_name]["properties"]["max_bytes"]["maximum"],
            998_976,
            "workflow schema must reserve its 1024-byte result envelope"
        );
        let names = listed
            .tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert!(names.contains(&"session"));
        assert!(names.contains(&"observe"));
        assert!(names.contains(&"workflow"));
        assert!(names.contains(&"workflow_status"));
        assert!(names.contains(&"extract"));
        assert!(names.contains(&"accessibility"));
        assert!(names.contains(&"screenshot_crop"));
        assert!(names.contains(&"artifact_register"));
        assert!(names.contains(&"artifact_verify"));
        assert!(names.contains(&"file_select"));
        assert!(names.contains(&"shared_observe"));
        assert!(names.contains(&"shared_tab"));
        assert!(names.contains(&"guide"));
        assert!(names.contains(&"app_capabilities"));
        assert!(names.contains(&"slides_plan_text_edit"));
        assert!(names.contains(&"canva_sync_preflight"));
        assert!(names.contains(&"capcut_web_plan"));
        assert!(names.contains(&"slides_deck_plan"));
        assert!(names.contains(&"canva_design_plan"));
        assert!(names.contains(&"capcut_recipe_plan"));
        let guide = client
            .call_tool(
                CallToolRequestParams::new("guide").with_arguments(
                    json!({
                        "topic":"clients",
                        "server_version":env!("CARGO_PKG_VERSION")
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .unwrap();
        let guide = guide.structured_content.unwrap();
        assert_eq!(guide["server_version"], env!("CARGO_PKG_VERSION"));
        assert!(guide["content"].as_str().unwrap().contains("OpenCode v2"));
        let master_guide = client
            .call_tool(
                CallToolRequestParams::new("guide").with_arguments(
                    json!({
                        "topic":"master",
                        "server_version":env!("CARGO_PKG_VERSION")
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(master_guide["guide_version"], "master-2026-10-08-v6");
        assert!(
            master_guide["content"]
                .as_str()
                .unwrap()
                .contains("master-2026-10-08-v6")
        );
        let stale_guide = client
            .call_tool(
                CallToolRequestParams::new("guide").with_arguments(
                    json!({"topic":"clients","server_version":"9.9.9"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await;
        assert!(stale_guide.is_err());
        for name in [
            "session",
            "observe",
            "extract",
            "accessibility",
            "screenshot_crop",
            "artifact_register",
            "artifact_verify",
            "file_select",
            "shared_observe",
            "workflow",
            "guide",
            "app_capabilities",
            "slides_plan_text_edit",
            "canva_sync_preflight",
            "capcut_web_plan",
            "slides_deck_plan",
            "canva_design_plan",
            "capcut_recipe_plan",
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
        for name in [
            "slides_plan_text_edit",
            "slides_deck_plan",
            "canva_sync_preflight",
            "canva_design_plan",
        ] {
            let tool = listed.tools.iter().find(|tool| tool.name == name).unwrap();
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            assert!(
                !schema["properties"]
                    .as_object()
                    .unwrap()
                    .contains_key("principal"),
                "{name} must not accept transport identity as an argument"
            );
            if name == "canva_design_plan" {
                let nested = serde_json::to_string(&schema).unwrap();
                assert!(!nested.contains("principal"));
            }
        }
        let app_caps = client
            .call_tool(
                CallToolRequestParams::new("app_capabilities")
                    .with_arguments(json!({"app":"google_slides"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let app_caps = app_caps.structured_content.unwrap();
        assert_eq!(app_caps["live_qualified"], false);
        assert_eq!(app_caps["side_effects"], false);
        let slides_tool = listed
            .tools
            .iter()
            .find(|tool| tool.name == "slides_deck_plan")
            .unwrap();
        let slides_schema = serde_json::to_value(&slides_tool.input_schema).unwrap();
        let slides_required = slides_schema["required"].as_array().unwrap();
        assert!(
            slides_required
                .iter()
                .any(|field| field == "asserted_existing_slide_ids")
        );
        assert!(
            slides_required
                .iter()
                .any(|field| field == "asserted_existing_object_ids")
        );
        let canva_tool = listed
            .tools
            .iter()
            .find(|tool| tool.name == "canva_design_plan")
            .unwrap();
        let canva_schema = serde_json::to_value(&canva_tool.input_schema).unwrap();
        let schema_text = serde_json::to_string(&canva_schema).unwrap();
        assert!(schema_text.contains("absolute"));
        assert!(schema_text.contains("unsupported"));
        let deck = client
            .call_tool(
                CallToolRequestParams::new("slides_deck_plan").with_arguments(
                    json!({"account_id":"a","presentation_id":"deck-1","required_revision_id":"rev-1","asserted_existing_slide_ids":["existing-slide"],"asserted_existing_object_ids":["existing-slide","placeholder-title","placeholder-body"]})
                        .as_object().unwrap().clone(),
                ),
            )
            .await.unwrap().structured_content.unwrap();
        assert_eq!(deck["status"], "planned_not_dispatched");
        assert_eq!(deck["plan"]["principal"], "local-stdio");
        assert_eq!(deck["plan"]["slide_titles"].as_array().unwrap().len(), 10);
        assert_eq!(deck["side_effects"], false);
        assert_eq!(deck["preconditions_authoritative"], false);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let canva_identity = json!({"account_id":"a","workspace_id":"w","design_id":"d"});
        let canva = client.call_tool(CallToolRequestParams::new("canva_design_plan").with_arguments(
            json!({
                "binding":{"identity":canva_identity,"session_id":"s","expected_version":"v"},
                "session":{"identity":{"account_id":"a","workspace_id":"w","design_id":"d"},"session_id":"s","current_version":"v","opened_at_ms":now_ms-1000,"expires_at_ms":now_ms+30_000,"page_type":"absolute","locked":false}
            }).as_object().unwrap().clone(),
        )).await.unwrap().structured_content.unwrap();
        assert_eq!(canva["status"], "planned_not_dispatched");
        assert_eq!(canva["plan"]["pages"].as_array().unwrap().len(), 5);
        assert_eq!(canva["preconditions_authoritative"], false);
        assert_eq!(canva["plan"]["sync_candidate"]["effect"], "advisory_only");
        assert_eq!(
            canva["plan"]["sync_candidate"]["identity"]["principal"],
            "local-stdio"
        );
        let spoofed_canva = client.call_tool(CallToolRequestParams::new("canva_design_plan").with_arguments(
            json!({
                "binding":{"identity":{"principal":"attacker","account_id":"a","workspace_id":"w","design_id":"d"},"session_id":"s","expected_version":"v"},
                "session":{"identity":{"principal":"attacker","account_id":"a","workspace_id":"w","design_id":"d"},"session_id":"s","current_version":"v","opened_at_ms":now_ms-1000,"expires_at_ms":now_ms+30_000,"page_type":"absolute","locked":false}
            }).as_object().unwrap().clone(),
        )).await;
        assert!(
            spoofed_canva.is_err() || {
                let result = spoofed_canva.unwrap();
                result.is_error.unwrap_or(false)
                    || result.structured_content.is_some_and(|value| {
                        value["plan"]["sync_candidate"]["identity"]["principal"]
                            == LOCAL_STDIO_PRINCIPAL
                    })
            }
        );
        let roles = [
            "shade_shot",
            "water_rest_shot",
            "neighbor_shot",
            "narration",
            "music",
        ];
        let assets = roles.iter().enumerate().map(|(index, role)| json!({
            "asset_id":format!("asset-{index}"),"source_and_rights":"owned test media; rights recorded",
            "sha256":format!("{:064x}",index+1),"duration_ms":30_000,"frame_rate_milli":30_000,
            "width":1920,"height":1080,"audio_tracks":if index >= 3 {1} else {0},"expected_use":role
        })).collect::<Vec<_>>();
        let recipe = json!({
            "assets":assets,
            "clips":[
                {"asset_id":"asset-0","source_in_ms":0,"source_out_ms":7000,"timeline_start_ms":0},
                {"asset_id":"asset-1","source_in_ms":1000,"source_out_ms":9000,"timeline_start_ms":7000},
                {"asset_id":"asset-2","source_in_ms":0,"source_out_ms":8000,"timeline_start_ms":15000}
            ],
            "captions":[
                {"text":"Find shade during peak heat.","start_ms":500,"end_ms":6500},
                {"text":"Drink water and take a cool break.","start_ms":7200,"end_ms":14500},
                {"text":"Check on a neighbor.","start_ms":15200,"end_ms":21500}
            ],
            "narration":{"asset_id":"asset-3","source_in_ms":0,"source_out_ms":23000,"timeline_start_ms":0,"gain_millidb":0},
            "music":{"asset_id":"asset-4","source_in_ms":0,"source_out_ms":23000,"timeline_start_ms":0,"gain_millidb":-18000},
            "end_card":{"text":"Plan ahead. Look out for each other.","start_ms":22000,"end_ms":23000,"background_hex":"#142B3A","text_hex":"#FFFFFF","font_size_px":48},
            "width":1920,"height":1080
        });
        let capcut = client
            .call_tool(
                CallToolRequestParams::new("capcut_recipe_plan")
                    .with_arguments(json!({"recipe":recipe}).as_object().unwrap().clone()),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(capcut["status"], "offline_plan_only");
        assert_eq!(capcut["plan"]["duration_ms"], 23_000);
        let plan = client
            .call_tool(
                CallToolRequestParams::new("slides_plan_text_edit").with_arguments(
                    json!({
                        "account_id":"a", "presentation_id":"d",
                        "required_revision_id":"r", "operation":"replace_range",
                        "object_id":"shape1", "start_index":1,"end_index":6,
                        "expected_text":"Draft", "new_text":"Final"
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .unwrap();
        let plan = plan.structured_content.unwrap();
        assert_eq!(plan["status"], "planned_not_dispatched");
        assert_eq!(plan["plan"]["principal"], "local-stdio");
        assert_eq!(
            plan["plan"]["request_body"]["writeControl"]["requiredRevisionId"],
            "r"
        );
        assert_eq!(plan["evidence"], json!([]));
        let spoofed_slides = client
            .call_tool(
                CallToolRequestParams::new("slides_plan_text_edit").with_arguments(
                    json!({
                        "principal":"attacker", "account_id":"a", "presentation_id":"d",
                        "required_revision_id":"r", "operation":"replace_range",
                        "object_id":"shape1", "start_index":1,"end_index":6,
                        "expected_text":"Draft", "new_text":"Final"
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await;
        assert!(
            spoofed_slides.is_err() || {
                let result = spoofed_slides.unwrap();
                result.is_error.unwrap_or(false)
                    || result
                        .structured_content
                        .is_some_and(|value| value["plan"]["principal"] == LOCAL_STDIO_PRINCIPAL)
            }
        );
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
        let mut locator_schema = &file_select_schema["properties"]["locator"];
        if let Some(reference) = locator_schema["$ref"].as_str() {
            let name = reference.strip_prefix("#/$defs/").unwrap();
            locator_schema = &file_select_schema["$defs"][name];
        }
        let locator_options = locator_schema["anyOf"]
            .as_array()
            .or_else(|| locator_schema["oneOf"].as_array())
            .expect("locator schema enumerates its supported object forms");
        assert!(locator_options.iter().any(|option| {
            let option = option["$ref"]
                .as_str()
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .map_or(option, |name| &file_select_schema["$defs"][name]);
            option["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|field| field == "selector"))
        }));
        assert!(
            serde_json::from_value::<super::FileSelectLocator>(json!({
                "selector":"input[type=file]",
                "unrecognized":"ignored"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<super::FileSelectLocator>(json!({
                "selector":"input[type=file]"
            }))
            .is_ok()
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
        let previous_trusted_scripts = std::env::var_os("CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS");
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
        let listed = client
            .call_tool(CallToolRequestParams::new("session").with_arguments(args(
                json!({"action":"list_targets","session_id":session_id}),
            )))
            .await
            .unwrap();
        assert!(!listed.is_error.unwrap_or(false), "{listed:?}");
        let target_ref =
            listed.structured_content.as_ref().unwrap()["targets"][0]["target_ref"].clone();
        if cfg!(unix) {
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
            let artifact_handle = artifact.structured_content.as_ref().unwrap()["handle"].clone();
            assert_eq!(
                artifact.structured_content.as_ref().unwrap()["sha256"],
                "8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4"
            );
            let verified = client
                .call_tool(
                    CallToolRequestParams::new("artifact_verify").with_arguments(args(json!({
                        "session_id":session_id,
                        "artifact_handle":artifact_handle.clone()
                    }))),
                )
                .await
                .unwrap()
                .structured_content
                .unwrap();
            assert_eq!(verified["status"], "passed");
            assert_eq!(verified["scope"], "local_artifact_integrity_only");
            assert_eq!(verified["app_acceptance_or_persistence_verified"], false);
            assert_eq!(verified["expected"], verified["actual"]);
            let artifact_id = artifact_handle
                .as_str()
                .unwrap()
                .strip_prefix("artifact_")
                .unwrap()
                .split('@')
                .next()
                .unwrap();
            let staged_file = std::env::temp_dir()
                .join(format!("controlla-artifact-{artifact_id}"))
                .join("fixture.txt");
            assert!(staged_file.is_file(), "registered artifact file must exist");
            std::fs::write(&staged_file, b"h").unwrap();
            let truncated = client
                .call_tool(
                    CallToolRequestParams::new("artifact_verify").with_arguments(args(json!({
                        "session_id":session_id,
                        "artifact_handle":artifact_handle.clone()
                    }))),
                )
                .await
                .unwrap()
                .structured_content
                .unwrap();
            assert_eq!(truncated["status"], "failed");
            assert_eq!(truncated["expected"]["byte_length"], 2);
            assert_eq!(truncated["actual"]["byte_length"], 1);

            let second_session = client
                .call_tool(
                    CallToolRequestParams::new("session").with_arguments(args(json!({
                        "action":"connect",
                        "provider":"explicit_cdp",
                        "target_ids":["tab-1"]
                    }))),
                )
                .await
                .unwrap()
                .structured_content
                .unwrap()["session_id"]
                .as_str()
                .unwrap()
                .to_owned();
            assert_ne!(second_session, session_id);
            let cross_session = client
                .call_tool(
                    CallToolRequestParams::new("artifact_verify").with_arguments(args(json!({
                        "session_id":second_session,
                        "artifact_handle":artifact_handle.clone()
                    }))),
                )
                .await;
            assert!(
                cross_session.is_err() || cross_session.unwrap().is_error.unwrap_or(false),
                "an artifact handle must not verify in another session"
            );
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
            assert!(
                mismatched_marker.is_err() || mismatched_marker.unwrap().is_error.unwrap_or(false)
            );
            assert_eq!(
                file_select_calls.load(Ordering::SeqCst),
                0,
                "a mismatched marker must withhold DOM.setFileInputFiles"
            );
        } else {
            let artifact = client
                .call_tool(
                    CallToolRequestParams::new("artifact_register").with_arguments(args(
                        json!({"session_id":session_id,"filename":"fixture.txt","bytes":[104,105]}),
                    )),
                )
                .await;
            let artifact = artifact.expect("artifact registration should be supported on Windows");
            assert!(
                !artifact.is_error.unwrap_or(false),
                "Windows artifact staging must succeed"
            );
            assert!(
                artifact
                    .structured_content
                    .as_ref()
                    .is_some_and(|value| value["size"] == 2)
            );
        }
        let observed = client.call_tool(CallToolRequestParams::new("observe").with_arguments(args(json!({"session_id":session_id,"target_ref":target_ref,"spec":{"selector":"p","fields":{"text":"p"},"max_items":10,"max_text_chars":100,"max_bytes":4096,"cursor":null}})))).await.unwrap();
        assert!(!observed.is_error.unwrap_or(false), "{observed:?}");
        let workflow = client.call_tool(CallToolRequestParams::new("workflow").with_arguments(args(json!({
            "session_id":session_id,"idempotency_key":"phase6-integration","target_ref":target_ref,
            "steps":[{"kind":"observe","spec":{"selector":"p","fields":{"text":"p"},"max_items":2,"max_text_chars":100,"max_bytes":4096,"cursor":null}},{"kind":"checkpoint"}]
        })))).await.unwrap();
        let workflow = workflow.structured_content.unwrap();
        assert!(
            workflow["operation_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        assert_eq!(workflow["status"], "accepted");
        let operation_id = workflow["operation_id"].as_str().unwrap().to_owned();
        let mut status = Value::Null;
        for _ in 0..50 {
            status = client
                .call_tool(
                    CallToolRequestParams::new("workflow_status").with_arguments(args(json!({
                        "session_id":session_id,"operation_id":operation_id
                    }))),
                )
                .await
                .unwrap()
                .structured_content
                .unwrap();
            if status["status"] == "completed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(status["status"], "completed", "{status}");
        assert_eq!(status["metrics"]["browser_operations"], 1);
        assert_eq!(status["result"]["target"]["target_id"], "tab-1");
        assert_eq!(status["result"]["result"]["browser_operations"], 1);
        let replay = client.call_tool(CallToolRequestParams::new("workflow").with_arguments(args(json!({
            "session_id":session_id,"idempotency_key":"phase6-integration","target_ref":target_ref,
            "steps":[{"kind":"observe","spec":{"selector":"p","fields":{"text":"p"},"max_items":2,"max_text_chars":100,"max_bytes":4096,"cursor":null}},{"kind":"checkpoint"}]
        })))).await.unwrap().structured_content.unwrap();
        assert_eq!(replay["operation_id"], operation_id);
        assert_eq!(replay["replayed"], true);
        unsafe { std::env::set_var("CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS", "1") };
        let artifact_start = client
            .call_tool(CallToolRequestParams::new("workflow").with_arguments(args(json!({
                "session_id":session_id,"idempotency_key":"phase6-artifact","target_ref":target_ref,
                "steps":[{"kind":"script","source":"await api.observe({selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:4096,cursor:null}); return {kind:'artifact',filename:'summary.json',media_type:'application/json',bytes:[123,125]};"}]
            }))))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let artifact_operation = artifact_start["operation_id"].as_str().unwrap().to_owned();
        let mut artifact_status = Value::Null;
        let artifact_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::time::Instant::now() < artifact_deadline {
            artifact_status = client
                .call_tool(
                    CallToolRequestParams::new("workflow_status").with_arguments(args(json!({
                        "session_id":session_id,"operation_id":artifact_operation
                    }))),
                )
                .await
                .unwrap()
                .structured_content
                .unwrap();
            if artifact_status["status"] == "completed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(artifact_status["status"], "completed", "{artifact_status}");
        let artifact = &artifact_status["result"]["artifacts"][0];
        assert_eq!(artifact["operation_id"], artifact_operation);
        assert_eq!(artifact["principal"], "local-stdio");
        assert_eq!(artifact["session_id"], session_id);
        assert_eq!(artifact["bytes"], json!([123, 125]));
        assert_eq!(
            artifact["sha256"],
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        let artifact_replay = client
            .call_tool(CallToolRequestParams::new("workflow").with_arguments(args(json!({
                "session_id":session_id,"idempotency_key":"phase6-artifact","target_ref":target_ref,
                "steps":[{"kind":"script","source":"await api.observe({selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:4096,cursor:null}); return {kind:'artifact',filename:'summary.json',media_type:'application/json',bytes:[123,125]};"}]
            }))))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(artifact_replay["replayed"], true);
        assert_eq!(
            artifact_replay["result"]["artifacts"][0]["bytes"],
            json!([123, 125])
        );
        unsafe {
            if let Some(value) = previous_trusted_scripts {
                std::env::set_var("CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS", value);
            } else {
                std::env::remove_var("CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS");
            }
        }
        let crossed_session = client.call_tool(CallToolRequestParams::new("workflow").with_arguments(args(json!({
            "session_id":session_id,"idempotency_key":"crossed-session","target_ref":{ "session_id":"other-session", "principal":"local-stdio", "capability_revision":1, "browser_instance_id":"x", "browser_generation":1, "target_id":"tab-1", "target_revision":"x", "frame_id":"x", "frame_revision":1, "account_revision":0, "document_revision":0 },
            "steps":[{"kind":"checkpoint"}]
        })))).await;
        assert!(crossed_session.is_err() || crossed_session.unwrap().is_error.unwrap_or(false));
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
                    "type":"hello","token":token,"extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"targets":["123"]
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
            let ready: Value =
                serde_json::from_str(peer.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(
                ready,
                json!({"type":"ready","server_version":env!("CARGO_PKG_VERSION"),"protocol_version":1})
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

        let ax = client.call_tool(CallToolRequestParams::new("shared_accessibility").with_arguments(args(json!({
            "session_id":session_id,"chrome_tab_id":"123","selector":"#selected","max_bytes":8192
        }))));
        let ax_replies = async {
            for (method, result) in [
                (
                    "Page.getFrameTree",
                    json!({"frameTree":{"frame":{"id":"root","loaderId":"doc-1","url":"https://fixture.test/"}}}),
                ),
                ("DOM.getDocument", json!({"root":{"nodeId":1}})),
                ("DOM.querySelector", json!({"nodeId":2})),
                (
                    "Accessibility.getPartialAXTree",
                    json!({"nodes":[{"nodeId":"7","role":{"value":"button"}}]}),
                ),
                (
                    "Page.getFrameTree",
                    json!({"frameTree":{"frame":{"id":"root","loaderId":"doc-1","url":"https://fixture.test/"}}}),
                ),
            ] {
                let request: Value = serde_json::from_str(
                    extension.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                assert_eq!(request["method"], method);
                if method == "Accessibility.getPartialAXTree" {
                    assert_eq!(request["params"]["fetchRelatives"], false);
                    assert_eq!(request["params"]["nodeId"], 2);
                }
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
        let (ax, ()) = tokio::join!(ax, ax_replies);
        let ax = ax.unwrap().structured_content.unwrap();
        assert_eq!(ax["nodes"][0]["nodeId"], "7");
        assert_eq!(ax["shared_frame_identity"]["loader_id"], "doc-1");
        assert!(serde_json::to_vec(&ax).unwrap().len() <= 8192);

        let frame_ax = client.call_tool(
            CallToolRequestParams::new("shared_accessibility").with_arguments(args(json!({
                "session_id":session_id,"chrome_tab_id":"123","frame_id":"child","max_bytes":1_000_000
            }))),
        );
        let frame_ax_replies = async {
            let tree = json!({"frameTree":{"frame":{"id":"root","loaderId":"doc-1","url":"https://fixture.test/"},"childFrames":[{"frame":{"id":"child","loaderId":"child-doc","url":"https://picker.fixture.test/"}}]}});
            for (method, result) in [
                ("Page.getFrameTree", tree.clone()),
                ("Page.getFrameTree", tree.clone()),
                (
                    "Accessibility.getFullAXTree",
                    json!({"nodes":[{"backendDOMNodeId":41,"role":{"value":"button"},"name":{"value":"Select account"}}]}),
                ),
                ("Page.getFrameTree", tree.clone()),
                ("Page.getFrameTree", tree),
            ] {
                let Ok(Some(Ok(message))) =
                    tokio::time::timeout(std::time::Duration::from_secs(1), extension.next()).await
                else {
                    break;
                };
                let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                assert_eq!(request["method"], method);
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
        let (frame_ax, ()) = tokio::join!(frame_ax, frame_ax_replies);
        let frame_ax = frame_ax.unwrap().structured_content.unwrap();
        assert_eq!(frame_ax["items"][0]["backend_node_id"], 41);
        assert_eq!(frame_ax["items"][0]["name"], "Select account");
        assert_eq!(frame_ax["routing"], "root_frame_id");
        assert_eq!(frame_ax["frame_loader_id"], "child-doc");

        let duplicate_name_frame_ax = client.call_tool(
            CallToolRequestParams::new("shared_accessibility").with_arguments(args(json!({
                "session_id":session_id,"chrome_tab_id":"123","frame_id":"child-two","max_bytes":1_000_000
            }))),
        );
        let duplicate_name_frame_ax_replies = async {
            let tree = json!({"frameTree":{"frame":{"id":"root","loaderId":"doc-1","url":"https://fixture.test/"},"childFrames":[
                {"frame":{"id":"child","loaderId":"child-doc","url":"https://picker.fixture.test/"}},
                {"frame":{"id":"child-two","loaderId":"child-two-doc","url":"https://drive.fixture.test/"}}
            ]}});
            for (method, result) in [
                ("Page.getFrameTree", tree.clone()),
                ("Page.getFrameTree", tree.clone()),
                (
                    "Accessibility.getFullAXTree",
                    json!({"nodes":[{"backendDOMNodeId":41,"role":{"value":"button"},"name":{"value":"Select account"}}]}),
                ),
                ("Page.getFrameTree", tree.clone()),
                ("Page.getFrameTree", tree),
            ] {
                let request: Value = serde_json::from_str(
                    extension.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                assert_eq!(request["method"], method);
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
        let (duplicate_name_frame_ax, ()) =
            tokio::join!(duplicate_name_frame_ax, duplicate_name_frame_ax_replies);
        let duplicate_name_frame_ax = duplicate_name_frame_ax.unwrap().structured_content.unwrap();
        assert_eq!(
            duplicate_name_frame_ax["items"][0]["name"],
            frame_ax["items"][0]["name"]
        );
        assert_eq!(frame_ax["items"][0]["reference"], "@f:child:child-doc:41");
        assert_eq!(
            duplicate_name_frame_ax["items"][0]["reference"],
            "@f:child-two:child-two-doc:41"
        );
        assert_ne!(
            frame_ax["items"][0]["reference"], duplicate_name_frame_ax["items"][0]["reference"],
            "same-name controls in separate frames remain bound to distinct frame/loader identities"
        );

        let oversized_ax = client.call_tool(CallToolRequestParams::new("shared_accessibility").with_arguments(args(json!({
            "session_id":session_id,"chrome_tab_id":"123","selector":"#selected","max_bytes":4096
        }))));
        let oversized_ax_replies = async {
            for (method, result) in [
                (
                    "Page.getFrameTree",
                    json!({"frameTree":{"frame":{"id":"root","loaderId":"doc-1","url":"https://fixture.test/"}}}),
                ),
                ("DOM.getDocument", json!({"root":{"nodeId":1}})),
                ("DOM.querySelector", json!({"nodeId":2})),
                (
                    "Accessibility.getPartialAXTree",
                    json!({"nodes":[{"name":{"value":"x".repeat(5000)}}]}),
                ),
            ] {
                let request: Value = serde_json::from_str(
                    extension.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                assert_eq!(request["method"], method);
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
        let (oversized_ax, ()) = tokio::join!(oversized_ax, oversized_ax_replies);
        assert!(oversized_ax.is_err() || oversized_ax.unwrap().is_error.unwrap_or(false));

        let timed_ax = client.call_tool(CallToolRequestParams::new("shared_accessibility").with_arguments(args(json!({
            "session_id":session_id,"chrome_tab_id":"123","selector":"#selected","max_bytes":8192,"timeout_ms":1000
        }))));
        let first_ax_request = async {
            let request: Value =
                serde_json::from_str(extension.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(request["method"], "Page.getFrameTree");
            request
        };
        let (timed_ax, first_ax_request) = tokio::join!(timed_ax, first_ax_request);
        assert!(timed_ax.is_err() || timed_ax.unwrap().is_error.unwrap_or(false));
        extension.send(Message::Text(json!({"type":"result","id":first_ax_request["id"],"result":{"frameTree":{"frame":{"id":"root","loaderId":"doc-1","url":"https://fixture.test/"}}}}).to_string().into())).await.unwrap();

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
    async fn mcp_shared_input_fills_with_readback_and_refuses_stale_or_ambiguous_targets() {
        use futures_util::{SinkExt, StreamExt};
        use std::sync::{Arc, Mutex as StdMutex};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let (server_io, client_io) = tokio::io::duplex(16_384);
        let server = serve_directly::<RoleServer, _, _, _, _>(App::default(), server_io, None);
        let server_task = tokio::spawn(async move { server.waiting().await });
        let client = ().serve(client_io).await.unwrap();
        let listed = client.list_tools(None).await.unwrap();
        assert!(listed.tools.iter().any(|tool| tool.name == "shared_input"));
        let args = |value: Value| value.as_object().unwrap().clone();
        let paired = client
            .call_tool(
                CallToolRequestParams::new("session")
                    .with_arguments(args(json!({"action":"pair_shared","target_ids":["123"]}))),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let endpoint = paired["endpoint"].as_str().unwrap().to_owned();
        let token = paired["one_session_token"].as_str().unwrap().to_owned();
        let session_id = paired["session_id"].as_str().unwrap().to_owned();
        let accept = client.call_tool(CallToolRequestParams::new("session").with_arguments(args(
            json!({"action":"accept_shared","session_id":session_id}),
        )));
        let extension = async move {
            let (mut peer, _) = connect_async(endpoint).await.unwrap();
            peer.send(Message::Text(
                json!({"type":"hello","token":token,"extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"targets":["123"]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            let ready: Value =
                serde_json::from_str(peer.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(
                ready,
                json!({"type":"ready","server_version":env!("CARGO_PKG_VERSION"),"protocol_version":1})
            );
            peer
        };
        let (accepted, mut extension) = tokio::join!(accept, extension);
        assert_eq!(
            accepted.unwrap().structured_content.unwrap()["accepted"],
            true
        );
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let expressions = Arc::new(StdMutex::new(Vec::new()));
        let mouse_events = Arc::new(StdMutex::new(Vec::new()));
        let key_events = Arc::new(StdMutex::new(Vec::new()));
        let key_down_times = Arc::new(StdMutex::new(Vec::new()));
        let replace_after_first_char = Arc::new(StdMutex::new(true));
        let type_identity_calls = Arc::new(StdMutex::new(Vec::new()));
        let key_failures = Arc::new(StdMutex::new(std::collections::BTreeMap::from([
            ("keyDown:d".to_owned(), 1usize),
            ("char:c".to_owned(), 1usize),
            ("keyUp:u".to_owned(), 1usize),
            ("keyUp:v".to_owned(), 2usize),
        ])));
        let release_object_failures = Arc::new(StdMutex::new(0usize));
        let frame_reads = Arc::new(StdMutex::new(0usize));
        let seen_server = seen.clone();
        let expressions_server = expressions.clone();
        let mouse_events_server = mouse_events.clone();
        let key_events_server = key_events.clone();
        let key_down_times_server = key_down_times.clone();
        let replace_after_first_char_server = replace_after_first_char.clone();
        let type_identity_calls_server = type_identity_calls.clone();
        let key_failures_server = key_failures.clone();
        let release_object_failures_server = release_object_failures.clone();
        let frame_reads_server = frame_reads.clone();
        let extension_task = tokio::spawn(async move {
            let mut eval_index = 0;
            let mut mouse_presses = 0;
            while let Some(Ok(message)) = extension.next().await {
                let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                let method = request["method"].as_str().unwrap().to_owned();
                seen_server.lock().unwrap().push(method.clone());
                let result = match method.as_str() {
                    "Page.getFrameTree" => {
                        *frame_reads_server.lock().unwrap() += 1;
                        json!({"frameTree":{"frame":{"id":"root","loaderId":"doc-1","url":"https://fixture.test/"}}})
                    }
                    "Runtime.evaluate" => {
                        let expression =
                            request["params"]["expression"].as_str().unwrap().to_owned();
                        expressions_server.lock().unwrap().push(expression.clone());
                        if expression.contains("shared_type_handle") {
                            json!({"result":{"type":"object","objectId":"input-object"}})
                        } else if expression.contains("route:'sequential_type_readback'") {
                            json!({"result":{"type":"object","value":{"ok":true,"route":"sequential_type_readback","value":"oldx"}}})
                        } else if expression.contains("route:'sequential_type'") {
                            if expression.contains("\"oldx\"") {
                                json!({"result":{"type":"object","value":{"ok":false,"route":"sequential_type","reason":"stale_value"}}})
                            } else {
                                json!({"result":{"type":"object","value":{"ok":true,"route":"sequential_type","value":"old"}}})
                            }
                        } else {
                            let outcomes = [
                                json!({"result":{"type":"object","value":{"ok":true,"value":"new"}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"value":"new"}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"value":"new"}}}),
                                json!({"result":{"type":"object","value":{"ok":false,"value":"changed by listener","dispatched":true}}}),
                                json!({"result":{"type":"object","value":{"ok":false,"reason":"stale_value"}}}),
                                json!({"result":{"type":"object","value":{"ok":false,"reason":"ambiguous"}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":12.0,"y":13.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":14.0,"y":15.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":false,"value":"Pending"}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":12.0,"y":13.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":14.0,"y":15.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"value":"Saved"}}}),
                                json!({"result":{"type":"object","value":{"ok":true}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":12.0,"y":13.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":14.0,"y":15.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":false,"reason":"stale_value"}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":12.0,"y":13.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":14.0,"y":15.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":12.0,"y":13.0,"postcondition_before_is_desired":false}}}),
                                json!({"result":{"type":"object","value":{"ok":true,"x":14.0,"y":15.0,"postcondition_before_is_desired":false}}}),
                            ];
                            let result = outcomes.get(eval_index).cloned().unwrap_or_else(
                                || json!({"result":{"type":"boolean","value":true}}),
                            );
                            eval_index += 1;
                            result
                        }
                    }
                    "Runtime.callFunctionOn" => {
                        let params = request["params"].clone();
                        type_identity_calls_server
                            .lock()
                            .unwrap()
                            .push(params.clone());
                        let arguments = params["arguments"].as_array().unwrap();
                        let expected = arguments[1]["value"].as_str().unwrap();
                        if arguments[2]["value"] == true {
                            json!({"result":{"type":"object","value":{"ok":true,"value":expected}}})
                        } else if expected == "oldx"
                            && *replace_after_first_char_server.lock().unwrap()
                        {
                            json!({"result":{"type":"object","value":{"ok":false,"reason":"target_replaced"}}})
                        } else {
                            json!({"result":{"type":"object","value":{"ok":true}}})
                        }
                    }
                    "Runtime.releaseObject" => {
                        let mut failures = release_object_failures_server.lock().unwrap();
                        if *failures > 0 {
                            *failures -= 1;
                            json!({"error":"fixture cleanup error"})
                        } else {
                            json!({"result":{}})
                        }
                    }
                    "Input.dispatchMouseEvent" => {
                        mouse_events_server.lock().unwrap().push((
                            request["params"]["type"].as_str().unwrap().to_owned(),
                            request["params"]["x"].as_f64().unwrap(),
                            request["params"]["y"].as_f64().unwrap(),
                        ));
                        if request["params"]["type"] == "mousePressed" {
                            mouse_presses += 1;
                            if mouse_presses == 3 {
                                json!({"error":"fixture uncertain dispatch"})
                            } else {
                                json!({"result":{}})
                            }
                        } else if mouse_presses == 4 {
                            json!({"error":"fixture uncertain release"})
                        } else {
                            json!({"result":{}})
                        }
                    }
                    "Input.dispatchKeyEvent" => {
                        let params = request["params"].clone();
                        key_events_server.lock().unwrap().push(params.clone());
                        let ty = params["type"].as_str().unwrap();
                        let key = params["key"].as_str().or(params["text"].as_str()).unwrap();
                        if ty == "keyDown" {
                            key_down_times_server
                                .lock()
                                .unwrap()
                                .push(std::time::Instant::now());
                        }
                        let failure_key = format!("{ty}:{key}");
                        let mut failures = key_failures_server.lock().unwrap();
                        if let Some(remaining) = failures.get_mut(&failure_key).filter(|n| **n > 0)
                        {
                            *remaining -= 1;
                            json!({"error":format!("fixture uncertain {failure_key}")})
                        } else {
                            json!({"result":{}})
                        }
                    }
                    _ => panic!("unexpected shared command: {method}"),
                };
                let response = if result.get("error").is_some() {
                    json!({"type":"result","id":request["id"],"error":result["error"]})
                } else {
                    json!({"type":"result","id":request["id"],"result":result})
                };
                extension
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        let missing_postcondition_selector = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"button.save",
                    "action":"click","expected_value":"Save","value":"","postcondition":"Saved"
                }))),
            )
            .await;
        assert!(
            missing_postcondition_selector.is_err()
                || missing_postcondition_selector
                    .unwrap()
                    .is_error
                    .unwrap_or(false)
        );
        let fill = |expected_value: &str| {
            client.call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                    "action":"fill","expected_value":expected_value,"value":"new"
                }))),
            )
        };
        let filled = fill("old").await.unwrap();
        assert_eq!(filled.structured_content.unwrap()["observed_value"], "new");
        let changed_after_event = fill("old").await.unwrap().structured_content.unwrap();
        assert_eq!(
            changed_after_event["status"], "unknown",
            "a post-dispatch listener mutation must not be called not-dispatched: {changed_after_event}"
        );
        assert_eq!(changed_after_event["dispatch_may_have_occurred"], true);
        let stale = fill("old").await;
        assert!(stale.is_err() || stale.unwrap().is_error.unwrap_or(false));
        let ambiguous = fill("old").await;
        assert!(ambiguous.is_err() || ambiguous.unwrap().is_error.unwrap_or(false));
        let clicked = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"button.save",
                    "action":"click","expected_value":"Save","value":"","postcondition_selector":"#save-state","postcondition":"Saved"
                }))),
            )
            .await;
        assert!(
            clicked.is_err() || clicked.unwrap().is_error.unwrap_or(false),
            "unchanged independently selected state must not verify a click"
        );
        let clicked = client
            .call_tool(CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                "session_id":session_id,"chrome_tab_id":"123","selector":"button.save",
                "action":"click","expected_value":"Save","value":"","postcondition_selector":"#save-state","postcondition":"Saved"
            }))))
            .await
            .unwrap();
        assert!(clicked.structured_content.unwrap()["verified"] == true);
        let missing_coordinates = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"button.save",
                    "action":"click","expected_value":"Save","value":"","postcondition_selector":"#save-state","postcondition":"Saved"
                }))),
            )
            .await;
        assert!(missing_coordinates.is_err());
        let uncertain_dispatch = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"button.save",
                    "action":"click","expected_value":"Save","value":"","postcondition_selector":"#save-state","postcondition":"Saved"
                }))),
            )
            .await;
        assert!(uncertain_dispatch.is_err());
        let stale_click = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"button.save",
                    "action":"click","expected_value":"Save","value":"","postcondition_selector":"#save-state","postcondition":"Saved"
                }))),
            )
            .await;
        assert!(stale_click.is_err());
        let uncertain_release = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"button.save",
                    "action":"click","expected_value":"Save","value":"","postcondition_selector":"#save-state","postcondition":"Saved"
                }))),
            )
            .await;
        assert!(
            uncertain_release.is_err(),
            "a lost release acknowledgement remains an error"
        );
        let unsupported_unicode = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                    "action":"type","expected_value":"old","value":"λ"
                }))),
            )
            .await;
        assert!(
            unsupported_unicode.is_err(),
            "sequential key events are ASCII only"
        );
        let typed = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                    "action":"type","expected_value":"old","value":"x"
                }))),
            )
            .await
            .unwrap();
        assert_eq!(typed.structured_content.unwrap()["observed_value"], "oldx");
        *replace_after_first_char.lock().unwrap() = false;
        let before_paced_keydowns = key_down_times.lock().unwrap().len();
        let paced = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                    "action":"type","expected_value":"old","value":"xy"
                }))),
            )
            .await
            .unwrap();
        assert_eq!(paced.structured_content.unwrap()["observed_value"], "oldxy");
        {
            let key_down_times = key_down_times.lock().unwrap();
            let paced_key_downs = &key_down_times[before_paced_keydowns..];
            assert_eq!(paced_key_downs.len(), 2);
            assert!(
                paced_key_downs[1].duration_since(paced_key_downs[0])
                    >= std::time::Duration::from_millis(60),
                "actual dispatched keyDown events must have the fixed minimum spacing"
            );
        }
        *replace_after_first_char.lock().unwrap() = true;
        let stopped_after_interference = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                    "action":"type","expected_value":"old","value":"xy"
                }))),
            )
            .await;
        assert!(
            stopped_after_interference.is_err()
                || stopped_after_interference
                    .unwrap()
                    .is_error
                    .unwrap_or(false),
            "typing stops when the exact value changes between characters"
        );
        let before_empty_type = seen.lock().unwrap().len();
        let empty_type = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                    "action":"type","expected_value":"old","value":""
                }))),
            )
            .await;
        assert!(empty_type.is_err(), "empty sequential typing is rejected");
        assert_eq!(
            seen.lock().unwrap().len(),
            before_empty_type,
            "empty typing dispatches nothing"
        );
        let before_over_deadline_type = seen.lock().unwrap().len();
        let over_deadline_type = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                    "action":"type","expected_value":"old","value":"x".repeat(100),"timeout_ms":6000
                }))),
            )
            .await;
        assert!(
            over_deadline_type.is_err(),
            "paced typing whose minimum duration cannot fit must be refused"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            before_over_deadline_type,
            "deadline preflight rejects before any browser command"
        );
        assert_eq!(
            shared_typing_minimum_duration(2),
            std::time::Duration::from_millis(60),
            "one fixed 60 ms delay separates adjacent characters"
        );
        assert_eq!(
            shared_typing_minimum_duration(1),
            std::time::Duration::ZERO,
            "single-character input needs no inter-character delay"
        );
        assert_eq!(
            shared_typing_minimum_duration(17),
            std::time::Duration::from_millis(960),
            "17 characters fit within the 1 second action budget at the 6 second minimum timeout"
        );
        assert_eq!(
            shared_typing_minimum_duration(18),
            std::time::Duration::from_millis(1020),
            "18 characters exceed the 1 second action budget at the 6 second minimum timeout"
        );
        for key in ["d", "c", "u", "v"] {
            if key == "v" {
                *release_object_failures.lock().unwrap() = 1;
            }
            let failed = client
                .call_tool(
                    CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                        "session_id":session_id,"chrome_tab_id":"123","selector":"input[name='title']",
                        "action":"type","expected_value":"old","value":key
                    }))),
                )
                .await;
            assert!(
                failed.is_err(),
                "uncertain {key} event never reports verified typing"
            );
            if key == "v" {
                let message = format!("{failed:?}");
                assert!(message.contains("key release remained uncertain"));
                assert!(message.contains("typing handle cleanup failed"));
            }
        }
        extension_task.abort();
        {
            let methods = seen.lock().unwrap();
            assert_eq!(
                methods
                    .iter()
                    .filter(|method| method.as_str() == "Input.dispatchMouseEvent")
                    .count(),
                12,
                "attempt a mouse release even when the press acknowledgement is uncertain"
            );
            assert_eq!(
                &mouse_events.lock().unwrap()[..3],
                &[
                    ("mouseMoved".to_owned(), 14.0, 15.0),
                    ("mousePressed".to_owned(), 14.0, 15.0),
                    ("mouseReleased".to_owned(), 14.0, 15.0)
                ],
                "dispatch must use the click geometry from the immediately refreshed hit test"
            );
            assert_eq!(
                *frame_reads.lock().unwrap(),
                34,
                "every attempted action performs a post-action identity read"
            );
            assert_eq!(
                methods
                    .iter()
                    .filter(|method| method.as_str() == "Runtime.evaluate")
                    .count(),
                25
            );
            let keys = key_events.lock().unwrap();
            assert_eq!(
                &keys[..6],
                &[
                    json!({"type":"keyDown","key":"x"}),
                    json!({"type":"char","text":"x","unmodifiedText":"x"}),
                    json!({"type":"keyUp","key":"x"}),
                    json!({"type":"keyDown","key":"x"}),
                    json!({"type":"char","text":"x","unmodifiedText":"x"}),
                    json!({"type":"keyUp","key":"x"})
                ],
                "sequential typing stops before the next character when its guard changes"
            );
            assert!(
                keys.windows(2).any(|events| events
                    == [
                        json!({"type":"keyDown","key":"d"}),
                        json!({"type":"keyUp","key":"d"})
                    ]),
                "uncertain keyDown triggers a release attempt"
            );
            assert!(
                keys.windows(3).any(|events| events
                    == [
                        json!({"type":"keyDown","key":"c"}),
                        json!({"type":"char","text":"c","unmodifiedText":"c"}),
                        json!({"type":"keyUp","key":"c"})
                    ]),
                "uncertain character dispatch triggers a release attempt"
            );
            assert_eq!(
                keys.iter()
                    .filter(|event| event == &&json!({"type":"keyUp","key":"u"}))
                    .count(),
                2,
                "uncertain keyUp gets one bounded retry but remains an error"
            );
            assert_eq!(
                keys.iter()
                    .filter(|event| event == &&json!({"type":"keyUp","key":"v"}))
                    .count(),
                2,
                "repeated keyUp failure stops after the bounded retry"
            );
            let identity_calls = type_identity_calls.lock().unwrap();
            assert_eq!(identity_calls.len(), 18);
            assert!(identity_calls.iter().all(|call| {
                call["objectId"] == "input-object"
                    && call["functionDeclaration"]
                        .as_str()
                        .unwrap()
                        .contains("es[0]!==e")
            }));
            assert!(
                identity_calls
                    .iter()
                    .any(|call| call["arguments"][1]["value"] == "oldx")
            );
            let expressions = expressions.lock().unwrap();
            assert_eq!(
                expressions
                    .iter()
                    .filter(|expression| expression.contains("shared_type_handle"))
                    .count(),
                7
            );
            assert!(
                expressions[0].contains("document.elementFromPoint"),
                "fill must reject covered controls"
            );
            assert!(
                expressions[0].contains("b.left<0")
                    && expressions[0].contains("b.right>innerWidth"),
                "fill must reject offscreen controls"
            );
            assert!(
                expressions[0].contains("matches(':disabled')"),
                "fill must reject inherited disabled state"
            );
            assert!(
                expressions[7].contains("!!e.form")
                    && expressions[7].contains("['submit','reset'].includes(t)"),
                "click must block submit/reset controls only when they can act on a form"
            );
            assert!(expressions[7].contains("matches(':disabled')"));
            assert!(expressions[6].contains("#save-state"));
            assert!(expressions[6].contains("postcondition_before"));
            assert!(expressions[8].contains("current.length<=16384"));
            assert!(expressions[8].contains("===false"));
        }
        client.cancel().await.unwrap();
        let _ = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn shared_pairing_accepts_extension_before_followup_tool_call() {
        use super::{Duration, Parameters, SessionArgs};
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};
        let app = App::default();
        let pair = app
            .session(Parameters(SessionArgs {
                action: "pair_shared".into(),
                provider: None,
                session_id: None,
                target_ids: Some(vec!["123".into()]),
                host_id: None,
            }))
            .await
            .unwrap()
            .0;
        let waiting = tokio::time::timeout(
            Duration::from_secs(1),
            app.session(Parameters(SessionArgs {
                action: "accept_shared".into(),
                provider: None,
                session_id: Some(pair["session_id"].as_str().unwrap().into()),
                target_ids: None,
                host_id: None,
            })),
        )
        .await
        .expect("status must not block the extension UI")
        .unwrap()
        .0;
        assert_eq!(waiting["accepted"], false);
        // One long wait can overlap extension hello and finish without polling accept_shared.
        let id = pair["session_id"].as_str().unwrap().to_owned();
        let wait = tokio::time::timeout(
            Duration::from_secs(2),
            app.session(Parameters(SessionArgs {
                action: "accept_shared_wait".into(),
                provider: None,
                session_id: Some(id.clone()),
                target_ids: None,
                host_id: None,
            })),
        );
        let connect = async {
            tokio::time::timeout(Duration::from_secs(1), async {
            let (mut socket, _) = connect_async(pair["endpoint"].as_str().unwrap())
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"hello", "token":pair["one_session_token"],
                "extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true, "targets":["123"]})
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let ready = socket.next().await.unwrap().unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(ready.to_text().unwrap()).unwrap()["type"],
                "ready"
            );
            socket
            })
            .await
            .expect("extension hello should complete")
        };
        let (accepted, mut socket) = tokio::join!(wait, connect);
        let accepted = accepted
            .expect("single event-driven wait should complete")
            .unwrap()
            .0;
        assert_eq!(accepted["accepted"], true);
        app.session(Parameters(SessionArgs {
            action: "release_shared".into(),
            provider: None,
            session_id: Some(id),
            target_ids: None,
            host_id: None,
        }))
        .await
        .unwrap();
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    }

    #[tokio::test]
    async fn a_server_can_keep_multiple_shared_sessions_without_forcing_pair_release_first() {
        use super::{Parameters, SessionArgs};

        let app = App::default();
        let first = app
            .session(Parameters(SessionArgs {
                action: "pair_shared".into(),
                provider: None,
                session_id: None,
                target_ids: Some(vec!["123".into()]),
                host_id: None,
            }))
            .await
            .unwrap()
            .0;

        let second = app
            .session(Parameters(SessionArgs {
                action: "pair_shared".into(),
                provider: None,
                session_id: None,
                target_ids: Some(vec!["456".into()]),
                host_id: None,
            }))
            .await
            .unwrap()
            .0;
        assert_ne!(first["session_id"], second["session_id"]);

        for session in [&first, &second] {
            let released = app
                .session(Parameters(SessionArgs {
                    action: "release_shared".into(),
                    provider: None,
                    session_id: session["session_id"].as_str().map(str::to_owned),
                    target_ids: None,
                    host_id: None,
                }))
                .await
                .unwrap()
                .0;
            assert_eq!(released["released"], true);
        }
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

    #[tokio::test]
    async fn shared_click_reports_observed_navigation_without_claiming_causality() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let (server_io, client_io) = tokio::io::duplex(16_384);
        let server = serve_directly::<RoleServer, _, _, _, _>(App::default(), server_io, None);
        let server_task = tokio::spawn(async move { server.waiting().await });
        let client = ().serve(client_io).await.unwrap();
        let args = |value: Value| value.as_object().unwrap().clone();
        let paired = client
            .call_tool(
                CallToolRequestParams::new("session").with_arguments(args(json!({
                    "action":"pair_shared","target_ids":["123"]
                }))),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let endpoint = paired["endpoint"].as_str().unwrap().to_owned();
        let token = paired["one_session_token"].as_str().unwrap().to_owned();
        let session_id = paired["session_id"].as_str().unwrap().to_owned();
        let accept = client.call_tool(CallToolRequestParams::new("session").with_arguments(args(
            json!({
                "action":"accept_shared","session_id":session_id
            }),
        )));
        let extension = async move {
            let (mut socket, _) = connect_async(endpoint).await.unwrap();
            socket.send(Message::Text(json!({"type":"hello","token":token,
                "extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"targets":["123"]
            }).to_string().into())).await.unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            socket
        };
        let (accepted, mut extension) = tokio::join!(accept, extension);
        assert_eq!(
            accepted.unwrap().structured_content.unwrap()["accepted"],
            true
        );
        let extension_task = tokio::spawn(async move {
            let mut frame_reads = 0;
            let mut eval_reads = 0;
            while let Some(Ok(message)) = extension.next().await {
                let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                let method = request["method"].as_str().unwrap();
                let response = match method {
                    "Page.getFrameTree" => {
                        frame_reads += 1;
                        let (loader, url) = if frame_reads == 1 {
                            ("doc-before", "https://fixture.test/team")
                        } else {
                            ("doc-after", "https://fixture.test/story")
                        };
                        json!({"type":"result","id":request["id"],"result":{"frameTree":{"frame":{"id":"root","loaderId":loader,"url":url}}}})
                    }
                    "Runtime.evaluate" => {
                        eval_reads += 1;
                        if eval_reads <= 2 {
                            json!({"type":"result","id":request["id"],"result":{"result":{"type":"object","value":{"ok":true,"x":10.0,"y":20.0,"postcondition_before_is_desired":false}}}})
                        } else {
                            json!({"type":"result","id":request["id"],"error":"Execution context was destroyed by navigation."})
                        }
                    }
                    "Input.dispatchMouseEvent" => {
                        json!({"type":"result","id":request["id"],"result":{}})
                    }
                    _ => panic!("unexpected shared command: {method}"),
                };
                extension
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        let clicked = client
            .call_tool(
                CallToolRequestParams::new("shared_input").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","selector":"a.story",
                    "action":"click","expected_value":"Story","value":"",
                    "postcondition_selector":"main h1","postcondition":"Story"
                }))),
            )
            .await
            .unwrap();
        let result = clicked.structured_content.unwrap();
        assert_eq!(result["navigation_observed"], true);
        assert_eq!(result["dispatch_acknowledged"], true);
        assert!(result.get("verified").is_none());
        assert_eq!(result["navigation"]["url"], "https://fixture.test/story");
        assert_eq!(result["navigation"]["loader_id"], "doc-after");
        extension_task.abort();
        client.cancel().await.unwrap();
        let _ = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn shared_double_click_does_not_promote_partial_delivery_from_focused_outcome() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let (server_io, client_io) = tokio::io::duplex(16_384);
        let server = serve_directly::<RoleServer, _, _, _, _>(App::default(), server_io, None);
        let server_task = tokio::spawn(async move { server.waiting().await });
        let client = ().serve(client_io).await.unwrap();
        let args = |value: Value| value.as_object().unwrap().clone();
        let paired = client
            .call_tool(
                CallToolRequestParams::new("session").with_arguments(args(json!({
                    "action":"pair_shared","target_ids":["123"]
                }))),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let endpoint = paired["endpoint"].as_str().unwrap().to_owned();
        let token = paired["one_session_token"].as_str().unwrap().to_owned();
        let session_id = paired["session_id"].as_str().unwrap().to_owned();
        let accept = client.call_tool(CallToolRequestParams::new("session").with_arguments(args(
            json!({"action":"accept_shared","session_id":session_id}),
        )));
        let extension = async move {
            let (mut socket, _) = connect_async(endpoint).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"hello","token":token,
                        "extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"targets":["123"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            socket
        };
        let (accepted, mut extension) = tokio::join!(accept, extension);
        assert_eq!(
            accepted.unwrap().structured_content.unwrap()["accepted"],
            true
        );

        let interaction = json!({
            "semantic_revision":1,
            "focus":"",
            "selection":"",
            "viewport":"[1280,800,0,0,1280,800,0,0,1]"
        });
        let public_snapshot = json!({
            "items":[{"name":"Search","role":"textbox","tag":"input","text":""}],
            "interaction_epoch":interaction,
            "complete":true,
            "text":"Search"
        });
        let outcome_polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let polls = outcome_polls.clone();
        let extension_task = tokio::spawn(async move {
            while let Some(Ok(message)) = extension.next().await {
                let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                let method = request["method"].as_str().unwrap();
                let params = &request["params"];
                let response = match method {
                    "Page.getFrameTree" => {
                        json!({"type":"result","id":request["id"],"result":{"frameTree":{"frame":{"id":"root","loaderId":"doc","url":"https://fixture.test/"}}}})
                    }
                    "Runtime.evaluate" => {
                        json!({"type":"result","id":request["id"],"result":{"result":{"type":"object","objectId":"snapshot-object"}}})
                    }
                    "Runtime.releaseObject" => {
                        json!({"type":"result","id":request["id"],"result":{}})
                    }
                    "Runtime.callFunctionOn"
                        if params["functionDeclaration"]
                            .as_str()
                            .unwrap_or_default()
                            .contains("this.public") =>
                    {
                        json!({"type":"result","id":request["id"],"result":{"result":{"type":"object","value":public_snapshot}}})
                    }
                    "Runtime.callFunctionOn"
                        if params["functionDeclaration"]
                            .as_str()
                            .unwrap_or_default()
                            .contains("interactionEpoch.v1") =>
                    {
                        json!({"type":"result","id":request["id"],"result":{"result":{"type":"object","value":interaction}}})
                    }
                    "Runtime.callFunctionOn"
                        if params["functionDeclaration"]
                            .as_str()
                            .unwrap_or_default()
                            .contains("q.kind === 'focused'") =>
                    {
                        let prior = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        json!({"type":"result","id":request["id"],"result":{"result":{"type":"boolean","value":prior > 0}}})
                    }
                    "Runtime.callFunctionOn" => {
                        json!({"type":"result","id":request["id"],"result":{"result":{"type":"object","value":{"ok":true,"x":10.0,"y":20.0,"viewport":"[1280,800,0,0,1280,800,0,0,1]"}}}})
                    }
                    "Controlla.pointerClick" => {
                        json!({"type":"result","id":request["id"],"result":{"status":"unknown","dispatch_acknowledged":false,"dispatch_may_have_occurred":true,"completed_clicks":1,"reason_code":"double_click_target_changed","reason":"The first click was sent, but the target changed before the second click."}})
                    }
                    _ => panic!("unexpected shared command: {method}"),
                };
                extension
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
            }
        });

        let snapshot = client
            .call_tool(CallToolRequestParams::new("shared_snapshot").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","max_text_chars":100,"max_bytes":8192
            }))))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let reference = format!("{}:0", snapshot["snapshot_id"].as_str().unwrap());
        let clicked = client
            .call_tool(
                CallToolRequestParams::new("shared_click").with_arguments(args(json!({
                    "session_id":session_id,"chrome_tab_id":"123","reference":reference,
                    "outcome":{"kind":"focused"},"click_kind":"double_click","timeout_ms":1000
                }))),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();

        assert_eq!(clicked["status"], "unknown");
        assert_eq!(clicked["reason_code"], "double_click_target_changed");
        assert_eq!(clicked["completed_clicks"], 1);
        assert_eq!(
            outcome_polls.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "only the pre-dispatch already-satisfied check may evaluate the outcome"
        );
        extension_task.abort();
        client.cancel().await.unwrap();
        let _ = server_task.await.unwrap();
    }
}

#[cfg(test)]
#[path = "shared_page_tests.rs"]
mod shared_page_tests;
