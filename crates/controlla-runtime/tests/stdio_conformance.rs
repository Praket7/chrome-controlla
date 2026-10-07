use rmcp::{
    ClientHandler, ServiceExt,
    model::{ClientConfig, Implementation, ProtocolVersion, ReadResourceRequestParams},
    service::{ClientLifecycleMode, ClientServiceExt},
    transport::child_process::TokioChildProcess,
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::process::Command;

#[derive(Clone)]
struct VersionedClient(ProtocolVersion);

impl ClientHandler for VersionedClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("chrome-controlla-stdio-conformance", "0.1.0"),
        )
        .with_protocol_version(self.0.clone())
    }
}

fn state_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "chrome-controlla-stdio-conformance-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[tokio::test]
async fn packaged_stdio_initializes_negotiates_lists_reads_and_reports_errors() {
    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla"));
    command.arg("mcp").env("CONTROLLA_STATE_DIR", &state_dir);
    let transport = TokioChildProcess::new(command).expect("spawn MCP stdio server");
    let client = VersionedClient(ProtocolVersion::LATEST_WITH_INITIALIZE)
        .serve(transport)
        .await
        .expect("SDK initialize handshake");
    assert_eq!(
        client.peer_info().unwrap().protocol_version,
        ProtocolVersion::LATEST_WITH_INITIALIZE,
        "initialize negotiates the newest revision that still defines that handshake"
    );

    let listed_resources = client.list_resources(None).await.unwrap();
    let uris = listed_resources
        .resources
        .iter()
        .map(|resource| resource.uri.as_str())
        .collect::<Vec<_>>();
    let clients_uri = format!("controlla://guide/clients/{}", env!("CARGO_PKG_VERSION"));
    let master_uri = format!("controlla://guide/master/{}", env!("CARGO_PKG_VERSION"));
    assert!(uris.contains(&clients_uri.as_str()));
    assert!(uris.contains(&master_uri.as_str()));
    assert!(
        listed_resources
            .resources
            .iter()
            .all(|resource| { resource.mime_type.as_deref() == Some("text/markdown") })
    );

    let clients = client
        .read_resource(ReadResourceRequestParams::new(&clients_uri))
        .await
        .unwrap();
    assert!(matches!(
        clients.contents.first(),
        Some(rmcp::model::ResourceContents::TextResourceContents { text, mime_type: Some(mime), .. })
            if mime == "text/markdown" && text.contains("OpenCode v2")
    ));

    let master = client
        .read_resource(ReadResourceRequestParams::new(&master_uri))
        .await
        .unwrap();
    assert!(!master.contents.is_empty());
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new(format!(
                "controlla://guide/clients/{}.stale",
                env!("CARGO_PKG_VERSION")
            )))
            .await
            .is_err()
    );

    let tools = client.list_tools(None).await.unwrap();
    let guide = tools
        .tools
        .iter()
        .find(|tool| tool.name == "guide")
        .expect("guide tool remains available");
    let schema = serde_json::to_value(&guide.input_schema).unwrap();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"].get("topic").is_some());
    assert!(schema["properties"].get("server_version").is_some());
    assert_eq!(schema["required"], json!(["topic", "server_version"]));
    for tool in &tools.tools {
        let schema = serde_json::to_value(&tool.input_schema).unwrap();
        assert_eq!(schema["type"], "object", "{} schema root", tool.name);
        assert!(schema["properties"].is_object(), "{} schema", tool.name);
    }
    assert!(
        client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("guide").with_arguments(
                    json!({"topic":"unknown","server_version":env!("CARGO_PKG_VERSION")})
                        .as_object()
                        .unwrap()
                        .clone()
                )
            )
            .await
            .is_err()
    );

    client.cancel().await.unwrap();
    std::fs::remove_dir_all(state_dir).unwrap();
}

#[tokio::test]
async fn packaged_stdio_supports_discovery_lifecycle_without_initialize() {
    let state_dir = state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_controlla"));
    command.arg("mcp").env("CONTROLLA_STATE_DIR", &state_dir);
    let transport = TokioChildProcess::new(command).expect("spawn MCP stdio server");
    let client = VersionedClient(ProtocolVersion::LATEST)
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::LATEST],
            },
        )
        .await
        .expect("discovery lifecycle handshake");

    assert_eq!(
        client.peer_info().unwrap().protocol_version,
        ProtocolVersion::LATEST
    );
    let tools = client.list_tools(None).await.unwrap();
    assert!(tools.tools.iter().any(|tool| tool.name == "session"));
    assert!(tools.tools.iter().any(|tool| tool.name == "guide"));
    let resources = client.list_resources(None).await.unwrap();
    assert!(resources.resources.iter().any(|resource| {
        resource.uri == format!("controlla://guide/master/{}", env!("CARGO_PKG_VERSION"))
    }));

    client.cancel().await.unwrap();
    std::fs::remove_dir_all(state_dir).unwrap();
}
