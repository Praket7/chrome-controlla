import test from 'node:test';
import assert from 'node:assert/strict';
import { analyzePaired, effectiveSuccess, evaluateClaimGate } from './phase11-analysis.mjs';

const winningRows = Array.from({ length: 24 }, (_, index) => ({
  task_id: `task-${index}`,
    candidate: {
      success: true,
      wall_ms: 700,
      prompt_to_verified_ms: 800,
      prompt_to_final_ms: 900,
    model_round_trips: 2,
    severe_wrong_target_events: 0,
    authority_violations: 0,
    silent_stale_mutations: 0,
  },
  baseline: {
      success: true,
      wall_ms: 1000,
      prompt_to_verified_ms: 1200,
      prompt_to_final_ms: 1400,
    model_round_trips: 4,
    severe_wrong_target_events: 0,
    authority_violations: 0,
    silent_stale_mutations: 0,
  },
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
