import assert from 'node:assert/strict';
import { test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';
import { parseCommand, runBenchmark, validateAdapterResult, loadAdapterContract } from './phase11-runner.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const mock = resolve(here, 'mock-phase11-command.mjs');
const node = process.execPath;
const environment = {
  chrome_version: 'test-chrome',
  viewport: '1280x720',
  model: 'test-model',
  prompt_hash: 'prompt-v1',
  token_budget: 1000,
  fixture_revision: 'fixture-v1',
  machine_profile: 'ci-test'
};

function adapter(id, wall, rounds) {
  return [node, mock, 'adapter', id, String(wall), String(rounds)];
}

const reset = [node, mock, 'reset'];
const verifier = [node, mock, 'verify'];

test('parseCommand rejects shell strings and accepts argv JSON', () => {
  assert.deepEqual(parseCommand('["node","adapter.mjs"]', 'TEST'), ['node', 'adapter.mjs']);
  assert.throws(() => parseCommand('node adapter.mjs', 'TEST'), /valid JSON/);
  assert.throws(() => parseCommand('[]', 'TEST'), /non-empty JSON array/);
});

test('validateAdapterResult rejects identity drift', async () => {
  const contract = await loadAdapterContract();
  const result = {
    adapter: 'wrong', task_id: 'task', mode: 'cold_unknown', concurrency: 1,
    success: true, independently_verified_success: true, wall_ms: 1, model_round_trips: 0,
    model_tokens: 0, mcp_calls: 0, browser_ops: 0, observation_bytes: 0, screenshot_bytes: 0,
    retries: 0, recovery_events: 0, cache_status: 'miss', verification_ms: 0, cpu_ms: 0,
    peak_rss_bytes: 0, focus_disruptions: 0, clipboard_disruptions: 0, human_interventions: 0,
    severe_wrong_target_events: 0, authority_violations: 0, silent_stale_mutations: 0
  };
  assert.throws(() => validateAdapterResult(result, contract, {
    adapter: 'controlla', task_id: 'task', mode: 'cold_unknown', concurrency: 1
  }), /identity/);
});

test('runner resets, independently verifies, preserves paired failures, and emits full matrix subset', async () => {
  const report = await runBenchmark({
    baselineId: 'playwright-mcp',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: {
      controlla: adapter('controlla', 80, 1),
      'playwright-mcp': adapter('playwright-mcp', 120, 3)
    },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['warm_skill'],
    concurrencyLevels: [1],
    deadlineMs: 5000
  });
  assert.equal(report.schema_version, 1);
  assert.equal(report.candidate_adapter, 'controlla');
  assert.equal(report.baseline_adapter, 'playwright-mcp');
  assert.equal(report.paired_rows.length, 12);
  for (const row of report.paired_rows) {
    assert.equal(row.candidate.success, true);
    assert.equal(row.candidate.independently_verified_success, true);
    assert.equal(row.baseline.independently_verified_success, true);
    assert.equal(row.candidate.wall_ms, 80);
    assert.equal(row.baseline.wall_ms, 120);
    assert.match(row.candidate.verifier_evidence_hash, /^mock-/);
    assert.equal(row.candidate.severe_wrong_target_events, 0);
    assert.equal(row.candidate.authority_violations, 0);
    assert.equal(row.candidate.silent_stale_mutations, 0);
  }
});

test('computer-use cannot be used as the structured release baseline', async () => {
  await assert.rejects(() => runBenchmark({
    baselineId: 'computer-use',
    fixtureBaseUrl: 'https://fixtures.example.test/',
    commands: { controlla: adapter('controlla', 80, 1), 'computer-use': adapter('computer-use', 100, 2) },
    resetCommand: reset,
    verifierCommand: verifier,
    environment,
    modes: ['cold_unknown'],
    concurrencyLevels: [1],
    deadlineMs: 5000
  }), /separate vision track/);
});
