#![cfg(target_os = "macos")]
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientConfig, Implementation, ProtocolVersion},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::process::Command;

#[derive(Clone)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("live-espn-foreground", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}
fn state_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "controlla-live-espn-fg-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn call(name: &str, v: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(v.as_object().unwrap().clone())
}
fn tool_payload(r: rmcp::model::CallToolResult) -> Value {
    if let Some(sc) = r.structured_content {
        return sc;
    }
    // fallback: try to parse first content text as json
    for c in &r.content {
        if let Some(txt) = c.as_text().and_then(|t| t.text.as_str().into()) {
            if let Ok(v) = serde_json::from_str::<Value>(txt) {
                return v;
            }
            return Value::String(txt.to_string());
        }
    }
    serde_json::json!({ "raw_content": r.content })
}

#[tokio::test]
#[ignore = "manual live ESPN qualification; requires a live Chrome page"]
async fn live_espn_foreground_open_newest() {
    let state = state_dir();
    std::fs::create_dir_all(&state).unwrap();
    let exe = std::env::var("COMPTROL_CHROME_EXECUTABLE")
        .unwrap_or_else(|_| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    cmd.env("CONTROLLA_STATE_DIR", &state)
        .env("COMPTROL_CHROME_EXECUTABLE", exe);
    let client = Client
        .serve(rmcp::transport::child_process::TokioChildProcess::new(cmd).unwrap())
        .await
        .unwrap();

    let url = "https://www.espn.com/nfl/team/_/name/buf/buffalo-bills";
    println!("launch foreground {url}");
    let launch_json = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        client.call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"foreground","url":url}),
        )),
    )
    .await
    {
        Ok(Ok(r)) => {
            println!("foreground launch succeeded");
            tool_payload(r)
        }
        Ok(Err(e)) => {
            println!(
                "foreground launch failed (expected if extension not paired): {e} -> falling back to headless"
            );
            let r = client
                .call_tool(call(
                    "browser",
                    serde_json::json!({"action":"launch","mode":"headless","url":url}),
                ))
                .await
                .unwrap();
            tool_payload(r)
        }
        Err(_) => {
            println!("foreground launch timed out after 15s -> falling back to headless");
            let r = client
                .call_tool(call(
                    "browser",
                    serde_json::json!({"action":"launch","mode":"headless","url":url}),
                ))
                .await
                .unwrap();
            tool_payload(r)
        }
    };
    println!(
        "launch json: {}",
        serde_json::to_string_pretty(&launch_json)
            .unwrap()
            .chars()
            .take(4000)
            .collect::<String>()
    );
    let session = launch_json["session_id"]
        .as_str()
        .or_else(|| launch_json["sessionId"].as_str())
        .expect("session_id")
        .to_owned();
    let tab = launch_json["target_ids"]
        .as_array()
        .and_then(|a| a.first())
        .or_else(|| launch_json["targetIds"].as_array().and_then(|a| a.first()))
        .expect("target_ids")
        .as_str()
        .expect("target id str")
        .to_owned();
    let mode = launch_json["mode"]
        .as_str()
        .unwrap_or("headless")
        .to_owned();

    // Poll snapshots for CSR hydration — ESPN FITT hydrates via JS, so wait up to ~16s
    let mut snap = Value::Null;
    let mut best_items: Vec<Value> = vec![];
    for attempt in 0..8 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let raw = client
            .call_tool(call(
                "snapshot",
                serde_json::json!({"session_id":session,"chrome_tab_id":tab}),
            ))
            .await
            .unwrap();
        snap = tool_payload(raw);
        let items = snap["items"].as_array().cloned().unwrap_or_default();
        let title = snap["public"]["title"].as_str().unwrap_or("").to_string();
        let item_count = items.len();
        let cand_count = items
            .iter()
            .filter(|i| {
                i["href"]
                    .as_str()
                    .map(|h| h.contains("espn.com"))
                    .unwrap_or(false)
                    && i["role"] == "link"
                    && i["name"].as_str().map(|s| s.len() > 8).unwrap_or(false)
            })
            .count();
        println!(
            "poll {}: title {:?} items {} candidates {} coverage {:?} omitted {:?}",
            attempt + 1,
            title.chars().take(80).collect::<String>(),
            item_count,
            cand_count,
            snap["public"]["coverage"],
            snap["public"]["omitted_by_limit"]
        );
        if cand_count > best_items.len() {
            best_items = items.clone();
            snap = snap.clone();
        }
        // stop early if we have story-like candidates with article hrefs
        let has_story = items.iter().any(|i| {
            i["href"]
                .as_str()
                .map(|h| {
                    h.contains("/story/")
                        || h.contains("/nfl/story")
                        || h.contains("/espn.com/nfl/news")
                })
                .unwrap_or(false)
        });
        if has_story && cand_count >= 5 {
            break;
        }
        if cand_count >= 12 {
            break;
        }
    }
    if snap.is_null() {
        snap = client
            .call_tool(call(
                "snapshot",
                serde_json::json!({"session_id":session,"chrome_tab_id":tab}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap_or(Value::Null);
    }
    println!(
        "final snapshot public {}",
        serde_json::to_string_pretty(&snap["public"])
            .unwrap()
            .chars()
            .take(2000)
            .collect::<String>()
    );
    let items = snap["items"].as_array().cloned().unwrap_or(best_items);
    println!("snapshot items {}", items.len());
    let all_link: Vec<_> = items
        .iter()
        .filter(|i| i["role"] == "link" && i["href"].is_string())
        .cloned()
        .collect();
    println!("all links {}", all_link.len());
    for c in all_link.iter().take(15) {
        println!(
            "  link dt {:?} rect {:?} name {:?} href {:?}",
            c["datetime"],
            c["rect_bucket"],
            c["name"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(90)
                .collect::<String>(),
            c["href"]
        );
    }

    // Prefer article/story links for Bills; fall back to any espn.com link >10 chars
    let mut candidates: Vec<Value> = items
        .iter()
        .filter(|i| {
            let href = i["href"].as_str().unwrap_or("");
            let name = i["name"].as_str().unwrap_or("");
            i["role"] == "link" && href.contains("espn.com") && name.len() > 10
        })
        .cloned()
        .collect();

    // Rank: story/news > schedule/game > other
    candidates.sort_by(|a, b| {
        let rank = |v: &Value| {
            let h = v["href"].as_str().unwrap_or("");
            if h.contains("/story/") {
                0
            } else if h.contains("/news/") {
                1
            } else if h.contains("/nfl/team") {
                2
            } else if h.contains("/game/") {
                3
            } else {
                4
            }
        };
        rank(a).cmp(&rank(b))
    });
    println!("candidates ranked {}", candidates.len());
    for c in candidates.iter().take(10) {
        println!(
            "  cand dt {:?} href {:?} name {:?}",
            c["datetime"],
            c["href"],
            c["name"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(90)
                .collect::<String>()
        );
    }
    // If still none (bot block / CSR not loaded), dump full items for diagnosis and skip gracefully
    if candidates.is_empty() {
        println!("NO candidates — dumping first 10 items for diagnosis");
        for c in items.iter().take(10) {
            println!("  item {:?}", c);
        }
        // Do not panic hard — this is a live-site flake diagnostic; fail gracefully with journal evidence
        let _ = client
            .call_tool(call(
                "browser",
                serde_json::json!({"action":"release","mode":mode,"session_id":session}),
            ))
            .await;
        client.cancel().await.unwrap();
        let _ = std::fs::remove_dir_all(state);
        panic!(
            "no espn.com link candidates found — page likely blocked JS/CSR or bot-detected; see dump above (headless). Try with real profile foreground extension."
        );
    }

    let with_dt: Vec<_> = candidates
        .iter()
        .filter(|c| c["datetime"].is_string())
        .cloned()
        .collect();
    let newest = if !with_dt.is_empty() {
        let mut sorted = with_dt.clone();
        sorted.sort_by(|a, b| a["datetime"].as_str().cmp(&b["datetime"].as_str()));
        let n = sorted.into_iter().last().unwrap();
        println!("selected by datetime newest");
        n
    } else {
        // No datetime evidence — pick topmost (lowest y) story-like candidate as ordering evidence
        // rect_bucket is [x,y,w,h] bucket; use y bucket as ordering hint, else original order
        let mut sorted = candidates.clone();
        sorted.sort_by(|a, b| {
            let ay = a["rect_bucket"]
                .as_array()
                .and_then(|v| v.get(1))
                .and_then(|v| v.as_i64())
                .unwrap_or(9999);
            let by = b["rect_bucket"]
                .as_array()
                .and_then(|v| v.get(1))
                .and_then(|v| v.as_i64())
                .unwrap_or(9999);
            ay.cmp(&by)
        });
        println!(
            "selected by ordering (no datetime evidence) — will report unresolved_latest if not verified as newest"
        );
        sorted.into_iter().next().unwrap()
    };
    println!(
        "chosen newest ref {} title {} href {} dt {:?} section {:?} rect {:?}",
        newest["reference"],
        newest["name"],
        newest["href"],
        newest["datetime"],
        newest["section"],
        newest["rect_bucket"]
    );
    let snap_before = snap.clone();
    let click_raw = client
        .call_tool(call(
            "act",
            serde_json::json!({
                "session_id": session,
                "chrome_tab_id": tab,
                "kind": "click",
                "reference": newest["reference"],
                "outcome": {"kind": "navigation", "url": newest["href"]}
            }),
        ))
        .await
        .unwrap();
    let click = tool_payload(click_raw);
    println!(
        "click: {}",
        serde_json::to_string_pretty(&click)
            .unwrap()
            .chars()
            .take(4000)
            .collect::<String>()
    );
    // Typed guard check — no bare blocked
    let click_str = serde_json::to_string(&click).unwrap();
    assert!(
        !click_str.contains("\"blocked\"") || click_str.contains("reason_code"),
        "bare blocked should not appear - must be typed reason_code"
    );

    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    let snap2_raw = client
        .call_tool(call(
            "snapshot",
            serde_json::json!({"session_id":session,"chrome_tab_id":tab}),
        ))
        .await
        .unwrap();
    let snap2 = tool_payload(snap2_raw);
    println!(
        "after url {} title {}",
        snap2["public"]["url"], snap2["public"]["title"]
    );

    // verify: exact href first, then loose contains if exact fails
    let verify_raw = client
        .call_tool(call(
            "verify",
            serde_json::json!({"session_id":session,"chrome_tab_id":tab,"predicate":{"kind":"url","equals":newest["href"]}}),
        ))
        .await
        .unwrap();
    let mut verify = tool_payload(verify_raw);
    println!(
        "verify(exact): {}",
        serde_json::to_string_pretty(&verify)
            .unwrap()
            .chars()
            .take(3000)
            .collect::<String>()
    );
    let verified = verify["verified"].as_bool().unwrap_or(false);
    if !verified {
        // try contains/matches if exact fails (tracking params stripping)
        if let Some(href) = newest["href"].as_str()
            && let Some(id) = href.split('/').next_back()
        {
            let v2 = client
                    .call_tool(call(
                        "verify",
                        serde_json::json!({"session_id":session,"chrome_tab_id":tab,"predicate":{"kind":"url","contains": id }}),
                    ))
                    .await
                    .unwrap();
            let v2p = tool_payload(v2);
            println!(
                "verify(contains id={id}): {}",
                serde_json::to_string_pretty(&v2p)
                    .unwrap()
                    .chars()
                    .take(2000)
                    .collect::<String>()
            );
            if v2p["verified"].as_bool().unwrap_or(false) {
                verify = v2p;
            }
        }
    }

    let receipt = serde_json::json!({
        "candidate_title": newest["name"],
        "candidate_href": newest["href"],
        "freshness_evidence": newest["datetime"],
        "freshness_kind": if with_dt.is_empty() { "ordering_topmost_y" } else { "datetime" },
        "candidate_section": newest["section"],
        "candidate_reference": newest["reference"],
        "candidates_considered": candidates.len(),
        "with_datetime": with_dt.len(),
        "resulting_url": snap2["public"]["url"],
        "resulting_title": snap2["public"]["title"],
        "verification": verify,
        "click": click,
        "snapshot_before_coverage": snap_before["public"]["coverage"],
        "snapshot_before_items": snap_before["items"].as_array().map(|a| a.len()),
        "mode": mode,
        "note": if with_dt.is_empty() { "no datetime on page — newest claimed by ordering only; verify URL match proves click, not absolute newestness without timestamp" } else { "datetime-based newest" }
    });
    println!(
        "EVIDENCE RECEIPT {}",
        serde_json::to_string_pretty(&receipt).unwrap()
    );

    let _ = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"release","mode":mode,"session_id":session}),
        ))
        .await;
    client.cancel().await.unwrap();
    let _ = std::fs::remove_dir_all(state);
}
