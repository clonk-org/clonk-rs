"""Restored workspace artifacts need a byte-verified ledger before Cargo runs."""

import re
import unittest

from _repo import REPOSITORY


WORKFLOWS = REPOSITORY / ".github/workflows"


def workflow_job(source, name):
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [a-z][a-z0-9_-]*:\n|\Z)", source)
    if match is None:
        raise AssertionError(f"workflow lacks {name}")
    return match[0]


def workspace_cache_steps(source, operation):
    return [
        step for step in re.findall(r"(?ms)^      - .*?(?=^      - |\Z)", source)
        if "uses: ./.github/actions/workspace-cache" in step
        and f"operation: {operation}\n" in step
    ]


def named_step(source, name):
    match = re.search(rf"(?ms)^      - name: {re.escape(name)}\n.*?(?=^      - |\Z)", source)
    if match is None:
        raise AssertionError(f"job lacks step {name}")
    return "\n".join(line for line in match[0].splitlines() if not line.startswith("      #")).rstrip()


def warm_job_is_admitted(source, name, *, cancelled=False, **changes):
    expression = re.search(r"(?m)^    if: \$\{\{ (.*?) \}\}$", workflow_job(source, name))[1]
    values = {
        "needs.qualification-context.result": "success",
        "needs.qualification-context.outputs.run-id": "71",
        "inputs.upload-diagnostics": True, "inputs.publish-recording-host-cache": True,
        "inputs.source-sha": "a" * 40, "github.sha": "a" * 40,
        "github.ref": "refs/heads/main", "github.event.repository.fork": False,
        "github.event_name": "push",
    }
    values.update(changes)
    for variable in sorted(values, key=len, reverse=True):
        expression = expression.replace(variable, repr(values[variable]))
    expression = expression.replace("!cancelled()", repr(not cancelled))
    expression = re.sub(r"!(?!=)", "not ", expression).replace("&&", "and").replace("||", "or")
    return eval(expression, {"__builtins__": {}}, {})


