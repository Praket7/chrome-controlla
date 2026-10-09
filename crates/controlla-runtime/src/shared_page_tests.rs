use super::*;
use controlla_browser::providers::DedicatedChromeProvider;
use controlla_browser::sessions::{CleanupObservation, IndependentTargetObserver};
use futures_util::{FutureExt, SinkExt, StreamExt};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use std::{panic::AssertUnwindSafe, path::Path};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
#[ignore = "requires installed Chrome; disposable profile with real CDP behind a test extension transport"]
async fn shared_snapshot_and_click_real_chrome_regressions() {
    let executable = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    assert!(executable.exists());
    let mut registry = SessionRegistry::new(ProviderGrants {
        dedicated_headless: true,
        ..Default::default()
    });
    let handle = registry
        .create_session(
            SessionSpec {
                mode: SessionMode::Headless,
                selected_target_ids: vec![],
            },
            "shared-page-fixture",
        )
        .unwrap();
    let browser = DedicatedChromeProvider::new(executable)
        .launch(&mut registry, &handle, "about:blank")
        .await
        .unwrap();
    let connection = browser.connection().clone();
    let target = browser.target_id().to_owned();
    let eval = |script: String| {
        let connection = connection.clone();
        let target = target.clone();
        async move {
            let (_, targets) = connection.target_snapshot().await;
            let t = targets.iter().find(|t| t.id == target).unwrap();
            connection
                .target_command(
                    &target,
                    t.generation,
                    &t.revision,
                    "Runtime.evaluate",
                    json!({"expression":script,"returnByValue":true}),
                )
                .await
                .unwrap()
        }
    };
    let result=AssertUnwindSafe(async {
        eval(r##"document.body.innerHTML='<style>button,a{display:block;margin:4px}</style>'+ '<div>padding</div>'.repeat(200)+'<button id="account" aria-label="Accounts" aria-expanded="false" onclick="this.setAttribute(\'aria-expanded\',\'true\')"></button><button id="menu" onclick="setTimeout(()=>{document.body.insertAdjacentHTML(\'beforeend\',\'<div id=late>Ready</div>\')},150)">Menu</button><button id="uncertain" onclick="window.clicks=(window.clicks||0)+1">Count</button><a id="nav" href="#done">Story</a><div data-masked>PRIVATE-SENTINEL</div><button><span data-requires-trusted>PROTECTED-SENTINEL</span></button><span id="secretLabel" data-masked>LABEL-SENTINEL</span><button aria-labelledby="secretLabel" aria-label="Safe">safe</button><button hidden>Hidden</button><div id="late-target">needle</div>';document.querySelector('#account').style.cssText='width:70px;height:25px';"##.into()).await;
        let spec=ObserveSpec{selector:"#late-target".into(),fields:BTreeMap::from([("text".into(),"#late-target".into())]),max_items:1,max_text_chars:100,max_bytes:4096,cursor:None};
        let command=observation_command(&spec).unwrap();
        let observed=eval(command["expression"].as_str().unwrap().into()).await;
        assert_eq!(observed["result"]["value"]["items"][0]["text"],"needle");
        assert_eq!(observed["result"]["value"]["limited"],false);
        let (server_io,client_io)=tokio::io::duplex(65536);
        let server=rmcp::service::serve_directly::<rmcp::RoleServer,_,_,_,_>(App::default(),server_io,None);
        let server_task=tokio::spawn(async move {let _=server.waiting().await;});
        let client=().serve(client_io).await.unwrap();
        let call=|name:&str,value:Value| CallToolRequestParams::new(name.to_owned()).with_arguments(value.as_object().unwrap().clone());
        let pair=client.call_tool(call("session",json!({"action":"pair_shared","target_ids":["123"]}))).await.unwrap().structured_content.unwrap();
        let session=pair["session_id"].as_str().unwrap().to_owned();
        let (mut socket,_)=connect_async(pair["endpoint"].as_str().unwrap()).await.unwrap();
        socket.send(Message::Text(json!({"type":"hello","token":pair["one_session_token"],"extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"targets":["123"]}).to_string().into())).await.unwrap();
        let _=socket.next().await.unwrap();
        let c=connection.clone(); let t=target.clone();
        let proxy=tokio::spawn(async move {
            while let Some(Ok(message))=socket.next().await {
                let request:Value=serde_json::from_str(message.to_text().unwrap()).unwrap();
                let Some(method)=request["method"].as_str() else {continue};
                let (_,targets)=c.target_snapshot().await;
                let target=targets.iter().find(|v|v.id==t).unwrap();
                let result=c.target_command(&t,target.generation,&target.revision,method,request["params"].clone()).await;
                let response=match result {Ok(value)=>json!({"type":"result","id":request["id"],"result":value}),Err(e)=>json!({"type":"result","id":request["id"],"error":e.to_string()})};
                if socket.send(Message::Text(response.to_string().into())).await.is_err(){break;}
            }
        });
        let accepted=client.call_tool(call("session",json!({"action":"accept_shared","session_id":session}))).await.unwrap();
        assert_eq!(accepted.structured_content.unwrap()["accepted"],true);
        let snapshot=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","max_text_chars":6000,"max_bytes":60000}))).await.unwrap().structured_content.unwrap();
        let rendered=snapshot.to_string();
        for secret in ["PRIVATE-SENTINEL","PROTECTED-SENTINEL","LABEL-SENTINEL"] {assert!(!rendered.contains(secret));}
        let item=|snapshot:&Value,name:&str|snapshot["items"].as_array().unwrap().iter().find(|v|v["name"]==name).unwrap().clone();
        assert!(!snapshot["items"].as_array().unwrap().iter().any(|v|v["name"]=="Hidden"));
        assert_eq!(item(&snapshot,"Accounts")["raw_value"],"");
        let first_ref=item(&snapshot,"Accounts")["reference"].clone();
        let clicked=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":first_ref,"outcome":{"kind":"expanded"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(clicked["status"],"verified","{clicked}");
        let duplicate=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":first_ref,"outcome":{"kind":"expanded"}}))).await;
        assert!(duplicate.is_err() || duplicate.unwrap().is_error==Some(true));
        let menu=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&clicked["snapshot"],"Menu")["reference"],"outcome":{"kind":"text","selector":"#late","text":"Ready"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(menu["status"],"verified","{menu}");
        let unknown=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&menu["snapshot"],"Count")["reference"],"outcome":{"kind":"visible","selector":"#never"},"timeout_ms":1000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(unknown["status"],"unknown");
        assert_eq!(eval("window.clicks".into()).await["result"]["value"],1);
        let count_ref=item(&unknown["snapshot"],"Count")["reference"].clone();
        eval("const old=document.querySelector('#uncertain');old.replaceWith(old.cloneNode(true))".into()).await;
        let stale=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":count_ref,"outcome":{"kind":"visible","selector":"#never"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(stale["status"],"not_dispatched");
        let nav=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&unknown["snapshot"],"Story")["reference"],"outcome":{"kind":"navigation"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(nav["status"],"verified","{nav}");
        assert_eq!(nav["navigation_observed"],true);
        assert!(nav["url"].as_str().unwrap().ends_with("#done"));
        proxy.abort(); let _=client.cancel().await; server_task.abort();
    }).catch_unwind().await;
    struct Observer;
    impl IndependentTargetObserver for Observer {
        fn verify_unchanged(&self, _: &CleanupObservation) -> Result<(), String> {
            Ok(())
        }
    }
    let cleanup = browser.shutdown(&mut registry, Some(&Observer)).await;
    assert!(
        cleanup.cleanup_error.is_none(),
        "{:?}",
        cleanup.cleanup_error
    );
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
