import os
import pathlib
import subprocess
import tempfile
import unittest

from _repo import REPOSITORY

METRIC_LINE = (
    "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=1.000000 successful_present_submissions={total} retained_gpu_present_submissions={gpu} cpu_present_submissions={cpu} presentation_submission_fps=1.000000 refreshed_frames=1 simulation_frames=1 simulation_fps=1.000000 automatic_graphics_skips=0"
)

PINNED_ORACLE_REVISION = "7d43b47b7d789b533f32d005e64596e0a07019cd"


class PublicScriptPortabilityTests(unittest.TestCase):
    def test_public_scripts_and_docs_do_not_assume_a_private_checkout_layout(self):
        paths = {
            *REPOSITORY.glob("*.md"),
            *(REPOSITORY / ".github").rglob("*.yml"),
            *(REPOSITORY / ".github").rglob("*.yaml"),
            *(REPOSITORY / "parity").rglob("*.md"),
            *(REPOSITORY / "parity").rglob("*.sh"),
            *(REPOSITORY / "scripts").glob("*.md"),
            *(REPOSITORY / "scripts").glob("*.py"),
            *(REPOSITORY / "scripts").glob("*.sh"),
        }
        forbidden = (
            "/Users/" + "tyler",
            "~/" + "Documents/code",
            "../" + "../vendor/" + "legacyclonk-" + "oracle",
            "vendor/" + "legacyclonk-" + "oracle",
            "/private" + "/tmp",
            "oracle-src-" + "pinned",
        )

        violations = []
        for path in sorted(paths):
            text = path.read_text(encoding="utf-8")
            for value in forbidden:
                if value in text:
                    violations.append(f"{path.relative_to(REPOSITORY)}: {value}")

        self.assertEqual(violations, [])

    def test_oracle_generator_defaults_to_the_pinned_repository_history(self):
        generator = (
            REPOSITORY / "parity/oracle/gen_golden.sh"
        ).read_text(encoding="utf-8")

        self.assertIn(
            'oracle_repo="${LEGACYCLONK_ORACLE_ROOT:-$repo}"',
            generator,
        )
        self.assertIn(
            PINNED_ORACLE_REVISION,
            generator,
        )
        subprocess.run(
            [
                "git",
                "-C",
                str(REPOSITORY),
                "cat-file",
                "-e",
                f"{PINNED_ORACLE_REVISION}:src/Fixed.h",
            ],
            check=True,
        )

    def test_deep_sea_benchmark_uses_the_standard_temp_environment(self):
        # Every Deep Sea arm builds its fixture through the shared helper, so
        # the temp-environment contract is asserted where it now lives rather
        # than once per wrapper.
        fixture = (
            REPOSITORY / "scripts/deep-sea-benchmark-fixture.sh"
        ).read_text(encoding="utf-8")

        self.assertIn("${TMPDIR:-/tmp}", fixture)

        for wrapper in (
            "scripts/run-deep-sea-gpu-benchmark.sh",
            "scripts/run-deep-sea-software-benchmark.sh",
        ):
            with self.subTest(wrapper=wrapper):
                self.assertIn(
                    "deep-sea-benchmark-fixture.sh",
                    (REPOSITORY / wrapper).read_text(encoding="utf-8"),
                )

    def test_deep_sea_benchmark_assigns_two_distinct_players_to_ordered_teams(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = pathlib.Path(temporary)
            binary = temporary / "fake-clonk-app"
            binary.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                'test "${LC_APP_PRESENTATION_BENCHMARK_PLAYER_TEAMS:-}" = "1,2"\n'
                'test "$#" -eq 5\n'
                'test -f "$4"\n'
                'test -f "$5"\n'
                'test "$4" != "$5"\n'
                'printf "%s\\n" "LC_APP_PRESENTATION_BENCHMARK_CONTEXT '
                "runtime_players=2 synchronized_player_infos=2 "
                "activated_nonhost_clients=0 runtime_crew_objects=2 "
                "runtime_players_with_live_crew=2 "
                "runtime_players_with_exactly_one_live_sf5b_crew=0 "
                "runtime_st5b_objects_at_measurement_start=0 "
                'runtime_st5b_objects_at_measurement_end=0"\n',
                encoding="utf-8",
            )
            binary.chmod(0o755)
            environment = os.environ.copy()
            environment.update(
                {
                    "LC_APP_BINARY": str(binary),
                    "TMPDIR": str(temporary),
                }
            )

            result = subprocess.run(
                [str(REPOSITORY / "scripts/run-deep-sea-gpu-benchmark.sh"), "1"],
                cwd=REPOSITORY,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def _software_benchmark_with_counts(self, temporary, gpu, cpu):
        """Run the software wrapper against a binary reporting these counts."""
        binary = temporary / "fake-clonk-app"
        binary.write_text(
            "#!/usr/bin/env bash\n"
            "set -eu\n"
            'test "${LC_SOFTWARE_PRESENTATION:-}" = "1"\n'
            'printf "%s\\n" "'
            + METRIC_LINE.format(gpu=gpu, cpu=cpu, total=gpu + cpu)
            + '"\n',
            encoding="utf-8",
        )
        binary.chmod(0o755)
        environment = os.environ.copy()
        environment.update({"LC_APP_BINARY": str(binary), "TMPDIR": str(temporary)})
        return subprocess.run(
            [str(REPOSITORY / "scripts/run-deep-sea-software-benchmark.sh"), "1"],
            cwd=REPOSITORY,
            env=environment,
            check=False,
            capture_output=True,
            text=True,
        )

    def test_software_benchmark_requests_software_presentation_and_accepts_it(self):
        # The wrapper's fake binary asserts LC_SOFTWARE_PRESENTATION itself, so
        # a run that forgot to request it fails before any counting.
        with tempfile.TemporaryDirectory() as temporary:
            result = self._software_benchmark_with_counts(
                pathlib.Path(temporary), gpu=0, cpu=36
            )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_software_benchmark_rejects_a_run_that_presented_through_the_gpu(self):
        # An unarmed run produces a complete, healthy-looking distribution of
        # the wrong path. It has to fail rather than be published as software.
        with tempfile.TemporaryDirectory() as temporary:
            result = self._software_benchmark_with_counts(
                pathlib.Path(temporary), gpu=36, cpu=0
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("LC_SOFTWARE_PRESENTATION did not take", result.stderr)

    def test_software_benchmark_rejects_a_run_with_no_software_presentations(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = self._software_benchmark_with_counts(
                pathlib.Path(temporary), gpu=0, cpu=0
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no software presentations were measured", result.stderr)

    def test_deep_sea_benchmark_rejects_a_nonplaying_runtime_context(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = pathlib.Path(temporary)
            binary = temporary / "fake-clonk-app"
            binary.write_text(
                "#!/usr/bin/env bash\n"
                'printf "%s\\n" "LC_APP_PRESENTATION_BENCHMARK_CONTEXT '
                "runtime_players=1 synchronized_player_infos=1 "
                "activated_nonhost_clients=0 runtime_crew_objects=0 "
                "runtime_players_with_live_crew=0 "
                "runtime_players_with_exactly_one_live_sf5b_crew=0 "
                "runtime_st5b_objects_at_measurement_start=0 "
                'runtime_st5b_objects_at_measurement_end=0"\n',
                encoding="utf-8",
            )
            binary.chmod(0o755)
            environment = os.environ.copy()
            environment.update(
                {
                    "LC_APP_BINARY": str(binary),
                    "TMPDIR": str(temporary),
                }
            )

            result = subprocess.run(
                [str(REPOSITORY / "scripts/run-deep-sea-gpu-benchmark.sh"), "1"],
                cwd=REPOSITORY,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("expected runtime_players=2", result.stderr)

    def test_public_shell_scripts_pass_bash_syntax_validation(self):
        scripts = [
            REPOSITORY / "parity/oracle/gen_golden.sh",
            *sorted(REPOSITORY.glob("scripts/*.sh")),
        ]
        subprocess.run(
            ["bash", "-n", *map(str, scripts)],
            check=True,
        )


if __name__ == "__main__":
    unittest.main()
