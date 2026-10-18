use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Workflow cancellation is checked at the browser-command boundary. A command
/// that has reserved this gate may finish, but after cancel returns no later
/// mutating command can be admitted.
pub struct WorkflowCancellation {
    cancelled: AtomicBool,
    dispatched: AtomicUsize,
    in_flight: AtomicUsize,
    gate: Arc<Mutex<()>>,
}

impl Default for WorkflowCancellation {
    fn default() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            dispatched: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            gate: Arc::new(Mutex::new(())),
        }
    }
}

impl WorkflowCancellation {
    /// Request cancellation immediately, then wait out any command already
    /// admitted so the caller's acknowledgement is a real dispatch fence.
    pub async fn cancel(&self) -> bool {
        self.cancelled.store(true, Ordering::SeqCst);
        let _gate = self.gate.clone().lock_owned().await;
        self.dispatched.load(Ordering::SeqCst) > 0 || self.in_flight.load(Ordering::SeqCst) > 0
    }

    pub fn has_dispatched(&self) -> bool {
        self.dispatched.load(Ordering::SeqCst) > 0
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

tokio::task_local! {
    static ACTIVE_WORKFLOW_CANCELLATION: Arc<WorkflowCancellation>;
}

pub async fn with_workflow_cancellation<T>(
    cancellation: Arc<WorkflowCancellation>,
    future: impl Future<Output = T>,
) -> T {
    ACTIVE_WORKFLOW_CANCELLATION
        .scope(cancellation, future)
        .await
}

pub struct BrowserCommandPermit {
    cancellation: Option<Arc<WorkflowCancellation>>,
    _gate: Option<OwnedMutexGuard<()>>,
}

impl Drop for BrowserCommandPermit {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub async fn admit_browser_command(
    method: &str,
) -> Result<BrowserCommandPermit, crate::BrowserError> {
    if !may_mutate(method) {
        return Ok(BrowserCommandPermit {
            cancellation: None,
            _gate: None,
        });
    }
    let Ok(cancellation) = ACTIVE_WORKFLOW_CANCELLATION.try_with(Arc::clone) else {
        return Ok(BrowserCommandPermit {
            cancellation: None,
            _gate: None,
        });
    };
    let gate = cancellation.gate.clone().lock_owned().await;
    if cancellation.cancelled.load(Ordering::SeqCst) {
        return Err(crate::BrowserError::Cancelled);
    }
    cancellation.dispatched.fetch_add(1, Ordering::SeqCst);
    cancellation.in_flight.fetch_add(1, Ordering::SeqCst);
    Ok(BrowserCommandPermit {
        cancellation: Some(cancellation),
        _gate: Some(gate),
    })
}

pub fn may_mutate(method: &str) -> bool {
    method.starts_with("Input.")
        // New extension commands are fenced by default. Only established
        // observation commands may run after cancellation for reconciliation.
        || (method.starts_with("Controlla.") && !matches!(method,
            "Controlla.presentationState" | "Controlla.checkIdentityMarker"
            | "Controlla.fileInputSnapshot" | "Controlla.findFileInput"
            | "Controlla.frameAccessibility" | "Controlla.newTabs"
            | "Controlla.waitForFileChooser"))
        || matches!(method, "Page.navigate" | "DOM.setFileInputFiles"
            | "Runtime.callFunctionOn" | "Runtime.evaluate")
}

#[cfg(test)]
mod tests {
    use super::{WorkflowCancellation, admit_browser_command, with_workflow_cancellation};
    use std::sync::Arc;

    #[tokio::test]
    async fn cancellation_during_preflight_fences_the_later_mutating_command() {
        let cancellation = Arc::new(WorkflowCancellation::default());
        let side_effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let for_call = cancellation.clone();
        let effects = side_effects.clone();
        let operation = tokio::spawn(async move {
            with_workflow_cancellation(for_call, async {
                tokio::task::yield_now().await; // async preflight before the browser mutation
                if let Ok(_permit) = admit_browser_command("Input.dispatchKeyEvent").await {
                    effects.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
            .await;
        });
        cancellation.cancel().await;
        operation.await.unwrap();
        assert_eq!(
            side_effects.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "cancellation during preflight prevents the side effect at the dispatch boundary"
        );
    }

    #[tokio::test]
    async fn cancellation_fences_all_extension_mutations_and_future_commands() {
        let cancellation = Arc::new(WorkflowCancellation::default());
        cancellation.cancel().await;
        with_workflow_cancellation(cancellation, async {
            for method in [
                "Controlla.pointerClick",
                "Controlla.pointerScroll",
                "Controlla.pointerSwipe",
                "Controlla.pointerMultiTouch",
                "Controlla.frameScroll",
                "Controlla.frameSwipe",
                "Controlla.scrollToTarget",
                "Controlla.futureMutation",
            ] {
                assert!(
                    admit_browser_command(method).await.is_err(),
                    "{method} escaped cancellation"
                );
            }
            for method in [
                "Controlla.presentationState",
                "Controlla.checkIdentityMarker",
                "Page.getFrameTree",
            ] {
                assert!(
                    admit_browser_command(method).await.is_ok(),
                    "read-only reconciliation {method} blocked"
                );
            }
        })
        .await;
    }

    #[tokio::test]
    async fn cancellation_waits_for_an_admitted_command_before_acknowledging() {
        let cancellation = Arc::new(WorkflowCancellation::default());
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let for_call = cancellation.clone();
        let command_entered = entered.clone();
        let command_release = release.clone();
        let operation = tokio::spawn(async move {
            with_workflow_cancellation(for_call, async {
                let _permit = admit_browser_command("Page.navigate").await.unwrap();
                command_entered.notify_one();
                command_release.notified().await;
            })
            .await;
        });
        entered.notified().await;
        let for_cancel = cancellation.clone();
        let cancel = tokio::spawn(async move { for_cancel.cancel().await });
        tokio::task::yield_now().await;
        assert!(
            !cancel.is_finished(),
            "cancel waits until the admitted command leaves the gate"
        );
        release.notify_one();
        assert!(
            cancel.await.unwrap(),
            "the caller is told that a command may have taken effect"
        );
        operation.await.unwrap();
    }

    #[tokio::test]
    async fn human_key_character_transaction_fences_queued_chars_until_cleanup_finishes() {
        let cancellation = Arc::new(WorkflowCancellation::default());
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release_cleanup = Arc::new(tokio::sync::Notify::new());
        let operation_cancellation = cancellation.clone();
        let operation_events = events.clone();
        let command_entered = entered.clone();
        let cleanup_release = release_cleanup.clone();
        let operation = tokio::spawn(async move {
            with_workflow_cancellation(operation_cancellation, async {
                for character in ['a', 'b', 'c'] {
                    let Ok(_permit) = admit_browser_command("Controlla.humanTypeCharacter").await
                    else {
                        break;
                    };
                    {
                        let mut emitted = operation_events.lock().unwrap();
                        emitted.push(format!("{character}:keydown"));
                        emitted.push(format!("{character}:char"));
                    }
                    if character == 'a' {
                        command_entered.notify_one();
                        cleanup_release.notified().await;
                    }
                    // The bridge's one-character response is sent only after its
                    // same-target keyUp cleanup (or explicit cleanup-limited result).
                    operation_events
                        .lock()
                        .unwrap()
                        .push(format!("{character}:cleanup_complete"));
                }
            })
            .await;
        });
        entered.notified().await;
        let cancel_cancellation = cancellation.clone();
        let cancel = tokio::spawn(async move { cancel_cancellation.cancel().await });
        tokio::task::yield_now().await;
        assert!(
            !cancel.is_finished(),
            "cancel waits for in-flight character cleanup"
        );
        release_cleanup.notify_one();
        assert!(
            cancel.await.unwrap(),
            "an admitted character may have dispatched"
        );
        operation.await.unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            ["a:keydown", "a:char", "a:cleanup_complete"],
            "only the in-flight character may finish; queued characters stay fenced"
        );
    }
}
