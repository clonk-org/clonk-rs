"""Device-loss qualification must establish recovery, not trust a verdict."""

import importlib.util
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

RUSTC_FIXTURE = "rustc 1.98.1 fixture\nhost: x86_64-unknown-linux-gnu\nrelease: 1.98.1\nLLVM version: 22.1.8\n"


def write_selected_runtime_manifest(root):
    """Build a complete artifact fixture for the mocked verified-binary boundary."""
    spec = importlib.util.spec_from_file_location("device_manifest_fixture", SCRIPTS / "release-prebuild-manifest.py")
    manifest = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(manifest)
    target = "x86_64-unknown-linux-gnu"
    environment = dict.fromkeys(manifest.build_environment_names(target))
    environment["RUSTFLAGS"] = "-Cdebuginfo=1"
    files = []
    for name in ("c4group", "clonk-app", "clonk-game"):
        path = root / f"payload/{name}"
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_bytes(b"verified shipped runtime")
        encoded = path.read_bytes()
        files.append({"path": f"payload/{name}", "size": len(encoded), "sha256": hashlib.sha256(encoded).hexdigest()})
    document = {
        "schema": 3, "head_sha": "1" * 40, "tree_sha": "2" * 40, "version": "1.2.3",
        "kind": "runtime", "target": target, "files": files,
        "provenance": {
            "recipe_sha": "3" * 64, "toolchain_sha": hashlib.sha256(RUSTC_FIXTURE.encode()).hexdigest(),
            "content_sha": "5" * 40, "builder": {"run_id": "123456", "run_attempt": 2},
            "producer_recipe": {
                "operation": "build", "profile": "release", "features": [], "target": target,
                "environment": environment, "rustc": RUSTC_FIXTURE, "parents": [],
                "native_tools": {"cc": None, "cxx": None, "linker": None, "sdk": {"root": None, "version": None}},
            },
        },
    }
    (root / "manifest.json").write_text(json.dumps(document))
    return document


