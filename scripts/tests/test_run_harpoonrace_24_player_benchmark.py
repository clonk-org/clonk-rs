import importlib.util
import json
import math
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


SCRIPT = (
    Path(__file__).resolve().parents[1]
    / "run_harpoonrace_24_player_benchmark.py"
)
SPEC = importlib.util.spec_from_file_location("harpoonrace_fleet", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def benchmark_network_line(
    local_client_id=1, players=24, lag_ms=12, host_route_lag_ms=9
):
    peers = [
        client_id
        for client_id in range(players + 1)
        if client_id != local_client_id
    ]
    rendered_peers = ",".join(str(peer) for peer in peers)
    return (
        "LC_APP_PRESENTATION_BENCHMARK_NETWORK inspection_status=ok "
        f"local_client_id={local_client_id} "
        f"preferred_message_route_peer_count={len(peers)} "
        f"preferred_message_route_peer_ids=[{rendered_peers}] "
        f"tcp_preferred_message_routes={len(peers)} "
        "udp_preferred_message_routes=0 "
        "unknown_preferred_message_routes=0 "
        f"nonnegative_ping_peer_count={len(peers)} "
        f"nonnegative_lag_peer_count={len(peers)} "
        f"max_nonnegative_ping_ms=7 max_nonnegative_lag_ms={lag_ms} "
        f"host_message_route_lag_ms={host_route_lag_ms} "
        "max_packet_loss=0 control_presend=4 "
        "avg_control_send_time_us=26813"
    )


def benchmark_network_evidence(
    local_client_id=1, players=24, lag_ms=12, host_route_lag_ms=9
):
    peers = [
        client_id
        for client_id in range(players + 1)
        if client_id != local_client_id
    ]
    return {
        "inspection_status": "ok",
        "local_client_id": local_client_id,
        "preferred_message_route_peer_count": len(peers),
        "preferred_message_route_peer_ids": peers,
        "tcp_preferred_message_routes": len(peers),
        "udp_preferred_message_routes": 0,
        "unknown_preferred_message_routes": 0,
        "nonnegative_ping_peer_count": len(peers),
        "nonnegative_lag_peer_count": len(peers),
        "max_nonnegative_ping_ms": 7,
        "max_nonnegative_lag_ms": lag_ms,
        "host_message_route_lag_ms": host_route_lag_ms,
        "max_packet_loss": 0,
        "control_presend": 4,
        "avg_control_send_time_us": 26_813,
    }


class BenchmarkLineTests(unittest.TestCase):
    NETWORK_LINE = (
        "LC_APP_PRESENTATION_BENCHMARK_NETWORK "
        "inspection_status=ok local_client_id=1 "
        "preferred_message_route_peer_count=2 "
        "preferred_message_route_peer_ids=[0,2] "
        "tcp_preferred_message_routes=1 udp_preferred_message_routes=1 "
        "unknown_preferred_message_routes=0 nonnegative_ping_peer_count=2 "
        "nonnegative_lag_peer_count=2 max_nonnegative_ping_ms=7 "
        "max_nonnegative_lag_ms=12 host_message_route_lag_ms=9 "
        "max_packet_loss=3 control_presend=4 "
        "avg_control_send_time_us=26813"
    )

    def test_per_player_input_line_is_parsed_with_raw_samples(self):
        parsed = MODULE.parse_benchmark_input_player_line(
            "LC_APP_PRESENTATION_BENCHMARK_INPUT_PLAYER player=2 "
            "elapsed_seconds=5.000000 submitted_inputs=2 executed_inputs=2 "
            "pending_inputs=0 input_latency_sample_count=2 "
            "input_latency_p50_ms=100.000000 input_latency_p95_ms=101.000000 "
            "input_latency_p99_ms=101.000000 input_latency_max_ms=101.000000 "
            "input_latency_samples_ns=[100000000,101000000]"
        )

        self.assertEqual(parsed["player"], 2)
        self.assertEqual(parsed["input_latency_samples_ns"], [100_000_000, 101_000_000])

    def test_parses_exact_preferred_message_route_network_evidence(self):
        evidence = MODULE.parse_benchmark_network_line(self.NETWORK_LINE)

        self.assertEqual(evidence["local_client_id"], 1)
        self.assertEqual(evidence["preferred_message_route_peer_ids"], [0, 2])
        self.assertEqual(evidence["preferred_message_route_peer_count"], 2)
        self.assertEqual(evidence["tcp_preferred_message_routes"], 1)
        self.assertEqual(evidence["udp_preferred_message_routes"], 1)
        self.assertEqual(evidence["max_nonnegative_lag_ms"], 12)
        self.assertEqual(evidence["host_message_route_lag_ms"], 9)
        self.assertEqual(evidence["control_presend"], 4)
        self.assertEqual(evidence["avg_control_send_time_us"], 26_813)

    def test_rejects_network_sample_count_without_a_matching_maximum(self):
        inconsistent = self.NETWORK_LINE.replace(
            "nonnegative_lag_peer_count=2 max_nonnegative_ping_ms=7 "
            "max_nonnegative_lag_ms=12",
            "nonnegative_lag_peer_count=0 max_nonnegative_ping_ms=7 "
            "max_nonnegative_lag_ms=12",
        )

        with self.assertRaisesRegex(
            ValueError,
            "max_nonnegative_lag_ms must be -1 when no lag samples exist",
        ):
            MODULE.parse_benchmark_network_line(inconsistent)

    def test_rejects_network_sample_coverage_without_a_maximum(self):
        inconsistent = self.NETWORK_LINE.replace(
            "max_nonnegative_ping_ms=7",
            "max_nonnegative_ping_ms=-1",
        )

        with self.assertRaisesRegex(
            ValueError,
            "max_nonnegative_ping_ms must be nonnegative when ping samples exist",
        ):
            MODULE.parse_benchmark_network_line(inconsistent)

    def test_parses_raw_graphics_samples_and_required_metrics(self):
        line = (
            "LC_APP_PRESENTATION_BENCHMARK "
            "elapsed_seconds=60.000000 successful_present_submissions=3 "
            "presentation_submission_fps=50.000000 refreshed_frames=3 "
            "simulation_frames=2100 simulation_fps=35.000000 "
            "automatic_graphics_skips=0 average_graphics_pass_ms=4.000000 "
            "max_graphics_pass_ms=7.000000 graphics_pass_sample_count=3 "
            "graphics_pass_p50_ms=4.000000 graphics_pass_p95_ms=7.000000 "
            "graphics_pass_p99_ms=7.000000 "
            "graphics_pass_samples_ns=[1000000,4000000,7000000]"
        )

        report = MODULE.parse_benchmark_machine_line(line)

        self.assertEqual(report["graphics_pass_sample_count"], 3)
        self.assertEqual(
            report["graphics_pass_samples_ns"],
            [1_000_000, 4_000_000, 7_000_000],
        )
        self.assertEqual(report["simulation_fps"], 35.0)

    def test_parses_input_latency_evidence_with_raw_samples(self):
        line = (
            "LC_APP_PRESENTATION_BENCHMARK_INPUT elapsed_seconds=60 "
            "submitted_inputs=240 executed_inputs=240 pending_inputs=0 "
            "input_latency_sample_count=240 input_latency_p50_ms=50 "
            "input_latency_p95_ms=100 input_latency_p99_ms=125 "
            "input_latency_max_ms=150 "
            "input_latency_samples_ns=[50000000,100000000,125000000,150000000]"
        )

        report = MODULE.parse_benchmark_input_line(line)

        self.assertEqual(report["submitted_inputs"], 240)
        self.assertEqual(report["input_latency_samples_ns"][2], 125_000_000)

    def test_input_probe_rejects_inconsistent_pending_or_sample_accounting(self):
        report = {
            "elapsed_seconds": 60.0,
            "submitted_inputs": 240,
            "executed_inputs": 238,
            "pending_inputs": 1,
            "input_latency_sample_count": 238,
            "input_latency_p50_ms": 50.0,
            "input_latency_p95_ms": 100.0,
            "input_latency_p99_ms": 100.0,
            "input_latency_max_ms": 100.0,
            "input_latency_samples_ns": [100_000_000] * 238,
        }

        failures = MODULE.input_probe_failures(
            report,
            expected_seconds=60,
            interval_ms=500,
            maximum_latency_ms=100.0,
            minimum_success_percent=95.0,
        )

        self.assertTrue(
            any("executed plus pending inputs" in failure for failure in failures)
        )

    def test_input_probe_accepts_exactly_95_percent_with_pending_inputs(self):
        samples = [100_000_000] * 228
        report = {
            "elapsed_seconds": 60.0,
            "submitted_inputs": 240,
            "executed_inputs": 228,
            "pending_inputs": 12,
            "input_latency_sample_count": 228,
            "input_latency_p50_ms": 100.0,
            "input_latency_p95_ms": 100.0,
            "input_latency_p99_ms": 100.0,
            "input_latency_max_ms": 100.0,
            "input_latency_samples_ns": samples,
        }

        self.assertEqual(
            MODULE.input_probe_failures(
                report,
                expected_seconds=60,
                interval_ms=500,
                maximum_latency_ms=100.0,
                minimum_success_percent=95.0,
            ),
            [],
        )

    def test_input_probe_requires_expected_volume_and_95_percent_threshold(self):
        samples = [100_000_000] * 225 + [101_000_000] * 12
        report = {
            "elapsed_seconds": 60.0,
            "submitted_inputs": 237,
            "executed_inputs": 237,
            "pending_inputs": 0,
            "input_latency_sample_count": 237,
            "input_latency_p50_ms": 100.0,
            "input_latency_p95_ms": 101.0,
            "input_latency_p99_ms": 101.0,
            "input_latency_max_ms": 101.0,
            "input_latency_samples_ns": samples,
        }

        failures = MODULE.input_probe_failures(
            report,
            expected_seconds=60,
            interval_ms=500,
            maximum_latency_ms=100.0,
            minimum_success_percent=95.0,
        )

        self.assertTrue(any("minimum expected volume" in failure for failure in failures))
        self.assertTrue(any("within 100.000ms" in failure for failure in failures))

    def test_rejects_duplicate_machine_results(self):
        metric = (
            "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=1 "
            "successful_present_submissions=1 presentation_submission_fps=1 "
            "refreshed_frames=1 simulation_frames=35 simulation_fps=35 "
            "automatic_graphics_skips=0 average_graphics_pass_ms=1 "
            "max_graphics_pass_ms=1 graphics_pass_sample_count=1 "
            "graphics_pass_p50_ms=1 graphics_pass_p95_ms=1 "
            "graphics_pass_p99_ms=1 graphics_pass_samples_ns=[1000000]"
        )

        context = (
            "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=24 "
            "synchronized_player_infos=24 activated_nonhost_clients=24 "
            "runtime_crew_objects=24 "
            "runtime_players_with_live_crew=24 "
            "runtime_players_with_exactly_one_live_sf5b_crew=24"
        )
        with self.assertRaisesRegex(ValueError, "exactly one metric"):
            MODULE.extract_benchmark_report(
                metric
                + "\n"
                + metric
                + "\n"
                + context
                + "\n"
                + benchmark_network_line(),
                "",
            )

    def test_context_requires_the_generic_per_player_live_crew_gate(self):
        context = (
            "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=24 "
            "synchronized_player_infos=24 activated_nonhost_clients=24 "
            "runtime_crew_objects=24 "
            "runtime_players_with_exactly_one_live_sf5b_crew=24"
        )

        with self.assertRaisesRegex(
            ValueError,
            "runtime_players_with_live_crew",
        ):
            MODULE.parse_benchmark_context_line(context)

    def test_metric_and_context_are_complete_without_an_in_process_assertion(self):
        metric = (
            "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=60 "
            "successful_present_submissions=2100 "
            "presentation_submission_fps=35 "
            "refreshed_frames=2100 simulation_frames=2100 "
            "simulation_fps=35 automatic_graphics_skips=0 "
            "average_graphics_pass_ms=1 max_graphics_pass_ms=1 "
            "graphics_pass_sample_count=1 graphics_pass_p50_ms=1 "
            "graphics_pass_p95_ms=1 graphics_pass_p99_ms=1 "
            "graphics_pass_samples_ns=[1000000]"
        )
        context = (
            "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=24 "
            "synchronized_player_infos=24 activated_nonhost_clients=24 "
            "runtime_crew_objects=24 "
            "runtime_players_with_live_crew=24 "
            "runtime_players_with_exactly_one_live_sf5b_crew=24"
        )

        report = MODULE.extract_benchmark_report(
            metric + "\n" + context + "\n" + benchmark_network_line(), ""
        )

        self.assertEqual(report["simulation_fps"], 35.0)
        self.assertEqual(report["benchmark_context"]["runtime_players"], 24)

    def test_requires_exact_24_player_runtime_context(self):
        metric = (
            "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=60 "
            "successful_present_submissions=1 presentation_submission_fps=1 "
            "refreshed_frames=1 simulation_frames=2100 simulation_fps=35 "
            "automatic_graphics_skips=0 average_graphics_pass_ms=1 "
            "max_graphics_pass_ms=1 graphics_pass_sample_count=1 "
            "graphics_pass_p50_ms=1 graphics_pass_p95_ms=1 "
            "graphics_pass_p99_ms=1 graphics_pass_samples_ns=[1000000]"
        )
        context = (
            "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=24 "
            "synchronized_player_infos=24 activated_nonhost_clients=24 "
            "runtime_crew_objects=24 "
            "runtime_players_with_live_crew=24 "
            "runtime_players_with_exactly_one_live_sf5b_crew=24"
        )

        report = MODULE.extract_benchmark_report(
            metric
            + "\n"
            + context
            + "\n"
            + benchmark_network_line()
            + "\n"
            + MODULE.MACHINE_PASS,
            "",
        )

        self.assertEqual(
            report["benchmark_context"],
            {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 24,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 24,
            },
        )

        report["benchmark_context"][
            "runtime_players_with_exactly_one_live_sf5b_crew"
        ] = 23
        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=60,
            expected_players=24,
            minimum_simulation_fps=35.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
        )
        self.assertTrue(
            any(
                "runtime_players_with_exactly_one_live_sf5b_crew" in failure
                for failure in failures
            )
        )

        report["benchmark_context"][
            "runtime_players_with_exactly_one_live_sf5b_crew"
        ] = 24
        report["benchmark_context"]["runtime_crew_objects"] = 23
        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=60,
            expected_players=24,
            minimum_simulation_fps=35.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
        )
        self.assertFalse(
            any("runtime_crew_objects" in failure for failure in failures)
        )

    def test_acceptance_enforces_native_cadence_and_performance_p99(self):
        passing = {
            "elapsed_seconds": 60.0,
            "successful_present_submissions": 2100,
            "presentation_submission_fps": 35.0,
            "refreshed_frames": 2100,
            "simulation_frames": 2100,
            "simulation_fps": 35.0,
            "automatic_graphics_skips": 0,
            "average_graphics_pass_ms": 24.0,
            "max_graphics_pass_ms": 24.0,
            "graphics_pass_sample_count": 2100,
            "graphics_pass_p50_ms": 24.0,
            "graphics_pass_p95_ms": 24.0,
            "graphics_pass_p99_ms": 24.0,
            "graphics_pass_samples_ns": [24_000_000] * 2100,
            "benchmark_context": {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 24,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 24,
            },
            "network_evidence": benchmark_network_evidence(),
        }
        self.assertEqual(
            MODULE.benchmark_failures(
                passing,
                expected_seconds=60,
                expected_players=24,
                minimum_simulation_fps=35.0,
                minimum_presentation_fps=35.0,
                maximum_graphics_p99_ms=25.0,
                maximum_network_lag_ms=100.0,
            ),
            [],
        )

        slow = dict(passing)
        slow["simulation_fps"] = 34.999
        slow["graphics_pass_p99_ms"] = 25.0
        failures = MODULE.benchmark_failures(
            slow,
            expected_seconds=60,
            expected_players=24,
            minimum_simulation_fps=35.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
        )
        self.assertTrue(any("simulation FPS" in failure for failure in failures))
        self.assertTrue(any("graphics p99" in failure for failure in failures))

        sparse = dict(passing)
        sparse["successful_present_submissions"] = 1
        sparse["presentation_submission_fps"] = 1.0 / 60.0
        sparse["refreshed_frames"] = 1
        sparse["graphics_pass_sample_count"] = 1
        sparse["graphics_pass_samples_ns"] = [24_000_000]
        failures = MODULE.benchmark_failures(
            sparse,
            expected_seconds=60,
            expected_players=24,
            minimum_simulation_fps=35.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
        )
        self.assertTrue(
            any("presentation FPS" in failure for failure in failures)
        )

    def test_runtime_only_acceptance_allows_an_occluded_client(self):
        report = {
            "elapsed_seconds": 300.0,
            "successful_present_submissions": 0,
            "presentation_submission_fps": 0.0,
            "refreshed_frames": 0,
            "simulation_frames": 11_400,
            "simulation_fps": 38.0,
            "automatic_graphics_skips": 0,
            "average_graphics_pass_ms": 0.0,
            "max_graphics_pass_ms": 0.0,
            "graphics_pass_sample_count": 0,
            "graphics_pass_p50_ms": 0.0,
            "graphics_pass_p95_ms": 0.0,
            "graphics_pass_p99_ms": 0.0,
            "graphics_pass_samples_ns": [],
            "benchmark_context": {
                "runtime_players": 4,
                "synchronized_player_infos": 4,
                "activated_nonhost_clients": 4,
                "runtime_crew_objects": 4,
                "runtime_players_with_live_crew": 4,
                "runtime_players_with_exactly_one_live_sf5b_crew": 0,
            },
            "network_evidence": benchmark_network_evidence(players=4),
        }

        self.assertEqual(
            MODULE.benchmark_failures(
                report,
                expected_seconds=300,
                expected_players=4,
                minimum_simulation_fps=38.0,
                minimum_presentation_fps=35.0,
                maximum_graphics_p99_ms=25.0,
                maximum_network_lag_ms=100.0,
                require_sf5b_crew=False,
                require_presentation=False,
            ),
            [],
        )

        report["simulation_fps"] = 37.999
        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=300,
            expected_players=4,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
        )
        self.assertTrue(any("simulation FPS" in failure for failure in failures))

        report["simulation_fps"] = 38.0
        report["benchmark_context"]["runtime_players_with_live_crew"] = 3
        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=300,
            expected_players=4,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
        )
        self.assertTrue(
            any("runtime_players_with_live_crew" in failure for failure in failures)
        )

    def test_acceptance_requires_exact_mesh_complete_samples_and_low_lag(self):
        report = {
            "elapsed_seconds": 60.0,
            "successful_present_submissions": 1,
            "presentation_submission_fps": 35.0,
            "refreshed_frames": 1,
            "simulation_frames": 2_100,
            "simulation_fps": 35.0,
            "automatic_graphics_skips": 0,
            "average_graphics_pass_ms": 1.0,
            "max_graphics_pass_ms": 1.0,
            "graphics_pass_sample_count": 1,
            "graphics_pass_p50_ms": 1.0,
            "graphics_pass_p95_ms": 1.0,
            "graphics_pass_p99_ms": 1.0,
            "graphics_pass_samples_ns": [1_000_000],
            "benchmark_context": {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 24,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 24,
            },
            "network_evidence": benchmark_network_evidence(
                lag_ms=101
            ),
        }
        report["network_evidence"]["preferred_message_route_peer_ids"].pop()
        report["network_evidence"]["preferred_message_route_peer_count"] -= 1
        report["network_evidence"]["tcp_preferred_message_routes"] -= 1
        report["network_evidence"]["nonnegative_ping_peer_count"] -= 2
        report["network_evidence"]["nonnegative_lag_peer_count"] -= 2

        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=60,
            expected_players=24,
            minimum_simulation_fps=35.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
        )

        self.assertTrue(any("peer coverage" in failure for failure in failures))
        self.assertTrue(
            any("ping coverage 22/24" in failure for failure in failures)
        )
        self.assertTrue(
            any("lag coverage 22/24" in failure for failure in failures)
        )
        self.assertTrue(
            any("maximum message-route lag 101ms" in failure for failure in failures)
        )

    def test_async_control_gates_the_host_route_not_idle_peer_lag(self):
        report = {
            "elapsed_seconds": 25.0,
            "successful_present_submissions": 0,
            "presentation_submission_fps": 0.0,
            "refreshed_frames": 0,
            "simulation_frames": 962,
            "simulation_fps": 38.48,
            "automatic_graphics_skips": 0,
            "average_graphics_pass_ms": 0.0,
            "max_graphics_pass_ms": 0.0,
            "graphics_pass_sample_count": 0,
            "graphics_pass_p50_ms": 0.0,
            "graphics_pass_p95_ms": 0.0,
            "graphics_pass_p99_ms": 0.0,
            "graphics_pass_samples_ns": [],
            "benchmark_context": {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 24,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 0,
            },
            "network_evidence": benchmark_network_evidence(
                lag_ms=15_000,
                host_route_lag_ms=99,
            ),
        }
        network = report["network_evidence"]
        network["preferred_message_route_peer_ids"].remove(24)
        network["preferred_message_route_peer_count"] -= 1
        network["tcp_preferred_message_routes"] -= 1
        network["nonnegative_ping_peer_count"] -= 1
        network["nonnegative_lag_peer_count"] -= 1

        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=25,
            expected_players=24,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
            control_mode=2,
        )

        self.assertEqual(failures, [])
        report["network_evidence"]["host_message_route_lag_ms"] = 101
        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=25,
            expected_players=24,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
            control_mode=2,
        )
        self.assertTrue(
            any("host message-route lag 101ms" in failure for failure in failures)
        )

        report["network_evidence"]["host_message_route_lag_ms"] = -1
        report["network_evidence"]["preferred_message_route_peer_ids"].remove(0)
        report["network_evidence"]["preferred_message_route_peer_count"] -= 1
        report["network_evidence"]["tcp_preferred_message_routes"] -= 1
        report["network_evidence"]["nonnegative_ping_peer_count"] -= 1
        report["network_evidence"]["nonnegative_lag_peer_count"] -= 1
        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=25,
            expected_players=24,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
            control_mode=2,
        )
        self.assertTrue(any("host route" in failure for failure in failures))

    def test_grouped_fleet_separates_player_and_client_context(self):
        report = {
            "elapsed_seconds": 25.0,
            "successful_present_submissions": 0,
            "presentation_submission_fps": 0.0,
            "refreshed_frames": 0,
            "simulation_frames": 962,
            "simulation_fps": 38.48,
            "automatic_graphics_skips": 0,
            "average_graphics_pass_ms": 0.0,
            "max_graphics_pass_ms": 0.0,
            "graphics_pass_sample_count": 0,
            "graphics_pass_p50_ms": 0.0,
            "graphics_pass_p95_ms": 0.0,
            "graphics_pass_p99_ms": 0.0,
            "graphics_pass_samples_ns": [],
            "benchmark_context": {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 12,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 0,
            },
            "network_evidence": benchmark_network_evidence(players=12),
        }

        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=25,
            expected_players=24,
            expected_clients=12,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
            control_mode=2,
        )

        self.assertEqual(failures, [])

    def test_grouped_input_requires_evidence_for_each_local_player(self):
        samples = [50_000_000] * 100
        aggregate = {
            "elapsed_seconds": 25.0,
            "submitted_inputs": 100,
            "executed_inputs": 100,
            "pending_inputs": 0,
            "input_latency_sample_count": 100,
            "input_latency_p50_ms": 50.0,
            "input_latency_p95_ms": 50.0,
            "input_latency_p99_ms": 50.0,
            "input_latency_max_ms": 50.0,
            "input_latency_samples_ns": samples,
        }
        report = {
            "elapsed_seconds": 25.0,
            "successful_present_submissions": 0,
            "presentation_submission_fps": 0.0,
            "refreshed_frames": 0,
            "simulation_frames": 962,
            "simulation_fps": 38.48,
            "automatic_graphics_skips": 0,
            "average_graphics_pass_ms": 0.0,
            "max_graphics_pass_ms": 0.0,
            "graphics_pass_sample_count": 0,
            "graphics_pass_p50_ms": 0.0,
            "graphics_pass_p95_ms": 0.0,
            "graphics_pass_p99_ms": 0.0,
            "graphics_pass_samples_ns": [],
            "benchmark_context": {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 12,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 0,
            },
            "network_evidence": benchmark_network_evidence(players=12),
            "input_probe": aggregate,
            "input_probe_players": [{"player": 1, **aggregate}],
        }

        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=25,
            expected_players=24,
            expected_clients=12,
            expected_local_players=2,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
            input_probe_interval_ms=500,
            maximum_input_latency_ms=100.0,
            minimum_input_success_percent=95.0,
            control_mode=2,
        )

        self.assertTrue(any("per-player input results" in failure for failure in failures))

        second = {"player": 2, **aggregate}
        report["input_probe_players"] = [report["input_probe_players"][0], second]
        report["input_probe"] = {
            **aggregate,
            "submitted_inputs": 200,
            "executed_inputs": 200,
            "input_latency_sample_count": 200,
            "input_latency_samples_ns": samples + samples,
        }
        passing = MODULE.benchmark_failures(
            report,
            expected_seconds=25,
            expected_players=24,
            expected_clients=12,
            expected_local_players=2,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
            input_probe_interval_ms=500,
            maximum_input_latency_ms=100.0,
            minimum_input_success_percent=95.0,
            control_mode=2,
        )
        self.assertEqual(passing, [])

        first_samples = [50_000_000] * 120
        second_samples = [50_000_000] * 94 + [101_000_000] * 6
        report["input_probe_players"] = [
            {
                "player": 1,
                **aggregate,
                "submitted_inputs": 120,
                "executed_inputs": 120,
                "input_latency_sample_count": 120,
                "input_latency_samples_ns": first_samples,
            },
            {
                "player": 2,
                **aggregate,
                "input_latency_p95_ms": 101.0,
                "input_latency_p99_ms": 101.0,
                "input_latency_max_ms": 101.0,
                "input_latency_samples_ns": second_samples,
            },
        ]
        report["input_probe"] = {
            **aggregate,
            "submitted_inputs": 220,
            "executed_inputs": 220,
            "input_latency_sample_count": 220,
            "input_latency_p99_ms": 101.0,
            "input_latency_max_ms": 101.0,
            "input_latency_samples_ns": first_samples + second_samples,
        }
        per_player_failure = MODULE.benchmark_failures(
            report,
            expected_seconds=25,
            expected_players=24,
            expected_clients=12,
            expected_local_players=2,
            minimum_simulation_fps=38.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
            require_sf5b_crew=False,
            require_presentation=False,
            input_probe_interval_ms=500,
            maximum_input_latency_ms=100.0,
            minimum_input_success_percent=95.0,
            control_mode=2,
        )
        self.assertTrue(
            any("player 2: input latency 94.000%" in failure for failure in per_player_failure)
        )

    def test_rare_cpp_automatic_skip_is_reported_but_not_an_independent_failure(self):
        # C++ src/C4Application.cpp:463-476 deliberately skips the next
        # graphics pass after an over-budget pass. Smoothness is governed by
        # the FPS and p99 gates; occurrence alone is not a parity failure.
        report = {
            "elapsed_seconds": 60.0,
            "successful_present_submissions": 2_140,
            "presentation_submission_fps": 35.666667,
            "refreshed_frames": 2_140,
            "simulation_frames": 2_140,
            "simulation_fps": 35.666667,
            "automatic_graphics_skips": 1,
            "average_graphics_pass_ms": 4.0,
            "max_graphics_pass_ms": 30.0,
            "graphics_pass_sample_count": 2,
            "graphics_pass_p50_ms": 4.0,
            "graphics_pass_p95_ms": 20.0,
            "graphics_pass_p99_ms": 20.0,
            "graphics_pass_samples_ns": [4_000_000, 20_000_000],
            "benchmark_context": {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 24,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 24,
            },
            "network_evidence": benchmark_network_evidence(),
        }

        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=60,
            expected_players=24,
            minimum_simulation_fps=35.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
        )

        self.assertFalse(
            any("automatic graphics skips" in failure for failure in failures)
        )

    def test_average_graphics_pass_still_obeys_cpp_native_tick_budget(self):
        # C++ src/C4Application.cpp:472-476 uses the 28ms game-tick budget to
        # decide whether graphics are slowing down the game.
        report = {
            "elapsed_seconds": 60.0,
            "successful_present_submissions": 100,
            "presentation_submission_fps": 35.0,
            "refreshed_frames": 100,
            "simulation_frames": 2_100,
            "simulation_fps": 35.0,
            "automatic_graphics_skips": 0,
            "average_graphics_pass_ms": 28.001,
            "max_graphics_pass_ms": 1_000.0,
            "graphics_pass_sample_count": 100,
            "graphics_pass_p50_ms": 1.0,
            "graphics_pass_p95_ms": 1.0,
            "graphics_pass_p99_ms": 1.0,
            "graphics_pass_samples_ns": [1_000_000] * 100,
            "benchmark_context": {
                "runtime_players": 24,
                "synchronized_player_infos": 24,
                "activated_nonhost_clients": 24,
                "runtime_crew_objects": 24,
                "runtime_players_with_live_crew": 24,
                "runtime_players_with_exactly_one_live_sf5b_crew": 24,
            },
            "network_evidence": benchmark_network_evidence(),
        }

        failures = MODULE.benchmark_failures(
            report,
            expected_seconds=60,
            expected_players=24,
            minimum_simulation_fps=35.0,
            minimum_presentation_fps=35.0,
            maximum_graphics_p99_ms=25.0,
            maximum_network_lag_ms=100.0,
        )

        self.assertTrue(
            any("average graphics pass" in failure for failure in failures)
        )


