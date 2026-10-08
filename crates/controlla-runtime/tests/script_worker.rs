use controlla_runtime::workflow::{ScriptBroker, run_script_isolated};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

fn broker() -> ScriptBroker {
    Arc::new(|raw| {
        Box::pin(async move {
            let spec: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            assert_eq!(spec["selector"], "p");
            Ok(r#"{"value":"parent-broker"}"#.to_owned())
        }) as Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
    })
}

fn worker() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_BIN_EXE_controlla"))
}

#[tokio::test]
async fn child_worker_brokers_reads_kills_on_deadline_and_recovers_after_exit() {
    let output = run_script_isolated(
        worker(),
        "const ambient=[typeof process,typeof fetch,typeof Deno,typeof require,typeof api]; let moduleAccess='blocked'; try{await import('node:fs');moduleAccess='available'}catch{} const r=await api.observe({selector:'p',fields:{text:'p'},max_items:1,max_text_chars:32,max_bytes:4096,cursor:null}); return {ambient:ambient.join(','),moduleAccess,value:r.value};",
        2_000,
        8 * 1024 * 1024,
        broker(),
    )
    .await
    .unwrap();
    assert_eq!(
        output,
        r#"{"ambient":"undefined,undefined,undefined,undefined,object","moduleAccess":"blocked","value":"parent-broker"}"#
    );

    assert!(
        run_script_isolated(
            worker(),
            "return 'x'.repeat(4_000_000);",
            2_000,
            2 * 1024 * 1024,
            broker(),
        )
        .await
        .is_err()
    );
    assert!(
        run_script_isolated(
            worker(),
            "return 'x'.repeat(70_000);",
            2_000,
            8 * 1024 * 1024,
            broker(),
        )
        .await
        .is_err()
    );

    let timed_out = tokio::time::timeout(
        Duration::from_secs(3),
        run_script_isolated(worker(), "while(true){}", 100, 8 * 1024 * 1024, broker()),
    )
    .await
    .expect("parent deadline must bound a wedged worker")
    .unwrap_err();
    assert!(timed_out.contains("deadline"));

    let crashed_worker = std::env::current_exe().unwrap();
    assert!(
        run_script_isolated(&crashed_worker, "return 1;", 500, 8 * 1024 * 1024, broker())
            .await
            .is_err()
    );

    let restarted = run_script_isolated(worker(), "return 1;", 2_000, 8 * 1024 * 1024, broker())
        .await
        .unwrap();
    assert_eq!(restarted, "1");
}
