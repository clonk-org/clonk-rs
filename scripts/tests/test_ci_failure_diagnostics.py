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
RETAIN_CARGO = REPOSITORY / "scripts" / "retain-nextest-cargo.sh"
PARITY = REPOSITORY / "xtask" / "src" / "parity.rs"


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
    fail) printf '<failure-one/>\\n' > "$LC_NEXTEST_JUNIT_SOURCE"; exit 23 ;;
    stale-fail) exit 37 ;;
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
                    "STATUS_FILE": str(root / "statuses"),
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
    rm -f "$LC_NEXTEST_JUNIT_SOURCE"
    "$LC_REAL_CARGO" "$@" || status=$?
    bash "$LC_HELPER" || true
    return "$status"
}
export -f cargo
export LC_REAL_CARGO LC_NEXTEST_JUNIT_SOURCE LC_NEXTEST_JUNIT_DIR LC_HELPER STATUS_FILE
bash -euo pipefail -c '
    first=0
    second=0
    cargo fail || first=$?
    cargo stale-fail || second=$?
    cargo pass
    printf "%s %s\\n" "$first" "$second" > "$STATUS_FILE"
'
""",
                ],
                cwd=REPOSITORY,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual((root / "statuses").read_text(encoding="utf-8"), "23 37\n")
            reports = sorted(retained.glob("junit-*.xml"))
            self.assertEqual(len(reports), 2)
            self.assertEqual(
                {path.read_text(encoding="utf-8") for path in reports},
                {"<failure-one/>\n", "<success/>\n"},
            )

    def test_junit_helper_follows_cargo_target_dir_when_source_is_unset(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "instrumented-target"
            source = target / "nextest" / "default" / "junit.xml"
            source.parent.mkdir(parents=True)
            source.write_text("<coverage-failure/>\n", encoding="utf-8")
            retained = root / "retained"
            environment = os.environ.copy()
            environment.pop("LC_NEXTEST_JUNIT_SOURCE", None)
            environment.update(
                {
                    "CARGO_TARGET_DIR": str(target),
                    "LC_NEXTEST_JUNIT_DIR": str(retained),
                }
            )
            completed = subprocess.run(
                ["bash", str(RETAIN_JUNIT)],
                cwd=REPOSITORY,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            repeated = subprocess.run(
                ["bash", str(RETAIN_JUNIT)],
                cwd=REPOSITORY,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(repeated.returncode, 0, repeated.stderr)
            reports = list(retained.glob("junit-*.xml"))
            self.assertEqual(len(reports), 2)
            self.assertEqual(
                {path.read_text(encoding="utf-8") for path in reports},
                {"<coverage-failure/>\n"},
            )
            self.assertEqual(len({path.name for path in reports}), 2)

    def test_native_cargo_wrapper_retains_failure_and_preserves_status(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "junit.xml"
            retained = root / "retained"
            fake_cargo = root / "fake-cargo"
            fake_cargo.write_text(
                """#!/usr/bin/env bash
