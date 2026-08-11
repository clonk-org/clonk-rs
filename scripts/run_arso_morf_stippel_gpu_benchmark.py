#!/usr/bin/env python3
"""Run the windowed Arso-Morf 1,000-ST5B presentation benchmark.

Build the two release executables first:

    cargo build --release --offline --locked \
      -p clonk-app --bin clonk-app \
      -p clonk-engine --example arso_morf_stippel_fixture

The checked-in scenario remains untouched. This runner copies it to a temporary
directory, asks the fixture executable to create real initialized ST5B objects
in that copy, and launches the normal app/viewport benchmark against the copy.
Optional paired mode retains one canonical fixture/config and complete baseline
and candidate evidence in a caller-selected artifact directory.
"""

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
from decimal import Decimal
from pathlib import Path


WORKSPACE = Path(__file__).resolve().parents[1]
SOURCE_SCENARIO = (
    WORKSPACE
    / "content/EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s"
)
EMBEDDED_PLAYER = (
    WORKSPACE / "crates/clonk-engine/tests/fixtures/embedded_player.c4p"
)
FIXTURE_MARKER = ".clonk-rs-disposable-stippel-benchmark"
FIXTURE_PREFIX = "LC_ARSO_MORF_STIPPEL_FIXTURE"
PRESENTATION_PREFIX = "LC_APP_PRESENTATION_BENCHMARK"
PRESENTATION_CONTEXT_PREFIX = "LC_APP_PRESENTATION_BENCHMARK_CONTEXT"
PRESENTATION_NETWORK_PREFIX = "LC_APP_PRESENTATION_BENCHMARK_NETWORK"
PRESENTATION_PASS = (
    "LC_APP_PRESENTATION_BENCHMARK result=pass native_tick_budget_ms=28"
)
TARGET_STIPPELS = 1_000
MINIMUM_RETAINED_STIPPELS = TARGET_STIPPELS * 99 // 100
SOURCE_STIPPELS = 20
SOURCE_OBJECTS = 1_063
TARGET_OBJECTS = SOURCE_OBJECTS + TARGET_STIPPELS - SOURCE_STIPPELS
SEED = 424_242
NATIVE_TICK_SECONDS = 0.028
PRESENTATION_WARMUP_SECONDS = 2
APP_TIMEOUT_GRACE_SECONDS = 30
BENCHMARK_LOG_FILTER = "info,wgpu_core::device=warn"


class BenchmarkFailure(RuntimeError):
    pass


def positive_integer(raw):
    try:
        value = int(raw)
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error
    if value <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return value


def parse_fields(line, prefix):
    words = line.strip().split()
    if not words or words[0] != prefix:
        raise BenchmarkFailure(f"expected {prefix} machine line")
    fields = {}
    pending = None
    for word in words[1:]:
        if pending is not None:
            key, value = pending
            value = f"{value} {word}"
            if word.endswith("]"):
                fields[key] = value
                pending = None
            else:
                pending = (key, value)
            continue
        if "=" not in word:
            raise BenchmarkFailure(f"malformed {prefix} field: {word}")
        key, value = word.split("=", 1)
        if key in fields:
            raise BenchmarkFailure(f"duplicate {prefix} field: {key}")
        if value.startswith("[") and not value.endswith("]"):
            pending = (key, value)
        else:
            fields[key] = value
    if pending is not None:
        raise BenchmarkFailure(
            f"unterminated {prefix} list field: {pending[0]}"
        )
    return fields


def parse_fixture_line(line):
    fields = parse_fields(line, FIXTURE_PREFIX)
    required = (
        "source_stippels",
        "prepared_stippels",
        "source_lifecycle_stippels",
        "prepared_lifecycle_stippels",
        "serialized_stippels",
        "source_objects",
        "serialized_objects",
        "seed",
    )
    try:
        return {key: int(fields[key]) for key in required}
    except (KeyError, ValueError) as error:
        raise BenchmarkFailure(f"invalid fixture evidence: {error}") from error


def validate_fixture_report(report):
    if report["source_stippels"] != SOURCE_STIPPELS:
        raise BenchmarkFailure(
            "source fixture contains "
            f"{report['source_stippels']} ST5B objects; expected {SOURCE_STIPPELS}"
        )
    if report["prepared_stippels"] != TARGET_STIPPELS:
        raise BenchmarkFailure(
            "prepared engine contains "
            f"{report['prepared_stippels']} ST5B objects; expected exactly "
            f"{TARGET_STIPPELS}"
        )
    if report["source_lifecycle_stippels"] != SOURCE_STIPPELS:
        raise BenchmarkFailure(
            f"{report['source_lifecycle_stippels']} source ST5B objects have "
            f"LifeCycle; expected {SOURCE_STIPPELS}"
        )
    if report["prepared_lifecycle_stippels"] != TARGET_STIPPELS:
        raise BenchmarkFailure(
            f"{report['prepared_lifecycle_stippels']} prepared ST5B objects "
            f"have LifeCycle; expected exactly {TARGET_STIPPELS}"
        )
    if report["source_objects"] != SOURCE_OBJECTS:
        raise BenchmarkFailure(
            "source fixture contains "
            f"{report['source_objects']} objects; expected {SOURCE_OBJECTS}"
        )
    if report["serialized_stippels"] != TARGET_STIPPELS:
        raise BenchmarkFailure(
            "serialized fixture contains "
            f"{report['serialized_stippels']} ST5B objects; expected exactly "
            f"{TARGET_STIPPELS}"
        )
    if report["serialized_objects"] != TARGET_OBJECTS:
        raise BenchmarkFailure(
            "serialized fixture contains "
            f"{report['serialized_objects']} objects; expected {TARGET_OBJECTS}"
        )
    if report["seed"] != SEED:
        raise BenchmarkFailure(
            f"fixture used seed {report['seed']}; expected fixed seed {SEED}"
        )