class ExactReferenceTests(unittest.TestCase):
    def test_reads_a_generic_scenario_title_from_context(self):
        with tempfile.TemporaryDirectory() as temporary:
            scenario = Path(temporary)
            (scenario / "Scenario.txt").write_text(
                "[Head]\nTitle=Deep Sea\n", encoding="cp1252"
            )

            self.assertEqual(
                MODULE.scenario_title_from_file(scenario), "Deep Sea"
            )

    def test_prefers_the_us_title_resource_used_by_the_lobby_reference(self):
        with tempfile.TemporaryDirectory() as temporary:
            scenario = Path(temporary)
            (scenario / "Scenario.txt").write_text(
                "[Head]\nTitle=Predator\n", encoding="cp1252"
            )
            (scenario / "Title.txt").write_text(
                "DE:AH - Predator\r\nUS:AH - Predator\r\n",
                encoding="cp1252",
            )

            self.assertEqual(
                MODULE.scenario_title_from_file(scenario), "AH - Predator"
            )

    REFERENCE = (
        "[Reference]\r\n"
        "State=Lobby\r\n"
        "MaxPlayers=24\r\n"
        'Title="HarpoonRace"\r\n'
        "\r\n"
        "  [PlayerInfos]\r\n"
        "  LastPlayerID=2\r\n"
        "\r\n"
        "    [Client]\r\n"
        "    ID=1\r\n"
        "\r\n"
        "      [Player]\r\n"
        '      Name="LoadPlayer-01"\r\n'
        "      Flags=Joined\r\n"
        "\r\n"
        "    [Client]\r\n"
        "    ID=8\r\n"
        "\r\n"
        "      [Player]\r\n"
        '      Name="RemovedPlayer"\r\n'
        "      Flags=Joined|Removed\r\n"
        "\r\n"
        "  [RestorePlayerInfos]\r\n"
        "\r\n"
        "    [Client]\r\n"
        "    ID=9\r\n"
        "\r\n"
        "      [Player]\r\n"
        '      Name="StalePlayer"\r\n'
        "\r\n"
        "  [Client]\r\n"
        "  ID=0\r\n"
        "  Activated=true\r\n"
        '  Name="LoadHost"\r\n'
        "\r\n"
        "  [Client]\r\n"
        "  ID=1\r\n"
        "  Activated=true\r\n"
        '  Name="LoadClient-01"\r\n'
        "\r\n"
        "  [Client]\r\n"
        "  ID=2\r\n"
        '  Name="LoadClient-02"\r\n'
    )

    def test_parses_real_serializer_shape_without_name_collisions(self):
        self.assertTrue(
            MODULE.reference_is_lobby(
                self.REFERENCE, title="HarpoonRace", max_players=24
            )
        )
        self.assertTrue(
            MODULE.reference_has_player(self.REFERENCE, "LoadPlayer-01")
        )
        self.assertFalse(
            MODULE.reference_has_player(self.REFERENCE, "StalePlayer")
        )
        # C++ StdAdaptors.h:923-947 serializes bitfields with `|`, and
        # C4PlayerInfo.cpp:327-330 retains Joined when adding Removed.
        self.assertFalse(
            MODULE.reference_has_player(self.REFERENCE, "RemovedPlayer")
        )
        self.assertFalse(
            MODULE.reference_has_player(self.REFERENCE, "LoadClient-01")
        )
        self.assertTrue(
            MODULE.reference_has_activated_clients(
                self.REFERENCE, {"LoadClient-01"}
            )
        )
        self.assertFalse(
            MODULE.reference_has_activated_clients(
                self.REFERENCE, {"LoadClient-01", "LoadClient-02"}
            )
        )

    def test_wait_for_reference_accepts_description_timeout_predicate(self):
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.host = {"process": mock.Mock(poll=mock.Mock(return_value=None))}
        runner.arguments = SimpleNamespace(base_port=31_111)
        runner.event = mock.Mock()
        with mock.patch.object(
            MODULE, "fetch_reference", return_value=self.REFERENCE
        ):
            observed = runner.wait_for_reference(
                "quoted HarpoonRace lobby",
                0.5,
                lambda reference: MODULE.reference_is_lobby(
                    reference, title="HarpoonRace", max_players=24
                ),
            )
        self.assertEqual(observed, self.REFERENCE)

    def test_player_admission_requires_one_current_coexisting_fleet(self):
        # C++ C4Network2Players.cpp:465-481 sends GO-time joins only for
        # players whose owning clients currently coexist and are activated.
        def reference(player_name, client_name, client_id):
            return (
                "[Reference]\r\n"
                "State=Lobby\r\n"
                "MaxPlayers=2\r\n"
                'Title="HarpoonRace"\r\n'
                "  [PlayerInfos]\r\n"
                "    [Client]\r\n"
                f"    ID={client_id}\r\n"
                "      [Player]\r\n"
                f'      Name="{player_name}"\r\n'
                "      Flags=Joined\r\n"
                "  [Client]\r\n"
                f"  ID={client_id}\r\n"
                "  Activated=true\r\n"
                f'  Name="{client_name}"\r\n'
            )

        with tempfile.TemporaryDirectory() as temporary:
            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.host = {
                "process": mock.Mock(poll=mock.Mock(return_value=None))
            }
            runner.arguments = SimpleNamespace(
                base_port=31_111,
                join_timeout=1.0,
                players=2,
            )
            runner.clients = [
                {
                    "index": 1,
                    "name": "LoadClient-01",
                    "client_name": "LoadClient-01",
                    "player_name": "LoadPlayer-01",
                    "launched_monotonic": 0.0,
                    "process": mock.Mock(poll=mock.Mock(return_value=None)),
                },
                {
                    "index": 2,
                    "name": "LoadClient-02",
                    "client_name": "LoadClient-02",
                    "player_name": "LoadPlayer-02",
                    "launched_monotonic": 0.0,
                    "process": mock.Mock(poll=mock.Mock(return_value=None)),
                },
            ]
            runner.admission_samples = []
            runner.artifact_dir = Path(temporary)
            runner.reference_before_start = ""
            runner.event = mock.Mock()
            first_only = reference(
                "LoadPlayer-01", "LoadClient-01", client_id=1
            )
            second_only = reference(
                "LoadPlayer-02", "LoadClient-02", client_id=2
            )
            with (
                mock.patch.object(
                    MODULE,
                    "fetch_reference",
                    side_effect=[first_only, second_only],
                ),
                mock.patch.object(
                    MODULE.time,
                    "monotonic",
                    side_effect=[0.0, 0.1, 0.2, 0.3, 0.4, 1.1],
                ),
                mock.patch.object(MODULE.time, "sleep"),
            ):
                with self.assertRaisesRegex(
                    MODULE.FleetFailure, "current coexisting fleet"
                ):
                    runner.wait_for_player_admission()

    def test_admission_event_does_not_replace_global_elapsed_time(self):
        reference = (
            "[Reference]\r\n"
            "State=Lobby\r\n"
            "MaxPlayers=1\r\n"
            'Title="HarpoonRace"\r\n'
            "  [PlayerInfos]\r\n"
            "    [Client]\r\n"
            "    ID=1\r\n"
            "      [Player]\r\n"
            '      Name="LoadPlayer-01"\r\n'
            "      Flags=Joined\r\n"
            "  [Client]\r\n"
            "  ID=1\r\n"
            "  Activated=true\r\n"
            '  Name="LoadClient-01"\r\n'
        )
        with tempfile.TemporaryDirectory() as temporary:
            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.host = {
                "process": mock.Mock(poll=mock.Mock(return_value=None))
            }
            runner.arguments = SimpleNamespace(
                base_port=31_111,
                join_timeout=1.0,
                players=1,
            )
            runner.clients = [
                {
                    "index": 1,
                    "name": "LoadClient-01",
                    "client_name": "LoadClient-01",
                    "player_name": "LoadPlayer-01",
                    "launched_monotonic": 1.0,
                    "process": mock.Mock(poll=mock.Mock(return_value=None)),
                }
            ]
            runner.admission_samples = []
            runner.artifact_dir = Path(temporary)
            runner.event = mock.Mock()
            with (
                mock.patch.object(
                    MODULE, "fetch_reference", return_value=reference
                ),
                mock.patch.object(
                    MODULE.time,
                    "monotonic",
                    side_effect=[10.0, 10.1, 10.2],
                ),
            ):
                runner.wait_for_player_admission()

        event = runner.event.call_args
        self.assertEqual(event.args[0], "player-info-admitted")
        self.assertNotIn("elapsed_ms", event.kwargs)
        self.assertEqual(
            event.kwargs["startup_to_player_info_admission_ms"], 9_200.0
        )

    def test_grouped_player_admission_flattens_every_clients_profiles(self):
        reference = (
            "[Reference]\r\nState=Lobby\r\nMaxPlayers=3\r\n"
            'Title="HarpoonRace"\r\n'
            "  [PlayerInfos]\r\n"
            "    [Client]\r\n    ID=1\r\n"
            "      [Player]\r\n      Name=LoadPlayer-01\r\n      Flags=Joined\r\n"
            "      [Player]\r\n      Name=LoadPlayer-02\r\n      Flags=Joined\r\n"
            "    [Client]\r\n    ID=2\r\n"
            "      [Player]\r\n      Name=LoadPlayer-03\r\n      Flags=Joined\r\n"
            "  [Client]\r\n  ID=1\r\n  Activated=true\r\n  Name=LoadClient-01\r\n"
            "  [Client]\r\n  ID=2\r\n  Activated=true\r\n  Name=LoadClient-02\r\n"
        )
        with tempfile.TemporaryDirectory() as temporary:
            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.host = {"process": mock.Mock(poll=mock.Mock(return_value=None))}
            runner.arguments = SimpleNamespace(base_port=31_111, join_timeout=1.0, players=3)
            runner.clients = [
                {
                    "index": 1,
                    "name": "LoadClient-01",
                    "client_name": "LoadClient-01",
                    "player_indices": [1, 2],
                    "player_names": ["LoadPlayer-01", "LoadPlayer-02"],
                    "launched_monotonic": 1.0,
                    "process": mock.Mock(poll=mock.Mock(return_value=None)),
                },
                {
                    "index": 2,
                    "name": "LoadClient-02",
                    "client_name": "LoadClient-02",
                    "player_indices": [3],
                    "player_names": ["LoadPlayer-03"],
                    "launched_monotonic": 1.0,
                    "process": mock.Mock(poll=mock.Mock(return_value=None)),
                },
            ]
            runner.admission_samples = []
            runner.artifact_dir = Path(temporary)
            runner.event = mock.Mock()
            with mock.patch.object(MODULE, "fetch_reference", return_value=reference), mock.patch.object(
                MODULE.time, "monotonic", side_effect=[2.0, 2.1, 2.2]
            ):
                runner.wait_for_player_admission()

        self.assertEqual(
            [(sample["index"], sample["client_index"]) for sample in runner.admission_samples],
            [(1, 1), (2, 1), (3, 2)],
        )

    def test_joined_lobby_is_rechecked_immediately_before_start(self):
        one_player_reference = (
            "[Reference]\r\n"
            "State=Lobby\r\n"
            "MaxPlayers=2\r\n"
            'Title="HarpoonRace"\r\n'
            "  [PlayerInfos]\r\n"
            "    [Client]\r\n"
            "    ID=1\r\n"
            "      [Player]\r\n"
            '      Name="LoadPlayer-01"\r\n'
            "      Flags=Joined\r\n"
            "  [Client]\r\n"
            "  ID=1\r\n"
            "  Activated=true\r\n"
            '  Name="LoadClient-01"\r\n'
        )
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.host = {
            "process": mock.Mock(poll=mock.Mock(return_value=None))
        }
        runner.arguments = SimpleNamespace(
            base_port=31_111,
            settle_seconds=0.0,
            players=2,
        )
        runner.clients = [
            {
                "name": "LoadClient-01",
                "client_name": "LoadClient-01",
                "player_name": "LoadPlayer-01",
                "process": mock.Mock(poll=mock.Mock(return_value=None)),
            },
            {
                "name": "LoadClient-02",
                "client_name": "LoadClient-02",
                "player_name": "LoadPlayer-02",
                "process": mock.Mock(poll=mock.Mock(return_value=None)),
            },
        ]
        runner.event = mock.Mock()

        with mock.patch.object(
            MODULE, "fetch_reference", return_value=one_player_reference
        ):
            with self.assertRaisesRegex(
                MODULE.FleetFailure, "current coexisting fleet"
            ):
                runner.settle_joined_lobby()


