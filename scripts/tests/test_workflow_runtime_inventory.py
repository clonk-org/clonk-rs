"""Static guards keeping CI and release checks aligned with shipped binaries."""

import unittest

from _repo import REPOSITORY

LANDING_WORKFLOW = REPOSITORY / ".github" / "workflows" / "landing.yml"
MAIN_WORKFLOW = REPOSITORY / ".github" / "workflows" / "rust.yml"
EXACT_SHA_QUALIFICATION_WORKFLOW = (
    REPOSITORY / ".github" / "workflows" / "exact-sha-qualification.yml"
)
RELEASE_WORKFLOW = REPOSITORY / ".github" / "workflows" / "release.yml"
RELEASE_BUILD_WORKFLOW = REPOSITORY / ".github" / "workflows" / "release-build.yml"
MSVC_RUNTIME_CONFIG = REPOSITORY / "scripts" / "configure-msvc-runtime.sh"
MSVC_RUNTIME_VALIDATION = REPOSITORY / "scripts" / "validate-msvc-runtime.sh"


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
    def test_checkouts_do_not_persist_credentials(self):
        marker = "uses: actions/checkout@"

        for workflow in (
            LANDING_WORKFLOW,
            MAIN_WORKFLOW,
            EXACT_SHA_QUALIFICATION_WORKFLOW,
            RELEASE_WORKFLOW,
            RELEASE_BUILD_WORKFLOW,
        ):
            source = workflow.read_text(encoding="utf-8")
            blocks = [
                block.split("\n      - ", 1)[0]
                for block in source.split(marker)[1:]
            ]
            self.assertTrue(blocks, workflow.name)
            for index, block in enumerate(blocks, start=1):
                with self.subTest(workflow=workflow.name, checkout=index):
                    self.assertIn("persist-credentials: false", block)

    def test_landing_workflow_uses_current_pinned_actions_and_nextest(self):
        workflow = LANDING_WORKFLOW.read_text(encoding="utf-8")
        checkout = (
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
            " # v7.0.1"
        )

        self.assertEqual(workflow.count(checkout), 2)
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

    def test_landing_smoke_covers_c4group_everywhere_it_is_inventoried(self):
        expected = {
            "Test the launcher and path resolution": "-p clonk-c4group",
            "Lint Windows paths": "-p clonk-c4group",
            "Compile the installer over a stand-in payload": (
                '$payload_dir/bin/c4group.exe'
            ),
        }
        for step, fragment in expected.items():
            with self.subTest(step=step):
                self.assertIn(fragment, step_script(LANDING_WORKFLOW, step))

    def test_msvc_builds_share_cached_linker_plugin_lto_configuration(self):
        landing = LANDING_WORKFLOW.read_text(encoding="utf-8")
        self.assertNotIn("runtime-msvc:", landing)
        self.assertNotIn("run: scripts/configure-msvc-runtime.sh", landing)
        self.assertNotIn("run: scripts/validate-msvc-runtime.sh", landing)

        for workflow in (MAIN_WORKFLOW, RELEASE_BUILD_WORKFLOW):
            with self.subTest(workflow=workflow.name):
                self.assertEqual(
                    workflow.read_text(encoding="utf-8").count(
                        "run: scripts/configure-msvc-runtime.sh"
                    ),
                    1,
                )
                self.assertEqual(
                    workflow.read_text(encoding="utf-8").count(
                        "run: scripts/validate-msvc-runtime.sh"
                    ),
                    1,
                )
                self.assertNotIn("_LINK_:", workflow.read_text(encoding="utf-8"))
                self.assertNotIn(
                    "CARGO_PROFILE_RELEASE_LTO",
                    workflow.read_text(encoding="utf-8"),
                )

        script = MSVC_RUNTIME_CONFIG.read_text(encoding="utf-8")
        for fragment in (
            "release: 1.97.1",
            "LLVM version: 22.1.6",
            "cargo_target=x86_64-pc-windows-msvc",
            'CARGO_BUILD_TARGET=$cargo_target',
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
            "expected_toolchain=1.97.1-x86_64-pc-windows-msvc",
            "rustup toolchain list --quiet",
            'rustup toolchain uninstall "$installed"',
            "-Ctarget-feature=+crt-static",
            "-Clinker-plugin-lto",
            "-Clinker-flavor=lld-link",
            "-Clink-arg=/lldltocache:",
            "cache_size_bytes=512m",
            "-Clink-arg=/DEBUG:NONE",
            "-Clink-arg=/OPT:REF,ICF",
            "-Clink-arg=/TIME",
            "-Clink-arg=/Brepro",
            "unset LINK _LINK_",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, script)
        self.assertNotIn("CARGO_PROFILE_RELEASE_LTO", script)
        self.assertNotIn("MSVC_THINLTO_BENCHMARK", script)
        self.assertNotIn("prune_interval", script)

    def test_msvc_runtime_validation_executes_and_inspects_exact_outputs(self):
        script = MSVC_RUNTIME_VALIDATION.read_text(encoding="utf-8")
        for fragment in (
            "vswhere.exe",
            "dumpbin.exe",
            "/DEPENDENTS",
            "for binary in clonk-app clonk-game c4group",
            "VCRUNTIME|MSVCP|CONCRT|UCRTBASE|API-MS-WIN-CRT|MSVCR",
            '"$binary_dir/clonk-game.exe" --version',
            '"$binary_dir/c4group.exe"',
            "sha256sum",
            "llvmcache-*",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, script)
        self.assertIn("THINLTO_CACHE_DIR", script)

    def test_thinlto_cache_is_published_only_by_trusted_main(self):
        landing = LANDING_WORKFLOW.read_text(encoding="utf-8")
        main = MAIN_WORKFLOW.read_text(encoding="utf-8")
        release_build = RELEASE_BUILD_WORKFLOW.read_text(encoding="utf-8")
        restore = (
            "actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9"
            " # v6.1.0"
        )
        save = (
            "actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9"
            " # v6.1.0"
        )

        self.assertEqual(landing.count(restore), 0)
        self.assertNotIn(save, landing)
        self.assertEqual(main.count(restore), 1)
        self.assertEqual(main.count(save), 1)
        self.assertEqual(release_build.count(restore), 1)
        self.assertNotIn(save, release_build)

        trusted_save = main[main.index("Publish trusted ThinLTO cache") :]
        for guard in (
            "github.event_name == 'push'",
            "github.event_name == 'workflow_dispatch'",
            "github.ref == 'refs/heads/main'",
            "steps.thinlto-cache.outputs.cache-hit != 'true'",
        ):
            self.assertIn(guard, trusted_save)

        production_key = (
            "clonk-msvc-thinlto-v1-windows-x64-rustc-1.97.1-llvm-22.1.6-${{ "
            "hashFiles('rust-toolchain.toml', '.cargo/config.toml', 'Cargo.toml', "
            "'Cargo.lock', 'crates/**/Cargo.toml', "
            "'scripts/configure-msvc-runtime.sh') }}"
        )
        self.assertNotIn("clonk-msvc-thinlto", landing)
        for workflow in (main, release_build):
            with self.subTest(workflow=workflow):
                self.assertIn(production_key, workflow)
                self.assertNotIn("restore-keys:", workflow)

    def test_exact_msvc_runtime_build_is_post_merge_and_release_gated(self):
        landing = LANDING_WORKFLOW.read_text(encoding="utf-8")
        main = MAIN_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("Build the Windows packaging tool", landing)
        self.assertNotIn("Build the runtime exactly as a release ships it", landing)
        self.assertNotIn("cargo build --release -p clonk-app", landing)
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
        release_gate = step_script(RELEASE_WORKFLOW, "Require exact-SHA validation")
        self.assertIn("for workflow in landing.yml rust.yml", release_gate)

    def test_installer_smoke_compiles_the_release_icon_branch(self):
        script = step_script(
            LANDING_WORKFLOW, "Compile the installer over a stand-in payload"
        )
        self.assertIn("crates/clonk-icon/res/windows/c4x.ico", script)
        self.assertIn('-DICON="$icon"', script)

    def test_windows_installer_toolchain_is_exactly_pinned(self):
        scripts = (
            step_script(LANDING_WORKFLOW, "Install NSIS"),
            step_script(
                RELEASE_BUILD_WORKFLOW, "Install the Windows installer toolchain"
            ),
        )
        for script in scripts:
            with self.subTest(script=script):
                self.assertIn(
                    "choco install nsis --version 3.12.0 --yes --no-progress",
                    script,
                )
                self.assertIn('"$nsis_dir/makensis.exe" /VERSION', script)
                self.assertIn("expected NSIS v3.12", script)

    def test_windows_installer_toolchain_retries_transient_download_failures(self):
        scripts = (
            step_script(LANDING_WORKFLOW, "Install NSIS"),
            step_script(
                RELEASE_BUILD_WORKFLOW, "Install the Windows installer toolchain"
            ),
        )
        for script in scripts:
            with self.subTest(script=script):
                self.assertIn("for attempt in 1 2 3; do", script)
                self.assertIn(
                    "if choco install nsis --version 3.12.0 --yes --no-progress; then",
                    script,
                )
                self.assertIn('if [[ "$attempt" -eq 3 ]]; then', script)
                self.assertIn('sleep "$((attempt * 10))"', script)
                self.assertNotIn("--ignore-checksums", script)
                self.assertNotIn("--allow-empty-checksums", script)

    def test_universal_release_verifies_every_shipped_binary(self):
        script = step_script(
            RELEASE_BUILD_WORKFLOW, "Check the macOS build is universal"
        )

        self.assertIn("for binary in clonk-app clonk-game c4group", script)


if __name__ == "__main__":
    unittest.main()
