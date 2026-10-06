from __future__ import annotations

import base64
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
import zipfile


SCRIPT = Path(__file__).resolve().parents[1] / "release-qualification-evidence.py"
REPOSITORY = "clonk-org/clonk-rs"
LINUX = (
    "presentation captures", "app 1/12", "app 12/12", "app 3+10/12", "app 2+7/12",
    "app 4+9/12", "app 5/12", "app 11/12", "app 6/12", "app 8/12",
    "engine integration 1/3", "engine integration 2/3", "engine integration 3/3",
    "engine and frontend unit and parity", "remaining workspace 1/2",
    "remaining workspace 2/2", "workspace quality", "engine contracts",
)
COVERAGE = (
    ("app 1+10/12", "app-1-10"), ("app 2+7/12", "app-2-7"), ("app 3/12", "app-3"),
    ("app 4+9/12", "app-4-9"), ("app 5/12", "app-5"), ("app 11+12/12", "app-11-12"),
    ("app 6+8/12", "app-6-8"), ("engine integration 1/3", "engine-1"),
    ("engine integration 2+3/3", "engine-2-3"),
    ("engine and frontend units", "engine-and-frontend-units"),
    ("remaining workspace 1/2", "remaining-1"), ("remaining workspace 2/2", "remaining-2"),
)


class QualificationEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.sandbox = Path(tempfile.mkdtemp(prefix="clonk-qualification-evidence-"))
        self.addCleanup(shutil.rmtree, self.sandbox, ignore_errors=True)
        self.root = self.sandbox / "source"
        origin = self.sandbox / "content-origin"
        self.environment = {
            key: value for key, value in os.environ.items()
            if not key.startswith(("GIT_", "GITHUB_", "FAKE_GH_"))
        }
        self.environment.update({
            "GIT_ALLOW_PROTOCOL": "file", "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
        })
        for directory in (self.root, origin):
            directory.mkdir()
            self.git(directory, "init", "--quiet")
            self.git(directory, "config", "user.name", "Qualification test")
            self.git(directory, "config", "user.email", "qualification@example.invalid")
        (origin / "Scenario.txt").write_text("pinned parity input\n", encoding="utf-8")
        self.git(origin, "add", "Scenario.txt")
        self.git(origin, "commit", "--quiet", "-m", "test: pin content")
        self.content_sha = self.git(origin, "rev-parse", "HEAD").stdout.strip()
        self.git(self.root, "submodule", "add", "--quiet", str(origin), "content")
        (self.root / "Cargo.toml").write_text(
            '[workspace]\nmembers=[]\n[workspace.package]\nversion="1.2.3"\n', encoding="utf-8",
        )
        (self.root / ".gitignore").write_text("target/\n", encoding="utf-8")
        (self.root / ".github" / "workflows").mkdir(parents=True)
        (self.root / ".github" / "workflows" / "landing.yml").write_text("name: Landing\n", encoding="utf-8")
        (self.root / "qualification-recipe.py").write_text("required = True\n", encoding="utf-8")
        self.git(self.root, "add", "Cargo.toml", ".gitignore", ".github", "qualification-recipe.py")
        self.git(self.root, "commit", "--quiet", "-m", "test: pin qualification source")
        self.sha = self.git(self.root, "rev-parse", "HEAD").stdout.strip()
        self.tree = self.git(self.root, "rev-parse", "HEAD^{tree}").stdout.strip()
        self.output = self.sandbox / "release-qualification-evidence.json"
        self.state_path = self.sandbox / "github.json"
        self.calls_path = self.sandbox / "github-calls.jsonl"
        fake_bin = self.sandbox / "bin"
        fake_bin.mkdir()
        fake_gh = fake_bin / "gh"
        fake_gh.write_text("#!/usr/bin/env python3\n" + '\n'.join((
            "import base64, json, os, sys",
            "from urllib.parse import urlsplit, parse_qs",
            "args = sys.argv[1:]",
            "with open(os.environ['FAKE_GH_CALLS'], 'a') as log: log.write(json.dumps(args) + '\\n')",
            "if os.environ.get('FAKE_GH_FAILURE'): sys.exit('simulated GitHub API error')",
            "state = json.load(open(os.environ['FAKE_GH_STATE']))",
            "endpoint = next(value for value in args if value.startswith('repos/'))",
            "url = urlsplit(endpoint); query = parse_qs(url.query); page = int(query.get('page', ['1'])[0])",
            "if url.path.endswith('/zip'):",
            "    sys.stdout.buffer.write(base64.b64decode(state['archive'])); sys.exit(0)",
            "if '/attempts/' in url.path and url.path.endswith('/jobs'):",
            "    field, values = 'jobs', state.get('attempt_jobs', {}).get(url.path.split('/')[-2], [])",
            "elif url.path.endswith('/jobs'): field, values = 'jobs', state['jobs']",
            "elif url.path.endswith('/artifacts'): field, values = 'artifacts', state['artifacts']",
            "elif url.path.endswith('/runs'):",
            "    field, values = 'workflow_runs', state.get('runs', [state['run']])",
            "else: print(json.dumps(state['run'])); sys.exit(0)",
            "print(json.dumps({'total_count': len(values), field: values[(page-1)*100:page*100]}))",
            "",
        )), encoding="utf-8")
        fake_gh.chmod(0o755)
        self.environment.update({
            "PATH": str(fake_bin) + os.pathsep + self.environment["PATH"],
            "FAKE_GH_STATE": str(self.state_path), "FAKE_GH_CALLS": str(self.calls_path),
        })
        self.state = {
            "run": {
                "id": 73, "run_attempt": 2, "workflow_id": 99, "name": "Landing",
                "path": ".github/workflows/landing.yml", "event": "merge_group",
                "status": "in_progress", "conclusion": None, "head_sha": self.sha,
                "repository": {"id": 999, "full_name": REPOSITORY},
                "head_repository": {"id": 999, "full_name": REPOSITORY},
            },
            "jobs": [], "artifacts": [],
        }
        for name in LINUX:
            self.add_job(f"Linux / {name}")
        for name in ("runtime and quality", "network tests"):
            self.add_job(f"Windows / {name}")
        for name, _ in COVERAGE:
            self.add_job(f"Qualify release candidate / Rust coverage / {name}")
        for name in (
            "Rust code coverage", "Recording-host material-order oracles (macOS)",
            "Platform lints / macOS", "Platform lints / Windows",
        ):
            self.add_job(f"Qualify release candidate / {name}")
        for platform in ("linux", "windows", "macos"):
            prefix = f"Build release candidate / {platform} / Compile shipped inputs / "
            self.add_job(prefix + "Validate release pull request")
            self.add_job(prefix + f"Prebuild packaging tool / {platform}")
            runtimes = ("macos-arm64", "macos-x86_64") if platform == "macos" else (platform,)
            for runtime in runtimes:
                self.add_job(prefix + f"Prebuild runtime / {runtime}")
            self.add_job(f"Build release candidate / {platform} / Package and qualify / Package {platform}",
                         steps=[{"name": "Qualify the packaged runtime", "status": "completed", "conclusion": "success"}])
        self.add_job("Resolve release candidate")
        self.add_job("Landing gate", status="queued", conclusion=None)
        self.add_job("Retain release qualification evidence", status="in_progress", conclusion=None)
        self.add_job("Pull request title", conclusion="skipped")
        for name in ("desktop-linux", "desktop-windows", "desktop-macos", "components-desktop-linux",
                     "components-desktop-windows", "components-desktop-macos", "release-tool"):
            self.add_artifact(name)
        for kind in ("tool-linux", "tool-windows", "tool-macos", "runtime-linux", "runtime-windows",
                     "runtime-macos-arm64", "runtime-macos-x86_64"):
            self.add_artifact(f"release-prebuild-{kind}-{self.sha}-73")
        for platform in ("linux", "windows", "macos"):
            self.add_artifact(f"release-qualified-runtime-{platform}-{self.sha}-73")
        for _, artifact in COVERAGE:
            self.add_artifact(f"rust-coverage-fragment-73-{artifact}")
        for platform in ("Linux", "Windows", "macOS"):
            self.add_artifact(f"device-loss-{platform}-{self.sha}")

    def git(self, root: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(root), *arguments], env=self.environment, check=True,
            capture_output=True, text=True, timeout=15,
        )

    def add_job(self, name: str, **overrides) -> dict:
        job = {
            "id": 1000 + len(self.state["jobs"]), "run_id": 73, "run_attempt": 2,
            "head_sha": self.sha, "name": name, "status": "completed", "conclusion": "success",
            "started_at": "2026-10-01T10:00:00Z", "completed_at": "2026-10-01T10:02:00Z",
            "steps": [], **overrides,
        }
        self.state["jobs"].append(job)
        return job

    def add_artifact(self, name: str, **overrides) -> dict:
        artifact = {
            "id": 2000 + len(self.state["artifacts"]), "name": name, "size_in_bytes": 100,
            "digest": "sha256:" + "a" * 64, "expired": False,
            "created_at": "2026-10-01T10:00:00Z", "updated_at": "2026-10-01T10:02:00Z",
            "expires_at": "2099-10-01T10:00:00Z",
            "workflow_run": {"id": 73, "repository_id": 999, "head_repository_id": 999, "head_sha": self.sha},
            **overrides,
        }
        self.state["artifacts"].append(artifact)
        return artifact

    def run_evidence(self, operation: str, *arguments: str) -> subprocess.CompletedProcess[str]:
        self.state_path.write_text(json.dumps(self.state), encoding="utf-8")
        return subprocess.run(
            [sys.executable, str(SCRIPT), operation, "--repository", REPOSITORY,
             "--source-sha", self.sha, "--output", str(self.output),
             *( ["--run-id", "73"] if operation == "write" else [] ), *arguments],
            cwd=self.root, env=self.environment, check=False, capture_output=True, text=True, timeout=15,
        )

    def publish_receipt(self) -> dict:
        completed = self.run_evidence("write")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        receipt = json.loads(self.output.read_text(encoding="utf-8"))
        self.archive_receipt(receipt)
        self.state["run"].update(status="completed", conclusion="success")
        for job in self.state["jobs"]:
            if job["name"] in ("Landing gate", "Retain release qualification evidence"):
                job.update(status="completed", conclusion="success")
        return receipt

    def archive_receipt(self, receipt: dict, filename: str = "release-qualification-evidence.json") -> None:
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as packed:
            packed.writestr(filename, json.dumps(receipt, sort_keys=True).encode())
        self.archive_payload(archive.getvalue())

    def archive_payload(self, payload: bytes) -> None:
        self.state["archive"] = base64.b64encode(payload).decode()
        artifact = next((item for item in self.state["artifacts"]
                         if item["name"] == f"release-qualification-evidence-{self.sha}"), None)
        if artifact is None:
            artifact = self.add_artifact(f"release-qualification-evidence-{self.sha}")
        artifact.update(size_in_bytes=len(payload), digest="sha256:" + hashlib.sha256(payload).hexdigest())

    def assert_fallback(self, completed: subprocess.CompletedProcess[str]) -> dict:
        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertFalse(result["reused"])
        self.assertTrue(result["reason"])
        self.assertEqual(completed.stdout, "reused=false\nrun-id=\nrun-attempt=\n")
        self.assertNotIn("Traceback", completed.stderr)
        return result

    def test_write_records_the_complete_successful_qualification_before_gate_finishes(self) -> None:
        completed = self.run_evidence("write")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        receipt = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(receipt["source"]["sha"], self.sha)
        self.assertEqual(receipt["source"]["tree_sha"], self.tree)
        self.assertEqual(receipt["source"]["content_sha"], self.content_sha)
        self.assertEqual(receipt["source"]["version"], "1.2.3")
        self.assertEqual(receipt["builder"]["run_id"], 73)
        self.assertEqual(receipt["builder"]["run_attempt"], 2)
        self.assertEqual(len(receipt["jobs"]), 49)
        self.assertEqual(len(receipt["artifacts"]), 32)
        self.assertTrue(all(job["conclusion"] == "success" for job in receipt["jobs"]))

    def test_composite_gpu_actions_may_expose_the_verified_inner_step_name(self) -> None:
        for job in self.state["jobs"]:
            if " / Package " in job["name"]:
                job["steps"][0]["name"] = "Qualify every desktop backend"

        completed = self.run_evidence("write")

        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_resolve_reuses_only_the_complete_matching_successful_landing_receipt(self) -> None:
        receipt = self.publish_receipt()
        github_output = self.sandbox / "github-output"

        completed = self.run_evidence("resolve", "--github-output", str(github_output))

        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertTrue(result["reused"])
        self.assertEqual(result["run_id"], 73)
        self.assertEqual(result["receipt"], receipt)
        self.assertIn("reused=true\n", completed.stdout)
        self.assertIn("run-id=73\n", completed.stdout)
        self.assertIn("run-attempt=2\n", completed.stdout)
        self.assertEqual(github_output.read_text(encoding="utf-8"), "reused=true\nrun-id=73\nrun-attempt=2\n")

    def test_failed_advisory_metrics_do_not_invalidate_successful_qualification(self) -> None:
        self.add_job("CI latency report", status="completed", conclusion="failure")
        self.publish_receipt()
        completed = self.run_evidence("resolve")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(json.loads(self.output.read_text())["reused"])

    def test_explicit_latency_observability_never_supplies_required_qualification(self) -> None:
        reporter = self.add_job("CI latency report", status="queued", conclusion=None)
        self.publish_receipt()
        reporter.update(status="completed", conclusion="success")
        self.add_artifact("ci-latency-73-2")

        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(json.loads(self.output.read_text(encoding="utf-8"))["reused"])

    def test_retained_successful_producers_keep_their_original_build_attempt(self) -> None:
        self.publish_receipt()
        self.state["run"]["run_attempt"] = 3
        gate = next(job for job in self.state["jobs"] if job["name"] == "Landing gate")
        gate["run_attempt"] = 3

        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertTrue(result["reused"])
        self.assertEqual(result["run_attempt"], 2)
        self.assertIn("run-attempt=2\n", completed.stdout)

    def test_missing_job_attempts_are_bound_to_the_complete_per_attempt_inventory(self) -> None:
        for job in self.state["jobs"]:
            if job["name"].startswith("Linux / "):
                job["run_attempt"] = 1
        self.add_job("Windows / runtime and quality", run_attempt=1, conclusion="failure")
        self.state["attempt_jobs"] = {
            str(attempt): [copy.deepcopy(job) for job in self.state["jobs"] if job["run_attempt"] == attempt]
            for attempt in (1, 2)
        }
        for jobs in (self.state["jobs"], *self.state["attempt_jobs"].values()):
            for job in jobs:
                job.pop("run_attempt")

        receipt = self.publish_receipt()
        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertTrue(result["reused"])
        self.assertEqual(result["run_attempt"], 2)
        self.assertEqual(len(receipt["jobs"]), 49)
        self.assertEqual(len(receipt["artifacts"]), 32)
        self.assertEqual({job["run_attempt"] for job in receipt["jobs"]}, {1, 2})
        for job in receipt["jobs"]:
            self.assertEqual(job["run_attempt"], 1 if job["name"].startswith("Linux / ") else 2)
        calls = self.calls_path.read_text(encoding="utf-8")
        for attempt in (1, 2):
            self.assertIn(f"/attempts/{attempt}/jobs?per_page=100&page=1", calls)

    def test_every_job_inventory_requires_a_real_id_and_the_exact_run_and_source(self) -> None:
        self.state["attempt_jobs"] = {"1": [], "2": copy.deepcopy(self.state["jobs"])}
        self.state["jobs"][0].pop("run_attempt")
        self.state["attempt_jobs"]["2"][0].pop("run_attempt")
        original = copy.deepcopy(self.state)
        for collection in ("all", "attempt"):
            for changes in (
                {"run_id": 73.0}, {"run_id": 74}, {"run_id": None},
                {"head_sha": "0" * 40}, {"head_sha": None}, {"id": True}, {"id": None},
            ):
                with self.subTest(collection=collection, changes=changes):
                    self.state = copy.deepcopy(original)
                    jobs = self.state["jobs"] if collection == "all" else self.state["attempt_jobs"]["2"]
                    jobs[0].update(changes)
                    self.output.unlink(missing_ok=True)
                    completed = self.run_evidence("write")
                    self.assertEqual(completed.returncode, 1, completed.stderr)
                    self.assertFalse(self.output.exists())
                    self.assertNotIn("Traceback", completed.stderr)

    def test_ambiguous_missing_or_contradictory_attempt_membership_never_qualifies(self) -> None:
        self.publish_receipt()
        self.state["attempt_jobs"] = {"1": [], "2": copy.deepcopy(self.state["jobs"])}
        self.state["jobs"][0].pop("run_attempt")
        self.state["attempt_jobs"]["2"][0].pop("run_attempt")
        original = copy.deepcopy(self.state)
        for invalid in ("ambiguous", "missing", "extra", "all_attempt_disagrees", "endpoint_disagrees"):
            with self.subTest(membership=invalid):
                self.state = copy.deepcopy(original)
                attempt_jobs = self.state["attempt_jobs"]
                if invalid == "ambiguous":
                    attempt_jobs["1"].append(copy.deepcopy(attempt_jobs["2"][0]))
                elif invalid == "missing":
                    attempt_jobs["2"].pop(0)
                elif invalid == "extra":
                    attempt_jobs["2"].append({**attempt_jobs["2"][0], "id": 99001})
                elif invalid == "all_attempt_disagrees":
                    self.state["jobs"][1]["run_attempt"] = 1
                else:
                    attempt_jobs["2"][0]["run_attempt"] = 1
                self.output.unlink(missing_ok=True)
                completed = self.run_evidence("write")
                self.assertEqual(completed.returncode, 1, completed.stderr)
                self.assertFalse(self.output.exists())
                self.assert_fallback(self.run_evidence("resolve"))

    def test_a_first_execution_proves_missing_job_attempts_without_rerun_guessing(self) -> None:
        self.state["run"]["run_attempt"] = 1
        for job in self.state["jobs"]:
            job.pop("run_attempt")
        self.state["jobs"][0]["run_attempt"] = None

        receipt = self.publish_receipt()
        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertTrue(result["reused"])
        self.assertEqual(result["run_attempt"], 1)
        self.assertTrue(all(job["run_attempt"] == 1 for job in receipt["jobs"]))
        self.assertNotIn("/attempts/", self.calls_path.read_text(encoding="utf-8"))

    def test_api_derived_attempts_preserve_the_receipt_producer_after_a_gate_rerun(self) -> None:
        receipt = self.publish_receipt()
        self.state["run"]["run_attempt"] = 3
        self.add_job("Landing gate", run_attempt=3)
        self.state["attempt_jobs"] = {
            str(attempt): [copy.deepcopy(job) for job in self.state["jobs"] if job["run_attempt"] == attempt]
            for attempt in (1, 2, 3)
        }
        for jobs in (self.state["jobs"], *self.state["attempt_jobs"].values()):
            for job in jobs:
                job.pop("run_attempt")

        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertTrue(result["reused"])
        self.assertEqual(result["receipt"], receipt)
        self.assertEqual(result["run_attempt"], 2)
        self.assertTrue(all(job["run_attempt"] == 2 for job in result["receipt"]["jobs"]))
        self.assertIn("run-attempt=2\n", completed.stdout)
        self.assertIn("/attempts/3/jobs", self.calls_path.read_text(encoding="utf-8"))

    def test_attempt_inventories_are_completely_paginated_before_qualification(self) -> None:
        for index in range(110):
            self.add_job(f"Optional first-attempt diagnostic {index}", run_attempt=1)
        self.state["attempt_jobs"] = {
            str(attempt): [copy.deepcopy(job) for job in self.state["jobs"] if job["run_attempt"] == attempt]
            for attempt in (1, 2)
        }
        for jobs in (self.state["jobs"], *self.state["attempt_jobs"].values()):
            for job in jobs:
                job.pop("run_attempt")

        receipt = self.publish_receipt()
        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(json.loads(self.output.read_text(encoding="utf-8"))["reused"])
        self.assertEqual(len(receipt["jobs"]), 49)
        self.assertEqual(len(receipt["artifacts"]), 32)
        calls = [json.loads(line) for line in self.calls_path.read_text(encoding="utf-8").splitlines()]
        self.assertTrue(all(arguments[arguments.index("--method") + 1] == "GET" for arguments in calls))
        for endpoint in (
            "/jobs?filter=all&per_page=100&page=2", "/attempts/1/jobs?per_page=100&page=2",
        ):
            self.assertTrue(any(endpoint in argument for arguments in calls for argument in arguments), endpoint)

    def test_malformed_supplied_job_attempts_are_never_replaced_by_a_guess(self) -> None:
        self.publish_receipt()
        self.state["attempt_jobs"] = {"1": [], "2": copy.deepcopy(self.state["jobs"])}
        self.state["jobs"][0].pop("run_attempt")
        self.state["attempt_jobs"]["2"][0].pop("run_attempt")
        original = copy.deepcopy(self.state)
        for collection in ("all", "attempt"):
            for invalid in (0, -1, True, 2.0, "2", 3):
                with self.subTest(collection=collection, attempt=invalid):
                    self.state = copy.deepcopy(original)
                    jobs = self.state["jobs"] if collection == "all" else self.state["attempt_jobs"]["2"]
                    jobs[1]["run_attempt"] = invalid
                    self.output.unlink(missing_ok=True)
                    completed = self.run_evidence("write")
                    self.assertEqual(completed.returncode, 1, completed.stderr)
                    self.assertFalse(self.output.exists())
                    self.assert_fallback(self.run_evidence("resolve"))

    def test_an_excessive_run_attempt_count_is_rejected_before_inventory_requests(self) -> None:
        self.publish_receipt()
        self.state["run"]["run_attempt"] = 1_000_000_000
        self.calls_path.unlink()
        self.output.unlink()

        completed = self.run_evidence("write")

        self.assertEqual(completed.returncode, 1, completed.stderr)
        self.assertIn("bounded attempt limit", completed.stderr)
        self.assertFalse(self.output.exists())
        result = self.assert_fallback(self.run_evidence("resolve"))
        self.assertIn("bounded attempt limit", result["reason"])
        calls = self.calls_path.read_text(encoding="utf-8")
        self.assertNotIn("/jobs?", calls)
        self.assertNotIn("/attempts/", calls)

    def test_missing_producer_jobs_fail_without_guessing_an_api_inventory(self) -> None:
        required = [job for job in self.state["jobs"] if job["name"] not in (
            "Resolve release candidate", "Landing gate", "Retain release qualification evidence", "Pull request title",
        )]
        original = self.state["jobs"][:]
        self.assertEqual(len(required), 49)
        for job in required:
            with self.subTest(name=job["name"]):
                self.state["jobs"] = [entry for entry in original if entry["id"] != job["id"]]
                completed = self.run_evidence("write")
                self.assertEqual(completed.returncode, 1, completed.stderr)
                self.assertIn("required job inventory differs", completed.stderr)
        self.state["jobs"] = original

    def test_malformed_artifact_metadata_is_a_normal_cold_fallback(self) -> None:
        self.publish_receipt()
        original = copy.deepcopy(self.state["artifacts"][0])
        for field in ("expires_at", "workflow_run", "digest", "name"):
            with self.subTest(field=field):
                self.state["artifacts"][0] = {**original, field: None}
                self.assert_fallback(self.run_evidence("resolve"))

    def test_untrusted_or_malformed_runs_never_reuse_successful_artifacts(self) -> None:
        self.publish_receipt()
        original = copy.deepcopy(self.state["run"])
        invalid = (
            {"event": "workflow_dispatch"}, {"head_sha": "0" * 40},
            {"status": "in_progress"}, {"conclusion": "failure"}, {"path": None},
            {"repository": {"id": 999, "full_name": None}},
            {"head_repository": {"id": 998, "full_name": "fork-owner/clonk-rs"}},
            {"run_attempt": 0}, {"name": "Other workflow"},
        )
        for changes in invalid:
            with self.subTest(changes=changes):
                self.state["run"] = {**original, **changes}
                self.assert_fallback(self.run_evidence("resolve"))

    def test_every_required_artifact_must_exist_and_match_the_receipt(self) -> None:
        self.publish_receipt()
        original = copy.deepcopy(self.state["artifacts"])
        required = original[:-1]
        self.assertEqual(len(required), 32)
        for artifact in required:
            with self.subTest(name=artifact["name"]):
                self.state["artifacts"] = [item for item in original if item["id"] != artifact["id"]]
                self.assert_fallback(self.run_evidence("resolve"))
        self.state["artifacts"] = copy.deepcopy(original)
        for changes in (
            {"id": 99001}, {"digest": "sha256:" + "b" * 64}, {"size_in_bytes": 101},
            {"expired": True}, {"expires_at": "2000-01-01T00:00:00Z"},
            {"workflow_run": {"id": 73, "repository_id": 999, "head_repository_id": 998, "head_sha": self.sha}},
        ):
            with self.subTest(changes=changes):
                self.state["artifacts"][0] = {**original[0], **changes}
                self.assert_fallback(self.run_evidence("resolve"))

    def test_receipt_identity_cannot_substitute_different_source_recipe_or_attempt(self) -> None:
        original = self.publish_receipt()
        changes = (
            ("schema_version", 2), ("repository", None),
            ("repository", "fork-owner/clonk-rs"),
            ("source", {**original["source"], "sha": "0" * 40}),
            ("source", {**original["source"], "tree_sha": "0" * 40}),
            ("source", {**original["source"], "content_sha": "0" * 40}),
            ("source", {**original["source"], "version": "1.2.4"}),
            ("source", {**original["source"], "recipe_sha256": "0" * 64}),
            ("builder", {**original["builder"], "run_id": 74}),
            ("builder", {**original["builder"], "run_attempt": 1}),
            ("jobs", original["jobs"][:-1]), ("artifacts", original["artifacts"][:-1]),
        )
        for field, value in changes:
            with self.subTest(field=field, value=value):
                self.archive_receipt({**original, field: value})
                self.assert_fallback(self.run_evidence("resolve"))

    def test_receipt_archives_are_digest_bound_and_never_extract_unsafe_members(self) -> None:
        receipt = self.publish_receipt()
        original = base64.b64decode(self.state["archive"])
        outside = self.sandbox / "unexpected.json"
        self.archive_receipt(receipt, "../unexpected.json")
        self.assert_fallback(self.run_evidence("resolve"))
        self.assertFalse(outside.exists())
        self.state["archive"] = base64.b64encode(original + b"tampered").decode()
        self.assert_fallback(self.run_evidence("resolve"))
        variants = []
        for extra, symlink in ((True, False), (False, True)):
            archive = io.BytesIO()
            with zipfile.ZipFile(archive, "w") as packed:
                member = zipfile.ZipInfo("release-qualification-evidence.json")
                if symlink:
                    member.create_system = 3
                    member.external_attr = (stat.S_IFLNK | 0o777) << 16
                packed.writestr(member, json.dumps(receipt))
                if extra:
                    packed.writestr("other.json", "{}")
            variants.append(archive.getvalue())
        variants.append(b"not a zip")
        encrypted = bytearray(original)
        central = encrypted.index(b"PK\x01\x02")
        encrypted[6:8] = (1).to_bytes(2, "little")
        encrypted[central + 8:central + 10] = (1).to_bytes(2, "little")
        variants.append(bytes(encrypted))
        damaged = bytearray(original)
        data_offset = 30 + int.from_bytes(damaged[26:28], "little") + int.from_bytes(damaged[28:30], "little")
        damaged[data_offset] = 0xff
        variants.append(bytes(damaged))
        for payload in variants:
            with self.subTest(payload_size=len(payload)):
                self.archive_payload(payload)
                self.assert_fallback(self.run_evidence("resolve"))

    def test_package_jobs_require_a_successful_outer_or_composite_gpu_step(self) -> None:
        self.publish_receipt()
        package = next(job for job in self.state["jobs"] if job["name"].endswith(" / Package windows"))
        good = {"name": "Qualify the packaged runtime", "status": "completed", "conclusion": "success"}
        inner = {**good, "name": "Qualify every desktop backend"}
        package["steps"] = [good, inner]
        completed = self.run_evidence("resolve")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(json.loads(self.output.read_text(encoding="utf-8"))["reused"])
        for invalid in ([], None, [None], [{**good, "conclusion": "skipped"}],
                        [{**good, "status": "in_progress"}], [good, {**inner, "conclusion": "failure"}]):
            with self.subTest(steps=invalid):
                package["steps"] = invalid
                self.assert_fallback(self.run_evidence("resolve"))

    def test_every_api_inventory_page_is_read_using_only_get_requests(self) -> None:
        for index in range(110):
            self.add_job(f"Optional diagnostic {index}")
            self.add_artifact(f"optional-diagnostic-{index}")
        receipt = self.publish_receipt()

        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(json.loads(self.output.read_text(encoding="utf-8"))["reused"])
        self.assertEqual(len(receipt["artifacts"]), 142)
        calls = [json.loads(line) for line in self.calls_path.read_text(encoding="utf-8").splitlines()]
        self.assertTrue(all(arguments[arguments.index("--method") + 1] == "GET" for arguments in calls))
        for endpoint in ("jobs?filter=all", "artifacts?per_page"):
            self.assertTrue(any(endpoint in argument and "page=2" in argument
                                for arguments in calls for argument in arguments), endpoint)

    def test_actual_source_and_materialized_content_must_still_match_the_clean_recipe(self) -> None:
        self.publish_receipt()
        paths = ("Cargo.toml", ".github/workflows/landing.yml", "qualification-recipe.py", "content/Scenario.txt")
        for relative in paths:
            with self.subTest(path=relative):
                path = self.root / relative
                original = path.read_bytes()
                path.write_bytes(original + b"# changed qualification input\n")
                os.utime(path, (1, 1))
                self.assert_fallback(self.run_evidence("resolve"))
                path.write_bytes(original)
        content = self.root / "content"
        ignored = content / "ignored-parity-input.txt"
        exclude = Path(self.git(content, "rev-parse", "--path-format=absolute", "--git-path", "info/exclude").stdout.strip())
        exclude.write_text(exclude.read_text(encoding="utf-8") + "\nignored-parity-input.txt\n", encoding="utf-8")
        ignored.write_text("unexpected materialized content\n", encoding="utf-8")
        self.assert_fallback(self.run_evidence("resolve"))
        ignored.unlink()
        unexpected = self.root / "new-qualification-source.py"
        unexpected.write_text("unexpected = True\n", encoding="utf-8")
        self.assert_fallback(self.run_evidence("resolve"))
        unexpected.unlink()
        (self.root / "qualification-recipe.py").write_text("required = False\n", encoding="utf-8")
        self.git(self.root, "add", "qualification-recipe.py")
        self.git(self.root, "commit", "--quiet", "-m", "test: change qualification recipe")
        self.sha = self.git(self.root, "rev-parse", "HEAD").stdout.strip()
        self.assert_fallback(self.run_evidence("resolve"))

    def test_a_successful_outer_run_never_masks_missing_skipped_or_failed_controls(self) -> None:
        self.publish_receipt()
        original = copy.deepcopy(self.state["jobs"])
        for name in ("Landing gate", "Retain release qualification evidence", "Linux / app 1/12"):
            for status, conclusion in (("queued", None), ("completed", "skipped"), ("completed", "failure")):
                with self.subTest(name=name, status=status, conclusion=conclusion):
                    self.state["jobs"] = copy.deepcopy(original)
                    job = next(item for item in self.state["jobs"] if item["name"] == name)
                    job.update(status=status, conclusion=conclusion)
                    self.assert_fallback(self.run_evidence("resolve"))
            self.state["jobs"] = [item for item in original if item["name"] != name]
            self.assert_fallback(self.run_evidence("resolve"))

    def test_writer_in_actions_must_be_the_actual_landing_producer_attempt(self) -> None:
        producer = {
            "GITHUB_ACTIONS": "true", "GITHUB_REPOSITORY": REPOSITORY, "GITHUB_RUN_ID": "73",
            "GITHUB_RUN_ATTEMPT": "2", "GITHUB_SHA": self.sha, "GITHUB_WORKFLOW": "Landing",
            "GITHUB_EVENT_NAME": "merge_group",
        }
        self.environment.update(producer)
        completed = self.run_evidence("write")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        for field, wrong in (
            ("GITHUB_RUN_ID", "74"), ("GITHUB_RUN_ATTEMPT", "3"), ("GITHUB_SHA", "0" * 40),
            ("GITHUB_REPOSITORY", "fork-owner/clonk-rs"), ("GITHUB_EVENT_NAME", "workflow_dispatch"),
            ("GITHUB_WORKFLOW", "Main"),
        ):
            with self.subTest(field=field):
                self.environment.update(producer)
                self.environment[field] = wrong
                completed = self.run_evidence("write")
                self.assertEqual(completed.returncode, 1, completed.stderr)
                self.assertIn("producer environment", completed.stderr)

    def test_api_errors_and_missing_receipts_fall_back_with_empty_github_identity(self) -> None:
        self.publish_receipt()
        github_output = self.sandbox / "github-output"
        self.environment["FAKE_GH_FAILURE"] = "1"
        result = self.assert_fallback(self.run_evidence("resolve", "--github-output", str(github_output)))
        self.assertIn("simulated GitHub API error", result["reason"])
        self.assertEqual(github_output.read_text(encoding="utf-8"), "reused=false\nrun-id=\nrun-attempt=\n")
        del self.environment["FAKE_GH_FAILURE"]
        self.state["runs"] = []
        self.assert_fallback(self.run_evidence("resolve"))
        del self.state["runs"]
        self.state["artifacts"].pop()
        self.assert_fallback(self.run_evidence("resolve"))

    def test_unverifiable_newer_candidates_do_not_hide_an_older_complete_exact_sha_run(self) -> None:
        self.publish_receipt()
        # Listings can race deletions or reruns. The authoritative run endpoint
        # still identifies only the complete original producer in this fixture.
        self.state["runs"] = [{**self.state["run"], "id": 100 + index} for index in range(12)]
        self.state["runs"].append(copy.deepcopy(self.state["run"]))

        completed = self.run_evidence("resolve")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertTrue(result["reused"])
        self.assertEqual(result["run_id"], 73)

    def test_resolver_run_id_hint_restricts_validation_to_that_authoritative_producer(self) -> None:
        self.publish_receipt()
        completed = self.run_evidence("resolve", "--run-id", "73")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(json.loads(self.output.read_text(encoding="utf-8"))["reused"])
        for hint in ("74", "0", "-1"):
            with self.subTest(hint=hint):
                self.calls_path.unlink(missing_ok=True)
                self.assert_fallback(self.run_evidence("resolve", "--run-id", hint))
                calls = self.calls_path.read_text(encoding="utf-8") if self.calls_path.exists() else ""
                self.assertNotIn("/workflows/landing.yml/runs", calls)
                self.assertNotIn("/actions/runs/73", calls)


if __name__ == "__main__":
    unittest.main()
