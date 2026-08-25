"""The release workflow's content hand-off guards, exercised as real shell.

`content.zip` is built and published by another repository, and this one records
its digest and size in the update manifest. With no manifest signature that
digest is the whole integrity story for 1.2 GB, so the publishing job downloads
the archive and checks it before anything is tagged or published.

These tests run the workflow's own `run:` bodies — extracted from
`release.yml`, not retyped — against stubbed `git`, `gh` and `curl`. A guard
deleted or weakened fails here rather than in a release nobody can re-run.
"""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from _repo import REPOSITORY

WORKFLOW = REPOSITORY / ".github" / "workflows" / "release.yml"

PIN = "d34d385591134ce6c262b8c9ed53faaa6229cc6b"
TAG = f"content-{PIN}"
CONTENT_REPOSITORY = "clonk-org/clonk-rs-content"
BASE = f"https://github.com/{CONTENT_REPOSITORY}/releases/download/{TAG}"


def step_script(name):
    """The `run:` block of the named step, verbatim.

    Parsed rather than depending on a YAML library: the system Python that runs
    these tests is not guaranteed to have one, and the shape being matched is
    fixed by the workflow's own formatting.
    """
    lines = WORKFLOW.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(f"{WORKFLOW.name} has no step named {name!r}") from None

    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line.startswith("      - "):
            break
        if line == "        run: |":
            body = []
            for candidate in lines[index + 1 :]:
                if candidate.strip() and not candidate.startswith(" " * 10):
                    break
                body.append(candidate[10:])
            return "\n".join(body)
    raise AssertionError(f"step {name!r} has no `run: |` block")


