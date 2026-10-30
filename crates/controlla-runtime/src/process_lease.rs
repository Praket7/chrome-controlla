//! Cross-process lease client for real Chrome mutations.
//! Lease key: (browser_instance, target_id, document_epoch, account_epoch, effect_class)

use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long past its recorded TTL a lease stays authoritative. A holder
/// refreshes the record while it acts, so only an abandoned lease passes this.
const LEASE_EXPIRY_GRACE_SECONDS: u64 = 5;

/// Published ownership for a held lock, on disk so another process can judge
/// whether the recorded owner generation is still live before waiting on it.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct OwnerRecord {
    schema_version: u32,
    pid: u32,
    process_generation: String,
    state_dir: String,
    owner: String,
    expires_unix_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LeaseKey {
    pub browser_instance: String,
    pub target_id: String,
    pub document_epoch: String,
    pub account_epoch: String,
    pub effect_class: String,
}

#[derive(Clone, Debug)]
pub struct Lease {
    pub key: LeaseKey,
    pub owner: String,
    pub expires_at: Instant,
    pub effect: String,
}

struct HeldLease {
    lease: Lease,
    _file: File,
    readers: Option<BTreeSet<String>>,
    unknown: bool,
    generation: u64,
}

#[derive(Clone)]
pub struct LeaseCoordinator {
    inner: Arc<Mutex<HashMap<LeaseKey, HeldLease>>>,
    lock_dir: Arc<PathBuf>,
    state_dir: Arc<PathBuf>,
    next_generation: Arc<std::sync::atomic::AtomicU64>,
    changed: Arc<tokio::sync::Notify>,
    takeovers: Arc<Mutex<HashMap<LeaseKey, String>>>,
}

