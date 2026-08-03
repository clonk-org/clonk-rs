import pathlib
import tomllib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]
DEPENDENCY_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def manifest(relative_path):
    return tomllib.loads((REPOSITORY / relative_path).read_text(encoding="utf-8"))


def workspace_manifest_paths():
    return tuple(
        pathlib.Path(member) / "Cargo.toml"
        for member in manifest("Cargo.toml")["workspace"]["members"]
    )


def dependency_declarations(cargo_manifest, package_name):
    declarations = []
    tables = [
        (section, cargo_manifest.get(section, {}))
        for section in DEPENDENCY_SECTIONS
    ]
    for target_name, target in cargo_manifest.get("target", {}).items():
        tables.extend(
            (f"target.{target_name}.{section}", target.get(section, {}))
            for section in DEPENDENCY_SECTIONS
        )
    for table_name, dependencies in tables:
        for alias, declaration in dependencies.items():
            declared_package = (
                declaration.get("package", alias)
                if isinstance(declaration, dict)
                else alias
            )
            if declared_package == package_name:
                declarations.append((table_name, alias, declaration))
    return declarations


class ZipDependencyContractTests(unittest.TestCase):
    def test_dependency_scan_covers_all_tables_and_renamed_packages(self):
        cargo_manifest = {
            "dependencies": {"runtime_zip": {"package": "zip", "version": "8.1"}},
            "dev-dependencies": {"zip": "8.1"},
            "build-dependencies": {
                "build_zip": {"package": "zip", "version": "8.1"}
            },
            "target": {
                "cfg(windows)": {
                    "dependencies": {
                        "target_zip": {"package": "zip", "version": "8.1"}
                    }
                }
            },
        }

        self.assertEqual(
            dependency_declarations(cargo_manifest, "zip"),
            [
                ("dependencies", "runtime_zip", {"package": "zip", "version": "8.1"}),
                ("dev-dependencies", "zip", "8.1"),
                (
                    "build-dependencies",
                    "build_zip",
                    {"package": "zip", "version": "8.1"},
                ),
                (
                    "target.cfg(windows).dependencies",
                    "target_zip",
                    {"package": "zip", "version": "8.1"},
                ),
            ],
        )

    def test_every_direct_consumer_uses_zip_8_1_with_the_rust_deflate_backend(self):
        manifests = workspace_manifest_paths()
        self.assertEqual(len(manifests), 31)
        self.assertEqual(len(set(manifests)), len(manifests))
        self.assertIn(pathlib.Path("third_party/pixels/Cargo.toml"), manifests)

        zip_dependency = {
            "version": "8.1",
            "default-features": False,
            "features": ["deflate-flate2"],
        }
        optional_zip_dependency = {**zip_dependency, "optional": True}
        backend_dependency = {
            "version": "1",
            "default-features": False,
            "features": ["rust_backend"],
        }
        optional_backend_dependency = {**backend_dependency, "optional": True}
        expected_zip = {
            "crates/clonk-game/Cargo.toml": [
                ("dev-dependencies", "zip", zip_dependency)
            ],
            "crates/clonk-launcher/Cargo.toml": [
                ("dependencies", "zip", zip_dependency)
            ],
            "crates/clonk-update/Cargo.toml": [
                ("dependencies", "zip", zip_dependency)
            ],
            "xtask/Cargo.toml": [
                ("dependencies", "zip", optional_zip_dependency)
            ],
        }
        expected_flate2 = {
            "crates/clonk-app-netplay/Cargo.toml": [
                ("dev-dependencies", "flate2", "1")
            ],
            "crates/clonk-app/Cargo.toml": [("dev-dependencies", "flate2", "1")],
            "crates/clonk-game/Cargo.toml": [
                ("dev-dependencies", "flate2", backend_dependency)
            ],
            "crates/clonk-launcher/Cargo.toml": [
                ("dependencies", "flate2", backend_dependency)
            ],
            "crates/clonk-network/Cargo.toml": [
                ("dependencies", "flate2", "1")
            ],
            "crates/clonk-resources/Cargo.toml": [
                ("dependencies", "flate2", "1")
            ],
            "crates/clonk-update/Cargo.toml": [
                ("dependencies", "flate2", backend_dependency)
            ],
            "xtask/Cargo.toml": [
                ("dependencies", "flate2", optional_backend_dependency)
            ],
        }
        actual_zip = {}
        actual_flate2 = {}
        for relative_path in manifests:
            cargo_manifest = manifest(relative_path)
            relative_path = relative_path.as_posix()
            if declarations := dependency_declarations(cargo_manifest, "zip"):
                actual_zip[relative_path] = declarations
            if declarations := dependency_declarations(cargo_manifest, "flate2"):
                actual_flate2[relative_path] = declarations

        self.assertEqual(actual_zip, expected_zip)
        self.assertEqual(actual_flate2, expected_flate2)

    def test_lockfile_contains_one_zip_8_crate(self):
        cargo_lock = manifest("Cargo.lock")
        package_names = {package["name"] for package in cargo_lock["package"]}
        locked = [
            package["version"]
            for package in cargo_lock["package"]
            if package["name"] == "zip"
        ]
        self.assertEqual(len(locked), 1, "the workspace must share one zip crate")
        self.assertEqual(locked[0].split(".", 1)[0], "8")
        self.assertTrue(
            {"zlib-rs", "zopfli"}.isdisjoint(package_names),
            "zip must not switch the workspace's flate2 backend away from miniz",
        )

if __name__ == "__main__":
    unittest.main()
