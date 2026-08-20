#!/usr/bin/env python3
"""Run and compare repeated HarpoonRace-shaped network-load measurements."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import platform
import random
import secrets
import shutil
import statistics
import subprocess
import sys
import time
import tomllib
from pathlib import Path
from typing import Any, Sequence


RUNNER_SCRIPT_PATH = Path(__file__).resolve()
TARGET_METRIC_SPECS = (
    ("control_completion_wait", "microseconds", 1),
    ("client_to_host_isolated_application_round_trip", "microseconds", 1),
)
DIAGNOSTIC_METRIC_SPECS = (
    ("client_to_host_application_round_trip", "microseconds", 1),
    ("client_to_host_round_trip", "milliseconds", 1),
)
METRIC_SPECS = TARGET_METRIC_SPECS + DIAGNOSTIC_METRIC_SPECS
REPORT_METRIC_SPECS = (
    ("join_duration", "microseconds"),
    ("client_to_host_round_trip", "milliseconds"),
    ("client_to_host_application_round_trip", "microseconds"),
    ("client_to_host_isolated_application_round_trip", "microseconds"),
    ("control_completion_wait", "microseconds"),
    ("participant_ready", "microseconds"),
    ("cadence_lateness", "microseconds"),
    ("native_control_wait", "milliseconds"),
)
PRIMARY_METRICS = tuple(name for name, _, _ in METRIC_SPECS)
TARGET_METRICS = tuple(name for name, _, _ in TARGET_METRIC_SPECS)
TEST_NAME = (
    "network_load_24::"
    "harpoonrace_shaped_24_player_control_transport_sustains_lockstep"
)
TEST_ARGUMENTS = [TEST_NAME, "--ignored", "--exact", "--nocapture"]
WARMUP_NOTE = (
    "The loaded Rust session fixes control warmup at 36 ticks (about 2 "
    "seconds); that may not exhaust reliable-UDP's 128-fragment adaptation, "
    "so this suite does not claim a longer steady-state adaptation period. "
    "The fresh isolated RTT warms 128 request/response exchanges before its "
    "256 measured exchanges."
)
SUPPORTED_REPORT_SCHEMA = 6
RUNNER_SCHEMA = 5
PROVENANCE_SCHEMA = 2
BOOTSTRAP_RESAMPLES = 10_000
MINIMUM_INDEPENDENT_RUNS = 20
AUTHORITATIVE_PAIR_COUNT = 20
TARGET_RATIO = 0.5
EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()
DEFAULT_MEASUREMENT_SECONDS = 60
BENCHMARK_CONTRACT_PATHS = (
    Path("crates/clonk-network/tests/network_load_24.rs"),
)
BUILD_ENVIRONMENT_FIELDS = (
    "CARGO_BUILD_TARGET",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_INCREMENTAL",
    "RUSTC",
    "RUSTDOC",
    "RUSTUP_TOOLCHAIN",
    "CC",
    "CXX",
    "AR",
    "CFLAGS",
    "CXXFLAGS",
    "CPPFLAGS",
    "LDFLAGS",
    "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "PKG_CONFIG_PATH",
)
BUILD_ENVIRONMENT_PREFIXES = (
    "CARGO_PROFILE_",
    "CARGO_BUILD_",
    "CARGO_TARGET_",
)
EXPECTED_REPORT_VALUES = {
    "workload": (
        "same-process Tokio IPv4-loopback real-socket "
        "HarpoonRace-shaped control transport"
    ),
    "workload_scope": (
        "HarpoonRace-shaped lobby/control parameters only; no "
        "scenario/resource loading or game simulation"
    ),
    "sequence": (
        "synthetic max_players=24 JoinData -> 24 PlayerInfo joins -> "
        "activate all -> GO"
    ),
    "round_trip_scope": (
        "native ping and loaded 24-client ReadyCheck fanout are diagnostics; "
        "primary RTT is a fresh one-host/one-client post-shutdown application "
        "exchange in the same Tokio process over IPv4 loopback"
    ),
    "application_round_trip_sequence": (
        "diagnostic loaded 24-client fanout after control measurement: "
        "sequential host Ready(client_id) broadcast -> addressed client Ready "
        "echo -> host receipt over selected message routes"
    ),
    "application_round_trip_rounds_per_client": 8,
    "isolated_application_round_trip_sequence": (
        "loaded-session shutdown -> fresh same-topology one-host/one-client "
        "join/status handshake -> 128 warmup + 256 measured sequential "
        "exchanges: host ReadyCheck(Other(index+2)) -> client "
        "ActivationRequest(index+2) -> matching host receipt; exactly two "
        "logical messages per exchange"
    ),
    "isolated_application_round_trip_warmup_samples": 128,
    "isolated_application_round_trip_samples": 256,
    "isolated_application_round_trip_client_id": 1,
    "player_profiles_joined": 24,
    "host_player_profiles": 0,
    "active_control_participants": 25,
    "control_target_fps": 38,
    "native_game_tick_ms": 28,
    "native_control_interval_ms": 56,
    "control_rate": 2,
    "warmup_ticks": 36,
}
REPORT_IDENTITY_FIELDS = (
    "schema_version",
    "workload",
    "workload_scope",
    "sequence",
    "round_trip_scope",
    "application_round_trip_sequence",
    "application_round_trip_rounds_per_client",
    "isolated_application_round_trip_sequence",
    "isolated_application_round_trip_warmup_samples",
    "isolated_application_round_trip_samples",
    "isolated_application_round_trip_client_id",
    "isolated_application_round_trip_preferred_message_routes",
    "authoritative_duration",
    "topology",
    "preferred_message_protocol",
    "player_profiles_joined",
    "host_player_profiles",
    "active_control_participants",
    "control_target_fps",
    "native_game_tick_ms",
    "native_control_interval_ms",
    "control_rate",
    "warmup_ticks",
    "requested_measurement_ms",
    "minimum_native_control_ticks",
)
FINGERPRINT_FIELDS = (
    "source_commit",
    "source_dirty",
    "content_revision",
    "rustc",
    "target_os",
    "target_arch",
    "cpu",
    "os_version",
    "cargo_profile",
)


def expected_assertion_names() -> list[str]:
    names = [
        "every-participant-ready",
        "native-control-cadence",
        "measurement-wall-duration",
        "exact-route-topology",
        "exact-preferred-message-routes",
        "aggregate-rtt-samples",
        "per-client-rtt-series",
    ]
    for client_id in range(1, 25):
        names.extend(
            (
                f"client-{client_id}-rtt-samples",
                f"client-{client_id}-rtt-p99",
            )
        )
    names.extend(
        (
            "aggregate-application-rtt-samples",
            "per-client-application-rtt-series",
        )
    )
    for client_id in range(1, 25):
        names.extend(
            (
                f"client-{client_id}-application-rtt-samples",
                f"client-{client_id}-application-rtt-p99",
            )
        )
    names.extend(
        (
            "aggregate-application-rtt-p99",
            "isolated-application-rtt-warmup-samples",
            "isolated-application-rtt-samples",
            "isolated-application-rtt-client-id",
            "isolated-application-rtt-preferred-message-routes",
            "isolated-application-rtt-p99",
            "aggregate-rtt-p99",
            "control-completion-p99",
            "loaded-session-clean-shutdown",
            "isolated-ping-clean-shutdown",
        )
    )
    return names


def expected_route_peers(topology: str) -> list[list[Any]]:
    direct_mesh = topology != "relay"
    routes: list[list[Any]] = [[0, list(range(1, 25))]]
    routes.extend(
        [
            client_id,
            (
                [peer_id for peer_id in range(25) if peer_id != client_id]
                if direct_mesh
                else [0]
            ),
        ]
        for client_id in range(1, 25)
    )
    return routes


def expected_preferred_message_routes(topology: str) -> list[dict[str, Any]]:
    protocol = "udp" if topology == "udp" else "tcp"
    return [
        {
            "process_client_id": process_client_id,
            "peer_client_id": peer_client_id,
            "protocol": protocol,
        }
        for process_client_id, peer_ids in expected_route_peers(topology)
        for peer_client_id in peer_ids
    ]


def expected_isolated_preferred_message_routes(
    topology: str,
) -> list[dict[str, Any]]:
    protocol = "udp" if topology == "udp" else "tcp"
    return [
        {
            "process_client_id": 0,
            "peer_client_id": 1,
            "protocol": protocol,
        },
        {
            "process_client_id": 1,
            "peer_client_id": 0,
            "protocol": protocol,
        },
    ]


class BenchmarkFailure(RuntimeError):
    """A benchmark could not produce comparable successful observations."""


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("value must be positive")
    return parsed


def safe_label(value: str) -> str:
    if (
        not value
        or len(value) > 64
        or not value[0].isascii()
        or not value[0].isalnum()
        or any(
            not character.isascii()
            or not (character.isalnum() or character in "._-")
            for character in value
        )
    ):
        raise argparse.ArgumentTypeError(
            "label must be one 1-64 character ASCII filename component "
            "starting with a letter or digit"
        )
    return value


def require_safe_label(value: str) -> str:
    try:
        return safe_label(value)
    except argparse.ArgumentTypeError as error:
        raise BenchmarkFailure(
            "benchmark label must be one safe filename component"
        ) from error


def _add_execution_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--runs",
        type=positive_integer,
        default=20,
        help="independent processes per cohort (default: 20)",
    )
    parser.add_argument(
        "--topology",
        choices=("udp", "tcp", "relay"),
        default="udp",
    )
    parser.add_argument(
        "--measurement-seconds",
        type=positive_integer,
        help="non-default duration; omit for the authoritative 60 seconds",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=positive_integer,
        default=300,
    )
    parser.add_argument(
        "--cargo-profile",
        help=(
            "Cargo profile; new builds default to release, while prebuilt "
            "binaries require this option explicitly"
        ),
    )
    parser.add_argument(
        "--repository-root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
    )


def build_argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    run = commands.add_parser("run", help="build once and run one cohort")
    _add_execution_arguments(run)
    run.add_argument("--binary", type=Path, help="use this prebuilt test binary")
    run.add_argument("--label", type=safe_label, default="candidate")
    run.add_argument("--output", type=Path)

    compare = commands.add_parser(
        "compare",
        help="compare two cohort directories or two prebuilt binaries",
    )
    _add_execution_arguments(compare)
    compare.add_argument("baseline", type=Path)
    compare.add_argument("candidate", type=Path)
    compare.add_argument("--output", type=Path)
    return parser


def randomized_pair_orders(pair_count: int, seed_hex: str) -> list[str]:
    try:
        seed = int(seed_hex, 16)
    except ValueError as error:
        raise BenchmarkFailure(
            "paired experiment randomization seed is not hexadecimal"
        ) from error
    orders = ["AB"] * ((pair_count + 1) // 2) + ["BA"] * (pair_count // 2)
    random.Random(seed).shuffle(orders)
    return orders


def paired_schedule(orders: Sequence[str]) -> list[dict[str, Any]]:
    schedule = []
    for pair_index, order in enumerate(orders, start=1):
        labels = (
            ("baseline", "candidate")
            if order == "AB"
            else ("candidate", "baseline")
        )
        for position, label in enumerate(labels, start=1):
            schedule.append(
                {
                    "global_sequence": len(schedule) + 1,
                    "pair_index": pair_index,
                    "order": order,
                    "position": position,
                    "label": label,
                    "run": pair_index,
                }
            )
    return schedule


def paired_experiment_manifest(
    *,
    pair_count: int,
    topology: str,
    measurement_seconds: int | None,
    timeout_seconds: int,
    cargo_profile: str,
    experiment_id: str | None = None,
    randomization_seed: str | None = None,
) -> dict[str, Any]:
    """Predeclare a reproducible randomized order for direct binary pairs."""

    if pair_count < 1:
        raise BenchmarkFailure("paired experiment count must be positive")
    experiment_id = experiment_id or secrets.token_hex(16)
    randomization_seed = randomization_seed or secrets.token_hex(16)
    orders = randomized_pair_orders(pair_count, randomization_seed)
    pairs = [
        {"pair_index": pair_index, "order": order}
        for pair_index, order in enumerate(orders, start=1)
    ]
    schedule = paired_schedule(orders)
    return {
        "schema_version": RUNNER_SCHEMA,
        "kind": "clonk-network-load-benchmark-paired-experiment",
        "experiment_id": experiment_id,
        "created_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "design": "randomized-balanced-direct-interleaved-pairs",
        "predeclared_pair_count": pair_count,
        "authoritative_pair_count": AUTHORITATIVE_PAIR_COUNT,
        "runner_script_sha256": runner_script_sha256(),
        "randomization": {
            "algorithm": "MT19937 shuffle from recorded 128-bit seed",
            "seed_hex": randomization_seed,
        },
        "configuration": {
            "topology": topology,
            "measurement_seconds": measurement_seconds,
            "process_timeout_seconds": timeout_seconds,
            "cargo_profile": cargo_profile,
        },
        "host_observations": runtime_host_observations(),
        "pairs": pairs,
        "schedule": schedule,
    }


def paired_experiment_binding(manifest: dict[str, Any]) -> dict[str, Any]:
    return {
        "experiment_id": manifest["experiment_id"],
        "manifest_sha256": sha256_bytes(
            canonical_json(manifest).encode("utf-8")
        ),
        "design": manifest["design"],
        "predeclared_pair_count": manifest["predeclared_pair_count"],
    }


def validate_experiment_binding(value: Any, context: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {
        "experiment_id",
        "manifest_sha256",
        "design",
        "predeclared_pair_count",
    }:
        raise BenchmarkFailure(f"{context} experiment binding is invalid")
    _require_hex_digest(
        value["experiment_id"], 32, f"{context} experiment ID"
    )
    _require_hex_digest(
        value["manifest_sha256"], 64, f"{context} experiment manifest SHA-256"
    )
    if value["design"] != "randomized-balanced-direct-interleaved-pairs":
        raise BenchmarkFailure(f"{context} experiment design is invalid")
    pair_count = value["predeclared_pair_count"]
    if (
        not isinstance(pair_count, int)
        or isinstance(pair_count, bool)
        or pair_count < 1
    ):
        raise BenchmarkFailure(f"{context} experiment pair count is invalid")
    return value


def sha256_file(path: Path) -> str:
    """Hash one exact benchmark executable without loading it into memory."""

    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def runner_script_sha256() -> str:
    return sha256_file(RUNNER_SCRIPT_PATH)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _command_bytes(repository_root: Path, command: Sequence[str]) -> bytes:
    completed = subprocess.run(
        list(command),
        cwd=repository_root,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        stderr = _timeout_output(completed.stderr).strip()
        raise BenchmarkFailure(
            f"provenance command failed ({' '.join(command)}): {stderr}"
        )
    return completed.stdout


def _command_text(repository_root: Path, command: Sequence[str]) -> str:
    return _command_bytes(repository_root, command).decode(
        "utf-8", errors="replace"
    ).strip()


def _file_hashes(repository_root: Path, paths: Sequence[Path]) -> dict[str, str]:
    hashes = {}
    for path in sorted(paths):
        if path.is_symlink():
            target = os.readlink(path)
            digest = hashlib.sha256(b"symlink\0" + os.fsencode(target))
            resolved = path.resolve()
            if resolved.is_file():
                digest.update(b"\0resolved-file\0")
                digest.update(bytes.fromhex(sha256_file(resolved)))
            hashes[path.relative_to(repository_root).as_posix()] = (
                digest.hexdigest()
            )
        elif path.is_file():
            hashes[path.relative_to(repository_root).as_posix()] = sha256_file(
                path
            )
    return hashes


def cargo_configuration_hashes(repository_root: Path) -> dict[str, str]:
    origins: dict[str, Path] = {}
    for distance, directory in enumerate(
        (repository_root, *repository_root.parents)
    ):
        origin_prefix = "workspace" if distance == 0 else f"ancestor-{distance}"
        for name in ("config", "config.toml"):
            path = directory / ".cargo" / name
            if path.is_file():
                origins[f"{origin_prefix}:.cargo/{name}"] = path
    cargo_home = Path(
        os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
    ).expanduser()
    for name in ("config", "config.toml"):
        path = cargo_home / name
        if path.is_file():
            origins[f"cargo-home:{name}"] = path
    return {
        origin: sha256_file(path)
        for origin, path in sorted(origins.items())
    }


def collect_content_provenance(repository_root: Path) -> dict[str, Any]:
    content_root = (repository_root / "content").resolve()
    if not content_root.is_dir():
        raise BenchmarkFailure(
            f"content checkout is missing for build provenance: {content_root}"
        )
    head = _command_text(content_root, ("git", "rev-parse", "HEAD"))
    tree = _command_text(content_root, ("git", "rev-parse", "HEAD^{tree}"))
    tracked_patch = _command_bytes(
        content_root,
        ("git", "diff", "--binary", "--no-ext-diff", "HEAD", "--", "."),
    )
    untracked_output = _command_bytes(
        content_root,
        ("git", "ls-files", "--others", "--exclude-standard", "-z"),
    )
    untracked_paths = [
        content_root / entry.decode("utf-8", errors="surrogateescape")
        for entry in untracked_output.split(b"\0")
        if entry
    ]
    untracked_hashes = _file_hashes(content_root, untracked_paths)
    untracked_digest_input = b"".join(
        path.encode("utf-8") + b"\0" + digest.encode("ascii") + b"\0"
        for path, digest in untracked_hashes.items()
    )
    gitlink_output = _command_text(
        repository_root, ("git", "ls-tree", "HEAD", "--", "content")
    )
    gitlink_parts = gitlink_output.split()
    if len(gitlink_parts) >= 3 and len(gitlink_parts[2]) == 40:
        parent_gitlink_mode = gitlink_parts[0]
        parent_gitlink_type = gitlink_parts[1]
        parent_gitlink_revision = gitlink_parts[2]
    else:
        parent_gitlink_mode = "missing"
        parent_gitlink_type = "missing"
        parent_gitlink_revision = head
    return {
        "head": head,
        "tree": tree,
        "parent_gitlink_mode": parent_gitlink_mode,
        "parent_gitlink_type": parent_gitlink_type,
        "parent_gitlink_revision": parent_gitlink_revision,
        "tracked_patch_sha256": sha256_bytes(tracked_patch),
        "untracked_inputs_sha256": sha256_bytes(untracked_digest_input),
        "untracked_input_files": untracked_hashes,
        "dirty": bool(tracked_patch or untracked_hashes),
    }


def collect_build_provenance_inputs(
    repository_root: Path,
    *,
    cargo_profile: str | None = None,
) -> dict[str, Any]:
    """Fingerprint all source/config/toolchain inputs that select a binary."""

    repository_root = repository_root.resolve()
    commit = _command_text(repository_root, ("git", "rev-parse", "HEAD"))
    head_tree = _command_text(
        repository_root, ("git", "rev-parse", "HEAD^{tree}")
    )
    tracked_patch = _command_bytes(
        repository_root,
        (
            "git",
            "diff",
            "--binary",
            "--no-ext-diff",
            "HEAD",
            "--",
            ".",
            ":(exclude)content",
        ),
    )
    untracked_output = _command_bytes(
        repository_root,
        (
            "git",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
            ":(exclude)content",
            ":(exclude)target",
        ),
    )
    untracked_paths = [
        repository_root / entry.decode("utf-8", errors="surrogateescape")
        for entry in untracked_output.split(b"\0")
        if entry
    ]
    untracked_hashes = _file_hashes(repository_root, untracked_paths)
    untracked_digest_input = b"".join(
        path.encode("utf-8") + b"\0" + digest.encode("ascii") + b"\0"
        for path, digest in untracked_hashes.items()
    )

    tracked_manifest_output = _command_bytes(
        repository_root,
        (
            "git",
            "ls-files",
            "-z",
            "--",
            "Cargo.toml",
            "**/Cargo.toml",
        ),
    )
    manifest_names = {
        entry.decode("utf-8", errors="surrogateescape")
        for entry in tracked_manifest_output.split(b"\0")
        if entry
    }
    manifest_names.update(
        path for path in untracked_hashes if path.endswith("Cargo.toml")
    )
    manifests = [repository_root / name for name in manifest_names]
    configurations = [
        path
        for path in (
            repository_root / ".cargo/config",
            repository_root / ".cargo/config.toml",
            repository_root / "rust-toolchain",
            repository_root / "rust-toolchain.toml",
        )
        if path.is_file()
    ]
    cargo_lock = repository_root / "Cargo.lock"
    if not cargo_lock.is_file():
        raise BenchmarkFailure(f"Cargo.lock is missing: {cargo_lock}")
    contract_files = [repository_root / path for path in BENCHMARK_CONTRACT_PATHS]
    missing_contracts = [
        path.relative_to(repository_root).as_posix()
        for path in contract_files
        if not path.is_file()
    ]
    if missing_contracts:
        raise BenchmarkFailure(
            "benchmark contract files are missing: "
            f"{', '.join(missing_contracts)}"
        )
    try:
        workspace_manifest = tomllib.loads(
            (repository_root / "Cargo.toml").read_text(encoding="utf-8")
        )
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise BenchmarkFailure(
            f"could not parse workspace Cargo.toml: {error}"
        ) from error
    workspace_profile_tables = workspace_manifest.get("profile", {})
    if not isinstance(workspace_profile_tables, dict):
        raise BenchmarkFailure("workspace Cargo profile tables are invalid")
    environment = {
        name: os.environ.get(name) for name in BUILD_ENVIRONMENT_FIELDS
    }
    environment.update(
        {
            name: value
            for name, value in os.environ.items()
            if any(
                name.startswith(prefix)
                for prefix in BUILD_ENVIRONMENT_PREFIXES
            )
        }
    )
    return {
        "source": {
            "commit": commit,
            "head_tree": head_tree,
            "tracked_patch_sha256": sha256_bytes(tracked_patch),
            "untracked_inputs_sha256": sha256_bytes(untracked_digest_input),
            "untracked_input_files": untracked_hashes,
            "dirty": bool(tracked_patch or untracked_hashes),
        },
        "content": collect_content_provenance(repository_root),
        "inputs": {
            "cargo_lock_sha256": sha256_file(cargo_lock),
            "manifest_files": _file_hashes(repository_root, manifests),
            "configuration_files": _file_hashes(
                repository_root, configurations
            ),
            "cargo_configuration_files": cargo_configuration_hashes(
                repository_root
            ),
            "benchmark_contract_files": _file_hashes(
                repository_root, contract_files
            ),
            "effective_profile": {
                "selected_profile": cargo_profile,
                "workspace_profile_tables": workspace_profile_tables,
                "cargo_artifact_profile": {},
            },
        },
        "toolchain": {
            "rustc_vv": _command_text(
                repository_root,
                (os.environ.get("RUSTC") or "rustc", "-Vv"),
            ),
            "cargo_version": _command_text(repository_root, ("cargo", "-Vv")),
        },
        "environment": dict(sorted(environment.items())),
    }


def write_json(path: Path, value: Any) -> None:
    path.write_text(canonical_json(value), encoding="utf-8")


def canonical_json(value: Any) -> str:
    return json.dumps(value, indent=2, sort_keys=True) + "\n"


def runtime_machine_fingerprint() -> dict[str, str]:
    if hasattr(os, "uname"):
        uname = os.uname()
        system = uname.sysname
    else:
        uname = platform.uname()
        system = uname.system
    cpuinfo = Path("/proc/cpuinfo")
    processor = ""
    if cpuinfo.is_file():
        processor = next(
            (
                line.split(":", 1)[1].strip()
                for line in cpuinfo.read_text(
                    encoding="utf-8", errors="replace"
                ).splitlines()
                if line.lower().startswith("model name") and ":" in line
            ),
            "",
        )
    if not processor:
        processor = (
            f"{uname.machine or 'unknown-architecture'} "
            f"{os.cpu_count() or 'unknown'} logical CPUs"
        )
    return {
        "system": system or "unknown-system",
        "release": uname.release or "unknown-release",
        "machine": uname.machine or "unknown-architecture",
        "processor": processor,
        "python_implementation": platform.python_implementation(),
        "python_version": platform.python_version(),
    }


def runtime_host_observations() -> dict[str, Any]:
    logical_cpu_count = os.cpu_count() or 1
    if hasattr(os, "sched_getaffinity"):
        try:
            affinity = {
                "status": "observed",
                "logical_cpu_ids": sorted(os.sched_getaffinity(0)),
            }
        except OSError as error:
            affinity = {"status": "unavailable", "reason": str(error)}
    else:
        affinity = {
            "status": "unavailable",
            "reason": "platform has no sched_getaffinity",
        }
    if hasattr(os, "getloadavg"):
        try:
            one, five, fifteen = os.getloadavg()
            load_average = {
                "status": "observed",
                "one_minute": one,
                "five_minutes": five,
                "fifteen_minutes": fifteen,
            }
        except OSError as error:
            load_average = {"status": "unavailable", "reason": str(error)}
    else:
        load_average = {
            "status": "unavailable",
            "reason": "platform has no getloadavg",
        }
    power_supplies = Path("/sys/class/power_supply")
    observed_power = []
    if power_supplies.is_dir():
        for supply in sorted(power_supplies.iterdir()):
            fields = {"name": supply.name}
            for field in ("type", "online", "status", "capacity"):
                path = supply / field
                if path.is_file():
                    try:
                        fields[field] = path.read_text(
                            encoding="utf-8", errors="replace"
                        ).strip()
                    except OSError:
                        continue
            observed_power.append(fields)
    power = (
        {"status": "observed", "supplies": observed_power}
        if observed_power
        else {
            "status": "unavailable",
            "reason": "portable power-state telemetry is unavailable",
        }
    )
    return {
        "captured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "logical_cpu_count": logical_cpu_count,
        "process_cpu_affinity": affinity,
        "load_average": load_average,
        "power": power,
    }


def validate_runtime_host_observations(value: Any) -> None:
    if not isinstance(value, dict):
        raise BenchmarkFailure("paired experiment host observations are invalid")
    _require_fields(
        value,
        (
            "captured_at_utc",
            "logical_cpu_count",
            "process_cpu_affinity",
            "load_average",
            "power",
        ),
        "paired experiment host observations",
    )
    if (
        not isinstance(value["logical_cpu_count"], int)
        or isinstance(value["logical_cpu_count"], bool)
        or value["logical_cpu_count"] < 1
    ):
        raise BenchmarkFailure("paired experiment CPU count is invalid")
    for field in ("process_cpu_affinity", "load_average", "power"):
        observation = value[field]
        if (
            not isinstance(observation, dict)
            or observation.get("status") not in {"observed", "unavailable"}
        ):
            raise BenchmarkFailure(
                f"paired experiment {field} observation is invalid"
            )


def _timeout_output(value: str | bytes | None) -> str:
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value or ""


def run_one(
    *,
    binary: Path,
    run_directory: Path,
    repository_root: Path,
    topology: str,
    measurement_seconds: int | None,
    expected_binary_sha256: str,
    timeout_seconds: int,
    experiment: dict[str, Any] | None = None,
    experiment_step: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Run one independent process and preserve all available evidence."""

    run_directory.mkdir(parents=True, exist_ok=False)
    binary = binary.resolve()
    report_path = (run_directory / "report.json").resolve()
    command = [str(binary), *TEST_ARGUMENTS]
    environment = os.environ.copy()
    environment["LC_NETWORK_LOAD_TOPOLOGY"] = topology
    environment["LC_NETWORK_LOAD_METRICS"] = str(report_path)
    if measurement_seconds is not None:
        environment["LC_NETWORK_LOAD_SECONDS"] = str(measurement_seconds)
    else:
        environment.pop("LC_NETWORK_LOAD_SECONDS", None)

    started_at = dt.datetime.now(dt.timezone.utc)
    started = time.monotonic()
    actual_digest = sha256_file(binary)
    return_code: int | None = None
    timed_out = False
    failure = None
    stdout = ""
    stderr = ""
    if actual_digest != expected_binary_sha256:
        failure = (
            "binary SHA-256 changed before run: "
            f"expected {expected_binary_sha256}, observed {actual_digest}"
        )
        stderr = failure + "\n"
    else:
        try:
            completed = subprocess.run(
                command,
                cwd=repository_root,
                env=environment,
                capture_output=True,
                text=True,
                timeout=timeout_seconds,
                check=False,
            )
            return_code = completed.returncode
            stdout = completed.stdout
            stderr = completed.stderr
            if return_code != 0:
                failure = f"test binary exited with status {return_code}"
            elif not report_path.is_file():
                failure = "test binary exited successfully without a report"
        except subprocess.TimeoutExpired as error:
            timed_out = True
            stdout = _timeout_output(error.stdout)
            stderr = _timeout_output(error.stderr)
            failure = f"test binary exceeded {timeout_seconds}s timeout"
        except OSError as error:
            failure = f"could not execute test binary: {error}"
            stderr = failure + "\n"

    elapsed_seconds = time.monotonic() - started
    final_digest = sha256_file(binary) if binary.is_file() else None
    if final_digest != expected_binary_sha256:
        binary_failure = (
            "binary SHA-256 changed during run: "
            f"expected {expected_binary_sha256}, observed {final_digest}"
        )
        failure = "; ".join(
            part for part in (failure, binary_failure) if part
        )
    (run_directory / "stdout.log").write_text(stdout, encoding="utf-8")
    (run_directory / "stderr.log").write_text(stderr, encoding="utf-8")
    report_present = report_path.is_file()
    execution = {
        "schema_version": RUNNER_SCHEMA,
        "kind": "clonk-network-load-benchmark-execution",
        "command": command,
        "started_at_utc": started_at.isoformat(),
        "elapsed_seconds": elapsed_seconds,
        "return_code": return_code,
        "timed_out": timed_out,
        "failure": failure,
        "report_present": report_present,
        "report_sha256": sha256_file(report_path) if report_present else None,
        "binary_sha256": actual_digest,
        "binary_sha256_before": actual_digest,
        "binary_sha256_after": final_digest,
        "environment": {
            "LC_NETWORK_LOAD_TOPOLOGY": topology,
            "LC_NETWORK_LOAD_METRICS": str(report_path),
            "LC_NETWORK_LOAD_SECONDS": (
                str(measurement_seconds)
                if measurement_seconds is not None
                else None
            ),
        },
    }
    if experiment is not None and experiment_step is not None:
        execution.update(
            {
                "experiment_id": experiment["experiment_id"],
                "experiment_manifest_sha256": experiment["manifest_sha256"],
                "pair_index": experiment_step["pair_index"],
                "pair_order": experiment_step["order"],
                "pair_position": experiment_step["position"],
                "global_sequence": experiment_step["global_sequence"],
            }
        )
    write_json(run_directory / "execution.json", execution)
    return {
        "passed": failure is None and return_code == 0 and report_path.is_file(),
        "report_path": str(report_path),
        "execution": execution,
    }


