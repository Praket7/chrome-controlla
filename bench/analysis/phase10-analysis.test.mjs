import test from 'node:test';
import assert from 'node:assert/strict';
import { analyze, validateRows, validateRunManifest } from './phase10-analysis.mjs';
import { digestManifest, validateHeldout } from '../../scripts/check-phase10-heldout.mjs';
import { readFile } from 'node:fs/promises';

const root = new URL('../../', import.meta.url);
const manifest = JSON.parse(await readFile(new URL('bench/tasks/phase10-heldout.json', root), 'utf8'));
const lock = JSON.parse(await readFile(new URL('bench/baselines/versions.lock.json', root), 'utf8'));

test('held-out manifest is frozen, disjoint from pilot, and has exact strata', () => {
  assert.deepEqual(validateHeldout(manifest, lock), []);
  assert.equal(digestManifest(manifest), manifest.freeze.split_digest);
  assert.equal(manifest.tasks.filter((t) => t.critical).length, 20);
  assert.equal(new Set(manifest.tasks.map((t) => t.id)).size, 100);
});

function fixture() {
  const m = { tasks: [{ id: 'H001', critical: false }], preregistration: { bootstrap_resamples: 200, bootstrap_seed: 11, noninferiority_margin_percentage_points: 2 }, freeze: { split_digest: 'fixture', controlla_revision: 'sha' } };
  const candidates = ['Controlla', 'Baseline'];
  const runs = [];
  for (let rep = 1; rep <= 5; rep++) for (const candidate of candidates) runs.push({
    task_id: 'H001', repetition: rep, candidate, run_status: 'complete', verification_status: 'independently_verified',
    verified_success: candidate === 'Controlla' || rep !== 5, severe_wrong_target_or_authority_violation: false,
    verification_evidence: 'fixture readback', cleanup_complete: true, cache_state: rep % 2 ? 'cold' : 'warm',
    wall_time_ms: candidate === 'Controlla' ? 80 : 100, input_tokens: 10, output_tokens: 5, model_calls: candidate === 'Controlla' ? 1 : 2,
    mcp_calls: 1, browser_operations: 2, retries: 0, human_interventions: 0, focus_disruptions: 0, clipboard_disruptions: 0,
    cleanup_failures: 0, cache_preparation_ms: rep % 2 ? 2 : 0, cache_maintenance_ms: 1,
  });
  return { m, candidates, runs };
}

test('analysis accepts complete independent fixture rows and reports paired strata', () => {
  const { m, candidates, runs } = fixture();
  const report = analyze(runs, m, candidates);
  assert.equal(report.rows, 10);
  assert.equal(report.candidates.Controlla.by_cache_state.cold.runs, 3);
  assert.equal(report.candidates.Baseline.by_cache_state.warm.runs, 2);
  assert.equal(report.comparisons.Baseline.success_difference.task_clusters, 1);
  assert.equal(report.candidates.Controlla.p50_wall_time_ms, 80);
});

test('analysis rejects incomplete or unverified result rows', () => {
  const { m, candidates, runs } = fixture();
  runs[0].verification_status = 'unverified';
  assert.ok(validateRows(runs, m, candidates).some((e) => e.includes('incomplete or unverified')));
});

test('zero control model calls yield an unavailable ratio instead of invalid numbers', () => {
  const { m, candidates, runs } = fixture();
  for (const row of runs.filter((item) => item.candidate === 'Baseline')) row.model_calls = 0;
  const report = analyze(runs, m, candidates);
  assert.equal(report.comparisons.Baseline.efficiency_targets.model_call_reduction, null);
  assert.equal(report.comparisons.Baseline.efficiency_targets.model_call_reduction_pass, null);
});

test('results must name the exact frozen candidate revision and task split', () => {
  const { m } = fixture();
  assert.deepEqual(validateRunManifest({ manifest_digest: 'fixture', controlla_revision: 'sha' }, m), []);
  assert.equal(validateRunManifest({ manifest_digest: 'fixture', controlla_revision: 'other' }, m).length, 1);
});
