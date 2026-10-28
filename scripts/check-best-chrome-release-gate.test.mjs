import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const reportPath = resolve(root, 'bench/reports/phase11-results.json');
const shardDir = resolve(root, 'bench/reports/phase11-release-gate-test-shards');
const fullPhase11 = JSON.parse(await readFile(resolve(root, 'bench/tasks/phase11-realistic.json'), 'utf8'));
const adapterContract = JSON.parse(await readFile(resolve(root, 'bench/runners/adapter-contract.json'), 'utf8'));
const phase11 = {
  ...fullPhase11,
  tasks: fullPhase11.tasks.slice(0, 2),
  modes: [...fullPhase11.modes],
  concurrency: [1],
  repetitions: 30,
  warmup_repetitions: 2,
};
const taskSetPath = resolve(root, 'bench/reports/phase11-release-gate-test-task-set.json');

function result(adapter, taskId, mode, concurrency, wallMs, modelRoundTrips, executionMode = 'headless') {
  return {
    adapter,
    task_id: taskId,
    mode,
    execution_mode: executionMode,
    concurrency,
    success: true,
    independently_verified_success: true,
    verifier_evidence_hash: `evidence-${adapter}-${taskId}-${mode}-${concurrency}`,
    wall_ms: wallMs,
    prompt_to_verified_ms: wallMs + 30,
    prompt_to_verifier_decision_ms: wallMs + 30,
    prompt_to_final_ms: wallMs + 60,
    prompt_trace: {
      prompt_sent: true,
      prompt_sent_observed: true,
      partial_counters: false,
      missing_counters: '',
      prompt_boundary: 'actual_user_submit',
      clock_domain: 'local_host_monotonic',
      clock_offset_ms: 0,
      clock_uncertainty_ms: 0.1,
      prompt_sent_monotonic_ns: '150',
      process_start_monotonic_ns: '100',
      process_end_monotonic_ns: '200',
      verifier_clock_domain: 'local_host_monotonic',
      verifier_decision_monotonic_ns: '300',
      first_tool_decision_ms: 5,
      controlla_received_ms: 10,
      first_browser_op_ms: 15,
      last_browser_op_ms: 20,
      tool_result_delivered_ms: wallMs,
      verified_outcome_ms: wallMs + 30,
      final_answer_ms: wallMs + 60,
      final_answer_observed: true,
      prompt_input_bytes: 200,
      tool_input_bytes: 100,
      output_bytes: 400,
      schema_bytes: 500,
      external_controlla_calls: 1,
      model_turns: modelRoundTrips,
      model_visible_observations: 1,
      internal_model_calls: 0,
      browser_cdp_commands: 20,
      cold_warm: mode,
      execution_tier: 'semantic_program',
      fallback_count: 0,
    },
    model_round_trips: modelRoundTrips,
    model_tokens: modelRoundTrips * 100,
    mcp_calls: 2,
    browser_ops: 3,
    external_controlla_calls: 1,
    model_turns: modelRoundTrips,
    model_visible_observations: 1,
    internal_model_calls: 0,
    browser_cdp_commands: 20,
    host_round_trips: 2,
    native_host_messages_sent: 2,
    native_host_messages_received: 2,
    schema_bytes: 500,
    prompt_input_bytes: 200,
    tool_input_bytes: 100,
    output_bytes: 400,
    observation_bytes: 1024,
    screenshot_bytes: 0,
    unknown_delivery_count: 0,
    fallback_count: 0,
    retries: 0,
    timeout_count: 0,
    cleanup_completed: true,
    cleanup_outcome: 'completed',
    recovery_events: 0,
    cache_status: mode === 'warm_qualified_route' ? 'hit' : 'miss',
    verification_ms: 5,
    cpu_ms: 10,
    peak_rss_bytes: 1024,
    focus_disruptions: 0,
    clipboard_disruptions: 0,
    human_interventions: 0,
    model_visible_calls: 2,
    browser_commands: 12,
    native_host_commands: 2,
    cdp_commands: 20,
    full_ax_reads: 1,
    frame_tree_reads: 0,
    target_rebinds: 0,
    automatic_recoveries: 0,
    user_visible_recoveries: 0,
    application_accepted: true,
    input_commands: 2,
    verification_reads: 1,
    auto_attach_calls: 0,
    severe_wrong_target_events: 0,
    authority_violations: 0,
    silent_stale_mutations: 0,
    metric_evidence: Object.fromEntries(fullPhase11.required_metrics.map(metric => [metric, {
      s: 'observed', p: adapterContract.measurement_metric_provenance[metric]
    }]))
  };
}

