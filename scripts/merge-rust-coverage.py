#!/usr/bin/env python3
"""Merge LCOV shards without double-counting shared source locations.

The aggregate line gate uses unique exported ``DA`` source lines, matching
LCOV's merge semantics. LLVM's ``LF``/``LH`` summaries may additionally count
macro-expansion lines that the LCOV detail records cannot identify or union.
The repository's 79% LLVM-summary floor corresponds to 79.45% on this
DA-based representation, calibrated against the last monolithic baseline.
Merged function totals necessarily report instrumented LCOV symbols rather
than LLVM's source-level deduplication because classic LCOV does not export the
grouping that produced ``FNF``/``FNH``. The enforced line metric is exact.
"""

import argparse
import os
import re
import sys
import tempfile
from dataclasses import dataclass, field
from decimal import Decimal, InvalidOperation
from pathlib import Path


class CoverageError(Exception):
    """An LCOV input cannot be merged without losing or corrupting data."""


@dataclass
class LineCoverage:
    hits: int
    checksum: str | None


@dataclass
class Record:
    source: str
    function_lines: dict[str, int] = field(default_factory=dict)
    function_hits: dict[str, int] = field(default_factory=dict)
    lines: dict[int, LineCoverage] = field(default_factory=dict)
    branches: dict[tuple[int, int, int], int | None] = field(default_factory=dict)
    totals: dict[str, int] = field(default_factory=dict)
    has_functions: bool = False
    has_branches: bool = False


SUMMARY_FIELDS = {"FNF", "FNH", "LF", "LH", "BRF", "BRH"}
CHECKSUM_PATTERN = re.compile(r"[0-9a-fA-F]{32}\Z")


def nonnegative_integer(value, description, location):
    if not value.isascii() or not value.isdecimal():
        raise CoverageError(f"{location}: invalid {description}: {value!r}")
    return int(value)


def positive_integer(value, description, location):
    parsed = nonnegative_integer(value, description, location)
    if parsed == 0:
        raise CoverageError(f"{location}: invalid {description}: must be positive")
    return parsed


def split_field(value, field_name, location, maximum_splits=1):
    parts = value.split(",", maximum_splits)
    if len(parts) != maximum_splits + 1 or any(part == "" for part in parts):
        raise CoverageError(f"{location}: malformed {field_name} field")
    return parts


def add_function(record, value, location):
    line_text, name = split_field(value, "FN", location)
    line = positive_integer(line_text, "function line", location)
    if name in record.function_lines:
        raise CoverageError(f"{location}: duplicate function declaration {name!r}")
    record.function_lines[name] = line
    record.has_functions = True


def add_function_hits(record, value, location):
    hits_text, name = split_field(value, "FNDA", location)
    hits = nonnegative_integer(hits_text, "function hit count", location)
    if name in record.function_hits:
        raise CoverageError(f"{location}: duplicate function hit count {name!r}")
    record.function_hits[name] = hits
    record.has_functions = True


def add_line(record, value, location):
    parts = value.split(",", 2)
    if len(parts) not in (2, 3) or any(part == "" for part in parts):
        raise CoverageError(f"{location}: malformed DA field")
    line = positive_integer(parts[0], "line number", location)
    hits = nonnegative_integer(parts[1], "line hit count", location)
    checksum = parts[2].lower() if len(parts) == 3 else None
    if checksum is not None and CHECKSUM_PATTERN.fullmatch(checksum) is None:
        raise CoverageError(f"{location}: invalid DA checksum {checksum!r}")
    if line in record.lines:
        raise CoverageError(f"{location}: duplicate line coverage for line {line}")
    record.lines[line] = LineCoverage(hits, checksum)


def add_branch(record, value, location):
    parts = split_field(value, "BRDA", location, maximum_splits=3)
    line = positive_integer(parts[0], "branch line", location)
    block = nonnegative_integer(parts[1], "branch block", location)
    branch = nonnegative_integer(parts[2], "branch number", location)
    taken = (
        None
        if parts[3] == "-"
        else nonnegative_integer(parts[3], "branch hit count", location)
    )
    key = (line, block, branch)
    if key in record.branches:
        raise CoverageError(f"{location}: duplicate branch coverage for {key}")
    record.branches[key] = taken
    record.has_branches = True


