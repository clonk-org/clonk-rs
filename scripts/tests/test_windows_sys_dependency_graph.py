import pathlib
import tomllib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


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

    def test_platform_declares_the_security_feature_its_win32_calls_require(self):
        manifest = tomllib.loads(
            (REPOSITORY / "crates" / "clonk-platform" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        dependency = manifest["target"]["cfg(windows)"]["dependencies"][
            "windows-sys"
        ]
        self.assertIn("Win32_Security", dependency["features"])


if __name__ == "__main__":
    unittest.main()
