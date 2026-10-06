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
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path, PurePosixPath


SCHEMA = 1
PROVENANCE_SCHEMA = 3
IDENTITY_FIELDS = ("head_sha", "tree_sha", "version", "kind", "target")
RECIPE_INPUTS = (
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
    "scripts/configure-msvc-runtime.sh", "scripts/release-prebuild-manifest.py",
    ".github/workflows/release-prebuild.yml", ".github/workflows/release-build.yml",
    ".github/workflows/device-loss-qualification.yml",
)
OPTIONAL_RECIPE_INPUTS = (
    ".github/workflows/release-platform.yml", ".github/actions/device-loss/action.yml",
    ".github/actions/verify-cache-handoff/action.yml",
    "scripts/ci-content.py", "scripts/ci-workspace-cache.py",
)
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
BUILD_ENVIRONMENT = (
    "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "CC", "CXX", "AR", "LD",
    "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "HOST_CC", "HOST_CXX", "TARGET_CC", "TARGET_CXX",
    "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "CMAKE_TOOLCHAIN_FILE", "LINK", "_LINK_",
)
RECIPE_FIELDS = {"operation", "profile", "features", "target", "environment", "rustc", "native_tools", "parents"}
RUNTIME_TARGETS = {"x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc",
                   "aarch64-apple-darwin", "x86_64-apple-darwin"}


class ManifestError(Exception):
    """A manifest or payload violates the hand-off contract."""


def provenance_context(root, identity):
    """Derive source/compiler identity; consumer build flags are deliberately absent."""
    root = Path(root)

    def git(*arguments):
        return subprocess.check_output(["git", "-C", str(root), *arguments], stderr=subprocess.PIPE)

    try:
        for field, revision in (("head_sha", "HEAD"), ("tree_sha", "HEAD^{tree}")):
            if git("rev-parse", revision).decode().strip() != identity[field]:
                raise ManifestError(f"provenance source identity mismatch: {field}")
        inputs = (".", ":(exclude)content")
        if (git("diff", "--name-only", "HEAD", "--", *inputs).strip()
                or git("ls-files", "--others", "--exclude-standard", "--", *inputs).strip()):
            raise ManifestError("provenance requires committed source inputs")
        version = tomllib.loads(git("show", "HEAD:Cargo.toml").decode())["workspace"]["package"]["version"]
        if identity["version"] != version:
            raise ManifestError("provenance source identity mismatch: version")
        content = git("ls-tree", "HEAD", "--", "content").decode().split()
        if (len(content) != 4 or content[:2] != ["160000", "commit"]
                or content[3] != "content" or not SHA_RE.fullmatch(content[2])):
            raise ManifestError("provenance requires the committed content gitlink")
        if identity["kind"] not in ("tool", "runtime"):
            raise ManifestError("provenance kind must be tool or runtime")
        recipe_paths = (*RECIPE_INPUTS,
                        *git("ls-tree", "--name-only", "HEAD", "--", *OPTIONAL_RECIPE_INPUTS).decode().splitlines())
        recipe = {
            "kind": identity["kind"], "target": identity["target"],
            "profile": "test" if identity["kind"] == "tool" else "release",
            "features": ["engine-tools"] if identity["kind"] == "tool" else [],
            "tree_sha": identity["tree_sha"],
            "sources": {name: hashlib.sha256(git("show", f"HEAD:{name}")).hexdigest()
                        for name in recipe_paths},
        }
        toolchain = subprocess.check_output(["rustc", "-vV"], cwd=root, stderr=subprocess.PIPE)
        channel = tomllib.loads(git("show", "HEAD:rust-toolchain.toml").decode())["toolchain"]["channel"]
        if compiler_field(toolchain.decode(), "release") != channel:
            raise ManifestError("provenance compiler does not match the pinned rust-toolchain channel")
    except (OSError, subprocess.CalledProcessError, UnicodeError, ValueError, KeyError) as error:
        raise ManifestError(f"cannot derive release provenance: {error}") from error

    actions = os.environ.get("GITHUB_ACTIONS") == "true"
    run_id = os.environ.get("GITHUB_RUN_ID", "" if actions else "local")
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "" if actions else "1")
    if not (re.fullmatch(r"[1-9][0-9]*", run_id) or (run_id == "local" and not actions)):
        raise ManifestError("provenance requires a positive producer GITHUB_RUN_ID")
    if not re.fullmatch(r"[1-9][0-9]*", attempt):
        raise ManifestError("provenance requires a positive GITHUB_RUN_ATTEMPT")
    encoded_recipe = json.dumps(recipe, sort_keys=True, separators=(",", ":")).encode()
    return {
        "recipe_sha": hashlib.sha256(encoded_recipe).hexdigest(),
        "toolchain_sha": hashlib.sha256(toolchain).hexdigest(),
        "content_sha": content[2],
        "builder": {"run_id": run_id, "run_attempt": int(attempt)},
    }


