"""Release validation guards, exercised from the workflow's real shell."""

import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_release_content_handoff import WORKFLOW, step_script
from test_release_prebuild_workflow import platform_matrices

BUILD_WORKFLOW = WORKFLOW.with_name("release-build.yml")
LANDING_WORKFLOW = WORKFLOW.with_name("landing.yml")
PREBUILD_WORKFLOW = WORKFLOW.with_name("release-prebuild.yml")
PLATFORM_WORKFLOW = WORKFLOW.with_name("release-platform.yml")
QUALIFICATION_WORKFLOW = WORKFLOW.with_name("exact-sha-qualification.yml")


def coverage_fragment_suffixes():
    source = QUALIFICATION_WORKFLOW.read_text(encoding="utf-8")
    collectors = source[
        source.index("  coverage-fragments:") : source.index("  coverage:")
    ]
    return re.findall(r"(?m)^            artifact: ([a-z0-9-]+)$", collectors)


def job_block(name):
    source = WORKFLOW.read_text(encoding="utf-8")
    marker = f"\n  {name}:\n"
    start = source.index(marker) + 1
    following = re.compile(r"^  [A-Za-z0-9_-]+:$", re.MULTILINE)
    match = following.search(source, start + 1)
    return source[start : match.start()] if match else source[start:]


def build_job_block(name, workflow=BUILD_WORKFLOW):
    source = workflow.read_text(encoding="utf-8")
    marker = f"\n  {name}:\n"
    start = source.index(marker) + 1
    following = re.compile(r"^  [A-Za-z0-9_-]+:$", re.MULTILINE)
    match = following.search(source, start + 1)
    return source[start : match.start()] if match else source[start:]


def build_step_script(name, workflow=BUILD_WORKFLOW):
    lines = workflow.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(
            f"{workflow.name} has no step named {name!r}"
        ) from None

    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line.startswith("      - "):
            break
        if line == "        run: |":
            body = []
            for candidate in lines[index + 1 :]:
                if candidate.strip() and not candidate.startswith(" " * 10):
                    break
                body.append(candidate[10:])
            return "\n".join(body)
    raise AssertionError(f"step {name!r} has no `run: |` block")


def publication_step_block(name):
    publish = job_block("publish")
    start = publish.index(f"      - name: {name}\n")
    following = re.search(r"(?m)^      - ", publish[start + 1:])
    return publish[start:start + 1 + following.start()] if following else publish[start:]


