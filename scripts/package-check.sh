#!/bin/sh
set -eu

files=$(cargo package --workspace --locked --allow-dirty --list)
for required in LICENSE NOTICE; do
    if ! printf '%s\n' "$files" | grep -Fx "$required" >/dev/null; then
        printf 'required license notice missing from package: %s\n' "$required" >&2
        exit 1
    fi
done
if printf '%s\n' "$files" | grep -E '(^|/)(adapters|comptrol-core|comptrol-platform-(windows|linux|macos)|target)(/|$)' >/dev/null; then
    printf '%s\n' "unexpected non-Chrome or build artifact in package file list" >&2
    printf '%s\n' "$files" | grep -E '(^|/)(adapters|comptrol-core|comptrol-platform-(windows|linux|macos)|target)(/|$)' >&2
    exit 1
fi
printf '%s\n' "$files"