def _integration_test_artifacts(cargo_stdout: str) -> list[dict[str, Any]]:
    candidates = []
    for line in cargo_stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = message.get("target", {})
        if (
            message.get("reason") == "compiler-artifact"
            and target.get("name") == "integration"
            and "test" in target.get("kind", [])
            and message.get("executable")
        ):
            candidates.append(message)
    unique_candidates = {
        message["executable"]: message for message in candidates
    }
    if len(unique_candidates) != 1:
        raise BenchmarkFailure(
            "Cargo reported "
            f"{len(unique_candidates)} clonk-network integration test binaries"
        )
    return list(unique_candidates.values())


def discover_test_binary_artifact(
    cargo_stdout: str,
) -> tuple[Path, dict[str, Any]]:
    """Return Cargo's executable and exact compiler-artifact profile."""

    artifact = _integration_test_artifacts(cargo_stdout)[0]
    profile = artifact.get("profile")
    if not isinstance(profile, dict) or not profile:
        raise BenchmarkFailure(
            "Cargo did not report the selected integration-test profile"
        )
    return Path(artifact["executable"]), profile


def binary_metadata(binary: Path) -> dict[str, Any]:
    binary = binary.resolve()
    if not binary.is_file():
        raise BenchmarkFailure(f"benchmark binary does not exist: {binary}")
    stat = binary.stat()
    return {
        "path": str(binary),
        "sha256": sha256_file(binary),
        "size_bytes": stat.st_size,
        "modified_time_ns": stat.st_mtime_ns,
    }


