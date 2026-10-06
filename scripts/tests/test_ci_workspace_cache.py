from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "ci-workspace-cache.py"


@unittest.skipUnless(shutil.which("cargo") and shutil.which("rustc"), "needs Cargo and rustc")
class WorkspaceCacheTests(unittest.TestCase):
    def setUp(self) -> None:
        self.sandbox = Path(tempfile.mkdtemp(prefix="clonk-ci-workspace-cache-"))
        self.addCleanup(shutil.rmtree, self.sandbox, ignore_errors=True)
        self.root = self.sandbox / "source"
        self.root.mkdir()
        self.target = self.sandbox / "target"
        self.environment = {
            key: value for key, value in os.environ.items()
            if not key.startswith(("CARGO_", "GIT_", "RUST")) or key == "CARGO_HOME"
        }
        self.environment.update({
            "CARGO_INCREMENTAL": "0", "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
        })
        self.git("init", "--quiet")
        self.write("Cargo.toml", '[workspace]\nmembers = ["base", "consumer"]\nresolver = "2"\n')
        self.write("base/Cargo.toml", '\n'.join((
            '[package]', 'name = "cache-base"', 'version = "0.1.0"', 'edition = "2021"',
            '[features]', 'alternate = []', '',
        )))
        self.write("base/src/lib.rs", "pub fn value() -> u32 { 1 }\n")
        self.write("consumer/Cargo.toml", '\n'.join((
            '[package]', 'name = "cache-consumer"', 'version = "0.1.0"', 'edition = "2021"',
            '[dependencies]', 'cache-base = { path = "../base" }', '',
        )))
        self.write("consumer/src/main.rs", 'fn main() { println!("{}", cache_base::value()); }\n')
        self.cargo("generate-lockfile")
        self.git("add", "Cargo.toml", "Cargo.lock", "base", "consumer")

    def write(self, name: str, contents: str) -> Path:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding="utf-8")
        return path

    def git(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(self.root), *arguments], env=self.environment,
            check=True, capture_output=True, text=True, timeout=15,
        )

    def cargo(self, command: str = "build", *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["cargo", command, "--offline", *arguments], cwd=self.root,
            env=self.environment, check=True, capture_output=True, text=True, timeout=30,
        )

    def build(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return self.cargo("build", "--workspace", "--target-dir", str(self.target), *arguments)

    def value(self) -> str:
        return subprocess.run(
            [str(self.target / "debug" / "cache-consumer")],
            check=True, capture_output=True, text=True, timeout=5,
        ).stdout.strip()

    def cache(self, operation: str, *arguments: str) -> dict:
        completed = self.cache_command(operation, *arguments)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        return json.loads(completed.stdout)

    def cache_command(self, operation: str, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), operation, "--root", str(self.root),
             "--target", str(self.target), *arguments],
            cwd=self.root, env=self.environment, check=False,
            capture_output=True, text=True, timeout=15,
        )

    def fingerprints(self, package: str) -> list[Path]:
        return sorted((self.target / "debug" / ".fingerprint").glob(f"{package}-*"))

    def test_prepare_outputs_warm_only_for_verified_units_without_invalidations(self) -> None:
        self.build()
        self.cache("record")
        output = self.sandbox / "github-output"
        output.write_text("existing=retained\n", encoding="utf-8")

        report = self.cache("prepare", "--github-output", str(output))

        self.assertEqual(len(report["reused_units"]), 2)
        self.assertFalse(report["invalidated_units"])
        self.assertEqual(output.read_text(encoding="utf-8"),
                         "existing=retained\ncache-state=warm\nreused-units=2\ninvalidated-units=0\n")

    def test_prepare_outputs_cold_for_valid_empty_or_missing_cache_and_environment_output(self) -> None:
        output = self.sandbox / "github-output"
        self.environment["GITHUB_OUTPUT"] = str(output)

        empty = self.cache("prepare", "--github-output")

        self.assertFalse(empty["reused_units"])
        self.assertEqual(output.read_text(encoding="utf-8"),
                         "cache-state=cold\nreused-units=0\ninvalidated-units=0\n")
        output.unlink()
        self.build()
        missed = self.cache("prepare", "--github-output")
        self.assertFalse(missed["cache_verified"])
        self.assertFalse(missed["reused_units"])
        self.assertEqual(output.read_text(encoding="utf-8"),
                         "cache-state=cold\nreused-units=0\ninvalidated-units=2\n")

    def test_prepare_mixed_reuse_and_invalidation_outputs_unknown(self) -> None:
        self.build()
        self.cache("record")
        unknown = self.target / "debug" / ".fingerprint" / "cache-base-00000000000000aa"
        unknown.mkdir()
        (unknown / "invoked.timestamp").write_text("unrecorded", encoding="utf-8")
        output = self.sandbox / "github-output"

        report = self.cache("prepare", "--github-output", str(output))

        self.assertEqual(len(report["reused_units"]), 2)
        self.assertEqual(len(report["invalidated_units"]), 1)
        self.assertEqual(output.read_text(encoding="utf-8"),
                         "cache-state=unknown\nreused-units=2\ninvalidated-units=1\n")

    def test_output_receipts_refuse_unsafe_paths_before_artifact_mutation(self) -> None:
        self.build()
        self.cache("record")
        output = self.sandbox / "github-output"
        output.write_text("existing=retained\n", encoding="utf-8")
        link = self.sandbox / "output-link"
        link.symlink_to(output)
        parent = self.sandbox / "output-parent-link"
        parent.symlink_to(self.sandbox, target_is_directory=True)
        for path in (link, parent / output.name, self.sandbox,
                     self.sandbox / "missing" / "output", self.root / "Cargo.toml",
                     self.target / "debug" / "cache-consumer", self.root / ".git" / "output"):
            with self.subTest(path=path):
                completed = self.cache_command("prepare", "--github-output", str(path))
                self.assertEqual(completed.returncode, 1, completed.stdout)
                self.assertEqual(output.read_text(encoding="utf-8"), "existing=retained\n")
                self.assertTrue(self.fingerprints("cache-base"))
                self.assertEqual(self.value(), "1")

    def test_record_and_failed_prepare_do_not_publish_output_receipts(self) -> None:
        self.build()
        output = self.sandbox / "github-output"
        output.write_text("existing=retained\n", encoding="utf-8")
        record = self.cache_command("record", "--github-output", str(output))
        self.assertEqual(record.returncode, 1, record.stdout)
        self.assertIn("require prepare", record.stderr)
        self.assertFalse((self.target / ".ci-workspace-inputs.json").exists())
        self.write("Cargo.toml", "not a Cargo manifest")

        failed = self.cache_command("prepare", "--github-output", str(output))

        self.assertEqual(failed.returncode, 1, failed.stdout)
        self.assertEqual(failed.stdout, "")
        self.assertEqual(output.read_text(encoding="utf-8"), "existing=retained\n")

    def rust_provider_paths(self) -> tuple[Path, str]:
        rustup = shutil.which("rustup", path=self.environment["PATH"])
        if not rustup or os.name != "posix":
            self.skipTest("needs real Rustup shims on POSIX")
        sysroot = Path(subprocess.run(
            ["rustc", "--print", "sysroot"], cwd=self.root, env=self.environment,
            check=True, capture_output=True, text=True, timeout=15,
        ).stdout.strip())
        shim = self.sandbox / "rust-shims"
        shim.mkdir()
        # Real Rustup proxy dispatch, including the less common rust-gdbgui
        # alias omitted by some distribution-installed Rustup packages.
        for tool in (sysroot / "bin").iterdir():
            (shim / tool.name).symlink_to(Path(rustup).resolve())
        (shim / "rustup").symlink_to(Path(rustup).resolve())
        return sysroot / "bin", str(shim) + os.pathsep + self.environment["PATH"]

    def test_equivalent_rust_sysroot_and_shims_reuse_actual_cargo_units(self) -> None:
        provider, shim_path = self.rust_provider_paths()
        self.environment["PATH"] = str(provider) + os.pathsep + shim_path
        self.build()
        self.cache("record")
        artifact = self.target / "debug" / "cache-consumer"
        built_at = artifact.stat().st_mtime_ns
        for name in ("Cargo.toml", "Cargo.lock", "base/Cargo.toml", "base/src/lib.rs",
                     "consumer/Cargo.toml", "consumer/src/main.rs"):
            os.utime(self.root / name, ns=(built_at + 100_000_000_000,) * 2)
        self.environment["PATH"] = shim_path

        report = self.cache("prepare")
        completed = self.build()

        self.assertTrue(report["cache_verified"], report["reason"])
        self.assertEqual(len(report["reused_units"]), 2)
        self.assertFalse(report["invalidated_units"])
        self.assertNotIn("Compiling", completed.stderr)
        self.assertEqual(artifact.stat().st_mtime_ns, built_at)
        self.assertEqual(self.value(), "1")

    def test_equivalent_rust_paths_still_rebuild_backdated_source_edits(self) -> None:
        provider, shim_path = self.rust_provider_paths()
        self.environment["PATH"] = str(provider) + os.pathsep + shim_path
        self.build()
        self.cache("record")
        changed = self.write("base/src/lib.rs", "pub fn value() -> u32 { 2 }\n")
        os.utime(changed, (1, 1))
        self.environment["PATH"] = shim_path
        self.build()
        self.assertEqual(self.value(), "1")

        report = self.cache("prepare")
        completed = self.build()

        self.assertTrue(report["cache_verified"], report["reason"])
        self.assertTrue(report["invalidated_units"])
        self.assertIn("Compiling cache-base", completed.stderr)
        self.assertIn("Compiling cache-consumer", completed.stderr)
        self.assertEqual(self.value(), "2")

    def test_unproven_rust_tool_fallback_keeps_sysroot_path_conservative(self) -> None:
        provider, shim_path = self.rust_provider_paths()
        # Hide one real provider tool. This search-path change must not become
        # equivalent merely because rustc and Cargo still print the same version.
        (self.sandbox / "rust-shims" / "rustfmt").unlink()
        unknown = self.sandbox / "rust-shims" / "rustfmt"
        unknown.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        unknown.chmod(0o755)
        self.environment["PATH"] = str(provider) + os.pathsep + shim_path
        self.build()
        self.cache("record")
        self.environment["PATH"] = shim_path

        report = self.cache("prepare")

        # Some machines also expose equivalent Rustup shims later in PATH.
        # A different executable early in PATH is always an unproven provider.
        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["reused_units"])

    def test_same_version_compiler_wrapper_byte_changes_invalidate_cached_units(self) -> None:
        provider, shim_path = self.rust_provider_paths()
        wrappers = self.sandbox / "compiler-wrappers"
        wrappers.mkdir()
        compiler = wrappers / "rustc"
        compiler.write_text(f'#!/bin/sh\nexec {shlex.quote(str(provider / "rustc"))} "$@"\n', encoding="utf-8")
        compiler.chmod(0o755)
        self.environment["PATH"] = str(wrappers) + os.pathsep + shim_path
        self.build()
        self.cache("record")
        compiler.write_text(compiler.read_text(encoding="utf-8") + "# replaced compiler wrapper\n", encoding="utf-8")

        report = self.cache("prepare")

        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["reused_units"])

    def test_same_path_native_compiler_byte_changes_invalidate_cached_units(self) -> None:
        compiler = shutil.which("cc", path=self.environment["PATH"])
        if not compiler or os.name != "posix":
            self.skipTest("needs a real native C compiler on POSIX")
        wrappers = self.sandbox / "native-compilers"
        wrappers.mkdir()
        native = wrappers / "cc"
        native.write_text(f'#!/bin/sh\nexec {shlex.quote(compiler)} "$@"\n', encoding="utf-8")
        native.chmod(0o755)
        self.environment["PATH"] = str(wrappers) + os.pathsep + self.environment["PATH"]
        self.build()
        self.cache("record")
        native.write_text(native.read_text(encoding="utf-8") + "# replaced native compiler\n", encoding="utf-8")

        report = self.cache("prepare")

        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["reused_units"])

    def test_external_ledger_survives_cache_action_cleanup_and_reuses_restored_unchanged_artifacts(self) -> None:
        self.build()
        recipe = "cargo build --workspace"
        self.cache("record", "--recipe", recipe)
        ledger = self.root / ".ci-cache-ledgers" / "landing.json"
        self.cache("record", "--recipe", recipe, "--ledger", str(ledger))
        recorded = ledger.read_bytes()
        artifact = self.target / "debug" / "cache-consumer"
        built_at = artifact.stat().st_mtime_ns

        # Pinned rust-cache cleanTargetDir removes every target-root file other
        # than CACHEDIR.TAG before saving, including the legacy ledger location.
        for path in self.target.iterdir():
            if path.is_file() and path.name != "CACHEDIR.TAG":
                path.unlink()
        self.assertFalse((self.target / ".ci-workspace-inputs.json").exists())
        self.assertEqual(ledger.read_bytes(), recorded)
        for name in ("Cargo.toml", "Cargo.lock", "base/Cargo.toml", "base/src/lib.rs",
                     "consumer/Cargo.toml", "consumer/src/main.rs"):
            os.utime(self.root / name, ns=(built_at + 100_000_000_000,) * 2)

        report = self.cache("prepare", "--recipe", recipe, "--ledger", str(ledger))
        completed = self.build()

        self.assertTrue(report["cache_verified"])
        self.assertFalse(report["invalidated_units"])
        self.assertEqual(len(report["reused_units"]), 2)
        self.assertNotIn("Compiling", completed.stderr)
        self.assertEqual(artifact.stat().st_mtime_ns, built_at)
        self.assertEqual(self.value(), "1")

    def test_missing_or_malformed_explicit_ledgers_invalidate_workspace_units_and_keep_dependencies(self) -> None:
        ledger = self.root / ".ci-cache-ledgers" / "landing.json"
        for contents in (None, "invalid json", '{"schema_version": 9000}'):
            with self.subTest(ledger=contents):
                self.build()
                self.cache("record", "--ledger", str(ledger))
                if contents is None:
                    ledger.unlink()
                else:
                    ledger.write_text(contents, encoding="utf-8")
                dependency = self.target / "debug" / ".fingerprint" / "serde-00000000000000bb"
                dependency.mkdir(exist_ok=True)
                (dependency / "invoked.timestamp").write_text("retain dependency", encoding="utf-8")

                report = self.cache("prepare", "--ledger", str(ledger))

                self.assertFalse(report["cache_verified"])
                self.assertFalse(report["reused_units"])
                self.assertFalse(self.fingerprints("cache-base"))
                self.assertFalse(self.fingerprints("cache-consumer"))
                self.assertTrue(dependency.exists())

    def test_ledger_paths_cannot_overwrite_source_metadata_content_or_external_files(self) -> None:
        self.build()
        self.cache("record")
        source = (self.root / "base" / "Cargo.toml").read_bytes()
        outside = self.sandbox / "outside-ledger.json"
        outside.write_text("retain external file", encoding="utf-8")
        default = (self.target / ".ci-workspace-inputs.json").read_bytes()
        for path in (
            self.root / "Cargo.toml", self.root / "base" / "Cargo.toml",
            self.root / "base" / "src" / "new-cache" / "ledger.json",
            self.root / ".git" / "cache" / "ledger.json",
            self.root / "content" / "cache" / "ledger.json", outside,
            Path(".ci-cache-ledgers/../base/ledger.json"),
        ):
            for operation in ("prepare", "record"):
                with self.subTest(path=path, operation=operation):
                    completed = self.cache_command(operation, "--ledger", str(path))
                    self.assertEqual(completed.returncode, 1, completed.stdout)
                    self.assertIn("ledger", completed.stderr)
                    self.assertEqual((self.root / "base" / "Cargo.toml").read_bytes(), source)
                    self.assertEqual(outside.read_text(encoding="utf-8"), "retain external file")
                    self.assertEqual((self.target / ".ci-workspace-inputs.json").read_bytes(), default)
                    self.assertTrue(self.fingerprints("cache-base"))
                    self.assertTrue(self.fingerprints("cache-consumer"))

    def test_symlinked_ledger_files_and_parents_refuse_without_touching_artifacts_or_external_files(self) -> None:
        self.build()
        self.cache("record")
        outside = self.sandbox / "external-ledger.json"
        outside.write_text("retain external ledger", encoding="utf-8")
        metadata = self.root / ".ci-cache-ledgers"
        metadata.mkdir()
        leaf = metadata / "landing.json"
        leaf.symlink_to(outside)
        parent = self.root / ".ci-linked-ledgers"
        parent.symlink_to(self.sandbox, target_is_directory=True)
        for path in (leaf, parent / "external-ledger.json"):
            for operation in ("prepare", "record"):
                with self.subTest(path=path, operation=operation):
                    completed = self.cache_command(operation, "--ledger", str(path))
                    self.assertEqual(completed.returncode, 1, completed.stdout)
                    self.assertIn("symlink", completed.stderr)
                    self.assertEqual(outside.read_text(encoding="utf-8"), "retain external ledger")
                    self.assertTrue(self.fingerprints("cache-base"))
        default = self.target / ".ci-workspace-inputs.json"
        default.unlink()
        default.symlink_to(outside)
        completed = self.cache_command("prepare")
        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("symlink", completed.stderr)
        self.assertEqual(outside.read_text(encoding="utf-8"), "retain external ledger")
        self.assertTrue(self.fingerprints("cache-base"))

    def test_failed_atomic_ledger_publication_keeps_previous_proof_and_removes_temporary_files(self) -> None:
        self.build()
        ledger = self.root / ".ci-cache-ledgers" / "landing.json"
        self.cache("record", "--ledger", ".ci-cache-ledgers/landing.json")
        original = ledger.read_bytes()
        spec = importlib.util.spec_from_file_location("workspace_cache_atomic_test", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        with (
            mock.patch.object(module.os, "replace", side_effect=OSError("publication failed")),
            self.assertRaisesRegex(OSError, "publication failed"),
        ):
            module.write_ledger(self.target, {"new": "unpublished proof"}, ledger)

        self.assertEqual(ledger.read_bytes(), original)
        self.assertEqual(list(ledger.parent.glob("landing.json.*")), [])

    def test_default_ledgers_cannot_write_into_content_or_tracked_source_namespaces(self) -> None:
        for target in (self.root / "content" / "cache", self.root / "base" / "src" / "cache"):
            with self.subTest(target=target):
                completed = self.cache_command("record", "--target", str(target))
                self.assertEqual(completed.returncode, 1, completed.stdout)
                self.assertIn("ledger", completed.stderr)
                self.assertFalse(target.exists())

    def test_backdated_source_edit_rebuilds_the_package_and_its_dependent(self) -> None:
        self.build()
        self.assertEqual(self.value(), "1")
        self.cache("record")
        edited = self.write("base/src/lib.rs", "pub fn value() -> u32 { 2 }\n")
        os.utime(edited, (1, 1))

        # Demonstrate Cargo's mtime-based false hit before applying the ledger.
        self.build()
        self.assertEqual(self.value(), "1")
        report = self.cache("prepare")

        self.assertFalse(self.fingerprints("cache-base"))
        self.assertTrue(report["invalidated_units"])
        self.build()
        self.assertEqual(self.value(), "2")

    def test_cross_package_include_invalidates_the_actual_compiler_input_user(self) -> None:
        shared = self.write("base/src/shared.rs", "fn external_value() -> u32 { 7 }\n")
        self.write("consumer/src/main.rs", '\n'.join((
            'include!("../../base/src/shared.rs");',
            'fn main() { println!("{}", external_value()); }', '',
        )))
        self.git("add", "base/src/shared.rs", "consumer/src/main.rs")
        self.build()
        self.cache("record")
        shared.write_text("fn external_value() -> u32 { 9 }\n", encoding="utf-8")
        os.utime(shared, (1, 1))
        self.build()
        self.assertEqual(self.value(), "7")

        self.cache("prepare")

        self.assertFalse(self.fingerprints("cache-consumer"))
        self.build()
        self.assertEqual(self.value(), "9")

    def test_restored_test_and_coverage_results_are_discarded_before_a_new_run(self) -> None:
        self.build()
        self.cache("record")
        results = (
            "nextest/default/junit.xml", "coverage/html/index.html", "coverage.lcov",
            "debug/a.profraw", "debug/b.profdata", "debug/junit-windows.xml",
            "debug/cobertura.xml", "lcov.info",
        )
        for name in results:
            path = self.target / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("stale result", encoding="utf-8")
        artifact = self.target / "debug" / "cache-consumer"
        original = artifact.read_bytes()

        self.cache("prepare")

        self.assertTrue(all(not (self.target / name).exists() for name in results))
        self.assertEqual(artifact.read_bytes(), original)

    def test_identical_source_with_new_checkout_mtimes_reuses_actual_cargo_units(self) -> None:
        self.build()
        self.cache("record", "--recipe", "cargo build --workspace")
        artifact = self.target / "debug" / "cache-consumer"
        built_at = artifact.stat().st_mtime_ns
        for name in ("Cargo.toml", "Cargo.lock", "base/Cargo.toml", "base/src/lib.rs",
                     "consumer/Cargo.toml", "consumer/src/main.rs"):
            os.utime(self.root / name, ns=(built_at + 100_000_000_000, built_at + 100_000_000_000))
        os.utime(self.root / "base" / "src", ns=(built_at + 100_000_000_000,) * 2)

        report = self.cache("prepare", "--recipe", "cargo build --workspace")
        completed = self.build()

        self.assertTrue(report["cache_verified"])
        self.assertFalse(report["invalidated_units"])
        self.assertEqual(len(report["reused_units"]), 2)
        self.assertNotIn("Compiling", completed.stderr)
        self.assertEqual(artifact.stat().st_mtime_ns, built_at)
        self.assertEqual(self.value(), "1")

    def test_misses_and_unknown_units_keep_third_party_fingerprints_in_every_profile(self) -> None:
        self.build()
        ledger = self.target / ".ci-workspace-inputs.json"
        for corrupt in (None, "not json", '{"schema_version": 9000}'):
            with self.subTest(ledger=corrupt):
                self.build()
                self.cache("record")
                if corrupt is None:
                    ledger.unlink()
                else:
                    ledger.write_text(corrupt, encoding="utf-8")
                for profile in ("debug", "release", "x86_64-unknown-linux-gnu/debug"):
                    directory = self.target / profile / ".fingerprint"
                    for name in ("cache-base-00000000000000aa", "serde-00000000000000bb"):
                        unit = directory / name
                        unit.mkdir(parents=True, exist_ok=True)
                        (unit / "invoked.timestamp").write_text("artifact", encoding="utf-8")

                report = self.cache("prepare")

                self.assertFalse(report["cache_verified"])
                self.assertFalse(self.fingerprints("cache-base"))
                self.assertFalse(self.fingerprints("cache-consumer"))
                for profile in ("debug", "release", "x86_64-unknown-linux-gnu/debug"):
                    directory = self.target / profile / ".fingerprint"
                    self.assertFalse((directory / "cache-base-00000000000000aa").exists())
                    self.assertTrue((directory / "serde-00000000000000bb").exists())

    def test_unknown_workspace_unit_cannot_borrow_an_unchanged_packages_proof(self) -> None:
        self.build()
        self.cache("record")
        unknown = self.target / "debug" / ".fingerprint" / "cache-base-00000000000000aa"
        unknown.mkdir()
        (unknown / "invoked.timestamp").write_text("unrecorded", encoding="utf-8")

        report = self.cache("prepare")

        self.assertFalse(unknown.exists())
        self.assertEqual(report["invalidated_units"], [unknown.relative_to(self.target).as_posix()])
        self.assertEqual(len(report["reused_units"]), 2)

    def test_unknown_workspace_units_are_invalidated_in_nested_cached_targets(self) -> None:
        self.build()
        self.cache("record")
        fingerprints = self.target / "old-coverage" / "x86_64-linux" / "debug" / ".fingerprint"
        own = fingerprints / "cache-base-00000000000000aa"
        dependency = fingerprints / "serde-00000000000000bb"
        for unit in (own, dependency):
            unit.mkdir(parents=True)
            (unit / "invoked.timestamp").write_text("unrecorded", encoding="utf-8")

        self.cache("prepare")

        self.assertFalse(own.exists())
        self.assertTrue(dependency.exists())

    def test_command_feature_recipe_change_discards_old_workspace_units(self) -> None:
        self.build()
        self.cache("record", "--recipe", "cargo build --workspace")

        report = self.cache("prepare", "--recipe", "cargo build --workspace --features cache-base/alternate")

        self.assertFalse(report["cache_verified"])
        self.assertFalse(self.fingerprints("cache-base"))
        self.assertFalse(self.fingerprints("cache-consumer"))
        self.build("--features", "cache-base/alternate")
        self.assertEqual(self.value(), "1")

    def test_actual_compiler_flags_are_part_of_the_ledger_recipe(self) -> None:
        self.build()
        self.cache("record")
        self.environment["RUSTFLAGS"] = "-C opt-level=1"

        report = self.cache("prepare")

        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["reused_units"])
        self.assertFalse(self.fingerprints("cache-base"))

    def test_record_does_not_publish_raw_build_environment_credentials(self) -> None:
        self.build()
        self.environment["CARGO_REGISTRY_TOKEN"] = "secret-that-must-not-be-cached"
        self.environment["CARGO_REGISTRIES_PRIVATE_INDEX"] = "https://user:password@example.invalid/index"

        self.cache("record")

        ledger = (self.target / ".ci-workspace-inputs.json").read_text(encoding="utf-8")
        self.assertNotIn("secret-that-must-not-be-cached", ledger)
        self.assertNotIn("user:password", ledger)

    def test_changed_compiler_search_path_invalidates_the_build_environment(self) -> None:
        self.build()
        self.cache("record")
        alternative = self.sandbox / "alternative-compilers"
        alternative.mkdir()
        self.environment["PATH"] = str(alternative) + os.pathsep + self.environment["PATH"]

        report = self.cache("prepare")

        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["reused_units"])

    def test_global_build_input_change_discards_all_workspace_units(self) -> None:
        self.write(".cargo/config.toml", "[build]\njobs = 1\n")
        self.git("add", ".cargo/config.toml")
        self.build()
        self.cache("record")
        changed = self.write(".cargo/config.toml", "[build]\njobs = 2\n")
        os.utime(changed, (1, 1))

        report = self.cache("prepare")

        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["reused_units"])
        self.assertFalse(self.fingerprints("cache-consumer"))

    def test_exact_content_gitlink_change_discards_all_workspace_units(self) -> None:
        self.git("update-index", "--add", "--cacheinfo", f"160000,{'1' * 40},content")
        self.build()
        self.cache("record")
        self.git("update-index", "--cacheinfo", f"160000,{'2' * 40},content")

        report = self.cache("prepare")

        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["reused_units"])
        self.assertFalse(self.fingerprints("cache-consumer"))

    def test_different_toolchain_or_lexical_root_cannot_reuse_an_old_ledger(self) -> None:
        self.build()
        ledger = self.target / ".ci-workspace-inputs.json"
        for key, value in (("rustc", "different compiler"), ("checkout_root", "/different/checkout")):
            with self.subTest(recipe=key):
                self.build()
                self.cache("record")
                previous = json.loads(ledger.read_text(encoding="utf-8"))
                previous["recipe"][key] = value
                ledger.write_text(json.dumps(previous), encoding="utf-8")

                report = self.cache("prepare")

                self.assertFalse(report["cache_verified"])
                self.assertFalse(report["reused_units"])
                self.assertFalse(self.fingerprints("cache-consumer"))

    def test_record_is_deterministic_and_does_not_include_generated_test_results(self) -> None:
        self.build()
        self.write("consumer/test-results/generated.json", '{"tests": "untracked output"}')
        self.cache("record")
        ledger = self.target / ".ci-workspace-inputs.json"
        first = ledger.read_bytes()

        self.cache("record")

        self.assertEqual(ledger.read_bytes(), first)
        self.assertNotIn("test-results", first.decode())
        self.assertEqual(list(self.target.glob(".ci-workspace-inputs.json.*")), [])

    def test_symlinked_roots_and_targets_are_refused_without_mutating_artifacts(self) -> None:
        self.build()
        self.cache("record")
        ledger = (self.target / ".ci-workspace-inputs.json").read_bytes()
        for flag, destination in (("--root", self.root), ("--target", self.target)):
            with self.subTest(path=flag):
                alias = self.sandbox / (flag.removeprefix("--") + "-alias")
                alias.symlink_to(destination, target_is_directory=True)

                completed = self.cache_command("prepare", flag, str(alias))

                self.assertEqual(completed.returncode, 1, completed.stdout)
                self.assertIn("symlink", completed.stderr)
                self.assertEqual((self.target / ".ci-workspace-inputs.json").read_bytes(), ledger)
                self.assertTrue(self.fingerprints("cache-base"))

    def test_cached_cargo_lock_cannot_redirect_cache_operations_outside_target(self) -> None:
        self.build()
        self.cache("record")
        outside = self.sandbox / "outside-lock"
        outside.write_text("retain this external file", encoding="utf-8")
        lock = self.target / "debug" / ".cargo-lock"
        lock.unlink()
        lock.symlink_to(outside)

        completed = self.cache_command("prepare")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("symlink", completed.stderr)
        self.assertEqual(outside.read_text(encoding="utf-8"), "retain this external file")

    def test_tracked_source_links_cannot_read_or_stamp_an_external_file(self) -> None:
        self.build()
        outside = self.sandbox / "outside.rs"
        outside.write_text("private external input", encoding="utf-8")
        modified = outside.stat().st_mtime_ns
        link = self.root / "base" / "src" / "outside.rs"
        link.symlink_to(outside)
        self.git("add", "base/src/outside.rs")

        completed = self.cache_command("record")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("leaves the checkout", completed.stderr)
        self.assertEqual(outside.read_text(encoding="utf-8"), "private external input")
        self.assertEqual(outside.stat().st_mtime_ns, modified)
        self.assertFalse((self.target / ".ci-workspace-inputs.json").exists())

    def test_escaped_dep_info_paths_in_path_modules_track_their_actual_bytes(self) -> None:
        shared = self.write("base/src/shared input.rs", "pub fn value() -> u32 { 7 }\n")
        self.write("consumer/src/main.rs", '\n'.join((
            '#[path = "../../base/src/shared input.rs"] mod shared;',
            'fn main() { println!("{}", shared::value()); }', '',
        )))
        self.git("add", "base/src/shared input.rs", "consumer/src/main.rs")
        self.build()
        self.cache("record")
        shared.write_text("pub fn value() -> u32 { 9 }\n", encoding="utf-8")
        os.utime(shared, (1, 1))

        self.cache("prepare")

        self.assertFalse(self.fingerprints("cache-consumer"))
        self.build()
        self.assertEqual(self.value(), "9")

    def test_tampered_dep_info_cannot_replace_a_recorded_unit_proof(self) -> None:
        self.build()
        self.cache("record")
        dep_info = next((self.target / "debug" / "deps").glob("cache_base-*.d"))
        dep_info.write_text(dep_info.read_text(encoding="utf-8") + "\n# changed cache\n", encoding="utf-8")

        self.cache("prepare")

        self.assertFalse(self.fingerprints("cache-base"))
        self.assertTrue(self.fingerprints("cache-consumer"))

    def test_unverified_generated_target_inputs_are_never_reused_as_source_proof(self) -> None:
        self.target.mkdir()
        generated = self.target / "generated.rs"
        generated.write_text("fn generated_value() -> u32 { 7 }\n", encoding="utf-8")
        self.write("consumer/src/main.rs", '\n'.join((
            f'include!("{generated.as_posix()}");',
            'fn main() { println!("{}", generated_value()); }', '',
        )))
        self.build()
        self.cache("record")
        generated.write_text("fn generated_value() -> u32 { 9 }\n", encoding="utf-8")
        os.utime(generated, (1, 1))

        self.cache("prepare")

        self.assertFalse(self.fingerprints("cache-consumer"))
        self.assertEqual(generated.stat().st_mtime_ns, 1_000_000_000)
        self.build()
        self.assertEqual(self.value(), "9")


if __name__ == "__main__":
    unittest.main()
