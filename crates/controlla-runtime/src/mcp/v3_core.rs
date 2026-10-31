//! Execution-core helpers extracted from the V3 MCP facade.
//!
//! These are pure or facade-level helpers for operation memory, automatic
//! single-tab binding, typed rebase/text receipts, and task-conditioned
//! observation. Keeping them out of `v3_full.rs` holds the facade below its
//! line ratchet while leaving dispatch in the facade.

use super::super::{BrowserClickOutcome, BrowserSessionArgs};
use super::{AppV3, V3ActArgs, V3BrowserArgs, invalid};
use rmcp::handler::server::wrapper::{Json, Parameters};
use serde_json::{Value, json};
use sha2::Digest as _;

/// Tabs that could satisfy a read-only task hint. Exact URL or same HTTP(S)
/// origin/path (query and fragment are views of one resource) counts; a tab ID
/// hint is authoritative. Results are ordered oldest-first and deduplicated so
/// caller-visible candidates are stable.
pub(super) fn matching_task_tabs(tabs: &[Value], urls: &[String], tab_ids: &[String]) -> Vec<Value> {
    let mut candidates = Vec::new();
    for tab in tabs {
        let tab_id = tab.get("id").and_then(Value::as_str).unwrap_or_default();
        if tab_id.is_empty() {
            continue;
        }
        let by_id = tab_ids.iter().any(|wanted| wanted == tab_id);
        let tab_url = tab.get("url").and_then(Value::as_str).unwrap_or_default();
        let by_url = urls.iter().any(|hint| {
            hint == tab_url
                || (url::Url::parse(hint)
                    .ok()
                    .zip(url::Url::parse(tab_url).ok())
                    .is_some_and(|(hint, tab)| {
                        matches!(hint.scheme(), "http" | "https")
                            && hint.scheme() == tab.scheme()
                            && hint.host_str() == tab.host_str()
                            && hint.path() == tab.path()
                    }))
        });
        if by_id || by_url {
            candidates.push(tab.clone());
        }
    }
    candidates.sort_by_key(|tab| {
        tab.get("last_accessed")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    });
    candidates.dedup_by(|left, right| {
        left.get("id").and_then(Value::as_str) == right.get("id").and_then(Value::as_str)
    });
    candidates
}

impl AppV3 {
    /// Bind a read-only task to one uniquely matching existing tab in the same
    /// call. The caller supplies no session and no tab, so the only safe
    /// outcome is one high-confidence candidate; zero or several candidates are
    /// reported as typed receipts instead of guessing.
    pub(super) async fn auto_bind_read_only_task(
        &self,
        args: &V3BrowserArgs,
        task: &crate::v3_tasks::TaskPrimitive,
    ) -> Result<AutoBindOutcome, rmcp::ErrorData> {
        let crate::v3_tasks::TaskPrimitive::Research { urls, tab_ids, .. } = task else {
            return Ok(AutoBindOutcome::NotApplicable);
        };
        if args.target_ids.is_some() || args.host_id.is_some() || args.window_id.is_some() {
            return Ok(AutoBindOutcome::NotApplicable);
        }
        let Json(discovery) = self
            .core
            .browser_session(Parameters(BrowserSessionArgs {
                action: "discover_shared_tabs".into(),
                provider: Some("shared_extension".into()),
                session_id: None,
                target_ids: None,
                host_id: None,
            }))
            .await?;
        let host_id = discovery["snapshot"]["host_id"]
            .as_str()
            .ok_or_else(|| invalid("fresh guarded discovery omitted host_id"))?
            .to_owned();
        let tabs = discovery["snapshot"]["tabs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let wanted_ids = tab_ids.clone().unwrap_or_default();
        let mut candidates = matching_task_tabs(&tabs, urls, &wanted_ids);
        match candidates.len() {
            0 => Ok(AutoBindOutcome::NoMatch {
                inspected: tabs.len(),
                hint_count: urls.len() + wanted_ids.len(),
            }),
            1 => {
                let candidate = candidates.remove(0);
                let tab_id = candidate
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("matched tab omitted its ID"))?
                    .to_owned();
                let matched_by = if wanted_ids.iter().any(|wanted| wanted == &tab_id) {
                    "tab_id"
                } else {
                    "url"
                };
                let (pair, _) = controlla_browser::scheduler::telemetry::measure_attach(
                    self.core.browser_session(Parameters(BrowserSessionArgs {
                        action: "pair_shared".into(),
                        provider: Some("shared_extension".into()),
                        session_id: None,
                        target_ids: Some(vec![tab_id.clone()]),
                        host_id: Some(host_id),
                    })),
                )
                .await;
                let Json(pair) = pair?;
                let session_id = pair["session_id"]
                    .as_str()
                    .ok_or_else(|| invalid("auto-bind pairing omitted session_id"))?
                    .to_owned();
                self.browser_modes
                    .lock()
                    .await
                    .insert(session_id.clone(), crate::v3::BrowserMode::Background);
                let accepted = self
                    .core
                    .browser_session(Parameters(BrowserSessionArgs {
                        action: "accept_shared_wait".into(),
                        provider: Some("shared_extension".into()),
                        session_id: Some(session_id.clone()),
                        target_ids: None,
                        host_id: None,
                    }))
                    .await;
                match accepted {
                    Ok(Json(value)) if value["accepted"] == true => Ok(AutoBindOutcome::Bound {
                        session_id,
                        tab_id,
                        matched_by,
                        url: candidate
                            .get("url")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    }),
                    Ok(_) => {
                        self.release_cold_session(&session_id).await;
                        Ok(AutoBindOutcome::NoMatch {
                            inspected: tabs.len(),
                            hint_count: urls.len() + wanted_ids.len(),
                        })
                    }
                    Err(error) => {
                        self.release_cold_session(&session_id).await;
                        Err(error)
                    }
                }
            }
            _ => Ok(AutoBindOutcome::Ambiguous {
                candidates: candidates
                    .into_iter()
                    .take(8)
                    .map(|tab| {
                        json!({
                            "tab_id":tab.get("id").and_then(Value::as_str),
                            "url":tab.get("url").and_then(Value::as_str),
                            "title":tab.get("title").and_then(Value::as_str),
                        })
                    })
                    .collect(),
            }),
        }
    }
}

