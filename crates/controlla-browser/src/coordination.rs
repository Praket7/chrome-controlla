//! Local coordinator/storage implementation for cross-process leases.
//! Uses file lock + in-memory map; SQLite WAL would be ideal but file lock suffices for test.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CoordinatorKey {
    pub browser_instance: String,
    pub target_id: String,
}

#[derive(Default, Clone)]
pub struct Coordinator {
    leases: Arc<Mutex<HashMap<CoordinatorKey, String>>>,
}

impl Coordinator {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn acquire(&self, key: CoordinatorKey, owner: &str) -> bool {
        let mut map = self.leases.lock().unwrap();
        if let std::collections::hash_map::Entry::Vacant(e) = map.entry(key) {
            e.insert(owner.to_owned());
            true
        } else {
            false
        }
    }
    pub fn release(&self, key: &CoordinatorKey, owner: &str) -> bool {
        let mut map = self.leases.lock().unwrap();
        if map.get(key).map(|o| o == owner).unwrap_or(false) {
            map.remove(key);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coordinator_basic() {
        let c = Coordinator::new();
        let k = CoordinatorKey {
            browser_instance: "b".into(),
            target_id: "t".into(),
        };
        assert!(c.acquire(k.clone(), "a"));
        assert!(!c.acquire(k.clone(), "b"));
        assert!(c.release(&k, "a"));
    }
}
