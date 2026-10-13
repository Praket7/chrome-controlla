#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { startAgentUxProxy } = require('./mcp-agent-ux-launch.cjs');

const override = process.env.CONTROLLA_BIN;
if (override && !path.isAbsolute(override)) {
  process.stderr.write('CONTROLLA_BIN must be an absolute path when set.\n');
  process.exit(2);
}
// The MCP subcommand currently lives in the v2-core executable. The package's
// POSIX `controlla` launcher and Windows `controlla.cmd` route `mcp` there too;
// resolve it directly so this plugin works on Windows without trying to spawn
// a nonexistent `controlla.exe` (or shelling through a .cmd file).
const binary = override || path.join(__dirname, '..', 'bin', `controlla-v2-core${process.platform === 'win32' ? '.exe' : ''}`);
if (!fs.existsSync(binary) || !fs.statSync(binary).isFile()) {
  process.stderr.write(`Chrome Controlla binary is missing from this package: ${binary}\n`);
  process.exit(127);
}
if (process.env.CONTROLLA_V2_BIN) {
  process.stderr.write('CONTROLLA_V2_BIN is deprecated; use CONTROLLA_BIN.\n');
}

const child = spawn(binary, ['mcp'], { stdio: ['pipe', 'pipe', 'pipe'], env: process.env });
child.stderr.pipe(process.stderr);
const proxy = startAgentUxProxy(child, { stateDir: process.env.CONTROLLA_STATE_DIR });
let stopping = false;
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    if (stopping) return;
    stopping = true;
    proxy.close().finally(() => {
      if (!child.killed) child.kill(signal);
    });
  });
}
child.on('error', error => {
  process.stderr.write(`Could not start Chrome Controlla: ${error.message}\n`);
  process.exitCode = 127;
});
child.on('exit', (code, signal) => {
  if (signal && !stopping) {
    process.stderr.write(`Chrome Controlla exited after ${signal}.\n`);
    process.exitCode = 1;
  } else if (!stopping) {
    process.exitCode = code ?? 1;
  }
});
