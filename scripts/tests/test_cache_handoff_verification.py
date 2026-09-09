"""Static guards for retrying cache handoff verification."""

import re
import unittest

from _repo import REPOSITORY


WORKFLOWS = REPOSITORY / ".github" / "workflows"
ACTION = REPOSITORY / ".github" / "actions" / "verify-cache-handoff" / "action.yml"
CACHE_ACTION = "actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9"


def steps(text):
    """Split a workflow or action into its verbatim step blocks."""
    boundaries = [match.start() for match in re.finditer(r"(?m)^ *- ", text)]
    return [
        text[start : boundaries[index + 1] if index + 1 < len(boundaries) else len(text)]
        for index, start in enumerate(boundaries)
    ]


class CacheHandoffVerificationTests(unittest.TestCase):
    def test_the_handoff_verifier_polls_before_declaring_a_save_lost(self):
        action = ACTION.read_text(encoding="utf-8")

        self.assertIn("using: composite", action)
        for name in ("path", "key"):
            with self.subTest(input_name=name):
                self.assertRegex(
                    action, rf"(?m)^  {name}:\n(?:    .*\n)*?    required: true\n"
                )

        lookups = [step for step in steps(action) if CACHE_ACTION in step]
        self.assertGreaterEqual(len(lookups), 3)
        for lookup in lookups:
            with self.subTest(lookup=lookup):
                self.assertIn("lookup-only: true", lookup)
                self.assertIn("path: ${{ inputs.path }}", lookup)
                self.assertIn("key: ${{ inputs.key }}", lookup)

        # Only the last attempt fails the row; the earlier ones feed the retry.
        self.assertNotIn("fail-on-cache-miss", "".join(lookups[:-1]))
        self.assertIn("fail-on-cache-miss: true", lookups[-1])

        waits = [step for step in steps(action) if "sleep " in step]
        self.assertGreaterEqual(len(waits), 2)
        self.assertGreaterEqual(
            sum(int(match) for match in re.findall(r"sleep (\d+)", action)), 10
        )

        # Every retry is gated on all of its predecessors: a skipped step
        # reports no cache-hit, so a partial condition would retry after a hit.
        attempts = re.findall(r"(?m)^    - name: .*\n(?:      .*\n)*", action)
        seen = 0
        for attempt in attempts:
            condition = re.search(r"if: (?P<condition>.*)", attempt)
            if condition is None:
                seen += 1
                continue
            self.assertEqual(
                condition.group("condition").count("!= 'true'"),
                seen,
                msg=attempt,
            )
            if CACHE_ACTION in attempt:
                seen += 1

    def test_no_workflow_verifies_its_own_save_with_a_single_lookup(self):
        for workflow in sorted(WORKFLOWS.glob("*.yml")):
            text = workflow.read_text(encoding="utf-8")
            for step in steps(text):
                if CACHE_ACTION not in step:
                    continue
                with self.subTest(workflow=workflow.name, step=step):
                    self.assertNotIn(
                        "lookup-only: true",
                        step if "fail-on-cache-miss: true" in step else "",
                    )

    def test_every_producer_hands_its_entry_off_through_the_verifier(self):
        verifiers = {
            workflow.name: workflow.read_text(encoding="utf-8").count(
                "uses: ./.github/actions/verify-cache-handoff"
            )
            for workflow in sorted(WORKFLOWS.glob("*.yml"))
        }

        self.assertEqual(verifiers["release-prebuild.yml"], 2)
        self.assertEqual(verifiers["rust.yml"], 1)

        prebuild = (WORKFLOWS / "release-prebuild.yml").read_text(encoding="utf-8")
        for step in steps(prebuild):
            if "uses: ./.github/actions/verify-cache-handoff" not in step:
                continue
            with self.subTest(step=step):
                self.assertIn(
                    "path: target/release-prebuild/${{ matrix.artifact }}", step
                )
                self.assertIn(
                    "key: release-prebuild-${{ matrix.artifact }}-"
                    "${{ inputs.source-sha }}-${{ github.run_id }}",
                    step,
                )

        content = (WORKFLOWS / "rust.yml").read_text(encoding="utf-8")
        verifier = next(
            step
            for step in steps(content)
            if "uses: ./.github/actions/verify-cache-handoff" in step
        )
        self.assertIn("if: steps.content-cache.outputs.cache-hit != 'true'", verifier)
        self.assertIn("path: .git/modules/content", verifier)
        self.assertIn(
            "key: clonk-content-git-v1-${{ runner.os }}-"
            "${{ hashFiles('.gitmodules') }}-${{ steps.content.outputs.revision }}",
            verifier,
        )


if __name__ == "__main__":
    unittest.main()
