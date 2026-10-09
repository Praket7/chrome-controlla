use rmcp::{
    ClientHandler, ServiceExt,
    model::{ClientConfig, Implementation, ProtocolVersion},
    transport::child_process::TokioChildProcess,
};
use std::{collections::BTreeSet, path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};
use tokio::process::Command;

#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("controlla-v2-contract-test", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
    }
}

fn state_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "controlla-v2-contract-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[tokio::test]
async fn compact_v2_advertises_full_guarded_execution_surface() {
    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla-v2"));
    command.env("CONTROLLA_STATE_DIR", &state_dir);
    let transport = TokioChildProcess::new(command).expect("spawn compact v2 MCP server");
    let client = Client.serve(transport).await.expect("initialize compact v2");
    let tools = client.list_tools(None).await.unwrap();
    let names = tools
        .tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        BTreeSet::from([
            "browser_act",
            "browser_extract",
            "browser_find",
            "browser_probe",
            "browser_session",
            "browser_skill",
            "browser_snapshot",
            "browser_verify",
            "browser_workflow",
        ])
    );

    let schema = |name: &str| {
        let tool = tools.tools.iter().find(|tool| tool.name == name).unwrap();
        serde_json::to_value(&tool.input_schema).unwrap()
    };
    let act = schema("browser_act");
    assert!(act["properties"].get("key").is_some());
    let probe = schema("browser_probe");
    for property in ["reference", "selector", "region"] {
        assert!(probe["properties"].get(property).is_some(), "probe {property}");
    }
    let skill = schema("browser_skill");
    for property in [
        "definition",
        "run_id",
        "predicate",
        "expires_at_ms",
        "severe_safety_failure",
    ] {
        assert!(skill["properties"].get(property).is_some(), "skill {property}");
    }
    let workflow_text = serde_json::to_string(&schema("browser_workflow")).unwrap();
    for step in [
        "navigate",
        "find",
        "click",
        "fill",
        "type",
        "press",
        "select",
        "wait_for",
        "observe",
        "extract",
        "assert",
        "verify",
        "checkpoint",
        "script",
    ] {
        assert!(workflow_text.contains(step), "workflow schema contains {step}");
    }

    client.cancel().await.unwrap();
    std::fs::remove_dir_all(state_dir).unwrap();
}
