#![cfg(target_os = "macos")]
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientConfig, Implementation, ProtocolVersion},
};
use serde_json::Value;
use tokio::process::Command;
#[derive(Clone)]
struct C;
impl ClientHandler for C {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("live-espn-shared", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}
fn call(n: &str, v: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(n.to_owned()).with_arguments(v.as_object().unwrap().clone())
}
fn payload(r: rmcp::model::CallToolResult) -> Value {
    if let Some(sc) = r.structured_content {
        return sc;
    }
    for c in &r.content {
        if let Some(txt) = c.as_text().and_then(|t| t.text.as_str().into()) {
            if let Ok(v) = serde_json::from_str::<Value>(txt) {
                return v;
            }
            return Value::String(txt.to_string());
        }
    }
    serde_json::json!({"raw":r.content})
}

#[tokio::test]
#[ignore = "manual live ESPN qualification; requires a paired foreground Bills tab"]
async fn live_espn_shared_foreground() {
    // Use DEFAULT CONTROLLA_STATE_DIR (no override) so native host talks to your real Chrome extension
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    // explicitly clear override so default is used - by not setting env
    cmd.env_remove("CONTROLLA_STATE_DIR");
    let client = C
        .serve(rmcp::transport::child_process::TokioChildProcess::new(cmd).unwrap())
        .await
        .unwrap();
    // list tools to confirm connection
    let tools = client.list_tools(Default::default()).await.unwrap();
    println!(
        "tools: {:?}",
        tools
            .tools
            .iter()
            .map(|t| t.name.as_ref())
            .collect::<Vec<_>>()
    );
    // Try discover_shared_tabs (extension route)
    let disc = match client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"discover_shared_tabs"}),
        ))
        .await
    {
        Ok(r) => {
            let p = payload(r);
            println!(
                "discover_shared_tabs: {}",
                serde_json::to_string_pretty(&p)
                    .unwrap()
                    .chars()
                    .take(6000)
                    .collect::<String>()
            );
            p
        }
        Err(e) => {
            println!("discover_shared_tabs failed: {e}");
            Value::Null
        }
    };
    // If discover worked, pair with first tab or create one
    // disc should have snapshot.tabs and host_id
    let mut session = String::new();
    let mut tab = String::new();
    let mut used_shared = false;
    if disc["snapshot"]["host_id"].is_string() || disc["host_id"].is_string() {
        let host_id = disc["snapshot"]["host_id"]
            .as_str()
            .or(disc["host_id"].as_str())
            .unwrap_or("")
            .to_string();
        let tabs = disc["snapshot"]["tabs"]
            .as_array()
            .or(disc["tabs"].as_array())
            .cloned()
            .unwrap_or_default();
        println!("host_id={host_id} tabs={}", tabs.len());
        for t in tabs.iter().take(5) {
            println!("  tab {:?}", t);
        }
        // Require existing Bills tab — ask user to open it manually (shared open not allowed via browser:open in foreground)
        let bills_tab = tabs.iter().find(|t| {
            t["url"]
                .as_str()
                .map(|u| u.contains("buffalo-bills"))
                .unwrap_or(false)
                || t["title"]
                    .as_str()
                    .map(|s| s.to_lowercase().contains("bills"))
                    .unwrap_or(false)
        });
        if bills_tab.is_none() {
            println!(
                "No Bills tab found among {} tabs — please open https://www.espn.com/nfl/team/_/name/buf/buffalo-bills in Chrome foreground, then re-run.",
                tabs.len()
            );
            for t in tabs.iter().take(10) {
                println!(
                    "  tab id {} title {:?} url {:?}",
                    t["id"], t["title"], t["url"]
                );
            }
            panic!(
                "Open ESPN Bills page in a foreground tab and re-run: cargo test -p controlla-runtime --test live_espn_shared -- --nocapture"
            );
        }
        let target_id_str: Option<String> = bills_tab
            .and_then(|t| t["id"].as_i64().or(t["target_id"].as_i64()))
            .map(|n| n.to_string());
        let target_ids: Vec<String> = target_id_str.into_iter().collect();
        println!("selected Bills target_ids {:?}", target_ids);
        let url = "https://www.espn.com/nfl/team/_/name/buf/buffalo-bills";
        if !target_ids.is_empty() && !host_id.is_empty() {
            let pair = client.call_tool(call("browser", serde_json::json!({"action":"pair_shared","host_id":host_id,"target_ids":target_ids}))).await.unwrap();
            let pp = payload(pair);
            println!(
                "pair_shared: {}",
                serde_json::to_string_pretty(&pp)
                    .unwrap()
                    .chars()
                    .take(4000)
                    .collect::<String>()
            );
            // accept — needs session_id; poll until accepted:true
            let mut ap = Value::Null;
            for _ in 0..5 {
                let acc = client
                    .call_tool(call(
                        "browser",
                        serde_json::json!({"action":"accept_shared","session_id":pp["session_id"]}),
                    ))
                    .await
                    .unwrap();
                ap = payload(acc);
                println!(
                    "accept_shared: {}",
                    serde_json::to_string_pretty(&ap)
                        .unwrap()
                        .chars()
                        .take(4000)
                        .collect::<String>()
                );
                if ap["accepted"] == true || ap["accepted"] == Value::Bool(true) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
            session = ap["session_id"]
                .as_str()
                .or(pp["session_id"].as_str())
                .unwrap_or("")
                .to_string();
            // Use the paired Bills tab directly — no need to open/navigate (foreground open is not via browser:open)
            // list_shared_targets confirms pairing; keep original tab id
            tab = target_ids[0].clone();
            if let Ok(r) = client
                .call_tool(call(
                    "browser",
                    serde_json::json!({"action":"list_shared_targets","session_id":session}),
                ))
                .await
            {
                let lp = payload(r);
                println!(
                    "list_shared_targets: {}",
                    serde_json::to_string_pretty(&lp)
                        .unwrap()
                        .chars()
                        .take(4000)
                        .collect::<String>()
                );
            }
            used_shared = !session.is_empty() && !tab.is_empty();
            // let let url var be used to avoid warning
            let _ = url;
        }
    }
    if !used_shared {
        println!(
            "shared pairing not established (no extension/host_id or no tabs) — dumping discover and falling back to headless for diagnosis"
        );
        // Fallback headless just to show snapshot still blocked
        let mut cmd2 = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
        // headless uses temp state so it works isolated
        let state = std::env::temp_dir().join(format!("controlla-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&state).unwrap();
        cmd2.env("CONTROLLA_STATE_DIR", &state);
        let client2 = C
            .serve(rmcp::transport::child_process::TokioChildProcess::new(cmd2).unwrap())
            .await
            .unwrap();
        let launch = client2.call_tool(call("browser", serde_json::json!({"action":"launch","mode":"headless","url":"https://www.espn.com/nfl/team/_/name/buf/buffalo-bills"}))).await.unwrap();
        let lp = payload(launch);
        println!(
            "headless fallback launch: {}",
            serde_json::to_string_pretty(&lp)
                .unwrap()
                .chars()
                .take(3000)
                .collect::<String>()
        );
        let sid = lp["session_id"].as_str().unwrap().to_string();
        let tid = lp["target_ids"][0].as_str().unwrap().to_string();
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        let snap = payload(
            client2
                .call_tool(call(
                    "snapshot",
                    serde_json::json!({"session_id":sid,"chrome_tab_id":tid}),
                ))
                .await
                .unwrap(),
        );
        println!(
            "headless snapshot items {} title {:?}",
            snap["items"].as_array().map(|a| a.len()).unwrap_or(0),
            snap["public"]["title"]
        );
        client2.cancel().await.unwrap();
        let _ = std::fs::remove_dir_all(state);
        panic!(
            "foreground extension not paired — open Chrome with Controlla extension, ensure at least one tab is open, then re-run. Headless fallback shows {} items (blocked) as expected.",
            snap["items"].as_array().map(|a| a.len()).unwrap_or(0)
        );
    }
    println!("USING SHARED session={session} tab={tab}");
    let run = client.call_tool(call("workflow", serde_json::json!({
        "session_id":session,"chrome_tab_id":tab,"steps":[
            {"kind":"snapshot","href_hint":"/story/"},
            {"kind":"find_all","role":"link","href_hint":"/story/","max_matches":32,"save_as":"stories"},
            {"kind":"choose","from":"stories","key":"datetime","save_as":"story"},
            {"kind":"click","reference":"$story","outcome":{"kind":"navigation"}},
            {"kind":"verify","predicate":{"kind":"url","equals":"$story.href"}}
        ]
    }))).await;
    let result = run.map(payload).unwrap_or_else(
        |error| serde_json::json!({"status":"tool_error","reason":error.to_string()}),
    );
    println!(
        "EVIDENCE_SUMMARY {}",
        serde_json::json!({"status":result["status"],"steps":result["steps"],"elapsed_ms":result["elapsed_ms"],"step_ms":result["step_ms"],"coverage":result["receipts"][0]["coverage"],"truncated":result["receipts"][0]["truncated"],"omitted":result["receipts"][0]["omitted_by_limit"],"pending_frames":result["receipts"][0]["same_origin_frames_pending"],"cross_origin_frames":result["receipts"][0]["cross_origin_frames_unread"],"first_party_complete":result["receipts"][0]["first_party_complete"],"find_complete":result["receipts"][1]["coverage_complete"],"story_count":result["receipts"][1]["count"],"choice":result["receipts"][2]["selected"],"freshness":result["receipts"][2]["freshness_evidence"],"candidate_freshness":result["receipts"][2]["candidate_freshness"],"reason_code":result["receipts"][2]["reason_code"],"metadata_error":result["receipts"][2]["metadata_error"],"click":result["receipts"][3]["status"],"click_reason":result["receipts"][3]["reason"],"click_reason_code":result["receipts"][3]["reason_code"],"click_dispatch_acknowledged":result["receipts"][3]["dispatch_acknowledged"],"click_navigation_observed":result["receipts"][3]["navigation_observed"],"click_url":result["receipts"][3]["url"],"verify":result["receipts"][4]["status"],"observed":result["receipts"][4]["observed"]})
    );
    let _ = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"release","mode":"foreground","session_id":session}),
        ))
        .await;
    client.cancel().await.unwrap();
    assert_eq!(
        result["status"], "completed",
        "workflow stopped: {:?} {:?}",
        result["receipts"][2]["reason_code"], result["receipts"][2]["metadata_error"]
    );
    assert_eq!(
        result["receipts"][4]["status"], "passed",
        "verification failed: {:?}",
        result["receipts"][4]
    );
}
