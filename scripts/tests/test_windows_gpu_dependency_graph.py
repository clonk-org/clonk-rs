import pathlib
import tomllib
import unittest


REPOSITORY = pathlib.Path(__file__).resolve().parents[2]


class WindowsGpuDependencyGraphTests(unittest.TestCase):
    def test_wgpu_hal_and_gpu_allocator_share_windows_bindings(self):
        cargo_lock = tomllib.loads(
            (REPOSITORY / "Cargo.lock").read_text(encoding="utf-8")
        )

        def package(package_name):
            matches = [
                package
                for package in cargo_lock["package"]
                if package["name"] == package_name
            ]
            self.assertEqual(
                len(matches),
                1,
                f"expected exactly one locked {package_name} package",
            )
            return matches[0]

        def windows_dependency(package_name):
            return next(
                dependency
                for dependency in package(package_name)["dependencies"]
                if dependency == "windows" or dependency.startswith("windows ")
            )

        self.assertEqual(
            windows_dependency("wgpu-hal"),
            windows_dependency("gpu-allocator"),
            "wgpu-hal passes Direct3D types into gpu-allocator, so both crates "
            "must resolve the same windows crate version",
        )


if __name__ == "__main__":
    unittest.main()
