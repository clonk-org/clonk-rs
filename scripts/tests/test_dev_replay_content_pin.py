"""The dev-replay goldens are recorded against one content commit.

`testdata/dev-replays/*.json` stores frame checkpoints whose values depend on
material indices, and those follow the unpacked content's directory order
(clonk-org/clonk-rs#382). A content bump therefore invalidates them.

That has now happened three times (clonk-org/clonk-rs#1135, #1142, #1144) and
again after clonk-org/clonk-rs#1148, each time surfacing only in the
post-landing macOS material-order job -- which does not gate the queue, so the
bump lands green and the diagnostic goes red afterwards
(clonk-org/clonk-rs#1152).

This check fails the bump instead. It is deliberately platform-independent and
cheap: it compares a recorded pin against the submodule gitlink and never runs
a replay, so it belongs in the landing gate where the real check cannot go.

Re-record on macOS. Directory order is a property of the recording host, not
of the content, so the three hosts disagree: for `tutorial01-idle.json` at
content `ab9094f9`, macOS produces `79d4ca59bf4af7d4` and a Linux box produces
`d2cff76cc8044f0e`. Only the macOS job asserts these values, so goldens
re-recorded anywhere else strand it exactly as a stale pin would --
clonk-org/clonk-rs#1406 bumped content, re-recorded off-host, and reddened the
release qualification it was meant to keep green. A bump that leaves material
order untouched needs no new hashes at all; check the content diff for
`Material.c4g` before recording anything.

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
            "goldens. Material indices follow the unpacked content's directory "
            "order, so the committed checkpoints in testdata/dev-replays are "
            "now stale and the macOS material-order job will go red after this "
            "lands. Re-record them on macOS -- that job is the only one that "
            "asserts them, and the hashes follow the recording host -- and "
            "update testdata/dev-replays/content-pin.txt in the same change.",
        )


if __name__ == "__main__":
    unittest.main()
