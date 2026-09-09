"""Networking can compile and test without the simulation implementation."""

import tomllib
import unittest

from _repo import REPOSITORY


class ProtocolDependencyContractTests(unittest.TestCase):
    def test_network_dependency_graph_does_not_reach_the_engine(self):
        pending = [REPOSITORY / "crates" / "clonk-network"]
        visited = set()
        while pending:
            directory = pending.pop().resolve()
            if directory in visited:
                continue
            visited.add(directory)
            manifest = tomllib.loads((directory / "Cargo.toml").read_text())
            self.assertNotEqual(manifest["package"]["name"], "clonk-engine")
            sections = [manifest, *manifest.get("target", {}).values()]
            for section in sections:
                for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
                    for dependency in section.get(kind, {}).values():
                        if isinstance(dependency, dict) and "path" in dependency:
                            pending.append(directory / dependency["path"])


if __name__ == "__main__":
    unittest.main()
