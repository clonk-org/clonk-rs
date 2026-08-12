"""Main validation keeps post-merge coverage visible and reproducible."""

import re
import tomllib
import unittest
from collections import Counter

from _repo import REPOSITORY

WORKFLOW = REPOSITORY / ".github" / "workflows" / "exact-sha-qualification.yml"


def job_block(name):
    """Return one top-level job without requiring a YAML dependency."""
    workflow = WORKFLOW.read_text(encoding="utf-8")
    marker = f"  {name}:"
    try:
        start = workflow.index(marker)
    except ValueError:
        raise AssertionError(f"{WORKFLOW.name} has no job named {name!r}") from None

    next_job = re.search(r"(?m)^  [A-Za-z0-9_-]+:$", workflow[start + 1 :])
    end = start + 1 + next_job.start() if next_job else None
    return workflow[start:end]


class RustCoverageGateTests(unittest.TestCase):
    def test_coverage_collectors_partition_every_workspace_test_once(self):
        collectors = job_block("coverage-fragments")
        commands = "\n".join(
            re.findall(r"(?m)^            command: (.+)$", collectors)
        )

        self.assertIn("name: Rust coverage / ${{ matrix.name }}", collectors)
        self.assertIn("timeout-minutes: 10", collectors)
        self.assertIn("cargo llvm-cov clean --workspace", collectors)
        self.assertIn("cargo llvm-cov --no-report nextest", collectors)
        self.assertIn("--no-fail-fast", collectors)
        self.assertIn("--locked", collectors)

        app_features = re.findall(r"app-test-shard-[1-9][0-9]*", commands)
        app_manifest = tomllib.loads(
            (REPOSITORY / "crates" / "clonk-app" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        expected_app_features = {
            feature
            for feature in app_manifest["features"]
            if re.fullmatch(r"app-test-shard-[1-9][0-9]*", feature)
        }
        self.assertEqual(Counter(app_features), Counter(expected_app_features))
        app_groups = {
            frozenset(features.split(","))
            for features in re.findall(
                r"-p clonk-app --features ([a-z0-9,-]+)", commands
            )
        }
        self.assertEqual(
            app_groups,
            {
                frozenset(("app-test-shard-1", "app-test-shard-10")),
                frozenset(("app-test-shard-2", "app-test-shard-7")),
                frozenset(("app-test-shard-3", "app-test-shard-12")),
                frozenset(("app-test-shard-4", "app-test-shard-9")),
                frozenset(("app-test-shard-5", "app-test-shard-11")),
                frozenset(("app-test-shard-6", "app-test-shard-8")),
            },
        )

        engine_features = re.findall(r"engine-it-shard-[1-9][0-9]*", commands)
        self.assertEqual(
            Counter(engine_features),
            Counter({"engine-it-shard-1", "engine-it-shard-2"}),
        )

        selected_packages = re.findall(r"(?:^|\s)-p\s+([a-z0-9-]+)", commands)
        workspace = tomllib.loads(
            (REPOSITORY / "Cargo.toml").read_text(encoding="utf-8")
        )["workspace"]
        expected_packages = {
            tomllib.loads(
                (REPOSITORY / member / "Cargo.toml").read_text(encoding="utf-8")
            )["package"]["name"]
            for member in workspace["members"]
        }
        # The two compile-time sharded packages appear once per feature group;
        # every other package belongs to exactly one collector.
        package_counts = Counter(selected_packages)
        self.assertEqual(package_counts.pop("clonk-app"), 6)
        self.assertEqual(package_counts.pop("clonk-engine-integration-tests"), 2)
        self.assertEqual(
            package_counts,
            Counter(
                expected_packages
                - {"clonk-app", "clonk-engine-integration-tests"}
            ),
        )

    def test_named_coverage_job_merges_fragments_before_enforcing_the_floor(self):
        coverage = job_block("coverage")

        self.assertIn("name: Rust code coverage", coverage)
        self.assertIn("needs: coverage-fragments", coverage)
        self.assertIn(
            'group: "main-coverage-report-${{ inputs.concurrency-suffix }}"',
            coverage,
        )
        self.assertIn("cancel-in-progress: true", coverage)
        self.assertEqual(coverage.count("actions/cache/restore@"), 11)
        self.assertEqual(coverage.count("fail-on-cache-miss: true"), 11)
        self.assertEqual(coverage.count("if: ${{ !inputs.upload-diagnostics }}"), 11)
        self.assertIn("actions/download-artifact@", coverage)
        self.assertIn("if: inputs.upload-diagnostics", coverage)
        self.assertIn(
            "pattern: rust-coverage-fragment-${{ github.run_id }}-*",
            coverage,
        )
        self.assertIn("merge-multiple: true", coverage)
        self.assertIn("EXPECTED_FRAGMENT_COUNT: '11'", coverage)
        self.assertIn('if [[ "${#fragments[@]}" -ne "$EXPECTED_FRAGMENT_COUNT" ]]', coverage)
        self.assertIn("diff -u", coverage)
        self.assertIn("python3 scripts/merge-rust-coverage.py", coverage)
        self.assertIn("COVERAGE_MIN_LINE_PERCENT: '79.45'", coverage)
        self.assertIn('--fail-under-lines "$COVERAGE_MIN_LINE_PERCENT"', coverage)
        self.assertNotIn("--check", coverage)
        self.assertNotIn("cargo llvm-cov --no-report nextest", coverage)
        self.assertNotIn("continue-on-error:", coverage)

    def test_fragment_handoffs_use_run_scoped_caches_not_release_artifacts(self):
        collectors = job_block("coverage-fragments")
        coverage = job_block("coverage")

        self.assertIn("actions/upload-artifact@", collectors)
        self.assertIn("if: inputs.upload-diagnostics", collectors)
        self.assertIn("retention-days: 1", collectors)
        self.assertIn("overwrite: true", collectors)
        self.assertIn(
            "name: rust-coverage-fragment-${{ github.run_id }}-"
            "${{ matrix.artifact }}",
            collectors,
        )
        self.assertIn("actions/cache/save@", collectors)
        self.assertEqual(
            collectors.count("if: ${{ !inputs.upload-diagnostics }}"), 2
        )
        self.assertIn("lookup-only: true", collectors)
        self.assertNotIn("actions: write", coverage)
        cache_lines = "\n".join(
            line
            for line in collectors.splitlines() + coverage.splitlines()
            if line.lstrip().startswith("key: rust-coverage-fragment-")
        )
        self.assertNotIn("github.run_attempt", cache_lines)

        artifacts = re.findall(r"(?m)^            artifact: (.+)$", collectors)
        self.assertEqual(len(artifacts), 11)
        self.assertIn(
            "target/coverage-fragments/${{ matrix.artifact }}.lcov.gz",
            collectors,
        )
        for artifact in artifacts:
            with self.subTest(artifact=artifact):
                path = f"target/coverage-fragments/{artifact}.lcov.gz"
                key = (
                    "rust-coverage-fragment-${{ github.run_id }}-"
                    f"{artifact}"
                )
                self.assertIn(path, coverage)
                self.assertIn(key, coverage)

    def test_coverage_collectors_use_the_pinned_instrumented_toolchain(self):
        collectors = job_block("coverage-fragments")

        self.assertIn("submodules: recursive", collectors)
        self.assertIn("components: llvm-tools-preview", collectors)
        self.assertIn("tool: cargo-nextest@0.9.91", collectors)
        self.assertIn("tool: cargo-llvm-cov@0.8.7", collectors)
        self.assertEqual(collectors.count("fallback: none"), 2)
        self.assertIn("LLVM_PROFILE_FILE_NAME: 'clonk-%8m.profraw'", collectors)
        self.assertIn("cache-targets: false", collectors)
        self.assertIn("shared-key: coverage-registry", collectors)
        self.assertIn(
            "save-if: ${{ inputs.upload-diagnostics "
            "&& matrix.artifact == 'app-1-10' }}",
            collectors,
        )
        self.assertIn("cargo llvm-cov clean --workspace", collectors)
        self.assertNotIn("continue-on-error:", collectors)

    def test_engine_unit_harnesses_do_not_extend_an_integration_tail(self):
        collectors = job_block("coverage-fragments")
        entries = re.findall(
            r"(?ms)^          - name: .+?(?=^          - name:|^    steps:)",
            collectors,
        )
        by_artifact = {
            re.search(r"(?m)^            artifact: (.+)$", entry).group(1): entry
            for entry in entries
        }

        first = by_artifact["engine-1"]
        second = by_artifact["engine-2"]
        units = by_artifact["engine-and-frontend-units"]
        self.assertNotIn("clonk-engine-unit-tests", first)
        self.assertNotIn("clonk-frontend-unit-tests", first)
        self.assertIn("-p clonk-engine-integration-tests", first)
        self.assertIn("engine-it-shard-1", first)
        self.assertNotIn("clonk-engine-unit-tests", second)
        self.assertNotIn("clonk-frontend-unit-tests", second)
        self.assertIn("-p clonk-engine-integration-tests", second)
        self.assertIn("engine-it-shard-2", second)
        self.assertIn("-p clonk-engine-unit-tests", units)
        self.assertIn("-p clonk-frontend-unit-tests", units)
        self.assertNotIn("clonk-engine-integration-tests", units)

    def test_coverage_reports_are_retained_when_the_floor_fails(self):
        coverage = job_block("coverage")
        command_text = re.sub(r"\s+", " ", coverage.replace("\\\n", " "))

        self.assertIn(
            "--output target/coverage/lcov.info",
            command_text,
        )
        self.assertIn(
            "genhtml target/coverage/lcov.info "
            "--output-directory target/coverage/html ",
            command_text,
        )
        upload_step = coverage[coverage.index("- name: Upload Rust coverage reports") :]
        self.assertIn("target/coverage/lcov.info", upload_step)
        self.assertIn("target/coverage/html", upload_step)
        self.assertNotIn("path: target/coverage\n", upload_step)
        merge = coverage.index("- name: Merge Rust coverage fragments")
        report = coverage.index("- name: Generate Rust coverage report")
        floor = coverage.index("- name: Enforce line coverage floor")
        upload = coverage.index("- name: Upload Rust coverage reports")
        self.assertLess(merge, report)
        self.assertLess(report, floor)
        self.assertLess(floor, upload)
        self.assertRegex(
            coverage,
            r"(?s)- name: Upload Rust coverage reports\s+"
            r"if: \$\{\{ always\(\) && inputs\.upload-diagnostics \}\}\s+"
            r"uses: actions/upload-artifact@"
            r"043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7\.0\.1\s+"
            r"with:\s+"
            r"name: rust-coverage-\$\{\{ github\.run_id \}\}-"
            r"\$\{\{ github\.run_attempt \}\}\s+"
            r"path: \|\s+"
            r"target/coverage/lcov\.info\s+"
            r"target/coverage/html\s+"
            r"if-no-files-found: warn\s+"
            r"retention-days: 14",
        )


if __name__ == "__main__":
    unittest.main()
