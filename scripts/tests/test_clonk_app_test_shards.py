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
SPLIT_CALL_PATTERN = re.compile(
    r'include_split_main_test_fragment!\(\s*"([^"]+)"\s*,\s*"([^"]+)"'
    r'\s*,\s*"([^"]+)"\s*,?\s*\);',
    re.DOTALL,
)
EXPECTED_SHARDS = {
    "app-test-shard-1": {"netplay.rs::netplay_shard_1"},
    "app-test-shard-2": {"menus.rs"},
    "app-test-shard-3": {"scenario_routes.rs::scenario_routes_shard_1"},
    "app-test-shard-4": {"audio.rs", "input.rs"},
    "app-test-shard-5": {"chat_messages.rs", "lobby.rs"},
    "app-test-shard-6": {"game_over.rs"},
    "app-test-shard-7": {"scensel.rs", "startup.rs"},
    "app-test-shard-8": {"net_resources.rs", "saves.rs"},
    "app-test-shard-9": {"league.rs", "rendering.rs"},
    "app-test-shard-10": {"netplay.rs::netplay_shard_2"},
    "app-test-shard-11": {
        "runtime.rs",
        "scenario_routes.rs::scenario_routes_shard_2",
    },
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
        split_calls = SPLIT_CALL_PATTERN.findall(source)
        assigned = defaultdict(set)
        paths = []
        for selector, relative in calls:
            self.assertIn(selector, selectors)
            path = (APP / "src" / relative).resolve()
            self.assertEqual(path.parent, FRAGMENTS.resolve())
            assigned[selector].add(path.name)
            paths.append(path)

        self.assertEqual(
            split_calls,
            [
                (
                    "app-test-shard-3",
                    "app-test-shard-11",
                    "main_tests/scenario_routes.rs",
                ),
                (
                    "app-test-shard-1",
                    "app-test-shard-10",
                    "main_tests/netplay.rs",
                )
            ],
        )
        for first, second, relative in split_calls:
            path = (APP / "src" / relative).resolve()
            self.assertEqual(path.parent, FRAGMENTS.resolve())
            assigned[first].add(f"{path.name}::{path.stem}_shard_1")
            assigned[second].add(f"{path.name}::{path.stem}_shard_2")
            paths.append(path)

        self.assertEqual(dict(assigned), EXPECTED_SHARDS)
        discovered = [path.resolve() for path in FRAGMENTS.glob("*.rs")]
        self.assertEqual(Counter(paths), Counter(discovered))
        called_selectors = {selector for selector, _ in calls}
        called_selectors.update(
            selector for first, second, _ in split_calls for selector in (first, second)
        )
        self.assertEqual(called_selectors, selectors)
        self.assertEqual(source.count("macro_rules! include_main_test_fragment"), 1)
        self.assertEqual(source.count("macro_rules! include_split_main_test_fragment"), 1)
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
        self.assertIn(
            '#[cfg(any(not(feature="app-test-shard-mode"),feature=$first,'
            'feature=$second))]include!($path);',
            compact,
            "the split fragment must compile for either half and ordinary tests",
        )
        selector_guard = ",".join(
            f'feature="{selector}"'
            for selector in sorted(selectors, key=lambda name: int(name.rsplit("-", 1)[1]))
        )
        self.assertIn(
            f'#[cfg(all(feature="{SENTINEL}",not(any({selector_guard}))))]'
            "compile_error!",
            compact,
            "the internal sentinel alone must not silently produce an empty shard",
        )

        for fragment, first, second in (
            ("netplay", "app-test-shard-1", "app-test-shard-10"),
            ("scenario_routes", "app-test-shard-3", "app-test-shard-11"),
        ):
            source = (FRAGMENTS / f"{fragment}.rs").read_text(encoding="utf-8")
            compact_fragment = normalized_rust(source)
            for index, selector in enumerate((first, second), start=1):
                self.assertIn(
                    '#[cfg(any(not(feature="app-test-shard-mode"),'
                    f'feature="{selector}"))]mod{fragment}_shard_{index}'
                    "{usesuper::*;",
                    compact_fragment,
                )

    def test_inline_test_modules_run_once_in_shard_five(self):
        expected_cfg = normalized_rust(
            """#[cfg(all(
                test,
                any(
                    not(feature = "app-test-shard-mode"),
                    feature = "app-test-shard-5",
                ),
            ))]"""
        )
        discovered = []
        module_pattern = re.compile(r"(?m)^\s*mod [A-Za-z0-9_]*tests\s*\{")

        for path in (APP / "src").rglob("*.rs"):
            if path == HARNESS or FRAGMENTS in path.parents:
                continue
            source = path.read_text(encoding="utf-8")
            for module in module_pattern.finditer(source):
                cfg_start = source.rfind("#[cfg(", 0, module.start())
                self.assertGreaterEqual(cfg_start, 0, path)
                self.assertEqual(
                    normalized_rust(source[cfg_start : module.start()]),
                    expected_cfg,
                    f"{path.relative_to(REPOSITORY)} must assign inline tests to shard 5",
                )
                discovered.append((path, module.group()))

        self.assertGreaterEqual(len(discovered), 30)


if __name__ == "__main__":
    unittest.main()
