"""Main validation keeps post-merge host-specific material oracles visible."""

import re
import unittest

from _repo import REPOSITORY

WORKFLOW = REPOSITORY / ".github" / "workflows" / "exact-sha-qualification.yml"
ORACLE_REASON = (
    "recording-host material order; required macOS CI job"
)
# Folder groups enumerate in their packed sort order rather than host readdir
# order (clonk-org/clonk-rs#1455), so these material-order oracles are
# host-independent and must not be platform-gated. The macOS job still runs
# them, as a cross-host check that the order really is host-independent.
MATERIAL_ORACLES = {
    "alchemy_real_scenario_subcases_batch_4",
    "app_virtual_keyboard_completes_real_tutorial02_route",
    "committed_real_scenario_replays_are_deterministic",
    "real_tutorial_seven_acid_rain_matches_cpp_animated_pxs_sequence",
    "tutorial02_virtual_player_completes_the_real_tutorial_route",
    "tutorial07_seed_zero_landscape_matches_cpp_surface8",
    "tutorial_landscapes_match_cpp_surface8_across_scenarios_and_seeds",
}


def step_script(name):
    """Return one workflow step's literal block without a YAML dependency."""
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


def recording_host_oracles():
    """Find every test deliberately ignored away from the recording host."""
    reason = re.escape(ORACLE_REASON)
    pattern = re.compile(
        rf"(?:#\[test\]\s*)?"
        rf'#\[cfg_attr\(\s*not\(target_os\s*=\s*"macos"\),\s*'
        rf'ignore\s*=\s*"{reason}"\s*\)\]\s*'
        rf"(?:#\[test\]\s*)?fn\s+([A-Za-z0-9_]+)",
        re.MULTILINE,
    )
    found = set()
    for source in (REPOSITORY / "crates").rglob("*.rs"):
        found.update(pattern.findall(source.read_text(encoding="utf-8")))
    return found


class RecordingHostGateTests(unittest.TestCase):
    def test_material_oracles_are_not_platform_gated(self):
        self.assertEqual(recording_host_oracles(), set())

    def test_named_macos_job_runs_every_material_oracle(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        job = workflow.index("  recording-host-oracles:")
        next_job = re.search(r"(?m)^  [A-Za-z0-9_-]+:$", workflow[job + 1 :])
        end = job + 1 + next_job.start() if next_job else None
        block = workflow[job:end]
        self.assertIn("name: Recording-host material-order oracles (macOS)", block)
        self.assertIn("runs-on: macos-latest", block)

        script = step_script("Run recording-host material-order oracles")
        self.assertEqual(script.count("cargo nextest run"), 1)
        for package in (
            "clonk-app",
            "clonk-engine-integration-tests",
            "clonk-frontend-unit-tests",
        ):
            with self.subTest(package=package):
                self.assertRegex(script, rf"(?:^|\s)-p\s+{re.escape(package)}\b")
        self.assertRegex(script, r"\s-E\s+")
        for oracle in MATERIAL_ORACLES:
            with self.subTest(oracle=oracle):
                self.assertIn(oracle, script)


if __name__ == "__main__":
    unittest.main()