def provenance_sidecar_path(binary: Path) -> Path:
    return binary.with_name(f"{binary.name}.network-load-provenance.json")


def _require_hex_digest(value: Any, length: int, context: str) -> None:
    if (
        not isinstance(value, str)
        or len(value) != length
        or any(character not in "0123456789abcdef" for character in value)
    ):
        raise BenchmarkFailure(f"{context} is not a {length}-digit hex digest")


def _validate_hash_map(value: Any, context: str) -> dict[str, str]:
    if not isinstance(value, dict) or any(
        not isinstance(path, str)
        or not path
        or Path(path).is_absolute()
        or ".." in Path(path).parts
        for path in value
    ):
        raise BenchmarkFailure(f"{context} is not a relative-path hash map")
    for path, digest in value.items():
        _require_hex_digest(digest, 64, f"{context} entry {path!r}")
    return value


def untracked_hash_map_digest(value: dict[str, str]) -> str:
    digest_input = b"".join(
        path.encode("utf-8") + b"\0" + digest.encode("ascii") + b"\0"
        for path, digest in sorted(value.items())
    )
    return sha256_bytes(digest_input)


def validate_dirty_evidence(value: dict[str, Any], context: str) -> None:
    untracked_files = value["untracked_input_files"]
    if value["untracked_inputs_sha256"] != untracked_hash_map_digest(
        untracked_files
    ):
        raise BenchmarkFailure(
            f"{context} untracked aggregate digest differs from its file map"
        )
    evidence_is_dirty = (
        value["tracked_patch_sha256"] != EMPTY_SHA256
        or bool(untracked_files)
    )
    if value["dirty"] is not evidence_is_dirty:
        raise BenchmarkFailure(
            f"{context} dirty state disagrees with recorded evidence"
        )


