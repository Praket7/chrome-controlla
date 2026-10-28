use super::*;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::{Json, Parameters},
    tool, tool_handler, tool_router,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const MAX_WORKFLOW_STEPS: usize = 20;
const DEFAULT_RECEIPT_RESPONSE_BYTES: usize = 2 * 1024;
const MAX_RECEIPT_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_TARGETED_RESPONSE_BYTES: usize = 12 * 1024;
const DEFAULT_BROAD_RESPONSE_BYTES: usize = 100_000;

fn snapshot_collection_budget(response_bytes: usize, text_chars: usize) -> usize {
    // Keep the collector's text/metadata reserve, but do not collect 100 KB
    // merely to discard it at the public response boundary.
    response_bytes
        .max(text_chars.min(6000) * 6 + 4096)
        .clamp(4096, 100_000)
}

fn default_snapshot_response_bytes(targeted_compact: bool) -> usize {
    if targeted_compact {
        DEFAULT_TARGETED_RESPONSE_BYTES
    } else {
        DEFAULT_BROAD_RESPONSE_BYTES
    }
}

fn validate_response_budget(
    requested: Option<usize>,
    reason: Option<&str>,
    default_bytes: usize,
) -> Result<(), &'static str> {
    if requested.is_some_and(|bytes| bytes > default_bytes)
        && !reason.is_some_and(|value| !value.trim().is_empty())
    {
        return Err("max_bytes above the default response budget requires a non-empty max_bytes_reason");
    }
    Ok(())
}

fn select_response_budget(
    requested: Option<usize>,
    reason: Option<&str>,
    default_bytes: usize,
) -> (usize, &'static str, String) {
    let requested = requested.map(|value| value.clamp(1024, 1_000_000));
    let reason = reason.map(str::trim).filter(|value| !value.is_empty());
    match requested {
        Some(bytes) if bytes > default_bytes && reason.is_some() => (
            bytes,
            "explicit",
            reason.unwrap_or_default().chars().take(256).collect(),
        ),
        Some(bytes) if bytes <= default_bytes => (
            bytes,
            "explicit",
            "caller requested a smaller response limit".to_owned(),
        ),
        _ => (
            default_bytes,
            "default",
            if default_bytes == DEFAULT_TARGETED_RESPONSE_BYTES {
                "small targeted compact observation".to_owned()
            } else {
                "broad or rich observation needs a larger context budget".to_owned()
            },
        ),
    }
}

#[derive(Clone, Default)]
struct SemanticTabState {
    document_key: String,
    document: Option<crate::semantic_state::SemanticDocumentKey>,
    next_ref: u32,
    root_scope_complete: bool,
    key_to_short: BTreeMap<String, String>,
    short_to_legacy: BTreeMap<String, String>,
    identities: BTreeMap<String, crate::reference_reacquire::SemanticIdentity>,
    items: BTreeMap<String, Value>,
    snapshot_id: String,
    scope_complete: bool,
}

impl SemanticTabState {
    fn page_ordered_items(&self) -> Vec<&Value> {
        let mut items = self.items.values().collect::<Vec<_>>();
        items.sort_by_key(|item| item["index"].as_u64().unwrap_or(u64::MAX));
        items
    }
}

fn invalidate_semantic_tab(state: &mut SemanticTabState) {
    let next_ref = state.next_ref.max(1);
    *state = SemanticTabState { next_ref, ..Default::default() };
}

fn reset_semantic_document(state: &mut SemanticTabState, doc_key: &str, snapshot: &Value) {
    // Keep public references monotonic for this paired tab even when the
    // document changes. This prevents an old @cN from aliasing a new item.
    let next_ref = state.next_ref.max(1);
    *state = SemanticTabState {
        document_key: doc_key.to_owned(),
        document: Some(crate::semantic_state::SemanticDocumentKey::from_snapshot(
            snapshot,
        )),
        next_ref,
        ..Default::default()
    };
}

#[derive(Clone)]
struct AppV2 {
    legacy: App,
    semantic: Arc<Mutex<BTreeMap<String, SemanticTabState>>>,
    observation_history: Arc<std::sync::Mutex<crate::observation_router::ObservationHistory>>,
}

impl AppV2 {
    fn new(legacy: App) -> Self {
        Self {
            legacy,
            semantic: Arc::default(),
            observation_history: Arc::new(std::sync::Mutex::new(
                crate::observation_router::ObservationHistory::open(
                    super::state_directory().join("observation-costs-v1.json"),
                ),
            )),
        }
    }
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserSessionArgs {
    action: String,
    provider: Option<String>,
    session_id: Option<String>,
    target_ids: Option<Vec<String>>,
    host_id: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserSnapshotArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: Option<String>,
    max_items: Option<usize>,
    max_text_chars: Option<usize>,
    #[schemars(description = "Targeted compact semantic observations default to 12 KiB; broad or rich observations use a larger bounded default. Set max_bytes_reason when requesting more than the default (up to 1 MiB).")]
    max_bytes: Option<usize>,
    #[schemars(description = "Required explanation when max_bytes exceeds the selected default response budget.")]
    max_bytes_reason: Option<String>,
    href_hint: Option<String>,
    frame_id: Option<String>,
    #[schemars(description = "Read bounded accessibility trees from already inventoried child frames and report readable/unread coverage. This is opt-in; root snapshot remains the default.")]
    include_frames: Option<bool>,
    #[schemars(
        description = "Optional targeted search. A match is a bounded candidate, not proof that no better or duplicate candidate exists; inspect global_complete and scope_complete before claiming exhaustive coverage."
    )]
    goal: Option<super::SnapshotGoal>,
    #[schemars(description = "Optional node-visit ceiling for targeted snapshots (1..65536).")]
    max_scan_nodes: Option<usize>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserFindArgs {
    session_id: String,
    chrome_tab_id: String,
    query: String,
    role: Option<String>,
    limit: Option<usize>,
    max_matches: Option<usize>,
    match_mode: Option<String>,
    #[schemars(
        description = "Optional semantic ancestor that narrows candidate matching to its subtree. Root-document scoped queries can use root-local completeness when unrelated frames are unread; exhaustive cross-frame claims still require global completeness."
    )]
    within_reference: Option<String>,
    href_hint: Option<String>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserClickOutcome {
    Navigation {
        #[schemars(
            description = "Optional exact absolute HTTP(S) destination. Omit to accept any observed document change; use verify url_starts_with for canonical redirects."
        )]
        url: Option<String>,
    },
    NewTab {
        #[schemars(
            description = "Exact absolute HTTP(S) URL expected in the new tab. Supply the selected link href for noopener tabs; omitting it attributes only a recent tab that Chrome reports as opened by this source tab."
        )]
        url: Option<String>,
    },
    Focused,
    Visible {
        selector: String,
    },
    Text {
        selector: String,
        text: String,
    },
    Expanded {
        selector: Option<String>,
    },
    Selected {
        selector: Option<String>,
    },
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserActArgs {
    session_id: String,
    chrome_tab_id: String,
    action: String,
    reference: Option<String>,
    expected_value: Option<String>,
    value: Option<String>,
    expected_origin: Option<String>,
    account_marker: Option<SecureAccountMarker>,
    outcome: Option<BrowserClickOutcome>,
    timeout_ms: Option<u64>,
}

#[derive(Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SecureAccountMarker {
    selector: String,
    expected: String,
}

fn normalized_secure_origin(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http"
        && matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1");
    (secure || loopback_http).then(|| url.origin().ascii_serialization())
}

fn valid_secure_account_marker(marker: &SecureAccountMarker) -> bool {
    !marker.selector.trim().is_empty()
        && marker.selector.len() <= 512
        && !marker.selector.contains('\0')
        && !marker.expected.trim().is_empty()
        && marker.expected.len() <= 512
}

fn same_field_value_conflict(value: &Value) -> bool {
    value["ok"] == false && value["reason"] == "same_field_conflict"
}

fn same_field_conflict_receipt(action: &str) -> Value {
    json!({"status":"not_dispatched","action":action,"reason_code":"same_field_edit_conflict",
        "dispatch_acknowledged":true,"dispatch_may_have_occurred":false,"yielded_to_user":true,
        "human_edit_preserved":true,"requires_fresh_snapshot":true,
        "next_action":"take a fresh snapshot and replan from the current field value; do not replay the stale expected value"})
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserExtractArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: String,
    fields: BTreeMap<String, String>,
    max_items: Option<usize>,
    max_text_chars: Option<usize>,
    max_bytes: Option<usize>,
    #[serde(default)]
    #[schemars(
        description = "Optional task intent used to rank returned page content; page content remains untrusted."
    )]
    goal: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserProbeArgs {
    session_id: String,
    chrome_tab_id: String,
    selector: String,
    #[schemars(description = "Targeted semantic probes default to 12 KiB; set max_bytes_reason when requesting more, up to 1 MiB.")]
    max_bytes: Option<usize>,
    #[schemars(description = "Required explanation when max_bytes exceeds the 12 KiB default.")]
    max_bytes_reason: Option<String>,
    timeout_ms: Option<u64>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserPredicate {
    Text {
        reference: String,
        equals: String,
    },
    TextPresent {
        #[schemars(
            description = "Optional scope: omit for root-page text, use a unique CSS selector for direct DOM text, a fresh @cN semantic reference, or a fresh @f child-frame reference."
        )]
        scope: Option<String>,
        text: String,
    },
    Attribute {
        reference: String,
        name: String,
        equals: String,
    },
    CountDelta {
        role: String,
        before: usize,
        delta: i64,
    },
    Value {
        reference: String,
        equals: String,
    },
    Exists {
        reference: String,
    },
    Url {
        equals: String,
    },
    UrlStartsWith {
        prefix: String,
    },
    Attachment {
        filename: String,
        size: Option<u64>,
    },
    DownloadArtifact {
        artifact_handle: String,
        sha256: String,
        min_bytes: u64,
    },
    VisualRegionChanged {
        evidence_before: String,
        evidence_after: String,
    },
    AppSpecific {
        #[schemars(
            description = "Registered versioned UI predicate: semantic_field_equals, Canva slide/active-slide, Google Drive picker selection, or Google Classroom attachment. App predicates prove caller-visible UI evidence only."
        )]
        adapter: String,
        #[schemars(
            description = "Canva manifest: exact URL/count and ordered fresh role/name/reference/ID. Active slide: exact URL/name/ID, uniquely selected in a complete snapshot. Drive picker: exact visible account, file ID and selection state. Classroom: exact account/class/assignment identities and attachment filename/size. All require a complete fresh semantic snapshot."
        )]
        predicate: serde_json::Value,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TextPredicateScope {
    Root,
    Reference(String),
    ScopedSelector(String),
    FrameReference(String),
}

fn parse_text_predicate_scope(scope: Option<&str>) -> Result<TextPredicateScope, &'static str> {
    let Some(scope) = scope else {
        return Ok(TextPredicateScope::Root);
    };
    if scope.starts_with("@f:") {
        frame_scope_parts(scope).ok_or("frame scope must be a fresh @f frame reference")?;
        return Ok(TextPredicateScope::FrameReference(scope.to_owned()));
    }
    if let Some(reference_id) = scope.strip_prefix("@c") {
        if reference_id.is_empty()
            || scope.len() > 64
            || !reference_id.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("semantic reference scope is malformed");
        }
        return Ok(TextPredicateScope::Reference(scope.to_owned()));
    }
    if scope.trim().is_empty() || scope.len() > 2_048 || scope.contains('\0') {
        return Err("scoped text selector is out of bounds");
    }
    Ok(TextPredicateScope::ScopedSelector(scope.to_owned()))
}

fn urls_identify_same_resource(expected: &str, actual: &str) -> bool {
    match (url::Url::parse(expected), url::Url::parse(actual)) {
        (Ok(expected), Ok(actual))
            if matches!(expected.scheme(), "http" | "https")
                && expected.scheme() == actual.scheme() =>
        {
            expected == actual
        }
        _ => expected == actual,
    }
}

