import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

REPOSITORY = Path(__file__).resolve().parents[2]
PIN = "7d43b47b7d789b533f32d005e64596e0a07019cd"
BRIDGE = REPOSITORY / "parity/bridge"
PATCH = BRIDGE / "oracle-group-bridge.patch"


class OracleGroupPatchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temporary.cleanup)
        root = Path(cls.temporary.name)
        source = root / "src/rust"
        cls.source = source
        source.mkdir(parents=True)
        for name in ("RustGroupBridge.cpp", "RustGroupBridge.h"):
            result = subprocess.run(
                ["git", "show", f"{PIN}:src/rust/{name}"],
                cwd=REPOSITORY, check=True, capture_output=True,
            )
            (source / name).write_bytes(result.stdout)
        (source.parent / "C4Group.cpp").write_bytes(subprocess.check_output(
            ["git", "show", f"{PIN}:src/C4Group.cpp"], cwd=REPOSITORY,
        ))
        if PATCH.exists():
            subprocess.run(["git", "apply", str(PATCH)], cwd=root, check=True)
        cls.executable = root / "diagnostic-probe"
        subprocess.run(
            [os.environ.get("CXX", "c++"), "-std=c++17", "-DC4ENGINE",
             "-DUSE_RUST_GROUP_VALIDATION", "-I", str(BRIDGE / "tests/group"),
             "-I", str(BRIDGE), "-I", str(source), str(source / "RustGroupBridge.cpp"),
             str(BRIDGE / "tests/group/diagnostic_probe.cpp"), "-o", str(cls.executable)],
            check=True, capture_output=True,
        )

    def test_agreeing_entries_emit_no_mismatch_and_release_the_complete_array(self):
        # RustGroupBridge.cpp:159-183 builds the Rust view and owns its array;
        # :214-216 emits no mismatch for agreement. The oracle is the actual
        # pinned C++ function, with fixture entry enumerators on both sides.
        result = subprocess.run([str(self.executable)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout, "")

    def test_entry_count_is_owned_after_the_ffi_call_in_either_argument_order(self):
        # C++17 permits evaluating the constructor's count argument before
        # lc_group_entries updates len (pinned RustGroupBridge.cpp:167-168).
        # Make that permitted lowering explicit so this fails on Clang/ARM
        # too, rather than depending on the compiler choosing it naturally.
        source = (self.source / "RustGroupBridge.cpp").read_text()
        source = source.replace(
            "EntryArrayPtr ffi_entries(lc_group_entries(handle.get(), &len), len);",
            "const auto count_argument = len;\n"
            "    EntryArrayPtr ffi_entries(lc_group_entries(handle.get(), &len), count_argument);",
        )
        lowered = self.source / "argument-order.cpp"
        lowered.write_text(source)
        executable = self.source / "argument-order-probe"
        subprocess.run(
            [os.environ.get("CXX", "c++"), "-std=c++17", "-DC4ENGINE",
             "-DUSE_RUST_GROUP_VALIDATION", "-I", str(BRIDGE / "tests/group"),
             "-I", str(BRIDGE), "-I", str(self.source), str(lowered),
             str(BRIDGE / "tests/group/diagnostic_probe.cpp"), "-o", str(executable)],
            check=True, capture_output=True,
        )
        result = subprocess.run([str(executable)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_each_disagreement_is_reported_and_canonical_names_agree(self):
        # RustGroupBridge.cpp:39-47 canonicalises names; :190-240 reports
        # each kind of disagreement without changing either group.
        for fixture, report in (
            ("missing", "entries missing from Rust view: Beta.txt"),
            ("additional", "additional entries reported by Rust: Extra.txt"),
            ("size", "size mismatch for entries: Alpha.txt"),
            ("type", "entry type mismatch for: Alpha.txt"),
            ("canonical", ""),
        ):
            with self.subTest(fixture=fixture):
                result = subprocess.run(
                    [str(self.executable), fixture], capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                if report:
                    self.assertEqual(result.stdout.count("Rust group validation"), 1)
                    self.assertIn(report, result.stdout)
                else:
                    self.assertEqual(result.stdout, "")

    def test_builder_applies_group_patch_idempotently_and_rejects_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            oracle = root / "oracle"
            subprocess.run(
                ["git", "worktree", "add", "--detach", str(oracle), PIN],
                cwd=REPOSITORY, check=True, capture_output=True,
            )
            try:
                bridge = root / "port/parity/bridge"
                bridge.mkdir(parents=True)
                for name in (
                    "build-oracle-validation.sh", "oracle-weather.patch",
                    "oracle-group-bridge.patch", "lc_engine_ffi.h",
                    "lc_config_ffi.h", "lc_group_ffi.h", "lc_platform_ffi.h",
                ):
                    shutil.copy2(BRIDGE / name, bridge / name)
                commands = root / "bin"
                commands.mkdir()
                for name, body in (
                    ("cmake", "#!/bin/sh\nexit 23\n"),
                    ("xcrun", "#!/bin/sh\nprintf '/tmp/fake-sdk\\n'\n"),
                ):
                    command = commands / name
                    command.write_text(body)
                    command.chmod(0o755)
                environment = dict(os.environ, PATH=f"{commands}:{os.environ['PATH']}")

                def build():
                    return subprocess.run(
                        [str(bridge / "build-oracle-validation.sh"), "--oracle-root",
                         str(oracle), "--with-group-validation"],
                        env=environment, capture_output=True, text=True,
                    )

                for _ in range(2):
                    result = build()
                    self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
                    applied = subprocess.run(
                        ["git", "apply", "--reverse", "--check", str(PATCH)],
                        cwd=oracle, capture_output=True, text=True,
                    )
                    self.assertEqual(applied.returncode, 0, applied.stderr)
                # CMake imports port/target regardless of the caller's Cargo
                # environment. A different target could silently reuse old
                # archives left at that import path.
                (commands / "cmake").write_text(
                    '#!/bin/sh\nif [ "$1" = --build ]; then\n'
                    '  printf "cargo target: %s\\n" "$CARGO_TARGET_DIR"\n'
                    '  exit 23\nfi\n'
                )
                python = commands / "python3"
                python.write_text("#!/bin/sh\nexit 0\n")
                python.chmod(0o755)
                environment["CARGO_TARGET_DIR"] = str(root / "unrelated-cache")
                targeted = build()
                self.assertEqual(targeted.returncode, 23, targeted.stdout + targeted.stderr)
                self.assertIn(f"cargo target: {bridge.parents[1]}/target", targeted.stdout)
                (commands / "cmake").write_text("#!/bin/sh\nexit 23\n")
                header = bridge / "lc_group_ffi.h"
                header.write_text("// drifted group ABI\n")
                drifted_header = build()
                self.assertNotEqual(drifted_header.returncode, 23)
                self.assertIn("lc_group_ffi.h differs", drifted_header.stderr)
                shutil.copy2(BRIDGE / header.name, header)
                source = oracle / "src/rust/RustGroupBridge.cpp"
                source.write_text(source.read_text().replace(
                    "EntryArrayPtr ffi_entries(raw_entries, len);",
                    "EntryArrayPtr ffi_entries(raw_entries, 0);",
                ))
                drifted = build()
                self.assertNotEqual(drifted.returncode, 23)
                self.assertIn("group-bridge patch", drifted.stderr)
            finally:
                subprocess.run(
                    ["git", "worktree", "remove", "--force", str(oracle)],
                    cwd=REPOSITORY, check=True, capture_output=True,
                )


if __name__ == "__main__":
    unittest.main()