@unittest.skipUnless(shutil.which("bash"), "needs bash")
@unittest.skipUnless(shutil.which("jq"), "needs jq, as the ubuntu runner has")
class ReleaseContentHandoffTests(unittest.TestCase):
    """Runs the publish job's two content steps in a sandbox."""

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.bin = self.root / "bin"
        self.store = self.root / "store"
        self.runner_temp = self.root / "tmp"
        for directory in (self.bin, self.store, self.runner_temp):
            directory.mkdir(parents=True)
        self._write_stubs()

    def _stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def _write_stubs(self):
        # The submodule is not checked out in the publish job; the step reads
        # the gitlink out of the commit's tree.
        self._stub("git", f'[[ "$1" == "rev-parse" ]] && {{ echo {PIN}; exit 0; }}\nexit 1\n')
        # `gh api` exits non-zero for every failure alike; only the message
        # distinguishes them, which is the behaviour under test.
        self._stub(
            "gh",
            "case \"$GH_MODE\" in\n"
            '  ok)        cat "$STORE/release.json"; exit 0 ;;\n'
            '  missing)   echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;\n'
            '  ratelimit) echo "gh: API rate limit exceeded (HTTP 403)" >&2; exit 1 ;;\n'
            '  network)   echo "error connecting to api.github.com" >&2; exit 1 ;;\n'
            '  noasset)   cat "$STORE/release-no-asset.json"; exit 0 ;;\n'
            "esac\n",
        )
        # Serves only the two URLs the workflow asks for.
        self._stub(
            "curl",
            'url=${@: -1}\nout=""\nprev=""\n'
            'for argument in "$@"; do [[ "$prev" == "--output" ]] && out=$argument; prev=$argument; done\n'
            "case \"$url\" in\n"
            '  *content.sha256) cat "$STORE/content.sha256"; exit 0 ;;\n'
            '  *content.zip)    cp "$STORE/content.zip" "$out"; exit 0 ;;\n'
            "esac\nexit 22\n",
        )
        # The workflow runs on ubuntu; macOS ships `shasum` instead.
        if not shutil.which("sha256sum"):
            self._stub("sha256sum", 'shasum -a 256 "$@"\n')

    def seed_release(self, payload=b"content bytes", digest=None, size=None):
        """Publishes a fake content release, optionally an inconsistent one."""
        archive = self.store / "content.zip"
        archive.write_bytes(payload)
        import hashlib

        real = hashlib.sha256(payload).hexdigest()
        (self.store / "content.sha256").write_text(
            f"{digest or real}  content.zip\n", encoding="utf-8"
        )
        (self.store / "release.json").write_text(
            '{"assets":[{"name":"content.zip","size":%d}]}\n'
            % (len(payload) if size is None else size),
            encoding="utf-8",
        )
        (self.store / "release-no-asset.json").write_text(
            '{"assets":[{"name":"release-notes.md","size":42}]}\n', encoding="utf-8"
        )
        return real

    def run_step(self, name, gh_mode="ok", **extra):
        github_output = self.root / "step-output"
        github_output.touch()
        environment = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "RUNNER_TEMP": str(self.runner_temp),
            "GITHUB_OUTPUT": str(github_output),
            "STORE": str(self.store),
            "GH_MODE": gh_mode,
            "GH_TOKEN": "stub",
            "CONTENT_REPOSITORY": CONTENT_REPOSITORY,
            **extra,
        }
        # The shell GitHub Actions gives a `run:` block.
        completed = subprocess.run(
            ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", step_script(name)],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
        )
        outputs = dict(
            line.split("=", 1)
            for line in github_output.read_text(encoding="utf-8").splitlines()
            if "=" in line
        )
        return completed, outputs

    def resolve(self, **kwargs):
        return self.run_step("Resolve the published content release", **kwargs)

    def verify(self, outputs):
        return self.run_step(
            "Verify the published content archive",
            CONTENT_BASE=outputs["base"],
            CONTENT_SHA256=outputs["sha256"],
            CONTENT_SIZE=outputs["size"],
        )

    # --- resolving -----------------------------------------------------------

    def test_a_healthy_content_release_resolves_to_its_digest_size_and_base(self):
        digest = self.seed_release()
        completed, outputs = self.resolve()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(outputs["commit"], PIN)
        self.assertEqual(outputs["sha256"], digest)
        self.assertEqual(outputs["size"], str(len(b"content bytes")))
        self.assertEqual(outputs["base"], BASE)

    def test_a_missing_content_release_says_which_commit_to_push(self):
        self.seed_release()
        completed, _ = self.resolve(gh_mode="missing")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn(f"has no {TAG} release", completed.stderr)
        self.assertIn("push that content commit", completed.stderr)

    def test_a_failed_lookup_is_never_reported_as_a_missing_release(self):
        # `|| true` used to funnel a bad token, a rate limit, a network error
        # and the content repository being made private into the one message
        # that says "push that content commit" — sending whoever reads the log
        # to push a commit that is already pushed.
        for mode in ("ratelimit", "network"):
            with self.subTest(mode=mode):
                self.seed_release()
                completed, _ = self.resolve(gh_mode=mode)

                self.assertNotEqual(completed.returncode, 0)
                self.assertIn(f"looking up {TAG} in {CONTENT_REPOSITORY} failed", completed.stderr)
                self.assertNotIn("push that content commit", completed.stderr)

    def test_a_release_without_the_archive_is_named_as_such(self):
        self.seed_release()
        completed, _ = self.resolve(gh_mode="noasset")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("publishes no content.zip", completed.stderr)

    def test_an_uppercase_sidecar_digest_is_refused(self):
        # The manifest records this verbatim and a client compares it as text,
        # so it is lowercase hex or nothing — matching `is_sha256_hex` in xtask.
        self.seed_release(digest="A" * 64)
        completed, _ = self.resolve()

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("not a lowercase digest", completed.stderr)

    # --- verifying -----------------------------------------------------------

    def test_an_archive_matching_its_sidecar_verifies_and_is_deleted(self):
        self.seed_release()
        _, outputs = self.resolve()
        completed, _ = self.verify(outputs)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("verified content.zip", completed.stdout)
        # 1.2 GB on a runner that still has to hold every platform's assets.
        self.assertFalse((self.runner_temp / "content.zip").exists())

    def test_a_sidecar_digest_the_bytes_do_not_have_fails_the_release(self):
        # The defect this whole step exists for: the digest was copied from the
        # producer's sidecar into the manifest without anything comparing it to
        # the bytes, so every client failed its own check after downloading
        # 1.2 GB — or installed whatever the sidecar happened to claim.
        self.seed_release(digest="b" * 64)
        _, outputs = self.resolve()
        completed, _ = self.verify(outputs)

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("refusing to publish a manifest pointing at unverified bytes", completed.stderr)

    def test_a_size_the_bytes_do_not_have_fails_the_release(self):
        self.seed_release(size=999_999)
        _, outputs = self.resolve()
        completed, _ = self.verify(outputs)

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("the release API says 999999", completed.stderr)

    def test_the_archive_never_lands_where_the_release_globs_would_publish_it(self):
        # `assets/` and `components/` are uploaded wholesale, so a stray 1.2 GB
        # download there would be published as one of this repository's own
        # assets — the duplicate upload that referencing the content release
        # removed in the first place.
        self.seed_release()
        _, outputs = self.resolve()
        script = step_script("Verify the published content archive")

        self.assertIn('archive="${RUNNER_TEMP}/content.zip"', script)
        completed, _ = self.verify(outputs)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(list(self.root.glob("**/content.zip")), [self.store / "content.zip"])

    def test_verification_runs_before_the_tag_and_the_release(self):
        # After either one, a bad content release would strand `vX.Y.Z` on a
        # commit whose release can never be completed, or publish it outright.
        text = WORKFLOW.read_text(encoding="utf-8")
        order = [
            text.index("      - name: Verify the published content archive"),
            text.index("      - name: Generate the update manifest"),
            text.index("      - name: Tag the built commit"),
            text.index("      - name: Publish the release"),
        ]
        self.assertEqual(order, sorted(order))


if __name__ == "__main__":
    unittest.main()
