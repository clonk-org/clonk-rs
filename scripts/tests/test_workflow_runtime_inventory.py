"""Static guards keeping CI and release checks aligned with shipped binaries."""

import re
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
RELEASE_PLATFORM_WORKFLOW = (
    REPOSITORY / ".github" / "workflows" / "release-platform.yml"
)
DEVICE_LOSS_WORKFLOW = (
    REPOSITORY / ".github" / "workflows" / "device-loss-qualification.yml"
)
DEPENDENCY_LICENSES_WORKFLOW = (
    REPOSITORY / ".github" / "workflows" / "dependency-licenses.yml"
)
MSVC_RUNTIME_CONFIG = REPOSITORY / "scripts" / "configure-msvc-runtime.sh"
MSVC_RUNTIME_VALIDATION = REPOSITORY / "scripts" / "validate-msvc-runtime.sh"
WINDOWS_INSTALLER = REPOSITORY / "scripts" / "windows-installer.nsi"
NSIS_INSTALLER = REPOSITORY / "scripts" / "install-nsis.sh"
WORKSPACE_CACHE_ACTION = REPOSITORY / ".github/actions/workspace-cache/action.yml"


def workflow_jobs(workflow):
    source = workflow.read_text(encoding="utf-8").split("\njobs:\n", 1)[1]
    matches = list(re.finditer(r"(?m)^  ([a-zA-Z0-9_-]+):$", source))
    return {
        match[1]: source[match.start():matches[index + 1].start() if index + 1 < len(matches) else len(source)]
        for index, match in enumerate(matches)
    }


