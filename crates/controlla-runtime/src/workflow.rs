use rquickjs::{AsyncContext, AsyncRuntime, Function, Promise, function::Async};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

pub const MAX_STEPS: usize = 20;
pub const MAX_WAIT_MS: u64 = 10_000;
pub const MAX_TOTAL_WAIT_MS: u64 = 30_000;
pub const MAX_OBSERVE_CALLS: usize = 20;
pub const MAX_JOB_DEADLINE_MS: u64 = 60_000;
pub const MAX_OUTPUT_BYTES: usize = 1_000_000;
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;
pub const MAX_SCRIPT_OUTPUT_BYTES: usize = 64 * 1024;
pub const MIN_SCRIPT_HEAP_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SCRIPT_HEAP_BYTES: usize = 128 * 1024 * 1024;
pub type ScriptBroker = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync,
>;

/// Runs trusted local JavaScript in a fresh QuickJS context. This is an in-process
/// resource-bounded interpreter, not an OS security boundary; callers must gate scripts.
pub async fn run_script(
    source: &str,
    timeout_ms: u64,
    heap_limit: usize,
    broker: ScriptBroker,
) -> Result<String, String> {
    if source.is_empty() || source.len() > MAX_SCRIPT_BYTES {
        return Err(format!("script must contain 1..={MAX_SCRIPT_BYTES} bytes"));
    }
    if timeout_ms == 0 || timeout_ms > MAX_JOB_DEADLINE_MS {
        return Err("script deadline exceeds workflow bound".into());
    }
    if !(MIN_SCRIPT_HEAP_BYTES..=MAX_SCRIPT_HEAP_BYTES).contains(&heap_limit) {
        return Err("script heap limit exceeds workflow bound".into());
    }
    let runtime = AsyncRuntime::new().map_err(|e| format!("QuickJS initialization failed: {e}"))?;
    runtime.set_memory_limit(heap_limit).await;
    runtime.set_max_stack_size(1024 * 1024).await;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    runtime
        .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)))
        .await;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|e| format!("QuickJS context failed: {e}"))?;
    let result = context.async_with(async |ctx| {
        let host_broker = broker.clone();
        let host = Function::new(ctx.clone(), Async(move |input: String| {
            let broker = host_broker.clone();
            async move {
                match broker(input).await {
                    Ok(value) => value,
                    Err(error) => serde_json::json!({"__controlla_error":error}).to_string(),
                }
            }
        })).map_err(|e| format!("QuickJS broker setup failed: {e}"))?;
        ctx.globals().set("__controlla_broker", host).map_err(|e| format!("QuickJS broker setup failed: {e}"))?;
        let init = "globalThis.api=((broker)=>Object.freeze({observe:async spec=>{if(!spec||typeof spec!=='object'||Object.keys(spec).some(k=>!['selector','fields','max_items','max_text_chars','max_bytes','cursor'].includes(k)))throw new Error('invalid broker spec');const r=JSON.parse(await broker(JSON.stringify(spec)));if(r&&r.__controlla_error)throw new Error(r.__controlla_error);return r}}))(__controlla_broker);delete globalThis.__controlla_broker;";
        ctx.eval::<(), _>(init).map_err(|e| format!("QuickJS API setup failed: {e}"))?;
        let wrapped = format!("(async()=>{{const value=await (async()=>{{{source}\n}})();return JSON.stringify(value===undefined?null:value)}})()");
        let promise = ctx.eval::<Promise, _>(wrapped).map_err(|e| format!("script failed: {e}"))?;
        promise.into_future::<String>().await.map_err(|e| format!("script failed: {e}"))
    }).await?;
    if result.len() > MAX_SCRIPT_OUTPUT_BYTES {
        return Err("script output exceeds 65536 bytes".into());
    }
    Ok(result)
}

#[derive(Clone, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkflowObserveSpec {
    pub selector: String,
    pub fields: std::collections::BTreeMap<String, String>,
    pub max_items: usize,
    pub max_text_chars: usize,
    pub max_bytes: usize,
    pub cursor: Option<String>,
}