class DeviceLossQualificationTests(unittest.TestCase):
    def test_prebuilt_release_runtime_is_selected_and_qualified_without_a_cargo_build(self):
        from PIL import Image
        import run_device_loss_probe as runner

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "evidence"
            prebuilt = root / "prebuilt"
            binary = prebuilt / "payload/clonk-app"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"verified shipped runtime")
            producer = write_selected_runtime_manifest(prebuilt)

            def execute(command, **_options):
                self.assertNotEqual(command[0], "cargo")
                self.assertEqual(Path(command[0]), binary)
                report = Path(command[command.index("--device-loss-probe") + 1])
                pixels = Image.new("RGBA", (2, 1), (12, 34, 56, 255))
                pixels.putpixel((1, 0), (90, 80, 70, 255))
                pixels.save(report.with_suffix(".before.png"))
                pixels.save(report.with_suffix(".after.png"))
                report.write_text(json.dumps({
                    "schema_version": 2, "kind": "clonk_device_loss_probe", "success": True,
                    "surface_counts_at_drop": [1, 0], "presented_before_loss": 30,
                    "generation_before": 1, "generation_after": 2,
                    "callback_diagnosis": "Destroyed", "rebuild_ok": True,
                    "presented_after_recovery": 3, "adapter": {"backend": "vulkan"},
                    "resource_recovery": {
                        "extent": [2, 1], "textures_before": 1, "textures_after": 1,
                        "texture_ids_before": [7], "texture_ids_after": [7],
                        "recreated_textures": 1, "full_upload_calls": 1,
                        "full_upload_bytes": 8, "pixels_identical": True,
                    },
                }))
                return SimpleNamespace(returncode=0)

            with (
                mock.patch.object(runner.presentation, "refuse_to_run_as_root"),
                mock.patch.object(runner, "source_identity", return_value={"source_dirty": False}),
                mock.patch.object(runner.presentation, "build_binary", return_value=binary) as build,
                mock.patch.object(runner.subprocess, "run", side_effect=execute) as run,
                mock.patch.object(runner.subprocess, "check_output", return_value=RUSTC_FIXTURE),
                mock.patch.dict(runner.os.environ, {"RUSTFLAGS": "-Cdebuginfo=0"}),
            ):
                self.assertEqual(runner.main([
                    "--release", "--prebuilt-root", str(prebuilt), "--no-xvfb",
                    "--backend", "vulkan", "--artifact-dir", str(artifacts),
                ]), 0)
            build.assert_called_once_with(runner.REPOSITORY, True, prebuilt)
            run.assert_called_once()
            qualification = json.loads((artifacts / "qualification.json").read_text())
            self.assertEqual(qualification["binary_sha256"], runner.presentation.file_digest(binary))
            self.assertEqual(qualification["prebuilt_manifest"], producer)
            self.assertEqual(qualification["prebuilt_manifest_sha256"], runner.presentation.file_digest(prebuilt / "manifest.json"))
            self.assertEqual(qualification["build_origin"], "prebuilt")
            self.assertEqual(qualification["rustflags"], "-Cdebuginfo=1")
            self.assertEqual(qualification["qualification_environment"]["rustflags"], "-Cdebuginfo=0")

    def test_prebuilt_device_probe_requires_release_before_selecting_or_building_a_binary(self):
        import run_device_loss_probe as runner

        with tempfile.TemporaryDirectory() as directory:
            with (
                mock.patch.object(runner.presentation, "refuse_to_run_as_root"),
                mock.patch.object(runner, "source_identity", return_value={"source_dirty": False}),
                mock.patch.object(runner.presentation, "build_binary",
                                  side_effect=AssertionError("debug prebuilt selection attempted")) as build,
                self.assertRaisesRegex(SystemExit, "requires --release"),
            ):
                runner.main([
                    "--prebuilt-root", directory, "--backend", "vulkan",
                    "--artifact-dir", str(Path(directory) / "reports"),
                ])
            build.assert_not_called()

    def test_prebuilt_device_probe_rejects_source_or_binary_changes_during_the_run(self):
        import run_device_loss_probe as runner

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "prebuilt/payload/clonk-app"
            binary.parent.mkdir(parents=True)
            for changed in ("source", "binary"):
                with self.subTest(changed=changed):
                    artifacts = root / changed
                    binary.write_bytes(b"verified runtime")
                    write_selected_runtime_manifest(root / "prebuilt")
                    before = {"source_dirty": False, "commit": "original"}
                    after = {**before, "commit": "changed"} if changed == "source" else before

                    def execute(_command, **_options):
                        if changed == "binary":
                            binary.write_bytes(b"changed runtime")
                        return SimpleNamespace(returncode=0)

                    with (
                        mock.patch.object(runner.presentation, "refuse_to_run_as_root"),
                        mock.patch.object(runner, "source_identity", side_effect=[before, after]),
                        mock.patch.object(runner.presentation, "build_binary", return_value=binary),
                        mock.patch.object(runner.subprocess, "run", side_effect=execute),
                        mock.patch.object(runner, "check_report", return_value={}),
                        self.assertRaisesRegex(SystemExit, "source or executable changed"),
                    ):
                        runner.main([
                            "--release", "--prebuilt-root", str(root / "prebuilt"), "--no-xvfb",
                            "--backend", "vulkan", "--artifact-dir", str(artifacts),
                        ])
                    self.assertFalse((artifacts / "qualification.json").exists())

    def test_prebuilt_manifest_changes_during_device_recovery_cannot_produce_qualification(self):
        import run_device_loss_probe as runner

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prebuilt = root / "prebuilt"
            write_selected_runtime_manifest(prebuilt)
            manifest = prebuilt / "manifest.json"
            artifacts = root / "evidence"

            def execute(command, **_options):
                self.assertNotEqual(command[0], "cargo")
                report = Path(command[command.index("--device-loss-probe") + 1])
                report.write_text(json.dumps({"adapter": {"backend": "vulkan"}}))
                for suffix in (".before.png", ".after.png"):
                    report.with_suffix(suffix).write_bytes(b"checked pixel fixture")
                manifest.write_bytes(manifest.read_bytes() + b"\n")
                return SimpleNamespace(returncode=0)

            with (
                mock.patch.object(runner.presentation, "refuse_to_run_as_root"),
                mock.patch.object(runner, "source_identity", return_value={"source_dirty": False}),
                mock.patch.object(runner.presentation, "build_binary", return_value=prebuilt / "payload/clonk-app"),
                mock.patch.object(runner, "check_report", return_value={"adapter": {"backend": "vulkan"}}),
                mock.patch.object(runner.subprocess, "run", side_effect=execute),
                mock.patch.object(runner.subprocess, "check_output", return_value=RUSTC_FIXTURE),
                self.assertRaisesRegex(SystemExit, "manifest changed"),
            ):
                runner.main([
                    "--release", "--prebuilt-root", str(prebuilt), "--no-xvfb",
                    "--backend", "vulkan", "--artifact-dir", str(artifacts),
                ])
            self.assertFalse((artifacts / "qualification.json").exists())

    def test_the_fixture_has_a_real_player_so_no_blinking_name_editor_opens(self):
        import configparser
        import run_device_loss_probe as runner

        with tempfile.TemporaryDirectory() as directory:
            artifacts = Path(directory)
            config = configparser.ConfigParser()
            config.read(runner.prepare_fixture(artifacts))
            player = Path(config["General"]["Participants"])
            self.assertEqual(player.parent, Path(config["General"]["PlayerPath"]))
            self.assertEqual(player.read_bytes(), (
                SCRIPTS.parent / "crates/clonk-engine/tests/fixtures/embedded_player.c4p"
            ).read_bytes())

    def test_the_probe_runs_lock_file_does_not_dirty_the_next_backends_source_check(self):
        # The probe runs the app with the checkout as its install root, so the
        # update lock it retains there (`crates/clonk-update/src/apply.rs`) must
        # not count as an uncommitted input for the next backend's run.
        import subprocess

        ignored = subprocess.run(
            ["git", "-C", str(SCRIPTS.parent), "check-ignore", "--no-index", "-q", ".clonk-update.lock"],
            check=False,
        )
        self.assertEqual(ignored.returncode, 0)

    def test_release_qualification_exercises_each_desktop_backend(self):
        workflow = (SCRIPTS.parent / ".github/workflows/device-loss-qualification.yml").read_text()
        self.assertIn("uses: ./.github/actions/device-loss", workflow)
        self.assertIn("source-sha: ${{ inputs.source-sha }}", workflow)
        action = (SCRIPTS.parent / ".github/actions/device-loss/action.yml").read_text()
        for backend in ("vulkan", "gl", "dx12", "metal"):
            self.assertIn(backend, action)
        self.assertIn("runner.os == 'Linux' && 'vulkan gl'", action)
        self.assertIn("runner.os == 'Windows' && 'dx12' || 'metal'", action)
        self.assertIn("run_device_loss_probe.py --release", action)
        self.assertIn("libxkbcommon-x11-0", action)
        self.assertIn("for backend in $PROBE_BACKENDS", action)
        self.assertIn('arguments+=(--prebuilt-root "$PREBUILT_ROOT")', action)
        self.assertIn('exit "$status"', action)
        release = (SCRIPTS.parent / ".github/workflows/exact-sha-qualification.yml").read_text()
        self.assertIn("uses: ./.github/workflows/device-loss-qualification.yml", release)

    def test_the_old_surface_must_be_released_before_replacement(self):
        from PIL import Image
        import run_device_loss_probe as runner

        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            pixels = Image.new("RGBA", (2, 1), (12, 34, 56, 255))
            pixels.putpixel((1, 0), (90, 80, 70, 255))
            pixels.save(report.with_suffix(".before.png"))
            pixels.save(report.with_suffix(".after.png"))
            evidence = {
                "schema_version": 2, "kind": "clonk_device_loss_probe", "success": True,
                "presented_before_loss": 30, "generation_before": 1, "generation_after": 2,
                "callback_diagnosis": "Destroyed", "rebuild_ok": True,
                "presented_after_recovery": 3, "adapter": {"backend": "vulkan"},
                "resource_recovery": {
                    "extent": [2, 1], "textures_before": 1, "textures_after": 1,
                    "recreated_textures": 1, "full_upload_calls": 1,
                    "full_upload_bytes": 8, "pixels_identical": True,
                },
            }
            report.write_text(json.dumps(evidence))
            with self.assertRaisesRegex(SystemExit, "surface"):
                runner.check_report(report, "vulkan")
            evidence["surface_counts_at_drop"] = [1, 0]
            report.write_text(json.dumps(evidence))
            with self.assertRaisesRegex(SystemExit, "identit"):
                runner.check_report(report, "vulkan")
            evidence["resource_recovery"].update({
                "texture_ids_before": [7], "texture_ids_after": [7],
                "resident_textures_before": 2,
            })
            report.write_text(json.dumps(evidence))
            self.assertEqual(runner.check_report(report, "vulkan"), evidence)
            evidence["resource_recovery"]["texture_ids_after"] = [8]
            report.write_text(json.dumps(evidence))
            with self.assertRaisesRegex(SystemExit, "identit"):
                runner.check_report(report, "vulkan")

    def test_current_schema_still_requires_images_and_recreated_resources(self):
        import run_device_loss_probe as runner

        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            report.write_text(json.dumps({
                "schema_version": 2, "kind": "clonk_device_loss_probe", "success": True,
                "presented_before_loss": 30, "generation_before": 1, "generation_after": 2,
                "callback_diagnosis": "Destroyed", "rebuild_ok": True,
                "presented_after_recovery": 3, "adapter": {"backend": "vulkan"},
            }))
            with self.assertRaises(SystemExit):
                runner.check_report(report, "vulkan")

    def test_a_success_verdict_without_resource_and_pixel_evidence_is_rejected(self):
        spec = importlib.util.spec_from_file_location(
            "device_loss_runner", SCRIPTS / "run_device_loss_probe.py",
        )
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            report.write_text(json.dumps({
                "schema_version": 1, "kind": "clonk_device_loss_probe", "success": True,
                "presented_before_loss": 30, "generation_before": 1, "generation_after": 2,
                "callback_diagnosis": "Destroyed", "rebuild_ok": True,
                "presented_after_recovery": 3, "adapter": {"backend": "vulkan"},
            }))
            with self.assertRaisesRegex(SystemExit, "resource|schema|pixel"):
                runner.check_report(report, "vulkan")


if __name__ == "__main__":
    unittest.main()