def verify_provenance(observed, expected, identity, provenance_root=None):
    if not isinstance(observed, dict) or set(observed) != {*expected, "producer_recipe"}:
        raise ManifestError("manifest provenance has unexpected or missing fields")
    for field in ("recipe_sha", "toolchain_sha", "content_sha"):
        pattern = SHA_RE if field == "content_sha" else SHA256_RE
        if (not isinstance(observed[field], str) or not pattern.fullmatch(observed[field])
                or observed[field] != expected[field]):
            raise ManifestError(f"manifest provenance mismatch: {field}")
    builder = observed["builder"]
    if not isinstance(builder, dict) or set(builder) != {"run_id", "run_attempt"}:
        raise ManifestError("manifest builder has unexpected or missing fields")
    if builder["run_id"] != expected["builder"]["run_id"]:
        raise ManifestError("manifest producer run ID mismatch")
    if type(builder["run_attempt"]) is not int or not 0 < builder["run_attempt"] <= expected["builder"]["run_attempt"]:
        raise ManifestError("manifest producer attempt is invalid or newer than this run")
    verify_producer_recipe(observed["producer_recipe"], identity, observed, provenance_root)


def compiler_field(identity, field):
    match = re.search(rf"^{re.escape(field)}: (.+)$", identity, re.MULTILINE)
    if match is None:
        raise ManifestError(f"compiler identity is missing {field}")
    return match[1]


def build_environment_names(target):
    suffix = target.replace("-", "_")
    return sorted({*BUILD_ENVIRONMENT, f"CARGO_TARGET_{suffix.upper()}_LINKER",
                   f"CARGO_TARGET_{suffix.upper()}_RUSTFLAGS", f"CMAKE_TOOLCHAIN_FILE_{suffix}",
                   *(f"{name}_{variant}" for name in ("CC", "CXX", "AR", "CFLAGS", "CXXFLAGS", "CPPFLAGS")
                     for variant in (target, suffix))})


def command_words(value):
    try:
        words = [value] if Path(value).is_file() else shlex.split(value, posix=os.name != "nt")
    except (OSError, ValueError) as error:
        raise ManifestError("native compiler command is malformed") from error
    if not words:
        raise ManifestError("native compiler command is empty")
    return words


def available_identity(command, root):
    try:
        return subprocess.check_output(command, cwd=root, stderr=subprocess.STDOUT, timeout=10).decode().strip() or None
    except subprocess.CalledProcessError as error:
        # cl.exe reports its banner and exits 2 when invoked without source files.
        if len(command) == 1 and Path(command[0]).name.lower() in ("cl", "cl.exe") and error.output:
            observed = error.output.decode(errors="replace").strip()
            if "C/C++ Optimizing Compiler Version" in observed:
                return observed
        return None
    except (OSError, subprocess.TimeoutExpired, UnicodeError):
        return None


