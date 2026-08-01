#!/usr/bin/env bash
set -euo pipefail

: "${THINLTO_CACHE_DIR:?ThinLTO cache directory is required}"

binary_dir=${1:-target/x86_64-pc-windows-msvc/release}
vswhere='/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe'
test -f "$vswhere"
dumpbin_native=$(
    "$vswhere" -latest -products '*' \
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
        -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' |
        tr -d '\r' |
        head -1
)
test -n "$dumpbin_native"
dumpbin=$(cygpath -u "$dumpbin_native")
test -f "$dumpbin"

failed=0
shopt -s nullglob
cache_entries=("$THINLTO_CACHE_DIR"/llvmcache-*)
if [[ ${#cache_entries[@]} -eq 0 ]]; then
    echo "the shipped build produced no reusable ThinLTO entries" >&2
    failed=1
fi
du -sh "$THINLTO_CACHE_DIR"

for binary in clonk-app clonk-game c4group; do
    path="$binary_dir/$binary.exe"
    if [[ ! -s "$path" ]]; then
        echo "$path is missing or empty" >&2
        failed=1
        continue
    fi
    native_path=$(cygpath -w "$path")
    imports=$(MSYS2_ARG_CONV_EXCL='*' "$dumpbin" /NOLOGO /DEPENDENTS "$native_path")
    echo "$imports"
    if grep -Eiq '(VCRUNTIME|MSVCP|CONCRT|UCRTBASE|API-MS-WIN-CRT|MSVCR)' <<<"$imports"; then
        echo "$path imports the dynamic C/C++ runtime; +crt-static did not take" >&2
        failed=1
    fi
    if [[ -e "${path%.exe}.pdb" ]]; then
        echo "$path emitted a PDB despite /DEBUG:NONE" >&2
        failed=1
    fi
    sha256sum "$path"
done

"$binary_dir/clonk-game.exe" --version
"$binary_dir/c4group.exe"

exit "$failed"
