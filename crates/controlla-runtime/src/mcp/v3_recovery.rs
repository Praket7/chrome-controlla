// Executed guard ladders for text and download dispatch.
//
// These two entry points are the only places a guard's recovery rungs run
// against a page. Each candidate target is resolved by a route that can
// satisfy the failing predicate, then the guard re-runs under the same
// deadline. The loop is bounded by `recovery_exec::MAX_LADDER_STEPS`, and the
// ledger it returns is what the receipt reports as `recovery.executed`.
//
// Kept out of `v3_full.rs` so the MCP facade composes the transaction instead
// of owning it; the source-structure ratchet enforces the split.

use serde_json::{Value, json};
use std::time::Duration;

fn invalid(message: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(message.into(), None)
}

/// Execute the editor guard's internal ladder for one text transaction.
///
/// The editor guard names a failing predicate (`not_editable`, `blocked`,
/// `target_replaced`, `stale_value`, `focus_or_selection_changed`). Each rung
/// resolves the same control by a route that can satisfy it — bind the one
/// editable descendant of a wrapper, click the editor's own geometry with
/// trusted input, wait for a transition, or revalidate after the guard's own
/// rebind — and then re-runs the guard. The loop is bounded by
/// [`crate::recovery_exec::MAX_LADDER_STEPS`], so a stuck page cannot loop, and
/// the returned ledger is what the receipt reports as `recovery.executed`.
///
/// Block, IME, and key-event typing all call this, so the ladder cannot drift
/// between entry points.
#[allow(clippy::too_many_arguments)]
pub(super) async fn complete_editor_guard(
    connection: &super::super::PageSessionTransport,
    registry: &controlla_browser::sessions::SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
    object_id: &str,
    index: usize,
    expected: &str,
    prepare: bool,
    normalize_paragraphs: bool,
    require_focus: bool,
    check_value: bool,
    deadline: tokio::time::Instant,
) -> Result<(Value, crate::recovery_exec::Executed), rmcp::ErrorData> {
    let mut executed = crate::recovery_exec::Executed::default();
    let mut ledger: Option<crate::action_recovery::RecoveryLedger> = None;
    let mut active_reason: Option<crate::guard_reason::GuardReason> = None;
    let mut checked = Value::Null;
    // The snapshot remote object is the retained `nodes` array at some dispatch
    // sites and the element itself at others. An editable-descendant rung
    // resolves a new element, so the guard is retargeted at that object instead
    // of relying on `nodes[index]` being writable.
    let mut active_object_id = object_id.to_owned();
    for _ in 0..=crate::recovery_exec::MAX_LADDER_STEPS {
        let response = crate::v3_runtime::await_key_batch(
            deadline,
            connection.command(
                registry,
                handle,
                tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId":active_object_id,"functionDeclaration":super::v3::EDITOR_GUARD,
                    "arguments":[{"value":index},{"value":expected},{"value":prepare},{"value":normalize_paragraphs},{"value":require_focus},{"value":check_value}],"returnByValue":true
                }),
            ),
        )
        .await
        .map_err(|_| invalid("type preflight timed out; no key dispatched"))?
        .map_err(|error| invalid(error.to_string()))?;
        checked = super::super::shared_value(&response).map_err(invalid)?;
        if checked["ok"] == true {
            executed.mark_resolved();
            break;
        }
        let Some(reason) = checked["reason"]
            .as_str()
            .and_then(crate::recovery_exec::guard_reason_for_preflight)
        else {
            break;
        };
        if active_reason != Some(reason) {
            active_reason = Some(reason);
            ledger = Some(crate::action_recovery::RecoveryLedger::new(reason));
        }
        let Some(rung) = ledger.as_mut().and_then(|ledger| ledger.next_rung()) else {
            break;
        };
        let operation = crate::recovery_exec::operation_for(rung);
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if !operation.is_executable() {
            continue;
        }
        executed.record(operation);
        match operation {
            crate::recovery_exec::RungOperation::EditableDescendant => {
                // Bind a wrapper reference to its one editable descendant. The
                // script rewrites `nodes[index]` when the remote object is the
                // retained array and returns the element either way, so the
                // guard can be retargeted at a real editable object.
                let target = connection
                    .command(
                        registry,
                        handle,
                        tab_id,
                        "Runtime.callFunctionOn",
                        json!({"objectId":active_object_id,"functionDeclaration":crate::recovery_exec::EDITABLE_SCRIPT,"arguments":[{"value":index}],"returnByValue":false}),
                    )
                    .await
                    .ok()
                    .and_then(|value| value.pointer("/result/objectId").and_then(Value::as_str).map(str::to_owned));
                let Some(target) = target else {
                    continue;
                };
                active_object_id = target;
            }
            crate::recovery_exec::RungOperation::TrustedPointer => {
                // Focus the editor through trusted CDP at its own geometry,
                // then revalidate. This is the route a person takes to place a
                // caret in an editor that does not accept synthetic focus.
                let anchor = connection
                    .command(
                        registry,
                        handle,
                        tab_id,
                        "Runtime.callFunctionOn",
                        json!({"objectId":active_object_id,"functionDeclaration":crate::recovery_exec::ANCHOR_SCRIPT,"arguments":[{"value":index},{"value":"self"}],"returnByValue":true}),
                    )
                    .await
                    .ok()
                    .and_then(|value| super::super::shared_value(&value).ok());
                let Some(anchor) = anchor.filter(|anchor| anchor["ok"] == true) else {
                    continue;
                };
                let Some(events) = super::super::pointer_click_events(
                    anchor["x"].as_f64().unwrap_or_default(),
                    anchor["y"].as_f64().unwrap_or_default(),
                    "click",
                )
                .ok()
                else {
                    continue;
                };
                for event in events {
                    let _ = connection
                        .command(
                            registry,
                            handle,
                            tab_id,
                            "Input.dispatchMouseEvent",
                            event,
                        )
                        .await;
                }
            }
            crate::recovery_exec::RungOperation::Settle { delay_ms } => {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            crate::recovery_exec::RungOperation::WaitForEnable => {
                tokio::time::sleep(Duration::from_millis(120)).await;
            }
            // The guard's own `prepare` pass already rebinds a replaced
            // editor; these rungs only need the bounded revalidation.
            crate::recovery_exec::RungOperation::Reacquire
            | crate::recovery_exec::RungOperation::Revalidate
            | crate::recovery_exec::RungOperation::ValueRecheck => {}
            _ => continue,
        }
    }
    Ok((checked, executed))
}

