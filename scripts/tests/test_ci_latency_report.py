"""Latency reporting uses recorded GitHub responses, never the network."""

import json
import copy
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta, timezone


SCRIPT = Path(__file__).resolve().parents[1] / "ci-latency-report.py"
REPOSITORY = "clonk-org/clonk-rs"
PREFIX = "repos/clonk-org/clonk-rs/actions"
PREPARE_PULLS = "repos/clonk-org/clonk-rs/pulls?state=all&head=clonk-org%3Arelease%2Fnext&base=main"
EPOCH = datetime(2026, 10, 6, tzinfo=timezone.utc)


def stamp(seconds):
    return (EPOCH + timedelta(seconds=seconds)).isoformat().replace("+00:00", "Z")


def run_record(identifier=900, **changes):
    return {
        "id": identifier, "workflow_id": 41, "event": "merge_group",
        "head_sha": "a" * 40, "head_branch": "gh-readonly-queue/main/pr-1-base",
        "html_url": f"https://github.com/clonk-org/clonk-rs/actions/runs/{identifier}",
        "run_attempt": 1, "status": "completed", "conclusion": "success",
        "created_at": stamp(0), "run_started_at": stamp(60), "updated_at": stamp(760),
        **changes,
    }


def job_record(identifier=1, name="Linux tests", start=60, end=760, **changes):
    return {
        "id": identifier, "name": name, "run_attempt": 1,
        "status": "completed", "conclusion": "success", "created_at": stamp(0),
        "started_at": stamp(start), "completed_at": stamp(end), "steps": [],
        **changes,
    }


def pull_record(identifier=40, run_id=900, created=100, **changes):
    return {
        "id": identifier, "number": identifier, "state": "open", "created_at": stamp(created),
        "merged_at": None, "merge_commit_sha": None, "title": "chore: release 1.2.3",
        "html_url": f"https://github.com/{REPOSITORY}/pull/{identifier}",
        "body": f"Prepared by workflow run https://github.com/{REPOSITORY}/actions/runs/{run_id}.",
        "head": {"ref": "release/next", "repo": {"full_name": REPOSITORY}},
        "base": {"ref": "main", "repo": {"full_name": REPOSITORY}}, **changes,
    }


class LatencyReportTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="clonk-ci-latency-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.fixtures = {
            f"{PREFIX}/runs/900": run_record(),
            f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1": {
                "total_count": 1, "jobs": [job_record()],
            },
            f"{PREFIX}/workflows/41/runs?status=completed&per_page=20&page=1": {
                "total_count": 0, "workflow_runs": [],
            },
        }
        self.calls = self.directory / "calls.jsonl"
        fake = self.directory / "gh"
        fake.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "endpoint = next(arg for arg in sys.argv[1:] if arg.startswith('repos/'))\n"
            "with Path(os.environ['FAKE_GH_CALLS']).open('a') as target:\n"
            "    target.write(json.dumps(sys.argv[1:]) + '\\n')\n"
            "fixture = json.loads(Path(os.environ['FAKE_GH_FIXTURES']).read_text()).get(endpoint)\n"
            "if fixture is None or (isinstance(fixture, dict) and 'api_error' in fixture):\n"
            "    print('fake gh API failure: ' + endpoint, file=sys.stderr)\n"
            "    sys.exit(42)\n"
            "print(json.dumps(fixture))\n",
            encoding="utf-8",
        )
        fake.chmod(0o755)
        self.environment = {
            **os.environ, "PATH": str(self.directory) + os.pathsep + os.environ["PATH"],
            "FAKE_GH_FIXTURES": str(self.directory / "fixtures.json"),
            "FAKE_GH_CALLS": str(self.calls), "GITHUB_STEP_SUMMARY": "",
        }

    def invoke(self, *arguments):
        Path(self.environment["FAKE_GH_FIXTURES"]).write_text(json.dumps(self.fixtures), encoding="utf-8")
        output = self.directory / "report.json"
        process = subprocess.run(
            [sys.executable, str(SCRIPT), "--repository", "clonk-org/clonk-rs",
             "--run-id", "900", "--phase", "landing", "--output", str(output), *arguments],
            capture_output=True, text=True, env=self.environment, timeout=20,
        )
        return process, json.loads(output.read_text()) if output.exists() else None

    def publication(self, identifier=900, merged=-10, published=80, version="1.2.3", source="a" * 40):
        record = self.fixtures[f"{PREFIX}/runs/{identifier}"]
        record.update(event="push", head_sha=source, head_commit={"id": source, "message": f"chore: release {version}\n\nSquashed subjects"})
        self.fixtures[f"repos/{REPOSITORY}/commits/{source}/pulls?per_page=100&page=1"] = [
            pull_record(identifier=identifier + 40, run_id=identifier, created=merged - 50,
                        state="closed", merged_at=stamp(merged), merge_commit_sha=source),
        ]
        self.fixtures[f"repos/{REPOSITORY}/releases/tags/v{version}"] = {
            "id": identifier + 80, "tag_name": f"v{version}", "target_commitish": "main", "draft": False,
            "html_url": f"https://github.com/{REPOSITORY}/releases/tag/v{version}",
            "created_at": stamp(published - 10), "published_at": stamp(published),
        }
        tag_object = f"{identifier:040x}"
        self.fixtures[f"repos/{REPOSITORY}/git/ref/tags/v{version}"] = {
            "ref": f"refs/tags/v{version}", "object": {"type": "tag", "sha": tag_object},
        }
        self.fixtures[f"repos/{REPOSITORY}/git/tags/{tag_object}"] = {
            "tag": f"v{version}", "sha": tag_object, "object": {"type": "commit", "sha": source},
        }
        jobs = self.fixtures[f"{PREFIX}/runs/{identifier}/jobs?filter=all&per_page=100&page=1"]["jobs"]
        jobs[0]["steps"] = [{"name": "Publish the release", "status": "completed", "conclusion": "success",
                             "started_at": stamp(published - 5), "completed_at": stamp(published)}]

    def test_slow_success_is_success_with_an_advisory_warning(self):
        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["schema"], 1)
        self.assertEqual(report["functional_conclusion"], "success")
        self.assertEqual(report["latency"]["execution_seconds"], 700)
        self.assertEqual(report["latency"]["elapsed_seconds"], 760)
        self.assertEqual(report["latency"]["initial_queue_seconds"], 60)
        self.assertEqual(report["latency"]["slo"]["class"], "ordinary")
        self.assertFalse(report["latency"]["slo"]["met"])

    def test_skipped_job_has_no_execution_clock_even_when_github_reverses_its_timestamps(self):
        endpoint = f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"
        skipped = job_record(2, "Optional report", start=1001, end=1000,
                             conclusion="skipped", created_at=stamp(1002), steps=[])
        self.fixtures[endpoint]["jobs"].append(skipped)
        self.fixtures[endpoint]["total_count"] = 2

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["functional_conclusion"], "success")
        self.assertEqual(report["latency"]["elapsed_seconds"], 760)
        retained = next(job for job in report["jobs"] if job["name"] == "Optional report")
        self.assertIsNone(retained["duration_seconds"])
        self.assertIsNone(retained["queue_seconds"])
        self.assertEqual(retained["started_at"], stamp(1001))
        self.assertIn("::warning::", process.stdout)
        self.assertEqual(report["cache_state"], "UNKNOWN")

    def test_all_job_pages_and_prebuilds_define_the_release_candidate(self):
        endpoint = f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page="
        for page, job in enumerate([
            job_record(2, "Release prebuild / Build runtime (linux)", 30, 620),
            job_record(3, "Release build / Package (linux)", 625, 715),
            job_record(4, "Landing gate", 716, 720),
        ], 1):
            self.fixtures[endpoint + str(page)] = {"total_count": 3, "jobs": [job]}

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["execution_seconds"], 690)
        self.assertEqual(report["latency"]["elapsed_seconds"], 720)
        self.assertEqual(report["latency"]["slo"]["class"], "release")
        self.assertEqual([job["id"] for job in report["jobs"]], [2, 3, 4])
        self.assertIn(endpoint + "3", self.calls.read_text())

    def test_a_failed_qualification_job_cannot_be_reported_as_success(self):
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"][0]["conclusion"] = "failure"

        process, report = self.invoke()

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(report["functional_conclusion"], "failure")

    def test_optional_postmerge_cache_retention_failure_keeps_its_outcome_and_cost(self):
        workflow = (SCRIPT.parent.parent / ".github/workflows/rust.yml").read_text(encoding="utf-8")
        header = workflow.split("  cache-retention:\n", 1)[1].split("    steps:\n", 1)[0]
        self.assertIn("name: Bound trusted workspace caches", header)
        self.assertIn("continue-on-error: true", header)
        jobs = [
            job_record(1, "Windows release tooling", 60, 200),
            job_record(2, "Bound trusted workspace caches", 150, 220, conclusion="failure"),
        ]
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(event="push")
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": len(jobs), "jobs": jobs,
        }

        summary = self.directory / "optional-retention-summary.md"
        process, report = self.invoke("--phase", "postmerge", "--summary", str(summary))

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["functional_conclusion"], "success")
        self.assertEqual(report["functional_excluded_job_ids"], [2])
        self.assertEqual(report["jobs"][1]["conclusion"], "failure")
        self.assertEqual(report["jobs"][1]["duration_seconds"], 70)
        self.assertEqual(report["latency"]["runner_active_seconds"], 210)
        self.assertEqual(report["latency"]["peak_runner_concurrency"], 2)
        self.assertEqual(report["latency"]["execution_seconds"], 160)
        self.assertEqual(report["attempts"][0]["functional_conclusion"], "success")
        self.assertIn("Optional maintenance excluded from functional conclusion: **2**", summary.read_text())
        self.assertIn("| Bound trusted workspace caches | 1 | failure | 70.0 |", summary.read_text())

    def test_other_required_cache_failures_and_landing_jobs_are_not_excluded(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(event="push")
        for phase, name in (
            ("postmerge", "Warm the Linux landing cache"),
            ("postmerge", "Bound CI cache storage"),
            ("landing", "Bound trusted workspace caches"),
        ):
            with self.subTest(phase=phase, name=name):
                self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
                    "total_count": 2, "jobs": [
                        job_record(1, "Windows release tooling", 60, 200),
                        job_record(2, name, 150, 220, conclusion="failure"),
                    ],
                }

                process, report = self.invoke("--phase", phase)

                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(report["functional_conclusion"], "failure")
                self.assertEqual(report["functional_excluded_job_ids"], [])
                self.assertEqual(report["attempts"][0]["functional_conclusion"], "failure")

    def test_live_postmerge_report_keeps_success_when_only_optional_retention_failed(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(event="push", status="in_progress", conclusion=None)
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": 2, "jobs": [
                job_record(1, "Windows release tooling", 60, 200),
                job_record(2, "Bound trusted workspace caches", 150, 220, conclusion="failure"),
            ],
        }

        process, report = self.invoke("--phase", "postmerge", "--allow-running", "true")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["functional_conclusion"], "success")
        self.assertEqual(report["functional_excluded_job_ids"], [2])
        self.assertTrue(report["qualification_complete"])
        self.assertEqual(report["latency"]["peak_runner_concurrency"], 2)

    def test_a_rerun_preserves_original_creation_and_supersedes_only_retried_jobs(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(
            run_attempt=2, run_started_at=stamp(600), updated_at=stamp(950),
        )
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": 3, "jobs": [
                job_record(1, "Linux tests", 60, 260, conclusion="failure"),
                job_record(2, "Linux tests", 650, 950, run_attempt=2, created_at=stamp(600)),
                job_record(3, "Windows tests", 60, 400),
            ],
        }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["run_attempt"], 2)
        self.assertEqual(report["original_created_at"], stamp(0))
        self.assertEqual(report["latency"]["elapsed_seconds"], 950)
        self.assertEqual(report["latency"]["execution_seconds"], 890)
        self.assertEqual([attempt["functional_conclusion"] for attempt in report["attempts"]], ["failure", "success"])
        self.assertTrue(report["jobs"][0]["superseded"])
        self.assertFalse(report["jobs"][2]["superseded"])

    def test_overlapping_runner_work_is_not_added_to_wall_clock_execution(self):
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": 3, "jobs": [
                job_record(1, "Context", 60, 90),
                job_record(2, "Linux tests", 100, 200, created_at=stamp(90)),
                job_record(3, "Windows tests", 150, 250, created_at=stamp(100)),
            ],
        }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["execution_seconds"], 190)
        self.assertEqual(report["latency"]["active_seconds"], 180)
        self.assertEqual(report["latency"]["waiting_seconds"], 70)
        self.assertEqual(report["latency"]["runner_active_seconds"], 230)
        self.assertEqual(report["latency"]["peak_runner_concurrency"], 2)
        self.assertEqual(report["latency"]["job_queue_seconds_sum"], 120)
        self.assertIsNone(report["latency"]["dependency_wait_seconds"])
        self.assertIsNone(report["latency"]["runner_wait_seconds"])

    def test_peak_concurrency_uses_positive_half_open_execution_intervals(self):
        scenarios = (
            ("sequential", [(60, 90), (100, 200)], 1),
            ("endpoint_adjacent", [(60, 100), (100, 200)], 1),
            ("grouped_edges", [(100, 170), (70, 100), (100, 160), (60, 100)], 2),
            ("zero_only", [(60, 60)], 0),
            ("zero_during_execution", [(100, 100), (60, 200)], 1),
        )
        endpoint = f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"
        for name, intervals, expected in scenarios:
            with self.subTest(scenario=name):
                jobs = [job_record(index, f"Job {index}", start, end)
                        for index, (start, end) in enumerate(intervals, 1)]
                jobs.extend((
                    job_record(90, "Skipped zero", 100, 100, conclusion="skipped"),
                    job_record(91, "Skipped reversed", 1001, 1000,
                               created_at=stamp(1002), conclusion="skipped"),
                ))
                self.fixtures[endpoint] = {"total_count": len(jobs), "jobs": jobs}

                process, report = self.invoke()

                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(report["latency"]["peak_runner_concurrency"], expected)
                self.assertEqual(report["latency"]["runner_active_seconds"], sum(end - start for start, end in intervals))
                self.assertIsNone(report["jobs"][-1]["duration_seconds"])

    def test_peak_concurrency_includes_superseded_attempts_and_the_original_clock(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(
            run_attempt=2, run_started_at=stamp(600), updated_at=stamp(700),
        )
        jobs = [
            job_record(1, "Linux tests", 60, 200, conclusion="failure"),
            job_record(2, "Windows tests", 60, 260, conclusion="failure"),
            job_record(3, "macOS tests", 60, 280, conclusion="failure"),
            job_record(4, "Linux tests", 600, 620, run_attempt=2, created_at=stamp(600)),
            job_record(5, "Windows tests", 620, 650, run_attempt=2, created_at=stamp(600)),
            job_record(6, "macOS tests", 650, 700, run_attempt=2, created_at=stamp(600)),
        ]
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": len(jobs), "jobs": jobs,
        }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["peak_runner_concurrency"], 3)
        self.assertEqual(report["latency"]["initial_queue_seconds"], 60)
        self.assertEqual(report["latency"]["execution_seconds"], 640)
        self.assertEqual(report["latency"]["elapsed_seconds"], 700)
        self.assertEqual([job["superseded"] for job in report["jobs"]], [True] * 3 + [False] * 3)

    def test_missing_job_creation_time_remains_unknown_despite_known_concurrency(self):
        jobs = [job_record(1, "Linux tests", 60, 200), job_record(2, "Windows tests", 80, 220)]
        for job in jobs:
            del job["created_at"]
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": len(jobs), "jobs": jobs,
        }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["peak_runner_concurrency"], 2)
        self.assertIsNone(report["latency"]["job_queue_seconds_sum"])
        self.assertTrue(all(job["queue_seconds"] is None for job in report["jobs"]))
        self.assertIsNone(report["latency"]["dependency_wait_seconds"])
        self.assertIsNone(report["latency"]["runner_wait_seconds"])

    def test_reversed_executing_job_interval_cannot_report_a_capacity_peak(self):
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"] = [
            job_record(start=100, end=90),
        ]

        process, report = self.invoke()

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(report["functional_conclusion"], "error")
        self.assertIn("timestamps are out of order", report["error"])
        self.assertNotIn("latency", report)

    def test_running_reporter_and_skipped_optional_jobs_do_not_extend_qualification(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(status="in_progress", conclusion=None)
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": 3, "jobs": [
                job_record(),
                job_record(2, "Release build / Package", status="completed", conclusion="skipped",
                           started_at=None, completed_at=None),
                job_record(3, "CI latency report", 760, status="in_progress", conclusion=None,
                           completed_at=None),
            ],
        }

        process, report = self.invoke("--allow-running", "true", "--release", "false")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["functional_conclusion"], "success")
        self.assertEqual(report["run_status"], "in_progress")
        self.assertTrue(report["qualification_complete"])
        self.assertEqual(report["latency"]["execution_seconds"], 700)
        self.assertEqual(report["excluded_reporting_job_ids"], [3])
        self.assertEqual(report["latency"]["slo"]["class"], "ordinary")

    def test_step_timings_distinguish_setup_build_test_package_export_and_transfers(self):
        job = self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"][0]
        cursor = 60
        names = ["Set up Rust", "Build runtime", "Run nextest", "Package bundles",
                 "Export qualification manifest", "Upload runtime", "Download tools", "Run actions/cache"]
        for number, name in enumerate(names, 1):
            job["steps"].append({
                "number": number, "name": name, "status": "completed", "conclusion": "success",
                "started_at": stamp(cursor), "completed_at": stamp(cursor + number),
            })
            cursor += number

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["jobs"][0]["step_seconds"], {
            "setup": 9, "build": 2, "test": 3, "package": 4,
            "export": 5, "upload": 6, "download": 7, "other": 0,
        })
        self.assertEqual(report["jobs"][0]["cache_state"], "UNKNOWN")
        self.assertEqual(report["jobs"][0]["step_classification_source"], "step_name")

    def test_history_pagination_compares_events_and_reports_success_percentiles_and_failures(self):
        historical = [run_record()]
        examples = [
            (901, 100, "success", "Linux tests", "merge_group"),
            (902, 200, "success", "Linux tests", "merge_group"),
            (903, 300, "success", "Linux tests", "merge_group"),
            (904, 400, "success", "Linux tests", "merge_group"),
            (905, 50, "failure", "Linux tests", "merge_group"),
            (906, 30, "cancelled", "Linux tests", "merge_group"),
            (907, 20, "neutral", "Linux tests", "merge_group"),
            (908, 900, "success", "Release prebuild / Build runtime", "merge_group"),
            (909, 5, "success", "Linux tests", "push"),
        ]
        for identifier, duration, conclusion, name, event in examples:
            historical.append(run_record(identifier, conclusion=conclusion, event=event, updated_at=stamp(60 + duration)))
            self.fixtures[f"{PREFIX}/runs/{identifier}/jobs?filter=all&per_page=100&page=1"] = {
                "total_count": 1, "jobs": [job_record(identifier * 10, name, 60, 60 + duration, conclusion=conclusion)],
            }
        for page, batch in enumerate([historical[:5], historical[5:]], 1):
            self.fixtures[f"{PREFIX}/workflows/41/runs?status=completed&per_page=20&page={page}"] = {
                "total_count": 10, "workflow_runs": batch,
            }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        ordinary = report["history"]["ordinary"]
        self.assertEqual(ordinary["sample_count"], 4)
        self.assertEqual(ordinary["completed_runs"], 7)
        self.assertEqual(ordinary["p50_seconds"], 260)
        self.assertEqual(ordinary["p95_seconds"], 460)
        self.assertEqual(ordinary["execution_p50_seconds"], 200)
        self.assertEqual(ordinary["execution_p95_seconds"], 400)
        self.assertEqual(ordinary["conclusions"], {"success": 4, "failure": 1, "cancelled": 1, "error": 0, "unknown": 1})
        self.assertEqual(ordinary["cache_states"], {"warm": 0, "cold": 0, "UNKNOWN": 7})
        self.assertEqual(report["history"]["release"]["sample_count"], 1)
        self.assertEqual(report["history"]["release"]["execution_p95_seconds"], 900)
        self.assertNotIn(f"runs/909/jobs", self.calls.read_text())

    def test_rerun_job_membership_is_resolved_when_all_jobs_omit_attempt_metadata(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(run_attempt=2, updated_at=stamp(950))
        jobs = [job_record(1, "Linux tests", 60, 200, conclusion="failure"),
                job_record(2, "Linux tests", 650, 950, created_at=stamp(600))]
        for job in jobs:
            del job["run_attempt"]
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {"total_count": 2, "jobs": jobs}
        for attempt, job in enumerate(jobs, 1):
            self.fixtures[f"{PREFIX}/runs/900/attempts/{attempt}/jobs?per_page=100&page=1"] = {
                "total_count": 1, "jobs": [job],
            }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual([job["run_attempt"] for job in report["jobs"]], [1, 2])
        self.assertEqual(report["latency"]["elapsed_seconds"], 950)
        self.assertIn("attempts/1/jobs", self.calls.read_text())
        for job in jobs:
            job["run_attempt"] = None
        process, report = self.invoke()
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual([job["run_attempt"] for job in report["jobs"]], [1, 2])

    def test_cache_temperature_requires_an_explicit_successful_receipt(self):
        job = self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"][0]
        job["steps"] = [{
            "name": "CI cache receipt: warm", "status": "completed", "conclusion": "success",
            "started_at": stamp(60), "completed_at": stamp(60),
        }]

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["jobs"][0]["cache_state"], "warm")
        self.assertEqual(report["cache_state"], "warm")
        self.assertEqual(report["jobs"][0]["cache_evidence"], "explicit_successful_receipt_step")

    def test_cache_temperature_rejects_incomplete_conflicting_and_unsupported_receipts(self):
        def receipt(name, status="completed", conclusion="success"):
            return {"name": name, "status": status, "conclusion": conclusion,
                    "started_at": stamp(60), "completed_at": stamp(60)}

        job = self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"][0]
        for steps, expected in (
            ([receipt("CI cache receipt: cold")], "cold"),
            ([receipt("CI cache receipt: warm", conclusion="failure")], "UNKNOWN"),
            ([receipt("CI cache receipt: warm", conclusion="skipped")], "UNKNOWN"),
            ([receipt("CI cache receipt: warm", status="in_progress", conclusion=None)], "UNKNOWN"),
            ([receipt("CI cache receipt: warm"), receipt("CI cache receipt: cold")], "UNKNOWN"),
            ([receipt("CI cache receipt: warm (registry only)")], "UNKNOWN"),
            ([receipt("Swatinem/rust-cache cache-hit=true"),
              receipt("Build runtime (cache hit)", conclusion="skipped")], "UNKNOWN"),
        ):
            with self.subTest(steps=steps):
                job["steps"] = steps

                process, report = self.invoke()

                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(report["cache_state"], expected)
                self.assertEqual(report["jobs"][0]["cache_state"], expected)
                self.assertEqual(
                    report["jobs"][0]["cache_evidence"],
                    "explicit_successful_receipt_step" if expected != "UNKNOWN" else None,
                )

    def test_history_limit_is_bounded_before_any_api_request(self):
        for limit in ("0", "-1", "101"):
            with self.subTest(limit=limit):
                process, report = self.invoke("--history-limit", limit)
                self.assertEqual(process.returncode, 2)
                self.assertIsNone(report)
                self.assertFalse(self.calls.exists())

    def test_malformed_metadata_timestamps_fail_even_when_they_are_not_finish_anchors(self):
        for field, value in (("updated_at", "bad"), ("run_started_at", "2026-10-06T00:00:00")):
            with self.subTest(field=field):
                self.fixtures[f"{PREFIX}/runs/900"] = run_record(**{field: value})
                process, report = self.invoke()
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(report["functional_conclusion"], "error")
                self.assertIn("timestamp", report["error"])

    def test_empty_optional_queue_timestamp_is_malformed_not_unknown(self):
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"][0]["created_at"] = ""

        process, report = self.invoke()

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(report["functional_conclusion"], "error")
        self.assertIn("timestamp", report["error"])

    def test_malformed_job_identity_produces_an_error_report(self):
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"][0]["id"] = []

        process, report = self.invoke()

        self.assertNotEqual(process.returncode, 0)
        self.assertIsNotNone(report)
        self.assertEqual(report["functional_conclusion"], "error")

    def test_api_failure_on_a_later_job_page_is_never_a_complete_report(self):
        endpoint = f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page="
        self.fixtures[endpoint + "1"]["total_count"] = 2
        self.fixtures[endpoint + "2"] = {"api_error": "unavailable"}

        process, report = self.invoke()

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(report["functional_conclusion"], "error")
        self.assertIn("page=2", report["error"])

    def test_unknown_workflow_status_cannot_use_running_mode_to_appear_successful(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(status="unrecognized", conclusion=None)

        process, report = self.invoke("--allow-running", "true")

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(report["functional_conclusion"], "error")

    def test_running_qualification_is_unknown_and_cannot_claim_its_slo(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(status="in_progress", conclusion=None)
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": 2, "jobs": [
                job_record(end=200),
                job_record(2, "Windows tests", 300, status="in_progress", conclusion=None, completed_at=None),
            ],
        }

        process, report = self.invoke("--allow-running", "true")

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(report["functional_conclusion"], "unknown")
        self.assertFalse(report["qualification_complete"])
        self.assertIsNone(report["latency"]["slo"]["met"])
        self.assertNotIn("::warning::", process.stdout)

    def test_summary_keeps_queue_and_execution_clocks_and_sample_counts_visible(self):
        summary = self.directory / "summary.md"
        summary.write_text("Existing summary\n", encoding="utf-8")

        process, report = self.invoke("--summary", str(summary))

        self.assertEqual(process.returncode, 0, process.stderr)
        rendered = summary.read_text()
        self.assertTrue(rendered.startswith("Existing summary\n"))
        self.assertIn("Initial queue", rendered)
        self.assertIn("p50", rendered)
        self.assertIn("p95", rendered)
        self.assertIn("Sample count", rendered)
        self.assertIn("Linux tests", rendered)
        self.assertIn("UNKNOWN", rendered)
        self.assertIn("runner-vs-dependency", rendered)
        self.assertIn("Peak observed runner concurrency: **1**", rendered)
        self.assertIn("completed positive-duration job intervals across all attempts", rendered)

    def test_phase_selects_the_named_slo_clock_even_with_zero_queue_delay(self):
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"] = [job_record(start=0, end=120)]
        for phase in ("landing", "postmerge"):
            with self.subTest(phase=phase):
                process, report = self.invoke("--phase", phase, "--release", "true")
                self.assertEqual(process.returncode, 0, process.stderr)
                slo = report["latency"]["slo"]
                self.assertEqual(slo["class"], "release")
                self.assertEqual(slo["target_seconds"], 600)
                self.assertEqual(slo["measurement"], "execution_seconds")
                self.assertTrue(slo["met"])
                self.assertNotIn("::warning::", process.stdout)

    def test_prepare_uses_original_dispatch_creation_to_its_exact_release_pull_request(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(event="workflow_dispatch", run_attempt=2)
        self.fixtures[PREPARE_PULLS + "&per_page=100&page=1"] = [pull_record(created=100)]
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]["jobs"][0]["run_attempt"] = 2

        process, report = self.invoke("--phase", "prepare", "--release", "true")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["elapsed_seconds"], 760)
        self.assertEqual(report["latency"]["operation_seconds"], 100)
        self.assertEqual(report["operation"]["clock"], "workflow_created_at_to_pull_request_created_at")
        self.assertEqual(report["operation"]["start_at"], stamp(0))
        self.assertEqual(report["operation"]["end_at"], stamp(100))
        self.assertEqual(report["operation"]["pull_request"]["number"], 40)
        self.assertEqual(report["latency"]["slo"]["measurement"], "operation_seconds")
        self.assertTrue(report["latency"]["slo"]["met"])
        self.assertNotIn("::warning::", process.stdout)

    def test_publication_uses_unique_merge_to_release_publication_and_the_resolved_tag_source(self):
        self.publication()

        process, report = self.invoke("--phase", "publication", "--release", "true")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["elapsed_seconds"], 760)
        self.assertEqual(report["latency"]["operation_seconds"], 90)
        self.assertEqual(report["operation"]["clock"], "pull_request_merged_at_to_release_published_at")
        self.assertEqual(report["operation"]["start_at"], stamp(-10))
        self.assertEqual(report["operation"]["end_at"], stamp(80))
        self.assertEqual(report["operation"]["release"]["version"], "1.2.3")
        self.assertEqual(report["operation"]["release"]["tag_target_sha"], "a" * 40)
        self.assertEqual(report["latency"]["slo"]["measurement"], "operation_seconds")
        self.assertTrue(report["latency"]["slo"]["met"])
        self.assertNotIn("::warning::", process.stdout)

    def test_prepare_history_uses_operation_clocks_and_audits_excluded_successful_noops(self):
        self.fixtures[f"{PREFIX}/runs/900"]["event"] = "workflow_dispatch"
        candidates = [self.fixtures[f"{PREFIX}/runs/900"]]
        for identifier, end, result in ((901, 1000, "success"), (902, 1500, "success"),
                                        (903, 70, "success"), (904, 80, "failure")):
            candidates.append(run_record(identifier, event="workflow_dispatch", conclusion=result))
            self.fixtures[f"{PREFIX}/runs/{identifier}/jobs?filter=all&per_page=100&page=1"] = {
                "total_count": 1, "jobs": [job_record(identifier * 10, end=end, conclusion=result)],
            }
        self.fixtures[f"{PREFIX}/workflows/41/runs?status=completed&per_page=20&page=1"] = {
            "total_count": 5, "workflow_runs": candidates,
        }
        self.fixtures[PREPARE_PULLS + "&per_page=100&page=1"] = [
            pull_record(40, 900, 100), pull_record(41, 901, 20), pull_record(42, 902, 200),
        ]

        process, report = self.invoke("--phase", "prepare")

        self.assertEqual(process.returncode, 0, process.stderr)
        history = report["history"]["release"]
        self.assertEqual(history["sample_count"], 2)
        self.assertEqual(history["completed_runs"], 4)
        self.assertEqual(history["excluded_sample_count"], 2)
        self.assertEqual(history["sample_measurement"], "operation_seconds")
        self.assertEqual(history["p50_seconds"], 20)
        self.assertEqual(history["p95_seconds"], 200)
        self.assertEqual(history["workflow_elapsed_p50_seconds"], 1000)
        self.assertEqual(history["workflow_elapsed_p95_seconds"], 1500)
        self.assertEqual(history["excluded_reasons"], {"functional_failure": 1, "reused_pull_request_from_another_run": 1})
        self.assertEqual(history["runs"][2]["operation_seconds"], None)
        self.assertFalse(history["runs"][2]["sample_eligible"])
        self.assertEqual(report["latency"]["slo"]["class"], "release")
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertEqual(sum(PREPARE_PULLS in argument for call in calls for argument in call), 1)

    def test_preparation_noops_reused_foreign_ambiguous_and_missing_api_anchors_are_unknown(self):
        cases = (
            ([], "no_pull_request_created_by_run"),
            ([pull_record(run_id=901)], "reused_pull_request_from_another_run"),
            ([pull_record(run_id=9001)], "reused_pull_request_from_another_run"),
            ([pull_record(head={"ref": "release/next", "repo": {"full_name": "fork-owner/clonk-rs"}})], "no_pull_request_created_by_run"),
            ([pull_record(), pull_record(41)], "ambiguous_producer_pull_request"),
            ({"api_error": "missing"}, "api_unavailable"),
        )
        for records, reason in cases:
            with self.subTest(reason=reason, records=records):
                self.fixtures[PREPARE_PULLS + "&per_page=100&page=1"] = records
                process, report = self.invoke("--phase", "prepare")
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(report["functional_conclusion"], "success")
                self.assertEqual(report["operation"]["status"], "UNKNOWN")
                self.assertEqual(report["operation"]["exclusion_reason"], reason)
                self.assertIsNone(report["latency"]["operation_seconds"])
                self.assertIsNone(report["latency"]["slo"]["met"])
                self.assertNotIn("::warning::", process.stdout)

    def test_publication_noops_preexisting_releases_and_missing_anchors_are_unknown(self):
        self.publication()
        original = copy.deepcopy(self.fixtures)
        source = f"{PREFIX}/runs/900"
        jobs = f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"
        pulls = f"repos/{REPOSITORY}/commits/{'a' * 40}/pulls?per_page=100&page=1"
        release = f"repos/{REPOSITORY}/releases/tags/v1.2.3"
        tag = f"repos/{REPOSITORY}/git/ref/tags/v1.2.3"
        skipped = {"name": "Publish the release", "status": "completed", "conclusion": "skipped",
                   "started_at": stamp(80), "completed_at": stamp(80)}
        for endpoint, replacement, reason in (
            (source, {**original[source], "head_commit": {"id": "a" * 40, "message": "fix: regular change"}}, "no_release_commit"),
            (jobs, {"total_count": 1, "jobs": [job_record(steps=[skipped])]}, "publication_skipped_or_reused"),
            (jobs, {"total_count": 1, "jobs": [job_record()]}, "publication_execution_unavailable"),
            (pulls, [], "no_merged_pull_request"),
            (pulls, [*original[pulls], {**original[pulls][0], "id": 1001, "number": 1001}], "ambiguous_merged_pull_request"),
            (release, {**original[release], "draft": True}, "release_not_published"),
            (release, {**original[release], "published_at": None}, "release_not_published"),
            (release, {**original[release], "published_at": stamp(-1), "created_at": stamp(-2)}, "release_already_published_before_run"),
            (release, {"api_error": "missing"}, "api_unavailable"),
            (tag, {"api_error": "missing"}, "api_unavailable"),
        ):
            with self.subTest(reason=reason):
                self.fixtures = copy.deepcopy(original)
                self.fixtures[endpoint] = replacement
                process, report = self.invoke("--phase", "publication")
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(report["operation"]["status"], "UNKNOWN")
                self.assertEqual(report["operation"]["exclusion_reason"], reason)
                self.assertIsNone(report["latency"]["operation_seconds"])
                self.assertIsNone(report["latency"]["slo"]["met"])

    def test_missing_actual_head_commit_cannot_fabricate_a_publication_version(self):
        self.publication()
        self.fixtures[f"{PREFIX}/runs/900"]["head_commit"] = None

        process, report = self.invoke("--phase", "publication")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["functional_conclusion"], "success")
        self.assertEqual(report["operation"]["status"], "UNKNOWN")
        self.assertEqual(report["operation"]["exclusion_reason"], "actual_head_commit_unavailable")
        self.assertIsNone(report["latency"]["operation_seconds"])

    def test_publication_rerun_keeps_the_original_merge_and_successful_first_attempt_publication(self):
        self.publication()
        self.fixtures[f"{PREFIX}/runs/900"].update(run_attempt=2, run_started_at=stamp(900), updated_at=stamp(1000))
        jobs = self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"]
        jobs["jobs"].append(job_record(2, start=900, end=1000, run_attempt=2, created_at=stamp(850), steps=[{
            "name": "Publish the release", "status": "completed", "conclusion": "skipped",
            "started_at": stamp(950), "completed_at": stamp(950),
        }]))
        jobs["total_count"] = 2

        process, report = self.invoke("--phase", "publication")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["run_attempt"], 2)
        self.assertEqual(report["latency"]["operation_seconds"], 90)
        self.assertEqual(report["latency"]["elapsed_seconds"], 1000)
        self.assertEqual(report["operation"]["end_at"], stamp(80))
        self.assertTrue(report["latency"]["slo"]["met"])

    def test_publication_history_percentiles_share_only_validated_merge_to_publication_samples(self):
        self.publication()
        candidates = [self.fixtures[f"{PREFIX}/runs/900"]]
        for identifier, end, version, published, source in (
            (901, 1000, "1.2.4", 20, "c" * 40), (902, 1500, "1.2.5", 200, "d" * 40),
            (903, 70, "1.2.6", 50, "e" * 40),
        ):
            self.fixtures[f"{PREFIX}/runs/{identifier}"] = run_record(identifier)
            self.fixtures[f"{PREFIX}/runs/{identifier}/jobs?filter=all&per_page=100&page=1"] = {
                "total_count": 1, "jobs": [job_record(identifier * 10, start=0, end=end)],
            }
            self.publication(identifier, published=published, version=version, source=source)
            candidates.append(self.fixtures[f"{PREFIX}/runs/{identifier}"])
        self.fixtures[f"repos/{REPOSITORY}/releases/tags/v1.2.6"].update(published_at=stamp(-1), created_at=stamp(-2))
        self.fixtures[f"{PREFIX}/workflows/41/runs?status=completed&per_page=20&page=1"] = {
            "total_count": 4, "workflow_runs": candidates,
        }

        process, report = self.invoke("--phase", "publication")

        self.assertEqual(process.returncode, 0, process.stderr)
        history = report["history"]["release"]
        self.assertEqual(history["sample_count"], 2)
        self.assertEqual(history["completed_runs"], 3)
        self.assertEqual(history["sample_measurement"], "operation_seconds")
        self.assertEqual(history["p50_seconds"], 30)
        self.assertEqual(history["p95_seconds"], 210)
        self.assertEqual(history["workflow_elapsed_p50_seconds"], 1000)
        self.assertEqual(history["workflow_elapsed_p95_seconds"], 1500)
        self.assertEqual(history["excluded_sample_count"], 1)
        self.assertEqual(history["excluded_reasons"], {"release_already_published_before_run": 1})
        self.assertEqual(history["runs"][2]["operation"]["status"], "UNKNOWN")

    def test_invalid_versions_tag_sources_and_impossible_phase_timestamps_are_errors(self):
        self.publication()
        original = copy.deepcopy(self.fixtures)
        for version in ("latest", "1.2", "1.2.3 extra", "01.2.3", "1.2.3-01", "1.2.3$(false)"):
            with self.subTest(version=version):
                self.fixtures = copy.deepcopy(original)
                self.fixtures[f"{PREFIX}/runs/900"]["head_commit"]["message"] = "chore: release " + version
                process, report = self.invoke("--phase", "publication")
                self.assertEqual(process.returncode, 1)
                self.assertEqual(report["functional_conclusion"], "error")

                self.assertIn("invalid release version", report["error"])
        annotated = f"repos/{REPOSITORY}/git/tags/{900:040x}"
        release = f"repos/{REPOSITORY}/releases/tags/v1.2.3"
        for endpoint, replacement in (
            (annotated, {**original[annotated], "object": {"type": "commit", "sha": "f" * 40}}),
            (release, {**original[release], "published_at": stamp(-20), "created_at": stamp(-30)}),
            (release, {**original[release], "published_at": "invalid"}),
            (release, {**original[release], "published_at": stamp(60), "created_at": stamp(70)}),
        ):
            with self.subTest(endpoint=endpoint, replacement=replacement):
                self.fixtures = copy.deepcopy(original)
                self.fixtures[endpoint] = replacement
                process, report = self.invoke("--phase", "publication")
                self.assertEqual(process.returncode, 1, process.stderr)
                self.assertEqual(report["functional_conclusion"], "error")
        self.fixtures = copy.deepcopy(original)
        for created in (stamp(-1), "invalid"):
            with self.subTest(created=created):
                self.fixtures[PREPARE_PULLS + "&per_page=100&page=1"] = [pull_record(created_at=created)]
                process, report = self.invoke("--phase", "prepare")
                self.assertEqual(process.returncode, 1, process.stderr)
                self.assertEqual(report["functional_conclusion"], "error")

    def test_release_operation_pull_requests_require_complete_bounded_pagination(self):
        first = [pull_record(identifier, run_id=901) for identifier in range(1, 101)]
        self.fixtures[PREPARE_PULLS + "&per_page=100&page=1"] = first
        self.fixtures[PREPARE_PULLS + "&per_page=100&page=2"] = [
            pull_record(400, created=100, state="closed", merged_at=stamp(200), merge_commit_sha="a" * 40),
        ]
        process, report = self.invoke("--phase", "prepare")
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["operation_seconds"], 100)
        self.assertIn("release%2Fnext", self.calls.read_text())
        self.assertIn("page=2", self.calls.read_text())
        self.fixtures[PREPARE_PULLS + "&per_page=100&page=1"][0] = pull_record(1)
        self.fixtures[PREPARE_PULLS + "&per_page=100&page=2"] = {"api_error": "unavailable"}
        process, report = self.invoke("--phase", "prepare")
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["operation"]["exclusion_reason"], "api_unavailable")
        self.assertIsNone(report["latency"]["operation_seconds"])
        self.publication()
        endpoint = f"repos/{REPOSITORY}/commits/{'a' * 40}/pulls?per_page=100&page="
        target = self.fixtures[endpoint + "1"][0]
        self.fixtures[endpoint + "1"] = [pull_record(identifier, state="closed", merged_at=stamp(200),
                                                     merge_commit_sha="f" * 40) for identifier in range(1, 101)]
        self.fixtures[endpoint + "2"] = [target]
        process, report = self.invoke("--phase", "publication")
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["operation_seconds"], 90)

    def test_phase_summary_retains_actual_anchors_operation_clocks_and_excluded_population(self):
        self.fixtures[PREPARE_PULLS + "&per_page=100&page=1"] = [pull_record(created=100)]
        summary = self.directory / "phase-summary.md"

        process, report = self.invoke("--phase", "prepare", "--summary", str(summary))

        self.assertEqual(process.returncode, 0, process.stderr)
        rendered = summary.read_text()
        self.assertIn("Release operation", rendered)
        self.assertIn("workflow_created_at_to_pull_request_created_at", rendered)
        self.assertIn(stamp(0), rendered)
        self.assertIn(stamp(100), rendered)
        self.assertIn("operation_seconds", rendered)
        self.assertIn("Excluded", rendered)
        self.assertIn("Workflow", rendered)
        self.assertNotIn("publication anchors are not inferred", rendered)

    def test_actual_github_squash_release_subject_retains_its_version_and_merged_pr_identity(self):
        self.publication(version="1.5.1")
        # Actual landed clonk-org/clonk-rs release subject, including GitHub's
        # squash suffix rather than the unsuffixed preparation commit subject.
        self.fixtures[f"{PREFIX}/runs/900"]["head_commit"]["message"] = "chore: release 1.5.1 (#1878)\n\nSquashed subjects"
        pull = self.fixtures[f"repos/{REPOSITORY}/commits/{'a' * 40}/pulls?per_page=100&page=1"][0]
        pull.update(number=1878, html_url=f"https://github.com/{REPOSITORY}/pull/1878")

        process, report = self.invoke("--phase", "publication")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["operation_seconds"], 90)
        self.assertEqual(report["operation"]["release"]["version"], "1.5.1")
        self.assertEqual(report["operation"]["pull_request"]["number"], 1878)

    def test_publication_accepts_only_same_repo_automation_prs_and_the_correct_squash_number(self):
        self.publication()
        original = copy.deepcopy(self.fixtures)
        endpoint = f"repos/{REPOSITORY}/commits/{'a' * 40}/pulls?per_page=100&page=1"
        for side, replacement in (
            ("head", {"ref": "feature/release", "repo": {"full_name": REPOSITORY}}),
            ("head", {"ref": "release/next", "repo": {"full_name": "fork-owner/clonk-rs"}}),
            ("base", {"ref": "other", "repo": {"full_name": REPOSITORY}}),
        ):
            with self.subTest(side=side, replacement=replacement):
                self.fixtures = copy.deepcopy(original)
                self.fixtures[endpoint][0][side] = replacement
                process, report = self.invoke("--phase", "publication")
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(report["operation"]["exclusion_reason"], "no_merged_pull_request")
                self.assertIsNone(report["latency"]["operation_seconds"])
        self.fixtures = copy.deepcopy(original)
        self.fixtures[f"{PREFIX}/runs/900"]["head_commit"]["message"] = "chore: release 1.2.3 (#999)"
        process, report = self.invoke("--phase", "publication")
        self.assertEqual(process.returncode, 1)
        self.assertIn("squash suffix", report["error"])

    def test_platform_chain_prebuilds_identify_release_latency_before_qualification_starts(self):
        self.fixtures[f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": 2, "jobs": [
                job_record(name="Build release candidate / linux / Compile shipped inputs / Prebuild runtime / linux"),
                job_record(2, "Qualify release candidate", conclusion="skipped", started_at=None, completed_at=None),
            ],
        }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["slo"]["class"], "release")

    def test_actual_release_squash_source_identifies_latency_when_release_jobs_are_skipped(self):
        self.fixtures[f"{PREFIX}/runs/900"]["head_commit"] = {
            "id": "a" * 40, "message": "chore: release 1.5.1 (#1878)\n\nSquashed subjects",
        }

        process, report = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(report["latency"]["slo"]["class"], "release")

    def test_valid_history_limit_truncates_fetches_and_labels_the_sample_window(self):
        self.fixtures[f"{PREFIX}/workflows/41/runs?status=completed&per_page=1&page=1"] = {
            "total_count": 50, "workflow_runs": [run_record(901)],
        }
        self.fixtures[f"{PREFIX}/runs/901/jobs?filter=all&per_page=100&page=1"] = {
            "total_count": 1, "jobs": [job_record(10, end=160)],
        }

        process, report = self.invoke("--history-limit", "1")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertTrue(report["history"]["truncated"])
        self.assertEqual(report["history"]["inspected_runs"], 1)
        self.assertEqual(report["history"]["ordinary"]["sample_count"], 1)
        self.assertEqual(report["history"]["ordinary"]["p95_seconds"], 160)
        self.assertEqual(report["history"]["ordinary"]["runs"][0]["run_id"], 901)
        self.assertNotIn("page=2", self.calls.read_text())

    def test_failure_cancel_error_and_unknown_conclusions_never_return_success(self):
        for conclusion, expected in (("failure", "failure"), ("timed_out", "failure"),
                                     ("cancelled", "cancelled"), ("startup_failure", "error"),
                                     ("neutral", "unknown"), ("skipped", "unknown")):
            with self.subTest(conclusion=conclusion):
                self.fixtures[f"{PREFIX}/runs/900"] = run_record(conclusion=conclusion)
                process, report = self.invoke()
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(report["functional_conclusion"], expected)

    def test_missing_or_duplicate_job_pages_fail_instead_of_truncating_measurement(self):
        endpoint = f"{PREFIX}/runs/900/jobs?filter=all&per_page=100&page="
        self.fixtures[endpoint + "1"]["total_count"] = 2
        for jobs in ([], [job_record()]):
            with self.subTest(jobs=jobs):
                self.fixtures[endpoint + "2"] = {"total_count": 2, "jobs": jobs}
                process, report = self.invoke()
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(report["functional_conclusion"], "error")

    def test_running_mode_does_not_overwrite_an_explicit_failure_conclusion(self):
        self.fixtures[f"{PREFIX}/runs/900"] = run_record(status="in_progress", conclusion="failure")

        process, report = self.invoke("--allow-running", "true")

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(report["functional_conclusion"], "failure")


if __name__ == "__main__":
    unittest.main()