fn text_presence_status(found: bool, truncated: bool) -> &'static str {
    if found {
        "passed"
    } else if truncated {
        "incomplete"
    } else {
        "failed"
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserVerifyArgs {
    session_id: String,
    chrome_tab_id: String,
    predicate: BrowserPredicate,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserWorkflowStep {
    Snapshot,
    Find {
        query: String,
        role: Option<String>,
        save_as: String,
    },
    Fill {
        reference: String,
        expected_value: String,
        value: String,
    },
    Select {
        reference: String,
        expected_value: String,
        value: String,
    },
    Click {
        reference: String,
        outcome: BrowserClickOutcome,
    },
    Verify {
        predicate: BrowserPredicate,
    },
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserWorkflowArgs {
    session_id: String,
    chrome_tab_id: String,
    steps: Vec<BrowserWorkflowStep>,
    #[schemars(description = "Workflow receipts default to 2 KiB. Larger responses are honored only with max_bytes_reason explaining why the extra step evidence is needed; maximum 1 MiB.")]
    max_bytes: Option<usize>,
    #[schemars(description = "Explain why the workflow needs a receipt larger than the 2 KiB default.")]
    max_bytes_reason: Option<String>,
}

#[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserSkillArgs {
    action: String,
    skill_id: Option<String>,
}

fn tab_key(session_id: &str, chrome_tab_id: &str) -> String {
    format!("{session_id}\u{1f}{chrome_tab_id}")
}

fn document_key(snapshot: &Value) -> String {
    let frame = snapshot
        .pointer("/document/frame_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let loader = snapshot
        .pointer("/document/loader_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = snapshot
        .pointer("/document/url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{frame}\u{1f}{loader}\u{1f}{url}")
}

fn semantic_string_field_equals(item: Option<&Value>, field: &str, expected: &str) -> bool {
    let Some(actual) = item.and_then(|item| item.get(field)) else {
        return false;
    };
    actual.as_str().is_some_and(|actual| actual == expected)
        || (matches!(field, "selected" | "expanded")
            && actual
                .as_bool()
                .is_some_and(|actual| actual.to_string() == expected))
}

fn semantic_text_is_authoritative(item: &Value) -> bool {
    item["feedback_region"] != true
        && !matches!(item["role"].as_str(), Some("alert" | "status" | "log"))
}

fn semantic_count_delta(
    items: &BTreeMap<String, Value>,
    role: &str,
    before: usize,
    delta: i64,
) -> (bool, usize) {
    let actual = items
        .values()
        .filter(|item| item["role"].as_str() == Some(role))
        .count();
    let expected = before as i128 + delta as i128;
    (
        expected >= 0 && expected <= usize::MAX as i128 && actual as i128 == expected,
        actual,
    )
}

fn changed_fields(previous: &Value, current: &Value) -> bool {
    [
        "role",
        "name",
        "raw_value",
        "href",
        "expanded",
        "selected",
        "disabled",
        "requires_trusted_events",
        "present",
        "rendered",
        "in_viewport",
        "pointer_events_none",
        "actionability",
        "editable_kind",
        "multiline",
    ]
    .into_iter()
    .any(|field| previous.get(field) != current.get(field))
}

pub(super) fn update_observation_bytes(value: &mut Value) -> u64 {
    if value.get("observation").is_none() {
        return 0;
    }
    let mut bytes = 0;
    for _ in 0..3 {
        bytes = serde_json::to_vec(value).map_or(0, |encoded| encoded.len() as u64);
        if value["observation"]["bytes"] == json!(bytes) {
            break;
        }
        value["observation"]["bytes"] = json!(bytes);
    }
    bytes
}

fn attach_observation_metadata(
    value: &mut Value,
    decision: crate::observation_router::ObservationDecision,
) -> u64 {
    value["observation"] = json!({"mode":decision.mode,"reason":decision.reason,"bytes":0});
    update_observation_bytes(value)
}

fn bound_snapshot_response(value: &mut Value, max_bytes: usize) -> u64 {
    let max_bytes = max_bytes.max(1);
    if value.pointer("/delta/full").and_then(Value::as_bool) == Some(true) {
        value["delta"]["upsert"] = json!([]);
    }
    let original_count = value["count"].as_u64().unwrap_or_else(|| {
        value["items"]
            .as_array()
            .map_or(0, |items| items.len() as u64)
    });
    let mut truncated = false;
    loop {
        if truncated {
            value["truncated"] = json!(true);
            value["truncation_reason"] = json!("response_byte_limit");
            value["scope_complete"] = json!(false);
            value["root_scope_complete"] = json!(false);
            value["global_complete"] = json!(false);
            value["count"] = json!(original_count);
            value["items_returned"] = json!(value["items"].as_array().map_or(0, Vec::len));
        }
        let bytes = update_observation_bytes(value);
        if bytes <= max_bytes as u64 {
            return bytes;
        }
        truncated = true;
        let Some(items) = value["items"].as_array_mut() else {
            break;
        };
        if items.pop().is_none() {
            break;
        }
        let visible_references = value["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["reference"].as_str().map(str::to_owned))
            .collect::<std::collections::HashSet<_>>();
        if let Some(upserts) = value
            .pointer_mut("/delta/upsert")
            .and_then(Value::as_array_mut)
        {
            upserts.retain(|item| {
                item["reference"]
                    .as_str()
                    .is_some_and(|reference| visible_references.contains(reference))
            });
        }
    }
    *value = json!({
        "status":"incomplete",
        "reason_code":"response_byte_limit",
        "max_bytes":max_bytes,
        "response_budget":value["response_budget"],
        "truncated":true,
        "scope_complete":false,
        "observation":{"mode":"bounded","reason":"final snapshot exceeded its byte budget","bytes":0}
    });
    let bytes = update_observation_bytes(value);
    if bytes <= max_bytes as u64 { bytes } else { 0 }
}

fn serialized_bytes(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |encoded| encoded.len())
}

fn bounded_string(value: &Value) -> Option<Value> {
    value.as_str().map(|text| {
        if text.len() <= 256 {
            json!(text)
        } else {
            let end = text.char_indices().take_while(|(index, _)| *index < 256).map(|(index, character)| index + character.len_utf8()).last().unwrap_or(0);
            json!(format!("{}…", &text[..end]))
        }
    })
}

fn compact_workflow_step(step: &Value) -> Value {
    let mut compact = serde_json::Map::new();
    for key in [
        "kind", "status", "reference", "save_as", "reason_code", "action",
        "semantic_revision", "scope_complete", "root_scope_complete", "global_complete",
        "truncated", "evidence_complete",
    ] {
        if let Some(value) = bounded_string(&step[key]) {
            compact.insert(key.to_owned(), value);
        } else if step[key].is_boolean() || step[key].is_number() {
            compact.insert(key.to_owned(), step[key].clone());
        }
    }
    if let Some(reason) = bounded_string(&step["reason"]) {
        compact.insert("reason".to_owned(), reason);
    }
    for key in ["match", "observed", "expected", "reconciliation"] {
        if let Some(object) = step.get(key).and_then(Value::as_object) {
            let mut selected = serde_json::Map::new();
            for field in [
                "reference", "role", "name", "href", "actionable", "rendered",
                "in_viewport", "status", "evidence_complete", "observer",
                "document", "semantic_revision", "reason", "reason_code",
                "causality_established", "mutation_retried",
            ] {
                if let Some(value) = object.get(field) {
                    if let Some(text) = bounded_string(value) {
                        selected.insert(field.to_owned(), text);
                    } else if value.is_boolean() || value.is_number() {
                        selected.insert(field.to_owned(), value.clone());
                    } else if field == "document" {
                        let document = ["frame_id", "loader_id", "url"]
                            .into_iter()
                            .filter_map(|name| {
                                bounded_string(&value[name]).map(|text| (name.to_owned(), text))
                            })
                            .collect::<serde_json::Map<_, _>>();
                        if !document.is_empty() {
                            selected.insert(field.to_owned(), Value::Object(document));
                        }
                    }
                }
            }
            if !selected.is_empty() {
                compact.insert(key.to_owned(), Value::Object(selected));
            }
        }
    }
    if let Some(document) = step.get("document").and_then(Value::as_object) {
        let identity = ["frame_id", "loader_id", "url"]
            .into_iter()
            .filter_map(|name| {
                bounded_string(document.get(name).unwrap_or(&Value::Null))
                    .map(|text| (name.to_owned(), text))
            })
            .collect::<serde_json::Map<_, _>>();
        if !identity.is_empty() {
            compact.insert("document".to_owned(), Value::Object(identity));
        }
    }
    Value::Object(compact)
}

fn bound_workflow_receipt(value: Value, requested_bytes: usize, reason: Option<&str>) -> Value {
    let explicit_reason = reason.map(str::trim).filter(|reason| !reason.is_empty());
    let requested = requested_bytes.clamp(DEFAULT_RECEIPT_RESPONSE_BYTES, MAX_RECEIPT_RESPONSE_BYTES);
    let (max_bytes, source) = if requested > DEFAULT_RECEIPT_RESPONSE_BYTES && explicit_reason.is_some() {
        (requested, "explicit")
    } else {
        (DEFAULT_RECEIPT_RESPONSE_BYTES, "default")
    };
    let budget_reason = if source == "explicit" {
        explicit_reason.unwrap_or_default().chars().take(256).collect::<String>()
    } else {
        "compact task receipt default".to_owned()
    };
    let original_steps = value["steps"].as_array().cloned().unwrap_or_default();
    let original_count = original_steps.len();
    let mut response = json!({
        "status":value["status"],
        "completed_steps":value["completed_steps"],
        "reconciled_unknown_mutations":value["reconciled_unknown_mutations"],
        "internal_model_calls":value["internal_model_calls"],
        "steps":[],
        "returned_steps":0,
        "omitted_steps":original_count,
        "truncated":original_count > 0,
        "response_budget":{"max_bytes":max_bytes,"source":source,"reason":budget_reason}
    });
    for step in original_steps {
        response["steps"].as_array_mut().expect("steps array").push(compact_workflow_step(&step));
        let returned = response["steps"].as_array().map_or(0, Vec::len);
        response["returned_steps"] = json!(returned);
        response["omitted_steps"] = json!(value["completed_steps"].as_u64().unwrap_or(returned as u64).saturating_sub(returned as u64));
        response["truncated"] = json!(response["omitted_steps"].as_u64().unwrap_or(0) > 0);
        if serialized_bytes(&response) > max_bytes {
            response["steps"].as_array_mut().expect("steps array").pop();
            let returned = response["steps"].as_array().map_or(0, Vec::len);
            response["returned_steps"] = json!(returned);
            response["omitted_steps"] = json!(value["completed_steps"].as_u64().unwrap_or(original_count as u64).saturating_sub(returned as u64));
            response["truncated"] = json!(true);
            break;
        }
    }
    response
}

fn query_needs_layout(query: &str) -> bool {
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .collect::<std::collections::BTreeSet<_>>();
    [
        "layout", "chart", "graph", "map", "canvas", "diagram", "overlap", "position",
    ]
    .iter()
    .any(|term| terms.contains(*term))
}

fn observation_task_class(query: Option<&str>) -> crate::observation_router::TaskClass {
    let words = query
        .unwrap_or_default()
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .collect::<std::collections::BTreeSet<_>>();
    if ["dashboard", "filter", "sort", "chart", "table"]
        .iter()
        .any(|word| words.contains(*word))
    {
        crate::observation_router::TaskClass::FilterSortDashboard
    } else if [
        "fill", "submit", "save", "delete", "send", "purchase", "edit",
    ]
    .iter()
    .any(|word| words.contains(*word))
    {
        crate::observation_router::TaskClass::Mutation
    } else if query.is_some() {
        crate::observation_router::TaskClass::Research
    } else {
        crate::observation_router::TaskClass::General
    }
}

fn scoped_html_expression(selector: &str, max_chars: usize) -> String {
    format!(
        r#"(()=>{{let root;try{{const matches=[...document.querySelectorAll({selector})];if(matches.length!==1)return {{ok:false,reason:matches.length===0?'no_match':'ambiguous',count:matches.length}};root=matches[0].cloneNode(true)}}catch(_){{return {{ok:false,reason:'invalid_selector'}}}}root.querySelectorAll('script,style,noscript,iframe,object,embed,svg,input,textarea,select,button').forEach(node=>node.remove());for(const node of [root,...root.querySelectorAll('*')]){{for(const attr of [...node.attributes]){{if(!['id','class','role','aria-label','aria-expanded','aria-selected','aria-controls','data-state'].includes(attr.name))node.removeAttribute(attr.name)}}}}return {{ok:true,html:root.outerHTML.slice(0,{max_chars}),truncated:root.outerHTML.length>{max_chars}}}}})()"#,
        selector = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
        max_chars = max_chars.clamp(1, 32_768),
    )
}

async fn scoped_html_view(
    app: &AppV2,
    session_id: &str,
    tab_id: &str,
    selector: &str,
    max_bytes: usize,
) -> Result<Value, rmcp::ErrorData> {
    if selector.trim().is_empty() || selector.len() > 2048 || selector.contains('\0') {
        return Err(invalid("scoped HTML requires one bounded CSS selector"));
    }
    let shared = app
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let shared = shared.lock().await;
    if shared.handle.principal != app.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let registry = shared.registry()?;
    let before = super::shared_frame_identity(connection, registry, &shared.handle, tab_id)
        .await
        .map_err(invalid)?;
    let expression = scoped_html_expression(selector, max_bytes.min(32_768));
    let response = connection
        .command(
            registry,
            &shared.handle,
            tab_id,
            "Runtime.evaluate",
            serde_json::json!({"expression":expression,"returnByValue":true}),
        )
        .await
        .map_err(|error| invalid(error.to_string()))?;
    let mut result = super::shared_value(&response).map_err(invalid)?;
    let after = super::shared_frame_identity(connection, registry, &shared.handle, tab_id)
        .await
        .map_err(invalid)?;
    if before != after {
        return Err(invalid("document changed during scoped HTML observation"));
    }
    if result["ok"] != true {
        return Err(invalid(format!(
            "scoped HTML unavailable: {}",
            result["reason"].as_str().unwrap_or("unknown")
        )));
    }
    let html = result["html"].as_str().unwrap_or_default().to_owned();
    let limit = max_bytes.clamp(4096, 32_768);
    let mut bounded = String::new();
    for character in html.chars() {
        if bounded.len() + character.len_utf8() > limit {
            break;
        }
        bounded.push(character);
    }
    result["html"] = json!(bounded);
    result["truncated"] = json!(result["truncated"] == true || bounded.len() < html.len());
    result["document"] = json!({"frame_id":before.0,"loader_id":before.1,"url":before.2});
    result["page_content_is_untrusted"] = json!(true);
    Ok(result)
}

async fn snapshot_impl(app: &AppV2, args: &BrowserSnapshotArgs) -> Result<Value, rmcp::ErrorData> {
    let started = std::time::Instant::now();
    // Reuse the last observed candidate set to choose the next snapshot's
    // detail level. This avoids taking a second snapshot just to estimate
    // ambiguity, while still allowing a compact first read to trigger a rich
    // follow-up when it exposed several indistinguishable controls.
    let prior_ambiguity = app
        .semantic
        .lock()
        .await
        .get(&tab_key(&args.session_id, &args.chrome_tab_id))
        .map(|state| semantic_ambiguity(state.items.values()))
        .unwrap_or(0.0);
    let request = crate::observation_router::ObservationRequest {
        task_class: observation_task_class(
            args.goal.as_ref().and_then(|goal| goal.query.as_deref()),
        ),
        model_profile: args
            .goal
            .as_ref()
            .and_then(|goal| goal.model_profile)
            .map(|profile| match profile {
                super::ObservationModelProfile::RichContext => {
                    crate::observation_router::ModelProfile::RichContext
                }
                super::ObservationModelProfile::CompactContext => {
                    crate::observation_router::ModelProfile::CompactContext
                }
            })
            .or_else(|| {
                args.max_text_chars.map(|limit| {
                    if limit > 2_000 {
                        crate::observation_router::ModelProfile::RichContext
                    } else {
                        crate::observation_router::ModelProfile::CompactContext
                    }
                })
            }),
        exhaustive: args.goal.as_ref().is_some_and(|goal| goal.exhaustive),
        needs_layout: args.goal.as_ref().is_some_and(|goal| goal.needs_layout)
            || args
                .goal
                .as_ref()
                .and_then(|goal| goal.query.as_deref())
                .is_some_and(query_needs_layout),
        ambiguity: prior_ambiguity,
        rich_context: args.max_text_chars.is_some_and(|limit| limit > 2_000),
        frame_id: args.frame_id.is_some() || args.include_frames == Some(true),
        scoped_selector: args.selector.is_some(),
        query_signature: stable_intent_signature(
            args.goal
                .as_ref()
                .map(|goal| {
                    format!(
                        "{}|{}",
                        goal.query.as_deref().unwrap_or_default(),
                        goal.roles
                            .as_ref()
                            .map(|roles| roles.join(","))
                            .unwrap_or_default()
                    )
                })
                .as_deref(),
        ),
        scope_signature: stable_intent_signature(Some(&format!("{}|{}|{}", args.session_id, args.chrome_tab_id, stable_intent_signature(
            args.selector
                .as_ref()
                .map(|selector| {
                    format!(
                        "{}|{}",
                        selector,
                        args.goal
                            .as_ref()
                            .and_then(|goal| goal.href_hint.as_deref())
                            .unwrap_or_default()
                    )
                })
                .or_else(|| args.goal.as_ref().and_then(|goal| goal.href_hint.clone()))
                .as_deref(),
        )))),
        query_required: args
            .goal
            .as_ref()
            .and_then(|goal| goal.query.as_deref())
            .is_some_and(|query| !query.trim().is_empty()),
    };
    let decision = app
        .observation_history
        .try_lock()
        .map(|history| {
            crate::observation_router::choose_observation_with_history(request, &history)
        })
        .unwrap_or_else(|_| crate::observation_router::choose_observation(request));
    if args.frame_id.is_some() && args.include_frames == Some(true) {
        return Err(invalid("include_frames applies to a root snapshot; use frame_id for one explicit child-frame snapshot"));
    }
    if let Some(frame_id) = args.frame_id.as_deref() {
        validate_response_budget(args.max_bytes, args.max_bytes_reason.as_deref(), DEFAULT_TARGETED_RESPONSE_BYTES)
            .map_err(invalid)?;
        let (response_max_bytes, response_source, response_reason) = select_response_budget(
            args.max_bytes,
            args.max_bytes_reason.as_deref(),
            DEFAULT_TARGETED_RESPONSE_BYTES,
        );
        let Json(mut value) = app
            .legacy
            .shared_accessibility(Parameters(SharedAccessibilityArgs {
                session_id: args.session_id.clone(),
                chrome_tab_id: args.chrome_tab_id.clone(),
                selector: None,
                frame_id: Some(frame_id.to_owned()),
                max_bytes: response_max_bytes,
                timeout_ms: Some(10_000),
            }))
            .await?;
        value["response_budget"] = json!({
            "max_bytes": response_max_bytes,
            "source": response_source,
            "reason": response_reason
        });
        attach_observation_metadata(&mut value, decision);
        let bytes = bound_snapshot_response(&mut value, response_max_bytes);
        record_observation_feedback(app, request, decision, started, &value, bytes);
        controlla_browser::scheduler::telemetry::record_observation(0, bytes).await;
        controlla_browser::scheduler::telemetry::record_phase(
            controlla_browser::scheduler::telemetry::OperationPhase::Snapshot,
            started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        )
        .await;
        return Ok(value);
    }
    let targeted_compact = decision.mode == crate::observation_router::ObservationMode::SemanticCompact
        && args.goal.as_ref().and_then(|goal| goal.query.as_deref()).is_some_and(|query| !query.trim().is_empty())
        && !args.goal.as_ref().is_some_and(|goal| goal.exhaustive);
    validate_response_budget(
        args.max_bytes,
        args.max_bytes_reason.as_deref(),
        default_snapshot_response_bytes(targeted_compact),
    )
    .map_err(invalid)?;
    let (response_max_bytes, response_source, response_reason) = select_response_budget(
        args.max_bytes,
        args.max_bytes_reason.as_deref(),
        default_snapshot_response_bytes(targeted_compact),
    );
    let collection_text_chars = args.max_text_chars.unwrap_or_else(|| {
        if decision.mode == crate::observation_router::ObservationMode::SemanticRich {
            6_000
        } else {
            snapshot_chars()
        }
    }).min(6000);
    let legacy = app
        .legacy
        .shared_snapshot(Parameters(SharedSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: args.selector.clone().unwrap_or_else(snapshot_scope),
            max_items: args.max_items.unwrap_or_else(snapshot_items).clamp(1, 200),
            max_text_chars: collection_text_chars,
            max_bytes: snapshot_collection_budget(response_max_bytes, collection_text_chars),
            href_hint: args.href_hint.clone(),
            goal: args.goal.clone(),
            max_scan_nodes: args.max_scan_nodes,
        }))
        .await?;
    let Json(mut snapshot) = legacy;
    if args.include_frames == Some(true) {
        let frame_handles = snapshot["frame_handles"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut frame_results = Vec::new();
        let mut unread = Vec::new();
        let children = frame_handles
            .iter()
            .filter(|frame| frame["parent_id"].as_str().is_some())
            .take(8);
        for frame in children {
            let frame_id = frame["frame_id"].as_str().unwrap_or_default();
            if frame_id.is_empty() {
                continue;
            }
            let response = app
                .legacy
                .shared_accessibility(Parameters(SharedAccessibilityArgs {
                    session_id: args.session_id.clone(),
                    chrome_tab_id: args.chrome_tab_id.clone(),
                    selector: None,
                    frame_id: Some(frame_id.to_owned()),
                    max_bytes: (response_max_bytes / 8).clamp(4096, 128_000),
                    timeout_ms: Some(2_000),
                }))
                .await;
            match response {
                Ok(Json(value)) if value["source_frame_id"] == frame_id => {
                    frame_results.push(value);
                }
                Ok(_) => unread.push(json!({"frame_id":frame_id,"reason":"frame_identity_mismatch"})),
                Err(error) => unread.push(json!({"frame_id":frame_id,"reason":error.message})),
            }
        }
        let frame_count = frame_handles
            .iter()
            .filter(|frame| frame["parent_id"].as_str().is_some())
            .count();
        let bounded = frame_count.min(8);
        let truncated = frame_count > bounded || snapshot["frame_handles_truncated"] == true;
        let frame_accessibility = aggregate_frame_accessibility(
            frame_results,
            unread,
            frame_count,
            bounded,
            truncated,
        );
        let complete = frame_accessibility["complete"] == true;
        snapshot["frame_accessibility"] = frame_accessibility;
        if !complete {
            snapshot["complete"] = json!(false);
            snapshot["global_complete"] = json!(false);
            snapshot["scope_complete"] = json!(false);
            let mut reasons = snapshot["incomplete_reasons"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !reasons
                .iter()
                .any(|reason| reason == "frame_accessibility_incomplete")
            {
                reasons.push(json!("frame_accessibility_incomplete"));
            }
            snapshot["incomplete_reasons"] = json!(reasons);
        }
    }
    let items = snapshot
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let intent_query = args.goal.as_ref().and_then(|goal| goal.query.as_deref());
    let items = crate::relevance::filter_items(&items, intent_query, args.max_items.unwrap_or(200));
    let item_keys = crate::semantic_state::stable_item_keys(
        &items,
        snapshot["snapshot_id"].as_str().unwrap_or_default(),
    );
    let key = tab_key(&args.session_id, &args.chrome_tab_id);
    let doc_key = document_key(&snapshot);
    let snapshot_id = snapshot
        .get("snapshot_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let scope_complete = snapshot["scope_complete"] == true;
    let root_scope_complete = snapshot_root_scope_complete(&snapshot);
    snapshot["root_scope_complete"] = json!(root_scope_complete);
    let mut semantic = app.semantic.lock().await;
    let state = semantic.entry(key.clone()).or_default();
    let document_changed = !state.document_key.is_empty() && state.document_key != doc_key;
    if state.document_key != doc_key {
        reset_semantic_document(state, &doc_key, &snapshot);
    }
    state.document = Some(crate::semantic_state::SemanticDocumentKey::from_snapshot(
        &snapshot,
    ));
    state.scope_complete = scope_complete;
    state.root_scope_complete = root_scope_complete;
    if state.next_ref == 0 {
        state.next_ref = 1;
    }
    let previous_items = state.items.clone();
    state.short_to_legacy.clear();
    state.items.clear();
    let mut public_items = Vec::with_capacity(items.len());
    for (index, (mut item, semantic_key)) in items.into_iter().zip(item_keys).enumerate() {
        let short = if let Some(existing) = state.key_to_short.get(&semantic_key) {
            existing.clone()
        } else {
            let new_ref = format!("@c{}", state.next_ref);
            state.next_ref = state.next_ref.saturating_add(1);
            state.key_to_short.insert(semantic_key, new_ref.clone());
            new_ref
        };
        let legacy_ref = item
            .get("reference")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("shared snapshot item omitted its retained reference"))?
            .to_owned();
        item["reference"] = json!(short);
        item["index"] = json!(index);
        if let Some(adapter) = crate::widget_adapters::resolve_widget(&item) {
            item["widget_adapter"] = json!({
                "kind": adapter.kind,
                "postcondition": adapter.live_postcondition,
                "fallback": adapter.fallback
            });
        }
        let identity = crate::reference_reacquire::SemanticIdentity::from_item(
            &item,
            state.document.as_ref().expect("snapshot document was set"),
        );
        state.identities.insert(short.clone(), identity);
        state.short_to_legacy.insert(short.clone(), legacy_ref);
        state.items.insert(short, item.clone());
        public_items.push(item);
    }
    let mut upsert = Vec::new();
    for (reference, item) in &state.items {
        match previous_items.get(reference) {
            Some(previous) if !changed_fields(previous, item) => {}
            _ => upsert.push(item.clone()),
        }
    }
    let removed = previous_items
        .keys()
        .filter(|reference| !state.items.contains_key(*reference))
        .cloned()
        .collect::<Vec<_>>();
    state.snapshot_id = snapshot_id.clone();
    snapshot["items"] = json!(public_items);
    snapshot["semantic_revision"] = json!(snapshot_id);
    snapshot["delta"] = json!({
        "full": previous_items.is_empty() || document_changed,
        "document_changed": document_changed,
        "upsert": upsert,
        "removed": removed
    });
    drop(semantic);
    if decision.mode == crate::observation_router::ObservationMode::HtmlScoped {
        if let Some(selector) = args.selector.as_deref() {
            snapshot["scoped_html"] = scoped_html_view(
                app,
                &args.session_id,
                &args.chrome_tab_id,
                selector,
                args.max_bytes.unwrap_or(32_768).min(32_768),
            )
            .await?;
        }
        snapshot["page_content_is_untrusted"] = json!(true);
    } else if decision.mode == crate::observation_router::ObservationMode::VisualTargeted {
        if let Some(selector) = args.selector.as_deref() {
            let visual = full::visual_probe(
                app,
                full::FullProbeArgs {
                    session_id: args.session_id.clone(),
                    chrome_tab_id: args.chrome_tab_id.clone(),
                    reference: None,
                    selector: Some(selector.to_owned()),
                    region: None,
                    full_page: None,
                    max_bytes: Some(args.max_bytes.unwrap_or(262_144).min(262_144)),
                    timeout_ms: Some(10_000),
                    save_artifact: None,
                    omit_image_payload: None,
                },
            )
            .await?;
            if visual["visual_ref"]["frame_id"] == snapshot["document"]["frame_id"]
                && visual["visual_ref"]["loader_id"] == snapshot["document"]["loader_id"]
            {
                snapshot["visual_probe"] = visual;
            } else {
                return Err(invalid(
                    "document changed between semantic and targeted visual observations",
                ));
            }
        }
        snapshot["page_content_is_untrusted"] = json!(true);
    }
    snapshot["response_budget"] = json!({
        "max_bytes": response_max_bytes,
        "source": response_source,
        "reason": response_reason
    });
    attach_observation_metadata(&mut snapshot, decision);
    let bytes = bound_snapshot_response(&mut snapshot, response_max_bytes);
    if snapshot["truncation_reason"] == "response_byte_limit"
        && let Some(state) = app.semantic.lock().await.get_mut(&key)
    {
        state.scope_complete = false;
        state.root_scope_complete = false;
    }
    record_observation_feedback(app, request, decision, started, &snapshot, bytes);
    controlla_browser::scheduler::telemetry::record_observation(
        snapshot["scanned_nodes"].as_u64().unwrap_or(0),
        bytes,
    )
    .await;
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Snapshot,
        started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    )
    .await;
    Ok(snapshot)
}

fn aggregate_frame_accessibility(
    frames: Vec<Value>,
    unread: Vec<Value>,
    frame_count: usize,
    attempted: usize,
    truncated: bool,
) -> Value {
    let trees_complete = frames.iter().all(|frame| frame["complete"] == true);
    let complete = unread.is_empty()
        && !truncated
        && trees_complete
        && frames.len() == frame_count;
    json!({
        "frames":frames,
        "unread":unread,
        "read_count":frames.len(),
        "frame_count_observed":frame_count,
        "frame_count_attempted":attempted,
        "truncated":truncated,
        "complete":complete,
        "evidence_scope":"bounded_accessibility_trees_with_source_frame_identity"
    })
}

fn record_observation_feedback(
    app: &AppV2,
    request: crate::observation_router::ObservationRequest,
    decision: crate::observation_router::ObservationDecision,
    started: std::time::Instant,
    value: &Value,
    bytes: u64,
) {
    let usable = observation_is_usable(request, value);
    // Telemetry may be dropped under pressure; it cannot stall a page action
    // or create an unbounded queue of disk writers.
    static RECORDERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let Ok(permit) = RECORDERS.try_acquire() else { return; };
    let history = app.observation_history.clone();
    let elapsed_ms = started.elapsed().as_millis().clamp(1, u64::MAX as u128) as u64;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if let Ok(mut history) = history.lock() {
            history.record(request, decision, elapsed_ms, bytes, usable);
        }
    });
}

fn observation_is_usable(
    request: crate::observation_router::ObservationRequest,
    value: &Value,
) -> bool {
    let has_content = value["items"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
        || value["nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty())
        || value["global_complete"] == true;
    let scope_complete = value["scope_complete"] == true
        || value["complete"] == true
        || value["global_complete"] == true;
    let no_truncation = value["omitted_by_limit"].as_u64().unwrap_or(0) == 0
        && value["omitted_oversized"].as_u64().unwrap_or(0) == 0
        && (value["text_requested"] != true || value["completeness"]["text"] == "complete");
    let intent_satisfied = !request.query_required || value["goal_satisfied"] == true;
    let explicitly_failed = matches!(
        value["status"].as_str(),
        Some("failed" | "unknown" | "incomplete")
    );
    has_content && scope_complete && no_truncation && intent_satisfied && !explicitly_failed
}

fn stable_intent_signature(value: Option<&str>) -> u64 {
    value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        })
}

fn semantic_ambiguity<'a>(items: impl Iterator<Item = &'a Value>) -> f32 {
    let mut counts = std::collections::BTreeMap::<(String, String), usize>::new();
    let mut total = 0usize;
    for item in items {
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if role.is_empty() || name.is_empty() {
            continue;
        }
        total += 1;
        *counts
            .entry((role.to_ascii_lowercase(), name.to_ascii_lowercase()))
            .or_default() += 1;
    }
    if total == 0 {
        return 0.0;
    }
    let collisions = counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum::<usize>();
    (collisions as f32 / total as f32).clamp(0.0, 1.0)
}

async fn resolve_reference(
    app: &AppV2,
    session_id: &str,
    tab_id: &str,
    reference: &str,
) -> Result<String, rmcp::ErrorData> {
    if !reference.starts_with("@c") || reference[2..].parse::<u32>().is_err() {
        return Err(invalid(
            "reference must be an exact @cN value returned by browser_snapshot/browser_find",
        ));
    }
    let mut semantic = app.semantic.lock().await;
    let state = semantic
        .get_mut(&tab_key(session_id, tab_id))
        .ok_or_else(|| invalid("reference is stale or unknown; take a fresh browser_snapshot"))?;
    if let Some(legacy) = state.short_to_legacy.get(reference) {
        return Ok(legacy.clone());
    }
    let expected = state
        .identities
        .get(reference)
        .ok_or_else(|| invalid("reference is stale or unknown; take a fresh browser_snapshot"))?;
    let current_document = state.document.as_ref().ok_or_else(|| {
        invalid("semantic document identity is unavailable; take a fresh browser_snapshot")
    })?;
    let items = state.items.values().cloned().collect::<Vec<_>>();
    let candidate = crate::target_registry::rebind_result(expected, Some(current_document), &items)
        .map_err(|error| {
            invalid(format!("reference could not be rebound safely: {error}"))
        })?;
    let current = state
        .items
        .get(&candidate.reference)
        .ok_or_else(|| invalid("reacquired candidate is no longer in the current snapshot"))?;
    let index = current["index"]
        .as_u64()
        .ok_or_else(|| invalid("reacquired candidate omitted its index"))?;
    let legacy = format!("{}:{index}", state.snapshot_id);
    state
        .short_to_legacy
        .insert(reference.to_owned(), legacy.clone());
    Ok(legacy)
}

async fn secure_password_fill(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
) -> Result<Value, rmcp::ErrorData> {
    let expected_origin = args
        .expected_origin
        .as_deref()
        .and_then(normalized_secure_origin);
    let account_marker = args.account_marker.as_ref();
    if expected_origin.is_none() || account_marker.is_none_or(|marker| !valid_secure_account_marker(marker)) {
        return Ok(json!({"status":"not_dispatched","action":"secure_fill","reason_code":"secure_destination_binding_required","dispatch_may_have_occurred":false,"verified":false,"reason":"An exact secure origin and visible account marker are required before password entry."}));
    }
    if args
        .value
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.chars().count() > 4096)
    {
        return Ok(
            json!({"status":"not_dispatched","action":"secure_fill","reason_code":"password_length_out_of_bounds",
            "dispatch_may_have_occurred":false,"verified":false,
            "reason":"secure_fill password must contain 1 to 4096 characters."}),
        );
    }
    if args.expected_value.is_some() {
        return Ok(
            json!({"status":"not_dispatched","action":"secure_fill","reason_code":"secret_argument_forbidden",
            "dispatch_may_have_occurred":false,"verified":false,
            "reason":"secure_fill accepts no expected_value; use only its one-shot value or the extension prompt."}),
        );
    }
    let shared = app
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = shared.lock().await;
    if shared.handle.principal != app.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let snapshot = shared
        .snapshots
        .get(&args.chrome_tab_id)
        .cloned()
        .ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (frame_id, loader_id, backend_node_id) = if reference.starts_with("@f:") {
        let (frame_id, loader_id, backend_node_id) =
            frame_scope_parts(reference).ok_or_else(|| {
                invalid("secure_fill requires a fresh @f frame reference or @c reference")
            })?;
        (frame_id, loader_id, backend_node_id)
    } else {
        let legacy =
            resolve_reference(app, &args.session_id, &args.chrome_tab_id, reference).await?;
        let (token, index) = legacy
            .rsplit_once(':')
            .ok_or_else(|| invalid("invalid retained reference"))?;
        let index = index
            .parse::<usize>()
            .map_err(|_| invalid("invalid retained reference index"))?;
        if index >= snapshot.count
            || (!crate::guard_reason::unrestricted_mode()
                && (token != snapshot.token || snapshot.consumed))
        {
            return Err(invalid(
                "stale or consumed reference; take a fresh browser_snapshot",
            ));
        }
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        let remote = connection.command(registry, &shared.handle, &args.chrome_tab_id, "Runtime.callFunctionOn", json!({
            "objectId":snapshot.object_id,"functionDeclaration":"function(index){return this.nodes[index]}",
            "arguments":[{"value":index}],"returnByValue":false
        })).await.map_err(|_| invalid("password reference is stale; take a fresh snapshot"))?;
        let object_id = shared_value(&remote)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/result/objectId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .ok_or_else(|| invalid("password reference could not be resolved"))?;
        let described = connection
            .command(
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                "DOM.describeNode",
                json!({"objectId":object_id}),
            )
            .await
            .map_err(|_| invalid("password field backend identity is unavailable"))?;
        let backend = shared_value(&described)
            .ok()
            .and_then(|value| value.pointer("/node/backendNodeId").and_then(Value::as_i64))
            .filter(|id| *id > 0)
            .ok_or_else(|| invalid("password field backend identity is unavailable"))?;
        let identity =
            super::shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
                .await
                .map_err(invalid)?;
        if identity != snapshot.identity {
            return Ok(
                json!({"status":"not_dispatched","action":"secure_fill","reason_code":"stale_document","dispatch_may_have_occurred":false,"verified":false}),
            );
        }
        (identity.0, identity.1, backend)
    };
    let identity = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        super::shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id)
            .await
            .map_err(invalid)?
    };
    if identity.0 != frame_id && reference.starts_with("@f:") {
        // Child-frame identity is independently checked by the extension. The root
        // document must still match the epoch captured by the original snapshot.
        if identity != snapshot.identity {
            return Ok(
                json!({"status":"not_dispatched","action":"secure_fill","reason_code":"stale_document","dispatch_may_have_occurred":false,"verified":false}),
            );
        }
    } else if identity != snapshot.identity {
        return Ok(
            json!({"status":"not_dispatched","action":"secure_fill","reason_code":"stale_document","dispatch_may_have_occurred":false,"verified":false}),
        );
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let registry = shared.registry()?;
    let frame_tree = connection
        .command(
            registry,
            &shared.handle,
            &args.chrome_tab_id,
            "Page.getFrameTree",
            Value::Null,
        )
        .await
        .map_err(|_| invalid("password frame identity is unavailable"))?;
    let Some((current_loader, frame_url)) = super::find_cdp_frame(&frame_tree, &frame_id) else {
        return Ok(json!({"status":"not_dispatched","action":"secure_fill","reason_code":"password_frame_unavailable","dispatch_may_have_occurred":false,"verified":false}));
    };
    if current_loader != loader_id
        || normalized_secure_origin(&frame_url).as_deref() != expected_origin.as_deref()
    {
        return Ok(json!({"status":"not_dispatched","action":"secure_fill","reason_code":"secure_origin_mismatch","dispatch_may_have_occurred":false,"verified":false,"reason":"The live password frame no longer matches the requested secure origin."}));
    }
    let marker = account_marker.expect("validated account marker");
    let marker_response = connection
        .command(
            registry,
            &shared.handle,
            &args.chrome_tab_id,
            "Controlla.checkIdentityMarker",
            json!({"selector":marker.selector,"expected":marker.expected}),
        )
        .await
        .map_err(|_| invalid("account identity could not be checked before password entry"))?;
    let marker_evidence = shared_value(&marker_response).map_err(invalid)?;
    if marker_evidence["matches"] != true {
        return Ok(json!({"status":"not_dispatched","action":"secure_fill","reason_code":"account_identity_mismatch","dispatch_may_have_occurred":false,"verified":false,"reason":"The visible account identity did not match the requested account marker; no password was sent."}));
    }
    let identity_after_marker = super::shared_frame_identity(
        connection,
        registry,
        &shared.handle,
        &args.chrome_tab_id,
    )
    .await
    .map_err(invalid)?;
    let frame_tree_after_marker = connection
        .command(
            registry,
            &shared.handle,
            &args.chrome_tab_id,
            "Page.getFrameTree",
            Value::Null,
        )
        .await
        .map_err(|_| invalid("password frame identity changed before entry"))?;
    if identity_after_marker != identity
        || super::find_cdp_frame(&frame_tree_after_marker, &frame_id)
            != Some((loader_id.clone(), frame_url))
    {
        return Ok(json!({"status":"not_dispatched","action":"secure_fill","reason_code":"secure_destination_changed","dispatch_may_have_occurred":false,"verified":false,"reason":"The target or password frame changed during destination binding; no password was sent."}));
    }
    if !crate::guard_reason::unrestricted_mode() {
        shared
            .snapshots
            .get_mut(&args.chrome_tab_id)
            .ok_or_else(|| invalid("snapshot disappeared before secure dispatch"))?
            .consumed = true;
    }
    let connection = shared
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
    let registry = shared.registry()?;
    let mut params = json!({
        "frame_id":frame_id,"loader_id":loader_id,"backend_node_id":backend_node_id,
        "label":"Password","timeout_ms":args.timeout_ms.unwrap_or(20_000).clamp(1_000,22_000)
    });
    if let Some(password) = args.value.as_deref() {
        params["password"] = json!(password);
    }
    let response = connection
        .command(
            registry,
            &shared.handle,
            &args.chrome_tab_id,
            "Controlla.securePasswordFill",
            params,
        )
        .await;
    match response {
        Ok(response) => Ok(frame_click_receipt(response)),
        Err(_) => Ok(
            json!({"status":"unknown","action":"secure_fill","dispatch_acknowledged":false,
            "dispatch_may_have_occurred":true,"verified":false,"reason_code":"secure_input_delivery_unknown",
            "reason":"Secure input delivery is uncertain; inspect the field before retrying."}),
        ),
    }
}

