"""The shipped third-party license corpus has to follow the release graph.

Regenerating needs the crate sources in the local Cargo registry, which a CI
row that has not fetched them does not have. These checks are deliberately
hermetic instead: they compare the generated index against `Cargo.lock`, which
is the file that actually changes when a dependency is added, removed or
bumped, so a stale corpus is caught wherever they run.

The last class covers the workflow that keeps the corpus current on Renovate's
own branches, because a bot cannot answer the freshness check by hand.
"""

import importlib.util
import pathlib
import re
import tomllib
import unittest
from unittest import mock

from _repo import REPOSITORY

CORPUS = REPOSITORY / "crates/clonk-frontend/src/dependency_licenses.txt"
INDEX = REPOSITORY / "crates/clonk-frontend/src/dependency_licenses.index"
GENERATOR = REPOSITORY / "scripts/generate_dependency_licenses.py"
REGENERATION_WORKFLOW = REPOSITORY / ".github/workflows/dependency-licenses.yml"

SPEC = importlib.util.spec_from_file_location("generate_dependency_licenses", GENERATOR)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

WORKSPACE_ROOT = "/workspace"


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


def declaration(name, *, optional=False, kind=None, rename=None, target=None):
    """One entry of a package's manifest dependency table, as cargo reports it."""
    return {
        "name": name,
        "optional": optional,
        "kind": kind,
        "rename": rename,
        "target": target,
    }


def package(name, *, dependencies=(), features=None, workspace=False):
    root = WORKSPACE_ROOT if workspace else "/registry"
    return {
        "id": f"{name} 1.0.0",
        "name": name,
        "version": "1.0.0",
        "license": "MIT",
        "manifest_path": f"{root}/{name}/Cargo.toml",
        "dependencies": list(dependencies),
        "features": dict(features or {}),
    }


def node(name, *, deps=(), features=()):
    return {
        "id": f"{name} 1.0.0",
        "features": list(features),
        "deps": list(deps),
    }


def edge(name, *, kind=None, target=None):
    return {
        "name": name.replace("-", "_"),
        "pkg": f"{name} 1.0.0",
        "dep_kinds": [{"kind": kind, "target": target}],
    }


def graph(packages, nodes, *, entry_dependencies=(), entry_deps=()):
    """Metadata for a workspace whose shipped binaries all reach `entry_deps`.

    `release_graph` refuses to run unless every shipped binary is a workspace
    member, so the four of them are always present; only the first carries the
    edges a test cares about.
    """
    binaries = sorted(MODULE.SHIPPED_BINARIES)
    members = [
        package(
            name,
            workspace=True,
            dependencies=entry_dependencies if name == binaries[0] else (),
        )
        for name in binaries
    ]
    member_nodes = [
        node(name, deps=entry_deps if name == binaries[0] else ()) for name in binaries
    ]
    return {
        "workspace_root": WORKSPACE_ROOT,
        "packages": members + list(packages),
        "resolve": {"nodes": member_nodes + list(nodes)},
    }


def attributed(metadata):
    return [entry["name"] for entry in MODULE.release_graph(metadata)]


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

    def test_an_optional_dependency_no_binary_compiles_is_absent(self):
        # flate2's default `runtime_detection` mentions zlib-rs only weakly, so
        # the lock carries it and no build compiles it: `cargo tree --workspace
        # --target all -i zlib-rs` prints nothing. The dialog says the listed
        # packages are linked, so a merely locked package must not appear.
        attributed = {name for name, _, _ in index_entries()}
        self.assertIn("flate2", attributed)
        self.assertIn("miniz_oxide", attributed)
        self.assertFalse(
            "zlib-rs" in attributed,
            "zlib-rs is attributed but no shipped binary compiles it; "
            "regenerate with scripts/generate_dependency_licenses.py",
        )

    def test_workspace_test_features_do_not_enter_the_shipped_corpus(self):
        # cargo metadata resolves features as the union over every workspace
        # member, so clonk-engine's test-only `test-graph` feature can make
        # proptest and its subtree look compiled into a release binary. The
        # corpus follows only the four shipped roots, not workspace tests.
        test_only_packages = {
            ("ppv-lite86", "0.2.21"),
            ("proptest", "1.11.0"),
            ("rand", "0.9.5"),
            ("rand_chacha", "0.9.0"),
            ("rand_core", "0.9.5"),
            ("rand_xorshift", "0.4.0"),
            ("termcolor", "1.4.1"),
            ("unarray", "0.1.4"),
        }
        attributed = {(name, version) for name, version, _ in index_entries()}
        for package in sorted(test_only_packages):
            with self.subTest(package=package):
                self.assertNotIn(package, attributed)

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


