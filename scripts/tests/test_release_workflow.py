"""Release validation guards, exercised from the workflow's real shell."""

import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from test_release_content_handoff import WORKFLOW, step_script

BUILD_WORKFLOW = WORKFLOW.with_name("release-build.yml")
LANDING_WORKFLOW = WORKFLOW.with_name("landing.yml")
PREBUILD_WORKFLOW = WORKFLOW.with_name("release-prebuild.yml")


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
        self.assertIn("uses: ./.github/workflows/release-build.yml", landing)
        self.assertIn("uses: ./.github/workflows/release-prebuild.yml", landing)
        self.assertIn("source-sha: ${{ github.sha }}", landing)

    def test_reusable_release_build_validates_the_requested_source_version(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("name: Validate the release source", reusable)
        self.assertIn("REQUESTED_VERSION: ${{ inputs.version }}", reusable)
        self.assertIn('[[ "$SOURCE_SHA" != "$MERGE_SHA" ]]', reusable)
        self.assertIn('workspace version ${actual_version}', reusable)
        self.assertIn('requested release ${REQUESTED_VERSION}', reusable)

    def test_merge_group_release_build_uses_exact_rerunnable_handoff_caches(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        prebuild = PREBUILD_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("shared-key: release-${{", prebuild)
        for trusted_cache in (
            "full-parity",
            "windows-runtime-msvc",
            "recording-host-oracles",
        ):
            with self.subTest(trusted_cache=trusted_cache):
                self.assertIn(f"shared-key: {trusted_cache}", prebuild)
        self.assertIn("actions/cache/save@", prebuild)
        self.assertIn("actions/cache/restore@", reusable)
        self.assertIn("fail-on-cache-miss: true", reusable)
        self.assertEqual(reusable.count("compression-level: 0"), 3)
        self.assertEqual(reusable.count("overwrite: true"), 3)

    def test_release_build_parallelizes_tools_runtimes_and_macos_architectures(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        prebuild = PREBUILD_WORKFLOW.read_text(encoding="utf-8")
        landing = LANDING_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("\n  tool:\n", prebuild)
        self.assertIn("\n  runtime:\n", prebuild)
        self.assertIn("\n  package:\n", reusable)
        package = build_job_block("package")
        self.assertNotIn("needs: [tool, runtime]", package)
        self.assertIn("needs: [release-context, release-prebuild]", landing)
        self.assertIn("name: macos-arm64", prebuild)
        self.assertIn("name: macos-x86_64", prebuild)
        self.assertIn("--target aarch64-apple-darwin", prebuild)
        self.assertIn("--target x86_64-apple-darwin", prebuild)
        self.assertIn("--skip-build", package)

    def test_release_build_handoffs_are_exact_run_scoped_caches(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        prebuild = PREBUILD_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("\n  package:\n", reusable)
        package = build_job_block("package")

        self.assertIn(
            "actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
            prebuild,
        )
        self.assertIn(
            "actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
            package,
        )
        for fragment in (
            "${{ inputs.source-sha }}",
            "${{ github.run_id }}",
            "fail-on-cache-miss: true",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, prebuild + reusable)
        self.assertNotIn("restore-keys:", prebuild + reusable)
        self.assertNotIn("github.run_attempt", reusable)

        # Only final distributables are Actions artifacts. Compile handoffs use
        # run-scoped caches so release.yml still verifies exactly seven names.
        self.assertEqual(reusable.count("actions/upload-artifact@"), 3)

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

    def test_release_build_enforces_half_of_the_measured_baseline(self):
        reusable = BUILD_WORKFLOW.read_text(encoding="utf-8")
        landing = LANDING_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("\n  latency:\n", reusable)
        gate = build_job_block("landing-gate", LANDING_WORKFLOW)
        self.assertIn("actions: read", gate)
        self.assertIn("MAX_RELEASE_BUILD_SECONDS: '898'", gate)
        self.assertIn("actions/runs/${GITHUB_RUN_ID}/jobs", gate)
        self.assertIn('if [[ "$elapsed" -gt "$MAX_RELEASE_BUILD_SECONDS" ]]', gate)

    def test_release_commits_have_a_sha_specific_concurrency_lane(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn(
            'group: "release-${{ startsWith(github.event.head_commit.message, '
            "'chore: release ') && github.sha || 'rolling' }}\"",
            workflow,
        )
        self.assertIn("cancel-in-progress: false", workflow)

    def test_publish_uses_a_shallow_checkout_and_the_tag_api(self):
        publish = job_block("publish")

        self.assertIn("fetch-depth: 1", publish)
        self.assertIn('git/ref/tags/v${version}', publish)
        self.assertNotIn('git rev-parse -q --verify "refs/tags/', publish)

    def test_publication_fails_its_two_minute_slo_closed(self):
        publish = job_block("publish")

        self.assertIn("name: Enforce release publication SLO", publish)
        self.assertIn("releases/tags/v${VERSION}", publish)
        self.assertIn("commits/${SHA}/pulls", publish)
        self.assertIn(".merge_commit_sha == $sha", publish)
        self.assertIn(".merged_at", publish)
        self.assertIn('if [[ "$elapsed" -gt 120 ]]', publish)
        self.assertIn("CI_POLL_ATTEMPTS: '5'", publish)
        self.assertIn("CI_POLL_SECONDS: '1'", publish)

    def test_already_published_rerun_still_enforces_publication_slo(self):
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

    def run_publication_slo(self, elapsed_seconds):
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

    def run_release_build_latency(self, elapsed_seconds, *, omit=None):
        started = datetime(2026, 8, 10, tzinfo=timezone.utc)
        completed = started + timedelta(seconds=elapsed_seconds)
        jobs = []
        matrix = {"Package": ("linux", "windows", "macos")}
        for phase, names in matrix.items():
            for name in names:
                separator = " " if phase == "Package" else " / "
                full_name = f"Build release candidate / {phase}{separator}{name}"
                if full_name == omit:
                    continue
                jobs.append(
                    {
                        "name": full_name,
                        "status": "completed",
                        "conclusion": "success",
                        "started_at": started.isoformat().replace("+00:00", "Z"),
                        "completed_at": completed.isoformat().replace("+00:00", "Z"),
                    }
                )
        jobs.extend(
            [
                {
                    "name": "Landing gate",
                    "status": "in_progress",
                    "conclusion": None,
                    "started_at": completed.isoformat().replace("+00:00", "Z"),
                    "completed_at": None,
                },
                {
                    "name": "Qualify release candidate / Rust code coverage",
                    "status": "completed",
                    "conclusion": "success",
                    "started_at": "2026-08-09T00:00:00Z",
                    "completed_at": "2026-08-11T00:00:00Z",
                },
            ]
        )
        inventory = self.root / "jobs.json"
        inventory.write_text(
            json.dumps({"total_count": len(jobs), "jobs": jobs}),
            encoding="utf-8",
        )
        self._stub('cat "$JOBS"\n')
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "GH_TOKEN": "stub",
            "GITHUB_RUN_ID": "31343777447",
            "JOBS": str(inventory),
            "MAX_RELEASE_BUILD_SECONDS": "898",
            "REPOSITORY": "clonk-org/clonk-rs",
        }
        return subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                build_step_script("Enforce release build latency", LANDING_WORKFLOW),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )

    @staticmethod
    def artifact_inventory(*, omit=None, extra=None, expired=None):
        names = [
            "desktop-linux",
            "desktop-windows",
            "desktop-macos",
            "components-desktop-linux",
            "components-desktop-windows",
            "components-desktop-macos",
            "release-tool",
        ]
        return json.dumps(
            {
                "artifacts": [
                    {"name": name, "expired": name == expired}
                    for name in names
                    if name != omit
                ]
                + ([{"name": extra, "expired": False}] if extra else [])
            }
        )

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

    def test_publication_slo_rejects_121_seconds(self):
        completed = self.run_publication_slo(121)

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("release publication exceeded its 120s SLO", completed.stderr)

    def test_release_build_latency_accepts_exactly_half_the_baseline(self):
        completed = self.run_release_build_latency(898)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("release build completed in 898s", completed.stdout)

    def test_release_build_latency_rejects_one_second_over_target(self):
        completed = self.run_release_build_latency(899)

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("release build took 899s", completed.stderr)

    def test_release_build_latency_rejects_a_partial_matrix(self):
        completed = self.run_release_build_latency(
            500,
            omit="Build release candidate / Package macos",
        )

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("unexpected release-build job inventory", completed.stderr)


if __name__ == "__main__":
    unittest.main()
