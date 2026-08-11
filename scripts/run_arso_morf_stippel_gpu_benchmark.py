#!/usr/bin/env python3
"""Run the windowed Arso-Morf 1,000-ST5B presentation benchmark.

Build the two release executables first:

    cargo build --release --offline --locked \
      -p clonk-app --bin clonk-app \
      -p clonk-engine --example arso_morf_stippel_fixture

The checked-in scenario remains untouched. This runner copies it to a temporary
directory, asks the fixture executable to create real initialized ST5B objects
in that copy, and launches the normal app/viewport benchmark against the copy.
"""

import argparse
import hashlib
import os
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
    for word in words[1:]:
        if "=" not in word:
            raise BenchmarkFailure(f"malformed {prefix} field: {word}")
        key, value = word.split("=", 1)
        if key in fields:
            raise BenchmarkFailure(f"duplicate {prefix} field: {key}")
        fields[key] = value
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
    )
    float_fields = (
        "elapsed_seconds",
        "presentation_submission_fps",
        "simulation_fps",
        "average_graphics_pass_ms",
    )
    try:
        parsed = {key: int(fields[key]) for key in integer_fields}
        parsed.update({key: float(fields[key]) for key in float_fields})
    except (KeyError, ValueError) as error:
        raise BenchmarkFailure(f"invalid presentation evidence: {error}") from error
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


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


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


def run_and_echo(command, *, environment=None, check=True, timeout=None):
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
        raise BenchmarkFailure(
            f"command timed out after {timeout} seconds: {command[0]}"
        ) from error
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


def app_command(arguments, *, config, fixture, ports):
    return [
        str(arguments.app_binary),
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
        type=Path,
        default=Path(
            os.environ.get(
                "LC_APP_BINARY", WORKSPACE / "target/release/clonk-app"
            )
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
        run_benchmark(build_argument_parser().parse_args())
    except BenchmarkFailure as error:
        print(
            f"LC_ARSO_MORF_STIPPEL_GPU_BENCHMARK result=fail error={error}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
