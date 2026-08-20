"""Static guards for the landing path's coverage and latency budget."""

import re
import tomllib
import unittest
from collections import Counter

from _repo import REPOSITORY

WORKFLOWS = REPOSITORY / ".github" / "workflows"
LANDING = WORKFLOWS / "landing.yml"
MAIN = WORKFLOWS / "rust.yml"
QUALIFICATION = WORKFLOWS / "exact-sha-qualification.yml"
DEPENDENCY_GUARD = WORKFLOWS / "dependency-guard.yml"
APT_INSTALLER = REPOSITORY / "scripts" / "install-apt-packages.sh"

# One step, from its first key to the next step's. `actions/cache/restore` is
# deliberately not matched: only the save halves can consume the budget.
STEP = re.compile(r"(?ms)^      - (?:name|uses|id):.*?(?=^      - (?:name|uses|id):|\Z)")
CACHE_WRITER = re.compile(r"Swatinem/rust-cache@|actions/cache(?:/save)?@")


def matrix_entry(workflow, name):
    """Return one literal Linux matrix entry."""
    marker = f"          - name: {name}\n"
    start = workflow.index(marker)
    end = workflow.find("\n          - name: ", start + len(marker))
    return workflow[start : end if end >= 0 else len(workflow)]


def cache_steps(workflow):
    """Return every step that can publish an Actions cache entry."""
    return [step for step in STEP.findall(workflow) if CACHE_WRITER.search(step)]


def fires_on_a_non_default_ref(workflow):
    """Report whether an event can run this workflow off the default branch."""
    triggers = re.search(r"(?ms)^on:\n(.*?)^(?=[a-z])", workflow).group(1)
    return "pull_request:" in triggers or "merge_group:" in triggers


