"""Guards for retained failed-shard reports and route traces."""

import os
import re
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

from _repo import REPOSITORY


WORKFLOWS = REPOSITORY / ".github" / "workflows"
LANDING = WORKFLOWS / "landing.yml"
QUALIFICATION = WORKFLOWS / "exact-sha-qualification.yml"
RELEASE = WORKFLOWS / "release.yml"
RETAIN_JUNIT = REPOSITORY / "scripts" / "retain-nextest-junit.sh"


def job_block(workflow: Path, name: str) -> str:
    source = workflow.read_text(encoding="utf-8")
    start = source.index(f"\n  {name}:\n") + 1
    following = re.search(r"(?m)^  [A-Za-z0-9_-]+:$", source[start + 1 :])
    end = start + 1 + following.start() if following else len(source)
    return source[start:end]


class CiFailureDiagnosticsTests(unittest.TestCase):
    def test_shard_wrapper_preserves_failure_before_successful_nextest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "junit.xml"
            retained = root / "retained"
            fake_cargo = root / "fake-cargo"
            fake_cargo.write_text(
                """#!/usr/bin/env bash
set -euo pipefail
case "$1" in
    fail) printf '<failure/>\\n' > "$LC_NEXTEST_JUNIT_SOURCE"; exit 1 ;;
    stale-fail) exit 1 ;;
    pass) printf '<success/>\\n' > "$LC_NEXTEST_JUNIT_SOURCE" ;;
    *) exit 2 ;;
esac
""",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            environment = os.environ.copy()
            environment.update(
                {
                    "LC_REAL_CARGO": str(fake_cargo),
                    "LC_NEXTEST_JUNIT_SOURCE": str(source),
                    "LC_NEXTEST_JUNIT_DIR": str(retained),
                    "LC_HELPER": str(RETAIN_JUNIT),
                }
            )
            completed = subprocess.run(
                [
                    "bash",
                    "-euo",
                    "pipefail",
                    "-c",
                    """cargo() {
    local status=0
    rm -f "${LC_NEXTEST_JUNIT_SOURCE:-target/nextest/default/junit.xml}"
    "$LC_REAL_CARGO" "$@" || status=$?
    bash "$LC_HELPER" || true
    return "$status"
}
export -f cargo
failed=0
cargo fail || failed=1
cargo stale-fail || failed=1
cargo pass || failed=1
exit "$failed"
""",
                ],
                cwd=REPOSITORY,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 1, completed.stderr)
            self.assertEqual(
                {path.read_text(encoding="utf-8") for path in retained.glob("junit-*.xml")},
                {"<failure/>\n", "<success/>\n"},
            )

    def test_default_nextest_profile_writes_failure_junit_output(self):
        config = tomllib.loads(
            (REPOSITORY / ".config" / "nextest.toml").read_text(encoding="utf-8")
        )
        junit = config["profile"]["default"]["junit"]
        self.assertEqual(junit["path"], "junit.xml")
        self.assertEqual(junit["report-name"], "nextest-run")
        self.assertFalse(junit["store-success-output"])
        self.assertTrue(junit["store-failure-output"])

    def test_each_linux_shard_has_identity_and_always_upload(self):
        linux = job_block(LANDING, "linux")
        matrix = linux[linux.index("        include:\n") : linux.index("    steps:\n")]
        shard_ids = re.findall(r"(?m)^            artifact: ([a-z0-9-]+)$", matrix)
        self.assertEqual(len(shard_ids), 18)
        self.assertEqual(len(set(shard_ids)), len(shard_ids))

        self.assertIn("- name: Prepare shard diagnostics", linux)
        self.assertIn("shutil.rmtree(diagnostic_dir)", linux)
        self.assertIn("- name: Reset stale JUnit report", linux)
        self.assertIn('Path("target/nextest/default/junit.xml").unlink(missing_ok=True)', linux)
        self.assertIn('"run_id": int(os.environ["GITHUB_RUN_ID"])', linux)
        self.assertIn('"run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"])', linux)
        self.assertIn('"source_sha": os.environ["SOURCE_SHA"]', linux)
        self.assertIn('"junit": "target/nextest/default/junit.xml"', linux)
        self.assertIn('"retained_junit": "junit/*.xml"', linux)
        upload = linux[linux.index("- name: Upload shard test diagnostics") :]
        self.assertIn("if: always()", upload)
        self.assertIn(
            "rust-test-diagnostics-${{ github.run_id }}-${{ github.run_attempt }}-"
            "${{ matrix.artifact }}-${{ github.sha }}",
            upload,
        )
        self.assertIn("${{ runner.temp }}/test-diagnostics", upload)
        self.assertIn("target/nextest/default/junit.xml", upload)
        self.assertIn("if-no-files-found: warn", upload)
        self.assertIn('export LC_NEXTEST_JUNIT_DIR="$DIAGNOSTIC_DIR/junit"', linux)
        self.assertIn("export -f cargo", linux)
        self.assertIn(
            'rm -f "${LC_NEXTEST_JUNIT_SOURCE:-target/nextest/default/junit.xml}"',
            linux,
        )
        self.assertIn("bash scripts/retain-nextest-junit.sh || true", linux)

    def test_exact_sha_shards_upload_diagnostics_outside_coverage_handoff(self):
        collectors = job_block(QUALIFICATION, "coverage-fragments")
        self.assertIn("- name: Prepare shard diagnostics", collectors)
        self.assertIn("shutil.rmtree(diagnostic_dir)", collectors)
        self.assertIn("- name: Reset stale JUnit report", collectors)
        self.assertIn('Path("target/nextest/default/junit.xml").unlink(missing_ok=True)', collectors)
        self.assertIn('"source_sha": os.environ["SOURCE_SHA"]', collectors)
        self.assertIn('"retained_junit": "junit/*.xml"', collectors)
        self.assertIn("LC_CAPTURE_VIRTUAL_PLAYER: '1'", collectors)
        self.assertIn(
            "LC_TEST_ARTIFACT_DIR: ${{ runner.temp }}/test-diagnostics/replays",
            collectors,
        )
        upload = collectors[collectors.index("- name: Upload shard test diagnostics") :]
        self.assertIn("if: always()", upload)
        self.assertIn(
            "rust-test-diagnostics-${{ github.run_id }}-${{ github.run_attempt }}-"
            "${{ matrix.artifact }}-${{ inputs.source-sha }}",
            upload,
        )
        self.assertIn("${{ runner.temp }}/test-diagnostics", upload)
        self.assertIn("target/nextest/default/junit.xml", upload)
        self.assertLess(
            upload.index("Upload shard test diagnostics"),
            upload.index("Upload coverage fragment"),
        )
        self.assertIn('export LC_NEXTEST_JUNIT_DIR="$DIAGNOSTIC_DIR/junit"', collectors)
        self.assertIn("export -f cargo", collectors)
        self.assertIn(
            'rm -f "${LC_NEXTEST_JUNIT_SOURCE:-target/nextest/default/junit.xml}"',
            collectors,
        )
        self.assertIn("bash scripts/retain-nextest-junit.sh || true", collectors)

    def test_release_inventory_excludes_only_run_scoped_test_diagnostics(self):
        publish = job_block(RELEASE, "publish")
        resolver = publish[publish.index("- name: Resolve exact-SHA release artifacts") :]
        self.assertIn(
            'diagnostic_prefix="rust-test-diagnostics-${run_id}-"', resolver
        )
        self.assertIn(
            "--arg diagnostic_prefix \"$diagnostic_prefix\"", resolver
        )
        self.assertIn("startswith($diagnostic_prefix)", resolver)


if __name__ == "__main__":
    unittest.main()