class IsolatedInputTests(unittest.TestCase):
    def test_twenty_four_profiles_are_balanced_across_twelve_clients(self):
        groups = MODULE.distribute_player_indices(24, 12)
        self.assertEqual(len(groups), 12)
        self.assertTrue(all(len(group) == 2 for group in groups))
        self.assertEqual(groups[0], [1, 2])
        self.assertEqual(groups[-1], [23, 24])

        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.binary = Path("/clonk-app")
        command = runner.client_command(
            index=1,
            config=Path("/client/config.ini"),
            profiles=[
                Path("/profiles/LoadPlayer-01.c4p"),
                Path("/profiles/LoadPlayer-02.c4p"),
            ],
            tcp_port=31_121,
            udp_port=31_122,
            reference_port=31_111,
        )
        self.assertEqual(
            command[5:7],
            [
                "/profiles/LoadPlayer-01.c4p",
                "/profiles/LoadPlayer-02.c4p",
            ],
        )
        self.assertEqual(command[7], "/join:127.0.0.1:31111")

    def test_client_process_count_is_distinct_from_player_count(self):
        arguments = MODULE.build_argument_parser().parse_args(
            ["--players", "24", "--clients", "12", "--base-port", "31111"]
        )
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.arguments = arguments

        ports = runner.port_plan()

        self.assertEqual(len(ports["clients"]), 12)
        self.assertEqual(ports["clients"][-1]["index"], 12)

    def test_scratch_defaults_to_the_platform_temp_directory(self):
        with mock.patch.object(
            MODULE.platform, "system", return_value="Darwin"
        ):
            arguments = MODULE.build_argument_parser().parse_args([])

        self.assertIsNone(arguments.scratch_root)

    def test_controlled_environment_ignores_parent_log_and_config_overrides(self):
        environment = MODULE.controlled_process_environment(
            {
                "LC_LOG": "trace",
                "RUST_LOG": "wgpu_core=trace",
                "LC_CONFIG_FILE": "/tmp/not-the-fleet-config.ini",
                "LC_RUST_ENGINE_RANDOM_SEED": "991",
                "LC_RUST_ENGINE_MAP_SEED": "992",
                "LC_RUST_ENGINE_STARTUP_PLAYERS": "99",
                "UNRELATED": "kept",
            }
        )
        self.assertEqual(environment["LC_LOG"], MODULE.FLEET_LOG_FILTER)
        self.assertNotIn("RUST_LOG", environment)
        self.assertNotIn("LC_CONFIG_FILE", environment)
        self.assertNotIn("LC_RUST_ENGINE_RANDOM_SEED", environment)
        self.assertNotIn("LC_RUST_ENGINE_MAP_SEED", environment)
        self.assertNotIn("LC_RUST_ENGINE_STARTUP_PLAYERS", environment)
        self.assertEqual(environment["UNRELATED"], "kept")

    def test_client_benchmark_keeps_running_and_enables_its_input_probe(self):
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.workspace = Path("/tmp/harpoonrace-workspace")
        runner.arguments = SimpleNamespace(
            measurement_seconds=60,
            input_probe_interval_ms=500,
        )

        environment = runner.process_environment(
            Path("/tmp/harpoonrace-client"),
            Path("/tmp/harpoonrace-artifact"),
            benchmark=True,
        )

        self.assertEqual(
            environment["LC_APP_PRESENTATION_BENCHMARK_KEEP_RUNNING"], "1"
        )
        self.assertNotIn(
            "LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK", environment
        )
        self.assertEqual(
            environment["LC_APP_PRESENTATION_BENCHMARK_INPUT_INTERVAL_MS"], "500"
        )
        self.assertEqual(environment["LC_GAME_UPDATE_RECOVERY_COMPLETE"], "1")

    def test_generated_config_and_profile_are_stable_and_loadable_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / "config.ini"
            MODULE.write_process_config(
                config,
                name="LoadClient-01",
                tcp_port=31_121,
                udp_port=31_122,
                reference_port=31_111,
                control_mode=1,
                width=800,
                height=600,
            )
            text = config.read_text(encoding="utf-8")
            self.assertIn("ConfigResetSafety=42\n", text)
            self.assertIn("Language=US\nLanguageEx=US\n", text)
            self.assertIn(
                "[Network]\n"
                "LocalName=LoadClient-01\n"
                "Nick=LoadClient-01\n",
                text,
            )
            self.assertIn("PortTCP=31121\n", text)
            self.assertIn("PortUDP=31122\n", text)
            self.assertIn("ControlMode=1\n", text)

            profile = root / "LoadPlayer-01.c4p"
            big_icon = root / "source-icon.png"
            big_icon.write_bytes(b"\x89PNG\r\nbenchmark-icon")
            MODULE.write_distinct_profile(
                profile,
                "LoadPlayer-01",
                index=1,
                count=24,
                big_icon=big_icon,
            )
            self.assertTrue(profile.is_dir())
            self.assertEqual(
                (profile / "BigIcon.png").read_bytes(),
                big_icon.read_bytes(),
            )
            player_text = (profile / "Player.txt").read_text(encoding="ascii")
            self.assertIn("[Player]\nName=LoadPlayer-01\n", player_text)
            # C++ StdColors.h:52 stores player colors as 0xRRGGBB.
            self.assertIn("ColorDw=16724530\n", player_text)