class CiBuildCacheWorkflowTests(unittest.TestCase):
    def test_cache_temperature_comes_from_successful_source_verification(self):
        roles = {
            "landing.yml": ("pull-request-quality", "linux"),
            "rust.yml": ("linux-landing-cache",),
            "release-prebuild.yml": ("tool", "runtime"),
            "exact-sha-qualification.yml": (
                "coverage-fragments", "coverage-cache-warm", "developer-feedback",
            ),
        }
        for filename, names in roles.items():
            source = (WORKFLOWS / filename).read_text()
            for name in names:
                with self.subTest(workflow=filename, job=name):
                    job = workflow_job(source, name)
                    steps = re.findall(r"(?ms)^      - .*?(?=^      - |\Z)", job)
                    prepare = next(step for step in steps if "ci-workspace-cache.py prepare " in step)
                    self.assertIn("id: workspace-inputs\n", prepare)
                    self.assertIn('--github-output "$GITHUB_OUTPUT"', prepare)
                    self.assertNotIn("continue-on-error", prepare)
                    receipt = next(step for step in steps if "CI cache receipt:" in step)
                    self.assertIn("steps.workspace-inputs.outputs.cache-state", receipt)
                    self.assertLess(steps.index(prepare), steps.index(receipt))
                    self.assertNotIn("cache-hit", receipt)
                    if filename == "release-prebuild.yml":
                        self.assertIn("if: matrix.name == 'linux'", prepare)
                        self.assertIn("if: matrix.name == 'linux'", receipt)
                    else:
                        self.assertNotIn("if:", receipt)

    def test_landing_checks_workspace_bytes_before_running_cargo(self):
        source = (WORKFLOWS / "landing.yml").read_text()
        linux = workflow_job(source, "linux")
        self.assertIn("cache-targets: false", linux)
        self.assertEqual(len(workspace_cache_steps(linux, "restore")), 1)
        self.assertEqual(workspace_cache_steps(linux, "save"), [])
        restored = workspace_cache_steps(linux, "restore")[0]
        for setting in ("lane: landing-linux", "target: target", "ledger: .ci-cache-ledgers/landing.json", "recipe: landing-v1"):
            self.assertIn(setting, restored)
        self.assertIn("ci-workspace-cache.py prepare --target target --recipe landing-v1", linux)
        self.assertLess(linux.index("uses: ./.github/actions/workspace-cache"), linux.index("ci-workspace-cache.py prepare"))
        self.assertLess(linux.index("ci-workspace-cache.py prepare"), linux.index("      - name: Run ${{ matrix.name }}"))
        self.assertIn("save-if: false", linux)
        self.assertNotIn("if: steps.workspace-cache.outputs.cache-hit", linux)

    def test_trusted_linux_producer_verifies_and_records_both_build_profiles(self):
        source = (WORKFLOWS / "rust.yml").read_text()
        producer = workflow_job(source, "linux-landing-cache")
        self.assertIn("cache-targets: false", producer)
        for operation in ("restore", "save"):
            steps = workspace_cache_steps(producer, operation)
            self.assertEqual(len(steps), 1)
            for setting in ("lane: landing-linux", "target: target", "ledger: .ci-cache-ledgers/landing.json", "recipe: landing-v1"):
                self.assertIn(setting, steps[0])
        self.assertIn("ci-workspace-cache.py prepare --target target --recipe landing-v1", producer)
        self.assertIn("ci-workspace-cache.py record --target target --recipe landing-v1", producer)
        self.assertLess(producer.index("ci-workspace-cache.py prepare"), producer.index("cargo nextest"))
        self.assertGreater(producer.index("ci-workspace-cache.py record"), producer.index("cargo build --locked --release -p clonk-app --features presentation-capture"))
        self.assertLess(producer.index("ci-workspace-cache.py record"), producer.index("      - name: Publish verified workspace build inputs"))
        self.assertNotIn("if: steps.linux-cache.outputs.cache-hit != 'true'", producer)

    def test_instrumented_artifacts_use_exact_flags_and_fresh_results(self):
        source = (WORKFLOWS / "exact-sha-qualification.yml").read_text()
        collector = workflow_job(source, "coverage-fragments")
        self.assertIn("cargo llvm-cov show-env --sh", collector)
        self.assertIn("cache-targets: false", collector)
        self.assertIn("shared-key: coverage-registry", collector)
        for operation in ("restore", "save"):
            steps = workspace_cache_steps(collector, operation)
            self.assertEqual(len(steps), 1)
            for setting in ("lane: coverage-linux", "target: target/coverage-build", "ledger: .ci-cache-ledgers/coverage.json", "recipe: coverage-v1"):
                self.assertIn(setting, steps[0])
        self.assertIn("ci-workspace-cache.py prepare --target target/coverage-build --recipe coverage-v1", collector)
        self.assertIn("ci-workspace-cache.py record --target target/coverage-build --recipe coverage-v1", collector)
        self.assertIn("cargo llvm-cov clean --profraw-only", collector)
        self.assertNotIn("cargo llvm-cov clean --workspace", collector)
        self.assertIn("inputs.upload-diagnostics && matrix.artifact == 'app-1-10'", collector)
        self.assertLess(collector.index("cargo llvm-cov show-env"), collector.index("Swatinem/rust-cache@"))
        self.assertLess(collector.index("ci-workspace-cache.py prepare"), collector.index("      - name: Collect instrumented"))
        self.assertGreater(collector.index("ci-workspace-cache.py record"), collector.index("      - name: Collect instrumented"))
        self.assertLess(collector.index("ci-workspace-cache.py record"), collector.index("      - name: Publish verified instrumented workspace inputs"))
        self.assertNotIn("outputs.cache-hit", collector)

    def test_verified_receipt_reuse_warms_instrumented_inputs_without_running_tests(self):
        source = (WORKFLOWS / "exact-sha-qualification.yml").read_text()
        collector = workflow_job(source, "coverage-fragments")
        warmer = workflow_job(source, "coverage-cache-warm")
        for guard in (
            "needs: qualification-context", "!cancelled()",
            "needs.qualification-context.result == 'success'",
            "needs.qualification-context.outputs.run-id != ''", "inputs.upload-diagnostics",
            "github.ref == 'refs/heads/main'",
        ):
            self.assertIn(guard, warmer)
        self.assertNotIn("matrix:", warmer)
        self.assertNotIn("continue-on-error", warmer)
        self.assertIn("runs-on: ubuntu-24.04", warmer)
        self.assertIn("CARGO_TARGET_DIR: target/coverage-build", warmer)
        self.assertIn("LLVM_PROFILE_FILE_NAME: 'clonk-%8m.profraw'", warmer)
        self.assertIn("cache-targets: false", warmer)
        for name in (
            "Install Rust toolchain", "Install cargo-nextest", "Install cargo-llvm-cov",
            "Resolve exact coverage build environment",
        ):
            self.assertEqual(named_step(collector, name), named_step(warmer, name))
        command = "cargo nextest run --no-run -p clonk-app --features app-test-shard-1,app-test-shard-10 --locked"
        self.assertIn(command, warmer)
        self.assertEqual(warmer.count("cargo nextest run --no-run"), 1)
        self.assertIn("cargo llvm-cov clean --profraw-only", warmer)
        self.assertNotIn("cargo llvm-cov clean --workspace", warmer)
        self.assertNotIn("--lcov", warmer)
        self.assertNotIn("actions/upload-artifact", warmer)
        for operation in ("restore", "save"):
            steps = workspace_cache_steps(warmer, operation)
            self.assertEqual(len(steps), 1)
            for setting in ("lane: coverage-linux", "target: target/coverage-build", "ledger: .ci-cache-ledgers/coverage.json", "recipe: coverage-v1"):
                self.assertIn(setting, steps[0])
        restore = warmer.index("      - name: Restore verified instrumented workspace inputs")
        prepare = warmer.index("ci-workspace-cache.py prepare --target target/coverage-build --recipe coverage-v1 --ledger .ci-cache-ledgers/coverage.json")
        compile_inputs = warmer.index(command)
        record = warmer.index("ci-workspace-cache.py record --target target/coverage-build --recipe coverage-v1 --ledger .ci-cache-ledgers/coverage.json")
        save = warmer.index("      - name: Publish verified instrumented workspace inputs")
        self.assertLess(restore, prepare)
        self.assertLess(prepare, compile_inputs)
        self.assertLess(compile_inputs, record)
        self.assertLess(record, save)
        self.assertNotIn("outputs.cache-hit", warmer)

    def test_verified_receipt_reuse_warms_only_the_existing_native_oracle_inputs(self):
        source = (WORKFLOWS / "exact-sha-qualification.yml").read_text()
        oracles = workflow_job(source, "recording-host-oracles")
        warmer = workflow_job(source, "recording-host-cache-warm")
        for guard in (
            "needs: qualification-context", "!cancelled()",
            "needs.qualification-context.result == 'success'",
            "needs.qualification-context.outputs.run-id != ''", "inputs.publish-recording-host-cache",
            "github.ref == 'refs/heads/main'",
        ):
            self.assertIn(guard, warmer)
        self.assertIn("runs-on: macos-latest", warmer)
        self.assertNotIn("continue-on-error", warmer)
        for name in ("Install Rust toolchain", "Install cargo-nextest", "Restore Rust build cache"):
            self.assertEqual(named_step(oracles, name), named_step(warmer, name))
        self.assertIn("shared-key: recording-host-oracles", warmer)
        self.assertIn("save-if: ${{ inputs.publish-recording-host-cache }}", warmer)
        self.assertEqual(warmer.count("cargo nextest run"), 2)
        self.assertEqual(warmer.count("cargo nextest run --no-run"), 2)
        for original_name, compile_name in (
            ("Run native microphone permission callback tests", "Compile native microphone permission callback tests"),
            ("Run recording-host material-order oracles", "Compile recording-host material-order oracles"),
        ):
            original = named_step(oracles, original_name).split("        run:", 1)[1]
            compiled = named_step(warmer, compile_name).split("        run:", 1)[1]
            self.assertEqual(compiled, original.replace("cargo nextest run", "cargo nextest run --no-run", 1))
        self.assertNotIn("actions/upload-artifact", warmer)

    def test_cache_warmers_reject_unverified_failed_cancelled_and_untrusted_contexts(self):
        source = (WORKFLOWS / "exact-sha-qualification.yml").read_text()
        for name, permission in (
            ("coverage-cache-warm", "inputs.upload-diagnostics"),
            ("recording-host-cache-warm", "inputs.publish-recording-host-cache"),
        ):
            with self.subTest(job=name):
                self.assertTrue(warm_job_is_admitted(source, name))
                self.assertFalse(warm_job_is_admitted(source, name, cancelled=True))
                for changes in (
                    {"needs.qualification-context.result": "failure"},
                    {"needs.qualification-context.result": "cancelled"},
                    {"needs.qualification-context.result": "skipped"},
                    {"needs.qualification-context.outputs.run-id": ""},
                    {permission: False},
                    {"github.event_name": "pull_request"},
                    {"github.event_name": "merge_group"},
                    {"github.event.repository.fork": True},
                    {"github.ref": "refs/heads/feature"},
                    {"inputs.source-sha": "b" * 40},
                ):
                    with self.subTest(changes=changes):
                        self.assertFalse(warm_job_is_admitted(source, name, **changes))

    def test_receipt_misses_keep_all_twelve_collectors_and_the_coverage_floor(self):
        source = (WORKFLOWS / "exact-sha-qualification.yml").read_text()
        collector = workflow_job(source, "coverage-fragments")
        coverage = workflow_job(source, "coverage")
        expected = {
            "app-1-10", "app-2-7", "app-3", "app-4-9", "app-5", "app-11-12", "app-6-8",
            "engine-1", "engine-2-3", "engine-and-frontend-units", "remaining-1", "remaining-2",
        }
        actual = re.findall(r"(?m)^            artifact: (\S+)\s*$", collector)
        self.assertEqual(len(actual), 12)
        self.assertEqual(set(actual), expected)
        commands = re.findall(r"(?m)^            command: (.+)$", collector)
        self.assertEqual(len(commands), 12)
        self.assertTrue(all("cargo nextest run" in command for command in commands))
        self.assertNotIn("cargo llvm-cov --no-report nextest", collector)
        self.assertTrue(all("--no-fail-fast --locked" in command for command in commands))
        self.assertTrue(all("--no-run" not in command for command in commands))
        self.assertIn("COVERAGE_MIN_LINE_PERCENT: '79.45'", coverage)
        self.assertIn("EXPECTED_FRAGMENT_COUNT: '12'", coverage)
        self.assertIn('--fail-under-lines "$COVERAGE_MIN_LINE_PERCENT"', coverage)
        for name in ("coverage-fragments", "recording-host-oracles"):
            self.assertIn("needs.qualification-context.outputs.run-id == ''", workflow_job(source, name))
        oracles = workflow_job(source, "recording-host-oracles")
        self.assertEqual(oracles.count("cargo nextest run"), 2)
        self.assertNotIn("--no-run", oracles)
        self.assertIn("shared-key: recording-host-oracles", oracles)

    def test_ci_scheduling_profile_matches_report_paths_and_metadata(self):
        for name in ("landing.yml", "rust.yml", "exact-sha-qualification.yml"):
            source = (WORKFLOWS / name).read_text()
            with self.subTest(workflow=name):
                self.assertIn("NEXTEST_PROFILE: ci", source)
                self.assertNotIn("nextest/default/junit.xml", source)
                self.assertNotIn('"nextest_profile": "default"', source)
        landing = (WORKFLOWS / "landing.yml").read_text()
        coverage = (WORKFLOWS / "exact-sha-qualification.yml").read_text()
        self.assertIn("JUNIT_SOURCE: target/nextest/ci/junit.xml", landing)
        self.assertIn("JUNIT_SOURCE: target/nextest/ci/junit.xml", coverage)
        self.assertIn('"nextest_profile": "ci"', landing)
        self.assertIn('"nextest_profile": "ci"', coverage)


if __name__ == "__main__":
    unittest.main()
