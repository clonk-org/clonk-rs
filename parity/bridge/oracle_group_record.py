#!/usr/bin/env python3
"""Bind the group differential to the current tree and the actual oracle link."""

import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys

PIN = "7d43b47b7d789b533f32d005e64596e0a07019cd"
RECORD = "group-link-record.json"
INPUTS = "group-build-inputs.json"
SYMBOLS = (
    "lc_group_open", "lc_group_free", "lc_group_entries", "lc_group_entries_free",
    "lc_group_read_file", "lc_group_buffer_free", "lc_group_exists",
    "lc_group_maker", "lc_group_root", "lc_group_string_free",
)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def git(root, *arguments):
    return subprocess.check_output(["git", "-C", str(root), *arguments])


def source_inputs(port, oracle):
    if git(oracle, "rev-parse", "HEAD").decode().strip() != PIN:
        raise ValueError("oracle is not at the pinned revision")
    header = port / "parity/bridge/lc_group_ffi.h"
    if header.read_bytes() != git(oracle, "show", f"{PIN}:rust/include/lc_group_ffi.h"):
        raise ValueError("group header differs from the pinned contract")
    paths = git(
        port, "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--",
        "Cargo.toml", "Cargo.lock", ".cargo", "crates", "xtask",
        "parity/bridge/lc_*_ffi.h", "parity/bridge/oracle-*-bridge.patch",
        "parity/bridge/oracle-weather.patch",
    ).decode().split("\0")
    sources = {path: digest(port / path) for path in sorted(set(paths)) if path}
    oracle_sources = {
        path: digest(oracle / path) for path in (
            "src/C4Group.cpp", "src/C4WinMain.cpp", "src/rust/RustGroupBridge.cpp",
            "src/rust/RustGroupBridge.h",
        )
    }
    return {"port": str(port), "oracle_pin": PIN, "sources": sources,
            "oracle_sources": oracle_sources}


def linked_resources(port, build):
    arguments = shlex.split((build / "CMakeFiles/clonk.dir/link.txt").read_text())
    archives = [Path(arg) for arg in arguments if arg.endswith("/liblc_resources.a")]
    if len(archives) != 1:
        raise ValueError("link must contain exactly one liblc_resources.a")
    archive = (build / archives[0]).resolve()
    if not archive.is_relative_to((port / "target").resolve()):
        raise ValueError(f"linked resources archive is not from the current tree: {archive}")
    if not archive.is_file():
        raise ValueError(f"missing current-tree liblc_resources archive: {archive}")
    return archive


def verify_artifacts(record):
    for path, expected in record["artifacts"].items():
        if not Path(path).is_file() or digest(path) != expected:
            raise ValueError(f"missing or changed artifact: {path}; rebuild the oracle")


def record_build(port, oracle, build):
    before = json.loads((build / INPUTS).read_text())
    if before != source_inputs(port, oracle):
        raise ValueError("build inputs changed during the oracle build; rebuild")
    cache = (build / "CMakeCache.txt").read_text()
    if "USE_RUST_GROUP_VALIDATION:BOOL=ON" not in cache.splitlines():
        raise ValueError("oracle was built without USE_RUST_GROUP_VALIDATION=ON")
    archive = linked_resources(port, build)
    binary = build / "clonk"
    # Apple's nm cannot decode some newer Rust LLVM bitcode members. The ten
    # native ABI definitions must still each be present; an incomplete symbol
    # table can never turn a missing required definition into a pass.
    symbols = subprocess.run(
        ["nm", "-g", str(archive)], capture_output=True, text=True, check=False,
    ).stdout
    exported = {line.split()[-1].removeprefix("_") for line in symbols.splitlines()
                if len(line.split()) >= 3 and line.split()[-2] == "T"}
    missing = set(SYMBOLS) - exported
    if missing:
        raise ValueError(f"resources archive is missing ABI symbols: {sorted(missing)}")
    arguments = shlex.split((build / "CMakeFiles/clonk.dir/link.txt").read_text())
    artifacts = [binary, archive, build / "CMakeCache.txt"]
    artifacts.extend(build / "CMakeFiles/clonk.dir" / name for name in (
        "link.txt", "flags.make", "cmake_pch.hxx",
    ))
    # The separate probe reuses these native objects and libraries. Bind them
    # too, so a partially rebuilt C++ tree cannot qualify a different binary.
    artifacts.extend((build / arg).resolve() for arg in arguments
                     if arg.endswith((".a", ".o")))
    record = {
        **before,
        "port_revision": git(port, "rev-parse", "HEAD").decode().strip(),
        "resources_archive": str(archive),
        "artifacts": {str(path): digest(path) for path in artifacts},
    }
    (build / RECORD).write_text(json.dumps(record, indent=2) + "\n")
    return record


def verify_build(port, oracle, build):
    record = json.loads((build / RECORD).read_text())
    current = source_inputs(port, oracle)
    if any(record.get(key) != value for key, value in current.items()):
        raise ValueError("current source differs from the recorded build; rebuild the oracle")
    if str(linked_resources(port, build)) != record["resources_archive"]:
        raise ValueError("link no longer names the recorded resources archive")
    verify_artifacts(record)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("capture", "record", "verify"))
    parser.add_argument("--oracle-root", type=Path, required=True)
    parser.add_argument("--build-dir", default="build-group")
    args = parser.parse_args()
    port = Path(__file__).resolve().parents[2]
    oracle = args.oracle_root.resolve()
    build = oracle / args.build_dir
    if args.action == "capture":
        build.mkdir(parents=True, exist_ok=True)
        # A failed rebuild must never leave an earlier attestation usable.
        (build / RECORD).unlink(missing_ok=True)
        (build / INPUTS).write_text(json.dumps(source_inputs(port, oracle), indent=2) + "\n")
    else:
        record = (record_build if args.action == "record" else verify_build)(port, oracle, build)
        archive = record["resources_archive"]
        print(f"resources archive: {archive}\nsha256: {record['artifacts'][archive]}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"group build record: {error}")