def validate_effective_profile(value: Any, context: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {
        "selected_profile",
        "workspace_profile_tables",
        "cargo_artifact_profile",
    }:
        raise BenchmarkFailure(f"{context} effective Cargo profile is invalid")
    if (
        not isinstance(value["selected_profile"], str)
        or not value["selected_profile"]
        or not isinstance(value["workspace_profile_tables"], dict)
        or not isinstance(value["cargo_artifact_profile"], dict)
        or not value["cargo_artifact_profile"]
    ):
        raise BenchmarkFailure(f"{context} effective Cargo profile is invalid")
    if not isinstance(
        value["cargo_artifact_profile"].get("debug_assertions"), bool
    ):
        raise BenchmarkFailure(
            f"{context} Cargo artifact debug-assertion state is invalid"
        )
    return value


def validate_content_provenance(value: Any, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise BenchmarkFailure(f"{context} content provenance is missing")
    _require_fields(
        value,
        (
            "head",
            "tree",
            "parent_gitlink_mode",
            "parent_gitlink_type",
            "parent_gitlink_revision",
            "tracked_patch_sha256",
            "untracked_inputs_sha256",
            "untracked_input_files",
            "dirty",
        ),
        f"{context} content provenance",
    )
    for field in ("head", "tree", "parent_gitlink_revision"):
        _require_hex_digest(
            value[field], 40, f"{context} content {field}"
        )
    for field in ("parent_gitlink_mode", "parent_gitlink_type"):
        if not isinstance(value[field], str) or not value[field]:
            raise BenchmarkFailure(
                f"{context} content {field} is invalid"
            )
    for field in ("tracked_patch_sha256", "untracked_inputs_sha256"):
        _require_hex_digest(
            value[field], 64, f"{context} content {field}"
        )
    _validate_hash_map(
        value["untracked_input_files"], f"{context} content untracked inputs"
    )
    if not isinstance(value["dirty"], bool):
        raise BenchmarkFailure(f"{context} content dirty state is invalid")
    validate_dirty_evidence(value, f"{context} content")
    return value


def load_prebuilt_provenance(binary: Path) -> dict[str, Any]:
    """Load provenance bound to an exact prebuilt executable."""

    binary = binary.resolve()
    sidecar = provenance_sidecar_path(binary)
    if not sidecar.is_file():
        raise BenchmarkFailure(
            "prebuilt benchmark binary requires provenance sidecar: "
            f"{sidecar}"
        )
    provenance = _load_report(sidecar)
    if provenance.get("schema_version") != PROVENANCE_SCHEMA:
        raise BenchmarkFailure(
            f"unsupported prebuilt provenance schema in {sidecar}"
        )
    if provenance.get("kind") != "clonk-network-load-build-provenance":
        raise BenchmarkFailure(f"invalid prebuilt provenance kind in {sidecar}")
    expected_binary = binary_metadata(binary)
    recorded_binary = provenance.get("binary")
    if not isinstance(recorded_binary, dict):
        raise BenchmarkFailure(f"prebuilt provenance has no binary record: {sidecar}")
    for field in ("sha256", "size_bytes"):
        if recorded_binary.get(field) != expected_binary[field]:
            raise BenchmarkFailure(
                f"prebuilt provenance binary {field} does not match {binary}"
            )
    source = provenance.get("source")
    if not isinstance(source, dict):
        raise BenchmarkFailure("prebuilt provenance source is missing")
    _require_fields(
        source,
        (
            "commit",
            "head_tree",
            "tracked_patch_sha256",
            "untracked_inputs_sha256",
            "untracked_input_files",
            "dirty",
        ),
        "prebuilt provenance source",
    )
    for field in ("commit", "head_tree"):
        _require_hex_digest(source.get(field), 40, f"provenance source {field}")
    for field in ("tracked_patch_sha256", "untracked_inputs_sha256"):
        _require_hex_digest(source.get(field), 64, f"provenance source {field}")
    if not isinstance(source["dirty"], bool):
        raise BenchmarkFailure("prebuilt provenance source dirty is not boolean")
    _validate_hash_map(
        source["untracked_input_files"],
        "prebuilt provenance untracked input files",
    )
    validate_dirty_evidence(source, "prebuilt provenance source")
    validate_content_provenance(provenance.get("content"), "prebuilt provenance")

    inputs = provenance.get("inputs")
    if not isinstance(inputs, dict):
        raise BenchmarkFailure("prebuilt provenance inputs are missing")
    _require_fields(
        inputs,
        (
            "cargo_lock_sha256",
            "manifest_files",
            "configuration_files",
            "cargo_configuration_files",
            "benchmark_contract_files",
            "effective_profile",
        ),
        "prebuilt provenance inputs",
    )
    _require_hex_digest(
        inputs["cargo_lock_sha256"], 64, "provenance Cargo.lock hash"
    )
    manifests = _validate_hash_map(
        inputs["manifest_files"], "prebuilt provenance manifest files"
    )
    if "Cargo.toml" not in manifests:
        raise BenchmarkFailure(
            "prebuilt provenance manifest files omit workspace Cargo.toml"
        )
    _validate_hash_map(
        inputs["configuration_files"],
        "prebuilt provenance configuration files",
    )
    _validate_hash_map(
        inputs["cargo_configuration_files"],
        "prebuilt provenance Cargo configuration files",
    )
    contract_files = _validate_hash_map(
        inputs["benchmark_contract_files"],
        "prebuilt provenance benchmark contract files",
    )
    if set(contract_files) != {
        path.as_posix() for path in BENCHMARK_CONTRACT_PATHS
    }:
        raise BenchmarkFailure(
            "prebuilt provenance benchmark contract file set is invalid"
        )
    effective_profile = validate_effective_profile(
        inputs["effective_profile"], "prebuilt provenance"
    )

    toolchain = provenance.get("toolchain")
    if not isinstance(toolchain, dict):
        raise BenchmarkFailure("prebuilt provenance toolchain is missing")
    _require_fields(
        toolchain,
        ("rustc_vv", "cargo_version"),
        "prebuilt provenance toolchain",
    )
    for field in ("rustc_vv", "cargo_version"):
        if not isinstance(toolchain[field], str) or not toolchain[field]:
            raise BenchmarkFailure(
                f"prebuilt provenance toolchain {field} is empty"
            )

    environment = provenance.get("environment")
    if not isinstance(environment, dict):
        raise BenchmarkFailure("prebuilt provenance environment is missing")
    _require_fields(
        environment,
        BUILD_ENVIRONMENT_FIELDS,
        "prebuilt provenance environment",
    )
    if any(
        value is not None and not isinstance(value, str)
        for value in environment.values()
    ):
        raise BenchmarkFailure(
            "prebuilt provenance environment contains a non-string value"
        )

    build = provenance.get("build")
    if not isinstance(build, dict):
        raise BenchmarkFailure("prebuilt provenance build is missing")
    _require_fields(
        build,
        (
            "cargo_profile",
            "command",
            "runner_contract_version",
            "runner_script_sha256",
        ),
        "prebuilt provenance build",
    )
    if not isinstance(build["cargo_profile"], str) or not build["cargo_profile"]:
        raise BenchmarkFailure("prebuilt provenance Cargo profile is empty")
    if not isinstance(build["command"], list) or not all(
        isinstance(argument, str) for argument in build["command"]
    ):
        raise BenchmarkFailure("prebuilt provenance build command is invalid")
    try:
        command_profile = build["command"][
            build["command"].index("--profile") + 1
        ]
    except (ValueError, IndexError):
        command_profile = None
    if command_profile != build["cargo_profile"]:
        raise BenchmarkFailure(
            "prebuilt provenance command does not select its Cargo profile"
        )
    if build["runner_contract_version"] != RUNNER_SCHEMA:
        raise BenchmarkFailure(
            "prebuilt provenance runner contract version is incompatible"
        )
    _require_hex_digest(
        build["runner_script_sha256"],
        64,
        "prebuilt provenance runner script hash",
    )
    if build["runner_script_sha256"] != runner_script_sha256():
        raise BenchmarkFailure(
            "prebuilt provenance runner script hash differs from the "
            "executing runner"
        )
    if effective_profile["selected_profile"] != build["cargo_profile"]:
        raise BenchmarkFailure(
            "prebuilt provenance effective Cargo profile differs from build"
        )
    return provenance


def prebuilt_build_record(binary: Path, cargo_profile: str) -> dict[str, Any]:
    provenance = load_prebuilt_provenance(binary)
    recorded_profile = provenance.get("build", {}).get("cargo_profile")
    if recorded_profile != cargo_profile:
        raise BenchmarkFailure(
            "prebuilt provenance Cargo profile differs from the requested "
            f"profile: {recorded_profile!r} != {cargo_profile!r}"
        )
    return {
        "mode": "provided-prebuilt-binary",
        "cargo_profile": recorded_profile,
        "binary": binary_metadata(binary),
        "provenance": provenance,
        "provenance_sidecar": str(provenance_sidecar_path(binary.resolve())),
        "provenance_sha256": sha256_bytes(
            canonical_json(provenance).encode("utf-8")
        ),
    }


def comparable_build_identity(build: dict[str, Any]) -> dict[str, Any]:
    provenance = build.get("provenance")
    if not isinstance(provenance, dict):
        raise BenchmarkFailure("cohort build provenance is missing")
    return {
        "cargo_profile": provenance.get("build", {}).get("cargo_profile"),
        "command": provenance.get("build", {}).get("command"),
        "toolchain": provenance.get("toolchain"),
        "content": provenance.get("content"),
        "environment": provenance.get("environment"),
        "cargo_lock_sha256": provenance.get("inputs", {}).get(
            "cargo_lock_sha256"
        ),
        "manifest_files": provenance.get("inputs", {}).get(
            "manifest_files"
        ),
        "configuration_files": provenance.get("inputs", {}).get(
            "configuration_files"
        ),
        "cargo_configuration_files": provenance.get("inputs", {}).get(
            "cargo_configuration_files"
        ),
        "benchmark_contract_files": provenance.get("inputs", {}).get(
            "benchmark_contract_files"
        ),
        "effective_profile": provenance.get("inputs", {}).get(
            "effective_profile"
        ),
        "runner_contract_version": provenance.get("build", {}).get(
            "runner_contract_version"
        ),
        "runner_script_sha256": provenance.get("build", {}).get(
            "runner_script_sha256"
        ),
    }


def require_comparable_builds(
    baseline_build: dict[str, Any], candidate_build: dict[str, Any]
) -> None:
    if comparable_build_identity(baseline_build) != comparable_build_identity(
        candidate_build
    ):
        raise BenchmarkFailure(
            "baseline and candidate build environments differ"
        )


def require_authoritative_source_evidence(
    builds: Sequence[dict[str, Any]], *, pair_count: int
) -> None:
    if pair_count != AUTHORITATIVE_PAIR_COUNT:
        return
    for build in builds:
        provenance = build.get("provenance", {})
        if provenance.get("source", {}).get("dirty") is not False:
            raise BenchmarkFailure(
                "authoritative paired experiment requires clean source "
                "provenance when dirty source bytes are not archived"
            )
        if provenance.get("content", {}).get("dirty") is not False:
            raise BenchmarkFailure(
                "authoritative paired experiment requires clean content "
                "provenance when dirty content bytes are not archived"
            )
        content = provenance.get("content", {})
        if (
            content.get("parent_gitlink_mode") != "160000"
            or content.get("parent_gitlink_type") != "commit"
        ):
            raise BenchmarkFailure(
                "authoritative paired experiment requires a real 160000 "
                "parent gitlink for content"
            )
        if content.get("head") != content.get("parent_gitlink_revision"):
            raise BenchmarkFailure(
                "authoritative paired experiment requires content HEAD to "
                "match the parent gitlink"
            )


def validate_retained_build(
    build: Any, binary: dict[str, Any], *, context: str
) -> dict[str, Any]:
    if not isinstance(build, dict):
        raise BenchmarkFailure(f"{context} build metadata is missing")
    profile = build.get("cargo_profile")
    if not isinstance(profile, str) or not profile:
        raise BenchmarkFailure(f"{context} Cargo profile is missing")
    provenance = build.get("provenance")
    if not isinstance(provenance, dict):
        raise BenchmarkFailure(f"{context} build provenance is missing")
    if (
        provenance.get("schema_version") != PROVENANCE_SCHEMA
        or provenance.get("kind")
        != "clonk-network-load-build-provenance"
    ):
        raise BenchmarkFailure(f"{context} build provenance schema is invalid")
    recorded_provenance_sha256 = build.get("provenance_sha256")
    _require_hex_digest(
        recorded_provenance_sha256, 64, f"{context} provenance SHA-256"
    )
    actual_provenance_sha256 = sha256_bytes(
        canonical_json(provenance).encode("utf-8")
    )
    if recorded_provenance_sha256 != actual_provenance_sha256:
        raise BenchmarkFailure(f"{context} embedded build provenance hash differs")
    provenance_binary = provenance.get("binary")
    if not isinstance(provenance_binary, dict):
        raise BenchmarkFailure(f"{context} provenance binary metadata is missing")
    if provenance_binary.get("sha256") != binary.get("sha256"):
        raise BenchmarkFailure(f"{context} provenance binary hash differs")
    if provenance_binary.get("size_bytes") != binary.get("size_bytes"):
        raise BenchmarkFailure(f"{context} provenance binary size differs")
    source = provenance.get("source")
    if not isinstance(source, dict):
        raise BenchmarkFailure(f"{context} provenance source is missing")
    _require_fields(
        source,
        (
            "commit",
            "head_tree",
            "tracked_patch_sha256",
            "untracked_inputs_sha256",
            "untracked_input_files",
            "dirty",
        ),
        f"{context} provenance source",
    )
    for field in ("commit", "head_tree"):
        _require_hex_digest(
            source[field], 40, f"{context} provenance source {field}"
        )
    for field in ("tracked_patch_sha256", "untracked_inputs_sha256"):
        _require_hex_digest(
            source[field], 64, f"{context} provenance source {field}"
        )
    if not isinstance(source["dirty"], bool):
        raise BenchmarkFailure(f"{context} provenance dirty state is invalid")
    _validate_hash_map(
        source["untracked_input_files"],
        f"{context} provenance untracked inputs",
    )
    validate_dirty_evidence(source, f"{context} provenance source")
    validate_content_provenance(
        provenance.get("content"), f"{context} provenance"
    )
    inputs = provenance.get("inputs")
    if not isinstance(inputs, dict):
        raise BenchmarkFailure(f"{context} provenance inputs are missing")
    _require_fields(
        inputs,
        (
            "cargo_lock_sha256",
            "manifest_files",
            "configuration_files",
            "cargo_configuration_files",
            "benchmark_contract_files",
            "effective_profile",
        ),
        f"{context} provenance inputs",
    )
    _require_hex_digest(
        inputs["cargo_lock_sha256"], 64, f"{context} Cargo.lock hash"
    )
    manifests = _validate_hash_map(
        inputs["manifest_files"], f"{context} provenance manifests"
    )
    if "Cargo.toml" not in manifests:
        raise BenchmarkFailure(
            f"{context} provenance omits workspace Cargo.toml"
        )
    _validate_hash_map(
        inputs["configuration_files"], f"{context} provenance configuration"
    )
    _validate_hash_map(
        inputs["cargo_configuration_files"],
        f"{context} provenance Cargo configuration",
    )
    contract_files = _validate_hash_map(
        inputs["benchmark_contract_files"],
        f"{context} provenance benchmark contract",
    )
    if set(contract_files) != {
        path.as_posix() for path in BENCHMARK_CONTRACT_PATHS
    }:
        raise BenchmarkFailure(f"{context} benchmark contract file set is invalid")
    effective_profile = validate_effective_profile(
        inputs["effective_profile"], context
    )
    identity = comparable_build_identity(build)
    if identity["cargo_profile"] != profile:
        raise BenchmarkFailure(f"{context} provenance Cargo profile differs")
    if effective_profile["selected_profile"] != profile:
        raise BenchmarkFailure(f"{context} effective Cargo profile differs")
    if (
        not isinstance(identity["command"], list)
        or not identity["command"]
        or not all(isinstance(argument, str) for argument in identity["command"])
    ):
        raise BenchmarkFailure(f"{context} provenance build command is invalid")
    try:
        command_profile = identity["command"][
            identity["command"].index("--profile") + 1
        ]
    except (ValueError, IndexError):
        command_profile = None
    if command_profile != profile:
        raise BenchmarkFailure(
            f"{context} provenance command Cargo profile differs"
        )
    if identity["runner_contract_version"] != RUNNER_SCHEMA:
        raise BenchmarkFailure(f"{context} runner contract version is invalid")
    _require_hex_digest(
        identity["runner_script_sha256"],
        64,
        f"{context} runner script hash",
    )
    if identity["runner_script_sha256"] != runner_script_sha256():
        raise BenchmarkFailure(
            f"{context} runner script hash differs from the executing runner"
        )
    toolchain = identity["toolchain"]
    if not isinstance(toolchain, dict) or any(
        not isinstance(toolchain.get(field), str) or not toolchain[field]
        for field in ("rustc_vv", "cargo_version")
    ):
        raise BenchmarkFailure(f"{context} provenance toolchain is invalid")
    environment = identity["environment"]
    if (
        not isinstance(environment, dict)
        or any(field not in environment for field in BUILD_ENVIRONMENT_FIELDS)
        or any(not isinstance(name, str) for name in environment)
        or any(
            value is not None and not isinstance(value, str)
            for value in environment.values()
        )
    ):
        raise BenchmarkFailure(f"{context} provenance environment is invalid")
    return provenance


def build_test_binary(
    *, repository_root: Path, cargo_profile: str
) -> tuple[Path, dict[str, Any]]:
    """Build once and return Cargo's exact prebuilt integration executable."""

    repository_root = repository_root.resolve()
    provenance_inputs = collect_build_provenance_inputs(
        repository_root, cargo_profile=cargo_profile
    )
    command = [
        "cargo",
        "test",
        "--locked",
        "-p",
        "clonk-network",
        "--test",
        "integration",
        "--profile",
        cargo_profile,
        "--no-run",
        "--message-format=json-render-diagnostics",
    ]
    started_at = dt.datetime.now(dt.timezone.utc)
    started = time.monotonic()
    completed = subprocess.run(
        command,
        cwd=repository_root,
        capture_output=True,
        text=True,
        check=False,
    )
    elapsed_seconds = time.monotonic() - started
    if completed.returncode != 0:
        raise BenchmarkFailure(
            f"Cargo benchmark build failed with status {completed.returncode}:\n"
            f"{completed.stderr}"
        )
    post_build_inputs = collect_build_provenance_inputs(
        repository_root, cargo_profile=cargo_profile
    )
    if post_build_inputs != provenance_inputs:
        raise BenchmarkFailure(
            "benchmark source, build inputs, toolchain, or compiler flags "
            "changed while Cargo was building"
        )
    binary, cargo_artifact_profile = discover_test_binary_artifact(
        completed.stdout
    )
    binary = binary.resolve()
    provenance_inputs["inputs"]["effective_profile"][
        "cargo_artifact_profile"
    ] = cargo_artifact_profile
    provenance = {
        "schema_version": PROVENANCE_SCHEMA,
        "kind": "clonk-network-load-build-provenance",
        "created_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "binary": binary_metadata(binary),
        "source": provenance_inputs["source"],
        "content": provenance_inputs["content"],
        "inputs": provenance_inputs["inputs"],
        "toolchain": provenance_inputs["toolchain"],
        "environment": provenance_inputs["environment"],
        "build": {
            "cargo_profile": cargo_profile,
            "command": command,
            "runner_contract_version": RUNNER_SCHEMA,
            "runner_script_sha256": runner_script_sha256(),
        },
    }
    sidecar = provenance_sidecar_path(binary)
    write_json(sidecar, provenance)
    metadata = {
        "mode": "cargo-build-once",
        "command": command,
        "cargo_profile": cargo_profile,
        "started_at_utc": started_at.isoformat(),
        "elapsed_seconds": elapsed_seconds,
        "cargo_stderr": completed.stderr,
        "binary": binary_metadata(binary),
        "binary_sha256": sha256_file(binary),
        "provenance_sidecar": str(sidecar),
        "provenance_sha256": sha256_file(sidecar),
        "provenance": provenance,
    }
    return binary, metadata


def _load_report(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BenchmarkFailure(f"could not read report {path}: {error}") from error
    if not isinstance(value, dict):
        raise BenchmarkFailure(f"network-load report is not an object: {path}")
    return value


def _initialize_cohort(
    *,
    binary: Path,
    output_directory: Path,
    label: str,
    runs: int,
    topology: str,
    measurement_seconds: int | None,
    timeout_seconds: int,
    build: dict[str, Any],
    experiment: dict[str, Any] | None = None,
) -> dict[str, Any]:
    label = require_safe_label(label)
    output_directory = output_directory.resolve()
    output_directory.mkdir(parents=True, exist_ok=False)
    source_binary_details = binary_metadata(binary)
    retained_binary = output_directory / (
        f"{label}-benchmark-binary{binary.suffix}"
    )
    if retained_binary.resolve().parent != output_directory:
        raise BenchmarkFailure(
            "retained benchmark binary escapes its cohort directory"
        )
    shutil.copy2(binary, retained_binary)
    details = binary_metadata(retained_binary)
    if details["sha256"] != source_binary_details["sha256"]:
        raise BenchmarkFailure("retained benchmark binary changed while copying")
    provenance = build.get("provenance")
    if isinstance(provenance, dict):
        write_json(output_directory / "build-provenance.json", provenance)
    runtime_machine = runtime_machine_fingerprint()
    configuration = {
        "requested_runs": runs,
        "topology": topology,
        "measurement_seconds": measurement_seconds,
        "authoritative_default_duration": measurement_seconds is None,
        "process_timeout_seconds": timeout_seconds,
        "test_name": TEST_NAME,
    }
    metadata = {
        "schema_version": RUNNER_SCHEMA,
        "kind": "clonk-network-load-benchmark-cohort",
        "created_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "label": label,
        "configuration": configuration,
        "binary": details,
        "build": build,
        "runtime_machine": runtime_machine,
        "methodology_notes": [WARMUP_NOTE],
    }
    if experiment is not None:
        metadata["experiment"] = experiment
    write_json(
        output_directory / "cohort-metadata.json",
        metadata,
    )
    return {
        "directory": output_directory,
        "binary_details": details,
        "binary": retained_binary,
        "build": build,
        "runtime_machine": runtime_machine,
        "experiment": experiment,
        "reports": [],
        "records": [],
    }


def _execute_cohort_run(
    *,
    state: dict[str, Any],
    binary: Path,
    run_index: int,
    repository_root: Path,
    topology: str,
    measurement_seconds: int | None,
    timeout_seconds: int,
    experiment_step: dict[str, Any] | None = None,
) -> None:
    run_name = f"run-{run_index:03d}"
    result = run_one(
        binary=binary,
        run_directory=state["directory"] / run_name,
        repository_root=repository_root,
        topology=topology,
        measurement_seconds=measurement_seconds,
        expected_binary_sha256=state["binary_details"]["sha256"],
        timeout_seconds=timeout_seconds,
        experiment=state["experiment"],
        experiment_step=experiment_step,
    )
    failure = result["execution"]["failure"]
    if not result["passed"] and failure is None:
        failure = "benchmark process did not produce a successful report"
    if result["passed"]:
        try:
            report = _load_report(Path(result["report_path"]))
            validate_report(
                report,
                expected_topology=topology,
                expected_measurement_seconds=measurement_seconds,
            )
            validate_report_against_build(report, state["build"])
        except BenchmarkFailure as error:
            failure = str(error)
        else:
            state["reports"].append(report)
    report_path = Path(result["report_path"])
    execution_path = state["directory"] / run_name / "execution.json"
    state["records"].append(
        {
            "run": run_index,
            "directory": run_name,
            "passed": failure is None,
            "failure": failure,
            "report_sha256": (
                sha256_file(report_path) if report_path.is_file() else None
            ),
            "execution_sha256": (
                sha256_file(execution_path) if execution_path.is_file() else None
            ),
            "binary_sha256": result["execution"].get("binary_sha256"),
            **(
                {
                    "pair_index": experiment_step["pair_index"],
                    "pair_order": experiment_step["order"],
                    "pair_position": experiment_step["position"],
                    "global_sequence": experiment_step["global_sequence"],
                }
                if experiment_step is not None
                else {}
            ),
        }
    )


def _finalize_cohort(
    *,
    state: dict[str, Any],
    label: str,
    runs: int,
    topology: str,
    measurement_seconds: int | None = None,
) -> dict[str, Any]:
    state["records"].sort(key=lambda record: record["run"])
    cohort_failure = None
    identity = None
    metrics = None
    expected_run_numbers = list(range(1, runs + 1))
    observed_run_numbers = [record["run"] for record in state["records"]]
    retained_binary = state.get("binary")
    if (
        isinstance(retained_binary, Path)
        and (
            not retained_binary.is_file()
            or sha256_file(retained_binary)
            != state["binary_details"]["sha256"]
        )
    ):
        cohort_failure = "retained benchmark binary changed before finalization"
    if observed_run_numbers != expected_run_numbers:
        cohort_failure = (
            "benchmark cohort run records differ from requested runs: "
            f"observed {observed_run_numbers}, expected {expected_run_numbers}"
        )
    elif len(state["reports"]) != runs:
        cohort_failure = (
            f"benchmark cohort expected {runs} successful reports, "
            f"observed {len(state['reports'])}"
        )
    if state["reports"]:
        try:
            identity = validate_report_set(
                state["reports"],
                expected_topology=topology,
                expected_measurement_seconds=measurement_seconds,
            )
        except BenchmarkFailure as error:
            cohort_failure = "; ".join(
                part for part in (cohort_failure, str(error)) if part
            )
        else:
            metrics = summarize_reports(state["reports"])
    elif cohort_failure is None:
        cohort_failure = "benchmark cohort has no successful reports"
    failed_runs = sum(not record["passed"] for record in state["records"])
    summary = {
        "schema_version": RUNNER_SCHEMA,
        "kind": "clonk-network-load-benchmark-summary",
        "label": label,
        "result": (
            "pass" if failed_runs == 0 and cohort_failure is None else "fail"
        ),
        "requested_runs": runs,
        "successful_runs": len(state["reports"]),
        "failed_runs": failed_runs,
        "cohort_failure": cohort_failure,
        "binary": state["binary_details"],
        "build": state["build"],
        "runtime_machine": state["runtime_machine"],
        "experiment": state.get("experiment"),
        "report_identity": identity,
        "metrics": metrics,
        "runs": state["records"],
    }
    write_json(state["directory"] / "benchmark-summary.json", summary)
    return summary


def run_cohort(
    *,
    binary: Path,
    output_directory: Path,
    repository_root: Path,
    label: str,
    runs: int,
    topology: str,
    measurement_seconds: int | None,
    timeout_seconds: int,
    build: dict[str, Any],
) -> dict[str, Any]:
    """Run one fixed binary repeatedly and retain every process artifact."""

    if runs < 1:
        raise BenchmarkFailure("run count must be positive")
    binary = binary.resolve()
    state = _initialize_cohort(
        binary=binary,
        output_directory=output_directory,
        label=label,
        runs=runs,
        topology=topology,
        measurement_seconds=measurement_seconds,
        timeout_seconds=timeout_seconds,
        build=build,
    )
    for run_index in range(1, runs + 1):
        _execute_cohort_run(
            state=state,
            binary=state["binary"],
            run_index=run_index,
            repository_root=repository_root,
            topology=topology,
            measurement_seconds=measurement_seconds,
            timeout_seconds=timeout_seconds,
        )
    return _finalize_cohort(
        state=state,
        label=label,
        runs=runs,
        topology=topology,
        measurement_seconds=measurement_seconds,
    )


def run_paired_binaries(
    *,
    baseline_binary: Path,
    candidate_binary: Path,
    output_directory: Path,
    repository_root: Path,
    runs: int,
    topology: str,
    measurement_seconds: int | None,
    timeout_seconds: int,
    cargo_profile: str,
) -> dict[str, Any]:
    """Run two prebuilt binaries in an AB/BA counterbalanced schedule."""

    output_directory.mkdir(parents=True, exist_ok=False)
    experiment_manifest = paired_experiment_manifest(
        pair_count=runs,
        topology=topology,
        measurement_seconds=measurement_seconds,
        timeout_seconds=timeout_seconds,
        cargo_profile=cargo_profile,
    )
    write_json(
        output_directory / "experiment-manifest.json", experiment_manifest
    )
    experiment = paired_experiment_binding(experiment_manifest)
    binaries = {
        "baseline": baseline_binary.resolve(),
        "candidate": candidate_binary.resolve(),
    }
    states: dict[str, dict[str, Any]] = {}
    for label, binary in binaries.items():
        build = prebuilt_build_record(binary, cargo_profile)
        states[label] = _initialize_cohort(
            binary=binary,
            output_directory=output_directory / label,
            label=label,
            runs=runs,
            topology=topology,
            measurement_seconds=measurement_seconds,
            timeout_seconds=timeout_seconds,
            build=build,
            experiment=experiment,
        )

    require_comparable_builds(
        states["baseline"]["build"], states["candidate"]["build"]
    )
    require_authoritative_source_evidence(
        [states["baseline"]["build"], states["candidate"]["build"]],
        pair_count=runs,
    )

    for experiment_step in experiment_manifest["schedule"]:
        label = experiment_step["label"]
        run_index = experiment_step["run"]
        _execute_cohort_run(
            state=states[label],
            binary=states[label]["binary"],
            run_index=run_index,
            repository_root=repository_root,
            topology=topology,
            measurement_seconds=measurement_seconds,
            timeout_seconds=timeout_seconds,
            experiment_step=experiment_step,
        )

    summaries = {
        label: _finalize_cohort(
            state=state,
            label=label,
            runs=runs,
            topology=topology,
            measurement_seconds=measurement_seconds,
        )
        for label, state in states.items()
    }

    if any(summary["result"] != "pass" for summary in summaries.values()):
        comparison = {
            "schema_version": RUNNER_SCHEMA,
            "kind": "clonk-network-load-benchmark-comparison",
            "result": "fail",
            "comparison_valid": False,
            "statistically_valid": False,
            "meets_target": None,
            "target_result": "invalid",
            "failure": "one or both benchmark cohorts failed",
            "cohorts": summaries,
        }
    else:
        comparison = compare_cohort_directories(
            states["baseline"]["directory"],
            states["candidate"]["directory"],
            expected_topology=topology,
            expected_runs=runs,
            expected_measurement_seconds=measurement_seconds,
            expected_timeout_seconds=timeout_seconds,
            expected_cargo_profile=cargo_profile,
        )
    write_json(output_directory / "comparison.json", comparison)
    return comparison


def nearest_rank_percentile(
    samples: Sequence[int | float], quantile: float
) -> int | float:
    """Return the nearest-rank percentile used by the Rust load report."""

    if not samples:
        raise ValueError("cannot summarize an empty sample series")
    if not 0.0 <= quantile <= 1.0:
        raise ValueError(f"quantile must be between zero and one: {quantile}")
    ordered = sorted(samples)
    rank = max(1, math.ceil(quantile * len(ordered)))
    return ordered[rank - 1]


def bootstrap_median_interval(
    samples: Sequence[int | float], *, seed_label: str
) -> dict[str, Any]:
    """Return a deterministic percentile-bootstrap 95% interval."""

    if not samples:
        raise ValueError("cannot bootstrap an empty sample series")
    seed = int.from_bytes(
        hashlib.sha256(
            (seed_label + repr(tuple(samples))).encode("utf-8")
        ).digest()[:8],
        "big",
    )
    generator = random.Random(seed)
    sample_count = len(samples)
    bootstrap_medians = [
        statistics.median(
            samples[generator.randrange(sample_count)]
            for _ in range(sample_count)
        )
        for _ in range(BOOTSTRAP_RESAMPLES)
    ]
    return {
        "lower": nearest_rank_percentile(bootstrap_medians, 0.025),
        "upper": nearest_rank_percentile(bootstrap_medians, 0.975),
        "resamples": BOOTSTRAP_RESAMPLES,
    }


def bootstrap_ratio_interval(
    baseline: Sequence[int],
    candidate: Sequence[int],
    *,
    seed_label: str,
) -> dict[str, Any] | None:
    """Bootstrap the ratio of independent cohort medians."""

    seed = int.from_bytes(
        hashlib.sha256(
            (
                seed_label
                + repr(tuple(baseline))
                + repr(tuple(candidate))
            ).encode("utf-8")
        ).digest()[:8],
        "big",
    )
    generator = random.Random(seed)
    baseline_count = len(baseline)
    candidate_count = len(candidate)
    ratios = []
    for _ in range(BOOTSTRAP_RESAMPLES):
        baseline_median = statistics.median(
            baseline[generator.randrange(baseline_count)]
            for _ in range(baseline_count)
        )
        candidate_median = statistics.median(
            candidate[generator.randrange(candidate_count)]
            for _ in range(candidate_count)
        )
        if baseline_median == 0:
            return None
        ratios.append(candidate_median / baseline_median)
    return {
        "lower": nearest_rank_percentile(ratios, 0.025),
        "upper": nearest_rank_percentile(ratios, 0.975),
        "resamples": BOOTSTRAP_RESAMPLES,
    }


def one_sided_sign_test_pvalue(successes: int, trials: int) -> float:
    """Exact P(X >= successes) for X ~ Binomial(trials, 0.5)."""

    return sum(
        math.comb(trials, value) for value in range(successes, trials + 1)
    ) / (2**trials)


def exact_paired_median_interval(
    paired_ratios: Sequence[int | float],
    *,
    minimum_confidence: float = 0.95,
) -> dict[str, Any]:
    """Distribution-free two-sided interval for a population median."""

    if not paired_ratios:
        raise ValueError("cannot interval an empty paired-ratio series")
    ordered = sorted(paired_ratios)
    trials = len(ordered)
    selected_rank = 1
    selected_confidence = 1.0 - 2.0 / (2**trials)
    for rank in range(1, trials // 2 + 1):
        tail_probability = sum(
            math.comb(trials, value) for value in range(rank)
        ) / (2**trials)
        confidence = 1.0 - 2.0 * tail_probability
        if confidence >= minimum_confidence:
            selected_rank = rank
            selected_confidence = confidence
        else:
            break
    upper_rank = trials - selected_rank + 1
    return {
        "lower": ordered[selected_rank - 1],
        "upper": ordered[upper_rank - 1],
        "confidence_level": selected_confidence,
        "lower_order_statistic": selected_rank,
        "upper_order_statistic": upper_rank,
        "method": "exact-binomial-order-statistic",
    }


def summarize_reports(reports: Sequence[dict[str, Any]]) -> dict[str, Any]:
    """Keep runs independent while also describing their pooled raw samples."""

    summary: dict[str, Any] = {}
    for metric_name in PRIMARY_METRICS:
        units = {report[metric_name]["unit"] for report in reports}
        if len(units) != 1:
            raise ValueError(f"{metric_name} units differ between runs")
        per_run = [
            nearest_rank_percentile(report[metric_name]["raw_samples"], 0.5)
            for report in reports
        ]
        pooled = [
            sample
            for report in reports
            for sample in report[metric_name]["raw_samples"]
        ]
        run_median = statistics.median(per_run)
        summary[metric_name] = {
            "unit": units.pop(),
            "statistical_unit": "one complete benchmark process",
            "run_statistic": "per-run raw-sample p50",
            "independent_run_count": len(per_run),
            "independent_run_values": per_run,
            "independent_run_median": run_median,
            "independent_run_median_absolute_deviation": statistics.median(
                abs(value - run_median) for value in per_run
            ),
            "independent_run_median_bootstrap_95ci": (
                bootstrap_median_interval(per_run, seed_label=metric_name)
            ),
            "pooled_sample_count": len(pooled),
            "pooled_p50": nearest_rank_percentile(pooled, 0.5),
            "pooled_summary": {
                "samples": len(pooled),
                "minimum": min(pooled),
                "p50": nearest_rank_percentile(pooled, 0.5),
                "p95": nearest_rank_percentile(pooled, 0.95),
                "p99": nearest_rank_percentile(pooled, 0.99),
                "maximum": max(pooled),
            },
            "pooled_samples_are_independent": False,
        }
    return summary


def _require_fields(
    value: dict[str, Any], fields: Sequence[str], context: str
) -> None:
    missing = [field for field in fields if field not in value]
    if missing:
        raise BenchmarkFailure(f"{context} is missing: {', '.join(missing)}")


def report_identity(report: dict[str, Any]) -> dict[str, Any]:
    """Return all workload and environment fields required for comparison."""

    _require_fields(report, REPORT_IDENTITY_FIELDS, "network-load report")
    fingerprint = report.get("fingerprint")
    if not isinstance(fingerprint, dict):
        raise BenchmarkFailure("network-load report fingerprint is missing")
    _require_fields(fingerprint, FINGERPRINT_FIELDS, "report fingerprint")
    for field in FINGERPRINT_FIELDS:
        if field == "source_dirty":
            if not isinstance(fingerprint[field], bool):
                raise BenchmarkFailure(
                    "report fingerprint source_dirty is not a boolean"
                )
        elif field in {
            "source_commit",
            "content_revision",
            "rustc",
            "cpu",
            "os_version",
        } and fingerprint[field] is None:
            continue
        elif not isinstance(fingerprint[field], str) or not fingerprint[field]:
            raise BenchmarkFailure(
                f"report fingerprint {field} is missing or empty"
            )
    return {
        "report": {field: report[field] for field in REPORT_IDENTITY_FIELDS},
        "fingerprint": {field: fingerprint[field] for field in FINGERPRINT_FIELDS},
    }


def recompute_metric_summary(samples: Sequence[int]) -> dict[str, Any]:
    return {
        "samples": len(samples),
        "p50": nearest_rank_percentile(samples, 0.50),
        "p95": nearest_rank_percentile(samples, 0.95),
        "p99": nearest_rank_percentile(samples, 0.99),
        "max": max(samples),
    }


def validate_metric_series(
    metric: Any, *, expected_unit: str, context: str
) -> list[int]:
    if not isinstance(metric, dict):
        raise BenchmarkFailure(f"network-load report has no {context}")
    if metric.get("unit") != expected_unit:
        raise BenchmarkFailure(
            f"{context} unit is {metric.get('unit')!r}, expected "
            f"{expected_unit!r}"
        )
    samples = metric.get("raw_samples")
    if not isinstance(samples, list) or not samples:
        raise BenchmarkFailure(f"{context} has no raw samples")
    if any(
        not isinstance(sample, int)
        or isinstance(sample, bool)
        or sample < 0
        for sample in samples
    ):
        raise BenchmarkFailure(f"{context} contains an invalid latency sample")
    expected_summary = recompute_metric_summary(samples)
    observed_summary = metric.get("summary")
    if (
        not isinstance(observed_summary, dict)
        or set(observed_summary) != set(expected_summary)
        or any(
            type(observed_summary[field]) is not int
            or observed_summary[field] != expected
            for field, expected in expected_summary.items()
        )
    ):
        raise BenchmarkFailure(f"{context} summary differs from raw samples")
    return samples


def validate_runtime_samples(
    value: Any, *, participant_count: int
) -> int:
    """Validate complete host/client telemetry groups and return wait count."""

    if (
        not isinstance(value, list)
        or not value
        or len(value) % participant_count != 0
    ):
        raise BenchmarkFailure(
            "runtime_samples must contain complete 25-process groups"
        )
    fields = {
        "elapsed_ms",
        "process_client_id",
        "route_count",
        "tcp_input_rate",
        "tcp_output_rate",
        "udp_input_rate",
        "udp_output_rate",
    }
    previous_elapsed_ms = -1
    for offset in range(0, len(value), participant_count):
        group = value[offset : offset + participant_count]
        if any(not isinstance(sample, dict) for sample in group):
            raise BenchmarkFailure("runtime_samples contains an invalid sample")
        if [sample.get("process_client_id") for sample in group] != list(
            range(participant_count)
        ):
            raise BenchmarkFailure(
                "runtime_samples process IDs must be exactly 0 through 24"
            )
        elapsed_values = [sample.get("elapsed_ms") for sample in group]
        if any(
            not isinstance(elapsed_ms, int)
            or isinstance(elapsed_ms, bool)
            or elapsed_ms < 0
            for elapsed_ms in elapsed_values
        ):
            raise BenchmarkFailure(
                "runtime_samples group elapsed time is invalid"
            )
        elapsed_ms = elapsed_values[0]
        if any(value != elapsed_ms for value in elapsed_values[1:]):
            raise BenchmarkFailure(
                "runtime_samples group elapsed times differ"
            )
        if elapsed_ms < previous_elapsed_ms:
            raise BenchmarkFailure(
                "runtime_samples group elapsed time is invalid"
            )
        previous_elapsed_ms = elapsed_ms
        for sample in group:
            if set(sample) != fields:
                raise BenchmarkFailure(
                    "runtime_samples fields differ from the report contract"
                )
            numeric_fields = fields - {"process_client_id"}
            if any(
                not isinstance(sample[field], int)
                or isinstance(sample[field], bool)
                or sample[field] < 0
                for field in numeric_fields
            ):
                raise BenchmarkFailure(
                    "runtime_samples contains an invalid numeric value"
                )
    return len(value) * participant_count


def validate_report(
    report: dict[str, Any],
    *,
    expected_topology: str,
    expected_measurement_seconds: int | None = None,
) -> dict[str, Any]:
    """Validate the existing socket harness's measurement contract."""

    identity = report_identity(report)
    if report["schema_version"] != SUPPORTED_REPORT_SCHEMA:
        raise BenchmarkFailure(
            "unsupported network-load report schema: "
            f"{report['schema_version']} (expected {SUPPORTED_REPORT_SCHEMA})"
        )
    if report["topology"] != expected_topology:
        raise BenchmarkFailure(
            f"report topology is {report['topology']!r}, expected "
            f"{expected_topology!r}"
        )
    for field, expected in EXPECTED_REPORT_VALUES.items():
        if report[field] != expected:
            raise BenchmarkFailure(
                f"report {field} is {report[field]!r}, expected {expected!r}"
            )
    measurement_seconds = (
        DEFAULT_MEASUREMENT_SECONDS
        if expected_measurement_seconds is None
        else expected_measurement_seconds
    )
    duration_values = {
        "requested_measurement_ms": measurement_seconds * 1_000,
        "minimum_native_control_ticks": math.ceil(
            measurement_seconds * 1_000 / EXPECTED_REPORT_VALUES[
                "native_control_interval_ms"
            ]
        ),
        "authoritative_duration": (
            measurement_seconds >= DEFAULT_MEASUREMENT_SECONDS
        ),
        "preferred_message_protocol": (
            "udp" if expected_topology == "udp" else "tcp"
        ),
    }
    for field, expected in duration_values.items():
        if report[field] != expected:
            raise BenchmarkFailure(
                f"report {field} is {report[field]!r}, expected {expected!r}"
            )
    measurement_wall_elapsed_ms = report.get("measurement_wall_elapsed_ms")
    measurement_interval_ms = EXPECTED_REPORT_VALUES[
        "native_control_interval_ms"
    ]
    minimum_wall_elapsed_ms = max(
        0, report["requested_measurement_ms"] - measurement_interval_ms
    )
    maximum_wall_elapsed_ms = (
        report["requested_measurement_ms"] + measurement_interval_ms
    )
    if (
        not isinstance(measurement_wall_elapsed_ms, int)
        or isinstance(measurement_wall_elapsed_ms, bool)
        or not minimum_wall_elapsed_ms
        <= measurement_wall_elapsed_ms
        <= maximum_wall_elapsed_ms
    ):
        raise BenchmarkFailure(
            "measurement wall elapsed is outside the requested duration plus "
            "or minus one native control interval"
        )
    if report.get("result") != "pass":
        raise BenchmarkFailure(
            f"network-load report result is {report.get('result')!r}"
        )
    assertions = report.get("assertions")
    if not isinstance(assertions, list) or not assertions:
        raise BenchmarkFailure("network-load report has no assertions")
    if any(not isinstance(assertion, dict) for assertion in assertions):
        raise BenchmarkFailure("network-load report contains an invalid assertion")
    if any(assertion.get("passed") is not True for assertion in assertions):
        raise BenchmarkFailure("network-load report contains a failed assertion")
    assertion_names = [assertion.get("name") for assertion in assertions]
    required_assertion_names = expected_assertion_names()
    if (
        len(assertion_names) != len(required_assertion_names)
        or set(assertion_names) != set(required_assertion_names)
    ):
        raise BenchmarkFailure(
            "network-load report assertion names differ from the complete "
            f"harness contract: observed {assertion_names!r}, "
            f"expected {required_assertion_names!r}"
        )
    for metric_name, expected_unit in REPORT_METRIC_SPECS:
        validate_metric_series(
            report.get(metric_name),
            expected_unit=expected_unit,
            context=metric_name,
        )
    measured_ticks = report.get("measured_ticks")
    expected_ready_deliveries = report.get("expected_ready_deliveries")
    observed_ready_deliveries = report.get("observed_ready_deliveries")
    if (
        not isinstance(measured_ticks, int)
        or isinstance(measured_ticks, bool)
        or measured_ticks != report["minimum_native_control_ticks"]
        or expected_ready_deliveries
        != measured_ticks * report["active_control_participants"]
        or observed_ready_deliveries != expected_ready_deliveries
    ):
        raise BenchmarkFailure(
            "ready delivery counts are inconsistent with measured ticks"
        )
    metric_sample_counts = {
        "join_duration": report["player_profiles_joined"],
        "control_completion_wait": measured_ticks,
        "client_to_host_isolated_application_round_trip": report[
            "isolated_application_round_trip_samples"
        ],
        "participant_ready": observed_ready_deliveries,
        "cadence_lateness": measured_ticks,
    }
    for metric_name, expected_count in metric_sample_counts.items():
        observed_count = len(report[metric_name]["raw_samples"])
        if observed_count != expected_count:
            raise BenchmarkFailure(
                f"{metric_name} has {observed_count} raw samples, expected "
                f"{expected_count} from the measured workload"
            )
    expected_native_wait_count = validate_runtime_samples(
        report.get("runtime_samples"),
        participant_count=report["active_control_participants"],
    )
    observed_native_wait_count = len(
        report["native_control_wait"]["raw_samples"]
    )
    if observed_native_wait_count != expected_native_wait_count:
        raise BenchmarkFailure(
            "native_control_wait has "
            f"{observed_native_wait_count} raw samples, expected "
            f"{expected_native_wait_count} from runtime telemetry groups"
        )
    expected_direct_tcp_mesh = expected_topology == "tcp"
    if report.get("direct_tcp_mesh") is not expected_direct_tcp_mesh:
        raise BenchmarkFailure(
            "direct_tcp_mesh is inconsistent with the selected topology"
        )
    mesh_establishment_us = report.get("mesh_establishment_us")
    if expected_topology == "relay":
        valid_mesh_establishment = mesh_establishment_us is None
    else:
        valid_mesh_establishment = (
            isinstance(mesh_establishment_us, int)
            and not isinstance(mesh_establishment_us, bool)
            and mesh_establishment_us >= 0
        )
    if not valid_mesh_establishment:
        raise BenchmarkFailure(
            "mesh_establishment_us is inconsistent with the selected topology"
        )
    if report.get("final_route_peers") != expected_route_peers(expected_topology):
        raise BenchmarkFailure(
            "final route peers differ from the exact "
            f"{expected_topology} topology"
        )
    if report.get(
        "final_preferred_message_routes"
    ) != expected_preferred_message_routes(expected_topology):
        raise BenchmarkFailure(
            "final preferred message routes differ from the exact "
            f"{expected_topology} topology"
        )
    if report.get(
        "isolated_application_round_trip_preferred_message_routes"
    ) != expected_isolated_preferred_message_routes(expected_topology):
        raise BenchmarkFailure(
            "isolated application RTT preferred message routes differ from "
            f"the exact {expected_topology} topology"
        )
    native_series = report.get("client_to_host_round_trip_by_client")
    if not isinstance(native_series, list) or len(native_series) != 24:
        raise BenchmarkFailure(
            "client_to_host_round_trip_by_client must contain 24 client series"
        )
    if any(not isinstance(series, dict) for series in native_series):
        raise BenchmarkFailure("native round-trip client series is invalid")
    native_client_ids = [series.get("client_id") for series in native_series]
    if native_client_ids != list(range(1, 25)):
        raise BenchmarkFailure(
            "native round-trip client IDs must be exactly 1 through 24"
        )
    native_samples = []
    for series in native_series:
        native_samples.extend(
            validate_metric_series(
                series.get("metrics"),
                expected_unit="milliseconds",
                context=f"client {series['client_id']} native RTT",
            )
        )
    if report["client_to_host_round_trip"]["raw_samples"] != native_samples:
        raise BenchmarkFailure(
            "aggregate native RTT samples differ from the 24 client series"
        )
    application_series = report.get(
        "client_to_host_application_round_trip_by_client"
    )
    if not isinstance(application_series, list) or len(application_series) != 24:
        raise BenchmarkFailure(
            "client_to_host_application_round_trip_by_client must contain "
            "24 client series"
        )
    if any(not isinstance(series, dict) for series in application_series):
        raise BenchmarkFailure("application round-trip client series is invalid")
    client_ids = [series.get("client_id") for series in application_series]
    if client_ids != list(range(1, 25)):
        raise BenchmarkFailure(
            "application round-trip client IDs must be exactly 1 through 24"
        )
    application_samples = []
    for series in application_series:
        metrics = series.get("metrics")
        if not isinstance(metrics, dict):
            raise BenchmarkFailure(
                f"client {series['client_id']} application RTT samples are invalid"
            )
        samples = metrics.get("raw_samples")
        if not isinstance(samples, list) or len(samples) != EXPECTED_REPORT_VALUES[
            "application_round_trip_rounds_per_client"
        ]:
            raise BenchmarkFailure(
                f"client {series['client_id']} application RTT samples are invalid"
            )
        samples = validate_metric_series(
            metrics,
            expected_unit="microseconds",
            context=f"client {series['client_id']} application RTT",
        )
        application_samples.extend(samples)
    aggregate_application_samples = report[
        "client_to_host_application_round_trip"
    ]["raw_samples"]
    if aggregate_application_samples != application_samples:
        raise BenchmarkFailure(
            "aggregate application RTT samples differ from the 24 client series"
        )
    return identity


def validate_report_against_build(
    report: dict[str, Any], build: dict[str, Any]
) -> None:
    expected_commit = build.get("provenance", {}).get("source", {}).get(
        "commit"
    )
    if not isinstance(expected_commit, str) or not expected_commit:
        raise BenchmarkFailure("benchmark build provenance has no source commit")
    content_head = build.get("provenance", {}).get("content", {}).get("head")
    if not isinstance(content_head, str) or not content_head:
        raise BenchmarkFailure("benchmark build provenance has no content revision")
    cargo_artifact_profile = (
        build.get("provenance", {})
        .get("inputs", {})
        .get("effective_profile", {})
        .get("cargo_artifact_profile")
    )
    if not isinstance(cargo_artifact_profile, dict) or not isinstance(
        cargo_artifact_profile.get("debug_assertions"), bool
    ):
        raise BenchmarkFailure(
            "benchmark build provenance has no Cargo debug-assertion state"
        )
    expected_profile_label = (
        "test-with-debug-assertions"
        if cargo_artifact_profile["debug_assertions"]
        else "test"
    )
    if (
        report.get("fingerprint", {}).get("cargo_profile")
        != expected_profile_label
    ):
        raise BenchmarkFailure(
            "report diagnostic Cargo profile label differs from the "
            "authoritative Cargo artifact profile"
        )
    # The compiled harness discovers its source checkout through its build-time
    # CARGO_MANIFEST_DIR. That checkout may be moved, deleted, or modified by
    # the time a preserved binary runs. The binary-bound sidecar above is the
    # authoritative source identity; the report's runtime Git probe is only a
    # diagnostic.


def validate_report_set(
    reports: Sequence[dict[str, Any]],
    *,
    expected_topology: str,
    expected_measurement_seconds: int | None = None,
) -> dict[str, Any]:
    """Require every successful observation in a cohort to be comparable."""

    if not reports:
        raise BenchmarkFailure("benchmark cohort has no successful reports")
    identities = [
        validate_report(
            report,
            expected_topology=expected_topology,
            expected_measurement_seconds=expected_measurement_seconds,
        )
        for report in reports
    ]
    first = identities[0]
    for index, identity in enumerate(identities[1:], start=2):
        if comparison_identity(identity) != comparison_identity(first):
            raise BenchmarkFailure(f"run {index} fingerprint differs from run 1")
    return first


def comparison_identity(identity: dict[str, Any]) -> dict[str, Any]:
    """Drop source-state fields that must differ across candidate binaries."""

    fingerprint = {
        field: value
        for field, value in identity["fingerprint"].items()
        if field
        not in {"source_commit", "source_dirty", "content_revision", "rustc"}
    }
    return {"report": identity["report"], "fingerprint": fingerprint}


def compare_report_sets(
    baseline_reports: Sequence[dict[str, Any]],
    candidate_reports: Sequence[dict[str, Any]],
    *,
    expected_topology: str,
    expected_measurement_seconds: int | None = None,
    paired: bool = False,
    verified_paired_experiment: bool = False,
) -> dict[str, Any]:
    """Compare cohort-level medians without treating in-run samples as runs."""

    if verified_paired_experiment and not paired:
        raise BenchmarkFailure(
            "a verified paired experiment requires paired report sets"
        )

    baseline_identity = validate_report_set(
        baseline_reports,
        expected_topology=expected_topology,
        expected_measurement_seconds=expected_measurement_seconds,
    )
    candidate_identity = validate_report_set(
        candidate_reports,
        expected_topology=expected_topology,
        expected_measurement_seconds=expected_measurement_seconds,
    )
    if comparison_identity(baseline_identity) != comparison_identity(
        candidate_identity
    ):
        raise BenchmarkFailure(
            "baseline and candidate workload/environment fingerprints differ"
        )

    baseline = summarize_reports(baseline_reports)
    candidate = summarize_reports(candidate_reports)
    if paired and len(baseline_reports) != len(candidate_reports):
        raise BenchmarkFailure(
            "paired comparison requires equal independent run counts"
        )
    metrics = {}
    statistically_valid = (
        paired
        and verified_paired_experiment
        and len(baseline_reports) == AUTHORITATIVE_PAIR_COUNT
        and len(candidate_reports) == AUTHORITATIVE_PAIR_COUNT
        and baseline_identity["report"]["authoritative_duration"] is True
        and candidate_identity["report"]["authoritative_duration"] is True
    )
    for metric_name in PRIMARY_METRICS:
        baseline_median = baseline[metric_name]["independent_run_median"]
        candidate_median = candidate[metric_name]["independent_run_median"]
        baseline_values = baseline[metric_name]["independent_run_values"]
        candidate_values = candidate[metric_name]["independent_run_values"]
        paired_ratios = None
        if paired:
            paired_ratios = (
                [
                    candidate_value / baseline_value
                    for baseline_value, candidate_value in zip(
                        baseline_values, candidate_values, strict=True
                    )
                ]
                if all(value != 0 for value in baseline_values)
                else None
            )
            ratio = (
                statistics.median(paired_ratios)
                if paired_ratios is not None
                else None
            )
            ratio_interval = (
                bootstrap_median_interval(
                    paired_ratios, seed_label=f"paired-ratio:{metric_name}"
                )
                if paired_ratios is not None
                else None
            )
        else:
            ratio = (
                candidate_median / baseline_median
                if baseline_median != 0
                else None
            )
            ratio_interval = (
                bootstrap_ratio_interval(
                    baseline_values,
                    candidate_values,
                    seed_label=f"ratio:{metric_name}",
                )
                if ratio is not None
                else None
            )
        paired_target_successes = (
            sum(value <= TARGET_RATIO for value in paired_ratios)
            if paired_ratios is not None
            else None
        )
        paired_sign_test_pvalue = (
            one_sided_sign_test_pvalue(
                paired_target_successes, len(paired_ratios)
            )
            if paired_target_successes is not None
            else None
        )
        decision_interval = (
            exact_paired_median_interval(paired_ratios)
            if paired_ratios is not None
            else ratio_interval
        )
        is_target_metric = metric_name in TARGET_METRICS
        target_result = (
            "indeterminate" if is_target_metric else "diagnostic-only"
        )
        target_reason = (
            (
                "only a verified direct interleaved paired experiment may "
                "establish the target"
                if not verified_paired_experiment
                else f"exactly {AUTHORITATIVE_PAIR_COUNT} paired runs are required"
            )
            if is_target_metric
            else (
                "native ping is a compatibility diagnostic whose whole-"
                "millisecond quantization cannot establish the latency target; "
                "the isolated application RTT is the target metric"
                if metric_name == "client_to_host_round_trip"
                else (
                    "the loaded 24-client fanout RTT is a diagnostic of the "
                    "loaded session; the fresh isolated application RTT is "
                    "the target metric"
                )
            )
        )
        if not is_target_metric:
            pass
        elif ratio is None or decision_interval is None:
            target_reason = "the baseline median is zero, so no ratio is defined"
        elif not statistically_valid:
            pass
        elif (
            decision_interval["upper"] <= TARGET_RATIO
            and paired_target_successes is not None
            and paired_target_successes >= 15
        ):
            target_result = "met"
            target_reason = (
                "the upper bound of the exact distribution-free paired-median "
                "interval is at or below the target and at least 15 of 20 "
                "pairs meet it"
            )
        elif decision_interval["lower"] > TARGET_RATIO:
            target_result = "not-met"
            target_reason = (
                "the lower bound of the exact distribution-free paired-median "
                "interval is above the target"
            )
        else:
            target_reason = (
                "the paired-median confidence interval crosses the target"
            )
        metrics[metric_name] = {
            "unit": baseline[metric_name]["unit"],
            "comparison_statistic": (
                "median of within-pair candidate/baseline per-run p50 ratios"
                if paired
                else "ratio of cohort medians of per-run raw-sample p50 values"
            ),
            "baseline_independent_run_count": baseline[metric_name][
                "independent_run_count"
            ],
            "candidate_independent_run_count": candidate[metric_name][
                "independent_run_count"
            ],
            "baseline_independent_run_median": baseline_median,
            "candidate_independent_run_median": candidate_median,
            "candidate_to_baseline_ratio": ratio,
            "paired_run_ratios": paired_ratios,
            "paired_target_successes": paired_target_successes,
            "paired_target_successes_required": (
                15 if is_target_metric and paired else None
            ),
            "paired_target_sign_test_one_sided_pvalue": (
                paired_sign_test_pvalue if is_target_metric else None
            ),
            "improvement_percent": None if ratio is None else (1.0 - ratio) * 100.0,
            "baseline_median_absolute_deviation": baseline[metric_name][
                "independent_run_median_absolute_deviation"
            ],
            "candidate_median_absolute_deviation": candidate[metric_name][
                "independent_run_median_absolute_deviation"
            ],
            "candidate_to_baseline_ratio_bootstrap_95ci": ratio_interval,
            "candidate_to_baseline_ratio_95ci": decision_interval,
            "target_ratio": TARGET_RATIO if is_target_metric else None,
            "target_result": target_result,
            "target_reason": target_reason,
            "measurement_resolution": (
                next(
                    resolution
                    for name, _, resolution in METRIC_SPECS
                    if name == metric_name
                )
            ),
        }
    target_results = {
        metrics[metric_name]["target_result"] for metric_name in TARGET_METRICS
    }
    if target_results == {"met"}:
        meets_target: bool | None = True
        target_result = "met"
    elif "not-met" in target_results:
        meets_target = False
        target_result = "not-met"
    else:
        meets_target = None
        target_result = "indeterminate"
    return {
        "comparison_valid": True,
        "statistically_valid": statistically_valid,
        "minimum_independent_runs": MINIMUM_INDEPENDENT_RUNS,
        "target_ratio": TARGET_RATIO,
        "meets_target": meets_target,
        "target_result": target_result,
        "comparison_design": (
            "verified-direct-interleaved-paired"
            if verified_paired_experiment
            else ("unverified-paired-by-index" if paired else "exploratory-cohorts")
        ),
        "same_host_authority": verified_paired_experiment,
        "runtime_fingerprint_role": (
            "descriptive environment evidence; same-host authority comes from "
            "one runner process executing the persisted interleaved schedule"
        ),
        "comparison_identity": comparison_identity(baseline_identity),
        "metrics": metrics,
    }


def load_cohort_reports(
    directory: Path,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Load only run artifacts admitted by a successful cohort summary."""

    directory = directory.resolve()
    summary_path = directory / "benchmark-summary.json"
    metadata_path = directory / "cohort-metadata.json"
    for artifact_path, name in (
        (summary_path, "benchmark summary"),
        (metadata_path, "cohort metadata"),
    ):
        if artifact_path.resolve().parent != directory:
            raise BenchmarkFailure(f"{name} escapes its cohort: {directory}")
        if artifact_path.is_symlink():
            raise BenchmarkFailure(f"{name} is a symlink: {directory}")
    summary = _load_report(summary_path)
    metadata = _load_report(metadata_path)
    for artifact, expected_kind, name in (
        (
            summary,
            "clonk-network-load-benchmark-summary",
            "benchmark summary",
        ),
        (
            metadata,
            "clonk-network-load-benchmark-cohort",
            "cohort metadata",
        ),
    ):
        if artifact.get("schema_version") != RUNNER_SCHEMA:
            raise BenchmarkFailure(f"unsupported {name} schema in {directory}")
        if artifact.get("kind") != expected_kind:
            raise BenchmarkFailure(f"invalid {name} kind in {directory}")
    if summary.get("result") != "pass":
        raise BenchmarkFailure(
            f"cohort did not pass and cannot be compared: {directory}"
        )
    requested_runs = summary.get("requested_runs")
    if not isinstance(requested_runs, int) or isinstance(requested_runs, bool):
        raise BenchmarkFailure(f"cohort requested run count is invalid: {directory}")
    if requested_runs < 1:
        raise BenchmarkFailure(f"cohort requested no runs: {directory}")
    if summary.get("successful_runs") != requested_runs:
        raise BenchmarkFailure(
            f"cohort successful run count differs from requested runs: {directory}"
        )
    if summary.get("failed_runs") != 0:
        raise BenchmarkFailure(f"cohort reports failed runs: {directory}")
    configuration = metadata.get("configuration")
    if not isinstance(configuration, dict):
        raise BenchmarkFailure(f"cohort configuration is missing: {directory}")
    if configuration.get("requested_runs") != requested_runs:
        raise BenchmarkFailure(
            f"cohort metadata run count differs from summary: {directory}"
        )
    for field in ("binary", "build", "runtime_machine"):
        if metadata.get(field) != summary.get(field):
            raise BenchmarkFailure(
                f"cohort {field} differs between metadata and summary: {directory}"
            )
    experiment = summary.get("experiment")
    if metadata.get("experiment") != experiment:
        raise BenchmarkFailure(
            f"cohort experiment differs between metadata and summary: {directory}"
        )
    if experiment is not None:
        validate_experiment_binding(experiment, f"cohort {directory}")
    runtime_machine = summary.get("runtime_machine")
    runtime_fields = (
        "system",
        "release",
        "machine",
        "processor",
        "python_implementation",
        "python_version",
    )
    if not isinstance(runtime_machine, dict) or any(
        not isinstance(runtime_machine.get(field), str)
        or not runtime_machine[field]
        for field in runtime_fields
    ):
        raise BenchmarkFailure(
            f"cohort runtime machine fingerprint is incomplete: {directory}"
        )
    binary = summary.get("binary")
    if not isinstance(binary, dict):
        raise BenchmarkFailure(f"cohort binary metadata is missing: {directory}")
    binary_sha256 = binary.get("sha256")
    _require_hex_digest(binary_sha256, 64, "cohort binary SHA-256")
    retained_binary_value = binary.get("path")
    if not isinstance(retained_binary_value, str) or not retained_binary_value:
        raise BenchmarkFailure(
            f"cohort retained benchmark binary path is missing: {directory}"
        )
    retained_binary = Path(retained_binary_value)
    if retained_binary.is_symlink():
        raise BenchmarkFailure(
            f"cohort retained benchmark binary is a symlink: {directory}"
        )
    if (
        retained_binary.resolve().parent != directory
        or not retained_binary.is_file()
    ):
        raise BenchmarkFailure(
            f"cohort retained benchmark binary is missing: {directory}"
        )
    retained_size = binary.get("size_bytes")
    if (
        not isinstance(retained_size, int)
        or isinstance(retained_size, bool)
        or retained_size != retained_binary.stat().st_size
        or sha256_file(retained_binary) != binary_sha256
    ):
        raise BenchmarkFailure(
            f"cohort retained benchmark binary hash or size differs: {directory}"
        )
    retained_provenance_path = directory / "build-provenance.json"
    if (
        retained_provenance_path.is_symlink()
        or retained_provenance_path.resolve().parent != directory
        or not retained_provenance_path.is_file()
    ):
        raise BenchmarkFailure(
            f"cohort retained build provenance is missing: {directory}"
        )
    retained_provenance = _load_report(retained_provenance_path)
    build_value = summary.get("build")
    if (
        not isinstance(build_value, dict)
        or retained_provenance != build_value.get("provenance")
        or sha256_file(retained_provenance_path)
        != build_value.get("provenance_sha256")
    ):
        raise BenchmarkFailure(
            f"cohort retained build provenance differs: {directory}"
        )
    validate_retained_build(
        build_value,
        binary,
        context=f"cohort {directory}",
    )
    run_records = summary.get("runs")
    if not isinstance(run_records, list) or len(run_records) != requested_runs:
        raise BenchmarkFailure(
            f"cohort run record count differs from requested runs: {directory}"
        )
    if any(not isinstance(record, dict) for record in run_records):
        raise BenchmarkFailure(f"cohort contains an invalid run record: {directory}")
    if any(record.get("passed") is not True for record in run_records):
        raise BenchmarkFailure(f"cohort retains a failed run: {directory}")
    if any(record.get("failure") is not None for record in run_records):
        raise BenchmarkFailure(
            f"cohort passing run record contains a failure: {directory}"
        )
    run_numbers = [record.get("run") for record in run_records]
    if run_numbers != list(range(1, requested_runs + 1)):
        raise BenchmarkFailure(
            f"cohort run numbers are missing, duplicated, or unordered: {directory}"
        )
    expected_directories = {
        f"run-{run_number:03d}" for run_number in range(1, requested_runs + 1)
    }
    recorded_directories = [record.get("directory") for record in run_records]
    if not all(isinstance(name, str) for name in recorded_directories):
        raise BenchmarkFailure(
            f"cohort run directory names are invalid: {directory}"
        )
    if set(recorded_directories) != expected_directories or len(
        set(recorded_directories)
    ) != requested_runs:
        raise BenchmarkFailure(
            "cohort run directory names are missing, duplicated, or invalid: "
            f"{directory}"
        )
    discovered_directories = {
        child.name
        for child in directory.iterdir()
        if child.name.startswith("run-")
    }
    if discovered_directories != expected_directories:
        raise BenchmarkFailure(
            f"cohort run directories differ from summary: {directory}"
        )

    reports = []
    executions = []
    resolved_run_directories: set[Path] = set()
    for record in run_records:
        recorded_run_directory = directory / record["directory"]
        if recorded_run_directory.is_symlink():
            raise BenchmarkFailure(
                f"cohort run directory is a symlink: {recorded_run_directory}"
            )
        run_directory = recorded_run_directory.resolve()
        if run_directory.parent != directory or not run_directory.is_dir():
            raise BenchmarkFailure(
                f"cohort run directory escapes or is missing: {run_directory}"
            )
        if run_directory in resolved_run_directories:
            raise BenchmarkFailure(
                f"cohort run directories resolve to a duplicate: {run_directory}"
            )
        resolved_run_directories.add(run_directory)
        report_path = run_directory / "report.json"
        execution_path = run_directory / "execution.json"
        for field, path in (
            ("report_sha256", report_path),
            ("execution_sha256", execution_path),
        ):
            if path.resolve().parent != run_directory:
                raise BenchmarkFailure(
                    f"cohort run {record['run']} {path.name} escapes its "
                    "run directory"
                )
            if path.is_symlink():
                raise BenchmarkFailure(
                    f"cohort run {record['run']} {path.name} is a symlink"
                )
            recorded_digest = record.get(field)
            _require_hex_digest(
                recorded_digest, 64, f"cohort run {record['run']} {field}"
            )
            if not path.is_file() or sha256_file(path) != recorded_digest:
                raise BenchmarkFailure(
                    f"cohort run {record['run']} {path.name} hash differs"
                )
        if record.get("binary_sha256") != binary_sha256:
            raise BenchmarkFailure(
                f"cohort run {record['run']} binary hash differs from cohort"
            )
        execution = _load_report(execution_path)
        if (
            execution.get("schema_version") != RUNNER_SCHEMA
            or execution.get("kind")
            != "clonk-network-load-benchmark-execution"
        ):
            raise BenchmarkFailure(
                f"cohort run {record['run']} execution metadata is invalid"
            )
        execution_contract = {
            "return_code": 0,
            "timed_out": False,
            "failure": None,
            "report_present": True,
            "report_sha256": record["report_sha256"],
            "binary_sha256": binary_sha256,
            "binary_sha256_before": binary_sha256,
            "binary_sha256_after": binary_sha256,
        }
        for field, expected in execution_contract.items():
            if execution.get(field) != expected:
                raise BenchmarkFailure(
                    f"cohort run {record['run']} execution {field} differs"
                )
        pairing_fields = (
            "pair_index",
            "pair_order",
            "pair_position",
            "global_sequence",
        )
        if experiment is None:
            if any(field in execution or field in record for field in pairing_fields):
                raise BenchmarkFailure(
                    f"cohort run {record['run']} has unbound pairing metadata"
                )
            if any(
                field in execution
                for field in (
                    "experiment_id",
                    "experiment_manifest_sha256",
                )
            ):
                raise BenchmarkFailure(
                    f"cohort run {record['run']} has an unbound experiment"
                )
        else:
            experiment_contract = {
                "experiment_id": experiment["experiment_id"],
                "experiment_manifest_sha256": experiment["manifest_sha256"],
            }
            for field, expected in experiment_contract.items():
                if execution.get(field) != expected:
                    raise BenchmarkFailure(
                        f"cohort run {record['run']} execution {field} differs"
                    )
            for field in pairing_fields:
                if execution.get(field) != record.get(field):
                    raise BenchmarkFailure(
                        f"cohort run {record['run']} execution {field} differs"
                    )
        executions.append(execution)
        reports.append(_load_report(report_path))
    summary["_cohort_metadata"] = metadata
    summary["_executions"] = executions
    return summary, reports


def validate_shared_paired_experiment(
    baseline_summary: dict[str, Any],
    candidate_summary: dict[str, Any],
    baseline_directory: Path,
    candidate_directory: Path,
) -> dict[str, Any]:
    baseline_directory = baseline_directory.resolve()
    candidate_directory = candidate_directory.resolve()
    if baseline_directory == candidate_directory:
        raise BenchmarkFailure(
            "paired comparison cohort directories resolve to the same path"
        )
    if baseline_directory.parent != candidate_directory.parent:
        raise BenchmarkFailure(
            "verified paired cohorts must share one experiment directory"
        )
    baseline_experiment = baseline_summary.get("experiment")
    candidate_experiment = candidate_summary.get("experiment")
    if baseline_experiment is None or candidate_experiment is None:
        raise BenchmarkFailure(
            "verified paired comparison requires experiment bindings"
        )
    if baseline_experiment != candidate_experiment:
        raise BenchmarkFailure("paired cohort experiment bindings differ")
    experiment = validate_experiment_binding(
        baseline_experiment, "paired comparison"
    )
    experiment_root = baseline_directory.parent
    manifest_path = experiment_root / "experiment-manifest.json"
    if manifest_path.resolve().parent != experiment_root:
        raise BenchmarkFailure("paired experiment manifest escapes its directory")
    if manifest_path.is_symlink():
        raise BenchmarkFailure("paired experiment manifest is a symlink")
    if not manifest_path.is_file():
        raise BenchmarkFailure("paired experiment manifest is missing")
    if sha256_file(manifest_path) != experiment["manifest_sha256"]:
        raise BenchmarkFailure("paired experiment manifest hash differs")
    manifest = _load_report(manifest_path)
    if (
        manifest.get("schema_version") != RUNNER_SCHEMA
        or manifest.get("kind")
        != "clonk-network-load-benchmark-paired-experiment"
    ):
        raise BenchmarkFailure("paired experiment manifest schema is invalid")
    _require_fields(
        manifest,
        (
            "experiment_id",
            "design",
            "predeclared_pair_count",
            "authoritative_pair_count",
            "runner_script_sha256",
            "configuration",
            "host_observations",
            "randomization",
            "pairs",
            "schedule",
        ),
        "paired experiment manifest",
    )
    if paired_experiment_binding(manifest) != experiment:
        raise BenchmarkFailure("paired experiment manifest binding differs")
    if manifest["authoritative_pair_count"] != AUTHORITATIVE_PAIR_COUNT:
        raise BenchmarkFailure(
            "paired experiment authoritative pair count is invalid"
        )
    _require_hex_digest(
        manifest["runner_script_sha256"],
        64,
        "paired experiment runner script hash",
    )
    if manifest["runner_script_sha256"] != runner_script_sha256():
        raise BenchmarkFailure(
            "paired experiment runner script differs from the executing runner"
        )
    validate_runtime_host_observations(manifest["host_observations"])
    pair_count = experiment["predeclared_pair_count"]
    require_authoritative_source_evidence(
        [baseline_summary["build"], candidate_summary["build"]],
        pair_count=pair_count,
    )
    for label, summary, directory in (
        ("baseline", baseline_summary, baseline_directory),
        ("candidate", candidate_summary, candidate_directory),
    ):
        metadata = summary["_cohort_metadata"]
        if summary.get("label") != label or metadata.get("label") != label:
            raise BenchmarkFailure(f"paired {label} cohort label is invalid")
        if directory.name != label:
            raise BenchmarkFailure(f"paired {label} cohort directory is invalid")
        if summary.get("requested_runs") != pair_count:
            raise BenchmarkFailure(
                f"paired {label} run count differs from experiment"
            )
    configuration = manifest.get("configuration")
    if not isinstance(configuration, dict):
        raise BenchmarkFailure("paired experiment configuration is invalid")
    baseline_configuration = baseline_summary["_cohort_metadata"][
        "configuration"
    ]
    expected_configuration = {
        "topology": baseline_configuration.get("topology"),
        "measurement_seconds": baseline_configuration.get(
            "measurement_seconds"
        ),
        "process_timeout_seconds": baseline_configuration.get(
            "process_timeout_seconds"
        ),
        "cargo_profile": baseline_summary["build"]["cargo_profile"],
    }
    if configuration != expected_configuration:
        raise BenchmarkFailure(
            "paired experiment configuration differs from its cohorts"
        )
    randomization = manifest.get("randomization")
    if not isinstance(randomization, dict) or set(randomization) != {
        "algorithm",
        "seed_hex",
    }:
        raise BenchmarkFailure("paired experiment randomization is invalid")
    if randomization["algorithm"] != "MT19937 shuffle from recorded 128-bit seed":
        raise BenchmarkFailure("paired experiment randomization algorithm is invalid")
    _require_hex_digest(
        randomization["seed_hex"], 32, "paired experiment randomization seed"
    )
    expected_orders = randomized_pair_orders(
        pair_count, randomization["seed_hex"]
    )
    expected_pairs = [
        {"pair_index": pair_index, "order": order}
        for pair_index, order in enumerate(expected_orders, start=1)
    ]
    if manifest.get("pairs") != expected_pairs:
        raise BenchmarkFailure("paired experiment pair orders are invalid")
    if pair_count == AUTHORITATIVE_PAIR_COUNT and (
        expected_orders.count("AB") != 10 or expected_orders.count("BA") != 10
    ):
        raise BenchmarkFailure(
            "authoritative paired experiment is not balanced 10 AB/10 BA"
        )
    expected_schedule = paired_schedule(expected_orders)
    if manifest.get("schedule") != expected_schedule:
        raise BenchmarkFailure("paired experiment schedule is invalid")
    for label, summary in (
        ("baseline", baseline_summary),
        ("candidate", candidate_summary),
    ):
        expected_steps = [
            step for step in expected_schedule if step["label"] == label
        ]
        executions = summary.get("_executions")
        if not isinstance(executions, list) or len(executions) != pair_count:
            raise BenchmarkFailure(
                f"paired {label} execution count differs from experiment"
            )
        for execution, step in zip(executions, expected_steps, strict=True):
            execution_step = {
                "global_sequence": execution.get("global_sequence"),
                "pair_index": execution.get("pair_index"),
                "order": execution.get("pair_order"),
                "position": execution.get("pair_position"),
                "label": label,
                "run": execution.get("pair_index"),
            }
            if execution_step != step:
                raise BenchmarkFailure(
                    f"paired {label} execution schedule differs from manifest"
                )
    return manifest


def compare_cohort_directories(
    baseline_directory: Path,
    candidate_directory: Path,
    *,
    expected_topology: str,
    expected_runs: int | None = None,
    expected_measurement_seconds: int | None = None,
    expected_timeout_seconds: int | None = None,
    expected_cargo_profile: str | None = None,
) -> dict[str, Any]:
    """Compare two already-retained, independently repeated cohorts."""

    baseline_summary, baseline_reports = load_cohort_reports(
        baseline_directory
    )
    candidate_summary, candidate_reports = load_cohort_reports(
        candidate_directory
    )
    baseline_profile = baseline_summary.get("build", {}).get("cargo_profile")
    candidate_profile = candidate_summary.get("build", {}).get("cargo_profile")
    if baseline_profile is None or candidate_profile is None:
        raise BenchmarkFailure("cohort build metadata has no Cargo profile")
    if baseline_profile != candidate_profile:
        raise BenchmarkFailure(
            "baseline and candidate Cargo profiles differ: "
            f"{baseline_profile!r} != {candidate_profile!r}"
        )
    if baseline_summary["runtime_machine"] != candidate_summary[
        "runtime_machine"
    ]:
        raise BenchmarkFailure("baseline and candidate runtime machines differ")
    require_comparable_builds(
        baseline_summary["build"], candidate_summary["build"]
    )
    if (
        expected_cargo_profile is not None
        and baseline_profile != expected_cargo_profile
    ):
        raise BenchmarkFailure(
            "retained cohort Cargo profile differs from requested profile: "
            f"{baseline_profile!r} != {expected_cargo_profile!r}"
        )
    baseline_configuration = baseline_summary["_cohort_metadata"][
        "configuration"
    ]
    candidate_configuration = candidate_summary["_cohort_metadata"][
        "configuration"
    ]
    for field in (
        "requested_runs",
        "topology",
        "measurement_seconds",
        "authoritative_default_duration",
        "process_timeout_seconds",
        "test_name",
    ):
        if baseline_configuration.get(field) != candidate_configuration.get(
            field
        ):
            raise BenchmarkFailure(
                f"baseline and candidate cohort {field} differ"
            )
    option_expectations = {
        "topology": expected_topology,
        "measurement_seconds": expected_measurement_seconds,
    }
    if expected_runs is not None:
        option_expectations["requested_runs"] = expected_runs
    if expected_timeout_seconds is not None:
        option_expectations["process_timeout_seconds"] = (
            expected_timeout_seconds
        )
    for field, expected in option_expectations.items():
        if baseline_configuration.get(field) != expected:
            raise BenchmarkFailure(
                f"retained cohort {field} differs from requested option: "
                f"{baseline_configuration.get(field)!r} != {expected!r}"
            )
    if len(baseline_reports) != len(candidate_reports):
        raise BenchmarkFailure(
            "baseline and candidate independent run counts differ: "
            f"{len(baseline_reports)} != {len(candidate_reports)}"
        )
    baseline_schedule = baseline_summary["_cohort_metadata"][
        "configuration"
    ].get("schedule")
    candidate_schedule = candidate_summary["_cohort_metadata"][
        "configuration"
    ].get("schedule")
    if baseline_schedule != candidate_schedule:
        raise BenchmarkFailure(
            "baseline and candidate comparison schedules differ"
        )
    if baseline_schedule is not None:
        raise BenchmarkFailure(
            "unverified paired schedule claim; a shared experiment manifest "
            "is required"
        )
    baseline_experiment = baseline_summary.get("experiment")
    candidate_experiment = candidate_summary.get("experiment")
    if baseline_experiment is None and candidate_experiment is None:
        paired = False
        verified_paired_experiment = False
        paired_manifest = None
    else:
        paired_manifest = validate_shared_paired_experiment(
            baseline_summary,
            candidate_summary,
            baseline_directory,
            candidate_directory,
        )
        paired = True
        verified_paired_experiment = True
    comparison = compare_report_sets(
        baseline_reports,
        candidate_reports,
        expected_topology=expected_topology,
        expected_measurement_seconds=expected_measurement_seconds,
        paired=paired,
        verified_paired_experiment=verified_paired_experiment,
    )
    def comparison_arm(
        directory: Path,
        summary: dict[str, Any],
        reports: Sequence[dict[str, Any]],
    ) -> dict[str, Any]:
        build = summary["build"]
        provenance = build["provenance"]
        return {
            "directory": str(directory.resolve()),
            "independent_runs": len(reports),
            "binary_sha256": summary.get("binary", {}).get("sha256"),
            "build_provenance_sha256": build["provenance_sha256"],
            "source_provenance_sha256": sha256_bytes(
                canonical_json(provenance["source"]).encode("utf-8")
            ),
            "content_provenance_sha256": sha256_bytes(
                canonical_json(provenance["content"]).encode("utf-8")
            ),
        }
    comparison.update(
        {
            "schema_version": RUNNER_SCHEMA,
            "kind": "clonk-network-load-benchmark-comparison",
            "result": "pass",
            "baseline": comparison_arm(
                baseline_directory, baseline_summary, baseline_reports
            ),
            "candidate": comparison_arm(
                candidate_directory, candidate_summary, candidate_reports
            ),
            "experiment": (
                baseline_experiment
                if paired_manifest is not None
                else None
            ),
            "runner_script_sha256": runner_script_sha256(),
        }
    )
    return comparison


def default_output_directory(repository_root: Path, label: str) -> Path:
    label = require_safe_label(label)
    timestamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return (
        repository_root
        / "target"
        / "network-load-benchmark"
        / f"{label}-{timestamp}"
    )


def print_metric_summary(value: dict[str, Any]) -> None:
    if "comparison_valid" in value:
        print(
            "comparison: valid="
            f"{value.get('comparison_valid')} statistically_valid="
            f"{value.get('statistically_valid')} target="
            f"{value.get('target_result')} meets_target="
            f"{value.get('meets_target')}"
        )
    metrics = value.get("metrics") or {}
    for metric_name in PRIMARY_METRICS:
        metric = metrics.get(metric_name)
        if not metric:
            continue
        if "candidate_to_baseline_ratio" in metric:
            print(
                f"{metric_name}: baseline="
                f"{metric['baseline_independent_run_median']} "
                f"candidate={metric['candidate_independent_run_median']} "
                f"ratio={metric['candidate_to_baseline_ratio']} "
                f"improvement={metric['improvement_percent']}% "
                f"ratio_95ci="
                f"{metric['candidate_to_baseline_ratio_95ci']} "
                f"target={metric['target_result']}"
            )
        else:
            print(
                f"{metric_name}: independent_runs="
                f"{metric['independent_run_count']} median_of_run_p50="
                f"{metric['independent_run_median']} {metric['unit']}"
            )


def main(argv: Sequence[str] | None = None) -> int:
    arguments = build_argument_parser().parse_args(argv)
    repository_root = arguments.repository_root.resolve()
    try:
        if arguments.command == "run":
            output = (
                arguments.output.resolve()
                if arguments.output is not None
                else default_output_directory(repository_root, arguments.label)
            )
            if arguments.binary is None:
                binary, build = build_test_binary(
                    repository_root=repository_root,
                    cargo_profile=arguments.cargo_profile or "release",
                )
            else:
                if arguments.cargo_profile is None:
                    raise BenchmarkFailure(
                        "prebuilt binary mode requires an explicit "
                        "--cargo-profile matching its provenance"
                    )
                binary = arguments.binary.resolve()
                build = prebuilt_build_record(binary, arguments.cargo_profile)
            summary = run_cohort(
                binary=binary,
                output_directory=output,
                repository_root=repository_root,
                label=arguments.label,
                runs=arguments.runs,
                topology=arguments.topology,
                measurement_seconds=arguments.measurement_seconds,
                timeout_seconds=arguments.timeout_seconds,
                build=build,
            )
            print(f"network-load benchmark artifacts: {output}")
            print_metric_summary(summary)
            return 0 if summary["result"] == "pass" else 1

        output = (
            arguments.output.resolve()
            if arguments.output is not None
            else default_output_directory(repository_root, "comparison")
        )
        baseline = arguments.baseline.resolve()
        candidate = arguments.candidate.resolve()
        if baseline.is_dir() and candidate.is_dir():
            output.mkdir(parents=True, exist_ok=False)
            comparison = compare_cohort_directories(
                baseline,
                candidate,
                expected_topology=arguments.topology,
                expected_runs=arguments.runs,
                expected_measurement_seconds=arguments.measurement_seconds,
                expected_timeout_seconds=arguments.timeout_seconds,
                expected_cargo_profile=arguments.cargo_profile,
            )
            write_json(output / "comparison.json", comparison)
        elif baseline.is_file() and candidate.is_file():
            if arguments.cargo_profile is None:
                raise BenchmarkFailure(
                    "prebuilt binary comparison requires an explicit "
                    "--cargo-profile matching both provenance sidecars"
                )
            comparison = run_paired_binaries(
                baseline_binary=baseline,
                candidate_binary=candidate,
                output_directory=output,
                repository_root=repository_root,
                runs=arguments.runs,
                topology=arguments.topology,
                measurement_seconds=arguments.measurement_seconds,
                timeout_seconds=arguments.timeout_seconds,
                cargo_profile=arguments.cargo_profile,
            )
        else:
            raise BenchmarkFailure(
                "baseline and candidate must both be cohort directories or "
                "both be prebuilt binary files"
            )
        print(f"network-load comparison artifacts: {output}")
        print_metric_summary(comparison)
        if comparison.get("comparison_valid") is not True:
            return 2
        return 0 if comparison.get("target_result") == "met" else 1
    except (BenchmarkFailure, OSError) as error:
        print(f"network-load benchmark: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
