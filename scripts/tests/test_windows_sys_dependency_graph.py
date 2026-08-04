import tomllib
import unittest

from _repo import REPOSITORY


class WindowsSysDependencyGraphTests(unittest.TestCase):
    def test_direct_windows_sys_bindings_use_the_current_handle_model(self):
        for crate in (
            "clonk-core",
            "clonk-game",
            "clonk-platform",
            "clonk-resources",
            "clonk-update",
        ):
            with self.subTest(crate=crate):
                manifest = tomllib.loads(
                    (REPOSITORY / "crates" / crate / "Cargo.toml").read_text(
                        encoding="utf-8"
                    )
                )
                dependency = manifest["target"]["cfg(windows)"]["dependencies"][
                    "windows-sys"
                ]
                self.assertEqual(dependency["version"], "0.61")

    def test_consumers_declare_the_security_feature_their_win32_calls_require(self):
        for crate in ("clonk-core", "clonk-platform"):
            with self.subTest(crate=crate):
                manifest = tomllib.loads(
                    (REPOSITORY / "crates" / crate / "Cargo.toml").read_text(
                        encoding="utf-8"
                    )
                )
                dependency = manifest["target"]["cfg(windows)"]["dependencies"][
                    "windows-sys"
                ]
                self.assertIn("Win32_Security", dependency["features"])


if __name__ == "__main__":
    unittest.main()
