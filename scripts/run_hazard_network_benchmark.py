#!/usr/bin/env python3
"""Run the complete competitive Hazard network benchmark sequentially.

Each map gets an isolated Harpoon-style host/client fleet.  Running one map at
a time keeps the requested four-player topology and port plan unambiguous, and
lets a failed map retain its evidence without preventing the remaining maps
from being measured.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any, NamedTuple, Sequence


class HazardFailure(RuntimeError):
    """An expected Hazard matrix setup or orchestration failure."""


class HazardScenario(NamedTuple):
    relative_path: Path
    title: str

    @property
    def slug(self) -> str:
        return self.relative_path.stem.lower()


# Scenario.txt titles were read from the checked-in Hazard pack.  This is an
# explicit compatibility inventory: a new, renamed, or removed map must be
# reviewed before it silently changes this long-running performance series.
HAZARD_COMPETITIVE_SCENARIOS = (
    HazardScenario(Path("AH_Predator.c4s"), "AH - Predator"),
    HazardScenario(Path("AS_Research.c4s"), "AS - Skynet"),
    HazardScenario(Path("BR_Docks.c4s"), "BR - Docks"),
    HazardScenario(Path("BR_Industrial.c4s"), "BR - Industrial"),
    HazardScenario(Path("CTF_DeepSea.c4s"), "AH/CTF - Gamma Breschnew"),
    HazardScenario(Path("CTF_Face.c4s"), "CTF - Face"),
    HazardScenario(Path("CTF_Space.c4s"), "CTF - Centauri S-003"),
    HazardScenario(Path("DEST_EpicVoltage.c4s"), "DEST - Epic Voltage"),
    HazardScenario(Path("DM_Baldoon.c4s"), "DM - Baldoon"),
    HazardScenario(Path("DM_Factory.c4s"), "DM - Forsak IV"),
    HazardScenario(Path("DM_KillingDay.c4s"), "DM - Killing day"),
    HazardScenario(Path("DOM_TransmitTower.c4s"), "DOM - Zetha Base"),
)
HAZARD_TOTAL_MEASUREMENT_SECONDS = 5 * 60
HAZARD_SCENARIO_MEASUREMENT_SECONDS = (
    HAZARD_TOTAL_MEASUREMENT_SECONDS // len(HAZARD_COMPETITIVE_SCENARIOS)
)
_EXPECTED_HAZARD_DIRECTORY_NAMES = frozenset(
    scenario.relative_path.name for scenario in HAZARD_COMPETITIVE_SCENARIOS
) | {"Tutorial.c4s"}
# One four-player fleet consumes offsets 0..17. Keep a full 64-port block so
# diagnostic reruns with the generic runner's 24-player topology (0..57) also
# remain disjoint while prior sockets leave TIME_WAIT.
SCENARIO_PORT_STRIDE = 64


def validate_hazard_inventory(hazard_root: Path) -> None:
    """Require the known twelve competitive scenarios plus Tutorial exactly."""

    if not hazard_root.is_dir():
        raise HazardFailure(f"Hazard content directory not found: {hazard_root}")
    observed = frozenset(
        child.name for child in hazard_root.iterdir() if child.is_dir() and child.suffix == ".c4s"
    )
    if observed != _EXPECTED_HAZARD_DIRECTORY_NAMES:
        missing = sorted(_EXPECTED_HAZARD_DIRECTORY_NAMES - observed)
        unexpected = sorted(observed - _EXPECTED_HAZARD_DIRECTORY_NAMES)
        raise HazardFailure(
            "Hazard scenario inventory drift: "
            f"missing={missing} unexpected={unexpected}"
        )
    missing_titles = [
        str(hazard_root / scenario.relative_path / "Scenario.txt")
        for scenario in HAZARD_COMPETITIVE_SCENARIOS
        if not (hazard_root / scenario.relative_path / "Scenario.txt").is_file()
    ]
    if missing_titles:
        raise HazardFailure(
            "Hazard scenario inventory drift: missing Scenario.txt: "
            + ", ".join(missing_titles)
        )


def default_artifact_dir(workspace: Path) -> Path:
    run_id = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    return workspace / "target" / "network-benchmark" / f"hazard-{run_id}"


def harpoon_command_for_scenario(
    arguments: argparse.Namespace,
    *,
    workspace: Path,
    scenario: HazardScenario,
    artifact_dir: Path,
    base_port: int | None = None,
) -> list[str]:
    """Build one generic fleet invocation while retaining every non-crew gate."""

    return [
        sys.executable,
        str(workspace / "scripts" / "run_harpoonrace_24_player_benchmark.py"),
        "--binary",
        arguments.binary,
        "--scenario",
        str(workspace / "content" / "Hazard.c4f" / scenario.relative_path),
        "--scenario-title",
        scenario.title,
        "--artifact-dir",
        str(artifact_dir),
        "--players",
        str(arguments.players),
        "--measurement-seconds",
        str(arguments.measurement_seconds),
        "--base-port",
        str(arguments.base_port if base_port is None else base_port),
        "--minimum-simulation-fps",
        str(arguments.minimum_simulation_fps),
        "--minimum-presentation-fps",
        str(arguments.minimum_presentation_fps),
        "--maximum-graphics-p99-ms",
        str(arguments.maximum_graphics_p99_ms),
        "--maximum-network-lag-ms",
        str(arguments.maximum_network_lag_ms),
        "--input-probe-interval-ms",
        str(arguments.input_probe_interval_ms),
        "--maximum-input-latency-ms",
        str(arguments.maximum_input_latency_ms),
        "--minimum-input-success-percent",
        str(arguments.minimum_input_success_percent),
        "--skip-sf5b-crew-assertion",
        "--runtime-only",
    ]


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def child_matrix_invariant_fingerprint(artifact_dir: Path) -> str:
    manifest_path = artifact_dir / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        fingerprint = manifest["input_fingerprint"][
            "matrix_invariant_sha256"
        ]
    except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise HazardFailure(
            f"child manifest fingerprint unavailable at {manifest_path}: {error}"
        ) from error
    if not isinstance(fingerprint, str) or not fingerprint:
        raise HazardFailure(
            f"child manifest fingerprint is invalid at {manifest_path}"
        )
    return fingerprint


def run_hazard_matrix(
    arguments: argparse.Namespace,
    *,
    workspace: Path,
    artifact_dir: Path,
) -> dict[str, Any]:
    validate_hazard_inventory(workspace / "content" / "Hazard.c4f")
    artifact_dir.mkdir(parents=True, exist_ok=False)
    results: list[dict[str, Any]] = []
    fingerprint_errors: list[str] = []
    matrix_invariant_sha256: str | None = None
    for sequence, scenario in enumerate(HAZARD_COMPETITIVE_SCENARIOS, start=1):
        scenario_artifact_dir = artifact_dir / f"{sequence:02d}-{scenario.slug}"
        command = harpoon_command_for_scenario(
            arguments,
            workspace=workspace,
            scenario=scenario,
            artifact_dir=scenario_artifact_dir,
            base_port=arguments.base_port + (sequence - 1) * SCENARIO_PORT_STRIDE,
        )
        print(
            f"[hazard-benchmark] start {sequence}/{len(HAZARD_COMPETITIVE_SCENARIOS)} "
            f"scenario={scenario.relative_path}"
        )
        completed = subprocess.run(command, check=False)
        child_fingerprint: str | None = None
        try:
            child_fingerprint = child_matrix_invariant_fingerprint(
                scenario_artifact_dir
            )
        except HazardFailure as error:
            fingerprint_errors.append(
                f"{scenario.relative_path}: {error}"
            )
        else:
            if matrix_invariant_sha256 is None:
                matrix_invariant_sha256 = child_fingerprint
            elif child_fingerprint != matrix_invariant_sha256:
                fingerprint_errors.append(
                    f"{scenario.relative_path}: child matrix fingerprint differs "
                    f"({child_fingerprint} != {matrix_invariant_sha256})"
                )
        results.append(
            {
                "sequence": sequence,
                "scenario": str(scenario.relative_path),
                "title": scenario.title,
                "artifact_dir": str(scenario_artifact_dir),
                "return_code": completed.returncode,
                "matrix_invariant_sha256": child_fingerprint,
            }
        )
        print(
            f"[hazard-benchmark] complete {sequence}/{len(HAZARD_COMPETITIVE_SCENARIOS)} "
            f"scenario={scenario.relative_path} return_code={completed.returncode}"
        )
    summary = {
        "schema_version": 1,
        "result": (
            "pass"
            if all(item["return_code"] == 0 for item in results)
            and not fingerprint_errors
            else "fail"
        ),
        "matrix_invariant_sha256": matrix_invariant_sha256,
        "fingerprint_errors": fingerprint_errors,
        "scenarios": results,
    }
    write_json(artifact_dir / "summary.json", summary)
    return summary


def build_argument_parser() -> argparse.ArgumentParser:
    workspace = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(
        description="Run all twelve competitive Hazard maps sequentially."
    )
    parser.add_argument(
        "--binary",
        default=os.environ.get("LC_APP_BINARY", str(workspace / "target/release/clonk-app")),
    )
    parser.add_argument("--artifact-dir", default=str(default_artifact_dir(workspace)))
    parser.add_argument("--players", type=int, default=4)
    parser.add_argument(
        "--measurement-seconds",
        type=int,
        default=HAZARD_SCENARIO_MEASUREMENT_SECONDS,
        help=(
            "measured seconds per map; the default gives all twelve maps "
            "five measured minutes in total"
        ),
    )
    parser.add_argument("--base-port", type=int, default=31_111)
    parser.add_argument("--minimum-simulation-fps", type=float, default=38.0)
    parser.add_argument("--minimum-presentation-fps", type=float, default=35.0)
    parser.add_argument("--maximum-graphics-p99-ms", type=float, default=25.0)
    parser.add_argument("--maximum-network-lag-ms", type=float, default=100.0)
    parser.add_argument("--input-probe-interval-ms", type=int, default=500)
    parser.add_argument("--maximum-input-latency-ms", type=float, default=100.0)
    parser.add_argument("--minimum-input-success-percent", type=float, default=95.0)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = build_argument_parser().parse_args(argv)
    workspace = Path(__file__).resolve().parents[1]
    artifact_dir = Path(arguments.artifact_dir).expanduser().resolve()
    try:
        summary = run_hazard_matrix(
            arguments, workspace=workspace, artifact_dir=artifact_dir
        )
    except (HazardFailure, OSError, subprocess.SubprocessError) as error:
        print(f"hazard network benchmark: {error}", file=sys.stderr)
        return 1
    print(f"[hazard-benchmark] result={summary['result']} artifacts={artifact_dir}")
    return 0 if summary["result"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