async fn retained_mutation(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    allow_password: bool,
    action: &str,
) -> Result<Value, rmcp::ErrorData> {
    let first = retained_mutation_once(app, args, reference, allow_password, action).await?;
    if !mutation_needs_reacquisition(&first) {
        return Ok(first);
    }

    let (expected, ancestor) = {
        let semantic = app.semantic.lock().await;
        let Some(state) = semantic.get(&tab_key(&args.session_id, &args.chrome_tab_id)) else {
            return Ok(first);
        };
        let Some(expected) = state.identities.get(reference).cloned() else {
            return Ok(first);
        };
        let ancestor = state.items.get(reference).and_then(|item| {
            Some(SnapshotSemanticAncestor {
                tag: item.pointer("/semantic_ancestor/tag")?.as_str()?.to_owned(),
                id: item.pointer("/semantic_ancestor/id")?.as_str()?.to_owned(),
            })
        });
        (expected, ancestor)
    };
    if expected.name.is_empty() || expected.role.is_empty() {
        let mut result = first;
        result["reacquisition"] =
            json!({"status":"not_rebound","reason":"target identity lacks an exact role or name"});
        return Ok(result);
    }
    let fresh = snapshot_impl(
        app,
        &BrowserSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: None,
            max_items: Some(200),
            max_text_chars: Some(0),
            max_bytes: Some(100_000),
            max_bytes_reason: None,
            href_hint: None,
            frame_id: None,
            include_frames: None,
            goal: Some(SnapshotGoal {
                query: Some(expected.name.clone()),
                roles: Some(vec![expected.role.clone()]),
                href_hint: expected.href.clone(),
                within: ancestor,
                exhaustive: true,
                model_profile: None,
                needs_layout: false,
            }),
            max_scan_nodes: Some(65_536),
        },
    )
    .await;
    let fresh = match fresh {
        Ok(snapshot) if snapshot["scope_complete"] == true => snapshot,
        Ok(snapshot) => {
            let mut result = first;
            result["reacquisition"] = json!({"status":"not_rebound","reason":"targeted semantic scope was incomplete","scope_complete":snapshot["scope_complete"]});
            return Ok(result);
        }
        Err(error) => {
            let mut result = first;
            result["reacquisition"] = json!({"status":"not_rebound","reason":error.to_string()});
            return Ok(result);
        }
    };
    let candidate = {
        let semantic = app.semantic.lock().await;
        let Some(state) = semantic.get(&tab_key(&args.session_id, &args.chrome_tab_id)) else {
            return Ok(first);
        };
        let Some(document) = state.document.as_ref() else {
            return Ok(first);
        };
        let items = state.items.values().cloned().collect::<Vec<_>>();
        crate::target_registry::rebind_result(&expected, Some(document), &items)
    };
    let candidate = match candidate {
        Ok(candidate) => candidate.reference,
        Err(error) => {
            let mut result = first;
            result["reacquisition"] = json!({"status":"not_rebound","reason":error.to_string()});
            return Ok(result);
        }
    };
    let mut rebound = retained_mutation_once(app, args, &candidate, allow_password, action).await?;
    rebound["reacquisition"] = json!({"status":"rebound","from":reference,"to":candidate,"semantic_revision":fresh["semantic_revision"]});
    Ok(rebound)
}

