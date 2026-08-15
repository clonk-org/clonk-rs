import hashlib
import importlib.util
import json
import os
import signal
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


SCRIPT = (
    Path(__file__).resolve().parents[1]
    / "run_hazard_24_player_gpu_benchmark.py"
)
SPEC = importlib.util.spec_from_file_location("hazard_24_player_gpu", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

ARSO_TEST_SCRIPT = (
    Path(__file__).resolve().parent
    / "test_run_arso_morf_stippel_gpu_benchmark.py"
)
ARSO_TEST_SPEC = importlib.util.spec_from_file_location(
    "arso_gpu_test_fixtures", ARSO_TEST_SCRIPT
)
assert ARSO_TEST_SPEC is not None and ARSO_TEST_SPEC.loader is not None
ARSO_TEST_MODULE = importlib.util.module_from_spec(ARSO_TEST_SPEC)
ARSO_TEST_SPEC.loader.exec_module(ARSO_TEST_MODULE)


class HazardCrewContractTests(unittest.TestCase):
    def test_accepts_four_hzck_player_templates_for_a_24_player_fleet(self):
        scenario_text = """\
[Head]
Title=DM - Baldoon
[Player1]
Crew=HZCK=1
[Player2]
Crew=HZCK=1
[Player3]
Crew=HZCK=1
[Player4]
Crew=HZCK=1
"""
        with tempfile.TemporaryDirectory() as temporary:
            scenario = Path(temporary) / "DM_Baldoon.c4s"
            scenario.mkdir()
            (scenario / "Scenario.txt").write_text(
                scenario_text, encoding="cp1252"
            )

            evidence = MODULE.validate_hzck_scenario_contract(scenario)

        self.assertEqual(evidence["crew_id"], "HZCK")
        self.assertEqual(
            evidence["player_sections"],
            ["Player1", "Player2", "Player3", "Player4"],
        )
        self.assertEqual(evidence["players_requested"], 24)
        self.assertEqual(
            evidence["scenario_file_fingerprint"],
            {
                "sha256": hashlib.sha256(
                    scenario_text.encode("cp1252")
                ).hexdigest(),
                "size_bytes": len(scenario_text.encode("cp1252")),
            },
        )
        self.assertFalse(evidence["runtime_reports_expose_crew_definition_ids"])

    def test_requires_every_visible_client_to_report_24_live_crew(self):
        clients = []
        for index in range(12):
            clients.append(
                {
                    "index": index + 1,
                    "client_name": f"client-{index + 1:02d}",
                    "report": {
                        "benchmark_context": {
                            "runtime_players": 24,
                            "synchronized_player_infos": 24,
                            "activated_nonhost_clients": 12,
                            "runtime_crew_objects": 24,
                            "runtime_players_with_live_crew": 24,
                        }
                    },
                }
            )
        with tempfile.TemporaryDirectory() as temporary:
            artifact_dir = Path(temporary)
            (artifact_dir / "presentation-raw.json").write_text(
                json.dumps({"schema_version": 1, "clients": clients}),
                encoding="utf-8",
            )

            evidence = MODULE.validate_hzck_runtime_evidence(
                artifact_dir, expected_clients=12
            )

            clients[7]["report"]["benchmark_context"][
                "runtime_crew_objects"
            ] = 23
            (artifact_dir / "presentation-raw.json").write_text(
                json.dumps({"schema_version": 1, "clients": clients}),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                MODULE.HazardBenchmarkFailure,
                "client-08.*runtime_crew_objects=23",
            ):
                MODULE.validate_hzck_runtime_evidence(
                    artifact_dir, expected_clients=12
                )

        self.assertEqual(evidence["clients_validated"], 12)
        self.assertEqual(evidence["runtime_players"], 24)
        self.assertEqual(evidence["runtime_crew_objects"], 24)


class RetainedGpuEvidenceTests(unittest.TestCase):
    def test_retains_candidate_profile_and_marks_unsupported_timestamps_unavailable(self):
        profile = ARSO_TEST_MODULE.retained_gpu_profile_v2()
        profile["frames"][0]["renderer"]["object_sprite_instances"] = 24
        profile["frames"][0]["renderer"]["object_sprite_upload_bytes"] = (
            24 * 88
        )
        report = {
            "graphics_pass_samples_ns": [5_000_000],
            "retained_gpu_present_submissions": 1,
        }
        with tempfile.TemporaryDirectory() as temporary:
            artifact_dir = Path(temporary)
            client_dir = artifact_dir / "client-01"
            client_dir.mkdir()
            (client_dir / "stdout.log").write_text(
                "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile) + "\n",
                encoding="utf-8",
            )
            (client_dir / "stderr.log").write_text("", encoding="utf-8")
            (artifact_dir / "presentation-raw.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "clients": [
                            {
                                "index": 1,
                                "client_name": "client-01",
                                "report": report,
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            evidence = MODULE.collect_retained_gpu_evidence(
                artifact_dir,
                expected_clients=1,
                profiles_required=True,
                expected_timestamp_request=True,
                minimum_schema_version=2,
            )

            retained = json.loads(
                (artifact_dir / "retained-gpu-evidence.json").read_text(
                    encoding="utf-8"
                )
            )

        self.assertEqual(evidence, retained)
        self.assertEqual(evidence["profiles_retained"], 1)
        self.assertEqual(
            evidence["timestamp_queries"],
            {
                "availability": "unavailable",
                "reason": "adapter_does_not_support_timestamp_queries",
                "requested": True,
                "supported": False,
                "enabled": False,
                "dropped_frames": 0,
                "readback_errors": 0,
                "device_discontinuities": 0,
                "gpu_pass_duration_ns": {},
                "gpu_pass_validity_counts": {},
            },
        )
        self.assertEqual(
            evidence["renderer_counters"]["object_sprite_instances"]["p50"],
            24.0,
        )
        self.assertEqual(
            evidence["renderer_counters"]["object_sprite_upload_bytes"]["p50"],
            2112.0,
        )
        self.assertEqual(
            evidence["raw_profiles"][0]["profile_sha256"],
            ARSO_TEST_MODULE.MODULE.json_sha256(profile),
        )

    def test_tolerant_raw_timestamps_count_validity_and_filter_distributions(self):
        profile = ARSO_TEST_MODULE.retained_gpu_profile_with_rollover()
        profile["schema_version"] = 2
        for frame in profile["frames"]:
            frame["renderer"]["landscape_instances"] = 0
            frame["renderer"]["landscape_instance_upload_bytes"] = 0
        report = {
            "graphics_pass_samples_ns": [5_000_000, 7_000_000],
            "retained_gpu_present_submissions": 2,
        }
        with tempfile.TemporaryDirectory() as temporary:
            artifact_dir = Path(temporary)
            client_dir = artifact_dir / "client-01"
            client_dir.mkdir()
            (client_dir / "stdout.log").write_text(
                "LC_APP_RETAINED_GPU_PROFILE " + json.dumps(profile) + "\n",
                encoding="utf-8",
            )
            (client_dir / "stderr.log").write_text("", encoding="utf-8")
            (artifact_dir / "presentation-raw.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "clients": [
                            {
                                "index": 1,
                                "client_name": "client-01",
                                "report": report,
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            evidence = MODULE.collect_retained_gpu_evidence(
                artifact_dir,
                expected_clients=1,
                profiles_required=True,
                expected_timestamp_request=True,
                minimum_schema_version=2,
            )

        timestamps = evidence["timestamp_queries"]
        self.assertEqual(timestamps["readback_errors"], 1)
        self.assertEqual(
            timestamps["gpu_pass_validity_counts"]["presentation"],
            {
                "valid": 1,
                "invalid_period": 0,
                "counter_rollover": 1,
                "invalid_duration": 0,
            },
        )
        self.assertEqual(
            timestamps["gpu_pass_duration_ns"]["presentation"][
                "sample_count"
            ],
            1,
        )
        self.assertEqual(
            timestamps["gpu_pass_duration_ns"]["scene"]["sample_count"], 2
        )
        self.assertEqual(evidence["raw_profiles"][0]["profile"], profile)

    def test_legacy_baseline_profile_is_optional_but_state_is_explicit(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifact_dir = Path(temporary)
            client_dir = artifact_dir / "client-01"
            client_dir.mkdir()
            (client_dir / "stdout.log").write_text("", encoding="utf-8")
            (client_dir / "stderr.log").write_text("", encoding="utf-8")
            (artifact_dir / "presentation-raw.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "clients": [
                            {
                                "index": 1,
                                "client_name": "client-01",
                                "report": {
                                    "graphics_pass_samples_ns": [5_000_000],
                                    "retained_gpu_present_submissions": 1,
                                },
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            evidence = MODULE.collect_retained_gpu_evidence(
                artifact_dir,
                expected_clients=1,
                profiles_required=False,
                expected_timestamp_request=False,
                minimum_schema_version=None,
            )

        self.assertEqual(evidence["profiles_retained"], 0)
        self.assertEqual(
            evidence["timestamp_queries"]["availability"], "not_emitted"
        )
        self.assertEqual(
            evidence["timestamp_queries"]["reason"],
            "optional_baseline_profile_not_emitted",
        )


class PairedCommandTests(unittest.TestCase):
    def test_builds_visible_24_player_hazard_arm_without_runtime_only_mode(self):
        arguments = MODULE.build_argument_parser().parse_args(
            [
                "--baseline-binary",
                "/baseline/clonk-app",
                "--candidate-binary",
                "/candidate/clonk-app",
                "--baseline-source-root",
                "/baseline",
                "--candidate-source-root",
                "/candidate",
            ]
        )

        command = MODULE.harpoon_arm_command(
            arguments,
            workspace=Path("/workspace"),
            binary=Path("/candidate/clonk-app"),
            artifact_dir=Path("/artifacts/candidate"),
            base_port=31_175,
        )

        self.assertEqual(command[command.index("--players") + 1], "24")
        self.assertEqual(command[command.index("--clients") + 1], "12")
        self.assertEqual(command[command.index("--control-mode") + 1], "2")
        self.assertEqual(
            command[command.index("--scenario") + 1],
            "/workspace/content/Hazard.c4f/DM_Baldoon.c4s",
        )
        self.assertEqual(
            command[command.index("--scenario-title") + 1], "DM - Baldoon"
        )
        self.assertEqual(command[command.index("--base-port") + 1], "31175")
        self.assertIn("--skip-sf5b-crew-assertion", command)
        self.assertNotIn("--runtime-only", command)
        self.assertIn("--minimum-presentation-fps", command)
        self.assertIn("--maximum-graphics-p99-ms", command)

    def test_arm_environment_makes_timestamp_request_explicit(self):
        inherited = {
            "PATH": "/usr/bin",
            "LC_GPU_TIMESTAMP_QUERIES": "ambient",
        }

        baseline = MODULE.arm_environment(
            inherited, timestamp_queries_requested=False
        )
        candidate = MODULE.arm_environment(
            inherited, timestamp_queries_requested=True
        )

        self.assertNotIn("LC_GPU_TIMESTAMP_QUERIES", baseline)
        self.assertEqual(candidate["LC_GPU_TIMESTAMP_QUERIES"], "1")
        self.assertEqual(baseline["PATH"], "/usr/bin")
        self.assertEqual(candidate["PATH"], "/usr/bin")
        self.assertEqual(inherited["LC_GPU_TIMESTAMP_QUERIES"], "ambient")


class FingerprintTests(unittest.TestCase):
    def test_profile_helper_provenance_failure_uses_the_harness_error(self):
        class ValidatorFailure(Exception):
            pass

        validator = SimpleNamespace(
            BenchmarkFailure=ValidatorFailure,
            collect_source_provenance=mock.Mock(
                side_effect=ValidatorFailure("git probe failed")
            ),
        )
        arguments = MODULE.build_argument_parser().parse_args(
            [
                "--baseline-binary",
                "/baseline/clonk-app",
                "--candidate-binary",
                "/candidate/clonk-app",
                "--baseline-source-root",
                "/baseline",
            ]
        )
        with mock.patch.object(
            MODULE, "_profile_validator_module", return_value=validator
        ), self.assertRaisesRegex(
            MODULE.HazardBenchmarkFailure, "git probe failed"
        ):
            MODULE.collect_run_provenance(arguments, workspace=Path("/workspace"))

    def test_pair_allows_only_binary_identity_to_differ(self):
        shared_invariant = {
            "workspace_commit": "abc",
            "workspace_source": {"sha256": "source"},
            "runner": {"sha256": "runner"},
            "content_revision": "content",
            "runtime_data": [{"tree_sha256": "runtime"}],
            "profile_big_icon": {"sha256": "icon"},
        }
        baseline = {
            "schema_version": 1,
            "full_sha256": "baseline-full",
            "matrix_invariant": {
                **shared_invariant,
                "binary": {"sha256": "baseline"},
            },
            "scenario": {"tree_sha256": "scenario"},
        }
        candidate = {
            "schema_version": 1,
            "full_sha256": "candidate-full",
            "matrix_invariant": {
                **shared_invariant,
                "binary": {"sha256": "candidate"},
            },
            "scenario": {"tree_sha256": "scenario"},
        }

        evidence = MODULE.validate_paired_input_fingerprints(
            baseline, candidate
        )

        self.assertNotEqual(
            evidence["baseline_full_sha256"],
            evidence["candidate_full_sha256"],
        )
        self.assertEqual(evidence["scenario_tree_sha256"], "scenario")
        self.assertEqual(evidence["binary_sha256"]["baseline"], "baseline")
        self.assertEqual(evidence["binary_sha256"]["candidate"], "candidate")

        candidate["scenario"]["tree_sha256"] = "changed"
        with self.assertRaisesRegex(
            MODULE.HazardBenchmarkFailure, "paired scenario fingerprint"
        ):
            MODULE.validate_paired_input_fingerprints(baseline, candidate)

    def test_child_input_fingerprint_must_remain_stable_through_the_arm(self):
        initial = {
            "schema_version": 1,
            "full_sha256": "initial",
            "matrix_invariant": {"binary": {"sha256": "binary"}},
            "scenario": {"tree_sha256": "scenario"},
        }
        with tempfile.TemporaryDirectory() as temporary:
            artifact_dir = Path(temporary)
            (artifact_dir / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "input_fingerprint": initial,
                        "topology": {"remote_player_profiles": 24},
                        "settings": {
                            "players": 24,
                            "clients": 12,
                            "runtime_only": False,
                        },
                    }
                ),
                encoding="utf-8",
            )
            (artifact_dir / "summary.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "result": "pass",
                        "players_requested": 24,
                        "clients_requested": 12,
                        "acceptance": {"presentation_required": True},
                    }
                ),
                encoding="utf-8",
            )
            (artifact_dir / "input-fingerprint-final.json").write_text(
                json.dumps({**initial, "full_sha256": "changed"}),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(
                MODULE.HazardBenchmarkFailure, "changed during the child arm"
            ):
                MODULE.load_child_arm_contract(
                    artifact_dir, expected_clients=12
                )

            (artifact_dir / "input-fingerprint-final.json").write_text(
                json.dumps(initial), encoding="utf-8"
            )
            contract = MODULE.load_child_arm_contract(
                artifact_dir, expected_clients=12
            )

        self.assertEqual(contract["input_fingerprint"], initial)
        self.assertEqual(contract["summary"]["result"], "pass")

    def test_gpu_pair_requires_matching_timestamp_device_features(self):
        baseline_profile = ARSO_TEST_MODULE.retained_gpu_profile_v2()
        ARSO_TEST_MODULE.enable_retained_gpu_timestamps(baseline_profile)
        candidate_profile = json.loads(json.dumps(baseline_profile))
        baseline = {
            "profiles_retained": 1,
            "fingerprint_sha256": "baseline-fingerprint",
            "raw_profiles": [{"profile": baseline_profile}],
        }
        candidate = {
            "profiles_retained": 1,
            "fingerprint_sha256": "candidate-fingerprint",
            "raw_profiles": [{"profile": candidate_profile}],
        }

        evidence = MODULE.validate_paired_gpu_fingerprints(
            baseline, candidate
        )

        self.assertEqual(evidence["comparison"], "matched")
        self.assertEqual(
            evidence["arm_fingerprint_sha256"],
            {
                "baseline": "baseline-fingerprint",
                "candidate": "candidate-fingerprint",
            },
        )

        candidate_profile["fingerprint"]["device"]["feature_bits"] = [0, 0]
        with self.assertRaisesRegex(
            MODULE.HazardBenchmarkFailure, "GPU fingerprints differ"
        ):
            MODULE.validate_paired_gpu_fingerprints(baseline, candidate)


class PairedRunnerTests(unittest.TestCase):
    def test_interrupted_owned_child_is_terminated_and_reaped(self):
        process = mock.Mock()
        process.poll.return_value = None
        process.wait.side_effect = [KeyboardInterrupt, 0]
        with mock.patch.object(
            MODULE.subprocess, "Popen", return_value=process
        ) as popen, self.assertRaises(KeyboardInterrupt):
            MODULE.run_owned_child(["harpoon", "--visible"], {"MODE": "ab"})

        popen.assert_called_once_with(
            ["harpoon", "--visible"],
            env={"MODE": "ab"},
            start_new_session=True,
        )
        process.terminate.assert_called_once_with()
        self.assertEqual(process.wait.call_count, 2)

    def test_termination_signal_enters_the_owned_child_cleanup_path(self):
        with self.assertRaises(KeyboardInterrupt):
            MODULE.handle_termination_signal(None, None)

    @unittest.skipUnless(os.name == "posix", "POSIX process-group cleanup")
    def test_unresponsive_owned_child_process_group_is_killed_and_reaped(self):
        process = mock.Mock(pid=4242)
        process.poll.return_value = None
        process.wait.side_effect = [
            KeyboardInterrupt,
            subprocess.TimeoutExpired("harpoon", 90),
            0,
        ]
        with mock.patch.object(
            MODULE.subprocess, "Popen", return_value=process
        ), mock.patch.object(MODULE.os, "killpg") as killpg, self.assertRaises(
            KeyboardInterrupt
        ):
            MODULE.run_owned_child(["harpoon"], {})

        killpg.assert_called_once_with(4242, signal.SIGKILL)
        self.assertEqual(process.wait.call_count, 3)

    @unittest.skipUnless(os.name == "posix", "POSIX process-group cleanup")
    def test_repeated_interrupt_still_kills_and_reaps_the_owned_group(self):
        process = mock.Mock(pid=4343)
        process.poll.return_value = None
        process.wait.side_effect = [KeyboardInterrupt, KeyboardInterrupt, 0]
        with mock.patch.object(
            MODULE.subprocess, "Popen", return_value=process
        ), mock.patch.object(MODULE.os, "killpg") as killpg, self.assertRaises(
            KeyboardInterrupt
        ):
            MODULE.run_owned_child(["harpoon"], {})

        killpg.assert_called_once_with(4343, signal.SIGKILL)
        self.assertEqual(process.wait.call_count, 3)

    def test_runs_both_arms_in_order_and_keeps_baseline_failure_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            workspace = root / "workspace"
            workspace.mkdir()
            artifact_dir = root / "artifacts"
            arguments = MODULE.build_argument_parser().parse_args(
                [
                    "--baseline-binary",
                    str(root / "baseline-app"),
                    "--candidate-binary",
                    str(root / "candidate-app"),
                    "--baseline-source-root",
                    str(root / "baseline-source"),
                    "--candidate-source-root",
                    str(root / "candidate-source"),
                    "--artifact-dir",
                    str(artifact_dir),
                ]
            )
            calls = []

            def run_child(command, environment):
                arm = (
                    "baseline"
                    if any(str(value).endswith("/baseline-app") for value in command)
                    else "candidate"
                )
                calls.append((arm, command, environment))
                child_dir = Path(command[command.index("--artifact-dir") + 1])
                child_dir.mkdir()
                binary_sha = f"{arm}-binary"
                fingerprint = {
                    "schema_version": 1,
                    "full_sha256": f"{arm}-full",
                    "matrix_invariant": {
                        "workspace_commit": "workspace",
                        "workspace_source": {"sha256": "source"},
                        "binary": {"sha256": binary_sha},
                        "runner": {"sha256": "runner"},
                        "content_revision": "content",
                        "runtime_data": [{"tree_sha256": "runtime"}],
                        "profile_big_icon": {"sha256": "icon"},
                    },
                    "scenario": {"tree_sha256": "scenario"},
                }
                (child_dir / "manifest.json").write_text(
                    json.dumps(
                        {
                            "schema_version": 1,
                            "input_fingerprint": fingerprint,
                            "topology": {"remote_player_profiles": 24},
                            "settings": {
                                "players": 24,
                                "clients": 12,
                                "runtime_only": False,
                            },
                        }
                    ),
                    encoding="utf-8",
                )
                (child_dir / "summary.json").write_text(
                    json.dumps(
                        {
                            "schema_version": 1,
                            "result": "fail" if arm == "baseline" else "pass",
                            "players_requested": 24,
                            "clients_requested": 12,
                            "acceptance": {"presentation_required": True},
                        }
                    ),
                    encoding="utf-8",
                )
                (child_dir / "input-fingerprint-final.json").write_text(
                    json.dumps(fingerprint), encoding="utf-8"
                )
                return 1 if arm == "baseline" else 0

            def run_unowned_child(command, *, check, env):
                self.assertFalse(check)
                return SimpleNamespace(returncode=run_child(command, env))

            with mock.patch.object(
                MODULE, "validate_paired_arguments"
            ), mock.patch.object(
                MODULE,
                "validate_hzck_scenario_contract",
                return_value={"crew_id": "HZCK"},
            ), mock.patch.object(
                MODULE,
                "collect_run_provenance",
                return_value={"sources": {}, "binaries": {}},
            ), mock.patch.object(
                MODULE,
                "validate_hzck_runtime_evidence",
                return_value={"runtime_crew_objects": 24},
            ), mock.patch.object(
                MODULE,
                "collect_retained_gpu_evidence",
                side_effect=[
                    {
                        "profiles_retained": 12,
                        "raw_profiles": [{"profile": "large raw profile"}],
                    },
                    {
                        "profiles_retained": 12,
                        "raw_profiles": [{"profile": "large raw profile"}],
                    },
                ],
            ) as collect_gpu, mock.patch.object(
                MODULE,
                "validate_paired_gpu_fingerprints",
                return_value={"comparison": "matched"},
            ), mock.patch.object(
                MODULE, "run_owned_child", side_effect=run_child
            ) as owned_child, mock.patch.object(
                MODULE.subprocess, "run", side_effect=run_unowned_child
            ):
                summary = MODULE.run_paired_benchmark(
                    arguments,
                    workspace=workspace,
                    artifact_dir=artifact_dir,
                )

        self.assertEqual([arm for arm, _, _ in calls], ["baseline", "candidate"])
        self.assertEqual(owned_child.call_count, 2)
        self.assertEqual(calls[0][2]["LC_GPU_TIMESTAMP_QUERIES"], "1")
        self.assertEqual(calls[1][2]["LC_GPU_TIMESTAMP_QUERIES"], "1")
        for call in collect_gpu.call_args_list:
            self.assertTrue(call.kwargs["profiles_required"])
            self.assertTrue(call.kwargs["expected_timestamp_request"])
            self.assertEqual(call.kwargs["minimum_schema_version"], 2)
        self.assertEqual(summary["result"], "fail")
        self.assertEqual(summary["arms"]["baseline"]["return_code"], 1)
        self.assertEqual(summary["arms"]["candidate"]["return_code"], 0)
        self.assertEqual(
            summary["paired_input_fingerprint"]["scenario_tree_sha256"],
            "scenario",
        )
        self.assertEqual(
            summary["paired_gpu_fingerprint"]["comparison"],
            "matched",
        )
        self.assertNotIn(
            "raw_profiles",
            summary["arms"]["candidate"]["retained_gpu_evidence"],
        )
        self.assertTrue(
            summary["arms"]["candidate"]["retained_gpu_evidence_artifact"].endswith(
                "/candidate/retained-gpu-evidence.json"
            )
        )

    def test_main_returns_the_paired_result_without_building_binaries(self):
        arguments = [
            "--baseline-binary",
            "/baseline/clonk-app",
            "--candidate-binary",
            "/candidate/clonk-app",
            "--baseline-source-root",
            "/baseline",
        ]
        with mock.patch.object(
            MODULE,
            "run_paired_benchmark",
            side_effect=[{"result": "pass"}, {"result": "fail"}],
        ) as run:
            passing = MODULE.main(arguments)
            failing = MODULE.main(arguments)

        self.assertEqual(passing, 0)
        self.assertEqual(failing, 1)
        self.assertEqual(run.call_count, 2)


if __name__ == "__main__":
    unittest.main()
