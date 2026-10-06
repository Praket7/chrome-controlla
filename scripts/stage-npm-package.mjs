import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const target = path.join(root, 'packages/chrome-controlla/dist');
const osNames = { darwin: 'darwin', linux: 'linux', win32: 'win32' };
const cpuNames = { x64: 'x64', arm64: 'arm64' };
const os = osNames[process.platform];
const cpu = cpuNames[process.arch];
if (!os || !cpu) throw new Error(`Unsupported package target: ${process.platform}/${process.arch}`);

const rust = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
if (rust.status !== 0) throw new Error(`Could not inspect Rust host: ${rust.stderr}`);
const host = rust.stdout.match(/^host: (.+)$/m)?.[1] ?? '';
const rustCpu = host.startsWith('aarch64-') ? 'arm64' : host.startsWith('x86_64-') ? 'x64' : '';
const rustOs = host.includes('apple-darwin') ? 'darwin' : host.includes('windows') ? 'win32' : host.includes('linux') ? 'linux' : '';
if (os !== rustOs || cpu !== rustCpu) throw new Error(`Node ${os}/${cpu} and Rust host ${host} differ; refusing to mislabel native binary`);

const core = path.join(root, 'packages/chrome-controlla/bin', os === 'win32' ? 'controlla-core.exe' : 'controlla-core');
await rm(target, { recursive: true, force: true });
await mkdir(path.join(target, 'bin'), { recursive: true });
const manifest = JSON.parse(await readFile(path.join(root, 'packages/chrome-controlla/package.json'), 'utf8'));
manifest.os = [os];
manifest.cpu = [cpu];
await writeFile(path.join(target, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
for (const file of ['controlla.cjs', 'controlla', 'controlla.cmd']) {
  await cp(path.join(root, 'packages/chrome-controlla/bin', file), path.join(target, 'bin', file));
}
await cp(core, path.join(target, 'bin', path.basename(core)));
await cp(path.join(root, 'LICENSE'), path.join(target, 'LICENSE'));
await cp(path.join(root, 'NOTICE'), path.join(target, 'NOTICE'));
