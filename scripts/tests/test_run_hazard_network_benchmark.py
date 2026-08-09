import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "run_hazard_network_benchmark.py"
SPEC = importlib.util.spec_from_file_location("hazard_benchmark", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class HazardInventoryTests(unittest.TestCase):
    def test_competitive_inventory_is_exactly_twelve_maps_without_tutorial(self):
        self.assertEqual(len(MODULE.HAZARD_COMPETITIVE_SCENARIOS), 12)
        self.assertNotIn(
            "Tutorial.c4s",
            {scenario.relative_path.name for scenario in MODULE.HAZARD_COMPETITIVE_SCENARIOS},
        )

    def test_inventory_drift_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            hazard = Path(temporary) / "Hazard.c4f"
            hazard.mkdir()
            for scenario in MODULE.HAZARD_COMPETITIVE_SCENARIOS:
                (hazard / scenario.relative_path.name).mkdir()
            (hazard / "Tutorial.c4s").mkdir()
            (hazard / "Unexpected.c4s").mkdir()

            with self.assertRaisesRegex(MODULE.HazardFailure, "inventory drift"):
                MODULE.validate_hazard_inventory(hazard)


class HazardRunnerTests(unittest.TestCase):
    def test_builds_each_map_with_hazard_runtime_and_input_gates(self):
        arguments = MODULE.build_argument_parser().parse_args([])
        scenario = MODULE.HAZARD_COMPETITIVE_SCENARIOS[0]

        command = MODULE.harpoon_command_for_scenario(
            arguments,
            workspace=Path("/workspace"),
            scenario=scenario,
            artifact_dir=Path("/artifacts") / scenario.slug,
        )

        self.assertIn("--scenario-title", command)
        self.assertIn(scenario.title, command)
        self.assertIn("--skip-sf5b-crew-assertion", command)
        self.assertIn("--runtime-only", command)
        self.assertIn("--input-probe-interval-ms", command)
        self.assertIn("500", command)
        self.assertIn("--measurement-seconds", command)
        self.assertIn("25", command)
        self.assertEqual(arguments.players, 24)
        self.assertEqual(command[command.index("--players") + 1], "24")
        self.assertEqual(arguments.clients, 12)
        self.assertEqual(command[command.index("--clients") + 1], "12")
        self.assertEqual(
            arguments.measurement_seconds
            * len(MODULE.HAZARD_COMPETITIVE_SCENARIOS),
            300,
        )
        self.assertIn("--minimum-simulation-fps", command)
        self.assertIn("38.0", command)
        self.assertEqual(command[command.index("--control-mode") + 1], "2")
        self.assertNotIn("--tcp-only", command)

    def test_full_matrix_continues_and_rejects_child_fingerprint_drift(self):
        arguments = MODULE.build_argument_parser().parse_args([])
        completed = [mock.Mock(returncode=0) for _ in range(12)]
        completed[2] = mock.Mock(returncode=1)

        with tempfile.TemporaryDirectory() as temporary:
            call_index = 0

            def run_child(command, check):
                nonlocal call_index
                artifact_dir = Path(
                    command[command.index("--artifact-dir") + 1]
                )
                artifact_dir.mkdir()
                fingerprint = "changed" if call_index == 4 else "stable"
                (artifact_dir / "manifest.json").write_text(
                    json.dumps(
                        {
                            "input_fingerprint": {
                                "matrix_invariant_sha256": fingerprint,
                            }
                        }
                    ),
                    encoding="utf-8",
                )
                result = completed[call_index]
                call_index += 1
                return result

            with mock.patch.object(MODULE, "validate_hazard_inventory"), mock.patch.object(
                MODULE.subprocess, "run", side_effect=run_child
            ) as run:
                summary = MODULE.run_hazard_matrix(
                    arguments,
                    workspace=Path("/workspace"),
                    artifact_dir=Path(temporary) / "artifacts",
                )

        self.assertEqual(run.call_count, 12)
        self.assertEqual(summary["result"], "fail")
        self.assertEqual(summary["scenarios"][2]["return_code"], 1)
        self.assertTrue(
            any("fingerprint differs" in error for error in summary["fingerprint_errors"])
        )
        self.assertEqual(
            [call.args[0][call.args[0].index("--scenario-title") + 1] for call in run.call_args_list],
            [scenario.title for scenario in MODULE.HAZARD_COMPETITIVE_SCENARIOS],
        )
        self.assertEqual(
            [
                int(call.args[0][call.args[0].index("--base-port") + 1])
                for call in run.call_args_list
            ],
            [31_111 + index * MODULE.SCENARIO_PORT_STRIDE for index in range(12)],
        )
