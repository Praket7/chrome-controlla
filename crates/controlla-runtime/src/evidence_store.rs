//! Bounded, restart-safe task evidence keyed by an opaque site-and-intent scope.

use crate::visual_probe::{
    EvidenceEntry, EvidenceTrail, MAX_TASK_EVIDENCE_BYTES, MAX_TASK_EVIDENCE_ENTRIES,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    collections::BTreeMap,
    fs::File,
    io,
    path::Path,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const STORE_VERSION: u32 = 2;
const MAX_SCOPES: usize = 128;
const MAX_STORE_BYTES: usize = 1024 * 1024;
// Keep Base64 below the existing MCP image-content cap so every stored visual
// can be retrieved as image content without placing pixel bytes in JSON.
const MAX_VISUAL_PNG_BYTES: usize = 270_000;
const MAX_VISUALS_PER_PRINCIPAL: usize = 8;
const MAX_VISUAL_BYTES_PER_PRINCIPAL: usize = 8 * 270_000;
const MAX_VISUALS_TOTAL: usize = 64;
const MAX_VISUAL_BYTES_TOTAL: usize = 64 * 270_000;
const VISUAL_TTL_MS: u64 = 60 * 60 * 1000;

/// Advisory OS lock held across each database read/modify/write transaction.
/// The lock file is stable and never atomically replaced with the database.
struct CrossProcessLock {
    file: File,
}

impl CrossProcessLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "evidence lock path has no parent".to_owned())?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        lock_file(&file).map_err(|error| error.to_string())?;
        Ok(Self { file })
    }
}

impl Drop for CrossProcessLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

#[cfg(unix)]
fn lock_file(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: flock only observes the live descriptor; the RAII guard keeps the
    // file open until after the matching unlock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_file(file: &File) {
    use std::os::fd::AsRawFd;
    // SAFETY: called while the descriptor remains open and held by the guard.
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
#[repr(C)]
struct WindowsOverlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut std::ffi::c_void,
}

#[cfg(windows)]
fn lock_file(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn LockFileEx(
            handle: *mut std::ffi::c_void,
            flags: u32,
            reserved: u32,
            low: u32,
            high: u32,
            overlapped: *mut WindowsOverlapped,
        ) -> i32;
    }
    let mut overlapped = WindowsOverlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        offset_high: 0,
        event: std::ptr::null_mut(),
    };
    // LOCKFILE_EXCLUSIVE_LOCK; omitting FAIL_IMMEDIATELY requests a blocking lock.
    // SAFETY: the file handle and OVERLAPPED remain alive for the synchronous wait.
    if unsafe { LockFileEx(file.as_raw_handle().cast(), 2, 0, 1, 0, &mut overlapped) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn unlock_file(file: &File) {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn UnlockFileEx(
            handle: *mut std::ffi::c_void,
            reserved: u32,
            low: u32,
            high: u32,
            overlapped: *mut WindowsOverlapped,
        ) -> i32;
    }
    let mut overlapped = WindowsOverlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        offset_high: 0,
        event: std::ptr::null_mut(),
    };
    // SAFETY: unlocks the exact byte range acquired above before File is dropped.
    let _ = unsafe { UnlockFileEx(file.as_raw_handle().cast(), 0, 1, 0, &mut overlapped) };
}

