"""Device-loss qualification must establish recovery, not trust a verdict."""

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))


class DeviceLossQualificationTests(unittest.TestCase):
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

    def test_release_qualification_exercises_each_desktop_backend(self):
        workflow = (SCRIPTS.parent / ".github/workflows/device-loss-qualification.yml").read_text()
        for backend in ("vulkan", "gl", "dx12", "metal"):
            self.assertIn(backend, workflow)
        self.assertIn("run_device_loss_probe.py --release", workflow)
        self.assertIn("libxkbcommon-x11-0", workflow)
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
            self.assertEqual(runner.check_report(report, "vulkan"), evidence)

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
