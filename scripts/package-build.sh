#!/bin/sh
set -eu
cargo build --locked --release -p controlla-runtime
mkdir -p packages/chrome-controlla/bin
if [ -f target/release/controlla.exe ]; then
    cp target/release/controlla.exe packages/chrome-controlla/bin/controlla-core.exe
    cp target/release/controlla-v2.exe packages/chrome-controlla/bin/controlla-v2-core.exe
else
    cp target/release/controlla packages/chrome-controlla/bin/controlla-core
    cp target/release/controlla-v2 packages/chrome-controlla/bin/controlla-v2-core
    chmod 755 packages/chrome-controlla/bin/controlla packages/chrome-controlla/bin/controlla-core packages/chrome-controlla/bin/controlla-v2-core
fi
node scripts/stage-npm-package.mjs
