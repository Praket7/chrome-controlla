import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const serverPath = resolve(root, 'bench/fixtures/realistic/server.mjs');
const verifierPath = resolve(root, 'bench/adapters/phase11-verifier.mjs');

test('independent verifier rejects visit and forged terminal evidence', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'phase11-verifier-'));
  const server = spawn(process.execPath, [serverPath, '--port=0'], {
    cwd: root, env: { ...process.env, PHASE11_STATE_FILE: join(dir, 'state.json') },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  try {
    const base = await new Promise((resolveBase, reject) => {
      let buffer = '';
      const timer = setTimeout(() => reject(new Error('fixture server startup timeout')), 5000);
      server.stdout.setEncoding('utf8');
      server.stdout.on('data', chunk => {
        buffer += chunk;
        const newline = buffer.indexOf('\n');
        if (newline < 0) return;
        clearTimeout(timer);
        try { resolveBase(JSON.parse(buffer.slice(0, newline)).base_url); } catch (error) { reject(error); }
      });
      server.once('error', reject);
    });
    const input = { task_id: 'extract-one', task_class: 'extraction', deadline_ms: 5000 };
    await fetch(`${base}/__reset`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ task_id: input.task_id }) });
    await fetch(`${base}/fixture/records-table?task=${input.task_id}`);
    assert.equal((await verify(base, input)).passed, false, 'visited alone must fail');
    await fetch(`${base}/__event`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ task_id: input.task_id, kind: 'extracted', count: 1 }) });
    const forged = await verify(base, input);
    assert.equal(forged.passed, false, 'unauthenticated fixture events cannot prove task completion');
    assert.equal(forged.evidence.oracle_status, 'missing');
    assert.match(forged.missing_reason, /not an independent task acceptance oracle/);
  } finally {
    server.kill('SIGTERM');
    await rm(dir, { recursive: true, force: true });
  }
});

function verify(base, input) {
  return new Promise((resolveResult, reject) => {
    const child = spawn(process.execPath, [verifierPath], {
      cwd: root, env: { ...process.env, PHASE11_FIXTURE_BASE_URL: base },
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
      if (code !== 0) return reject(new Error(stderr || `verifier exited ${code}`));
      try { resolveResult(JSON.parse(stdout)); } catch (error) { reject(error); }
    });
    child.stdin.end(JSON.stringify({ input }));
  });
}
