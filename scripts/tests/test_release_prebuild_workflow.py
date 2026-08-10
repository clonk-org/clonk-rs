"""Static guards for trusted release prebuild artifacts."""

import re
import unittest

from _repo import REPOSITORY


WORKFLOW = REPOSITORY / ".github" / "workflows" / "release-prebuild.yml"


def job_block(name):
    source = WORKFLOW.read_text(encoding="utf-8")
    marker = f"\n  {name}:\n"
    start = source.index(marker) + 1
    following = re.compile(r"^  [A-Za-z0-9_-]+:$", re.MULTILINE)
    match = following.search(source, start + 1)
    return source[start : match.start()] if match else source[start:]


class ReleasePrebuildWorkflowTests(unittest.TestCase):
    def test_reusable_workflow_accepts_only_release_identity_inputs(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("workflow_call:", workflow)
        for input_name in ("source-sha", "tree-sha", "version", "pr-number"):
            with self.subTest(input_name=input_name):
                match = re.search(
                    rf"^      {re.escape(input_name)}:\n(?P<body>(?:        .*\n)+)",
                    workflow,
                    re.MULTILINE,
                )
                self.assertIsNotNone(match)
                input_block = match.group("body")
                self.assertIn("required: true", input_block)
                self.assertIn("type: string", input_block)

        permissions = workflow.split("\npermissions:\n", 1)[1].split("\nenv:\n", 1)[0]
        self.assertIn("  contents: read", permissions)
        self.assertIn("  pull-requests: read", permissions)
        self.assertNotIn("write", permissions)
        self.assertNotIn("actions/create-github-app-token@", workflow)
        self.assertNotIn("RELEASE_APP_PRIVATE_KEY", workflow)

    def test_validation_precedes_every_build_and_pins_the_trusted_release_pr(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        validation = job_block("validate")

        for fragment in (
            "github.event_name == 'merge_group'",
            '[[ "$SOURCE_SHA" != "$MERGE_SHA" ]]',
            'gh api "repos/${REPOSITORY}/pulls/${PR_NUMBER}"',
            '.state == "open"',
            '.base.ref == "main"',
            '.base.repo.full_name == $repository',
            '.head.ref == "release/next"',
            '.head.repo.full_name == $repository',
            'gh api "repos/${REPOSITORY}/git/commits/${SOURCE_SHA}" --jq .tree.sha',
            '[[ "$source_tree" != "$TREE_SHA" ]]',
            "ref: ${{ inputs.source-sha }}",
            '[[ "$(git rev-parse HEAD)" == "$SOURCE_SHA" ]]',
            '[[ "$(git rev-parse HEAD^{tree})" == "$TREE_SHA" ]]',
            "REQUESTED_VERSION: ${{ inputs.version }}",
            "workspace version ${actual_version} does not match requested release ${REQUESTED_VERSION}",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, validation)

        for name in ("tool", "runtime"):
            with self.subTest(job=name):
                self.assertIn("needs: validate", job_block(name))

    def test_build_topology_has_three_host_tools_and_four_native_runtimes(self):
        tool = job_block("tool")
        runtime = job_block("runtime")

        for platform in ("linux", "windows", "macos"):
            with self.subTest(tool=platform):
                self.assertEqual(tool.count(f"artifact: release-prebuild-tool-{platform}"), 1)
        for platform in ("linux", "windows", "macos-arm64", "macos-x86_64"):
            with self.subTest(runtime=platform):
                self.assertEqual(
                    runtime.count(f"artifact: release-prebuild-runtime-{platform}"),
                    1,
                )

        self.assertIn("cargo build --profile test --locked -p xtask", tool)
        self.assertIn("--features engine-tools --bin xtask-engine-tools", tool)
        self.assertNotIn("cargo build --release", tool)
        self.assertIn("--target aarch64-apple-darwin", runtime)
        self.assertIn("--target x86_64-apple-darwin", runtime)
        self.assertEqual(tool.count("target: host"), 3)
        for target in (
            "x86_64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ):
            with self.subTest(target=target):
                self.assertIn(f"target: {target}", runtime)

    def test_prebuilds_preserve_trusted_caches_and_msvc_validation(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        runtime = job_block("runtime")

        for cache in (
            "shared-key: full-parity",
            "shared-key: windows-runtime-msvc",
            "shared-key: recording-host-oracles",
            "shared-key: shipped-msvc-runtime-v1",
        ):
            with self.subTest(cache=cache):
                self.assertIn(cache, workflow)
        self.assertIn("run: scripts/configure-msvc-runtime.sh", runtime)
        self.assertIn("name: Restore trusted-main ThinLTO cache", runtime)
        self.assertIn("clonk-msvc-thinlto-v2-windows-x64-rustc-1.97.1-llvm-22.1.6-", runtime)
        self.assertIn("run: scripts/validate-msvc-runtime.sh", runtime)
        self.assertNotIn("Publish trusted ThinLTO cache", workflow)

    def test_every_artifact_is_flattened_under_payload_and_has_a_manifest(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        tool = job_block("tool")
        runtime = job_block("runtime")

        self.assertIn('artifact_dir="target/release-prebuild/${{ matrix.artifact }}"', tool)
        self.assertIn('rm -rf -- "$artifact_dir"', tool)
        self.assertIn('mkdir -p "$artifact_dir/payload"', tool)
        self.assertIn('[[ ! -s "${{ matrix.tool_path }}" ]]', tool)
        self.assertIn('"$artifact_dir/payload/${{ matrix.filename }}"', tool)
        self.assertIn('artifact_dir="target/release-prebuild/${{ matrix.artifact }}"', runtime)
        self.assertIn('rm -rf -- "$artifact_dir"', runtime)
        self.assertIn('mkdir -p "$artifact_dir/payload"', runtime)
        self.assertIn('"$artifact_dir/payload/${binary}${{ matrix.suffix }}"', runtime)

        self.assertEqual(workflow.count("scripts/release-prebuild-manifest.py write"), 2)
        for block, kind in ((tool, "tool"), (runtime, "runtime")):
            with self.subTest(kind=kind):
                for fragment in (
                    '--root "$artifact_dir"',
                    '--manifest "$artifact_dir/manifest.json"',
                    '--head-sha "$SOURCE_SHA"',
                    '--tree-sha "$TREE_SHA"',
                    '--version "$VERSION"',
                    f"--kind {kind}",
                    '--target "${{ matrix.target }}"',
                ):
                    self.assertIn(fragment, block)

        self.assertIn("--file \"payload/${{ matrix.filename }}\"", tool)
        for binary in ("c4group", "clonk-app", "clonk-game"):
            self.assertIn(f'--file "payload/{binary}${{{{ matrix.suffix }}}}"', runtime)

    def test_matrix_expands_to_exactly_seven_exact_run_cache_handoffs(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        expected = {
            "release-prebuild-tool-linux",
            "release-prebuild-tool-windows",
            "release-prebuild-tool-macos",
            "release-prebuild-runtime-linux",
            "release-prebuild-runtime-windows",
            "release-prebuild-runtime-macos-arm64",
            "release-prebuild-runtime-macos-x86_64",
        }
        declared = set(re.findall(r"artifact: (release-prebuild-[a-z0-9_-]+)", workflow))

        self.assertEqual(declared, expected)
        self.assertEqual(workflow.count("uses: actions/cache/save@"), 2)
        self.assertEqual(workflow.count("uses: actions/cache/restore@"), 3)
        self.assertEqual(workflow.count("lookup-only: true"), 2)
        self.assertEqual(workflow.count("fail-on-cache-miss: true"), 2)
        self.assertEqual(workflow.count("name: Hand off the"), 2)
        self.assertEqual(workflow.count("path: ${{ matrix.artifact_dir }}"), 0)
        self.assertEqual(workflow.count("path: target/release-prebuild/${{ matrix.artifact }}"), 4)
        self.assertEqual(
            workflow.count(
                "key: release-prebuild-${{ matrix.artifact }}-"
                "${{ inputs.source-sha }}-${{ github.run_id }}"
            ),
            4,
        )
        self.assertNotIn("actions/upload-artifact@", workflow)


if __name__ == "__main__":
    unittest.main()
