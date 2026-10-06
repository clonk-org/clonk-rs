from __future__ import annotations

import json
import importlib.util
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock
import zlib


REPOSITORY = Path(__file__).resolve().parents[2]
SCRIPT = REPOSITORY / "scripts" / "ci-content.py"
SPEC = importlib.util.spec_from_file_location("ci_content", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ColdFetchBudgetTests(unittest.TestCase):
    def test_a_transient_fetch_error_leaves_the_remaining_download_budget_for_retry(self) -> None:
        elapsed = 0.0
        budgets = []

        def download(repository, budget, command):
            nonlocal elapsed
            budgets.append(budget)
            duration = 12 if len(budgets) == 1 else 310
            elapsed += min(duration, budget)
            return 128 if len(budgets) == 1 else (0 if duration <= budget else 124)

        report = {"attempts": []}
        with mock.patch.object(MODULE.time, "monotonic", side_effect=lambda: elapsed), \
                mock.patch.object(MODULE, "reset_owned_content"), \
                mock.patch.object(MODULE, "run_bounded", side_effect=download), \
                mock.patch.object(MODULE, "verify_checkout"), \
                mock.patch.object(MODULE, "verify_object_store"):
            MODULE.materialize(REPOSITORY, "a" * 40, report)

        self.assertEqual(budgets, [480, 468])
        self.assertEqual(elapsed, 322)
        self.assertTrue(report["materialized"])

    def test_an_exhausted_download_deadline_does_not_start_or_reset_another_attempt(self) -> None:
        elapsed = 0.0

        def timeout(repository, budget, command):
            nonlocal elapsed
            elapsed += budget
            return 124

        report = {"attempts": []}
        with mock.patch.object(MODULE.time, "monotonic", side_effect=lambda: elapsed), \
                mock.patch.object(MODULE, "reset_owned_content") as reset, \
                mock.patch.object(MODULE, "run_bounded", side_effect=timeout) as download:
            with self.assertRaisesRegex(MODULE.ContentError, "deadline"):
                MODULE.materialize(REPOSITORY, "a" * 40, report)

        self.assertEqual(elapsed, 480)
        self.assertEqual(download.call_count, 1)
        self.assertEqual(reset.call_count, 1)


class ContentCheckoutTests(unittest.TestCase):
    def setUp(self) -> None:
        self.sandbox = Path(tempfile.mkdtemp(prefix="clonk-ci-content-"))
        self.addCleanup(shutil.rmtree, self.sandbox, onerror=MODULE.remove_readonly)
        self.environment = {
            **os.environ,
            "GIT_ALLOW_PROTOCOL": "file",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GITHUB_STEP_SUMMARY": str(self.sandbox / "summary.md"),
        }
        self.source = self.sandbox / "source"
        self.parent = self.sandbox / "parent"
        for root in (self.source, self.parent):
            root.mkdir()
            self.git(root, "init", "--quiet")
            self.git(root, "config", "user.name", "Content test")
            self.git(root, "config", "user.email", "content-test@example.invalid")
        (self.source / "scenario.c4s").write_text("pinned scenario\n", encoding="utf-8")
        self.git(self.source, "add", "scenario.c4s")
        self.git(self.source, "commit", "--quiet", "-m", "test: pin content")
        self.revision = self.git(self.source, "rev-parse", "HEAD").stdout.strip()
        self.git(self.parent, "submodule", "add", "--quiet", str(self.source), "content")
        self.git(self.parent, "commit", "--quiet", "-m", "test: pin submodule")
        scripts = self.parent / "scripts"
        scripts.mkdir()
        shutil.copy2(REPOSITORY / "scripts" / "run_with_timeout.py", scripts)
        if SCRIPT.exists():
            shutil.copy2(SCRIPT, scripts)
        self.report = self.sandbox / "report.json"

    def git(self, root: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(root), *arguments], check=True,
            capture_output=True, text=True, env=self.environment,
        )

    def run_content(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.parent / "scripts" / "ci-content.py"),
             "--revision", self.revision, "--report", str(self.report), *arguments],
            check=False, capture_output=True, text=True,
            env=self.environment, timeout=15,
        )

    def remove_checkout(self) -> None:
        shutil.rmtree(self.parent / "content", onerror=MODULE.remove_readonly)
        shutil.rmtree(self.parent / ".git" / "modules" / "content", onerror=MODULE.remove_readonly)

    def test_a_missing_cache_materializes_the_exact_clean_gitlink(self) -> None:
        self.remove_checkout()

        completed = self.run_content()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            self.git(self.parent / "content", "rev-parse", "HEAD").stdout.strip(),
            self.revision,
        )
        self.assertEqual(
            self.git(self.parent / "content", "status", "--porcelain", "--untracked-files=all").stdout,
            "",
        )
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertTrue(report["checkout_clean"])
        self.assertTrue(report["materialized"])
        self.assertFalse(report["cache_hit"])
        self.assertEqual(report["content_revision"], self.revision)
        self.assertEqual(report["cache_format"], "v2")

    def test_a_verified_cache_needs_no_fetch_or_checkout(self) -> None:
        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertTrue(report["cache_hit"])
        self.assertTrue(report["cache_verified"])
        self.assertFalse(report["materialized"])
        self.assertEqual(report["attempts"], [])

    def test_a_cold_fetch_downloads_only_the_pinned_revision_not_the_remote_tip(self) -> None:
        (self.source / "unrelated.c4s").write_text("new unpinned scenario\n", encoding="utf-8")
        self.git(self.source, "add", "unrelated.c4s")
        self.git(self.source, "commit", "--quiet", "-m", "test: advance the remote tip")
        remote_tip = self.git(self.source, "rev-parse", "HEAD").stdout.strip()
        self.git(self.parent, "config", "--file", ".gitmodules", "submodule.content.url", self.source.as_uri())
        self.git(self.parent, "add", ".gitmodules")
        self.git(self.parent, "commit", "--quiet", "-m", "test: use a transported content origin")
        self.remove_checkout()

        completed = self.run_content()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual((self.parent / "content" / "scenario.c4s").read_text(), "pinned scenario\n")
        self.assertFalse((self.parent / "content" / "unrelated.c4s").exists())
        unpinned_commit = subprocess.run(
            ["git", "-C", str(self.parent / "content"), "cat-file", "-e", remote_tip],
            check=False, capture_output=True, env={**self.environment, "GIT_NO_LAZY_FETCH": "1"},
        )
        self.assertNotEqual(unpinned_commit.returncode, 0, "cold fetch downloaded the unpinned remote tip")

    def test_an_object_store_only_warm_cache_hydrates_without_fetching_or_replacing_the_store(self) -> None:
        modules = self.parent / ".git" / "modules" / "content"
        marker = modules / "info" / "restored-store-marker"
        marker.write_text("retain verified objects", encoding="utf-8")
        objects = {path.relative_to(modules): path.read_bytes() for path in (modules / "objects").rglob("*") if path.is_file()}
        shutil.rmtree(self.parent / "content")
        # Any fetch would fail: only the restored portable store remains usable.
        self.source.rename(self.sandbox / "offline-source")

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(marker.read_text(encoding="utf-8"), "retain verified objects")
        self.assertEqual({path.relative_to(modules): path.read_bytes() for path in (modules / "objects").rglob("*") if path.is_file()}, objects)
        self.assertEqual(self.git(self.parent / "content", "status", "--porcelain", "--untracked-files=all").stdout, "")
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertTrue(report["cache_store_verified"])
        self.assertTrue(report["cache_verified"])
        self.assertTrue(report["materialized"])
        self.assertFalse(report["repaired_cache"])
        self.assertEqual(report["hydration_source"], "object_store")
        self.assertEqual(report["attempts"], [])
        self.assertEqual(report["object_store_bytes_before"], report["object_store_bytes_after"])
        # actions/checkout also leaves empty gitlink directories on fresh runners.
        shutil.rmtree(self.parent / "content")
        (self.parent / "content").mkdir()
        repeated = self.run_content("--cache-hit", "true")
        self.assertEqual(repeated.returncode, 0, repeated.stderr)
        self.assertEqual(json.loads(self.report.read_text())["attempts"], [])
        self.assertEqual(marker.read_text(encoding="utf-8"), "retain verified objects")

    def test_an_object_store_with_forged_commit_bytes_is_replaced_from_the_exact_origin(self) -> None:
        modules = self.parent / ".git" / "modules" / "content"
        commit = modules / "objects" / self.revision[:2] / self.revision[2:]
        data = zlib.decompress(commit.read_bytes()).split(b"\0", 1)[1] + b"forged cached metadata\n"
        # Local Git clones hardlink objects; detach before corrupting only the cache.
        commit.unlink()
        commit.write_bytes(zlib.compress(f"commit {len(data)}\0".encode() + data))
        shutil.rmtree(self.parent / "content")

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertFalse(report["cache_store_verified"])
        self.assertTrue(report["repaired_cache"])
        self.assertEqual(report["hydration_source"], "origin")
        self.assertIn("fsck", report["cache_store_error"])
        self.assertEqual((self.parent / "content" / "scenario.c4s").read_text(), "pinned scenario\n")

    def test_object_store_hydration_rejects_external_worktree_configuration(self) -> None:
        external = self.sandbox / "private-worktree"
        external.mkdir()
        marker = external / "keep.txt"
        marker.write_text("private input", encoding="utf-8")
        self.git(self.parent / "content", "config", "core.worktree", str(external))
        shutil.rmtree(self.parent / "content")

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertEqual(report["hydration_source"], "origin")
        self.assertIn("external worktree", report["cache_store_error"])
        self.assertEqual(marker.read_text(encoding="utf-8"), "private input")

    def test_object_store_hydration_cannot_depend_on_external_alternate_objects(self) -> None:
        modules = self.parent / ".git" / "modules" / "content"
        alternates = modules / "objects" / "info" / "alternates"
        alternates.write_text(str(self.source / ".git" / "objects") + "\n", encoding="utf-8")
        source_commit = self.source / ".git" / "objects" / self.revision[:2] / self.revision[2:]
        original = source_commit.read_bytes()
        shutil.rmtree(self.parent / "content")

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertEqual(report["hydration_source"], "origin")
        self.assertIn("external objects", report["cache_store_error"])
        self.assertEqual(source_commit.read_bytes(), original)

    def test_cached_store_with_a_missing_pinned_blob_is_refetched_and_never_claims_an_offline_hit(self) -> None:
        modules = self.parent / ".git" / "modules" / "content"
        blob = self.git(self.source, "rev-parse", "HEAD:scenario.c4s").stdout.strip()
        (modules / "objects" / blob[:2] / blob[2:]).unlink()
        self.git(self.parent / "content", "config", "remote.origin.promisor", "true")
        shutil.rmtree(self.parent / "content")

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertFalse(report["cache_verified"])
        self.assertFalse(report["cache_store_verified"])
        self.assertEqual(report["hydration_source"], "origin")
        self.assertEqual(len(report["attempts"]), 1)
        self.assertTrue(report["checkout_clean"])

    def test_cached_store_cannot_redirect_its_common_directory_into_a_private_repository(self) -> None:
        external = self.sandbox / "private-repository"
        self.git(self.sandbox, "clone", "--quiet", str(self.source), str(external))
        private_config = (external / ".git" / "config").read_bytes()
        private_index = (external / ".git" / "index").read_bytes()
        modules = self.parent / ".git" / "modules" / "content"
        (modules / "commondir").write_text(str(external / ".git") + "\n", encoding="utf-8")
        shutil.rmtree(self.parent / "content")

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertIn("root is outside", report["cache_store_error"])
        self.assertEqual(report["hydration_source"], "origin")
        self.assertEqual((external / ".git" / "config").read_bytes(), private_config)
        self.assertEqual((external / ".git" / "index").read_bytes(), private_index)

    def test_verification_rejects_a_checkout_from_another_origin(self) -> None:
        self.git(self.parent / "content", "remote", "set-url", "origin", str(self.sandbox / "wrong"))

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("origin", completed.stderr)
        self.assertFalse(json.loads(self.report.read_text(encoding="utf-8"))["checkout_clean"])

    def test_verification_cannot_accept_an_external_repository_as_content(self) -> None:
        self.git(self.source, "remote", "add", "origin", str(self.source))
        shutil.rmtree(self.parent / "content")
        (self.parent / "content").symlink_to(self.source, target_is_directory=True)

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("real directory", completed.stderr)
        self.assertEqual(self.git(self.source, "rev-parse", "HEAD").stdout.strip(), self.revision)

    def test_verification_rejects_an_external_git_directory_even_for_the_right_revision(self) -> None:
        external = self.sandbox / "external"
        self.git(self.sandbox, "clone", "--quiet", str(self.source), str(external))
        self.git(external, "config", "core.worktree", str(self.parent / "content"))
        (self.parent / "content" / ".git").write_text(
            f"gitdir: {external / '.git'}\n", encoding="utf-8",
        )

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("Git directory", completed.stderr)

    def test_untracked_restored_content_is_repaired_without_touching_other_parent_files(self) -> None:
        (self.parent / "content" / "injected.c4s").write_text("wrong parity input", encoding="utf-8")
        unrelated = self.parent / "unrelated.txt"
        unrelated.write_text("retain me", encoding="utf-8")

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertTrue(report["repaired_cache"])
        self.assertFalse(report["cache_verified"])
        self.assertTrue(report["materialized"])
        self.assertTrue(report["checkout_clean"])
        self.assertIn("untracked", report["cache_error"])
        self.assertFalse((self.parent / "content" / "injected.c4s").exists())
        self.assertEqual(unrelated.read_text(encoding="utf-8"), "retain me")

    def test_ignored_untracked_parity_input_is_still_rejected(self) -> None:
        exclusions = self.parent / ".git" / "modules" / "content" / "info" / "exclude"
        exclusions.write_text("injected.c4s\n", encoding="utf-8")
        (self.parent / "content" / "injected.c4s").write_text("wrong parity input", encoding="utf-8")

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("untracked", completed.stderr)

    def test_a_cold_fetch_reports_measured_hydration_and_store_bytes_without_claiming_network_bytes(self) -> None:
        self.remove_checkout()

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertTrue(report["cache_hit"])
        self.assertFalse(report["cache_verified"])
        self.assertGreater(report["hydration_elapsed_seconds"], 0)
        self.assertEqual(report["object_store_bytes_before"], 0)
        self.assertGreater(report["object_store_bytes_after"], 0)
        self.assertIsNone(report["network_bytes"])
        self.assertGreater(report["attempts"][0]["budget_seconds"], 0)
        self.assertLessEqual(report["attempts"][0]["budget_seconds"], 480)
        self.assertIn("Object-store bytes", (self.sandbox / "summary.md").read_text(encoding="utf-8"))

    def test_verify_only_cannot_fall_back_to_the_parent_repository_when_git_metadata_is_missing(self) -> None:
        (self.parent / "content" / ".git").unlink()

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("root", completed.stderr)

    def test_the_requested_revision_must_match_the_parent_gitlink_before_any_repair(self) -> None:
        marker = self.parent / "content" / "untracked.txt"
        marker.write_text("must remain", encoding="utf-8")

        completed = self.run_content("--revision", "0" * 40)

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("parent HEAD:content", completed.stderr)
        self.assertEqual(marker.read_text(encoding="utf-8"), "must remain")

    def test_a_wrong_restored_head_is_rejected_and_can_be_refetched(self) -> None:
        (self.source / "scenario.c4s").write_text("newer, unrequested scenario\n", encoding="utf-8")
        self.git(self.source, "commit", "--quiet", "-am", "test: advance content")
        self.git(self.parent / "content", "fetch", "--quiet", "origin")
        self.git(self.parent / "content", "checkout", "--quiet", self.git(self.source, "rev-parse", "HEAD").stdout.strip())

        rejected = self.run_content("--verify-only")
        self.assertEqual(rejected.returncode, 1, rejected.stdout)
        self.assertIn("HEAD", rejected.stderr)

        recovered = self.run_content("--cache-hit", "true")
        self.assertEqual(recovered.returncode, 0, recovered.stderr)
        self.assertEqual((self.parent / "content" / "scenario.c4s").read_text(encoding="utf-8"), "pinned scenario\n")

    def test_dirty_tracked_content_and_corrupted_git_index_fail_closed(self) -> None:
        content = self.parent / "content"
        (content / "scenario.c4s").write_text("modified parity input", encoding="utf-8")
        rejected = self.run_content("--verify-only")
        self.assertEqual(rejected.returncode, 1, rejected.stdout)
        self.assertIn("modified", rejected.stderr)

        self.git(content, "checkout", "--", "scenario.c4s")
        index = self.parent / ".git" / "modules" / "content" / "index"
        index.write_bytes(b"corrupted index")
        rejected = self.run_content("--verify-only")
        self.assertEqual(rejected.returncode, 1, rejected.stdout)
        self.assertIn("git status", rejected.stderr)
        self.assertEqual(index.read_bytes(), b"corrupted index")

        recovered = self.run_content()
        self.assertEqual(recovered.returncode, 0, recovered.stderr)
        self.assertTrue(json.loads(self.report.read_text(encoding="utf-8"))["checkout_clean"])

    def test_repair_does_not_follow_a_restored_external_gitdir_pointer(self) -> None:
        external = self.sandbox / "external"
        self.git(self.sandbox, "clone", "--quiet", str(self.source), str(external))
        self.git(external, "config", "core.worktree", str(self.parent / "content"))
        external_config = (external / ".git" / "config").read_bytes()
        (self.parent / "content" / ".git").write_text(
            f"gitdir: {external / '.git'}\n", encoding="utf-8",
        )

        completed = self.run_content()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual((external / ".git" / "config").read_bytes(), external_config)
        self.assertTrue((external / ".git" / "objects").is_dir())

    def test_an_external_parent_modules_link_is_never_followed_or_removed(self) -> None:
        external = self.sandbox / "private"
        external.mkdir()
        marker = external / "keep.txt"
        marker.write_text("private Git store", encoding="utf-8")
        modules = self.parent / ".git" / "modules"
        modules.rename(self.parent / ".git" / "saved-modules")
        modules.symlink_to(external, target_is_directory=True)

        completed = self.run_content()

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("refusing external", completed.stderr)
        self.assertEqual(marker.read_text(encoding="utf-8"), "private Git store")
        self.assertTrue(modules.is_symlink())

    def test_an_empty_restore_is_a_recoverable_cache_miss(self) -> None:
        self.remove_checkout()
        (self.parent / "content").mkdir()
        (self.parent / ".git" / "modules" / "content").mkdir()

        completed = self.run_content("--cache-hit", "true")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertFalse(json.loads(self.report.read_text(encoding="utf-8"))["cache_verified"])

    def test_a_missing_origin_fails_with_both_bounded_attempts_and_no_clean_result(self) -> None:
        self.git(self.parent, "config", "--file", ".gitmodules", "submodule.content.url", str(self.sandbox / "missing"))
        self.git(self.parent, "add", ".gitmodules")
        self.git(self.parent, "commit", "--quiet", "-m", "test: missing content source")
        self.remove_checkout()

        completed = self.run_content()

        self.assertEqual(completed.returncode, 1, completed.stdout)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertEqual(len(report["attempts"]), 2)
        self.assertTrue(all(0 < entry["budget_seconds"] <= 480 for entry in report["attempts"]))
        self.assertLess(report["attempts"][1]["budget_seconds"], report["attempts"][0]["budget_seconds"])
        self.assertTrue(all(entry["returncode"] != 0 for entry in report["attempts"]))
        self.assertFalse(report["checkout_clean"])
        self.assertFalse(report["materialized"])
        self.assertIn("Could not read from remote repository", completed.stderr)

    def test_cached_index_flags_cannot_hide_modified_parity_bytes(self) -> None:
        content = self.parent / "content"
        self.git(content, "update-index", "--assume-unchanged", "scenario.c4s")
        scenario = content / "scenario.c4s"
        original = scenario.stat()
        scenario.write_text("broken scenario\n", encoding="utf-8")
        os.utime(scenario, ns=(original.st_atime_ns, original.st_mtime_ns))
        self.assertEqual(self.git(content, "status", "--porcelain").stdout, "")

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("tracked", completed.stderr)
        self.assertEqual(scenario.read_text(encoding="utf-8"), "broken scenario\n")

    def test_a_parent_blob_named_content_is_rejected_before_any_checkout_repair(self) -> None:
        marker = self.parent / "content" / "retain.txt"
        marker.write_text("must remain", encoding="utf-8")
        blob = self.git(self.parent, "hash-object", "-w", str(self.parent / "content" / "scenario.c4s")).stdout.strip()
        self.git(self.parent, "update-index", "--cacheinfo", f"100644,{blob},content")
        self.git(self.parent, "commit", "--quiet", "-m", "test: replace the gitlink")

        completed = self.run_content("--revision", blob)

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertIn("gitlink", completed.stderr)
        self.assertEqual(marker.read_text(encoding="utf-8"), "must remain")

    def test_repair_handles_windows_readonly_cached_files(self) -> None:
        scenario = self.parent / "content" / "scenario.c4s"
        scenario.chmod(stat.S_IRUSR)
        unlink = os.unlink

        def windows_unlink(path, *, dir_fd=None):
            if Path(path).name == "scenario.c4s" and not os.stat(path, dir_fd=dir_fd).st_mode & stat.S_IWUSR:
                raise PermissionError("cached file has the Windows readonly attribute")
            return unlink(path, dir_fd=dir_fd)

        with mock.patch.object(MODULE.os, "unlink", side_effect=windows_unlink):
            MODULE.reset_owned_content(self.parent)

        self.assertFalse((self.parent / "content").exists())
        self.assertFalse((self.parent / ".git" / "modules" / "content").exists())

    def test_fresh_index_verification_preserves_git_autocrlf_semantics(self) -> None:
        content = self.parent / "content"
        self.git(content, "config", "core.autocrlf", "true")
        (self.parent / ".git" / "modules" / "content" / "index").unlink()
        (content / "scenario.c4s").unlink()
        self.git(content, "read-tree", "HEAD")
        self.git(content, "checkout-index", "--all")
        self.assertEqual((content / "scenario.c4s").read_bytes(), b"pinned scenario\r\n")
        self.assertEqual(self.git(content, "status", "--porcelain").stdout, "")

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(json.loads(self.report.read_text(encoding="utf-8"))["checkout_clean"])

    def test_broken_git_object_metadata_recovers_through_the_exact_origin(self) -> None:
        (self.parent / ".git" / "modules" / "content" / "HEAD").write_text("broken-ref\n", encoding="utf-8")

        rejected = self.run_content("--verify-only")
        self.assertEqual(rejected.returncode, 1, rejected.stdout)

        completed = self.run_content("--cache-hit", "true")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertEqual(report["content_revision"], self.revision)
        self.assertTrue(report["repaired_cache"])

    def test_a_cache_permission_error_is_reported_without_a_clean_result(self) -> None:
        (self.parent / "content" / "untracked.txt").write_text("stale cache", encoding="utf-8")

        with (
            mock.patch.object(MODULE, "REPOSITORY", self.parent),
            mock.patch.object(MODULE, "reset_owned_content", side_effect=PermissionError("cache permission denied")),
        ):
            status = MODULE.main(["--revision", self.revision, "--report", str(self.report)])

        self.assertEqual(status, 1)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertFalse(report["checkout_clean"])
        self.assertIn("permission denied", report["error"])

    def test_verification_does_not_rewrite_the_restored_git_index(self) -> None:
        scenario = self.parent / "content" / "scenario.c4s"
        old = scenario.stat()
        os.utime(scenario, ns=(old.st_atime_ns, old.st_mtime_ns + 1_000_000_000))
        index = self.parent / ".git" / "modules" / "content" / "index"
        previous = index.read_bytes()

        completed = self.run_content("--verify-only")

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(index.read_bytes(), previous)


