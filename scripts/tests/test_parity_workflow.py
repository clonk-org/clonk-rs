"""Keep the required parity comparators in their feature-shaped test shards."""

import unittest

from _repo import REPOSITORY


LANDING = REPOSITORY / ".github" / "workflows" / "landing.yml"
PARITY_FILTER = "test(/(^|::)parity_differential_matches_cpp_golden$/)"


def matrix_entry(workflow, name):
    """Return one Linux matrix entry by its display name."""
    start = workflow.index(f"- name: {name}")
    end = workflow.find("\n          - name:", start + 1)
    return workflow[start:] if end == -1 else workflow[start:end]


class ParityWorkflowTests(unittest.TestCase):
    def test_each_comparator_is_required_in_the_existing_shard_graph(self):
        workflow = LANDING.read_text(encoding="utf-8")
        app = matrix_entry(workflow, "app 4+9/12")
        units = matrix_entry(workflow, "engine and frontend unit and parity")

        self.assertIn(
            "cargo nextest run -p clonk-app --features app-test-shard-4,app-test-shard-9",
            app,
        )
        self.assertIn(PARITY_FILTER, app)
        self.assertIn("--no-tests=fail", app)

        self.assertNotIn("cargo xtask parity verify", units)
        for package in ("clonk-engine-unit-tests", "clonk-frontend-unit-tests"):
            with self.subTest(package=package):
                self.assertIn(f"package({package}) and {PARITY_FILTER}", units)
                self.assertIn("--no-tests=fail", units)
        self.assertEqual(units.count(PARITY_FILTER), 2)
        self.assertEqual(units.count("--features clonk-engine-unit-tests/ffi"), 3)


if __name__ == "__main__":
    unittest.main()
