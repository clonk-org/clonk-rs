import subprocess
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
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

    def test_public_shell_scripts_pass_bash_syntax_validation(self):
        subprocess.run(
            [
                "bash",
                "-n",
                str(REPOSITORY / "parity/oracle/gen_golden.sh"),
                str(REPOSITORY / "scripts/run-deep-sea-gpu-benchmark.sh"),
            ],
            check=True,
        )


if __name__ == "__main__":
    unittest.main()