impl Default for LeaseCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl LeaseCoordinator {
    pub fn new() -> Self {
        let lock_dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir)
            .join(".controlla")
            .join("target-leases");
        Self::with_lock_dir(lock_dir)
    }

    pub fn with_lock_dir(lock_dir: impl Into<PathBuf>) -> Self {
        Self::with_lock_and_state_dir(lock_dir, state_directory())
    }

    pub fn with_lock_and_state_dir(
        lock_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            lock_dir: Arc::new(lock_dir.into()),
            state_dir: Arc::new(state_dir.into()),
            next_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            changed: Arc::new(tokio::sync::Notify::new()),
            takeovers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A takeover that this process performed for `key`, if any. Reading it
    /// clears the note so one admission is reported by one receipt.
    pub fn take_takeover(&self, key: &LeaseKey) -> Option<String> {
        self.takeovers.lock().unwrap().remove(key)
    }

    /// Register before attempting acquisition, then keep a bounded polling
    /// fallback for releases in another process (which cannot notify us).
    pub fn changed(&self) -> tokio::sync::futures::Notified<'_> {
        self.changed.notified()
    }

    /// Try to acquire a target lease. OS file locks exclude other MCP processes;
    /// the in-memory map preserves owner details and same-process read sharing.
    pub fn try_acquire(
        &self,
        key: LeaseKey,
        owner: &str,
        ttl: Duration,
        is_read: bool,
    ) -> Result<Lease, String> {
        let mut map = self.inner.lock().unwrap();
        // A lease whose TTL has passed cannot still have an effect in flight, so
        // it must not keep blocking. Without this, a client that dies while
        // holding the target leaves every later call waiting on a lease that no
        // live process owns until someone restarts the whole server.
        if map
            .get(&key)
            .is_some_and(|held| Instant::now() >= held.lease.expires_at)
        {
            // Dropping the entry closes its lock file, which releases the OS
            // lock this process held for it.
            map.remove(&key);
        }
        if map.contains_key(&key) {
            if is_read
                && let Some(readers) = map.get_mut(&key).and_then(|held| held.readers.as_mut())
            {
                readers.insert(owner.to_owned());
                let mut shared = map.get(&key).unwrap().lease.clone();
                shared.owner = owner.to_owned();
                return Ok(shared);
            }
            // The same holder resuming after an unknown outcome or a long action
            // re-enters its own lease instead of waiting out the retention: the
            // effect was already fenced against other processes, and this
            // process is the one the receipt told to inspect before retrying.
            if !is_read {
                let same_owner = map.get(&key).is_some_and(|held| held.lease.owner == owner);
                if same_owner {
                    let path = self
                        .lock_dir
                        .join(format!("{}.lock", lease_key_digest(&key)));
                    write_owner_record(&path, &self.state_dir, &key, owner, ttl);
                    let held = map.get_mut(&key).unwrap();
                    held.lease.expires_at = Instant::now() + ttl;
                    held.unknown = false;
                    return Ok(held.lease.clone());
                }
            }
            let existing = map.get(&key).unwrap();
            return Err(format!("lease held by {}", existing.lease.owner));
        }

        std::fs::create_dir_all(self.lock_dir.as_path())
            .map_err(|error| format!("unable to create target lock directory: {error}"))?;
        restrict_lock_directory(self.lock_dir.as_path())
            .map_err(|error| format!("unable to secure target lock directory: {error}"))?;
        let path = self
            .lock_dir
            .join(format!("{}.lock", lease_key_digest(&key)));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("unable to open target lock file: {error}"))?;
        let lock_result = if is_read {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        let mut file = file;
        match lock_result {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                // A lock whose recorded owner generation is absent from the live
                // runtime records belongs to a process that can no longer finish
                // the effect. Take it over immediately instead of blocking behind
                // it until the lease expiry, and remember why.
                let Some(reason) = self.stale_owner(&key) else {
                    return Err("lease held by another MCP process".to_owned());
                };
                let _ = std::fs::remove_file(&path);
                let refreshed = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&path)
                    .map_err(|error| format!("unable to open target lock file: {error}"))?;
                let retry = if is_read {
                    refreshed.try_lock_shared()
                } else {
                    refreshed.try_lock()
                };
                match retry {
                    Ok(()) => {
                        file = refreshed;
                        self.takeovers.lock().unwrap().insert(key.clone(), reason);
                    }
                    Err(_) => return Err("lease held by another MCP process".to_owned()),
                }
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(format!("unable to acquire target lock: {error}"));
            }
        }
        write_owner_record(&path, &self.state_dir, &key, owner, ttl);

        let lease = Lease {
            key: key.clone(),
            owner: owner.to_owned(),
            expires_at: Instant::now() + ttl,
            effect: if is_read {
                "read".into()
            } else {
                "mutation".into()
            },
        };
        let generation = self
            .next_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        map.insert(
            key,
            HeldLease {
                lease: lease.clone(),
                _file: file,
                readers: is_read.then(|| BTreeSet::from([owner.to_owned()])),
                unknown: false,
                generation,
            },
        );
        Ok(lease)
    }

    /// Return a human-readable reason when the recorded owner of `key` is no
    /// longer a live runtime generation, otherwise `None`.
    ///
    /// The record is written by the holder when it acquires the lock. A live
    /// generation keeps `stdio-runtime.json` fresh; a crashed or replaced
    /// process does not. The heartbeat grace is longer than the publish
    /// interval, so a normal write pause is not mistaken for death.
    fn stale_owner(&self, key: &LeaseKey) -> Option<String> {
        let path = self
            .lock_dir
            .join(format!("{}.lock.owner.json", lease_key_digest(key)));
        let bytes = std::fs::read(&path).ok()?;
        let record: OwnerRecord = serde_json::from_slice(&bytes).ok()?;
        if record.pid == std::process::id() {
            return None;
        }
        // A lease's own TTL bounds how long its holder may act on the target.
        // Once that TTL has passed (with a small grace for clock skew), the
        // record is stale even when the process is still running: a live but
        // idle holder that retained an unknown write must not block every other
        // MCP process until someone restarts it.
        let now = unix_now();
        let expired = now
            > record
                .expires_unix_seconds
                .saturating_add(LEASE_EXPIRY_GRACE_SECONDS);
        let expiry_reason = || {
            format!(
                "recorded owner {} (pid {}) lease expired {}s ago in {}",
                record.owner,
                record.pid,
                now.saturating_sub(record.expires_unix_seconds),
                record.state_dir
            )
        };
        let alive = process_alive(record.pid);
        if alive {
            // No runtime record is not proof of death: a live process that never
            // publishes one (library use, tests) must keep its lease.
            let runtime = runtime_record(&record.state_dir)?;
            let service_pid = runtime
                .get("service_pid")
                .and_then(|pid| pid.as_u64())
                .and_then(|pid| u32::try_from(pid).ok());
            let generation = runtime
                .get("process_generation")
                .and_then(|value| value.as_str());
            // Only the process that published the record can be judged by it. A
            // record owned by a different pid says nothing about this live
            // holder, so the lease stays with it.
            if service_pid != Some(record.pid) {
                return None;
            }
            if generation == Some(record.process_generation.as_str()) {
                return expired.then(expiry_reason);
            }
        }
        Some(format!(
            "recorded owner generation {} (pid {}) is absent from the live runtime records in {}",
            record.process_generation, record.pid, record.state_dir
        ))
    }

    pub fn release(&self, key: &LeaseKey, owner: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        let Some(held) = map.get_mut(key) else {
            return false;
        };
        if let Some(readers) = held.readers.as_mut() {
            if !readers.remove(owner) {
                return false;
            }
            if readers.is_empty() {
                map.remove(key);
                self.changed.notify_waiters();
            }
            return true;
        }
        if held.lease.owner == owner {
            map.remove(key);
            self.changed.notify_waiters();
            true
        } else {
            false
        }
    }

    pub fn retain_on_unknown(&self, key: &LeaseKey, owner: &str) {
        self.retain_on_unknown_for(key.clone(), owner.to_owned(), Duration::from_secs(120));
    }

    /// Release a retained lock only after the caller explicitly reconciles the
    /// uncertain mutation with fresh read-only evidence.
    ///
    /// A lease whose TTL has already passed is released too: the retention
    /// window is what makes reconciliation necessary in the first place, so a
    /// lease that outlived it must not keep the target unusable forever.
    pub fn release_unknown(&self, key: &LeaseKey) -> bool {
        let mut map = self.inner.lock().unwrap();
        let now = Instant::now();
        if map
            .get(key)
            .is_some_and(|held| held.unknown || now >= held.lease.expires_at)
        {
            map.remove(key);
            self.changed.notify_waiters();
            true
        } else {
            false
        }
    }

    fn retain_on_unknown_for(&self, key: LeaseKey, owner: String, hold: Duration) {
        // Unknown dispatches need a reconciliation window. The OS lock remains
        // held during that window, then a timer releases it if the caller does
        // not explicitly resolve the outcome first.
        let generation = {
            let mut map = self.inner.lock().unwrap();
            if let Some(held) = map.get_mut(&key)
                && held.lease.owner == owner
            {
                held.unknown = true;
                Some(held.generation)
            } else {
                None
            }
        };
        let Some(generation) = generation else {
            return;
        };
        let coordinator = self.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                tokio::time::sleep(hold).await;
                coordinator.release_generation(&key, &owner, generation);
            });
        }
    }

    fn release_generation(&self, key: &LeaseKey, owner: &str, generation: u64) -> bool {
        let mut map = self.inner.lock().unwrap();
        // The generation identifies the exact hold. Do not also require the
        // owner string to match: a nested reacquire can rewrite the visible
        // owner while the same hold owns the OS lock, and a failed match would
        // leave that lock held with no owner able to release it.
        let _ = owner;
        if map
            .get(key)
            .is_some_and(|held| held.generation == generation)
        {
            map.remove(key);
            self.changed.notify_waiters();
            true
        } else {
            false
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn state_directory() -> PathBuf {
    std::env::var_os("CONTROLLA_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".chrome-controlla"))
        })
        .unwrap_or_else(|| PathBuf::from(".chrome-controlla"))
}

fn write_owner_record(
    lock_path: &Path,
    state_dir: &Path,
    key: &LeaseKey,
    owner: &str,
    ttl: Duration,
) {
    let now = unix_now();
    let record = OwnerRecord {
        schema_version: 1,
        pid: std::process::id(),
        process_generation: crate::doctor::process_generation().to_owned(),
        state_dir: state_dir.display().to_string(),
        owner: owner.to_owned(),
        expires_unix_seconds: now.saturating_add(ttl.as_secs()),
    };
    let path = lock_path.with_extension("lock.owner.json");
    if let Ok(bytes) = serde_json::to_vec(&record) {
        let _ = std::fs::write(path, bytes);
    }
    let _ = key;
}

fn runtime_record(state_dir: &str) -> Option<serde_json::Value> {
    let bytes = std::fs::read(Path::new(state_dir).join("stdio-runtime.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // Signal 0 tests existence without delivering a signal. EPERM still means
    // the process exists; it just belongs to another user.
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    // Without a cheap existence probe, rely on the published heartbeat alone.
    true
}

fn lease_key_digest(key: &LeaseKey) -> String {
    let mut hasher = Sha256::new();
    for value in [
        &key.browser_instance,
        &key.target_id,
        &key.document_epoch,
        &key.account_epoch,
        &key.effect_class,
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn restrict_lock_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn released_lease_wakes_waiters_before_reconciliation_poll() {
        let coord = LeaseCoordinator::with_lock_dir(test_dir());
        coord
            .try_acquire(key(), "owner", Duration::from_secs(10), false)
            .unwrap();
        let changed = coord.changed();
        tokio::pin!(changed);
        changed.as_mut().enable();
        assert!(coord.release(&key(), "owner"));
        tokio::time::timeout(Duration::from_millis(100), changed)
            .await
            .expect("release did not notify waiters");
    }

    fn key() -> LeaseKey {
        LeaseKey {
            browser_instance: "b".into(),
            target_id: "t".into(),
            document_epoch: "d".into(),
            account_epoch: "a".into(),
            effect_class: "mutation".into(),
        }
    }

    #[test]
    fn read_read_may_share_mutation_serializes() {
        let coord = LeaseCoordinator::with_lock_dir(test_dir());
        let key = key();
        let r1 = coord.try_acquire(key.clone(), "c1", Duration::from_secs(10), true);
        assert!(r1.is_ok());
        let r2 = coord.try_acquire(key.clone(), "c2", Duration::from_secs(10), true);
        assert!(r2.is_ok());

        let key = LeaseKey {
            browser_instance: "b2".into(),
            target_id: "t2".into(),
            ..key
        };
        let m1 = coord
            .try_acquire(key.clone(), "c1", Duration::from_secs(10), false)
            .unwrap();
        assert!(
            coord
                .try_acquire(key.clone(), "c2", Duration::from_secs(10), false)
                .is_err()
        );
        assert!(coord.release(&key, "c1"));
        assert!(
            coord
                .try_acquire(key, "c2", Duration::from_secs(10), false)
                .is_ok()
        );
        let _ = m1;
    }

    /// A crashed holder leaves an owner record behind but its OS lock is
    /// released by the kernel, so a stale record alone must never block
    /// acquisition. This is the cross-platform contract.
    #[test]
    fn a_stale_owner_record_does_not_block_acquisition() {
        let dir = test_dir();
        let state = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let coord = LeaseCoordinator::with_lock_and_state_dir(&dir, &state);
        let key = key();
        let path = dir.join(format!("{}.lock", lease_key_digest(&key)));
        // No runtime record exists for this state directory, so the recorded
        // generation is absent from the live records and the lease is dead.
        let record = OwnerRecord {
            schema_version: 1,
            pid: 4_000_000,
            process_generation: "dead-generation".into(),
            state_dir: state.display().to_string(),
            owner: "crashed-client".into(),
            expires_unix_seconds: 0,
        };
        std::fs::write(
            path.with_extension("lock.owner.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let lease = coord
            .try_acquire(
                key.clone(),
                "recovering-owner",
                Duration::from_secs(10),
                false,
            )
            .expect("a stale owner record must not block acquisition");
        assert_eq!(lease.owner, "recovering-owner");
        let written: OwnerRecord =
            serde_json::from_slice(&std::fs::read(path.with_extension("lock.owner.json")).unwrap())
                .unwrap();
        assert_eq!(written.owner, "recovering-owner");
        assert_eq!(written.pid, std::process::id());
    }

    /// POSIX can replace a lock file that another process still has open, so a
    /// dead generation can be taken over even while its stale handle is alive.
    /// Windows reports a sharing violation instead; there the kernel releases a
    /// crashed holder's lock, which the test above covers.
    #[cfg(unix)]
    #[test]
    fn dead_owner_generation_is_taken_over_immediately() {
        let dir = test_dir();
        let state = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let coord = LeaseCoordinator::with_lock_and_state_dir(&dir, &state);
        let key = key();
        let path = dir.join(format!("{}.lock", lease_key_digest(&key)));
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        held.try_lock().unwrap();
        // No runtime record exists for this state directory, so the recorded
        // generation is absent from the live records and the lease is dead.
        let record = OwnerRecord {
            schema_version: 1,
            pid: 4_000_000,
            process_generation: "dead-generation".into(),
            state_dir: state.display().to_string(),
            owner: "crashed-client".into(),
            expires_unix_seconds: 0,
        };
        std::fs::write(
            path.with_extension("lock.owner.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let lease = coord
            .try_acquire(
                key.clone(),
                "recovering-owner",
                Duration::from_secs(10),
                false,
            )
            .expect("a dead owner generation must not block acquisition");
        let takeover = coord
            .take_takeover(&key)
            .expect("takeover must be reported");
        assert!(
            takeover.contains("absent from the live runtime records"),
            "{takeover}"
        );
        assert_eq!(lease.owner, "recovering-owner");
    }

    #[test]
    fn live_owner_generation_is_not_taken_over() {
        let dir = test_dir();
        let state = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        // A live runtime record owned by the recorded pid and generation.
        std::fs::write(
            state.join("stdio-runtime.json"),
            serde_json::to_vec(&serde_json::json!({
                "service_pid": 1,
                "process_generation": crate::doctor::process_generation()
            }))
            .unwrap(),
        )
        .unwrap();
        let coord = LeaseCoordinator::with_lock_and_state_dir(&dir, &state);
        let key = key();
        let path = dir.join(format!("{}.lock", lease_key_digest(&key)));
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        held.try_lock().unwrap();
        let record = OwnerRecord {
            schema_version: 1,
            // A different, live pid: the published generation is what proves it.
            pid: 1,
            process_generation: crate::doctor::process_generation().into(),
            state_dir: state.display().to_string(),
            owner: "live-owner".into(),
            expires_unix_seconds: unix_now() + 120,
        };
        std::fs::write(
            path.with_extension("lock.owner.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(
            coord
                .try_acquire(key.clone(), "waiting-owner", Duration::from_secs(10), false)
                .is_err(),
            "a live owner generation must keep its lease"
        );
        assert!(coord.take_takeover(&key).is_none());
    }

    #[test]
    fn expired_lease_of_a_live_process_is_taken_over() {
        // A live process that retained an unknown write and never reconciled it
        // must not block every other MCP process forever. Its own record says
        // when its lease stops being authoritative.
        let dir = test_dir();
        let state = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        // The holder is alive and its published generation still matches, so
        // only the lease expiry can free the target.
        std::fs::write(
            state.join("stdio-runtime.json"),
            serde_json::to_vec(&serde_json::json!({
                "service_pid": 1,
                "process_generation": "expired-generation"
            }))
            .unwrap(),
        )
        .unwrap();
        let coord = LeaseCoordinator::with_lock_and_state_dir(&dir, &state);
        let key = key();
        let path = dir.join(format!("{}.lock", lease_key_digest(&key)));
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        held.try_lock().unwrap();
        let record = OwnerRecord {
            schema_version: 1,
            pid: 1,
            process_generation: "expired-generation".into(),
            state_dir: state.display().to_string(),
            owner: "retained-unknown".into(),
            expires_unix_seconds: unix_now().saturating_sub(60),
        };
        std::fs::write(
            path.with_extension("lock.owner.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        coord
            .try_acquire(key.clone(), "waiting-owner", Duration::from_secs(10), false)
            .expect("an expired lease must be taken over even while its process lives");
        let takeover = coord
            .take_takeover(&key)
            .expect("takeover must be reported as admission evidence");
        assert!(takeover.contains("expired"), "{takeover}");
    }

    #[tokio::test]
    async fn reconciliation_releases_an_expired_retained_lease() {
        let coord = LeaseCoordinator::with_lock_dir(test_dir());
        let key = key();
        coord
            .try_acquire(key.clone(), "p1", Duration::from_millis(1), false)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(coord.release_unknown(&key));
        assert!(
            coord
                .try_acquire(key, "p2", Duration::from_secs(10), false)
                .is_ok()
        );
    }

    #[test]
    fn independent_coordinators_share_the_same_os_target_lock() {
        let dir = test_dir();
        let process_a = LeaseCoordinator::with_lock_dir(&dir);
        let process_b = LeaseCoordinator::with_lock_dir(&dir);
        let key = key();
        process_a
            .try_acquire(key.clone(), "a", Duration::from_secs(10), false)
            .unwrap();
        assert!(
            process_b
                .try_acquire(key.clone(), "b", Duration::from_secs(10), false)
                .is_err()
        );
        assert!(process_a.release(&key, "a"));
        assert!(
            process_b
                .try_acquire(key, "b", Duration::from_secs(10), false)
                .is_ok()
        );
    }

    #[test]
    fn unknown_retains_lease() {
        let coord = LeaseCoordinator::with_lock_dir(test_dir());
        let key = key();
        coord
            .try_acquire(key.clone(), "p1", Duration::from_secs(60), false)
            .unwrap();
        coord.retain_on_unknown(&key, "p1");
        assert!(
            coord
                .try_acquire(key, "p2", Duration::from_secs(10), false)
                .is_err()
        );
    }

    #[tokio::test]
    async fn unknown_retention_releases_after_bounded_hold() {
        let coord = LeaseCoordinator::with_lock_dir(test_dir());
        let key = key();
        coord
            .try_acquire(key.clone(), "p1", Duration::from_secs(60), false)
            .unwrap();
        let released = coord.changed();
        tokio::pin!(released);
        released.as_mut().enable();
        coord.retain_on_unknown_for(key.clone(), "p1".into(), Duration::from_millis(20));
        assert!(
            coord
                .try_acquire(key.clone(), "p2", Duration::from_secs(10), false)
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(2), released)
            .await
            .expect("unknown hold did not expire and notify waiters");
        assert!(
            coord
                .try_acquire(key, "p2", Duration::from_secs(10), false)
                .is_ok()
        );
    }

    #[tokio::test]
    async fn explicit_reconciliation_releases_only_an_unknown_lock() {
        let coord = LeaseCoordinator::with_lock_dir(test_dir());
        let key = key();
        coord
            .try_acquire(key.clone(), "p1", Duration::from_secs(60), false)
            .unwrap();
        assert!(!coord.release_unknown(&key));
        coord.retain_on_unknown_for(key.clone(), "p1".into(), Duration::from_secs(60));
        assert!(coord.release_unknown(&key));
        assert!(
            coord
                .try_acquire(key, "p2", Duration::from_secs(10), false)
                .is_ok()
        );
    }

    #[tokio::test]
    async fn stale_unknown_timer_does_not_release_reacquired_lease() {
        let coord = LeaseCoordinator::with_lock_dir(test_dir());
        let key = key();
        coord
            .try_acquire(key.clone(), "same-owner", Duration::from_secs(60), false)
            .unwrap();
        coord.retain_on_unknown_for(key.clone(), "same-owner".into(), Duration::from_millis(25));
        assert!(coord.release_unknown(&key));
        coord
            .try_acquire(key.clone(), "same-owner", Duration::from_secs(60), false)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            coord
                .try_acquire(key, "competing-owner", Duration::from_secs(10), false)
                .is_err()
        );
    }

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "controlla-lease-test-{}-{}",
            std::process::id(),
            getrandom::u32().unwrap()
        ))
    }
}
