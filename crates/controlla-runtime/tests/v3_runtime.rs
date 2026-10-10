use controlla_runtime::v3::{BrowserMode, InteractionEpoch, RecoveryClass, TypingMode};
use controlla_runtime::v3_runtime::{
    BatchAction, BatchActionKind, BrowserLaunchPlan, ClientKind, ClientQualification,
    DEFAULT_AGENT_TOOLS, PageToolAuthority, PageToolCall, PageToolProgram, ReconnectController,
    ReconnectDecision, TypingPlan, await_key_batch, compact_tool_surface_valid,
    execute_guarded_batch, result_within_budget,
};
use serde_json::json;
use std::collections::BTreeSet;

fn epoch() -> InteractionEpoch {
    InteractionEpoch {
        browser_generation: 1,
        target_id: "tab-1".into(),
        target_revision: "target-1".into(),
        frame_id: "frame-1".into(),
        loader_id: "loader-1".into(),
        document_id: "doc-1".into(),
        focused_backend_node_id: Some(10),
        selection_fingerprint: Some("10:0:0".into()),
        semantic_revision: "sem-1".into(),
        viewport_revision: 1,
    }
}

#[test]
fn browser_modes_produce_explicit_launch_plans() {
    let foreground = BrowserLaunchPlan::for_mode(BrowserMode::Foreground);
    let background = BrowserLaunchPlan::for_mode(BrowserMode::Background);
    let headless = BrowserLaunchPlan::for_mode(BrowserMode::Headless);
    assert!(foreground.may_activate_window);
    assert!(!background.may_activate_window);
    assert!(background.chrome_args.contains(&"--start-minimized"));
    assert!(headless.chrome_args.contains(&"--headless=new"));
    assert_eq!(headless.provider, "dedicated_headless");
}

#[tokio::test]
async fn key_batch_waiting_stops_at_the_callers_deadline() {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1);
    let result = await_key_batch(
        deadline,
        tokio::time::sleep(std::time::Duration::from_millis(20)),
    )
    .await;
    assert!(
        result.is_err(),
        "an expired batch deadline must stop waiting for delivery"
    );
}

#[test]
fn guarded_batch_is_one_host_round_trip_and_stops_before_stale_mutation() {
    let grounded = epoch();
    let actions = vec![
        BatchAction {
            id: "read".into(),
            kind: BatchActionKind::Read,
        },
        BatchAction {
            id: "fill".into(),
            kind: BatchActionKind::TextMutation,
        },
        BatchAction {
            id: "click".into(),
            kind: BatchActionKind::PointerMutation,
        },
    ];
    let mut stale = grounded.clone();
    stale.selection_fingerprint = Some("10:1:1".into());
    let receipt = execute_guarded_batch(
        &grounded,
        &[grounded.clone(), stale, grounded.clone()],
        &actions,
    )
    .unwrap();
    assert_eq!(receipt.host_round_trips, 1);
    assert_eq!(receipt.completed, vec!["read"]);
    assert_eq!(receipt.stopped_before.as_deref(), Some("fill"));
    assert!(receipt.stale_transition);
}

#[test]
fn block_typing_is_constant_protocol_work_and_fast_keys_have_no_delay_floor() {
    let payload = "a".repeat(1_000);
    let block = TypingPlan::build(&payload, TypingMode::Block, None).unwrap();
    assert_eq!(block.protocol_batches, 1);
    assert_eq!(block.events.len(), 1);

    let fast = TypingPlan::build(&payload, TypingMode::FastKeys, Some(60)).unwrap();
    assert_eq!(fast.policy.delay_ms, 0);
    assert!(fast.protocol_batches <= 63);
    assert!(fast.events.iter().all(|event| event.delay_after_ms == 0));
    assert!(fast.requires_focus_revalidation);

    let human = TypingPlan::build("abc", TypingMode::HumanKeys, Some(24)).unwrap();
    assert_eq!(human.policy.delay_ms, 24);
    assert_eq!(human.protocol_batches, 3);
}

#[test]
fn ime_typing_uses_composition_events() {
    let plan = TypingPlan::build("こんにちは", TypingMode::Ime, None).unwrap();
    assert_eq!(plan.protocol_batches, 1);
    assert_eq!(plan.events.len(), 4);
    assert_eq!(plan.events[1].text, "こんにちは");
}

#[test]
fn page_tools_are_allowlisted_bounded_and_typed() {
    let allowed = BTreeSet::from(["search".to_string()]);
    let call = PageToolCall {
        name: "search".into(),
        authority: PageToolAuthority::ReadOnly,
        input: json!({"query":"rust"}),
        timeout_ms: 1_000,
        max_output_bytes: 16_384,
    };
    assert!(call.validate(&allowed).is_ok());
    let mut oversized = call.clone();
    oversized.max_output_bytes = 300_000;
    assert!(oversized.validate(&allowed).is_err());
}

