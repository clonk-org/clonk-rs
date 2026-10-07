#!/usr/bin/env python3
"""Verify restored, isolated Cargo workspace artifacts against actual source bytes.

Use prepare after cache restore and record only after a successful build. Cargo's
normal feature/profile/dependency fingerprints still apply; --recipe identifies
command-line inputs which do not live in the environment or tracked checkout.
No target directory may be shared or symlinked between checkouts.
"""

from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib


SCHEMA_VERSION = 2
LEDGER = ".ci-workspace-inputs.json"
FINGERPRINT = re.compile(r"(?P<package>.+)-(?P<metadata>[0-9a-f]{16})")
BUILD_ENVIRONMENT = (
    "CARGO", "RUST", "CC", "CXX", "CFLAGS", "CPPFLAGS", "LDFLAGS", "AR", "LD",
    "CMAKE", "PKG_CONFIG", "LIBCLANG",
)
VERIFIED_MTIME_NS = 1_000_000_000
RUST_PROVIDER_TOOLS = {
    "cargo", "cargo-clippy", "cargo-fmt", "clippy-driver", "rustc", "rustdoc",
    "rustfmt", "rust-gdb", "rust-gdbgui", "rust-lldb",
}


class CacheError(Exception):
    """An input or artifact cannot safely be verified."""


def command(root: Path, arguments: list[str]) -> str:
    completed = subprocess.run(
        arguments, cwd=root, check=False, capture_output=True, text=True, timeout=30,
        env={**os.environ, "GIT_NO_LAZY_FETCH": "1"},
    )
    if completed.returncode:
        raise CacheError(f"{' '.join(arguments)} failed: {completed.stderr.strip()}")
    return completed.stdout


def real_directory(path: Path) -> Path:
    lexical = Path(os.path.abspath(path))
    if lexical.resolve() != lexical:
        raise CacheError(f"symlinked directory is unsafe: {lexical}")
    if not lexical.is_dir():
        raise CacheError(f"not a directory: {lexical}")
    return lexical


def relative_file(root: Path, name: str) -> Path:
    relative = Path(name)
    if relative.is_absolute() or ".." in relative.parts or not relative.parts:
        raise CacheError(f"unsafe input path: {name!r}")
    path = root / relative
    if path.parent.resolve() != path.parent or not path.is_file():
        raise CacheError(f"input is missing or has a symlinked parent: {path}")
    return path


def digest(root: Path, name: str) -> str:
    path = relative_file(root, name)
    if path.is_symlink():
        # A tracked in-checkout leaf link (e.g. CLAUDE.md -> AGENTS.md) is safe
        # only when its actual regular input and link text can both be proved.
        resolved = path.resolve()
        if not resolved.is_relative_to(root) or resolved.parent.resolve() != resolved.parent:
            raise CacheError(f"input symlink leaves the checkout: {path}")
        payload = os.readlink(path).encode() + b"\0" + resolved.read_bytes()
        return "symlink:" + hashlib.sha256(payload).hexdigest()
    return hashlib.sha256(path.read_bytes()).hexdigest()


def packages(root: Path) -> dict[str, str]:
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    patterns = workspace.get("workspace", {}).get("members", [])
    exclusions = workspace.get("workspace", {}).get("exclude", [])
    manifests = {root / "Cargo.toml"} if "package" in workspace else set()
    for pattern in patterns:
        if Path(pattern).is_absolute() or ".." in Path(pattern).parts:
            raise CacheError(f"workspace member leaves the checkout: {pattern}")
        manifests.update(member / "Cargo.toml" for member in root.glob(pattern))
    excluded = {member for pattern in exclusions for member in root.glob(pattern)}
    result = {}
    for manifest in sorted(manifests):
        if manifest.parent in excluded:
            continue
        relative_file(root, manifest.relative_to(root).as_posix())
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        name = package.get("name")
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", name):
            raise CacheError(f"invalid workspace package name: {manifest}")
        if name in result:
            raise CacheError(f"duplicate workspace package: {name}")
        result[name] = manifest.parent.relative_to(root).as_posix()
    if not result:
        raise CacheError("no workspace packages found")
    return result


def build_global(name: str, mode: str) -> bool:
    """Whether Cargo reads a non-package file for the whole workspace build.

    Other non-package files (documentation, scripts, workflows, data) reach a
    compiled unit only through rustc dep-info, which compiler_inputs records
    as that package's external inputs. Content gitlinks stay global.
    """
    path = Path(name)
    return (mode == "160000" or ".cargo" in path.parts[:-1] or name == ".gitmodules"
            or path.name in ("Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml"))


