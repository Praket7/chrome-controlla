import { randomUUID, createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');
const MAX_STDIO_BYTES = 1024 * 1024;
const MAX_REPORT_ROW_BYTES = 12 * 1024;
const MAX_PAIRED_ROW_BYTES = 2 * MAX_REPORT_ROW_BYTES + 2048;
export const MAX_REPORT_BYTES = 256 * 1024 * 1024;
const OPTIONAL_OUTPUT_FIELDS = [
  'source_ref', 'schema_bytes', 'external_controlla_calls', 'model_turns',
  'model_visible_observations', 'internal_model_calls', 'browser_cdp_commands',
  'native_host_messages_sent', 'native_host_messages_received', 'tool_input_bytes',
  'unknown_delivery_count', 'timeout_count', 'cleanup_completed', 'cleanup_outcome',
  'metric_evidence', 'execution_tier', 'fallback_count', 'runner_error', 'prompt_to_final_ms',
  'codex_event_types', 'codex_error_class', 'codex_turn_failed', 'codex_failed_tool',
  'codex_tool_failure_status', 'codex_tool_failure_reason', 'codex_failed_action',
];

export async function loadPhase11Config(configPath = 'bench/tasks/phase11-realistic.json') {
  return JSON.parse(await readFile(resolve(root, configPath), 'utf8'));
}

export async function loadAdapterContract() {
  return JSON.parse(await readFile(resolve(root, 'bench/runners/adapter-contract.json'), 'utf8'));
}

function commandEnvName(adapterId) {
  return `PHASE11_${adapterId.toUpperCase().replace(/[^A-Z0-9]+/g, '_')}_COMMAND_JSON`;
}

export function parseCommand(value, label) {
  if (!value) throw new Error(`${label} is required and must be a JSON argv array`);
  let argv;
  try {
    argv = JSON.parse(value);
  } catch (error) {
    throw new Error(`${label} must be valid JSON: ${error.message}`);
  }
  if (!Array.isArray(argv) || argv.length === 0 || argv.some(part => typeof part !== 'string' || part.length === 0)) {
    throw new Error(`${label} must be a non-empty JSON array of non-empty strings`);
  }
  return argv;
}

function boundedAppend(current, chunk, label) {
  const next = current + chunk;
  if (Buffer.byteLength(next) > MAX_STDIO_BYTES) throw new Error(`${label} exceeded ${MAX_STDIO_BYTES} bytes`);
  return next;
}

function sanitizePromptTrace(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const output = {};
  for (const key of ['trace_valid', 'final_answer_observed']) {
    if (typeof value[key] === 'boolean') output[key] = value[key];
  }
  output.prompt_sent_observed = value.prompt_sent_observed === true && value.prompt_boundary === 'actual_user_submit';
  if (value.prompt_boundary === 'actual_user_submit') output.prompt_boundary = value.prompt_boundary;
  if (value.clock_domain === 'local_host_monotonic') output.clock_domain = value.clock_domain;
  for (const key of ['prompt_sent_monotonic_ns', 'process_start_monotonic_ns', 'process_end_monotonic_ns']) {
    if (typeof value[key] === 'string' && /^\d{1,32}$/.test(value[key])) output[key] = value[key];
  }
  if (value.clock_offset_ms === 0) output.clock_offset_ms = 0;
  if (Number.isFinite(value.clock_uncertainty_ms) && value.clock_uncertainty_ms >= 0 && value.clock_uncertainty_ms <= 5) {
    output.clock_uncertainty_ms = value.clock_uncertainty_ms;
  }
  for (const key of ['process_start_to_exit_ms', 'prompt_to_first_tool_decision_ms', 'prompt_to_controlla_received_ms',
    'prompt_to_first_browser_op_ms', 'prompt_to_last_browser_op_ms', 'prompt_to_verified_ms',
    'prompt_to_tool_result_delivered_ms', 'prompt_to_final_ms']) {
    if (Number.isFinite(value[key]) && value[key] >= 0) output[key] = value[key];
  }
  if (value.counters && typeof value.counters === 'object' && !Array.isArray(value.counters)) {
    const counters = {};
    if (typeof value.counters.partial_counters === 'boolean') counters.partial_counters = value.counters.partial_counters;
    if (typeof value.counters.missing_counters === 'string') counters.missing_counters = value.counters.missing_counters.slice(0, 256);
    output.counters = counters;
  }
  return Object.keys(output).length ? output : null;
}

function attachPartialPromptTrace(error, stdout) {
  try {
    const value = JSON.parse(stdout);
    error.partial_prompt_trace = sanitizePromptTrace(value?.prompt_trace);
  } catch {
    error.partial_prompt_trace = null;
  }
  return error;
}

export async function invokeJsonCommand(argv, payload, timeoutMs, env = process.env, instrumentation = undefined) {
  const inputText = JSON.stringify(payload);
  const startedAt = process.hrtime.bigint();
  return await new Promise((resolvePromise, reject) => {
    const child = spawn(argv[0], argv.slice(1), {
      cwd: root,
      env,
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
    });
    let stdout = '';
    let stderr = '';
    let settled = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(attachPartialPromptTrace(error, stdout));
      else resolvePromise(value);
    };
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      finish(new Error(`command deadline exceeded after ${timeoutMs}ms`));
    }, timeoutMs);
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      try { stdout = boundedAppend(stdout, chunk, 'stdout'); }
      catch (error) { child.kill('SIGKILL'); finish(error); }
    });
    child.stderr.on('data', chunk => {
      try { stderr = boundedAppend(stderr, chunk, 'stderr'); }
      catch (error) { child.kill('SIGKILL'); finish(error); }
    });
    child.on('error', error => finish(error));
    child.on('close', code => {
      if (settled) return;
      if (instrumentation) {
        const now = process.hrtime.bigint();
        instrumentation.output_bytes = Buffer.byteLength(stdout);
        instrumentation.tool_result_delivered_ns = now.toString();
        instrumentation.command_elapsed_ms = Number(now - startedAt) / 1_000_000;
      }
      if (code !== 0) return finish(new Error(`command exited ${code}: ${stderr.trim()}`));
      try {
        finish(null, JSON.parse(stdout));
      } catch (error) {
        finish(new Error(`command returned invalid JSON: ${error.message}; stderr=${stderr.trim()}`));
      }
    });
    child.stdin.end(inputText);
  });
}

