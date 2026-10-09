//! Persistent qualification and replay policy for reusable browser skills.

use crate::{
    skill_store::{SkillStore, SkillStoreError},
    skills::{SkillDefinition, SkillError, SkillRecord, SkillStatus},
    verifier::Verification,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
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
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SkillReplay {
    pub skill_id: String,
    pub workflow: Value,
    pub version: u32,
    pub internal_model_calls: u32,
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
        let _guard = self.lock()?;
        let mut record = self
            .store
            .get(skill_id)?
            .ok_or(SkillRuntimeError::NotFound)?;
        if !record.replay_allowed(now_ms) {
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
        })
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
        sync::{Arc, Barrier},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn runtime() -> SkillRuntime {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        SkillRuntime::new(std::env::temp_dir().join(format!(
            "controlla-skill-runtime-{}-{nonce}/skills.json",
            std::process::id()
        )))
    }

    fn definition() -> SkillDefinition {
        SkillDefinition {
            skill_id: "registration".into(),
            schema_version: SKILL_SCHEMA_VERSION,
            site_scope: "https://example.test".into(),
            intent_fingerprint: "registration-form".into(),
            parameter_names: vec!["email".into()],
            preconditions: vec!["form-present".into()],
            workflow: json!({"steps":[{"kind":"find"},{"kind":"fill"},{"kind":"verify"}]}),
            postconditions: vec!["email-matches".into()],
            structural_signatures: vec!["registration:v1".into()],
            version: 1,
        }
    }

    fn qualify(runtime: &SkillRuntime) {
        runtime.register_candidate(definition()).unwrap();
        runtime
            .record_training_success("registration", "train-1")
            .unwrap();
        runtime
            .record_validation_success("registration", "validate-1")
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
