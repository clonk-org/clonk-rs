#!/usr/bin/env python3
"""Write or verify an exact inventory for a release prebuild artifact.

The manifest may live below ``--root``.  In that case it is metadata and is
the one regular file excluded from the payload inventory.
"""

import argparse
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
from pathlib import Path, PurePosixPath


SCHEMA = 1
IDENTITY_FIELDS = ("head_sha", "tree_sha", "version", "kind", "target")
SHA_RE = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")
SEMVER_RE = re.compile(
    r"(?:0|[1-9][0-9]*)\."
    r"(?:0|[1-9][0-9]*)\."
    r"(?:0|[1-9][0-9]*)"
    r"(?:-(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?\Z"
)
LABEL_RE = re.compile(r"[0-9A-Za-z][0-9A-Za-z._-]*\Z")
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")


class ManifestError(Exception):
    """A manifest or payload violates the hand-off contract."""


def identity_from_args(args):
    identity = {field: getattr(args, field) for field in IDENTITY_FIELDS}
    for field in ("head_sha", "tree_sha"):
        if not SHA_RE.fullmatch(identity[field]):
            raise ManifestError(f"--{field.replace('_', '-')} must be a lowercase Git object ID")
    if not SEMVER_RE.fullmatch(identity["version"]):
        raise ManifestError("--version must be SemVer without a leading v")
    for field in ("kind", "target"):
        if not LABEL_RE.fullmatch(identity[field]):
            raise ManifestError(
                f"--{field} must contain only letters, digits, dot, underscore, and hyphen"
            )
    return identity


def absolute(path):
    """An absolute lexical path; unlike resolve(), this does not follow links."""
    return Path(os.path.abspath(os.fspath(path)))


def excluded_manifest_path(root, manifest):
    try:
        return manifest.relative_to(root).as_posix()
    except ValueError:
        return None


def validate_relative_path(value, source="payload"):
    if not isinstance(value, str) or not value or "\\" in value:
        raise ManifestError(f"unsafe {source} path: {value!r}")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
        raise ManifestError(f"unsafe {source} path: {value!r}")
    if path.as_posix() != value:
        raise ManifestError(f"unsafe {source} path: {value!r}")
    return value


def digest_regular_file(path):
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise ManifestError(f"cannot read regular file {path}: {error}") from error

    digest = hashlib.sha256()
    size = 0
    with os.fdopen(descriptor, "rb") as source:
        mode = os.fstat(source.fileno()).st_mode
        if not stat.S_ISREG(mode):
            raise ManifestError(f"payload entry is not a regular file: {path}")
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            size += len(chunk)
            digest.update(chunk)
    return size, digest.hexdigest()


def inventory(root, excluded_path):
    try:
        root_mode = root.lstat().st_mode
    except OSError as error:
        raise ManifestError(f"cannot inspect payload root {root}: {error}") from error
    if stat.S_ISLNK(root_mode) or not stat.S_ISDIR(root_mode):
        raise ManifestError(f"payload root is not a real directory: {root}")

    files = []

    def visit(directory, prefix):
        try:
            entries = sorted(os.scandir(directory), key=lambda entry: entry.name)
        except OSError as error:
            raise ManifestError(f"cannot scan payload directory {directory}: {error}") from error
        for entry in entries:
            relative = "/".join((*prefix, entry.name))
            validate_relative_path(relative)
            if relative == excluded_path:
                continue
            try:
                mode = entry.stat(follow_symlinks=False).st_mode
            except OSError as error:
                raise ManifestError(f"cannot inspect payload entry {relative}: {error}") from error
            if stat.S_ISDIR(mode):
                visit(Path(entry.path), (*prefix, entry.name))
            elif stat.S_ISREG(mode):
                size, digest = digest_regular_file(entry.path)
                files.append({"path": relative, "size": size, "sha256": digest})
            elif stat.S_ISLNK(mode):
                raise ManifestError(f"payload contains symlink: {relative}")
            else:
                raise ManifestError(f"payload contains non-regular entry: {relative}")

    visit(root, ())
    files.sort(key=lambda entry: entry["path"])
    return files


def declared_paths(values):
    if not values:
        return None
    paths = [validate_relative_path(value, "declared") for value in values]
    if len(paths) != len(set(paths)):
        raise ManifestError("--file paths must be unique")
    return sorted(paths)


def require_declared_file_set(entries, declared):
    if declared is None:
        return
    observed = {entry["path"] for entry in entries}
    expected = set(declared)
    missing = sorted(expected - observed)
    extra = sorted(observed - expected)
    if missing or extra:
        details = []
        if missing:
            details.append("declared but missing: " + ", ".join(missing))
        if extra:
            details.append("undeclared: " + ", ".join(extra))
        raise ManifestError("payload does not match --file declarations (" + "; ".join(details) + ")")


def write_json_atomic(path, document):
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
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
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass


def write_manifest(root, manifest, identity, declared):
    excluded_path = excluded_manifest_path(root, manifest)
    if manifest.exists() or manifest.is_symlink():
        mode = manifest.lstat().st_mode
        if not stat.S_ISREG(mode):
            raise ManifestError(f"manifest path is not a regular file: {manifest}")
    files = inventory(root, excluded_path)
    require_declared_file_set(files, declared)
    document = {
        "schema": SCHEMA,
        **identity,
        "files": files,
    }
    write_json_atomic(manifest, document)
    return len(document["files"])


def reject_duplicate_keys(pairs):
    document = {}
    for key, value in pairs:
        if key in document:
            raise ManifestError(f"manifest has duplicate key {key!r}")
        document[key] = value
    return document


def load_manifest(path):
    try:
        mode = path.lstat().st_mode
    except OSError as error:
        raise ManifestError(f"cannot inspect manifest {path}: {error}") from error
    if not stat.S_ISREG(mode):
        raise ManifestError(f"manifest path is not a regular file: {path}")
    try:
        with path.open("r", encoding="utf-8") as source:
            return json.load(source, object_pairs_hook=reject_duplicate_keys)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ManifestError(f"cannot read manifest {path}: {error}") from error


def validated_file_entries(document):
    expected_keys = {"schema", *IDENTITY_FIELDS, "files"}
    if not isinstance(document, dict) or set(document) != expected_keys:
        raise ManifestError("manifest has unexpected or missing top-level fields")
    if type(document["schema"]) is not int or document["schema"] != SCHEMA:
        raise ManifestError(f"manifest schema must be {SCHEMA}")
    if not isinstance(document["files"], list):
        raise ManifestError("manifest files must be a list")

    entries = []
    previous = None
    for raw in document["files"]:
        if not isinstance(raw, dict) or set(raw) != {"path", "size", "sha256"}:
            raise ManifestError("manifest file entry has unexpected or missing fields")
        path = validate_relative_path(raw["path"], "manifest")
        if previous is not None and path <= previous:
            raise ManifestError("manifest file paths must be unique and sorted")
        previous = path
        if type(raw["size"]) is not int or raw["size"] < 0:
            raise ManifestError(f"manifest size for {path} is invalid")
        if not isinstance(raw["sha256"], str) or not SHA256_RE.fullmatch(raw["sha256"]):
            raise ManifestError(f"manifest sha256 for {path} is invalid")
        entries.append(raw)
    return entries


def verify_manifest(root, manifest, identity, declared):
    document = load_manifest(manifest)
    entries = validated_file_entries(document)
    require_declared_file_set(entries, declared)
    mismatches = [
        field
        for field in IDENTITY_FIELDS
        if not isinstance(document[field], str) or document[field] != identity[field]
    ]
    if mismatches:
        names = ", ".join(field.replace("_", "-") for field in mismatches)
        raise ManifestError(f"manifest identity mismatch: {names}")

    excluded_path = excluded_manifest_path(root, manifest)
    actual = inventory(root, excluded_path)
    expected_by_path = {entry["path"]: entry for entry in entries}
    actual_by_path = {entry["path"]: entry for entry in actual}
    missing = sorted(expected_by_path.keys() - actual_by_path.keys())
    extra = sorted(actual_by_path.keys() - expected_by_path.keys())
    if missing or extra:
        details = []
        if missing:
            details.append("missing: " + ", ".join(missing))
        if extra:
            details.append("extra: " + ", ".join(extra))
        raise ManifestError("payload file set mismatch (" + "; ".join(details) + ")")

    for path in sorted(expected_by_path):
        expected = expected_by_path[path]
        observed = actual_by_path[path]
        if observed["size"] != expected["size"]:
            raise ManifestError(f"payload size mismatch: {path}")
        if observed["sha256"] != expected["sha256"]:
            raise ManifestError(f"payload sha256 mismatch: {path}")
    return len(entries)


def parser():
    argument_parser = argparse.ArgumentParser(description=__doc__)
    argument_parser.add_argument("operation", choices=("write", "verify"))
    argument_parser.add_argument("--root", required=True, type=Path)
    argument_parser.add_argument("--manifest", required=True, type=Path)
    argument_parser.add_argument("--head-sha", required=True)
    argument_parser.add_argument("--tree-sha", required=True)
    argument_parser.add_argument("--version", required=True)
    argument_parser.add_argument("--kind", required=True)
    argument_parser.add_argument("--target", required=True)
    argument_parser.add_argument(
        "--file",
        action="append",
        dest="files",
        metavar="RELATIVE_PATH",
        help="expected payload file relative to --root (repeatable; required for write)",
    )
    return argument_parser


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        identity = identity_from_args(args)
        declared = declared_paths(args.files)
        root = absolute(args.root)
        manifest = absolute(args.manifest)
        if args.operation == "write":
            if declared is None:
                raise ManifestError("write requires at least one --file")
            count = write_manifest(root, manifest, identity, declared)
            print(f"wrote {count} payload files to {manifest}")
        else:
            count = verify_manifest(root, manifest, identity, declared)
            print(f"verified {count} payload files from {manifest}")
    except ManifestError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