/// Execute the non-mutating recovery ladder for a download preflight.
///
/// A download never dispatches the pointer route; the guard only proves the
/// link is connected, visible, and enabled. The rungs that can change that are
/// bounded and non-mutating: scroll the link into view (the anchor script does
/// the scroll even when its coordinates are not used), wait for a transition,
/// rebind by semantic identity, or revalidate. A refused download therefore
/// reports the rungs it ran, not a plan the caller has to re-derive.
#[allow(clippy::too_many_arguments)]
pub(super) async fn complete_download_guard(
    connection: &super::super::PageSessionTransport,
    registry: &controlla_browser::sessions::SessionRegistry,
    handle: &controlla_browser::sessions::SessionHandle,
    tab_id: &str,
    object_id: &str,
    index: usize,
    guard: &str,
    extra_values: &[Value],
    deadline: tokio::time::Instant,
) -> Result<(Value, crate::recovery_exec::Executed), rmcp::ErrorData> {
    let mut executed = crate::recovery_exec::Executed::default();
    let mut ledger: Option<crate::action_recovery::RecoveryLedger> = None;
    let mut active_reason: Option<crate::guard_reason::GuardReason> = None;
    let mut checked = Value::Null;
    for _ in 0..=crate::recovery_exec::MAX_LADDER_STEPS {
        let mut arguments = vec![json!({"value":index})];
        arguments.extend(extra_values.iter().map(|value| json!({"value":value})));
        let response = crate::v3_runtime::await_key_batch(
            deadline,
            connection.command(
                registry,
                handle,
                tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId":object_id,"functionDeclaration":guard,
                    "arguments":arguments,"returnByValue":true
                }),
            ),
        )
        .await
        .map_err(|_| invalid("download preflight timed out; nothing was dispatched"))?
        .map_err(|error| invalid(error.to_string()))?;
        checked = super::super::shared_value(&response).map_err(invalid)?;
        if checked["ok"] == true {
            executed.mark_resolved();
            break;
        }
        let Some(reason) = crate::recovery_exec::reason_of(&checked)
            .or_else(|| {
                checked["reason"]
                    .as_str()
                    .and_then(crate::recovery_exec::guard_reason_for_preflight)
            })
        else {
            break;
        };
        if active_reason != Some(reason) {
            active_reason = Some(reason);
            ledger = Some(crate::action_recovery::RecoveryLedger::new(reason));
        }
        let Some(rung) = ledger.as_mut().and_then(|ledger| ledger.next_rung()) else {
            break;
        };
        let operation = crate::recovery_exec::operation_for(rung);
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if !operation.is_executable() {
            continue;
        }
        executed.record(operation);
        match operation {
            crate::recovery_exec::RungOperation::Rescroll { .. }
            | crate::recovery_exec::RungOperation::TrustedPointer
            | crate::recovery_exec::RungOperation::AncestorPointer => {
                let _ = connection
                    .command(
                        registry,
                        handle,
                        tab_id,
                        "Runtime.callFunctionOn",
                        json!({"objectId":object_id,"functionDeclaration":crate::recovery_exec::ANCHOR_SCRIPT,"arguments":[{"value":index},{"value":"self"}],"returnByValue":true}),
                    )
                    .await;
            }
            crate::recovery_exec::RungOperation::Reacquire => {
                let rebound = connection
                    .command(
                        registry,
                        handle,
                        tab_id,
                        "Runtime.callFunctionOn",
                        json!({"objectId":object_id,"functionDeclaration":crate::recovery_exec::REACQUIRE_SCRIPT,"arguments":[{"value":index}],"returnByValue":true}),
                    )
                    .await
                    .ok()
                    .and_then(|value| super::super::shared_value(&value).ok());
                if rebound.as_ref().is_none_or(|rebound| rebound["ok"] != true) {
                    continue;
                }
            }
            crate::recovery_exec::RungOperation::Settle { delay_ms } => {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            crate::recovery_exec::RungOperation::WaitForEnable => {
                tokio::time::sleep(Duration::from_millis(120)).await;
            }
            crate::recovery_exec::RungOperation::Revalidate
            | crate::recovery_exec::RungOperation::ValueRecheck => {}
            _ => continue,
        }
    }
    Ok((checked, executed))
}
