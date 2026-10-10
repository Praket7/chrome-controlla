use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientConfig, Implementation, ProtocolVersion},
    transport::child_process::TokioChildProcess,
};
use std::{
    collections::BTreeSet,
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
            Implementation::new("controlla-v3-contract-test", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}

fn state_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "controlla-v3-contract-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[tokio::test]
async fn compact_v3_advertises_task_route_and_guarded_tools() {
    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    command.env("CONTROLLA_STATE_DIR", &state_dir);
    let transport = TokioChildProcess::new(command).expect("spawn compact v3 MCP server");
    let client = Client
        .serve(transport)
        .await
        .expect("initialize compact v3");
    let tools = client.list_tools(None).await.unwrap();
    let names = tools
        .tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        BTreeSet::from([
            "act",
            "artifact_read",
            "artifact_register",
            "browser",
            "extract",
            "snapshot",
            "task",
            "verify",
            "workflow"
        ])
    );
    assert_eq!(tools.tools.len(), 9);

    let schema = |name: &str| {
        let tool = tools.tools.iter().find(|tool| tool.name == name).unwrap();
        serde_json::to_value(&tool.input_schema).unwrap()
    };
    let browser = schema("browser");
    for property in ["action", "mode", "provider", "session_id", "target_ids"] {
        assert!(
            browser["properties"].get(property).is_some(),
            "browser {property}"
        );
    }
    let act = schema("act");
    for property in [
        "action",
        "reference",
        "expected_value",
        "value",
        "key",
        "typing_mode",
        "delay_ms",
        "client_id",
    ] {
        assert!(act["properties"].get(property).is_some(), "act {property}");
    }
    let workflow_text = serde_json::to_string(&schema("workflow")).unwrap();
    let task = schema("task");
    for property in ["session_id", "chrome_tab_id", "task"] {
        assert!(
            task["properties"].get(property).is_some(),
            "task {property}"
        );
    }
    for step in [
        "snapshot", "find", "click", "fill", "type", "press", "extract", "verify",
    ] {
        assert!(
            workflow_text.contains(step),
            "workflow schema contains {step}"
        );
    }
    let schema_bytes = serde_json::to_vec(&tools.tools).unwrap().len();
    assert!(
        schema_bytes <= 48 * 1024,
        "default tool schema budget is {schema_bytes} bytes"
    );

    let pair = serde_json::json!({"action":"pair_shared","target_ids":["123"],"mode":"foreground"});
    let paired = client
        .call_tool(
            CallToolRequestParams::new("browser").with_arguments(pair.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let session_id = paired["session_id"].as_str().unwrap();
    let request = serde_json::json!({"session_id":session_id,"chrome_tab_id":"123",
        "task":{"kind":"download","reference":"@c1"}});
    let result = client
        .call_tool(
            CallToolRequestParams::new("task").with_arguments(request.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let receipt = result.structured_content.unwrap();
    assert_eq!(receipt["status"], "unsupported", "{receipt}");
    assert_eq!(receipt["steps"], 1);
    assert_eq!(receipt["receipts"][0]["verified"], false);

    let research = serde_json::json!({"session_id":"unpaired","chrome_tab_id":"123",
        "task":{"kind":"research","urls":["https://example.test/a","https://example.test/b"],"selector":"article"}});
    let result = client
        .call_tool(
            CallToolRequestParams::new("task")
                .with_arguments(research.as_object().unwrap().clone()),
        )
        .await
        .expect_err("a V2/unknown session must be rejected before task dispatch");
    assert!(result.to_string().contains("V3 browser route"), "{result}");

    client.cancel().await.unwrap();
    std::fs::remove_dir_all(state_dir).unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires installed Google Chrome and a private headless browser"]
async fn private_headless_download_returns_verified_artifact_bytes() {
    let executable = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    assert!(std::path::Path::new(executable).is_file());
    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    command
        .env("CONTROLLA_STATE_DIR", &state_dir)
        .env("COMPTROL_CHROME_EXECUTABLE", executable);
    let client = Client
        .serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();
    let call = |name: &str, value: serde_json::Value| {
        CallToolRequestParams::new(name.to_owned())
            .with_arguments(value.as_object().unwrap().clone())
    };
    let page = "data:text/html,%3Ca%20href%3D%22data%3Atext%2Fplain%2Cfixture%22%20download%3D%22export.txt%22%3EExport%3C%2Fa%3E%3Cinput%20id%3Dfile%20type%3Dfile%3E%3Cspan%20id%3Daccount%3EAccount%20A%3C%2Fspan%3E";
    let launch = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"headless","url":page}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let session_id = launch["session_id"].as_str().unwrap();
    let tab_id = launch["target_ids"][0].as_str().unwrap();
    let snapshot = client
        .call_tool(call(
            "snapshot",
            serde_json::json!({"session_id":session_id,"chrome_tab_id":tab_id}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let reference = snapshot["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Export")
        .unwrap()["reference"]
        .as_str()
        .unwrap();
    let result = client
        .call_tool(call(
            "task",
            serde_json::json!({"session_id":session_id,"chrome_tab_id":tab_id,
        "task":{"kind":"download","reference":reference}}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(result["status"], "completed", "{result}");
    let handle = result["receipts"][0]["artifact_handle"].as_str().unwrap();
    let read = client
        .call_tool(call(
            "artifact_read",
            serde_json::json!({"session_id":session_id,"artifact_handle":handle}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        read["bytes"],
        serde_json::json!([102, 105, 120, 116, 117, 114, 101])
    );
    assert_eq!(read["sha256"], result["receipts"][0]["sha256"]);
    let registered = client
        .call_tool(call(
            "artifact_register",
            serde_json::json!({"session_id":session_id,
        "filename":"upload.txt","bytes":[117,112,108,111,97,100]}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let upload_handle = registered["artifact_handle"].as_str().unwrap();
    let uploaded = client
        .call_tool(call(
            "task",
            serde_json::json!({"session_id":session_id,"chrome_tab_id":tab_id,
        "task":{"kind":"upload","artifact_handle":upload_handle,"selector":"#file",
            "account_marker":["#account","Account A"]}}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(uploaded["status"], "incomplete", "{uploaded}");
    assert_eq!(uploaded["receipts"][0]["status"], "selected", "{uploaded}");
    assert_eq!(uploaded["receipts"][0]["filename"], "upload.txt");
    let released = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"release","mode":"headless","session_id":session_id}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(released["released"], true, "{released}");
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(state_dir).unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires installed Google Chrome and a private headless browser"]
async fn private_headless_task_research_draft_and_repeat() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut request = [0u8; 2048];
            let size = stream.read(&mut request).await.unwrap_or(0);
            let path = String::from_utf8_lossy(&request[..size]);
            let fact = if path.starts_with("GET /a ") {
                "Fact A"
            } else {
                "Fact B"
            };
            let body = format!(
                "<main>{fact}</main><input aria-label='Name' value='before'><button type='button' onclick=\"document.querySelector('#saved').textContent='Saved'\">Save</button><span id='saved'></span>"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    command.env("CONTROLLA_STATE_DIR", &state_dir).env(
        "COMPTROL_CHROME_EXECUTABLE",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    );
    let client = Client
        .serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();
    let call = |name: &str, value: serde_json::Value| {
        CallToolRequestParams::new(name.to_owned())
            .with_arguments(value.as_object().unwrap().clone())
    };
    let url_a = format!("http://{address}/a");
    let url_b = format!("http://{address}/b");
    let launch = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"headless","url":url_a}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let session_id = launch["session_id"].as_str().unwrap();
    let tab_id = launch["target_ids"][0].as_str().unwrap();
    let research = client
        .call_tool(call(
            "task",
            serde_json::json!({"session_id":session_id,"chrome_tab_id":tab_id,
        "task":{"kind":"research","urls":[url_a,url_b],"selector":"main"}}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(research["status"], "completed", "{research}");
    assert_eq!(research["receipts"][1]["url"], url_a);
    assert_eq!(research["receipts"][1]["snippet"], "Fact A");
    assert_eq!(research["receipts"][3]["url"], url_b);
    assert_eq!(research["receipts"][3]["snippet"], "Fact B");
    let snapshot = client
        .call_tool(call(
            "snapshot",
            serde_json::json!({"session_id":session_id,"chrome_tab_id":tab_id}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let items = snapshot["items"].as_array().unwrap();
    let name = items.iter().find(|item| item["name"] == "Name").unwrap()["reference"]
        .as_str()
        .unwrap();
    let save = items.iter().find(|item| item["name"] == "Save").unwrap()["reference"]
        .as_str()
        .unwrap();
    let draft = client.call_tool(call("task", serde_json::json!({"session_id":session_id,"chrome_tab_id":tab_id,
        "task":{"kind":"form_draft","fields":[{"reference":name,"expected_value":"before","value":"after"}],
            "save_without_submit":true,"save_control":{"reference":save,"confirmation_selector":"#saved","confirmation_text":"Saved"}}})))
        .await.unwrap().structured_content.unwrap();
    assert_eq!(draft["status"], "completed", "{draft}");
    assert_eq!(draft["receipts"][1]["kind"], "save_draft");
    let repeat = client
        .call_tool(call(
            "task",
            serde_json::json!({"session_id":session_id,"chrome_tab_id":tab_id,
        "task":{"kind":"repeat","references":[name,save],"action":"verify_visible"}}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(repeat["status"], "completed", "{repeat}");
    let released = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"release","mode":"headless","session_id":session_id}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(released["released"], true, "{released}");
    client.cancel().await.unwrap();
    server.abort();
    std::fs::remove_dir_all(state_dir).unwrap();
}
