#!/usr/bin/env python3
"""Qualify the real-window GPU teardown path on headed driver hardware.

The default mode is deliberately narrow: only a Linux Wayland session using a
discrete NVIDIA adapter through the proprietary Vulkan driver can qualify the
crashes reported in clonk-org/clonk-rs#53 and clonk-org/clonk-rs#54.
``--wiring-only`` still drives the shipped event loop and real surfaces on
another platform, but never claims that it reproduces the affected driver
stack.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import platform
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Mapping, Sequence


WORKSPACE = Path(__file__).resolve().parents[1]
REPORT_SCHEMA = 1
REPORT_KIND = "clonk_headed_surface_smoke"
EVIDENCE_KIND = "clonk_headed_surface_smoke_qualification"
NVIDIA_VENDOR_ID = 0x10DE
MAX_REPORT_BYTES = 1_000_000
AUTHORITATIVE_BUILD_TIMEOUT_SECONDS = 900.0
CONTROLLED_ENVIRONMENT_KEYS = (
    "LC_APP_ROOT",
    "LC_CACHE_DIR",
    "LC_CONFIG_FILE",
    "LC_CONTENT_DIR",
    "LC_GAME_UPDATE_NOTICE",
    "LC_INSTALL_ROOT",
    "LC_LANGUAGE_OVERRIDE",
    "LC_LOG",
    "LC_LOGS_DIR",
    "LC_TEMP_DIR",
    "LC_USER_DATA_DIR",
    "RUST_LOG",
)
CONTROLLED_BUILD_ENVIRONMENT_KEYS = (
    "CARGO_BUILD_TARGET",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_INCREMENTAL",
    "RUSTC",
    "RUSTDOC",
    "RUSTUP_TOOLCHAIN",
    "CC",
    "CXX",
    "AR",
    "CFLAGS",
    "CXXFLAGS",
    "CPPFLAGS",
    "LDFLAGS",
    "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "PKG_CONFIG_PATH",
    "CARGO_TARGET_DIR",
)
CONTROLLED_BUILD_ENVIRONMENT_PREFIXES = (
    "CARGO_PROFILE_",
    "CARGO_BUILD_",
    "CARGO_TARGET_",
)
SMOKE_CONFIG = (
    "[Graphics]\n"
    "ResolutionX=800\n"
    "ResolutionY=600\n"
    "DisplayMode=1\n"
    "Maximized=false\n"
    "\n"
    "[Sound]\n"
    "Sound=false\n"
    "Music=false\n"
    "MenuMusic=false\n"
    "MenuSound=false\n"
)

TOP_LEVEL_KEYS = frozenset(
    {
        "schema_version",
        "kind",
        "success",
        "failure",
        "display_backend",
        "wayland_display",
        "xdg_session_type",
        "surface_windows",
        "instance_acquisitions",
        "retained_registry",
        "shell_adapter",
        "child_adapter",
        "shell_presented_before_close",
        "child_presented_before_close",
        "child_closed_while_shell_survived",
        "child_released_after_close",
        "shell_presented_after_child_close",
        "loop_exiting_release_order",
        "registry_empty_on_loop_exiting",
        "shell_released_on_loop_exiting",
    }
)
ADAPTER_KEYS = frozenset(
    {
        "name",
        "vendor_id",
        "device_id",
        "device_type",
        "pci_bus_id",
        "driver",
        "driver_info",
        "backend",
        "subgroup_min_size",
        "subgroup_max_size",
        "transient_saves_memory",
    }
)
KNOWN_DISPLAY_BACKENDS = frozenset(
    {
        "wayland",
        "x11",
        "appkit",
        "windows",
        "uikit",
        "orbital",
        "ohos",
        "drm",
        "gbm",
        "web",
        "android",
        "haiku",
    }
)


class SmokeFailure(RuntimeError):
    """A setup, execution, or evidence failure from the headed probe."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise SmokeFailure(message)


