import assert from 'node:assert/strict';
import { access, readFile, realpath, stat } from 'node:fs/promises';
import { analyzePaired, evaluateClaimGate } from '../bench/analysis/phase11-analysis.mjs';
import { MAX_REPORT_BYTES } from '../bench/runners/phase11-runner.mjs';
import { isAbsolute, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const requiredFiles = [
  'bench/phase10-pilot.test.mjs',
  'scripts/check-phase10-heldout.mjs',
  'bench/tasks/phase11-realistic.json',
  'bench/runners/phase11-runner.mjs',
  'bench/runners/phase11-suite.mjs',
  'apps/qualification-matrix.json',
  'docs/design/best-chrome-use-v2.md',
  'crates/controlla-runtime/tests/best_chrome_use_faults.rs'
];
for (const path of requiredFiles) await access(new URL(path, root));

const phase11Path = process.env.PHASE11_TASK_SET ?? 'bench/tasks/phase11-realistic.json';
const phase11 = JSON.parse(await readFile(new URL(phase11Path, root), 'utf8'));
assert.equal(phase11.frozen_phase10_unchanged, true);
assert.equal(phase11.claim_gates.maximum_severe_wrong_target_events, 0);
assert.equal(phase11.claim_gates.maximum_authority_violations, 0);
assert.equal(phase11.claim_gates.maximum_silent_stale_mutations, 0);

const claimRequested = process.argv.includes('--claim');
if (!claimRequested) {
  console.log('best-chrome release structure is valid; superiority claim remains disabled until measured Phase 11 results are supplied');
  process.exit(0);
}

const adapterContract = JSON.parse(await readFile(new URL('bench/runners/adapter-contract.json', root), 'utf8'));
assert.deepEqual([...adapterContract.measurement_metric_fields].sort(), [...phase11.required_metrics].sort(),
  'claim metric-evidence contract must cover every required Phase 11 metric');

let results;
try {
  results = JSON.parse(await readFile(new URL('bench/reports/phase11-results.json', root), 'utf8'));
} catch (error) {
  throw new Error(`--claim requires bench/reports/phase11-results.json: ${error.message}`);
}

function requireEnvironment(environment) {
  assert.ok(environment && typeof environment === 'object' && !Array.isArray(environment), 'claim evidence requires an environment manifest');
  const required = phase11.required_environment;
  for (const field of required) {
    assert.ok(field in environment, `claim evidence environment missing ${field}`);
    assert.notEqual(environment[field], '', `claim evidence environment ${field} must not be empty`);
  }
  assert.match(environment.candidate_source_sha, /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/i, 'candidate source SHA must be a full Git or SHA-256 hash');
  for (const field of ['dirty_tree_patch_sha256','runtime_binary_sha256','extension_build_sha256']) {
    assert.match(environment[field], /^[a-f0-9]{64}$/i, `${field} must be a SHA-256 digest`);
  }
  assert.ok(Number.isSafeInteger(environment.token_budget) && environment.token_budget > 0, 'token_budget must be positive');
  assert.ok(environment.client_adapter_versions && typeof environment.client_adapter_versions === 'object'
    && !Array.isArray(environment.client_adapter_versions) && Object.keys(environment.client_adapter_versions).length > 0
    && Object.values(environment.client_adapter_versions).every(value => typeof value === 'string' && value.trim()),
  'claim evidence must pin client adapter versions');
  const expectedAdapters = phase11.adapters.filter(adapter => adapter.track === 'structured').map(adapter => adapter.id);
  assert.deepEqual(Object.keys(environment.client_adapter_versions).sort(), expectedAdapters.sort(),
    'claim evidence must pin every configured structured client adapter exactly once');
}

function validateResultSide(side, expectedAdapter, taskId, mode, executionMode, concurrency) {
  assert.equal(side.adapter, expectedAdapter, `adapter identity mismatch for ${expectedAdapter}`);
  assert.equal(side.task_id, taskId, `task identity mismatch for ${expectedAdapter}`);
  assert.equal(side.mode, mode, `mode identity mismatch for ${expectedAdapter}`);
  assert.equal(side.execution_mode, executionMode, `execution mode identity mismatch for ${expectedAdapter}`);
  assert.equal(side.concurrency, concurrency, `concurrency identity mismatch for ${expectedAdapter}`);
  assert.equal(typeof side.success, 'boolean');
  assert.equal(typeof side.independently_verified_success, 'boolean');
  assert.ok(!(side.independently_verified_success && !side.success), 'independent success cannot be true when acting success is false');
  for (const metric of phase11.required_metrics) {
    const evidence = side.metric_evidence?.[metric];
    assert.ok(evidence && typeof evidence === 'object' && !Array.isArray(evidence),
      `missing ${expectedAdapter}.${metric} metric_evidence`);
    assert.equal(evidence.p, adapterContract.measurement_metric_provenance[metric],
      `missing ${expectedAdapter}.${metric} evidence provenance`);
    if (side[metric] === null || side[metric] === undefined) {
      assert.equal(evidence.s, 'unavailable', `${expectedAdapter}.${metric} null value needs unavailable evidence status`);
      assert.equal(typeof evidence.r, 'string', `${expectedAdapter}.${metric} unavailable value needs a missing reason`);
      assert.ok(evidence.r.trim(), `${expectedAdapter}.${metric} unavailable value needs a non-empty missing reason`);
    } else {
      assert.equal(evidence.s, 'observed', `${expectedAdapter}.${metric} observed value needs observed evidence status`);
      assert.ok(!Object.hasOwn(evidence, 'r'), `${expectedAdapter}.${metric} observed value cannot include a missing reason`);
    }
  }
  assert.equal(typeof side.verifier_evidence_hash, 'string');
  assert.ok(side.verifier_evidence_hash.length >= 8, 'claim evidence requires an independent verifier evidence hash');
  assert.equal(typeof side.prompt_to_verifier_decision_ms, 'number', `missing numeric ${expectedAdapter}.prompt_to_verifier_decision_ms`);
  assert.ok(Number.isFinite(side.prompt_to_verifier_decision_ms) && side.prompt_to_verifier_decision_ms >= 0,
    `invalid ${expectedAdapter}.prompt_to_verifier_decision_ms`);
  if (side.independently_verified_success) {
    assert.equal(typeof side.prompt_to_verified_ms, 'number', `verified ${expectedAdapter} attempt requires prompt_to_verified_ms`);
    assert.ok(Number.isFinite(side.prompt_to_verified_ms) && side.prompt_to_verified_ms >= 0,
      `invalid ${expectedAdapter}.prompt_to_verified_ms`);
  } else {
    assert.equal(side.prompt_to_verified_ms, null, `unverified ${expectedAdapter} attempt must not claim prompt_to_verified_ms`);
  }
  if (side.prompt_to_final_ms !== null) {
    assert.equal(typeof side.prompt_to_final_ms, 'number', `invalid ${expectedAdapter}.prompt_to_final_ms`);
    assert.ok(Number.isFinite(side.prompt_to_final_ms) && side.prompt_to_final_ms >= 0, `invalid ${expectedAdapter}.prompt_to_final_ms`);
  }
  assert.ok(side.prompt_trace && side.prompt_trace.prompt_sent === true, `missing prompt-send trace for ${expectedAdapter}`);
  assert.equal(typeof side.prompt_trace.final_answer_observed, 'boolean', `${expectedAdapter} trace must state whether the final answer was observed`);
  if (side.prompt_trace.final_answer_observed) {
    assert.equal(typeof side.prompt_to_final_ms, 'number', `${expectedAdapter} observed final answer requires prompt_to_final_ms`);
    assert.equal(side.prompt_trace.final_answer_ms, side.prompt_to_final_ms, `${expectedAdapter} final-answer spans must agree`);
  } else {
    assert.equal(side.prompt_to_final_ms, null, `${expectedAdapter} cannot report final timing without an observed final answer`);
    assert.equal(side.prompt_trace.final_answer_ms, null, `${expectedAdapter} trace cannot report final timing without an observed final answer`);
  }
  assert.equal(side.prompt_trace.prompt_sent_observed, true, `${expectedAdapter} prompt trace must observe a real prompt-send boundary`);
  assert.equal(side.prompt_trace.prompt_boundary, 'actual_user_submit',
    `${expectedAdapter} prompt trace must use an actual prompt-send boundary`);
  assert.equal(side.prompt_trace.clock_domain, 'local_host_monotonic', `${expectedAdapter} prompt and verifier timestamps must share the qualified local host monotonic domain`);
  assert.equal(side.prompt_trace.clock_offset_ms, 0, `${expectedAdapter} same-host monotonic alignment must not apply an unmeasured offset`);
  assert.equal(typeof side.prompt_trace.clock_uncertainty_ms, 'number', `${expectedAdapter} prompt write must report clock uncertainty`);
  assert.ok(Number.isFinite(side.prompt_trace.clock_uncertainty_ms) && side.prompt_trace.clock_uncertainty_ms >= 0
    && side.prompt_trace.clock_uncertainty_ms <= 5, `${expectedAdapter} prompt write clock uncertainty exceeds 5ms`);
  assert.equal(side.prompt_trace.partial_counters, false, `${expectedAdapter} prompt trace has partial counters`);
  assert.equal(side.prompt_trace.missing_counters, '', `${expectedAdapter} prompt trace is missing required counters`);
  assert.match(side.prompt_trace.prompt_sent_monotonic_ns, /^\d+$/, `${expectedAdapter} prompt trace requires its monotonic timestamp`);
  assert.match(side.prompt_trace.process_start_monotonic_ns, /^\d+$/, `${expectedAdapter} prompt trace requires process start`);
  assert.match(side.prompt_trace.process_end_monotonic_ns, /^\d+$/, `${expectedAdapter} prompt trace requires process end`);
  assert.equal(side.prompt_trace.verifier_clock_domain, 'local_host_monotonic', `${expectedAdapter} verifier must share the qualified local host monotonic domain`);
  assert.match(side.prompt_trace.verifier_decision_monotonic_ns, /^\d+$/, `${expectedAdapter} trace requires verifier decision timestamp`);
  assert.ok(BigInt(side.prompt_trace.process_start_monotonic_ns) <= BigInt(side.prompt_trace.prompt_sent_monotonic_ns)
    && BigInt(side.prompt_trace.prompt_sent_monotonic_ns) <= BigInt(side.prompt_trace.process_end_monotonic_ns),
  `${expectedAdapter} prompt send must fall inside the observed client process lifetime`);
  assert.ok(BigInt(side.prompt_trace.prompt_sent_monotonic_ns) <= BigInt(side.prompt_trace.verifier_decision_monotonic_ns),
    `${expectedAdapter} verifier decision must follow observed prompt submission in the shared clock domain`);
  for (const span of ['first_tool_decision_ms','controlla_received_ms','first_browser_op_ms','last_browser_op_ms','tool_result_delivered_ms','verified_outcome_ms','final_answer_ms']) {
    const value = side.prompt_trace[span];
    if (span === 'verified_outcome_ms' && !side.independently_verified_success) {
      assert.equal(value, null, `${expectedAdapter} failed attempt cannot claim verified_outcome_ms`);
    } else if (value !== null) {
      assert.equal(typeof value, 'number', `invalid ${expectedAdapter} prompt_trace.${span}`);
      assert.ok(Number.isFinite(value) && value >= 0, `invalid ${expectedAdapter} prompt_trace.${span}`);
    }
  }
  for (const counter of ['prompt_input_bytes','tool_input_bytes','output_bytes','schema_bytes','external_controlla_calls','model_turns','model_visible_observations','internal_model_calls','browser_cdp_commands','fallback_count']) {
    assert.equal(typeof side.prompt_trace[counter], 'number', `missing ${expectedAdapter} prompt_trace.${counter}`);
    assert.ok(Number.isSafeInteger(side.prompt_trace[counter]) && side.prompt_trace[counter] >= 0, `invalid ${expectedAdapter} prompt_trace.${counter}`);
  }
  for (const metric of [
    'wall_ms','model_round_trips','model_tokens','mcp_calls','browser_ops','observation_bytes','screenshot_bytes',
    'retries','recovery_events','verification_ms','cpu_ms','peak_rss_bytes','focus_disruptions','clipboard_disruptions',
    'human_interventions','severe_wrong_target_events','authority_violations','silent_stale_mutations'
  ]) {
    assert.equal(typeof side[metric], 'number', `missing numeric ${expectedAdapter}.${metric}`);
    assert.ok(Number.isFinite(side[metric]) && side[metric] >= 0, `invalid ${expectedAdapter}.${metric}`);
  }
  assert.equal(typeof side.cache_status, 'string');
}

function validateCompleteMatrix(rows, baselineId) {
  const taskIds = phase11.tasks.map(task => task.id);
  const repetitions = phase11.repetitions ?? 1;
  const expectedCount = taskIds.length * phase11.modes.length * phase11.execution_modes.length * phase11.concurrency.length * repetitions;
  assert.equal(rows.length, expectedCount, `${baselineId} claim evidence must cover the complete ${expectedCount}-row task/warmth/execution-mode/concurrency/replicate matrix`);
  const seen = new Set();
  const pairedOrders = new Map();
  for (const row of rows) {
    assert.ok(taskIds.includes(row.task_id), `unknown task_id ${row.task_id}`);
    assert.ok(phase11.modes.includes(row.mode), `unknown mode ${row.mode}`);
    assert.ok(phase11.execution_modes.includes(row.execution_mode), `unknown execution_mode ${row.execution_mode}`);
    assert.ok(phase11.concurrency.includes(row.concurrency), `unknown concurrency ${row.concurrency}`);
    assert.ok(Number.isInteger(row.replicate) && row.replicate >= 1 && row.replicate <= repetitions, `invalid replicate ${row.replicate}`);
    assert.ok(['first', 'second'].includes(row.candidate_order), 'paired order must be recorded for every trial');
    const key = `${row.task_id}\u001f${row.mode}\u001f${row.execution_mode}\u001f${row.concurrency}\u001f${row.replicate}`;
    assert.ok(!seen.has(key), `duplicate Phase 11 row ${key}`);
    seen.add(key);
    const cohortKey = `${row.task_id}\u001f${row.mode}\u001f${row.execution_mode}\u001f${row.concurrency}`;
    const orders = pairedOrders.get(cohortKey) ?? new Set();
    orders.add(row.candidate_order);
    pairedOrders.set(cohortKey, orders);
    validateResultSide(row.candidate, 'controlla', row.task_id, row.mode, row.execution_mode, row.concurrency);
    validateResultSide(row.baseline, baselineId, row.task_id, row.mode, row.execution_mode, row.concurrency);
  }
  if (repetitions > 1) for (const [cohort, orders] of pairedOrders) {
    assert.equal(orders.size, 2, `${baselineId} paired order must counterbalance candidate/baseline across cohort ${cohort}`);
  }
  for (const taskId of taskIds) {
    for (const mode of phase11.modes) for (const executionMode of phase11.execution_modes) {
      for (const concurrency of phase11.concurrency) {
        for (let replicate = 1; replicate <= repetitions; replicate++) {
          const key = `${taskId}\u001f${mode}\u001f${executionMode}\u001f${concurrency}\u001f${replicate}`;
          assert.ok(seen.has(key), `missing Phase 11 row ${key}`);
        }
      }
    }
  }
}

function validateWarmupMatrix(rows, baselineId) {
  const taskIds = phase11.tasks.map(task => task.id);
  const repetitions = phase11.warmup_repetitions ?? 0;
  const expectedCount = taskIds.length * phase11.modes.length * phase11.execution_modes.length * phase11.concurrency.length * repetitions;
  assert.ok(Array.isArray(rows), `${baselineId} comparison must preserve separately recorded warmup_rows`);
  assert.equal(rows.length, expectedCount, `${baselineId} warmups must cover the configured separate warmup matrix`);
  const seen = new Set();
  for (const row of rows) {
    assert.equal(row.warmup, true, 'warmup rows must be explicitly marked and kept out of measured trials');
    assert.ok(taskIds.includes(row.task_id), `unknown warmup task_id ${row.task_id}`);
    assert.ok(phase11.modes.includes(row.mode), `unknown warmup mode ${row.mode}`);
    assert.ok(phase11.execution_modes.includes(row.execution_mode), `unknown warmup execution_mode ${row.execution_mode}`);
    assert.ok(phase11.concurrency.includes(row.concurrency), `unknown warmup concurrency ${row.concurrency}`);
    assert.ok(Number.isInteger(row.replicate) && row.replicate >= 1 && row.replicate <= repetitions, `invalid warmup replicate ${row.replicate}`);
    assert.ok(['first', 'second'].includes(row.candidate_order), 'warmup paired order must be recorded');
    const key = `${row.task_id}\u001f${row.mode}\u001f${row.execution_mode}\u001f${row.concurrency}\u001f${row.replicate}`;
    assert.ok(!seen.has(key), `duplicate warmup row ${key}`);
    seen.add(key);
    validateResultSide(row.candidate, 'controlla', row.task_id, row.mode, row.execution_mode, row.concurrency);
    validateResultSide(row.baseline, baselineId, row.task_id, row.mode, row.execution_mode, row.concurrency);
  }
}

assert.ok([2, 3].includes(results.schema_version), 'broad superiority claims require full-suite Phase 11 prompt-to-outcome evidence');
if (results.schema_version === 3) assert.equal(results.results_storage, 'comparison_files', 'schema 3 reports must use comparison evidence files');
assert.equal(results.track, 'phase11-realistic');
assert.equal(results.candidate_adapter, 'controlla');
assert.deepEqual(results.randomization, { method: 'seeded_paired_order', seed: phase11.random_seed },
  'broad superiority claims require the preregistered seeded paired-order');
requireEnvironment(results.environment);
assert.ok(Array.isArray(results.comparisons), 'claim evidence requires comparisons');

const structuredBaselines = phase11.adapters
  .filter(adapter => adapter.track === 'structured' && adapter.id !== 'controlla')
  .map(adapter => adapter.id)
  .sort();
const suppliedBaselines = results.comparisons.map(comparison => comparison.baseline_adapter).sort();
assert.deepEqual(suppliedBaselines, structuredBaselines,
  'broad superiority claim requires measured evidence against every declared structured competitor');

const comparisonReports = [];
let broadEligible = true;
for (const manifestComparison of results.comparisons) {
  let comparison = manifestComparison;
  if (results.schema_version === 3) {
    assert.equal(typeof manifestComparison.evidence_file, 'string', `evidence_file missing for ${manifestComparison.baseline_adapter}`);
    const reportsRoot = fileURLToPath(root);
    const evidencePath = resolve(reportsRoot, manifestComparison.evidence_file);
    const relativeEvidencePath = relative(reportsRoot, evidencePath);
    assert.ok(relativeEvidencePath && !relativeEvidencePath.startsWith('..') && !isAbsolute(relativeEvidencePath),
      `comparison evidence path escapes repository root for ${manifestComparison.baseline_adapter}`);
    const resolvedEvidencePath = await realpath(evidencePath);
    const realRelativeEvidencePath = relative(reportsRoot, resolvedEvidencePath);
    assert.ok(realRelativeEvidencePath && !realRelativeEvidencePath.startsWith('..') && !isAbsolute(realRelativeEvidencePath),
      `comparison evidence symlink escapes repository root for ${manifestComparison.baseline_adapter}`);
    const evidenceStats = await stat(resolvedEvidencePath);
    assert.ok(evidenceStats.isFile() && evidenceStats.size <= MAX_REPORT_BYTES,
      `comparison evidence exceeds the ${MAX_REPORT_BYTES}-byte report limit for ${manifestComparison.baseline_adapter}`);
    const evidence = JSON.parse(await readFile(resolvedEvidencePath, 'utf8'));
    assert.equal(evidence.schema_version, 2, `comparison shard schema mismatch for ${manifestComparison.baseline_adapter}`);
    assert.equal(evidence.baseline_adapter, manifestComparison.baseline_adapter, 'comparison shard adapter identity mismatch');
    assert.deepEqual(evidence.randomization, manifestComparison.randomization, 'comparison shard randomization mismatch');
    assert.deepEqual(evidence.environment, results.environment, 'comparison shard environment mismatch');
    comparison = evidence;
  }
  assert.ok(structuredBaselines.includes(comparison.baseline_adapter), `unexpected baseline ${comparison.baseline_adapter}`);
  assert.ok(Array.isArray(comparison.paired_rows), `paired_rows missing for ${comparison.baseline_adapter}`);
  assert.deepEqual(comparison.randomization, results.randomization, `randomization metadata mismatch for ${comparison.baseline_adapter}`);
  validateWarmupMatrix(comparison.warmup_rows, comparison.baseline_adapter);
  validateCompleteMatrix(comparison.paired_rows, comparison.baseline_adapter);
  const report = analyzePaired(comparison.paired_rows, { iterations: 10000, seed: 20261009 });
  const gate = evaluateClaimGate(report, phase11.claim_gates);
  assert.equal(report.all_attempt_time_to_outcome_complete, true,
    `all-attempt time-to-outcome evidence is incomplete for ${comparison.baseline_adapter}`);
  assert.equal(report.success_conditioned_latency_complete, true,
    `success-conditioned latency evidence is incomplete for ${comparison.baseline_adapter}`);
  assert.equal(report.repeated_run_reliability_complete, true,
    `repeated-run reliability evidence is incomplete for ${comparison.baseline_adapter}`);
  assert.ok(report.candidate_false_successes <= report.baseline_false_successes + phase11.claim_gates.maximum_false_success_regression,
    `candidate false successes regress against ${comparison.baseline_adapter}`);
  assert.ok(report.candidate_repeated_run_reliability >= report.baseline_repeated_run_reliability,
    `candidate repeated-run reliability regresses against ${comparison.baseline_adapter}`);
  const executionModeReports = phase11.execution_modes.map(executionMode => {
    const modeRows = comparison.paired_rows.filter(row => row.execution_mode === executionMode);
    const modeReport = analyzePaired(modeRows, { iterations: 10000, seed: 20261009 });
    const modeGate = evaluateClaimGate(modeReport, phase11.claim_gates);
    assert.ok(modeReport.candidate_false_successes <= modeReport.baseline_false_successes + phase11.claim_gates.maximum_false_success_regression,
      `candidate false successes regress for ${comparison.baseline_adapter} in ${executionMode}`);
    return { execution_mode: executionMode, report: modeReport, checks: modeGate.checks, eligible: modeGate.eligible };
  });
  const lifecycleCohortReports = phase11.modes.map(lifecycleCohort => {
    const cohortRows = comparison.paired_rows.filter(row => row.mode === lifecycleCohort);
    const cohortReport = analyzePaired(cohortRows, { iterations: 10000, seed: 20261009 });
    const cohortGate = evaluateClaimGate(cohortReport, phase11.claim_gates);
    assert.ok(cohortReport.candidate_false_successes <= cohortReport.baseline_false_successes + phase11.claim_gates.maximum_false_success_regression,
      `candidate false successes regress for ${comparison.baseline_adapter} in ${lifecycleCohort}`);
    return { lifecycle_cohort: lifecycleCohort, report: cohortReport, checks: cohortGate.checks, eligible: cohortGate.eligible };
  });
  const eligible = gate.eligible && executionModeReports.every(mode => mode.eligible)
    && lifecycleCohortReports.every(cohort => cohort.eligible);
  broadEligible &&= eligible;
  comparisonReports.push({ baseline_adapter: comparison.baseline_adapter, report, checks: gate.checks,
    execution_mode_reports: executionModeReports, lifecycle_cohort_reports: lifecycleCohortReports, eligible });
}

if (!broadEligible) {
  throw new Error(`broad superiority claim blocked: ${JSON.stringify({ comparisons: comparisonReports })}`);
}
console.log(JSON.stringify({ claim_eligible: true, scope: 'all_declared_structured_competitors', comparisons: comparisonReports }, null, 2));
