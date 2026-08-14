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
import math
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
RETAINED_GPU_PROFILE_PREFIX = "LC_APP_RETAINED_GPU_PROFILE"
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
        submission_kind_fields = (
            "retained_gpu_present_submissions",
            "cpu_present_submissions",
        )
        present_submission_kind_fields = [
            key for key in submission_kind_fields if key in fields
        ]
        if present_submission_kind_fields and len(
            present_submission_kind_fields
        ) != len(submission_kind_fields):
            raise ValueError(
                "retained and CPU submission counts must appear together"
            )
        parsed.update(
            {
                key: int(fields[key])
                for key in present_submission_kind_fields
            }
        )
        for key in submission_kind_fields:
            parsed.setdefault(key, None)
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
    successful_submissions = parsed["successful_present_submissions"]
    if raw_sample_count != successful_submissions:
        raise BenchmarkFailure(
            f"{raw_sample_count} graphics samples were reported for "
            f"{successful_submissions} successful submissions"
        )
    if any(sample < 0 for sample in parsed["graphics_pass_samples_ns"]):
        raise BenchmarkFailure("graphics pass samples cannot be negative")
    if any(parsed[key] < 0 for key in integer_fields):
        raise BenchmarkFailure("presentation counters cannot be negative")
    if any(
        not math.isfinite(parsed[key]) or parsed[key] < 0
        for key in float_fields
    ):
        raise BenchmarkFailure(
            "presentation summary values must be finite and nonnegative"
        )
    if parsed["elapsed_seconds"] == 0:
        raise BenchmarkFailure("presentation elapsed_seconds must be positive")
    retained_submissions = parsed["retained_gpu_present_submissions"]
    cpu_submissions = parsed["cpu_present_submissions"]
    if retained_submissions is not None:
        if retained_submissions < 0 or cpu_submissions < 0:
            raise BenchmarkFailure("presentation counters cannot be negative")
        submission_total = retained_submissions + cpu_submissions
        if submission_total != parsed["successful_present_submissions"]:
            raise BenchmarkFailure(
                f"submission kind counts total {submission_total} but "
                f"{parsed['successful_present_submissions']} successful "
                "submissions were reported"
            )
    return parsed


def _reject_duplicate_json_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise BenchmarkFailure(f"duplicate retained GPU profile key: {key}")
        result[key] = value
    return result


def _reject_json_constant(value):
    raise BenchmarkFailure(
        f"invalid retained GPU profile JSON constant: {value}"
    )


def _exact_nonnegative_integer(value, label):
    if type(value) is not int or value < 0 or value > (1 << 64) - 1:
        raise BenchmarkFailure(f"{label} must be a nonnegative integer")
    return value


def _exact_unsigned_integer(value, bits, label):
    if type(value) is not int or value < 0 or value > (1 << bits) - 1:
        raise BenchmarkFailure(f"{label} must be an unsigned {bits}-bit integer")
    return value


def _exact_signed_integer(value, bits, label):
    minimum = -(1 << (bits - 1))
    maximum = (1 << (bits - 1)) - 1
    if type(value) is not int or value < minimum or value > maximum:
        raise BenchmarkFailure(f"{label} must be a signed {bits}-bit integer")
    return value


def _exact_string(value, label, *, nonempty=False):
    if type(value) is not str or (nonempty and not value):
        qualifier = "nonempty " if nonempty else ""
        raise BenchmarkFailure(f"{label} must be a {qualifier}string")
    return value


def _extent(value, label):
    if (
        not isinstance(value, list)
        or len(value) != 2
        or any(
            type(dimension) is not int
            or dimension <= 0
            or dimension > (1 << 32) - 1
            for dimension in value
        )
    ):
        raise BenchmarkFailure(
            f"{label} must be two positive unsigned 32-bit integers"
        )
    return value


def _require_mapping(value, label):
    if not isinstance(value, dict):
        raise BenchmarkFailure(f"{label} must be a JSON object")
    return value


def _feature_bits(value, label):
    if (
        not isinstance(value, list)
        or len(value) != 2
        or any(type(word) is not int or word < 0 or word > (1 << 64) - 1 for word in value)
    ):
        raise BenchmarkFailure(f"{label} must be two unsigned 64-bit integers")
    return value


