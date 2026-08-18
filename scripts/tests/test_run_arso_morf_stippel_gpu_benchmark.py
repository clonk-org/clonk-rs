import copy
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT = (
    Path(__file__).resolve().parents[1]
    / "run_arso_morf_stippel_gpu_benchmark.py"
)
SPEC = importlib.util.spec_from_file_location("arso_morf_stippel_gpu", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def retained_gpu_profile():
    return {
        "schema_version": 1,
        "fingerprint": {
            "adapter": {
                "name": "Test Adapter",
                "vendor_id": 1,
                "device_id": 2,
                "device_type": "integrated_gpu",
                "pci_bus_id": None,
                "driver": "test",
                "driver_info": "test 1.0",
                "backend": "metal",
                "subgroup_min_size": 4,
                "subgroup_max_size": 32,
                "transient_saves_memory": False,
            },
            "adapter_feature_bits": [0, 0],
            "device": {
                "feature_bits": [0, 0],
                "limits_debug": "Limits { max_texture_dimension_2d: 8192 }",
                "max_texture_dimension_2d": 8_192,
                "timestamp_period_ns": 1.0,
            },
            "surface": {
                "format": "Bgra8UnormSrgb",
                "present_mode": "AutoVsync",
                "alpha_mode": "Auto",
                "surface_extent": [800, 600],
                "buffer_extent": [800, 600],
            },
            "renderer": {
                "mipmaps": False,
                "smooth_landscape": False,
                "shader_landscape": False,
                "landscape_detail": 1,
                "surface_format": "Bgra8UnormSrgb",
            },
            "frontend": {
                "no_alpha_add": False,
                "no_box_fades": False,
                "tex_indent": 0,
                "blit_offset": 0,
                "allowed_blit_modes": 15,
                "shader": False,
                "use_shader_gamma": True,
                "disable_gamma": False,
            },
            "presentation": {
                "physical_extent": [800, 600],
                "scale": 1,
                "crop_top": 0,
            },
        },
        "timestamp_queries": {
            "requested": True,
            "supported": False,
            "enabled": False,
            "dropped_frames": 0,
            "readback_errors": 0,
            "device_discontinuities": 0,
        },
        "frames": [
            {
                "sample_index": 0,
                "end_to_end_ns": 5_000_000,
                "timestamp_frame_id": None,
                "cpu": {
                    "frame_preparation_ns": 1_000_000,
                    "validation_ns": 500_000,
                    "texture_synchronization_ns": 500_000,
                    "stream_packing_upload_ns": 500_000,
                    "command_encoding_ns": 500_000,
                    "drawable_acquisition_ns": 500_000,
                    "queue_submission_ns": 500_000,
                    "presentation_ns": 500_000,
                    "named_total_ns": 4_500_000,
                    "unclassified_ns": 500_000,
                    "overrun_ns": 0,
                },
                "renderer": {
                    "resident_source_textures": 2,
                    "created_source_textures": 0,
                    "full_upload_calls": 0,
                    "full_upload_bytes": 0,
                    "dirty_upload_calls": 0,
                    "dirty_upload_bytes": 0,
                    "draw_calls": 2,
                    "quad_draw_calls": 1,
                    "sprite_draw_calls": 0,
                    "object_sprite_draw_calls": 1,
                    "landscape_draw_calls": 0,
                    "shader_landscape_draw_calls": 0,
                    "solid_draw_calls": 0,
                    "solid_rect_draw_calls": 0,
                    "monitor_gamma_draw_calls": 1,
                    "presentation_draw_calls": 1,
                    "total_draw_calls": 4,
                    "compatible_resource_runs": 2,
                    "generic_vertices": 4,
                    "generic_vertex_upload_bytes": 288,
                    "quad_instances": 0,
                    "sprite_instances": 0,
                    "object_sprite_instances": 1,
                    "solid_rect_instances": 0,
                    "quad_instance_upload_bytes": 0,
                    "sprite_instance_upload_bytes": 0,
                    "object_sprite_upload_bytes": 88,
                    "solid_rect_upload_bytes": 0,
                    "composition_recreated": False,
                },
                "frontend_capture": {
                    "generic_sprite_fallbacks": 0,
                    "spatial_fog_fallbacks": 0,
                    "precomputed_fog_modulation_fallbacks": 0,
                    "texture_indent_fallbacks": 0,
                    "owner_mask_fallbacks": 0,
                    "physical_texture_tile_fallbacks": 0,
                    "fog_expanded_chunks": 0,
                },
            }
        ],
        "gpu_timestamp_frames": [],
        "future_field": {"preserved": True},
    }


def retained_gpu_profile_v2():
    profile = retained_gpu_profile()
    profile["schema_version"] = 2
    renderer = profile["frames"][0]["renderer"]
    renderer["landscape_instances"] = 2
    renderer["landscape_instance_upload_bytes"] = 144
    return profile


def retained_gpu_profile_for_durations(durations):
    profile = retained_gpu_profile()
    template = profile["frames"][0]
    profile["frames"] = []
    for sample_index, duration in enumerate(durations):
        frame = copy.deepcopy(template)
        frame["sample_index"] = sample_index
        frame["end_to_end_ns"] = duration
        frame["cpu"]["unclassified_ns"] = (
            duration - frame["cpu"]["named_total_ns"]
        )
        profile["frames"].append(frame)
    return profile


def retained_gpu_profile_v2_for_durations(durations):
    profile = retained_gpu_profile_for_durations(durations)
    profile["schema_version"] = 2
    for frame in profile["frames"]:
        renderer = frame["renderer"]
        renderer["landscape_instances"] = 0
        renderer["landscape_instance_upload_bytes"] = 0
    return profile


def enable_retained_gpu_timestamps(profile):
    timestamp_bit = 1 << 7
    profile["fingerprint"]["adapter_feature_bits"] = [0, timestamp_bit]
    profile["fingerprint"]["device"]["feature_bits"] = [0, timestamp_bit]
    profile["timestamp_queries"].update(
        {"supported": True, "enabled": True}
    )
    profile["gpu_timestamp_frames"] = []
    for sample_index, frame in enumerate(profile["frames"]):
        frame_id = sample_index + 1
        tick_offset = sample_index * 1_000
        frame["timestamp_frame_id"] = frame_id
        profile["gpu_timestamp_frames"].append(
            {
                "frame_id": frame_id,
                "renderer_generation": 1,
                "timestamp_period_ns": 1.0,
                "passes": [
                    {
                        "pass": "scene",
                        "begin_tick": tick_offset + 100,
                        "end_tick": tick_offset + 110,
                        "duration_ns": 10.0,
                        "validity": "valid",
                    },
                    {
                        "pass": "monitor_gamma",
                        "begin_tick": tick_offset + 111,
                        "end_tick": tick_offset + 116,
                        "duration_ns": 5.0,
                        "validity": "valid",
                    },
                    {
                        "pass": "presentation",
                        "begin_tick": tick_offset + 120,
                        "end_tick": tick_offset + 130,
                        "duration_ns": 10.0,
                        "validity": "valid",
                    },
                ],
            }
        )
    return profile


def retained_gpu_profile_with_rollover():
    profile = enable_retained_gpu_timestamps(
        retained_gpu_profile_for_durations([5_000_000, 7_000_000])
    )
    presentation = profile["gpu_timestamp_frames"][1]["passes"][-1]
    scene, monitor = profile["gpu_timestamp_frames"][1]["passes"][:2]
    scene.update({"end_tick": 1_150, "duration_ns": 50.0})
    monitor.update(
        {"begin_tick": 1_140, "end_tick": 1_145, "duration_ns": 5.0}
    )
    presentation.update(
        {
            "begin_tick": 2_500,
            "end_tick": 2_000,
            "duration_ns": None,
            "validity": "counter_rollover",
        }
    )
    profile["timestamp_queries"]["readback_errors"] = 1
    return profile


class FixtureEvidenceTests(unittest.TestCase):
    def test_requires_exact_real_stippel_census(self):
        report = MODULE.parse_fixture_line(
            "LC_ARSO_MORF_STIPPEL_FIXTURE "
            "source_stippels=20 prepared_stippels=1000 "
            "source_lifecycle_stippels=20 prepared_lifecycle_stippels=1000 "
            "serialized_stippels=1000 source_objects=1063 "
            "serialized_objects=2043 seed=424242"
        )

        MODULE.validate_fixture_report(report)

        report["serialized_stippels"] = 999
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "serialized fixture contains 999 ST5B objects; expected exactly 1000",
        ):
            MODULE.validate_fixture_report(report)

        report["serialized_stippels"] = 1_000
        report["prepared_lifecycle_stippels"] = 999
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "999 prepared ST5B objects have LifeCycle; expected exactly 1000",
        ):
            MODULE.validate_fixture_report(report)

    def test_rejects_a_fixture_not_derived_from_checked_in_arso_morf(self):
        report = MODULE.parse_fixture_line(
            "LC_ARSO_MORF_STIPPEL_FIXTURE "
            "source_stippels=19 prepared_stippels=1000 "
            "source_lifecycle_stippels=19 prepared_lifecycle_stippels=1000 "
            "serialized_stippels=1000 source_objects=1063 "
            "serialized_objects=2043 seed=424242"
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "source fixture contains 19 ST5B objects; expected 20",
        ):
            MODULE.validate_fixture_report(report)

    def test_rejects_non_stippel_object_inventory_drift(self):
        report = MODULE.parse_fixture_line(
            "LC_ARSO_MORF_STIPPEL_FIXTURE "
            "source_stippels=20 prepared_stippels=1000 "
            "source_lifecycle_stippels=20 prepared_lifecycle_stippels=1000 "
            "serialized_stippels=1000 source_objects=1062 "
            "serialized_objects=2042 seed=424242"
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "source fixture contains 1062 objects; expected 1063",
        ):
            MODULE.validate_fixture_report(report)


