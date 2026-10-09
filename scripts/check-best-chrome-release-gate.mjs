import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';
import { analyzePaired, evaluateClaimGate } from '../bench/analysis/phase11-analysis.mjs';

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

const phase11 = JSON.parse(await readFile(new URL('bench/tasks/phase11-realistic.json', root), 'utf8'));
assert.equal(phase11.frozen_phase10_unchanged, true);
assert.equal(phase11.claim_gates.maximum_severe_wrong_target_events, 0);
assert.equal(phase11.claim_gates.maximum_authority_violations, 0);
assert.equal(phase11.claim_gates.maximum_silent_stale_mutations, 0);

const claimRequested = process.argv.includes('--claim');
if (!claimRequested) {
  console.log('best-chrome release structure is valid; superiority claim remains disabled until measured Phase 11 results are supplied');
  process.exit(0);
}

let results;
try {
  results = JSON.parse(await readFile(new URL('bench/reports/phase11-results.json', root), 'utf8'));
} catch (error) {
  throw new Error(`--claim requires bench/reports/phase11-results.json: ${error.message}`);
}

function requireEnvironment(environment) {
  const required = ['chrome_version','viewport','model','prompt_hash','token_budget','fixture_revision','machine_profile'];
  assert.ok(environment && typeof environment === 'object' && !Array.isArray(environment), 'claim evidence requires an environment manifest');
  for (const field of required) {
    assert.ok(field in environment, `claim evidence environment missing ${field}`);
    assert.notEqual(environment[field], '', `claim evidence environment ${field} must not be empty`);
  }
}

function validateResultSide(side, expectedAdapter, taskId, mode, concurrency) {
  assert.equal(side.adapter, expectedAdapter, `adapter identity mismatch for ${expectedAdapter}`);
  assert.equal(side.task_id, taskId, `task identity mismatch for ${expectedAdapter}`);
  assert.equal(side.mode, mode, `mode identity mismatch for ${expectedAdapter}`);
  assert.equal(side.concurrency, concurrency, `concurrency identity mismatch for ${expectedAdapter}`);
  assert.equal(typeof side.success, 'boolean');
  assert.equal(typeof side.independently_verified_success, 'boolean');
  assert.ok(!(side.independently_verified_success && !side.success), 'independent success cannot be true when acting success is false');
  assert.equal(typeof side.verifier_evidence_hash, 'string');
  assert.ok(side.verifier_evidence_hash.length >= 8, 'claim evidence requires an independent verifier evidence hash');
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
  const expectedCount = taskIds.length * phase11.modes.length * phase11.concurrency.length;
  assert.equal(rows.length, expectedCount, `${baselineId} claim evidence must cover the complete ${expectedCount}-row task/mode/concurrency matrix`);
  const seen = new Set();
  for (const row of rows) {
    assert.ok(taskIds.includes(row.task_id), `unknown task_id ${row.task_id}`);
    assert.ok(phase11.modes.includes(row.mode), `unknown mode ${row.mode}`);
    assert.ok(phase11.concurrency.includes(row.concurrency), `unknown concurrency ${row.concurrency}`);
    const key = `${row.task_id}\u001f${row.mode}\u001f${row.concurrency}`;
    assert.ok(!seen.has(key), `duplicate Phase 11 row ${key}`);
    seen.add(key);
    validateResultSide(row.candidate, 'controlla', row.task_id, row.mode, row.concurrency);
    validateResultSide(row.baseline, baselineId, row.task_id, row.mode, row.concurrency);
  }
  for (const taskId of taskIds) {
    for (const mode of phase11.modes) {
      for (const concurrency of phase11.concurrency) {
        const key = `${taskId}\u001f${mode}\u001f${concurrency}`;
        assert.ok(seen.has(key), `missing Phase 11 row ${key}`);
      }
    }
  }
}

assert.equal(results.schema_version, 2, 'broad superiority claims require full-suite Phase 11 schema_version 2 evidence');
assert.equal(results.track, 'phase11-realistic');
assert.equal(results.candidate_adapter, 'controlla');
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
for (const comparison of results.comparisons) {
  assert.ok(structuredBaselines.includes(comparison.baseline_adapter), `unexpected baseline ${comparison.baseline_adapter}`);
  assert.ok(Array.isArray(comparison.paired_rows), `paired_rows missing for ${comparison.baseline_adapter}`);
  validateCompleteMatrix(comparison.paired_rows, comparison.baseline_adapter);
  const report = analyzePaired(comparison.paired_rows, { iterations: 10000, seed: 20261009 });
  const gate = evaluateClaimGate(report, phase11.claim_gates);
  broadEligible &&= gate.eligible;
  comparisonReports.push({ baseline_adapter: comparison.baseline_adapter, report, checks: gate.checks, eligible: gate.eligible });
}

if (!broadEligible) {
  throw new Error(`broad superiority claim blocked: ${JSON.stringify({ comparisons: comparisonReports })}`);
}
console.log(JSON.stringify({ claim_eligible: true, scope: 'all_declared_structured_competitors', comparisons: comparisonReports }, null, 2));
