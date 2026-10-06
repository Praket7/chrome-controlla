//! Bounded, target-bound DOM observations and schema-guided list extraction.
use crate::{
    BrowserConnection, BrowserError,
    sessions::{IdentityRevisions, SessionRegistry, TargetRef},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

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
    expected_count_verified: bool,
    terminal_evidence: bool,
    missing: bool,
    truncated: bool,
    records: usize,
) -> Completeness {
    if !account_verified {
        Completeness::Unknown
    } else if expected_count_verified && terminal_evidence && !missing && !truncated {
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
    /// Must be independently authoritative; a UI's possibly stale count is insufficient.
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

fn serialized_len(value: &impl Serialize) -> Result<usize, BrowserError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| BrowserError::InvalidResponse(error.to_string()))
}

fn fit_observation(mut result: Observation, max_bytes: usize) -> Result<Observation, BrowserError> {
    let mut omitted = 0;
    while serialized_len(&result)? > max_bytes && !result.items.is_empty() {
        result.items.pop();
        omitted += 1;
        result.truncated = true;
        result
            .omissions
            .retain(|item| !item.starts_with("result byte budget omitted"));
        result
            .omissions
            .push(format!("result byte budget omitted {omitted} item(s)"));
    }
    if serialized_len(&result)? > max_bytes {
        return Err(BrowserError::InvalidResponse(
            "byte budget is too small for observation metadata".into(),
        ));
    }
    Ok(result)
}

fn fit_extraction(
    mut result: ExtractionResult,
    max_bytes: usize,
) -> Result<ExtractionResult, BrowserError> {
    let mut omitted = 0;
    while serialized_len(&result)? > max_bytes && !result.records.is_empty() {
        result.records.pop();
        omitted += 1;
        result.unique_count = result.records.len();
        result.truncated = true;
        if result.completeness == Completeness::Complete {
            result.completeness = Completeness::Partial;
        }
        result
            .missing
            .retain(|item| !item.starts_with("result byte budget omitted"));
        result
            .missing
            .push(format!("result byte budget omitted {omitted} record(s)"));
    }
    if serialized_len(&result)? > max_bytes {
        return Err(BrowserError::InvalidResponse(
            "byte budget is too small for extraction metadata".into(),
        ));
    }
    Ok(result)
}

fn validate_selector(s: &str) -> Result<(), BrowserError> {
    if s.is_empty() || s.len() > 512 || s.chars().any(char::is_control) {
        return Err(BrowserError::InvalidResponse("invalid CSS selector".into()));
    }
    Ok(())
}

fn validate_field_name(s: &str) -> Result<(), BrowserError> {
    if s.is_empty() || s.len() > 64 || s.chars().any(char::is_control) {
        return Err(BrowserError::InvalidResponse(
            "field names must be 1..=64 printable bytes".into(),
        ));
    }
    Ok(())
}

fn extraction_page_script(config: &str) -> String {
    format!(
        r#"(()=>{{const q={config};let account=false;if(q.account){{const a=document.querySelector(q.account[0]);account=!!a&&(a.innerText||a.getAttribute('content')||'').trim()===q.account[1]}}if(!account)return {{account:false,items:[],end:false,limited:false,missingFields:[],clipped:false}};const c=document.querySelector(q.container);if(!c)return {{error:'container_missing',account:true}};const walker=document.createTreeWalker(c,NodeFilter.SHOW_ELEMENT),items=[],missingFields=new Set(),encoder=new TextEncoder();let limited=false,clipped=false,bytes=0,scanned=0;while(scanned<q.maxScanNodes){{const e=walker.nextNode();if(!e)break;scanned++;if(!e.matches(q.record))continue;if(items.length>=q.maxRecords){{limited=true;break}}const o={{}};for(const [k,s] of Object.entries(q.fields)){{const n=e.matches(s)?e:e.querySelector(s);if(!n){{o[k]=null;missingFields.add(k);continue}}let t=(n.innerText||n.value||n.getAttribute('aria-label')||'').trim();if(t.length>q.maxTextChars){{t=t.slice(0,q.maxTextChars);clipped=true}}o[k]=t}}const size=encoder.encode(JSON.stringify(o)).length+1;if(bytes+size>q.maxBytes-512){{limited=true;break}}items.push(o);bytes+=size}}if(scanned>=q.maxScanNodes)limited=true;const end=!!q.terminal&&!!c.querySelector(q.terminal)&&c.scrollTop+c.clientHeight>=c.scrollHeight-2;const before=c.scrollTop;c.scrollTop=Math.min(c.scrollTop+c.clientHeight,c.scrollHeight);const result={{items,end,before,after:c.scrollTop,account,limited,missingFields:[...missingFields],clipped}};while(encoder.encode(JSON.stringify(result)).length>q.maxBytes&&result.items.length){{result.items.pop();result.limited=true}}if(encoder.encode(JSON.stringify(result)).length>q.maxBytes)return {{budgetError:true}};return result}})()"#,
        config = config
    )
}

