import json
import pathlib
import subprocess
import unittest

from _repo import REPOSITORY, manifest

CARGO_REGISTRY_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"


def workspace_packages():
    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
        ],
        cwd=REPOSITORY,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(completed.stdout)
    packages_by_id = {package["id"]: package for package in metadata["packages"]}
    return tuple(packages_by_id[member] for member in metadata["workspace_members"])


def metadata_dependency_location(dependency):
    section = {
        None: "dependencies",
        "dev": "dev-dependencies",
        "build": "build-dependencies",
    }[dependency["kind"]]
    alias = dependency["rename"] or dependency["name"]
    location = f"{section}.{alias}"
    if target := dependency["target"]:
        location = f"target.{target}.{location}"
    return location


def relative_manifest_path(package):
    return pathlib.Path(package["manifest_path"]).resolve().relative_to(REPOSITORY)


def normalized_metadata_dependency(dependency):
    return (
        dependency["req"],
        dependency["uses_default_features"],
        frozenset(dependency["features"]),
        dependency["optional"],
        dependency["source"],
        dependency["registry"],
    )


def direct_dependencies(packages, package_name):
    return {
        (
            relative_manifest_path(package).as_posix(),
            metadata_dependency_location(dependency),
        ): normalized_metadata_dependency(dependency)
        for package in packages
        for dependency in package["dependencies"]
        if dependency["name"] == package_name
    }


def locked_package(cargo_lock, package_name):
    packages = [
        package
        for package in cargo_lock["package"]
        if package["name"] == package_name
    ]
    if len(packages) != 1:
        raise AssertionError(
            f"expected one locked {package_name} package, found {len(packages)}"
        )
    return packages[0]


def locked_crates_io_package(cargo_lock, package_name):
    package = locked_package(cargo_lock, package_name)
    if package.get("source") != CARGO_REGISTRY_SOURCE:
        raise AssertionError(f"locked {package_name} must come from crates.io")
    checksum = package.get("checksum", "")
    if len(checksum) != 64 or any(
        character not in "0123456789abcdef" for character in checksum
    ):
        raise AssertionError(f"locked {package_name} must retain its crates.io checksum")
    return package


def locked_dependency_names(package):
    return {
        dependency.partition(" ")[0]
        for dependency in package.get("dependencies", [])
    }


class ZipDependencyContractTests(unittest.TestCase):
    def test_metadata_locations_cover_all_tables_targets_and_renames(self):
        dependency = {
            "name": "zip",
            "rename": "runtime_zip",
            "kind": "build",
            "target": 'cfg(target_os = "windows")',
        }
        self.assertEqual(
            metadata_dependency_location(dependency),
            'target.cfg(target_os = "windows").build-dependencies.runtime_zip',
        )
        dependency.update({"kind": "dev", "target": None, "rename": None})
        self.assertEqual(metadata_dependency_location(dependency), "dev-dependencies.zip")
        dependency["kind"] = None
        self.assertEqual(metadata_dependency_location(dependency), "dependencies.zip")

    def test_every_direct_consumer_uses_zip_8_1_with_the_rust_deflate_backend(self):
        packages = workspace_packages()
        manifests = {relative_manifest_path(package) for package in packages}
        self.assertEqual(len(manifests), len(packages))
        self.assertIn(pathlib.Path("crates/clonk-surface/Cargo.toml"), manifests)

        zip_dependency = (
            "^8.1",
            False,
            frozenset({"deflate-flate2"}),
            False,
            CARGO_REGISTRY_SOURCE,
            None,
        )
        optional_zip_dependency = (
            "^8.1",
            False,
            frozenset({"deflate-flate2"}),
            True,
            CARGO_REGISTRY_SOURCE,
            None,
        )
        backend_dependency = (
            "^1",
            False,
            frozenset({"rust_backend"}),
            False,
            CARGO_REGISTRY_SOURCE,
            None,
        )
        optional_backend_dependency = (
            "^1",
            False,
            frozenset({"rust_backend"}),
            True,
            CARGO_REGISTRY_SOURCE,
            None,
        )
        default_flate2 = (
            "^1",
            True,
            frozenset(),
            False,
            CARGO_REGISTRY_SOURCE,
            None,
        )
        expected_zip = {
            ("crates/clonk-game/Cargo.toml", "dev-dependencies.zip"): zip_dependency,
            ("crates/clonk-launcher/Cargo.toml", "dependencies.zip"): zip_dependency,
            ("crates/clonk-update/Cargo.toml", "dependencies.zip"): zip_dependency,
            ("xtask/Cargo.toml", "dependencies.zip"): optional_zip_dependency,
        }
        expected_flate2 = {
            (
                "crates/clonk-app-netplay/Cargo.toml",
                "dev-dependencies.flate2",
            ): default_flate2,
            ("crates/clonk-app/Cargo.toml", "dev-dependencies.flate2"): default_flate2,
            (
                "crates/clonk-game/Cargo.toml",
                "dev-dependencies.flate2",
            ): backend_dependency,
            (
                "crates/clonk-launcher/Cargo.toml",
                "dependencies.flate2",
            ): backend_dependency,
            (
                "crates/clonk-network/Cargo.toml",
                "dependencies.flate2",
            ): default_flate2,
            (
                "crates/clonk-resources/Cargo.toml",
                "dependencies.flate2",
            ): default_flate2,
            (
                "crates/clonk-update/Cargo.toml",
                "dependencies.flate2",
            ): backend_dependency,
            (
                "xtask/Cargo.toml",
                "dependencies.flate2",
            ): optional_backend_dependency,
        }

        self.assertEqual(direct_dependencies(packages, "zip"), expected_zip)
        self.assertEqual(direct_dependencies(packages, "flate2"), expected_flate2)

    def test_lockfile_contains_one_zip_8_crate(self):
        cargo_lock = manifest("Cargo.lock")
        zip_package = locked_crates_io_package(cargo_lock, "zip")
        flate2_package = locked_crates_io_package(cargo_lock, "flate2")
        self.assertEqual(zip_package["version"].split(".", 1)[0], "8")

        flate2_dependencies = locked_dependency_names(flate2_package)
        self.assertIn("miniz_oxide", flate2_dependencies)
        self.assertTrue(
            {
                "libz-sys",
                "libz-ng-sys",
                "cloudflare-zlib-sys",
                "zlib-rs",
            }.isdisjoint(flate2_dependencies),
            flate2_dependencies,
        )
        self.assertNotIn("zopfli", locked_dependency_names(zip_package))

if __name__ == "__main__":
    unittest.main()
