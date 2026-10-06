//! Browser providers for isolated dedicated profiles.
//!
//! The launch path uses an explicit executable and a fresh profile directory;
//! it does not inspect or attach to the user's default Chrome profile.

use crate::sessions::{ProviderKind, SessionHandle, SessionMode, SessionRegistry};
use crate::{BrowserConnection, BrowserError, BrowserManager};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, oneshot};
use tokio::time::sleep;
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};

static NEXT_PROFILE_SUFFIX: AtomicU64 = AtomicU64::new(1);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
type PendingReplies = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider session rejected: {0}")]
    Session(String),
    #[error("Chrome executable is not an existing file: {0}")]
    InvalidExecutable(PathBuf),
    #[error("provider I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Chrome did not publish a usable DevToolsActivePort endpoint")]
    StartupTimeout,
    #[error("Chrome exited before publishing DevToolsActivePort (status: {0})")]
    StartupExit(std::process::ExitStatus),
    #[error("Chrome did not exit after Browser.close; process and profile are retained")]
    ShutdownTimeout,
    #[error(transparent)]
    Browser(#[from] BrowserError),
    #[error("shared extension attachment failed: {0}")]
    Extension(String),
}

#[derive(Clone)]
pub struct DedicatedChromeProvider {
    manager: BrowserManager,
    chrome_executable: PathBuf,
}

impl DedicatedChromeProvider {
    pub fn new(chrome_executable: impl Into<PathBuf>) -> Self {
        Self {
            manager: BrowserManager::new(),
            chrome_executable: chrome_executable.into(),
        }
    }

    pub async fn launch(
        &self,
        registry: &mut SessionRegistry,
        session: &SessionHandle,
        initial_url: &str,
    ) -> Result<DedicatedBrowserSession, ProviderError> {
        registry
            .authorize_target_creation(session)
            .map_err(|error| ProviderError::Session(format!("{error:?}")))?;
        if !matches!(
            session.provider,
            ProviderKind::DedicatedHeaded | ProviderKind::DedicatedHeadless
        ) {
            return Err(ProviderError::Session(
                "session is not a dedicated Chrome provider".to_owned(),
            ));
        }
        if !self.chrome_executable.is_file() {
            return Err(ProviderError::InvalidExecutable(
                self.chrome_executable.clone(),
            ));
        }
        let mut process = ChromeProcess::launch(&self.chrome_executable, session.mode)?;
        let endpoint = match process.wait_for_endpoint().await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                process.stop();
                return Err(error);
            }
        };
        let connection = match self.manager.connect(&endpoint).await {
            Ok(connection) => connection,
            Err(error) => {
                process.stop();
                return Err(error.into());
            }
        };
        let target_id = match connection
            .create_owned_target(registry, session, initial_url)
            .await
        {
            Ok(target_id) => target_id,
            Err(error) => {
                process.stop();
                return Err(error.into());
            }
        };
        let attached_session = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (_, targets) = connection.target_snapshot().await;
                if let Some(target) = targets.iter().find(|target| target.id == target_id)
                    && target.attached
                    && let Some(session_id) = target.session_id.clone()
                {
                    break session_id;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        let attached_session = match attached_session {
            Ok(session_id) => session_id,
            Err(_) => {
                let _ = connection
                    .command(None, "Target.closeTarget", json!({"targetId": target_id}))
                    .await;
                let _ = registry.release_session(&session.id, |_| Ok(()));
                process.stop();
                return Err(ProviderError::Browser(BrowserError::Timeout));
            }
        };
        if let Err(error) = connection.bootstrap_target(attached_session).await {
            let _ = connection
                .command(None, "Target.closeTarget", json!({"targetId": target_id}))
                .await;
            let _ = registry.release_session(&session.id, |_| Ok(()));
            process.stop();
            return Err(error.into());
        }
        Ok(DedicatedBrowserSession {
            process: process.preserve_after_launch(),
            connection,
            endpoint,
            target_id,
            session_id: session.id.clone(),
            mode: session.mode,
            manager: self.manager.clone(),
            shutdown_phase: ShutdownPhase::ReleaseTargets,
            last_receipt: None,
        })
    }
}

pub struct DedicatedBrowserSession {
    process: ChromeProcess,
    connection: Arc<BrowserConnection>,
    endpoint: String,
    target_id: String,
    session_id: String,
    mode: SessionMode,
    manager: BrowserManager,
    shutdown_phase: ShutdownPhase,
    last_receipt: Option<crate::sessions::CleanupReceipt>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShutdownPhase {
    ReleaseTargets,
    BlockedByPreservedResources,
    ProcessCleanup,
}

/// Receipt from a shutdown attempt. If cleanup cannot be safely completed,
/// `recovery` retains the live browser process and private profile for retry.
pub struct ShutdownOutcome {
    pub receipt: crate::sessions::CleanupReceipt,
    pub recovery: Option<DedicatedBrowserSession>,
    pub cleanup_error: Option<String>,
}

impl DedicatedBrowserSession {
    pub fn connection(&self) -> &Arc<BrowserConnection> {
        &self.connection
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    pub fn profile_directory(&self) -> &Path {
        &self.process.profile_directory
    }

    pub async fn shutdown(
        mut self,
        registry: &mut SessionRegistry,
        observer: Option<&dyn crate::sessions::IndependentTargetObserver>,
    ) -> ShutdownOutcome {
        let receipt = match self.shutdown_phase {
            ShutdownPhase::ReleaseTargets => {
                let receipt = self
                    .connection
                    .release_owned_session(registry, &self.session_id, observer)
                    .await;
                self.last_receipt = Some(receipt.clone());
                if !receipt.remaining.is_empty() || !receipt.preserved.is_empty() {
                    if receipt.remaining.is_empty() {
                        self.shutdown_phase = ShutdownPhase::BlockedByPreservedResources;
                    }
                    return ShutdownOutcome {
                        receipt,
                        recovery: Some(self),
                        cleanup_error: Some(
                            "session resources remain; browser process preserved".into(),
                        ),
                    };
                }
                self.shutdown_phase = ShutdownPhase::ProcessCleanup;
                receipt
            }
            ShutdownPhase::BlockedByPreservedResources => {
                return ShutdownOutcome {
                    receipt: self.last_receipt.clone().unwrap_or_default(),
                    recovery: Some(self),
                    cleanup_error: Some(
                        "preserved session resources prevent browser shutdown".into(),
                    ),
                };
            }
            ShutdownPhase::ProcessCleanup => self.last_receipt.clone().unwrap_or_default(),
        };
        if self.mode == SessionMode::Headed {
            return ShutdownOutcome {
                receipt,
                recovery: Some(self),
                cleanup_error: Some(
                    "headed Chrome is preserved; whole-window shutdown is disabled".into(),
                ),
            };
        }
        let target_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let targets = match self
                .connection
                .command(None, "Target.getTargets", Value::Null)
                .await
            {
                Ok(targets) => targets,
                Err(error) => {
                    return ShutdownOutcome {
                        receipt,
                        recovery: Some(self),
                        cleanup_error: Some(format!(
                            "cannot verify remaining page targets: {error}"
                        )),
                    };
                }
            };
            let Some(target_infos) = targets.get("targetInfos").and_then(Value::as_array) else {
                return ShutdownOutcome {
                    receipt,
                    recovery: Some(self),
                    cleanup_error: Some("Target.getTargets returned no targetInfos array".into()),
                };
            };
            let live_pages = target_infos
                .iter()
                .filter(|info| info.get("type").and_then(Value::as_str) == Some("page"))
                .map(|info| {
                    json!({
                        "targetId": info.get("targetId"),
                        "url": info.get("url"),
                    })
                })
                .collect::<Vec<_>>();
            if live_pages.is_empty() {
                break;
            }
            if tokio::time::Instant::now() >= target_deadline {
                return ShutdownOutcome {
                    receipt,
                    recovery: Some(self),
                    cleanup_error: Some(format!(
                        "live page targets remain; Chrome process and profile preserved: {live_pages:?}"
                    )),
                };
            }
            sleep(Duration::from_millis(50)).await;
        }
        if let Err(error) = self.process.graceful_stop(&self.connection).await {
            return ShutdownOutcome {
                receipt,
                recovery: Some(self),
                cleanup_error: Some(error.to_string()),
            };
        }
        self.manager.evict(&self.endpoint).await;
        ShutdownOutcome {
            receipt,
            recovery: None,
            cleanup_error: None,
        }
    }
}

struct ChromeProcess {
    child: Option<Child>,
    profile_directory: PathBuf,
    preserve_on_drop: bool,
    close_requested: bool,
}

impl ChromeProcess {
    fn launch(executable: &Path, mode: SessionMode) -> Result<Self, ProviderError> {
        if !executable.is_file() {
            return Err(ProviderError::InvalidExecutable(executable.to_owned()));
        }
        let profile_directory = unique_profile_directory()?;
        if let Err(error) = restrict_profile_permissions(&profile_directory) {
            let _ = std::fs::remove_dir_all(&profile_directory);
            return Err(ProviderError::Io(error));
        }
        let mut command = chrome_command(executable, &profile_directory, mode);
        match command.spawn() {
            Ok(child) => Ok(Self {
                child: Some(child),
                profile_directory,
                preserve_on_drop: false,
                close_requested: false,
            }),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&profile_directory);
                Err(ProviderError::Io(error))
            }
        }
    }

    async fn wait_for_endpoint(&mut self) -> Result<String, ProviderError> {
        let active_port = self.profile_directory.join("DevToolsActivePort");
        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            let Some(child) = self.child.as_mut() else {
                return Err(ProviderError::StartupTimeout);
            };
            if let Some(status) = child.try_wait()? {
                return Err(ProviderError::StartupExit(status));
            }
            if let Ok(contents) = std::fs::read_to_string(&active_port)
                && let Some(endpoint) = parse_devtools_active_port(&contents)
            {
                return Ok(endpoint);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ProviderError::StartupTimeout);
            }
            sleep(Duration::from_millis(40)).await;
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.profile_directory);
    }

    async fn graceful_stop(&mut self, connection: &BrowserConnection) -> Result<(), ProviderError> {
        if self.child.is_some() && !self.close_requested {
            connection
                .command(None, "Browser.close", Value::Null)
                .await?;
            self.close_requested = true;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while let Some(child) = self.child.as_mut() {
            if child.try_wait()?.is_some() {
                self.child.take();
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ProviderError::ShutdownTimeout);
            }
            sleep(Duration::from_millis(50)).await;
        }
        match std::fs::remove_dir_all(&self.profile_directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ProviderError::Io(error)),
        }
    }

    fn preserve_after_launch(mut self) -> Self {
        self.preserve_on_drop = true;
        self
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        // A canceled or failed launch still owns its private process/profile.
        // After a session is returned, unresolved browser state must survive
        // dropping the recovery handle.
        if !self.preserve_on_drop {
            self.stop();
        }
    }
}

