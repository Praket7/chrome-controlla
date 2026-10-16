import assert from 'node:assert/strict';
import { test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';
import { analyzePaired, evaluateClaimGate } from '../analysis/phase11-analysis.mjs';
import { parseCommand, runBenchmark, validateAdapterResult, loadAdapterContract, loadPhase11Config, liveTaskUrl } from './phase11-runner.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const mock = resolve(here, 'mock-phase11-command.mjs');
const node = process.execPath;
const environment = {
  candidate_source_sha: 'a'.repeat(40),
  dirty_tree_patch_sha256: 'b'.repeat(64),
  runtime_binary_sha256: 'c'.repeat(64),
  extension_build_sha256: 'd'.repeat(64),
  chrome_version: 'test-chrome',
  os_build: 'test-os-build',
  viewport: '1280x720',
  network_profile: 'isolated-fixture',
  model: 'test-model',
  model_settings: 'temperature=0',
  prompt_hash: 'prompt-v1',
  token_budget: 1000,
  fixture_revision: 'fixture-v1',
  machine_profile: 'ci-test',
  client_adapter_versions: Object.fromEntries(['controlla','agent-browser','browser-use','chrome-devtools-mcp','controlla-1','controlla-2-pre-stage2','playwright-mcp','stagehand'].map(id => [id, 'test'])),
  account_fixture_id: 'synthetic-fixture-account'
};

function adapter(id, wall, rounds) {
  return [node, mock, 'adapter', id, String(wall), String(rounds)];
}

const reset = [node, mock, 'reset'];
const verifier = [node, mock, 'verify'];

test('parseCommand rejects shell strings and accepts argv JSON', () => {
  assert.deepEqual(parseCommand('["node","adapter.mjs"]', 'TEST'), ['node', 'adapter.mjs']);
  assert.throws(() => parseCommand('node adapter.mjs', 'TEST'), /valid JSON/);
  assert.throws(() => parseCommand('[]', 'TEST'), /non-empty JSON array/);
});

test('live site URLs require HTTPS and exact host allowlists', () => {
  assert.equal(liveTaskUrl({ id:'safe', url:'https://demo.example.test/path', allowed_hosts:['demo.example.test'] }), 'https://demo.example.test/path');
  assert.throws(() => liveTaskUrl({ id:'host-drift', url:'https://evil.example.test/path', allowed_hosts:['demo.example.test'] }), /exact allowlisted host/);
  assert.throws(() => liveTaskUrl({ id:'credentials', url:'https://user:pass@demo.example.test/path', allowed_hosts:['demo.example.test'] }), /credential-free HTTPS/);
  assert.throws(() => liveTaskUrl({ id:'http', url:'http://demo.example.test/path', allowed_hosts:['demo.example.test'] }), /credential-free HTTPS/);
});

test('validateAdapterResult rejects identity drift', async () => {
  const contract = await loadAdapterContract();
  const result = completeMetricResult(contract);
  result.adapter = 'wrong';
  assert.throws(() => validateAdapterResult(result, contract, {
    adapter: 'controlla', task_id: 'task', mode: 'cold_browser', execution_mode: 'headless', concurrency: 1
  }), /identity/);
});

test('metric evidence accepts complete observed values including measured zero', async () => {
  const contract = await loadAdapterContract();
  const result = completeMetricResult(contract);
  const validated = validateAdapterResult(result, contract, {
    adapter: 'controlla', task_id: 'task', mode: 'cold_browser', execution_mode: 'headless', concurrency: 1
  });
  assert.equal(validated.fallback_count, 0);
  assert.equal(validated.metric_evidence.fallback_count.provenance, contract.measurement_metric_provenance.fallback_count);
});

test('metric evidence accepts unavailable null only with a reason and provenance', async () => {
  const contract = await loadAdapterContract();
  const result = completeMetricResult(contract);
  result.model_visible_observations = null;
  result.metric_evidence.model_visible_observations = {
    provenance: contract.measurement_metric_provenance.model_visible_observations,
    missing_reason: 'this client has no observation hook'
  };
  assert.doesNotThrow(() => validateAdapterResult(result, contract, {
    adapter: 'controlla', task_id: 'task', mode: 'cold_browser', execution_mode: 'headless', concurrency: 1
  }));
});

test('metric evidence rejects missing per-field provenance', async () => {
  const contract = await loadAdapterContract();
  const result = completeMetricResult(contract);
  delete result.metric_evidence.browser_cdp_commands.provenance;
  assert.throws(() => validateAdapterResult(result, contract, {
    adapter: 'controlla', task_id: 'task', mode: 'cold_browser', execution_mode: 'headless', concurrency: 1
  }), /metric_evidence missing provenance for browser_cdp_commands/);
});

function completeMetricResult(contract) {
  const result = {
    adapter: 'controlla', task_id: 'task', mode: 'cold_browser', execution_mode: 'headless', concurrency: 1,
    success: true, independently_verified_success: true, wall_ms: 1, model_round_trips: 0,
    model_tokens: 0, mcp_calls: 0, browser_ops: 0, observation_bytes: 0, screenshot_bytes: 0,
    retries: 0, recovery_events: 0, cache_status: 'miss', verification_ms: 0, cpu_ms: 0,
    peak_rss_bytes: 0, focus_disruptions: 0, clipboard_disruptions: 0, human_interventions: 0,
    severe_wrong_target_events: 0, authority_violations: 0, silent_stale_mutations: 0,
    metric_evidence: {}
  };
  for (const field of contract.output_required) {
    if (field in result) continue;
    const type = contract.measurement_metric_types?.[field];
    result[field] = type === 'boolean' ? true : type === 'string' ? 'complete' : 0;
  }
  for (const field of contract.measurement_metric_fields) {
    result.metric_evidence[field] = { provenance: contract.measurement_metric_provenance[field] };
  }
  return result;
}

test('runner resets, independently verifies, preserves paired failures, and emits full matrix subset', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: {
      controlla: adapter('controlla', 80, 1),
      'playwright-mcp': adapter('playwright-mcp', 120, 3)
    },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_qualified_route'],
    executionModes: ['headless'],
    concurrencyLevels: [1],
    repetitions: 1,
    warmupRepetitions: 0,
    deadlineMs: 5000
  });
  assert.equal(report.schema_version, 2);
  assert.deepEqual(report.randomization, { method: 'seeded_paired_order', seed: 20261015 });
  assert.equal(report.candidate_adapter, 'controlla');
  assert.equal(report.baseline_adapter, 'playwright-mcp');
  assert.equal(report.paired_rows.length, (await loadPhase11Config()).tasks.length);
  assert.ok(report.paired_rows.every(row => row.execution_mode === 'headless' && row.candidate.execution_mode === 'headless'));
  for (const row of report.paired_rows) {
    assert.equal(row.candidate.success, true);
    assert.equal(row.candidate.independently_verified_success, true);
    assert.equal(row.baseline.independently_verified_success, true);
    assert.equal(row.candidate.wall_ms, 80);
    assert.equal(row.candidate.prompt_to_verified_ms, null);
    assert.equal(typeof row.candidate.adapter_request_to_verified_ms, 'number');
    assert.equal(row.candidate.prompt_trace.prompt_sent, null);
    assert.equal(row.candidate.prompt_trace.tool_result_delivered_ms, null);
    assert.equal(typeof row.candidate.prompt_trace.adapter_command_completed_ms, 'number');
    assert.equal(row.baseline.wall_ms, 120);
    assert.match(row.candidate.verifier_evidence_hash, /^mock-/);
    assert.equal(row.candidate.severe_wrong_target_events, 0);
    assert.equal(row.candidate.authority_violations, 0);
    assert.equal(row.candidate.silent_stale_mutations, 0);
  }
});

