//! Shared-target lease admission and notification-backed contention waits.
use super::*;

pub(super) fn lease_target_key(_session_id: &str, tab_id: &str) -> crate::v3::TargetKey {
    crate::v3::TargetKey {
        // A tab may be paired through more than one MCP session; session IDs do not identify Chrome.
        browser_id: "shared-extension".into(),
        tab_id: tab_id.to_owned(),
    }
}

pub(super) const TARGET_LEASE_QUEUE_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const TARGET_READ_LEASE_QUEUE_TIMEOUT: Duration = Duration::from_secs(2);
pub(super) const TARGET_READ_LEASE_TTL_MS: u64 = 30_000;

pub(super) fn process_lease_key(target: &crate::v3::TargetKey) -> crate::process_lease::LeaseKey {
    crate::process_lease::LeaseKey {
        browser_instance: target.browser_id.clone(),
        target_id: target.tab_id.clone(),
        // Shared-extension tabs are serialized across document/account changes
        // too: the mutation route revalidates those identities at dispatch.
        document_epoch: String::new(),
        account_epoch: String::new(),
        effect_class: "shared-target-mutation".into(),
    }
}

pub(super) fn process_lease_coordinator() -> &'static crate::process_lease::LeaseCoordinator {
    static COORDINATOR: std::sync::OnceLock<crate::process_lease::LeaseCoordinator> =
        std::sync::OnceLock::new();
    COORDINATOR.get_or_init(crate::process_lease::LeaseCoordinator::new)
}

/// Acquire a short, shared observation lease. Readers never take the
/// process-wide mutation lock and never fence another reader.
pub(super) async fn acquire_queued_read_lease(
    leases: &Mutex<crate::v3::LeaseTable>,
    target: &crate::v3::TargetKey,
    owner: &str,
) -> Result<crate::v3::TargetReadLease, crate::v3::TargetLease> {
    let deadline = tokio::time::Instant::now() + TARGET_READ_LEASE_QUEUE_TIMEOUT;
    loop {
        let changed = process_lease_coordinator().changed();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let mut table = leases.lock().await;
        let result = table.acquire_read(target.clone(), owner, now_ms(), TARGET_READ_LEASE_TTL_MS);
        drop(table);
        match result {
            Ok(lease) => return Ok(lease),
            Err(existing) if tokio::time::Instant::now() >= deadline => return Err(existing),
            Err(_) => {
                tokio::select! {
                    _ = changed.as_mut() => {},
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                }
            }
        }
    }
}

pub(super) async fn acquire_queued_lease(
    leases: &Mutex<crate::v3::LeaseTable>,
    target: &crate::v3::TargetKey,
    owner: &str,
    nested: bool,
) -> Result<crate::v3::TargetLease, crate::v3::TargetLease> {
    acquire_queued_lease_with_admission(leases, target, owner, nested)
        .await
        .map(|(lease, _)| lease)
}

/// Acquire the exclusive target lease, returning any dead-owner takeover this
/// process performed so the action receipt can report it.
pub(super) async fn acquire_queued_lease_with_admission(
    leases: &Mutex<crate::v3::LeaseTable>,
    target: &crate::v3::TargetKey,
    owner: &str,
    nested: bool,
) -> Result<(crate::v3::TargetLease, Option<String>), crate::v3::TargetLease> {
    let deadline = tokio::time::Instant::now() + TARGET_LEASE_QUEUE_TIMEOUT;
    loop {
        let changed = process_lease_coordinator().changed();
        tokio::pin!(changed);
        changed.as_mut().enable();
        // Do not yield between acquiring the process lock and owning the
        // in-memory lease: cancellation there would orphan the process lock.
        let mut table = leases.lock().await;
        let process_key = process_lease_key(target);
        let mut takeover = None;
        if !nested {
            if process_lease_coordinator()
                .try_acquire(
                    process_key.clone(),
                    owner,
                    Duration::from_millis(120_000),
                    false,
                )
                .is_err()
            {
                if tokio::time::Instant::now() >= deadline {
                    let now = now_ms();
                    return Err(crate::v3::TargetLease {
                        owner: "another MCP process".into(),
                        acquired_at_ms: now,
                        expires_at_ms: now.saturating_add(120_000),
                    });
                }
                drop(table);
                tokio::select! {
                    _ = changed.as_mut() => {},
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {},
                }
                continue;
            }
            // A dead-owner takeover is admission evidence: the previous holder
            // could not have completed its effect, so the receipt must say so.
            takeover = process_lease_coordinator().take_takeover(&process_key);
        }
        let result = if nested {
            table.acquire_nested(target.clone(), owner, now_ms(), 120_000)
        } else {
            table.acquire(target.clone(), owner, now_ms(), 120_000)
        };
        drop(table);
        match result {
            Ok(lease) => return Ok((lease, takeover)),
            Err(existing) if tokio::time::Instant::now() >= deadline => {
                if !nested {
                    process_lease_coordinator().release(&process_key, owner);
                }
                return Err(existing);
            }
            Err(_) => {
                if !nested {
                    process_lease_coordinator().release(&process_key, owner);
                }
                tokio::select! {
                    _ = changed.as_mut() => {},
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {},
                }
            }
        }
    }
}