def native_tool_identities(root, environment, target):
    suffix = target.replace("-", "_")
    result = {}
    for name, variable, fallback in (("cc", "CC", "cl" if os.name == "nt" else "cc"),
                                    ("cxx", "CXX", "cl" if os.name == "nt" else "c++"),
                                    ("linker", "LD", "link" if os.name == "nt" else "cc")):
        candidates = ([f"CARGO_TARGET_{suffix.upper()}_LINKER"] if name == "linker" else [])
        candidates += [f"{variable}_{target}", f"{variable}_{suffix}", f"TARGET_{variable}", variable]
        source = next((key for key in candidates if environment.get(key)), None)
        # Cargo discovers the MSVC linker through Visual Studio. An available
        # PATH link.exe may instead be Git Bash's unrelated GNU coreutils tool;
        # LD also does not prove Cargo selected it for the Rust host build.
        if (name == "linker" and target.endswith("-windows-msvc")
                and source != f"CARGO_TARGET_{suffix.upper()}_LINKER"):
            result[name] = None
            continue
        value = environment[source] if source else shutil.which(fallback)
        if value is None:
            result[name] = None
            continue
        command = command_words(value)
        executable = Path(command[0]).name.lower()
        arguments = ([] if executable in ("cl", "cl.exe") else
                     ["-flavor", "link", "--version"] if "rust-lld" in executable else ["--version"])
        result[name] = {"source": source or "available", "command": command,
                        "version": available_identity([*command, *arguments], root)}
    result["sdk"] = {"root": environment["SDKROOT"], "version": None}
    if sys.platform == "darwin" and target.endswith("apple-darwin"):
        sdk = environment["SDKROOT"] or "macosx"
        result["sdk"]["version"] = available_identity(["xcrun", "--sdk", sdk, "--show-sdk-version"], root)
    return result


def producer_recipe(root, identity, profile, target, features):
    rustc = subprocess.check_output(["rustc", "-vV"], cwd=root, stderr=subprocess.PIPE).decode()
    environment = {name: os.environ.get(name) for name in build_environment_names(target)}
    return {"operation": "build", "profile": profile, "features": features, "target": target,
            "environment": environment, "rustc": rustc,
            "native_tools": native_tool_identities(root, environment, target), "parents": []}


def verify_producer_recipe(recipe, identity, provenance, provenance_root=None):
    if not isinstance(recipe, dict) or set(recipe) != RECIPE_FIELDS:
        raise ManifestError("manifest producer recipe has unexpected or missing fields")
    if recipe["operation"] == "transform":
        verify_transform_recipe(recipe, identity, provenance, provenance_root)
        return
    if recipe["operation"] != "build" or recipe["parents"] != []:
        raise ManifestError("manifest producer recipe operation is invalid")
    if (recipe["profile"] != ("test" if identity["kind"] == "tool" else "release")
            or recipe["features"] != (["engine-tools"] if identity["kind"] == "tool" else [])):
        raise ManifestError("manifest producer recipe profile/features mismatch")
    rustc = recipe["rustc"]
    if (not isinstance(rustc, str) or hashlib.sha256(utf8_bytes(rustc)).hexdigest() != provenance["toolchain_sha"]):
        raise ManifestError("manifest producer recipe compiler identity mismatch")
    target = compiler_field(rustc, "host") if identity["kind"] == "tool" else identity["target"]
    if (recipe["target"] != target or (identity["kind"] == "tool" and identity["target"] != "host")
            or (identity["kind"] == "runtime" and target not in RUNTIME_TARGETS)):
        raise ManifestError("manifest producer recipe target mismatch")
    environment = recipe["environment"]
    if (not isinstance(environment, dict) or set(environment) != set(build_environment_names(target))
            or any(value is not None and not isinstance(value, str) for value in environment.values())
            or environment["CARGO_BUILD_TARGET"] not in (None, target)):
        raise ManifestError("manifest producer recipe environment mismatch")
    tools = recipe["native_tools"]
    if not isinstance(tools, dict) or set(tools) != {"cc", "cxx", "linker", "sdk"}:
        raise ManifestError("manifest native tool identities are invalid")
    if (target.endswith("-windows-msvc") and tools["linker"] is not None
            and (not isinstance(tools["linker"], dict)
                 or tools["linker"].get("source") != f"CARGO_TARGET_{target.replace('-', '_').upper()}_LINKER")):
        raise ManifestError("manifest MSVC linker selection is unproven")
    for tool in (tools["cc"], tools["cxx"], tools["linker"]):
        if tool is None:
            continue
        if (not isinstance(tool, dict) or set(tool) != {"source", "command", "version"}
                or not isinstance(tool["source"], str) or tool["source"] not in {"available", *environment}
                or not isinstance(tool["command"], list) or not tool["command"]
                or any(not isinstance(word, str) or not word for word in tool["command"])
                or (tool["version"] is not None and not isinstance(tool["version"], str))):
            raise ManifestError("manifest native tool identity is invalid")
        if tool["source"] != "available" and tool["command"] != command_words(environment[tool["source"]] or ""):
            raise ManifestError("manifest native tool selection mismatch")
    if (not isinstance(tools["sdk"], dict) or set(tools["sdk"]) != {"root", "version"}
            or tools["sdk"]["root"] != environment["SDKROOT"]
            or (tools["sdk"]["version"] is not None and not isinstance(tools["sdk"]["version"], str))):
        raise ManifestError("manifest SDK identity is invalid")
    if identity["kind"] == "runtime" and target == "x86_64-pc-windows-msvc":
        verify_msvc_recipe(recipe)