test('runner drops uncontracted adapter payloads before retaining report evidence', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp', fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset, verifierCommand: verifier, environment,
    modes: ['warm_qualified_route'], executionModes: ['headless'], concurrencyLevels: [1],
    repetitions: 1, warmupRepetitions: 0, deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_EXTRA_EVIDENCE: '1' },
  });
  assert.equal('unbounded_page_dump' in report.paired_rows[0].candidate, false);
  assert.ok(Buffer.byteLength(JSON.stringify(report.paired_rows[0])) <= 16 * 1024);
});

test('runner retains only observed prompt trace from a failed adapter attempt', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp', fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset, verifierCommand: verifier, environment,
    modes: ['warm_qualified_route'], executionModes: ['headless'], concurrencyLevels: [1],
    repetitions: 1, warmupRepetitions: 0, deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_TRACE: '1', PHASE11_MOCK_FAIL_WITH_TRACE: '1' },
  });
  const failed = report.paired_rows[0].candidate;
  assert.equal(failed.success, false);
  assert.ok(failed.wall_ms > 0, 'runner failure still retains observed adapter wall time');
  assert.equal(failed.prompt_trace.prompt_sent, true);
  assert.equal(failed.prompt_trace.prompt_boundary, 'actual_user_submit');
  assert.equal(failed.prompt_to_verified_ms, null);
  assert.equal(failed.prompt_to_final_ms, null);
});

