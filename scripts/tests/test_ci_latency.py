"""Static guards for the landing path's coverage and latency budget."""

import re
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
LANDING = REPOSITORY / ".github" / "workflows" / "landing.yml"
MAIN = REPOSITORY / ".github" / "workflows" / "rust.yml"
DEPENDENCY_GUARD = REPOSITORY / ".github" / "workflows" / "dependency-guard.yml"


class CiLatencyTests(unittest.TestCase):
    def test_restore_only_landing_caches_have_trusted_main_producers(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")

        scopes = set(re.findall(r"shared-key: ([a-z0-9-]+)", landing))
        self.assertEqual(scopes, {"full-parity", "windows-runtime-msvc"})
        self.assertEqual(landing.count("save-if: false"), 3)
        for scope in scopes:
            with self.subTest(scope=scope):
                self.assertIn(f"shared-key: {scope}", main)

        linux_producer = main[main.index("  linux-landing-cache:") : main.index("  coverage:")]
        self.assertIn("workspaces: . -> target", linux_producer)
        self.assertIn("shared-key: full-parity", linux_producer)
        self.assertNotIn("cache-on-failure:", linux_producer)
        self.assertIn("id: linux-cache", linux_producer)
        self.assertIn(
            "if: steps.linux-cache.outputs.cache-hit != 'true'",
            linux_producer,
        )
        self.assertIn(
            "cargo nextest run --workspace --features xtask/engine-tools "
            "--no-run --locked",
            linux_producer,
        )

        windows_producer = main[main.index("  windows-release-tools:") :]
        self.assertIn("shared-key: windows-runtime-msvc", windows_producer)
        self.assertNotIn("cache-on-failure:", windows_producer)

    def test_cache_producers_finish_while_obsolete_diagnostics_cancel(self):
        main = MAIN.read_text(encoding="utf-8")

        self.assertNotRegex(main, r"(?m)^concurrency:\s*$")
        linux_producer = main[main.index("  linux-landing-cache:") : main.index("  coverage:")]
        coverage = main[main.index("  coverage:") : main.index("  recording-host-oracles:")]
        recording_host = main[
            main.index("  recording-host-oracles:") : main.index("  windows-release-tools:")
        ]
        windows_producer = main[main.index("  windows-release-tools:") :]

        for producer in (linux_producer, windows_producer):
            self.assertIn("cancel-in-progress: false", producer)
        for diagnostic in (coverage, recording_host):
            self.assertIn("cancel-in-progress: true", diagnostic)
        self.assertIn("needs: linux-landing-cache", coverage)
        self.assertIn("save-if: false", coverage)

    def test_normal_workspace_is_an_exhaustive_compile_time_partition(self):
        workflow = LANDING.read_text(encoding="utf-8")

        app_selectors = re.findall(
            r"cargo nextest run -p clonk-app --features (app-test-shard-[1-9][0-9]*)",
            workflow,
        )
        engine_selectors = re.findall(
            r"cargo nextest run -p clonk-engine-integration-tests --test engine_it "
            r"--features (engine-it-shard-[1-9][0-9]*)",
            workflow,
        )
        self.assertEqual(app_selectors, [f"app-test-shard-{index}" for index in range(1, 6)])
        self.assertEqual(engine_selectors, ["engine-it-shard-1", "engine-it-shard-2"])
        self.assertIn(
            "cargo nextest run -p clonk-engine-unit-tests --no-fail-fast --locked",
            workflow,
        )
        self.assertIn(
            "cargo nextest run -p clonk-frontend-unit-tests --no-fail-fast --locked",
            workflow,
        )
        remainder = next(
            line.strip()
            for line in workflow.splitlines()
            if "cargo nextest run --workspace --exclude clonk-app" in line
        )
        for package in (
            "clonk-app",
            "clonk-engine-integration-tests",
            "clonk-engine-unit-tests",
            "clonk-frontend-unit-tests",
        ):
            self.assertIn(f"--exclude {package}", remainder)
        self.assertNotIn(
            "cargo nextest run --workspace --no-fail-fast --locked",
            workflow,
        )

    def test_literal_required_commands_remain_on_the_landing_tree(self):
        workflow = LANDING.read_text(encoding="utf-8")
        required = (
            "cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings",
            "cargo xtask parity verify",
            "cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked",
            "cargo xtask engine-snapshots verify",
            "cargo fmt --all -- --check",
            "python3 -m unittest discover -s scripts/tests -p 'test_*.py'",
        )
        for command in required:
            with self.subTest(command=command):
                self.assertIn(command, workflow)

        self.assertIn("fetch-depth: 0", workflow)
        self.assertIn("python3-pil", workflow)

    def test_slow_diagnostics_are_post_merge_and_release_sha_is_not_cancelled(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")

        self.assertNotIn("cargo llvm-cov", landing)
        self.assertNotIn("runs-on: macos-latest", landing)
        self.assertIn("cargo llvm-cov", main)
        self.assertIn("runs-on: macos-latest", main)
        self.assertIn(
            "startsWith(github.event.head_commit.message, 'chore: release ')",
            main,
        )
        self.assertIn("&& github.sha ||", main)
        self.assertIn("cancel-in-progress: true", main)

    def test_post_merge_render_probe_consumes_a_fresh_deterministic_replay(self):
        workflow = MAIN.read_text(encoding="utf-8")
        replay = workflow.index("- name: Generate deterministic replay evidence")
        render = workflow.index("- name: Render the replay snapshot")
        coverage = workflow.index("- name: Collect instrumented workspace coverage")

        self.assertLess(replay, render)
        self.assertLess(render, coverage)
        self.assertIn(
            "dev_feedback_replay::real_scenario_replays_repeat_with_native_group_order",
            workflow[replay:render],
        )
        self.assertIn("dev_feedback_render --ignored --exact", workflow[render:coverage])

    def test_dependency_guard_does_not_repeat_the_full_packaging_gate(self):
        workflow = DEPENDENCY_GUARD.read_text(encoding="utf-8")
        self.assertIn(
            "cargo check --workspace --features xtask/engine-tools --locked",
            workflow,
        )
        self.assertNotIn(
            "cargo test -p xtask --features engine-tools "
            "--bin xtask-engine-tools --locked",
            workflow,
        )

    def test_parity_uses_the_lightweight_xtask_dispatcher(self):
        dispatcher = (REPOSITORY / "xtask" / "src" / "dispatcher.rs").read_text(
            encoding="utf-8"
        )
        lightweight = 'Some("parity") => return xtask::parity::command(&args[1..]),'
        self.assertIn(lightweight, dispatcher)
        self.assertLess(
            dispatcher.index(lightweight),
            dispatcher.index("Command::new(cargo)"),
        )


if __name__ == "__main__":
    unittest.main()