def utf8_bytes(value):
    try:
        return value.encode("utf-8")
    except UnicodeError as error:
        raise ManifestError("manifest provenance contains invalid UTF-8 text") from error


def runtime_files(target):
    suffix = ".exe" if target == "x86_64-pc-windows-msvc" else ""
    return [f"payload/{name}{suffix}" for name in ("c4group", "clonk-app", "clonk-game")]


def require_runtime_files(entries, target):
    require_declared_file_set(entries, runtime_files(target))
    if any(entry["size"] == 0 for entry in entries):
        raise ManifestError("runtime payload contains an empty shipped binary")


def parent_targets(identity):
    if identity["kind"] != "runtime":
        raise ManifestError("only a runtime can retain parent manifests")
    if identity["target"] == "universal-apple-darwin":
        return {"aarch64-apple-darwin", "x86_64-apple-darwin"}
    if identity["target"] not in RUNTIME_TARGETS:
        raise ManifestError("packaging runtime target is unsupported")
    return {identity["target"]}


def verify_parent_document(document, identity, provenance, provenance_root):
    entries = validated_file_entries(document, require_provenance=True)
    target = document["target"]
    if not isinstance(target, str) or target not in parent_targets(identity):
        raise ManifestError("parent manifest target does not match the packaged runtime")
    parent_identity = {**identity, "target": target}
    if any(document[field] != parent_identity[field] for field in IDENTITY_FIELDS):
        raise ManifestError("parent manifest source/version/runtime identity mismatch")
    parent_provenance = document["provenance"]
    if (not isinstance(parent_provenance, dict) or not isinstance(parent_provenance.get("producer_recipe"), dict)
            or parent_provenance["producer_recipe"].get("operation") != "build"):
        raise ManifestError("parent manifest must retain an original compiled runtime")
    if provenance_root is None:
        raise ManifestError("parent verification requires the provenance source checkout")
    expected = provenance_context(provenance_root, parent_identity)
    expected["builder"] = dict(provenance["builder"])
    verify_provenance(parent_provenance, expected, parent_identity, provenance_root)
    require_runtime_files(entries, target)
    return parent_identity, expected


def verify_transform_recipe(recipe, identity, provenance, provenance_root):
    if (recipe["target"] != identity["target"]
            or any(recipe[field] is not None for field in ("profile", "features", "environment", "rustc", "native_tools"))
            or not isinstance(recipe["parents"], list)
            or len(recipe["parents"]) != len(parent_targets(identity))):
        raise ManifestError("transform producer recipe must retain the exact original runtime parents")
    targets = []
    for parent in recipe["parents"]:
        if (not isinstance(parent, dict) or set(parent) != {"sha256", "manifest_json"}
                or not isinstance(parent["sha256"], str) or not SHA256_RE.fullmatch(parent["sha256"])
                or not isinstance(parent["manifest_json"], str)
                or hashlib.sha256(utf8_bytes(parent["manifest_json"])).hexdigest() != parent["sha256"]):
            raise ManifestError("retained parent manifest digest or shape is invalid")
        try:
            document = json.loads(parent["manifest_json"], object_pairs_hook=reject_duplicate_keys)
        except json.JSONDecodeError as error:
            raise ManifestError("retained parent manifest is malformed") from error
        parent_identity, _ = verify_parent_document(document, identity, provenance, provenance_root)
        targets.append(parent_identity["target"])
    if targets != sorted(parent_targets(identity)):
        raise ManifestError("retained parent targets must be unique, complete, and sorted")


