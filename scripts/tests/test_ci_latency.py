"""Static guards for the landing path's coverage and latency budget."""

import re
import tomllib
import unittest
from collections import Counter
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
LANDING = REPOSITORY / ".github" / "workflows" / "landing.yml"
MAIN = REPOSITORY / ".github" / "workflows" / "rust.yml"
DEPENDENCY_GUARD = REPOSITORY / ".github" / "workflows" / "dependency-guard.yml"


def matrix_entry(workflow, name):
    """Return one literal Linux matrix entry."""
    marker = f"          - name: {name}\n"
    start = workflow.index(marker)
    end = workflow.find("\n          - name: ", start + len(marker))
    return workflow[start : end if end >= 0 else len(workflow)]


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

    def test_post_merge_work_leaves_the_next_landing_runner_budget(self):
        main = MAIN.read_text(encoding="utf-8")
        dependency_guard = DEPENDENCY_GUARD.read_text(encoding="utf-8")

        recording_host = main[
            main.index("  recording-host-oracles:") : main.index(
                "  windows-release-tools:"
            )
        ]
        self.assertIn("needs: linux-landing-cache", recording_host)

        triggers = dependency_guard[
            dependency_guard.index("on:\n") : dependency_guard.index("permissions:\n")
        ]
        self.assertNotIn("\n  push:\n", triggers)

    def test_normal_workspace_is_an_exhaustive_compile_time_partition(self):
        workflow = LANDING.read_text(encoding="utf-8")

        app_commands = re.findall(
            r"cargo nextest run -p clonk-app --features (app-test-shard-[1-9][0-9]*)"
            r"(?: --partition (hash:[12]/2))? --no-fail-fast --locked",
            workflow,
        )
        self.assertEqual(
            app_commands,
            [("app-test-shard-1", "hash:1/2"), ("app-test-shard-1", "hash:2/2")]
            + [(f"app-test-shard-{index}", "") for index in range(2, 10)],
        )
        engine_command = (
            "cargo nextest run -p clonk-engine-integration-tests --test engine_it "
            "--no-fail-fast --locked"
        )
        self.assertEqual(workflow.count(engine_command), 1)
        self.assertNotIn("--features engine-it-shard-", workflow)
        unit_and_parity = matrix_entry(workflow, "workspace unit and parity")
        self.assertIn(
            "cargo nextest run -p clonk-engine-unit-tests "
            "-p clonk-frontend-unit-tests --no-fail-fast --locked",
            unit_and_parity,
        )
        self.assertIn("cargo xtask parity verify", unit_and_parity)
        dedicated_packages = {
            "clonk-app",
            "clonk-engine-integration-tests",
            "clonk-engine-unit-tests",
            "clonk-frontend-unit-tests",
        }
        workspace = tomllib.loads(
            (REPOSITORY / "Cargo.toml").read_text(encoding="utf-8")
        )["workspace"]
        workspace_packages = {
            tomllib.loads(
                (REPOSITORY / member / "Cargo.toml").read_text(encoding="utf-8")
            )["package"]["name"]
            for member in workspace["members"]
        }
        remaining_shards = re.findall(
            r"          - name: remaining workspace ([1-9][0-9]*)/([1-9][0-9]*)\n"
            r"(?:            apt: [^\n]+\n)?"
            r"            command: cargo nextest run (.*?) --no-fail-fast --locked",
            workflow,
        )
        self.assertEqual(
            [(index, total) for index, total, _ in remaining_shards],
            [("1", "2"), ("2", "2")],
        )
        remaining_packages = Counter()
        for _, _, arguments in remaining_shards:
            tokens = arguments.split()
            self.assertEqual(tokens[::2], ["-p"] * (len(tokens) // 2))
            remaining_packages.update(tokens[1::2])
        self.assertEqual(
            remaining_packages,
            Counter(workspace_packages - dedicated_packages),
        )
        self.assertIn("-p clonk-app-netplay", remaining_shards[0][2])
        self.assertNotIn("-p clonk-app-netplay", remaining_shards[1][2])
        self.assertNotIn(
            "cargo nextest run --workspace --no-fail-fast --locked",
            workflow,
        )

    def test_overlapping_linux_checks_share_setup_without_failing_open(self):
        workflow = LANDING.read_text(encoding="utf-8")
        unit_and_parity = matrix_entry(workflow, "workspace unit and parity")
        quality = matrix_entry(workflow, "workspace quality")

        for entry in (unit_and_parity, quality):
            self.assertIn("failed=0", entry)
            self.assertIn('exit "$failed"', entry)
        self.assertIn("cargo xtask parity verify || failed=1", unit_and_parity)
        self.assertIn("cargo clippy --version || failed=1", quality)
        self.assertIn("rustfmt --version || failed=1", quality)
        for command in (
            "cargo fmt --all -- --check || failed=1",
            "python3 -m unittest discover -s scripts/tests -p 'test_*.py' || failed=1",
            "cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings || failed=1",
        ):
            self.assertIn(command, quality)
        for old_name in (
            "engine unit",
            "frontend unit",
            "workspace lints",
            "C++ parity",
            "repository hygiene",
        ):
            self.assertNotIn(f"          - name: {old_name}\n", workflow)
        self.assertNotIn("components: clippy, rustfmt", workflow)

    def test_quality_fetches_only_the_pinned_oracle_history(self):
        workflow = LANDING.read_text(encoding="utf-8")
        quality = matrix_entry(workflow, "workspace quality")
        fetch = (
            "git fetch --no-tags --depth=1 origin "
            "7d43b47b7d789b533f32d005e64596e0a07019cd"
        )

        self.assertIn(fetch, quality)
        self.assertLess(quality.index(fetch), quality.index("python3 -m unittest"))

    def test_linux_setup_is_pinned_fast_and_matrix_scoped(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]

        self.assertIn("runs-on: ubuntu-24.04", linux)
        self.assertNotIn("filter: blob:none", linux)
        app_rows = ["app netplay 1/2", "app netplay 2/2"] + [
            f"app {index}/9" for index in range(2, 10)
        ]
        for name in app_rows:
            self.assertIn(
                "apt: libasound2-dev libudev-dev",
                matrix_entry(workflow, name),
            )
        expected_apt = {
            "remaining workspace 1/2": "mesa-vulkan-drivers",
            "remaining workspace 2/2": "libasound2-dev libxmp4",
            "workspace quality": "libasound2-dev libudev-dev python3-pil",
        }
        for name, packages in expected_apt.items():
            self.assertIn(f"apt: {packages}", matrix_entry(workflow, name))
        for name in (
            "engine integration",
            "workspace unit and parity",
            "engine contracts",
        ):
            self.assertNotIn("\n            apt:", matrix_entry(workflow, name))

        self.assertIn("if: matrix.apt != ''", linux)
        self.assertIn("if ! sudo apt-get install", linux)
        self.assertEqual(linux.count("sudo apt-get update"), 1)
        self.assertIn("rustc 1.97.1", linux)
        self.assertIn("id: preinstalled-rust", linux)
        self.assertIn("if: steps.preinstalled-rust.outputs.exact != 'true'", linux)

    def test_hosted_toolchains_and_cached_registry_are_reused_safely(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]
        windows_smoke = workflow[
            workflow.index("  windows-smoke:") : workflow.index("  runtime-msvc:")
        ]
        runtime = workflow[
            workflow.index("  runtime-msvc:") : workflow.index("  landing-gate:")
        ]

        for job in (linux, windows_smoke, runtime):
            self.assertIn("rustup toolchain list", job)
            self.assertIn('rustup run "$toolchain" rustc --version', job)
            self.assertIn(
                'rustup run "$toolchain" rustc --print sysroot',
                job,
            )
            self.assertIn('echo "$sysroot/bin" >> "$GITHUB_PATH"', job)
            self.assertIn('CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}', job)
            self.assertNotIn('RUSTUP_TOOLCHAIN=$toolchain', job)
            self.assertIn("if: steps.preinstalled-rust.outputs.exact != 'true'", job)

        self.assertIn("if ! cargo fetch --locked --offline; then", runtime)
        self.assertIn("cargo fetch --locked", runtime)

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

        self.assertIn("fetch-depth: 1", workflow)
        self.assertNotIn("fetch-depth: 0", workflow)
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
