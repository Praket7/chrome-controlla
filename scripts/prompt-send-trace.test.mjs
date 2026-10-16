import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { recordPromptTraceEvent, readPromptTrace, summarizePromptTrace } from './prompt-send-trace.mjs';

test('prompt trace computes monotonic prompt-to-verified and prompt-to-final spans with execution counters', () => {
  const summary = summarizePromptTrace([
    { trace_id: 'r1', event: 'prompt_sent', monotonic_ns: '1000000', prompt_boundary: 'actual_user_submit' },
    { trace_id: 'r1', event: 'prompt_input_written', monotonic_ns: '1100000', prompt_input_bytes: 12, cold_warm: 'warm' },
    { trace_id: 'r1', event: 'first_tool_decision', monotonic_ns: '2000000' },
    { trace_id: 'r1', event: 'controlla_received', monotonic_ns: '3000000', tool_input_bytes: 80 },
    { trace_id: 'r1', event: 'browser_operation', monotonic_ns: '4000000', cdp_commands: 2, schema_bytes: 20 },
    { trace_id: 'r1', event: 'verified_outcome', monotonic_ns: '5000000', selected_execution_tier: 'qualified_skill' },
    { trace_id: 'r1', event: 'tool_result_delivered', monotonic_ns: '6000000', output_bytes: 40,
      internal_model_calls: 0, host_round_trips: 3, observation_bytes: 512, screenshot_bytes: 2048,
      fallback_count: 0,
      partial_counters: true, missing_counters: 'cdp_commands' },
    { trace_id: 'r1', event: 'model_visible_observation', monotonic_ns: '7000000' },
    { trace_id: 'r1', event: 'final_answer', monotonic_ns: '8000000' },
    { trace_id: 'r1', event: 'model_turn', monotonic_ns: '9000000' },
  ]);

  assert.equal(summary.prompt_to_verified_ms, 4);
  assert.equal(summary.prompt_to_final_ms, 7);
  assert.equal(summary.final_answer_observed, true);
  assert.equal(summary.prompt_sent_observed, true);
  assert.equal(summary.prompt_boundary, 'actual_user_submit');
  assert.equal(summary.counters.external_controlla_calls, 1);
  assert.equal(summary.counters.browser_cdp_commands, 2);
  assert.equal(summary.counters.prompt_input_bytes, 12);
  assert.equal(summary.counters.tool_input_bytes, 80);
  assert.equal(summary.counters.output_bytes, 40);
  assert.equal(summary.counters.schema_bytes, 20);
  assert.equal(summary.counters.internal_model_calls, 0);
  assert.equal(summary.counters.model_turns, 1);
  assert.equal(summary.counters.model_visible_observations, 1);
  assert.equal(summary.counters.host_round_trips, 3);
  assert.equal(summary.counters.observation_bytes, 512);
  assert.equal(summary.counters.screenshot_bytes, 2048);
  assert.equal(summary.counters.partial_counters, true);
  assert.equal(summary.counters.missing_counters, 'cdp_commands');
  assert.equal(summary.execution_tier, 'qualified_skill');
  assert.equal(summary.cold_warm, 'warm');
});

test('internal model calls sum per delivered tool call, deduplicate call ids, and preserve missing counts', () => {
  const base = { trace_id: 'calls', event: 'tool_result_delivered', monotonic_ns: '100' };
  const metrics = { cdp_commands: 0, host_round_trips: 0, observation_bytes: 0,
    screenshot_bytes: 0, fallback_count: 0, output_bytes: 10 };
  const complete = summarizePromptTrace([
    { ...base, call_id: 'a', ...metrics, internal_model_calls: 0 },
    { ...base, monotonic_ns: '200', call_id: 'b', ...metrics, internal_model_calls: 2 },
    { ...base, monotonic_ns: '300', call_id: 'b', ...metrics, internal_model_calls: 2 },
  ]);
  assert.equal(complete.counters.internal_model_calls, 2);
  assert.equal(complete.counters.partial_counters, false);
  assert.equal(complete.counters.missing_counters, '');

  const partial = summarizePromptTrace([
    { ...base, call_id: 'a', ...metrics, internal_model_calls: 0 },
    { ...base, monotonic_ns: '200', call_id: 'b', ...metrics, partial_counters: true },
  ]);
  assert.equal(partial.counters.internal_model_calls, null);
  assert.equal(partial.counters.partial_counters, true);
  assert.equal(partial.counters.missing_counters, 'internal_model_calls');
});