def transform_recipe(paths, identity, provenance, provenance_root):
    if len(paths) != len(parent_targets(identity)):
        raise ManifestError("packaging requires the exact original runtime parent manifests")
    parents = []
    for path in paths:
        path = absolute(path)
        document = load_manifest(path)
        parent_identity, expected = verify_parent_document(document, identity, provenance, provenance_root)
        verify_manifest(path.parent, path, parent_identity, runtime_files(parent_identity["target"]), expected,
                        provenance_root=provenance_root)
        try:
            raw = path.read_bytes().decode("utf-8")
            if json.loads(raw, object_pairs_hook=reject_duplicate_keys) != document:
                raise ManifestError("parent manifest changed while packaging")
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise ManifestError("cannot retain the verified parent manifest") from error
        parents.append((parent_identity["target"], {"sha256": hashlib.sha256(utf8_bytes(raw)).hexdigest(),
                                                   "manifest_json": raw}))
    return {"operation": "transform", "profile": None, "features": None, "target": identity["target"],
            "environment": None, "rustc": None, "native_tools": None,
            "parents": [parent for _, parent in sorted(parents, key=lambda item: item[0])]}


def verify_msvc_recipe(recipe):
    environment = recipe["environment"]
    encoded = environment["CARGO_ENCODED_RUSTFLAGS"]
    try:
        flags = encoded.split("\x1f") if encoded is not None else shlex.split(environment["RUSTFLAGS"] or "")
    except ValueError as error:
        raise ManifestError("Windows MSVC compiler flags are malformed") from error
    normalized = []
    iterator = iter(flags)
    for flag in iterator:
        normalized.append("-C" + next(iterator, "") if flag == "-C" else flag)
    required = {
        "-Ctarget-feature=+crt-static", "-Clinker-plugin-lto", "-Clinker-flavor=lld-link",
        "-Clink-arg=/lldltocachepolicy:cache_size=0%:cache_size_bytes=512m",
        "-Clink-arg=/DEBUG:NONE", "-Clink-arg=/OPT:REF,ICF", "-Clink-arg=/TIME", "-Clink-arg=/Brepro",
    }
    controlled = ("-Ctarget-feature=", "-Clinker-plugin-lto", "-Clinker-flavor=", "-Clinker=", "-Clink-arg=/DEBUG")
    linker = recipe["native_tools"]["linker"]
    if (not required.issubset(normalized)
            or any(flag.startswith(controlled) and flag not in required for flag in normalized)
            or not any(flag.startswith("-Clink-arg=/lldltocache:") for flag in normalized)
            or environment["CARGO_BUILD_TARGET"] != "x86_64-pc-windows-msvc"
            or not environment["CFLAGS_x86_64_pc_windows_msvc"]
            or not environment["CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc"]
            or any(re.search(r"(?:^|\s)[/-]MDd?(?=\s|$)", value or "", re.IGNORECASE)
                   for key, value in environment.items() if key.startswith(("CFLAGS", "CXXFLAGS")))
            or any(environment[key] not in (None, "") for key in ("LINK", "_LINK_"))
            or compiler_field(recipe["rustc"], "LLVM version") != "22.1.8"
            or linker is None or linker["source"] != "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER"
            or linker["version"] is None or not re.search(r"\bLLD 22\.1\.8\b", linker["version"])):
        raise ManifestError("Windows MSVC producer recipe violates the shipped static CRT/LLVM/LLD contract")


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


def write_manifest(root, manifest, identity, declared, provenance=None):
    excluded_path = excluded_manifest_path(root, manifest)
    if manifest.exists() or manifest.is_symlink():
        mode = manifest.lstat().st_mode
        if not stat.S_ISREG(mode):
            raise ManifestError(f"manifest path is not a regular file: {manifest}")
    files = inventory(root, excluded_path)
    require_declared_file_set(files, declared)
    if provenance is not None and identity["kind"] == "runtime":
        require_runtime_files(files, identity["target"])
    document = {
        "schema": PROVENANCE_SCHEMA if provenance is not None else SCHEMA,
        **identity,
        "files": files,
    }
    if provenance is not None:
        document["provenance"] = provenance
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


def validated_file_entries(document, require_provenance=False):
    expected_keys = {"schema", *IDENTITY_FIELDS, "files"}
    schema = PROVENANCE_SCHEMA if require_provenance else SCHEMA
    if (not isinstance(document, dict) or type(document.get("schema")) is not int
            or document["schema"] != schema):
        raise ManifestError(f"manifest schema must be {schema}")
    if require_provenance:
        expected_keys.add("provenance")
    if not isinstance(document, dict) or set(document) != expected_keys:
        raise ManifestError("manifest has unexpected or missing top-level fields")
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