class StatisticsTests(unittest.TestCase):
    def test_nearest_rank_percentiles_are_stable_and_finite(self):
        samples = [5.0, 1.0, 3.0, 2.0, 4.0]
        self.assertEqual(MODULE.nearest_rank_percentile(samples, 0.50), 3.0)
        self.assertEqual(MODULE.nearest_rank_percentile(samples, 0.95), 5.0)
        self.assertTrue(
            math.isfinite(MODULE.nearest_rank_percentile(samples, 0.99))
        )

    def test_event_json_fallback_serializes_paths_only(self):
        encoded = json.dumps(
            {"artifact": Path("/tmp/results")},
            default=MODULE.json_fallback,
        )
        self.assertEqual(encoded, '{"artifact": "/tmp/results"}')
        with self.assertRaises(TypeError):
            MODULE.json_fallback(object())

    def test_go_timestamp_requires_an_exact_timestamped_info_marker(self):
        text = (
            "2026-07-24T08:05:45.380394Z  INFO Go!\n"
            "2026-07-24T08:05:46.000000Z  INFO Go! later\n"
        )

        self.assertEqual(
            MODULE.extract_go_log_timestamp(text),
            "2026-07-24T08:05:45.380394Z",
        )
        self.assertIsNone(MODULE.extract_go_log_timestamp("INFO Go!\n"))

    def test_event_rejects_fields_that_would_corrupt_timeline_keys(self):
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)

        with self.assertRaisesRegex(ValueError, "elapsed_ms"):
            runner.event("player-info-admitted", elapsed_ms=9_200.0)


