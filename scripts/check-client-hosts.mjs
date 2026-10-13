#!/usr/bin/env node
import { resolveClientHostAdapter } from './client-host-adapters.mjs';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const matrix = JSON.parse(await readFile(resolve(root, 'apps/client-qualification-matrix.json'), 'utf8'));
const results = await Promise.all(Object.keys(matrix.clients).map(id => resolveClientHostAdapter(id)));
console.log(JSON.stringify({ schema_version: 1, qualification_status: 'not_run', clients: results }, null, 2));
if (results.some(result => result.status === 'missing_dependency')) process.exitCode = 2;
