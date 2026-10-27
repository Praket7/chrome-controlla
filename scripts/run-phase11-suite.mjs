#!/usr/bin/env node
import { randomBytes } from 'node:crypto';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

export function configuredAdapterIds(candidate, baselines) {
  const allowed = new Set(['controlla','controlla-1','controlla-2-pre-stage2','playwright-mcp','chrome-devtools-mcp','agent-browser','stagehand','browser-use']);
  const selected = baselines.split(',').map(value => value.trim()).filter(Boolean);
  if (!allowed.has(candidate) || !selected.length || selected.some(value => !allowed.has(value) || value === candidate) || new Set(selected).size !== selected.length) throw new Error('invalid candidate or baseline adapters');
  return [candidate, ...selected];
}

export function verifyAdapterVersions(versions, runtimeHash) {
  for (const [id, version] of Object.entries(versions)) {
    const expected = id === 'controlla' ? `sha256:${runtimeHash}` : id === 'playwright-mcp' ? '0.0.83' : null;
    if (!expected || version !== expected) throw new Error(`unverified adapter identity for ${id}`);
  }
}

export async function hashDirectory(directory) {
  const entries = [];
  async function walk(current) {
    for (const item of (await readdir(current, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const path = join(current, item.name);
      if (item.isDirectory()) await walk(path);
      else if (item.isFile()) entries.push([relative(directory, path).split('\\').join('/'), await readFile(path)]);
      else throw new Error(`unsupported extension build asset type: ${path}`);
    }
  }
  await walk(directory);
  if (entries.length === 0) throw new Error('extension build asset directory is empty');
  const hash = createHash('sha256');
  for (const [path, bytes] of entries) {
    const pathBytes = Buffer.from(path, 'utf8');
    const pathLength = Buffer.alloc(8);
    const contentLength = Buffer.alloc(8);
    pathLength.writeBigUInt64BE(BigInt(pathBytes.length));
    contentLength.writeBigUInt64BE(BigInt(bytes.length));
    hash.update(pathLength);
    hash.update(pathBytes);
    hash.update(contentLength);
    hash.update(bytes);
  }
  return hash.digest('hex');
}

export function requireCleanTree(root) {
  const status = execFileSync('git', ['status', '--porcelain=v1', '--untracked-files=all'], { cwd: root, encoding: 'utf8' });
  if (status.trim()) throw new Error('qualified convenience run requires a clean Git tree, including staged and untracked files');
  return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
}

export function parseAdapterVersions(raw, adapters) {
  if (!raw) throw new Error('PHASE11_CLIENT_ADAPTER_VERSIONS_JSON must provide exact observed versions for every configured adapter');
  let versions;
  try { versions = JSON.parse(raw); } catch { throw new Error('PHASE11_CLIENT_ADAPTER_VERSIONS_JSON must be valid JSON'); }
  for (const id of adapters) {
    if (typeof versions?.[id] !== 'string' || !versions[id].trim() || /^(local|unknown|latest|test)(-|$)/i.test(versions[id])) {
      throw new Error(`exact observed client adapter version required for ${id}`);
    }
  }
  return Object.fromEntries(adapters.map(id => [id, versions[id]]));
}

async function main() {
  const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
  const adapters = configuredAdapterIds(process.env.PHASE11_CANDIDATE_ID ?? 'controlla', process.env.PHASE11_BASELINE_IDS ?? 'playwright-mcp');
  const candidateSourceSha = requireCleanTree(root);
  const clientVersions = parseAdapterVersions(process.env.PHASE11_CLIENT_ADAPTER_VERSIONS_JSON, adapters);
  const stateDir = await mkdtemp(join(tmpdir(), 'controlla-phase11-'));
  const stateFile = join(stateDir, 'state.json');
  const oracleToken=randomBytes(32).toString('hex');
  const fixture = spawn(process.execPath, [resolve(root, 'bench/fixtures/realistic/server.mjs'), '--port=0'], {
    cwd: root, env: { ...process.env, PHASE11_ORACLE_TOKEN:oracleToken, PHASE11_STATE_FILE: stateFile }, stdio: ['ignore', 'pipe', 'inherit'],
  });
  try {
    const base = await new Promise((resolveBase, reject) => {
      let buffer = '';
      const timer = setTimeout(() => reject(new Error('fixture server startup timeout')), 10000);
      fixture.stdout.setEncoding('utf8');
      fixture.stdout.on('data', chunk => {
        buffer += chunk;
        const newline = buffer.indexOf('\n');
        if (newline < 0) return;
        clearTimeout(timer);
        try { resolveBase(JSON.parse(buffer.slice(0, newline)).base_url); } catch (error) { reject(error); }
      });
      fixture.once('error', reject);
    });
    const binary = process.env.PHASE11_CONTROLLA_BINARY ?? resolve(root, 'target/release/controlla-v2');
    const chrome = process.env.PHASE11_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
    const codex = process.env.PHASE11_CODEX_PATH ?? '/Users/pcg/.local/bin/codex';
    const sha = async path => createHash('sha256').update(await readFile(path)).digest('hex');
    const [runtimeHash, extensionHash] = await Promise.all([
      sha(binary), hashDirectory(resolve(root, 'extensions/chrome-controlla')),
    ]);
    verifyAdapterVersions(clientVersions, runtimeHash);
    let playwrightCliHash = null;
    if (adapters.includes('playwright-mcp')) {
      const cli = process.env.PHASE11_PLAYWRIGHT_MCP_CLI;
      const version = cli ? execFileSync(process.execPath, [cli,'--version'], {encoding:'utf8',timeout:60000}).trim() : execFileSync('npx', ['-y','@playwright/mcp@0.0.83','--version'], {encoding:'utf8',timeout:60000}).trim();
      if (version !== 'Version 0.0.83') throw new Error(`Playwright MCP version check failed: ${version}`);
      if (cli) playwrightCliHash = await sha(cli);
    }
    const chromeVersion = execFileSync(chrome, ['--version'], { encoding: 'utf8' }).trim();
    if (!chromeVersion) throw new Error('Chrome did not report its version');
    const codexVersion = execFileSync(codex, ['--version'], { encoding: 'utf8' }).trim();
    if (!codexVersion) throw new Error('Codex did not report its version');
    const dirtyTreeHash = createHash('sha256').update('').digest('hex');
    const env = {
      ...process.env,
      PHASE11_ORACLE_TOKEN:oracleToken,
      PHASE11_BASELINE_IDS: process.env.PHASE11_BASELINE_IDS ?? 'playwright-mcp',
      PHASE11_FIXTURE_BASE_URL: base,
      PHASE11_STATE_FILE: stateFile,
      PHASE11_CODEX_PATH: codex,
      PHASE11_CONTROLLA_BINARY: binary,
      PHASE11_CHROME_PATH: chrome,
      PHASE11_ENVIRONMENT_JSON: JSON.stringify({
        candidate_source_sha: candidateSourceSha,
        dirty_tree_patch_sha256: dirtyTreeHash,
        runtime_binary_sha256: runtimeHash,
        extension_build_sha256: extensionHash,
        chrome_version: chromeVersion,
        browser_version: chromeVersion,
        codex_version: codexVersion,
        os_build: `${process.platform}-${process.arch}`,
        machine_profile: `${process.platform}-${process.arch}`,
        model: process.env.PHASE11_MODEL ?? 'gpt-6-luna',
        model_settings: process.env.PHASE11_MODEL_SETTINGS ?? 'unspecified',
        prompt_hash: process.env.PHASE11_PROMPT_HASH ?? 'adapter-generated',
        token_budget: Number(process.env.PHASE11_TOKEN_BUDGET ?? 32000),
        fixture_revision: 'local-realistic-v2-server-acceptance',
        network_profile: 'loopback fixture; client and model network unrestricted',
        account_fixture_id: 'phase11-local-disposable',
        client_adapter_versions: clientVersions,
        ...(playwrightCliHash ? {playwright_mcp_cli_sha256:playwrightCliHash} : {}),
      }),
      PHASE11_RESET_COMMAND_JSON: JSON.stringify([process.execPath, resolve(root, 'bench/adapters/phase11-reset.mjs')]),
      PHASE11_VERIFIER_COMMAND_JSON: JSON.stringify([process.execPath, resolve(root, 'bench/adapters/phase11-verifier.mjs')]),
    };
    for (const id of adapters) env[`PHASE11_${id.toUpperCase().replace(/[^A-Z0-9]+/g, '_')}_COMMAND_JSON`] = JSON.stringify([process.execPath, resolve(root, 'bench/adapters/phase11-command.mjs'), id]);
    const args = [
      'bench/runners/phase11-suite.mjs',
      `--modes=${process.env.PHASE11_MODES ?? 'cold_browser,warm_browser_cold_site,warm_qualified_route,reconnect'}`,
      `--execution-modes=${process.env.PHASE11_EXECUTION_MODES ?? 'headless'}`,
      `--concurrency=${process.env.PHASE11_CONCURRENCY ?? '1'}`,
      `--repetitions=${process.env.PHASE11_REPETITIONS ?? '1'}`,
      `--deadline-ms=${process.env.PHASE11_DEADLINE_MS ?? '180000'}`,
      `--baseline-ids=${process.env.PHASE11_BASELINE_IDS ?? 'playwright-mcp'}`,
      `--output=${process.env.PHASE11_OUTPUT ?? 'bench/reports/phase11-suite-local.json'}`,
    ];
    const code = await new Promise(done => {
      const child = spawn(process.execPath, args, { cwd: root, env, stdio: 'inherit' });
      child.once('close', done);
      child.once('error', () => done(1));
    });
    process.exitCode = code;
  } finally {
    fixture.kill('SIGTERM');
    await rm(stateDir, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exit(1); });
}
