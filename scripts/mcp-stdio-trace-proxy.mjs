#!/usr/bin/env node
// Optional transparent stdio proxy for prompt-to-tool wall-time measurement.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
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

function summaryFields(result) {
  const roots = contentValues(result);
  const metrics = collectValues(roots, value => value.metrics && typeof value.metrics === 'object').map(value => value.metrics);
  const cdp = metrics.map(item => item.browser?.cdp_commands ?? item.cdp_commands).find(Number.isSafeInteger);
  const reportedInternal = collectValues(roots, value => Number.isSafeInteger(value.execution_route?.actual_cost?.reported_internal_model_calls))
    .map(value => value.execution_route.actual_cost.reported_internal_model_calls).find(Number.isSafeInteger);
  const internal = metrics.map(item => item.internal_model_calls).find(Number.isSafeInteger) ?? reportedInternal;
  const tier = collectValues(roots, value => value.execution_route?.selected || value.execution_route?.tier)
    .map(value => value.execution_route.selected ?? value.execution_route.tier).find(value => typeof value === 'string');
  const fallbacks = collectValues(roots, value => typeof value.fallback_reason === 'string').length;
  const terminal = roots.find(value => value && typeof value === 'object' && (value.status || value.workflow)) ?? {};
  const workflow = terminal.workflow && typeof terminal.workflow === 'object' ? terminal.workflow : terminal;
  const receipts = Array.isArray(workflow.receipts) ? workflow.receipts : [];
  const terminalReceipt = receipts.at(-1);
  const containsFailure = receipts.some(receipt => ['failed', 'unknown', 'not_dispatched', 'incomplete', 'unsupported'].includes(String(receipt?.status ?? '').toLowerCase()));
  const verified = !containsFailure && (terminal.workflow
    ? workflow.status === 'completed' && terminalReceipt?.step === 'verify' && terminalReceipt.status === 'passed'
    : terminal.status === 'verified' || terminal.verified === true || terminal.status === 'passed' && !!terminal.predicate);
  return { ...(Number.isSafeInteger(cdp) ? { cdp_commands: cdp } : {}),
    ...(Number.isSafeInteger(internal) ? { internal_model_calls: internal } : {}),
    ...(tier ? { selected_execution_tier: tier } : {}), fallback_count: fallbacks, verified };
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
  pending.set(String(message.id), { kind: 'tool', name });
  trace('controlla_received', { tool: name, input_bytes: bytes });
  trace('browser_operation', { tool: name, input_bytes: bytes,
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
        if (fields.verified) trace('verified_outcome', { selected_execution_tier: fields.selected_execution_tier ?? 'unreported',
          cdp_commands: fields.cdp_commands, ...(Number.isSafeInteger(fields.internal_model_calls) ? { internal_model_calls: fields.internal_model_calls } : {}), fallback_count: fields.fallback_count });
        process.stdout.write(Buffer.concat([lineBytes, Buffer.from('\n')]), () => trace('tool_result_delivered', { output_bytes: lineBytes.length + 1 }));
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

main().catch(error => { process.stderr.write(`${error.stack ?? error.message}\n`); process.exitCode = 2; });
