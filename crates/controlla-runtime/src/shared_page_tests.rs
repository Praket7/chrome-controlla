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
    run_shared_page_fixture(false).await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; uses a disposable profile and read-only test bridge"]
async fn shared_preflight_batches_identity_and_epoch_real_chrome() {
    run_shared_page_fixture(true).await;
}

async fn run_shared_page_fixture(preflight_only: bool) {
    let configured = std::env::var("CONTROLLA_CHROME")
        .unwrap_or_else(|_| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    let executable = Path::new(&configured);
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
                    json!({"expression":script,"returnByValue":true,"awaitPromise":true}),
                )
                .await
                .unwrap()
        }
    };
    let result=AssertUnwindSafe(async {
        eval(r##"document.body.innerHTML='<style>button,a{display:block;margin:4px}</style>'+ '<div>padding</div>'.repeat(200)+'<button id="account" aria-label="Accounts" aria-expanded="false" onclick="this.setAttribute(\'aria-expanded\',\'true\')"></button><button aria-label="Twin">Twin</button><button aria-label="Twin">Twin</button><button id="menu" onclick="setTimeout(()=>{document.body.insertAdjacentHTML(\'beforeend\',\'<div id=late>Ready</div>\')},150)">Menu</button><button id="uncertain" onclick="window.clicks=(window.clicks||0)+1">Count</button><a id="nav" href="#done">Story</a><input id="trusted" aria-label="Trusted keys" type="text" data-requires-trusted value="before" onclick="window.trustedClicks=(window.trustedClicks||0)+1"><div data-masked>PRIVATE-SENTINEL</div><button><span data-requires-trusted>PROTECTED-SENTINEL</span></button><span id="secretLabel" data-masked>LABEL-SENTINEL</span><button aria-labelledby="secretLabel" aria-label="Safe">safe</button><button hidden>Hidden</button><div id="late-target">needle</div><iframe id="composed-text" srcdoc="&lt;body&gt;&lt;label&gt;COMPOSED-FRAME-SENTINEL&lt;/label&gt;&lt;/body&gt;"></iframe>';document.querySelector('#account').style.cssText='width:70px;height:25px';"##.into()).await;
        eval(r##"document.body.insertAdjacentHTML('beforeend','<article><a href="/story-date">Date precision</a><time datetime="2024-04-01">April 1</time></article><article><a href="/story-instant">Instant precision</a><time datetime="2024-04-02T12:00:00Z">April 2</time></article><article><a target="_blank" rel="noopener" href="/story-new-tab">New tab story</a></article><article><a href="/story-relative">Fresh Bills story</a><time>2h</time><span>Nick Suss</span></article>');"##.into()).await;
        eval(r##"document.body.insertAdjacentHTML('beforeend','<button id="aria-upload" aria-label="ARIA Upload" aria-disabled="true" onclick="document.body.insertAdjacentHTML(\'beforeend\',\'<div id=upload-opened>Upload picker</div>\')">Upload</button>');"##.into()).await;
        eval("document.querySelector('#aria-upload').style.cssText='position:fixed;bottom:0';".into()).await;
        let linked_dates = include_str!("shared_page/linked_dates.js");
        let stale_revision = eval(format!("({linked_dates})([],{{url:location.href,revision:'stale'}})")).await;
        assert_eq!(stale_revision["result"]["value"]["snapshot_matches"], false, "same-URL reload must invalidate a prior candidate snapshot");
        let current_revision = eval(format!("({linked_dates})([],{{url:location.href,revision:String(performance.timeOrigin)}})")).await;
        assert_eq!(current_revision["result"]["value"]["snapshot_matches"], true, "the current document revision should pass the metadata guard");
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
        socket.send(Message::Text(json!({"type":"hello","token":pair["one_session_token"],"extension_version":env!("CARGO_PKG_VERSION"),"document_identity":true,"batch_execution":true,"batch_deadline":true,"targets":["123"]}).to_string().into())).await.unwrap();
        let _=socket.next().await.unwrap();
        let c=connection.clone(); let t=target.clone();
        let host_round_trips=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let trips=host_round_trips.clone();
        let command_trace=std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let trace=command_trace.clone();
        let lost_release_ack=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let lose_ack=lost_release_ack.clone();
        let proxy=tokio::spawn(async move {
            while let Some(Ok(message))=socket.next().await {
                let request:Value=serde_json::from_str(message.to_text().unwrap()).unwrap();
                trips.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                let actions=if request["type"]=="batch" {request["actions"].as_array().unwrap().clone()} else {vec![request.clone()]};
                trace.lock().unwrap().push(json!({
                    "request_type":request["type"],
                    "methods":actions.iter().map(|action|action["method"].clone()).collect::<Vec<_>>()
                }));
                let mut receipts=Vec::new();
                for (index,action) in actions.iter().enumerate() {
                    let (_,targets)=c.target_snapshot().await;
                    let target=targets.iter().find(|v|v.id==t).unwrap();
                    let result=c.target_command(&t,target.generation,&target.revision,action["method"].as_str().unwrap(),action["params"].clone()).await;
                    receipts.push(match result {Ok(value)=>json!({"index":index,"method":action["method"],"result":value}),Err(e)=>json!({"index":index,"method":action["method"],"error":e.to_string()})});
                    if receipts.last().unwrap().get("error").is_some(){break;}
                }
                let response=if request["type"]=="batch" {json!({"type":"batch_result","id":request["id"],"result":{"completed":receipts.len(),"receipts":receipts,"host_round_trips":1}})} else {
                    let receipt=&receipts[0];
                    if let Some(error)=receipt.get("error") {json!({"type":"result","id":request["id"],"error":error})}
                    else if request["method"]=="Input.dispatchMouseEvent" && request["params"]["type"]=="mouseReleased" && lose_ack.swap(false,std::sync::atomic::Ordering::Relaxed) {json!({"type":"result","id":request["id"],"error":"simulated lost release acknowledgement"})}
                    else {json!({"type":"result","id":request["id"],"result":receipt["result"]})}
                };
                if socket.send(Message::Text(response.to_string().into())).await.is_err(){break;}
            }
        });
        let accepted=client.call_tool(call("session",json!({"action":"accept_shared","session_id":session}))).await.unwrap();
        assert_eq!(accepted.structured_content.unwrap()["accepted"],true);
        let mut snapshot=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","max_text_chars":6000,"max_bytes":60000}))).await.unwrap().structured_content.unwrap();
        assert!(snapshot["text"].as_str().unwrap_or_default().contains("COMPOSED-FRAME-SENTINEL"), "text walker omitted text in a loaded same-origin frame: {snapshot}");
        let root_control=snapshot["items"].as_array().unwrap().iter().find(|item|item["name"]=="Accounts").unwrap();
        assert!(root_control["bounds"]["width"].as_f64().is_some_and(|width|width>0.0),"root semantic controls need exact page bounds for targeted region grounding: {root_control}");
        let date_precision = snapshot["items"].as_array().unwrap().iter().find(|item| item["name"] == "Date precision").unwrap();
        assert_eq!(date_precision["datetime_precision"], "date");
        assert!(date_precision["datetime_utc_ms"].is_null());
        assert!(date_precision["freshness_upper_bound_ms"].as_i64().is_some());
        assert_eq!(date_precision["freshness_upper_bound_ms"], 1_712_059_199_999_i64, "timezone-free dates must include the UTC-12 local-day boundary");
        let instant_precision = snapshot["items"].as_array().unwrap().iter().find(|item| item["name"] == "Instant precision").unwrap();
        assert_eq!(instant_precision["datetime_precision"], "instant");
        assert!(instant_precision["datetime_utc_ms"].as_i64().is_some());
        let new_tab_story = snapshot["items"].as_array().unwrap().iter().find(|item| item["name"] == "New tab story").unwrap();
        assert_eq!(new_tab_story["target"], "_blank");
        assert_eq!(new_tab_story["rel"], "noopener");
        assert_eq!(host_round_trips.load(std::sync::atomic::Ordering::Relaxed),3,"fresh snapshot uses one identity read, one evaluation, and one readback batch");
        let freshness_goal=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Bills story","roles":["link"],"href_hint":"/story-relative"},"max_items":10,"max_text_chars":0,"max_bytes":60000}))).await.unwrap().structured_content.unwrap();
        let fresh_story=freshness_goal["items"].as_array().unwrap().iter().find(|item|item["href"].as_str().is_some_and(|href|href.ends_with("/story-relative"))).unwrap();
        assert_eq!(fresh_story["freshness_text"],"2h","relative publication evidence must be extracted from the story card");
        assert!(fresh_story["freshness_observed_at_ms"].as_i64().is_some());
        let goal_snapshot=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Accounts","roles":["button"]},"max_items":1,"max_text_chars":6000,"max_bytes":60000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(goal_snapshot["goal_satisfied"],true,"{goal_snapshot}");
        assert!(goal_snapshot["text"].as_str().unwrap_or_default().contains("COMPOSED-FRAME-SENTINEL"),"goal snapshots must collect bounded text from the same-origin frame scopes: {goal_snapshot}");
        assert_eq!(goal_snapshot["completeness"]["text"],"complete","text completeness is independent of targeted control completeness");
        let fast_twin=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Twin","roles":["button"]},"max_items":1,"max_text_chars":0,"max_bytes":60000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(fast_twin["items"][0]["target_uniqueness"],"unverified","early stop must not imply a unique target");
        let exhaustive_twins=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Twin","roles":["button"],"exhaustive":true},"max_items":10,"max_text_chars":0,"max_bytes":60000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(exhaustive_twins["scope_complete"],true,"{exhaustive_twins}");
        assert_eq!(
            exhaustive_twins["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["name"] == "Twin")
                .count(),
            2,
            "exhaustive intent ranking must retain both exact duplicate labels without treating the query as a hard filter"
        );
        snapshot=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","max_text_chars":6000,"max_bytes":60000}))).await.unwrap().structured_content.unwrap();
        let rendered=snapshot.to_string();
        for secret in ["PRIVATE-SENTINEL","PROTECTED-SENTINEL","LABEL-SENTINEL"] {assert!(!rendered.contains(secret));}
        let item=|snapshot:&Value,name:&str|snapshot["items"].as_array().unwrap().iter().find(|v|v["name"]==name).unwrap().clone();
        let mut aria_upload=item(&snapshot,"ARIA Upload");
        assert_eq!(aria_upload["disabled"],false,"ARIA-only disabled state must not be reported as a browser-native disabled control");
        assert_eq!(aria_upload["native_disabled"],false);
        assert_eq!(aria_upload["aria_disabled"],true);
        assert_eq!(aria_upload["actionability"],"candidate");
        assert_eq!(item(&snapshot,"Trusted keys")["requires_trusted_events"],true);
        assert!(!snapshot["items"].as_array().unwrap().iter().any(|v|v["name"]=="Hidden"));
        assert_eq!(item(&snapshot,"Accounts")["raw_value"],"");
        let before_guard_scroll=eval("scrollY".into()).await["result"]["value"].as_i64().unwrap();
        let preflight_trace_start=command_trace.lock().unwrap().len();
        let wrong_outcome=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&snapshot,"Trusted keys")["reference"],"outcome":{"kind":"navigation"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(wrong_outcome["status"],"not_dispatched","{wrong_outcome}");
        let preflight_commands=command_trace.lock().unwrap()[preflight_trace_start..].to_vec();
        assert_eq!(preflight_commands[0]["request_type"], "batch", "identity and interaction epoch must share a read-only bridge request");
        assert_eq!(preflight_commands[0]["methods"], json!(["Runtime.callFunctionOn", "Page.getFrameTree"]));
        assert_eq!(eval("window.trustedClicks||0".into()).await["result"]["value"],0);
        assert_eq!(eval("scrollY".into()).await["result"]["value"].as_i64().unwrap(),before_guard_scroll,"a denied trusted-event action must not move the page");
        if preflight_only { return; }
        let trusted=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&snapshot,"Trusted keys")["reference"],"outcome":{"kind":"focused"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(trusted["status"],"verified","{trusted}");
        assert_eq!(eval("document.activeElement.id".into()).await["result"]["value"],"trusted");
        snapshot=trusted["snapshot"].clone();
        let first_ref=item(&snapshot,"Accounts")["reference"].clone();
        let click_start=host_round_trips.load(std::sync::atomic::Ordering::Relaxed);
        let click_trace_start=command_trace.lock().unwrap().len();
        let clicked=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":first_ref,"outcome":{"kind":"expanded"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(clicked["status"],"verified","{clicked}");
        let click_round_trips=host_round_trips.load(std::sync::atomic::Ordering::Relaxed)-click_start;
        let click_commands=command_trace.lock().unwrap()[click_trace_start..].to_vec();
        assert_eq!(click_commands[0]["request_type"], "batch", "identity and interaction epoch must share a read-only bridge request");
        assert_eq!(click_commands[0]["methods"], json!(["Runtime.callFunctionOn", "Page.getFrameTree"]));
        eprintln!("S3-04 warm click/readback fixture: {click_round_trips} native bridge round trips; command groups {click_commands:?}");
        assert!(click_round_trips<=6,"warm click/readback exceeded the proposed six native bridge round trips: {click_commands:?}");
        snapshot=clicked["snapshot"].clone();
        aria_upload=item(&snapshot,"ARIA Upload");
        let aria_upload_click=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":aria_upload["reference"],"outcome":{"kind":"visible","selector":"#upload-opened"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(aria_upload_click["status"],"verified","a working upload-picker button with a stale aria-disabled annotation must still dispatch and verify: {aria_upload_click}");
        let duplicate=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":first_ref,"outcome":{"kind":"expanded"}}))).await;
        assert!(duplicate.is_err() || duplicate.unwrap().is_error==Some(true));
        let menu=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&aria_upload_click["snapshot"],"Menu")["reference"],"outcome":{"kind":"text","selector":"#late","text":"Ready"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(menu["status"],"verified","{menu}");
        let unknown=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&menu["snapshot"],"Count")["reference"],"outcome":{"kind":"visible","selector":"#never"},"timeout_ms":1000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(unknown["status"],"unknown");
        assert_eq!(eval("window.clicks".into()).await["result"]["value"],1);
        let count_ref=item(&unknown["snapshot"],"Count")["reference"].clone();
        eval("const old=document.querySelector('#uncertain');old.replaceWith(old.cloneNode(true))".into()).await;
        let stale=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":count_ref,"outcome":{"kind":"visible","selector":"#never"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(stale["status"], "unknown", "the unique same-document replacement should be reacquired and clicked: {stale}");
        assert_eq!(eval("window.clicks".into()).await["result"]["value"], 2);
        lost_release_ack.store(true,std::sync::atomic::Ordering::Relaxed);
        let navigation_start=host_round_trips.load(std::sync::atomic::Ordering::Relaxed);
        let navigation_trace_start=command_trace.lock().unwrap().len();
        let nav=client.call_tool(call("shared_click",json!({"session_id":session,"chrome_tab_id":"123","reference":item(&stale["snapshot"],"Story")["reference"],"outcome":{"kind":"navigation"}}))).await.unwrap().structured_content.unwrap();
        assert_eq!(nav["status"],"verified","{nav}");
        assert_eq!(nav["dispatch_acknowledged"],false,"lost release acknowledgement must remain visible");
        assert_eq!(nav["navigation_observed"],true);
        assert!(nav["url"].as_str().unwrap().ends_with("#done"));
        let extracted=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Story","roles":["link"]},"max_items":3,"max_text_chars":0,"max_bytes":8192}))).await.unwrap().structured_content.unwrap();
        assert_eq!(extracted["goal_satisfied"],true,"navigation follow-up evidence must be extracted from the expected story control: {extracted}");
        let navigation_round_trips=host_round_trips.load(std::sync::atomic::Ordering::Relaxed)-navigation_start;
        let navigation_commands=command_trace.lock().unwrap()[navigation_trace_start..].to_vec();
        eprintln!("S3-04 navigation plus extraction fixture: {navigation_round_trips} native bridge round trips; command groups {navigation_commands:?}");
        assert!(navigation_round_trips<=12,"navigation plus extraction exceeded the proposed twelve native bridge round trips: {navigation_commands:?}");
        eval("document.body.replaceChildren(Object.assign(document.createElement('button'),{textContent:'Open requested panel'}),...Array.from({length:50000},(_,i)=>Object.assign(document.createElement('div'),{textContent:'dashboard cell '+i})))".into()).await;
        let targeted=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Open requested panel","roles":["button"]},"max_scan_nodes":50000,"max_items":1,"max_text_chars":100,"max_bytes":8192}))).await.unwrap().structured_content.unwrap();
        assert_eq!(targeted["goal_satisfied"],true,"{targeted}");
        assert_eq!(targeted["global_complete"],false,"an early target match must not be reported as exhaustive");
        assert_eq!(targeted["scope_complete"],false,"early stop cannot prove uniqueness or exhaustive target scope");
        assert_eq!(targeted["items"][0]["target_uniqueness"],"unverified","fast top-one candidate is not claimed unique");
        assert_eq!(targeted["truncation_reason"],"ranked_top_one","{targeted}");
        assert!(targeted["scanned_nodes"].as_u64().unwrap()<100,"targeted scan traversed too much of the 50k-node fixture: {targeted}");
        let partial=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Open","roles":["button"]},"max_scan_nodes":3,"max_items":1,"max_text_chars":100,"max_bytes":8192}))).await.unwrap().structured_content.unwrap();
        assert_eq!(partial["goal_satisfied"],false,"partial text plus a role bonus must not early-stop: {partial}");
        assert_eq!(partial["scanned_nodes"],3,"scan ceiling must not visit an extra node: {partial}");
        assert_eq!(partial["search_budget_exhausted"],true);
        assert_eq!(targeted["items"].as_array().unwrap().len(),1);
        eval("document.body.replaceChildren(Object.assign(document.createElement('button'),{textContent:'Open requested panel'}),...Array.from({length:50000},()=>{const node=document.createElement('div');node.append(document.createTextNode(''));return node}))".into()).await;
        let sparse_text=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Open requested panel","roles":["button"]},"max_scan_nodes":50000,"max_items":1,"max_text_chars":100,"max_bytes":8192}))).await.unwrap().structured_content.unwrap();
        assert_eq!(sparse_text["goal_satisfied"],true,"{sparse_text}");
        assert_eq!(sparse_text["completeness"]["text"],"partial","bounded sparse-text traversal must be reported as incomplete");
        assert!(sparse_text["scanned_text_nodes"].as_u64().unwrap()<=4096,"targeted snapshot text traversal exceeded its separate budget: {sparse_text}");
        eval("document.body.replaceChildren(Object.assign(document.createElement('button'),{textContent:'edit New announcement'}))".into()).await;
        let weak_exhaustive_match=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"announcement editor textbox attach file Post button","exhaustive":true},"max_scan_nodes":50000,"max_items":20,"max_text_chars":0,"max_bytes":20000}))).await.unwrap().structured_content.unwrap();
        assert_eq!(weak_exhaustive_match["goal_satisfied"],false,"one weak token overlap must not satisfy an exhaustive semantic goal: {weak_exhaustive_match}");
        assert_eq!(weak_exhaustive_match["items"][0]["name"],"edit New announcement","fixture must retain the tempting weak candidate");
        let no_text=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","goal":{"query":"Open requested panel","roles":["button"]},"max_scan_nodes":50000,"max_items":1,"max_text_chars":0,"max_bytes":8192}))).await.unwrap().structured_content.unwrap();
        assert_eq!(no_text["scanned_text_nodes"],0,"zero requested text must skip text traversal");
        assert_eq!(no_text["text_requested"],false,"control-only snapshots must state that page text was omitted");
        assert_eq!(no_text["completeness"]["text"],"partial","unread text must never be called complete");
        eval("document.body.insertAdjacentHTML('beforeend','<section><h2 data-masked>HEADING-SENTINEL</h2><a href=\"/story/visible\">Visible story</a></section>')".into()).await;
        let scoped=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","href_hint":"/story/"}))).await.unwrap().structured_content.unwrap();
        assert!(!scoped.to_string().contains("HEADING-SENTINEL"),"masked group heading leaked");
        eval("document.body.innerHTML='<span>FIRST-TEXT</span><span>SECOND-TEXT</span>'".into()).await;
        let separated=client.call_tool(call("shared_snapshot",json!({"session_id":session,"chrome_tab_id":"123","max_text_chars":100,"max_bytes":8192}))).await.unwrap().structured_content.unwrap();
        assert!(separated["text"].as_str().unwrap().contains("FIRST-TEXT\nSECOND-TEXT"),"separate text nodes should be separated by an actual newline: {separated}");
        assert!(!separated["text"].as_str().unwrap().contains("FIRST-TEXT\\\\nSECOND-TEXT"),"text must not contain a literal backslash-n separator: {separated}");
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
