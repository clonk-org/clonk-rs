"""Static guards for trusted release prebuild artifacts."""

import json
import re
import unittest

from _repo import REPOSITORY


WORKFLOW = REPOSITORY / ".github" / "workflows" / "release-prebuild.yml"


def job_block(name, workflow=WORKFLOW):
    source = workflow.read_text(encoding="utf-8")
    marker = f"\n  {name}:\n"
    start = source.index(marker) + 1
    following = re.compile(r"^  [A-Za-z0-9_-]+:$", re.MULTILINE)
    match = following.search(source, start + 1)
    return source[start : match.start()] if match else source[start:]


def platform_matrices(block):
    """Expand the bounded platform selector used by the reusable workflows."""
    match = re.search(r"include: >-\s+\$\{\{ fromJSON\((.*?)\) \}\}", block, re.S)
    if match is None:
        raise AssertionError("platform job has no JSON include matrix")
    selector = re.fullmatch(
        r"inputs\.platform == 'macos' && '(?P<macos>[^']+)' \|\| "
        r"inputs\.platform == 'windows' && '(?P<windows>[^']+)' \|\| "
        r"'(?P<linux>[^']+)'",
        match.group(1),
    )
    if selector is None:
        raise AssertionError("platform matrix does not select exactly the supported platforms")
    matrices = {platform: json.loads(rows) for platform, rows in selector.groupdict().items()}
    if any(not isinstance(rows, list) or not rows for rows in matrices.values()):
        raise AssertionError("each platform must have a nonempty include matrix")
    return matrices


def job_dependencies(block):
    match = re.search(r"(?m)^    needs: (.+)$", block)
    if match is None:
        return set()
    value = match.group(1)
    if re.fullmatch(r"[a-z][a-z0-9_-]*", value):
        return {value}
    if not re.fullmatch(r"\[[a-z0-9_, -]+\]", value):
        raise AssertionError("release job must declare explicit bounded dependencies")
    return {name.strip() for name in value[1:-1].split(",")}


