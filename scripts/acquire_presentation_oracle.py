#!/usr/bin/env python3
"""Acquire and verify audited C++/Rust presentation evidence.

Every mutating operation targets one explicit fresh directory outside both
source checkouts. Engine artifacts and receipts must be emitted by the real
producers; this launcher never manufactures a missing PNG, layout trace, or
engine receipt. Acceptance is explicit and follows complete strict validation.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import secrets
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


WORKSPACE = Path(__file__).resolve().parents[1]
ORACLE_SOURCE_COMMIT = "7d43b47b7d789b533f32d005e64596e0a07019cd"
ORACLE_SOURCE_CONTENT_GITLINK = "67a54d0e662bda3aa0202134efc065d7bc420872"
FIXTURE_CONTENT_COMMIT = "ab9094f96838ae9c8cb77555560a8b887231640a"

CAPTURE_WIDTH = 1280
CAPTURE_HEIGHT = 720
CAPTURE_SCALE = 100
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
PNG_STREAM_BLOCK_SIZE = 1024 * 1024
PROVENANCE_SCHEMA = "clonk-rs/presentation-oracle/v2"
LAYOUT_SCHEMA = "clonk-rs/presentation-layout/v1"

CAPTURE_PATCH_SOURCE_PATH = "parity/oracle/presentation_capture.patch"
CAPTURE_PATCH_RETAINED_PATH = "inputs/cpp-capture.patch"
LAUNCHER_SOURCE_PATH = "scripts/acquire_presentation_oracle.py"
LAUNCHER_RETAINED_PATH = "launcher/acquire_presentation_oracle.py"
CAPTURE_PATCH_PATH = WORKSPACE / CAPTURE_PATCH_SOURCE_PATH
LAUNCHER_PATH = WORKSPACE / LAUNCHER_SOURCE_PATH
CAPTURE_MANIFEST_SOURCE_PATH = "compat/presentation_captures.json"
CAPTURE_MANIFEST_PATH = WORKSPACE / CAPTURE_MANIFEST_SOURCE_PATH
CASE_SPECS_SOURCE_PATH = "compat/presentation/case_specs.json"
CASE_SPECS_PATH = WORKSPACE / CASE_SPECS_SOURCE_PATH
CPP_CONFIG_SOURCE_PATH = "compat/presentation/cpp.config"
RUST_CONFIG_SOURCE_PATH = "compat/presentation/rust.config"
NATIVE_CONFIG_SHA256 = "8e7351443514744d638c5af2c4d534b85f2791ad1b0759d72822aa257c21e1bb"
PLAYER_SOURCE_PATH = "compat/presentation/player.c4p"
PLAYER_RETAINED_PATH = "inputs/Presentation.c4p"
PLAYER_SHA256 = "8dcaf794355d1f8d7e8dfa3efa76b8f601a8a911561d161a3da8ead2a40cd5c0"
NETWORK_REFERENCES_SOURCE_PATH = "compat/presentation/network_references.json"
NETWORK_REFERENCES_RETAINED_PATH = "inputs/network-references.json"
NETWORK_REFERENCES_SHA256 = (
    "922c7ccf941069bafd38a18e3ed71a747eadaa0c2a037b4e34422ce7312c8bf4"
)
PRESENTATION_RNG_ALGORITHM = "darwin-libc-rand-park-miller-v1"
PRESENTATION_RNG_SEED = 587
PRESENTATION_RNG_EMPTY_TRACE_SHA256 = hashlib.sha256(b"").hexdigest()
PRESENTATION_RNG_PROBE_VECTOR = (
    9_865_709,
    456_730_344,
    1_160_337_230,
    488_826_203,
    1_577_044_046,
)
RUST_SOURCE_IDENTITY_RETAINED_PATH = "inputs/rust-source-identity.json"
RUST_SOURCE_INVENTORY_RETAINED_PATH = "inputs/rust-source-inputs.json"
RUST_SOURCE_INVENTORY_SCHEMA = "clonk-rs/presentation-rust-source-inputs/v1"
RUNTIME_RESOURCE_MANIFEST_ALGORITHM = "sha256-of-sorted-sha256-path-lines-v1"
RUNTIME_RESOURCE_GROUPS = {
    "graphics": ("planet/Graphics.c4g", "inputs/Graphics.c4g"),
    "system": ("planet/System.c4g", "inputs/System.c4g"),
}
CPP_RUNTIME_CONTENT_GROUPS = {
    "tutorial": ("Tutorial.c4f", "inputs/Tutorial.c4f"),
    "objects": ("Objects.c4d", "inputs/Objects.c4d"),
    "material": ("Material.c4g", "inputs/Material.c4g"),
}
PINNED_CPP_RUNTIME_RESOURCES = {
    "graphics": {
        "tree": "153c435149fffeb0f6790d95b3c2136278393310",
        "manifest_sha256": (
            "106dcd8d6888d2ceffb8f4c3d0eb06523ec2c04cb272edbfe9fb7453be16334d"
        ),
    },
    "system": {
        "tree": "dd38a61e74a8f8bda29e66107e5dc3d7008f58c3",
        "manifest_sha256": (
            "aa10a7d5605d1aab8b09863efe2e8450c71c415c98c18f6cb88e4f47f78c606b"
        ),
    },
}
RUST_SOURCE_INVENTORY_PATHS = (
    ".cargo",
    "Cargo.lock",
    "Cargo.toml",
    "COPYING",
    "rust-toolchain.toml",
    "crates",
    "planet",
    "xtask",
    CPP_CONFIG_SOURCE_PATH,
    RUST_CONFIG_SOURCE_PATH,
    CASE_SPECS_SOURCE_PATH,
    NETWORK_REFERENCES_SOURCE_PATH,
    PLAYER_SOURCE_PATH,
    CAPTURE_PATCH_SOURCE_PATH,
    LAUNCHER_SOURCE_PATH,
)
ACQUISITION_SOURCE_PATHS = (
    CAPTURE_PATCH_SOURCE_PATH,
    LAUNCHER_SOURCE_PATH,
    CAPTURE_MANIFEST_SOURCE_PATH,
    CASE_SPECS_SOURCE_PATH,
    CPP_CONFIG_SOURCE_PATH,
    RUST_CONFIG_SOURCE_PATH,
    PLAYER_SOURCE_PATH,
    NETWORK_REFERENCES_SOURCE_PATH,
    "compat/profile.json",
    "Cargo.lock",
    "Cargo.toml",
)

CASE_IDS = (
    "startup-main",
    "startup-scenario-selection",
    "startup-network-browser",
    "startup-player-selection",
    "startup-options",
    "startup-about",
    "network-lobby",
    "loader",
    "hud",
    "ingame-menu",
    "object-menu",
    "gameplay",
    "evaluation",
)
LAYOUT_CASE_IDS = frozenset(
    (*CASE_IDS[:6], "hud", "ingame-menu", "object-menu", "gameplay", "evaluation")
)
LAYOUT_PORT_ASSET_EXEMPTIONS = {
    "startup-main": {
        "startup/main/branding/logo": "branding",
        "startup/main/branding/version": "branding",
        "startup/main/branding/fan-project": "branding",
    },
    "startup-scenario-selection": {
        "startup/scenario-selection/background": "super-resolved-startup-art",
    },
    "startup-network-browser": {
        "startup/network-browser/background": "super-resolved-startup-art",
    },
    "startup-player-selection": {
        "startup/player-selection/background": "super-resolved-startup-art",
    },
    "startup-options": {
        "startup/options/tabs/paper": "super-resolved-startup-art",
    },
    "startup-about": {
        "startup/about/branding/fan-project": "branding",
    },
    **{
        case_id: {"game/upper-board/branding/logo": "branding"}
        for case_id in ("hud", "ingame-menu", "object-menu", "gameplay", "evaluation")
    },
}
LAYOUT_PORT_ASSETS = {
    case_id: frozenset(exemptions.values())
    for case_id, exemptions in LAYOUT_PORT_ASSET_EXEMPTIONS.items()
}
PORT_ASSET_EXEMPTIONS = {
    case_id: dict(LAYOUT_PORT_ASSET_EXEMPTIONS.get(case_id, {}))
    for case_id in CASE_IDS
}

CONTEXT_FIELDS = frozenset(
    {
        "oracle_source_tree",
        "rust_source_commit",
        "rust_source_tree",
        "fixture_content_tree",
        "oracle_planet_tree",
        "rust_planet_tree",
        "oracle_system_tree",
        "rust_system_tree",
        "effective_system_scripts_sha256",
        "runtime_resources",
    }
)
EXPECTED_GEOMETRY = {
    "width": CAPTURE_WIDTH,
    "height": CAPTURE_HEIGHT,
    "scale": CAPTURE_SCALE,
}
EXPECTED_POINTER_INPUT = {
    "position": [32, 32],
    "button": "none",
    "modifiers": [],
    "help": False,
}
EXPECTED_NORMALIZATION = {
    "cpp_savepng_readback": "real-height-minus-one-minus-y",
    "pointer_input": EXPECTED_POINTER_INPUT,
    "color": "rgb-or-rgba8-srgb",
}
EXPECTED_LOCALE = {
    "language": "US",
    "charset": "Windows-1252",
    "lang": "C",
    "lc_all": "C",
    "tz": "UTC",
}
ACCEPTED_DESTINATION = WORKSPACE / "compat/presentation/oracle/v1"
FINAL_PRESENTATION_GATE_EVIDENCE = ".github/workflows/landing.yml (presentation captures)"
RUN_IDS = ("run-1", "run-2")
ENGINE_IDS = ("cpp", "rust")
ENGINE_PRODUCERS = {
    "cpp": "legacyclonk-capture-patch-v1",
    "rust": "clonk-rs-capture-driver-v1",
}
ENGINE_PROFILES = {"cpp": "oracle-native", "rust": "legacy-clonk"}
CPP_STARTUP_ARGUMENTS = {
    "startup-main": "/startup:main",
    "startup-scenario-selection": "/startup:scen",
    "startup-network-browser": "/startup:net",
    "startup-player-selection": "/startup:plrsel",
    "startup-options": "/startup:options",
    "startup-about": "/startup:about",
}
CPP_RUNTIME_SCENARIOS = {
    "network-lobby": "Tutorial.c4f/Tutorial01.c4s",
    "loader": "Tutorial.c4f/Tutorial01.c4s",
    "hud": "Tutorial.c4f/Tutorial01.c4s",
    "ingame-menu": "Tutorial.c4f/Tutorial01.c4s",
    "object-menu": "Tutorial.c4f/Tutorial03.c4s",
    "gameplay": "Tutorial.c4f/Tutorial02.c4s",
    "evaluation": "Tutorial.c4f/Tutorial01.c4s",
}
CPP_RUNTIME_SCENARIO_PATHS = (
    "Tutorial.c4f/Tutorial01.c4s",
    "Tutorial.c4f/Tutorial02.c4s",
    "Tutorial.c4f/Tutorial03.c4s",
)
BUILD_ENVIRONMENT = {
    "CARGO_INCREMENTAL": "0",
    "LANG": "C",
    "LC_ALL": "C",
    "TZ": "UTC",
}
DEFAULT_COMMAND_TIMEOUT_SECONDS = 1_800
CAPTURE_COMMAND_TIMEOUT_SECONDS = 180
BUILD_HOST_ENV_ALLOWLIST = frozenset(
    {
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "TMP",
        "TEMP",
        "RUSTUP_HOME",
        "CARGO_HOME",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
    }
)
RUNTIME_HOST_ENV_ALLOWLIST = frozenset(
    {
        "PATH",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "TMP",
        "TEMP",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
    }
)
ENGINE_RECEIPT_SCHEMA = "clonk-rs/presentation-engine-receipt/v2"
LAUNCHER_RECEIPT_SCHEMA = "clonk-rs/presentation-launcher-receipt/v2"
COMPARISON_RECEIPT_SCHEMA = "clonk-rs/presentation-comparison/v1"
COMPARISON_ATTESTATION_SCHEMA = (
    "clonk-rs/presentation-comparison-attestation/v1"
)
LAUNCHER_RECEIPT_FIELDS = frozenset(
    {
        "schema",
        "run_id",
        "nonce",
        "launcher_sha256",
        "case_specs_sha256",
        "engine_receipts",
        "launches_sha256",
    }
)
ENGINE_RECEIPT_FIELDS = frozenset(
    {
        "schema",
        "run_id",
        "launcher_nonce",
        "engine",
        "producer",
        "case_id",
        "binary_sha256",
        "source_tree",
        "content_tree",
        "profile",
        "config_sha256",
        "player_sha256",
        "network_references_sha256",
        "locale",
        "seeds",
        "trigger",
        "scenario",
        "frame",
        "runtime_resources",
        "artifacts",
    }
)
V2_INDEX_FIELDS = frozenset(
    {
        "schema",
        "contract",
        "sources",
        "patch",
        "inputs",
        "builds",
        "case_specs",
        "runs",
        "comparisons",
    }
)


class AcquisitionFailure(RuntimeError):
    """A fail-closed input, staging, or evidence validation failure."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AcquisitionFailure(message)


def _is_lower_hex(value: Any, length: int) -> bool:
    return (
        isinstance(value, str)
        and len(value) == length
        and all(character in "0123456789abcdef" for character in value)
    )


def darwin_park_miller_values(seed: int, count: int) -> tuple[int, ...]:
    _require(
        type(seed) is int and 0 < seed < 2_147_483_647,
        "Darwin Park-Miller seed is outside its canonical domain",
    )
    _require(type(count) is int and count >= 0, "Park-Miller count is invalid")
    state = seed
    values = []
    for _ in range(count):
        state = state * 16_807 % 2_147_483_647
        values.append(state)
    return tuple(values)


def _validate_presentation_rng(value: Any, label: str) -> dict[str, Any]:
    _require(
        darwin_park_miller_values(
            PRESENTATION_RNG_SEED,
            len(PRESENTATION_RNG_PROBE_VECTOR),
        )
        == PRESENTATION_RNG_PROBE_VECTOR,
        "internal Darwin Park-Miller probe vector drift",
    )
    state = _require_exact_keys(
        value,
        {"algorithm", "seed", "calls", "trace_sha256"},
        label,
    )
    _require(
        state["algorithm"] == PRESENTATION_RNG_ALGORITHM,
        f"{label} algorithm drift",
    )
    _require(state["seed"] == PRESENTATION_RNG_SEED, f"{label} seed drift")
    _require(
        type(state["calls"]) is int and state["calls"] >= 0,
        f"{label} calls must be a non-negative integer",
    )
    _require(
        _is_lower_hex(state["trace_sha256"], 64),
        f"{label} trace SHA-256 is invalid",
    )
    if state["calls"] == 0:
        _require(
            state["trace_sha256"] == PRESENTATION_RNG_EMPTY_TRACE_SHA256,
            f"{label} empty trace SHA-256 mismatch",
        )
    return state


