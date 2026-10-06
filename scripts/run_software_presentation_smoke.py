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
import datetime
import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import tomllib
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent

REPORT_KIND = "clonk_software_present_smoke"
SCHEMA_VERSION = 3

#: Force software directly; the alternative disables GPU backends explicitly
#: and checks that ordinary startup chooses software after GPU attempts fail.
FORCE_SOFTWARE_ENVIRONMENT = "LC_SOFTWARE_PRESENTATION"
SMOKE_CONFIG = (
    "[Graphics]\nResolutionX=800\nResolutionY=600\nDisplayMode=1\nMaximized=false\n"
    "\n[Sound]\nSound=false\nMusic=false\nMenuMusic=false\nMenuSound=false\n"
)


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
        "--prebuilt-root",
        type=Path,
        help="verify and run a shipped runtime payload instead of building (requires --release)",
    )
    parser.add_argument(
        "--no-xvfb",
        action="store_true",
        help="never wrap the probe in xvfb-run, even without a display",
    )
    parser.add_argument(
        "--check-input", action="store_true",
        help="require a native pointer event mapped at application scale 2",
    )
    parser.add_argument(
        "--automatic-fallback", action="store_true",
        help="disable all GPU backends and require automatic software fallback",
    )
    return parser.parse_args(argv)


def refuse_to_run_as_root() -> None:
    """`clonk-app` refuses to run as root; say so before spending a build."""
    if hasattr(os, "geteuid") and os.geteuid() == 0:
        raise SystemExit(
            "clonk-app refuses to run as root, so this probe cannot either. "
            "Run it as an ordinary user."
        )


def verify_prebuilt_binary(root: Path, prebuilt_root: Path) -> Path:
    """Accept only the current source's exact shipped native release payload."""
    architecture = platform.machine().lower()
    target = {
        ("linux", "x86_64"): "x86_64-unknown-linux-gnu",
        ("win32", "amd64"): "x86_64-pc-windows-msvc",
        ("win32", "x86_64"): "x86_64-pc-windows-msvc",
        ("darwin", "arm64"): "aarch64-apple-darwin",
        ("darwin", "aarch64"): "aarch64-apple-darwin",
        ("darwin", "x86_64"): "x86_64-apple-darwin",
    }.get((sys.platform, architecture))
    if target is None:
        raise SystemExit(f"no shipped prebuilt runtime for {sys.platform}/{architecture}")

    def git(*arguments):
        return subprocess.check_output(["git", "-C", str(root), *arguments], text=True).strip()

    inputs = (".", ":(exclude)content")
    if (git("diff", "--name-only", "HEAD", "--", *inputs)
            or git("ls-files", "--others", "--exclude-standard", "--", *inputs)):
        raise SystemExit("prebuilt release qualification requires committed source inputs")

    prebuilt_root = prebuilt_root.absolute()
    manifest = prebuilt_root / "manifest.json"
    try:
        document = json.loads(manifest.read_text(encoding="utf-8"))
        manifest_target = document.get("target") if isinstance(document, dict) else None
        version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    except (OSError, UnicodeError, ValueError, KeyError, TypeError) as error:
        raise SystemExit(f"cannot read prebuilt runtime identity: {error}") from error
    allowed_targets = {target}
    if sys.platform == "darwin":
        allowed_targets.add("universal-apple-darwin")
    if not isinstance(manifest_target, str) or manifest_target not in allowed_targets:
        raise SystemExit(f"prebuilt runtime target {manifest_target!r} cannot run on {target}")

    suffix = ".exe" if sys.platform == "win32" else ""
    filenames = [f"payload/{name}{suffix}" for name in ("c4group", "clonk-app", "clonk-game")]
    command = [
        sys.executable, str(root / "scripts/release-prebuild-manifest.py"), "verify",
        "--root", str(prebuilt_root), "--manifest", str(manifest),
        "--head-sha", git("rev-parse", "HEAD"),
        "--tree-sha", git("rev-parse", "HEAD^{tree}"),
        "--version", version, "--kind", "runtime", "--target", manifest_target,
        "--provenance-root", str(root),
    ]
    for name in filenames:
        command.extend(("--file", name))
    try:
        subprocess.run(command, cwd=root, check=True, capture_output=True, text=True)
    except subprocess.CalledProcessError as error:
        raise SystemExit(f"prebuilt runtime verification failed: {error.stderr.strip()}") from error
    if sys.platform != "win32":
        for name in filenames:
            binary = prebuilt_root / name
            binary.chmod(binary.stat().st_mode | 0o111)
    return prebuilt_root / f"payload/clonk-app{suffix}"