def verify_manifest(root, manifest, identity, declared, provenance=None, provenance_root=None):
    document = load_manifest(manifest)
    entries = validated_file_entries(document, require_provenance=provenance is not None)
    require_declared_file_set(entries, declared)
    mismatches = [
        field
        for field in IDENTITY_FIELDS
        if not isinstance(document[field], str) or document[field] != identity[field]
    ]
    if mismatches:
        names = ", ".join(field.replace("_", "-") for field in mismatches)
        raise ManifestError(f"manifest identity mismatch: {names}")
    if provenance is not None:
        verify_provenance(document["provenance"], provenance, identity, provenance_root)
        if identity["kind"] == "runtime":
            require_runtime_files(entries, identity["target"])

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
    argument_parser.add_argument("--provenance-root", type=Path,
                                 help="require schema 3 derived from this clean source checkout")
    argument_parser.add_argument("--build-profile", help="observed Cargo profile (schema-3 write only)")
    argument_parser.add_argument("--build-target", help="observed Cargo target triple (schema-3 write only)")
    argument_parser.add_argument("--build-feature", action="append", default=[],
                                 help="observed Cargo feature (repeatable; schema-3 write only)")
    argument_parser.add_argument("--parent-manifest", action="append", default=[], type=Path,
                                 help="verified original runtime manifest (repeat for universal packaging; write only)")
    argument_parser.add_argument("--build-run-id",
                                 help="expected producer run ID (defaults to GITHUB_RUN_ID)")
    argument_parser.add_argument("--build-run-attempt",
                                 help="verified producer's maximum attempt (verify only; requires --build-run-id)")
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
        provenance = provenance_context(absolute(args.provenance_root), identity) if args.provenance_root else None
        if args.build_profile is not None or args.build_target is not None or args.build_feature:
            if args.operation != "write" or provenance is None:
                raise ManifestError("observed build inputs require a write with --provenance-root")
        if args.parent_manifest:
            if args.operation != "write" or provenance is None:
                raise ManifestError("--parent-manifest requires a write with --provenance-root")
            if args.build_profile is not None or args.build_target is not None or args.build_feature:
                raise ManifestError("a parent transform cannot claim new compiler build inputs")
        if args.build_run_id is not None:
            if provenance is None:
                raise ManifestError("--build-run-id requires --provenance-root")
            if not (re.fullmatch(r"[1-9][0-9]*", args.build_run_id)
                    or (args.build_run_id == "local" and os.environ.get("GITHUB_ACTIONS") != "true")):
                raise ManifestError("--build-run-id must identify a real producer run")
            if args.operation == "write" and args.build_run_id != provenance["builder"]["run_id"]:
                raise ManifestError("write run ID must match the current producer")
            provenance["builder"]["run_id"] = args.build_run_id
        if args.build_run_attempt is not None:
            if provenance is None or args.build_run_id is None:
                raise ManifestError("--build-run-attempt requires --build-run-id and --provenance-root")
            if args.operation != "verify":
                raise ManifestError("--build-run-attempt is only valid for verification")
            if not re.fullmatch(r"[1-9][0-9]*", args.build_run_attempt):
                raise ManifestError("--build-run-attempt must be positive")
            provenance["builder"]["run_attempt"] = int(args.build_run_attempt)
        if args.operation == "write":
            if declared is None:
                raise ManifestError("write requires at least one --file")
            if provenance is not None:
                if args.parent_manifest:
                    provenance["producer_recipe"] = transform_recipe(
                        args.parent_manifest, identity, provenance, absolute(args.provenance_root),
                    )
                elif not args.build_profile or not args.build_target:
                    raise ManifestError("schema-3 build write requires --build-profile and --build-target")
                else:
                    provenance["producer_recipe"] = producer_recipe(
                        absolute(args.provenance_root), identity, args.build_profile, args.build_target, args.build_feature,
                    )
                verify_producer_recipe(provenance["producer_recipe"], identity, provenance, absolute(args.provenance_root))
            count = write_manifest(root, manifest, identity, declared, provenance)
            print(f"wrote {count} payload files to {manifest}")
        else:
            count = verify_manifest(root, manifest, identity, declared, provenance,
                                    provenance_root=absolute(args.provenance_root) if args.provenance_root else None)
            print(f"verified {count} payload files from {manifest}")
    except ManifestError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
