import json
import unittest

from _repo import REPOSITORY


def load_config():
    return json.loads(
        (REPOSITORY / ".github" / "renovate.json").read_text(encoding="utf-8")
    )


class RenovateConfigTests(unittest.TestCase):
    def test_dependency_pr_titles_omit_the_disallowed_scope(self):
        config = load_config()

        self.assertIn(":semanticCommitTypeAll(chore)", config["extends"])
        self.assertIn(":semanticCommitScopeDisabled", config["extends"])

    def test_updates_are_looked_for_daily(self):
        config = load_config()

        self.assertIn("schedule:daily", config["extends"])
        self.assertNotIn("schedule:monthly", config["extends"])

    def test_a_release_soaks_for_a_week_unless_it_fixes_a_vulnerability(self):
        # `internalChecksFilter` defaults to "strict", so an update younger than
        # this opens no pull request at all; it waits on the dependency
        # dashboard. A security fix must not wait, hence the null override.
        config = load_config()

        self.assertEqual("7 days", config["minimumReleaseAge"])
        self.assertIsNone(config["vulnerabilityAlerts"]["minimumReleaseAge"])

    def test_a_dependency_pull_request_merges_itself_once_green(self):
        config = load_config()

        self.assertTrue(config["automerge"])

    def test_automerge_hands_off_to_the_merge_queue(self):
        # `main` lands through a merge queue, so Renovate must not merge the
        # branch itself: only GitHub's native auto-merge enqueues an entry.
        # This is Renovate's default and is written down because it is what
        # makes automerge work here at all.
        config = load_config()

        self.assertTrue(config["platformAutomerge"])

    def test_lock_file_maintenance_states_its_own_cadence(self):
        # Renovate defaults `lockFileMaintenance.schedule` to
        # ["before 4am on monday"], and a child object's own schedule wins over
        # the inherited top-level one, so this cadence has to be written down
        # here rather than assumed from the repository schedule.
        config = load_config()

        self.assertEqual(
            ["before 4am on monday"], config["lockFileMaintenance"]["schedule"]
        )


if __name__ == "__main__":
    unittest.main()
