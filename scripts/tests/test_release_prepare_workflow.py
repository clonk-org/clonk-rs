"""Static guards for the GitHub-native release preparation flow."""

import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
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

    def test_schedule_prepares_a_pr_and_release_only_reacts_to_main(self):
        prepare = PREPARE.read_text(encoding="utf-8")
        publish = PUBLISH.read_text(encoding="utf-8")

        self.assertIn("schedule:", prepare)
        self.assertNotIn("schedule:", publish)
        self.assertNotIn("workflow_dispatch:", publish)

    def test_app_created_pr_runs_the_repository_checks(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertNotIn("A pull request opened with GITHUB_TOKEN", workflow)
        self.assertNotIn("cargo check --workspace --locked", workflow)
        self.assertNotIn(
            "cargo test -p xtask --features engine-tools", workflow
        )


if __name__ == "__main__":
    unittest.main()
