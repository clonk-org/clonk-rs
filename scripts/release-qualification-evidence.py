#!/usr/bin/env python3
"""Record exact release qualification provenance or resolve verified reuse.

Only immutable artifacts of a fully successful, same-repository Landing
merge-group run can be reused. Missing, stale or unverifiable evidence is a
normal cold fallback; it never substitutes a success assumption for a gate.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import tomllib
import zipfile
import zlib


SCHEMA_VERSION = 1
WORKFLOW = ".github/workflows/landing.yml"
WRITER = "Retain release qualification evidence"
GATE = "Landing gate"
OBSERVABILITY = "CI latency report"
RECEIPT_FILENAME = "release-qualification-evidence.json"
MAX_ARCHIVE_BYTES = 8 * 1024 * 1024
MAX_RECEIPT_BYTES = 4 * 1024 * 1024
MAX_RUN_ATTEMPTS = 100
SHA = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
LINUX = (
    "presentation captures", "app 1/12", "app 12/12", "app 3+10/12", "app 2+7/12",
    "app 4+9/12", "app 5/12", "app 11/12", "app 6/12", "app 8/12",
    "engine integration 1/3", "engine integration 2/3", "engine integration 3/3",
    "engine and frontend unit and parity", "remaining workspace 1/2",
    "remaining workspace 2/2", "workspace quality", "engine contracts",
)
COVERAGE = (
    ("app 1+10/12", "app-1-10"), ("app 2+7/12", "app-2-7"), ("app 3/12", "app-3"),
    ("app 4+9/12", "app-4-9"), ("app 5/12", "app-5"), ("app 11+12/12", "app-11-12"),
    ("app 6+8/12", "app-6-8"), ("engine integration 1/3", "engine-1"),
    ("engine integration 2+3/3", "engine-2-3"),
    ("engine and frontend units", "engine-and-frontend-units"),
    ("remaining workspace 1/2", "remaining-1"), ("remaining workspace 2/2", "remaining-2"),
)
PLATFORMS = ("linux", "windows", "macos")
PREBUILDS = (
    "tool-linux", "tool-windows", "tool-macos", "runtime-linux", "runtime-windows",
    "runtime-macos-arm64", "runtime-macos-x86_64",
)
PUBLIC_ARTIFACTS = (
    "desktop-linux", "desktop-windows", "desktop-macos", "components-desktop-linux",
    "components-desktop-windows", "components-desktop-macos", "release-tool",
)


class EvidenceError(Exception):
    """Qualification evidence is missing, unsuccessful or untrusted."""


def execute(arguments: list[str], root: Path | None = None) -> bytes:
    try:
        completed = subprocess.run(
            arguments, cwd=root, check=False, capture_output=True, timeout=30,
            env={**os.environ, "GIT_NO_LAZY_FETCH": "1"},
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise EvidenceError(f"cannot run {arguments[0]}: {error}") from error
    if completed.returncode:
        raise EvidenceError(f"{arguments[0]} failed: {completed.stderr.decode(errors='replace').strip()}")
    return completed.stdout


def git(root: Path, *arguments: str) -> bytes:
    return execute(["git", "-C", str(root), *arguments])


def source_identity(root: Path, source_sha: str) -> dict:
    root = Path(os.path.abspath(root))
    if root.resolve() != root or not root.is_dir():
        raise EvidenceError("source root is not a real lexical directory")
    if git(root, "rev-parse", "--show-toplevel").decode().strip() != str(root):
        raise EvidenceError("source root is not the checkout's Git root")
    if git(root, "rev-parse", "HEAD").decode().strip() != source_sha:
        raise EvidenceError("checkout does not match the exact source SHA")
    paths = (".", ":(exclude)content")
    if (git(root, "diff", "--name-only", "HEAD", "--", *paths).strip()
            or git(root, "diff", "--cached", "--name-only", "HEAD", "--", "content").strip()
            or git(root, "ls-files", "--others", "--exclude-standard", "--", *paths).strip()):
        raise EvidenceError("qualification source has modified or untracked inputs")
    content = root / "content"
    content_sha = git(root, "rev-parse", "HEAD:content").decode().strip()
    content_entry = git(root, "ls-tree", "HEAD", "--", "content").decode().split()
    if (not SHA.fullmatch(content_sha) or len(content_entry) != 4
            or content_entry[:2] != ["160000", "commit"]):
        raise EvidenceError("qualification source has no exact content gitlink")
    if content.is_symlink() or not content.is_dir():
        raise EvidenceError("content must be a real materialized checkout")
    if (git(content, "rev-parse", "--show-toplevel").decode().strip() != str(content)
            or git(content, "rev-parse", "HEAD").decode().strip() != content_sha
            or git(content, "status", "--porcelain=v1", "--untracked-files=all").strip()
            or git(content, "ls-files", "--others", "-z").strip()):
        raise EvidenceError("materialized content differs from the pinned clean input")
    manifest = root / "Cargo.toml"
    if manifest.is_symlink() or manifest.parent.resolve() != manifest.parent:
        raise EvidenceError("workspace manifest is symlinked")
    version = tomllib.loads(manifest.read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    if not isinstance(version, str) or not re.fullmatch(r"[0-9A-Za-z.+_-]+", version):
        raise EvidenceError("workspace version is invalid")
    # This committed full-tree manifest covers every workflow, script, golden,
    # build recipe and source file, including the exact content gitlink. The
    # clean checkout and materialized-content checks above bind actual inputs.
    recipe = git(root, "ls-tree", "--full-tree", "-r", "-z", "HEAD")
    return {
        "sha": source_sha, "tree_sha": git(root, "rev-parse", "HEAD^{tree}").decode().strip(),
        "content_sha": content_sha, "version": version,
        "recipe_sha256": hashlib.sha256(recipe).hexdigest(),
    }


def api(endpoint: str) -> bytes:
    return execute(["gh", "api", "--method", "GET", endpoint,
                    "-H", "Accept: application/vnd.github+json",
                    "-H", "X-GitHub-Api-Version: 2022-11-28"])


def api_json(endpoint: str) -> dict:
    result = json.loads(api(endpoint))
    if not isinstance(result, dict):
        raise EvidenceError("GitHub API response is not an object")
    return result


def inventory(endpoint: str, field: str) -> list[dict]:
    """Read every page and reject an incomplete or unstable API inventory."""
    values = []
    total = None
    for page in range(1, 101):
        separator = "&" if "?" in endpoint else "?"
        response = api_json(f"{endpoint}{separator}per_page=100&page={page}")
        count = response.get("total_count")
        entries = response.get(field)
        if (type(count) is not int or count < 0 or not isinstance(entries, list)
                or len(entries) > 100 or not all(isinstance(entry, dict) for entry in entries)):
            raise EvidenceError(f"invalid {field} API inventory")
        if total is not None and count != total:
            raise EvidenceError(f"{field} API inventory changed during pagination")
        total = count
        values.extend(entries)
        if len(values) == total:
            if len({entry.get("id") for entry in values}) != len(values):
                raise EvidenceError(f"duplicate {field} IDs")
            return values
        if not entries or len(values) > total:
            raise EvidenceError(f"incomplete {field} API inventory")
    raise EvidenceError(f"{field} API inventory exceeds the bounded page limit")


def positive(value) -> bool:
    return type(value) is int and value > 0


def trusted_run(run: dict, repository: str, source: dict, completed: bool) -> dict:
    own = run.get("repository", {})
    head = run.get("head_repository", {})
    if (not positive(run.get("id")) or not positive(run.get("run_attempt"))
            or not positive(run.get("workflow_id")) or run.get("name") != "Landing"
            or not isinstance(run.get("path"), str) or run["path"].split("@", 1)[0] != WORKFLOW
            or run.get("event") != "merge_group" or run.get("head_sha") != source["sha"]
            or not isinstance(own, dict) or not isinstance(head, dict)
            or not isinstance(own.get("full_name"), str) or not isinstance(head.get("full_name"), str)
            or own["full_name"].casefold() != repository.casefold()
            or head["full_name"].casefold() != repository.casefold()
            or not positive(own.get("id")) or own.get("id") != head.get("id")):
        raise EvidenceError("run is not the trusted exact-SHA Landing merge group")
    if completed and (run.get("status") != "completed" or run.get("conclusion") != "success"):
        raise EvidenceError("whole Landing run has not completed successfully")
    if not completed and run.get("status") not in ("queued", "in_progress", "completed"):
        raise EvidenceError("Landing run has an invalid status")
    return {
        "run_id": run["id"], "run_attempt": run["run_attempt"], "workflow_id": run["workflow_id"],
        "repository_id": own["id"], "workflow_path": WORKFLOW, "event": "merge_group",
    }


def run_jobs(repository: str, builder: dict, source_sha: str) -> list[dict]:
    """Bind optional job attempt fields to authoritative execution membership."""
    if builder["run_attempt"] > MAX_RUN_ATTEMPTS:
        raise EvidenceError("Landing run exceeds the bounded attempt limit")
    endpoint = f"repos/{repository}/actions/runs/{builder['run_id']}"
    jobs = inventory(f"{endpoint}/jobs?filter=all", "jobs")

    def check_identity(job: dict) -> None:
        if (not positive(job.get("id")) or not positive(job.get("run_id"))
                or job["run_id"] != builder["run_id"]
                or job.get("head_sha") != source_sha):
            raise EvidenceError("job belongs to another run or source")
        attempt = job.get("run_attempt")
        if attempt is not None and (not positive(attempt) or attempt > builder["run_attempt"]):
            raise EvidenceError("job run attempt is invalid")

    for job in jobs:
        check_identity(job)
    membership = {}
    if builder["run_attempt"] > 1 and any(job.get("run_attempt") is None for job in jobs):
        for attempt in range(1, builder["run_attempt"] + 1):
            for job in inventory(f"{endpoint}/attempts/{attempt}/jobs", "jobs"):
                check_identity(job)
                if job.get("run_attempt") not in (None, attempt):
                    raise EvidenceError("job attempt disagrees with its authoritative attempt endpoint")
                if job["id"] in membership:
                    raise EvidenceError("job appears in multiple Landing attempts")
                membership[job["id"]] = attempt
        if set(membership) != {job["id"] for job in jobs}:
            raise EvidenceError("all-attempt jobs disagree with the complete attempt inventories")
    for job in jobs:
        attempt = job.get("run_attempt")
        if attempt is None:
            attempt = membership.get(job["id"], 1 if builder["run_attempt"] == 1 else None)
        if not positive(attempt) or attempt > builder["run_attempt"]:
            raise EvidenceError("job has no authoritative Landing attempt membership")
        if membership and membership.get(job["id"]) != attempt:
            raise EvidenceError("job attempt disagrees with its authoritative attempt membership")
        job["run_attempt"] = attempt
    return jobs


def required_jobs(jobs: list[dict], builder: dict, source_sha: str, completed: bool) -> list[dict]:
    latest = {}
    for job in jobs:
        name = job.get("name")
        if (not isinstance(name, str) or not name or not positive(job.get("id"))
                or job.get("run_id") != builder["run_id"] or job.get("head_sha") != source_sha
                or not positive(job.get("run_attempt")) or job["run_attempt"] > builder["run_attempt"]):
            raise EvidenceError("job identity or run attempt is invalid")
        previous = latest.get(name)
        if previous and previous["run_attempt"] == job["run_attempt"]:
            raise EvidenceError(f"ambiguous duplicate job execution: {name}")
        if previous is None or previous["run_attempt"] < job["run_attempt"]:
            latest[name] = job
    for name, job in latest.items():
        if name == OBSERVABILITY or (name in (GATE, WRITER) and not completed):
            continue
        if job.get("status") != "completed" or job.get("conclusion") not in ("success", "skipped"):
            raise EvidenceError(f"job is incomplete or unsuccessful: {name}")
    if completed:
        for control in (GATE, WRITER):
            if latest.get(control, {}).get("conclusion") != "success":
                raise EvidenceError(f"required control job did not succeed: {control}")
    suffixes = {f"Linux / {name}": 1 for name in LINUX}
    suffixes.update({f"Windows / {name}": 1 for name in ("runtime and quality", "network tests")})
    suffixes.update({f"Rust coverage / {name}": 1 for name, _ in COVERAGE})
    suffixes.update({name: 1 for name in (
        "Rust code coverage", "Recording-host material-order oracles (macOS)",
        "Platform lints / macOS", "Platform lints / Windows",
    )})
    suffixes.update({f"Prebuild packaging tool / {platform}": 1 for platform in PLATFORMS})
    suffixes.update({f"Prebuild runtime / {platform}": 1 for platform in (
        "linux", "windows", "macos-arm64", "macos-x86_64",
    )})
    suffixes.update({f"Package {platform}": 1 for platform in PLATFORMS})
    suffixes["Validate release pull request"] = 3
    selected = []
    for suffix, count in suffixes.items():
        matches = [job for name, job in latest.items() if name == suffix or name.endswith(" / " + suffix)]
        if len(matches) != count:
            raise EvidenceError(f"required job inventory differs: {suffix} (expected {count}, found {len(matches)})")
        for job in matches:
            if job.get("status") != "completed" or job.get("conclusion") != "success":
                raise EvidenceError(f"required job did not succeed: {job['name']}")
            if suffix.startswith("Package "):
                steps = job.get("steps")
                if not isinstance(steps, list) or not all(isinstance(step, dict) for step in steps):
                    raise EvidenceError(f"packaged GPU qualification has no valid step inventory: {job['name']}")
                gpu = [step for step in steps if step.get("name") in (
                    "Qualify the packaged runtime", "Qualify every desktop backend",
                )]
                if not gpu or any(step.get("status") != "completed" or step.get("conclusion") != "success"
                                  for step in gpu):
                    raise EvidenceError(f"packaged GPU qualification did not succeed: {job['name']}")
            selected.append({key: job[key] for key in (
                "id", "name", "run_id", "run_attempt", "head_sha", "status", "conclusion",
                "started_at", "completed_at",
            )})
    return sorted(selected, key=lambda job: job["name"])


def artifact_names(source_sha: str, run_id: int) -> set[str]:
    return {
        *PUBLIC_ARTIFACTS,
        *(f"release-prebuild-{kind}-{source_sha}-{run_id}" for kind in PREBUILDS),
        *(f"release-qualified-runtime-{platform}-{source_sha}-{run_id}" for platform in PLATFORMS),
        *(f"rust-coverage-fragment-{run_id}-{artifact}" for _, artifact in COVERAGE),
        *(f"device-loss-{platform}-{source_sha}" for platform in ("Linux", "Windows", "macOS")),
    }


def checked_artifacts(artifacts: list[dict], builder: dict, source_sha: str) -> list[dict]:
    checked = []
    names = set()
    for artifact in artifacts:
        name = artifact.get("name")
        owner = artifact.get("workflow_run", {})
        expiry = artifact.get("expires_at")
        if not isinstance(expiry, str):
            raise EvidenceError(f"artifact expiry is missing or invalid: {name}")
        expires = datetime.fromisoformat(expiry.replace("Z", "+00:00"))
        if (not isinstance(name, str) or not name or name in names or not positive(artifact.get("id"))
                or not positive(artifact.get("size_in_bytes")) or artifact.get("expired") is not False
                or not isinstance(artifact.get("digest"), str) or not DIGEST.fullmatch(artifact["digest"])
                or expires.tzinfo is None or expires <= datetime.now(timezone.utc)
                or not isinstance(owner, dict) or owner.get("id") != builder["run_id"]
                or owner.get("head_sha") != source_sha
                or owner.get("repository_id") != builder["repository_id"]
                or owner.get("head_repository_id") != builder["repository_id"]):
            raise EvidenceError(f"artifact identity, digest or expiry is invalid: {name}")
        names.add(name)
        if name == f"release-qualification-evidence-{source_sha}":
            continue  # A receipt cannot include its own future upload digest.
        checked.append({key: artifact[key] for key in (
            "id", "name", "digest", "size_in_bytes", "expired", "created_at", "updated_at",
            "expires_at", "workflow_run",
        )})
    missing = artifact_names(source_sha, builder["run_id"]) - names
    if missing:
        raise EvidenceError("required artifacts are missing: " + ", ".join(sorted(missing)))
    return sorted(checked, key=lambda artifact: artifact["name"])


def atomic_json(path: Path, document: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix="." + path.name + ".", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(document, handle, indent=2, sort_keys=True)
            handle.write("\n")
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def write(repository: str, run_id: int, source: dict) -> dict:
    run = api_json(f"repos/{repository}/actions/runs/{run_id}")
    builder = trusted_run(run, repository, source, completed=False)
    if builder["run_id"] != run_id:
        raise EvidenceError("producer run ID differs from the requested run")
    if os.environ.get("GITHUB_ACTIONS") == "true":
        expected = {
            "GITHUB_REPOSITORY": repository, "GITHUB_RUN_ID": str(run_id),
            "GITHUB_RUN_ATTEMPT": str(builder["run_attempt"]), "GITHUB_SHA": source["sha"],
            "GITHUB_WORKFLOW": "Landing", "GITHUB_EVENT_NAME": "merge_group",
        }
        for name, value in expected.items():
            if os.environ.get(name) != value:
                raise EvidenceError(f"actual producer environment differs from the API identity: {name}")
    jobs = required_jobs(run_jobs(repository, builder, source["sha"]),
                         builder, source["sha"], completed=False)
    artifacts = checked_artifacts(inventory(f"repos/{repository}/actions/runs/{run_id}/artifacts", "artifacts"),
                                  builder, source["sha"])
    return {"schema_version": SCHEMA_VERSION, "repository": repository,
            "source": source, "builder": builder, "jobs": jobs, "artifacts": artifacts}


def receipt_archive(repository: str, artifact: dict) -> dict:
    if artifact["size_in_bytes"] > MAX_ARCHIVE_BYTES:
        raise EvidenceError("qualification receipt archive is too large")
    payload = api(f"repos/{repository}/actions/artifacts/{artifact['id']}/zip")
    if (len(payload) != artifact["size_in_bytes"] or len(payload) > MAX_ARCHIVE_BYTES
            or "sha256:" + hashlib.sha256(payload).hexdigest() != artifact["digest"]):
        raise EvidenceError("qualification receipt archive differs from its immutable digest")
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        entries = archive.infolist()
        if len(entries) != 1:
            raise EvidenceError("qualification receipt archive must contain exactly one file")
        entry = entries[0]
        mode = stat.S_IFMT(entry.external_attr >> 16)
        if (entry.filename != RECEIPT_FILENAME or entry.is_dir()
                or mode not in (0, stat.S_IFREG) or entry.file_size > MAX_RECEIPT_BYTES
                or entry.flag_bits & 1 or entry.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)):
            raise EvidenceError("qualification receipt archive contains an unsafe or oversized member")
        # Never extract an archive path into the source or runner filesystem.
        try:
            with archive.open(entry) as member:
                document = json.loads(member.read(MAX_RECEIPT_BYTES + 1))
        except zlib.error as error:
            raise EvidenceError("qualification receipt archive compression is invalid") from error
        if not isinstance(document, dict):
            raise EvidenceError("qualification receipt is not an object")
        return document


def validate_receipt(receipt: dict, repository: str, source: dict, builder: dict,
                     jobs: list[dict], artifacts: list[dict]) -> None:
    if (type(receipt.get("schema_version")) is not int or receipt["schema_version"] != SCHEMA_VERSION
            or not isinstance(receipt.get("repository"), str)
            or receipt["repository"].casefold() != repository.casefold()
            or receipt.get("source") != source):
        raise EvidenceError("qualification receipt schema, source or recipe does not match")
    recorded_builder = receipt.get("builder")
    if (not isinstance(recorded_builder, dict) or set(recorded_builder) != set(builder)
            or not positive(recorded_builder.get("run_attempt"))
            or recorded_builder["run_attempt"] > builder["run_attempt"]
            or any(value != recorded_builder.get(key) for key, value in builder.items() if key != "run_attempt")):
        raise EvidenceError("qualification receipt producer does not match the completed Landing run")
    if (receipt.get("jobs") != jobs
            or any(job["run_attempt"] > recorded_builder["run_attempt"] for job in jobs)):
        raise EvidenceError("qualification receipt jobs differ from the successful API executions")
    recorded = receipt.get("artifacts")
    if not isinstance(recorded, list) or not all(isinstance(item, dict) for item in recorded):
        raise EvidenceError("qualification receipt has no complete artifact inventory")
    checked = checked_artifacts(recorded, builder, source["sha"])
    current = {artifact["name"]: artifact for artifact in artifacts}
    for artifact in checked:
        if current.get(artifact["name"]) != artifact:
            raise EvidenceError(f"qualification artifact changed or disappeared: {artifact['name']}")
    recorded_names = {artifact["name"] for artifact in checked}
    for name in current.keys() - recorded_names:
        # The observability job runs after the writer/gate and may produce this
        # explicitly unrelated report. It cannot supply qualification evidence.
        match = re.fullmatch(rf"ci-latency-{builder['run_id']}-([1-9][0-9]*)", name)
        if not match or int(match[1]) > builder["run_attempt"]:
            raise EvidenceError(f"artifact inventory gained an unrecorded artifact: {name}")


def resolve(repository: str, source: dict, hinted_run_id: int | None = None) -> dict:
    if hinted_run_id is not None:
        if not positive(hinted_run_id):
            raise EvidenceError("resolve run ID hint must be positive")
        candidates = [api_json(f"repos/{repository}/actions/runs/{hinted_run_id}")]
    else:
        candidates = inventory(
            f"repos/{repository}/actions/workflows/landing.yml/runs"
            f"?head_sha={source['sha']}&event=merge_group&status=success", "workflow_runs",
        )
    reasons = []
    for candidate in sorted(candidates, key=lambda run: run.get("id", 0), reverse=True):
        try:
            candidate_builder = trusted_run(candidate, repository, source, completed=True)
            run_id = candidate_builder["run_id"]
            if hinted_run_id is not None and run_id != hinted_run_id:
                raise EvidenceError("authoritative Landing run differs from the requested hint")
            run = candidate if hinted_run_id is not None else api_json(f"repos/{repository}/actions/runs/{run_id}")
            builder = trusted_run(run, repository, source, completed=True)
            if builder != candidate_builder:
                raise EvidenceError("Landing producer identity changed during resolution")
            jobs = required_jobs(run_jobs(repository, builder, source["sha"]),
                                 builder, source["sha"], completed=True)
            raw_artifacts = inventory(f"repos/{repository}/actions/runs/{run_id}/artifacts", "artifacts")
            artifacts = checked_artifacts(raw_artifacts, builder, source["sha"])
            evidence = [artifact for artifact in raw_artifacts
                        if artifact.get("name") == f"release-qualification-evidence-{source['sha']}"]
            if len(evidence) != 1:
                raise EvidenceError("complete qualification receipt artifact is missing or ambiguous")
            receipt = receipt_archive(repository, evidence[0])
            validate_receipt(receipt, repository, source, builder, jobs, artifacts)
            return {"schema_version": SCHEMA_VERSION, "reused": True,
                    "run_id": run_id, "run_attempt": receipt["builder"]["run_attempt"], "receipt": receipt,
                    "source_sha": source["sha"], "reason": "complete exact-SHA qualification verified"}
        except (EvidenceError, OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
            reasons.append(str(error))
    reason = reasons[0] if reasons else "no successful exact-SHA Landing merge group found"
    return {"schema_version": SCHEMA_VERSION, "reused": False, "source_sha": source["sha"], "reason": reason}


def github_outputs(result: dict, destination: Path | None) -> None:
    values = (f"reused={'true' if result['reused'] else 'false'}\n"
              f"run-id={result.get('run_id', '')}\nrun-attempt={result.get('run_attempt', '')}\n")
    print(values, end="")
    if destination:
        with destination.open("a", encoding="utf-8") as handle:
            handle.write(values)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("write", "resolve"))
    parser.add_argument("--repository", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--run-id", type=int)
    parser.add_argument("--root", default=Path.cwd(), type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--github-output", type=Path)
    arguments = parser.parse_args(argv)
    try:
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", arguments.repository):
            raise EvidenceError("repository must be an owner/repository name")
        if not SHA.fullmatch(arguments.source_sha):
            raise EvidenceError("source SHA must be a complete lowercase Git object ID")
        source = source_identity(arguments.root, arguments.source_sha)
        if arguments.operation == "resolve":
            result = resolve(arguments.repository, source, arguments.run_id)
        else:
            if not positive(arguments.run_id):
                raise EvidenceError("write requires a positive producer run ID")
            receipt = write(arguments.repository, arguments.run_id, source)
            atomic_json(arguments.output, receipt)
            print(json.dumps({"written": True, "run_id": arguments.run_id,
                              "jobs": len(receipt["jobs"]), "artifacts": len(receipt["artifacts"])}))
            return 0
    except (EvidenceError, OSError, ValueError, KeyError, TypeError) as error:
        if arguments.operation == "write":
            print(f"qualification evidence refused: {error}", file=sys.stderr)
            return 1
        result = {"schema_version": SCHEMA_VERSION, "reused": False,
                  "source_sha": arguments.source_sha, "reason": str(error)}
    try:
        atomic_json(arguments.output, result)
        github_outputs(result, arguments.github_output)
        if not result["reused"]:
            print(f"qualification reuse fallback: {result['reason']}", file=sys.stderr)
        return 0
    except OSError as error:
        print(f"cannot record qualification resolution: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
