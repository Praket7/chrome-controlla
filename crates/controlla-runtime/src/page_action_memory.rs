//! Exact-identity hints for previously verified page affordances.

use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

pub const MAX_ACTION_MEMORY_ENTRIES: usize = 512;
pub const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayStatus {
    Hit,
    Miss,
    Stale,
    Disabled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReplayPlan {
    Extract { selector: String },
}

#[cfg(test)]
mod tests {
    use super::{ActionMemory, ActionMemoryEntry, ActionMemoryKey, ReplayPlan};
    use crate::verifier::Verification;
    use serde_json::json;

    fn key() -> ActionMemoryKey {
        ActionMemoryKey {
            origin: "https://example.test".into(),
            structural_signature: "sig".into(),
            intent: "research:abc".into(),
        }
    }

    fn entry(selector: &str) -> ActionMemoryEntry {
        ActionMemoryEntry {
            semantic_target: json!({"selector": selector}),
            action: "extract".into(),
            expected_postcondition: json!({"kind":"nonempty_complete_extraction"}),
            replay: Some(ReplayPlan::Extract {
                selector: selector.into(),
            }),
            last_verified_ms: 100,
        }
    }

    #[test]
    fn verified_replay_record_round_trips_and_can_be_invalidated() {
        let mut memory = ActionMemory::default();
        let key = key();
        assert!(memory.record(key.clone(), entry("main article"), Verification::Passed));
        assert!(memory.lookup(&key, 101).is_some());
        assert!(memory.invalidate(&key));
        assert!(memory.lookup(&key, 101).is_none());
    }

    #[test]
    fn replay_record_rejects_secret_bearing_targets() {
        let mut memory = ActionMemory::default();
        assert!(!memory.record(
            key(),
            entry("input[type=password]").clone(),
            Verification::Passed,
        ));
        assert!(!memory.record(
            key(),
            ActionMemoryEntry {
                semantic_target: json!({"selector":"main"}),
                action: "fill".into(),
                expected_postcondition: json!({"kind":"filled","value":"super-secret"}),
                replay: None,
                last_verified_ms: 100,
            },
            Verification::Passed,
        ));
    }

    #[test]
    fn stale_records_are_not_replayable() {
        let mut memory = ActionMemory::default();
        let key = key();
        assert!(memory.record(key.clone(), entry("main"), Verification::Passed));
        assert!(memory.lookup(&key, 100 + super::MAX_AGE_MS + 1).is_none());
        assert_eq!(
            memory.replay_status(&key, 100 + super::MAX_AGE_MS + 1),
            super::ReplayStatus::Stale
        );
    }
}

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
    /// A bounded, non-secret replay description. Older persisted entries omit it.
    #[serde(default)]
    pub replay: Option<ReplayPlan>,
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
        if verification != Verification::Passed {
            self.entries.remove(&key);
            let _ = self.persist();
            return false;
        }
        if !valid_key(&key)
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

    pub fn replay_status(&self, key: &ActionMemoryKey, now_ms: u64) -> ReplayStatus {
        let Some(entry) = self.entries.get(key) else {
            return ReplayStatus::Miss;
        };
        if entry.replay.is_none() {
            return ReplayStatus::Disabled;
        }
        if entry.last_verified_ms > now_ms
            || now_ms.saturating_sub(entry.last_verified_ms) > MAX_AGE_MS
        {
            return ReplayStatus::Stale;
        }
        ReplayStatus::Hit
    }

    /// Removes one exact hint after structural drift or an unverified result.
    pub fn invalidate(&mut self, key: &ActionMemoryKey) -> bool {
        let removed = self.entries.remove(key).is_some();
        if removed {
            let _ = self.persist();
        }
        removed
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
        && !contains_secret_marker(&entry.semantic_target)
        && !contains_secret_marker(&entry.expected_postcondition)
        && entry.replay.as_ref().is_none_or(valid_replay_plan)
        && serde_json::to_vec(entry).is_ok_and(|bytes| bytes.len() <= 4096)
}

fn valid_replay_plan(plan: &ReplayPlan) -> bool {
    match plan {
        ReplayPlan::Extract { selector } => {
            !selector.is_empty()
                && selector.len() <= 512
                && !contains_secret_marker(&Value::String(selector.clone()))
        }
    }
}

fn contains_secret_marker(value: &Value) -> bool {
    fn visit(value: &Value) -> bool {
        match value {
            Value::String(text) => {
                let lower = text.to_ascii_lowercase();
                [
                    "password",
                    "passwd",
                    "secret",
                    "credential",
                    "api_key",
                    "access_token",
                ]
                .iter()
                .any(|marker| lower.contains(marker))
            }
            Value::Object(object) => object.iter().any(|(key, child)| {
                let lower = key.to_ascii_lowercase();
                [
                    "password",
                    "passwd",
                    "secret",
                    "credential",
                    "raw_value",
                    "access_token",
                ]
                .iter()
                .any(|marker| lower.contains(marker))
                    || visit(child)
            }),
            Value::Array(items) => items.iter().any(visit),
            _ => false,
        }
    }
    visit(value)
}

fn valid_key(key: &ActionMemoryKey) -> bool {
    !key.origin.is_empty()
        && key.origin.len() <= 512
        && !key.structural_signature.is_empty()
        && key.structural_signature.len() <= 256
        && !key.intent.is_empty()
        && key.intent.len() <= 256
}
