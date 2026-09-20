#!/usr/bin/env python3
"""Qualify device destruction and recovery through the shipped window loop."""

import argparse
import datetime
import json
import os
import platform
from pathlib import Path
import shutil
import subprocess

import run_software_presentation_smoke as presentation

REPOSITORY = Path(__file__).resolve().parent.parent


def prepare_fixture(artifacts: Path) -> Path:
    # A fresh user directory opens the new-player name editor, whose caret
    # blinks. Use the real packed profile fixture to keep the reference static.
    players = artifacts / "players"
    players.mkdir(parents=True, exist_ok=True)
    player = players / "Probe.c4p"
    shutil.copyfile(REPOSITORY / "crates/clonk-engine/tests/fixtures/embedded_player.c4p", player)
    config = artifacts / "Clonk.ini"
    config.write_text(
        f"[General]\nPlayerPath={players}\nParticipants={player}\n\n{presentation.SMOKE_CONFIG}",
        encoding="utf-8",
    )
    return config


def source_identity() -> dict:
    identity = presentation.source_identity()
    inputs = ["scripts/run_device_loss_probe.py", ".github/workflows/device-loss-qualification.yml"]
    identity["probe_inputs"] = {
        name: presentation.file_digest(REPOSITORY / name) for name in inputs
        if (REPOSITORY / name).is_file()
    }
    for arguments in (
        ["diff", "--name-only", "HEAD"],
        ["ls-files", "--others", "--exclude-standard"],
    ):
        if subprocess.check_output(["git", *arguments, "--", *inputs], cwd=REPOSITORY).strip():
            identity["source_dirty"] = True
    return identity