function completeReport() {
  const structuredBaselines = phase11.adapters
    .filter(adapter => adapter.track === 'structured' && adapter.id !== 'controlla')
    .map(adapter => adapter.id);
  const makeRows = (baselineId, warmup) => {
    const rows = [];
    const repetitions = warmup ? phase11.warmup_repetitions ?? 0 : phase11.repetitions ?? 1;
    for (const task of phase11.tasks) for (const mode of phase11.modes)
      for (const executionMode of phase11.execution_modes) for (const concurrency of phase11.concurrency)
        for (let replicate = 1; replicate <= repetitions; replicate++) {
          rows.push({
            ...(warmup ? { warmup: true } : {}),
            replicate,
            task_id: task.id,
            mode,
            execution_mode: executionMode,
            concurrency,
            candidate_order: replicate % 2 ? 'first' : 'second',
            candidate: result('controlla', task.id, mode, concurrency, 700, 2, executionMode),
            baseline: result(baselineId, task.id, mode, concurrency, 1000, 4, executionMode)
          });
        }
    return rows;
  };
  return {
    schema_version: 2,
    track: 'phase11-realistic',
    candidate_adapter: 'controlla',
    randomization: { method: 'seeded_paired_order', seed: phase11.random_seed },
    environment: {
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
    },
    comparisons: structuredBaselines.map(baselineId => ({
      baseline_adapter: baselineId,
      randomization: { method: 'seeded_paired_order', seed: phase11.random_seed },
      warmup_rows: makeRows(baselineId, true),
      paired_rows: makeRows(baselineId, false)
    }))
  };
}

function runClaimGate() {
  return spawnSync(process.execPath, [resolve(root, 'scripts/check-best-chrome-release-gate.mjs'), '--claim'], {
    cwd: root,
    encoding: 'utf8',
    env: { ...process.env, PHASE11_TASK_SET: taskSetPath }
  });
}

function runStructuralCheck() {
  return spawnSync(process.execPath, [resolve(root, 'scripts/check-best-chrome-release-gate.mjs')], {
    cwd: root, encoding: 'utf8'
  });
}

