import tomllib
import unittest

from _repo import REPOSITORY


class DesktopDependencyContractTests(unittest.TestCase):
    def test_desktop_integrations_use_the_reviewed_major_versions(self):
        app = tomllib.loads(
            (REPOSITORY / "crates" / "clonk-app" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        launcher = tomllib.loads(
            (
                REPOSITORY / "crates" / "clonk-launcher-shell" / "Cargo.toml"
            ).read_text(encoding="utf-8")
        )

        expectations = (
            ("clonk-app gilrs", app["dependencies"]["gilrs"], "0.11"),
            ("clonk-app rfd", app["dependencies"]["rfd"], "0.17"),
            (
                "clonk-launcher-shell rfd",
                launcher["dependencies"]["rfd"],
                "0.17",
            ),
        )
        for name, actual, expected in expectations:
            with self.subTest(dependency=name):
                self.assertEqual(actual, expected)

        zbus = app["target"]["cfg(target_os = \"linux\")"]["dependencies"]["zbus"]
        zbus_expectations = (
            ("clonk-app zbus version", zbus["version"], "5"),
            (
                "clonk-app zbus default features",
                zbus["default-features"],
                False,
            ),
            (
                "clonk-app zbus features",
                set(zbus["features"]),
                {"async-io", "blocking-api"},
            ),
        )
        for name, actual, expected in zbus_expectations:
            with self.subTest(dependency=name):
                self.assertEqual(actual, expected)

    def test_rfd_upgrade_drops_the_obsolete_block_patch(self):
        workspace = tomllib.loads(
            (REPOSITORY / "Cargo.toml").read_text(encoding="utf-8")
        )
        lockfile = tomllib.loads((REPOSITORY / "Cargo.lock").read_text(encoding="utf-8"))

        self.assertNotIn("block", workspace["patch"]["crates-io"])
        self.assertFalse((REPOSITORY / "third_party" / "block").exists())
        self.assertNotIn("block", {package["name"] for package in lockfile["package"]})


if __name__ == "__main__":
    unittest.main()
