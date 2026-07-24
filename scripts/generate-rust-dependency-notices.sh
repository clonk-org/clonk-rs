#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
tool_version=0.9.1
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/clonk-rust-licenses.XXXXXX")
trap 'rm -rf -- "$work_dir"' EXIT

tool_root="${CLONK_CARGO_ABOUT_ROOT:-$repo_root/target/license-tools/cargo-about-$tool_version}"
tool="$tool_root/bin/cargo-about"
template="$script_dir/rust-dependency-notices.hbs"
config="$repo_root/about.toml"
notice="$repo_root/licenses/RUST_THIRD_PARTY_LICENSES.txt"

if [[ ! -x "$tool" ]]; then
    cargo install \
        --locked \
        --version "$tool_version" \
        --features cli \
        --root "$tool_root" \
        cargo-about
fi
if [[ $("$tool" --version) != "cargo-about $tool_version" ]]; then
    echo "unexpected cargo-about executable at $tool" >&2
    exit 1
fi
cargo fetch --locked --manifest-path "$repo_root/Cargo.toml"

for binary in clonk-app clonk-game; do
    "$tool" generate \
        --config "$config" \
        --manifest-path "$repo_root/crates/$binary/Cargo.toml" \
        --locked \
        --offline \
        --fail \
        --output-file "$work_dir/$binary.txt" \
        "$template"
done

read -r lock_fingerprint input_fingerprint < <(
    python3 - "$repo_root" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])

def update(value, data):
    for byte in data:
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value

lock_value = update(0xCBF29CE484222325, (root / "Cargo.lock").read_bytes())

paths = [
    root / "Cargo.lock",
    root / "Cargo.toml",
    root / "about.toml",
    root / "scripts/generate-rust-dependency-notices.sh",
    root / "scripts/rust-dependency-notices.hbs",
]
for directory in (root / "crates", root / "third_party"):
    if directory.is_dir():
        paths.extend(directory.rglob("Cargo.toml"))

input_value = 0xCBF29CE484222325
for path in sorted(set(paths), key=lambda item: item.relative_to(root).as_posix()):
    relative = path.relative_to(root).as_posix().encode()
    input_value = update(input_value, relative)
    input_value = update(input_value, b"\0")
    input_value = update(input_value, path.read_bytes())
    input_value = update(input_value, b"\0")

print(f"{lock_value:016x} {input_value:016x}")
PY
)

{
    echo "Clonk Rust — Third-Party Rust Dependency Notices"
    echo "================================================"
    echo
    echo "Generated with cargo-about $tool_version from the locked, all-target release"
    echo "dependency graphs of clonk-app and clonk-game. Development dependencies are"
    echo "excluded. Duplicate entries between the two binary graphs are intentional."
    echo
    echo "Cargo.lock FNV-1a 64: $lock_fingerprint"
    echo "Release dependency inputs FNV-1a 64: $input_fingerprint"
    echo "Project source: ISC (see ../COPYING)."
    echo "Game content: separate CC BY-NC terms (see clonk_content_license.txt)."
    echo "Third-party Rust crates: their respective licenses below."
    echo
    echo "Bundled native libraries have additional notices under third_party/."
    echo
    echo "========================================================================"
    echo "clonk-app release dependency graph"
    echo "========================================================================"
    cat "$work_dir/clonk-app.txt"
    echo
    echo "========================================================================"
    echo "clonk-game release dependency graph"
    echo "========================================================================"
    cat "$work_dir/clonk-game.txt"
} > "$notice"

echo "Wrote $notice"
