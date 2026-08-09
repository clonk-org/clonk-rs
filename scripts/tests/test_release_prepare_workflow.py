"""Guards for the GitHub-native release preparation flow."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from _repo import REPOSITORY

PREPARE = REPOSITORY / ".github" / "workflows" / "release-prepare.yml"
PUBLISH = REPOSITORY / ".github" / "workflows" / "release.yml"
PREPARE_SCRIPT = REPOSITORY / "scripts" / "prepare-release.sh"


def prepare_step_script(name):
    """Return a named release-prepare step's real shell body."""
    lines = PREPARE.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(f"{PREPARE.name} has no step named {name!r}") from None

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


class ReleasePrepareWorkflowTests(unittest.TestCase):
    def test_inferred_version_scans_only_unreleased_commits(self):
        script = PREPARE_SCRIPT.read_text(encoding="utf-8")

        self.assertIn(
            'version=$("$tool" --config "$repo_root/cliff.toml" '
            "--unreleased --bumped-version 2>/dev/null)",
            script,
        )

    def test_git_cliff_cache_survives_release_script_edits(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertIn("key: git-cliff-${{ runner.os }}-v2.13.1", workflow)
        self.assertIn("restore-keys: git-cliff-${{ runner.os }}-", workflow)
        self.assertNotIn("hashFiles('scripts/prepare-release.sh')", workflow)

    def test_release_preparation_fails_its_two_minute_slo_closed(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertIn("actions: read", workflow)
        self.assertIn("id: pull-request", workflow)
        self.assertIn('echo "url=$pr" >> "$GITHUB_OUTPUT"', workflow)
        self.assertIn("name: Enforce release preparation SLO", workflow)
        self.assertIn("actions/runs/${GITHUB_RUN_ID}", workflow)
        self.assertIn('echo "url=$pr" >> "$GITHUB_OUTPUT"', workflow)
        self.assertIn("name: Ensure the release pull request is queued", workflow)
        self.assertIn("actions/runs/${{ github.run_id }}", workflow)
        self.assertIn(
            "steps.pull-request.outputs.url != '' || steps.existing.outputs.url != ''",
            workflow,
        )
        self.assertIn(
            "steps.pull-request.outputs.url || steps.existing.outputs.url", workflow
        )
        self.assertIn('if [[ "$elapsed" -gt 120 ]]', workflow)

    def test_release_preparation_uses_the_app_owned_pull_request(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertIn("actions/create-github-app-token@", workflow)
        self.assertIn("client-id: ${{ vars.RELEASE_APP_CLIENT_ID }}", workflow)
        self.assertIn("private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}", workflow)
        self.assertIn("token: ${{ steps.release-app.outputs.token }}", workflow)
        self.assertIn('branch="release/next"', workflow)
        self.assertIn('ref="refs/heads/${branch}"', workflow)
        self.assertIn("--force-with-lease=", workflow)
        self.assertIn("gh pr create", workflow)
        self.assertIn("gh pr merge", workflow)
        self.assertIn("--auto --squash", workflow)

    def test_branch_seeding_survives_a_ref_left_by_an_earlier_run(self):
        # A failure between seeding `release/next` and opening the pull request
        # leaves the branch behind: on 2026-07-31 a GitHub 504 did exactly
        # that. A ruleset forbids deleting the branch, so a create-only seed
        # then fails every later run with "Reference already exists" — the
        # daily schedule included. Seeding must reset an existing ref to the
        # base commit so the force-with-lease push still holds its lease.
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertIn(
            'gh api --method PATCH "repos/${REPOSITORY}/git/refs/heads/${branch}"',
            workflow,
        )
        self.assertIn("-F force=true", workflow)

    def test_schedule_prepares_a_pr_and_release_only_reacts_to_main(self):
        prepare = PREPARE.read_text(encoding="utf-8")
        publish = PUBLISH.read_text(encoding="utf-8")

        self.assertIn("schedule:", prepare)
        self.assertNotIn("schedule:", publish)
        self.assertNotIn("workflow_dispatch:", publish)

    def test_publish_resolver_has_no_legacy_preparation_ancestor(self):
        workflow = PUBLISH.read_text(encoding="utf-8")
        publish = workflow.split("\n  publish:\n", 1)[1]

        self.assertNotIn("\n  prepare:\n", workflow)
        self.assertNotIn("needs: [prepare]", publish)
        self.assertNotIn("needs.prepare", publish)
        self.assertIn("ref: ${{ github.sha }}", publish)
        self.assertIn("RESOLVED_SHA: ${{ github.sha }}", publish)

    def test_app_created_pr_runs_the_repository_checks(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertNotIn("A pull request opened with GITHUB_TOKEN", workflow)
        self.assertNotIn("cargo check --workspace --locked", workflow)
        self.assertNotIn(
            "cargo test -p xtask --features engine-tools", workflow
        )


@unittest.skipUnless(shutil.which("bash"), "needs bash")
class ReleasePreparationSloTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self._stub(
            "gh",
            'if [[ "$1" == "api" ]]; then echo "$RUN_CREATED_AT"; exit 0; fi\n'
            'if [[ "$1 $2" == "pr view" ]]; then echo "$PR_CREATED_AT"; exit 0; fi\n'
            "exit 1\n",
        )
        self._stub(
            "date",
            '[[ "$1 $2" == "-u -d" ]] || exit 1\n'
            'case "$3" in\n'
            '  run-created) echo "$RUN_EPOCH" ;;\n'
            '  pr-created) echo "$PR_EPOCH" ;;\n'
            '  *) exit 1 ;;\n'
            'esac\n',
        )

    def _stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def run_slo(self, elapsed):
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "ACTIONS_TOKEN": "actions-token",
            "GH_TOKEN": "app-token",
            "GITHUB_RUN_ID": "123",
            "PR_URL": "https://github.com/clonk-org/clonk-rs/pull/999",
            "REPOSITORY": "clonk-org/clonk-rs",
            "RUN_CREATED_AT": "run-created",
            "PR_CREATED_AT": "pr-created",
            "RUN_EPOCH": "1000",
            "PR_EPOCH": str(1000 + elapsed),
        }
        return subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                prepare_step_script("Enforce release preparation SLO"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )

    def run_existing_check(self, pull_requests):
        output = self.root / "existing-output"
        self._stub("gh", 'printf "%s\\n" "$PULL_REQUESTS"\n')
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "GH_TOKEN": "app-token",
            "GITHUB_OUTPUT": str(output),
            "REPOSITORY": "clonk-org/clonk-rs",
            "RUN_URL": "https://github.com/clonk-org/clonk-rs/actions/runs/123",
            "PULL_REQUESTS": json.dumps(pull_requests),
        }
        completed = subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                prepare_step_script("Check for an in-flight release"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )
        return completed, output.read_text(encoding="utf-8") if output.exists() else ""

    def test_preparation_slo_accepts_its_exact_boundary(self):
        completed = self.run_slo(120)

        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_preparation_slo_rejects_one_second_over_budget(self):
        completed = self.run_slo(121)

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("exceeded its 120s SLO", completed.stderr)

    def test_existing_pr_from_an_earlier_dispatch_is_already_satisfied(self):
        completed = self.run_slo(-1)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("predates this workflow dispatch", completed.stdout)

    def test_rerun_recovers_its_already_merged_release_pr(self):
        completed, output = self.run_existing_check(
            [
                {
                    "url": "https://github.com/clonk-org/clonk-rs/pull/999",
                    "state": "MERGED",
                    "createdAt": "2026-08-09T01:00:00Z",
                    "body": (
                        "Prepared by workflow run "
                        "https://github.com/clonk-org/clonk-rs/actions/runs/123."
                    ),
                }
            ]
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("open=true", output)
        self.assertIn("state=MERGED", output)
        self.assertIn("pull/999", output)

    def test_rerun_refuses_to_replace_its_closed_release_pr(self):
        completed, _ = self.run_existing_check(
            [
                {
                    "url": "https://github.com/clonk-org/clonk-rs/pull/999",
                    "state": "CLOSED",
                    "createdAt": "2026-08-09T01:00:00Z",
                    "body": (
                        "Prepared by workflow run "
                        "https://github.com/clonk-org/clonk-rs/actions/runs/123."
                    ),
                }
            ]
        )

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("was closed", completed.stderr)


if __name__ == "__main__":
    unittest.main()
