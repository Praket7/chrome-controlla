#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const root = new URL('../../', import.meta.url);
const read = (path) => readFile(new URL(path, root), 'utf8');

const [codex, opencodeRaw, claudeRaw, freebuffRaw] = await Promise.all([
  read('integrations/codex/config.toml.example'),
  read('integrations/opencode/opencode.json.example'),
  read('integrations/claude-code/mcp.json.example'),
  read('integrations/freebuff/chrome-controlla.mcp.json'),
]);
assert.match(codex, /mcp_servers\.chrome-controlla/);
assert.match(codex, /args = \["mcp"\]/);
const opencode = JSON.parse(opencodeRaw);
assert.deepEqual(opencode.mcp['chrome-controlla'].command.slice(-1), ['mcp']);
const claude = JSON.parse(claudeRaw);
assert.deepEqual(claude.mcpServers['chrome-controlla'].args, ['mcp']);
const freebuff = JSON.parse(freebuffRaw);
assert.equal(freebuff.transport, 'stdio');
assert.equal(freebuff.session.reuseProcess, true);
assert.equal(freebuff.safety.explicitTargetAuthority, true);
assert.equal(freebuff.safety.unknownMutationRetry, false);
for (const raw of [codex, opencodeRaw, claudeRaw, freebuffRaw]) {
  assert.match(raw, /CONTROLLA_STATE_DIR/);
  assert.match(raw, /v3-compact/);
}
console.log('V3 adapters preserve one stdio runtime and compact semantics.');
