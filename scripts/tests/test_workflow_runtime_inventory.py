"""Static guards keeping CI and release checks aligned with shipped binaries."""

import unittest
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[2]
LANDING_WORKFLOW = REPOSITORY / ".github" / "workflows" / "landing.yml"
MAIN_WORKFLOW = REPOSITORY / ".github" / "workflows" / "rust.yml"
RELEASE_WORKFLOW = REPOSITORY / ".github" / "workflows" / "release.yml"


def step_script(workflow, name):
    """Return the verbatim `run` command for one named workflow step."""
    lines = workflow.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(f"      - name: {name}")
    except ValueError:
        raise AssertionError(f"{workflow.name} has no step named {name!r}") from None

    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line.startswith("      - "):
            break
        if line.startswith("        run: ") and line != "        run: |":
            return line.removeprefix("        run: ")
        if line == "        run: |":
            body = []
            for candidate in lines[index + 1 :]:
                if candidate.strip() and not candidate.startswith(" " * 10):
                    break
                body.append(candidate[10:])
            return "\n".join(body)
    raise AssertionError(f"step {name!r} has no `run: |` block")


class WorkflowRuntimeInventoryTests(unittest.TestCase):
    def test_landing_workflow_uses_current_pinned_actions_and_nextest(self):
        workflow = LANDING_WORKFLOW.read_text(encoding="utf-8")
        checkout = (
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
            " # v7.0.1"
        )

        self.assertEqual(workflow.count(checkout), 3)
        self.assertNotIn("actions/checkout@11d5960a326750d5838078e36cf38b85af677262", workflow)
        self.assertIn("tool: cargo-nextest@0.9.91", workflow)

        upload = "actions/upload-artifact@"
        for line in workflow.splitlines():
            if upload in line:
                self.assertIn(
                    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
                    " # v7.0.1",
                    line,
                )

    def test_landing_jobs_cover_c4group_everywhere_the_runtime_is_inventoried(self):
        expected = {
            "Test the launcher and path resolution": "-p clonk-c4group",
            "Lint Windows paths": "-p clonk-c4group",
            "Build the runtime exactly as a release ships it": "-p clonk-c4group",
            "Check the runtime is self-contained": (
                "for binary in clonk-app clonk-game c4group"
            ),
            "Compile the installer over a stand-in payload": (
                '$payload_dir/bin/c4group.exe'
            ),
        }
        for step, fragment in expected.items():
            with self.subTest(step=step):
                self.assertIn(fragment, step_script(LANDING_WORKFLOW, step))

    def test_windows_release_tool_build_is_post_merge_and_not_runtime_serial(self):
        landing = LANDING_WORKFLOW.read_text(encoding="utf-8")
        main = MAIN_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("Build the Windows packaging tool", landing)
        self.assertIn(
            "cargo build --release --locked -p xtask --features engine-tools "
            "--bin xtask-engine-tools",
            step_script(MAIN_WORKFLOW, "Build the Windows packaging tool"),
        )
        self.assertIn(
            "cargo build --release -p clonk-app -p clonk-game -p clonk-c4group "
            "--locked --timings",
            step_script(MAIN_WORKFLOW, "Refresh the shipped MSVC runtime cache"),
        )

    def test_installer_smoke_compiles_the_release_icon_branch(self):
        script = step_script(
            LANDING_WORKFLOW, "Compile the installer over a stand-in payload"
        )
        self.assertIn("crates/clonk-icon/res/windows/c4x.ico", script)
        self.assertIn('-DICON="$icon"', script)

    def test_universal_release_verifies_every_shipped_binary(self):
        script = step_script(RELEASE_WORKFLOW, "Check the macOS build is universal")

        self.assertIn("for binary in clonk-app clonk-game c4group", script)


if __name__ == "__main__":
    unittest.main()