async fn retained_click(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    outcome: BrowserClickOutcome,
) -> Result<Value, rmcp::ErrorData> {
    let first = retained_click_once(app, args, reference, &outcome).await?;
    if !click_needs_reacquisition(&first) {
        return Ok(first);
    }
    let expected = {
        let semantic = app.semantic.lock().await;
        semantic
            .get(&tab_key(&args.session_id, &args.chrome_tab_id))
            .and_then(|state| state.identities.get(reference))
            .cloned()
    };
    let Some(expected) =
        expected.filter(|identity| !identity.name.is_empty() && !identity.role.is_empty())
    else {
        return Ok(first);
    };
    let fresh = snapshot_impl(
        app,
        &BrowserSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: None,
            max_items: Some(200),
            max_text_chars: Some(0),
            max_bytes: Some(100_000),
            max_bytes_reason: None,
            href_hint: None,
            frame_id: None,
            include_frames: None,
            goal: Some(SnapshotGoal {
                query: Some(expected.name.clone()),
                roles: Some(vec![expected.role.clone()]),
                href_hint: expected.href.clone(),
                within: None,
                exhaustive: true,
                model_profile: None,
                needs_layout: false,
            }),
            max_scan_nodes: Some(65_536),
        },
    )
    .await?;
    if fresh["scope_complete"] != true {
        return Ok(first);
    }
    let candidate = {
        let semantic = app.semantic.lock().await;
        let Some(state) = semantic.get(&tab_key(&args.session_id, &args.chrome_tab_id)) else {
            return Ok(first);
        };
        let Some(document) = state.document.as_ref() else {
            return Ok(first);
        };
        crate::target_registry::rebind_result(
            &expected,
            Some(document),
            &state.items.values().cloned().collect::<Vec<_>>(),
        )
    };
    let candidate = match candidate {
        Ok(candidate) => candidate.reference,
        Err(_) => return Ok(first),
    };
    let mut rebound = retained_click_once(app, args, &candidate, &outcome).await?;
    rebound["reacquisition"] = json!({"status":"rebound_after_not_dispatched","from":reference,"to":candidate,"semantic_revision":fresh["semantic_revision"]});
    Ok(rebound)
}

fn click_needs_reacquisition(receipt: &Value) -> bool {
    pre_dispatch_reacquisition_needed(receipt)
}

fn mutation_needs_reacquisition(receipt: &Value) -> bool {
    pre_dispatch_reacquisition_needed(receipt)
}

fn pre_dispatch_reacquisition_needed(receipt: &Value) -> bool {
    receipt["status"] == "not_dispatched"
        && (matches!(
            receipt["reason_code"].as_str(),
            Some("changed_target" | "stale_watcher_revision" | "stale_watcher_document")
        ) || receipt["readback"]["reason"] == "target_replaced")
}

async fn retained_click_once(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    outcome: &BrowserClickOutcome,
) -> Result<Value, rmcp::ErrorData> {
    let legacy_reference =
        resolve_reference(app, &args.session_id, &args.chrome_tab_id, reference).await?;
    let shared_outcome: SharedClickOutcome =
        serde_json::from_value(serde_json::to_value(outcome).map_err(|e| invalid(e.to_string()))?)
            .map_err(|e| invalid(e.to_string()))?;
    let Json(value) = app
        .legacy
        .shared_click(Parameters(SharedClickArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            reference: legacy_reference,
            outcome: shared_outcome,
            timeout_ms: args.timeout_ms,
            input_mode: (args.action == "touch").then(|| "touch".to_owned()),
            click_kind: Some(args.action.as_str().to_owned()).filter(|kind| kind != "touch"),
            expected_target_revision: None,
        }))
        .await?;
    Ok(value)
}

async fn retained_mutation_once(
    app: &AppV2,
    args: &BrowserActArgs,
    reference: &str,
    allow_password: bool,
    action: &str,
) -> Result<Value, rmcp::ErrorData> {
    let legacy_reference =
        resolve_reference(app, &args.session_id, &args.chrome_tab_id, reference).await?;
    let expected = args.expected_value.clone().unwrap_or_default();
    let value = args.value.clone().unwrap_or_default();
    if expected.len() > 16_384 || value.len() > 16_384 {
        return Err(invalid(
            "expected_value/value exceed the 16384 character input bound",
        ));
    }
    let shared = app
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(&args.session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown shared session_id"))?;
    let mut shared = shared.lock().await;
    if shared.handle.principal != app.legacy.principal.as_ref() {
        return Err(invalid("session is not owned by this server principal"));
    }
    let snapshot = shared
        .snapshots
        .get(&args.chrome_tab_id)
        .cloned()
        .ok_or_else(|| invalid("take browser_snapshot first"))?;
    let (token, index) = legacy_reference
        .rsplit_once(':')
        .ok_or_else(|| invalid("invalid retained reference"))?;
    let index = index
        .parse::<usize>()
        .map_err(|_| invalid("invalid retained reference index"))?;
    let unrestricted = crate::guard_reason::unrestricted_mode();
    if index >= snapshot.count
        || (!unrestricted
            && (token != snapshot.token
                || snapshot.consumed
                    && !super::internal_workflow_snapshot_allowed(&snapshot.token)))
    {
        return Err(invalid(
            "stale or consumed reference; take one fresh browser_snapshot",
        ));
    }
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(10_000).clamp(1_000, 30_000));
    let deadline = tokio::time::Instant::now() + timeout;
    let (before, interaction) = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        tokio::time::timeout_at(
            deadline,
            super::shared_identity_and_epoch(
                connection,
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                &snapshot.object_id,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            ),
        )
        .await
        .map_err(|_| invalid("action preflight timed out; no mutation dispatched"))?
        .map_err(invalid)?
    };
    let epoch_use = if action == "slider" {
        crate::v3::EpochUse::PointerMutation
    } else {
        crate::v3::EpochUse::TextMutation
    };
    if !unrestricted
        && !super::shared_epoch_matches(
            &snapshot,
            &args.chrome_tab_id,
            &before,
            &interaction,
            epoch_use,
        )
    {
        // A same-document rerender can detach the referenced node and advance the
        // semantic epoch. Permit only a read-only connectivity probe in that case;
        // retained_mutation may then reacquire the unique semantic identity and
        // retry once. Never use a stale epoch to dispatch a mutation.
        if before == snapshot.identity {
            let detached = {
                let connection = shared
                    .connection
                    .as_ref()
                    .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
                connection.command(
                    shared.registry()?,
                    &shared.handle,
                    &args.chrome_tab_id,
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId": snapshot.object_id,
                        "functionDeclaration": "function(index){const e=this.nodes[index];return {connected:!!e&&e.isConnected}}",
                        "arguments": [{"value": index}],
                        "returnByValue": true
                    }),
                ).await
            };
            if let Ok(response) = detached
                && shared_value(&response).is_ok_and(|value| value["connected"] == false)
            {
                return Ok(json!({
                    "status":"not_dispatched",
                    "action":action,
                    "reason":"target was replaced after the snapshot; no mutation was dispatched",
                    "readback":{"reason":"target_replaced","connected":false}
                }));
            }
        }
        return Err(invalid(
            "document changed since snapshot; take a fresh browser_snapshot",
        ));
    }
    if allow_password && action != "fill" {
        return Err(invalid("allow_password is supported only for action=fill"));
    }
    let function = if action == "fill" && allow_password {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLInputElement)||e.type!=='password'||e.matches(':disabled')||e.readOnly||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};const r=e.getBoundingClientRect(),s=getComputedStyle(e);if(r.width<=0||r.height<=0||r.left<0||r.top<0||r.right>innerWidth||r.bottom>innerHeight||s.visibility==='hidden'||s.display==='none'||s.pointerEvents==='none'||document.elementFromPoint(r.left+r.width/2,r.top+r.height/2)!==e)return {ok:false,reason:'blocked'};const setter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:null}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,credential_filled:e.value===value,dispatched:true};}"#
    } else if action == "fill" {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)||['password','hidden','file','checkbox','radio','button','submit','reset','image'].includes(e.type||'')||e.matches(':disabled')||e.readOnly||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'same_field_conflict'};const setter=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:value}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,value:e.value,dispatched:true};}"#
    } else if action == "slider" {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLInputElement)||e.type!=='range'||e.matches(':disabled')||e.getAttribute('aria-disabled')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'same_field_conflict'};const n=Number(value),min=e.min===''?0:Number(e.min),max=e.max===''?100:Number(e.max),step=e.step==='any'?0:Number(e.step);if(!Number.isFinite(n)||n<min||n>max||(step>0&&Math.abs((n-min)/step-Math.round((n-min)/step))>1e-6))return {ok:false,reason:'invalid_range_value'};const setter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;if(!setter)return {ok:false,reason:'missing_value_setter'};setter.call(e,String(n));e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertReplacementText',data:String(n)}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===String(n),value:e.value,aria_value:e.getAttribute('aria-valuenow'),dispatched:true};}"#
    } else {
        r#"function(index,expected,value){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};if(!(e instanceof HTMLSelectElement)||e.matches(':disabled')||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'blocked'};if(e.value!==expected)return {ok:false,reason:'same_field_conflict'};if(![...e.options].some(o=>o.value===value))return {ok:false,reason:'option_missing'};e.value=value;e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,value:e.value};}"#
    };
    // Consume before dispatch so a lost response cannot permit duplicate mutation via the same reference.
    if !unrestricted {
        shared
            .snapshots
            .get_mut(&args.chrome_tab_id)
            .ok_or_else(|| invalid("snapshot disappeared before dispatch"))?
            .consumed = true;
    }
    let verification_started = std::time::Instant::now();
    let response = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        tokio::time::timeout_at(
            deadline,
            connection.command(
                registry,
                &shared.handle,
                &args.chrome_tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId": snapshot.object_id,
                    "functionDeclaration": function,
                    "arguments":[{"value":index},{"value":expected},{"value":value}],
                    "returnByValue":true
                }),
            ),
        )
        .await
    };
    let mut result = match response {
        Ok(Ok(response)) => match shared_value(&response) {
            Ok(value) if value["ok"] == true => {
                json!({"status":"verified","dispatch_acknowledged":true,"action":action,"readback":value})
            }
            Ok(value) if value["dispatched"] == true => {
                json!({"status":"unknown","dispatch_acknowledged":true,"dispatch_may_have_occurred":true,"action":action,"reason":"fill readback changed after input/change dispatch; inspect state before retrying","readback":value})
            }
            Ok(value) => {
                if same_field_value_conflict(&value) {
                    same_field_conflict_receipt(action)
                } else {
                    json!({"status":"not_dispatched","dispatch_acknowledged":true,"action":action,"reason":value["reason"],"readback":value})
                }
            }
            Err(error) => json!({
                "status":"unknown","dispatch_acknowledged":true,
                "reason":format!("browser mutation reply could not be interpreted after dispatch: {error}; inspect state and do not automatically retry")
            }),
        },
        Ok(Err(error)) => json!({
            "status":"unknown","dispatch_acknowledged":false,
            "reason":format!("browser mutation delivery is uncertain: {error}; inspect state and do not automatically retry")
        }),
        Err(_) => json!({
            "status":"unknown","dispatch_acknowledged":false,
            "reason":"browser mutation deadline exceeded; effect may have occurred; inspect state and do not automatically retry"
        }),
    };
    let after = {
        let connection = shared
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
        let registry = shared.registry()?;
        tokio::time::timeout_at(
            deadline,
            shared_frame_identity(connection, registry, &shared.handle, &args.chrome_tab_id),
        )
        .await
    };
    if !unrestricted
        && !matches!(after, Ok(Ok(ref identity)) if identity == &before)
        && result["status"] == "verified"
    {
        result = json!({
            "status":"unknown","dispatch_acknowledged":true,
            "reason":"document changed after mutation; local readback cannot establish the final page state; inspect before retrying"
        });
    }
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Verification,
        verification_started
            .elapsed()
            .as_micros()
            .min(u64::MAX as u128) as u64,
    )
    .await;
    drop(shared);
    if allow_password && result["status"] == "verified" {
        result["readback"] = json!({"credential_filled":true,"ok":true});
    }
    if result["status"] == "verified" {
        super::authorize_internal_workflow_snapshot(&snapshot.token);
        result["delta"] = json!({
            "reference": reference,
            "before": if allow_password { Value::Null } else { json!(expected) },
            "after": if allow_password { Value::Null } else { result["readback"].get("value").cloned().unwrap_or(json!(value)) },
            "credential_filled": allow_password,
            "document_changed": false,
            "verified": true
        });
    }
    Ok(result)
}

fn snapshot_root_scope_complete(snapshot: &Value) -> bool {
    snapshot["scope_complete"] == true
        || matches!(
            snapshot["truncation_reason"].as_str(),
            Some("cross_origin_frames_unread" | "same_origin_frames_pending")
        )
}

fn find_scope_is_complete(
    global_complete: bool,
    root_complete: bool,
    within_root: bool,
    exact_root_matches_only: bool,
) -> bool {
    global_complete || root_complete && (within_root || exact_root_matches_only)
}

fn exact_root_scope_only(
    global_complete: bool,
    mode: &str,
    within_root: bool,
    ranked: &[Value],
) -> bool {
    !global_complete
        && !within_root
        && mode == "exact"
        && ranked.iter().any(|item| item["source_frame"] == "main")
        && ranked.iter().all(|item| item["source_frame"] == "main")
}

fn workflow_find_is_usable(found: &Value) -> bool {
    found["scope_complete"] == true
        && found["ambiguous"] != true
        && found["matches"]
            .as_array()
            .is_some_and(|matches| matches.len() == 1)
}