class ContentPathTests(unittest.TestCase):
    def test_windows_reparse_points_are_links_even_when_pathlib_does_not_identify_junctions(self) -> None:
        path = mock.Mock()
        path.is_symlink.return_value = False
        path.is_junction.return_value = False
        path.lstat.return_value = mock.Mock(st_file_attributes=0x400)

        self.assertTrue(MODULE.is_link(path))


class BoundedContentFetchTests(unittest.TestCase):
    def test_fetch_timeout_terminates_descendants_before_they_change_the_checkout(self) -> None:
        with tempfile.TemporaryDirectory(prefix="clonk-ci-content-timeout-") as directory:
            marker = Path(directory) / "orphan.txt"
            child = f"import time; from pathlib import Path; time.sleep(0.35); Path({str(marker)!r}).write_text('orphan')"
            parent = f"import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', {child!r}]); time.sleep(30)"

            with mock.patch.object(MODULE.sys, "stderr", sys.__stderr__):
                status = MODULE.run_bounded(REPOSITORY, 0.1, [sys.executable, "-c", parent])

            self.assertEqual(status, 124)
            time.sleep(0.4)
            self.assertFalse(marker.exists())

    def test_windows_fetch_is_gated_until_its_kill_on_close_job_is_assigned(self) -> None:
        with tempfile.TemporaryDirectory(prefix="clonk-ci-content-gate-") as directory:
            marker = Path(directory) / "started.txt"
            events = []

            class Job:
                def assign(self, process):
                    time.sleep(0.03)
                    self.assert_not_started()
                    events.append("assigned")

                def assert_not_started(self):
                    if marker.exists():
                        raise AssertionError("Git command started before job assignment")

                def close(self):
                    events.append("closed")

            command = [sys.executable, "-c", f"from pathlib import Path; Path({str(marker)!r}).write_text('started')"]
            with (
                mock.patch.object(MODULE, "WINDOWS", True, create=True),
                mock.patch.object(MODULE, "WindowsJob", return_value=Job(), create=True),
                mock.patch.object(MODULE.sys, "stderr", sys.__stderr__),
            ):
                status = MODULE.run_bounded(REPOSITORY, 1, command)

            self.assertEqual(status, 0)
            self.assertEqual(events, ["assigned", "closed"])
            self.assertTrue(marker.exists())

    def test_windows_job_creation_failure_cannot_start_an_unbounded_fetch(self) -> None:
        with (
            mock.patch.object(MODULE, "WINDOWS", True),
            mock.patch.object(MODULE, "WindowsJob", side_effect=OSError("job unavailable")),
            mock.patch.object(MODULE.subprocess, "Popen") as start,
            self.assertRaisesRegex(MODULE.ContentError, "cannot bound"),
        ):
            MODULE.run_bounded(REPOSITORY, 1, [sys.executable, "-c", "pass"])

        start.assert_not_called()

    def test_windows_gate_remains_a_waiting_parent_until_the_timeout_helper_exits(self) -> None:
        with tempfile.TemporaryDirectory(prefix="clonk-ci-content-window-parent-") as directory:
            marker = Path(directory) / "helper-parent.txt"
            observations = {}

            class Job:
                def assign(self, process):
                    observations["assigned_pid"] = process.pid

                def close(self):
                    observations["completed_before_close"] = marker.exists()

            command = [sys.executable, "-c",
                       f"import os, sys, time; from pathlib import Path; time.sleep(0.05); "
                       f"Path({str(marker)!r}).write_text(str(os.getppid())); sys.exit(37)"]
            with (
                mock.patch.object(MODULE, "WINDOWS", True),
                mock.patch.object(MODULE, "WindowsJob", return_value=Job()),
                mock.patch.object(MODULE.sys, "stderr", sys.__stderr__),
            ):
                status = MODULE.run_bounded(REPOSITORY, 2, command)

            self.assertEqual(status, 37)
            self.assertTrue(observations["completed_before_close"])
            # The assigned gate must wait for a separate helper process rather
            # than replace itself with execv (which changes PID on Windows).
            self.assertNotEqual(int(marker.read_text()), observations["assigned_pid"])


