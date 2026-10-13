#!/usr/bin/env node
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runClientMcpContract } from './client-adapters.mjs';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const matrix = JSON.parse(await readFile(resolve(root, 'apps/client-qualification-matrix.json'), 'utf8'));
const results = [];
for (const [client, row] of Object.entries(matrix.clients)) results.push(await runClientMcpContract(client, row));
const output = resolve(process.env.CONTROLLA_CLIENT_MCP_CONTRACT_OUTPUT ?? resolve(root, 'apps/client-mcp-contract-results.json'));
await mkdir(dirname(output), { recursive: true });
await writeFile(output, `${JSON.stringify({ schema_version: 1, contract_only: true, qualification_status: 'not_run', adapters: results }, null, 2)}\n`, { flag: 'wx' });
console.log(`Verified ${results.length} client MCP launch contracts (not behavioral qualification): ${output}`);