async fn find_impl(app: &AppV2, args: &BrowserFindArgs) -> Result<Value, rmcp::ErrorData> {
    if args.query.trim().is_empty() || args.query.len() > 512 {
        return Err(invalid("query must contain 1..512 characters"));
    }
    let key = tab_key(&args.session_id, &args.chrome_tab_id);
    if !app.semantic.lock().await.contains_key(&key) {
        let _ = snapshot_impl(
            app,
            &BrowserSnapshotArgs {
                session_id: args.session_id.clone(),
                chrome_tab_id: args.chrome_tab_id.clone(),
                selector: None,
                max_items: None,
                max_text_chars: None,
                max_bytes: None,
            max_bytes_reason: None,
                href_hint: None,
                frame_id: None,
                include_frames: None,
                goal: None,
                max_scan_nodes: None,
            },
        )
        .await?;
    }
    let mode = args.match_mode.as_deref().unwrap_or("all");
    if !matches!(mode, "all" | "best" | "exact")
        || args
            .href_hint
            .as_ref()
            .is_some_and(|hint| hint.is_empty() || hint.len() > 512)
    {
        return Err(invalid(
            "match_mode must be all, best, or exact and href_hint must be bounded",
        ));
    }
    let state = app.semantic.lock().await;
    let state = state
        .get(&key)
        .ok_or_else(|| invalid("semantic snapshot state unavailable"))?;
    let (ancestor_id, within_root) = if let Some(reference) = args.within_reference.as_deref() {
        let item = state
            .items
            .get(reference)
            .ok_or_else(|| invalid("within_reference must identify a stable semantic ancestor"))?;
        let ancestor = item
            .pointer("/semantic_ancestor/id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("within_reference must identify a stable semantic ancestor"))?;
        (Some(ancestor), item["source_frame"] == "main")
    } else {
        (None, false)
    };
    let mut ranked = crate::semantic_state::rank_candidates(
        state.items.values().cloned(),
        &args.query,
        args.role.as_deref(),
        args.href_hint.as_deref(),
        ancestor_id,
        mode == "exact",
    );
    let exact_root_matches_only =
        exact_root_scope_only(state.scope_complete, mode, within_root, &ranked);
    if exact_root_matches_only {
        ranked.retain(|item| item["source_frame"] == "main");
    }
    let root_scoped = within_root || exact_root_matches_only;
    let scope_complete = find_scope_is_complete(
        state.scope_complete,
        state.root_scope_complete,
        within_root,
        exact_root_matches_only,
    );
    let ambiguous = find_is_ambiguous(mode, scope_complete, &ranked);
    let limit = args.max_matches.or(args.limit).unwrap_or(10).clamp(1, 50);
    let matches = ranked
        .into_iter()
        .take(if mode == "best" && !ambiguous {
            1
        } else {
            limit
        })
        .collect::<Vec<_>>();
    let count = matches.len();
    Ok(
        json!({"matches":matches,"count":count,"ambiguous":ambiguous,"scope_complete":scope_complete,"global_scope_complete":state.scope_complete,"scope":if root_scoped {"root_document_subtree"} else {"all_collected_frames"},"semantic_revision":state.snapshot_id}),
    )
}

fn find_is_ambiguous(mode: &str, scope_complete: bool, ranked: &[Value]) -> bool {
    match mode {
        "best" => {
            !scope_complete
                || ranked
                    .first()
                    .is_some_and(|item| item["confidence"] != "high")
        }
        "exact" => !scope_complete || ranked.len() > 1,
        _ => false,
    }
}

async fn verify_shared_dom_text(
    app: &AppV2,
    session_id: &str,
    tab_id: &str,
    scope: Option<&str>,
    expected: &str,
) -> Result<Value, rmcp::ErrorData> {
    let selector = match parse_text_predicate_scope(scope).map_err(invalid)? {
        TextPredicateScope::Root => None,
        TextPredicateScope::ScopedSelector(selector) => Some(selector),
        _ => {
            return Err(invalid(
                "DOM text verification requires root or CSS-selector scope",
            ));
        }
    };
    let live = app
        .legacy
        .shared_sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| invalid("unknown session for text verification"))?;
    let live = live.lock().await;
    if live.handle.principal != app.legacy.principal.as_ref() {
        return Err(invalid(
            "text verification session is not owned by this server principal",
        ));
    }
    let connection = live
        .connection
        .as_ref()
        .ok_or_else(|| invalid("shared browser connection is unavailable"))?;
    let registry = live.registry()?;
    let before = super::shared_frame_identity(connection, registry, &live.handle, tab_id)
        .await
        .map_err(invalid)?;
    let selector_json =
        serde_json::to_string(&selector).map_err(|error| invalid(error.to_string()))?;
    let expected_json =
        serde_json::to_string(expected).map_err(|error| invalid(error.to_string()))?;
    let expression = format!(
        "(()=>{{let e=document.body||document.documentElement;if({selector_json}!==null){{let found;try{{found=[...document.querySelectorAll({selector_json})]}}catch(_){{return {{ok:false,reason:'invalid_selector'}}}};if(found.length!==1)return {{ok:false,reason:'selector_not_unique'}};e=found[0]}}if(!e)return {{ok:false,reason:'scope_missing'}};const s=String(e.innerText??e.textContent??'');return {{ok:true,text_present:s.slice(0,4097).includes({expected_json}),truncated:s.length>4096}}}})()",
        selector_json = selector_json,
        expected_json = expected_json
    );
    let response = connection
        .command(
            registry,
            &live.handle,
            tab_id,
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true}),
        )
        .await
        .map_err(|error| invalid(error.to_string()))?;
    let value = super::shared_value(&response).map_err(invalid)?;
    if value["ok"] != true {
        return Ok(
            json!({"status":"incomplete","observer":"shared_root_dom_text","expected":{"text_contains":expected},"observed":{"scope":if selector.is_some() {"selector"} else {"root"},"reason":value["reason"]},"evidence_complete":false,"dispatch_occurred":false}),
        );
    }
    let after = super::shared_frame_identity(connection, registry, &live.handle, tab_id)
        .await
        .map_err(invalid)?;
    if before != after {
        return Ok(
            json!({"status":"incomplete","observer":"shared_root_dom_text","expected":{"text_contains":expected},"observed":null,"evidence_complete":false,"reason":"document changed during text readback","dispatch_occurred":false}),
        );
    }
    let found = value["text_present"] == true;
    let complete = value["truncated"] != true;
    Ok(
        json!({"status":text_presence_status(found,!complete),"observer":"shared_root_dom_text","expected":{"text_contains":expected},"observed":{"text_present":found,"scope":if selector.is_some() {"selector"} else {"root"}},"evidence_complete":complete,"document":{"frame_id":before.0,"loader_id":before.1,"url":before.2},"dispatch_occurred":false}),
    )
}

async fn verify_impl(app: &AppV2, args: &BrowserVerifyArgs) -> Result<Value, rmcp::ErrorData> {
    let started = std::time::Instant::now();
    let result = verify_impl_unmeasured(app, args).await;
    controlla_browser::scheduler::telemetry::record_phase(
        controlla_browser::scheduler::telemetry::OperationPhase::Verification,
        started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    )
    .await;
    result
}

async fn verify_impl_unmeasured(
    app: &AppV2,
    args: &BrowserVerifyArgs,
) -> Result<Value, rmcp::ErrorData> {
    if matches!(
        &args.predicate,
        BrowserPredicate::Url { .. } | BrowserPredicate::UrlStartsWith { .. }
    ) {
        let live = app
            .legacy
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown session for URL verification"))?;
        let live = live.lock().await;
        if live.handle.principal != app.legacy.principal.as_ref() {
            return Err(invalid(
                "URL verification session is not owned by this server principal",
            ));
        }
        let connection = live
            .connection
            .as_ref()
            .ok_or_else(|| invalid("shared browser connection is unavailable"))?;
        let registry = live.registry()?;
        let identity =
            super::shared_frame_identity(connection, registry, &live.handle, &args.chrome_tab_id)
                .await
                .map_err(invalid)?;
        let actual = identity.2;
        let passed = match &args.predicate {
            BrowserPredicate::Url { equals } => urls_identify_same_resource(equals, &actual),
            BrowserPredicate::UrlStartsWith { prefix } => {
                !prefix.is_empty() && actual.starts_with(prefix)
            }
            _ => unreachable!(),
        };
        return Ok(json!({
            "status": if passed { "passed" } else { "failed" },
            "observer": "browser_frame_identity",
            "observed": actual,
            "document": {"frame_id":identity.0,"loader_id":identity.1},
            "scope_complete": true
        }));
    }
    if let BrowserPredicate::TextPresent {
        scope: Some(scope),
        text,
    } = &args.predicate
        && scope.starts_with("@f:")
    {
        if text.is_empty() || text.len() > 4096 {
            return Err(invalid("verification text must contain 1..4096 bytes"));
        }
        let (frame_id, loader_id, _) = frame_scope_parts(scope).ok_or_else(|| {
            invalid("frame-scoped text verification requires a fresh @f frame reference")
        })?;
        let frame = snapshot_impl(
            app,
            &BrowserSnapshotArgs {
                session_id: args.session_id.clone(),
                chrome_tab_id: args.chrome_tab_id.clone(),
                selector: None,
                max_items: Some(200),
                max_text_chars: Some(0),
                max_bytes: Some(1_000_000),
            max_bytes_reason: None,
                href_hint: None,
                frame_id: Some(frame_id.clone()),
                include_frames: None,
                goal: None,
                max_scan_nodes: None,
            },
        )
        .await?;
        let (found, complete) = frame_scoped_text_result(scope, text, &frame)
            .ok_or_else(|| invalid("frame-scoped text verification reference is malformed"))?;
        return Ok(json!({
            "status": if found { "passed" } else if complete { "failed" } else { "incomplete" },
            "observer": "fresh_shared_child_frame_accessibility",
            "observed": {"text_present":found,"frame_id":frame_id,"frame_loader_id":loader_id},
            "scope_complete":complete,
            "document":frame["shared_frame_identity"]
        }));
    }
    if let BrowserPredicate::TextPresent { scope, text } = &args.predicate {
        if text.is_empty() || text.len() > 4096 {
            return Err(invalid("verification text must contain 1..4096 bytes"));
        }
        match parse_text_predicate_scope(scope.as_deref()).map_err(invalid)? {
            TextPredicateScope::Root | TextPredicateScope::ScopedSelector(_) => {
                return verify_shared_dom_text(
                    app,
                    &args.session_id,
                    &args.chrome_tab_id,
                    scope.as_deref(),
                    text,
                )
                .await;
            }
            TextPredicateScope::Reference(reference) => {
                let state = app.semantic.lock().await;
                let Some(state) = state.get(&tab_key(&args.session_id, &args.chrome_tab_id)) else {
                    return Ok(
                        json!({"status":"incomplete","observer":"semantic_reference","expected":{"text_contains":text},"observed":null,"evidence_complete":false,"reason":"semantic evidence is unavailable"}),
                    );
                };
                let Some(item) = state
                    .items
                    .get(&reference)
                    .filter(|item| semantic_text_is_authoritative(item))
                else {
                    return Ok(
                        json!({"status":"incomplete","observer":"semantic_reference","expected":{"text_contains":text},"observed":null,"evidence_complete":false,"reason":"reference is missing or has no authoritative text"}),
                    );
                };
                let actual = item
                    .get("text")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let passed = actual.contains(text);
                return Ok(
                    json!({"status":if passed {"passed"} else {"failed"},"observer":"fresh_semantic_reference","expected":{"text_contains":text},"observed":{"text_present":passed,"reference":reference},"evidence_complete":true,"document":item.get("document")}),
                );
            }
            TextPredicateScope::FrameReference(_) => {
                unreachable!("frame references returned above")
            }
        }
    }
    let snapshot = snapshot_impl(
        app,
        &BrowserSnapshotArgs {
            session_id: args.session_id.clone(),
            chrome_tab_id: args.chrome_tab_id.clone(),
            selector: None,
            max_items: None,
            max_text_chars: Some(6000),
            max_bytes: Some(60_000),
            max_bytes_reason: None,
            href_hint: None,
            frame_id: None,
            include_frames: None,
            goal: None,
            max_scan_nodes: None,
        },
    )
    .await?;
    if let BrowserPredicate::DownloadArtifact {
        artifact_handle,
        sha256,
        min_bytes,
    } = &args.predicate
    {
        if artifact_handle.is_empty() || artifact_handle.len() > 256 || !valid_sha256(sha256) {
            return Err(invalid(
                "download verification requires a bounded artifact handle and lowercase SHA-256",
            ));
        }
        let live = app
            .legacy
            .shared_sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
            .ok_or_else(|| invalid("unknown shared session for download verification"))?;
        let live = live.lock().await;
        if live.handle.principal != app.legacy.principal.as_ref() {
            return Err(invalid(
                "artifact session is not owned by this server principal",
            ));
        }
        let handle: controlla_browser::sessions::ArtifactHandle =
            serde_json::from_value(json!(artifact_handle))
                .map_err(|_| invalid("invalid opaque artifact_handle"))?;
        let registry = live.registry()?;
        let metadata = registry
            .artifact_metadata(&live.handle, &handle)
            .map_err(|_| invalid("download artifact is unavailable in this session"))?;
        let path = registry
            .artifact_path_for(&live.handle.id, &live.handle.principal, &handle)
            .map_err(|_| invalid("download artifact path is unavailable"))?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| invalid("download artifact could not be read"))?;
        let digest = crate::artifacts::ArtifactExpectation::sha256(&bytes);
        let passed = bytes.len() as u64 >= *min_bytes
            && digest == *sha256
            && metadata.size as u64 == bytes.len() as u64;
        return Ok(
            json!({"status":if passed {"passed"} else {"failed"},"observer":"runtime_owned_artifact_readback",
            "observed":{"sha256":digest,"bytes":bytes.len()},"document":snapshot["document"]}),
        );
    }
    let state = app.semantic.lock().await;
    let state = state
        .get(&tab_key(&args.session_id, &args.chrome_tab_id))
        .ok_or_else(|| invalid("semantic snapshot state unavailable"))?;
    let (passed, observed, complete) = match &args.predicate {
        BrowserPredicate::Exists { reference } => (
            state.items.contains_key(reference),
            json!(state.items.get(reference)),
            true,
        ),
        BrowserPredicate::Text { reference, equals } => {
            let item = state
                .items
                .get(reference)
                .filter(|item| semantic_text_is_authoritative(item));
            let actual = item.and_then(|v| v.get("name")).and_then(Value::as_str);
            (
                semantic_string_field_equals(item, "name", equals),
                json!(actual),
                true,
            )
        }
        BrowserPredicate::TextPresent { scope, text } => {
            if text.is_empty() || text.len() > 4096 {
                return Err(invalid("verification text must contain 1..4096 bytes"));
            }
            let matched = if let Some(reference) = scope {
                state.items.get(reference).is_some_and(|item| {
                    semantic_text_is_authoritative(item)
                        && item
                            .get("text")
                            .or_else(|| item.get("name"))
                            .and_then(Value::as_str)
                            .is_some_and(|value| value.contains(text))
                })
            } else {
                state.items.values().any(|item| {
                    semantic_text_is_authoritative(item)
                        && item
                            .get("text")
                            .or_else(|| item.get("name"))
                            .and_then(Value::as_str)
                            .is_some_and(|value| value.contains(text))
                })
            };
            let complete = scope.is_some() || snapshot["scope_complete"] == true;
            (
                matched,
                json!({"text_present":matched,"scope":scope}),
                complete,
            )
        }
        BrowserPredicate::Attribute {
            reference,
            name,
            equals,
        } => {
            if !matches!(
                name.as_str(),
                "href" | "datetime" | "role" | "selected" | "expanded"
            ) {
                return Err(invalid(
                    "attribute verification is limited to href, datetime, role, selected, or expanded",
                ));
            }
            let item = state.items.get(reference);
            let actual = item
                .and_then(|item| item.get(name))
                .cloned()
                .unwrap_or(Value::Null);
            (
                semantic_string_field_equals(item, name, equals),
                actual,
                true,
            )
        }
        BrowserPredicate::CountDelta {
            role,
            before,
            delta,
        } => {
            if role.is_empty() || role.len() > 64 {
                return Err(invalid("count_delta role must be 1..64 bytes"));
            }
            let (matched, actual) = semantic_count_delta(&state.items, role, *before, *delta);
            (
                matched,
                json!({"role":role,"count":actual,"before":before,"delta":delta}),
                snapshot["scope_complete"] == true,
            )
        }
        BrowserPredicate::Value { reference, equals } => {
            let item = state.items.get(reference);
            let actual = item
                .and_then(|v| v.get("raw_value"))
                .and_then(Value::as_str);
            (
                semantic_string_field_equals(item, "raw_value", equals),
                json!(actual),
                true,
            )
        }
        BrowserPredicate::Url { equals } => {
            let actual = snapshot
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (
                urls_identify_same_resource(equals, actual),
                json!(actual),
                true,
            )
        }
        BrowserPredicate::UrlStartsWith { prefix } => {
            if prefix.is_empty() {
                return Err(invalid("URL prefix cannot be empty"));
            }
            let actual = snapshot
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (actual.starts_with(prefix), json!(actual), true)
        }
        BrowserPredicate::Attachment { filename, size } => {
            if filename.is_empty() || filename.len() > 512 {
                return Err(invalid("attachment filename must contain 1..512 bytes"));
            }
            let found = state.items.values().find(|item| {
                semantic_text_is_authoritative(item)
                    && matches!(
                        item.get("role").and_then(Value::as_str),
                        Some("attachment" | "listitem" | "option")
                    )
                    && (item.get("filename").and_then(Value::as_str) == Some(filename)
                        || item.get("name").and_then(Value::as_str) == Some(filename))
                    && size.is_none_or(|expected| {
                        item.get("size").and_then(Value::as_u64) == Some(expected)
                    })
            });
            (
                found.is_some(),
                json!({"filename":filename,"size":found.and_then(|item|item.get("size")).cloned()}),
                snapshot["scope_complete"] == true,
            )
        }
        BrowserPredicate::DownloadArtifact { .. } => {
            unreachable!("handled before the semantic snapshot borrow")
        }
        BrowserPredicate::VisualRegionChanged { .. } => {
            return Ok(
                json!({"status":"inconclusive","observer":"fresh_shared_page_snapshot","reason":"visual_region_changed requires two same-document runtime probe results supplied to workflow verify","document":snapshot["document"]}),
            );
        }
        BrowserPredicate::AppSpecific { adapter, predicate } => {
            if adapter == "canva_slide_discovery_v1" {
                let (passed, observed) = crate::apps::verify_canva_slide_discovery_v1(
                    predicate,
                    &state.items,
                    state.scope_complete,
                    snapshot["url"].as_str().unwrap_or_default(),
                )
                .map_err(invalid)?;
                (passed, observed, state.scope_complete)
            } else if adapter == "canva_slide_manifest_v1" {
                let (passed, observed) = crate::apps::verify_canva_slide_manifest_v1(
                    predicate,
                    &state.items,
                    state.scope_complete,
                    snapshot["url"].as_str().unwrap_or_default(),
                )
                .map_err(invalid)?;
                (passed, observed, state.scope_complete)
            } else if adapter == "google_classroom_attachment_v1" {
                let (passed, observed) = crate::apps::verify_classroom_attachment_v1(
                    predicate,
                    &state.items,
                    state.scope_complete,
                )
                .map_err(invalid)?;
                (passed, observed, state.scope_complete)
            } else if adapter == "google_drive_picker_v1" {
                let (passed, observed) = crate::apps::verify_google_drive_picker_v1(
                    predicate,
                    &state.items,
                    state.scope_complete,
                    snapshot["url"].as_str().unwrap_or_default(),
                )
                .map_err(invalid)?;
                (passed, observed, state.scope_complete)
            } else if adapter == "google_drive_attachment_v1" {
                let (passed, observed) = crate::apps::verify_google_drive_attachment_v1(
                    predicate,
                    &state.items,
                    state.scope_complete,
                    snapshot["url"].as_str().unwrap_or_default(),
                )
                .map_err(invalid)?;
                (passed, observed, state.scope_complete)
            } else if adapter == "canva_active_slide_v1" {
                let (passed, observed) = crate::apps::verify_canva_active_slide_v1(
                    predicate,
                    &state.items,
                    state.scope_complete,
                    snapshot["url"].as_str().unwrap_or_default(),
                )
                .map_err(invalid)?;
                (passed, observed, state.scope_complete)
            } else {
                if adapter != "semantic_field_equals" {
                    return Err(invalid("app_specific adapter is not registered"));
                }
                let reference = predicate["reference"]
                    .as_str()
                    .ok_or_else(|| invalid("semantic_field_equals requires reference"))?;
                let field = predicate["field"]
                    .as_str()
                    .ok_or_else(|| invalid("semantic_field_equals requires field"))?;
                let expected = predicate
                    .get("equals")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("semantic_field_equals requires string equals"))?;
                if !matches!(
                    field,
                    "name" | "role" | "href" | "datetime" | "selected" | "expanded" | "raw_value"
                ) || expected.len() > 4096
                {
                    return Err(invalid(
                        "semantic_field_equals field or value is outside its allowlist",
                    ));
                }
                let item = state
                    .items
                    .get(reference)
                    .filter(|item| semantic_text_is_authoritative(item));
                let actual = item
                    .and_then(|item| item.get(field))
                    .and_then(Value::as_str);
                (
                    actual == Some(expected),
                    json!({"field":field,"value":actual}),
                    true,
                )
            }
        }
    };
    Ok(json!({
        "status": if passed { "passed" } else if complete { "failed" } else { "incomplete" },
        "observer":"fresh_shared_page_snapshot",
        "observed":observed,
        "semantic_revision":snapshot["semantic_revision"],
        "document":snapshot["document"]
    }))
}