def build_binary(root: Path, release: bool, prebuilt_root: Path | None = None) -> Path:
    if prebuilt_root is not None:
        if not release:
            raise SystemExit("prebuilt runtime qualification requires --release")
        return verify_prebuilt_binary(root, prebuilt_root)
    profile = ["--release"] if release else []
    # Preserve feature unification with the other shipped Windows executables.
    # The release job validates this same graph and its static-CRT imports.
    targets = (
        ["-p", "clonk-app", "-p", "clonk-game", "-p", "clonk-c4group"]
        if release and sys.platform == "win32"
        else ["-p", "clonk-app", "--bin", "clonk-app"]
    )
    subprocess.run(
        ["cargo", "build", "--locked", *targets, *profile],
        cwd=root,
        check=True,
    )
    target = os.environ.get("CARGO_TARGET_DIR")
    root = Path(target) if target else root / "target"
    if triple := os.environ.get("CARGO_BUILD_TARGET"):
        root /= triple
    executable = "clonk-app.exe" if sys.platform == "win32" else "clonk-app"
    binary = root / ("release" if release else "debug") / executable
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


def file_digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def prebuilt_manifest_digest(prebuilt_root: Path | None) -> str | None:
    if prebuilt_root is None:
        return None
    return hashlib.sha256(read_prebuilt_manifest(prebuilt_root)).hexdigest()


def read_prebuilt_manifest(prebuilt_root: Path) -> bytes:
    manifest = prebuilt_root / "manifest.json"
    try:
        if prebuilt_root.is_symlink() or manifest.is_symlink():
            raise OSError("prebuilt manifest must be a regular file in a real payload directory")
        flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
        with os.fdopen(os.open(manifest, flags), "rb") as source:
            if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
                raise OSError("prebuilt manifest is not a regular file")
            return source.read()
    except OSError as error:
        raise SystemExit(f"cannot read prebuilt runtime identity: {error}") from error


def build_evidence(root: Path, release: bool, prebuilt_root: Path | None = None) -> dict:
    """Record build inputs after ``build_binary`` verified the selected artifact."""
    qualifier = {
        "build_target": os.environ.get("CARGO_BUILD_TARGET"),
        "rustflags": os.environ.get("RUSTFLAGS"),
        "encoded_rustflags": os.environ.get("CARGO_ENCODED_RUSTFLAGS"),
        "rustc": subprocess.check_output(["rustc", "-vV"], cwd=root, text=True),
    }
    evidence = {
        "build_origin": "local", "build_profile": "release" if release else "debug", **qualifier,
        "qualification_environment": qualifier, "prebuilt_manifest": None, "prebuilt_manifest_sha256": None,
    }
    if prebuilt_root is None:
        return evidence
    try:
        encoded = read_prebuilt_manifest(prebuilt_root)
        document = json.loads(encoded)
        recipe = document["provenance"]["producer_recipe"]
        compiled = recipe["operation"] == "build"
        environment = recipe["environment"] if compiled else {}
        evidence.update({
            "build_origin": "prebuilt", "build_profile": recipe["profile"],
            "build_target": recipe["target"] if compiled else None,
            "rustflags": environment.get("RUSTFLAGS"),
            "encoded_rustflags": environment.get("CARGO_ENCODED_RUSTFLAGS"), "rustc": recipe["rustc"],
            "prebuilt_manifest": document, "prebuilt_manifest_sha256": hashlib.sha256(encoded).hexdigest(),
        })
    except (OSError, UnicodeError, ValueError, KeyError, TypeError, AttributeError) as error:
        raise SystemExit(f"cannot read verified prebuilt build metadata: {error}") from error
    return evidence