class NativeCadenceTests(unittest.TestCase):
    BENCHMARK_LINE = (
        "LC_APP_PRESENTATION_BENCHMARK "
        "elapsed_seconds=0.084001 successful_present_submissions=3 "
        "retained_gpu_present_submissions=3 cpu_present_submissions=0 "
        "presentation_submission_fps=35.713860 refreshed_frames=3 "
        "simulation_frames=3 simulation_fps=35.713860 "
        "automatic_graphics_skips=0 average_graphics_pass_ms=7.000000 "
        "max_graphics_pass_ms=9.000000 graphics_pass_sample_count=3 "
        "graphics_pass_p50_ms=7.000000 graphics_pass_p95_ms=9.000000 "
        "graphics_pass_p99_ms=9.000000 "
        "graphics_pass_samples_ns=[5000000, 7000000, 9000000]"
    )
    CONTEXT_LINE = (
        "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=3 "
        "synchronized_player_infos=3 activated_nonhost_clients=0 "
        "runtime_crew_objects=1 runtime_players_with_live_crew=1 "
        "runtime_players_with_exactly_one_live_sf5b_crew=1 "
        "runtime_st5b_objects_at_measurement_start=1000 "
        "runtime_st5b_objects_at_measurement_end=1000"
    )
    NETWORK_LINE = (
        "LC_APP_PRESENTATION_BENCHMARK_NETWORK inspection_status=ok "
        "local_client_id=0 preferred_message_route_peer_count=0 "
        "preferred_message_route_peer_ids=[] tcp_preferred_message_routes=0 "
        "udp_preferred_message_routes=0 unknown_preferred_message_routes=0 "
        "nonnegative_ping_peer_count=0 nonnegative_lag_peer_count=0 "
        "max_nonnegative_ping_ms=-1 max_nonnegative_lag_ms=-1 "
        "host_message_route_lag_ms=-1 max_packet_loss=0 control_presend=0 "
        "avg_control_send_time_us=0"
    )

    def test_native_frame_count_accepts_the_deep_sea_reference_cadence(self):
        report = MODULE.parse_presentation_line(self.BENCHMARK_LINE)

        self.assertEqual(MODULE.required_native_frames(report), 3)
        MODULE.validate_native_cadence(report)

    def test_preserves_every_raw_graphics_pass_sample(self):
        report = MODULE.parse_presentation_line(self.BENCHMARK_LINE)

        self.assertEqual(
            report["graphics_pass_samples_ns"],
            [5_000_000, 7_000_000, 9_000_000],
        )

    def test_preserves_retained_and_cpu_submission_counts(self):
        line = self.BENCHMARK_LINE.replace(
            "retained_gpu_present_submissions=3 cpu_present_submissions=0",
            "retained_gpu_present_submissions=2 cpu_present_submissions=1",
        )

        report = MODULE.parse_presentation_line(line)

        self.assertEqual(report["retained_gpu_present_submissions"], 2)
        self.assertEqual(report["cpu_present_submissions"], 1)

    def test_accepts_legacy_baseline_without_submission_kind_counts(self):
        line = self.BENCHMARK_LINE.replace(
            "retained_gpu_present_submissions=3 cpu_present_submissions=0 ",
            "",
        )

        report = MODULE.parse_presentation_line(line)

        self.assertIsNone(report["retained_gpu_present_submissions"])
        self.assertIsNone(report["cpu_present_submissions"])

    def test_submission_kind_counts_must_reconcile_with_successes(self):
        line = self.BENCHMARK_LINE.replace(
            "retained_gpu_present_submissions=3",
            "retained_gpu_present_submissions=2",
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "submission kind counts total 2 but 3 successful submissions were reported",
        ):
            MODULE.parse_presentation_line(line)

    def test_raw_graphics_samples_cover_every_successful_submission(self):
        line = self.BENCHMARK_LINE.replace(
            "successful_present_submissions=3",
            "successful_present_submissions=4",
        ).replace(
            "retained_gpu_present_submissions=3",
            "retained_gpu_present_submissions=4",
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "3 graphics samples were reported for 4 successful submissions",
        ):
            MODULE.parse_presentation_line(line)

    def test_rejects_nonfinite_presentation_summary_values(self):
        line = self.BENCHMARK_LINE.replace(
            "average_graphics_pass_ms=7.000000",
            "average_graphics_pass_ms=nan",
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "presentation summary values must be finite and nonnegative",
        ):
            MODULE.parse_presentation_line(line)

    def test_rejects_a_truncated_raw_graphics_pass_distribution(self):
        line = self.BENCHMARK_LINE.replace(
            "graphics_pass_sample_count=3",
            "graphics_pass_sample_count=2",
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "graphics pass sample count is 2 but 3 raw samples were reported",
        ):
            MODULE.parse_presentation_line(line)
    def test_native_frame_count_is_exact_at_a_tick_boundary(self):
        report = MODULE.parse_presentation_line(
            self.BENCHMARK_LINE.replace(
                "elapsed_seconds=0.084001", "elapsed_seconds=0.112000"
            )
        )

        self.assertEqual(MODULE.required_native_frames(report), 4)

    def test_native_frame_count_rejects_one_missing_tick(self):
        report = MODULE.parse_presentation_line(
            self.BENCHMARK_LINE.replace(
                "simulation_frames=3", "simulation_frames=2"
            )
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "2 simulation frames; native cadence requires at least 3",
        ):
            MODULE.validate_native_cadence(report)

    def test_native_presentation_cadence_requires_both_frame_counters(self):
        report = MODULE.parse_presentation_line(self.BENCHMARK_LINE)
        MODULE.validate_native_presentation_cadence(report)

        report["refreshed_frames"] = 2
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "2 refreshed frames; native cadence requires at least 3",
        ):
            MODULE.validate_native_presentation_cadence(report)

        report["refreshed_frames"] = 3
        report["successful_present_submissions"] = 2
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "2 successful submissions; native cadence requires at least 3",
        ):
            MODULE.validate_native_presentation_cadence(report)

    def test_requires_the_apps_presentation_budget_result(self):
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "native presentation budget did not report pass",
        ):
            MODULE.require_single_result(
                [self.BENCHMARK_LINE],
                "LC_APP_PRESENTATION_BENCHMARK result=pass native_tick_budget_ms=28",
            )

    def test_paired_evidence_rejects_a_headless_zero_sample_baseline(self):
        headless = (
            self.BENCHMARK_LINE.replace(
                "retained_gpu_present_submissions=3 "
                "cpu_present_submissions=0 ",
                "",
            )
            .replace(
                "successful_present_submissions=3",
                "successful_present_submissions=0",
            )
            .replace("refreshed_frames=3", "refreshed_frames=0")
            .replace("graphics_pass_sample_count=3", "graphics_pass_sample_count=0")
            .replace(
                "graphics_pass_samples_ns=[5000000, 7000000, 9000000]",
                "graphics_pass_samples_ns=[]",
            )
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "paired arm produced no refreshed presentation",
        ):
            MODULE.parse_presentation_evidence(
                [
                    headless,
                    self.CONTEXT_LINE,
                    self.NETWORK_LINE,
                    "LC_APP_PRESENTATION_BENCHMARK result=fail "
                    "error=benchmark produced no refreshed presentation",
                ],
                2,
            )

    def test_requires_ninety_nine_percent_retention_at_both_edges(self):
        context = MODULE.parse_presentation_context_line(self.CONTEXT_LINE)

        MODULE.validate_runtime_stippel_census(context)

        context["runtime_st5b_objects_at_measurement_start"] = 990
        context["runtime_st5b_objects_at_measurement_end"] = 990
        MODULE.validate_runtime_stippel_census(context)

        context["runtime_st5b_objects_at_measurement_start"] = 989
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "measurement started with 989 active ST5B objects; expected at least 990",
        ):
            MODULE.validate_runtime_stippel_census(context)

        context["runtime_st5b_objects_at_measurement_start"] = 990
        context["runtime_st5b_objects_at_measurement_end"] = 989
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "measurement ended with 989 active ST5B objects; expected at least 990",
        ):
            MODULE.validate_runtime_stippel_census(context)

    def test_requires_a_synchronized_playing_host_with_live_crew(self):
        context = MODULE.parse_presentation_context_line(self.CONTEXT_LINE)

        MODULE.validate_playing_context(context)

        unsynchronized = dict(context)
        unsynchronized["synchronized_player_infos"] = 2
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "synchronized_player_infos is 2; expected runtime_players 3",
        ):
            MODULE.validate_playing_context(unsynchronized)

        no_live_crew = dict(context)
        no_live_crew["runtime_players_with_live_crew"] = 0
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "runtime_players_with_live_crew is 0; expected at least 1",
        ):
            MODULE.validate_playing_context(no_live_crew)

    def test_requires_one_successful_network_host_evidence_line(self):
        evidence = MODULE.require_network_evidence([self.NETWORK_LINE])
        self.assertEqual(evidence["inspection_status"], "ok")
        self.assertEqual(evidence["local_client_id"], 0)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "expected exactly one network evidence line; observed 2",
        ):
            MODULE.require_network_evidence([self.NETWORK_LINE, self.NETWORK_LINE])

        failed = self.NETWORK_LINE.replace("inspection_status=ok", "inspection_status=error")
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "network inspection status is error; expected ok",
        ):
            MODULE.require_network_evidence([failed])

        client = self.NETWORK_LINE.replace("local_client_id=0", "local_client_id=1")
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "network host local_client_id is 1; expected 0",
        ):
            MODULE.require_network_evidence([client])


