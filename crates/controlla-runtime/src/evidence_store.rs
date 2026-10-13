//! Bounded, restart-safe task evidence keyed by an opaque site-and-intent scope.

use crate::visual_probe::{
    EvidenceEntry, EvidenceTrail, MAX_TASK_EVIDENCE_BYTES, MAX_TASK_EVIDENCE_ENTRIES,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const STORE_VERSION: u32 = 1;
const MAX_SCOPES: usize = 128;
const MAX_STORE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct EvidenceDatabase {
    version: u32,
    scopes: BTreeMap<String, Vec<EvidenceEntry>>,
}

pub struct PersistentEvidenceStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl PersistentEvidenceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn replace(&self, scope: &str, trail: &EvidenceTrail) -> Result<(), String> {
        if scope.is_empty() || scope.len() > 128 {
            return Err("evidence scope must be a bounded opaque key".into());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "evidence store lock poisoned")?;
        let mut database = self.read()?;
        database
            .scopes
            .insert(scope.to_owned(), trail.recent(MAX_TASK_EVIDENCE_ENTRIES));
        while database.scopes.len() > MAX_SCOPES {
            let oldest = database.scopes.keys().next().cloned().unwrap_or_default();
            database.scopes.remove(&oldest);
        }
        let encoded = serde_json::to_vec(&database).map_err(|error| error.to_string())?;
        if encoded.len() > MAX_STORE_BYTES {
            return Err("evidence store reached its total byte budget".into());
        }
        self.write_atomic(&encoded)
    }

    pub fn recent_relevant(&self, scope: &str, intent: &str, limit: usize) -> Vec<EvidenceEntry> {
        let Ok(_guard) = self.lock.lock() else {
            return Vec::new();
        };
        let Ok(database) = self.read() else {
            return Vec::new();
        };
        database
            .scopes
            .get(scope)
            .map(|entries| relevant_entries(entries, intent, limit))
            .unwrap_or_default()
    }

    pub fn recent(&self, scope: &str, limit: usize) -> Vec<EvidenceEntry> {
        let Ok(_guard) = self.lock.lock() else {
            return Vec::new();
        };
        let Ok(database) = self.read() else {
            return Vec::new();
        };
        database
            .scopes
            .get(scope)
            .map(|entries| {
                entries
                    .iter()
                    .rev()
                    .take(limit.min(16))
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn read(&self) -> Result<EvidenceDatabase, String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(EvidenceDatabase {
                    version: STORE_VERSION,
                    ..Default::default()
                });
            }
            Err(error) => return Err(error.to_string()),
        };
        if bytes.len() > MAX_STORE_BYTES {
            return Err("evidence store exceeds its byte budget".into());
        }
        let mut database: EvidenceDatabase =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if database.version != STORE_VERSION
            || database.scopes.len() > MAX_SCOPES
            || database.scopes.iter().any(|(scope, entries)| {
                scope.is_empty() || scope.len() > 128 || !valid_entries(entries)
            })
        {
            return Err("evidence store failed schema or bounds validation".into());
        }
        database.version = STORE_VERSION;
        Ok(database)
    }

    fn write_atomic(&self, bytes: &[u8]) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "evidence path has no parent".to_owned())?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = self
            .path
            .with_extension(format!("tmp-{}-{nonce}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        use std::io::Write;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, &self.path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            error.to_string()
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn valid_entries(entries: &[EvidenceEntry]) -> bool {
    entries.len() <= MAX_TASK_EVIDENCE_ENTRIES
        && entries.iter().all(|entry| {
            entry.fact.len() <= 2048
                && entry.kind.len() <= 64
                && entry
                    .public_ref
                    .as_ref()
                    .is_none_or(|value| value.len() <= 128)
        })
        && serde_json::to_vec(entries).is_ok_and(|bytes| bytes.len() <= MAX_TASK_EVIDENCE_BYTES)
}

fn relevant_entries(entries: &[EvidenceEntry], intent: &str, limit: usize) -> Vec<EvidenceEntry> {
    let terms = intent
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 3)
        .collect::<std::collections::BTreeSet<_>>();
    let mut ranked = entries
        .iter()
        .filter_map(|entry| {
            let content = format!(
                "{} {} {}",
                entry.kind,
                entry.fact,
                entry.public_ref.as_deref().unwrap_or_default()
            );
            let words = content
                .split(|character: char| !character.is_alphanumeric())
                .map(str::to_lowercase)
                .collect::<std::collections::BTreeSet<_>>();
            let score = terms.intersection(&words).count();
            (score > 0).then_some((score, entry.step, entry))
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(score, step, _)| (std::cmp::Reverse(*score), std::cmp::Reverse(*step)));
    ranked
        .into_iter()
        .take(limit.min(16))
        .map(|(_, _, entry)| entry.clone())
        .collect()
}

pub fn scope_key(origin: &str, intent_fingerprint: &str) -> String {
    let digest = sha2::Sha256::digest(format!("{origin}\u{1f}{intent_fingerprint}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scopes_are_opaque_and_stable_without_exposing_site_or_intent() {
        let a = scope_key("https://example.test", "invoice-search");
        assert_eq!(a, scope_key("https://example.test", "invoice-search"));
        assert_ne!(a, scope_key("https://example.test", "customer-search"));
        assert_eq!(a.len(), 64);
        assert!(!a.contains("example"));
    }
}
