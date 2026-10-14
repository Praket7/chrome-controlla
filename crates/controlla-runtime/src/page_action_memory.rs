//! Exact-identity hints for previously verified page affordances.

use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

pub const MAX_ACTION_MEMORY_ENTRIES: usize = 512;
pub const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionMemoryKey {
    pub origin: String,
    pub structural_signature: String,
    pub intent: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionMemoryEntry {
    pub semantic_target: Value,
    pub action: String,
    pub expected_postcondition: Value,
    pub last_verified_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ActionMemory {
    entries: BTreeMap<ActionMemoryKey, ActionMemoryEntry>,
    path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct StoredActionMemory {
    version: u32,
    entries: Vec<(ActionMemoryKey, ActionMemoryEntry)>,
}

impl ActionMemory {
    /// Opens a bounded, cross-session action-memory file. Malformed or oversized
    /// state is rejected instead of silently treated as trusted memory.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let mut memory = Self {
            entries: BTreeMap::new(),
            path: Some(path.clone()),
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 512 * 1024 {
                    return Err("action memory exceeds its byte budget".into());
                }
                let stored: StoredActionMemory =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if stored.version != 1
                    || stored.entries.len() > MAX_ACTION_MEMORY_ENTRIES
                    || stored
                        .entries
                        .iter()
                        .any(|(key, entry)| !valid_key(key) || !valid_entry(entry))
                {
                    return Err("action memory failed schema or bounds validation".into());
                }
                memory.entries.extend(stored.entries);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        Ok(memory)
    }

    /// Store only independently passed evidence. A failed run invalidates the exact hint.
    pub fn record(
        &mut self,
        key: ActionMemoryKey,
        entry: ActionMemoryEntry,
        verification: Verification,
    ) -> bool {
        if verification == Verification::Failed {
            self.entries.remove(&key);
            let _ = self.persist();
            return false;
        }
        if verification != Verification::Passed
            || !valid_key(&key)
            || entry.action.is_empty()
            || entry.action.len() > 64
            || entry.semantic_target.is_null()
            || entry.expected_postcondition.is_null()
            || !valid_entry(&entry)
        {
            return false;
        }
        if !self.entries.contains_key(&key)
            && self.entries.len() >= MAX_ACTION_MEMORY_ENTRIES
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_verified_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, entry);
        self.persist().is_ok()
    }

    /// Returns a bounded grounding hint only; callers must still revalidate the live target.
    pub fn lookup(&self, key: &ActionMemoryKey, now_ms: u64) -> Option<&ActionMemoryEntry> {
        self.entries.get(key).filter(|entry| {
            entry.last_verified_ms <= now_ms
                && now_ms.saturating_sub(entry.last_verified_ms) <= MAX_AGE_MS
        })
    }

    /// Fast prefilter for the common miss path. Callers can avoid a live
    /// structural snapshot unless a fresh entry could match this origin/intent.
    pub fn has_fresh_origin_intent(&self, origin: &str, intent: &str, now_ms: u64) -> bool {
        self.entries.iter().any(|(key, entry)| {
            key.origin == origin
                && key.intent == intent
                && entry.last_verified_ms <= now_ms
                && now_ms.saturating_sub(entry.last_verified_ms) <= MAX_AGE_MS
        })
    }

    /// Returns only fresh hints for an exact page identity. Hints remain advisory;
    /// execution must resolve a new live semantic target before dispatch.
    pub fn lookup_page(
        &self,
        origin: &str,
        structural_signature: &str,
        now_ms: u64,
        limit: usize,
    ) -> Vec<(ActionMemoryKey, ActionMemoryEntry)> {
        let mut entries = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                key.origin == origin
                    && key.structural_signature == structural_signature
                    && entry.last_verified_ms <= now_ms
                    && now_ms.saturating_sub(entry.last_verified_ms) <= MAX_AGE_MS
            })
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.last_verified_ms));
        entries.truncate(limit.min(16));
        entries
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let stored = StoredActionMemory {
            version: 1,
            entries: self
                .entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.clone()))
                .collect(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(|error| error.to_string())?;
        if bytes.len() > 512 * 1024 {
            return Err("action memory reached its byte budget".into());
        }
        let parent = path.parent().ok_or("action memory path has no parent")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&temp).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        std::fs::rename(&temp, path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })
    }
}

fn valid_entry(entry: &ActionMemoryEntry) -> bool {
    !entry.action.is_empty()
        && entry.action.len() <= 64
        && !entry.semantic_target.is_null()
        && !entry.expected_postcondition.is_null()
        && serde_json::to_vec(entry).is_ok_and(|bytes| bytes.len() <= 4096)
}

fn valid_key(key: &ActionMemoryKey) -> bool {
    !key.origin.is_empty()
        && key.origin.len() <= 512
        && !key.structural_signature.is_empty()
        && key.structural_signature.len() <= 256
        && !key.intent.is_empty()
        && key.intent.len() <= 256
}
