"""Tests for deterministic, exact-SHA release prebuild manifests."""

import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from _repo import REPOSITORY


SCRIPT = REPOSITORY / "scripts" / "release-prebuild-manifest.py"
HEAD_SHA = "1" * 40
TREE_SHA = "2" * 40


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


if __name__ == "__main__":
    unittest.main()
