import copy
import hashlib
import json
import unittest

from _repo import REPOSITORY


REPORT = REPOSITORY / "parity/reports/goldrush_seed_424242.json"
RNG_PATCH = REPOSITORY / "parity/reports/goldrush_seed_424242_rng_ledger.diff"
PINNED_ORACLE_COMMIT = "7d43b47b7d789b533f32d005e64596e0a07019cd"
HISTORICAL_RUST_COMMIT = "40579922c8f0eb7b16c846b1acab047663975b4c"
CONTENT_COMMIT = "67a54d0e662bda3aa0202134efc065d7bc420872"
CPP_SOURCE_TREE = "f0597bdde7201f58b5e28d7ed42071d489ddf6de"
RUST_SOURCE_TREE = "3bad3e5313662c38ed89811b77bfdca42beb3191"
PINNED_RUST_SOURCE_TREE = "e412c65ba3a8c6d7212a08cbf22c08947a0858df"
PLAYER_SHA256 = "bfcfe9ac6b7a5085a48dc03bd495e7c31ddf824506a2b317c34f3862b47f1694"
RECORD_SHA256 = "da584c47f35f14543ba1dfeec4a9f178a83da7904c865503790fa758f0a6241f"
RNG_PATCH_SHA256 = "0aced9aa61ae911814612f40f07c5e73579850b2479653c4bd72cfac0e5a6283"
PATCHED_BINARY_SHA256 = "442c8abd0593df3e193b1393ca529d92a81d2d52de286ebc6f12cd0002b53c3f"
MATCH_HORIZON = 15_000


def exact_keys(value, expected, label):
    assert type(value) is dict, f"{label} must be an object"
    observed = set(value)
    assert observed == expected, (
        f"{label} schema drift: missing={sorted(expected - observed)} "
        f"unexpected={sorted(observed - expected)}"
    )
    return value


def exact_integer(value, expected, label):
    assert type(value) is int and value == expected, (
        f"{label} must be the integer {expected}"
    )
    return value


def load_report(path=REPORT):
    def object_without_duplicate_keys(pairs):
        value = {}
        for key, item in pairs:
            assert key not in value, f"duplicate JSON key: {key}"
            value[key] = item
        return value

    return json.loads(
        path.read_text(encoding="utf-8"),
        object_pairs_hook=object_without_duplicate_keys,
    )


