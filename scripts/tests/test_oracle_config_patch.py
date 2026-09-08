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
PATCH = BRIDGE / "oracle-config-bridge.patch"
WEATHER_PATCH = BRIDGE / "oracle-weather.patch"
BUILD_SCRIPT = BRIDGE / "build-oracle-validation.sh"
HEADERS = (
    "lc_engine_ffi.h",
    "lc_config_ffi.h",
    "lc_group_ffi.h",
    "lc_platform_ffi.h",
)
# The engine header extends the pin with the runtime observation transports the
# layered engine patch consumes; the other three must be the pin's bytes.
PINNED_HEADERS = HEADERS[1:]
EXPECTED_ORACLE_PATHS = {
    "src/C4Config.cpp",
    "src/rust/RustConfigBridge.cpp",
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


class OracleConfigPatchTests(unittest.TestCase):
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

    def test_patch_keeps_the_bridge_report_out_of_the_lost_early_log(self):
        # C4Config::Load runs before LogSystem.OpenLog (C4Application.cpp), so
        # every DebugLog the pinned compare block writes is dropped. The patch
        # must route the whole report to stderr, leak the bridge mutex that
        # C4Config's static destructor locks after it is gone, and say when
        # parity held or the compare never ran.
        patch = PATCH.read_text(encoding="utf-8")
        added = [line[1:] for line in patch.splitlines() if line.startswith("+")]
        removed = [line[1:] for line in patch.splitlines() if line.startswith("-")]
        self.assertTrue(any("void RustConfigLog(" in line for line in added))
        self.assertFalse(
            any("DebugLog(spdlog::level" in line for line in added),
            "the compare block must not add DebugLog calls that the closed log drops",
        )
        self.assertTrue(any("std::mutex &g_mutex = *new std::mutex;" in line for line in added))
        self.assertTrue(any("std::mutex g_mutex;" in line for line in removed))
        for report in (
            "Rust config parity verified; overrides active",
            "Rust config dump unavailable; parity not checked",
        ):
            self.assertTrue(any(report in line for line in added), report)

    def test_vendored_bridge_headers_match_the_pin(self):
        for header in PINNED_HEADERS:
            pinned = run(
                "git", "show", f"{PINNED_ORACLE_REVISION}:rust/include/{header}"
            ).stdout
            self.assertEqual(
                pinned,
                (BRIDGE / header).read_bytes(),
                f"parity/bridge/{header} drifted from the pin",
            )

    def test_build_script_applies_the_patch_only_with_the_config_option(self):
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
                for source in (BUILD_SCRIPT, PATCH, WEATHER_PATCH):
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

                without = build()
                self.assertEqual(without.returncode, 0, without.stdout + without.stderr)
                self.assertNotIn("config-bridge", without.stdout)
                self.assertIn("vendored config, group and platform headers match the pin", without.stdout)
                run("git", "apply", "--check", str(bridge / PATCH.name), cwd=oracle)

                first = build("--with-config")
                self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
                self.assertIn("applied the oracle config-bridge compile fix", first.stdout)
                self.assertIn("run-config-differential.sh", first.stdout)

                second = build("--with-config")
                self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
                self.assertIn(
                    "oracle config-bridge compile fix already applied", second.stdout
                )

                run(
                    "git",
                    "apply",
                    "--reverse",
                    "--include=src/rust/RustConfigBridge.cpp",
                    str(bridge / PATCH.name),
                    cwd=oracle,
                )
                partial = build("--with-config")
                self.assertNotEqual(partial.returncode, 0)
                self.assertIn(
                    "oracle config-bridge patch is partially applied", partial.stderr
                )

                (bridge / "lc_config_ffi.h").write_bytes(b"// drifted\n")
                drifted = build()
                self.assertNotEqual(drifted.returncode, 0)
                self.assertIn(
                    "parity/bridge/lc_config_ffi.h differs from rust/include/lc_config_ffi.h",
                    drifted.stderr,
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