impl WorkflowObserveSpec {
    pub fn into_observe_spec(self) -> controlla_browser::ObserveSpec {
        controlla_browser::ObserveSpec {
            selector: self.selector,
            fields: self.fields,
            max_items: self.max_items,
            max_text_chars: self.max_text_chars,
            max_bytes: self.max_bytes,
            cursor: self.cursor,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowStep {
    Observe { spec: WorkflowObserveSpec },
    Wait { ms: u64 },
    Checkpoint,
    Script { source: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRequest {
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug)]
pub struct WorkflowGraph {
    pub steps: Vec<WorkflowStep>,
}

pub fn compile_steps(steps: Vec<WorkflowStep>) -> Result<WorkflowGraph, String> {
    compile(serde_json::json!({"steps":steps}))
}

pub fn validate_binding(
    request_session: &str,
    reference_session: &str,
    principal: &str,
    expected_principal: &str,
) -> Result<(), &'static str> {
    if request_session != reference_session || principal != expected_principal {
        Err("target_ref is not bound to this session")
    } else {
        Ok(())
    }
}

pub fn compile(request: Value) -> Result<WorkflowGraph, String> {
    fn only_keys(value: &Value, allowed: &[&str]) -> bool {
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
    }
    if !only_keys(&request, &["steps"]) {
        return Err("workflow accepts only steps".into());
    }
    let raw_steps = request["steps"]
        .as_array()
        .ok_or("steps must be an array")?;
    for raw in raw_steps {
        let kind = raw["kind"].as_str().ok_or("step kind is required")?;
        let keys: &[&str] = match kind {
            "observe" => &["kind", "spec"],
            "wait" => &["kind", "ms"],
            "checkpoint" => &["kind"],
            "script" => &["kind", "source"],
            _ => return Err(format!("unsupported workflow node: {kind}")),
        };
        if !only_keys(raw, keys) {
            return Err(format!("unsupported fields in {kind} node"));
        }
        if kind == "observe" {
            let spec = &raw["spec"];
            if !only_keys(
                spec,
                &[
                    "selector",
                    "fields",
                    "max_items",
                    "max_text_chars",
                    "max_bytes",
                    "cursor",
                ],
            ) || !spec["fields"]
                .as_object()
                .is_some_and(|fields| fields.values().all(Value::is_string))
            {
                return Err("invalid observe spec fields".into());
            }
        }
    }
    let request: WorkflowRequest =
        serde_json::from_value(request).map_err(|error| format!("invalid workflow: {error}"))?;
    if request.steps.is_empty() || request.steps.len() > MAX_STEPS {
        return Err(format!("steps must contain 1..={MAX_STEPS} nodes"));
    }
    let mut waits = 0_u64;
    let mut declared_output = 0_usize;
    let mut observe_calls = 0_usize;
    let scripts: Vec<_> = request
        .steps
        .iter()
        .filter_map(|step| match step {
            WorkflowStep::Script { source } => Some(source),
            _ => None,
        })
        .collect();
    if !scripts.is_empty()
        && (request.steps.len() != 1
            || scripts.len() != 1
            || scripts[0].is_empty()
            || scripts[0].len() > MAX_SCRIPT_BYTES)
    {
        return Err("a script must be the only workflow step and fit the script byte limit".into());
    }
    for step in &request.steps {
        match step {
            WorkflowStep::Wait { ms } if *ms == 0 || *ms > MAX_WAIT_MS => {
                return Err(format!("wait must be 1..={MAX_WAIT_MS} ms"));
            }
            WorkflowStep::Wait { ms } => waits = waits.saturating_add(*ms),
            WorkflowStep::Observe { spec } => {
                observe_calls += 1;
                if spec.selector.is_empty()
                    || spec.selector.len() > 512
                    || spec.fields.is_empty()
                    || spec.fields.len() > 32
                    || spec.max_items == 0
                    || spec.max_items > 1000
                    || spec.max_text_chars == 0
                    || spec.max_text_chars > 100_000
                    || !(4096..=MAX_OUTPUT_BYTES).contains(&spec.max_bytes)
                    || spec
                        .cursor
                        .as_ref()
                        .is_some_and(|cursor| cursor.len() > 4096)
                    || spec.fields.iter().any(|(name, selector)| {
                        name.is_empty()
                            || name.len() > 64
                            || selector.is_empty()
                            || selector.len() > 512
                    })
                {
                    return Err("observe spec exceeds workflow bounds".into());
                }
                declared_output = declared_output.saturating_add(spec.max_bytes);
            }
            _ => {}
        }
    }
    if waits > MAX_TOTAL_WAIT_MS {
        return Err(format!("total wait exceeds {MAX_TOTAL_WAIT_MS} ms"));
    }
    if (observe_calls == 0 && scripts.is_empty()) || observe_calls > MAX_OBSERVE_CALLS {
        return Err(format!(
            "workflow requires 1..={MAX_OBSERVE_CALLS} observations"
        ));
    }
    if declared_output > MAX_OUTPUT_BYTES - 1024 {
        return Err("aggregate observe byte budget exceeds workflow output limit".into());
    }
    Ok(WorkflowGraph {
        steps: request.steps,
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkflowReceipt {
    pub status: &'static str,
    pub completed_steps: usize,
    pub browser_operations: usize,
    pub steps: Vec<Value>,
    pub artifacts: Vec<Value>,
    pub error: Option<String>,
}

impl WorkflowReceipt {
    pub fn partial(error: impl Into<String>, steps: Vec<Value>) -> Self {
        Self {
            status: "partial",
            completed_steps: steps.len(),
            browser_operations: steps.iter().filter(|s| s["kind"] == "observe").count(),
            steps,
            artifacts: vec![],
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{future::Future, pin::Pin, sync::Arc};

    type MockBroker = Arc<
        dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
            + Send
            + Sync,
    >;

    fn broker() -> MockBroker {
        Arc::new(|input| Box::pin(async move { Ok(format!("{{\"received\":{input}}}")) }))
    }

    #[tokio::test]
    async fn js_worker_awaits_only_the_async_broker_api() {
        let output = run_script(
            "return await api.observe({selector:'p'});",
            1000,
            8 * 1024 * 1024,
            broker(),
        )
        .await
        .unwrap();
        assert_eq!(output, "{\"received\":{\"selector\":\"p\"}}");
    }

    #[tokio::test]
    async fn js_worker_interrupts_loops_and_enforces_memory_and_output_limits() {
        assert!(
            run_script("while(true){}", 20, 8 * 1024 * 1024, broker())
                .await
                .is_err()
        );
        assert!(
            run_script(
                "return 'x'.repeat(4_000_000);",
                1000,
                2 * 1024 * 1024,
                broker()
            )
            .await
            .is_err()
        );
        assert!(
            run_script(
                "return 'x'.repeat(70_000);",
                1000,
                8 * 1024 * 1024,
                broker()
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn js_worker_has_no_ambient_module_network_file_or_process_access() {
        let globals = run_script(
            "return [typeof process,typeof fetch,typeof Deno,typeof require,typeof api].join(',');",
            1000,
            8 * 1024 * 1024,
            broker(),
        )
        .await
        .unwrap();
        assert_eq!(
            globals,
            "\"undefined,undefined,undefined,undefined,object\""
        );
        assert!(
            run_script(
                "await import('node:fs'); return 'bad';",
                1000,
                8 * 1024 * 1024,
                broker()
            )
            .await
            .is_err()
        );
        assert!(
            run_script(
                "return await api.observe({selector:'p',session_id:'other'});",
                1000,
                8 * 1024 * 1024,
                broker()
            )
            .await
            .is_err()
        );
    }

    #[test]
    fn bounded_workflow_compiles_and_rejects_escape_and_resource_limits() {
        let valid = json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"text":"p"},"max_items":2,"max_text_chars":40,"max_bytes":4096,"cursor":null}},{"kind":"wait","ms":10},{"kind":"checkpoint"}]});
        assert!(compile(valid).is_ok());
        for invalid in [
            json!({"steps":[{"kind":"script","source":"x"},{"kind":"checkpoint"}]}),
            json!({"steps":[{"kind":"script","source":"x".repeat(MAX_SCRIPT_BYTES+1)}]}),
            json!({"steps":(0..22).map(|_|json!({"kind":"checkpoint"})).collect::<Vec<_>>()}),
            json!({"steps":[{"kind":"wait","ms":10001}]}),
            json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":1001,"max_text_chars":40,"max_bytes":4096,"cursor":null}}]}),
            json!({"steps":[{"kind":"checkpoint","module":"node:fs"}]}),
            json!({"steps":[{"kind":"checkpoint","network":"fetch"}]}),
            json!({"steps":[{"kind":"checkpoint","process":"spawn"}]}),
            json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":2,"max_text_chars":40,"max_bytes":1000001,"cursor":null}}]}),
            json!({"steps":[{"kind":"wait","ms":10000},{"kind":"wait","ms":10000},{"kind":"wait","ms":10000},{"kind":"wait","ms":1}]}),
            json!({"steps":[{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":2,"max_text_chars":40,"max_bytes":600000,"cursor":null}},{"kind":"observe","spec":{"selector":"p","fields":{"x":"p"},"max_items":2,"max_text_chars":40,"max_bytes":400000,"cursor":null}}]}),
        ] {
            assert!(compile(invalid.clone()).is_err(), "accepted: {invalid}");
        }
        assert!(compile(json!({"steps":[{"kind":"script","source":"return 1;"}]})).is_ok());
    }

    #[test]
    fn workflow_rejects_cross_session_and_cross_principal_handles() {
        assert!(validate_binding("s1", "s2", "p", "p").is_err());
        assert!(validate_binding("s1", "s1", "p1", "p2").is_err());
        assert!(validate_binding("s1", "s1", "p", "p").is_ok());
    }
}
