#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'Usage: scripts/check.sh --suite regression|interference|extraction|security|clients'
}

if [ "${1:-}" = --help ]; then
    usage
    exit 0
fi
if [ "$#" -ne 2 ] || [ "$1" != --suite ]; then
    usage >&2
    exit 2
fi

suite=$2
case "$suite" in
    regression)
        cargo test --workspace --locked
        ;;
    interference)
        cargo test --locked -p controlla-browser input::tests
        cargo test --locked -p controlla-browser --test phase2 --test phase5_advanced
        ;;
    extraction)
        cargo test --locked -p controlla-browser observe::tests
        cargo test --locked -p controlla-browser --test phase5_advanced
        ;;
    security)
        cargo test --locked -p controlla-runtime --test phase7
        ;;
    clients)
        npm run check:clients
        ;;
    *)
        printf 'Unknown suite: %s\n' "$suite" >&2
        usage >&2
        exit 2
        ;;
esac

printf '%s\n' "LIVE ENVIRONMENT: BLOCKED/UNAVAILABLE for suite '$suite' (this entrypoint runs local tests only; no live Chrome/client acceptance was attempted)."