fn observation_page_script(config: &str) -> String {
    format!(
        r#"(()=>{{const q={config},walker=document.createTreeWalker(document,NodeFilter.SHOW_ELEMENT),items=[],missing=new Set(),encoder=new TextEncoder();let bytes=0,clipped=0,limited=false,scanned=0,count=0;while(scanned<q.maxScanNodes&&count<=q.limit){{const e=walker.nextNode();if(!e)break;scanned++;if(!e.matches(q.selector))continue;count++;if(count>q.limit){{limited=true;break}}const o={{}};for(const [k,s] of Object.entries(q.fields)){{const n=e.matches(s)?e:e.querySelector(s);if(!n){{o[k]=null;missing.add(k);continue}}const t=(n.innerText||n.value||n.getAttribute('aria-label')||'').trim();if(t.length>q.chars)clipped++;o[k]=t.slice(0,q.chars)}}const size=encoder.encode(JSON.stringify(o)).length+1;if(bytes+size>q.bytes-512){{limited=true;break}}items.push(o);bytes+=size}}if(scanned>=q.maxScanNodes)limited=true;const result={{items,count,missing:[...missing],clipped,limited}};while(encoder.encode(JSON.stringify(result)).length>q.bytes&&result.items.length){{result.items.pop();result.limited=true}}if(encoder.encode(JSON.stringify(result)).length>q.bytes)return {{budgetError:true}};return result}})()"#,
        config = config
    )
}