class RetainedGpuProfileTests(unittest.TestCase):
    def test_preserves_raw_profile_and_unknown_future_fields(self):
        profile = retained_gpu_profile()
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        parsed = MODULE.parse_retained_gpu_profile([line], required=True)

        self.assertEqual(parsed, profile)

    def test_accepts_v2_landscape_instance_stream(self):
        profile = retained_gpu_profile_v2()
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        parsed = MODULE.parse_retained_gpu_profile([line], required=True)

        self.assertEqual(parsed, profile)

    def test_v2_requires_landscape_instance_stream_counters(self):
        for key in (
            "landscape_instances",
            "landscape_instance_upload_bytes",
        ):
            with self.subTest(key=key):
                profile = retained_gpu_profile_v2()
                del profile["frames"][0]["renderer"][key]
                line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

                with self.assertRaisesRegex(
                    MODULE.BenchmarkFailure,
                    "retained GPU frame 0 "
                    f"renderer.{key} must be a nonnegative integer",
                ):
                    MODULE.parse_retained_gpu_profile([line], required=True)

    def test_v2_rejects_landscape_instance_stream_byte_count_drift(self):
        profile = retained_gpu_profile_v2()
        profile["frames"][0]["renderer"][
            "landscape_instance_upload_bytes"
        ] += 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU stream bytes do not reconcile for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_cpu_stage_reconciliation_drift(self):
        profile = retained_gpu_profile()
        profile["frames"][0]["cpu"]["unclassified_ns"] += 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU CPU reconciliation failed",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_accepts_an_adapter_that_does_not_report_transient_attachment_savings(self):
        profile = retained_gpu_profile()
        profile["fingerprint"]["adapter"]["transient_saves_memory"] = None
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        parsed = MODULE.parse_retained_gpu_profile([line], required=True)

        self.assertIsNone(parsed["fingerprint"]["adapter"]["transient_saves_memory"])

    def test_rejects_booleans_in_integer_fields(self):
        profile = retained_gpu_profile()
        profile["frames"][0]["renderer"]["draw_calls"] = True
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU frame 0 renderer.draw_calls must be a nonnegative integer",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_integer_fields_outside_the_serialized_u64_range(self):
        profile = retained_gpu_profile()
        profile["frames"][0]["end_to_end_ns"] = 1 << 64
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU frame 0 end_to_end_ns must be a nonnegative integer",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_renderer_draw_count_drift(self):
        profile = retained_gpu_profile()
        profile["frames"][0]["renderer"]["total_draw_calls"] += 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU draw counts do not reconcile for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_stream_byte_count_drift(self):
        profile = retained_gpu_profile()
        profile["frames"][0]["renderer"]["generic_vertex_upload_bytes"] += 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU stream bytes do not reconcile for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_fallback_reason_count_above_generic_total(self):
        profile = retained_gpu_profile()
        capture = profile["frames"][0]["frontend_capture"]
        capture["spatial_fog_fallbacks"] = 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU fallback reasons exceed generic fallbacks for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_fog_chunks_require_a_spatial_fog_fallback(self):
        profile = retained_gpu_profile()
        profile["frames"][0]["frontend_capture"]["fog_expanded_chunks"] = 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU fog chunks lack a spatial fallback for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_malformed_adapter_fingerprint(self):
        profile = retained_gpu_profile()
        profile["fingerprint"]["adapter"]["vendor_id"] = True
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU adapter.vendor_id must be an unsigned 32-bit integer",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_unknown_adapter_enum_values(self):
        for key, value in (
            ("device_type", "quantum_gpu"),
            ("backend", "future_backend"),
        ):
            with self.subTest(key=key):
                profile = retained_gpu_profile()
                profile["fingerprint"]["adapter"][key] = value
                line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

                with self.assertRaisesRegex(
                    MODULE.BenchmarkFailure,
                    f"retained GPU adapter.{key} has an unknown value",
                ):
                    MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_upload_bytes_without_upload_calls(self):
        profile = retained_gpu_profile()
        profile["frames"][0]["renderer"]["dirty_upload_bytes"] = 4
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU dirty upload calls/bytes disagree for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_created_textures_require_full_upload_calls(self):
        profile = retained_gpu_profile()
        renderer = profile["frames"][0]["renderer"]
        renderer["created_source_textures"] = 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU created textures exceed full uploads for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_requires_one_presentation_draw_per_successful_frame(self):
        profile = retained_gpu_profile()
        renderer = profile["frames"][0]["renderer"]
        renderer["presentation_draw_calls"] = 0
        renderer["total_draw_calls"] -= 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU frame 0 must report exactly one presentation draw",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_shader_landscape_draw_requires_enabled_renderer_config(self):
        profile = retained_gpu_profile()
        renderer = profile["frames"][0]["renderer"]
        renderer["shader_landscape_draw_calls"] = 1
        renderer["total_draw_calls"] += 1
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU shader-landscape draw contradicts renderer config for sample 0",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_gpu_timestamp_duration_that_does_not_match_raw_ticks(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        profile["gpu_timestamp_frames"][0]["passes"][0]["duration_ns"] = 9.0
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "GPU timestamp duration does not match raw ticks",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_enabled_timestamps_use_wgpu_native_then_web_feature_words(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())

        parsed = MODULE.parse_retained_gpu_profile(
            ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
            required=True,
        )

        self.assertEqual(
            parsed["fingerprint"]["device"]["feature_bits"], [0, 1 << 7]
        )

    def test_gpu_timestamp_period_must_exactly_match_device_fingerprint(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        profile["gpu_timestamp_frames"][0]["timestamp_period_ns"] = 1.0000000005
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp period disagrees with device fingerprint",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_invalid_gpu_timestamp_but_preserves_raw_ticks(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        sample = profile["gpu_timestamp_frames"][0]["passes"][0]
        sample.update(
            {
                "begin_tick": 250,
                "end_tick": 5,
                "duration_ns": None,
                "validity": "counter_rollover",
            }
        )
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp sample is not valid: counter_rollover",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

        self.assertIn('"begin_tick": 250', line)
        self.assertIn('"end_tick": 5', line)

    def test_explicit_strict_timestamp_policy_rejects_rollover_samples(self):
        profile = retained_gpu_profile_with_rollover()
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp telemetry readback_errors is nonzero",
        ):
            MODULE.parse_retained_gpu_profile(
                [line],
                required=True,
                timestamp_sample_policy="strict",
            )

    def test_tolerant_raw_policy_accepts_valid_scene_and_rollover_presentation(self):
        profile = retained_gpu_profile_with_rollover()

        parsed = MODULE.parse_retained_gpu_profile(
            ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
            required=True,
            timestamp_sample_policy="tolerant_raw",
        )

        self.assertEqual(parsed, profile)

    def test_tolerant_raw_policy_requires_valid_sample_for_every_rendered_pass(self):
        profile = retained_gpu_profile_with_rollover()
        presentation = profile["gpu_timestamp_frames"][0]["passes"][-1]
        presentation.update(
            {
                "begin_tick": 500,
                "end_tick": 400,
                "duration_ns": None,
                "validity": "counter_rollover",
            }
        )
        profile["timestamp_queries"]["readback_errors"] = 2

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp passes have no valid samples: presentation",
        ):
            MODULE.parse_retained_gpu_profile(
                ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
                required=True,
                timestamp_sample_policy="tolerant_raw",
            )

    def test_tolerant_raw_policy_accepts_enumerated_non_rollover_dispositions(self):
        for validity in ("invalid_period", "invalid_duration"):
            with self.subTest(validity=validity):
                profile = retained_gpu_profile_with_rollover()
                presentation = profile["gpu_timestamp_frames"][1]["passes"][-1]
                presentation.update(
                    {
                        "begin_tick": 2_000,
                        "end_tick": 2_500,
                        "validity": validity,
                    }
                )

                parsed = MODULE.parse_retained_gpu_profile(
                    ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
                    required=True,
                    timestamp_sample_policy="tolerant_raw",
                )

                self.assertEqual(
                    parsed["gpu_timestamp_frames"][1]["passes"][-1][
                        "validity"
                    ],
                    validity,
                )

    def test_tolerant_raw_policy_rejects_malformed_invalid_samples(self):
        cases = (
            (
                {"validity": "driver_magic"},
                "invalid validity value",
            ),
            (
                {"duration_ns": 500.0},
                "invalid retained GPU timestamp sample must have null duration",
            ),
            (
                {"end_tick": 2_600},
                "counter_rollover ticks do not roll over",
            ),
            (
                {"begin_tick": True},
                "presentation begin_tick must be a nonnegative integer",
            ),
        )
        for mutation, message in cases:
            with self.subTest(mutation=mutation):
                profile = retained_gpu_profile_with_rollover()
                profile["gpu_timestamp_frames"][1]["passes"][-1].update(
                    mutation
                )

                with self.assertRaisesRegex(MODULE.BenchmarkFailure, message):
                    MODULE.parse_retained_gpu_profile(
                        [
                            "LC_APP_RETAINED_GPU_PROFILE "
                            + json.dumps(profile)
                        ],
                        required=True,
                        timestamp_sample_policy="tolerant_raw",
                    )

    def test_tolerant_raw_policy_reconciles_invalid_frames_with_readback_errors(self):
        profile = retained_gpu_profile_with_rollover()
        profile["timestamp_queries"]["readback_errors"] = 0

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "readback_errors are fewer than frames containing invalid dispositions",
        ):
            MODULE.parse_retained_gpu_profile(
                ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
                required=True,
                timestamp_sample_policy="tolerant_raw",
            )

        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        profile["timestamp_queries"]["readback_errors"] = 1
        parsed = MODULE.parse_retained_gpu_profile(
            ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
            required=True,
            timestamp_sample_policy="tolerant_raw",
        )
        self.assertEqual(parsed["timestamp_queries"]["readback_errors"], 1)

    def test_tolerant_raw_policy_keeps_loss_and_discontinuity_telemetry_strict(self):
        for key in ("dropped_frames", "device_discontinuities"):
            with self.subTest(key=key):
                profile = retained_gpu_profile_with_rollover()
                profile["timestamp_queries"][key] = 1

                with self.assertRaisesRegex(
                    MODULE.BenchmarkFailure,
                    f"retained GPU timestamp telemetry {key} is nonzero",
                ):
                    MODULE.parse_retained_gpu_profile(
                        [
                            "LC_APP_RETAINED_GPU_PROFILE "
                            + json.dumps(profile)
                        ],
                        required=True,
                        timestamp_sample_policy="tolerant_raw",
                    )

    def test_requires_gpu_timestamp_frames_in_cpu_sample_order(self):
        profile = enable_retained_gpu_timestamps(
            retained_gpu_profile_for_durations([5_000_000, 7_000_000])
        )
        profile["gpu_timestamp_frames"].reverse()
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp frames must match CPU frame order",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_requires_gpu_passes_in_render_order(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        passes = profile["gpu_timestamp_frames"][0]["passes"]
        passes[0]["pass"], passes[1]["pass"] = (
            passes[1]["pass"],
            passes[0]["pass"],
        )
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp passes do not match frame 1 draws",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_enabled_timestamps_accept_every_optional_gpu_pass(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        profile["fingerprint"]["renderer"]["shader_landscape"] = True
        renderer = profile["frames"][0]["renderer"]
        renderer["shader_landscape_draw_calls"] = 1
        renderer["total_draw_calls"] += 1
        profile["gpu_timestamp_frames"][0]["passes"].insert(
            0,
            {
                "pass": "shader_landscape",
                "begin_tick": 90,
                "end_tick": 99,
                "duration_ns": 9.0,
                "validity": "valid",
            },
        )

        parsed = MODULE.parse_retained_gpu_profile(
            ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
            required=True,
        )

        self.assertEqual(
            [
                sample["pass"]
                for sample in parsed["gpu_timestamp_frames"][0]["passes"]
            ],
            ["shader_landscape", "scene", "monitor_gamma", "presentation"],
        )

    def test_enabled_timestamps_require_one_gpu_frame_per_cpu_frame(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        profile["gpu_timestamp_frames"].clear()
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp frames must match CPU frame order",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_gpu_timestamp_intervals_must_not_overlap(self):
        profile = enable_retained_gpu_timestamps(retained_gpu_profile())
        monitor = profile["gpu_timestamp_frames"][0]["passes"][1]
        monitor["begin_tick"] = 105
        monitor["duration_ns"] = 11.0
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp intervals are not ordered in frame 1",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_zero_telemetry_requires_one_positive_renderer_generation(self):
        profile = enable_retained_gpu_timestamps(
            retained_gpu_profile_for_durations([5_000_000, 7_000_000])
        )
        profile["gpu_timestamp_frames"][1]["renderer_generation"] = 2
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp renderer generation changed without telemetry",
        ):
            MODULE.parse_retained_gpu_profile([line], required=True)

    def test_rejects_every_nonzero_timestamp_telemetry_counter(self):
        for key in (
            "dropped_frames",
            "readback_errors",
            "device_discontinuities",
        ):
            with self.subTest(key=key):
                profile = retained_gpu_profile()
                profile["timestamp_queries"][key] = 1
                line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

                with self.assertRaisesRegex(
                    MODULE.BenchmarkFailure,
                    f"retained GPU timestamp telemetry {key} is nonzero",
                ):
                    MODULE.parse_retained_gpu_profile([line], required=True)

    def test_unsupported_timestamp_queries_keep_device_features_empty(self):
        profile = retained_gpu_profile()

        parsed = MODULE.parse_retained_gpu_profile(
            ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
            required=True,
        )

        self.assertTrue(parsed["timestamp_queries"]["requested"])
        self.assertFalse(parsed["timestamp_queries"]["supported"])
        self.assertEqual(parsed["fingerprint"]["device"]["feature_bits"], [0, 0])

    def test_tolerant_policy_rejects_readback_errors_when_timestamps_are_disabled(self):
        profile = retained_gpu_profile()
        profile["timestamp_queries"]["readback_errors"] = 1

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp telemetry readback_errors is nonzero",
        ):
            MODULE.parse_retained_gpu_profile(
                ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
                required=True,
                timestamp_sample_policy="tolerant_raw",
            )

    def test_disabled_supported_timestamp_queries_keep_device_features_empty(self):
        profile = retained_gpu_profile()
        profile["fingerprint"]["adapter_feature_bits"] = [0, 1 << 7]
        profile["timestamp_queries"].update(
            {"requested": False, "supported": True, "enabled": False}
        )

        parsed = MODULE.parse_retained_gpu_profile(
            ["LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)],
            required=True,
        )

        self.assertEqual(parsed["fingerprint"]["device"]["feature_bits"], [0, 0])

    def test_optional_profile_can_be_absent_but_required_profile_cannot(self):
        self.assertIsNone(MODULE.parse_retained_gpu_profile([], required=False))

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "required retained GPU profile is missing",
        ):
            MODULE.parse_retained_gpu_profile([], required=True)

    def test_rejects_duplicate_profile_lines_and_duplicate_json_keys(self):
        line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(
            retained_gpu_profile()
        )
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "expected exactly one retained GPU profile; observed 2",
        ):
            MODULE.parse_retained_gpu_profile([line, line], required=True)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "duplicate retained GPU profile key: schema_version",
        ):
            MODULE.parse_retained_gpu_profile(
                [
                    "LC_APP_RETAINED_GPU_PROFILE "
                    '{"schema_version":1,"schema_version":1}'
                ],
                required=True,
            )

    def test_rejects_nonstandard_json_constants_even_in_future_fields(self):
        encoded = json.dumps(retained_gpu_profile())
        encoded = encoded[:-1] + ', "future_number": NaN}'

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "invalid retained GPU profile JSON constant: NaN",
        ):
            MODULE.parse_retained_gpu_profile(
                ["LC_APP_RETAINED_GPU_PROFILE " + encoded], required=True
            )

    def test_profile_frame_count_must_match_retained_submissions(self):
        profile_line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(
            retained_gpu_profile()
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "profile frame count is 1 but 3 retained submissions were reported",
        ):
            MODULE.parse_presentation_evidence(
                [
                    NativeCadenceTests.BENCHMARK_LINE,
                    NativeCadenceTests.CONTEXT_LINE,
                    NativeCadenceTests.NETWORK_LINE,
                    profile_line,
                    MODULE.PRESENTATION_PASS,
                ],
                0,
                require_retained_gpu_profile=True,
            )

    def test_candidate_presentation_evidence_requires_profile_line(self):
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "required retained GPU profile is missing",
        ):
            MODULE.parse_presentation_evidence(
                [
                    NativeCadenceTests.BENCHMARK_LINE,
                    NativeCadenceTests.CONTEXT_LINE,
                    NativeCadenceTests.NETWORK_LINE,
                    MODULE.PRESENTATION_PASS,
                ],
                0,
                require_retained_gpu_profile=True,
                minimum_retained_gpu_profile_schema_version=2,
                expected_timestamp_query_request=True,
            )

    def test_candidate_presentation_evidence_rejects_legacy_profile_schema(self):
        profile = retained_gpu_profile_for_durations(
            [5_000_000, 7_000_000, 9_000_000]
        )
        profile_line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU profile schema_version 1 is older than required 2",
        ):
            MODULE.parse_presentation_evidence(
                [
                    NativeCadenceTests.BENCHMARK_LINE,
                    NativeCadenceTests.CONTEXT_LINE,
                    NativeCadenceTests.NETWORK_LINE,
                    profile_line,
                    MODULE.PRESENTATION_PASS,
                ],
                0,
                require_retained_gpu_profile=True,
                minimum_retained_gpu_profile_schema_version=2,
                expected_timestamp_query_request=True,
            )

    def test_baseline_presentation_evidence_accepts_legacy_profile_schema(self):
        profile = retained_gpu_profile_for_durations(
            [5_000_000, 7_000_000, 9_000_000]
        )
        profile_line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        evidence = MODULE.parse_presentation_evidence(
            [
                NativeCadenceTests.BENCHMARK_LINE,
                NativeCadenceTests.CONTEXT_LINE,
                NativeCadenceTests.NETWORK_LINE,
                profile_line,
                MODULE.PRESENTATION_PASS,
            ],
            0,
            require_retained_gpu_profile=False,
        )

        self.assertEqual(evidence["retained_gpu_profile"]["schema_version"], 1)

    def test_profile_evidence_requires_submission_kind_counts(self):
        profile_line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(
            retained_gpu_profile_v2()
        )
        line = NativeCadenceTests.BENCHMARK_LINE.replace(
            "retained_gpu_present_submissions=3 "
            "cpu_present_submissions=0 ",
            "",
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU profile requires submission kind counts",
        ):
            MODULE.parse_presentation_evidence(
                [
                    line,
                    NativeCadenceTests.CONTEXT_LINE,
                    NativeCadenceTests.NETWORK_LINE,
                    profile_line,
                    MODULE.PRESENTATION_PASS,
                ],
                0,
                require_retained_gpu_profile=True,
                minimum_retained_gpu_profile_schema_version=2,
                expected_timestamp_query_request=True,
            )

    def test_candidate_profile_must_confirm_timestamp_request(self):
        profile = retained_gpu_profile_v2_for_durations(
            [5_000_000, 7_000_000, 9_000_000]
        )
        profile["timestamp_queries"]["requested"] = False
        profile_line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU timestamp request status disagrees with benchmark environment",
        ):
            MODULE.parse_presentation_evidence(
                [
                    NativeCadenceTests.BENCHMARK_LINE,
                    NativeCadenceTests.CONTEXT_LINE,
                    NativeCadenceTests.NETWORK_LINE,
                    profile_line,
                    MODULE.PRESENTATION_PASS,
                ],
                0,
                require_retained_gpu_profile=True,
                minimum_retained_gpu_profile_schema_version=2,
                expected_timestamp_query_request=True,
            )

    def test_candidate_profile_durations_must_match_legacy_samples(self):
        profile = retained_gpu_profile_v2_for_durations(
            [5_000_000, 7_000_000, 9_000_000]
        )
        profile["frames"][0]["end_to_end_ns"] += 1
        profile["frames"][0]["cpu"]["unclassified_ns"] += 1
        profile_line = "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "retained GPU profile durations do not match the legacy raw graphics samples",
        ):
            MODULE.parse_presentation_evidence(
                [
                    NativeCadenceTests.BENCHMARK_LINE,
                    NativeCadenceTests.CONTEXT_LINE,
                    NativeCadenceTests.NETWORK_LINE,
                    profile_line,
                    MODULE.PRESENTATION_PASS,
                ],
                0,
                require_retained_gpu_profile=True,
                minimum_retained_gpu_profile_schema_version=2,
                expected_timestamp_query_request=True,
            )


