from __future__ import annotations

import subprocess
import sys
import time
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
SCRIPT = REPOSITORY / "scripts" / "run_with_timeout.py"


class RunWithTimeoutTests(unittest.TestCase):
    def test_runs_command_that_finishes_before_deadline(self) -> None:
        completed = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "1",
                sys.executable,
                "-c",
                "print('finished')",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "finished\n")

    def test_returns_timeout_status_when_deadline_expires(self) -> None:
        started = time.monotonic()
        completed = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "0.05",
                sys.executable,
                "-c",
                "import time; time.sleep(60)",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 124, completed.stderr)
        self.assertLess(time.monotonic() - started, 2)


if __name__ == "__main__":
    unittest.main()