class LogEvidenceTests(unittest.TestCase):
    def test_retained_texture_info_spam_is_counted_and_rejected_specifically(self):
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "stderr.log"
            log.write_text(
                "2026-07-23 INFO ordinary application message\n"
                "2026-07-23 INFO Created texture label=unrelated\n"
                "2026-07-23 INFO Created texture "
                'label=Some("lc_gpu_retained_source")\n',
                encoding="utf-8",
            )

            statistics = MODULE.file_log_statistics(log)

            self.assertEqual(statistics["lines"], 3)
            self.assertEqual(statistics["created_texture_info_lines"], 2)
            self.assertEqual(
                statistics["retained_texture_creation_info_lines"], 1
            )
            self.assertEqual(
                MODULE.retained_texture_log_failures([statistics]),
                [
                    "retained GPU texture creation INFO spam was emitted "
                    "1 time(s)"
                ],
            )

    def test_ordinary_info_and_unrelated_texture_logs_remain_accepted(self):
        statistics = {
            "created_texture_info_lines": 4,
            "retained_texture_creation_info_lines": 0,
        }

        self.assertEqual(
            MODULE.retained_texture_log_failures([statistics]), []
        )

    def test_log_summary_retains_volume_and_spam_totals(self):
        statistics = MODULE.summarize_log_statistics(
            [
                {
                    "exists": True,
                    "bytes": 100,
                    "lines": 3,
                    "created_texture_info_lines": 2,
                    "retained_texture_creation_info_lines": 1,
                    "retained_source_mentions": 1,
                },
                {
                    "exists": False,
                    "bytes": 0,
                    "lines": 0,
                    "created_texture_info_lines": 0,
                    "retained_texture_creation_info_lines": 0,
                    "retained_source_mentions": 0,
                },
            ]
        )

        self.assertEqual(
            statistics,
            {
                "files": 2,
                "existing_files": 1,
                "bytes": 100,
                "lines": 3,
                "created_texture_info_lines": 2,
                "retained_texture_creation_info_lines": 1,
                "retained_source_mentions": 1,
            },
        )