def step_blocks(source, indentation=6):
    matches = list(re.finditer(rf"(?m)^{' ' * indentation}- ", source))
    return [
        source[match.start():matches[index + 1].start() if index + 1 < len(matches) else len(source)]
        for index, match in enumerate(matches)
    ]


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

    def test_checkouts_are_pinned_and_read_only_jobs_do_not_persist_credentials(self):
        marker = "uses: actions/checkout@"
        pin = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"

        for workflow in (
            LANDING_WORKFLOW,
            MAIN_WORKFLOW,
            EXACT_SHA_QUALIFICATION_WORKFLOW,
            RELEASE_WORKFLOW,
            RELEASE_BUILD_WORKFLOW,
            RELEASE_PREBUILD_WORKFLOW,
            RELEASE_PREPARE_WORKFLOW,
            DEVICE_LOSS_WORKFLOW,
            # This one pushes, and still checks out without credentials: it
            # runs the branch's own generator, so the token reaches the working
            # tree only in the step that publishes the result.
            DEPENDENCY_LICENSES_WORKFLOW,
        ):
            source = workflow.read_text(encoding="utf-8")
            blocks = [(name, step) for name, job in workflow_jobs(workflow).items()
                      for step in step_blocks(job) if marker in step]
            self.assertTrue(blocks, workflow.name)
            for index, (name, block) in enumerate(blocks, start=1):
                with self.subTest(workflow=workflow.name, checkout=index):
                    self.assertIn(pin, block)
                    if workflow == RELEASE_PREPARE_WORKFLOW and name == "prepare":
                        # The automation-owned preparation writer uses its
                        # restricted App token to push only release/next.
                        self.assertIn("token: ${{ steps.release-app.outputs.token }}", block)
                        self.assertIn("ref: ${{ github.sha }}", block)
                    else:
                        self.assertIn("persist-credentials: false", block)

    def test_landing_workflow_uses_current_pinned_actions_and_nextest(self):
        workflow = LANDING_WORKFLOW.read_text(encoding="utf-8")
        checkout = (
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
            " # v7.0.1"
        )

        checkouts = [step for job in workflow_jobs(LANDING_WORKFLOW).values()
                     for step in step_blocks(job) if "uses: actions/checkout@" in step]
        self.assertTrue(checkouts)
        for block in checkouts:
            self.assertIn(checkout, block)
            self.assertIn("persist-credentials: false", block)
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
        workflow = LANDING_WORKFLOW.read_text(encoding="utf-8")
        windows = workflow[
            workflow.index("  windows-smoke:") : workflow.index("  landing-gate:")
        ]

        self.assertGreaterEqual(windows.count("-p clonk-c4group"), 2)
        self.assertIn(
            '$payload_dir/bin/c4group.exe',
            step_script(
                LANDING_WORKFLOW,
                "Compile the installer over a stand-in payload",
            ),
        )

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
            "release: 1.98.1",
            "LLVM version: 22.1.8",
            "cargo_target=x86_64-pc-windows-msvc",
            'CARGO_BUILD_TARGET=$cargo_target',
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
            "expected_toolchain=1.98.1-x86_64-pc-windows-msvc",
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

    def test_explicit_caches_are_published_only_by_trusted_main(self):
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

        # Every queue cache is restore-only, whether the implementation is a
        # registry action or the source-aware compiled-input composite.
        self.assertNotIn(save, landing)
        queue_caches = [step for job in workflow_jobs(LANDING_WORKFLOW).values()
                        for step in step_blocks(job)
                        if "uses: Swatinem/rust-cache@" in step
                        or "uses: ./.github/actions/workspace-cache" in step]
        self.assertTrue(queue_caches)
        for cache in queue_caches:
            if "uses: Swatinem/rust-cache@" in cache:
                self.assertIn("save-if: false", cache)
            else:
                self.assertIn("operation: restore", cache)
                self.assertNotIn("operation: save", cache)
        for job in workflow_jobs(RELEASE_PREBUILD_WORKFLOW).values():
            for cache in step_blocks(job):
                if "uses: Swatinem/rust-cache@" in cache:
                    self.assertIn("save-if: false", cache)
        self.assertNotIn(save, release_prebuild)

        producer = workflow_jobs(MAIN_WORKFLOW)["linux-landing-cache"]
        composed_saves = [step for job in workflow_jobs(MAIN_WORKFLOW).values()
                          for step in step_blocks(job)
                          if "uses: ./.github/actions/workspace-cache" in step
                          and "operation: save" in step]
        self.assertEqual(len(composed_saves), 1)
        self.assertIn(composed_saves[0], producer)
        self.assertIn("lane: landing-linux", composed_saves[0])
        self.assertIn("ledger: .ci-cache-ledgers/landing.json", composed_saves[0])
        self.assertLess(producer.index("scripts/ci-workspace-cache.py record"),
                        producer.index(composed_saves[0]))
        cache_action = WORKSPACE_CACHE_ACTION.read_text(encoding="utf-8")
        for guard in (
            'os.environ.get("GITHUB_REF") != "refs/heads/main"',
            'event_name not in ("push", "workflow_dispatch", "schedule")',
            'repository.get("fork") is not False',
            'event.get("after") != head',
            "inputs.operation == 'save' && steps.identity.outputs.trusted-save == 'true'",
        ):
            self.assertIn(guard, cache_action)
        self.assertIn(save, cache_action)
        self.assertIn(restore, cache_action)
        bootstrap = workflow_jobs(MAIN_WORKFLOW)["verify-landing-cache-bootstrap"]
        self.assertIn("operation: lookup", bootstrap)
        self.assertIn('[[ "$CACHE_HIT" == "true" ]]', bootstrap)
        thinlto_start = release_prebuild.index("Restore trusted-main ThinLTO cache")
        thinlto_end = release_prebuild.index("\n      - name:", thinlto_start)
        thinlto_step = release_prebuild[thinlto_start:thinlto_end]
        self.assertEqual(thinlto_step.count(restore), 1)
        self.assertNotIn("Publish trusted ThinLTO cache", release_prebuild)
        self.assertNotIn(
            "key: ${{ steps.thinlto-cache.outputs.cache-primary-key }}",
            release_prebuild,
        )

        trusted_save = next(step for step in step_blocks(
            workflow_jobs(MAIN_WORKFLOW)["windows-release-tools"]
        ) if "name: Publish trusted ThinLTO cache" in step)
        for guard in (
            "github.event_name == 'push'",
            "github.event_name == 'workflow_dispatch'",
            "github.ref == 'refs/heads/main'",
            "steps.thinlto-cache.outputs.cache-hit != 'true'",
        ):
            self.assertIn(guard, trusted_save)

        production_key = (
            "clonk-msvc-thinlto-v2-windows-x64-rustc-1.98.1-llvm-22.1.8-${{ "
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
            "cargo build --profile test --locked -p xtask --features engine-tools "
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
            "uses: ./.github/workflows/release-platform.yml",
            "source-sha: ${{ github.sha }}",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, landing)
        platform = workflow_jobs(RELEASE_PLATFORM_WORKFLOW)
        self.assertIn("uses: ./.github/workflows/release-prebuild.yml", platform["prebuild"])
        self.assertIn("needs: prebuild", platform["package"])
        self.assertIn("uses: ./.github/workflows/release-build.yml", platform["package"])
        for role in ("prebuild", "package"):
            self.assertIn("source-sha: ${{ inputs.source-sha }}", platform[role])
            self.assertIn("platform: ${{ inputs.platform }}", platform[role])

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
                self.assertIn(version_read, source)
                self.assertNotIn(r"^\[workspace\.package\]$", source)
        preparation = step_script(RELEASE_PREPARE_WORKFLOW, "Prepare the release")
        self.assertIn(f"current = {version_read}", preparation)
        self.assertIn(f"print({version_read})", preparation)
        self.assertIn("existing release version {original} is no longer newer than main {current}", preparation)

    def test_universal_release_verifies_every_shipped_binary(self):
        script = step_script(
            RELEASE_BUILD_WORKFLOW, "Check the macOS build is universal"
        )

        self.assertIn("for binary in clonk-app clonk-game c4group", script)


if __name__ == "__main__":
    unittest.main()
