import fcntl
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import tomllib
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
SCRIPT = REPOSITORY / "scripts" / "seed_worktree_target.py"
SPEC = importlib.util.spec_from_file_location("seed_worktree_target", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

TOOLCHAIN = tomllib.loads(
    (REPOSITORY / "rust-toolchain.toml").read_text(encoding="utf-8")
)["toolchain"]["channel"]


def scratch_env():
    # Keep the caller's target directory out, and a hook's GIT_DIR, which
    # would aim git at the repository under test instead of the scratch one.
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("CARGO_", "GIT_")) or key == "CARGO_HOME"
    }
    env["RUSTUP_TOOLCHAIN"] = TOOLCHAIN
    return env


def tempdir_can_reflink():
    with tempfile.TemporaryDirectory() as scratch:
        original = Path(scratch) / "original"
        original.write_bytes(b"extent")
        clone = Path(scratch) / "clone"
        return subprocess.run(
            ["cp", "--reflink=always", str(original), str(clone)],
            check=False,
            capture_output=True,
        ).returncode == 0


REFLINK = shutil.which("cp") is not None and tempdir_can_reflink()


def write_probe_package_source(root, value, age_seconds=0):
    # An edit made before another checkout's build is older than that build.
    source = root / "src" / "main.rs"
    source.write_text(f'fn main() {{ println!("{value}"); }}\n', encoding="utf-8")
    modified = source.stat().st_mtime - age_seconds
    os.utime(source, (modified, modified))


def write_probe_package(root, value, age_seconds=0):
    (root / "src").mkdir(parents=True)
    (root / "Cargo.toml").write_text(
        '[package]\nname = "probe"\nversion = "0.1.0"\nedition = "2021"\n',
        encoding="utf-8",
    )
    write_probe_package_source(root, value, age_seconds)