fn unique_profile_directory() -> Result<PathBuf, std::io::Error> {
    loop {
        let suffix = NEXT_PROFILE_SUFFIX.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chrome-controlla-profile-{}-{suffix}",
            std::process::id()
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn chrome_command(executable: &Path, profile_directory: &Path, mode: SessionMode) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--remote-debugging-address=127.0.0.1")
        .arg("--remote-debugging-port=0")
        .arg(format!(
            "--user-data-dir={}",
            profile_directory.to_string_lossy()
        ))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--no-startup-window")
        .arg("--disable-sync")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if mode == SessionMode::Headless {
        command.arg("--headless=new");
    }
    command
}

fn restrict_profile_permissions(_path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn parse_devtools_active_port(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    let port: u16 = lines.next()?.parse().ok()?;
    if port == 0 {
        return None;
    }
    let path = lines.next()?;
    if !path.starts_with("/devtools/browser/") || path.contains(['\r', '\n']) {
        return None;
    }
    Some(format!("ws://127.0.0.1:{port}{path}"))
}

/// Authenticated loopback pairing endpoint for the explicitly selected-tab
/// Chrome extension. The token is single-session and never exposed on a
/// non-loopback interface.
pub struct SharedExtensionProvider {
    listener: TcpListener,
    pairing: ExtensionPairing,
    session_id: String,
    principal: String,
    capability_revision: u64,
    browser_instance_id: u128,
    expected_targets: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct ExtensionPairing {
    pub endpoint: String,
    pub token: String,
}

impl SharedExtensionProvider {
    pub async fn bind(
        registry: &mut SessionRegistry,
        session: &SessionHandle,
    ) -> Result<Self, ProviderError> {
        if session.provider != ProviderKind::SharedExtension || session.mode != SessionMode::Shared
        {
            return Err(ProviderError::Session(
                "session is not shared-extension mode".into(),
            ));
        }
        registry
            .authorize_shared_extension(session)
            .map_err(|error| ProviderError::Session(format!("{error:?}")))?;
        let expected_targets = registry
            .selected_targets(&session.id)
            .ok_or_else(|| ProviderError::Session("unknown shared session".into()))?;
        if expected_targets
            .iter()
            .any(|target| target.parse::<u32>().is_err())
        {
            return Err(ProviderError::Session(
                "shared target IDs must be Chrome tabs.Tab.id values".into(),
            ));
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let token = random_token()?;
        let browser_instance_id = random_instance_id()?;
        let endpoint = format!("ws://{}/", listener.local_addr()?);
        Ok(Self {
            listener,
            pairing: ExtensionPairing { endpoint, token },
            session_id: session.id.clone(),
            principal: session.principal.clone(),
            capability_revision: session.capability_revision,
            browser_instance_id,
            expected_targets,
        })
    }

    pub fn pairing(&self) -> &ExtensionPairing {
        &self.pairing
    }

    pub async fn accept(
        &mut self,
        registry: &mut SessionRegistry,
    ) -> Result<SharedExtensionSession, ProviderError> {
        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let (stream, peer) = tokio::time::timeout(remaining, self.listener.accept())
                .await
                .map_err(|_| ProviderError::Extension("pairing timed out".into()))??;
            if !peer.ip().is_loopback() {
                return Err(ProviderError::Extension(
                    "non-loopback client rejected".into(),
                ));
            }
            let mut socket = accept_async(stream)
                .await
                .map_err(|error| ProviderError::Extension(error.to_string()))?;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let message = tokio::time::timeout(remaining, socket.next())
                .await
                .map_err(|_| ProviderError::Extension("extension hello timed out".into()))?
                .ok_or_else(|| {
                    ProviderError::Extension("extension disconnected before hello".into())
                })?
                .map_err(|error| ProviderError::Extension(error.to_string()))?;
            let hello = message
                .to_text()
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(text).ok());
            let Some(hello) = hello else {
                let _ = socket.send(Message::Close(None)).await;
                continue;
            };
            let Some(target_values) = hello["targets"].as_array() else {
                let _ = socket.send(Message::Close(None)).await;
                continue;
            };
            let Some(targets) = target_values
                .iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<BTreeSet<_>>>()
            else {
                let _ = socket.send(Message::Close(None)).await;
                continue;
            };
            let valid_token = hello["token"]
                .as_str()
                .is_some_and(|token| constant_time_eq(token, &self.pairing.token));
            if hello["type"] != "hello" || !valid_token || targets != self.expected_targets {
                let _ = socket.send(Message::Close(None)).await;
                continue;
            }
            if targets.iter().any(String::is_empty) {
                let _ = socket.send(Message::Close(None)).await;
                continue;
            }
            if let Err(error) = socket
                .send(Message::Text(json!({"type":"ready"}).to_string().into()))
                .await
            {
                return Err(ProviderError::Extension(error.to_string()));
            }
            if let Err(error) = registry
                .bind_session_to_browser(
                    &SessionHandle {
                        id: self.session_id.clone(),
                        principal: self.principal.clone(),
                        provider: ProviderKind::SharedExtension,
                        mode: SessionMode::Shared,
                        capability_revision: self.capability_revision,
                    },
                    self.browser_instance_id,
                )
                .map_err(|error| ProviderError::Session(format!("{error:?}")))
            {
                let _ = socket.send(Message::Close(None)).await;
                return Err(error);
            }
            let (sink, mut stream) = socket.split();
            let sink = Arc::new(Mutex::new(sink));
            let pending: PendingReplies = Arc::new(Mutex::new(HashMap::new()));
            let pending_reader = Arc::clone(&pending);
            let reader_task = tokio::spawn(async move {
                while let Some(Ok(message)) = stream.next().await {
                    let Ok(text) = message.to_text() else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_str::<Value>(text) else {
                        continue;
                    };
                    let Some(id) = value["id"].as_u64() else {
                        continue;
                    };
                    if let Some(sender) = pending_reader.lock().await.remove(&id) {
                        let _ = sender.send(Ok(value));
                    }
                }
                let remaining = std::mem::take(&mut *pending_reader.lock().await);
                for (_, sender) in remaining {
                    let _ = sender.send(Err("extension disconnected".to_owned()));
                }
            });
            return Ok(SharedExtensionSession {
                sink,
                pending,
                session_id: self.session_id.clone(),
                principal: self.principal.clone(),
                capability_revision: self.capability_revision,
                browser_instance_id: self.browser_instance_id,
                selected_targets: targets,
                next_command_id: AtomicU64::new(1),
                scheduler: crate::scheduler::TargetScheduler::default(),
                reader_task: Some(reader_task),
                released: std::sync::atomic::AtomicBool::new(false),
            });
        }
    }
}

pub struct SharedExtensionSession {
    sink: Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>,
    pending: PendingReplies,
    session_id: String,
    principal: String,
    capability_revision: u64,
    browser_instance_id: u128,
    selected_targets: BTreeSet<String>,
    next_command_id: AtomicU64,
    scheduler: crate::scheduler::TargetScheduler,
    reader_task: Option<tokio::task::JoinHandle<()>>,
    released: std::sync::atomic::AtomicBool,
}

impl SharedExtensionSession {
    pub async fn release(&mut self) -> Result<(), ProviderError> {
        if self.released.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let result = self
            .sink
            .lock()
            .await
            .send(Message::Close(None))
            .await
            .map_err(|error| ProviderError::Extension(error.to_string()));
        let remaining = std::mem::take(&mut *self.pending.lock().await);
        for (_, sender) in remaining {
            let _ = sender.send(Err("shared extension session released".into()));
        }
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
        result
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn browser_instance_id(&self) -> u128 {
        self.browser_instance_id
    }

    pub fn selected_targets(&self) -> &BTreeSet<String> {
        &self.selected_targets
    }

    pub async fn command(
        &self,
        registry: &SessionRegistry,
        session: &SessionHandle,
        target_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, ProviderError> {
        self.authorize_command(registry, session, target_id, method)?;
        let target_id = target_id.to_owned();
        let method = method.to_owned();
        self.scheduler
            .run_target(&target_id, self.command_wire(&target_id, &method, params))
            .await
    }

    /// Run a shared-document mutation under both the tab actor and document
    /// lock. `document_id` is supplied by the caller's identity observer.
    pub async fn document_mutation(
        &self,
        registry: &SessionRegistry,
        session: &SessionHandle,
        target_id: &str,
        document_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, ProviderError> {
        self.authorize_command(registry, session, target_id, method)?;
        let target_id = target_id.to_owned();
        let document_id = document_id.to_owned();
        let method = method.to_owned();
        self.scheduler
            .run_document_mutation(
                &target_id,
                &document_id,
                self.command_wire(&target_id, &method, params),
            )
            .await
    }

    fn authorize_command(
        &self,
        registry: &SessionRegistry,
        session: &SessionHandle,
        target_id: &str,
        method: &str,
    ) -> Result<(), ProviderError> {
        if session.id != self.session_id
            || session.principal != self.principal
            || session.capability_revision != self.capability_revision
        {
            return Err(ProviderError::Extension("session identity changed".into()));
        }
        registry
            .authorize_shared_target(session, self.browser_instance_id, target_id)
            .map_err(|error| ProviderError::Extension(format!("{error:?}")))?;
        if !self.selected_targets.contains(target_id) {
            return Err(ProviderError::Extension(
                "target is not explicitly attached".into(),
            ));
        }
        if method.is_empty() || method.starts_with("Browser.") || method.starts_with("Target.") {
            return Err(ProviderError::Extension(
                "command is outside the shared target scope".into(),
            ));
        }
        Ok(())
    }

    async fn command_wire(
        &self,
        target_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, ProviderError> {
        let id = self.next_command_id.fetch_add(1, Ordering::Relaxed);
        let (sender, mut receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        let message = Message::Text(
            json!({"type":"command", "id":id, "target_id":target_id, "method":method, "params":params})
                .to_string()
                .into(),
        );
        if let Err(error) = self.sink.lock().await.send(message).await {
            self.pending.lock().await.remove(&id);
            return Err(ProviderError::Extension(error.to_string()));
        }
        let result = match tokio::time::timeout(STARTUP_TIMEOUT, &mut receiver).await {
            Ok(Ok(Ok(value))) => value,
            Ok(Ok(Err(error))) => return Err(ProviderError::Extension(error)),
            Ok(Err(_)) => {
                return Err(ProviderError::Extension(
                    "extension response channel closed".into(),
                ));
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(ProviderError::Extension(
                    "extension command timed out".into(),
                ));
            }
        };
        if let Some(error) = result.get("error") {
            return Err(ProviderError::Extension(error.to_string()));
        }
        Ok(result.get("result").cloned().unwrap_or(Value::Null))
    }
}

impl Drop for SharedExtensionSession {
    fn drop(&mut self) {
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
        // Dropping the sink closes the underlying connection; the extension's
        // socket close handler detaches only this pairing generation.
    }
}

fn random_token() -> Result<String, ProviderError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| ProviderError::Extension(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn random_instance_id() -> Result<u128, ProviderError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| ProviderError::Extension(error.to_string()))?;
    Ok(u128::from_be_bytes(bytes))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        diff |= usize::from(
            left.as_bytes().get(index).copied().unwrap_or(0)
                ^ right.as_bytes().get(index).copied().unwrap_or(0),
        );
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devtools_active_port_requires_loopback_browser_websocket_path() {
        assert_eq!(
            parse_devtools_active_port("9222\n/devtools/browser/abc\n"),
            Some("ws://127.0.0.1:9222/devtools/browser/abc".to_owned())
        );
        assert_eq!(parse_devtools_active_port("0\n/devtools/browser/abc"), None);
        assert_eq!(parse_devtools_active_port("9222\n/json/list"), None);
        assert_eq!(parse_devtools_active_port("9222\n"), None);
    }

    #[test]
    fn profile_directories_are_unique_and_separate() {
        let first = unique_profile_directory().unwrap();
        let second = unique_profile_directory().unwrap();
        assert_ne!(first, second);
        assert!(first.is_dir() && second.is_dir());
        std::fs::remove_dir_all(first).unwrap();
        std::fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn launch_arguments_use_distinct_profile_and_headless_mode() {
        let profile = PathBuf::from("/tmp/chrome profile/with spaces");
        let headed = chrome_command(Path::new("/opt/chrome"), &profile, SessionMode::Headed);
        let headed = headed
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            headed
                .iter()
                .any(|arg| arg == &format!("--user-data-dir={}", profile.to_string_lossy()))
        );
        assert!(!headed.iter().any(|arg| arg == "--headless=new"));
        assert!(!headed.iter().any(|arg| arg == "about:blank"));
        assert!(headed.iter().any(|arg| arg == "--no-startup-window"));
        let headless = chrome_command(Path::new("/opt/chrome"), &profile, SessionMode::Headless);
        assert!(headless.get_args().any(|arg| arg == "--headless=new"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dedicated_process_uses_private_profile_and_reads_its_endpoint() {
        use std::os::unix::fs::PermissionsExt;
        let root = unique_profile_directory().unwrap();
        let executable = root.join("fake chrome");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprofile=''\nfor arg in \"$@\"; do\n  case \"$arg\" in --user-data-dir=*) profile=\"${arg#*=}\";; esac\ndone\nprintf '9222\\n/devtools/browser/fixture\\n' > \"$profile/DevToolsActivePort\"\nexec /bin/sleep 60\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut process = ChromeProcess::launch(&executable, SessionMode::Headless).unwrap();
        assert_eq!(
            process.wait_for_endpoint().await.unwrap(),
            "ws://127.0.0.1:9222/devtools/browser/fixture"
        );
        assert_eq!(
            std::fs::metadata(&process.profile_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let profile = process.profile_directory.clone();
        process.stop();
        process.stop();
        assert!(!profile.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dedicated_startup_failure_kills_child_and_removes_private_profile() {
        use std::os::unix::fs::PermissionsExt;
        let root = unique_profile_directory().unwrap();
        let executable = root.join("failing chrome");
        std::fs::write(&executable, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut process = ChromeProcess::launch(&executable, SessionMode::Headless).unwrap();
        let profile = process.profile_directory.clone();
        assert!(matches!(
            process.wait_for_endpoint().await,
            Err(ProviderError::StartupExit(_))
        ));
        process.stop();
        drop(process);
        assert!(!profile.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dedicated_provider_launch_failure_removes_its_spawned_profile() {
        use crate::sessions::ProviderGrants;
        use std::os::unix::fs::PermissionsExt;

        let root = unique_profile_directory().unwrap();
        let executable = root.join("failing chrome executable");
        let marker = root.join("profile-path.txt");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in --user-data-dir=*) profile=\"${{arg#*=}}\";; esac; done\nprintf '%s' \"$profile\" > '{}'\nexit 3\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: Vec::new(),
                },
                "launch-failure-test",
            )
            .unwrap();
        let provider = DedicatedChromeProvider::new(&executable);
        assert!(matches!(
            provider
                .launch(&mut registry, &session, "about:blank")
                .await,
            Err(ProviderError::StartupExit(_))
        ));
        let attempted_profile = std::fs::read_to_string(marker).unwrap();
        assert!(!Path::new(&attempted_profile).exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn canceling_dedicated_launch_kills_child_and_removes_private_profile() {
        use crate::sessions::ProviderGrants;
        use std::os::unix::fs::PermissionsExt;

        let root = unique_profile_directory().unwrap();
        let executable = root.join("slow chrome executable");
        let marker = root.join("launch-state.txt");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in --user-data-dir=*) profile=\"${{arg#*=}}\";; esac; done\nprintf '%s\\n%s' \"$profile\" \"$$\" > '{}'\nexec /bin/sleep 60\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let provider = DedicatedChromeProvider::new(&executable);
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: Vec::new(),
                },
                "cancel-launch-test",
            )
            .unwrap();
        let launch = tokio::spawn(async move {
            provider
                .launch(&mut registry, &session, "about:blank")
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !marker.is_file() {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let state = std::fs::read_to_string(marker).unwrap();
        let mut lines = state.lines();
        let profile = PathBuf::from(lines.next().unwrap());
        let pid: u32 = lines.next().unwrap().parse().unwrap();
        launch.abort();
        let _ = launch.await;
        assert!(!profile.exists(), "canceled launch left a private profile");
        assert!(
            !Command::new("/bin/kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()
                .unwrap()
                .success(),
            "canceled launch left its child process running"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_returns_live_recovery_when_owned_target_lacks_observer() {
        use crate::sessions::ProviderGrants;
        use futures_util::{SinkExt, StreamExt};

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            for event in [
                json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"17","type":"page"}}}),
                json!({"method":"Target.attachedToTarget","params":{"sessionId":"cdp-17","targetInfo":{"targetId":"17","type":"page"}}}),
            ] {
                socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .unwrap();
            }
            let request = socket.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(&request.to_string()).unwrap();
            assert_eq!(request["method"], "Target.createTarget");
            socket
                .send(Message::Text(
                    json!({
                        "id":request["id"], "result":{"targetId":"17"}
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.next())
                    .await
                    .is_err()
            );
        });
        let connection = Arc::new(
            BrowserConnection::connect(&format!("ws://{address}"))
                .await
                .unwrap(),
        );
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let handle = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: Vec::new(),
                },
                "fixture",
            )
            .unwrap();
        connection
            .create_owned_target(&mut registry, &handle, "about:blank")
            .await
            .unwrap();
        let root = unique_profile_directory().unwrap();
        let profile = root.join("private profile");
        std::fs::create_dir(&profile).unwrap();
        restrict_profile_permissions(&profile).unwrap();
        let child = Command::new("/bin/sleep").arg("60").spawn().unwrap();
        let child_pid = child.id();
        let session = DedicatedBrowserSession {
            process: ChromeProcess {
                child: Some(child),
                profile_directory: profile.clone(),
                preserve_on_drop: true,
                close_requested: false,
            },
            connection,
            endpoint: format!("ws://{address}"),
            target_id: "17".to_owned(),
            session_id: handle.id,
            mode: SessionMode::Headless,
            manager: BrowserManager::new(),
            shutdown_phase: ShutdownPhase::ReleaseTargets,
            last_receipt: None,
        };
        let outcome = session.shutdown(&mut registry, None).await;
        assert_eq!(outcome.receipt.remaining[0].target_id, "17");
        let mut recovery = outcome.recovery.expect("live recovery handle was lost");
        assert!(profile.is_dir());
        assert!(
            recovery
                .process
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        drop(recovery);
        assert!(profile.is_dir(), "dropping recovery erased the profile");
        assert!(
            Command::new("/bin/kill")
                .arg("-0")
                .arg(child_pid.to_string())
                .status()
                .unwrap()
                .success()
        );
        Command::new("/bin/kill")
            .arg("-9")
            .arg(child_pid.to_string())
            .status()
            .unwrap();
        std::fs::remove_dir_all(&profile).unwrap();
        assert!(!profile.exists());
        std::fs::remove_dir_all(root).unwrap();
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn headless_shutdown_preserves_browser_when_unowned_page_remains() {
        use crate::sessions::ProviderGrants;
        use futures_util::StreamExt;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                let request: Value = serde_json::from_str(&message.to_string()).unwrap();
                assert_eq!(request["method"], "Target.getTargets");
                socket.send(Message::Text(
                    json!({"id":request["id"], "result":{"targetInfos":[{"targetId":"user-page","type":"page"}]}})
                        .to_string().into(),
                )).await.unwrap();
            }
        });
        let connection = Arc::new(
            BrowserConnection::connect(&format!("ws://{address}"))
                .await
                .unwrap(),
        );
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let handle = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: Vec::new(),
                },
                "fixture",
            )
            .unwrap();
        let root = unique_profile_directory().unwrap();
        let profile = root.join("recoverable profile");
        std::fs::create_dir(&profile).unwrap();
        let child = Command::new("/bin/sleep").arg("60").spawn().unwrap();
        let session = DedicatedBrowserSession {
            process: ChromeProcess {
                child: Some(child),
                profile_directory: profile.clone(),
                preserve_on_drop: true,
                close_requested: false,
            },
            connection,
            endpoint: format!("ws://{address}"),
            target_id: "created-target".into(),
            session_id: handle.id,
            mode: SessionMode::Headless,
            manager: BrowserManager::new(),
            shutdown_phase: ShutdownPhase::ProcessCleanup,
            last_receipt: Some(crate::sessions::CleanupReceipt::default()),
        };
        let outcome = session.shutdown(&mut registry, None).await;
        assert!(
            outcome
                .cleanup_error
                .unwrap()
                .contains("live page targets remain")
        );
        let mut recovery = outcome.recovery.unwrap();
        assert!(profile.is_dir());
        assert!(
            recovery
                .process
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        recovery.process.stop();
        drop(recovery);
        std::fs::remove_dir_all(root).unwrap();
        server.abort();
        let _ = server.await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn headed_shutdown_never_closes_the_entire_browser_window() {
        use crate::sessions::ProviderGrants;
        use futures_util::StreamExt;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.next())
                    .await
                    .is_err()
            );
        });
        let connection = Arc::new(
            BrowserConnection::connect(&format!("ws://{address}"))
                .await
                .unwrap(),
        );
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headed: true,
            ..ProviderGrants::default()
        });
        let handle = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "fixture",
            )
            .unwrap();
        let root = unique_profile_directory().unwrap();
        let profile = root.join("headed profile");
        std::fs::create_dir(&profile).unwrap();
        let child = Command::new("/bin/sleep").arg("60").spawn().unwrap();
        let session = DedicatedBrowserSession {
            process: ChromeProcess {
                child: Some(child),
                profile_directory: profile.clone(),
                preserve_on_drop: true,
                close_requested: false,
            },
            connection,
            endpoint: format!("ws://{address}"),
            target_id: "created-target".into(),
            session_id: handle.id,
            mode: SessionMode::Headed,
            manager: BrowserManager::new(),
            shutdown_phase: ShutdownPhase::ProcessCleanup,
            last_receipt: Some(crate::sessions::CleanupReceipt::default()),
        };
        let outcome = session.shutdown(&mut registry, None).await;
        assert!(
            outcome
                .cleanup_error
                .unwrap()
                .contains("headed Chrome is preserved")
        );
        let mut recovery = outcome.recovery.unwrap();
        assert!(profile.is_dir());
        assert!(
            recovery
                .process
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        recovery.process.stop();
        std::fs::remove_dir_all(root).unwrap();
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_cleanup_retry_skips_retired_session_release() {
        use crate::sessions::{IndependentTargetObserver, ProviderGrants};
        use futures_util::{SinkExt, StreamExt};

        struct TestObserver;
        impl IndependentTargetObserver for TestObserver {
            fn verify_unchanged(
                &self,
                _: &crate::sessions::CleanupObservation,
            ) -> Result<(), String> {
                Ok(())
            }
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let root = unique_profile_directory().unwrap();
        let profile = root.join("retry profile");
        std::fs::create_dir(&profile).unwrap();
        let child = Command::new("/bin/sleep").arg("60").spawn().unwrap();
        let child_pid = child.id();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            for event in [
                json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned","type":"page","url":"about:blank"}}}),
                json!({"method":"Target.attachedToTarget","params":{"sessionId":"cdp-owned","targetInfo":{"targetId":"owned","type":"page"}}}),
            ] {
                socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .unwrap();
            }
            let create = socket.next().await.unwrap().unwrap();
            let create: Value = serde_json::from_str(&create.to_string()).unwrap();
            assert_eq!(create["method"], "Target.createTarget");
            socket
                .send(Message::Text(
                    json!({"id":create["id"], "result":{"targetId":"owned"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();

            let close_target = socket.next().await.unwrap().unwrap();
            let close_target: Value = serde_json::from_str(&close_target.to_string()).unwrap();
            assert_eq!(close_target["method"], "Target.closeTarget");
            socket
                .send(Message::Text(
                    json!({"id":close_target["id"], "result":{"success":true}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();

            let first_targets = socket.next().await.unwrap().unwrap();
            let first_targets: Value = serde_json::from_str(&first_targets.to_string()).unwrap();
            assert_eq!(first_targets["method"], "Target.getTargets");
            socket.send(Message::Text(json!({"id":first_targets["id"], "error":{"code":-32000,"message":"fixture query failure"}}).to_string().into())).await.unwrap();

            let retry_targets = socket.next().await.unwrap().unwrap();
            let retry_targets: Value = serde_json::from_str(&retry_targets.to_string()).unwrap();
            assert_eq!(
                retry_targets["method"], "Target.getTargets",
                "retry must resume process cleanup without releasing retired session again"
            );
            socket
                .send(Message::Text(
                    json!({"id":retry_targets["id"], "result":{"targetInfos":[]}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();

            let failed_close = socket.next().await.unwrap().unwrap();
            let failed_close: Value = serde_json::from_str(&failed_close.to_string()).unwrap();
            assert_eq!(failed_close["method"], "Browser.close");
            socket.send(Message::Text(json!({"id":failed_close["id"], "error":{"code":-32000,"message":"fixture close failure"}}).to_string().into())).await.unwrap();

            let retry_targets = socket.next().await.unwrap().unwrap();
            let retry_targets: Value = serde_json::from_str(&retry_targets.to_string()).unwrap();
            assert_eq!(retry_targets["method"], "Target.getTargets");
            socket
                .send(Message::Text(
                    json!({"id":retry_targets["id"], "result":{"targetInfos":[]}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();

            let browser_close = socket.next().await.unwrap().unwrap();
            let browser_close: Value = serde_json::from_str(&browser_close.to_string()).unwrap();
            assert_eq!(browser_close["method"], "Browser.close");
            socket
                .send(Message::Text(
                    json!({"id":browser_close["id"], "result":{}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            Command::new("/bin/kill")
                .arg("-TERM")
                .arg(child_pid.to_string())
                .status()
                .unwrap();
        });
        let connection = Arc::new(
            BrowserConnection::connect(&format!("ws://{address}"))
                .await
                .unwrap(),
        );
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let handle = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: Vec::new(),
                },
                "fixture",
            )
            .unwrap();
        let target_id = connection
            .create_owned_target(&mut registry, &handle, "about:blank")
            .await
            .unwrap();
        let session = DedicatedBrowserSession {
            process: ChromeProcess {
                child: Some(child),
                profile_directory: profile.clone(),
                preserve_on_drop: true,
                close_requested: false,
            },
            connection,
            endpoint: format!("ws://{address}"),
            target_id,
            session_id: handle.id,
            mode: SessionMode::Headless,
            manager: BrowserManager::new(),
            shutdown_phase: ShutdownPhase::ReleaseTargets,
            last_receipt: None,
        };
        let first = session.shutdown(&mut registry, Some(&TestObserver)).await;
        assert_eq!(first.receipt.closed, vec!["owned"]);
        assert!(
            first
                .cleanup_error
                .unwrap()
                .contains("cannot verify remaining page targets")
        );
        let recovery = first.recovery.unwrap();
        assert!(profile.is_dir());
        let second = recovery.shutdown(&mut registry, None).await;
        assert!(
            second
                .cleanup_error
                .unwrap()
                .contains("fixture close failure")
        );
        let mut recovery = second.recovery.expect("close failure must retain recovery");
        assert!(profile.is_dir());
        assert!(
            recovery
                .process
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        assert!(!recovery.process.close_requested);
        let third = recovery.shutdown(&mut registry, None).await;
        assert!(third.cleanup_error.is_none(), "{:?}", third.cleanup_error);
        assert!(third.recovery.is_none());
        assert!(!profile.exists());
        std::fs::remove_dir_all(root).unwrap();
        server.await.unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires installed Google Chrome; runs an isolated headless profile"]
    async fn real_chrome_headless_provider_launch_and_runtime_smoke() {
        use crate::sessions::{CleanupObservation, IndependentTargetObserver, ProviderGrants};
        let executable = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(executable.is_file());
        let provider = DedicatedChromeProvider::new(executable);
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let handle = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: Vec::new(),
                },
                "real-chrome-fixture",
            )
            .unwrap();
        let page =
            "data:text/html,%3Cinput%20id%3D%22field%22%20type%3D%22text%22%20value%3D%22%22%3E";
        let mut session = provider.launch(&mut registry, &handle, page).await.unwrap();
        // A failed assertion must not leave this test's isolated Chrome alive.
        session.process.preserve_on_drop = false;
        assert!(session.profile_directory().is_dir());
        let (_, targets) = session.connection().target_snapshot().await;
        assert_eq!(
            targets
                .iter()
                .filter(|target| target.target_type == "page")
                .count(),
            1
        );
        let target = targets
            .iter()
            .find(|target| target.id == session.target_id())
            .unwrap();
        let result = session
            .connection()
            .target_command(
                &target.id,
                target.generation,
                &target.revision,
                "Runtime.evaluate",
                json!({"expression":"document.visibilityState", "returnByValue":true}),
            )
            .await
            .unwrap();
        assert_eq!(result["result"]["value"], "visible");
        let frame_id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let target_now = session
                    .connection()
                    .targets
                    .read()
                    .await
                    .targets
                    .get(&target.id)
                    .cloned();
                if let Some(target_now) = target_now
                    && let Ok(ready) = session
                        .connection()
                        .target_command(
                            &target.id,
                            target_now.generation,
                            &target_now.revision,
                            "Runtime.evaluate",
                            json!({"expression":"!!document.querySelector('#field')","returnByValue":true}),
                        )
                        .await
                    && ready["result"]["value"] == true
                    && let Some(frame) = session
                        .connection()
                        .frames
                        .read()
                        .await
                        .frames
                        .values()
                        .find(|frame| frame.target_id == target.id)
                {
                    break frame.id.clone();
                }
                let frames = session.connection().frames.read().await;
                drop(frames);
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|error| {
            let frames = session
                .connection()
                .frames
                .try_read()
                .map(|frames| format!("{:?}", frames.frames.values().collect::<Vec<_>>()))
                .unwrap_or_else(|error| error.to_string());
            panic!("input fixture navigation did not create a frame: {error:?}; target={target:?}; frames={frames}");
        });
        let reference = session
            .connection()
            .capture_target_ref(&registry, &handle, &target.id, &frame_id, 1, 1)
            .await
            .unwrap();
        session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                "Runtime.evaluate",
                json!({"expression":"(()=>{document.body.innerHTML='<input id=field type=text><input id=interference type=text value=before><input id=masked type=tel data-masked value=><div id=editable contenteditable=true></div><input id=dependent type=text data-requires-trusted><input id=password type=password value=secret><canvas id=canvas width=200 height=100></canvas><button id=covered style=\"position:absolute;left:250px;top:20px;width:80px;height:40px\">covered</button><div id=overlay style=\"position:absolute;z-index:2;left:250px;top:20px;width:80px;height:40px\"></div><div id=drag style=\"position:absolute;left:20px;top:100px;width:40px;height:40px;background:red\"></div>';document.querySelector('#interference').addEventListener('focus',e=>e.target.value='external');const d=document.querySelector('#drag');let active=false,ox=0,oy=0;d.addEventListener('mousedown',e=>{active=true;ox=e.clientX-d.getBoundingClientRect().left;oy=e.clientY-d.getBoundingClientRect().top});document.addEventListener('mousemove',e=>{if(active){d.style.left=(e.clientX-ox)+'px';d.style.top=(e.clientY-oy)+'px'}});document.addEventListener('mouseup',()=>active=false);document.querySelector('#covered').addEventListener('click',e=>e.target.dataset.clicked='yes');const dep=document.querySelector('#dependent');dep.addEventListener('input',e=>{if(e.isTrusted)dep.dataset.model=dep.value});return true})()","returnByValue":true}),
            )
            .await
            .unwrap();
        let snapshot = crate::input::GuardSnapshot {
            navigation: 1,
            account: 1,
            document: 1,
            dependencies: ["#field".into()].into_iter().collect(),
            strict_background: true,
            requires_native: false,
        };
        let locator = crate::input::SemanticLocator::Css("#field".into());
        let native_before = crate::native::NativeSnapshot::capture().unwrap();
        let fill = crate::input::InputAction::Fill("héllo 👋".into());
        let filled = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &locator,
                    action: &fill,
                    expected_value: "",
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(filled,crate::input::InputOutcome::Applied{observed_value:Some(value),postcondition_verified:true,..} if value=="héllo 👋")
        );
        let native_after = crate::native::NativeSnapshot::capture().unwrap();
        assert_eq!(
            native_before.frontmost_bundle_id,
            native_after.frontmost_bundle_id
        );
        assert_eq!(
            native_before.pasteboard_change_count,
            native_after.pasteboard_change_count
        );
        assert_eq!(native_before.cursor_x, native_after.cursor_x);
        assert_eq!(native_before.cursor_y, native_after.cursor_y);
        session.connection().target_ref_command(&registry,&reference,"real-chrome-fixture",crate::sessions::IdentityRevisions{account:1,document:1},"Runtime.evaluate",json!({"expression":"(()=>{const e=document.querySelector('#field');e.setSelectionRange(e.value.length,e.value.length);return e.selectionStart})()","returnByValue":true})).await.unwrap();
        let insert = crate::input::InputAction::Insert("λ".into());
        let inserted = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &locator,
                    action: &insert,
                    expected_value: "héllo 👋",
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(inserted,crate::input::InputOutcome::Applied{observed_value:Some(value),postcondition_verified:true,..} if value=="héllo 👋λ")
        );
        let keys = crate::input::InputAction::SequentialKeys("a".into());
        let typed = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &locator,
                    action: &keys,
                    expected_value: "héllo 👋λ",
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(typed,crate::input::InputOutcome::Applied{observed_value:Some(value),postcondition_verified:true,..} if value=="héllo 👋λa")
        );
        let position=session.connection().target_ref_command(&registry,&reference,"real-chrome-fixture",crate::sessions::IdentityRevisions{account:1,document:1},"Runtime.evaluate",json!({"expression":"({value:document.querySelector('#field').value,selectionStart:document.querySelector('#field').selectionStart})","returnByValue":true})).await.unwrap();
        assert_eq!(position["result"]["value"]["value"], "héllo 👋λa");
        assert_eq!(
            position["result"]["value"]["selectionStart"],
            "héllo 👋λa".encode_utf16().count()
        );
        let stale_fill = crate::input::InputAction::Fill("must-not-write".into());
        let stale = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &locator,
                    action: &stale_fill,
                    expected_value: "old-value",
                },
            )
            .await;
        assert!(matches!(stale, Err(crate::BrowserError::StaleReference(_))));

        // Real Chrome event ordering: focusing the field changes it after the
        // initial probe, so the final guarded fill must yield without overwrite.
        let interference_locator = crate::input::SemanticLocator::Css("#interference".into());
        let interference_fill = crate::input::InputAction::Fill("requested".into());
        let interference = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &interference_locator,
                    action: &interference_fill,
                    expected_value: "before",
                },
            )
            .await
            .unwrap();
        assert!(matches!(interference, crate::input::InputOutcome::Stale(_)));
        let interference_value = session.connection().target_ref_command(
            &registry, &reference, "real-chrome-fixture",
            crate::sessions::IdentityRevisions { account: 1, document: 1 },
            "Runtime.evaluate",
            json!({"expression":"document.querySelector('#interference').value","returnByValue":true}),
        ).await.unwrap();
        assert_eq!(interference_value["result"]["value"], "external");

        // The browser's IME protocol emits a real composition event sequence;
        // this probes Chrome protocol support, not a product-level input action.
        session.connection().target_ref_command(
            &registry, &reference, "real-chrome-fixture",
            crate::sessions::IdentityRevisions { account: 1, document: 1 },
            "Runtime.evaluate",
            json!({"expression":"window.compositionEvents=[];const e=document.querySelector('#field');for(const name of ['compositionstart','compositionupdate','compositionend'])e.addEventListener(name,event=>window.compositionEvents.push({type:event.type,data:event.data}));e.addEventListener('input',event=>window.compositionEvents.push({type:'input',isComposing:event.isComposing}));e.focus();true","returnByValue":true}),
        ).await.unwrap();
        session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                "Input.imeSetComposition",
                json!({"text":"あ","selectionStart":1,"selectionEnd":1}),
            )
            .await
            .unwrap();
        let composing = session.connection().target_ref_command(
            &registry, &reference, "real-chrome-fixture",
            crate::sessions::IdentityRevisions { account: 1, document: 1 },
            "Runtime.evaluate",
            json!({"expression":"({events:window.compositionEvents.map(event=>event.type),composingInput:window.compositionEvents.some(event=>event.type==='input'&&event.isComposing),value:document.querySelector('#field').value})","returnByValue":true}),
        ).await.unwrap();
        assert!(
            composing["result"]["value"]["events"]
                .as_array()
                .unwrap()
                .contains(&json!("compositionstart"))
        );
        assert!(
            composing["result"]["value"]["events"]
                .as_array()
                .unwrap()
                .contains(&json!("compositionupdate"))
        );
        assert_eq!(composing["result"]["value"]["composingInput"], true);
        session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                "Input.insertText",
                json!({"text":"あ"}),
            )
            .await
            .unwrap();
        let ended = session.connection().target_ref_command(
            &registry, &reference, "real-chrome-fixture",
            crate::sessions::IdentityRevisions { account: 1, document: 1 },
            "Runtime.evaluate",
            json!({"expression":"({events:window.compositionEvents.map(event=>event.type),value:document.querySelector('#field').value})","returnByValue":true}),
        ).await.unwrap();
        assert!(
            ended["result"]["value"]["events"]
                .as_array()
                .unwrap()
                .contains(&json!("compositionend"))
        );
        assert_eq!(ended["result"]["value"]["value"], "héllo 👋λaあ");

        session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                "Runtime.evaluate",
                json!({"expression":"window.compositionEvents=[];true","returnByValue":true}),
            )
            .await
            .unwrap();
        let ime = crate::input::InputAction::ImeText("に".into());
        let ime_result = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &locator,
                    action: &ime,
                    expected_value: "héllo 👋λaあ",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            ime_result,
            crate::input::InputOutcome::Applied {
                observed_value: Some(value),
                postcondition_verified: true,
                ..
            } if value == "héllo 👋λaあに"
        ));
        let ime_events = session.connection().target_ref_command(
            &registry, &reference, "real-chrome-fixture",
            crate::sessions::IdentityRevisions { account: 1, document: 1 },
            "Runtime.evaluate",
            json!({"expression":"window.compositionEvents.map(event=>event.type)","returnByValue":true}),
        ).await.unwrap();
        let event_types = ime_events["result"]["value"].as_array().unwrap();
        assert!(event_types.contains(&json!("compositionstart")));
        assert!(event_types.contains(&json!("compositionupdate")));
        assert!(event_types.contains(&json!("compositionend")));
        let composing_input = session.connection().target_ref_command(
            &registry,
            &reference,
            "real-chrome-fixture",
            crate::sessions::IdentityRevisions { account: 1, document: 1 },
            "Runtime.evaluate",
            json!({"expression":"window.compositionEvents.some(event=>event.type==='input'&&event.isComposing)","returnByValue":true}),
        ).await.unwrap();
        assert_eq!(composing_input["result"]["value"], true);

        let masked_locator = crate::input::SemanticLocator::Css("#masked".into());
        let masked_fill = crate::input::InputAction::Fill("2125550100".into());
        let masked_result = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &masked_locator,
                    action: &masked_fill,
                    expected_value: "",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            masked_result,
            crate::input::InputOutcome::Unsupported(_)
        ));
        let masked_value = session.connection().target_ref_command(&registry, &reference, "real-chrome-fixture", crate::sessions::IdentityRevisions { account: 1, document: 1 }, "Runtime.evaluate", json!({"expression":"document.querySelector('#masked').value","returnByValue":true})).await.unwrap();
        assert_eq!(masked_value["result"]["value"], "");

        for (selector, value) in [
            ("#editable", "plain contenteditable"),
            ("#dependent", "event dependent"),
        ] {
            let locator = crate::input::SemanticLocator::Css(selector.into());
            let fill = crate::input::InputAction::Fill(value.into());
            let result = session
                .connection()
                .perform_guarded_input(
                    &registry,
                    &reference,
                    "real-chrome-fixture",
                    crate::sessions::IdentityRevisions {
                        account: 1,
                        document: 1,
                    },
                    crate::input::GuardedInput {
                        expected: &snapshot,
                        current: &snapshot,
                        locator: &locator,
                        action: &fill,
                        expected_value: "",
                    },
                )
                .await
                .unwrap();
            assert!(
                matches!(result, crate::input::InputOutcome::Unsupported(_)),
                "{selector}: {result:?}"
            );
        }
        let password_locator = crate::input::SemanticLocator::Css("#password".into());
        let password_fill = crate::input::InputAction::Fill("replacement".into());
        let password_result = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &password_locator,
                    action: &password_fill,
                    expected_value: "",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            password_result,
            crate::input::InputOutcome::Unsupported(_)
        ));
        assert!(format!("{password_result:?}").find("secret").is_none());

        let covered_locator = crate::input::SemanticLocator::Css("#covered".into());
        let covered_click = crate::input::InputAction::Click {
            x: 280.0,
            y: 40.0,
            postcondition: crate::input::ClickPostcondition::ActiveElement,
        };
        let mut foreground_snapshot = snapshot.clone();
        foreground_snapshot.strict_background = false;
        let covered_result = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &foreground_snapshot,
                    current: &foreground_snapshot,
                    locator: &covered_locator,
                    action: &covered_click,
                    expected_value: "",
                },
            )
            .await;
        assert!(matches!(
            covered_result,
            Err(crate::BrowserError::StaleReference(_))
        ));
        let covered_state = session.connection().target_ref_command(&registry, &reference, "real-chrome-fixture", crate::sessions::IdentityRevisions { account: 1, document: 1 }, "Runtime.evaluate", json!({"expression":"document.querySelector('#covered').dataset.clicked||''","returnByValue":true})).await.unwrap();
        assert_eq!(covered_state["result"]["value"], "");

        let drag_locator = crate::input::SemanticLocator::Css("#drag".into());
        let drag = crate::input::InputAction::Drag {
            from: (40.0, 120.0),
            to: (160.0, 180.0),
            postcondition: crate::input::DragPostcondition {
                left: 140.0,
                top: 160.0,
                tolerance: 1.0,
            },
        };
        let dragged = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &foreground_snapshot,
                    current: &foreground_snapshot,
                    locator: &drag_locator,
                    action: &drag,
                    expected_value: "",
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
        let canvas_drag = crate::input::InputAction::Drag {
            from: (10.0, 10.0),
            to: (80.0, 60.0),
            postcondition: crate::input::DragPostcondition {
                left: 0.0,
                top: 0.0,
                tolerance: 0.0,
            },
        };
        let canvas_result = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &foreground_snapshot,
                    current: &foreground_snapshot,
                    locator: &crate::input::SemanticLocator::Css("#canvas".into()),
                    action: &canvas_drag,
                    expected_value: "",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            canvas_result,
            crate::input::InputOutcome::Unsupported(_)
        ));
        let empty_paste = crate::input::InputAction::PasteInternalClipboard;
        let empty_paste_result = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &locator,
                    action: &empty_paste,
                    expected_value: "héllo 👋λaあに",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            empty_paste_result,
            crate::input::InputOutcome::Unsupported(_)
        ));
        registry
            .set_internal_clipboard_text(&handle, " clipboard text")
            .unwrap();
        let native_before_internal_paste = crate::native::NativeSnapshot::capture().unwrap();
        let paste = crate::input::InputAction::PasteInternalClipboard;
        let pasted = session
            .connection()
            .perform_guarded_input(
                &registry,
                &reference,
                "real-chrome-fixture",
                crate::sessions::IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                crate::input::GuardedInput {
                    expected: &snapshot,
                    current: &snapshot,
                    locator: &locator,
                    action: &paste,
                    expected_value: "héllo 👋λaあに",
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            pasted,
            crate::input::InputOutcome::Applied {
                observed_value: Some(value),
                postcondition_verified: true,
                ..
            } if value == "héllo 👋λaあに clipboard text"
        ));
        assert_eq!(
            registry.internal_clipboard_text(&handle).unwrap(),
            Some(" clipboard text")
        );
        let native_after_internal_paste = crate::native::NativeSnapshot::capture().unwrap();
        assert_eq!(
            native_before_internal_paste.frontmost_bundle_id,
            native_after_internal_paste.frontmost_bundle_id
        );
        assert_eq!(
            native_before_internal_paste.pasteboard_change_count,
            native_after_internal_paste.pasteboard_change_count
        );
        struct TestOwnedTargetObserver;
        impl IndependentTargetObserver for TestOwnedTargetObserver {
            fn verify_unchanged(&self, _: &CleanupObservation) -> Result<(), String> {
                Ok(())
            }
        }
        let profile = session.profile_directory().to_owned();
        let outcome = session
            .shutdown(&mut registry, Some(&TestOwnedTargetObserver))
            .await;
        assert_eq!(outcome.receipt.closed, vec![target.id.clone()]);
        assert!(outcome.receipt.remaining.is_empty());
        assert!(
            outcome.cleanup_error.is_none(),
            "{:?}",
            outcome.cleanup_error
        );
        assert!(outcome.recovery.is_none());
        assert!(
            !profile.exists(),
            "isolated Chrome profile remained after shutdown: {profile:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires installed Google Chrome; runs an isolated headless profile"]
    async fn real_chrome_extracts_virtualized_phase5_fixture_and_guards_wrong_account() {
        use crate::{
            observe::{Completeness, ExtractionSpec},
            sessions::{CleanupObservation, IndependentTargetObserver, ProviderGrants},
        };
        use std::collections::BTreeMap;

        let executable = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        assert!(executable.is_file());
        let html = r#"<!doctype html><meta charset="utf-8">
<div id="account">fixture-account</div>
<div id="list"><div id="spacer"></div><div id="rows"></div></div>
<script>
const list=document.querySelector('#list'),rows=document.querySelector('#rows');
list.style.cssText='height:240px;overflow:auto;position:relative';
document.querySelector('#spacer').style.height='1680px';
const render=()=>{const first=Math.min(34,Math.floor(list.scrollTop/40));rows.replaceChildren();
for(let i=first;i<Math.min(42,first+8);i++){const r=document.createElement('div');r.className='record';
r.style.cssText='position:absolute;top:'+(i*40)+'px;height:40px';
r.innerHTML='<span class="id">item-'+String(i+1).padStart(2,'0')+'</span><span class="label">record '+(i+1)+'</span>';rows.append(r)}
list.querySelector('.terminal')?.remove();
if(list.scrollTop+list.clientHeight>=list.scrollHeight-2){const end=document.createElement('span');end.className='terminal';end.style.cssText='position:absolute;bottom:0';end.textContent='end';list.append(end)}};
list.addEventListener('scroll',render);render();
</script>"#;
        let encoded = html
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect::<String>();
        let provider = DedicatedChromeProvider::new(executable);
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let handle = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: Vec::new(),
                },
                "real-chrome-phase5-fixture",
            )
            .unwrap();
        let mut session = provider
            .launch(&mut registry, &handle, &format!("data:text/html,{encoded}"))
            .await
            .unwrap();
        // Drop always kills the isolated child and removes its profile after a failed assertion.
        session.process.preserve_on_drop = false;
        let (_, targets) = session.connection().target_snapshot().await;
        let target = targets
            .iter()
            .find(|target| target.id == session.target_id())
            .unwrap();
        let frame_id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let target_now = session
                    .connection()
                    .targets
                    .read()
                    .await
                    .targets
                    .get(&target.id)
                    .cloned();
                if let Some(target_now) = target_now
                    && let Ok(ready) = session
                        .connection()
                        .target_command(
                            &target.id,
                            target_now.generation,
                            &target_now.revision,
                            "Runtime.evaluate",
                            json!({"expression":"!!document.querySelector('#list .record')","returnByValue":true}),
                        )
                        .await
                    && ready["result"]["value"] == true
                    && let Some(frame) = session
                        .connection()
                        .frames
                        .read()
                        .await
                        .frames
                        .values()
                        .find(|frame| frame.target_id == target.id)
                {
                    break frame.id.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("virtualized fixture did not load");
        let reference = session
            .connection()
            .capture_target_ref(&registry, &handle, &target.id, &frame_id, 1, 1)
            .await
            .unwrap();
        let revisions = crate::sessions::IdentityRevisions {
            account: 1,
            document: 1,
        };
        let spec = ExtractionSpec {
            container: "#list".into(),
            record: ".record".into(),
            fields: BTreeMap::from([
                ("id".into(), ".id".into()),
                ("label".into(), ".label".into()),
            ]),
            id_field: "id".into(),
            max_steps: 40,
            max_records: 100,
            max_text_chars: 100,
            max_bytes: 64 * 1024,
            expected_count: Some(42),
            account_marker: Some(("#account".into(), "fixture-account".into())),
            terminal_selector: Some(".terminal".into()),
            expand: vec![],
            cursor: None,
        };
        let extraction_started = std::time::Instant::now();
        let result = session
            .connection()
            .extract(
                &registry,
                &reference,
                "real-chrome-phase5-fixture",
                revisions,
                &spec,
            )
            .await;
        let extraction_elapsed = extraction_started.elapsed();
        let result = result.unwrap();
        assert_eq!(result.unique_count, 42, "{result:?}");
        assert_eq!(result.records.len(), 42);
        assert_eq!(result.completeness, Completeness::Complete, "{result:?}");
        assert_eq!(result.expected_count, Some(42));
        assert!(!result.terminal_evidence.is_empty());
        let ids = result
            .records
            .iter()
            .map(|record| record["id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), 42);

        let baseline_started = std::time::Instant::now();
        let baseline = session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-phase5-fixture",
                revisions,
                "Runtime.evaluate",
                json!({"expression":"({html:document.documentElement.outerHTML,text:document.body.innerText})","returnByValue":true}),
            )
            .await
            .unwrap();
        let baseline_elapsed = baseline_started.elapsed();
        let baseline_bytes = serde_json::to_vec(&baseline).unwrap().len();
        let extraction_bytes = serde_json::to_vec(&result).unwrap().len();
        println!(
            "synthetic Chrome extraction diagnostic: extraction_elapsed_ms={} extraction_result_bytes={} full_dom_snapshot_elapsed_ms={} full_dom_snapshot_response_bytes={baseline_bytes}",
            extraction_elapsed.as_secs_f64() * 1000.0,
            extraction_bytes,
            baseline_elapsed.as_secs_f64() * 1000.0
        );

        session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-phase5-fixture",
                revisions,
                "Runtime.evaluate",
                json!({"expression":"(()=>{const l=document.querySelector('#list');l.scrollTop=80;document.querySelector('#account').textContent='wrong-account';return new Promise(r=>requestAnimationFrame(()=>r(l.scrollTop)))})()","returnByValue":true,"awaitPromise":true}),
            )
            .await
            .unwrap();
        let before = session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-phase5-fixture",
                revisions,
                "Runtime.evaluate",
                json!({"expression":"document.querySelector('#list').scrollTop","returnByValue":true}),
            )
            .await
            .unwrap()["result"]["value"]
            .as_f64()
            .unwrap();
        let wrong_account = session
            .connection()
            .extract(
                &registry,
                &reference,
                "real-chrome-phase5-fixture",
                revisions,
                &spec,
            )
            .await
            .unwrap();
        let after = session
            .connection()
            .target_ref_command(
                &registry,
                &reference,
                "real-chrome-phase5-fixture",
                revisions,
                "Runtime.evaluate",
                json!({"expression":"document.querySelector('#list').scrollTop","returnByValue":true}),
            )
            .await
            .unwrap()["result"]["value"]
            .as_f64()
            .unwrap();
        assert_eq!(wrong_account.completeness, Completeness::Unknown);
        assert_eq!(wrong_account.unique_count, 0);
        assert_eq!(before, after, "wrong-account extraction scrolled the list");

        struct OwnedTargetObserver;
        impl IndependentTargetObserver for OwnedTargetObserver {
            fn verify_unchanged(&self, _: &CleanupObservation) -> Result<(), String> {
                Ok(())
            }
        }
        let profile = session.profile_directory().to_owned();
        let outcome = session
            .shutdown(&mut registry, Some(&OwnedTargetObserver))
            .await;
        assert!(
            outcome.cleanup_error.is_none(),
            "{:?}",
            outcome.cleanup_error
        );
        assert!(outcome.recovery.is_none());
        assert!(!profile.exists(), "isolated profile remained: {profile:?}");
    }

    #[tokio::test]
    async fn dedicated_provider_rejects_shared_extension_session_before_launch() {
        let mut registry = SessionRegistry::new(crate::sessions::ProviderGrants {
            shared_extension: true,
            ..crate::sessions::ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["selected".to_owned()],
                },
                "alice",
            )
            .unwrap();
        let provider = DedicatedChromeProvider::new("/no/such/chrome");
        assert!(matches!(
            provider
                .launch(&mut registry, &session, "about:blank")
                .await,
            Err(ProviderError::Session(_))
        ));
    }

    #[tokio::test]
    async fn shared_extension_authenticates_selected_targets_and_routes_commands() {
        use crate::sessions::ProviderGrants;
        use serde_json::json;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["17".to_owned(), "18".to_owned()],
                },
                "alice",
            )
            .unwrap();
        let mut provider = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        let pairing = provider.pairing().clone();
        let (attached, mut socket) = tokio::join!(provider.accept(&mut registry), async {
            let (mut socket, _) = connect_async(&pairing.endpoint).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"hello", "token":pairing.token, "targets":["17", "18"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket
        });
        let attached = attached.unwrap();
        let command_socket = tokio::spawn(async move {
            let mut message = socket.next().await.unwrap().unwrap();
            while serde_json::from_str::<Value>(message.to_text().unwrap()).unwrap()["type"]
                == "ready"
            {
                message = socket.next().await.unwrap().unwrap();
            }
            let first: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let second = socket.next().await.unwrap().unwrap();
            let second: Value = serde_json::from_str(second.to_text().unwrap()).unwrap();
            for request in [first, second] {
                socket.send(Message::Text(
                    json!({"type":"result", "id":request["id"], "result":{"target":request["target_id"]}})
                        .to_string().into(),
                )).await.unwrap();
            }
        });
        let (first, second) = tokio::join!(
            attached.command(
                &registry,
                &session,
                "17",
                "Runtime.evaluate",
                json!({"expression":"1+1"}),
            ),
            attached.command(
                &registry,
                &session,
                "18",
                "Runtime.evaluate",
                json!({"expression":"2+2"}),
            ),
        );
        assert_eq!(first.unwrap(), json!({"target":"17"}));
        assert_eq!(second.unwrap(), json!({"target":"18"}));
        command_socket.await.unwrap();
        registry.revoke_provider(ProviderKind::SharedExtension);
        assert!(
            attached
                .command(&registry, &session, "17", "Runtime.evaluate", Value::Null)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn shared_extension_rejects_wrong_selection_and_token() {
        use crate::sessions::ProviderGrants;
        use serde_json::json;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["17".to_owned()],
                },
                "alice",
            )
            .unwrap();
        let mut provider = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        let pairing = provider.pairing().clone();
        let (accepted, _socket) = tokio::join!(provider.accept(&mut registry), async {
            let (mut bad_socket, _) = connect_async(&pairing.endpoint).await.unwrap();
            bad_socket
                .send(Message::Text(
                    json!({"type":"hello", "token":"wrong", "targets":["18"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            assert!(matches!(
                bad_socket.next().await.unwrap().unwrap(),
                Message::Close(_)
            ));
            let (mut socket, _) = connect_async(&pairing.endpoint).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"hello", "token":pairing.token, "targets":["17"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket
        });
        let mut accepted = accepted.unwrap();
        assert_eq!(
            accepted.selected_targets(),
            &BTreeSet::from(["17".to_owned()])
        );
        accepted.release().await.unwrap();
    }

    #[tokio::test]
    async fn shared_extension_provider_setup_does_not_mutate_existing_browser_binding() {
        use crate::sessions::ProviderGrants;
        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["17".into()],
                },
                "alice",
            )
            .unwrap();
        registry.bind_session_to_browser(&session, 99).unwrap();
        let provider = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        drop(provider);
        assert!(registry.session_bound_to_browser(&session.id, 99));
    }

    #[tokio::test]
    async fn shared_extension_failed_pairing_allows_fresh_provider_retry() {
        use crate::sessions::ProviderGrants;
        use serde_json::json;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["17".into()],
                },
                "alice",
            )
            .unwrap();
        let mut failed = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        let failed_instance = failed.browser_instance_id;
        let pairing = failed.pairing().clone();
        let timed_out = tokio::time::timeout(Duration::from_millis(150), async {
            tokio::join!(failed.accept(&mut registry), async {
                let (mut peer, _) = connect_async(&pairing.endpoint).await.unwrap();
                peer.send(Message::Text(
                    json!({"type":"hello", "token":"bad", "targets":["17"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
                assert!(matches!(
                    peer.next().await.unwrap().unwrap(),
                    Message::Close(_)
                ));
            })
        })
        .await;
        assert!(timed_out.is_err());
        assert!(!registry.session_bound_to_browser(&session.id, failed_instance));
        drop(failed);

        let mut retry = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        let pairing = retry.pairing().clone();
        let (accepted, _) = tokio::join!(retry.accept(&mut registry), async {
            let (mut peer, _) = connect_async(&pairing.endpoint).await.unwrap();
            peer.send(Message::Text(
                json!({"type":"hello", "token":pairing.token, "targets":["17"]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            peer
        });
        let accepted = accepted.unwrap();
        assert!(registry.session_bound_to_browser(&session.id, accepted.browser_instance_id()));
    }

    #[tokio::test]
    async fn shared_extension_release_and_drop_close_peer_connection() {
        use crate::sessions::ProviderGrants;
        use futures_util::StreamExt;
        use serde_json::json;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["17".into()],
                },
                "alice",
            )
            .unwrap();
        let mut provider = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        let pairing = provider.pairing().clone();
        let (accepted, mut peer) = tokio::join!(provider.accept(&mut registry), async {
            let (mut peer, _) = connect_async(&pairing.endpoint).await.unwrap();
            peer.send(Message::Text(
                json!({"type":"hello", "token":pairing.token, "targets":["17"]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            peer
        });
        let mut accepted = accepted.unwrap();
        assert!(matches!(
            peer.next().await.unwrap().unwrap(),
            Message::Text(_)
        ));
        accepted.release().await.unwrap();
        assert!(matches!(
            peer.next().await.unwrap().unwrap(),
            Message::Close(_)
        ));
        accepted.release().await.unwrap();
    }

    #[tokio::test]
    async fn dropping_shared_extension_session_closes_peer_connection() {
        use crate::sessions::ProviderGrants;
        use futures_util::StreamExt;
        use serde_json::json;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["17".into()],
                },
                "alice",
            )
            .unwrap();
        let mut provider = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        let pairing = provider.pairing().clone();
        let (accepted, mut peer) = tokio::join!(provider.accept(&mut registry), async {
            let (mut peer, _) = connect_async(&pairing.endpoint).await.unwrap();
            peer.send(Message::Text(
                json!({"type":"hello", "token":pairing.token, "targets":["17"]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            peer
        });
        let accepted = accepted.unwrap();
        assert!(matches!(
            peer.next().await.unwrap().unwrap(),
            Message::Text(_)
        ));
        drop(accepted);
        let terminal = tokio::time::timeout(Duration::from_secs(1), peer.next())
            .await
            .unwrap();
        assert!(
            matches!(terminal, None | Some(Err(_)) | Some(Ok(Message::Close(_)))),
            "drop did not terminate the provider socket: {terminal:?}"
        );
    }

    #[tokio::test]
    async fn shared_extension_rejects_unselected_tab_then_accepts_selected_retry() {
        use crate::sessions::ProviderGrants;
        use serde_json::json;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                crate::sessions::SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["17".into()],
                },
                "alice",
            )
            .unwrap();
        let mut provider = SharedExtensionProvider::bind(&mut registry, &session)
            .await
            .unwrap();
        let pairing = provider.pairing().clone();
        let (accepted, _socket) = tokio::join!(provider.accept(&mut registry), async {
            let (mut rejected, _) = connect_async(&pairing.endpoint).await.unwrap();
            rejected
                .send(Message::Text(
                    json!({"type":"hello", "token":pairing.token, "targets":["18"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            assert!(matches!(
                rejected.next().await.unwrap().unwrap(),
                Message::Close(_)
            ));
            let (mut socket, _) = connect_async(&pairing.endpoint).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"hello", "token":pairing.token, "targets":["17"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket
        });
        let accepted = accepted.unwrap();
        assert_eq!(accepted.selected_targets(), &BTreeSet::from(["17".into()]));
        assert!(registry.session_bound_to_browser(&session.id, accepted.browser_instance_id()));
    }
}
