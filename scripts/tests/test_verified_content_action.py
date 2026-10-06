"""Pinned game data is a verified input, never a cached qualification result."""

import unittest

from _repo import REPOSITORY
from test_workflow_runtime_inventory import step_blocks, workflow_jobs


class VerifiedContentActionTests(unittest.TestCase):
    def test_every_content_consumer_uses_the_same_bounded_verifier(self):
        workflows = REPOSITORY / ".github/workflows"
        consumers = {
            "landing.yml": ("pull-request-quality", "linux", "release-evidence"),
            "rust.yml": ("content-landing-cache", "linux-landing-cache",
                         "qualification-reuse", "windows-release-tools"),
            "exact-sha-qualification.yml": (
                "qualification-context", "coverage-fragments", "coverage-cache-warm",
                "developer-feedback", "recording-host-oracles", "recording-host-cache-warm",
            ),
            "release-build.yml": ("package",),
            "device-loss-qualification.yml": ("recover",),
            "release.yml": ("publish",),
        }
        for name, roles in consumers.items():
            with self.subTest(workflow=name):
                workflow = workflows / name
                text = workflow.read_text()
                self.assertNotIn("submodules: recursive", text)
                self.assertNotIn("git submodule update", text)
                self.assertNotIn("submodule deinit", text)
                self.assertNotIn("git -C content fetch", text)
                self.assertNotIn("git -C content reset", text)
                self.assertNotIn(".git/modules/content", text)
                jobs = workflow_jobs(workflow)
                for role in roles:
                    with self.subTest(workflow=name, job=role):
                        self.assertIn(role, jobs)
                        verifiers = [step for step in step_blocks(jobs[role])
                                     if "uses: ./.github/actions/verified-content" in step]
                        self.assertTrue(verifiers, f"{name}:{role} has no pinned-content verifier")
                        for step in verifiers:
                            self.assertNotIn("continue-on-error: true", step)
                            self.assertIn("timeout-minutes: 10", step)
                        if name == "rust.yml" and role == "windows-release-tools":
                            self.assertTrue(any("        if:" not in step
                                                or "if: inputs.software_presentation" in step
                                                for step in verifiers))
                        elif name == "release.yml":
                            self.assertTrue(any("steps.resolve.outputs.release == 'true'" in step
                                                for step in verifiers))
                        else:
                            self.assertTrue(any("        if:" not in step for step in verifiers),
                                            f"{name}:{role} can skip its required input verifier")

    def test_receipt_and_workspace_ledger_consumers_always_verify_actual_content(self):
        for name in ("landing.yml", "rust.yml", "exact-sha-qualification.yml", "release.yml"):
            for role, job in workflow_jobs(REPOSITORY / ".github/workflows" / name).items():
                if not any(marker in job for marker in (
                    "scripts/release-qualification-evidence.py", "scripts/ci-workspace-cache.py prepare",
                )):
                    continue
                with self.subTest(workflow=name, job=role):
                    self.assertIn("uses: ./.github/actions/verified-content", job)
                    self.assertLess(job.index("uses: ./.github/actions/verified-content"),
                                    min(job.index(marker) for marker in (
                                        "scripts/release-qualification-evidence.py",
                                        "scripts/ci-workspace-cache.py prepare",
                                    ) if marker in job))

    def test_optional_portable_cache_is_always_verified_and_only_main_publishes(self):
        action = (REPOSITORY / ".github/actions/verified-content/action.yml").read_text()
        self.assertIn("git rev-parse HEAD:content", action)
        self.assertIn("clonk-content-git-v2-", action)
        self.assertNotIn("${{ runner.os }}", action)
        self.assertIn("enableCrossOsArchive: true", action)
        self.assertIn("continue-on-error: true", action)
        self.assertIn("python3 scripts/ci-content.py", action)
        self.assertIn('--revision "$CONTENT_REVISION"', action)
        self.assertIn('--cache-hit "$CACHE_HIT"', action)
        self.assertIn("timeout --kill-after=10s 600s scripts/install-apt-packages.sh", action)
        self.assertNotIn("timeout-minutes:", action)
        self.assertIn("github.ref == 'refs/heads/main'", action)
        self.assertIn("inputs.publish == 'true'", action)
        self.assertIn("uses: ./.github/actions/verify-cache-handoff", action)
        verification = action.split("name: Materialize and verify pinned content", 1)[1].split("    - name:", 1)[0]
        self.assertNotIn("if:", verification)

    def test_existing_exact_linux_object_cache_remains_a_verified_restore_source(self):
        action = (REPOSITORY / ".github/actions/verified-content/action.yml").read_text()
        restore = action.split("name: Restore optional pinned content objects", 1)[1].split("    - name:", 1)[0]
        expected = ("clonk-content-git-v1-Linux-${{ hashFiles('.gitmodules') }}-"
                    "${{ steps.identity.outputs.revision }}")
        self.assertIn("restore-keys:", restore)
        self.assertIn(expected, restore)
        self.assertNotIn("${{ runner.os }}", restore)
        publication = action.split("name: Publish verified pinned content objects", 1)[1].split("    - name:", 1)[0]
        self.assertIn("key: clonk-content-git-v2-", publication)
        self.assertNotIn("clonk-content-git-v1-", publication)


if __name__ == "__main__":
    unittest.main()
