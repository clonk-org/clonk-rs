#!/usr/bin/env python3
"""Run a visible paired 24-player Hazard retained-GPU benchmark."""

from __future__ import annotations

import argparse
import configparser
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
import signal
import subprocess
import sys
from pathlib import Path
from typing import Any, Sequence


HAZARD_PLAYERS = 24
HAZARD_CREW_ID = "HZCK"
HAZARD_PLAYER_SECTIONS = tuple(f"Player{index}" for index in range(1, 5))
DEFAULT_HAZARD_SCENARIO = Path("content/Hazard.c4f/DM_Baldoon.c4s")
DEFAULT_HAZARD_TITLE = "DM - Baldoon"
ARM_PORT_STRIDE = 64
OWNED_CHILD_SHUTDOWN_SECONDS = 90
CPU_STAGE_KEYS = (
    "frame_preparation_ns",
    "validation_ns",
    "texture_synchronization_ns",
    "stream_packing_upload_ns",
    "command_encoding_ns",
    "drawable_acquisition_ns",
    "queue_submission_ns",
    "presentation_ns",
    "named_total_ns",
    "unclassified_ns",
    "overrun_ns",
)
RENDERER_COUNTER_KEYS = (
    "resident_source_textures",
    "created_source_textures",
    "full_upload_calls",
    "full_upload_bytes",
    "dirty_upload_calls",
    "dirty_upload_bytes",
    "draw_calls",
    "quad_draw_calls",
    "sprite_draw_calls",
    "object_sprite_draw_calls",
    "landscape_draw_calls",
    "shader_landscape_draw_calls",
    "solid_draw_calls",
    "solid_rect_draw_calls",
    "monitor_gamma_draw_calls",
    "presentation_draw_calls",
    "total_draw_calls",
    "compatible_resource_runs",
    "generic_vertices",
    "generic_vertex_upload_bytes",
    "quad_instances",
    "sprite_instances",
    "object_sprite_instances",
    "solid_rect_instances",
    "quad_instance_upload_bytes",
    "sprite_instance_upload_bytes",
    "object_sprite_upload_bytes",
    "solid_rect_upload_bytes",
    "landscape_instances",
    "landscape_instance_upload_bytes",
)
FRONTEND_COUNTER_KEYS = (
    "generic_sprite_fallbacks",
    "spatial_fog_fallbacks",
    "precomputed_fog_modulation_fallbacks",
    "texture_indent_fallbacks",
    "owner_mask_fallbacks",
    "physical_texture_tile_fallbacks",
    "fog_expanded_chunks",
)
GPU_TIMESTAMP_VALIDITIES = (
    "valid",
    "invalid_period",
    "counter_rollover",
    "invalid_duration",
)


class HazardBenchmarkFailure(RuntimeError):
    """An expected paired-benchmark setup or evidence failure."""


