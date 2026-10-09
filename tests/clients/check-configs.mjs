import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { generate, install } from '../../integrations/generate-config.mjs';

const binary = '/opt/chrome-controlla/controlla';
const state = '/var/lib/chrome-controlla';
const clients = ['freebuff', 'opencode-v1', 'opencode-v2'];
const generated = clients.map((client) => generate(client, binary, state));
const entries = generated.map((config) => {
  const entry = config.mcpServers?.['chrome-controlla'] ??
    config.mcp?.['chrome-controlla'] ??
    config.mcp?.servers?.['chrome-controlla'];
  assert.ok(entry, 'config contains the named server');
  return entry;
});
assert.deepEqual(entries[0], {
  command: binary,
  args: ['mcp'],
  env: {
    CONTROLLA_STATE_DIR: path.join(state, 'freebuff'),
  },
});
assert.deepEqual(entries[1], {
  type: 'local',
  command: [binary, 'mcp'],
  enabled: true,
  environment: {
    CONTROLLA_STATE_DIR: path.join(state, 'opencode-v1'),
  },
});
assert.deepEqual(entries[2], {
  type: 'local',
  command: [binary, 'mcp'],
  environment: {
    CONTROLLA_STATE_DIR: path.join(state, 'opencode-v2'),
  },
});
assert.equal(new Set(entries.map((entry) => entry.env?.CONTROLLA_STATE_DIR ?? entry.environment.CONTROLLA_STATE_DIR)).size, 3);
assert.match(generate('claude', binary, state), /^claude mcp add .*--transport stdio --scope user chrome-controlla -- '\/opt\/chrome-controlla\/controlla' mcp/m);
const quotedClaude = generate('claude', "/tmp/agent's $(touch nope)/controlla", "/tmp/state with spaces");
assert.ok(quotedClaude.includes("'/tmp/agent'\"'\"'s $(touch nope)/controlla'"));
assert.ok(quotedClaude.includes(`'CONTROLLA_STATE_DIR=${path.join('/tmp/state with spaces', 'claude')}'`));
assert.throws(() => generate('chatgpt', binary, state), /Unsupported local-stdio/);
assert.throws(() => generate('freebuff', 'controlla', state), /absolute paths/);
const tempDir = await mkdtemp(path.join(os.tmpdir(), 'controlla-config-install-'));
try {
  const configPath = path.join(tempDir, 'freebuff.json');
  await writeFile(configPath, JSON.stringify({ keep: { enabled: true }, mcpServers: { other: { command: 'safe' } } }, null, 2));
  const beforeMode = (await stat(configPath)).mode & 0o777;
  assert.deepEqual(await install('freebuff', configPath, binary, state), { installed: true, path: configPath });
  const installedText = await readFile(configPath, 'utf8');
  const installed = JSON.parse(installedText);
  assert.deepEqual(installed.keep, { enabled: true });
  assert.deepEqual(installed.mcpServers.other, { command: 'safe' });
  assert.deepEqual(installed.mcpServers['chrome-controlla'], entries[0]);
  assert.equal((await stat(configPath)).mode & 0o777, beforeMode);
  assert.deepEqual(await install('freebuff', configPath, binary, state), { installed: false, path: configPath });
  assert.equal(await readFile(configPath, 'utf8'), installedText, 'identical install is a no-op');

  const conflictPath = path.join(tempDir, 'conflict.json');
  const conflictText = JSON.stringify({ mcpServers: { 'chrome-controlla': { command: '/different' } } });
  await writeFile(conflictPath, conflictText);
  await assert.rejects(install('freebuff', conflictPath, binary, state), /different chrome-controlla entry/);
  assert.equal(await readFile(conflictPath, 'utf8'), conflictText, 'conflicting entry remains untouched');

  const malformedPath = path.join(tempDir, 'malformed.json');
  await writeFile(malformedPath, '{broken');
  await assert.rejects(install('opencode-v1', malformedPath, binary, state), /not valid JSON/);
  assert.equal(await readFile(malformedPath, 'utf8'), '{broken');
  await assert.rejects(install('claude', path.join(tempDir, 'claude.json'), binary, state), /run the generated command yourself/);
  await assert.rejects(install('freebuff', 'relative.json', binary, state), /must be absolute/);
  const cliConfigPath = path.join(tempDir, 'cli config.json');
  const generatorPath = fileURLToPath(new URL('../../integrations/generate-config.mjs', import.meta.url));
  const cliResult = execFileSync(process.execPath, [generatorPath, '--install', 'freebuff', cliConfigPath, binary, state], { encoding: 'utf8' });
  assert.match(cliResult, /"installed": true/);
  assert.deepEqual(JSON.parse(await readFile(cliConfigPath, 'utf8')).mcpServers['chrome-controlla'], entries[0]);
  for (const [client, configName, parentPath] of [
    ['opencode-v1', 'opencode-v1.json', ['mcp']],
    ['opencode-v2', 'opencode-v2.json', ['mcp', 'servers']],
  ]) {
    const clientPath = path.join(tempDir, configName);
    await writeFile(clientPath, JSON.stringify({ existing: true }));
    await install(client, clientPath, binary, state);
    const result = JSON.parse(await readFile(clientPath, 'utf8'));
    let parent = result;
    for (const key of parentPath) parent = parent[key];
    assert.ok(parent['chrome-controlla'], `${client} installs at its version-specific config path`);
    assert.equal(result.existing, true, `${client} preserves unrelated settings`);
  }
} finally {
  await rm(tempDir, { recursive: true, force: true });
}
const docs = await readFile(new URL('../../docs/clients.md', import.meta.url), 'utf8');
function documentedJson(heading) {
  const section = docs.split(`${heading}\n`)[1]?.split('\n## ')[0];
  const block = section?.match(/```json\s*([\s\S]*?)\s*```/);
  assert.ok(block, `${heading} has a JSON example`);
  return JSON.parse(block[1]);
}
function normalizePathSeparators(value) {
  if (typeof value === 'string') return value.replaceAll('\\', '/');
  if (Array.isArray(value)) return value.map(normalizePathSeparators);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, normalizePathSeparators(item)]));
  return value;
}
assert.deepEqual(documentedJson('## Freebuff / Codebuff CLI MCP config'), normalizePathSeparators(generate('freebuff')));
assert.deepEqual(documentedJson('## OpenCode v1'), normalizePathSeparators(generate('opencode-v1')));
assert.deepEqual(documentedJson('## OpenCode v2'), normalizePathSeparators(generate('opencode-v2')));
const claudeSection = docs.split('## Claude Code\n')[1]?.split('\n## ')[0] ?? '';
assert.ok(claudeSection.includes(normalizePathSeparators(generate('claude')).split('\n')[0]), 'Claude command matches generator output');
assert.match(docs, /ChatGPT does not connect directly to a local stdio process/);
console.log('Versioned client configs and safe JSON merge installation checks passed.');