class CiLatencyTests(unittest.TestCase):
    def test_native_dependency_install_retries_are_shared_and_bounded(self):
        installer = APT_INSTALLER.read_text(encoding="utf-8")
        workflows = "\n".join(
            path.read_text(encoding="utf-8") for path in WORKFLOWS.glob("*.yml")
        )

        self.assertEqual(workflows.count("scripts/install-apt-packages.sh"), 6)
        self.assertNotIn("apt_install()", workflows)
        self.assertTrue(APT_INSTALLER.stat().st_mode & 0o111)
        self.assertLess(installer.index("apt_install"), installer.index("apt_refresh"))
        self.assertIn("timeout 240 sudo apt-get install", installer)
        self.assertIn("timeout 180 sudo apt-get update", installer)

    def test_restore_only_landing_caches_have_trusted_main_producers(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")

        scopes = set(re.findall(r"shared-key: ([a-z0-9-]+)", landing))
        self.assertEqual(
            scopes,
            {"full-parity", "windows-runtime-msvc-v2"},
        )
        self.assertEqual(landing.count("save-if: false"), 3)
        self.assertNotIn(
            "save-if: ${{ github.event_name == 'workflow_dispatch' }}",
            landing,
        )
        for scope in scopes:
            with self.subTest(scope=scope):
                self.assertIn(f"shared-key: {scope}", main)

        linux_producer = main[
            main.index("  linux-landing-cache:") : main.index(
                "  exact-sha-qualification:"
            )
        ]
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

        windows_producer = main[
            main.index("  windows-landing-cache:") : main.index(
                "  windows-release-tools:"
            )
        ]
        release_tools = main[main.index("  windows-release-tools:") :]
        self.assertIn("shared-key: windows-runtime-msvc-v2", windows_producer)
        self.assertIn("-p clonk-network", windows_producer)
        self.assertIn("cargo clippy --profile test --no-deps", windows_producer)
        compile_only = next(
            line
            for line in windows_producer.splitlines()
            if "cargo nextest run" in line and "--no-run" in line
        )
        self.assertNotIn("--no-fail-fast", compile_only)
        self.assertNotIn("cache-on-failure:", windows_producer)
        self.assertNotIn("cache-workspace-crates: true", windows_producer)
        self.assertIn("needs: windows-landing-cache", release_tools)
        self.assertNotIn("windows-runtime-msvc-v2", release_tools)
        self.assertEqual(release_tools.count("Swatinem/rust-cache@"), 1)

    def test_no_workflow_publishes_a_cache_only_its_own_ref_can_restore(self):
        # GitHub restores a cache from the current branch or the default one,
        # so an entry saved from `refs/pull/N/merge` or a merge-queue ref is
        # dead on arrival: nothing outside that one ref can ever read it, while
        # it still spends the repository's 10 GiB budget and evicts by LRU the
        # entries the merge queue and the shipped Windows build need.
        producers = set()
        consumers = {}
        for path in sorted(WORKFLOWS.glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            ref_scoped = fires_on_a_non_default_ref(workflow)
            for step in cache_steps(workflow):
                scope = re.search(r"shared-key: (\S+)", step)
                scope = scope.group(1) if scope else path.name
                with self.subTest(workflow=path.name, scope=scope):
                    if "save-if: false" in step:
                        consumers.setdefault(scope, path.name)
                        continue
                    self.assertFalse(
                        ref_scoped,
                        f"{path.name} saves a cache from a ref only its own "
                        "re-runs can restore; add `save-if: false`",
                    )
                    producers.add(scope)

        # A restore-only scope with no producer is the same waste read from the
        # other end: a step that can only ever miss.
        for scope, workflow in consumers.items():
            with self.subTest(workflow=workflow, scope=scope):
                self.assertIn(scope, producers)

    def test_landing_reuses_the_exact_trusted_content_checkout(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")
        content_producer = main[
            main.index("  content-landing-cache:") : main.index(
                "  linux-landing-cache:"
            )
        ]
        linux_producer = main[
            main.index("  linux-landing-cache:") : main.index(
                "  exact-sha-qualification:"
            )
        ]
        quality = landing[
            landing.index("  pull-request-quality:") : landing.index(
                "  release-context:"
            )
        ]
        linux = landing[
            landing.index("  linux:") : landing.index("  windows-smoke:")
        ]

        for consumer in (quality, linux, linux_producer):
            self.assertNotIn("submodules: recursive", consumer)
            self.assertIn("actions/cache/restore@", consumer)
            self.assertIn(".git/modules/content", consumer)
            self.assertIn(
                "key: clonk-content-git-v1-${{ runner.os }}-"
                "${{ hashFiles('.gitmodules') }}-"
                "${{ steps.content.outputs.revision }}",
                consumer,
            )
            self.assertIn(
                "git submodule update --init --force --depth=1 "
                "--filter=blob:none content",
                consumer,
            )
            self.assertIn('[[ "$actual" == "$CONTENT_REVISION" ]]', consumer)
            self.assertNotIn("actions/cache/save@", consumer)
            self.assertNotIn("restore-keys:", consumer)

        self.assertIn("needs: content-landing-cache", linux_producer)
        self.assertNotIn("submodules: recursive", content_producer)
        self.assertIn("actions/cache/restore@", content_producer)
        self.assertIn("actions/cache/save@", content_producer)
        self.assertIn(".git/modules/content", content_producer)
        self.assertIn("lookup-only: true", content_producer)
        self.assertIn(
            "key: clonk-content-git-v1-${{ runner.os }}-"
            "${{ hashFiles('.gitmodules') }}-"
            "${{ steps.content.outputs.revision }}",
            content_producer,
        )
        self.assertIn(
            "git submodule update --init --force --depth=1 "
            "--filter=blob:none content",
            content_producer,
        )
        self.assertIn('[[ "$actual" == "$CONTENT_REVISION" ]]', content_producer)
        self.assertEqual(
            content_producer.count(
                "if: steps.content-cache.outputs.cache-hit != 'true'"
            ),
            4,
        )
        self.assertIn("fail-on-cache-miss: true", content_producer)
        self.assertNotIn("restore-keys:", content_producer)
        self.assertIn("github.event_name == 'workflow_dispatch'", content_producer)
        self.assertIn("github.sha || 'rolling'", content_producer)
        self.assertIn("cancel-in-progress: false", content_producer)

    def test_cache_producers_finish_while_obsolete_diagnostics_cancel(self):
        main = MAIN.read_text(encoding="utf-8")
        qualification = QUALIFICATION.read_text(encoding="utf-8")

        self.assertNotRegex(main, r"(?m)^concurrency:\s*$")
        linux_producer = main[
            main.index("  linux-landing-cache:") : main.index(
                "  exact-sha-qualification:"
            )
        ]
        caller = main[
            main.index("  exact-sha-qualification:") : main.index(
                "  windows-release-tools:"
            )
        ]
        coverage_fragments = qualification[
            qualification.index("  coverage-fragments:") : qualification.index(
                "  coverage:"
            )
        ]
        coverage_report = qualification[
            qualification.index("  coverage:") : qualification.index(
                "  coverage-html:"
            )
        ]
        coverage_html = qualification[
            qualification.index("  coverage-html:") : qualification.index(
                "  developer-feedback:"
            )
        ]
        developer_feedback = qualification[
            qualification.index("  developer-feedback:") : qualification.index(
                "  recording-host-oracles:"
            )
        ]
        recording_host = qualification[
            qualification.index("  recording-host-oracles:") :
        ]
        content_producer = main[
            main.index("  content-landing-cache:") : main.index(
                "  linux-landing-cache:"
            )
        ]
        windows_producer = main[
            main.index("  windows-landing-cache:") : main.index(
                "  windows-release-tools:"
            )
        ]
        release_tools = main[main.index("  windows-release-tools:") :]

        for producer in (
            content_producer,
            linux_producer,
            windows_producer,
            release_tools,
        ):
            self.assertIn("cancel-in-progress: false", producer)
        for diagnostic in (
            coverage_fragments,
            coverage_report,
            coverage_html,
            developer_feedback,
            recording_host,
        ):
            self.assertIn("cancel-in-progress: true", diagnostic)
        self.assertIn(
            'group: "main-coverage-report-${{ inputs.concurrency-suffix }}"',
            coverage_report,
        )
        self.assertIn("needs: linux-landing-cache", caller)
        self.assertIn("save-if: false", developer_feedback)
        self.assertIn("publish-recording-host-cache: true", caller)

    def test_cache_only_dispatch_finishes_after_trusted_producers(self):
        main = MAIN.read_text(encoding="utf-8")
        trigger = main[main.index("on:\n") : main.index("permissions:\n")]
        verifier = main[
            main.index("  verify-landing-cache-bootstrap:") : main.index(
                "  content-landing-cache:"
            )
        ]
        qualification = main[
            main.index("  exact-sha-qualification:") : main.index(
                "  windows-landing-cache:"
            )
        ]
        release_tools = main[main.index("  windows-release-tools:") :]

        self.assertIn("cache_only:", trigger)
        self.assertIn("type: boolean", trigger)
        self.assertIn("default: false", trigger)
        skip_guard = (
            "github.event_name != 'workflow_dispatch' || !inputs.cache_only"
        )
        self.assertIn(skip_guard, qualification)
        self.assertIn(skip_guard, release_tools)
        self.assertIn("github.event_name == 'workflow_dispatch'", verifier)
        self.assertIn("inputs.cache_only", verifier)
        self.assertIn("needs:\n      - linux-landing-cache", verifier)
        self.assertIn("- windows-landing-cache", verifier)
        self.assertIn("shared-key: full-parity", verifier)
        self.assertIn("shared-key: windows-runtime-msvc-v2", verifier)
        self.assertIn("lookup-only: true", verifier)
        self.assertIn("save-if: false", verifier)
        self.assertIn('[[ "$CACHE_HIT" == "true" ]]', verifier)

    def test_post_merge_work_leaves_the_next_landing_runner_budget(self):
        main = MAIN.read_text(encoding="utf-8")
        dependency_guard = DEPENDENCY_GUARD.read_text(encoding="utf-8")

        qualification = main[
            main.index("  exact-sha-qualification:") : main.index(
                "  windows-release-tools:"
            )
        ]
        self.assertIn("needs: linux-landing-cache", qualification)

        triggers = dependency_guard[
            dependency_guard.index("on:\n") : dependency_guard.index("permissions:\n")
        ]
        self.assertNotIn("\n  push:\n", triggers)

    def test_preinstalled_rust_probe_preserves_cache_toolchain_inventory(self):
        landing = LANDING.read_text(encoding="utf-8")
        probe = landing[
            landing.index("      - name: Reuse exact preinstalled Rust") : landing.index(
                "      - name: Install exact Rust toolchain"
            )
        ]

        self.assertNotIn("RUSTUP_AUTO_INSTALL", probe)

    def test_merge_group_rows_share_only_non_cancelling_cache_lanes(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")

        shared_lanes = {
            "linux-landing-cache-rolling": "app 2+7/12",
            "windows-landing-cache-rolling": "runtime tests",
        }
        self.assertEqual(
            set(re.findall(r"\b[a-z0-9-]+-rolling\b", landing)),
            set(shared_lanes),
        )
        for group, claimant in shared_lanes.items():
            with self.subTest(group=group):
                self.assertEqual(landing.count(group), 1)
                self.assertIn(claimant, landing)
                producer_group = (
                    f'group: "{group.removesuffix("rolling")}'
                    "${{ (github.event_name == 'workflow_dispatch' || "
                    "startsWith(github.event.head_commit.message, "
                    "'chore: release ')) && github.sha || 'rolling' }}\""
                )
                self.assertIn(producer_group, main)

        linux_job = landing[
            landing.index("  linux:") : landing.index("  windows-smoke:")
        ]
        linux_rows = set(re.findall(r"(?m)^          - name: (.+)$", linux_job))
        self.assertIn("app 2+7/12", linux_rows)
        self.assertIn(
            "matrix.name == 'app 2+7/12' && 'linux-landing-cache-rolling'",
            landing,
        )
        self.assertIn(
            "matrix.name == 'runtime tests' && "
            "'windows-landing-cache-rolling'",
            landing,
        )

        self.assertIn(
            "format('landing-linux-{0}-{1}', github.run_id, matrix.name)",
            landing,
        )
        self.assertIn(
            "format('landing-windows-{0}-{1}', github.run_id, matrix.name)",
            landing,
        )
        self.assertEqual(
            landing.count(
                "cancel-in-progress: ${{ github.event_name == 'merge_group' }}"
            ),
            2,
        )

    def test_normal_workspace_is_an_exhaustive_compile_time_partition(self):
        workflow = LANDING.read_text(encoding="utf-8")

        app_commands = re.findall(
            r"cargo nextest run -p clonk-app --features ([a-z0-9,-]+)"
            r" --no-fail-fast --locked",
            workflow,
        )
        self.assertEqual(
            [tuple(command.split(",")) for command in app_commands],
            [
                ("app-test-shard-1", "app-test-shard-12"),
                ("app-test-shard-3", "app-test-shard-10"),
                ("app-test-shard-2", "app-test-shard-7"),
                ("app-test-shard-4", "app-test-shard-9"),
                ("app-test-shard-5",),
                ("app-test-shard-11",),
                ("app-test-shard-6", "app-test-shard-8"),
            ],
        )
        app_manifest = tomllib.loads(
            (REPOSITORY / "crates" / "clonk-app" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        selectors = {
            feature
            for feature in app_manifest["features"]
            if re.fullmatch(r"app-test-shard-[1-9][0-9]*", feature)
        }
        self.assertEqual(
            Counter(
                feature
                for command in app_commands
                for feature in command.split(",")
            ),
            Counter(selectors),
        )

        engine_commands = re.findall(
            r"--features (?:clonk-engine-integration-tests/)?(engine-it-shard-[123]) "
            r"--no-fail-fast --locked",
            workflow,
        )
        self.assertEqual(
            engine_commands,
            ["engine-it-shard-1", "engine-it-shard-2", "engine-it-shard-3"],
        )
        for shard in (1, 2, 3):
            entry = matrix_entry(workflow, f"engine integration {shard}/3")
            self.assertIn(
                "cargo nextest run -p clonk-engine-integration-tests "
                f"--test engine_it --features engine-it-shard-{shard} "
                "--no-fail-fast --locked",
                entry,
            )
        engine_unit_and_parity = matrix_entry(workflow, "engine unit and parity")
        self.assertIn(
            "cargo nextest run -p clonk-engine-unit-tests "
            "--no-fail-fast --locked",
            engine_unit_and_parity,
        )
        self.assertIn("cargo xtask parity verify", engine_unit_and_parity)
        frontend_unit = matrix_entry(workflow, "frontend unit")
        self.assertIn(
            "cargo nextest run -p clonk-frontend-unit-tests "
            "--no-fail-fast --locked",
            frontend_unit,
        )
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
        self.assertNotIn("-p clonk-app-render", remaining_shards[0][2])
        self.assertIn("-p clonk-app-render", remaining_shards[1][2])
        self.assertIn("-p clonk-network", remaining_shards[0][2])
        self.assertNotIn("-p clonk-network", remaining_shards[1][2])
        self.assertNotIn("-p clonk-app-menus", remaining_shards[0][2])
        self.assertIn("-p clonk-app-menus", remaining_shards[1][2])
        self.assertNotIn(
            "cargo nextest run --workspace --no-fail-fast --locked",
            workflow,
        )

    def test_overlapping_linux_checks_share_setup_without_failing_open(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]
        unit_and_parity = matrix_entry(workflow, "engine unit and parity")
        quality = matrix_entry(workflow, "workspace quality")

        for entry in (unit_and_parity, quality):
            self.assertIn("failed=0", entry)
            self.assertIn('exit "$failed"', entry)
        self.assertIn("cargo xtask parity verify || failed=1", unit_and_parity)
        self.assertIn("cargo clippy --version || failed=1", quality)
        self.assertIn("rustfmt --version || failed=1", quality)
        for command in (
            "cargo fmt --all -- --check || failed=1",
            "python3 -m unittest discover --buffer -s scripts/tests -p 'test_*.py' || failed=1",
            "cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings || failed=1",
        ):
            self.assertIn(command, quality)
        for old_name in (
            "engine unit",
            "workspace unit and parity",
            "workspace lints",
            "C++ parity",
            "repository hygiene",
        ):
            self.assertNotIn(f"          - name: {old_name}\n", workflow)
        self.assertNotIn("components: clippy, rustfmt", linux)

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
        app_rows = [
            "app 1+12/12",
            "app 3+10/12",
            "app 2+7/12",
            "app 4+9/12",
            "app 5/12",
            "app 11/12",
            "app 6+8/12",
        ]
        for name in app_rows:
            self.assertIn(
                "apt: libasound2-dev libudev-dev",
                matrix_entry(workflow, name),
            )
        expected_apt = {
            "remaining workspace 2/2": "libasound2-dev libxmp4 mesa-vulkan-drivers",
            "workspace quality": "libasound2-dev libudev-dev python3-pil",
        }
        for name, packages in expected_apt.items():
            self.assertIn(f"apt: {packages}", matrix_entry(workflow, name))
        self.assertNotIn(
            "\n            apt:",
            matrix_entry(workflow, "remaining workspace 1/2"),
        )
        for name in (
            "engine integration 1/3",
            "engine integration 2/3",
            "engine integration 3/3",
            "engine unit and parity",
            "frontend unit",
            "engine contracts",
        ):
            self.assertNotIn("\n            apt:", matrix_entry(workflow, name))

        self.assertEqual(
            len(re.findall(r"(?m)^          - name: ", linux)),
            16,
            "16 Linux rows plus three Windows lanes and release context fit the runner pool",
        )

        self.assertIn("if: matrix.apt != ''", linux)
        self.assertIn("scripts/install-apt-packages.sh", linux)
        self.assertIn("timeout-minutes: 10", linux)
        self.assertIn("rustc 1.97.1", linux)
        self.assertIn("id: preinstalled-rust", linux)
        self.assertIn("if: steps.preinstalled-rust.outputs.exact != 'true'", linux)

    def test_hosted_toolchains_and_cached_registry_are_reused_safely(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]
        windows_smoke = workflow[
            workflow.index("  windows-smoke:") : workflow.index("  landing-gate:")
        ]

        for job in (linux,):
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

        pinned_toolchain = (
            "uses: dtolnay/rust-toolchain@"
            "46511b1c83438f0dd37c02d843619ece5a4abb5b"
        )
        for job in (windows_smoke,):
            self.assertIn(pinned_toolchain, job)
            self.assertNotIn("id: preinstalled-rust", job)
            self.assertNotIn("CARGO_HOME=", job)

        self.assertNotIn("cargo build --release -p clonk-app", workflow)
        self.assertNotIn("scripts/configure-msvc-runtime.sh", workflow)

    def test_windows_smoke_long_poles_run_on_three_parallel_rows(self):
        workflow = LANDING.read_text(encoding="utf-8")
        windows = workflow[
            workflow.index("  windows-smoke:") : workflow.index("  landing-gate:")
        ]
        rows = set(re.findall(r"(?m)^          - name: (.+)$", windows))

        self.assertEqual(rows, {"runtime tests", "network tests", "quality"})
        self.assertIn("name: Windows / ${{ matrix.name }}", windows)
        self.assertIn("if: matrix.nextest", windows)
        self.assertEqual(windows.count("if: matrix.installer"), 2)

        runtime = matrix_entry(workflow, "runtime tests")
        self.assertIn("-p clonk-game -p clonk-c4group", runtime)
        self.assertIn("-p clonk-logging --test crash_log_descriptor", runtime)
        self.assertNotIn("-p clonk-network", runtime)

        network = matrix_entry(workflow, "network tests")
        self.assertIn("-p clonk-network --lib", network)
        self.assertNotIn("cargo clippy", network)

        quality = matrix_entry(workflow, "quality")
        self.assertIn("nextest: false", quality)
        self.assertIn("installer: true", quality)
        self.assertIn("cargo clippy --profile test --no-deps", quality)

    def test_literal_required_commands_remain_on_the_landing_tree(self):
        workflow = LANDING.read_text(encoding="utf-8")
        required = (
            "cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings",
            "cargo xtask parity verify",
            "cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked",
            "cargo xtask engine-snapshots verify",
            "cargo fmt --all -- --check",
            "python3 -m unittest discover --buffer -s scripts/tests -p 'test_*.py'",
        )
        for command in required:
            with self.subTest(command=command):
                self.assertIn(command, workflow)

        self.assertIn("fetch-depth: 1", workflow)
        self.assertNotIn("fetch-depth: 0", workflow)
        self.assertIn("python3-pil", workflow)

    def test_slow_diagnostics_yield_to_landing_but_release_qualification_does_not(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")
        qualification = QUALIFICATION.read_text(encoding="utf-8")
        main_caller = main[
            main.index("  exact-sha-qualification:") : main.index(
                "  windows-release-tools:"
            )
        ]
        release_caller = landing[
            landing.index("  release-qualification:") : landing.index("  linux:")
        ]

        self.assertNotIn("cargo llvm-cov", landing)
        self.assertNotIn("runs-on: macos-latest", landing)
        self.assertIn(
            "uses: ./.github/workflows/exact-sha-qualification.yml", main
        )
        self.assertIn("cargo llvm-cov", qualification)
        self.assertIn("runs-on: macos-latest", qualification)
        self.assertIn("concurrency-suffix: rolling", main_caller)
        self.assertIn("concurrency-suffix: ${{ github.sha }}", release_caller)
        self.assertIn("cancel-in-progress: true", qualification)

    def test_post_merge_render_probe_consumes_a_fresh_deterministic_replay(self):
        workflow = QUALIFICATION.read_text(encoding="utf-8")
        developer = workflow[
            workflow.index("  developer-feedback:") : workflow.index(
                "  recording-host-oracles:"
            )
        ]
        replay = developer.index("- name: Generate deterministic replay evidence")
        render = developer.index("- name: Render the replay snapshot")
        upload = developer.index("- name: Upload developer-feedback artifacts")

        self.assertLess(replay, render)
        self.assertLess(render, upload)
        self.assertIn("if: inputs.upload-diagnostics", developer)
        self.assertNotIn("cargo llvm-cov", developer)
        self.assertIn(
            "dev_feedback_replay::real_scenario_replays_repeat_with_native_group_order",
            developer[replay:render],
        )
        self.assertIn("dev_feedback_render --ignored --exact", developer[render:upload])

    def test_post_merge_replay_writes_to_the_repository_artifact_root(self):
        workflow = QUALIFICATION.read_text(encoding="utf-8")
        replay = workflow.index("- name: Generate deterministic replay evidence")
        render = workflow.index("- name: Render the replay snapshot")

        self.assertIn(
            "LC_TEST_ARTIFACT_DIR: ${{ github.workspace }}/target/dev-check/replay",
            workflow[replay:render],
        )

    def test_post_merge_render_uses_repository_artifact_paths(self):
        workflow = QUALIFICATION.read_text(encoding="utf-8")
        render = workflow.index("- name: Render the replay snapshot")
        upload = workflow.index("- name: Upload developer-feedback artifacts")
        render_step = workflow[render:upload]

        for path in (
            "LC_DEV_CHECK_SNAPSHOT: ${{ github.workspace }}/target/dev-check/snapshot-final.json",
            "LC_DEV_CHECK_FRAME_PNG: ${{ github.workspace }}/target/dev-check/frame-final.png",
            "LC_DEV_CHECK_RENDER_METRICS: ${{ github.workspace }}/target/dev-check/render-metrics.json",
        ):
            with self.subTest(path=path):
                self.assertIn(path, render_step)

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

    def test_dependency_guard_cross_checks_the_windows_renderer_graph(self):
        workflow = DEPENDENCY_GUARD.read_text(encoding="utf-8")
        self.assertIn("targets: x86_64-pc-windows-msvc", workflow)
        self.assertIn(
            "cargo check --locked --target x86_64-pc-windows-msvc "
            "-p clonk-surface",
            workflow,
        )
        self.assertIn(
            "cargo check --locked --target x86_64-pc-windows-msvc "
            "-p clonk-platform",
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