test('client and MCP call identities correlate only for a unique tool-name pair', () => {
  const unique = summarizePromptTrace([
    { trace_id: 'join', event: 'client_tool_decision', monotonic_ns: '100',
      span_id: 'client_tool_decision:100', client_call_id: 'client-1', tool: 'snapshot' },
    { trace_id: 'join', event: 'controlla_received', monotonic_ns: '200',
      span_id: 'controlla_received:200', call_id: 'mcp-1', tool: 'snapshot' },
  ]);
  assert.deepEqual(unique.call_correlations, [{ client_call_id: 'client-1', mcp_call_id: 'mcp-1',
    tool: 'snapshot', method: 'unique_tool_name_within_trace',
    client_span_id: 'client_tool_decision:100', mcp_span_id: 'controlla_received:200' }]);
  assert.deepEqual(unique.unmatched_client_call_ids, []);
  assert.deepEqual(unique.unmatched_mcp_call_ids, []);

  const ambiguous = summarizePromptTrace([
    { trace_id: 'join', event: 'client_tool_decision', monotonic_ns: '100', client_call_id: 'client-1', tool: 'snapshot' },
    { trace_id: 'join', event: 'client_tool_decision', monotonic_ns: '110', client_call_id: 'client-2', tool: 'snapshot' },
    { trace_id: 'join', event: 'controlla_received', monotonic_ns: '200', call_id: 'mcp-1', tool: 'snapshot' },
  ]);
  assert.deepEqual(ambiguous.call_correlations, []);
  assert.deepEqual(ambiguous.unmatched_client_call_ids, ['client-1', 'client-2']);
  assert.deepEqual(ambiguous.unmatched_mcp_call_ids, ['mcp-1']);
});

test('tool results without counter metadata are marked partial instead of appearing complete', () => {
  const summary = summarizePromptTrace([
    { trace_id: 'counter-missing', event: 'tool_result_delivered', monotonic_ns: '100',
      call_id: 'a', output_bytes: 10 },
  ]);
  assert.equal(summary.counters.internal_model_calls, null);
  assert.equal(summary.counters.browser_cdp_commands, null);
  assert.equal(summary.counters.host_round_trips, null);
  assert.equal(summary.counters.observation_bytes, null);
  assert.equal(summary.counters.screenshot_bytes, null);
  assert.equal(summary.counters.fallback_count, null);
  assert.equal(summary.counters.partial_counters, true);
  assert.equal(summary.counters.missing_counters,
    'cdp_commands,fallback_count,host_round_trips,internal_model_calls,observation_bytes,screenshot_bytes');
});

test('per-call result counters deduplicate repeated receipts and reject conflicting duplicates', () => {
  const base = { trace_id: 'counter-dedupe', event: 'tool_result_delivered', monotonic_ns: '100' };
  const fields = { internal_model_calls: 1, cdp_commands: 2, host_round_trips: 3,
    observation_bytes: 40, screenshot_bytes: 50, fallback_count: 0, output_bytes: 60 };
  const summary = summarizePromptTrace([
    { ...base, call_id: 'a', ...fields },
    { ...base, monotonic_ns: '200', call_id: 'a', ...fields },
    { ...base, monotonic_ns: '300', call_id: 'b', ...fields },
  ]);
  assert.equal(summary.counters.internal_model_calls, 2);
  assert.equal(summary.counters.browser_cdp_commands, 4);
  assert.equal(summary.counters.host_round_trips, 6);
  assert.equal(summary.counters.observation_bytes, 80);
  assert.equal(summary.counters.screenshot_bytes, 100);
  assert.equal(summary.counters.fallback_count, 0);
  assert.equal(summary.counters.output_bytes, 120);
  assert.equal(summary.counters.partial_counters, false);

  const conflicting = summarizePromptTrace([
    { ...base, call_id: 'a', ...fields },
    { ...base, monotonic_ns: '200', call_id: 'a', ...fields, cdp_commands: 9 },
  ]);
  assert.equal(conflicting.counters.browser_cdp_commands, null);
  assert.equal(conflicting.counters.partial_counters, true);
  assert.ok(conflicting.counters.missing_counters.split(',').includes('cdp_commands'));
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
  assert.equal(summary.prompt_boundary, null);
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
    await recordPromptTraceEvent(path, 'r1', 'prompt_input_written', {
      prompt_input_bytes: 12, clock_domain: 'local_host_monotonic', clock_offset_ms: 0,
      clock_uncertainty_ms: 0.25, prompt_boundary: 'client_stdin_write_finished'
    }, 123n);
    await recordPromptTraceEvent(path, 'r1', 'final_answer', {}, 456n);
    const events = await readPromptTrace(path);
    assert.equal(events.length, 2);
    assert.equal(events[0].monotonic_ns, '123');
    assert.equal(events[0].schema_version, 2);
    assert.equal(events[0].span_id, 'prompt_input_written:123');
    assert.equal(events[0].operation_id, 'r1');
    assert.equal(events[0].clock_domain, 'local_host_monotonic');
    assert.equal(events[0].boundary, 'prompt_input_written');
    assert.equal(events[0].provenance, 'client_adapter_hook');
    assert.equal(events[1].event, 'final_answer');
    assert.equal(summarizePromptTrace(events).clock_uncertainty_ms, null);
    assert.equal(summarizePromptTrace(events).prompt_sent_observed, false);
    assert.equal(summarizePromptTrace(events).counters.prompt_input_bytes, 12);
    assert.ok((await readFile(path, 'utf8')).endsWith('\n'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
