import json
import pathlib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


class RenovateConfigTests(unittest.TestCase):
    def test_dependency_pr_titles_omit_the_disallowed_scope(self):
        config = json.loads(
            (REPOSITORY / ".github" / "renovate.json").read_text(encoding="utf-8")
        )

        self.assertIn(":semanticCommitTypeAll(chore)", config["extends"])
        self.assertIn(":semanticCommitScopeDisabled", config["extends"])


if __name__ == "__main__":
    unittest.main()
