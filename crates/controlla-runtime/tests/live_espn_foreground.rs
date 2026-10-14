#![cfg(target_os = "macos")]

use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientConfig, Implementation, ProtocolVersion},
};
use serde_json::{Value, json};
use tokio::process::Command;

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("live-espn-foreground", "0.2.0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}

fn call(name: &str, value: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(value.as_object().unwrap().clone())
}

fn payload(result: rmcp::model::CallToolResult) -> Value {
    result.structured_content.unwrap_or_else(|| {
        result
            .content
            .iter()
            .find_map(|content| {
                content
                    .as_text()
                    .and_then(|text| serde_json::from_str(&text.text).ok())
            })
            .unwrap_or(Value::Null)
    })
}

#[allow(clippy::await_holding_lock)]
async fn live_espn_open_newest(mode: &str) {
    let _serial = TEST_LOCK.lock().unwrap();
    let command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    let client = Client
        .serve(rmcp::transport::child_process::TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();

    // Exercise the requested live mode directly. Do not convert a bridge error
    // into a headless launch: that would make a foreground/background smoke
    // pass without exercising the shared extension path.
    let opened = client
        .call_tool(call(
            "browser",
            json!({
                "action":"open",
                "mode":mode,
                "url":"https://www.espn.com/nfl/team/_/name/buf/buffalo-bills",
                "idempotency_key":format!("espn-{mode}-{}", std::process::id()),
                "ready_when":{"kind":"url_prefix","prefix":"https://www.espn.com/nfl/team/_/name/buf/buffalo-bills"},
                "steps":[
                    {"kind":"snapshot","goal":{"query":"Buffalo Bills newest story headlines publication dates","roles":["link"],"href_hint":"/nfl/story/","exhaustive":true},"max_items":20,"max_text_chars":0,"max_scan_nodes":20000},
                    {"kind":"find_all","role":"link","href_hint":"/nfl/story/","max_matches":20,"save_as":"stories"},
                    {"kind":"choose","from":"stories","key":"datetime","save_as":"story"},
                    {"kind":"navigate_selected","from":"story"},
                    {"kind":"verify","predicate":{"kind":"url","equals":"$story.href"}}
                ]
            }),
        ))
        .await
        .unwrap_or_else(|error| panic!("{mode} open must use a live paired Controlla bridge: {error}"));
    let opened = payload(opened);
    println!(
        "{mode} ESPN receipt: {}",
        serde_json::to_string_pretty(&opened).unwrap()
    );

    let workflow = &opened["workflow"];
    let chosen_href = workflow["receipts"][2]["selected"]["href"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let result = workflow.clone();
    let actual_mode = opened["mode"].clone();
    let verified = workflow["receipts"][4]["status"] == "passed";
    if let Some(session_id) = opened["session_id"].as_str() {
        let _ = client
            .call_tool(call(
                "browser",
                json!({
                    "action":"release", "mode":mode, "session_id":session_id
                }),
            ))
            .await;
    }
    client.cancel().await.unwrap();

    assert_eq!(
        actual_mode, mode,
        "must not silently switch browser modes: {opened}"
    );
    assert_eq!(
        result["status"], "completed",
        "story workflow failed: {opened}"
    );
    assert!(
        chosen_href.starts_with("https://www.espn.com/nfl/story/"),
        "selection must be a Buffalo Bills NFL story, not a betting, schedule, or navigation link: {opened}"
    );
    assert!(
        verified,
        "article URL must be independently verified: {opened}"
    );
}

#[tokio::test]
#[ignore = "manual live ESPN qualification; requires the loaded Controlla Chrome bridge"]
async fn live_espn_foreground_open_newest() {
    live_espn_open_newest("foreground").await;
}

#[tokio::test]
#[ignore = "manual live ESPN qualification; requires the loaded Controlla Chrome bridge"]
async fn live_espn_background_open_newest() {
    live_espn_open_newest("background").await;
}