function numericMetric(result, name, { unknownAllowed = false } = {}) {
  if (unknownAllowed && result[name] === null) return;
  if (typeof result[name] !== 'number' || !Number.isFinite(result[name]) || result[name] < 0) {
    throw new Error(`adapter result missing non-negative numeric ${name}`);
  }
}

export function validateAdapterResult(result, contract, expected) {
  if (!result || typeof result !== 'object' || Array.isArray(result)) throw new Error('adapter result must be an object');
  for (const field of contract.output_required) {
    if (!(field in result)) throw new Error(`adapter result missing ${field}`);
  }
  if (result.adapter !== expected.adapter || result.task_id !== expected.task_id || result.mode !== expected.mode
      || result.execution_mode !== expected.execution_mode || result.concurrency !== expected.concurrency) {
    throw new Error('adapter result identity does not match requested run');
  }
  if (expected.source_ref && result.source_ref !== expected.source_ref) throw new Error(`adapter ${expected.adapter} did not execute pinned source_ref ${expected.source_ref}`);
  if (result.extraction_value !== undefined && (typeof result.extraction_value !== 'string' || result.extraction_value.length > 256)) throw new Error('invalid extraction evidence');
  if (typeof result.success !== 'boolean' || typeof result.independently_verified_success !== 'boolean') {
    throw new Error('success fields must be booleans');
  }
  if (result.codex_turn_failed !== undefined && typeof result.codex_turn_failed !== 'boolean') throw new Error('codex_turn_failed must be a boolean');
  if (result.codex_event_types !== undefined && (typeof result.codex_event_types !== 'string' || result.codex_event_types.length > 256)) throw new Error('codex_event_types must be a bounded string');
  if (result.codex_error_class !== undefined && !['rate_limit','quota','authentication','context_limit','mcp_startup','timeout','provider_unavailable','transport','unclassified_host_failure','mcp_tool_failure','actor_incomplete',null].includes(result.codex_error_class)) throw new Error('codex_error_class is invalid');
  for (const field of ['codex_failed_tool','codex_tool_failure_status','codex_tool_failure_reason','codex_failed_action']) {
    if (result[field] !== undefined && result[field] !== null && (typeof result[field] !== 'string' || !/^[a-zA-Z0-9_-]{1,80}$/.test(result[field]))) throw new Error(`${field} must be a bounded identifier or null`);
  }
  for (const field of [
    'wall_ms','model_round_trips','model_tokens','mcp_calls','browser_ops','observation_bytes','screenshot_bytes',
    'retries','recovery_events','verification_ms','cpu_ms','peak_rss_bytes','focus_disruptions','clipboard_disruptions',
    'human_interventions','severe_wrong_target_events','authority_violations','silent_stale_mutations'
  ]) numericMetric(result, field, { unknownAllowed: field !== 'wall_ms' });
  const derivedMetrics = new Set(['prompt_to_verified_ms', 'prompt_to_final_ms']);
  for (const [field, type] of Object.entries(contract.measurement_metric_types ?? {})) {
    if (derivedMetrics.has(field)) continue;
    if (type === 'number') numericMetric(result, field, { unknownAllowed: true });
    else if (type === 'boolean' && result[field] !== null && typeof result[field] !== 'boolean') {
      throw new Error(`adapter result ${field} must be a boolean or null`);
    } else if (type === 'string' && result[field] !== null
      && (typeof result[field] !== 'string' || result[field].length > 256)) {
      throw new Error(`adapter result ${field} must be a bounded string or null`);
    }
  }
  for (const field of contract.measurement_metric_fields ?? []) {
    const metadata = result.metric_evidence?.[field];
    if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)
      || typeof metadata.provenance !== 'string' || !metadata.provenance.trim()
      || metadata.provenance !== contract.measurement_metric_provenance?.[field]) {
      throw new Error(`adapter result metric_evidence missing provenance for ${field}`);
    }
    if (!derivedMetrics.has(field)) {
      if (!(field in result)) throw new Error(`adapter result missing measurement ${field}`);
      const value = result[field];
      if (value === null) {
        if (typeof metadata.missing_reason !== 'string' || !metadata.missing_reason.trim()
          || metadata.missing_reason.length > 256) {
          throw new Error(`adapter result unavailable ${field} needs a non-empty missing_reason`);
        }
      } else if ('missing_reason' in metadata) {
        throw new Error(`adapter result observed ${field} must not include missing_reason`);
      }
    }
  }
  if (result.cache_status !== null && typeof result.cache_status !== 'string') throw new Error('cache_status must be a string or null');
  if (typeof result.cache_status === 'string' && result.cache_status.length > 128) throw new Error('cache_status exceeds 128 characters');
  for (const field of OPTIONAL_OUTPUT_FIELDS) {
    if (field in result && typeof result[field] === 'string' && result[field].length > (field === 'runner_error' ? 512 : 256)) {
      throw new Error(`adapter result ${field} exceeds its retained evidence limit`);
    }
  }
  return result;
}

