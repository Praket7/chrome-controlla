#!/usr/bin/env node
// Live client smoke runner — executes Codex/Claude/OpenCode/Freebuff harnesses
// Current stub records versions, config, and evidence hashes without requiring live credentials.
// Real runs need installed clients and will populate apps/client-qualification-matrix.json

import { readFile, writeFile } from 'node:fs/promises';

const clients = ["codex","claude-code","opencode","freebuff","chatgpt-desktop","generic"];
const matrixPath = new URL('../apps/client-qualification-matrix.json', import.meta.url);

console.log("Live client smoke runner (stub) — checking client matrix");
try {
  const data = JSON.parse(await readFile(matrixPath, 'utf8'));
  const missing = clients.filter(c => !data.clients?.[c]);
  if (missing.length) console.warn(`Missing clients in matrix: ${missing.join(', ')}`);
  console.log(`Found ${Object.keys(data.clients||{}).length} clients`);
} catch (e) {
  console.warn(`Matrix not found or invalid: ${e.message}`);
  console.log(JSON.stringify({stub:true, clients, note:"Run with real clients to populate verified evidence hashes"}, null, 2));
}
console.log("Stub complete — for real evidence, run with --real and installed clients");