class NetworkLaunchTests(unittest.TestCase):
    def test_process_environment_clears_ambient_overrides(self):
        environment = MODULE.controlled_process_environment(
            {
                "PATH": "/bin",
                "LC_CONFIG_FILE": "/ambient/config.ini",
                "RUST_LOG": "trace",
                "LC_RUST_ENGINE_RANDOM_SEED": "1",
                "LC_RUST_ENGINE_MAP_SEED": "2",
                "LC_RUST_ENGINE_STARTUP_PLAYERS": "99",
                "LC_GPU_TIMESTAMP_QUERIES": "ambient",
            }
        )

        self.assertEqual(environment["PATH"], "/bin")
        self.assertEqual(environment["LC_INSTALL_ROOT"], str(MODULE.WORKSPACE))
        for key in (
            "LC_CONFIG_FILE",
            "RUST_LOG",
            "LC_RUST_ENGINE_RANDOM_SEED",
            "LC_RUST_ENGINE_MAP_SEED",
            "LC_RUST_ENGINE_STARTUP_PLAYERS",
            "LC_GPU_TIMESTAMP_QUERIES",
        ):
            self.assertNotIn(key, environment)

    def test_candidate_environment_enables_timestamp_queries_without_mutating_base(self):
        base = MODULE.controlled_process_environment({"PATH": "/bin"})

        candidate = MODULE.timestamp_query_process_environment(base)

        self.assertNotIn("LC_GPU_TIMESTAMP_QUERIES", base)
        self.assertEqual(candidate["LC_GPU_TIMESTAMP_QUERIES"], "1")

    def test_app_timeout_fails_closed(self):
        expired = subprocess.TimeoutExpired(["clonk-app"], timeout=32)
        with patch.object(MODULE.subprocess, "run", side_effect=expired):
            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "command timed out after 32 seconds: clonk-app",
            ):
                MODULE.run_and_echo(["clonk-app"], timeout=32)

    def test_process_output_can_be_retained_verbatim(self):
        completed = subprocess.CompletedProcess(
            ["clonk-app"],
            0,
            stdout="one\ntwo\n",
            stderr="warning\n",
        )
        with tempfile.TemporaryDirectory() as temporary:
            stdout_path = Path(temporary) / "stdout.log"
            stderr_path = Path(temporary) / "stderr.log"
            with patch.object(MODULE.subprocess, "run", return_value=completed):
                MODULE.run_and_echo(
                    ["clonk-app"],
                    stdout_path=stdout_path,
                    stderr_path=stderr_path,
                )

            self.assertEqual(stdout_path.read_text(encoding="utf-8"), "one\ntwo\n")
            self.assertEqual(stderr_path.read_text(encoding="utf-8"), "warning\n")

    def test_immediate_host_command_uses_isolated_network_ports(self):
        command = MODULE.app_command(
            SimpleNamespace(app_binary=Path("/bin/clonk-app")),
            config=Path("/tmp/config.ini"),
            fixture=Path("/tmp/Arso-Morf.c4s"),
            ports={"tcp": 21_001, "udp": 21_002, "reference": 21_003},
        )

        self.assertIn("/network", command)
        self.assertIn("/nosignup", command)
        self.assertNotIn("/lobby", command)
        self.assertIn("/tcpport:21001", command)
        self.assertIn("/udpport:21002", command)

    def test_network_config_is_private_and_carries_all_three_ports(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "config.ini"
            MODULE.write_process_config(
                path,
                {"tcp": 21_001, "udp": 21_002, "reference": 21_003},
            )
            text = path.read_text(encoding="utf-8")

        self.assertIn("PortTCP=21001", text)
        self.assertIn("PortUDP=21002", text)
        self.assertIn("PortRefServer=21003", text)
        self.assertIn("PortDiscovery=0", text)
        self.assertIn("MasterServerSignUp=false", text)
        self.assertIn("AutoFrameSkip=true", text)


class PairedBenchmarkTests(unittest.TestCase):
    FIXTURE_LINE = (
        "LC_ARSO_MORF_STIPPEL_FIXTURE "
        "source_stippels=20 prepared_stippels=1000 "
        "source_lifecycle_stippels=20 prepared_lifecycle_stippels=1000 "
        "serialized_stippels=1000 source_objects=1063 "
        "serialized_objects=2043 seed=424242"
    )
    PRESENTATION_LINE = NativeCadenceTests.BENCHMARK_LINE
    CONTEXT_LINE = NativeCadenceTests.CONTEXT_LINE
    NETWORK_LINE = NativeCadenceTests.NETWORK_LINE
    CANDIDATE_PROFILE = retained_gpu_profile_v2_for_durations(
        [5_000_000, 7_000_000, 9_000_000]
    )

    def test_parser_preserves_the_existing_single_arm_cli(self):
        arguments = MODULE.build_argument_parser().parse_args(
            ["17", "--app-binary", "/tmp/single-app"]
        )

        self.assertEqual(arguments.measurement_seconds, 17)
        self.assertEqual(arguments.app_binary, Path("/tmp/single-app"))
        self.assertIsNone(arguments.baseline_app_binary)
        self.assertIsNone(arguments.paired_artifact_dir)

    def test_parser_accepts_explicit_baseline_and_candidate_binaries(self):
        arguments = MODULE.build_argument_parser().parse_args(
            [
                "20",
                "--baseline-app-binary",
                "/tmp/baseline-app",
                "--baseline-source-root",
                "/tmp/origin-main",
                "--candidate-app-binary",
                "/tmp/candidate-app",
                "--paired-artifact-dir",
                "/tmp/artifacts",
            ]
        )

        self.assertEqual(arguments.baseline_app_binary, Path("/tmp/baseline-app"))
        self.assertEqual(arguments.baseline_source_root, Path("/tmp/origin-main"))
        self.assertEqual(arguments.app_binary, Path("/tmp/candidate-app"))
        self.assertEqual(arguments.paired_artifact_dir, Path("/tmp/artifacts"))

    def test_discovers_the_git_worktree_that_built_a_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "origin-main"
            binary = root / "target" / "release" / "clonk-app"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"app")
            (root / ".git").write_text("gitdir: /tmp/worktrees/origin-main\n")
            (root / "Cargo.toml").write_text("[workspace]\n")

            resolved = MODULE.resolve_source_root(None, binary, label="baseline")

        self.assertEqual(resolved, root.resolve())

    def test_paired_arguments_are_all_or_nothing(self):
        arguments = MODULE.build_argument_parser().parse_args(
            ["--baseline-app-binary", "/tmp/baseline-app"]
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "--baseline-app-binary and --paired-artifact-dir must be used together",
        ):
            MODULE.validate_paired_arguments(arguments)

    def test_fixture_and_config_fingerprint_detects_byte_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = root / "Arso-Morf.c4s"
            fixture.mkdir()
            (fixture / "Objects.txt").write_bytes(b"[Object]\nid=ST5B\n")
            (fixture / "Game.txt").write_bytes(b"seed=424242\n")
            config = root / "config.ini"
            config.write_bytes(b"[Graphics]\nResolutionX=800\n")
            expected = MODULE.capture_paired_input_fingerprint(fixture, config)

            MODULE.verify_paired_input_fingerprint(
                expected,
                fixture,
                config,
                stage="before baseline",
            )
            (fixture / "Objects.txt").write_bytes(b"changed\n")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "paired fixture or config changed before baseline",
            ):
                MODULE.verify_paired_input_fingerprint(
                    expected,
                    fixture,
                    config,
                    stage="before baseline",
                )

    def test_binary_provenance_binds_size_and_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "clonk-app"
            binary.write_bytes(b"candidate executable")

            provenance = MODULE.binary_provenance(binary)

        self.assertEqual(provenance["size_bytes"], 20)
        self.assertEqual(
            provenance["sha256"],
            "99470767eb36321a2b5ebe7dc1e9a085fdcf6ac9153712ee554804c438044975",
        )

    def test_source_provenance_ignores_only_root_runtime_update_lock(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(
                ["git", "init", "--quiet"], cwd=root, check=True
            )
            (root / "Cargo.lock").write_text("fixture\n", encoding="utf-8")
            subprocess.run(["git", "add", "Cargo.lock"], cwd=root, check=True)
            subprocess.run(
                [
                    "git",
                    "-c",
                    "user.name=Benchmark Test",
                    "-c",
                    "user.email=benchmark@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture",
                ],
                cwd=root,
                check=True,
            )
            (root / ".clonk-update.lock").write_text("", encoding="utf-8")

            lock_only = MODULE.collect_source_provenance(root)

            (root / "unrelated.tmp").write_text("drift\n", encoding="utf-8")
            (root / "nested").mkdir()
            (root / "nested" / ".clonk-update.lock").write_text(
                "drift\n", encoding="utf-8"
            )
            with_unrelated = MODULE.collect_source_provenance(root)

        self.assertEqual(lock_only["untracked_files"], {})
        self.assertFalse(lock_only["dirty"])
        self.assertEqual(
            set(with_unrelated["untracked_files"]),
            {"nested/.clonk-update.lock", "unrelated.tmp"},
        )
        self.assertTrue(with_unrelated["dirty"])

    def test_single_candidate_run_forces_and_requires_timestamp_profile(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.c4s"
            source.mkdir()
            (source / "Objects.txt").write_bytes(b"id=ST5B\n" * 20)
            embedded_player = root / "embedded_player.c4p"
            embedded_player.write_bytes(b"player")
            candidate = root / "candidate-app"
            builder = root / "fixture-builder"
            for executable in (candidate, builder):
                executable.write_bytes(executable.name.encode("ascii"))
                executable.chmod(0o755)
            arguments = SimpleNamespace(
                app_binary=candidate,
                fixture_builder=builder,
                measurement_seconds=20,
            )
            observed_timestamp_environment = []

            def fake_run(command, **keywords):
                if command[0] == str(builder):
                    fixture = Path(command[1])
                    (fixture / "Objects.txt").write_bytes(b"id=ST5B\n" * 1_000)
                    return [self.FIXTURE_LINE], 0
                observed_timestamp_environment.append(
                    keywords["environment"].get("LC_GPU_TIMESTAMP_QUERIES")
                )
                return [
                    self.PRESENTATION_LINE,
                    self.CONTEXT_LINE,
                    self.NETWORK_LINE,
                    "LC_APP_RETAINED_GPU_PROFILE "
                    + json.dumps(self.CANDIDATE_PROFILE),
                    MODULE.PRESENTATION_PASS,
                ], 0

            with patch.object(MODULE, "SOURCE_SCENARIO", source), patch.object(
                MODULE, "EMBEDDED_PLAYER", embedded_player
            ), patch.object(
                MODULE,
                "allocate_network_ports",
                return_value={"tcp": 21_001, "udp": 21_002, "reference": 21_003},
            ), patch.object(MODULE, "run_and_echo", side_effect=fake_run):
                MODULE.run_benchmark(arguments)

        self.assertEqual(observed_timestamp_environment, ["1"])

    def test_paired_run_reuses_inputs_and_retains_raw_artifacts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.c4s"
            source.mkdir()
            (source / "Objects.txt").write_bytes(b"id=ST5B\n" * 20)
            embedded_player = root / "embedded_player.c4p"
            embedded_player.write_bytes(b"player")
            baseline = root / "baseline-app"
            candidate = root / "candidate-app"
            builder = root / "fixture-builder"
            for executable in (baseline, candidate, builder):
                executable.write_bytes(executable.name.encode("ascii"))
                executable.chmod(0o755)
            artifacts = root / "artifacts"
            arguments = SimpleNamespace(
                app_binary=candidate,
                baseline_app_binary=baseline,
                baseline_source_root=MODULE.WORKSPACE,
                candidate_source_root=MODULE.WORKSPACE,
                fixture_builder=builder,
                paired_artifact_dir=artifacts,
                measurement_seconds=20,
            )
            app_inputs = []

            def fake_run(command, **keywords):
                stdout_path = keywords.get("stdout_path")
                stderr_path = keywords.get("stderr_path")
                if command[0] == str(builder):
                    fixture = Path(command[1])
                    (fixture / "Objects.txt").write_bytes(b"id=ST5B\n" * 1_000)
                    lines = [self.FIXTURE_LINE]
                    status = 0
                else:
                    config = Path(command[2])
                    fixture = Path(command[3])
                    app_inputs.append(
                        (
                            command[0],
                            MODULE.capture_paired_input_fingerprint(fixture, config),
                            keywords["environment"].get(
                                "LC_GPU_TIMESTAMP_QUERIES"
                            ),
                        )
                    )
                    config.write_text(
                        f"saved by {Path(command[0]).name}\n",
                        encoding="utf-8",
                    )
                    result = "pass" if command[0] == str(candidate) else "fail"
                    presentation_line = self.PRESENTATION_LINE
                    if command[0] == str(baseline):
                        presentation_line = presentation_line.replace(
                            "retained_gpu_present_submissions=3 "
                            "cpu_present_submissions=0 ",
                            "",
                        )
                    lines = [
                        presentation_line,
                        self.CONTEXT_LINE,
                        self.NETWORK_LINE,
                        "LC_APP_PRESENTATION_BENCHMARK "
                        f"result={result} native_tick_budget_ms=28",
                    ]
                    if command[0] == str(candidate):
                        lines.insert(
                            3,
                            "LC_APP_RETAINED_GPU_PROFILE "
                            + json.dumps(self.CANDIDATE_PROFILE),
                        )
                    status = 0 if result == "pass" else 2
                if stdout_path is not None:
                    stdout_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
                if stderr_path is not None:
                    stderr_path.write_text("", encoding="utf-8")
                return lines, status

            with patch.object(MODULE, "SOURCE_SCENARIO", source), patch.object(
                MODULE, "EMBEDDED_PLAYER", embedded_player
            ), patch.object(
                MODULE, "allocate_network_ports",
                return_value={"tcp": 21_001, "udp": 21_002, "reference": 21_003},
            ), patch.object(
                MODULE, "run_and_echo", side_effect=fake_run
            ), patch.object(
                MODULE,
                "collect_run_provenance",
                return_value={"test_provenance": True},
            ):
                MODULE.run_paired_benchmark(arguments)

            manifest = json.loads(
                (artifacts / "manifest.json").read_text(encoding="utf-8")
            )
            retained_artifacts = {
                "fixture": (artifacts / "fixture" / "Arso-Morf.c4s").is_dir(),
                "config": (artifacts / "config.ini").is_file(),
                "baseline_stdout": (artifacts / "baseline" / "stdout.log").is_file(),
                "candidate_stderr": (artifacts / "candidate" / "stderr.log").is_file(),
            }

        self.assertEqual([entry[0] for entry in app_inputs], [str(baseline), str(candidate)])
        self.assertEqual(app_inputs[0][1], app_inputs[1][1])
        self.assertEqual([entry[2] for entry in app_inputs], [None, "1"])
        self.assertEqual(manifest["schema_version"], 2)
        self.assertEqual(manifest["result"], "pass")
        self.assertEqual(manifest["runs"]["baseline"]["schema_version"], 2)
        self.assertEqual(manifest["runs"]["candidate"]["schema_version"], 2)
        self.assertEqual(manifest["runs"]["baseline"]["budget_result"], "fail")
        self.assertEqual(manifest["runs"]["candidate"]["budget_result"], "pass")
        self.assertEqual(
            manifest["runs"]["candidate"]["presentation"]["graphics_pass_samples_ns"],
            [5_000_000, 7_000_000, 9_000_000],
        )
        self.assertIsNone(
            manifest["runs"]["baseline"]["retained_gpu_profile"]
        )
        self.assertEqual(
            manifest["runs"]["candidate"]["retained_gpu_profile"],
            self.CANDIDATE_PROFILE,
        )
        self.assertEqual(
            manifest["runs"]["candidate"]["retained_gpu_profile_sha256"],
            MODULE.json_sha256(self.CANDIDATE_PROFILE),
        )
        self.assertIsNone(
            manifest["runs"]["baseline"]["retained_gpu_profile_sha256"]
        )
        self.assertEqual(
            manifest["timestamp_query_environment"],
            {"baseline": None, "candidate": "1"},
        )
        self.assertIsNone(
            manifest["runs"]["baseline"]["timestamp_query_environment"]
        )
        self.assertEqual(
            manifest["runs"]["candidate"]["timestamp_query_environment"],
            "1",
        )
        self.assertTrue(all(retained_artifacts.values()))


if __name__ == "__main__":
    unittest.main()
