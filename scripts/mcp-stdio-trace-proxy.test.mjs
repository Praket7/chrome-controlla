import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { join, resolve } from 'node:path';
import { readPromptTrace, summarizePromptTrace, recordPromptTraceEvent } from './prompt-send-trace.mjs';

test('transparent stdio proxy records receive, browser, verification, and delivered-result spans', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-1';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { input_bytes: 9 });
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
    assert.ok(events.some(event => event.event === 'verified_outcome' && event.selected_execution_tier === 'qualified_skill'));
    assert.ok(events.some(event => event.event === 'tool_result_delivered' && event.schema_bytes > 0));
    const summary = summarizePromptTrace(events);
    assert.equal(summary.counters.external_controlla_calls, 1);
    assert.equal(summary.counters.browser_cdp_commands, 3);
    assert.equal(summary.counters.internal_model_calls, 0);
    assert.equal(summary.execution_tier, 'qualified_skill');
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
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent');
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
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
