use crate::skills::SkillRecord;
use std::{collections::BTreeMap, path::{Path, PathBuf}};

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
}

#[derive(Clone, Debug)]
pub struct SkillStore {
    path: PathBuf,
}

impl SkillStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<BTreeMap<String, SkillRecord>, SkillStoreError> {
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

    pub fn get(&self, skill_id: &str) -> Result<Option<SkillRecord>, SkillStoreError> {
        Ok(self.load()?.remove(skill_id))
    }

    pub fn upsert(&self, record: SkillRecord) -> Result<(), SkillStoreError> {
        let mut records = self.load()?;
        if !records.contains_key(&record.definition.skill_id) && records.len() >= MAX_SKILLS {
            return Err(SkillStoreError::Capacity);
        }
        records.insert(record.definition.skill_id.clone(), record);
        self.save(&records)
    }

    pub fn save(&self, records: &BTreeMap<String, SkillRecord>) -> Result<(), SkillStoreError> {
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
        let temp = self.path.with_extension(format!("tmp-{}", std::process::id()));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{SKILL_SCHEMA_VERSION, SkillDefinition, SkillRecord};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn record() -> SkillRecord {
        SkillRecord::candidate(SkillDefinition {
            skill_id: "skill-1".into(),
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
}
