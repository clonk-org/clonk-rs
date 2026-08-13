import pathlib
import unittest

from _repo import REPOSITORY, manifest

DEPENDENCY_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def dependency_declarations(cargo_manifest, dependency_name):
    declarations = []

    def append_from_table(location, dependency_table):
        for alias, dependency in dependency_table.items():
            package = (
                dependency.get("package", alias)
                if isinstance(dependency, dict)
                else alias
            )
            if package == dependency_name:
                declarations.append((f"{location}.{alias}", dependency))

    for section in DEPENDENCY_SECTIONS:
        append_from_table(section, cargo_manifest.get(section, {}))
    for target_name, target in cargo_manifest.get("target", {}).items():
        for section in DEPENDENCY_SECTIONS:
            append_from_table(
                f"target.{target_name}.{section}", target.get(section, {})
            )
    return declarations


def normalized_dependency(dependency):
    if isinstance(dependency, str):
        return dependency, True, frozenset()
    return (
        dependency["version"],
        dependency.get("default-features", True),
        frozenset(dependency.get("features", [])),
    )


class RandDependencyContractTests(unittest.TestCase):
    def dependency_manifest_paths(self):
        return {
            pathlib.Path(member) / "Cargo.toml"
            for member in manifest("Cargo.toml")["workspace"]["members"]
        }

    def direct_dependencies(self, dependency_name):
        return {
            (relative_path.as_posix(), location): normalized_dependency(dependency)
            for relative_path in self.dependency_manifest_paths()
            for location, dependency in dependency_declarations(
                manifest(relative_path), dependency_name
            )
        }

    def test_dependency_contract_scans_every_workspace_member(self):
        workspace_manifests = {
            pathlib.Path(member) / "Cargo.toml"
            for member in manifest("Cargo.toml")["workspace"]["members"]
        }
        self.assertEqual(len(workspace_manifests), 31)
        self.assertIn(
            pathlib.Path("crates/clonk-surface/Cargo.toml"), workspace_manifests
        )
        self.assertEqual(self.dependency_manifest_paths(), workspace_manifests)

    def test_dependency_discovery_scans_all_dependency_tables(self):
        cargo_manifest = {
            "dependencies": {"rand": "0.10"},
            "dev-dependencies": {"rand": "0.10"},
            "build-dependencies": {
                "random": {"package": "rand", "version": "0.10"}
            },
            "target": {
                'cfg(target_os = "windows")': {
                    "dependencies": {"rand": "0.10"},
                    "dev-dependencies": {"rand": "0.10"},
                    "build-dependencies": {"rand": "0.10"},
                }
            },
        }
        locations = {
            location
            for location, _dependency in dependency_declarations(
                cargo_manifest, "rand"
            )
        }
        self.assertEqual(
            locations,
            {
                "dependencies.rand",
                "dev-dependencies.rand",
                "build-dependencies.random",
                'target.cfg(target_os = "windows").dependencies.rand',
                'target.cfg(target_os = "windows").dev-dependencies.rand',
                'target.cfg(target_os = "windows").build-dependencies.rand',
            },
        )

    def test_direct_rand_users_require_0_10(self):
        self.assertEqual(
            self.direct_dependencies("rand"),
            {
                ("crates/clonk-core/Cargo.toml", "dependencies.rand"): (
                    "0.10",
                    False,
                    frozenset({"std", "sys_rng"}),
                ),
                ("crates/clonk-engine/Cargo.toml", "dependencies.rand"): (
                    "0.10",
                    False,
                    frozenset({"std"}),
                ),
                (
                    "crates/clonk-engine-integration-tests/Cargo.toml",
                    "dev-dependencies.rand",
                ): ("0.10", False, frozenset({"std"})),
                (
                    "crates/clonk-engine-unit-tests/Cargo.toml",
                    "dev-dependencies.rand",
                ): ("0.10", False, frozenset({"std"})),
                (
                    "crates/clonk-graphics/Cargo.toml",
                    "dev-dependencies.rand",
                ): ("0.10", False, frozenset({"std"})),
            },
        )

    def test_direct_rand_core_users_require_0_10(self):
        self.assertEqual(
            self.direct_dependencies("rand_core"),
            {
                (
                    "crates/clonk-engine-core/Cargo.toml",
                    "dependencies.rand_core",
                ): ("0.10", True, frozenset()),
                (
                    "crates/clonk-engine/Cargo.toml",
                    "dependencies.rand_core",
                ): ("0.10", True, frozenset()),
                (
                    "crates/clonk-engine-integration-tests/Cargo.toml",
                    "dev-dependencies.rand_core",
                ): ("0.10", True, frozenset()),
                (
                    "crates/clonk-engine-unit-tests/Cargo.toml",
                    "dev-dependencies.rand_core",
                ): ("0.10", True, frozenset()),
            },
        )


if __name__ == "__main__":
    unittest.main()
