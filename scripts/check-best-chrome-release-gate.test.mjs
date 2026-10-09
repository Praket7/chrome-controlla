import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const reportPath = resolve(root, 'bench/reports/phase11-results.json');
const phase11 = JSON.parse(await readFile(resolve(root, 'bench/tasks/phase11-realistic.json'), 'utf8'));

function result(adapter, taskId, mode, concurrency, wallMs, modelRoundTrips) {
  return {
    adapter,
    task_id: taskId,
    mode,
    concurrency,
    success: true,
    independently_verified_success: true,
    verifier_evidence_hash: `evidence-${adapter}-${taskId}-${mode}-${concurrency}`,
    wall_ms: wallMs,
    model_round_trips: modelRoundTrips,
    model_tokens: modelRoundTrips * 100,
    mcp_calls: 2,
    browser_ops: 3,
    observation_bytes: 1024,
    screenshot_bytes: 0,
    retries: 0,
    recovery_events: 0,
    cache_status: mode === 'warm_skill' ? 'hit' : 'miss',
    verification_ms: 5,
    cpu_ms: 10,
    peak_rss_bytes: 1024,
    focus_disruptions: 0,
    clipboard_disruptions: 0,
    human_interventions: 0,
    severe_wrong_target_events: 0,
    authority_violations: 0,
    silent_stale_mutations: 0
  };
}

function completeReport() {
  const structuredBaselines = phase11.adapters
    .filter(adapter => adapter.track === 'structured' && adapter.id !== 'controlla')
    .map(adapter => adapter.id);
  return {
    schema_version: 2,
    track: 'phase11-realistic',
    candidate_adapter: 'controlla',
    environment: {
      chrome_version: 'test-chrome',
      viewport: '1280x720',
      model: 'test-model',
      prompt_hash: 'prompt-v1',
      token_budget: 1000,
      fixture_revision: 'fixture-v1',
      machine_profile: 'ci-test'
    },
    comparisons: structuredBaselines.map(baselineId => ({
      baseline_adapter: baselineId,
      paired_rows: phase11.tasks.flatMap(task => phase11.modes.flatMap(mode => phase11.concurrency.map(concurrency => ({
        task_id: task.id,
        mode,
        concurrency,
        candidate: result('controlla', task.id, mode, concurrency, 700, 2),
        baseline: result(baselineId, task.id, mode, concurrency, 1000, 4)
      }))))
    }))
  };
}

function runClaimGate() {
  return spawnSync(process.execPath, [resolve(root, 'scripts/check-best-chrome-release-gate.mjs'), '--claim'], {
    cwd: root,
    encoding: 'utf8'
  });
}

async function writeReport(report) {
  await mkdir(dirname(reportPath), { recursive: true });
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

test('claim gate passes only complete independently verified structured competitor evidence', async () => {
  try {
    const good = completeReport();
    await writeReport(good);
    const passing = runClaimGate();
    assert.equal(passing.status, 0, passing.stderr || passing.stdout);
    assert.match(passing.stdout, /"claim_eligible": true/);

    const missingRow = structuredClone(good);
    missingRow.comparisons[0].paired_rows.pop();
    await writeReport(missingRow);
    const incomplete = runClaimGate();
    assert.notEqual(incomplete.status, 0);
    assert.match(`${incomplete.stderr}\n${incomplete.stdout}`, /complete 144-row task\/mode\/concurrency matrix/);

    const forgedVerification = structuredClone(good);
    forgedVerification.comparisons[0].paired_rows[0].candidate.verifier_evidence_hash = '';
    await writeReport(forgedVerification);
    const forged = runClaimGate();
    assert.notEqual(forged.status, 0);
    assert.match(`${forged.stderr}\n${forged.stdout}`, /independent verifier evidence hash/);
  } finally {
    await rm(reportPath, { force: true });
  }
});