#[test]
fn page_tool_program_uses_the_document_model_context_api_without_interpolation() {
    let discovery = PageToolProgram::discovery();
    let invocation = PageToolProgram::invocation();
    assert!(discovery.contains("context.getTools()"));
    assert!(invocation.contains("context.executeTool(tool,input"));
    assert!(invocation.contains("async function(name,input,expectedSchema,timeoutMs)"));
    assert!(invocation.contains("expectedSchema"));
    assert!(invocation.contains("{signal:controller.signal}"));
    assert!(invocation.contains("controller.abort()"));
    assert!(!invocation.contains("JSON.stringify(name)"));
}

#[tokio::test]
async fn page_tool_programs_use_webmcp_fixture_and_reject_schema_drift() {
    let runtime = rquickjs::AsyncRuntime::new().unwrap();
    let context = rquickjs::AsyncContext::full(&runtime).await.unwrap();
    let (discovery, invoked, drifted, tool_invoked) = context
        .async_with(async |ctx| {
            ctx.eval::<(), _>(r#"globalThis.AbortController=class{constructor(){this.signal={aborted:false}}abort(){this.signal.aborted=true}};globalThis.setTimeout=(callback)=>{callback();return 1};globalThis.clearTimeout=()=>{};globalThis.document={modelContext:{getTools:async()=>[{name:"search",description:"Search",inputSchema:{type:"object",properties:{query:{type:"string"}},required:["query"],additionalProperties:false}}],executeTool:async(tool,input,options)=>({tool:tool.name,query:input.query,signal_aborted:options.signal.aborted})}};"#).unwrap();
            let discover = format!("(async()=>JSON.stringify(await ({})()))()", PageToolProgram::discovery());
            let discovery = ctx.eval::<rquickjs::Promise, _>(discover).unwrap().into_future::<String>().await.unwrap();
            let schema = json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false});
            let name = serde_json::to_string("search").unwrap();
            let input = serde_json::to_string(&json!({"query":"cats"})).unwrap();
            let schema_literal = serde_json::to_string(&schema).unwrap();
            let invoked = ctx.eval::<rquickjs::Promise, _>(format!("(async()=>JSON.stringify(await ({})({name},{input},{schema_literal},2000)))()", PageToolProgram::invocation())).unwrap().into_future::<String>().await.unwrap();
            ctx.eval::<(), _>("document.modelContext.getTools=async()=>[{name:'search',inputSchema:{type:'object',properties:{query:{type:'string'},limit:{type:'string'}},required:['query'],additionalProperties:false}}];globalThis.toolInvoked=false;document.modelContext.executeTool=async()=>{toolInvoked=true;return 'unexpected'};").unwrap();
            let drifted = ctx.eval::<rquickjs::Promise, _>(format!("(async()=>JSON.stringify(await ({})({name},{input},{schema_literal})))()", PageToolProgram::invocation())).unwrap().into_future::<String>().await.unwrap();
            Ok::<_, rquickjs::Error>((discovery, invoked, drifted, ctx.eval::<bool, _>("toolInvoked").unwrap()))
        })
        .await
        .unwrap();
    let discovery: serde_json::Value = serde_json::from_str(&discovery).unwrap();
    let invoked: serde_json::Value = serde_json::from_str(&invoked).unwrap();
    let drifted: serde_json::Value = serde_json::from_str(&drifted).unwrap();
    assert_eq!(discovery["available"], true);
    assert_eq!(discovery["tools"][0]["name"], "search");
    assert_eq!(invoked["result"]["tool"], "search");
    assert_eq!(invoked["result"]["query"], "cats");
    assert_eq!(invoked["result"]["signal_aborted"], true);
    assert_eq!(drifted["schema_changed"], true);
    assert!(!tool_invoked);
}

#[tokio::test]
async fn page_tool_schema_comparison_preserves_proto_named_properties() {
    let runtime = rquickjs::AsyncRuntime::new().unwrap();
    let context = rquickjs::AsyncContext::full(&runtime).await.unwrap();
    let expected = json!({"type":"object","properties":{"__proto__":{"type":"string"}},"required":[],"additionalProperties":false});
    let current = json!({"type":"object","properties":{"__proto__":{"type":"integer"}},"required":[],"additionalProperties":false});
    let expected = serde_json::to_string(&expected).unwrap();
    let expected = serde_json::to_string(&expected).unwrap();
    let current = serde_json::to_string(&current).unwrap();
    let current = serde_json::to_string(&current).unwrap();
    let (result, invoked) = context
        .async_with(async |ctx| {
            ctx.eval::<(), _>(format!("globalThis.AbortController=class{{constructor(){{this.signal={{aborted:false}}}}abort(){{this.signal.aborted=true}}}};globalThis.setTimeout=()=>1;globalThis.clearTimeout=()=>{{}};globalThis.toolInvoked=false;globalThis.document={{modelContext:{{getTools:async()=>[{{name:'search',inputSchema:JSON.parse({current})}}],executeTool:async()=>{{toolInvoked=true;return 'unexpected'}}}}}};")).unwrap();
            let expression = format!("(async()=>{{try{{return JSON.stringify(await ({})('search',{{}},JSON.parse({expected}),2000))}}catch(error){{return JSON.stringify({{error:String(error)}})}}}})()", PageToolProgram::invocation());
            let result = ctx
                .eval::<rquickjs::Promise, _>(expression)
                .unwrap_or_else(|error| panic!("page invocation failed: {error}"))
                .into_future::<String>()
                .await
                .unwrap_or_else(|error| panic!("page promise failed: {error}"));
            Ok::<_, rquickjs::Error>((result, ctx.eval::<bool, _>("toolInvoked").unwrap()))
        })
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["schema_changed"], true, "{result}");
    assert!(!invoked);
}

