import pathlib
import unittest

from _repo import REPOSITORY, dependency_declarations, manifest


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
        workspace_manifests = [
            pathlib.Path(member) / "Cargo.toml"
            for member in manifest("Cargo.toml")["workspace"]["members"]
        ]
        self.assertIn(
            pathlib.Path("third_party/pixels/Cargo.toml"), workspace_manifests
        )

        def dependency_versions(cargo_manifest, dependency_name):
            return [
                declaration
                if isinstance(declaration, str)
                else declaration["version"]
                for declaration in dependency_declarations(
                    cargo_manifest, dependency_name
                )
            ]

        image_versions = {}
        png_versions = {}
        jpeg_decoder_versions = {}
        for relative_path in workspace_manifests:
            cargo_manifest = manifest(relative_path)
            relative_path = relative_path.as_posix()
            if versions := dependency_versions(cargo_manifest, "image"):
                image_versions[relative_path] = versions
            if versions := dependency_versions(cargo_manifest, "png"):
                png_versions[relative_path] = versions
            if versions := dependency_versions(cargo_manifest, "jpeg-decoder"):
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
        resources_manifest = manifest("crates/clonk-resources/Cargo.toml")
        jpeg_decoder_dependency = resources_manifest["dependencies"]["jpeg-decoder"]
        self.assertFalse(jpeg_decoder_dependency["default-features"])
        self.assertNotIn("features", jpeg_decoder_dependency)

        cargo_lock = manifest("Cargo.lock")
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