def add_summary(record, tag, value, location):
    if tag in record.totals:
        raise CoverageError(f"{location}: duplicate {tag} summary")
    record.totals[tag] = nonnegative_integer(value, f"{tag} total", location)
    if tag in ("FNF", "FNH"):
        record.has_functions = True
    elif tag in ("BRF", "BRH"):
        record.has_branches = True


def validate_record(record, location):
    declared_functions = set(record.function_lines)
    counted_functions = set(record.function_hits)
    if declared_functions != counted_functions:
        missing_counts = sorted(declared_functions - counted_functions)
        missing_declarations = sorted(counted_functions - declared_functions)
        details = []
        if missing_counts:
            details.append("missing FNDA for " + ", ".join(missing_counts))
        if missing_declarations:
            details.append("missing FN for " + ", ".join(missing_declarations))
        raise CoverageError(f"{location}: malformed function data ({'; '.join(details)})")

    # LLVM summaries count source-level functions and macro-expansion lines,
    # while FN/FNDA/DA expose instrumented symbols and displayable lines. The
    # two representations intentionally need not have equal cardinality.
    for found_tag, hit_tag in (("FNF", "FNH"), ("LF", "LH"), ("BRF", "BRH")):
        found = record.totals.get(found_tag)
        hit = record.totals.get(hit_tag)
        if (found is None) != (hit is None):
            raise CoverageError(
                f"{location}: {found_tag} and {hit_tag} must appear together"
            )
        if found is not None and hit > found:
            raise CoverageError(
                f"{location}: {hit_tag} declares {hit}, exceeding "
                f"{found_tag} {found}"
            )


def parse_lcov(path):
    try:
        contents = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise CoverageError(f"cannot read {path}: {error}") from error

    records = []
    current = None
    saw_content = False
    for line_number, line in enumerate(contents.splitlines(), start=1):
        if not line:
            continue
        saw_content = True
        location = f"{path}:{line_number}"
        if line == "end_of_record":
            if current is None:
                raise CoverageError(f"{location}: unexpected end_of_record")
            validate_record(current, location)
            records.append(current)
            current = None
            continue

        tag, separator, value = line.partition(":")
        if not separator:
            raise CoverageError(f"{location}: malformed LCOV field {line!r}")
        if tag == "TN" and current is None:
            continue
        if tag == "SF":
            if current is not None:
                raise CoverageError(f"{location}: missing end_of_record before next SF")
            if not value:
                raise CoverageError(f"{location}: malformed SF field")
            current = Record(value)
            continue
        if current is None:
            raise CoverageError(f"{location}: {tag} appears outside an SF record")

        if tag == "FN":
            add_function(current, value, location)
        elif tag == "FNDA":
            add_function_hits(current, value, location)
        elif tag == "DA":
            add_line(current, value, location)
        elif tag == "BRDA":
            add_branch(current, value, location)
        elif tag in SUMMARY_FIELDS:
            add_summary(current, tag, value, location)
        else:
            raise CoverageError(f"{location}: unknown LCOV field {tag!r}")

    if current is not None:
        raise CoverageError(f"{path}: missing end_of_record at end of input")
    if not saw_content or not records:
        raise CoverageError(f"{path}: empty LCOV input")
    return records


def merge_line(target, incoming, source, line):
    existing = target.lines.get(line)
    if existing is None:
        target.lines[line] = LineCoverage(incoming.hits, incoming.checksum)
        return
    if (
        existing.checksum is not None
        and incoming.checksum is not None
        and existing.checksum != incoming.checksum
    ):
        raise CoverageError(f"checksum conflict for {source}:{line}")
    existing.hits += incoming.hits
    existing.checksum = existing.checksum or incoming.checksum


def merge_record(target, incoming):
    for name, line in incoming.function_lines.items():
        existing_line = target.function_lines.get(name)
        if existing_line is not None and existing_line != line:
            raise CoverageError(
                f"function location conflict for {name!r} in {target.source}: "
                f"lines {existing_line} and {line}"
            )
        target.function_lines[name] = line
        target.function_hits[name] = (
            target.function_hits.get(name, 0) + incoming.function_hits[name]
        )

    for line, coverage in incoming.lines.items():
        merge_line(target, coverage, target.source, line)

    for branch, incoming_hits in incoming.branches.items():
        if branch not in target.branches:
            target.branches[branch] = incoming_hits
            continue
        existing_hits = target.branches[branch]
        if existing_hits is None:
            target.branches[branch] = incoming_hits
        elif incoming_hits is not None:
            target.branches[branch] = existing_hits + incoming_hits

    target.has_functions |= incoming.has_functions
    target.has_branches |= incoming.has_branches


