#!/bin/sh
set -eu

tree=$(cargo tree --workspace --edges normal --prefix none)
if printf '%s\n' "$tree" | grep -E '^comptrol-(core|platform-|software|settings|popup|app-registry)' >/dev/null; then
    printf '%s\n' "unexpected Comptrol desktop or monolithic dependency in browser distribution" >&2
    printf '%s\n' "$tree" | grep -E '^comptrol-(core|platform-|software|settings|popup|app-registry)' >&2
    exit 1
fi
