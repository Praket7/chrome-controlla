#!/usr/bin/env node
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const shellQuote = (value) => `'${value.replaceAll("'", `'"'"'`)}'`;

export function generateCodexCommand(binary, stateRoot) {
  if (!path.isAbsolute(binary) || !path.isAbsolute(stateRoot)) {
    throw new Error('Codex executable and state directory inputs must be absolute paths');
  }
  const state = path.join(stateRoot, 'codex');
  return `codex mcp add chrome-controlla --env ${shellQuote(`CONTROLLA_STATE_DIR=${state}`)} -- ${shellQuote(binary)} mcp\ncodex mcp get chrome-controlla --json`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const binary = process.argv[2];
  const state = process.argv[3];
  if (!binary || !state) {
    console.error('usage: node integrations/codex-config.mjs /absolute/path/to/controlla /absolute/state-root');
    process.exit(2);
  }
  try {
    console.log(generateCodexCommand(binary, state));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}
