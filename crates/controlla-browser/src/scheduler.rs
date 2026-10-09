//! Per-target execution gates, shared-document mutation serialization, and adaptive tab concurrency.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify, OwnedMutexGuard};

#[path = "input_strategy.rs"]
pub mod input_strategy;
#[path = "probe.rs"]
pub mod probe;
#[path = "semantic.rs"]
pub mod semantic;
#[path = "telemetry.rs"]
pub mod telemetry;

type GateRegistry = Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerLimits {
    pub min_active: usize,
    pub initial_active: usize,
    pub max_active: usize,
}

impl Default for SchedulerLimits {
    fn default() -> Self {
        Self {
            min_active: 1,
            initial_active: 4,
            max_active: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    ActiveTabLimitMustBePositive,
    InvalidLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerPressure {
    Healthy,
    Neutral,
    Slow,
    Timeout,
    TransportFailure,
}

struct AdaptiveGate {
    limits: SchedulerLimits,
    current_limit: AtomicUsize,
    active: AtomicUsize,
    healthy_streak: AtomicUsize,
    notify: Notify,
}

impl AdaptiveGate {
    fn new(limits: SchedulerLimits) -> Result<Arc<Self>, SchedulerError> {
        if limits.min_active == 0 || limits.initial_active == 0 || limits.max_active == 0 {
            return Err(SchedulerError::ActiveTabLimitMustBePositive);
        }
        if limits.min_active > limits.initial_active || limits.initial_active > limits.max_active {
            return Err(SchedulerError::InvalidLimits);
        }
        Ok(Arc::new(Self {
            limits,
            current_limit: AtomicUsize::new(limits.initial_active),
            active: AtomicUsize::new(0),
            healthy_streak: AtomicUsize::new(0),
            notify: Notify::new(),
        }))
    }

    async fn acquire(self: &Arc<Self>) -> AdaptivePermit {
        loop {
            let notified = self.notify.notified();
            let limit = self.current_limit.load(Ordering::Acquire);
            let active = self.active.load(Ordering::Acquire);
            if active < limit
                && self
                    .active
                    .compare_exchange(active, active + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return AdaptivePermit {
                    gate: Arc::clone(self),
                };
            }
            notified.await;
        }
    }

    fn record(&self, pressure: SchedulerPressure) {
        match pressure {
            SchedulerPressure::Healthy => {
                let streak = self.healthy_streak.fetch_add(1, Ordering::AcqRel) + 1;
                if streak >= 8 {
                    self.healthy_streak.store(0, Ordering::Release);
                    let mut current = self.current_limit.load(Ordering::Acquire);
                    while current < self.limits.max_active {
                        match self.current_limit.compare_exchange(
                            current,
                            current + 1,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => {
                                self.notify.notify_waiters();
                                break;
                            }
                            Err(actual) => current = actual,
                        }
                    }
                }
            }
            SchedulerPressure::Neutral => {}
            SchedulerPressure::Slow
            | SchedulerPressure::Timeout
            | SchedulerPressure::TransportFailure => {
                self.healthy_streak.store(0, Ordering::Release);
                let mut current = self.current_limit.load(Ordering::Acquire);
                loop {
                    let next = (current / 2).max(self.limits.min_active);
                    if next == current {
                        break;
                    }
                    match self.current_limit.compare_exchange(
                        current,
                        next,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => break,
                        Err(actual) => current = actual,
                    }
                }
            }
        }
    }
}

struct AdaptivePermit {
    gate: Arc<AdaptiveGate>,
}

impl Drop for AdaptivePermit {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::AcqRel);
        self.gate.notify.notify_one();
    }
}

#[derive(Clone)]
pub struct TargetScheduler {
    targets: GateRegistry,
    documents: GateRegistry,
    adaptive: Arc<AdaptiveGate>,
}

impl Default for TargetScheduler {
    fn default() -> Self {
        Self::with_limits(SchedulerLimits::default()).expect("default scheduler limits are valid")
    }
}

impl TargetScheduler {
    pub fn with_limits(limits: SchedulerLimits) -> Result<Self, SchedulerError> {
        Ok(Self {
            targets: Arc::new(Mutex::new(HashMap::new())),
            documents: Arc::new(Mutex::new(HashMap::new())),
            adaptive: AdaptiveGate::new(limits)?,
        })
    }

    pub fn with_max_active_tabs(max_active_tabs: usize) -> Result<Self, SchedulerError> {
        if max_active_tabs == 0 {
            return Err(SchedulerError::ActiveTabLimitMustBePositive);
        }
        Self::with_limits(SchedulerLimits {
            min_active: 1,
            initial_active: max_active_tabs,
            max_active: max_active_tabs,
        })
    }