function normalizedMetricEvidence(raw, normalized, contract, invocationMetrics) {
  const evidence = {};
  for (const field of contract.measurement_metric_fields ?? []) {
    const metadata = raw.metric_evidence[field];
    const value = normalized[field];
    if (field === 'output_bytes' && Number.isSafeInteger(invocationMetrics.output_bytes)) {
      evidence[field] = { s: 'observed', p: contract.measurement_metric_provenance[field] };
      continue;
    }
    if (value === null || value === undefined) {
      if (typeof metadata.missing_reason !== 'string' || !metadata.missing_reason.trim()) {
        throw new Error(`normalized unavailable ${field} needs a non-empty missing_reason`);
      }
      evidence[field] = { s: 'unavailable', p: metadata.provenance,
        r: metadata.missing_reason.slice(0, 256) };
    } else {
      if ('missing_reason' in metadata) throw new Error(`normalized observed ${field} conflicts with missing_reason`);
      evidence[field] = { s: 'observed', p: metadata.provenance };
    }
  }
  return evidence;
}

function unavailableMetricEvidence(contract, reason) {
  return Object.fromEntries((contract.measurement_metric_fields ?? []).map(field => [field, {
    provenance: contract.measurement_metric_provenance[field],
    missing_reason: reason,
  }]));
}

function compactAdapterOutput(raw, contract) {
  const retained = {};
  for (const field of [...contract.output_required, ...OPTIONAL_OUTPUT_FIELDS]) {
    if (field in raw) retained[field] = raw[field];
  }
  return retained;
}

export function assertBoundedRow(row, paired = false) {
  const bytes = Buffer.byteLength(JSON.stringify(row));
  const limit = paired ? MAX_PAIRED_ROW_BYTES : MAX_REPORT_ROW_BYTES;
  if (bytes > limit) {
    throw new Error(`benchmark evidence row exceeded ${limit} bytes after normalization (${bytes} bytes)`);
  }
  return bytes;
}

async function resetFixture(resetCommand, input, timeoutMs, env) {
  const result = await invokeJsonCommand(resetCommand, { kind: 'reset', ...input }, timeoutMs, env);
  if (result?.ok !== true) throw new Error(`fixture reset failed for ${input.task_id}`);
}

async function independentlyVerify(verifierCommand, input, actingResult, timeoutMs, env) {
  const result = await invokeJsonCommand(verifierCommand, { kind: 'verify', input, acting_result: actingResult }, timeoutMs, env);
  if (typeof result?.passed !== 'boolean' || typeof result?.evidence_hash !== 'string' || result.evidence_hash.length < 8) {
    throw new Error('independent verifier must return {passed:boolean,evidence_hash:string}');
  }
  return result;
}

export function actingAdapterEnvironment(env) {
  const actingEnv = { ...env };
  delete actingEnv.PHASE11_ORACLE_TOKEN;
  delete actingEnv.PHASE11_STATE_FILE;
  return actingEnv;
}

