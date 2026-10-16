//! Persistent qualification and replay policy for reusable browser skills.

use crate::{
    skill_store::{SkillStore, SkillStoreError},
    skills::{SkillDefinition, SkillError, SkillRecord, SkillStatus},
    verifier::Verification,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    future::Future,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Debug, thiserror::Error)]
pub enum SkillRuntimeError {
    #[error(transparent)]
    Store(#[from] SkillStoreError),
    #[error(transparent)]
    Skill(#[from] SkillError),
    #[error("unknown skill_id")]
    NotFound,
    #[error("skill replay is not currently qualified")]
    NotQualified,
    #[error("skill runtime lock is poisoned")]
    LockPoisoned,
    #[error("skill trace store failed: {0}")]
    Trace(String),
    #[error("account identity key unavailable or invalid: {0}")]
    AccountIdentityKey(String),
}

// Schedule best-effort post-response learning without awaiting it on the MCP
// response path. The task is owned by the active Tokio runtime.
pub fn schedule_post_response_work<F>(work: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(work);
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SkillReplay {
    pub skill_id: String,
    pub workflow: Value,
    pub version: u32,
    pub internal_model_calls: u32,
    /// Induction-derived safe fallback classes. Unknown mutations are never
    /// represented here and cannot authorize retry.
    #[serde(default)]
    pub failure_guards: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SkillRuntime {
    store: SkillStore,
    operation_lock: Arc<Mutex<()>>,
}

impl SkillRuntime {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            store: SkillStore::new(path),
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn store(&self) -> &SkillStore {
        &self.store
    }

    /// Persist only a keyed fingerprint. The installation key is a private,
    /// random sibling of the skill store and raw marker text is never saved.
    pub fn account_identity_fingerprint(
        &self,
        selector: &str,
        expected: &str,
    ) -> Result<Option<String>, SkillRuntimeError> {
        let key = self.load_or_create_account_identity_key()?;
        Ok(crate::skill_induction::account_identity_fingerprint(
            &key, selector, expected,
        ))
    }

    fn load_or_create_account_identity_key(&self) -> Result<[u8; 32], SkillRuntimeError> {
        let key_path = self.store.path().with_extension("account-identity-key-v1");
        let parent = key_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| SkillRuntimeError::AccountIdentityKey(error.to_string()))?;
        match fs::symlink_metadata(&key_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || metadata.len() != 32 {
                    return Err(SkillRuntimeError::AccountIdentityKey(
                        "key file is not a regular 32-byte secret".into(),
                    ));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o077 != 0 {
                        return Err(SkillRuntimeError::AccountIdentityKey(
                            "key file permissions are broader than 0600".into(),
                        ));
                    }
                }
                let mut options = OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NOFOLLOW);
                }
                let mut key = [0u8; 32];
                options
                    .open(&key_path)
                    .and_then(|mut file| {
                        let opened = file.metadata()?;
                        if !opened.is_file() || opened.len() != 32 {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "key file changed during open",
                            ));
                        }
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if opened.permissions().mode() & 0o077 != 0 {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::PermissionDenied,
                                    "key file permissions are broader than 0600",
                                ));
                            }
                        }
                        file.read_exact(&mut key)
                    })
                    .map_err(|error| SkillRuntimeError::AccountIdentityKey(error.to_string()))?;
                Ok(key)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut key = [0u8; 32];
                getrandom::fill(&mut key)
                    .map_err(|error| SkillRuntimeError::AccountIdentityKey(error.to_string()))?;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                match options.open(&key_path) {
                    Ok(mut file) => {
                        file.write_all(&key)
                            .and_then(|()| file.sync_all())
                            .map_err(|error| {
                                SkillRuntimeError::AccountIdentityKey(error.to_string())
                            })?;
                        Ok(key)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        self.load_or_create_account_identity_key()
                    }
                    Err(error) => Err(SkillRuntimeError::AccountIdentityKey(error.to_string())),
                }
            }
            Err(error) => Err(SkillRuntimeError::AccountIdentityKey(error.to_string())),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>, SkillRuntimeError> {
        self.operation_lock
            .lock()
            .map_err(|_| SkillRuntimeError::LockPoisoned)
    }

    pub fn register_candidate(
        &self,
        definition: SkillDefinition,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        let _guard = self.lock()?;
        let record = SkillRecord::candidate(definition)?;
        self.store.upsert(record.clone())?;
        Ok(record)
    }

    /// Persist a completed task trace and attempt candidate induction on a
    /// blocking worker. The caller must schedule this only after building the
    /// user-visible result; this method never waits for disk or induction work.
    pub fn record_trace_async(
        self: &Arc<Self>,
        traces: Arc<crate::skill_induction::SkillTraceStore>,
        trace: crate::skill_induction::SkillTrace,
    ) {
        let runtime = Arc::clone(self);
        std::mem::drop(tokio::task::spawn_blocking(move || {
            let site_scope = trace.site_scope.clone();
            let intent_fingerprint = trace.intent_fingerprint.clone();
            let account_identity_fingerprint = trace.account_identity_fingerprint.clone();
            if traces.record(trace).is_ok() {
                let _ = runtime.induce_from_traces_for_identity(
                    &traces,
                    &site_scope,
                    &intent_fingerprint,
                    account_identity_fingerprint.as_deref(),
                );
            }
        }));
    }

    /// Induce and persist a candidate from bounded local traces. Candidates are
    /// never promoted here; qualification still requires independent runs.
    pub fn induce_from_traces(
        &self,
        traces: &crate::skill_induction::SkillTraceStore,
        site_scope: &str,
        intent_fingerprint: &str,
    ) -> Result<Option<SkillRecord>, SkillRuntimeError> {
        self.induce_from_traces_for_identity(traces, site_scope, intent_fingerprint, None)
    }

    pub fn induce_from_traces_for_identity(
        &self,
        traces: &crate::skill_induction::SkillTraceStore,
        site_scope: &str,
        intent_fingerprint: &str,
        account_identity_fingerprint: Option<&str>,
    ) -> Result<Option<SkillRecord>, SkillRuntimeError> {
        let evidence = traces
            .evidence(site_scope, intent_fingerprint, account_identity_fingerprint)
            .map_err(SkillRuntimeError::Trace)?;
        let Some(candidate) = crate::skill_induction::induce_candidate(&evidence) else {
            return Ok(None);
        };
        if let Some(existing) = self.store.get(&candidate.definition.skill_id)? {
            // Do not replace training/validation evidence already attached to a
            // candidate. A new induction round needs a new versioned proposal.
            return Ok(Some(existing));
        }
        self.register_candidate(candidate.definition).map(Some)
    }

    pub fn record_training_success(
        &self,
        skill_id: &str,
        run_id: impl Into<String>,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        self.update(skill_id, |record| {
            record.record_training_success(run_id);
            Ok(())
        })
    }

    pub fn record_validation_success(
        &self,
        skill_id: &str,
        run_id: impl Into<String>,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        self.update(skill_id, |record| {
            record.record_validation_success(run_id);
            Ok(())
        })
    }

    pub fn record_training_example(
        &self,
        skill_id: &str,
        run_id: impl Into<String>,
        input_signature: impl Into<String>,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        let signature = input_signature.into();
        self.update(skill_id, |record| {
            record.record_training_example(run_id, signature);
            Ok(())
        })
    }

    pub fn record_validation_example(
        &self,
        skill_id: &str,
        run_id: impl Into<String>,
        input_signature: impl Into<String>,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        let signature = input_signature.into();
        self.update(skill_id, |record| {
            record.record_validation_example(run_id, signature);
            Ok(())
        })
    }

    pub fn qualify(
        &self,
        skill_id: &str,
        verification: Verification,
        qualified_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        self.update(skill_id, |record| {
            record.qualify(verification, qualified_at_ms, expires_at_ms)?;
            Ok(())
        })
    }

    pub fn status(&self, skill_id: &str, now_ms: u64) -> Result<SkillRecord, SkillRuntimeError> {
        let _guard = self.lock()?;
        let mut record = self
            .store
            .get(skill_id)?
            .ok_or(SkillRuntimeError::NotFound)?;
        if record.status == SkillStatus::Qualified && !record.replay_allowed(now_ms) {
            self.store.upsert(record.clone())?;
        }
        Ok(record)
    }

    /// Returns a zero-model-call replay only when the persisted skill is still
    /// qualified and the exact site/structure preconditions match. Any structural
    /// mismatch quarantines the skill instead of attempting a brittle replay.
    pub fn replay(
        &self,
        skill_id: &str,
        site_scope: &str,
        structural_signature: &str,
        now_ms: u64,
    ) -> Result<SkillReplay, SkillRuntimeError> {
        // Production MCP callsites that cannot prove a visible account marker
        // deliberately use this unscoped wrapper. Account-bound skills fail closed.
        self.replay_with_identity(skill_id, site_scope, structural_signature, None, now_ms)
    }

    /// Replay with an optional already-verified account identity fingerprint.
    /// The raw marker is not accepted or persisted at this layer.
    pub fn replay_with_identity(
        &self,
        skill_id: &str,
        site_scope: &str,
        structural_signature: &str,
        account_identity_fingerprint: Option<&str>,
        now_ms: u64,
    ) -> Result<SkillReplay, SkillRuntimeError> {
        let _guard = self.lock()?;
        let mut record = self
            .store
            .get(skill_id)?
            .ok_or(SkillRuntimeError::NotFound)?;
        if !record.replay_allowed(now_ms) {
            self.store.upsert(record)?;
            return Err(SkillRuntimeError::NotQualified);
        }
        let identity_matches = match record.definition.account_identity_fingerprint.as_deref() {
            Some(expected) => {
                account_identity_fingerprint == Some(expected)
                    && account_identity_fingerprint
                        .is_some_and(crate::skills::valid_account_identity_fingerprint)
            }
            None => !record.definition.requires_account_identity(),
        };
        if !identity_matches {
            record.quarantine(
                if record.definition.account_identity_fingerprint.is_some() {
                    "account identity missing or changed"
                } else {
                    "account identity scope required for mutation"
                },
            );
            self.store.upsert(record)?;
            return Err(SkillRuntimeError::NotQualified);
        }
        let structural_match = record.definition.site_scope == site_scope
            && record
                .definition
                .structural_signatures
                .iter()
                .any(|signature| signature == structural_signature);
        if !structural_match {
            record.quarantine("structural drift");
            self.store.upsert(record)?;
            return Err(SkillRuntimeError::NotQualified);
        }
        Ok(SkillReplay {
            skill_id: record.definition.skill_id,
            workflow: record.definition.workflow,
            version: record.definition.version,
            internal_model_calls: 0,
            failure_guards: record.definition.failure_guards,
        })
    }

    /// Finds a qualified skill for a deterministic task intent. Only exact site
    /// and structural identity can produce a replay; stale structure is quarantined.
    pub fn replay_matching(
        &self,
        intent_fingerprint: &str,
        site_scope: &str,
        structural_signature: &str,
        now_ms: u64,
    ) -> Result<Option<SkillReplay>, SkillRuntimeError> {
        self.replay_matching_with_identity(
            intent_fingerprint,
            site_scope,
            structural_signature,
            None,
            now_ms,
        )
    }

    pub fn replay_matching_with_identity(
        &self,
        intent_fingerprint: &str,
        site_scope: &str,
        structural_signature: &str,
        account_identity_fingerprint: Option<&str>,
        now_ms: u64,
    ) -> Result<Option<SkillReplay>, SkillRuntimeError> {
        let candidates = self.store.load()?;
        for (skill_id, record) in candidates {
            if record.status != SkillStatus::Qualified
                || record.definition.intent_fingerprint != intent_fingerprint
                || record.definition.site_scope != site_scope
            {
                continue;
            }
            if account_identity_fingerprint.is_some()
                && record.definition.account_identity_fingerprint.is_some()
                && record.definition.account_identity_fingerprint.as_deref()
                    != account_identity_fingerprint
            {
                continue;
            }
            match self.replay_with_identity(
                &skill_id,
                site_scope,
                structural_signature,
                account_identity_fingerprint,
                now_ms,
            ) {
                Ok(replay) => return Ok(Some(replay)),
                Err(SkillRuntimeError::NotQualified) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    pub fn has_qualified_intent(
        &self,
        intent_fingerprint: &str,
        site_scope: &str,
        now_ms: u64,
    ) -> Result<bool, SkillRuntimeError> {
        Ok(self.store.load()?.values().any(|record| {
            record.status == SkillStatus::Qualified
                && record.definition.intent_fingerprint == intent_fingerprint
                && record.definition.site_scope == site_scope
                && record.qualified_at_ms.is_some_and(|start| now_ms >= start)
                && record.expires_at_ms.is_some_and(|end| now_ms < end)
        }))
    }

    pub fn record_runtime_verification(
        &self,
        skill_id: &str,
        verification: Verification,
        severe_safety_failure: bool,
        structural_drift: bool,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        self.update(skill_id, |record| {
            record.record_runtime_verification(
                verification,
                severe_safety_failure,
                structural_drift,
            );
            Ok(())
        })
    }

    fn update(
        &self,
        skill_id: &str,
        update: impl FnOnce(&mut SkillRecord) -> Result<(), SkillRuntimeError>,
    ) -> Result<SkillRecord, SkillRuntimeError> {
        let _guard = self.lock()?;
        let mut record = self
            .store
            .get(skill_id)?
            .ok_or(SkillRuntimeError::NotFound)?;
        update(&mut record)?;
        self.store.upsert(record.clone())?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::SKILL_SCHEMA_VERSION;
    use serde_json::json;
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicU64, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn account_fingerprints_are_stable_per_installation_and_key_is_private() {
        let store_path = test_path("account-key-test");
        let runtime = SkillRuntime::new(&store_path);
        let first = runtime
            .account_identity_fingerprint("#account", "person@example.test")
            .unwrap()
            .unwrap();
        let again = runtime
            .account_identity_fingerprint("#account", "person@example.test")
            .unwrap()
            .unwrap();
        assert_eq!(first, again);
        let key_path = store_path.with_extension("account-identity-key-v1");
        assert_eq!(std::fs::metadata(&key_path).unwrap().len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let other = SkillRuntime::new(store_path.with_file_name("other.json"));
        let other_fp = other
            .account_identity_fingerprint("#account", "person@example.test")
            .unwrap()
            .unwrap();
        assert_ne!(first, other_fp);
    }

    #[test]
    fn account_identity_fingerprint_fails_closed_on_invalid_key_file() {
        let store_path = test_path("account-key-invalid");
        std::fs::create_dir_all(store_path.parent().unwrap()).unwrap();
        let key_path = store_path.with_extension("account-identity-key-v1");
        std::fs::write(&key_path, b"short").unwrap();
        let runtime = SkillRuntime::new(&store_path);
        assert!(
            runtime
                .account_identity_fingerprint("#account", "person@example.test")
                .is_err()
        );
    }

    fn test_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "controlla-{label}-{}-{nonce}/skills.json",
            std::process::id()
        ))
    }

    fn runtime() -> SkillRuntime {
        static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        SkillRuntime::new(std::env::temp_dir().join(format!(
            "controlla-skill-runtime-{}-{nonce}-{}/skills.json",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn definition() -> SkillDefinition {
        SkillDefinition {
            skill_id: "registration".into(),
            schema_version: SKILL_SCHEMA_VERSION,
            site_scope: "https://example.test".into(),
            intent_fingerprint: "registration-form".into(),
            account_identity_fingerprint: None,
            parameter_names: vec!["email".into()],
            preconditions: vec!["form-present".into()],
            workflow: json!({"steps":[{"kind":"extract"},{"kind":"verify"}]}),
            postconditions: vec!["email-matches".into()],
            failure_guards: Vec::new(),
            structural_signatures: vec!["registration:v1".into()],
            version: 1,
        }
    }

    fn qualify(runtime: &SkillRuntime) {
        qualify_definition(runtime, definition());
    }

    fn qualify_definition(runtime: &SkillRuntime, definition: SkillDefinition) {
        runtime.register_candidate(definition).unwrap();
        runtime
            .record_training_example("registration", "train-1", "input-1")
            .unwrap();
        runtime
            .record_training_example("registration", "train-2", "input-2")
            .unwrap();
        runtime
            .record_training_example("registration", "train-3", "input-3")
            .unwrap();
        runtime
            .record_validation_example("registration", "validate-1", "input-4")
            .unwrap();
        runtime
            .record_validation_example("registration", "validate-2", "input-5")
            .unwrap();
        runtime
            .qualify("registration", Verification::Passed, 100, 1000)
            .unwrap();
    }

    #[test]
    fn qualified_exact_match_replays_with_zero_internal_model_calls() {
        let runtime = runtime();
        qualify(&runtime);
        let replay = runtime
            .replay(
                "registration",
                "https://example.test",
                "registration:v1",
                200,
            )
            .unwrap();
        assert_eq!(replay.internal_model_calls, 0);
        assert_eq!(replay.skill_id, "registration");
        let _ = std::fs::remove_dir_all(runtime.store().path().parent().unwrap());
    }

    #[test]
    fn replay_matching_retrieves_only_qualified_intent_on_exact_site_and_structure() {
        let runtime = runtime();
        qualify(&runtime);
        assert!(
            runtime
                .has_qualified_intent("registration-form", "https://example.test", 200)
                .unwrap()
        );
        assert!(
            !runtime
                .has_qualified_intent("registration-form", "https://example.test", 1000)
                .unwrap()
        );
        assert_eq!(
            runtime
                .replay_matching(
                    "registration-form",
                    "https://example.test",
                    "registration:v1",
                    200
                )
                .unwrap()
                .unwrap()
                .skill_id,
            "registration"
        );
        assert!(
            runtime
                .replay_matching(
                    "other-intent",
                    "https://example.test",
                    "registration:v1",
                    200
                )
                .unwrap()
                .is_none()
        );
        assert!(
            runtime
                .replay_matching(
                    "registration-form",
                    "https://other.test",
                    "registration:v1",
                    200
                )
                .unwrap()
                .is_none()
        );
        assert!(
            runtime
                .replay_matching(
                    "registration-form",
                    "https://example.test",
                    "registration:v2",
                    200
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(
            runtime.status("registration", 200).unwrap().status,
            SkillStatus::Quarantined
        );
        let _ = std::fs::remove_dir_all(runtime.store().path().parent().unwrap());
    }

    #[test]
    fn structural_drift_quarantines_and_persists() {
        let runtime = runtime();
        qualify(&runtime);
        assert!(matches!(
            runtime.replay(
                "registration",
                "https://example.test",
                "registration:v2",
                200
            ),
            Err(SkillRuntimeError::NotQualified)
        ));
        assert_eq!(
            runtime.status("registration", 200).unwrap().status,
            SkillStatus::Quarantined
        );
        let _ = std::fs::remove_dir_all(runtime.store().path().parent().unwrap());
    }

    #[test]
    fn account_scoped_replay_requires_exact_identity_and_quarantines_drift() {
        let identity =
            "hmac-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let changed =
            "hmac-sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let make_scoped = || {
            let runtime = runtime();
            let mut definition = definition();
            definition.account_identity_fingerprint = Some(identity.into());
            runtime.register_candidate(definition).unwrap();
            runtime
                .record_training_example("registration", "train-1", "input-1")
                .unwrap();
            runtime
                .record_training_example("registration", "train-2", "input-2")
                .unwrap();
            runtime
                .record_training_example("registration", "train-3", "input-3")
                .unwrap();
            runtime
                .record_validation_example("registration", "validate-1", "input-4")
                .unwrap();
            runtime
                .record_validation_example("registration", "validate-2", "input-5")
                .unwrap();
            runtime
                .qualify("registration", Verification::Passed, 100, 1000)
                .unwrap();
            runtime
        };

        let missing_runtime = make_scoped();
        assert!(matches!(
            missing_runtime.replay(
                "registration",
                "https://example.test",
                "registration:v1",
                200
            ),
            Err(SkillRuntimeError::NotQualified)
        ));
        let missing_record = missing_runtime.status("registration", 200).unwrap();
        assert_eq!(missing_record.status, SkillStatus::Quarantined);
        assert_eq!(
            missing_record.quarantine_reason.as_deref(),
            Some("account identity missing or changed")
        );

        let exact_runtime = make_scoped();
        assert!(
            exact_runtime
                .replay_with_identity(
                    "registration",
                    "https://example.test",
                    "registration:v1",
                    Some(identity),
                    200
                )
                .is_ok()
        );

        let changed_runtime = make_scoped();
        assert!(matches!(
            changed_runtime.replay_with_identity(
                "registration",
                "https://example.test",
                "registration:v1",
                Some(changed),
                200
            ),
            Err(SkillRuntimeError::NotQualified)
        ));
        assert_eq!(
            changed_runtime.status("registration", 200).unwrap().status,
            SkillStatus::Quarantined
        );
    }

    #[test]
    fn unscoped_mutation_cannot_replay_but_unscoped_read_only_can() {
        let mutation_runtime = runtime();
        let mut mutation = definition();
        mutation.workflow = json!({"steps":[{"kind":"fill"}]});
        qualify_definition(&mutation_runtime, mutation);
        assert!(matches!(
            mutation_runtime.replay(
                "registration",
                "https://example.test",
                "registration:v1",
                200
            ),
            Err(SkillRuntimeError::NotQualified)
        ));
        assert_eq!(
            mutation_runtime.status("registration", 200).unwrap().status,
            SkillStatus::Quarantined
        );
        assert_eq!(
            mutation_runtime
                .status("registration", 200)
                .unwrap()
                .quarantine_reason
                .as_deref(),
            Some("account identity scope required for mutation")
        );

        let read_runtime = runtime();
        qualify(&read_runtime);
        let marker = "hmac-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(
            read_runtime
                .replay_with_identity(
                    "registration",
                    "https://example.test",
                    "registration:v1",
                    Some(marker),
                    200
                )
                .is_ok()
        );
    }

    #[test]
    fn expiry_and_severe_safety_fail_closed() {
        let expiring_runtime = runtime();
        qualify(&expiring_runtime);
        assert!(matches!(
            expiring_runtime.replay(
                "registration",
                "https://example.test",
                "registration:v1",
                1000
            ),
            Err(SkillRuntimeError::NotQualified)
        ));
        assert_eq!(
            expiring_runtime
                .status("registration", 1000)
                .unwrap()
                .status,
            SkillStatus::Expired
        );
        let _ = std::fs::remove_dir_all(expiring_runtime.store().path().parent().unwrap());

        let safety_runtime = runtime();
        qualify(&safety_runtime);
        let record = safety_runtime
            .record_runtime_verification("registration", Verification::Passed, true, false)
            .unwrap();
        assert_eq!(record.status, SkillStatus::Quarantined);
        let _ = std::fs::remove_dir_all(safety_runtime.store().path().parent().unwrap());
    }

    #[test]
    fn concurrent_updates_to_one_skill_do_not_lose_evidence() {
        const WRITERS: usize = 16;
        let runtime = runtime();
        runtime.register_candidate(definition()).unwrap();
        let barrier = Arc::new(Barrier::new(WRITERS));
        let mut threads = Vec::with_capacity(WRITERS);
        for index in 0..WRITERS {
            let runtime = runtime.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                runtime
                    .record_training_success("registration", format!("train-{index}"))
                    .unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let record = runtime.status("registration", 0).unwrap();
        assert_eq!(record.training_run_ids.len(), WRITERS);
        let _ = std::fs::remove_dir_all(runtime.store().path().parent().unwrap());
    }
}

#[cfg(test)]
mod post_response_tests {
    use super::*;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn post_response_scheduler_returns_before_delayed_work_finishes() {
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let started = Instant::now();
        schedule_post_response_work(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            let _ = done_tx.send(());
        });
        assert!(started.elapsed() < Duration::from_millis(40));
        tokio::time::timeout(Duration::from_secs(1), done_rx)
            .await
            .expect("background task should finish")
            .expect("background task should signal completion");
    }

    #[tokio::test]
    async fn trace_persistence_is_scheduled_and_eventually_visible() {
        let root = std::env::temp_dir().join(format!(
            "controlla-skill-background-{}-{}/",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runtime = Arc::new(SkillRuntime::new(root.join("skills.json")));
        let traces = Arc::new(
            crate::skill_induction::SkillTraceStore::open(root.join("traces.json")).unwrap(),
        );
        let trace = crate::skill_induction::SkillTrace::failure(
            "background-run",
            "https://example.test",
            "read-intent",
            "structure-v1",
            crate::skill_induction::FailureClass::ReadFailure,
            "fixture failure",
        );
        runtime.record_trace_async(Arc::clone(&traces), trace);
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if !traces
                .evidence("https://example.test", "read-intent", None)
                .unwrap()
                .traces
                .is_empty()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "background worker should persist the trace"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