def _finite_positive_number(value, label):
    if type(value) not in (int, float) or not math.isfinite(value) or value <= 0:
        raise BenchmarkFailure(f"{label} must be positive and finite")
    return float(value)


def _exact_bool(value, label):
    if type(value) is not bool:
        raise BenchmarkFailure(f"{label} must be a boolean")
    return value


def validate_retained_gpu_profile(profile):
    schema_version = profile.get("schema_version")
    if type(schema_version) is not int or schema_version not in (1, 2):
        raise BenchmarkFailure("retained GPU profile schema_version must be 1 or 2")
    fingerprint = _require_mapping(
        profile.get("fingerprint"), "retained GPU fingerprint"
    )
    adapter = _require_mapping(
        fingerprint.get("adapter"), "retained GPU adapter fingerprint"
    )
    for key in ("name", "driver", "driver_info"):
        _exact_string(adapter.get(key), f"retained GPU adapter.{key}")
    _exact_string(
        adapter.get("device_type"),
        "retained GPU adapter.device_type",
        nonempty=True,
    )
    _exact_string(
        adapter.get("backend"),
        "retained GPU adapter.backend",
        nonempty=True,
    )
    adapter_enum_values = {
        "device_type": {
            "other",
            "integrated_gpu",
            "discrete_gpu",
            "virtual_gpu",
            "cpu",
        },
        "backend": {"noop", "vulkan", "metal", "dx12", "gl", "webgpu"},
    }
    for key, allowed_values in adapter_enum_values.items():
        if adapter[key] not in allowed_values:
            raise BenchmarkFailure(
                f"retained GPU adapter.{key} has an unknown value"
            )
    for key in ("vendor_id", "device_id", "subgroup_min_size", "subgroup_max_size"):
        _exact_unsigned_integer(
            adapter.get(key), 32, f"retained GPU adapter.{key}"
        )
    pci_bus_id = adapter.get("pci_bus_id")
    if pci_bus_id is not None:
        _exact_string(
            pci_bus_id,
            "retained GPU adapter.pci_bus_id",
            nonempty=True,
        )
    if adapter["subgroup_min_size"] > adapter["subgroup_max_size"]:
        raise BenchmarkFailure(
            "retained GPU adapter subgroup bounds are reversed"
        )
    _exact_bool(
        adapter.get("transient_saves_memory"),
        "retained GPU adapter.transient_saves_memory",
    )
    adapter_features = _feature_bits(
        fingerprint.get("adapter_feature_bits"),
        "retained GPU adapter_feature_bits",
    )
    device = _require_mapping(
        fingerprint.get("device"), "retained GPU device fingerprint"
    )
    device_features = _feature_bits(
        device.get("feature_bits"), "retained GPU device.feature_bits"
    )
    device_period = _finite_positive_number(
        device.get("timestamp_period_ns"),
        "retained GPU device.timestamp_period_ns",
    )
    _exact_string(
        device.get("limits_debug"),
        "retained GPU device.limits_debug",
        nonempty=True,
    )
    if (
        _exact_unsigned_integer(
            device.get("max_texture_dimension_2d"),
            32,
            "retained GPU device.max_texture_dimension_2d",
        )
        == 0
    ):
        raise BenchmarkFailure(
            "retained GPU device.max_texture_dimension_2d must be positive"
        )
    surface = _require_mapping(
        fingerprint.get("surface"), "retained GPU surface fingerprint"
    )
    for key in ("format", "present_mode", "alpha_mode"):
        _exact_string(
            surface.get(key),
            f"retained GPU surface.{key}",
            nonempty=True,
        )
    surface_extent = _extent(
        surface.get("surface_extent"), "retained GPU surface.surface_extent"
    )
    _extent(
        surface.get("buffer_extent"), "retained GPU surface.buffer_extent"
    )
    renderer_config = _require_mapping(
        fingerprint.get("renderer"), "retained GPU renderer fingerprint"
    )
    for key in ("mipmaps", "smooth_landscape", "shader_landscape"):
        _exact_bool(
            renderer_config.get(key), f"retained GPU renderer.{key}"
        )
    landscape_detail = _exact_unsigned_integer(
        renderer_config.get("landscape_detail"),
        32,
        "retained GPU renderer.landscape_detail",
    )
    if not 1 <= landscape_detail <= 4:
        raise BenchmarkFailure(
            "retained GPU renderer.landscape_detail must be in 1..=4"
        )
    renderer_surface_format = _exact_string(
        renderer_config.get("surface_format"),
        "retained GPU renderer.surface_format",
        nonempty=True,
    )
    if renderer_surface_format != surface["format"]:
        raise BenchmarkFailure(
            "retained GPU renderer and surface formats disagree"
        )
    frontend = _require_mapping(
        fingerprint.get("frontend"), "retained GPU frontend fingerprint"
    )
    for key in (
        "no_alpha_add",
        "no_box_fades",
        "shader",
        "use_shader_gamma",
        "disable_gamma",
    ):
        _exact_bool(frontend.get(key), f"retained GPU frontend.{key}")
    for key in ("tex_indent", "blit_offset"):
        _exact_signed_integer(
            frontend.get(key), 32, f"retained GPU frontend.{key}"
        )
    _exact_unsigned_integer(
        frontend.get("allowed_blit_modes"),
        32,
        "retained GPU frontend.allowed_blit_modes",
    )
    presentation = _require_mapping(
        fingerprint.get("presentation"),
        "retained GPU presentation fingerprint",
    )
    physical_extent = _extent(
        presentation.get("physical_extent"),
        "retained GPU presentation.physical_extent",
    )
    if physical_extent != surface_extent:
        raise BenchmarkFailure(
            "retained GPU presentation and surface extents disagree"
        )
    _finite_positive_number(
        presentation.get("scale"), "retained GPU presentation.scale"
    )
    _exact_unsigned_integer(
        presentation.get("crop_top"),
        32,
        "retained GPU presentation.crop_top",
    )
    timestamp_queries = _require_mapping(
        profile.get("timestamp_queries"), "retained GPU timestamp_queries"
    )
    requested = _exact_bool(
        timestamp_queries.get("requested"),
        "retained GPU timestamp_queries.requested",
    )
    supported = _exact_bool(
        timestamp_queries.get("supported"),
        "retained GPU timestamp_queries.supported",
    )
    enabled = _exact_bool(
        timestamp_queries.get("enabled"),
        "retained GPU timestamp_queries.enabled",
    )
    if enabled != (requested and supported):
        raise BenchmarkFailure(
            "retained GPU timestamp enabled status disagrees with request/support"
        )
    timestamp_bit = 1 << 7
    if supported != bool(adapter_features[0] & timestamp_bit):
        raise BenchmarkFailure(
            "retained GPU timestamp support disagrees with adapter features"
        )
    expected_device_features = [timestamp_bit, 0] if enabled else [0, 0]
    if device_features != expected_device_features:
        raise BenchmarkFailure(
            "retained GPU device features do not match timestamp-query status"
        )
    for key in ("dropped_frames", "readback_errors", "device_discontinuities"):
        if _exact_nonnegative_integer(
            timestamp_queries.get(key), f"retained GPU timestamp_queries.{key}"
        ) != 0:
            raise BenchmarkFailure(f"retained GPU timestamp telemetry {key} is nonzero")

    frames = profile.get("frames")
    if not isinstance(frames, list):
        raise BenchmarkFailure("retained GPU profile frames must be a JSON array")
    stage_keys = (
        "frame_preparation_ns",
        "validation_ns",
        "texture_synchronization_ns",
        "stream_packing_upload_ns",
        "command_encoding_ns",
        "drawable_acquisition_ns",
        "queue_submission_ns",
        "presentation_ns",
    )
    for sample_index, frame_value in enumerate(frames):
        frame = _require_mapping(frame_value, f"retained GPU frame {sample_index}")
        if _exact_nonnegative_integer(
            frame.get("sample_index"),
            f"retained GPU frame {sample_index} sample_index",
        ) != sample_index:
            raise BenchmarkFailure(
                "retained GPU profile sample indices must be consecutive from zero"
            )
        end_to_end = _exact_nonnegative_integer(
            frame.get("end_to_end_ns"),
            f"retained GPU frame {sample_index} end_to_end_ns",
        )
        cpu = _require_mapping(
            frame.get("cpu"), f"retained GPU frame {sample_index} cpu"
        )
        named_sum = sum(
            _exact_nonnegative_integer(
                cpu.get(key), f"retained GPU frame {sample_index} cpu.{key}"
            )
            for key in stage_keys
        )
        named_total = _exact_nonnegative_integer(
            cpu.get("named_total_ns"),
            f"retained GPU frame {sample_index} cpu.named_total_ns",
        )
        unclassified = _exact_nonnegative_integer(
            cpu.get("unclassified_ns"),
            f"retained GPU frame {sample_index} cpu.unclassified_ns",
        )
        overrun = _exact_nonnegative_integer(
            cpu.get("overrun_ns"),
            f"retained GPU frame {sample_index} cpu.overrun_ns",
        )
        if named_sum != named_total or named_total + unclassified != end_to_end + overrun:
            raise BenchmarkFailure(
                f"retained GPU CPU reconciliation failed for sample {sample_index}"
            )
        renderer = _require_mapping(
            frame.get("renderer"),
            f"retained GPU frame {sample_index} renderer",
        )
        renderer_counter_keys = (
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
        )
        if schema_version >= 2:
            renderer_counter_keys += (
                "landscape_instances",
                "landscape_instance_upload_bytes",
            )
        renderer_counts = {
            key: _exact_nonnegative_integer(
                renderer.get(key),
                f"retained GPU frame {sample_index} renderer.{key}",
            )
            for key in renderer_counter_keys
        }
        _exact_bool(
            renderer.get("composition_recreated"),
            f"retained GPU frame {sample_index} renderer.composition_recreated",
        )
        classified_scene = sum(
            renderer_counts[key]
            for key in (
                "quad_draw_calls",
                "sprite_draw_calls",
                "object_sprite_draw_calls",
                "landscape_draw_calls",
                "solid_draw_calls",
                "solid_rect_draw_calls",
            )
        )
        classified_total = sum(
            renderer_counts[key]
            for key in (
                "draw_calls",
                "shader_landscape_draw_calls",
                "monitor_gamma_draw_calls",
                "presentation_draw_calls",
            )
        )
        if (
            classified_scene != renderer_counts["draw_calls"]
            or renderer_counts["compatible_resource_runs"]
            != renderer_counts["draw_calls"]
            or classified_total != renderer_counts["total_draw_calls"]
        ):
            raise BenchmarkFailure(
                f"retained GPU draw counts do not reconcile for sample {sample_index}"
            )
        if renderer_counts["presentation_draw_calls"] != 1:
            raise BenchmarkFailure(
                f"retained GPU frame {sample_index} must report exactly one "
                "presentation draw"
            )
        if (
            renderer_counts["shader_landscape_draw_calls"] > 1
            or renderer_counts["monitor_gamma_draw_calls"] > 1
        ):
            raise BenchmarkFailure(
                f"retained GPU fixed-pass draw count exceeds one for sample {sample_index}"
            )
        if (
            not renderer_config["shader_landscape"]
            and renderer_counts["shader_landscape_draw_calls"] != 0
        ):
            raise BenchmarkFailure(
                "retained GPU shader-landscape draw contradicts renderer config "
                f"for sample {sample_index}"
            )
        stream_layouts = (
            ("generic_vertices", "generic_vertex_upload_bytes", 72),
            ("quad_instances", "quad_instance_upload_bytes", 232),
            ("sprite_instances", "sprite_instance_upload_bytes", 40),
            ("object_sprite_instances", "object_sprite_upload_bytes", 88),
            ("solid_rect_instances", "solid_rect_upload_bytes", 36),
        )
        if schema_version >= 2:
            stream_layouts += (
                (
                    "landscape_instances",
                    "landscape_instance_upload_bytes",
                    72,
                ),
            )
        if any(
            renderer_counts[byte_key]
            != renderer_counts[count_key] * byte_stride
            for count_key, byte_key, byte_stride in stream_layouts
        ):
            raise BenchmarkFailure(
                f"retained GPU stream bytes do not reconcile for sample {sample_index}"
            )
        for upload_kind in ("full", "dirty"):
            has_calls = renderer_counts[f"{upload_kind}_upload_calls"] != 0
            has_bytes = renderer_counts[f"{upload_kind}_upload_bytes"] != 0
            if has_calls != has_bytes:
                raise BenchmarkFailure(
                    f"retained GPU {upload_kind} upload calls/bytes disagree "
                    f"for sample {sample_index}"
                )
        if (
            renderer_counts["created_source_textures"]
            > renderer_counts["full_upload_calls"]
        ):
            raise BenchmarkFailure(
                "retained GPU created textures exceed full uploads "
                f"for sample {sample_index}"
            )
        capture = _require_mapping(
            frame.get("frontend_capture"),
            f"retained GPU frame {sample_index} frontend_capture",
        )
        capture_keys = (
            "generic_sprite_fallbacks",
            "spatial_fog_fallbacks",
            "precomputed_fog_modulation_fallbacks",
            "texture_indent_fallbacks",
            "owner_mask_fallbacks",
            "physical_texture_tile_fallbacks",
            "fog_expanded_chunks",
        )
        capture_counts = {
            key: _exact_nonnegative_integer(
                capture.get(key),
                f"retained GPU frame {sample_index} frontend_capture.{key}",
            )
            for key in capture_keys
        }
        generic_fallbacks = capture_counts["generic_sprite_fallbacks"]
        if any(
            capture_counts[key] > generic_fallbacks
            for key in (
                "spatial_fog_fallbacks",
                "precomputed_fog_modulation_fallbacks",
                "texture_indent_fallbacks",
                "owner_mask_fallbacks",
                "physical_texture_tile_fallbacks",
            )
        ):
            raise BenchmarkFailure(
                "retained GPU fallback reasons exceed generic fallbacks "
                f"for sample {sample_index}"
            )
        if (
            capture_counts["spatial_fog_fallbacks"] == 0
            and capture_counts["fog_expanded_chunks"] != 0
        ):
            raise BenchmarkFailure(
                "retained GPU fog chunks lack a spatial fallback "
                f"for sample {sample_index}"
            )

    gpu_frames = profile.get("gpu_timestamp_frames")
    if not isinstance(gpu_frames, list):
        raise BenchmarkFailure(
            "retained GPU gpu_timestamp_frames must be a JSON array"
        )
    frame_ids = [frame.get("timestamp_frame_id") for frame in frames]
    if not enabled:
        if any(frame_id is not None for frame_id in frame_ids) or gpu_frames:
            raise BenchmarkFailure(
                "disabled retained GPU timestamps must not report frame IDs or samples"
            )
        return
    if any(type(frame_id) is not int or frame_id <= 0 for frame_id in frame_ids):
        raise BenchmarkFailure(
            "enabled retained GPU timestamps require positive frame IDs"
        )
    if len(set(frame_ids)) != len(frame_ids):
        raise BenchmarkFailure("retained GPU timestamp frame IDs must be unique")
    if frame_ids != sorted(frame_ids):
        raise BenchmarkFailure(
            "retained GPU timestamp frame IDs must increase with CPU samples"
        )

    gpu_by_id = {}
    gpu_frame_ids = []
    renderer_generations = set()
    allowed_passes = {
        "shader_landscape",
        "scene",
        "monitor_gamma",
        "presentation",
    }
    for gpu_value in gpu_frames:
        gpu = _require_mapping(gpu_value, "retained GPU timestamp frame")
        frame_id = _exact_nonnegative_integer(
            gpu.get("frame_id"), "retained GPU timestamp frame_id"
        )
        if frame_id == 0 or frame_id in gpu_by_id:
            raise BenchmarkFailure("retained GPU timestamp frame IDs must be unique")
        gpu_frame_ids.append(frame_id)
        renderer_generation = _exact_nonnegative_integer(
            gpu.get("renderer_generation"),
            f"retained GPU timestamp frame {frame_id} renderer_generation",
        )
        if renderer_generation == 0:
            raise BenchmarkFailure(
                "retained GPU timestamp renderer generation must be positive"
            )
        renderer_generations.add(renderer_generation)
        period = _finite_positive_number(
            gpu.get("timestamp_period_ns"),
            f"retained GPU timestamp frame {frame_id} timestamp_period_ns",
        )
        if period != device_period:
            raise BenchmarkFailure(
                "retained GPU timestamp period disagrees with device fingerprint"
            )
        passes = gpu.get("passes")
        if not isinstance(passes, list):
            raise BenchmarkFailure(
                f"retained GPU timestamp frame {frame_id} passes must be an array"
            )
        observed_passes = []
        observed_pass_names = set()
        previous_end_tick = None
        for pass_value in passes:
            sample = _require_mapping(
                pass_value, f"retained GPU timestamp frame {frame_id} pass"
            )
            pass_name = sample.get("pass")
            if pass_name not in allowed_passes or pass_name in observed_pass_names:
                raise BenchmarkFailure(
                    f"retained GPU timestamp frame {frame_id} has invalid or duplicate pass"
                )
            observed_passes.append(pass_name)
            observed_pass_names.add(pass_name)
            begin = _exact_nonnegative_integer(
                sample.get("begin_tick"),
                f"retained GPU timestamp frame {frame_id} {pass_name} begin_tick",
            )
            end = _exact_nonnegative_integer(
                sample.get("end_tick"),
                f"retained GPU timestamp frame {frame_id} {pass_name} end_tick",
            )
            validity = sample.get("validity")
            if validity not in {
                "valid",
                "invalid_period",
                "counter_rollover",
                "invalid_duration",
            }:
                raise BenchmarkFailure(
                    "retained GPU timestamp sample has an invalid validity value"
                )
            if validity != "valid":
                raise BenchmarkFailure(
                    f"retained GPU timestamp sample is not valid: {validity}"
                )
            if end < begin:
                raise BenchmarkFailure("retained GPU timestamp ends before it begins")
            if previous_end_tick is not None and begin < previous_end_tick:
                raise BenchmarkFailure(
                    "retained GPU timestamp intervals are not ordered in "
                    f"frame {frame_id}"
                )
            previous_end_tick = end
            duration = sample.get("duration_ns")
            if type(duration) not in (int, float) or not math.isfinite(duration) or duration < 0:
                raise BenchmarkFailure(
                    "retained GPU timestamp duration must be nonnegative and finite"
                )
            expected_duration = (end - begin) * period
            if not math.isclose(
                float(duration), expected_duration, rel_tol=1e-9, abs_tol=1e-6
            ):
                raise BenchmarkFailure(
                    "GPU timestamp duration does not match raw ticks"
                )
        gpu_by_id[frame_id] = observed_passes
    if frame_ids != gpu_frame_ids:
        raise BenchmarkFailure(
            "retained GPU timestamp frames must match CPU frame order"
        )
    if len(renderer_generations) != 1:
        raise BenchmarkFailure(
            "retained GPU timestamp renderer generation changed without telemetry"
        )
    for frame in frames:
        frame_id = frame["timestamp_frame_id"]
        renderer = _require_mapping(
            frame.get("renderer"), f"retained GPU frame {frame_id} renderer"
        )
        expected_passes = []
        if _exact_nonnegative_integer(
            renderer.get("shader_landscape_draw_calls"),
            f"retained GPU frame {frame_id} shader_landscape_draw_calls",
        ):
            expected_passes.append("shader_landscape")
        expected_passes.append("scene")
        if _exact_nonnegative_integer(
            renderer.get("monitor_gamma_draw_calls"),
            f"retained GPU frame {frame_id} monitor_gamma_draw_calls",
        ):
            expected_passes.append("monitor_gamma")
        expected_passes.append("presentation")
        if gpu_by_id[frame_id] != expected_passes:
            raise BenchmarkFailure(
                f"retained GPU timestamp passes do not match frame {frame_id} draws"
            )