fn frame_scope_parts(reference: &str) -> Option<(String, String, i64)> {
    let parts = reference
        .strip_prefix("@f:")?
        .splitn(3, ':')
        .collect::<Vec<_>>();
    if parts.len() != 3 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    let backend_node_id = parts[2].parse::<i64>().ok().filter(|id| *id > 0)?;
    Some((parts[0].to_owned(), parts[1].to_owned(), backend_node_id))
}

fn frame_scoped_text_result(scope: &str, text: &str, snapshot: &Value) -> Option<(bool, bool)> {
    let (frame_id, loader_id, _) = frame_scope_parts(scope)?;
    let frame_matches =
        snapshot["source_frame_id"] == frame_id && snapshot["frame_loader_id"] == loader_id;
    let found = frame_matches
        && snapshot["items"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item.get("name")
                    .or_else(|| item.get("text"))
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.contains(text))
            })
        });
    // The frame snapshot is an accessibility tree, not a complete rendered
    // text inventory. A positive match is useful evidence, but absence from AX
    // cannot prove absence from visible text (for example, unlabelled text or
    // content omitted by the browser's AX projection). Keep negative results
    // incomplete until a complete DOM-text observer is available.
    Some((found, false))
}

fn substitute_reference(
    reference: &str,
    vars: &BTreeMap<String, String>,
) -> Result<String, rmcp::ErrorData> {
    if let Some(name) = reference.strip_prefix('$') {
        vars.get(name)
            .cloned()
            .ok_or_else(|| invalid(format!("workflow variable ${name} is unresolved")))
    } else if let Some(value) = vars.get(reference) {
        Ok(value.clone())
    } else {
        Ok(reference.to_owned())
    }
}

fn is_workflow_reconciliation_step(step: &BrowserWorkflowStep) -> bool {
    matches!(step, BrowserWorkflowStep::Verify { .. })
}

#[derive(Debug, PartialEq, Eq)]
enum WorkflowReceiptProgress {
    Continue,
    ReconcileUnknown(usize),
    StopUnknown,
    StopFailed,
}

fn record_workflow_receipt(
    receipts: &mut Vec<Value>,
    receipt: Value,
    reconciliation_index: Option<usize>,
    next_step: Option<&BrowserWorkflowStep>,
) -> WorkflowReceiptProgress {
    let status = receipt
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(action_index) = reconciliation_index {
        let passed = status.as_deref() == Some("passed");
        receipts[action_index]["reconciliation"] = json!({
            "status": if passed { "postcondition_observed" } else { "not_confirmed" },
            "verify_step": receipts.len(),
            "causality_established": false,
            "mutation_retried": false,
        });
        if !passed {
            receipts.push(receipt);
            return WorkflowReceiptProgress::StopUnknown;
        }
    }

    let receipt_index = receipts.len();
    receipts.push(receipt);
    match status.as_deref() {
        Some("unknown" | "dispatched_pending_verification")
            if next_step.is_some_and(is_workflow_reconciliation_step) =>
        {
            WorkflowReceiptProgress::ReconcileUnknown(receipt_index)
        }
        Some("unknown" | "dispatched_pending_verification") => WorkflowReceiptProgress::StopUnknown,
        Some("failed" | "not_dispatched") => WorkflowReceiptProgress::StopFailed,
        _ => WorkflowReceiptProgress::Continue,
    }
}

fn frame_click_receipt(response: Value) -> Value {
    response
        .pointer("/result/value")
        .or_else(|| response.get("result"))
        .cloned()
        .unwrap_or(response)
}

/// Add stable recovery instructions to unknown mutation receipts at every MCP version
/// boundary. Traverse only known receipt containers so page-provided data stays opaque.
fn normalize_unknown_receipts(value: &mut Value) {
    fn normalize(value: &mut Value, receipt: bool) {
        match value {
            Value::Object(object) => {
                if receipt && object.get("status").and_then(Value::as_str) == Some("unknown") {
                    if !object.get("reason_code").is_some_and(Value::is_string) {
                        let acknowledged =
                            object.get("dispatch_acknowledged") == Some(&Value::Bool(true));
                        object.insert(
                            "reason_code".into(),
                            json!(if acknowledged {
                                "outcome_verification_timeout"
                            } else {
                                "action_delivery_unknown"
                            }),
                        );
                    }
                    object
                        .entry("mutation_retry_policy")
                        .or_insert_with(|| json!("reconcile_read_only_first"));
                    object
                        .entry("next_action")
                        .or_insert_with(|| json!("inspect_current_state_once_before_retrying"));
                }
                if receipt
                    && object.get("status").and_then(Value::as_str) == Some("not_dispatched")
                {
                    let next_action = match object.get("reason_code").and_then(Value::as_str) {
                        Some("stale_watcher_revision" | "stale_watcher_document") => "refresh_and_reacquire_once",
                        Some("stale_reference" | "stale_document" | "changed_target") => {
                            "take_fresh_snapshot_and_retry"
                        }
                        Some("activation_forbidden") => "use_a_controlla_owned_automation_window_or_navigate_by_observed_href",
                        Some("challenge_handoff_required") => "let_the_user_complete_the_challenge_then_observe_again",
                        _ => "correct_the_named_cause_before_retrying",
                    };
                    object
                        .entry("next_action")
                        .or_insert_with(|| json!(next_action));
                }
                for key in ["receipts", "steps"] {
                    if let Some(child) = object.get_mut(key) {
                        normalize(child, true);
                    }
                }
                if let Some(Value::Object(nodes)) = object.get_mut("nodes") {
                    for node in nodes.values_mut() {
                        normalize(node, true);
                    }
                }
            }
            Value::Array(items) if receipt => {
                for item in items {
                    normalize(item, true);
                }
            }
            Value::Array(_)
            | Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_) => {}
        }
    }
    normalize(value, true);
}

#[tool_router]
impl AppV2 {
    #[tool(
        name = "browser_session",
        description = "Discover, pair, connect, list, or release explicitly selected Chrome targets. This is a compact alias over Controlla's existing authority-preserving session broker."
    )]
    async fn browser_session(
        &self,
        Parameters(args): Parameters<BrowserSessionArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        self.legacy
            .session(Parameters(SessionArgs {
                action: args.action,
                provider: args.provider,
                session_id: args.session_id,
                target_ids: args.target_ids,
                host_id: args.host_id,
            }))
            .await
    }

    #[tool(
        name = "browser_snapshot",
        description = "Return a compact semantic snapshot for an explicitly paired Chrome tab. Controls report present/rendered/in_viewport separately; in_viewport is only a candidate, and mutations still require actionability checks. Completeness reports text, controls, frames, and limits. Set frame_id to request one bounded child-frame accessibility snapshot; optionally set include_frames=true on a root snapshot to read bounded AX trees from inventoried child frames and report unread/truncated coverage. Returned frame references carry source frame and loader identity."
    )]
    async fn browser_snapshot(
        &self,
        Parameters(args): Parameters<BrowserSnapshotArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(snapshot_impl(self, &args).await?))
    }

    #[tool(
        name = "browser_find",
        description = "Find controls in the current compact semantic snapshot by name, role, value, or observed href. Returns exact @cN refs; if no snapshot exists, one is captured first."
    )]
    async fn browser_find(
        &self,
        Parameters(args): Parameters<BrowserFindArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(find_impl(self, &args).await?))
    }

    #[tool(
        name = "browser_act",
        description = "Act on an exact semantic reference. click uses guarded mouse input; touch uses guarded touchStart/touchEnd with no mouse fallback. For a visible empty password field, secure_fill accepts an optional one-shot value or opens Controlla's local prompt when omitted; only the verified boolean is returned. Never use password values in other actions or workflows. fill/select use retained DOM identity, consume references before dispatch, verify readback, and return unknown rather than retry when delivery is uncertain."
    )]
    async fn browser_act(
        &self,
        Parameters(args): Parameters<BrowserActArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let reference = args
            .reference
            .as_deref()
            .ok_or_else(|| invalid("reference is required"))?;
        let mut result = match args.action.as_str() {
            "click" | "double_click" | "context_click" | "touch" => {
                if let Some(
                    BrowserClickOutcome::Navigation { url: Some(url) }
                    | BrowserClickOutcome::NewTab { url: Some(url) },
                ) = args.outcome.as_ref()
                {
                    super::validate_shared_url(url)?;
                }
                if let Some(frame_ref) = reference.strip_prefix("@f:") {
                    if matches!(args.action.as_str(), "double_click" | "context_click") {
                        return Ok(Json(json!({"status":"not_dispatched","action":args.action,"reason_code":"frame_pointer_variant_unsupported","dispatch_may_have_occurred":false,"reason":"double/context click on child-frame references is not wired through the guarded frame route"})));
                    }
                    let mut parts = frame_ref.splitn(3, ':');
                    let frame_id = parts.next().unwrap_or_default();
                    let loader_id = parts.next().unwrap_or_default();
                    let backend = parts.next().unwrap_or_default();
                    let backend_node_id = backend
                        .parse::<i64>()
                        .ok()
                        .filter(|id| *id > 0)
                        .ok_or_else(|| invalid("frame reference has an invalid backend node id"))?;
                    if frame_id.is_empty()
                        || frame_id.len() > 256
                        || loader_id.is_empty()
                        || loader_id.len() > 256
                        || args.outcome.is_none()
                    {
                        return Err(invalid(
                            "frame pointer action requires a valid frame identity and explicit outcome",
                        ));
                    }
                    let outcome = args
                        .outcome
                        .as_ref()
                        .ok_or_else(|| invalid("frame click requires an explicit outcome"))?;
                    let shared = self
                        .legacy
                        .shared_sessions
                        .lock()
                        .await
                        .get(&args.session_id)
                        .cloned()
                        .ok_or_else(|| invalid("unknown shared session_id"))?;
                    let shared = shared.lock().await;
                    if shared.handle.principal != self.legacy.principal.as_ref() {
                        return Err(invalid("session is not owned by this server principal"));
                    }
                    let connection = shared
                        .connection
                        .as_ref()
                        .ok_or_else(|| invalid("shared extension session has not been accepted"))?;
                    let response = connection.command(shared.registry()?, &shared.handle, &args.chrome_tab_id, "Controlla.frameClick", serde_json::json!({"frame_id":frame_id,"loader_id":loader_id,"backend_node_id":backend_node_id,"outcome":outcome,"input_mode":if args.action == "touch" {"touch"} else {"mouse"},"timeout_ms":args.timeout_ms.unwrap_or(5000).clamp(250,10000)})).await;
                    match response {
                        Ok(response) => {
                            let value = frame_click_receipt(response);
                            let status = if value["verified"] == true {
                                "verified"
                            } else if value["dispatch_may_have_occurred"] == false {
                                "not_dispatched"
                            } else {
                                "unknown"
                            };
                            json!({"status":status,"dispatch_acknowledged":value["dispatch_acknowledged"],"dispatch_may_have_occurred":value["dispatch_may_have_occurred"],"verified":value["verified"] == true,"reference":reference,"source_frame_id":frame_id,"requested_outcome":outcome,"reason":value["reason"],"dispatch":value})
                        }
                        Err(error) => {
                            json!({"status":"unknown","dispatch_acknowledged":null,"dispatch_may_have_occurred":true,"verified":false,"reference":reference,"source_frame_id":frame_id,"requested_outcome":outcome,"reason":format!("frame click delivery or readback became uncertain; inspect the frame before retrying: {error}")})
                        }
                    }
                } else {
                    let outcome = args.outcome.clone().ok_or_else(|| {
                        invalid("pointer action requires an explicit bounded outcome")
                    })?;
                    retained_click(self, &args, reference, outcome).await?
                }
            }
            "secure_fill" => secure_password_fill(self, &args, reference).await?,
            "fill" | "select" | "slider" => {
                retained_mutation(self, &args, reference, false, &args.action).await?
            }
            _ => {
                return Err(invalid(
                    "action must be click, double_click, context_click, touch, fill, secure_fill, select, or slider",
                ));
            }
        };
        normalize_unknown_receipts(&mut result);
        Ok(Json(result))
    }

    #[tool(
        name = "browser_extract",
        description = "Run a bounded structured extraction from one explicitly paired Chrome tab while requiring the root frame, loader, and URL to remain stable across the read."
    )]
    async fn browser_extract(
        &self,
        Parameters(args): Parameters<BrowserExtractArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let relevance_goal = args.goal.clone();
        let result = self
            .legacy
            .shared_observe(Parameters(SharedObserveArgs {
                session_id: args.session_id,
                chrome_tab_id: args.chrome_tab_id,
                spec: ObserveInput {
                    selector: args.selector,
                    fields: args.fields,
                    max_items: args.max_items.unwrap_or(100).clamp(1, 500),
                    max_text_chars: args.max_text_chars.unwrap_or(10_000).clamp(1, 10_000),
                    max_bytes: args.max_bytes.unwrap_or(200_000).clamp(4096, 1_000_000),
                    cursor: None,
                },
            }))
            .await;
        match result {
            Ok(value) => {
                let mut value = value.0;
                if let Some(items) = value["items"].as_array().cloned() {
                    value["items"] = json!(crate::relevance::filter_items(
                        &items,
                        relevance_goal.as_deref(),
                        args.max_items.unwrap_or(100)
                    ));
                }
                crate::relevance::mark_untrusted(&mut value);
                Ok(Json(value))
            }
            Err(error) => match selector_error_receipt(&error.message) {
                Some(receipt) => Ok(Json(receipt)),
                None => Err(error),
            },
        }
    }

    #[tool(
        name = "browser_probe",
        description = "Perform a targeted accessibility probe on one CSS-selected node in an explicitly paired Chrome tab. This is a scoped fallback, not a full-page screenshot loop."
    )]
    async fn browser_probe(
        &self,
        Parameters(args): Parameters<BrowserProbeArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        let (max_bytes, source, reason) = select_response_budget(
            args.max_bytes,
            args.max_bytes_reason.as_deref(),
            DEFAULT_TARGETED_RESPONSE_BYTES,
        );
        let Json(mut value) = self.legacy
            .shared_accessibility(Parameters(SharedAccessibilityArgs {
                session_id: args.session_id,
                chrome_tab_id: args.chrome_tab_id,
                selector: Some(args.selector),
                frame_id: None,
                max_bytes,
                timeout_ms: args.timeout_ms,
            }))
            .await?;
        value["response_budget"] = json!({"max_bytes":max_bytes,"source":source,"reason":reason});
        bound_snapshot_response(&mut value, max_bytes);
        Ok(Json(value))
    }

    #[tool(
        name = "browser_verify",
        description = "Independently re-observe the selected tab with a fresh snapshot and evaluate a bounded value/text/existence/URL predicate. Caller-supplied prior evidence is never treated as proof."
    )]
    async fn browser_verify(
        &self,
        Parameters(args): Parameters<BrowserVerifyArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        Ok(Json(verify_impl(self, &args).await?))
    }

    #[tool(
        name = "browser_workflow",
        description = "Execute up to 20 deterministic snapshot/find/fill/select/click/verify steps with no internal model calls and no loops. Receipts default to 2 KiB; larger responses require max_bytes_reason and are capped at 1 MiB. Find requires an exact control name, one matching reference, and complete scope evidence; unread unrelated frames are allowed only for a unique root-document exact match. After an unknown mutation, only one immediately following read-only verify step may reconcile its postcondition; no mutation is retried."
    )]
    async fn browser_workflow(
        &self,
        Parameters(args): Parameters<BrowserWorkflowArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        if args.steps.is_empty() || args.steps.len() > MAX_WORKFLOW_STEPS {
            return Err(invalid("workflow must contain 1..20 steps"));
        }
        validate_response_budget(
            args.max_bytes,
            args.max_bytes_reason.as_deref(),
            DEFAULT_RECEIPT_RESPONSE_BYTES,
        )
        .map_err(invalid)?;
        let mut vars = BTreeMap::<String, String>::new();
        let mut receipts: Vec<Value> = Vec::new();
        let response_max_bytes = args.max_bytes.unwrap_or(DEFAULT_RECEIPT_RESPONSE_BYTES);
        let response_reason = args.max_bytes_reason.clone();
        let mut steps = args.steps.into_iter().peekable();
        let mut unknown_receipt_to_reconcile: Option<usize> = None;
        while let Some(step) = steps.next() {
            let reconciliation_index = unknown_receipt_to_reconcile.take();
            if reconciliation_index.is_some() && !is_workflow_reconciliation_step(&step) {
                break;
            }
            let receipt = match step {
                BrowserWorkflowStep::Snapshot => {
                    snapshot_impl(
                        self,
                        &BrowserSnapshotArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            selector: None,
                            max_items: None,
                            max_text_chars: None,
                            max_bytes: None,
                            max_bytes_reason: None,
                            href_hint: None,
                            frame_id: None,
                            include_frames: None,
                            goal: None,
                            max_scan_nodes: None,
                        },
                    )
                    .await?
                }
                BrowserWorkflowStep::Find {
                    query,
                    role,
                    save_as,
                } => {
                    if save_as.is_empty()
                        || save_as.len() > 64
                        || !save_as
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return Err(invalid(
                            "workflow save_as must be 1..64 ASCII alphanumeric/underscore characters",
                        ));
                    }
                    let found = find_impl(
                        self,
                        &BrowserFindArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            query,
                            role,
                            limit: Some(2),
                            max_matches: None,
                            match_mode: Some("exact".into()),
                            within_reference: None,
                            href_hint: None,
                        },
                    )
                    .await?;
                    let matches = found["matches"].as_array().cloned().unwrap_or_default();
                    if !workflow_find_is_usable(&found) {
                        return Err(invalid(format!(
                            "workflow find for {save_as} must resolve exactly one control"
                        )));
                    }
                    let reference = matches[0]["reference"]
                        .as_str()
                        .ok_or_else(|| invalid("workflow find omitted reference"))?
                        .to_owned();
                    vars.insert(save_as.clone(), reference.clone());
                    json!({"kind":"find","save_as":save_as,"reference":reference,"match":matches[0]})
                }
                BrowserWorkflowStep::Fill {
                    reference,
                    expected_value,
                    value,
                } => {
                    let reference = substitute_reference(&reference, &vars)?;
                    retained_mutation(
                        self,
                        &BrowserActArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            action: "fill".into(),
                            reference: Some(reference.clone()),
                            expected_value: Some(expected_value),
                            value: Some(value),
            expected_origin: None,
            account_marker: None,
                            outcome: None,
                            timeout_ms: None,
                        },
                        &reference,
                        false,
                        "fill",
                    )
                    .await?
                }
                BrowserWorkflowStep::Select {
                    reference,
                    expected_value,
                    value,
                } => {
                    let reference = substitute_reference(&reference, &vars)?;
                    retained_mutation(
                        self,
                        &BrowserActArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            action: "select".into(),
                            reference: Some(reference.clone()),
                            expected_value: Some(expected_value),
                            value: Some(value),
            expected_origin: None,
            account_marker: None,
                            outcome: None,
                            timeout_ms: None,
                        },
                        &reference,
                        false,
                        "select",
                    )
                    .await?
                }
                BrowserWorkflowStep::Click { reference, outcome } => {
                    let reference = substitute_reference(&reference, &vars)?;
                    let legacy_reference =
                        resolve_reference(self, &args.session_id, &args.chrome_tab_id, &reference)
                            .await?;
                    let outcome: SharedClickOutcome = serde_json::from_value(
                        serde_json::to_value(outcome).map_err(|e| invalid(e.to_string()))?,
                    )
                    .map_err(|e| invalid(e.to_string()))?;
                    let Json(value) = self
                        .legacy
                        .shared_click(Parameters(SharedClickArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            reference: legacy_reference,
                            outcome,
                            timeout_ms: None,
                            input_mode: None,
                            click_kind: None,
                            expected_target_revision: None,
                        }))
                        .await?;
                    value
                }
                BrowserWorkflowStep::Verify { predicate } => {
                    let verification = verify_impl(
                        self,
                        &BrowserVerifyArgs {
                            session_id: args.session_id.clone(),
                            chrome_tab_id: args.chrome_tab_id.clone(),
                            predicate,
                        },
                    )
                    .await;
                    match verification {
                        Ok(receipt) => receipt,
                        Err(error) if reconciliation_index.is_some() => json!({
                            "status":"incomplete",
                            "reason":format!("bounded read-only reconciliation could not complete: {error}"),
                        }),
                        Err(error) => return Err(error),
                    }
                }
            };
            match record_workflow_receipt(
                &mut receipts,
                receipt,
                reconciliation_index,
                steps.peek(),
            ) {
                WorkflowReceiptProgress::Continue => {}
                WorkflowReceiptProgress::ReconcileUnknown(index) => {
                    unknown_receipt_to_reconcile = Some(index);
                }
                WorkflowReceiptProgress::StopUnknown | WorkflowReceiptProgress::StopFailed => break,
            }
        }
        let terminal = receipts
            .last()
            .and_then(|r| r.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("completed");
        let unreconciled_unknown = receipts.iter().any(|receipt| {
            receipt["status"] == "unknown"
                && receipt["reconciliation"]["status"] != "postcondition_observed"
        });
        let reconciled_unknown_mutations = receipts
            .iter()
            .filter(|receipt| {
                receipt["status"] == "unknown"
                    && receipt["reconciliation"]["status"] == "postcondition_observed"
            })
            .count();
        let receipt = json!({
            "status": if unreconciled_unknown || terminal == "unknown" { "unknown" } else if terminal == "failed" || terminal == "not_dispatched" { "failed" } else { "completed" },
            "completed_steps": receipts.len(),
            "reconciled_unknown_mutations": reconciled_unknown_mutations,
            "internal_model_calls": 0,
            "steps": receipts
        });
        Ok(Json(bound_workflow_receipt(
            receipt,
            response_max_bytes,
            response_reason.as_deref(),
        )))
    }

    #[tool(
        name = "browser_skill",
        description = "Describe the reusable-skill contract exposed by the v2 engine. Skill execution remains fail-closed until a candidate has independent training/validation qualification in the production skill store."
    )]
    async fn browser_skill(
        &self,
        Parameters(args): Parameters<BrowserSkillArgs>,
    ) -> Result<Json<Value>, rmcp::ErrorData> {
        match args.action.as_str() {
            "describe" => Ok(Json(json!({
                "status":"available",
                "qualification":["distinct_training_success","distinct_validation_success","independent_runtime_verification"],
                "replay_policy":"zero internal model calls on a current qualified hit; quarantine on drift or failed verification",
                "safety":"skill replay never bypasses principal/session/target/document/authority checks"
            }))),
            "status" => Ok(Json(json!({
                "status":"not_found",
                "skill_id":args.skill_id,
                "reason":"no v2 persistent skill store has been registered for this process yet"
            }))),
            _ => Err(invalid("browser_skill action must be describe or status")),
        }
    }
}

