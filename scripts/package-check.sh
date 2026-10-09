#!/bin/sh
set -eu
for schema in capabilities.json doctor-config.json; do
    printf 'package-check: comparing schema %s\n' "$schema"
    if ! cmp -s "schemas/$schema" "crates/controlla-runtime/schemas/$schema"; then
        printf 'schema copies differ: %s\n' "$schema" >&2
        exit 1
    fi
done

printf '%s\n' 'package-check: collecting Cargo package file list'
files=$(cargo package --workspace --locked --allow-dirty --list)
for required in LICENSE NOTICE; do
    if ! printf '%s\n' "$files" | grep -Fx "$required" >/dev/null; then
        printf 'required license notice missing from package: %s\n' "$required" >&2
        exit 1
    fi
done
printf '%s\n' 'package-check: packing Cargo workspace crates'
# Runtime depends on the sibling browser crate by path; verify the workspace
# builds/tests separately rather than resolving an unpublished crate from crates.io.
cargo package --workspace --locked --allow-dirty --no-verify --offline
printf '%s\n' 'package-check: checking staged npm package and excluded paths'
if [ ! -f packages/chrome-controlla/dist/package.json ]; then
    printf '%s\n' 'package launcher component missing (run scripts/package-build.sh)' >&2
    exit 1
fi
if printf '%s\n' "$files" | grep -E '(^|/)(adapters|comptrol-core|comptrol-platform-(windows|linux|macos)|target)(/|$)' >/dev/null; then
    printf '%s\n' "unexpected non-Chrome or build artifact in package file list" >&2
    printf '%s\n' "$files" | grep -E '(^|/)(adapters|comptrol-core|comptrol-platform-(windows|linux|macos)|target)(/|$)' >&2
    exit 1
fi
node_bin=$(command -v node)
# Isolate consumer installs from the invoking user's global npm allow-scripts policy.
temp=$(mktemp -d "${TMPDIR:-/tmp}/controlla package.XXXXXX")
: > "$temp/empty.npmrc"
npm_cmd() { NPM_CONFIG_USERCONFIG="$temp/empty.npmrc" npm_config_userconfig="$temp/empty.npmrc" NPM_CONFIG_ALLOW_SCRIPTS='' npm_config_allow_scripts='' npm "$@"; }
printf '%s\n' 'package-check: creating npm archive'
trap '"$node_bin" -e '\''require("node:fs").rmSync(process.argv[1],{recursive:true,force:true})'\'' "$temp"' EXIT
package="$PWD/packages/chrome-controlla/dist"
archive_name=$(npm_cmd pack --silent --pack-destination "$temp" "$package")
archive="$temp/$archive_name"
printf 'package-check: inspecting npm archive %s\n' "$archive_name"
archive_files=$(tar -tzf "$archive" | tr -d '\r')
expect_archive_entry() {
    printf 'package-check: checking archive entry %s\n' "$1"
    if ! printf '%s\n' "$archive_files" | grep -Fx "$1" >/dev/null; then
        printf 'package-check: required archive entry missing: %s\n' "$1" >&2
        exit 1
    fi
}
expect_archive_entry 'package/bin/controlla.cjs'
expect_archive_entry 'package/bin/controlla-client.cjs'
expect_archive_entry 'package/LICENSE'
expect_archive_entry 'package/NOTICE'
if [ "$(node -p 'process.platform')" = win32 ]; then
    expect_archive_entry 'package/bin/controlla-core.exe'
else
    expect_archive_entry 'package/bin/controlla-core'
fi
printf '%s\n' 'package-check: validating packed target metadata and license'
tar -xOf "$archive" package/package.json | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const p=JSON.parse(s);if(p.os?.length!==1||p.os[0]!==process.platform||p.cpu?.length!==1||p.cpu[0]!==process.arch){console.error(`package target mismatch: ${p.os}/${p.cpu} != ${process.platform}/${process.arch}`);process.exit(1)}if(p.license!=="Apache-2.0"){console.error(`unexpected license: ${p.license}`);process.exit(1)}})'
prefix="$temp/clean prefix"
printf '%s\n' 'package-check: installing packed archive into clean prefix'
npm_cmd install --prefix "$prefix" --ignore-scripts --no-audit --no-fund "$archive"
installed="$prefix/node_modules/chrome-controlla/bin/controlla.cjs"
"$node_bin" "$installed" --help | grep -F 'Usage: controlla' >/dev/null
PATH='' "$node_bin" "$installed" --version | grep -F 'controlla 0.1.0' >/dev/null
printf '%s\n' 'package-check: checking npm command shim'
# npm exec selects and launches the platform-appropriate command shim (including
# Windows .cmd under Git Bash) as an installed consumer would.
npm_cmd exec --prefix "$prefix" -- controlla --version | grep -F 'controlla 0.1.0' >/dev/null
printf 'Verified host-bound npm archive (%s/%s), attribution, and clean-prefix package-local install.\n' "$(node -p 'process.platform')" "$(node -p 'process.arch')"
node scripts/check-package-lifecycle.mjs