test('runner stops before retaining more than the configured aggregate report budget', async () => {
  await assert.rejects(() => runBenchmark({
    baselineId: 'playwright-mcp', fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset, verifierCommand: verifier, environment,
    modes: ['warm_qualified_route'], executionModes: ['headless'], concurrencyLevels: [1],
    repetitions: 1, warmupRepetitions: 0, deadlineMs: 5000, maxReportBytes: 1,
  }), /benchmark report exceeded 1 bytes/);
});

test('paired trial ordering is reproducible and randomized from the matrix order', async () => {
  const options = {
    baselineId: 'playwright-mcp', fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset, verifierCommand: verifier, environment,
    modes: ['cold_browser', 'warm_browser_cold_site'], concurrencyLevels: [1], repetitions: 1, warmupRepetitions: 0, deadlineMs: 5000,
    executionModes: ['foreground'],
  };
  const first = await runBenchmark(options);
  const second = await runBenchmark(options);
  assert.deepEqual(first.paired_rows.map(row => [row.task_id, row.mode, row.execution_mode]), second.paired_rows.map(row => [row.task_id, row.mode, row.execution_mode]));
  assert.notDeepEqual(first.paired_rows.slice(0, 5).map(row => [row.task_id, row.mode, row.execution_mode]),
    (await loadPhase11Config()).tasks.slice(0, 5).map(task => [task.id, 'cold_browser']));
});

test('warmup trials are recorded separately from measured paired trials', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp', fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset, verifierCommand: verifier, environment,
    modes: ['warm_qualified_route'], concurrencyLevels: [1], repetitions: 1, warmupRepetitions: 1, deadlineMs: 5000,
    executionModes: ['background'],
  });
  assert.equal(report.warmup_rows.length, (await loadPhase11Config()).tasks.length);
  assert.equal(report.paired_rows.length, (await loadPhase11Config()).tasks.length);
  assert.ok(report.warmup_rows.every(row => row.warmup === true));
  assert.ok(report.paired_rows.every(row => row.warmup !== true));
});

test('adapter crashes retain elapsed wall time and leave unobserved counters unknown', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp', fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: [node, '-e', 'process.exit(1)'], 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset, verifierCommand: verifier, environment, executionModes: ['headless'],
    modes: ['warm_qualified_route'], concurrencyLevels: [1], repetitions: 1, warmupRepetitions: 0, deadlineMs: 1000
  });
  const failed = report.paired_rows[0].candidate;
  assert.equal(failed.success, false);
  assert.ok(failed.wall_ms >= 0 && failed.wall_ms <= 1000);
  for (const field of ['model_round_trips', 'mcp_calls', 'browser_ops', 'retries', 'verification_ms', 'severe_wrong_target_events']) {
    assert.equal(failed[field], null, `${field} must remain unknown after an adapter crash`);
  }
  const analysis = analyzePaired(report.paired_rows, { iterations: 100, seed: 8 });
  assert.equal(analysis.model_round_trip_reduction, null);
  assert.equal(analysis.severe_wrong_target_events, null);
  assert.equal(evaluateClaimGate(analysis).checks.severe_wrong_target, false);
});

test('prompt-to-verified ends at the independent verifier, not an acting-client receipt', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: {
      controlla: adapter('controlla', 80, 1),
      'playwright-mcp': adapter('playwright-mcp', 120, 3)
    },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_qualified_route'],
    executionModes: ['headless'],
    concurrencyLevels: [1],
    repetitions: 1,
    warmupRepetitions: 0,
    deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_TRACE: '1' }
  });
  const result = report.paired_rows[0].candidate;
  assert.equal(result.independently_verified_success, true);
  assert.equal(result.prompt_trace.prompt_boundary, 'actual_user_submit');
  assert.equal(typeof result.prompt_to_verified_ms, 'number');
  assert.equal(typeof result.prompt_to_verifier_decision_ms, 'number');
  assert.equal(typeof result.prompt_to_verified_ms, 'number');
  assert.equal(typeof result.process_start_to_outcome_ms, 'number');
  assert.equal(result.prompt_trace.partial_counters, false);
  assert.ok(result.prompt_to_verified_ms > 0);
  assert.ok(result.prompt_to_verified_ms < result.adapter_request_to_verified_ms);
});

