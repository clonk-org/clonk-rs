import importlib.util
import configparser
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from PIL import Image


SCRIPT = Path(__file__).resolve().parents[1] / "run_software_presentation_smoke.py"
SPEC = importlib.util.spec_from_file_location("software_presentation_smoke", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def presented_report():
    return {
        "schema_version": MODULE.SCHEMA_VERSION,
        "kind": MODULE.REPORT_KIND,
        "success": True,
        "failure": None,
        "initial_extent": [800, 600],
        "resized_extent": [760, 560],
        "presented_before_resize": True,
        "presented_after_resize": True,
        "phases": [
            {"name": name, "presented": True, "scale": scale,
             "drawable_extent": [760 * scale, 560 * scale],
             "clip_rect": [0, 0, 760 * scale, 560 * scale]}
            for name, scale in (("windowed", 1), ("fullscreen", 2), ("windowed-again", 1))
        ],
        "registry_empty_at_exit": True,
        "software_reason": "forced",
        "gpu_attempt_backends": [],
        "display_backend": "windows",
    }


def write_captures(report_path):
    for suffix, size in ((".screenshot.png", (760, 560)), (".thumbnail.png", (200, 150))):
        Image.new("RGBA", size, (0x6f, 0x2f, 0xa8, 255)).save(report_path.with_suffix(suffix))


class SoftwarePresentationRunnerTests(unittest.TestCase):
    def test_native_screenshots_stay_inside_the_artifact_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifacts = Path(temporary)
            binary = artifacts / "clonk-app.exe"
            binary.touch()

            def execute(command, **_options):
                config = configparser.ConfigParser()
                config.read(command[command.index("--config") + 1])
                folder = config.get("General", "ScreenshotFolder", fallback="Screenshots")
                destination = (MODULE.REPOSITORY / folder).resolve()
                self.assertEqual(destination, (artifacts / "screenshots").resolve())
                self.assertTrue(destination.is_dir())
                raise RuntimeError("checked native screenshot destination")

            with (
                mock.patch.object(MODULE, "refuse_to_run_as_root"),
                mock.patch.object(MODULE, "build_binary", return_value=binary),
                mock.patch.object(MODULE, "source_identity", return_value={}),
                mock.patch.object(MODULE.subprocess, "run", side_effect=execute),
                self.assertRaisesRegex(RuntimeError, "checked native screenshot destination"),
            ):
                MODULE.main(["--artifact-dir", str(artifacts), "--no-xvfb"])

    def test_windows_release_preserves_the_shipped_package_graph(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "release" / "clonk-app.exe"
            binary.parent.mkdir()
            binary.touch()
            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.dict(MODULE.os.environ, {"CARGO_TARGET_DIR": temporary}, clear=True),
                mock.patch.object(MODULE.subprocess, "run") as run,
            ):
                self.assertEqual(MODULE.build_binary(release=True), binary)
            command = run.call_args.args[0]
            self.assertIn("clonk-game", command)
            self.assertIn("clonk-c4group", command)
            self.assertNotIn("--bin", command)

    def test_windows_dispatch_qualifies_both_modes_on_the_shipped_runtime(self):
        workflow = (SCRIPT.parent.parent / ".github/workflows/rust.yml").read_text()
        self.assertIn("      software_presentation:\n", workflow)
        admission = workflow[workflow.index("  diagnostic-admission:"):workflow.index("  exact-sha-qualification:")]
        self.assertIn("!inputs.software_presentation", admission)
        job = workflow[workflow.index("  windows-release-tools:"):]
        validation = job.index("run: scripts/validate-msvc-runtime.sh")
        smoke = job.index("python scripts/run_software_presentation_smoke.py")
        self.assertLess(validation, smoke)
        self.assertIn("git submodule update --init --recursive", job)
        self.assertIn("--release --check-input", job)
        self.assertIn("--automatic-fallback", job)
        self.assertIn("name: windows-software-presentation", job)
        self.assertIn("always() && inputs.software_presentation", job)

    def test_source_changes_during_a_run_cannot_produce_qualification(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifacts = Path(temporary)
            binary = artifacts / "clonk-app.exe"
            binary.touch()

            def execute(command, **_options):
                report = Path(command[command.index("--software-present-smoke") + 1])
                report.write_text(json.dumps(presented_report()))
                write_captures(report)
                return SimpleNamespace(returncode=0)

            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.object(MODULE, "refuse_to_run_as_root"),
                mock.patch.object(MODULE, "build_binary", return_value=binary),
                mock.patch.object(MODULE, "source_identity", create=True,
                                  side_effect=[{"commit": "before"}, {"commit": "after"}]),
                mock.patch.object(MODULE.subprocess, "run", side_effect=execute),
                self.assertRaisesRegex(SystemExit, "source changed"),
            ):
                MODULE.main(["--artifact-dir", str(artifacts), "--no-xvfb"])

    def test_windows_qualification_rejects_another_window_backend(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            report = presented_report()
            report["display_backend"] = "appkit"
            path.write_text(json.dumps(report))
            write_captures(path)
            with self.assertRaisesRegex(SystemExit, "window backend"):
                MODULE.check_report(path, expected_backend="windows")

    def test_report_rejects_incorrect_thumbnail_pixels(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            path.write_text(json.dumps(presented_report()))
            write_captures(path)
            Image.new("RGBA", (200, 150), (0, 0, 0, 255)).save(path.with_suffix(".thumbnail.png"))
            with self.assertRaisesRegex(SystemExit, "thumbnail"):
                MODULE.check_report(path)

    def test_fallback_requires_failed_gpu_attempts_without_backend_widening(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            report = presented_report()
            for reason, attempts in (
                ("no-adapter", []),
                ("no-adapter", [["vulkan"]]),
                ("forced", [[], []]),
            ):
                with self.subTest(reason=reason, attempts=attempts):
                    report["software_reason"] = reason
                    report["gpu_attempt_backends"] = attempts
                    path.write_text(json.dumps(report))
                    with self.assertRaisesRegex(SystemExit, "fallback"):
                        MODULE.check_report(path, automatic_fallback=True)
            report["software_reason"] = "no-adapter"
            report["gpu_attempt_backends"] = [[], []]
            path.write_text(json.dumps(report))
            write_captures(path)
            MODULE.check_report(path, automatic_fallback=True)

    def test_input_qualification_is_opt_in(self):
        self.assertTrue(MODULE.parse_arguments(["--check-input"]).check_input)
        self.assertFalse(MODULE.parse_arguments([]).check_input)

    def test_report_requires_an_observed_scaled_pointer_event(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            report.write_text(json.dumps(presented_report()))
            with self.assertRaisesRegex(SystemExit, "pointer"):
                MODULE.check_report(report, check_input=True)

    def test_windows_build_uses_the_configured_release_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary)
            triple = "x86_64-pc-windows-msvc"
            binary = target / triple / "release" / "clonk-app.exe"
            binary.parent.mkdir(parents=True)
            binary.touch()
            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.dict(MODULE.os.environ, {
                    "CARGO_TARGET_DIR": str(target), "CARGO_BUILD_TARGET": triple,
                }, clear=True),
                mock.patch.object(MODULE.subprocess, "run"),
            ):
                self.assertEqual(MODULE.build_binary(True), binary)

    def test_windows_build_resolves_the_release_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary)
            binary = target / "release" / "clonk-app.exe"
            binary.parent.mkdir()
            binary.touch()
            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.dict(MODULE.os.environ, {"CARGO_TARGET_DIR": str(target)}, clear=True),
                mock.patch.object(MODULE.subprocess, "run") as run,
            ):
                self.assertEqual(MODULE.build_binary(True), binary)
            self.assertIn("--release", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
