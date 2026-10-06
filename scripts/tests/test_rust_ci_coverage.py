"""Main validation keeps post-merge coverage visible and reproducible."""

import re
import tomllib
import unittest
from collections import Counter

from _repo import REPOSITORY

WORKFLOW = REPOSITORY / ".github" / "workflows" / "exact-sha-qualification.yml"
VERIFIED_CONTENT = REPOSITORY / ".github" / "actions" / "verified-content" / "action.yml"
WORKSPACE_CACHE = REPOSITORY / ".github" / "actions" / "workspace-cache" / "action.yml"

FRAGMENT_ARTIFACTS = {
    "app-1-10", "app-2-7", "app-3", "app-4-9", "app-5", "app-11-12", "app-6-8",
    "engine-1", "engine-2-3", "engine-and-frontend-units", "remaining-1", "remaining-2",
}
VERIFIED_RUN = "${{ needs.qualification-context.outputs.run-id || github.run_id }}"


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


def step_block(job, name, indent=6):
    prefix = " " * indent
    marker = f"{prefix}- name: {name}\n"
    start = job.index(marker)
    next_step = re.search(rf"(?m)^{prefix}- (?:name:|uses:)", job[start + len(marker) :])
    end = start + len(marker) + next_step.start() if next_step else None
    return job[start:end]


