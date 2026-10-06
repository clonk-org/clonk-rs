"""Static guards for the landing path's coverage and latency budget."""

import os
import re
import subprocess
import tempfile
import textwrap
import tomllib
import unittest
from collections import Counter
from pathlib import Path

from _repo import REPOSITORY

WORKFLOWS = REPOSITORY / ".github" / "workflows"
ACTIONS = REPOSITORY / ".github" / "actions"
LANDING = WORKFLOWS / "landing.yml"
MAIN = WORKFLOWS / "rust.yml"
QUALIFICATION = WORKFLOWS / "exact-sha-qualification.yml"
DEPENDENCY_GUARD = WORKFLOWS / "dependency-guard.yml"
APT_INSTALLER = REPOSITORY / "scripts" / "install-apt-packages.sh"
VERIFIED_CONTENT = ACTIONS / "verified-content" / "action.yml"
CONTENT_HELPER = REPOSITORY / "scripts" / "ci-content.py"
WORKSPACE_CACHE = ACTIONS / "workspace-cache" / "action.yml"

NATIVE_DEPENDENCY_ROLES = {
    ("workflows/landing.yml", "pull-request-quality"),
    ("workflows/exact-sha-qualification.yml", "coverage-fragments"),
    ("workflows/exact-sha-qualification.yml", "coverage-cache-warm"),
    ("workflows/release-build.yml", "package"),
    ("workflows/release-prebuild.yml", "tool"),
    ("workflows/release-prebuild.yml", "runtime"),
    ("actions/device-loss/action.yml", "composite"),
    ("actions/verified-content/action.yml", "composite"),
}

# One step, from its first key to the next step's. `actions/cache/restore` is
# deliberately not matched: only the save halves can consume the budget.
STEP = re.compile(
    r"(?ms)^(?:      |    )- (?:name|uses|id):.*?"
    r"(?=^(?:      |    )- (?:name|uses|id):|^  [a-z][a-z0-9_-]*:|\Z)"
)
CACHE_WRITER = re.compile(r"Swatinem/rust-cache@|actions/cache(?:/save)?@")


def ci_sources():
    """Include shared implementations as well as their workflow callers."""
    return sorted(WORKFLOWS.glob("*.yml")) + sorted(ACTIONS.glob("*/action.yml"))


def matrix_entry(workflow, name):
    """Return one literal Linux matrix entry."""
    marker = f"          - name: {name}\n"
    start = workflow.index(marker)
    end = workflow.find("\n          - name: ", start + len(marker))
    return workflow[start : end if end >= 0 else len(workflow)]


def cache_steps(workflow):
    """Return every step that can publish an Actions cache entry."""
    return [step for step in STEP.findall(workflow) if CACHE_WRITER.search(step)]


def workspace_cache_steps(workflow, operation):
    """Return explicit restore, lookup or publish phases of the target cache."""
    return [
        step for step in STEP.findall(workflow)
        if "uses: ./.github/actions/workspace-cache" in step
        and re.search(rf"(?m)^          operation: {re.escape(operation)}\s*$", step)
    ]


def ci_jobs(path, workflow):
    """Return jobs or the one composite, preserving exact consumer identities."""
    if path.name == "action.yml":
        return [("composite", workflow)]
    return re.findall(
        r"(?ms)^  ([a-z][a-z0-9_-]*):\n(.*?)(?=^  [a-z][a-z0-9_-]*:\n|\Z)",
        workflow.split("jobs:\n", 1)[1],
    )


def native_dependency_steps():
    return [
        ((path.relative_to(REPOSITORY / ".github").as_posix(), name), step)
        for path in ci_sources()
        for name, job in ci_jobs(path, path.read_text(encoding="utf-8"))
        for step in STEP.findall(job)
        if "scripts/install-apt-packages.sh" in step
    ]


def workflow_job(workflow, name):
    """Return a literal job without absorbing newly inserted sibling jobs."""
    return re.search(
        rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [a-z][a-z0-9_-]*:\n|\Z)",
        workflow,
    ).group(0)


def fires_on_a_non_default_ref(workflow):
    """Report whether an event can run this workflow off the default branch."""
    triggers = re.search(r"(?ms)^on:\n(.*?)^(?=[a-z])", workflow).group(1)
    return "pull_request:" in triggers or "merge_group:" in triggers


