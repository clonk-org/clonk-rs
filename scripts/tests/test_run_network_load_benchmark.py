import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "run_network_load_benchmark.py"
SPEC = importlib.util.spec_from_file_location("network_load_benchmark", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
EMPTY_SHA256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"


def metric_series(unit, samples):
    samples = list(samples)
    return {
        "unit": unit,
        "summary": {
            "samples": len(samples),
            "p50": MODULE.nearest_rank_percentile(samples, 0.50),
            "p95": MODULE.nearest_rank_percentile(samples, 0.95),
            "p99": MODULE.nearest_rank_percentile(samples, 0.99),
            "max": max(samples),
        },
        "raw_samples": samples,
    }


def expected_route_peers(topology):
    direct_mesh = topology != "relay"
    routes = [[0, list(range(1, 25))]]
    routes.extend(
        [
            client_id,
            (
                [peer_id for peer_id in range(25) if peer_id != client_id]
                if direct_mesh
                else [0]
            ),
        ]
        for client_id in range(1, 25)
    )
    return routes


def expected_preferred_message_routes(topology):
    protocol = "udp" if topology == "udp" else "tcp"
    return [
        {
            "process_client_id": process_client_id,
            "peer_client_id": peer_client_id,
            "protocol": protocol,
        }
        for process_client_id, peer_ids in expected_route_peers(topology)
        for peer_client_id in peer_ids
    ]


def runtime_sample_group(topology, *, elapsed_ms=1_000):
    route_counts = dict(expected_route_peers(topology))
    return [
        {
            "elapsed_ms": elapsed_ms,
            "process_client_id": process_client_id,
            "route_count": len(route_counts[process_client_id]),
            "tcp_input_rate": 0,
            "tcp_output_rate": 0,
            "udp_input_rate": 0,
            "udp_output_rate": 0,
        }
        for process_client_id in range(25)
    ]


def replace_metric_samples(metric, samples):
    unit = metric["unit"]
    metric.clear()
    metric.update(metric_series(unit, samples))


def set_control_completion_wait(report, samples):
    values = list(samples)
    expanded = [
        values[index % len(values)]
        for index in range(report.get("measured_ticks", 1072))
    ]
    replace_metric_samples(report["control_completion_wait"], expanded)


def set_native_round_trip(report, samples):
    values = list(samples)
    for client in report["client_to_host_round_trip_by_client"]:
        replace_metric_samples(client["metrics"], values)
    replace_metric_samples(
        report["client_to_host_round_trip"], values * 24
    )


def set_measurement_duration(report, *, milliseconds, measured_ticks):
    report["requested_measurement_ms"] = milliseconds
    report["measurement_wall_elapsed_ms"] = milliseconds
    report["minimum_native_control_ticks"] = measured_ticks
    report["measured_ticks"] = measured_ticks
    expected_ready_deliveries = measured_ticks * 25
    report["expected_ready_deliveries"] = expected_ready_deliveries
    report["observed_ready_deliveries"] = expected_ready_deliveries
    set_control_completion_wait(report, [10, 20, 30])
    replace_metric_samples(
        report["participant_ready"], [10] * expected_ready_deliveries
    )
    replace_metric_samples(
        report["cadence_lateness"], [1] * measured_ticks
    )


def network_load_report(*, source_commit="abc", topology="udp"):
    assertion_names = [
        "every-participant-ready",
        "native-control-cadence",
        "measurement-wall-duration",
        "exact-route-topology",
        "exact-preferred-message-routes",
        "aggregate-rtt-samples",
        "per-client-rtt-series",
    ]
    for client_id in range(1, 25):
        assertion_names.extend(
            [
                f"client-{client_id}-rtt-samples",
                f"client-{client_id}-rtt-p99",
            ]
        )
    assertion_names.extend(
        [
            "aggregate-application-rtt-samples",
            "per-client-application-rtt-series",
        ]
    )
    for client_id in range(1, 25):
        assertion_names.extend(
            [
                f"client-{client_id}-application-rtt-samples",
                f"client-{client_id}-application-rtt-p99",
            ]
        )
    assertion_names.extend(
        [
            "aggregate-application-rtt-p99",
            "isolated-application-rtt-warmup-samples",
            "isolated-application-rtt-samples",
            "isolated-application-rtt-client-id",
            "isolated-application-rtt-preferred-message-routes",
            "isolated-application-rtt-p99",
            "aggregate-rtt-p99",
            "control-completion-p99",
            "loaded-session-clean-shutdown",
            "isolated-ping-clean-shutdown",
        ]
    )
    native_by_client = [
        {
            "client_id": client_id,
            "metrics": metric_series("milliseconds", [1, 2, 3]),
        }
        for client_id in range(1, 25)
    ]
    application_by_client = [
        {
            "client_id": client_id,
            "metrics": metric_series(
                "microseconds", [100 + client_id] * 8
            ),
        }
        for client_id in range(1, 25)
    ]
    route_peers = expected_route_peers(topology)
    measured_ticks = 1072
    expected_ready_deliveries = measured_ticks * 25
    control_completion_wait = [10, 20, 30]
    control_completion_wait = [
        control_completion_wait[index % len(control_completion_wait)]
        for index in range(measured_ticks)
    ]
    return {
        "schema_version": 6,
        "workload": (
            "same-process Tokio IPv4-loopback real-socket "
            "HarpoonRace-shaped control transport"
        ),
        "workload_scope": (
            "HarpoonRace-shaped lobby/control parameters only; no "
            "scenario/resource loading or game simulation"
        ),
        "sequence": (
            "synthetic max_players=24 JoinData -> 24 PlayerInfo joins -> "
            "activate all -> GO"
        ),
        "round_trip_scope": (
            "native ping and loaded 24-client ReadyCheck fanout are "
            "diagnostics; primary RTT is a fresh one-host/one-client "
            "post-shutdown application exchange in the same Tokio process "
            "over IPv4 loopback"
        ),
        "application_round_trip_sequence": (
            "diagnostic loaded 24-client fanout after control measurement: "
            "sequential host Ready(client_id) broadcast -> addressed client "
            "Ready echo -> host receipt over selected message routes"
        ),
        "application_round_trip_rounds_per_client": 8,
        "isolated_application_round_trip_sequence": (
            "loaded-session shutdown -> fresh same-topology one-host/one-"
            "client join/status handshake -> 128 warmup + 256 measured "
            "sequential exchanges: host ReadyCheck(Other(index+2)) -> client "
            "ActivationRequest(index+2) -> matching host receipt; exactly two "
            "logical messages per exchange"
        ),
        "isolated_application_round_trip_warmup_samples": 128,
        "isolated_application_round_trip_samples": 256,
        "isolated_application_round_trip_client_id": 1,
        "authoritative_duration": True,
        "topology": topology,
        "preferred_message_protocol": (
            "udp" if topology == "udp" else "tcp"
        ),
        "direct_tcp_mesh": topology == "tcp",
        "player_profiles_joined": 24,
        "host_player_profiles": 0,
        "active_control_participants": 25,
        "control_target_fps": 38,
        "native_game_tick_ms": 28,
        "native_control_interval_ms": 56,
        "control_rate": 2,
        "warmup_ticks": 36,
        "requested_measurement_ms": 60_000,
        "measurement_wall_elapsed_ms": 60_000,
        "minimum_native_control_ticks": 1072,
        "measured_ticks": measured_ticks,
        "expected_ready_deliveries": expected_ready_deliveries,
        "observed_ready_deliveries": expected_ready_deliveries,
        "mesh_establishment_us": (
            None if topology == "relay" else 100
        ),
        "final_route_peers": route_peers,
        "final_preferred_message_routes": (
            expected_preferred_message_routes(topology)
        ),
        "isolated_application_round_trip_preferred_message_routes": [
            {
                "process_client_id": 0,
                "peer_client_id": 1,
                "protocol": "udp" if topology == "udp" else "tcp",
            },
            {
                "process_client_id": 1,
                "peer_client_id": 0,
                "protocol": "udp" if topology == "udp" else "tcp",
            },
        ],
        "join_duration": metric_series("microseconds", [100] * 24),
        "client_to_host_round_trip": metric_series(
            "milliseconds",
            [
                sample
                for client in native_by_client
                for sample in client["metrics"]["raw_samples"]
            ],
        ),
        "client_to_host_round_trip_by_client": native_by_client,
        "client_to_host_application_round_trip": metric_series(
            "microseconds",
            [
                sample
                for client in application_by_client
                for sample in client["metrics"]["raw_samples"]
            ],
        ),
        "client_to_host_application_round_trip_by_client": (
            application_by_client
        ),
        "client_to_host_isolated_application_round_trip": metric_series(
            "microseconds", [200] * 256
        ),
        "control_completion_wait": metric_series(
            "microseconds", control_completion_wait
        ),
        "participant_ready": metric_series(
            "microseconds", [10] * expected_ready_deliveries
        ),
        "cadence_lateness": metric_series(
            "microseconds", [1] * measured_ticks
        ),
        "native_control_wait": metric_series("milliseconds", [0] * 625),
        "runtime_samples": runtime_sample_group(topology),
        "fingerprint": {
            "source_commit": source_commit,
            "source_dirty": False,
            "content_revision": "content",
            "rustc": "rustc 1.2.3",
            "target_os": "linux",
            "target_arch": "x86_64",
            "cpu": "Example CPU",
            "os_version": "Example OS",
            "cargo_profile": "test",
        },
        "result": "pass",
        "assertions": [
            {"name": name, "passed": True} for name in assertion_names
        ],
    }


def set_application_round_trip(report, value):
    for client in report["client_to_host_application_round_trip_by_client"]:
        replace_metric_samples(client["metrics"], [value] * 8)
    replace_metric_samples(
        report["client_to_host_application_round_trip"],
        [value] * (24 * 8),
    )


def set_isolated_application_round_trip(report, value):
    replace_metric_samples(
        report["client_to_host_isolated_application_round_trip"],
        [value] * 256,
    )


def write_prebuilt_provenance(binary, *, cargo_profile="release"):
    provenance = {
        "schema_version": MODULE.PROVENANCE_SCHEMA,
        "kind": "clonk-network-load-build-provenance",
        "binary": MODULE.binary_metadata(binary),
        "source": {
            "commit": "a" * 40,
            "head_tree": "b" * 40,
            "tracked_patch_sha256": "1" * 64,
            "untracked_inputs_sha256": EMPTY_SHA256,
            "untracked_input_files": {},
            "dirty": True,
        },
        "content": {
            "head": "6" * 40,
            "tree": "7" * 40,
            "parent_gitlink_mode": "160000",
            "parent_gitlink_type": "commit",
            "parent_gitlink_revision": "6" * 40,
            "tracked_patch_sha256": EMPTY_SHA256,
            "untracked_inputs_sha256": EMPTY_SHA256,
            "untracked_input_files": {},
            "dirty": False,
        },
        "inputs": {
            "cargo_lock_sha256": "3" * 64,
            "manifest_files": {"Cargo.toml": "4" * 64},
            "configuration_files": {},
            "cargo_configuration_files": {},
            "benchmark_contract_files": {
                "crates/clonk-network/tests/network_load_24.rs": "5" * 64,
            },
            "effective_profile": {
                "selected_profile": cargo_profile,
                "workspace_profile_tables": {
                    "release": {"lto": "thin", "codegen-units": 1},
                },
                "cargo_artifact_profile": {
                    "opt_level": "3",
                    "debuginfo": 0,
                    "debug_assertions": False,
                },
            },
        },
        "toolchain": {
            "rustc_vv": "rustc 1.2.3\nbinary: rustc",
            "cargo_version": "cargo 1.2.3",
        },
        "environment": dict.fromkeys(MODULE.BUILD_ENVIRONMENT_FIELDS),
        "build": {
            "cargo_profile": cargo_profile,
            "command": ["cargo", "test", "--profile", cargo_profile],
            "runner_contract_version": MODULE.RUNNER_SCHEMA,
            "runner_script_sha256": MODULE.runner_script_sha256(),
        },
    }
    MODULE.write_json(MODULE.provenance_sidecar_path(binary), provenance)
    return provenance


def write_cohort(
    cohort,
    *,
    source_commit=None,
    cargo_profile="release",
    control_samples=(10, 20, 30),
    binary_sha256=None,
):
    cohort.mkdir()
    source_commit = source_commit or ("a" * 40)
    binary_marker = binary_sha256 or ("a" * 64)
    retained_binary = cohort / "candidate-benchmark-binary"
    retained_binary.write_bytes(binary_marker.encode("ascii"))
    binary_details = MODULE.binary_metadata(retained_binary)
    binary_sha256 = binary_details["sha256"]
    report = network_load_report(source_commit=source_commit)
    set_control_completion_wait(report, control_samples)
    run = cohort / "run-001"
    run.mkdir()
    report_path = run / "report.json"
    report_path.write_text(json.dumps(report), encoding="utf-8")
    execution = {
        "schema_version": MODULE.RUNNER_SCHEMA,
        "kind": "clonk-network-load-benchmark-execution",
        "return_code": 0,
        "timed_out": False,
        "failure": None,
        "report_present": True,
        "report_sha256": MODULE.sha256_file(report_path),
        "binary_sha256": binary_sha256,
        "binary_sha256_before": binary_sha256,
        "binary_sha256_after": binary_sha256,
    }
    execution_path = run / "execution.json"
    execution_path.write_text(json.dumps(execution), encoding="utf-8")
    provenance = {
        "schema_version": MODULE.PROVENANCE_SCHEMA,
        "kind": "clonk-network-load-build-provenance",
        "binary": {
            "sha256": binary_sha256,
            "size_bytes": binary_details["size_bytes"],
        },
        "source": {
            "commit": source_commit,
            "head_tree": "b" * 40,
            "tracked_patch_sha256": EMPTY_SHA256,
            "untracked_inputs_sha256": EMPTY_SHA256,
            "untracked_input_files": {},
            "dirty": False,
        },
        "content": {
            "head": "6" * 40,
            "tree": "7" * 40,
            "parent_gitlink_mode": "160000",
            "parent_gitlink_type": "commit",
            "parent_gitlink_revision": "6" * 40,
            "tracked_patch_sha256": EMPTY_SHA256,
            "untracked_inputs_sha256": EMPTY_SHA256,
            "untracked_input_files": {},
            "dirty": False,
        },
        "inputs": {
            "cargo_lock_sha256": "3" * 64,
            "manifest_files": {"Cargo.toml": "4" * 64},
            "configuration_files": {},
            "cargo_configuration_files": {},
            "benchmark_contract_files": {
                "crates/clonk-network/tests/network_load_24.rs": "5" * 64,
            },
            "effective_profile": {
                "selected_profile": cargo_profile,
                "workspace_profile_tables": {
                    "release": {"lto": "thin", "codegen-units": 1},
                },
                "cargo_artifact_profile": {
                    "opt_level": "3",
                    "debuginfo": 0,
                    "debug_assertions": False,
                },
            },
        },
        "toolchain": {
            "rustc_vv": "rustc 1.2.3\nbinary: rustc",
            "cargo_version": "cargo 1.2.3",
        },
        "environment": dict.fromkeys(MODULE.BUILD_ENVIRONMENT_FIELDS),
        "build": {
            "cargo_profile": cargo_profile,
            "command": ["cargo", "test", "--profile", cargo_profile],
            "runner_contract_version": MODULE.RUNNER_SCHEMA,
            "runner_script_sha256": MODULE.runner_script_sha256(),
        },
    }
    build = {
        "cargo_profile": cargo_profile,
        "provenance": provenance,
        "provenance_sha256": MODULE.sha256_bytes(
            MODULE.canonical_json(provenance).encode("utf-8")
        ),
    }
    MODULE.write_json(cohort / "build-provenance.json", provenance)
    binary = binary_details
    runtime_machine = {
        "system": "Example OS",
        "release": "1.0",
        "machine": "x86_64",
        "processor": "Example CPU",
        "python_implementation": "CPython",
        "python_version": "3.14.0",
    }
    metadata = {
        "schema_version": MODULE.RUNNER_SCHEMA,
        "kind": "clonk-network-load-benchmark-cohort",
        "configuration": {
            "requested_runs": 1,
            "topology": "udp",
            "measurement_seconds": None,
            "authoritative_default_duration": True,
            "process_timeout_seconds": 300,
            "test_name": MODULE.TEST_NAME,
        },
        "binary": binary,
        "build": build,
        "runtime_machine": runtime_machine,
    }
    (cohort / "cohort-metadata.json").write_text(
        json.dumps(metadata), encoding="utf-8"
    )
    summary = {
        "schema_version": MODULE.RUNNER_SCHEMA,
        "kind": "clonk-network-load-benchmark-summary",
        "result": "pass",
        "binary": binary,
        "build": build,
        "runtime_machine": runtime_machine,
        "requested_runs": 1,
        "successful_runs": 1,
        "failed_runs": 0,
        "runs": [
            {
                "run": 1,
                "directory": "run-001",
                "passed": True,
                "failure": None,
                "report_sha256": MODULE.sha256_file(report_path),
                "execution_sha256": MODULE.sha256_file(execution_path),
                "binary_sha256": binary_sha256,
            }
        ],
    }
    (cohort / "benchmark-summary.json").write_text(
        json.dumps(summary), encoding="utf-8"
    )
    return summary


class NetworkLoadStatisticsTests(unittest.TestCase):
    def test_printed_comparison_uses_the_authoritative_decision_interval(self):
        metric = {
            "baseline_independent_run_median": 100,
            "candidate_independent_run_median": 50,
            "candidate_to_baseline_ratio": 0.5,
            "improvement_percent": 50.0,
            "candidate_to_baseline_ratio_bootstrap_95ci": {
                "lower": 0.1,
                "upper": 0.9,
            },
            "candidate_to_baseline_ratio_95ci": {
                "lower": 0.4,
                "upper": 0.5,
            },
            "target_result": "met",
        }
        value = {
            "comparison_valid": True,
            "statistically_valid": True,
            "target_result": "met",
            "meets_target": True,
            "metrics": {"control_completion_wait": metric},
        }

        with mock.patch("builtins.print") as printed:
            MODULE.print_metric_summary(value)

        output = "\n".join(call.args[0] for call in printed.call_args_list)
        self.assertIn("ratio_95ci={'lower': 0.4, 'upper': 0.5}", output)
        self.assertNotIn("ratio_95ci={'lower': 0.1, 'upper': 0.9}", output)

    def test_exact_paired_median_interval_uses_distribution_free_order_statistics(self):
        interval = MODULE.exact_paired_median_interval(list(range(1, 21)))

        self.assertEqual(interval["lower"], 6)
        self.assertEqual(interval["upper"], 15)
        self.assertEqual(interval["lower_order_statistic"], 6)
        self.assertEqual(interval["upper_order_statistic"], 15)
        self.assertAlmostEqual(interval["confidence_level"], 0.9586105346679688)
        self.assertEqual(interval["method"], "exact-binomial-order-statistic")

    def test_separately_collected_cohorts_are_exploratory_only(self):
        baseline = [network_load_report() for _ in range(20)]
        candidate = [network_load_report() for _ in range(20)]
        for report in baseline:
            set_control_completion_wait(report, [100])
            set_isolated_application_round_trip(report, 100)
        for report in candidate:
            set_control_completion_wait(report, [50])
            set_isolated_application_round_trip(report, 50)

        comparison = MODULE.compare_report_sets(
            baseline,
            candidate,
            expected_topology="udp",
        )

        self.assertTrue(comparison["comparison_valid"])
        self.assertFalse(comparison["statistically_valid"])
        self.assertEqual(comparison["comparison_design"], "exploratory-cohorts")
        self.assertIsNone(comparison["meets_target"])
        self.assertEqual(comparison["target_result"], "indeterminate")

    def test_authoritative_paired_claim_requires_exactly_twenty_prespecified_pairs(
        self,
    ):
        baseline = [network_load_report() for _ in range(21)]
        candidate = [network_load_report() for _ in range(21)]
        for report in baseline:
            set_control_completion_wait(report, [100])
            set_isolated_application_round_trip(report, 100)
        for report in candidate:
            set_control_completion_wait(report, [50])
            set_isolated_application_round_trip(report, 50)

        comparison = MODULE.compare_report_sets(
            baseline,
            candidate,
            expected_topology="udp",
            paired=True,
            verified_paired_experiment=True,
        )

        self.assertFalse(comparison["statistically_valid"])
        self.assertIsNone(comparison["meets_target"])
        self.assertEqual(comparison["target_result"], "indeterminate")

    def test_ratio_above_target_is_indeterminate_when_confidence_crosses_target(self):
        baseline = [network_load_report() for _ in range(20)]
        candidate = [network_load_report() for _ in range(20)]
        for report in baseline:
            set_control_completion_wait(report, [100])
            set_isolated_application_round_trip(report, 100)
        for index, report in enumerate(candidate):
            candidate_value = 40 if index < 9 else 60
            set_control_completion_wait(report, [candidate_value])
            set_isolated_application_round_trip(report, candidate_value)

        comparison = MODULE.compare_report_sets(
            baseline,
            candidate,
            expected_topology="udp",
            paired=True,
            verified_paired_experiment=True,
        )

        control = comparison["metrics"]["control_completion_wait"]
        self.assertEqual(control["candidate_to_baseline_ratio"], 0.6)
        self.assertEqual(
            control["candidate_to_baseline_ratio_95ci"],
            {
                "lower": 0.4,
                "upper": 0.6,
                "confidence_level": 0.9586105346679688,
                "lower_order_statistic": 6,
                "upper_order_statistic": 15,
                "method": "exact-binomial-order-statistic",
            },
        )
        self.assertEqual(control["target_result"], "indeterminate")
        self.assertIsNone(comparison["meets_target"])
        self.assertEqual(comparison["target_result"], "indeterminate")

    def test_target_requires_at_least_fifteen_of_twenty_paired_successes(self):
        baseline = [network_load_report() for _ in range(20)]
        candidate = [network_load_report() for _ in range(20)]
        for report in baseline:
            set_control_completion_wait(report, [100])
            set_isolated_application_round_trip(report, 100)
        for index, report in enumerate(candidate):
            candidate_value = 50 if index < 14 else 60
            set_control_completion_wait(report, [candidate_value])
            set_isolated_application_round_trip(report, candidate_value)

        favorable_interval = {
            "lower": 0.5,
            "upper": 0.5,
            "resamples": MODULE.BOOTSTRAP_RESAMPLES,
        }
        with mock.patch.object(
            MODULE,
            "bootstrap_median_interval",
            return_value=favorable_interval,
        ):
            comparison = MODULE.compare_report_sets(
                baseline,
                candidate,
                expected_topology="udp",
                paired=True,
                verified_paired_experiment=True,
            )

        control = comparison["metrics"]["control_completion_wait"]
        self.assertEqual(control["paired_target_successes"], 14)
        self.assertEqual(control["paired_target_successes_required"], 15)
        self.assertEqual(control["target_result"], "indeterminate")
        self.assertIsNone(comparison["meets_target"])

    def test_summarizes_each_run_as_one_independent_observation(self):
        reports = [
            {
                "control_completion_wait": {
                    "unit": "microseconds",
                    "raw_samples": [10, 20, 30],
                },
                "client_to_host_round_trip": {
                    "unit": "milliseconds",
                    "raw_samples": [1, 2, 3],
                },
                "client_to_host_application_round_trip": {
                    "unit": "microseconds",
                    "raw_samples": [100, 200, 300],
                },
                "client_to_host_isolated_application_round_trip": {
                    "unit": "microseconds",
                    "raw_samples": [100, 200, 300],
                },
            },
            {
                "control_completion_wait": {
                    "unit": "microseconds",
                    "raw_samples": [100, 200, 300],
                },
                "client_to_host_round_trip": {
                    "unit": "milliseconds",
                    "raw_samples": [4, 5, 6],
                },
                "client_to_host_application_round_trip": {
                    "unit": "microseconds",
                    "raw_samples": [400, 500, 600],
                },
                "client_to_host_isolated_application_round_trip": {
                    "unit": "microseconds",
                    "raw_samples": [400, 500, 600],
                },
            },
        ]

        summary = MODULE.summarize_reports(reports)

        control = summary["control_completion_wait"]
        self.assertEqual(control["independent_run_values"], [20, 200])
        self.assertEqual(control["independent_run_median"], 110)
        self.assertEqual(control["independent_run_median_absolute_deviation"], 90)
        self.assertEqual(
            control["independent_run_median_bootstrap_95ci"],
            {"lower": 20, "upper": 200, "resamples": 10_000},
        )
        self.assertEqual(control["pooled_sample_count"], 6)
        self.assertEqual(control["pooled_p50"], 30)
        self.assertEqual(
            control["pooled_summary"],
            {
                "samples": 6,
                "minimum": 10,
                "p50": 30,
                "p95": 300,
                "p99": 300,
                "maximum": 300,
            },
        )
        self.assertFalse(control["pooled_samples_are_independent"])

    def test_loaded_and_native_rtt_are_diagnostics_not_target_evidence(self):
        baseline = [network_load_report() for _ in range(20)]
        candidate = [
            network_load_report(source_commit="def") for _ in range(20)
        ]
        for report in baseline:
            set_control_completion_wait(report, [20, 30, 40])
            set_native_round_trip(report, [1, 2, 3])
            set_application_round_trip(report, 300)
            set_isolated_application_round_trip(report, 300)
        for report in candidate:
            set_control_completion_wait(report, [10, 15, 20])
            set_native_round_trip(report, [0, 1, 2])
            set_application_round_trip(report, 600)
            set_isolated_application_round_trip(report, 150)

        comparison = MODULE.compare_report_sets(
            baseline,
            candidate,
            expected_topology="udp",
            paired=True,
            verified_paired_experiment=True,
        )

        self.assertTrue(comparison["comparison_valid"])
        self.assertTrue(comparison["statistically_valid"])
        self.assertEqual(
            comparison["metrics"]["control_completion_wait"]["target_result"],
            "met",
        )
        ping = comparison["metrics"]["client_to_host_round_trip"]
        self.assertEqual(ping["target_result"], "diagnostic-only")
        self.assertIn("millisecond quantization", ping["target_reason"])
        application = comparison["metrics"][
            "client_to_host_application_round_trip"
        ]
        self.assertEqual(application["target_result"], "diagnostic-only")
        self.assertIn("loaded 24-client fanout", application["target_reason"])
        isolated = comparison["metrics"][
            "client_to_host_isolated_application_round_trip"
        ]
        self.assertEqual(isolated["target_result"], "met")
        self.assertTrue(comparison["meets_target"])
        self.assertEqual(comparison["target_result"], "met")

    def test_paired_comparison_uses_within_pair_ratios(self):
        baseline = [network_load_report() for _ in range(3)]
        candidate = [
            network_load_report(source_commit="def") for _ in range(3)
        ]
        for report, value in zip(baseline, (10, 100, 100), strict=True):
            set_control_completion_wait(report, [value])
        for report, value in zip(candidate, (9, 10, 50), strict=True):
            set_control_completion_wait(report, [value])

        comparison = MODULE.compare_report_sets(
            baseline,
            candidate,
            expected_topology="udp",
            paired=True,
            verified_paired_experiment=True,
        )

        control = comparison["metrics"]["control_completion_wait"]
        self.assertEqual(
            comparison["comparison_design"],
            "verified-direct-interleaved-paired",
        )
        self.assertEqual(control["paired_run_ratios"], [0.9, 0.1, 0.5])
        self.assertEqual(control["candidate_to_baseline_ratio"], 0.5)


class ReportValidationTests(unittest.TestCase):
    def test_runner_contract_version_bumps_for_schema6_target_semantics(self):
        self.assertEqual(MODULE.RUNNER_SCHEMA, 5)

    def test_accepts_the_schema6_isolated_application_rtt_contract(self):
        report = network_load_report()

        MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_an_isolated_application_rtt_sample_count_mismatch(self):
        report = network_load_report()
        replace_metric_samples(
            report["client_to_host_isolated_application_round_trip"],
            [200] * 255,
        )

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "has 255 raw samples, expected 256",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_wrong_isolated_application_rtt_routes(self):
        report = network_load_report()
        report[
            "isolated_application_round_trip_preferred_message_routes"
        ][0]["protocol"] = "tcp"

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "isolated application RTT preferred message routes differ",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_a_missing_isolated_cleanup_assertion(self):
        report = network_load_report()
        report["assertions"] = [
            assertion
            for assertion in report["assertions"]
            if assertion["name"] != "isolated-ping-clean-shutdown"
        ]

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "assertion names differ from the complete harness contract",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_wrong_isolated_application_rtt_metadata(self):
        report = network_load_report()
        report["isolated_application_round_trip_warmup_samples"] = 127

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "isolated_application_round_trip_warmup_samples is 127, "
            "expected 128",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_wrong_isolated_application_rtt_unit(self):
        report = network_load_report()
        report["client_to_host_isolated_application_round_trip"][
            "unit"
        ] = "milliseconds"

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "unit is 'milliseconds', expected 'microseconds'",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_an_isolated_application_rtt_summary_mismatch(self):
        report = network_load_report()
        report["client_to_host_isolated_application_round_trip"][
            "summary"
        ]["p99"] += 1

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "isolated_application_round_trip summary differs from raw samples",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_a_metric_summary_not_recomputed_from_raw_samples(self):
        report = network_load_report()
        report["control_completion_wait"]["summary"]["p50"] += 1

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "control_completion_wait summary differs from raw samples",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_a_metric_summary_with_a_non_integer_percentile(self):
        report = network_load_report()
        summary = report["control_completion_wait"]["summary"]
        summary["p50"] = float(summary["p50"])

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "control_completion_wait summary differs from raw samples",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_a_per_client_summary_not_recomputed_from_raw_samples(self):
        report = network_load_report()
        report["client_to_host_round_trip_by_client"][0]["metrics"][
            "summary"
        ]["p99"] += 1

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "client 1 native RTT summary differs from raw samples",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_ready_delivery_counts_inconsistent_with_measured_ticks(self):
        report = network_load_report()
        report["observed_ready_deliveries"] -= 1

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "ready delivery counts are inconsistent with measured ticks",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_measurement_wall_elapsed_outside_one_control_interval(self):
        report = network_load_report()
        report["measurement_wall_elapsed_ms"] = 60_057

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "measurement wall elapsed is outside",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_route_topology_inconsistent_with_selected_transport(self):
        report = network_load_report()
        report["final_route_peers"][1][1].pop()

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "final route peers differ from the exact udp topology",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_prebuilt_sidecar_is_authoritative_when_runtime_git_is_unavailable(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            write_prebuilt_provenance(binary)
            build = MODULE.prebuilt_build_record(binary, "release")
            report = network_load_report(source_commit=None)
            report["fingerprint"]["content_revision"] = None
            report["fingerprint"]["rustc"] = None

            MODULE.validate_report(report, expected_topology="udp")
            MODULE.validate_report_against_build(report, build)

    def test_report_profile_must_match_authoritative_build_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            write_prebuilt_provenance(binary, cargo_profile="release")
            build = MODULE.prebuilt_build_record(binary, "release")
            report = network_load_report()
            report["fingerprint"]["cargo_profile"] = "release"

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "diagnostic Cargo profile label differs",
            ):
                MODULE.validate_report_against_build(report, build)

    def test_report_profile_label_comes_from_cargo_artifact_debug_assertions(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            provenance = write_prebuilt_provenance(
                binary, cargo_profile="release"
            )
            provenance["inputs"]["effective_profile"][
                "cargo_artifact_profile"
            ]["debug_assertions"] = False
            MODULE.write_json(
                MODULE.provenance_sidecar_path(binary), provenance
            )
            build = MODULE.prebuilt_build_record(binary, "release")
            report = network_load_report()
            report["fingerprint"]["cargo_profile"] = "release"

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "diagnostic Cargo profile label differs",
            ):
                MODULE.validate_report_against_build(report, build)

            provenance["inputs"]["effective_profile"][
                "cargo_artifact_profile"
            ]["debug_assertions"] = True
            MODULE.write_json(
                MODULE.provenance_sidecar_path(binary), provenance
            )
            debug_build = MODULE.prebuilt_build_record(binary, "release")
            report["fingerprint"][
                "cargo_profile"
            ] = "test-with-debug-assertions"
            MODULE.validate_report_against_build(report, debug_build)

    def test_report_validation_against_build_still_requires_provenance(self):
        report = network_load_report(source_commit="b" * 40)
        build = {
            "provenance": {
                "source": {},
            }
        }

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "no source commit",
        ):
            MODULE.validate_report_against_build(report, build)

    def test_rejects_a_retained_run_directory_that_escapes_its_cohort(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cohort = root / "cohort"
            summary = write_cohort(cohort)
            outside = root / "outside"
            (cohort / "run-001").rename(outside)
            summary["runs"][0]["directory"] = "../outside"
            (cohort / "benchmark-summary.json").write_text(
                json.dumps(summary), encoding="utf-8"
            )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "run directory",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_an_internal_retained_run_directory_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            cohort = Path(temporary) / "cohort"
            write_cohort(cohort)
            run_directory = cohort / "run-001"
            actual_directory = cohort / "actual-run"
            run_directory.rename(actual_directory)
            try:
                run_directory.symlink_to(
                    actual_directory, target_is_directory=True
                )
            except OSError as error:
                self.skipTest(f"symlinks are unavailable: {error}")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "run directory is a symlink",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_a_retained_report_changed_after_summary(self):
        with tempfile.TemporaryDirectory() as temporary:
            cohort = Path(temporary) / "cohort"
            write_cohort(cohort)
            report_path = cohort / "run-001/report.json"
            report_path.write_text("{}", encoding="utf-8")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "report.json hash differs",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_a_retained_cohort_without_exact_binary_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            cohort = Path(temporary) / "cohort"
            summary = write_cohort(cohort)
            Path(summary["binary"]["path"]).unlink()

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "retained benchmark binary is missing",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_a_retained_cohort_without_exact_provenance_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            cohort = Path(temporary) / "cohort"
            write_cohort(cohort)
            (cohort / "build-provenance.json").unlink()

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "retained build provenance is missing",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_a_retained_execution_with_a_different_post_run_binary_hash(self):
        with tempfile.TemporaryDirectory() as temporary:
            cohort = Path(temporary) / "cohort"
            summary = write_cohort(cohort)
            execution_path = cohort / "run-001/execution.json"
            execution = json.loads(execution_path.read_text(encoding="utf-8"))
            execution["binary_sha256_after"] = "c" * 64
            execution_path.write_text(
                json.dumps(execution), encoding="utf-8"
            )
            summary["runs"][0]["execution_sha256"] = MODULE.sha256_file(
                execution_path
            )
            (cohort / "benchmark-summary.json").write_text(
                json.dumps(summary), encoding="utf-8"
            )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "execution binary_sha256_after differs",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_a_retained_summary_symlink_that_escapes_its_cohort(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cohort = root / "cohort"
            write_cohort(cohort)
            summary_path = cohort / "benchmark-summary.json"
            outside = root / "outside-summary.json"
            summary_path.rename(outside)
            try:
                summary_path.symlink_to(outside)
            except OSError as error:
                self.skipTest(f"symlinks are unavailable: {error}")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "benchmark summary escapes its cohort",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_incomplete_retained_compiler_environment_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            cohort = Path(temporary) / "cohort"
            write_cohort(cohort)
            for artifact_name in (
                "benchmark-summary.json",
                "cohort-metadata.json",
            ):
                artifact_path = cohort / artifact_name
                artifact = json.loads(artifact_path.read_text(encoding="utf-8"))
                provenance = artifact["build"]["provenance"]
                del provenance["environment"]["RUSTFLAGS"]
                artifact["build"]["provenance_sha256"] = MODULE.sha256_bytes(
                    MODULE.canonical_json(provenance).encode("utf-8")
                )
                artifact_path.write_text(
                    json.dumps(artifact), encoding="utf-8"
                )
            summary = json.loads(
                (cohort / "benchmark-summary.json").read_text(encoding="utf-8")
            )
            MODULE.write_json(
                cohort / "build-provenance.json",
                summary["build"]["provenance"],
            )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "provenance environment is invalid",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_a_passing_run_record_with_a_failure_reason(self):
        with tempfile.TemporaryDirectory() as temporary:
            cohort = Path(temporary) / "cohort"
            summary = write_cohort(cohort)
            summary["runs"][0]["failure"] = "forged failure"
            (cohort / "benchmark-summary.json").write_text(
                json.dumps(summary), encoding="utf-8"
            )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "passing run record contains a failure",
            ):
                MODULE.load_cohort_reports(cohort)

    def test_rejects_a_different_workload_shape(self):
        report = network_load_report()
        report["workload"] = "different workload"

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "workload is",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_a_report_with_a_different_measurement_duration(self):
        report = network_load_report()
        report["requested_measurement_ms"] = 10_000
        report["minimum_native_control_ticks"] = 179
        report["authoritative_duration"] = False

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "requested_measurement_ms",
        ):
            MODULE.validate_report(
                report,
                expected_topology="udp",
                expected_measurement_seconds=None,
            )

    def test_rejects_a_report_missing_a_required_harness_assertion(self):
        report = network_load_report()
        report["assertions"] = report["assertions"][:-1]

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "assertion names differ",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_an_incomplete_application_round_trip_workload(self):
        report = network_load_report()
        metrics = report["client_to_host_application_round_trip_by_client"][0][
            "metrics"
        ]
        replace_metric_samples(metrics, metrics["raw_samples"][:-1])

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "client 1 application RTT samples",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_native_control_wait_count_that_does_not_match_participants(self):
        report = network_load_report()
        replace_metric_samples(report["native_control_wait"], [0] * 24)

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "native_control_wait has 24 raw samples, expected 625",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_accepts_native_wait_for_every_process_state_in_runtime_group(self):
        report = network_load_report()
        report["runtime_samples"] = runtime_sample_group("udp")
        replace_metric_samples(report["native_control_wait"], [0] * 625)

        MODULE.validate_report(report, expected_topology="udp")

    def test_runtime_route_count_is_diagnostic_not_message_topology(self):
        report = network_load_report()
        for sample in report["runtime_samples"]:
            sample["route_count"] += 1

        MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_malformed_runtime_elapsed_without_a_type_error(self):
        report = network_load_report()
        report["runtime_samples"][0]["elapsed_ms"] = []

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "runtime_samples group elapsed time is invalid",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_rejects_a_report_without_a_target_architecture(self):
        report = network_load_report()
        report["fingerprint"]["target_arch"] = None

        with self.assertRaisesRegex(
            MODULE.BenchmarkFailure,
            "fingerprint target_arch",
        ):
            MODULE.validate_report(report, expected_topology="udp")

    def test_runner_machine_fingerprint_has_no_nullable_identity_fields(self):
        fingerprint = MODULE.runtime_machine_fingerprint()

        for field in (
            "system",
            "release",
            "machine",
            "processor",
            "python_implementation",
            "python_version",
        ):
            self.assertIsInstance(fingerprint[field], str)
            self.assertTrue(fingerprint[field])

    def test_runtime_host_observations_capture_affinity_resources_and_power(self):
        observations = MODULE.runtime_host_observations()

        self.assertIsInstance(observations["logical_cpu_count"], int)
        self.assertGreater(observations["logical_cpu_count"], 0)
        self.assertIsInstance(observations["process_cpu_affinity"], dict)
        self.assertIn("status", observations["process_cpu_affinity"])
        self.assertIsInstance(observations["load_average"], dict)
        self.assertIn("status", observations["load_average"])
        self.assertIsInstance(observations["power"], dict)
        self.assertIn("status", observations["power"])

    def test_runtime_rustc_probe_may_change_between_prebuilt_runs(self):
        reports = [network_load_report(), network_load_report()]
        reports[0]["fingerprint"]["rustc"] = None
        reports[1]["fingerprint"]["rustc"] = "later runtime rustc"

        MODULE.validate_report_set(reports, expected_topology="udp")

    def test_runtime_source_probe_may_change_between_prebuilt_runs(self):
        reports = [
            network_load_report(source_commit=None),
            network_load_report(source_commit="later checkout state"),
        ]

        identity = MODULE.validate_report_set(
            reports, expected_topology="udp"
        )

        self.assertIsNone(identity["fingerprint"]["source_commit"])

    def test_compares_independent_run_medians_across_source_commits(self):
        baseline = [network_load_report(), network_load_report()]
        candidate = [
            network_load_report(source_commit="def"),
            network_load_report(source_commit="def"),
        ]
        set_control_completion_wait(baseline[1], [30, 40, 50])
        set_native_round_trip(baseline[1], [3, 4, 5])
        set_control_completion_wait(candidate[0], [5, 10, 15])
        set_control_completion_wait(candidate[1], [15, 20, 25])
        set_native_round_trip(candidate[0], [0, 1, 2])
        set_native_round_trip(candidate[1], [1, 2, 3])

        comparison = MODULE.compare_report_sets(
            baseline,
            candidate,
            expected_topology="udp",
        )

        control = comparison["metrics"]["control_completion_wait"]
        self.assertEqual(control["baseline_independent_run_median"], 30)
        self.assertEqual(control["candidate_independent_run_median"], 15)
        self.assertEqual(control["candidate_to_baseline_ratio"], 0.5)
        self.assertEqual(control["improvement_percent"], 50.0)

    def test_compares_retained_successful_cohort_directories(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cohort_paths = []
            for label, commit, control_samples, binary_sha256 in (
                ("baseline", "a" * 40, [10, 20, 30], "a" * 64),
                ("candidate", "b" * 40, [5, 10, 15], "b" * 64),
            ):
                cohort = root / label
                write_cohort(
                    cohort,
                    source_commit=commit,
                    control_samples=control_samples,
                    binary_sha256=binary_sha256,
                )
                cohort_paths.append(cohort)

            comparison = MODULE.compare_cohort_directories(
                cohort_paths[0],
                cohort_paths[1],
                expected_topology="udp",
            )

            control = comparison["metrics"]["control_completion_wait"]
            self.assertEqual(control["candidate_to_baseline_ratio"], 0.5)
            self.assertEqual(
                comparison["baseline"]["binary_sha256"],
                MODULE.sha256_bytes(("a" * 64).encode("ascii")),
            )
            self.assertEqual(
                comparison["candidate"]["binary_sha256"],
                MODULE.sha256_bytes(("b" * 64).encode("ascii")),
            )
            baseline_summary = json.loads(
                (cohort_paths[0] / "benchmark-summary.json").read_text(
                    encoding="utf-8"
                )
            )
            baseline_build = baseline_summary["build"]
            self.assertEqual(
                comparison["baseline"]["build_provenance_sha256"],
                baseline_build["provenance_sha256"],
            )
            self.assertEqual(
                comparison["baseline"]["source_provenance_sha256"],
                MODULE.sha256_bytes(
                    MODULE.canonical_json(
                        baseline_build["provenance"]["source"]
                    ).encode("utf-8")
                ),
            )

    def test_rejects_cohorts_built_with_different_cargo_profiles(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cohorts = []
            for index, (label, profile) in enumerate(
                (("baseline", "release"), ("candidate", "test"))
            ):
                cohort = root / label
                write_cohort(
                    cohort,
                    cargo_profile=profile,
                    binary_sha256=("a" if index == 0 else "b") * 64,
                )
                cohorts.append(cohort)

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "Cargo profiles differ",
            ):
                MODULE.compare_cohort_directories(
                    cohorts[0], cohorts[1], expected_topology="udp"
                )

    def test_rejects_cohorts_measured_on_different_runtime_machines(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            write_cohort(baseline, binary_sha256="a" * 64)
            summary = write_cohort(candidate, binary_sha256="b" * 64)
            metadata_path = candidate / "cohort-metadata.json"
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            metadata["runtime_machine"]["processor"] = "Different CPU"
            summary["runtime_machine"]["processor"] = "Different CPU"
            metadata_path.write_text(json.dumps(metadata), encoding="utf-8")
            (candidate / "benchmark-summary.json").write_text(
                json.dumps(summary), encoding="utf-8"
            )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "runtime machines differ",
            ):
                MODULE.compare_cohort_directories(
                    baseline, candidate, expected_topology="udp"
                )

    def test_rejects_unrelated_cohorts_with_a_forged_paired_schedule_string(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            write_cohort(baseline, binary_sha256="a" * 64)
            write_cohort(
                candidate,
                source_commit="b" * 40,
                binary_sha256="b" * 64,
            )
            for cohort in (baseline, candidate):
                metadata_path = cohort / "cohort-metadata.json"
                metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
                metadata["configuration"]["schedule"] = "counterbalanced AB/BA"
                metadata_path.write_text(
                    json.dumps(metadata), encoding="utf-8"
                )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "unverified paired schedule claim",
            ):
                MODULE.compare_cohort_directories(
                    baseline, candidate, expected_topology="udp"
                )


class BinaryExecutionTests(unittest.TestCase):
    def test_paired_experiment_manifest_prespecifies_randomized_balanced_orders(self):
        manifest = MODULE.paired_experiment_manifest(
            pair_count=20,
            topology="udp",
            measurement_seconds=None,
            timeout_seconds=300,
            cargo_profile="release",
            experiment_id="1" * 32,
            randomization_seed="2" * 32,
        )

        orders = [pair["order"] for pair in manifest["pairs"]]
        self.assertEqual(orders.count("AB"), 10)
        self.assertEqual(orders.count("BA"), 10)
        self.assertNotEqual(orders, ["AB", "BA"] * 10)
        self.assertEqual(manifest["predeclared_pair_count"], 20)
        self.assertEqual(
            manifest["runner_script_sha256"],
            MODULE.runner_script_sha256(),
        )
        self.assertEqual(
            [step["global_sequence"] for step in manifest["schedule"]],
            list(range(1, 41)),
        )
        for pair_index in range(1, 21):
            pair_steps = [
                step
                for step in manifest["schedule"]
                if step["pair_index"] == pair_index
            ]
            self.assertEqual(len(pair_steps), 2)
            self.assertEqual(
                [step["position"] for step in pair_steps], [1, 2]
            )

    def test_paired_prebuilt_binaries_reject_different_compiler_flags(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            baseline.write_bytes(b"baseline")
            candidate.write_bytes(b"candidate")
            write_prebuilt_provenance(baseline)
            provenance = write_prebuilt_provenance(candidate)
            provenance["environment"]["RUSTFLAGS"] = "-C target-cpu=native"
            MODULE.write_json(
                MODULE.provenance_sidecar_path(candidate), provenance
            )

            with mock.patch.object(MODULE, "run_one") as run_one:
                with self.assertRaisesRegex(
                    MODULE.BenchmarkFailure,
                    "build environments differ",
                ):
                    MODULE.run_paired_binaries(
                        baseline_binary=baseline,
                        candidate_binary=candidate,
                        output_directory=root / "comparison",
                        repository_root=root,
                        runs=1,
                        topology="udp",
                        measurement_seconds=None,
                        timeout_seconds=300,
                        cargo_profile="release",
                    )

            run_one.assert_not_called()

    def test_same_profile_name_rejects_different_effective_profile_settings(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            baseline.write_bytes(b"baseline")
            candidate.write_bytes(b"candidate")
            write_prebuilt_provenance(baseline)
            provenance = write_prebuilt_provenance(candidate)
            provenance["inputs"]["effective_profile"][
                "workspace_profile_tables"
            ]["release"]["lto"] = False
            MODULE.write_json(
                MODULE.provenance_sidecar_path(candidate), provenance
            )

            baseline_build = MODULE.prebuilt_build_record(
                baseline, "release"
            )
            candidate_build = MODULE.prebuilt_build_record(
                candidate, "release"
            )
            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "build environments differ",
            ):
                MODULE.require_comparable_builds(
                    baseline_build, candidate_build
                )

    def test_paired_prebuilt_binaries_reject_different_dependency_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            baseline.write_bytes(b"baseline")
            candidate.write_bytes(b"candidate")
            write_prebuilt_provenance(baseline)
            provenance = write_prebuilt_provenance(candidate)
            provenance["inputs"]["cargo_lock_sha256"] = "9" * 64
            provenance["inputs"]["manifest_files"]["Cargo.toml"] = "8" * 64
            MODULE.write_json(
                MODULE.provenance_sidecar_path(candidate), provenance
            )

            baseline_build = MODULE.prebuilt_build_record(
                baseline, "release"
            )
            candidate_build = MODULE.prebuilt_build_record(
                candidate, "release"
            )
            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "build environments differ",
            ):
                MODULE.require_comparable_builds(
                    baseline_build, candidate_build
                )

    def test_authoritative_paired_experiment_rejects_unarchived_dirty_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            write_prebuilt_provenance(binary)
            build = MODULE.prebuilt_build_record(binary, "release")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "authoritative paired experiment requires clean source",
            ):
                MODULE.require_authoritative_source_evidence(
                    [build], pair_count=20
                )

    def test_authoritative_content_requires_a_matching_parent_gitlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            provenance = write_prebuilt_provenance(binary)
            provenance["source"].update(
                {
                    "tracked_patch_sha256": EMPTY_SHA256,
                    "dirty": False,
                }
            )
            provenance["content"].update(
                {
                    "parent_gitlink_mode": "100644",
                    "parent_gitlink_type": "blob",
                }
            )
            MODULE.write_json(
                MODULE.provenance_sidecar_path(binary), provenance
            )
            build = MODULE.prebuilt_build_record(binary, "release")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "real 160000 parent gitlink",
            ):
                MODULE.require_authoritative_source_evidence(
                    [build], pair_count=20
                )

    def test_authoritative_content_head_must_equal_parent_gitlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            provenance = write_prebuilt_provenance(binary)
            provenance["source"].update(
                {
                    "tracked_patch_sha256": EMPTY_SHA256,
                    "dirty": False,
                }
            )
            provenance["content"]["parent_gitlink_revision"] = "8" * 40
            MODULE.write_json(
                MODULE.provenance_sidecar_path(binary), provenance
            )
            build = MODULE.prebuilt_build_record(binary, "release")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "content HEAD to match the parent gitlink",
            ):
                MODULE.require_authoritative_source_evidence(
                    [build], pair_count=20
                )

    def test_paired_prebuilt_binaries_require_identical_benchmark_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            baseline.write_bytes(b"baseline")
            candidate.write_bytes(b"candidate")
            write_prebuilt_provenance(baseline)
            provenance = write_prebuilt_provenance(candidate)
            provenance["inputs"]["benchmark_contract_files"][
                "crates/clonk-network/tests/network_load_24.rs"
            ] = "9" * 64
            MODULE.write_json(
                MODULE.provenance_sidecar_path(candidate), provenance
            )

            with mock.patch.object(MODULE, "run_one") as run_one:
                with self.assertRaisesRegex(
                    MODULE.BenchmarkFailure,
                    "build environments differ",
                ):
                    MODULE.run_paired_binaries(
                        baseline_binary=baseline,
                        candidate_binary=candidate,
                        output_directory=root / "comparison",
                        repository_root=root,
                        runs=1,
                        topology="udp",
                        measurement_seconds=None,
                        timeout_seconds=300,
                        cargo_profile="release",
                    )

            run_one.assert_not_called()

    def test_retained_directory_comparison_enforces_requested_run_count_option(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            write_cohort(baseline, binary_sha256="a" * 64)
            write_cohort(
                candidate,
                source_commit="b" * 40,
                binary_sha256="b" * 64,
            )

            exit_code = MODULE.main(
                [
                    "compare",
                    str(baseline),
                    str(candidate),
                    "--runs",
                    "2",
                    "--output",
                    str(root / "comparison"),
                ]
            )

            self.assertEqual(exit_code, 2)

    def test_valid_but_statistically_indeterminate_comparison_exits_one(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            write_cohort(baseline, binary_sha256="a" * 64)
            write_cohort(
                candidate,
                source_commit="b" * 40,
                binary_sha256="b" * 64,
            )

            exit_code = MODULE.main(
                [
                    "compare",
                    str(baseline),
                    str(candidate),
                    "--runs",
                    "1",
                    "--output",
                    str(root / "comparison"),
                ]
            )

            self.assertEqual(exit_code, 1)
            comparison = json.loads(
                (root / "comparison/comparison.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertTrue(comparison["comparison_valid"])
            self.assertEqual(comparison["target_result"], "indeterminate")

    def test_non_authoritative_duration_is_validated_against_the_requested_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")

            def process(command, **kwargs):
                report = network_load_report()
                set_measurement_duration(
                    report,
                    milliseconds=10_000,
                    measured_ticks=179,
                )
                report["authoritative_duration"] = False
                Path(kwargs["env"]["LC_NETWORK_LOAD_METRICS"]).write_text(
                    json.dumps(report), encoding="utf-8"
                )
                return SimpleNamespace(returncode=0, stdout="", stderr="")

            with mock.patch.object(
                MODULE.subprocess, "run", side_effect=process
            ):
                summary = MODULE.run_cohort(
                    binary=binary,
                    output_directory=root / "output",
                    repository_root=root,
                    label="smoke",
                    runs=1,
                    topology="udp",
                    measurement_seconds=10,
                    timeout_seconds=300,
                    build={
                        "cargo_profile": "release",
                        "provenance": {
                            "source": {"commit": "abc"},
                            "content": {"head": "content"},
                            "build": {"cargo_profile": "test"},
                            "inputs": {
                                "effective_profile": {
                                    "cargo_artifact_profile": {
                                        "debug_assertions": False,
                                    }
                                }
                            },
                        },
                    },
                )

            self.assertEqual(summary["result"], "pass")

    def test_prebuilt_provenance_requires_source_toolchain_and_build_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            provenance = write_prebuilt_provenance(binary)
            del provenance["source"]["tracked_patch_sha256"]
            MODULE.write_json(MODULE.provenance_sidecar_path(binary), provenance)

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "source is missing",
            ):
                MODULE.load_prebuilt_provenance(binary)

    def test_prebuilt_provenance_requires_cargo_debug_assertion_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            provenance = write_prebuilt_provenance(binary)
            del provenance["inputs"]["effective_profile"][
                "cargo_artifact_profile"
            ]["debug_assertions"]
            MODULE.write_json(
                MODULE.provenance_sidecar_path(binary), provenance
            )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "Cargo artifact debug-assertion state",
            ):
                MODULE.load_prebuilt_provenance(binary)

    def test_prebuilt_provenance_rejects_dirty_state_inconsistent_with_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            provenance = write_prebuilt_provenance(binary)
            provenance["source"]["dirty"] = False
            MODULE.write_json(MODULE.provenance_sidecar_path(binary), provenance)

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "source dirty state disagrees with recorded evidence",
            ):
                MODULE.load_prebuilt_provenance(binary)

    def test_prebuilt_provenance_must_match_the_executing_runner_script(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "integration-test"
            binary.write_bytes(b"exact binary")
            provenance = write_prebuilt_provenance(binary)
            provenance["build"]["runner_script_sha256"] = "0" * 64
            MODULE.write_json(
                MODULE.provenance_sidecar_path(binary), provenance
            )

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "runner script hash differs from the executing runner",
            ):
                MODULE.prebuilt_build_record(binary, "release")

    def test_build_provenance_hashes_tracked_and_untracked_source_without_status(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".cargo").mkdir()
            (root / "crates/clonk-network/src").mkdir(parents=True)
            (root / "crates/clonk-network/tests").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\n[profile.release]\nlto = "thin"\n',
                encoding="utf-8",
            )
            (root / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
            (root / ".cargo/config.toml").write_text(
                "[build]\n", encoding="utf-8"
            )
            cargo_home = root / ".cargo-home"
            cargo_home.mkdir()
            (cargo_home / "config.toml").write_text(
                '[build]\ntarget-dir = "shared-target"\n',
                encoding="utf-8",
            )
            selected_rustc = cargo_home / "selected-rustc"
            selected_rustc.write_text(
                "#!/bin/sh\nprintf 'selected rustc -Vv\\n'\n",
                encoding="utf-8",
            )
            selected_rustc.chmod(0o755)
            source = root / "crates/clonk-network/src/lib.rs"
            source.write_text("pub fn baseline() {}\n", encoding="utf-8")
            (root / "crates/clonk-network/tests/network_load_24.rs").write_text(
                "// benchmark contract\n", encoding="utf-8"
            )
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(
                [
                    "git",
                    "-c",
                    "user.name=Benchmark Test",
                    "-c",
                    "user.email=benchmark@example.invalid",
                    "commit",
                    "-qm",
                    "initial",
                ],
                cwd=root,
                check=True,
            )
            external_content = root / ".content-target"
            external_content.mkdir()
            (external_content / "Scenario.txt").write_text(
                "[Head]\n", encoding="utf-8"
            )
            subprocess.run(["git", "init", "-q"], cwd=external_content, check=True)
            subprocess.run(["git", "add", "."], cwd=external_content, check=True)
            subprocess.run(
                [
                    "git",
                    "-c",
                    "user.name=Benchmark Test",
                    "-c",
                    "user.email=benchmark@example.invalid",
                    "commit",
                    "-qm",
                    "content",
                ],
                cwd=external_content,
                check=True,
            )
            (root / "content").symlink_to(external_content, target_is_directory=True)

            with mock.patch.dict(
                MODULE.os.environ,
                {
                    "CARGO_PROFILE_RELEASE_LTO": "thin",
                    "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER": "clang",
                    "RUSTC": str(selected_rustc),
                    "CARGO_HOME": str(cargo_home),
                },
            ):
                clean = MODULE.collect_build_provenance_inputs(
                    root, cargo_profile="release"
                )
            source.write_text("pub fn candidate() {}\n", encoding="utf-8")
            tracked = MODULE.collect_build_provenance_inputs(
                root, cargo_profile="release"
            )
            (root / "crates/clonk-network/src/new.rs").write_text(
                "pub fn added() {}\n", encoding="utf-8"
            )
            untracked = MODULE.collect_build_provenance_inputs(
                root, cargo_profile="release"
            )
            (root / "linked-source").symlink_to(
                root / "crates/clonk-network/src", target_is_directory=True
            )
            symlinked = MODULE.collect_build_provenance_inputs(
                root, cargo_profile="release"
            )

            self.assertFalse(clean["source"]["dirty"])
            self.assertTrue(tracked["source"]["dirty"])
            self.assertNotEqual(
                clean["source"]["tracked_patch_sha256"],
                tracked["source"]["tracked_patch_sha256"],
            )
            self.assertNotEqual(
                tracked["source"]["untracked_inputs_sha256"],
                untracked["source"]["untracked_inputs_sha256"],
            )
            self.assertEqual(
                set(untracked["inputs"]["benchmark_contract_files"]),
                {"crates/clonk-network/tests/network_load_24.rs"},
            )
            self.assertEqual(
                clean["environment"]["CARGO_PROFILE_RELEASE_LTO"], "thin"
            )
            self.assertEqual(
                clean["environment"][
                    "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER"
                ],
                "clang",
            )
            self.assertEqual(
                clean["environment"]["RUSTC"], str(selected_rustc)
            )
            self.assertEqual(
                clean["toolchain"]["rustc_vv"], "selected rustc -Vv"
            )
            self.assertEqual(
                set(clean["inputs"]["cargo_configuration_files"]),
                {"workspace:.cargo/config.toml", "cargo-home:config.toml"},
            )
            self.assertEqual(
                clean["inputs"]["effective_profile"],
                {
                    "selected_profile": "release",
                    "workspace_profile_tables": {
                        "release": {"lto": "thin"}
                    },
                    "cargo_artifact_profile": {},
                },
            )
            self.assertEqual(
                clean["content"]["head"],
                subprocess.run(
                    ["git", "rev-parse", "HEAD"],
                    cwd=external_content,
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout.strip(),
            )
            self.assertIn(
                "linked-source",
                symlinked["source"]["untracked_input_files"],
            )

    def test_successful_process_without_a_report_is_a_failed_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")

            with mock.patch.object(
                MODULE.subprocess,
                "run",
                return_value=SimpleNamespace(
                    returncode=0,
                    stdout="",
                    stderr="",
                ),
            ):
                result = MODULE.run_one(
                    binary=binary,
                    run_directory=root / "run-001",
                    repository_root=root,
                    topology="udp",
                    measurement_seconds=None,
                    expected_binary_sha256=MODULE.sha256_file(binary),
                    timeout_seconds=300,
                )

            self.assertFalse(result["passed"])
            self.assertEqual(
                result["execution"]["failure"],
                "test binary exited successfully without a report",
            )

    def test_run_fails_if_the_executable_changes_during_the_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")
            expected_digest = MODULE.sha256_file(binary)

            def process(command, **kwargs):
                binary.write_bytes(b"changed binary")
                Path(kwargs["env"]["LC_NETWORK_LOAD_METRICS"]).write_text(
                    json.dumps(network_load_report()), encoding="utf-8"
                )
                return SimpleNamespace(returncode=0, stdout="", stderr="")

            with mock.patch.object(
                MODULE.subprocess, "run", side_effect=process
            ):
                result = MODULE.run_one(
                    binary=binary,
                    run_directory=root / "run-001",
                    repository_root=root,
                    topology="udp",
                    measurement_seconds=None,
                    expected_binary_sha256=expected_digest,
                    timeout_seconds=300,
                )

            self.assertFalse(result["passed"])
            execution = result["execution"]
            self.assertEqual(execution["binary_sha256_before"], expected_digest)
            self.assertNotEqual(
                execution["binary_sha256_after"], expected_digest
            )
            self.assertIn("changed during run", execution["failure"])

    def test_cohort_requires_exactly_the_requested_number_of_reports(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            state = {
                "directory": root,
                "binary_details": {"sha256": "a" * 64},
                "build": {"cargo_profile": "release"},
                "runtime_machine": MODULE.runtime_machine_fingerprint(),
                "reports": [network_load_report()],
                "records": [
                    {
                        "run": 1,
                        "directory": "run-001",
                        "passed": True,
                        "failure": None,
                    },
                    {
                        "run": 2,
                        "directory": "run-002",
                        "passed": True,
                        "failure": None,
                    },
                ],
            }

            summary = MODULE._finalize_cohort(
                state=state,
                label="candidate",
                runs=2,
                topology="udp",
            )

            self.assertEqual(summary["result"], "fail")
            self.assertIn("expected 2 successful reports", summary["cohort_failure"])

    def test_prebuilt_binary_requires_a_build_provenance_sidecar(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")

            with mock.patch.object(MODULE, "run_cohort") as cohort:
                exit_code = MODULE.main(
                    [
                        "run",
                        "--repository-root",
                        str(root),
                        "--binary",
                        str(binary),
                        "--output",
                        str(root / "output"),
                    ]
                )

            self.assertEqual(exit_code, 2)
            cohort.assert_not_called()

    def test_prebuilt_binary_requires_an_explicit_cargo_profile(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")
            write_prebuilt_provenance(binary)

            with mock.patch.object(MODULE, "run_cohort") as cohort:
                exit_code = MODULE.main(
                    [
                        "run",
                        "--repository-root",
                        str(root),
                        "--binary",
                        str(binary),
                        "--output",
                        str(root / "output"),
                    ]
                )

            self.assertEqual(exit_code, 2)
            cohort.assert_not_called()

    def test_invalid_paired_comparison_exits_two(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = root / "baseline"
            candidate = root / "candidate"
            baseline.write_bytes(b"baseline")
            candidate.write_bytes(b"candidate")

            with mock.patch.object(
                MODULE,
                "run_paired_binaries",
                return_value={
                    "comparison_valid": False,
                    "statistically_valid": False,
                    "target_result": "invalid",
                    "metrics": {},
                },
            ):
                exit_code = MODULE.main(
                    [
                        "compare",
                        str(baseline),
                        str(candidate),
                        "--cargo-profile",
                        "release",
                        "--output",
                        str(root / "comparison"),
                    ]
                )

            self.assertEqual(exit_code, 2)

    def test_run_command_defaults_to_twenty_authoritative_udp_runs(self):
        arguments = MODULE.build_argument_parser().parse_args(["run"])

        self.assertEqual(arguments.runs, 20)
        self.assertEqual(arguments.topology, "udp")
        self.assertIsNone(arguments.cargo_profile)
        self.assertIsNone(arguments.measurement_seconds)

    def test_run_command_rejects_a_label_that_is_not_one_safe_component(self):
        with self.assertRaises(SystemExit):
            MODULE.build_argument_parser().parse_args(
                ["run", "--label", "../escaped"]
            )

    def test_cohort_initialization_rejects_an_escaping_internal_label(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")

            with self.assertRaisesRegex(
                MODULE.BenchmarkFailure,
                "one safe filename component",
            ):
                MODULE._initialize_cohort(
                    binary=binary,
                    output_directory=root / "output",
                    label="../escaped",
                    runs=1,
                    topology="udp",
                    measurement_seconds=None,
                    timeout_seconds=300,
                    build={},
                )

            self.assertFalse((root / "escaped-benchmark-binary").exists())

    def test_run_command_builds_once_then_reuses_the_discovered_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")
            output = root / "results"

            with mock.patch.object(
                MODULE,
                "build_test_binary",
                return_value=(binary, {"mode": "cargo-build-once"}),
            ) as build, mock.patch.object(
                MODULE,
                "run_cohort",
                return_value={"result": "pass", "metrics": {}},
            ) as cohort:
                exit_code = MODULE.main(
                    [
                        "run",
                        "--repository-root",
                        str(root),
                        "--output",
                        str(output),
                        "--runs",
                        "2",
                    ]
                )

            self.assertEqual(exit_code, 0)
            build.assert_called_once_with(
                repository_root=root.resolve(),
                cargo_profile="release",
            )
            cohort.assert_called_once()
            self.assertEqual(cohort.call_args.kwargs["binary"], binary)
            self.assertEqual(cohort.call_args.kwargs["runs"], 2)

    def test_paired_binary_schedule_counterbalances_first_position(self):
        self.assertEqual(
            MODULE.counterbalanced_schedule(4),
            [
                ("baseline", 1),
                ("candidate", 1),
                ("candidate", 2),
                ("baseline", 2),
                ("baseline", 3),
                ("candidate", 3),
                ("candidate", 4),
                ("baseline", 4),
            ],
        )

    def test_discovers_the_prebuilt_integration_test_from_cargo_messages(self):
        messages = "\n".join(
            [
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {"kind": ["lib"], "name": "clonk_network"},
                        "executable": None,
                    }
                ),
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {"kind": ["test"], "name": "integration"},
                        "executable": "/tmp/integration-abc",
                    }
                ),
                json.dumps({"reason": "build-finished", "success": True}),
            ]
        )

        self.assertEqual(
            MODULE.discover_test_binary(messages),
            Path("/tmp/integration-abc"),
        )

    def test_records_cargos_effective_profile_for_the_selected_test_artifact(self):
        profile = {
            "opt_level": "3",
            "debuginfo": 0,
            "debug_assertions": False,
            "overflow_checks": False,
            "test": True,
        }
        messages = json.dumps(
            {
                "reason": "compiler-artifact",
                "target": {"kind": ["test"], "name": "integration"},
                "executable": "/tmp/integration-abc",
                "profile": profile,
            }
        )

        binary, observed_profile = MODULE.discover_test_binary_artifact(
            messages
        )

        self.assertEqual(binary, Path("/tmp/integration-abc"))
        self.assertEqual(observed_profile, profile)

    def test_builds_once_and_records_the_exact_binary_and_profile(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-abc"
            binary.write_bytes(b"built binary")
            cargo_stdout = json.dumps(
                {
                    "reason": "compiler-artifact",
                    "target": {"kind": ["test"], "name": "integration"},
                    "executable": str(binary),
                    "profile": {
                        "opt_level": "3",
                        "debuginfo": 0,
                        "test": True,
                    },
                }
            )
            completed = SimpleNamespace(
                returncode=0,
                stdout=cargo_stdout,
                stderr="finished",
            )
            captured_inputs = {
                "source": {
                    "commit": "abc",
                    "head_tree": "tree",
                    "tracked_patch_sha256": "1" * 64,
                    "untracked_inputs_sha256": EMPTY_SHA256,
                    "untracked_input_files": {},
                    "dirty": True,
                },
                "content": {
                    "head": "6" * 40,
                    "tree": "7" * 40,
                    "parent_gitlink_revision": "6" * 40,
                    "tracked_patch_sha256": EMPTY_SHA256,
                    "untracked_inputs_sha256": EMPTY_SHA256,
                    "untracked_input_files": {},
                    "dirty": False,
                },
                "inputs": {
                    "cargo_lock_sha256": "3" * 64,
                    "configuration_files": {},
                    "cargo_configuration_files": {},
                    "manifest_files": {},
                    "benchmark_contract_files": {
                        "crates/clonk-network/tests/network_load_24.rs": "5"
                        * 64,
                    },
                    "effective_profile": {
                        "selected_profile": "test",
                        "workspace_profile_tables": {},
                        "cargo_artifact_profile": {},
                    },
                },
                "toolchain": {
                    "rustc_vv": "rustc 1.2.3",
                    "cargo_version": "cargo 1.2.3",
                },
                "environment": {
                    "CARGO_ENCODED_RUSTFLAGS": None,
                    "RUSTFLAGS": None,
                    "CARGO_BUILD_TARGET": None,
                },
            }

            with mock.patch.object(
                MODULE.subprocess,
                "run",
                return_value=completed,
            ) as cargo, mock.patch.object(
                MODULE,
                "collect_build_provenance_inputs",
                return_value=captured_inputs,
                create=True,
            ) as collect:
                observed_binary, metadata = MODULE.build_test_binary(
                    repository_root=root,
                    cargo_profile="test",
                )

            cargo.assert_called_once()
            command = cargo.call_args.args[0]
            self.assertIn("--no-run", command)
            self.assertEqual(command[command.index("--profile") + 1], "test")
            self.assertEqual(observed_binary, binary.resolve())
            self.assertEqual(metadata["cargo_profile"], "test")
            self.assertEqual(metadata["binary_sha256"], MODULE.sha256_file(binary))
            self.assertEqual(
                collect.call_args_list,
                [
                    mock.call(root.resolve(), cargo_profile="test"),
                    mock.call(root.resolve(), cargo_profile="test"),
                ],
            )
            sidecar = MODULE.provenance_sidecar_path(binary.resolve())
            provenance = json.loads(sidecar.read_text(encoding="utf-8"))
            self.assertEqual(provenance["source"], captured_inputs["source"])
            self.assertEqual(provenance["build"]["cargo_profile"], "test")
            self.assertEqual(
                provenance["binary"]["sha256"], MODULE.sha256_file(binary)
            )
            self.assertEqual(
                provenance["inputs"]["effective_profile"][
                    "cargo_artifact_profile"
                ],
                {"opt_level": "3", "debuginfo": 0, "test": True},
            )

    def test_failed_run_retains_report_stdout_stderr_and_execution_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")
            run_directory = root / "run-001"

            def failed_process(command, **kwargs):
                report_path = Path(
                    kwargs["env"]["LC_NETWORK_LOAD_METRICS"]
                )
                report_path.write_text(
                    json.dumps(network_load_report()),
                    encoding="utf-8",
                )
                self.assertEqual(command[0], str(binary.resolve()))
                self.assertEqual(command[1:], MODULE.TEST_ARGUMENTS)
                return SimpleNamespace(
                    returncode=1,
                    stdout="failed stdout\n",
                    stderr="failed stderr\n",
                )

            with mock.patch.object(
                MODULE.subprocess,
                "run",
                side_effect=failed_process,
            ):
                result = MODULE.run_one(
                    binary=binary,
                    run_directory=run_directory,
                    repository_root=root,
                    topology="udp",
                    measurement_seconds=None,
                    expected_binary_sha256=MODULE.sha256_file(binary),
                    timeout_seconds=300,
                )

            self.assertFalse(result["passed"])
            self.assertEqual(
                (run_directory / "stdout.log").read_text(encoding="utf-8"),
                "failed stdout\n",
            )
            self.assertEqual(
                (run_directory / "stderr.log").read_text(encoding="utf-8"),
                "failed stderr\n",
            )
            self.assertTrue((run_directory / "report.json").is_file())
            execution = json.loads(
                (run_directory / "execution.json").read_text(encoding="utf-8")
            )
            self.assertEqual(execution["schema_version"], MODULE.RUNNER_SCHEMA)
            self.assertEqual(
                execution["kind"], "clonk-network-load-benchmark-execution"
            )
            self.assertEqual(execution["return_code"], 1)
            self.assertTrue(execution["report_present"])
            self.assertEqual(
                execution["report_sha256"],
                MODULE.sha256_file(run_directory / "report.json"),
            )

    def test_cohort_runs_every_process_and_summarizes_only_successes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "integration-test"
            binary.write_bytes(b"exact binary")
            output = root / "candidate"
            return_codes = iter([1, 0])

            def process(command, **kwargs):
                report_path = Path(kwargs["env"]["LC_NETWORK_LOAD_METRICS"])
                report_path.write_text(
                    json.dumps(network_load_report()),
                    encoding="utf-8",
                )
                return SimpleNamespace(
                    returncode=next(return_codes),
                    stdout="stdout",
                    stderr="stderr",
                )

            with mock.patch.object(
                MODULE.subprocess,
                "run",
                side_effect=process,
            ) as execute:
                summary = MODULE.run_cohort(
                    binary=binary,
                    output_directory=output,
                    repository_root=root,
                    label="candidate",
                    runs=2,
                    topology="udp",
                    measurement_seconds=None,
                    timeout_seconds=300,
                    build={
                        "mode": "provided",
                        "cargo_profile": "test",
                        "provenance": {
                            "source": {"commit": "abc"},
                            "content": {"head": "content"},
                            "build": {"cargo_profile": "test"},
                            "inputs": {
                                "effective_profile": {
                                    "cargo_artifact_profile": {
                                        "debug_assertions": False,
                                    }
                                }
                            },
                        },
                    },
                )

            self.assertEqual(execute.call_count, 2)
            self.assertEqual(summary["result"], "fail")
            self.assertEqual(summary["successful_runs"], 1)
            self.assertEqual(summary["failed_runs"], 1)
            self.assertEqual(
                summary["metrics"]["control_completion_wait"][
                    "independent_run_count"
                ],
                1,
            )
            self.assertTrue((output / "run-001" / "report.json").is_file())
            self.assertTrue((output / "run-002" / "report.json").is_file())
            self.assertTrue((output / "benchmark-summary.json").is_file())
            self.assertEqual(
                summary["runs"][1]["report_sha256"],
                MODULE.sha256_file(output / "run-002" / "report.json"),
            )
            self.assertEqual(
                summary["runs"][1]["execution_sha256"],
                MODULE.sha256_file(output / "run-002" / "execution.json"),
            )
            metadata = json.loads(
                (output / "cohort-metadata.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                summary["runtime_machine"], metadata["runtime_machine"]
            )
            self.assertIn(
                "128-fragment",
                " ".join(metadata["methodology_notes"]),
            )
            self.assertIn(
                "isolated RTT warms 128",
                " ".join(metadata["methodology_notes"]),
            )

    def test_direct_binary_comparison_runs_counterbalanced_cohorts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline_binary = root / "baseline-bin"
            candidate_binary = root / "candidate-bin"
            baseline_binary.write_bytes(b"baseline")
            candidate_binary.write_bytes(b"candidate")
            write_prebuilt_provenance(baseline_binary, cargo_profile="test")
            candidate_provenance = write_prebuilt_provenance(
                candidate_binary, cargo_profile="test"
            )
            candidate_provenance["source"]["commit"] = "c" * 40
            MODULE.write_json(
                MODULE.provenance_sidecar_path(candidate_binary),
                candidate_provenance,
            )
            observed = []

            def run_one(**kwargs):
                label = kwargs["run_directory"].parent.name
                observed.append(label)
                kwargs["run_directory"].mkdir(parents=True)
                report = network_load_report(
                    source_commit=("a" if label == "baseline" else "c") * 40
                )
                if label == "candidate":
                    set_control_completion_wait(report, [5, 10, 15])
                report_path = kwargs["run_directory"] / "report.json"
                report_path.write_text(json.dumps(report), encoding="utf-8")
                execution = {
                    "schema_version": MODULE.RUNNER_SCHEMA,
                    "kind": "clonk-network-load-benchmark-execution",
                    "return_code": 0,
                    "timed_out": False,
                    "failure": None,
                    "report_present": True,
                    "report_sha256": MODULE.sha256_file(report_path),
                    "binary_sha256": kwargs["expected_binary_sha256"],
                    "binary_sha256_before": kwargs[
                        "expected_binary_sha256"
                    ],
                    "binary_sha256_after": kwargs[
                        "expected_binary_sha256"
                    ],
                }
                experiment = kwargs["experiment"]
                step = kwargs["experiment_step"]
                execution.update(
                    {
                        "experiment_id": experiment["experiment_id"],
                        "experiment_manifest_sha256": experiment[
                            "manifest_sha256"
                        ],
                        "pair_index": step["pair_index"],
                        "pair_order": step["order"],
                        "pair_position": step["position"],
                        "global_sequence": step["global_sequence"],
                    }
                )
                MODULE.write_json(
                    kwargs["run_directory"] / "execution.json", execution
                )
                return {
                    "passed": True,
                    "report_path": str(report_path),
                    "execution": execution,
                }

            with mock.patch.object(MODULE, "run_one", side_effect=run_one):
                comparison = MODULE.run_paired_binaries(
                    baseline_binary=baseline_binary,
                    candidate_binary=candidate_binary,
                    output_directory=root / "comparison",
                    repository_root=root,
                    runs=2,
                    topology="udp",
                    measurement_seconds=None,
                    timeout_seconds=300,
                    cargo_profile="test",
                )

            self.assertEqual(
                comparison["metrics"]["control_completion_wait"][
                    "candidate_to_baseline_ratio"
                ],
                0.5,
            )
            self.assertEqual(
                comparison["comparison_design"],
                "verified-direct-interleaved-paired",
            )
            experiment_path = root / "comparison" / "experiment-manifest.json"
            self.assertTrue(experiment_path.is_file())
            experiment = json.loads(experiment_path.read_text(encoding="utf-8"))
            self.assertEqual(
                comparison["experiment"]["experiment_id"],
                experiment["experiment_id"],
            )
            self.assertEqual(
                comparison["experiment"]["manifest_sha256"],
                MODULE.sha256_file(experiment_path),
            )
            self.assertEqual(
                observed,
                [step["label"] for step in experiment["schedule"]],
            )
            self.assertEqual(experiment["predeclared_pair_count"], 2)
            self.assertEqual(len(experiment["schedule"]), 4)
            self.assertTrue((root / "comparison" / "comparison.json").is_file())


if __name__ == "__main__":
    unittest.main()