class RustCoverageGateTests(unittest.TestCase):
    def test_instrumented_junit_follows_the_pinned_nextest_store_outside_its_build_target(self):
        # A live 0.9.91 probe writes this report beneath the workspace-relative
        # nextest store even when Cargo binaries use target/coverage-build.
        config = tomllib.loads((REPOSITORY / ".config/nextest.toml").read_text(encoding="utf-8"))
        store = config.get("store", {}).get("dir", "target/nextest")
        report = config["profile"]["ci"]["junit"]["path"]
        collectors = job_block("coverage-fragments")
        self.assertIn("CARGO_TARGET_DIR: target/coverage-build", collectors)
        source = re.search(r"(?m)^      JUNIT_SOURCE: (.+)$", collectors)
        self.assertIsNotNone(source)
        self.assertEqual(source.group(1), f"{store}/ci/{report}")
        collect = step_block(collectors, "Collect instrumented coverage fragment")
        upload = step_block(collectors, "Upload shard test diagnostics")
        self.assertIn("LC_NEXTEST_JUNIT_SOURCE: ${{ env.JUNIT_SOURCE }}", collect)
        self.assertIn("${{ env.JUNIT_SOURCE }}", upload)
        self.assertIn('Path(os.environ["JUNIT_SOURCE"]).unlink(missing_ok=True)', collectors)

    def test_exported_instrumentation_uses_direct_nextest_without_a_recursive_wrapper(self):
        # cargo-llvm-cov v0.8.7 src/context.rs:85-96 exports its wrapper and
        # context. src/wrapper.rs:79-81,111-113,157-161 chains a prior wrapper;
        # running llvm-cov nextest again would therefore wrap itself. Its
        # src/cli.rs:1396-1398 also treats nextest --no-run as report-only.
        for job in ("coverage-fragments", "coverage-cache-warm"):
            with self.subTest(job=job):
                source = job_block(job)
                environment = step_block(source, "Resolve exact coverage build environment")
                self.assertIn('eval "$(cargo llvm-cov show-env --sh)"', environment)
                self.assertIn('os.environ["GITHUB_ENV"]', environment)
                for key in ("RUSTC_WRAPPER", "RUSTFLAGS", "RUSTDOCFLAGS", "LLVM_PROFILE", "CARGO_LLVM_COV", "__CARGO_LLVM_COV"):
                    self.assertIn(f'"{key}"', environment)
                steps = source[source.index("    steps:\n"):]
                self.assertNotIn("run: cargo ", steps[:steps.index(environment)])
                self.assertLess(source.index(environment), source.index("scripts/ci-workspace-cache.py prepare"))
                if job == "coverage-fragments":
                    commands = re.findall(r"(?m)^            command: (.+)$", source)
                    self.assertEqual(len(commands), 12)
                    for command in commands:
                        with self.subTest(command=command):
                            self.assertTrue(command.startswith("cargo nextest run "), command)
                            self.assertNotIn("llvm-cov", command)
                            self.assertNotIn("--no-run", command)
                            self.assertIn("--no-fail-fast", command)
                            self.assertIn("--locked", command)
                    collect = step_block(source, "Collect instrumented coverage fragment")
                    self.assertIn('bash -euo pipefail -c "$SHARD_COMMAND"', collect)
                    self.assertIn("cargo llvm-cov report --lcov", collect)
                else:
                    collect = step_block(source, "Compile instrumented inputs without rerunning qualified tests")
                    self.assertIn(
                        "cargo nextest run --no-run -p clonk-app --features app-test-shard-1,app-test-shard-10 --locked",
                        collect,
                    )
                    self.assertNotIn("cargo llvm-cov --no-report nextest", collect)
                self.assertIn("cargo llvm-cov clean --profraw-only", collect)
                self.assertLess(source.index(environment), source.index(collect))

    def test_reuse_hints_require_a_verified_receipt_and_a_miss_keeps_every_fresh_collector(self):
        context = job_block("qualification-context")
        self.assertIn("if: inputs.qualification-run-id != ''", context)
        self.assertIn("run-id: ${{ steps.evidence.outputs.run-id }}", context)
        self.assertIn("ref: ${{ inputs.source-sha }}", context)
        self.assertIn("uses: ./.github/actions/verified-content", context)
        self.assertNotIn("continue-on-error:", context)
        evidence = step_block(context, "Independently verify the exact-source qualification receipt")
        self.assertIn("id: evidence", evidence)
        self.assertIn("SOURCE_SHA: ${{ inputs.source-sha }}", evidence)
        self.assertIn("REPOSITORY: ${{ github.repository }}", evidence)
        self.assertIn("QUALIFICATION_RUN_ID: ${{ inputs.qualification-run-id }}", evidence)
        command = re.sub(r"\s+", " ", evidence.replace("\\\n", " "))
        self.assertIn(
            'python3 scripts/release-qualification-evidence.py resolve '
            '--repository "$REPOSITORY" --source-sha "$SOURCE_SHA" '
            '--run-id "$QUALIFICATION_RUN_ID" '
            '--output "$RUNNER_TEMP/qualification-context.json" '
            '--github-output "$GITHUB_OUTPUT"',
            command,
        )
        fallback = (
            "if: ${{ always() && (needs.qualification-context.result == 'success' "
            "|| needs.qualification-context.result == 'skipped') "
            "&& needs.qualification-context.outputs.run-id == '' }}"
        )
        for job in ("coverage-fragments", "recording-host-oracles", "platform-lints"):
            with self.subTest(job=job):
                source = job_block(job)
                self.assertRegex(source, r"(?m)^    needs: (?:qualification-context|\[[^\n]*\bqualification-context\b[^\n]*\])$")
                self.assertIn(fallback, source)
                self.assertNotIn("inputs.qualification-run-id", source)
                self.assertIn("ref: ${{ inputs.source-sha }}", source)
        collectors = job_block("coverage-fragments")
        artifacts = re.findall(r"(?m)^            artifact: (.+)$", collectors)
        self.assertEqual(Counter(artifacts), Counter(FRAGMENT_ARTIFACTS))

    def test_coverage_collectors_partition_every_workspace_test_once(self):
        collectors = job_block("coverage-fragments")
        commands = "\n".join(
            re.findall(r"(?m)^            command: (.+)$", collectors)
        )

        self.assertIn("name: Rust coverage / ${{ matrix.name }}", collectors)
        self.assertIn("timeout-minutes: ${{ matrix.timeout || 15 }}", collectors)
        self.assertIn("cargo llvm-cov clean --profraw-only", collectors)
        self.assertNotIn("cargo llvm-cov clean --workspace", collectors)
        self.assertIn("cargo nextest run", collectors)
        self.assertNotIn("cargo llvm-cov --no-report nextest", collectors)
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
                frozenset(("app-test-shard-3",)),
                frozenset(("app-test-shard-4", "app-test-shard-9")),
                frozenset(("app-test-shard-5",)),
                frozenset(("app-test-shard-11", "app-test-shard-12")),
                frozenset(("app-test-shard-6", "app-test-shard-8")),
            },
        )

        engine_features = re.findall(r"engine-it-shard-[1-9][0-9]*", commands)
        self.assertEqual(
            Counter(engine_features),
            Counter(
                {
                    "engine-it-shard-1",
                    "engine-it-shard-2",
                    "engine-it-shard-3",
                }
            ),
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
        self.assertEqual(package_counts.pop("clonk-app"), 7)
        self.assertEqual(package_counts.pop("clonk-engine-integration-tests"), 2)
        self.assertEqual(
            package_counts,
            Counter(
                expected_packages
                - {"clonk-app", "clonk-engine-integration-tests"}
            ),
        )

    def test_every_collector_row_declares_its_own_measured_budget(self):
        collectors = job_block("coverage-fragments")

        rows = re.findall(
            r"(?m)^          - name: (.+)\n            timeout: (\d+)$", collectors
        )
        self.assertEqual(len(rows), 12)

        budgets = dict(rows)
        # The common budget is the slowest green row measured over the
        # 2026-09-24/25 qualification runs (engine integration 2+3/3 at
        # 14m55s, app 1+10/12 at 14m32s) plus the five minutes a stalled
        # first content-fetch attempt costs before its bounded retry lands.
        # Under the previous 15-minute budget those rows were cancelled with
        # their coverage report already written, and one such cancellation
        # evicted the 1.1.0 release entry (clonk-org/clonk-rs#1802).
        #
        # At that budget decision, `app 5/12` had not finished an instrumented
        # run since 2026-09-25 15:28 UTC: every test in the row, capture or not,
        # ran two to three times slower than its last green run while the other
        # application rows kept their usual times. Preserve its investigation
        # budget while input reuse changes; that budget was not a measurement.
        self.assertEqual(budgets.pop("app 5/12"), "30")
        self.assertEqual({minutes for minutes in budgets.values()}, {"20"})

    def test_named_coverage_job_merges_fragments_before_enforcing_the_floor(self):
        coverage = job_block("coverage")

        self.assertIn("name: Rust code coverage", coverage)
        self.assertIn("needs: [coverage-fragments, qualification-context]", coverage)
        self.assertIn(
            "if: ${{ always() && (needs.qualification-context.result == 'success' "
            "|| needs.qualification-context.result == 'skipped') "
            "&& (needs.coverage-fragments.result == 'success' "
            "|| needs.qualification-context.outputs.run-id != '') }}",
            coverage,
        )
        self.assertIn(
            'group: "main-coverage-report-${{ inputs.concurrency-suffix }}"',
            coverage,
        )
        self.assertIn("cancel-in-progress: true", coverage)
        self.assertNotIn("actions/cache/restore@", coverage)
        self.assertNotIn("fail-on-cache-miss: true", coverage)
        download = step_block(coverage, "Download coverage fragments")
        self.assertIn("actions/download-artifact@", download)
        self.assertIn("github-token: ${{ github.token }}", download)
        self.assertIn(f"run-id: {VERIFIED_RUN}", download)
        self.assertNotIn("if: inputs.upload-diagnostics", coverage)
        self.assertNotIn("if: ${{ !inputs.upload-diagnostics }}", coverage)
        self.assertIn(
            f"pattern: rust-coverage-fragment-{VERIFIED_RUN}-*",
            download,
        )
        self.assertIn("merge-multiple: true", coverage)
        self.assertIn("EXPECTED_FRAGMENT_COUNT: '12'", coverage)
        self.assertIn('if [[ "${#fragments[@]}" -ne "$EXPECTED_FRAGMENT_COUNT" ]]', coverage)
        self.assertIn("diff -u", coverage)
        expected = re.search(r"(?ms)^          expected=\(\n(.+?)^          \)", coverage)
        self.assertIsNotNone(expected)
        self.assertEqual(set(expected.group(1).split()), FRAGMENT_ARTIFACTS)
        self.assertNotIn("inputs.qualification-run-id", coverage)
        self.assertIn("ref: ${{ inputs.source-sha }}", coverage)
        self.assertIn("python3 scripts/merge-rust-coverage.py", coverage)
        self.assertIn("COVERAGE_MIN_LINE_PERCENT: '79.45'", coverage)
        self.assertIn('--fail-under-lines "$COVERAGE_MIN_LINE_PERCENT"', coverage)
        self.assertNotIn("--output", coverage)
        self.assertIn(
            '--fail-under-lines "$COVERAGE_MIN_LINE_PERCENT" '
            "target/coverage-fragments/*.lcov",
            re.sub(r"\s+", " ", coverage.replace("\\\n", " ")),
        )
        self.assertNotIn("--check", coverage)
        self.assertNotIn("cargo llvm-cov --no-report nextest", coverage)
        self.assertNotIn("actions/upload-artifact@", coverage)
        self.assertNotIn("continue-on-error:", coverage)

    def test_fragment_handoffs_use_run_scoped_artifacts_for_every_qualification(self):
        collectors = job_block("coverage-fragments")
        coverage = job_block("coverage")

        upload = step_block(collectors, "Upload coverage fragment")
        self.assertIn("uses: actions/upload-artifact@", upload)
        self.assertNotIn("if: always()", upload)
        self.assertNotIn("continue-on-error:", upload)
        self.assertIn("if-no-files-found: error", upload)
        self.assertIn(
            "retention-days: ${{ inputs.upload-diagnostics && 1 || 30 }}",
            upload,
        )
        self.assertIn("overwrite: true", upload)
        self.assertIn(
            "name: rust-coverage-fragment-${{ github.run_id }}-"
            "${{ matrix.artifact }}",
            upload,
        )
        self.assertNotIn("actions/cache/save@", collectors)
        self.assertNotIn("actions/cache/restore@", collectors)
        collect = step_block(collectors, "Collect instrumented coverage fragment")
        self.assertNotIn("\n        if:", collect)
        self.assertNotIn("\n        if:", upload)
        self.assertNotIn("lookup-only: true", collectors)
        self.assertNotIn("actions: write", coverage)
        self.assertNotIn("key: rust-coverage-fragment-", collectors + coverage)

        artifacts = re.findall(r"(?m)^            artifact: (.+)$", collectors)
        self.assertEqual(len(artifacts), 12)
        self.assertEqual(Counter(artifacts), Counter(FRAGMENT_ARTIFACTS))
        self.assertIn("app-3", artifacts)
        self.assertIn("app-11-12", artifacts)
        self.assertNotIn("app-3-12", artifacts)
        self.assertIn("app-5", artifacts)
        self.assertNotIn("app-5-11", artifacts)
        self.assertIn(
            "target/coverage-fragments/${{ matrix.artifact }}.lcov.gz",
            upload,
        )
        self.assertIn(
            f"pattern: rust-coverage-fragment-{VERIFIED_RUN}-*",
            coverage,
        )
        self.assertIn(f"run-id: {VERIFIED_RUN}", coverage)
        self.assertNotIn("inputs.qualification-run-id", coverage)
        self.assertIn("ref: ${{ inputs.source-sha }}", collectors)

    def test_coverage_collectors_use_the_pinned_instrumented_toolchain(self):
        collectors = job_block("coverage-fragments")

        self.assertIn("ref: ${{ inputs.source-sha }}", collectors)
        content = step_block(collectors, "Materialize verified pinned content")
        self.assertIn("uses: ./.github/actions/verified-content", content)
        self.assertNotIn("\n        if:", content)
        self.assertNotIn("submodules:", collectors)
        self.assertNotIn("git submodule update", collectors)
        content_action = VERIFIED_CONTENT.read_text(encoding="utf-8")
        self.assertIn("git rev-parse HEAD:content", content_action)
        materialize = step_block(content_action, "Materialize and verify pinned content", indent=4)
        self.assertIn('python3 scripts/ci-content.py --revision "$CONTENT_REVISION"', materialize)
        self.assertIn('exit "$failed"', materialize)
        self.assertNotIn("\n      if:", materialize)
        self.assertNotIn("continue-on-error:", materialize)
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
        collect = step_block(collectors, "Collect instrumented coverage fragment")
        self.assertIn("cargo llvm-cov clean --profraw-only", collect)
        self.assertNotIn("cargo llvm-cov clean --workspace", collect)
        self.assertNotIn("continue-on-error:", collect)

    def test_instrumented_input_reuse_requires_external_ledger_verification_before_fresh_results(self):
        collectors = job_block("coverage-fragments")
        environment = step_block(collectors, "Resolve exact coverage build environment")
        restore = step_block(collectors, "Restore verified instrumented workspace inputs")
        verify = step_block(collectors, "Verify restored coverage build inputs and discard prior results")
        collect = step_block(collectors, "Collect instrumented coverage fragment")
        record = step_block(collectors, "Record verified coverage build inputs")
        publish = step_block(collectors, "Publish verified instrumented workspace inputs")

        self.assertIn('eval "$(cargo llvm-cov show-env --sh)"', environment)
        self.assertIn('os.environ["GITHUB_ENV"]', environment)
        for step, operation in ((restore, "restore"), (publish, "save")):
            with self.subTest(operation=operation):
                self.assertIn("uses: ./.github/actions/workspace-cache", step)
                self.assertIn(f"operation: {operation}", step)
                self.assertIn("lane: coverage-linux", step)
                self.assertIn("target: target/coverage-build", step)
                self.assertIn("ledger: .ci-cache-ledgers/coverage.json", step)
                self.assertIn("recipe: coverage-v1", step)
        arguments = "--target target/coverage-build --recipe coverage-v1 --ledger .ci-cache-ledgers/coverage.json"
        self.assertIn(f"python3 scripts/ci-workspace-cache.py prepare {arguments}", verify)
        self.assertIn(f"python3 scripts/ci-workspace-cache.py record {arguments}", record)
        for step in (verify, collect, record):
            with self.subTest(step=step.splitlines()[0]):
                self.assertNotIn("continue-on-error:", step)
                self.assertNotIn("\n        if:", step)
        self.assertLess(collectors.index(environment), collectors.index(restore))
        self.assertLess(collectors.index(restore), collectors.index(verify))
        self.assertLess(collectors.index(verify), collectors.index(collect))
        self.assertLess(collect.index("cargo llvm-cov clean --profraw-only"), collect.index('bash -euo pipefail -c "$SHARD_COMMAND"'))
        self.assertLess(collect.index('bash -euo pipefail -c "$SHARD_COMMAND"'), collect.index("cargo llvm-cov report --lcov"))
        self.assertLess(collectors.index(collect), collectors.index(record))
        self.assertLess(collectors.index(record), collectors.index(publish))
        cache_action = WORKSPACE_CACHE.read_text(encoding="utf-8")
        self.assertIn("${{ steps.identity.outputs.target }}", cache_action)
        self.assertIn("${{ steps.identity.outputs.ledger }}", cache_action)
        self.assertNotIn("rust-coverage-fragment", cache_action)

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
        second = by_artifact["engine-2-3"]
        units = by_artifact["engine-and-frontend-units"]
        self.assertNotIn("clonk-engine-unit-tests", first)
        self.assertNotIn("clonk-frontend-unit-tests", first)
        self.assertIn("-p clonk-engine-integration-tests", first)
        self.assertIn("engine-it-shard-1", first)
        self.assertNotIn("clonk-engine-unit-tests", second)
        self.assertNotIn("clonk-frontend-unit-tests", second)
        self.assertIn("-p clonk-engine-integration-tests", second)
        self.assertIn("engine-it-shard-2", second)
        self.assertIn("engine-it-shard-3", second)
        self.assertIn("-p clonk-engine-unit-tests", units)
        self.assertIn("-p clonk-frontend-unit-tests", units)
        self.assertNotIn("clonk-engine-integration-tests", units)

    def test_coverage_reports_are_retained_when_the_floor_fails(self):
        coverage = job_block("coverage")
        html = job_block("coverage-html")
        command_text = re.sub(r"\s+", " ", coverage.replace("\\\n", " "))
        html_command_text = re.sub(r"\s+", " ", html.replace("\\\n", " "))

        self.assertNotIn("--output", command_text)
        self.assertNotIn("genhtml", coverage)
        self.assertIn(
            "genhtml target/coverage/lcov.info "
            "--output-directory target/coverage/html ",
            html_command_text,
        )
        merge = coverage.index("- name: Enforce merged line coverage floor")
        self.assertEqual(
            coverage.count("python3 scripts/merge-rust-coverage.py"), 1
        )
        self.assertIn('--fail-under-lines "$COVERAGE_MIN_LINE_PERCENT"', coverage[merge:])
        self.assertIn("name: Rust coverage HTML report", html)
        self.assertIn("needs: [coverage-fragments, qualification-context]", html)
        self.assertIn(
            "if: ${{ always() && inputs.upload-diagnostics "
            "&& (needs.qualification-context.result == 'success' "
            "|| needs.qualification-context.result == 'skipped') "
            "&& (needs.coverage-fragments.result == 'success' "
            "|| needs.qualification-context.outputs.run-id != '') }}",
            html,
        )
        self.assertNotIn("needs: coverage\n", html)
        self.assertIn(
            'group: "main-coverage-html-${{ inputs.concurrency-suffix }}"',
            html,
        )
        download = step_block(html, "Download Rust coverage fragments")
        self.assertIn("actions/download-artifact@", download)
        self.assertIn("github-token: ${{ github.token }}", download)
        self.assertIn(f"run-id: {VERIFIED_RUN}", download)
        self.assertIn(
            f"pattern: rust-coverage-fragment-{VERIFIED_RUN}-*", download
        )
        self.assertIn("EXPECTED_FRAGMENT_COUNT: '12'", html)
        self.assertIn("python3 scripts/merge-rust-coverage.py", html)
        self.assertIn("--output target/coverage/lcov.info", html_command_text)
        self.assertNotIn("--fail-under-lines", html)
        self.assertNotIn("overwrite: true", html)
        self.assertNotIn("inputs.qualification-run-id", html)
        expected = re.search(r"(?ms)^          expected=\(\n(.+?)^          \)", html)
        self.assertIsNotNone(expected)
        self.assertEqual(set(expected.group(1).split()), FRAGMENT_ARTIFACTS)
        self.assertIn('if [[ "${#fragments[@]}" -ne "$EXPECTED_FRAGMENT_COUNT" ]]', html)
        self.assertIn("diff -u", html)
        self.assertIn("ref: ${{ inputs.source-sha }}", html)
        self.assertIn("if: always()", step_block(html, "Upload Rust coverage reports"))
        self.assertIn("target/coverage/lcov.info", html)
        self.assertIn("target/coverage/html", html)


if __name__ == "__main__":
    unittest.main()
