use rquickjs::{AsyncContext, AsyncRuntime, Function, Promise, function::Async};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt};

pub const MAX_STEPS: usize = 20;
pub const MAX_WAIT_MS: u64 = 10_000;
pub const MAX_TOTAL_WAIT_MS: u64 = 30_000;
pub const MAX_OBSERVE_CALLS: usize = 20;
pub const MAX_JOB_DEADLINE_MS: u64 = 60_000;
pub const MAX_OUTPUT_BYTES: usize = 1_000_000;
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;
pub const MAX_SCRIPT_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_GENERATED_ARTIFACT_BYTES: usize = 12 * 1024;
pub const MIN_SCRIPT_HEAP_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SCRIPT_HEAP_BYTES: usize = 128 * 1024 * 1024;
const MAX_WORKER_FRAME_BYTES: usize = MAX_OUTPUT_BYTES * 6 + 16_384;
const MAX_WORKER_REQUEST_BYTES: usize = MAX_SCRIPT_BYTES * 6 + 8192;
const SCRIPT_WORKER_ARG: &str = "--__controlla-script-worker";

pub fn script_worker_executable() -> Result<std::path::PathBuf, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(test)]
    if let Some(binary) = current
        .parent()
        .and_then(std::path::Path::parent)
        .map(|directory| directory.join("controlla"))
        .filter(|path| path.is_file())
    {
        return Ok(binary);
    }
    Ok(current)
}
pub type ScriptBroker = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync,
>;

/// Evaluates one script in the worker process. The caller must supply only a
/// trusted executable and keep browser authorization in the broker callback.
pub async fn run_script_isolated(
    executable: &std::path::Path,
    source: &str,
    timeout_ms: u64,
    heap_limit: usize,
    broker: ScriptBroker,
) -> Result<String, String> {
    validate_script_bounds(source, timeout_ms, heap_limit)?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let mut child = tokio::process::Command::new(executable)
        .arg(SCRIPT_WORKER_ARG)
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("script worker failed to start: {error}"))?;
    let input = child
        .stdin
        .take()
        .ok_or_else(|| "script worker stdin unavailable".to_owned())?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| "script worker stdout unavailable".to_owned())?;
    let run = async {
        let mut input = tokio::io::BufWriter::new(input);
        let mut output = tokio::io::BufReader::new(output);
        write_worker_input(
            &mut input,
            &WorkerInput::Run {
                source: source.to_owned(),
                timeout_ms,
                heap_limit,
            },
        )
        .await?;
        loop {
            let line = read_worker_line(&mut output, MAX_WORKER_FRAME_BYTES)
                .await?
                .ok_or_else(|| "script worker exited without a result".to_owned())?;
            let message: WorkerOutput = serde_json::from_str(&line)
                .map_err(|error| format!("invalid script worker response: {error}"))?;
            match message {
                WorkerOutput::Observe { id, spec } => {
                    let result = broker(spec).await;
                    write_worker_input(&mut input, &WorkerInput::BrokerReply { id, result })
                        .await?;
                }
                WorkerOutput::Done { result } => {
                    let status = child
                        .wait()
                        .await
                        .map_err(|error| format!("script worker wait failed: {error}"))?;
                    if !status.success() {
                        return Err("script worker exited unsuccessfully".into());
                    }
                    return result;
                }
            }
        }
    };
    match tokio::time::timeout_at(deadline, run).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            Err(error)
        }
        Err(_) => {
            let _ = child.kill().await;
            Err("script deadline exceeded".into())
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerInput {
    Run {
        source: String,
        timeout_ms: u64,
        heap_limit: usize,
    },
    BrokerReply {
        id: u64,
        result: Result<String, String>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerOutput {
    Observe { id: u64, spec: String },
    Done { result: Result<String, String> },
}

fn validate_script_bounds(source: &str, timeout_ms: u64, heap_limit: usize) -> Result<(), String> {
    if source.is_empty() || source.len() > MAX_SCRIPT_BYTES {
        return Err(format!("script must contain 1..={MAX_SCRIPT_BYTES} bytes"));
    }
    if timeout_ms == 0 || timeout_ms > MAX_JOB_DEADLINE_MS {
        return Err("script deadline exceeds workflow bound".into());
    }
    if !(MIN_SCRIPT_HEAP_BYTES..=MAX_SCRIPT_HEAP_BYTES).contains(&heap_limit) {
        return Err("script heap limit exceeds workflow bound".into());
    }
    Ok(())
}

async fn write_worker_input(
    input: &mut (impl tokio::io::AsyncWrite + Unpin),
    message: &WorkerInput,
) -> Result<(), String> {
    let encoded = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    if encoded.len() + 1 > MAX_WORKER_FRAME_BYTES {
        return Err("script worker frame exceeds limit".into());
    }
    input
        .write_all(&encoded)
        .await
        .map_err(|error| error.to_string())?;
    input
        .write_all(b"\n")
        .await
        .map_err(|error| error.to_string())?;
    input
        .flush()
        .await
        .map_err(|error| format!("script worker write failed: {error}"))
}

async fn read_worker_line(
    input: &mut (impl AsyncBufRead + Unpin),
    limit: usize,
) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    loop {
        let available = input
            .fill_buf()
            .await
            .map_err(|error| format!("script worker read failed: {error}"))?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err("script worker returned a truncated frame".into())
            };
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len() + count > limit {
            return Err("script worker frame exceeds limit".into());
        }
        let done = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        input.consume(count);
        if done {
            bytes.pop();
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "script worker returned invalid UTF-8".into());
        }
    }
}

