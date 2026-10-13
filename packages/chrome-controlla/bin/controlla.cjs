#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

// Keep the package CLI on the current command binary. POSIX packages also ship
// a `controlla` script, while Windows packages contain `controlla-core.exe`.
const binary = path.join(__dirname, `controlla-core${process.platform === 'win32' ? '.exe' : ''}`);
const legacyV2 = path.join(__dirname, `controlla-v2-core${process.platform === 'win32' ? '.exe' : ''}`);
const target = fs.existsSync(binary) ? binary : legacyV2;
if (!fs.existsSync(target)) {
  process.stderr.write(`Chrome Controlla binary is missing from this package: ${binary}\n`);
  process.exit(127);
}
if (target === legacyV2) {
  process.stderr.write('controlla-v2-core is deprecated; use controlla\n');
}
const child = spawnSync(target, process.argv.slice(2), { stdio: 'inherit' });
if (child.error) {
  process.stderr.write(`Could not start the package-local Chrome Controlla binary: ${child.error.message}\n`);
  process.exit(127);
}
process.exit(child.status ?? 1);
