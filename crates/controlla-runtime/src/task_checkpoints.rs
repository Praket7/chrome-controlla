//! Small durable checkpoints for resumable V3 tasks.
//!
//! The store contains only digests and completed target identities. It never
//! stores page text, user values, credentials, or mutation receipts. A resumed
//! target is reused only after the live tab still reports the exact expected
//! URL; extraction is then repeated against a fresh document identity. The
//! generic postcondition API records identities only: callers must independently
//! verify current application state before marking a postcondition complete.
//! It does not make unknown mutations safe to replay or prove persistence.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    path::PathBuf,
};

const MAX_CHECKPOINTS: usize = 128;
const MAX_STORE_BYTES: usize = 256 * 1024;
const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const MAX_POSTCONDITIONS: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    task_digest: String,
    completed_targets: BTreeMap<String, String>,
    #[serde(default)]
    completed_postconditions: BTreeMap<String, String>,
    /// Write-ahead intents for mutations whose outcome may not yet have been
    /// observed. Keys bind task, step, target, document and predicate digests.
    #[serde(default)]
    pending_mutations: BTreeMap<String, String>,
    updated_at_ms: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCheckpoints {
    version: u32,
    entries: BTreeMap<String, Checkpoint>,
}

#[derive(Default)]
pub struct TaskCheckpointStore {
    path: Option<PathBuf>,
    entries: BTreeMap<String, Checkpoint>,
}

#[path = "checkpoint_worker.rs"]
mod worker;
pub use worker::CheckpointWorker;

