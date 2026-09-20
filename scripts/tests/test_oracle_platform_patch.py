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
PATCH = BRIDGE / "oracle-platform-bridge.patch"
SIBLING_PATCHES = (
    BRIDGE / "oracle-config-bridge.patch",
    BRIDGE / "oracle-group-bridge.patch",
    BRIDGE / "oracle-weather.patch",
)
BUILD_SCRIPT = BRIDGE / "build-oracle-validation.sh"
HEADERS = (
    "lc_engine_ffi.h",
    "lc_config_ffi.h",
    "lc_group_ffi.h",
    "lc_platform_ffi.h",
    "lc_gui_ffi.h",
)
EXPECTED_ORACLE_PATHS = {
    "src/rust/RustPlatformBridge.cpp",
    "src/rust/RustPlatformBridge.h",
}
GETTERS = (
    "install_root",
    "planet_dir",
    "system_group_path",
    "user_data_dir",
    "cache_dir",
    "logs_dir",
    "temp_dir",
    "config_dir",
)


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


class OraclePlatformPatchTests(unittest.TestCase):
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

    def test_the_three_bridge_patches_apply_together_in_any_order(self):
        # The builder layers them one after another; none may depend on the
        # others' context, which is why the platform patch keeps out of
        # C4Config.cpp and does its non-engine undef in the bridge header.
        patches = [PATCH, *SIBLING_PATCHES[:2]]
        paths = set(EXPECTED_ORACLE_PATHS)
        for patch in patches:
            numstat = run("git", "apply", "--numstat", str(patch), text=True).stdout
            paths |= {line.split("\t", 2)[2] for line in numstat.splitlines()}
        archive = run("git", "archive", PINNED_ORACLE_REVISION, "--", *sorted(paths)).stdout
        for order in (patches, list(reversed(patches))):
            with tempfile.TemporaryDirectory() as temporary:
                oracle = Path(temporary)
                with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as files:
                    files.extractall(oracle)
                for patch in order:
                    run("git", "apply", str(patch), cwd=oracle)
                for patch in order:
                    run("git", "apply", "--reverse", "--check", str(patch), cwd=oracle)

    def test_patch_reports_the_set_names_the_null_getter_and_frees_before_faulting(self):
        patch = PATCH.read_text(encoding="utf-8")
        added = [line[1:] for line in patch.splitlines() if line.startswith("+")]
        self.assertTrue(any("Rust platform paths: install='%s'" in line for line in added))
        self.assertTrue(
            any("Rust platform paths unavailable (%s returned null); legacy paths used" in line for line in added)
        )
        self.assertTrue(any("Rust platform user directories: %s" in line for line in added))
        for getter in GETTERS:
            self.assertTrue(
                any(f'Fetch(&lc_platform_{getter}, "lc_platform_{getter}");' in line for line in added),
                getter,
            )
        # The fault discards the answer only after TakeString freed it.
        joined = "\n".join(added)
        self.assertLess(joined.index("auto value = TakeString(getter());"), joined.index("value.reset();"))
        self.assertTrue(any('#if defined(USE_RUST_PLATFORM_PATHS) && !defined(C4ENGINE)' in line for line in added))

    def test_build_script_applies_the_patch_only_with_the_platform_option(self):
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
                for source in (BUILD_SCRIPT, PATCH, *SIBLING_PATCHES):
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

                without = build("--with-config", "--with-group-validation")
                self.assertEqual(without.returncode, 0, without.stdout + without.stderr)
                self.assertNotIn("platform-bridge", without.stdout)
                run("git", "apply", "--check", str(bridge / PATCH.name), cwd=oracle)

                first = build("--with-platform-paths")
                self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
                self.assertIn("applied the oracle platform-bridge compile fix", first.stdout)
                self.assertIn("run-platform-differential.sh", first.stdout)

                second = build("--with-platform-paths")
                self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
                self.assertIn(
                    "oracle platform-bridge compile fix already applied", second.stdout
                )

                run(
                    "git",
                    "apply",
                    "--reverse",
                    "--include=src/rust/RustPlatformBridge.h",
                    str(bridge / PATCH.name),
                    cwd=oracle,
                )
                partial = build("--with-platform-paths")
                self.assertNotEqual(partial.returncode, 0)
                self.assertIn(
                    "oracle platform-bridge patch is partially applied", partial.stderr
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
