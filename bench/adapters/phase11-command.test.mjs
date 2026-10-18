import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadAdapterContract, validateAdapterResult } from '../runners/phase11-runner.mjs';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const adapterPath = resolve(root, 'bench/adapters/phase11-command.mjs');

test('unsupported adapter task classes report missing evidence and leave safety counters unknown', async () => {
  const input = { task_id: 'eight-tab-pressure', task_class: 'concurrent_tabs', mode: 'cold_browser', execution_mode: 'headless', concurrency: 1 };
  const output = await invoke(input);
  assert.match(output.runner_error, /not an independent task acceptance oracle/);
  for (const field of ['focus_disruptions', 'clipboard_disruptions', 'human_interventions', 'severe_wrong_target_events', 'authority_violations', 'silent_stale_mutations']) {
    assert.equal(output[field], null, `${field} is uninstrumented and must remain missing`);
    assert.ok(output.metric_evidence[field].missing_reason);
  }
  assert.equal(output.success, false);
  assert.equal(output.independently_verified_success, false);
  assert.doesNotThrow(() => validateAdapterResult(output, awaitContract, {
    adapter: 'playwright-mcp', task_id: input.task_id, mode: input.mode,
    execution_mode: input.execution_mode, concurrency: input.concurrency,
  }));
});

const awaitContract = await loadAdapterContract();

function invoke(input) {
  return new Promise((resolveOutput, reject) => {
    const child = spawn(process.execPath, [adapterPath, 'playwright-mcp'], {
      cwd: root, env: { ...process.env, PHASE11_FIXTURE_BASE_URL: 'http://127.0.0.1:1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', code => {
      if (code !== 0) return reject(new Error(stderr || `adapter exited ${code}`));
      try { resolveOutput(JSON.parse(stdout)); } catch (error) { reject(error); }
    });
    child.stdin.end(JSON.stringify(input));
  });
}

test('generic extraction is unqualified without independent fixture acceptance', async () => {
  const output = await invoke({ task_id:'structured-extraction', task_class:'extraction', mode:'cold_browser', execution_mode:'headless', concurrency:1 });
  assert.equal(output.success,false);
  assert.equal(output.independently_verified_success,false);
  assert.match(output.runner_error,/not an independent task acceptance oracle/);
});
