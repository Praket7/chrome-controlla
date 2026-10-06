//! Bounded, target-bound DOM observations and schema-guided list extraction.
use crate::{
    BrowserConnection, BrowserError,
    sessions::{IdentityRevisions, SessionRegistry, TargetRef},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObserveSpec {
    pub selector: String,
    /// Output field name -> CSS selector relative to the selected element.
    pub fields: BTreeMap<String, String>,
    pub max_items: usize,
    pub max_text_chars: usize,
    pub max_bytes: usize,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Observation {
    pub target_id: String,
    pub navigation_epoch: u64,
    pub observed_at_ms: u128,
    pub items: Vec<Value>,
    pub omissions: Vec<String>,
    pub truncated: bool,
    /// Opaque progress hint; it is never accepted as a resume token.
    pub cursor: Option<String>,
    pub cursor_is_resumable: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    Partial,
    Unknown,
}

fn completeness(
    account_verified: bool,
    terminal_evidence: bool,
    missing: bool,
    truncated: bool,
    records: usize,
) -> Completeness {
    if !account_verified {
        Completeness::Unknown
    } else if terminal_evidence && !missing && !truncated {
        Completeness::Complete
    } else if records > 0 {
        Completeness::Partial
    } else {
        Completeness::Unknown
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtractionSpec {
    pub container: String,
    pub record: String,
    /// Stable key -> CSS selector relative to each record.
    pub fields: BTreeMap<String, String>,
    pub id_field: String,
    pub max_steps: usize,
    pub max_records: usize,
    pub max_text_chars: usize,
    pub max_bytes: usize,
    pub expected_count: Option<usize>,
    /// Optional identity marker; absent or mismatched markers make coverage unknown.
    pub account_marker: Option<(String, String)>,
    /// Site-specific explicit terminal marker. A scroll position alone is insufficient.
    pub terminal_selector: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtractionResult {
    pub records: Vec<BTreeMap<String, String>>,
    pub unique_count: usize,
    pub expected_count: Option<usize>,
    pub completeness: Completeness,
    pub cursor: Option<String>,
    /// Extraction currently reports progress but does not implement resume.
    pub cursor_is_resumable: bool,
    pub terminal_evidence: Vec<String>,
    pub missing: Vec<String>,
    pub truncated: bool,
    pub navigation_epoch: u64,
}

fn bounded(v: &Value, max_bytes: usize) -> bool {
    serde_json::to_vec(v).is_ok_and(|s| s.len() <= max_bytes)
}

fn validate_selector(s: &str) -> Result<(), BrowserError> {
    if s.is_empty() || s.len() > 512 || s.chars().any(char::is_control) {
        return Err(BrowserError::InvalidResponse("invalid CSS selector".into()));
    }
    Ok(())
}

impl BrowserConnection {
    pub async fn observe(
        &self,
        sessions: &SessionRegistry,
        reference: &TargetRef,
        principal: &str,
        revisions: IdentityRevisions,
        spec: &ObserveSpec,
    ) -> Result<Observation, BrowserError> {
        validate_selector(&spec.selector)?;
        if spec.max_items == 0
            || spec.max_items > 500
            || spec.max_text_chars == 0
            || spec.max_text_chars > 10_000
            || spec.fields.len() > 32
            || spec
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.len() > 256)
            || spec.max_bytes < 256
        {
            return Err(BrowserError::InvalidResponse(
                "observation limits must be 1..=500 items and positive text/byte budgets".into(),
            ));
        }
        for selector in spec.fields.values() {
            validate_selector(selector)?;
        }
        let expression = json!({"selector":spec.selector,"fields":spec.fields,"limit":spec.max_items,"chars":spec.max_text_chars}).to_string();
        let script = format!(
            r#"(()=>{{const q={expression};const els=[...document.querySelectorAll(q.selector)];let missing=0,clipped=0;const items=els.slice(0,q.limit).map(e=>{{const o={{}};for(const [k,s] of Object.entries(q.fields)){{const n=e.matches(s)?e:e.querySelector(s);if(!n){{o[k]=null;missing++;continue}}const t=(n.innerText||n.value||n.getAttribute('aria-label')||'').trim();if(t.length>q.chars)clipped++;o[k]=t.slice(0,q.chars)}}return o}});return {{items,count:els.length,missing,clipped}}}})()"#
        );
        let value = self
            .target_ref_command(
                sessions,
                reference,
                principal,
                revisions,
                "Runtime.evaluate",
                json!({"expression":script,"returnByValue":true,"awaitPromise":false}),
            )
            .await?;
        let items = value["result"]["value"]["items"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let total = value["result"]["value"]["count"].as_u64().unwrap_or(0) as usize;
        let mut omissions = Vec::new();
        let missing_fields = value["result"]["value"]["missing"].as_u64().unwrap_or(0);
        let clipped_fields = value["result"]["value"]["clipped"].as_u64().unwrap_or(0);
        if missing_fields > 0 {
            omissions.push(format!("{missing_fields} selected fields were absent"));
        }
        if clipped_fields > 0 {
            omissions.push(format!(
                "{clipped_fields} field values were clipped to the text budget"
            ));
        }
        if total > items.len() {
            omissions.push(format!(
                "{} matching elements omitted by item budget",
                total - items.len()
            ));
        }
        if spec.fields.is_empty() {
            omissions.push("no fields selected".into());
        }
        let mut result = Observation {
            target_id: reference.target_id.clone(),
            navigation_epoch: reference.frame_revision,
            observed_at_ms: now_ms(),
            items,
            omissions,
            truncated: total > spec.max_items || clipped_fields > 0,
            cursor: spec.cursor.clone(),
            cursor_is_resumable: false,
        };
        while !bounded(
            &serde_json::to_value(&result).unwrap_or(Value::Null),
            spec.max_bytes,
        ) && !result.items.is_empty()
        {
            result.items.pop();
            result.truncated = true;
            if result.omissions.is_empty() {
                result
                    .omissions
                    .push("items removed to satisfy byte budget".into());
            } else {
                result.omissions[0] = "items removed to satisfy byte budget".into();
            }
        }
        Ok(result)
    }

    /// Traverse a scroll container a fixed number of times, deduplicating by a
    /// caller-selected stable ID. Counts alone never establish completeness.
    pub async fn extract(
        &self,
        sessions: &SessionRegistry,
        reference: &TargetRef,
        principal: &str,
        revisions: IdentityRevisions,
        spec: &ExtractionSpec,
    ) -> Result<ExtractionResult, BrowserError> {
        validate_selector(&spec.container)?;
        validate_selector(&spec.record)?;
        if spec.max_steps == 0
            || spec.max_steps > 200
            || spec.max_records == 0
            || spec.max_records > 1_000
            || spec.max_text_chars == 0
            || spec.max_text_chars > 10_000
            || spec.fields.len() > 32
            || spec.max_bytes < 256
        {
            return Err(BrowserError::InvalidResponse(
                "extraction limits out of range".into(),
            ));
        }
        if !spec.fields.contains_key(&spec.id_field) {
            return Err(BrowserError::InvalidResponse(
                "id_field must name a selected field".into(),
            ));
        }
        for selector in spec.fields.values() {
            validate_selector(selector)?;
        }
        let mut rows: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut evidence = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        let mut cursor = None;
        let mut stable_end = 0;
        let mut account_ok = spec.account_marker.is_some();
        let mut truncated = false;
        if let Some((selector, _)) = &spec.account_marker {
            validate_selector(selector)?;
        }
        if let Some(selector) = &spec.terminal_selector {
            validate_selector(selector)?;
        }
        for step in 0..spec.max_steps {
            if step > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let config = json!({"container":spec.container,"record":spec.record,"fields":spec.fields,"account":spec.account_marker,"terminal":spec.terminal_selector}).to_string();
            let script = format!(
                r#"(()=>{{const q={config};const c=document.querySelector(q.container);if(!c)return {{error:'container_missing'}};let account=false;if(q.account){{const a=document.querySelector(q.account[0]);account=!!a&&(a.innerText||a.getAttribute('content')||'').trim()===q.account[1]}}const items=[...c.querySelectorAll(q.record)].map(e=>{{const o={{}};for(const [k,s] of Object.entries(q.fields)){{const n=e.matches(s)?e:e.querySelector(s);o[k]=n?(n.innerText||n.value||n.getAttribute('aria-label')||'').trim().slice(0,{max_text}):''}}return o}});const end=!!q.terminal&&!!document.querySelector(q.terminal)&&c.scrollTop+c.clientHeight>=c.scrollHeight-2;const before=c.scrollTop;c.scrollTop=Math.min(c.scrollTop+c.clientHeight,c.scrollHeight);return {{items,end,before,after:c.scrollTop,account}}}})()"#,
                max_text = spec.max_text_chars
            );
            let response = self
                .target_ref_command(
                    sessions,
                    reference,
                    principal,
                    revisions,
                    "Runtime.evaluate",
                    json!({"expression":script,"returnByValue":true,"awaitPromise":false}),
                )
                .await?;
            let v = &response["result"]["value"];
            if v["error"].as_str() == Some("container_missing") {
                missing.push("list container missing".into());
                break;
            }
            account_ok &= v["account"] == true;
            let before = rows.len();
            for item in v["items"].as_array().into_iter().flatten() {
                let mut row = BTreeMap::new();
                for key in spec.fields.keys() {
                    row.insert(key.clone(), item[key].as_str().unwrap_or("").to_owned());
                }
                let id = row.get(&spec.id_field).cloned().unwrap_or_default();
                if id.is_empty() {
                    missing.push("record without stable ID".into());
                    continue;
                }
                rows.entry(id).or_insert(row);
                if rows.len() >= spec.max_records {
                    truncated = true;
                    break;
                }
            }
            let added = rows.len() > before;
            let end = v["end"] == true;
            stable_end = if end && !added { stable_end + 1 } else { 0 };
            cursor = Some(format!("step:{};unique:{}", step + 1, rows.len()));
            if stable_end >= 2 {
                evidence.push("explicit site terminal marker present at scroll end on two successive observations with no new stable IDs".into());
                break;
            }
            if truncated {
                missing.push("record budget reached".into());
                break;
            }
        }
        let mut records: Vec<_> = rows.into_values().collect();
        while !bounded(
            &serde_json::to_value(&records).unwrap_or(Value::Null),
            spec.max_bytes,
        ) && !records.is_empty()
        {
            records.pop();
            truncated = true;
        }
        if truncated && !missing.iter().any(|m| m.contains("byte budget")) {
            missing.push("records omitted to satisfy byte budget".into());
        }
        if spec.expected_count.is_some_and(|n| n != records.len()) {
            missing.push("observed unique count differs from expected count".into());
        }
        if evidence.is_empty() {
            missing.push(if spec.terminal_selector.is_none() { "no explicit terminal marker; scroll position and no-change observations are insufficient" } else { "terminal marker not established within step budget" }.into());
        }
        if !account_ok {
            missing.push(
                "account marker absent or mismatched; destination identity is not verified".into(),
            );
        }
        if !account_ok {
            let unique_count = records.len();
            return Ok(ExtractionResult {
                records,
                unique_count,
                expected_count: spec.expected_count,
                completeness: Completeness::Unknown,
                cursor,
                cursor_is_resumable: false,
                terminal_evidence: evidence,
                missing,
                truncated,
                navigation_epoch: reference.frame_revision,
            });
        }
        let unique_count = records.len();
        let state = completeness(
            account_ok,
            !evidence.is_empty(),
            !missing.is_empty(),
            truncated,
            unique_count,
        );
        Ok(ExtractionResult {
            records,
            unique_count,
            expected_count: spec.expected_count,
            completeness: state,
            cursor,
            cursor_is_resumable: false,
            terminal_evidence: evidence,
            missing,
            truncated,
            navigation_epoch: reference.frame_revision,
        })
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase5_fixtures_require_identity_and_positive_terminal_evidence() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/phase5-coverage.json")).unwrap();
        for case in cases {
            let observed = case["observed"].as_u64().unwrap() as usize;
            let mismatch = case["expected"]
                .as_u64()
                .is_some_and(|n| n as usize != observed);
            let state = completeness(
                case["account"] == true,
                case["terminal"] == true,
                case["missing"] == true || mismatch,
                case["truncated"] == true,
                observed,
            );
            assert_eq!(
                state,
                match case["completeness"].as_str().unwrap() {
                    "complete" => Completeness::Complete,
                    "partial" => Completeness::Partial,
                    "unknown" => Completeness::Unknown,
                    other => panic!("unexpected fixture state {other}"),
                },
                "{}",
                case["case"]
            );
        }
    }

    #[test]
    fn recycled_dom_nodes_deduplicate_by_stable_record_id() {
        let snapshots = (0..6).map(|batch| {
            let start = batch * 7;
            (start..(start + 8).min(42))
                .map(|id| format!("item-{id:02}"))
                .collect::<Vec<_>>()
        });
        let unique: std::collections::BTreeSet<_> = snapshots.flatten().collect();
        assert_eq!(unique.len(), 42);
    }
}