class CiLatencyTests(unittest.TestCase):
    def test_qualification_uses_portable_command_timeouts(self):
        qualification = QUALIFICATION.read_text(encoding="utf-8")
        action = VERIFIED_CONTENT.read_text(encoding="utf-8")
        helper = CONTENT_HELPER.read_text(encoding="utf-8")

        self.assertNotIn('timeout "$budget" git submodule update', qualification)
        for name in (
            "coverage-fragments", "developer-feedback", "recording-host-oracles",
            "qualification-context",
        ):
            with self.subTest(job=name):
                self.assertEqual(
                    workflow_job(qualification, name).count(
                        "uses: ./.github/actions/verified-content"
                    ),
                    1,
                )
        self.assertNotIn("git submodule update", qualification)
        self.assertIn('python3 scripts/ci-content.py --revision "$CONTENT_REVISION"', action)
        self.assertIn('str(repository / "scripts" / "run_with_timeout.py")', helper)
        self.assertIn("wrapped = [sys.executable,", helper)
        self.assertIn("ATTEMPT_BUDGETS = (300, 180)", helper)
        self.assertIn('"submodule", "update"', helper)
        self.assertIn("status = run_bounded(", helper)
        self.assertNotIn('subprocess.run(["timeout"', helper)

    def test_native_dependency_install_retries_are_shared_and_bounded(self):
        installer = APT_INSTALLER.read_text(encoding="utf-8")
        workflows = "\n".join(
            path.read_text(encoding="utf-8") for path in ci_sources()
        )

        installations = native_dependency_steps()
        self.assertEqual({role for role, step in installations}, NATIVE_DEPENDENCY_ROLES)
        self.assertEqual(len(installations), len(NATIVE_DEPENDENCY_ROLES))
        self.assertEqual(workflows.count("scripts/install-apt-packages.sh"), len(NATIVE_DEPENDENCY_ROLES))
        self.assertNotIn("apt_install()", workflows)
        self.assertTrue(APT_INSTALLER.stat().st_mode & 0o111)
        self.assertLess(installer.index("apt_install"), installer.index("apt_refresh"))
        self.assertIn("timeout \"$budget\" sudo apt-get", installer)

        # A stalled mirror that accepts the connection and then stops sending
        # is bounded by apt itself rather than by the wall-clock timeout, which
        # would otherwise spend its whole budget transferring nothing.
        self.assertIn("-o Acquire::http::Timeout=15", installer)
        self.assertIn("-o Acquire::https::Timeout=15", installer)

    def test_native_dependency_retries_move_off_the_mirror_that_failed(self):
        installer = APT_INSTALLER.read_text(encoding="utf-8")

        # Every apt-get invocation re-reads the runner mirror list and picks the
        # same head entry, so a retry that does not rotate is the request that
        # just failed, sent again.
        self.assertIn("rotate_mirror()", installer)
        self.assertEqual(installer.count("\n    rotate_mirror\n"), 2)
        self.assertIn("http://archive.ubuntu.com/ubuntu/", installer)

        retry = installer.index("for attempt in")
        for call in re.finditer(r"^    if apt_install", installer[retry:], re.M):
            following = installer[retry + call.start() :]
            self.assertIn("rotate_mirror", following.split("exit 0", 1)[1])

    def test_native_dependency_ladder_fits_every_step_that_runs_it(self):
        installer = APT_INSTALLER.read_text(encoding="utf-8")
        budget = int(
            re.search(r"readonly LADDER_BUDGET_SECONDS=(\d+)", installer).group(1)
        )

        # The ladder is killed mid-attempt, reporting neither package nor
        # mirror, whenever it can outlast the step that runs it.
        steps = []
        installations = native_dependency_steps()
        self.assertEqual({role for role, step in installations}, NATIVE_DEPENDENCY_ROLES)
        for role, step in installations:
            minutes = re.search(r"timeout-minutes: (\d+)", step)
            self.assertIsNotNone(minutes, f"{role} bounds no apt step")
            steps.append(int(minutes.group(1)) * 60)

        self.assertEqual(len(steps), len(NATIVE_DEPENDENCY_ROLES))
        self.assertLess(budget, min(steps))

    def test_restore_only_landing_caches_have_trusted_main_producers(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")

        scopes = set(re.findall(r"shared-key: ([a-z0-9-]+)", landing))
        self.assertEqual(scopes, {"full-parity", "windows-runtime-msvc-v2"})
        # Three restore-only consumers: pull-request quality, the Linux
        # matrix rows, and the Windows runtime row.
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
                "  diagnostic-admission:"
            )
        ]
        self.assertIn("shared-key: full-parity", linux_producer)
        self.assertNotIn("cache-on-failure:", linux_producer)
        self.assertIn("cache-targets: false", linux_producer)
        prepare = next(
            step for step in STEP.findall(linux_producer)
            if "scripts/ci-workspace-cache.py prepare " in step
        )
        compile_graph = next(
            step for step in STEP.findall(linux_producer)
            if "cargo nextest run --workspace" in step
        )
        record = next(
            step for step in STEP.findall(linux_producer)
            if "scripts/ci-workspace-cache.py record " in step
        )
        self.assertIn(
            "cargo nextest run --workspace --features xtask/engine-tools "
            "--no-run --locked",
            compile_graph,
        )
        for step in (prepare, compile_graph, record):
            self.assertNotIn("if:", step)
            self.assertNotIn("continue-on-error", step)
        for step in (prepare, record):
            self.assertIn("--target target", step)
            self.assertIn("--recipe landing-v1", step)
        # Immutable cache hits may contain older workspace units. Validation
        # and an exhaustive Cargo compile still run before recording provenance.
        self.assertLess(linux_producer.index(prepare), linux_producer.index(compile_graph))
        self.assertLess(linux_producer.index(compile_graph), linux_producer.index(record))
        self.assertLess(
            linux_producer.index("cargo build --locked --release -p clonk-app --features presentation-capture"),
            linux_producer.index(record),
        )

        windows_producer = main[
            main.index("  windows-landing-cache:") : main.index(
                "  windows-release-tools:"
            )
        ]
        release_tools = workflow_job(main, "windows-release-tools")
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
        self.assertIn("needs: [windows-landing-cache, qualification-reuse]", release_tools)
        self.assertIn("needs.windows-landing-cache.result == 'success'", release_tools)
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
        action = VERIFIED_CONTENT.read_text(encoding="utf-8")
        helper = CONTENT_HELPER.read_text(encoding="utf-8")
        content_producer = main[
            main.index("  content-landing-cache:") : main.index(
                "  linux-landing-cache:"
            )
        ]
        linux_producer = main[
            main.index("  linux-landing-cache:") : main.index(
                "  diagnostic-admission:"
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
            self.assertEqual(
                consumer.count("uses: ./.github/actions/verified-content"), 1
            )
            self.assertNotIn("publish: 'true'", consumer)
            self.assertNotIn("actions/cache/save@", consumer)

        restore = next(
            step for step in STEP.findall(action) if "actions/cache/restore@" in step
        )
        materialize = next(
            step for step in STEP.findall(action) if "id: materialize" in step
        )
        self.assertIn("value: ${{ steps.identity.outputs.revision }}", action)
        self.assertIn("git rev-parse HEAD:content", action)
        self.assertIn("path: .git/modules/content", restore)
        self.assertIn(
            "key: clonk-content-git-v2-${{ hashFiles('.gitmodules') }}-"
            "${{ steps.identity.outputs.revision }}",
            restore,
        )
        self.assertIn("enableCrossOsArchive: true", restore)
        self.assertIn("continue-on-error: true", restore)
        self.assertNotIn("restore-keys:", restore)
        self.assertNotIn("fail-on-cache-miss", restore)
        self.assertNotIn("continue-on-error", materialize)
        self.assertNotIn("if:", materialize)
        self.assertIn(
            'python3 scripts/ci-content.py --revision "$CONTENT_REVISION"',
            materialize,
        )

        # A cache hit is only an optional object source. Every consumer still
        # validates the parent gitlink, exact HEAD and actual clean input bytes.
        for validation in (
            "def verify_parent_gitlink(",
            'git(repository, "ls-tree", "HEAD", "--", "content")',
            "if match.group(1) != revision:",
            'if git(content, "rev-parse", "HEAD") != revision:',
            'if git(content, "status", "--porcelain=v1", "--untracked-files=all"):',
            'if git(content, "ls-files", "--others", "-z"):',
            '"read-tree", revision',
            '"update-index", "--really-refresh"',
            '"diff-files", "--quiet", "--no-ext-diff"',
            "verify_parent_gitlink(REPOSITORY, arguments.revision)",
            "verify_checkout(REPOSITORY, arguments.revision)",
            "verify_checkout(repository, revision)",
        ):
            with self.subTest(validation=validation):
                self.assertIn(validation, helper)
        self.assertRegex(
            helper,
            r'"submodule",\s*"update",\s*"--init",\s*"--force",\s*"--checkout"',
        )
        self.assertRegex(
            helper, r'"--depth=1",\s*"--filter=blob:none",\s*"--",\s*"content"'
        )

        self.assertIn("needs: content-landing-cache", linux_producer)
        self.assertNotIn("submodules: recursive", content_producer)
        self.assertIn(
            "uses: ./.github/actions/verified-content",
            content_producer,
        )
        self.assertIn("publish: 'true'", content_producer)
        publications = [
            step for step in STEP.findall(action)
            if "actions/cache/save@" in step
            or "uses: ./.github/actions/verify-cache-handoff" in step
        ]
        self.assertEqual(len(publications), 2)
        for step in publications:
            self.assertIn(
                "if: inputs.publish == 'true' && github.ref == 'refs/heads/main' "
                "&& steps.cache.outputs.cache-hit != 'true'",
                step,
            )
            self.assertIn("path: .git/modules/content", step)
            self.assertIn(
                "key: clonk-content-git-v2-${{ hashFiles('.gitmodules') }}-"
                "${{ steps.identity.outputs.revision }}",
                step,
            )
        self.assertIn("github.event_name == 'workflow_dispatch'", content_producer)
        self.assertIn("github.sha || 'rolling'", content_producer)
        self.assertIn("cancel-in-progress: false", content_producer)

    def test_cache_producers_finish_while_obsolete_diagnostics_cancel(self):
        main = MAIN.read_text(encoding="utf-8")
        qualification = QUALIFICATION.read_text(encoding="utf-8")

        self.assertNotRegex(main, r"(?m)^concurrency:\s*$")
        linux_producer = main[
            main.index("  linux-landing-cache:") : main.index(
                "  diagnostic-admission:"
            )
        ]
        caller = main[
            main.index("  exact-sha-qualification:") : main.index(
                "  windows-landing-cache:"
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
        self.assertIn("needs: [diagnostic-admission, qualification-reuse]", caller)
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
        linux_lookups = workspace_cache_steps(verifier, "lookup")
        self.assertEqual(len(linux_lookups), 1)
        self.assertIn("if: matrix.name == 'Linux'", linux_lookups[0])
        self.assertIn("id: linux-cache", linux_lookups[0])
        for field in (
            "lane: landing-linux", "target: target",
            "ledger: .ci-cache-ledgers/landing.json", "recipe: landing-v1",
        ):
            self.assertIn(field, linux_lookups[0])
        self.assertEqual(workspace_cache_steps(verifier, "save"), [])
        self.assertIn("shared-key: windows-runtime-msvc-v2", verifier)
        self.assertIn("lookup-only: true", verifier)
        self.assertIn("save-if: false", verifier)
        self.assertIn('[[ "$CACHE_HIT" == "true" ]]', verifier)

    def test_post_merge_work_leaves_the_next_landing_runner_budget(self):
        main = MAIN.read_text(encoding="utf-8")
        dependency_guard = DEPENDENCY_GUARD.read_text(encoding="utf-8")

        qualification = main[
            main.index("  exact-sha-qualification:") : main.index(
                "  windows-landing-cache:"
            )
        ]
        admission = workflow_job(main, "diagnostic-admission")
        self.assertIn("needs: linux-landing-cache", admission)
        self.assertIn("needs: [diagnostic-admission, qualification-reuse]", qualification)

        triggers = dependency_guard[
            dependency_guard.index("on:\n") : dependency_guard.index("permissions:\n")
        ]
        self.assertNotIn("\n  push:\n", triggers)

    def test_release_receipt_follows_all_required_jobs_and_is_itself_required(self):
        landing = LANDING.read_text(encoding="utf-8")
        evidence = workflow_job(landing, "release-evidence")
        gate = workflow_job(landing, "landing-gate")

        self.assertIn(
            "needs: [release-context, release-build, release-qualification, linux, windows-smoke]",
            evidence,
        )
        self.assertIn(
            "if: github.event_name == 'merge_group' && needs.release-context.outputs.release == 'true'",
            evidence,
        )
        self.assertIn("uses: ./.github/actions/verified-content", evidence)
        write = next(
            step for step in STEP.findall(evidence)
            if "scripts/release-qualification-evidence.py write " in step
        )
        self.assertIn("SOURCE_SHA: ${{ github.sha }}", write)
        self.assertIn("REPOSITORY: ${{ github.repository }}", write)
        self.assertIn('--repository "$REPOSITORY" --source-sha "$SOURCE_SHA"', write)
        self.assertIn('--run-id "$GITHUB_RUN_ID"', write)
        self.assertNotIn("continue-on-error", write)
        upload = next(
            step for step in STEP.findall(evidence) if "actions/upload-artifact@" in step
        )
        self.assertLess(evidence.index(write), evidence.index(upload))
        self.assertIn("name: release-qualification-evidence-${{ github.sha }}", upload)
        self.assertIn("path: ${{ runner.temp }}/release-qualification-evidence.json", upload)
        self.assertIn("if-no-files-found: error", upload)
        self.assertNotIn("continue-on-error", upload)
        self.assertIn("      - release-evidence\n", gate)
        self.assertIn("RELEASE_EVIDENCE_RESULT: ${{ needs.release-evidence.result }}", gate)
        self.assertEqual(gate.count('require_result release-evidence "$RELEASE_EVIDENCE_RESULT" success'), 1)
        self.assertEqual(gate.count('require_result release-evidence "$RELEASE_EVIDENCE_RESULT" skipped'), 3)

    def test_qualification_reuse_depends_on_fresh_exact_source_validation(self):
        qualification = QUALIFICATION.read_text(encoding="utf-8")
        self.assertRegex(
            qualification,
            r"(?m)^      qualification-run-id:\n(?:        .*\n)*?        default: ''\n",
        )
        context = workflow_job(qualification, "qualification-context")
        self.assertIn("ref: ${{ inputs.source-sha }}", context)
        self.assertIn("uses: ./.github/actions/verified-content", context)
        self.assertIn("  actions: read\n", qualification[:qualification.index("jobs:\n")])
        self.assertNotIn("actions: write", qualification)
        self.assertIn("scripts/release-qualification-evidence.py resolve", context)
        self.assertIn('--repository "$REPOSITORY" --source-sha "$SOURCE_SHA"', context)
        self.assertIn("SOURCE_SHA: ${{ inputs.source-sha }}", context)
        self.assertIn("QUALIFICATION_RUN_ID: ${{ inputs.qualification-run-id }}", context)
        self.assertIn('--run-id "$QUALIFICATION_RUN_ID"', context)
        self.assertRegex(
            context,
            r"(?m)^      run-id: \$\{\{ steps\.[a-z0-9_-]+\.outputs\.run-id \}\}\n",
        )
        self.assertNotIn("continue-on-error", context)
        # A caller's raw run ID is only a hint. Required collectors and native
        # gates may skip solely from the freshly validated context's output.
        for name in (
            "device-loss", "coverage-fragments", "recording-host-oracles", "platform-lints"
        ):
            job = workflow_job(qualification, name)
            with self.subTest(job=name):
                self.assertIn("needs: qualification-context", job)
                self.assertIn("needs.qualification-context.outputs.run-id == ''", job)
                self.assertIn("always()", job)
                self.assertIn("needs.qualification-context.result == 'success'", job)
                self.assertIn("needs.qualification-context.result == 'skipped'", job)
                self.assertNotIn("inputs.qualification-run-id", job)

    def test_reused_coverage_still_runs_the_full_floor_html_and_replay_diagnostics(self):
        qualification = QUALIFICATION.read_text(encoding="utf-8")
        expected = {
            "app-1-10", "app-2-7", "app-3", "app-4-9", "app-5", "app-11-12",
            "app-6-8", "engine-1", "engine-2-3", "engine-and-frontend-units",
            "remaining-1", "remaining-2",
        }
        for name in ("coverage", "coverage-html"):
            job = workflow_job(qualification, name)
            with self.subTest(job=name):
                self.assertIn("needs: [coverage-fragments, qualification-context]", job)
                self.assertIn("always()", job)
                self.assertIn("needs.qualification-context.result == 'success'", job)
                self.assertIn("needs.qualification-context.result == 'skipped'", job)
                self.assertIn("needs.coverage-fragments.result == 'success'", job)
                self.assertIn("needs.qualification-context.outputs.run-id != ''", job)
                self.assertNotIn("inputs.qualification-run-id", job)
                download = next(
                    step for step in STEP.findall(job) if "actions/download-artifact@" in step
                )
                self.assertIn("github-token: ${{ github.token }}", download)
                self.assertIn(
                    "run-id: ${{ needs.qualification-context.outputs.run-id || github.run_id }}",
                    download,
                )
                self.assertIn(
                    "pattern: rust-coverage-fragment-${{ needs.qualification-context.outputs.run-id || github.run_id }}-*",
                    download,
                )
                self.assertIn("path: target/coverage-fragments", download)
                self.assertNotIn("continue-on-error", download)
                self.assertIn("EXPECTED_FRAGMENT_COUNT: '12'", job)
                names = re.search(r"(?ms)^          expected=\(\n(.*?)^          \)\n", job).group(1).split()
                self.assertEqual(Counter(names), Counter(expected))
                self.assertIn('"${#fragments[@]}" -ne "$EXPECTED_FRAGMENT_COUNT"', job)
                self.assertIn('if [[ "$actual_names" != "$expected_names" ]]', job)
                self.assertIn("python3 scripts/merge-rust-coverage.py", job)
        coverage = workflow_job(qualification, "coverage")
        self.assertIn("COVERAGE_MIN_LINE_PERCENT: '79.45'", coverage)
        self.assertIn('--fail-under-lines "$COVERAGE_MIN_LINE_PERCENT"', coverage)
        html = workflow_job(qualification, "coverage-html")
        self.assertIn("inputs.upload-diagnostics", html)
        self.assertIn("genhtml target/coverage/lcov.info", html)
        developer = workflow_job(qualification, "developer-feedback")
        self.assertNotIn("qualification-context", developer)
        self.assertNotIn("qualification-run-id", developer)
        self.assertIn("if: inputs.upload-diagnostics", developer)
        self.assertIn(
            "dev_feedback_replay::real_scenario_replays_repeat_with_native_group_order",
            developer,
        )
        self.assertIn("dev_feedback_render --ignored --exact", developer)

    def test_preinstalled_rust_probe_never_downloads_a_toolchain(self):
        landing = LANDING.read_text(encoding="utf-8")
        linux = landing[landing.index("  linux:") : landing.index("  windows-smoke:")]
        caller = next(
            step for step in STEP.findall(linux) if "id: preinstalled-rust" in step
        )
        probe = VERIFIED_CONTENT.read_text(encoding="utf-8")

        self.assertIn("uses: ./.github/actions/verified-content", caller)
        self.assertIn("probe-rust: 'true'", caller)
        self.assertIn("RUSTUP_AUTO_INSTALL: '0'", probe)
        self.assertNotIn("rustup toolchain install", probe)
        self.assertNotIn("rustup update", probe)

    def test_linux_preparation_overlaps_independent_setup_and_fails_closed(self):
        landing = LANDING.read_text(encoding="utf-8")
        linux = landing[landing.index("  linux:") : landing.index("  windows-smoke:")]
        caller = next(
            step for step in STEP.findall(linux) if "id: preinstalled-rust" in step
        )
        action = VERIFIED_CONTENT.read_text(encoding="utf-8")
        preparation = next(
            step for step in STEP.findall(action) if "id: materialize" in step
        )
        helper = CONTENT_HELPER.read_text(encoding="utf-8")

        self.assertIn("uses: ./.github/actions/verified-content", caller)
        self.assertIn("apt-packages: ${{ matrix.apt }}", caller)
        self.assertIn("probe-rust: 'true'", caller)
        self.assertIn("APT_PACKAGES: ${{ inputs.apt-packages }}", preparation)
        self.assertIn("PROBE_RUST: ${{ inputs.probe-rust }}", preparation)
        self.assertIn(
            'python3 scripts/ci-content.py --revision "$CONTENT_REVISION"',
            preparation,
        )
        self.assertIn("scripts/install-apt-packages.sh", preparation)
        self.assertIn("content_pid=$!", preparation)
        self.assertIn("apt_pid=$!", preparation)
        self.assertIn('wait "$content_pid" || failed=1', preparation)
        self.assertIn('wait "$apt_pid" || failed=1', preparation)
        self.assertLess(
            preparation.index("apt_pid=$!"), preparation.index('wait "$content_pid"')
        )
        # Git errors must fail validation, rather than become empty status text.
        self.assertIn("if completed.returncode:", helper)
        self.assertIn("raise ContentError(", helper)
        self.assertIn(
            'if git(content, "status", "--porcelain=v1", "--untracked-files=all"):',
            helper,
        )
        self.assertNotIn('[[ -z "$(git -C content status', preparation)
        self.assertIn("failed=0", preparation)
        self.assertIn('exit "$failed"', preparation)
        self.assertNotIn("continue-on-error", preparation)
        self.assertNotIn("continue-on-error", caller)
        self.assertNotIn("      - name: Install native dependencies", linux)
        self.assertNotIn("      - name: Verify pinned content checkout", linux)

    def test_linux_restores_the_producer_archive_without_naming_its_key(self):
        landing = LANDING.read_text(encoding="utf-8")
        linux = landing[landing.index("  linux:") : landing.index("  windows-smoke:")]
        cache = next(step for step in cache_steps(linux) if "shared-key: full-parity" in step)

        # Restore-only is the constraint: merge-queue refs must not spend the
        # repository's 10 GiB quota on copies no later candidate can restore.
        self.assertIn("Swatinem/rust-cache@", cache)
        self.assertIn("shared-key: full-parity", cache)
        self.assertIn("save-if: false", cache)
        self.assertIn("cache-targets: false", cache)
        restores = workspace_cache_steps(linux, "restore")
        self.assertEqual(len(restores), 1)
        self.assertIn("lane: landing-linux", restores[0])
        self.assertEqual(workspace_cache_steps(linux, "save"), [])

    def test_workspace_artifact_reuse_always_validates_its_persisted_ledger(self):
        expected = {
            ("landing.yml", "pull-request-quality"): ("target", "landing-v1", "landing.json", "landing-linux"),
            ("landing.yml", "linux"): ("target", "landing-v1", "landing.json", "landing-linux"),
            ("rust.yml", "linux-landing-cache"): ("target", "landing-v1", "landing.json", "landing-linux"),
            ("release-prebuild.yml", "tool"): ("target", "landing-v1", "landing.json", "landing-linux"),
            ("release-prebuild.yml", "runtime"): ("target", "landing-v1", "landing.json", "landing-linux"),
            ("exact-sha-qualification.yml", "coverage-fragments"): (
                "target/coverage-build", "coverage-v1", "coverage.json", "coverage-linux"
            ),
            ("exact-sha-qualification.yml", "coverage-cache-warm"): (
                "target/coverage-build", "coverage-v1", "coverage.json", "coverage-linux"
            ),
            ("exact-sha-qualification.yml", "developer-feedback"): (
                "target", "landing-v1", "landing.json", "landing-linux"
            ),
        }
        publishers = {
            ("rust.yml", "linux-landing-cache"),
            ("exact-sha-qualification.yml", "coverage-fragments"),
            ("exact-sha-qualification.yml", "coverage-cache-warm"),
        }
        compile_commands = {
            ("landing.yml", "pull-request-quality"): (
                "cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings"
            ),
            ("landing.yml", "linux"): 'bash -euo pipefail -c "$SHARD_COMMAND"',
            ("rust.yml", "linux-landing-cache"): (
                "cargo nextest run --workspace --features xtask/engine-tools --no-run --locked"
            ),
            ("release-prebuild.yml", "tool"): (
                "cargo build --profile test --locked -p xtask --features engine-tools --bin xtask-engine-tools"
            ),
            ("release-prebuild.yml", "runtime"): (
                "cargo build --release --locked -p clonk-app -p clonk-game -p clonk-c4group"
            ),
            ("exact-sha-qualification.yml", "coverage-fragments"): 'bash -euo pipefail -c "$SHARD_COMMAND"',
            ("exact-sha-qualification.yml", "coverage-cache-warm"): (
                "cargo nextest run --no-run -p clonk-app --features app-test-shard-1,app-test-shard-10 --locked"
            ),
            ("exact-sha-qualification.yml", "developer-feedback"): (
                "dev_feedback_replay::real_scenario_replays_repeat_with_native_group_order"
            ),
        }

        def assert_guard(step, allowed=None):
            guards = re.findall(r"(?m)^        if: (.+)$", step)
            self.assertEqual(guards, [] if allowed is None else [allowed])
            self.assertNotIn("continue-on-error", step)

        seen = set()
        published = set()
        for path in sorted(WORKFLOWS.glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            for name, job in ci_jobs(path, workflow):
                identity = (path.name, name)
                self.assertNotIn("cache-workspace-crates: true", job)
                publishes = workspace_cache_steps(job, "save")
                if publishes:
                    self.assertIn(identity, publishers)
                    self.assertEqual(len(publishes), 1)
                    published.add(identity)
                restores = workspace_cache_steps(job, "restore")
                self.assertLessEqual(len(restores), 1)
                for restore in restores:
                    seen.add(identity)
                    self.assertIn(identity, expected)
                    target, recipe, filename, lane = expected[identity]
                    with self.subTest(workflow=path.name, job=name):
                        linux_only = path.name == "release-prebuild.yml"
                        platform_guard = "matrix.name == 'linux'" if linux_only else None
                        registry_scope = "coverage-registry" if recipe == "coverage-v1" else "full-parity"
                        registry = next(
                            step for step in cache_steps(job)
                            if "Swatinem/rust-cache@" in step
                            and f"shared-key: {registry_scope}\n" in step
                        )
                        self.assertIn("cache-targets: false", registry)
                        assert_guard(registry, platform_guard)
                        assert_guard(restore, platform_guard)
                        prepare = next(
                            step for step in STEP.findall(job)
                            if "scripts/ci-workspace-cache.py prepare " in step
                        )
                        assert_guard(prepare, platform_guard)
                        compile_graph = next(
                            step for step in STEP.findall(job)
                            if compile_commands[identity] in step
                        )
                        assert_guard(compile_graph, platform_guard if name == "runtime" and linux_only else None)
                        self.assertNotIn("cache-hit", compile_graph)
                        self.assertLess(job.index(restore), job.index(prepare))
                        self.assertLess(job.index(prepare), job.index(compile_graph))
                        commands = [prepare]
                        if identity not in publishers:
                            self.assertEqual(publishes, [])
                            self.assertIn("save-if: false", registry)
                            self.assertNotIn("scripts/ci-workspace-cache.py record ", job)
                        else:
                            self.assertEqual(len(publishes), 1)
                            record = next(
                                step for step in STEP.findall(job)
                                if "scripts/ci-workspace-cache.py record " in step
                            )
                            commands.append(record)
                            assert_guard(record)
                            publish_guard = (
                                "inputs.upload-diagnostics && matrix.artifact == 'app-1-10'"
                                if name == "coverage-fragments" else None
                            )
                            assert_guard(publishes[0], publish_guard)
                            self.assertLess(job.index(compile_graph), job.index(record))
                            self.assertLess(job.index(record), job.index(publishes[0]))
                        for phase in [restore] + publishes:
                            self.assertIn(f"target: {target}\n", phase)
                            self.assertIn(f"ledger: .ci-cache-ledgers/{filename}\n", phase)
                            self.assertIn(f"recipe: {recipe}\n", phase)
                            self.assertIn(f"lane: {lane}\n", phase)
                        for command in commands:
                            self.assertIn(f"--target {target}", command)
                            self.assertIn(f"--recipe {recipe}", command)
                            self.assertIn(f"--ledger .ci-cache-ledgers/{filename}", command)
                        if recipe == "coverage-v1":
                            self.assertLess(
                                job.index("- name: Resolve exact coverage build environment"),
                                job.index(restore),
                            )
                            self.assertIn("shared-key: coverage-registry", registry)
                            collect = next(
                                step for step in STEP.findall(job)
                                if "cargo llvm-cov clean --profraw-only" in step
                            )
                            self.assertLess(job.index(prepare), job.index(collect))
                            self.assertLess(job.index(collect), job.index(record))
                            if name == "coverage-fragments":
                                self.assertIn('bash -euo pipefail -c "$SHARD_COMMAND"', collect)
                                self.assertIn("cargo llvm-cov report --lcov", collect)
                            else:
                                self.assertIn(compile_commands[identity], collect)
                                self.assertNotIn("cargo llvm-cov report", job)
                                self.assertNotIn("lcov", job.lower())
                                header = job[:job.index("    steps:\n")]
                                for authority in (
                                    "needs: qualification-context",
                                    "needs.qualification-context.result == 'success'",
                                    "needs.qualification-context.outputs.run-id != ''",
                                    "inputs.source-sha == github.sha", "inputs.upload-diagnostics",
                                    "github.ref == 'refs/heads/main'", "!github.event.repository.fork",
                                    "github.event_name == 'push'", "github.event_name == 'workflow_dispatch'",
                                    "github.event_name == 'schedule'",
                                ):
                                    self.assertIn(authority, header)
        self.assertEqual(seen, set(expected))
        self.assertEqual(published, publishers)

        action = WORKSPACE_CACHE.read_text(encoding="utf-8")
        restore = next(step for step in STEP.findall(action) if "actions/cache/restore@" in step)
        save = next(step for step in STEP.findall(action) if "actions/cache/save@" in step)
        self.assertIn("continue-on-error: true", restore)
        self.assertIn("key: ${{ steps.identity.outputs.key }}", restore)
        self.assertIn("restore-keys: ${{ steps.identity.outputs.prefix }}", restore)
        self.assertIn("lookup-only: ${{ inputs.operation == 'lookup' }}", restore)
        self.assertIn("if: inputs.operation == 'save' && steps.identity.outputs.trusted-save == 'true'", save)
        self.assertIn("key: ${{ steps.identity.outputs.key }}", save)
        self.assertNotIn("restore-keys:", save)
        for step in (restore, save):
            self.assertIn("${{ steps.identity.outputs.target }}", step)
            self.assertIn("${{ steps.identity.outputs.ledger }}", step)
        self.assertIn('"key": prefix + tree', action)
        self.assertIn('git("rev-parse", "HEAD^{tree}")', action)
        self.assertIn("clonk-ci-target-v1-", action)

    def test_ci_nextest_profile_and_junit_sources_remain_consistent(self):
        for path in (LANDING, MAIN, QUALIFICATION):
            workflow = path.read_text(encoding="utf-8")
            environment = workflow[:workflow.index("jobs:\n")]
            with self.subTest(workflow=path.name):
                self.assertIn("  NEXTEST_PROFILE: ci\n", environment)
                self.assertNotIn("NEXTEST_PROFILE: default", workflow)
                self.assertNotIn("nextest/default/", workflow)
        for path, junit in (
            (LANDING, "target/nextest/ci/junit.xml"),
            (QUALIFICATION, "target/nextest/ci/junit.xml"),
        ):
            workflow = path.read_text(encoding="utf-8")
            with self.subTest(workflow=path.name):
                self.assertIn(f"JUNIT_SOURCE: {junit}\n", workflow)
                self.assertIn('"nextest_profile": "ci",', workflow)
                self.assertIn('Path(os.environ["JUNIT_SOURCE"]).unlink(missing_ok=True)', workflow)
                self.assertIn("LC_NEXTEST_JUNIT_SOURCE: ${{ env.JUNIT_SOURCE }}", workflow)
                self.assertIn("${{ env.JUNIT_SOURCE }}", workflow)

    def test_no_landing_row_names_a_rust_cache_key_by_hand(self):
        landing = LANDING.read_text(encoding="utf-8")
        code = "\n".join(
            line for line in landing.splitlines() if not line.lstrip().startswith("#")
        )

        # A literal key hash cannot be kept correct by asking the next reader to
        # keep it aligned. The prefix that used to sit here asked for a hash the
        # producer stopped writing on 2026-08-31, and every Linux row restored a
        # six-day-stale archive that reported a clean hit and rebuilt anyway.
        self.assertNotIn("v0-rust-", code)
        self.assertNotRegex(code, r"(?m)^\s*restore-keys:")

    def test_merge_group_rows_use_run_scoped_lanes(self):
        landing = LANDING.read_text(encoding="utf-8")
        main = MAIN.read_text(encoding="utf-8")

        cache_lanes = (
            "linux-landing-cache-rolling",
            "windows-landing-cache-rolling",
        )
        for group in cache_lanes:
            with self.subTest(group=group):
                self.assertNotIn(group, landing)
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
        windows_job = landing[
            landing.index("  windows-smoke:") : landing.index("  landing-gate:")
        ]
        for job, platform in ((linux_job, "linux"), (windows_job, "windows")):
            with self.subTest(platform=platform):
                self.assertIn(
                    "group: ${{ format('landing-"
                    + platform
                    + "-{0}-{1}', github.run_id, matrix.name) }}",
                    job,
                )
                self.assertNotRegex(job, r"\b[a-z0-9-]+-rolling\b")
                self.assertIn(
                    "cancel-in-progress: ${{ github.event_name == 'merge_group' }}",
                    job,
                )

    def test_normal_workspace_is_an_exhaustive_compile_time_partition(self):
        workflow = LANDING.read_text(encoding="utf-8")

        app_commands = re.findall(
            r"cargo nextest run -p clonk-app --features ([a-z0-9,-]+)"
            r" --no-fail-fast --locked",
            workflow,
        )
        self.assertEqual(
            Counter(tuple(command.split(",")) for command in app_commands),
            Counter([
                ("app-test-shard-1",),
                ("app-test-shard-12",),
                ("app-test-shard-3", "app-test-shard-10"),
                ("app-test-shard-2", "app-test-shard-7"),
                ("app-test-shard-4", "app-test-shard-9"),
                ("app-test-shard-5",),
                ("app-test-shard-11",),
                ("app-test-shard-6",),
                ("app-test-shard-8",),
            ]),
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
            Counter(engine_commands),
            Counter(["engine-it-shard-1", "engine-it-shard-2", "engine-it-shard-3"]),
        )
        for shard in (1, 2, 3):
            entry = matrix_entry(workflow, f"engine integration {shard}/3")
            self.assertIn(
                "cargo nextest run -p clonk-engine-integration-tests "
                f"--test engine_it --features engine-it-shard-{shard} "
                "--no-fail-fast --locked",
                entry,
            )
        engine_unit_and_parity = matrix_entry(
            workflow, "engine and frontend unit and parity"
        )
        self.assertIn(
            "cargo nextest run -p clonk-engine-unit-tests "
            "-p clonk-frontend-unit-tests "
            "--no-fail-fast --locked",
            engine_unit_and_parity,
        )
        parity_filter = "test(/(^|::)parity_differential_matches_cpp_golden$/)"
        self.assertNotIn("cargo xtask parity verify", engine_unit_and_parity)
        self.assertEqual(engine_unit_and_parity.count(parity_filter), 2)
        for package in ("clonk-engine-unit-tests", "clonk-frontend-unit-tests"):
            self.assertIn(
                f"package({package}) and {parity_filter}",
                engine_unit_and_parity,
            )
        self.assertEqual(
            engine_unit_and_parity.count("--no-tests=fail"),
            3,  # two parity comparators and the resources ABI tests
        )
        self.assertNotIn("          - name: frontend unit\n", workflow)
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
            Counter((index, total) for index, total, _ in remaining_shards),
            Counter([("1", "2"), ("2", "2")]),
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
        by_index = {index: arguments for index, total, arguments in remaining_shards}
        self.assertIn("-p clonk-app-netplay", by_index["1"])
        self.assertNotIn("-p clonk-app-netplay", by_index["2"])
        self.assertNotIn("-p clonk-app-render", by_index["1"])
        self.assertIn("-p clonk-app-render", by_index["2"])
        self.assertIn("-p clonk-network", by_index["1"])
        self.assertNotIn("-p clonk-network", by_index["2"])
        self.assertNotIn("-p clonk-app-menus", by_index["1"])
        self.assertIn("-p clonk-app-menus", by_index["2"])
        self.assertNotIn(
            "cargo nextest run --workspace --no-fail-fast --locked",
            workflow,
        )

    def test_overlapping_linux_checks_share_setup_without_failing_open(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]
        unit_and_parity = matrix_entry(
            workflow, "engine and frontend unit and parity"
        )
        quality = matrix_entry(workflow, "workspace quality")

        for entry in (unit_and_parity, quality):
            self.assertIn("failed=0", entry)
            self.assertIn('exit "$failed"', entry)
        self.assertNotIn("cargo xtask parity verify", unit_and_parity)
        self.assertEqual(
            unit_and_parity.count("--no-tests=fail"),
            3,  # two parity comparators and the resources ABI tests
        )
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

    def test_oracle_verifiers_fetch_only_the_pinned_oracle_history(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]
        fetch = (
            "git fetch --no-tags --depth=1 origin "
            "7d43b47b7d789b533f32d005e64596e0a07019cd"
        )

        for name in (
            "workspace quality",
            "presentation captures",
            "engine contracts",
        ):
            with self.subTest(name=name):
                self.assertIn("oracle: true", matrix_entry(workflow, name))
        self.assertEqual(linux.count("oracle: true"), 3)
        self.assertIn("if: matrix.oracle == true", linux)
        self.assertIn(fetch, linux)
        self.assertLess(
            linux.index("- name: Fetch pinned LegacyClonk oracle commit"),
            linux.index("- name: Run ${{ matrix.name }}"),
        )

    def test_linux_setup_is_pinned_fast_and_matrix_scoped(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]

        self.assertIn("runs-on: ubuntu-24.04", linux)
        self.assertNotIn("filter: blob:none", linux)
        app_rows = [
            "app 1/12",
            "app 12/12",
            "app 3+10/12",
            "app 2+7/12",
            "app 4+9/12",
            "app 5/12",
            "app 11/12",
            "app 6/12",
            "app 8/12",
        ]
        for name in app_rows:
            self.assertIn(
                "apt: libasound2-dev libudev-dev",
                matrix_entry(workflow, name),
            )
        expected_apt = {
            "engine and frontend unit and parity": "libasound2-dev libudev-dev",
            "remaining workspace 2/2": "libasound2-dev libxmp4 mesa-vulkan-drivers",
            "app 4+9/12": "libasound2-dev libudev-dev mesa-vulkan-drivers",
            "workspace quality": "libasound2-dev libudev-dev python3-pil",
            "presentation captures": "libasound2-dev libudev-dev",
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
            "engine contracts",
        ):
            self.assertNotIn("\n            apt:", matrix_entry(workflow, name))

        # Start the measured longest rows first under the release cap, with
        # capture acquisition ahead of the rest. Membership and multiplicity
        # remain independently pinned by the compile-time partition guards.
        self.assertEqual(
            re.findall(r"(?m)^          - name: (.+)$", linux),
            [
                "presentation captures", "app 4+9/12", "app 5/12", "app 3+10/12",
                "app 2+7/12", "app 6/12", "remaining workspace 2/2", "app 1/12",
                "workspace quality", "engine contracts", "app 12/12", "engine integration 1/3",
                "engine integration 3/3", "engine and frontend unit and parity", "app 11/12",
                "app 8/12", "remaining workspace 1/2", "engine integration 2/3",
            ],
        )
        self.assertIn(
            "max-parallel: ${{ needs.release-context.outputs.release == 'true' && 5 || 17 }}",
            linux,
        )

        action = VERIFIED_CONTENT.read_text(encoding="utf-8")
        self.assertIn("apt-packages: ${{ matrix.apt }}", linux)
        self.assertIn("probe-rust: 'true'", linux)
        self.assertIn('if [[ -n "$APT_PACKAGES" ]]', action)
        self.assertIn("scripts/install-apt-packages.sh", action)
        self.assertIn("timeout-minutes: 10", action)
        self.assertIn("rustc 1.98.1", action)
        self.assertIn("id: preinstalled-rust", linux)
        self.assertIn("if: steps.preinstalled-rust.outputs.exact != 'true'", linux)

    def test_presentation_captures_are_a_bounded_fail_closed_landing_row(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]
        capture = matrix_entry(workflow, "presentation captures")

        self.assertIn("timeout-minutes: ${{ matrix.timeout || 15 }}", linux)
        self.assertIn("timeout: 30", capture)
        self.assertIn(
            'cargo xtask presentation verify-current --profile release --output-dir "$RUNNER_TEMP/presentation-current"',
            capture,
        )
        upload = linux[linux.index("- name: Upload presentation capture failure") :]
        self.assertIn("if: failure() && matrix.name == 'presentation captures'", upload)
        self.assertIn(
            "uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1",
            upload,
        )
        for path in (
            "${{ runner.temp }}/presentation-current/run-*/rust/artifacts",
            "${{ runner.temp }}/presentation-current/run-*/rust/receipts",
            "${{ runner.temp }}/presentation-current/verify-current.json",
        ):
            with self.subTest(path=path):
                self.assertIn(path, upload)
        self.assertIn("if-no-files-found: warn", upload)
        self.assertIn("retention-days: 14", upload)
        self.assertNotIn("continue-on-error", upload)

    def test_presentation_command_uses_the_lightweight_xtask_dispatcher(self):
        dispatcher = (REPOSITORY / "xtask" / "src" / "dispatcher.rs").read_text(
            encoding="utf-8"
        )
        engine_xtask = (REPOSITORY / "xtask" / "src" / "main.rs").read_text(
            encoding="utf-8"
        )

        lightweight = (
            'Some("presentation") => return xtask::presentation::command(&args[1..]),'
        )
        self.assertIn(lightweight, dispatcher)
        self.assertLess(
            dispatcher.index(lightweight),
            dispatcher.index("Command::new(cargo)"),
        )
        self.assertIn('Some("presentation") => {', engine_xtask)
        self.assertIn("xtask::presentation::command(&tail)", engine_xtask)
        self.assertIn(
            "cargo xtask presentation verify-current --profile <p> --output-dir <dir>",
            engine_xtask,
        )

    def test_hosted_toolchains_and_cached_registry_are_reused_safely(self):
        workflow = LANDING.read_text(encoding="utf-8")
        linux = workflow[workflow.index("  linux:") : workflow.index("  windows-smoke:")]
        windows_smoke = workflow[
            workflow.index("  windows-smoke:") : workflow.index("  landing-gate:")
        ]
        producer = workflow_job(MAIN.read_text(encoding="utf-8"), "linux-landing-cache")

        probe = VERIFIED_CONTENT.read_text(encoding="utf-8")
        # The cache ledger keys tool selection by PATH. Both ends must use
        # the same hosted-toolchain probe and its sysroot/bin path setup.
        for job in (linux, producer):
            self.assertIn("uses: ./.github/actions/verified-content", job)
            self.assertIn("id: preinstalled-rust", job)
            self.assertIn("probe-rust: 'true'", job)
            self.assertIn("if: steps.preinstalled-rust.outputs.exact != 'true'", job)
            self.assertIn("tool: cargo-nextest@0.9.91", job)
        self.assertIn("rustup toolchain list", probe)
        self.assertIn('rustup run "$toolchain" rustc --version', probe)
        self.assertIn('rustup run "$toolchain" rustc --print sysroot', probe)
        self.assertIn('echo "$sysroot/bin" >> "$GITHUB_PATH"', probe)
        self.assertIn('CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}', probe)
        self.assertNotIn('RUSTUP_TOOLCHAIN=$toolchain', probe)

        pinned_toolchain = (
            "uses: dtolnay/rust-toolchain@"
            "ce678459e9fc7500d337468f904b95f1b5c10b5e"
        )
        for job in (linux, producer):
            self.assertIn(pinned_toolchain, job)
        for job in (windows_smoke,):
            self.assertIn(pinned_toolchain, job)
            self.assertNotIn("id: preinstalled-rust", job)
            self.assertNotIn("CARGO_HOME=", job)

        self.assertNotIn("cargo build --release -p clonk-app", workflow)
        self.assertNotIn("scripts/configure-msvc-runtime.sh", workflow)

    def test_windows_smoke_packs_quality_with_the_shorter_runtime_pole(self):
        workflow = LANDING.read_text(encoding="utf-8")
        windows = workflow[
            workflow.index("  windows-smoke:") : workflow.index("  landing-gate:")
        ]
        rows = set(re.findall(r"(?m)^          - name: (.+)$", windows))

        self.assertEqual(rows, {"runtime and quality", "network tests"})
        self.assertIn("name: Windows / ${{ matrix.name }}", windows)
        self.assertIn("if: matrix.nextest", windows)
        installers = [step for step in STEP.findall(windows) if "if: matrix.installer" in step]
        self.assertEqual(len(installers), 3)
        for name in (
            "Install NSIS", "Compile the installer over a stand-in payload",
            "Validate native Windows content timeout boundaries",
        ):
            self.assertTrue(any(f"- name: {name}\n" in step for step in installers))
        self.assertIn("python -m unittest test_ci_content.NativeWindowsBoundsTests", windows)

        runtime = matrix_entry(workflow, "runtime and quality")
        self.assertIn("-p clonk-game -p clonk-c4group", runtime)
        self.assertIn("-p clonk-logging --test crash_log_descriptor", runtime)
        self.assertNotIn("cargo nextest run -p clonk-network", runtime)
        self.assertIn("cargo clippy --profile test --no-deps", runtime)
        self.assertIn("nextest: true", runtime)
        self.assertIn("installer: true", runtime)
        self.assertIn("failed=0", runtime)
        self.assertIn('exit "$failed"', runtime)

        network = matrix_entry(workflow, "network tests")
        self.assertIn("cargo nextest run -p clonk-network --lib", network)
        self.assertIn("discovery_multicast_target_uses_cpp_default_interface", network)
        self.assertIn("a_windows_port_unreachable_closes_only_the_refusing_peer", network)
        self.assertNotIn("cargo clippy", network)
        self.assertIn("nextest: true", network)
        self.assertIn("installer: false", network)

    def test_literal_required_commands_remain_on_the_landing_tree(self):
        workflow = LANDING.read_text(encoding="utf-8")
        required = (
            "cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings",
            "package(clonk-engine-unit-tests) and test(/(^|::)parity_differential_matches_cpp_golden$/)",
            "cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked",
            "cargo xtask engine-snapshots verify",
            "cargo xtask compat verify",
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
        landing_concurrency = landing[
            landing.index("concurrency:\n") : landing.index("env:\n")
        ]
        admission = workflow_job(main, "diagnostic-admission")
        main_caller = main[
            main.index("  exact-sha-qualification:") : main.index(
                "  windows-landing-cache:"
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
        self.assertIn("github.event_name == 'merge_group'", landing_concurrency)
        self.assertIn("landing-runner-priority", landing_concurrency)
        self.assertIn("cancel-in-progress: true", landing_concurrency)
        self.assertIn("needs: linux-landing-cache", admission)
        self.assertIn("actions: read", admission)
        self.assertNotIn("actions: write", admission)
        self.assertIn("actions/workflows/landing.yml/runs", admission)
        self.assertIn("event=merge_group", admission)
        self.assertIn('.status != "completed"', admission)
        self.assertIn("run_diagnostics", admission)
        self.assertIn("needs: [diagnostic-admission, qualification-reuse]", main_caller)
        self.assertIn(
            "needs.diagnostic-admission.outputs.run_diagnostics == 'true'",
            main_caller,
        )
        self.assertIn("group: landing-runner-priority", main_caller)
        self.assertIn("cancel-in-progress: false", main_caller)
        self.assertIn("concurrency-suffix: rolling", main_caller)
        self.assertIn("concurrency-suffix: ${{ github.sha }}", release_caller)
        self.assertIn("cancel-in-progress: true", qualification)

    def test_diagnostic_admission_fails_closed_around_merge_group_activity(self):
        main = MAIN.read_text(encoding="utf-8")
        admission = workflow_job(main, "diagnostic-admission")
        script = textwrap.dedent(admission.split("        run: |\n", 1)[1])

        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            fake_gh = temporary / "gh"
            fake_gh.write_text(
                "#!/bin/sh\n"
                "if [ \"${FAKE_GH_FAILURE:-0}\" = 1 ]; then exit 42; fi\n"
                "printf '%s' \"${FAKE_GH_OUTPUT:-}\"\n",
                encoding="utf-8",
            )
            fake_gh.chmod(0o755)

            def admit(fake_output="", *, fail=False):
                output = temporary / "github-output"
                output.write_text("", encoding="utf-8")
                environment = os.environ.copy()
                environment.update(
                    {
                        "PATH": f"{temporary}:{environment['PATH']}",
                        "GH_TOKEN": "test-token",
                        "REPOSITORY": "clonk-org/clonk-rs",
                        "GITHUB_OUTPUT": str(output),
                        "FAKE_GH_OUTPUT": fake_output,
                        "FAKE_GH_FAILURE": "1" if fail else "0",
                    }
                )
                completed = subprocess.run(
                    ["bash", "-euo", "pipefail", "-c", script],
                    check=False,
                    capture_output=True,
                    text=True,
                    env=environment,
                )
                return completed, output.read_text(encoding="utf-8")

            idle, idle_output = admit()
            active, active_output = admit("32409353928")
            failed, failed_output = admit(fail=True)

        self.assertEqual(idle.returncode, 0)
        self.assertEqual(idle_output, "run_diagnostics=true\n")
        self.assertEqual(active.returncode, 0)
        self.assertEqual(active_output, "run_diagnostics=false\n")
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(failed_output, "")

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
