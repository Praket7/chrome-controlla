#!/usr/bin/env node
// Optional transparent stdio proxy for prompt-to-tool wall-time measurement.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { recordPromptTraceEvent } from './prompt-send-trace.mjs';

const TRACE_FILE = process.env.CONTROLLA_PROMPT_TRACE_FILE;
const TRACE_ID = process.env.CONTROLLA_PROMPT_TRACE_ID;
const MODEL_CALLS = new Set(['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow']);
const pending = new Map();
let outputBuffer = Buffer.alloc(0);
let traceQueue = Promise.resolve();
let traceError = null;

function trace(event, fields = {}) {
  if (!TRACE_FILE || !TRACE_ID) return;
  const happenedAt = process.hrtime.bigint();
  traceQueue = traceQueue.then(() => recordPromptTraceEvent(TRACE_FILE, TRACE_ID, event, fields, happenedAt))
    .catch(error => { traceError ??= error; });
}

function parseJsonLine(line) {
  try { return JSON.parse(line); } catch { return null; }
}

function collectValues(value, predicate, out = []) {
  if (Array.isArray(value)) { for (const child of value) collectValues(child, predicate, out); return out; }
  if (!value || typeof value !== 'object') return out;
  if (predicate(value)) out.push(value);
  for (const child of Object.values(value)) collectValues(child, predicate, out);
  return out;
}

function contentValues(result) {
  const texts = result?.content?.filter?.(item => item.type === 'text').map(item => item.text) ?? [];
  const parsed = texts.flatMap(text => { try { return [JSON.parse(text)]; } catch { return []; } });
  return [result?.structuredContent, result, ...parsed].filter(Boolean);
}

function uniqueObservedCounter(values) {
  const reported = values.filter(value => value !== undefined && value !== null);
  if (reported.some(value => !Number.isSafeInteger(value) || value < 0)) return null;
  const observed = reported;
  const distinct = [...new Set(observed)];
  return distinct.length === 1 ? distinct[0] : null;
}

export function summaryFields(result) {
  const roots = contentValues(result);
  const metrics = collectValues(roots, value => value.metrics && typeof value.metrics === 'object').map(value => value.metrics);
  const cdp = uniqueObservedCounter(metrics.flatMap(item => [item.browser?.cdp_commands, item.cdp_commands]));
  const reportedInternal = collectValues(roots, value => Number.isSafeInteger(value.execution_route?.actual_cost?.reported_internal_model_calls))
    .map(value => value.execution_route.actual_cost.reported_internal_model_calls);
  const internalCounts = [...metrics.map(item => item.internal_model_calls).filter(Number.isSafeInteger), ...reportedInternal];
  const internal = uniqueObservedCounter(internalCounts);
  const fallbackCount = uniqueObservedCounter(collectValues(roots,
    value => Number.isSafeInteger(value.fallback_count) && value.fallback_count >= 0)
    .map(value => value.fallback_count));
  const tier = collectValues(roots, value => value.execution_route?.selected || value.execution_route?.tier)
    .map(value => value.execution_route.selected ?? value.execution_route.tier).find(value => typeof value === 'string');
  const metric = key => uniqueObservedCounter(metrics.map(item => item[key]));
  const phaseNames = ['queue_us','startup_us','pair_us','attach_us','readiness_us','discovery_us',
    'snapshot_us','grounding_us','dispatch_us','verification_us','artifact_transfer_us','serialization_us'];
  const phaseValues = {};
  const missingPhases = [];
  for (const name of phaseNames) {
    const values = metrics.map(item => item.phases?.[name] ?? item[name]);
    const observed = values.some(value => value !== undefined && value !== null);
    const phase = uniqueObservedCounter(values);
    if (phase !== null) phaseValues[`runtime_${name}`] = phase;
    else if (observed || values.length === 0 || values.some(value => value === undefined || value === null)) {
      missingPhases.push(`runtime_${name}`);
    }
  }
  const requiredCounters = ['host_round_trips','cdp_commands','observation_bytes','screenshot_bytes','fallback_count','internal_model_calls'];
  const missingCounters = requiredCounters.filter(name => !Number.isSafeInteger(name === 'cdp_commands' ? cdp
    : name === 'fallback_count' ? fallbackCount
      : name === 'internal_model_calls' ? internal : metric(name)));
  return { ...(Number.isSafeInteger(cdp) ? { cdp_commands: cdp } : {}),
    ...(Number.isSafeInteger(internal) ? { internal_model_calls: internal } : {}),
    ...(typeof metrics.find(item => typeof item.operation_id === 'string')?.operation_id === 'string'
      ? { mcp_operation_id: metrics.find(item => typeof item.operation_id === 'string').operation_id } : {}),
    ...(Number.isSafeInteger(metric('host_round_trips')) ? { host_round_trips: metric('host_round_trips') } : {}),
    ...(Number.isSafeInteger(metric('native_host_messages_sent')) ? { native_host_messages_sent: metric('native_host_messages_sent') } : {}),
    ...(Number.isSafeInteger(metric('native_host_messages_received')) ? { native_host_messages_received: metric('native_host_messages_received') } : {}),
    ...(Number.isSafeInteger(metric('first_browser_op_us')) ? { first_browser_op_us: metric('first_browser_op_us') } : {}),
    ...(Number.isSafeInteger(metric('last_browser_op_us')) ? { last_browser_op_us: metric('last_browser_op_us') } : {}),
    ...(Number.isSafeInteger(metric('observation_bytes')) ? { observation_bytes: metric('observation_bytes') } : {}),
    ...(Number.isSafeInteger(metric('screenshot_bytes')) ? { screenshot_bytes: metric('screenshot_bytes') } : {}),
    ...(tier ? { selected_execution_tier: tier } : {}), fallback_count: fallbackCount ?? null,
    partial_counters: missingCounters.length > 0,
    missing_counters: missingCounters.join(','), missing_phases: missingPhases.join(','), ...phaseValues };
}