def _load_script_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise HazardBenchmarkFailure(f"cannot load benchmark helper {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _profile_validator_module():
    return _load_script_module(
        "hazard_retained_gpu_profile_validator",
        Path(__file__).resolve().with_name(
            "run_arso_morf_stippel_gpu_benchmark.py"
        ),
    )


def nearest_rank_percentile(
    samples: Sequence[float], quantile: float
) -> float:
    if not samples:
        raise ValueError("cannot calculate a percentile without samples")
    if not 0.0 <= quantile <= 1.0:
        raise ValueError("quantile must be between zero and one")
    ordered = sorted(float(sample) for sample in samples)
    rank = max(1, math.ceil(quantile * len(ordered)))
    return ordered[rank - 1]


def sample_statistics(samples: Sequence[float]) -> dict[str, Any]:
    finite = [float(sample) for sample in samples]
    if not finite:
        return {"sample_count": 0}
    if not all(math.isfinite(sample) for sample in finite):
        raise HazardBenchmarkFailure("profile statistics contain non-finite data")
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


def json_sha256(value: Any) -> str:
    encoded = json.dumps(
        value,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def read_json_object(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise HazardBenchmarkFailure(f"cannot read JSON artifact {path}: {error}") from error
    if not isinstance(value, dict):
        raise HazardBenchmarkFailure(f"JSON artifact is not an object: {path}")
    return value


def validate_hzck_scenario_contract(scenario: Path) -> dict[str, Any]:
    """Pin the checked-in Hazard player templates to one HZCK crew each."""

    scenario_file = scenario / "Scenario.txt"
    parser = configparser.ConfigParser(
        interpolation=None,
        strict=True,
    )
    parser.optionxform = str
    try:
        scenario_bytes = scenario_file.read_bytes()
        parser.read_string(scenario_bytes.decode("cp1252"))
    except (OSError, configparser.Error) as error:
        raise HazardBenchmarkFailure(
            f"cannot read Hazard scenario contract {scenario_file}: {error}"
        ) from error

    observed_sections = [
        section for section in parser.sections() if section.startswith("Player")
    ]
    if observed_sections != list(HAZARD_PLAYER_SECTIONS):
        raise HazardBenchmarkFailure(
            "Hazard player template sections differ from Player1..Player4: "
            f"{observed_sections}"
        )
    expected_crew = f"{HAZARD_CREW_ID}=1"
    invalid = [
        section
        for section in HAZARD_PLAYER_SECTIONS
        if parser.get(section, "Crew", fallback=None) != expected_crew
    ]
    if invalid:
        raise HazardBenchmarkFailure(
            "Hazard player templates do not each declare Crew=HZCK=1: "
            + ", ".join(invalid)
        )
    return {
        "schema_version": 1,
        "scenario_path": str(scenario.resolve()),
        "scenario_file": str(scenario_file.resolve()),
        "scenario_file_fingerprint": {
            "sha256": hashlib.sha256(scenario_bytes).hexdigest(),
            "size_bytes": len(scenario_bytes),
        },
        "crew_id": HAZARD_CREW_ID,
        "crew_per_player": 1,
        "player_sections": list(HAZARD_PLAYER_SECTIONS),
        "players_requested": HAZARD_PLAYERS,
        "runtime_reports_expose_crew_definition_ids": False,
        "inference": (
            "HZCK identity is inferred from this byte-hashed Scenario.txt "
            "contract assigning Crew=HZCK=1 to Player1..Player4, combined "
            "with the runtime exact 24-player, 24-live-player-crew, and "
            "24-crew-object census. Runtime reports do not expose crew "
            "definition IDs."
        ),
    }


def validate_hzck_runtime_evidence(
    artifact_dir: Path, *, expected_clients: int
) -> dict[str, Any]:
    """Require every visible fleet report to prove 24 live crew objects."""

    evidence_path = artifact_dir / "presentation-raw.json"
    try:
        evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
        clients = evidence["clients"]
    except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise HazardBenchmarkFailure(
            f"cannot read fleet presentation evidence {evidence_path}: {error}"
        ) from error
    if evidence.get("schema_version") != 1 or not isinstance(clients, list):
        raise HazardBenchmarkFailure(
            f"invalid fleet presentation evidence at {evidence_path}"
        )
    if len(clients) != expected_clients:
        raise HazardBenchmarkFailure(
            f"visible client evidence count is {len(clients)}, expected "
            f"{expected_clients}"
        )

    required = {
        "runtime_players": HAZARD_PLAYERS,
        "synchronized_player_infos": HAZARD_PLAYERS,
        "activated_nonhost_clients": expected_clients,
        "runtime_crew_objects": HAZARD_PLAYERS,
        "runtime_players_with_live_crew": HAZARD_PLAYERS,
    }
    validated_names: list[str] = []
    for index, client in enumerate(clients, start=1):
        if not isinstance(client, dict):
            raise HazardBenchmarkFailure(
                f"visible client evidence {index} is not an object"
            )
        name = client.get("client_name")
        report = client.get("report")
        context = (
            report.get("benchmark_context")
            if isinstance(report, dict)
            else None
        )
        label = name if isinstance(name, str) and name else f"client {index}"
        if not isinstance(context, dict):
            raise HazardBenchmarkFailure(
                f"{label} has no benchmark context"
            )
        for field, expected in required.items():
            observed = context.get(field)
            if type(observed) is not int or observed != expected:
                raise HazardBenchmarkFailure(
                    f"{label} benchmark context {field}={observed!r}, "
                    f"expected {expected}"
                )
        validated_names.append(label)
    return {
        "schema_version": 1,
        "evidence_path": str(evidence_path.resolve()),
        "clients_validated": len(clients),
        "client_names": validated_names,
        **required,
        "crew_id_inference": HAZARD_CREW_ID,
    }


def _read_presentation_clients(artifact_dir: Path) -> list[dict[str, Any]]:
    path = artifact_dir / "presentation-raw.json"
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        clients = value["clients"]
    except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise HazardBenchmarkFailure(
            f"cannot read fleet presentation evidence {path}: {error}"
        ) from error
    if value.get("schema_version") != 1 or not isinstance(clients, list):
        raise HazardBenchmarkFailure(f"invalid fleet presentation evidence at {path}")
    return clients


def _counter_distributions(
    frames: Sequence[dict[str, Any]],
    *,
    section: str,
    keys: Sequence[str],
) -> dict[str, dict[str, Any]]:
    return {
        key: sample_statistics(
            [frame[section][key] for frame in frames if key in frame[section]]
        )
        for key in keys
    }


def collect_retained_gpu_evidence(
    artifact_dir: Path,
    *,
    expected_clients: int,
    profiles_required: bool,
    expected_timestamp_request: bool,
    minimum_schema_version: int | None,
) -> dict[str, Any]:
    """Validate and retain clonk-org/clonk-rs#267 profiles from every client."""

    clients = _read_presentation_clients(artifact_dir)
    if len(clients) != expected_clients:
        raise HazardBenchmarkFailure(
            f"visible client evidence count is {len(clients)}, expected "
            f"{expected_clients}"
        )
    validator = _profile_validator_module()
    raw_profiles: list[dict[str, Any]] = []
    frames: list[dict[str, Any]] = []
    timestamp_states: list[tuple[bool, bool, bool]] = []
    timestamp_telemetry = {
        "dropped_frames": 0,
        "readback_errors": 0,
        "device_discontinuities": 0,
    }
    fingerprint_hashes: set[str] = set()
    gpu_pass_samples: dict[str, list[float]] = {}
    gpu_pass_validity_counts: dict[str, dict[str, int]] = {}
    for position, client in enumerate(clients, start=1):
        if not isinstance(client, dict):
            raise HazardBenchmarkFailure(
                f"visible client evidence {position} is not an object"
            )
        index = client.get("index")
        if type(index) is not int or index <= 0:
            raise HazardBenchmarkFailure(
                f"visible client evidence {position} has invalid index {index!r}"
            )
        name = client.get("client_name")
        label = name if isinstance(name, str) and name else f"client-{index:02d}"
        client_dir = artifact_dir / f"client-{index:02d}"
        lines: list[str] = []
        source_paths: list[str] = []
        for filename in ("stdout.log", "stderr.log"):
            path = client_dir / filename
            if path.is_file():
                lines.extend(
                    path.read_text(encoding="utf-8", errors="replace").splitlines()
                )
                source_paths.append(str(path.relative_to(artifact_dir)))
        try:
            profile = validator.parse_retained_gpu_profile(
                lines,
                required=profiles_required,
                minimum_schema_version=minimum_schema_version,
                timestamp_sample_policy=(
                    validator.TIMESTAMP_SAMPLE_POLICY_TOLERANT_RAW
                ),
            )
        except validator.BenchmarkFailure as error:
            raise HazardBenchmarkFailure(
                f"{label} retained GPU profile is invalid: {error}"
            ) from error
        if profile is None:
            continue
        timestamp = profile["timestamp_queries"]
        if timestamp["requested"] != expected_timestamp_request:
            raise HazardBenchmarkFailure(
                f"{label} timestamp request is {timestamp['requested']}, "
                f"expected {expected_timestamp_request}"
            )
        report = client.get("report")
        if not isinstance(report, dict):
            raise HazardBenchmarkFailure(f"{label} has no presentation report")
        durations = [frame["end_to_end_ns"] for frame in profile["frames"]]
        retained_submissions = report.get("retained_gpu_present_submissions")
        if retained_submissions != len(durations):
            raise HazardBenchmarkFailure(
                f"{label} profile frame count is {len(durations)} but "
                f"{retained_submissions!r} retained submissions were reported"
            )
        if report.get("graphics_pass_samples_ns") != durations:
            raise HazardBenchmarkFailure(
                f"{label} profile durations do not match legacy raw graphics samples"
            )
        fingerprint_hash = validator.json_sha256(profile["fingerprint"])
        fingerprint_hashes.add(fingerprint_hash)
        state = (
            timestamp["requested"],
            timestamp["supported"],
            timestamp["enabled"],
        )
        timestamp_states.append(state)
        for key in timestamp_telemetry:
            timestamp_telemetry[key] += timestamp[key]
        frames.extend(profile["frames"])
        for gpu_frame in profile["gpu_timestamp_frames"]:
            for gpu_pass in gpu_frame["passes"]:
                pass_name = gpu_pass["pass"]
                validity = gpu_pass["validity"]
                counts = gpu_pass_validity_counts.setdefault(
                    pass_name,
                    {name: 0 for name in GPU_TIMESTAMP_VALIDITIES},
                )
                counts[validity] += 1
                if validity == "valid":
                    gpu_pass_samples.setdefault(pass_name, []).append(
                        float(gpu_pass["duration_ns"])
                    )
        raw_profiles.append(
            {
                "client_index": index,
                "client_name": label,
                "source_logs": source_paths,
                "profile_sha256": validator.json_sha256(profile),
                "profile": profile,
            }
        )

    if raw_profiles and len(raw_profiles) != expected_clients:
        raise HazardBenchmarkFailure(
            "retained GPU profiles were emitted by only "
            f"{len(raw_profiles)} of {expected_clients} visible clients"
        )
    if len(fingerprint_hashes) > 1:
        raise HazardBenchmarkFailure(
            "visible clients reported different retained GPU fingerprints"
        )
    if len(set(timestamp_states)) > 1:
        raise HazardBenchmarkFailure(
            "visible clients reported different timestamp-query availability"
        )

    if not raw_profiles:
        timestamp_evidence = {
            "availability": "not_emitted",
            "reason": "optional_baseline_profile_not_emitted",
            "requested": expected_timestamp_request,
            "supported": None,
            "enabled": None,
            "dropped_frames": None,
            "readback_errors": None,
            "device_discontinuities": None,
            "gpu_pass_duration_ns": {},
            "gpu_pass_validity_counts": {},
        }
    else:
        requested, supported, enabled = timestamp_states[0]
        if enabled:
            availability = "available"
            reason = None
        elif requested and not supported:
            availability = "unavailable"
            reason = "adapter_does_not_support_timestamp_queries"
        else:
            availability = "not_requested"
            reason = "timestamp_queries_not_requested"
        timestamp_evidence = {
            "availability": availability,
            "reason": reason,
            "requested": requested,
            "supported": supported,
            "enabled": enabled,
            **timestamp_telemetry,
            "gpu_pass_duration_ns": {
                name: sample_statistics(samples)
                for name, samples in sorted(gpu_pass_samples.items())
            },
            "gpu_pass_validity_counts": {
                name: counts
                for name, counts in sorted(gpu_pass_validity_counts.items())
            },
        }
    evidence = {
        "schema_version": 1,
        "profiles_required": profiles_required,
        "minimum_profile_schema_version": minimum_schema_version,
        "profiles_retained": len(raw_profiles),
        "clients_expected": expected_clients,
        "fingerprint_sha256": (
            next(iter(fingerprint_hashes)) if fingerprint_hashes else None
        ),
        "end_to_end_ns": sample_statistics(
            [frame["end_to_end_ns"] for frame in frames]
        ),
        "cpu_stage_ns": _counter_distributions(
            frames, section="cpu", keys=CPU_STAGE_KEYS
        ),
        "renderer_counters": _counter_distributions(
            frames, section="renderer", keys=RENDERER_COUNTER_KEYS
        ),
        "frontend_counters": _counter_distributions(
            frames, section="frontend_capture", keys=FRONTEND_COUNTER_KEYS
        ),
        "composition_recreated_frames": sum(
            bool(frame["renderer"]["composition_recreated"])
            for frame in frames
        ),
        "timestamp_queries": timestamp_evidence,
        "raw_profiles": raw_profiles,
    }
    write_json(artifact_dir / "retained-gpu-evidence.json", evidence)
    return evidence


def validate_paired_input_fingerprints(
    baseline: dict[str, Any], candidate: dict[str, Any]
) -> dict[str, Any]:
    """Require identical Harpoon inputs except for the measured binary."""

    try:
        baseline_invariant = baseline["matrix_invariant"]
        candidate_invariant = candidate["matrix_invariant"]
        baseline_binary_sha = baseline_invariant["binary"]["sha256"]
        candidate_binary_sha = candidate_invariant["binary"]["sha256"]
        baseline_scenario_sha = baseline["scenario"]["tree_sha256"]
        candidate_scenario_sha = candidate["scenario"]["tree_sha256"]
        baseline_full_sha = baseline["full_sha256"]
        candidate_full_sha = candidate["full_sha256"]
    except (KeyError, TypeError) as error:
        raise HazardBenchmarkFailure(
            f"paired child input fingerprint is incomplete: {error}"
        ) from error
    if baseline.get("schema_version") != 1 or candidate.get("schema_version") != 1:
        raise HazardBenchmarkFailure(
            "paired child input fingerprint schema_version must be 1"
        )
    if baseline_scenario_sha != candidate_scenario_sha:
        raise HazardBenchmarkFailure(
            "paired scenario fingerprint differs: "
            f"{baseline_scenario_sha!r} != {candidate_scenario_sha!r}"
        )
    baseline_without_binary = {
        key: value
        for key, value in baseline_invariant.items()
        if key != "binary"
    }
    candidate_without_binary = {
        key: value
        for key, value in candidate_invariant.items()
        if key != "binary"
    }
    if baseline_without_binary != candidate_without_binary:
        raise HazardBenchmarkFailure(
            "paired Harpoon inputs differ outside the measured binary"
        )
    if (
        not isinstance(baseline_binary_sha, str)
        or not baseline_binary_sha
        or not isinstance(candidate_binary_sha, str)
        or not candidate_binary_sha
    ):
        raise HazardBenchmarkFailure("paired binary fingerprints are invalid")
    if baseline_binary_sha == candidate_binary_sha:
        raise HazardBenchmarkFailure(
            "baseline and candidate binary fingerprints are identical"
        )
    return {
        "schema_version": 1,
        "baseline_full_sha256": baseline_full_sha,
        "candidate_full_sha256": candidate_full_sha,
        "scenario_tree_sha256": baseline_scenario_sha,
        "shared_input_sha256": json_sha256(baseline_without_binary),
        "binary_sha256": {
            "baseline": baseline_binary_sha,
            "candidate": candidate_binary_sha,
        },
    }


def load_child_arm_contract(
    artifact_dir: Path, *, expected_clients: int
) -> dict[str, Any]:
    """Load one fleet result and validate its topology/input invariants."""

    manifest = read_json_object(artifact_dir / "manifest.json")
    summary = read_json_object(artifact_dir / "summary.json")
    final_fingerprint = read_json_object(
        artifact_dir / "input-fingerprint-final.json"
    )
    try:
        initial_fingerprint = manifest["input_fingerprint"]
        topology_players = manifest["topology"]["remote_player_profiles"]
        settings = manifest["settings"]
        summary_players = summary["players_requested"]
        summary_clients = summary["clients_requested"]
        presentation_required = summary["acceptance"][
            "presentation_required"
        ]
    except (KeyError, TypeError) as error:
        raise HazardBenchmarkFailure(
            f"child arm contract is incomplete at {artifact_dir}: {error}"
        ) from error
    if manifest.get("schema_version") != 1 or summary.get("schema_version") != 1:
        raise HazardBenchmarkFailure(
            f"child arm artifact schema is invalid at {artifact_dir}"
        )
    if initial_fingerprint != final_fingerprint:
        raise HazardBenchmarkFailure(
            f"input fingerprint changed during the child arm at {artifact_dir}"
        )
    expected_values = {
        "topology remote_player_profiles": topology_players,
        "settings players": settings.get("players"),
        "summary players_requested": summary_players,
    }
    invalid_players = {
        label: value
        for label, value in expected_values.items()
        if value != HAZARD_PLAYERS
    }
    if invalid_players:
        raise HazardBenchmarkFailure(
            f"child arm is not an exact {HAZARD_PLAYERS}-player fleet: "
            f"{invalid_players}"
        )
    if settings.get("clients") != expected_clients or summary_clients != expected_clients:
        raise HazardBenchmarkFailure(
            "child arm visible-client topology differs from requested "
            f"{expected_clients}"
        )
    if settings.get("runtime_only") is not False or presentation_required is not True:
        raise HazardBenchmarkFailure(
            "child arm did not require visible retained-GPU presentation"
        )
    return {
        "manifest": manifest,
        "summary": summary,
        "input_fingerprint": initial_fingerprint,
        "final_input_fingerprint": final_fingerprint,
    }


def validate_paired_gpu_fingerprints(
    baseline: dict[str, Any], candidate: dict[str, Any]
) -> dict[str, Any]:
    """Pin both timestamp-enabled arms to one exact GPU/config fingerprint."""

    candidate_profiles = candidate.get("raw_profiles")
    if not isinstance(candidate_profiles, list) or not candidate_profiles:
        raise HazardBenchmarkFailure(
            "candidate retained GPU fingerprint is unavailable"
        )
    baseline_profiles = baseline.get("raw_profiles")
    if not isinstance(baseline_profiles, list) or not baseline_profiles:
        raise HazardBenchmarkFailure(
            "baseline retained GPU fingerprint is unavailable"
        )
    arm_hashes = {
        "baseline": baseline.get("fingerprint_sha256"),
        "candidate": candidate.get("fingerprint_sha256"),
    }
    try:
        baseline_fingerprint = baseline_profiles[0]["profile"]["fingerprint"]
        candidate_fingerprint = candidate_profiles[0]["profile"]["fingerprint"]
    except (KeyError, TypeError) as error:
        raise HazardBenchmarkFailure(
            f"paired retained GPU fingerprint is incomplete: {error}"
        ) from error
    if baseline_fingerprint != candidate_fingerprint:
        raise HazardBenchmarkFailure(
            "baseline and candidate GPU fingerprints differ"
        )
    return {
        "schema_version": 1,
        "comparison": "matched",
        "shared_normalized_fingerprint_sha256": json_sha256(
            baseline_fingerprint
        ),
        "arm_fingerprint_sha256": arm_hashes,
    }


def default_artifact_dir(workspace: Path) -> Path:
    run_id = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    return (
        workspace
        / "target"
        / "network-benchmark"
        / f"hazard-24-player-gpu-ab-{run_id}"
    )


def harpoon_arm_command(
    arguments: argparse.Namespace,
    *,
    workspace: Path,
    binary: Path,
    artifact_dir: Path,
    base_port: int,
) -> list[str]:
    scenario = Path(arguments.scenario)
    if not scenario.is_absolute():
        scenario = workspace / scenario
    return [
        sys.executable,
        str(workspace / "scripts" / "run_harpoonrace_24_player_benchmark.py"),
        "--binary",
        str(binary),
        "--scenario",
        str(scenario),
        "--scenario-title",
        arguments.scenario_title,
        "--artifact-dir",
        str(artifact_dir),
        "--players",
        str(HAZARD_PLAYERS),
        "--clients",
        str(arguments.clients),
        "--control-mode",
        "2",
        "--measurement-seconds",
        str(arguments.measurement_seconds),
        "--base-port",
        str(base_port),
        "--minimum-simulation-fps",
        str(arguments.minimum_simulation_fps),
        "--minimum-presentation-fps",
        str(arguments.minimum_presentation_fps),
        "--maximum-graphics-p99-ms",
        str(arguments.maximum_graphics_p99_ms),
        "--maximum-network-lag-ms",
        str(arguments.maximum_network_lag_ms),
        "--input-probe-interval-ms",
        str(arguments.input_probe_interval_ms),
        "--maximum-input-latency-ms",
        str(arguments.maximum_input_latency_ms),
        "--minimum-input-success-percent",
        str(arguments.minimum_input_success_percent),
        "--window-width",
        str(arguments.window_width),
        "--window-height",
        str(arguments.window_height),
        "--skip-sf5b-crew-assertion",
    ]


def arm_environment(
    inherited: dict[str, str], *, timestamp_queries_requested: bool
) -> dict[str, str]:
    environment = inherited.copy()
    if timestamp_queries_requested:
        environment["LC_GPU_TIMESTAMP_QUERIES"] = "1"
    else:
        environment.pop("LC_GPU_TIMESTAMP_QUERIES", None)
    return environment


def _terminate_owned_child(process: subprocess.Popen[Any]) -> None:
    """Give the Harpoon supervisor time to reap its visible process fleet."""

    if process.poll() is not None:
        return
    terminate_requested = False
    force_group_reap = False
    while True:
        try:
            if not terminate_requested:
                try:
                    process.terminate()
                except ProcessLookupError:
                    pass
                terminate_requested = True
            if force_group_reap:
                try:
                    if os.name == "posix":
                        os.killpg(process.pid, signal.SIGKILL)
                    else:
                        process.kill()
                except ProcessLookupError:
                    pass
                process.wait()
            else:
                process.wait(timeout=OWNED_CHILD_SHUTDOWN_SECONDS)
            return
        except (KeyboardInterrupt, subprocess.TimeoutExpired):
            force_group_reap = True


def run_owned_child(command: Sequence[str], environment: dict[str, str]) -> int:
    """Run one arm while retaining ownership across cancellation or failure."""

    process = subprocess.Popen(
        command,
        env=environment,
        start_new_session=os.name == "posix",
    )
    try:
        return process.wait()
    except BaseException:
        _terminate_owned_child(process)
        raise


def validate_paired_arguments(
    arguments: argparse.Namespace, *, workspace: Path, artifact_dir: Path
) -> None:
    if artifact_dir.exists():
        raise HazardBenchmarkFailure(
            f"artifact directory already exists: {artifact_dir}"
        )
    if not 1 <= arguments.clients <= HAZARD_PLAYERS:
        raise HazardBenchmarkFailure(
            f"--clients must be in 1..={HAZARD_PLAYERS}"
        )
    if arguments.measurement_seconds <= 0:
        raise HazardBenchmarkFailure("--measurement-seconds must be positive")
    if arguments.input_probe_interval_ms < 0:
        raise HazardBenchmarkFailure(
            "--input-probe-interval-ms must be nonnegative"
        )
    if arguments.window_width <= 0 or arguments.window_height <= 0:
        raise HazardBenchmarkFailure("window dimensions must be positive")
    maximum_port = (
        arguments.base_port
        + ARM_PORT_STRIDE
        + 10
        + (arguments.clients - 1) * 2
        + 1
    )
    if arguments.base_port < 1024 or maximum_port > 65_535:
        raise HazardBenchmarkFailure(
            f"paired port range is invalid: {arguments.base_port}..{maximum_port}"
        )
    for label, path_value in (
        ("baseline", arguments.baseline_binary),
        ("candidate", arguments.candidate_binary),
    ):
        path = Path(path_value).expanduser().resolve()
        if not path.is_file() or not os.access(path, os.X_OK):
            raise HazardBenchmarkFailure(
                f"{label} binary not found or not executable: {path}"
            )
    for label, path_value in (
        ("baseline", arguments.baseline_source_root),
        ("candidate", arguments.candidate_source_root),
    ):
        path = Path(path_value).expanduser().resolve()
        if not path.is_dir() or not (path / "Cargo.toml").is_file():
            raise HazardBenchmarkFailure(
                f"{label} source root is not a clonk-rs worktree: {path}"
            )
    scenario = Path(arguments.scenario)
    if not scenario.is_absolute():
        scenario = workspace / scenario
    if not scenario.is_dir():
        raise HazardBenchmarkFailure(f"Hazard scenario not found: {scenario}")


def collect_run_provenance(
    arguments: argparse.Namespace, *, workspace: Path
) -> dict[str, Any]:
    validator = _profile_validator_module()
    script = Path(__file__).resolve()
    harpoon = script.with_name("run_harpoonrace_24_player_benchmark.py")
    profile_validator = script.with_name(
        "run_arso_morf_stippel_gpu_benchmark.py"
    )
    try:
        return {
            "schema_version": 1,
            "sources": {
                "baseline": validator.collect_source_provenance(
                    Path(arguments.baseline_source_root).expanduser().resolve()
                ),
                "candidate": validator.collect_source_provenance(
                    Path(arguments.candidate_source_root).expanduser().resolve()
                ),
            },
            "binaries": {
                "baseline": validator.binary_provenance(
                    Path(arguments.baseline_binary).expanduser().resolve()
                ),
                "candidate": validator.binary_provenance(
                    Path(arguments.candidate_binary).expanduser().resolve()
                ),
            },
            "harness": {
                "workspace": str(workspace.resolve()),
                "paired_runner": {
                    "path": str(script),
                    **validator.file_fingerprint(script),
                },
                "fleet_runner": {
                    "path": str(harpoon),
                    **validator.file_fingerprint(harpoon),
                },
                "profile_validator": {
                    "path": str(profile_validator),
                    **validator.file_fingerprint(profile_validator),
                },
                "python": sys.version,
            },
        }
    except validator.BenchmarkFailure as error:
        raise HazardBenchmarkFailure(
            f"cannot collect benchmark provenance: {error}"
        ) from error


def _arm_public_summary(
    *,
    artifact_dir: Path,
    command: Sequence[str],
    return_code: int | None,
    contract: dict[str, Any] | None,
    crew_evidence: dict[str, Any] | None,
    gpu_evidence: dict[str, Any] | None,
    failures: Sequence[str],
) -> dict[str, Any]:
    summarized_gpu_evidence = (
        None
        if gpu_evidence is None
        else {
            key: value
            for key, value in gpu_evidence.items()
            if key != "raw_profiles"
        }
    )
    return {
        "result": (
            "pass"
            if return_code == 0
            and contract is not None
            and contract["summary"].get("result") == "pass"
            and not failures
            else "fail"
        ),
        "artifact_dir": str(artifact_dir),
        "command": list(command),
        "return_code": return_code,
        "child_result": (
            None if contract is None else contract["summary"].get("result")
        ),
        "input_fingerprint": (
            None if contract is None else contract["input_fingerprint"]
        ),
        "hzck_runtime_evidence": crew_evidence,
        "retained_gpu_evidence": summarized_gpu_evidence,
        "retained_gpu_evidence_artifact": (
            None
            if gpu_evidence is None
            else str(artifact_dir / "retained-gpu-evidence.json")
        ),
        "failures": list(failures),
    }


def run_paired_benchmark(
    arguments: argparse.Namespace,
    *,
    workspace: Path,
    artifact_dir: Path,
) -> dict[str, Any]:
    """Run baseline then candidate, retaining each arm even if one fails."""

    validate_paired_arguments(
        arguments, workspace=workspace, artifact_dir=artifact_dir
    )
    scenario = Path(arguments.scenario)
    if not scenario.is_absolute():
        scenario = (workspace / scenario).resolve()
    scenario_contract = validate_hzck_scenario_contract(scenario)
    initial_provenance = collect_run_provenance(arguments, workspace=workspace)
    artifact_dir.mkdir(parents=True, exist_ok=False)

    arm_specs = (
        (
            "baseline",
            Path(arguments.baseline_binary).expanduser().resolve(),
            True,
            True,
            2,
            arguments.base_port,
        ),
        (
            "candidate",
            Path(arguments.candidate_binary).expanduser().resolve(),
            True,
            True,
            2,
            arguments.base_port + ARM_PORT_STRIDE,
        ),
    )
    commands = {
        name: harpoon_arm_command(
            arguments,
            workspace=workspace,
            binary=binary,
            artifact_dir=artifact_dir / name,
            base_port=base_port,
        )
        for name, binary, _, _, _, base_port in arm_specs
    }
    write_json(
        artifact_dir / "manifest.json",
        {
            "schema_version": 1,
            "scenario_contract": scenario_contract,
            "provenance": initial_provenance,
            "topology": {
                "players": HAZARD_PLAYERS,
                "visible_clients": arguments.clients,
                "host_processes": 1,
                "host_has_player": False,
                "scenario": str(scenario),
            },
            "settings": {
                "measurement_seconds": arguments.measurement_seconds,
                "control_mode": 2,
                "window": [arguments.window_width, arguments.window_height],
                "input_probe_interval_ms": arguments.input_probe_interval_ms,
                "baseline_timestamp_queries_requested": True,
                "candidate_timestamp_queries_requested": True,
                "baseline_minimum_retained_gpu_profile_schema_version": 2,
                "candidate_minimum_retained_gpu_profile_schema_version": 2,
                "arm_port_stride": ARM_PORT_STRIDE,
            },
            "commands": commands,
            "limitations": [
                "Both visible fleets run sequentially on one machine and share "
                "its CPU and GPU; compare only this matched pair.",
                "Binary hashes and source-tree provenance are retained, but an "
                "external build attestation is required to prove that each "
                "binary was built from its named source tree.",
            ],
        },
    )

    arms: dict[str, dict[str, Any]] = {}
    contracts: dict[str, dict[str, Any]] = {}
    gpu_evidences: dict[str, dict[str, Any]] = {}
    for (
        name,
        _binary,
        timestamp_requested,
        profiles_required,
        minimum_schema_version,
        _base_port,
    ) in arm_specs:
        command = commands[name]
        arm_artifact_dir = artifact_dir / name
        failures: list[str] = []
        return_code: int | None = None
        contract: dict[str, Any] | None = None
        crew_evidence: dict[str, Any] | None = None
        gpu_evidence: dict[str, Any] | None = None
        print(
            f"[hazard-24-gpu-ab] start arm={name} "
            f"timestamp_queries={timestamp_requested}"
        )
        try:
            return_code = run_owned_child(
                command,
                arm_environment(
                    os.environ,
                    timestamp_queries_requested=timestamp_requested,
                ),
            )
        except (OSError, subprocess.SubprocessError) as error:
            failures.append(f"child invocation failed: {error}")
        try:
            contract = load_child_arm_contract(
                arm_artifact_dir, expected_clients=arguments.clients
            )
            contracts[name] = contract
        except HazardBenchmarkFailure as error:
            failures.append(str(error))
        try:
            crew_evidence = validate_hzck_runtime_evidence(
                arm_artifact_dir, expected_clients=arguments.clients
            )
        except HazardBenchmarkFailure as error:
            failures.append(str(error))
        try:
            gpu_evidence = collect_retained_gpu_evidence(
                arm_artifact_dir,
                expected_clients=arguments.clients,
                profiles_required=profiles_required,
                expected_timestamp_request=timestamp_requested,
                minimum_schema_version=minimum_schema_version,
            )
            gpu_evidences[name] = gpu_evidence
        except HazardBenchmarkFailure as error:
            failures.append(str(error))
        arms[name] = _arm_public_summary(
            artifact_dir=arm_artifact_dir,
            command=command,
            return_code=return_code,
            contract=contract,
            crew_evidence=crew_evidence,
            gpu_evidence=gpu_evidence,
            failures=failures,
        )
        print(
            f"[hazard-24-gpu-ab] complete arm={name} "
            f"return_code={return_code} result={arms[name]['result']}"
        )

    pair_failures: list[str] = []
    paired_input_fingerprint: dict[str, Any] | None = None
    if set(contracts) == {"baseline", "candidate"}:
        try:
            paired_input_fingerprint = validate_paired_input_fingerprints(
                contracts["baseline"]["input_fingerprint"],
                contracts["candidate"]["input_fingerprint"],
            )
        except HazardBenchmarkFailure as error:
            pair_failures.append(str(error))
    else:
        pair_failures.append(
            "both child arm contracts are required for paired fingerprint validation"
        )
    paired_gpu_fingerprint: dict[str, Any] | None = None
    if set(gpu_evidences) == {"baseline", "candidate"}:
        try:
            paired_gpu_fingerprint = validate_paired_gpu_fingerprints(
                gpu_evidences["baseline"], gpu_evidences["candidate"]
            )
        except HazardBenchmarkFailure as error:
            pair_failures.append(str(error))
    else:
        pair_failures.append(
            "both arm GPU evidence records are required for paired fingerprint validation"
        )
    try:
        final_provenance = collect_run_provenance(arguments, workspace=workspace)
    except (HazardBenchmarkFailure, OSError) as error:
        final_provenance = None
        pair_failures.append(f"final provenance capture failed: {error}")
    else:
        if final_provenance != initial_provenance:
            pair_failures.append("source, binary, or harness provenance changed during A/B run")

    summary = {
        "schema_version": 1,
        "result": (
            "pass"
            if all(arm["result"] == "pass" for arm in arms.values())
            and not pair_failures
            else "fail"
        ),
        "scenario_contract": scenario_contract,
        "initial_provenance": initial_provenance,
        "final_provenance": final_provenance,
        "paired_input_fingerprint": paired_input_fingerprint,
        "paired_gpu_fingerprint": paired_gpu_fingerprint,
        "arms": arms,
        "pair_failures": pair_failures,
    }
    write_json(artifact_dir / "summary.json", summary)
    return summary


def build_argument_parser() -> argparse.ArgumentParser:
    workspace = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(
        description=(
            "Run baseline and candidate visible 24-player Hazard fleets and "
            "retain validated clonk-org/clonk-rs#267 GPU profiles. "
            "Binaries must already exist."
        )
    )
    parser.add_argument("--baseline-binary", type=Path, required=True)
    parser.add_argument("--candidate-binary", type=Path, required=True)
    parser.add_argument("--baseline-source-root", type=Path, required=True)
    parser.add_argument(
        "--candidate-source-root", type=Path, default=workspace
    )
    parser.add_argument("--scenario", type=Path, default=DEFAULT_HAZARD_SCENARIO)
    parser.add_argument("--scenario-title", default=DEFAULT_HAZARD_TITLE)
    parser.add_argument(
        "--artifact-dir", type=Path, default=default_artifact_dir(workspace)
    )
    parser.add_argument("--clients", type=int, default=12)
    parser.add_argument("--measurement-seconds", type=int, default=60)
    parser.add_argument("--base-port", type=int, default=31_111)
    parser.add_argument("--minimum-simulation-fps", type=float, default=38.0)
    parser.add_argument("--minimum-presentation-fps", type=float, default=35.0)
    parser.add_argument("--maximum-graphics-p99-ms", type=float, default=25.0)
    parser.add_argument("--maximum-network-lag-ms", type=float, default=100.0)
    parser.add_argument("--input-probe-interval-ms", type=int, default=500)
    parser.add_argument("--maximum-input-latency-ms", type=float, default=100.0)
    parser.add_argument(
        "--minimum-input-success-percent", type=float, default=95.0
    )
    parser.add_argument("--window-width", type=int, default=800)
    parser.add_argument("--window-height", type=int, default=600)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = build_argument_parser().parse_args(argv)
    workspace = Path(__file__).resolve().parents[1]
    artifact_dir = Path(arguments.artifact_dir).expanduser().resolve()
    try:
        summary = run_paired_benchmark(
            arguments,
            workspace=workspace,
            artifact_dir=artifact_dir,
        )
    except (HazardBenchmarkFailure, OSError, subprocess.SubprocessError) as error:
        print(f"hazard 24-player GPU A/B benchmark: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print(
            "hazard 24-player GPU A/B benchmark: interrupted",
            file=sys.stderr,
        )
        return 130
    print(
        f"[hazard-24-gpu-ab] result={summary['result']} "
        f"artifacts={artifact_dir}"
    )
    return 0 if summary["result"] == "pass" else 1


def handle_termination_signal(_signum: int | None, _frame: Any) -> None:
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, handle_termination_signal)
    raise SystemExit(main())
