import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { recordPromptTraceEvent, readPromptTrace, summarizePromptTrace } from './prompt-send-trace.mjs';

test('prompt trace computes monotonic prompt-to-verified and prompt-to-final spans with execution counters', () => {
  const summary = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '1000000', input_bytes: 12, cold_warm: 'warm' },
    { trace_id: 'r1', event: 'first_tool_decision', monotonic_ns: '2000000' },
    { trace_id: 'r1', event: 'controlla_received', monotonic_ns: '3000000', input_bytes: 80 },
    { trace_id: 'r1', event: 'browser_operation', monotonic_ns: '4000000', cdp_commands: 2, schema_bytes: 20 },
    { trace_id: 'r1', event: 'tool_result_delivered', monotonic_ns: '5000000', output_bytes: 40, internal_model_calls: 0 },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '6000000', selected_execution_tier: 'qualified_skill' },
    { trace_id: 'r1', event: 'final_answer', monotonic_ns: '8000000' },
    { trace_id: 'r1', event: 'model_turn', monotonic_ns: '2000000' },
    { trace_id: 'r1', event: 'model_visible_observation', monotonic_ns: '5000000' },
  ]);

  assert.equal(summary.prompt_to_verified_ms, 5);
  assert.equal(summary.prompt_to_final_ms, 7);
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

test('prompt trace leaves unverifiable spans unavailable and rejects mixed traces or backwards clocks', () => {
  const summary = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '900' },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '800' },
    { trace_id: 'r2', event: 'final_answer', monotonic_ns: '1000' },
  ]);
  assert.equal(summary.prompt_to_verified_ms, null);
  assert.equal(summary.prompt_to_final_ms, null);
  assert.equal(summary.trace_valid, false);
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