def cargo_run(root):
    return subprocess.run(
        ["cargo", "run", "--quiet"],
        cwd=root,
        env=scratch_env(),
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def plain_copy(source, destination):
    shutil.copytree(source, destination, symlinks=True)


def make_fingerprints(target, profile, names):
    for name in names:
        (target / profile / ".fingerprint" / name).mkdir(parents=True)


def fingerprints(target, profile):
    return sorted(path.name for path in (target / profile / ".fingerprint").iterdir())


class DropLocalFingerprintsTests(unittest.TestCase):
    def test_drops_only_fingerprints_of_path_packages_in_every_profile(self):
        with tempfile.TemporaryDirectory() as scratch:
            target = Path(scratch)
            for profile in ("debug", "release"):
                make_fingerprints(
                    target,
                    profile,
                    (
                        "clonk-app-0123456789abcdef",
                        "clonk-app-fedcba9876543210",
                        "xtask-00000000000000aa",
                        "xtask-helper-00000000000000bb",
                        "serde-00000000000000cc",
                    ),
                )

            dropped = MODULE.drop_local_fingerprints(target, {"clonk-app", "xtask"})

            self.assertEqual(dropped, 6)
            for profile in ("debug", "release"):
                self.assertEqual(
                    fingerprints(target, profile),
                    ["serde-00000000000000cc", "xtask-helper-00000000000000bb"],
                )

    def test_drops_fingerprints_under_an_explicit_target_triple(self):
        with tempfile.TemporaryDirectory() as scratch:
            target = Path(scratch)
            profile = "x86_64-pc-windows-gnu/debug"
            make_fingerprints(
                target,
                profile,
                ("clonk-app-0123456789abcdef", "serde-00000000000000cc"),
            )

            dropped = MODULE.drop_local_fingerprints(target, {"clonk-app"})

            self.assertEqual(dropped, 1)
            self.assertEqual(fingerprints(target, profile), ["serde-00000000000000cc"])


@unittest.skipUnless(shutil.which("cargo"), "needs cargo")
class SeedTests(unittest.TestCase):
    def test_seeded_worktree_rebuilds_a_path_package_its_seed_built_later(self):
        # Cargo judges a path package fresh when no source file is newer than
        # its last build, and the metadata hash is relative to the workspace
        # root, so a copied build of another checkout matches this one's units.
        with tempfile.TemporaryDirectory() as scratch:
            main = Path(scratch) / "main"
            worktree = Path(scratch) / "worktree"
            write_probe_package(main, "main")
            write_probe_package(worktree, "worktree", age_seconds=60)
            self.assertEqual(cargo_run(main), "main\n")

            MODULE.seed(main / "target", worktree / "target", {"probe"}, plain_copy)

            self.assertEqual(cargo_run(worktree), "worktree\n")


class SeedGuardTests(unittest.TestCase):
    def test_refuses_to_merge_into_an_existing_target(self):
        with tempfile.TemporaryDirectory() as scratch:
            source = Path(scratch) / "main" / "target"
            destination = Path(scratch) / "worktree" / "target"
            make_fingerprints(source, "debug", ("probe-0123456789abcdef",))
            make_fingerprints(destination, "debug", ("probe-fedcba9876543210",))

            with self.assertRaisesRegex(MODULE.SeedRefused, "already exists"):
                MODULE.seed(source, destination, {"probe"}, plain_copy)

            self.assertEqual(fingerprints(destination, "debug"), ["probe-fedcba9876543210"])

    def test_a_failed_copy_leaves_no_partial_target_behind(self):
        def copy_then_fail(source, destination):
            plain_copy(source, destination)
            raise subprocess.CalledProcessError(1, ["cp", "--reflink=always"])

        with tempfile.TemporaryDirectory() as scratch:
            source = Path(scratch) / "main" / "target"
            worktree = Path(scratch) / "worktree"
            make_fingerprints(source, "debug", ("probe-0123456789abcdef",))
            worktree.mkdir()

            with self.assertRaisesRegex(MODULE.SeedRefused, "could not copy"):
                MODULE.seed(source, worktree / "target", {"probe"}, copy_then_fail)

            self.assertEqual(list(worktree.iterdir()), [])

    @unittest.skipUnless(os.name == "posix", "Cargo locks build directories with flock")
    def test_waits_for_a_build_holding_the_source_profile_lock(self):
        with tempfile.TemporaryDirectory() as scratch:
            source = Path(scratch) / "main" / "target"
            destination = Path(scratch) / "worktree" / "target"
            make_fingerprints(source, "debug", ("serde-00000000000000cc",))
            destination.parent.mkdir()
            build_lock = source / "debug" / ".cargo-lock"
            build_lock.touch()

            with build_lock.open("rb") as running_build:
                fcntl.flock(running_build, fcntl.LOCK_EX)
                seeding = threading.Thread(
                    target=MODULE.seed,
                    args=(source, destination, {"probe"}, plain_copy),
                )
                seeding.start()
                seeding.join(timeout=0.2)
                self.assertTrue(seeding.is_alive())
                self.assertFalse(destination.exists())

            seeding.join(timeout=5)
            self.assertEqual(fingerprints(destination, "debug"), ["serde-00000000000000cc"])


def git(root, *arguments):
    subprocess.run(
        ["git", "-c", "user.name=seed", "-c", "user.email=seed@example.invalid", *arguments],
        cwd=root,
        env=scratch_env(),
        check=True,
        capture_output=True,
    )


def make_main_checkout(root):
    write_probe_package(root, "main")
    (root / ".gitignore").write_text("target/\n", encoding="utf-8")
    cargo_run(root)
    git(root, "init", "--quiet", "--initial-branch=main")
    git(root, "add", ".")
    git(root, "commit", "--quiet", "--message=probe")


def run_script(root):
    return subprocess.run(
        [sys.executable, str(SCRIPT)],
        cwd=root,
        env=scratch_env(),
        check=False,
        capture_output=True,
        text=True,
    )


@unittest.skipUnless(shutil.which("cargo") and shutil.which("git"), "needs cargo and git")
class CommandLineTests(unittest.TestCase):
    def test_refuses_to_seed_the_main_checkout_from_itself(self):
        with tempfile.TemporaryDirectory() as scratch:
            main = Path(scratch) / "main"
            make_main_checkout(main)

            completed = run_script(main)

            self.assertEqual(completed.returncode, 1, completed.stderr)
            self.assertIn("main checkout", completed.stderr)
            self.assertEqual(cargo_run(main), "main\n")

    @unittest.skipUnless(REFLINK, "temporary directory cannot reflink (btrfs and XFS can)")
    def test_seeds_a_worktree_that_then_builds_its_own_source(self):
        with tempfile.TemporaryDirectory() as scratch:
            main = Path(scratch) / "main"
            worktree = Path(scratch) / "worktree"
            make_main_checkout(main)
            git(main, "worktree", "add", "--quiet", str(worktree))
            write_probe_package_source(worktree, "worktree", age_seconds=60)

            completed = run_script(worktree)

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertIn("dropped 1 fingerprints", completed.stdout)
            self.assertEqual(cargo_run(worktree), "worktree\n")
            self.assertEqual(cargo_run(main), "main\n")

    @unittest.skipIf(REFLINK, "temporary directory can reflink")
    def test_refuses_rather_than_duplicate_the_cache_without_reflink(self):
        with tempfile.TemporaryDirectory() as scratch:
            main = Path(scratch) / "main"
            worktree = Path(scratch) / "worktree"
            make_main_checkout(main)
            git(main, "worktree", "add", "--quiet", str(worktree))

            completed = run_script(worktree)

            self.assertEqual(completed.returncode, 1, completed.stderr)
            self.assertIn("could not copy", completed.stderr)
            self.assertFalse((worktree / "target").exists())


if __name__ == "__main__":
    unittest.main()
