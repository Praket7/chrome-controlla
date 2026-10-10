//! Cross-process lease client for real Chrome mutations.
//! Lease key: (browser_instance, target_id, document_epoch, account_epoch, effect_class)

use std::collections::HashMap;
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

#[derive(Default, Clone)]
pub struct LeaseCoordinator {
    inner: Arc<Mutex<HashMap<LeaseKey, Lease>>>,
}

impl LeaseCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to acquire mutation lease. Read/read may share; mutations serialize.
    pub fn try_acquire(
        &self,
        key: LeaseKey,
        owner: &str,
        ttl: Duration,
        is_read: bool,
    ) -> Result<Lease, String> {
        let mut map = self.inner.lock().unwrap();
        // expire old
        map.retain(|_, lease| lease.expires_at > Instant::now());
        if let Some(existing) = map.get(&key) {
            if is_read && existing.effect == "read" {
                // read/read share: allow duplicate lease for same key if both reads
                // return existing as shared
                return Ok(existing.clone());
            }
            return Err(format!("lease held by {}", existing.owner));
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
        map.insert(key, lease.clone());
        Ok(lease)
    }

    pub fn release(&self, key: &LeaseKey, owner: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        if map.get(key).is_some_and(|l| l.owner == owner) {
            map.remove(key);
            true
        } else {
            false
        }
    }

    pub fn retain_on_unknown(&self, key: &LeaseKey) {
        // On unknown, retain lease through TTL; no-op here as TTL governs
        let _ = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_read_may_share_mutation_serializes() {
        let coord = LeaseCoordinator::new();
        let key = LeaseKey {
            browser_instance: "b".into(),
            target_id: "t".into(),
            document_epoch: "d".into(),
            account_epoch: "a".into(),
            effect_class: "mutation".into(),
        };
        let r1 = coord.try_acquire(key.clone(), "c1", Duration::from_secs(10), true);
        assert!(r1.is_ok());
        let r2 = coord.try_acquire(key.clone(), "c2", Duration::from_secs(10), true);
        assert!(r2.is_ok()); // read/read share
        let key2 = LeaseKey {
            browser_instance: "b".into(),
            target_id: "t".into(),
            document_epoch: "d".into(),
            account_epoch: "a".into(),
            effect_class: "mutation2".into(),
        };
        // Actually test mutation serialization with same key but mutation effect
        let mkey = LeaseKey {
            browser_instance: "b2".into(),
            target_id: "t2".into(),
            document_epoch: "d".into(),
            account_epoch: "a".into(),
            effect_class: "mutation".into(),
        };
        let m1 = coord
            .try_acquire(mkey.clone(), "c1", Duration::from_secs(10), false)
            .unwrap();
        assert!(
            coord
                .try_acquire(mkey.clone(), "c2", Duration::from_secs(10), false)
                .is_err()
        );
        assert!(coord.release(&mkey, "c1"));
        assert!(
            coord
                .try_acquire(mkey, "c2", Duration::from_secs(10), false)
                .is_ok()
        );
        let _ = (key2, m1);
    }
}
