import pathlib
import tomllib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]
ZIP_CONSUMERS = (
    ("crates/clonk-game/Cargo.toml", "dev-dependencies"),
    ("crates/clonk-launcher/Cargo.toml", "dependencies"),
    ("crates/clonk-update/Cargo.toml", "dependencies"),
    ("xtask/Cargo.toml", "dependencies"),
)


class ZipDependencyContractTests(unittest.TestCase):
    def test_every_direct_consumer_uses_zip_8_with_only_deflate(self):
        for relative, section in ZIP_CONSUMERS:
            with self.subTest(manifest=relative):
                manifest = tomllib.loads(
                    (REPOSITORY / relative).read_text(encoding="utf-8")
                )
                dependency = manifest[section]["zip"]
                self.assertEqual(dependency["version"], "8")
                self.assertFalse(dependency["default-features"])
                self.assertEqual(dependency["features"], ["deflate"])

    def test_lockfile_contains_one_zip_8_crate(self):
        cargo_lock = tomllib.loads(
            (REPOSITORY / "Cargo.lock").read_text(encoding="utf-8")
        )
        locked = [
            package["version"]
            for package in cargo_lock["package"]
            if package["name"] == "zip"
        ]
        self.assertEqual(len(locked), 1, "the workspace must share one zip crate")
        self.assertEqual(locked[0].split(".", 1)[0], "8")


if __name__ == "__main__":
    unittest.main()
