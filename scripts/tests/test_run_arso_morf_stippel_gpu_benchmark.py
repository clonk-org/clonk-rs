import importlib.util
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
        "elapsed_seconds=20.002000 successful_present_submissions=1200 "
        "presentation_submission_fps=59.994001 refreshed_frames=1200 "
        "simulation_frames=714 simulation_fps=35.696430 "
        "automatic_graphics_skips=0 average_graphics_pass_ms=5.794000 "
        "max_graphics_pass_ms=9.000000 graphics_pass_sample_count=1200 "
        "graphics_pass_p50_ms=5.000000 graphics_pass_p95_ms=7.000000 "
        "graphics_pass_p99_ms=8.000000 graphics_pass_samples_ns=[5000000]"
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

        self.assertEqual(MODULE.required_native_frames(report), 714)
        MODULE.validate_native_cadence(report)

    def test_native_frame_count_is_exact_at_a_tick_boundary(self):
        report = MODULE.parse_presentation_line(
            self.BENCHMARK_LINE.replace(
                "elapsed_seconds=20.002000", "elapsed_seconds=20.048000"
            )
        )

        self.assertEqual(MODULE.required_native_frames(report), 716)

    def test_native_frame_count_rejects_one_missing_tick(self):
        report = MODULE.parse_presentation_line(
            self.BENCHMARK_LINE.replace(
                "simulation_frames=714", "simulation_frames=713"
            )
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "713 simulation frames; native cadence requires at least 714",
        ):
            MODULE.validate_native_cadence(report)

    def test_native_presentation_cadence_requires_both_frame_counters(self):
        report = MODULE.parse_presentation_line(self.BENCHMARK_LINE)
        MODULE.validate_native_presentation_cadence(report)

        report["refreshed_frames"] = 713
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "713 refreshed frames; native cadence requires at least 714",
        ):
            MODULE.validate_native_presentation_cadence(report)

        report["refreshed_frames"] = 714
        report["successful_present_submissions"] = 713
        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "713 successful submissions; native cadence requires at least 714",
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
        ):
            self.assertNotIn(key, environment)

    def test_app_timeout_fails_closed(self):
        expired = subprocess.TimeoutExpired(["clonk-app"], timeout=32)
        with patch.object(MODULE.subprocess, "run", side_effect=expired):
            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "command timed out after 32 seconds: clonk-app",
            ):
                MODULE.run_and_echo(["clonk-app"], timeout=32)

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


if __name__ == "__main__":
    unittest.main()
