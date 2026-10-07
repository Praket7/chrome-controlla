import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { generate } from '../../integrations/generate-config.mjs';

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
    COMPTROL_CHROME_AUTO_CONNECT: '1',
    CONTROLLA_STATE_DIR: `${state}/freebuff`,
  },
});
assert.deepEqual(entries[1], {
  type: 'local',
  command: [binary, 'mcp'],
  enabled: true,
  environment: {
    COMPTROL_CHROME_AUTO_CONNECT: '1',
    CONTROLLA_STATE_DIR: `${state}/opencode-v1`,
  },
});
assert.deepEqual(entries[2], {
  type: 'local',
  command: [binary, 'mcp'],
  environment: {
    COMPTROL_CHROME_AUTO_CONNECT: '1',
    CONTROLLA_STATE_DIR: `${state}/opencode-v2`,
  },
});
assert.equal(new Set(entries.map((entry) => entry.env?.CONTROLLA_STATE_DIR ?? entry.environment.CONTROLLA_STATE_DIR)).size, 3);
assert.match(generate('claude', binary, state), /^claude mcp add .*--transport stdio --scope user chrome-controlla -- '\/opt\/chrome-controlla\/controlla' mcp/m);
const quotedClaude = generate('claude', "/tmp/agent's $(touch nope)/controlla", "/tmp/state with spaces");
assert.ok(quotedClaude.includes("'/tmp/agent'\"'\"'s $(touch nope)/controlla'"));
assert.ok(quotedClaude.includes("'CONTROLLA_STATE_DIR=/tmp/state with spaces/claude'"));
assert.throws(() => generate('chatgpt', binary, state), /Unsupported local-stdio/);
assert.throws(() => generate('freebuff', 'controlla', state), /absolute paths/);
const docs = await readFile(new URL('../../docs/clients.md', import.meta.url), 'utf8');
function documentedJson(heading) {
  const section = docs.split(`${heading}\n`)[1]?.split('\n## ')[0];
  const block = section?.match(/```json\s*([\s\S]*?)\s*```/);
  assert.ok(block, `${heading} has a JSON example`);
  return JSON.parse(block[1]);
}
assert.deepEqual(documentedJson('## Freebuff / Codebuff CLI MCP config'), generate('freebuff'));
assert.deepEqual(documentedJson('## OpenCode v1'), generate('opencode-v1'));
assert.deepEqual(documentedJson('## OpenCode v2'), generate('opencode-v2'));
const claudeSection = docs.split('## Claude Code\n')[1]?.split('\n## ')[0] ?? '';
assert.ok(claudeSection.includes(generate('claude').split('\n')[0]), 'Claude command matches generator output');
assert.match(docs, /ChatGPT does not connect directly to a local stdio process/);
console.log('Versioned local client configurations have valid shapes and isolated state directories.');
