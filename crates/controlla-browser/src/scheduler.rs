//! Per-target execution gates and shared-document mutation serialization.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Weak};
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

type GateRegistry = Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>;

#[derive(Clone)]
pub struct TargetScheduler {
    targets: GateRegistry,
    documents: GateRegistry,
    active_tabs: Arc<Semaphore>,
    max_active_tabs: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    ActiveTabLimitMustBePositive,
}

impl Default for TargetScheduler {
    fn default() -> Self {
        Self::with_max_active_tabs(4).expect("four is a valid active tab limit")
    }
}

impl TargetScheduler {
    pub fn with_max_active_tabs(max_active_tabs: usize) -> Result<Self, SchedulerError> {
        if max_active_tabs == 0 {
            return Err(SchedulerError::ActiveTabLimitMustBePositive);
        }
        Ok(Self {
            targets: Arc::new(Mutex::new(HashMap::new())),
            documents: Arc::new(Mutex::new(HashMap::new())),
            active_tabs: Arc::new(Semaphore::new(max_active_tabs)),
            max_active_tabs,
        })
    }

    pub fn max_active_tabs(&self) -> usize {
        self.max_active_tabs
    }

    async fn gate(registry: &GateRegistry, key: &str) -> OwnedMutexGuard<()> {
        let gate = {
            let mut registry = registry.lock().await;
            registry.retain(|_, gate| gate.strong_count() > 0);
            registry
                .get(key)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let gate = Arc::new(Mutex::new(()));
                    registry.insert(key.to_owned(), Arc::downgrade(&gate));
                    gate
                })
        };
        gate.lock_owned().await
    }

    /// Run one operation while holding only the selected target's actor gate.
    pub async fn run_target<F: Future>(&self, target_id: &str, operation: F) -> F::Output {
        let _guard = Self::gate(&self.targets, target_id).await;
        let _permit = self
            .active_tabs
            .clone()
            .acquire_owned()
            .await
            .expect("scheduler semaphore is never closed");
        operation.await
    }

    /// Serialize mutations that share a document, even when they target
    /// different tabs. Reads and unrelated documents remain independent.
    pub async fn run_document_mutation<F: Future>(
        &self,
        target_id: &str,
        document_id: &str,
        operation: F,
    ) -> F::Output {
        let _target_guard = Self::gate(&self.targets, target_id).await;
        let _document_guard = Self::gate(&self.documents, document_id).await;
        let _permit: OwnedSemaphorePermit = self
            .active_tabs
            .clone()
            .acquire_owned()
            .await
            .expect("scheduler semaphore is never closed");
        operation.await
    }
}
