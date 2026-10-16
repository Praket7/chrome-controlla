import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { join, resolve } from 'node:path';
import { readPromptTrace, summarizePromptTrace, recordPromptTraceEvent } from './prompt-send-trace.mjs';
import { summaryFields } from './mcp-stdio-trace-proxy.mjs';

test('proxy metric extraction rejects conflicting values across structured and textual receipts', () => {
  const fields = summaryFields({
    structuredContent: { metrics: { browser: { cdp_commands: 4 }, host_round_trips: 2,
      observation_bytes: 100, screenshot_bytes: 20, internal_model_calls: 0 }, fallback_count: 0 },
    content: [{ type: 'text', text: JSON.stringify({ metrics: { browser: { cdp_commands: 5 },
      host_round_trips: 2, observation_bytes: 100, screenshot_bytes: 20, internal_model_calls: 0 }, fallback_count: 0 }) }],
  });
  assert.equal(fields.cdp_commands, undefined);
  assert.equal(fields.host_round_trips, 2);
  assert.equal(fields.partial_counters, true);
  assert.ok(fields.missing_counters.split(',').includes('cdp_commands'));
});

test('proxy rejects nested/top-level CDP conflicts and conflicting or negative phase durations', () => {
  const fields = summaryFields({ metrics: {
    browser: { cdp_commands: 4 }, cdp_commands: 5, host_round_trips: 2,
    observation_bytes: 100, screenshot_bytes: 20, internal_model_calls: 0,
    phases: { queue_us: 10, startup_us: -1 },
  }, fallback_count: 0, content: [{ type: 'text', text: JSON.stringify({ metrics: {
    browser: { cdp_commands: 4 }, cdp_commands: 5, host_round_trips: 2,
    observation_bytes: 100, screenshot_bytes: 20, internal_model_calls: 0,
    phases: { queue_us: 11, startup_us: 3 },
  }, fallback_count: 0 }) }] });
  assert.equal(fields.cdp_commands, undefined);
  assert.equal(fields.partial_counters, true);
  assert.ok(fields.missing_counters.split(',').includes('cdp_commands'));
  assert.equal(fields.runtime_queue_us, undefined, 'conflicting duplicate phase durations are not selected');
  assert.equal(fields.runtime_startup_us, undefined, 'negative phase durations are rejected');
  assert.ok(fields.missing_phases.split(',').includes('runtime_queue_us'));
  assert.ok(fields.missing_phases.split(',').includes('runtime_startup_us'));
});

