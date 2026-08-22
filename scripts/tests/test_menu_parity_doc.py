"""`docs/MENU_PARITY.md` must stay a C++ reference, not a status census.

The file used to carry a per-row "Rust status" column and a dated audit
snapshot. Both drifted far enough out of date to contradict production code and
already-closed issues, which is worse than having no inventory at all: a stale
census causes duplicate issues, false parity claims and wrong priority calls
(clonk-org/clonk-rs#967).

Status now lives in the tests and in the tracker issues, both of which move with
the code. These checks fail if a census grows back.
"""

import re
import unittest
from pathlib import Path

DOC = Path(__file__).resolve().parents[2] / "docs" / "MENU_PARITY.md"

# The status vocabulary the retired census used. The definitions section still
# explains the words, so only *claims* are rejected: a table cell that is
# nothing but a status, or a bulleted claim that leads with one.
STATUS_WORDS = ("Complete", "Partial", "Missing", "Fail-fast", "Dropped")
STATUS_CELL = re.compile(
    r"\|\s*\*\*(?:" + "|".join(STATUS_WORDS) + r")\*\*\s*\|"
)
STATUS_BULLET = re.compile(
    r"\(\*\*(?:" + "|".join(STATUS_WORDS) + r")\*\*\s*:"
)
# `Audit snapshot (2026-07-20): implementation through <sha>` and friends.
DATED_SNAPSHOT = re.compile(r"[Aa]udit snapshot\s*\(\d{4}-\d{2}-\d{2}\)")
# The private work queue the snapshot used to cite; those ids resolve only
# outside this repository.
PRIVATE_QUEUE = re.compile(r"\bM\d{2}\s+queue\b|\bM\d{2}-P\d-L\d{3}\b|\bCLO-\d{3}\b")
# This gap inventory was left behind in two retired census fragments and
# contradicted the detailed, tested portrait-selector row.
PORTRAIT_GAP_CLAIM = re.compile(
    r"\bPortrait\b[^.]*\bstill lacks\b", re.IGNORECASE | re.DOTALL
)


class MenuParityDocTest(unittest.TestCase):
    def setUp(self):
        self.text = DOC.read_text(encoding="utf-8")

    def test_no_status_column(self):
        # The prose may *name* the retired column while explaining why it went;
        # what must not come back is the column itself.
        offenders = [
            line
            for line in self.text.splitlines()
            if line.startswith("|") and "Rust status" in line
        ]
        self.assertEqual(
            [],
            offenders,
            "docs/MENU_PARITY.md must not carry a status column; record status "
            "in the tracker issues instead (clonk-org/clonk-rs#381, #383)",
        )

    def test_no_status_cells(self):
        offenders = [
            line
            for line in self.text.splitlines()
            if line.startswith("|") and STATUS_CELL.search(line)
        ]
        self.assertEqual(
            [],
            offenders,
            "a table cell claims an implementation status; describe what C++ "
            "does and let the tests and tracker issues carry status",
        )

    def test_no_status_bullets(self):
        offenders = [
            line for line in self.text.splitlines() if STATUS_BULLET.search(line)
        ]
        self.assertEqual([], offenders, "a bullet leads with an implementation status")

    def test_no_dated_audit_snapshot(self):
        self.assertIsNone(
            DATED_SNAPSHOT.search(self.text),
            "a dated audit snapshot is a second source of truth that goes stale; "
            "the tests and the tracker issues are the current state",
        )

    def test_no_private_queue_reference(self):
        match = PRIVATE_QUEUE.search(self.text)
        self.assertIsNone(
            match,
            "private work-queue ids resolve only outside this repository; "
            "state the fact or cite a public issue instead",
        )

    def test_no_stale_portrait_gap_claims(self):
        match = PORTRAIT_GAP_CLAIM.search(self.text)
        self.assertIsNone(
            match,
            "the portrait-selector reference must not repeat the retired gap "
            "inventory after its detailed row records the regression evidence",
        )

    def test_cpp_anchors_survive(self):
        # The point of keeping the file is the anchors. If a future edit guts
        # the tables, this catches it before the reference becomes worthless.
        anchors = len(re.findall(r"`C4[A-Za-z0-9_]+(?:\.(?:cpp|h))?[.:][^`]*`", self.text))
        self.assertGreater(
            anchors,
            150,
            "docs/MENU_PARITY.md exists to preserve C++ anchors; retire it "
            "deliberately rather than letting them erode",
        )


if __name__ == "__main__":
    unittest.main()
