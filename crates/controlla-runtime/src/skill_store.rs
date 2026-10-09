use crate::skills::SkillRecord;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

const MAX_SKILLS: usize = 1024;
const MAX_STORE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SkillStoreError {
    #[error("skill store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("skill store JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("skill store exceeds its bounded capacity")]
    Capacity,
    #[error("skill store lock is poisoned")]
    LockPoisoned,
}

#[derive(Clone, Debug)]
pub struct SkillStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl SkillStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>, SkillStoreError> {
        self.lock.lock().map_err(|_| SkillStoreError::LockPoisoned)
    }

    fn load_unlocked(&self) -> Result<BTreeMap<String, SkillRecord>, SkillStoreError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let metadata = std::fs::metadata(&self.path)?;
        if metadata.len() > MAX_STORE_BYTES {
            return Err(SkillStoreError::Capacity);
        }
        let bytes = std::fs::read(&self.path)?;
        let records = serde_json::from_slice::<BTreeMap<String, SkillRecord>>(&bytes)?;
        if records.len() > MAX_SKILLS {
            return Err(SkillStoreError::Capacity);
        }
        Ok(records)
    }

    fn save_unlocked(
        &self,
        records: &BTreeMap<String, SkillRecord>,
    ) -> Result<(), SkillStoreError> {
        if records.len() > MAX_SKILLS {
            return Err(SkillStoreError::Capacity);
        }
        let bytes = serde_json::to_vec_pretty(records)?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(SkillStoreError::Capacity);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temp, bytes)?;
        if let Err(first_error) = std::fs::rename(&temp, &self.path) {
            if self.path.exists() {
                std::fs::remove_file(&self.path)?;
                std::fs::rename(&temp, &self.path)?;
            } else {
                return Err(first_error.into());
            }
        }
        Ok(())
    }

    pub fn load(&self) -> Result<BTreeMap<String, SkillRecord>, SkillStoreError> {
        let _guard = self.lock()?;
        self.load_unlocked()
    }

    pub fn get(&self, skill_id: &str) -> Result<Option<SkillRecord>, SkillStoreError> {
        let _guard = self.lock()?;
        Ok(self.load_unlocked()?.remove(skill_id))
    }

    pub fn upsert(&self, record: SkillRecord) -> Result<(), SkillStoreError> {
        let _guard = self.lock()?;
        let mut records = self.load_unlocked()?;
        if !records.contains_key(&record.definition.skill_id) && records.len() >= MAX_SKILLS {
            return Err(SkillStoreError::Capacity);
        }
        records.insert(record.definition.skill_id.clone(), record);
        self.save_unlocked(&records)
    }

    pub fn save(&self, records: &BTreeMap<String, SkillRecord>) -> Result<(), SkillStoreError> {
        let _guard = self.lock()?;
        self.save_unlocked(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{SKILL_SCHEMA_VERSION, SkillDefinition, SkillRecord};
    use serde_json::json;
    use std::{
        sync::{Arc, Barrier},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_store() -> SkillStore {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        SkillStore::new(std::env::temp_dir().join(format!(
            "controlla-skill-store-{}-{nonce}/skills.json",
            std::process::id()
        )))
    }

    fn record_with_id(skill_id: impl Into<String>) -> SkillRecord {
        SkillRecord::candidate(SkillDefinition {
            skill_id: skill_id.into(),
            schema_version: SKILL_SCHEMA_VERSION,
            site_scope: "https://example.test".into(),
            intent_fingerprint: "intent".into(),
            parameter_names: vec![],
            preconditions: vec!["ready".into()],
            workflow: json!({"steps":[]}),
            postconditions: vec!["done".into()],
            structural_signatures: vec!["sig".into()],
            version: 1,
        })
        .unwrap()
    }

    fn record() -> SkillRecord {
        record_with_id("skill-1")
    }

    #[test]
    fn store_roundtrips_and_replaces_records() {
        let store = unique_store();
        let record = record();
        store.upsert(record.clone()).unwrap();
        assert_eq!(store.get("skill-1").unwrap(), Some(record.clone()));
        store.upsert(record).unwrap();
        assert_eq!(store.load().unwrap().len(), 1);
        let root = store.path().parent().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_store_fails_closed() {
        let store = unique_store();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), b"not-json").unwrap();
        assert!(matches!(store.load(), Err(SkillStoreError::Json(_))));
        let root = store.path().parent().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cloned_stores_serialize_concurrent_updates() {
        const WRITERS: usize = 16;
        let store = unique_store();
        let barrier = Arc::new(Barrier::new(WRITERS));
        let mut threads = Vec::with_capacity(WRITERS);
        for index in 0..WRITERS {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                store
                    .upsert(record_with_id(format!("skill-{index}")))
                    .unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(store.load().unwrap().len(), WRITERS);
        let root = store.path().parent().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}