test('transparent stdio proxy records receive and delivered-result spans without claiming independent verification', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-1';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_input_bytes: 9, prompt_boundary: 'actual_user_submit' });
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
    assert.equal(events.some(event => event.event === 'browser_operation'), false,
      'the proxy observes tool arrival, not the actual browser dispatch boundary');
    assert.equal(events.some(event => event.event === 'verified_outcome'), false, 'tool receipts are not the independent benchmark verifier');
    assert.ok(events.some(event => event.event === 'schema_delivered' && event.schema_bytes > 0));
    const delivered = events.find(event => event.event === 'tool_result_delivered');
    assert.ok(delivered && delivered.output_bytes > 0);
    assert.equal(delivered.schema_bytes, undefined, 'tool-result timing must not end at tools/list schema delivery');
    assert.equal(delivered.partial_counters, false);
    assert.equal(delivered.host_round_trips, 2);
    assert.equal(delivered.mcp_operation_id, 'mcp:fixture:1');
    assert.equal(delivered.native_host_messages_sent, 2);
    assert.equal(delivered.native_host_messages_received, 2);
    assert.equal(delivered.first_browser_op_us, 30);
    assert.equal(delivered.last_browser_op_us, 80);
    assert.equal(delivered.browser_offset_clock_domain, 'mcp_operation_relative');
    assert.equal(delivered.observation_bytes, 700);
    assert.equal(delivered.runtime_queue_us, 10);
    assert.equal(delivered.runtime_readiness_us, 20);
    assert.ok(delivered.missing_phases.split(',').includes('runtime_startup_us'),
      'unmeasured startup is explicit missingness rather than a measured zero');
    assert.ok(delivered.missing_phases.split(',').includes('runtime_attach_us'),
      'unmeasured attach is explicit missingness rather than a measured zero');
    const summary = summarizePromptTrace(events);
    assert.equal(summary.counters.external_controlla_calls, 1);
    assert.equal(summary.counters.browser_cdp_commands, 3);
    assert.equal(summary.counters.internal_model_calls, 0);
    assert.equal(summary.execution_tier, 'qualified_skill');
    assert.equal(summary.prompt_to_first_browser_op_ms, null);
    assert.equal(summary.prompt_to_last_browser_op_ms, null);
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
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'actual_user_submit' });
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
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'actual_user_submit' });
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
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'actual_user_submit' });
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
    assert.equal(summary.counters.fallback_count, null);
    assert.ok(summary.counters.missing_counters.split(',').includes('fallback_count'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('fallback telemetry uses the explicit counter once across duplicate structured and text receipts', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-fallback-count';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'actual_user_submit' });
    const child = spawn(process.execPath, [proxy, process.execPath, server], {
      env: { ...process.env, CONTROLLA_PROMPT_TRACE_FILE: tracePath, CONTROLLA_PROMPT_TRACE_ID: traceId, TRACE_FIXTURE_DUPLICATE_FALLBACK: '1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    child.stdout.resume();
    child.stderr.resume();
    child.stdin.end(`${JSON.stringify({ jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'workflow', arguments: {} } })}\n`);
    const exit = await new Promise((resolveExit, reject) => {
      child.once('error', reject);
      child.once('close', resolveExit);
    });
    assert.equal(exit, 0);
    const events = await readPromptTrace(tracePath);
    const delivered = events.find(event => event.event === 'tool_result_delivered');
    assert.equal(delivered.fallback_count, 1);
    assert.equal(delivered.partial_counters, false);
    assert.equal(summarizePromptTrace(events).counters.fallback_count, 1);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('conflicting fallback counters remain unknown instead of selecting a receipt traversal value', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-conflicting-fallback-count';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    await recordPromptTraceEvent(tracePath, traceId, 'prompt_sent', { prompt_boundary: 'actual_user_submit' });
    const child = spawn(process.execPath, [proxy, process.execPath, server], {
      env: { ...process.env, CONTROLLA_PROMPT_TRACE_FILE: tracePath, CONTROLLA_PROMPT_TRACE_ID: traceId,
        TRACE_FIXTURE_DUPLICATE_FALLBACK: '1', TRACE_FIXTURE_CONFLICTING_FALLBACK: '1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    child.stdout.resume();
    child.stderr.resume();
    child.stdin.end(`${JSON.stringify({ jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'workflow', arguments: {} } })}\n`);
    const exit = await new Promise((resolveExit, reject) => {
      child.once('error', reject);
      child.once('close', resolveExit);
    });
    assert.equal(exit, 0);
    const events = await readPromptTrace(tracePath);
    const delivered = events.find(event => event.event === 'tool_result_delivered');
    assert.equal(delivered.fallback_count, undefined, 'untrusted missing/conflicting counter is not serialized as zero');
    assert.equal(delivered.partial_counters, true);
    assert.ok(delivered.missing_counters.split(',').includes('fallback_count'));
    assert.equal(summarizePromptTrace(events).counters.fallback_count, null);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('a missing internal model counter is explicitly partial at the tool boundary', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-trace-proxy-'));
  try {
    const tracePath = join(directory, 'prompt-trace.jsonl');
    const traceId = 'prompt-missing-internal-counter';
    const proxy = resolve(fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)));
    const server = resolve(fileURLToPath(new URL('./test-fixtures/traced-mcp-server.mjs', import.meta.url)));
    const child = spawn(process.execPath, [proxy, process.execPath, server], {
      env: { ...process.env, CONTROLLA_PROMPT_TRACE_FILE: tracePath, CONTROLLA_PROMPT_TRACE_ID: traceId,
        TRACE_FIXTURE_DUPLICATE_FALLBACK: '1', TRACE_FIXTURE_NO_INTERNAL: '1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    child.stdout.resume();
    child.stderr.resume();
    child.stdin.end(`${JSON.stringify({ jsonrpc: '2.0', id: 5, method: 'tools/call', params: { name: 'workflow', arguments: {} } })}\n`);
    const exit = await new Promise((resolveExit, reject) => {
      child.once('error', reject);
      child.once('close', resolveExit);
    });
    assert.equal(exit, 0);
    const delivered = (await readPromptTrace(tracePath)).find(event => event.event === 'tool_result_delivered');
    assert.equal(delivered.internal_model_calls, undefined);
    assert.equal(delivered.partial_counters, true);
    assert.ok(delivered.missing_counters.split(',').includes('internal_model_calls'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