def _canonical_json_sha256(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _require_exact_keys(value: Any, expected: Iterable[str], label: str) -> dict[str, Any]:
    _require(isinstance(value, dict), f"{label} must be an object")
    expected_keys = frozenset(expected)
    observed_keys = frozenset(value)
    _require(
        observed_keys == expected_keys,
        f"{label} schema drift: missing={sorted(expected_keys - observed_keys)} "
        f"unexpected={sorted(observed_keys - expected_keys)}",
    )
    return value


def _command_text(command: Sequence[str]) -> str:
    return " ".join(repr(argument) for argument in command)


def _run(
    command: Sequence[str],
    *,
    cwd: Path | None = None,
    input_stream: Any = None,
    text: bool = False,
    environment: Mapping[str, str] | None = None,
    timeout_seconds: int = DEFAULT_COMMAND_TIMEOUT_SECONDS,
    capture_output: bool | None = None,
) -> subprocess.CompletedProcess[Any]:
    should_capture_output = input_stream is None if capture_output is None else capture_output
    try:
        result = subprocess.run(
            list(command),
            cwd=cwd,
            stdin=input_stream,
            check=False,
            capture_output=should_capture_output,
            text=text,
            env=dict(environment) if environment is not None else None,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout if error.stdout is not None else ""
        stderr = error.stderr if error.stderr is not None else ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode("utf-8", errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        details = " ".join(
            part
            for part in (
                f"stdout: {stdout.strip()}" if stdout.strip() else "",
                f"stderr: {stderr.strip()}" if stderr.strip() else "",
            )
            if part
        )
        suffix = f": {details}" if details else ""
        raise AcquisitionFailure(
            f"command timed out after {timeout_seconds} seconds: "
            f"{_command_text(command)}{suffix}"
        ) from error
    except OSError as error:
        raise AcquisitionFailure(
            f"could not run {_command_text(command)}: {error}"
        ) from error
    if result.returncode != 0:
        stderr = result.stderr if result.stderr is not None else ""
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        raise AcquisitionFailure(
            f"command failed ({result.returncode}): {_command_text(command)}: {stderr.strip()}"
        )
    return result


def _git_bytes(repository: Path, arguments: Sequence[str]) -> bytes:
    command = ["git", "-C", str(repository), *arguments]
    result = _run(command)
    return bytes(result.stdout)


def _git_text(repository: Path, arguments: Sequence[str]) -> str:
    return _git_bytes(repository, arguments).decode("utf-8", errors="strict").strip()


def require_clean_source_revision(
    repository: Path,
    *,
    required_paths: Sequence[str] = ACQUISITION_SOURCE_PATHS,
) -> tuple[str, str]:
    """Return one committed source identity after rejecting every parent-tree drift.

    The explicit pathspec exclusion avoids asking Git to inspect a worktree's
    potentially symlinked ``content`` gitlink. Fixture content is archived and
    verified independently at its pinned commit.
    """

    _require(repository.is_dir(), f"Rust source repository does not exist: {repository}")
    commit = _git_text(repository, ["rev-parse", "--verify", "HEAD^{commit}"])
    _require(_is_lower_hex(commit, 40), "Rust source HEAD is not a SHA-1 commit")
    tracked_drift = _git_text(
        repository,
        ["diff", "--name-only", "HEAD", "--", ".", ":(exclude)content"],
    )
    _require(
        not tracked_drift,
        "tracked source drift prevents an authoritative acquisition: "
        + ", ".join(tracked_drift.splitlines()),
    )
    untracked = _git_text(
        repository,
        [
            "ls-files",
            "--others",
            "--exclude-standard",
            "--",
            ".",
            ":(exclude)content",
            ":(top,exclude).clonk-update.lock",
        ],
    )
    _require(
        not untracked,
        "untracked source files prevent an authoritative acquisition: "
        + ", ".join(untracked.splitlines()),
    )
    for relative in required_paths:
        _require(
            isinstance(relative, str)
            and relative
            and not Path(relative).is_absolute()
            and ".." not in Path(relative).parts,
            f"invalid required source path: {relative!r}",
        )
        entry = _git_text(repository, ["ls-tree", commit, "--", relative])
        fields = entry.split(maxsplit=3)
        _require(
            len(fields) == 4
            and fields[1] == "blob"
            and fields[3] == relative,
            f"required acquisition input is not a committed regular file: {relative}",
        )
        path = repository / relative
        _regular_file(path, f"required acquisition input {relative}")
    return commit, tree_oid(repository, commit)


def verify_clean_source_checkpoint(
    repository: Path,
    *,
    expected_commit: str,
    expected_tree: str,
    expected_inventory: Any,
    label: str,
    required_paths: Sequence[str] = ACQUISITION_SOURCE_PATHS,
    source_paths: Sequence[str] = RUST_SOURCE_INVENTORY_PATHS,
) -> None:
    """Fail if a build/capture changed any committed or inventoried source input."""

    try:
        commit, tree = require_clean_source_revision(
            repository,
            required_paths=required_paths,
        )
        inventory = rust_source_input_inventory(
            repository,
            commit,
            source_paths=source_paths,
        )
        _require(commit == expected_commit, f"commit changed from {expected_commit} to {commit}")
        _require(tree == expected_tree, f"tree changed from {expected_tree} to {tree}")
        validate_rust_source_input_inventory(
            inventory,
            expected=expected_inventory,
        )
    except AcquisitionFailure as error:
        raise AcquisitionFailure(f"{label} source checkpoint failed: {error}") from error


def verify_checkout_head(
    checkout: Path,
    *,
    expected_commit: str = ORACLE_SOURCE_COMMIT,
) -> str:
    """Require a supplied oracle checkout to be parked at the exact pin."""

    _require(checkout.is_dir(), f"oracle checkout does not exist: {checkout}")
    _require(_is_lower_hex(expected_commit, 40), "expected oracle commit is not a SHA-1")
    head = _git_text(checkout, ["rev-parse", "--verify", "HEAD^{commit}"])
    _require(
        head == expected_commit,
        f"oracle checkout HEAD is {head}, expected {expected_commit}",
    )
    return head


def read_gitlink(repository: Path, revision: str, relative_path: str) -> str:
    """Read one exact 160000 tree entry without consulting a submodule checkout."""

    output = _git_text(
        repository,
        ["ls-tree", revision, "--", relative_path],
    )
    fields = output.split(maxsplit=3)
    _require(
        len(fields) == 4,
        f"{revision}:{relative_path} is not one exact git tree entry",
    )
    mode, kind, object_id, observed_path = fields
    _require(
        mode == "160000" and kind == "commit" and observed_path == relative_path,
        f"{revision}:{relative_path} is not a gitlink",
    )
    _require(_is_lower_hex(object_id, 40), "gitlink object name is not a SHA-1")
    return object_id


def tree_oid(repository: Path, revision: str, relative_path: str | None = None) -> str:
    """Resolve the tree object for a commit or one directory inside it."""

    expression = f"{revision}:{relative_path}" if relative_path else f"{revision}^{{tree}}"
    object_id = _git_text(repository, ["rev-parse", "--verify", expression])
    _require(_is_lower_hex(object_id, 40), f"{expression} did not resolve to a SHA-1 tree")
    kind = _git_text(repository, ["cat-file", "-t", object_id])
    _require(kind == "tree", f"{expression} resolved to {kind}, not a tree")
    return object_id


def _require_destination_outside_repository(repository: Path, destination: Path) -> None:
    try:
        destination.resolve(strict=False).relative_to(repository.resolve(strict=True))
    except ValueError:
        return
    except OSError as error:
        raise AcquisitionFailure(f"could not resolve archive paths: {error}") from error
    raise AcquisitionFailure(f"archive destination is inside source repository: {destination}")


def materialize_git_archive(repository: Path, revision: str, destination: Path) -> None:
    """Extract an exact commit into a new destination without touching its checkout."""

    _require(_is_lower_hex(revision, 40), "archive revision must be an exact SHA-1 commit")
    _require(not destination.exists(), f"archive destination must not exist: {destination}")
    _git_text(repository, ["rev-parse", "--verify", f"{revision}^{{commit}}"])
    _require_destination_outside_repository(repository, destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.mkdir()
    archive_command = [
        "git",
        "-C",
        str(repository),
        "archive",
        "--format=tar",
        revision,
    ]
    extract_command = ["tar", "-xf", "-", "-C", str(destination)]
    try:
        archive = subprocess.Popen(
            archive_command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        shutil.rmtree(destination)
        raise AcquisitionFailure(
            f"could not run {_command_text(archive_command)}: {error}"
        ) from error
    assert archive.stdout is not None
    assert archive.stderr is not None
    try:
        with archive.stdout, archive.stderr:
            extract = subprocess.run(
                extract_command,
                stdin=archive.stdout,
                check=False,
                capture_output=True,
            )
            archive_stderr = archive.stderr.read()
        archive_returncode = archive.wait()
    except OSError as error:
        archive.kill()
        archive.wait()
        shutil.rmtree(destination)
        raise AcquisitionFailure(
            f"could not run {_command_text(extract_command)}: {error}"
        ) from error
    if archive_returncode != 0 or extract.returncode != 0:
        shutil.rmtree(destination)
        detail = (archive_stderr + extract.stderr).decode("utf-8", errors="replace").strip()
        raise AcquisitionFailure(f"could not materialize {revision}: {detail}")


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise AcquisitionFailure(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_json(path: Path) -> Any:
    _require(path.is_file() and not path.is_symlink(), f"JSON is not a regular file: {path}")
    try:
        return json.loads(
            path.read_text(encoding="utf-8"),
            object_pairs_hook=_reject_duplicate_keys,
        )
    except AcquisitionFailure:
        raise
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise AcquisitionFailure(f"could not read JSON {path}: {error}") from error


def load_json_at_revision(repository: Path, revision: str, relative_path: str) -> Any:
    """Load strict UTF-8 JSON from a committed blob, never from the worktree."""

    try:
        encoded = _git_bytes(repository, ["cat-file", "blob", f"{revision}:{relative_path}"])
        return json.loads(
            encoded.decode("utf-8", errors="strict"),
            object_pairs_hook=_reject_duplicate_keys,
        )
    except AcquisitionFailure:
        raise
    except (UnicodeError, json.JSONDecodeError) as error:
        raise AcquisitionFailure(
            f"could not read JSON {revision}:{relative_path}: {error}"
        ) from error


def capture_manifest_contract_value_sha256(manifest: Any) -> str:
    """Hash comparison terms without binding evidence lifecycle bookkeeping.

    A capture is produced before its accepted evidence can be committed. The
    manifest therefore changes from ``pending`` to ``captured`` only after the
    run. Binding the whole file would make that honest transition invalidate
    the run that supplied the evidence. This projection keeps every field that
    can weaken or redirect a comparison while excluding status, blocker,
    description, note, and evidence-path bookkeeping.
    """

    _require(isinstance(manifest, dict), "capture manifest must be an object")
    capture = _require_exact_keys(
        manifest.get("capture"),
        {"resolution", "scale", "pointer", "note"},
        "capture geometry",
    )
    pointer = _require_exact_keys(
        capture["pointer"],
        {"position", "button", "modifiers", "help", "note"},
        "capture pointer input",
    )
    _require(
        {key: pointer[key] for key in EXPECTED_POINTER_INPUT}
        == EXPECTED_POINTER_INPUT,
        "capture pointer input drift",
    )
    _require(
        isinstance(pointer["note"], str) and pointer["note"].strip(),
        "capture pointer input note must be non-empty",
    )
    tolerance = _require_exact_keys(
        manifest.get("tolerance"),
        {"cpu_max_channel_delta", "gpu_max_channel_delta", "note"},
        "capture tolerance",
    )
    screens = manifest.get("screens")
    _require(isinstance(screens, list), "capture screens must be an array")
    screen_terms = []
    for index, screen in enumerate(screens):
        _require(isinstance(screen, dict), f"capture screen {index} must be an object")
        _require(
            isinstance(screen.get("id"), str) and screen["id"],
            f"capture screen {index} id must be non-empty",
        )
        screen_terms.append(
            {
                "id": screen["id"],
                "comparison": screen.get("comparison", "pixel"),
                "port_assets": screen.get("port_assets", []),
            }
        )
    masks = manifest.get("masks", [])
    _require(isinstance(masks, list), "capture masks must be an array")
    mask_terms = []
    for index, mask in enumerate(masks):
        mask = _require_exact_keys(
            mask,
            {"screen", "region", "reason", "authority"},
            f"capture mask {index}",
        )
        mask_terms.append(dict(mask))
    projection = {
        "capture": {
            "resolution": capture["resolution"],
            "scale": capture["scale"],
            "pointer": dict(EXPECTED_POINTER_INPUT),
        },
        "tolerance": {
            "cpu_max_channel_delta": tolerance["cpu_max_channel_delta"],
            "gpu_max_channel_delta": tolerance["gpu_max_channel_delta"],
        },
        "screens": screen_terms,
        "masks": mask_terms,
        "comparison_terms": manifest.get("comparison_terms"),
        "port_assets": manifest.get("port_assets"),
    }
    return _canonical_json_sha256(projection)


def capture_manifest_contract_sha256(path: Path) -> str:
    return capture_manifest_contract_value_sha256(load_json(path))


def capture_manifest_contract_sha256_at_revision(
    repository: Path,
    revision: str,
) -> str:
    return capture_manifest_contract_value_sha256(
        load_json_at_revision(
            repository,
            revision,
            CAPTURE_MANIFEST_SOURCE_PATH,
        )
    )


def _validate_final_presentation_lifecycle(manifest: Any, profile: Any) -> None:
    _require(isinstance(manifest, dict), "final capture manifest must be an object")
    screens = manifest.get("screens")
    _require(isinstance(screens, list), "final capture screens must be an array")
    _require(
        [screen.get("id") for screen in screens if isinstance(screen, dict)]
        == list(CASE_IDS),
        "final capture screen identity or order drift",
    )
    for screen in screens:
        case_id = screen["id"]
        _require(
            screen.get("status") == "captured",
            f"final {case_id} lifecycle is not captured",
        )
        _require(
            screen.get("blocker") is None,
            f"final {case_id} still carries a blocker",
        )
        suffix = "layout.json" if case_id in LAYOUT_CASE_IDS else "png"
        expected_evidence = {
            engine: (
                "compat/presentation/oracle/v1/run-1/"
                f"{engine}/artifacts/{case_id}.{suffix}"
            )
            for engine in ENGINE_IDS
        }
        evidence = _require_exact_keys(
            screen.get("evidence"),
            ENGINE_IDS,
            f"final {case_id} evidence",
        )
        _require(
            evidence == expected_evidence,
            f"final {case_id} evidence is detached from its indexed comparison pair",
        )

    _require(isinstance(profile, dict), "final compatibility profile must be an object")
    try:
        presentation_evidence = profile["promise"]["presentation"]["evidence"]
    except (KeyError, TypeError) as error:
        raise AcquisitionFailure("final profile presentation evidence is missing") from error
    _require(
        isinstance(presentation_evidence, list),
        "final profile presentation evidence must be an array",
    )
    lifecycle = [
        entry
        for entry in presentation_evidence
        if isinstance(entry, dict)
        and entry.get("value")
        in {"clonk-org/clonk-rs#587", FINAL_PRESENTATION_GATE_EVIDENCE}
    ]
    _require(
        len(lifecycle) == 1,
        "final profile must contain exactly one presentation capture lifecycle slot",
    )
    entry = _require_exact_keys(
        lifecycle[0],
        {"kind", "value", "status", "note"},
        "final profile presentation capture lifecycle",
    )
    _require(
        {
            "kind": entry["kind"],
            "value": entry["value"],
            "status": entry["status"],
        }
        == {
            "kind": "test",
            "value": FINAL_PRESENTATION_GATE_EVIDENCE,
            "status": "held",
        },
        "final profile presentation capture lifecycle is not held by the live gate",
    )
    _require(
        isinstance(entry["note"], str) and entry["note"].strip(),
        "final profile presentation capture lifecycle note must be non-empty",
    )


def compat_profile_contract_value_sha256(profile: Any) -> str:
    """Bind the profile while normalizing its one presentation-evidence lifecycle slot."""

    _require(isinstance(profile, dict), "compatibility profile must be an object")
    normalized = copy.deepcopy(profile)
    try:
        evidence = normalized["promise"]["presentation"]["evidence"]
    except (KeyError, TypeError) as error:
        raise AcquisitionFailure("profile presentation evidence is missing") from error
    _require(isinstance(evidence, list), "profile presentation evidence must be an array")
    matched = 0
    for index, entry in enumerate(evidence):
        _require(isinstance(entry, dict), f"profile presentation evidence {index} is invalid")
        value = entry.get("value")
        if value not in {
            "clonk-org/clonk-rs#587",
            FINAL_PRESENTATION_GATE_EVIDENCE,
        }:
            continue
        matched += 1
        _require(
            frozenset(entry) == {"kind", "value", "status", "note"},
            "profile presentation lifecycle schema drift",
        )
        _require(
            isinstance(entry["note"], str) and entry["note"].strip(),
            "profile presentation lifecycle note must be non-empty",
        )
        expected_identity = (
            {"kind": "issue", "status": "pending"}
            if value == "clonk-org/clonk-rs#587"
            else {"kind": "test", "status": "held"}
        )
        _require(
            all(entry[field] == expected for field, expected in expected_identity.items()),
            "profile presentation lifecycle identity/status drift",
        )
        evidence[index] = {"presentation_evidence_lifecycle": "normalized"}
    _require(
        matched == 1,
        "profile must contain exactly one presentation evidence lifecycle slot",
    )
    return _canonical_json_sha256(normalized)


def compat_profile_contract_sha256(path: Path) -> str:
    return compat_profile_contract_value_sha256(load_json(path))


def validate_fixture_content_pin(
    repository: Path,
    profile_path: Path,
    *,
    expected_commit: str = FIXTURE_CONTENT_COMMIT,
) -> str:
    """Cross-check the contract pin against the current root gitlink."""

    profile = load_json(profile_path)
    try:
        profile_pin = profile["pinned"]["content_commit"]
    except (KeyError, TypeError) as error:
        raise AcquisitionFailure("profile content pin is missing") from error
    _require(
        profile_pin == expected_commit,
        f"profile content pin is {profile_pin!r}, expected {expected_commit}",
    )
    gitlink = read_gitlink(repository, "HEAD", "content")
    _require(
        gitlink == expected_commit,
        f"current content gitlink is {gitlink}, expected profile pin {expected_commit}",
    )
    return profile_pin


def withheld_system_script_paths(profile: Any) -> tuple[str, ...]:
    """Mirror the compatibility profile's System.c4g script withholding rule."""

    _require(isinstance(profile, dict), "compatibility profile must be an object")
    divergences = profile.get("divergences")
    _require(isinstance(divergences, list), "compatibility profile divergences are missing")
    paths: set[str] = set()
    prefix = "planet/System.c4g/"
    for divergence in divergences:
        if not isinstance(divergence, dict) or divergence.get("profile_action") != "reverted":
            continue
        citations = divergence.get("cited_in", [])
        _require(isinstance(citations, list), "divergence cited_in must be an array")
        for citation in citations:
            if isinstance(citation, str) and citation.startswith(prefix) and citation.endswith(".c"):
                paths.add(citation.removeprefix(prefix))
    _require(paths, "compatibility profile withholds no System.c4g scripts")
    return tuple(sorted(paths))


def effective_system_script_manifest(
    repository: Path,
    revision: str,
    *,
    excluded_paths: Iterable[str],
) -> dict[str, Any]:
    """Hash the exact effective System.c4g ``.c`` path/content projection."""

    excluded = frozenset(excluded_paths)
    raw_names = _git_bytes(
        repository,
        ["ls-tree", "-r", "-z", "--name-only", revision, "--", "planet/System.c4g"],
    )
    prefix = "planet/System.c4g/"
    names = []
    for encoded in raw_names.split(b"\0"):
        if not encoded:
            continue
        name = encoded.decode("utf-8", errors="strict")
        _require(name.startswith(prefix), f"unexpected System.c4g path: {name}")
        relative = name.removeprefix(prefix)
        if relative.endswith(".c") and relative not in excluded:
            names.append(relative)
    names.sort()
    entries = []
    canonical = bytearray()
    for relative in names:
        blob = _git_bytes(
            repository,
            ["cat-file", "blob", f"{revision}:planet/System.c4g/{relative}"],
        )
        digest = hashlib.sha256(blob).hexdigest()
        entries.append({"path": relative, "sha256": digest})
        canonical.extend(f"{digest}  {relative}\n".encode("utf-8"))
    _require(entries, f"{revision} contains no effective System.c4g scripts")
    return {
        "algorithm": "sha256-of-sorted-sha256-path-lines-v1",
        "sha256": hashlib.sha256(canonical).hexdigest(),
        "entries": entries,
    }


def validate_case_inventory(
    capture_ids: Iterable[str],
    layout_ids: Iterable[str],
) -> None:
    captures = tuple(capture_ids)
    layouts = frozenset(layout_ids)
    expected_captures = frozenset(CASE_IDS)
    observed_captures = frozenset(captures)
    _require(
        len(captures) == len(observed_captures),
        "capture inventory contains duplicate case IDs",
    )
    _require(
        observed_captures == expected_captures,
        "capture inventory drift: "
        f"missing={sorted(expected_captures - observed_captures)} "
        f"unexpected={sorted(observed_captures - expected_captures)}",
    )
    _require(
        layouts == LAYOUT_CASE_IDS,
        "layout inventory drift: "
        f"missing={sorted(LAYOUT_CASE_IDS - layouts)} "
        f"unexpected={sorted(layouts - LAYOUT_CASE_IDS)}",
    )


def _regular_file(path: Path, label: str) -> None:
    _require(path.is_file() and not path.is_symlink(), f"{label} is not a regular file: {path}")


def _read_png_bytes(source: Any, length: int, path: Path, label: str) -> bytes:
    data = source.read(length)
    _require(len(data) == length, f"PNG is truncated in {label}: {path}")
    return data


class _PngScanlineStream:
    def __init__(self, width: int, height: int, channels: int, path: Path) -> None:
        self.scanline_size = 1 + width * channels
        self.expected_size = height * self.scanline_size
        self.decoded_size = 0
        self.next_filter_offset = 0
        self.inflater = zlib.decompressobj()
        self.path = path

    def _consume(self, decoded: bytes) -> None:
        end = self.decoded_size + len(decoded)
        _require(
            end <= self.expected_size,
            f"PNG decompressed scanlines exceed the expected length: {self.path}",
        )
        while self.next_filter_offset < end:
            filter_byte = decoded[self.next_filter_offset - self.decoded_size]
            _require(
                filter_byte <= 4,
                f"PNG scanline has illegal filter byte {filter_byte}: {self.path}",
            )
            self.next_filter_offset += self.scanline_size
        self.decoded_size = end

    def feed(self, compressed: bytes) -> None:
        pending = compressed
        while pending:
            before = len(pending)
            output_limit = min(
                PNG_STREAM_BLOCK_SIZE,
                self.expected_size - self.decoded_size + 1,
            )
            try:
                decoded = self.inflater.decompress(pending, output_limit)
            except zlib.error as error:
                raise AcquisitionFailure(
                    f"PNG IDAT stream is not valid zlib data: {self.path}: {error}"
                ) from error
            self._consume(decoded)
            _require(
                not self.inflater.unused_data,
                f"PNG IDAT contains data after its zlib stream: {self.path}",
            )
            pending = self.inflater.unconsumed_tail
            _require(
                not pending or len(pending) < before or bool(decoded),
                f"PNG IDAT decompressor made no progress: {self.path}",
            )

    def finish(self) -> None:
        try:
            decoded = self.inflater.flush()
        except zlib.error as error:
            raise AcquisitionFailure(
                f"PNG IDAT stream could not be completed: {self.path}: {error}"
            ) from error
        self._consume(decoded)
        _require(self.inflater.eof, f"PNG IDAT zlib stream is incomplete: {self.path}")
        _require(
            not self.inflater.unused_data and not self.inflater.unconsumed_tail,
            f"PNG IDAT zlib stream has unconsumed data: {self.path}",
        )
        _require(
            self.decoded_size == self.expected_size,
            "PNG decompressed scanline length is "
            f"{self.decoded_size}, expected {self.expected_size}: {self.path}",
        )
        _require(
            self.next_filter_offset == self.expected_size,
            f"PNG does not contain every expected scanline filter byte: {self.path}",
        )


def validate_png(path: Path) -> dict[str, int]:
    """Validate a complete, decodable canonical capture PNG."""

    _regular_file(path, "PNG")
    try:
        with path.open("rb") as source:
            signature = _read_png_bytes(source, len(PNG_SIGNATURE), path, "signature")
            _require(signature == PNG_SIGNATURE, f"invalid PNG signature: {path}")
            chunk_index = 0
            seen_idat = False
            idat_closed = False
            scanlines: _PngScanlineStream | None = None
            width = height = bit_depth = color_type = 0
            while True:
                raw_length = _read_png_bytes(source, 4, path, "chunk length")
                length = struct.unpack(">I", raw_length)[0]
                _require(length <= 0x7FFFFFFF, f"PNG chunk is too large: {path}")
                kind = _read_png_bytes(source, 4, path, "chunk type")
                _require(
                    len(kind) == 4 and all(
                        ord("A") <= byte <= ord("Z") or ord("a") <= byte <= ord("z")
                        for byte in kind
                    ),
                    f"PNG chunk type is invalid: {path}",
                )
                _require(
                    not (kind[2] & 0x20),
                    f"PNG chunk type uses a lowercase reserved byte: {path}",
                )
                if chunk_index == 0:
                    _require(kind == b"IHDR", f"PNG does not begin with IHDR: {path}")
                if kind == b"IHDR":
                    _require(chunk_index == 0 and length == 13, f"PNG has invalid IHDR: {path}")
                if kind == b"IDAT":
                    _require(not idat_closed, f"PNG IDAT chunks are not consecutive: {path}")
                    _require(scanlines is not None, f"PNG IDAT appears before IHDR: {path}")
                    seen_idat = True
                elif seen_idat:
                    idat_closed = True
                if kind == b"IEND":
                    _require(length == 0, f"PNG IEND chunk is not empty: {path}")

                payload = bytearray() if kind == b"IHDR" else None
                observed_crc = zlib.crc32(kind)
                remaining = length
                while remaining:
                    block_length = min(remaining, PNG_STREAM_BLOCK_SIZE)
                    block = _read_png_bytes(source, block_length, path, kind.decode("ascii"))
                    observed_crc = zlib.crc32(block, observed_crc)
                    if payload is not None:
                        payload.extend(block)
                    if kind == b"IDAT":
                        assert scanlines is not None
                        scanlines.feed(block)
                    remaining -= len(block)
                expected_crc = struct.unpack(
                    ">I", _read_png_bytes(source, 4, path, f"{kind.decode('ascii')} CRC")
                )[0]
                _require(
                    observed_crc & 0xFFFFFFFF == expected_crc,
                    f"PNG {kind.decode('ascii')} CRC mismatch: {path}",
                )

                if kind == b"IHDR":
                    assert payload is not None
                    (
                        width,
                        height,
                        bit_depth,
                        color_type,
                        compression,
                        filtering,
                        interlace,
                    ) = struct.unpack(">IIBBBBB", payload)
                    _require(
                        (width, height) == (CAPTURE_WIDTH, CAPTURE_HEIGHT),
                        "PNG geometry is "
                        f"{width}x{height}, expected {CAPTURE_WIDTH}x{CAPTURE_HEIGHT}: {path}",
                    )
                    _require(bit_depth == 8, f"PNG must be 8-bit: {path}")
                    _require(color_type in {2, 6}, f"PNG must be RGB or RGBA: {path}")
                    _require(compression == 0, f"PNG has unsupported compression method: {path}")
                    _require(filtering == 0, f"PNG has unsupported filter method: {path}")
                    _require(interlace == 0, f"PNG must be non-interlaced: {path}")
                    scanlines = _PngScanlineStream(
                        width,
                        height,
                        3 if color_type == 2 else 4,
                        path,
                    )
                elif kind == b"IEND":
                    _require(seen_idat, f"PNG has no IDAT chunk: {path}")
                    assert scanlines is not None
                    scanlines.finish()
                    _require(not source.read(1), f"PNG has trailing bytes after IEND: {path}")
                    break
                chunk_index += 1
    except OSError as error:
        raise AcquisitionFailure(f"could not read PNG {path}: {error}") from error
    return {
        "width": width,
        "height": height,
        "bit_depth": bit_depth,
        "color_type": color_type,
    }


def _validate_layout_rect(rect: Any, label: str) -> None:
    _require(isinstance(rect, dict), f"{label} must be an object")
    _require(
        frozenset(rect) == {"x", "y", "width", "height"},
        f"{label} schema drift",
    )
    for coordinate in ("x", "y"):
        value = rect[coordinate]
        _require(
            type(value) is int and -(2**31) <= value < 2**31,
            f"{label}.{coordinate} must be an i32",
        )
    for extent in ("width", "height"):
        value = rect[extent]
        _require(
            type(value) is int and 0 <= value < 2**32,
            f"{label}.{extent} must be a u32",
        )


def validate_layout_trace(
    path: Path,
    case_id: str,
    *,
    port_asset_exemptions: Mapping[str, str] | None = None,
) -> None:
    """Validate the strict ordered layout contract consumed by the Rust comparator."""

    _require(case_id in LAYOUT_CASE_IDS, f"{case_id} is not a layout capture case")
    exemptions = dict(
        LAYOUT_PORT_ASSET_EXEMPTIONS[case_id]
        if port_asset_exemptions is None
        else port_asset_exemptions
    )
    trace = load_json(path)
    _require(isinstance(trace, dict), f"layout trace must be an object: {path}")
    expected_keys = frozenset({"schema", "screen", "resolution", "scale", "elements"})
    _require(
        frozenset(trace) == expected_keys,
        f"layout trace schema drift for {case_id}: {path}",
    )
    _require(trace["schema"] == LAYOUT_SCHEMA, f"wrong layout schema for {case_id}")
    _require(trace["screen"] == case_id, f"wrong layout screen for {case_id}")
    _require(
        trace["resolution"] == f"{CAPTURE_WIDTH}x{CAPTURE_HEIGHT}",
        f"wrong layout resolution for {case_id}",
    )
    _require(
        type(trace["scale"]) is int and trace["scale"] == CAPTURE_SCALE,
        f"wrong layout scale for {case_id}",
    )
    _require(isinstance(trace["elements"], list), f"layout elements must be an array for {case_id}")
    _require(trace["elements"], f"layout elements must not be empty for {case_id}")
    required_element_keys = frozenset({"path", "role", "rect", "visible", "caption", "lines"})
    allowed_element_keys = required_element_keys | {"port_asset"}
    seen_paths: set[str] = set()
    observed_exemptions: dict[str, str] = {}
    for index, element in enumerate(trace["elements"]):
        label = f"layout element {index} for {case_id}"
        _require(isinstance(element, dict), f"{label} must be an object")
        keys = frozenset(element)
        _require(
            required_element_keys <= keys and keys <= allowed_element_keys,
            f"{label} schema drift",
        )
        _require(isinstance(element["path"], str), f"{label}.path must be a string")
        _require(element["path"].strip(), f"{label}.path must be non-empty")
        _require(
            element["path"] not in seen_paths,
            f"{label} has duplicate element path {element['path']!r}",
        )
        seen_paths.add(element["path"])
        _require(isinstance(element["role"], str), f"{label}.role must be a string")
        _require(element["role"].strip(), f"{label}.role must be non-empty")
        _validate_layout_rect(element["rect"], f"{label}.rect")
        _require(type(element["visible"]) is bool, f"{label}.visible must be a boolean")
        if "port_asset" in element:
            port_asset = element["port_asset"]
            _require(isinstance(port_asset, str), f"{label}.port_asset must be a string")
            _require(
                exemptions.get(element["path"]) == port_asset,
                f"{label}.port_asset does not match an exact port asset exemption path for {case_id}",
            )
            observed_exemptions[element["path"]] = port_asset
        _require(isinstance(element["caption"], str), f"{label}.caption must be a string")
        lines = element["lines"]
        _require(isinstance(lines, list), f"{label}.lines must be an array")
        for line_index, line in enumerate(lines):
            line_label = f"{label}.lines[{line_index}]"
            _require(isinstance(line, dict), f"{line_label} must be an object")
            _require(
                frozenset(line) == {"text", "rect"},
                f"{line_label} schema drift",
            )
            _require(isinstance(line["text"], str), f"{line_label}.text must be a string")
            _validate_layout_rect(line["rect"], f"{line_label}.rect")
    _require(
        observed_exemptions == exemptions,
        f"layout port asset exemption inventory drift for {case_id}",
    )


def artifact_filenames() -> tuple[str, ...]:
    pngs = tuple(f"{case_id}.png" for case_id in CASE_IDS)
    layouts = tuple(
        f"{case_id}.layout.json" for case_id in CASE_IDS if case_id in LAYOUT_CASE_IDS
    )
    return (*pngs, *layouts)


def validate_duplicate_runs(first: Path, second: Path) -> list[Path]:
    """Require two complete, independently emitted artifact sets to match."""

    _require(first.is_dir() and not first.is_symlink(), f"first run is not a directory: {first}")
    _require(second.is_dir() and not second.is_symlink(), f"repeat run is not a directory: {second}")
    accepted = []
    for name in artifact_filenames():
        left = first / name
        right = second / name
        _regular_file(left, name)
        _regular_file(right, name)
        if name.endswith(".png"):
            validate_png(left)
            validate_png(right)
        else:
            case_id = name.removesuffix(".layout.json")
            validate_layout_trace(left, case_id)
            validate_layout_trace(right, case_id)
        try:
            identical = left.read_bytes() == right.read_bytes()
        except OSError as error:
            raise AcquisitionFailure(f"could not compare duplicate artifact {name}: {error}") from error
        _require(identical, f"duplicate capture runs differ for {name}")
        accepted.append(left)
    return accepted


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for block in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as error:
        raise AcquisitionFailure(f"could not hash {path}: {error}") from error
    return digest.hexdigest()


def _file_record(root: Path, relative_path: str) -> dict[str, Any]:
    _require(
        not Path(relative_path).is_absolute() and ".." not in Path(relative_path).parts,
        f"unsafe file-record path: {relative_path}",
    )
    path = root / relative_path
    _regular_file(path, relative_path)
    try:
        size = path.stat().st_size
    except OSError as error:
        raise AcquisitionFailure(f"could not stat {path}: {error}") from error
    return {
        "path": relative_path,
        "sha256": _sha256_file(path),
        "size_bytes": size,
    }


def _git_tree_object_sha1(entries: Sequence[tuple[bytes, bytes, bytes]]) -> str:
    payload = b"".join(
        mode + b" " + name + b"\0" + object_id
        for mode, name, object_id in entries
    )
    framed = b"tree " + str(len(payload)).encode("ascii") + b"\0" + payload
    return hashlib.sha1(framed).hexdigest()


def runtime_resource_identity(directory: Path) -> dict[str, str]:
    """Recompute a strict Git-tree OID and flat byte manifest from one live group."""

    _require(
        directory.is_dir() and not directory.is_symlink(),
        f"runtime resource is not a regular directory: {directory}",
    )
    manifest_entries: list[tuple[str, str]] = []

    def visit(current: Path, relative: Path) -> str:
        tree_entries: list[tuple[bytes, bytes, bytes, bytes]] = []
        try:
            children = list(current.iterdir())
        except OSError as error:
            raise AcquisitionFailure(
                f"could not enumerate runtime resource directory {current}: {error}"
            ) from error
        for child in children:
            try:
                child_stat = child.lstat()
            except OSError as error:
                raise AcquisitionFailure(
                    f"could not inspect runtime resource entry {child}: {error}"
                ) from error
            try:
                encoded_name = child.name.encode("utf-8", errors="strict")
            except UnicodeError as error:
                raise AcquisitionFailure(
                    f"runtime resource name is not UTF-8: {child}"
                ) from error
            _require(
                encoded_name and b"/" not in encoded_name and b"\0" not in encoded_name,
                f"runtime resource name is invalid: {child}",
            )
            child_relative = relative / child.name
            if stat.S_ISDIR(child_stat.st_mode):
                object_id = bytes.fromhex(visit(child, child_relative))
                mode = b"40000"
                sort_name = encoded_name + b"/"
            elif stat.S_ISREG(child_stat.st_mode):
                digest = _sha256_file(child)
                relative_name = child_relative.as_posix()
                manifest_entries.append((relative_name, digest))
                try:
                    contents = child.read_bytes()
                except OSError as error:
                    raise AcquisitionFailure(
                        f"could not read runtime resource file {child}: {error}"
                    ) from error
                framed = b"blob " + str(len(contents)).encode("ascii") + b"\0" + contents
                object_id = hashlib.sha1(framed).digest()
                mode = b"100755" if child_stat.st_mode & 0o111 else b"100644"
                sort_name = encoded_name
            else:
                raise AcquisitionFailure(
                    f"runtime resource contains a symlink or special entry: {child}"
                )
            tree_entries.append((sort_name, mode, encoded_name, object_id))
        tree_entries.sort(key=lambda entry: entry[0])
        return _git_tree_object_sha1(
            [(mode, name, object_id) for _, mode, name, object_id in tree_entries]
        )

    root_tree = visit(directory, Path())
    _require(manifest_entries, f"runtime resource directory is empty: {directory}")
    manifest_entries.sort(key=lambda entry: entry[0])
    canonical = bytearray()
    for relative_name, digest in manifest_entries:
        canonical.extend(f"{digest}  {relative_name}\n".encode("utf-8"))
    return {
        "tree": root_tree,
        "manifest_sha256": hashlib.sha256(canonical).hexdigest(),
    }


def runtime_resource_identity_at_revision(
    repository: Path,
    revision: str,
    relative_path: str,
) -> dict[str, str]:
    """Compute the same identity from exact committed Git blobs."""

    prefix = f"{relative_path}/"
    raw = _git_bytes(
        repository,
        [
            "ls-tree",
            "-r",
            "-z",
            "--long",
            revision,
            "--",
            relative_path,
        ],
    )
    entries: list[tuple[str, str]] = []
    for encoded in raw.split(b"\0"):
        if not encoded:
            continue
        try:
            metadata, encoded_path = encoded.split(b"\t", 1)
            mode, kind, object_id, encoded_size = metadata.decode("ascii").split()
            observed_path = encoded_path.decode("utf-8", errors="strict")
            size = int(encoded_size)
        except (ValueError, UnicodeError) as error:
            raise AcquisitionFailure(
                f"could not parse runtime resource tree {revision}:{relative_path}"
            ) from error
        _require(
            mode in {"100644", "100755"} and kind == "blob",
            f"runtime resource contains a symlink or special entry: {observed_path}",
        )
        _require(
            observed_path.startswith(prefix) and len(observed_path) > len(prefix),
            f"runtime resource path escaped {relative_path}: {observed_path}",
        )
        blob = _git_bytes(repository, ["cat-file", "blob", object_id])
        _require(len(blob) == size, f"runtime resource blob size drift: {observed_path}")
        entries.append(
            (observed_path.removeprefix(prefix), hashlib.sha256(blob).hexdigest())
        )
    _require(entries, f"runtime resource tree is empty: {revision}:{relative_path}")
    entries.sort(key=lambda entry: entry[0])
    canonical = bytearray()
    for relative_name, digest in entries:
        canonical.extend(f"{digest}  {relative_name}\n".encode("utf-8"))
    return {
        "tree": tree_oid(repository, revision, relative_path),
        "manifest_sha256": hashlib.sha256(canonical).hexdigest(),
    }


def materialize_runtime_resource_tree(
    repository: Path,
    revision: str,
    relative_path: str | None,
    destination: Path,
) -> None:
    """Write exact Git blob bytes without archive/checkout attribute filters."""

    _require(not destination.exists(), f"runtime resource destination exists: {destination}")
    _require_destination_outside_repository(repository, destination)
    prefix = f"{relative_path}/" if relative_path else ""
    arguments = ["ls-tree", "-r", "-z", "--long", revision]
    if relative_path:
        arguments.extend(["--", relative_path])
    raw = _git_bytes(
        repository,
        arguments,
    )
    records: list[tuple[str, str, str, int]] = []
    for encoded in raw.split(b"\0"):
        if not encoded:
            continue
        try:
            metadata, encoded_path = encoded.split(b"\t", 1)
            mode, kind, object_id, encoded_size = metadata.decode("ascii").split()
            observed_path = encoded_path.decode("utf-8", errors="strict")
            size = int(encoded_size)
        except (ValueError, UnicodeError) as error:
            raise AcquisitionFailure(
                f"could not parse runtime resource tree {revision}:{relative_path or '/'}"
            ) from error
        _require(
            mode in {"100644", "100755"} and kind == "blob",
            f"runtime resource contains a symlink or special entry: {observed_path}",
        )
        _require(
            not prefix or observed_path.startswith(prefix),
            f"runtime resource path escaped {relative_path or '/'}: {observed_path}",
        )
        relative_name = observed_path.removeprefix(prefix)
        relative = Path(relative_name)
        _require(
            relative_name
            and not relative.is_absolute()
            and ".." not in relative.parts
            and relative.as_posix() == relative_name,
            f"runtime resource path is unsafe: {observed_path}",
        )
        records.append((relative_name, mode, object_id, size))
    _require(
        records,
        f"runtime resource tree is empty: {revision}:{relative_path or '/'}",
    )
    destination.mkdir(parents=True)
    command = ["git", "-C", str(repository), "cat-file", "--batch"]
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        shutil.rmtree(destination)
        raise AcquisitionFailure(f"could not run {_command_text(command)}: {error}") from error
    assert process.stdin is not None
    assert process.stdout is not None
    assert process.stderr is not None
    try:
        for relative_name, mode, object_id, size in records:
            process.stdin.write(object_id.encode("ascii") + b"\n")
            process.stdin.flush()
            header = process.stdout.readline()
            expected_header = f"{object_id} blob {size}\n".encode("ascii")
            _require(
                header == expected_header,
                f"git returned an unexpected runtime resource blob header: {header!r}",
            )
            target = destination / relative_name
            target.parent.mkdir(parents=True, exist_ok=True)
            with target.open("xb") as output:
                remaining = size
                while remaining:
                    block = process.stdout.read(min(remaining, 1024 * 1024))
                    _require(
                        block,
                        f"git truncated runtime resource blob {relative_name}",
                    )
                    output.write(block)
                    remaining -= len(block)
            _require(
                process.stdout.read(1) == b"\n",
                f"git omitted the batch separator for {relative_name}",
            )
            target.chmod(0o755 if mode == "100755" else 0o644)
        process.stdin.close()
        stderr = process.stderr.read()
        returncode = process.wait()
        _require(
            returncode == 0,
            f"git cat-file failed while materializing runtime resources: "
            f"{stderr.decode('utf-8', errors='replace').strip()}",
        )
    except Exception:
        if process.poll() is None:
            process.kill()
        process.wait()
        shutil.rmtree(destination)
        raise
    finally:
        process.stdout.close()
        process.stderr.close()


def expected_runtime_resources(
    oracle_repository: Path,
    workspace: Path,
    rust_revision: str,
) -> dict[str, Any]:
    resources = {
        "cpp": {
            group: runtime_resource_identity_at_revision(
                oracle_repository,
                ORACLE_SOURCE_COMMIT,
                source_path,
            )
            for group, (source_path, _) in RUNTIME_RESOURCE_GROUPS.items()
        },
        "rust": {
            group: runtime_resource_identity_at_revision(
                workspace,
                rust_revision,
                source_path,
            )
            for group, (source_path, _) in RUNTIME_RESOURCE_GROUPS.items()
        },
    }
    _require(
        resources["cpp"] == PINNED_CPP_RUNTIME_RESOURCES,
        "pinned C++ runtime resource identities drifted",
    )
    return resources


def expected_cpp_runtime_content_resources(content_repository: Path) -> dict[str, Any]:
    return {
        group: runtime_resource_identity_at_revision(
            content_repository,
            FIXTURE_CONTENT_COMMIT,
            source_path,
        )
        for group, (source_path, _) in CPP_RUNTIME_CONTENT_GROUPS.items()
    }


def expected_cpp_runtime_scenario_resources(
    content_repository: Path,
) -> dict[str, Any]:
    return {
        scenario_path: runtime_resource_identity_at_revision(
            content_repository,
            FIXTURE_CONTENT_COMMIT,
            scenario_path,
        )
        for scenario_path in CPP_RUNTIME_SCENARIO_PATHS
    }


def _validate_staged_cpp_runtime_content(
    candidate_root: Path,
    expected: Any,
) -> dict[str, Any]:
    resources = _require_exact_keys(
        expected,
        CPP_RUNTIME_CONTENT_GROUPS,
        "C++ runtime content resources",
    )
    validated = {}
    for group, (_, retained_path) in CPP_RUNTIME_CONTENT_GROUPS.items():
        identity = _validate_runtime_resource_identity(
            resources[group],
            f"C++ runtime content {group}",
        )
        observed = runtime_resource_identity(candidate_root / retained_path)
        _require(
            observed == identity,
            f"{Path(retained_path).name} runtime content digest/tree mismatch",
        )
        validated[group] = identity
    return validated


def _validate_staged_cpp_runtime_scenario(
    candidate_root: Path,
    case_id: str,
    expected: Any,
) -> dict[str, str]:
    scenario_path = CPP_RUNTIME_SCENARIOS.get(case_id)
    _require(
        scenario_path is not None,
        f"C++ runtime scenario for {case_id} is not implemented or audited",
    )
    resources = _require_exact_keys(
        expected,
        CPP_RUNTIME_SCENARIO_PATHS,
        "C++ runtime scenario resources",
    )
    identity = _validate_runtime_resource_identity(
        resources[scenario_path],
        f"C++ {case_id} scenario resource",
    )
    observed = runtime_resource_identity(
        candidate_root / "work/fixture-content" / scenario_path
    )
    _require(
        observed == identity,
        f"C++ {case_id} scenario digest/tree mismatch",
    )
    return identity


def _validate_runtime_resource_identity(value: Any, label: str) -> dict[str, str]:
    identity = _require_exact_keys(value, {"tree", "manifest_sha256"}, label)
    _require(_is_lower_hex(identity["tree"], 40), f"{label} tree is invalid")
    _require(
        _is_lower_hex(identity["manifest_sha256"], 64),
        f"{label} manifest digest is invalid",
    )
    return identity


def _validate_engine_runtime_resources(value: Any, label: str) -> dict[str, Any]:
    resources = _require_exact_keys(value, RUNTIME_RESOURCE_GROUPS, label)
    return {
        group: _validate_runtime_resource_identity(
            resources[group], f"{label} {group}"
        )
        for group in RUNTIME_RESOURCE_GROUPS
    }


def _validate_staged_cpp_runtime_resources(
    candidate_root: Path,
    expected: Any,
) -> dict[str, Any]:
    resources = _validate_engine_runtime_resources(expected, "C++ runtime resources")
    for group, (_, retained_path) in RUNTIME_RESOURCE_GROUPS.items():
        observed = runtime_resource_identity(candidate_root / retained_path)
        _require(
            observed == resources[group],
            f"{Path(retained_path).name} runtime resource digest/tree mismatch",
        )
    return resources


def _validate_staged_rust_runtime_resources(
    rust_source_root: Path,
    expected: Any,
) -> dict[str, Any]:
    resources = _validate_engine_runtime_resources(
        expected,
        "Rust runtime resources",
    )
    for group, (source_path, _) in RUNTIME_RESOURCE_GROUPS.items():
        observed = runtime_resource_identity(rust_source_root / source_path)
        _require(
            observed == resources[group],
            f"Rust {Path(source_path).name} runtime resource digest/tree mismatch",
        )
    return resources


def _copy_regular_file(source: Path, destination: Path, label: str) -> None:
    _regular_file(source, label)
    _require(not destination.exists(), f"{label} destination already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        shutil.copyfile(source, destination)
    except OSError as error:
        raise AcquisitionFailure(f"could not copy {label}: {error}") from error
    _regular_file(destination, f"copied {label}")
    _require(
        _sha256_file(source) == _sha256_file(destination),
        f"copied {label} differs from its source",
    )


def _write_json_new(path: Path, value: Any) -> None:
    _require(not path.exists(), f"JSON destination already exists: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")
    try:
        with path.open("xb") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
    except OSError as error:
        raise AcquisitionFailure(f"could not write JSON {path}: {error}") from error


def _write_bytes_new(path: Path, value: bytes) -> None:
    _require(
        not path.exists() and not path.is_symlink(),
        f"output destination already exists: {path}",
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("xb") as output:
            output.write(value)
            output.flush()
            os.fsync(output.fileno())
    except OSError as error:
        raise AcquisitionFailure(f"could not write output {path}: {error}") from error


def _git_blob_sha256(repository: Path, revision: str, relative_path: str) -> str:
    return hashlib.sha256(
        _git_bytes(repository, ["cat-file", "blob", f"{revision}:{relative_path}"])
    ).hexdigest()


def rust_source_input_inventory(
    repository: Path,
    revision: str,
    *,
    source_paths: Sequence[str] = RUST_SOURCE_INVENTORY_PATHS,
) -> dict[str, Any]:
    """Fingerprint all local build/capture bytes that must survive squash merge."""

    commit = _git_text(repository, ["rev-parse", "--verify", f"{revision}^{{commit}}"])
    raw = _git_bytes(
        repository,
        [
            "ls-tree",
            "-r",
            "-z",
            "--long",
            "--full-tree",
            commit,
            "--",
            *source_paths,
        ],
    )
    entries: list[dict[str, Any]] = []
    blob_hashes: dict[str, str] = {}
    seen_paths: set[str] = set()
    for encoded in raw.split(b"\0"):
        if not encoded:
            continue
        try:
            metadata, encoded_path = encoded.split(b"\t", 1)
            mode, kind, object_id, encoded_size = metadata.decode("ascii").split()
            relative_path = encoded_path.decode("utf-8", errors="strict")
            size = int(encoded_size)
        except (ValueError, UnicodeError) as error:
            raise AcquisitionFailure("could not parse Rust source input tree") from error
        _require(kind == "blob", f"Rust source input is not a blob: {relative_path}")
        _require(_is_lower_hex(object_id, 40), f"invalid blob OID for {relative_path}")
        _require(relative_path not in seen_paths, f"duplicate source input: {relative_path}")
        seen_paths.add(relative_path)
        digest = blob_hashes.get(object_id)
        if digest is None:
            blob = _git_bytes(repository, ["cat-file", "blob", object_id])
            _require(len(blob) == size, f"blob size drift for {relative_path}")
            digest = hashlib.sha256(blob).hexdigest()
            blob_hashes[object_id] = digest
        entries.append(
            {
                "path": relative_path,
                "mode": mode,
                "git_oid": object_id,
                "sha256": digest,
                "size_bytes": size,
            }
        )
    entries.sort(key=lambda entry: entry["path"])
    _require(entries, "Rust source input inventory is empty")
    inventoried_paths = [entry["path"] for entry in entries]
    for requested in source_paths:
        _require(
            requested in inventoried_paths
            or any(path.startswith(f"{requested}/") for path in inventoried_paths),
            f"Rust source input path is not committed or is empty: {requested}",
        )
    return {
        "schema": RUST_SOURCE_INVENTORY_SCHEMA,
        "algorithm": "sorted-git-blob-sha256-v1",
        "sha256": _canonical_json_sha256(entries),
        "entries": entries,
    }


def validate_rust_source_input_inventory(
    value: Any,
    *,
    expected: Any | None = None,
) -> dict[str, Any]:
    inventory = _require_exact_keys(
        value,
        {"schema", "algorithm", "sha256", "entries"},
        "Rust source input inventory",
    )
    _require(
        inventory["schema"] == RUST_SOURCE_INVENTORY_SCHEMA,
        "Rust source input inventory schema is unsupported",
    )
    _require(
        inventory["algorithm"] == "sorted-git-blob-sha256-v1",
        "Rust source input inventory algorithm drift",
    )
    entries = inventory["entries"]
    _require(isinstance(entries, list) and entries, "Rust source input entries are missing")
    observed_paths: list[str] = []
    for index, entry in enumerate(entries):
        entry = _require_exact_keys(
            entry,
            {"path", "mode", "git_oid", "sha256", "size_bytes"},
            f"Rust source input {index}",
        )
        relative_path = entry["path"]
        _require(
            isinstance(relative_path, str)
            and relative_path
            and not Path(relative_path).is_absolute()
            and ".." not in Path(relative_path).parts,
            f"Rust source input {index} has an unsafe path",
        )
        _require(
            isinstance(entry["mode"], str)
            and len(entry["mode"]) == 6
            and entry["mode"].isdigit(),
            f"Rust source input {relative_path} has an invalid mode",
        )
        _require(
            _is_lower_hex(entry["git_oid"], 40),
            f"Rust source input {relative_path} has an invalid Git OID",
        )
        _require(
            _is_lower_hex(entry["sha256"], 64),
            f"Rust source input {relative_path} has an invalid SHA-256",
        )
        _require(
            type(entry["size_bytes"]) is int and entry["size_bytes"] >= 0,
            f"Rust source input {relative_path} has an invalid size",
        )
        observed_paths.append(relative_path)
    _require(
        observed_paths == sorted(observed_paths)
        and len(observed_paths) == len(set(observed_paths)),
        "Rust source input paths must be sorted and unique",
    )
    _require(
        inventory["sha256"] == _canonical_json_sha256(entries),
        "Rust source input inventory SHA-256 mismatch",
    )
    if expected is not None:
        _require(
            inventory == validate_rust_source_input_inventory(expected),
            "Rust source inputs differ from the recorded acquisition bytes",
        )
    return inventory


def _validate_launch_record(
    launch: Any,
    *,
    artifact_root: Path,
    run_id: str,
    nonce: str,
    engine: str,
    case_id: str,
    expected_runtime_resources: Any,
    accepted_inventory: bool,
) -> Path:
    launch = _require_exact_keys(
        launch,
        {"argv", "cwd", "environment", "runtime_resources"},
        f"{run_id} {engine} {case_id} launch",
    )
    launch_resources = _validate_engine_runtime_resources(
        launch["runtime_resources"],
        f"{run_id} {engine} {case_id} launch runtime resources",
    )
    _require(
        launch_resources == expected_runtime_resources,
        f"{run_id} {engine} {case_id} launch runtime resources drift",
    )
    argv = launch["argv"]
    _require(
        isinstance(argv, list)
        and argv
        and all(isinstance(argument, str) and argument for argument in argv),
        f"{run_id} {engine} {case_id} launch argv is invalid",
    )
    _require(
        isinstance(launch["cwd"], str) and Path(launch["cwd"]).is_absolute(),
        f"{run_id} {engine} {case_id} launch cwd is invalid",
    )
    environment = launch["environment"]
    _require(
        isinstance(environment, dict)
        and all(
            isinstance(key, str) and key and isinstance(value, str)
            for key, value in environment.items()
        ),
        f"{run_id} {engine} {case_id} launch environment is invalid",
    )
    output = Path(environment.get("CLONK_PRESENTATION_OUTPUT_DIR", ""))
    _require(output.is_absolute(), f"{run_id} {engine} {case_id} output path is invalid")
    try:
        recorded_root = output.parents[2]
    except IndexError as error:
        raise AcquisitionFailure(
            f"{run_id} {engine} {case_id} output path has no candidate root"
        ) from error
    expected_environment = _capture_environment(
        recorded_root,
        run_id=run_id,
        nonce=nonce,
        engine=engine,
        case_id=case_id,
    )
    _require(
        environment == expected_environment,
        f"{run_id} {engine} {case_id} launch environment drift",
    )
    if not accepted_inventory:
        _require(
            recorded_root.resolve() == artifact_root.resolve(),
            f"{run_id} {engine} {case_id} launch candidate root mismatch",
        )
    expected_cwd = (
        recorded_root / "inputs"
        if engine == "cpp"
        else recorded_root / "work/rust-source"
    )
    _require(
        Path(launch["cwd"]) == expected_cwd,
        f"{run_id} {engine} {case_id} launch cwd drift",
    )
    if engine == "rust":
        expected_argv = [
            str(recorded_root / "work/rust-source/target/release/clonk-app"),
            "--config",
            str(recorded_root / "inputs/rust.config"),
            str(recorded_root / PLAYER_RETAINED_PATH),
            "/compatprofile:legacy-clonk",
        ]
        _require(argv == expected_argv, f"{run_id} rust {case_id} launch argv drift")
    else:
        _require(engine == "cpp", f"unsupported launch engine: {engine}")
        _require(
            argv[0]
            == str(recorded_root / "work/oracle-source/build/clonk.app/Contents/MacOS/clonk"),
            f"{run_id} C++ {case_id} executable path drift",
        )
        _require(
            f"/config:{recorded_root / 'inputs/cpp.config'}" in argv[1:],
            f"{run_id} C++ {case_id} config argv drift",
        )
        startup = CPP_STARTUP_ARGUMENTS.get(case_id)
        if startup is not None:
            _require(
                argv == [argv[0], startup, f"/config:{recorded_root / 'inputs/cpp.config'}"],
                f"{run_id} C++ {case_id} startup argv drift",
            )
        else:
            scenario_relative = CPP_RUNTIME_SCENARIOS.get(case_id)
            _require(
                scenario_relative is not None,
                f"{run_id} C++ {case_id} uses an unaudited runtime argv",
            )
            network_arguments = (
                ["/network", "/lobby"] if case_id == "network-lobby" else []
            )
            _require(
                argv
                == [
                    argv[0],
                    str(recorded_root / "work/fixture-content" / scenario_relative),
                    *network_arguments,
                    f"/config:{recorded_root / 'inputs/cpp.config'}",
                ],
                f"{run_id} C++ {case_id} runtime argv drift",
            )
    return recorded_root


def _validate_v2_patch_binding(
    index: Any,
    artifact_root: Path,
    *,
    trusted_patch: Path | None,
    trusted_patch_sha256: str | None = None,
    retained: bool = True,
) -> list[Path]:
    _require(isinstance(index, dict), "provenance index must be an object")
    patch = index.get("patch")
    _require(isinstance(patch, dict), "capture patch provenance is missing")
    _require(
        frozenset(patch) == {"source_path", "retained_path", "base_commit", "sha256"},
        "capture patch schema drift",
    )
    _require(
        patch["source_path"] == CAPTURE_PATCH_SOURCE_PATH,
        "capture patch source path drift",
    )
    _require(
        patch["retained_path"] == CAPTURE_PATCH_RETAINED_PATH,
        "capture patch retained path drift",
    )
    _require(
        patch["base_commit"] == ORACLE_SOURCE_COMMIT,
        "capture patch base commit drift",
    )
    _require(_is_lower_hex(patch["sha256"], 64), "capture patch SHA-256 is invalid")
    if trusted_patch_sha256 is None:
        _require(trusted_patch is not None, "trusted capture patch is missing")
        _regular_file(trusted_patch, "trusted capture patch")
        trusted_patch_sha256 = _sha256_file(trusted_patch)
    _require(
        patch["sha256"] == trusted_patch_sha256,
        "capture patch SHA-256 does not bind the trusted patch",
    )
    if not retained:
        return []
    retained_patch = artifact_root / CAPTURE_PATCH_RETAINED_PATH
    _regular_file(retained_patch, "retained capture patch")
    _require(
        patch["sha256"] == _sha256_file(retained_patch),
        "capture patch SHA-256 does not bind the retained patch",
    )
    return [retained_patch]


def _validate_v2_file_record(
    record: Any,
    artifact_root: Path,
    *,
    expected_path: str,
    label: str,
) -> Path:
    record = _require_exact_keys(record, {"path", "sha256", "size_bytes"}, label)
    _require(record["path"] == expected_path, f"{label} path drift")
    _require(_is_lower_hex(record["sha256"], 64), f"{label} SHA-256 is invalid")
    _require(
        type(record["size_bytes"]) is int and record["size_bytes"] >= 0,
        f"{label} size_bytes must be a non-negative integer",
    )
    path = artifact_root / expected_path
    _regular_file(path, label)
    _require(_sha256_file(path) == record["sha256"], f"{label} SHA-256 mismatch")
    try:
        observed_size = path.stat().st_size
    except OSError as error:
        raise AcquisitionFailure(f"could not stat {label} {path}: {error}") from error
    _require(observed_size == record["size_bytes"], f"{label} size mismatch")
    return path


def _validate_v2_file_record_metadata(
    record: Any,
    *,
    expected_path: str,
    label: str,
) -> None:
    record = _require_exact_keys(record, {"path", "sha256", "size_bytes"}, label)
    _require(record["path"] == expected_path, f"{label} path drift")
    _require(_is_lower_hex(record["sha256"], 64), f"{label} SHA-256 is invalid")
    _require(
        type(record["size_bytes"]) is int and record["size_bytes"] >= 0,
        f"{label} size_bytes must be a non-negative integer",
    )


def _validate_v2_sources(sources: Any, expected_fields: Mapping[str, str]) -> None:
    _require(
        frozenset(expected_fields) == CONTEXT_FIELDS,
        "internal expected v2 provenance context is incomplete",
    )
    sources = _require_exact_keys(
        sources,
        {
            "oracle",
            "rust",
            "fixture_content",
            "effective_system_scripts_sha256",
            "runtime_resources",
        },
        "sources",
    )
    oracle = _require_exact_keys(
        sources["oracle"],
        {"commit", "tree", "historical_content_gitlink", "planet_tree", "system_tree"},
        "oracle source",
    )
    rust = _require_exact_keys(
        sources["rust"],
        {"commit", "tree", "planet_tree", "system_tree"},
        "Rust source",
    )
    fixture = _require_exact_keys(
        sources["fixture_content"], {"commit", "tree"}, "fixture content"
    )
    expected = {
        "oracle.commit": ORACLE_SOURCE_COMMIT,
        "oracle.tree": expected_fields["oracle_source_tree"],
        "oracle.historical_content_gitlink": ORACLE_SOURCE_CONTENT_GITLINK,
        "oracle.planet_tree": expected_fields["oracle_planet_tree"],
        "oracle.system_tree": expected_fields["oracle_system_tree"],
        "rust.commit": expected_fields["rust_source_commit"],
        "rust.tree": expected_fields["rust_source_tree"],
        "rust.planet_tree": expected_fields["rust_planet_tree"],
        "rust.system_tree": expected_fields["rust_system_tree"],
        "fixture.commit": FIXTURE_CONTENT_COMMIT,
        "fixture.tree": expected_fields["fixture_content_tree"],
        "effective_system_scripts_sha256": expected_fields[
            "effective_system_scripts_sha256"
        ],
    }
    observed = {
        "oracle.commit": oracle["commit"],
        "oracle.tree": oracle["tree"],
        "oracle.historical_content_gitlink": oracle["historical_content_gitlink"],
        "oracle.planet_tree": oracle["planet_tree"],
        "oracle.system_tree": oracle["system_tree"],
        "rust.commit": rust["commit"],
        "rust.tree": rust["tree"],
        "rust.planet_tree": rust["planet_tree"],
        "rust.system_tree": rust["system_tree"],
        "fixture.commit": fixture["commit"],
        "fixture.tree": fixture["tree"],
        "effective_system_scripts_sha256": sources["effective_system_scripts_sha256"],
    }
    for field, expected_value in expected.items():
        observed_value = observed[field]
        length = 64 if field == "effective_system_scripts_sha256" else 40
        _require(_is_lower_hex(observed_value, length), f"sources {field} is invalid")
        _require(observed_value == expected_value, f"sources {field} drift")
    runtime_resources = _require_exact_keys(
        sources["runtime_resources"], ENGINE_IDS, "sources runtime resources"
    )
    expected_runtime = _require_exact_keys(
        expected_fields["runtime_resources"],
        ENGINE_IDS,
        "expected runtime resources",
    )
    for engine in ENGINE_IDS:
        observed_resources = _validate_engine_runtime_resources(
            runtime_resources[engine], f"sources {engine} runtime resources"
        )
        expected_resources = _validate_engine_runtime_resources(
            expected_runtime[engine], f"expected {engine} runtime resources"
        )
        _require(
            observed_resources == expected_resources,
            f"sources {engine} runtime resources drift",
        )


def _validate_v2_case_specs(
    case_specs: Any,
    *,
    expected_case_specs: Sequence[Mapping[str, Any]] | None,
    trusted_manifest: Any | None = None,
) -> list[Mapping[str, Any]]:
    if expected_case_specs is None:
        _require(
            CASE_SPECS_PATH.is_file() and not CASE_SPECS_PATH.is_symlink(),
            f"presentation case contract not implemented: {CASE_SPECS_SOURCE_PATH}",
        )
        loaded_case_specs = load_json(CASE_SPECS_PATH)
        _require(isinstance(loaded_case_specs, list), "tracked case contract must be an array")
        expected_case_specs = loaded_case_specs
    _require(isinstance(case_specs, list), "case_specs must be an array")
    expected = [dict(spec) for spec in expected_case_specs]
    _require(case_specs == expected, "case_specs differ from the trusted case contract")
    _require(
        [spec.get("id") for spec in case_specs] == list(CASE_IDS),
        "case_specs must use the canonical case order",
    )
    manifest = load_json(CAPTURE_MANIFEST_PATH) if trusted_manifest is None else trusted_manifest
    screens = manifest.get("screens") if isinstance(manifest, dict) else None
    _require(isinstance(screens, list), "capture manifest screens are missing")
    manifest_terms = {
        screen.get("id"): {
            "comparison": screen.get("comparison", "pixel"),
            "port_assets": screen.get("port_assets", []),
        }
        for screen in screens
        if isinstance(screen, dict)
    }
    for spec in case_specs:
        spec = _require_exact_keys(
            spec,
            {
                "id",
                "comparison",
                "port_asset_exemptions",
                "config_sha256",
                "player_sha256",
                "locale",
                "seeds",
                "trigger",
                "scenario",
                "frame",
                "runtime_resources",
            },
            f"case spec {spec.get('id') if isinstance(spec, dict) else '?'}",
        )
        case_id = spec["id"]
        _require(case_id in manifest_terms, f"unknown case spec {case_id}")
        _require(
            spec["comparison"] == manifest_terms[case_id]["comparison"],
            f"case spec comparison term drift for {case_id}",
        )
        exemptions = spec["port_asset_exemptions"]
        _require(
            isinstance(exemptions, dict)
            and all(
                isinstance(path, str)
                and path
                and isinstance(asset, str)
                and asset
                for path, asset in exemptions.items()
            ),
            f"case spec port asset exemptions are invalid for {case_id}",
        )
        _require(
            exemptions == PORT_ASSET_EXEMPTIONS[case_id],
            f"case spec port asset exemption paths drift for {case_id}",
        )
        _require(
            sorted(set(exemptions.values()))
            == sorted(manifest_terms[case_id]["port_assets"]),
            f"case spec port asset classes drift for {case_id}",
        )
        config_sha256 = _require_exact_keys(
            spec["config_sha256"], ENGINE_IDS, f"{case_id} config hashes"
        )
        for engine in ENGINE_IDS:
            _require(
                _is_lower_hex(config_sha256[engine], 64),
                f"{case_id} {engine} config SHA-256 is invalid",
            )
        _require(
            _is_lower_hex(spec["player_sha256"], 64),
            f"{case_id} player SHA-256 is invalid",
        )
        _require(
            spec["player_sha256"] == PLAYER_SHA256,
            f"{case_id} player SHA-256 differs from the audited presentation player",
        )
        locale = _require_exact_keys(
            spec["locale"], {"language", "charset", "lang", "lc_all", "tz"}, f"{case_id} locale"
        )
        _require(
            all(isinstance(value, str) and value for value in locale.values()),
            f"{case_id} locale values must be non-empty strings",
        )
        _require(
            locale == EXPECTED_LOCALE,
            f"{case_id} locale differs from the canonical locale",
        )
        seeds = _require_exact_keys(
            spec["seeds"], {"simulation", "presentation"}, f"{case_id} seeds"
        )
        simulation = _require_exact_keys(
            seeds["simulation"], {"seed", "calls"}, f"{case_id} simulation seed"
        )
        _require(
            all(
                type(simulation[field]) is int and simulation[field] >= 0
                for field in ("seed", "calls")
            ),
            f"{case_id} simulation seed and calls must be non-negative integers",
        )
        _validate_presentation_rng(
            seeds["presentation"], f"{case_id} presentation seed"
        )
        trigger = _require_exact_keys(spec["trigger"], {"id"}, f"{case_id} trigger")
        _require(
            isinstance(trigger["id"], str) and trigger["id"].strip(),
            f"{case_id} trigger id must be non-empty",
        )
        scenario = _require_exact_keys(
            spec["scenario"], {"path", "content_tree"}, f"{case_id} scenario"
        )
        _require(
            scenario["path"] is None or isinstance(scenario["path"], str),
            f"{case_id} scenario path must be a string or null",
        )
        _require(
            _is_lower_hex(scenario["content_tree"], 40),
            f"{case_id} scenario content tree is invalid",
        )
        frame = _require_exact_keys(
            spec["frame"], {"checkpoint", "number"}, f"{case_id} frame"
        )
        _require(
            isinstance(frame["checkpoint"], str) and frame["checkpoint"].strip(),
            f"{case_id} frame checkpoint must be non-empty",
        )
        _require(
            type(frame["number"]) is int and frame["number"] >= 0,
            f"{case_id} frame number must be a non-negative integer",
        )
        runtime_resources = _require_exact_keys(
            spec["runtime_resources"], ENGINE_IDS, f"{case_id} runtime resources"
        )
        for engine in ENGINE_IDS:
            _validate_engine_runtime_resources(
                runtime_resources[engine],
                f"{case_id} {engine} runtime resources",
            )
    return expected


def _validate_v2_provenance_index(
    index: Any,
    artifact_root: Path,
    *,
    expected_fields: Mapping[str, str],
    expected_case_specs: Sequence[Mapping[str, Any]] | None,
    trusted_patch: Path | None,
    trusted_launcher: Path | None,
    trusted_manifest: Any | None = None,
    trusted_manifest_contract_sha256: str | None = None,
    trusted_profile: Any | None = None,
    trusted_profile_contract_sha256: str | None = None,
    expected_source_inventory: Any | None = None,
    trusted_patch_sha256: str | None = None,
    trusted_launcher_sha256: str | None = None,
    accepted_inventory: bool = False,
) -> list[Path]:
    index = _require_exact_keys(index, V2_INDEX_FIELDS, "provenance index")
    _require(index["schema"] == PROVENANCE_SCHEMA, "unsupported provenance schema")
    accepted = _validate_v2_patch_binding(
        index,
        artifact_root,
        trusted_patch=trusted_patch,
        trusted_patch_sha256=trusted_patch_sha256,
        retained=not accepted_inventory,
    )
    _validate_v2_sources(index["sources"], expected_fields)
    case_specs = _validate_v2_case_specs(
        index["case_specs"],
        expected_case_specs=expected_case_specs,
        trusted_manifest=trusted_manifest,
    )
    case_specs_by_id = {spec["id"]: spec for spec in case_specs}
    for case_spec in case_specs:
        scenario = _require_exact_keys(
            case_spec["scenario"], {"path", "content_tree"}, f"{case_spec['id']} scenario"
        )
        _require(
            scenario["content_tree"] == expected_fields["fixture_content_tree"],
            f"{case_spec['id']} scenario content tree drift",
        )
        _require(
            case_spec["runtime_resources"] == index["sources"]["runtime_resources"],
            f"{case_spec['id']} runtime resource provenance drift",
        )
    case_specs_sha256 = _canonical_json_sha256(case_specs)

    contract = _require_exact_keys(
        index["contract"],
        {
            "capture_manifest",
            "compat_profile",
            "case_specs",
            "geometry",
            "normalization",
            "launcher",
        },
        "capture contract",
    )
    capture_manifest = _require_exact_keys(
        contract["capture_manifest"],
        {"path", "contract_sha256"},
        "capture manifest binding",
    )
    _require(
        capture_manifest["path"] == CAPTURE_MANIFEST_SOURCE_PATH,
        "capture manifest path drift",
    )
    _require(
        capture_manifest["contract_sha256"]
        == (
            trusted_manifest_contract_sha256
            if trusted_manifest_contract_sha256 is not None
            else capture_manifest_contract_value_sha256(trusted_manifest)
            if trusted_manifest is not None
            else capture_manifest_contract_sha256(CAPTURE_MANIFEST_PATH)
        ),
        "capture manifest contract SHA-256 mismatch",
    )
    compat_profile = _require_exact_keys(
        contract["compat_profile"],
        {"path", "contract_sha256"},
        "compatibility profile binding",
    )
    _require(
        compat_profile["path"] == "compat/profile.json",
        "compatibility profile path drift",
    )
    _require(
        compat_profile["contract_sha256"]
        == (
            trusted_profile_contract_sha256
            if trusted_profile_contract_sha256 is not None
            else compat_profile_contract_value_sha256(trusted_profile)
            if trusted_profile is not None
            else compat_profile_contract_sha256(WORKSPACE / "compat/profile.json")
        ),
        "compatibility profile contract SHA-256 mismatch",
    )
    case_specs_binding = _require_exact_keys(
        contract["case_specs"], {"path", "sha256"}, "case_specs binding"
    )
    _require(
        case_specs_binding["path"] == CASE_SPECS_SOURCE_PATH,
        "case_specs path drift",
    )
    _require(case_specs_binding["sha256"] == case_specs_sha256, "case_specs SHA-256 mismatch")
    _require(contract["geometry"] == EXPECTED_GEOMETRY, "capture geometry drift")
    _require(
        contract["normalization"] == EXPECTED_NORMALIZATION,
        "capture normalization drift",
    )
    launcher = _require_exact_keys(
        contract["launcher"], {"source_path", "retained_path", "sha256"}, "launcher binding"
    )
    _require(launcher["source_path"] == LAUNCHER_SOURCE_PATH, "launcher source path drift")
    _require(
        launcher["retained_path"] == LAUNCHER_RETAINED_PATH,
        "launcher retained path drift",
    )
    if trusted_launcher_sha256 is None:
        _require(trusted_launcher is not None, "trusted launcher is missing")
        _regular_file(trusted_launcher, "trusted launcher")
        trusted_launcher_sha256 = _sha256_file(trusted_launcher)
    launcher_sha256 = trusted_launcher_sha256
    _require(
        launcher["sha256"] == launcher_sha256,
        "launcher SHA-256 mismatch",
    )
    if not accepted_inventory:
        retained_launcher = artifact_root / LAUNCHER_RETAINED_PATH
        _regular_file(retained_launcher, "retained launcher")
        _require(
            launcher_sha256 == _sha256_file(retained_launcher),
            "retained launcher SHA-256 mismatch",
        )
        accepted.append(retained_launcher)

    inputs = _require_exact_keys(
        index["inputs"],
        {"configs", "player", "network_references", "rust_source_inventory"},
        "capture inputs",
    )
    configs = _require_exact_keys(inputs["configs"], ENGINE_IDS, "capture configs")
    for engine in ENGINE_IDS:
        config = _require_exact_keys(
            configs[engine], {"path", "sha256", "size_bytes", "profile"}, f"{engine} config"
        )
        _require(config["profile"] == ENGINE_PROFILES[engine], f"{engine} config profile drift")
        record = {key: config[key] for key in ("path", "sha256", "size_bytes")}
        accepted.append(
            _validate_v2_file_record(
                record,
                artifact_root,
                expected_path=f"inputs/{engine}.config",
                label=f"{engine} native config",
            )
        )
        _require(
            config["sha256"] == NATIVE_CONFIG_SHA256,
            f"{engine} native config SHA-256 drift",
        )
    player = _require_exact_keys(
        inputs["player"], {"path", "sha256", "size_bytes", "id"}, "player fixture"
    )
    _require(
        isinstance(player["id"], str) and player["id"].strip(),
        "player fixture id must be non-empty",
    )
    accepted.append(
        _validate_v2_file_record(
            {key: player[key] for key in ("path", "sha256", "size_bytes")},
            artifact_root,
            expected_path=PLAYER_RETAINED_PATH,
            label="player fixture",
        )
    )
    _require(
        player["sha256"] == PLAYER_SHA256,
        "player fixture SHA-256 drift",
    )
    network_references = _validate_v2_file_record(
        inputs["network_references"],
        artifact_root,
        expected_path=NETWORK_REFERENCES_RETAINED_PATH,
        label="network reference fixture",
    )
    accepted.append(network_references)
    _require(
        inputs["network_references"]["sha256"] == NETWORK_REFERENCES_SHA256,
        "network reference fixture SHA-256 drift",
    )
    network_value = _require_exact_keys(
        load_json(network_references),
        {"schema", "references"},
        "network reference fixture",
    )
    _require(
        network_value["schema"] == "clonk-rs/presentation-network-references/v1"
        and network_value["references"] == [],
        "network reference fixture must be the canonical completed-empty list",
    )
    source_inventory_path = _validate_v2_file_record(
        inputs["rust_source_inventory"],
        artifact_root,
        expected_path=RUST_SOURCE_INVENTORY_RETAINED_PATH,
        label="Rust source input inventory",
    )
    accepted.append(source_inventory_path)
    validate_rust_source_input_inventory(
        load_json(source_inventory_path),
        expected=expected_source_inventory,
    )
    for case_spec in case_specs:
        for engine in ENGINE_IDS:
            _require(
                case_spec["config_sha256"][engine] == configs[engine]["sha256"],
                f"{case_spec['id']} {engine} config binding drift",
            )
        _require(
            case_spec["player_sha256"] == player["sha256"],
            f"{case_spec['id']} player fixture binding drift",
        )

    builds = _require_exact_keys(index["builds"], ENGINE_IDS, "capture builds")
    for engine in ENGINE_IDS:
        build = _require_exact_keys(
            builds[engine],
            {"source_tree", "capture_patch_sha256", "producer", "profile", "recipe", "binary"},
            f"{engine} build",
        )
        source_tree = (
            expected_fields["oracle_source_tree"]
            if engine == "cpp"
            else expected_fields["rust_source_tree"]
        )
        expected_patch = index["patch"]["sha256"] if engine == "cpp" else None
        _require(build["source_tree"] == source_tree, f"{engine} build source tree drift")
        _require(
            build["capture_patch_sha256"] == expected_patch,
            f"{engine} build patch binding drift",
        )
        _require(build["producer"] == ENGINE_PRODUCERS[engine], f"{engine} producer drift")
        _require(build["profile"] == ENGINE_PROFILES[engine], f"{engine} build profile drift")
        recipe = _require_exact_keys(
            build["recipe"], {"commands", "sha256"}, f"{engine} build recipe"
        )
        commands = recipe["commands"]
        _require(
            isinstance(commands, list) and commands,
            f"{engine} build commands must be a non-empty array",
        )
        for command_index, command in enumerate(commands):
            command = _require_exact_keys(
                command,
                {"argv", "cwd", "environment"},
                f"{engine} build command {command_index}",
            )
            _require(
                isinstance(command["argv"], list)
                and command["argv"]
                and all(isinstance(value, str) and value for value in command["argv"]),
                f"{engine} build command {command_index} argv is invalid",
            )
            _require(
                isinstance(command["cwd"], str) and command["cwd"],
                f"{engine} build command {command_index} cwd is invalid",
            )
            _require(
                isinstance(command["environment"], dict)
                and all(
                    isinstance(key, str)
                    and key
                    and isinstance(value, str)
                    for key, value in command["environment"].items()
                ),
                f"{engine} build command {command_index} environment is invalid",
            )
        _require(
            recipe["sha256"] == _canonical_json_sha256({"commands": commands}),
            f"{engine} build recipe SHA-256 mismatch",
        )
        if accepted_inventory:
            _validate_v2_file_record_metadata(
                build["binary"],
                expected_path=f"builds/{engine}/binary",
                label=f"{engine} binary",
            )
        else:
            accepted.append(
                _validate_v2_file_record(
                    build["binary"],
                    artifact_root,
                    expected_path=f"builds/{engine}/binary",
                    label=f"{engine} binary",
                )
            )

    runs = index["runs"]
    _require(isinstance(runs, list), "runs must be an array")
    _require(
        [run.get("id") for run in runs if isinstance(run, dict)] == list(RUN_IDS),
        "runs drift",
    )
    nonces = [run.get("nonce") for run in runs if isinstance(run, dict)]
    _require(
        len(nonces) == len(RUN_IDS) and len(set(nonces)) == len(RUN_IDS),
        "the two launcher runs must carry genuinely distinct nonces",
    )
    repeat_artifacts: dict[str, dict[str, dict[str, str]]] = {
        engine: {} for engine in ENGINE_IDS
    }
    captured_artifacts: dict[str, dict[str, dict[str, dict[str, Any]]]] = {
        run_id: {engine: {} for engine in ENGINE_IDS} for run_id in RUN_IDS
    }
    for run in runs:
        run = _require_exact_keys(
            run, {"id", "nonce", "launcher_receipt", "engines"}, f"run {run.get('id')}"
        )
        run_id = run["id"]
        _require(_is_lower_hex(run["nonce"], 64), f"{run_id} nonce is invalid")
        launcher_receipt = _validate_v2_file_record(
            run["launcher_receipt"],
            artifact_root,
            expected_path=f"{run_id}/launcher-receipt.json",
            label=f"{run_id} launcher receipt",
        )
        accepted.append(launcher_receipt)
        launcher_receipt_value = _require_exact_keys(
            load_json(launcher_receipt),
            LAUNCHER_RECEIPT_FIELDS,
            f"{run_id} launcher receipt",
        )
        _require(
            launcher_receipt_value["schema"] == LAUNCHER_RECEIPT_SCHEMA,
            f"{run_id} launcher receipt schema is unsupported",
        )
        _require(
            launcher_receipt_value["run_id"] == run_id,
            f"{run_id} launcher receipt run identity mismatch",
        )
        _require(
            launcher_receipt_value["nonce"] == run["nonce"],
            f"{run_id} launcher receipt is not nonce-bound",
        )
        _require(
            launcher_receipt_value["launcher_sha256"] == launcher_sha256,
            f"{run_id} launcher receipt hash mismatch",
        )
        _require(
            launcher_receipt_value["case_specs_sha256"] == case_specs_sha256,
            f"{run_id} launcher receipt case contract mismatch",
        )
        engines = _require_exact_keys(run["engines"], ENGINE_IDS, f"{run_id} engines")
        expected_receipt_bindings: dict[str, list[dict[str, str]]] = {}
        launches: list[dict[str, Any]] = []
        recorded_roots: set[Path] = set()
        for engine in ENGINE_IDS:
            expected_receipt_bindings[engine] = []
            repeat_artifacts[engine][run_id] = {}
            engine_run = _require_exact_keys(
                engines[engine], {"cases"}, f"{run_id} {engine} evidence"
            )
            cases = engine_run["cases"]
            _require(isinstance(cases, list), f"{run_id} {engine} cases must be an array")
            _require(
                [case.get("id") for case in cases if isinstance(case, dict)] == list(CASE_IDS),
                f"{run_id} {engine} case order drift",
            )
            for case in cases:
                case = _require_exact_keys(
                    case, {"id", "receipt", "launch"}, f"{run_id} {engine} case entry"
                )
                case_id = case["id"]
                _require(case_id in case_specs_by_id, f"receipt names unknown case {case_id}")
                case_spec = case_specs_by_id[case_id]
                recorded_roots.add(
                    _validate_launch_record(
                        case["launch"],
                        artifact_root=artifact_root,
                        run_id=run_id,
                        nonce=run["nonce"],
                        engine=engine,
                        case_id=case_id,
                        expected_runtime_resources=case_spec["runtime_resources"][engine],
                        accepted_inventory=accepted_inventory,
                    )
                )
                launches.append(
                    {"engine": engine, "case_id": case_id, **case["launch"]}
                )
                receipt = _validate_v2_file_record(
                    case["receipt"],
                    artifact_root,
                    expected_path=f"{run_id}/{engine}/receipts/{case_id}.json",
                    label=f"{run_id} {engine} {case_id} receipt",
                )
                accepted.append(receipt)
                expected_receipt_bindings[engine].append(
                    {"case_id": case_id, "sha256": case["receipt"]["sha256"]}
                )
                receipt_value = _require_exact_keys(
                    load_json(receipt),
                    ENGINE_RECEIPT_FIELDS,
                    f"{run_id} {engine} {case_id} receipt",
                )
                _require(
                    receipt_value["schema"] == ENGINE_RECEIPT_SCHEMA,
                    f"{run_id} {engine} {case_id} receipt schema is unsupported",
                )
                _require(
                    receipt_value["run_id"] == run_id,
                    f"{run_id} {engine} {case_id} receipt run identity mismatch",
                )
                _require(
                    receipt_value["launcher_nonce"] == run["nonce"],
                    f"{run_id} {engine} {case_id} receipt is not nonce-bound",
                )
                _require(
                    receipt_value["engine"] == engine,
                    f"{run_id} {engine} {case_id} receipt engine identity mismatch",
                )
                _require(
                    receipt_value["producer"] == ENGINE_PRODUCERS[engine],
                    f"{run_id} {engine} {case_id} receipt producer identity mismatch",
                )
                _require(
                    receipt_value["case_id"] == case_id,
                    f"{run_id} {engine} {case_id} receipt case identity mismatch",
                )
                _require(
                    receipt_value["frame"] == case_spec["frame"],
                    f"{run_id} {engine} {case_id} receipt frame/checkpoint mismatch",
                )
                _require(
                    receipt_value["content_tree"]
                    == case_spec["scenario"]["content_tree"],
                    f"{run_id} {engine} {case_id} receipt content tree mismatch",
                )
                _require(
                    receipt_value["profile"] == ENGINE_PROFILES[engine],
                    f"{run_id} {engine} {case_id} receipt profile mismatch",
                )
                _require(
                    receipt_value["binary_sha256"]
                    == builds[engine]["binary"]["sha256"],
                    f"{run_id} {engine} {case_id} receipt binary SHA-256 mismatch",
                )
                expected_input_bindings = {
                    "source_tree": builds[engine]["source_tree"],
                    "config_sha256": case_spec["config_sha256"][engine],
                    "player_sha256": case_spec["player_sha256"],
                    "network_references_sha256": inputs["network_references"][
                        "sha256"
                    ],
                    "locale": case_spec["locale"],
                    "seeds": case_spec["seeds"],
                    "trigger": case_spec["trigger"],
                    "scenario": case_spec["scenario"],
                    "runtime_resources": case_spec["runtime_resources"][engine],
                }
                for field, expected_value in expected_input_bindings.items():
                    _require(
                        receipt_value[field] == expected_value,
                        f"{run_id} {engine} {case_id} receipt {field} binding mismatch",
                    )
                expected_artifact_keys = (
                    {"png", "layout"} if case_id in LAYOUT_CASE_IDS else {"png"}
                )
                artifacts = _require_exact_keys(
                    receipt_value["artifacts"],
                    expected_artifact_keys,
                    f"{run_id} {engine} {case_id} artifacts",
                )
                png = _validate_v2_file_record(
                    artifacts["png"],
                    artifact_root,
                    expected_path=f"{run_id}/{engine}/artifacts/{case_id}.png",
                    label=f"{run_id} {engine} {case_id} PNG artifact",
                )
                validate_png(png)
                accepted.append(png)
                repeat_artifacts[engine][run_id][f"{case_id}.png"] = artifacts[
                    "png"
                ]["sha256"]
                if case_id in LAYOUT_CASE_IDS:
                    layout = _validate_v2_file_record(
                        artifacts["layout"],
                        artifact_root,
                        expected_path=(
                            f"{run_id}/{engine}/artifacts/{case_id}.layout.json"
                        ),
                        label=f"{run_id} {engine} {case_id} layout artifact",
                    )
                    validate_layout_trace(
                        layout,
                        case_id,
                        port_asset_exemptions=case_spec[
                            "port_asset_exemptions"
                        ],
                    )
                    accepted.append(layout)
                    repeat_artifacts[engine][run_id][f"{case_id}.layout.json"] = artifacts[
                        "layout"
                    ]["sha256"]
                captured_artifacts[run_id][engine][case_id] = copy.deepcopy(artifacts)
        _require(
            launcher_receipt_value["engine_receipts"] == expected_receipt_bindings,
            f"{run_id} launcher receipt does not bind the ordered engine receipts",
        )
        _require(
            len(recorded_roots) == 1,
            f"{run_id} launches do not share one historical candidate root",
        )
        _require(
            launcher_receipt_value["launches_sha256"]
            == _canonical_json_sha256(launches),
            f"{run_id} launcher receipt does not bind exact launches",
        )
    for engine in ENGINE_IDS:
        _require(
            repeat_artifacts[engine][RUN_IDS[0]]
            == repeat_artifacts[engine][RUN_IDS[1]],
            f"{engine} repeat capture artifacts differ",
        )
    comparisons = index["comparisons"]
    _require(isinstance(comparisons, list), "comparisons must be an array")
    _require(
        [comparison.get("id") for comparison in comparisons if isinstance(comparison, dict)]
        == list(RUN_IDS),
        "comparison runs drift",
    )
    for comparison_run in comparisons:
        comparison_run = _require_exact_keys(
            comparison_run,
            {"id", "cases"},
            f"comparison run {comparison_run.get('id') if isinstance(comparison_run, dict) else '?'}",
        )
        run_id = comparison_run["id"]
        cases = comparison_run["cases"]
        _require(isinstance(cases, list), f"{run_id} comparisons must be an array")
        _require(
            [case.get("id") for case in cases if isinstance(case, dict)]
            == list(CASE_IDS),
            f"{run_id} comparison case order drift",
        )
        for case in cases:
            case = _require_exact_keys(
                case,
                {"id", "comparison", "reference", "actual", "receipt"},
                f"{run_id} comparison entry",
            )
            case_id = case["id"]
            artifact_key = "layout" if case_id in LAYOUT_CASE_IDS else "png"
            expected_term = "layout" if case_id in LAYOUT_CASE_IDS else "pixel"
            _require(
                case["comparison"] == expected_term,
                f"{run_id} {case_id} comparison term drift",
            )
            _require(
                case["reference"]
                == captured_artifacts[run_id]["cpp"][case_id][artifact_key],
                f"{run_id} {case_id} comparison reference binding drift",
            )
            _require(
                case["actual"]
                == captured_artifacts[run_id]["rust"][case_id][artifact_key],
                f"{run_id} {case_id} comparison actual binding drift",
            )
            receipt = _validate_v2_file_record(
                case["receipt"],
                artifact_root,
                expected_path=f"{run_id}/comparisons/{case_id}.json",
                label=f"{run_id} {case_id} comparison receipt",
            )
            accepted.append(receipt)
            expected_result = {
                "schema": COMPARISON_RECEIPT_SCHEMA,
                "case_id": case_id,
                "comparison": expected_term,
                "status": "match",
            }
            attestation = _require_exact_keys(
                load_json(receipt),
                {
                    "schema",
                    "run_id",
                    "case_id",
                    "comparison",
                    "comparator",
                    "reference",
                    "actual",
                    "stdout",
                },
                f"{run_id} {case_id} comparison attestation",
            )
            _require(
                attestation["schema"] == COMPARISON_ATTESTATION_SCHEMA
                and attestation["run_id"] == run_id
                and attestation["case_id"] == case_id
                and attestation["comparison"] == expected_term,
                f"{run_id} {case_id} comparison attestation identity mismatch",
            )
            comparator = _require_exact_keys(
                attestation["comparator"],
                {"schema", "binary_sha256"},
                f"{run_id} {case_id} comparison comparator",
            )
            _require(
                comparator
                == {
                    "schema": COMPARISON_RECEIPT_SCHEMA,
                    "binary_sha256": builds["rust"]["binary"]["sha256"],
                },
                f"{run_id} {case_id} comparison comparator binding mismatch",
            )
            for binding_name, indexed_record in (
                ("reference", case["reference"]),
                ("actual", case["actual"]),
            ):
                binding = _require_exact_keys(
                    attestation[binding_name],
                    {"path", "sha256"},
                    f"{run_id} {case_id} comparison {binding_name}",
                )
                _require(
                    binding
                    == {
                        "path": indexed_record["path"],
                        "sha256": indexed_record["sha256"],
                    },
                    f"{run_id} {case_id} comparison {binding_name} binding mismatch",
                )
            compact = json.dumps(expected_result, separators=(",", ":"))
            _require(
                attestation["stdout"] in {compact, compact + "\n"},
                f"{run_id} {case_id} comparison stdout is not exact",
            )
    expected_files = {
        "index.json",
        *(path.relative_to(artifact_root).as_posix() for path in accepted),
    }
    observed_files: set[str] = set()
    for path in artifact_root.rglob("*"):
        _require(not path.is_symlink(), f"candidate evidence contains a symlink: {path}")
        if path.is_file():
            observed_files.add(path.relative_to(artifact_root).as_posix())
        else:
            _require(path.is_dir(), f"candidate evidence contains a special file: {path}")
    _require(
        observed_files == expected_files,
        "candidate file inventory drift: "
        f"missing={sorted(expected_files - observed_files)} "
        f"unexpected={sorted(observed_files - expected_files)}",
    )
    return accepted


def validate_provenance_index(
    index: Any,
    artifact_root: Path,
    *,
    expected_fields: Mapping[str, str],
    expected_case_specs: Sequence[Mapping[str, Any]] | None = None,
    trusted_patch: Path = CAPTURE_PATCH_PATH,
    trusted_launcher: Path = LAUNCHER_PATH,
    trusted_manifest: Any | None = None,
    trusted_manifest_contract_sha256: str | None = None,
    trusted_profile: Any | None = None,
    trusted_profile_contract_sha256: str | None = None,
    expected_source_inventory: Any | None = None,
    trusted_patch_sha256: str | None = None,
    trusted_launcher_sha256: str | None = None,
    accepted_inventory: bool = False,
) -> list[Path]:
    """Validate exact global provenance and every indexed capture artifact."""

    return _validate_v2_provenance_index(
        index,
        artifact_root,
        expected_fields=expected_fields,
        expected_case_specs=expected_case_specs,
        trusted_patch=trusted_patch,
        trusted_launcher=trusted_launcher,
        trusted_manifest=trusted_manifest,
        trusted_manifest_contract_sha256=trusted_manifest_contract_sha256,
        trusted_profile=trusted_profile,
        trusted_profile_contract_sha256=trusted_profile_contract_sha256,
        expected_source_inventory=expected_source_inventory,
        trusted_patch_sha256=trusted_patch_sha256,
        trusted_launcher_sha256=trusted_launcher_sha256,
        accepted_inventory=accepted_inventory,
    )


def validate_candidate_output(
    output_directory: Path,
    *,
    expected_fields: Mapping[str, str],
    expected_case_specs: Sequence[Mapping[str, Any]] | None = None,
    trusted_patch: Path = CAPTURE_PATCH_PATH,
    trusted_launcher: Path = LAUNCHER_PATH,
    trusted_manifest: Any | None = None,
    trusted_manifest_contract_sha256: str | None = None,
    trusted_profile: Any | None = None,
    trusted_profile_contract_sha256: str | None = None,
    expected_source_inventory: Any | None = None,
    trusted_patch_sha256: str | None = None,
    trusted_launcher_sha256: str | None = None,
    accepted_inventory: bool = False,
    trusted_comparator: Path | None = None,
) -> list[Path]:
    """Validate one exact v2 candidate evidence tree."""

    _require(
        trusted_comparator is not None,
        "validation requires a trusted Rust presentation comparator",
    )
    _require(
        output_directory.is_dir() and not output_directory.is_symlink(),
        f"explicit output directory is not a directory: {output_directory}",
    )
    index_path = output_directory / "index.json"
    index = load_json(index_path)
    indexed = validate_provenance_index(
        index,
        output_directory,
        expected_fields=expected_fields,
        expected_case_specs=expected_case_specs,
        trusted_patch=trusted_patch,
        trusted_launcher=trusted_launcher,
        trusted_manifest=trusted_manifest,
        trusted_manifest_contract_sha256=trusted_manifest_contract_sha256,
        trusted_profile=trusted_profile,
        trusted_profile_contract_sha256=trusted_profile_contract_sha256,
        expected_source_inventory=expected_source_inventory,
        trusted_patch_sha256=trusted_patch_sha256,
        trusted_launcher_sha256=trusted_launcher_sha256,
        accepted_inventory=accepted_inventory,
    )
    _rerun_indexed_comparisons(output_directory, index, trusted_comparator)
    return [index_path, *indexed]


def accept_candidate_output(
    output_directory: Path,
    destination: Path,
    *,
    expected_fields: Mapping[str, str],
    expected_case_specs: Sequence[Mapping[str, Any]] | None = None,
    trusted_patch: Path = CAPTURE_PATCH_PATH,
    trusted_launcher: Path = LAUNCHER_PATH,
    trusted_manifest: Any | None = None,
    trusted_manifest_contract_sha256: str | None = None,
    trusted_profile: Any | None = None,
    trusted_profile_contract_sha256: str | None = None,
    expected_source_inventory: Any | None = None,
    trusted_comparator: Path | None = None,
) -> list[Path]:
    """Copy audited artifacts/provenance, excluding acquisition-time binaries."""

    validated = validate_candidate_output(
        output_directory,
        expected_fields=expected_fields,
        expected_case_specs=expected_case_specs,
        trusted_patch=trusted_patch,
        trusted_launcher=trusted_launcher,
        trusted_manifest=trusted_manifest,
        trusted_manifest_contract_sha256=trusted_manifest_contract_sha256,
        trusted_profile=trusted_profile,
        trusted_profile_contract_sha256=trusted_profile_contract_sha256,
        expected_source_inventory=expected_source_inventory,
        trusted_comparator=trusted_comparator,
    )
    _require(not destination.exists(), f"accept destination must not exist: {destination}")
    _require(destination.is_absolute(), "accept destination must be absolute")
    candidate_resolved = output_directory.resolve()
    destination_resolved = destination.resolve(strict=False)
    try:
        destination_resolved.relative_to(candidate_resolved)
    except ValueError:
        pass
    else:
        raise AcquisitionFailure("accept destination must not be inside candidate output")
    try:
        candidate_resolved.relative_to(destination_resolved)
    except ValueError:
        pass
    else:
        raise AcquisitionFailure("candidate output must not be inside accept destination")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.mkdir()
    copied = []
    try:
        for source in validated:
            relative = source.relative_to(output_directory)
            retained_inputs = {
                "index.json",
                "inputs/cpp.config",
                "inputs/rust.config",
                PLAYER_RETAINED_PATH,
                NETWORK_REFERENCES_RETAINED_PATH,
                RUST_SOURCE_INVENTORY_RETAINED_PATH,
            }
            if (
                relative.as_posix() not in retained_inputs
                and relative.parts[0] not in RUN_IDS
            ):
                continue
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            copied.append(target)
        revalidated = validate_candidate_output(
            destination,
            expected_fields=expected_fields,
            expected_case_specs=expected_case_specs,
            trusted_patch=trusted_patch,
            trusted_launcher=trusted_launcher,
            trusted_manifest=trusted_manifest,
            trusted_manifest_contract_sha256=trusted_manifest_contract_sha256,
            trusted_profile=trusted_profile,
            trusted_profile_contract_sha256=trusted_profile_contract_sha256,
            expected_source_inventory=expected_source_inventory,
            accepted_inventory=True,
            trusted_comparator=trusted_comparator,
        )
        _require(
            {path.relative_to(destination).as_posix() for path in revalidated}
            == {path.relative_to(destination).as_posix() for path in copied},
            "accepted copy file inventory differs after revalidation",
        )
    except Exception as error:
        try:
            shutil.rmtree(destination)
        except OSError as cleanup_error:
            raise AcquisitionFailure(
                f"could not remove failed accepted evidence {destination}: {cleanup_error}"
            ) from error
        if isinstance(error, AcquisitionFailure):
            raise
        raise AcquisitionFailure(f"could not accept validated evidence: {error}") from error
    return copied


def verify_accepted_output(
    repository_root: Path,
    *,
    output_directory: Path | None = None,
    rerun_comparisons: bool = True,
    trusted_comparator_context: tuple[Path, str, str, dict[str, Any]] | None = None,
) -> list[Path]:
    """Revalidate checked-in evidence against squash-stable current source inputs."""

    repository_root = repository_root.resolve()
    _require(repository_root.is_dir(), f"repository root does not exist: {repository_root}")
    output = (
        output_directory.resolve()
        if output_directory is not None
        else repository_root / "compat/presentation/oracle/v1"
    )
    if rerun_comparisons:
        (
            trusted_comparator,
            comparator_commit,
            comparator_tree,
            comparator_inventory,
        ) = _select_trusted_current_comparator(
            repository_root,
            trusted_comparator_context,
        )
    else:
        _require(
            trusted_comparator_context is None,
            "index-only verification cannot accept a comparator",
        )
        trusted_comparator = None
        comparator_commit, comparator_tree = require_clean_source_revision(
            repository_root
        )
        comparator_inventory = rust_source_input_inventory(
            repository_root,
            comparator_commit,
        )
    index = load_json(output / "index.json")
    _require(isinstance(index, dict), "accepted provenance index must be an object")
    sources = index.get("sources")
    _require(isinstance(sources, dict), "accepted provenance sources are missing")
    rust = sources.get("rust")
    _require(isinstance(rust, dict), "accepted Rust source provenance is missing")
    rust_commit = rust.get("commit")
    _require(_is_lower_hex(rust_commit, 40), "accepted Rust source commit is invalid")

    recorded_tree = rust.get("tree")
    _require(_is_lower_hex(recorded_tree, 40), "accepted Rust source tree is invalid")
    expected_fields = expected_provenance_fields(
        repository_root,
        rust_revision="HEAD",
        workspace=repository_root,
        require_oracle_head=False,
    )
    # Branch commits are not durable after this repository's squash merge.
    # Keep the capture identity informative while the retained blob inventory
    # below supplies the squash-stable proof of the source bytes actually used.
    expected_fields["rust_source_commit"] = rust_commit
    expected_fields["rust_source_tree"] = recorded_tree
    trusted_manifest = load_json_at_revision(
        repository_root,
        "HEAD",
        CAPTURE_MANIFEST_SOURCE_PATH,
    )
    trusted_profile = load_json_at_revision(
        repository_root,
        "HEAD",
        "compat/profile.json",
    )
    _validate_final_presentation_lifecycle(trusted_manifest, trusted_profile)
    trusted_case_specs = load_json_at_revision(
        repository_root,
        "HEAD",
        CASE_SPECS_SOURCE_PATH,
    )
    _require(isinstance(trusted_case_specs, list), "recorded case specs must be an array")

    input_bindings = {
        CPP_CONFIG_SOURCE_PATH: ("inputs", "configs", "cpp"),
        RUST_CONFIG_SOURCE_PATH: ("inputs", "configs", "rust"),
        PLAYER_SOURCE_PATH: ("inputs", "player"),
        NETWORK_REFERENCES_SOURCE_PATH: ("inputs", "network_references"),
    }
    for source_path, index_path in input_bindings.items():
        record: Any = index
        for component in index_path:
            _require(isinstance(record, dict), f"accepted input binding is missing: {source_path}")
            record = record.get(component)
        _require(isinstance(record, dict), f"accepted input record is missing: {source_path}")
        _require(
            record.get("sha256")
            == _git_blob_sha256(repository_root, "HEAD", source_path),
            f"accepted input differs from the current checked-in blob: {source_path}",
        )

    patch_sha256 = _git_blob_sha256(
        repository_root,
        "HEAD",
        CAPTURE_PATCH_SOURCE_PATH,
    )
    launcher_sha256 = _git_blob_sha256(
        repository_root,
        "HEAD",
        LAUNCHER_SOURCE_PATH,
    )
    validation_arguments = {
        "expected_fields": expected_fields,
        "expected_case_specs": trusted_case_specs,
        "trusted_patch": None,
        "trusted_launcher": None,
        "trusted_manifest": trusted_manifest,
        "trusted_manifest_contract_sha256": capture_manifest_contract_value_sha256(
            trusted_manifest
        ),
        "trusted_profile": trusted_profile,
        "trusted_profile_contract_sha256": compat_profile_contract_value_sha256(
            trusted_profile
        ),
        "expected_source_inventory": None,
        "trusted_patch_sha256": patch_sha256,
        "trusted_launcher_sha256": launcher_sha256,
        "accepted_inventory": True,
    }
    if trusted_comparator is not None:
        validated = validate_candidate_output(
            output,
            **validation_arguments,
            trusted_comparator=trusted_comparator,
        )
    else:
        indexed = validate_provenance_index(
            index,
            output,
            **validation_arguments,
        )
        validated = [output / "index.json", *indexed]
    verify_clean_source_checkpoint(
        repository_root,
        expected_commit=comparator_commit,
        expected_tree=comparator_tree,
        expected_inventory=comparator_inventory,
        label="after accepted comparison",
    )
    return validated


def verify_accepted_index(repository_root: Path) -> list[Path]:
    """Validate accepted evidence structure and provenance without execution."""

    return verify_accepted_output(repository_root, rerun_comparisons=False)


def _validate_current_rust_case(
    output_root: Path,
    case_entry: Mapping[str, Any],
    *,
    run_id: str,
    nonce: str,
    case_spec: Mapping[str, Any],
    source_tree: str,
    binary_sha256: str,
    config_sha256: str,
    player_sha256: str,
    network_references_sha256: str,
) -> dict[str, str]:
    case_id = case_spec["id"]
    receipt_path = output_root / case_entry["receipt"]["path"]
    receipt = _require_exact_keys(
        load_json(receipt_path),
        ENGINE_RECEIPT_FIELDS,
        f"current Rust {run_id} {case_id} receipt",
    )
    expected = {
        "schema": ENGINE_RECEIPT_SCHEMA,
        "run_id": run_id,
        "launcher_nonce": nonce,
        "engine": "rust",
        "producer": ENGINE_PRODUCERS["rust"],
        "case_id": case_id,
        "binary_sha256": binary_sha256,
        "source_tree": source_tree,
        "content_tree": case_spec["scenario"]["content_tree"],
        "profile": ENGINE_PROFILES["rust"],
        "config_sha256": config_sha256,
        "player_sha256": player_sha256,
        "network_references_sha256": network_references_sha256,
        "locale": case_spec["locale"],
        "seeds": case_spec["seeds"],
        "trigger": case_spec["trigger"],
        "scenario": case_spec["scenario"],
        "frame": case_spec["frame"],
        "runtime_resources": case_spec["runtime_resources"]["rust"],
    }
    for field, value in expected.items():
        _require(
            receipt[field] == value,
            f"current Rust {run_id} {case_id} receipt {field} mismatch",
        )
    expected_artifacts = {"png", "layout"} if case_id in LAYOUT_CASE_IDS else {"png"}
    artifacts = _require_exact_keys(
        receipt["artifacts"],
        expected_artifacts,
        f"current Rust {run_id} {case_id} artifacts",
    )
    hashes = {}
    png = _validate_v2_file_record(
        artifacts["png"],
        output_root,
        expected_path=f"{run_id}/rust/artifacts/{case_id}.png",
        label=f"current Rust {run_id} {case_id} PNG",
    )
    validate_png(png)
    hashes[f"{case_id}.png"] = artifacts["png"]["sha256"]
    if case_id in LAYOUT_CASE_IDS:
        layout = _validate_v2_file_record(
            artifacts["layout"],
            output_root,
            expected_path=f"{run_id}/rust/artifacts/{case_id}.layout.json",
            label=f"current Rust {run_id} {case_id} layout",
        )
        validate_layout_trace(
            layout,
            case_id,
            port_asset_exemptions=case_spec["port_asset_exemptions"],
        )
        hashes[f"{case_id}.layout.json"] = artifacts["layout"]["sha256"]
    return hashes


def _run_presentation_comparator(
    binary: Path,
    *,
    case_id: str,
    reference: Path,
    actual: Path,
) -> tuple[dict[str, Any], str]:
    _regular_file(binary, "Rust presentation comparator")
    _regular_file(reference, f"C++ {case_id} comparison reference")
    _regular_file(actual, f"Rust {case_id} comparison artifact")
    environment = {
        "CLONK_PRESENTATION_COMPARE": "1",
        "CLONK_PRESENTATION_CASE": case_id,
        "CLONK_PRESENTATION_REFERENCE": str(reference.resolve()),
        "CLONK_PRESENTATION_ACTUAL": str(actual.resolve()),
        **BUILD_ENVIRONMENT,
    }
    completed = _run(
        [str(binary)],
        cwd=binary.parent,
        text=True,
        environment=_capture_runtime_environment(environment),
    )
    try:
        result = json.loads(
            completed.stdout,
            object_pairs_hook=_reject_duplicate_keys,
        )
    except (TypeError, json.JSONDecodeError) as error:
        raise AcquisitionFailure(
            f"Rust comparator emitted invalid JSON for {case_id}: {error}"
        ) from error
    result = _require_exact_keys(
        result,
        {"schema", "case_id", "comparison", "status"},
        f"Rust {case_id} comparison receipt",
    )
    expected = {
        "schema": COMPARISON_RECEIPT_SCHEMA,
        "case_id": case_id,
        "comparison": "layout" if case_id in LAYOUT_CASE_IDS else "pixel",
        "status": "match",
    }
    _require(
        result == expected,
        f"Rust comparator did not prove a match for {case_id}",
    )
    compact = json.dumps(expected, separators=(",", ":"))
    _require(
        completed.stdout in {compact, compact + "\n"},
        f"Rust comparator stdout is not the exact compact receipt for {case_id}",
    )
    return result, completed.stdout


def _compare_current_artifact(
    binary: Path,
    *,
    case_id: str,
    reference: Path,
    actual: Path,
) -> dict[str, Any]:
    result, _stdout = _run_presentation_comparator(
        binary,
        case_id=case_id,
        reference=reference,
        actual=actual,
    )
    return result


def verify_current_rust(
    repository_root: Path,
    output_directory: Path,
    *,
    build_profile: str = "release",
) -> dict[str, Any]:
    """Capture the current Rust producer and compare it to accepted C++ evidence."""

    repository_root = repository_root.resolve()
    accepted_root = repository_root / "compat/presentation/oracle/v1"
    rust_commit, rust_tree = require_clean_source_revision(repository_root)
    source_inventory = rust_source_input_inventory(repository_root, rust_commit)
    verify_clean_source_checkpoint(
        repository_root,
        expected_commit=rust_commit,
        expected_tree=rust_tree,
        expected_inventory=source_inventory,
        label="before build",
    )
    output_directory = _require_fresh_external_output(
        output_directory,
        workspace=repository_root,
        oracle_checkout=repository_root,
    )
    case_specs = load_json_at_revision(
        repository_root,
        rust_commit,
        CASE_SPECS_SOURCE_PATH,
    )
    _require(isinstance(case_specs, list), "current case specs must be an array")
    manifest = load_json_at_revision(
        repository_root,
        rust_commit,
        CAPTURE_MANIFEST_SOURCE_PATH,
    )
    case_specs = _validate_v2_case_specs(
        case_specs,
        expected_case_specs=case_specs,
        trusted_manifest=manifest,
    )
    context = expected_provenance_fields(
        repository_root,
        rust_revision=rust_commit,
        workspace=repository_root,
        require_oracle_head=False,
    )
    for case_spec in case_specs:
        _require(
            case_spec["runtime_resources"] == context["runtime_resources"],
            f"current {case_spec['id']} runtime resource contract drift",
        )
    content_tree = context["fixture_content_tree"]

    output_directory.mkdir(parents=True)
    for relative in (
        "inputs/rust.config",
        PLAYER_RETAINED_PATH,
        NETWORK_REFERENCES_RETAINED_PATH,
    ):
        _copy_regular_file(
            accepted_root / relative,
            output_directory / relative,
            f"accepted live-verification input {relative}",
        )
    runtime_content_resources = expected_cpp_runtime_content_resources(
        repository_root / "content"
    )
    for _, (source_path, retained_path) in CPP_RUNTIME_CONTENT_GROUPS.items():
        materialize_runtime_resource_tree(
            repository_root / "content",
            FIXTURE_CONTENT_COMMIT,
            source_path,
            output_directory / retained_path,
        )
    _validate_staged_cpp_runtime_content(
        output_directory,
        runtime_content_resources,
    )
    source_identity_path = output_directory / RUST_SOURCE_IDENTITY_RETAINED_PATH
    _write_json_new(
        source_identity_path,
        {
            "schema": "clonk-rs/presentation-source-identity/v1",
            "commit": rust_commit,
            "tree": rust_tree,
            "content_tree": content_tree,
        },
    )

    recipe = rust_current_build_recipe(repository_root, build_profile)
    _execute_current_rust_build(
        recipe,
        repository_root,
        build_profile=build_profile,
    )
    verify_clean_source_checkpoint(
        repository_root,
        expected_commit=rust_commit,
        expected_tree=rust_tree,
        expected_inventory=source_inventory,
        label="after build",
    )
    target_profile = "release" if build_profile == "release" else "debug"
    built_binary = repository_root / f"target/{target_profile}" / (
        "clonk-app.exe" if os.name == "nt" else "clonk-app"
    )
    _regular_file(built_binary, "current Rust executable")
    verify_accepted_output(
        repository_root,
        output_directory=accepted_root,
        trusted_comparator_context=(
            built_binary,
            rust_commit,
            rust_tree,
            source_inventory,
        ),
    )
    runtime_binary = output_directory / "builds/rust/binary"
    _copy_regular_file(
        built_binary,
        runtime_binary,
        "current Rust executable",
    )
    try:
        runtime_binary.chmod(0o755)
    except OSError as error:
        raise AcquisitionFailure(
            f"could not make copied current Rust executable runnable: {error}"
        ) from error
    binary_sha256 = _sha256_file(runtime_binary)
    config_sha256 = _sha256_file(output_directory / "inputs/rust.config")
    player_sha256 = _sha256_file(output_directory / PLAYER_RETAINED_PATH)
    network_sha256 = _sha256_file(
        output_directory / NETWORK_REFERENCES_RETAINED_PATH
    )
    rust_runtime_root = output_directory / "work/rust-source"
    materialize_runtime_resource_tree(
        repository_root,
        rust_commit,
        "planet",
        rust_runtime_root / "planet",
    )
    materialize_runtime_resource_tree(
        repository_root / "content",
        FIXTURE_CONTENT_COMMIT,
        None,
        rust_runtime_root / "content",
    )
    for group, (source_path, _) in RUNTIME_RESOURCE_GROUPS.items():
        observed = runtime_resource_identity(
            rust_runtime_root / source_path
        )
        _require(
            observed == context["runtime_resources"]["rust"][group],
            f"current Rust {group} runtime resource staging drift",
        )
    runtime_content_identity = runtime_resource_identity(
        rust_runtime_root / "content"
    )
    _require(
        runtime_content_identity["tree"] == content_tree,
        "current Rust runtime content tree differs from pinned fixture content",
    )
    _install_rust_player_discovery_fixture(output_directory, rust_runtime_root)

    run_summaries = []
    duplicate_hashes = {}
    nonces: set[str] = set()
    for run_id in RUN_IDS:
        nonce = secrets.token_hex(32)
        _require(nonce not in nonces, "secure live-verification nonce repeated")
        nonces.add(nonce)
        _ensure_capture_directories(output_directory, run_id, "rust")
        entries = [
            _run_capture_case(
                output_directory,
                {"rust": runtime_binary},
                run_id=run_id,
                nonce=nonce,
                engine="rust",
                case_id=case_id,
                runtime_resources=context["runtime_resources"],
                runtime_content_resources=runtime_content_resources,
                rust_source_root=rust_runtime_root,
            )
            for case_id in CASE_IDS
        ]
        hashes = {}
        for entry, case_spec in zip(entries, case_specs, strict=True):
            hashes.update(
                _validate_current_rust_case(
                    output_directory,
                    entry,
                    run_id=run_id,
                    nonce=nonce,
                    case_spec=case_spec,
                    source_tree=rust_tree,
                    binary_sha256=binary_sha256,
                    config_sha256=config_sha256,
                    player_sha256=player_sha256,
                    network_references_sha256=network_sha256,
                )
            )
        duplicate_hashes[run_id] = hashes
        run_summaries.append({"id": run_id, "nonce": nonce, "cases": entries})
    _require(
        duplicate_hashes[RUN_IDS[0]] == duplicate_hashes[RUN_IDS[1]],
        "current Rust duplicate capture runs differ",
    )

    comparisons = []
    for run_id in RUN_IDS:
        for case_id in CASE_IDS:
            suffix = "layout.json" if case_id in LAYOUT_CASE_IDS else "png"
            comparisons.append(
                {
                    "run_id": run_id,
                    **_compare_current_artifact(
                        runtime_binary,
                        case_id=case_id,
                        reference=(
                            accepted_root
                            / f"run-1/cpp/artifacts/{case_id}.{suffix}"
                        ),
                        actual=(
                            output_directory
                            / f"{run_id}/rust/artifacts/{case_id}.{suffix}"
                        ),
                    ),
                }
            )

    verify_clean_source_checkpoint(
        repository_root,
        expected_commit=rust_commit,
        expected_tree=rust_tree,
        expected_inventory=source_inventory,
        label="after capture",
    )

    try:
        _remove_rust_player_discovery_fixture(
            output_directory,
            rust_runtime_root,
        )
        _remove_runtime_content_resources(
            output_directory,
            runtime_content_resources,
        )
        source_identity_path.unlink()
        shutil.rmtree(output_directory / "work")
    except OSError as error:
        raise AcquisitionFailure(
            f"could not remove live-verification source identity: {error}"
        ) from error
    summary = {
        "schema": "clonk-rs/presentation-current-verification/v1",
        "rust_source": {"commit": rust_commit, "tree": rust_tree},
        "rust_source_inventory_sha256": source_inventory["sha256"],
        "content_tree": content_tree,
        "runtime_content_manifest_sha256": runtime_content_identity[
            "manifest_sha256"
        ],
        "build": {
            "recipe": recipe,
            "binary": _file_record(output_directory, "builds/rust/binary"),
        },
        "runs": run_summaries,
        "comparisons": comparisons,
    }
    _write_json_new(output_directory / "verify-current.json", summary)
    expected_files = {
        "verify-current.json",
        "builds/rust/binary",
        "inputs/rust.config",
        PLAYER_RETAINED_PATH,
        NETWORK_REFERENCES_RETAINED_PATH,
    }
    for run_id in RUN_IDS:
        for case_id in CASE_IDS:
            expected_files.add(f"{run_id}/rust/receipts/{case_id}.json")
            expected_files.add(f"{run_id}/rust/artifacts/{case_id}.png")
            if case_id in LAYOUT_CASE_IDS:
                expected_files.add(
                    f"{run_id}/rust/artifacts/{case_id}.layout.json"
                )
    observed_files = set()
    for path in output_directory.rglob("*"):
        _require(not path.is_symlink(), f"live-verification output contains a symlink: {path}")
        if path.is_file():
            observed_files.add(path.relative_to(output_directory).as_posix())
        else:
            _require(path.is_dir(), f"live-verification output has a special file: {path}")
    _require(
        observed_files == expected_files,
        "live-verification file inventory drift: "
        f"missing={sorted(expected_files - observed_files)} "
        f"unexpected={sorted(observed_files - expected_files)}",
    )
    return {
        "rust_source_commit": rust_commit,
        "verified_cases": len(comparisons),
        "output": str(output_directory),
    }


def expected_provenance_fields(
    oracle_repository: Path,
    *,
    rust_revision: str = "HEAD",
    workspace: Path = WORKSPACE,
    require_oracle_head: bool = True,
) -> dict[str, str]:
    """Compute the exact source/content context a capture records.

    ``rust_revision`` is explicit for acquisition-time validation and records
    the informative commit/tree identity carried by receipts. Durable accepted
    verification uses the retained squash-stable blob inventory instead,
    because branch commits do not survive this repository's squash merge.
    """

    if require_oracle_head:
        verify_checkout_head(oracle_repository)
    else:
        resolved_oracle = _git_text(
            oracle_repository,
            ["rev-parse", "--verify", f"{ORACLE_SOURCE_COMMIT}^{{commit}}"],
        )
        _require(
            resolved_oracle == ORACLE_SOURCE_COMMIT,
            "pinned oracle source commit does not resolve",
        )
    rust_commit = _git_text(
        workspace,
        ["rev-parse", "--verify", f"{rust_revision}^{{commit}}"],
    )
    profile = load_json_at_revision(
        workspace,
        rust_commit,
        "compat/profile.json",
    )
    try:
        fixture_pin = profile["pinned"]["content_commit"]
    except (KeyError, TypeError) as error:
        raise AcquisitionFailure("recorded profile content pin is missing") from error
    _require(
        fixture_pin == FIXTURE_CONTENT_COMMIT,
        f"recorded profile content pin is {fixture_pin!r}, expected {FIXTURE_CONTENT_COMMIT}",
    )
    _require(
        read_gitlink(workspace, rust_commit, "content") == fixture_pin,
        f"recorded Rust source {rust_commit} used a different content gitlink",
    )
    historical_gitlink = read_gitlink(
        oracle_repository,
        ORACLE_SOURCE_COMMIT,
        "content",
    )
    _require(
        historical_gitlink == ORACLE_SOURCE_CONTENT_GITLINK,
        "oracle historical content gitlink drift",
    )
    withheld = withheld_system_script_paths(profile)
    oracle_scripts = effective_system_script_manifest(
        oracle_repository,
        ORACLE_SOURCE_COMMIT,
        excluded_paths=(),
    )
    rust_scripts = effective_system_script_manifest(
        workspace,
        rust_commit,
        excluded_paths=withheld,
    )
    _require(
        oracle_scripts == rust_scripts,
        "effective Rust System.c4g script manifest differs from the oracle",
    )
    return {
        "oracle_source_tree": tree_oid(oracle_repository, ORACLE_SOURCE_COMMIT),
        "rust_source_commit": rust_commit,
        "rust_source_tree": tree_oid(workspace, rust_commit),
        "fixture_content_tree": tree_oid(workspace / "content", fixture_pin),
        "oracle_planet_tree": tree_oid(
            oracle_repository, ORACLE_SOURCE_COMMIT, "planet"
        ),
        "rust_planet_tree": tree_oid(workspace, rust_commit, "planet"),
        "oracle_system_tree": tree_oid(
            oracle_repository, ORACLE_SOURCE_COMMIT, "planet/System.c4g"
        ),
        "rust_system_tree": tree_oid(workspace, rust_commit, "planet/System.c4g"),
        "effective_system_scripts_sha256": oracle_scripts["sha256"],
        "runtime_resources": expected_runtime_resources(
            oracle_repository,
            workspace,
            rust_commit,
        ),
    }


def _recipe(commands: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    normalized = [
        {
            "argv": list(command["argv"]),
            "cwd": str(command["cwd"]),
            "environment": dict(command["environment"]),
        }
        for command in commands
    ]
    return {
        "commands": normalized,
        "sha256": _canonical_json_sha256({"commands": normalized}),
    }


def cpp_build_recipe() -> dict[str, Any]:
    configure = [
        "cmake",
        "-S",
        ".",
        "-B",
        "build",
        "-G",
        "Ninja",
        "-DCMAKE_BUILD_TYPE=Debug",
        "-DUSE_TESTS=OFF",
        "-DUSE_LTO=OFF",
        "-DUSE_PCH=OFF",
        "-DUSE_RUST_ENGINE_VALIDATION=OFF",
        "-DUSE_RUST_CONFIG=OFF",
        "-DUSE_RUST_GROUP_VALIDATION=OFF",
        "-DUSE_RUST_GUI_VALIDATION=OFF",
        "-DUSE_MINIUPNPC=OFF",
        "-DCMAKE_OSX_DEPLOYMENT_TARGET=13.3",
        "-Dfmt_DIR=/opt/homebrew/Cellar/fmt/11.1.2/lib/cmake/fmt",
        "-DOPENSSL_ROOT_DIR=/opt/homebrew/opt/openssl@3",
        "-DOPENSSL_INCLUDE_DIR=/opt/homebrew/opt/openssl@3/include",
    ]
    return _recipe(
        [
            {
                "argv": configure,
                "cwd": "work/oracle-source",
                "environment": BUILD_ENVIRONMENT,
            },
            {
                "argv": ["cmake", "--build", "build", "--target", "clonk", "-j", "4"],
                "cwd": "work/oracle-source",
                "environment": BUILD_ENVIRONMENT,
            },
        ]
    )


def rust_build_recipe() -> dict[str, Any]:
    return _recipe(
        [
            {
                "argv": [
                    "cargo",
                    "build",
                    "--locked",
                    "--release",
                    "-p",
                    "clonk-app",
                    "--features",
                    "presentation-capture",
                ],
                "cwd": "work/rust-source",
                "environment": BUILD_ENVIRONMENT,
            }
        ]
    )


def rust_current_build_recipe(
    repository_root: Path,
    build_profile: str = "release",
) -> dict[str, Any]:
    _require(
        build_profile in {"release", "test"},
        f"unsupported current Rust build profile: {build_profile}",
    )
    profile_arguments = (
        ["--release"]
        if build_profile == "release"
        else ["--profile", "test"]
    )
    return _recipe(
        [
            {
                "argv": [
                    "cargo",
                    "build",
                    "--locked",
                    *profile_arguments,
                    "-p",
                    "clonk-app",
                    "--features",
                    "presentation-capture",
                ],
                "cwd": str(repository_root),
                "environment": BUILD_ENVIRONMENT,
            }
        ]
    )


def _execute_current_rust_build(
    recipe: Mapping[str, Any],
    repository_root: Path,
    *,
    build_profile: str = "release",
) -> None:
    commands = recipe.get("commands")
    _require(isinstance(commands, list) and len(commands) == 1, "current build recipe drift")
    command = commands[0]
    expected = rust_current_build_recipe(repository_root, build_profile)["commands"][0]
    _require(
        command == expected,
        "current build recipe is not the audited clean-tree command",
    )
    _run(
        command["argv"],
        cwd=repository_root,
        environment=_build_process_environment(command["environment"]),
        capture_output=False,
    )


def _build_trusted_current_comparator(
    repository_root: Path,
) -> tuple[Path, str, str, dict[str, Any]]:
    commit, tree = require_clean_source_revision(repository_root)
    inventory = rust_source_input_inventory(repository_root, commit)
    verify_clean_source_checkpoint(
        repository_root,
        expected_commit=commit,
        expected_tree=tree,
        expected_inventory=inventory,
        label="before trusted comparator build",
    )
    _execute_current_rust_build(
        rust_current_build_recipe(repository_root),
        repository_root,
        build_profile="release",
    )
    verify_clean_source_checkpoint(
        repository_root,
        expected_commit=commit,
        expected_tree=tree,
        expected_inventory=inventory,
        label="after trusted comparator build",
    )
    binary = repository_root / "target/release" / (
        "clonk-app.exe" if os.name == "nt" else "clonk-app"
    )
    _regular_file(binary, "current trusted Rust presentation comparator")
    return binary, commit, tree, inventory


def _select_trusted_current_comparator(
    repository_root: Path,
    prebuilt: tuple[Path, str, str, dict[str, Any]] | None = None,
) -> tuple[Path, str, str, dict[str, Any]]:
    if prebuilt is None:
        return _build_trusted_current_comparator(repository_root)
    binary, commit, tree, inventory = prebuilt
    _regular_file(binary, "prebuilt current Rust presentation comparator")
    _require(_is_lower_hex(commit, 40), "prebuilt comparator commit is invalid")
    _require(_is_lower_hex(tree, 40), "prebuilt comparator tree is invalid")
    _require(isinstance(inventory, dict), "prebuilt comparator inventory is invalid")
    return binary, commit, tree, inventory


def _build_process_environment(overrides: Mapping[str, str]) -> dict[str, str]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if key in BUILD_HOST_ENV_ALLOWLIST
    }
    environment.update(overrides)
    return environment


def _capture_runtime_environment(overrides: Mapping[str, str]) -> dict[str, str]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if key in RUNTIME_HOST_ENV_ALLOWLIST
    }
    environment.update(overrides)
    return environment


def _execute_recipe(recipe: Mapping[str, Any], candidate_root: Path) -> None:
    for command in recipe["commands"]:
        relative_cwd = Path(command["cwd"])
        _require(
            not relative_cwd.is_absolute() and ".." not in relative_cwd.parts,
            f"build recipe has an unsafe cwd: {relative_cwd}",
        )
        cwd = candidate_root / relative_cwd
        _require(cwd.is_dir() and not cwd.is_symlink(), f"build cwd is invalid: {cwd}")
        _run(
            command["argv"],
            cwd=cwd,
            environment=_build_process_environment(command["environment"]),
            capture_output=False,
        )


def _require_fresh_external_output(
    output_directory: Path,
    *,
    workspace: Path,
    oracle_checkout: Path,
) -> Path:
    _require(output_directory.is_absolute(), "acquisition output directory must be absolute")
    _require(not output_directory.exists(), f"acquisition output already exists: {output_directory}")
    _require(
        output_directory == output_directory.resolve(strict=False),
        "acquisition output directory must be canonical and contain no symlinked parent",
    )
    _require(len(output_directory.parts) > 2, "acquisition output directory is too broad")
    _require_destination_outside_repository(workspace, output_directory)
    _require_destination_outside_repository(oracle_checkout, output_directory)
    return output_directory


def _copy_fixture_content(fixture_source: Path, destination: Path) -> None:
    _require(fixture_source.is_dir() and not fixture_source.is_symlink(), "fixture archive is invalid")
    if destination.exists() or destination.is_symlink():
        _require(
            destination.is_dir()
            and not destination.is_symlink()
            and next(destination.iterdir(), None) is None,
            f"fixture content destination already exists: {destination}",
        )
        # `git archive` materializes a submodule gitlink as an empty directory.
        # Remove only that validated placeholder before installing the pinned
        # fixture tree.
        destination.rmdir()
    for path in fixture_source.rglob("*"):
        _require(
            not path.is_symlink() and (path.is_dir() or path.is_file()),
            f"fixture content contains a symlink or special entry: {path}",
        )
    try:
        shutil.copytree(
            fixture_source,
            destination,
            symlinks=False,
            copy_function=os.link,
        )
    except OSError as error:
        raise AcquisitionFailure(f"could not install fixture content at {destination}: {error}") from error


def _stage_acquisition(
    oracle_checkout: Path,
    output_directory: Path,
    *,
    workspace: Path,
    rust_commit: str,
    source_inventory: Mapping[str, Any],
    runtime_resources: Mapping[str, Any],
) -> dict[str, Any]:
    work = output_directory / "work"
    oracle_source = work / "oracle-source"
    rust_source = work / "rust-source"
    fixture_content = work / "fixture-content"
    output_directory.mkdir(parents=True)
    try:
        materialize_git_archive(oracle_checkout, ORACLE_SOURCE_COMMIT, oracle_source)
        materialize_git_archive(workspace, rust_commit, rust_source)
        for _, (source_path, _) in RUNTIME_RESOURCE_GROUPS.items():
            archived_resource = rust_source / source_path
            _require(
                archived_resource.is_dir() and not archived_resource.is_symlink(),
                f"archived Rust runtime resource is invalid: {archived_resource}",
            )
            shutil.rmtree(archived_resource)
            materialize_runtime_resource_tree(
                workspace,
                rust_commit,
                source_path,
                archived_resource,
            )
        _validate_staged_rust_runtime_resources(
            rust_source,
            runtime_resources["rust"],
        )
        materialize_runtime_resource_tree(
            workspace / "content",
            FIXTURE_CONTENT_COMMIT,
            None,
            fixture_content,
        )
        _copy_fixture_content(fixture_content, rust_source / "content")

        retained_sources = {
            rust_source / CPP_CONFIG_SOURCE_PATH: output_directory / "inputs/cpp.config",
            rust_source / RUST_CONFIG_SOURCE_PATH: output_directory / "inputs/rust.config",
            rust_source / PLAYER_SOURCE_PATH: output_directory / PLAYER_RETAINED_PATH,
            rust_source / NETWORK_REFERENCES_SOURCE_PATH: (
                output_directory / NETWORK_REFERENCES_RETAINED_PATH
            ),
            rust_source / CAPTURE_PATCH_SOURCE_PATH: (
                output_directory / CAPTURE_PATCH_RETAINED_PATH
            ),
            rust_source / LAUNCHER_SOURCE_PATH: output_directory / LAUNCHER_RETAINED_PATH,
        }
        for source, destination in retained_sources.items():
            _copy_regular_file(source, destination, source.relative_to(rust_source).as_posix())
        for _, (source_path, retained_path) in RUNTIME_RESOURCE_GROUPS.items():
            materialize_runtime_resource_tree(
                oracle_checkout,
                ORACLE_SOURCE_COMMIT,
                source_path,
                output_directory / retained_path,
            )
        runtime_content_resources = expected_cpp_runtime_content_resources(
            workspace / "content"
        )
        runtime_scenario_resources = expected_cpp_runtime_scenario_resources(
            workspace / "content"
        )
        for _, (source_path, retained_path) in CPP_RUNTIME_CONTENT_GROUPS.items():
            materialize_runtime_resource_tree(
                workspace / "content",
                FIXTURE_CONTENT_COMMIT,
                source_path,
                output_directory / retained_path,
            )
        _validate_staged_cpp_runtime_resources(
            output_directory,
            runtime_resources["cpp"],
        )
        _validate_staged_cpp_runtime_content(
            output_directory,
            runtime_content_resources,
        )
        _write_json_new(
            output_directory / RUST_SOURCE_INVENTORY_RETAINED_PATH,
            source_inventory,
        )

        retained_patch = output_directory / CAPTURE_PATCH_RETAINED_PATH
        _run(
            ["git", "apply", "--check", "--whitespace=nowarn", str(retained_patch)],
            cwd=oracle_source,
        )
        _run(
            ["git", "apply", "--whitespace=nowarn", str(retained_patch)],
            cwd=oracle_source,
        )
    except Exception:
        # The explicit candidate remains for diagnostics. It is incomplete and
        # cannot validate or be accepted because it has no provenance index.
        raise
    return {
        "work": work,
        "oracle_source": oracle_source,
        "rust_source": rust_source,
        "fixture_content": fixture_content,
        "cpp_runtime_content_resources": runtime_content_resources,
        "cpp_runtime_scenario_resources": runtime_scenario_resources,
    }


def _build_acquisition_binaries(
    candidate_root: Path,
    staged: Mapping[str, Path],
    *,
    context: Mapping[str, str],
    patch_sha256: str,
) -> tuple[dict[str, Any], dict[str, Path]]:
    _require(
        sys.platform == "darwin",
        "the audited C++ application-bundle build recipe is currently macOS-only",
    )
    recipes = {"cpp": cpp_build_recipe(), "rust": rust_build_recipe()}
    for recipe in recipes.values():
        _execute_recipe(recipe, candidate_root)
    runtime_binaries = {
        "cpp": staged["oracle_source"] / "build/clonk.app/Contents/MacOS/clonk",
        "rust": staged["rust_source"] / "target/release/clonk-app",
    }
    for engine, binary in runtime_binaries.items():
        _regular_file(binary, f"built {engine} executable")
        _copy_regular_file(
            binary,
            candidate_root / f"builds/{engine}/binary",
            f"built {engine} executable",
        )
    builds = {}
    for engine in ENGINE_IDS:
        builds[engine] = {
            "source_tree": context[
                "oracle_source_tree" if engine == "cpp" else "rust_source_tree"
            ],
            "capture_patch_sha256": patch_sha256 if engine == "cpp" else None,
            "producer": ENGINE_PRODUCERS[engine],
            "profile": ENGINE_PROFILES[engine],
            "recipe": recipes[engine],
            "binary": _file_record(candidate_root, f"builds/{engine}/binary"),
        }
    return builds, runtime_binaries


def _acquisition_trusted_comparator(candidate_root: Path) -> Path:
    binary = candidate_root / "builds/rust/binary"
    _regular_file(binary, "retained acquisition Rust comparator")
    try:
        binary.chmod(binary.stat().st_mode | stat.S_IXUSR)
    except OSError as error:
        raise AcquisitionFailure(
            f"could not make retained acquisition comparator runnable: {error}"
        ) from error
    _regular_file(binary, "runnable retained acquisition Rust comparator")
    return binary


def _capture_environment(
    candidate_root: Path,
    *,
    run_id: str,
    nonce: str,
    engine: str,
    case_id: str,
) -> dict[str, str]:
    environment = {
        "CLONK_PRESENTATION_CAPTURE": "1",
        "CLONK_PRESENTATION_RUN_ID": run_id,
        "CLONK_PRESENTATION_NONCE": nonce,
        "CLONK_PRESENTATION_CASE": case_id,
        "CLONK_PRESENTATION_OUTPUT_DIR": str(
            candidate_root / run_id / engine / "artifacts"
        ),
        "CLONK_PRESENTATION_RECEIPT": str(
            candidate_root / run_id / engine / "receipts" / f"{case_id}.json"
        ),
        "LC_USER_DATA_DIR": str(
            candidate_root / "work/user-data" / run_id / engine / case_id
        ),
        "LC_CONTENT_DIR": str(candidate_root / "inputs"),
        "LC_LANGUAGE": "US",
        # Stabilize C++ sound-variant SafeRandom; Rust ignores SDL audio.
        "SDL_AUDIODRIVER": "dummy",
        **BUILD_ENVIRONMENT,
    }
    if engine == "rust":
        environment["CLONK_PRESENTATION_SOURCE_IDENTITY"] = str(
            candidate_root / RUST_SOURCE_IDENTITY_RETAINED_PATH
        )
    return environment


def _capture_command(
    candidate_root: Path,
    runtime_binaries: Mapping[str, Path],
    *,
    engine: str,
    case_id: str,
    runtime_resources: Any | None = None,
    runtime_content_resources: Any | None = None,
    scenario_resources: Any | None = None,
    rust_source_root: Path | None = None,
) -> tuple[list[str], Path]:
    if engine == "cpp":
        _require(runtime_resources is not None, "C++ runtime resource identity is missing")
        _validate_staged_cpp_runtime_resources(candidate_root, runtime_resources)
        startup = CPP_STARTUP_ARGUMENTS.get(case_id)
        if startup is not None:
            return (
                [
                    str(runtime_binaries[engine]),
                    startup,
                    f"/config:{candidate_root / 'inputs/cpp.config'}",
                ],
                candidate_root / "inputs",
            )
        scenario_relative = CPP_RUNTIME_SCENARIOS.get(case_id)
        _require(
            scenario_relative is not None,
            f"C++ launch argv for {case_id} is not implemented or audited",
        )
        _require(
            runtime_content_resources is not None,
            "C++ runtime content identity is missing",
        )
        _validate_staged_cpp_runtime_content(
            candidate_root,
            runtime_content_resources,
        )
        _require(
            scenario_resources is not None,
            "C++ runtime scenario identity is missing",
        )
        _validate_staged_cpp_runtime_scenario(
            candidate_root,
            case_id,
            scenario_resources,
        )
        scenario = candidate_root / "work/fixture-content" / scenario_relative
        network_arguments = ["/network", "/lobby"] if case_id == "network-lobby" else []
        return (
            [
                str(runtime_binaries[engine]),
                str(scenario),
                *network_arguments,
                f"/config:{candidate_root / 'inputs/cpp.config'}",
            ],
            candidate_root / "inputs",
        )
    _require(engine == "rust", f"unsupported capture engine: {engine}")
    source_root = rust_source_root or candidate_root / "work/rust-source"
    _require(runtime_resources is not None, "Rust runtime resource identity is missing")
    _validate_staged_rust_runtime_resources(source_root, runtime_resources)
    _require(
        runtime_content_resources is not None,
        "Rust capture content identity is missing",
    )
    _validate_staged_cpp_runtime_content(
        candidate_root,
        runtime_content_resources,
    )
    return (
        [
            str(runtime_binaries[engine]),
            "--config",
            str(candidate_root / "inputs/rust.config"),
            str(candidate_root / PLAYER_RETAINED_PATH),
            "/compatprofile:legacy-clonk",
        ],
        source_root,
    )


def _ensure_capture_directories(candidate_root: Path, run_id: str, engine: str) -> None:
    for leaf in ("artifacts", "receipts"):
        directory = candidate_root / run_id / engine / leaf
        _require(not directory.exists(), f"capture directory is not fresh: {directory}")
        directory.mkdir(parents=True)


def _prepare_capture_user_data_directory(directory: Path) -> None:
    _require(
        not directory.exists() and not directory.is_symlink(),
        f"capture user-data directory is not fresh: {directory}",
    )
    try:
        directory.mkdir(parents=True)
    except OSError as error:
        raise AcquisitionFailure(
            f"could not create capture user-data directory {directory}: {error}"
        ) from error
    _require(
        directory.is_dir()
        and not directory.is_symlink()
        and not any(directory.iterdir()),
        f"capture user-data directory is not fresh: {directory}",
    )


def _install_rust_player_discovery_fixture(
    candidate_root: Path,
    rust_source_root: Path,
) -> Path:
    _require(
        rust_source_root.is_dir() and not rust_source_root.is_symlink(),
        f"Rust source root is invalid: {rust_source_root}",
    )
    source = candidate_root / PLAYER_RETAINED_PATH
    _regular_file(source, "retained presentation player")
    _require(
        _sha256_file(source) == PLAYER_SHA256,
        "retained presentation player differs from the audited bytes",
    )
    destination = rust_source_root / "Presentation.c4p"
    _require(
        destination.parent == rust_source_root
        and destination.name == "Presentation.c4p",
        f"unsafe Rust player discovery fixture target: {destination}",
    )
    _copy_regular_file(source, destination, "Rust player discovery fixture")
    return destination


def _remove_rust_player_discovery_fixture(
    candidate_root: Path,
    rust_source_root: Path,
) -> None:
    destination = rust_source_root / "Presentation.c4p"
    _require(
        destination.parent == rust_source_root
        and destination.name == "Presentation.c4p",
        f"unsafe Rust player discovery fixture cleanup target: {destination}",
    )
    _regular_file(destination, "Rust player discovery fixture")
    _require(
        _sha256_file(destination)
        == _sha256_file(candidate_root / PLAYER_RETAINED_PATH)
        == PLAYER_SHA256,
        "Rust player discovery fixture changed during capture",
    )
    try:
        destination.unlink()
    except OSError as error:
        raise AcquisitionFailure(
            f"could not remove Rust player discovery fixture {destination}: {error}"
        ) from error


def _require_fresh_cpp_network_cache(candidate_root: Path) -> None:
    network = candidate_root / "inputs/Network"
    _require(
        not network.exists() and not network.is_symlink(),
        f"C++ Network acquisition directory is not fresh: {network}",
    )


def _remove_cpp_network_cache(candidate_root: Path) -> None:
    network = candidate_root / "inputs/Network"
    _require(
        network.parent == candidate_root / "inputs" and network.name == "Network",
        f"unsafe C++ Network cleanup target: {network}",
    )
    if not network.exists() and not network.is_symlink():
        return
    _require(
        network.is_dir() and not network.is_symlink(),
        f"C++ Network acquisition path is a symlink or special entry: {network}",
    )
    for entry in network.rglob("*"):
        try:
            entry_stat = entry.lstat()
        except OSError as error:
            raise AcquisitionFailure(
                f"could not inspect C++ Network acquisition entry {entry}: {error}"
            ) from error
        _require(
            stat.S_ISDIR(entry_stat.st_mode) or stat.S_ISREG(entry_stat.st_mode),
            f"C++ Network acquisition path contains a symlink or special entry: {entry}",
        )
    try:
        shutil.rmtree(network)
    except OSError as error:
        raise AcquisitionFailure(
            f"could not remove acquisition-only C++ Network directory {network}: {error}"
        ) from error


def _run_capture_case(
    candidate_root: Path,
    runtime_binaries: Mapping[str, Path],
    *,
    run_id: str,
    nonce: str,
    engine: str,
    case_id: str,
    runtime_resources: Mapping[str, Any],
    runtime_content_resources: Any | None = None,
    scenario_resources: Any | None = None,
    rust_source_root: Path | None = None,
) -> dict[str, Any]:
    selected_resources = _validate_engine_runtime_resources(
        runtime_resources[engine],
        f"{engine} {case_id} runtime resources",
    )
    command, cwd = _capture_command(
        candidate_root,
        runtime_binaries,
        engine=engine,
        case_id=case_id,
        runtime_resources=selected_resources,
        runtime_content_resources=runtime_content_resources,
        scenario_resources=scenario_resources,
        rust_source_root=rust_source_root,
    )
    controlled_environment = _capture_environment(
        candidate_root,
        run_id=run_id,
        nonce=nonce,
        engine=engine,
        case_id=case_id,
    )
    _prepare_capture_user_data_directory(
        Path(controlled_environment["LC_USER_DATA_DIR"])
    )
    receipt_relative = f"{run_id}/{engine}/receipts/{case_id}.json"
    receipt_path = candidate_root / receipt_relative
    expected_outputs = [candidate_root / f"{run_id}/{engine}/artifacts/{case_id}.png"]
    if case_id in LAYOUT_CASE_IDS:
        expected_outputs.append(
            candidate_root / f"{run_id}/{engine}/artifacts/{case_id}.layout.json"
        )
    for expected in [receipt_path, *expected_outputs]:
        _require(not expected.exists(), f"capture output is not fresh: {expected}")
    cpp_launch = engine == "cpp"
    cpp_network_lobby = cpp_launch and case_id == "network-lobby"
    if cpp_launch:
        _require_fresh_cpp_network_cache(candidate_root)
    try:
        _run(
            command,
            cwd=cwd,
            environment=_capture_runtime_environment(controlled_environment),
            timeout_seconds=CAPTURE_COMMAND_TIMEOUT_SECONDS,
        )
    finally:
        if cpp_network_lobby:
            _remove_cpp_network_cache(candidate_root)
    _regular_file(receipt_path, f"{run_id} {engine} {case_id} engine receipt")
    receipt = _require_exact_keys(
        load_json(receipt_path),
        ENGINE_RECEIPT_FIELDS,
        f"{run_id} {engine} {case_id} engine receipt",
    )
    _require(
        receipt["run_id"] == run_id
        and receipt["launcher_nonce"] == nonce
        and receipt["engine"] == engine
        and receipt["case_id"] == case_id,
        f"{run_id} {engine} {case_id} engine receipt identity mismatch",
    )
    return {
        "id": case_id,
        "receipt": _file_record(candidate_root, receipt_relative),
        "launch": {
            "argv": command,
            "cwd": str(cwd),
            "environment": controlled_environment,
            "runtime_resources": copy.deepcopy(selected_resources),
        },
    }


def _write_launcher_receipt(
    candidate_root: Path,
    *,
    run_id: str,
    nonce: str,
    engine_runs: Mapping[str, Any],
    launcher_sha256: str,
    case_specs_sha256: str,
) -> dict[str, Any]:
    engine_receipts = {
        engine: [
            {"case_id": case["id"], "sha256": case["receipt"]["sha256"]}
            for case in engine_runs[engine]["cases"]
        ]
        for engine in ENGINE_IDS
    }
    launches = [
        {
            "engine": engine,
            "case_id": case["id"],
            **case["launch"],
        }
        for engine in ENGINE_IDS
        for case in engine_runs[engine]["cases"]
    ]
    receipt_value = {
        "schema": LAUNCHER_RECEIPT_SCHEMA,
        "run_id": run_id,
        "nonce": nonce,
        "launcher_sha256": launcher_sha256,
        "case_specs_sha256": case_specs_sha256,
        "engine_receipts": engine_receipts,
        "launches_sha256": _canonical_json_sha256(launches),
    }
    relative = f"{run_id}/launcher-receipt.json"
    _write_json_new(candidate_root / relative, receipt_value)
    return _file_record(candidate_root, relative)


def _run_acquisition_captures(
    candidate_root: Path,
    runtime_binaries: Mapping[str, Path],
    *,
    launcher_sha256: str,
    case_specs_sha256: str,
    runtime_resources: Mapping[str, Any],
    runtime_content_resources: Mapping[str, Any],
    scenario_resources: Mapping[str, Any],
    rust_source_root: Path,
) -> list[dict[str, Any]]:
    player_copy = runtime_binaries["cpp"].parent / "Presentation.c4p"
    _copy_regular_file(
        candidate_root / PLAYER_RETAINED_PATH,
        player_copy,
        "executable-adjacent player-selection fixture",
    )
    _install_rust_player_discovery_fixture(candidate_root, rust_source_root)
    runs = []
    try:
        nonces: set[str] = set()
        for run_id in RUN_IDS:
            nonce = secrets.token_hex(32)
            _require(nonce not in nonces, "secure launcher nonce unexpectedly repeated")
            nonces.add(nonce)
            engine_runs = {}
            for engine in ENGINE_IDS:
                _ensure_capture_directories(candidate_root, run_id, engine)
                cases = [
                    _run_capture_case(
                        candidate_root,
                        runtime_binaries,
                        run_id=run_id,
                        nonce=nonce,
                        engine=engine,
                        case_id=case_id,
                        runtime_resources=runtime_resources,
                        runtime_content_resources=runtime_content_resources,
                        scenario_resources=scenario_resources,
                    )
                    for case_id in CASE_IDS
                ]
                engine_runs[engine] = {"cases": cases}
            launcher_receipt = _write_launcher_receipt(
                candidate_root,
                run_id=run_id,
                nonce=nonce,
                engine_runs=engine_runs,
                launcher_sha256=launcher_sha256,
                case_specs_sha256=case_specs_sha256,
            )
            runs.append(
                {
                    "id": run_id,
                    "nonce": nonce,
                    "launcher_receipt": launcher_receipt,
                    "engines": engine_runs,
                }
            )
    finally:
        try:
            player_copy.unlink(missing_ok=True)
        except OSError as error:
            raise AcquisitionFailure(
                f"could not remove executable-adjacent player fixture: {error}"
            ) from error
        _remove_rust_player_discovery_fixture(candidate_root, rust_source_root)
    return runs


def _run_acquisition_comparisons(
    candidate_root: Path,
    rust_binary: Path,
) -> list[dict[str, Any]]:
    comparisons = []
    for run_id in RUN_IDS:
        cases = []
        for case_id in CASE_IDS:
            suffix = "layout.json" if case_id in LAYOUT_CASE_IDS else "png"
            reference_relative = f"{run_id}/cpp/artifacts/{case_id}.{suffix}"
            actual_relative = f"{run_id}/rust/artifacts/{case_id}.{suffix}"
            receipt_relative = f"{run_id}/comparisons/{case_id}.json"
            reference = _file_record(candidate_root, reference_relative)
            actual = _file_record(candidate_root, actual_relative)
            _result, stdout = _run_presentation_comparator(
                rust_binary,
                case_id=case_id,
                reference=candidate_root / reference_relative,
                actual=candidate_root / actual_relative,
            )
            comparison = "layout" if case_id in LAYOUT_CASE_IDS else "pixel"
            _write_json_new(
                candidate_root / receipt_relative,
                {
                    "schema": COMPARISON_ATTESTATION_SCHEMA,
                    "run_id": run_id,
                    "case_id": case_id,
                    "comparison": comparison,
                    "comparator": {
                        "schema": COMPARISON_RECEIPT_SCHEMA,
                        "binary_sha256": _sha256_file(rust_binary),
                    },
                    "reference": {
                        "path": reference["path"],
                        "sha256": reference["sha256"],
                    },
                    "actual": {
                        "path": actual["path"],
                        "sha256": actual["sha256"],
                    },
                    "stdout": stdout,
                },
            )
            cases.append(
                {
                    "id": case_id,
                    "comparison": comparison,
                    "reference": reference,
                    "actual": actual,
                    "receipt": _file_record(candidate_root, receipt_relative),
                }
            )
        comparisons.append({"id": run_id, "cases": cases})
    return comparisons


def _require_comparator_negative_canaries(
    trusted_comparator: Path,
    artifact_root: Path,
) -> None:
    def require_rejected(
        term: str,
        case_id: str,
        reference: Path,
        actual: Path,
    ) -> None:
        _regular_file(reference, f"{term} negative-canary reference")
        _regular_file(actual, f"{term} negative-canary mismatch")
        try:
            _run_presentation_comparator(
                trusted_comparator,
                case_id=case_id,
                reference=reference,
                actual=actual,
            )
        except AcquisitionFailure as error:
            _require(
                f"{term} mismatch:" in str(error),
                f"trusted comparator did not report a {term} mismatch for its "
                f"negative canary: {error}",
            )
            return
        raise AcquisitionFailure(
            f"trusted comparator accepted a {term} negative canary"
        )

    artifacts = artifact_root / "run-1/cpp/artifacts"
    require_rejected(
        "pixel",
        "network-lobby",
        artifacts / "network-lobby.png",
        artifacts / "loader.png",
    )

    layout_reference = artifacts / "startup-main.layout.json"
    _regular_file(layout_reference, "layout negative-canary reference")
    mutated_layout = load_json(layout_reference)
    elements = mutated_layout.get("elements") if isinstance(mutated_layout, dict) else None
    _require(
        isinstance(elements, list),
        "layout negative-canary reference has no element array",
    )
    element = next(
        (
            value
            for value in elements
            if isinstance(value, dict)
            and "port_asset" not in value
            and isinstance(value.get("rect"), dict)
            and type(value["rect"].get("x")) is int
        ),
        None,
    )
    _require(
        element is not None,
        "layout negative-canary reference has no untagged rect",
    )
    element["rect"]["x"] += 1
    with tempfile.TemporaryDirectory(prefix="clonk-presentation-layout-canary-") as temporary:
        layout_actual = Path(temporary) / "startup-main.layout.json"
        _write_json_new(layout_actual, mutated_layout)
        require_rejected(
            "layout",
            "startup-main",
            layout_reference,
            layout_actual,
        )


def _rerun_indexed_comparisons(
    artifact_root: Path,
    index: Mapping[str, Any],
    trusted_comparator: Path,
) -> list[dict[str, Any]]:
    """Rerun every canonical retained pair with a caller-trusted executable."""

    _regular_file(trusted_comparator, "trusted Rust presentation comparator")
    comparison_runs = index.get("comparisons")
    _require(isinstance(comparison_runs, list), "comparisons must be an array")
    _require(
        [entry.get("id") for entry in comparison_runs if isinstance(entry, dict)]
        == list(RUN_IDS),
        "comparison runs drift",
    )
    results = []
    for run_id, comparison_run in zip(RUN_IDS, comparison_runs, strict=True):
        comparison_run = _require_exact_keys(
            comparison_run,
            {"id", "cases"},
            f"trusted comparison run {run_id}",
        )
        cases = comparison_run["cases"]
        _require(isinstance(cases, list), f"{run_id} comparisons must be an array")
        _require(
            [entry.get("id") for entry in cases if isinstance(entry, dict)]
            == list(CASE_IDS),
            f"{run_id} comparison case order drift",
        )
        for case_id, case in zip(CASE_IDS, cases, strict=True):
            case = _require_exact_keys(
                case,
                {"id", "comparison", "reference", "actual", "receipt"},
                f"trusted {run_id} {case_id} comparison entry",
            )
            suffix = "layout.json" if case_id in LAYOUT_CASE_IDS else "png"
            expected_term = "layout" if case_id in LAYOUT_CASE_IDS else "pixel"
            _require(
                case["id"] == case_id and case["comparison"] == expected_term,
                f"trusted {run_id} {case_id} comparison identity drift",
            )
            reference = _validate_v2_file_record(
                case["reference"],
                artifact_root,
                expected_path=f"{run_id}/cpp/artifacts/{case_id}.{suffix}",
                label=f"trusted {run_id} {case_id} comparison reference",
            )
            actual = _validate_v2_file_record(
                case["actual"],
                artifact_root,
                expected_path=f"{run_id}/rust/artifacts/{case_id}.{suffix}",
                label=f"trusted {run_id} {case_id} comparison actual",
            )
            result, _stdout = _run_presentation_comparator(
                trusted_comparator,
                case_id=case_id,
                reference=reference,
                actual=actual,
            )
            results.append({"run_id": run_id, **result})
    _require_comparator_negative_canaries(trusted_comparator, artifact_root)
    return results


def _remove_cpp_runtime_resources(
    candidate_root: Path,
    expected: Any,
    expected_content: Any,
) -> None:
    _validate_staged_cpp_runtime_resources(candidate_root, expected)
    _validate_staged_cpp_runtime_content(candidate_root, expected_content)
    inputs = candidate_root / "inputs"
    for _, retained_path in RUNTIME_RESOURCE_GROUPS.values():
        target = candidate_root / retained_path
        _require(
            target.parent == inputs
            and target.name in {"Graphics.c4g", "System.c4g"},
            f"unsafe C++ runtime resource cleanup target: {target}",
        )
        try:
            shutil.rmtree(target)
        except OSError as error:
            raise AcquisitionFailure(
                f"could not remove acquisition-only runtime resource {target}: {error}"
            ) from error
    _remove_runtime_content_resources(candidate_root, expected_content)
    log = inputs / "Clonk.log"
    if log.exists() or log.is_symlink():
        _regular_file(log, "acquisition-only C++ log")
        try:
            log.unlink()
        except OSError as error:
            raise AcquisitionFailure(
                f"could not remove acquisition-only C++ log {log}: {error}"
            ) from error


def _remove_runtime_content_resources(
    candidate_root: Path,
    expected_content: Any,
) -> None:
    _validate_staged_cpp_runtime_content(candidate_root, expected_content)
    inputs = candidate_root / "inputs"
    for _, retained_path in CPP_RUNTIME_CONTENT_GROUPS.values():
        target = candidate_root / retained_path
        _require(
            target.parent == inputs
            and target.name in {"Tutorial.c4f", "Objects.c4d", "Material.c4g"},
            f"unsafe runtime content cleanup target: {target}",
        )
        try:
            shutil.rmtree(target)
        except OSError as error:
            raise AcquisitionFailure(
                f"could not remove acquisition-only runtime content {target}: {error}"
            ) from error


def _acquisition_inputs(candidate_root: Path) -> dict[str, Any]:
    configs = {}
    for engine in ENGINE_IDS:
        record = _file_record(candidate_root, f"inputs/{engine}.config")
        _require(
            record["sha256"] == NATIVE_CONFIG_SHA256,
            f"{engine} native config differs from the audited bytes",
        )
        configs[engine] = {**record, "profile": ENGINE_PROFILES[engine]}
    player = {
        **_file_record(candidate_root, PLAYER_RETAINED_PATH),
        "id": "presentation-player",
    }
    _require(
        player["sha256"] == PLAYER_SHA256,
        "player fixture differs from the audited presentation player",
    )
    network = _file_record(candidate_root, NETWORK_REFERENCES_RETAINED_PATH)
    _require(
        network["sha256"] == NETWORK_REFERENCES_SHA256,
        "network reference fixture differs from the audited completed-empty fixture",
    )
    return {
        "configs": configs,
        "player": player,
        "network_references": network,
        "rust_source_inventory": _file_record(
            candidate_root,
            RUST_SOURCE_INVENTORY_RETAINED_PATH,
        ),
    }


def _validate_case_input_hashes(
    case_specs: Sequence[Mapping[str, Any]],
    inputs: Mapping[str, Any],
    *,
    fixture_content_tree: str,
    runtime_resources: Mapping[str, Any],
) -> None:
    for spec in case_specs:
        case_id = spec["id"]
        for engine in ENGINE_IDS:
            _require(
                spec["config_sha256"][engine]
                == inputs["configs"][engine]["sha256"],
                f"{case_id} does not bind the exact {engine} native config",
            )
        _require(
            spec["player_sha256"] == inputs["player"]["sha256"],
            f"{case_id} does not bind the exact packed player fixture",
        )
        _require(
            spec["scenario"]["content_tree"] == fixture_content_tree,
            f"{case_id} does not bind the pinned fixture content tree",
        )
        _require(
            spec["runtime_resources"] == runtime_resources,
            f"{case_id} does not bind the exact runtime resources",
        )


def _build_acquisition_index(
    candidate_root: Path,
    *,
    context: Mapping[str, str],
    manifest: Any,
    profile: Any,
    case_specs: Sequence[Mapping[str, Any]],
    inputs: Mapping[str, Any],
    builds: Mapping[str, Any],
    runs: Sequence[Mapping[str, Any]],
    comparisons: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    patch_sha256 = _sha256_file(candidate_root / CAPTURE_PATCH_RETAINED_PATH)
    launcher_sha256 = _sha256_file(candidate_root / LAUNCHER_RETAINED_PATH)
    return {
        "schema": PROVENANCE_SCHEMA,
        "contract": {
            "capture_manifest": {
                "path": CAPTURE_MANIFEST_SOURCE_PATH,
                "contract_sha256": capture_manifest_contract_value_sha256(manifest),
            },
            "compat_profile": {
                "path": "compat/profile.json",
                "contract_sha256": compat_profile_contract_value_sha256(profile),
            },
            "case_specs": {
                "path": CASE_SPECS_SOURCE_PATH,
                "sha256": _canonical_json_sha256(case_specs),
            },
            "geometry": dict(EXPECTED_GEOMETRY),
            "normalization": dict(EXPECTED_NORMALIZATION),
            "launcher": {
                "source_path": LAUNCHER_SOURCE_PATH,
                "retained_path": LAUNCHER_RETAINED_PATH,
                "sha256": launcher_sha256,
            },
        },
        "sources": {
            "oracle": {
                "commit": ORACLE_SOURCE_COMMIT,
                "tree": context["oracle_source_tree"],
                "historical_content_gitlink": ORACLE_SOURCE_CONTENT_GITLINK,
                "planet_tree": context["oracle_planet_tree"],
                "system_tree": context["oracle_system_tree"],
            },
            "rust": {
                "commit": context["rust_source_commit"],
                "tree": context["rust_source_tree"],
                "planet_tree": context["rust_planet_tree"],
                "system_tree": context["rust_system_tree"],
            },
            "fixture_content": {
                "commit": FIXTURE_CONTENT_COMMIT,
                "tree": context["fixture_content_tree"],
            },
            "effective_system_scripts_sha256": context[
                "effective_system_scripts_sha256"
            ],
            "runtime_resources": copy.deepcopy(context["runtime_resources"]),
        },
        "patch": {
            "source_path": CAPTURE_PATCH_SOURCE_PATH,
            "retained_path": CAPTURE_PATCH_RETAINED_PATH,
            "base_commit": ORACLE_SOURCE_COMMIT,
            "sha256": patch_sha256,
        },
        "inputs": dict(inputs),
        "builds": dict(builds),
        "case_specs": [dict(spec) for spec in case_specs],
        "runs": [dict(run) for run in runs],
        "comparisons": [dict(comparison) for comparison in comparisons],
    }


def acquire_presentation_oracle(
    oracle_checkout: Path,
    output_directory: Path,
    *,
    workspace: Path = WORKSPACE,
    accept: bool = False,
    accepted_destination: Path | None = None,
) -> dict[str, Any]:
    """Build and run both producers; accept only complete validated evidence."""

    workspace = workspace.resolve()
    oracle_checkout = oracle_checkout.resolve()
    verify_checkout_head(oracle_checkout)
    rust_commit, _ = require_clean_source_revision(workspace)
    output_directory = _require_fresh_external_output(
        output_directory,
        workspace=workspace,
        oracle_checkout=oracle_checkout,
    )
    context = expected_provenance_fields(
        oracle_checkout,
        rust_revision=rust_commit,
        workspace=workspace,
    )
    manifest = load_json_at_revision(
        workspace,
        rust_commit,
        CAPTURE_MANIFEST_SOURCE_PATH,
    )
    profile = load_json_at_revision(workspace, rust_commit, "compat/profile.json")
    case_specs_value = load_json_at_revision(
        workspace,
        rust_commit,
        CASE_SPECS_SOURCE_PATH,
    )
    _require(isinstance(case_specs_value, list), "tracked case specs must be an array")
    case_specs = _validate_v2_case_specs(
        case_specs_value,
        expected_case_specs=case_specs_value,
        trusted_manifest=manifest,
    )
    source_inventory = rust_source_input_inventory(workspace, rust_commit)
    staged = _stage_acquisition(
        oracle_checkout,
        output_directory,
        workspace=workspace,
        rust_commit=rust_commit,
        source_inventory=source_inventory,
        runtime_resources=context["runtime_resources"],
    )
    inputs = _acquisition_inputs(output_directory)
    _validate_case_input_hashes(
        case_specs,
        inputs,
        fixture_content_tree=context["fixture_content_tree"],
        runtime_resources=context["runtime_resources"],
    )
    source_identity = {
        "schema": "clonk-rs/presentation-source-identity/v1",
        "commit": context["rust_source_commit"],
        "tree": context["rust_source_tree"],
        "content_tree": context["fixture_content_tree"],
    }
    source_identity_path = output_directory / RUST_SOURCE_IDENTITY_RETAINED_PATH
    _write_json_new(source_identity_path, source_identity)
    patch_sha256 = _sha256_file(
        output_directory / CAPTURE_PATCH_RETAINED_PATH
    )
    builds, runtime_binaries = _build_acquisition_binaries(
        output_directory,
        staged,
        context=context,
        patch_sha256=patch_sha256,
    )
    launcher_sha256 = _sha256_file(output_directory / LAUNCHER_RETAINED_PATH)
    case_specs_sha256 = _canonical_json_sha256(case_specs)
    runs = _run_acquisition_captures(
        output_directory,
        runtime_binaries,
        launcher_sha256=launcher_sha256,
        case_specs_sha256=case_specs_sha256,
        runtime_resources=context["runtime_resources"],
        runtime_content_resources=staged["cpp_runtime_content_resources"],
        scenario_resources=staged["cpp_runtime_scenario_resources"],
        rust_source_root=staged["rust_source"],
    )
    comparisons = _run_acquisition_comparisons(
        output_directory,
        runtime_binaries["rust"],
    )
    _remove_cpp_runtime_resources(
        output_directory,
        context["runtime_resources"]["cpp"],
        staged["cpp_runtime_content_resources"],
    )
    _require(
        patch_sha256
        == _sha256_file(output_directory / CAPTURE_PATCH_RETAINED_PATH),
        "capture patch changed during acquisition",
    )

    try:
        source_identity_path.unlink()
        work = staged["work"]
        _require(work.parent == output_directory and work.name == "work", "unsafe work cleanup")
        shutil.rmtree(work)
    except OSError as error:
        raise AcquisitionFailure(f"could not remove acquisition-only files: {error}") from error

    index = _build_acquisition_index(
        output_directory,
        context=context,
        manifest=manifest,
        profile=profile,
        case_specs=case_specs,
        inputs=inputs,
        builds=builds,
        runs=runs,
        comparisons=comparisons,
    )
    _write_json_new(output_directory / "index.json", index)
    trusted_comparator = _acquisition_trusted_comparator(output_directory)
    validated = validate_candidate_output(
        output_directory,
        expected_fields=context,
        expected_case_specs=case_specs,
        trusted_patch=workspace / CAPTURE_PATCH_SOURCE_PATH,
        trusted_launcher=workspace / LAUNCHER_SOURCE_PATH,
        trusted_manifest=manifest,
        trusted_profile=profile,
        expected_source_inventory=source_inventory,
        trusted_comparator=trusted_comparator,
    )
    accepted = []
    if accept:
        destination = accepted_destination or workspace / "compat/presentation/oracle/v1"
        accepted = accept_candidate_output(
            output_directory,
            destination,
            expected_fields=context,
            expected_case_specs=case_specs,
            trusted_patch=workspace / CAPTURE_PATCH_SOURCE_PATH,
            trusted_launcher=workspace / LAUNCHER_SOURCE_PATH,
            trusted_manifest=manifest,
            trusted_profile=profile,
            expected_source_inventory=source_inventory,
            trusted_comparator=trusted_comparator,
        )
    return {
        "rust_source_commit": rust_commit,
        "candidate": str(output_directory),
        "validated_files": len(validated),
        "accepted_files": len(accepted),
    }


def stage_inputs(oracle_checkout: Path, output_directory: Path) -> dict[str, str]:
    """Materialize immutable pinned inputs without building or capturing."""

    _require(
        not output_directory.exists(),
        f"staging output directory must not exist: {output_directory}",
    )
    context = expected_provenance_fields(oracle_checkout)
    _require_destination_outside_repository(oracle_checkout, output_directory)
    output_directory.mkdir(parents=True)
    try:
        materialize_git_archive(
            oracle_checkout,
            ORACLE_SOURCE_COMMIT,
            output_directory / "oracle-source",
        )
        materialize_runtime_resource_tree(
            WORKSPACE / "content",
            FIXTURE_CONTENT_COMMIT,
            None,
            output_directory / "fixture-content",
        )
    except Exception:
        shutil.rmtree(output_directory)
        raise
    return context


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Acquire or nonmutatingly verify pinned presentation-oracle evidence."
        )
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    stage = subparsers.add_parser("stage", help="materialize immutable source/content inputs")
    stage.add_argument("--oracle-root", required=True, type=Path)
    stage.add_argument("--output-dir", required=True, type=Path)

    acquire = subparsers.add_parser(
        "acquire",
        help="build both engines and run two fresh captures for every canonical case",
    )
    acquire.add_argument("--oracle-root", required=True, type=Path)
    acquire.add_argument("--output-dir", required=True, type=Path)
    acquire.add_argument(
        "--accept",
        action="store_true",
        help=f"copy only complete validated evidence to {ACCEPTED_DESTINATION}",
    )

    validate = subparsers.add_parser("validate", help="validate two complete instrumented runs")
    validate.add_argument("--oracle-root", required=True, type=Path)
    validate.add_argument("--output-dir", required=True, type=Path)
    validate.add_argument(
        "--accept",
        action="store_true",
        help=f"copy only validated explicit evidence to {ACCEPTED_DESTINATION}",
    )
    verify_accepted = subparsers.add_parser(
        "verify-accepted",
        help="verify checked-in evidence against squash-stable current source inputs",
    )
    verify_accepted.add_argument("--repo-root", required=True, type=Path)
    verify_accepted_index = subparsers.add_parser(
        "verify-accepted-index",
        help="verify checked-in evidence structure and provenance without building",
    )
    verify_accepted_index.add_argument("--repo-root", required=True, type=Path)

    verify_current = subparsers.add_parser(
        "verify-current",
        help="capture current Rust twice and compare all cases to accepted C++",
    )
    verify_current.add_argument("--repo-root", required=True, type=Path)
    verify_current.add_argument("--output-dir", required=True, type=Path)
    verify_current.add_argument(
        "--profile",
        choices=("release", "test"),
        default="release",
        help="Cargo build profile for the current Rust capture (default: release)",
    )
    return parser


def main(arguments: Sequence[str] | None = None) -> int:
    options = build_parser().parse_args(arguments)
    try:
        if options.command == "stage":
            context = stage_inputs(options.oracle_root, options.output_dir)
            print(
                json.dumps(
                    {
                        "status": "staged",
                        "capture_ready": False,
                        "reason": (
                            "stage only materializes immutable inputs; no build or capture "
                            "was requested and no evidence was created"
                        ),
                        "oracle_source": str(options.output_dir / "oracle-source"),
                        "fixture_content": str(options.output_dir / "fixture-content"),
                        "provenance_context": context,
                    },
                    indent=2,
                    sort_keys=True,
                )
            )
            return 0

        if options.command == "acquire":
            result = acquire_presentation_oracle(
                options.oracle_root,
                options.output_dir,
                accept=options.accept,
            )
            print(json.dumps(result, indent=2, sort_keys=True))
            return 0

        if options.command == "verify-accepted":
            validated = verify_accepted_output(options.repo_root)
            print(f"verified {len(validated)} checked-in presentation evidence files")
            return 0

        if options.command == "verify-accepted-index":
            validated = verify_accepted_index(options.repo_root)
            print(
                f"verified {len(validated)} checked-in presentation evidence files "
                "without executing comparisons"
            )
            return 0

        if options.command == "verify-current":
            result = verify_current_rust(
                options.repo_root,
                options.output_dir,
                build_profile=options.profile,
            )
            print(json.dumps(result, indent=2, sort_keys=True))
            return 0

        context = expected_provenance_fields(options.oracle_root)
        rust_commit = context["rust_source_commit"]
        committed_case_specs = load_json_at_revision(
            WORKSPACE,
            rust_commit,
            CASE_SPECS_SOURCE_PATH,
        )
        _require(isinstance(committed_case_specs, list), "committed case specs must be an array")
        committed_manifest = load_json_at_revision(
            WORKSPACE,
            rust_commit,
            CAPTURE_MANIFEST_SOURCE_PATH,
        )
        committed_profile = load_json_at_revision(
            WORKSPACE,
            rust_commit,
            "compat/profile.json",
        )
        source_inventory = rust_source_input_inventory(WORKSPACE, rust_commit)
        (
            trusted_comparator,
            comparator_commit,
            comparator_tree,
            comparator_inventory,
        ) = _build_trusted_current_comparator(WORKSPACE)
        _require(
            comparator_commit == rust_commit,
            "trusted comparator source commit differs from validation source",
        )
        if options.accept:
            accepted = accept_candidate_output(
                options.output_dir,
                ACCEPTED_DESTINATION,
                expected_fields=context,
                expected_case_specs=committed_case_specs,
                trusted_manifest=committed_manifest,
                trusted_profile=committed_profile,
                expected_source_inventory=source_inventory,
                trusted_comparator=trusted_comparator,
            )
            print(f"accepted {len(accepted)} validated files into {ACCEPTED_DESTINATION}")
        else:
            validated = validate_candidate_output(
                options.output_dir,
                expected_fields=context,
                expected_case_specs=committed_case_specs,
                trusted_manifest=committed_manifest,
                trusted_profile=committed_profile,
                expected_source_inventory=source_inventory,
                trusted_comparator=trusted_comparator,
            )
            print(
                f"validated {len(validated)} files; rerun with --accept to copy explicit evidence"
            )
        verify_clean_source_checkpoint(
            WORKSPACE,
            expected_commit=comparator_commit,
            expected_tree=comparator_tree,
            expected_inventory=comparator_inventory,
            label="after candidate comparison",
        )
        return 0
    except AcquisitionFailure as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