def merged_records(paths):
    merged = {}
    for path in paths:
        for incoming in parse_lcov(path):
            target = merged.setdefault(incoming.source, Record(incoming.source))
            merge_record(target, incoming)
    if not merged:
        raise CoverageError("coverage inputs contain no source records")
    return merged


def render_lcov(records):
    output = []
    for source in sorted(records):
        record = records[source]
        output.append(f"SF:{source}")

        functions = sorted(
            record.function_lines.items(), key=lambda item: (item[1], item[0])
        )
        for name, line in functions:
            output.append(f"FN:{line},{name}")
        for name, _line in functions:
            output.append(f"FNDA:{record.function_hits[name]},{name}")
        if record.has_functions:
            output.append(f"FNF:{len(functions)}")
            output.append(
                f"FNH:{sum(record.function_hits[name] > 0 for name, _ in functions)}"
            )

        for line in sorted(record.lines):
            coverage = record.lines[line]
            checksum = "" if coverage.checksum is None else f",{coverage.checksum}"
            output.append(f"DA:{line},{coverage.hits}{checksum}")

        for (line, block, branch), hits in sorted(record.branches.items()):
            taken = "-" if hits is None else str(hits)
            output.append(f"BRDA:{line},{block},{branch},{taken}")
        if record.has_branches:
            output.append(f"BRF:{len(record.branches)}")
            output.append(
                "BRH:"
                + str(
                    sum(
                        hits is not None and hits > 0
                        for hits in record.branches.values()
                    )
                )
            )
        output.append(f"LF:{len(record.lines)}")
        output.append(f"LH:{sum(line.hits > 0 for line in record.lines.values())}")
        output.append("end_of_record")
    return "\n".join(output) + "\n"


def write_atomic(path, contents):
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                newline="\n",
                dir=path.parent,
                prefix=f".{path.name}.",
                delete=False,
            ) as output:
                temporary = Path(output.name)
                output.write(contents)
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        finally:
            if temporary is not None:
                try:
                    temporary.unlink()
                except FileNotFoundError:
                    pass
    except OSError as error:
        raise CoverageError(f"cannot write {path}: {error}") from error


def coverage_counts(records):
    found = sum(len(record.lines) for record in records.values())
    if found == 0:
        raise CoverageError("coverage inputs contain no line data")
    hit = sum(
        line.hits > 0
        for record in records.values()
        for line in record.lines.values()
    )
    return hit, found


def percentage(value):
    try:
        parsed = Decimal(value)
    except InvalidOperation as error:
        raise argparse.ArgumentTypeError("must be a percentage from 0 to 100") from error
    if not parsed.is_finite() or parsed < 0 or parsed > 100:
        raise argparse.ArgumentTypeError("must be a percentage from 0 to 100")
    return parsed


def arguments(argv):
    parser = argparse.ArgumentParser(
        description="Merge LCOV reports by source location and validate line coverage."
    )
    parser.add_argument("inputs", nargs="+", type=Path, help="LCOV reports to merge")
    parser.add_argument("--output", type=Path, help="write the merged LCOV report")
    parser.add_argument(
        "--fail-under-lines",
        type=percentage,
        metavar="PERCENT",
        help="fail if aggregate unique line coverage is below PERCENT",
    )
    parsed = parser.parse_args(argv)
    if parsed.output is None and parsed.fail_under_lines is None:
        parser.error("--output is required unless --fail-under-lines is supplied")
    return parsed


def main(argv=None):
    args = arguments(argv)
    try:
        records = merged_records(args.inputs)
        if args.output is not None:
            write_atomic(args.output, render_lcov(records))
        hit, found = coverage_counts(records)
        line_percent = Decimal(hit) * 100 / Decimal(found)
        print(f"lines: {hit}/{found} ({line_percent:.2f}%)")
        if (
            args.fail_under_lines is not None
            and Decimal(hit) * 100 < args.fail_under_lines * Decimal(found)
        ):
            raise CoverageError(
                f"line coverage {line_percent:.2f}% is below required "
                f"{args.fail_under_lines}%"
            )
    except CoverageError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