def parse_presentation_line(line):
    fields = parse_fields(line, PRESENTATION_PREFIX)
    integer_fields = (
        "successful_present_submissions",
        "refreshed_frames",
        "simulation_frames",
        "automatic_graphics_skips",
        "graphics_pass_sample_count",
    )
    float_fields = (
        "elapsed_seconds",
        "presentation_submission_fps",
        "simulation_fps",
        "average_graphics_pass_ms",
        "max_graphics_pass_ms",
        "graphics_pass_p50_ms",
        "graphics_pass_p95_ms",
        "graphics_pass_p99_ms",
    )
    try:
        parsed = {key: int(fields[key]) for key in integer_fields}
        parsed.update({key: float(fields[key]) for key in float_fields})
        raw_samples = fields["graphics_pass_samples_ns"]
        if not raw_samples.startswith("[") or not raw_samples.endswith("]"):
            raise ValueError("graphics samples must use a bracketed list")
        values = raw_samples[1:-1].strip()
        parsed["graphics_pass_samples_ns"] = (
            []
            if not values
            else [int(value.strip()) for value in values.split(",")]
        )
    except (KeyError, ValueError) as error:
        raise BenchmarkFailure(f"invalid presentation evidence: {error}") from error
    sample_count = parsed["graphics_pass_sample_count"]
    raw_sample_count = len(parsed["graphics_pass_samples_ns"])
    if sample_count != raw_sample_count:
        raise BenchmarkFailure(
            f"graphics pass sample count is {sample_count} but "
            f"{raw_sample_count} raw samples were reported"
        )
    if any(sample < 0 for sample in parsed["graphics_pass_samples_ns"]):
        raise BenchmarkFailure("graphics pass samples cannot be negative")
    return parsed


def parse_presentation_context_line(line):
    fields = parse_fields(line, PRESENTATION_CONTEXT_PREFIX)
    required = (
        "runtime_players",
        "synchronized_player_infos",
        "activated_nonhost_clients",
        "runtime_players_with_live_crew",
        "runtime_players_with_exactly_one_live_sf5b_crew",
        "runtime_st5b_objects_at_measurement_start",
        "runtime_st5b_objects_at_measurement_end",
    )
    try:
        return {key: int(fields[key]) for key in required}
    except (KeyError, ValueError) as error:
        raise BenchmarkFailure(f"invalid presentation context: {error}") from error


def validate_playing_context(context):
    runtime_players = context["runtime_players"]
    if runtime_players < 1:
        raise BenchmarkFailure(
            f"runtime_players is {runtime_players}; expected at least 1"
        )
    synchronized = context["synchronized_player_infos"]
    if synchronized != runtime_players:
        raise BenchmarkFailure(
            "synchronized_player_infos is "
            f"{synchronized}; expected runtime_players {runtime_players}"
        )
    activated_nonhost_clients = context["activated_nonhost_clients"]
    if activated_nonhost_clients != 0:
        raise BenchmarkFailure(
            "activated_nonhost_clients is "
            f"{activated_nonhost_clients}; expected 0"
        )
    for key in (
        "runtime_players_with_live_crew",
        "runtime_players_with_exactly_one_live_sf5b_crew",
    ):
        observed = context[key]
        if observed < 1:
            raise BenchmarkFailure(
                f"{key} is {observed}; expected at least 1"
            )


def validate_runtime_stippel_census(context):
    started = context["runtime_st5b_objects_at_measurement_start"]
    if started < MINIMUM_RETAINED_STIPPELS:
        raise BenchmarkFailure(
            f"measurement started with {started} active ST5B objects; expected "
            f"at least {MINIMUM_RETAINED_STIPPELS}"
        )
    ended = context["runtime_st5b_objects_at_measurement_end"]
    if ended < MINIMUM_RETAINED_STIPPELS:
        raise BenchmarkFailure(
            f"measurement ended with {ended} active ST5B objects; expected at "
            f"least {MINIMUM_RETAINED_STIPPELS}"
        )


def required_native_frames(report):
    return int(
        Decimal(str(report["elapsed_seconds"]))
        / Decimal(str(NATIVE_TICK_SECONDS))
    )


def validate_native_cadence(report):
    required = required_native_frames(report)
    observed = report["simulation_frames"]
    if observed < required:
        raise BenchmarkFailure(
            f"observed {observed} simulation frames; native cadence requires at "
            f"least {required} in {report['elapsed_seconds']:.6f}s"
        )


def validate_native_presentation_cadence(report):
    required = required_native_frames(report)
    refreshed = report["refreshed_frames"]
    if refreshed < required:
        raise BenchmarkFailure(
            f"observed {refreshed} refreshed frames; native cadence requires at "
            f"least {required} in {report['elapsed_seconds']:.6f}s"
        )
    submissions = report["successful_present_submissions"]
    if submissions < required:
        raise BenchmarkFailure(
            f"observed {submissions} successful submissions; native cadence "
            f"requires at least {required} in {report['elapsed_seconds']:.6f}s"
        )


def require_single_result(lines, expected):
    results = [
        line.strip()
        for line in lines
        if line.startswith(f"{PRESENTATION_PREFIX} result=")
    ]
    if results != [expected]:
        raise BenchmarkFailure(
            "native presentation budget did not report pass "
            f"(observed {results or ['no result']})"
        )


