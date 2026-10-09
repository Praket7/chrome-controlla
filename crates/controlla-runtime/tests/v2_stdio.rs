use rmcp::{
    ClientHandler, ServiceExt,
    model::{ClientConfig, Implementation, ProtocolVersion},
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
async fn compact_v3_advertises_exactly_six_guarded_tools() {
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
        BTreeSet::from(["act", "browser", "extract", "snapshot", "verify", "workflow"])
    );
    assert_eq!(tools.tools.len(), 6);

    let schema = |name: &str| {
        let tool = tools.tools.iter().find(|tool| tool.name == name).unwrap();
        serde_json::to_value(&tool.input_schema).unwrap()
    };
    let browser = schema("browser");
    for property in ["action", "mode", "provider", "session_id", "target_ids"] {
        assert!(browser["properties"].get(property).is_some(), "browser {property}");
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
    for step in ["snapshot", "find", "click", "fill", "type", "press", "extract", "verify"] {
        assert!(workflow_text.contains(step), "workflow schema contains {step}");
    }
    let schema_bytes = serde_json::to_vec(&tools.tools).unwrap().len();
    assert!(schema_bytes <= 48 * 1024, "default tool schema budget is {schema_bytes} bytes");

    client.cancel().await.unwrap();
    std::fs::remove_dir_all(state_dir).unwrap();
}