def tracked_inputs(root: Path, target: Path, members: dict[str, str]) -> tuple[dict, dict]:
    global_inputs = {}
    inputs = {name: {"directory": directory, "files": {}} for name, directory in members.items()}
    for entry in command(root, ["git", "ls-files", "--stage", "-z"]).split("\0"):
        if not entry:
            continue
        metadata, name = entry.split("\t", 1)
        mode, revision, stage = metadata.split()
        if stage != "0":
            raise CacheError("unmerged index cannot identify source inputs")
        if (root / name).is_relative_to(target):
            raise CacheError(f"target contains tracked source: {name}")
        value = f"gitlink:{revision}" if mode == "160000" else digest(root, name)
        owners = [
            package for package, directory in members.items()
            if Path(name).is_relative_to(Path(directory))
        ]
        if owners:
            # Nested members belong to their deepest package.
            owner = max(owners, key=lambda package: len(Path(members[package]).parts))
            inputs[owner]["files"][name] = value
        elif build_global(name, mode):
            global_inputs[name] = value
    return global_inputs, inputs


def units(target: Path, members: dict[str, str]) -> dict[str, tuple[Path, str]]:
    found = {}
    # The usual profiles and explicit target triples also expose unsafe profile
    # links here; os.walk below never follows links into another build tree.
    directories = {*target.glob("*/.fingerprint"), *target.glob("*/*/.fingerprint")}
    for parent, children, _ in os.walk(target, followlinks=False):
        if ".fingerprint" in children:
            directories.add(Path(parent) / ".fingerprint")
            children.remove(".fingerprint")
        children[:] = [name for name in children if not (Path(parent) / name).is_symlink()]
    for directory in sorted(directories):
        real_directory(directory)
        for unit in sorted(directory.iterdir()):
            match = FINGERPRINT.fullmatch(unit.name)
            if match and match["package"] in members:
                real_directory(unit)
                if any(path.is_symlink() for path in unit.iterdir()):
                    raise CacheError(f"workspace fingerprint contains a symlink: {unit}")
                found[unit.relative_to(target).as_posix()] = (unit, match["package"])
    return found


