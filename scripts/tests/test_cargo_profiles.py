"""Static guards for the measured release and test-profile codegen balance."""

import tomllib
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
MANIFEST = REPOSITORY / "Cargo.toml"


class CargoProfilesTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.profiles = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))["profile"]

    def test_shipped_release_profile_keeps_measured_codegen_balance(self):
        release = self.profiles["release"]

        self.assertEqual(release["lto"], "thin")
        self.assertEqual(release["codegen-units"], 1)
        self.assertEqual(
            release["package"],
            {"clonk-app": {"codegen-units": 8}},
        )

    def test_test_profile_keeps_explicit_parallel_codegen(self):
        test = self.profiles["test"]

        self.assertEqual(test["inherits"], "release")
        self.assertEqual(test["codegen-units"], 256)
        self.assertEqual(test["package"]["clonk-app"]["codegen-units"], 256)


if __name__ == "__main__":
    unittest.main()
