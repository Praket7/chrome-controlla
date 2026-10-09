import test from 'node:test';
import assert from 'node:assert/strict';
import { analyzePaired, evaluateClaimGate } from './phase11-analysis.mjs';

const winningRows = Array.from({ length: 24 }, (_, index) => ({
  task_id: `task-${index}`,
  candidate: {
    success: true,
    wall_ms: 700,
    model_round_trips: 2,
    severe_wrong_target_events: 0,
    authority_violations: 0,
    silent_stale_mutations: 0,
  },
  baseline: {
    success: true,
    wall_ms: 1000,
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
