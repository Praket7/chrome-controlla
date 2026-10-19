use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientConfig, Implementation, ProtocolVersion},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};
use tokio::process::Command;

#[derive(Clone)]
struct Client;
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("espn-latest-story-regression", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}

fn state_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "controlla-espn-story-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn fixture_server() -> (String, thread::JoinHandle<()>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        while !thread_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 2048];
                    let count = stream.read(&mut request).unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..count]);
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    let (status, body) = match path {
                        "/" => ("200 OK", include_str!("../../../bench/fixtures/espn_bills_latest_story/index.html").to_owned()),
                        "/live-shaped" => {
                            let scoreboard = (0..96).map(|index| format!("<a href='/game/{index}'>Gamecast Game {index}</a>")).collect::<String>();
                            ("200 OK", format!("<html><head><title>Buffalo Bills latest stories</title></head><body><main><h1>Buffalo Bills</h1><nav>{scoreboard}</nav><section id='news'><article><a href='/nfl/story/older-relative'>Bills look ahead to Sunday</a><time>3h</time></article><article><a href='/nfl/story/newest-relative'>Josh Allen: Buffalo's latest injury update</a><time>2h</time></article></section><iframe src='https://ads.example.test/embed'></iframe></main></body></html>"))
                        },
                        "/nfl/story/older" => ("200 OK", r#"<html><head><script type="application/ld+json">{"@type":"NewsArticle","url":"/nfl/story/older","datePublished":"2026-10-09T16:30:00Z"}</script></head><body><h1>Bills look ahead to Sunday</h1></body></html>"#.to_owned()),
                        "/nfl/story/newest" => ("200 OK", r#"<html><head><script type="application/ld+json">{"@type":"NewsArticle","url":"/nfl/story/newest","datePublished":"2026-10-11T12:00:00Z"}</script></head><body><h1>Josh Allen: Buffalo's latest injury update</h1></body></html>"#.to_owned()),
                        "/overlay" => ("200 OK", include_str!("../../../bench/fixtures/espn_bills_latest_story/index.html").replace("</body>", "<div style='position:fixed;inset:0;background:#0008;z-index:99999'>fixture obstruction</div></body>")),
                        "/linked" => ("200 OK", "<html><head><title>Linked story metadata</title></head><body><main><article><a href='/article-older.html'>Bills look ahead to the weekend</a></article><article><a href='/article-newest.html'>Bills latest: injury report before Sunday</a></article></main></body></html>".to_owned()),
                        "/listed" => ("200 OK", r#"<html><head><title>Listed story metadata</title><script type="application/ld+json">{"@graph":[{"@type":"NewsArticle","url":"/article-older.html","datePublished":"2026-10-09T16:30:00Z"},{"@type":"NewsArticle","url":"/article-newest.html","datePublished":"2026-10-10T12:00:00Z"}]}</script></head><body><main><article><a href='/article-older.html'>Bills look ahead to the weekend</a></article><article><a href='/article-newest.html'>Bills latest: injury report before Sunday</a></article></main></body></html>"#.to_owned()),
                        "/nested" => ("200 OK", "<html><body><main><div id='shadow'></div><div id='masked' data-masked></div><iframe src='/frame'></iframe><iframe sandbox='allow-scripts' srcdoc='<a href=/opaque>Unread</a>'></iframe></main><script>let r=document.getElementById('shadow').attachShadow({mode:'open'});r.innerHTML='<article data-id=shadow-card><a href=/article-newest.html>Shadow story</a><time datetime=2026-10-10T12:00:00Z>Today</time></article>';document.getElementById('masked').attachShadow({mode:'open'}).innerHTML='<a href=/secret>Secret story</a>'</script></body></html>".to_owned()),
                        "/frame" => ("200 OK", "<html><body><p>SAME_ORIGIN_FRAME_SENTINEL</p><article data-id=frame-card><a href=/article-older.html>Frame story</a></article></body></html>".to_owned()),
                        "/article-newest.html" => ("200 OK", include_str!("../../../bench/fixtures/espn_bills_latest_story/article-newest.html").to_owned()),
                        "/article-older.html" => ("200 OK", include_str!("../../../bench/fixtures/espn_bills_latest_story/article-older.html").to_owned()),
                        _ => ("404 Not Found", "not found".to_owned()),
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    (format!("http://{address}"), handle, stop)
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn nested_snapshot_reports_read_and_unread_sources() {
    let _serial = TEST_LOCK.lock().unwrap();
    let state = state_dir();
    std::fs::create_dir_all(&state).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    command.env("CONTROLLA_STATE_DIR", &state).env(
        "COMPTROL_CHROME_EXECUTABLE",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    );
    let client = Client
        .serve(rmcp::transport::child_process::TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();
    let (base, server, stop_server) = fixture_server();
    let call = |name: &str, value: serde_json::Value| {
        CallToolRequestParams::new(name.to_owned())
            .with_arguments(value.as_object().unwrap().clone())
    };
    let launch = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"headless","url":format!("{base}/nested")}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let session = launch["session_id"].as_str().unwrap();
    let tab = launch["target_ids"][0].as_str().unwrap();
    let mut snapshot = serde_json::Value::Null;
    for _ in 0..8 {
        snapshot = client
            .call_tool(call(
                "snapshot",
                serde_json::json!({"session_id":session,"chrome_tab_id":tab,"max_items":20}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        if snapshot["items"].as_array().is_some_and(|items| {
            items.iter().any(|item| item["source_kind"] == "shadow")
                && items
                    .iter()
                    .any(|item| item["source_kind"] == "same_origin_frame")
        }) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let items = snapshot["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|item| item["source_kind"] == "shadow" && item["name"] == "Shadow story"),
        "{snapshot}"
    );
    assert!(
        !items.iter().any(|item| item["name"] == "Secret story"),
        "masked shadow content leaked: {snapshot}"
    );
    assert!(
        items
            .iter()
            .any(|item| item["source_kind"] == "same_origin_frame"
                && item["source_frame_url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/frame"))),
        "{snapshot}"
    );
    assert!(
        snapshot["cross_origin_frames_unread"].as_u64().unwrap_or(0) >= 1,
        "{snapshot}"
    );
    assert_eq!(snapshot["truncation_reason"], "cross_origin_frames_unread");
    assert!(
        snapshot["text"]
            .as_str()
            .unwrap()
            .contains("SAME_ORIGIN_FRAME_SENTINEL"),
        "same-origin frame text omitted despite complete reachable frame scan: {snapshot}"
    );
    assert_eq!(snapshot["first_party_complete"], false);
    let scoped = client
        .call_tool(call(
            "snapshot",
            serde_json::json!({
                "session_id":session,"chrome_tab_id":tab,"href_hint":"/article-"
            }),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(scoped["first_party_complete"], true, "{scoped}");
    let _ = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"release","mode":"headless","session_id":session}),
        ))
        .await;
    client.cancel().await.unwrap();
    stop_server.store(true, Ordering::Relaxed);
    let _ = server.join();
    let _ = std::fs::remove_dir_all(state);
}

async fn run_latest_story(
    overlay: bool,
    linked: bool,
    listed: bool,
    live_shape: bool,
    live_url: Option<String>,
) -> serde_json::Value {
    let started = std::time::Instant::now();
    let state = state_dir();
    std::fs::create_dir_all(&state).unwrap();
    let executable = std::env::var("COMPTROL_CHROME_EXECUTABLE")
        .unwrap_or_else(|_| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    command
        .env("CONTROLLA_STATE_DIR", &state)
        .env("COMPTROL_CHROME_EXECUTABLE", executable);
    let client = Client
        .serve(rmcp::transport::child_process::TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();
    let (base, server, stop_server) = fixture_server();
    let call = |name: &str, value: serde_json::Value| {
        CallToolRequestParams::new(name.to_owned())
            .with_arguments(value.as_object().unwrap().clone())
    };
    let start_url = if let Some(url) = live_url.as_ref() {
        url.clone()
    } else if live_shape {
        format!("{base}/live-shaped")
    } else if listed {
        format!("{base}/listed")
    } else if linked {
        format!("{base}/linked")
    } else if overlay {
        format!("{base}/overlay")
    } else {
        base.clone()
    };
    // Headless Chrome may report the newly created target before a remote
    // navigation has committed. Launch blank, then navigate with one bounded
    // research call; the workflow's first snapshot supplies the task evidence.
    let launch_url = if live_url.is_some() {
        "about:blank"
    } else {
        start_url.as_str()
    };
    let launch = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"headless","url":launch_url}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let session = launch["session_id"].as_str().unwrap();
    let tab = launch["target_ids"][0].as_str().unwrap();
    let live_navigation = if live_url.is_some() {
        Some(client.call_tool(call("browser", serde_json::json!({
            "action":"task","session_id":session,"chrome_tab_id":tab,
            "task":{"kind":"research","urls":[start_url.clone()],"goal":"open the Buffalo Bills team page"}
        }))).await.unwrap().structured_content.unwrap())
    } else {
        None
    };
    let snapshot_args = if live_shape {
        serde_json::json!({"session_id":session,"chrome_tab_id":tab,"goal":{"query":"Buffalo Bills newest story headlines publication dates","roles":["link"],"exhaustive":true},"max_items":8,"max_bytes":8192,"max_text_chars":0,"max_scan_nodes":20000})
    } else {
        serde_json::json!({"session_id":session,"chrome_tab_id":tab,"max_items":20,"max_text_chars":0})
    };
    let snapshot = if live_url.is_some() {
        serde_json::Value::Null
    } else {
        client
            .call_tool(call("snapshot", snapshot_args.clone()))
            .await
            .unwrap()
            .structured_content
            .unwrap()
    };
    if !live_shape && live_url.is_none() {
        assert_eq!(
            snapshot["scope_complete"], true,
            "fixture story candidates need a complete freshness scope: {snapshot}"
        );
    }
    let workflow_steps = if live_shape {
        serde_json::json!([
            {"kind":"snapshot","goal":{"query":"Buffalo Bills newest story headlines publication dates","roles":["link"],"href_hint":"/nfl/story/","exhaustive":true},"max_items":8,"max_text_chars":0,"max_scan_nodes":20000},
            {"kind":"find_all","role":"link","href_hint":"/nfl/story/","max_matches":10,"save_as":"stories"},
            {"kind":"choose","from":"stories","key":"datetime","save_as":"story"},
            {"kind":"navigate_selected","from":"story"},
            {"kind":"verify","predicate":{"kind":"url","equals":"$story.href"}}
        ])
    } else {
        serde_json::json!([
            {"kind":"find_all","role":"link","href_hint":"/article-","max_matches":10,"save_as":"stories"},
            {"kind":"choose","from":"stories","key":"datetime","save_as":"story"},
            {"kind":"click","reference":"$story","outcome":{"kind":"navigation"}},
            {"kind":"verify","predicate":{"kind":"url","equals":"$story.href"}}
        ])
    };
    let workflow = client
        .call_tool(call(
            "workflow",
            serde_json::json!({"session_id":session,"chrome_tab_id":tab,"steps":workflow_steps,
                "max_bytes":8192,"max_bytes_reason":"fixture asserts each guarded workflow step"}),
        ))
        .await;
    let result = match workflow {
        Ok(response) => response.structured_content.unwrap_or_default(),
        Err(error) => {
            serde_json::json!({"status":"workflow_error","error":error.to_string(),"steps":2})
        }
    };
    let _ = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"release","mode":"headless","session_id":session}),
        ))
        .await;
    client.cancel().await.unwrap();
    stop_server.store(true, Ordering::Relaxed);
    let _ = server.join();
    let _ = std::fs::remove_dir_all(state);
    let workflow_steps = result["steps"].as_u64().unwrap_or(0);
    let mcp_calls = 2 + usize::from(live_navigation.is_some()) + usize::from(live_url.is_none()); // launch, navigation, optional standalone snapshot, workflow
    serde_json::json!({"launch":launch,"live_navigation":live_navigation,"result":result,"candidate_snapshot":snapshot,"mcp_calls":mcp_calls,"workflow_steps":workflow_steps,"elapsed_ms":started.elapsed().as_millis()})
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn espn_latest_story_flow_completes_and_verifies_newest() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(false, false, false, false, None).await;
    assert_eq!(result["result"]["status"], "completed", "{result}");
    assert_eq!(result["result"]["steps"], 4, "workflow step budget");
    assert!(result["result"]["elapsed_ms"].is_number(), "{result}");
    assert_eq!(result["result"]["step_ms"].as_array().unwrap().len(), 4);
    assert_eq!(result["result"]["receipts"][0]["count"], 2);
    assert!(
        result["result"]["receipts"][1]["selected"]["href"]
            .as_str()
            .unwrap()
            .ends_with("/article-newest.html"),
        "selected reference must target the candidate with the latest fixture timestamp"
    );
    assert!(
        result.to_string().contains("2026-10-10T12:00:00Z"),
        "newest timestamp evidence missing; telemetry {metrics}",
        metrics = serde_json::json!({"mcp_calls":result["mcp_calls"],"workflow_steps":result["workflow_steps"],"elapsed_ms":result["elapsed_ms"]})
    );
    assert!(
        !result.to_string().contains("blocked"),
        "opaque guard result: {result}"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn espn_latest_story_recovers_from_safe_obstruction_without_model_retry() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(true, false, false, false, None).await;
    let click = &result["result"]["receipts"][2];
    assert!(
        click["status"] == "verified"
            || (click["status"] == "not_dispatched"
                && click["reason_code"] == "obscured"
                && click["blocker"].is_object()),
        "obstruction needs bounded recovery or actionable diagnostics; telemetry {metrics}, click {click}",
        metrics = serde_json::json!({"mcp_calls":result["mcp_calls"],"workflow_steps":result["workflow_steps"],"elapsed_ms":result["elapsed_ms"]}),
        click = click
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn linked_article_metadata_resolves_same_origin_story_dates() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(false, true, false, false, None).await;
    assert_eq!(result["result"]["status"], "completed", "{result}");
    assert_eq!(
        result["result"]["receipts"][1]["selected"]["freshness_source"],
        "linked_article_metadata"
    );
    assert!(
        result["result"]["receipts"][1]["selected"]["href"]
            .as_str()
            .unwrap()
            .ends_with("/article-newest.html")
    );
    assert_eq!(result["result"]["receipts"][3]["status"], "passed");
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn listing_json_ld_avoids_linked_page_reads() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(false, false, true, false, None).await;
    assert_eq!(result["result"]["status"], "completed", "{result}");
    assert_eq!(
        result["result"]["receipts"][1]["selected"]["freshness_source"],
        "listing_json_ld"
    );
    assert_eq!(result["result"]["receipts"][3]["status"], "passed");
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn live_shaped_story_task_survives_natural_query_scoreboard_and_unread_embed() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(false, false, false, true, None).await;
    let snapshot = &result["candidate_snapshot"];
    let candidates = snapshot["items"].as_array().unwrap();
    assert!(
        candidates.iter().any(|item| item["href"]
            .as_str()
            .is_some_and(|href| href.ends_with("/nfl/story/newest-relative"))),
        "natural-language query and a large scoreboard must retain the relevant story links: {snapshot}"
    );
    assert!(
        candidates[0]["href"]
            .as_str()
            .is_some_and(|href| href.ends_with("/nfl/story/newest-relative"))
            && candidates[0]["goal_score"].as_i64().unwrap_or_default()
                > candidates
                    .iter()
                    .filter(|item| item["section"] == "nav")
                    .map(|item| item["goal_score"].as_i64().unwrap_or_default())
                    .max()
                    .unwrap_or_default(),
        "relevance-ranked stories must precede scoreboard links within the bounded result: {snapshot}"
    );
    let execution = &result["result"];
    assert!(
        result["mcp_calls"].as_u64().is_some_and(|calls| calls <= 3),
        "live Bills flow must use at most three MCP calls: {result}"
    );
    assert_eq!(execution["status"], "completed", "{result}");
    assert_eq!(
        execution["receipts"][1]["coverage_complete"], true,
        "an unread unrelated cross-origin embed must not invalidate the complete first-party story-link scope: {result}"
    );
    assert_eq!(
        execution["receipts"][3]["status"], "verified",
        "story click must be dispatched only after scope-local freshness proof: {result}"
    );
    assert_eq!(
        execution["receipts"][4]["status"], "passed",
        "selected article URL must be independently verified: {result}"
    );
    assert!(
        execution["receipts"][2]["selected"]["href"]
            .as_str()
            .is_some_and(|href| href.ends_with("/nfl/story/newest-relative")),
        "the latest article should win from linked publication metadata or observed relative-age labels: {result}"
    );
    assert_eq!(
        execution["receipts"][2]["selected"]["freshness_source"],
        "relative_age_label"
    );
    assert_eq!(
        execution["receipts"][2]["freshness_evidence"]["label"],
        "2h"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "live ESPN network qualification; isolated headless Chrome only"]
#[allow(clippy::await_holding_lock)]
async fn live_espn_headless_latest_story_smoke() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(
        false,
        false,
        false,
        true,
        Some("https://www.espn.com/nfl/team/_/name/buf/buffalo-bills".into()),
    )
    .await;
    eprintln!(
        "live ESPN headless evidence: mcp_calls={} total_elapsed_ms={} workflow_elapsed_ms={} workflow_steps={} startup_us={} workflow_metrics={}",
        result["mcp_calls"],
        result["elapsed_ms"],
        result["result"]["elapsed_ms"],
        result["workflow_steps"],
        result["launch"]["metrics"]["startup_us"],
        result["result"]["metrics"],
    );
    let execution = &result["result"];
    assert_eq!(execution["status"], "completed", "{result}");
    assert_eq!(
        execution["receipts"][3]["status"], "verified",
        "live ESPN story click must be guarded and observed: {result}"
    );
    assert_eq!(
        execution["receipts"][4]["status"], "passed",
        "live ESPN article destination must be independently verified: {result}"
    );
    assert!(
        execution["receipts"][2]["selected"]["href"]
            .as_str()
            .is_some_and(|href| href.starts_with("https://www.espn.com/nfl/story/")),
        "selected article must remain within ESPN NFL stories: {result}"
    );
}
