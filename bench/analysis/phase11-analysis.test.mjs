import test from 'node:test';
import assert from 'node:assert/strict';
import { analyzePaired, effectiveSuccess, evaluateClaimGate, REQUIRED_METRIC_EVIDENCE_FIELDS } from './phase11-analysis.mjs';

function completeMetricEvidence(result) {
  const values = Object.fromEntries(REQUIRED_METRIC_EVIDENCE_FIELDS.map(field => [field,
    ['success','independently_verified_success','cleanup_completed'].includes(field) ? true
      : ['cache_status','cleanup_outcome'].includes(field) ? 'observed' : 1]));
  return {
    ...values,
    ...result,
    metric_evidence: Object.fromEntries(REQUIRED_METRIC_EVIDENCE_FIELDS.map(field => [field,
      { s: 'observed', p: `fixture_${field}` }]))
  };
}

const winningRows = Array.from({ length: 24 }, (_, index) => ({
  task_id: `task-${index}`,
    candidate: completeMetricEvidence({
      success: true,
      wall_ms: 700,
      prompt_to_verified_ms: 800,
      prompt_to_final_ms: 900,
    model_round_trips: 2,
    severe_wrong_target_events: 0,
    authority_violations: 0,
    silent_stale_mutations: 0,
  }),
  baseline: completeMetricEvidence({
      success: true,
      wall_ms: 1000,
      prompt_to_verified_ms: 1200,
      prompt_to_final_ms: 1400,
    model_round_trips: 4,
    severe_wrong_target_events: 0,
    authority_violations: 0,
    silent_stale_mutations: 0,
  }),
}));

test('phase11 gates allow only a paired safety-clean measured win', () => {
  const report = analyzePaired(winningRows, { iterations: 1000, seed: 7 });
  const gate = evaluateClaimGate(report);
  assert.equal(gate.eligible, true);
  assert.ok(report.model_round_trip_reduction >= 0.30);
  assert.ok(report.median_wall_time_reduction >= 0.25);
  assert.ok(report.prompt_to_verified_median_reduction >= 0.25);
  assert.ok(report.prompt_to_verified_p95_reduction >= 0.25);
  assert.ok(report.prompt_to_final_median_reduction >= 0.25);
  assert.ok(report.prompt_to_final_p95_reduction >= 0.25);
});

test('p95 uses nearest-rank exact percentile and bootstrap resamples task clusters', () => {
  const rows = Array.from({ length: 20 }, (_, index) => ({
    task_id: `task-${Math.floor(index / 2)}`,
    candidate: { success: true, wall_ms: index + 1, prompt_to_verified_ms: index + 1, prompt_to_final_ms: index + 1,
      model_round_trips: 1, severe_wrong_target_events: 0, authority_violations: 0, silent_stale_mutations: 0 },
    baseline: { success: true, wall_ms: index + 1, prompt_to_verified_ms: index + 1, prompt_to_final_ms: index + 1,
      model_round_trips: 1, severe_wrong_target_events: 0, authority_violations: 0, silent_stale_mutations: 0 },
  }));
  const report = analyzePaired(rows, { iterations: 100, seed: 31 });
  assert.equal(report.candidate_p95_prompt_to_verified_ms, 19);
  assert.equal(report.paired_rows, 20);
  assert.equal(report.paired_task_clusters, 10);
});

test('prompt-to-final p95 regression blocks the claim gate', () => {
  const rows = structuredClone(winningRows);
  rows.forEach((row, index) => {
    row.candidate.prompt_to_final_ms = index < 2 ? 5000 : 700;
    row.baseline.prompt_to_final_ms = 1000;
  });
  const report = analyzePaired(rows, { iterations: 500, seed: 12 });
  assert.ok(report.prompt_to_final_median_reduction >= 0.25);
  assert.ok(report.prompt_to_final_p95_reduction < 0);
  assert.equal(evaluateClaimGate(report).checks.prompt_to_final_p95, false);
});

test('prompt-to-verified release gate rejects p95 regressions even when median improves', () => {
  const rows = structuredClone(winningRows);
  rows.forEach((row, index) => {
    row.candidate.prompt_to_verified_ms = index < 2 ? 5000 : 700;
    row.baseline.prompt_to_verified_ms = 1000;
  });
  const report = analyzePaired(rows, { iterations: 1000, seed: 8 });
  assert.ok(report.prompt_to_verified_median_reduction >= 0.25);
  assert.ok(report.prompt_to_verified_p95_reduction < 0);
  const gate = evaluateClaimGate(report);
  assert.equal(gate.eligible, false);
  assert.equal(gate.checks.prompt_to_verified_p95, false);
});

test('missing prompt-to-verified evidence cannot qualify a release claim', () => {
  const rows = structuredClone(winningRows);
  for (const row of rows) delete row.candidate.prompt_to_verified_ms;
  const gate = evaluateClaimGate(analyzePaired(rows, { iterations: 100, seed: 10 }));
  assert.equal(gate.eligible, false);
  assert.equal(gate.checks.prompt_to_verified_evidence, false);
});

