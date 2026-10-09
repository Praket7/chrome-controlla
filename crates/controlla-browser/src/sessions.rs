//! Logical browser session identities and tab ownership bookkeeping.
//!
//! This registry does not launch Chrome or establish extension/native-messaging
//! connections. Providers must perform those effects before marking a session
//! connected; this module enforces their mode-specific grants and identity.

use crate::{FrameRecord, TargetRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_REGISTRY_NAMESPACE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    Headed,
    Headless,
    Shared,
    DirectCdp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    DedicatedHeaded,
    DedicatedHeadless,
    SharedExtension,
    DirectCdp,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderGrants {
    pub dedicated_headed: bool,
    pub dedicated_headless: bool,
    pub shared_extension: bool,
    pub direct_cdp: bool,
}

impl ProviderGrants {
    fn allows(self, provider: ProviderKind) -> bool {
        match provider {
            ProviderKind::DedicatedHeaded => self.dedicated_headed,
            ProviderKind::DedicatedHeadless => self.dedicated_headless,
            ProviderKind::SharedExtension => self.shared_extension,
            ProviderKind::DirectCdp => self.direct_cdp,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionSpec {
    pub mode: SessionMode,
    /// Required for shared-browser sessions; these targets are recorded as
    /// adopted and are never candidates for cleanup.
    #[serde(default)]
    pub selected_target_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionHandle {
    pub id: String,
    pub principal: String,
    pub mode: SessionMode,
    pub provider: ProviderKind,
    pub capability_revision: u64,
}

/// A target ID returned by a successful provider create call. Fields are
/// private so callers cannot claim ownership by supplying an arbitrary ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedTargetReceipt {
    session_id: String,
    browser_instance_id: u128,
    target_id: String,
}

impl CreatedTargetReceipt {
    pub(crate) fn from_provider(
        session_id: &str,
        browser_instance_id: u128,
        target_id: String,
    ) -> Self {
        Self {
            session_id: session_id.to_owned(),
            browser_instance_id,
            target_id,
        }
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    ProviderDenied(ProviderKind),
    TargetSelectionRequired,
    UnexpectedTargetSelection,
    OwnedTabRequiresCreationReceipt,
    UnknownSession,
    InternalClipboardTooLarge,
    ArtifactUnsupported,
    ArtifactInvalidFilename,
    ArtifactEmpty,
    ArtifactTooLarge,
    ArtifactStoreFull,
    ArtifactNotFound,
    ArtifactIo,
}

const MAX_INTERNAL_CLIPBOARD_BYTES: usize = 1_048_576;
const MAX_ARTIFACT_BYTES: usize = 10 * 1024 * 1024;
const MAX_SESSION_ARTIFACT_COUNT: usize = 16;
const MAX_SESSION_ARTIFACT_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArtifactHandle(String);

impl ArtifactHandle {
    pub fn opaque_id(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArtifactMetadata {
    pub handle: ArtifactHandle,
    pub filename: String,
    pub size: usize,
}

#[derive(Debug)]
struct StoredArtifact {
    metadata: ArtifactMetadata,
    path: std::path::PathBuf,
    directory: std::path::PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetRef {
    pub session_id: String,
    pub principal: String,
    pub capability_revision: u64,
    #[serde(with = "u128_string")]
    pub browser_instance_id: u128,
    pub browser_generation: u64,
    pub target_id: String,
    pub target_revision: String,
    pub frame_id: String,
    pub frame_revision: u64,
    pub account_revision: u64,
    pub document_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetIdentity {
    pub session_id: String,
    #[serde(with = "u128_string")]
    pub browser_instance_id: u128,
    pub browser_generation: u64,
    pub target_id: String,
    pub target_revision: String,
    pub frame_id: String,
    pub frame_revision: u64,
    pub account_revision: u64,
    pub document_revision: u64,
}

mod u128_string {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityRevisions {
    pub account: u64,
    pub document: u64,
}

/// Current CDP identity passed to an independent browser UI observer before
/// cleanup. The observer must inspect the tab's current in-page state; target
/// metadata alone is not sufficient to authorize closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupObservation {
    pub session_id: String,
    pub target_id: String,
    pub browser_instance_id: u128,
    pub browser_generation: u64,
    pub target_revision: String,
}

/// Trusted application-supplied observer for current in-tab changes (for
/// example an independent accessibility or screenshot observer). No observer
/// ships in this crate, so cleanup preserves owned targets by default.
pub trait IndependentTargetObserver: Send + Sync {
    fn verify_unchanged(&self, observation: &CleanupObservation) -> Result<(), String>;
}

impl From<&TargetRef> for TargetIdentity {
    fn from(reference: &TargetRef) -> Self {
        Self {
            session_id: reference.session_id.clone(),
            browser_instance_id: reference.browser_instance_id,
            browser_generation: reference.browser_generation,
            target_id: reference.target_id.clone(),
            target_revision: reference.target_revision.clone(),
            frame_id: reference.frame_id.clone(),
            frame_revision: reference.frame_revision,
            account_revision: reference.account_revision,
            document_revision: reference.document_revision,
        }
    }
}

impl TargetIdentity {
    pub fn from_snapshot(
        browser_instance_id: u128,
        session_id: &str,
        target: &TargetRecord,
        frame: &FrameRecord,
        account_revision: u64,
        document_revision: u64,
    ) -> Result<Self, StaleTarget> {
        if !target.attached || target.session_id.is_none() {
            return Err(StaleTarget::TargetChanged);
        }
        if target.generation != frame.generation || frame.target_id != target.id {
            return Err(StaleTarget::FrameChanged);
        }
        Ok(Self {
            session_id: session_id.to_owned(),
            browser_instance_id,
            browser_generation: target.generation,
            target_id: target.id.clone(),
            target_revision: target.revision.clone(),
            frame_id: frame.id.clone(),
            frame_revision: frame.revision,
            account_revision,
            document_revision,
        })
    }
}

impl TargetRef {
    pub fn capture(
        registry: &SessionRegistry,
        session: &SessionHandle,
        browser_instance_id: u128,
        target: &TargetRecord,
        frame: &FrameRecord,
        account_revision: u64,
        document_revision: u64,
    ) -> Result<Self, StaleTarget> {
        registry.verify_target_membership(session, browser_instance_id, &target.id)?;
        let identity = TargetIdentity::from_snapshot(
            browser_instance_id,
            &session.id,
            target,
            frame,
            account_revision,
            document_revision,
        )?;
        Ok(Self {
            session_id: session.id.clone(),
            principal: session.principal.clone(),
            capability_revision: session.capability_revision,
            browser_instance_id: identity.browser_instance_id,
            browser_generation: identity.browser_generation,
            target_id: identity.target_id,
            target_revision: identity.target_revision,
            frame_id: identity.frame_id,
            frame_revision: identity.frame_revision,
            account_revision,
            document_revision,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaleTarget {
    PrincipalChanged,
    SessionChanged,
    SessionEnded,
    CapabilityChanged,
    GrantRevoked,
    BrowserReconnected,
    BrowserProfileChanged,
    ProviderMismatch,
    TargetChanged,
    FrameChanged,
    AccountChanged,
    DocumentChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    Owned,
    Borrowed,
    Adopted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct TabRecord {
    target_id: String,
    ownership: Ownership,
    user_changed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreservedTab {
    pub target_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemainingTab {
    pub target_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CleanupReceipt {
    pub closed: Vec<String>,
    pub preserved: Vec<PreservedTab>,
    pub remaining: Vec<RemainingTab>,
    /// Opaque artifact handles whose private files or directories need retry.
    #[serde(default)]
    pub artifact_cleanup_pending: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SessionRegistry {
    grants: ProviderGrants,
    #[serde(default)]
    session_namespace: u64,
    next_session_id: u64,
    next_capability_revision: u64,
    sessions: BTreeMap<String, SessionHandle>,
    tabs: BTreeMap<String, BTreeMap<String, TabRecord>>,
    #[serde(default)]
    browser_instances: BTreeMap<String, u128>,
    #[serde(default)]
    inactive_sessions: BTreeSet<String>,
    /// Ephemeral task text; never serialized or copied to the OS pasteboard.
    #[serde(skip)]
    internal_clipboard_text: BTreeMap<String, String>,
    /// Private generated files backing opaque session handles; never serialized.
    #[serde(skip)]
    artifacts: BTreeMap<String, BTreeMap<String, StoredArtifact>>,
}

impl SessionRegistry {
    pub fn new(grants: ProviderGrants) -> Self {
        Self {
            grants,
            session_namespace: NEXT_REGISTRY_NAMESPACE.fetch_add(1, Ordering::Relaxed),
            next_session_id: 1,
            next_capability_revision: 1,
            sessions: BTreeMap::new(),
            tabs: BTreeMap::new(),
            browser_instances: BTreeMap::new(),
            inactive_sessions: BTreeSet::new(),
            internal_clipboard_text: BTreeMap::new(),
            artifacts: BTreeMap::new(),
        }
    }

    fn validate_clipboard_session(&self, session: &SessionHandle) -> Result<(), SessionError> {
        let Some(current) = self.sessions.get(&session.id) else {
            return Err(SessionError::UnknownSession);
        };
        if current != session
            || self.inactive_sessions.contains(&session.id)
            || !self.grants.allows(session.provider)
        {
            return Err(SessionError::ProviderDenied(session.provider));
        }
        Ok(())
    }

    pub fn put_artifact_bytes(
        &mut self,
        session: &SessionHandle,
        filename: &str,
        bytes: &[u8],
    ) -> Result<ArtifactMetadata, SessionError> {
        self.validate_clipboard_session(session)?;
        if !valid_artifact_filename(filename) {
            return Err(SessionError::ArtifactInvalidFilename);
        }
        if bytes.is_empty() {
            return Err(SessionError::ArtifactEmpty);
        }
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(SessionError::ArtifactTooLarge);
        }
        let current = self.artifacts.get(&session.id);
        if current.is_some_and(|items| {
            items.len() >= MAX_SESSION_ARTIFACT_COUNT
                || items
                    .values()
                    .map(|item| item.metadata.size)
                    .sum::<usize>()
                    .saturating_add(bytes.len())
                    > MAX_SESSION_ARTIFACT_BYTES
        }) {
            return Err(SessionError::ArtifactStoreFull);
        }
        #[cfg(not(unix))]
        {
            let _ = (filename, bytes);
            Err(SessionError::ArtifactUnsupported)
        }
        #[cfg(unix)]
        {
            let mut nonce = [0u8; 16];
            getrandom::fill(&mut nonce).map_err(|_| SessionError::ArtifactIo)?;
            let id = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
            let directory = std::env::temp_dir().join(format!("controlla-artifact-{id}"));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(|_| SessionError::ArtifactIo)?;
            let path = directory.join(filename);
            use std::io::Write;
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| {
                    let _ = std::fs::remove_dir(&directory);
                    SessionError::ArtifactIo
                })?;
            if file.write_all(bytes).is_err() {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_dir(&directory);
                return Err(SessionError::ArtifactIo);
            }
            let handle = ArtifactHandle(format!("artifact_{id}"));
            let metadata = ArtifactMetadata {
                handle: handle.clone(),
                filename: filename.to_owned(),
                size: bytes.len(),
            };
            self.artifacts
                .entry(session.id.clone())
                .or_default()
                .insert(
                    id,
                    StoredArtifact {
                        metadata: metadata.clone(),
                        path,
                        directory,
                    },
                );
            Ok(metadata)
        }
    }

    pub fn artifact_metadata(
        &self,
        session: &SessionHandle,
        handle: &ArtifactHandle,
    ) -> Result<ArtifactMetadata, SessionError> {
        self.validate_clipboard_session(session)?;
        self.artifacts
            .get(&session.id)
            .and_then(|items| items.values().find(|item| item.metadata.handle == *handle))
            .map(|item| item.metadata.clone())
            .ok_or(SessionError::ArtifactNotFound)
    }

    /// Read the bounded bytes behind a principal/session-scoped opaque handle.
    /// The caller must compare them with an expectation captured independently
    /// of this read; this only verifies the local staged artifact, not app state.
    pub fn read_artifact_bytes(
        &self,
        session: &SessionHandle,
        handle: &ArtifactHandle,
    ) -> Result<Vec<u8>, SessionError> {
        self.validate_clipboard_session(session)?;
        let item = self
            .artifacts
            .get(&session.id)
            .and_then(|items| items.values().find(|item| item.metadata.handle == *handle))
            .ok_or(SessionError::ArtifactNotFound)?;
        let mut file = std::fs::File::open(&item.path).map_err(|_| SessionError::ArtifactIo)?;
        let mut bytes = Vec::with_capacity(item.metadata.size);
        use std::io::Read;
        file.by_ref()
            .take((MAX_ARTIFACT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| SessionError::ArtifactIo)?;
        if bytes.is_empty() {
            return Err(SessionError::ArtifactEmpty);
        }
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(SessionError::ArtifactTooLarge);
        }
        Ok(bytes)
    }

    pub(crate) fn artifact_path_for(
        &self,
        session_id: &str,
        principal: &str,
        handle: &ArtifactHandle,
    ) -> Result<&std::path::Path, SessionError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or(SessionError::UnknownSession)?;
        if session.principal != principal {
            return Err(SessionError::ProviderDenied(session.provider));
        }
        self.validate_clipboard_session(session)?;
        self.artifacts
            .get(session_id)
            .and_then(|items| items.values().find(|item| item.metadata.handle == *handle))
            .map(|item| item.path.as_path())
            .ok_or(SessionError::ArtifactNotFound)
    }

    pub(crate) fn artifact_metadata_for(
        &self,
        session_id: &str,
        principal: &str,
        handle: &ArtifactHandle,
    ) -> Result<ArtifactMetadata, SessionError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or(SessionError::UnknownSession)?;
        if session.principal != principal {
            return Err(SessionError::ProviderDenied(session.provider));
        }
        self.validate_clipboard_session(session)?;
        self.artifacts
            .get(session_id)
            .and_then(|items| items.values().find(|item| item.metadata.handle == *handle))
            .map(|item| item.metadata.clone())
            .ok_or(SessionError::ArtifactNotFound)
    }

    fn clear_artifacts(&mut self, session_id: &str) -> Vec<String> {
        let Some(items) = self.artifacts.get_mut(session_id) else {
            return Vec::new();
        };
        let mut removed = Vec::new();
        let mut pending = Vec::new();
        for (key, item) in items.iter() {
            let file_removed = matches!(std::fs::remove_file(&item.path), Ok(()))
                || std::fs::metadata(&item.path)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
            let directory_removed = matches!(std::fs::remove_dir(&item.directory), Ok(()))
                || std::fs::metadata(&item.directory)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
            if file_removed && directory_removed {
                removed.push(key.clone());
            } else {
                pending.push(item.metadata.handle.opaque_id().to_owned());
            }
        }
        for key in removed {
            items.remove(&key);
        }
        if items.is_empty() {
            self.artifacts.remove(session_id);
        }
        pending
    }

    /// Store bounded plain text for this active session only. This buffer is
    /// in-memory and deliberately omitted from registry serialization.
    pub fn set_internal_clipboard_text(
        &mut self,
        session: &SessionHandle,
        text: impl Into<String>,
    ) -> Result<(), SessionError> {
        self.validate_clipboard_session(session)?;
        let text = text.into();
        if text.len() > MAX_INTERNAL_CLIPBOARD_BYTES {
            return Err(SessionError::InternalClipboardTooLarge);
        }
        self.internal_clipboard_text
            .insert(session.id.clone(), text);
        Ok(())
    }

    pub fn internal_clipboard_text(
        &self,
        session: &SessionHandle,
    ) -> Result<Option<&str>, SessionError> {
        self.validate_clipboard_session(session)?;
        Ok(self
            .internal_clipboard_text
            .get(&session.id)
            .map(String::as_str))
    }

    pub(crate) fn internal_clipboard_text_for(
        &self,
        session_id: &str,
        principal: &str,
    ) -> Result<Option<&str>, SessionError> {
        let Some(session) = self.sessions.get(session_id) else {
            return Err(SessionError::UnknownSession);
        };
        if session.principal != principal {
            return Err(SessionError::ProviderDenied(session.provider));
        }
        self.validate_clipboard_session(session)?;
        Ok(self
            .internal_clipboard_text
            .get(session_id)
            .map(String::as_str))
    }

    pub fn create_session(
        &mut self,
        spec: SessionSpec,
        principal: impl Into<String>,
    ) -> Result<SessionHandle, SessionError> {
        if matches!(spec.mode, SessionMode::Shared | SessionMode::DirectCdp)
            && spec.selected_target_ids.is_empty()
        {
            return Err(SessionError::TargetSelectionRequired);
        }
        if !matches!(spec.mode, SessionMode::Shared | SessionMode::DirectCdp)
            && !spec.selected_target_ids.is_empty()
        {
            return Err(SessionError::UnexpectedTargetSelection);
        }
        let provider = match spec.mode {
            SessionMode::Headed => ProviderKind::DedicatedHeaded,
            SessionMode::Headless => ProviderKind::DedicatedHeadless,
            SessionMode::Shared => ProviderKind::SharedExtension,
            SessionMode::DirectCdp => ProviderKind::DirectCdp,
        };
        if !self.grants.allows(provider) {
            return Err(SessionError::ProviderDenied(provider));
        }
        let handle = SessionHandle {
            id: format!(
                "session-{}-{}",
                self.session_namespace, self.next_session_id
            ),
            principal: principal.into(),
            mode: spec.mode,
            provider,
            capability_revision: self.next_capability_revision,
        };
        self.next_session_id = self.next_session_id.saturating_add(1);
        self.next_capability_revision = self.next_capability_revision.saturating_add(1);
        self.sessions.insert(handle.id.clone(), handle.clone());
        let tabs = self.tabs.entry(handle.id.clone()).or_default();
        if matches!(spec.mode, SessionMode::Shared | SessionMode::DirectCdp) {
            for target_id in spec.selected_target_ids {
                tabs.insert(
                    target_id.clone(),
                    TabRecord {
                        target_id,
                        ownership: Ownership::Adopted,
                        user_changed: false,
                    },
                );
            }
        }
        Ok(handle)
    }

    pub fn revoke_provider(&mut self, provider: ProviderKind) {
        match provider {
            ProviderKind::DedicatedHeaded => self.grants.dedicated_headed = false,
            ProviderKind::DedicatedHeadless => self.grants.dedicated_headless = false,
            ProviderKind::SharedExtension => self.grants.shared_extension = false,
            ProviderKind::DirectCdp => self.grants.direct_cdp = false,
        }
        self.next_capability_revision = self.next_capability_revision.saturating_add(1);
    }

    pub fn bind_session_to_browser(
        &mut self,
        session: &SessionHandle,
        browser_instance_id: u128,
    ) -> Result<(), StaleTarget> {
        self.bind_session_to_browser_if_unbound(session, browser_instance_id)
            .map(|_| ())
    }

    pub(crate) fn validate_session_browser_binding(
        &self,
        session: &SessionHandle,
        browser_instance_id: u128,
    ) -> Result<(), StaleTarget> {
        let current = self
            .sessions
            .get(&session.id)
            .ok_or(StaleTarget::SessionEnded)?;
        if current != session || self.inactive_sessions.contains(&session.id) {
            return Err(StaleTarget::SessionEnded);
        }
        if !self.grants.allows(session.provider) {
            return Err(StaleTarget::GrantRevoked);
        }
        if self
            .browser_instances
            .get(&session.id)
            .is_some_and(|bound| *bound != browser_instance_id)
        {
            return Err(StaleTarget::BrowserProfileChanged);
        }
        Ok(())
    }

    /// Bind the browser identity and report whether this call inserted it.
    /// Callers that perform an asynchronous provider operation can roll back
    /// only their own insertion if that operation fails.
    pub(crate) fn bind_session_to_browser_if_unbound(
        &mut self,
        session: &SessionHandle,
        browser_instance_id: u128,
    ) -> Result<bool, StaleTarget> {
        self.validate_session_browser_binding(session, browser_instance_id)?;
        match self.browser_instances.get(&session.id) {
            Some(bound) if *bound != browser_instance_id => Err(StaleTarget::BrowserProfileChanged),
            Some(_) => Ok(false),
            None => {
                self.browser_instances
                    .insert(session.id.clone(), browser_instance_id);
                Ok(true)
            }
        }
    }

    pub(crate) fn rollback_browser_binding_if_matches(
        &mut self,
        session_id: &str,
        browser_instance_id: u128,
    ) -> bool {
        if self.browser_instances.get(session_id) == Some(&browser_instance_id) {
            self.browser_instances.remove(session_id);
            true
        } else {
            false
        }
    }

    pub(crate) fn session_bound_to_browser(
        &self,
        session_id: &str,
        browser_instance_id: u128,
    ) -> bool {
        self.browser_instances.get(session_id) == Some(&browser_instance_id)
    }

    fn verify_target_membership(
        &self,
        session: &SessionHandle,
        browser_instance_id: u128,
        target_id: &str,
    ) -> Result<(), StaleTarget> {
        let current = self
            .sessions
            .get(&session.id)
            .ok_or(StaleTarget::SessionEnded)?;
        if current != session || self.inactive_sessions.contains(&session.id) {
            return Err(StaleTarget::SessionEnded);
        }
        if !self.grants.allows(session.provider) {
            return Err(StaleTarget::GrantRevoked);
        }
        if self.browser_instances.get(&session.id) != Some(&browser_instance_id) {
            return Err(StaleTarget::BrowserProfileChanged);
        }
        if !self
            .tabs
            .get(&session.id)
            .is_some_and(|tabs| tabs.contains_key(target_id))
        {
            return Err(StaleTarget::TargetChanged);
        }
        Ok(())
    }

    pub fn authorize_direct_cdp(&self, session_id: &str) -> Result<(), StaleTarget> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or(StaleTarget::SessionEnded)?;
        if matches!(session.provider, ProviderKind::SharedExtension) {
            return Err(StaleTarget::ProviderMismatch);
        }
        if !self.grants.allows(session.provider) {
            return Err(StaleTarget::GrantRevoked);
        }
        Ok(())
    }

    pub fn contains_target(&self, session: &SessionHandle, target_id: &str) -> bool {
        self.sessions.get(&session.id) == Some(session)
            && self
                .tabs
                .get(&session.id)
                .is_some_and(|tabs| tabs.contains_key(target_id))
    }

    pub(crate) fn selected_targets(&self, session_id: &str) -> Option<BTreeSet<String>> {
        self.tabs
            .get(session_id)
            .map(|tabs| tabs.keys().cloned().collect())
    }

    pub(crate) fn authorize_target_creation(
        &self,
        session: &SessionHandle,
    ) -> Result<(), SessionError> {
        let Some(current) = self.sessions.get(&session.id) else {
            return Err(SessionError::UnknownSession);
        };
        if current != session
            || self.inactive_sessions.contains(&session.id)
            || !self.grants.allows(session.provider)
        {
            return Err(SessionError::ProviderDenied(session.provider));
        }
        if session.provider == ProviderKind::SharedExtension {
            return Err(SessionError::ProviderDenied(session.provider));
        }
        Ok(())
    }

    pub(crate) fn authorize_shared_extension(
        &self,
        session: &SessionHandle,
    ) -> Result<(), SessionError> {
        let Some(current) = self.sessions.get(&session.id) else {
            return Err(SessionError::UnknownSession);
        };
        if current != session
            || self.inactive_sessions.contains(&session.id)
            || !self.grants.allows(ProviderKind::SharedExtension)
            || session.provider != ProviderKind::SharedExtension
        {
            return Err(SessionError::ProviderDenied(ProviderKind::SharedExtension));
        }
        Ok(())
    }

    pub(crate) fn authorize_shared_target(
        &self,
        session: &SessionHandle,
        browser_instance_id: u128,
        target_id: &str,
    ) -> Result<(), StaleTarget> {
        if session.provider != ProviderKind::SharedExtension {
            return Err(StaleTarget::ProviderMismatch);
        }
        self.verify_target_membership(session, browser_instance_id, target_id)
    }

    pub fn register_tab(
        &mut self,
        session_id: &str,
        target_id: impl Into<String>,
        ownership: Ownership,
    ) -> Result<(), SessionError> {
        if ownership == Ownership::Owned {
            return Err(SessionError::OwnedTabRequiresCreationReceipt);
        }
        let tabs = self
            .tabs
            .get_mut(session_id)
            .ok_or(SessionError::UnknownSession)?;
        let target_id = target_id.into();
        tabs.insert(
            target_id.clone(),
            TabRecord {
                target_id,
                ownership,
                user_changed: false,
            },
        );
        Ok(())
    }

    /// Record ownership from a provider-created target receipt.
    pub fn record_created_tab(
        &mut self,
        receipt: CreatedTargetReceipt,
    ) -> Result<(), SessionError> {
        let session_id = receipt.session_id;
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(SessionError::UnknownSession)?;
        if self.inactive_sessions.contains(&session_id)
            || !self.grants.allows(session.provider)
            || self.browser_instances.get(&session_id) != Some(&receipt.browser_instance_id)
        {
            return Err(SessionError::ProviderDenied(session.provider));
        }
        let tabs = self
            .tabs
            .get_mut(&session_id)
            .ok_or(SessionError::UnknownSession)?;
        let target_id = receipt.target_id;
        tabs.insert(
            target_id.clone(),
            TabRecord {
                target_id,
                ownership: Ownership::Owned,
                user_changed: false,
            },
        );
        Ok(())
    }

    pub fn mark_user_changed(
        &mut self,
        session_id: &str,
        target_id: &str,
    ) -> Result<(), SessionError> {
        let record = self
            .tabs
            .get_mut(session_id)
            .and_then(|tabs| tabs.get_mut(target_id))
            .ok_or(SessionError::UnknownSession)?;
        record.user_changed = true;
        Ok(())
    }

    pub(crate) fn release_session(
        &mut self,
        session_id: &str,
        mut close_tab: impl FnMut(&str) -> Result<(), String>,
    ) -> CleanupReceipt {
        let mut receipt = CleanupReceipt {
            artifact_cleanup_pending: self.clear_artifacts(session_id),
            ..CleanupReceipt::default()
        };
        let Some(tabs) = self.tabs.get(session_id).cloned() else {
            self.internal_clipboard_text.remove(session_id);
            if !receipt.artifact_cleanup_pending.is_empty() {
                self.inactive_sessions.insert(session_id.to_owned());
            } else if self.sessions.remove(session_id).is_none() {
                receipt.remaining.push(RemainingTab {
                    target_id: session_id.to_owned(),
                    reason: "unknown session; no cleanup was attempted".to_owned(),
                });
            } else {
                self.inactive_sessions.remove(session_id);
                self.browser_instances.remove(session_id);
            }
            return receipt;
        };
        self.inactive_sessions.insert(session_id.to_owned());
        self.internal_clipboard_text.remove(session_id);
        let mut retry = BTreeMap::new();
        for record in tabs.into_values() {
            if record.ownership == Ownership::Owned && !record.user_changed {
                match close_tab(&record.target_id) {
                    Ok(()) => receipt.closed.push(record.target_id),
                    Err(reason) => {
                        receipt.remaining.push(RemainingTab {
                            target_id: record.target_id.clone(),
                            reason,
                        });
                        retry.insert(record.target_id.clone(), record);
                    }
                }
            } else {
                receipt.preserved.push(PreservedTab {
                    target_id: record.target_id,
                    reason: if record.user_changed {
                        "tab changed by the user; ownership no longer permits cleanup".to_owned()
                    } else {
                        format!("tab is {:?}, not owned by this session", record.ownership)
                    },
                });
            }
        }
        if retry.is_empty() && receipt.artifact_cleanup_pending.is_empty() {
            self.tabs.remove(session_id);
            self.sessions.remove(session_id);
            self.inactive_sessions.remove(session_id);
            self.browser_instances.remove(session_id);
        } else if retry.is_empty() {
            self.tabs.remove(session_id);
            self.inactive_sessions.insert(session_id.to_owned());
        } else {
            self.tabs.insert(session_id.to_owned(), retry);
        }
        receipt
    }

    pub(crate) fn owned_cleanup_candidates(&self, session_id: &str) -> Vec<String> {
        self.tabs
            .get(session_id)
            .into_iter()
            .flat_map(|tabs| tabs.values())
            .filter(|record| record.ownership == Ownership::Owned && !record.user_changed)
            .map(|record| record.target_id.clone())
            .collect()
    }

    pub fn reconcile_after_crash(&self, live_target_ids: &[&str]) -> CleanupReceipt {
        let live = live_target_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let mut receipt = CleanupReceipt::default();
        for tabs in self.tabs.values() {
            for record in tabs.values() {
                if !live.contains(record.target_id.as_str()) {
                    continue;
                }
                if record.ownership == Ownership::Owned && !record.user_changed {
                    receipt.remaining.push(RemainingTab {
                        target_id: record.target_id.clone(),
                        reason: "owned tab survived process loss; explicit reconciliation required"
                            .to_owned(),
                    });
                } else {
                    receipt.preserved.push(PreservedTab {
                        target_id: record.target_id.clone(),
                        reason: "user-owned, adopted, borrowed, or user-changed tab preserved after process loss".to_owned(),
                    });
                }
            }
        }
        receipt
    }

    pub fn resolve_target(
        expected: &TargetRef,
        principal: &str,
        current: &TargetIdentity,
    ) -> Result<TargetIdentity, StaleTarget> {
        if principal != expected.principal {
            return Err(StaleTarget::PrincipalChanged);
        }
        if expected.session_id != current.session_id {
            return Err(StaleTarget::SessionChanged);
        }
        if expected.browser_generation != current.browser_generation {
            return Err(StaleTarget::BrowserReconnected);
        }
        if expected.browser_instance_id != current.browser_instance_id {
            return Err(StaleTarget::BrowserProfileChanged);
        }
        if expected.target_id != current.target_id
            || expected.target_revision != current.target_revision
        {
            return Err(StaleTarget::TargetChanged);
        }
        if expected.frame_id != current.frame_id
            || expected.frame_revision != current.frame_revision
        {
            return Err(StaleTarget::FrameChanged);
        }
        if expected.account_revision != current.account_revision {
            return Err(StaleTarget::AccountChanged);
        }
        if expected.document_revision != current.document_revision {
            return Err(StaleTarget::DocumentChanged);
        }
        Ok(current.clone())
    }

    /// Resolve against the registry's live session and provider grant before
    /// accepting the caller's current target snapshot.
    pub fn resolve_active_target(
        &self,
        expected: &TargetRef,
        principal: &str,
        current: &TargetIdentity,
    ) -> Result<TargetIdentity, StaleTarget> {
        let session = self
            .sessions
            .get(&expected.session_id)
            .ok_or(StaleTarget::SessionEnded)?;
        if self.inactive_sessions.contains(&expected.session_id) {
            return Err(StaleTarget::SessionEnded);
        }
        if principal != session.principal || principal != expected.principal {
            return Err(StaleTarget::PrincipalChanged);
        }
        if expected.capability_revision != session.capability_revision {
            return Err(StaleTarget::CapabilityChanged);
        }
        if !self.grants.allows(session.provider) {
            return Err(StaleTarget::GrantRevoked);
        }
        if self.browser_instances.get(&expected.session_id) != Some(&current.browser_instance_id) {
            return Err(StaleTarget::BrowserProfileChanged);
        }
        if !self
            .tabs
            .get(&expected.session_id)
            .is_some_and(|tabs| tabs.contains_key(&expected.target_id))
        {
            return Err(StaleTarget::TargetChanged);
        }
        Self::resolve_target(expected, principal, current)
    }
}

impl Clone for SessionRegistry {
    fn clone(&self) -> Self {
        Self {
            grants: self.grants,
            session_namespace: self.session_namespace,
            next_session_id: self.next_session_id,
            next_capability_revision: self.next_capability_revision,
            sessions: self.sessions.clone(),
            tabs: self.tabs.clone(),
            browser_instances: self.browser_instances.clone(),
            inactive_sessions: self.inactive_sessions.clone(),
            internal_clipboard_text: BTreeMap::new(),
            artifacts: BTreeMap::new(),
        }
    }
}

impl Drop for SessionRegistry {
    fn drop(&mut self) {
        for items in self.artifacts.values() {
            for item in items.values() {
                let _ = std::fs::remove_file(&item.path);
                let _ = std::fs::remove_dir(&item.directory);
            }
        }
    }
}

fn valid_artifact_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 255
        && filename != "."
        && filename != ".."
        && !filename.contains(['/', '\\', '\0'])
        && !filename.chars().any(char::is_control)
}

#[cfg(test)]
mod direct_cdp_grant_tests {
    use super::*;

    #[test]
    fn direct_cdp_requires_opt_in_and_remains_distinct_from_shared_extension() {
        let spec = SessionSpec {
            mode: SessionMode::DirectCdp,
            selected_target_ids: vec!["tab-1".into()],
        };
        let mut denied = SessionRegistry::new(ProviderGrants::default());
        assert!(denied.create_session(spec.clone(), "local").is_err());

        let mut allowed = SessionRegistry::new(ProviderGrants {
            direct_cdp: true,
            ..Default::default()
        });
        let handle = allowed.create_session(spec, "local").unwrap();
        assert_eq!(handle.provider, ProviderKind::DirectCdp);
        allowed.bind_session_to_browser(&handle, 17).unwrap();
        assert!(allowed.authorize_direct_cdp(&handle.id).is_ok());

        let mut extension = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..Default::default()
        });
        let ext = extension
            .create_session(
                SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["tab-1".into()],
                },
                "local",
            )
            .unwrap();
        assert_eq!(
            extension.authorize_direct_cdp(&ext.id),
            Err(StaleTarget::ProviderMismatch)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn artifacts_are_opaque_bounded_principal_scoped_ephemeral_and_released() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..Default::default()
        });
        let make = |registry: &mut SessionRegistry, who| {
            registry
                .create_session(
                    SessionSpec {
                        mode: SessionMode::Headless,
                        selected_target_ids: vec![],
                    },
                    who,
                )
                .unwrap()
        };
        let alice = make(&mut registry, "alice");
        let bob = make(&mut registry, "bob");
        let artifact = registry
            .put_artifact_bytes(&alice, "payload.txt", b"hello")
            .unwrap();
        assert_eq!(artifact.size, 5);
        assert_eq!(
            registry
                .read_artifact_bytes(&alice, &artifact.handle)
                .unwrap(),
            b"hello"
        );
        assert_eq!(
            registry
                .artifact_metadata(&alice, &artifact.handle)
                .unwrap(),
            artifact
        );
        assert_eq!(
            registry.artifact_metadata(&bob, &artifact.handle),
            Err(SessionError::ArtifactNotFound)
        );
        assert_eq!(
            registry.artifact_path_for(&alice.id, "mallory", &artifact.handle),
            Err(SessionError::ProviderDenied(alice.provider))
        );
        assert_eq!(
            registry.put_artifact_bytes(&alice, "../escape", b"x"),
            Err(SessionError::ArtifactInvalidFilename)
        );
        assert_eq!(
            registry.put_artifact_bytes(&alice, "x\\y", b"x"),
            Err(SessionError::ArtifactInvalidFilename)
        );
        assert_eq!(
            registry.put_artifact_bytes(&alice, "", b"x"),
            Err(SessionError::ArtifactInvalidFilename)
        );
        assert_eq!(
            registry.put_artifact_bytes(&alice, "empty", b""),
            Err(SessionError::ArtifactEmpty)
        );
        assert_eq!(
            registry.put_artifact_bytes(&alice, "large", &vec![0; MAX_ARTIFACT_BYTES + 1]),
            Err(SessionError::ArtifactTooLarge)
        );
        for index in 0..(MAX_SESSION_ARTIFACT_COUNT - 1) {
            registry
                .put_artifact_bytes(&alice, &format!("extra-{index}"), b"x")
                .unwrap();
        }
        assert_eq!(
            registry.put_artifact_bytes(&alice, "too-many", b"x"),
            Err(SessionError::ArtifactStoreFull)
        );
        let private_path = registry
            .artifact_path_for(&alice.id, "alice", &artifact.handle)
            .unwrap()
            .to_path_buf();
        assert!(private_path.exists());
        assert_eq!(private_path.file_name().unwrap(), "payload.txt");
        assert_eq!(
            std::fs::metadata(&private_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let serialized = serde_json::to_string(&registry).unwrap();
        assert!(!serialized.contains(artifact.handle.opaque_id()));
        assert!(!serialized.contains(&private_path.display().to_string()));
        assert!(!serialized.contains("hello"));
        let recovered: SessionRegistry = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            recovered.artifact_metadata(&alice, &artifact.handle),
            Err(SessionError::ArtifactNotFound)
        );
        registry.release_session(&alice.id, |_| Ok(()));
        assert!(!private_path.exists());
        assert!(!private_path.parent().unwrap().exists());
        assert_eq!(
            registry.artifact_metadata(&alice, &artifact.handle),
            Err(SessionError::UnknownSession)
        );
    }

    #[cfg(unix)]
    #[test]
    fn artifact_cleanup_failure_is_reported_retained_and_retryable() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..Default::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: vec![],
                },
                "alice",
            )
            .unwrap();
        let artifact = registry
            .put_artifact_bytes(&session, "retry.txt", b"retry me")
            .unwrap();
        let path = registry
            .artifact_path_for(&session.id, "alice", &artifact.handle)
            .unwrap()
            .to_path_buf();
        let parent = path.parent().unwrap().to_path_buf();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("blocker"), b"keep cleanup from succeeding").unwrap();

        let failed = registry.release_session(&session.id, |_| Ok(()));
        assert_eq!(
            failed.artifact_cleanup_pending,
            vec![artifact.handle.opaque_id().to_owned()]
        );
        assert!(
            registry
                .artifacts
                .get(&session.id)
                .unwrap()
                .contains_key(artifact.handle.opaque_id().trim_start_matches("artifact_"))
        );

        std::fs::remove_file(path.join("blocker")).unwrap();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"retry me").unwrap();
        let retried = registry.release_session(&session.id, |_| Ok(()));
        assert!(retried.artifact_cleanup_pending.is_empty());
        assert!(
            retried.remaining.is_empty(),
            "artifact-only retry reported a missing session as a leftover tab"
        );
        assert!(!parent.exists());
    }

    #[cfg(unix)]
    #[test]
    fn dropping_registry_cleans_artifact_files() {
        let path = {
            let mut registry = SessionRegistry::new(ProviderGrants {
                dedicated_headless: true,
                ..Default::default()
            });
            let session = registry
                .create_session(
                    SessionSpec {
                        mode: SessionMode::Headless,
                        selected_target_ids: vec![],
                    },
                    "alice",
                )
                .unwrap();
            let artifact = registry
                .put_artifact_bytes(&session, "drop.txt", b"drop me")
                .unwrap();
            registry
                .artifact_path_for(&session.id, "alice", &artifact.handle)
                .unwrap()
                .to_path_buf()
        };
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn cloned_registry_does_not_own_or_delete_source_artifacts() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..Default::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: vec![],
                },
                "alice",
            )
            .unwrap();
        let artifact = registry
            .put_artifact_bytes(&session, "clone.txt", b"owned once")
            .unwrap();
        let path = registry
            .artifact_path_for(&session.id, "alice", &artifact.handle)
            .unwrap()
            .to_path_buf();
        let clone = registry.clone();
        assert!(clone.artifacts.is_empty());
        drop(clone);
        assert!(path.exists());
        let receipt = registry.release_session(&session.id, |_| Ok(()));
        assert!(receipt.artifact_cleanup_pending.is_empty());
        assert!(!path.exists());
    }

    fn add_owned_fixture(registry: &mut SessionRegistry, session_id: &str, target_id: &str) {
        registry
            .tabs
            .entry(session_id.to_owned())
            .or_default()
            .insert(
                target_id.to_owned(),
                TabRecord {
                    target_id: target_id.to_owned(),
                    ownership: Ownership::Owned,
                    user_changed: false,
                },
            );
    }

    fn target(id: &str) -> TargetRecord {
        TargetRecord {
            id: id.to_owned(),
            target_type: "page".to_owned(),
            browser_context_id: Some("profile-context".to_owned()),
            session_id: Some(format!("cdp-{id}")),
            url: Some("https://fixture.test/".to_owned()),
            title: None,
            opener_id: None,
            attached: true,
            generation: 1,
            revision: "target-r1".to_owned(),
        }
    }

    fn frame(target_id: &str) -> FrameRecord {
        FrameRecord {
            id: format!("frame-{target_id}"),
            parent_id: None,
            target_id: target_id.to_owned(),
            loader_id: Some("loader-1".to_owned()),
            execution_context_ids: vec![1],
            generation: 1,
            revision: 1,
        }
    }

    #[test]
    fn capture_rejects_target_outside_session_membership() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headed: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "alice",
            )
            .unwrap();
        registry.bind_session_to_browser(&session, 10).unwrap();
        assert_eq!(
            TargetRef::capture(
                &registry,
                &session,
                10,
                &target("unlisted"),
                &frame("unlisted"),
                1,
                1
            ),
            Err(StaleTarget::TargetChanged)
        );
    }

    #[test]
    fn browser_binding_rollback_removes_only_the_new_matching_insertion() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headed: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "alice",
            )
            .unwrap();
        assert!(
            registry
                .bind_session_to_browser_if_unbound(&session, 10)
                .unwrap()
        );
        assert!(registry.rollback_browser_binding_if_matches(&session.id, 10));
        assert!(!registry.session_bound_to_browser(&session.id, 10));

        registry.bind_session_to_browser(&session, 11).unwrap();
        assert!(
            !registry
                .bind_session_to_browser_if_unbound(&session, 11)
                .unwrap()
        );
        assert!(!registry.rollback_browser_binding_if_matches(&session.id, 10));
        assert!(registry.session_bound_to_browser(&session.id, 11));
    }

    #[test]
    fn refs_are_bound_to_browser_instance_and_allowed_target() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headed: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "alice",
            )
            .unwrap();
        registry.bind_session_to_browser(&session, 10).unwrap();
        registry
            .register_tab(&session.id, "allowed", Ownership::Borrowed)
            .unwrap();
        let reference = TargetRef::capture(
            &registry,
            &session,
            10,
            &target("allowed"),
            &frame("allowed"),
            1,
            1,
        )
        .unwrap();
        let current = TargetIdentity::from_snapshot(
            10,
            &session.id,
            &target("allowed"),
            &frame("allowed"),
            1,
            1,
        )
        .unwrap();
        assert!(
            registry
                .resolve_active_target(&reference, "alice", &current)
                .is_ok()
        );
        assert_eq!(
            registry.resolve_active_target(
                &reference,
                "alice",
                &TargetIdentity::from_snapshot(
                    11,
                    &session.id,
                    &target("allowed"),
                    &frame("allowed"),
                    1,
                    1
                )
                .unwrap()
            ),
            Err(StaleTarget::BrowserProfileChanged)
        );
    }

    #[test]
    fn persisted_browser_session_binding_rejects_new_instance() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headed: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "alice",
            )
            .unwrap();
        registry.bind_session_to_browser(&session, 10).unwrap();
        let persisted = serde_json::to_vec(&registry).unwrap();
        let mut recovered: SessionRegistry = serde_json::from_slice(&persisted).unwrap();
        assert_eq!(
            recovered.bind_session_to_browser(&session, 11),
            Err(StaleTarget::BrowserProfileChanged)
        );
    }

    #[test]
    fn release_closes_only_unchanged_owned_tabs_and_preserves_user_tabs() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headed: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "alice",
            )
            .unwrap();
        add_owned_fixture(&mut registry, &session.id, "owned-1");
        add_owned_fixture(&mut registry, &session.id, "owned-2");
        registry
            .register_tab(&session.id, "borrowed", Ownership::Borrowed)
            .unwrap();
        registry.mark_user_changed(&session.id, "owned-1").unwrap();
        let mut closed = Vec::new();
        let receipt = registry.release_session(&session.id, |target| {
            closed.push(target.to_owned());
            Ok::<_, String>(())
        });
        assert_eq!(closed, vec!["owned-2"]);
        assert_eq!(receipt.preserved.len(), 2);
        assert!(receipt.remaining.is_empty());
    }

    #[test]
    fn crash_reconciliation_reports_owned_leftovers_and_preserves_user_tabs() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            shared_extension: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Shared,
                    selected_target_ids: vec!["adopted".into()],
                },
                "alice",
            )
            .unwrap();
        add_owned_fixture(&mut registry, &session.id, "owned");
        let serialized = serde_json::to_vec(&registry).unwrap();
        let recovered: SessionRegistry = serde_json::from_slice(&serialized).unwrap();
        let receipt = recovered.reconcile_after_crash(&["owned", "adopted"]);
        assert_eq!(
            receipt
                .remaining
                .iter()
                .map(|item| item.target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["owned"]
        );
        assert_eq!(
            receipt
                .preserved
                .iter()
                .map(|item| item.target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["adopted"]
        );
    }

    #[test]
    fn internal_clipboard_is_plain_text_session_scoped_ephemeral_and_principal_bound() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let make_session = |registry: &mut SessionRegistry, principal| {
            registry
                .create_session(
                    SessionSpec {
                        mode: SessionMode::Headless,
                        selected_target_ids: vec![],
                    },
                    principal,
                )
                .unwrap()
        };
        let first = make_session(&mut registry, "alice");
        let second = make_session(&mut registry, "bob");
        registry
            .set_internal_clipboard_text(&first, "session-one text")
            .unwrap();
        registry
            .set_internal_clipboard_text(&second, "session-two text")
            .unwrap();
        assert_eq!(
            registry.internal_clipboard_text(&first).unwrap(),
            Some("session-one text")
        );
        assert_eq!(
            registry.internal_clipboard_text(&second).unwrap(),
            Some("session-two text")
        );
        assert_eq!(
            registry.set_internal_clipboard_text(&first, "x".repeat(1_048_577)),
            Err(SessionError::InternalClipboardTooLarge)
        );
        assert_eq!(
            registry.internal_clipboard_text(&first).unwrap(),
            Some("session-one text")
        );

        let mut spoofed = first.clone();
        spoofed.principal = "mallory".into();
        assert!(registry.internal_clipboard_text(&spoofed).is_err());

        let serialized = serde_json::to_string(&registry).unwrap();
        assert!(!serialized.contains("session-one text"));
        let recovered: SessionRegistry = serde_json::from_str(&serialized).unwrap();
        assert_eq!(recovered.internal_clipboard_text(&first).unwrap(), None);

        registry.release_session(&first.id, |_| Ok(()));
        assert!(registry.internal_clipboard_text(&first).is_err());
        assert_eq!(
            registry.internal_clipboard_text(&second).unwrap(),
            Some("session-two text")
        );
    }

    #[test]
    fn failed_owned_close_remains_available_for_retry() {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headed: true,
            ..ProviderGrants::default()
        });
        let session = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headed,
                    selected_target_ids: Vec::new(),
                },
                "alice",
            )
            .unwrap();
        add_owned_fixture(&mut registry, &session.id, "owned");
        let failed =
            registry.release_session(&session.id, |_| Err("provider disconnected".to_owned()));
        assert_eq!(failed.remaining[0].target_id, "owned");
        let retried = registry.release_session(&session.id, |_| Ok(()));
        assert_eq!(retried.closed, vec!["owned"]);
        assert!(retried.remaining.is_empty());
    }
}
