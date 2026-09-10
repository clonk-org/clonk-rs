import re
import unittest

from _repo import REPOSITORY, manifest


WORKFLOWS = REPOSITORY / ".github/workflows"
MSVC_RUNTIME_CONFIG = REPOSITORY / "scripts/configure-msvc-runtime.sh"
ACTION_PIN = re.compile(
    r"uses: dtolnay/rust-toolchain@(?P<sha>[0-9a-f]{40}) # (?P<version>\S+)"
)


class RustToolchainPinTests(unittest.TestCase):
    """Renovate advances `rust-toolchain.toml` but is disabled for the
    `dtolnay/rust-toolchain` action, whose commits are cut per channel. A bump
    that moves only the channel leaves CI installing extra targets for the old
    compiler while rustup runs the new one, and the MSVC runtime contract
    rejects the new `rustc -vV`."""

    @classmethod
    def setUpClass(cls):
        cls.channel = manifest("rust-toolchain.toml")["toolchain"]["channel"]
        cls.workflows = {
            path.name: path.read_text(encoding="utf-8")
            for path in sorted(WORKFLOWS.glob("*.yml"))
        }

    def test_every_workflow_installs_the_checked_in_channel(self):
        pins = [
            (name, match)
            for name, workflow in self.workflows.items()
            for match in ACTION_PIN.finditer(workflow)
        ]
        self.assertTrue(pins, "no dtolnay/rust-toolchain pins found")
        for name, match in pins:
            with self.subTest(workflow=name):
                self.assertEqual(match["version"], self.channel)
        self.assertEqual(
            len({match["sha"] for _, match in pins}),
            1,
            "every workflow must pin the one commit cut for the channel",
        )

    def test_msvc_runtime_contract_expects_the_checked_in_channel(self):
        script = MSVC_RUNTIME_CONFIG.read_text(encoding="utf-8")
        self.assertIn(f"grep -Fqx 'release: {self.channel}'", script)
        self.assertIn(
            f"expected_toolchain={self.channel}-x86_64-pc-windows-msvc", script
        )

    def test_preinstalled_rust_probe_requires_the_checked_in_channel(self):
        self.assertIn(
            f"required='rustc {self.channel} '", self.workflows["landing.yml"]
        )

    def test_thinlto_cache_keys_name_the_checked_in_channel(self):
        for name in ("rust.yml", "release-prebuild.yml"):
            with self.subTest(workflow=name):
                keys = re.findall(
                    r"clonk-msvc-thinlto-v2-windows-x64-rustc-([^-]+)-",
                    self.workflows[name],
                )
                self.assertTrue(keys)
                self.assertEqual(set(keys), {self.channel})


if __name__ == "__main__":
    unittest.main()
