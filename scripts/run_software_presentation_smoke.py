#!/usr/bin/env python3
"""Drive the software-presentation smoke probe and check what it reported.

`run_headed_surface_teardown_smoke.py` validates GPU adapter and driver
teardown and quotes `adapter_info()` in its evidence, so it cannot speak for a
presentation path that has no adapter to report. This is the runner for that
path: it opens a real window, presents through the wgpu-free presenter, resizes
the drawable, presents again, and fails unless every phase happened.

The interesting environment is one with no usable GPU at all, which is what the
software presenter exists for. On a headless machine this runner supplies that
itself by launching under `xvfb-run`, so an X11 session with no GPU behind it
can be exercised anywhere Xvfb is installed.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent

REPORT_KIND = "clonk_software_present_smoke"
SCHEMA_VERSION = 1

#: The probe reads this and refuses to run without it, so that a machine with a
#: working adapter cannot quietly qualify the GPU presenter instead.
FORCE_SOFTWARE_ENVIRONMENT = "LC_SOFTWARE_PRESENTATION"


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--artifact-dir",
        type=Path,
        help="where to write the report (default: target/software-present-smoke)",
    )
    parser.add_argument(
        "--release",
        action="store_true",
        help="build and run the release binary instead of the debug one",
    )
    parser.add_argument(
        "--no-xvfb",
        action="store_true",
        help="never wrap the probe in xvfb-run, even without a display",
    )
    return parser.parse_args(argv)


def refuse_to_run_as_root() -> None:
    """`clonk-app` refuses to run as root; say so before spending a build."""
    if hasattr(os, "geteuid") and os.geteuid() == 0:
        raise SystemExit(
            "clonk-app refuses to run as root, so this probe cannot either. "
            "Run it as an ordinary user."
        )


def build_binary(release: bool) -> Path:
    profile = ["--release"] if release else []
    subprocess.run(
        ["cargo", "build", "--locked", "-p", "clonk-app", "--bin", "clonk-app", *profile],
        cwd=REPOSITORY,
        check=True,
    )
    target = os.environ.get("CARGO_TARGET_DIR")
    root = Path(target) if target else REPOSITORY / "target"
    binary = root / ("release" if release else "debug") / "clonk-app"
    if not binary.is_file():
        raise SystemExit(f"cargo reported success but {binary} is not there")
    return binary


def has_a_display() -> bool:
    return bool(os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"))


def launch_prefix(*, no_xvfb: bool) -> list[str]:
    """`xvfb-run` when there is no display and it is available.

    A machine with a session of its own uses it: the probe should exercise the
    real compositor where there is one. Xvfb is the fallback that makes a
    headless runner able to qualify this path at all.
    """
    # macOS and Windows present through their own window servers and set
    # neither variable, so the absence of one says nothing there.
    if no_xvfb or sys.platform != "linux" or has_a_display():
        return []
    if shutil.which("xvfb-run") is None:
        raise SystemExit(
            "no display and no xvfb-run; install Xvfb (and xauth) or run from a session"
        )
    return ["xvfb-run", "-a", "--server-args=-screen 0 1024x768x24"]


def check_report(report_path: Path) -> dict:
    if not report_path.is_file():
        raise SystemExit(f"the probe wrote no report to {report_path}")
    report = json.loads(report_path.read_text(encoding="utf-8"))

    if report.get("kind") != REPORT_KIND:
        raise SystemExit(f"{report_path} is not a {REPORT_KIND} report")
    if report.get("schema_version") != SCHEMA_VERSION:
        raise SystemExit(
            f"report schema {report.get('schema_version')} is not the expected {SCHEMA_VERSION}"
        )

    failures = []
    if not report.get("presented_before_resize"):
        failures.append("no frame reached the window before the resize")
    if not report.get("presented_after_resize"):
        failures.append("no frame reached the window after the resize")
    if report.get("initial_extent") == report.get("resized_extent"):
        failures.append(
            "the drawable never changed size, so the resize proved nothing "
            f"({report.get('initial_extent')})"
        )
    if not report.get("registry_empty_at_exit"):
        failures.append("a window outlived the event loop")
    if not report.get("success"):
        failures.append(f"the probe reported failure: {report.get('failure')}")
    if failures:
        raise SystemExit("software presentation smoke failed:\n  " + "\n  ".join(failures))
    return report


def main(argv: list[str] | None = None) -> int:
    arguments = parse_arguments(argv)
    refuse_to_run_as_root()

    artifacts = arguments.artifact_dir or (REPOSITORY / "target" / "software-present-smoke")
    artifacts.mkdir(parents=True, exist_ok=True)
    report_path = artifacts / "report.json"
    # The probe refuses to overwrite an existing report, so a stale one from a
    # previous run would fail before it started.
    report_path.unlink(missing_ok=True)

    binary = build_binary(arguments.release)
    environment = dict(os.environ)
    environment[FORCE_SOFTWARE_ENVIRONMENT] = "1"

    command = [
        *launch_prefix(no_xvfb=arguments.no_xvfb),
        str(binary),
        "--software-present-smoke",
        str(report_path),
    ]
    completed = subprocess.run(command, cwd=REPOSITORY, env=environment, check=False)

    report = check_report(report_path)
    if completed.returncode != 0:
        raise SystemExit(
            f"the probe reported success but exited {completed.returncode}; "
            "treat the exit code as authoritative"
        )

    print(
        "software presentation smoke passed: presented at "
        f"{report['initial_extent']}, resized to {report['resized_extent']}, "
        "and left no window behind"
    )
    print(f"report: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