async function writeReport(report) {
  await mkdir(dirname(reportPath), { recursive: true });
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

test('claim gate passes only complete independently verified structured competitor evidence', async () => {
  try {
    await mkdir(dirname(taskSetPath), { recursive: true });
    await writeFile(taskSetPath, `${JSON.stringify(phase11, null, 2)}\n`);
    const good = completeReport();
    await writeReport(good);
    const passing = runClaimGate();
    assert.equal(passing.status, 0, passing.stderr || passing.stdout);
    assert.match(passing.stdout, /"claim_eligible": true/);

    const unavailableMetric = structuredClone(good);
    const unavailableSide = unavailableMetric.comparisons[0].paired_rows[0].candidate;
    unavailableSide.output_bytes = null;
    unavailableSide.metric_evidence.output_bytes = {
      s: 'unavailable', p: adapterContract.measurement_metric_provenance.output_bytes,
      r: 'runner output boundary was not observed'
    };
    await writeReport(unavailableMetric);
    const unavailableMetricFailure = runClaimGate();
    assert.notEqual(unavailableMetricFailure.status, 0, 'a null required metric must block a public claim');
    assert.match(`${unavailableMetricFailure.stderr}\n${unavailableMetricFailure.stdout}`, /metric_evidence/);

    const missingMetricProvenance = structuredClone(good);
    delete missingMetricProvenance.comparisons[0].paired_rows[0].candidate.metric_evidence.browser_cdp_commands.p;
    await writeReport(missingMetricProvenance);
    const missingMetricProvenanceFailure = runClaimGate();
    assert.notEqual(missingMetricProvenanceFailure.status, 0, 'required metric evidence without provenance must block a public claim');
    assert.match(`${missingMetricProvenanceFailure.stderr}\n${missingMetricProvenanceFailure.stdout}`, /browser_cdp_commands evidence provenance/);

    const sharded = structuredClone(good);
    sharded.schema_version = 3;
    sharded.results_storage = 'comparison_files';
    await mkdir(shardDir, { recursive: true });
    for (const comparison of sharded.comparisons) {
      const shardPath = `bench/reports/phase11-release-gate-test-shards/${comparison.baseline_adapter}.json`;
      await writeFile(resolve(root, shardPath), `${JSON.stringify({
        schema_version: 2,
        track: sharded.track,
        candidate_adapter: sharded.candidate_adapter,
        baseline_adapter: comparison.baseline_adapter,
        environment: sharded.environment,
        randomization: comparison.randomization,
        warmup_rows: comparison.warmup_rows,
        paired_rows: comparison.paired_rows,
      })}\n`);
      sharded.comparisons[sharded.comparisons.indexOf(comparison)] = {
        baseline_adapter: comparison.baseline_adapter,
        randomization: comparison.randomization,
        evidence_file: shardPath,
      };
    }
    await writeReport(sharded);
    const shardedPassing = runClaimGate();
    assert.equal(shardedPassing.status, 0, shardedPassing.stderr || shardedPassing.stdout);

    const unobservedFinal = structuredClone(good);
    unobservedFinal.comparisons[0].paired_rows[0].candidate.prompt_trace.final_answer_observed = false;
    await writeReport(unobservedFinal);
    const unobservedFinalFailure = runClaimGate();
    assert.notEqual(unobservedFinalFailure.status, 0, 'numeric final timing without an observed final event must fail closed');
    assert.match(`${unobservedFinalFailure.stderr}\n${unobservedFinalFailure.stdout}`, /cannot report final timing without an observed final answer/);

    const falseSuccessRegression = structuredClone(good);
    falseSuccessRegression.comparisons[0].paired_rows[0].candidate.independently_verified_success = false;
    falseSuccessRegression.comparisons[0].paired_rows[0].candidate.prompt_to_verified_ms = null;
    falseSuccessRegression.comparisons[0].paired_rows[0].candidate.prompt_trace.verified_outcome_ms = null;
    falseSuccessRegression.comparisons[0].paired_rows[0].candidate.metric_evidence.prompt_to_verified_ms = {
      s: 'unavailable', p: adapterContract.measurement_metric_provenance.prompt_to_verified_ms,
      r: 'independent verifier rejected the outcome'
    };
    await writeReport(falseSuccessRegression);
    const falseSuccessFailure = runClaimGate();
    assert.notEqual(falseSuccessFailure.status, 0, 'false-success regressions must block a release claim');
    assert.match(`${falseSuccessFailure.stderr}\n${falseSuccessFailure.stdout}`, /false successes regress/);

    const failedAttempt = structuredClone(good);
    const failed = failedAttempt.comparisons[0].paired_rows[0].candidate;
    failed.success = false;
    failed.independently_verified_success = false;
    failed.wall_ms = 5000;
    failed.prompt_to_verified_ms = null;
    failed.prompt_to_verifier_decision_ms = 5100;
    failed.prompt_to_final_ms = null;
    for (const metric of ['prompt_to_verified_ms', 'prompt_to_final_ms']) failed.metric_evidence[metric] = {
      s: 'unavailable', p: adapterContract.measurement_metric_provenance[metric], r: 'no qualifying success boundary'
    };
    failed.prompt_trace.verified_outcome_ms = null;
    failed.prompt_trace.final_answer_ms = null;
    failed.prompt_trace.final_answer_observed = false;
    await writeReport(failedAttempt);
    const failedAttemptResult = runClaimGate();
    assert.notEqual(failedAttemptResult.status, 0, 'a failed attempt still affects release qualification');
    assert.doesNotMatch(`${failedAttemptResult.stderr}\n${failedAttemptResult.stdout}`, /unverified candidate attempt must not claim|missing numeric candidate\.prompt_to_verified_ms/,
      'failed attempts must reach all-attempt analysis with decision-time evidence');

    const oneModeRegression = structuredClone(good);
    for (const comparison of oneModeRegression.comparisons) {
      for (const row of comparison.paired_rows.filter(item => item.execution_mode === 'background')) {
        row.candidate.wall_ms = 1400;
      }
    }
    await writeReport(oneModeRegression);
    const oneModeFailure = runClaimGate();
    assert.notEqual(oneModeFailure.status, 0, 'a failing execution mode must block the aggregate claim');
    assert.match(`${oneModeFailure.stderr}\n${oneModeFailure.stdout}`, /background/);

    const oneLifecycleRegression = structuredClone(good);
    for (const comparison of oneLifecycleRegression.comparisons) {
      for (const row of comparison.paired_rows.filter(item => item.mode === 'reconnect')) {
        row.candidate.wall_ms = 1400;
      }
    }
    await writeReport(oneLifecycleRegression);
    const lifecycleFailure = runClaimGate();
    assert.notEqual(lifecycleFailure.status, 0, 'a failing lifecycle cohort must block the aggregate claim');
    assert.match(`${lifecycleFailure.stderr}\n${lifecycleFailure.stdout}`, /reconnect/);

    const unpinnedAdapter = structuredClone(good);
    delete unpinnedAdapter.environment.client_adapter_versions['chrome-devtools-mcp'];
    await writeReport(unpinnedAdapter);
    const unpinned = runClaimGate();
    assert.notEqual(unpinned.status, 0);
    assert.match(`${unpinned.stderr}\n${unpinned.stdout}`, /pin every configured structured client adapter/);

    const missingRow = structuredClone(good);
    missingRow.comparisons[0].paired_rows.pop();
    await writeReport(missingRow);
    const incomplete = runClaimGate();
    assert.notEqual(incomplete.status, 0);
    const expectedRows = phase11.tasks.length * phase11.modes.length * phase11.execution_modes.length * phase11.concurrency.length * phase11.repetitions;
    assert.match(`${incomplete.stderr}\n${incomplete.stdout}`, new RegExp(`complete ${expectedRows}-row task/warmth/execution-mode/concurrency/replicate matrix`));

    const forgedVerification = structuredClone(good);
    forgedVerification.comparisons[0].paired_rows[0].candidate.verifier_evidence_hash = '';
    await writeReport(forgedVerification);
    const forged = runClaimGate();
    assert.notEqual(forged.status, 0);
    assert.match(`${forged.stderr}\n${forged.stdout}`, /independent verifier evidence hash/);

    const missingTrace = structuredClone(good);
    delete missingTrace.comparisons[0].paired_rows[0].candidate.prompt_trace.model_turns;
    await writeReport(missingTrace);
    const missingTraceResult = runClaimGate();
    assert.notEqual(missingTraceResult.status, 0);
    assert.match(`${missingTraceResult.stderr}\n${missingTraceResult.stdout}`, /prompt_trace\.model_turns/);

    const partialCounters = structuredClone(good);
    partialCounters.comparisons[0].paired_rows[0].candidate.prompt_trace.partial_counters = true;
    partialCounters.comparisons[0].paired_rows[0].candidate.prompt_trace.missing_counters = 'screenshot_bytes';
    await writeReport(partialCounters);
    const partialCounterResult = runClaimGate();
    assert.notEqual(partialCounterResult.status, 0);
    assert.match(`${partialCounterResult.stderr}\n${partialCounterResult.stdout}`, /partial counters/);

    const processArgumentTrace = structuredClone(good);
    processArgumentTrace.comparisons[0].paired_rows[0].candidate.prompt_trace.prompt_boundary = 'client_process_spawn_argument';
    await writeReport(processArgumentTrace);
    const processArgumentResult = runClaimGate();
    assert.notEqual(processArgumentResult.status, 0);
    assert.match(`${processArgumentResult.stderr}\n${processArgumentResult.stdout}`, /actual prompt-send boundary/);

    const unrandomized = structuredClone(good);
    delete unrandomized.randomization;
    await writeReport(unrandomized);
    const unrandomizedResult = runClaimGate();
    assert.notEqual(unrandomizedResult.status, 0);
    assert.match(`${unrandomizedResult.stderr}\n${unrandomizedResult.stdout}`, /seeded paired-order/);
  } finally {
    await rm(reportPath, { force: true });
    await rm(taskSetPath, { force: true });
    await rm(shardDir, { recursive: true, force: true });
  }
});

test('Stage 3 qualification preregisters thirty paired trials and one-point success margin', () => {
  assert.ok(fullPhase11.repetitions >= 30, 'every task/mode/concurrency cohort needs at least 30 paired trials');
  assert.equal(fullPhase11.claim_gates.success_noninferiority_lower_bound, -0.01);
  assert.deepEqual(fullPhase11.execution_modes, ['foreground','background','headless']);
});

test('local structural check remains claim-disabled without measurement reports', () => {
  const structure = runStructuralCheck();
  assert.equal(structure.status, 0, structure.stderr || structure.stdout);
  assert.match(structure.stdout, /superiority claim remains disabled/);
});
