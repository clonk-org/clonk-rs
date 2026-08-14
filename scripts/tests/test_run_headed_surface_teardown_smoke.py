import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


SCRIPT = (
    Path(__file__).resolve().parents[1]
    / "run_headed_surface_teardown_smoke.py"
)
SPEC = importlib.util.spec_from_file_location("headed_surface_teardown_smoke", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def nvidia_adapter():
    return {
        "name": "NVIDIA GeForce RTX 4070",
        "vendor_id": 0x10DE,
        "device_id": 0x2786,
        "device_type": "discrete-gpu",
        "pci_bus_id": "0000:01:00.0",
        "driver": "NVIDIA",
        "driver_info": "610.43.03",
        "backend": "vulkan",
        "subgroup_min_size": 32,
        "subgroup_max_size": 32,
        "transient_saves_memory": False,
    }


def authoritative_report():
    adapter = nvidia_adapter()
    return {
        "schema_version": 1,
        "kind": "clonk_headed_surface_smoke",
        "success": True,
        "failure": None,
        "display_backend": "wayland",
        "wayland_display": "wayland-0",
        "xdg_session_type": "wayland",
        "surface_windows": [
            {
                "role": "shell",
                "window_id": "WindowId(1)",
                "instance_entry_id": 1,
            },
            {
                "role": "viewport",
                "window_id": "WindowId(2)",
                "instance_entry_id": 1,
            },
        ],
        "instance_acquisitions": [
            {
                "sequence": 1,
                "entry_id": 1,
                "requested_backends": ["vulkan"],
                "created": True,
            },
            {
                "sequence": 2,
                "entry_id": 1,
                "requested_backends": ["vulkan"],
                "created": False,
            },
        ],
        "retained_registry": [
            {
                "entry_id": 1,
                "requested_backends": ["vulkan"],
                "acquisitions": 2,
                "resident_at_loop_exit": True,
            }
        ],
        "shell_adapter": adapter,
        "child_adapter": copy.deepcopy(adapter),
        "shell_presented_before_close": True,
        "child_presented_before_close": True,
        "child_closed_while_shell_survived": True,
        "child_released_after_close": True,
        "shell_presented_after_child_close": True,
        "loop_exiting_release_order": [0],
        "registry_empty_on_loop_exiting": True,
        "shell_released_on_loop_exiting": True,
    }


def mutate_both_adapters(report, **changes):
    report["shell_adapter"].update(changes)
    report["child_adapter"].update(changes)


def wiring_report():
    report = authoritative_report()
    report.update(
        display_backend="appkit",
        wayland_display=None,
        xdg_session_type=None,
    )
    mutate_both_adapters(
        report,
        name="Apple M3",
        vendor_id=0,
        device_id=0,
        device_type="integrated-gpu",
        pci_bus_id="",
        driver="",
        driver_info="",
        backend="metal",
    )
    for acquisition in report["instance_acquisitions"]:
        acquisition["requested_backends"] = ["metal"]
    report["retained_registry"][0]["requested_backends"] = ["metal"]
    return report


class HeadedSurfaceReportTests(unittest.TestCase):
    def test_accepts_exact_wayland_vulkan_nvidia_lifecycle_evidence(self):
        MODULE.validate_report(
            authoritative_report(),
            authoritative=True,
            expected_backend="vulkan",
        )

    def test_rejects_any_missing_driver_or_lifecycle_proof(self):
        mutations = {
            "failed app probe": lambda report: report.update(success=False),
            "wrong display": lambda report: report.update(display_backend="x11"),
            "same window twice": lambda report: report["surface_windows"][1].update(
                window_id="WindowId(1)"
            ),
            "second instance": lambda report: report["instance_acquisitions"][1].update(
                entry_id=2
            ),
            "second creation": lambda report: report["instance_acquisitions"][1].update(
                created=True
            ),
            "wrong vendor": lambda report: mutate_both_adapters(
                report, vendor_id=0x8086
            ),
            "Mesa NVK driver": lambda report: mutate_both_adapters(
                report, driver="Mesa NVK"
            ),
            "non-discrete adapter": lambda report: mutate_both_adapters(
                report, device_type="integrated-gpu"
            ),
            "empty driver": lambda report: mutate_both_adapters(report, driver=""),
            "survivor not presented": lambda report: report.update(
                shell_presented_after_child_close=False
            ),
            "shell not released": lambda report: report.update(
                shell_released_on_loop_exiting=False
            ),
            "release order missing": lambda report: report.update(
                loop_exiting_release_order=[]
            ),
            "boolean schema": lambda report: report.update(schema_version=True),
            "boolean sequence": lambda report: report["instance_acquisitions"][0].update(
                sequence=True
            ),
            "floating acquisition count": lambda report: report["retained_registry"][0].update(
                acquisitions=2.0
            ),
            "boolean release id": lambda report: report.update(
                loop_exiting_release_order=[False]
            ),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                report = authoritative_report()
                mutate(report)
                with self.assertRaises(MODULE.SmokeFailure):
                    MODULE.validate_report(
                        report,
                        authoritative=True,
                        expected_backend="vulkan",
                    )

    def test_wiring_only_accepts_real_non_nvidia_surface_evidence_without_claiming_crash_coverage(
        self,
    ):
        MODULE.validate_report(
            wiring_report(),
            authoritative=False,
            expected_backend="metal",
        )

    def test_duplicate_json_keys_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            report.write_text('{"schema_version": 1, "schema_version": 1}', encoding="utf-8")

            with self.assertRaisesRegex(MODULE.SmokeFailure, "duplicate JSON key"):
                MODULE.load_report(report)


class HeadedSurfaceHostTests(unittest.TestCase):
    def test_authoritative_run_requires_linux_wayland(self):
        environment = {
            "WAYLAND_DISPLAY": "wayland-0",
            "XDG_SESSION_TYPE": "wayland",
        }
        MODULE.validate_host_environment(
            environment,
            authoritative=True,
            system_name="Linux",
        )

        for system_name, changed_environment in (
            ("Darwin", environment),
            ("Linux", {"XDG_SESSION_TYPE": "wayland"}),
            ("Linux", {"WAYLAND_DISPLAY": "wayland-0", "XDG_SESSION_TYPE": "x11"}),
        ):
            with self.subTest(system_name=system_name, environment=changed_environment):
                with self.assertRaises(MODULE.SmokeFailure):
                    MODULE.validate_host_environment(
                        changed_environment,
                        authoritative=True,
                        system_name=system_name,
                    )

    def test_command_enters_only_the_hidden_production_lifecycle_mode(self):
        command = MODULE.build_command(
            binary=Path("/workspace/target/debug/clonk-app"),
            config=Path("/artifacts/Clonk.ini"),
            report=Path("/artifacts/app-report.json"),
            authoritative=True,
        )

        self.assertEqual(command.count("--headed-surface-smoke"), 1)
        self.assertIn("--display-server", command)
        self.assertIn("wayland", command)
        self.assertNotIn("--headless", command)
        self.assertNotIn("--sandbox", command)

    def test_timeout_values_must_be_finite_and_positive(self):
        self.assertEqual(MODULE.positive_seconds("1.5"), 1.5)
        for value in ("0", "-1", "nan", "inf", "-inf"):
            with self.subTest(value=value):
                with self.assertRaises(MODULE.argparse.ArgumentTypeError):
                    MODULE.positive_seconds(value)

    def test_controlled_environment_removes_ambient_config_and_adapter_overrides(self):
        environment = MODULE.build_environment(
            {
                "PATH": "/bin",
                "LC_CONFIG_FILE": "/home/operator/Clonk.ini",
                "LC_LOG": "operator.log",
                "RUST_LOG": "trace",
                "WGPU_ADAPTER_NAME": "software",
                "WGPU_POWER_PREF": "low",
                "WGPU_DX12_COMPILER": "hostile",
                "LC_CACHE_DIR": "/home/operator/cache",
                "LC_LOGS_DIR": "/home/operator/logs",
                "LC_TEMP_DIR": "/home/operator/temp",
                "LC_GAME_UPDATE_NOTICE": "stale notice",
                "LC_APP_OPEN_MENU": "Options",
            },
            workspace=Path("/workspace"),
            user_data=Path("/artifacts/user-data"),
            cache_dir=Path("/artifacts/cache"),
            logs_dir=Path("/artifacts/logs"),
            temp_dir=Path("/artifacts/temp"),
            expected_backend="vulkan",
        )

        self.assertEqual(environment["PATH"], "/bin")
        self.assertEqual(environment["WGPU_BACKEND"], "vulkan")
        self.assertEqual(environment["LC_CACHE_DIR"], "/artifacts/cache")
        self.assertEqual(environment["LC_LOGS_DIR"], "/artifacts/logs")
        self.assertEqual(environment["LC_TEMP_DIR"], "/artifacts/temp")
        for key in (
            "LC_CONFIG_FILE",
            "LC_GAME_UPDATE_NOTICE",
            "LC_APP_OPEN_MENU",
            "WGPU_ADAPTER_NAME",
            "WGPU_POWER_PREF",
            "WGPU_DX12_COMPILER",
        ):
            self.assertNotIn(key, environment)

    def test_clean_source_check_allows_only_the_process_lock(self):
        with mock.patch.object(
            MODULE,
            "_git_path_lines",
            side_effect=[[], [".clonk-update.lock"]],
        ):
            MODULE.require_clean_workspace(Path("/workspace"))

        with mock.patch.object(
            MODULE,
            "_git_path_lines",
            side_effect=[["crates/clonk-app/src/main.rs"], []],
        ):
            with self.assertRaisesRegex(MODULE.SmokeFailure, "clean source tree"):
                MODULE.require_clean_workspace(Path("/workspace"))

    def test_content_checkout_must_match_the_clean_pinned_gitlink(self):
        commit = "d" * 40
        with (
            mock.patch.object(MODULE, "_git_revision", side_effect=[commit, commit]),
            mock.patch.object(MODULE, "_git_path_lines", side_effect=[[], []]),
        ):
            self.assertEqual(MODULE.require_clean_content(Path("/workspace")), commit)

        with mock.patch.object(
            MODULE,
            "_git_revision",
            side_effect=[commit, "e" * 40],
        ):
            with self.assertRaisesRegex(MODULE.SmokeFailure, "pins"):
                MODULE.require_clean_content(Path("/workspace"))

    def test_authoritative_run_builds_and_binds_the_exact_clean_binary(self):
        commit = "a" * 40
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary) / "repo"
            workspace.mkdir()
            (workspace / "content").mkdir()
            artifact_dir = Path(temporary) / "artifacts"
            calls = []

            def run_process(command, **kwargs):
                calls.append((command, kwargs))
                if command[0] == "cargo":
                    target_dir = Path(command[command.index("--target-dir") + 1])
                    binary = target_dir / "release" / "clonk-app"
                    binary.parent.mkdir(parents=True)
                    binary.write_bytes(b"release binary tied to this build")
                    stdout = json.dumps(
                        {
                            "reason": "compiler-artifact",
                            "target": {"name": "clonk-app", "kind": ["bin"]},
                            "executable": str(binary),
                        }
                    )
                else:
                    report_index = command.index("--headed-surface-smoke") + 1
                    Path(command[report_index]).write_text(
                        json.dumps(authoritative_report()),
                        encoding="utf-8",
                    )
                    stdout = "ok\n"
                return SimpleNamespace(returncode=0, stdout=stdout, stderr="")

            arguments = MODULE.build_argument_parser().parse_args(
                [
                    "--workspace",
                    str(workspace),
                    "--artifact-dir",
                    str(artifact_dir),
                ]
            )
            ambient = {
                "WAYLAND_DISPLAY": "wayland-0",
                "XDG_SESSION_TYPE": "wayland",
                "LC_CONFIG_FILE": "/home/operator/Clonk.ini",
                "WGPU_ADAPTER_NAME": "software",
                "CARGO_TARGET_DIR": "/tmp/hostile-cargo-target",
                "CARGO_BUILD_TARGET": "not-the-host",
                "RUSTFLAGS": "--cfg hostile_build",
                "RUSTC_WRAPPER": "/tmp/hostile-rustc-wrapper",
                "RUSTUP_TOOLCHAIN": "nightly",
                "CARGO_PROFILE_RELEASE_LTO": "off",
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS": "--cfg hostile_target",
            }
            with (
                mock.patch.object(MODULE, "_git_commit", side_effect=[commit, commit]),
                mock.patch.object(MODULE, "require_clean_workspace") as clean,
                mock.patch.object(
                    MODULE,
                    "require_clean_content",
                    side_effect=["d" * 40, "d" * 40],
                ) as clean_content,
                mock.patch.object(MODULE.subprocess, "run", side_effect=run_process),
                mock.patch.object(MODULE.platform, "system", return_value="Linux"),
                mock.patch.object(MODULE.platform, "platform", return_value="Linux-test"),
                mock.patch.dict(MODULE.os.environ, ambient, clear=True),
            ):
                evidence_path = MODULE.run_smoke(arguments)

            self.assertEqual(clean.call_count, 2)
            self.assertEqual(clean_content.call_count, 2)
            self.assertEqual(len(calls), 2)
            expected_target_dir = (artifact_dir / "cargo-target").resolve()
            self.assertEqual(
                calls[0][0], MODULE.authoritative_build_command(expected_target_dir)
            )
            self.assertNotIn("CARGO_TARGET_DIR", calls[0][1]["env"])
            self.assertNotIn("CARGO_BUILD_TARGET", calls[0][1]["env"])
            self.assertNotIn("RUSTFLAGS", calls[0][1]["env"])
            self.assertNotIn("RUSTC_WRAPPER", calls[0][1]["env"])
            self.assertNotIn("RUSTUP_TOOLCHAIN", calls[0][1]["env"])
            self.assertNotIn("CARGO_PROFILE_RELEASE_LTO", calls[0][1]["env"])
            self.assertNotIn(
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
                calls[0][1]["env"],
            )
            launch, launch_options = calls[1]
            self.assertEqual(
                launch[0],
                str((artifact_dir / "clonk-app").resolve()),
            )
            self.assertEqual(launch_options["env"]["WGPU_BACKEND"], "vulkan")
            self.assertNotIn("LC_CONFIG_FILE", launch_options["env"])
            self.assertNotIn("WGPU_ADAPTER_NAME", launch_options["env"])

            evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
            self.assertEqual(evidence["git_commit"], commit)
            self.assertEqual(evidence["content_commit"], "d" * 40)
            self.assertTrue(evidence["source_clean_before_and_after"])
            self.assertEqual(
                evidence["binary_sha256_before"], evidence["binary_sha256_after"]
            )
            self.assertEqual(
                evidence["qualification"],
                "linux-wayland-vulkan-proprietary-nvidia",
            )
            self.assertEqual(evidence["build_target_dir"], str(expected_target_dir))
            self.assertEqual(
                evidence["cargo_artifact_sha256"], evidence["binary_sha256_before"]
            )
            self.assertIsNone(evidence["controlled_environment"]["LC_CONFIG_FILE"])
            self.assertEqual(
                (artifact_dir / "Clonk.ini").read_text(encoding="utf-8"),
                MODULE.SMOKE_CONFIG,
            )
            self.assertIn("DisplayMode=1\n", MODULE.SMOKE_CONFIG)
            self.assertIn("Maximized=false\n", MODULE.SMOKE_CONFIG)
            self.assertFalse(expected_target_dir.exists())

    def test_wiring_only_nonzero_exit_cannot_create_qualification(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary) / "repo"
            workspace.mkdir()
            (workspace / "content").mkdir()
            binary = workspace / "clonk-app"
            binary.write_bytes(b"wiring binary")
            artifact_dir = Path(temporary) / "artifacts"
            arguments = MODULE.build_argument_parser().parse_args(
                [
                    "--workspace",
                    str(workspace),
                    "--artifact-dir",
                    str(artifact_dir),
                    "--binary",
                    str(binary),
                    "--wiring-only",
                    "--backend",
                    "metal",
                ]
            )
            with (
                mock.patch.object(MODULE, "_git_commit", return_value="b" * 40),
                mock.patch.object(
                    MODULE.subprocess,
                    "run",
                    return_value=SimpleNamespace(
                        returncode=7,
                        stdout="partial stdout\n",
                        stderr="fatal stderr\n",
                    ),
                ),
            ):
                with self.assertRaisesRegex(MODULE.SmokeFailure, "exited 7"):
                    MODULE.run_smoke(arguments)

            self.assertEqual(
                (artifact_dir / "stdout.log").read_text(encoding="utf-8"),
                "partial stdout\n",
            )
            self.assertFalse((artifact_dir / "qualification.json").exists())

    def test_wiring_only_rejects_a_binary_that_changes_during_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary) / "repo"
            workspace.mkdir()
            (workspace / "content").mkdir()
            binary = workspace / "clonk-app"
            binary.write_bytes(b"binary before execution")
            artifact_dir = Path(temporary) / "artifacts"
            arguments = MODULE.build_argument_parser().parse_args(
                [
                    "--workspace",
                    str(workspace),
                    "--artifact-dir",
                    str(artifact_dir),
                    "--binary",
                    str(binary),
                    "--wiring-only",
                    "--backend",
                    "metal",
                ]
            )

            def mutate_binary(command, **_kwargs):
                self.assertNotIn("--display-server", command)
                report_index = command.index("--headed-surface-smoke") + 1
                Path(command[report_index]).write_text(
                    json.dumps(wiring_report()),
                    encoding="utf-8",
                )
                binary.write_bytes(b"binary changed during execution")
                return SimpleNamespace(returncode=0, stdout="", stderr="")

            with (
                mock.patch.object(MODULE, "_git_commit", return_value="c" * 40),
                mock.patch.object(MODULE.subprocess, "run", side_effect=mutate_binary),
            ):
                with self.assertRaisesRegex(MODULE.SmokeFailure, "binary changed"):
                    MODULE.run_smoke(arguments)

            self.assertFalse((artifact_dir / "qualification.json").exists())


if __name__ == "__main__":
    unittest.main()