class CargoTreeTests(unittest.TestCase):
    def test_tree_selection_keeps_all_targets_and_non_dev_edges(self):
        output = """\
clonk-app v1.0.0 (/workspace)
├── normal-dependency v1.0.0
└── build-dependency v2.0.0 (*)
"""
        with mock.patch.object(
            MODULE.subprocess,
            "run",
            return_value=mock.Mock(stdout=output),
        ) as run:
            self.assertEqual(
                MODULE.cargo_tree_packages("clonk-app"),
                {
                    ("clonk-app", "1.0.0"),
                    ("normal-dependency", "1.0.0"),
                    ("build-dependency", "2.0.0"),
                },
            )

        command = run.call_args.args[0]
        self.assertIn("--target", command)
        self.assertEqual(command[command.index("--target") + 1], "all")
        self.assertIn("--edges", command)
        self.assertEqual(command[command.index("--edges") + 1], "normal,build")
        self.assertIn("--locked", command)


class ReleaseGraphTests(unittest.TestCase):
    """What the walk counts as compiled in, edge by edge.

    `Cargo.lock` and `resolve.nodes` record an optional dependency as soon as a
    feature mentions it, even weakly, so the resolve graph is a superset of
    what any binary compiles. The About dialog says "Clonk Rust links the
    packages listed below", so the walk has to read activated features too.
    """

    def test_a_weakly_referenced_optional_dependency_is_not_attributed(self):
        # flate2 1.1.10's default `runtime_detection` carries `zlib-rs?/std`.
        # A weak reference enables nothing, so the backend the binary compiles
        # is miniz_oxide and zlib-rs is never built.
        metadata = graph(
            packages=[
                package(
                    "flate2",
                    dependencies=[
                        declaration("miniz_oxide", optional=True),
                        declaration("zlib-rs", optional=True),
                    ],
                    features={
                        "default": ["rust_backend", "runtime_detection"],
                        "rust_backend": ["miniz_oxide", "any_impl"],
                        "runtime_detection": ["zlib-rs?/std"],
                        "miniz_oxide": ["any_impl", "dep:miniz_oxide"],
                        "zlib-rs": ["dep:zlib-rs"],
                        "any_impl": [],
                    },
                ),
                package("miniz_oxide"),
                package("zlib-rs"),
            ],
            nodes=[
                node(
                    "flate2",
                    deps=[edge("miniz_oxide"), edge("zlib-rs")],
                    features=[
                        "any_impl",
                        "default",
                        "miniz_oxide",
                        "runtime_detection",
                        "rust_backend",
                    ],
                ),
                node("miniz_oxide"),
                node("zlib-rs"),
            ],
            entry_dependencies=[declaration("flate2")],
            entry_deps=[edge("flate2")],
        )
        self.assertEqual(attributed(metadata), ["flate2", "miniz_oxide"])

    def test_an_optional_dependency_a_dep_reference_enables_is_attributed(self):
        # Cargo materializes the implicit feature an optional dependency gets
        # as `dep:<name>`, so this is the shape metadata reports for most of
        # the graph.
        metadata = graph(
            packages=[
                package(
                    "host",
                    dependencies=[declaration("serde", optional=True)],
                    features={"serde": ["dep:serde"]},
                ),
                package("serde"),
            ],
            nodes=[
                node("host", deps=[edge("serde")], features=["serde"]),
                node("serde"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host", "serde"])

    def test_an_enabled_feature_named_after_the_dependency_is_enough(self):
        # An implicit feature carries the name of the dependency it enables.
        # Cargo spells it out in the feature table, but reading the name alone
        # keeps attribution erring towards listing a package if it ever stops.
        metadata = graph(
            packages=[
                package(
                    "host",
                    dependencies=[declaration("serde", optional=True)],
                ),
                package("serde"),
            ],
            nodes=[
                node("host", deps=[edge("serde")], features=["serde"]),
                node("serde"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host", "serde"])

    def test_a_non_weak_feature_reference_activates_the_optional_dependency(self):
        # `serde/std` enables serde itself; only `serde?/std` does not.
        metadata = graph(
            packages=[
                package(
                    "host",
                    dependencies=[declaration("serde", optional=True)],
                    features={"default": ["serde/std"]},
                ),
                package("serde"),
            ],
            nodes=[
                node("host", deps=[edge("serde")], features=["default"]),
                node("serde"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host", "serde"])

    def test_an_unactivated_optional_dependency_drops_what_it_alone_reaches(self):
        # The whole subtree behind an uncompiled edge is uncompiled too.
        metadata = graph(
            packages=[
                package(
                    "host",
                    dependencies=[declaration("backend", optional=True)],
                    features={"backend": ["dep:backend"]},
                ),
                package("backend", dependencies=[declaration("sys")]),
                package("sys"),
            ],
            nodes=[
                node("host", deps=[edge("backend")], features=["default"]),
                node("backend", deps=[edge("sys")]),
                node("sys"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host"])

    def test_a_renamed_optional_dependency_is_read_under_its_alias(self):
        # Features name a renamed dependency by its alias, never by the
        # package it resolves to.
        metadata = graph(
            packages=[
                package(
                    "host",
                    dependencies=[
                        declaration("rustls", optional=True, rename="tls"),
                    ],
                    features={"secure": ["dep:tls"]},
                ),
                package("rustls"),
            ],
            nodes=[
                node("host", deps=[edge("rustls")], features=["secure"]),
                node("rustls"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host", "rustls"])

    def test_a_dependency_optional_only_for_dev_stays_attributed(self):
        # The optional declaration a `dev` edge carries says nothing about the
        # ordinary edge that compiles the package in.
        metadata = graph(
            packages=[
                package(
                    "host",
                    dependencies=[
                        declaration("shared"),
                        declaration("shared", optional=True, kind="dev"),
                    ],
                ),
                package("shared"),
            ],
            nodes=[
                node("host", deps=[edge("shared")]),
                node("shared"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host", "shared"])

    def test_a_dev_only_edge_reaches_no_binary(self):
        metadata = graph(
            packages=[
                package("host", dependencies=[declaration("harness", kind="dev")]),
                package("harness"),
            ],
            nodes=[
                node("host", deps=[edge("harness", kind="dev")]),
                node("harness"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host"])

    def test_a_foreign_platform_dependency_stays_attributed(self):
        # The corpus ships on every platform, so a Windows-only package must
        # be attributed by a Linux run. Activated features are read; the
        # target the edge carries deliberately is not.
        metadata = graph(
            packages=[
                package(
                    "host",
                    dependencies=[
                        declaration("windows-sys", target="cfg(windows)"),
                        declaration(
                            "core-foundation",
                            optional=True,
                            target='cfg(target_os = "macos")',
                        ),
                    ],
                    features={"apple": ["dep:core-foundation"]},
                ),
                package("windows-sys"),
                package("core-foundation"),
            ],
            nodes=[
                node(
                    "host",
                    deps=[
                        edge("windows-sys", target="cfg(windows)"),
                        edge("core-foundation", target='cfg(target_os = "macos")'),
                    ],
                    features=["apple"],
                ),
                node("windows-sys"),
                node("core-foundation"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(
            attributed(metadata), ["core-foundation", "host", "windows-sys"]
        )

    def test_an_edge_with_no_manifest_declaration_is_attributed(self):
        # Attribution errs towards listing a package rather than dropping one
        # the binary does link, so an edge the manifest tables cannot explain
        # is kept.
        metadata = graph(
            packages=[
                package("host"),
                package("mystery"),
            ],
            nodes=[
                node("host", deps=[edge("mystery")]),
                node("mystery"),
            ],
            entry_dependencies=[declaration("host")],
            entry_deps=[edge("host")],
        )
        self.assertEqual(attributed(metadata), ["host", "mystery"])


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