def parse_retained_gpu_profile(
    lines, *, required, minimum_schema_version=None
):
    marker = f"{RETAINED_GPU_PROFILE_PREFIX} "
    matches = [line.strip()[len(marker) :] for line in lines if line.startswith(marker)]
    if not matches:
        if required:
            raise BenchmarkFailure("required retained GPU profile is missing")
        return None
    if len(matches) != 1:
        raise BenchmarkFailure(
            "expected exactly one retained GPU profile; observed "
            f"{len(matches)}"
        )
    try:
        profile = json.loads(
            matches[0],
            object_pairs_hook=_reject_duplicate_json_keys,
            parse_constant=_reject_json_constant,
        )
    except (json.JSONDecodeError, TypeError) as error:
        raise BenchmarkFailure(f"invalid retained GPU profile JSON: {error}") from error
    if not isinstance(profile, dict):
        raise BenchmarkFailure("retained GPU profile must be a JSON object")
    validate_retained_gpu_profile(profile)
    if (
        minimum_schema_version is not None
        and profile["schema_version"] < minimum_schema_version
    ):
        raise BenchmarkFailure(
            "retained GPU profile schema_version "
            f"{profile['schema_version']} is older than required "
            f"{minimum_schema_version}"
        )
    return profile


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
        "LC_GPU_TIMESTAMP_QUERIES",
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