@unittest.skipUnless(os.name == "nt", "requires the native Windows Job Object API")
class NativeWindowsBoundsTests(unittest.TestCase):
    def test_delayed_success_finishes_before_the_job_is_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="clonk-ci-content-native-success-") as directory:
            marker = Path(directory) / "completed.txt"
            command = [sys.executable, "-c",
                       f"import time; from pathlib import Path; time.sleep(0.25); "
                       f"Path({str(marker)!r}).write_text('completed')"]
            with mock.patch.object(MODULE.sys, "stderr", sys.__stderr__):
                status = MODULE.run_bounded(REPOSITORY, 5, command)

            self.assertEqual(status, 0)
            self.assertEqual(marker.read_text(), "completed")

    def test_delayed_failure_preserves_the_actual_process_exit_code(self) -> None:
        with tempfile.TemporaryDirectory(prefix="clonk-ci-content-native-failure-") as directory:
            marker = Path(directory) / "completed.txt"
            command = [sys.executable, "-c",
                       f"import sys, time; from pathlib import Path; time.sleep(0.25); "
                       f"Path({str(marker)!r}).write_text('failed'); sys.exit(37)"]
            with mock.patch.object(MODULE.sys, "stderr", sys.__stderr__):
                status = MODULE.run_bounded(REPOSITORY, 5, command)

            self.assertEqual(status, 37)
            self.assertEqual(marker.read_text(), "failed")


if __name__ == "__main__":
    unittest.main()
