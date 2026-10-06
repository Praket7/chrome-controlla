#![deny(unsafe_code)]

mod blocking;
mod input;
mod manager;
pub mod providers;
pub mod scheduler;
mod session;
pub mod sessions;

pub use blocking::{BlockingBrowserManager, WaitGraphSnapshot};
pub use input::{
    GuardDecision, GuardSnapshot, GuardedInput, InputAction, InputOutcome, InvalidationSet,
    SemanticLocator, on_external_change, validate_step,
};
pub use manager::BrowserManager;
pub use session::{
    BrowserSession, CachedTargetState, SessionProvider, TargetStateCache,
    connect_permissioned_auto_connect, list_sessions, select_provider,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

/// Evidence stages for a browser upload. Generic CDP can normally establish
/// selection and transfer only; application acceptance and persistence require
/// an independent adapter or postcondition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum UploadStage {
    Selected,
    TransferStarted,
    TransferCompleted,
    ApplicationAccepted,
    Persisted,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UploadTransaction {
    pub operation_id: String,
    pub stage: UploadStage,
}

impl UploadTransaction {
    pub fn new(operation_id: impl Into<String>) -> Self {
        Self {
            operation_id: operation_id.into(),
            stage: UploadStage::Selected,
        }
    }

    pub fn advance(&mut self, next: UploadStage) -> Result<(), BrowserError> {
        let allowed = matches!(
            (self.stage, next),
            (UploadStage::Selected, UploadStage::TransferStarted)
                | (UploadStage::TransferStarted, UploadStage::TransferCompleted)
                | (
                    UploadStage::TransferCompleted,
                    UploadStage::ApplicationAccepted
                )
                | (UploadStage::ApplicationAccepted, UploadStage::Persisted)
                | (_, UploadStage::Failed)
        );
        if !allowed {
            return Err(BrowserError::InvalidResponse(format!(
                "invalid upload transition {:?} -> {:?}",
                self.stage, next
            )));
        }
        self.stage = next;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum DownloadStage {
    Started,
    InProgress,
    BrowserCompleted,
    FileVerified,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadTransaction {
    pub operation_id: String,
    pub guid: String,
    pub stage: DownloadStage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum Locator {
    RoleName { role: String, name: String },
    Label(String),
    Placeholder(String),
    Text(String),
    TestId(String),
    AltText(String),
    Href(String),
    Css(String),
    BackendNodeId(i64),
}

impl Locator {
    /// Parse the compact wire locator used by the core MCP surface.
    ///
    /// Keeping this conversion in the browser crate makes locator validation
    /// consistent across CDP and future native browser backends.
    pub fn from_value(value: &Value) -> Result<Self, BrowserError> {
        let object = value
            .as_object()
            .ok_or_else(|| BrowserError::InvalidResponse("locator must be an object".to_owned()))?;
        let string = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        };
        let locator = if let (Some(role), Some(name)) = (string("role"), string("name")) {
            Self::RoleName { role, name }
        } else if let Some(value) = string("label") {
            Self::Label(value)
        } else if let Some(value) = string("placeholder") {
            Self::Placeholder(value)
        } else if let Some(value) = string("text") {
            Self::Text(value)
        } else if let Some(value) = string("test_id") {
            Self::TestId(value)
        } else if let Some(value) = string("alt_text") {
            Self::AltText(value)
        } else if let Some(value) = string("href_contains") {
            Self::Href(value)
        } else if let Some(value) = string("selector") {
            Self::Css(value)
        } else if let Some(value) = object.get("backend_node_id").and_then(Value::as_i64) {
            Self::BackendNodeId(value)
        } else {
            return Err(BrowserError::InvalidResponse(
                "locator must provide one supported identity".to_owned(),
            ));
        };
        locator.validate()?;
        Ok(locator)
    }

    pub fn validate(&self) -> Result<(), BrowserError> {
        let valid = match self {
            Self::RoleName { role, name } => !role.trim().is_empty() && !name.trim().is_empty(),
            Self::Label(value)
            | Self::Placeholder(value)
            | Self::Text(value)
            | Self::TestId(value)
            | Self::AltText(value)
            | Self::Href(value)
            | Self::Css(value) => !value.trim().is_empty(),
            Self::BackendNodeId(value) => *value > 0,
        };
        if valid {
            Ok(())
        } else {
            Err(BrowserError::InvalidResponse(
                "locator must contain a non-empty, positive identity".to_owned(),
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Actionability {
    pub attached: bool,
    pub visible: bool,
    pub stable: bool,
    pub enabled: bool,
    pub receives_events: bool,
    pub unobscured: bool,
}

impl Actionability {
    pub fn click_ready(self) -> bool {
        self.attached
            && self.visible
            && self.stable
            && self.enabled
            && self.receives_events
            && self.unobscured
    }

    pub fn fill_ready(self) -> bool {
        self.attached && self.visible && self.stable && self.enabled
    }
}

impl DownloadTransaction {
    pub fn new(operation_id: impl Into<String>, guid: impl Into<String>) -> Self {
        Self {
            operation_id: operation_id.into(),
            guid: guid.into(),
            stage: DownloadStage::Started,
        }
    }

    pub fn advance(&mut self, next: DownloadStage) -> Result<(), BrowserError> {
        let allowed = matches!(
            (self.stage, next),
            (DownloadStage::Started, DownloadStage::InProgress)
                | (DownloadStage::InProgress, DownloadStage::BrowserCompleted)
                | (DownloadStage::BrowserCompleted, DownloadStage::FileVerified)
                | (_, DownloadStage::Failed)
        );
        if !allowed {
            return Err(BrowserError::InvalidResponse(format!(
                "invalid download transition {:?} -> {:?}",
                self.stage, next
            )));
        }
        self.stage = next;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("browser connection failed: {0}")]
    Connection(String),
    #[error("browser command channel closed")]
    Closed,
    #[error("browser response was invalid: {0}")]
    InvalidResponse(String),
    #[error("browser command cancelled")]
    Cancelled,
    #[error("browser event wait timed out")]
    Timeout,
    #[error("target reference is stale or unavailable: {0}")]
    StaleReference(String),
}

#[derive(Debug)]
struct OutgoingCommand {
    id: u64,
    session_id: Option<String>,
    method: String,
    params: Value,
    response: oneshot::Sender<Result<Value, BrowserError>>,
}

type PendingCommands = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, BrowserError>>>>>;

#[derive(Clone, Debug, PartialEq)]
pub struct BrowserEvent {
    pub sequence: u64,
    pub value: Value,
}

/// One browser-level WebSocket with a dedicated writer and reader.
/// Commands are correlated by id and never hold a global lock during I/O.
#[derive(Clone)]
pub struct BrowserConnection {
    outgoing: mpsc::Sender<OutgoingCommand>,
    instance_id: u128,
    scheduler: scheduler::TargetScheduler,
    pub targets: Arc<RwLock<TargetGraph>>,
    pub frames: Arc<RwLock<FrameGraph>>,
    next_command_id: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
    cancellation: CancellationToken,
    events: broadcast::Sender<BrowserEvent>,
    event_replay: Arc<Mutex<VecDeque<BrowserEvent>>>,
    event_sequence: Arc<AtomicU64>,
}

impl BrowserConnection {
    pub async fn connect(url: &str) -> Result<Self, BrowserError> {
        use futures_util::{SinkExt, StreamExt};
        let instance_id = new_browser_instance_id()?;
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| BrowserError::Connection(error.to_string()))?;
        let (mut writer, mut reader) = socket.split();
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<OutgoingCommand>(128);
        let pending: PendingCommands = Arc::new(Mutex::new(HashMap::new()));
        let pending_for_reader = Arc::clone(&pending);
        let targets = Arc::new(RwLock::new(TargetGraph::default()));
        let frames = Arc::new(RwLock::new(FrameGraph::default()));
        let targets_for_reader = Arc::clone(&targets);
        let frames_for_reader = Arc::clone(&frames);
        let generation_for_reader = Arc::new(AtomicU64::new(0));
        let generation_for_disconnect = Arc::clone(&generation_for_reader);
        let (events, _) = broadcast::channel(512);
        let events_for_reader = events.clone();
        let event_replay = Arc::new(Mutex::new(VecDeque::with_capacity(512)));
        let replay_for_reader = Arc::clone(&event_replay);
        let event_sequence = Arc::new(AtomicU64::new(0));
        let sequence_for_reader = Arc::clone(&event_sequence);
        let cancellation = CancellationToken::new();
        let cancellation_for_tasks = cancellation.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_for_tasks.cancelled() => break,
                    outgoing = outgoing_rx.recv() => {
                        let Some(command) = outgoing else { break };
                        let mut message = serde_json::json!({
                            "id": command.id,
                            "method": command.method,
                            "params": command.params,
                        });
                        if let Some(session_id) = command.session_id {
                            message["sessionId"] = Value::String(session_id);
                        }
                        pending_for_reader.lock().await.insert(command.id, command.response);
                        if writer.send(tokio_tungstenite::tungstenite::Message::Text(message.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let pending_for_responses = Arc::clone(&pending);
        let cancellation_for_reader = cancellation.clone();
        tokio::spawn(async move {
            while let Some(Ok(message)) = reader.next().await {
                let text = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text,
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        match String::from_utf8(bytes.to_vec()) {
                            Ok(text) => text.into(),
                            Err(_) => continue,
                        }
                    }
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(Value::as_u64) else {
                    let event = BrowserEvent {
                        sequence: sequence_for_reader.fetch_add(1, Ordering::AcqRel) + 1,
                        value: value.clone(),
                    };
                    let _ = events_for_reader.send(event.clone());
                    let mut replay = replay_for_reader.lock().await;
                    if replay.len() == 512 {
                        replay.pop_front();
                    }
                    replay.push_back(event);
                    let frame_target_id =
                        if let Some(session_id) = value.get("sessionId").and_then(Value::as_str) {
                            targets_for_reader
                                .read()
                                .await
                                .target_id_for_session(session_id)
                        } else {
                            None
                        };
                    targets_for_reader.write().await.apply_event(&value);
                    let generation = generation_for_disconnect.load(Ordering::Acquire);
                    let mut frames = frames_for_reader.write().await;
                    if let Some(target_id) = frame_target_id {
                        frames.apply_event_for_target(&value, generation, &target_id);
                    } else {
                        frames.apply_event_at_generation(&value, generation);
                    }
                    if value.get("method").and_then(Value::as_str) == Some("Target.targetDestroyed")
                        && let Some(target_id) = value
                            .get("params")
                            .and_then(|params| params.get("targetId"))
                            .and_then(Value::as_str)
                    {
                        frames.remove_target(target_id);
                    }
                    continue;
                };
                if let Some(sender) = pending_for_responses.lock().await.remove(&id) {
                    let result = if let Some(error) = value.get("error") {
                        Err(BrowserError::InvalidResponse(error.to_string()))
                    } else {
                        Ok(value.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = sender.send(result);
                }
            }
            cancellation_for_reader.cancel();
            let _generation = generation_for_disconnect.fetch_add(1, Ordering::AcqRel) + 1;
            targets_for_reader.write().await.reconnect();
            frames_for_reader.write().await.frames.clear();
            let mut pending = pending_for_responses.lock().await;
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(BrowserError::Closed));
            }
        });
        Ok(Self {
            outgoing: outgoing_tx,
            instance_id,
            scheduler: scheduler::TargetScheduler::default(),
            targets,
            frames,
            next_command_id: Arc::new(AtomicU64::new(1)),
            generation: generation_for_reader,
            cancellation,
            events,
            event_replay,
            event_sequence,
        })
    }

    pub async fn command(
        &self,
        session_id: Option<String>,
        method: impl Into<String>,
        params: Value,
    ) -> Result<Value, BrowserError> {
        if self.cancellation.is_cancelled() {
            return Err(BrowserError::Closed);
        }
        let id = self.next_command_id.fetch_add(1, Ordering::Relaxed);
        let (response_tx, response_rx) = oneshot::channel();
        self.outgoing
            .send(OutgoingCommand {
                id,
                session_id,
                method: method.into(),
                params,
                response: response_tx,
            })
            .await
            .map_err(|_| BrowserError::Closed)?;
        response_rx.await.map_err(|_| BrowserError::Cancelled)?
    }

    /// Create one target through this direct CDP provider and persist ownership
    /// only from the successful provider response. Shared-extension sessions
    /// require their own provider and cannot call this method.
    pub async fn create_owned_target(
        &self,
        registry: &mut crate::sessions::SessionRegistry,
        session: &crate::sessions::SessionHandle,
        url: &str,
    ) -> Result<String, BrowserError> {
        registry
            .authorize_target_creation(session)
            .map_err(|error| {
                BrowserError::InvalidResponse(format!(
                    "session cannot create a CDP target: {error:?}"
                ))
            })?;
        registry
            .validate_session_browser_binding(session, self.instance_id)
            .map_err(|error| {
                BrowserError::InvalidResponse(format!("session browser binding failed: {error:?}"))
            })?;
        let response = self
            .command(None, "Target.createTarget", serde_json::json!({"url": url}))
            .await?;
        let target_id = response
            .get("targetId")
            .and_then(Value::as_str)
            .filter(|target_id| !target_id.is_empty())
            .ok_or_else(|| {
                BrowserError::InvalidResponse(
                    "Target.createTarget response omitted targetId".to_owned(),
                )
            })?
            .to_owned();
        let binding_inserted =
            match registry.bind_session_to_browser_if_unbound(session, self.instance_id) {
                Ok(inserted) => inserted,
                Err(error) => {
                    let _ = self
                        .command(
                            None,
                            "Target.closeTarget",
                            serde_json::json!({"targetId":target_id}),
                        )
                        .await;
                    return Err(BrowserError::InvalidResponse(format!(
                        "session browser binding failed after target creation: {error:?}"
                    )));
                }
            };
        let receipt = crate::sessions::CreatedTargetReceipt::from_provider(
            &session.id,
            self.instance_id,
            target_id.clone(),
        );
        if let Err(error) = registry.record_created_tab(receipt) {
            if binding_inserted {
                registry.rollback_browser_binding_if_matches(&session.id, self.instance_id);
            }
            let _ = self
                .command(
                    None,
                    "Target.closeTarget",
                    serde_json::json!({"targetId":target_id}),
                )
                .await;
            return Err(BrowserError::InvalidResponse(format!(
                "created target could not be recorded: {error:?}"
            )));
        }
        Ok(target_id)
    }

    /// Close only owned targets that a trusted independent in-tab observer
    /// verifies immediately before dispatch. With no observer, all owned tabs
    /// remain in the receipt for later reconciliation.
    pub async fn release_owned_session(
        &self,
        registry: &mut crate::sessions::SessionRegistry,
        session_id: &str,
        observer: Option<&dyn crate::sessions::IndependentTargetObserver>,
    ) -> crate::sessions::CleanupReceipt {
        let mut outcomes = BTreeMap::new();
        for target_id in registry.owned_cleanup_candidates(session_id) {
            let Some(observer) = observer else {
                outcomes.insert(
                    target_id,
                    Err("no independent observer is available; target remains open".to_owned()),
                );
                continue;
            };
            let (generation, current) = {
                let graph = self.targets.read().await;
                (graph.generation, graph.targets.get(&target_id).cloned())
            };
            let Some(current) =
                current.filter(|target| target.attached && target.generation == generation)
            else {
                outcomes.insert(
                    target_id,
                    Err(
                        "owned target is no longer attached in the current browser generation"
                            .to_owned(),
                    ),
                );
                continue;
            };
            if !registry.session_bound_to_browser(session_id, self.instance_id) {
                outcomes.insert(
                    target_id,
                    Err("session/browser identity changed".to_owned()),
                );
                continue;
            }
            let observation = crate::sessions::CleanupObservation {
                session_id: session_id.to_owned(),
                target_id: target_id.clone(),
                browser_instance_id: self.instance_id,
                browser_generation: generation,
                target_revision: current.revision.clone(),
            };
            if let Err(reason) = observer.verify_unchanged(&observation) {
                outcomes.insert(
                    target_id,
                    Err(format!(
                        "independent observer did not confirm unchanged state: {reason}"
                    )),
                );
                continue;
            }
            // Revalidate the CDP snapshot after observation, then ask the
            // independent observer again at the last synchronous boundary.
            let still_current = {
                let graph = self.targets.read().await;
                graph.generation == generation
                    && graph.targets.get(&target_id).is_some_and(|target| {
                        target.attached
                            && target.generation == generation
                            && target.revision == current.revision
                    })
            } && registry
                .session_bound_to_browser(session_id, self.instance_id);
            if !still_current {
                outcomes.insert(
                    target_id,
                    Err("target changed during cleanup observation".to_owned()),
                );
                continue;
            }
            if let Err(reason) = observer.verify_unchanged(&observation) {
                outcomes.insert(
                    target_id,
                    Err(format!(
                        "independent observer confirmation expired: {reason}"
                    )),
                );
                continue;
            }
            let outcome = match self
                .command(
                    None,
                    "Target.closeTarget",
                    serde_json::json!({"targetId": target_id}),
                )
                .await
            {
                Ok(response) if response.get("success").and_then(Value::as_bool) == Some(true) => {
                    Ok(())
                }
                Ok(response) => Err(format!(
                    "Target.closeTarget did not confirm success: {response}"
                )),
                Err(error) => Err(error.to_string()),
            };
            outcomes.insert(target_id, outcome);
        }
        registry.release_session(session_id, |target_id| {
            outcomes
                .remove(target_id)
                .unwrap_or_else(|| Err("no provider close result was recorded".to_owned()))
        })
    }

    /// Receive the next protocol event from the shared browser reader. This
    /// is intentionally separate from command correlation so event waits never
    /// need to open or lock a target WebSocket.
    pub async fn next_event(&self, duration: Duration) -> Result<Value, BrowserError> {
        if let Some(event) = self.event_replay.lock().await.pop_front() {
            return Ok(event.value);
        }
        let mut receiver = self.events.subscribe();
        timeout(duration, receiver.recv())
            .await
            .map_err(|_| BrowserError::Timeout)?
            .map(|event| event.value)
            .map_err(|error| BrowserError::InvalidResponse(error.to_string()))
    }

    pub async fn next_event_after(
        &self,
        cursor: u64,
        duration: Duration,
    ) -> Result<BrowserEvent, BrowserError> {
        if let Some(event) = self
            .event_replay
            .lock()
            .await
            .iter()
            .find(|event| event.sequence > cursor)
            .cloned()
        {
            return Ok(event);
        }
        let mut receiver = self.events.subscribe();
        loop {
            let event = timeout(duration, receiver.recv())
                .await
                .map_err(|_| BrowserError::Timeout)?
                .map_err(|error| BrowserError::InvalidResponse(error.to_string()))?;
            if event.sequence > cursor {
                return Ok(event);
            }
        }
    }

    pub fn latest_event_sequence(&self) -> u64 {
        self.event_sequence.load(Ordering::Acquire)
    }

    pub async fn bootstrap(&self) -> Result<(), BrowserError> {
        self.command(
            None,
            "Target.setDiscoverTargets",
            serde_json::json!({"discover": true}),
        )
        .await?;
        self.command(
            None,
            "Target.setAutoAttach",
            serde_json::json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": false,
                "flatten": true
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn bootstrap_target(
        &self,
        session_id: impl Into<String>,
    ) -> Result<(), BrowserError> {
        let session_id = session_id.into();
        let target_id = self
            .targets
            .read()
            .await
            .target_id_for_session(&session_id)
            .ok_or_else(|| BrowserError::StaleReference(session_id.clone()))?;
        self.scheduler
            .run_target(&target_id, async {
                for method in [
                    "Page.enable",
                    "Runtime.enable",
                    "DOM.enable",
                    "Network.enable",
                    "Accessibility.enable",
                ] {
                    self.command(Some(session_id.clone()), method, Value::Null)
                        .await?;
                }
                Ok(())
            })
            .await
    }

    /// Snapshot the current target graph generation and matching page records
    /// without any network round trip. Restore verification and other waiters
    /// poll this in-memory view instead of issuing `/json/list` discovery; the
    /// persistent connection keeps the graph current from target events.
    pub async fn target_snapshot(&self) -> (u64, Vec<TargetRecord>) {
        let graph = self.targets.read().await;
        let records = graph
            .targets
            .values()
            .filter(|target| target.target_type == "page")
            .cloned()
            .collect();
        (graph.generation, records)
    }

    /// Enable the required domains for all targets attached during browser
    /// bootstrap. The graph is copied before dispatch so no graph lock is held
    /// across WebSocket I/O.
    pub async fn bootstrap_attached_targets(&self) -> Result<usize, BrowserError> {
        let sessions = {
            let graph = self.targets.read().await;
            graph
                .targets
                .values()
                .filter(|target| {
                    target.attached && matches!(target.target_type.as_str(), "page" | "iframe")
                })
                .filter_map(|target| target.session_id.clone())
                .collect::<Vec<_>>()
        };
        for session_id in &sessions {
            self.bootstrap_target(session_id.clone()).await?;
        }
        Ok(sessions.len())
    }

    /// Send a command to an already-attached target from the live graph.
    ///
    /// The caller must provide the target generation and revision it observed.
    /// This makes target binding explicit and prevents a warm operation from
    /// silently reusing a session after navigation, reconnect, or target
    /// replacement. Ordinary warm calls therefore do not need `/json/list`.
    pub async fn target_command(
        &self,
        target_id: &str,
        expected_generation: u64,
        expected_revision: &str,
        method: impl Into<String>,
        params: Value,
    ) -> Result<Value, BrowserError> {
        let method = method.into();
        self.scheduler
            .run_target(
                target_id,
                self.target_command_unlocked(
                    target_id,
                    expected_generation,
                    expected_revision,
                    method,
                    params,
                ),
            )
            .await
    }

    /// Serialize a mutation with other tabs that the caller has resolved to
    /// the same application document. The document key must come from the
    /// caller's identity observer.
    pub async fn target_document_mutation(
        &self,
        target_id: &str,
        expected_generation: u64,
        expected_revision: &str,
        document_id: &str,
        method: impl Into<String>,
        params: Value,
    ) -> Result<Value, BrowserError> {
        self.scheduler
            .run_document_mutation(
                target_id,
                document_id,
                self.target_command_unlocked(
                    target_id,
                    expected_generation,
                    expected_revision,
                    method.into(),
                    params,
                ),
            )
            .await
    }

    async fn target_command_unlocked(
        &self,
        target_id: &str,
        expected_generation: u64,
        expected_revision: &str,
        method: String,
        params: Value,
    ) -> Result<Value, BrowserError> {
        let session_id = {
            let graph = self.targets.read().await;
            let target = graph
                .targets
                .get(target_id)
                .ok_or_else(|| BrowserError::StaleReference(target_id.to_owned()))?;
            if graph.generation != expected_generation
                || target.generation != expected_generation
                || target.revision != expected_revision
                || !target.attached
            {
                return Err(BrowserError::StaleReference(format!(
                    "target {target_id} generation/revision changed"
                )));
            }
            target
                .session_id
                .clone()
                .ok_or_else(|| BrowserError::StaleReference(target_id.to_owned()))?
        };
        self.command(Some(session_id), method, params).await
    }

    /// Capture a target reference from this connection's event-maintained
    /// target and frame graphs.
    pub async fn capture_target_ref(
        &self,
        sessions: &crate::sessions::SessionRegistry,
        session: &crate::sessions::SessionHandle,
        target_id: &str,
        frame_id: &str,
        account_revision: u64,
        document_revision: u64,
    ) -> Result<crate::sessions::TargetRef, crate::sessions::StaleTarget> {
        let target = self
            .targets
            .read()
            .await
            .targets
            .get(target_id)
            .cloned()
            .ok_or(crate::sessions::StaleTarget::TargetChanged)?;
        let frame = self
            .frames
            .read()
            .await
            .frames
            .get(frame_id)
            .cloned()
            .ok_or(crate::sessions::StaleTarget::FrameChanged)?;
        crate::sessions::TargetRef::capture(
            sessions,
            session,
            self.instance_id,
            &target,
            &frame,
            account_revision,
            document_revision,
        )
    }

    /// Resolve a saved reference against the current target/frame graph and
    /// current session grant. Account/document revisions are supplied by the
    /// caller's identity observer; this crate does not infer app identity.
    pub async fn resolve_target_ref(
        &self,
        sessions: &crate::sessions::SessionRegistry,
        reference: &crate::sessions::TargetRef,
        principal: &str,
        account_revision: u64,
        document_revision: u64,
    ) -> Result<ResolvedTarget, crate::sessions::StaleTarget> {
        let target = self
            .targets
            .read()
            .await
            .targets
            .get(&reference.target_id)
            .cloned()
            .ok_or(crate::sessions::StaleTarget::TargetChanged)?;
        let frame = self
            .frames
            .read()
            .await
            .frames
            .get(&reference.frame_id)
            .cloned()
            .ok_or(crate::sessions::StaleTarget::FrameChanged)?;
        let identity = crate::sessions::TargetIdentity::from_snapshot(
            self.instance_id,
            &reference.session_id,
            &target,
            &frame,
            account_revision,
            document_revision,
        )?;
        sessions.resolve_active_target(reference, principal, &identity)?;
        Ok(ResolvedTarget {
            target,
            frame,
            identity,
        })
    }

    /// Resolve and revalidate the identity immediately before dispatch through
    /// the target's current flattened-session binding.
    pub async fn target_ref_command(
        &self,
        sessions: &crate::sessions::SessionRegistry,
        reference: &crate::sessions::TargetRef,
        principal: &str,
        identity_revisions: crate::sessions::IdentityRevisions,
        method: impl Into<String>,
        params: Value,
    ) -> Result<Value, BrowserError> {
        sessions
            .authorize_direct_cdp(&reference.session_id)
            .map_err(|stale| BrowserError::StaleReference(format!("{stale:?}")))?;
        let resolved = self
            .resolve_target_ref(
                sessions,
                reference,
                principal,
                identity_revisions.account,
                identity_revisions.document,
            )
            .await
            .map_err(|stale| BrowserError::StaleReference(format!("{stale:?}")))?;
        let method = method.into();
        let mut params = params;
        if method == "Runtime.evaluate"
            && let Some(context_id) = resolved.frame.execution_context_ids.last()
            && let Some(object) = params.as_object_mut()
        {
            object.insert("contextId".to_owned(), Value::from(*context_id));
        }
        self.target_command(
            &reference.target_id,
            reference.browser_generation,
            &reference.target_revision,
            method,
            params,
        )
        .await
    }

    /// Send a Runtime command in the execution context owned by a specific
    /// frame. OOPIF events arrive on the browser socket with a flattened
    /// session id, so the frame graph is the authoritative session/context
    /// binding and no second page WebSocket is needed.
    pub async fn frame_command(
        &self,
        frame_id: &str,
        expected_generation: u64,
        expected_revision: u64,
        method: impl Into<String>,
        mut params: Value,
    ) -> Result<Value, BrowserError> {
        let method = method.into();
        let target_id = self
            .frames
            .read()
            .await
            .frames
            .get(frame_id)
            .map(|frame| frame.target_id.clone())
            .ok_or_else(|| BrowserError::StaleReference(frame_id.to_owned()))?;
        self.scheduler
            .run_target(&target_id, async {
                let (current_target_id, context_id) = {
                    let frames = self.frames.read().await;
                    let frame = frames
                        .frames
                        .get(frame_id)
                        .ok_or_else(|| BrowserError::StaleReference(frame_id.to_owned()))?;
                    if frame.generation != expected_generation
                        || frame.revision != expected_revision
                    {
                        return Err(BrowserError::StaleReference(format!(
                            "frame {frame_id} generation/revision changed"
                        )));
                    }
                    (
                        frame.target_id.clone(),
                        frame.execution_context_ids.last().copied(),
                    )
                };
                if current_target_id != target_id {
                    return Err(BrowserError::StaleReference(frame_id.to_owned()));
                }
                let target = {
                    let targets = self.targets.read().await;
                    let target = targets
                        .targets
                        .get(&target_id)
                        .ok_or_else(|| BrowserError::StaleReference(target_id.clone()))?;
                    if targets.generation != expected_generation
                        || target.generation != expected_generation
                        || !target.attached
                    {
                        return Err(BrowserError::StaleReference(target_id.clone()));
                    }
                    target
                        .session_id
                        .clone()
                        .ok_or_else(|| BrowserError::StaleReference(target_id.clone()))?
                };
                if method == "Runtime.evaluate"
                    && let Some(context_id) = context_id
                    && let Some(object) = params.as_object_mut()
                {
                    object.insert("contextId".to_owned(), Value::from(context_id));
                }
                self.command(Some(target), method, params).await
            })
            .await
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn is_closed(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub async fn reconnect_generation(&self) -> u64 {
        let next = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.targets.write().await.reconnect();
        next
    }
}

fn new_browser_instance_id() -> Result<u128, BrowserError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        BrowserError::Connection(format!("OS random instance ID unavailable: {error}"))
    })?;
    Ok(u128::from_be_bytes(bytes))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TargetRecord {
    pub id: String,
    pub target_type: String,
    pub browser_context_id: Option<String>,
    pub session_id: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub opener_id: Option<String>,
    pub attached: bool,
    pub generation: u64,
    pub revision: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTarget {
    pub target: TargetRecord,
    pub frame: FrameRecord,
    pub identity: crate::sessions::TargetIdentity,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TargetGraph {
    pub generation: u64,
    pub targets: BTreeMap<String, TargetRecord>,
    #[serde(default)]
    revision_sequence: u64,
}

impl TargetGraph {
    fn next_revision(&mut self, id: &str) -> String {
        self.revision_sequence = self.revision_sequence.saturating_add(1);
        format!(
            "generation:{}:target:{}:revision:{}",
            self.generation, id, self.revision_sequence
        )
    }

    pub fn apply_created(&mut self, mut target: TargetRecord) {
        target.generation = self.generation;
        target.revision = self.next_revision(&target.id);
        self.targets.insert(target.id.clone(), target);
    }

    pub fn apply_changed(&mut self, id: &str, url: Option<String>, title: Option<String>) {
        if self.targets.contains_key(id) {
            let target = self.targets.get_mut(id).expect("target was checked above");
            let url_changed = url
                .as_ref()
                .is_some_and(|value| target.url.as_ref() != Some(value));
            if let Some(url) = url {
                target.url = Some(url);
            }
            if let Some(title) = title {
                target.title = Some(title);
            }
            if url_changed {
                let revision = self.next_revision(id);
                self.targets
                    .get_mut(id)
                    .expect("target was checked above")
                    .revision = revision;
            }
        }
    }

    pub fn apply_destroyed(&mut self, id: &str) {
        self.targets.remove(id);
    }

    pub fn target_id_for_session(&self, session_id: &str) -> Option<String> {
        self.targets
            .values()
            .find(|target| target.session_id.as_deref() == Some(session_id))
            .map(|target| target.id.clone())
    }

    pub fn reconnect(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.revision_sequence = self.revision_sequence.saturating_add(1);
        let revision_sequence = self.revision_sequence;
        for target in self.targets.values_mut() {
            target.attached = false;
            target.session_id = None;
            target.generation = self.generation;
            target.revision = format!(
                "generation:{}:target:{}:revision:{}",
                self.generation, target.id, revision_sequence
            );
        }
    }

    pub fn apply_event(&mut self, event: &Value) {
        match event.get("method").and_then(Value::as_str) {
            Some("Target.targetCreated") => {
                if let Some(info) = event
                    .get("params")
                    .and_then(|params| params.get("targetInfo"))
                {
                    let id = info
                        .get("targetId")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !id.is_empty() {
                        self.apply_created(TargetRecord {
                            id: id.to_owned(),
                            target_type: info
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            browser_context_id: info
                                .get("browserContextId")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            session_id: None,
                            url: info.get("url").and_then(Value::as_str).map(str::to_owned),
                            title: info.get("title").and_then(Value::as_str).map(str::to_owned),
                            opener_id: info
                                .get("openerId")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            attached: false,
                            generation: self.generation,
                            revision: String::new(),
                        });
                    }
                }
            }
            Some("Target.targetInfoChanged") => {
                if let Some(info) = event
                    .get("params")
                    .and_then(|params| params.get("targetInfo"))
                {
                    let id = info
                        .get("targetId")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !id.is_empty() {
                        self.apply_changed(
                            id,
                            info.get("url").and_then(Value::as_str).map(str::to_owned),
                            info.get("title").and_then(Value::as_str).map(str::to_owned),
                        );
                    }
                }
            }
            Some("Target.targetDestroyed") => {
                if let Some(id) = event
                    .get("params")
                    .and_then(|params| params.get("targetId"))
                    .and_then(Value::as_str)
                {
                    self.apply_destroyed(id);
                }
            }
            Some("Target.attachedToTarget") => {
                if let Some(params) = event.get("params") {
                    let target_id = params
                        .get("targetInfo")
                        .and_then(|info| info.get("targetId"))
                        .and_then(Value::as_str);
                    let target_id =
                        target_id.or_else(|| params.get("targetId").and_then(Value::as_str));
                    if let (Some(target_id), Some(session_id)) =
                        (target_id, params.get("sessionId").and_then(Value::as_str))
                        && self.targets.contains_key(target_id)
                    {
                        let revision = self.next_revision(target_id);
                        let target = self
                            .targets
                            .get_mut(target_id)
                            .expect("target was checked above");
                        target.session_id = Some(session_id.to_owned());
                        target.attached = true;
                        target.generation = self.generation;
                        target.revision = revision;
                    }
                }
            }
            Some("Target.detachedFromTarget") => {
                if let Some(params) = event.get("params") {
                    let target_id = params.get("targetId").and_then(Value::as_str);
                    if let Some(target_id) = target_id
                        && self.targets.contains_key(target_id)
                    {
                        let revision = self.next_revision(target_id);
                        let target = self
                            .targets
                            .get_mut(target_id)
                            .expect("target was checked above");
                        target.session_id = None;
                        target.attached = false;
                        target.revision = revision;
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FrameRecord {
    pub id: String,
    pub parent_id: Option<String>,
    pub target_id: String,
    pub loader_id: Option<String>,
    pub execution_context_ids: Vec<u64>,
    pub generation: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FrameGraph {
    pub frames: BTreeMap<String, FrameRecord>,
    #[serde(default)]
    revision_sequence: u64,
}

impl FrameGraph {
    fn next_revision(&mut self) -> u64 {
        self.revision_sequence = self.revision_sequence.saturating_add(1);
        self.revision_sequence
    }

    pub fn upsert(&mut self, mut frame: FrameRecord) {
        frame.revision = self.next_revision();
        self.frames.insert(frame.id.clone(), frame);
    }

    pub fn remove(&mut self, id: &str) {
        let mut removed = BTreeSet::from([id.to_owned()]);
        loop {
            let previous_len = removed.len();
            for frame in self.frames.values() {
                if frame
                    .parent_id
                    .as_ref()
                    .is_some_and(|parent_id| removed.contains(parent_id))
                {
                    removed.insert(frame.id.clone());
                }
            }
            if removed.len() == previous_len {
                break;
            }
        }
        self.frames
            .retain(|frame_id, _| !removed.contains(frame_id));
    }

    pub fn remove_target(&mut self, target_id: &str) {
        let roots = self
            .frames
            .values()
            .filter(|frame| frame.target_id == target_id)
            .map(|frame| frame.id.clone())
            .collect::<Vec<_>>();
        for root in roots {
            self.remove(&root);
        }
    }

    pub fn apply_event(&mut self, event: &Value) {
        self.apply_event_at_generation(event, 0);
    }

    pub fn apply_event_for_target(&mut self, event: &Value, generation: u64, target_id: &str) {
        let mut event = event.clone();
        if let Some(params) = event.get_mut("params").and_then(Value::as_object_mut) {
            params
                .entry("targetId")
                .or_insert_with(|| Value::String(target_id.to_owned()));
        }
        self.apply_event_at_generation(&event, generation);
    }

    pub fn apply_event_at_generation(&mut self, event: &Value, generation: u64) {
        match event.get("method").and_then(Value::as_str) {
            Some("Page.frameAttached") => {
                if let Some(params) = event.get("params")
                    && let Some(id) = params.get("frameId").and_then(Value::as_str)
                {
                    self.upsert(FrameRecord {
                        id: id.to_owned(),
                        parent_id: params
                            .get("parentFrameId")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        target_id: params
                            .get("targetId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        loader_id: None,
                        execution_context_ids: Vec::new(),
                        generation,
                        revision: 0,
                    });
                }
            }
            Some("Page.frameNavigated") => {
                let frame_id = event
                    .get("params")
                    .and_then(|params| params.get("frame"))
                    .and_then(|frame| frame.get("id").and_then(Value::as_str))
                    .map(str::to_owned);
                if let Some(frame_id) = frame_id {
                    let info = event.get("params").and_then(|params| params.get("frame"));
                    let parent_id = info
                        .and_then(|value| value.get("parentId"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let target_id = event
                        .get("params")
                        .and_then(|params| params.get("targetId"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let loader_id = info
                        .and_then(|value| value.get("loaderId"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    if self.frames.contains_key(&frame_id) {
                        let revision = self.next_revision();
                        let frame = self
                            .frames
                            .get_mut(&frame_id)
                            .expect("frame was checked above");
                        frame.parent_id = parent_id;
                        frame.target_id = target_id;
                        frame.loader_id = loader_id;
                        frame.generation = generation;
                        frame.revision = revision;
                    } else {
                        self.upsert(FrameRecord {
                            id: frame_id,
                            parent_id,
                            target_id,
                            loader_id,
                            execution_context_ids: Vec::new(),
                            generation,
                            revision: 0,
                        });
                    }
                }
            }
            Some("Page.navigatedWithinDocument") => {
                if let Some(frame_id) = event
                    .get("params")
                    .and_then(|params| params.get("frameId"))
                    .and_then(Value::as_str)
                    && self.frames.contains_key(frame_id)
                {
                    let revision = self.next_revision();
                    let frame = self
                        .frames
                        .get_mut(frame_id)
                        .expect("frame was checked above");
                    frame.generation = generation;
                    frame.revision = revision;
                }
            }
            Some("Page.frameDetached") => {
                if let Some(id) = event
                    .get("params")
                    .and_then(|params| params.get("frameId"))
                    .and_then(Value::as_str)
                {
                    self.remove(id);
                }
            }
            Some("Runtime.executionContextCreated") => {
                if let Some(context) = event.get("params").and_then(|params| params.get("context"))
                    && let (Some(frame_id), Some(context_id)) = (
                        context
                            .get("auxData")
                            .and_then(|data| data.get("frameId"))
                            .and_then(Value::as_str),
                        context.get("id").and_then(Value::as_u64),
                    )
                    && self.frames.contains_key(frame_id)
                {
                    let should_add = !self.frames[frame_id]
                        .execution_context_ids
                        .contains(&context_id);
                    if should_add {
                        let revision = self.next_revision();
                        let frame = self
                            .frames
                            .get_mut(frame_id)
                            .expect("frame was checked above");
                        frame.execution_context_ids.push(context_id);
                        frame.generation = generation;
                        frame.revision = revision;
                    }
                }
            }
            Some("Runtime.executionContextDestroyed") => {
                if let Some(context_id) = event
                    .get("params")
                    .and_then(|params| params.get("executionContextId"))
                    .and_then(Value::as_u64)
                {
                    let affected = self
                        .frames
                        .values()
                        .filter(|frame| frame.execution_context_ids.contains(&context_id))
                        .map(|frame| frame.id.clone())
                        .collect::<Vec<_>>();
                    for frame_id in affected {
                        let revision = self.next_revision();
                        let frame = self
                            .frames
                            .get_mut(&frame_id)
                            .expect("affected frame remains present");
                        frame.execution_context_ids.retain(|id| *id != context_id);
                        frame.generation = generation;
                        frame.revision = revision;
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BrowserCommand {
    pub id: u64,
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex as AsyncMutex;

    struct FixtureIndependentObserver;

    impl crate::sessions::IndependentTargetObserver for FixtureIndependentObserver {
        fn verify_unchanged(
            &self,
            observation: &crate::sessions::CleanupObservation,
        ) -> Result<(), String> {
            assert_eq!(observation.target_id, "provider-owned-tab");
            Ok(())
        }
    }

    struct FixtureRevisionObserver {
        revision: String,
    }

    impl crate::sessions::IndependentTargetObserver for FixtureRevisionObserver {
        fn verify_unchanged(
            &self,
            observation: &crate::sessions::CleanupObservation,
        ) -> Result<(), String> {
            if observation.target_revision == self.revision {
                Ok(())
            } else {
                Err("target revision changed".to_owned())
            }
        }
    }

    async fn direct_dispatch_peak(target_count: usize, shared_document: bool) -> usize {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for index in 0..target_count {
                let target_id = format!("dispatch-target-{index}");
                let session_id = format!("dispatch-session-{index}");
                for event in [
                    serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":target_id,"type":"page","url":"https://fixture.test/"}}}),
                    serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":session_id,"targetInfo":{"targetId":target_id,"type":"page"}}}),
                ] {
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            event.to_string().into(),
                        ))
                        .await
                        .unwrap();
                }
            }
            let (writer, mut reader) = socket.split();
            let writer = Arc::new(AsyncMutex::new(writer));
            let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let expected_wave = if shared_document {
                1
            } else {
                target_count.min(4)
            };
            let mut remaining = target_count;
            while remaining > 0 {
                // Do not let a fast fixture response race a slow test runner's
                // first-wave dispatch. Record an entire scheduler wave before
                // sending any response, then repeat for later waves.
                let wave_size = remaining.min(expected_wave);
                let mut wave = Vec::with_capacity(wave_size);
                for _ in 0..wave_size {
                    let request = tokio::time::timeout(Duration::from_secs(2), reader.next())
                        .await
                        .expect("dispatch wave did not arrive")
                        .unwrap()
                        .unwrap();
                    let request: Value = serde_json::from_str(&request.to_string()).unwrap();
                    let active_now = active.fetch_add(1, Ordering::AcqRel) + 1;
                    peak.fetch_max(active_now, Ordering::AcqRel);
                    wave.push(request);
                }
                let mut replies = Vec::with_capacity(wave_size);
                for request in wave {
                    let writer = Arc::clone(&writer);
                    let active = Arc::clone(&active);
                    replies.push(tokio::spawn(async move {
                        writer.lock().await.send(tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::json!({"id":request["id"],"result":{"result":{"type":"number","value":2}}}).to_string().into()
                        )).await.unwrap();
                        active.fetch_sub(1, Ordering::AcqRel);
                    }));
                }
                for reply in replies {
                    reply.await.unwrap();
                }
                remaining -= wave_size;
            }
            peak.load(Ordering::Acquire)
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let targets = timeout(Duration::from_secs(1), async {
            loop {
                let (generation, targets) = connection.target_snapshot().await;
                if targets.iter().filter(|target| target.attached).count() == target_count {
                    break (generation, targets);
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut commands = tokio::task::JoinSet::new();
        for target in targets.1.into_iter().filter(|target| target.attached) {
            let connection = connection.clone();
            let generation = targets.0;
            commands.spawn(async move {
                let result = if shared_document {
                    connection
                        .target_document_mutation(
                            &target.id,
                            generation,
                            &target.revision,
                            "same-document",
                            "Runtime.evaluate",
                            serde_json::Value::Null,
                        )
                        .await
                } else {
                    connection
                        .target_command(
                            &target.id,
                            generation,
                            &target.revision,
                            "Runtime.evaluate",
                            serde_json::Value::Null,
                        )
                        .await
                };
                result.unwrap();
            });
        }
        while commands.join_next().await.is_some() {}
        server.await.unwrap()
    }

    fn target() -> TargetRecord {
        TargetRecord {
            id: "tab".to_owned(),
            target_type: "page".to_owned(),
            browser_context_id: Some("default".to_owned()),
            session_id: Some("session".to_owned()),
            url: Some("https://example.test".to_owned()),
            title: Some("Example".to_owned()),
            opener_id: None,
            attached: true,
            generation: 0,
            revision: "generation:0:target:tab".to_owned(),
        }
    }

    #[test]
    fn upload_transaction_requires_ordered_evidence() {
        let mut transaction = UploadTransaction::new("upload-1");
        assert_eq!(transaction.stage, UploadStage::Selected);
        assert!(transaction.advance(UploadStage::TransferCompleted).is_err());
        transaction.advance(UploadStage::TransferStarted).unwrap();
        transaction.advance(UploadStage::TransferCompleted).unwrap();
        transaction
            .advance(UploadStage::ApplicationAccepted)
            .unwrap();
        transaction.advance(UploadStage::Persisted).unwrap();
        assert_eq!(transaction.stage, UploadStage::Persisted);
    }

    #[test]
    fn upload_failure_is_terminal_and_cannot_be_overclaimed() {
        let mut transaction = UploadTransaction::new("upload-2");
        transaction.advance(UploadStage::Failed).unwrap();
        assert!(transaction.advance(UploadStage::Persisted).is_err());
        assert_eq!(transaction.stage, UploadStage::Failed);
    }

    #[test]
    fn download_transaction_requires_browser_and_filesystem_evidence() {
        let mut transaction = DownloadTransaction::new("download-1", "guid-1");
        assert!(transaction.advance(DownloadStage::FileVerified).is_err());
        transaction.advance(DownloadStage::InProgress).unwrap();
        transaction
            .advance(DownloadStage::BrowserCompleted)
            .unwrap();
        transaction.advance(DownloadStage::FileVerified).unwrap();
        assert_eq!(transaction.stage, DownloadStage::FileVerified);
    }

    #[test]
    fn wire_locators_are_typed_and_reject_empty_identity() {
        assert_eq!(
            Locator::from_value(&serde_json::json!({"selector": "#save"})).unwrap(),
            Locator::Css("#save".to_owned())
        );
        assert_eq!(
            Locator::from_value(&serde_json::json!({"role": "button", "name": "Save"})).unwrap(),
            Locator::RoleName {
                role: "button".to_owned(),
                name: "Save".to_owned()
            }
        );
        assert!(Locator::from_value(&serde_json::json!({"selector": " "})).is_err());
        assert!(Locator::from_value(&serde_json::json!({})).is_err());
    }

    #[test]
    fn locators_and_actionability_require_safe_targets() {
        assert!(Locator::Text("  ".to_owned()).validate().is_err());
        assert!(Locator::Css("button.save".to_owned()).validate().is_ok());
        assert!(Locator::BackendNodeId(0).validate().is_err());
        let ready = Actionability {
            attached: true,
            visible: true,
            stable: true,
            enabled: true,
            receives_events: true,
            unobscured: true,
        };
        assert!(ready.click_ready());
        assert!(ready.fill_ready());
        assert!(
            !Actionability {
                unobscured: false,
                ..ready
            }
            .click_ready()
        );
    }

    #[test]
    fn reconnect_invalidates_sessions_and_increments_generation() {
        let mut graph = TargetGraph::default();
        graph.apply_created(target());
        graph.reconnect();
        let target = graph.targets.get("tab").unwrap();
        assert_eq!(graph.generation, 1);
        assert_eq!(target.session_id, None);
        assert!(!target.attached);
        assert_eq!(target.generation, 1);
    }

    #[test]
    fn target_close_and_reuse_get_a_new_revision() {
        let mut graph = TargetGraph::default();
        graph.apply_created(target());
        let first_revision = graph.targets["tab"].revision.clone();
        graph.apply_destroyed("tab");
        graph.apply_created(target());
        assert_ne!(graph.targets["tab"].revision, first_revision);
    }

    #[test]
    fn target_navigation_advances_the_stale_reference_revision() {
        let mut graph = TargetGraph::default();
        graph.apply_created(target());
        let first_revision = graph.targets["tab"].revision.clone();
        graph.apply_changed(
            "tab",
            Some("https://example.test/next".to_owned()),
            Some("Next".to_owned()),
        );
        assert_ne!(graph.targets["tab"].revision, first_revision);
    }

    #[test]
    fn title_only_target_change_preserves_target_identity_revision() {
        let mut graph = TargetGraph::default();
        graph.apply_created(target());
        let revision = graph.targets["tab"].revision.clone();
        graph.apply_changed("tab", None, Some("Renamed tab".to_owned()));
        assert_eq!(graph.targets["tab"].revision, revision);
        assert_eq!(graph.targets["tab"].title.as_deref(), Some("Renamed tab"));
    }

    #[test]
    fn frame_detach_and_reuse_advance_revision() {
        let mut graph = FrameGraph::default();
        let frame = FrameRecord {
            id: "frame".to_owned(),
            parent_id: None,
            target_id: "tab".to_owned(),
            loader_id: Some("loader-1".to_owned()),
            execution_context_ids: vec![1],
            generation: 0,
            revision: 0,
        };
        graph.upsert(frame.clone());
        let first_revision = graph.frames["frame"].revision;
        graph.remove("frame");
        graph.upsert(FrameRecord {
            loader_id: Some("loader-2".to_owned()),
            ..frame
        });
        assert!(graph.frames["frame"].revision > first_revision);
    }

    #[test]
    fn frame_detach_removes_all_descendants() {
        let mut graph = FrameGraph::default();
        for (id, parent_id) in [
            ("root", None),
            ("child", Some("root")),
            ("grandchild", Some("child")),
        ] {
            graph.upsert(FrameRecord {
                id: id.to_owned(),
                parent_id: parent_id.map(str::to_owned),
                target_id: "tab".to_owned(),
                loader_id: None,
                execution_context_ids: Vec::new(),
                generation: 0,
                revision: 0,
            });
        }
        graph.remove("root");
        assert!(graph.frames.is_empty());
    }

    #[test]
    fn execution_context_destruction_invalidates_frame_revision() {
        let mut graph = FrameGraph::default();
        graph.upsert(FrameRecord {
            id: "frame".to_owned(),
            parent_id: None,
            target_id: "tab".to_owned(),
            loader_id: Some("loader".to_owned()),
            execution_context_ids: vec![17],
            generation: 0,
            revision: 0,
        });
        let revision = graph.frames["frame"].revision;
        graph.apply_event_at_generation(
            &serde_json::json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {"executionContextId": 17}
            }),
            0,
        );
        assert!(graph.frames["frame"].revision > revision);
        assert!(graph.frames["frame"].execution_context_ids.is_empty());
    }

    #[test]
    fn flattened_frame_event_uses_target_bound_to_session_id() {
        let mut graph = FrameGraph::default();
        graph.apply_event_for_target(
            &serde_json::json!({
                "method": "Page.frameAttached",
                "params": {"frameId": "oopif-frame", "parentFrameId": "root"}
            }),
            2,
            "oopif-target",
        );
        assert_eq!(graph.frames["oopif-frame"].target_id, "oopif-target");
    }

    #[test]
    fn first_seen_main_frame_navigation_creates_capturable_frame() {
        let mut graph = FrameGraph::default();
        graph.apply_event_for_target(
            &serde_json::json!({
                "method":"Page.frameNavigated",
                "params":{"frame":{"id":"first-main","loaderId":"loader-1","url":"https://fixture.test/"}}
            }),
            4,
            "target-1",
        );
        let frame = graph.frames.get("first-main").expect("frame was created");
        assert_eq!(frame.target_id, "target-1");
        assert_eq!(frame.generation, 4);
        assert_eq!(frame.loader_id.as_deref(), Some("loader-1"));
    }

    #[tokio::test]
    async fn browser_dispatch_enforces_four_active_targets_for_one_four_and_eight() {
        for count in [1, 4, 8] {
            assert_eq!(direct_dispatch_peak(count, false).await, count.min(4));
        }
        assert_eq!(direct_dispatch_peak(4, true).await, 1);
    }

    #[tokio::test]
    async fn guarded_text_actions_use_revision_bound_cdp_and_race_yields() {
        use tokio_tungstenite::tungstenite::Message;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (race_event_seen_tx, race_event_seen_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for event in [
                serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"input-tab","type":"page","url":"https://fixture.test/"}}}),
                serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":"input-session","targetInfo":{"targetId":"input-tab","type":"page"}}}),
                serde_json::json!({"sessionId":"input-session","method":"Page.frameNavigated","params":{"frame":{"id":"input-frame","loaderId":"input-load","url":"https://fixture.test/"}}}),
            ] {
                socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .unwrap();
            }
            let mut methods = Vec::new();
            let mut guard_started = None;
            let mut measured_window_micros = 0;
            let mut race_event_seen_rx = Some(race_event_seen_rx);
            let dom_value = |value: &str| serde_json::json!({"ok":true,"count":1,"tag":"INPUT","type":"text","editable":false,"disabled":false,"visible":true,"rect":{"x":0,"y":0,"width":100,"height":20},"hit":true,"value":value,"selectionStart":value.encode_utf16().count(),"selectionEnd":value.encode_utf16().count()});
            while methods.len() < 32 {
                let msg = socket.next().await.unwrap().unwrap();
                let req: Value = serde_json::from_str(&msg.to_string()).unwrap();
                if methods.is_empty() {
                    guard_started = Some(std::time::Instant::now());
                }
                methods.push(req["method"].as_str().unwrap().to_owned());
                let n = methods.len();
                if n == 3 {
                    measured_window_micros = guard_started.unwrap().elapsed().as_micros();
                    assert!(
                        req["params"]["expression"]
                            .as_str()
                            .unwrap()
                            .contains("e.value!==\"\"")
                    );
                }
                let method = req["method"].as_str().unwrap();
                let response = if method != "Runtime.evaluate" {
                    serde_json::json!({"id":req["id"],"sessionId":"input-session","result":{}})
                } else {
                    let value = match n {
                        1 => dom_value(""),
                        2 | 6 | 7 | 11 | 12 | 14 | 16 => Value::Bool(true),
                        3 => serde_json::json!({"stale":false,"applied":true}),
                        4 | 5 | 8 => dom_value("héllo 👋"),
                        9 | 10 => dom_value("héllo 👋!"),
                        13 | 15 | 17 => Value::Bool(true),
                        18 | 19 | 24 | 31 | 32 => dom_value("héllo 👋!a"),
                        20 | 23 | 25 | 26 | 30 | 33 => Value::Bool(true),
                        21 | 22 | 27 | 28 | 29 => Value::Bool(true),
                        _ => panic!("unexpected evaluate request {n}"),
                    };
                    serde_json::json!({"id":req["id"],"sessionId":"input-session","result":{"result":{"type":if value.is_object(){"object"}else{"boolean"},"value":value}}})
                };
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
                if n == 32 {
                    socket.send(Message::Text(serde_json::json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"input-tab","type":"page","url":"https://fixture.test/raced"}}}).to_string().into())).await.unwrap();
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        race_event_seen_rx.take().unwrap(),
                    )
                    .await
                    .expect("client did not observe the injected target change")
                    .expect("race event observer was dropped");
                }
            }
            assert!(
                timeout(Duration::from_millis(100), socket.next())
                    .await
                    .is_err(),
                "mutation callback must be withheld after target revision changes"
            );
            (methods, measured_window_micros)
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let mut registry = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            ..Default::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: vec![],
                },
                "fixture-principal",
            )
            .unwrap();
        registry
            .bind_session_to_browser(&session, connection.instance_id)
            .unwrap();
        let reference = timeout(Duration::from_secs(1), async {
            loop {
                if connection
                    .targets
                    .read()
                    .await
                    .targets
                    .contains_key("input-tab")
                    && connection
                        .frames
                        .read()
                        .await
                        .frames
                        .contains_key("input-frame")
                {
                    registry
                        .register_tab(
                            &session.id,
                            "input-tab",
                            crate::sessions::Ownership::Borrowed,
                        )
                        .unwrap();
                    break connection
                        .capture_target_ref(&registry, &session, "input-tab", "input-frame", 1, 1)
                        .await
                        .unwrap();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let snap = crate::input::GuardSnapshot {
            navigation: 1,
            account: 1,
            document: 1,
            dependencies: ["field:a".into()].into_iter().collect(),
            strict_background: true,
            requires_native: false,
        };
        let mut stale = snap.clone();
        stale.account += 1;
        let locator = crate::input::SemanticLocator::Css("input[name='x']".into());
        let unsafe_action = crate::input::InputAction::Fill("unsafe".into());
        assert!(matches!(
            connection
                .perform_guarded_input(
                    &registry,
                    &reference,
                    "fixture-principal",
                    crate::sessions::IdentityRevisions {
                        account: 1,
                        document: 1
                    },
                    crate::input::GuardedInput {
                        expected: &snap,
                        current: &stale,
                        locator: &locator,
                        action: &unsafe_action,
                        expected_value: "",
                    }
                )
                .await,
            Err(BrowserError::StaleReference(_))
        ));
        let fill = crate::input::InputAction::Fill("héllo 👋".into());
        let outcome = connection
            .perform_guarded_input(
                &registry,
                &reference,
                "fixture-principal",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snap,
                    current: &snap,
                    locator: &locator,
                    action: &fill,
                    expected_value: "",
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(outcome,crate::input::InputOutcome::Applied{observed_value:Some(v),..} if v=="héllo 👋")
        );
        let insert = crate::input::InputAction::Insert("!".into());
        let inserted = connection
            .perform_guarded_input(
                &registry,
                &reference,
                "fixture-principal",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snap,
                    current: &snap,
                    locator: &locator,
                    action: &insert,
                    expected_value: "héllo 👋",
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(inserted,crate::input::InputOutcome::Applied{observed_value:Some(v),..} if v=="héllo 👋!")
        );
        let keys = crate::input::InputAction::SequentialKeys("a".into());
        let typed = connection
            .perform_guarded_input(
                &registry,
                &reference,
                "fixture-principal",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snap,
                    current: &snap,
                    locator: &locator,
                    action: &keys,
                    expected_value: "héllo 👋!",
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(typed,crate::input::InputOutcome::Applied{observed_value:Some(v),..} if v=="héllo 👋!a")
        );
        let click = crate::input::InputAction::Click {
            x: 25.0,
            y: 10.0,
            postcondition: crate::input::ClickPostcondition::Value("héllo 👋!a".into()),
        };
        assert_eq!(
            connection
                .perform_guarded_input(
                    &registry,
                    &reference,
                    "fixture-principal",
                    crate::sessions::IdentityRevisions {
                        account: 1,
                        document: 1
                    },
                    crate::input::GuardedInput {
                        expected: &snap,
                        current: &snap,
                        locator: &locator,
                        action: &click,
                        expected_value: "héllo 👋!a",
                    }
                )
                .await
                .unwrap(),
            crate::input::InputOutcome::NeedsForeground
        );
        let foreground = snap.clone();
        let mut foreground = foreground;
        foreground.strict_background = false;
        let clicked = connection
            .perform_guarded_input(
                &registry,
                &reference,
                "fixture-principal",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &foreground,
                    current: &foreground,
                    locator: &locator,
                    action: &click,
                    expected_value: "héllo 👋!a",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            clicked,
            crate::input::InputOutcome::Applied {
                postcondition_verified: true,
                ..
            }
        ));
        let drag = crate::input::InputAction::Drag {
            from: (25.0, 10.0),
            to: (30.0, 10.0),
            postcondition: crate::input::DragPostcondition {
                left: 20.0,
                top: 5.0,
                tolerance: 1.0,
            },
        };
        let dragged = connection
            .perform_guarded_input(
                &registry,
                &reference,
                "fixture-principal",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &foreground,
                    current: &foreground,
                    locator: &locator,
                    action: &drag,
                    expected_value: "héllo 👋!a",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            dragged,
            crate::input::InputOutcome::Applied {
                postcondition_verified: true,
                ..
            }
        ));
        let raced_ref = connection
            .capture_target_ref(&registry, &session, "input-tab", "input-frame", 1, 1)
            .await
            .unwrap();
        let race_observer = connection.clone();
        let race_event_seen = tokio::spawn(async move {
            loop {
                let event = race_observer
                    .next_event(Duration::from_secs(1))
                    .await
                    .unwrap();
                if event["method"] == "Target.targetInfoChanged"
                    && event["params"]["targetInfo"]["url"] == "https://fixture.test/raced"
                {
                    break;
                }
            }
            timeout(Duration::from_secs(1), async {
                loop {
                    if race_observer.targets.read().await.targets["input-tab"]
                        .url
                        .as_deref()
                        == Some("https://fixture.test/raced")
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("target graph did not apply the injected change");
            let _ = race_event_seen_tx.send(());
        });
        let overwrite = crate::input::InputAction::Fill("overwrite".into());
        let race = connection
            .perform_guarded_input(
                &registry,
                &raced_ref,
                "fixture-principal",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snap,
                    current: &snap,
                    locator: &locator,
                    action: &overwrite,
                    expected_value: "héllo 👋!a",
                },
            )
            .await;
        assert!(matches!(race, Err(BrowserError::StaleReference(_))));
        race_event_seen.await.unwrap();
        timeout(Duration::from_secs(1), async {
            loop {
                if connection.targets.read().await.targets["input-tab"]
                    .url
                    .as_deref()
                    == Some("https://fixture.test/raced")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (methods, measured_window_micros) = server.await.unwrap();
        assert!(measured_window_micros > 0 && measured_window_micros < 5_000_000);
        println!("CDP fixture probe-to-fill dispatch interval: {measured_window_micros} us");
        assert_eq!(
            methods
                .iter()
                .filter(|m| m.as_str() == "Input.insertText")
                .count(),
            1
        );
        assert_eq!(
            methods
                .iter()
                .filter(|m| m.as_str() == "Input.dispatchKeyEvent")
                .count(),
            3
        );
        assert!(
            methods.len() < 33,
            "the raced fill request must be withheld"
        );
    }

    #[tokio::test]
    async fn blocked_browser_target_does_not_block_healthy_target_dispatch() {
        use tokio_tungstenite::tungstenite::Message;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for (target_id, session_id) in [
                ("blocked", "session-blocked"),
                ("healthy", "session-healthy"),
            ] {
                for event in [
                    serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":target_id,"type":"page"}}}),
                    serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":session_id,"targetInfo":{"targetId":target_id,"type":"page"}}}),
                ] {
                    socket
                        .send(Message::Text(event.to_string().into()))
                        .await
                        .unwrap();
                }
            }
            let mut blocked_seen = false;
            let mut healthy_seen = false;
            for _ in 0..2 {
                let message = socket.next().await.unwrap().unwrap();
                let request: Value = serde_json::from_str(&message.to_string()).unwrap();
                match request["params"]["expression"].as_str().unwrap() {
                    "blocked" => blocked_seen = true,
                    "healthy" => {
                        healthy_seen = true;
                        socket
                            .send(Message::Text(
                                serde_json::json!({
                                    "id":request["id"],
                                    "result":{"result":{"type":"string","value":"healthy"}}
                                })
                                .to_string()
                                .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    other => panic!("unexpected dispatch {other}"),
                }
            }
            assert!(blocked_seen && healthy_seen);
        });
        let connection = Arc::new(
            BrowserConnection::connect(&format!("ws://{address}"))
                .await
                .unwrap(),
        );
        let (generation, targets) = timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = connection.target_snapshot().await;
                if snapshot.1.len() == 2 && snapshot.1.iter().all(|target| target.attached) {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let blocked = targets
            .iter()
            .find(|target| target.id == "blocked")
            .unwrap()
            .clone();
        let healthy = targets
            .iter()
            .find(|target| target.id == "healthy")
            .unwrap()
            .clone();
        let blocked_connection = Arc::clone(&connection);
        let blocked_task = tokio::spawn(async move {
            blocked_connection
                .target_command(
                    &blocked.id,
                    generation,
                    &blocked.revision,
                    "Runtime.evaluate",
                    serde_json::json!({"expression":"blocked"}),
                )
                .await
        });
        let healthy_result = timeout(
            Duration::from_secs(1),
            connection.target_command(
                &healthy.id,
                generation,
                &healthy.revision,
                "Runtime.evaluate",
                serde_json::json!({"expression":"healthy"}),
            ),
        )
        .await
        .expect("healthy target was blocked")
        .unwrap();
        assert_eq!(healthy_result["result"]["value"], "healthy");
        blocked_task.abort();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn target_command_rejects_stale_graph_reference_before_dispatch() {
        let connection = BrowserConnection {
            outgoing: tokio::sync::mpsc::channel(1).0,
            instance_id: 1,
            scheduler: scheduler::TargetScheduler::default(),
            targets: Arc::new(RwLock::new(TargetGraph::default())),
            frames: Arc::new(RwLock::new(FrameGraph::default())),
            next_command_id: Arc::new(AtomicU64::new(1)),
            generation: Arc::new(AtomicU64::new(0)),
            cancellation: CancellationToken::new(),
            events: broadcast::channel(8).0,
            event_replay: Arc::new(Mutex::new(VecDeque::new())),
            event_sequence: Arc::new(AtomicU64::new(0)),
        };
        connection.targets.write().await.apply_created(target());
        let result = connection
            .target_command("tab", 0, "wrong-revision", "Runtime.evaluate", Value::Null)
            .await;
        assert!(matches!(result, Err(BrowserError::StaleReference(_))));
    }

    #[tokio::test]
    async fn provider_create_target_receipt_is_the_owned_tab_source() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for event in [
                serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"provider-owned-tab","type":"page","url":"https://fixture.test/new"}}}),
                serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":"provider-owned-cdp","targetInfo":{"targetId":"provider-owned-tab","type":"page"}}}),
            ] {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        event.to_string().into(),
                    ))
                    .await
                    .unwrap();
            }
            let request = socket.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(&request.to_string()).unwrap();
            assert_eq!(request["method"], "Target.createTarget");
            assert_eq!(request["params"]["url"], "https://fixture.test/new");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id": request["id"], "result":{"targetId":"provider-owned-tab"}}).to_string().into(),
                ))
                .await
                .unwrap();
            let close = socket.next().await.unwrap().unwrap();
            let close: Value = serde_json::from_str(&close.to_string()).unwrap();
            assert_eq!(close["method"], "Target.closeTarget");
            assert_eq!(close["params"]["targetId"], "provider-owned-tab");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id": close["id"], "result":{"success":true}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let mut registry = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            dedicated_headless: false,
            shared_extension: false,
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "fixture-principal",
            )
            .unwrap();
        let created = connection
            .create_owned_target(&mut registry, &session, "https://fixture.test/new")
            .await
            .unwrap();
        assert_eq!(created, "provider-owned-tab");
        timeout(Duration::from_secs(1), async {
            loop {
                if connection.targets.read().await.targets["provider-owned-tab"].attached {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let receipt = connection
            .release_owned_session(
                &mut registry,
                &session.id,
                Some(&FixtureIndependentObserver),
            )
            .await;
        assert_eq!(receipt.closed, vec!["provider-owned-tab"]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn failed_target_creation_rolls_back_new_binding_for_fresh_connection_retry() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (first_stream, _) = listener.accept().await.unwrap();
            let mut first = tokio_tungstenite::accept_async(first_stream).await.unwrap();
            let failed = first.next().await.unwrap().unwrap();
            let failed: Value = serde_json::from_str(&failed.to_string()).unwrap();
            assert_eq!(failed["method"], "Target.createTarget");
            first
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id":failed["id"], "error":{"code":-32000,"message":"fixture create failure"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();

            let (retry_stream, _) = listener.accept().await.unwrap();
            let mut retry = tokio_tungstenite::accept_async(retry_stream).await.unwrap();
            let request = retry.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(&request.to_string()).unwrap();
            assert_eq!(request["method"], "Target.createTarget");
            retry
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id":request["id"], "result":{"targetId":"retry-owned-target"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        });

        let mut registry = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            ..crate::sessions::ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "fixture-principal",
            )
            .unwrap();

        let first = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let first_instance = first.instance_id;
        assert!(
            first
                .create_owned_target(&mut registry, &session, "about:blank")
                .await
                .is_err()
        );
        assert!(!registry.session_bound_to_browser(&session.id, first_instance));
        drop(first);

        let retry = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        assert_eq!(
            retry
                .create_owned_target(&mut registry, &session, "about:blank")
                .await
                .unwrap(),
            "retry-owned-target"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn canceling_pending_target_creation_does_not_leave_browser_binding() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request = socket.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(&request.to_string()).unwrap();
            assert_eq!(request["method"], "Target.createTarget");
            let _ = request_seen_tx.send(());
            let _ = socket.next().await;
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let mut registry = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            ..crate::sessions::ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "fixture-principal",
            )
            .unwrap();
        let instance_id = connection.instance_id;
        {
            let creation = connection.create_owned_target(&mut registry, &session, "about:blank");
            tokio::pin!(creation);
            tokio::select! {
                _ = &mut creation => panic!("unanswered CDP create unexpectedly completed"),
                _ = request_seen_rx => {},
            }
        }
        assert!(!registry.session_bound_to_browser(&session.id, instance_id));
        drop(connection);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn release_preserves_owned_target_without_independent_observer() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request = socket.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(&request.to_string()).unwrap();
            assert_eq!(request["method"], "Target.createTarget");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id": request["id"], "result":{"targetId":"unobserved-tab"}}).to_string().into(),
                ))
                .await
                .unwrap();
            assert!(
                timeout(Duration::from_millis(100), socket.next())
                    .await
                    .is_err()
            );
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let mut registry = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            dedicated_headless: false,
            shared_extension: false,
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "fixture-principal",
            )
            .unwrap();
        connection
            .create_owned_target(&mut registry, &session, "about:blank")
            .await
            .unwrap();
        let receipt = connection
            .release_owned_session(&mut registry, &session.id, None)
            .await;
        assert_eq!(receipt.remaining.len(), 1);
        assert!(
            receipt.remaining[0]
                .reason
                .contains("no independent observer")
        );
        assert!(receipt.closed.is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_proof_from_stale_target_snapshot_never_closes_target() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for event in [
                serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned-target","type":"page","url":"about:blank"}}}),
                serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":"owned-cdp","targetInfo":{"targetId":"owned-target","type":"page"}}}),
            ] {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        event.to_string().into(),
                    ))
                    .await
                    .unwrap();
            }
            let create = socket.next().await.unwrap().unwrap();
            let create: Value = serde_json::from_str(&create.to_string()).unwrap();
            assert_eq!(create["method"], "Target.createTarget");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id":create["id"],"result":{"targetId":"owned-target"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            assert!(
                timeout(Duration::from_millis(150), socket.next())
                    .await
                    .is_err()
            );
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let mut registry = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            ..crate::sessions::ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "alice",
            )
            .unwrap();
        connection
            .create_owned_target(&mut registry, &session, "about:blank")
            .await
            .unwrap();
        timeout(Duration::from_secs(1), async {
            loop {
                if connection.targets.read().await.targets["owned-target"].attached {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let revision = connection.targets.read().await.targets["owned-target"]
            .revision
            .clone();
        connection.targets.write().await.apply_changed(
            "owned-target",
            Some("https://fixture.test/navigated".to_owned()),
            None,
        );
        let receipt = connection
            .release_owned_session(
                &mut registry,
                &session.id,
                Some(&FixtureRevisionObserver { revision }),
            )
            .await;
        assert!(receipt.closed.is_empty());
        assert_eq!(receipt.remaining[0].target_id, "owned-target");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn frame_command_routes_oopif_context_over_flattened_session() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for event in [
                serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"oopif-target","type":"iframe","url":"https://cross-origin.test/"}}}),
                serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":"session-oopif","targetInfo":{"targetId":"oopif-target","type":"iframe"}}}),
                serde_json::json!({"sessionId":"session-oopif","method":"Page.frameAttached","params":{"frameId":"frame-oopif","parentFrameId":"root"}}),
                serde_json::json!({"sessionId":"session-oopif","method":"Runtime.executionContextCreated","params":{"context":{"id":99,"auxData":{"frameId":"frame-oopif"}}}}),
            ] {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        event.to_string().into(),
                    ))
                    .await
                    .unwrap();
            }
            let request = socket.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(&request.to_string()).unwrap();
            assert_eq!(request["sessionId"], "session-oopif");
            assert_eq!(request["method"], "Runtime.evaluate");
            assert_eq!(request["params"]["contextId"], 99);
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id": request["id"], "sessionId":"session-oopif", "result":{"result":{"type":"string","value":"https://cross-origin.test/"}}}).to_string().into(),
                ))
                .await
                .unwrap();
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let revision = timeout(Duration::from_secs(1), async {
            loop {
                let frames = connection.frames.read().await;
                if let Some(frame) = frames.frames.get("frame-oopif")
                    && frame.target_id == "oopif-target"
                    && frame.execution_context_ids.contains(&99)
                {
                    break frame.revision;
                }
                drop(frames);
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("OOPIF frame events were not bound to the flattened target");
        assert_eq!(
            revision,
            connection.frames.read().await.frames["frame-oopif"].revision
        );
        let mut sessions = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            dedicated_headless: false,
            shared_extension: false,
        });
        let session = sessions
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "fixture-principal",
            )
            .unwrap();
        sessions
            .bind_session_to_browser(&session, connection.instance_id)
            .unwrap();
        sessions
            .register_tab(
                &session.id,
                "oopif-target",
                crate::sessions::Ownership::Borrowed,
            )
            .unwrap();
        let reference = connection
            .capture_target_ref(&sessions, &session, "oopif-target", "frame-oopif", 1, 1)
            .await
            .unwrap();
        let result = connection
            .target_ref_command(
                &sessions,
                &reference,
                "fixture-principal",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                "Runtime.evaluate",
                serde_json::json!({"expression":"location.href"}),
            )
            .await
            .unwrap();
        assert_eq!(result["result"]["value"], "https://cross-origin.test/");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn direct_cdp_dispatch_rejects_shared_extension_session() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for event in [
                serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"shared-tab","type":"page","url":"https://fixture.test/"}}}),
                serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":"shared-cdp","targetInfo":{"targetId":"shared-tab","type":"page"}}}),
                serde_json::json!({"sessionId":"shared-cdp","method":"Page.frameNavigated","params":{"frame":{"id":"shared-frame","loaderId":"load-1","url":"https://fixture.test/"}}}),
            ] {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        event.to_string().into(),
                    ))
                    .await
                    .unwrap();
            }
            assert!(
                timeout(Duration::from_millis(150), socket.next())
                    .await
                    .is_err()
            );
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let mut registry = crate::sessions::SessionRegistry::new(crate::sessions::ProviderGrants {
            shared_extension: true,
            ..crate::sessions::ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Shared,
                    selected_target_ids: vec!["shared-tab".to_owned()],
                },
                "alice",
            )
            .unwrap();
        registry
            .bind_session_to_browser(&session, connection.instance_id)
            .unwrap();
        let reference = timeout(Duration::from_secs(1), async {
            loop {
                if connection
                    .frames
                    .read()
                    .await
                    .frames
                    .contains_key("shared-frame")
                {
                    break connection
                        .capture_target_ref(&registry, &session, "shared-tab", "shared-frame", 1, 1)
                        .await
                        .unwrap();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let result = connection
            .target_ref_command(
                &registry,
                &reference,
                "alice",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                "Runtime.evaluate",
                serde_json::json!({"expression":"1+1"}),
            )
            .await;
        assert!(
            matches!(result, Err(BrowserError::StaleReference(reason)) if reason.contains("ProviderMismatch"))
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn manager_rejects_empty_endpoint_without_creating_state() {
        let manager = BrowserManager::new();
        let result = manager.connect("").await;
        assert!(matches!(
            result,
            Err(BrowserError::Connection(message)) if message == "empty debugger endpoint"
        ));
        assert_eq!(manager.len().await, 0);
    }

    #[tokio::test]
    async fn multiplexes_out_of_order_responses_on_one_socket() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut requests = Vec::new();
            while requests.len() < 2 {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    socket.next().await
                {
                    requests.push(serde_json::from_str::<Value>(&text).unwrap());
                }
            }
            socket.send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"event-tab","type":"page","url":"https://event.test","title":"Event tab"}}}).to_string().into()
            )).await.unwrap();
            for request in requests.into_iter().rev() {
                socket.send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"id": request["id"], "result": {"method": request["method"]}}).to_string().into()
                )).await.unwrap();
            }
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let first = connection.command(Some("session-a".to_owned()), "Runtime.enable", Value::Null);
        let second = connection.command(Some("session-b".to_owned()), "Page.enable", Value::Null);
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.unwrap()["method"], "Runtime.enable");
        assert_eq!(second.unwrap()["method"], "Page.enable");
        let targets = connection.targets.read().await;
        assert_eq!(
            targets.targets["event-tab"].title.as_deref(),
            Some("Event tab")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_enables_discovery_and_flattened_auto_attach() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut methods = Vec::new();
            while methods.len() < 2 {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    socket.next().await
                {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    methods.push(request["method"].as_str().unwrap().to_owned());
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::json!({"id":request["id"],"result":{}})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                }
            }
            methods
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        connection.bootstrap().await.unwrap();
        assert_eq!(
            server.await.unwrap(),
            vec!["Target.setDiscoverTargets", "Target.setAutoAttach"]
        );
    }

    #[tokio::test]
    async fn target_bootstrap_enables_required_domains_on_flattened_session() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut methods = Vec::new();
            while methods.len() < 5 {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    socket.next().await
                {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(request["sessionId"], "session-1");
                    methods.push(request["method"].as_str().unwrap().to_owned());
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::json!({"id":request["id"],"result":{}})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                }
            }
            methods
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        connection.targets.write().await.apply_created(target());
        connection
            .targets
            .write()
            .await
            .apply_event(&serde_json::json!({
                "method":"Target.attachedToTarget",
                "params":{"sessionId":"session-1","targetInfo":{"targetId":"tab","type":"page"}}
            }));
        connection.bootstrap_target("session-1").await.unwrap();
        assert_eq!(
            server.await.unwrap(),
            vec![
                "Page.enable",
                "Runtime.enable",
                "DOM.enable",
                "Network.enable",
                "Accessibility.enable"
            ]
        );
    }

    #[test]
    fn applies_target_and_frame_lifecycle_events() {
        let mut targets = TargetGraph::default();
        targets.apply_event(&serde_json::json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"tab-1","type":"page","url":"https://example.test","title":"Example"}}}));
        assert_eq!(targets.targets["tab-1"].title.as_deref(), Some("Example"));
        targets.apply_event(&serde_json::json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"tab-1","url":"https://example.test/next","title":"Next"}}}));
        assert_eq!(
            targets.targets["tab-1"].url.as_deref(),
            Some("https://example.test/next")
        );
        targets.apply_event(&serde_json::json!({"method":"Target.attachedToTarget","params":{"sessionId":"session-1","targetInfo":{"targetId":"tab-1"}}}));
        assert_eq!(
            targets.targets["tab-1"].session_id.as_deref(),
            Some("session-1")
        );
        assert!(targets.targets["tab-1"].attached);
        targets.apply_event(&serde_json::json!({"method":"Target.detachedFromTarget","params":{"targetId":"tab-1","sessionId":"session-1"}}));
        assert!(!targets.targets["tab-1"].attached);
        assert!(targets.targets["tab-1"].session_id.is_none());

        let mut frames = FrameGraph::default();
        frames.apply_event(&serde_json::json!({"method":"Page.frameAttached","params":{"frameId":"frame-1","parentFrameId":"root","targetId":"tab-1"}}));
        frames.apply_event(&serde_json::json!({"method":"Runtime.executionContextCreated","params":{"context":{"id":7,"auxData":{"frameId":"frame-1"}}}}));
        assert_eq!(frames.frames["frame-1"].execution_context_ids, vec![7]);
        frames.apply_event(
            &serde_json::json!({"method":"Page.frameDetached","params":{"frameId":"frame-1"}}),
        );
        assert!(frames.frames.is_empty());
    }
}
