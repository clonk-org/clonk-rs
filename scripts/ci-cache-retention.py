#!/usr/bin/env python3
"""Bound optional main-branch build caches; never decide build correctness."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


PREFIX = "clonk-ci-target-v1-"
LANES = ("landing-linux", "coverage-linux")
MAIN_REF = "refs/heads/main"
MAX_PAGES = 200
INVENTORY_TIMEOUT = 180


class RetentionError(Exception):
    """Cache inventory or a requested mutation could not be verified."""


def require_trusted_main(repository: str) -> None:
    event = os.environ.get("GITHUB_EVENT_NAME")
    if (
        os.environ.get("GITHUB_REF") != MAIN_REF
        or os.environ.get("GITHUB_REPOSITORY", "").casefold() != repository.casefold()
        or event not in ("push", "workflow_dispatch", "schedule")
        or os.environ.get("GITHUB_HEAD_REF")
        or not os.environ.get("GITHUB_EVENT_PATH")
    ):
        raise RetentionError("--apply requires a trusted main-branch Actions event from the requested repository")
    try:
        payload = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
        full_name = payload["repository"]["full_name"]
        valid = isinstance(full_name, str) and full_name.casefold() == repository.casefold()
        if event == "push":
            valid = valid and payload.get("ref") == MAIN_REF and payload.get("deleted", False) is False
        elif "ref" in payload:
            valid = valid and payload["ref"] in (MAIN_REF, "main")
        if not valid:
            raise ValueError("event repository or branch differs")
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise RetentionError(f"--apply requires a verified trusted main event payload: {error}") from error


def inventory(repository: str) -> list[dict]:
    caches, total, identifiers, deadline = [], None, set(), time.monotonic() + INVENTORY_TIMEOUT
    for number in range(1, MAX_PAGES + 1):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RetentionError("GitHub cache inventory exceeded its time limit")
        endpoint = f"repos/{repository}/actions/caches?per_page=100&page={number}"
        result = subprocess.run(
            ["gh", "api", endpoint, "-H", "Accept: application/vnd.github+json",
             "-H", "X-GitHub-Api-Version: 2022-11-28"],
            capture_output=True, text=True, timeout=min(30, remaining), check=False,
        )
        if result.returncode:
            raise RetentionError(f"GitHub cache inventory failed: {result.stderr.strip()}")
        if len(result.stdout) > 4 * 1024 * 1024:
            raise RetentionError("GitHub cache inventory page exceeded its size limit")
        document = json.loads(result.stdout)
        if not isinstance(document, dict):
            raise RetentionError("GitHub cache inventory page is not an object")
        count, batch = document.get("total_count"), document.get("actions_caches")
        if type(count) is not int or not 0 <= count <= MAX_PAGES * 100 or not isinstance(batch, list) or len(batch) > 100:
            raise RetentionError("GitHub cache pagination is malformed")
        if total is not None and total != count:
            raise RetentionError("GitHub cache pagination changed during collection")
        total = count
        for item in batch:
            cache = validate_cache(item)
            if cache["id"] in identifiers:
                raise RetentionError("GitHub cache pagination duplicated a cache ID")
            identifiers.add(cache["id"])
            caches.append(cache)
        if len(caches) > total or (not batch and len(caches) < total):
            raise RetentionError("GitHub cache pagination disagrees with total_count")
        if len(caches) == total:
            return caches
    raise RetentionError("GitHub cache inventory exceeded its page limit")


def timestamp(value: object) -> datetime:
    if not isinstance(value, str) or not re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})", value
    ):
        raise RetentionError(f"GitHub cache timestamp is malformed: {value!r}")
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)
    except ValueError as error:
        raise RetentionError(f"GitHub cache timestamp is invalid: {value!r}") from error


def validate_cache(record: object) -> dict:
    if not isinstance(record, dict) or type(record.get("id")) is not int or record["id"] < 1:
        raise RetentionError("GitHub cache entry has no valid ID")
    if type(record.get("size_in_bytes")) is not int or record["size_in_bytes"] < 0:
        raise RetentionError("GitHub cache entry has no valid byte size")
    for field in ("key", "ref"):
        value = record.get(field)
        if (
            not isinstance(value, str) or not value
            or any(ord(character) < 32 or ord(character) == 127 for character in value)
        ):
            raise RetentionError(f"GitHub cache entry has no valid {field}")
    created = timestamp(record.get("created_at"))
    accessed = timestamp(record.get("last_accessed_at"))
    if accessed < created:
        raise RetentionError("GitHub cache access timestamp precedes its creation")
    return {field: record[field] for field in ("id", "key", "ref", "size_in_bytes", "created_at", "last_accessed_at")}


def lane(cache: dict) -> str | None:
    if cache["ref"] != MAIN_REF:
        return None
    return next((
        name for name in LANES
        if cache["key"].startswith(PREFIX + name + "-") and len(cache["key"]) > len(PREFIX + name + "-")
    ), None)


def newest(cache: dict) -> tuple[datetime, datetime, int]:
    return timestamp(cache["created_at"]), timestamp(cache["last_accessed_at"]), cache["id"]


def plan(caches: list[dict], arguments: argparse.Namespace) -> dict:
    eligible = [dict(cache, lane=name) for cache in caches if (name := lane(cache))]
    ordered = sorted(eligible, key=newest, reverse=True)
    current = {
        next((cache["id"] for cache in ordered if cache["lane"] == name), None) for name in LANES
    }
    selected, evicted, counts, used = [], [], dict.fromkeys(LANES, 0), 0
    # Reserve each lane's latest source cache before considering older extras.
    for cache in sorted(ordered, key=lambda item: (item["id"] in current, newest(item)), reverse=True):
        reason = None
        if cache["size_in_bytes"] > arguments.max_bytes:
            reason = "cache-larger-than-budget"
        elif counts[cache["lane"]] >= arguments.keep_per_lane:
            reason = "per-lane-limit"
        elif used + cache["size_in_bytes"] > arguments.max_bytes:
            reason = "current-caches-exceed-budget" if cache["id"] in current else "byte-budget"
        if reason:
            evicted.append(dict(cache, reason=reason))
        else:
            selected.append(dict(cache, reason="current" if cache["id"] in current else "older"))
            counts[cache["lane"]] += 1
            used += cache["size_in_bytes"]
    return {
        "schema": 1, "mode": "apply" if arguments.apply else "dry-run",
        "repository": arguments.repository, "prefix": PREFIX, "ref": MAIN_REF,
        "max_bytes": arguments.max_bytes, "keep_per_lane": arguments.keep_per_lane,
        "recency_order": "created_at, last_accessed_at, id descending",
        "inventory_count": len(caches), "inventory_bytes": sum(cache["size_in_bytes"] for cache in caches),
        "eligible_count": len(eligible), "eligible_bytes": sum(cache["size_in_bytes"] for cache in eligible),
        "ignored_count": len(caches) - len(eligible), "selected": selected, "selected_count": len(selected),
        "ignored_bytes": sum(cache["size_in_bytes"] for cache in caches if lane(cache) is None),
        "selected_bytes": used, "evicted": evicted, "evicted_count": len(evicted),
        "evicted_bytes": sum(cache["size_in_bytes"] for cache in evicted),
        "deleted": [], "deleted_count": 0, "deleted_bytes": 0, "status": "success",
        "already_missing": [], "already_missing_count": 0, "already_missing_bytes": 0,
    }


def apply(repository: str, receipt: dict) -> None:
    deadline = time.monotonic() + INVENTORY_TIMEOUT
    for cache in receipt["evicted"]:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RetentionError("GitHub cache deletion exceeded its time limit")
        endpoint = f"repos/{repository}/actions/caches/{cache['id']}"
        result = subprocess.run(
            ["gh", "api", "--method", "DELETE", "--include", endpoint,
             "-H", "Accept: application/vnd.github+json", "-H", "X-GitHub-Api-Version: 2022-11-28"],
            capture_output=True, text=True, timeout=min(30, remaining), check=False,
        )
        # --include puts the response status first. A body or stderr mentioning
        # HTTP 404 does not prove that this DELETE found an absent cache.
        status_line = re.match(r"\AHTTP/\S+[ \t]+(\d{3})(?:[ \t]|\r?\n)", result.stdout)
        status = int(status_line[1]) if status_line else None
        if status == 404:
            receipt["already_missing"].append(dict(cache, outcome="already-missing"))
            receipt["already_missing_count"] += 1
            receipt["already_missing_bytes"] += cache["size_in_bytes"]
            continue
        if result.returncode or status != 204:
            raise RetentionError(
                f"GitHub cache deletion {cache['id']} failed (HTTP {status}): {result.stderr.strip()}"
            )
        receipt["deleted"].append(dict(cache, outcome="deleted"))
        receipt["deleted_count"] += 1
        receipt["deleted_bytes"] += cache["size_in_bytes"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--prefix", default=PREFIX, choices=(PREFIX,))
    parser.add_argument("--keep-per-lane", type=int, default=2)
    parser.add_argument("--max-bytes", type=int, default=4 * 1024 ** 3)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    receipt = {
        "schema": 1, "mode": "apply" if arguments.apply else "dry-run", "repository": arguments.repository,
        "selected": [], "selected_count": 0, "selected_bytes": 0,
        "deleted": [], "deleted_count": 0, "deleted_bytes": 0,
        "already_missing": [], "already_missing_count": 0, "already_missing_bytes": 0,
    }
    exit_code, temporary_output = 0, None
    try:
        if arguments.output:
            if arguments.output.is_symlink() or (arguments.output.exists() and not arguments.output.is_file()):
                raise RetentionError("--output must name a regular JSON file")
            arguments.output.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(
                dir=arguments.output.parent, prefix=f".{arguments.output.name}.", delete=False
            ) as output:
                temporary_output = Path(output.name)
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}", arguments.repository):
            raise RetentionError("--repository must be an owner/repository name")
        if arguments.keep_per_lane < 1 or arguments.max_bytes < 0:
            raise RetentionError("--keep-per-lane must be positive and --max-bytes must be nonnegative")
        if arguments.apply:
            require_trusted_main(arguments.repository)
        receipt = plan(inventory(arguments.repository), arguments)
        if arguments.apply:
            apply(arguments.repository, receipt)
    except (RetentionError, OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        print(f"cache retention failed: {error}", file=sys.stderr)
        receipt.update(status="failure", error=str(error))
        exit_code = 1
    encoded = json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    print(encoded, end="")
    if temporary_output:
        try:
            temporary_output.write_text(encoded, encoding="utf-8")
            os.replace(temporary_output, arguments.output)
        except OSError as error:
            print(f"cache retention receipt publication failed: {error}", file=sys.stderr)
            exit_code = 1
        finally:
            temporary_output.unlink(missing_ok=True)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
