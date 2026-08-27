#!/usr/bin/env bash
set -euo pipefail

# Keep the three shipped Windows build paths on one compiler/linker contract.
# Cargo owns the declared thin-LTO graph; linker-plugin LTO moves its backend
# work into the matching bundled LLD, whose cache is reusable.
: "${GITHUB_ENV:?GitHub Actions environment file is required}"
: "${RUNNER_TEMP:?GitHub Actions runner temp directory is required}"

rustc_info=$(rustc -vV)
grep -Fqx 'release: 1.98.0' <<<"$rustc_info"
grep -Fqx 'LLVM version: 22.1.6' <<<"$rustc_info"

cargo_target=x86_64-pc-windows-msvc
target_libdir=$(cygpath -u "$(rustc --target "$cargo_target" --print target-libdir)")
rust_lld="${target_libdir%/lib}/bin/rust-lld.exe"
test -f "$rust_lld"
"$rust_lld" -flavor link --version | grep -F 'LLD 22.1.6'

thinlto_cache=$(cygpath -u "$RUNNER_TEMP")/clonk-msvc-thinlto
thinlto_cache_native=$(cygpath -m "$thinlto_cache")
mkdir -p "$thinlto_cache"

# rust-cache hashes every installed rustup toolchain. Hosted runner images can
# carry different unrelated toolchains, so retain only the compiler that owns
# this build before the workflow computes its cache identity.
expected_toolchain=1.98.0-x86_64-pc-windows-msvc
while IFS= read -r installed; do
    if [[ "$installed" != "$expected_toolchain" ]]; then
        rustup toolchain uninstall "$installed"
    fi
done < <(rustup toolchain list --quiet)
actual_toolchains=$(rustup toolchain list --quiet)
if [[ "$actual_toolchains" != "$expected_toolchain" ]]; then
    echo "expected only $expected_toolchain, found: $actual_toolchains" >&2
    exit 1
fi

rustflags='-Ctarget-feature=+crt-static -Clinker-plugin-lto -Clinker-flavor=lld-link'
rustflags+=" -Clink-arg=/lldltocache:$thinlto_cache_native"
rustflags+=' -Clink-arg=/lldltocachepolicy:cache_size=0%:cache_size_bytes=512m'
rustflags+=' -Clink-arg=/DEBUG:NONE -Clink-arg=/OPT:REF,ICF -Clink-arg=/TIME -Clink-arg=/Brepro'

# LLVM's COFF driver reads LINK/_LINK_ before explicit rustc arguments. Empty
# both for later steps so nothing can silently override the measured contract.
unset LINK _LINK_
{
    echo "CARGO_BUILD_TARGET=$cargo_target"
    echo "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=$(cygpath -w "$rust_lld")"
    echo "RUSTFLAGS=$rustflags"
    echo "THINLTO_CACHE_DIR=$thinlto_cache"
    echo 'LINK='
    echo '_LINK_='
} >>"$GITHUB_ENV"
