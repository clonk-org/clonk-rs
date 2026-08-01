"""Release validation guards, exercised from the workflow's real shell."""

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_release_content_handoff import WORKFLOW, step_script


def job_block(name):
    source = WORKFLOW.read_text(encoding="utf-8")
    marker = f"\n  {name}:\n"
    start = source.index(marker) + 1
    following = re.compile(r"^  [A-Za-z0-9_-]+:$", re.MULTILINE)
    match = following.search(source, start + 1)
    return source[start : match.start()] if match else source[start:]


class ReleaseWorkflowTopologyTests(unittest.TestCase):
    def test_release_commits_have_a_sha_specific_concurrency_lane(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn(
            'group: "release-${{ startsWith(github.event.head_commit.message, '
            "'chore: release ') && github.sha || 'rolling' }}\"",
            workflow,
        )
        self.assertIn("cancel-in-progress: false", workflow)

    def test_resolver_blocks_builds_on_exact_sha_validation(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        resolve = job_block("resolve")
        build = job_block("build")

        self.assertNotIn("\n  prepare:\n", workflow)
        self.assertIn("actions: read", resolve)
        self.assertIn("if: steps.resolve.outputs.release == 'true'", resolve)
        self.assertIn("CI_SHA: ${{ steps.resolve.outputs.sha }}", resolve)
        self.assertIn("landing.yml", resolve)
        self.assertIn("rust.yml", resolve)
        self.assertIn("needs: resolve", build)

    def test_partial_publication_can_resume_without_persisted_git_credentials(self):
        resolve = job_block("resolve")
        publish = job_block("publish")

        self.assertIn('"repos/${REPOSITORY}/releases/tags/v${version}"', resolve)
        self.assertIn("--jq '.draft'", resolve)
        self.assertIn("'(HTTP 404)'", resolve)
        self.assertIn('if [[ "$release_state" == "false" ]]', resolve)
        self.assertIn("persist-credentials: false", publish)
        self.assertIn('gh release create "v${VERSION}" --draft', publish)
        self.assertIn('gh release edit "v${VERSION}" --draft=false --latest', publish)


@unittest.skipUnless(shutil.which("bash"), "needs bash")
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

    def run_gate(self, **extra):
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "GH_TOKEN": "stub",
            "CI_SHA": "0123456789abcdef",
            "CI_POLL_ATTEMPTS": "3",
            "CI_POLL_SECONDS": "0",
            **extra,
        }
        return subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                step_script("Require exact-SHA validation"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )

    def test_gate_accepts_only_the_two_exact_event_verdicts(self):
        self._stub(
            '[[ " $* " == *" --commit $CI_SHA "* ]] || { echo completed:failure; exit 0; }\n'
            'if [[ " $* " == *" --workflow landing.yml "* ]]; then\n'
            '  [[ " $* " == *" --event merge_group "* ]] || { echo completed:failure; exit 0; }\n'
            '  [[ " $* " != *" --branch "* ]] || { echo completed:failure; exit 0; }\n'
            'elif [[ " $* " == *" --workflow rust.yml "* ]]; then\n'
            '  [[ " $* " == *" --event push "* ]] || { echo completed:failure; exit 0; }\n'
            '  [[ " $* " == *" --branch main "* ]] || { echo completed:failure; exit 0; }\n'
            'else\n  echo completed:failure; exit 0\nfi\n'
            "echo completed:success\n"
        )
        completed = self.run_gate()
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_gate_waits_for_running_checks_without_hiding_them(self):
        self._stub(
            '[[ " $* " != *" --status completed "* ]] || exit 9\n'
            'counter=.calls\n'
            'calls=$(cat "$counter" 2>/dev/null || echo 0)\n'
            'calls=$((calls + 1))\n'
            'echo "$calls" > "$counter"\n'
            'if (( calls <= 2 )); then echo in_progress:none; else echo completed:success; fi\n'
        )
        completed = self.run_gate()
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("in_progress:none", completed.stdout)

    def test_gate_rejects_a_terminal_failure_immediately(self):
        self._stub(
            'if [[ " $* " == *" --workflow landing.yml "* ]]; then\n'
            "  echo completed:failure\n"
            "else\n  echo completed:success\nfi\n"
        )
        completed = self.run_gate()
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("completed:failure", completed.stderr)

    def test_gate_times_out_when_a_verdict_never_appears(self):
        self._stub("echo missing\n")
        completed = self.run_gate(CI_POLL_ATTEMPTS="2")
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("timed out", completed.stderr)


if __name__ == "__main__":
    unittest.main()
