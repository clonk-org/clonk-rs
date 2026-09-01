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
PATCH = REPOSITORY / "parity" / "bridge" / "oracle-weather.patch"
BUILD_SCRIPT = REPOSITORY / "parity" / "bridge" / "build-oracle-validation.sh"
EXPECTED_ORACLE_PATHS = {
    "src/C4Weather.cpp",
    "src/C4Weather.h",
    "src/rust/RustEngineBridge.cpp",
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


class OracleWeatherPatchTests(unittest.TestCase):
    def test_patch_applies_to_the_exact_pin_and_only_changes_the_reviewed_paths(self):
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

    def test_build_script_is_idempotent_and_rejects_a_partial_patch(self):
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
                shutil.copy2(BUILD_SCRIPT, bridge / BUILD_SCRIPT.name)
                shutil.copy2(PATCH, bridge / PATCH.name)
                shutil.copy2(
                    REPOSITORY / "parity" / "bridge" / "lc_engine_ffi.h",
                    bridge / "lc_engine_ffi.h",
                )

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

                first = subprocess.run(
                    [str(bridge / BUILD_SCRIPT.name), "--oracle-root", str(oracle)],
                    check=False,
                    capture_output=True,
                    text=True,
                    env=environment,
                )
                self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
                self.assertIn(
                    "applied the oracle weather instrumentation patch", first.stdout
                )

                second = subprocess.run(
                    [str(bridge / BUILD_SCRIPT.name), "--oracle-root", str(oracle)],
                    check=False,
                    capture_output=True,
                    text=True,
                    env=environment,
                )
                self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
                self.assertIn(
                    "oracle weather instrumentation patch already applied",
                    second.stdout,
                )

                run(
                    "git",
                    "apply",
                    "--reverse",
                    "--include=src/C4Weather.h",
                    str(bridge / PATCH.name),
                    cwd=oracle,
                )
                partial = subprocess.run(
                    [str(bridge / BUILD_SCRIPT.name), "--oracle-root", str(oracle)],
                    check=False,
                    capture_output=True,
                    text=True,
                    env=environment,
                )
                self.assertNotEqual(partial.returncode, 0)
                self.assertIn(
                    "oracle weather patch is partially applied", partial.stderr
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
