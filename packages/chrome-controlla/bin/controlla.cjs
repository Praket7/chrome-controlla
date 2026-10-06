#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const binary = path.join(__dirname, `controlla-core${process.platform === 'win32' ? '.exe' : ''}`);
if (!fs.existsSync(binary)) {
  process.stderr.write(`Chrome Controlla binary is missing from this package: ${binary}\n`);
  process.exit(127);
}
const child = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });
if (child.error) {
  process.stderr.write(`Could not start the package-local Chrome Controlla binary: ${child.error.message}\n`);
  process.exit(127);
}
process.exit(child.status ?? 1);
