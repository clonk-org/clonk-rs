"""Static exhaustiveness checks for the compile-time clonk-app test shards."""

import re
import tomllib
import unittest
from collections import Counter, defaultdict
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
APP = REPOSITORY / "crates" / "clonk-app"
MANIFEST = APP / "Cargo.toml"
HARNESS = APP / "src" / "main_tests.rs"
FRAGMENTS = APP / "src" / "main_tests"
SENTINEL = "app-test-shard-mode"
SELECTOR_PATTERN = re.compile(r"app-test-shard-[1-9][0-9]*\Z")
CALL_PATTERN = re.compile(
    r'include_main_test_fragment!\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,?\s*\);',
    re.DOTALL,
)
EXPECTED_SHARDS = {
    "app-test-shard-1": {"net_resources.rs", "netplay.rs"},
    "app-test-shard-2": {
        "audio.rs",
        "chat_messages.rs",
        "menus.rs",
        "scensel.rs",
    },
    "app-test-shard-3": {"league.rs", "scenario_routes.rs", "startup.rs"},
    "app-test-shard-4": {"game_over.rs", "input.rs", "rendering.rs"},
    "app-test-shard-5": {"lobby.rs", "runtime.rs", "saves.rs"},
}


def feature_closure(features, roots):
    """Return local Cargo features reachable from the named roots."""
    pending = list(roots)
    reached = set()
    while pending:
        feature = pending.pop()
        if feature in reached or feature not in features:
            continue
        reached.add(feature)
        pending.extend(
            dependency
            for dependency in features[feature]
            if "/" not in dependency and not dependency.startswith("dep:")
        )
    return reached


def normalized_rust(source):
    """Remove formatting-only whitespace and trailing meta-item commas."""
    compact = re.sub(r"\s+", "", source)
    while ",)" in compact:
        compact = compact.replace(",)", ")")
    return compact


class ClonkAppTestShardTests(unittest.TestCase):
    def test_fragments_form_the_exhaustive_default_full_shard_union(self):
        manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
        features = manifest.get("features", {})
        selectors = {name for name in features if SELECTOR_PATTERN.fullmatch(name)}

        self.assertIn(SENTINEL, features)
        self.assertEqual(features[SENTINEL], [])
        self.assertEqual(selectors, set(EXPECTED_SHARDS))
        for selector in selectors:
            with self.subTest(selector=selector):
                self.assertEqual(features[selector], [SENTINEL])
        self.assertNotIn(
            SENTINEL,
            feature_closure(features, features.get("default", [])),
            "ordinary cargo test must leave shard mode disabled and include every fragment",
        )

        source = HARNESS.read_text(encoding="utf-8")
        calls = CALL_PATTERN.findall(source)
        assigned = defaultdict(set)
        paths = []
        for selector, relative in calls:
            self.assertIn(selector, selectors)
            path = (APP / "src" / relative).resolve()
            self.assertEqual(path.parent, FRAGMENTS.resolve())
            assigned[selector].add(path.name)
            paths.append(path)

        self.assertEqual(dict(assigned), EXPECTED_SHARDS)
        discovered = [path.resolve() for path in FRAGMENTS.glob("*.rs")]
        self.assertEqual(Counter(paths), Counter(discovered))
        self.assertEqual({selector for selector, _ in calls}, selectors)
        self.assertEqual(source.count("macro_rules! include_main_test_fragment"), 1)
        self.assertEqual(
            re.findall(r'include!\(\s*"main_tests/[^"]+"\s*\)', source),
            [],
            "every fragment must pass through the single shard gate",
        )

        compact = normalized_rust(source)
        self.assertIn(
            '#[cfg(any(not(feature="app-test-shard-mode"),feature=$selector))]'
            "include!($path);",
            compact,
            "without shard mode the macro must include every fragment",
        )
        selector_guard = ",".join(
            f'feature="{selector}"' for selector in sorted(selectors)
        )
        self.assertIn(
            f'#[cfg(all(feature="{SENTINEL}",not(any({selector_guard}))))]'
            "compile_error!",
            compact,
            "the internal sentinel alone must not silently produce an empty shard",
        )


if __name__ == "__main__":
    unittest.main()
