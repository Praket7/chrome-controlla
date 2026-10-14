import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { join, resolve } from 'node:path';
import { readPromptTrace, summarizePromptTrace, recordPromptTraceEvent } from './prompt-send-trace.mjs';

test('transparent stdio proxy records receive and delivered-result spans without claiming independent verification', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-1';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { input_bytes: 9, prompt_boundary: 'client_stdin_write' });
    const child = spawn(process.execPath, [proxy, process.execPath, server], {
      env: { ...process.env, CONTROLLA_PROMPT_TRACE_FILE: tracePath, CONTROLLA_PROMPT_TRACE_ID: traceId },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on('data', chunk => stdout.push(chunk));
    child.stderr.on('data', chunk => stderr.push(chunk));
    child.stdin.end(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} })}\n${JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'browser', arguments: { action: 'task' } } })}\n`);
    const exit = await new Promise((resolveExit, reject) => {
      child.once('error', reject);
      child.once('close', code => resolveExit(code));
    });
    assert.equal(exit, 0, Buffer.concat(stderr).toString());
    assert.equal(Buffer.concat(stdout).toString().split('\n').filter(Boolean).length, 2);
    const events = await readPromptTrace(tracePath);
    assert.ok(events.some(event => event.event === 'controlla_received' && event.tool === 'browser'));
    assert.ok(events.some(event => event.event === 'browser_operation'));
    assert.equal(events.some(event => event.event === 'verified_outcome'), false, 'tool receipts are not the independent benchmark verifier');
    assert.ok(events.some(event => event.event === 'schema_delivered' && event.schema_bytes > 0));
    const delivered = events.find(event => event.event === 'tool_result_delivered');
    assert.ok(delivered && delivered.output_bytes > 0);
    assert.equal(delivered.schema_bytes, undefined, 'tool-result timing must not end at tools/list schema delivery');
    const summary = summarizePromptTrace(events);
    assert.equal(summary.counters.external_controlla_calls, 1);
    assert.equal(summary.counters.browser_cdp_commands, 3);
    assert.equal(summary.counters.internal_model_calls, 0);
    assert.equal(summary.execution_tier, 'qualified_skill');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('multiple tool results, including a completed V3 task receipt, stay valid without acting as independent verification', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-multiple';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'client_stdin_write' });
    const child = spawn(process.execPath, [proxy, process.execPath, server], {
      env: { ...process.env, CONTROLLA_PROMPT_TRACE_FILE: tracePath, CONTROLLA_PROMPT_TRACE_ID: traceId, TRACE_FIXTURE_MULTI_CALL: '1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    child.stdout.resume(); child.stderr.resume();
    child.stdin.end([
      { jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'snapshot', arguments: { action: 'read' } } },
      { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'browser', arguments: { action: 'task' } } },
    ].map(value => JSON.stringify(value)).join('\n') + '\n');
    const exit = await new Promise((resolveExit, reject) => { child.once('error', reject); child.once('close', resolveExit); });
    assert.equal(exit, 0);
    const events = await readPromptTrace(tracePath);
    assert.equal(events.filter(event => event.event === 'tool_result_delivered').length, 2);
    assert.equal(events.some(event => event.event === 'verified_outcome'), false);
    const summary = summarizePromptTrace(events);
    assert.equal(summary.trace_valid, true);
    assert.equal(summary.counters.external_controlla_calls, 2);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('partial workflow with an earlier verified receipt is not traced as a verified outcome', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-partial';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'client_stdin_write' });
    const child = spawn(process.execPath, [proxy, process.execPath, server], {
      env: { ...process.env, CONTROLLA_PROMPT_TRACE_FILE: tracePath, CONTROLLA_PROMPT_TRACE_ID: traceId, TRACE_FIXTURE_PARTIAL: '1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    child.stdout.resume(); child.stderr.resume();
    child.stdin.end(`${JSON.stringify({ jsonrpc: '2.0', id: 7, method: 'tools/call', params: { name: 'workflow', arguments: {} } })}\n`);
    const exit = await new Promise((resolveExit, reject) => { child.once('error', reject); child.once('close', resolveExit); });
    assert.equal(exit, 0);
    const events = await readPromptTrace(tracePath);
    assert.equal(events.some(event => event.event === 'verified_outcome'), false);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('unavailable browser-command telemetry stays null instead of becoming a false zero', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-no-metrics';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'client_stdin_write' });
    const child = spawn(process.execPath, [proxy, process.execPath, server], {
      env: { ...process.env, CONTROLLA_PROMPT_TRACE_FILE: tracePath, CONTROLLA_PROMPT_TRACE_ID: traceId, TRACE_FIXTURE_NO_METRICS: '1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    child.stdout.resume();
    child.stderr.resume();
    child.stdin.end(`${JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'browser', arguments: { action: 'task' } } })}\n`);
    const exit = await new Promise((resolveExit, reject) => {
      child.once('error', reject);
      child.once('close', resolveExit);
    });
    assert.equal(exit, 0);
    const summary = summarizePromptTrace(await readPromptTrace(tracePath));
    assert.equal(summary.counters.browser_cdp_commands, null);
    assert.equal(summary.counters.internal_model_calls, null);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