#[tokio::test]
async fn page_tool_timeout_aborts_the_page_call_and_waits_for_cancellation() {
    let runtime = rquickjs::AsyncRuntime::new().unwrap();
    let context = rquickjs::AsyncContext::full(&runtime).await.unwrap();
    let result = context
        .async_with(async |ctx| {
            ctx.eval::<(), _>(
                r#"globalThis.AbortController=class{constructor(){this.listeners=[];this.signal={aborted:false,addEventListener:(_,listener)=>this.listeners.push(listener)}}abort(){this.signal.aborted=true;for(const listener of this.listeners)listener()}};globalThis.setTimeout=(callback)=>{queueMicrotask(callback);return 1};globalThis.clearTimeout=()=>{};globalThis.document={modelContext:{getTools:async()=>[{name:"save",inputSchema:{type:"object",properties:{},required:[],additionalProperties:false}}],executeTool:async(_tool,_input,{signal})=>new Promise((_,reject)=>signal.addEventListener("abort",()=>reject(new Error("cancelled"))))}};"#,
            )
            .unwrap();
            let schema = json!({"type":"object","properties":{},"required":[],"additionalProperties":false});
            let schema = serde_json::to_string(&serde_json::to_string(&schema).unwrap()).unwrap();
            let invocation = format!(
                "(async()=>{{try{{return JSON.stringify(await ({})('save',{{}},JSON.parse({schema}),1000))}}catch(error){{return JSON.stringify({{error:String(error)}})}}}})()",
                PageToolProgram::invocation()
            );
            let promise = ctx.eval::<rquickjs::Promise, _>(invocation).unwrap();
            promise.into_future::<String>().await
        })
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["error"], "Error: cancelled");
}

#[tokio::test]
async fn unavailable_webmcp_fixture_routes_to_semantic_ui() {
    let runtime = rquickjs::AsyncRuntime::new().unwrap();
    let context = rquickjs::AsyncContext::full(&runtime).await.unwrap();
    let discovery = context
        .async_with(async |ctx| {
            ctx.eval::<(), _>("globalThis.document={};").unwrap();
            let expression = format!(
                "(async()=>JSON.stringify(await ({})()))()",
                PageToolProgram::discovery()
            );
            ctx.eval::<rquickjs::Promise, _>(expression)
                .unwrap()
                .into_future::<String>()
                .await
        })
        .await
        .unwrap();
    let discovery: serde_json::Value = serde_json::from_str(&discovery).unwrap();
    assert_eq!(discovery, json!({"available":false,"tools":[]}));
    assert_eq!(
        controlla_runtime::v3_tasks::route_page_tool(&[], "save", true),
        controlla_runtime::v3_tasks::PageToolRoute::SemanticUi
    );
}

#[test]
fn reconnect_never_blindly_replays_mutations_and_is_bounded() {
    let mut controller = ReconnectController::new(2).unwrap();
    assert_eq!(
        controller.on_disconnect(RecoveryClass::SideEffecting),
        ReconnectDecision::StopUnknownMutation
    );
    assert_eq!(
        controller.on_disconnect(RecoveryClass::UnknownDelivery),
        ReconnectDecision::ObserveBeforeDecision
    );
    assert_eq!(
        controller.on_disconnect(RecoveryClass::IdempotentRead),
        ReconnectDecision::Terminal
    );
    assert_eq!(controller.attempts(), 2);
}

#[test]
fn default_agent_surface_is_exactly_six_compact_tools() {
    assert_eq!(
        DEFAULT_AGENT_TOOLS,
        [
            "browser", "snapshot", "act", "workflow", "extract", "verify"
        ]
    );
    assert!(compact_tool_surface_valid(47 * 1024));
    assert!(!compact_tool_surface_valid(49 * 1024));
    assert!(result_within_budget(&json!({"ok":true,"delta":[1,2,3]})));
}

#[test]
fn client_qualification_fails_closed() {
    let mut q = ClientQualification {
        client: ClientKind::Codex,
        startup: true,
        persistent_session: true,
        foreground: true,
        background: true,
        headless: true,
        typed_input: true,
        multi_tab: true,
        interference_safe: true,
        upload_download: true,
        research_extract: true,
        verified_mutations: false,
    };
    assert!(!q.qualified());
    q.verified_mutations = true;
    assert!(q.qualified());
}
