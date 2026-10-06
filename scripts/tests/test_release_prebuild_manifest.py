"""Tests for deterministic, exact-SHA release prebuild manifests."""

import hashlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

from _repo import REPOSITORY


SCRIPT = REPOSITORY / "scripts" / "release-prebuild-manifest.py"
HEAD_SHA = "1" * 40
TREE_SHA = "2" * 40
SPEC = importlib.util.spec_from_file_location("release_prebuild_manifest", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
REAL_CHECK_OUTPUT = subprocess.check_output


class ReleasePrebuildManifestTests(unittest.TestCase):
    def setUp(self):
        self.sandbox = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.sandbox, ignore_errors=True)
        self.root = self.sandbox / "artifact"
        self.root.mkdir()
        (self.root / "nested").mkdir()
        (self.root / "nested" / "first.txt").write_bytes(b"first\n")
        (self.root / "second.bin").write_bytes(b"\x00\xffsecond")
        self.manifest = self.root / "manifest.json"

    def run_manifest(self, operation, **overrides):
        files = overrides.pop("files", ("nested/first.txt", "second.bin"))
        identity = {
            "head_sha": HEAD_SHA,
            "tree_sha": TREE_SHA,
            "version": "1.2.3-rc.1",
            "kind": "runtime",
            "target": "linux",
            **overrides,
        }
        command = [
            sys.executable,
            str(SCRIPT),
            operation,
            "--root",
            str(self.root),
            "--manifest",
            str(self.manifest),
        ]
        for name, value in identity.items():
            command.extend((f"--{name.replace('_', '-')}", value))
        for path in files:
            command.extend(("--file", path))
        return subprocess.run(command, capture_output=True, text=True)

    def write_manifest(self):
        completed = self.run_manifest("write")
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def verify_manifest(self, **overrides):
        return self.run_manifest("verify", **overrides)

    def manifest_document(self):
        return json.loads(self.manifest.read_text(encoding="utf-8"))

    def replace_manifest_document(self, document):
        self.manifest.write_text(json.dumps(document), encoding="utf-8")

    def test_write_is_deterministic_and_verify_accepts_the_exact_payload(self):
        self.write_manifest()
        first_bytes = self.manifest.read_bytes()
        self.write_manifest()

        self.assertEqual(self.manifest.read_bytes(), first_bytes)
        self.assertEqual(
            json.loads(first_bytes),
            {
                "files": [
                    {
                        "path": "nested/first.txt",
                        "sha256": hashlib.sha256(b"first\n").hexdigest(),
                        "size": 6,
                    },
                    {
                        "path": "second.bin",
                        "sha256": hashlib.sha256(b"\x00\xffsecond").hexdigest(),
                        "size": 8,
                    },
                ],
                "head_sha": HEAD_SHA,
                "kind": "runtime",
                "schema": 1,
                "target": "linux",
                "tree_sha": TREE_SHA,
                "version": "1.2.3-rc.1",
            },
        )
        completed = self.verify_manifest()
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_write_rejects_payload_outside_the_declared_file_set(self):
        completed = self.run_manifest("write", files=("nested/first.txt",))

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("undeclared: second.bin", completed.stderr)

    def test_write_rejects_a_declared_file_that_is_absent(self):
        completed = self.run_manifest(
            "write",
            files=("nested/first.txt", "second.bin", "missing.txt"),
        )

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("declared but missing: missing.txt", completed.stderr)

    def test_verify_rejects_each_identity_mismatch(self):
        self.write_manifest()
        mismatches = {
            "head_sha": "3" * 40,
            "tree_sha": "4" * 40,
            "version": "1.2.4",
            "kind": "tool",
            "target": "windows",
        }

        for field, value in mismatches.items():
            with self.subTest(field=field):
                completed = self.verify_manifest(**{field: value})
                self.assertNotEqual(completed.returncode, 0)
                self.assertIn("identity mismatch", completed.stderr)
                self.assertIn(field.replace("_", "-"), completed.stderr)

    def test_verify_rejects_a_missing_file(self):
        self.write_manifest()
        (self.root / "second.bin").unlink()

        completed = self.verify_manifest()

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("missing: second.bin", completed.stderr)

    def test_verify_rejects_an_extra_file(self):
        self.write_manifest()
        (self.root / "unlisted.txt").write_text("extra", encoding="utf-8")

        completed = self.verify_manifest()

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("extra: unlisted.txt", completed.stderr)

    def test_verify_rejects_same_size_tampering(self):
        self.write_manifest()
        (self.root / "second.bin").write_bytes(b"tampered")

        completed = self.verify_manifest()

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("sha256 mismatch: second.bin", completed.stderr)

    def test_verify_rejects_a_payload_file_replaced_by_a_symlink(self):
        self.write_manifest()
        replacement = self.sandbox / "replacement.bin"
        replacement.write_bytes(b"\x00\xffsecond")
        payload = self.root / "second.bin"
        payload.unlink()
        try:
            payload.symlink_to(replacement)
        except OSError as error:
            self.skipTest(f"cannot create a symlink on this platform: {error}")

        completed = self.verify_manifest()

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("payload contains symlink: second.bin", completed.stderr)

    def test_verify_rejects_path_traversal_before_reading_outside_the_root(self):
        self.write_manifest()
        outside = self.sandbox / "outside.txt"
        outside.write_bytes(b"first\n")
        document = self.manifest_document()
        document["files"][0]["path"] = "../outside.txt"
        self.replace_manifest_document(document)

        completed = self.verify_manifest()

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("unsafe manifest path: '../outside.txt'", completed.stderr)

    def test_verify_rejects_noncanonical_or_duplicate_manifest_paths(self):
        self.write_manifest()
        for paths in (
            ["second.bin", "nested/first.txt"],
            ["nested/first.txt", "nested/first.txt"],
        ):
            with self.subTest(paths=paths):
                document = self.manifest_document()
                document["files"] = [
                    {
                        "path": path,
                        "size": 0,
                        "sha256": "0" * 64,
                    }
                    for path in paths
                ]
                self.replace_manifest_document(document)

                completed = self.verify_manifest()

                self.assertNotEqual(completed.returncode, 0)
                self.assertIn("unique and sorted", completed.stderr)
                self.write_manifest()


class ReleasePrebuildProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.sandbox = Path(self.temporary.name)
        self.source = self.sandbox / "source"
        self.source.mkdir()
        self.recipe_inputs = (
            "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
            "scripts/configure-msvc-runtime.sh", "scripts/release-prebuild-manifest.py",
            ".github/workflows/release-prebuild.yml", ".github/workflows/release-build.yml",
            ".github/workflows/device-loss-qualification.yml",
            ".github/workflows/release-platform.yml", ".github/actions/device-loss/action.yml",
            ".github/actions/verify-cache-handoff/action.yml",
            "scripts/ci-content.py", "scripts/ci-workspace-cache.py",
        )
        for name in self.recipe_inputs:
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"fixture recipe input: {name}\n")
        (self.source / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3"\n')
        (self.source / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.1"\n')
        self.git("init", "-q")
        self.git("add", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo", "scripts", ".github")
        self.git("update-index", "--add", "--cacheinfo", f"160000,{'5' * 40},content")
        self.git("-c", "user.name=Manifest Tests", "-c", "user.email=manifest@example.invalid",
                 "commit", "-q", "-m", "fixture")
        self.identity = {
            "head_sha": self.git("rev-parse", "HEAD"),
            "tree_sha": self.git("rev-parse", "HEAD^{tree}"),
            "version": "1.2.3", "kind": "runtime", "target": "x86_64-unknown-linux-gnu",
        }
        self.root = self.sandbox / "artifact"
        self.root.mkdir()
        self.install_runtime()
        self.manifest = self.root / "manifest.json"
        self.toolchain = (b"rustc 1.98.1 fixture\nhost: x86_64-unknown-linux-gnu\n"
                          b"release: 1.98.1\nLLVM version: 22.1.8\n")
        self.native_outputs = {}
        self.native_commands = []

    def git(self, *arguments):
        return REAL_CHECK_OUTPUT(["git", "-C", str(self.source), *arguments], text=True).strip()

    def install_runtime(self, target="x86_64-unknown-linux-gnu"):
        payload = self.root / "payload"
        if payload.exists():
            shutil.rmtree(payload)
        payload.mkdir()
        suffix = ".exe" if target.endswith("windows-msvc") else ""
        self.files = [f"payload/{name}{suffix}" for name in ("c4group", "clonk-app", "clonk-game")]
        for name in self.files:
            (self.root / name).write_bytes(f"runtime {name}".encode())

    def run_manifest(self, operation, *, run_id="123456", attempt="2", actions="true", provenance=True,
                     build_inputs=True, parents=(), **identity):
        arguments = [operation, "--root", str(self.root), "--manifest", str(self.manifest)]
        for name in self.files:
            arguments.extend(("--file", name))
        for path in parents:
            arguments.extend(("--parent-manifest", str(path)))
        if provenance:
            arguments.extend(("--provenance-root", str(self.source)))
            if operation == "write" and build_inputs and not parents:
                kind = identity.get("kind", self.identity["kind"])
                arguments.extend(("--build-profile", "test" if kind == "tool" else "release",
                                  "--build-target", MODULE.compiler_field(self.toolchain.decode(), "host") if kind == "tool"
                                  else identity.get("target", self.identity["target"])))
                if kind == "tool":
                    arguments.extend(("--build-feature", "engine-tools"))
        for field, value in {**self.identity, **identity}.items():
            arguments.extend((f"--{field.replace('_', '-')}", value))

        def output(command, **options):
            if command == ["rustc", "-vV"]:
                return self.toolchain
            if command[0] in self.native_outputs:
                self.native_commands.append(command)
                observed = self.native_outputs[command[0]]
                if isinstance(observed, Exception):
                    raise observed
                return observed
            return REAL_CHECK_OUTPUT(command, **options)

        environment = dict(os.environ)
        for key, value in (("GITHUB_ACTIONS", actions), ("GITHUB_RUN_ID", run_id), ("GITHUB_RUN_ATTEMPT", attempt)):
            if value is None:
                environment.pop(key, None)
            else:
                environment[key] = value
        stdout, stderr = io.StringIO(), io.StringIO()
        with (
            mock.patch.dict(os.environ, environment, clear=True),
            mock.patch.object(subprocess, "check_output", side_effect=output),
            redirect_stdout(stdout), redirect_stderr(stderr),
        ):
            code = MODULE.main(arguments)
        return code, stderr.getvalue()

    def test_production_manifest_records_and_verifies_exact_source_and_builder_provenance(self):
        code, error = self.run_manifest("write")
        self.assertEqual(code, 0, error)
        document = json.loads(self.manifest.read_text())
        self.assertEqual(document["schema"], 3)
        self.assertEqual(document["provenance"]["content_sha"], "5" * 40)
        self.assertEqual(document["provenance"]["toolchain_sha"], hashlib.sha256(self.toolchain).hexdigest())
        self.assertEqual(document["provenance"]["builder"], {"run_id": "123456", "run_attempt": 2})
        self.assertRegex(document["provenance"]["recipe_sha"], r"^[0-9a-f]{64}$")
        code, error = self.run_manifest("verify")
        self.assertEqual(code, 0, error)

    def test_producer_recipe_records_build_inputs_without_comparing_consumer_flags(self):
        with mock.patch.dict(os.environ, {"RUSTFLAGS": "-Cdebuginfo=1", "SDKROOT": "/producer/sdk",
                                          "GITHUB_TOKEN": "credential-must-not-be-recorded"}):
            code, error = self.run_manifest("write")
        self.assertEqual(code, 0, error)
        document = json.loads(self.manifest.read_text())
        self.assertEqual(document["schema"], 3)
        recipe = document["provenance"]["producer_recipe"]
        self.assertEqual((recipe["operation"], recipe["profile"], recipe["features"], recipe["target"]),
                         ("build", "release", [], "x86_64-unknown-linux-gnu"))
        self.assertEqual(recipe["environment"]["RUSTFLAGS"], "-Cdebuginfo=1")
        self.assertEqual(recipe["environment"]["SDKROOT"], "/producer/sdk")
        self.assertEqual(recipe["rustc"], self.toolchain.decode())
        self.assertNotIn("credential-must-not-be-recorded", self.manifest.read_text())
        with mock.patch.dict(os.environ, {"RUSTFLAGS": "-Cdebuginfo=0", "SDKROOT": "/consumer/sdk"}):
            code, error = self.run_manifest("verify")
        self.assertEqual(code, 0, error)

    def test_producer_recipe_rejects_missing_wrong_and_unexpected_observed_build_inputs(self):
        for flags in (
            {"build_inputs": False}, {"build_profile": "dev"},
            {"build_target": "x86_64-pc-windows-msvc"}, {"build_feature": "unshipped-feature"},
        ):
            with self.subTest(flags=flags):
                code, error = self.run_manifest("write", **flags)
                self.assertEqual(code, 1, error)
                self.assertRegex(error, "build|recipe")
        code, error = self.run_manifest("verify", build_profile="release")
        self.assertEqual(code, 1, error)
        self.assertIn("write", error)

    def test_producer_recipe_rejects_malformed_native_identity_and_incomplete_environment(self):
        self.assertEqual(self.run_manifest("write")[0], 0)
        original = self.manifest.read_text()
        mutations = (
            lambda recipe: recipe["native_tools"].update(unknown_compiler={}),
            lambda recipe: recipe["native_tools"].update(cc={"source": {}, "command": ["cc"], "version": None}),
            lambda recipe: recipe["environment"].pop("CFLAGS"),
            lambda recipe: recipe["environment"].update(GITHUB_TOKEN="secret"),
            lambda recipe: recipe.update(rustc="forged compiler identity"),
            lambda recipe: recipe.update(unknown_recipe=True),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                document = json.loads(original)
                mutate(document["provenance"]["producer_recipe"])
                self.manifest.write_text(json.dumps(document))
                code, error = self.run_manifest("verify")
                self.assertEqual(code, 1, error)
                self.assertRegex(error, "recipe|native|compiler")

    def test_production_rejects_schema_two_and_compilers_outside_the_committed_pin(self):
        self.assertEqual(self.run_manifest("write")[0], 0)
        document = json.loads(self.manifest.read_text())
        document["schema"] = 2
        self.manifest.write_text(json.dumps(document))
        code, error = self.run_manifest("verify")
        self.assertEqual(code, 1, error)
        self.assertIn("schema must be 3", error)
        self.toolchain = self.toolchain.replace(b"release: 1.98.1", b"release: 1.98.2")
        code, error = self.run_manifest("write")
        self.assertEqual(code, 1, error)
        self.assertIn("pinned", error)

    def windows_environment(self):
        self.install_runtime("x86_64-pc-windows-msvc")
        self.toolchain = self.toolchain.replace(b"host: x86_64-unknown-linux-gnu", b"host: x86_64-pc-windows-msvc")
        self.native_outputs["/fixture/rust-lld.exe"] = b"LLD 22.1.8 (compatible with MSVC link.exe)\n"
        return {
            "CARGO_BUILD_TARGET": "x86_64-pc-windows-msvc",
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER": "/fixture/rust-lld.exe",
            "CARGO_ENCODED_RUSTFLAGS": "",
            "CFLAGS_x86_64_pc_windows_msvc": "-I/fixture/zlib",
            "CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc": "/fixture/native.cmake",
            "LINK": "", "_LINK_": "",
            "RUSTFLAGS": ("-Ctarget-feature=+crt-static -Clinker-plugin-lto -Clinker-flavor=lld-link "
                          "-Clink-arg=/lldltocache:C:/fixture/cache "
                          "-Clink-arg=/lldltocachepolicy:cache_size=0%:cache_size_bytes=512m "
                          "-Clink-arg=/DEBUG:NONE -Clink-arg=/OPT:REF,ICF -Clink-arg=/TIME -Clink-arg=/Brepro"),
        }

    def test_windows_recipe_refuses_dynamic_crt_and_hidden_compiler_overrides(self):
        environment = self.windows_environment()
        environment.pop("CARGO_ENCODED_RUSTFLAGS")
        for changes in (
            {"RUSTFLAGS": environment["RUSTFLAGS"].replace("+crt-static", "-crt-static")},
            {"CARGO_ENCODED_RUSTFLAGS": "-Ctarget-feature=-crt-static"},
            {"LINK": "/DEBUG:FULL"}, {"CFLAGS_x86_64_pc_windows_msvc": ""},
            {"RUSTFLAGS": environment["RUSTFLAGS"] + " -Clinker=unrecorded-linker.exe"},
            {"CFLAGS_x86_64_pc_windows_msvc": "-I/fixture/zlib /MD"},
        ):
            with self.subTest(changes=changes), mock.patch.dict(os.environ, {**environment, **changes}):
                code, error = self.run_manifest("write", target="x86_64-pc-windows-msvc")
                self.assertEqual(code, 1, error)
                self.assertRegex(error, "MSVC|Windows")

    def test_windows_recipe_retains_verified_shipped_flags_and_the_actual_linker_identity(self):
        environment = self.windows_environment()
        environment.pop("CARGO_ENCODED_RUSTFLAGS")
        with mock.patch.dict(os.environ, environment):
            code, error = self.run_manifest("write", target="x86_64-pc-windows-msvc")
        self.assertEqual(code, 0, error)
        recipe = json.loads(self.manifest.read_text())["provenance"]["producer_recipe"]
        self.assertEqual(recipe["environment"]["CFLAGS_x86_64_pc_windows_msvc"], "-I/fixture/zlib")
        self.assertEqual(recipe["native_tools"]["linker"]["source"], "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER")
        self.assertIn("LLD 22.1.8", recipe["native_tools"]["linker"]["version"])
        code, error = self.run_manifest("verify", target="x86_64-pc-windows-msvc")
        self.assertEqual(code, 0, error)

    def test_windows_msvc_host_linker_does_not_claim_an_unselected_gnu_path_tool(self):
        gnu_link = self.sandbox / "link.exe"
        gnu_link.write_text("#!/bin/sh\nprintf 'link (GNU coreutils) fixture\\n'\n")
        gnu_link.chmod(0o755)
        target = "x86_64-pc-windows-msvc"
        environment = {name: None for name in MODULE.build_environment_names(target)}
        for ld in (None, str(gnu_link)):
            environment["LD"] = ld
            with (
                self.subTest(ld=ld),
                mock.patch.object(MODULE, "os", SimpleNamespace(name="nt", environ=os.environ)),
                mock.patch.object(MODULE.shutil, "which", side_effect=lambda name: str(gnu_link) if name == "link" else None),
            ):
                tools = MODULE.native_tool_identities(self.source, environment, target)

                self.assertIsNone(tools["linker"])

    def test_windows_host_manifest_verification_refuses_a_fabricated_default_linker_selection(self):
        self.toolchain = self.toolchain.replace(b"host: x86_64-unknown-linux-gnu", b"host: x86_64-pc-windows-msvc")
        with mock.patch.dict(os.environ, {
            "CARGO_BUILD_TARGET": "x86_64-pc-windows-msvc",
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER": "",
        }):
            code, error = self.run_manifest("write", kind="tool", target="host")
        self.assertEqual(code, 0, error)
        document = json.loads(self.manifest.read_text())
        recipe = document["provenance"]["producer_recipe"]
        self.assertIsNone(recipe["native_tools"]["linker"])
        recipe["native_tools"]["linker"] = {
            "source": "available", "command": ["C:/Git/usr/bin/link.exe"],
            "version": "link (GNU coreutils) fixture",
        }
        self.manifest.write_text(json.dumps(document))

        code, error = self.run_manifest("verify", kind="tool", target="host")
        self.assertEqual(code, 1, error)
        self.assertIn("MSVC linker selection is unproven", error)

    def test_unavailable_native_tool_is_explicitly_unknown_and_malformed_commands_fail_closed(self):
        with mock.patch.dict(os.environ, {"CC": "/fixture/compiler-not-available"}):
            code, error = self.run_manifest("write")
        self.assertEqual(code, 0, error)
        tool = json.loads(self.manifest.read_text())["provenance"]["producer_recipe"]["native_tools"]["cc"]
        self.assertEqual(tool, {"source": "CC", "command": ["/fixture/compiler-not-available"], "version": None})
        code, error = self.run_manifest("verify")
        self.assertEqual(code, 0, error)
        for value in ('"unclosed command', " "):
            with self.subTest(value=value), mock.patch.dict(os.environ, {"CC": value}):
                code, error = self.run_manifest("write")
                self.assertEqual(code, 1, error)
                self.assertIn("command", error)

    def test_available_msvc_compiler_version_is_recorded_even_when_its_version_probe_exits_two(self):
        self.native_outputs["cl.exe"] = subprocess.CalledProcessError(
            2, ["cl.exe"], output=b"Microsoft (R) C/C++ Optimizing Compiler Version 19.44.35207 for x64\n",
        )
        with mock.patch.dict(os.environ, {"CC": "cl.exe"}):
            code, error = self.run_manifest("write")
        self.assertEqual(code, 0, error)
        compiler = json.loads(self.manifest.read_text())["provenance"]["producer_recipe"]["native_tools"]["cc"]
        self.assertEqual(compiler["source"], "CC")
        self.assertIn("Compiler Version 19.44.35207", compiler["version"])

    def test_sdk_identity_queries_the_sdk_selected_by_the_actual_producer_environment(self):
        self.native_outputs["xcrun"] = b"16.3\n"
        with (
            mock.patch.object(MODULE.sys, "platform", "darwin"),
            mock.patch.dict(os.environ, {"SDKROOT": "/fixture/selected-sdk"}),
        ):
            code, error = self.run_manifest("write", target="aarch64-apple-darwin")
        self.assertEqual(code, 0, error)
        self.assertIn(["xcrun", "--sdk", "/fixture/selected-sdk", "--show-sdk-version"], self.native_commands)
        sdk = json.loads(self.manifest.read_text())["provenance"]["producer_recipe"]["native_tools"]["sdk"]
        self.assertEqual(sdk, {"root": "/fixture/selected-sdk", "version": "16.3"})

    def test_production_verification_rejects_changed_or_incomplete_provenance(self):
        self.assertEqual(self.run_manifest("write")[0], 0)
        original = self.manifest.read_text()
        for field, value in (
            ("recipe_sha", "6" * 64), ("toolchain_sha", "7" * 64),
            ("content_sha", "8" * 40), ("recipe_sha", "invalid digest"),
            ("recipe_sha", None), ("unknown_context", "unrecognized"),
            ("builder", {"run_id": "123456", "run_attempt": 2, "unknown": True}),
        ):
            with self.subTest(field=field, value=value):
                document = json.loads(original)
                if value is None:
                    del document["provenance"][field]
                else:
                    document["provenance"][field] = value
                self.manifest.write_text(json.dumps(document))
                code, error = self.run_manifest("verify")
                self.assertEqual(code, 1, error)
                self.assertRegex(error, "provenance|builder")

    def test_production_manifest_accepts_earlier_attempts_but_rejects_other_runs_or_future_attempts(self):
        self.assertEqual(self.run_manifest("write")[0], 0)
        code, error = self.run_manifest("verify", attempt="3")
        self.assertEqual(code, 0, error)
        original = self.manifest.read_text()
        for field, value in (
            ("run_id", "654321"), ("run_attempt", 3), ("run_attempt", 0),
            ("run_attempt", "2"), ("run_attempt", True),
        ):
            with self.subTest(field=field, value=value):
                document = json.loads(original)
                document["provenance"]["builder"][field] = value
                self.manifest.write_text(json.dumps(document))
                code, error = self.run_manifest("verify")
                self.assertEqual(code, 1, error)
                self.assertIn("producer", error)

    def test_production_verification_can_name_the_original_producer_without_relabeling_a_write(self):
        self.assertEqual(self.run_manifest("write")[0], 0)
        code, error = self.run_manifest("verify", run_id="654321", build_run_id="123456")
        self.assertEqual(code, 0, error)
        code, error = self.run_manifest("verify", run_id="654321")
        self.assertEqual(code, 1, error)
        self.assertIn("producer run ID mismatch", error)
        code, error = self.run_manifest("write", build_run_id="654321")
        self.assertEqual(code, 1, error)
        self.assertIn("match the current producer", error)

    def test_production_provenance_rejects_wrong_checkout_identity_and_dirty_source(self):
        for field, value in (("head_sha", "3" * 40), ("tree_sha", "4" * 40), ("version", "1.2.4")):
            with self.subTest(field=field):
                code, error = self.run_manifest("write", **{field: value})
                self.assertEqual(code, 1, error)
                self.assertIn("source identity mismatch", error)
        cargo = self.source / "Cargo.toml"
        original = cargo.read_bytes()
        cargo.write_bytes(original + b"# changed source\n")
        code, error = self.run_manifest("write")
        self.assertEqual(code, 1, error)
        self.assertIn("committed source inputs", error)
        cargo.write_bytes(original)
        (self.source / "new_source.rs").write_text("// untracked source\n")
        code, error = self.run_manifest("write")
        self.assertEqual(code, 1, error)
        self.assertIn("committed source inputs", error)

    def test_production_verification_refuses_legacy_manifests_and_a_changed_compiler(self):
        MODULE.write_manifest(self.root, self.manifest, self.identity, self.files)
        code, error = self.run_manifest("verify")
        self.assertEqual(code, 1, error)
        self.assertIn("schema must be 3", error)
        self.assertEqual(self.run_manifest("write")[0], 0)
        self.toolchain += b"LLVM version: changed\n"
        code, error = self.run_manifest("verify")
        self.assertEqual(code, 1, error)
        self.assertIn("toolchain_sha", error)

    def test_recipe_hash_pins_the_target_profile_features_tree_and_all_committed_recipe_inputs(self):
        for kind, target, profile, features in (
            ("runtime", "x86_64-unknown-linux-gnu", "release", []),
            ("tool", "host", "test", ["engine-tools"]),
        ):
            with self.subTest(kind=kind):
                code, error = self.run_manifest("write", kind=kind, target=target)
                self.assertEqual(code, 0, error)
                recipe = {
                    "kind": kind, "target": target, "profile": profile, "features": features,
                    "tree_sha": self.identity["tree_sha"],
                    "sources": {name: hashlib.sha256((self.source / name).read_bytes()).hexdigest()
                                for name in self.recipe_inputs},
                }
                encoded = json.dumps(recipe, sort_keys=True, separators=(",", ":")).encode()
                document = json.loads(self.manifest.read_text())
                self.assertEqual(document["provenance"]["recipe_sha"], hashlib.sha256(encoded).hexdigest())

    def test_github_provenance_requires_real_producer_metadata_while_local_fallback_is_explicit(self):
        for missing in ({"run_id": None}, {"attempt": None}, {"run_id": "local"}):
            with self.subTest(missing=missing):
                code, error = self.run_manifest("write", **missing)
                self.assertEqual(code, 1, error)
                self.assertIn("GITHUB_RUN_", error)
        code, error = self.run_manifest("write", actions=None, run_id=None, attempt=None)
        self.assertEqual(code, 0, error)
        document = json.loads(self.manifest.read_text())
        self.assertEqual(document["provenance"]["builder"], {"run_id": "local", "run_attempt": 1})
        code, error = self.run_manifest("verify", actions=None, run_id=None, attempt=None)
        self.assertEqual(code, 0, error)

    def test_producer_override_must_be_a_nonempty_real_run_id(self):
        for value in ("", "0", "unrecognized", "local"):
            with self.subTest(value=value):
                code, error = self.run_manifest("write", build_run_id=value)
                self.assertEqual(code, 1, error)
                self.assertIn("real producer run", error)

    def test_cross_run_verification_uses_the_validated_producer_attempt_instead_of_the_consumer_attempt(self):
        self.assertEqual(self.run_manifest("write", attempt="2")[0], 0)
        code, error = self.run_manifest("verify", run_id="654321", attempt="1", build_run_id="123456")
        self.assertEqual(code, 1, error)
        self.assertIn("producer attempt", error)
        code, error = self.run_manifest(
            "verify", run_id="654321", attempt="1", build_run_id="123456", build_run_attempt="2",
        )
        self.assertEqual(code, 0, error)
        self.assertEqual(self.run_manifest("write", attempt="1")[0], 0)
        code, error = self.run_manifest(
            "verify", run_id="654321", attempt="1", build_run_id="123456", build_run_attempt="2",
        )
        self.assertEqual(code, 0, error)

    def test_explicit_producer_attempt_rejects_malformed_limits_and_artifacts_from_future_attempts(self):
        self.assertEqual(self.run_manifest("write", attempt="2")[0], 0)
        for value in ("", "0", "-1", "2.5", "unknown"):
            with self.subTest(attempt=value):
                code, error = self.run_manifest("verify", build_run_id="123456", build_run_attempt=value)
                self.assertEqual(code, 1, error)
                self.assertIn("must be positive", error)
        self.assertEqual(self.run_manifest("write", attempt="3")[0], 0)
        code, error = self.run_manifest(
            "verify", run_id="654321", attempt="1", build_run_id="123456", build_run_attempt="2",
        )
        self.assertEqual(code, 1, error)
        self.assertIn("producer attempt", error)

    def test_explicit_producer_attempt_requires_named_provenance_and_cannot_relabel_a_write(self):
        self.assertEqual(self.run_manifest("write")[0], 0)
        original = self.manifest.read_bytes()
        for operation, provenance, flags, diagnosis in (
            ("verify", True, {"build_run_attempt": "2"}, "requires"),
            ("verify", False, {"build_run_id": "123456", "build_run_attempt": "2"}, "requires"),
            ("write", True, {"build_run_id": "123456", "build_run_attempt": "2"}, "only valid for verification"),
        ):
            with self.subTest(operation=operation, provenance=provenance):
                code, error = self.run_manifest(operation, provenance=provenance, **flags)
                self.assertEqual(code, 1, error)
                self.assertIn(diagnosis, error)
                self.assertEqual(self.manifest.read_bytes(), original)

    def parent_manifest(self, target, *, attempt="2", run_id="123456", flags="-Cdebuginfo=1"):
        original = self.root, self.manifest, self.files
        try:
            self.root = self.sandbox / f"parent-{len(list(self.sandbox.glob('parent-*')))}"
            self.root.mkdir()
            self.install_runtime(target)
            self.manifest = self.root / "manifest.json"
            with mock.patch.dict(os.environ, {"RUSTFLAGS": flags}):
                code, error = self.run_manifest("write", target=target, attempt=attempt, run_id=run_id)
            self.assertEqual(code, 0, error)
            return self.manifest
        finally:
            self.root, self.manifest, self.files = original

    def test_universal_packaging_retains_exact_verified_parent_metadata_without_inventing_build_flags(self):
        parents = [self.parent_manifest("aarch64-apple-darwin", attempt="1", flags="-Cdebuginfo=1"),
                   self.parent_manifest("x86_64-apple-darwin", flags="-Cdebuginfo=0")]
        originals = [path.read_bytes() for path in parents]
        code, error = self.run_manifest("write", target="universal-apple-darwin", parents=parents)
        self.assertEqual(code, 0, error)
        document = json.loads(self.manifest.read_text())
        recipe = document["provenance"]["producer_recipe"]
        self.assertEqual(recipe["operation"], "transform")
        self.assertEqual(recipe["target"], "universal-apple-darwin")
        for field in ("profile", "features", "environment", "rustc", "native_tools"):
            self.assertIsNone(recipe[field])
        self.assertEqual(len(recipe["parents"]), 2)
        for entry, original in zip(recipe["parents"], originals, strict=True):
            self.assertEqual(entry["manifest_json"].encode(), original)
            self.assertEqual(entry["sha256"], hashlib.sha256(original).hexdigest())
        retained = [json.loads(entry["manifest_json"]) for entry in recipe["parents"]]
        self.assertEqual([parent["provenance"]["builder"]["run_attempt"] for parent in retained], [1, 2])
        self.assertEqual([parent["provenance"]["producer_recipe"]["environment"]["RUSTFLAGS"] for parent in retained],
                         ["-Cdebuginfo=1", "-Cdebuginfo=0"])
        for path in parents:
            shutil.rmtree(path.parent)
        code, error = self.run_manifest("verify", target="universal-apple-darwin", run_id="654321", attempt="1",
                                        build_run_id="123456", build_run_attempt="2")
        self.assertEqual(code, 0, error)

    def test_native_packaging_retains_one_original_parent_and_authenticates_its_producer(self):
        parent = self.parent_manifest("x86_64-unknown-linux-gnu", attempt="1")
        code, error = self.run_manifest("write", parents=[parent])
        self.assertEqual(code, 0, error)
        document = json.loads(self.manifest.read_text())
        self.assertEqual(document["provenance"]["builder"], {"run_id": "123456", "run_attempt": 2})
        retained = json.loads(document["provenance"]["producer_recipe"]["parents"][0]["manifest_json"])
        self.assertEqual(retained["provenance"]["builder"], {"run_id": "123456", "run_attempt": 1})
        self.assertEqual(self.run_manifest("verify")[0], 0)
        for flags in ({"run_id": "654321"}, {"attempt": "3"}):
            invalid = self.parent_manifest("x86_64-unknown-linux-gnu", **flags)
            code, error = self.run_manifest("write", parents=[invalid])
            self.assertEqual(code, 1, error)
            self.assertIn("producer", error)

    def test_windows_packaging_verifies_original_static_crt_inputs_without_reusing_consumer_flags(self):
        environment = self.windows_environment()
        environment.pop("CARGO_ENCODED_RUSTFLAGS")
        with mock.patch.dict(os.environ, environment):
            parent = self.parent_manifest("x86_64-pc-windows-msvc", flags=environment["RUSTFLAGS"])
        code, error = self.run_manifest("write", target="x86_64-pc-windows-msvc", parents=[parent])
        self.assertEqual(code, 0, error)
        code, error = self.run_manifest("verify", target="x86_64-pc-windows-msvc")
        self.assertEqual(code, 0, error)
        retained = json.loads(json.loads(self.manifest.read_text())["provenance"]["producer_recipe"]["parents"][0]["manifest_json"])
        self.assertEqual(retained["provenance"]["producer_recipe"]["environment"]["RUSTFLAGS"], environment["RUSTFLAGS"])

    def test_packaging_refuses_incomplete_duplicate_wrong_platform_and_nonruntime_parents(self):
        arm = self.parent_manifest("aarch64-apple-darwin")
        x86 = self.parent_manifest("x86_64-apple-darwin")
        for target, parents, extra in (
            ("universal-apple-darwin", [arm], {}),
            ("universal-apple-darwin", [arm, arm], {}),
            ("x86_64-unknown-linux-gnu", [arm], {}),
            ("host", [arm], {"kind": "tool"}),
        ):
            with self.subTest(target=target, parents=parents):
                code, error = self.run_manifest("write", target=target, parents=parents, **extra)
                self.assertEqual(code, 1, error)
                self.assertRegex(error, "parent|runtime")
        code, error = self.run_manifest("verify", target="universal-apple-darwin", parents=[arm, x86])
        self.assertEqual(code, 1, error)
        self.assertIn("write", error)
        code, error = self.run_manifest("write", target="universal-apple-darwin", parents=[arm, x86],
                                        build_profile="release", build_target="aarch64-apple-darwin")
        self.assertEqual(code, 1, error)
        self.assertIn("new compiler", error)

    def test_packaging_refuses_changed_parent_bytes_source_schema_and_runtime_inventory(self):
        parent = self.parent_manifest("x86_64-unknown-linux-gnu")
        original = parent.read_text()
        payload = parent.parent / "payload/clonk-app"
        payload.write_bytes(b"damaged runtime")
        code, error = self.run_manifest("write", parents=[parent])
        self.assertEqual(code, 1, error)
        self.assertIn("payload", error)
        payload.write_bytes(b"runtime payload/clonk-app")
        for mutate in (
            lambda document: document.update(head_sha="9" * 40),
            lambda document: document.update(tree_sha="8" * 40),
            lambda document: document.update(version="1.2.4"),
            lambda document: document.update(kind="tool"),
            lambda document: document.update(schema=2),
            lambda document: document["files"].pop(),
        ):
            with self.subTest(mutate=mutate):
                document = json.loads(original)
                mutate(document)
                parent.write_text(json.dumps(document))
                code, error = self.run_manifest("write", parents=[parent])
                self.assertEqual(code, 1, error)
                self.assertRegex(error, "parent|manifest|declared")

    def test_transformed_verification_rejects_forged_parent_metadata_digest_and_future_attempts(self):
        parent = self.parent_manifest("x86_64-unknown-linux-gnu")
        self.assertEqual(self.run_manifest("write", parents=[parent])[0], 0)
        original = self.manifest.read_text()
        for mutation in ("digest", "unknown", "profile", "attempt", "source", "transformed-parent"):
            with self.subTest(mutation=mutation):
                document = json.loads(original)
                entry = document["provenance"]["producer_recipe"]["parents"][0]
                if mutation == "digest":
                    entry["sha256"] = "0" * 64
                elif mutation == "unknown":
                    entry["unknown_field"] = True
                else:
                    retained = json.loads(entry["manifest_json"])
                    if mutation == "profile":
                        retained["provenance"]["producer_recipe"]["profile"] = "dev"
                    elif mutation == "attempt":
                        retained["provenance"]["builder"]["run_attempt"] = 3
                    elif mutation == "source":
                        retained["head_sha"] = "9" * 40
                    else:
                        retained["provenance"]["producer_recipe"]["operation"] = "transform"
                    entry["manifest_json"] = json.dumps(retained)
                    entry["sha256"] = hashlib.sha256(entry["manifest_json"].encode()).hexdigest()
                self.manifest.write_text(json.dumps(document))
                code, error = self.run_manifest("verify", attempt="3")
                self.assertEqual(code, 1, error)
                self.assertRegex(error, "parent|producer|recipe")

    def test_production_runtime_inventory_refuses_empty_missing_and_additional_shipped_binaries(self):
        (self.root / "payload/clonk-app").write_bytes(b"")
        code, error = self.run_manifest("write")
        self.assertEqual(code, 1, error)
        self.assertIn("empty", error)
        self.install_runtime()
        self.files.pop()
        (self.root / "payload/clonk-game").unlink()
        code, error = self.run_manifest("write")
        self.assertEqual(code, 1, error)
        self.assertIn("missing", error)
        self.install_runtime()
        (self.root / "payload/extra").write_bytes(b"unshipped")
        self.files.append("payload/extra")
        code, error = self.run_manifest("write")
        self.assertEqual(code, 1, error)
        self.assertIn("undeclared", error)


if __name__ == "__main__":
    unittest.main()