def _require_exact_keys(value: Any, expected: frozenset[str], label: str) -> dict[str, Any]:
    _require(isinstance(value, dict), f"{label} must be a JSON object")
    observed = frozenset(value)
    _require(
        observed == expected,
        f"{label} schema drift: missing={sorted(expected - observed)} "
        f"unexpected={sorted(observed - expected)}",
    )
    return value


def _require_nonempty_string(value: Any, label: str) -> str:
    _require(isinstance(value, str) and bool(value.strip()), f"{label} must be nonempty")
    return value


def _require_integer(value: Any, label: str, *, minimum: int = 0) -> int:
    _require(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    return value


def _validate_adapter(
    adapter: Any,
    label: str,
    expected_backend: str,
    *,
    authoritative: bool,
) -> dict[str, Any]:
    value = _require_exact_keys(adapter, ADAPTER_KEYS, label)
    _require_nonempty_string(value["name"], f"{label}.name")
    for key in ("pci_bus_id", "driver", "driver_info"):
        _require(isinstance(value[key], str), f"{label}.{key} must be a string")
    if authoritative:
        for key in ("driver", "driver_info"):
            text = _require_nonempty_string(value[key], f"{label}.{key}")
            _require(text not in {"?", "unknown"}, f"{label}.{key} is not useful evidence")
    _require_integer(value["vendor_id"], f"{label}.vendor_id")
    _require_integer(value["device_id"], f"{label}.device_id")
    _require_integer(value["subgroup_min_size"], f"{label}.subgroup_min_size")
    _require_integer(value["subgroup_max_size"], f"{label}.subgroup_max_size")
    _require(
        type(value["transient_saves_memory"]) is bool
        or value["transient_saves_memory"] is None,
        f"{label}.transient_saves_memory must be a boolean or null",
    )
    _require(
        value["backend"] == expected_backend,
        f"{label}.backend is {value['backend']!r}, expected {expected_backend!r}",
    )
    _require_nonempty_string(value["device_type"], f"{label}.device_type")
    return value


def validate_report(
    report: Any,
    *,
    authoritative: bool,
    expected_backend: str,
) -> None:
    """Fail closed unless the app proved the entire production lifecycle."""

    value = _require_exact_keys(report, TOP_LEVEL_KEYS, "headed surface report")
    _require(
        type(value["schema_version"]) is int
        and value["schema_version"] == REPORT_SCHEMA,
        "unsupported report schema",
    )
    _require(value["kind"] == REPORT_KIND, "unexpected report kind")
    _require(value["success"] is True, f"app probe failed: {value['failure']!r}")
    _require(value["failure"] is None, "successful report must not carry a failure")
    _require(
        value["display_backend"] in KNOWN_DISPLAY_BACKENDS,
        f"unknown display backend: {value['display_backend']!r}",
    )

    if authoritative:
        _require(value["display_backend"] == "wayland", "authoritative display must be Wayland")
        _require_nonempty_string(value["wayland_display"], "wayland_display")
        _require(
            isinstance(value["xdg_session_type"], str)
            and value["xdg_session_type"].lower() == "wayland",
            "authoritative XDG session must be Wayland",
        )
        _require(expected_backend == "vulkan", "authoritative backend must be Vulkan")

    windows = value["surface_windows"]
    _require(
        isinstance(windows, list) and len(windows) == 2,
        "exactly two real windows are required",
    )
    window_keys = frozenset({"role", "window_id", "instance_entry_id"})
    for index, role in enumerate(("shell", "viewport")):
        window = _require_exact_keys(windows[index], window_keys, f"surface_windows[{index}]")
        _require(window["role"] == role, f"surface_windows[{index}] must be {role}")
        _require_nonempty_string(window["window_id"], f"surface_windows[{index}].window_id")
        _require_integer(
            window["instance_entry_id"],
            f"surface_windows[{index}].instance_entry_id",
            minimum=1,
        )
    _require(
        windows[0]["window_id"] != windows[1]["window_id"],
        "the two surfaces must use distinct windows",
    )
    instance_id = windows[0]["instance_entry_id"]
    _require(
        windows[1]["instance_entry_id"] == instance_id,
        "the child and survivor must share one retained instance",
    )

    acquisitions = value["instance_acquisitions"]
    _require(
        isinstance(acquisitions, list) and len(acquisitions) == 2,
        "exactly two instance acquisitions are required",
    )
    acquisition_keys = frozenset({"sequence", "entry_id", "requested_backends", "created"})
    for index, acquisition in enumerate(acquisitions):
        event = _require_exact_keys(
            acquisition,
            acquisition_keys,
            f"instance_acquisitions[{index}]",
        )
        _require_integer(
            event["sequence"],
            f"instance_acquisitions[{index}].sequence",
            minimum=1,
        )
        _require_integer(
            event["entry_id"],
            f"instance_acquisitions[{index}].entry_id",
            minimum=1,
        )
        _require(
            type(event["created"]) is bool,
            f"instance_acquisitions[{index}].created must be a boolean",
        )
        _require(event["sequence"] == index + 1, "instance acquisition sequence is not contiguous")
        _require(
            event["entry_id"] == instance_id,
            "surface acquisition is not linked to the retained instance",
        )
        _require(
            event["requested_backends"] == [expected_backend],
            "surface acquisition did not request the forced backend only",
        )
        _require(
            event["created"] is (index == 0),
            "instance acquisitions must be one creation followed by one reuse",
        )

    retained = value["retained_registry"]
    _require(
        isinstance(retained, list) and len(retained) == 1,
        "exactly one retained registry entry is required",
    )
    retained_keys = frozenset(
        {"entry_id", "requested_backends", "acquisitions", "resident_at_loop_exit"}
    )
    entry = _require_exact_keys(retained[0], retained_keys, "retained_registry[0]")
    _require_integer(entry["entry_id"], "retained_registry[0].entry_id", minimum=1)
    _require_integer(entry["acquisitions"], "retained_registry[0].acquisitions", minimum=1)
    _require(entry["entry_id"] == instance_id, "retained registry entry does not own both surfaces")
    _require(entry["requested_backends"] == [expected_backend], "retained registry backend drifted")
    _require(entry["acquisitions"] == 2, "retained instance must have exactly two acquisitions")
    _require(
        entry["resident_at_loop_exit"] is True,
        "retained instance did not survive window teardown",
    )

    shell_adapter = _validate_adapter(
        value["shell_adapter"],
        "shell_adapter",
        expected_backend,
        authoritative=authoritative,
    )
    child_adapter = _validate_adapter(
        value["child_adapter"],
        "child_adapter",
        expected_backend,
        authoritative=authoritative,
    )
    _require(shell_adapter == child_adapter, "the two real surfaces used different adapters")
    if authoritative:
        _require(shell_adapter["vendor_id"] == NVIDIA_VENDOR_ID, "adapter is not NVIDIA")
        _require(shell_adapter["device_type"] == "discrete-gpu", "adapter is not a discrete GPU")
        _require(
            shell_adapter["driver"].strip().casefold() == "nvidia",
            "adapter does not use the proprietary NVIDIA driver required by the reported crashes",
        )

    for key in (
        "shell_presented_before_close",
        "child_presented_before_close",
        "child_closed_while_shell_survived",
        "child_released_after_close",
        "shell_presented_after_child_close",
        "registry_empty_on_loop_exiting",
        "shell_released_on_loop_exiting",
    ):
        _require(value[key] is True, f"lifecycle proof {key} is missing")
    release_order = value["loop_exiting_release_order"]
    _require(
        isinstance(release_order, list)
        and len(release_order) == 1
        and type(release_order[0]) is int
        and release_order[0] == 0,
        "LoopExiting must release exactly the surviving shell window",
    )


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise SmokeFailure(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_report(path: Path) -> dict[str, Any]:
    """Read one bounded JSON object while rejecting duplicate keys."""

    try:
        size = path.stat().st_size
        _require(0 < size <= MAX_REPORT_BYTES, f"report size {size} is not credible")
        text = path.read_text(encoding="utf-8")
        report = json.loads(text, object_pairs_hook=_reject_duplicate_keys)
    except SmokeFailure:
        raise
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise SmokeFailure(f"could not read headed surface report {path}: {error}") from error
    _require(isinstance(report, dict), "headed surface report root must be an object")
    return report


def validate_host_environment(
    environment: Mapping[str, str],
    *,
    authoritative: bool,
    system_name: str,
) -> None:
    if not authoritative:
        return
    _require(system_name == "Linux", "authoritative qualification requires Linux")
    _require_nonempty_string(environment.get("WAYLAND_DISPLAY"), "WAYLAND_DISPLAY")
    _require(
        environment.get("XDG_SESSION_TYPE", "").lower() == "wayland",
        "authoritative qualification requires XDG_SESSION_TYPE=wayland",
    )


def build_command(
    *,
    binary: Path,
    config: Path,
    report: Path,
    authoritative: bool,
) -> list[str]:
    command = [
        str(binary),
        "--config",
        str(config),
        "--headed-surface-smoke",
        str(report),
    ]
    if authoritative:
        command.extend(["--display-server", "wayland"])
    return command


def authoritative_build_command(target_dir: Path) -> list[str]:
    return [
        "cargo",
        "build",
        "--release",
        "--locked",
        "-p",
        "clonk-app",
        "--bin",
        "clonk-app",
        "--target-dir",
        str(target_dir),
        "--message-format=json-render-diagnostics",
    ]


def build_environment(
    base: Mapping[str, str],
    *,
    workspace: Path,
    user_data: Path,
    cache_dir: Path,
    logs_dir: Path,
    temp_dir: Path,
    expected_backend: str,
) -> dict[str, str]:
    environment = dict(base)
    for key in CONTROLLED_ENVIRONMENT_KEYS:
        environment.pop(key, None)
    for key in tuple(environment):
        if key.startswith("LC_APP_") or key.startswith("WGPU_"):
            environment.pop(key)
    environment.update(
        {
            "WGPU_BACKEND": expected_backend,
            "LC_INSTALL_ROOT": str(workspace),
            "LC_CONTENT_DIR": str(workspace / "content"),
            "LC_USER_DATA_DIR": str(user_data),
            "LC_CACHE_DIR": str(cache_dir),
            "LC_LOGS_DIR": str(logs_dir),
            "LC_TEMP_DIR": str(temp_dir),
            "LC_GAME_UPDATE_RECOVERY_COMPLETE": "1",
            "RUST_BACKTRACE": "1",
        }
    )
    return environment


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _git_commit(workspace: Path) -> str:
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=workspace,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise SmokeFailure(f"could not identify the tested commit: {completed.stderr.strip()}")
    return _require_nonempty_string(completed.stdout.strip(), "tested commit")


def _git_path_lines(workspace: Path, arguments: Sequence[str], label: str) -> list[str]:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=workspace,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise SmokeFailure(f"could not inspect {label}: {completed.stderr.strip()}")
    return [line for line in completed.stdout.splitlines() if line]


def require_clean_workspace(workspace: Path) -> None:
    """Reject source drift without asking Git to inspect the content symlink."""

    tracked = _git_path_lines(
        workspace,
        ["diff", "--name-only", "HEAD", "--", ".", ":(exclude)content"],
        "tracked workspace changes",
    )
    untracked = _git_path_lines(
        workspace,
        ["ls-files", "--others", "--exclude-standard", "--", ".", ":(exclude)content"],
        "untracked workspace files",
    )
    unexpected_untracked = [path for path in untracked if path != ".clonk-update.lock"]
    _require(
        not tracked and not unexpected_untracked,
        "authoritative qualification requires a clean source tree; "
        f"tracked={tracked} untracked={unexpected_untracked}",
    )


def _git_revision(workspace: Path, revision: str, label: str) -> str:
    completed = subprocess.run(
        ["git", "rev-parse", revision],
        cwd=workspace,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise SmokeFailure(f"could not identify {label}: {completed.stderr.strip()}")
    return _require_nonempty_string(completed.stdout.strip(), label)


def require_clean_content(workspace: Path) -> str:
    expected = _git_revision(workspace, "HEAD:content", "pinned content commit")
    content = (workspace / "content").resolve()
    actual = _git_revision(content, "HEAD", "checked-out content commit")
    _require(
        actual == expected,
        f"content checkout is {actual}, but the tested commit pins {expected}",
    )
    tracked = _git_path_lines(
        content,
        ["diff", "--name-only", "HEAD", "--", "."],
        "tracked content changes",
    )
    untracked = _git_path_lines(
        content,
        ["ls-files", "--others", "--exclude-standard", "--", "."],
        "untracked content files",
    )
    _require(
        not tracked and not untracked,
        f"authoritative qualification requires clean pinned content; "
        f"tracked={tracked} untracked={untracked}",
    )
    return actual


def _write_text(path: Path, value: str) -> None:
    with path.open("x", encoding="utf-8") as handle:
        handle.write(value)
        handle.flush()
        os.fsync(handle.fileno())


def _write_json(path: Path, value: Any) -> None:
    _write_text(path, json.dumps(value, indent=2, sort_keys=True) + "\n")


def _copy_built_binary(source: Path, destination: Path) -> None:
    with source.open("rb") as source_handle, destination.open("xb") as destination_handle:
        shutil.copyfileobj(source_handle, destination_handle)
        destination_handle.flush()
        os.fsync(destination_handle.fileno())
    destination.chmod(source.stat().st_mode & 0o777)


def default_artifact_dir(workspace: Path) -> Path:
    timestamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return workspace / "target" / "headed-surface-smoke" / f"{timestamp}-{os.getpid()}"


def _timeout_output(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return str(value)


def _run_authoritative_build(
    *,
    workspace: Path,
    artifact_dir: Path,
    target_dir: Path,
    timeout_seconds: float,
) -> Path:
    command = authoritative_build_command(target_dir)
    environment = os.environ.copy()
    for key in CONTROLLED_BUILD_ENVIRONMENT_KEYS:
        environment.pop(key, None)
    for key in tuple(environment):
        if key.startswith(CONTROLLED_BUILD_ENVIRONMENT_PREFIXES):
            environment.pop(key)
    try:
        completed = subprocess.run(
            command,
            cwd=workspace,
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        _write_text(artifact_dir / "cargo-build.stdout.log", _timeout_output(error.stdout))
        _write_text(artifact_dir / "cargo-build.stderr.log", _timeout_output(error.stderr))
        raise SmokeFailure(
            f"authoritative release build timed out after {timeout_seconds}s; "
            f"artifacts: {artifact_dir}"
        ) from error
    _write_text(artifact_dir / "cargo-build.stdout.log", completed.stdout)
    _write_text(artifact_dir / "cargo-build.stderr.log", completed.stderr)
    _require(
        completed.returncode == 0,
        f"authoritative release build exited {completed.returncode}; artifacts: {artifact_dir}",
    )
    executables = []
    for line in completed.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            raise SmokeFailure(f"Cargo emitted a non-JSON build record: {line!r}") from error
        if (
            isinstance(message, dict)
            and message.get("reason") == "compiler-artifact"
            and isinstance(message.get("target"), dict)
            and message["target"].get("name") == "clonk-app"
            and "bin" in message["target"].get("kind", [])
            and isinstance(message.get("executable"), str)
        ):
            executable = Path(message["executable"])
            executables.append(
                (executable if executable.is_absolute() else workspace / executable).resolve()
            )
    _require(
        len(executables) == 1,
        f"Cargo reported {len(executables)} clonk-app executables instead of one",
    )
    binary = executables[0]
    target_dir = target_dir.resolve()
    _require(
        target_dir in binary.parents,
        f"Cargo reported an executable outside the forced target directory: {binary}",
    )
    _require(binary.is_file(), f"Cargo's clonk-app executable does not exist: {binary}")
    return binary


def run_smoke(arguments: argparse.Namespace) -> Path:
    authoritative = not arguments.wiring_only
    workspace = arguments.workspace.resolve()
    artifact_dir = (arguments.artifact_dir or default_artifact_dir(workspace)).resolve()
    expected_backend = arguments.backend

    validate_host_environment(
        os.environ,
        authoritative=authoritative,
        system_name=platform.system(),
    )
    if authoritative:
        _require(expected_backend == "vulkan", "authoritative qualification forces Vulkan")
        _require(
            arguments.binary is None,
            "authoritative qualification builds clonk-app in-run; --binary is wiring-only",
        )
    _require(
        (workspace / "content").is_dir(),
        f"content checkout is unavailable: {workspace / 'content'}",
    )
    try:
        artifact_dir.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        raise SmokeFailure(
            f"could not create fresh artifact directory {artifact_dir}: {error}"
        ) from error

    commit_before = _git_commit(workspace)
    if authoritative:
        require_clean_workspace(workspace)
        content_commit = require_clean_content(workspace)
        build_target_dir = (artifact_dir / "cargo-target").resolve()
        built_binary = _run_authoritative_build(
            workspace=workspace,
            artifact_dir=artifact_dir,
            target_dir=build_target_dir,
            timeout_seconds=arguments.build_timeout_seconds,
        )
        built_binary_sha256 = _sha256(built_binary)
        binary = artifact_dir / "clonk-app"
        _copy_built_binary(built_binary, binary)
        _require(
            _sha256(binary) == built_binary_sha256,
            "the preserved clonk-app binary differs from Cargo's executable",
        )
        try:
            shutil.rmtree(build_target_dir)
        except OSError as error:
            raise SmokeFailure(
                f"could not remove the temporary authoritative target {build_target_dir}: {error}"
            ) from error
    else:
        build_target_dir = None
        built_binary_sha256 = None
        content_commit = None
        binary = (arguments.binary or workspace / "target" / "debug" / "clonk-app").resolve()
    _require(binary.is_file(), f"clonk-app binary does not exist: {binary}")
    binary_sha256_before = _sha256(binary)

    app_report = artifact_dir / "app-report.json"
    config = artifact_dir / "Clonk.ini"
    user_data = artifact_dir / "user-data"
    cache_dir = artifact_dir / "cache"
    logs_dir = artifact_dir / "logs"
    temp_dir = artifact_dir / "temp"
    for directory in (user_data, cache_dir, logs_dir, temp_dir):
        directory.mkdir()
    _write_text(config, SMOKE_CONFIG)
    command = build_command(
        binary=binary,
        config=config,
        report=app_report,
        authoritative=authoritative,
    )
    environment = build_environment(
        os.environ,
        workspace=workspace,
        user_data=user_data,
        cache_dir=cache_dir,
        logs_dir=logs_dir,
        temp_dir=temp_dir,
        expected_backend=expected_backend,
    )

    started = dt.datetime.now(dt.timezone.utc)
    monotonic_start = time.monotonic()
    try:
        completed = subprocess.run(
            command,
            cwd=workspace,
            env=environment,
            capture_output=True,
            text=True,
            timeout=arguments.timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        _write_text(artifact_dir / "stdout.log", _timeout_output(error.stdout))
        _write_text(artifact_dir / "stderr.log", _timeout_output(error.stderr))
        raise SmokeFailure(
            f"headed surface smoke timed out after {arguments.timeout_seconds}s; "
            f"artifacts: {artifact_dir}"
        ) from error
    elapsed = time.monotonic() - monotonic_start
    _write_text(artifact_dir / "stdout.log", completed.stdout)
    _write_text(artifact_dir / "stderr.log", completed.stderr)
    _require(
        completed.returncode == 0,
        f"clonk-app exited {completed.returncode}; artifacts: {artifact_dir}",
    )
    binary_sha256_after = _sha256(binary)
    _require(
        binary_sha256_after == binary_sha256_before,
        "tested clonk-app binary changed during the headed surface run",
    )
    commit_after = _git_commit(workspace)
    _require(commit_after == commit_before, "workspace HEAD changed during the headed surface run")
    if authoritative:
        require_clean_workspace(workspace)
        _require(
            require_clean_content(workspace) == content_commit,
            "content commit changed during the headed surface run",
        )

    report = load_report(app_report)
    validate_report(
        report,
        authoritative=authoritative,
        expected_backend=expected_backend,
    )
    if authoritative:
        _require(
            report["wayland_display"] == environment["WAYLAND_DISPLAY"],
            "report came from a different Wayland display",
        )

    evidence_path = artifact_dir / "qualification.json"
    _write_json(
        evidence_path,
        {
            "schema_version": 1,
            "kind": EVIDENCE_KIND,
            "authoritative": authoritative,
            "qualification": (
                "linux-wayland-vulkan-proprietary-nvidia"
                if authoritative
                else "real-surface-wiring-only"
            ),
            "started_at_utc": started.isoformat(),
            "elapsed_seconds": elapsed,
            "workspace": str(workspace),
            "git_commit": commit_before,
            "content_commit": content_commit,
            "source_clean_before_and_after": authoritative,
            "build_command": (
                authoritative_build_command(build_target_dir)
                if build_target_dir is not None
                else None
            ),
            "build_target_dir": (
                str(build_target_dir) if build_target_dir is not None else None
            ),
            "controlled_build_environment": (
                {
                    "cleared_keys": list(CONTROLLED_BUILD_ENVIRONMENT_KEYS),
                    "cleared_prefixes": list(CONTROLLED_BUILD_ENVIRONMENT_PREFIXES),
                }
                if authoritative
                else None
            ),
            "binary": str(binary),
            "cargo_artifact_sha256": built_binary_sha256,
            "binary_sha256_before": binary_sha256_before,
            "binary_sha256_after": binary_sha256_after,
            "platform": platform.platform(),
            "command": command,
            "controlled_environment": {
                key: environment.get(key)
                for key in (
                    "WGPU_BACKEND",
                    "WGPU_ADAPTER_NAME",
                    "WGPU_POWER_PREF",
                    "LC_INSTALL_ROOT",
                    "LC_CONTENT_DIR",
                    "LC_USER_DATA_DIR",
                    "LC_CACHE_DIR",
                    "LC_LOGS_DIR",
                    "LC_TEMP_DIR",
                    "LC_CONFIG_FILE",
                    "LC_LOG",
                    "LC_GAME_UPDATE_NOTICE",
                    "LC_GAME_UPDATE_RECOVERY_COMPLETE",
                    "RUST_LOG",
                    "RUST_BACKTRACE",
                    "WAYLAND_DISPLAY",
                    "XDG_SESSION_TYPE",
                )
            },
            "process_returncode": completed.returncode,
            "app_report": report,
        },
    )
    return evidence_path


def positive_seconds(value: str) -> float:
    try:
        seconds = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("must be a number") from error
    if not math.isfinite(seconds) or seconds <= 0:
        raise argparse.ArgumentTypeError("must be finite and positive")
    return seconds


def build_argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=WORKSPACE)
    parser.add_argument(
        "--binary",
        type=Path,
        help="existing clonk-app binary for --wiring-only (authoritative runs build in-run)",
    )
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--timeout-seconds", type=positive_seconds, default=45.0)
    parser.add_argument(
        "--build-timeout-seconds",
        type=positive_seconds,
        default=AUTHORITATIVE_BUILD_TIMEOUT_SECONDS,
    )
    parser.add_argument(
        "--backend",
        choices=("vulkan", "metal", "dx12", "gl"),
        default="vulkan",
        help="single WGPU backend to force (authoritative mode requires vulkan)",
    )
    parser.add_argument(
        "--wiring-only",
        action="store_true",
        help="drive real surfaces without claiming NVIDIA/Wayland crash coverage",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = build_argument_parser().parse_args(argv)
    try:
        evidence = run_smoke(arguments)
    except SmokeFailure as error:
        print(f"headed surface smoke failed: {error}", file=sys.stderr)
        return 1
    mode = (
        "authoritative proprietary-NVIDIA/Wayland qualification"
        if not arguments.wiring_only
        else "wiring-only real-surface smoke"
    )
    print(f"{mode} passed: {evidence}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
