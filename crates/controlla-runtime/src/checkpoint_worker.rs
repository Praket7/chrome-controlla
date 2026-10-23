//! Ordered checkpoint operations on blocking workers, outside async executors.
use super::{MutationCheckpointState, TaskCheckpointStore};
use std::sync::Arc;
use tokio::sync::Mutex;

/// At most one blocking operation per store. The owned guard stays with an
/// operation even if its async caller is cancelled. Write-ahead calls are
/// awaited before dispatch; they are never sent to a fire-and-forget queue.
#[derive(Clone)]
pub struct CheckpointWorker {
    inner: Arc<Mutex<TaskCheckpointStore>>,
}

impl CheckpointWorker {
    pub fn new(store: TaskCheckpointStore) -> Self {
        Self {
            inner: Arc::new(Mutex::new(store)),
        }
    }

    async fn run<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut TaskCheckpointStore) -> R + Send + 'static,
    ) -> Result<R, String> {
        let mut guard = self.inner.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || operation(&mut guard))
            .await
            .map_err(|_| "checkpoint worker failed".to_owned())
    }

    pub async fn target_completed(
        &self,
        key: &str,
        task_digest: &str,
        target: &str,
        target_identity: &str,
    ) -> bool {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let target = target.to_owned();
        let target_identity = target_identity.to_owned();
        self.run(move |store| store.target_completed(&key, &task_digest, &target, &target_identity))
            .await
            .unwrap_or(false)
    }

    pub async fn bind(&self, key: &str, task_digest: &str) -> Result<(), String> {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        self.run(move |store| store.bind(&key, &task_digest))
            .await?
    }

    pub async fn mark_target_completed(
        &self,
        key: &str,
        task_digest: &str,
        target: &str,
        target_identity: &str,
    ) -> Result<(), String> {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let target = target.to_owned();
        let target_identity = target_identity.to_owned();
        self.run(move |store| {
            store.mark_target_completed(&key, &task_digest, &target, &target_identity)
        })
        .await?
    }

    pub async fn bind_target_identity(
        &self,
        key: &str,
        task_digest: &str,
        target: &str,
        target_identity: &str,
    ) -> Result<bool, String> {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let target = target.to_owned();
        let target_identity = target_identity.to_owned();
        self.run(move |store| {
            store.bind_target_identity(&key, &task_digest, &target, &target_identity)
        })
        .await?
    }

    pub async fn finish(&self, key: &str) -> Result<(), String> {
        let key = key.to_owned();
        self.run(move |store| store.finish(&key)).await?
    }

    pub async fn mark_mutation_dispatch_may_have_occurred(
        &self,
        key: &str,
        task_digest: &str,
        step_identity: &str,
        target_identity: &str,
        document_identity: &str,
        predicate_identity: &str,
    ) -> Result<MutationCheckpointState, String> {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let step_identity = step_identity.to_owned();
        let target_identity = target_identity.to_owned();
        let document_identity = document_identity.to_owned();
        let predicate_identity = predicate_identity.to_owned();
        self.run(move |store| {
            store.mark_mutation_dispatch_may_have_occurred(
                &key,
                &task_digest,
                &step_identity,
                &target_identity,
                &document_identity,
                &predicate_identity,
            )
        })
        .await?
    }

    pub async fn mark_mutation_verified(
        &self,
        key: &str,
        task_digest: &str,
        step_identity: &str,
        target_identity: &str,
        document_identity: &str,
        predicate_identity: &str,
    ) -> Result<(), String> {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let step_identity = step_identity.to_owned();
        let target_identity = target_identity.to_owned();
        let document_identity = document_identity.to_owned();
        let predicate_identity = predicate_identity.to_owned();
        self.run(move |store| {
            store.mark_mutation_verified(
                &key,
                &task_digest,
                &step_identity,
                &target_identity,
                &document_identity,
                &predicate_identity,
            )
        })
        .await?
    }

    pub async fn mutation_state(
        &self,
        key: &str,
        task_digest: &str,
        step_identity: &str,
        target_identity: &str,
        document_identity: &str,
        predicate_identity: &str,
    ) -> MutationCheckpointState {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let step_identity = step_identity.to_owned();
        let target_identity = target_identity.to_owned();
        let document_identity = document_identity.to_owned();
        let predicate_identity = predicate_identity.to_owned();
        self.run(move |store| {
            store.mutation_state(
                &key,
                &task_digest,
                &step_identity,
                &target_identity,
                &document_identity,
                &predicate_identity,
            )
        })
        .await
        .unwrap_or(MutationCheckpointState::Unavailable)
    }

    pub async fn mark_postcondition_completed(
        &self,
        key: &str,
        task_digest: &str,
        artifact_identity: &str,
        evidence_identity: &str,
        postcondition_identity: &str,
    ) -> Result<(), String> {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let artifact_identity = artifact_identity.to_owned();
        let evidence_identity = evidence_identity.to_owned();
        let postcondition_identity = postcondition_identity.to_owned();
        self.run(move |store| {
            store.mark_postcondition_completed(
                &key,
                &task_digest,
                &artifact_identity,
                &evidence_identity,
                &postcondition_identity,
            )
        })
        .await?
    }

    pub async fn postcondition_completed(
        &self,
        key: &str,
        task_digest: &str,
        artifact_identity: &str,
        evidence_identity: &str,
        postcondition_identity: &str,
    ) -> bool {
        let key = key.to_owned();
        let task_digest = task_digest.to_owned();
        let artifact_identity = artifact_identity.to_owned();
        let evidence_identity = evidence_identity.to_owned();
        let postcondition_identity = postcondition_identity.to_owned();
        self.run(move |store| {
            store.postcondition_completed(
                &key,
                &task_digest,
                &artifact_identity,
                &evidence_identity,
                &postcondition_identity,
            )
        })
        .await
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_checkpoints::{MutationCheckpointState, TaskCheckpointStore};

    #[tokio::test]
    async fn cancelled_caller_cannot_release_storage_ordering_early() {
        let worker = CheckpointWorker::new(TaskCheckpointStore::empty());
        let copy = worker.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let first = tokio::spawn(async move {
            copy.run(move |store| {
                let _ = started.send(());
                wait.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                store.bind("task", &"a".repeat(64))
            })
            .await
        });
        ready.await.unwrap();
        first.abort();
        let mut second = tokio::spawn(async move { worker.bind("task", &"b".repeat(64)).await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut second)
                .await
                .is_err()
        );
        release.send(()).unwrap();
        assert!(
            second.await.unwrap().is_err(),
            "cancelled write must finish before another binding is admitted"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_storage_does_not_block_executor() {
        let worker = CheckpointWorker::new(TaskCheckpointStore::empty());
        let started = std::time::Instant::now();
        let (stored, tick) = tokio::join!(
            worker.run(|_| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                true
            }),
            async {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                started.elapsed()
            },
        );
        assert!(stored.unwrap());
        assert!(
            tick < std::time::Duration::from_millis(150),
            "storage blocked executor for {tick:?}"
        );
    }

    #[tokio::test]
    async fn worker_awaits_write_ahead_and_preserves_exact_mutation_state() {
        let worker = CheckpointWorker::new(TaskCheckpointStore::empty());
        let digest = "a".repeat(64);
        worker.bind("task", &digest).await.unwrap();
        let first = worker
            .mark_mutation_dispatch_may_have_occurred(
                "task",
                &digest,
                "step",
                "target",
                "document",
                "predicate",
            )
            .await
            .unwrap();
        assert_eq!(first, MutationCheckpointState::NotStarted);
        assert_eq!(
            worker
                .mutation_state("task", &digest, "step", "target", "document", "predicate")
                .await,
            MutationCheckpointState::DispatchMayHaveOccurred
        );
        worker
            .mark_mutation_verified("task", &digest, "step", "target", "document", "predicate")
            .await
            .unwrap();
        assert_eq!(
            worker
                .mutation_state("task", &digest, "step", "target", "document", "predicate")
                .await,
            MutationCheckpointState::Verified
        );
    }
}
