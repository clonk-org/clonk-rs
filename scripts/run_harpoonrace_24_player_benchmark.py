#!/usr/bin/env python3
"""Run one real HarpoonRace host and a 24-player rendered client fleet.

The host follows the classic C++ lobby path: it opens HarpoonRace without a
local player, applies ``/set maxplayer 24`` while still in the lobby, admits
24 distinct directory-backed ``.c4p`` profiles, and starts only after the
host's exact HTTP reference exposes every PlayerInfo.

This is intentionally an opt-in, process-level benchmark.  It opens 25 native
windows and should be run from an interactive desktop session on otherwise
idle hardware.  Results and raw samples are retained below ``target/``.
"""

from __future__ import annotations

import argparse
import colorsys
import datetime as dt
import hashlib
import http.client
import json
import math
import os
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable, Iterable, Sequence


MACHINE_PREFIX = "LC_APP_PRESENTATION_BENCHMARK "
MACHINE_METRIC_PREFIX = MACHINE_PREFIX + "elapsed_seconds="
MACHINE_PASS = MACHINE_PREFIX + "result=pass native_tick_budget_ms=28"
MACHINE_FAIL_PREFIX = MACHINE_PREFIX + "result=fail"
MACHINE_CONTEXT_PREFIX = "LC_APP_PRESENTATION_BENCHMARK_CONTEXT "
MACHINE_NETWORK_PREFIX = "LC_APP_PRESENTATION_BENCHMARK_NETWORK "
FLEET_LOG_FILTER = "info,wgpu_core::device=warn"
NATIVE_GAME_TICK_MS = 28.0
BENCHMARK_WARMUP_SECONDS = 2.0
GO_LOG_PATTERN = re.compile(
    r"^(?P<timestamp_utc>"
    r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z"
    r")\s+INFO\s+Go!\s*$",
    re.MULTILINE,
)
DEFAULT_SCENARIO = (
    "content/EkeReloaded.c4f/InterplanetaryCivilwar.c4f/"
    "HarpoonRace.c4s"
)
DEFAULT_PROFILE_BIG_ICON = (
    "content/Fantasy.c4f/Drachenfels.c4s/"
    "ScriptPlr-1.c4p/BigIcon.png"
)
INTEGER_METRICS = {
    "successful_present_submissions",
    "refreshed_frames",
    "simulation_frames",
    "automatic_graphics_skips",
    "graphics_pass_sample_count",
}
FLOAT_METRICS = {
    "elapsed_seconds",
    "presentation_submission_fps",
    "simulation_fps",
    "average_graphics_pass_ms",
    "max_graphics_pass_ms",
    "graphics_pass_p50_ms",
    "graphics_pass_p95_ms",
    "graphics_pass_p99_ms",
}
REQUIRED_METRICS = INTEGER_METRICS | FLOAT_METRICS | {
    "graphics_pass_samples_ns"
}
NETWORK_INTEGER_FIELDS = {
    "local_client_id",
    "preferred_message_route_peer_count",
    "tcp_preferred_message_routes",
    "udp_preferred_message_routes",
    "unknown_preferred_message_routes",
    "nonnegative_ping_peer_count",
    "nonnegative_lag_peer_count",
    "max_nonnegative_ping_ms",
    "max_nonnegative_lag_ms",
    "max_packet_loss",
    "control_presend",
    "avg_control_send_time_us",
}
RUNTIME_ERROR_PATTERNS = (
    re.compile(r"(^|\s)ERROR(\s|$)"),
    re.compile(r"\bpanicked at\b", re.IGNORECASE),
    re.compile(r"\bdesync(?:hronization)?\b", re.IGNORECASE),
    re.compile(r"synchronization (?:check )?mismatch", re.IGNORECASE),
    re.compile(r"retained GPU render failed", re.IGNORECASE),
)


class FleetFailure(RuntimeError):
    """An expected benchmark-orchestration failure."""


def nearest_rank_percentile(samples: Sequence[float], quantile: float) -> float:
    """Return the deterministic nearest-rank percentile used in reports."""

    if not samples:
        raise ValueError("cannot calculate a percentile without samples")
    if not 0.0 <= quantile <= 1.0:
        raise ValueError(f"quantile must be between zero and one: {quantile}")
    ordered = sorted(float(sample) for sample in samples)
    rank = max(1, math.ceil(quantile * len(ordered)))
    return ordered[rank - 1]


def sample_statistics(samples: Sequence[float]) -> dict[str, Any]:
    """Summarize samples while keeping raw values in a separate artifact."""

    finite = [float(sample) for sample in samples]
    if not finite:
        return {"sample_count": 0}
    if not all(math.isfinite(sample) for sample in finite):
        raise ValueError("statistics contain a non-finite sample")
    return {
        "sample_count": len(finite),
        "minimum": min(finite),
        "p01": nearest_rank_percentile(finite, 0.01),
        "p05": nearest_rank_percentile(finite, 0.05),
        "p50": nearest_rank_percentile(finite, 0.50),
        "p95": nearest_rank_percentile(finite, 0.95),
        "p99": nearest_rank_percentile(finite, 0.99),
        "maximum": max(finite),
    }


def parse_benchmark_machine_line(line: str) -> dict[str, Any]:
    """Parse one complete presentation benchmark machine line."""

    line = line.strip()
    if not line.startswith(MACHINE_METRIC_PREFIX):
        raise ValueError("line is not a presentation benchmark metric")
    fields: dict[str, Any] = {}
    for token in line[len(MACHINE_PREFIX) :].split():
        if "=" not in token:
            raise ValueError(f"benchmark token has no value: {token!r}")
        key, raw = token.split("=", 1)
        if key in fields:
            raise ValueError(f"duplicate benchmark field: {key}")
        if key == "graphics_pass_samples_ns":
            if not raw.startswith("[") or not raw.endswith("]"):
                raise ValueError("graphics samples must use a bracketed list")
            values = raw[1:-1]
            fields[key] = (
                []
                if not values
                else [int(value, 10) for value in values.split(",")]
            )
        elif key in INTEGER_METRICS:
            fields[key] = int(raw, 10)
        elif key in FLOAT_METRICS:
            value = float(raw)
            if not math.isfinite(value):
                raise ValueError(f"non-finite benchmark field: {key}")
            fields[key] = value
        else:
            fields[key] = raw

    missing = sorted(REQUIRED_METRICS - fields.keys())
    if missing:
        raise ValueError(
            "benchmark metric is missing required fields: " + ", ".join(missing)
        )
    samples = fields["graphics_pass_samples_ns"]
    if any(sample < 0 for sample in samples):
        raise ValueError("graphics samples cannot be negative")
    return fields


def parse_benchmark_context_line(line: str) -> dict[str, int]:
    line = line.strip()
    if not line.startswith(MACHINE_CONTEXT_PREFIX):
        raise ValueError("line is not a presentation benchmark context")
    fields: dict[str, int] = {}
    for token in line[len(MACHINE_CONTEXT_PREFIX) :].split():
        if "=" not in token:
            raise ValueError(f"benchmark context token has no value: {token!r}")
        key, raw = token.split("=", 1)
        if key in fields:
            raise ValueError(f"duplicate benchmark context field: {key}")
        fields[key] = int(raw, 10)
    required = {
        "runtime_players",
        "synchronized_player_infos",
        "activated_nonhost_clients",
        "runtime_crew_objects",
        "runtime_players_with_exactly_one_live_sf5b_crew",
    }
    missing = sorted(required - fields.keys())
    if missing:
        raise ValueError(
            "benchmark context is missing required fields: " + ", ".join(missing)
        )
    return fields


def parse_benchmark_network_line(line: str) -> dict[str, Any]:
    """Parse one post-measurement preferred-message-route snapshot."""

    line = line.strip()
    if not line.startswith(MACHINE_NETWORK_PREFIX):
        raise ValueError("line is not presentation benchmark network evidence")
    fields: dict[str, Any] = {}
    for token in line[len(MACHINE_NETWORK_PREFIX) :].split():
        if "=" not in token:
            raise ValueError(f"benchmark network token has no value: {token!r}")
        key, raw = token.split("=", 1)
        if key in fields:
            raise ValueError(f"duplicate benchmark network field: {key}")
        if key == "preferred_message_route_peer_ids":
            if not raw.startswith("[") or not raw.endswith("]"):
                raise ValueError(
                    "preferred message-route peers must use a bracketed list"
                )
            values = raw[1:-1]
            fields[key] = (
                [] if not values else [int(value, 10) for value in values.split(",")]
            )
        elif key in NETWORK_INTEGER_FIELDS:
            fields[key] = int(raw, 10)
        else:
            fields[key] = raw

    if fields.get("inspection_status") != "ok":
        error_code = fields.get("error_code", "unspecified")
        raise ValueError(
            "benchmark network inspection failed: " + str(error_code)
        )
    required = NETWORK_INTEGER_FIELDS | {
        "inspection_status",
        "preferred_message_route_peer_ids",
    }
    missing = sorted(required - fields.keys())
    if missing:
        raise ValueError(
            "benchmark network evidence is missing required fields: "
            + ", ".join(missing)
        )
    peers = fields["preferred_message_route_peer_ids"]
    if any(peer < 0 for peer in peers):
        raise ValueError("preferred message-route peer IDs cannot be negative")
    if len(peers) != len(set(peers)):
        raise ValueError("preferred message-route peer IDs must be unique")
    if fields["preferred_message_route_peer_count"] != len(peers):
        raise ValueError(
            "preferred message-route peer count does not match peer IDs"
        )
    protocol_count = sum(
        fields[name]
        for name in (
            "tcp_preferred_message_routes",
            "udp_preferred_message_routes",
            "unknown_preferred_message_routes",
        )
    )
    if protocol_count != len(peers):
        raise ValueError(
            "preferred message-route protocol counts do not match peer IDs"
        )
    for field in (
        "nonnegative_ping_peer_count",
        "nonnegative_lag_peer_count",
    ):
        if not 0 <= fields[field] <= len(peers):
            raise ValueError(
                f"{field} must be between zero and the preferred peer count"
            )
    for count_field, maximum_field, label in (
        (
            "nonnegative_ping_peer_count",
            "max_nonnegative_ping_ms",
            "ping",
        ),
        (
            "nonnegative_lag_peer_count",
            "max_nonnegative_lag_ms",
            "lag",
        ),
    ):
        sample_count = fields[count_field]
        maximum = fields[maximum_field]
        if sample_count == 0 and maximum != -1:
            raise ValueError(
                f"{maximum_field} must be -1 when no {label} samples exist"
            )
        if sample_count > 0 and maximum < 0:
            raise ValueError(
                f"{maximum_field} must be nonnegative when {label} samples exist"
            )
    if fields["local_client_id"] < 0:
        raise ValueError("local client ID cannot be negative")
    if fields["max_packet_loss"] < 0:
        raise ValueError("packet loss cannot be negative")
    return fields