def compiler_environment(root: Path) -> tuple[str, dict]:
    """Remove only a proven redundant Rust provider, retaining other PATH order."""
    search = os.environ.get("PATH", os.defpath)
    hashes = {}

    def selected(name: str, path: str = search) -> Path | None:
        directories = [str(root / entry) if not Path(entry).is_absolute() else entry
                       for entry in path.split(os.pathsep)]
        executable = str(root / name) if os.path.dirname(name) and not Path(name).is_absolute() else name
        found = shutil.which(executable, path=os.pathsep.join(directories))
        return Path(found) if found else None

    def identity(path: Path | None) -> dict | None:
        if path is None:
            return None
        resolved = path.resolve(strict=True)
        if resolved not in hashes:
            with resolved.open("rb") as handle:
                hashes[resolved] = hashlib.file_digest(handle, "sha256").hexdigest()
        return {"realpath": str(resolved), "sha256": hashes[resolved]}

    rustc = os.environ.get("RUSTC", "rustc")
    cargo = os.environ.get("CARGO", "cargo")
    rust_tools = {
        "rustc": {"binary": identity(selected(rustc)), "version": command(root, [rustc, "-vV"])},
        "cargo": {"binary": identity(selected(cargo)), "version": command(root, [cargo, "-vV"])},
    }
    rustup = selected("rustup")
    if os.name == "posix" and rustup:
        provider = Path(command(root, [rustc, "--print", "sysroot"]).strip()) / "bin"
        rustup_identity = identity(rustup)

        def dispatches(path: Path | None, name: str) -> bool:
            actual = identity(path)
            expected = identity(provider / name)
            if actual == expected:
                return True
            return bool(actual and actual["sha256"] == rustup_identity["sha256"]
                        and identity(Path(command(root, [str(rustup), "which", name]).strip())) == expected)

        try:
            tools = sorted(provider.iterdir())
            pure = bool(tools) and all(tool.name in RUST_PROVIDER_TOOLS and tool.is_file()
                                      and os.access(tool, os.X_OK) for tool in tools)
            if (pure and dispatches(selected(rustc), "rustc") and dispatches(selected(cargo), "cargo")
                    and all(command(root, [str(provider / name), "-vV"]) == rust_tools[name]["version"]
                            for name in ("rustc", "cargo"))):
                versions = {name: rust_tools[name]["version"] for name in ("rustc", "cargo")}
                rust_tools = {tool.name: {"binary": identity(tool),
                                         **({"version": versions[tool.name]} if tool.name in versions else {})}
                              for tool in tools}
                rust_tools["rustup"] = {"binary": rustup_identity}
                entries = search.split(os.pathsep)
                retained = [entry for entry in entries if (root / entry).resolve() != provider.resolve()]
                fallback = os.pathsep.join(retained)
                # No Rust command may appear, disappear or change providers.
                # Unknown entries and incomplete shim installations stay literal.
                if (retained and all(dispatches(selected(tool.name, fallback), tool.name) for tool in tools)
                        and all(dispatches(selected(tool.name), tool.name) for tool in tools)):
                    search = fallback
        except (CacheError, OSError, ValueError, subprocess.TimeoutExpired):
            # Unknown dispatch or provider contents never justify PATH relaxation.
            pass
    native_names = {name: name for name in ("cc", "c++", "ar", "ld", "cmake", "pkg-config", "cl", "link", "lib")}
    for key, value in os.environ.items():
        if (re.fullmatch(r"(?:CC|CXX|AR|LD)(?:_.+)?", key) or key in ("CMAKE", "PKG_CONFIG")
                or re.fullmatch(r"CARGO_TARGET_.+_LINKER", key)
                or key in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")):
            arguments = shlex.split(value, posix=os.name == "posix")
            if arguments:
                native_names[key] = arguments[0].strip('"')
    native_tools = {name: identity(selected(value)) for name, value in sorted(native_names.items())}
    return search, {"rust": rust_tools, "native": native_tools}


def recipe(root: Path, extra: str) -> dict:
    search, compilers = compiler_environment(root)
    return {
        "checkout_root": str(root),
        "rustc": command(root, [os.environ.get("RUSTC", "rustc"), "-vV"]),
        # Hash exact values: machine paths stay conservative and embedded URL
        # credentials cannot leak into a downloadable cache. Authentication
        # alone does not change compiled artifacts or their dependency identity.
        "environment": {
            key: hashlib.sha256((search if key == "PATH" else value).encode()).hexdigest()
            for key, value in sorted(os.environ.items())
            if (key == "PATH" or key.startswith(BUILD_ENVIRONMENT))
            and not (key.startswith("CARGO") and re.search(
                r"(?:^|_)(?:TOKEN|PASSWORD|SECRET|CREDENTIALS?)(?:_|$)", key,
            ))
        },
        "compilers": compilers,
        "command_recipe": extra,
    }


def dep_info_inputs(contents: str) -> set[str]:
    """Read rustc's make-style dependency paths, including escaped spaces."""
    inputs = set()
    saw_rule = False
    for line in contents.replace("\\\n", "").splitlines():
        if not line or line.startswith("#"):
            continue
        escaped = False
        separator = None
        for index, character in enumerate(line):
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == ":" and (index + 1 == len(line) or line[index + 1].isspace()):
                separator = index
                break
        if separator is None:
            raise CacheError("unsupported compiler dep-info rule")
        words = []
        word = ""
        escaped = False
        for character in line[separator + 1:]:
            if escaped:
                word += character
                escaped = False
            elif character == "\\":
                escaped = True
            elif character.isspace():
                if word:
                    words.append(word.replace("$$", "$"))
                    word = ""
            elif character == "#":
                break
            else:
                word += character
        if escaped:
            raise CacheError("incomplete compiler dep-info escape")
        if word:
            words.append(word.replace("$$", "$"))
        if words:
            saw_rule = True
            inputs.update(words)
    if not saw_rule:
        raise CacheError("compiler dep-info has no source inputs")
    return inputs


def compiler_inputs(root: Path, target: Path, inputs: dict, known_units: dict) -> dict:
    """Bind each rustc dep-info file to its package via Cargo's unit hash.

    Companion test crates compile other packages through include! and #[path].
    Their actual rustc inputs, rather than Cargo.toml dependency edges alone,
    therefore decide which workspace units can be reused. Units without this
    evidence (including build-script executions) are deliberately invalidated.
    """
    index = {}
    for name, (unit, package) in known_units.items():
        match = FINGERPRINT.fullmatch(unit.name)
        index[(unit.parent.parent, match["metadata"])] = (name, package)
    verified_units = {}
    for package in inputs.values():
        package["external_inputs"] = {}
        package["compiler_inputs_verified"] = True
    for dep_info in sorted(target.rglob("*.d")):
        suffix = re.search(r"-([0-9a-f]{16})\.d$", dep_info.name)
        if not suffix:
            continue
        owners = [
            owner for (profile, metadata), owner in index.items()
            if metadata == suffix[1] and dep_info.is_relative_to(profile)
        ]
        if not owners:
            continue
        if len(owners) != 1:
            raise CacheError("compiler dep-info has ambiguous workspace ownership")
        unit, package = owners[0]
        try:
            relative = dep_info.relative_to(target).as_posix()
            path = relative_file(target, relative)
            if path.is_symlink():
                raise CacheError("compiler dep-info is a symlink")
            sources = dep_info_inputs(path.read_text(encoding="utf-8"))
            for source in sorted(sources):
                source_path = Path(os.path.abspath(root / source))
                if source_path.is_relative_to(target):
                    # Generated target outputs are not source identity. Without
                    # a separate producer proof, recompiling this user is safer
                    # than stamping or trusting a possibly stale generated file.
                    raise CacheError("compiler input is generated under target")
                if not source_path.is_relative_to(root):
                    raise CacheError("compiler input is outside the source checkout")
                name = source_path.relative_to(root).as_posix()
                actual = digest(root, name)
                if name not in inputs[package]["files"]:
                    inputs[package]["external_inputs"][name] = actual
            verified_units.setdefault(unit, {})[relative] = digest(target, relative)
        except (CacheError, OSError, ValueError):
            inputs[package]["compiler_inputs_verified"] = False
    return verified_units


def snapshot(root: Path, target: Path, members: dict[str, str], extra: str) -> dict:
    global_inputs, inputs = tracked_inputs(root, target, members)
    known_units = units(target, members)
    return {
        "schema_version": SCHEMA_VERSION, "recipe": recipe(root, extra),
        "global_inputs": global_inputs, "packages": inputs,
        "units": compiler_inputs(root, target, inputs, known_units),
    }


def stamp(root: Path, files: dict[str, str]) -> None:
    directories = set()
    for name, expected in sorted(files.items()):
        if expected.startswith("gitlink:"):
            continue
        if digest(root, name) != expected:
            raise CacheError(f"input changed during cache verification: {name}")
        path = relative_file(root, name).resolve()
        os.utime(path, ns=(VERIFIED_MTIME_NS, VERIFIED_MTIME_NS))
        directories.update(parent for parent in path.parents if parent.is_relative_to(root))
    for directory in sorted(directories, key=lambda path: len(path.parts), reverse=True):
        real_directory(directory)
        os.utime(directory, ns=(VERIFIED_MTIME_NS, VERIFIED_MTIME_NS))


def ledger_path(root: Path, target: Path, requested: Path | None) -> Path:
    """Keep explicit ledgers in isolated checkout metadata, never source."""
    if requested is None:
        path = target / LEDGER
    else:
        if ".." in requested.parts or not requested.parts:
            raise CacheError("ledger path must not traverse outside its metadata directory")
        path = requested if requested.is_absolute() else root / requested
        path = Path(os.path.abspath(path))
    if not path.is_relative_to(root):
        if requested is not None:
            raise CacheError("ledger must be inside the checkout")
    else:
        relative = path.relative_to(root)
        if len(relative.parts) < 2 or relative.parts[0].casefold() in (".git", "content"):
            raise CacheError("ledger must occupy an isolated metadata directory")
        tracked = command(root, ["git", "ls-files", "-z"]).split("\0")
        if any(name and Path(name).parts[0].casefold() == relative.parts[0].casefold() for name in tracked):
            raise CacheError("ledger must not occupy a tracked source directory")
    if path.resolve() != path or path.is_symlink():
        raise CacheError(f"ledger path is symlinked: {path}")
    if path.exists() and not path.is_file():
        raise CacheError(f"ledger is not a regular file: {path}")
    return path


def write_ledger(target: Path, current: dict, ledger: Path | None = None) -> None:
    path = ledger if ledger is not None else target / LEDGER
    if path.resolve() != path or path.is_symlink():
        raise CacheError(f"ledger path is symlinked: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    real_directory(path.parent)
    descriptor, temporary = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(current, handle, sort_keys=True, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def discard_results(target: Path) -> list[str]:
    """Restored test/coverage reports are never evidence about this checkout."""
    removed = []
    for name in ("nextest", "coverage", "coverage-reports", "coverage-html"):
        path = target / name
        if path.is_symlink() or path.is_file():
            path.unlink()
            removed.append(name)
        elif path.is_dir():
            shutil.rmtree(path)
            removed.append(name)
    for directory, children, files in os.walk(target, followlinks=False):
        children[:] = [name for name in children if not (Path(directory) / name).is_symlink()]
        for name in files:
            lowered = name.lower()
            result = (
                lowered.endswith((".profraw", ".profdata", ".lcov"))
                or lowered in ("lcov.info", "cobertura.xml", "coverage.xml")
                or (lowered.startswith(("junit", "coverage", "cobertura")) and lowered.endswith(".xml"))
            )
            if result:
                path = Path(directory) / name
                path.unlink()
                removed.append(path.relative_to(target).as_posix())
    return sorted(removed)


def changed_names(previous: dict, current: dict) -> list[str]:
    return sorted(name for name in previous.keys() | current.keys() if previous.get(name) != current.get(name))


def changed_recipe(previous: dict, current: dict) -> list[str]:
    """Name the differing recipe fields; their values may be machine paths."""
    names = []
    for key in sorted(previous.keys() | current.keys()):
        before, after = previous.get(key), current.get(key)
        if before == after:
            continue
        if isinstance(before, dict) and isinstance(after, dict):
            names.extend(f"{key}.{name}" for name in changed_names(before, after))
        else:
            names.append(key)
    return names


def listing(names: list[str], limit: int = 10) -> str:
    shown = ", ".join(names[:limit])
    return shown + (f" (+{len(names) - limit} more)" if len(names) > limit else "")


def prepare(root: Path, target: Path, members: dict[str, str], current: dict, ledger: Path | None = None) -> dict:
    reason = "verified"
    changed = {"recipe": [], "global": [], "packages": {}}
    try:
        path = ledger if ledger is not None else target / LEDGER
        if path.parent.resolve() != path.parent or path.is_symlink():
            raise CacheError("ledger is a symlink")
        previous = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(previous, dict) or previous.get("schema_version") != SCHEMA_VERSION:
            raise CacheError("unknown ledger schema")
        if not isinstance(previous.get("packages"), dict) or not isinstance(previous.get("units"), dict):
            raise CacheError("incomplete ledger")
        if previous.get("recipe") != current["recipe"]:
            changed["recipe"] = changed_recipe(previous.get("recipe") or {}, current["recipe"])
            raise CacheError(f"build recipe changed: {listing(changed['recipe'])}")
        if previous.get("global_inputs") != current["global_inputs"]:
            changed["global"] = changed_names(previous.get("global_inputs") or {}, current["global_inputs"])
            raise CacheError(f"global build inputs changed: {listing(changed['global'])}")
    except (CacheError, OSError, ValueError, AttributeError) as error:
        previous = {"packages": {}, "units": {}}
        reason = str(error)
    reusable = {
        name for name, inputs in current["packages"].items()
        if inputs["compiler_inputs_verified"] and previous["packages"].get(name) == inputs
    }
    for name, inputs in sorted(current["packages"].items()):
        before = previous["packages"].get(name)
        if name in reusable or before is None:
            continue
        if not inputs["compiler_inputs_verified"]:
            changed["packages"][name] = ["unverified compiler inputs"]
        elif isinstance(before, dict):
            changed["packages"][name] = sorted(
                set(changed_names(before.get("files") or {}, inputs["files"]))
                | set(changed_names(before.get("external_inputs") or {}, inputs["external_inputs"]))
            ) or ["package identity"]
    invalidated = []
    reused = []
    for name, (unit, package) in units(target, members).items():
        artifacts = [path for path in unit.iterdir() if path.is_file()]
        if (package in reusable and name in current["units"]
                and previous["units"].get(name) == current["units"][name] and artifacts
                and min(path.stat().st_mtime_ns for path in artifacts) > VERIFIED_MTIME_NS):
            reused.append(name)
        else:
            shutil.rmtree(unit)
            invalidated.append(name)
    verified = dict(current["global_inputs"]) if reusable else {}
    for name in reusable:
        verified.update(current["packages"][name]["files"])
        verified.update(current["packages"][name]["external_inputs"])
    stamp(root, verified)
    return {
        "operation": "prepare", "cache_verified": reason == "verified", "reason": reason,
        "changed_inputs": changed, "invalidated_units": invalidated, "reused_units": reused,
    }


def github_output_path(root: Path, target: Path, requested: str | None) -> Path | None:
    if requested is None:
        return None
    name = requested or os.environ.get("GITHUB_OUTPUT", "")
    if not name or ".." in Path(name).parts:
        raise CacheError("GitHub output needs an explicit, safe file path or GITHUB_OUTPUT")
    path = Path(os.path.abspath(name))
    if path.resolve() != path or path.is_symlink():
        raise CacheError("GitHub output path is symlinked")
    real_directory(path.parent)
    if path.exists() and not path.is_file():
        raise CacheError("GitHub output is not a regular file")
    if path.is_relative_to(target):
        raise CacheError("GitHub output must not overwrite cached artifacts")
    if path.is_relative_to(root):
        relative = path.relative_to(root)
        tracked = command(root, ["git", "ls-files", "-z"]).split("\0")
        if (not relative.parts or relative.parts[0].casefold() in (".git", "content")
                or any(name and Path(name).parts[0].casefold() == relative.parts[0].casefold()
                       for name in tracked)):
            raise CacheError("GitHub output must not overwrite source inputs")
    return path


def append_github_output(path: Path, report: dict) -> None:
    reused, invalidated = len(report["reused_units"]), len(report["invalidated_units"])
    state = "cold" if not reused else "unknown" if invalidated else "warm"
    payload = f"cache-state={state}\nreused-units={reused}\ninvalidated-units={invalidated}\n".encode()
    # One O_APPEND write preserves unrelated step outputs without publishing
    # source paths, fingerprints, hashes or environment observations.
    real_directory(path.parent)
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT
                         | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0), 0o600)
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise CacheError("GitHub output is not a regular file")
        if os.write(descriptor, payload) != len(payload):
            raise CacheError("incomplete GitHub output append")
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("prepare", "record"))
    parser.add_argument("--target", required=True, type=Path)
    parser.add_argument("--root", default=Path.cwd(), type=Path)
    parser.add_argument("--recipe", default="")
    parser.add_argument("--ledger", type=Path,
                        help="isolated ledger path inside --root (default: --target/.ci-workspace-inputs.json)")
    parser.add_argument("--github-output", nargs="?", const="",
                        help="append prepare receipt to this file (or GITHUB_OUTPUT when omitted)")
    arguments = parser.parse_args(argv)
    try:
        root = real_directory(arguments.root)
        if Path(command(root, ["git", "rev-parse", "--show-toplevel"]).strip()) != root:
            raise CacheError("--root is not the checkout's lexical Git root")
        target = Path(os.path.abspath(arguments.target))
        if target == root or root.is_relative_to(target) or target.is_relative_to(root / ".git"):
            raise CacheError("target must be an isolated artifact directory")
        if target.resolve() != target:
            raise CacheError(f"target path is symlinked: {target}")
        if arguments.github_output is not None and arguments.operation != "prepare":
            raise CacheError("GitHub output receipts require prepare")
        output = github_output_path(root, target, arguments.github_output)
        ledger = ledger_path(root, target, arguments.ledger)
        target.mkdir(parents=True, exist_ok=True)
        real_directory(target)
        members = packages(root)
        with ExitStack() as held:
            if os.name == "posix":
                import fcntl

                for lock in [*target.glob("*/.cargo-lock"), *target.glob("*/*/.cargo-lock")]:
                    relative_file(target, lock.relative_to(target).as_posix())
                    if lock.is_symlink():
                        raise CacheError(f"Cargo build lock is a symlink: {lock}")
                    handle = held.enter_context(lock.open("rb"))
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            removed = discard_results(target) if arguments.operation == "prepare" else []
            current = snapshot(root, target, members, arguments.recipe)
            if arguments.operation == "record":
                write_ledger(target, current, ledger)
                report = {"operation": "record", "recorded_units": sorted(current["units"])}
            else:
                report = prepare(root, target, members, current, ledger)
                report["removed_result_paths"] = removed
        if output is not None:
            append_github_output(output, report)
        print(json.dumps({"schema_version": SCHEMA_VERSION, **report}, sort_keys=True))
        return 0
    except (CacheError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"workspace cache refused: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
