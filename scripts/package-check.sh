#!/bin/sh
set -eu
for schema in capabilities.json doctor-config.json; do
    if ! cmp -s "schemas/$schema" "crates/controlla-runtime/schemas/$schema"; then
        printf 'schema copies differ: %s\n' "$schema" >&2
        exit 1
    fi
done

files=$(cargo package --workspace --locked --allow-dirty --list)
for required in LICENSE NOTICE; do
    if ! printf '%s\n' "$files" | grep -Fx "$required" >/dev/null; then
        printf 'required license notice missing from package: %s\n' "$required" >&2
        exit 1
    fi
done
cargo package -p controlla-runtime --locked --allow-dirty
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
temp=$(mktemp -d "${TMPDIR:-/tmp}/controlla package.XXXXXX")
trap '"$node_bin" -e '\''require("node:fs").rmSync(process.argv[1],{recursive:true,force:true})'\'' "$temp"' EXIT
package="$PWD/packages/chrome-controlla/dist"
archive_name=$(npm pack --silent --pack-destination "$temp" "$package")
archive="$temp/$archive_name"
archive_files=$(tar -tzf "$archive")
printf '%s\n' "$archive_files" | grep -Fx 'package/bin/controlla.cjs' >/dev/null
printf '%s\n' "$archive_files" | grep -Fx 'package/LICENSE' >/dev/null
printf '%s\n' "$archive_files" | grep -Fx 'package/NOTICE' >/dev/null
if [ -f packages/chrome-controlla/dist/bin/controlla-core ]; then
    printf '%s\n' "$archive_files" | grep -Fx 'package/bin/controlla-core' >/dev/null
else
    printf '%s\n' "$archive_files" | grep -Fx 'package/bin/controlla-core.exe' >/dev/null
fi
tar -xOf "$archive" package/package.json | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const p=JSON.parse(s);if(p.os?.length!==1||p.os[0]!==process.platform||p.cpu?.length!==1||p.cpu[0]!==process.arch)process.exit(1);if(p.license!=="Apache-2.0")process.exit(1)})'
prefix="$temp/clean prefix"
npm install --prefix "$prefix" --ignore-scripts --no-audit --no-fund "$archive"
installed="$prefix/node_modules/chrome-controlla/bin/controlla.cjs"
"$node_bin" "$installed" --help | grep -F 'Usage: controlla' >/dev/null
PATH='' "$node_bin" "$installed" --version | grep -F 'controlla 0.1.0' >/dev/null
"$prefix/node_modules/.bin/controlla" --version | grep -F 'controlla 0.1.0' >/dev/null
printf 'Verified host-bound npm archive (%s/%s), attribution, and clean-prefix package-local install.\n' "$(node -p 'process.platform')" "$(node -p 'process.arch')"
