"""Static and executable guards for the five-minute landing pipeline."""

import json
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

from _repo import REPOSITORY
WORKFLOWS = REPOSITORY / ".github" / "workflows"
LANDING = WORKFLOWS / "landing.yml"
MAIN_VALIDATION = WORKFLOWS / "rust.yml"
EXACT_SHA_QUALIFICATION = WORKFLOWS / "exact-sha-qualification.yml"
LEGACY_WINDOWS = WORKFLOWS / "windows.yml"
QUEUE_JOBS = ("linux", "windows-smoke")


def job_block(workflow: Path, job: str) -> str:
    """Return one top-level workflow job without a YAML dependency."""
    source = workflow.read_text(encoding="utf-8")
    marker = f"\n  {job}:\n"
    try:
        start = source.index(marker) + 1
    except ValueError:
        raise AssertionError(f"{workflow.name} has no job named {job!r}") from None
    following = re.compile(r"^  [A-Za-z0-9_-]+:$", re.MULTILINE)
    match = following.search(source, start + 1)
    return source[start : match.start()] if match else source[start:]


def trigger_block(workflow: Path) -> str:
    source = workflow.read_text(encoding="utf-8")
    return source[source.index("\non:\n") : source.index("\npermissions:")]


def step_script(workflow: Path, name: str) -> str:
    lines = workflow.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(f"{workflow.name} has no step named {name!r}") from None
    try:
        body = lines.index("        run: |", start) + 1
    except ValueError:
        raise AssertionError(f"step {name!r} has no literal run block") from None
    script = []
    for line in lines[body:]:
        if line.strip() and not line.startswith(" " * 10):
            break
        script.append(line[10:])
    return "\n".join(script)


