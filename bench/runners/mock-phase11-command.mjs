import { readFileSync } from 'node:fs';

let input = '';
for await (const chunk of process.stdin) input += chunk;
const payload = JSON.parse(input || '{}');
const [role = 'adapter', adapter = 'controlla', wall = '100', rounds = '2'] = process.argv.slice(2);

if (role === 'reset') {
  process.stdout.write(JSON.stringify({ ok: true }));
  process.exit(0);
}
if (role === 'verify') {
  const identity = `${payload.input?.task_id ?? 'task'}:${payload.acting_result?.adapter ?? 'adapter'}`;
  process.stdout.write(JSON.stringify({ passed: (Boolean(payload.acting_result?.success) || process.env.PHASE11_MOCK_VERIFY_PASS_ON_FAILURE === '1')
    && !process.env.PHASE11_MOCK_VERIFY_FAIL, evidence_hash: `mock-${identity}-evidence` }));
  process.exit(0);
}

const wallMs = Number(wall);
const modelRounds = Number(rounds);
const promptSentNs = process.env.PHASE11_MOCK_TRACE ? process.hrtime.bigint().toString() : null;
const tracePromptBoundary = process.env.PHASE11_MOCK_NO_PROMPT_BOUNDARY !== '1';
const successful = process.env.PHASE11_MOCK_RAW_FAILURE !== '1';
const verifiedPromptOutcome = Boolean(promptSentNs && tracePromptBoundary && successful
  && process.env.PHASE11_MOCK_VERIFY_FAIL !== '1');
const metricProvenance = JSON.parse(readFileSync('bench/runners/adapter-contract.json', 'utf8')).measurement_metric_provenance;
const metricEvidence = Object.fromEntries(Object.entries(metricProvenance).map(([field, provenance]) => [field, {
  provenance,
  ...((field === 'prompt_to_verified_ms' && !verifiedPromptOutcome)
    || (field === 'fallback_count' && process.env.PHASE11_MOCK_PARTIAL_COUNTERS === '1')
    || (field === 'output_bytes' && process.env.PHASE11_MOCK_OUTPUT_UNAVAILABLE === '1')
    || field === 'prompt_to_final_ms'
    ? { missing_reason: field === 'fallback_count' ? 'mock counter unavailable' : 'mock did not emit a qualifying verified-outcome or final-answer boundary' } : {})
}]));
process.stdout.write(JSON.stringify({
  adapter,
  ...(payload.source_ref ? { source_ref: payload.source_ref } : {}),
  task_id: payload.task_id,
  mode: payload.mode,
  execution_mode: payload.execution_mode,
  concurrency: payload.concurrency,
  success: successful,
  independently_verified_success: false,
  wall_ms: wallMs,
  model_round_trips: modelRounds,
  model_tokens: modelRounds * 100,
  mcp_calls: 2,
  browser_ops: 3,
  observation_bytes: 1024,
  screenshot_bytes: 0,
  retries: 0,
  recovery_events: 0,
  cache_status: payload.mode === 'warm_qualified_route' ? 'hit' : 'miss',
  verification_ms: 5,
  cpu_ms: 10,
  peak_rss_bytes: 1024,
  focus_disruptions: 0,
  clipboard_disruptions: 0,
  human_interventions: 0,
  severe_wrong_target_events: 0,
  authority_violations: 0,
  silent_stale_mutations: 0,
  external_controlla_calls: 2,
  model_turns: modelRounds,
  model_visible_observations: 1,
  internal_model_calls: 0,
  browser_cdp_commands: 3,
  host_round_trips: 2,
  native_host_messages_sent: 2,
  native_host_messages_received: 2,
  schema_bytes: 512,
  prompt_input_bytes: 128,
  tool_input_bytes: 64,
  output_bytes: process.env.PHASE11_MOCK_OUTPUT_UNAVAILABLE === '1' ? null : 1024,
  unknown_delivery_count: 0,
  fallback_count: process.env.PHASE11_MOCK_PARTIAL_COUNTERS === '1' ? null : 0,
  timeout_count: 0,
  cleanup_completed: true,
  cleanup_outcome: 'completed',
  metric_evidence: metricEvidence,
  ...(process.env.PHASE11_MOCK_UNOBSERVED_FINAL === '1' ? { prompt_to_final_ms: 2 } : {}),
  ...(process.env.PHASE11_MOCK_EXTRA_EVIDENCE ? { unbounded_page_dump: 'x'.repeat(900_000) } : {}),
  ...(promptSentNs ? { prompt_trace: { trace_valid: tracePromptBoundary, prompt_sent_observed: tracePromptBoundary,
    final_answer_observed: false,
    ...(tracePromptBoundary ? { prompt_boundary: 'actual_user_submit', prompt_sent_monotonic_ns: promptSentNs } : {}),
    clock_domain: 'local_host_monotonic', clock_offset_ms: 0, clock_uncertainty_ms: 0.1,
    process_start_monotonic_ns: (BigInt(promptSentNs) - 1_000_000n).toString(),
    process_end_monotonic_ns: (BigInt(promptSentNs) + 1_000_000n).toString(),
    ...(process.env.PHASE11_MOCK_PARTIAL_COUNTERS === '1'
      ? { counters: { host_round_trips: 2, partial_counters: true, missing_counters: 'screenshot_bytes' } } : {}),
    ...(tracePromptBoundary && process.env.PHASE11_MOCK_FAIL_WITH_TRACE !== '1' ? { prompt_to_final_ms: 2 } : {}) } } : {})
}));
if (process.env.PHASE11_MOCK_FAIL_WITH_TRACE === '1') process.exitCode = 7;
