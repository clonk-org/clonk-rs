"""Scheduling guards for the four-thread CI nextest profile."""

import tomllib
import unittest

from _repo import REPOSITORY


class NextestCiProfileTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.config = tomllib.loads(
            (REPOSITORY / ".config" / "nextest.toml").read_text(encoding="utf-8")
        )

    def test_ci_runs_all_tests_and_retains_failure_diagnostics(self):
        self.assertIn("ci", self.config["profile"])
        ci = self.config["profile"]["ci"]
        self.assertFalse(ci["fail-fast"])
        self.assertNotIn("default-filter", ci)
        self.assertNotIn("inherits", ci)
        self.assertEqual(ci["junit"]["path"], "junit.xml")
        self.assertEqual(ci["junit"]["report-name"], "nextest-run")
        self.assertFalse(ci["junit"]["store-success-output"])
        self.assertTrue(ci["junit"]["store-failure-output"])

    def test_ci_starts_isolated_latency_contracts_before_scenario_work(self):
        defaults = self.config["profile"]["default"]["overrides"]
        ci = self.config["profile"]["ci"].get("overrides", [])
        contracts = {
            "initial_network_game_join_fully_loads_the_client_lobby_within_500ms": 12,
            "selected_clonkmars_host_reference_is_queryable_within_one_second": 24,
        }
        for name, threads in contracts.items():
            with self.subTest(test=name):
                default = next(row for row in defaults if name in row["filter"])
                self.assertEqual(default["threads-required"], threads)
                self.assertTrue(default["junit"]["store-success-output"])
                matching = [row for row in ci if row["filter"] == default["filter"]]
                self.assertEqual(len(matching), 1)
                self.assertEqual(matching[0]["priority"], 100)
                # Only scheduling changes: isolation and JUnit output inherit.
                self.assertEqual(set(matching[0]), {"filter", "priority"})

    def test_ci_keeps_the_existing_scenario_tier_below_latency_contracts(self):
        defaults = self.config["profile"]["default"]["overrides"]
        route_tiers = [row for row in defaults if row.get("priority") == 100]
        self.assertEqual(len(route_tiers), 1)
        ci = self.config["profile"]["ci"].get("overrides", [])
        matching = [row for row in ci if row["filter"] == route_tiers[0]["filter"]]
        self.assertEqual(len(matching), 1)
        self.assertEqual(matching[0]["priority"], 90)
        self.assertEqual(set(matching[0]), {"filter", "priority"})
        self.assertEqual(self.config["nextest-version"], "0.9.91")
        self.assertTrue(all(-100 <= row.get("priority", 0) <= 100 for row in ci))

    def test_ci_inherits_retries_timeouts_and_test_selection(self):
        default_filters = {
            row["filter"] for row in self.config["profile"]["default"]["overrides"]
        }
        ci = self.config["profile"]["ci"]
        self.assertEqual(set(ci), {"fail-fast", "junit", "overrides"})
        for row in ci["overrides"]:
            with self.subTest(filter=row["filter"]):
                self.assertIn(row["filter"], default_filters)
                self.assertTrue(set(row) <= {"filter", "threads-required", "priority"})

    def test_only_ci_reduces_the_existing_allocation_weight(self):
        defaults = self.config["profile"]["default"]["overrides"]
        heavy = [
            row for row in defaults
            if "threads-required" in row and "app_virtual_keyboard" in row["filter"]
        ]
        self.assertEqual(len(heavy), 1)
        self.assertEqual(heavy[0]["threads-required"], 12)
        ci = self.config["profile"]["ci"]["overrides"]
        weighted = [row for row in ci if "threads-required" in row]
        self.assertEqual(len(weighted), 1)
        self.assertEqual(weighted[0]["filter"], heavy[0]["filter"])
        self.assertEqual(weighted[0]["threads-required"], 2)
        self.assertEqual(set(weighted[0]), {"filter", "threads-required"})


if __name__ == "__main__":
    unittest.main()