/// Hidden CLI entry point for the bounded QuickJS child. Browser operations
/// are requests over stdio; this process has no session or browser credentials.
pub fn run_script_worker() -> Result<(), String> {
    let input = std::io::stdin();
    let mut line = String::new();
    read_sync_worker_line(&mut input.lock(), &mut line, MAX_WORKER_REQUEST_BYTES)?;
    let WorkerInput::Run {
        source,
        timeout_ms,
        heap_limit,
    } = serde_json::from_str(&line).map_err(|error| format!("invalid worker request: {error}"))?
    else {
        return Err("worker expected a run request".into());
    };
    let next_id = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let broker: ScriptBroker = Arc::new(move |spec| {
        let id = next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            let output = std::io::stdout();
            let mut output = output.lock();
            serde_json::to_writer(&mut output, &WorkerOutput::Observe { id, spec })
                .map_err(|error| error.to_string())?;
            use std::io::Write;
            output
                .write_all(b"\n")
                .and_then(|_| output.flush())
                .map_err(|error| error.to_string())?;
            let input = std::io::stdin();
            let mut line = String::new();
            read_sync_worker_line(&mut input.lock(), &mut line, MAX_WORKER_FRAME_BYTES)?;
            match serde_json::from_str::<WorkerInput>(&line).map_err(|error| error.to_string())? {
                WorkerInput::BrokerReply {
                    id: reply_id,
                    result,
                } if reply_id == id => result,
                _ => Err("worker received an invalid broker reply".into()),
            }
        })
    });
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?
        .block_on(run_script_in_process(
            &source, timeout_ms, heap_limit, broker,
        ));
    let output = std::io::stdout();
    let mut output = output.lock();
    serde_json::to_writer(&mut output, &WorkerOutput::Done { result })
        .map_err(|error| error.to_string())?;
    use std::io::Write;
    output
        .write_all(b"\n")
        .and_then(|_| output.flush())
        .map_err(|error| error.to_string())
}

