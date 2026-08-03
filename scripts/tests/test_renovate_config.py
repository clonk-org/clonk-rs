import json
import pathlib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


def load_config():
    return json.loads(
        (REPOSITORY / ".github" / "renovate.json").read_text(encoding="utf-8")
    )


class RenovateConfigTests(unittest.TestCase):
    def test_dependency_pr_titles_omit_the_disallowed_scope(self):
        config = load_config()

        self.assertIn(":semanticCommitTypeAll(chore)", config["extends"])
        self.assertIn(":semanticCommitScopeDisabled", config["extends"])

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