def source_identity() -> dict:
    def git(*args, root=REPOSITORY):
        return subprocess.check_output(["git", "-C", str(root), *args])

    inputs = [".cargo", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates",
              "scripts/run_software_presentation_smoke.py",
              "scripts/configure-msvc-runtime.sh", "scripts/validate-msvc-runtime.sh"]
    paths = git("ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *inputs)
    sources = {
        name: file_digest(REPOSITORY / name) if (REPOSITORY / name).is_file() else "missing"
        for name in sorted(set(paths.decode().split("\0"))) if name
    }
    content = git("rev-parse", "HEAD", root=REPOSITORY / "content").decode().strip()
    if content != git("rev-parse", "HEAD:content").decode().strip():
        raise SystemExit("content checkout differs from the pinned content revision")
    if (git("diff", "--name-only", "HEAD", root=REPOSITORY / "content").strip()
            or git("ls-files", "--others", "--exclude-standard", root=REPOSITORY / "content").strip()):
        raise SystemExit("content checkout has modified parity input")
    return {
        "commit": git("rev-parse", "HEAD").decode().strip(),
        "content_commit": content,
        "sources_sha256": hashlib.sha256(json.dumps(sources, sort_keys=True).encode()).hexdigest(),
        "source_dirty": bool(
            git("diff", "--name-only", "HEAD", "--", *inputs).strip()
            or git("ls-files", "--others", "--exclude-standard", "--", *inputs).strip()
        ),
    }


def check_report(
    report_path: Path, *, check_input: bool = False, automatic_fallback: bool = False,
    expected_backend: str | None = None,
) -> dict:
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
    if expected_backend and report.get("display_backend") != expected_backend:
        failures.append(f"window backend is not {expected_backend}: {report.get('display_backend')}")
    attempts = report.get("gpu_attempt_backends")
    if automatic_fallback:
        if (report.get("software_reason") != "no-adapter"
                or not isinstance(attempts, list) or not attempts
                or any(backends != [] for backends in attempts)):
            failures.append("automatic fallback did not follow failed GPU attempts with no backends")
    elif report.get("software_reason") != "forced" or attempts != []:
        failures.append("forced software presentation attempted GPU startup")
    if check_input and report.get("input_mapping") != {
        "window_position": [128, 96], "gui_position": [64, 48], "scale": 2,
    }:
        failures.append("no correctly mapped native pointer event at application scale 2")
    from PIL import Image

    for suffix, extent in (
        (".screenshot.png", report.get("resized_extent")),
        (".thumbnail.png", [200, 150]),
    ):
        capture = report_path.with_suffix(suffix)
        try:
            with Image.open(capture) as decoded:
                if list(decoded.size) != extent:
                    failures.append(f"{capture.name} has incorrect dimensions: {decoded.size}")
                if decoded.convert("RGBA").getextrema() != (
                    (0x6f, 0x6f), (0x2f, 0x2f), (0xa8, 0xa8), (255, 255),
                ):
                    failures.append(f"{capture.name} does not contain the presented pixels")
        except (OSError, ValueError) as error:
            failures.append(f"cannot decode {capture.name}: {error}")
    if not report.get("presented_before_resize"):
        failures.append("no frame reached the window before the resize")
    if not report.get("presented_after_resize"):
        failures.append("no frame reached the window after the resize")
    if report.get("initial_extent") == report.get("resized_extent"):
        failures.append(
            "the drawable never changed size, so the resize proved nothing "
            f"({report.get('initial_extent')})"
        )
    # A resize moves the frame and the drawable together, so it never produces
    # a scale above one. Only the transition phases do, which is why they are
    # checked separately rather than folded into the resize above.
    phases = report.get("phases") or []
    names = [phase.get("name") for phase in phases]
    if names != ["windowed", "fullscreen", "windowed-again"]:
        failures.append(f"the transition sequence is incomplete: {names}")
    for phase in phases:
        name = phase.get("name")
        if not phase.get("presented"):
            failures.append(f"the {name} phase presented no frame")
        clip = phase.get("clip_rect") or [0, 0, 0, 0]
        drawable = phase.get("drawable_extent") or [0, 0]
        if clip[0] + clip[2] > drawable[0] or clip[1] + clip[3] > drawable[1]:
            failures.append(
                f"the {name} phase presented through a clip that does not fit "
                f"its drawable: clip {clip} in {drawable}"
            )
    fullscreen = next(
        (phase for phase in phases if phase.get("name") == "fullscreen"), None
    )
    if fullscreen is not None and fullscreen.get("scale", 0) <= 1:
        failures.append(
            "the fullscreen phase never scaled, so a wrong scale would go "
            "unnoticed"
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

    artifacts = (arguments.artifact_dir or (REPOSITORY / "target" / "software-present-smoke")).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    report_path = artifacts / "report.json"
    # The probe refuses to overwrite an existing report, so a stale one from a
    # previous run would fail before it started.
    report_path.unlink(missing_ok=True)
    for name in ("report.screenshot.png", "report.thumbnail.png", "qualification.json"):
        (artifacts / name).unlink(missing_ok=True)

    source_before = source_identity()
    if arguments.release and source_before.get("source_dirty"):
        raise SystemExit("release qualification requires committed source inputs")
    if arguments.prebuilt_root is not None and not arguments.release:
        raise SystemExit("prebuilt runtime qualification requires --release")
    manifest_sha256 = prebuilt_manifest_digest(arguments.prebuilt_root)
    binary = build_binary(REPOSITORY, arguments.release, arguments.prebuilt_root)
    binary_sha256 = file_digest(binary)
    config = artifacts / "Clonk.ini"
    screenshots = artifacts / "screenshots"
    screenshots.mkdir(exist_ok=True)
    # C++ appends ScreenshotFolder to the install root as raw text, even for
    # absolute paths. Use a relative path and precreate its parent directories.
    screenshot_folder = os.path.relpath(screenshots, REPOSITORY)
    config.write_text(
        f"[General]\nScreenshotFolder={screenshot_folder}\n\n{SMOKE_CONFIG}", encoding="utf-8",
    )
    environment = dict(os.environ)
    for key in tuple(environment):
        if key.startswith(("LC_APP_", "WGPU_")) or key in (
            "LC_CONFIG_FILE", "LC_GAME_UPDATE_NOTICE", "LC_LANGUAGE_OVERRIDE", "LC_LOG",
        ):
            environment.pop(key)
    environment.update({
        "LC_INSTALL_ROOT": str(REPOSITORY),
        "LC_CONTENT_DIR": str(REPOSITORY / "content"),
        "LC_USER_DATA_DIR": str(artifacts / "user-data"),
        "LC_CACHE_DIR": str(artifacts / "cache"),
        "LC_LOGS_DIR": str(artifacts / "logs"),
        "LC_TEMP_DIR": str(artifacts / "temp"),
        "LC_GAME_UPDATE_RECOVERY_COMPLETE": "1",
    })
    if arguments.automatic_fallback:
        environment.pop(FORCE_SOFTWARE_ENVIRONMENT, None)
        environment["WGPU_BACKEND"] = ""
    else:
        environment[FORCE_SOFTWARE_ENVIRONMENT] = "1"

    command = [
        *launch_prefix(no_xvfb=arguments.no_xvfb),
        str(binary),
        "--config", str(config),
        "--software-present-smoke",
        str(report_path),
    ]
    if arguments.check_input:
        command.append("--software-present-input")
    with (artifacts / "run.log").open("w", encoding="utf-8") as output:
        try:
            completed = subprocess.run(
                command, cwd=REPOSITORY, env=environment, check=False,
                stdout=output, stderr=subprocess.STDOUT, timeout=60,
            )
        except subprocess.TimeoutExpired as error:
            raise SystemExit(f"software probe timed out; see {artifacts / 'run.log'}") from error

    report = check_report(
        report_path, check_input=arguments.check_input,
        automatic_fallback=arguments.automatic_fallback,
        expected_backend={"win32": "windows", "darwin": "appkit"}.get(sys.platform),
    )
    if completed.returncode != 0:
        raise SystemExit(
            f"the probe reported success but exited {completed.returncode}; "
            "treat the exit code as authoritative"
        )
    if source_identity() != source_before:
        raise SystemExit("source changed during software presentation qualification")
    if file_digest(binary) != binary_sha256:
        raise SystemExit("the software presentation executable changed during the run")
    build = build_evidence(REPOSITORY, arguments.release, arguments.prebuilt_root)
    if build["prebuilt_manifest_sha256"] != manifest_sha256:
        raise SystemExit("the prebuilt runtime manifest changed during qualification")
    evidence = {
        "schema_version": 1,
        "kind": "clonk_software_presentation_qualification",
        "recorded_at": datetime.datetime.now(datetime.UTC).isoformat(),
        **source_before,
        "os": platform.platform(),
        "os_version": platform.version(),
        "architecture": platform.machine(),
        **build,
        "binary_sha256": binary_sha256,
        "window_backend": report["display_backend"],
        "automatic_fallback": arguments.automatic_fallback,
        "native_input_checked": arguments.check_input,
        "artifacts": {name: file_digest(artifacts / name) for name in (
            "report.json", "report.screenshot.png", "report.thumbnail.png", "run.log",
        )},
    }
    (artifacts / "qualification.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")

    print(
        "software presentation smoke passed: presented at "
        f"{report['initial_extent']}, resized to {report['resized_extent']}, "
        "and left no window behind"
    )
    print(f"report: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
