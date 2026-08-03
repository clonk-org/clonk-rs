import pathlib
import tomllib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


def manifest(relative_path):
    return tomllib.loads((REPOSITORY / relative_path).read_text(encoding="utf-8"))


def dependency_declarations(cargo_manifest, dependency_name):
    declarations = []
    dependency_sections = ("dependencies", "dev-dependencies", "build-dependencies")
    for section in dependency_sections:
        if dependency_name in cargo_manifest.get(section, {}):
            declarations.append(cargo_manifest[section][dependency_name])
    for target in cargo_manifest.get("target", {}).values():
        for section in dependency_sections:
            if dependency_name in target.get(section, {}):
                declarations.append(target[section][dependency_name])
    return declarations


class LowRiskDependencyContractTests(unittest.TestCase):
    def test_criterion_uses_the_supported_release(self):
        for relative_path in (
            "crates/clonk-engine/Cargo.toml",
            "crates/clonk-script/Cargo.toml",
        ):
            with self.subTest(manifest=relative_path):
                dependency = manifest(relative_path)["dependencies"]["criterion"]
                self.assertEqual(dependency["version"], "0.8")

    def test_libloading_uses_the_supported_release(self):
        dependency = manifest("crates/clonk-audio/Cargo.toml")["dependencies"][
            "libloading"
        ]
        self.assertEqual(dependency, "0.9")

    def test_sha1_uses_the_supported_release_without_unused_features(self):
        for relative_path in (
            "crates/clonk-app/Cargo.toml",
            "crates/clonk-network/Cargo.toml",
        ):
            with self.subTest(manifest=relative_path):
                dependency = manifest(relative_path)["dependencies"]["sha1"]
                self.assertEqual(
                    dependency,
                    {"version": "0.11", "default-features": False},
                )

    def test_workspace_thiserror_dependencies_use_one_major_version(self):
        expected_manifests = {
            "crates/clonk-app-render/Cargo.toml",
            "crates/clonk-app-netplay/Cargo.toml",
            "crates/clonk-app/Cargo.toml",
            "crates/clonk-audio/Cargo.toml",
            "crates/clonk-engine-integration-tests/Cargo.toml",
            "crates/clonk-engine-unit-tests/Cargo.toml",
            "crates/clonk-engine/Cargo.toml",
            "crates/clonk-graphics/Cargo.toml",
            "crates/clonk-network/Cargo.toml",
            "crates/clonk-platform/Cargo.toml",
            "crates/clonk-resources/Cargo.toml",
            "crates/clonk-script/Cargo.toml",
            "crates/clonk-update-net/Cargo.toml",
            "crates/clonk-update/Cargo.toml",
            "third_party/pixels/Cargo.toml",
        }
        actual_manifests = set()
        dependencies_by_manifest = {}
        for member in manifest("Cargo.toml")["workspace"]["members"]:
            relative_path = pathlib.Path(member) / "Cargo.toml"
            cargo_manifest = manifest(relative_path)
            declarations = dependency_declarations(cargo_manifest, "thiserror")
            if declarations:
                relative_path = relative_path.as_posix()
                actual_manifests.add(relative_path)
                dependencies_by_manifest[relative_path] = declarations
        self.assertEqual(actual_manifests, expected_manifests)
        for relative_path in sorted(actual_manifests):
            with self.subTest(manifest=relative_path):
                for dependency in dependencies_by_manifest[relative_path]:
                    version = (
                        dependency
                        if isinstance(dependency, str)
                        else dependency["version"]
                    )
                    self.assertEqual(version, "2")

    def test_network_uses_the_renamed_xml_release(self):
        dependencies = manifest("crates/clonk-network/Cargo.toml")["dependencies"]
        self.assertNotIn("xml-rs", dependencies)
        self.assertEqual(dependencies["xml"], "1")


if __name__ == "__main__":
    unittest.main()
