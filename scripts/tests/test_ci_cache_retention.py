"""Cache retention exercises recorded GitHub responses without using the network."""

from datetime import datetime, timedelta, timezone
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "ci-cache-retention.py"
REPOSITORY = "clonk-org/clonk-rs"
PREFIX = "clonk-ci-target-v1-"
API = f"repos/{REPOSITORY}/actions/caches"
EPOCH = datetime(2026, 10, 6, tzinfo=timezone.utc)


def stamp(seconds):
    return (EPOCH + timedelta(seconds=seconds)).isoformat().replace("+00:00", "Z")


def cache(identifier, lane="landing-linux", size=100, created=None, **changes):
    return {
        "id": identifier, "key": f"{PREFIX}{lane}-source-{identifier}",
        "ref": "refs/heads/main", "size_in_bytes": size,
        "created_at": stamp(identifier if created is None else created),
        "last_accessed_at": stamp(identifier if created is None else created),
        **changes,
    }


class CacheRetentionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="clonk-ci-cache-retention-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.fixtures = {}
        self.calls = self.directory / "calls.jsonl"
        fake = self.directory / "gh"
        fake.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "arguments = sys.argv[1:]\n"
            "endpoint = next(arg for arg in arguments if arg.startswith('repos/'))\n"
            "method = arguments[arguments.index('--method') + 1] if '--method' in arguments else 'GET'\n"
            "with Path(os.environ['FAKE_GH_CALLS']).open('a') as target:\n"
            "    target.write(json.dumps(arguments) + '\\n')\n"
            "fixture = json.loads(Path(os.environ['FAKE_GH_FIXTURES']).read_text()).get(method + ' ' + endpoint)\n"
            "if fixture is None:\n"
            "    print('missing fake gh response: ' + method + ' ' + endpoint, file=sys.stderr)\n"
            "    sys.exit(42)\n"
            "if isinstance(fixture, dict) and 'exit_code' in fixture:\n"
            "    print(fixture.get('stdout', ''), end='')\n"
            "    print(fixture.get('stderr', ''), end='', file=sys.stderr)\n"
            "    sys.exit(fixture['exit_code'])\n"
            "print(json.dumps(fixture))\n",
            encoding="utf-8",
        )
        fake.chmod(0o755)
        event = self.directory / "event.json"
        event.write_text(json.dumps({
            "repository": {"full_name": REPOSITORY}, "ref": "refs/heads/main",
        }), encoding="utf-8")
        self.environment = {
            **os.environ, "PATH": str(self.directory) + os.pathsep + os.environ["PATH"],
            "FAKE_GH_FIXTURES": str(self.directory / "fixtures.json"),
            "FAKE_GH_CALLS": str(self.calls), "GITHUB_REF": "refs/heads/main",
            "GITHUB_EVENT_NAME": "push", "GITHUB_REPOSITORY": REPOSITORY,
            "GITHUB_EVENT_PATH": str(event), "GITHUB_HEAD_REF": "",
        }

    def inventory(self, *batches):
        total = sum(len(batch) for batch in batches)
        for page, batch in enumerate(batches or ([],), 1):
            self.fixtures[f"GET {API}?per_page=100&page={page}"] = {
                "total_count": total, "actions_caches": batch,
            }

    def invoke(self, *arguments):
        Path(self.environment["FAKE_GH_FIXTURES"]).write_text(json.dumps(self.fixtures), encoding="utf-8")
        output = self.directory / "receipt.json"
        if output.exists():
            output.unlink()
        process = subprocess.run(
            [sys.executable, str(SCRIPT), "--repository", REPOSITORY,
             "--output", str(output), *arguments],
            capture_output=True, text=True, env=self.environment, timeout=20,
        )
        return process, json.loads(output.read_text()) if output.exists() else None

    def requests(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def deleted_ids(self):
        return [int(next(arg for arg in request if arg.startswith(API + "/")).rsplit("/", 1)[1])
                for request in self.requests() if "DELETE" in request]

    def test_dry_run_keeps_two_newest_per_lane_and_never_deletes(self):
        self.inventory([
            cache(1), cache(2), cache(3),
            cache(4, "coverage-linux"), cache(5, "coverage-linux"), cache(6, "coverage-linux"),
        ])

        process, receipt = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(receipt["mode"], "dry-run")
        self.assertEqual({item["id"] for item in receipt["selected"]}, {2, 3, 5, 6})
        self.assertEqual({item["id"] for item in receipt["evicted"]}, {1, 4})
        self.assertEqual(receipt["selected_bytes"], 400)
        self.assertEqual(receipt["deleted_bytes"], 0)
        self.assertEqual(receipt["deleted_count"], 0)
        self.assertFalse(self.deleted_ids())

    def test_every_inventory_page_is_read_before_selecting_the_newest_caches(self):
        self.inventory([cache(1), cache(2)], [cache(3), cache(4)])

        process, receipt = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual({item["id"] for item in receipt["selected"]}, {3, 4})
        self.assertEqual(receipt["inventory_count"], 4)
        self.assertEqual([next(arg for arg in request if arg.startswith(API))
                          for request in self.requests()],
                         [f"{API}?per_page=100&page=1", f"{API}?per_page=100&page=2"])

    def test_apply_refuses_untrusted_events_refs_and_repository_identity_before_any_api_call(self):
        self.inventory([cache(1), cache(2), cache(3)])
        original = self.environment.copy()
        for changes in (
            {"GITHUB_EVENT_NAME": "pull_request"}, {"GITHUB_EVENT_NAME": "merge_group"},
            {"GITHUB_EVENT_NAME": "pull_request_target"}, {"GITHUB_EVENT_NAME": "workflow_run"},
            {"GITHUB_REF": "refs/heads/release/next"}, {"GITHUB_REPOSITORY": "someone/clonk-rs"},
            {"GITHUB_REPOSITORY": ""}, {"GITHUB_EVENT_NAME": ""}, {"GITHUB_HEAD_REF": "fork-branch"},
            {"GITHUB_EVENT_PATH": ""},
        ):
            with self.subTest(environment=changes):
                self.environment = {**original, **changes}
                process, _ = self.invoke("--apply")
                self.assertNotEqual(process.returncode, 0)
                self.assertIn("trusted main", process.stderr)
                self.assertFalse(self.requests())
        self.environment = original

    def test_trusted_main_apply_deletes_only_eligible_cache_ids_after_all_pages(self):
        self.inventory([
            cache(1), cache(2), cache(10, key="unrelated-cache"),
            cache(11, ref="refs/pull/1886/merge"), cache(12, lane="unsupported-linux"),
        ], [cache(3), cache(4), cache(13, ref="refs/heads/feature")])
        for identifier in (1, 2):
            self.fixtures[f"DELETE {API}/{identifier}"] = {
                "exit_code": 0, "stdout": "HTTP/2.0 204 No Content\r\n\r\n",
            }

        process, receipt = self.invoke("--apply")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(receipt["mode"], "apply")
        self.assertEqual(receipt["selected_count"], 2)
        self.assertEqual(receipt["ignored_count"], 4)
        self.assertEqual(set(self.deleted_ids()), {1, 2})
        self.assertEqual(receipt["deleted_count"], 2)
        self.assertEqual(receipt["deleted_bytes"], 200)
        methods = ["DELETE" if "DELETE" in request else "GET" for request in self.requests()]
        self.assertEqual(methods, ["GET", "GET", "DELETE", "DELETE"])

    def test_an_http_404_from_a_concurrent_deletion_is_idempotent_and_accounted_separately(self):
        self.inventory([cache(1), cache(2), cache(3), cache(4)])
        self.fixtures[f"DELETE {API}/2"] = {
            "exit_code": 1, "stdout": "HTTP/2.0 404 Not Found\r\n\r\n{\"message\":\"Not Found\"}",
            "stderr": "gh: Not Found (HTTP 404)",
        }
        self.fixtures[f"DELETE {API}/1"] = {
            "exit_code": 0, "stdout": "HTTP/2.0 204 No Content\r\n\r\n",
        }

        process, receipt = self.invoke("--apply")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(receipt["deleted_count"], 1)
        self.assertEqual(receipt["deleted_bytes"], 100)
        self.assertEqual(receipt["already_missing_count"], 1)
        self.assertEqual(receipt["already_missing_bytes"], 100)
        self.assertEqual([item["id"] for item in receipt["already_missing"]], [2])
        self.assertEqual(self.deleted_ids(), [2, 1])

    def test_current_caches_get_priority_and_oversize_entries_have_explicit_eviction_reasons(self):
        self.inventory([
            cache(10, size=500), cache(9, "coverage-linux"), cache(7), cache(6, "coverage-linux"),
        ])

        process, receipt = self.invoke("--max-bytes", "250")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual({item["id"] for item in receipt["selected"]}, {7, 9})
        self.assertEqual(receipt["selected_bytes"], 200)
        reasons = {item["id"]: item["reason"] for item in receipt["evicted"]}
        self.assertEqual(reasons, {10: "cache-larger-than-budget", 6: "byte-budget"})
        self.assertEqual(receipt["evicted_bytes"], 600)

    def test_malformed_entries_even_outside_the_namespace_block_all_deletions(self):
        malformed = []
        for field in ("id", "key", "ref", "size_in_bytes", "created_at", "last_accessed_at"):
            entry = cache(9, key="unrelated-cache")
            del entry[field]
            malformed.append(entry)
        malformed.extend(cache(9, key="unrelated-cache", **changes) for changes in (
            {"id": True}, {"id": 0}, {"id": -1}, {"id": "9"},
            {"size_in_bytes": -1}, {"size_in_bytes": True}, {"size_in_bytes": "100"},
            {"ref": ""}, {"ref": None}, {"created_at": "not-a-time"},
            {"created_at": "2026-10-06T00:00:09"}, {"last_accessed_at": stamp(8)},
        ))
        malformed.extend((None, "cache entry"))
        for entry in malformed:
            with self.subTest(entry=entry):
                self.fixtures = {}
                if self.calls.exists():
                    self.calls.unlink()
                self.inventory([cache(1), cache(2), cache(3)], [entry])
                self.fixtures[f"DELETE {API}/1"] = {
                    "exit_code": 0, "stdout": "HTTP/2.0 204 No Content\r\n\r\n",
                }

                process, receipt = self.invoke("--apply")

                self.assertNotEqual(process.returncode, 0)
                self.assertFalse(self.deleted_ids())
                self.assertEqual(receipt["status"], "failure")

    def test_only_complete_keys_in_the_two_exact_supported_lanes_are_eligible(self):
        self.inventory([
            cache(1), cache(2), cache(10, key=PREFIX + "landing-linux-"),
            cache(11, key=PREFIX + "coverage-linux"), cache(12, key=PREFIX + "coverage-windows-tree"),
            cache(13, key="clonk-ci-target-v10-landing-linux-tree"),
            cache(14, key="other-" + PREFIX + "landing-linux-tree"),
        ])

        process, receipt = self.invoke()

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual({item["id"] for item in receipt["selected"]}, {1, 2})
        self.assertEqual(receipt["eligible_count"], 2)
        self.assertEqual(receipt["ignored_count"], 5)
        self.assertFalse(receipt["evicted"])

    def test_invalid_repository_budget_and_prefix_arguments_fail_before_api_calls(self):
        self.inventory([cache(1), cache(2), cache(3)])
        for arguments in (
            ("--repository", "../repo"), ("--repository", "owner/repo/other"),
            ("--repository", "owner/%repo"), ("--repository", "not-a-repository"),
            ("--keep-per-lane", "0"), ("--keep-per-lane", "-1"),
            ("--max-bytes", "-1"), ("--prefix", "clonk-"),
        ):
            with self.subTest(arguments=arguments):
                if self.calls.exists():
                    self.calls.unlink()
                process, _ = self.invoke(*arguments)
                self.assertNotEqual(process.returncode, 0)
                self.assertFalse(self.requests())

    def test_only_the_response_status_can_prove_a_concurrent_404(self):
        self.inventory([cache(1), cache(2), cache(3)])
        self.fixtures[f"DELETE {API}/1"] = {
            "exit_code": 1,
            "stdout": "HTTP/2.0 403 Forbidden\r\ncontent-type: text/plain\r\n\r\nHTTP/2.0 404 Not Found\r\n",
            "stderr": "request body mentioned HTTP 404",
        }

        process, receipt = self.invoke("--apply")

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(receipt["status"], "failure")
        self.assertEqual(receipt["deleted_count"], 0)
        self.assertEqual(receipt["already_missing_count"], 0)
        self.assertIn("HTTP 403", receipt["error"])

    def test_current_cache_in_each_lane_precedes_newer_extras_when_both_fit(self):
        self.inventory([cache(1, "coverage-linux"), cache(2), cache(3)])

        process, receipt = self.invoke("--max-bytes", "200")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual({item["id"] for item in receipt["selected"]}, {1, 3})
        self.assertEqual([item["id"] for item in receipt["evicted"]], [2])
        self.assertEqual({item["reason"] for item in receipt["selected"]}, {"current"})

    def test_combined_current_caches_over_budget_keep_the_newer_one_and_a_fitting_older_fallback(self):
        self.inventory([cache(1, size=200), cache(2, size=800), cache(3, "coverage-linux", size=800)])

        process, receipt = self.invoke("--max-bytes", "1000")

        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual({item["id"] for item in receipt["selected"]}, {1, 3})
        self.assertEqual(receipt["selected_bytes"], 1000)
        self.assertEqual(receipt["evicted"][0]["reason"], "current-caches-exceed-budget")

    def test_equal_timestamps_are_ordered_by_access_then_id_independently_of_api_order(self):
        entries = [
            cache(1, created=100, last_accessed_at=stamp(120)),
            cache(2, created=100, last_accessed_at=stamp(130)),
            cache(3, created=100, last_accessed_at=stamp(130)),
            cache(4, created=90, last_accessed_at=stamp(999)),
        ]
        for batch in (entries, list(reversed(entries))):
            with self.subTest(batch=batch):
                self.inventory(batch)
                process, receipt = self.invoke()
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual([item["id"] for item in receipt["selected"]], [3, 2])

    def test_a_zero_byte_budget_and_one_cache_per_lane_limit_are_explicit_options(self):
        self.inventory([cache(1), cache(2), cache(3, "coverage-linux")])

        process, receipt = self.invoke("--keep-per-lane", "1")
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual({item["id"] for item in receipt["selected"]}, {2, 3})
        self.assertEqual(receipt["evicted"][0]["reason"], "per-lane-limit")

        process, receipt = self.invoke("--max-bytes", "0")
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(receipt["selected_bytes"], 0)
        self.assertEqual(receipt["selected_count"], 0)
        self.assertEqual(receipt["evicted_count"], 3)
        self.assertTrue(all(item["reason"] == "cache-larger-than-budget" for item in receipt["evicted"]))

    def test_main_push_dispatch_and_schedule_contexts_are_accepted(self):
        self.inventory([])
        for event in ("push", "workflow_dispatch", "schedule"):
            with self.subTest(event=event):
                self.environment["GITHUB_EVENT_NAME"] = event
                payload = {"repository": {"full_name": REPOSITORY}}
                if event == "push":
                    payload.update(ref="refs/heads/main", deleted=False)
                Path(self.environment["GITHUB_EVENT_PATH"]).write_text(json.dumps(payload), encoding="utf-8")
                process, receipt = self.invoke("--apply")
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(receipt["inventory_count"], 0)
                self.assertFalse(self.deleted_ids())

    def test_a_malformed_or_cross_repository_event_payload_refuses_apply(self):
        self.inventory([cache(1), cache(2), cache(3)])
        for payload in (
            {}, None, [], {"repository": {"full_name": "another/repo"}, "ref": "refs/heads/main"},
            {"repository": {"full_name": REPOSITORY}, "ref": "refs/heads/feature"},
            {"repository": {"full_name": REPOSITORY}, "ref": "refs/heads/main", "deleted": True},
            {"repository": {"full_name": REPOSITORY}, "ref": "refs/heads/main", "deleted": "false"},
        ):
            with self.subTest(payload=payload):
                Path(self.environment["GITHUB_EVENT_PATH"]).write_text(json.dumps(payload), encoding="utf-8")
                process, receipt = self.invoke("--apply")
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(receipt["status"], "failure")
                self.assertFalse(self.requests())

    def test_pagination_changes_duplicates_missing_pages_and_invalid_counts_never_delete(self):
        endpoint = f"GET {API}?per_page=100&page="
        for second in (
            {"total_count": 5, "actions_caches": [cache(4)]},
            {"total_count": 4, "actions_caches": [cache(1)]},
            {"total_count": 4, "actions_caches": []},
            {"total_count": 4, "actions_caches": [cache(4), cache(5)]},
            {"total_count": True, "actions_caches": [cache(4)]},
            {"total_count": 4}, {"total_count": 4, "actions_caches": {}}, [],
            {"exit_code": 1, "stderr": "inventory request failed"},
        ):
            with self.subTest(second_page=second):
                self.fixtures = {
                    endpoint + "1": {"total_count": 4, "actions_caches": [cache(1), cache(2), cache(3)]},
                    endpoint + "2": second,
                }
                if self.calls.exists():
                    self.calls.unlink()
                process, receipt = self.invoke("--apply")
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(receipt["status"], "failure")
                self.assertFalse(self.deleted_ids())

    def test_non_404_errors_stop_further_deletions_and_preserve_a_partial_receipt(self):
        self.inventory([cache(1), cache(2), cache(3), cache(4), cache(5)])
        self.fixtures[f"DELETE {API}/3"] = {
            "exit_code": 0, "stdout": "HTTP/2.0 204 No Content\r\n\r\n",
        }
        self.fixtures[f"DELETE {API}/2"] = {
            "exit_code": 1, "stdout": "HTTP/2.0 403 Forbidden\r\n\r\n",
            "stderr": "gh: Resource not accessible by integration (HTTP 403)",
        }

        process, receipt = self.invoke("--apply")

        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(self.deleted_ids(), [3, 2])
        self.assertEqual(receipt["status"], "failure")
        self.assertEqual(receipt["deleted_count"], 1)
        self.assertEqual(receipt["deleted_bytes"], 100)
        self.assertEqual([item["id"] for item in receipt["deleted"]], [3])
        self.assertIn("HTTP 403", receipt["error"])

    def test_unverified_404_text_and_unexpected_success_responses_fail(self):
        self.inventory([cache(1), cache(2), cache(3)])
        for response in (
            {"exit_code": 1, "stderr": "gh: Not Found (HTTP 404)"},
            {"exit_code": 1, "stdout": '{"message":"Not Found","status":"404"}'},
            {"exit_code": 0, "stdout": "{}"},
            {"exit_code": 0, "stdout": "HTTP/2.0 500 Internal Server Error\r\n\r\n"},
        ):
            with self.subTest(response=response):
                self.fixtures[f"DELETE {API}/1"] = response
                process, receipt = self.invoke("--apply")
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(receipt["deleted_count"], 0)
                self.assertEqual(receipt["already_missing_count"], 0)

    def test_subprocesses_and_the_total_pagination_are_bounded(self):
        spec = importlib.util.spec_from_file_location("ci_cache_retention", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        response = subprocess.CompletedProcess([], 0, json.dumps({"total_count": 0, "actions_caches": []}), "")
        with mock.patch.object(module.subprocess, "run", return_value=response) as run:
            self.assertEqual(module.inventory(REPOSITORY), [])
            self.assertGreater(run.call_args.kwargs["timeout"], 0)
            self.assertLessEqual(run.call_args.kwargs["timeout"], 30)
        with mock.patch.object(module.time, "monotonic", side_effect=(0, module.INVENTORY_TIMEOUT + 1)):
            with mock.patch.object(module.subprocess, "run") as run:
                with self.assertRaisesRegex(module.RetentionError, "time limit"):
                    module.inventory(REPOSITORY)
                run.assert_not_called()
        with mock.patch.object(module, "MAX_PAGES", 1):
            response.stdout = json.dumps({"total_count": 2, "actions_caches": [cache(1)]})
            with mock.patch.object(module.subprocess, "run", return_value=response) as run:
                with self.assertRaisesRegex(module.RetentionError, "page limit"):
                    module.inventory(REPOSITORY)
                self.assertEqual(run.call_count, 1)

    def test_an_unwritable_receipt_destination_is_rejected_before_any_deletion(self):
        self.inventory([cache(1), cache(2), cache(3)])
        self.fixtures[f"DELETE {API}/1"] = {
            "exit_code": 0, "stdout": "HTTP/2.0 204 No Content\r\n\r\n",
        }
        output = self.directory / "not-a-file"
        output.mkdir()

        process, _ = self.invoke("--apply", "--output", str(output))

        self.assertNotEqual(process.returncode, 0)
        self.assertFalse(self.requests())
        self.assertEqual(json.loads(process.stdout)["status"], "failure")


if __name__ == "__main__":
    unittest.main()
