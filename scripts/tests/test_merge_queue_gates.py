"""Static guards for the merge-queue split of the required checks.

Two invariants, both of which fail silently and expensively:

* A workflow that produces a required check must also run on `merge_group`.
  Without it the check is never reported for a queued pull request, and the
  queue waits for a status that will never arrive.
* The long jobs must not run per pull request. A public repository gets 20
  concurrent jobs; at this repository's commit rate, three ~20-minute jobs on
  every open pull request spends all of them on work the merge group repeats
  against the state that actually lands.
"""

import re
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
WORKFLOWS = REPOSITORY / ".github" / "workflows"

QUEUE_TIME_GUARD = "if: github.event_name != 'pull_request'"

# Jobs whose verdict is about what lands, not about fast feedback.
QUEUE_TIME_JOBS = {
    "rust.yml": ("coverage", "full-parity"),
    "windows.yml": ("runtime-msvc",),
}

# Jobs a pull request must still get an answer from before it can be queued.
PULL_REQUEST_JOBS = {
    "rust.yml": (
        "formatting",
        "focused-feedback",
        "recording-host-oracles",
        "workspace-lints",
    ),
    "windows.yml": ("launcher", "installer"),
}


def job_block(workflow: str, job: str) -> str:
    """The YAML lines belonging to one job, without parsing YAML.

    The rest of `scripts/tests` reads these workflows as text rather than
    taking a PyYAML dependency the runner does not install, so this does too.
    """
    source = (WORKFLOWS / workflow).read_text(encoding="utf-8")
    start = source.index(f"\n  {job}:\n") + 1
    following = re.compile(r"^  [A-Za-z0-9_-]+:$", re.MULTILINE)
    match = following.search(source, start + 1)
    return source[start : match.start()] if match else source[start:]


class MergeQueueGateTests(unittest.TestCase):
    def test_required_check_workflows_report_on_the_merge_group(self):
        for workflow in ("rust.yml", "windows.yml"):
            with self.subTest(workflow=workflow):
                source = (WORKFLOWS / workflow).read_text(encoding="utf-8")
                triggers = source[source.index("\non:\n") : source.index("\npermissions:")]
                self.assertIn("merge_group:", triggers)

    def test_the_long_jobs_are_queue_time_only(self):
        for workflow, jobs in QUEUE_TIME_JOBS.items():
            for job in jobs:
                with self.subTest(workflow=workflow, job=job):
                    self.assertIn(QUEUE_TIME_GUARD, job_block(workflow, job))

    def test_fast_feedback_still_answers_the_pull_request(self):
        for workflow, jobs in PULL_REQUEST_JOBS.items():
            for job in jobs:
                with self.subTest(workflow=workflow, job=job):
                    self.assertNotIn(QUEUE_TIME_GUARD, job_block(workflow, job))


if __name__ == "__main__":
    unittest.main()