#[cfg(not(any(unix, windows)))]
fn lock_file(_file: &File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "cross-process evidence locking is unsupported on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn unlock_file(_file: &File) {}

#[cfg(test)]
fn test_write_interleave_delay() {
    if let Ok(milliseconds) = std::env::var("CONTROLLA_TEST_EVIDENCE_WRITE_DELAY_MS")
        && let Ok(milliseconds) = milliseconds.parse::<u64>()
    {
        std::thread::sleep(std::time::Duration::from_millis(milliseconds.min(1000)));
    }
}

#[cfg(not(test))]
fn test_write_interleave_delay() {}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct EvidenceDatabase {
    version: u32,
    scopes: BTreeMap<String, Vec<EvidenceEntry>>,
    /// Opaque, stable retrieval handles. Handles point to a scope so replacing a
    /// scope refreshes its trail without duplicating evidence in the database.
    #[serde(default)]
    handles: BTreeMap<String, String>,
}

pub struct PersistentEvidenceStore {
    path: PathBuf,
    lock: Mutex<()>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct VisualPngMetadata {
    principal_sha256: String,
    sha256: String,
    size: usize,
    expires_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredVisualPng {
    pub handle: String,
    pub sha256: String,
    pub size: usize,
    pub expires_at_ms: u64,
}

impl PersistentEvidenceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    fn lock_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    fn lock_cross_process(&self) -> Result<CrossProcessLock, String> {
        CrossProcessLock::acquire(&self.lock_path())
    }

    fn visual_dir(&self) -> PathBuf {
        self.path.with_extension("visuals")
    }

    fn visual_png_path(&self, handle: &str) -> PathBuf {
        self.visual_dir().join(format!("{handle}.png"))
    }

    fn visual_metadata_path(&self, handle: &str) -> PathBuf {
        self.visual_dir().join(format!("{handle}.json"))
    }

    /// Save exact validated screenshot bytes outside the session artifact store.
    /// The store binds access to the configured server principal and expires after one hour.
    pub fn store_visual_png(
        &self,
        principal: &str,
        bytes: &[u8],
    ) -> Result<StoredVisualPng, String> {
        if principal.is_empty()
            || principal.len() > 256
            || bytes.is_empty()
            || bytes.len() > MAX_VISUAL_PNG_BYTES
        {
            return Err(
                "visual evidence is empty or exceeds its principal-scoped size bound".into(),
            );
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "evidence store lock poisoned")?;
        let _process_guard = self.lock_cross_process()?;
        let directory = self.visual_dir();
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        self.cleanup_visuals()?;
        let principal_sha256 = sha256_hex(principal.as_bytes());
        let mut retained_count = 0usize;
        let mut retained_bytes = 0usize;
        let mut total_count = 0usize;
        let mut total_bytes = 0usize;
        for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
                && let Ok(metadata) = read_visual_metadata(&entry.path())
            {
                total_count += 1;
                total_bytes = total_bytes.saturating_add(metadata.size);
                if metadata.principal_sha256 == principal_sha256 {
                    retained_count += 1;
                    retained_bytes = retained_bytes.saturating_add(metadata.size);
                }
            }
        }
        if retained_count >= MAX_VISUALS_PER_PRINCIPAL
            || retained_bytes.saturating_add(bytes.len()) > MAX_VISUAL_BYTES_PER_PRINCIPAL
            || total_count >= MAX_VISUALS_TOTAL
            || total_bytes.saturating_add(bytes.len()) > MAX_VISUAL_BYTES_TOTAL
        {
            return Err("principal visual evidence retention limit reached".into());
        }
        let handle = loop {
            let candidate = format!("visual_{}", new_handle()?);
            if !self.visual_png_path(&candidate).exists()
                && !self.visual_metadata_path(&candidate).exists()
            {
                break candidate;
            }
        };
        let sha256 = sha256_hex(bytes);
        let expires_at_ms = now_ms().saturating_add(VISUAL_TTL_MS);
        let metadata = VisualPngMetadata {
            principal_sha256,
            sha256: sha256.clone(),
            size: bytes.len(),
            expires_at_ms,
        };
        let png_path = self.visual_png_path(&handle);
        let metadata_path = self.visual_metadata_path(&handle);
        write_new_private(&png_path, bytes)?;
        if let Err(error) = write_new_private(
            &metadata_path,
            &serde_json::to_vec(&metadata).map_err(|error| error.to_string())?,
        ) {
            let _ = std::fs::remove_file(&png_path);
            return Err(error);
        }
        Ok(StoredVisualPng {
            handle,
            sha256,
            size: bytes.len(),
            expires_at_ms,
        })
    }

    /// Return exact bytes only when handle, principal, expiry, bounds and digest match.
    pub fn read_visual_png(&self, principal: &str, handle: &str) -> Result<Vec<u8>, String> {
        if principal.is_empty() || !valid_visual_handle(handle) {
            return Err("visual evidence not found".into());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "evidence store lock poisoned")?;
        let _process_guard = self.lock_cross_process()?;
        let metadata = read_visual_metadata(&self.visual_metadata_path(handle))?;
        if metadata.principal_sha256 != sha256_hex(principal.as_bytes())
            || metadata.expires_at_ms <= now_ms()
            || metadata.size == 0
            || metadata.size > MAX_VISUAL_PNG_BYTES
        {
            return Err("visual evidence not found or expired".into());
        }
        let bytes = std::fs::read(self.visual_png_path(handle))
            .map_err(|_| "visual evidence not found".to_owned())?;
        if bytes.len() != metadata.size || sha256_hex(&bytes) != metadata.sha256 {
            return Err("visual evidence integrity check failed".into());
        }
        Ok(bytes)
    }

    fn cleanup_visuals(&self) -> Result<(), String> {
        let directory = self.visual_dir();
        let now = now_ms();
        for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let expired = read_visual_metadata(&entry.path())
                    .map_or(true, |metadata| metadata.expires_at_ms <= now);
                if expired {
                    let _ = std::fs::remove_file(entry.path());
                    if let Some(stem) = entry.path().file_stem() {
                        let _ = std::fs::remove_file(
                            directory.join(format!("{}.png", stem.to_string_lossy())),
                        );
                    }
                }
            } else if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "png")
            {
                let metadata_path = entry.path().with_extension("json");
                if !metadata_path.exists() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn expire_visual_png_for_test(&self, handle: &str) {
        let path = self.visual_metadata_path(handle);
        let mut metadata = read_visual_metadata(&path).unwrap();
        metadata.expires_at_ms = 0;
        std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    }

    pub fn replace(&self, scope: &str, trail: &EvidenceTrail) -> Result<(), String> {
        self.replace_with_handle(scope, trail).map(|_| ())
    }

    /// Persist a trail and return its stable opaque retrieval handle.
    /// Retention follows the store's existing 128-scope policy; on overflow,
    /// lexicographically smallest scope keys are evicted. There is no time expiry.
    pub fn replace_with_handle(
        &self,
        scope: &str,
        trail: &EvidenceTrail,
    ) -> Result<String, String> {
        if scope.is_empty() || scope.len() > 128 {
            return Err("evidence scope must be a bounded opaque key".into());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "evidence store lock poisoned")?;
        let _process_guard = self.lock_cross_process()?;
        let mut database = self.read()?;
        test_write_interleave_delay();
        database
            .scopes
            .insert(scope.to_owned(), trail.recent(MAX_TASK_EVIDENCE_ENTRIES));
        let handle = database
            .handles
            .iter()
            .find_map(|(handle, mapped_scope)| (mapped_scope == scope).then(|| handle.clone()))
            .unwrap_or_else(String::new);
        let handle = if !handle.is_empty() {
            handle
        } else {
            loop {
                let candidate = new_handle()?;
                if !database.handles.contains_key(&candidate) {
                    break candidate;
                }
            }
        };
        database.handles.insert(handle.clone(), scope.to_owned());
        // Never evict the scope this call just wrote. A saturated store plus a
        // scope that sorts first would otherwise discard the caller's evidence
        // in the same transaction and return a handle that resolves to nothing.
        while database.scopes.len() > MAX_SCOPES {
            let Some(evicted) = database
                .scopes
                .keys()
                .find(|key| key.as_str() != scope)
                .cloned()
            else {
                break;
            };
            database.scopes.remove(&evicted);
            database
                .handles
                .retain(|_, mapped_scope| mapped_scope != &evicted);
        }
        let encoded = serde_json::to_vec(&database).map_err(|error| error.to_string())?;
        if encoded.len() > MAX_STORE_BYTES {
            return Err("evidence store reached its total byte budget".into());
        }
        self.write_atomic(&encoded)?;
        Ok(handle)
    }

    /// Retrieve at most 16 persisted entries for a previously issued handle.
    /// Unknown, malformed, or evicted handles return an empty result.
    pub fn lookup(&self, handle: &str, limit: usize) -> Vec<EvidenceEntry> {
        self.lookup_page(handle, 0, limit).0
    }

    /// Retrieve one bounded page and total count for a bearer handle.
    pub fn lookup_page(
        &self,
        handle: &str,
        offset: usize,
        limit: usize,
    ) -> (Vec<EvidenceEntry>, usize) {
        if !valid_handle(handle) {
            return (Vec::new(), 0);
        }
        let Ok(_guard) = self.lock.lock() else {
            return (Vec::new(), 0);
        };
        let Ok(_process_guard) = self.lock_cross_process() else {
            return (Vec::new(), 0);
        };
        let Ok(database) = self.read() else {
            return (Vec::new(), 0);
        };
        let Some(entries) = database
            .handles
            .get(handle)
            .and_then(|scope| database.scopes.get(scope))
        else {
            return (Vec::new(), 0);
        };
        let start = offset.min(entries.len());
        let end = start.saturating_add(limit.min(16)).min(entries.len());
        (entries[start..end].to_vec(), entries.len())
    }

    pub fn recent_relevant(&self, scope: &str, intent: &str, limit: usize) -> Vec<EvidenceEntry> {
        let Ok(_guard) = self.lock.lock() else {
            return Vec::new();
        };
        let Ok(_process_guard) = self.lock_cross_process() else {
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
        let Ok(_process_guard) = self.lock_cross_process() else {
            return Vec::new();
        };
        let Ok(database) = self.read() else {
            return Vec::new();
        };
        database
            .scopes
            .get(scope)
            .map(|entries| bounded_recent(entries, limit))
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
        if !matches!(database.version, 1 | STORE_VERSION)
            || database.scopes.len() > MAX_SCOPES
            || database.handles.len() > MAX_SCOPES
            || database.scopes.iter().any(|(scope, entries)| {
                scope.is_empty() || scope.len() > 128 || !valid_entries(entries)
            })
            || database.handles.iter().any(|(handle, scope)| {
                !valid_handle(handle) || !database.scopes.contains_key(scope)
            })
        {
            return Err("evidence store failed schema or bounds validation".into());
        }
        // Version 1 databases had no retrieval-handle index. Existing trails
        // remain available for in-scope recent reads; a random bearer handle is
        // issued the next time the scope is updated.
        if database.version == 1 {
            database.handles.clear();
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

fn new_handle() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("cannot create evidence handle: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn valid_visual_handle(handle: &str) -> bool {
    handle.strip_prefix("visual_").is_some_and(valid_handle)
}

fn read_visual_metadata(path: &Path) -> Result<VisualPngMetadata, String> {
    let bytes = std::fs::read(path).map_err(|_| "visual evidence not found".to_owned())?;
    if bytes.len() > 1024 {
        return Err("visual evidence metadata exceeds its bound".into());
    }
    let metadata: VisualPngMetadata = serde_json::from_slice(&bytes)
        .map_err(|_| "visual evidence metadata is corrupt".to_owned())?;
    if metadata.size == 0
        || metadata.size > MAX_VISUAL_PNG_BYTES
        || metadata.sha256.len() != 64
        || metadata.principal_sha256.len() != 64
    {
        return Err("visual evidence metadata failed bounds validation".into());
    }
    Ok(metadata)
}

fn write_new_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(path);
        return Err(error.to_string());
    }
    Ok(())
}

fn valid_handle(handle: &str) -> bool {
    handle.len() == 32 && handle.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn bounded_recent(entries: &[EvidenceEntry], limit: usize) -> Vec<EvidenceEntry> {
    entries
        .iter()
        .rev()
        .take(limit.min(16))
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
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
                && entry.payload.as_ref().is_none_or(|payload| {
                    serde_json::to_vec(payload).is_ok_and(|bytes| {
                        bytes.len() <= crate::visual_probe::MAX_EVIDENCE_PAYLOAD_BYTES
                    })
                })
                && match (&entry.payload, &entry.payload_sha256) {
                    (None, None) => true,
                    (Some(payload), Some(expected)) => {
                        serde_json::to_vec(payload).is_ok_and(|bytes| {
                            let actual = sha2::Sha256::digest(bytes)
                                .iter()
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>();
                            expected == &actual
                        })
                    }
                    _ => false,
                }
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
    use std::sync::atomic::{AtomicU64, Ordering};

    fn store_path() -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "controlla-evidence-store-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn trail(count: u32) -> EvidenceTrail {
        let mut trail = EvidenceTrail::default();
        for step in 0..count {
            trail.record(
                "observation",
                Some(format!("ref-{step}")),
                None,
                format!("verified fact {step}"),
                true,
            );
        }
        trail
    }

    #[test]
    fn scopes_are_opaque_and_stable_without_exposing_site_or_intent() {
        let a = scope_key("https://example.test", "invoice-search");
        assert_eq!(a, scope_key("https://example.test", "invoice-search"));
        assert_ne!(a, scope_key("https://example.test", "customer-search"));
        assert_eq!(a.len(), 64);
        assert!(!a.contains("example"));
    }

    #[test]
    fn opaque_handle_looks_up_persisted_trail_after_restart() {
        let path = store_path();
        let scope = scope_key("https://example.test", "invoice-search");
        let handle = PersistentEvidenceStore::new(&path)
            .replace_with_handle(&scope, &trail(3))
            .unwrap();
        assert_eq!(handle.len(), 32);
        assert!(!handle.contains("example"));
        let restarted = PersistentEvidenceStore::new(&path);
        let entries = restarted.lookup(&handle, 8);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].fact, "verified fact 0");
        assert_eq!(entries[2].fact, "verified fact 2");
        assert_eq!(
            restarted.replace_with_handle(&scope, &trail(4)).unwrap(),
            handle
        );
        assert_eq!(restarted.lookup(&handle, 16).len(), 4);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn payload_round_trips_with_digest_and_tampering_fails_closed() {
        let path = store_path();
        let scope = scope_key("https://example.test", "review");
        let mut trail = EvidenceTrail::default();
        trail.record_with_payload(
            "snapshot",
            Some("@c1".into()),
            None,
            "button Continue",
            false,
            Some(serde_json::json!({"nodes":[{"reference":"@c1","name":"Continue"}]})),
        );
        let store = PersistentEvidenceStore::new(&path);
        let handle = store.replace_with_handle(&scope, &trail).unwrap();
        let restored = PersistentEvidenceStore::new(&path).lookup(&handle, 4);
        assert_eq!(
            restored[0].payload.as_ref().unwrap()["nodes"][0]["name"],
            "Continue"
        );
        let expected = restored[0].payload_sha256.as_ref().unwrap();
        let actual = sha2::Sha256::digest(
            serde_json::to_vec(restored[0].payload.as_ref().unwrap()).unwrap(),
        )
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
        assert_eq!(expected, &actual);

        let mut database: EvidenceDatabase =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        database.scopes.get_mut(&scope).unwrap()[0]
            .payload
            .as_mut()
            .unwrap()["nodes"][0]["name"] = serde_json::json!("tampered");
        std::fs::write(&path, serde_json::to_vec(&database).unwrap()).unwrap();
        assert!(
            PersistentEvidenceStore::new(&path)
                .lookup(&handle, 4)
                .is_empty()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persistence_failure_does_not_issue_a_retrieval_handle() {
        let parent = store_path();
        std::fs::write(&parent, b"not a directory").unwrap();
        let path = parent.join("evidence.json");
        let scope = scope_key("https://example.test", "failure");
        assert!(
            PersistentEvidenceStore::new(path)
                .replace_with_handle(&scope, &trail(1))
                .is_err()
        );
        let _ = std::fs::remove_file(parent);
    }

    #[test]
    fn cross_process_writes_keep_each_scope_and_handle() {
        if let Ok(scope) = std::env::var("CONTROLLA_EVIDENCE_CHILD_SCOPE") {
            let path = PathBuf::from(std::env::var_os("CONTROLLA_EVIDENCE_CHILD_DB").unwrap());
            let start = PathBuf::from(std::env::var_os("CONTROLLA_EVIDENCE_CHILD_START").unwrap());
            let ready = PathBuf::from(std::env::var_os("CONTROLLA_EVIDENCE_CHILD_READY").unwrap());
            std::fs::write(ready, b"ready").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !start.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(start.exists(), "parent did not release concurrent writers");
            let handle = PersistentEvidenceStore::new(path)
                .replace_with_handle(&scope, &trail(1))
                .unwrap();
            assert_eq!(handle.len(), 32);
            return;
        }

        const WRITERS: usize = 8;
        let db = store_path();
        let start = db.with_extension("start");
        let ready_dir = db.with_extension("ready");
        std::fs::create_dir_all(&ready_dir).unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut children = Vec::new();
        for index in 0..WRITERS {
            let mut command = std::process::Command::new(&executable);
            command
                .arg("--exact")
                .arg("evidence_store::tests::cross_process_writes_keep_each_scope_and_handle")
                .arg("--nocapture")
                .env(
                    "CONTROLLA_EVIDENCE_CHILD_SCOPE",
                    format!("scope-{index:03}"),
                )
                .env("CONTROLLA_EVIDENCE_CHILD_DB", &db)
                .env("CONTROLLA_EVIDENCE_CHILD_START", &start)
                .env(
                    "CONTROLLA_EVIDENCE_CHILD_READY",
                    ready_dir.join(index.to_string()),
                )
                .env("CONTROLLA_TEST_EVIDENCE_WRITE_DELAY_MS", "200");
            children.push(command.spawn().unwrap());
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while (0..WRITERS).any(|index| !ready_dir.join(index.to_string()).exists())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            (0..WRITERS).all(|index| ready_dir.join(index.to_string()).exists()),
            "writers did not all become ready"
        );
        std::fs::write(&start, b"go").unwrap();
        for mut child in children {
            assert!(child.wait().unwrap().success(), "writer subprocess failed");
        }

        let database = PersistentEvidenceStore::new(&db).read().unwrap();
        assert_eq!(
            database.scopes.len(),
            WRITERS,
            "every distinct scope write must survive"
        );
        assert_eq!(
            database.handles.len(),
            WRITERS,
            "every persisted scope must keep its handle"
        );
        for index in 0..WRITERS {
            let scope = format!("scope-{index:03}");
            assert!(database.scopes.contains_key(&scope), "missing {scope}");
            assert!(
                database.handles.values().any(|mapped| mapped == &scope),
                "missing handle for {scope}"
            );
        }
        let _ = std::fs::remove_file(&db);
        let _ = std::fs::remove_file(format!("{}.lock", db.display()));
        let _ = std::fs::remove_file(start);
        let _ = std::fs::remove_dir_all(ready_dir);
    }

    #[test]
    fn unknown_and_cross_scope_handles_do_not_reveal_other_trails() {
        let path = store_path();
        let store = PersistentEvidenceStore::new(&path);
        let first_scope = scope_key("https://one.test", "search");
        let second_scope = scope_key("https://two.test", "search");
        let first = store.replace_with_handle(&first_scope, &trail(1)).unwrap();
        let second = store.replace_with_handle(&second_scope, &trail(2)).unwrap();
        assert_ne!(first, second);
        assert_eq!(store.lookup(&first, 16).len(), 1);
        assert_eq!(store.lookup(&second, 16).len(), 2);
        assert!(store.lookup(&"f".repeat(64), 16).is_empty());
        assert!(store.lookup("malformed", 16).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lookup_applies_entry_and_limit_bounds() {
        let path = store_path();
        let store = PersistentEvidenceStore::new(&path);
        let scope = scope_key("https://example.test", "bounded-search");
        let handle = store.replace_with_handle(&scope, &trail(40)).unwrap();
        assert_eq!(store.lookup(&handle, usize::MAX).len(), 16);
        assert_eq!(store.lookup(&handle, 2).len(), 2);
        assert!(store.lookup(&handle, 0).is_empty());
        let (page, total) = store.lookup_page(&handle, 4, 4);
        assert_eq!(page.len(), 4);
        assert_eq!(total, 16);
        assert_eq!(page[0].step, 28);
        let (last_page, total) = store.lookup_page(&handle, 15, 4);
        assert_eq!(last_page.len(), 1);
        assert_eq!(total, 16);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn handles_follow_existing_scope_eviction_policy() {
        let path = store_path();
        let store = PersistentEvidenceStore::new(&path);
        let evicted_scope = "0".repeat(64);
        let retained_scope = "z".repeat(64);
        let evicted = store
            .replace_with_handle(&evicted_scope, &trail(1))
            .unwrap();
        for index in 0..MAX_SCOPES {
            store
                .replace_with_handle(&format!("scope-{index:03}"), &trail(1))
                .unwrap();
        }
        let retained = store
            .replace_with_handle(&retained_scope, &trail(1))
            .unwrap();
        assert!(store.lookup(&evicted, 1).is_empty());
        assert_eq!(store.lookup(&retained, 1).len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn visual_png_handle_is_principal_scoped_durable_bounded_and_integrity_checked() {
        let path = store_path();
        let store = PersistentEvidenceStore::new(&path);
        let png = b"\x89PNG\r\n\x1a\nvalidated fixture";
        let stored = store.store_visual_png("principal-a", png).unwrap();
        assert_eq!(stored.size, png.len());
        assert_eq!(
            store
                .read_visual_png("principal-a", &stored.handle)
                .unwrap(),
            png
        );
        assert!(
            store
                .read_visual_png("principal-b", &stored.handle)
                .is_err()
        );

        let reopened = PersistentEvidenceStore::new(&path);
        assert_eq!(
            reopened
                .read_visual_png("principal-a", &stored.handle)
                .unwrap(),
            png
        );
        let png_path = reopened.visual_png_path(&stored.handle);
        std::fs::write(png_path, b"corrupt").unwrap();
        assert!(
            reopened
                .read_visual_png("principal-a", &stored.handle)
                .is_err()
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(reopened.visual_dir());
    }

    #[test]
    fn visual_png_storage_rejects_oversize_and_expired_handles() {
        let path = store_path();
        let store = PersistentEvidenceStore::new(&path);
        assert!(
            store
                .store_visual_png("principal-a", &vec![0; MAX_VISUAL_PNG_BYTES + 1])
                .is_err()
        );
        let stored = store.store_visual_png("principal-a", b"png").unwrap();
        store.expire_visual_png_for_test(&stored.handle);
        assert!(
            store
                .read_visual_png("principal-a", &stored.handle)
                .is_err()
        );
        for _ in 0..MAX_VISUALS_PER_PRINCIPAL {
            store.store_visual_png("principal-b", b"png").unwrap();
        }
        assert!(store.store_visual_png("principal-b", b"png").is_err());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(store.visual_dir());
    }
}