def validate_report(report):
    # Pinned C++ collects the six raw C4Fixed fields before every comparison and
    # disables the runtime on a false return (src/rust/RustEngineBridge.cpp:
    # 1141-1176,1974-2022,2127-2164 at 7d43b47).
    value = exact_keys(
        report,
        {
            "schema_version",
            "kind",
            "issue",
            "scope",
            "oracle",
            "rust_runtime",
            "instrumentation",
            "scenario",
            "comparison",
            "terminal_probe",
            "source_citations",
        },
        "report",
    )
    assert type(value["schema_version"]) is int and value["schema_version"] == 1
    assert value["kind"] == "historical_full_scenario_differential"
    assert value["issue"] == "clonk-org/clonk-rs#394"

    scope = exact_keys(
        value["scope"],
        {"historical", "validates_current_head"},
        "scope",
    )
    assert scope["historical"] is True
    assert scope["validates_current_head"] is False

    oracle = exact_keys(
        value["oracle"],
        {
            "pinned_commit",
            "execution_checkout_commit",
            "pinned_base_src_tree",
            "execution_base_src_tree",
            "pinned_rust_tree",
            "content_commit",
        },
        "oracle",
    )
    assert oracle["pinned_commit"] == PINNED_ORACLE_COMMIT
    assert oracle["execution_checkout_commit"] == HISTORICAL_RUST_COMMIT
    assert oracle["pinned_base_src_tree"] == CPP_SOURCE_TREE
    assert oracle["execution_base_src_tree"] == CPP_SOURCE_TREE
    assert oracle["pinned_rust_tree"] == PINNED_RUST_SOURCE_TREE
    assert oracle["pinned_rust_tree"] != RUST_SOURCE_TREE
    assert oracle["content_commit"] == CONTENT_COMMIT

    runtime = exact_keys(
        value["rust_runtime"],
        {"commit", "tree"},
        "rust_runtime",
    )
    assert runtime["commit"] == HISTORICAL_RUST_COMMIT
    assert runtime["tree"] == RUST_SOURCE_TREE

    instrumentation = exact_keys(
        value["instrumentation"],
        {"kind", "changes_simulation_logic", "patch", "patch_sha256", "files"},
        "instrumentation",
    )
    assert instrumentation["kind"] == "fail_closed_sync_comparison"
    assert instrumentation["changes_simulation_logic"] is False
    assert instrumentation["patch"] == RNG_PATCH.relative_to(REPOSITORY).as_posix()
    assert instrumentation["patch_sha256"] == RNG_PATCH_SHA256
    assert instrumentation["files"] == [
        "rust/crates/lc-engine/src/ffi.rs",
        "rust/include/lc_engine_ffi.h",
        "src/rust/RustEngineBridge.cpp",
    ]
    assert hashlib.sha256(RNG_PATCH.read_bytes()).hexdigest() == RNG_PATCH_SHA256
    patch = RNG_PATCH.read_text(encoding="utf-8")
    assert "index 6d168dc69..87b9d6e10 100644" in patch
    added_lines = [
        line[1:].strip() for line in patch.splitlines() if line.startswith("+")
    ]
    assert added_lines.count(
        "if rng.rnd3_ptr() != rng_random3 || rng.hold != rng_hold || "
        "rng.count != rng_count {"
    ) == 2
    assert added_lines.count("return false;") == 2
    assert added_lines.count("int32_t rng_random3,") == 1
    assert added_lines.count("rng_random3: i32,") == 1
    assert added_lines.count("FRndPtr3,") == 1

    scenario = exact_keys(
        value["scenario"],
        {
            "path",
            "random_seed",
            "map_seed",
            "startup_player_count",
            "use_fair_crew",
            "fair_crew_strength",
            "control_rate",
            "auto_frame_skip",
            "flags",
            "player_profile",
        },
        "scenario",
    )
    assert scenario["path"] == "Western.c4f/Goldrush.c4s"
    exact_integer(scenario["random_seed"], 424242, "random_seed")
    exact_integer(scenario["map_seed"], 38954, "map_seed")
    exact_integer(scenario["startup_player_count"], 1, "startup_player_count")
    assert scenario["use_fair_crew"] is True
    exact_integer(scenario["fair_crew_strength"], 1000, "fair_crew_strength")
    exact_integer(scenario["control_rate"], 2, "control_rate")
    assert scenario["auto_frame_skip"] is True
    assert scenario["flags"] == ["/nonetwork", "/console"]

    player = exact_keys(
        scenario["player_profile"],
        {
            "sha256",
            "size",
            "source_record",
            "source_record_sha256",
            "offset",
            "length",
        },
        "player_profile",
    )
    assert player["sha256"] == PLAYER_SHA256
    exact_integer(player["size"], 31_254, "player_profile.size")
    assert player["source_record"] == "Records.c4f/178-Goldrush.c4s/CtrlRec.c4b"
    assert player["source_record_sha256"] == RECORD_SHA256
    exact_integer(player["offset"], 28, "player_profile.offset")
    exact_integer(player["length"], player["size"], "player_profile.length")

    comparison = exact_keys(
        value["comparison"],
        {
            "status",
            "first_frame",
            "previous_horizon",
            "last_matching_frame",
            "comparator",
            "raw_fixed_fields",
            "synced_rng_fields",
            "random3_compared",
            "safe_random_compared",
            "presentation_randomness_compared",
            "synced_rng_fail_closed",
        },
        "comparison",
    )
    assert comparison["status"] == "match"
    exact_integer(comparison["first_frame"], 1, "first_frame")
    exact_integer(comparison["previous_horizon"], 14_415, "previous_horizon")
    exact_integer(
        comparison["last_matching_frame"], MATCH_HORIZON, "last_matching_frame"
    )
    assert comparison["last_matching_frame"] > comparison["previous_horizon"]
    assert comparison["comparator"] == "lc_engine_runtime_compare_snapshot"
    assert comparison["raw_fixed_fields"] == [
        "fix_x.val",
        "fix_y.val",
        "fix_r.val",
        "xdir.val",
        "ydir.val",
        "rdir.val",
    ]
    assert comparison["synced_rng_fields"] == [
        "FRndPtr3",
        "RandomHold",
        "RandomCount",
    ]
    assert comparison["random3_compared"] is True
    assert comparison["safe_random_compared"] is False
    assert comparison["presentation_randomness_compared"] is False
    assert comparison["synced_rng_fail_closed"] is True

    probe = exact_keys(
        value["terminal_probe"],
        {
            "debugger",
            "architecture",
            "binary_sha256",
            "breakpoint_symbol",
            "frame_argument_register",
            "frame_argument",
            "return_register",
            "return_value",
            "parity_mismatch_log_entries",
        },
        "terminal_probe",
    )
    assert probe["debugger"] == "lldb"
    assert probe["architecture"] == "arm64"
    assert probe["binary_sha256"] == PATCHED_BINARY_SHA256
    assert probe["breakpoint_symbol"] == comparison["comparator"]
    assert probe["frame_argument_register"] == "x1"
    exact_integer(
        probe["frame_argument"],
        comparison["last_matching_frame"],
        "terminal_probe.frame_argument",
    )
    assert probe["return_register"] == "w0"
    exact_integer(probe["return_value"], 1, "terminal_probe.return_value")
    exact_integer(
        probe["parity_mismatch_log_entries"],
        0,
        "terminal_probe.parity_mismatch_log_entries",
    )

    citations = value["source_citations"]
    assert citations == [
        "src/C4GameParameters.cpp:424-427",
        "src/C4Landscape.cpp:561-580",
        "src/C4Game.cpp:796-870,1899-1903",
        "src/C4Random.cpp:22-41",
        "src/C4Control.cpp:445-451,482-492",
        "src/rust/RustEngineBridge.cpp:1141-1176,1974-2022,2127-2164",
        "rust/crates/lc-engine/src/ffi.rs:1397-1754,2361-2507",
        "rust/crates/lc-engine/src/lib.rs:20773-20806",
    ]


