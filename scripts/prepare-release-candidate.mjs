import { createHash } from 'node:crypto';
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, 'dist', 'release-candidate');
const run = (cmd, args, options = {}) => {
  const result = spawnSync(cmd, args, { cwd: root, encoding: 'utf8', stdio: 'inherit', ...options });
  if (result.status !== 0) throw new Error(`${cmd} ${args.join(' ')} failed (${result.status ?? result.signal})`);
  return result.stdout ?? '';
};

const workingTree = spawnSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' });
if (workingTree.status !== 0) throw new Error('Could not inspect the release source tree');
if (workingTree.stdout.trim()) throw new Error('Commit all source changes before preparing a release candidate');

run('cargo', ['fmt', '--all', '--', '--check']);
run('cargo', ['clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings']);
run('cargo', ['test', '--workspace', '--locked']);
run('npm', ['run', 'check:docs']);
run('npm', ['run', 'check:provenance']);
run('npm', ['run', 'check:clients']);
run('npm', ['run', 'check:apps']);
run('npm', ['run', 'check:bench']);
run('sh', ['scripts/check-dependencies.sh']);
run('node', ['extensions/chrome-controlla/test-background.cjs']);
run('sh', ['scripts/package-build.sh']);
run('sh', ['scripts/package-check.sh']);

const packageDir = path.join(root, 'packages/chrome-controlla/dist');
const packageJson = JSON.parse(await readFile(path.join(packageDir, 'package.json'), 'utf8'));
const extensionManifest = JSON.parse(await readFile(path.join(root, 'extensions/chrome-controlla/manifest.json'), 'utf8'));
if (extensionManifest.version !== packageJson.version) throw new Error('MCP package and Chrome extension versions must match');
const target = `${process.platform}-${process.arch}`;
await rm(out, { recursive: true, force: true });
await mkdir(out, { recursive: true });
const isolatedNpmConfig = path.join(out, '.empty.npmrc');
await writeFile(isolatedNpmConfig, '');
run('npm', ['pack', packageDir, '--pack-destination', out, '--json'], {
  stdio: 'pipe',
  env: { ...process.env, NPM_CONFIG_USERCONFIG: isolatedNpmConfig, npm_config_userconfig: isolatedNpmConfig },
});
await rm(isolatedNpmConfig, { force: true });
const packageArchive = (await readdir(out)).find((name) => name.endsWith('.tgz'));
if (!packageArchive) throw new Error('npm pack did not produce an archive');

const extensionArchive = `chrome-controlla-extension-${packageJson.version}.tar.gz`;
run('tar', ['-czf', path.join(out, extensionArchive), '-C', path.join(root, 'extensions/chrome-controlla'), 'manifest.json', 'background.js', 'popup.html', 'popup.js', 'README.md']);

const metadata = JSON.parse(run('cargo', ['metadata', '--locked', '--format-version', '1'], { stdio: 'pipe' }));
if (!metadata.resolve) throw new Error('cargo metadata did not resolve the Controlla CLI dependency graph');
const binary = metadata.packages.find((item) => item.name === 'controlla-runtime' && item.targets.some((entry) => entry.name === 'controlla' && entry.kind.includes('bin')));
const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
const reachable = new Set();
const visit = (id) => {
  if (reachable.has(id)) return;
  reachable.add(id);
  for (const dependency of nodes.get(id)?.deps ?? []) {
    if (dependency.dep_kinds.some((kind) => kind.kind === null)) visit(dependency.pkg);
  }
};
if (!binary) throw new Error('cargo metadata did not include the Controlla CLI package');
visit(metadata.resolve.nodes.find((node) => node.id === binary.id).id);
const runtimePackages = metadata.packages.filter((item) => reachable.has(item.id) && item.id !== binary.id);
const runtimeRootRef = `pkg:cargo/${binary.name}@${binary.version}`;
const components = runtimePackages.map((item) => ({
  type: 'library',
  name: item.name,
  version: item.version,
  purl: `pkg:cargo/${item.name}@${item.version}`,
  'bom-ref': `pkg:cargo/${item.name}@${item.version}`,
  ...(item.checksum ? { hashes: [{ alg: 'SHA-256', content: item.checksum }] } : {}),
}));
const sbom = {
  bomFormat: 'CycloneDX',
  specVersion: '1.6',
  version: 1,
  metadata: {
    timestamp: new Date().toISOString(),
    component: { type: 'application', name: 'chrome-controlla', version: packageJson.version, 'bom-ref': runtimeRootRef },
    properties: [
      { name: 'controlla.host', value: target },
      { name: 'controlla.release-status', value: 'local-candidate-not-published' },
    ],
  },
  components,
  dependencies: [{
    ref: runtimeRootRef,
    dependsOn: (nodes.get(binary.id)?.deps ?? [])
      .filter((dependency) => dependency.dep_kinds.some((kind) => kind.kind === null))
      .map((dependency) => runtimePackages.find((child) => child.id === dependency.pkg))
      .filter(Boolean)
      .map((child) => `pkg:cargo/${child.name}@${child.version}`),
  }, ...runtimePackages.map((item) => ({
    ref: `pkg:cargo/${item.name}@${item.version}`,
    dependsOn: (nodes.get(item.id)?.deps ?? [])
      .filter((dependency) => dependency.dep_kinds.some((kind) => kind.kind === null))
      .map((dependency) => runtimePackages.find((child) => child.id === dependency.pkg))
      .filter(Boolean)
      .map((child) => `pkg:cargo/${child.name}@${child.version}`),
  }))],
};
await writeFile(path.join(out, 'SBOM.cdx.json'), `${JSON.stringify(sbom, null, 2)}\n`);

const git = run('git', ['rev-parse', 'HEAD'], { stdio: 'pipe' }).trim();
const files = (await readdir(out)).filter((name) => name !== 'release-manifest.json' && name !== 'SHA256SUMS').sort();
const digests = [];
for (const name of files) {
  const bytes = await readFile(path.join(out, name));
  digests.push(`${createHash('sha256').update(bytes).digest('hex')}  ${name}`);
}
await writeFile(path.join(out, 'SHA256SUMS'), `${digests.join('\n')}\n`);
await writeFile(path.join(out, 'release-manifest.json'), `${JSON.stringify({
  schema_version: 1,
  version: packageJson.version,
  source_commit: git,
  host: target,
  status: 'local-candidate-not-published',
  artifacts: [...files, 'SHA256SUMS'],
  verification: ['host-native package build', 'package archive install/lifecycle checks'],
  live_acceptance: 'not run; requires explicit Chrome profile and app/client connection',
  runtime_dependencies: runtimePackages.map((item) => `${item.name}@${item.version}`),
}, null, 2)}\n`);
console.log(`Prepared local release candidate ${packageJson.version} for ${target}: ${out}`);
