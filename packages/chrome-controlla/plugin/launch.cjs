#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');

const override = process.env.CONTROLLA_V2_BIN;
if (override && !path.isAbsolute(override)) {
  process.stderr.write('CONTROLLA_V2_BIN must be an absolute path when set.\n');
  process.exit(2);
}
const binary = override || path.join(__dirname, '..', 'bin', `controlla-v2-core${process.platform === 'win32' ? '.exe' : ''}`);
if (!fs.existsSync(binary) || !fs.statSync(binary).isFile()) {
  process.stderr.write(`Chrome Controlla v2 binary is missing from this package: ${binary}\n`);
  process.exit(127);
}

const child = spawn(binary, [], { stdio: 'inherit', env: process.env });
let stopping = false;
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    if (stopping) return;
    stopping = true;
    if (!child.killed) child.kill(signal);
  });
}
child.on('error', (error) => {
  process.stderr.write(`Could not start Chrome Controlla v2: ${error.message}\n`);
  process.exitCode = 127;
});
child.on('exit', (code, signal) => {
  if (signal) {
    process.stderr.write(`Chrome Controlla v2 exited after ${signal}.\n`);
    process.exitCode = 1;
  } else {
    process.exitCode = code ?? 1;
  }
});