def check_report(path: Path, backend: str) -> dict:
    report = json.loads(path.read_text(encoding="utf-8"))
    if report.get("schema_version") != 2:
        raise SystemExit("device-loss resource evidence requires report schema 2")
    failures = []
    counts = report.get("surface_counts_at_drop")
    if (not isinstance(counts, list) or len(counts) != 2
            or counts[0] < 1 or counts[1] != counts[0] - 1):
        failures.append("the old surface was not released before replacement")
    if report.get("kind") != "clonk_device_loss_probe" or report.get("success") is not True:
        failures.append(f"probe did not recover: {report.get('failure')}")
    if (report.get("adapter") or {}).get("backend") != backend:
        failures.append("the requested GPU backend was not used")
    before = report.get("generation_before") or 0
    after = report.get("generation_after") or 0
    if before < 1 or after <= before:
        failures.append("the renderer generation did not advance")
    if (report.get("presented_before_loss") or 0) < 1 or (report.get("presented_after_recovery") or 0) < 3:
        failures.append("missing real presentations before or after the loss")
    if report.get("rebuild_ok") is not True or "Destroyed" not in (report.get("callback_diagnosis") or ""):
        failures.append("no device-destroyed callback followed by successful recreation")
    resources = report.get("resource_recovery") or {}
    textures = resources.get("textures_before") or 0
    ids_before = resources.get("texture_ids_before") or []
    ids_after = resources.get("texture_ids_after") or []
    if (len(ids_before) != textures or len(set(ids_before)) != textures
            or ids_before != ids_after):
        failures.append("the recovered scene's source texture identities do not match")
    if (textures < 1 or resources.get("textures_after") != textures
            or resources.get("recreated_textures") != textures
            or (resources.get("full_upload_calls") or 0) < textures
            or (resources.get("full_upload_bytes") or 0) < 1
            or resources.get("pixels_identical") is not True):
        failures.append("incomplete resource recreation or pixel evidence")
    from PIL import Image

    try:
        with Image.open(path.with_suffix(".before.png")) as original:
            original.load()
            before_pixels = original.convert("RGBA")
        with Image.open(path.with_suffix(".after.png")) as recovered:
            recovered.load()
            after_pixels = recovered.convert("RGBA")
        if (list(before_pixels.size) != resources.get("extent")
                or before_pixels.size != after_pixels.size
                or before_pixels.tobytes() != after_pixels.tobytes()):
            failures.append("the recovered pixels differ from the presented reference")
        if all(low == high for low, high in before_pixels.getextrema()):
            failures.append("the reference is a flat frame, not restored textured content")
    except (OSError, ValueError) as error:
        failures.append(f"cannot decode presented pixel evidence: {error}")
    if failures:
        raise SystemExit("device-loss qualification failed:\n  " + "\n  ".join(failures))
    return report


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", required=True, choices=("vulkan", "gl", "dx12", "metal"))
    parser.add_argument("--artifact-dir", type=Path, required=True)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--no-xvfb", action="store_true")
    arguments = parser.parse_args(argv)
    presentation.refuse_to_run_as_root()
    artifacts = arguments.artifact_dir.resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    report_path = artifacts / "report.json"
    for name in ("report.json", "report.before.png", "report.after.png", "qualification.json"):
        (artifacts / name).unlink(missing_ok=True)
    identity = source_identity()
    if arguments.release and identity["source_dirty"]:
        raise SystemExit("release qualification requires committed source inputs")
    binary = presentation.build_binary(arguments.release)
    binary_sha256 = presentation.file_digest(binary)
    config = prepare_fixture(artifacts)
    environment = {
        key: value for key, value in os.environ.items()
        if not key.startswith(("LC_", "WGPU_"))
    }
    environment.update({
        "WGPU_BACKEND": arguments.backend,
        "LC_INSTALL_ROOT": str(REPOSITORY),
        "LC_CONTENT_DIR": str(REPOSITORY / "content"),
        "LC_USER_DATA_DIR": str(artifacts / "user-data"),
        "LC_CACHE_DIR": str(artifacts / "cache"),
        "LC_LOGS_DIR": str(artifacts / "logs"),
        "LC_TEMP_DIR": str(artifacts / "temp"),
        "LC_GAME_UPDATE_RECOVERY_COMPLETE": "1",
    })
    command = [
        *presentation.launch_prefix(no_xvfb=arguments.no_xvfb), str(binary),
        "--config", str(config), "--device-loss-probe", str(report_path),
    ]
    with (artifacts / "run.log").open("w", encoding="utf-8") as output:
        try:
            completed = subprocess.run(
                command, cwd=REPOSITORY, env=environment, stdout=output,
                stderr=subprocess.STDOUT, timeout=60, check=False,
            )
        except subprocess.TimeoutExpired as error:
            raise SystemExit(f"device-loss probe timed out; see {artifacts / 'run.log'}") from error
    if completed.returncode:
        raise SystemExit(f"device-loss probe exited {completed.returncode}; see {artifacts / 'run.log'}")
    report = check_report(report_path, arguments.backend)
    if identity != source_identity() or presentation.file_digest(binary) != binary_sha256:
        raise SystemExit("source or executable changed during qualification")
    qualification = {
        "schema_version": 1, "kind": "clonk_device_loss_qualification",
        "recorded_at": datetime.datetime.now(datetime.UTC).isoformat(),
        **identity, "os": platform.platform(), "os_version": platform.version(),
        "architecture": platform.machine(), "backend": arguments.backend,
        "adapter": report["adapter"],
        "build_profile": "release" if arguments.release else "debug",
        "build_target": os.environ.get("CARGO_BUILD_TARGET"),
        "rustflags": os.environ.get("RUSTFLAGS"),
        "encoded_rustflags": os.environ.get("CARGO_ENCODED_RUSTFLAGS"),
        "rustc": subprocess.check_output(["rustc", "-vV"], cwd=REPOSITORY, text=True),
        "binary_sha256": binary_sha256,
        "artifacts": {name: presentation.file_digest(artifacts / name) for name in (
            "report.json", "report.before.png", "report.after.png", "run.log", "players/Probe.c4p",
        )},
    }
    (artifacts / "qualification.json").write_text(
        json.dumps(qualification, indent=2) + "\n", encoding="utf-8",
    )
    print(f"device-loss recovery qualified on {arguments.backend}: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
