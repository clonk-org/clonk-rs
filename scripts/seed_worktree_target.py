#!/usr/bin/env python3

from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
import re
import shutil
import subprocess
import sys
from collections.abc import Callable, Iterator
from pathlib import Path


# Cargo names each unit's fingerprint directory `<package>-<16 hex metadata>`.
FINGERPRINT_NAME = re.compile(r"(?P<package>.+)-[0-9a-f]{16}")


class SeedRefused(Exception):
    pass


def drop_local_fingerprints(target: Path, packages: set[str]) -> int:
    dropped = 0
    # `<profile>/.fingerprint`, or `<triple>/<profile>/.fingerprint` for an
    # explicit `--target`.
    fingerprint_dirs = [*target.glob("*/.fingerprint"), *target.glob("*/*/.fingerprint")]
    for fingerprint_dir in fingerprint_dirs:
        for unit in fingerprint_dir.iterdir():
            name = FINGERPRINT_NAME.fullmatch(unit.name)
            if name and name["package"] in packages:
                shutil.rmtree(unit)
                dropped += 1
    return dropped


@contextlib.contextmanager
def holding_build_locks(target: Path) -> Iterator[None]:
    # A build holds its profile directory's `.cargo-lock` exclusively; sharing
    # it keeps that build from writing while the seed reads. Host profiles
    # lock before target triples, the order Cargo takes them, so no build can
    # hold one lock while waiting on the other.
    locks = [*sorted(target.glob("*/.cargo-lock")), *sorted(target.glob("*/*/.cargo-lock"))]
    with contextlib.ExitStack() as held:
        for lock in locks:
            handle = held.enter_context(lock.open("rb"))
            try:
                fcntl.flock(handle, fcntl.LOCK_SH | fcntl.LOCK_NB)
            except BlockingIOError:
                print(f"waiting for the build holding {lock}", file=sys.stderr)
                fcntl.flock(handle, fcntl.LOCK_SH)
        yield


def reflink_copy(source: Path, destination: Path) -> None:
    # Only a copy-on-write clone shares the seed's extents; a plain copy would
    # duplicate the whole cache on disk, which is what seeding exists to avoid.
    subprocess.run(["cp", "-a", "--reflink=always", str(source), str(destination)], check=True)


def seed(
    source: Path,
    destination: Path,
    packages: set[str],
    copy_tree: Callable[[Path, Path], None] = reflink_copy,
) -> int:
    if source.resolve() == destination.resolve():
        raise SeedRefused(
            f"{source} is this checkout's own target directory; seed a worktree, "
            "not the main checkout"
        )
    if not source.is_dir():
        raise SeedRefused(f"{source} holds no build to seed from")
    if destination.exists():
        raise SeedRefused(f"{destination} already exists; `cargo clean` it to reseed")
    # Stage under a directory named `target` so `.gitignore` hides an
    # interrupted seed, then publish it with one rename.
    staging_root = destination.parent / f".{destination.name}-seed-{os.getpid()}"
    staging = staging_root / "target"
    try:
        staging_root.mkdir()
        with holding_build_locks(source):
            copy_tree(source, staging)
        dropped = drop_local_fingerprints(staging, packages)
        staging.rename(destination)
    except (OSError, subprocess.CalledProcessError) as error:
        raise SeedRefused(f"could not copy {source}: {error}") from error
    finally:
        shutil.rmtree(staging_root, ignore_errors=True)
    return dropped


def main_checkout() -> Path:
    listing = subprocess.run(
        ["git", "worktree", "list", "--porcelain"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout
    # The main working tree is always listed first.
    return Path(listing.splitlines()[0].removeprefix("worktree "))


def workspace() -> tuple[Path, set[str]]:
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked"],
            check=True,
            stdout=subprocess.PIPE,
            text=True,
        ).stdout
    )
    local = {package["name"] for package in metadata["packages"] if package["source"] is None}
    return Path(metadata["target_directory"]), local


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Seed this worktree's Cargo target directory from the main "
        "checkout's with a copy-on-write clone."
    )
    parser.add_argument(
        "--from",
        dest="source",
        type=Path,
        help="target directory to seed from (default: the main checkout's target/)",
    )
    arguments = parser.parse_args()

    try:
        destination, packages = workspace()
        source = arguments.source or main_checkout() / "target"
    except subprocess.CalledProcessError as error:
        print(f"not seeding: `{' '.join(error.cmd)}` failed", file=sys.stderr)
        return 1
    try:
        dropped = seed(source, destination, packages)
    except SeedRefused as refusal:
        print(f"not seeding: {refusal}", file=sys.stderr)
        return 1
    print(
        f"seeded {destination} from {source}; dropped {dropped} fingerprints of "
        "this checkout's own packages so Cargo rebuilds them"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
