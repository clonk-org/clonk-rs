"""Static guards for the GitHub Actions critical-path reductions."""

import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
WORKFLOWS = (
    REPOSITORY / ".github" / "workflows" / "rust.yml",
    REPOSITORY / ".github" / "workflows" / "windows.yml",
    REPOSITORY / ".github" / "workflows" / "dependency-guard.yml",
)


class CiLatencyTests(unittest.TestCase):
    def test_obsolete_pushes_are_superseded_without_cancelling_release_commits(self):
        for workflow in WORKFLOWS:
            with self.subTest(workflow=workflow.name):
                source = workflow.read_text(encoding="utf-8")
                self.assertIn(
                    "startsWith(github.event.head_commit.message, 'chore: release ')",
                    source,
                )
                self.assertIn("&& github.sha ||", source)
                self.assertIn("cancel-in-progress: true", source)
                self.assertNotIn(
                    "cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}",
                    source,
                )

    def test_full_parity_reuses_the_engine_tools_test_graph(self):
        workflow = WORKFLOWS[0].read_text(encoding="utf-8")
        full_parity = workflow[workflow.index("  full-parity:") :]
        workspace = full_parity.index(
            "run: cargo nextest run --workspace --no-fail-fast "
            "--features xtask/engine-tools --locked"
        )
        packaging = full_parity.index(
            "run: cargo test -p xtask --features engine-tools "
            "--bin xtask-engine-tools --locked"
        )
        parity = full_parity.index("run: cargo xtask parity verify")
        snapshots = full_parity.index(
            "run: cargo xtask engine-snapshots verify"
        )

        self.assertLess(workspace, parity)
        self.assertLess(parity, packaging)
        self.assertLess(packaging, snapshots)

    def test_workspace_lints_run_in_parallel_with_full_parity(self):
        workflow = WORKFLOWS[0].read_text(encoding="utf-8")
        lint_start = workflow.index("  workspace-lints:")
        full_start = workflow.index("  full-parity:")
        lint_block = workflow[lint_start:full_start]
        full_parity = workflow[full_start:]
        command = (
            "cargo clippy --profile test --workspace --lib --bins --tests "
            "--features xtask/engine-tools --locked -- -D warnings"
        )

        self.assertIn(f"run: {command}", lint_block)
        self.assertNotIn(command, full_parity)


if __name__ == "__main__":
    unittest.main()
