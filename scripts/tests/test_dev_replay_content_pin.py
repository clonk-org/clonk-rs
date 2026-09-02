"""The dev-replay goldens are recorded against one content commit.

`testdata/dev-replays/*.json` stores frame checkpoints whose values depend on
material indices. Folder-backed groups enumerate in stored-name byte order
(clonk-org/clonk-rs#1455), so the goldens are host-independent; a content bump
that changes `Material.c4g` can still invalidate them.

This check fails the bump instead. It is deliberately cheap: it compares a
recorded pin against the submodule gitlink and never runs a replay, so it
belongs in the landing gate. A bump that leaves material files untouched needs
no new hashes; check the content diff for `Material.c4g` before recording
anything.

`compat/profile.json` protects its own content claims the same way, via
`pinned.content_commit`.
"""

import pathlib
import subprocess
import unittest

REPOSITORY = pathlib.Path(__file__).resolve().parents[2]
PIN_FILE = REPOSITORY / "testdata" / "dev-replays" / "content-pin.txt"


def submodule_pin():
    completed = subprocess.run(
        ["git", "ls-tree", "HEAD", "content"],
        capture_output=True,
        text=True,
        check=True,
        cwd=REPOSITORY,
    )
    # `160000 commit <sha>\tcontent`
    return completed.stdout.split()[2]


class DevReplayContentPinTest(unittest.TestCase):
    def test_recorded_pin_is_a_full_commit_sha(self):
        recorded = PIN_FILE.read_text(encoding="utf-8").strip()
        self.assertEqual(len(recorded), 40, f"not a full sha: {recorded!r}")
        self.assertTrue(
            all(character in "0123456789abcdef" for character in recorded),
            f"not lowercase hex: {recorded!r}",
        )

    def test_goldens_were_recorded_against_the_pinned_content(self):
        recorded = PIN_FILE.read_text(encoding="utf-8").strip()
        self.assertEqual(
            recorded,
            submodule_pin(),
            "The content submodule moved without re-recording the dev-replay "
            "goldens. Material indices follow folder-group entry order, so the "
            "committed checkpoints in testdata/dev-replays may now be stale. "
            "Re-record them and update testdata/dev-replays/content-pin.txt in "
            "the same change.",
        )


if __name__ == "__main__":
    unittest.main()
