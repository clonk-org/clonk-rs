"""Static guards for the GitHub-native release preparation flow."""

import unittest

from _repo import REPOSITORY

PREPARE = REPOSITORY / ".github" / "workflows" / "release-prepare.yml"
PUBLISH = REPOSITORY / ".github" / "workflows" / "release.yml"


class ReleasePrepareWorkflowTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
