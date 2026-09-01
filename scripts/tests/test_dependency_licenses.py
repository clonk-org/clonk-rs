"""The shipped third-party license corpus has to follow the release graph.

Regenerating needs the crate sources in the local Cargo registry, which a CI
row that has not fetched them does not have. These checks are deliberately
hermetic instead: they compare the generated index against `Cargo.lock`, which
is the file that actually changes when a dependency is added, removed or
bumped, so a stale corpus is caught wherever they run.

The last class covers the workflow that keeps the corpus current on Renovate's
own branches, because a bot cannot answer the freshness check by hand.
"""

import pathlib
import re
import tomllib
import unittest

from _repo import REPOSITORY

CORPUS = REPOSITORY / "crates/clonk-frontend/src/dependency_licenses.txt"
INDEX = REPOSITORY / "crates/clonk-frontend/src/dependency_licenses.index"
GENERATOR = REPOSITORY / "scripts/generate_dependency_licenses.py"
REGENERATION_WORKFLOW = REPOSITORY / ".github/workflows/dependency-licenses.yml"


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


class RenovateRegenerationWorkflowTests(unittest.TestCase):
    """Renovate moves Cargo.lock, so something has to move the corpus with it.

    The Mend-hosted app runs no `postUpgradeTasks`, so it cannot regenerate the
    corpus itself, and every cargo pull request it opens would otherwise sit red
    on the freshness check above until a human pushed the output by hand.
    """

    def workflow(self) -> str:
        return REGENERATION_WORKFLOW.read_text(encoding="utf-8")

    def test_it_regenerates_only_on_renovate_branches_of_this_repository(self):
        # A fork chooses its own head branch name, so the `renovate/` prefix on
        # its own would hand a write-scoped token to untrusted code.
        workflow = self.workflow()

        self.assertIn(
            "github.event.pull_request.head.repo.full_name == github.repository",
            workflow,
        )
        self.assertIn(
            "github.event.pull_request.user.login == 'renovate[bot]'", workflow
        )
        self.assertIn("startsWith(github.head_ref, 'renovate/')", workflow)

    def test_it_pushes_with_an_app_token_rather_than_the_actions_token(self):
        # A `GITHUB_TOKEN` push retriggers nothing, so it would advance the head
        # to a commit carrying no checks and leave the pull request permanently
        # blocked -- strictly worse than the red it replaces.
        workflow = self.workflow()

        self.assertIn("actions/create-github-app-token@", workflow)
        self.assertIn("client-id: ${{ vars.RELEASE_APP_CLIENT_ID }}", workflow)
        self.assertIn("private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}", workflow)
        self.assertIn("permission-contents: write", workflow)
        self.assertNotIn("secrets.GITHUB_TOKEN", workflow)

    def test_it_fetches_the_crate_sources_the_generator_reads(self):
        # The freshness check is hermetic precisely because it has no registry;
        # the row that regenerates needs one, and needs no build to get it.
        workflow = self.workflow()

        self.assertIn("cargo fetch --locked", workflow)
        self.assertIn("python3 scripts/generate_dependency_licenses.py", workflow)
        self.assertIn("- 'Cargo.lock'", workflow)
        self.assertIn("- '**/Cargo.toml'", workflow)

    def test_it_commits_only_the_generated_corpus(self):
        workflow = self.workflow()

        for path in (CORPUS, INDEX):
            self.assertIn(str(path.relative_to(REPOSITORY)), workflow)
        self.assertNotIn("git add -A", workflow)
        self.assertNotIn("git add .", workflow)

    def test_its_commit_subject_is_a_conventional_commit(self):
        # The queue squashes, and `squash_merge_commit_message` is
        # COMMIT_MESSAGES, so this subject reaches the body of a commit on main.
        workflow = self.workflow()

        subject = re.search(r'git commit -m "([^"]+)"', workflow)
        self.assertIsNotNone(subject, "the workflow makes no commit")
        self.assertRegex(
            subject.group(1),
            r"^(build|chore|ci|docs|feat|fix|perf|refactor|style|test)!?: .+",
        )

    def test_a_renovate_force_push_supersedes_the_run_instead_of_failing_it(self):
        # Renovate force-pushes the branch whenever it rebases, which on this
        # repository is often. The run for the new head regenerates anyway, so
        # losing the lease is convergence, not a failure to report.
        workflow = self.workflow()

        self.assertIn("--force-with-lease=", workflow)
        self.assertIn("synchronize", workflow)


if __name__ == "__main__":
    unittest.main()