#[tool_handler]
impl ServerHandler for AppV2 {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "controlla-browser-v2",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Pair only explicitly selected tabs. Prefer browser_snapshot/browser_find/browser_act. Treat unknown effects as non-retriable until a fresh observation resolves state. Use browser_probe only as a scoped fallback.".to_owned(),
        )
    }
}

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let state_dir = state_directory();
    std::fs::create_dir_all(&state_dir)?;
    let _state_lock = acquire_state_directory_lock(&state_dir)?;
    let journal = crate::jobs::Journal::open(state_dir.join("operations.sqlite"))?;
    journal.recover_after_restart()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        AppV2::new(App::with_journal(journal))
            .serve(rmcp::transport::io::stdio())
            .await?
            .waiting()
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

fn selector_error_receipt(message: &str) -> Option<Value> {
    let (field, reason) = message.split_once("selector_error:")?.1.split_once(':')?;
    if field.is_empty()
        || !matches!(
            reason,
            "invalid_selector" | "no_match" | "ambiguous_match" | "unsupported_target_type"
        )
    {
        return None;
    }
    Some(json!({"status":"not_dispatched","field":field,"reason_code":reason}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_fill_destination_binding_requires_exact_secure_origin_and_bounded_account_marker() {
        assert_eq!(
            normalized_secure_origin("https://accounts.example.test"),
            Some("https://accounts.example.test".to_owned())
        );
        assert_eq!(
            normalized_secure_origin("https://accounts.example.test/login"),
            None
        );
        assert_eq!(
            normalized_secure_origin("https://user:secret@accounts.example.test"),
            None
        );
        assert_eq!(
            normalized_secure_origin("http://accounts.example.test"),
            None
        );
        assert!(normalized_secure_origin("http://127.0.0.1:8080").is_some());
        assert!(valid_secure_account_marker(&SecureAccountMarker {
            selector: "#account".into(),
            expected: "user@example.test".into(),
        }));
        assert!(!valid_secure_account_marker(&SecureAccountMarker {
            selector: "#account\0secret".into(),
            expected: "user@example.test".into(),
        }));
        assert!(!valid_secure_account_marker(&SecureAccountMarker {
            selector: "#account".into(),
            expected: " ".into(),
        }));
    }

    #[test]
    fn shared_field_conflicts_yield_without_returning_the_new_value() {
        let conflict = json!({"ok":false,"reason":"same_field_conflict","value":"human edit"});
        assert!(same_field_value_conflict(&conflict));
        let receipt = same_field_conflict_receipt("fill");
        assert_eq!(receipt["status"], "not_dispatched");
        assert_eq!(receipt["reason_code"], "same_field_edit_conflict");
        assert_eq!(receipt["human_edit_preserved"], true);
        assert_eq!(receipt["requires_fresh_snapshot"], true);
        assert!(!receipt.to_string().contains("human edit"));
        assert!(!same_field_value_conflict(&json!({"ok":false,"reason":"blocked"})));
    }

    #[test]
    fn frame_accessibility_aggregation_preserves_unread_and_truncated_coverage() {
        let readable = json!({"source_frame_id":"child-a","frame_loader_id":"loader-a","complete":true,"items":[{"reference":"@f:child-a:loader-a:4"}]});
        let complete = aggregate_frame_accessibility(vec![readable.clone()], vec![], 1, 1, false);
        assert_eq!(complete["complete"], true);
        assert_eq!(complete["frames"][0]["source_frame_id"], "child-a");

        let partial = aggregate_frame_accessibility(
            vec![readable],
            vec![json!({"frame_id":"child-b","reason":"frame_detached"})],
            2,
            2,
            false,
        );
        assert_eq!(partial["complete"], false);
        assert_eq!(partial["unread"][0]["frame_id"], "child-b");

        let tree_truncated = aggregate_frame_accessibility(
            vec![json!({"source_frame_id":"child-a","complete":false,"truncated":true})],
            vec![],
            1,
            1,
            false,
        );
        assert_eq!(tree_truncated["complete"], false);

        let capped = aggregate_frame_accessibility(vec![], vec![], 9, 8, true);
        assert_eq!(capped["complete"], false);
        assert_eq!(capped["truncated"], true);
        assert_eq!(capped["frame_count_attempted"], 8);
    }

    #[test]
    fn url_verification_normalizes_only_equivalent_http_url_forms() {
        assert!(urls_identify_same_resource(
            "https://example.test",
            "https://example.test/"
        ));
        assert!(urls_identify_same_resource(
            "https://example.test:443/a",
            "https://example.test/a"
        ));
        assert!(!urls_identify_same_resource(
            "https://example.test/a",
            "https://example.test/b"
        ));
        assert!(!urls_identify_same_resource(
            "https://example.test/a#one",
            "https://example.test/a#two"
        ));
        assert!(!urls_identify_same_resource(
            "https://example.test/a",
            "https://other.test/a"
        ));
    }

    #[test]
    fn default_semantic_snapshot_budget_is_compact_and_keeps_actionable_identity() {
        assert_eq!(snapshot_collection_budget(12 * 1024, 0), 12 * 1024);
        assert_eq!(snapshot_collection_budget(4096, 6000), 40_096);
        assert_eq!(snapshot_collection_budget(1_048_576, 6000), 100_000);
        assert_eq!(default_snapshot_response_bytes(true), 12 * 1024);
        assert_eq!(default_snapshot_response_bytes(false), 100_000);
        let items = (0..80)
            .map(|index| json!({"reference":format!("@c{index}"),"role":"button","name":format!("Open report {index}"),"rendered":true,"in_viewport":true,"actionable":true,"text":"x".repeat(256)}))
            .collect::<Vec<_>>();
        let mut snapshot = json!({
            "count":items.len(), "scope_complete":true, "root_scope_complete":true,
            "items":items, "delta":{"full":true,"upsert":items,"removed":[]},
            "response_budget":{"max_bytes":4096,"source":"explicit","reason":"test bounded response"},
            "observation":{"mode":"semantic_compact","reason":"default budget fixture","bytes":0}
        });
        let bytes = bound_snapshot_response(&mut snapshot, DEFAULT_TARGETED_RESPONSE_BYTES);
        assert!(bytes <= 12 * 1024);
        assert_eq!(snapshot["truncated"], true);
        assert_eq!(snapshot["scope_complete"], false);
        let first = &snapshot["items"][0];
        assert_eq!(first["reference"], "@c0");
        assert_eq!(first["role"], "button");
        assert_eq!(first["name"], "Open report 0");
        assert_eq!(first["actionable"], true);
        assert_eq!(snapshot["count"], 80);
        assert_eq!(snapshot["observation"]["bytes"], json!(bytes));
        assert_eq!(snapshot["response_budget"]["reason"], "test bounded response");
    }

    #[test]
    fn larger_semantic_observation_budget_requires_and_reports_a_reason() {
        assert!(validate_response_budget(Some(64 * 1024), None, 12 * 1024).is_err());
        assert!(validate_response_budget(Some(12 * 1024), None, 12 * 1024).is_ok());
        let (bytes, source, reason) = select_response_budget(Some(64 * 1024), None, 12 * 1024);
        assert_eq!(bytes, 12 * 1024);
        assert_eq!(source, "default");
        assert_eq!(reason, "small targeted compact observation");

        let (bytes, source, reason) = select_response_budget(
            Some(64 * 1024),
            Some("inspect form fields for review"),
            12 * 1024,
        );
        assert_eq!(bytes, 64 * 1024);
        assert_eq!(source, "explicit");
        assert_eq!(reason, "inspect form fields for review");
    }

    #[test]
    fn default_workflow_receipt_is_bounded_and_keeps_status_and_target_identity() {
        let response = json!({
            "status":"completed", "completed_steps":20, "reconciled_unknown_mutations":0,
            "internal_model_calls":0,
            "steps":(0..20).map(|index| json!({
                "kind":"click", "status":"completed", "reference":format!("@c{index}"),
                "scope_complete":true,
                "document":{"frame_id":"frame-1","loader_id":"loader-1"},
                "page_text":"x".repeat(500), "debug":"x".repeat(300)
            })).collect::<Vec<_>>()
        });
        let bounded = bound_workflow_receipt(response, DEFAULT_RECEIPT_RESPONSE_BYTES, None);
        assert!(serialized_bytes(&bounded) <= DEFAULT_RECEIPT_RESPONSE_BYTES);
        assert_eq!(bounded["status"], "completed");
        assert_eq!(bounded["completed_steps"], 20);
        assert!(bounded["steps"].as_array().unwrap().len() < 20);
        assert_eq!(bounded["returned_steps"], bounded["steps"].as_array().unwrap().len());
        assert!(bounded["omitted_steps"].as_u64().unwrap() > 0);
        assert!(bounded["steps"][0]["reference"].as_str().unwrap().starts_with("@c"));
        assert_eq!(bounded["steps"][0]["document"]["frame_id"], "frame-1");
        assert_eq!(bounded["steps"][0]["scope_complete"], true);
        assert!(bounded["steps"][0].get("page_text").is_none());
        assert!(bounded["steps"][0].get("debug").is_none());
        assert_eq!(bounded["truncated"], true);
    }

    #[test]
    fn larger_workflow_receipt_budget_requires_and_reports_a_reason() {
        let response = json!({"status":"completed", "completed_steps":1, "steps":[]});
        let denied = bound_workflow_receipt(response.clone(), 4096, None);
        assert_eq!(denied["response_budget"]["max_bytes"], DEFAULT_RECEIPT_RESPONSE_BYTES);
        assert_eq!(denied["response_budget"]["source"], "default");

        let requested = bound_workflow_receipt(response, 4096, Some("review all step evidence"));
        assert_eq!(requested["response_budget"]["max_bytes"], 4096);
        assert_eq!(requested["response_budget"]["reason"], "review all step evidence");
        assert_eq!(requested["response_budget"]["source"], "explicit");
    }

    #[test]
    fn snapshot_response_respects_byte_budget_without_duplicate_full_upserts() {
        let items = (0..16)
            .map(|index| json!({"reference":format!("@c{index}"),"name":"x".repeat(512)}))
            .collect::<Vec<_>>();
        let mut snapshot = json!({
            "count":items.len(), "scope_complete":true, "root_scope_complete":true,
            "items":items, "delta":{"full":true,"upsert":items,"removed":[]},
            "observation":{"mode":"semantic_compact","reason":"fixture","bytes":0}
        });
        let bytes = bound_snapshot_response(&mut snapshot, 4096);
        assert!(bytes <= 4096, "serialized snapshot is {bytes} bytes");
        assert_eq!(
            snapshot["delta"]["upsert"],
            json!([]),
            "full snapshots use `items` as their single item payload"
        );
        assert!(snapshot["truncated"] == true);
        assert_eq!(snapshot["scope_complete"], false);
        assert_eq!(snapshot["root_scope_complete"], false);
        assert!(
            snapshot["items"].as_array().unwrap().len()
                < snapshot["count"].as_u64().unwrap() as usize
        );
        assert_eq!(snapshot["observation"]["bytes"], json!(bytes));
        assert_eq!(serde_json::to_vec(&snapshot).unwrap().len() as u64, bytes);
    }

    #[test]
    fn unread_unrelated_frames_do_not_invalidate_a_complete_root_scope() {
        assert!(snapshot_root_scope_complete(
            &json!({"scope_complete":false,"truncation_reason":"cross_origin_frames_unread"})
        ));
        assert!(snapshot_root_scope_complete(
            &json!({"scope_complete":false,"truncation_reason":"same_origin_frames_pending"})
        ));
        assert!(!snapshot_root_scope_complete(
            &json!({"scope_complete":false,"truncation_reason":"scanned_nodes_exceeded"})
        ));
        assert!(!find_scope_is_complete(false, true, false, false));
        assert!(find_scope_is_complete(false, true, true, false));
        assert!(find_scope_is_complete(false, true, false, true));
        let root_match = json!({"source_frame":"main"});
        let frame_match = json!({"source_frame":"frame:7"});
        assert!(exact_root_scope_only(
            false,
            "exact",
            false,
            std::slice::from_ref(&root_match)
        ));
        assert!(!exact_root_scope_only(
            false,
            "exact",
            false,
            &[root_match.clone(), frame_match.clone()]
        ));
        assert!(!exact_root_scope_only(
            false,
            "all",
            false,
            std::slice::from_ref(&root_match)
        ));
        assert!(!exact_root_scope_only(
            false,
            "exact",
            false,
            std::slice::from_ref(&frame_match)
        ));
        assert!(!exact_root_scope_only(
            true,
            "exact",
            false,
            std::slice::from_ref(&root_match)
        ));
        assert!(workflow_find_is_usable(
            &json!({"matches":[root_match.clone()],"scope_complete":true,"ambiguous":false})
        ));
        assert!(!workflow_find_is_usable(
            &json!({"matches":[root_match],"scope_complete":false,"ambiguous":false})
        ));
    }

    #[test]
    fn text_verification_scope_is_explicit_and_selector_failures_are_not_references() {
        assert_eq!(
            parse_text_predicate_scope(None),
            Ok(TextPredicateScope::Root)
        );
        assert_eq!(
            parse_text_predicate_scope(Some("@c12")),
            Ok(TextPredicateScope::Reference("@c12".into()))
        );
        assert_eq!(
            parse_text_predicate_scope(Some("#upload-status")),
            Ok(TextPredicateScope::ScopedSelector("#upload-status".into()))
        );
        assert_eq!(
            parse_text_predicate_scope(Some("@f:frame:loader:2")),
            Ok(TextPredicateScope::FrameReference(
                "@f:frame:loader:2".into()
            ))
        );
        assert!(parse_text_predicate_scope(Some("@f:bad")).is_err());
        assert!(parse_text_predicate_scope(Some("@c")).is_err());
    }

    #[test]
    fn text_absence_requires_complete_dom_evidence_but_presence_is_proven() {
        assert_eq!(text_presence_status(true, true), "passed");
        assert_eq!(text_presence_status(false, false), "failed");
        assert_eq!(text_presence_status(false, true), "incomplete");
    }

    #[test]
    fn observation_learning_requires_relevant_complete_scope_not_just_nonempty_items() {
        let request = crate::observation_router::ObservationRequest {
            query_required: true,
            ..Default::default()
        };
        let irrelevant =
            json!({"items":[{"name":"scoreboard"}],"scope_complete":true,"goal_satisfied":false});
        assert!(!observation_is_usable(request, &irrelevant));
        let truncated = json!({"items":[{"name":"Bills story"}],"scope_complete":true,"goal_satisfied":true,"omitted_by_limit":1});
        assert!(!observation_is_usable(request, &truncated));
        let scoped = json!({"items":[{"name":"Bills story"}],"scope_complete":true,"complete":false,"truncated":true,"incomplete_reasons":["cross_origin_frames_unread"],"goal_satisfied":true});
        assert!(
            observation_is_usable(request, &scoped),
            "unrelated unread frames must not invalidate a complete relevant scope"
        );
    }

    #[test]
    fn frame_scoped_text_verification_requires_matching_loader_and_complete_evidence() {
        let scope = "@f:frame-1:loader-1:42";
        let visible = json!({
            "source_frame_id":"frame-1", "frame_loader_id":"loader-1", "complete":true,
            "items":[{"name":"controlla-classroom-test.txt"}]
        });
        assert_eq!(
            frame_scoped_text_result(scope, "classroom-test.txt", &visible),
            Some((true, false))
        );

        let stale = json!({
            "source_frame_id":"frame-1", "frame_loader_id":"loader-2", "complete":true,
            "items":[{"name":"controlla-classroom-test.txt"}]
        });
        assert_eq!(
            frame_scoped_text_result(scope, "classroom-test.txt", &stale),
            Some((false, false))
        );

        let incomplete = json!({
            "source_frame_id":"frame-1", "frame_loader_id":"loader-1", "complete":false,
            "items":[{"name":"controlla-classroom-test.txt"}]
        });
        assert_eq!(
            frame_scoped_text_result(scope, "classroom-test.txt", &incomplete),
            Some((true, false))
        );
        assert_eq!(frame_scope_parts("@f:frame-1:loader-1:0"), None);
    }

    #[test]
    fn expanded_verifier_predicates_are_exposed_with_typed_inputs() {
        let download: BrowserVerifyArgs = serde_json::from_value(json!({"session_id":"s","chrome_tab_id":"t","predicate":{"kind":"download_artifact","artifact_handle":"opaque","sha256":"a".repeat(64),"min_bytes":12}})).unwrap();
        assert!(matches!(
            download.predicate,
            BrowserPredicate::DownloadArtifact { min_bytes: 12, .. }
        ));
        let attachment: BrowserVerifyArgs = serde_json::from_value(json!({"session_id":"s","chrome_tab_id":"t","predicate":{"kind":"attachment","filename":"report.pdf","size":42}})).unwrap();
        assert!(matches!(
            attachment.predicate,
            BrowserPredicate::Attachment { size: Some(42), .. }
        ));
        let app_specific: BrowserVerifyArgs = serde_json::from_value(json!({"session_id":"s","chrome_tab_id":"t","predicate":{"kind":"app_specific","adapter":"semantic_field_equals","predicate":{"reference":"@c1","field":"selected","equals":"true"}}})).unwrap();
        assert!(matches!(
            app_specific.predicate,
            BrowserPredicate::AppSpecific { .. }
        ));
    }

    #[test]
    fn document_reset_never_reuses_prior_semantic_reference_numbers() {
        let mut state = SemanticTabState {
            document_key: "old-document".into(),
            next_ref: 9,
            ..Default::default()
        };
        reset_semantic_document(
            &mut state,
            "new-document",
            &json!({
                "document":{"frame_id":"frame-2","loader_id":"loader-2","url":"https://example.test/new"}
            }),
        );
        assert_eq!(state.document_key, "new-document");
        assert_eq!(state.next_ref, 9);
        assert!(state.key_to_short.is_empty());
        state.items.insert("@c8".into(), json!({"name":"old"}));
        invalidate_semantic_tab(&mut state);
        assert_eq!(state.next_ref, 9);
        assert!(state.document_key.is_empty());
        assert!(state.items.is_empty());
    }

    #[test]
    fn extraction_selector_failures_are_typed_and_field_bound() {
        let receipt = selector_error_receipt(
            "browser response was invalid: selector_error:announcement:invalid_selector",
        )
        .unwrap();
        assert_eq!(receipt["status"], "not_dispatched");
        assert_eq!(receipt["field"], "announcement");
        assert_eq!(receipt["reason_code"], "invalid_selector");
        assert!(selector_error_receipt("selector_error:body:browser_crash").is_none());
        assert!(selector_error_receipt("unrelated failure").is_none());
    }

    #[test]
    fn missing_or_non_string_semantic_fields_never_match_empty_expected_values() {
        assert!(!semantic_string_field_equals(None, "name", ""));
        assert!(!semantic_string_field_equals(
            Some(&json!({"name": null})),
            "name",
            ""
        ));
        assert!(semantic_string_field_equals(
            Some(&json!({"name": ""})),
            "name",
            ""
        ));
    }

    #[test]
    fn boolean_semantic_attributes_match_string_predicates() {
        assert!(semantic_string_field_equals(
            Some(&json!({"selected": true})),
            "selected",
            "true"
        ));
        assert!(semantic_string_field_equals(
            Some(&json!({"expanded": false})),
            "expanded",
            "false"
        ));
        assert!(!semantic_string_field_equals(
            Some(&json!({"selected": true})),
            "selected",
            "false"
        ));
        assert!(!semantic_string_field_equals(
            Some(&json!({"selected": null})),
            "selected",
            "true"
        ));
    }

    #[test]
    fn repeated_semantic_controls_raise_observation_ambiguity() {
        let unique = [
            json!({"role":"button","name":"Submit"}),
            json!({"role":"link","name":"Help"}),
        ];
        let ambiguous = [
            json!({"role":"button","name":"Open"}),
            json!({"role":"button","name":"Open"}),
            json!({"role":"button","name":"Save"}),
        ];
        assert_eq!(semantic_ambiguity(unique.iter()), 0.0);
        let score = semantic_ambiguity(ambiguous.iter());
        assert!((score - 1.0 / 3.0).abs() < f32::EPSILON);
        assert_eq!(
            crate::observation_router::choose_observation(
                crate::observation_router::ObservationRequest {
                    ambiguity: score,
                    ..Default::default()
                }
            )
            .mode,
            crate::observation_router::ObservationMode::SemanticRich,
        );
        assert_eq!(semantic_ambiguity(std::iter::empty()), 0.0);
    }

    #[test]
    fn scoped_html_expression_quotes_scope_and_redacts_executable_or_input_content() {
        let expression = scoped_html_expression("main[data-x=\"a'\"]", 50_000);
        assert!(expression.contains("main[data-x=\\\"a'\\\"]"));
        assert!(expression.contains(
            "script,style,noscript,iframe,object,embed,svg,input,textarea,select,button"
        ));
        assert!(expression.contains("32_768") || expression.contains("32768"));
        assert!(query_needs_layout("read the chart labels"));
        assert!(!query_needs_layout("summarize this paragraph"));
    }

    #[test]
    fn observation_metadata_reports_the_serialized_payload_size() {
        let mut value = json!({"items":[{"name":"fixture"}]});
        let bytes = attach_observation_metadata(
            &mut value,
            crate::observation_router::ObservationDecision {
                mode: crate::observation_router::ObservationMode::SemanticCompact,
                reason: "test",
            },
        );
        assert_eq!(bytes as usize, serde_json::to_vec(&value).unwrap().len());
        assert_eq!(value["observation"]["bytes"], json!(bytes));
    }

    #[test]
    fn semantic_count_delta_uses_live_counts_and_rejects_underflow() {
        let items = BTreeMap::from([
            ("a".into(), json!({"role":"row"})),
            ("b".into(), json!({"role":"row"})),
            ("c".into(), json!({"role":"button"})),
        ]);
        assert_eq!(semantic_count_delta(&items, "row", 1, 1), (true, 2));
        assert_eq!(semantic_count_delta(&items, "row", 1, -1), (false, 2));
        assert_eq!(semantic_count_delta(&items, "row", 0, -1), (false, 2));
    }

    #[test]
    fn success_toast_and_live_regions_cannot_satisfy_page_outcome_text() {
        assert!(!semantic_text_is_authoritative(
            &json!({"role":"status","text":"Saved successfully"})
        ));
        assert!(!semantic_text_is_authoritative(
            &json!({"role":"generic","feedback_region":true,"text":"Saved successfully"})
        ));
        assert!(semantic_text_is_authoritative(
            &json!({"role":"heading","feedback_region":false,"text":"Saved record"})
        ));
    }

    #[test]
    fn frame_click_keeps_the_native_receipt_when_transport_has_unwrapped_it() {
        let receipt =
            json!({"dispatch_acknowledged":true,"dispatch_may_have_occurred":true,"verified":true});
        assert_eq!(frame_click_receipt(receipt.clone()), receipt);
        assert_eq!(
            frame_click_receipt(json!({"result":{"value":receipt}})),
            receipt
        );
    }

    #[test]
    fn unknown_mutation_receipts_expose_reconciliation_contract_without_walking_page_data() {
        let mut receipt = json!({
            "status":"unknown",
            "dispatch_acknowledged":false,
            "steps":[{"status":"unknown","dispatch_acknowledged":true}],
            "page_content":{"status":"unknown"}
        });
        normalize_unknown_receipts(&mut receipt);
        assert_eq!(receipt["reason_code"], "action_delivery_unknown");
        assert_eq!(
            receipt["mutation_retry_policy"],
            "reconcile_read_only_first"
        );
        assert_eq!(
            receipt["steps"][0]["reason_code"],
            "outcome_verification_timeout"
        );
        assert_eq!(
            receipt["steps"][0]["next_action"],
            "inspect_current_state_once_before_retrying"
        );
        assert!(receipt["page_content"].get("reason_code").is_none());
    }

    #[test]
    fn undispatched_receipts_expose_a_concrete_next_action() {
        let mut receipt = json!({
            "status":"not_dispatched",
            "reason_code":"stale_watcher_revision",
            "dispatch_may_have_occurred":false,
            "page_content":{"status":"not_dispatched"}
        });
        normalize_unknown_receipts(&mut receipt);
        assert_eq!(receipt["next_action"], "refresh_and_reacquire_once");
        assert!(receipt["page_content"].get("next_action").is_none());
    }

    #[test]
    fn only_an_immediate_verify_step_can_follow_an_unknown_workflow_action() {
        let verify = BrowserWorkflowStep::Verify {
            predicate: BrowserPredicate::TextPresent {
                scope: None,
                text: "Saved".into(),
            },
        };
        let snapshot = BrowserWorkflowStep::Snapshot;
        let click = BrowserWorkflowStep::Click {
            reference: "@c1".into(),
            outcome: BrowserClickOutcome::Focused,
        };

        assert!(is_workflow_reconciliation_step(&verify));
        assert!(!is_workflow_reconciliation_step(&snapshot));
        assert!(!is_workflow_reconciliation_step(&click));
        assert!(!is_workflow_reconciliation_step(
            &BrowserWorkflowStep::Fill {
                reference: "@c1".into(),
                expected_value: "".into(),
                value: "mutate".into(),
            }
        ));
    }

    #[test]
    fn pending_navigation_requires_immediate_read_only_reconciliation() {
        let verify = BrowserWorkflowStep::Verify {
            predicate: BrowserPredicate::UrlStartsWith {
                prefix: "https://example.test/".into(),
            },
        };
        let click = BrowserWorkflowStep::Click {
            reference: "@c1".into(),
            outcome: BrowserClickOutcome::Navigation { url: None },
        };
        let mut receipts = Vec::new();
        assert_eq!(
            record_workflow_receipt(
                &mut receipts,
                json!({"status":"dispatched_pending_verification","dispatch_acknowledged":true}),
                None,
                Some(&verify),
            ),
            WorkflowReceiptProgress::ReconcileUnknown(0)
        );
        assert_eq!(
            record_workflow_receipt(
                &mut receipts,
                json!({"status":"dispatched_pending_verification","dispatch_acknowledged":true}),
                None,
                Some(&click),
            ),
            WorkflowReceiptProgress::StopUnknown
        );
    }

    #[test]
    fn passing_unknown_workflow_reconciliation_records_state_without_replaying_action() {
        let verify = BrowserWorkflowStep::Verify {
            predicate: BrowserPredicate::UrlStartsWith {
                prefix: "https://example.test/".into(),
            },
        };
        let mut receipts = Vec::new();
        assert_eq!(
            record_workflow_receipt(
                &mut receipts,
                json!({"status":"unknown","action":"click"}),
                None,
                Some(&verify),
            ),
            WorkflowReceiptProgress::ReconcileUnknown(0),
        );

        assert_eq!(
            record_workflow_receipt(
                &mut receipts,
                json!({"status":"passed","observer":"fresh_snapshot"}),
                Some(0),
                None,
            ),
            WorkflowReceiptProgress::Continue,
        );
        assert_eq!(receipts[0]["status"], "unknown");
        assert_eq!(
            receipts[0]["reconciliation"]["status"],
            "postcondition_observed"
        );
        assert_eq!(
            receipts[0]["reconciliation"]["causality_established"],
            false
        );
        assert_eq!(receipts[0]["reconciliation"]["mutation_retried"], false);
    }

    #[test]
    fn failed_or_incomplete_unknown_reconciliation_stops_before_a_retry() {
        for verify_status in ["failed", "incomplete"] {
            let verify = BrowserWorkflowStep::Verify {
                predicate: BrowserPredicate::TextPresent {
                    scope: None,
                    text: "Saved".into(),
                },
            };
            let retry_click = BrowserWorkflowStep::Click {
                reference: "@c1".into(),
                outcome: BrowserClickOutcome::Focused,
            };
            let mut receipts = Vec::new();
            let first = record_workflow_receipt(
                &mut receipts,
                json!({"status":"unknown","action":"click","reference":"@c1"}),
                None,
                Some(&verify),
            );
            assert_eq!(first, WorkflowReceiptProgress::ReconcileUnknown(0));
            assert!(is_workflow_reconciliation_step(&verify));

            let after_verify = record_workflow_receipt(
                &mut receipts,
                json!({"status":verify_status}),
                Some(0),
                Some(&retry_click),
            );
            assert_eq!(after_verify, WorkflowReceiptProgress::StopUnknown);
            assert_eq!(receipts.len(), 2, "the following click must not execute");
            assert_eq!(receipts[0]["status"], "unknown");
            assert_eq!(receipts[0]["reconciliation"]["status"], "not_confirmed");
            assert_eq!(receipts[0]["reconciliation"]["mutation_retried"], false);
        }
    }

    #[test]
    fn page_order_survives_two_digit_reference_keys() {
        let mut state = SemanticTabState::default();
        state
            .items
            .insert("@c10".into(), json!({"reference":"@c10","index":9}));
        state
            .items
            .insert("@c2".into(), json!({"reference":"@c2","index":1}));
        assert_eq!(state.page_ordered_items()[0]["reference"], "@c2");
    }

    #[test]
    fn stable_keys_disambiguate_duplicate_controls_without_using_mutable_values() {
        let items = vec![
            json!({"role":"textbox","name":"Email","raw_value":"a"}),
            json!({"role":"textbox","name":"Email","raw_value":"b"}),
        ];
        let keys = crate::semantic_state::stable_item_keys(&items, "s1");
        assert_ne!(keys[0], keys[1]);
        let changed = vec![
            json!({"role":"textbox","name":"Email","raw_value":"new-a"}),
            json!({"role":"textbox","name":"Email","raw_value":"new-b"}),
        ];
        assert_ne!(
            keys,
            crate::semantic_state::stable_item_keys(&changed, "s2")
        );
    }

    #[test]
    fn workflow_variable_substitution_fails_closed() {
        let vars = BTreeMap::from([("email".to_owned(), "@c2".to_owned())]);
        assert_eq!(substitute_reference("$email", &vars).unwrap(), "@c2");
        assert_eq!(substitute_reference("email", &vars).unwrap(), "@c2");
        assert!(substitute_reference("$missing", &vars).is_err());
        assert_eq!(substitute_reference("@c1", &vars).unwrap(), "@c1");
    }

    #[test]
    fn exact_find_does_not_claim_uniqueness_from_partial_scope_or_duplicates() {
        let unique = vec![json!({"confidence":"high"})];
        assert!(find_is_ambiguous("exact", false, &unique));
        assert!(!find_is_ambiguous("exact", true, &unique));
        assert!(find_is_ambiguous(
            "exact",
            true,
            &[unique[0].clone(), unique[0].clone()]
        ));
        assert!(!find_is_ambiguous("all", false, &unique));
    }

    #[test]
    fn click_reacquisition_only_follows_a_typed_pre_dispatch_target_change() {
        assert!(click_needs_reacquisition(
            &json!({"status":"not_dispatched","reason_code":"changed_target"})
        ));
        assert!(click_needs_reacquisition(
            &json!({"status":"not_dispatched","reason_code":"stale_watcher_revision"})
        ));
        assert!(mutation_needs_reacquisition(
            &json!({"status":"not_dispatched","readback":{"reason":"target_replaced"}})
        ));
        assert!(!click_needs_reacquisition(
            &json!({"status":"unknown","reason_code":"changed_target"})
        ));
        assert!(!click_needs_reacquisition(
            &json!({"status":"not_dispatched","reason_code":"other"})
        ));
    }
}
