"""Static guards keeping CI and release checks aligned with shipped binaries."""

import unittest

from _repo import REPOSITORY

LANDING_WORKFLOW = REPOSITORY / ".github" / "workflows" / "landing.yml"
MAIN_WORKFLOW = REPOSITORY / ".github" / "workflows" / "rust.yml"
EXACT_SHA_QUALIFICATION_WORKFLOW = (
    REPOSITORY / ".github" / "workflows" / "exact-sha-qualification.yml"
)
RELEASE_WORKFLOW = REPOSITORY / ".github" / "workflows" / "release.yml"
RELEASE_PREPARE_WORKFLOW = (
    REPOSITORY / ".github" / "workflows" / "release-prepare.yml"
)
RELEASE_BUILD_WORKFLOW = REPOSITORY / ".github" / "workflows" / "release-build.yml"
RELEASE_PREBUILD_WORKFLOW = (
    REPOSITORY / ".github" / "workflows" / "release-prebuild.yml"
)
MSVC_RUNTIME_CONFIG = REPOSITORY / "scripts" / "configure-msvc-runtime.sh"
MSVC_RUNTIME_VALIDATION = REPOSITORY / "scripts" / "validate-msvc-runtime.sh"
WINDOWS_INSTALLER = REPOSITORY / "scripts" / "windows-installer.nsi"
NSIS_INSTALLER = REPOSITORY / "scripts" / "install-nsis.sh"


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
    def test_windows_installer_uses_fast_solid_compression(self):
        installer = WINDOWS_INSTALLER.read_text(encoding="utf-8")

        self.assertIn("SetCompressor /SOLID zlib", installer)
        self.assertNotIn("SetCompressor /SOLID lzma", installer)

    def test_checkouts_do_not_persist_credentials(self):
        marker = "uses: actions/checkout@"

        for workflow in (
            LANDING_WORKFLOW,
            MAIN_WORKFLOW,
            EXACT_SHA_QUALIFICATION_WORKFLOW,
            RELEASE_WORKFLOW,
            RELEASE_BUILD_WORKFLOW,
            RELEASE_PREBUILD_WORKFLOW,
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

    def test_exact_sha_coverage_uses_current_pinned_artifact_handoffs(self):
        workflow = EXACT_SHA_QUALIFICATION_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn(
            "actions/upload-artifact@"
            "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1",
            workflow,
        )
        self.assertIn(
            "actions/download-artifact@"
            "3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1",
            workflow,
        )
        self.assertNotIn("actions/cache/restore@", workflow)
        self.assertNotIn("actions/cache/save@", workflow)

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

        for workflow in (MAIN_WORKFLOW, RELEASE_PREBUILD_WORKFLOW):
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
        release_prebuild = RELEASE_PREBUILD_WORKFLOW.read_text(encoding="utf-8")
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
        thinlto_start = release_prebuild.index("Restore trusted-main ThinLTO cache")
        thinlto_end = release_prebuild.index("\n      - name:", thinlto_start)
        thinlto_step = release_prebuild[thinlto_start:thinlto_end]
        self.assertEqual(thinlto_step.count(restore), 1)
        self.assertNotIn("Publish trusted ThinLTO cache", release_prebuild)
        self.assertNotIn(
            "key: ${{ steps.thinlto-cache.outputs.cache-primary-key }}",
            release_prebuild,
        )

        trusted_save = main[main.index("Publish trusted ThinLTO cache") :]
        for guard in (
            "github.event_name == 'push'",
            "github.event_name == 'workflow_dispatch'",
            "github.ref == 'refs/heads/main'",
            "steps.thinlto-cache.outputs.cache-hit != 'true'",
        ):
            self.assertIn(guard, trusted_save)

        production_key = (
            "clonk-msvc-thinlto-v2-windows-x64-rustc-1.97.1-llvm-22.1.6-${{ "
            "hashFiles('rust-toolchain.toml', '.cargo/config.toml', "
            "'scripts/configure-msvc-runtime.sh', 'crates/**/*.rs') }}"
        )
        self.assertNotIn("clonk-msvc-thinlto", landing)
        for workflow in (main, release_prebuild):
            with self.subTest(workflow=workflow):
                self.assertIn(production_key, workflow)
                self.assertNotIn("restore-keys:", workflow)

        # A release commit changes the workspace package version in both files.
        # LLVM's cache validates its own bitcode keys, while including either
        # metadata file here would force every release to start empty.
        for metadata in ("Cargo.toml", "Cargo.lock", "crates/**/Cargo.toml"):
            with self.subTest(metadata=metadata):
                self.assertNotIn(metadata, production_key)

    def test_exact_msvc_runtime_build_is_merge_group_release_gated(self):
        landing = LANDING_WORKFLOW.read_text(encoding="utf-8")
        main = MAIN_WORKFLOW.read_text(encoding="utf-8")
        prebuild = RELEASE_PREBUILD_WORKFLOW.read_text(encoding="utf-8")
        release_build = RELEASE_BUILD_WORKFLOW.read_text(encoding="utf-8")
        release = RELEASE_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("Build the Windows packaging tool", landing)
        self.assertNotIn("Build the runtime exactly as a release ships it", landing)
        self.assertNotIn("cargo build --release -p clonk-app", landing)
        self.assertIn("run: scripts/configure-msvc-runtime.sh", prebuild)
        self.assertIn("run: scripts/validate-msvc-runtime.sh", prebuild)
        self.assertNotIn("cargo build --release", release_build)
        self.assertIn("scripts/release-prebuild-manifest.py verify", release_build)
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
        for fragment in (
            "if: needs.release-context.outputs.release == 'true'",
            "uses: ./.github/workflows/release-build.yml",
            "source-sha: ${{ github.sha }}",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, landing)

        artifact_resolver = step_script(
            RELEASE_WORKFLOW, "Resolve exact-SHA release artifacts"
        )
        self.assertIn("--workflow landing.yml", artifact_resolver)
        self.assertNotIn("rust.yml", artifact_resolver)
        self.assertIn(
            "run-id: ${{ steps.artifacts.outputs.run-id }}", release
        )
        qualification = EXACT_SHA_QUALIFICATION_WORKFLOW.read_text(encoding="utf-8")
        collectors = qualification[
            qualification.index("  coverage-fragments:") : qualification.index(
                "  coverage:"
            )
        ]
        upload = collectors.index("- name: Upload coverage fragment")
        self.assertRegex(
            collectors[upload : upload + 180],
            r"- name: Upload coverage fragment\n"
            r"        uses: actions/upload-artifact@",
        )
        self.assertNotIn("actions/cache/save@", collectors)
        self.assertNotIn("actions/cache/restore@", qualification)
        coverage = qualification[
            qualification.index("  coverage:") : qualification.index(
                "  coverage-html:"
            )
        ]
        self.assertRegex(
            coverage,
            r"- name: Download coverage fragments\n"
            r"        uses: actions/download-artifact@",
        )

    def test_installer_smoke_compiles_the_release_icon_branch(self):
        script = step_script(
            LANDING_WORKFLOW, "Compile the installer over a stand-in payload"
        )
        self.assertIn("crates/clonk-icon/res/windows/c4x.ico", script)
        self.assertIn('-DICON="$icon"', script)

    def test_windows_installer_toolchain_uses_verified_upstream_archive(self):
        workflow_scripts = (
            step_script(LANDING_WORKFLOW, "Install NSIS"),
            step_script(
                RELEASE_BUILD_WORKFLOW, "Install the Windows installer toolchain"
            ),
        )
        self.assertEqual(
            workflow_scripts,
            ("scripts/install-nsis.sh", "scripts/install-nsis.sh"),
        )

        self.assertTrue(NSIS_INSTALLER.stat().st_mode & 0o111)
        script = NSIS_INSTALLER.read_text(encoding="utf-8")
        url = (
            "https://downloads.sourceforge.net/project/nsis/"
            "NSIS%203/3.12/nsis-3.12.zip"
        )
        digest = "56581f90db321581c5381193d796fffcf2d24b2f8fed2160a6c6a3baa67f2c4f"
        self.assertIn(f"nsis_url='{url}'", script)
        self.assertIn(f"nsis_sha256='{digest}'", script)
        self.assertIn("curl -fsSL --retry 5 --retry-all-errors", script)
        download = '--output "$archive" "$nsis_url"'
        checksum = 'actual_sha256=$(sha256sum "$archive"'
        digest_check = 'if [[ "$actual_sha256" != "$nsis_sha256" ]]; then'
        extraction = 'python3 -m zipfile -e "$archive" "$runner_temp"'
        extracted_root = 'nsis_dir="$runner_temp/nsis-3.12"'
        version_check = '"$nsis_dir/makensis.exe" /VERSION'
        fragments = (
            download,
            checksum,
            digest_check,
            extraction,
            extracted_root,
            version_check,
        )
        for fragment in fragments:
            self.assertIn(fragment, script)
        self.assertEqual(
            [script.index(fragment) for fragment in fragments],
            sorted(script.index(fragment) for fragment in fragments),
        )
        mismatch_branch = script[
            script.index(digest_check) : script.index(extraction)
        ]
        self.assertIn("exit 1", mismatch_branch)
        self.assertIn("expected NSIS v3.12", script)
        self.assertIn(
            'echo "$(cygpath -w "$nsis_dir")" >> "$GITHUB_PATH"', script
        )
        self.assertNotIn("choco", script.lower())

    def test_workspace_versions_are_read_from_toml(self):
        version_read = (
            'tomllib.load(open("Cargo.toml", "rb"))'
            '["workspace"]["package"]["version"]'
        )
        for workflow in (
            RELEASE_PREPARE_WORKFLOW,
            RELEASE_WORKFLOW,
            RELEASE_PREBUILD_WORKFLOW,
            RELEASE_BUILD_WORKFLOW,
        ):
            with self.subTest(workflow=workflow.name):
                source = workflow.read_text(encoding="utf-8")
                self.assertEqual(source.count(version_read), 1)
                self.assertNotIn(r"^\[workspace\.package\]$", source)

    def test_universal_release_verifies_every_shipped_binary(self):
        script = step_script(
            RELEASE_BUILD_WORKFLOW, "Check the macOS build is universal"
        )

        self.assertIn("for binary in clonk-app clonk-game c4group", script)


if __name__ == "__main__":
    unittest.main()
