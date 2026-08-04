"""Static and executable guards for the five-minute landing pipeline."""

import os
import re
import subprocess
import unittest
from pathlib import Path

from _repo import REPOSITORY
WORKFLOWS = REPOSITORY / ".github" / "workflows"
LANDING = WORKFLOWS / "landing.yml"
MAIN_VALIDATION = WORKFLOWS / "rust.yml"
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
            "LINUX_RESULT": "success",
            "WINDOWS_SMOKE_RESULT": "success",
        }

        cases = (
            ("pull request", {"EVENT_NAME": "pull_request", "TITLE_RESULT": "success", "LINUX_RESULT": "skipped", "WINDOWS_SMOKE_RESULT": "skipped"}, 0),
            ("merge group", {}, 0),
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