class ReleasePrebuildWorkflowTests(unittest.TestCase):
    def test_native_runtime_slots_are_reused_by_each_platform_tool_and_package(self):
        tool = job_block("tool")
        runtime = job_block("runtime")
        tools = platform_matrices(tool)
        runtimes = platform_matrices(runtime)
        self.assertEqual(job_dependencies(runtime), {"validate"})
        self.assertEqual(job_dependencies(tool), {"validate", "runtime"})
        self.assertNotIn("max-parallel:", runtime)

        # Validation releases four critical runtime rows. The three host tools
        # wait for their own platform's runtime matrix, keeping those builds
        # inside four slots instead of starting seven concurrent build rows.
        validated = {"validate"}
        ready_builds = sum(
            len(rows)
            for block, matrices in ((runtime, runtimes), (tool, tools))
            if job_dependencies(block) <= validated
            for rows in matrices.values()
        )
        self.assertEqual(ready_builds, 4)
        self.assertEqual(sum(len(rows) for rows in tools.values()), 3)

        platform_workflow = REPOSITORY / ".github/workflows/release-platform.yml"
        prebuild = job_block("prebuild", platform_workflow)
        package = job_block("package", platform_workflow)
        self.assertEqual(job_dependencies(prebuild), set())
        self.assertEqual(job_dependencies(package), {"prebuild"})
        for block in (prebuild, package):
            self.assertIn("platform: ${{ inputs.platform }}", block)
            self.assertIn("source-sha: ${{ inputs.source-sha }}", block)
            self.assertIn("tree-sha: ${{ inputs.tree-sha }}", block)
            self.assertIn("version: ${{ inputs.version }}", block)
        self.assertIn("uses: ./.github/workflows/release-prebuild.yml", prebuild)
        self.assertIn("uses: ./.github/workflows/release-build.yml", package)

        slots = {
            platform: max(len(runtimes[platform]), len(tools[platform]), 1)
            for platform in ("linux", "windows", "macos")
        }
        self.assertEqual(slots, {"linux": 1, "windows": 1, "macos": 2})
        self.assertEqual(sum(slots.values()), 4)

    def test_packaged_runtime_retains_the_original_native_build_manifests(self):
        packaging = (REPOSITORY / ".github/workflows/release-build.yml").read_text()
        start = packaging.index("      - name: Stage the packaged runtime for qualification")
        stage = packaging[start:packaging.index("      - name: Qualify the packaged runtime", start)]
        self.assertIn('--parent-manifest "target/release-prebuild/${{ matrix.runtime_artifact }}/manifest.json"', stage)
        self.assertIn('--parent-manifest "target/release-prebuild/${{ matrix.runtime_artifact_2 }}/manifest.json"', stage)
        self.assertIn('"${parent_arguments[@]}"', stage)
        native = stage.split("          else\n", 1)[1].split("          fi\n", 1)[0]
        self.assertIn("parent_arguments=", native)

    def test_windows_native_flags_are_retained_for_manifest_observation(self):
        runtime = job_block("runtime")
        build = runtime.split("      - name: Build the Windows runtime", 1)[1].split("      - name:", 1)[0]
        self.assertIn('echo "CFLAGS_x86_64_pc_windows_msvc=$CFLAGS_x86_64_pc_windows_msvc" >> "$GITHUB_ENV"', build)

    def test_linux_builders_consume_the_published_source_verified_input_cache(self):
        for job in ("tool", "runtime"):
            with self.subTest(job=job):
                source = job_block(job)
                restore = source.index("uses: ./.github/actions/workspace-cache")
                verify = source.index("scripts/ci-workspace-cache.py prepare")
                build = source.index("run: cargo build")
                self.assertLess(restore, verify)
                self.assertLess(verify, build)
                for field in ("lane: landing-linux", "target: target", "ledger: .ci-cache-ledgers/landing.json", "recipe: landing-v1"):
                    self.assertIn(field, source)
                self.assertNotIn("operation: save", source)

    def test_reusable_workflow_accepts_only_release_identity_inputs(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("workflow_call:", workflow)
        for input_name in ("source-sha", "tree-sha", "version", "pr-number", "platform"):
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
            "REQUESTED_PLATFORM: ${{ inputs.platform }}",
            'case "$REQUESTED_PLATFORM" in',
            "linux|windows|macos) ;;",
            "unsupported release platform:",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, validation)

        for name, dependencies in (("tool", {"validate", "runtime"}), ("runtime", {"validate"})):
            with self.subTest(job=name):
                self.assertEqual(job_dependencies(job_block(name)), dependencies)

    def test_build_topology_has_three_host_tools_and_four_native_runtimes(self):
        tool = job_block("tool")
        runtime = job_block("runtime")
        tools = platform_matrices(tool)
        runtimes = platform_matrices(runtime)
        expected_tools = {
            "linux": {
                "name": "linux", "runner": "ubuntu-latest",
                "artifact": "release-prebuild-tool-linux",
                "tool_path": "target/debug/xtask-engine-tools",
                "filename": "xtask-engine-tools", "target": "host",
            },
            "windows": {
                "name": "windows", "runner": "windows-latest",
                "artifact": "release-prebuild-tool-windows",
                "tool_path": "target/debug/xtask-engine-tools.exe",
                "filename": "xtask-engine-tools.exe", "target": "host",
            },
            "macos": {
                "name": "macos", "runner": "macos-latest",
                "artifact": "release-prebuild-tool-macos",
                "tool_path": "target/debug/xtask-engine-tools",
                "filename": "xtask-engine-tools", "target": "host",
            },
        }
        expected_runtimes = {
            "linux": [{
                "name": "linux", "runner": "ubuntu-latest",
                "artifact": "release-prebuild-runtime-linux",
                "source_dir": "target/release", "suffix": "",
                "target": "x86_64-unknown-linux-gnu",
            }],
            "windows": [{
                "name": "windows", "runner": "windows-latest",
                "artifact": "release-prebuild-runtime-windows",
                "source_dir": "target/x86_64-pc-windows-msvc/release", "suffix": ".exe",
                "target": "x86_64-pc-windows-msvc",
            }],
            "macos": [{
                "name": "macos-arm64", "runner": "macos-latest",
                "artifact": "release-prebuild-runtime-macos-arm64",
                "source_dir": "target/aarch64-apple-darwin/release", "suffix": "",
                "target": "aarch64-apple-darwin",
            }, {
                "name": "macos-x86_64", "runner": "macos-latest",
                "artifact": "release-prebuild-runtime-macos-x86_64",
                "source_dir": "target/x86_64-apple-darwin/release", "suffix": "",
                "target": "x86_64-apple-darwin",
            }],
        }
        self.assertEqual(tools, {platform: [row] for platform, row in expected_tools.items()})
        self.assertEqual(runtimes, expected_runtimes)

        self.assertIn("cargo build --profile test --locked -p xtask", tool)
        self.assertIn("--features engine-tools --bin xtask-engine-tools", tool)
        self.assertNotIn("cargo build --release", tool)
        runtime_command = "cargo build --release --locked -p clonk-app -p clonk-game -p clonk-c4group"
        self.assertEqual(runtime.count(runtime_command), 4)
        for target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
            with self.subTest(target=target):
                self.assertIn(f"{runtime_command} --target {target}", runtime)

    def test_prebuilds_preserve_trusted_caches_and_msvc_validation(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        runtime = job_block("runtime")

        for cache in (
            "shared-key: full-parity",
            "shared-key: windows-runtime-msvc-v2",
            "shared-key: recording-host-oracles",
            "shared-key: shipped-msvc-runtime-v1",
        ):
            with self.subTest(cache=cache):
                self.assertIn(cache, workflow)
        self.assertIn("run: scripts/configure-msvc-runtime.sh", runtime)
        self.assertIn("name: Restore trusted-main ThinLTO cache", runtime)
        self.assertIn("clonk-msvc-thinlto-v2-windows-x64-rustc-1.98.1-llvm-22.1.8-", runtime)
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
                    '--provenance-root "$GITHUB_WORKSPACE"',
                ):
                    self.assertIn(fragment, block)

        self.assertIn("--file \"payload/${{ matrix.filename }}\"", tool)
        for binary in ("c4group", "clonk-app", "clonk-game"):
            self.assertIn(f'--file "payload/{binary}${{{{ matrix.suffix }}}}"', runtime)

    def test_matrix_expands_to_exactly_seven_required_run_artifact_handoffs(self):
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
        rows = [
            row
            for name in ("tool", "runtime")
            for matrix in platform_matrices(job_block(name)).values()
            for row in matrix
        ]
        declared = {row["artifact"] for row in rows}

        self.assertEqual(declared, expected)
        self.assertEqual(len(rows), 7)
        self.assertEqual(workflow.count("uses: actions/cache/save@"), 0)
        # The ThinLTO warm start is optional. Required compile handoffs use
        # immutable artifacts scoped to the exact candidate and workflow run.
        self.assertEqual(workflow.count("uses: actions/cache/restore@"), 1)
        self.assertEqual(workflow.count("lookup-only: true"), 0)
        self.assertEqual(workflow.count("fail-on-cache-miss: true"), 0)
        self.assertEqual(
            workflow.count("uses: actions/upload-artifact@"), 2
        )
        self.assertEqual(workflow.count("name: Hand off the"), 2)
        self.assertEqual(workflow.count("path: ${{ matrix.artifact_dir }}"), 0)
        self.assertEqual(workflow.count("path: target/release-prebuild/${{ matrix.artifact }}"), 2)
        self.assertEqual(
            workflow.count(
                "name: ${{ matrix.artifact }}-"
                "${{ inputs.source-sha }}-${{ github.run_id }}"
            ),
            2,
        )
        self.assertEqual(workflow.count("if-no-files-found: error"), 2)
        self.assertEqual(workflow.count("overwrite: true"), 2)
        self.assertEqual(workflow.count("compression-level: 0"), 2)


if __name__ == "__main__":
    unittest.main()