def timestamp_query_process_environment(base):
    environment = base.copy()
    environment["LC_GPU_TIMESTAMP_QUERIES"] = "1"
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


def parse_presentation_evidence(
    lines,
    process_status,
    *,
    require_retained_gpu_profile=False,
    minimum_retained_gpu_profile_schema_version=None,
    expected_timestamp_query_request=None,
):
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
    retained_gpu_profile = parse_retained_gpu_profile(
        lines,
        required=require_retained_gpu_profile,
        minimum_schema_version=minimum_retained_gpu_profile_schema_version,
    )
    if retained_gpu_profile is not None:
        requested = retained_gpu_profile["timestamp_queries"]["requested"]
        if (
            expected_timestamp_query_request is not None
            and requested != expected_timestamp_query_request
        ):
            raise BenchmarkFailure(
                "retained GPU timestamp request status disagrees with "
                "benchmark environment"
            )
        profile_frames = retained_gpu_profile["frames"]
        retained_submissions = report["retained_gpu_present_submissions"]
        if retained_submissions is None:
            raise BenchmarkFailure(
                "retained GPU profile requires submission kind counts"
            )
        if len(profile_frames) != retained_submissions:
            raise BenchmarkFailure(
                f"profile frame count is {len(profile_frames)} but "
                f"{retained_submissions} retained submissions were reported"
            )
        profile_durations = [frame["end_to_end_ns"] for frame in profile_frames]
        if profile_durations != report["graphics_pass_samples_ns"]:
            raise BenchmarkFailure(
                "retained GPU profile durations do not match the legacy raw "
                "graphics samples"
            )
    return {
        "schema_version": 2,
        "process_status": process_status,
        "budget_result": budget_result,
        "presentation": report,
        "context": context,
        "network": network,
        "retained_gpu_profile": retained_gpu_profile,
        "retained_gpu_profile_sha256": (
            None
            if retained_gpu_profile is None
            else json_sha256(retained_gpu_profile)
        ),
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
    require_retained_gpu_profile,
    minimum_retained_gpu_profile_schema_version,
    expected_timestamp_query_request,
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
        "timestamp_query_environment": environment.get(
            "LC_GPU_TIMESTAMP_QUERIES"
        ),
        **parse_presentation_evidence(
            lines,
            process_status,
            require_retained_gpu_profile=require_retained_gpu_profile,
            minimum_retained_gpu_profile_schema_version=(
                minimum_retained_gpu_profile_schema_version
            ),
            expected_timestamp_query_request=expected_timestamp_query_request,
        ),
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
        "schema_version": 2,
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
        baseline_environment = environment.copy()
        candidate_environment = timestamp_query_process_environment(
            environment
        )
        manifest["timestamp_query_environment"] = {
            "baseline": baseline_environment.get(
                "LC_GPU_TIMESTAMP_QUERIES"
            ),
            "candidate": candidate_environment[
                "LC_GPU_TIMESTAMP_QUERIES"
            ],
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
                environment=baseline_environment,
                artifact_dir=artifact_dir,
                expected_inputs=inputs,
                require_retained_gpu_profile=False,
                minimum_retained_gpu_profile_schema_version=None,
                expected_timestamp_query_request=False,
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
                environment=candidate_environment,
                artifact_dir=artifact_dir,
                expected_inputs=inputs,
                require_retained_gpu_profile=True,
                minimum_retained_gpu_profile_schema_version=2,
                expected_timestamp_query_request=True,
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
        environment = timestamp_query_process_environment(environment)
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
        evidence = parse_presentation_evidence(
            presentation_lines,
            presentation_status,
            require_retained_gpu_profile=True,
            minimum_retained_gpu_profile_schema_version=2,
            expected_timestamp_query_request=True,
        )
        report = evidence["presentation"]
        context = evidence["context"]
        network_evidence = evidence["network"]
        validate_native_cadence(report)
        validate_native_presentation_cadence(report)
        if evidence["budget_result"] != "pass":
            raise BenchmarkFailure(
                "candidate did not pass the native presentation budget"
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
