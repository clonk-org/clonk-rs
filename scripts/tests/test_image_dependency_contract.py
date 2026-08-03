import pathlib
import tomllib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


class ImageDependencyContractTests(unittest.TestCase):
    def test_direct_image_dependencies_use_current_codec_versions(self):
        expected_image_manifests = {
            "crates/clonk-app-netplay/Cargo.toml",
            "crates/clonk-app/Cargo.toml",
            "crates/clonk-engine-integration-tests/Cargo.toml",
            "crates/clonk-engine-unit-tests/Cargo.toml",
            "crates/clonk-engine/Cargo.toml",
            "crates/clonk-frontend-unit-tests/Cargo.toml",
            "crates/clonk-frontend/Cargo.toml",
            "crates/clonk-icon/Cargo.toml",
            "crates/clonk-resources/Cargo.toml",
            "xtask/Cargo.toml",
        }
        workspace_manifests = [REPOSITORY / "Cargo.toml", REPOSITORY / "xtask/Cargo.toml"]
        workspace_manifests.extend((REPOSITORY / "crates").glob("*/Cargo.toml"))

        def dependency_versions(manifest, dependency_name):
            dependency_tables = [
                manifest.get(table_name, {})
                for table_name in ("dependencies", "dev-dependencies", "build-dependencies")
            ]
            dependency_tables.extend(
                dependencies
                for target in manifest.get("target", {}).values()
                for table_name in ("dependencies", "dev-dependencies", "build-dependencies")
                if (dependencies := target.get(table_name)) is not None
            )
            declarations = [
                dependencies[dependency_name]
                for dependencies in dependency_tables
                if dependency_name in dependencies
            ]
            return [
                declaration
                if isinstance(declaration, str)
                else declaration["version"]
                for declaration in declarations
            ]

        image_versions = {}
        png_versions = {}
        jpeg_decoder_versions = {}
        for path in workspace_manifests:
            manifest = tomllib.loads(path.read_text(encoding="utf-8"))
            relative_path = str(path.relative_to(REPOSITORY))
            if versions := dependency_versions(manifest, "image"):
                image_versions[relative_path] = versions
            if versions := dependency_versions(manifest, "png"):
                png_versions[relative_path] = versions
            if versions := dependency_versions(manifest, "jpeg-decoder"):
                jpeg_decoder_versions[relative_path] = versions

        self.assertEqual(set(image_versions), expected_image_manifests)
        self.assertTrue(
            all(versions == ["0.25"] for versions in image_versions.values()),
            image_versions,
        )
        self.assertEqual(png_versions, {"crates/clonk-app/Cargo.toml": ["0.18"]})
        self.assertEqual(
            jpeg_decoder_versions,
            {"crates/clonk-resources/Cargo.toml": ["0.3"]},
        )
        resources_manifest = tomllib.loads(
            (REPOSITORY / "crates/clonk-resources/Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        jpeg_decoder_dependency = resources_manifest["dependencies"]["jpeg-decoder"]
        self.assertFalse(jpeg_decoder_dependency["default-features"])
        self.assertNotIn("features", jpeg_decoder_dependency)

        cargo_lock = tomllib.loads(
            (REPOSITORY / "Cargo.lock").read_text(encoding="utf-8")
        )
        for package_name, version_prefix in (
            ("image", "0.25."),
            ("png", "0.18."),
            ("jpeg-decoder", "0.3."),
        ):
            versions = [
                package["version"]
                for package in cargo_lock["package"]
                if package["name"] == package_name
            ]
            with self.subTest(package=package_name):
                self.assertEqual(len(versions), 1, versions)
                self.assertTrue(versions[0].startswith(version_prefix), versions)


if __name__ == "__main__":
    unittest.main()