def extract_benchmark_report(stdout: str, stderr: str) -> dict[str, Any]:
    """Require exactly one complete metric/context/network triple.

    Fleet acceptance belongs to the supervisor after every participant has
    reported. An in-process assertion can otherwise disconnect a slower or
    failed client while its peers are still measuring, changing the very
    24-player topology under observation.
    """

    combined_lines = stdout.splitlines() + stderr.splitlines()
    metric_lines = [
        line for line in combined_lines if line.startswith(MACHINE_METRIC_PREFIX)
    ]
    if len(metric_lines) != 1:
        raise ValueError(
            "expected exactly one metric line, "
            f"observed {len(metric_lines)}"
        )
    context_lines = [
        line
        for line in combined_lines
        if line.startswith(MACHINE_CONTEXT_PREFIX)
    ]
    if len(context_lines) != 1:
        raise ValueError(
            "expected exactly one benchmark context line, "
            f"observed {len(context_lines)}"
        )
    network_lines = [
        line
        for line in combined_lines
        if line.startswith(MACHINE_NETWORK_PREFIX)
    ]
    if len(network_lines) != 1:
        raise ValueError(
            "expected exactly one benchmark network line, "
            f"observed {len(network_lines)}"
        )
    report = parse_benchmark_machine_line(metric_lines[0])
    report["benchmark_context"] = parse_benchmark_context_line(
        context_lines[0]
    )
    report["network_evidence"] = parse_benchmark_network_line(
        network_lines[0]
    )
    return report


def extract_go_log_timestamp(text: str) -> str | None:
    """Return the first source timestamp from an exact ``INFO Go!`` log."""

    match = GO_LOG_PATTERN.search(text)
    return None if match is None else match.group("timestamp_utc")


def file_modified_utc(path: Path) -> str | None:
    """Return filesystem mtime evidence without treating it as emit time."""

    if not path.is_file():
        return None
    return dt.datetime.fromtimestamp(
        path.stat().st_mtime_ns / 1_000_000_000,
        tz=dt.timezone.utc,
    ).isoformat()


def benchmark_completion_timeout_seconds(
    measurement_seconds: int,
    completion_grace_seconds: float,
) -> float:
    return (
        float(measurement_seconds)
        + BENCHMARK_WARMUP_SECONDS
        + completion_grace_seconds
    )


