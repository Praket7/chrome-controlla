import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { hashDirectory, parseAdapterVersions, requireCleanTree } from './run-phase11-suite.mjs';

test('adapter provenance requires explicit exact versions for every configured client', () => {
  assert.throws(() => parseAdapterVersions('', ['one']), /must provide exact observed versions/);
  assert.throws(() => parseAdapterVersions('{"one":"local-command-adapter-v1"}', ['one']), /exact observed client adapter version required/);
  assert.deepEqual(parseAdapterVersions('{"one":"2.7.1"}', ['one']), { one: '2.7.1' });
});

test('qualified convenience run requires a clean Git tree', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'phase11-clean-tree-'));
  try {
    execFileSync('git', ['init', '-q'], { cwd: dir });
    execFileSync('git', ['-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '--allow-empty', '-qm', 'baseline'], { cwd: dir });
    assert.match(requireCleanTree(dir), /^[a-f0-9]{40}$/);
    await writeFile(join(dir, 'untracked.txt'), 'dirty');
    assert.throws(() => requireCleanTree(dir), /requires a clean Git tree/);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('extension build hash covers every asset path and content', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'phase11-assets-'));
  try {
    await mkdir(join(dir, 'nested'));
    await writeFile(join(dir, 'manifest.json'), '{}');
    await writeFile(join(dir, 'nested', 'background.js'), 'first');
    const first = await hashDirectory(dir);
    await writeFile(join(dir, 'nested', 'background.js'), 'second');
    assert.notEqual(await hashDirectory(dir), first);
    await writeFile(join(dir, 'nested', 'new.js'), 'new asset');
    assert.notEqual(await hashDirectory(dir), first);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('extension hash framing distinguishes embedded NULs from separate files', async () => {
  const left=await mkdtemp(join(tmpdir(),'phase11-hash-left-'));
  const right=await mkdtemp(join(tmpdir(),'phase11-hash-right-'));
  try {
    await writeFile(join(left,'a'),Buffer.from('x\0b\0y'));
    await writeFile(join(right,'a'),Buffer.from('x'));
    await writeFile(join(right,'b'),Buffer.from('y'));
    assert.notEqual(await hashDirectory(left),await hashDirectory(right));
  } finally { await rm(left,{recursive:true,force:true}); await rm(right,{recursive:true,force:true}); }
});
