"""Platform pipelines wait only for their own producers and qualify shipped bytes."""

import re
import unittest

from _repo import REPOSITORY
from test_release_prebuild_workflow import platform_matrices


WORKFLOWS = REPOSITORY / ".github/workflows"


class ReleasePlatformPipelineTests(unittest.TestCase):
    def test_each_platform_packages_after_its_own_prebuild(self):
        workflow = (WORKFLOWS / "release-platform.yml").read_text()
        self.assertIn("uses: ./.github/workflows/release-prebuild.yml", workflow)
        self.assertIn("uses: ./.github/workflows/release-build.yml", workflow)
        self.assertIn("needs: prebuild", workflow)
        self.assertEqual(workflow.count("platform: ${{ inputs.platform }}"), 2)
        landing = (WORKFLOWS / "landing.yml").read_text()
        self.assertIn("uses: ./.github/workflows/release-platform.yml", landing)
        self.assertIn("platform: [linux, windows, macos]", landing)
        self.assertIn("needs: release-context", landing)
        self.assertNotIn("needs: [release-context, linux, windows-smoke]", landing)
        self.assertIn("max-parallel: ${{ needs.release-context.outputs.release == 'true' && 5 || 17 }}", landing)
        qualification = (WORKFLOWS / "exact-sha-qualification.yml").read_text()
        self.assertIn("max-parallel: ${{ inputs.upload-diagnostics && 12 || 6 }}", qualification)
        prebuild = (WORKFLOWS / "release-prebuild.yml").read_text()
        tool = prebuild[prebuild.index("  tool:"):prebuild.index("  runtime:")]
        runtime = prebuild[prebuild.index("  runtime:"):]
        self.assertIn("needs: [validate, runtime]", tool)
        self.assertIn("needs: validate", runtime)
        # Runtime slots are reused by the platform's own tool/package.
        pipeline_slots = sum(len(rows) for rows in platform_matrices(runtime).values())
        linux_slots = int(re.search(r"release == 'true' && (\d+) \|\| 17", landing)[1])
        coverage_slots = int(re.search(r"upload-diagnostics && 12 \|\| (\d+)", qualification)[1])
        windows = landing[landing.index("  windows-smoke:"):landing.index("  release-evidence:")]
        smoke_slots = len(re.findall(r"(?m)^          - name:", windows))
        lints = qualification[qualification.index("  platform-lints:"):]
        native_slots = 1 + len(re.findall(r"(?m)^          - name:", lints))
        self.assertEqual(linux_slots + coverage_slots + pipeline_slots + smoke_slots + native_slots, 20)

    def test_gpu_recovery_uses_verified_runtime_after_platform_transforms(self):
        workflow = (WORKFLOWS / "release-build.yml").read_text()
        self.assertIn("uses: ./.github/actions/device-loss", workflow)
        self.assertIn("prebuilt-root: target/release-qualified/${{ matrix.name }}", workflow)
        self.assertLess(workflow.index("name: Check the macOS build is universal"), workflow.index("name: Qualify the packaged runtime"))
        self.assertIn("source_root=target/dist/qualification-unpack/Contents/MacOS", workflow)
        self.assertIn("--target universal-apple-darwin", workflow)
        self.assertIn("scripts/release-prebuild-manifest.py write", workflow)
        action = (REPOSITORY / ".github/actions/device-loss/action.yml").read_text()
        self.assertIn('--prebuilt-root "$PREBUILT_ROOT"', action)
        self.assertIn("vulkan gl", action)
        self.assertIn("dx12", action)
        self.assertIn("metal", action)
        self.assertNotIn("cargo build", action)
        self.assertIn("if: always()", action)
        self.assertIn("device-loss-${{ runner.os }}-${{ inputs.source-sha }}", action)

    def test_every_release_result_remains_required(self):
        landing = (WORKFLOWS / "landing.yml").read_text()
        gate = landing[landing.index("  landing-gate:"):]
        self.assertIn("release-build", gate)
        self.assertIn('require_result release-build "$RELEASE_BUILD_RESULT" success', gate)
        self.assertIn('require_result release-qualification "$RELEASE_QUALIFICATION_RESULT" success', gate)
        self.assertNotIn("RELEASE_PREBUILD_RESULT", gate)
        # Captures start in the first wave even when release qualification shares the pool.
        linux = landing[landing.index("  linux:"):landing.index("  windows-smoke:")]
        first = re.search(r"include:\n          - name: ([^\n]+)", linux)
        self.assertEqual(first.group(1), "presentation captures")


if __name__ == "__main__":
    unittest.main()