test('failed independent verification reports decision latency without a verified-success span', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_qualified_route'],
    executionModes: ['headless'],
    concurrencyLevels: [1],
    repetitions: 1,
    warmupRepetitions: 0,
    deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_TRACE: '1', PHASE11_MOCK_VERIFY_FAIL: '1' }
  });
  const result = report.paired_rows[0].candidate;
  assert.equal(result.independently_verified_success, false);
  assert.equal(result.prompt_to_verified_ms, null);
  assert.equal(result.adapter_request_to_verified_ms, null);
  assert.equal(typeof result.prompt_to_verifier_decision_ms, 'number');
  assert.equal(typeof result.adapter_request_to_verifier_decision_ms, 'number');
  assert.equal(result.prompt_to_verified_ms, null);
  assert.equal(typeof result.prompt_to_verifier_decision_ms, 'number');
});

test('failed adapter run cannot receive verified-success timing even if the verifier predicate passes', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_qualified_route'],
    executionModes: ['headless'],
    concurrencyLevels: [1],
    repetitions: 1,
    warmupRepetitions: 0,
    deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_TRACE: '1', PHASE11_MOCK_RAW_FAILURE: '1', PHASE11_MOCK_VERIFY_PASS_ON_FAILURE: '1' }
  });
  const result = report.paired_rows[0].candidate;
  assert.equal(result.success, false);
  assert.equal(result.independently_verified_success, false);
  assert.equal(result.prompt_to_verified_ms, null);
  assert.equal(result.prompt_to_verified_ms, null);
  assert.equal(typeof result.prompt_to_verifier_decision_ms, 'number');
});

test('runner ignores final-answer timing not backed by an observed final-answer trace event', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_qualified_route'],
    executionModes: ['headless'],
    concurrencyLevels: [1],
    repetitions: 1,
    warmupRepetitions: 0,
    deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_TRACE: '1', PHASE11_MOCK_RAW_FAILURE: '1', PHASE11_MOCK_UNOBSERVED_FINAL: '1' }
  });
  const result = report.paired_rows[0].candidate;
  assert.equal(result.success, false);
  assert.equal(result.prompt_trace.final_answer_observed, false);
  assert.equal(result.prompt_to_final_ms, null);
  assert.equal(result.prompt_to_final_ms, null);
});

test('clients without a submit hook retain a separate process-start-to-outcome measurement', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_qualified_route'],
    executionModes: ['headless'],
    concurrencyLevels: [1],
    repetitions: 1,
    warmupRepetitions: 0,
    deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_TRACE: '1', PHASE11_MOCK_NO_PROMPT_BOUNDARY: '1' }
  });
  const result = report.paired_rows[0].candidate;
  assert.equal(result.prompt_to_verified_ms, null);
  assert.equal(result.prompt_to_verifier_decision_ms, null);
  assert.equal(result.prompt_trace.prompt_sent, null);
  assert.equal(typeof result.process_start_to_outcome_ms, 'number');
});

test('nested partial-counter metadata survives runner normalization', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'playwright-mcp': adapter('playwright-mcp', 120, 3) },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_qualified_route'],
    executionModes: ['headless'],
    concurrencyLevels: [1],
    repetitions: 1,
    warmupRepetitions: 0,
    deadlineMs: 5000,
    env: { ...process.env, PHASE11_MOCK_TRACE: '1', PHASE11_MOCK_PARTIAL_COUNTERS: '1' }
  });
  const result = report.paired_rows[0].candidate;
  assert.equal(result.host_round_trips, 2);
  assert.equal(result.prompt_trace.partial_counters, true);
  assert.equal(result.prompt_trace.missing_counters, 'fallback_count,screenshot_bytes');
  assert.equal(result.fallback_count, null, 'generic recovery events cannot stand in for explicit fallback counts');
});

test('computer-use cannot be used as the structured release baseline', async () => {
  await assert.rejects(() => runBenchmark({
    baselineId: 'computer-use',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'computer-use': adapter('computer-use', 100, 2) },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['cold_browser'],
    executionModes: ['foreground'],
    concurrencyLevels: [1],
    warmupRepetitions: 0,
    deadlineMs: 5000
  }), /separate vision track/);
});
