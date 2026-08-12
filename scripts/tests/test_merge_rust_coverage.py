"""Tests for merging independently collected Rust LCOV reports."""

import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

from _repo import REPOSITORY


SCRIPT = REPOSITORY / "scripts" / "merge-rust-coverage.py"
CHECKSUM_A = "a" * 32
CHECKSUM_B = "b" * 32
CHECKSUM_C = "c" * 32


class MergeRustCoverageTests(unittest.TestCase):
    def setUp(self):
        self.sandbox = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.sandbox, ignore_errors=True)
        self.output = self.sandbox / "merged.info"

    def write_coverage(self, name, contents):
        path = self.sandbox / name
        path.write_text(textwrap.dedent(contents).lstrip(), encoding="utf-8")
        return path

    def run_merge(self, *inputs, threshold=None, output=True):
        command = [sys.executable, str(SCRIPT), *(str(path) for path in inputs)]
        if output:
            command.extend(("--output", str(self.output if output is True else output)))
        if threshold is not None:
            command.extend(("--fail-under-lines", str(threshold)))
        return subprocess.run(command, capture_output=True, text=True)

    def test_overlapping_records_sum_hits_and_count_unique_lines_once(self):
        first = self.write_coverage(
            "first.info",
            f"""
            SF:/workspace/src/lib.rs
            DA:3,0,{CHECKSUM_C}
            DA:1,2,{CHECKSUM_A}
            LF:2
            LH:1
            end_of_record
            """,
        )
        second = self.write_coverage(
            "second.info",
            f"""
            SF:/workspace/src/lib.rs
            DA:2,0,{CHECKSUM_B}
            DA:1,4,{CHECKSUM_A}
            LF:2
            LH:1
            end_of_record
            """,
        )

        completed = self.run_merge(second, first)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "lines: 1/3 (33.33%)\n")
        self.assertEqual(
            self.output.read_text(encoding="utf-8"),
            textwrap.dedent(
                f"""\
                SF:/workspace/src/lib.rs
                DA:1,6,{CHECKSUM_A}
                DA:2,0,{CHECKSUM_B}
                DA:3,0,{CHECKSUM_C}
                LF:3
                LH:1
                end_of_record
                """
            ),
        )

    def test_functions_and_branches_are_unioned_summed_and_recounted(self):
        first = self.write_coverage(
            "first.info",
            """
            SF:/workspace/src/control.rs
            FN:20,bar
            FN:10,foo
            FNDA:0,bar
            FNDA:2,foo
            FNF:2
            FNH:1
            DA:10,2
            LF:1
            LH:1
            BRDA:11,0,1,-
            BRDA:11,0,0,2
            BRDA:21,1,0,0
            BRF:3
            BRH:1
            end_of_record
            """,
        )
        second = self.write_coverage(
            "second.info",
            """
            SF:/workspace/src/control.rs
            FN:30,baz
            FN:10,foo
            FNDA:1,baz
            FNDA:3,foo
            FNF:2
            FNH:2
            DA:10,3
            LF:1
            LH:1
            BRDA:31,2,0,0
            BRDA:11,0,0,4
            BRDA:11,0,1,1
            BRF:3
            BRH:2
            end_of_record
            """,
        )

        completed = self.run_merge(first, second)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            self.output.read_text(encoding="utf-8"),
            textwrap.dedent(
                """\
                SF:/workspace/src/control.rs
                FN:10,foo
                FN:20,bar
                FN:30,baz
                FNDA:5,foo
                FNDA:0,bar
                FNDA:1,baz
                FNF:3
                FNH:2
                DA:10,5
                BRDA:11,0,0,6
                BRDA:11,0,1,1
                BRDA:21,1,0,0
                BRDA:31,2,0,0
                BRF:4
                BRH:2
                LF:1
                LH:1
                end_of_record
                """
            ),
        )

    def test_output_is_deterministic_across_input_and_record_order(self):
        first = self.write_coverage(
            "first.info",
            """
            SF:/workspace/src/z.rs
            DA:9,1
            LF:1
            LH:1
            end_of_record
            SF:/workspace/src/a.rs
            DA:7,0
            LF:1
            LH:0
            end_of_record
            """,
        )
        second = self.write_coverage(
            "second.info",
            """
            SF:/workspace/src/a.rs
            DA:2,1
            LF:1
            LH:1
            end_of_record
            """,
        )
        reverse_output = self.sandbox / "reverse.info"

        forward = self.run_merge(first, second)
        reverse = self.run_merge(second, first, output=reverse_output)

        self.assertEqual(forward.returncode, 0, forward.stderr)
        self.assertEqual(reverse.returncode, 0, reverse.stderr)
        self.assertEqual(self.output.read_bytes(), reverse_output.read_bytes())
        self.assertLess(
            self.output.read_text(encoding="utf-8").index("SF:/workspace/src/a.rs"),
            self.output.read_text(encoding="utf-8").index("SF:/workspace/src/z.rs"),
        )

    def test_line_threshold_uses_aggregate_unique_line_coverage(self):
        first = self.write_coverage(
            "first.info",
            """
            SF:/workspace/src/lib.rs
            DA:1,1
            DA:2,0
            LF:2
            LH:1
            end_of_record
            """,
        )
        second = self.write_coverage(
            "second.info",
            """
            SF:/workspace/src/lib.rs
            DA:2,1
            DA:3,0
            LF:2
            LH:1
            end_of_record
            """,
        )

        passing = self.run_merge(first, second, threshold="66.6")
        failing = self.run_merge(first, second, threshold="66.7")

        self.assertEqual(passing.returncode, 0, passing.stderr)
        self.assertEqual(passing.stdout, "lines: 2/3 (66.67%)\n")
        self.assertNotEqual(failing.returncode, 0)
        self.assertEqual(failing.stdout, "lines: 2/3 (66.67%)\n")
        self.assertIn("below required 66.7%", failing.stderr)

    def test_threshold_can_be_checked_without_rewriting_a_report(self):
        coverage = self.write_coverage(
            "coverage.info",
            """
            SF:/workspace/src/lib.rs
            DA:1,1
            DA:2,0
            LF:2
            LH:1
            end_of_record
            """,
        )
        original = coverage.read_bytes()

        completed = self.run_merge(coverage, threshold="50", output=False)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "lines: 1/2 (50.00%)\n")
        self.assertEqual(coverage.read_bytes(), original)
        self.assertFalse(self.output.exists())

    def test_accepts_llvm_function_totals_deduplicated_from_symbols(self):
        coverage = self.write_coverage(
            "llvm.info",
            """
            SF:/workspace/src/lib.rs
            FN:10,_RNCNvCsFirst4crate4work0B3_
            FN:10,_RNCNvCsSecond4crate4work0B3_
            FNDA:3,_RNCNvCsFirst4crate4work0B3_
            FNDA:0,_RNCNvCsSecond4crate4work0B3_
            FNF:1
            FNH:1
            DA:10,3
            LF:1
            LH:1
            end_of_record
            """,
        )

        completed = self.run_merge(coverage, threshold="100", output=False)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "lines: 1/1 (100.00%)\n")

    def test_accepts_llvm_summaries_with_expansion_line_totals(self):
        coverage = self.write_coverage(
            "llvm-expansions.info",
            """
            SF:/workspace/src/lib.rs
            FNF:0
            FNH:0
            DA:10,3
            BRF:0
            BRH:0
            LF:2
            LH:2
            end_of_record
            """,
        )

        completed = self.run_merge(coverage, threshold="100", output=False)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "lines: 1/1 (100.00%)\n")

    def test_rejects_empty_and_malformed_inputs(self):
        fixtures = {
            "empty": "",
            "missing terminator": "SF:/workspace/src/lib.rs\nDA:1,1\n",
            "invalid line count": (
                "SF:/workspace/src/lib.rs\nDA:1,not-a-count\nend_of_record\n"
            ),
            "invalid checksum": (
                "SF:/workspace/src/lib.rs\nDA:1,1,not-an-md5\nend_of_record\n"
            ),
            "unknown record field": (
                "SF:/workspace/src/lib.rs\nXX:lost\nend_of_record\n"
            ),
        }

        for label, contents in fixtures.items():
            with self.subTest(label=label):
                coverage = self.write_coverage(f"{label}.info", contents)
                completed = self.run_merge(coverage)

                self.assertNotEqual(completed.returncode, 0)
                self.assertIn(label.split()[0], completed.stderr.lower())

    def test_rejects_conflicting_checksums_and_function_locations(self):
        conflicts = {
            "checksum": (
                f"DA:1,1,{CHECKSUM_A}",
                f"DA:1,2,{CHECKSUM_B}",
            ),
            "function": ("FN:10,work\nFNDA:1,work", "FN:20,work\nFNDA:1,work"),
        }

        for label, (first_data, second_data) in conflicts.items():
            with self.subTest(label=label):
                first = self.write_coverage(
                    f"{label}-first.info",
                    f"SF:/workspace/src/lib.rs\n{first_data}\nend_of_record\n",
                )
                second = self.write_coverage(
                    f"{label}-second.info",
                    f"SF:/workspace/src/lib.rs\n{second_data}\nend_of_record\n",
                )

                completed = self.run_merge(first, second)

                self.assertNotEqual(completed.returncode, 0)
                self.assertIn(label, completed.stderr.lower())


if __name__ == "__main__":
    unittest.main()
