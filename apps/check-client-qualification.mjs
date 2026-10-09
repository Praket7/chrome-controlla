#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const matrix = JSON.parse(await readFile(new URL('./client-qualification-matrix.json', import.meta.url), 'utf8'));
assert.equal(matrix.schema, 'controlla-client-qualification-v3');
const required = new Set(matrix.requiredCapabilities);
for (const capability of [
  'startup', 'persistent_session', 'foreground', 'background', 'headless',
  'block_typing', 'fast_keys', 'human_keys', 'ime', 'multi_tab',
  'user_interference', 'upload_download', 'research_extract', 'verified_mutations',
]) {
  assert.ok(required.has(capability), `missing required client capability ${capability}`);
}
for (const client of ['freebuff', 'opencode', 'claude-code', 'codex', 'chatgpt-desktop', 'generic-mcp']) {
  const entry = matrix.clients[client];
  assert.ok(entry, `missing client ${client}`);
  assert.ok(['stdio-mcp', 'packaged-local-plugin'].includes(entry.contract));
  assert.equal(entry.status, 'qualified');
}
assert.equal(matrix.qualificationPolicy.generatedEvidenceMaySupportBestClaim, false);
assert.equal(matrix.qualificationPolicy.realClientSmokeRequiredForPublicCompatibilityClaim, true);
assert.equal(matrix.qualificationPolicy.unknownMutationRetry, false);
console.log('V3 client qualification matrix is fail-closed and complete.');