    pub fn max_active_tabs(&self) -> usize {
        self.adaptive.limits.max_active
    }

    pub fn current_active_limit(&self) -> usize {
        self.adaptive.current_limit.load(Ordering::Acquire)
    }

    pub fn active_operations(&self) -> usize {
        self.adaptive.active.load(Ordering::Acquire)
    }

    pub fn record_transport_failure(&self) {
        self.adaptive.record(SchedulerPressure::TransportFailure);
    }

    pub fn record_timeout(&self) {
        self.adaptive.record(SchedulerPressure::Timeout);
    }

    pub fn record_pressure(&self, pressure: SchedulerPressure) {
        self.adaptive.record(pressure);
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

    fn pressure(queue_wait: Duration, execution: Duration) -> SchedulerPressure {
        if queue_wait >= Duration::from_secs(1) || execution >= Duration::from_secs(5) {
            SchedulerPressure::Slow
        } else if queue_wait <= Duration::from_millis(100) && execution <= Duration::from_secs(2) {
            SchedulerPressure::Healthy
        } else {
            SchedulerPressure::Neutral
        }
    }

    /// Run one operation while holding only the selected target's actor gate.
    pub async fn run_target<F: Future>(&self, target_id: &str, operation: F) -> F::Output {
        let queued_at = Instant::now();
        let _guard = Self::gate(&self.targets, target_id).await;
        let _permit = self.adaptive.acquire().await;
        let queue_wait = queued_at.elapsed();
        let started_at = Instant::now();
        let output = operation.await;
        self.adaptive
            .record(Self::pressure(queue_wait, started_at.elapsed()));
        output
    }

    /// Serialize mutations that share a document, even when they target
    /// different tabs. Reads and unrelated documents remain independent.
    pub async fn run_document_mutation<F: Future>(
        &self,
        target_id: &str,
        document_id: &str,
        operation: F,
    ) -> F::Output {
        let queued_at = Instant::now();
        let _target_guard = Self::gate(&self.targets, target_id).await;
        let _document_guard = Self::gate(&self.documents, document_id).await;
        let _permit = self.adaptive.acquire().await;
        let queue_wait = queued_at.elapsed();
        let started_at = Instant::now();
        let output = operation.await;
        self.adaptive
            .record(Self::pressure(queue_wait, started_at.elapsed()));
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_validate_and_default_to_one_four_eight() {
        assert_eq!(
            SchedulerLimits::default(),
            SchedulerLimits {
                min_active: 1,
                initial_active: 4,
                max_active: 8
            }
        );
        assert_eq!(
            TargetScheduler::with_limits(SchedulerLimits {
                min_active: 2,
                initial_active: 1,
                max_active: 8
            })
            .err(),
            Some(SchedulerError::InvalidLimits)
        );
        assert_eq!(
            TargetScheduler::with_max_active_tabs(0).err(),
            Some(SchedulerError::ActiveTabLimitMustBePositive)
        );
    }

    #[test]
    fn aimd_drops_fast_and_recovers_conservatively() {
        let scheduler = TargetScheduler::default();
        assert_eq!(scheduler.current_active_limit(), 4);
        scheduler.record_transport_failure();
        assert_eq!(scheduler.current_active_limit(), 2);
        scheduler.record_timeout();
        assert_eq!(scheduler.current_active_limit(), 1);
        for _ in 0..8 {
            scheduler.record_pressure(SchedulerPressure::Healthy);
        }
        assert_eq!(scheduler.current_active_limit(), 2);
    }

    #[test]
    fn neutral_pressure_does_not_grow_concurrency() {
        let scheduler = TargetScheduler::default();
        for _ in 0..64 {
            scheduler.record_pressure(SchedulerPressure::Neutral);
        }
        assert_eq!(scheduler.current_active_limit(), 4);
    }

    #[tokio::test]
    async fn same_target_is_serialized_without_global_tab_lock() {
        let scheduler = TargetScheduler::with_limits(SchedulerLimits {
            min_active: 1,
            initial_active: 2,
            max_active: 2,
        })
        .unwrap();
        let first = scheduler.clone();
        let first_task = tokio::spawn(async move {
            first
                .run_target("a", async {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    1usize
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let second = scheduler.clone();
        let second_task =
            tokio::spawn(async move { second.run_target("b", async { 2usize }).await });
        assert_eq!(second_task.await.unwrap(), 2);
        assert_eq!(first_task.await.unwrap(), 1);
    }
}
