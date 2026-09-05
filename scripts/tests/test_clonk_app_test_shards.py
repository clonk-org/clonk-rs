"""Static exhaustiveness checks for the compile-time clonk-app test shards."""

import re
import tomllib
import unittest
from collections import Counter, defaultdict

from _repo import REPOSITORY

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
SHARED_CALL_PATTERN = re.compile(
    r'include_shared_main_test_fragment!\(\s*"([^"]+)"\s*,\s*"([^"]+)"'
    r'\s*,\s*"([^"]+)"\s*,?\s*\);',
    re.DOTALL,
)
EXPECTED_SHARDS = {
    "app-test-shard-1": {"netplay_1.rs"},
    "app-test-shard-2": {"menus_1.rs"},
    "app-test-shard-3": {"scenario_routes_1.rs"},
    "app-test-shard-4": {"audio.rs", "input.rs", "mouse_target_parity.rs"},
    "app-test-shard-5": {"chat_messages.rs", "lobby.rs"},
    "app-test-shard-6": {"game_over.rs"},
    "app-test-shard-7": {"scensel.rs", "startup.rs"},
    "app-test-shard-8": {"net_resources.rs", "saves.rs"},
    "app-test-shard-9": {"league.rs", "rendering.rs"},
    "app-test-shard-10": {"netplay_2.rs"},
    "app-test-shard-11": {
        "menus_2.rs",
        "scenario_routes_2.rs",
    },
    "app-test-shard-12": {"runtime.rs"},
}
EXPECTED_SHARED = [
    (
        "app-test-shard-3",
        "app-test-shard-11",
        "main_tests/scenario_routes_common.rs",
    )
]
# Opt-in probes are included directly by the test harness and intentionally
# stay out of the default compile-time shard union.
OPT_IN_FRAGMENTS = {"presentation_profile.rs": "presentation-profile"}


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
        for feature in OPT_IN_FRAGMENTS.values():
            self.assertIn(feature, features)
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
        shared_calls = SHARED_CALL_PATTERN.findall(source)
        assigned = defaultdict(set)
        paths = []
        for selector, relative in calls:
            self.assertIn(selector, selectors)
            path = (APP / "src" / relative).resolve()
            self.assertEqual(path.parent, FRAGMENTS.resolve())
            assigned[selector].add(path.name)
            paths.append(path)

        self.assertEqual(shared_calls, EXPECTED_SHARED)
        for first, second, relative in shared_calls:
            path = (APP / "src" / relative).resolve()
            self.assertEqual(path.parent, FRAGMENTS.resolve())
            paths.append(path)

        self.assertEqual(dict(assigned), EXPECTED_SHARDS)
        discovered = [
            path.resolve()
            for path in FRAGMENTS.glob("*.rs")
            if path.name not in OPT_IN_FRAGMENTS
        ]
        self.assertEqual(Counter(paths), Counter(discovered))
        called_selectors = {selector for selector, _ in calls}
        called_selectors.update(
            selector for first, second, _ in shared_calls for selector in (first, second)
        )
        self.assertEqual(called_selectors, selectors)
        self.assertEqual(source.count("macro_rules! include_main_test_fragment"), 1)
        self.assertEqual(source.count("macro_rules! include_shared_main_test_fragment"), 1)
        self.assertNotIn("include_split_main_test_fragment", source)
        direct_includes = re.findall(
            r'include!\(\s*"main_tests/([^"]+)"\s*\)', source
        )
        self.assertEqual(
            direct_includes,
            sorted(OPT_IN_FRAGMENTS),
            "only registered opt-in probes may bypass the shard gate",
        )
        for fragment, feature in OPT_IN_FRAGMENTS.items():
            self.assertRegex(
                source,
                rf'#\[cfg\(all\(test,\s*feature = "{re.escape(feature)}"\)\)\]'
                rf'\s*include!\(\s*"main_tests/{re.escape(fragment)}"\s*\);',
                f"{fragment} must be gated by its opt-in feature",
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
            "shared support must compile for either consumer and ordinary tests",
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

        for fragment in ("netplay", "scenario_routes", "menus"):
            for index in (1, 2):
                part = (FRAGMENTS / f"{fragment}_{index}.rs").read_text(
                    encoding="utf-8"
                )
                self.assertNotIn(f"mod {fragment}_shard_", part)
                self.assertNotIn("#[rustfmt::skip]", part)
                self.assertNotIn("use super::*;", part)

    def test_shared_harness_tests_run_once_in_shard_five(self):
        source = HARNESS.read_text(encoding="utf-8")
        tests = re.findall(r"(?m)^#\[(?:tokio::)?test\]", source)
        gated_tests = re.findall(
            r'(?m)^#\[cfg\(any\(not\(feature = "app-test-shard-mode"\), '
            r'feature = "app-test-shard-5"\)\)\]\n#\[(?:tokio::)?test\]',
            source,
        )

        self.assertTrue(tests, "the shared harness test inventory changed")
        self.assertEqual(len(gated_tests), len(tests))

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