def single_budget_result(lines):
    matches = [
        line.strip()
        for line in lines
        if line.startswith(f"{PRESENTATION_PREFIX} result=")
    ]
    if len(matches) != 1:
        raise BenchmarkFailure(
            "expected exactly one presentation budget result; observed "
            f"{len(matches)}"
        )
    if matches[0] == PRESENTATION_PASS:
        return "pass"
    if matches[0].startswith(f"{PRESENTATION_PREFIX} result=fail"):
        return "fail"
    raise BenchmarkFailure(f"invalid presentation budget result: {matches[0]}")


def require_network_evidence(lines):
    matches = [
        line.strip()
        for line in lines
        if line.startswith(f"{PRESENTATION_NETWORK_PREFIX} ")
    ]
    if len(matches) != 1:
        raise BenchmarkFailure(
            "expected exactly one network evidence line; observed "
            f"{len(matches)}"
        )
    fields = parse_fields(matches[0], PRESENTATION_NETWORK_PREFIX)
    status = fields.get("inspection_status")
    if status != "ok":
        raise BenchmarkFailure(
            f"network inspection status is {status or 'missing'}; expected ok"
        )
    try:
        local_client_id = int(fields["local_client_id"])
    except (KeyError, ValueError) as error:
        raise BenchmarkFailure(f"invalid network host evidence: {error}") from error
    if local_client_id != 0:
        raise BenchmarkFailure(
            f"network host local_client_id is {local_client_id}; expected 0"
        )
    return {"inspection_status": status, "local_client_id": local_client_id}


def single_machine_line(lines, prefix, first_field):
    marker = f"{prefix} {first_field}="
    matches = [line.strip() for line in lines if line.startswith(marker)]
    if len(matches) != 1:
        raise BenchmarkFailure(
            f"expected exactly one {prefix} {first_field} line; observed {len(matches)}"
        )
    return matches[0]


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256(path):
    return sha256_file(path)


def canonical_json(value):
    return json.dumps(value, indent=2, sort_keys=True) + "\n"