async function runOne({ adapter, command, task, mode, executionMode, concurrency, fixtureBaseUrl, deadlineMs, resetCommand, verifierCommand, contract, environment, env }) {
  const fixtureUrl = task.url ? liveTaskUrl(task) : (() => { const url = new URL(task.fixture, fixtureBaseUrl); url.searchParams.set('task', task.id); if (task.scenario) url.searchParams.set('scenario', task.scenario); return url.toString(); })();
  const acceptance=task.id==='form-guarded-fill'?{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'}:task.id==='guarded-upload'?{kind:'upload',filename:'benchmark.txt',size:Buffer.byteLength('Controlla benchmark\n'),sha256:createHash('sha256').update('Controlla benchmark\n').digest('hex')}:task.id==='structured-extraction'?{kind:'extraction',row:'record-42',status:'older'}:undefined;
  const input = { run_id:randomUUID(), ...(acceptance?{acceptance}:{}), task_id: task.id, task_class: task.class, scenario: task.scenario, mode, execution_mode: executionMode, concurrency, fixture_url: fixtureUrl, site_urls: task.site_urls ?? [fixtureUrl], deadline_ms: deadlineMs, source_ref: adapter.source_ref ?? null };
  await resetFixture(resetCommand, input, deadlineMs, env);
  const adapterRequestStartedNs = process.hrtime.bigint();
  const invocationMetrics = {};
  let raw;
  try {
    const actingEnv = actingAdapterEnvironment(env);
    raw = await invokeJsonCommand(command, { kind: 'run', ...input, environment }, deadlineMs, actingEnv, invocationMetrics);
  } catch (error) {
    const elapsedMs = Number(process.hrtime.bigint() - adapterRequestStartedNs) / 1e6;
    raw = {
      adapter: adapter.id, task_id: task.id, mode, execution_mode: executionMode, concurrency, success: false, independently_verified_success: false,
      ...(adapter.source_ref ? { source_ref: adapter.source_ref } : {}),
      wall_ms: Math.min(deadlineMs, Math.max(0, elapsedMs)), model_round_trips: null, model_tokens: null, mcp_calls: null, browser_ops: null,
      observation_bytes: null, screenshot_bytes: null, retries: null, recovery_events: null, cache_status: null,
      verification_ms: null, cpu_ms: null, peak_rss_bytes: null, focus_disruptions: null, clipboard_disruptions: null,
      human_interventions: null, severe_wrong_target_events: null, authority_violations: null, silent_stale_mutations: null,
      external_controlla_calls: null, model_turns: null, model_visible_observations: null, internal_model_calls: null,
      model_visible_calls: null, browser_commands: null, native_host_commands: null, cdp_commands: null,
      full_ax_reads: null, frame_tree_reads: null, target_rebinds: null, automatic_recoveries: null,
      user_visible_recoveries: null, application_accepted: null, input_commands: null, verification_reads: null,
      auto_attach_calls: null,
      browser_cdp_commands: null, host_round_trips: null, native_host_messages_sent: null, native_host_messages_received: null,
      schema_bytes: null, prompt_input_bytes: null, tool_input_bytes: null, output_bytes: invocationMetrics.output_bytes ?? null,
      unknown_delivery_count: null, fallback_count: null, timeout_count: null, cleanup_completed: null, cleanup_outcome: null,
      metric_evidence: unavailableMetricEvidence(contract, 'adapter_failure'),
      runner_error: error.message,
      prompt_trace: error.partial_prompt_trace ?? null,
    };
    for (const field of ['success', 'independently_verified_success', 'wall_ms']) {
      raw.metric_evidence[field] = { provenance: contract.measurement_metric_provenance[field] };
    }
    if (Number.isSafeInteger(invocationMetrics.output_bytes)) {
      raw.metric_evidence.output_bytes = { provenance: contract.measurement_metric_provenance.output_bytes };
    }
  }
  validateAdapterResult(raw, contract, { adapter: adapter.id, task_id: task.id, mode, execution_mode: executionMode, concurrency, source_ref: adapter.source_ref });
  const verification = await independentlyVerify(verifierCommand, input, raw, deadlineMs, env);
  const verifierDecisionNs = process.hrtime.bigint();
  const promptTrace = raw.prompt_trace && typeof raw.prompt_trace === 'object' ? raw.prompt_trace : {};
  const promptBoundary = promptTrace.prompt_boundary;
  const processStartNs = typeof promptTrace.process_start_monotonic_ns === 'string' && /^\d+$/.test(promptTrace.process_start_monotonic_ns)
    ? BigInt(promptTrace.process_start_monotonic_ns) : null;
  const processEndNs = typeof promptTrace.process_end_monotonic_ns === 'string' && /^\d+$/.test(promptTrace.process_end_monotonic_ns)
    ? BigInt(promptTrace.process_end_monotonic_ns) : null;
  const promptSentNs = typeof promptTrace.prompt_sent_monotonic_ns === 'string' && /^\d+$/.test(promptTrace.prompt_sent_monotonic_ns)
    ? BigInt(promptTrace.prompt_sent_monotonic_ns) : null;
  const exactPromptTrace = promptTrace.trace_valid === true && promptTrace.prompt_sent_observed === true
    && promptBoundary === 'actual_user_submit'
    && promptSentNs !== null && promptSentNs <= verifierDecisionNs
    && promptTrace.clock_domain === 'local_host_monotonic' && promptTrace.clock_offset_ms === 0
    && Number.isFinite(promptTrace.clock_uncertainty_ms) && promptTrace.clock_uncertainty_ms >= 0
    && promptTrace.clock_uncertainty_ms <= 5
    && /^\d+$/.test(String(promptTrace.process_start_monotonic_ns ?? ''))
    && /^\d+$/.test(String(promptTrace.process_end_monotonic_ns ?? ''))
    && BigInt(promptTrace.process_start_monotonic_ns) <= promptSentNs
    && promptSentNs <= BigInt(promptTrace.process_end_monotonic_ns);
  const reportedFinalMs = exactPromptTrace && promptTrace.final_answer_observed === true
    && Number.isFinite(promptTrace.prompt_to_final_ms) ? promptTrace.prompt_to_final_ms : null;
  const processStartToOutcomeMs = processStartNs !== null && processEndNs !== null
    && processStartNs <= processEndNs && processEndNs <= verifierDecisionNs
    ? Number(verifierDecisionNs - processStartNs) / 1_000_000 : null;
  const traceCounters = promptTrace.counters && typeof promptTrace.counters === 'object' ? promptTrace.counters : {};
  const promptToVerifierDecisionMs = exactPromptTrace ? Number(verifierDecisionNs - promptSentNs) / 1_000_000 : null;
  const independentlyVerifiedSuccess = Boolean(verification.passed);
  const success = Boolean(raw.success && independentlyVerifiedSuccess);
  const promptToVerifiedMs = independentlyVerifiedSuccess && exactPromptTrace ? promptToVerifierDecisionMs : null;
  const adapterRequestToVerifierDecisionMs = Number(verifierDecisionNs - adapterRequestStartedNs) / 1_000_000;
  const adapterRequestToVerifiedMs = independentlyVerifiedSuccess ? adapterRequestToVerifierDecisionMs : null;
  const fallbackCount = Number.isSafeInteger(raw.fallback_count) ? raw.fallback_count
    : Number.isSafeInteger(traceCounters.fallback_count) ? traceCounters.fallback_count : null;
  const missingCounterNames = [...new Set([raw.missing_counters, promptTrace.missing_counters, traceCounters.missing_counters]
    .filter(value => typeof value === 'string').flatMap(value => value.split(',').filter(Boolean)))];
  if (fallbackCount === null) missingCounterNames.push('fallback_count');
  const result = {
    ...compactAdapterOutput(raw, contract),
    success,
    prompt_to_verified_ms: promptToVerifiedMs,
    prompt_to_verifier_decision_ms: promptToVerifierDecisionMs,
    prompt_to_final_ms: Number.isFinite(reportedFinalMs) && reportedFinalMs >= 0 ? reportedFinalMs : null,
    process_start_to_outcome_ms: processStartToOutcomeMs,
    adapter_request_to_verified_ms: adapterRequestToVerifiedMs,
    adapter_request_to_verifier_decision_ms: adapterRequestToVerifierDecisionMs,
    external_controlla_calls: raw.external_controlla_calls,
    model_turns: raw.model_turns,
    model_visible_observations: raw.model_visible_observations,
    internal_model_calls: raw.internal_model_calls,
    browser_cdp_commands: raw.browser_cdp_commands,
    host_round_trips: raw.host_round_trips,
    native_host_messages_sent: raw.native_host_messages_sent,
    native_host_messages_received: raw.native_host_messages_received,
    schema_bytes: raw.schema_bytes,
    prompt_input_bytes: raw.prompt_input_bytes,
    tool_input_bytes: raw.tool_input_bytes,
    output_bytes: invocationMetrics.output_bytes ?? raw.output_bytes,
    unknown_delivery_count: raw.unknown_delivery_count,
    fallback_count: raw.fallback_count,
    timeout_count: raw.timeout_count,
    cleanup_completed: raw.cleanup_completed,
    cleanup_outcome: raw.cleanup_outcome,
    prompt_trace: {
      prompt_sent: promptTrace.prompt_sent_observed === true ? true : null,
      prompt_boundary: typeof promptBoundary === 'string' ? promptBoundary : null,
      clock_domain: exactPromptTrace ? promptTrace.clock_domain : null,
      clock_offset_ms: exactPromptTrace ? promptTrace.clock_offset_ms : null,
      clock_uncertainty_ms: exactPromptTrace ? promptTrace.clock_uncertainty_ms : null,
      prompt_sent_monotonic_ns: exactPromptTrace ? promptSentNs.toString() : null,
      process_start_monotonic_ns: exactPromptTrace ? String(promptTrace.process_start_monotonic_ns) : null,
      process_end_monotonic_ns: exactPromptTrace ? String(promptTrace.process_end_monotonic_ns) : null,
      verifier_clock_domain: exactPromptTrace ? 'local_host_monotonic' : null,
      verifier_decision_monotonic_ns: exactPromptTrace ? verifierDecisionNs.toString() : null,
      prompt_sent_observed: promptTrace.prompt_sent_observed === true,
      final_answer_observed: promptTrace.final_answer_observed === true,
      first_tool_decision_ms: exactPromptTrace && Number.isFinite(promptTrace.prompt_to_first_tool_decision_ms) ? promptTrace.prompt_to_first_tool_decision_ms : null,
      controlla_received_ms: exactPromptTrace && Number.isFinite(promptTrace.prompt_to_controlla_received_ms) ? promptTrace.prompt_to_controlla_received_ms : null,
      first_browser_op_ms: exactPromptTrace && Number.isFinite(promptTrace.prompt_to_first_browser_op_ms) ? promptTrace.prompt_to_first_browser_op_ms : null,
      last_browser_op_ms: exactPromptTrace && Number.isFinite(promptTrace.prompt_to_last_browser_op_ms) ? promptTrace.prompt_to_last_browser_op_ms : null,
      tool_result_delivered_ms: exactPromptTrace && Number.isFinite(promptTrace.prompt_to_tool_result_delivered_ms)
        ? promptTrace.prompt_to_tool_result_delivered_ms : null,
      adapter_command_completed_ms: Number.isFinite(invocationMetrics.command_elapsed_ms) ? invocationMetrics.command_elapsed_ms : null,
      partial_counters: raw.partial_counters === true || promptTrace.partial_counters === true || traceCounters.partial_counters === true,
      missing_counters: [...new Set(missingCounterNames)].sort().join(','),
      cold_warm: mode,
      execution_tier: typeof promptTrace.execution_tier === 'string' ? promptTrace.execution_tier : raw.execution_tier ?? null,
      trace_valid: exactPromptTrace,
    },
    independently_verified_success: independentlyVerifiedSuccess,
    verifier_evidence_hash: verification.evidence_hash,
  };
  result.metric_evidence = normalizedMetricEvidence(raw, result, contract, invocationMetrics);
  assertBoundedRow(result);
  return result;
}

export function liveTaskUrl(task) {
  if (typeof task.url !== 'string' || !Array.isArray(task.allowed_hosts)) throw new Error(`live task ${task.id} must declare a URL and exact allowed_hosts`);
  const url = new URL(task.url);
  if (url.protocol !== 'https:' || url.username || url.password || url.hash || !task.allowed_hosts.includes(url.hostname)) {
    throw new Error(`live task ${task.id} URL must be credential-free HTTPS on an exact allowlisted host`);
  }
  return url.toString();
}

function validateLiveTaskUrls(task) {
  liveTaskUrl(task);
  if (task.site_urls === undefined) return;
  if (!Array.isArray(task.site_urls) || !Array.isArray(task.site_allowlists) || task.site_urls.length !== task.site_allowlists.length || task.site_urls.length === 0 || task.site_urls.length > 8) {
    throw new Error(`live task ${task.id} site_urls must have a matching bounded site_allowlists array`);
  }
  for (const [index, raw] of task.site_urls.entries()) {
    const url = new URL(raw);
    const hosts = task.site_allowlists[index];
    if (url.protocol !== 'https:' || url.username || url.password || url.hash || !Array.isArray(hosts) || hosts.length !== 1 || hosts[0] !== url.hostname) {
      throw new Error(`live task ${task.id} site_urls must be credential-free HTTPS on exact per-site host allowlists`);
    }
  }
}

function assertEnvironment(environment, config, selectedAdapters) {
  if (!environment || typeof environment !== 'object' || Array.isArray(environment)) throw new Error('environment manifest is required');
  const required = config.required_environment ?? [];
  for (const field of required) {
    if (!(field in environment) || environment[field] === '' || environment[field] === null || environment[field] === undefined) {
      throw new Error(`environment missing ${field}`);
    }
  }
  for (const field of ['candidate_source_sha']) {
    if (!/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/i.test(environment[field])) throw new Error(`environment ${field} must be a full source hash`);
  }
  for (const field of ['dirty_tree_patch_sha256','runtime_binary_sha256','extension_build_sha256']) {
    if (!/^[a-f0-9]{64}$/i.test(environment[field])) throw new Error(`environment ${field} must be a SHA-256 digest`);
  }
  if (!Number.isSafeInteger(environment.token_budget) || environment.token_budget <= 0) throw new Error('environment token_budget must be a positive integer');
  if (!environment.client_adapter_versions || typeof environment.client_adapter_versions !== 'object'
      || Array.isArray(environment.client_adapter_versions) || Object.keys(environment.client_adapter_versions).length === 0
      || Object.values(environment.client_adapter_versions).some(value => typeof value !== 'string' || !value.trim())) {
    throw new Error('environment client_adapter_versions must identify configured client adapters');
  }
  const configuredAdapters = new Set(config.adapters.map(adapter => adapter.id));
  for (const adapterId of Object.keys(environment.client_adapter_versions)) {
    if (!configuredAdapters.has(adapterId)) throw new Error(`environment contains unknown client adapter version ${adapterId}`);
  }
  for (const adapterId of selectedAdapters) {
    if (typeof environment.client_adapter_versions[adapterId] !== 'string'
        || !environment.client_adapter_versions[adapterId].trim()) {
      throw new Error(`environment client_adapter_versions missing selected adapter ${adapterId}`);
    }
  }
}

export async function runBenchmark({
  candidateId = 'controlla',
  baselineId,
  fixtureBaseUrl,
  commands,
  resetCommand,
  verifierCommand,
  environment,
  modes,
  concurrencyLevels,
  repetitions,
  warmupRepetitions,
  executionModes,
  maxReportBytes = MAX_REPORT_BYTES,
  onRow,
  deadlineMs = 60_000,
  env = process.env,
  configPath = 'bench/tasks/phase11-realistic.json',
} = {}) {
  const config = await loadPhase11Config(configPath);
  const contract = await loadAdapterContract();
  const candidate = config.adapters.find(adapter => adapter.id === candidateId);
  const baseline = config.adapters.find(adapter => adapter.id === baselineId);
  if (!candidate || !baseline) throw new Error(`unknown baseline ${baselineId}`);
  if (baseline.track !== 'structured') throw new Error('release comparison requires a structured baseline; computer-use stays on the separate vision track');
  if (!fixtureBaseUrl && config.tasks.some(task => !task.url)) throw new Error('fixtureBaseUrl is required for fixture tasks');
  for (const task of config.tasks) if (task.url) validateLiveTaskUrls(task);
  assertEnvironment(environment, config, [candidateId, baselineId]);
  const selectedModes = modes ?? config.modes;
  const selectedExecutionModes = executionModes ?? config.execution_modes;
  const selectedConcurrency = concurrencyLevels ?? config.concurrency;
  const selectedRepetitions = repetitions ?? config.repetitions ?? 1;
  const selectedWarmupRepetitions = warmupRepetitions ?? config.warmup_repetitions ?? 0;
  const randomSeed = Number.isSafeInteger(config.random_seed) && config.random_seed >= 0 ? config.random_seed : 1;
  const random = seededRandom(randomSeed);
  if (!Number.isInteger(selectedRepetitions) || selectedRepetitions < 1 || selectedRepetitions > 100) throw new Error('repetitions must be an integer from 1 to 100');
  if (!Number.isInteger(selectedWarmupRepetitions) || selectedWarmupRepetitions < 0 || selectedWarmupRepetitions > 20) throw new Error('warmupRepetitions must be an integer from 0 to 20');
  for (const mode of selectedModes) if (!config.modes.includes(mode)) throw new Error(`unsupported mode ${mode}`);
  for (const executionMode of selectedExecutionModes) if (!config.execution_modes.includes(executionMode)) throw new Error(`unsupported execution mode ${executionMode}`);
  for (const value of selectedConcurrency) if (!config.concurrency.includes(value)) throw new Error(`unsupported concurrency ${value}`);
  if (selectedConcurrency.some(value => value > 1)) throw new Error('concurrency levels above 1 are unsupported: the runner executes paired adapter commands sequentially');
  const candidateCommand = commands?.[candidateId];
  const baselineCommand = commands?.[baselineId];
  if (!candidateCommand || !baselineCommand || !resetCommand || !verifierCommand) throw new Error('candidate, baseline, reset, and verifier commands are required');

  const warmupRows = [];
  let reportBytes = 0;
  const warmupCases = [];
  for (let replicate = 0; replicate < selectedWarmupRepetitions; replicate++) {
    for (const task of config.tasks) for (const mode of selectedModes) for (const executionMode of selectedExecutionModes) for (const concurrency of selectedConcurrency) {
      warmupCases.push({ replicate: replicate + 1, task, mode, executionMode, concurrency });
    }
  }
  shuffle(warmupCases, random);
  for (const trial of warmupCases) {
    const { replicate, task, mode, executionMode, concurrency } = trial;
    const common = { task, mode, executionMode, concurrency, fixtureBaseUrl, deadlineMs, resetCommand, verifierCommand, contract, environment, env };
    const candidateFirst = random() < 0.5;
    const candidatePromise = () => runOne({ ...common, adapter: candidate, command: candidateCommand });
    const baselinePromise = () => runOne({ ...common, adapter: baseline, command: baselineCommand });
    let candidateResult;
    let baselineResult;
    if (candidateFirst) {
      candidateResult = await candidatePromise();
      baselineResult = await baselinePromise();
    } else {
      baselineResult = await baselinePromise();
      candidateResult = await candidatePromise();
    }
    const row = { warmup: true, replicate, task_id: task.id, mode, execution_mode: executionMode, concurrency,
      candidate_order: candidateFirst ? 'first' : 'second', candidate: candidateResult, baseline: baselineResult };
    reportBytes += assertBoundedRow(row, true);
    if (reportBytes > maxReportBytes) throw new Error(`benchmark report exceeded ${maxReportBytes} bytes`);
    warmupRows.push(row);
    if (onRow) await onRow(row);
  }
  const cases = [];
  for (let replicate = 0; replicate < selectedRepetitions; replicate++) {
    for (const task of config.tasks) {
      for (const mode of selectedModes) {
        for (const executionMode of selectedExecutionModes) for (const concurrency of selectedConcurrency) {
          cases.push({ replicate: replicate + 1, task, mode, executionMode, concurrency });
        }
      }
    }
  }
  shuffle(cases, random);
  const pairedRows = [];
  for (const trial of cases) {
        const { replicate, task, mode, executionMode, concurrency } = trial;
        const common = { task, mode, executionMode, concurrency, fixtureBaseUrl, deadlineMs, resetCommand, verifierCommand, contract, environment, env };
        const candidateFirst = random() < 0.5;
        const runCandidate = () => runOne({ ...common, adapter: candidate, command: candidateCommand });
        const runBaseline = () => runOne({ ...common, adapter: baseline, command: baselineCommand });
        let candidateResult;
        let baselineResult;
        if (candidateFirst) {
          candidateResult = await runCandidate();
          baselineResult = await runBaseline();
        } else {
          baselineResult = await runBaseline();
          candidateResult = await runCandidate();
        }
        const row = { replicate, task_id: task.id, mode, execution_mode: executionMode, concurrency,
          candidate_order: candidateFirst ? 'first' : 'second', candidate: candidateResult, baseline: baselineResult };
        reportBytes += assertBoundedRow(row, true);
        if (reportBytes > maxReportBytes) throw new Error(`benchmark report exceeded ${maxReportBytes} bytes`);
        pairedRows.push(row);
        if (onRow) await onRow(row);
    }
  const report = {
    schema_version: 2,
    track: config.track,
    candidate_adapter: candidate.id,
    baseline_adapter: baseline.id,
    randomization: { method: 'seeded_paired_order', seed: randomSeed },
    environment,
    warmup_rows: warmupRows,
    paired_rows: pairedRows,
  };
  const serializedBytes = Buffer.byteLength(JSON.stringify(report));
  if (serializedBytes > maxReportBytes) throw new Error(`benchmark report exceeded ${maxReportBytes} bytes`);
  return report;
}

function shuffle(items, random) {
  for (let index = items.length - 1; index > 0; index--) {
    const other = Math.floor(random() * (index + 1));
    [items[index], items[other]] = [items[other], items[index]];
  }
}

function seededRandom(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6D2B79F5) | 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function argValue(name) {
  const prefix = `--${name}=`;
  return process.argv.find(arg => arg.startsWith(prefix))?.slice(prefix.length);
}

async function main() {
  const baselineId = argValue('baseline');
  const fixtureBaseUrl = process.env.PHASE11_FIXTURE_BASE_URL;
  const configPath = argValue('task-set') ?? process.env.PHASE11_TASK_SET ?? 'bench/tasks/phase11-realistic.json';
  const environment = JSON.parse(process.env.PHASE11_ENVIRONMENT_JSON ?? '{}');
  const deadlineMs = Number(argValue('deadline-ms') ?? 60_000);
  const modes = argValue('modes')?.split(',').filter(Boolean);
  const concurrencyLevels = argValue('concurrency')?.split(',').filter(Boolean).map(Number);
  const repetitions = argValue('repetitions') === undefined ? undefined : Number(argValue('repetitions'));
  const stamp = new Date().toISOString().replaceAll(/[:.]/g, '-');
  const output = resolve(root, argValue('output') ?? `bench/reports/phase11-${baselineId}-${stamp}.json`);
  if (!baselineId) throw new Error('--baseline=<structured-adapter-id> is required');
  const commands = {
    [argValue('candidate') ?? 'controlla']: parseCommand(process.env[commandEnvName(argValue('candidate') ?? 'controlla')], commandEnvName(argValue('candidate') ?? 'controlla')),
    [baselineId]: parseCommand(process.env[commandEnvName(baselineId)], commandEnvName(baselineId)),
  };
  const resetCommand = parseCommand(process.env.PHASE11_RESET_COMMAND_JSON, 'PHASE11_RESET_COMMAND_JSON');
  const verifierCommand = parseCommand(process.env.PHASE11_VERIFIER_COMMAND_JSON, 'PHASE11_VERIFIER_COMMAND_JSON');
  const report = await runBenchmark({ candidateId: argValue('candidate') ?? 'controlla', baselineId, fixtureBaseUrl, commands, resetCommand, verifierCommand, environment, modes, concurrencyLevels, repetitions, deadlineMs, configPath });
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(report)}\n`, { flag: 'wx' });
  console.log(`wrote ${report.paired_rows.length} paired Phase 11 rows to ${output}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exit(1); });
}
