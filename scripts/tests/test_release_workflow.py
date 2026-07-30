"""Release control-flow guards, exercised from the workflow's real shell."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_release_content_handoff import step_script


@unittest.skipUnless(shutil.which("bash"), "needs bash")
class ReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.github_output = self.root / "github-output"

    def _stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def run_step(self, name, **extra):
        self.github_output.write_text("", encoding="utf-8")
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "GH_TOKEN": "stub",
            "FORCE": "false",
            "CI_SHA": "0123456789abcdef",
            "GITHUB_OUTPUT": str(self.github_output),
            "REQUESTED_VERSION": "",
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
                step_script(name),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )

    def seed_prepared_release(self, subject, version="0.5.0"):
        scripts = self.root / "scripts"
        scripts.mkdir()
        prepare = scripts / "prepare-release.sh"
        prepare.write_text("#!/usr/bin/env bash\nexit 3\n", encoding="utf-8")
        prepare.chmod(0o755)
        (self.root / "Cargo.toml").write_text(
            f'[workspace.package]\nversion = "{version}"\n\n[workspace]\nmembers = []\n',
            encoding="utf-8",
        )
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        subprocess.run(
            ["git", "config", "user.email", "release-test@example.invalid"],
            cwd=self.root,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "Release Test"],
            cwd=self.root,
            check=True,
        )
        subprocess.run(["git", "add", "Cargo.toml"], cwd=self.root, check=True)
        subprocess.run(
            ["git", "commit", "-q", "-m", subject],
            cwd=self.root,
            check=True,
        )
        return subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def test_ci_gate_accepts_only_push_runs_for_the_exact_release_sha(self):
        self._stub(
            "gh",
            '[[ " $* " == *" --commit $CI_SHA "* ]] || { echo completed:failure; exit 0; }\n'
            '[[ " $* " == *" --event push "* ]] || { echo completed:failure; exit 0; }\n'
            "echo completed:success\n",
        )

        completed = self.run_step("Require main's CI to be green")

        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_ci_gate_does_not_hide_a_newer_in_progress_run(self):
        self._stub(
            "gh",
            'if [[ " $* " == *" --status completed "* ]]; then\n'
            "  echo success\n"
            "else\n"
            "  echo in_progress:none\n"
            "fi\n",
        )

        completed = self.run_step("Require main's CI to be green")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("in_progress:none", completed.stderr)

    def test_ci_gate_rejects_one_failed_required_workflow(self):
        self._stub(
            "gh",
            'if [[ " $* " == *" --workflow Windows "* ]]; then\n'
            "  echo completed:failure\n"
            "else\n"
            "  echo completed:success\n"
            "fi\n",
        )

        completed = self.run_step("Require main's CI to be green")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("completed:failure", completed.stderr)

    def test_retry_resolves_the_original_release_commit_instead_of_head(self):
        release_sha = self.seed_prepared_release("chore: release 0.5.0")
        (self.root / "README.md").write_text("later docs\n", encoding="utf-8")
        subprocess.run(["git", "add", "README.md"], cwd=self.root, check=True)
        subprocess.run(
            ["git", "commit", "-q", "-m", "docs: later change"],
            cwd=self.root,
            check=True,
        )

        completed = self.run_step("Bump and commit")
        outputs = dict(
            line.split("=", 1)
            for line in self.github_output.read_text(encoding="utf-8").splitlines()
            if "=" in line
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(outputs["sha"], release_sha)

    def test_retry_fails_closed_without_the_prepared_release_commit(self):
        self.seed_prepared_release("docs: version metadata without release commit")

        completed = self.run_step("Bump and commit")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn(
            "no reachable 'chore: release 0.5.0' commit exists",
            completed.stderr,
        )
        self.assertNotIn(
            "prepared=true",
            self.github_output.read_text(encoding="utf-8"),
        )

    def test_retry_fails_closed_on_ambiguous_release_commits(self):
        self.seed_prepared_release("chore: release 0.5.0")
        (self.root / "README.md").write_text("duplicate release\n", encoding="utf-8")
        subprocess.run(["git", "add", "README.md"], cwd=self.root, check=True)
        subprocess.run(
            ["git", "commit", "-q", "-m", "chore: release 0.5.0"],
            cwd=self.root,
            check=True,
        )

        completed = self.run_step("Bump and commit")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("multiple reachable", completed.stderr)

    def test_retry_rejects_a_release_subject_with_the_wrong_tree_version(self):
        self.seed_prepared_release("chore: release 0.5.0", version="0.4.0")
        (self.root / "Cargo.toml").write_text(
            '[workspace.package]\nversion = "0.5.0"\n\n[workspace]\nmembers = []\n',
            encoding="utf-8",
        )
        subprocess.run(["git", "add", "Cargo.toml"], cwd=self.root, check=True)
        subprocess.run(
            ["git", "commit", "-q", "-m", "fix: align version metadata"],
            cwd=self.root,
            check=True,
        )

        completed = self.run_step("Bump and commit")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("contains workspace version 0.4.0", completed.stderr)


if __name__ == "__main__":
    unittest.main()
