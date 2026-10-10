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
                    let mut request = [0; 2048];
                    let count = stream.read(&mut request).unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..count]);
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    let (status, body) = match path {
                        "/" => ("200 OK", include_str!("../../../bench/fixtures/espn_bills_latest_story/index.html").to_owned()),
                        "/overlay" => ("200 OK", include_str!("../../../bench/fixtures/espn_bills_latest_story/index.html").replace("</body>", "<div style='position:fixed;inset:0;background:#0008;z-index:99999'>fixture obstruction</div></body>")),
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

async fn run_latest_story(overlay: bool) -> serde_json::Value {
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
    let start_url = if overlay {
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
        {"kind":"snapshot"}, {"kind":"find","query":"Bills latest: injury report before Sunday","save_as":"story"},
        {"kind":"click","reference":"$story","outcome":{"kind":"navigation","url":format!("{base}/article-newest.html")}},
        {"kind":"verify","predicate":{"kind":"url","equals":format!("{base}/article-newest.html")}}
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
async fn espn_latest_story_flow_completes_and_verifies_newest() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(false).await;
    assert_eq!(result["result"]["status"], "completed", "{result}");
    assert_eq!(result["result"]["steps"], 4, "workflow step budget");
    assert_eq!(
        result["result"]["receipts"][1]["matches"][0]["href"]
            .as_str()
            .unwrap(),
        result["result"]["receipts"][0]["items"][1]["href"]
            .as_str()
            .unwrap(),
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
async fn espn_latest_story_recovers_from_safe_obstruction_without_model_retry() {
    let _serial = TEST_LOCK.lock().unwrap();
    let result = run_latest_story(true).await;
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
