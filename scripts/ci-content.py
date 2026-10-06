#!/usr/bin/env python3
"""Materialize and verify CI's exact pinned content checkout; caches are optional."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time


CACHE_FORMAT = "v2"
DOWNLOAD_BUDGET_SECONDS = 480
DOWNLOAD_ATTEMPTS = 2
REPOSITORY = Path(__file__).resolve().parent.parent
WINDOWS = os.name == "nt"


class ContentError(Exception):
    """The content input cannot be verified or materialized."""


def is_link(path: Path) -> bool:
    if path.is_symlink() or getattr(path, "is_junction", lambda: False)():
        return True
    try:
        return bool(getattr(path.lstat(), "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400))
    except FileNotFoundError:
        return False


def git(repository: Path, *arguments: str, extra_env: dict | None = None) -> str:
    try:
        completed = subprocess.run(
            ["git", "-C", str(repository), "-c", "core.fsmonitor=false", *arguments], check=False,
            capture_output=True, text=True, timeout=30,
            env={**os.environ, "GIT_NO_LAZY_FETCH": "1", "GIT_OPTIONAL_LOCKS": "0", **(extra_env or {})},
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ContentError(f"git {' '.join(arguments)} failed: {error}") from error
    if completed.returncode:
        raise ContentError(
            f"git {' '.join(arguments)} exited {completed.returncode}: {completed.stderr.strip()}"
        )
    return completed.stdout.strip()


def owned_modules_path(repository: Path) -> Path:
    """Never follow a restored gitdir or parent-directory link during repair."""
    metadata = repository / ".git"
    if is_link(metadata) or not metadata.is_dir():
        raise ContentError("CI content repair requires the checkout's own real .git directory")
    if Path(git(repository, "rev-parse", "--show-toplevel")).resolve() != repository.resolve():
        raise ContentError("parent Git root differs from this CI checkout")
    if Path(git(repository, "rev-parse", "--absolute-git-dir")).resolve() != metadata.resolve():
        raise ContentError("parent Git directory differs from this CI checkout")
    if is_link(metadata / "modules"):
        raise ContentError("CI modules directory is a symlink; refusing external cache repair")
    return metadata / "modules" / "content"


def configured_url(repository: Path) -> str:
    if git(repository, "config", "--file", ".gitmodules", "--get", "submodule.content.path") != "content":
        raise ContentError(".gitmodules does not configure the fixed content directory")
    url = git(repository, "config", "--file", ".gitmodules", "--get", "submodule.content.url")
    if not url or "\n" in url:
        raise ContentError(".gitmodules content URL is empty or ambiguous")
    return url


def verify_parent_gitlink(repository: Path, revision: str) -> None:
    entry = git(repository, "ls-tree", "HEAD", "--", "content")
    match = re.fullmatch(r"160000 commit ([0-9a-f]{40})\tcontent", entry)
    if match is None:
        raise ContentError("parent HEAD:content is not a content gitlink")
    if match.group(1) != revision:
        raise ContentError("requested revision differs from parent HEAD:content")


def verify_checkout(repository: Path, revision: str) -> None:
    content = repository / "content"
    if is_link(content) or not content.is_dir():
        raise ContentError("content checkout is not a real directory")
    if Path(git(content, "rev-parse", "--show-toplevel")).resolve() != content.resolve():
        raise ContentError("content Git root is not the content checkout")
    modules = owned_modules_path(repository)
    if is_link(modules):
        raise ContentError("content Git directory is a symlink")
    if Path(git(content, "rev-parse", "--absolute-git-dir")).resolve() != modules.resolve():
        raise ContentError("content Git directory is outside CI's own modules/content store")
    expected_url = configured_url(repository)
    if git(content, "config", "--get", "remote.origin.url") != expected_url:
        raise ContentError("content origin differs from the configured .gitmodules URL")
    if git(content, "rev-parse", "HEAD") != revision:
        raise ContentError("content HEAD differs from the requested gitlink")
    if git(content, "status", "--porcelain=v1", "--untracked-files=all"):
        raise ContentError("content has modified or untracked parity input")
    if git(content, "ls-files", "--others", "-z"):
        raise ContentError("content has untracked parity input, including ignored files")
    # Rebuild only a scratch index so stale cached stat data, assume-unchanged,
    # and skip-worktree flags cannot hide modified or missing parity bytes.
    with tempfile.TemporaryDirectory(prefix="clonk-ci-content-index-") as directory:
        environment = {"GIT_INDEX_FILE": str(Path(directory) / "index")}
        git(content, "-c", "core.sparseCheckout=false", "read-tree", revision, extra_env=environment)
        try:
            git(content, "-c", "core.sparseCheckout=false", "update-index", "--really-refresh", extra_env=environment)
            git(content, "-c", "core.sparseCheckout=false", "diff-files", "--quiet", "--no-ext-diff", extra_env=environment)
        except ContentError as error:
            raise ContentError(f"content tracked bytes differ or cannot be verified: {error}") from error


def remove_readonly(function, name: str, information: tuple) -> None:
    error = information[1]
    path = Path(name)
    if not isinstance(error, PermissionError) or is_link(path):
        raise error
    path.chmod(stat.S_IRWXU if path.is_dir() else stat.S_IRUSR | stat.S_IWUSR)
    function(name)


def reset_owned_content(repository: Path, *, include_store: bool = True) -> None:
    modules = owned_modules_path(repository)
    for path in ((repository / "content", modules) if include_store else (repository / "content",)):
        if is_link(path) and path.is_dir() and not path.is_symlink():
            path.rmdir()
        elif path.is_symlink() or path.is_file():
            path.unlink()
        elif path.exists():
            shutil.rmtree(path, onerror=remove_readonly)


def verify_object_store(repository: Path, revision: str) -> None:
    """Validate a portable cache without requiring its old worktree to exist."""
    modules, content = owned_modules_path(repository), repository / "content"
    if is_link(modules) or not modules.is_dir():
        raise ContentError("cached content Git store is absent or is an external link")
    for directory, children, files in os.walk(modules, followlinks=False):
        if any(is_link(Path(directory) / name) for name in children + files):
            raise ContentError("cached content Git store contains an external link")
    alternates = modules / "objects" / "info" / "alternates"
    if (alternates.exists() and alternates.read_text(encoding="utf-8").strip()) or os.environ.get("GIT_ALTERNATE_OBJECT_DIRECTORIES"):
        raise ContentError("cached content Git store depends on external objects")
    prefix = (f"--git-dir={modules}", f"--work-tree={content}")
    environment = {"GIT_ALLOW_PROTOCOL": ""}

    def stored_git(*arguments: str) -> str:
        return git(repository, *prefix, *arguments, extra_env=environment)

    if Path(stored_git("rev-parse", "--absolute-git-dir")).resolve() != modules.resolve() or Path(
        stored_git("rev-parse", "--path-format=absolute", "--git-common-dir")
    ).resolve() != modules.resolve():
        raise ContentError("cached content Git store root is outside modules/content")
    if Path(stored_git("rev-parse", "--show-toplevel")).resolve() != content.resolve():
        raise ContentError("cached content Git worktree root differs from content")
    worktree = Path(stored_git("config", "--local", "--get", "core.worktree"))
    if (worktree if worktree.is_absolute() else modules / worktree).resolve() != content.resolve():
        raise ContentError("cached content Git configuration names an external worktree")
    if stored_git("config", "--local", "--get", "remote.origin.url") != configured_url(repository):
        raise ContentError("cached content origin differs from the configured .gitmodules URL")
    if stored_git("rev-parse", "--verify", "HEAD") != revision or stored_git("cat-file", "-t", revision) != "commit":
        raise ContentError("cached content HEAD differs from the requested gitlink")
    stored_git("fsck", "--full", "--no-reflogs", "--no-dangling")
    # Promisor repositories may exempt missing blobs from fsck; every pinned
    # tree object must still be locally available before offline hydration.
    stored_git("rev-list", "--objects", "--missing=error", revision)


def object_store_bytes(modules: Path) -> int | None:
    """Measure stored Git bytes, never claim these are downloaded bytes."""
    objects = modules / "objects"
    if is_link(modules) or is_link(objects):
        return None
    if not objects.exists():
        return 0
    total = 0
    try:
        for directory, children, files in os.walk(objects, followlinks=False):
            for name in children + files:
                entry = Path(directory) / name
                mode = entry.lstat().st_mode
                if is_link(entry):
                    return None
                if stat.S_ISREG(mode):
                    total += entry.lstat().st_size
    except OSError:
        return None
    return total


def write_report(report: dict, destination: Path | None) -> None:
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if destination:
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(encoded, encoding="utf-8")
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(summary).open("a", encoding="utf-8") as output:
            output.write(
                "### Verified pinned content\n\n"
                "| Observation | Value |\n| --- | --- |\n"
                f"| Requested revision | `{report['requested_revision']}` |\n"
                f"| Cache hit reported by action | {report['cache_hit']} |\n"
                f"| Cached input verified | {report['cache_verified']} |\n"
                f"| Restored Git store verified | {report['cache_store_verified']} |\n"
                f"| Hydrated checkout | {report['materialized']} |\n"
                f"| Hydration source | {report['hydration_source']} |\n"
                f"| Checkout clean | {report['checkout_clean']} |\n"
                f"| Hydration seconds | {report['hydration_elapsed_seconds']:.3f} |\n"
                f"| Total seconds | {report['total_elapsed_seconds']:.3f} |\n"
                f"| Object-store bytes before | {report['object_store_bytes_before']} |\n"
                f"| Object-store bytes after | {report['object_store_bytes_after']} |\n\n"
                "Object-store bytes are logical file sizes; network bytes are not measured.\n\n"
            )
    print(encoded, end="")


class WindowsJob:
    """All fetch descendants die when this job closes, including on timeout."""

    def __init__(self) -> None:
        import ctypes
        from ctypes import wintypes

        class BasicLimits(ctypes.Structure):
            _fields_ = [
                ("process_time", ctypes.c_longlong), ("job_time", ctypes.c_longlong),
                ("flags", wintypes.DWORD), ("minimum_working_set", ctypes.c_size_t),
                ("maximum_working_set", ctypes.c_size_t), ("active_processes", wintypes.DWORD),
                ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                ("scheduling", wintypes.DWORD),
            ]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [
                ("basic", BasicLimits), ("io_counters", ctypes.c_ulonglong * 6),
                ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                ("peak_process_memory", ctypes.c_size_t), ("peak_job_memory", ctypes.c_size_t),
            ]

        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.kernel.CreateJobObjectW.restype = wintypes.HANDLE
        self.kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.kernel.SetInformationJobObject.restype = wintypes.BOOL
        self.kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.kernel.AssignProcessToJobObject.restype = wintypes.BOOL
        self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.kernel.OpenProcess.restype = wintypes.HANDLE
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.kernel.CloseHandle.restype = wintypes.BOOL
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
        if not self.kernel.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.WinError(ctypes.get_last_error())
            self.close()
            raise error

    def assign(self, process: subprocess.Popen) -> None:
        import ctypes

        handle = self.kernel.OpenProcess(0x0101, False, process.pid)  # SET_QUOTA | TERMINATE.
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            if not self.kernel.AssignProcessToJobObject(self.handle, handle):
                raise ctypes.WinError(ctypes.get_last_error())
        finally:
            self.kernel.CloseHandle(handle)

    def close(self) -> None:
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


def run_bounded(repository: Path, budget: float, command: list[str], *, extra_env: dict | None = None) -> int:
    wrapped = [sys.executable, str(repository / "scripts" / "run_with_timeout.py"), str(budget), *command]
    environment = {**os.environ, **(extra_env or {})}
    if not WINDOWS:
        return subprocess.run(wrapped, check=False, stdout=sys.stderr, env=environment).returncode

    # The gate prevents even a fast child from escaping before job assignment.
    # EOF on a parent crash exits without starting the timeout helper or Git.
    # Windows execv starts a new PID. Keep the assigned gate alive while its
    # helper runs, or waiting on the old PID can close the job prematurely.
    gate = "import subprocess, sys; ready = sys.stdin.buffer.read(1); sys.exit(125) if ready != b'G' else sys.exit(subprocess.run(sys.argv[1:], check=False).returncode)"
    job = None
    process = None
    try:
        job = WindowsJob()
        process = subprocess.Popen(
            [sys.executable, "-c", gate, *wrapped], stdin=subprocess.PIPE, stdout=sys.stderr, env=environment,
        )
        job.assign(process)
        process.stdin.write(b"G")
        process.stdin.close()
        try:
            return process.wait(timeout=budget + 5)
        except subprocess.TimeoutExpired:
            return 124
    except OSError as error:
        raise ContentError(f"cannot bound the Windows content fetch process tree: {error}") from error
    finally:
        if job is not None:
            job.close()
        if process is not None and process.stdin is not None and not process.stdin.closed:
            process.stdin.close()
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=5)


def hydrate_object_store(repository: Path, revision: str, report: dict) -> None:
    reset_owned_content(repository, include_store=False)
    status = run_bounded(
        repository, 180,
        ["git", "-C", str(repository), "submodule", "update", "--init", "--force", "--checkout", "--no-fetch", "--", "content"],
        extra_env={"GIT_NO_LAZY_FETCH": "1", "GIT_ALLOW_PROTOCOL": ""},
    )
    if status:
        raise ContentError(f"offline cached content hydration exited {status}")
    verify_checkout(repository, revision)
    report.update(materialized=True, hydration_source="object_store", cache_store_verified=True)


def fetch_snapshot(repository: Path, revision: str) -> None:
    """Fetch all pinned objects together, without a branch-tip clone or lazy blobs."""
    verify_parent_gitlink(repository, revision)
    modules = owned_modules_path(repository)
    content = repository / "content"
    url = configured_url(repository)
    modules.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["git", "init", "--quiet", f"--separate-git-dir={modules}", str(content)], check=True,
    )
    for arguments in (
        ("config", "core.worktree", "../../../content"),
        ("remote", "add", "origin", url),
        ("fetch", "--no-tags", "--depth=1", "origin", revision),
        ("checkout", "--quiet", "--detach", revision),
    ):
        if arguments[0] in ("fetch", "checkout"):
            print(f"Content snapshot: {arguments[0]} {revision}", file=sys.stderr, flush=True)
        environment = None
        if arguments[0] == "fetch":
            # A live large snapshot gets the remaining overall deadline. A
            # stalled HTTP transfer still aborts early enough for a retry.
            environment = {**os.environ, "GIT_HTTP_LOW_SPEED_LIMIT": "1024", "GIT_HTTP_LOW_SPEED_TIME": "60"}
        subprocess.run(["git", "-C", str(content), *arguments], check=True, env=environment)


def materialize(repository: Path, revision: str, report: dict) -> None:
    deadline = time.monotonic() + DOWNLOAD_BUDGET_SECONDS
    for _ in range(DOWNLOAD_ATTEMPTS):
        if time.monotonic() >= deadline:
            break
        reset_owned_content(repository)
        started = time.monotonic()
        budget = deadline - started
        if budget <= 0:
            break
        status = run_bounded(
            repository, budget,
            [sys.executable, str(repository / "scripts" / "ci-content.py"),
             "--revision", revision, "--fetch-snapshot"],
        )
        report["attempts"].append({
            "budget_seconds": budget, "elapsed_seconds": time.monotonic() - started,
            "returncode": status,
        })
        if status == 0:
            verify_checkout(repository, revision)
            verify_object_store(repository, revision)
            report["materialized"] = True
            report["hydration_source"] = "origin"
            return
    raise ContentError("content download failed within its bounded retry deadline")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--cache-hit", choices=("true", "false"), default="false")
    parser.add_argument("--verify-only", action="store_true")
    parser.add_argument("--fetch-snapshot", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--report", type=Path)
    arguments = parser.parse_args(argv)
    if arguments.fetch_snapshot:
        try:
            fetch_snapshot(REPOSITORY, arguments.revision)
        except (ContentError, OSError, subprocess.CalledProcessError) as error:
            print(f"error: {error}", file=sys.stderr)
            return error.returncode if isinstance(error, subprocess.CalledProcessError) else 1
        return 0
    started = time.monotonic()
    report = {
        "schema_version": 1, "cache_format": CACHE_FORMAT,
        "requested_revision": arguments.revision, "content_revision": None,
        "cache_hit": arguments.cache_hit == "true", "materialized": False,
        "cache_verified": False, "cache_store_verified": False, "repaired_cache": False,
        "hydration_source": None,
        "checkout_clean": False, "attempts": [],
        "hydration_elapsed_seconds": 0.0,
        "object_store_bytes_before": None, "object_store_bytes_after": None,
        "network_bytes": None,
    }
    modules = None
    status = 0
    try:
        if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision):
            raise ContentError("requested content revision must be a lowercase full Git SHA")
        verify_parent_gitlink(REPOSITORY, arguments.revision)
        modules = owned_modules_path(REPOSITORY)
        configured_url(REPOSITORY)
        report["object_store_bytes_before"] = object_store_bytes(modules)
        content = REPOSITORY / "content"
        had_checkout = is_link(content) or content.is_file() or (content.is_dir() and any(content.iterdir()))
        try:
            verify_checkout(REPOSITORY, arguments.revision)
            verify_object_store(REPOSITORY, arguments.revision)
            report["cache_verified"] = True
            report["cache_store_verified"] = True
        except ContentError as error:
            if arguments.verify_only:
                raise
            report["cache_error"] = str(error)
            hydration_started = time.monotonic()
            try:
                try:
                    verify_object_store(REPOSITORY, arguments.revision)
                    hydrate_object_store(REPOSITORY, arguments.revision, report)
                    report["cache_verified"] = not had_checkout
                    report["repaired_cache"] = had_checkout
                except ContentError as store_error:
                    report["cache_store_error"] = str(store_error)
                    report["repaired_cache"] = had_checkout or modules.exists() or is_link(modules)
                    materialize(REPOSITORY, arguments.revision, report)
            finally:
                report["hydration_elapsed_seconds"] = time.monotonic() - hydration_started
        report["content_revision"] = arguments.revision
        report["checkout_clean"] = True
    except (ContentError, OSError, subprocess.TimeoutExpired) as error:
        report["error"] = str(error)
        print(f"error: {error}", file=sys.stderr)
        status = 1
    if modules is not None:
        report["object_store_bytes_after"] = object_store_bytes(modules)
    report["total_elapsed_seconds"] = time.monotonic() - started
    write_report(report, arguments.report)
    return status


if __name__ == "__main__":
    raise SystemExit(main())
