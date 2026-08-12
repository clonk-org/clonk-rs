import os
import pathlib
import subprocess
import tempfile
import unittest

from _repo import REPOSITORY

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
        benchmark = (
            REPOSITORY / "scripts/run-deep-sea-gpu-benchmark.sh"
        ).read_text(encoding="utf-8")

        self.assertIn("${TMPDIR:-/tmp}", benchmark)

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
