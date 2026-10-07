import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { cp, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const packageDir = path.join(root, 'packages/chrome-controlla/dist');
const digest = async (file) => createHash('sha256').update(await readFile(file)).digest('hex');
const temp = await mkdtemp(path.join(os.tmpdir(), 'controlla-lifecycle-'));
const isolatedNpmConfig = path.join(temp, 'empty.npmrc');
await writeFile(isolatedNpmConfig, '');
const run = (command, args, options = {}) => {
  const windowsNpm = process.platform === 'win32' && command === 'npm';
  const executable = windowsNpm
    ? `npm.cmd ${args.map((arg) => `"${arg}"`).join(' ')}`
    : command;
  const result = spawnSync(executable, windowsNpm ? [] : args, {
    encoding: 'utf8',
    env: { ...process.env, NPM_CONFIG_USERCONFIG: isolatedNpmConfig, npm_config_userconfig: isolatedNpmConfig, NPM_CONFIG_ALLOW_SCRIPTS: '', npm_config_allow_scripts: '' },
    shell: windowsNpm,
    ...options,
  });
  assert.equal(result.status, 0, `${command} ${args.join(' ')} failed:\n${result.stderr ?? result.error?.message ?? ''}`);
  return result.stdout.trim();
};

try {
  const userData = path.join(temp, 'user-data');
  const sentinel = path.join(userData, 'keep.txt');
  await mkdir(userData);
  await writeFile(sentinel, 'user data survives package lifecycle checks');

  const archives = [];
  const previousBinary = process.env.CONTROLLA_PREVIOUS_BINARY;
  const currentBinary = process.env.CONTROLLA_CURRENT_BINARY;
  assert.equal(Boolean(previousBinary), Boolean(currentBinary), 'provide both distinct binary paths or neither');
  const binaryName = process.platform === 'win32' ? 'controlla-core.exe' : 'controlla-core';
  let binaryDigests;
  if (previousBinary && currentBinary) {
    binaryDigests = [await digest(previousBinary), await digest(currentBinary)];
    assert.notEqual(binaryDigests[0], binaryDigests[1], 'upgrade smoke requires distinct server binaries');
  }
  for (const version of ['0.1.0', '0.1.1']) {
    const staged = path.join(temp, `package-${version}`);
    await cp(packageDir, staged, { recursive: true });
    if (binaryDigests) {
      const source = version === '0.1.0' ? previousBinary : currentBinary;
      await cp(source, path.join(staged, 'bin', binaryName));
    }
    const manifestPath = path.join(staged, 'package.json');
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    manifest.version = version;
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    const name = run('npm', ['pack', '--silent', '--pack-destination', temp, staged]);
    archives.push(path.join(temp, name));
  }

  const prefix = path.join(temp, 'install prefix');
  const installedManifest = path.join(prefix, 'node_modules/chrome-controlla/package.json');
  const installedLauncher = path.join(prefix, 'node_modules/chrome-controlla/bin/controlla.cjs');
  const installedBinary = path.join(prefix, 'node_modules/chrome-controlla/bin', binaryName);
  const checkVersion = async (expected) => {
    const installed = JSON.parse(await readFile(installedManifest, 'utf8'));
    assert.equal(installed.version, expected, `expected installed package ${expected}, got ${installed.version}`);
    if (binaryDigests) {
      const expectedBinary = expected === '0.1.0' ? binaryDigests[0] : binaryDigests[1];
      assert.equal(await digest(installedBinary), expectedBinary, `installed ${expected} binary must match its staged source`);
    }
    assert.match(run(process.execPath, [installedLauncher, '--help']), /Usage: controlla/);
    assert.equal(await readFile(sentinel, 'utf8'), 'user data survives package lifecycle checks');
  };
  const install = (archive) => run('npm', ['install', '--prefix', prefix, '--ignore-scripts', '--no-audit', '--no-fund', archive]);

  install(archives[0]);
  await checkVersion('0.1.0');
  install(archives[1]);
  await checkVersion('0.1.1');
  install(archives[0]);
  await checkVersion('0.1.0');
  run('npm', ['uninstall', '--prefix', prefix, 'chrome-controlla', '--no-audit', '--no-fund']);
  await assert.rejects(readFile(installedManifest), { code: 'ENOENT' });
  assert.equal(await readFile(sentinel, 'utf8'), 'user data survives package lifecycle checks');
  const binaryNote = binaryDigests ? `distinct binary digests ${binaryDigests[0]} → ${binaryDigests[1]}` : 'same binary in both package versions';
  console.log(`Verified packed install, upgrade, rollback, uninstall, and user-data preservation (${process.platform}/${process.arch}; ${binaryNote}).`);
} finally {
  await rm(temp, { recursive: true, force: true });
}
