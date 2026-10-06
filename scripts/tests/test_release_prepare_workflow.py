"""Guards for the GitHub-native release preparation flow."""

import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

from _repo import REPOSITORY

PREPARE = REPOSITORY / ".github" / "workflows" / "release-prepare.yml"
PUBLISH = REPOSITORY / ".github" / "workflows" / "release.yml"
PREPARE_SCRIPT = REPOSITORY / "scripts" / "prepare-release.sh"


def digest_of(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare_step_option(name, key):
    """Read a step option without matching nested environment or shell text."""
    lines = PREPARE.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(f"{PREPARE.name} has no step named {name!r}") from None
    for line in lines[start + 1 :]:
        if line.startswith("      - "):
            break
        if line.startswith(f"        {key}: "):
            return line.split(": ", 1)[1]
    return None


def prepare_step_env(name):
    """Return a named release-prepare step's literal `env:` values.

    Only literals: a step reading `${{ ... }}` into the environment has no
    fixed value to hand a local run.
    """
    lines = PREPARE.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(f"{PREPARE.name} has no step named {name!r}") from None

    values = {}
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line.startswith("      - "):
            break
        if line == "        env:":
            for candidate in lines[index + 1 :]:
                if not candidate.startswith(" " * 10):
                    break
                key, _, value = candidate.strip().partition(": ")
                values[key] = value.strip().strip("'\"")
            break
    return values


def prepare_script_tool_probe():
    """Return prepare-release.sh's own git-cliff lookup, as shell.

    Evaluating the script's real assignments couples this test to the script:
    a version bump or a relocated tool root fails here instead of silently
    reintroducing the source build the workflow exists to avoid.
    """
    wanted = ("tool_version=", "tool_root=", "tool=")
    body = [
        line
        for line in PREPARE_SCRIPT.read_text(encoding="utf-8").splitlines()
        if line.startswith(wanted)
    ]
    missing = [
        name for name in wanted if not any(line.startswith(name) for line in body)
    ]
    if missing:
        raise AssertionError(f"prepare-release.sh no longer assigns {missing}")
    return "\n".join(body)


def prepare_step_script(name):
    """Return a named release-prepare step's real shell body."""
    lines = PREPARE.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(f"{PREPARE.name} has no step named {name!r}") from None

    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line.startswith("      - "):
            break
        if line == "        run: |":
            body = []
            for candidate in lines[index + 1 :]:
                if candidate.strip() and not candidate.startswith(" " * 10):
                    break
                body.append(candidate[10:])
            return "\n".join(body)
    raise AssertionError(f"step {name!r} has no `run: |` block")


class ReleasePrepareWorkflowTests(unittest.TestCase):
    def test_inferred_version_scans_only_unreleased_commits(self):
        script = PREPARE_SCRIPT.read_text(encoding="utf-8")

        self.assertIn(
            'version=$("$tool" --config "$repo_root/cliff.toml" '
            "--unreleased --bumped-version 2>/dev/null)",
            script,
        )

    def test_git_cliff_availability_does_not_depend_on_a_cache(self):
        # The compiled binary used to be cached, which made preparation only
        # as reliable as an entry this repository cannot keep: it sits against
        # the 10 GiB Actions cache cap, and a once-a-day entry is what gets
        # evicted first. Restoring nothing meant a 2m36s source build, a blown
        # SLO, and a failed job -- and `actions/cache` does not save from a
        # failed job, so the rebuild repeated on every run after it.
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertNotIn("actions/cache@", workflow)
        self.assertNotIn("git-cliff-${{ runner.os }}", workflow)
        self.assertIn("name: Install git-cliff", workflow)
        self.assertLess(
            workflow.index("name: Install git-cliff"),
            workflow.index("name: Prepare the release"),
        )

    def test_release_preparation_fails_its_two_minute_slo_closed(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertIn("actions: read", workflow)
        self.assertIn("id: pull-request", workflow)
        self.assertIn('echo "url=$pr" >> "$GITHUB_OUTPUT"', workflow)
        self.assertIn("name: Enforce release preparation SLO", workflow)
        self.assertIn("actions/runs/${GITHUB_RUN_ID}", workflow)
        self.assertIn('echo "url=$pr" >> "$GITHUB_OUTPUT"', workflow)
        self.assertIn("name: Ensure the release pull request is queued", workflow)
        self.assertIn("actions/runs/${{ github.run_id }}", workflow)
        self.assertIn(
            "steps.pull-request.outputs.url != '' || steps.existing.outputs.url != ''",
            workflow,
        )
        self.assertIn(
            "steps.pull-request.outputs.url || steps.existing.outputs.url", workflow
        )
        self.assertIn('if [[ "$elapsed" -gt 120 ]]', workflow)

    def test_release_preparation_uses_the_app_owned_pull_request(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertIn("actions/create-github-app-token@", workflow)
        self.assertIn("client-id: ${{ vars.RELEASE_APP_CLIENT_ID }}", workflow)
        self.assertIn("private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}", workflow)
        self.assertIn("token: ${{ steps.release-app.outputs.token }}", workflow)
        self.assertIn('branch="release/next"', workflow)
        self.assertIn('ref="refs/heads/${branch}"', workflow)
        self.assertIn("--force-with-lease=", workflow)
        self.assertIn("gh pr create", workflow)
        self.assertIn("gh pr merge", workflow)
        self.assertIn("--auto --squash", workflow)

    def test_branch_seeding_survives_a_ref_left_by_an_earlier_run(self):
        # A failure between seeding `release/next` and opening the pull request
        # leaves the branch behind: on 2026-07-31 a GitHub 504 did exactly
        # that. A ruleset forbids deleting the branch, so a create-only seed
        # then fails every later run with "Reference already exists" — the
        # daily schedule included. Seeding must reset an existing ref to the
        # base commit so the force-with-lease push still holds its lease.
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertIn(
            'gh api --method PATCH "repos/${REPOSITORY}/git/refs/heads/${branch}"',
            workflow,
        )
        self.assertIn("-F force=true", workflow)

    def test_schedule_prepares_a_pr_and_publication_can_recover_a_landed_sha(self):
        prepare = PREPARE.read_text(encoding="utf-8")
        publish = PUBLISH.read_text(encoding="utf-8")

        self.assertIn("schedule:", prepare)
        self.assertNotIn("schedule:", publish)
        self.assertIn("workflow_dispatch:", publish)
        self.assertIn("release-sha:", publish)
        self.assertIn("required: true", publish)

    def test_publish_resolver_has_no_legacy_preparation_ancestor(self):
        workflow = PUBLISH.read_text(encoding="utf-8")
        publish = workflow.split("\n  publish:\n", 1)[1]

        self.assertNotIn("\n  prepare:\n", workflow)
        self.assertNotIn("needs: [prepare]", publish)
        self.assertNotIn("needs.prepare", publish)
        self.assertIn("ref: ${{ inputs.release-sha || github.sha }}", publish)
        self.assertIn("RESOLVED_SHA: ${{ inputs.release-sha || github.sha }}", publish)

    def test_app_created_pr_runs_the_repository_checks(self):
        workflow = PREPARE.read_text(encoding="utf-8")

        self.assertNotIn("A pull request opened with GITHUB_TOKEN", workflow)
        self.assertNotIn("cargo check --workspace --locked", workflow)
        self.assertNotIn(
            "cargo test -p xtask --features engine-tools", workflow
        )

    def test_preparation_latency_report_is_advisory_and_skips_empty_main_pushes(self):
        workflow = PREPARE.read_text(encoding="utf-8")
        self.assertIn("\n  prepare-latency-report:\n", workflow)
        report = workflow.split("\n  prepare-latency-report:\n", 1)[1]
        self.assertIn("needs: [prepare]", report)
        self.assertIn("continue-on-error: true", report)
        self.assertIn("needs.prepare.outputs.release-candidate == 'true'", report)
        self.assertIn("actions: read", report)
        self.assertIn("pull-requests: read", report)
        self.assertNotIn("contents: write", report)
        self.assertNotIn("steps.release-app", report)
        self.assertIn("--phase prepare --allow-running true", report)
        self.assertIn("--history-limit 20", report)
        self.assertIn('GH_TOKEN: ${{ github.token }}', report)
        self.assertIn("if-no-files-found: warn", report)

    def test_source_ownership_creation_refresh_and_queue_checks_remain_required(self):
        for name in ("Check for an in-flight release", "Validate existing release candidate",
                     "Prepare the release", "Refresh existing release pull request",
                     "Open the pull request", "Ensure the release pull request is queued"):
            with self.subTest(step=name):
                self.assertIn(prepare_step_option(name, "continue-on-error"), (None, "false"))
        prepare = PREPARE.read_text(encoding="utf-8").split("\n  prepare:\n", 1)[1]
        prepare = prepare.split("\n  prepare-latency-report:\n", 1)[0]
        self.assertNotRegex(prepare, r"(?m)^    continue-on-error: (?!false$)")


@unittest.skipUnless(shutil.which("bash"), "needs bash")
class ReleasePreparationSloTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self._stub(
            "gh",
            'if [[ "$1" == "api" ]]; then echo "$RUN_CREATED_AT"; exit 0; fi\n'
            'if [[ "$1 $2" == "pr view" ]]; then echo "$PR_CREATED_AT"; exit 0; fi\n'
            "exit 1\n",
        )
        self._stub(
            "date",
            '[[ "$1 $2" == "-u -d" ]] || exit 1\n'
            'case "$3" in\n'
            '  run-created) echo "$RUN_EPOCH" ;;\n'
            '  pr-created) echo "$PR_EPOCH" ;;\n'
            '  *) exit 1 ;;\n'
            'esac\n',
        )

    def _stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def run_slo(self, elapsed):
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "ACTIONS_TOKEN": "actions-token",
            "GH_TOKEN": "app-token",
            "GITHUB_RUN_ID": "123",
            "PR_URL": "https://github.com/clonk-org/clonk-rs/pull/999",
            "REPOSITORY": "clonk-org/clonk-rs",
            "RUN_CREATED_AT": "run-created",
            "PR_CREATED_AT": "pr-created",
            "RUN_EPOCH": "1000",
            "PR_EPOCH": str(1000 + elapsed),
        }
        return subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                prepare_step_script("Enforce release preparation SLO"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )

    def run_existing_check(self, pull_requests, event="workflow_dispatch"):
        output = self.root / "existing-output"
        self._stub("gh", 'printf "%s\\n" "$PULL_REQUESTS"\n')
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "GH_TOKEN": "app-token",
            "GITHUB_OUTPUT": str(output),
            "REPOSITORY": "clonk-org/clonk-rs",
            "RUN_URL": "https://github.com/clonk-org/clonk-rs/actions/runs/123",
            "PULL_REQUESTS": json.dumps(pull_requests),
            "GITHUB_EVENT_NAME": event,
        }
        completed = subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                prepare_step_script("Check for an in-flight release"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )
        return completed, output.read_text(encoding="utf-8") if output.exists() else ""

    def test_preparation_slo_accepts_its_exact_boundary(self):
        completed = self.run_slo(120)

        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_slow_preparation_keeps_functional_success_and_reports_latency_miss(self):
        completed = self.run_slo(121)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("::warning::", completed.stdout)
        self.assertIn("exceeded its 120s SLO", completed.stdout)

    def test_existing_pr_from_an_earlier_dispatch_is_not_a_new_latency_sample(self):
        completed = self.run_slo(-1)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("predates this workflow dispatch", completed.stdout)
        self.assertNotIn("SLO already satisfied", completed.stdout)
        self.assertNotIn("release pull request opened -", completed.stdout)

    def test_timestamp_api_failure_is_advisory_after_pr_creation(self):
        self._stub("gh", 'echo "timestamp API unavailable" >&2\nexit 37\n')

        completed = self.run_slo(0)

        self.assertEqual(completed.returncode, 37)
        self.assertIn("timestamp API unavailable", completed.stderr)
        self.assertNotIn("release pull request opened", completed.stdout)
        self.assertEqual(prepare_step_option("Enforce release preparation SLO", "continue-on-error"), "true")

    def test_invalid_timestamp_keeps_error_evidence_without_failing_release_preparation(self):
        self._stub("date", 'echo "invalid API timestamp" >&2\nexit 29\n')

        completed = self.run_slo(0)

        self.assertEqual(completed.returncode, 29)
        self.assertIn("invalid API timestamp", completed.stderr)
        self.assertNotIn("release pull request opened", completed.stdout)
        self.assertEqual(prepare_step_option("Enforce release preparation SLO", "continue-on-error"), "true")

    def test_rerun_recovers_its_already_merged_release_pr(self):
        completed, output = self.run_existing_check(
            [
                {
                    "url": "https://github.com/clonk-org/clonk-rs/pull/999",
                    "state": "MERGED",
                    "createdAt": "2026-08-09T01:00:00Z",
                    "body": (
                        "Prepared by workflow run "
                        "https://github.com/clonk-org/clonk-rs/actions/runs/123."
                    ),
                }
            ]
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("open=true", output)
        self.assertIn("state=MERGED", output)
        self.assertIn("pull/999", output)

    def test_rerun_refuses_to_replace_its_closed_release_pr(self):
        completed, _ = self.run_existing_check(
            [
                {
                    "url": "https://github.com/clonk-org/clonk-rs/pull/999",
                    "state": "CLOSED",
                    "createdAt": "2026-08-09T01:00:00Z",
                    "body": (
                        "Prepared by workflow run "
                        "https://github.com/clonk-org/clonk-rs/actions/runs/123."
                    ),
                }
            ]
        )

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("was closed", completed.stderr)

    def test_main_push_without_an_open_release_is_a_read_only_noop(self):
        completed, output = self.run_existing_check([], event="push")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("open=false", output)
        self.assertIn("process=false", output)
        self.assertIn("create=false", output)

    def test_schedule_and_dispatch_without_a_candidate_still_create_releases(self):
        for event in ("schedule", "workflow_dispatch"):
            with self.subTest(event=event):
                completed, output = self.run_existing_check([], event=event)

                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertIn("open=false", output)
                self.assertIn("process=true", output)
                self.assertIn("create=true", output)

    def test_run_id_prefix_does_not_select_a_different_runs_merged_release(self):
        completed, output = self.run_existing_check([
            {"url": "https://github.com/clonk-org/clonk-rs/pull/998", "state": "MERGED",
             "createdAt": "2026-08-09T01:00:00Z",
             "body": "Prepared by workflow run https://github.com/clonk-org/clonk-rs/actions/runs/1234."},
            {"url": "https://github.com/clonk-org/clonk-rs/pull/999", "state": "OPEN",
             "createdAt": "2026-08-10T01:00:00Z",
             "body": "Prepared by workflow run https://github.com/clonk-org/clonk-rs/actions/runs/456."},
        ])

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("state=OPEN", output)
        self.assertIn("pull/999", output)
        self.assertNotIn("pull/998", output)


@unittest.skipUnless(shutil.which("bash") and shutil.which("git") and shutil.which("jq"), "needs bash, Git and jq")
class ReleaseRefreshTests(unittest.TestCase):
    """Execute workflow shell against a real tiny origin and fake GitHub API."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="clonk-release-refresh-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo, self.origin, self.bin = self.root / "checkout", self.root / "origin.git", self.root / "bin"
        self.repo.mkdir()
        self.bin.mkdir()
        self.environment = {**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
        self.git("init", "--quiet", "--initial-branch=main")
        self.command(["git", "init", "--quiet", "--bare", str(self.origin)])
        self.identity("Main author", "main@example.invalid")
        (self.repo / "scripts").mkdir()
        (self.repo / ".github" / "workflows").mkdir(parents=True)
        (self.repo / ".github" / "workflows" / "fixture.yml").write_text("name: Original workflow\n")
        shutil.copy2(PREPARE_SCRIPT, self.repo / "scripts" / "prepare-release.sh")
        for name, content in {
            "Cargo.toml": '[workspace]\nmembers = []\n[workspace.package]\nversion = "0.1.0"\n[profile.release]\n',
            "Cargo.lock": 'version = 4\n[[package]]\nname = "fixture"\nversion = "0.1.0"\n',
            "CHANGELOG.md": "# Changelog\n", "engine.txt": "initial main input\n",
            "cliff.toml": "# fixture\n", ".gitignore": "target/\n",
        }.items():
            (self.repo / name).write_text(content, encoding="utf-8")
        self.git("add", "--", "Cargo.toml", "Cargo.lock", "CHANGELOG.md", "engine.txt", "cliff.toml", ".gitignore", "scripts/prepare-release.sh", ".github/workflows/fixture.yml")
        self.git("commit", "--quiet", "-m", "feat: original main input")
        self.original_base = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("remote", "add", "origin", str(self.origin))
        self.git("push", "--quiet", "origin", "main")
        self.cliff_calls = self.root / "cliff-calls.jsonl"
        self.calls = self.root / "gh-calls.jsonl"
        self.fixture = self.root / "pull-request.json"
        self.environment.update(FAKE_CLIFF_CALLS=str(self.cliff_calls), FAKE_GH_CALLS=str(self.calls),
                                FAKE_PR=str(self.fixture), FAKE_ORIGIN=str(self.origin),
                                FAKE_GQL_CALLS=str(self.root / "graphql-calls.jsonl"),
                                FAKE_QUEUED_HEAD=str(self.root / "queued-head"))
        cliff = self.repo / "target" / "release-tools" / "git-cliff-2.13.1" / "bin" / "git-cliff"
        cliff.parent.mkdir(parents=True)
        self.executable(cliff,
            "import json, os, subprocess, sys\nfrom pathlib import Path\n"
            "with Path(os.environ['FAKE_CLIFF_CALLS']).open('a') as target: target.write(json.dumps(sys.argv[1:])+'\\n')\n"
            "if sys.argv[1:] == ['--version']: print('git-cliff 2.13.1'); sys.exit(0)\n"
            "args=sys.argv[1:]; tag=args[args.index('--tag')+1]; path=Path(args[args.index('--prepend')+1])\n"
            "subjects=subprocess.check_output(['git','log','--format=%s','HEAD'], text=True)\n"
            "path.write_text('## '+tag+'\\n'+subjects+'\\n'+path.read_text())\n")
        self.executable(self.bin / "cargo",
            "import sys, tomllib\nfrom pathlib import Path\n"
            "if sys.argv[1:] == ['fetch','--locked']: sys.exit(0)\n"
            "if sys.argv[1:] != ['update','--workspace','--offline']: sys.exit(91)\n"
            "version=tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']\n"
            "Path('Cargo.lock').write_text('version = 4\\n[[package]]\\nname = \"fixture\"\\nversion = \"'+version+'\"\\n')\n")
        self.command(["bash", "scripts/prepare-release.sh", "0.2.0"])
        self.identity("clonk-rs-release[bot]", "311066358+clonk-rs-release[bot]@users.noreply.github.com")
        self.git("add", "--", "Cargo.toml", "Cargo.lock", "CHANGELOG.md")
        self.git("commit", "--quiet", "-m", "chore: release 0.2.0")
        self.head = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("push", "--quiet", "origin", "HEAD:refs/heads/release/next")
        self.git("checkout", "--quiet", "--detach", self.original_base)
        self.identity("Main author", "main@example.invalid")
        (self.repo / "engine.txt").write_text("newer main input\n")
        (self.repo / ".github" / "workflows" / "fixture.yml").write_text("name: Updated main workflow\n")
        self.git("commit", "--quiet", "-am", "fix: newer main input")
        self.selected = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("push", "--quiet", "origin", "HEAD:refs/heads/main")
        self.run_url = "https://github.com/clonk-org/clonk-rs/actions/runs/123"
        self.details = {
            "number": 19, "state": "open", "merged": False, "commits": 1,
            "html_url": "https://github.com/clonk-org/clonk-rs/pull/19",
            "title": "chore: release 0.2.0", "created_at": "2026-10-05T01:00:00Z",
            "body": "Automated release preparation for v0.2.0.\n\nPrepared by workflow run " + self.run_url + ".",
            "user": {"login": "clonk-rs-release[bot]", "type": "Bot"},
            "head": {"ref": "release/next", "sha": self.head, "repo": {"full_name": "clonk-org/clonk-rs"}},
            "base": {"ref": "main", "repo": {"full_name": "clonk-org/clonk-rs"}},
        }
        self.executable(self.bin / "gh",
            "import json, os, subprocess, sys\nfrom pathlib import Path\n"
            "args=sys.argv[1:]; pr=json.loads(Path(os.environ['FAKE_PR']).read_text())\n"
            "with Path(os.environ['FAKE_GH_CALLS']).open('a') as target: target.write(json.dumps(args)+'\\n')\n"
            "origin=['git','--git-dir='+os.environ['FAKE_ORIGIN']]\n"
            "if args[:2] == ['api','graphql']:\n"
            "    request=json.loads(Path(args[args.index('--input')+1]).read_text()); fields=request['variables']['input']\n"
            "    assert 'updateRefs(input: $input)' in request['query'] and fields['repositoryId'] == 'R_fixture'\n"
            "    updates=fields['refUpdates']; zero='0'*40\n"
            "    with Path(os.environ['FAKE_GQL_CALLS']).open('a') as target: target.write(json.dumps(fields)+'\\n')\n"
            "    if os.environ.get('FAKE_TEMP_REF_RACE') == 'true' and any(u['name'] == 'refs/heads/release/next' for u in updates):\n"
            "        temporary=next(u for u in updates if u['name'].startswith('refs/heads/ci-release-refresh/'))\n"
            "        subprocess.run(origin+['update-ref',temporary['name'],pr['head']['sha'],temporary['beforeOid']],check=True,capture_output=True)\n"
            "    if os.environ.get('FAKE_REJECT_DELETE') == 'true' and any(u['afterOid'] == zero for u in updates): sys.exit('temporary ref deletion denied')\n"
            "    if os.environ.get('FAKE_REJECT_REFRESH') == 'true' and any(u['name'] == 'refs/heads/release/next' for u in updates): sys.exit('GraphQL refresh permission denied')\n"
            "    if os.environ.get('FAKE_GRAPHQL_ERRORS') == 'true': print(json.dumps({'errors':[{'message':'ref update rejected'}]})); sys.exit(0)\n"
            "    transaction=['start']\n"
            "    for update in updates:\n"
            "        assert update['name'] == 'refs/heads/release/next' or update['name'].startswith('refs/heads/ci-release-refresh/')\n"
            "        before,after=update['beforeOid'],update['afterOid']\n"
            "        assert len(before) == len(after) == 40\n"
            "        if after != zero:\n"
            "            subprocess.run(origin+['cat-file','-e',after+'^{commit}'],check=True,capture_output=True)\n"
            "            if update['name'] == 'refs/heads/release/next': assert update['force'] is True\n"
            "            transaction.append('update '+update['name']+' '+after+' '+before)\n"
            "        else: transaction.append('delete '+update['name']+' '+before)\n"
            "    result=subprocess.run(origin+['update-ref','--stdin'],input='\\n'.join(transaction+['prepare','commit'])+'\\n',text=True,capture_output=True)\n"
            "    if result.returncode: sys.exit('stale info: atomic ref update rejected: '+result.stderr)\n"
            "    if os.environ.get('FAKE_LOST_CAS_RESPONSE') == 'true' and any(u['name'] == 'refs/heads/release/next' for u in updates): sys.exit('CAS response lost after server apply')\n"
            "    print(json.dumps({'data':{'updateRefs':{'clientMutationId':fields['clientMutationId']}}})); sys.exit(0)\n"
            "if args[:2] == ['pr','list']: print(json.dumps([{'url':pr['html_url'],'number':pr['number'],'state':'OPEN','createdAt':pr['created_at'],'body':pr['body'],'headRefOid':pr['head']['sha']}])); sys.exit(0)\n"
            "if args[0] == 'api':\n"
            "    endpoint=next(arg for arg in args if arg.startswith('repos/'))\n"
            "    if endpoint == 'repos/clonk-org/clonk-rs': print('R_fixture'); sys.exit(0)\n"
            "    if endpoint.endswith('/git/refs'):\n"
            "        assert args[args.index('--method')+1] == 'POST'\n"
            "        ref=next(arg.removeprefix('ref=') for arg in args if arg.startswith('ref='))\n"
            "        sha=next(arg.removeprefix('sha=') for arg in args if arg.startswith('sha='))\n"
            "        assert ref.startswith('refs/heads/ci-release-refresh/')\n"
            "        subprocess.run(origin+['merge-base','--is-ancestor',sha,'refs/heads/main'],check=True,capture_output=True)\n"
            "        subprocess.run(origin+['update-ref',ref,sha,'0'*40],check=True,capture_output=True)\n"
            "        if os.environ.get('FAKE_SEED_REF_RACE') == 'true': subprocess.run(origin+['update-ref',ref,pr['head']['sha'],sha],check=True,capture_output=True)\n"
            "        if os.environ.get('FAKE_AMBIGUOUS_SEED') == 'true': sys.exit('seed response lost after creation')\n"
            "        print(json.dumps({'ref':ref,'object':{'type':'commit','sha':sha}})); sys.exit(0)\n"
            "    if endpoint.endswith('/pulls/19'): print(json.dumps(pr)); sys.exit(0)\n"
            "    if '/commits/' in endpoint:\n"
            "        actor={'login':os.environ.get('FAKE_COMMIT_LOGIN','clonk-rs-release[bot]'),'type':os.environ.get('FAKE_COMMIT_TYPE','Bot')}\n"
            "        print(json.dumps({'author':actor,'committer':actor})); sys.exit(0)\n"
            "    sys.exit('unexpected branch API mutation')\n"
            "head=subprocess.check_output(['git','--git-dir='+os.environ['FAKE_ORIGIN'],'rev-parse','refs/heads/release/next'],text=True).strip()\n"
            "if args[:2] == ['pr','view']: print('OPEN\\t'+head); sys.exit(0)\n"
            "if args[:2] == ['pr','merge']:\n"
            "    assert '--auto' in args and '--squash' in args\n"
            "    assert args[args.index('--match-head-commit')+1] == head\n"
            "    Path(os.environ['FAKE_QUEUED_HEAD']).write_text(head); sys.exit(0)\n"
            "sys.exit('refresh must not create another pull request')\n")
        self.invocations = 0
        self.cliff_calls.unlink()
        # A Contents-only App can push the three generated files onto a ref
        # seeded at main, but cannot introduce main's intervening workflow
        # changes through a Git push onto the old candidate branch.
        self.executable(self.origin / "hooks" / "pre-receive",
            "import os, subprocess, sys\n"
            "if os.environ.get('FAKE_CONTENTS_ONLY') != 'true': sys.exit(0)\n"
            "for line in sys.stdin:\n"
            "    before, after, ref=line.split()\n"
            "    if before == '0'*40: paths=subprocess.check_output(['git','ls-tree','-r','--name-only',after],text=True).splitlines()\n"
            "    else: paths=subprocess.check_output(['git','diff','--name-only',before,after],text=True).splitlines()\n"
            "    if any(path.startswith('.github/workflows/') for path in paths): sys.exit('Workflows permission required for Git push')\n")

    def executable(self, path, source):
        path.write_text(f"#!{sys.executable}\n" + source, encoding="utf-8")
        path.chmod(0o755)

    def command(self, arguments):
        return subprocess.run(arguments, cwd=self.repo, env=self.environment, capture_output=True, text=True, check=True)

    def git(self, *arguments):
        return self.command(["git", *arguments])

    def identity(self, name, email):
        self.git("config", "user.name", name)
        self.git("config", "user.email", email)

    def amend_release(self, change=None, name="clonk-rs-release[bot]",
                      email="311066358+clonk-rs-release[bot]@users.noreply.github.com", message=None):
        previous = self.head
        self.git("checkout", "--quiet", "--detach", previous)
        self.identity(name, email)
        if change:
            change()
        arguments = ["commit", "--quiet", "--amend", "--reset-author"]
        arguments += ["-m", message] if message else ["--no-edit"]
        self.git(*arguments)
        self.head = self.git("rev-parse", "HEAD").stdout.strip()
        self.details["head"]["sha"] = self.head
        self.git("push", "--quiet", "--force-with-lease=refs/heads/release/next:" + previous,
                 "origin", "HEAD:refs/heads/release/next")
        self.git("checkout", "--quiet", "--detach", self.selected)
        self.identity("Main author", "main@example.invalid")

    def step(self, name, **extra):
        self.fixture.write_text(json.dumps(self.details), encoding="utf-8")
        self.invocations += 1
        output = self.root / f"outputs-{self.invocations}"
        environment = {**self.environment, "GITHUB_OUTPUT": str(output), "RUNNER_TEMP": str(self.root),
                       "REPOSITORY": "clonk-org/clonk-rs", "GH_TOKEN": "fixture-token",
                       "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1",
                       "BASE_SHA": self.selected, "PR_NUMBER": "19", "PR_URL": self.details["html_url"],
                       "RUN_URL": self.run_url, "REQUESTED_VERSION": "", "EXPECTED_HEAD": self.head,
                       "VERSION": "0.2.0", **extra}
        result = subprocess.run(["bash", "--noprofile", "--norc", "-euo", "pipefail", "-c", prepare_step_script(name)],
                                cwd=self.repo, env=environment, capture_output=True, text=True, timeout=10)
        values = dict(line.split("=", 1) for line in output.read_text().splitlines()) if output.exists() else {}
        return result, values

    def test_owned_single_generated_commit_on_older_main_requires_refresh_at_the_original_version(self):
        result, values = self.step("Validate existing release candidate")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(values["refresh"], "true")
        self.assertEqual(values["version"], "0.2.0")
        self.assertEqual(values["head-sha"], self.head)
        self.assertEqual(values["prepared-run-id"], "123")

    def test_refresh_regenerates_from_selected_main_preserves_pr_metadata_and_queues_the_exact_new_head(self):
        original = json.loads(json.dumps(self.details))
        checked, initial = self.step("Check for an in-flight release", GITHUB_EVENT_NAME="push")
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(initial["create"], "false")
        self.assertEqual(initial["process"], "true")
        checked, context = self.step("Validate existing release candidate", EXPECTED_HEAD=initial["head-sha"])
        self.assertEqual(checked.returncode, 0, checked.stderr)
        prepared, outputs = self.step("Prepare the release", REQUESTED_VERSION=context["version"], REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        self.assertEqual(outputs["version"], "0.2.0")

        refreshed, outputs = self.step("Refresh existing release pull request", EXPECTED_HEAD=context["head-sha"],
                                       VERSION=context["version"], PREPARED_RUN_ID=context["prepared-run-id"])

        self.assertEqual(refreshed.returncode, 0, refreshed.stderr)
        new_head = outputs["head-sha"]
        self.assertNotEqual(new_head, self.head)
        self.assertEqual(self.git("rev-parse", new_head + "^").stdout.strip(), self.selected)
        self.assertEqual(set(self.git("diff-tree", "--no-commit-id", "--name-only", "-r", new_head).stdout.splitlines()),
                         {"Cargo.toml", "Cargo.lock", "CHANGELOG.md"})
        self.assertIn("fix: newer main input", (self.repo / "CHANGELOG.md").read_text())
        self.assertEqual(self.details, original)
        queued, _ = self.step("Ensure the release pull request is queued", EXPECTED_HEAD=new_head)
        self.assertEqual(queued.returncode, 0, queued.stderr)
        self.assertEqual((self.root / "queued-head").read_text(), new_head)
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertFalse(any(call[:2] in (["pr", "create"], ["pr", "edit"]) for call in calls))
        self.assertFalse(any("PATCH" in call or "ref=refs/heads/release/next" in call for call in calls))
        cliff = [json.loads(line) for line in self.cliff_calls.read_text().splitlines()]
        self.assertEqual(sum("--prepend" in call for call in cliff), 1)

    def test_contents_only_refresh_uses_existing_repo_objects_for_intervening_workflow_changes(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refreshed, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                       FAKE_CONTENTS_ONLY="true")

        self.assertEqual(refreshed.returncode, 0, refreshed.stderr)
        new_head = outputs["head-sha"]
        self.assertEqual(self.git("rev-parse", new_head + "^").stdout.strip(), self.selected)
        remote = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        self.assertEqual(remote.stdout.strip(), new_head)
        self.assertEqual(self.command(["git", "--git-dir=" + str(self.origin), "for-each-ref",
                                       "--format=%(refname)", "refs/heads/ci-release-refresh/"]).stdout, "")
        workflow = PREPARE.read_text()
        self.assertNotIn("permission-workflows:", workflow)
        self.assertNotIn("workflows: write", workflow)
        mutations = [json.loads(line) for line in (self.root / "graphql-calls.jsonl").read_text().splitlines()]
        self.assertEqual(len(mutations), 1)
        replacement, deletion = mutations[0]["refUpdates"]
        self.assertEqual(replacement, {"name": "refs/heads/release/next", "beforeOid": self.head,
                                      "afterOid": new_head, "force": True})
        self.assertTrue(deletion["name"].startswith("refs/heads/ci-release-refresh/123-1-"))
        self.assertEqual(deletion["beforeOid"], new_head)
        self.assertEqual(deletion["afterOid"], "0" * 40)

    def test_graphql_permission_failure_keeps_candidate_and_removes_only_owned_temporary_ref(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_REJECT_REFRESH="true", FAKE_CONTENTS_ONLY="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("GraphQL refresh permission denied", refused.stderr)
        self.assertNotIn("head-sha", outputs)
        self.assertEqual(outputs["cleanup"], "complete")
        receipt = json.loads((self.root / "release-refresh-ref.json").read_text())
        self.assertEqual(receipt["candidate_update"], "pending")
        self.assertEqual(receipt["cleanup"], "complete")
        remote = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        self.assertEqual(remote.stdout.strip(), self.head)
        self.assertEqual(self.command(["git", "--git-dir=" + str(self.origin), "for-each-ref",
                                       "--format=%(refname)", "refs/heads/ci-release-refresh/"]).stdout, "")

    def test_denied_temporary_ref_deletion_rejects_candidate_swap_atomically_and_retains_pending_receipt(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_REJECT_DELETE="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertNotIn("head-sha", outputs)
        self.assertEqual(outputs["cleanup"], "pending")
        self.assertIn("cleanup remains pending", refused.stdout)
        receipt = json.loads((self.root / "release-refresh-ref.json").read_text())
        self.assertEqual(receipt["candidate_update"], "pending")
        self.assertEqual(receipt["cleanup"], "pending")
        remote = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        self.assertEqual(remote.stdout.strip(), self.head)
        temporary = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", receipt["temporary_ref"]])
        self.assertEqual(temporary.stdout.strip(), receipt["generated_commit"])
        mutations = [json.loads(line) for line in (self.root / "graphql-calls.jsonl").read_text().splitlines()]
        self.assertEqual(len(mutations), 2)
        self.assertEqual(len(mutations[-1]["refUpdates"]), 1)
        self.assertEqual(mutations[-1]["refUpdates"][0]["name"], receipt["temporary_ref"])
        workflow = PREPARE.read_text()
        self.assertIn("timeout --kill-after=5s 30s gh api graphql", workflow)
        self.assertIn("Retain unresolved release refresh cleanup receipt", workflow)
        self.assertIn("steps.refresh.outputs.cleanup == 'pending'", workflow)

    def test_changed_temporary_ref_is_preserved_and_prevents_candidate_swap(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_TEMP_REF_RACE="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertNotIn("head-sha", outputs)
        self.assertEqual(outputs["cleanup"], "pending")
        receipt = json.loads((self.root / "release-refresh-ref.json").read_text())
        candidate = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        temporary = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", receipt["temporary_ref"]])
        self.assertEqual(candidate.stdout.strip(), self.head)
        self.assertEqual(temporary.stdout.strip(), self.head)
        self.assertNotEqual(temporary.stdout.strip(), receipt["generated_commit"])

    def test_git_lease_preserves_a_temporary_ref_changed_before_generated_commit_push(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_SEED_REF_RACE="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("stale info", refused.stderr)
        self.assertNotIn("head-sha", outputs)
        self.assertEqual(outputs["cleanup"], "pending")
        receipt = json.loads((self.root / "release-refresh-ref.json").read_text())
        candidate = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        temporary = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", receipt["temporary_ref"]])
        self.assertEqual(candidate.stdout.strip(), self.head)
        self.assertEqual(temporary.stdout.strip(), self.head)

    def test_ambiguous_seed_response_never_claims_ownership_or_deletes_the_unknown_ref(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_AMBIGUOUS_SEED="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual(outputs["cleanup"], "pending")
        self.assertNotIn("head-sha", outputs)
        self.assertFalse((self.root / "graphql-calls.jsonl").exists())
        receipt = json.loads((self.root / "release-refresh-ref.json").read_text())
        temporary = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", receipt["temporary_ref"]])
        self.assertEqual(temporary.stdout.strip(), self.selected)

    def test_graphql_errors_in_a_success_http_response_cannot_report_a_refreshed_head(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_GRAPHQL_ERRORS="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertNotIn("head-sha", outputs)
        self.assertEqual(outputs["cleanup"], "pending")
        candidate = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        self.assertEqual(candidate.stdout.strip(), self.head)

    def test_applied_swap_with_lost_response_keeps_candidate_status_pending_without_reverting_it(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_LOST_CAS_RESPONSE="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertNotIn("head-sha", outputs)
        receipt = json.loads((self.root / "release-refresh-ref.json").read_text())
        candidate = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        self.assertEqual(candidate.stdout.strip(), receipt["generated_commit"])
        self.assertEqual(receipt["candidate_update"], "pending")
        self.assertEqual(receipt["cleanup"], "pending")
        self.assertEqual(self.command(["git", "--git-dir=" + str(self.origin), "for-each-ref",
                                       "--format=%(refname)", "refs/heads/ci-release-refresh/"]).stdout, "")

    def test_successful_push_with_failed_client_response_retains_exact_temporary_ref_as_pending(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        real_git = shutil.which("git")
        self.executable(self.bin / "git",
            f"import os, subprocess, sys\nreal={real_git!r}\n"
            "result=subprocess.run([real]+sys.argv[1:])\n"
            "if sys.argv[1] == 'push' and result.returncode == 0 and os.environ.get('FAKE_LOST_PUSH_RESPONSE') == 'true': sys.exit('Git push response lost after server apply')\n"
            "sys.exit(result.returncode)\n")

        refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123",
                                     FAKE_LOST_PUSH_RESPONSE="true")

        self.assertNotEqual(refused.returncode, 0)
        self.assertNotIn("head-sha", outputs)
        receipt = json.loads((self.root / "release-refresh-ref.json").read_text())
        candidate = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        temporary = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", receipt["temporary_ref"]])
        self.assertEqual(candidate.stdout.strip(), self.head)
        self.assertEqual(temporary.stdout.strip(), receipt["generated_commit"])
        self.assertEqual(receipt["temporary_expected_oid"], self.selected)
        self.assertEqual(receipt["candidate_update"], "pending")
        self.assertEqual(receipt["cleanup"], "pending")

    def test_refresh_refuses_to_downgrade_a_version_already_advanced_on_main(self):
        manifest = self.repo / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('version = "0.1.0"', 'version = "0.3.0"'))
        self.git("commit", "--quiet", "-am", "chore: release 0.3.0")
        self.selected = self.git("rev-parse", "HEAD").stdout.strip()
        checked, context = self.step("Validate existing release candidate")
        self.assertEqual(checked.returncode, 0, checked.stderr)

        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION=context["version"], REFRESHING_RELEASE="true")

        self.assertNotEqual(prepared.returncode, 0)
        self.assertIn("no longer newer than main", prepared.stderr)
        self.assertFalse(self.cliff_calls.exists())
        self.assertIn('version = "0.3.0"', manifest.read_text())

    def test_candidate_prepared_on_selected_main_needs_no_regeneration(self):
        self.git("checkout", "--quiet", "--detach", self.original_base)

        checked, context = self.step("Validate existing release candidate", BASE_SHA=self.original_base)

        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(context["refresh"], "false")
        self.assertEqual(context["prepared-parent"], self.original_base)
        self.assertFalse(self.cliff_calls.exists())

    def test_explicit_version_can_repeat_original_but_cannot_replace_it(self):
        accepted, context = self.step("Validate existing release candidate", REQUESTED_VERSION="v0.2.0")
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        self.assertEqual(context["version"], "0.2.0")

        refused, _ = self.step("Validate existing release candidate", REQUESTED_VERSION="0.2.1")

        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("differs from existing release version", refused.stderr)
        self.assertFalse(self.cliff_calls.exists())

    def test_manual_forked_changed_and_multiple_commit_candidates_are_rejected(self):
        original = json.loads(json.dumps(self.details))
        changes = [
            ("manual PR", ("user", "login"), "maintainer"),
            ("user rather than bot", ("user", "type"), "User"),
            ("fork", ("head", "repo", "full_name"), "attacker/clonk-rs"),
            ("manual branch", ("head", "ref"), "release/manual"),
            ("wrong base", ("base", "ref"), "develop"),
            ("changed branch", ("head", "sha"), "f" * 40),
            ("multiple commits", ("commits",), 2),
        ]
        for label, keys, value in changes:
            with self.subTest(label=label):
                self.details = json.loads(json.dumps(original))
                target = self.details
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value

                refused, outputs = self.step("Validate existing release candidate")

                self.assertNotEqual(refused.returncode, 0)
                self.assertNotEqual(outputs.get("refresh"), "true")
                self.assertIn("refusing to refresh", refused.stderr)
        self.assertFalse(self.cliff_calls.exists())

    def test_bot_owned_pr_does_not_authorize_a_manual_commit(self):
        self.amend_release(name="Maintainer", email="maintainer@example.invalid")

        refused, _ = self.step("Validate existing release candidate")

        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("identity is not the release automation", refused.stderr)
        self.assertFalse(self.cliff_calls.exists())

    def test_raw_bot_email_requires_matching_github_commit_attribution(self):
        refused, outputs = self.step("Validate existing release candidate",
                                     FAKE_COMMIT_LOGIN="maintainer", FAKE_COMMIT_TYPE="User")

        self.assertNotEqual(refused.returncode, 0)
        self.assertNotEqual(outputs.get("refresh"), "true")
        self.assertFalse(self.cliff_calls.exists())

    def test_release_commit_message_cannot_include_manual_body_or_another_version(self):
        for message in ("chore: release 0.2.0\n\nManual commit body", "chore: release 0.2.1"):
            with self.subTest(message=message):
                self.amend_release(message=message)

                refused, outputs = self.step("Validate existing release candidate")

                self.assertNotEqual(refused.returncode, 0)
                self.assertNotEqual(outputs.get("refresh"), "true")
        self.assertFalse(self.cliff_calls.exists())

    def test_api_single_commit_count_does_not_authorize_a_merge_commit(self):
        self.identity("clonk-rs-release[bot]", "311066358+clonk-rs-release[bot]@users.noreply.github.com")
        previous = self.head
        tree = self.git("rev-parse", previous + "^{tree}").stdout.strip()
        self.head = self.git("commit-tree", tree, "-p", self.original_base, "-p", self.selected,
                             "-m", "chore: release 0.2.0").stdout.strip()
        self.details["head"]["sha"] = self.head
        self.git("push", "--quiet", "--force-with-lease=refs/heads/release/next:" + previous,
                 "origin", self.head + ":refs/heads/release/next")

        refused, outputs = self.step("Validate existing release candidate")

        self.assertNotEqual(refused.returncode, 0)
        self.assertNotEqual(outputs.get("refresh"), "true")
        self.assertFalse(self.cliff_calls.exists())

    def test_generated_commit_cannot_hide_an_engine_change(self):
        def alter_engine():
            (self.repo / "engine.txt").write_text("unauthorized release engine change\n")
            self.git("add", "--", "engine.txt")
        self.amend_release(change=alter_engine)

        refused, _ = self.step("Validate existing release candidate")

        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("illegal generated release file: engine.txt", refused.stderr)
        self.assertFalse(self.cliff_calls.exists())

    def test_original_preparation_anchor_survives_a_new_dispatch(self):
        checked, context = self.step("Validate existing release candidate",
                                     RUN_URL=self.run_url.rsplit("/", 1)[0] + "/456")

        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(context["prepared-run-id"], "123")
        self.assertEqual(self.details["created_at"], "2026-10-05T01:00:00Z")

    def test_missing_or_duplicate_preparation_anchors_are_rejected(self):
        original = self.details["body"]
        for body in ("Manual release", original + "\nPrepared by workflow run " + self.run_url + "."):
            with self.subTest(body=body):
                self.details["body"] = body

                refused, _ = self.step("Validate existing release candidate")

                self.assertNotEqual(refused.returncode, 0)
                self.assertIn("missing unique original preparation anchor", refused.stderr)
        self.assertFalse(self.cliff_calls.exists())

    def test_refresh_lease_preserves_a_branch_changed_after_validation(self):
        checked, context = self.step("Validate existing release candidate")
        self.assertEqual(checked.returncode, 0, checked.stderr)
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION=context["version"], REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        # The PR API intentionally remains at the previously observed head.
        # The atomic mutation must still compare the actual Git reference.
        self.command(["git", "--git-dir=" + str(self.origin), "update-ref",
                      "refs/heads/release/next", self.selected, self.head])

        refused, outputs = self.step("Refresh existing release pull request", EXPECTED_HEAD=context["head-sha"],
                                     VERSION=context["version"], PREPARED_RUN_ID=context["prepared-run-id"])

        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("stale info", refused.stderr)
        self.assertNotIn("head-sha", outputs)
        remote = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        self.assertEqual(remote.stdout.strip(), self.selected)
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertFalse(any("PATCH" in call or "ref=refs/heads/release/next" in call for call in calls))
        self.assertFalse((self.root / "queued-head").exists())

    def test_queue_refuses_a_head_changed_since_validation(self):
        self.command(["git", "--git-dir=" + str(self.origin), "update-ref",
                      "refs/heads/release/next", self.selected, self.head])

        refused, _ = self.step("Ensure the release pull request is queued", EXPECTED_HEAD=self.head)

        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("cannot queue a changed release", refused.stderr)
        self.assertFalse((self.root / "queued-head").exists())

    def test_candidate_merged_while_validation_started_has_no_writes(self):
        self.details.update(merged=True, state="closed")

        checked, context = self.step("Validate existing release candidate")

        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(context["refresh"], "false")
        self.assertFalse(self.cliff_calls.exists())
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertEqual(calls, [["api", "repos/clonk-org/clonk-rs/pulls/19"]])

    def test_older_rerun_cannot_rewind_a_candidate_already_refreshed_on_newer_main(self):
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION="0.2.0", REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        refreshed, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123")
        self.assertEqual(refreshed.returncode, 0, refreshed.stderr)
        self.head = outputs["head-sha"]
        self.details["head"]["sha"] = self.head
        newer_parent = self.selected
        self.git("checkout", "--quiet", "--detach", self.original_base)

        checked, context = self.step("Validate existing release candidate", BASE_SHA=self.original_base)

        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(context["refresh"], "false")
        self.assertEqual(context["prepared-parent"], newer_parent)
        self.assertIn("older rerun will not rewind", checked.stdout)

    def test_candidate_identity_and_anchor_are_rechecked_immediately_before_refresh_push(self):
        checked, context = self.step("Validate existing release candidate")
        self.assertEqual(checked.returncode, 0, checked.stderr)
        prepared, _ = self.step("Prepare the release", REQUESTED_VERSION=context["version"], REFRESHING_RELEASE="true")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        original = json.loads(json.dumps(self.details))
        for label in ("owner", "anchor"):
            with self.subTest(label=label):
                self.details = json.loads(json.dumps(original))
                if label == "owner":
                    self.details["user"]["login"] = "maintainer"
                else:
                    self.details["body"] = "Edited manually after validation."

                refused, outputs = self.step("Refresh existing release pull request", PREPARED_RUN_ID="123")

                self.assertNotEqual(refused.returncode, 0)
                self.assertIn("candidate changed before refresh", refused.stderr)
                self.assertNotIn("head-sha", outputs)
                self.assertEqual(self.git("rev-parse", "HEAD").stdout.strip(), self.selected)
        remote = self.command(["git", "--git-dir=" + str(self.origin), "rev-parse", "refs/heads/release/next"])
        self.assertEqual(remote.stdout.strip(), self.head)


@unittest.skipUnless(
    shutil.which("bash") and shutil.which("sha256sum") and shutil.which("tar"),
    "needs bash, sha256sum and tar",
)
class GitCliffInstallTests(unittest.TestCase):
    """The release must not build git-cliff from source.

    On 2026-08-11 it did: the cache holding the compiled binary had been
    evicted -- the repository sits against the 10 GiB Actions cache cap and
    this entry is touched once a day, so it loses to caches touched on every
    push -- and `prepare-release.sh` fell back to `cargo install`. That took
    2m36s and blew the 120s SLO below, which failed the job, which stopped
    `actions/cache` from saving the binary it had just built, so every later
    run rebuilt it and failed identically. Installing the pinned prebuilt
    release instead removes the compile, and with it the cache it depended on.
    """

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.bin = self.root / "stubs"
        self.bin.mkdir()
        self.temp = self.root / "runner-temp"
        self.temp.mkdir()

        version = re.search(
            r"^tool_version=(\S+)$",
            PREPARE_SCRIPT.read_text(encoding="utf-8"),
            re.M,
        )
        self.assertIsNotNone(version, "prepare-release.sh pins no git-cliff version")
        self.version = version.group(1)

        # A stand-in for the published archive: same layout, same reported
        # version, so the step's extraction and the script's `--version` check
        # are both exercised without reaching the network.
        self.archive = self.root / "git-cliff.tar.gz"
        self._write_archive(self.archive, f'echo "git-cliff {self.version}"\n')

    def _write_archive(self, path, body):
        payload = ("#!/usr/bin/env bash\n" + body).encode("utf-8")
        with tarfile.open(path, "w:gz") as archive:
            entry = tarfile.TarInfo(f"git-cliff-{self.version}/git-cliff")
            entry.size = len(payload)
            entry.mode = 0o755
            archive.addfile(entry, io.BytesIO(payload))

    def _stub_curl(self):
        # Serves whatever CURL_PAYLOAD names, so a test can hand the step
        # bytes that do not match the pinned digest.
        stub = self.bin / "curl"
        stub.write_text(
            "#!/usr/bin/env bash\n"
            "output=\n"
            'while [[ $# -gt 0 ]]; do\n'
            '  if [[ "$1" == "--output" ]]; then\n'
            '    output="$2"\n'
            "    shift 2\n"
            "    continue\n"
            "  fi\n"
            "  shift\n"
            "done\n"
            '[[ -n "$output" ]] || exit 1\n'
            'cp "$CURL_PAYLOAD" "$output"\n',
            encoding="utf-8",
        )
        stub.chmod(0o755)

    def run_install(self, payload, digest=None):
        self._stub_curl()
        # The step's own pins, verbatim: if the workflow and the script drift
        # apart on the version, the install lands in a directory the probe
        # below does not look in, and this fails. The digest is the one pin a
        # local run must override -- the workflow pins the published archive,
        # and these tests serve a stand-in.
        environment = {
            **os.environ,
            **prepare_step_env("Install git-cliff"),
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "CURL_PAYLOAD": str(payload),
            "RUNNER_TEMP": str(self.temp),
        }
        if digest is not None:
            environment["GIT_CLIFF_SHA256"] = digest
        return subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                prepare_step_script("Install git-cliff"),
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )

    def probe_installed_tool(self):
        """Run prepare-release.sh's own lookup against the installed tree."""
        return subprocess.run(
            [
                "bash",
                "--noprofile",
                "--norc",
                "-eo",
                "pipefail",
                "-c",
                'repo_root="$PWD"\n'
                + prepare_script_tool_probe()
                + '\nif [[ ! -x "$tool" ]]; then\n'
                '  echo "no executable git-cliff at $tool" >&2\n'
                "  exit 1\n"
                "fi\n"
                '"$tool" --version\n',
            ],
            cwd=self.root,
            env={k: v for k, v in os.environ.items() if k != "CLONK_GIT_CLIFF_ROOT"},
            capture_output=True,
            text=True,
        )

    def test_install_lands_where_the_release_script_looks_for_it(self):
        installed = self.run_install(self.archive, digest=digest_of(self.archive))
        self.assertEqual(installed.returncode, 0, installed.stderr)

        probed = self.probe_installed_tool()

        self.assertEqual(probed.returncode, 0, probed.stderr)
        self.assertEqual(probed.stdout.strip(), f"git-cliff {self.version}")

    def test_archive_that_is_not_the_pinned_bytes_installs_nothing(self):
        substitute = self.root / "substitute.tar.gz"
        self._write_archive(substitute, 'echo "git-cliff 0.0.0"\n')

        installed = self.run_install(substitute, digest=digest_of(self.archive))

        self.assertNotEqual(installed.returncode, 0)
        self.assertIn(digest_of(substitute), installed.stderr)
        self.assertNotEqual(self.probe_installed_tool().returncode, 0)

    def test_workflow_pins_a_digest_for_the_published_archive(self):
        # Nothing offline can prove the pin matches the published bytes; a
        # wrong one fails the release loudly rather than installing whatever
        # the URL happens to serve.
        pinned = prepare_step_env("Install git-cliff").get("GIT_CLIFF_SHA256", "")

        self.assertRegex(pinned, r"^[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
