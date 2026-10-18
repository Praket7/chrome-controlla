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
async fn invalid_tool_arguments_return_readable_tool_errors() {
    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    command.env("CONTROLLA_STATE_DIR", &state_dir);
    let client = Client
        .serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();
    let bad_args = serde_json::json!({"action":"open","url":"https://example.test/","idempotency_key":"bad-args","unknown_field":true});
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser")
                .with_arguments(bad_args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let payload = result.structured_content.unwrap();
    assert_eq!(payload["reason_code"], "invalid_tool_request");
    assert!(
        payload["reason"]
            .as_str()
            .unwrap()
            .contains("unknown_field")
    );
    assert!(
        payload["next_action"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(state_dir).ok();
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
            "act", "browser", "extract", "snapshot", "verify", "workflow"
        ])
    );
    assert_eq!(tools.tools.len(), 6);

    let schema = |name: &str| {
        let tool = tools.tools.iter().find(|tool| tool.name == name).unwrap();
        serde_json::to_value(&tool.input_schema).unwrap()
    };
    let browser = schema("browser");
    let browser_description = tools
        .tools
        .iter()
        .find(|tool| tool.name == "browser")
        .unwrap()
        .description
        .as_deref()
        .unwrap_or_default();
    assert!(browser_description.contains("action=task"));
    assert!(browser_description.contains("one-call private headless mode"));
    assert!(browser_description.contains("read-only Research task without tab_ids"));
    assert!(
        browser["properties"]["ready_when"]["anyOf"].is_array()
            || browser["properties"]["ready_when"]["oneOf"].is_array(),
        "ready_when exposes a tagged readiness policy"
    );
    assert!(
        browser["properties"]["task"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("\"kind\":\"research\""))
    );
    assert!(
        browser["properties"]["task"]["description"]
            .as_str()
            .is_some_and(|description| description
                .to_lowercase()
                .contains("read-only research tasks without tab_ids"))
    );
    assert!(
        browser["properties"]["ready_when"]["description"]
            .as_str()
            .is_some_and(|description| {
                description.contains("Tagged object") && description.contains("document_committed")
            })
    );
    assert!(
        browser["properties"].get("file_path").is_some(),
        "browser file staging exposes the local path fast path"
    );
    for property in [
        "action",
        "mode",
        "provider",
        "session_id",
        "target_ids",
        "chrome_tab_id",
        "url",
        "ready_when",
        "task",
        "filename",
        "bytes",
        "encoding",
        "artifact_handle",
        "client_id",
        "idempotency_key",
        "steps",
    ] {
        assert!(
            browser["properties"].get(property).is_some(),
            "browser {property}"
        );
    }
    assert!(schema("workflow")["properties"].get("program").is_some());
    assert!(
        browser["properties"]["idempotency_key"]["description"]
            .as_str()
            .is_some_and(|value| value.contains("Required for open"))
    );
    assert_eq!(
        browser["allOf"][0]["if"]["properties"]["action"]["const"], "open",
        "schema makes the key conditional on open"
    );
    assert!(
        browser["allOf"][0]["then"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|property| property == "idempotency_key")
    );
    for task_kind in ["research", "form_draft", "upload", "download", "repeat"] {
        assert!(
            browser.to_string().contains(task_kind),
            "browser task schema contains {task_kind}"
        );
    }
    let act = schema("act");
    let snapshot_description = tools
        .tools
        .iter()
        .find(|tool| tool.name == "snapshot")
        .unwrap()
        .description
        .as_deref()
        .unwrap_or_default();
    let act_description = tools
        .tools
        .iter()
        .find(|tool| tool.name == "act")
        .unwrap()
        .description
        .as_deref()
        .unwrap_or_default();
    assert!(snapshot_description.contains("credential_input"));
    assert!(snapshot_description.contains("raw_value=null"));
    let snapshot = schema("snapshot");
    assert!(snapshot.to_string().contains("element_focus"));
    assert!(snapshot.to_string().contains("region_focus"));
    assert!(act_description.contains("allow_password=true"));
    assert!(
        act.to_string()
            .contains("Optional exact absolute HTTP(S) destination")
    );
    for property in [
        "action",
        "reference",
        "expected_value",
        "value",
        "allow_password",
        "key",
        "typing_mode",
        "delay_ms",
        "client_id",
    ] {
        assert!(act["properties"].get(property).is_some(), "act {property}");
    }
    let workflow_text = serde_json::to_string(&schema("workflow")).unwrap();
    assert!(schema("workflow")["properties"].get("program").is_some());
    assert_eq!(
        schema("workflow")["properties"]["retain_text_evidence"]["type"],
        "boolean"
    );
    assert!(
        schema("workflow")["properties"]["retain_text_evidence"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("may include sensitive data"))
    );
    for step in [
        "snapshot", "find", "click", "fill", "type", "press", "extract", "verify", "upload",
    ] {
        assert!(
            workflow_text.contains(step),
            "workflow schema contains {step}"
        );
    }
    let schema_bytes = serde_json::to_vec(&tools.tools).unwrap().len();
    let schema_sizes = tools
        .tools
        .iter()
        .map(|tool| {
            let value = serde_json::to_value(tool).unwrap();
            format!(
                "{}={}",
                value["name"],
                serde_json::to_vec(&value).unwrap().len()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "default MCP tool schemas: {schema_bytes} bytes ({} KiB budget)",
        (60 * 1024) / 1024
    );
    assert!(
        schema_bytes <= 60 * 1024,
        "default tool schema budget is {schema_bytes} bytes ({schema_sizes})"
    );

    let described = client
        .call_tool(
            CallToolRequestParams::new("browser").with_arguments(
                serde_json::json!({"action":"describe"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        described["browser_actions"]["foreground"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("open"))
    );
    let mut surface = tools
        .tools
        .iter()
        .map(|tool| serde_json::to_value(tool).unwrap())
        .collect::<Vec<_>>();
    surface.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let expected_fingerprint = {
        use sha2::{Digest, Sha256};
        Sha256::digest(serde_json::to_vec(&surface).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert_eq!(described["schema_sha256"], expected_fingerprint);
    assert_eq!(
        described["client_loaded_tool_state"],
        "unverified_until_client_acknowledges_surface_fingerprint"
    );
    assert!(
        !described["act_actions"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("upload"))
    );

    let invalid_cold_open = client
        .call_tool(CallToolRequestParams::new("browser").with_arguments(
            serde_json::json!({"action":"open","mode":"foreground","url":"javascript:alert(1)"})
                .as_object()
                .unwrap()
                .clone(),
        ))
        .await
        .unwrap();
    assert!(
        invalid_cold_open.is_error == Some(true)
            && serde_json::to_string(&invalid_cold_open)
                .unwrap()
                .contains("absolute HTTP(S) URL without credentials"),
        "reject invalid navigation before pairing: {invalid_cold_open:?}"
    );
    let invalid_open_workflow = client
        .call_tool(CallToolRequestParams::new("browser").with_arguments(
            serde_json::json!({"action":"open","mode":"foreground","url":"https://example.test/","steps":[]})
                .as_object().unwrap().clone(),
        ))
        .await.unwrap();
    assert!(
        invalid_open_workflow.is_error == Some(true)
            && serde_json::to_string(&invalid_open_workflow)
                .unwrap()
                .contains("workflow requires 1..40 steps")
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
    let failed_workflow = client.call_tool(CallToolRequestParams::new("workflow").with_arguments(
        serde_json::json!({"session_id":session_id,"chrome_tab_id":"123","steps":[{"kind":"snapshot"}]})
            .as_object().unwrap().clone(),
    )).await.unwrap().structured_content.unwrap();
    assert_eq!(failed_workflow["status"], "failed");
    assert_eq!(failed_workflow["failed_step"], 0);
    assert_eq!(failed_workflow["receipts"][0]["status"], "failed");
    let missing_open_key = serde_json::json!({"action":"open","mode":"foreground","session_id":session_id,"url":"https://example.test/"});
    let error = client
        .call_tool(
            CallToolRequestParams::new("browser")
                .with_arguments(missing_open_key.as_object().unwrap().clone()),
        )
        .await
        .expect("retry-safe open error is returned as an MCP tool result");
    assert!(
        error.is_error == Some(true)
            && serde_json::to_string(&error)
                .unwrap()
                .contains("requires idempotency_key"),
        "{error:?}"
    );
    let open = serde_json::json!({"action":"open","mode":"foreground","session_id":session_id,"url":"https://example.test/","idempotency_key":"stdio-open-123456"});
    let error = client
        .call_tool(
            CallToolRequestParams::new("browser").with_arguments(open.as_object().unwrap().clone()),
        )
        .await
        .expect("unaccepted shared session error is returned as an MCP tool result");
    assert!(
        error.is_error == Some(true)
            && serde_json::to_string(&error)
                .unwrap()
                .contains("shared extension session has not been accepted"),
        "{error:?}"
    );
    let background_open = serde_json::json!({"action":"open","mode":"background","session_id":session_id,"url":"https://example.test/","idempotency_key":"stdio-bg-open-123456"});
    let error = client
        .call_tool(
            CallToolRequestParams::new("browser")
                .with_arguments(background_open.as_object().unwrap().clone()),
        )
        .await
        .expect("background mode error is returned as an MCP tool result");
    assert!(
        error.is_error == Some(true)
            && serde_json::to_string(&error)
                .unwrap()
                .contains("session was not created in the requested browser mode"),
        "a background open must use a background-paired session: {error:?}"
    );
    let request = serde_json::json!({"action":"task","session_id":session_id,"chrome_tab_id":"123",
        "task":{"kind":"download","reference":"@c1"}});
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser")
                .with_arguments(request.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let receipt = result.structured_content.unwrap();
    assert_eq!(receipt["status"], "unsupported", "{receipt}");
    assert_eq!(receipt["steps"], 1, "{receipt}");
    assert_eq!(receipt["receipts"][0]["verified"], false, "{receipt}");

    let research = serde_json::json!({"action":"task","session_id":"unpaired","chrome_tab_id":"123",
        "task":{"kind":"research","urls":["https://example.test/a","https://example.test/b"],"selector":"article"}});
    let result = client
        .call_tool(
            CallToolRequestParams::new("browser")
                .with_arguments(research.as_object().unwrap().clone()),
        )
        .await
        .expect("unknown session error is returned as an MCP tool result");
    assert!(
        result.is_error == Some(true)
            && serde_json::to_string(&result)
                .unwrap()
                .contains("V3 browser route"),
        "{result:?}"
    );

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
            "browser",
            serde_json::json!({"action":"task","session_id":session_id,"chrome_tab_id":tab_id,
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
            "browser",
            serde_json::json!({"action":"artifact_read","session_id":session_id,"artifact_handle":handle}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    use base64::Engine as _;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(read["bytes_base64"].as_str().unwrap())
            .unwrap(),
        b"fixture"
    );
    assert_eq!(read["sha256"], result["receipts"][0]["sha256"]);
    let registered = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"artifact_register","session_id":session_id,
        "filename":"upload.txt","bytes":[117,112,108,111,97,100]}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(registered["elapsed_ms"].as_u64().is_some(), "{registered}");
    let upload_handle = registered["artifact_handle"].as_str().unwrap();
    let uploaded = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"task","session_id":session_id,"chrome_tab_id":tab_id,
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
    assert!(uploaded["elapsed_ms"].as_u64().is_some(), "{uploaded}");
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
                "<title>Fixture {fact}</title><main>{fact}</main><input aria-label='Name' value='before'><button type='button' onclick=\"document.querySelector('#saved').textContent='Saved'\">Save</button><span id='saved'></span>"
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
    command
        .env("CONTROLLA_STATE_DIR", &state_dir)
        .env("RUST_MIN_STACK", "8388608")
        .env(
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
    let one_call = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"task","mode":"headless","release_after_task":true,
        "task":{"kind":"research","urls":[url_a.clone()],"selector":"main"}}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(one_call["status"], "completed", "{one_call}");
    assert_eq!(
        one_call["receipts"][1]["title"], "Fixture Fact A",
        "{one_call}"
    );
    assert_eq!(one_call["private_session_released"], true, "{one_call}");
    let launch = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"headless","url":url_a}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        launch["next"]
            .as_str()
            .is_some_and(|next| next.contains("action=task")),
        "{launch}"
    );
    assert!(
        !launch["next"]
            .as_str()
            .unwrap_or_default()
            .contains("action=list"),
        "{launch}"
    );
    let session_id = launch["session_id"].as_str().unwrap();
    let tab_id = launch["target_ids"][0].as_str().unwrap();
    let research = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"task","session_id":session_id,"chrome_tab_id":tab_id,
        "max_bytes":16000,"max_bytes_reason":"verify all multi-page research receipts",
        "task":{"kind":"research","urls":[url_a,url_b],"selector":"main"}}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(research["status"], "completed", "{research}");
    assert_eq!(research["receipts"][1]["url"], url_a, "{research}");
    assert_eq!(
        research["receipts"][1]["title"], "Fixture Fact A",
        "{research}"
    );
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
    let draft = client.call_tool(call("browser", serde_json::json!({"action":"task","session_id":session_id,"chrome_tab_id":tab_id,
        "task":{"kind":"form_draft","fields":[{"reference":name,"expected_value":"before","value":"after"}],
            "save_without_submit":true,"save_control":{"reference":save,"confirmation_selector":"#saved","confirmation_text":"Saved"}}})))
        .await.unwrap().structured_content.unwrap();
    assert_eq!(draft["status"], "completed", "{draft}");
    assert_eq!(draft["receipts"][1]["kind"], "save_draft");
    let repeat = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"task","session_id":session_id,"chrome_tab_id":tab_id,"release_after_task":true,
        "task":{"kind":"repeat","references":[name,save],"action":"verify_visible"}}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(repeat["status"], "completed", "{repeat}");
    assert_eq!(repeat["private_session_released"], true, "{repeat}");
    client.cancel().await.unwrap();
    server.abort();
    std::fs::remove_dir_all(state_dir).unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires installed Google Chrome and a private headless browser"]
async fn private_headless_widget_drag_drop_verifies_exact_membership() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request).await;
            let body = r#"<!doctype html><meta charset="utf-8"><title>Drag fixture</title>
<style>#drag,#drop{position:absolute;top:80px;height:80px}#drag{left:20px;width:80px}#drop{left:180px;width:120px}</style>
<div id="drag" draggable="true" aria-label="Drag item"><span>Drag item</span></div><div id="drop" aria-dropeffect="move" aria-label="Drop target"><span>Drop target</span></div><div id="status" aria-live="polite"></div>
<script>const source=document.querySelector('#drag'),target=document.querySelector('#drop');source.addEventListener('dragstart',e=>e.dataTransfer.setData('text/plain','drag'));target.addEventListener('dragover',e=>e.preventDefault());target.addEventListener('drop',e=>{e.preventDefault();document.querySelector('#status').textContent='Dropped';target.appendChild(source)});</script>"#;
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
    let launch = client
        .call_tool(call(
            "browser",
            serde_json::json!({"action":"launch","mode":"headless","url":format!("http://{address}/fixture")}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let session_id = launch["session_id"].as_str().unwrap();
    let tab_id = launch["target_ids"][0].as_str().unwrap();
    let workflow = client
        .call_tool(call(
            "workflow",
            serde_json::json!({
                "session_id":session_id,
                "chrome_tab_id":tab_id,
                "steps":[
                    {"kind":"snapshot","max_items":8},
                    {"kind":"widget","reference":"@c1","value":"Drop target"},
                    {"kind":"verify","predicate":{"kind":"text_present","text":"Drag item"}}
                ]
            }),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(workflow["status"], "completed", "{workflow}");
    assert_eq!(workflow["receipts"][1]["status"], "verified", "{workflow}");
    assert_eq!(workflow["receipts"][2]["status"], "passed", "{workflow}");

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
