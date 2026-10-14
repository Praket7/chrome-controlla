import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { recordPromptTraceEvent, readPromptTrace, summarizePromptTrace } from './prompt-send-trace.mjs';

test('prompt trace computes monotonic prompt-to-verified and prompt-to-final spans with execution counters', () => {
  const summary = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '1000000', input_bytes: 12, cold_warm: 'warm', prompt_boundary: 'client_stdin_write' },
    { trace_id: 'r1', event: 'first_tool_decision', monotonic_ns: '2000000' },
    { trace_id: 'r1', event: 'controlla_received', monotonic_ns: '3000000', input_bytes: 80 },
    { trace_id: 'r1', event: 'browser_operation', monotonic_ns: '4000000', cdp_commands: 2, schema_bytes: 20 },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '5000000', selected_execution_tier: 'qualified_skill' },
    { trace_id: 'r1', event: 'tool_result_delivered', monotonic_ns: '6000000', output_bytes: 40, internal_model_calls: 0 },
    { trace_id: 'r1', event: 'model_visible_observation', monotonic_ns: '7000000' },
    { trace_id: 'r1', event: 'final_answer', monotonic_ns: '8000000' },
    { trace_id: 'r1', event: 'model_turn', monotonic_ns: '9000000' },
  ]);

  assert.equal(summary.prompt_to_verified_ms, 4);
  assert.equal(summary.prompt_to_final_ms, 7);
  assert.equal(summary.prompt_sent_observed, true);
  assert.equal(summary.prompt_boundary, 'client_stdin_write');
  assert.equal(summary.counters.external_controlla_calls, 1);
  assert.equal(summary.counters.browser_cdp_commands, 2);
  assert.equal(summary.counters.input_bytes, 92);
  assert.equal(summary.counters.output_bytes, 40);
  assert.equal(summary.counters.schema_bytes, 20);
  assert.equal(summary.counters.internal_model_calls, 0);
  assert.equal(summary.counters.model_turns, 1);
  assert.equal(summary.counters.model_visible_observations, 1);
  assert.equal(summary.execution_tier, 'qualified_skill');
  assert.equal(summary.cold_warm, 'warm');
});

test('process-argument timestamps are not reported as prompt-send timing', () => {
  const summary = summarizePromptTrace([
    { trace_id: 'arg', event: 'prompt_sent', monotonic_ns: '100', prompt_boundary: 'client_process_spawn_argument' },
    { trace_id: 'arg', event: 'controlla_received', monotonic_ns: '400' },
    { trace_id: 'arg', event: 'browser_operation', monotonic_ns: '500' },
    { trace_id: 'arg', event: 'tool_result_delivered', monotonic_ns: '700' },
    { trace_id: 'arg', event: 'verified_outcome', monotonic_ns: '800' },
  ]);
  assert.equal(summary.trace_valid, true);
  assert.equal(summary.prompt_boundary, 'client_process_spawn_argument');
  assert.equal(summary.prompt_sent_observed, false);
  assert.equal(summary.prompt_to_verified_ms, null);
});

test('prompt trace leaves unverifiable spans unavailable and rejects mixed traces or backwards clocks', () => {
  const summary = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '900' },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '800' },
    { trace_id: 'r2', event: 'final_answer', monotonic_ns: '1000' },
  ]);
  assert.equal(summary.prompt_to_verified_ms, null);
  assert.equal(summary.prompt_to_final_ms, null);
  assert.equal(summary.prompt_sent_observed, false);
  assert.equal(summary.trace_valid, false);

  const backwards = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '900' },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '800' },
  ]);
  assert.equal(backwards.prompt_to_verified_ms, null);
  assert.equal(backwards.trace_valid, false);

  const duplicatePrompt = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '100', prompt_boundary: 'client_process_spawn_argument' },
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '200', prompt_boundary: 'client_process_spawn_argument' },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '300' },
  ]);
  assert.equal(duplicatePrompt.prompt_to_verified_ms, null);
  assert.equal(duplicatePrompt.trace_valid, false);

  const missingBoundary = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '100' },
    { trace_id: 'r1', event: 'first_tool_decision', monotonic_ns: '200' },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '300' },
  ]);
  assert.equal(missingBoundary.prompt_to_verified_ms, null);
  assert.equal(missingBoundary.trace_valid, true);
  assert.equal(missingBoundary.prompt_sent_observed, false);

  const semanticOrder = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '100', prompt_boundary: 'client_process_spawn_argument' },
    { trace_id: 'r1', event: 'controlla_received', monotonic_ns: '200' },
    { trace_id: 'r1', event: 'first_tool_decision', monotonic_ns: '300' },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '400' },
  ]);
  assert.equal(semanticOrder.prompt_to_verified_ms, null);
  assert.equal(semanticOrder.trace_valid, false);
});

test('prompt trace records bounded JSONL events and reads them back', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-prompt-trace-'));
  try {
    const path = join(directory, 'trace.jsonl');
    await recordPromptTraceEvent(path, 'r1', 'prompt_sent', { input_bytes: 12 }, 123n);
    await recordPromptTraceEvent(path, 'r1', 'final_answer', {}, 456n);
    const events = await readPromptTrace(path);
    assert.equal(events.length, 2);
    assert.equal(events[0].monotonic_ns, '123');
    assert.equal(events[1].event, 'final_answer');
    assert.ok((await readFile(path, 'utf8')).endsWith('\n'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
