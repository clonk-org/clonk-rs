"""Main validation keeps post-merge coverage visible and reproducible."""

import re
import unittest

from _repo import REPOSITORY

WORKFLOW = REPOSITORY / ".github" / "workflows" / "rust.yml"


def job_block(name):
    """Return one top-level job without requiring a YAML dependency."""
    workflow = WORKFLOW.read_text(encoding="utf-8")
    marker = f"  {name}:"
    try:
        start = workflow.index(marker)
    except ValueError:
        raise AssertionError(f"{WORKFLOW.name} has no job named {name!r}") from None

    next_job = re.search(r"(?m)^  [A-Za-z0-9_-]+:$", workflow[start + 1 :])
    end = start + 1 + next_job.start() if next_job else None
    return workflow[start:end]


class RustCoverageGateTests(unittest.TestCase):
    def test_named_coverage_job_runs_the_locked_workspace_suite(self):
        coverage = job_block("coverage")

        self.assertIn("name: Rust code coverage", coverage)
        self.assertIn("submodules: recursive", coverage)
        self.assertIn("components: llvm-tools-preview", coverage)
        self.assertIn("tool: cargo-nextest@0.9.91", coverage)
        self.assertIn("tool: cargo-llvm-cov@0.8.7", coverage)
        self.assertEqual(coverage.count("fallback: none"), 2)
        self.assertIn("LLVM_PROFILE_FILE_NAME: 'clonk-%8m.profraw'", coverage)
        self.assertIn("COVERAGE_MIN_LINE_PERCENT: '79'", coverage)
        self.assertIn("cargo llvm-cov clean --workspace", coverage)
        command_text = coverage.replace("\\\n", " ")
        self.assertRegex(
            command_text,
            r"cargo llvm-cov --no-report nextest\s+"
            r"--workspace\s+"
            r"--no-fail-fast\s+"
            r"--features xtask/engine-tools\s+"
            r"--locked",
        )
        self.assertIn(
            'cargo llvm-cov report --fail-under-lines '
            '"$COVERAGE_MIN_LINE_PERCENT"',
            coverage,
        )
        self.assertNotIn("continue-on-error:", coverage)

    def test_coverage_reports_are_retained_when_the_floor_fails(self):
        coverage = job_block("coverage")
        command_text = coverage.replace("\\\n", " ")

        self.assertIn(
            "cargo llvm-cov report --lcov "
            "--output-path target/coverage/lcov.info",
            command_text,
        )
        self.assertIn(
            "cargo llvm-cov report --html "
            "--output-dir target/coverage ",
            command_text,
        )
        self.assertNotIn("--output-dir target/coverage/html", command_text)
        ignore_option = '--ignore-filename-regex "$COVERAGE_IGNORE_REGEX"'
        self.assertEqual(coverage.count(ignore_option), 3)
        self.assertRegex(
            coverage,
            r"(?s)- name: Upload Rust coverage reports\s+"
            r"if: always\(\)\s+"
            r"uses: actions/upload-artifact@"
            r"043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7\.0\.1\s+"
            r"with:\s+"
            r"name: rust-coverage-\$\{\{ github\.run_id \}\}-"
            r"\$\{\{ github\.run_attempt \}\}\s+"
            r"path: target/coverage\s+"
            r"if-no-files-found: warn\s+"
            r"retention-days: 14",
        )

if __name__ == "__main__":
    unittest.main()
