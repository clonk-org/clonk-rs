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
        "max_graphics_pass_ms=9.000000 graphics_pass_sample_count=1 "
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

    def test_preserves_every_raw_graphics_pass_sample(self):
        line = self.BENCHMARK_LINE.replace(
            "graphics_pass_sample_count=1",
            "graphics_pass_sample_count=3",
        ).replace(
            "graphics_pass_samples_ns=[5000000]",
            "graphics_pass_samples_ns=[5000000, 7000000, 9000000]",
        )

        report = MODULE.parse_presentation_line(line)

        self.assertEqual(
            report["graphics_pass_samples_ns"],
            [5_000_000, 7_000_000, 9_000_000],
        )

    def test_rejects_a_truncated_raw_graphics_pass_distribution(self):
        line = self.BENCHMARK_LINE.replace(
            "graphics_pass_sample_count=1",
            "graphics_pass_sample_count=2",
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "graphics pass sample count is 2 but 1 raw samples were reported",
        ):
            MODULE.parse_presentation_line(line)

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

    def test_paired_evidence_rejects_a_headless_zero_sample_baseline(self):
        headless = (
            self.BENCHMARK_LINE.replace(
                "successful_present_submissions=1200",
                "successful_present_submissions=0",
            )
            .replace("refreshed_frames=1200", "refreshed_frames=0")
            .replace("graphics_pass_sample_count=1", "graphics_pass_sample_count=0")
            .replace("graphics_pass_samples_ns=[5000000]", "graphics_pass_samples_ns=[]")
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
    PRESENTATION_LINE = NativeCadenceTests.BENCHMARK_LINE.replace(
        "graphics_pass_sample_count=1",
        "graphics_pass_sample_count=3",
    ).replace(
        "graphics_pass_samples_ns=[5000000]",
        "graphics_pass_samples_ns=[5000000, 7000000, 9000000]",
    )
    CONTEXT_LINE = NativeCadenceTests.CONTEXT_LINE
    NETWORK_LINE = NativeCadenceTests.NETWORK_LINE

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
                        )
                    )
                    config.write_text(
                        f"saved by {Path(command[0]).name}\n",
                        encoding="utf-8",
                    )
                    result = "pass" if command[0] == str(candidate) else "fail"
                    lines = [
                        self.PRESENTATION_LINE,
                        self.CONTEXT_LINE,
                        self.NETWORK_LINE,
                        "LC_APP_PRESENTATION_BENCHMARK "
                        f"result={result} native_tick_budget_ms=28",
                    ]
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
        self.assertEqual(manifest["result"], "pass")
        self.assertEqual(manifest["runs"]["baseline"]["budget_result"], "fail")
        self.assertEqual(manifest["runs"]["candidate"]["budget_result"], "pass")
        self.assertEqual(
            manifest["runs"]["candidate"]["presentation"]["graphics_pass_samples_ns"],
            [5_000_000, 7_000_000, 9_000_000],
        )
        self.assertTrue(all(retained_artifacts.values()))


if __name__ == "__main__":
    unittest.main()
