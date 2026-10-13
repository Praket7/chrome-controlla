#!/usr/bin/env node
// Compatibility entry point for the paired Phase 11 runner.
import { spawn } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const runner = resolve(here, '../bench/runners/phase11-suite.mjs');
const child = spawn(process.execPath, [runner, ...process.argv.slice(2)], {
  cwd: resolve(here, '..'),
  env: process.env,
  stdio: 'inherit',
  windowsHide: true,
});
child.on('error', error => {
  console.error(`Could not start paired Phase 11 runner: ${error.message}`);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => {
  if (signal) {
    console.error(`Paired Phase 11 runner stopped by ${signal}`);
    process.exitCode = 1;
  } else {
    process.exitCode = code ?? 1;
  }
});
