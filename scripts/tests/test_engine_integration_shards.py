"""Static guards for the clonk-engine integration-test feature shards."""

import re
import tomllib
import unittest
from collections import Counter
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
MANIFEST = REPOSITORY / "crates" / "clonk-engine-integration-tests" / "Cargo.toml"
HARNESS = REPOSITORY / "crates" / "clonk-engine" / "tests" / "it"
SENTINEL = "engine-it-sharded"
SELECTORS = ("engine-it-shard-1", "engine-it-shard-2")
RECORDING_HOST_TESTS = {
    "dev_feedback_replay": "committed_real_scenario_replays_are_deterministic",
    "elevator_motion_oracle": "tutorial07_seed_zero_landscape_matches_cpp_surface8",
    "real_scenario_harness": "alchemy_real_scenario_subcases_batch_4",
    "real_tutorial02_virtual_play": (
        "tutorial02_virtual_player_completes_the_real_tutorial_route"
    ),
}


def compact(source: str) -> str:
    return " ".join(source.split())


def squash(source: str) -> str:
    return re.sub(r"\s+", "", source)


class EngineIntegrationShardTests(unittest.TestCase):
    def test_shards_form_an_exact_default_preserving_partition(self):
        manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
        features = manifest["features"]
        source = (HARNESS / "main.rs").read_text(encoding="utf-8")
        compact_source = compact(source)
        squashed_source = squash(source)

        self.assertEqual(features[SENTINEL], [])
        self.assertEqual(features["default"], [])
        for selector in SELECTORS:
            self.assertEqual(features[selector], [SENTINEL])

        selector_features = {
            feature
            for feature, dependencies in features.items()
            if dependencies == [SENTINEL]
        }
        self.assertEqual(selector_features, set(SELECTORS))

        calls = re.findall(
            r'shard_modules!\(\s*"([^"]+)",(?P<body>.*?)\n\);',
            source,
            re.DOTALL,
        )
        self.assertEqual(Counter(selector for selector, _ in calls), Counter(SELECTORS))

        assignments = {
            selector: re.findall(r"\b([a-z][a-z0-9_]*)\s*,", body)
            for selector, body in calls
        }
        assigned = Counter(
            module for modules in assignments.values() for module in modules
        )
        inventory = {path.stem for path in HARNESS.glob("*.rs")} - {"main"}
        self.assertEqual(set(assigned), inventory)
        self.assertTrue(all(count == 1 for count in assigned.values()))
        self.assertNotIn("support", assigned)
        self.assertRegex(source, r"(?m)^mod support;$")

        self.assertIn(
            '#[cfg(any( not(feature = "engine-it-sharded"), '
            'feature = $selector, ))] mod $module;',
            compact_source,
        )
        self.assertIn(
            '#[cfg(all(feature="engine-it-sharded",not(any('
            'feature="engine-it-shard-1",feature="engine-it-shard-2",)),))]'
            "compile_error!(",
            squashed_source,
        )

        shard_one = set(assignments[SELECTORS[0]])
        shard_two = set(assignments[SELECTORS[1]])
        self.assertTrue(RECORDING_HOST_TESTS.keys() <= shard_one)
        self.assertTrue(RECORDING_HOST_TESTS.keys().isdisjoint(shard_two))
        for module, test in RECORDING_HOST_TESTS.items():
            module_source = (HARNESS / f"{module}.rs").read_text(encoding="utf-8")
            self.assertRegex(module_source, rf"\bfn\s+{re.escape(test)}\s*\(")

        support_source = squash(
            (HARNESS / "support" / "dev_feedback.rs").read_text(encoding="utf-8")
        )
        self.assertIn(
            '#[cfg(any(not(feature="engine-it-sharded"),'
            'feature="engine-it-shard-1",))]#[test]fn'
            "snapshot_hash_ignores_serialized_debug_draw_sidecars(",
            support_source,
        )


if __name__ == "__main__":
    unittest.main()
