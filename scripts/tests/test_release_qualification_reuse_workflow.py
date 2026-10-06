"""Guard post-merge reuse of independently qualified Windows runtimes."""

import json
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

from _repo import REPOSITORY


MAIN = REPOSITORY / ".github" / "workflows" / "rust.yml"


def workflow_job(workflow, name):
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [a-z][a-z0-9_-]*:\n|\Z)",
        workflow,
    )
    if match is None:
        raise AssertionError(f"missing job: {name}")
    return match.group(0)


def workflow_steps(job):
    return re.findall(
        r"(?ms)^      - (?:name|uses|id):.*?(?=^      - (?:name|uses|id):|\Z)",
        job,
    )


def named_step(job, name):
    for step in workflow_steps(job):
        if f"      - name: {name}\n" in step:
            return step
    raise AssertionError(f"missing step: {name}")


def shell_script(step):
    match = re.search(r"(?ms)^        run: \|\n(.*)", step)
    if match is None:
        raise AssertionError("expected a literal shell script")
    return textwrap.dedent(match.group(1))


class ReleaseQualificationReuseWorkflowTests(unittest.TestCase):
    def test_windows_dependency_graph_handles_nonrelease_skips(self):
        workflow = MAIN.read_text(encoding="utf-8")
        job = workflow_job(workflow, "windows-release-tools")
        header = job[:job.index("    steps:\n")]

        self.assertIn("needs: [windows-landing-cache, qualification-reuse]", header)
        self.assertIn("always()", header)
        self.assertIn("!cancelled()", header)
        self.assertIn("needs.windows-landing-cache.result == 'success'", header)
        self.assertIn("needs.qualification-reuse.result == 'success'", header)
        self.assertIn("needs.qualification-reuse.result == 'skipped'", header)
        self.assertIn("github.event_name != 'workflow_dispatch' || !inputs.cache_only", header)

    def test_windows_download_uses_the_verified_original_run_and_exact_artifact(self):
        workflow = MAIN.read_text(encoding="utf-8")
        resolver = workflow_job(workflow, "qualification-reuse")
        self.assertIn("github.event_name == 'push'", resolver)
        self.assertIn("startsWith(github.event.head_commit.message, 'chore: release ')", resolver)
        self.assertIn("scripts/release-qualification-evidence.py resolve", resolver)
        self.assertIn("run-id: ${{ steps.evidence.outputs.run-id }}", resolver)
        self.assertIn("run-attempt: ${{ steps.evidence.outputs.run-attempt }}", resolver)

        job = workflow_job(workflow, "windows-release-tools")
        self.assertIn("      actions: read", job[:job.index("    steps:\n")])
        downloads = [step for step in workflow_steps(job) if "actions/download-artifact@" in step]
        self.assertEqual(len(downloads), 1)
        download = downloads[0]
        self.assertIn("if: needs.qualification-reuse.outputs.run-id != ''", download)
        self.assertIn(
            "name: release-qualified-runtime-windows-${{ github.sha }}-${{ needs.qualification-reuse.outputs.run-id }}",
            download,
        )
        self.assertIn("run-id: ${{ needs.qualification-reuse.outputs.run-id }}", download)
        self.assertIn("repository: ${{ github.repository }}", download)
        self.assertIn("github-token: ${{ github.token }}", download)
        self.assertIn("path: target/release-qualified/windows", download)
        self.assertNotIn("pattern:", download)

    def test_manifest_passes_before_exact_binaries_are_installed(self):
        job = workflow_job(MAIN.read_text(encoding="utf-8"), "windows-release-tools")
        step = named_step(job, "Verify and install the qualified Windows runtime")
        self.assertIn("if: needs.qualification-reuse.outputs.run-id != ''", step)
        self.assertIn("SOURCE_SHA: ${{ github.sha }}", step)
        self.assertIn("BUILD_RUN_ID: ${{ needs.qualification-reuse.outputs.run-id }}", step)
        self.assertIn("BUILD_RUN_ATTEMPT: ${{ needs.qualification-reuse.outputs.run-attempt }}", step)
        script = shell_script(step)
        self.assertNotIn("release-prebuild-manifest.py write", script)
        self.assertNotIn("--build-profile", script)
        self.assertNotIn("--build-feature", script)
        self.assertNotIn("--build-target", script)
        self.assertLess(job.index("Download the independently qualified Windows runtime"), job.index("Verify and install"))
        self.assertLess(job.index("Verify and install"), job.index("Validate the shipped MSVC runtime"))

        with tempfile.TemporaryDirectory(prefix="clonk-qualified-windows-") as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3"\n', encoding="utf-8")
            subprocess.run(["git", "init", "-q"], cwd=root, check=True, capture_output=True)
            subprocess.run(["git", "add", "Cargo.toml"], cwd=root, check=True, capture_output=True)
            subprocess.run(
                ["git", "-c", "user.name=CI fixture", "-c", "user.email=ci@example.com",
                 "-c", "commit.gpgSign=false", "commit", "-qm", "fixture"],
                cwd=root, check=True, capture_output=True,
            )
            head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
            tree = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=root, text=True).strip()
            tools = root / "tools"
            tools.mkdir()
            verifier = tools / "python3"
            verifier.write_text(
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                "from pathlib import Path\n"
                "if sys.argv[1:3] == ['scripts/release-prebuild-manifest.py', 'verify']:\n"
                "    Path(os.environ['VERIFY_LOG']).write_text(json.dumps(sys.argv[1:]))\n"
                "    sys.exit(int(os.environ['VERIFY_EXIT']))\n"
                "os.execv(sys.executable, [sys.executable, *sys.argv[1:]])\n",
                encoding="utf-8",
            )
            verifier.chmod(0o755)
            payload = root / "target/release-qualified/windows/payload"
            payload.mkdir(parents=True)
            destination = root / "target/x86_64-pc-windows-msvc/release"
            destination.mkdir(parents=True)
            names = ("c4group.exe", "clonk-app.exe", "clonk-game.exe")
            for name in names:
                (payload / name).write_bytes(f"original qualified {name}".encode())
            log = root / "verify.json"
            environment = dict(
                os.environ, PATH=f"{tools}{os.pathsep}{os.environ['PATH']}",
                GITHUB_WORKSPACE=str(root), BUILD_RUN_ID="2042", BUILD_RUN_ATTEMPT="3",
                VERIFY_LOG=str(log),
            )
            for status, source in ((1, head), (0, "0" * 40), (0, head)):
                with self.subTest(verifier_status=status, source=source):
                    for name in names:
                        (destination / name).write_bytes(b"unqualified local warming output")
                    log.unlink(missing_ok=True)
                    process = subprocess.run(
                        ["bash", "-e", "-o", "pipefail", "-c", script], cwd=root,
                        env=dict(environment, VERIFY_EXIT=str(status), SOURCE_SHA=source),
                        capture_output=True, text=True, timeout=10,
                    )
                    accepted = status == 0 and source == head
                    self.assertEqual(process.returncode == 0, accepted, process.stderr)
                    for name in names:
                        expected = (payload / name).read_bytes() if accepted else b"unqualified local warming output"
                        self.assertEqual((destination / name).read_bytes(), expected)
                    if source != head:
                        self.assertFalse(log.exists(), "wrong checkout must fail before manifest verification")
                    else:
                        arguments = json.loads(log.read_text(encoding="utf-8"))
                        for flag, value in (
                            ("--provenance-root", str(root)), ("--head-sha", head),
                            ("--tree-sha", tree), ("--version", "1.2.3"),
                            ("--build-run-id", "2042"), ("--build-run-attempt", "3"),
                            ("--kind", "runtime"), ("--target", "x86_64-pc-windows-msvc"),
                        ):
                            self.assertEqual(arguments[arguments.index(flag) + 1], value)
                        self.assertEqual(
                            [arguments[index + 1] for index, arg in enumerate(arguments) if arg == "--file"],
                            [f"payload/{name}" for name in names],
                        )

    def test_receipt_misses_build_and_reuse_still_warms_missing_caches(self):
        job = workflow_job(MAIN.read_text(encoding="utf-8"), "windows-release-tools")
        host = named_step(job, "Build the Windows packaging tool")
        self.assertIn("!inputs.software_presentation", host)
        self.assertIn("needs.qualification-reuse.outputs.run-id == ''", host)
        self.assertIn(
            "cargo build --profile test --locked -p xtask --features engine-tools --bin xtask-engine-tools",
            host,
        )
        for name in (
            "Configure the shipped MSVC runtime", "Restore trusted ThinLTO cache",
            "Restore or publish the shipped MSVC dependency cache", "Validate the shipped MSVC runtime",
        ):
            self.assertNotIn("        if:", named_step(job, name))
        dependency_cache = named_step(job, "Restore or publish the shipped MSVC dependency cache")
        self.assertIn("id: shipped-cache", dependency_cache)
        self.assertIn("shared-key: shipped-msvc-runtime-v1", dependency_cache)
        build = named_step(job, "Refresh the shipped MSVC runtime cache")
        self.assertIn("needs.qualification-reuse.outputs.run-id == ''", build)
        self.assertIn("steps.thinlto-cache.outputs.cache-hit != 'true'", build)
        self.assertIn("steps.shipped-cache.outputs.cache-hit != 'true'", build)
        self.assertIn("cargo fetch --locked", build)
        self.assertIn("cargo build --release -p clonk-app -p clonk-game -p clonk-c4group --locked --timings", build)
        self.assertLess(job.index("Refresh the shipped MSVC runtime cache"), job.index("Verify and install"))
        self.assertIn("scripts/validate-msvc-runtime.sh", named_step(job, "Validate the shipped MSVC runtime"))
        for name in (
            "Materialize verified Windows presentation content", "Prepare Windows software presentation qualification",
            "Qualify forced Windows software presentation", "Qualify automatic Windows software presentation",
            "Preserve Windows software presentation evidence",
        ):
            manual = named_step(job, name)
            self.assertIn("inputs.software_presentation", manual)
            self.assertNotIn("needs.qualification-reuse.outputs", manual)
        self.assertIn(
            "--release --check-input", named_step(job, "Qualify forced Windows software presentation"),
        )
        self.assertIn(
            "--automatic-fallback", named_step(job, "Qualify automatic Windows software presentation"),
        )

    def test_retention_is_optional_trusted_main_and_keeps_a_bounded_receipt(self):
        workflow = MAIN.read_text(encoding="utf-8")
        job = workflow_job(workflow, "cache-retention")
        header = job[:job.index("    steps:\n")]
        self.assertIn("needs: [linux-landing-cache, exact-sha-qualification]", header)
        self.assertIn("always()", header)
        self.assertIn("github.ref == 'refs/heads/main'", header)
        self.assertIn("github.event_name == 'push' || github.event_name == 'workflow_dispatch'", header)
        self.assertIn("continue-on-error: true", header)
        self.assertIn("      contents: read", header)
        self.assertIn("      actions: write", header)
        self.assertEqual(workflow.count("actions: write"), 1)
        retention = named_step(job, "Bound trusted workspace build caches")
        self.assertIn("GH_TOKEN: ${{ github.token }}", retention)
        self.assertIn("REPOSITORY: ${{ github.repository }}", retention)
        for argument in (
            'scripts/ci-cache-retention.py --repository "$REPOSITORY"',
            "--prefix clonk-ci-target-v1-", "--keep-per-lane 2",
            "--max-bytes 4294967296", "--apply",
            '--output "$RUNNER_TEMP/ci-cache-retention.json"',
        ):
            self.assertIn(argument, retention)
        receipt = named_step(job, "Retain the cache retention receipt")
        self.assertIn("if: always()", receipt)
        self.assertIn("actions/upload-artifact@", receipt)
        self.assertIn("name: ci-cache-retention-${{ github.run_id }}-${{ github.run_attempt }}", receipt)
        self.assertIn("path: ${{ runner.temp }}/ci-cache-retention.json", receipt)
        self.assertIn("if-no-files-found: error", receipt)
        self.assertIn("retention-days: 14", receipt)

    def test_latency_report_waits_for_every_other_job_and_cannot_gate_them(self):
        workflow = MAIN.read_text(encoding="utf-8")
        jobs = set(re.findall(r"(?m)^  ([a-z][a-z0-9_-]*):$", workflow[workflow.index("jobs:\n"):]))
        report = workflow_job(workflow, "latency-report")
        header = report[:report.index("    steps:\n")]
        needs = re.search(r"(?ms)^    needs:\n(.*?)(?=^    [a-z])", header)
        self.assertIsNotNone(needs)
        self.assertEqual(set(re.findall(r"(?m)^      - ([a-z][a-z0-9_-]*)$", needs.group(1))), jobs - {"latency-report"})
        self.assertIn("name: CI latency report (postmerge)", header)
        self.assertIn("if: always()", header)
        self.assertIn("continue-on-error: true", header)
        self.assertIn("      contents: read", header)
        self.assertIn("      actions: read", header)
        self.assertNotIn("      actions: write", header)
        for name in jobs - {"latency-report"}:
            functional_header = workflow_job(workflow, name).split("    steps:\n")[0]
            self.assertNotIn("latency-report", functional_header, name)
        observation = named_step(report, "Report completed post-merge work")
        self.assertIn("GH_TOKEN: ${{ github.token }}", observation)
        self.assertIn("REPOSITORY: ${{ github.repository }}", observation)
        self.assertIn("SOURCE_SHA: ${{ github.sha }}", observation)
        self.assertIn("RUN_ID: ${{ github.run_id }}", observation)
        script = shell_script(observation)
        for argument in (
            'scripts/ci-latency-report.py --repository "$REPOSITORY"',
            '--run-id "$RUN_ID"', "--phase postmerge", "--allow-running true",
            "--history-limit 20", '--release "$release"',
            '--output "$RUNNER_TEMP/ci-latency-postmerge.json"',
            '--summary "$GITHUB_STEP_SUMMARY"',
        ):
            self.assertIn(argument, script)
        self.assertIn('[[ "$(git rev-parse HEAD)" == "$SOURCE_SHA" ]]', script)
        self.assertIn("subject=$(git log -1 --format=%s)", script)
        receipt = named_step(report, "Retain the post-merge latency report")
        self.assertIn("if: always()", receipt)
        self.assertIn("actions/upload-artifact@", receipt)
        self.assertIn("path: ${{ runner.temp }}/ci-latency-postmerge.json", receipt)
        self.assertIn("retention-days: 14", receipt)

        with tempfile.TemporaryDirectory(prefix="clonk-postmerge-report-") as temporary:
            directory = Path(temporary)
            git = directory / "git"
            git.write_text(
                f"#!{sys.executable}\nimport os, sys\n"
                "if sys.argv[1:] == ['rev-parse', 'HEAD']:\n"
                "    print(os.environ['ACTUAL_SHA'])\n"
                "elif sys.argv[1:] == ['log', '-1', '--format=%s']:\n"
                "    print(os.environ['HEAD_SUBJECT'])\n"
                "else:\n    sys.exit(1)\n",
                encoding="utf-8",
            )
            git.chmod(0o755)
            python = directory / "python3"
            python.write_text(
                f"#!{sys.executable}\nimport json, os, sys\nfrom pathlib import Path\n"
                "Path(os.environ['REPORT_ARGS']).write_text(json.dumps(sys.argv[1:]))\n",
                encoding="utf-8",
            )
            python.chmod(0o755)
            arguments_path = directory / "report-args.json"
            environment = dict(
                os.environ, PATH=f"{directory}{os.pathsep}{os.environ['PATH']}",
                ACTUAL_SHA="a" * 40, SOURCE_SHA="a" * 40, RUN_ID="900",
                REPOSITORY="clonk-org/clonk-rs", RUNNER_TEMP=str(directory),
                GITHUB_STEP_SUMMARY=str(directory / "summary.md"), REPORT_ARGS=str(arguments_path),
            )
            for subject, release in (("chore: release 1.2.3", "true"), ("fix: runtime startup", "false")):
                process = subprocess.run(
                    ["bash", "-e", "-o", "pipefail", "-c", script],
                    env=dict(environment, HEAD_SUBJECT=subject),
                    capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(process.returncode, 0, process.stderr)
                arguments = json.loads(arguments_path.read_text(encoding="utf-8"))
                self.assertEqual(arguments[arguments.index("--release") + 1], release)
                self.assertEqual(arguments[arguments.index("--phase") + 1], "postmerge")


if __name__ == "__main__":
    unittest.main()
