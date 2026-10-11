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
                        "/overlay" => ("200 OK", include_str!("../../../bench/fixtures/espn_bills_latest_story/index.html").replace("</body>", "<div style='position:fixed;inset:0;background:#0008;z-index:99999'>fixture obstruction</div></body>")),
                        "/linked" => ("200 OK", "<html><head><title>Linked story metadata</title></head><body><main><article><a href='/article-older.html'>Bills look ahead to the weekend</a></article><article><a href='/article-newest.html'>Bills latest: injury report before Sunday</a></article></main></body></html>".to_owned()),
                        "/listed" => ("200 OK", r#"<html><head><title>Listed story metadata</title><script type="application/ld+json">{"@graph":[{"@type":"NewsArticle","url":"/article-older.html","datePublished":"2026-10-09T16:30:00Z"},{"@type":"NewsArticle","url":"/article-newest.html","datePublished":"2026-10-10T12:00:00Z"}]}</script></head><body><main><article><a href='/article-older.html'>Bills look ahead to the weekend</a></article><article><a href='/article-newest.html'>Bills latest: injury report before Sunday</a></article></main></body></html>"#.to_owned()),
                        "/nested" => ("200 OK", "<html><body><main><div id='shadow'></div><div id='masked' data-masked></div><iframe src='/frame'></iframe><iframe sandbox='allow-scripts' srcdoc='<a href=/opaque>Unread</a>'></iframe></main><script>let r=document.getElementById('shadow').attachShadow({mode:'open'});r.innerHTML='<article data-id=shadow-card><a href=/article-newest.html>Shadow story</a><time datetime=2026-10-10T12:00:00Z>Today</time></article>';document.getElementById('masked').attachShadow({mode:'open'}).innerHTML='<a href=/secret>Secret story</a>'</script></body></html>".to_owned()),
                        "/frame" => ("200 OK", "<html><body><article data-id=frame-card><a href=/article-older.html>Frame story</a></article></body></html>".to_owned()),
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

async fn run_latest_story(overlay: bool, linked: bool, listed: bool) -> serde_json::Value {
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
    let start_url = if listed {
        format!("{base}/listed")
    } else if linked {
        format!("{base}/linked")
    } else if overlay {
        format!("{base}/overlay")
    } else {
        base.clone()
    };
    let launch = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"headless","url":start_url}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let session = launch["session_id"].as_str().unwrap();
    let tab = launch["target_ids"][0].as_str().unwrap();
    let workflow = client.call_tool(call("workflow", serde_json::json!({"session_id":session,"chrome_tab_id":tab,"steps":[
        {"kind":"snapshot","href_hint":"/article-"},
        {"kind":"find_all","role":"link","href_hint":"/article-","max_matches":10,"save_as":"stories"},
        {"kind":"choose","from":"stories","key":"datetime","save_as":"story"},
        {"kind":"click","reference":"$story","outcome":{"kind":"navigation"}},
        {"kind":"verify","predicate":{"kind":"url","equals":"$story.href"}}
    ]}))).await;
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
    serde_json::json!({"result":result,"mcp_calls":3,"workflow_steps":workflow_steps,"elapsed_ms":started.elapsed().as_millis()})
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn espn_latest_story_flow_completes_and_verifies_newest() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(false, false, false).await;
    assert_eq!(result["result"]["status"], "completed", "{result}");
    assert_eq!(result["result"]["steps"], 5, "workflow step budget");
    assert!(result["result"]["elapsed_ms"].is_number());
    assert_eq!(result["result"]["step_ms"].as_array().unwrap().len(), 5);
    assert_eq!(result["result"]["receipts"][0]["item_count"], 2);
    assert!(
        result["result"]["receipts"][2]["selected"]["href"]
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
    let result = run_latest_story(true, false, false).await;
    let click = &result["result"]["receipts"][3];
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
    let result = run_latest_story(false, true, false).await;
    assert_eq!(result["result"]["status"], "completed", "{result}");
    assert_eq!(
        result["result"]["receipts"][2]["selected"]["freshness_source"],
        "linked_article_metadata"
    );
    assert!(
        result["result"]["receipts"][2]["selected"]["href"]
            .as_str()
            .unwrap()
            .ends_with("/article-newest.html")
    );
    assert_eq!(result["result"]["receipts"][4]["status"], "passed");
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn listing_json_ld_avoids_linked_page_reads() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(false, false, true).await;
    assert_eq!(result["result"]["status"], "completed", "{result}");
    assert_eq!(
        result["result"]["receipts"][2]["selected"]["freshness_source"],
        "listing_json_ld"
    );
    assert_eq!(result["result"]["receipts"][4]["status"], "passed");
}
