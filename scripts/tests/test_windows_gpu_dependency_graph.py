import tomllib
import unittest

from _repo import REPOSITORY


class WindowsGpuDependencyGraphTests(unittest.TestCase):
    def test_direct_windows_bindings_match_the_dx12_graph(self):
        cargo_lock = tomllib.loads(
            (REPOSITORY / "Cargo.lock").read_text(encoding="utf-8")
        )

        def windows_dependency(package_name):
            package = next(
                package
                for package in cargo_lock["package"]
                if package["name"] == package_name
            )
            return next(
                dependency
                for dependency in package["dependencies"]
                if dependency == "windows" or dependency.startswith("windows ")
            )

        dx12_windows = windows_dependency("wgpu-hal")
        for package_name in ("clonk-app", "clonk-platform"):
            with self.subTest(package=package_name):
                self.assertEqual(
                    windows_dependency(package_name),
                    dx12_windows,
                    "direct Windows bindings must resolve the same crate "
                    "identity as the DX12 graph",
                )

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
