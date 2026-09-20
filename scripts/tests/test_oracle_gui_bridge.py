import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[2]
PINNED_ORACLE_REVISION = "7d43b47b7d789b533f32d005e64596e0a07019cd"
BRIDGE = REPOSITORY / "parity" / "bridge"
BUILD_SCRIPT = BRIDGE / "build-oracle-validation.sh"
PATCHES = (
    BRIDGE / "oracle-config-bridge.patch",
    BRIDGE / "oracle-group-bridge.patch",
    BRIDGE / "oracle-platform-bridge.patch",
    BRIDGE / "oracle-weather.patch",
)
HEADERS = (
    "lc_engine_ffi.h",
    "lc_config_ffi.h",
    "lc_group_ffi.h",
    "lc_platform_ffi.h",
    "lc_gui_ffi.h",
)


def run(*arguments, cwd=REPOSITORY, **kwargs):
    return subprocess.run(
        arguments,
        cwd=cwd,
        check=True,
        capture_output=True,
        **kwargs,
    )


class OracleGuiBridgeTests(unittest.TestCase):
    def test_the_gui_header_is_the_pins_bytes(self):
        pinned = run(
            "git", "show", f"{PINNED_ORACLE_REVISION}:rust/include/lc_gui_ffi.h"
        ).stdout
        self.assertEqual(pinned, (BRIDGE / "lc_gui_ffi.h").read_bytes())

    def test_the_scenario_example_needs_the_ffi_feature(self):
        # The example measures with the bridge's font from the `ffi` module, so
        # a default workspace test build, which compiles examples, must skip it.
        manifest = (REPOSITORY / "crates" / "clonk-gui" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        self.assertIn('name = "bridge_scenario"', manifest)
        self.assertIn('required-features = ["ffi"]', manifest)
        self.assertTrue(
            (REPOSITORY / "crates" / "clonk-gui" / "examples" / "bridge_scenario.rs").is_file()
        )

    def test_harness_and_scenario_run_the_same_script(self):
        # The differential diffs the two dumps byte for byte, so the script's
        # fixed strings must agree at the source.
        harness = (BRIDGE / "gui-harness.cpp").read_text(encoding="utf-8")
        scenario = (
            REPOSITORY / "crates" / "clonk-gui" / "examples" / "bridge_scenario.rs"
        ).read_text(encoding="utf-8")
        for literal in ('"Hello, bridge"', '"Hello, bridge!"', '"Press me"', '"Nested"', "320.0", "240.0"):
            self.assertIn(literal, harness, literal)
            self.assertIn(literal, scenario, literal)
        for event in (
            '"move"',
            '"down"',
            '"up"',
            '"down-outside"',
            '"up-outside"',
            '"key-down-tab"',
            '"key-down-enter"',
            '"key-up-enter"',
            '"key-down-escape"',
        ):
            self.assertIn(event, harness, event)
            self.assertIn(event, scenario, event)
        self.assertIn("--perturb", harness)
        self.assertIn("--perturb", scenario)

    def test_build_script_offers_the_gui_differential_only_with_the_option(self):
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
                for source in (BUILD_SCRIPT, *PATCHES):
                    shutil.copy2(source, bridge / source.name)
                for header in HEADERS:
                    shutil.copy2(BRIDGE / header, bridge / header)

                fake_bin = root / "bin"
                fake_bin.mkdir()
                cmake_log = root / "cmake.log"
                for command, body in (
                    ("cmake", f'#!/bin/sh\nprintf \'%s\\n\' "$@" >> "{cmake_log}"\nexit 0\n'),
                    ("xcrun", "#!/bin/sh\nprintf '/tmp/fake-sdk\\n'\n"),
                ):
                    executable = fake_bin / command
                    executable.write_text(body, encoding="utf-8")
                    executable.chmod(0o755)

                environment = os.environ.copy()
                environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
                script = str(bridge / BUILD_SCRIPT.name)

                def build(*extra):
                    cmake_log.write_text("", encoding="utf-8")
                    return subprocess.run(
                        [script, "--oracle-root", str(oracle), *extra],
                        check=False,
                        capture_output=True,
                        text=True,
                        env=environment,
                    )

                without = build()
                self.assertEqual(without.returncode, 0, without.stdout + without.stderr)
                self.assertNotIn("run-gui-differential.sh", without.stdout)
                self.assertIn("-DUSE_RUST_GUI_VALIDATION=OFF", cmake_log.read_text(encoding="utf-8"))

                with_gui = build("--with-gui-validation")
                self.assertEqual(with_gui.returncode, 0, with_gui.stdout + with_gui.stderr)
                self.assertIn("run-gui-differential.sh", with_gui.stdout)
                self.assertIn("-DUSE_RUST_GUI_VALIDATION=ON", cmake_log.read_text(encoding="utf-8"))
                self.assertIn("nothing in the pinned", with_gui.stdout)

                (bridge / "lc_gui_ffi.h").write_bytes(b"// drifted\n")
                drifted = build("--with-gui-validation")
                self.assertNotEqual(drifted.returncode, 0)
                self.assertIn("parity/bridge/lc_gui_ffi.h differs", drifted.stderr)
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