def benchmark_failures(
    report: dict[str, Any],
    *,
    expected_seconds: int,
    expected_players: int,
    minimum_simulation_fps: float,
    minimum_presentation_fps: float,
    maximum_graphics_p99_ms: float,
    maximum_network_lag_ms: float,
) -> list[str]:
    """Apply the benchmark's local acceptance contract."""

    failures: list[str] = []
    if report["elapsed_seconds"] < expected_seconds:
        failures.append(
            "measured window "
            f"{report['elapsed_seconds']:.6f}s is shorter than "
            f"{expected_seconds}s"
        )
    if report["successful_present_submissions"] <= 0:
        failures.append("benchmark produced no successful presentation")
    if report["refreshed_frames"] <= 0:
        failures.append("benchmark produced no refreshed frame")
    if report["average_graphics_pass_ms"] > NATIVE_GAME_TICK_MS:
        failures.append(
            "average graphics pass "
            f"{report['average_graphics_pass_ms']:.6f}ms exceeds the "
            f"native {NATIVE_GAME_TICK_MS:.0f}ms game tick"
        )
    if report["simulation_fps"] < minimum_simulation_fps:
        failures.append(
            f"simulation FPS {report['simulation_fps']:.6f} is below "
            f"{minimum_simulation_fps:.6f}"
        )
    if report["presentation_submission_fps"] < minimum_presentation_fps:
        failures.append(
            "presentation FPS "
            f"{report['presentation_submission_fps']:.6f} is below "
            f"{minimum_presentation_fps:.6f}"
        )
    if report["graphics_pass_p99_ms"] >= maximum_graphics_p99_ms:
        failures.append(
            f"graphics p99 {report['graphics_pass_p99_ms']:.6f}ms is not "
            f"below {maximum_graphics_p99_ms:.6f}ms"
        )
    for field in (
        "runtime_players",
        "synchronized_player_infos",
        "activated_nonhost_clients",
        "runtime_players_with_exactly_one_live_sf5b_crew",
    ):
        observed = report["benchmark_context"][field]
        if observed != expected_players:
            failures.append(
                f"benchmark context {field}={observed}, expected "
                f"{expected_players}"
            )

    network = report["network_evidence"]
    local_client_id = network["local_client_id"]
    expected_peer_ids = set(range(expected_players + 1))
    expected_peer_ids.discard(local_client_id)
    observed_peer_ids = set(network["preferred_message_route_peer_ids"])
    if observed_peer_ids != expected_peer_ids:
        failures.append(
            "preferred message-route peer coverage "
            f"{sorted(observed_peer_ids)}, expected {sorted(expected_peer_ids)} "
            f"for local client {local_client_id}"
        )
    expected_peer_count = len(expected_peer_ids)
    for field, label in (
        ("nonnegative_ping_peer_count", "ping"),
        ("nonnegative_lag_peer_count", "lag"),
    ):
        observed = network[field]
        if observed != expected_peer_count:
            failures.append(
                f"nonnegative preferred-message-route {label} coverage "
                f"{observed}/{expected_peer_count}"
            )
    max_lag_ms = network["max_nonnegative_lag_ms"]
    if max_lag_ms < 0:
        failures.append("benchmark produced no nonnegative message-route lag sample")
    elif max_lag_ms > maximum_network_lag_ms:
        failures.append(
            f"maximum message-route lag {max_lag_ms}ms exceeds "
            f"{maximum_network_lag_ms:.3f}ms"
        )

    samples_ns = report["graphics_pass_samples_ns"]
    sample_count = report["graphics_pass_sample_count"]
    if sample_count != len(samples_ns):
        failures.append(
            f"graphics sample count {sample_count} does not match "
            f"{len(samples_ns)} raw samples"
        )
    if sample_count != report["successful_present_submissions"]:
        failures.append(
            f"graphics sample count {sample_count} does not match "
            f"{report['successful_present_submissions']} submissions"
        )
    if samples_ns:
        raw_p99_ms = nearest_rank_percentile(samples_ns, 0.99) / 1_000_000.0
        if raw_p99_ms >= maximum_graphics_p99_ms:
            failures.append(
                f"raw graphics p99 {raw_p99_ms:.6f}ms is not below "
                f"{maximum_graphics_p99_ms:.6f}ms"
            )
        if not math.isclose(
            raw_p99_ms,
            report["graphics_pass_p99_ms"],
            rel_tol=0.0,
            abs_tol=0.001,
        ):
            failures.append(
                "reported graphics p99 does not match raw nearest-rank "
                f"p99 ({report['graphics_pass_p99_ms']:.6f}ms versus "
                f"{raw_p99_ms:.6f}ms)"
            )
    return failures


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_tree(path: Path) -> str:
    digest = hashlib.sha256()
    for child in sorted(candidate for candidate in path.rglob("*") if candidate.is_file()):
        digest.update(child.relative_to(path).as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(sha256_file(child).encode("ascii"))
        digest.update(b"\0")
    return digest.hexdigest()


def command_output(arguments: Sequence[str], cwd: Path) -> str | None:
    try:
        return subprocess.run(
            arguments,
            cwd=cwd,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w", encoding="utf-8") as destination:
        json.dump(value, destination, indent=2, sort_keys=True)
        destination.write("\n")
    temporary.replace(path)


def json_fallback(value: Any) -> str:
    if isinstance(value, Path):
        return str(value)
    raise TypeError(
        f"object of type {type(value).__name__} is not JSON serializable"
    )


def controlled_process_environment(
    inherited: dict[str, str],
) -> dict[str, str]:
    """Return a reproducible, error-visible process environment.

    The app gives ``LC_CONFIG_FILE`` precedence over its ``--config`` flag.
    Ambient logging can either hide errors or turn dependency tracing into a
    benchmark observer effect, so both inputs are normalized here.
    """

    environment = inherited.copy()
    environment.pop("LC_CONFIG_FILE", None)
    environment.pop("RUST_LOG", None)
    environment["LC_LOG"] = FLEET_LOG_FILTER
    return environment


def write_process_config(
    path: Path,
    *,
    name: str,
    tcp_port: int,
    udp_port: int,
    reference_port: int,
    width: int,
    height: int,
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "[General]\n"
        f"Name={name}\n"
        'Participants=""\n'
        "ConfigResetSafety=42\n"
        "\n"
        "[Network]\n"
        f"LocalName={name}\n"
        f"Nick={name}\n"
        f"PortTCP={tcp_port}\n"
        f"PortUDP={udp_port}\n"
        f"PortRefServer={reference_port}\n"
        "PortDiscovery=0\n"
        "MasterServerSignUp=false\n"
        "EnableUPnP=false\n"
        "NoRuntimeJoin=true\n"
        "ControlMode=0\n"
        "ControlRate=2\n"
        "AsyncMaxWait=2\n"
        "\n"
        "[Graphics]\n"
        f"ResolutionX={width}\n"
        f"ResolutionY={height}\n"
        "Scale=100\n"
        "DisplayMode=1\n"
        "Maximized=false\n"
        "PointFiltering=false\n"
        "AutoFrameSkip=true\n",
        encoding="utf-8",
    )


def write_distinct_profile(
    path: Path,
    name: str,
    index: int,
    count: int,
    *,
    big_icon: Path | None = None,
) -> None:
    """Create one valid directory-backed C4Group player profile."""

    path.mkdir(parents=True, exist_ok=False)
    red, green, blue = colorsys.hsv_to_rgb((index - 1) / max(1, count), 0.8, 1.0)
    color_dw = (
        (int(red * 255) << 16)
        | (int(green * 255) << 8)
        | int(blue * 255)
    )
    (path / "Player.txt").write_text(
        "[Player]\n"
        f"Name={name}\n"
        "Comment=HarpoonRace 24-player load test\n"
        "Score=0\n"
        "Rounds=0\n"
        "RoundsWon=0\n"
        "RoundsLost=0\n"
        "TotalPlayingTime=0\n"
        "\n"
        "[Preferences]\n"
        f"Color={(index - 1) % 8}\n"
        f"ColorDw={color_dw}\n"
        "Position=0\n"
        "Control=0\n"
        "Mouse=0\n"
        "AutoStopControl=1\n"
        "AutoContextMenu=1\n",
        encoding="ascii",
    )
    if big_icon is not None:
        shutil.copy2(big_icon, path / "BigIcon.png")


def port_available(port: int, socket_type: int) -> bool:
    probe = socket.socket(socket.AF_INET6, socket_type)
    try:
        probe.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        probe.bind(("::", port))
        if socket_type == socket.SOCK_STREAM:
            probe.listen(1)
        return True
    except OSError:
        return False
    finally:
        probe.close()


def fetch_reference(port: int, timeout: float = 1.0) -> str:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        connection.request("GET", "/")
        response = connection.getresponse()
        body = response.read()
        if response.status != 200:
            raise OSError(f"reference server returned HTTP {response.status}")
        return body.decode("cp1252")
    finally:
        connection.close()


_REFERENCE_SECTION = re.compile(r"^(?P<indent> *)\[(?P<name>[^\]]+)\]\s*$")
_REFERENCE_FIELD = re.compile(
    r"^(?P<indent> *)(?P<name>[^=\s][^=]*)=(?P<value>.*)$"
)


def decode_reference_value(raw: str) -> str:
    """Decode the quoted byte escapes used by C4Network2Reference."""

    raw = raw.strip()
    if len(raw) < 2 or raw[0] != '"' or raw[-1] != '"':
        return raw
    source = raw[1:-1]
    decoded: list[str] = []
    index = 0
    named_escapes = {
        "a": "\x07",
        "b": "\x08",
        "f": "\x0c",
        "n": "\n",
        "r": "\r",
        "t": "\t",
        "v": "\x0b",
        '"': '"',
        "\\": "\\",
    }
    while index < len(source):
        character = source[index]
        if character != "\\":
            decoded.append(character)
            index += 1
            continue
        index += 1
        if index >= len(source):
            decoded.append("\\")
            break
        escaped = source[index]
        if escaped in "01234567":
            end = index + 1
            while end < len(source) and end < index + 3 and source[end] in "01234567":
                end += 1
            decoded.append(chr(int(source[index:end], 8)))
            index = end
            continue
        decoded.append(named_escapes.get(escaped, escaped))
        index += 1
    return "".join(decoded)


def reference_root_fields(reference: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in reference.splitlines():
        match = _REFERENCE_FIELD.fullmatch(line)
        if match is not None and len(match.group("indent")) == 0:
            fields[match.group("name").strip()] = decode_reference_value(
                match.group("value")
            )
    return fields


def reference_is_lobby(
    reference: str,
    *,
    title: str,
    max_players: int | None = None,
) -> bool:
    fields = reference_root_fields(reference)
    return (
        fields.get("State") == "Lobby"
        and fields.get("Title") == title
        and (
            max_players is None
            or fields.get("MaxPlayers") == str(max_players)
        )
    )


def _reference_section_records(
    reference: str,
    *,
    section_indent: int,
    section_name: str,
    parent_suffix: Sequence[str],
) -> list[dict[str, str]]:
    """Collect scalar fields from exact indentation-scoped sections."""

    stack: list[tuple[int, str]] = []
    records: list[dict[str, str]] = []
    current: dict[str, str] | None = None
    for line in reference.splitlines():
        section = _REFERENCE_SECTION.fullmatch(line)
        if section is not None:
            indent = len(section.group("indent"))
            while stack and stack[-1][0] >= indent:
                stack.pop()
            ancestry = [name for _depth, name in stack]
            name = section.group("name")
            if current is not None and indent <= section_indent:
                records.append(current)
                current = None
            if (
                indent == section_indent
                and name == section_name
                and ancestry[-len(parent_suffix) :] == list(parent_suffix)
            ):
                current = {}
            stack.append((indent, name))
            continue
        field = _REFERENCE_FIELD.fullmatch(line)
        if (
            current is not None
            and field is not None
            and len(field.group("indent")) == section_indent
        ):
            current[field.group("name").strip()] = decode_reference_value(
                field.group("value")
            )
    if current is not None:
        records.append(current)
    return records


def reference_player_names(reference: str) -> set[str]:
    return {
        record["Name"]
        for record in _reference_section_records(
            reference,
            section_indent=6,
            section_name="Player",
            parent_suffix=("Reference", "PlayerInfos", "Client"),
        )
        if "Name" in record
        and "Removed"
        not in {
            flag.strip()
            for flag in re.split(r"[,|]", record.get("Flags", ""))
        }
    }


def reference_has_player(reference: str, player_name: str) -> bool:
    return player_name in reference_player_names(reference)


def reference_client_records(reference: str) -> list[dict[str, str]]:
    return _reference_section_records(
        reference,
        section_indent=2,
        section_name="Client",
        parent_suffix=("Reference",),
    )


def reference_has_activated_clients(
    reference: str, client_names: set[str]
) -> bool:
    records = reference_client_records(reference)
    for name in client_names:
        matching = [record for record in records if record.get("Name") == name]
        if len(matching) != 1 or matching[0].get("Activated") != "true":
            return False
    return True


def reference_has_fleet(
    reference: str,
    *,
    state: str,
    max_players: int,
    player_names: set[str],
    client_names: set[str],
) -> bool:
    fields = reference_root_fields(reference)
    return (
        fields.get("State") == state
        and fields.get("MaxPlayers") == str(max_players)
        and reference_player_names(reference) == player_names
        and reference_has_activated_clients(reference, client_names)
    )


def file_log_statistics(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {
            "path": str(path),
            "exists": False,
            "bytes": 0,
            "lines": 0,
            "created_texture_info_lines": 0,
            "retained_texture_creation_info_lines": 0,
            "retained_source_mentions": 0,
        }
    data = path.read_bytes()
    lines = data.splitlines()
    return {
        "path": str(path),
        "exists": True,
        "bytes": len(data),
        "lines": len(lines),
        "created_texture_info_lines": sum(
            b"INFO" in line and b"Created texture" in line for line in lines
        ),
        "retained_texture_creation_info_lines": sum(
            b"INFO" in line
            and b"Created texture" in line
            and b"lc_gpu_retained_source" in line
            for line in lines
        ),
        "retained_source_mentions": data.count(b"lc_gpu_retained_source"),
    }


def retained_texture_log_failures(
    statistics: Sequence[dict[str, Any]],
) -> list[str]:
    count = sum(
        int(item["retained_texture_creation_info_lines"])
        for item in statistics
    )
    if count == 0:
        return []
    return [
        "retained GPU texture creation INFO spam was emitted "
        f"{count} time(s)"
    ]


def summarize_log_statistics(
    statistics: Sequence[dict[str, Any]],
) -> dict[str, int]:
    summed_fields = (
        "bytes",
        "lines",
        "created_texture_info_lines",
        "retained_texture_creation_info_lines",
        "retained_source_mentions",
    )
    return {
        "files": len(statistics),
        "existing_files": sum(bool(item["exists"]) for item in statistics),
        **{
            field: sum(int(item[field]) for item in statistics)
            for field in summed_fields
        },
    }


def process_log_statistics(record: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        {
            "stream": stream,
            **file_log_statistics(Path(record[path_key])),
        }
        for stream, path_key in (
            ("stdout", "stdout_path"),
            ("stderr", "stderr_path"),
            ("session", "session_log_path"),
        )
    ]


def scan_runtime_errors(paths: Iterable[Path]) -> list[str]:
    errors: list[str] = []
    seen: set[str] = set()
    for path in paths:
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for line in text.splitlines():
            if any(pattern.search(line) for pattern in RUNTIME_ERROR_PATTERNS):
                rendered = f"{path.name}: {line}"
                if rendered not in seen:
                    seen.add(rendered)
                    errors.append(rendered)
    return errors


class FleetRunner:
    def __init__(self, arguments: argparse.Namespace) -> None:
        self.arguments = arguments
        self.workspace = Path(__file__).resolve().parents[1]
        self.binary = Path(arguments.binary).expanduser().resolve()
        self.scenario = Path(arguments.scenario)
        if not self.scenario.is_absolute():
            self.scenario = (self.workspace / self.scenario).resolve()
        self.profile_big_icon = (
            self.workspace / DEFAULT_PROFILE_BIG_ICON
        ).resolve()
        self.artifact_dir = Path(arguments.artifact_dir).expanduser().resolve()
        self.scratch = Path(
            tempfile.mkdtemp(prefix="lc-harpoonrace-24-", dir=arguments.scratch_root)
        )
        self.processes: list[dict[str, Any]] = []
        self.host: dict[str, Any] | None = None
        self.clients: list[dict[str, Any]] = []
        self.failures: list[str] = []
        self.events_file = None
        self.started_monotonic = time.monotonic()
        self.started_utc = utc_now()
        self.admission_samples: list[dict[str, Any]] = []
        self.reference_before_start = ""
        self.go_command_sent: dict[str, Any] | None = None
        self.cleaned = False
        self.cleanup_in_progress = False

    def event(self, event: str, **fields: Any) -> dict[str, Any]:
        reserved = {"event", "timestamp_utc", "elapsed_ms"} & fields.keys()
        if reserved:
            raise ValueError(
                "event fields cannot replace reserved timeline keys: "
                + ", ".join(sorted(reserved))
            )
        payload = {
            "timestamp_utc": utc_now(),
            "elapsed_ms": round(
                (time.monotonic() - self.started_monotonic) * 1_000.0, 3
            ),
            "event": event,
            **fields,
        }
        if self.events_file is not None:
            self.events_file.write(
                json.dumps(
                    payload, default=json_fallback, sort_keys=True
                )
                + "\n"
            )
            self.events_file.flush()
        detail = " ".join(f"{key}={value}" for key, value in fields.items())
        print(f"[harpoonrace-fleet] {event}{(' ' + detail) if detail else ''}")
        return payload

    def validate_inputs(self) -> None:
        if not self.binary.is_file() or not os.access(self.binary, os.X_OK):
            raise FleetFailure(
                f"release binary not found or not executable: {self.binary}\n"
                "build it with: cargo build --release --offline --locked "
                "-p clonk-app --bin clonk-app"
            )
        if not self.scenario.is_dir():
            raise FleetFailure(f"HarpoonRace scenario not found: {self.scenario}")
        if not self.profile_big_icon.is_file():
            raise FleetFailure(
                "benchmark player BigIcon not found: "
                f"{self.profile_big_icon}"
            )
        if self.arguments.players <= 0:
            raise FleetFailure("--players must be positive")
        if self.arguments.measurement_seconds <= 0:
            raise FleetFailure("--measurement-seconds must be positive")
        finite_nonnegative = {
            "--host-timeout": self.arguments.host_timeout,
            "--join-timeout": self.arguments.join_timeout,
            "--completion-grace-seconds": (
                self.arguments.completion_grace_seconds
            ),
            "--join-stagger-ms": self.arguments.join_stagger_ms,
            "--settle-seconds": self.arguments.settle_seconds,
            "--minimum-simulation-fps": (
                self.arguments.minimum_simulation_fps
            ),
            "--minimum-presentation-fps": (
                self.arguments.minimum_presentation_fps
            ),
            "--maximum-graphics-p99-ms": (
                self.arguments.maximum_graphics_p99_ms
            ),
            "--maximum-network-lag-ms": (
                self.arguments.maximum_network_lag_ms
            ),
        }
        invalid = [
            name
            for name, value in finite_nonnegative.items()
            if not math.isfinite(value) or value < 0.0
        ]
        if invalid:
            raise FleetFailure(
                "arguments must be finite and nonnegative: "
                + ", ".join(invalid)
            )
        for required_positive in (
            "--host-timeout",
            "--join-timeout",
            "--minimum-simulation-fps",
            "--minimum-presentation-fps",
            "--maximum-graphics-p99-ms",
            "--maximum-network-lag-ms",
        ):
            if finite_nonnegative[required_positive] == 0.0:
                raise FleetFailure(f"{required_positive} must be positive")
        if self.arguments.window_width <= 0 or self.arguments.window_height <= 0:
            raise FleetFailure("window dimensions must be positive")
        maximum_port = (
            self.arguments.base_port + 10 + (self.arguments.players - 1) * 2 + 1
        )
        if self.arguments.base_port < 1024 or maximum_port > 65_535:
            raise FleetFailure(
                f"base port range is invalid: {self.arguments.base_port}..{maximum_port}"
            )

    def port_plan(self) -> dict[str, Any]:
        base = self.arguments.base_port
        return {
            "reference": base,
            "host_tcp": base + 1,
            "host_udp": base + 2,
            "clients": [
                {
                    "index": index,
                    "tcp": base + 10 + (index - 1) * 2,
                    "udp": base + 11 + (index - 1) * 2,
                }
                for index in range(1, self.arguments.players + 1)
            ],
        }

    def validate_ports(self, ports: dict[str, Any]) -> None:
        checks = [
            ("reference TCP", ports["reference"], socket.SOCK_STREAM),
            ("host TCP", ports["host_tcp"], socket.SOCK_STREAM),
            ("host UDP", ports["host_udp"], socket.SOCK_DGRAM),
        ]
        for client in ports["clients"]:
            checks.extend(
                [
                    (
                        f"client {client['index']:02d} TCP",
                        client["tcp"],
                        socket.SOCK_STREAM,
                    ),
                    (
                        f"client {client['index']:02d} UDP",
                        client["udp"],
                        socket.SOCK_DGRAM,
                    ),
                ]
            )
        unavailable = [
            f"{label}={port}"
            for label, port, socket_type in checks
            if not port_available(port, socket_type)
        ]
        if unavailable:
            raise FleetFailure("benchmark ports are occupied: " + ", ".join(unavailable))

    def process_environment(
        self,
        process_root: Path,
        log_root: Path,
        *,
        benchmark: bool,
    ) -> dict[str, str]:
        environment = controlled_process_environment(os.environ)
        environment.update(
            {
                "LC_INSTALL_ROOT": str(self.workspace),
                "LC_CONTENT_DIR": str(self.workspace / "content"),
                "LC_USER_DATA_DIR": str(process_root / "user"),
                "LC_CACHE_DIR": str(process_root / "cache"),
                "LC_TEMP_DIR": str(process_root / "tmp"),
                "LC_LOGS_DIR": str(log_root / "session"),
                "LC_PIN_SEED": "1",
                "RUST_BACKTRACE": environment.get("RUST_BACKTRACE", "1"),
            }
        )
        if benchmark:
            environment.update(
                {
                    "LC_APP_PRESENTATION_BENCHMARK_SECONDS": str(
                        self.arguments.measurement_seconds
                    ),
                    # A client that finishes first must remain in the lockstep
                    # participant set while slower peers collect their tails.
                    "LC_APP_PRESENTATION_BENCHMARK_KEEP_RUNNING": "1",
                }
            )
            # The supervisor applies the native budget and fleet performance
            # gates only after all reports exist. The app's assertion exits
            # immediately on failure, even when KEEP_RUNNING is set, which
            # would mutate the lockstep roster during peer measurements.
            environment.pop(
                "LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK", None
            )
        else:
            environment.pop("LC_APP_PRESENTATION_BENCHMARK_SECONDS", None)
            environment.pop(
                "LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK", None
            )
            environment.pop(
                "LC_APP_PRESENTATION_BENCHMARK_KEEP_RUNNING", None
            )
        return environment

    def host_command(
        self, config: Path, ports: dict[str, Any]
    ) -> list[str]:
        return [
            str(self.binary),
            "--config",
            str(config),
            "--player-name",
            "LoadHost",
            str(self.scenario),
            "/network",
            "/lobby",
            "/nosignup",
            "/console",
            f"/tcpport:{ports['host_tcp']}",
            f"/udpport:{ports['host_udp']}",
        ]

    def client_command(
        self,
        *,
        index: int,
        config: Path,
        profile: Path,
        tcp_port: int,
        udp_port: int,
        reference_port: int,
    ) -> list[str]:
        client_name = f"LoadClient-{index:02d}"
        return [
            str(self.binary),
            "--config",
            str(config),
            "--player-name",
            client_name,
            str(profile),
            f"/join:127.0.0.1:{reference_port}",
            "/nosignup",
            f"/tcpport:{tcp_port}",
            f"/udpport:{udp_port}",
        ]

    def dry_run_plan(self) -> dict[str, Any]:
        ports = self.port_plan()
        placeholder = Path("<RUN>")
        clients = [
            self.client_command(
                index=client["index"],
                config=placeholder
                / f"client-{client['index']:02d}"
                / "config.ini",
                profile=placeholder
                / "profiles"
                / f"LoadPlayer-{client['index']:02d}.c4p",
                tcp_port=client["tcp"],
                udp_port=client["udp"],
                reference_port=ports["reference"],
            )
            for client in ports["clients"]
        ]
        return {
            "topology": "one local classic console host plus rendered GUI clients",
            "players": self.arguments.players,
            "host_has_player": False,
            "scenario": str(self.scenario),
            "ports": ports,
            "host_command": self.host_command(
                placeholder / "host" / "config.ini", ports
            ),
            "first_client_command": clients[0],
            "last_client_command": clients[-1],
            "lobby_commands": [
                f"/set maxplayer {self.arguments.players}",
                "/start 0",
                "/quit",
            ],
            "measurement_seconds": self.arguments.measurement_seconds,
            "minimum_simulation_fps": self.arguments.minimum_simulation_fps,
            "minimum_presentation_fps": (
                self.arguments.minimum_presentation_fps
            ),
            "maximum_graphics_p99_ms": self.arguments.maximum_graphics_p99_ms,
            "maximum_network_lag_ms": self.arguments.maximum_network_lag_ms,
        }

    def open_artifacts(self) -> None:
        self.artifact_dir.mkdir(parents=True, exist_ok=False)
        self.events_file = (self.artifact_dir / "events.jsonl").open(
            "w", encoding="utf-8"
        )

    def write_manifest(self, ports: dict[str, Any]) -> None:
        cargo_lock = self.workspace / "Cargo.lock"
        content_revision = command_output(
            ["git", "rev-parse", "HEAD"], self.workspace / "content"
        )
        manifest = {
            "schema_version": 1,
            "started_utc": self.started_utc,
            "topology": {
                "kind": "single-machine-process-fleet",
                "host_processes": 1,
                "rendered_client_processes": self.arguments.players,
                "host_has_player": False,
                "limitations": [
                    "All rendered clients share one CPU and GPU; this measures "
                    "local fleet contention, not independent client hardware.",
                    "Startup-to-PlayerInfo-admission timing includes process, "
                    "asset, and profile startup and is not network RTT.",
                ],
            },
            "workspace": {
                "path": str(self.workspace),
                "commit": command_output(
                    ["git", "rev-parse", "HEAD"], self.workspace
                ),
                "status_porcelain": (
                    command_output(
                        ["git", "status", "--porcelain=v1"], self.workspace
                    )
                    or ""
                ).splitlines(),
                "content_revision": content_revision,
                "cargo_lock_sha256": (
                    sha256_file(cargo_lock) if cargo_lock.is_file() else None
                ),
            },
            "binary": {
                "path": str(self.binary),
                "sha256": sha256_file(self.binary),
                "size_bytes": self.binary.stat().st_size,
                "modified_ns": self.binary.stat().st_mtime_ns,
            },
            "scenario": {
                "path": str(self.scenario),
                "tree_sha256": sha256_tree(self.scenario),
            },
            "profile_big_icon": {
                "path": str(self.profile_big_icon),
                "sha256": sha256_file(self.profile_big_icon),
            },
            "runner": {
                "path": str(Path(__file__).resolve()),
                "sha256": sha256_file(Path(__file__).resolve()),
                "python": sys.version,
            },
            "machine": {
                "platform": platform.platform(),
                "system": platform.system(),
                "release": platform.release(),
                "machine": platform.machine(),
                "processor": platform.processor(),
                "cpu_count": os.cpu_count(),
            },
            "settings": {
                "players": self.arguments.players,
                "measurement_seconds": self.arguments.measurement_seconds,
                "join_stagger_ms": self.arguments.join_stagger_ms,
                "settle_seconds": self.arguments.settle_seconds,
                "window": {
                    "width": self.arguments.window_width,
                    "height": self.arguments.window_height,
                },
                "minimum_simulation_fps": self.arguments.minimum_simulation_fps,
                "minimum_presentation_fps": (
                    self.arguments.minimum_presentation_fps
                ),
                "maximum_graphics_p99_ms": (
                    self.arguments.maximum_graphics_p99_ms
                ),
                "maximum_network_lag_ms": (
                    self.arguments.maximum_network_lag_ms
                ),
                "ports": ports,
                "inherited_log_filter": {
                    "LC_LOG": os.environ.get("LC_LOG"),
                    "RUST_LOG": os.environ.get("RUST_LOG"),
                },
                "effective_log_filter": {
                    "LC_LOG": FLEET_LOG_FILTER,
                    "RUST_LOG": None,
                },
                "ignored_inherited_config_file": os.environ.get(
                    "LC_CONFIG_FILE"
                ),
            },
        }
        write_json(self.artifact_dir / "manifest.json", manifest)

    def launch(
        self,
        *,
        name: str,
        command: Sequence[str],
        environment: dict[str, str],
        output_root: Path,
        with_stdin: bool,
    ) -> dict[str, Any]:
        output_root.mkdir(parents=True, exist_ok=True)
        for directory in (
            Path(environment["LC_USER_DATA_DIR"]),
            Path(environment["LC_CACHE_DIR"]),
            Path(environment["LC_TEMP_DIR"]),
            Path(environment["LC_LOGS_DIR"]),
        ):
            directory.mkdir(parents=True, exist_ok=True)
        stdout_path = output_root / "stdout.log"
        stderr_path = output_root / "stderr.log"
        stdout_handle = stdout_path.open("w", encoding="utf-8")
        stderr_handle = stderr_path.open("w", encoding="utf-8")
        launched_monotonic = time.monotonic()
        process = subprocess.Popen(
            list(command),
            cwd=self.workspace,
            env=environment,
            stdin=subprocess.PIPE if with_stdin else subprocess.DEVNULL,
            stdout=stdout_handle,
            stderr=stderr_handle,
            text=True,
        )
        record = {
            "name": name,
            "command": list(command),
            "process": process,
            "stdout_path": stdout_path,
            "stderr_path": stderr_path,
            "session_log_path": Path(environment["LC_LOGS_DIR"]) / "Clonk.log",
            "stdout_handle": stdout_handle,
            "stderr_handle": stderr_handle,
            "launched_monotonic": launched_monotonic,
            "launched_utc": utc_now(),
            "exit_code": None,
            "benchmark_report_observed": False,
            "benchmark_report_observation": None,
            "go_observation": None,
            "all_report_barrier_observed": False,
            "supervisor_terminated": False,
        }
        self.processes.append(record)
        self.event("process-launched", name=name, pid=process.pid)
        return record

    def send_host_command(self, command: str) -> None:
        if self.host is None:
            raise FleetFailure("host process is not available")
        process = self.host["process"]
        if process.poll() is not None or process.stdin is None:
            raise FleetFailure(
                f"host exited before console command {command!r}: "
                f"{process.returncode}"
            )
        try:
            process.stdin.write(command + "\n")
            process.stdin.flush()
        except (BrokenPipeError, OSError) as error:
            raise FleetFailure(
                f"failed to send host console command {command!r}: {error}"
            ) from error
        event_name = (
            "go-command-sent" if command == "/start 0" else "host-command"
        )
        payload = self.event(event_name, command=command)
        if command == "/start 0":
            self.go_command_sent = {
                "command": command,
                "sent_at_utc": payload["timestamp_utc"],
                "sent_elapsed_ms": payload["elapsed_ms"],
                "source": "supervisor console write and flush",
            }

    def observe_go_logs(self) -> None:
        """Record timestamped scenario GO evidence when process logs expose it."""

        for record in getattr(self, "processes", []):
            if record["go_observation"] is not None:
                continue
            for path_key in ("stderr_path", "session_log_path"):
                path = Path(record[path_key])
                if not path.is_file():
                    continue
                source_timestamp = extract_go_log_timestamp(
                    path.read_text(encoding="utf-8", errors="replace")
                )
                if source_timestamp is None:
                    continue
                payload = self.event(
                    "go-observed",
                    process=record["name"],
                    source_timestamp_utc=source_timestamp,
                    source_file=str(path.relative_to(self.artifact_dir)),
                )
                record["go_observation"] = {
                    "source_timestamp_utc": source_timestamp,
                    "source_file": str(path.relative_to(self.artifact_dir)),
                    "observed_at_utc": payload["timestamp_utc"],
                    "observed_elapsed_ms": payload["elapsed_ms"],
                }
                break

    def wait_for_reference(
        self,
        description: str,
        timeout_seconds: float,
        predicate: Callable[[str], bool],
    ) -> str:
        if self.host is None:
            raise FleetFailure("host process is not available")
        deadline = time.monotonic() + timeout_seconds
        last_error = "reference has not answered"
        while time.monotonic() < deadline:
            host_code = self.host["process"].poll()
            if host_code is not None:
                raise FleetFailure(
                    f"host exited while waiting for {description}: {host_code}"
                )
            try:
                reference = fetch_reference(self.arguments.base_port)
                if predicate(reference):
                    self.event("reference-ready", condition=description)
                    return reference
                last_error = f"last reference did not satisfy {description}"
            except (OSError, http.client.HTTPException) as error:
                last_error = str(error)
            time.sleep(0.1)
        raise FleetFailure(
            f"timed out after {timeout_seconds:.1f}s waiting for "
            f"{description}: {last_error}"
        )

    def prepare_profiles_and_configs(
        self, ports: dict[str, Any]
    ) -> tuple[Path, list[dict[str, Any]]]:
        host_root = self.scratch / "host"
        host_config = host_root / "config.ini"
        write_process_config(
            host_config,
            name="LoadHost",
            tcp_port=ports["host_tcp"],
            udp_port=ports["host_udp"],
            reference_port=ports["reference"],
            width=self.arguments.window_width,
            height=self.arguments.window_height,
        )
        (self.artifact_dir / "host").mkdir(parents=True, exist_ok=True)
        shutil.copy2(host_config, self.artifact_dir / "host" / "config.initial.ini")

        profiles_root = self.scratch / "profiles"
        profile_evidence = self.artifact_dir / "profiles.initial"
        profile_evidence.mkdir(parents=True, exist_ok=True)
        clients: list[dict[str, Any]] = []
        profile_manifest: list[dict[str, Any]] = []
        for port_spec in ports["clients"]:
            index = port_spec["index"]
            client_root = self.scratch / f"client-{index:02d}"
            config = client_root / "config.ini"
            profile_name = f"LoadPlayer-{index:02d}"
            client_name = f"LoadClient-{index:02d}"
            profile = profiles_root / f"{profile_name}.c4p"
            write_distinct_profile(
                profile,
                profile_name,
                index,
                self.arguments.players,
                big_icon=self.profile_big_icon,
            )
            write_process_config(
                config,
                name=client_name,
                tcp_port=port_spec["tcp"],
                udp_port=port_spec["udp"],
                reference_port=ports["reference"],
                width=self.arguments.window_width,
                height=self.arguments.window_height,
            )
            client_evidence = (
                self.artifact_dir / f"client-{index:02d}"
            )
            client_evidence.mkdir(parents=True, exist_ok=True)
            shutil.copy2(
                config, client_evidence / "config.initial.ini"
            )
            evidence = profile_evidence / f"{profile_name}.Player.txt"
            shutil.copy2(profile / "Player.txt", evidence)
            icon_evidence = profile_evidence / f"{profile_name}.BigIcon.png"
            shutil.copy2(profile / "BigIcon.png", icon_evidence)
            profile_manifest.append(
                {
                    "index": index,
                    "client_name": client_name,
                    "player_name": profile_name,
                    "profile_path": str(profile),
                    "player_core_sha256": sha256_file(profile / "Player.txt"),
                    "big_icon_sha256": sha256_file(
                        profile / "BigIcon.png"
                    ),
                    "tcp_port": port_spec["tcp"],
                    "udp_port": port_spec["udp"],
                }
            )
            clients.append(
                {
                    "index": index,
                    "root": client_root,
                    "config": config,
                    "profile": profile,
                    "client_name": client_name,
                    "player_name": profile_name,
                    "tcp_port": port_spec["tcp"],
                    "udp_port": port_spec["udp"],
                }
            )
        write_json(self.artifact_dir / "profiles.initial.json", profile_manifest)
        return host_config, clients

    def launch_host(self, config: Path, ports: dict[str, Any]) -> None:
        root = self.scratch / "host"
        output = self.artifact_dir / "host"
        environment = self.process_environment(root, output, benchmark=False)
        self.host = self.launch(
            name="host",
            command=self.host_command(config, ports),
            environment=environment,
            output_root=output,
            with_stdin=True,
        )

    def launch_clients(
        self,
        specs: Sequence[dict[str, Any]],
        reference_port: int,
    ) -> None:
        for offset, spec in enumerate(specs):
            output = self.artifact_dir / f"client-{spec['index']:02d}"
            environment = self.process_environment(
                spec["root"], output, benchmark=True
            )
            command = self.client_command(
                index=spec["index"],
                config=spec["config"],
                profile=spec["profile"],
                tcp_port=spec["tcp_port"],
                udp_port=spec["udp_port"],
                reference_port=reference_port,
            )
            process = self.launch(
                name=spec["client_name"],
                command=command,
                environment=environment,
                output_root=output,
                with_stdin=False,
            )
            process.update(spec)
            self.clients.append(process)
            if (
                offset + 1 < len(specs)
                and self.arguments.join_stagger_ms > 0
            ):
                time.sleep(self.arguments.join_stagger_ms / 1_000.0)

    def wait_for_player_admission(self) -> None:
        expected = {client["player_name"]: client for client in self.clients}
        expected_player_names = set(expected)
        expected_client_names = {
            client["client_name"] for client in self.clients
        }
        admitted: set[str] = set()
        deadline = time.monotonic() + self.arguments.join_timeout
        last_reference = ""
        complete_reference = False
        while time.monotonic() < deadline:
            if self.host is None or self.host["process"].poll() is not None:
                code = None if self.host is None else self.host["process"].returncode
                raise FleetFailure(
                    f"host exited while players were joining: {code}"
                )
            early_exits = [
                (client["name"], client["process"].poll())
                for client in self.clients
                if client["process"].poll() is not None
            ]
            if early_exits:
                rendered = ", ".join(
                    f"{name}={code}" for name, code in early_exits
                )
                raise FleetFailure(
                    "clients exited before PlayerInfo admission: " + rendered
                )
            try:
                last_reference = fetch_reference(self.arguments.base_port)
            except (OSError, http.client.HTTPException):
                time.sleep(0.1)
                continue
            now = time.monotonic()
            current_players = reference_player_names(last_reference)
            for player_name, client in expected.items():
                if player_name in admitted:
                    continue
                if player_name in current_players:
                    admitted.add(player_name)
                    latency_ms = (
                        now - client["launched_monotonic"]
                    ) * 1_000.0
                    sample = {
                        "index": client["index"],
                        "client_name": client["client_name"],
                        "player_name": player_name,
                        "startup_to_player_info_admission_ms": latency_ms,
                    }
                    self.admission_samples.append(sample)
                    self.event(
                        "player-info-admitted",
                        player=player_name,
                        admitted=f"{len(admitted)}/{len(expected)}",
                        startup_to_player_info_admission_ms=round(
                            latency_ms, 3
                        ),
                    )
            if reference_has_fleet(
                last_reference,
                state="Lobby",
                max_players=self.arguments.players,
                player_names=expected_player_names,
                client_names=expected_client_names,
            ):
                complete_reference = True
                break
            time.sleep(0.1)

        if not complete_reference:
            current_players = reference_player_names(last_reference)
            missing = sorted(expected_player_names - current_players)
            raise FleetFailure(
                f"timed out after {self.arguments.join_timeout:.1f}s waiting "
                "for one current coexisting fleet reference"
                + (
                    f"; missing PlayerInfos: {', '.join(missing)}"
                    if missing
                    else ""
                )
            )
        write_json(
            self.artifact_dir / "join-admission-samples.json",
            {
                "schema_version": 1,
                "measurement": (
                    "client process launch to first exact host-reference "
                    "PlayerInfo observation; includes startup and is not RTT"
                ),
                "poll_interval_ms": 100,
                "samples": sorted(
                    self.admission_samples, key=lambda sample: sample["index"]
                ),
            },
        )

    def settle_joined_lobby(self) -> str:
        deadline = time.monotonic() + self.arguments.settle_seconds
        while time.monotonic() < deadline:
            if self.host is None or self.host["process"].poll() is not None:
                raise FleetFailure("host exited during joined-lobby settling")
            exited = [
                f"{client['name']}={client['process'].poll()}"
                for client in self.clients
                if client["process"].poll() is not None
            ]
            if exited:
                raise FleetFailure(
                    "clients exited during joined-lobby settling: "
                    + ", ".join(exited)
                )
            time.sleep(0.1)
        try:
            reference = fetch_reference(self.arguments.base_port)
        except (OSError, http.client.HTTPException) as error:
            raise FleetFailure(
                "failed to obtain the final current coexisting fleet "
                f"reference: {error}"
            ) from error
        if not reference_has_fleet(
            reference,
            state="Lobby",
            max_players=self.arguments.players,
            player_names={
                client["player_name"] for client in self.clients
            },
            client_names={
                client["client_name"] for client in self.clients
            },
        ):
            raise FleetFailure(
                "final lobby reference does not contain the current "
                "coexisting fleet"
            )
        self.event(
            "joined-lobby-settled",
            seconds=self.arguments.settle_seconds,
        )
        return reference

    def wait_for_clients(self) -> None:
        deadline = (
            time.monotonic()
            + benchmark_completion_timeout_seconds(
                self.arguments.measurement_seconds,
                self.arguments.completion_grace_seconds,
            )
        )
        next_progress = time.monotonic() + 10.0
        while time.monotonic() < deadline:
            completed = [
                client
                for client in self.clients
                if client["benchmark_report_observed"]
            ]
            if len(completed) == len(self.clients):
                disconnected = [
                    f"{client['name']}={client['process'].poll()}"
                    for client in self.clients
                    if client["process"].poll() is not None
                ]
                if disconnected:
                    self.failures.append(
                        "clients disconnected before the all-report barrier: "
                        + ", ".join(disconnected)
                    )
                else:
                    for client in self.clients:
                        client["all_report_barrier_observed"] = True
                break
            if self.host is None or self.host["process"].poll() is not None:
                self.failures.append(
                    "host exited before all client benchmarks completed"
                )
                break
            premature_exits = []
            for client in self.clients:
                if client["benchmark_report_observed"]:
                    if client["process"].poll() is not None:
                        premature_exits.append(
                            f"{client['name']} disconnected after reporting"
                        )
                    continue
                code = client["process"].poll()
                stdout = (
                    client["stdout_path"].read_text(
                        encoding="utf-8", errors="replace"
                    )
                    if client["stdout_path"].is_file()
                    else ""
                )
                stderr = (
                    client["stderr_path"].read_text(
                        encoding="utf-8", errors="replace"
                    )
                    if client["stderr_path"].is_file()
                    else ""
                )
                try:
                    extract_benchmark_report(stdout, stderr)
                except (ValueError, OverflowError):
                    if code is not None:
                        premature_exits.append(
                            f"{client['name']} exited before a complete "
                            f"report (code {code})"
                        )
                    continue
                client["benchmark_report_observed"] = True
                payload = self.event(
                    "client-benchmark-reported",
                    client=client["name"],
                    completed=(
                        f"{len(completed) + 1}/{len(self.clients)}"
                    ),
                )
                client["benchmark_report_observation"] = {
                    "observed_at_utc": payload["timestamp_utc"],
                    "observed_elapsed_ms": payload["elapsed_ms"],
                    "stdout_file_modified_at_utc": file_modified_utc(
                        client["stdout_path"]
                    ),
                }
                completed.append(client)
                # A complete report proves the app has passed GO. Scan all
                # process logs once at that point instead of repeatedly
                # rereading growing logs during scenario startup.
                self.observe_go_logs()
            if premature_exits:
                self.failures.extend(premature_exits)
                break
            now = time.monotonic()
            if now >= next_progress:
                self.event(
                    "report-completion-progress",
                    completed=f"{len(completed)}/{len(self.clients)}",
                )
                next_progress = now + 10.0
            time.sleep(0.2)
        missing = [
            client
            for client in self.clients
            if not client["benchmark_report_observed"]
        ]
        if missing:
            self.failures.append(
                "client benchmark completion timeout: "
                + ", ".join(client["name"] for client in missing)
            )
        self.event(
            "client-benchmarks-reported",
            completed=sum(
                client["benchmark_report_observed"]
                for client in self.clients
            ),
            expected=len(self.clients),
        )
        self.observe_go_logs()

    def write_benchmark_timing(self) -> None:
        """Persist only directly observed wall-clock timing evidence."""

        clients: list[dict[str, Any]] = []
        for client in sorted(
            self.clients, key=lambda record: record["index"]
        ):
            report = None
            try:
                stdout = (
                    client["stdout_path"].read_text(
                        encoding="utf-8", errors="replace"
                    )
                    if client["stdout_path"].is_file()
                    else ""
                )
                stderr = (
                    client["stderr_path"].read_text(
                        encoding="utf-8", errors="replace"
                    )
                    if client["stderr_path"].is_file()
                    else ""
                )
                report = extract_benchmark_report(stdout, stderr)
            except (ValueError, OverflowError):
                pass
            clients.append(
                {
                    "index": client["index"],
                    "client_name": client["name"],
                    "go_observation": client["go_observation"],
                    "warmup": {
                        "configured_duration_seconds": (
                            BENCHMARK_WARMUP_SECONDS
                        ),
                        "started_at_utc": None,
                    },
                    "measurement": {
                        "started_at_utc": None,
                        "finished_at_utc": None,
                        "reported_elapsed_seconds": (
                            None
                            if report is None
                            else report["elapsed_seconds"]
                        ),
                    },
                    "report": {
                        "emitted_at_utc": None,
                        **(
                            client["benchmark_report_observation"]
                            or {
                                "observed_at_utc": None,
                                "observed_elapsed_ms": None,
                                "stdout_file_modified_at_utc": (
                                    file_modified_utc(
                                        client["stdout_path"]
                                    )
                                ),
                            }
                        ),
                    },
                }
            )
        write_json(
            self.artifact_dir / "benchmark-timing.json",
            {
                "schema_version": 1,
                "go_command": self.go_command_sent,
                "host_go_observation": (
                    None
                    if self.host is None
                    else self.host["go_observation"]
                ),
                "clients": clients,
                "observability": {
                    "go_command": (
                        "supervisor wall clock at the successful /start 0 "
                        "console write and flush"
                    ),
                    "go_observation": (
                        "source timestamp from an exact INFO Go! process-log "
                        "line, plus the later supervisor observation time"
                    ),
                    "warmup_and_measurement": (
                        "the app reports configured warmup duration and measured "
                        "elapsed seconds, but does not emit wall-clock start/end "
                        "markers; unavailable timestamps remain null"
                    ),
                    "report": (
                        "the machine report has no source timestamp; emitted time "
                        "remains null, while supervisor observation and stdout "
                        "filesystem mtime are retained as distinct evidence"
                    ),
                },
            },
        )

    def finish_fleet_after_reports(self) -> None:
        if not self.clients or not all(
            client["benchmark_report_observed"] for client in self.clients
        ):
            return
        # Ask the host to leave through its exact console command, then stop
        # every held-open benchmark client in one supervisor pass.  The client
        # benchmark deliberately has no in-game UI automation hook; SIGTERM is
        # recorded as an expected post-measurement supervisor action, never as
        # a passing substitute for a missing metric/context pair.
        if self.host is not None and self.host["process"].poll() is None:
            try:
                self.send_host_command("/quit")
            except FleetFailure as error:
                self.failures.append(str(error))
        live_clients = [
            client
            for client in self.clients
            if client["process"].poll() is None
        ]
        for client in live_clients:
            client["supervisor_terminated"] = True
            client["process"].terminate()
        self.event(
            "fleet-release-sent",
            clients=len(live_clients),
            mechanism="SIGTERM-after-all-reports",
        )
        deadline = time.monotonic() + 10.0
        remaining = live_clients
        while remaining and time.monotonic() < deadline:
            remaining = [
                client
                for client in remaining
                if client["process"].poll() is None
            ]
            time.sleep(0.1)
        for client in remaining:
            client["process"].kill()
            self.failures.append(
                f"{client['name']} required SIGKILL after benchmark release"
            )
        for client in live_clients:
            try:
                client["process"].wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                client["process"].kill()
                client["process"].wait()
                self.failures.append(
                    f"{client['name']} did not reap after benchmark release"
                )
            client["exit_code"] = client["process"].returncode
        if self.host is not None and self.host["process"].poll() is None:
            try:
                self.host["process"].wait(timeout=15.0)
            except subprocess.TimeoutExpired:
                self.failures.append(
                    "host did not exit within 15s after synchronized /quit"
                )
        if self.host is not None:
            self.host["exit_code"] = self.host["process"].poll()
        self.event("fleet-release-complete")

    def close_output_handles(self) -> None:
        for record in self.processes:
            for key in ("stdout_handle", "stderr_handle"):
                handle = record.get(key)
                if handle is not None and not handle.closed:
                    handle.flush()
                    handle.close()

    def terminate_recorded_processes(self) -> None:
        running = [
            record
            for record in self.processes
            if record["process"].poll() is None
        ]
        for record in running:
            record["process"].terminate()
        deadline = time.monotonic() + 5.0
        while running and time.monotonic() < deadline:
            running = [
                record
                for record in running
                if record["process"].poll() is None
            ]
            time.sleep(0.1)
        for record in running:
            record["process"].kill()
        for record in self.processes:
            try:
                record["process"].wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                record["process"].kill()
                record["process"].wait()
            record["exit_code"] = record["process"].returncode

    def graceful_host_shutdown(self) -> None:
        if self.host is None or self.host["process"].poll() is not None:
            return
        try:
            self.send_host_command("/quit")
        except FleetFailure as error:
            self.failures.append(str(error))
            return
        try:
            self.host["process"].wait(timeout=15.0)
        except subprocess.TimeoutExpired:
            self.failures.append("host did not exit within 15s after /quit")
        self.host["exit_code"] = self.host["process"].poll()
        if self.host["exit_code"] == 0:
            self.event("host-exited", exit_code=0)

    def collect_results(self) -> dict[str, Any]:
        raw_clients: list[dict[str, Any]] = []
        process_logs: list[dict[str, Any]] = []
        all_log_statistics: list[dict[str, Any]] = []
        pooled_graphics_ms: list[float] = []
        simulation_fps: list[float] = []
        presentation_fps: list[float] = []
        per_client_p99_ms: list[float] = []
        preferred_message_route_peer_counts: list[float] = []
        nonnegative_ping_peer_counts: list[float] = []
        nonnegative_lag_peer_counts: list[float] = []
        maximum_message_route_ping_ms: list[float] = []
        maximum_message_route_lag_ms: list[float] = []
        maximum_packet_loss: list[float] = []
        control_presend: list[float] = []
        average_control_send_time_us: list[float] = []
        protocol_totals = {"tcp": 0, "udp": 0, "unknown": 0}
        local_client_ids: list[int] = []

        for client in sorted(self.clients, key=lambda record: record["index"]):
            stdout = (
                client["stdout_path"].read_text(
                    encoding="utf-8", errors="replace"
                )
                if client["stdout_path"].is_file()
                else ""
            )
            stderr = (
                client["stderr_path"].read_text(
                    encoding="utf-8", errors="replace"
                )
                if client["stderr_path"].is_file()
                else ""
            )
            client_failures: list[str] = []
            report = None
            client_log_statistics = process_log_statistics(client)
            all_log_statistics.extend(client_log_statistics)
            process_logs.append(
                {
                    "process": client["name"],
                    "files": client_log_statistics,
                    "totals": summarize_log_statistics(
                        client_log_statistics
                    ),
                }
            )
            expected_post_report_exit = (
                client["all_report_barrier_observed"]
                and client["exit_code"] in (-signal.SIGTERM, 0)
            )
            if client["exit_code"] != 0 and not expected_post_report_exit:
                client_failures.append(
                    f"process exit code is {client['exit_code']!r}, expected 0"
                )
            try:
                report = extract_benchmark_report(stdout, stderr)
                client_failures.extend(
                    benchmark_failures(
                        report,
                        expected_seconds=self.arguments.measurement_seconds,
                        expected_players=self.arguments.players,
                        minimum_simulation_fps=(
                            self.arguments.minimum_simulation_fps
                        ),
                        minimum_presentation_fps=(
                            self.arguments.minimum_presentation_fps
                        ),
                        maximum_graphics_p99_ms=(
                            self.arguments.maximum_graphics_p99_ms
                        ),
                        maximum_network_lag_ms=(
                            self.arguments.maximum_network_lag_ms
                        ),
                    )
                )
            except (ValueError, OverflowError) as error:
                client_failures.append(str(error))

            runtime_errors = scan_runtime_errors(
                [
                    client["stdout_path"],
                    client["stderr_path"],
                    client["session_log_path"],
                ]
            )
            client_failures.extend(runtime_errors)
            client_failures.extend(
                retained_texture_log_failures(client_log_statistics)
            )
            if report is not None:
                graphics_ms = [
                    sample / 1_000_000.0
                    for sample in report["graphics_pass_samples_ns"]
                ]
                pooled_graphics_ms.extend(graphics_ms)
                simulation_fps.append(report["simulation_fps"])
                presentation_fps.append(
                    report["presentation_submission_fps"]
                )
                per_client_p99_ms.append(report["graphics_pass_p99_ms"])
                network = report["network_evidence"]
                local_client_ids.append(network["local_client_id"])
                preferred_message_route_peer_counts.append(
                    network["preferred_message_route_peer_count"]
                )
                nonnegative_ping_peer_counts.append(
                    network["nonnegative_ping_peer_count"]
                )
                nonnegative_lag_peer_counts.append(
                    network["nonnegative_lag_peer_count"]
                )
                if network["max_nonnegative_ping_ms"] >= 0:
                    maximum_message_route_ping_ms.append(
                        network["max_nonnegative_ping_ms"]
                    )
                if network["max_nonnegative_lag_ms"] >= 0:
                    maximum_message_route_lag_ms.append(
                        network["max_nonnegative_lag_ms"]
                    )
                maximum_packet_loss.append(network["max_packet_loss"])
                control_presend.append(network["control_presend"])
                average_control_send_time_us.append(
                    network["avg_control_send_time_us"]
                )
                protocol_totals["tcp"] += network[
                    "tcp_preferred_message_routes"
                ]
                protocol_totals["udp"] += network[
                    "udp_preferred_message_routes"
                ]
                protocol_totals["unknown"] += network[
                    "unknown_preferred_message_routes"
                ]
            if client_failures:
                self.failures.extend(
                    f"{client['name']}: {failure}"
                    for failure in client_failures
                )
            raw_clients.append(
                {
                    "index": client["index"],
                    "client_name": client["client_name"],
                    "player_name": client["player_name"],
                    "pid": client["process"].pid,
                    "exit_code": client["exit_code"],
                    "supervisor_terminated_after_all_reports": (
                        client["supervisor_terminated"]
                    ),
                    "all_report_barrier_observed": client[
                        "all_report_barrier_observed"
                    ],
                    "command": client["command"],
                    "report": report,
                    "failures": client_failures,
                }
            )

        expected_local_client_ids = set(range(1, self.arguments.players + 1))
        observed_local_client_ids = set(local_client_ids)
        if (
            len(local_client_ids) == len(self.clients)
            and (
                observed_local_client_ids != expected_local_client_ids
                or len(local_client_ids) != len(observed_local_client_ids)
            )
        ):
            self.failures.append(
                "benchmark client local IDs are not the exact nonhost fleet: "
                f"observed={sorted(local_client_ids)} "
                f"expected={sorted(expected_local_client_ids)}"
            )

        host_errors: list[str] = []
        if self.host is not None:
            host_log_statistics = process_log_statistics(self.host)
            all_log_statistics.extend(host_log_statistics)
            process_logs.append(
                {
                    "process": "host",
                    "files": host_log_statistics,
                    "totals": summarize_log_statistics(
                        host_log_statistics
                    ),
                }
            )
            if self.host["exit_code"] != 0:
                host_errors.append(
                    f"host process exit code is {self.host['exit_code']!r}, "
                    "expected 0"
                )
            host_errors.extend(
                scan_runtime_errors(
                    [
                        self.host["stdout_path"],
                        self.host["stderr_path"],
                        self.host["session_log_path"],
                    ]
                )
            )
            host_errors.extend(
                retained_texture_log_failures(host_log_statistics)
            )
            self.failures.extend(f"host: {failure}" for failure in host_errors)

        write_json(
            self.artifact_dir / "presentation-raw.json",
            {
                "schema_version": 1,
                "clients": raw_clients,
            },
        )
        admission_values = [
            sample["startup_to_player_info_admission_ms"]
            for sample in self.admission_samples
        ]
        return {
            "schema_version": 1,
            "result": "pass" if not self.failures else "fail",
            "started_utc": self.started_utc,
            "finished_utc": utc_now(),
            "duration_seconds": time.monotonic() - self.started_monotonic,
            "players_requested": self.arguments.players,
            "players_admitted_before_start": len(self.admission_samples),
            "clients_with_reports": sum(
                client["report"] is not None for client in raw_clients
            ),
            "clients_passing": sum(
                client["report"] is not None and not client["failures"]
                for client in raw_clients
            ),
            "acceptance": {
                "minimum_simulation_fps": (
                    self.arguments.minimum_simulation_fps
                ),
                "minimum_presentation_fps": (
                    self.arguments.minimum_presentation_fps
                ),
                "maximum_graphics_p99_ms_exclusive": (
                    self.arguments.maximum_graphics_p99_ms
                ),
                "maximum_network_lag_ms_inclusive": (
                    self.arguments.maximum_network_lag_ms
                ),
                "preferred_message_route_topology": (
                    "full mesh benchmark requirement; C++ relay fallback "
                    "remains valid engine behavior"
                ),
                "automatic_graphics_skips": (
                    "reported; bounded by presentation FPS"
                ),
                "native_average_graphics_budget_ms": NATIVE_GAME_TICK_MS,
            },
            "startup_to_player_info_admission_ms": sample_statistics(
                admission_values
            ),
            "logging": {
                "effective_filter": FLEET_LOG_FILTER,
                "totals": summarize_log_statistics(all_log_statistics),
                "processes": process_logs,
            },
            "presentation": {
                "simulation_fps_across_clients": sample_statistics(
                    simulation_fps
                ),
                "submission_fps_across_clients": sample_statistics(
                    presentation_fps
                ),
                "per_client_graphics_p99_ms": sample_statistics(
                    per_client_p99_ms
                ),
                "pooled_graphics_pass_ms": sample_statistics(
                    pooled_graphics_ms
                ),
                "pooled_graphics_pass_sample_count": len(
                    pooled_graphics_ms
                ),
            },
            "network": {
                "preferred_message_route_peer_count_across_clients": (
                    sample_statistics(preferred_message_route_peer_counts)
                ),
                "nonnegative_ping_peer_count_across_clients": (
                    sample_statistics(nonnegative_ping_peer_counts)
                ),
                "nonnegative_lag_peer_count_across_clients": (
                    sample_statistics(nonnegative_lag_peer_counts)
                ),
                "max_nonnegative_message_route_ping_ms_across_clients": (
                    sample_statistics(maximum_message_route_ping_ms)
                ),
                "max_nonnegative_message_route_lag_ms_across_clients": (
                    sample_statistics(maximum_message_route_lag_ms)
                ),
                "max_packet_loss_across_clients": sample_statistics(
                    maximum_packet_loss
                ),
                "control_presend_across_clients": sample_statistics(
                    control_presend
                ),
                "avg_control_send_time_us_across_clients": sample_statistics(
                    average_control_send_time_us
                ),
                "preferred_message_route_protocol_totals": protocol_totals,
                "local_client_ids": sorted(local_client_ids),
            },
            "host_errors": host_errors,
            "failures": self.failures,
            "limitations": [
                f"All {self.arguments.players} rendered clients ran on one "
                "machine and shared its CPU and GPU. Compare only matching "
                "topology/hardware fingerprints.",
                "The post-measurement snapshot exports current preferred-route "
                "ping/lag/loss, not a time series. Startup-to-admission timing "
                "must not be reported as network RTT.",
            ],
        }

    def cleanup(self) -> None:
        if self.cleaned or self.cleanup_in_progress:
            return
        self.cleanup_in_progress = True
        interrupted: KeyboardInterrupt | None = None
        try:
            try:
                self.graceful_host_shutdown()
            except KeyboardInterrupt as error:
                interrupted = error
            while True:
                try:
                    self.terminate_recorded_processes()
                    break
                except KeyboardInterrupt as error:
                    interrupted = error
                    # Cancellation never converts an incomplete reap into a
                    # completed cleanup. Retry until every recorded child has
                    # been waited, then propagate the interruption.
            self.close_output_handles()
            self.observe_go_logs()
            if self.events_file is not None:
                self.events_file.close()
                self.events_file = None
            if not self.arguments.keep_scratch:
                shutil.rmtree(self.scratch)
            else:
                print(
                    f"[harpoonrace-fleet] scratch-retained path={self.scratch}"
                )
            self.cleaned = True
        finally:
            self.cleanup_in_progress = False
        if interrupted is not None:
            raise interrupted

    def run(self) -> int:
        self.validate_inputs()
        if self.arguments.dry_run:
            print(json.dumps(self.dry_run_plan(), indent=2, sort_keys=True))
            shutil.rmtree(self.scratch)
            self.cleaned = True
            return 0

        ports = self.port_plan()
        self.validate_ports(ports)
        self.open_artifacts()
        try:
            self.write_manifest(ports)
            host_config, client_specs = self.prepare_profiles_and_configs(
                ports
            )
            self.event(
                "benchmark-started",
                artifacts=self.artifact_dir,
                scratch=self.scratch,
            )
            self.launch_host(host_config, ports)
            lobby_reference = self.wait_for_reference(
                "HarpoonRace lobby reference",
                self.arguments.host_timeout,
                lambda reference: reference_is_lobby(
                    reference,
                    title="HarpoonRace",
                ),
            )
            (self.artifact_dir / "reference-lobby-initial.txt").write_text(
                lobby_reference, encoding="cp1252"
            )
            self.send_host_command(
                f"/set maxplayer {self.arguments.players}"
            )
            raised_reference = self.wait_for_reference(
                f"MaxPlayers={self.arguments.players}",
                self.arguments.host_timeout,
                lambda reference: reference_is_lobby(
                    reference,
                    title="HarpoonRace",
                    max_players=self.arguments.players,
                ),
            )
            (
                self.artifact_dir / "reference-lobby-max-players.txt"
            ).write_text(raised_reference, encoding="cp1252")
            self.launch_clients(client_specs, ports["reference"])
            self.wait_for_player_admission()
            self.reference_before_start = self.settle_joined_lobby()
            (
                self.artifact_dir / "reference-before-start.txt"
            ).write_text(self.reference_before_start, encoding="cp1252")
            self.send_host_command("/start 0")
            self.wait_for_clients()
            self.finish_fleet_after_reports()
        except (FleetFailure, OSError, subprocess.SubprocessError) as error:
            self.failures.append(str(error))
            self.event("benchmark-failed-early", error=str(error))
        finally:
            self.cleanup()

        summary = self.collect_results()
        self.write_benchmark_timing()
        write_json(self.artifact_dir / "summary.json", summary)
        print(
            f"[harpoonrace-fleet] result={summary['result']} "
            f"artifacts={self.artifact_dir}"
        )
        if self.failures:
            for failure in self.failures:
                print(f"[harpoonrace-fleet] failure: {failure}", file=sys.stderr)
            return 1
        return 0


def default_artifact_dir(workspace: Path) -> Path:
    run_id = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    return (
        workspace
        / "target"
        / "network-benchmark"
        / f"harpoonrace-24-{run_id}"
    )


def build_argument_parser() -> argparse.ArgumentParser:
    workspace = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(
        description=(
            "Host a classic HarpoonRace lobby, admit 24 distinct player "
            "clients, and retain process-level performance evidence."
        )
    )
    parser.add_argument(
        "--binary",
        default=os.environ.get(
            "LC_APP_BINARY", str(workspace / "target/release/clonk-app")
        ),
        help="release clonk-app binary",
    )
    parser.add_argument(
        "--scenario",
        default=str(workspace / DEFAULT_SCENARIO),
        help="HarpoonRace .c4s directory",
    )
    parser.add_argument(
        "--artifact-dir",
        default=str(default_artifact_dir(workspace)),
        help="new directory in which retained results are written",
    )
    parser.add_argument(
        "--scratch-root",
        default="/private/tmp" if platform.system() == "Darwin" else None,
        help="parent for disposable isolated process state",
    )
    parser.add_argument(
        "--players",
        type=int,
        default=24,
        help="number of distinct remote player profiles (default: 24)",
    )
    parser.add_argument(
        "--measurement-seconds",
        type=int,
        default=60,
        help="measured running interval after the app's 2s warmup",
    )
    parser.add_argument(
        "--base-port",
        type=int,
        default=31_111,
        help="reference port; the remaining fleet ports follow it",
    )
    parser.add_argument(
        "--host-timeout",
        type=float,
        default=180.0,
        help="seconds allowed for each host-lobby readiness phase",
    )
    parser.add_argument(
        "--join-timeout",
        type=float,
        default=300.0,
        help="seconds allowed for all PlayerInfos to appear",
    )
    parser.add_argument(
        "--completion-grace-seconds",
        type=float,
        default=300.0,
        help="extra seconds beyond the measured window for load/start/exit",
    )
    parser.add_argument(
        "--join-stagger-ms",
        type=float,
        default=50.0,
        help="delay between GUI client launches",
    )
    parser.add_argument(
        "--settle-seconds",
        type=float,
        default=10.0,
        help="joined-lobby settling interval before /start 0",
    )
    parser.add_argument(
        "--minimum-simulation-fps",
        type=float,
        default=35.0,
        help="required simulation FPS for every client",
    )
    parser.add_argument(
        "--minimum-presentation-fps",
        type=float,
        default=35.0,
        help="required successful presentation FPS for every client",
    )
    parser.add_argument(
        "--maximum-graphics-p99-ms",
        type=float,
        default=25.0,
        help="exclusive per-client p99 graphics-pass limit",
    )
    parser.add_argument(
        "--maximum-network-lag-ms",
        type=float,
        default=100.0,
        help=(
            "inclusive maximum current preferred-message-route lag per client"
        ),
    )
    parser.add_argument("--window-width", type=int, default=800)
    parser.add_argument("--window-height", type=int, default=600)
    parser.add_argument(
        "--keep-scratch",
        action="store_true",
        help="retain isolated user/cache/temp trees for diagnosis",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="validate inputs and print the exact topology without launching",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = build_argument_parser().parse_args(argv)
    runner: FleetRunner | None = None
    try:
        runner = FleetRunner(arguments)
        return runner.run()
    except (FleetFailure, OSError, ValueError) as error:
        if runner is not None:
            runner.cleanup()
        print(f"harpoonrace fleet benchmark: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        if runner is not None:
            runner.failures.append("interrupted")
            runner.cleanup()
        print("harpoonrace fleet benchmark: interrupted", file=sys.stderr)
        return 130


def handle_termination_signal(_signum: int, _frame: Any) -> None:
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, handle_termination_signal)
    raise SystemExit(main())
