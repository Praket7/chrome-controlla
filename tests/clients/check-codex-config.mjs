import assert from 'node:assert/strict';
import path from 'node:path';
import { generateCodexCommand } from '../../integrations/codex-config.mjs';

const binary = '/opt/chrome-controlla/controlla';
const state = '/var/lib/chrome-controlla';
const command = generateCodexCommand(binary, state);
assert.equal(
  command,
  `codex mcp add chrome-controlla --env 'CONTROLLA_STATE_DIR=${path.join(state, 'codex')}' -- '/opt/chrome-controlla/controlla' mcp\ncodex mcp get chrome-controlla --json`,
);
assert.match(command, /^codex mcp add chrome-controlla --env /);
assert.match(command, / -- '\/opt\/chrome-controlla\/controlla' mcp$/m);
assert.match(command, /codex mcp get chrome-controlla --json$/m);
const quoted = generateCodexCommand("/tmp/agent's $(touch nope)/controlla", '/tmp/state with spaces');
assert.ok(quoted.includes("'/tmp/agent'\"'\"'s $(touch nope)/controlla'"));
assert.ok(quoted.includes(`'CONTROLLA_STATE_DIR=${path.join('/tmp/state with spaces', 'codex')}'`));
assert.throws(() => generateCodexCommand('controlla', state), /absolute paths/);
console.log('Codex CLI stdio registration is isolated, shell-quoted, and matches current codex mcp add syntax.');