set -euo pipefail
case "$2" in
    fail) printf '<native-failure/>\\n' > "$LC_NEXTEST_JUNIT_SOURCE"; exit 29 ;;
    stale-fail) exit 37 ;;
    pass) printf '<native-success/>\\n' > "$LC_NEXTEST_JUNIT_SOURCE" ;;
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
                    "LC_NEXTEST_JUNIT_HELPER": str(RETAIN_JUNIT),
                    "LC_CARGO_WRAPPER": str(RETAIN_CARGO),
                    "STATUS_FILE": str(root / "statuses"),
                }
            )
            completed = subprocess.run(
                [
                    "bash",
                    "-euo",
                    "pipefail",
                    "-c",
                    """first=0
second=0
"$LC_CARGO_WRAPPER" nextest fail || first=$?
"$LC_CARGO_WRAPPER" nextest stale-fail || second=$?
"$LC_CARGO_WRAPPER" nextest pass
printf "%s %s\\n" "$first" "$second" > "$STATUS_FILE"
""",
                ],
                cwd=REPOSITORY,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual((root / "statuses").read_text(encoding="utf-8"), "29 37\n")
            reports = sorted(retained.glob("junit-*.xml"))
            self.assertEqual(len(reports), 2)
            self.assertEqual(
                {path.read_text(encoding="utf-8") for path in reports},
                {"<native-failure/>\n", "<native-success/>\n"},
            )

    def test_native_cargo_wrapper_finds_real_cargo_when_nested_environment_drops_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "junit.xml"
            retained = root / "retained"
            real_bin = root / "real-bin"
            wrapper_bin = root / "wrapper-bin"
            real_bin.mkdir()
            wrapper_bin.mkdir()
            fake_cargo = real_bin / "cargo"
            fake_cargo.write_text(
                """#!/usr/bin/env bash
set -euo pipefail
case "$2" in
    fail) printf '<nested-failure/>\\n' > "$LC_NEXTEST_JUNIT_SOURCE"; exit 31 ;;
    pass) printf '<nested-success/>\\n' > "$LC_NEXTEST_JUNIT_SOURCE" ;;
    *) exit 2 ;;
esac
""",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            wrapper = wrapper_bin / "cargo"
            wrapper.write_bytes(RETAIN_CARGO.read_bytes())
            wrapper.chmod(0o755)
            environment = os.environ.copy()
            environment.pop("LC_REAL_CARGO", None)
            environment.update(
                {
                    "CARGO": str(wrapper),
                    "PATH": f"{wrapper_bin}:{real_bin}:{environment['PATH']}",
                    "LC_NEXTEST_JUNIT_SOURCE": str(source),
                    "LC_NEXTEST_JUNIT_DIR": str(retained),
                    "LC_NEXTEST_JUNIT_HELPER": str(RETAIN_JUNIT),
                    "STATUS_FILE": str(root / "status"),
                }
            )
            completed = subprocess.run(
                [
                    "bash",
                    "-euo",
                    "pipefail",
                    "-c",
                    """first=0
second=0
"$CARGO" nextest fail || first=$?
"$CARGO" nextest pass
printf "%s\\n" "$first" > "$STATUS_FILE"
""",
                ],
                cwd=REPOSITORY,
                env=environment,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual((root / "status").read_text(encoding="utf-8"), "31\n")
            reports = sorted(retained.glob("junit-*.xml"))
            self.assertEqual(len(reports), 2)
            self.assertEqual(
                {path.read_text(encoding="utf-8") for path in reports},
                {"<nested-failure/>\n", "<nested-success/>\n"},
            )

    def test_parity_keeps_the_wrapper_after_cargo_rewrites_its_environment(self):
        source = PARITY.read_text(encoding="utf-8")
        self.assertIn('std::env::var_os("LC_CARGO_WRAPPER")', source)
        self.assertIn("cargo_program(", source)

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
        self.assertIn("JUNIT_SOURCE: target/nextest/default/junit.xml", linux)
        self.assertIn('Path(os.environ["JUNIT_SOURCE"]).unlink(missing_ok=True)', linux)
        self.assertIn('"run_id": int(os.environ["GITHUB_RUN_ID"])', linux)
        self.assertIn('"run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"])', linux)
        self.assertIn('"shard": os.environ["SHARD_ID"]', linux)
        self.assertIn('"name": os.environ["MATRIX_NAME"]', linux)
        self.assertIn('"command": os.environ["SHARD_COMMAND"]', linux)
        self.assertIn('"source_sha": os.environ["SOURCE_SHA"]', linux)
        self.assertIn('"junit": os.environ["JUNIT_SOURCE"]', linux)
        self.assertIn('"retained_junit": "junit/*.xml"', linux)
        upload = linux[linux.index("- name: Upload shard test diagnostics") :]
        self.assertIn("if: always()", upload)
        self.assertIn(
            "rust-test-diagnostics-${{ github.run_id }}-${{ github.run_attempt }}-"
            "${{ matrix.artifact }}-${{ github.sha }}",
            upload,
        )
        self.assertIn("${{ runner.temp }}/test-diagnostics", upload)
        self.assertIn("${{ env.JUNIT_SOURCE }}", upload)
        self.assertIn("if-no-files-found: warn", upload)
        self.assertIn('export LC_NEXTEST_JUNIT_DIR="$DIAGNOSTIC_DIR/junit"', linux)
        self.assertIn("retain-nextest-cargo.sh", linux)
        self.assertIn('cargo_wrapper_dir="$RUNNER_TEMP/nextest-cargo-wrapper"', linux)
        self.assertIn('export PATH="$cargo_wrapper_dir:$PATH"', linux)
        wrapper = RETAIN_CARGO.read_text(encoding="utf-8")
        self.assertIn('rm -f "$source_path"', wrapper)
        self.assertIn('bash "$helper" || true', RETAIN_CARGO.read_text(encoding="utf-8"))
        self.assertIn('if [[ "$argument" == nextest ]]; then', wrapper)

    def test_exact_sha_shards_upload_diagnostics_outside_coverage_handoff(self):
        collectors = job_block(QUALIFICATION, "coverage-fragments")
        self.assertIn("CARGO_TARGET_DIR: target/coverage-build", collectors)
        self.assertIn(
            "JUNIT_SOURCE: target/coverage-build/nextest/default/junit.xml", collectors
        )
        self.assertIn("- name: Prepare shard diagnostics", collectors)
        self.assertIn("shutil.rmtree(diagnostic_dir)", collectors)
        self.assertIn("- name: Reset stale JUnit report", collectors)
        self.assertIn('Path(os.environ["JUNIT_SOURCE"]).unlink(missing_ok=True)', collectors)
        exact_artifacts = re.findall(
            r"(?m)^            artifact: ([a-z0-9-]+)$", collectors
        )
        self.assertEqual(len(exact_artifacts), 12)
        self.assertEqual(len(set(exact_artifacts)), 12)
        self.assertIn('"run_id": int(os.environ["GITHUB_RUN_ID"])', collectors)
        self.assertIn('"run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"])', collectors)
        self.assertIn('"shard": os.environ["SHARD_ID"]', collectors)
        self.assertIn('"name": os.environ["MATRIX_NAME"]', collectors)
        self.assertIn('"command": os.environ["SHARD_COMMAND"]', collectors)
        self.assertIn('"source_sha": os.environ["SOURCE_SHA"]', collectors)
        self.assertIn('"junit": os.environ["JUNIT_SOURCE"]', collectors)
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
            "coverage-${{ matrix.artifact }}-${{ inputs.source-sha }}",
            upload,
        )
        self.assertIn("${{ runner.temp }}/test-diagnostics", upload)
        self.assertIn("${{ env.JUNIT_SOURCE }}", upload)
        self.assertLess(
            upload.index("Upload shard test diagnostics"),
            upload.index("Upload coverage fragment"),
        )
        self.assertIn('export LC_NEXTEST_JUNIT_DIR="$DIAGNOSTIC_DIR/junit"', collectors)
        self.assertIn("retain-nextest-cargo.sh", collectors)
        self.assertIn('cargo_wrapper_dir="$RUNNER_TEMP/nextest-cargo-wrapper"', collectors)
        self.assertIn('export PATH="$cargo_wrapper_dir:$PATH"', collectors)

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
