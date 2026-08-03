import re
import tomllib
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
NETWORK = REPOSITORY / "crates" / "clonk-network"
UPDATER = REPOSITORY / "crates" / "clonk-update-net"


def manifest(crate: Path) -> dict:
    with (crate / "Cargo.toml").open("rb") as handle:
        return tomllib.load(handle)


class ReqwestDependencyContractTests(unittest.TestCase):
    def test_direct_consumers_use_reqwest_013_with_the_ring_rustls_backend(self):
        expected_features = {
            NETWORK: {"cookies", "gzip", "rustls-no-provider"},
            UPDATER: {"rustls-no-provider"},
        }

        for crate, features in expected_features.items():
            with self.subTest(crate=crate.name):
                dependencies = manifest(crate)["dependencies"]
                reqwest = dependencies["reqwest"]
                self.assertEqual(reqwest["version"], "0.13")
                self.assertFalse(reqwest["default-features"])
                self.assertEqual(set(reqwest["features"]), features)

                rustls = dependencies["rustls"]
                self.assertEqual(rustls["version"], "0.23")
                self.assertFalse(rustls["default-features"])
                self.assertEqual(set(rustls["features"]), {"ring"})

    def test_every_shipped_client_explicitly_uses_only_bundled_mozilla_roots(self):
        for crate, source in {
            NETWORK: NETWORK / "src" / "http_backend.rs",
            UPDATER: UPDATER / "src" / "transport.rs",
        }.items():
            with self.subTest(crate=crate.name):
                dependencies = manifest(crate)["dependencies"]
                self.assertEqual(dependencies["webpki-root-certs"], "1")

                implementation = source.read_text(encoding="utf-8")
                self.assertIn("webpki_root_certs::TLS_SERVER_ROOT_CERTS", implementation)
                self.assertIn(".tls_backend_rustls()", implementation)
                self.assertIn(".tls_certs_only(", implementation)

                production_sources = "\n".join(
                    path.read_text(encoding="utf-8").split("#[cfg(test)]", 1)[0]
                    for path in (crate / "src").rglob("*.rs")
                )
                raw_builders = re.findall(
                    r"(?<![A-Za-z_])(?:reqwest::)?Client::builder\(\)",
                    production_sources,
                )
                self.assertEqual(
                    raw_builders,
                    ["reqwest::Client::builder()"],
                    "production clients must start from the bundled-root helper",
                )


if __name__ == "__main__":
    unittest.main()