fn account_marker_script(marker: &Option<(String, String)>) -> String {
    let marker = serde_json::to_string(marker).expect("account marker serializes");
    format!(
        "(()=>{{const m={marker};if(!m)return false;const e=document.querySelector(m[0]);return !!e&&(e.innerText||e.getAttribute('content')||'').trim()===m[1]}})()"
    )
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
            || spec.max_bytes < 4096
        {
            return Err(BrowserError::InvalidResponse(
                "observation limits must be 1..=500 items and positive text/byte budgets".into(),
            ));
        }
        for (name, selector) in &spec.fields {
            validate_field_name(name)?;
            validate_selector(selector)?;
        }
        let max_scan_nodes = spec.max_items.saturating_mul(64).clamp(64, 65_536);
        let expression = json!({"selector":spec.selector,"fields":spec.fields,"limit":spec.max_items,"maxScanNodes":max_scan_nodes,"chars":spec.max_text_chars,"bytes":spec.max_bytes}).to_string();
        let script = observation_page_script(&expression);
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
        if value["result"]["value"]["budgetError"] == true {
            return Err(BrowserError::InvalidResponse(
                "byte budget is too small for page observation metadata".into(),
            ));
        }
        let items = value["result"]["value"]["items"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let total = value["result"]["value"]["count"].as_u64().unwrap_or(0) as usize;
        let mut omissions = Vec::new();
        let missing_fields = value["result"]["value"]["missing"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let clipped_fields = value["result"]["value"]["clipped"].as_u64().unwrap_or(0);
        if !missing_fields.is_empty() {
            omissions.push(format!(
                "selected fields absent: {}",
                missing_fields
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if clipped_fields > 0 {
            omissions.push(format!(
                "{clipped_fields} field values were clipped to the text budget"
            ));
        }
        let page_limited = value["result"]["value"]["limited"] == true;
        if page_limited {
            omissions.push(
                "observation stopped at a scan or byte limit; additional matches may be omitted"
                    .into(),
            );
        } else if total > items.len() {
            omissions.push(format!(
                "{} matching elements omitted by item budget",
                total - items.len()
            ));
        }
        if spec.fields.is_empty() {
            omissions.push("no fields selected".into());
        }
        let result = Observation {
            target_id: reference.target_id.clone(),
            navigation_epoch: reference.frame_revision,
            observed_at_ms: now_ms(),
            items,
            omissions,
            truncated: total > spec.max_items || clipped_fields > 0 || page_limited,
            cursor: spec.cursor.clone(),
            cursor_is_resumable: false,
        };
        fit_observation(result, spec.max_bytes)
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
            || spec.max_bytes < 4096
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
        for (name, selector) in &spec.fields {
            validate_field_name(name)?;
            validate_selector(selector)?;
        }
        let mut rows: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut evidence = Vec::new();
        let mut missing = BTreeSet::new();
        let mut cursor = None;
        let mut stable_end = 0;
        let mut truncated = false;
        if let Some((selector, _)) = &spec.account_marker {
            validate_selector(selector)?;
        }
        if let Some(selector) = &spec.terminal_selector {
            validate_selector(selector)?;
        }
        let account = self.target_ref_command(
            sessions, reference, principal, revisions, "Runtime.evaluate",
            json!({"expression":account_marker_script(&spec.account_marker),"returnByValue":true,"awaitPromise":false}),
        ).await?;
        let mut account_ok = account["result"]["value"] == true;
        if !account_ok {
            missing.insert(
                "account marker absent or mismatched; destination identity is not verified".into(),
            );
            return fit_extraction(
                ExtractionResult {
                    records: Vec::new(),
                    unique_count: 0,
                    expected_count: spec.expected_count,
                    completeness: Completeness::Unknown,
                    cursor: None,
                    cursor_is_resumable: false,
                    terminal_evidence: Vec::new(),
                    missing: missing.into_iter().collect(),
                    truncated: false,
                    navigation_epoch: reference.frame_revision,
                },
                spec.max_bytes,
            );
        }
        for step in 0..spec.max_steps {
            if step > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if rows.len() >= spec.max_records {
                truncated = true;
                missing.insert("record limit reached".to_owned());
                break;
            }
            let max_scan_nodes = spec
                .max_records
                .saturating_sub(rows.len())
                .saturating_mul(64)
                .clamp(64, 65_536);
            let config = json!({"container":spec.container,"record":spec.record,"fields":spec.fields,"account":spec.account_marker,"terminal":spec.terminal_selector,"maxRecords":spec.max_records-rows.len(),"maxScanNodes":max_scan_nodes,"maxTextChars":spec.max_text_chars,"maxBytes":spec.max_bytes}).to_string();
            let script = extraction_page_script(&config);
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
            if response["result"]["value"]["budgetError"] == true {
                return Err(BrowserError::InvalidResponse(
                    "byte budget is too small for page extraction metadata".into(),
                ));
            }
            let v = &response["result"]["value"];
            if v["error"].as_str() == Some("container_missing") {
                missing.insert("list container missing".into());
                break;
            }
            account_ok &= v["account"] == true;
            if !account_ok {
                break;
            }
            if let Some(fields) = v["missingFields"].as_array() {
                for field in fields.iter().filter_map(Value::as_str) {
                    missing.insert(format!("field missing: {field}"));
                }
            }
            if v["clipped"] == true {
                missing.insert("one or more fields clipped by text budget".into());
                truncated = true;
            }
            if v["limited"] == true {
                missing.insert("page result limited by record or byte budget".into());
                truncated = true;
            }
            let before = rows.len();
            for item in v["items"].as_array().into_iter().flatten() {
                let mut row = BTreeMap::new();
                for key in spec.fields.keys() {
                    if let Some(value) = item[key].as_str() {
                        row.insert(key.clone(), value.to_owned());
                    }
                }
                let id = row.get(&spec.id_field).cloned().unwrap_or_default();
                if id.is_empty() {
                    missing.insert("record missing stable ID".into());
                    continue;
                }
                rows.entry(id).or_insert(row);
            }
            let added = rows.len() > before;
            let end = v["end"] == true;
            let expected_covered = spec.expected_count.is_some_and(|count| count == rows.len());
            stable_end = if end && !added && expected_covered {
                stable_end + 1
            } else {
                0
            };
            cursor = Some(format!("step:{};unique:{}", step + 1, rows.len()));
            if stable_end >= 2 {
                evidence.push("explicit site terminal marker present at scroll end on two successive observations with no new stable IDs".into());
                break;
            }
            if truncated {
                break;
            }
        }
        let records: Vec<_> = rows.into_values().collect();
        let expected_count_verified = spec.expected_count.is_some_and(|n| n == records.len());
        if spec.expected_count.is_none() {
            missing.insert("authoritative expected count required for completeness".into());
        } else if !expected_count_verified {
            missing.insert("observed unique count differs from expected count".into());
        }
        if evidence.is_empty() {
            missing.insert(if spec.terminal_selector.is_none() { "no explicit container-scoped terminal marker; scroll position and no-change observations are insufficient" } else { "terminal marker not established with expected coverage within step budget" }.into());
        }
        if !account_ok {
            missing.insert(
                "account marker absent or mismatched; destination identity is not verified".into(),
            );
        }
        let missing: Vec<_> = missing.into_iter().collect();
        if !account_ok {
            let unique_count = records.len();
            return fit_extraction(
                ExtractionResult {
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
                },
                spec.max_bytes,
            );
        }
        let unique_count = records.len();
        let state = completeness(
            account_ok,
            expected_count_verified,
            !evidence.is_empty(),
            !missing.is_empty(),
            truncated,
            unique_count,
        );
        fit_extraction(
            ExtractionResult {
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
            },
            spec.max_bytes,
        )
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
                case["expected"]
                    .as_u64()
                    .is_some_and(|n| n as usize == observed),
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

    #[test]
    fn missing_field_prevents_complete_classification() {
        assert_eq!(
            completeness(true, true, true, true, false, 1),
            Completeness::Partial
        );
        assert_eq!(
            completeness(true, false, true, false, false, 42),
            Completeness::Partial,
            "terminal evidence and no-change reads cannot prove virtualized coverage without an authoritative count"
        );
    }

    #[test]
    fn final_serialized_results_fit_the_declared_byte_budget_or_error() {
        let observation = Observation {
            target_id: "target".into(),
            navigation_epoch: 1,
            observed_at_ms: 2,
            items: vec![json!({"value":"x".repeat(10_000)})],
            omissions: vec![],
            truncated: false,
            cursor: None,
            cursor_is_resumable: false,
        };
        let fitted = fit_observation(observation, 4096).unwrap();
        assert!(serialized_len(&fitted).unwrap() <= 4096);
        let oversized_envelope = Observation {
            target_id: "x".repeat(5000),
            ..fitted
        };
        assert!(fit_observation(oversized_envelope, 4096).is_err());

        let extraction = ExtractionResult {
            records: vec![BTreeMap::from([("value".into(), "x".repeat(10_000))])],
            unique_count: 1,
            expected_count: Some(1),
            completeness: Completeness::Complete,
            cursor: None,
            cursor_is_resumable: false,
            terminal_evidence: vec!["end".into()],
            missing: vec![],
            truncated: false,
            navigation_epoch: 1,
        };
        let fitted = fit_extraction(extraction, 4096).unwrap();
        assert!(serialized_len(&fitted).unwrap() <= 4096);
        assert_eq!(fitted.completeness, Completeness::Partial);
        let oversized_envelope = ExtractionResult {
            cursor: Some("x".repeat(5000)),
            ..fitted
        };
        assert!(fit_extraction(oversized_envelope, 4096).is_err());
    }

    #[test]
    fn extraction_script_checks_account_before_scroll_assignment() {
        let script = extraction_page_script(r##"{"account":["#account","expected"]}"##);
        let reject = script.find("if(!account)return").unwrap();
        let scroll = script.find("c.scrollTop=").unwrap();
        assert!(reject < scroll);
    }

    #[test]
    fn extraction_page_script_caps_records_text_and_bytes_before_response() {
        let script = extraction_page_script("config");
        assert!(script.contains("while(scanned<q.maxScanNodes)"));
        assert!(!script.contains("querySelectorAll(q.record)"));
        assert!(script.contains("q.maxTextChars"));
        assert!(script.contains("q.maxBytes-512"));
        assert!(script.contains("budgetError:true"));

        let observation = observation_page_script("config");
        assert!(observation.contains("while(scanned<q.maxScanNodes"));
        assert!(observation.contains("q.chars"));
        assert!(observation.contains("q.bytes-512"));
        assert!(observation.contains("budgetError:true"));
    }

    #[tokio::test]
    async fn wrong_account_preflight_sends_no_scroll_command() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::net::TcpListener;
        use tokio_tungstenite::tungstenite::Message;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for event in [
                json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"observe-tab","type":"page","url":"https://fixture.test/"}}}),
                json!({"method":"Target.attachedToTarget","params":{"sessionId":"observe-session","targetInfo":{"targetId":"observe-tab","type":"page"}}}),
                json!({"sessionId":"observe-session","method":"Page.frameNavigated","params":{"frame":{"id":"observe-frame","loaderId":"observe-load","url":"https://fixture.test/"}}}),
            ] {
                socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .unwrap();
            }
            let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let request: Value = serde_json::from_str(&message.to_string()).unwrap();
            let expression = request["params"]["expression"].as_str().unwrap();
            assert!(expression.contains("#account"));
            assert!(!expression.contains("scrollTop"));
            socket.send(Message::Text(json!({"id":request["id"],"sessionId":"observe-session","result":{"result":{"type":"boolean","value":false}}}).to_string().into())).await.unwrap();
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), socket.next())
                    .await
                    .is_err(),
                "wrong account must not dispatch the scroll evaluation"
            );
        });
        let connection = BrowserConnection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        let mut sessions = SessionRegistry::new(crate::sessions::ProviderGrants {
            dedicated_headed: true,
            ..Default::default()
        });
        let session = sessions
            .create_session(
                crate::sessions::SessionSpec {
                    mode: crate::sessions::SessionMode::Headed,
                    selected_target_ids: vec![],
                },
                "fixture-principal",
            )
            .unwrap();
        sessions
            .bind_session_to_browser(&session, connection.instance_id)
            .unwrap();
        let reference = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if connection
                    .targets
                    .read()
                    .await
                    .targets
                    .contains_key("observe-tab")
                    && connection
                        .frames
                        .read()
                        .await
                        .frames
                        .contains_key("observe-frame")
                {
                    sessions
                        .register_tab(
                            &session.id,
                            "observe-tab",
                            crate::sessions::Ownership::Borrowed,
                        )
                        .unwrap();
                    break connection
                        .capture_target_ref(
                            &sessions,
                            &session,
                            "observe-tab",
                            "observe-frame",
                            1,
                            1,
                        )
                        .await
                        .unwrap();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let spec = ExtractionSpec {
            container: "#list".into(),
            record: ".row".into(),
            fields: BTreeMap::from([("id".into(), ".id".into())]),
            id_field: "id".into(),
            max_steps: 2,
            max_records: 10,
            max_text_chars: 64,
            max_bytes: 4096,
            expected_count: Some(1),
            account_marker: Some(("#account".into(), "right".into())),
            terminal_selector: Some("[data-end]".into()),
        };
        let result = connection
            .extract(
                &sessions,
                &reference,
                "fixture-principal",
                IdentityRevisions {
                    account: 1,
                    document: 1,
                },
                &spec,
            )
            .await
            .unwrap();
        assert_eq!(result.completeness, Completeness::Unknown);
        assert_eq!(result.unique_count, 0);
        server.await.unwrap();
    }
}
