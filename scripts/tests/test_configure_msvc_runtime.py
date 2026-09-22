"""Exercise the native compiler policy used by shipped Windows builds."""

import json
import os
import pathlib
import subprocess
import tempfile
import unittest

from _repo import REPOSITORY


class ConfigureMsvcRuntimeTests(unittest.TestCase):
    def test_native_archives_disable_ipo_when_rust_uses_linker_plugin_lto(self):
        with tempfile.TemporaryDirectory(prefix="msvc runtime ") as temporary:
            root = pathlib.Path(temporary)
            commands = root / "commands"
            commands.mkdir()
            libdir = root / "toolchain" / "lib"
            libdir.mkdir(parents=True)
            linker = libdir.parent / "bin" / "rust-lld.exe"
            linker.parent.mkdir()
            linker.write_text("#!/bin/sh\necho 'LLD 22.1.8'\n")
            linker.chmod(0o755)
            stubs = {
                "cygpath": '#!/bin/sh\nprintf "%s\\n" "$2"\n',
                "rustc": (
                    '#!/bin/sh\nif [ "$1" = -vV ]; then\n'
                    "  printf 'release: 1.98.1\\nLLVM version: 22.1.8\\n'\n"
                    'else\n  printf "%s\\n" "$TEST_LIBDIR"\nfi\n'
                ),
                "rustup": (
                    '#!/bin/sh\n[ "$*" = "toolchain list --quiet" ] || exit 1\n'
                    "echo 1.98.1-x86_64-pc-windows-msvc\n"
                ),
            }
            for name, source in stubs.items():
                command = commands / name
                command.write_text(source)
                command.chmod(0o755)
            environment_file = root / "github-env"
            environment = {
                **os.environ,
                "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                "GITHUB_ENV": str(environment_file),
                "RUNNER_TEMP": str(root),
                "TEST_LIBDIR": str(libdir),
            }
            subprocess.run(
                ["bash", str(REPOSITORY / "scripts/configure-msvc-runtime.sh")],
                env=environment, check=True, capture_output=True, text=True,
            )
            exported = dict(
                line.split("=", 1) for line in environment_file.read_text().splitlines()
            )
            self.assertIn("-Clinker-plugin-lto", exported["RUSTFLAGS"])
            toolchain = exported["CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc"]
            source = root / "source"
            source.mkdir()
            (source / "native.c").write_text("int native_symbol(void) { return 42; }\n")
            (source / "CMakeLists.txt").write_text(
                "cmake_minimum_required(VERSION 3.16)\n"
                "project(native_archive C)\n"
                "add_library(native STATIC native.c)\n"
            )
            for configuration in ("Release", "MinSizeRel", "RelWithDebInfo", "Debug"):
                with self.subTest(configuration=configuration):
                    build = root / configuration
                    result = subprocess.run(
                        [
                            "cmake", "-S", str(source), "-B", str(build),
                            f"-DCMAKE_TOOLCHAIN_FILE={toolchain}",
                            f"-DCMAKE_BUILD_TYPE={configuration}",
                            # opusic-sys enables this when Rust requests plugin LTO.
                            "-DCMAKE_INTERPROCEDURAL_OPTIMIZATION=ON",
                            "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON",
                        ],
                        capture_output=True, text=True,
                    )
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    command = json.loads((build / "compile_commands.json").read_text())[0]["command"]
                    self.assertNotIn("-flto", command)
                    self.assertNotIn("/GL", command)
                    result = subprocess.run(
                        ["cmake", "--build", str(build)], capture_output=True, text=True,
                    )
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
