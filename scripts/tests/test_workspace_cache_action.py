"""Source-aware compiled-input keys and publication authority for CI caches."""

import contextlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest
from unittest import mock

from _repo import REPOSITORY


ACTION = REPOSITORY / ".github/actions/workspace-cache/action.yml"
CACHE_REVISION = "55cc8345863c7cc4c66a329aec7e433d2d1c52a9"
DEPENDENCY_CHECKSUM = "8f42a60cbdf9a97f5d2305f08a87dc4e09308d1276d28c869c684d7777685682"


def identity_program():
    text = ACTION.read_text(encoding="utf-8")
    match = re.search(
        r"(?ms)^        python3 - <<'PYKEY'\n(.*?)^        PYKEY\s*$", text
    )
    if match is None:
        raise AssertionError("the action must contain the executable identity program")
    return textwrap.dedent(match[1])


class WorkspaceCacheActionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="workspace-cache-action-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name).resolve()
        self.root = self.directory / "checkout"
        self.root.mkdir()
        (self.root / ".git").mkdir()
        (self.root / ".cargo").mkdir()
        (self.root / "Cargo.toml").write_text('''[workspace]
members = ["crates/example"]
resolver = "2"

[workspace.package]
version = "1.5.0"
edition = "2021"

[profile.test]
opt-level = 2
''', encoding="utf-8")
        member = self.root / "crates/example"
        member.mkdir(parents=True)
        (member / "Cargo.toml").write_text('''[package]
name = "example"
version.workspace = true
edition.workspace = true

[features]
default = []
compact = []

[dependencies]
itoa = { version = "1", default-features = false }
''', encoding="utf-8")
        (member / "src").mkdir()
        (member / "src/lib.rs").write_text("pub fn fixture() {}\n", encoding="utf-8")
        (self.root / "Cargo.lock").write_text('''version = 4

[[package]]
name = "example"
version = "1.5.0"
dependencies = ["itoa"]

[[package]]
name = "itoa"
version = "1.0.18"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "1111111111111111111111111111111111111111111111111111111111111111"
'''.replace("1" * 64, DEPENDENCY_CHECKSUM), encoding="utf-8")
        (self.root / ".cargo/config.toml").write_text("[build]\n", encoding="utf-8")
        (self.root / "rust-toolchain.toml").write_text(
            '[toolchain]\nchannel = "1.98.1"\n', encoding="utf-8"
        )
        self.head = "a" * 40
        self.tree = "b" * 40
        self.content = "c" * 40
        self.compiler = b"rustc 1.98.1\nhost: x86_64-unknown-linux-gnu\ncommit-hash: compiler\n"
        self.output = self.directory / "output"
        self.event_path = self.directory / "event.json"
        self.event = {
            "repository": {"full_name": "clonk-org/clonk-rs", "fork": False},
            "ref": "refs/heads/main", "after": self.head, "deleted": False,
        }
        self.environment = {
            "GITHUB_WORKSPACE": str(self.root), "GITHUB_SHA": self.head,
            "GITHUB_OUTPUT": str(self.output), "GITHUB_REPOSITORY": "clonk-org/clonk-rs",
            "GITHUB_REF": "refs/heads/main", "GITHUB_EVENT_NAME": "push",
            "GITHUB_EVENT_PATH": str(self.event_path), "RUNNER_OS": "Linux",
            "CACHE_OPERATION": "restore", "CACHE_LANE": "landing-linux",
            "CACHE_TARGET": "target", "CACHE_LEDGER": ".ci-cache-ledgers/landing.json",
            "CACHE_RECIPE": "landing-v1", "CARGO_INCREMENTAL": "0",
        }
        self.tracked = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml",
                        "rust-toolchain.toml", "crates/example/Cargo.toml",
                        "crates/example/src/lib.rs", "content"]
        self.dirty = b""

    def fake_command(self, arguments, **kwargs):
        if arguments[0] in ("rustc", "/selected/rustc"):
            self.assertEqual(arguments[1:], ["-vV"])
            return self.compiler
        self.assertEqual(arguments[:3], ["git", "-C", str(self.root)])
        command = arguments[3:]
        if command == ["rev-parse", "--show-toplevel"]:
            return (str(self.root) + "\n").encode()
        if command == ["rev-parse", "HEAD"]:
            return (self.head + "\n").encode()
        if command == ["rev-parse", "HEAD^{tree}"]:
            return (self.tree + "\n").encode()
        if command == ["ls-tree", "HEAD", "--", "content"]:
            return f"160000 commit {self.content}\tcontent\n".encode()
        if command == ["ls-files", "-z"]:
            return ("\0".join(self.tracked) + "\0").encode()
        if command == ["diff", "--name-only", "HEAD", "--", ".", ":(exclude)content"]:
            return self.dirty
        raise AssertionError(f"unexpected identity command: {arguments}")

    def execute_identity(self, **changes):
        self.output.unlink(missing_ok=True)
        self.event_path.write_text(json.dumps(self.event), encoding="utf-8")
        captured = io.StringIO()
        environment = {**self.environment, **changes}
        with mock.patch.dict(os.environ, environment, clear=True), \
                mock.patch.object(subprocess, "check_output", side_effect=self.fake_command), \
                contextlib.redirect_stdout(captured), contextlib.redirect_stderr(captured):
            exec(compile(identity_program(), str(ACTION), "exec"), {"__name__": "__main__"})
        outputs = dict(line.split("=", 1) for line in self.output.read_text().splitlines())
        return outputs, captured.getvalue()

    def prepare_save(self):
        (self.root / "target").mkdir(exist_ok=True)
        ledger = self.root / self.environment["CACHE_LEDGER"]
        ledger.parent.mkdir(exist_ok=True)
        ledger.write_text('{"schema_version": 1}\n', encoding="utf-8")

    def test_source_tree_changes_primary_key_but_preserves_restore_prefix(self):
        original, _ = self.execute_identity()
        self.tree = "d" * 40
        changed, _ = self.execute_identity()
        self.assertNotEqual(original["key"], changed["key"])
        self.assertEqual(original["prefix"], changed["prefix"])
        self.assertTrue(original["key"].startswith(original["prefix"]))
        self.assertTrue(original["key"].endswith("b" * 40))
        self.assertTrue(changed["key"].endswith(self.tree))

    def test_workspace_release_version_bump_restores_dependencies_under_a_new_source_key(self):
        original, _ = self.execute_identity()
        for name in ("Cargo.toml", "Cargo.lock"):
            path = self.root / name
            path.write_text(path.read_text().replace('"1.5.0"', '"1.5.1"'), encoding="utf-8")
        self.tree = "d" * 40
        changed, _ = self.execute_identity()
        self.assertEqual(original["prefix"], changed["prefix"])
        self.assertNotEqual(original["key"], changed["key"])
        self.assertTrue(changed["key"].endswith(self.tree))

    def test_declared_workspace_profiles_dependencies_and_features_change_restore_prefix(self):
        original, _ = self.execute_identity()
        for name, before, after in (
            ("Cargo.toml", "opt-level = 2", "opt-level = 3"),
            ("crates/example/Cargo.toml", 'version = "1"', 'version = "=1.0.18"'),
            ("crates/example/Cargo.toml", "default = []", 'default = ["compact"]'),
        ):
            with self.subTest(manifest=name, setting=before):
                path = self.root / name
                previous = path.read_text()
                path.write_text(previous.replace(before, after), encoding="utf-8")
                changed, _ = self.execute_identity()
                self.assertNotEqual(original["prefix"], changed["prefix"])
                path.write_text(previous, encoding="utf-8")

    def test_unknown_operations_lanes_and_missing_recipes_fail_closed(self):
        for changes in (
            {"CACHE_OPERATION": "erase"}, {"CACHE_LANE": "unowned"},
            {"CACHE_RECIPE": ""}, {"CACHE_RECIPE": "release\nwrite=true"},
        ):
            with self.subTest(changes=changes), self.assertRaises(SystemExit):
                self.execute_identity(**changes)

    def test_cache_identity_rejects_wrong_commit_and_symlinked_checkout(self):
        with self.assertRaises(SystemExit):
            self.execute_identity(GITHUB_SHA="f" * 40)
        alias = self.directory / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(SystemExit):
            self.execute_identity(GITHUB_WORKSPACE=str(alias))

    def test_malformed_git_tree_and_content_identities_are_rejected(self):
        for field in ("tree", "content"):
            with self.subTest(field=field):
                previous = getattr(self, field)
                setattr(self, field, "not-a-git-object")
                try:
                    with self.assertRaises(SystemExit):
                        self.execute_identity()
                finally:
                    setattr(self, field, previous)

    def test_cache_paths_cannot_cover_source_metadata_or_other_checkouts(self):
        for changes in (
            {"CACHE_TARGET": "."}, {"CACHE_TARGET": "../other"},
            {"CACHE_TARGET": str(self.directory / "other")},
            {"CACHE_TARGET": ".git/cache"}, {"CACHE_TARGET": "content/cache"},
            {"CACHE_TARGET": "crates/example/target"},
            {"CACHE_TARGET": "target\nother"},
            {"CACHE_LEDGER": "Cargo.lock"}, {"CACHE_LEDGER": "target/ledger.json"},
            {"CACHE_LEDGER": "crates/ledger.json"},
            {"CACHE_LEDGER": "../ledger.json"},
        ):
            with self.subTest(changes=changes), self.assertRaises(SystemExit):
                self.execute_identity(**changes)
        outside = self.directory / "outside"
        outside.mkdir()
        (self.root / "linked").symlink_to(outside, target_is_directory=True)
        for changes in (
            {"CACHE_TARGET": "linked/target"}, {"CACHE_LEDGER": "linked/ledger.json"},
        ):
            with self.subTest(changes=changes), self.assertRaises(SystemExit):
                self.execute_identity(**changes)

    def test_main_publication_requires_the_same_repository_trusted_event_and_ledger(self):
        self.prepare_save()
        for event_name in ("push", "workflow_dispatch", "schedule"):
            with self.subTest(event=event_name):
                outputs, _ = self.execute_identity(
                    CACHE_OPERATION="save", GITHUB_EVENT_NAME=event_name
                )
                self.assertEqual(outputs["trusted-save"], "true")

    def test_main_publication_rejects_forks_pull_requests_and_missing_authority(self):
        self.prepare_save()
        trusted_event = self.event
        cases = (
            ({"GITHUB_EVENT_NAME": "pull_request"}, trusted_event),
            ({"GITHUB_EVENT_NAME": "merge_group"}, trusted_event),
            ({"GITHUB_REF": "refs/pull/1/merge"}, trusted_event),
            ({}, {**trusted_event, "repository": {"full_name": "someone/clonk-rs", "fork": True}}),
            ({}, {**trusted_event, "repository": {"full_name": "clonk-org/clonk-rs", "fork": True}}),
            ({}, {**trusted_event, "pull_request": {}}),
            ({}, {**trusted_event, "after": "e" * 40}),
            ({}, {**trusted_event, "deleted": True}),
            ({}, {**trusted_event, "ref": "refs/heads/feature"}),
            ({"GITHUB_EVENT_NAME": "workflow_dispatch"}, {**trusted_event, "ref": "feature"}),
            ({"GITHUB_EVENT_PATH": str(self.directory / "missing-event")}, trusted_event),
            ({}, []),
            ({"GITHUB_REPOSITORY": ""}, {**trusted_event, "repository": {"full_name": "", "fork": False}}),
        )
        for changes, event in cases:
            with self.subTest(changes=changes, event=event):
                self.event = event
                with self.assertRaises(SystemExit):
                    self.execute_identity(CACHE_OPERATION="save", **changes)
        self.event = trusted_event

    def test_recipe_files_cannot_be_symlinked_to_another_checkout(self):
        configuration = self.root / ".cargo/config.toml"
        external = self.directory / "external-config.toml"
        external.write_bytes(configuration.read_bytes())
        configuration.unlink()
        configuration.symlink_to(external)
        with self.assertRaises(SystemExit):
            self.execute_identity()

    def test_compiler_build_flags_recipe_and_lane_change_restore_prefix(self):
        original, _ = self.execute_identity()
        for variable in (
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CC", "CXX", "CFLAGS", "CPPFLAGS",
            "CMAKE_BUILD_TYPE", "SDKROOT", "LLVM_PROFILE_FILE", "__CARGO_LLVM_COV_FLAGS",
            "_CARGO_LLVM_COV_FLAGS", "CARGO_INCREMENTAL", "RUSTC_WRAPPER",
        ):
            with self.subTest(variable=variable):
                changed, _ = self.execute_identity(**{variable: "different-build-input"})
                self.assertNotEqual(original["prefix"], changed["prefix"])
        for variable, value in (
            ("CACHE_RECIPE", "coverage-profile-features"), ("CACHE_LANE", "coverage-linux"),
        ):
            changed, _ = self.execute_identity(**{variable: value})
            self.assertNotEqual(original["prefix"], changed["prefix"])
        self.compiler += b"llvm-version: different\n"
        changed, _ = self.execute_identity()
        self.assertNotEqual(original["prefix"], changed["prefix"])

    def test_external_dependency_lock_config_toolchain_and_content_pin_change_restore_prefix(self):
        original, _ = self.execute_identity()
        for name in ("Cargo.lock", ".cargo/config.toml", "rust-toolchain.toml"):
            with self.subTest(file=name):
                path = self.root / name
                previous = path.read_bytes()
                changed_input = (
                    previous.replace(b'"1.0.18"', b'"1.0.19"')
                    if name == "Cargo.lock" else previous + b"# changed actual input\n"
                )
                path.write_bytes(changed_input)
                changed, _ = self.execute_identity()
                self.assertNotEqual(original["prefix"], changed["prefix"])
                path.write_bytes(previous)
        self.content = "e" * 40
        changed, _ = self.execute_identity()
        self.assertNotEqual(original["prefix"], changed["prefix"])

    def test_hosted_native_image_changes_invalidate_prefix_without_source_or_rust_changes(self):
        original, _ = self.execute_identity(ImageOS="ubuntu24", ImageVersion="20261005.1")
        for image in (
            {"ImageOS": "ubuntu24", "ImageVersion": "20261006.1"},
            {"ImageOS": "ubuntu22", "ImageVersion": "20261005.1"},
            {},
        ):
            with self.subTest(image=image):
                changed, _ = self.execute_identity(**image)
                self.assertNotEqual(original["prefix"], changed["prefix"])
                self.assertEqual(original["key"].removeprefix(original["prefix"]), self.tree)
                self.assertEqual(changed["key"].removeprefix(changed["prefix"]), self.tree)

    def test_invalid_recipe_toml_fails_closed_before_optional_cache_transfers(self):
        for name in (
            "Cargo.lock", "Cargo.toml", ".cargo/config.toml", "rust-toolchain.toml",
            "crates/example/Cargo.toml",
        ):
            with self.subTest(file=name):
                path = self.root / name
                previous = path.read_bytes()
                path.write_text("[malformed\n", encoding="utf-8")
                with self.assertRaises(SystemExit):
                    self.execute_identity()
                path.write_bytes(previous)

    def test_workspace_package_version_is_omitted_but_dependency_versions_are_preserved(self):
        manifest = self.root / "crates/example/Cargo.toml"
        manifest.write_text(manifest.read_text().replace("version.workspace = true", 'version = "1.5.0"'), encoding="utf-8")
        original, _ = self.execute_identity()
        manifest.write_text(manifest.read_text().replace('version = "1.5.0"', 'version = "1.5.1"'), encoding="utf-8")
        changed, _ = self.execute_identity()
        self.assertEqual(original["prefix"], changed["prefix"])

    def test_external_lock_checksums_and_dependency_edges_are_retained(self):
        original, _ = self.execute_identity()
        lock = self.root / "Cargo.lock"
        previous = lock.read_text()
        for content in (
            previous.replace(DEPENDENCY_CHECKSUM, "2" * 64),
            previous + 'dependencies = ["native-helper"]\n',
        ):
            with self.subTest(lock=content):
                lock.write_text(content, encoding="utf-8")
                changed, _ = self.execute_identity()
                self.assertNotEqual(original["prefix"], changed["prefix"])
        lock.write_text(previous, encoding="utf-8")

    def test_declared_workspace_manifests_must_exist_inside_the_checkout(self):
        manifest = self.root / "Cargo.toml"
        previous = manifest.read_text()
        for member in ("../outside", "/outside", "crates/missing"):
            with self.subTest(member=member):
                manifest.write_text(previous.replace('"crates/example"', json.dumps(member)), encoding="utf-8")
                with self.assertRaises(SystemExit):
                    self.execute_identity()
        manifest.write_text(previous, encoding="utf-8")

    def test_credentials_do_not_enter_the_key_or_console_output(self):
        original, _ = self.execute_identity()
        changed, console = self.execute_identity(
            CARGO_REGISTRIES_CRATES_IO_TOKEN="registry-private-value",
            CMAKE_AUTH_TOKEN="compiler-private-value",
            RUST_SECRET="rust-private-value", LLVM_API_KEY="llvm-private-value",
            CARGO_CREDENTIAL_PROVIDER="credential-helper",
        )
        self.assertEqual(original["key"], changed["key"])
        self.assertEqual(console, "")
        self.assertNotIn("private-value", self.output.read_text())

    def test_save_requires_existing_target_and_nonempty_ledger_and_committed_inputs(self):
        with self.assertRaises(SystemExit):
            self.execute_identity(CACHE_OPERATION="save")
        self.prepare_save()
        ledger = self.root / self.environment["CACHE_LEDGER"]
        ledger.write_text("", encoding="utf-8")
        with self.assertRaises(SystemExit):
            self.execute_identity(CACHE_OPERATION="save")
        ledger.write_text('{"schema_version": 1}\n', encoding="utf-8")
        self.dirty = b"crates/example/src/lib.rs\n"
        with self.assertRaises(SystemExit):
            self.execute_identity(CACHE_OPERATION="save")

    def test_restore_and_lookup_share_keys_without_publication_authority(self):
        restore, _ = self.execute_identity(GITHUB_EVENT_NAME="pull_request")
        lookup, _ = self.execute_identity(CACHE_OPERATION="lookup", GITHUB_EVENT_NAME="merge_group")
        self.assertEqual(restore["key"], lookup["key"])
        self.assertEqual(restore["prefix"], lookup["prefix"])
        self.assertEqual(restore["trusted-save"], "false")
        self.assertEqual(lookup["trusted-save"], "false")

    def test_cache_transfers_are_optional_pinned_and_use_paired_validated_paths(self):
        text = ACTION.read_text(encoding="utf-8")
        restore = text.split("    - name: Restore or look up optional compiled inputs\n", 1)[1]
        restore, save = restore.split("    - name: Publish optional compiled inputs from trusted main\n", 1)
        identity = text.split("    - name: Restore or look up optional compiled inputs\n", 1)[0]
        self.assertNotIn("continue-on-error", identity)
        self.assertNotIn("Swatinem", text)
        self.assertIn("if: inputs.operation == 'restore' || inputs.operation == 'lookup'", restore)
        self.assertIn("lookup-only: ${{ inputs.operation == 'lookup' }}", restore)
        self.assertIn("restore-keys: ${{ steps.identity.outputs.prefix }}", restore)
        self.assertIn("if: inputs.operation == 'save' && steps.identity.outputs.trusted-save == 'true'", save)
        self.assertNotIn("restore-keys:", save)
        for block, operation in ((restore, "restore"), (save, "save")):
            with self.subTest(operation=operation):
                self.assertIn(f"uses: actions/cache/{operation}@{CACHE_REVISION}", block)
                self.assertIn("continue-on-error: true", block)
                self.assertIn("${{ steps.identity.outputs.target }}", block)
                self.assertIn("${{ steps.identity.outputs.ledger }}", block)
                self.assertIn("key: ${{ steps.identity.outputs.key }}", block)


if __name__ == "__main__":
    unittest.main()
