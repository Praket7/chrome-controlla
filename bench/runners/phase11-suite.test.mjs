import assert from 'node:assert/strict';
import { test } from 'node:test';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runFullSuite, structuredBaselineIds } from './phase11-suite.mjs';
import { loadPhase11Config } from './phase11-runner.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const mock = resolve(here, 'mock-phase11-command.mjs');
const node = process.execPath;
const environment = {
  chrome_version: 'test-chrome', viewport: '1280x720', model: 'test-model', prompt_hash: 'prompt-v1',
  token_budget: 1000, fixture_revision: 'fixture-v1', machine_profile: 'ci-test'
};
const adapter = (id, wall, rounds) => [node, mock, 'adapter', id, String(wall), String(rounds)];
const reset = [node, mock, 'reset'];
const verifier = [node, mock, 'verify'];

test('full suite covers every declared structured baseline and excludes vision track', async () => {
  const baselines = await structuredBaselineIds();
  const config = await loadPhase11Config();
  assert.deepEqual(baselines.sort(), ['agent-browser','browser-use','chrome-devtools-mcp','controlla-1','controlla-2-pre-stage2','playwright-mcp','stagehand'].sort());
  const commands = { controlla: adapter('controlla', 80, 1) };
  for (const [index, id] of baselines.entries()) commands[id] = adapter(id, 100 + index, 3);
  const report = await runFullSuite({
    fixtureBaseUrl: 'https://fixtures.example.test/', commands, resetCommand: reset, verifierCommand: verifier,
    environment, modes: ['warm_skill'], concurrencyLevels: [1], deadlineMs: 5000,
    repetitions: 1,
  });
  assert.equal(report.schema_version, 2);
  assert.equal(report.comparisons.length, baselines.length);
  assert.deepEqual(report.comparisons.map(item => item.baseline_adapter).sort(), baselines.sort());
  for (const comparison of report.comparisons) {
    assert.equal(comparison.paired_rows.length, config.tasks.length);
    assert.ok(comparison.paired_rows.every(row => row.candidate.independently_verified_success));
    assert.ok(comparison.paired_rows.every(row => row.baseline.independently_verified_success));
  }
});

test('historical Controlla 1 can be paired against the frozen pre-Stage-2 Controlla 2 revision', async () => {
  const baseline = 'controlla-2-pre-stage2';
  const report = await runFullSuite({
    candidateId: 'controlla-1', baselineIds: [baseline], fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: {
      'controlla-1': adapter('controlla-1', 120, 4),
      [baseline]: adapter(baseline, 100, 3),
    }, resetCommand: reset, verifierCommand: verifier, environment,
    modes: ['warm_skill'], concurrencyLevels: [1], deadlineMs: 5000,
    repetitions: 1,
  });
  assert.equal(report.candidate_adapter, 'controlla-1');
  assert.deepEqual(report.comparisons.map(item => item.baseline_adapter), [baseline]);
  assert.ok(report.comparisons[0].paired_rows.every(row => row.candidate.adapter === 'controlla-1'));
});

test('full suite fails closed when any structured baseline command is missing', async () => {
  await assert.rejects(() => runFullSuite({
    fixtureBaseUrl: 'https://fixtures.example.test/', commands: { controlla: adapter('controlla', 80, 1) },
    resetCommand: reset, verifierCommand: verifier, environment, modes: ['warm_skill'], concurrencyLevels: [1], deadlineMs: 5000, repetitions: 1
  }), /missing command for structured baseline/);
});