class ReleaseWorkflowTopologyTests(unittest.TestCase):
    def test_publish_promotes_the_successful_exact_sha_landing_artifacts(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        publish = job_block("publish")

        self.assertNotIn("\n  resolve:\n", workflow)
        self.assertNotIn("\n  build:\n", workflow)
        self.assertIn("Decide whether this commit releases", publish)
        self.assertIn("Resolve exact-SHA release artifacts", publish)
        self.assertNotIn("rust.yml", publish)
        self.assertNotIn("needs:", publish)
        self.assertIn("actions: read", publish)
        for fragment in (
            "github-token: ${{ github.token }}",
            "repository: ${{ github.repository }}",
            "run-id: ${{ steps.artifacts.outputs.run-id }}",
        ):
            with self.subTest(fragment=fragment):
                self.assertEqual(publish.count(fragment), 3)

    def test_release_artifact_build_is_a_reusable_workflow(self):
        landing = WORKFLOW.with_name("landing.yml").read_text(encoding="utf-8")

        self.assertTrue(BUILD_WORKFLOW.exists())
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("workflow_call:", reusable)
        self.assertIn("source-sha:", reusable)
        self.assertIn("tree-sha:", reusable)
        self.assertIn("version:", reusable)
        platform = PLATFORM_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("uses: ./.github/workflows/release-platform.yml", landing)
        self.assertIn("uses: ./.github/workflows/release-build.yml", platform)
        self.assertIn("uses: ./.github/workflows/release-prebuild.yml", platform)
        self.assertIn("source-sha: ${{ github.sha }}", landing)
        for input_name in ("source-sha", "tree-sha", "version", "platform"):
            with self.subTest(input_name=input_name):
                self.assertEqual(
                    platform.count(f"{input_name}: ${{{{ inputs.{input_name} }}}}"),
                    2,
                )

    def test_reusable_release_build_validates_the_requested_source_version(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("name: Validate the release source", reusable)
        self.assertIn("REQUESTED_VERSION: ${{ inputs.version }}", reusable)
        self.assertIn('[[ "$SOURCE_SHA" != "$MERGE_SHA" ]]', reusable)
        self.assertIn('workspace version ${actual_version}', reusable)
        self.assertIn('requested release ${REQUESTED_VERSION}', reusable)

    def test_merge_group_release_build_uses_exact_rerunnable_handoff_artifacts(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        prebuild = PREBUILD_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("shared-key: release-${{", prebuild)
        for trusted_cache in (
            "full-parity",
            "windows-runtime-msvc-v2",
            "recording-host-oracles",
        ):
            with self.subTest(trusted_cache=trusted_cache):
                self.assertIn(f"shared-key: {trusted_cache}", prebuild)
        self.assertIn("actions/upload-artifact@", prebuild)
        self.assertIn("actions/download-artifact@", reusable)
        self.assertNotIn("fail-on-cache-miss: true", reusable)
        # Three public upload steps expand to seven public assets. The fourth
        # retains the final qualified runtime for each platform privately.
        self.assertEqual(reusable.count("compression-level: 0"), 4)
        self.assertEqual(reusable.count("overwrite: true"), 4)

    def test_release_build_parallelizes_tools_runtimes_and_macos_architectures(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        prebuild = PREBUILD_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("\n  tool:\n", prebuild)
        self.assertIn("\n  runtime:\n", prebuild)
        self.assertIn("\n  package:\n", reusable)
        package = build_job_block("package")
        self.assertNotIn("needs: [tool, runtime]", package)
        release_build = build_job_block("release-build", LANDING_WORKFLOW)
        self.assertIn("needs: release-context", release_build)
        self.assertIn("platform: [linux, windows, macos]", release_build)
        self.assertNotIn("release-prebuild", release_build)
        self.assertIn("platform: ${{ matrix.platform }}", release_build)
        own_package = build_job_block("package", PLATFORM_WORKFLOW)
        self.assertIn("needs: prebuild", own_package)
        self.assertNotIn("needs: [", own_package)
        runtimes = platform_matrices(build_job_block("runtime", PREBUILD_WORKFLOW))
        self.assertEqual(
            [row["name"] for row in runtimes["macos"]],
            ["macos-arm64", "macos-x86_64"],
        )
        self.assertIn("--target aarch64-apple-darwin", prebuild)
        self.assertIn("--target x86_64-apple-darwin", prebuild)
        self.assertIn("--skip-build", package)

    def test_release_build_handoffs_are_exact_run_scoped_artifacts(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        prebuild = PREBUILD_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("\n  package:\n", reusable)
        package = build_job_block("package")

        self.assertIn(
            "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
            prebuild,
        )
        self.assertIn(
            "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
            package,
        )
        for fragment in (
            "${{ inputs.source-sha }}",
            "${{ github.run_id }}",
            "if-no-files-found: error",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, prebuild + reusable)
        self.assertNotIn("restore-keys:", prebuild + reusable)
        self.assertNotIn("github.run_attempt", reusable)

        # Required compile inputs and final qualification inputs are immutable
        # artifacts, separate from the three public payload upload steps.
        self.assertEqual(reusable.count("actions/upload-artifact@"), 4)
        self.assertIn(
            "name: release-qualified-runtime-${{ matrix.name }}-"
            "${{ inputs.source-sha }}-${{ github.run_id }}",
            package,
        )

    def test_each_package_consumes_only_its_platform_producers(self):
        packages = platform_matrices(build_job_block("package"))
        tools = platform_matrices(build_job_block("tool", PREBUILD_WORKFLOW))
        runtimes = platform_matrices(build_job_block("runtime", PREBUILD_WORKFLOW))
        for platform in ("linux", "windows", "macos"):
            with self.subTest(platform=platform):
                self.assertEqual(len(packages[platform]), 1)
                package = packages[platform][0]
                self.assertEqual(package["name"], platform)
                self.assertEqual(package["artifact"], f"desktop-{platform}")
                self.assertEqual(package["tool_artifact"], tools[platform][0]["artifact"])
                self.assertEqual(package["tool_filename"], tools[platform][0]["filename"])
                self.assertEqual(package["tool_path"], tools[platform][0]["tool_path"])
                self.assertEqual(package["runtime_suffix"], runtimes[platform][0]["suffix"])
                self.assertEqual(package["runtime_artifact"], runtimes[platform][0]["artifact"])
                self.assertEqual(package["runtime_target"], runtimes[platform][0]["target"])
                if platform == "macos":
                    self.assertEqual(package["runtime_artifact_2"], runtimes[platform][1]["artifact"])
                    self.assertEqual(package["runtime_target_2"], runtimes[platform][1]["target"])
                else:
                    self.assertNotIn("runtime_artifact_2", package)
                    self.assertNotIn("runtime_target_2", package)

    def test_device_loss_qualifies_the_final_packaged_bytes_before_public_uploads(self):
        package = build_job_block("package")
        staging = build_step_script("Stage the packaged runtime for qualification")
        stages = [
            "Check the macOS build is universal",
            "Stage the packaged runtime for qualification",
            "Qualify the packaged runtime",
            "Retain the qualified runtime and manifest",
            "${{ matrix.artifact }}",
        ]
        for before, after in zip(stages, stages[1:]):
            with self.subTest(before=before, after=after):
                self.assertLess(package.index(f"name: {before}"), package.index(f"name: {after}"))
        for fragment in (
            # Packaging moves the bundle into the disk image and deletes it, so
            # macOS stages the signed universal payload from its engine component.
            'unzip -q "${engines[0]}" -d target/dist/qualification-unpack',
            "source_root=target/dist/qualification-unpack/Contents/MacOS",
            "target_arguments=(--target universal-apple-darwin)",
            "source_root=target/dist/clonk-rust/bin",
            'cmp "target/release-prebuild/${{ matrix.runtime_artifact }}/payload/$filename"',
            '--provenance-root "$GITHUB_WORKSPACE"',
            '--head-sha "$SOURCE_SHA" --tree-sha "$TREE_SHA" --version "$VERSION"',
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, staging)
        for binary in ("c4group", "clonk-app", "clonk-game"):
            self.assertIn(f"--file 'payload/{binary}${{{{ matrix.runtime_suffix }}}}'", staging)
        self.assertIn("uses: ./.github/actions/device-loss", package)
        self.assertIn("prebuilt-root: target/release-qualified/${{ matrix.name }}", package)
        self.assertNotIn("cargo build", package)

    def test_release_resolver_coverage_inventory_matches_collectors(self):
        resolver = step_script("Resolve exact-SHA release artifacts")
        match = re.search(
            r"coverage_fragment_suffixes=\(\n(?P<body>.*?)\n\)",
            resolver,
            re.S,
        )
        self.assertIsNotNone(match)
        allowlisted = re.findall(r"(?m)^\s+([a-z0-9-]+)$", match.group("body"))

        self.assertCountEqual(allowlisted, coverage_fragment_suffixes())
        self.assertIn(
            'expected_artifacts+=("rust-coverage-fragment-${run_id}-${suffix}")',
            resolver,
        )
        self.assertNotIn("is_coverage_fragment", resolver)

    def test_publication_download_allowlist_contains_only_the_seven_public_payloads(self):
        resolver = step_script("Resolve exact-SHA release artifacts")
        match = re.search(r"expected_payloads=\(\n(?P<body>.*?)\n\)", resolver, re.S)
        self.assertIsNotNone(match)
        payloads = re.findall(r"(?m)^\s+([a-z0-9-]+)$", match.group("body"))
        rows = [matrix[0] for matrix in platform_matrices(build_job_block("package")).values()]
        expected = [row["artifact"] for row in rows]
        expected += [f"components-{row['artifact']}" for row in rows]
        expected.append("release-tool")
        self.assertCountEqual(payloads, expected)
        self.assertEqual(len(payloads), 7)

        publish = job_block("publish")
        download_blocks = re.findall(
            r"(?m)^      - uses: actions/download-artifact@.*\n"
            r"(?:(?:        .*|\s*)\n)*",
            publish,
        )
        self.assertEqual(len(download_blocks), 3)
        selectors = [
            match.group(1)
            for block in download_blocks
            for match in re.finditer(r"(?m)^          (?:pattern|name): (.+)$", block)
        ]
        self.assertCountEqual(selectors, ["desktop-*", "components-*", "release-tool"])

    def test_publication_verifies_the_authoritative_receipt_before_downloads(self):
        publish = job_block("publish")
        stages = [
            "Resolve exact-SHA release artifacts",
            "Materialize verified qualification inputs",
            "Verify exact-SHA qualification receipt",
        ]
        for before, after in zip(stages, stages[1:]):
            with self.subTest(before=before, after=after):
                self.assertLess(publish.index(f"name: {before}"), publish.index(f"name: {after}"))
        self.assertLess(publish.index("name: Verify exact-SHA qualification receipt"),
                        publish.index("uses: actions/download-artifact@"))
        self.assertIn("uses: ./.github/actions/verified-content", publish)
        self.assertNotIn("continue-on-error: true",
                         publication_step_block("Verify exact-SHA qualification receipt"))
        script = step_script("Verify exact-SHA qualification receipt")
        self.assertIn("scripts/release-qualification-evidence.py resolve", script)
        self.assertIn('--source-sha "$CI_SHA"', script)
        self.assertIn('--run-id "$QUALIFICATION_RUN_ID"', script)
        self.assertNotIn("continue-on-error", script)

    def test_publication_latency_reporting_is_advisory_and_uses_operation_clocks(self):
        report = job_block("latency-report")
        self.assertIn("name: CI latency report", report)
        self.assertIn("needs: publish", report)
        self.assertIn("    continue-on-error: true\n", report)
        self.assertIn("if: always()", report)
        self.assertIn("actions: read", report)
        permissions = re.search(r"    permissions:\n(?P<body>(?:      .*\n)+)", report)
        self.assertIsNotNone(permissions)
        self.assertNotIn(": write", permissions.group("body"))
        for fragment in (
            "scripts/ci-latency-report.py", '--run-id "$GITHUB_RUN_ID"',
            "--phase publication", "--allow-running true", "--history-limit 20",
            "name: ci-latency-${{ github.run_id }}-${{ github.run_attempt }}",
            "if-no-files-found: warn",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, report)
        self.assertNotIn("latency-report", job_block("publish"))

    def test_only_publication_clock_measurement_is_advisory_inside_the_publisher(self):
        publish = job_block("publish")
        self.assertNotRegex(publish, r"(?m)^    continue-on-error: true$")
        measurement = publication_step_block("Enforce release publication SLO")
        self.assertIn("        continue-on-error: true\n", measurement)
        # The measurement may fail its script on missing or invalid clocks.
        # Its optional UI step cannot mask required promotion failures.
        for name in (
            "Decide whether this commit releases", "Resolve exact-SHA release artifacts",
            "Materialize verified qualification inputs", "Verify exact-SHA qualification receipt",
            "Resolve the published content release", "Verify the published content archive",
            "Generate the update manifest", "Tag the built commit", "Publish the release",
        ):
            with self.subTest(step=name):
                self.assertNotIn("continue-on-error: true", publication_step_block(name))

    def test_release_packaging_tool_uses_the_non_lto_test_profile(self):
        prebuild = PREBUILD_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("\n  tool:\n", prebuild)
        tool = build_job_block("tool", PREBUILD_WORKFLOW)

        self.assertIn(
            "cargo build --profile test --locked -p xtask --features engine-tools ",
            tool,
        )
        self.assertIn("--bin xtask-engine-tools", tool)
        self.assertNotIn("cargo build --release", tool)
        self.assertIn("target/debug/xtask-engine-tools", tool)
        self.assertNotIn("target/test/xtask-engine-tools", prebuild)

    def test_whole_candidate_latency_report_uses_the_complete_landing_run(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        self.assertNotIn("\n  latency:\n", reusable)
        report = build_job_block("latency-report", LANDING_WORKFLOW)
        self.assertIn("name: CI latency report", report)
        self.assertIn("needs: [landing-gate, release-context]", report)
        self.assertIn("actions: read", report)
        self.assertIn("scripts/ci-latency-report.py", report)
        for fragment in (
            '--run-id "$GITHUB_RUN_ID"',
            "--phase landing",
            "--allow-running true",
            "--release \"${IS_RELEASE:-false}\"",
            "IS_RELEASE: ${{ needs.release-context.outputs.release }}",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, report)
        self.assertNotIn("jobs?filter=latest", report)

    def test_latency_reporting_is_advisory_after_the_functional_landing_gate(self):
        gate = build_job_block("landing-gate", LANDING_WORKFLOW)
        report = build_job_block("latency-report", LANDING_WORKFLOW)
        self.assertNotIn("MAX_RELEASE_BUILD_SECONDS", gate)
        self.assertNotIn("Enforce release build latency", gate)
        self.assertNotIn("latency-report", gate)
        self.assertIn("continue-on-error: true", report)
        self.assertIn("if: always() && github.event_name != 'pull_request'", report)
        for required in (
            "release-build", "release-qualification", "release-evidence", "linux", "windows-smoke",
        ):
            with self.subTest(required=required):
                self.assertIn(f"      - {required}\n", gate)

    def test_latency_report_retains_run_and_attempt_scoped_evidence(self):
        report = build_job_block("latency-report", LANDING_WORKFLOW)
        self.assertIn("actions/upload-artifact@", report)
        self.assertIn("if: always()", report)
        self.assertIn("name: ci-latency-${{ github.run_id }}-${{ github.run_attempt }}", report)
        self.assertIn("if-no-files-found: warn", report)
        self.assertIn('--output "$RUNNER_TEMP/ci-latency.json"', report)
        self.assertIn("path: ${{ runner.temp }}/ci-latency.json", report)

    def test_release_commits_have_a_sha_specific_concurrency_lane(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn(
            'group: "release-${{ inputs.release-sha || (startsWith(github.event.head_commit.message, '
            "'chore: release ') && github.sha || 'rolling') }}\"",
            workflow,
        )
        self.assertIn("cancel-in-progress: false", workflow)

    def test_publish_uses_a_shallow_checkout_and_the_tag_api(self):
        publish = job_block("publish")

        self.assertIn("fetch-depth: 1", publish)
        self.assertIn('git/ref/tags/v${version}', publish)
        self.assertNotIn('git rev-parse -q --verify "refs/tags/', publish)

    def test_publication_reports_its_original_two_minute_slo(self):
        publish = job_block("publish")

        self.assertIn("name: Enforce release publication SLO", publish)
        self.assertIn("releases/tags/v${VERSION}", publish)
        self.assertIn("commits/${SHA}/pulls", publish)
        self.assertIn(".merge_commit_sha == $sha", publish)
        self.assertIn(".merged_at", publish)
        self.assertIn('if [[ "$elapsed" -gt 120 ]]', publish)
        self.assertIn("CI_POLL_ATTEMPTS: '5'", publish)
        self.assertIn("CI_POLL_SECONDS: '1'", publish)

    def test_already_published_rerun_preserves_the_original_publication_clock(self):
        resolve = step_script("Decide whether this commit releases")
        published_noop = resolve.rindex('echo "release=false" >> "$GITHUB_OUTPUT"')

        self.assertLess(
            resolve.index('echo "version=$version" >> "$GITHUB_OUTPUT"'),
            published_noop,
        )
        self.assertLess(
            resolve.index('echo "sha=$RESOLVED_SHA" >> "$GITHUB_OUTPUT"'),
            published_noop,
        )
        self.assertIn(
            "if: steps.resolve.outputs.version != ''",
            job_block("publish"),
        )

    def test_partial_publication_can_resume_without_persisted_git_credentials(self):
        publish = job_block("publish")

        self.assertIn('"repos/${REPOSITORY}/releases/tags/v${version}"', publish)
        self.assertIn("--jq '.draft'", publish)
        self.assertIn("'(HTTP 404)'", publish)
        self.assertIn('if [[ "$release_state" == "false" ]]', publish)
        self.assertIn("persist-credentials: false", publish)
        self.assertIn('gh release create "v${VERSION}" --draft', publish)
        self.assertIn('gh release edit "v${VERSION}" --draft=false --latest', publish)


@unittest.skipUnless(shutil.which("bash"), "needs bash")
@unittest.skipUnless(shutil.which("jq"), "needs jq, as the ubuntu runner has")
class ReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.bin = self.root / "bin"
        self.bin.mkdir()

    def _stub(self, body):
        path = self.bin / "gh"
        path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def run_artifact_resolver(self, **extra):
        output = self.root / "github-output"
        output.unlink(missing_ok=True)
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "GH_TOKEN": "stub",
            "CI_SHA": "0123456789abcdef",
            "REPOSITORY": "clonk-org/clonk-rs",
            "CI_POLL_ATTEMPTS": "3",
            "CI_POLL_SECONDS": "0",
            "GITHUB_OUTPUT": str(output),
            **extra,
        }
        completed = subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                step_script("Resolve exact-SHA release artifacts"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )
        return completed, output.read_text(encoding="utf-8") if output.exists() else ""

    def run_publication_slo(self, elapsed_seconds, event_name="push"):
        self._stub(
            'if [[ "$*" == "api repos/${REPOSITORY}/commits/${SHA}/pulls" ]]; then\n'
            '  printf \'[{"merge_commit_sha":"%s","merged_at":"%s"}]\\n\' "$SHA" "$LANDED_AT"\n'
            'elif [[ "$*" == "api repos/${REPOSITORY}/releases/tags/v${VERSION} --jq .published_at" ]]; then\n'
            '  printf \'%s\\n\' "$PUBLISHED_AT"\n'
            "else exit 1; fi\n"
        )
        date = self.bin / "date"
        date.write_text(
            "#!/usr/bin/env bash\n"
            'if [[ "$*" == "-u -d ${LANDED_AT} +%s" ]]; then\n'
            "  echo 0\n"
            'elif [[ "$*" == "-u -d ${PUBLISHED_AT} +%s" ]]; then\n'
            '  echo "$ELAPSED_SECONDS"\n'
            "else exit 1; fi\n",
            encoding="utf-8",
        )
        date.chmod(0o755)
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "ELAPSED_SECONDS": str(elapsed_seconds),
            "EVENT_NAME": event_name,
            "GH_TOKEN": "stub",
            "LANDED_AT": "2026-08-09T10:30:55Z",
            "PUBLISHED_AT": "2026-08-09T10:32:55Z",
            "REPOSITORY": "clonk-org/clonk-rs",
            "SHA": "0123456789abcdef",
            "VERSION": "0.9.4",
        }
        return subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                step_script("Enforce release publication SLO"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )

    def run_release_resolver(self, **extra):
        (self.root / "Cargo.toml").write_text(
            '[workspace.package]\nversion = "0.28.0"\n', encoding="utf-8"
        )
        git = self.bin / "git"
        git.write_text(
            "#!/usr/bin/env bash\n"
            'case "$*" in\n'
            '  "show -s --format=%s HEAD") echo "$CHECKOUT_SUBJECT" ;;\n'
            '  "rev-parse HEAD") echo "$CHECKOUT_SHA" ;;\n'
            "  *) exit 1 ;;\nesac\n",
            encoding="utf-8",
        )
        git.chmod(0o755)
        self._stub(
            'case "$*" in\n'
            '  "api repos/${REPOSITORY}/compare/${RESOLVED_SHA}...main --jq .status")\n'
            '    echo "$COMPARISON" ;;\n'
            '  "api repos/${REPOSITORY}/git/ref/tags/v0.28.0 "*)\n'
            '    if [[ -n "${TAG_OBJECT:-}" ]]; then\n'
            '      printf "%s\\n" "$TAG_OBJECT"\n'
            '    else echo "gh: Not Found (HTTP 404)" >&2; exit 1; fi ;;\n'
            "  *) exit 1 ;;\nesac\n"
        )
        output = self.root / "github-output"
        output.unlink(missing_ok=True)
        sha = "0123456789abcdef0123456789abcdef01234567"
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "HEAD_SUBJECT": "",
            "EVENT_NAME": "workflow_dispatch",
            "RESOLVED_SHA": sha,
            "CHECKOUT_SHA": sha,
            "CHECKOUT_SUBJECT": "chore: release 0.28.0 (#1661)",
            "COMPARISON": "ahead",
            "REPOSITORY": "clonk-org/clonk-rs",
            "RUNNER_TEMP": str(self.root),
            "GITHUB_OUTPUT": str(output),
            **extra,
        }
        completed = subprocess.run(
            [
                "bash", "--noprofile", "--norc", "-eo", "pipefail", "-c",
                step_script("Decide whether this commit releases"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )
        return completed, output.read_text(encoding="utf-8") if output.exists() else ""

    def run_qualification_receipt_guard(self, document, *, helper_status=0):
        scripts = self.root / "scripts"
        scripts.mkdir(exist_ok=True)
        helper = scripts / "release-qualification-evidence.py"
        helper.write_text(
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "Path(os.environ['HELPER_LOG']).write_text(json.dumps(sys.argv[1:]))\n"
            "Path(sys.argv[sys.argv.index('--output') + 1]).write_text(os.environ['RECEIPT_RESULT'])\n"
            "sys.exit(int(os.environ['HELPER_STATUS']))\n",
            encoding="utf-8",
        )
        environment = {
            **os.environ, "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "CI_SHA": "a" * 40, "QUALIFICATION_RUN_ID": "91", "GITHUB_RUN_ID": "92",
            "REPOSITORY": "clonk-org/clonk-rs", "RUNNER_TEMP": str(self.root),
            "RECEIPT_RESULT": document if isinstance(document, str) else json.dumps(document),
            "HELPER_STATUS": str(helper_status), "HELPER_LOG": str(self.root / "receipt-helper.json"),
        }
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c",
             step_script("Verify exact-SHA qualification receipt")],
            cwd=self.root, env=environment, capture_output=True, text=True,
        )

    def test_receipt_guard_requires_the_exact_resolved_producer_and_source(self):
        completed = self.run_qualification_receipt_guard(
            {"reused": True, "run_id": 91, "source_sha": "a" * 40}
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        arguments = json.loads((self.root / "receipt-helper.json").read_text())
        self.assertEqual(arguments[0], "resolve")
        self.assertEqual(arguments[arguments.index("--run-id") + 1], "91")
        self.assertEqual(arguments[arguments.index("--source-sha") + 1], "a" * 40)
        self.assertEqual(arguments[arguments.index("--repository") + 1], "clonk-org/clonk-rs")

    def test_receipt_guard_fails_closed_on_fallback_mismatch_and_malformed_results(self):
        cases = [
            {"reused": False, "run_id": 91, "source_sha": "a" * 40},
            {"reused": "true", "run_id": 91, "source_sha": "a" * 40},
            {"reused": True, "run_id": 90, "source_sha": "a" * 40},
            {"reused": True, "run_id": "91", "source_sha": "a" * 40},
            {"reused": True, "run_id": 91, "source_sha": "f" * 40},
            {"reused": True}, "malformed json",
        ]
        for document in cases:
            with self.subTest(document=document):
                completed = self.run_qualification_receipt_guard(document)
                self.assertNotEqual(completed.returncode, 0)
        completed = self.run_qualification_receipt_guard(
            {"reused": True, "run_id": 91, "source_sha": "a" * 40}, helper_status=42,
        )
        self.assertNotEqual(completed.returncode, 0)

    def test_recovery_rejects_an_ordinary_commit(self):
        completed, output = self.run_release_resolver(
            CHECKOUT_SUBJECT="fix: ordinary change"
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertNotIn("release=true", output)
        self.assertIn("not a release commit", completed.stderr)

    def test_recovery_requires_the_exact_commit_to_have_landed_on_main(self):
        for comparison in ("behind", "diverged", "", "unknown"):
            with self.subTest(comparison=comparison):
                completed, output = self.run_release_resolver(COMPARISON=comparison)
                self.assertNotEqual(completed.returncode, 0)
                self.assertNotIn("release=true", output)
                self.assertIn("not on main", completed.stderr)

    def test_release_eligibility_rejects_a_tag_bound_to_another_source_commit(self):
        completed, output = self.run_release_resolver(TAG_OBJECT="commit\t" + "f" * 40)
        self.assertNotEqual(completed.returncode, 0)
        self.assertNotIn("release=true", output)
        self.assertIn("v0.28.0 points at", completed.stderr)

    def test_recovery_rejects_mutable_refs_and_a_different_checkout(self):
        cases = [
            {"RESOLVED_SHA": "main"},
            {"RESOLVED_SHA": "01234567"},
            {"CHECKOUT_SHA": "f" * 40},
        ]
        for case in cases:
            with self.subTest(case=case):
                completed, output = self.run_release_resolver(**case)
                self.assertNotEqual(completed.returncode, 0)
                self.assertNotIn("release=true", output)
                self.assertIn("exact commit SHA", completed.stderr)

    def test_recovery_resolves_the_original_release_version_and_sha(self):
        for comparison in ("ahead", "identical"):
            with self.subTest(comparison=comparison):
                completed, output = self.run_release_resolver(COMPARISON=comparison)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertEqual(
                    output,
                    "version=0.28.0\n"
                    "sha=0123456789abcdef0123456789abcdef01234567\n"
                    "release=true\n",
                )

    def test_ordinary_main_push_does_not_release(self):
        completed, output = self.run_release_resolver(
            EVENT_NAME="push", HEAD_SUBJECT="fix: ordinary change"
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(output, "release=false\n")

    @staticmethod
    def artifact_inventory(*, omit=None, extra=None, expired=None):
        release_names = [
            "desktop-linux",
            "desktop-windows",
            "desktop-macos",
            "components-desktop-linux",
            "components-desktop-windows",
            "components-desktop-macos",
            "release-tool",
        ]
        coverage_names = [
            f"rust-coverage-fragment-91-{suffix}"
            for suffix in coverage_fragment_suffixes()
        ]
        prebuild_names = [
            f"release-prebuild-{kind}-{platform}-0123456789abcdef-91"
            for kind, platforms in (
                ("tool", ("linux", "windows", "macos")),
                ("runtime", ("linux", "windows", "macos-arm64", "macos-x86_64")),
            )
            for platform in platforms
        ]
        qualified_names = [
            f"release-qualified-runtime-{platform}-0123456789abcdef-91"
            for platform in ("linux", "windows", "macos")
        ]
        receipt_names = ["release-qualification-evidence-0123456789abcdef"]
        names = release_names + coverage_names + prebuild_names + qualified_names + receipt_names
        artifacts = [
            {"name": name, "expired": name == expired}
            for name in names
            if name != omit
        ] + ([{"name": extra, "expired": False}] if extra else [])
        return json.dumps({"total_count": len(artifacts), "artifacts": artifacts})

    def paginated_artifact_inventory(self):
        artifacts = json.loads(self.artifact_inventory())["artifacts"]
        receipt = artifacts.pop()
        # Previous failed attempts can retain enough diagnostics to push the
        # authoritative receipt onto a later page. They remain non-public.
        artifacts += [
            {"name": f"rust-test-diagnostics-91-{attempt}-app-1-0123456789abcdef",
             "expired": False}
            for attempt in range(1, 86)
        ]
        artifacts += [
            {"name": f"{kind}-91-{attempt}", "expired": False}
            for kind in ("ci-latency", "presentation-capture-failure")
            for attempt in (1, 2)
        ]
        artifacts.append(receipt)
        return [{"total_count": len(artifacts), "artifacts": artifacts[start:start + 100]}
                for start in range(0, len(artifacts), 100)]

    def test_resolver_paginates_the_complete_inventory_across_rerun_diagnostics(self):
        self._stub(
            'echo "$*" >> "$GH_LOG"\n'
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then printf "%s\\n" "$ARTIFACTS";\n'
            "else exit 1; fi\n"
        )
        pages = self.paginated_artifact_inventory()
        self.assertGreater(pages[0]["total_count"], 100)
        self.assertEqual(pages[-1]["artifacts"][-1]["name"],
                         "release-qualification-evidence-0123456789abcdef")
        log = self.root / "pagination.log"
        completed, output = self.run_artifact_resolver(GH_LOG=str(log), ARTIFACTS=json.dumps(pages))
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(output, "run-id=91\n")
        self.assertIn("--paginate --slurp", log.read_text())

    def test_resolver_rejects_incomplete_changed_or_expired_later_artifact_pages(self):
        self._stub(
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then printf "%s\\n" "$ARTIFACTS";\n'
            "else exit 1; fi\n"
        )
        cases = {}
        pages = self.paginated_artifact_inventory()
        cases["truncated"] = pages[:1]
        pages = self.paginated_artifact_inventory()
        pages[-1]["total_count"] += 1
        cases["changed count"] = pages
        pages = self.paginated_artifact_inventory()
        pages[-1]["artifacts"][-1]["expired"] = True
        cases["expired receipt on second page"] = pages
        pages = self.paginated_artifact_inventory()
        pages[-1]["artifacts"].append({"name": "unreviewed-payload", "expired": False})
        for page in pages:
            page["total_count"] += 1
        cases["unreviewed payload on second page"] = pages
        for name, pages in cases.items():
            with self.subTest(case=name):
                completed, output = self.run_artifact_resolver(
                    ARTIFACTS=json.dumps(pages), CI_POLL_ATTEMPTS="1",
                )
                self.assertNotEqual(completed.returncode, 0)
                self.assertEqual(output, "")
                self.assertIn("timed out", completed.stderr)

    def test_resolver_accepts_only_an_exact_successful_landing_inventory(self):
        self._stub(
            'echo "$*" >> "$GH_LOG"\n'
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then\n'
            '  printf "%s\\n" "$ARTIFACTS"\n'
            "else exit 1; fi\n"
        )
        log = self.root / "gh.log"
        completed, output = self.run_artifact_resolver(
            GH_LOG=str(log), ARTIFACTS=self.artifact_inventory()
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(output, "run-id=91\n")
        commands = log.read_text(encoding="utf-8")
        self.assertIn("--workflow landing.yml", commands)
        self.assertIn("--event merge_group", commands)
        self.assertIn("--commit 0123456789abcdef", commands)
        self.assertNotIn("--branch", commands)

    def test_resolver_ignores_only_current_run_scoped_diagnostics(self):
        self._stub(
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then\n'
            '  printf "%s\\n" "$ARTIFACTS"\n'
            "else exit 1; fi\n"
        )
        for diagnostic in (
            "rust-test-diagnostics-91-1-app-1-0123456789abcdef",
            "ci-latency-91-1",
            "presentation-capture-failure-91-1",
        ):
            with self.subTest(diagnostic=diagnostic):
                completed, output = self.run_artifact_resolver(
                    ARTIFACTS=self.artifact_inventory(extra=diagnostic)
                )
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertEqual(output, "run-id=91\n")

    def test_resolver_accepts_device_loss_diagnostics_for_the_release_commit(self):
        self._stub(
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then printf "%s\\n" "$ARTIFACTS";\n'
            "else exit 1; fi\n"
        )
        inventory = json.loads(self.artifact_inventory())
        inventory["artifacts"].extend(
            {"name": f"device-loss-{platform}-0123456789abcdef", "expired": False}
            for platform in ("Linux", "Windows", "macOS")
        )
        inventory["total_count"] = len(inventory["artifacts"])
        completed, output = self.run_artifact_resolver(
            ARTIFACTS=json.dumps(inventory), CI_POLL_ATTEMPTS="1"
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(output, "run-id=91\n")

    def test_resolver_fails_closed_on_missing_or_expired_coverage_artifacts(self):
        self._stub(
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then printf "%s\\n" "$ARTIFACTS";\n'
            "else exit 1; fi\n"
        )
        cases = {
            "missing": self.artifact_inventory(
                omit="rust-coverage-fragment-91-app-1-10"
            ),
            "expired": self.artifact_inventory(
                expired="rust-coverage-fragment-91-app-1-10"
            ),
            "stale run": self.artifact_inventory(
                omit="rust-coverage-fragment-91-app-1-10",
                extra="rust-coverage-fragment-90-app-1-10",
            ),
        }
        for name, inventory in cases.items():
            with self.subTest(name=name):
                completed, output = self.run_artifact_resolver(
                    ARTIFACTS=inventory, CI_POLL_ATTEMPTS="1"
                )
                self.assertNotEqual(completed.returncode, 0)
                self.assertEqual(output, "")
                self.assertIn("timed out", completed.stderr)

    def test_resolver_requires_every_prebuilt_and_qualified_runtime_and_receipt(self):
        self._stub(
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then printf "%s\\n" "$ARTIFACTS";\n'
            "else exit 1; fi\n"
        )
        required = [
            f"release-prebuild-{kind}-{platform}-0123456789abcdef-91"
            for kind, platforms in (
                ("tool", ("linux", "windows", "macos")),
                ("runtime", ("linux", "windows", "macos-arm64", "macos-x86_64")),
            )
            for platform in platforms
        ]
        required += [
            f"release-qualified-runtime-{platform}-0123456789abcdef-91"
            for platform in ("linux", "windows", "macos")
        ]
        required.append("release-qualification-evidence-0123456789abcdef")
        for name in required:
            for failure in ("omit", "expired"):
                with self.subTest(artifact=name, failure=failure):
                    completed, output = self.run_artifact_resolver(
                        ARTIFACTS=self.artifact_inventory(**{failure: name}),
                        CI_POLL_ATTEMPTS="1",
                    )
                    self.assertNotEqual(completed.returncode, 0)
                    self.assertEqual(output, "")
                    self.assertIn("timed out", completed.stderr)

    def test_resolver_fails_closed_on_incomplete_or_unexpected_inventory(self):
        self._stub(
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then printf "%s\\n" "$ARTIFACTS";\n'
            "else exit 1; fi\n"
        )
        cases = {
            "missing": self.artifact_inventory(omit="release-tool"),
            "expired": self.artifact_inventory(expired="desktop-linux"),
            "unexpected": self.artifact_inventory(extra="unreviewed-payload"),
            "unknown coverage": self.artifact_inventory(
                extra="rust-coverage-fragment-91-unreviewed"
            ),
            "stale device loss": self.artifact_inventory(
                extra="device-loss-Linux-different-commit"
            ),
            "unknown device loss": self.artifact_inventory(
                extra="device-loss-unreviewed-0123456789abcdef"
            ),
            "unknown qualified runtime": self.artifact_inventory(
                extra="release-qualified-runtime-unreviewed-0123456789abcdef-91"
            ),
            "stale qualified runtime": self.artifact_inventory(
                omit="release-qualified-runtime-linux-0123456789abcdef-91",
                extra="release-qualified-runtime-linux-0123456789abcdef-90",
            ),
            "stale receipt": self.artifact_inventory(
                omit="release-qualification-evidence-0123456789abcdef",
                extra="release-qualification-evidence-different-commit",
            ),
            "stale latency diagnostics": self.artifact_inventory(extra="ci-latency-90-1"),
            "stale capture diagnostics": self.artifact_inventory(
                extra="presentation-capture-failure-90-1"
            ),
        }
        for name, inventory in cases.items():
            with self.subTest(name=name):
                completed, output = self.run_artifact_resolver(
                    ARTIFACTS=inventory, CI_POLL_ATTEMPTS="1"
                )
                self.assertNotEqual(completed.returncode, 0)
                self.assertEqual(output, "")
                self.assertIn("timed out", completed.stderr)

    def test_resolver_retries_artifact_api_visibility(self):
        self._stub(
            'if [[ "$1 $2" == "run list" ]]; then\n'
            '  printf "91\\t%s\\tcompleted\\tsuccess\\n" "$CI_SHA"\n'
            'elif [[ "$1" == "api" ]]; then\n'
            '  calls=$(cat .api-calls 2>/dev/null || echo 0); calls=$((calls + 1)); echo "$calls" > .api-calls\n'
            '  (( calls > 1 )) || exit 1\n'
            '  printf "%s\\n" "$ARTIFACTS"\n'
            "else exit 1; fi\n"
        )
        completed, output = self.run_artifact_resolver(
            ARTIFACTS=self.artifact_inventory()
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(output, "run-id=91\n")
        self.assertEqual((self.root / ".api-calls").read_text().strip(), "2")

    def test_publication_slo_accepts_exactly_120_seconds(self):
        completed = self.run_publication_slo(120)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("release published 120s after its commit landed", completed.stdout)

    def test_invalid_publication_clock_keeps_its_error_diagnostic(self):
        completed = self.run_publication_slo(-1)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("release publication predates its commit", completed.stderr)

    def test_slow_publication_keeps_functional_success_and_reports_latency_miss(self):
        completed = self.run_publication_slo(121)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("::warning::", completed.stdout)
        self.assertIn("original publication missed its 120s SLO", completed.stdout)

    def test_recovery_reports_the_original_missed_publication_slo(self):
        completed = self.run_publication_slo(86400, event_name="workflow_dispatch")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("::warning::", completed.stdout)
        self.assertIn("original publication missed its 120s SLO", completed.stdout)
        self.assertIn("86400s", completed.stdout)


if __name__ == "__main__":
    unittest.main()
