import hashlib
import io
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[2]
PINNED_ORACLE_REVISION = "7d43b47b7d789b533f32d005e64596e0a07019cd"
BRIDGE = REPOSITORY / "parity" / "bridge"
PATCH = BRIDGE / "oracle-group-bridge.patch"
CONFIG_PATCH = BRIDGE / "oracle-config-bridge.patch"
WEATHER_PATCH = BRIDGE / "oracle-weather.patch"
BUILD_SCRIPT = BRIDGE / "build-oracle-validation.sh"
HEADERS = (
    "lc_engine_ffi.h",
    "lc_config_ffi.h",
    "lc_group_ffi.h",
    "lc_platform_ffi.h",
)
EXPECTED_ORACLE_PATHS = {
    "src/C4Group.cpp",
    "src/rust/RustGroupBridge.cpp",
}


def run(*arguments, cwd=REPOSITORY, **kwargs):
    return subprocess.run(
        arguments,
        cwd=cwd,
        check=True,
        capture_output=True,
        **kwargs,
    )


def file_digests(root):
    return {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in root.rglob("*")
        if path.is_file()
    }


class OracleGroupPatchTests(unittest.TestCase):
    def test_patch_applies_to_the_exact_pin_and_only_changes_the_bridge_paths(self):
        numstat = run("git", "apply", "--numstat", str(PATCH), text=True).stdout
        patch_paths = {line.split("\t", 2)[2] for line in numstat.splitlines()}
        self.assertEqual(patch_paths, EXPECTED_ORACLE_PATHS)

        archive = run(
            "git",
            "archive",
            PINNED_ORACLE_REVISION,
            "--",
            *sorted(EXPECTED_ORACLE_PATHS),
        ).stdout
        with tempfile.TemporaryDirectory() as temporary:
            oracle = Path(temporary)
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as files:
                files.extractall(oracle)

            before = file_digests(oracle)
            run("git", "apply", "--check", str(PATCH), cwd=oracle)
            run("git", "apply", str(PATCH), cwd=oracle)
            after = file_digests(oracle)

            changed = {
                path
                for path in before.keys() | after.keys()
                if before.get(path) != after.get(path)
            }
            self.assertEqual(changed, EXPECTED_ORACLE_PATHS)
            run("git", "apply", "--reverse", "--check", str(PATCH), cwd=oracle)

    def test_patch_fixes_the_moved_key_and_names_every_report_path(self):
        # The pinned bridge moved each entry's canonical name into the map
        # value while naming it as the key, so the Rust view collapsed to one
        # empty key and every real folder was reported as mostly missing. The
        # patch keys on a copy, says when the lists agree, exposes each report
        # path to a fault hook, and reads every file in deep mode.
        patch = PATCH.read_text(encoding="utf-8")
        added = [line[1:] for line in patch.splitlines() if line.startswith("+")]
        removed = [line[1:] for line in patch.splitlines() if line.startswith("-")]
        self.assertTrue(
            any(
                "rust_entries.emplace(canonical, RustEntry{std::move(original), std::move(canonical)" in line
                for line in removed
            )
        )
        self.assertTrue(any("rust_entries.emplace(std::move(key)," in line for line in added))
        for report in (
            "Rust group validation {}: {} entries agree",
            "Rust group validation {}: deep check agrees ({} files read)",
            "Rust group validation {}: read mismatch for entries: {}",
            'std::getenv("LC_RUST_GROUP_FAULT")',
            'std::getenv("LC_RUST_GROUP_DEEP")',
        ):
            self.assertTrue(any(report in line for line in added), report)
        for fault in ('"missing"', '"additional"', '"size"', '"type"', '"read"', '"open"'):
            self.assertTrue(any(f"== {fault}" in line for line in added), fault)
        # Every Rust allocation the deep mode takes is given back.
        for free in ("lc_group_buffer_free(rust, rust_len)", "lc_group_string_free(maker)", "lc_group_string_free(root)"):
            self.assertTrue(any(free in line for line in added), free)

    def test_build_script_applies_the_patch_only_with_the_group_option(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            oracle = root / "oracle"
            run(
                "git",
                "worktree",
                "add",
                "--detach",
                str(oracle),
                PINNED_ORACLE_REVISION,
            )
            try:
                bridge = root / "port" / "parity" / "bridge"
                bridge.mkdir(parents=True)
                for source in (BUILD_SCRIPT, PATCH, CONFIG_PATCH, WEATHER_PATCH):
                    shutil.copy2(source, bridge / source.name)
                for header in HEADERS:
                    shutil.copy2(BRIDGE / header, bridge / header)

                fake_bin = root / "bin"
                fake_bin.mkdir()
                for command, body in (
                    ("cmake", "#!/bin/sh\nexit 0\n"),
                    ("xcrun", "#!/bin/sh\nprintf '/tmp/fake-sdk\\n'\n"),
                ):
                    executable = fake_bin / command
                    executable.write_text(body, encoding="utf-8")
                    executable.chmod(0o755)

                environment = os.environ.copy()
                environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
                script = str(bridge / BUILD_SCRIPT.name)

                def build(*extra):
                    return subprocess.run(
                        [script, "--oracle-root", str(oracle), *extra],
                        check=False,
                        capture_output=True,
                        text=True,
                        env=environment,
                    )

                without = build("--with-config")
                self.assertEqual(without.returncode, 0, without.stdout + without.stderr)
                self.assertNotIn("group-bridge", without.stdout)
                run("git", "apply", "--check", str(bridge / PATCH.name), cwd=oracle)

                first = build("--with-group-validation")
                self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
                self.assertIn("applied the oracle group-bridge compile fix", first.stdout)
                self.assertIn("run-group-differential.sh", first.stdout)

                second = build("--with-group-validation")
                self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
                self.assertIn(
                    "oracle group-bridge compile fix already applied", second.stdout
                )

                run(
                    "git",
                    "apply",
                    "--reverse",
                    "--include=src/C4Group.cpp",
                    str(bridge / PATCH.name),
                    cwd=oracle,
                )
                partial = build("--with-group-validation")
                self.assertNotEqual(partial.returncode, 0)
                self.assertIn(
                    "oracle group-bridge patch is partially applied", partial.stderr
                )
            finally:
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(REPOSITORY),
                        "worktree",
                        "remove",
                        "--force",
                        str(oracle),
                    ],
                    check=False,
                    capture_output=True,
                )


if __name__ == "__main__":
    unittest.main()