pub(super) enum AutoBindOutcome {
    NotApplicable,
    Bound {
        session_id: String,
        tab_id: String,
        matched_by: &'static str,
        url: Option<String>,
    },
    NoMatch {
        inspected: usize,
        hint_count: usize,
    },
    Ambiguous {
        candidates: Vec<Value>,
    },
}


pub(super) fn action_memory_signature_from_items(origin: &str, items: &[&Value]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(origin.as_bytes());
    for item in items {
        for field in ["role", "name", "href", "disabled"] {
            if let Some(value) = item.get(field) {
                hasher.update(field.as_bytes());
                hasher.update([0]);
                hasher.update(value.to_string().as_bytes());
                hasher.update([0xff]);
            }
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn learned_target_template(target: &Value) -> Option<crate::page_action_memory::TargetTemplate> {
    let role = target.get("role").and_then(Value::as_str)?.trim();
    let name = target.get("name").and_then(Value::as_str).unwrap_or_default();
    if role.is_empty() {
        return None;
    }
    Some(crate::page_action_memory::TargetTemplate {
        role: Some(role.chars().take(64).collect()),
        name: (!name.trim().is_empty()).then(|| name.chars().take(512).collect()),
        href: target
            .get("href")
            .and_then(Value::as_str)
            .filter(|href| !href.is_empty())
            .map(|href| href.chars().take(2048).collect()),
        ancestor: None,
        test_id: None,
    })
}

pub(super) fn learned_click_predicate(outcome: Option<&BrowserClickOutcome>) -> Option<crate::page_action_memory::VerifyPredicate> {
    match outcome? {
        BrowserClickOutcome::Text { text, .. } if !text.trim().is_empty() => {
            Some(crate::page_action_memory::VerifyPredicate::TextPresent {
                text: text.chars().take(1024).collect(),
            })
        }
        BrowserClickOutcome::Navigation { url: Some(url) } if url.starts_with("http") => {
            Some(crate::page_action_memory::VerifyPredicate::UrlMatches {
                pattern: url.chars().take(2048).collect(),
            })
        }
        _ => None,
    }
}

/// Evolve page memory from extraction-only hints to bounded action plans.
/// These remain hints: the caller re-resolves a fresh target and the live
/// action receipt still owns verification.
pub(super) fn learned_action_replay(
    action: &str,
    target: &Value,
    outcome: Option<&BrowserClickOutcome>,
) -> Option<crate::page_action_memory::ReplayPlan> {
    let target = learned_target_template(target)?;
    match action {
        "click" | "double_click" | "context_click" => Some(match learned_click_predicate(outcome) {
            Some(outcome) => crate::page_action_memory::ReplayPlan::Click { target, outcome },
            None => crate::page_action_memory::ReplayPlan::ClickHint { target },
        }),
        "fill" => Some(crate::page_action_memory::ReplayPlan::FillHint {
            target,
            value_slot: "value".into(),
        }),
        "type" => Some(crate::page_action_memory::ReplayPlan::Type {
            target,
            value_slot: "value".into(),
            strategy_hint: Some("auto".into()),
        }),
        "select" => Some(crate::page_action_memory::ReplayPlan::SelectHint {
            target,
            value_slot: "value".into(),
        }),
        "page_tool" => Some(crate::page_action_memory::ReplayPlan::Tool {
            capability_id: "page_tool".into(),
            schema_hash: String::new(),
        }),
        _ => None,
    }
}

pub(super) fn operation_step_for_expanded(
    step: &crate::v3_tasks::ExpandedStep,
    index: usize,
) -> crate::page_action_memory::OperationStep {
    use crate::page_action_memory::OperationStep;
    match step {
        crate::v3_tasks::ExpandedStep::Open { .. } => OperationStep {
            action: "navigate".into(),
            target: None,
            postcondition: None,
            value_slot: None,
        },
        crate::v3_tasks::ExpandedStep::Extract { .. } => OperationStep {
            action: "extract".into(),
            target: None,
            postcondition: None,
            value_slot: None,
        },
        crate::v3_tasks::ExpandedStep::Fill { .. } => OperationStep {
            action: "fill".into(),
            target: None,
            postcondition: None,
            value_slot: Some(format!("value_{index}")),
        },
        crate::v3_tasks::ExpandedStep::Upload { .. }
        | crate::v3_tasks::ExpandedStep::UploadPreviousDownload { .. } => OperationStep {
            action: "upload".into(),
            target: None,
            postcondition: None,
            value_slot: None,
        },
        crate::v3_tasks::ExpandedStep::Download { .. } => OperationStep {
            action: "download".into(),
            target: None,
            postcondition: None,
            value_slot: None,
        },
        crate::v3_tasks::ExpandedStep::Click { .. } => OperationStep {
            action: "click".into(),
            target: None,
            postcondition: None,
            value_slot: None,
        },
        crate::v3_tasks::ExpandedStep::VerifyVisible { .. } => OperationStep {
            action: "verify_visible".into(),
            target: None,
            postcondition: None,
            value_slot: None,
        },
        crate::v3_tasks::ExpandedStep::SaveDraft { .. } => OperationStep {
            action: "save_draft".into(),
            target: None,
            postcondition: None,
            value_slot: None,
        },
    }
}

/// Promote a verified, relevant read into a reusable URL tool. Promotion is
/// input-schema-free at the URL layer, but the capability is registered only
/// for this exact origin and structural signature, and remains read-only.
pub(super) async fn promote_learned_url_operation(
    app: &AppV3,
    task: &crate::v3_tasks::TaskPrimitive,
    url: &str,
    value: &Value,
    session_id: &str,
    tab_id: &str,
) -> Result<Option<crate::site_profiles::SiteProfile>, String> {
    let crate::v3_tasks::TaskPrimitive::Research {
        urls,
        goal: Some(goal),
        ..
    } = task
    else {
        return Ok(None);
    };
    if urls.len() != 1 || urls[0] != url {
        return Ok(None);
    }
    let Some(operation) = crate::capability_registry::operation_from_goal(goal) else {
        return Ok(None);
    };
    if value["truncated"].as_bool().unwrap_or(true)
        || !value["items"].as_array().is_some_and(|items| !items.is_empty())
        || !crate::relevance::speculative_result_matches(goal, value)
    {
        return Ok(None);
    }
    let Ok((site, signature)) =
        super::super::full::current_skill_context_core(&app.core, session_id, tab_id).await
    else {
        return Ok(None);
    };
    if Some(site.clone()) != super::site_scope_from_url(url) {
        return Ok(None);
    }
    let profile = app
        .site_profiles
        .lock()
        .await
        .promote_verified_url(operation, url, &signature, super::now_ms())
        .map_err(str::to_owned)?;
    if let Some(profile) = profile.as_ref() {
        app.capabilities
            .lock()
            .await
            .register(crate::capability_registry::SiteCapability {
                name: profile.intent.clone(),
                site_scope: profile.origin.clone(),
                structural_signature: profile.structural_signature.clone(),
                input_schema: profile.result_schema.clone(),
                implementation: crate::capability_registry::CapabilityImplementation::SiteProfile {
                    profile_id: profile.id.clone(),
                },
                effect: crate::workflow::EffectClass::ReadOnly,
                independently_verified: true,
                version: 1,
            })
            .map_err(str::to_owned)?;
    }
    Ok(profile)
}

pub(super) async fn record_task_operation(
    app: &AppV3,
    task: &crate::v3_tasks::TaskPrimitive,
    intent: &str,
    session_id: &str,
    tab_id: &str,
    receipts: &[Value],
    verified: bool,
) -> bool {
    if !verified
        || receipts.is_empty()
        || receipts.iter().any(|receipt| receipt["verified"] != true)
    {
        return false;
    }
    let Ok(steps) = task.expand() else {
        return false;
    };
    if steps.len() != receipts.len() {
        return false;
    }
    let Some(memory) = app.action_memory.as_ref() else {
        return false;
    };
    let Ok((origin, structural_signature)) =
        super::super::full::current_skill_context_core(&app.core, session_id, tab_id).await
    else {
        return false;
    };
    let key = crate::page_action_memory::ActionMemoryKey {
        origin,
        structural_signature,
        intent: format!("operation:{intent}"),
    };
    let operation = crate::page_action_memory::OperationPlan {
        steps: steps
            .iter()
            .enumerate()
            .map(|(index, step)| operation_step_for_expanded(step, index))
            .collect(),
        effect: if matches!(task, crate::v3_tasks::TaskPrimitive::Research { .. }) {
            crate::page_action_memory::OperationEffect::ReadOnly
        } else {
            crate::page_action_memory::OperationEffect::Mutating
        },
        successes: 1,
        promoted: false,
    };
    memory
        .lock()
        .await
        .record_operation(key, operation, crate::verifier::Verification::Passed)
}

pub(super) fn read_only_research_skill_steps(
    mut workflow: crate::browser_workflow::BrowserWorkflowDefinition,
    url: &str,
    selector: &str,
) -> Option<Vec<crate::browser_workflow::BrowserWorkflowStep>> {
    use crate::browser_workflow::{
        BrowserWorkflowPredicate as Predicate, BrowserWorkflowStep as Step,
    };
    if !super::safe_persistable_skill_url(url) || workflow.steps.is_empty() || workflow.steps.len() > 40 {
        return None;
    }
    let requested_scope = super::site_scope_from_url(url)?;
    let mut saw_extract = false;
    let mut final_url_verify = false;
    for (index, step) in workflow.steps.iter_mut().enumerate() {
        match step {
            Step::Navigate { url: target }
                if index == 0
                    && super::safe_persistable_skill_url(target)
                    && super::site_scope_from_url(target).as_deref() == Some(&requested_scope) =>
            {
                *target = url.to_owned();
            }
            Step::Extract {
                selector: selected, ..
            } if selected == selector => {
                saw_extract = true;
                final_url_verify = false;
            }
            Step::Assert {
                predicate: Predicate::Url { equals },
            }
            | Step::Verify {
                predicate: Predicate::Url { equals },
            } if super::safe_persistable_skill_url(equals)
                && super::site_scope_from_url(equals).as_deref() == Some(&requested_scope) =>
            {
                if !saw_extract {
                    return None;
                }
                *equals = url.to_owned();
                final_url_verify = true;
            }
            Step::Checkpoint => {}
            other if other.effect() == crate::workflow::EffectClass::ReadOnly => return None,
            _ => return None,
        }
    }
    if !saw_extract || !final_url_verify {
        return None;
    }
    Some(workflow.steps)
}

/// A lost reference is a typed transition, not an undifferentiated "stale"
/// error. Classify it against the same document and snapshot exactly once.
pub(super) async fn stale_reference_receipt(
    app: &AppV3,
    args: &V3ActArgs,
    reference: &str,
) -> Value {
    let decision = {
        let semantic = app.core.semantic.lock().await;
        semantic
            .get(&super::super::tab_key(&args.session_id, &args.chrome_tab_id))
            .and_then(|state| {
                let identity = state.identities.get(reference)?;
                let items = state.items.values().cloned().collect::<Vec<_>>();
                Some(crate::reference_rebase::classify_rebase(
                    reference,
                    identity,
                    state.document.as_ref(),
                    &items,
                ))
            })
    };
    let rebase = decision.as_ref().map(|decision| decision.receipt());
    let reason_code = match decision.as_ref().map(|decision| decision.class) {
        Some(crate::reference_rebase::RebaseClass::DocumentReplaced) => "document_replaced",
        Some(crate::reference_rebase::RebaseClass::Ambiguous) => "reference_ambiguous",
        Some(crate::reference_rebase::RebaseClass::Removed) => "reference_removed",
        Some(crate::reference_rebase::RebaseClass::Rebound) => "reference_rebound",
        Some(crate::reference_rebase::RebaseClass::Stable) => "reference_still_present",
        None => "stale_reference",
    };
    json!({
        "status":"not_dispatched",
        "action":args.action,
        "reference":reference,
        "reason_code":reason_code,
        "dispatch_acknowledged":false,
        "dispatch_may_have_occurred":false,
        "rebase":rebase,
        "next_action":match reason_code {
            "reference_rebound" => "retry once with the rebound reference",
            "reference_still_present" => "take one fresh snapshot; the retained index moved",
            "document_replaced" => "take a fresh snapshot for the new document",
            "reference_removed" => "take one fresh snapshot; the control is gone",
            _ => "take one fresh snapshot and choose a unique target",
        }
    })
}

/// One cheap text-conflict decision before any input is dispatched.
///
/// A fill or type names the value it expects to replace. If the retained
/// semantic item exposes a trustworthy value and it is neither the expected
/// value nor the requested value, this is a human/other-agent edit: yield and
/// let the caller take one fresh observation. If it is already the requested
/// value, the desired state holds and no write is needed. Unreadable surfaces
/// fall through to their surface-specific proof.
pub(super) async fn text_conflict_preflight(
    app: &AppV3,
    args: &V3ActArgs,
    reference: &str,
    expected: &str,
    requested: &str,
) -> Option<Value> {
    if reference.starts_with("@f:") || expected.len() > 16_384 || requested.len() > 16_384 {
        return None;
    }
    let observed = {
        let semantic = app.core.semantic.lock().await;
        let item = semantic
            .get(&super::super::tab_key(&args.session_id, &args.chrome_tab_id))?
            .items
            .get(reference)?;
        if item["value_readable"] == false || item["source_kind"] == "same_origin_blank_frame" {
            return None;
        }
        // Rich editors expose a serialized projection, not a canonical value.
        // Their own guard and readback already prove the exact postcondition;
        // comparing a projected newline/paragraph sentinel here would refuse a
        // legitimate append.
        if item["editable_kind"] == "rich_text" {
            return None;
        }
        item["raw_value"].as_str().map(str::to_owned)
    };
    let mut observed = observed?;
    // A blank rich editor commonly exposes a lone newline (or other markdown
    // whitespace) for an empty value. Canonicalizing that sentinel is required
    // before it can be compared to an empty expected_value.
    if expected.is_empty() && observed.trim().is_empty() {
        observed.clear();
    }
    let conflict = crate::text_conflict::classify_text_conflict(expected, requested, Some(&observed));
    match conflict.class {
        crate::text_conflict::TextConflictClass::None
        | crate::text_conflict::TextConflictClass::Unreadable => None,
        crate::text_conflict::TextConflictClass::AlreadyAtRequestedValue => Some(json!({
            "status":"verified",
            "verified":true,
            "action":args.action,
            "reference":reference,
            "dispatch_acknowledged":false,
            "dispatch_may_have_occurred":false,
            "no_write_needed":true,
            "text_conflict":conflict.receipt(),
            "reason":"the requested value is already present; no input was dispatched",
            "next_action":"none"
        })),
        crate::text_conflict::TextConflictClass::HumanEdited => Some(json!({
            "status":"not_dispatched",
            "verified":false,
            "action":args.action,
            "reference":reference,
            "reason_code":"same_field_human_edit_detected",
            "dispatch_acknowledged":false,
            "dispatch_may_have_occurred":false,
            "text_conflict":conflict.receipt(),
            "reason":"the live value changed after the caller's expected_value; overwriting would discard an edit this call did not make",
            "next_action":"take one fresh snapshot; decide whether to preserve the live value or supply a new expected_value"
        })),
    }
}

/// True when the editable element's DOM value cannot represent what was typed.
///
/// Canvas-backed editors paint their text (Google Docs), and some host the
/// editable element in a blank same-origin frame that only relays key events.
/// The snapshot marks those items `value_readable: false`; the blank-frame
/// `source_kind` is kept as a fallback for observations taken before that field
/// existed. A value precondition is meaningless on these surfaces, so demanding
/// one is how an empty Google Doc refused an empty `expected_value`.
pub(super) async fn editor_value_unreadable(app: &AppV3, args: &V3ActArgs, reference: &str) -> bool {
    app.core
        .semantic
        .lock()
        .await
        .get(&super::super::tab_key(&args.session_id, &args.chrome_tab_id))
        .and_then(|state| state.items.get(reference))
        .is_some_and(|item| {
            item["value_readable"] == false
                || item["source_kind"] == "same_origin_blank_frame"
        })
}

/// Probe for the hidden-canvas refusal; the policy itself lives in
/// `v3_execution::canvas` so the facade only reads the two inputs it needs.
pub(super) async fn hidden_canvas_editor_refusal(app: &AppV3, args: &V3ActArgs) -> Option<Value> {
    let reference = args.reference.as_deref()?;
    let unreadable = editor_value_unreadable(app, args, reference).await;
    let state = super::presentation_state(app, &args.session_id, &args.chrome_tab_id).await?;
    crate::v3_execution::canvas::hidden_canvas_editor_refusal(
        unreadable,
        state["visibility_state"].as_str(),
    )
}



pub(super) fn task_snapshot_goal(task: &crate::v3_tasks::TaskPrimitive) -> Option<super::super::SnapshotGoal> {
    let filter = crate::task_observation::TaskObservationFilter::for_task(task);
    if filter.query.is_none() && filter.required_roles.is_empty() {
        return None;
    }
    Some(super::super::SnapshotGoal {
        query: filter.query,
        roles: (!filter.required_roles.is_empty()).then_some(filter.required_roles),
        href_hint: None,
        within: None,
        exhaustive: filter.exhaustive,
        model_profile: None,
        needs_layout: false,
    })
}

pub(super) fn research_profile_input(
    task: &crate::v3_tasks::TaskPrimitive,
    operation: &str,
    url: &str,
) -> Option<String> {
    let goal = match task {
        crate::v3_tasks::TaskPrimitive::Research {
            goal: Some(goal), ..
        } => goal.trim(),
        _ => "",
    };
    let lower = goal.to_ascii_lowercase();
    let extracted = match operation {
        "search" => {
            let value = ["search for ", "search ", "find ", "look for "]
                .iter()
                .find_map(|prefix| lower.strip_prefix(prefix).map(|_| &goal[prefix.len()..]))
                .unwrap_or(goal);
            (!value.trim().is_empty()).then(|| value.trim().to_owned())
        }
        "filter" => ["filter results by ", "filter by "]
            .iter()
            .find_map(|prefix| {
                lower
                    .strip_prefix(prefix)
                    .map(|_| goal[prefix.len()..].trim().to_owned())
            })
            .filter(|value| !value.is_empty()),
        "sort" => lower
            .strip_prefix("sort by ")
            .map(|_| goal[8..].trim().to_owned())
            .filter(|value| !value.is_empty()),
        "paginate" => {
            let tokens = lower
                .split(|ch: char| !ch.is_ascii_alphanumeric())
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>();
            tokens
                .windows(2)
                .find_map(|pair| {
                    (pair[0] == "page" && pair[1].bytes().all(|b| b.is_ascii_digit()))
                        .then(|| pair[1].to_owned())
                })
                .filter(|value| value.parse::<u32>().is_ok_and(|n| n > 0))
        }
        _ => None,
    };
    let input = extracted.or_else(|| {
        (operation == "search")
            .then(|| super::research_query(url))
            .flatten()
    })?;
    (input.len() <= 512).then_some(input)
}
pub(super) fn page_tool_signature(name: &str, schema: &Value) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(
        [
            name.as_bytes(),
            &serde_json::to_vec(schema).unwrap_or_default(),
        ]
        .concat(),
    )
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect()
}


pub(super) fn finalize_visual_requirement(route: &mut Value, result: &mut Value) {
    if route["visual_evidence_required"] != true {
        return;
    }
    let evidence_present = result["receipts"].as_array().is_some_and(|receipts| {
        receipts
            .iter()
            .any(|receipt| receipt["targeted_visual_evidence"]["status"] == "verified_read")
    });
    if result["status"] == "completed" {
        result["status"] = json!(if evidence_present {
            "visual_evidence_ready"
        } else {
            "incomplete_visual_evidence"
        });
    }
    route["verification"] = json!(if evidence_present {
        "required"
    } else {
        "inconclusive"
    });
    route["visual_reasoning"] = json!(if evidence_present {
        "required_from_attached_image_content"
    } else {
        "visual_probe_unavailable"
    });
}