def json_sha256(value):
    encoded = json.dumps(
        value,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.tmp")
    temporary.write_text(canonical_json(value), encoding="utf-8")
    temporary.replace(path)


def file_fingerprint(path):
    stat = path.stat()
    return {
        "sha256": sha256_file(path),
        "size_bytes": stat.st_size,
    }


def tree_fingerprint(path):
    entries = []
    for child in sorted(path.rglob("*")):
        relative = child.relative_to(path).as_posix()
        if child.is_symlink():
            entries.append(
                {
                    "kind": "symlink",
                    "path": relative,
                    "target": os.readlink(child),
                }
            )
        elif child.is_file():
            entries.append(
                {
                    "kind": "file",
                    "path": relative,
                    **file_fingerprint(child),
                }
            )
    return {
        "sha256": json_sha256(entries),
        "files": entries,
    }


def capture_paired_input_fingerprint(fixture, config):
    value = {
        "fixture": tree_fingerprint(fixture),
        "config": file_fingerprint(config),
    }
    return {"sha256": json_sha256(value), **value}


def verify_paired_input_fingerprint(expected, fixture, config, *, stage):
    observed = capture_paired_input_fingerprint(fixture, config)
    if observed != expected:
        raise BenchmarkFailure(
            f"paired fixture or config changed {stage}; refusing a non-identical A/B run"
        )
    return observed


def binary_provenance(path):
    resolved = path.resolve()
    stat = resolved.stat()
    return {
        "path": str(resolved),
        "sha256": sha256_file(resolved),
        "size_bytes": stat.st_size,
        "modified_ns": stat.st_mtime_ns,
    }


def resolve_source_root(explicit_root, binary, *, label):
    if explicit_root is not None:
        candidates = [explicit_root.resolve()]
    else:
        candidates = list(binary.resolve().parents)
    root = next(
        (
            candidate
            for candidate in candidates
            if (candidate / ".git").exists()
            and (candidate / "Cargo.toml").is_file()
        ),
        None,
    )
    if root is None:
        raise BenchmarkFailure(
            f"could not identify the {label} source worktree; pass "
            f"--{label}-source-root"
        )
    return root


def _command_bytes(command, *, cwd):
    try:
        completed = subprocess.run(
            list(command),
            cwd=cwd,
            capture_output=True,
            check=False,
            timeout=15,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise BenchmarkFailure(
            f"provenance command failed ({' '.join(command)}): {error}"
        ) from error
    if completed.returncode != 0:
        stderr = completed.stderr.decode("utf-8", errors="replace").strip()
        raise BenchmarkFailure(
            f"provenance command failed ({' '.join(command)}): "
            f"{stderr or f'exit {completed.returncode}'}"
        )
    return completed.stdout


def _command_text(command, *, cwd):
    return _command_bytes(command, cwd=cwd).decode(
        "utf-8", errors="replace"
    ).strip()


def _untracked_file_hashes(root, command):
    output = _command_bytes(command, cwd=root)
    paths = [
        root / entry.decode("utf-8", errors="surrogateescape")
        for entry in output.split(b"\0")
        if entry
    ]
    hashes = {}
    for path in sorted(paths):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            hashes[relative] = hashlib.sha256(
                b"symlink\0" + os.fsencode(os.readlink(path))
            ).hexdigest()
        elif path.is_file():
            hashes[relative] = sha256_file(path)
    return hashes


def collect_source_provenance(root):
    root = root.resolve()
    tracked_patch = _command_bytes(
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
        cwd=root,
    )
    untracked = _untracked_file_hashes(
        root,
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
    cargo_lock = root / "Cargo.lock"
    return {
        "path": str(root),
        "commit": _command_text(("git", "rev-parse", "HEAD"), cwd=root),
        "head_tree": _command_text(
            ("git", "rev-parse", "HEAD^{tree}"), cwd=root
        ),
        "cargo_lock": (
            file_fingerprint(cargo_lock) if cargo_lock.is_file() else None
        ),
        "tracked_patch_sha256": hashlib.sha256(tracked_patch).hexdigest(),
        "untracked_files": untracked,
        "untracked_files_sha256": json_sha256(untracked),
        "dirty": bool(tracked_patch or untracked),
    }


def collect_content_provenance():
    content = (WORKSPACE / "content").resolve()
    tracked_patch = _command_bytes(
        ("git", "diff", "--binary", "--no-ext-diff", "HEAD", "--", "."),
        cwd=content,
    )
    untracked = _untracked_file_hashes(
        content,
        ("git", "ls-files", "--others", "--exclude-standard", "-z"),
    )
    gitlink = _command_text(
        ("git", "ls-tree", "HEAD", "--", "content"), cwd=WORKSPACE
    ).split()
    return {
        "head": _command_text(("git", "rev-parse", "HEAD"), cwd=content),
        "tree": _command_text(
            ("git", "rev-parse", "HEAD^{tree}"), cwd=content
        ),
        "parent_gitlink_revision": gitlink[2] if len(gitlink) >= 3 else None,
        "tracked_patch_sha256": hashlib.sha256(tracked_patch).hexdigest(),
        "untracked_files": untracked,
        "untracked_files_sha256": json_sha256(untracked),
        "dirty": bool(tracked_patch or untracked),
    }


def _best_effort_probe(command):
    try:
        completed = subprocess.run(
            list(command),
            cwd=WORKSPACE,
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return {
            "command": list(command),
            "status": "unavailable",
            "reason": str(error),
        }
    output = "\n".join(
        line
        for line in completed.stdout.splitlines()
        if "serial" not in line.lower() and "uuid" not in line.lower()
    )
    return {
        "command": list(command),
        "status": "observed" if completed.returncode == 0 else "failed",
        "exit_status": completed.returncode,
        "stdout": output,
        "stderr": completed.stderr,
    }


def _linux_power_probe():
    status_paths = sorted(Path("/sys/class/power_supply").glob("*/status"))
    if not status_paths:
        return {"status": "unavailable", "reason": "no power-supply status files"}
    return {
        "status": "observed",
        "supplies": {
            path.parent.name: path.read_text(
                encoding="utf-8", errors="replace"
            ).strip()
            for path in status_paths
        },
    }


def collect_machine_and_display_provenance():
    system = platform.system()
    if system == "Darwin":
        machine_probe = _best_effort_probe(
            ("sysctl", "-n", "hw.model", "machdep.cpu.brand_string", "hw.memsize")
        )
        display_probe = _best_effort_probe(
            ("system_profiler", "SPDisplaysDataType", "-detailLevel", "mini")
        )
        power_probe = _best_effort_probe(("pmset", "-g", "batt"))
    elif system == "Linux":
        machine_probe = _best_effort_probe(("lscpu",))
        display_probe = _best_effort_probe(("xrandr", "--current"))
        power_probe = _linux_power_probe()
    else:
        machine_probe = {"status": "unavailable", "reason": "no platform probe"}
        display_probe = {"status": "unavailable", "reason": "no platform probe"}
        power_probe = {"status": "unavailable", "reason": "no platform probe"}
    return {
        "machine": {
            "platform": platform.platform(),
            "system": system,
            "release": platform.release(),
            "architecture": platform.machine(),
            "processor": platform.processor(),
            "logical_cpu_count": os.cpu_count(),
            "probe": machine_probe,
        },
        "display": {
            "configured_window": {
                "width": 800,
                "height": 600,
                "scale_percent": 100,
            },
            "session_environment": {
                key: os.environ.get(key)
                for key in ("DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE")
            },
            "probe": display_probe,
        },
        "power": power_probe,
    }


def collect_run_provenance(arguments):
    cargo_lock = WORKSPACE / "Cargo.lock"
    machine = collect_machine_and_display_provenance()
    baseline_source = resolve_source_root(
        getattr(arguments, "baseline_source_root", None),
        arguments.baseline_app_binary,
        label="baseline",
    )
    candidate_source = resolve_source_root(
        getattr(arguments, "candidate_source_root", WORKSPACE),
        arguments.app_binary,
        label="candidate",
    )
    return {
        "source": {
            "baseline": collect_source_provenance(baseline_source),
            "candidate": collect_source_provenance(candidate_source),
        },
        "content": collect_content_provenance(),
        "inputs": {
            "cargo_lock": file_fingerprint(cargo_lock),
            "source_scenario": tree_fingerprint(SOURCE_SCENARIO),
            "embedded_player": file_fingerprint(EMBEDDED_PLAYER),
            "runner": binary_provenance(Path(__file__)),
        },
        "binaries": {
            "baseline_app": binary_provenance(arguments.baseline_app_binary),
            "candidate_app": binary_provenance(arguments.app_binary),
            "fixture_builder": binary_provenance(arguments.fixture_builder),
        },
        "toolchain": {
            "rustc_vv": _command_text(
                (os.environ.get("RUSTC", "rustc"), "-Vv"),
                cwd=WORKSPACE,
            ),
            "cargo_vv": _command_text(("cargo", "-Vv"), cwd=WORKSPACE),
            "python": sys.version,
        },
        **machine,
    }


def count_stippels(objects_path):
    return sum(
        line.rstrip(b"\r") == b"id=ST5B"
        for line in objects_path.read_bytes().splitlines()
    )


def controlled_process_environment(inherited):
    environment = inherited.copy()
    for key in (
        "LC_CONFIG_FILE",
        "RUST_LOG",
        "LC_RUST_ENGINE_RANDOM_SEED",
        "LC_RUST_ENGINE_MAP_SEED",
        "LC_RUST_ENGINE_STARTUP_PLAYERS",
        "LC_APP_PRESENTATION_BENCHMARK_KEEP_RUNNING",
        "LC_APP_PRESENTATION_BENCHMARK_INPUT_INTERVAL_MS",
    ):
        environment.pop(key, None)
    environment.update(
        {
            "LC_INSTALL_ROOT": str(WORKSPACE),
            "LC_CONTENT_DIR": str(WORKSPACE / "content"),
            "LC_LOG": BENCHMARK_LOG_FILTER,
        }
    )
    return environment


def _output_text(value):
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def _retain_process_output(path, value):
    if path is not None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(_output_text(value), encoding="utf-8")


def run_and_echo(
    command,
    *,
    environment=None,
    check=True,
    timeout=None,
    stdout_path=None,
    stderr_path=None,
):
    try:
        completed = subprocess.run(
            command,
            cwd=WORKSPACE,
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        _retain_process_output(stdout_path, error.stdout)
        _retain_process_output(stderr_path, error.stderr)
        raise BenchmarkFailure(
            f"command timed out after {timeout} seconds: {command[0]}"
        ) from error
    _retain_process_output(stdout_path, completed.stdout)
    _retain_process_output(stderr_path, completed.stderr)
    sys.stdout.write(completed.stdout)
    sys.stdout.flush()
    sys.stderr.write(completed.stderr)
    sys.stderr.flush()
    lines = completed.stdout.splitlines() + completed.stderr.splitlines()
    if check and completed.returncode != 0:
        raise BenchmarkFailure(
            f"command exited with status {completed.returncode}: {command[0]}"
        )
    return lines, completed.returncode


def executable(path, build_hint):
    if not path.is_file() or not os.access(path, os.X_OK):
        raise BenchmarkFailure(
            f"release executable not found: {path}\nbuild it with: {build_hint}"
        )


def free_port(socket_type, excluded):
    while True:
        probe = socket.socket(socket.AF_INET6, socket_type)
        try:
            probe.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
            probe.bind(("::", 0))
            port = probe.getsockname()[1]
        finally:
            probe.close()
        if port not in excluded:
            return port


def allocate_network_ports():
    ports = {}
    excluded = set()
    for name, socket_type in (
        ("tcp", socket.SOCK_STREAM),
        ("udp", socket.SOCK_DGRAM),
        ("reference", socket.SOCK_STREAM),
    ):
        ports[name] = free_port(socket_type, excluded)
        excluded.add(ports[name])
    return ports


def write_process_config(path, ports):
    path.write_text(
        "[General]\n"
        "Name=ST5B Benchmark Host\n"
        'Participants=""\n'
        "ConfigResetSafety=42\n"
        "Language=US\n"
        "LanguageEx=US\n"
        "\n"
        "[Network]\n"
        "LocalName=ST5B Benchmark Host\n"
        "Nick=ST5B Benchmark Host\n"
        f"PortTCP={ports['tcp']}\n"
        f"PortUDP={ports['udp']}\n"
        f"PortRefServer={ports['reference']}\n"
        "PortDiscovery=0\n"
        "MasterServerSignUp=false\n"
        "EnableUPnP=false\n"
        "NoRuntimeJoin=true\n"
        "ControlMode=0\n"
        "ControlRate=2\n"
        "\n"
        "[Graphics]\n"
        "ResolutionX=800\n"
        "ResolutionY=600\n"
        "Scale=100\n"
        "PointFiltering=false\n"
        "DisplayMode=1\n"
        "Maximized=false\n"
        "AutoFrameSkip=true\n",
        encoding="utf-8",
    )


def app_command(arguments, *, config, fixture, ports, app_binary=None):
    return [
        str(app_binary or arguments.app_binary),
        "--config",
        str(config),
        str(fixture),
        str(EMBEDDED_PLAYER),
        "/network",
        "/nosignup",
        f"/tcpport:{ports['tcp']}",
        f"/udpport:{ports['udp']}",
    ]


def build_argument_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "measurement_seconds",
        nargs="?",
        type=positive_integer,
        default=20,
        help="measured seconds after the app's two-second warmup (default: 20)",
    )
    parser.add_argument(
        "--app-binary",
        "--candidate-app-binary",
        dest="app_binary",
        type=Path,
        default=Path(
            os.environ.get(
                "LC_APP_BINARY", WORKSPACE / "target/release/clonk-app"
            )
        ),
    )
    parser.add_argument(
        "--baseline-app-binary",
        type=Path,
        help=(
            "origin/main app binary for a paired run; requires "
            "--paired-artifact-dir"
        ),
    )
    parser.add_argument(
        "--baseline-source-root",
        type=Path,
        help=(
            "Git worktree used to build the baseline; inferred from the "
            "binary path when it remains below that worktree"
        ),
    )
    parser.add_argument(
        "--candidate-source-root",
        type=Path,
        default=WORKSPACE,
        help="Git worktree used to build the candidate (default: this workspace)",
    )
    parser.add_argument(
        "--paired-artifact-dir",
        type=Path,
        help=(
            "new directory that retains one fixture/config, both arms' raw "
            "logs, and the provenance manifest"
        ),
    )
    parser.add_argument(
        "--fixture-builder",
        type=Path,
        default=Path(
            os.environ.get(
                "LC_STIPPEL_FIXTURE_BINARY",
                WORKSPACE / "target/release/examples/arso_morf_stippel_fixture",
            )
        ),
    )
    return parser


def validate_paired_arguments(arguments):
    if (
        arguments.baseline_source_root is not None
        and arguments.baseline_app_binary is None
    ):
        raise BenchmarkFailure(
            "--baseline-source-root requires --baseline-app-binary"
        )
    requested = (
        arguments.baseline_app_binary is not None,
        arguments.paired_artifact_dir is not None,
    )
    if any(requested) and not all(requested):
        raise BenchmarkFailure(
            "--baseline-app-binary and --paired-artifact-dir must be used together"
        )
    return all(requested)


def parse_presentation_evidence(lines, process_status):
    report = parse_presentation_line(
        single_machine_line(lines, PRESENTATION_PREFIX, "elapsed_seconds")
    )
    if (
        report["successful_present_submissions"] <= 0
        or report["refreshed_frames"] <= 0
        or report["graphics_pass_sample_count"] <= 0
    ):
        raise BenchmarkFailure("paired arm produced no refreshed presentation")
    context = parse_presentation_context_line(
        single_machine_line(
            lines,
            PRESENTATION_CONTEXT_PREFIX,
            "runtime_players",
        )
    )
    network = require_network_evidence(lines)
    validate_playing_context(context)
    validate_runtime_stippel_census(context)
    budget_result = single_budget_result(lines)
    expected_status = 0 if budget_result == "pass" else 2
    if process_status != expected_status:
        raise BenchmarkFailure(
            f"app reported budget result={budget_result} but exited with status "
            f"{process_status}; expected {expected_status}"
        )
    return {
        "process_status": process_status,
        "budget_result": budget_result,
        "presentation": report,
        "context": context,
        "network": network,
    }


def run_paired_arm(
    arguments,
    *,
    label,
    binary,
    config,
    fixture,
    ports,
    environment,
    artifact_dir,
    expected_inputs,
):
    output_dir = artifact_dir / label
    output_dir.mkdir(parents=True, exist_ok=False)
    run_config = output_dir / "config.ini"
    shutil.copyfile(config, run_config)
    input_before = capture_paired_input_fingerprint(fixture, run_config)
    if input_before != expected_inputs:
        raise BenchmarkFailure(
            f"{label} did not receive the canonical fixture and config bytes"
        )
    command = app_command(
        arguments,
        config=run_config,
        fixture=fixture,
        ports=ports,
        app_binary=binary,
    )
    lines, process_status = run_and_echo(
        command,
        environment=environment,
        check=False,
        timeout=(
            arguments.measurement_seconds
            + PRESENTATION_WARMUP_SECONDS
            + APP_TIMEOUT_GRACE_SECONDS
        ),
        stdout_path=output_dir / "stdout.log",
        stderr_path=output_dir / "stderr.log",
    )
    evidence = {
        "label": label,
        "binary": binary_provenance(binary),
        "command": command,
        "input_sha256_before": input_before["sha256"],
        "config_after": file_fingerprint(run_config),
        **parse_presentation_evidence(lines, process_status),
    }
    write_json(output_dir / "report.json", evidence)
    return evidence


def comparison_summary(baseline, candidate):
    baseline_report = baseline["presentation"]
    candidate_report = candidate["presentation"]
    metrics = {}
    for field in (
        "presentation_submission_fps",
        "simulation_fps",
        "average_graphics_pass_ms",
        "max_graphics_pass_ms",
        "graphics_pass_p50_ms",
        "graphics_pass_p95_ms",
        "graphics_pass_p99_ms",
    ):
        baseline_value = baseline_report[field]
        candidate_value = candidate_report[field]
        metrics[field] = {
            "baseline": baseline_value,
            "candidate": candidate_value,
            "candidate_minus_baseline": candidate_value - baseline_value,
            "candidate_over_baseline": (
                candidate_value / baseline_value
                if baseline_value != 0
                else None
            ),
        }
    return {"metrics": metrics}


def run_paired_benchmark(arguments):
    executable(
        arguments.app_binary,
        "cargo build --release --offline --locked -p clonk-app --bin clonk-app",
    )
    executable(
        arguments.baseline_app_binary,
        "build an instrumented origin/main clonk-app release binary",
    )
    executable(
        arguments.fixture_builder,
        "cargo build --release --offline --locked -p clonk-engine "
        "--example arso_morf_stippel_fixture",
    )
    if count_stippels(SOURCE_SCENARIO / "Objects.txt") != SOURCE_STIPPELS:
        raise BenchmarkFailure(
            "checked-in Arso-Morf no longer has the expected 20-ST5B baseline"
        )
    source_hash = sha256_file(SOURCE_SCENARIO / "Objects.txt")
    artifact_dir = arguments.paired_artifact_dir.resolve()
    content_root = (WORKSPACE / "content").resolve()
    try:
        artifact_dir.relative_to(content_root)
    except ValueError:
        pass
    else:
        raise BenchmarkFailure(
            "paired artifact directory must remain outside installed content"
        )
    provenance = collect_run_provenance(arguments)
    try:
        artifact_dir.mkdir(parents=True, exist_ok=False)
    except FileExistsError as error:
        raise BenchmarkFailure(
            f"paired artifact directory already exists: {artifact_dir}"
        ) from error

    manifest = {
        "schema_version": 1,
        "benchmark": "Arso-Morf 1,000-ST5B network presentation A/B",
        "result": "running",
        "started_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "settings": {
            "measurement_seconds": arguments.measurement_seconds,
            "warmup_seconds": PRESENTATION_WARMUP_SECONDS,
            "seed": SEED,
            "target_stippels": TARGET_STIPPELS,
            "minimum_retained_stippels": MINIMUM_RETAINED_STIPPELS,
            "native_tick_seconds": NATIVE_TICK_SECONDS,
            "run_order": ["baseline", "candidate"],
        },
        "provenance": provenance,
        "input_checks": [],
        "runs": {},
    }
    write_json(artifact_dir / "manifest.json", manifest)

    try:
        fixture = artifact_dir / "fixture" / "Arso-Morf.c4s"
        fixture.parent.mkdir()
        shutil.copytree(SOURCE_SCENARIO, fixture)
        (fixture / FIXTURE_MARKER).write_text(
            "clonk-rs Arso-Morf ST5B GPU benchmark fixture v1\n",
            encoding="utf-8",
        )
        fixture_command = [
            str(arguments.fixture_builder),
            str(fixture),
            str(SEED),
        ]
        fixture_lines, _ = run_and_echo(
            fixture_command,
            stdout_path=artifact_dir / "fixture-builder.stdout.log",
            stderr_path=artifact_dir / "fixture-builder.stderr.log",
        )
        fixture_report = parse_fixture_line(
            single_machine_line(
                fixture_lines,
                FIXTURE_PREFIX,
                "source_stippels",
            )
        )
        validate_fixture_report(fixture_report)
        serialized_count = count_stippels(fixture / "Objects.txt")
        if serialized_count != TARGET_STIPPELS:
            raise BenchmarkFailure(
                "independent Objects.txt census found "
                f"{serialized_count} ST5B objects; expected exactly "
                f"{TARGET_STIPPELS}"
            )

        config = artifact_dir / "config.ini"
        ports = allocate_network_ports()
        write_process_config(config, ports)
        inputs = capture_paired_input_fingerprint(fixture, config)
        manifest["fixture_builder"] = {
            "command": fixture_command,
            "report": fixture_report,
        }
        manifest["ports"] = ports
        manifest["paired_inputs"] = inputs
        manifest["input_checks"].append(
            {"stage": "after fixture generation", "sha256": inputs["sha256"]}
        )
        write_json(artifact_dir / "input-fingerprint.json", inputs)
        write_json(artifact_dir / "manifest.json", manifest)

        environment = controlled_process_environment(os.environ)
        environment.update(
            {
                "LC_PIN_SEED": str(SEED),
                "LC_APP_PRESENTATION_BENCHMARK_SECONDS": str(
                    arguments.measurement_seconds
                ),
                "LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK": "1",
            }
        )
        manifest["environment"] = {
            key: environment[key]
            for key in (
                "LC_INSTALL_ROOT",
                "LC_CONTENT_DIR",
                "LC_LOG",
                "LC_PIN_SEED",
                "LC_APP_PRESENTATION_BENCHMARK_SECONDS",
                "LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK",
            )
        }

        observed = verify_paired_input_fingerprint(
            inputs, fixture, config, stage="before baseline"
        )
        manifest["input_checks"].append(
            {"stage": "before baseline", "sha256": observed["sha256"]}
        )
        try:
            baseline = run_paired_arm(
                arguments,
                label="baseline",
                binary=arguments.baseline_app_binary,
                config=config,
                fixture=fixture,
                ports=ports,
                environment=environment,
                artifact_dir=artifact_dir,
                expected_inputs=inputs,
            )
            manifest["runs"]["baseline"] = baseline
            write_json(artifact_dir / "manifest.json", manifest)
        finally:
            observed = verify_paired_input_fingerprint(
                inputs, fixture, config, stage="after baseline"
            )
            manifest["input_checks"].append(
                {"stage": "after baseline", "sha256": observed["sha256"]}
            )

        observed = verify_paired_input_fingerprint(
            inputs, fixture, config, stage="before candidate"
        )
        manifest["input_checks"].append(
            {"stage": "before candidate", "sha256": observed["sha256"]}
        )
        try:
            candidate = run_paired_arm(
                arguments,
                label="candidate",
                binary=arguments.app_binary,
                config=config,
                fixture=fixture,
                ports=ports,
                environment=environment,
                artifact_dir=artifact_dir,
                expected_inputs=inputs,
            )
            manifest["runs"]["candidate"] = candidate
            write_json(artifact_dir / "manifest.json", manifest)
        finally:
            observed = verify_paired_input_fingerprint(
                inputs, fixture, config, stage="after candidate"
            )
            manifest["input_checks"].append(
                {"stage": "after candidate", "sha256": observed["sha256"]}
            )

        validate_native_cadence(candidate["presentation"])
        validate_native_presentation_cadence(candidate["presentation"])
        if candidate["budget_result"] != "pass":
            raise BenchmarkFailure("candidate did not pass the native presentation budget")
        if sha256_file(SOURCE_SCENARIO / "Objects.txt") != source_hash:
            raise BenchmarkFailure("checked-in Arso-Morf Objects.txt was modified")

        manifest["comparison"] = comparison_summary(baseline, candidate)
        manifest["result"] = "pass"
        manifest["completed_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
        write_json(artifact_dir / "manifest.json", manifest)
    except (BenchmarkFailure, OSError) as error:
        manifest["result"] = "fail"
        manifest["error"] = str(error)
        manifest["completed_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
        write_json(artifact_dir / "manifest.json", manifest)
        if isinstance(error, BenchmarkFailure):
            raise
        raise BenchmarkFailure(f"paired benchmark artifact failure: {error}") from error

    print(
        "LC_ARSO_MORF_STIPPEL_GPU_BENCHMARK_PAIRED "
        f"result=pass artifact_dir={artifact_dir} "
        f"input_sha256={inputs['sha256']} "
        f"baseline_budget_result={baseline['budget_result']} "
        f"candidate_budget_result={candidate['budget_result']} "
        "baseline_average_graphics_pass_ms="
        f"{baseline['presentation']['average_graphics_pass_ms']:.6f} "
        "candidate_average_graphics_pass_ms="
        f"{candidate['presentation']['average_graphics_pass_ms']:.6f}"
    )


def run_benchmark(arguments):
    executable(
        arguments.app_binary,
        "cargo build --release --offline --locked -p clonk-app --bin clonk-app",
    )
    executable(
        arguments.fixture_builder,
        "cargo build --release --offline --locked -p clonk-engine "
        "--example arso_morf_stippel_fixture",
    )
    if count_stippels(SOURCE_SCENARIO / "Objects.txt") != SOURCE_STIPPELS:
        raise BenchmarkFailure(
            "checked-in Arso-Morf no longer has the expected 20-ST5B baseline"
        )
    source_hash = sha256(SOURCE_SCENARIO / "Objects.txt")

    temporary_root = os.environ.get("TMPDIR")
    with tempfile.TemporaryDirectory(
        prefix="clonk-rust-arso-morf-stippel-gpu-benchmark.",
        dir=temporary_root,
    ) as temporary:
        fixture = Path(temporary) / "Arso-Morf.c4s"
        shutil.copytree(SOURCE_SCENARIO, fixture)
        (fixture / FIXTURE_MARKER).write_text(
            "clonk-rs Arso-Morf ST5B GPU benchmark fixture v1\n",
            encoding="utf-8",
        )

        fixture_lines, _ = run_and_echo(
            [str(arguments.fixture_builder), str(fixture), str(SEED)]
        )
        fixture_report = parse_fixture_line(
            single_machine_line(
                fixture_lines, FIXTURE_PREFIX, "source_stippels"
            )
        )
        validate_fixture_report(fixture_report)
        serialized_count = count_stippels(fixture / "Objects.txt")
        if serialized_count != TARGET_STIPPELS:
            raise BenchmarkFailure(
                "independent Objects.txt census found "
                f"{serialized_count} ST5B objects; expected exactly {TARGET_STIPPELS}"
            )

        config = Path(temporary) / "config.ini"
        ports = allocate_network_ports()
        write_process_config(config, ports)
        environment = controlled_process_environment(os.environ)
        environment.update(
            {
                "LC_PIN_SEED": str(SEED),
                "LC_APP_PRESENTATION_BENCHMARK_SECONDS": str(
                    arguments.measurement_seconds
                ),
                "LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK": "1",
            }
        )
        presentation_lines, presentation_status = run_and_echo(
            app_command(
                arguments,
                config=config,
                fixture=fixture,
                ports=ports,
            ),
            environment=environment,
            check=False,
            timeout=(
                arguments.measurement_seconds
                + PRESENTATION_WARMUP_SECONDS
                + APP_TIMEOUT_GRACE_SECONDS
            ),
        )
        report = parse_presentation_line(
            single_machine_line(
                presentation_lines, PRESENTATION_PREFIX, "elapsed_seconds"
            )
        )
        context = parse_presentation_context_line(
            single_machine_line(
                presentation_lines,
                PRESENTATION_CONTEXT_PREFIX,
                "runtime_players",
            )
        )
        network_evidence = require_network_evidence(presentation_lines)
        validate_playing_context(context)
        validate_runtime_stippel_census(context)
        validate_native_cadence(report)
        validate_native_presentation_cadence(report)
        require_single_result(presentation_lines, PRESENTATION_PASS)
        if presentation_status != 0:
            raise BenchmarkFailure(
                "app exited with status "
                f"{presentation_status} after reporting a passing budget"
            )

    if sha256(SOURCE_SCENARIO / "Objects.txt") != source_hash:
        raise BenchmarkFailure("checked-in Arso-Morf Objects.txt was modified")

    required = required_native_frames(report)
    print(
        "LC_ARSO_MORF_STIPPEL_GPU_BENCHMARK "
        f"result=pass target_stippels={TARGET_STIPPELS} seed={SEED} "
        f"elapsed_seconds={report['elapsed_seconds']:.6f} "
        f"required_native_frames={required} "
        f"simulation_frames={report['simulation_frames']} "
        f"simulation_fps={report['simulation_fps']:.6f} "
        f"runtime_players={context['runtime_players']} "
        "runtime_players_with_live_crew="
        f"{context['runtime_players_with_live_crew']} "
        "runtime_st5b_objects_at_measurement_start="
        f"{context['runtime_st5b_objects_at_measurement_start']} "
        "runtime_st5b_objects_at_measurement_end="
        f"{context['runtime_st5b_objects_at_measurement_end']} "
        f"minimum_retained_st5b_objects={MINIMUM_RETAINED_STIPPELS} "
        f"presentation_submission_fps="
        f"{report['presentation_submission_fps']:.6f} "
        f"automatic_graphics_skips={report['automatic_graphics_skips']} "
        f"average_graphics_pass_ms="
        f"{report['average_graphics_pass_ms']:.6f} "
        f"network_inspection_status={network_evidence['inspection_status']} "
        f"network_local_client_id={network_evidence['local_client_id']}"
    )


def main():
    try:
        arguments = build_argument_parser().parse_args()
        if validate_paired_arguments(arguments):
            run_paired_benchmark(arguments)
        else:
            run_benchmark(arguments)
    except BenchmarkFailure as error:
        print(
            f"LC_ARSO_MORF_STIPPEL_GPU_BENCHMARK result=fail error={error}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
