#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { generateCodexCommand } from '../integrations/codex-config.mjs';

const required = [
  'startup', 'persistent_session', 'foreground', 'background', 'headless',
  'block_typing', 'fast_keys', 'human_keys', 'ime', 'multi_tab',
  'user_interference', 'upload_download', 'research_extract', 'verified_mutations',
];
const root = new URL('../', import.meta.url);
const read = (file) => readFile(new URL(file, root), 'utf8');
const json = async (file) => JSON.parse(await read(file));

export async function loadClientQualification() {
  const [matrix, freebuff, opencode, claude, codex, chatgpt, generic] = await Promise.all([
    json('apps/client-qualification-matrix.json'),
    json('integrations/freebuff/chrome-controlla.mcp.json'),
    json('integrations/opencode/opencode.json.example'),
    json('integrations/claude-code/mcp.json.example'),
    read('integrations/codex/config.toml.example'),
    json('packages/chrome-controlla/plugin/mcp.json'),
    json('integrations/generic/mcp.json.example'),
  ]);
  const codexCommand = generateCodexCommand('/absolute/path/to/controlla', '/absolute/path/to/controlla-state');
  const codexEntry = codex.match(/\[mcp_servers\.([^\]]+)\][\s\S]*?command = "([^"]+)"[\s\S]*?args = \["([^"]+)"\]/);
  assert.ok(codexEntry, 'Codex config must contain a named MCP server, command, and arguments');
  const codexCli = codexCommand.match(/codex mcp add ([^ ]+)[\s\S]*? -- '([^']+)' ([^\n]+)/);
  assert.ok(codexCli, 'generated Codex command must install and inspect a named server');
  const adapters = {
    freebuff: { serverId: freebuff.name, command: [freebuff.command, ...freebuff.args], surface: freebuff.env.CONTROLLA_AGENT_SURFACE },
    opencode: { serverId: Object.keys(opencode.mcp)[0], command: opencode.mcp['chrome-controlla'].command, surface: opencode.mcp['chrome-controlla'].environment.CONTROLLA_AGENT_SURFACE },
    'claude-code': { serverId: Object.keys(claude.mcpServers)[0], command: [claude.mcpServers['chrome-controlla'].command, ...claude.mcpServers['chrome-controlla'].args], surface: claude.mcpServers['chrome-controlla'].env.CONTROLLA_AGENT_SURFACE },
    codex: { serverId: codexEntry[1], command: [codexEntry[2], codexEntry[3]], generatedServerId: codexCli[1], generatedCommand: [codexCli[2], codexCli[3]], surface: codex.match(/CONTROLLA_AGENT_SURFACE = "([^"]+)"/)?.[1] },
    'chatgpt-desktop': { serverId: Object.keys(chatgpt.mcpServers)[0], command: [chatgpt.mcpServers['chrome-controlla'].command, ...chatgpt.mcpServers['chrome-controlla'].args], surface: 'packaged-local-plugin' },
    'generic-mcp': { serverId: Object.keys(generic.mcpServers)[0], command: [generic.mcpServers['chrome-controlla'].command, ...generic.mcpServers['chrome-controlla'].args], surface: generic.mcpServers['chrome-controlla'].env.CONTROLLA_AGENT_SURFACE },
  };
  return { matrix, adapters };
}

export function validateClientQualification(matrix, adapters) {
  assert.equal(matrix.schema, 'controlla-client-qualification-v3');
  assert.deepEqual(matrix.requiredCapabilities, required, 'required capability evidence dimensions must remain complete and ordered');
  for (const name of ['freebuff', 'opencode', 'claude-code', 'codex', 'chatgpt-desktop', 'generic-mcp']) {
    const entry = matrix.clients[name];
    const adapter = adapters[name];
    assert.ok(entry, `missing required client ${name}`);
    assert.ok(adapter, `missing adapter config for ${name}`);
    assert.ok(['stdio-mcp', 'packaged-local-plugin'].includes(entry.contract));
    assert.equal(entry.contractStatus, 'verified');
    assert.equal(entry.liveSmokeStatus, 'unverified');
    assert.equal(adapter.serverId, entry.serverId, `${name} adapter server id does not match the qualification row`);
    assert.deepEqual(adapter.command, entry.command, `${name} adapter command does not match the qualification row`);
    assert.equal(adapter.surface, entry.surface, `${name} adapter does not select the required capability surface`);
    assert.deepEqual(entry.requiredEvidenceCategories, required, `${name} is missing required capability evidence categories`);
    if (name === 'codex') {
      assert.equal(adapter.generatedServerId, entry.serverId, 'generated Codex command server id does not match');
      assert.deepEqual(adapter.generatedCommand, entry.command, 'generated Codex command does not match');
    }
  }
  assert.equal(matrix.qualificationPolicy.generatedEvidenceMaySupportBestClaim, false);
  assert.equal(matrix.qualificationPolicy.realClientSmokeRequiredForPublicCompatibilityClaim, true);
  assert.equal(matrix.qualificationPolicy.unknownMutationRetry, false);
}

if (process.argv[1] && new URL(`file://${process.argv[1]}`).href === import.meta.url) {
  const loaded = await loadClientQualification();
  validateClientQualification(loaded.matrix, loaded.adapters);
  console.log('V3 client qualification matrix is bound to adapter configs and remains fail-closed.');
}