test('claim eligibility requires every required metric to be observed with provenance', () => {
  const absentMetric = structuredClone(winningRows);
  delete absentMetric[0].candidate.external_controlla_calls;
  const absentGate = evaluateClaimGate(analyzePaired(absentMetric, { iterations: 100, seed: 10 }));
  assert.equal(absentGate.eligible, false);
  assert.equal(absentGate.checks.metric_evidence, false);

  const unavailableMetric = structuredClone(winningRows);
  unavailableMetric[0].candidate.output_bytes = null;
  unavailableMetric[0].candidate.metric_evidence.output_bytes = {
    s: 'unavailable', p: 'runner_output_counter', r: 'output boundary was not observed'
  };
  const unavailableGate = evaluateClaimGate(analyzePaired(unavailableMetric, { iterations: 100, seed: 10 }));
  assert.equal(unavailableGate.eligible, false);
  assert.equal(unavailableGate.checks.metric_evidence, false);
  assert.equal(unavailableMetric[0].candidate.output_bytes, null, 'unavailable output bytes must not be converted to zero');

  const missingProvenance = structuredClone(winningRows);
  delete missingProvenance[0].candidate.metric_evidence.browser_cdp_commands.p;
  const provenanceGate = evaluateClaimGate(analyzePaired(missingProvenance, { iterations: 100, seed: 10 }));
  assert.equal(provenanceGate.eligible, false);
  assert.equal(provenanceGate.checks.metric_evidence, false);
});

test('one severe wrong-target event blocks the superiority gate', () => {
  const rows = structuredClone(winningRows);
  rows[0].candidate.severe_wrong_target_events = 1;
  const gate = evaluateClaimGate(analyzePaired(rows, { iterations: 500, seed: 3 }));
  assert.equal(gate.eligible, false);
  assert.equal(gate.checks.severe_wrong_target, false);
});

test('latency accounting retains failed runs rather than conditioning on success', () => {
  const rows = structuredClone(winningRows.slice(0, 4));
  rows[0].candidate.success = false;
  rows[0].candidate.wall_ms = 5000;
  const report = analyzePaired(rows, { iterations: 500, seed: 5 });
  assert.equal(report.paired_rows, 4);
  assert.ok(report.candidate_median_wall_ms >= 700);
});

test('phase11 reports all-attempt time, success-conditioned latency, repeated reliability, and false successes', () => {
  const rows = Array.from({ length: 10 }, (_, index) => {
    const task_id = `task-${Math.floor(index / 5)}`;
    const candidateSuccess = index !== 0;
    const baselineSuccess = true;
    return {
      task_id,
      mode: 'warm_qualified_route', execution_mode: 'background', concurrency: 1,
      candidate: { success: true, independently_verified_success: candidateSuccess,
        wall_ms: candidateSuccess ? 500 : 5000, prompt_to_verified_ms: candidateSuccess ? 400 : null,
        prompt_to_verifier_decision_ms: 5000, model_round_trips: 1, severe_wrong_target_events: 0,
        authority_violations: 0, silent_stale_mutations: 0 },
      baseline: { success: true, independently_verified_success: baselineSuccess,
        wall_ms: 900, prompt_to_verified_ms: 800, prompt_to_verifier_decision_ms: 850,
        model_round_trips: 2, severe_wrong_target_events: 0, authority_violations: 0, silent_stale_mutations: 0 },
    };
  });
  const report = analyzePaired(rows, { iterations: 100, seed: 43 });
  assert.equal(report.candidate_false_successes, 1);
  assert.equal(report.baseline_false_successes, 0);
  assert.equal(report.candidate_repeated_run_reliability, 0.5);
  assert.equal(report.baseline_repeated_run_reliability, 1);
  assert.equal(report.candidate_median_all_attempt_time_to_outcome_ms, 400);
  assert.equal(report.candidate_median_success_conditioned_prompt_to_verified_ms, 400);
  assert.equal(report.candidate_success_conditioned_attempts, 9);
});

test('independent verification overrides a self-reported acting success', () => {
  assert.equal(effectiveSuccess({ success: true, independently_verified_success: false }), false);
  assert.equal(effectiveSuccess({ success: true, independently_verified_success: true }), true);
  assert.equal(effectiveSuccess({ success: false, independently_verified_success: true }), false);
  assert.equal(effectiveSuccess({ success: true }), true);

  const rows = structuredClone(winningRows.slice(0, 4));
  for (const row of rows) {
    row.candidate.independently_verified_success = true;
    row.baseline.independently_verified_success = true;
  }
  rows[0].candidate.independently_verified_success = false;
  const report = analyzePaired(rows, { iterations: 500, seed: 9 });
  assert.equal(report.candidate_verified_successes, 3);
  assert.equal(report.baseline_verified_successes, 4);
  assert.equal(report.success_delta, -0.25);
});