impl TaskCheckpointStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let mut store = Self {
            path: Some(path.clone()),
            entries: BTreeMap::new(),
        };
        let _lock = store.lock_cross_process()?;
        store.refresh_from_disk()?;
        Ok(store)
    }

    fn lock_cross_process(&self) -> Result<Option<File>, String> {
        let Some(path) = self.path.as_ref() else {
            return Ok(None);
        };
        let parent = path.parent().ok_or("task checkpoint path has no parent")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let lock_path = path.with_extension("lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(lock_path).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        file.lock().map_err(|error| error.to_string())?;
        Ok(Some(file))
    }

    fn refresh_from_disk(&mut self) -> Result<(), String> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        match std::fs::read(path) {
            Ok(bytes) => {
                if bytes.len() > MAX_STORE_BYTES {
                    return Err("task checkpoint store exceeds its byte limit".into());
                }
                let stored: StoredCheckpoints =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if !matches!(stored.version, 2 | 3)
                    || stored.entries.len() > MAX_CHECKPOINTS
                    || stored.entries.iter().any(|(key, checkpoint)| {
                        key.len() != 64
                            || !is_sha256(&checkpoint.task_digest)
                            || checkpoint.completed_targets.len() > 32
                            || checkpoint.completed_postconditions.len() > MAX_POSTCONDITIONS
                            || checkpoint.pending_mutations.len() > MAX_POSTCONDITIONS
                            || checkpoint
                                .completed_targets
                                .iter()
                                .any(|(target, identity)| {
                                    !is_sha256(target) || !is_sha256(identity)
                                })
                            || checkpoint.completed_postconditions.iter().any(
                                |(identity, postcondition)| {
                                    !is_sha256(identity) || !is_sha256(postcondition)
                                },
                            )
                            || checkpoint
                                .pending_mutations
                                .iter()
                                .any(|(identity, predicate)| {
                                    !is_sha256(identity) || !is_sha256(predicate)
                                })
                    })
                {
                    return Err("task checkpoint store failed schema or bounds validation".into());
                }
                self.entries = stored.entries;
                self.prune(now_ms());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.entries.clear();
            }
            Err(error) => return Err(error.to_string()),
        }
        Ok(())
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn target_completed(
        &mut self,
        key: &str,
        task_digest: &str,
        target: &str,
        target_identity: &str,
    ) -> bool {
        let _lock = match self.lock_cross_process() {
            Ok(lock) => lock,
            Err(_) => return false,
        };
        if self.refresh_from_disk().is_err() {
            return false;
        }
        let checkpoint_key = digest(key.as_bytes());
        let target_digest = digest(target.as_bytes());
        let now = now_ms();
        let Some(checkpoint) = self.entries.get_mut(&checkpoint_key) else {
            return false;
        };
        if checkpoint.task_digest != task_digest {
            return false;
        }
        if now.saturating_sub(checkpoint.updated_at_ms) > MAX_AGE_MS {
            self.entries.remove(&checkpoint_key);
            let _ = self.persist();
            return false;
        }
        checkpoint.updated_at_ms = now;
        checkpoint
            .completed_targets
            .get(&target_digest)
            .is_some_and(|identity| identity == &digest(target_identity.as_bytes()))
    }

    pub fn bind(&mut self, key: &str, task_digest: &str) -> Result<(), String> {
        let _lock = self.lock_cross_process()?;
        self.refresh_from_disk()?;
        if !is_sha256(task_digest) {
            return Err("task checkpoint digest is invalid".into());
        }
        let checkpoint_key = digest(key.as_bytes());
        if let Some(existing) = self.entries.get(&checkpoint_key) {
            if existing.task_digest != task_digest {
                return Err("resume key is already bound to a different task".into());
            }
            return Ok(());
        }
        self.prune(now_ms());
        if self.entries.len() >= MAX_CHECKPOINTS
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.updated_at_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(
            checkpoint_key,
            Checkpoint {
                task_digest: task_digest.to_owned(),
                completed_targets: BTreeMap::new(),
                completed_postconditions: BTreeMap::new(),
                pending_mutations: BTreeMap::new(),
                updated_at_ms: now_ms(),
            },
        );
        self.persist()
    }

    pub fn mark_target_completed(
        &mut self,
        key: &str,
        task_digest: &str,
        target: &str,
        target_identity: &str,
    ) -> Result<(), String> {
        let _lock = self.lock_cross_process()?;
        self.refresh_from_disk()?;
        if !is_sha256(task_digest) {
            return Err("task checkpoint digest is invalid".into());
        }
        let checkpoint_key = digest(key.as_bytes());
        self.prune(now_ms());
        if !self.entries.contains_key(&checkpoint_key)
            && self.entries.len() >= MAX_CHECKPOINTS
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.updated_at_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        let checkpoint = self
            .entries
            .entry(checkpoint_key)
            .or_insert_with(|| Checkpoint {
                task_digest: task_digest.to_owned(),
                completed_targets: BTreeMap::new(),
                completed_postconditions: BTreeMap::new(),
                pending_mutations: BTreeMap::new(),
                updated_at_ms: now_ms(),
            });
        if checkpoint.task_digest != task_digest {
            return Err("resume key is already bound to a different task".into());
        }
        checkpoint.completed_targets.insert(
            digest(target.as_bytes()),
            digest(target_identity.as_bytes()),
        );
        checkpoint.updated_at_ms = now_ms();
        self.persist()
    }

    /// Bind a resumable single-document workflow to its first observed target.
    /// Returns false if the same resume key is later presented with a different
    /// document identity; it never replaces the original binding.
    pub fn bind_target_identity(
        &mut self,
        key: &str,
        task_digest: &str,
        target: &str,
        target_identity: &str,
    ) -> Result<bool, String> {
        let _lock = self.lock_cross_process()?;
        self.refresh_from_disk()?;
        if !is_sha256(task_digest) {
            return Err("task checkpoint digest is invalid".into());
        }
        let checkpoint_key = digest(key.as_bytes());
        self.prune(now_ms());
        if !self.entries.contains_key(&checkpoint_key)
            && self.entries.len() >= MAX_CHECKPOINTS
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.updated_at_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        let checkpoint = self
            .entries
            .entry(checkpoint_key)
            .or_insert_with(|| Checkpoint {
                task_digest: task_digest.to_owned(),
                completed_targets: BTreeMap::new(),
                completed_postconditions: BTreeMap::new(),
                pending_mutations: BTreeMap::new(),
                updated_at_ms: now_ms(),
            });
        if checkpoint.task_digest != task_digest {
            return Err("resume key is already bound to a different task".into());
        }
        let target_digest = digest(target.as_bytes());
        let identity_digest = digest(target_identity.as_bytes());
        if let Some(existing) = checkpoint.completed_targets.get(&target_digest) {
            if now_ms().saturating_sub(checkpoint.updated_at_ms) > MAX_AGE_MS
                && (!checkpoint.pending_mutations.is_empty()
                    || !checkpoint.completed_postconditions.is_empty())
            {
                return Ok(false);
            }
            checkpoint.updated_at_ms = now_ms();
            return Ok(existing == &identity_digest);
        }
        if now_ms().saturating_sub(checkpoint.updated_at_ms) > MAX_AGE_MS
            && (!checkpoint.pending_mutations.is_empty()
                || !checkpoint.completed_postconditions.is_empty())
        {
            return Ok(false);
        }
        if checkpoint.completed_targets.len() >= 32 {
            return Err("task checkpoint reached its target limit".into());
        }
        checkpoint
            .completed_targets
            .insert(target_digest, identity_digest);
        checkpoint.updated_at_ms = now_ms();
        self.persist()?;
        Ok(true)
    }

    pub fn finish(&mut self, key: &str) -> Result<(), String> {
        let _lock = self.lock_cross_process()?;
        self.refresh_from_disk()?;
        self.entries.remove(&digest(key.as_bytes()));
        self.persist()
    }

    /// Persist a write-ahead marker before dispatching a non-idempotent click.
    /// Repeating the same marker is safe; callers must never dispatch when this
    /// identity is already pending.
    pub fn mark_mutation_dispatch_may_have_occurred(
        &mut self,
        key: &str,
        task_digest: &str,
        step_identity: &str,
        target_identity: &str,
        document_identity: &str,
        predicate_identity: &str,
    ) -> Result<MutationCheckpointState, String> {
        let _lock = self.lock_cross_process()?;
        self.refresh_from_disk()?;
        if !is_sha256(task_digest) {
            return Err("task checkpoint digest is invalid".into());
        }
        let checkpoint_key = digest(key.as_bytes());
        let checkpoint = self
            .entries
            .entry(checkpoint_key)
            .or_insert_with(|| Checkpoint {
                task_digest: task_digest.to_owned(),
                completed_targets: BTreeMap::new(),
                completed_postconditions: BTreeMap::new(),
                pending_mutations: BTreeMap::new(),
                updated_at_ms: now_ms(),
            });
        if checkpoint.task_digest != task_digest {
            return Err("resume key is already bound to a different task".into());
        }
        let identity = mutation_identity(step_identity, target_identity, document_identity);
        let predicate = digest(predicate_identity.as_bytes());
        if let Some(existing) = checkpoint.completed_postconditions.get(&identity) {
            checkpoint.updated_at_ms = now_ms();
            return Ok(if existing == &predicate {
                MutationCheckpointState::Verified
            } else {
                MutationCheckpointState::Drifted
            });
        }
        if let Some(existing) = checkpoint.pending_mutations.get(&identity) {
            checkpoint.updated_at_ms = now_ms();
            return Ok(if existing == &predicate {
                MutationCheckpointState::DispatchMayHaveOccurred
            } else {
                MutationCheckpointState::Drifted
            });
        }
        if checkpoint.pending_mutations.len() + checkpoint.completed_postconditions.len()
            >= MAX_POSTCONDITIONS
        {
            return Err("task checkpoint reached its mutation limit".into());
        }
        checkpoint.pending_mutations.insert(identity, predicate);
        checkpoint.updated_at_ms = now_ms();
        self.persist()?;
        Ok(MutationCheckpointState::NotStarted)
    }

    /// Complete a write-ahead mutation only after the caller has freshly
    /// verified its exact postcondition on the bound document.
    pub fn mark_mutation_verified(
        &mut self,
        key: &str,
        task_digest: &str,
        step_identity: &str,
        target_identity: &str,
        document_identity: &str,
        predicate_identity: &str,
    ) -> Result<(), String> {
        let _lock = self.lock_cross_process()?;
        self.refresh_from_disk()?;
        let checkpoint_key = digest(key.as_bytes());
        let checkpoint = self
            .entries
            .get_mut(&checkpoint_key)
            .ok_or("mutation checkpoint is missing")?;
        if checkpoint.task_digest != task_digest {
            return Err("resume key is already bound to a different task".into());
        }
        let identity = mutation_identity(step_identity, target_identity, document_identity);
        let predicate = digest(predicate_identity.as_bytes());
        if checkpoint.pending_mutations.get(&identity) != Some(&predicate) {
            return Err("mutation write-ahead identity or predicate changed".into());
        }
        checkpoint.pending_mutations.remove(&identity);
        checkpoint
            .completed_postconditions
            .insert(identity, predicate);
        checkpoint.updated_at_ms = now_ms();
        self.persist()
    }

    pub fn mutation_state(
        &mut self,
        key: &str,
        task_digest: &str,
        step_identity: &str,
        target_identity: &str,
        document_identity: &str,
        predicate_identity: &str,
    ) -> MutationCheckpointState {
        let _lock = match self.lock_cross_process() {
            Ok(lock) => lock,
            Err(_) => return MutationCheckpointState::Unavailable,
        };
        if self.refresh_from_disk().is_err() {
            return MutationCheckpointState::Unavailable;
        }
        let identity = mutation_identity(step_identity, target_identity, document_identity);
        let predicate = digest(predicate_identity.as_bytes());
        let Some(checkpoint) = self.entries.get_mut(&digest(key.as_bytes())) else {
            return MutationCheckpointState::NotStarted;
        };
        if checkpoint.task_digest != task_digest {
            return MutationCheckpointState::Drifted;
        }
        let now = now_ms();
        if now.saturating_sub(checkpoint.updated_at_ms) > MAX_AGE_MS
            && (!checkpoint.pending_mutations.is_empty()
                || !checkpoint.completed_postconditions.is_empty())
        {
            return MutationCheckpointState::Expired;
        }
        checkpoint.updated_at_ms = now;
        if let Some(saved) = checkpoint.completed_postconditions.get(&identity) {
            return if saved == &predicate {
                MutationCheckpointState::Verified
            } else {
                MutationCheckpointState::Drifted
            };
        }
        if let Some(saved) = checkpoint.pending_mutations.get(&identity) {
            return if saved == &predicate {
                MutationCheckpointState::DispatchMayHaveOccurred
            } else {
                MutationCheckpointState::Drifted
            };
        }
        MutationCheckpointState::NotStarted
    }

    /// Record a postcondition after the caller has verified it against current
    /// application state. Artifact and evidence identifiers are digested before
    /// persistence; no raw identity or evidence content is stored.
    pub fn mark_postcondition_completed(
        &mut self,
        key: &str,
        task_digest: &str,
        artifact_identity: &str,
        evidence_identity: &str,
        postcondition_identity: &str,
    ) -> Result<(), String> {
        let _lock = self.lock_cross_process()?;
        self.refresh_from_disk()?;
        if !is_sha256(task_digest) {
            return Err("task checkpoint digest is invalid".into());
        }
        let checkpoint_key = digest(key.as_bytes());
        self.prune(now_ms());
        if !self.entries.contains_key(&checkpoint_key)
            && self.entries.len() >= MAX_CHECKPOINTS
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.updated_at_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        let checkpoint = self
            .entries
            .entry(checkpoint_key)
            .or_insert_with(|| Checkpoint {
                task_digest: task_digest.to_owned(),
                completed_targets: BTreeMap::new(),
                completed_postconditions: BTreeMap::new(),
                pending_mutations: BTreeMap::new(),
                updated_at_ms: now_ms(),
            });
        if checkpoint.task_digest != task_digest {
            return Err("resume key is already bound to a different task".into());
        }
        let identity = digest(
            &serde_json::to_vec(&(artifact_identity, evidence_identity))
                .map_err(|error| error.to_string())?,
        );
        let postcondition = digest(postcondition_identity.as_bytes());
        if !checkpoint.completed_postconditions.contains_key(&identity)
            && checkpoint.completed_postconditions.len() >= MAX_POSTCONDITIONS
        {
            return Err("task checkpoint reached its postcondition limit".into());
        }
        checkpoint
            .completed_postconditions
            .insert(identity, postcondition);
        checkpoint.updated_at_ms = now_ms();
        self.persist()
    }

    /// Check an exact artifact/evidence/postcondition tuple. This is an identity
    /// lookup, not a substitute for verifying the live postcondition.
    pub fn postcondition_completed(
        &mut self,
        key: &str,
        task_digest: &str,
        artifact_identity: &str,
        evidence_identity: &str,
        postcondition_identity: &str,
    ) -> bool {
        let _lock = match self.lock_cross_process() {
            Ok(lock) => lock,
            Err(_) => return false,
        };
        if self.refresh_from_disk().is_err() {
            return false;
        }
        let checkpoint_key = digest(key.as_bytes());
        let now = now_ms();
        let Some(checkpoint) = self.entries.get_mut(&checkpoint_key) else {
            return false;
        };
        if checkpoint.task_digest != task_digest {
            return false;
        }
        if now.saturating_sub(checkpoint.updated_at_ms) > MAX_AGE_MS {
            self.entries.remove(&checkpoint_key);
            let _ = self.persist();
            return false;
        }
        let identity = digest(
            &serde_json::to_vec(&(artifact_identity, evidence_identity)).unwrap_or_default(),
        );
        checkpoint.updated_at_ms = now;
        checkpoint
            .completed_postconditions
            .get(&identity)
            .is_some_and(|saved| saved == &digest(postcondition_identity.as_bytes()))
    }

    fn prune(&mut self, now: u64) {
        self.entries.retain(|_, entry| {
            now.saturating_sub(entry.updated_at_ms) <= MAX_AGE_MS
                || !entry.pending_mutations.is_empty()
                || !entry.completed_postconditions.is_empty()
        });
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let stored = StoredCheckpoints {
            version: 3,
            entries: self.entries.clone(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_STORE_BYTES {
            return Err("task checkpoint store reached its byte limit".into());
        }
        let parent = path.parent().ok_or("task checkpoint path has no parent")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temp = path.with_extension(format!("tmp-{}-{}", std::process::id(), now_ms()));
        std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        std::fs::rename(temp, path).map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationCheckpointState {
    NotStarted,
    DispatchMayHaveOccurred,
    Verified,
    Drifted,
    Expired,
    Unavailable,
}

fn mutation_identity(step: &str, target: &str, document: &str) -> String {
    digest(&serde_json::to_vec(&(step, target, document)).expect("tuple serializes"))
}

pub fn task_digest<T: Serialize>(task: &T) -> Result<String, String> {
    serde_json::to_vec(task)
        .map(|bytes| digest(&bytes))
        .map_err(|error| error.to_string())
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "controlla-task-checkpoints-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[test]
    fn checkpoints_survive_reopen_and_bind_exact_task_and_target() {
        let path = path();
        let task_digest =
            task_digest(&serde_json::json!({"urls":["https://example.test/"]})).unwrap();
        let mut store = TaskCheckpointStore::open(&path).unwrap();
        let identity = "https://example.test/\u{1f}loader-1\u{1f}frame-1";
        store
            .mark_target_completed("run-1", &task_digest, "https://example.test/", identity)
            .unwrap();
        drop(store);
        let mut reopened = TaskCheckpointStore::open(&path).unwrap();
        assert!(reopened.target_completed(
            "run-1",
            &task_digest,
            "https://example.test/",
            identity
        ));
        assert!(!reopened.target_completed(
            "run-1",
            &task_digest,
            "https://example.test/",
            "https://example.test/\u{1f}loader-2\u{1f}frame-1"
        ));
        assert!(!reopened.target_completed("run-1", &task_digest, "https://other.test/", identity));
        assert!(!reopened.target_completed(
            "run-1",
            &digest(b"different task"),
            "https://example.test/",
            identity
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn completion_removes_checkpoint_and_changed_task_key_is_rejected() {
        let mut store = TaskCheckpointStore::empty();
        let first = task_digest(&serde_json::json!({"kind":"research","urls":["https://a.test/"]}))
            .unwrap();
        let changed =
            task_digest(&serde_json::json!({"kind":"research","urls":["https://b.test/"]}))
                .unwrap();
        store
            .mark_target_completed("resume", &first, "https://a.test/", "identity")
            .unwrap();
        assert!(
            store
                .mark_target_completed("resume", &changed, "https://b.test/", "identity")
                .is_err()
        );
        store.finish("resume").unwrap();
        assert!(!store.target_completed("resume", &first, "https://a.test/", "identity"));
    }

    #[test]
    fn postconditions_bind_exact_artifact_evidence_task_and_verified_identity() {
        let path = path();
        let task =
            task_digest(&serde_json::json!({"kind":"authoring","artifact":"draft-7"})).unwrap();
        let mut store = TaskCheckpointStore::open(&path).unwrap();
        store
            .mark_postcondition_completed(
                "resume-authoring",
                &task,
                "artifact:report-7",
                "evidence:revision-12",
                "postcondition:draft-visible",
            )
            .unwrap();
        drop(store);

        let mut reopened = TaskCheckpointStore::open(&path).unwrap();
        let exact = (
            "artifact:report-7",
            "evidence:revision-12",
            "postcondition:draft-visible",
        );
        assert!(reopened.postcondition_completed(
            "resume-authoring",
            &task,
            exact.0,
            exact.1,
            exact.2
        ));
        assert!(!reopened.postcondition_completed(
            "resume-authoring",
            &task,
            "artifact:other",
            exact.1,
            exact.2
        ));
        assert!(!reopened.postcondition_completed(
            "resume-authoring",
            &task,
            exact.0,
            "evidence:stale",
            exact.2
        ));
        assert!(!reopened.postcondition_completed(
            "resume-authoring",
            &task,
            exact.0,
            exact.1,
            "postcondition:unknown"
        ));
        assert!(!reopened.postcondition_completed(
            "resume-authoring",
            &digest(b"other task"),
            exact.0,
            exact.1,
            exact.2
        ));
        let bytes = std::fs::read(&path).unwrap();
        let saved = String::from_utf8(bytes).unwrap();
        assert!(!saved.contains("artifact:report-7"));
        assert!(!saved.contains("evidence:revision-12"));
        assert!(!saved.contains("postcondition:draft-visible"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn postcondition_marks_reject_changed_task_binding() {
        let mut store = TaskCheckpointStore::empty();
        let first = task_digest(&serde_json::json!({"task":1})).unwrap();
        let changed = task_digest(&serde_json::json!({"task":2})).unwrap();
        store
            .mark_postcondition_completed("resume", &first, "a", "e", "p")
            .unwrap();
        assert!(
            store
                .mark_postcondition_completed("resume", &changed, "a", "e", "p")
                .is_err()
        );
    }

    #[test]
    fn write_ahead_mutation_state_survives_reopen_and_fences_drift() {
        let path = path();
        let task = task_digest(&serde_json::json!({"kind":"click","outcome":"saved"})).unwrap();
        let mut store = TaskCheckpointStore::open(&path).unwrap();
        assert_eq!(
            store
                .mark_mutation_dispatch_may_have_occurred(
                    "resume",
                    &task,
                    "step:0",
                    "tab:1",
                    "doc:1",
                    "text:saved"
                )
                .unwrap(),
            MutationCheckpointState::NotStarted
        );
        drop(store);
        let mut store = TaskCheckpointStore::open(&path).unwrap();
        assert_eq!(
            store.mutation_state("resume", &task, "step:0", "tab:1", "doc:1", "text:saved"),
            MutationCheckpointState::DispatchMayHaveOccurred
        );
        assert_eq!(
            store.mutation_state("resume", &task, "step:0", "tab:1", "doc:1", "text:other"),
            MutationCheckpointState::Drifted
        );
        assert_eq!(
            store.mutation_state("resume", &task, "step:0", "tab:1", "doc:2", "text:saved"),
            MutationCheckpointState::NotStarted
        );
        store
            .mark_mutation_verified("resume", &task, "step:0", "tab:1", "doc:1", "text:saved")
            .unwrap();
        let checkpoint = store.entries.get_mut(&digest(b"resume")).unwrap();
        checkpoint.updated_at_ms = 0;
        store.persist().unwrap();
        drop(store);
        let mut store = TaskCheckpointStore::open(&path).unwrap();
        assert_eq!(
            store.mutation_state("resume", &task, "step:0", "tab:1", "doc:1", "text:saved"),
            MutationCheckpointState::Expired
        );
        assert!(
            store
                .mark_mutation_verified("resume", &task, "step:0", "tab:1", "doc:1", "text:saved")
                .is_err()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn write_ahead_subprocess_exit_never_authorizes_replay() {
        if let Some(path) = std::env::var_os("CONTROLLA_MUTATION_CHILD_PATH") {
            let task =
                task_digest(&serde_json::json!({"kind":"save","predicate":"saved"})).unwrap();
            let mut store = TaskCheckpointStore::open(path).unwrap();
            store
                .mark_mutation_dispatch_may_have_occurred(
                    "resume",
                    &task,
                    "step:save",
                    "tab:1",
                    "doc:1",
                    "text:saved",
                )
                .unwrap();
            // Simulates process exit after the write-ahead marker but before
            // the remote effect or its verification.
            return;
        }
        let path = path();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("task_checkpoints::tests::write_ahead_subprocess_exit_never_authorizes_replay")
            .arg("--nocapture")
            .env("CONTROLLA_MUTATION_CHILD_PATH", &path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let task = task_digest(&serde_json::json!({"kind":"save","predicate":"saved"})).unwrap();
        let mut reopened = TaskCheckpointStore::open(&path).unwrap();
        assert_eq!(
            reopened.mutation_state("resume", &task, "step:save", "tab:1", "doc:1", "text:saved"),
            MutationCheckpointState::DispatchMayHaveOccurred
        );
        assert_eq!(
            reopened.mutation_state(
                "resume",
                &task,
                "step:save",
                "tab:1",
                "doc:1",
                "text:drifted"
            ),
            MutationCheckpointState::Drifted
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("lock"));
    }

    #[test]
    fn resumable_fill_target_binding_survives_reopen_and_refuses_document_drift() {
        let path = path();
        let task = task_digest(&serde_json::json!({"kind":"fill-only","fields":2})).unwrap();
        let mut store = TaskCheckpointStore::open(&path).unwrap();
        assert!(
            store
                .bind_target_identity("resume", &task, "https://example.test/", "loader-1/frame-1")
                .unwrap()
        );
        drop(store);

        let mut reopened = TaskCheckpointStore::open(&path).unwrap();
        assert!(
            reopened
                .bind_target_identity("resume", &task, "https://example.test/", "loader-1/frame-1")
                .unwrap()
        );
        assert!(
            !reopened
                .bind_target_identity("resume", &task, "https://example.test/", "loader-2/frame-1")
                .unwrap()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn concurrent_process_checkpoint_updates_are_merged() {
        if let Ok(key) = std::env::var("CONTROLLA_CHECKPOINT_CHILD_KEY") {
            let path = PathBuf::from(std::env::var_os("CONTROLLA_CHECKPOINT_CHILD_PATH").unwrap());
            let task = task_digest(&serde_json::json!({"kind":"shared-checkpoint"})).unwrap();
            let ready =
                PathBuf::from(std::env::var_os("CONTROLLA_CHECKPOINT_CHILD_READY").unwrap());
            let start =
                PathBuf::from(std::env::var_os("CONTROLLA_CHECKPOINT_CHILD_START").unwrap());
            std::fs::write(ready, b"ready").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !start.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(
                start.exists(),
                "parent did not release concurrent checkpoint writers"
            );
            let mut store = TaskCheckpointStore::open(path).unwrap();
            store
                .mark_target_completed(&key, &task, "target", &format!("identity:{key}"))
                .unwrap();
            return;
        }

        const WRITERS: usize = 8;
        let path = path();
        let start = path.with_extension("start");
        let ready_dir = path.with_extension("ready");
        std::fs::create_dir_all(&ready_dir).unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut children = Vec::new();
        for index in 0..WRITERS {
            let mut command = std::process::Command::new(&executable);
            command
                .arg("--exact")
                .arg("task_checkpoints::tests::concurrent_process_checkpoint_updates_are_merged")
                .arg("--nocapture")
                .env("CONTROLLA_CHECKPOINT_CHILD_KEY", format!("key-{index:03}"))
                .env("CONTROLLA_CHECKPOINT_CHILD_PATH", &path)
                .env("CONTROLLA_CHECKPOINT_CHILD_START", &start)
                .env(
                    "CONTROLLA_CHECKPOINT_CHILD_READY",
                    ready_dir.join(index.to_string()),
                )
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            children.push(command.spawn().unwrap());
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while (0..WRITERS).any(|index| !ready_dir.join(index.to_string()).exists())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!((0..WRITERS).all(|index| ready_dir.join(index.to_string()).exists()));
        std::fs::write(&start, b"go").unwrap();
        for mut child in children {
            assert!(
                child.wait().unwrap().success(),
                "checkpoint writer subprocess failed"
            );
        }

        let task = task_digest(&serde_json::json!({"kind":"shared-checkpoint"})).unwrap();
        let mut store = TaskCheckpointStore::open(&path).unwrap();
        for index in 0..WRITERS {
            let key = format!("key-{index:03}");
            assert!(store.target_completed(&key, &task, "target", &format!("identity:{key}")));
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("lock"));
        let _ = std::fs::remove_file(start);
        let _ = std::fs::remove_dir_all(ready_dir);
    }
}