class GoldrushDifferentialReportTests(unittest.TestCase):
    def test_report_advances_the_raw_fixed_and_synced_rng_horizon(self):
        validate_report(load_report())

    def test_report_schema_fails_closed_on_weakened_evidence(self):
        report = load_report()
        mutations = {
            "wrong oracle pin": lambda value: value["oracle"].update(
                pinned_commit="0" * 40
            ),
            "simulation patch overclaim": lambda value: value["instrumentation"].update(
                changes_simulation_logic=True
            ),
            "wrong seed": lambda value: value["scenario"].update(random_seed=0),
            "wrong player": lambda value: value["scenario"]["player_profile"].update(
                sha256="0" * 64
            ),
            "old horizon": lambda value: value["comparison"].update(
                last_matching_frame=14_415
            ),
            "inflated horizon and probe": lambda value: (
                value["comparison"].update(last_matching_frame=50_000),
                value["terminal_probe"].update(frame_argument=50_000),
            ),
            "integer horizon replaced by boolean": lambda value: value[
                "comparison"
            ].update(last_matching_frame=True),
            "boolean startup count": lambda value: value["scenario"].update(
                startup_player_count=True
            ),
            "wrong first frame": lambda value: value["comparison"].update(
                first_frame=0
            ),
            "raw field omitted": lambda value: value["comparison"][
                "raw_fixed_fields"
            ].pop(),
            "RNG made log-only": lambda value: value["comparison"].update(
                synced_rng_fail_closed=False
            ),
            "terminal return failed": lambda value: value["terminal_probe"].update(
                return_value=0
            ),
            "boolean terminal return": lambda value: value["terminal_probe"].update(
                return_value=True
            ),
            "boolean mismatch count": lambda value: value["terminal_probe"].update(
                parity_mismatch_log_entries=False
            ),
            "current HEAD overclaim": lambda value: value["scope"].update(
                validates_current_head=True
            ),
            "unexpected field": lambda value: value.update(unreviewed=True),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                changed = copy.deepcopy(report)
                mutate(changed)
                with self.assertRaises((AssertionError, ValueError)):
                    validate_report(changed)

    def test_duplicate_json_keys_fail_closed(self):
        with self.assertRaisesRegex(AssertionError, "duplicate JSON key"):
            json.loads(
                '{"schema_version": 1, "schema_version": 1}',
                object_pairs_hook=lambda pairs: self._reject_duplicate_keys(pairs),
            )

    @staticmethod
    def _reject_duplicate_keys(pairs):
        value = {}
        for key, item in pairs:
            assert key not in value, f"duplicate JSON key: {key}"
            value[key] = item
        return value


if __name__ == "__main__":
    unittest.main()