fn read_sync_worker_line(
    input: &mut impl std::io::BufRead,
    line: &mut String,
    limit: usize,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    loop {
        let available = input
            .fill_buf()
            .map_err(|error| format!("worker input failed: {error}"))?;
        if available.is_empty() {
            return Err("worker input ended".into());
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len() + count > limit {
            return Err("worker frame exceeds limit".into());
        }
        let done = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        input.consume(count);
        if done {
            bytes.pop();
            break;
        }
    }
    *line = String::from_utf8(bytes).map_err(|_| "worker frame is not UTF-8".to_owned())?;
    Ok(())
}

/// Runs trusted local JavaScript inside the worker process's QuickJS context.
async fn run_script_in_process(
    source: &str,
    timeout_ms: u64,
    heap_limit: usize,
    broker: ScriptBroker,
) -> Result<String, String> {
    validate_script_bounds(source, timeout_ms, heap_limit)?;
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
    let observe_budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let result = tokio::time::timeout(Duration::from_millis(timeout_ms), context.async_with(async |ctx| {
        let host_broker = broker.clone();
        let host_budget = observe_budget.clone();
        let host = Function::new(ctx.clone(), Async(move |input: String| {
            let broker = host_broker.clone();
            let budget = host_budget.clone();
            async move {
                let spec: WorkflowObserveSpec = match serde_json::from_str(&input) {
                    Ok(spec) => spec,
                    Err(_) => return serde_json::json!({"__controlla_error":"invalid brokered observe spec"}).to_string(),
                };
                if let Err(error) = compile_steps(vec![WorkflowStep::Observe { spec: spec.clone() }]) {
                    return serde_json::json!({"__controlla_error":error}).to_string();
                }
                if budget.try_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |used| used.checked_add(spec.max_bytes).filter(|total| *total <= MAX_OUTPUT_BYTES - 1024),
                ).is_err() {
                    return serde_json::json!({"__controlla_error":"aggregate observe byte budget exceeds workflow output limit"}).to_string();
                }
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
    }))
    .await
    .map_err(|_| "script deadline exceeded".to_string())??;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptArtifactRequest {
    kind: String,
    filename: String,
    media_type: String,
    bytes: Vec<u8>,
}

/// Parses the explicit script artifact return shape into an inline, receipt-bound value.
pub fn validate_script_artifact(
    output: &str,
    operation_id: &str,
    principal: &str,
    session_id: &str,
) -> Result<Option<Value>, String> {
    let value: Value = serde_json::from_str(output).map_err(|_| "invalid script JSON output")?;
    if value.get("kind").and_then(Value::as_str) != Some("artifact") {
        return Ok(None);
    }
    let artifact: ScriptArtifactRequest =
        serde_json::from_value(value).map_err(|_| "invalid artifact return fields")?;
    if artifact.kind != "artifact"
        || artifact.bytes.is_empty()
        || artifact.bytes.len() > MAX_GENERATED_ARTIFACT_BYTES
    {
        return Err("artifact bytes must contain 1..=12288 bytes".into());
    }
    if artifact.filename.is_empty()
        || artifact.filename.len() > 128
        || artifact.filename.starts_with('.')
        || artifact.filename == ".."
        || !artifact
            .filename
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err("artifact filename must be 1..=128 safe ASCII characters".into());
    }
    if !matches!(
        artifact.media_type.as_str(),
        "application/json" | "application/pdf" | "image/png" | "text/plain"
    ) {
        return Err("artifact media_type is unsupported".into());
    }
    let sha256 = Sha256::digest(&artifact.bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Some(serde_json::json!({
        "artifact_id": format!("{operation_id}:artifact:0"),
        "operation_id": operation_id,
        "principal": principal,
        "session_id": session_id,
        "filename": artifact.filename,
        "media_type": artifact.media_type,
        "encoding": "uint8-array",
        "size": artifact.bytes.len(),
        "sha256": sha256,
        "bytes": artifact.bytes
    })))
}

/// Declared effects are scheduling hints, never proof that a page is side-effect free.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectClass {
    ReadOnly,
    Reversible,
    External,
    AuthorityBoundary,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedStep {
    pub id: String,
    pub dependencies: Vec<String>,
    pub effect: EffectClass,
}

/// Returns step IDs whose declared inputs changed, including their dependent steps.
pub fn invalidated_steps(steps: &[PlannedStep], changed: &[String]) -> Vec<String> {
    let mut invalid: std::collections::BTreeSet<String> = changed.iter().cloned().collect();
    let mut result = Vec::new();
    loop {
        let newly_invalid: Vec<_> = steps
            .iter()
            .filter(|step| !invalid.contains(&step.id))
            .filter(|step| step.dependencies.iter().any(|key| invalid.contains(key)))
            .map(|step| step.id.clone())
            .collect();
        if newly_invalid.is_empty() {
            break;
        }
        for id in newly_invalid {
            invalid.insert(id.clone());
            result.push(id);
        }
    }
    result
}

/// Groups adjacent declared read-only steps. Every other effect is isolated.
pub fn effect_aware_batches(steps: &[PlannedStep]) -> Vec<std::ops::Range<usize>> {
    let mut batches = Vec::new();
    let mut start = 0;
    while start < steps.len() {
        let end = if steps[start].effect == EffectClass::ReadOnly {
            (start + 1..steps.len())
                .find(|&index| steps[index].effect != EffectClass::ReadOnly)
                .unwrap_or(steps.len())
        } else {
            start + 1
        };
        batches.push(start..end);
        start = end;
    }
    batches
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

    fn rejecting_broker() -> MockBroker {
        Arc::new(|_| Box::pin(async { Err("broker rejected".to_owned()) }))
    }

    fn delayed_broker() -> MockBroker {
        Arc::new(|input| {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(format!("{{\"received\":{input}}}"))
            })
        })
    }

    #[tokio::test]
    async fn js_worker_awaits_only_the_async_broker_api() {
        let output = run_script_in_process(
            "return await api.observe({selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:4096,cursor:null});",
            1000,
            8 * 1024 * 1024,
            delayed_broker(),
        )
        .await
        .unwrap();
        assert_eq!(
            output,
            "{\"received\":{\"selector\":\"p\",\"fields\":{\"text\":\"p\"},\"max_items\":2,\"max_text_chars\":40,\"max_bytes\":4096,\"cursor\":null}}"
        );
    }

    #[tokio::test]
    async fn js_worker_returns_fulfilled_async_promise_from_broker() {
        let output = run_script_in_process(
            "return await api.observe({selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:4096,cursor:null}).then(value=>value.received.selector);",
            1000,
            8 * 1024 * 1024,
            broker(),
        )
        .await
        .unwrap();
        assert_eq!(output, "\"p\"");
    }

    #[tokio::test]
    async fn js_worker_resolves_broker_promise_after_exactly_12_seconds() {
        let broker: MockBroker = Arc::new(|input| {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_secs(12)).await;
                Ok(format!("{{\"received\":{input}}}"))
            })
        });
        let output = run_script_in_process(
            "return await api.observe({selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:4096,cursor:null}).then(value=>value.received.selector);",
            15_000,
            8 * 1024 * 1024,
            broker,
        )
        .await
        .unwrap();
        assert_eq!(output, "\"p\"");
    }

    #[tokio::test]
    async fn js_worker_propagates_rejected_async_promise_from_broker() {
        let caught = run_script_in_process(
            "return await api.observe({selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:4096,cursor:null}).catch(error=>error.message);",
            1000,
            8 * 1024 * 1024,
            rejecting_broker(),
        )
        .await
        .unwrap();
        assert_eq!(caught, "\"broker rejected\"");

        let uncaught = run_script_in_process(
            "return await api.observe({selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:4096,cursor:null});",
            1000,
            8 * 1024 * 1024,
            rejecting_broker(),
        )
        .await;
        assert!(uncaught.is_err());
    }

    #[tokio::test]
    async fn js_worker_enforces_aggregate_observe_bytes_before_excess_broker_calls() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let broker: MockBroker = Arc::new(move |_| {
            let seen = seen.clone();
            Box::pin(async move {
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok("{}".to_owned())
            })
        });
        let source = "const spec={selector:'p',fields:{text:'p'},max_items:2,max_text_chars:40,max_bytes:600000,cursor:null}; await api.observe(spec); await api.observe(spec); return 'bad';";
        assert!(
            run_script_in_process(source, 1000, 8 * 1024 * 1024, broker)
                .await
                .is_err()
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn js_worker_interrupts_loops_and_enforces_memory_and_output_limits() {
        assert!(
            run_script_in_process("while(true){}", 20, 8 * 1024 * 1024, broker())
                .await
                .is_err()
        );
        assert!(
            run_script_in_process(
                "return 'x'.repeat(4_000_000);",
                1000,
                2 * 1024 * 1024,
                broker()
            )
            .await
            .is_err()
        );
        assert!(
            run_script_in_process(
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
    async fn js_worker_deadline_cancels_a_never_settling_promise() {
        let result = tokio::time::timeout(
            Duration::from_millis(150),
            run_script_in_process(
                "return await new Promise(()=>{});",
                20,
                8 * 1024 * 1024,
                broker(),
            ),
        )
        .await
        .expect("run_script must enforce its own deadline");
        assert!(result.unwrap_err().contains("deadline"));
    }

    #[test]
    fn generated_artifact_contract_is_bounded_strict_and_identity_bound() {
        let result = validate_script_artifact(
            r#"{"kind":"artifact","filename":"summary.json","media_type":"application/json","bytes":[123,125]}"#,
            "operation-1",
            "local-stdio",
            "session-1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(result["operation_id"], "operation-1");
        assert_eq!(result["principal"], "local-stdio");
        assert_eq!(result["session_id"], "session-1");
        assert_eq!(result["filename"], "summary.json");
        assert_eq!(result["bytes"], json!([123, 125]));
        for invalid in [
            r#"{"kind":"artifact","filename":"../x","media_type":"text/plain","bytes":[1]}"#,
            r#"{"kind":"artifact","filename":"x","media_type":"application/x-executable","bytes":[1]}"#,
            r#"{"kind":"artifact","filename":"x","media_type":"text/plain","bytes":[256]}"#,
            r#"{"kind":"artifact","filename":"x","media_type":"text/plain","bytes":[1],"path":"/tmp/x"}"#,
        ] {
            assert!(validate_script_artifact(invalid, "op", "p", "s").is_err());
        }
        let oversized = format!(
            r#"{{"kind":"artifact","filename":"x","media_type":"text/plain","bytes":[{}]}}"#,
            std::iter::repeat_n("1", MAX_GENERATED_ARTIFACT_BYTES + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(validate_script_artifact(&oversized, "op", "p", "s").is_err());
        assert!(
            validate_script_artifact(r#"{"kind":"value","anything":true}"#, "op", "p", "s")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn js_worker_has_no_ambient_module_network_file_or_process_access() {
        let globals = run_script_in_process(
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
            run_script_in_process(
                "await import('node:fs'); return 'bad';",
                1000,
                8 * 1024 * 1024,
                broker()
            )
            .await
            .is_err()
        );
        assert!(
            run_script_in_process(
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

    #[test]
    fn fixture_planner_invalidates_only_dependents_and_splits_effect_boundaries() {
        let steps = vec![
            PlannedStep {
                id: "read_account".into(),
                dependencies: vec!["account".into()],
                effect: EffectClass::ReadOnly,
            },
            PlannedStep {
                id: "read_rows".into(),
                dependencies: vec!["account".into(), "rows".into()],
                effect: EffectClass::ReadOnly,
            },
            PlannedStep {
                id: "format".into(),
                dependencies: vec!["read_rows".into()],
                effect: EffectClass::ReadOnly,
            },
            PlannedStep {
                id: "submit".into(),
                dependencies: vec!["format".into()],
                effect: EffectClass::External,
            },
            PlannedStep {
                id: "confirm".into(),
                dependencies: vec!["submit".into()],
                effect: EffectClass::Unknown,
            },
            PlannedStep {
                id: "audit".into(),
                dependencies: vec!["unrelated_counter".into()],
                effect: EffectClass::ReadOnly,
            },
        ];
        assert_eq!(
            invalidated_steps(&steps, &["rows".into()]),
            ["read_rows", "format", "submit", "confirm"]
        );
        assert_eq!(effect_aware_batches(&steps), [0..3, 3..4, 4..5, 5..6]);
        // Fixture-only comparison: same six logical steps, with deterministic round-trip counts.
        let fixed_batch_calls = 1;
        let bounded_code_calls = 1;
        let guarded_compiler_calls = effect_aware_batches(&steps).len();
        assert_eq!(
            (
                fixed_batch_calls,
                bounded_code_calls,
                guarded_compiler_calls
            ),
            (1, 1, 4)
        );
    }
}