class ProcessLifecycleTests(unittest.TestCase):
    def test_collect_results_rejects_overlapping_input_owners_across_clients(self):
        def input_metrics(player=None):
            metrics = {
                "elapsed_seconds": 1.0,
                "submitted_inputs": 2,
                "executed_inputs": 2,
                "pending_inputs": 0,
                "input_latency_sample_count": 2,
                "input_latency_p50_ms": 10.0,
                "input_latency_p95_ms": 10.0,
                "input_latency_p99_ms": 10.0,
                "input_latency_max_ms": 10.0,
                "input_latency_samples_ns": [10_000_000, 10_000_000],
            }
            return metrics if player is None else {"player": player, **metrics}

        def report(local_client_id):
            player_reports = [input_metrics(0), input_metrics(1)]
            return {
                "elapsed_seconds": 1.0,
                "successful_present_submissions": 0,
                "presentation_submission_fps": 0.0,
                "refreshed_frames": 0,
                "simulation_frames": 38,
                "simulation_fps": 38.0,
                "automatic_graphics_skips": 0,
                "average_graphics_pass_ms": 0.0,
                "max_graphics_pass_ms": 0.0,
                "graphics_pass_sample_count": 0,
                "graphics_pass_p50_ms": 0.0,
                "graphics_pass_p95_ms": 0.0,
                "graphics_pass_p99_ms": 0.0,
                "graphics_pass_samples_ns": [],
                "benchmark_context": {
                    "runtime_players": 4,
                    "synchronized_player_infos": 4,
                    "activated_nonhost_clients": 2,
                    "runtime_crew_objects": 4,
                    "runtime_players_with_live_crew": 4,
                    "runtime_players_with_exactly_one_live_sf5b_crew": 0,
                },
                "network_evidence": benchmark_network_evidence(
                    local_client_id=local_client_id,
                    players=2,
                ),
                "input_probe": {
                    **input_metrics(),
                    "submitted_inputs": 4,
                    "executed_inputs": 4,
                    "input_latency_sample_count": 4,
                    "input_latency_samples_ns": [10_000_000] * 4,
                },
                "input_probe_players": player_reports,
            }

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clients = []
            for client_index, player_indices in ((1, [1, 2]), (2, [3, 4])):
                stdout = root / f"client-{client_index:02d}.stdout"
                stderr = root / f"client-{client_index:02d}.stderr"
                stdout.write_text("", encoding="utf-8")
                stderr.write_text("", encoding="utf-8")
                clients.append(
                    {
                        "index": client_index,
                        "name": f"LoadClient-{client_index:02d}",
                        "client_name": f"LoadClient-{client_index:02d}",
                        "player_indices": player_indices,
                        "player_names": [
                            f"LoadPlayer-{player_index:02d}"
                            for player_index in player_indices
                        ],
                        "stdout_path": stdout,
                        "stderr_path": stderr,
                        "session_log_path": root / f"client-{client_index:02d}.log",
                        "all_report_barrier_observed": True,
                        "exit_code": 0,
                        "process": SimpleNamespace(pid=100 + client_index),
                        "supervisor_terminated": False,
                        "command": [f"client-{client_index:02d}"],
                    }
                )

            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.arguments = SimpleNamespace(
                players=4,
                clients=2,
                measurement_seconds=1,
                minimum_simulation_fps=38.0,
                minimum_presentation_fps=35.0,
                maximum_graphics_p99_ms=25.0,
                maximum_network_lag_ms=100.0,
                skip_sf5b_crew_assertion=True,
                runtime_only=True,
                input_probe_interval_ms=500,
                maximum_input_latency_ms=100.0,
                minimum_input_success_percent=95.0,
                control_mode=2,
            )
            runner.clients = clients
            runner.host = None
            runner.failures = []
            runner.artifact_dir = root
            runner.admission_samples = []
            runner.started_utc = "2026-08-09T00:00:00+00:00"
            runner.started_monotonic = MODULE.time.monotonic()

            with mock.patch.object(
                MODULE,
                "extract_benchmark_report",
                side_effect=[report(1), report(2)],
            ):
                result = runner.collect_results()

        self.assertEqual(result["result"], "fail")
        self.assertTrue(
            any(
                "input player owners are not the exact runtime fleet" in failure
                for failure in result["failures"]
            )
        )

    def test_input_fingerprint_rejects_same_name_source_and_parent_content_byte_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            source = workspace / "crates" / "app" / "src" / "main.rs"
            source.parent.mkdir(parents=True)
            source.write_text("first\n", encoding="utf-8")
            before = MODULE.scoped_paths_fingerprint(workspace, ("crates",))

            # The dirty path/status name is unchanged; only its bytes move.
            source.write_text("other\n", encoding="utf-8")
            after = MODULE.scoped_paths_fingerprint(workspace, ("crates",))

            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.artifact_dir = workspace / "artifacts"
            runner.artifact_dir.mkdir()
            runner.failures = []
            runner.initial_input_fingerprint = {
                "full_sha256": before["sha256"],
            }
            with mock.patch.object(
                runner,
                "capture_input_fingerprint",
                return_value={"full_sha256": after["sha256"]},
            ):
                runner.verify_input_fingerprint_invariance()

            self.assertNotEqual(before["sha256"], after["sha256"])
            self.assertEqual(
                runner.failures,
                ["benchmark input fingerprint changed during the run"],
            )

            scenario = workspace / "content" / "Hazard.c4f" / "DM_Baldoon.c4s"
            scenario.mkdir(parents=True)
            (scenario / "Scenario.txt").write_text("[Head]\n", encoding="ascii")
            material = scenario.parent / "Material.c4g" / "TexMap.txt"
            material.parent.mkdir()
            material.write_text("first\n", encoding="ascii")
            for runtime_group in (
                workspace / "planet" / "System.c4g",
                workspace / "content" / "Objects.c4d",
                workspace / "content" / "Hazard.c4d",
            ):
                runtime_group.mkdir(parents=True)
                (runtime_group / "Names.txt").write_text("stable\n", encoding="ascii")
            graphics = workspace / "planet" / "Graphics.c4g" / "Graphics.txt"
            graphics.parent.mkdir()
            graphics.write_text("first\n", encoding="ascii")
            binary = workspace / "clonk-app"
            binary.write_bytes(b"binary")
            profile_icon = workspace / "BigIcon.png"
            profile_icon.write_bytes(b"icon")
            runner.workspace = workspace
            runner.binary = binary
            runner.scenario = scenario
            runner.profile_big_icon = profile_icon

            content_before = runner.capture_input_fingerprint()
            material.write_text("other\n", encoding="ascii")
            content_after = runner.capture_input_fingerprint()

            self.assertNotEqual(
                content_before["matrix_invariant_sha256"],
                content_after["matrix_invariant_sha256"],
            )

            graphics_before = content_after
            graphics.write_text("other\n", encoding="ascii")
            graphics_after = runner.capture_input_fingerprint()

            self.assertNotEqual(
                graphics_before["matrix_invariant_sha256"],
                graphics_after["matrix_invariant_sha256"],
            )

    def test_workspace_status_probe_failure_fails_closed(self):
        workspace = Path("/tmp/harpoonrace-status-probe")
        failure = MODULE.subprocess.CalledProcessError(
            128,
            ["git", "status", "--porcelain=v1"],
            stderr="fatal: expected submodule path 'content' not to be a symbolic link",
        )

        with mock.patch.object(MODULE.subprocess, "run", side_effect=failure):
            with self.assertRaisesRegex(
                MODULE.FleetFailure,
                "workspace status capture failed",
            ):
                MODULE.workspace_status_porcelain(workspace)

    def test_content_status_probe_failure_fails_closed(self):
        content = Path("/tmp/harpoonrace-content-status-probe")
        failure = MODULE.subprocess.CalledProcessError(
            128,
            ["git", "status", "--porcelain=v1"],
            stderr="fatal: content repository is unavailable",
        )

        with mock.patch.object(MODULE.subprocess, "run", side_effect=failure):
            with self.assertRaisesRegex(
                MODULE.FleetFailure,
                "content status capture failed",
            ):
                MODULE.content_status_porcelain(content)

    def test_completion_deadline_includes_the_app_warmup(self):
        self.assertEqual(
            MODULE.benchmark_completion_timeout_seconds(60, 300.0),
            362.0,
        )

    def test_cleanup_marks_complete_only_after_process_waits_finish(self):
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.cleaned = False
        runner.cleanup_in_progress = False
        runner.arguments = SimpleNamespace(keep_scratch=True)
        runner.scratch = Path("/tmp/unused-harpoonrace-cleanup-test")
        runner.events_file = None
        runner.graceful_host_shutdown = mock.Mock()
        runner.close_output_handles = mock.Mock()
        runner.observe_go_logs = mock.Mock()

        def waited():
            self.assertFalse(runner.cleaned)

        runner.terminate_recorded_processes = mock.Mock(side_effect=waited)
        runner.cleanup()

        runner.terminate_recorded_processes.assert_called_once_with()
        self.assertTrue(runner.cleaned)
        self.assertFalse(runner.cleanup_in_progress)

    def test_cleanup_retries_child_wait_after_interrupt_before_reraising(self):
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.cleaned = False
        runner.cleanup_in_progress = False
        runner.arguments = SimpleNamespace(keep_scratch=True)
        runner.scratch = Path("/tmp/unused-harpoonrace-cleanup-test")
        runner.events_file = None
        runner.graceful_host_shutdown = mock.Mock()
        runner.close_output_handles = mock.Mock()
        runner.observe_go_logs = mock.Mock()
        runner.terminate_recorded_processes = mock.Mock(
            side_effect=[KeyboardInterrupt(), KeyboardInterrupt(), None]
        )

        with self.assertRaises(KeyboardInterrupt):
            runner.cleanup()

        self.assertEqual(runner.terminate_recorded_processes.call_count, 3)
        self.assertTrue(runner.cleaned)
        self.assertFalse(runner.cleanup_in_progress)

    def test_report_progress_event_names_what_is_actually_observed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stdout = root / "stdout.log"
            stderr = root / "stderr.log"
            stdout.write_text("", encoding="utf-8")
            stderr.write_text("", encoding="utf-8")
            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.arguments = SimpleNamespace(
                measurement_seconds=20,
                completion_grace_seconds=0.0,
            )
            runner.host = {
                "process": mock.Mock(poll=mock.Mock(return_value=None))
            }
            runner.clients = [
                {
                    "name": "LoadClient-01",
                    "benchmark_report_observed": False,
                    "process": mock.Mock(poll=mock.Mock(return_value=None)),
                    "stdout_path": stdout,
                    "stderr_path": stderr,
                }
            ]
            runner.processes = []
            runner.failures = []
            runner.event = mock.Mock()
            with (
                mock.patch.object(
                    MODULE.time,
                    "monotonic",
                    side_effect=[0.0, 0.0, 1.0, 11.0, 23.0],
                ),
                mock.patch.object(MODULE.time, "sleep"),
            ):
                runner.wait_for_clients()

        event_names = [call.args[0] for call in runner.event.call_args_list]
        self.assertIn("report-completion-progress", event_names)
        self.assertNotIn("benchmark-progress", event_names)

    def test_timing_artifact_keeps_unobservable_instants_null(self):
        metric = (
            "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=5 "
            "successful_present_submissions=1 presentation_submission_fps=1 "
            "refreshed_frames=1 simulation_frames=1 simulation_fps=1 "
            "automatic_graphics_skips=0 average_graphics_pass_ms=1 "
            "max_graphics_pass_ms=1 graphics_pass_sample_count=1 "
            "graphics_pass_p50_ms=1 graphics_pass_p95_ms=1 "
            "graphics_pass_p99_ms=1 graphics_pass_samples_ns=[1000000]\n"
            "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=1 "
            "synchronized_player_infos=1 activated_nonhost_clients=1 "
            "runtime_crew_objects=1 "
            "runtime_players_with_live_crew=1 "
            "runtime_players_with_exactly_one_live_sf5b_crew=1\n"
        ) + benchmark_network_line(local_client_id=1, players=1) + "\n"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            client_root = root / "client-01"
            host_root = root / "host"
            client_root.mkdir()
            host_root.mkdir()
            stdout = client_root / "stdout.log"
            stderr = client_root / "stderr.log"
            host_stderr = host_root / "stderr.log"
            stdout.write_text(metric, encoding="utf-8")
            stderr.write_text(
                "2026-07-24T08:05:45.380394Z  INFO Go!\n",
                encoding="utf-8",
            )
            host_stderr.write_text("", encoding="utf-8")
            client = {
                "index": 1,
                "name": "LoadClient-01",
                "stdout_path": stdout,
                "stderr_path": stderr,
                "session_log_path": client_root / "missing.log",
                "go_observation": None,
                "benchmark_report_observation": {
                    "observed_at_utc": "2026-07-24T08:05:52+00:00",
                    "observed_elapsed_ms": 50_000.0,
                    "stdout_file_modified_at_utc": (
                        "2026-07-24T08:05:51+00:00"
                    ),
                },
            }
            host = {
                "name": "host",
                "stderr_path": host_stderr,
                "session_log_path": host_root / "missing.log",
                "go_observation": None,
            }
            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.artifact_dir = root
            runner.clients = [client]
            runner.host = host
            runner.processes = [host, client]
            runner.go_command_sent = {
                "command": "/start 0",
                "sent_at_utc": "2026-07-24T08:05:30+00:00",
                "sent_elapsed_ms": 28_000.0,
                "source": "supervisor console write and flush",
            }
            runner.event = mock.Mock(
                return_value={
                    "timestamp_utc": "2026-07-24T08:05:46+00:00",
                    "elapsed_ms": 44_000.0,
                }
            )

            runner.observe_go_logs()
            runner.write_benchmark_timing()
            timing = json.loads(
                (root / "benchmark-timing.json").read_text(encoding="utf-8")
            )

        observed = timing["clients"][0]
        self.assertEqual(
            observed["go_observation"]["source_timestamp_utc"],
            "2026-07-24T08:05:45.380394Z",
        )
        self.assertIsNone(observed["warmup"]["started_at_utc"])
        self.assertIsNone(observed["measurement"]["started_at_utc"])
        self.assertIsNone(observed["measurement"]["finished_at_utc"])
        self.assertEqual(
            observed["measurement"]["reported_elapsed_seconds"], 5.0
        )
        self.assertIsNone(observed["report"]["emitted_at_utc"])
        self.assertEqual(
            observed["report"]["observed_at_utc"],
            "2026-07-24T08:05:52+00:00",
        )

    def test_complete_raw_reports_are_counted_before_acceptance(self):
        metric = (
            "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=1 "
            "successful_present_submissions=35 "
            "presentation_submission_fps=35 refreshed_frames=35 "
            "simulation_frames=35 simulation_fps=35 "
            "automatic_graphics_skips=0 average_graphics_pass_ms=1 "
            "max_graphics_pass_ms=1 graphics_pass_sample_count=1 "
            "graphics_pass_p50_ms=1 graphics_pass_p95_ms=1 "
            "graphics_pass_p99_ms=1 graphics_pass_samples_ns=[1000000]\n"
            "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=2 "
            "synchronized_player_infos=2 activated_nonhost_clients=2 "
            "runtime_crew_objects=2 "
            "runtime_players_with_live_crew=2 "
            "runtime_players_with_exactly_one_live_sf5b_crew=2\n"
        ) + benchmark_network_line(local_client_id=1, players=2) + "\n"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stdout_paths = [root / "client-01.log", root / "client-02.log"]
            stdout_paths[0].write_text(metric, encoding="utf-8")
            stdout_paths[1].write_text(
                metric
                + "LC_APP_PRESENTATION_BENCHMARK result=fail "
                "error=automatic graphics skips must be zero (observed 1)\n",
                encoding="utf-8",
            )
            stderr_paths = [root / "client-01.err", root / "client-02.err"]
            for path in stderr_paths:
                path.write_text("", encoding="utf-8")

            runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
            runner.arguments = SimpleNamespace(
                measurement_seconds=1,
                completion_grace_seconds=1.0,
            )
            runner.host = {
                "process": mock.Mock(poll=mock.Mock(return_value=None))
            }
            runner.clients = [
                {
                    "name": "LoadClient-01",
                    "benchmark_report_observed": False,
                    "all_report_barrier_observed": False,
                    "process": mock.Mock(
                        poll=mock.Mock(return_value=None)
                    ),
                    "stdout_path": stdout_paths[0],
                    "stderr_path": stderr_paths[0],
                },
                {
                    "name": "LoadClient-02",
                    "benchmark_report_observed": False,
                    "all_report_barrier_observed": False,
                    "process": mock.Mock(poll=mock.Mock(return_value=2)),
                    "stdout_path": stdout_paths[1],
                    "stderr_path": stderr_paths[1],
                },
            ]
            runner.failures = []
            runner.event = mock.Mock(
                return_value={
                    "timestamp_utc": "2026-07-24T08:05:52+00:00",
                    "elapsed_ms": 50_000.0,
                }
            )

            runner.wait_for_clients()

        self.assertTrue(
            all(
                client["benchmark_report_observed"]
                for client in runner.clients
            )
        )
        self.assertFalse(
            any("completion timeout" in failure for failure in runner.failures)
        )
        self.assertEqual(
            runner.failures,
            [
                "clients disconnected before the all-report barrier: "
                "LoadClient-02=2"
            ],
        )

    def test_all_reporting_clients_must_still_be_connected(self):
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.arguments = SimpleNamespace(
            measurement_seconds=1,
            completion_grace_seconds=1.0,
        )
        runner.host = {
            "process": mock.Mock(poll=mock.Mock(return_value=None))
        }
        runner.clients = [
            {
                "name": "LoadClient-01",
                "benchmark_report_observed": True,
                "process": mock.Mock(poll=mock.Mock(return_value=None)),
            },
            {
                "name": "LoadClient-02",
                "benchmark_report_observed": True,
                "process": mock.Mock(poll=mock.Mock(return_value=0)),
            },
        ]
        runner.failures = []
        runner.event = mock.Mock()

        runner.wait_for_clients()

        self.assertEqual(
            runner.failures,
            [
                "clients disconnected before the all-report barrier: "
                "LoadClient-02=0"
            ],
        )


if __name__ == "__main__":
    unittest.main()
