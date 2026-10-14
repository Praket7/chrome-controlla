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
  process.stdout.write(JSON.stringify({ passed: Boolean(payload.acting_result?.success) && !process.env.PHASE11_MOCK_VERIFY_FAIL, evidence_hash: `mock-${identity}-evidence` }));
  process.exit(0);
}

const wallMs = Number(wall);
const modelRounds = Number(rounds);
const promptSentNs = process.env.PHASE11_MOCK_TRACE ? process.hrtime.bigint().toString() : null;
process.stdout.write(JSON.stringify({
  adapter,
  ...(payload.source_ref ? { source_ref: payload.source_ref } : {}),
  task_id: payload.task_id,
  mode: payload.mode,
  concurrency: payload.concurrency,
  success: true,
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
  cache_status: payload.mode === 'warm_skill' ? 'hit' : 'miss',
  verification_ms: 5,
  cpu_ms: 10,
  peak_rss_bytes: 1024,
  focus_disruptions: 0,
  clipboard_disruptions: 0,
  human_interventions: 0,
  severe_wrong_target_events: 0,
  authority_violations: 0,
  silent_stale_mutations: 0,
  ...(promptSentNs ? { prompt_trace: { trace_valid: true, prompt_sent_observed: true,
    prompt_boundary: 'client_stdin_write', prompt_sent_monotonic_ns: promptSentNs,
    prompt_to_final_ms: 2 } } : {})
}));
