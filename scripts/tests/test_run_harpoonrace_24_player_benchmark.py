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


def benchmark_network_line(local_client_id=1, players=24, lag_ms=12):
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
        "max_packet_loss=0 control_presend=4 "
        "avg_control_send_time_us=26813"
    )


def benchmark_network_evidence(local_client_id=1, players=24, lag_ms=12):
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
        "max_nonnegative_lag_ms=12 max_packet_loss=3 control_presend=4 "
        "avg_control_send_time_us=26813"
    )

    def test_parses_exact_preferred_message_route_network_evidence(self):
        evidence = MODULE.parse_benchmark_network_line(self.NETWORK_LINE)

        self.assertEqual(evidence["local_client_id"], 1)
        self.assertEqual(evidence["preferred_message_route_peer_ids"], [0, 2])
        self.assertEqual(evidence["preferred_message_route_peer_count"], 2)
        self.assertEqual(evidence["tcp_preferred_message_routes"], 1)
        self.assertEqual(evidence["udp_preferred_message_routes"], 1)
        self.assertEqual(evidence["max_nonnegative_lag_ms"], 12)
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

    def test_context_requires_the_per_player_live_sf5b_gate(self):
        # HarpoonRace creates one SF5B and adds it to each C4Player::Crew
        # (HarpoonRace.c4s/Script.c:66-73; src/C4Player.cpp:1173-1202).
        context = (
            "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=24 "
            "synchronized_player_infos=24 activated_nonhost_clients=24 "
            "runtime_crew_objects=24"
        )

        with self.assertRaisesRegex(
            ValueError,
            "runtime_players_with_exactly_one_live_sf5b_crew",
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
    def test_controlled_environment_ignores_parent_log_and_config_overrides(self):
        environment = MODULE.controlled_process_environment(
            {
                "LC_LOG": "trace",
                "RUST_LOG": "wgpu_core=trace",
                "LC_CONFIG_FILE": "/tmp/not-the-fleet-config.ini",
                "UNRELATED": "kept",
            }
        )
        self.assertEqual(environment["LC_LOG"], MODULE.FLEET_LOG_FILTER)
        self.assertNotIn("RUST_LOG", environment)
        self.assertNotIn("LC_CONFIG_FILE", environment)
        self.assertEqual(environment["UNRELATED"], "kept")

    def test_client_benchmark_keeps_running_for_supervisor_acceptance(self):
        runner = MODULE.FleetRunner.__new__(MODULE.FleetRunner)
        runner.workspace = Path("/tmp/harpoonrace-workspace")
        runner.arguments = SimpleNamespace(measurement_seconds=60)

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
                width=800,
                height=600,
            )
            text = config.read_text(encoding="utf-8")
            self.assertIn("ConfigResetSafety=42\n", text)
            self.assertIn(
                "[Network]\n"
                "LocalName=LoadClient-01\n"
                "Nick=LoadClient-01\n",
                text,
            )
            self.assertIn("PortTCP=31121\n", text)
            self.assertIn("PortUDP=31122\n", text)

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
