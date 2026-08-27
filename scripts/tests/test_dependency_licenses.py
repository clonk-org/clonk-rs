"""The shipped third-party license corpus has to follow the release graph.

Regenerating needs the crate sources in the local Cargo registry, which a CI
row that has not fetched them does not have. These checks are deliberately
hermetic instead: they compare the generated index against `Cargo.lock`, which
is the file that actually changes when a dependency is added, removed or
bumped, so a stale corpus is caught wherever they run.
"""

import pathlib
import tomllib
import unittest

from _repo import REPOSITORY

CORPUS = REPOSITORY / "crates/clonk-frontend/src/dependency_licenses.txt"
INDEX = REPOSITORY / "crates/clonk-frontend/src/dependency_licenses.index"
GENERATOR = REPOSITORY / "scripts/generate_dependency_licenses.py"


def index_entries() -> list[tuple[str, str, str]]:
    entries = []
    for line in INDEX.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        name, version, expression = line.split("\t")
        entries.append((name, version, expression))
    return entries


def locked_packages() -> dict[str, set[str]]:
    lock = tomllib.loads((REPOSITORY / "Cargo.lock").read_text(encoding="utf-8"))
    packages: dict[str, set[str]] = {}
    for package in lock["package"]:
        packages.setdefault(package["name"], set()).add(package["version"])
    return packages


class DependencyLicenseCorpusTests(unittest.TestCase):
    def test_every_attributed_package_is_locked_at_that_version(self):
        locked = locked_packages()
        for name, version, _ in index_entries():
            with self.subTest(package=name):
                self.assertIn(
                    name,
                    locked,
                    f"{name} is attributed but no longer in Cargo.lock; "
                    "regenerate with scripts/generate_dependency_licenses.py",
                )
                self.assertIn(
                    version,
                    locked[name],
                    f"{name} is attributed at {version}, which Cargo.lock does "
                    "not carry; regenerate the corpus",
                )

    def test_the_index_is_sorted_and_free_of_duplicates(self):
        entries = [(name, version) for name, version, _ in index_entries()]
        self.assertEqual(entries, sorted(entries), "the index must be generated sorted")
        self.assertEqual(len(entries), len(set(entries)), "one entry per package")

    def test_the_corpus_covers_exactly_the_indexed_packages(self):
        corpus = CORPUS.read_text(encoding="utf-8")
        declared = next(
            int(line.removeprefix("Packages: "))
            for line in corpus.splitlines()
            if line.startswith("Packages: ")
        )
        self.assertEqual(declared, len(index_entries()))

    def test_identical_texts_share_one_section(self):
        # Without grouping, one Apache-2.0 text would ship hundreds of times.
        # This is what keeps the corpus small enough to compile in.
        corpus = CORPUS.read_text(encoding="utf-8")
        sections = sum(
            1 for line in corpus.splitlines() if line.startswith("Applies to: ")
        )
        self.assertGreater(sections, 0)
        self.assertLess(sections, len(index_entries()))

    def test_a_package_without_a_distributed_text_keeps_its_spdx_expression(self):
        # Attribution must never invent a text a package does not publish, so
        # those packages appear with their declared expression instead.
        corpus = CORPUS.read_text(encoding="utf-8")
        self.assertIn("distribute no license file", corpus)
        self.assertIn("rather than a text they do not publish", corpus)

    def test_the_corpus_is_reachable_from_the_dialog(self):
        dialog = (
            REPOSITORY / "crates/clonk-frontend/src/startup_about_dlg.rs"
        ).read_text(encoding="utf-8")
        self.assertIn('include_str!("dependency_licenses.txt")', dialog)

    def test_the_generator_documents_how_to_refresh_it(self):
        generator = GENERATOR.read_text(encoding="utf-8")
        self.assertIn("python3 scripts/generate_dependency_licenses.py", generator)
        self.assertIn("--check", generator)


if __name__ == "__main__":
    unittest.main()
