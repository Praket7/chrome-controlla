import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';
import { analyzePaired, evaluateClaimGate } from '../bench/analysis/phase11-analysis.mjs';

const root = new URL('../', import.meta.url);
const requiredFiles = [
  'bench/phase10-pilot.test.mjs',
  'scripts/check-phase10-heldout.mjs',
  'bench/tasks/phase11-realistic.json',
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
assert.equal(results.schema_version, 1);
assert.equal(results.track, 'phase11-realistic');
assert.ok(Array.isArray(results.paired_rows) && results.paired_rows.length >= phase11.tasks.length,
  'claim evidence must contain paired results across the realistic task track');
for (const row of results.paired_rows) {
  for (const side of ['candidate','baseline']) {
    assert.equal(typeof row[side].success, 'boolean');
    for (const metric of ['wall_ms','model_round_trips','severe_wrong_target_events','authority_violations','silent_stale_mutations']) {
      assert.equal(typeof row[side][metric], 'number', `missing numeric ${side}.${metric}`);
    }
  }
}
const report = analyzePaired(results.paired_rows, { iterations: 10000, seed: 20261009 });
const gate = evaluateClaimGate(report, phase11.claim_gates);
if (!gate.eligible) {
  throw new Error(`superiority claim blocked: ${JSON.stringify({ report, checks: gate.checks })}`);
}
console.log(JSON.stringify({ claim_eligible: true, report, checks: gate.checks }, null, 2));
