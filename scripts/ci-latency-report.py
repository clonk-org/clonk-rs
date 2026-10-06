#!/usr/bin/env python3
"""Report Actions correctness and latency separately, without inventing queue evidence."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import quote, urlencode


class ReportError(Exception):
    """GitHub did not provide complete, usable timing evidence."""


class APIUnavailable(ReportError):
    """An operation anchor could not be fetched; it cannot supply a sample."""


STEP_CLASSES = {
    "upload": r"upload|publish.*artifact", "download": r"download|fetch.*artifact",
    "export": r"export|manifest|collect.*(?:coverage|artifact)|lcov|html report",
    "package": r"package|bundle|lipo|sign.*(?:binary|package)",
    "setup": r"set up|setup|checkout|install|toolchain|hydrate|materialize|restore|cache",
    "test": r"test|nextest|clippy|lint|parity|oracle|snapshot|qualif|verify|validation|format|compat",
    "build": r"build|compile|cargo check", "other": r"",
}
SEMVER = (r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
          r"(?:-(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
          r"(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?"
          r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?")
RELEASE_SUBJECT = r"chore: release (" + SEMVER + r")(?: \(#([1-9][0-9]*)\))?"
OPTIONAL_POSTMERGE_JOB = "Bound trusted workspace caches"


def api(endpoint: str) -> dict | list:
    try:
        result = subprocess.run(
            ["gh", "api", "--method", "GET", endpoint, "-H", "Accept: application/vnd.github+json",
             "-H", "X-GitHub-Api-Version: 2022-11-28"],
            capture_output=True, text=True, timeout=30, check=False,
        )
        if result.returncode:
            raise APIUnavailable(f"gh api {endpoint} failed: {result.stderr.strip()}")
        document = json.loads(result.stdout)
        if not isinstance(document, (dict, list)):
            raise ReportError(f"gh api {endpoint} returned an invalid response")
        return document
    except (OSError, subprocess.TimeoutExpired) as error:
        raise APIUnavailable(f"gh api {endpoint} failed: {error}") from error
    except json.JSONDecodeError as error:
        raise ReportError(f"gh api {endpoint} returned invalid JSON: {error}") from error


def timestamp(value: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            raise ValueError("timestamp has no timezone")
        return parsed.astimezone(timezone.utc)
    except (AttributeError, TypeError, ValueError) as error:
        raise ReportError(f"invalid GitHub timestamp: {value!r}") from error


def optional_timestamp(record: dict, field: str) -> datetime | None:
    return timestamp(record[field]) if record.get(field) is not None else None


def pages(endpoint: str, collection: str, limit: int | None = None) -> tuple[list[dict], int]:
    items, total, number = [], None, 1
    page_size = min(limit or 100, 100)
    while total is None or len(items) < min(total, limit or total):
        separator = "&" if "?" in endpoint else "?"
        document = api(f"{endpoint}{separator}per_page={page_size}&page={number}")
        count, batch = document["total_count"], document[collection]
        if type(count) is not int or count < 0 or not isinstance(batch, list):
            raise ReportError("malformed GitHub pagination")
        if total is not None and count != total:
            raise ReportError("GitHub pagination changed during collection")
        total = count
        if not batch and len(items) < min(total, limit or total):
            raise ReportError("GitHub pagination ended before total_count")
        if any(not isinstance(item, dict) or type(item.get("id")) is not int or item["id"] <= 0 for item in batch):
            raise ReportError("GitHub page contains a malformed entry")
        identifiers = [item["id"] for item in items + batch]
        if len(set(identifiers)) != len(identifiers) or len(identifiers) > total:
            raise ReportError("GitHub pagination duplicated entries or exceeded total_count")
        items.extend(batch)
        number += 1
    return items[:limit] if limit else items, total


def operation_api(endpoint: str, cache: dict) -> dict | list:
    # One collection shares an API snapshot across current and historical
    # operations, without storing timing or qualification results in a cache.
    if endpoint not in cache:
        try:
            cache[endpoint] = api(endpoint)
        except APIUnavailable as error:
            cache[endpoint] = error
    document = cache[endpoint]
    if isinstance(document, APIUnavailable):
        raise document
    return document


def array_pages(endpoint: str, cache: dict) -> list[dict]:
    items, seen = [], set()
    for number in range(1, 101):
        separator = "&" if "?" in endpoint else "?"
        batch = operation_api(f"{endpoint}{separator}per_page=100&page={number}", cache)
        if (not isinstance(batch, list) or len(batch) > 100
                or any(not isinstance(item, dict) or type(item.get("id")) is not int or item["id"] <= 0
                       for item in batch)):
            raise ReportError("malformed pull-request pagination")
        for item in batch:
            if item["id"] in seen:
                raise ReportError("pull-request pagination duplicated entries")
            seen.add(item["id"])
        items.extend(batch)
        if len(batch) < 100:
            return items
    raise ReportError("pull-request pagination exceeds the complete bounded inventory")


def same_repository_pull(pull: dict, repository: str) -> bool:
    for side in ("head", "base"):
        metadata = pull.get(side)
        if not isinstance(metadata, dict):
            raise ReportError("pull request has malformed repository metadata")
        owner = metadata.get("repo")
        if not isinstance(owner, dict) or owner.get("full_name", "").casefold() != repository.casefold():
            return False
    return pull["base"].get("ref") == "main"


def pull_identity(pull: dict, repository: str) -> dict:
    if (type(pull.get("number")) is not int or pull["number"] < 1
            or pull.get("html_url") != f"https://github.com/{repository}/pull/{pull['number']}"
            or pull.get("state") not in ("open", "closed")):
        raise ReportError("pull request has invalid identity or state")
    created = timestamp(pull["created_at"])
    merged = optional_timestamp(pull, "merged_at")
    if merged is not None and (merged < created or pull["state"] != "closed"):
        raise ReportError("pull request merge timestamps or state are out of order")
    return {key: pull.get(key) for key in ("id", "number", "html_url", "created_at", "merged_at", "merge_commit_sha")}


def unknown_operation(clock: str | None, reason: str, detail: str | None = None) -> dict:
    return {"clock": clock, "status": "UNKNOWN", "seconds": None, "start_at": None, "end_at": None,
            "exclusion_reason": reason, "detail": detail}


def tag_source(repository: str, tag: str, cache: dict) -> str:
    ref = operation_api(f"repos/{repository}/git/ref/tags/{quote(tag, safe='')}", cache)
    if not isinstance(ref, dict) or ref.get("ref") != f"refs/tags/{tag}":
        raise ReportError("release tag ref has an invalid identity")
    target, seen = ref.get("object"), set()
    for _ in range(16):
        if (not isinstance(target, dict) or not isinstance(target.get("sha"), str)
                or not re.fullmatch(r"[0-9a-f]{40}", target["sha"])):
            raise ReportError("release tag target has an invalid object identity")
        if target.get("type") == "commit":
            return target["sha"]
        if target.get("type") != "tag" or target["sha"] in seen:
            raise ReportError("release tag chain is invalid or cyclic")
        seen.add(target["sha"])
        tagged = operation_api(f"repos/{repository}/git/tags/{target['sha']}", cache)
        if not isinstance(tagged, dict) or tagged.get("sha") != target["sha"]:
            raise ReportError("annotated release tag has an invalid identity")
        target = tagged.get("object")
    raise ReportError("release tag chain exceeds its bounded inventory")


def publication_operation(arguments: argparse.Namespace, run: dict, jobs: list[dict], cache: dict) -> dict:
    clock = "pull_request_merged_at_to_release_published_at"
    commit = run.get("head_commit")
    if commit is None:
        return unknown_operation(clock, "actual_head_commit_unavailable")
    if (not isinstance(commit, dict) or commit.get("id") != run["head_sha"]
            or not isinstance(commit.get("message"), str) or not commit["message"]):
        raise ReportError("publication workflow has no valid actual head commit")
    subject = commit["message"].splitlines()[0]
    if not subject.startswith("chore: release"):
        return unknown_operation(clock, "no_release_commit")
    release_subject = re.fullmatch(RELEASE_SUBJECT, subject)
    if not release_subject:
        raise ReportError("actual release commit has an invalid release version")
    version = release_subject.group(1)
    publishing = [step for job in jobs for step in job.get("steps", []) if step["name"] == "Publish the release"]
    if not any(step.get("status") == "completed" and step.get("conclusion") == "success" for step in publishing):
        return unknown_operation(clock, "publication_skipped_or_reused" if publishing else "publication_execution_unavailable")
    try:
        pulls = array_pages(f"repos/{arguments.repository}/commits/{run['head_sha']}/pulls", cache)
        candidates = [pull for pull in pulls if same_repository_pull(pull, arguments.repository)
                      and pull["head"].get("ref") == "release/next"
                      and pull.get("merge_commit_sha") == run["head_sha"] and pull.get("merged_at") is not None]
        if len(candidates) != 1:
            return unknown_operation(clock, "ambiguous_merged_pull_request" if candidates else "no_merged_pull_request")
        pull = candidates[0]
        identity = pull_identity(pull, arguments.repository)
        if release_subject.group(2) is not None and int(release_subject.group(2)) != pull["number"]:
            raise ReportError("release commit squash suffix differs from its unique merged pull request")
        tag = "v" + version
        release = operation_api(f"repos/{arguments.repository}/releases/tags/{quote(tag, safe='')}", cache)
        if (not isinstance(release, dict) or type(release.get("id")) is not int or release["id"] < 1
                or release.get("tag_name") != tag or type(release.get("draft")) is not bool
                or not isinstance(release.get("html_url"), str)):
            raise ReportError("published release has an invalid identity")
        created = timestamp(release["created_at"])
        published = optional_timestamp(release, "published_at")
        if published is not None and published < created:
            raise ReportError("release publication predates release creation")
        if release["draft"] or published is None:
            return unknown_operation(clock, "release_not_published")
        resolved_source = tag_source(arguments.repository, tag, cache)
        if resolved_source != run["head_sha"] or resolved_source != pull["merge_commit_sha"]:
            raise ReportError("published release tag does not match the exact merged source")
        seconds = (published - timestamp(pull["merged_at"])).total_seconds()
        if seconds < 0:
            raise ReportError("release publication predates its source pull request merge")
        if published < timestamp(run["created_at"]):
            return unknown_operation(clock, "release_already_published_before_run")
        return {"clock": clock, "status": "measured", "seconds": seconds,
                "start_at": pull["merged_at"], "end_at": release["published_at"], "exclusion_reason": None,
                "pull_request": identity, "release": {"id": release["id"], "html_url": release["html_url"],
                    "version": version, "tag_name": tag, "tag_target_sha": resolved_source,
                    "created_at": release["created_at"], "published_at": release["published_at"]}}
    except APIUnavailable as error:
        return unknown_operation(clock, "api_unavailable", str(error))


def phase_operation(arguments: argparse.Namespace, run: dict, jobs: list[dict], cache: dict) -> dict:
    if arguments.phase not in ("prepare", "publication"):
        return {"clock": None, "status": "not_applicable", "seconds": None, "exclusion_reason": None}
    if arguments.phase == "publication":
        return publication_operation(arguments, run, jobs, cache)
    clock = "workflow_created_at_to_pull_request_created_at"
    try:
        run_url = f"https://github.com/{arguments.repository}/actions/runs/{run['id']}"
        if run.get("html_url") != run_url:
            raise ReportError("release preparation producer URL is invalid")
        query = urlencode({"state": "all", "head": arguments.repository.split("/")[0] + ":release/next", "base": "main"})
        pulls = array_pages(f"repos/{arguments.repository}/pulls?{query}", cache)
        candidates, reused = [], False
        for pull in pulls:
            if not same_repository_pull(pull, arguments.repository) or pull["head"].get("ref") != "release/next":
                continue
            body = pull.get("body")
            if body is not None and not isinstance(body, str):
                raise ReportError("release pull request has a malformed body")
            urls = re.findall(r"https?://[^\s<>\"'`()\[\]{}]+", body or "")
            if any(url.rstrip(".,;:") == run_url for url in urls):
                candidates.append(pull)
            elif pull.get("state") == "open":
                reused = True
        if len(candidates) != 1:
            reason = "ambiguous_producer_pull_request" if candidates else (
                "reused_pull_request_from_another_run" if reused else "no_pull_request_created_by_run")
            return unknown_operation(clock, reason)
        pull = candidates[0]
        identity = pull_identity(pull, arguments.repository)
        seconds = (timestamp(pull["created_at"]) - timestamp(run["created_at"])).total_seconds()
        if seconds < 0:
            raise ReportError("release pull request creation predates its producer workflow")
        return {"clock": clock, "status": "measured", "seconds": seconds,
                "start_at": run["created_at"], "end_at": pull["created_at"], "exclusion_reason": None,
                "producer_run_url": run_url, "pull_request": identity}
    except APIUnavailable as error:
        return unknown_operation(clock, "api_unavailable", str(error))


def release_source(run: dict) -> bool:
    commit = run.get("head_commit")
    return (isinstance(commit, dict) and commit.get("id") == run["head_sha"]
            and isinstance(commit.get("message"), str) and bool(commit["message"])
            and re.fullmatch(RELEASE_SUBJECT, commit["message"].splitlines()[0]) is not None)


def run_jobs(prefix: str, run: dict) -> list[dict]:
    if any(type(run.get(field)) is not int or run[field] < 1 for field in ("id", "workflow_id", "run_attempt")):
        raise ReportError("GitHub run identity or attempt is invalid")
    if run["status"] not in ("completed", "queued", "in_progress", "waiting", "pending", "requested"):
        raise ReportError("GitHub workflow status is unknown")
    if not re.fullmatch(r"[0-9a-f]{40}", run["head_sha"]) or not isinstance(run["event"], str) or not run["event"]:
        raise ReportError("GitHub workflow source or event is invalid")
    endpoint = f"{prefix}/runs/{run['id']}"
    jobs, _ = pages(f"{endpoint}/jobs?filter=all", "jobs")
    membership = {}
    if run["run_attempt"] > 1 and any(job.get("run_attempt") is None for job in jobs):
        for attempt in range(1, run["run_attempt"] + 1):
            attempt_jobs, _ = pages(f"{endpoint}/attempts/{attempt}/jobs", "jobs")
            for job in attempt_jobs:
                if job["id"] in membership and membership[job["id"]] != attempt:
                    raise ReportError("job appears in multiple workflow attempts")
                membership[job["id"]] = attempt
    for job in jobs:
        if job.get("run_id", run["id"]) != run["id"] or job.get("head_sha", run["head_sha"]) != run["head_sha"]:
            raise ReportError("GitHub job belongs to another workflow run or source")
        attempt = job.get("run_attempt")
        if attempt is None:
            attempt = membership.get(job["id"], 1 if run["run_attempt"] == 1 else None)
        if type(attempt) is not int or not 1 <= attempt <= run["run_attempt"]:
            raise ReportError("GitHub job has no valid workflow attempt")
        if membership and membership.get(job["id"]) != attempt:
            raise ReportError("GitHub all-attempt job inventory disagrees with attempt inventories")
        job["run_attempt"] = attempt
    return jobs


def conclusion(value: str | None) -> str:
    return {
        "success": "success", "failure": "failure", "timed_out": "failure",
        "cancelled": "cancelled", "startup_failure": "error", "action_required": "error",
    }.get(value, "unknown")


def functional(run: dict, jobs: list[dict]) -> str:
    result = conclusion(run["conclusion"])
    if result not in ("success", "unknown"):
        return result
    observed = [conclusion(job.get("conclusion")) for job in jobs]
    for problem in ("failure", "error", "cancelled"):
        if problem in observed:
            return problem
    if result == "success" and "success" in observed and all(
        job["status"] == "completed" and job.get("conclusion") in ("success", "skipped") for job in jobs
    ):
        return "success"
    return "unknown"


def active_seconds(intervals: list[tuple[datetime, datetime]]) -> float:
    merged, total = None, 0.0
    for start, end in sorted(intervals):
        if merged is not None and start <= merged[1]:
            merged = (merged[0], max(merged[1], end))
        else:
            if merged is not None:
                total += (merged[1] - merged[0]).total_seconds()
            merged = (start, end)
    return total + ((merged[1] - merged[0]).total_seconds() if merged else 0)


def peak_runner_concurrency(intervals: list[tuple[datetime, datetime]]) -> int:
    # Jobs occupy half-open execution intervals. Group simultaneous edges so
    # completions release their slots before new jobs start at that timestamp.
    # A zero-duration interval supplies no measured runner occupancy.
    boundaries = {}
    for start, end in intervals:
        if start == end:
            continue
        boundaries.setdefault(start, [0, 0])[1] += 1
        boundaries.setdefault(end, [0, 0])[0] += 1
    active, peak = 0, 0
    for ending, starting in (counts for moment, counts in sorted(boundaries.items())):
        active -= ending
        active += starting
        peak = max(peak, active)
    return peak


def step_timings(job: dict) -> None:
    totals = dict.fromkeys(STEP_CLASSES, 0.0)
    unmeasured, receipts = 0, set()
    for step in job.get("steps", []):
        category = next(name for name, pattern in STEP_CLASSES.items() if re.search(pattern, step["name"], re.I))
        start, end = optional_timestamp(step, "started_at"), optional_timestamp(step, "completed_at")
        if start is not None and end is not None and end < start:
            raise ReportError("GitHub step timestamps are out of order")
        duration = (end - start).total_seconds() if start is not None and end is not None else None
        step.update(classification=category, duration_seconds=duration)
        totals[category] += duration or 0
        unmeasured += duration is None
        receipt = re.fullmatch(r"CI cache receipt: (warm|cold)", step["name"])
        if receipt and step.get("conclusion") == "success" and step.get("status") == "completed":
            receipts.add(receipt.group(1))
    state = next(iter(receipts)) if len(receipts) == 1 else "UNKNOWN"
    job.update(step_seconds=totals, unmeasured_steps=unmeasured, step_classification_source="step_name",
               cache_state=state, cache_evidence="explicit_successful_receipt_step" if state != "UNKNOWN" else None)


def observe(arguments: argparse.Namespace, run: dict, jobs: list[dict], phase_cache: dict | None = None) -> dict:
    for field in ("created_at", "run_started_at", "updated_at"):
        optional_timestamp(run, field)
    if run["status"] != "completed" and arguments.allow_running != "true":
        raise ReportError("workflow is incomplete; use --allow-running for an explicitly partial observation")
    excluded = [job["id"] for job in jobs if re.search(r"latency[ -]report", job["name"], re.I)]
    jobs = [job for job in jobs if job["id"] not in excluded]
    latest = {}
    for job in jobs:
        latest[job["name"]] = max(latest.get(job["name"], 0), job["run_attempt"])
    for job in jobs:
        job["superseded"] = job["run_attempt"] != latest[job["name"]]
    created = timestamp(run["created_at"])
    intervals, queues = [], []
    for job in jobs:
        start, end = optional_timestamp(job, "started_at"), optional_timestamp(job, "completed_at")
        queued = optional_timestamp(job, "created_at")
        if job.get("conclusion") == "skipped":
            # GitHub synthesizes these metadata timestamps without running a
            # job, and can report completed_at before started_at or created_at.
            job.update(duration_seconds=None, queue_seconds=None)
            step_timings(job)
            continue
        if job.get("conclusion") == "success" and (start is None or end is None):
            raise ReportError("successful GitHub job has no complete timestamps")
        if (start is not None and start < created) or (end is not None and start is not None and end < start) or (
            queued is not None and start is not None and queued > start
        ):
            raise ReportError("GitHub job timestamps are out of order")
        if start is not None and end is not None:
            intervals.append((start, end))
        job["duration_seconds"] = (end - start).total_seconds() if start is not None and end is not None else None
        job["queue_seconds"] = (start - queued).total_seconds() if queued is not None and start is not None else None
        if start is not None:
            queues.append(job["queue_seconds"])
        step_timings(job)
    started = min((start for start, _ in intervals), default=None)
    completed = max((end for _, end in intervals), default=None)
    active = active_seconds(intervals)
    elapsed = (completed - created).total_seconds() if completed is not None else None
    execution = (completed - started).total_seconds() if completed is not None and started is not None else None
    release = arguments.release == "true" if arguments.release is not None else arguments.phase in ("prepare", "publication") or release_source(run) or any(
        re.search(r"build release candidate|release[- ](?:prebuild|build|qualification)|qualify release", job["name"], re.I)
        for job in jobs if job.get("conclusion") != "skipped"
    )
    target = 120 if arguments.phase in ("prepare", "publication") else 600
    operation = phase_operation(arguments, run, jobs, phase_cache if phase_cache is not None else {})
    measurement = operation["seconds"] if arguments.phase in ("prepare", "publication") else execution
    current = [job for job in jobs if not job["superseded"]]
    functional_excluded = {
        job["id"] for job in jobs
        if arguments.phase == "postmerge" and job["name"] == OPTIONAL_POSTMERGE_JOB
    }
    required_current = [job for job in current if job["id"] not in functional_excluded]
    complete = bool(current) and all(job["status"] == "completed" for job in current)
    result = functional({**run, "conclusion": "success"} if run["status"] != "completed" and complete
                        and run.get("conclusion") is None else run, required_current)
    cache_states = {job["cache_state"] for job in current if job.get("conclusion") != "skipped"}
    return {
        "schema": 1, "source": {"repository": arguments.repository, "run_id": run["id"],
                                  "workflow_id": run["workflow_id"], "head_sha": run["head_sha"],
                                  "url": run.get("html_url")},
        "event": run["event"], "phase": arguments.phase, "run_attempt": run["run_attempt"],
        "original_created_at": run["created_at"], "functional_conclusion": result,
        "run_status": run["status"], "qualification_complete": complete, "excluded_reporting_job_ids": excluded,
        "functional_excluded_job_ids": sorted(functional_excluded),
        "attempts": [
            {"attempt": attempt, "functional_conclusion": functional({"conclusion": "success"},
              [job for job in jobs if job["run_attempt"] == attempt and job["id"] not in functional_excluded])}
            for attempt in sorted({job["run_attempt"] for job in jobs})
        ],
        "cache_state": next(iter(cache_states)) if len(cache_states) == 1 else "UNKNOWN", "jobs": jobs,
        "operation": operation, "latency": {
            "elapsed_seconds": elapsed, "execution_seconds": execution,
            "operation_seconds": operation["seconds"],
            "initial_queue_seconds": (started - created).total_seconds() if started is not None else None,
            "active_seconds": active, "waiting_seconds": elapsed - active if elapsed is not None else None,
            "runner_active_seconds": sum(job["duration_seconds"] or 0 for job in jobs),
            "peak_runner_concurrency": peak_runner_concurrency(intervals),
            "job_queue_seconds_sum": sum(queues) if all(value is not None for value in queues) else None,
            "dependency_wait_seconds": None, "runner_wait_seconds": None,
            "candidate_started_at": started.isoformat() if started is not None else None,
            "candidate_completed_at": completed.isoformat() if completed is not None else None,
            "slo": {"class": "release" if release else "ordinary", "target_seconds": target,
                    "measurement": "operation_seconds" if arguments.phase in ("prepare", "publication") else "execution_seconds",
                    "met": measurement <= target if complete and measurement is not None else None},
        },
    }


def distribution(reports: list[dict], phase: str) -> dict:
    measurement = "operation_seconds" if phase in ("prepare", "publication") else "elapsed_seconds"
    def exclusion(report: dict) -> str | None:
        if report["functional_conclusion"] != "success":
            return "functional_" + report["functional_conclusion"]
        if not report["qualification_complete"]:
            return "qualification_incomplete"
        if report["latency"][measurement] is None:
            return report["operation"]["exclusion_reason"] or "missing_" + measurement
        return None
    exclusions = [exclusion(report) for report in reports]
    successful = [report for report, reason in zip(reports, exclusions) if reason is None]
    def percentile(field: str, quantile: float) -> float | None:
        values = sorted(report["latency"][field] for report in successful)
        return values[math.ceil(len(values) * quantile) - 1] if values else None
    return {
        "sample_count": len(successful), "completed_runs": len(reports),
        "sample_measurement": measurement, "excluded_sample_count": len(reports) - len(successful),
        "excluded_reasons": {reason: exclusions.count(reason) for reason in sorted(set(exclusions) - {None})},
        "p50_seconds": percentile(measurement, 0.50), "p95_seconds": percentile(measurement, 0.95),
        "workflow_elapsed_p50_seconds": percentile("elapsed_seconds", 0.50),
        "workflow_elapsed_p95_seconds": percentile("elapsed_seconds", 0.95),
        "execution_p50_seconds": percentile("execution_seconds", 0.50),
        "execution_p95_seconds": percentile("execution_seconds", 0.95),
        "conclusions": {name: sum(report["functional_conclusion"] == name for report in reports)
                        for name in ("success", "failure", "cancelled", "error", "unknown")},
        "cache_states": {name: sum(report["cache_state"] == name for report in reports)
                         for name in ("warm", "cold", "UNKNOWN")},
        "runs": [{"run_id": report["source"]["run_id"], "head_sha": report["source"]["head_sha"],
                  "original_created_at": report["original_created_at"], "run_attempt": report["run_attempt"],
                  "functional_conclusion": report["functional_conclusion"], "cache_state": report["cache_state"],
                  "elapsed_seconds": report["latency"]["elapsed_seconds"],
                  "execution_seconds": report["latency"]["execution_seconds"],
                  "operation_seconds": report["latency"]["operation_seconds"], "operation": report["operation"],
                  "sample_eligible": reason is None, "exclusion_reason": reason}
                 for report, reason in zip(reports, exclusions)],
    }


def collect(arguments: argparse.Namespace) -> dict:
    prefix = f"repos/{arguments.repository}/actions"
    run = api(f"{prefix}/runs/{arguments.run_id}")
    if run["id"] != arguments.run_id:
        raise ReportError("GitHub returned a different workflow run")
    jobs = run_jobs(prefix, run)
    phase_cache = {}
    report = observe(arguments, run, jobs, phase_cache)
    historical, available = pages(f"{prefix}/workflows/{run['workflow_id']}/runs?status=completed",
                                  "workflow_runs", arguments.history_limit)
    groups = {"ordinary": [], "release": []}
    historical_arguments = argparse.Namespace(**{**vars(arguments), "release": None, "allow_running": "false"})
    for candidate in historical:
        if candidate["id"] == run["id"] or candidate["event"] != run["event"]:
            continue
        if candidate["workflow_id"] != run["workflow_id"] or candidate["status"] != "completed":
            raise ReportError("GitHub history contains a foreign or incomplete workflow run")
        candidate_jobs = run_jobs(prefix, candidate)
        observation = observe(historical_arguments, candidate, candidate_jobs, phase_cache)
        groups[observation["latency"]["slo"]["class"]].append(observation)
    report["history"] = {
        "limit": arguments.history_limit, "available_runs": available, "inspected_runs": len(historical),
        "truncated": available > len(historical), "percentile_method": "nearest_rank",
        "comparability": ["workflow_id", "event", "phase", "release_class"],
        "percentile_population": ("successful completed runs with validated operation clocks; missing, reused and no-op operations excluded"
                                  if arguments.phase in ("prepare", "publication") else
                                  "successful completed runs; elapsed clock retains original creation across reruns"),
        **{name: distribution(observations, arguments.phase) for name, observations in groups.items()},
    }
    return report


def bounded_integer(value: str, maximum: int) -> int:
    parsed = int(value)
    if not 1 <= parsed <= maximum:
        raise argparse.ArgumentTypeError(f"value must be between 1 and {maximum}")
    return parsed


def write_summary(report: dict, destination: str | Path) -> None:
    latency, slo = report["latency"], report["latency"]["slo"]
    with Path(destination).open("a", encoding="utf-8") as output:
        output.write(
            f"### CI latency ({report['phase']})\n\nFunctional conclusion: **{report['functional_conclusion']}**; "
            f"advisory SLO: **{ {True: 'met', False: 'miss', None: 'unknown'}[slo['met']]}** "
            f"({slo['class']}, {slo['target_seconds']}s). Cache: **{report['cache_state']}**.\n\n"
            "| Clock | Seconds |\n| --- | ---: |\n"
        )
        for label, field in (("Workflow original creation to qualification", "elapsed_seconds"),
                             ("First qualification start to last completion", "execution_seconds"),
                             ("Release operation", "operation_seconds"),
                             ("Initial queue", "initial_queue_seconds"), ("Active wall time", "active_seconds"),
                             ("Waiting or idle wall time", "waiting_seconds"), ("Runner work sum", "runner_active_seconds")):
            output.write(f"| {label} | {latency[field]} |\n")
        output.write(f"\nPeak observed runner concurrency: **{latency['peak_runner_concurrency']}** "
                     "(completed positive-duration job intervals across all attempts; "
                     "reporting and skipped jobs excluded).\n")
        if report["functional_excluded_job_ids"]:
            identifiers = ", ".join(str(identifier) for identifier in report["functional_excluded_job_ids"])
            output.write(f"\nOptional maintenance excluded from functional conclusion: **{identifiers}** "
                         "(job IDs); raw outcomes and runner costs remain below.\n")
        operation = report["operation"]
        if operation["clock"] is not None:
            output.write(f"\nRelease operation clock: `{operation['clock']}`; status **{operation['status']}**. "
                         f"Anchors: `{operation['start_at']}` → `{operation['end_at']}`. "
                         f"Excluded reason: `{operation['exclusion_reason'] or 'none'}`.\n")
        output.write("\n| Class | Clock | Sample count | Completed runs | Excluded | p50 clock | p95 clock | "
                     "p50 workflow | p95 workflow | p50 execution | p95 execution |\n"
                     "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n")
        for name in ("ordinary", "release"):
            values = report["history"][name]
            output.write(f"| {name} | {values['sample_measurement']} | {values['sample_count']} | "
                         f"{values['completed_runs']} | {values['excluded_sample_count']} | {values['p50_seconds']} | "
                         f"{values['p95_seconds']} | {values['workflow_elapsed_p50_seconds']} | "
                         f"{values['workflow_elapsed_p95_seconds']} | {values['execution_p50_seconds']} | "
                         f"{values['execution_p95_seconds']} |\n")
        for name in ("ordinary", "release"):
            reasons = report["history"][name]["excluded_reasons"]
            if reasons:
                output.write(f"\nExcluded {name} samples: " + ", ".join(f"`{reason}`={count}" for reason, count in reasons.items()) + ".\n")
        output.write("\n| Job | Attempt | Functional | Seconds | Cache | " + " | ".join(STEP_CLASSES) + " |\n"
                     "| --- | ---: | --- | ---: | --- |" + " ---: |" * len(STEP_CLASSES) + "\n")
        for job in report["jobs"]:
            name = job["name"].replace("|", r"\|").replace("\n", " ")
            output.write(f"| {name} | {job['run_attempt']} | {job.get('conclusion')} | {job['duration_seconds']} | "
                         f"{job['cache_state']} | " + " | ".join(str(job["step_seconds"][key]) for key in STEP_CLASSES) + " |\n")
        output.write("\nPercentiles use successful completed runs with the named measured clock and nearest rank. "
                     "Release operation anchors use validated PR and release API timestamps; missing, reused and no-op "
                     "operation samples are excluded. Step buckets classify names; runner-vs-dependency wait is unavailable.\n\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--run-id", type=lambda value: bounded_integer(value, sys.maxsize), required=True)
    parser.add_argument("--phase", choices=("landing", "postmerge", "prepare", "publication"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--history-limit", type=lambda value: bounded_integer(value, 100), default=20)
    parser.add_argument("--release", choices=("true", "false"))
    parser.add_argument("--allow-running", nargs="?", const="true", choices=("true", "false"), default="false")
    arguments = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", arguments.repository) or any(
        part in (".", "..") for part in arguments.repository.split("/")
    ):
        parser.error("repository must be an owner/name pair")
    try:
        report = collect(arguments)
    except (ReportError, KeyError, ValueError, TypeError, AttributeError) as error:
        report = {"schema": 1, "source": {"repository": arguments.repository, "run_id": arguments.run_id},
                  "phase": arguments.phase, "functional_conclusion": "error", "error": str(error)}
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if "error" in report:
        print(f"::error::{report['error']}", file=sys.stderr)
        return 1
    if report["latency"]["slo"]["met"] is False:
        print(f"::warning::CI latency target missed; functional conclusion: {report['functional_conclusion']}")
    summary = arguments.summary or os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        write_summary(report, summary)
    return 0 if report["functional_conclusion"] == "success" else 1


if __name__ == "__main__":
    sys.exit(main())