function observeServerLine(line, bytes) {
  const message = parseJsonLine(line);
  if (!message || typeof message !== 'object') return;
  if (message.method === 'tools/list' && message.id !== undefined) {
    pending.set(String(message.id), { kind: 'schema' });
    return;
  }
  if (message.method !== 'tools/call' || message.id === undefined) return;
  const name = message.params?.name;
  if (typeof name !== 'string' || !MODEL_CALLS.has(name)) return;
  const callId = String(message.id);
  pending.set(callId, { kind: 'tool', name, call_id: callId });
  trace('controlla_received', { tool: name, call_id: callId, tool_input_bytes: bytes,
    ...(process.env.CONTROLLA_PROMPT_COLD_WARM ? { cold_warm: process.env.CONTROLLA_PROMPT_COLD_WARM } : {}) });
}

function inspectServerChunk(chunk) {
  outputBuffer = Buffer.concat([outputBuffer, chunk]);
  while (true) {
    const newline = outputBuffer.indexOf(0x0a);
    if (newline < 0) break;
    const lineBytes = outputBuffer.subarray(0, newline);
    outputBuffer = outputBuffer.subarray(newline + 1);
    const line = lineBytes.toString('utf8').replace(/\r$/, '');
    const response = parseJsonLine(line);
    const isResponse = response && response.id !== undefined && pending.has(String(response.id));
    if (isResponse) {
      const context = pending.get(String(response.id));
      if (context.kind === 'schema') {
        pending.delete(String(response.id));
        trace('schema_delivered', { schema_bytes: lineBytes.length + 1, output_bytes: lineBytes.length + 1 });
        process.stdout.write(Buffer.concat([lineBytes, Buffer.from('\n')]));
      } else {
        pending.delete(String(response.id));
        const fields = summaryFields(response.result);
        process.stdout.write(Buffer.concat([lineBytes, Buffer.from('\n')]), () => trace('tool_result_delivered', {
          output_bytes: lineBytes.length + 1,
          call_id: context.call_id,
          ...(fields.mcp_operation_id ? { mcp_operation_id: fields.mcp_operation_id } : {}),
          ...(fields.selected_execution_tier ? { selected_execution_tier: fields.selected_execution_tier } : {}),
          ...(Number.isSafeInteger(fields.cdp_commands) ? { cdp_commands: fields.cdp_commands } : {}),
          ...(Number.isSafeInteger(fields.native_host_messages_sent) ? { native_host_messages_sent: fields.native_host_messages_sent } : {}),
          ...(Number.isSafeInteger(fields.native_host_messages_received) ? { native_host_messages_received: fields.native_host_messages_received } : {}),
          ...(Number.isSafeInteger(fields.first_browser_op_us) ? { first_browser_op_us: fields.first_browser_op_us } : {}),
          ...(Number.isSafeInteger(fields.last_browser_op_us) ? { last_browser_op_us: fields.last_browser_op_us } : {}),
          ...(Number.isSafeInteger(fields.first_browser_op_us) || Number.isSafeInteger(fields.last_browser_op_us)
            ? { browser_offset_clock_domain: 'mcp_operation_relative' } : {}),
          ...(Number.isSafeInteger(fields.internal_model_calls) ? { internal_model_calls: fields.internal_model_calls } : {}),
          ...(Number.isSafeInteger(fields.host_round_trips) ? { host_round_trips: fields.host_round_trips } : {}),
          ...(Number.isSafeInteger(fields.observation_bytes) ? { observation_bytes: fields.observation_bytes } : {}),
          ...(Number.isSafeInteger(fields.screenshot_bytes) ? { screenshot_bytes: fields.screenshot_bytes } : {}),
          partial_counters: fields.partial_counters,
          missing_counters: fields.missing_counters,
          missing_phases: fields.missing_phases,
          ...Object.fromEntries(Object.entries(fields).filter(([key, value]) => key.startsWith('runtime_') && Number.isSafeInteger(value))),
          fallback_count: fields.fallback_count,
        }));
      }
    } else {
      process.stdout.write(Buffer.concat([lineBytes, Buffer.from('\n')]));
    }
  }
  if (outputBuffer.length > 1024 * 1024) {
    process.stderr.write('Controlla trace proxy stopped parsing an oversized MCP line.\n');
    outputBuffer = Buffer.alloc(0);
  }
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  if (!command) throw new Error('usage: mcp-stdio-trace-proxy <mcp-command> [args...]');
  const child = spawn(command, args, { stdio: ['pipe', 'pipe', 'inherit'], env: process.env, windowsHide: true });
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
  input.on('line', line => {
    const bytes = Buffer.byteLength(line) + 1;
    observeServerLine(line, bytes);
    child.stdin.write(`${line}\n`);
  });
  child.stdout.on('data', inspectServerChunk);
  process.stdin.on('end', () => child.stdin.end());
  child.once('error', error => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
  child.once('close', async code => {
    await traceQueue;
    if (traceError) process.stderr.write(`Prompt trace error: ${traceError.message}\n`);
    if (outputBuffer.length) process.stdout.write(outputBuffer);
    process.exitCode = code ?? 1;
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { process.stderr.write(`${error.stack ?? error.message}\n`); process.exitCode = 2; });
}
