//! Cross-process lease client for real Chrome mutations.
//! Lease key: (browser_instance, target_id, document_epoch, account_epoch, effect_class)

use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    next_generation: Arc<std::sync::atomic::AtomicU64>,
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
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            lock_dir: Arc::new(lock_dir.into()),
            next_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
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
        if map.contains_key(&key) {
            if is_read
                && let Some(readers) = map.get_mut(&key).and_then(|held| held.readers.as_mut())
            {
                readers.insert(owner.to_owned());
                let mut shared = map.get(&key).unwrap().lease.clone();
                shared.owner = owner.to_owned();
                return Ok(shared);
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
            .open(path)
            .map_err(|error| format!("unable to open target lock file: {error}"))?;
        let lock_result = if is_read {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match lock_result {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err("lease held by another MCP process".to_owned());
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(format!("unable to acquire target lock: {error}"));
            }
        }

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
            }
            return true;
        }
        if held.lease.owner == owner {
            map.remove(key);
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
    pub fn release_unknown(&self, key: &LeaseKey) -> bool {
        let mut map = self.inner.lock().unwrap();
        if map.get(key).is_some_and(|held| held.unknown) {
            map.remove(key);
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
        if map.get(key).is_some_and(|held| {
            held.lease.owner == owner && held.generation == generation && held.unknown
        }) {
            map.remove(key);
            true
        } else {
            false
        }
    }
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
        coord.retain_on_unknown_for(key.clone(), "p1".into(), Duration::from_millis(20));
        assert!(
            coord
                .try_acquire(key.clone(), "p2", Duration::from_secs(10), false)
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
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
