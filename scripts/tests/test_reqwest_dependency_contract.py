import re
import tomllib
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
NETWORK = REPOSITORY / "crates" / "clonk-network"
UPDATER = REPOSITORY / "crates" / "clonk-update-net"
PIXELS = REPOSITORY / "third_party" / "pixels"


def manifest(crate: Path) -> dict:
    with (crate / "Cargo.toml").open("rb") as handle:
        return tomllib.load(handle)


def workspace_members() -> tuple[Path, ...]:
    members = manifest(REPOSITORY)["workspace"]["members"]
    return tuple(REPOSITORY / member for member in members)


def dependency_tables(contents: dict):
    table_names = ("dependencies", "dev-dependencies", "build-dependencies")
    for table_name in table_names:
        yield table_name, contents.get(table_name, {})
    for target_name, target in contents.get("target", {}).items():
        for table_name in table_names:
            yield f"target.{target_name}.{table_name}", target.get(table_name, {})


def workspace_dependency_declarations(package: str) -> dict:
    declarations = {}
    for member in workspace_members():
        for table_name, dependencies in dependency_tables(manifest(member)):
            for alias, requirement in dependencies.items():
                declared_package = (
                    requirement.get("package", alias)
                    if isinstance(requirement, dict)
                    else alias
                )
                if declared_package == package:
                    location = (
                        member.relative_to(REPOSITORY).as_posix(),
                        table_name,
                        alias,
                    )
                    declarations[location] = requirement
    return declarations


class ReqwestDependencyContractTests(unittest.TestCase):
    def test_contract_covers_all_31_workspace_members_including_vendored_pixels(self):
        covered_members = set(workspace_members())
        self.assertEqual((len(covered_members), PIXELS in covered_members), (31, True))

    def test_direct_consumers_use_reqwest_013_with_the_ring_rustls_backend(self):
        expected_features = {
            ("crates/clonk-network", "dependencies", "reqwest"): {
                "cookies",
                "gzip",
                "rustls-no-provider",
            },
            ("crates/clonk-update-net", "dependencies", "reqwest"): {
                "rustls-no-provider"
            },
        }
        reqwest_declarations = workspace_dependency_declarations("reqwest")
        self.assertEqual(set(reqwest_declarations), set(expected_features))

        for location, features in expected_features.items():
            with self.subTest(location=location):
                reqwest = reqwest_declarations[location]
                self.assertEqual(
                    set(reqwest), {"version", "default-features", "features"}
                )
                self.assertEqual(reqwest["version"], "0.13")
                self.assertFalse(reqwest["default-features"])
                self.assertEqual(set(reqwest["features"]), features)

        expected_rustls = {
            ("crates/clonk-network", "dependencies", "rustls"),
            ("crates/clonk-update-net", "dependencies", "rustls"),
        }
        rustls_declarations = workspace_dependency_declarations("rustls")
        self.assertEqual(set(rustls_declarations), expected_rustls)
        for location, rustls in rustls_declarations.items():
            with self.subTest(location=location):
                self.assertEqual(
                    set(rustls), {"version", "default-features", "features"}
                )
                self.assertEqual(rustls["version"], "0.23")
                self.assertFalse(rustls["default-features"])
                self.assertEqual(set(rustls["features"]), {"ring"})

        expected_roots = {
            ("crates/clonk-network", "dependencies", "webpki-root-certs"): "1",
            ("crates/clonk-update-net", "dependencies", "webpki-root-certs"): "1",
        }
        self.assertEqual(
            workspace_dependency_declarations("webpki-root-certs"), expected_roots
        )

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