class MergeQueueGateTests(unittest.TestCase):
    def test_release_merge_group_runs_existing_exact_sha_qualification(self):
        workflow = LANDING.read_text(encoding="utf-8")
        qualification = job_block(LANDING, "release-qualification")
        gate = job_block(LANDING, "landing-gate")

        self.assertTrue(EXACT_SHA_QUALIFICATION.exists())
        reusable = EXACT_SHA_QUALIFICATION.read_text(encoding="utf-8")
        self.assertIn("name: Rust code coverage", reusable)
        self.assertIn(
            "name: Recording-host material-order oracles (macOS)", reusable
        )
        self.assertEqual(
            reusable.count("if: ${{ always() && inputs.upload-diagnostics }}"),
            1,
        )
        self.assertIn("if: inputs.upload-diagnostics", reusable)
        self.assertIn("name: Rust coverage HTML report", reusable)
        self.assertIn(
            "needs: [release-context, linux, windows-smoke]", qualification
        )
        self.assertIn(
            "needs.release-context.outputs.release == 'true'", qualification
        )
        self.assertIn(
            "uses: ./.github/workflows/exact-sha-qualification.yml",
            qualification,
        )
        self.assertIn("source-sha: ${{ github.sha }}", qualification)
        self.assertIn("concurrency-suffix: ${{ github.sha }}", qualification)
        self.assertIn("publish-recording-host-cache: false", qualification)
        self.assertIn("upload-diagnostics: false", qualification)
        self.assertIn(
            "upload-diagnostics: true",
            job_block(MAIN_VALIDATION, "exact-sha-qualification"),
        )
        self.assertRegex(gate, r"(?m)^      - release-qualification$")
        self.assertIn("RELEASE_QUALIFICATION_RESULT", gate)

    def test_release_merge_group_builds_exact_sha_artifacts_before_landing(self):
        workflow = LANDING.read_text(encoding="utf-8")
        context = job_block(LANDING, "release-context")
        prebuild = job_block(LANDING, "release-prebuild")
        build = job_block(LANDING, "release-build")
        gate = job_block(LANDING, "landing-gate")

        self.assertIn("github.event.merge_group.head_commit.message", context)
        self.assertIn("release: ${{ steps.release.outputs.release }}", context)
        self.assertIn("tree-sha: ${{ steps.release.outputs.tree-sha }}", context)
        self.assertIn("pr-number: ${{ steps.release.outputs.pr-number }}", context)
        self.assertIn("version: ${{ steps.release.outputs.version }}", context)
        self.assertIn("needs: release-context", prebuild)
        self.assertIn("uses: ./.github/workflows/release-prebuild.yml", prebuild)
        self.assertIn("source-sha: ${{ github.sha }}", prebuild)
        self.assertIn("needs: [release-context, release-prebuild]", build)
        self.assertIn("needs.release-context.outputs.release == 'true'", build)
        self.assertIn("uses: ./.github/workflows/release-build.yml", build)
        self.assertIn("source-sha: ${{ github.sha }}", build)
        self.assertIn(
            "tree-sha: ${{ needs.release-context.outputs.tree-sha }}", build
        )
        self.assertIn(
            "version: ${{ needs.release-context.outputs.version }}",
            build,
        )
        for job in ("release-context", "release-prebuild", "release-build"):
            self.assertRegex(gate, rf"(?m)^      - {job}$")
        self.assertIn("RELEASE_CONTEXT_RESULT", gate)
        self.assertIn("RELEASE_PREBUILD_RESULT", gate)
        self.assertIn("RELEASE_BUILD_RESULT", gate)
        self.assertIn("IS_RELEASE", gate)

    def test_landing_owns_pr_and_merge_group_while_main_validation_owns_push(self):
        landing = trigger_block(LANDING)
        main = trigger_block(MAIN_VALIDATION)

        self.assertIn("pull_request:", landing)
        self.assertIn("types: [opened, synchronize, reopened, edited]", landing)
        self.assertIn("merge_group:", landing)
        self.assertNotIn("push:", landing)
        self.assertNotIn("pull_request_target:", landing)
        self.assertIn("push:", main)
        self.assertIn("branches: [main]", main)
        self.assertNotIn("pull_request:", main)
        self.assertNotIn("merge_group:", main)
        self.assertFalse(
            LEGACY_WINDOWS.exists(),
            "Windows landing checks belong in the one fail-closed landing graph",
        )

    def test_pull_requests_run_formatting_and_strict_workspace_clippy(self):
        quality = job_block(LANDING, "pull-request-quality")

        self.assertRegex(
            quality,
            r"(?m)^    if: github\.event_name == 'pull_request'$",
        )
        self.assertIn("runs-on: ubuntu-24.04", quality)
        self.assertIn("timeout-minutes: 15", quality)
        self.assertIn("fetch-depth: 1", quality)
        self.assertIn("persist-credentials: false", quality)
        self.assertNotIn("submodules: recursive", quality)
        self.assertIn("actions/cache/restore@", quality)
        self.assertIn("path: .git/modules/content", quality)
        self.assertIn(
            "run: git submodule update --init --force --depth=1 "
            "--filter=blob:none content",
            quality,
        )
        self.assertIn("libasound2-dev libudev-dev", quality)
        self.assertIn(
            "uses: dtolnay/rust-toolchain@"
            "f8be11a05b1d4f3fcebe6410cc16743212b999b0",
            quality,
        )
        self.assertIn("components: clippy, rustfmt", quality)
        self.assertIn(
            "uses: Swatinem/rust-cache@"
            "6323deb102c322ba6fcbdcafc7e3dddab59af2b6",
            quality,
        )
        self.assertIn("shared-key: full-parity", quality)
        self.assertEqual(quality.count("save-if: false"), 1)
        commands = (
            "cargo fmt --all -- --check",
            "cargo clippy --profile test --workspace --lib --bins --tests "
            "--features xtask/engine-tools --locked -- -D warnings",
        )
        for command in commands:
            with self.subTest(command=command):
                self.assertRegex(quality, rf"(?m)^        run: {re.escape(command)}$")
        self.assertNotIn("cargo nextest", quality)
        self.assertNotIn("python3 -m unittest", quality)
        self.assertNotIn("continue-on-error:", quality)
        self.assertNotRegex(quality, r"(?m)^        if:")

    def test_landing_gate_requires_pull_request_quality_only_during_admission(self):
        gate = job_block(LANDING, "landing-gate")
        script = step_script(LANDING, "Enforce landing results")

        self.assertRegex(gate, r"(?m)^      - pull-request-quality$")
        self.assertIn(
            "QUALITY_RESULT: ${{ needs.pull-request-quality.result }}",
            gate,
        )
        self.assertEqual(
            script.count('require_result quality "$QUALITY_RESULT" success'),
            1,
        )
        self.assertEqual(
            script.count('require_result quality "$QUALITY_RESULT" skipped'),
            2,
        )

    def test_one_required_gate_fails_closed_over_every_queue_job(self):
        workflow = LANDING.read_text(encoding="utf-8")
        self.assertEqual(workflow.count("name: Landing gate"), 1)
        self.assertIn("permissions:\n  contents: read", workflow)

        gate = job_block(LANDING, "landing-gate")
        self.assertIn("name: Landing gate", gate)
        self.assertIn("if: always()", gate)
        for job in ("pull-request-title", *QUEUE_JOBS):
            with self.subTest(job=job):
                self.assertRegex(gate, rf"(?m)^      - {re.escape(job)}$")

        for job in QUEUE_JOBS:
            with self.subTest(job=job):
                self.assertIn(
                    "if: github.event_name != 'pull_request'",
                    job_block(LANDING, job),
                )
        self.assertRegex(
            job_block(LANDING, "pull-request-title"),
            r"(?m)^    if: github\.event_name == 'pull_request'$",
        )

    def test_landing_result_script_accepts_only_the_intended_phase_results(self):
        script = step_script(LANDING, "Enforce landing results")
        base = {
            **os.environ,
            "EVENT_NAME": "merge_group",
            "TITLE_RESULT": "skipped",
            "QUALITY_RESULT": "skipped",
            "RELEASE_CONTEXT_RESULT": "success",
            "RELEASE_PREBUILD_RESULT": "skipped",
            "RELEASE_BUILD_RESULT": "skipped",
            "RELEASE_QUALIFICATION_RESULT": "skipped",
            "IS_RELEASE": "false",
            "LINUX_RESULT": "success",
            "WINDOWS_SMOKE_RESULT": "success",
        }

        cases = (
            (
                "pull request",
                {
                    "EVENT_NAME": "pull_request",
                    "TITLE_RESULT": "success",
                    "QUALITY_RESULT": "success",
                    "RELEASE_CONTEXT_RESULT": "skipped",
                    "RELEASE_PREBUILD_RESULT": "skipped",
                    "IS_RELEASE": "",
                    "LINUX_RESULT": "skipped",
                    "WINDOWS_SMOKE_RESULT": "skipped",
                },
                0,
            ),
            (
                "pull request quality failed",
                {
                    "EVENT_NAME": "pull_request",
                    "TITLE_RESULT": "success",
                    "QUALITY_RESULT": "failure",
                    "RELEASE_CONTEXT_RESULT": "skipped",
                    "RELEASE_PREBUILD_RESULT": "skipped",
                    "IS_RELEASE": "",
                    "LINUX_RESULT": "skipped",
                    "WINDOWS_SMOKE_RESULT": "skipped",
                },
                1,
            ),
            ("ordinary merge group", {}, 0),
            (
                "merge group unexpectedly ran pull request quality",
                {"QUALITY_RESULT": "success"},
                1,
            ),
            (
                "workflow dispatch",
                {
                    "EVENT_NAME": "workflow_dispatch",
                    "RELEASE_CONTEXT_RESULT": "skipped",
                    "IS_RELEASE": "",
                },
                0,
            ),
            (
                "workflow dispatch unexpectedly ran pull request quality",
                {
                    "EVENT_NAME": "workflow_dispatch",
                    "QUALITY_RESULT": "success",
                    "RELEASE_CONTEXT_RESULT": "skipped",
                    "IS_RELEASE": "",
                },
                1,
            ),
            (
                "release merge group",
                {
                    "RELEASE_BUILD_RESULT": "success",
                    "RELEASE_PREBUILD_RESULT": "success",
                    "RELEASE_QUALIFICATION_RESULT": "success",
                    "IS_RELEASE": "true",
                },
                0,
            ),
            (
                "release qualification failed",
                {
                    "RELEASE_BUILD_RESULT": "success",
                    "RELEASE_PREBUILD_RESULT": "success",
                    "RELEASE_QUALIFICATION_RESULT": "failure",
                    "IS_RELEASE": "true",
                },
                1,
            ),
            (
                "release prebuild failed",
                {
                    "RELEASE_BUILD_RESULT": "skipped",
                    "RELEASE_PREBUILD_RESULT": "failure",
                    "RELEASE_QUALIFICATION_RESULT": "success",
                    "IS_RELEASE": "true",
                },
                1,
            ),
            (
                "ordinary merge unexpectedly qualified",
                {"RELEASE_QUALIFICATION_RESULT": "success"},
                1,
            ),
            ("failed child", {"LINUX_RESULT": "failure"}, 1),
            ("cancelled child", {"WINDOWS_SMOKE_RESULT": "cancelled"}, 1),
            ("skipped merge child", {"WINDOWS_SMOKE_RESULT": "skipped"}, 1),
        )
        for name, changed, expected in cases:
            with self.subTest(case=name):
                completed = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", script],
                    env={**base, **changed},
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(completed.returncode, expected, completed.stderr)

    def test_release_context_accepts_only_the_exact_merge_queue_subject(self):
        script = step_script(LANDING, "Resolve the merge-group release")
        cases = (
            ("fix: ordinary candidate (#230)", 0, "release=false\n"),
            ("chore: release 0.9.4", 1, ""),
            ("chore: release next (#231)", 1, ""),
        )
        for subject, expected_status, expected_output in cases:
            with self.subTest(subject=subject), tempfile.TemporaryDirectory() as root:
                output = Path(root) / "github-output"
                completed = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", script],
                    env={
                        **os.environ,
                        "MERGE_SUBJECT": subject,
                        "GITHUB_OUTPUT": str(output),
                    },
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(completed.returncode, expected_status, completed.stderr)
                actual = output.read_text(encoding="utf-8") if output.exists() else ""
                self.assertEqual(actual, expected_output)

    def test_release_context_resolves_the_exact_merge_group_tree(self):
        script = step_script(LANDING, "Resolve the merge-group release")
        merge = "a" * 40
        tree = "b" * 40
        pr = {
            "title": "chore: release 0.9.4",
            "state": "open",
            "head": {
                "ref": "release/next",
                "repo": {"full_name": "clonk-org/clonk-rs"},
            },
            "base": {"ref": "main"},
        }
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            output = root / "github-output"
            stub = root / "gh"
            stub.write_text(
                "#!/usr/bin/env bash\n"
                "case \"$*\" in\n"
                '  *"pulls/231"*) printf "%s\\n" "$PR_JSON" ;;\n'
                '  *"git/commits/$MERGE_SHA"*) printf "%s\\n" "$TREE_SHA" ;;\n'
                "  *) exit 1 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            stub.chmod(0o755)
            completed = subprocess.run(
                ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", script],
                env={
                    **os.environ,
                    "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                    "GH_TOKEN": "stub",
                    "MERGE_SHA": merge,
                    "MERGE_SUBJECT": (
                        "chore: release 0.9.4 (#231)\n\nSquashed commits follow."
                    ),
                    "REPOSITORY": "clonk-org/clonk-rs",
                    "GITHUB_OUTPUT": str(output),
                    "PR_JSON": json.dumps(pr),
                    "TREE_SHA": tree,
                },
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(
                output.read_text(encoding="utf-8"),
                "release=true\npr-number=231\n"
                f"tree-sha={tree}\nversion=0.9.4\n",
            )

    def test_pull_request_title_is_an_unscoped_subject_only(self):
        script = step_script(LANDING, "Check the title is a Conventional Commit subject")
        cases = (
            ("fix: keep the queue exact", 0),
            ("perf!: change the shipped profile", 0),
            ("fix(engine): forbidden scope", 1),
            ("fix: subject\n\nbody", 1),
            ("Fix: wrong case", 1),
        )
        for title, expected in cases:
            with self.subTest(title=title):
                completed = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", script],
                    env={**os.environ, "TITLE": title},
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(completed.returncode, expected, completed.stderr)

    def test_every_external_action_is_pinned_to_a_commit(self):
        source = LANDING.read_text(encoding="utf-8")
        action = re.compile(r"(?m)^\s*- uses: ([^./\s][^@\s]*)@([^\s]+)")
        uses = action.findall(source)
        self.assertGreater(len(uses), 0)
        for name, revision in uses:
            with self.subTest(action=name):
                self.assertRegex(revision, r"^[0-9a-f]{40}$")


if __name__ == "__main__":
    unittest.main()
