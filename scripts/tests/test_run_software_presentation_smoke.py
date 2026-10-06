import importlib.util
import configparser
import io
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from PIL import Image


SCRIPT = Path(__file__).resolve().parents[1] / "run_software_presentation_smoke.py"
SPEC = importlib.util.spec_from_file_location("software_presentation_smoke", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
REAL_RUN = subprocess.run
REAL_CHECK_OUTPUT = subprocess.check_output


def presented_report():
    return {
        "schema_version": MODULE.SCHEMA_VERSION,
        "kind": MODULE.REPORT_KIND,
        "success": True,
        "failure": None,
        "initial_extent": [800, 600],
        "resized_extent": [760, 560],
        "presented_before_resize": True,
        "presented_after_resize": True,
        "phases": [
            {"name": name, "presented": True, "scale": scale,
             "drawable_extent": [760 * scale, 560 * scale],
             "clip_rect": [0, 0, 760 * scale, 560 * scale]}
            for name, scale in (("windowed", 1), ("fullscreen", 2), ("windowed-again", 1))
        ],
        "registry_empty_at_exit": True,
        "software_reason": "forced",
        "gpu_attempt_backends": [],
        "display_backend": "windows",
    }


def write_captures(report_path):
    for suffix, size in ((".screenshot.png", (760, 560)), (".thumbnail.png", (200, 150))):
        Image.new("RGBA", size, (0x6f, 0x2f, 0xa8, 255)).save(report_path.with_suffix(suffix))


class SoftwarePresentationRunnerTests(unittest.TestCase):
    def test_native_screenshots_stay_inside_the_artifact_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifacts = Path(temporary)
            binary = artifacts / "clonk-app.exe"
            binary.touch()

            def execute(command, **_options):
                config = configparser.ConfigParser()
                config.read(command[command.index("--config") + 1])
                folder = config.get("General", "ScreenshotFolder", fallback="Screenshots")
                destination = (MODULE.REPOSITORY / folder).resolve()
                self.assertEqual(destination, (artifacts / "screenshots").resolve())
                self.assertTrue(destination.is_dir())
                raise RuntimeError("checked native screenshot destination")

            with (
                mock.patch.object(MODULE, "refuse_to_run_as_root"),
                mock.patch.object(MODULE, "build_binary", return_value=binary),
                mock.patch.object(MODULE, "source_identity", return_value={}),
                mock.patch.object(MODULE.subprocess, "run", side_effect=execute),
                self.assertRaisesRegex(RuntimeError, "checked native screenshot destination"),
            ):
                MODULE.main(["--artifact-dir", str(artifacts), "--no-xvfb"])

    def test_windows_release_preserves_the_shipped_package_graph(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "release" / "clonk-app.exe"
            binary.parent.mkdir()
            binary.touch()
            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.dict(MODULE.os.environ, {"CARGO_TARGET_DIR": temporary}, clear=True),
                mock.patch.object(MODULE.subprocess, "run") as run,
            ):
                self.assertEqual(MODULE.build_binary(MODULE.REPOSITORY, release=True), binary)
            command = run.call_args.args[0]
            self.assertIn("clonk-game", command)
            self.assertIn("clonk-c4group", command)
            self.assertNotIn("--bin", command)

    def test_windows_dispatch_qualifies_both_modes_on_the_shipped_runtime(self):
        workflow = (SCRIPT.parent.parent / ".github/workflows/rust.yml").read_text()
        self.assertIn("      software_presentation:\n", workflow)
        admission = workflow[workflow.index("  diagnostic-admission:"):workflow.index("  exact-sha-qualification:")]
        self.assertIn("!inputs.software_presentation", admission)
        job = workflow[workflow.index("  windows-release-tools:"):]
        packaging = job[job.index("      - name: Build the Windows packaging tool"):job.index("      - name: Configure the shipped MSVC runtime")]
        self.assertIn("if: ${{ !inputs.software_presentation && needs.qualification-reuse.outputs.run-id == '' }}", packaging)
        validation = job.index("run: scripts/validate-msvc-runtime.sh")
        smoke = job.index("python scripts/run_software_presentation_smoke.py")
        self.assertLess(validation, smoke)
        self.assertIn("uses: ./.github/actions/verified-content", job)
        content_action = (SCRIPT.parent.parent / ".github/actions/verified-content/action.yml").read_text()
        self.assertIn('python3 scripts/ci-content.py --revision "$CONTENT_REVISION"', content_action)
        self.assertIn("--release --check-input", job)
        self.assertIn("--automatic-fallback", job)
        self.assertIn("name: windows-software-presentation", job)
        self.assertIn("always() && inputs.software_presentation", job)

    def test_source_changes_during_a_run_cannot_produce_qualification(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifacts = Path(temporary)
            binary = artifacts / "clonk-app.exe"
            binary.touch()

            def execute(command, **_options):
                report = Path(command[command.index("--software-present-smoke") + 1])
                report.write_text(json.dumps(presented_report()))
                write_captures(report)
                return SimpleNamespace(returncode=0)

            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.object(MODULE, "refuse_to_run_as_root"),
                mock.patch.object(MODULE, "build_binary", return_value=binary),
                mock.patch.object(MODULE, "source_identity", create=True,
                                  side_effect=[{"commit": "before"}, {"commit": "after"}]),
                mock.patch.object(MODULE.subprocess, "run", side_effect=execute),
                self.assertRaisesRegex(SystemExit, "source changed"),
            ):
                MODULE.main(["--artifact-dir", str(artifacts), "--no-xvfb"])

    def test_windows_qualification_rejects_another_window_backend(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            report = presented_report()
            report["display_backend"] = "appkit"
            path.write_text(json.dumps(report))
            write_captures(path)
            with self.assertRaisesRegex(SystemExit, "window backend"):
                MODULE.check_report(path, expected_backend="windows")

    def test_report_rejects_incorrect_thumbnail_pixels(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            path.write_text(json.dumps(presented_report()))
            write_captures(path)
            Image.new("RGBA", (200, 150), (0, 0, 0, 255)).save(path.with_suffix(".thumbnail.png"))
            with self.assertRaisesRegex(SystemExit, "thumbnail"):
                MODULE.check_report(path)

    def test_fallback_requires_failed_gpu_attempts_without_backend_widening(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            report = presented_report()
            for reason, attempts in (
                ("no-adapter", []),
                ("no-adapter", [["vulkan"]]),
                ("forced", [[], []]),
            ):
                with self.subTest(reason=reason, attempts=attempts):
                    report["software_reason"] = reason
                    report["gpu_attempt_backends"] = attempts
                    path.write_text(json.dumps(report))
                    with self.assertRaisesRegex(SystemExit, "fallback"):
                        MODULE.check_report(path, automatic_fallback=True)
            report["software_reason"] = "no-adapter"
            report["gpu_attempt_backends"] = [[], []]
            path.write_text(json.dumps(report))
            write_captures(path)
            MODULE.check_report(path, automatic_fallback=True)

    def test_input_qualification_is_opt_in(self):
        self.assertTrue(MODULE.parse_arguments(["--check-input"]).check_input)
        self.assertFalse(MODULE.parse_arguments([]).check_input)

    def test_report_requires_an_observed_scaled_pointer_event(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            report.write_text(json.dumps(presented_report()))
            with self.assertRaisesRegex(SystemExit, "pointer"):
                MODULE.check_report(report, check_input=True)

    def test_windows_build_uses_the_configured_release_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary)
            triple = "x86_64-pc-windows-msvc"
            binary = target / triple / "release" / "clonk-app.exe"
            binary.parent.mkdir(parents=True)
            binary.touch()
            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.dict(MODULE.os.environ, {
                    "CARGO_TARGET_DIR": str(target), "CARGO_BUILD_TARGET": triple,
                }, clear=True),
                mock.patch.object(MODULE.subprocess, "run"),
            ):
                self.assertEqual(MODULE.build_binary(MODULE.REPOSITORY, True), binary)

    def test_windows_build_resolves_the_release_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary)
            binary = target / "release" / "clonk-app.exe"
            binary.parent.mkdir()
            binary.touch()
            with (
                mock.patch.object(MODULE.sys, "platform", "win32"),
                mock.patch.dict(MODULE.os.environ, {"CARGO_TARGET_DIR": str(target)}, clear=True),
                mock.patch.object(MODULE.subprocess, "run") as run,
            ):
                self.assertEqual(MODULE.build_binary(MODULE.REPOSITORY, True), binary)
            self.assertIn("--release", run.call_args.args[0])


class PrebuiltSoftwarePresentationRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.sandbox = Path(self.temporary.name)
        self.root = self.sandbox / "repository"
        self.root.mkdir()
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3"\n')
        for name in (
            "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
            "scripts/configure-msvc-runtime.sh", "scripts/ci-content.py", "scripts/ci-workspace-cache.py",
            ".github/workflows/release-prebuild.yml", ".github/workflows/release-build.yml",
            ".github/workflows/device-loss-qualification.yml", ".github/workflows/release-platform.yml",
            ".github/actions/device-loss/action.yml", ".github/actions/verify-cache-handoff/action.yml",
        ):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"fixture recipe input: {name}\n")
        (self.root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.1"\n')
        shutil.copyfile(
            SCRIPT.parent / "release-prebuild-manifest.py",
            self.root / "scripts/release-prebuild-manifest.py",
        )
        manifest_spec = importlib.util.spec_from_file_location(
            "probe_fixture_manifest", self.root / "scripts/release-prebuild-manifest.py",
        )
        self.manifest_module = importlib.util.module_from_spec(manifest_spec)
        manifest_spec.loader.exec_module(self.manifest_module)
        self.git("init", "-q")
        self.git("add", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo", "scripts", ".github")
        self.git("update-index", "--add", "--cacheinfo", f"160000,{'5' * 40},content")
        self.git("-c", "user.name=Probe Tests", "-c", "user.email=probe@example.invalid",
                 "commit", "-q", "-m", "fixture")
        self.prebuilt_root = self.sandbox / "prebuilt"
        self.prebuilt_root.mkdir()
        (self.prebuilt_root / "payload").mkdir()
        self.toolchain = (b"rustc 1.98.1 fixture\nhost: x86_64-unknown-linux-gnu\n"
                          b"release: 1.98.1\nLLVM version: 22.1.8\n")
        self.run_id = "123456"
        self.run_attempt = "2"

    def git(self, *arguments):
        return subprocess.check_output(["git", "-C", str(self.root), *arguments], text=True).strip()

    def write_manifest(self, target="x86_64-unknown-linux-gnu", *, provenance=True, parents=()):
        if provenance and target == "universal-apple-darwin" and not parents:
            final_root = self.prebuilt_root
            parents = []
            for parent_target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                self.prebuilt_root = self.sandbox / parent_target
                self.prebuilt_root.mkdir()
                (self.prebuilt_root / "payload").mkdir()
                self.write_manifest(parent_target)
                parents.append(self.prebuilt_root / "manifest.json")
            self.prebuilt_root = final_root
        suffix = ".exe" if "windows" in target else ""
        files = [f"payload/{name}{suffix}" for name in ("c4group", "clonk-app", "clonk-game")]
        for name in files:
            binary = self.prebuilt_root / name
            binary.write_bytes(b"packaged runtime\n")
            binary.chmod(0o755)
        command = [
            sys.executable, str(self.root / "scripts/release-prebuild-manifest.py"), "write",
            "--root", str(self.prebuilt_root),
            "--manifest", str(self.prebuilt_root / "manifest.json"),
            "--head-sha", self.git("rev-parse", "HEAD"),
            "--tree-sha", self.git("rev-parse", "HEAD^{tree}"),
            "--version", "1.2.3", "--kind", "runtime", "--target", target,
        ]
        if provenance:
            command.extend(("--provenance-root", str(self.root)))
            if parents:
                for path in parents:
                    command.extend(("--parent-manifest", str(path)))
            else:
                command.extend(("--build-profile", "release", "--build-target", target))
        for name in files:
            command.extend(("--file", name))
        environment = {}
        if target == "x86_64-pc-windows-msvc" and not parents:
            environment = {
                "CARGO_BUILD_TARGET": target, "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER": "/fixture/rust-lld.exe",
                "CFLAGS_x86_64_pc_windows_msvc": "-I/fixture/zlib",
                "CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc": "/fixture/native.cmake", "LINK": "", "_LINK_": "",
                "RUSTFLAGS": ("-Ctarget-feature=+crt-static -Clinker-plugin-lto -Clinker-flavor=lld-link "
                              "-Clink-arg=/lldltocache:C:/fixture/cache "
                              "-Clink-arg=/lldltocachepolicy:cache_size=0%:cache_size_bytes=512m "
                              "-Clink-arg=/DEBUG:NONE -Clink-arg=/OPT:REF,ICF -Clink-arg=/TIME -Clink-arg=/Brepro"),
            }
        with mock.patch.dict(MODULE.os.environ, environment):
            self.run_without_cargo(command, check=True, capture_output=True, text=True)

    def run_without_cargo(self, command, **options):
        self.assertNotEqual(command[0], "cargo", "a verified prebuilt runtime must not rebuild")
        if command[:2] == [sys.executable, str(self.root / "scripts/release-prebuild-manifest.py")]:
            def output(arguments, **settings):
                if arguments == ["rustc", "-vV"]:
                    return self.toolchain.decode() if settings.get("text") else self.toolchain
                if arguments[0] == "/fixture/rust-lld.exe":
                    return b"LLD 22.1.8 (compatible with MSVC link.exe)\n"
                return REAL_CHECK_OUTPUT(arguments, **settings)

            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                mock.patch.dict(MODULE.os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_RUN_ID": self.run_id,
                                                     "GITHUB_RUN_ATTEMPT": self.run_attempt}),
                mock.patch.object(self.manifest_module.subprocess, "check_output", side_effect=output),
                redirect_stdout(stdout), redirect_stderr(stderr),
            ):
                returncode = self.manifest_module.main(command[2:])
            if options.get("check") and returncode:
                raise subprocess.CalledProcessError(returncode, command, stdout.getvalue(), stderr.getvalue())
            return subprocess.CompletedProcess(command, returncode, stdout.getvalue(), stderr.getvalue())
        return REAL_RUN(command, **options)

    def qualify(self, *, operating_system="linux", mutate_manifest=False):
        artifacts = self.sandbox / "reports"
        source = {"commit": self.git("rev-parse", "HEAD"), "content_commit": "5" * 40,
                  "sources_sha256": "6" * 64, "source_dirty": False}

        def output(command, **options):
            self.assertNotEqual(command[0], "cargo")
            if command == ["rustc", "-vV"]:
                return self.toolchain.decode() if options.get("text") else self.toolchain
            return REAL_CHECK_OUTPUT(command, **options)

        def execute(command, **options):
            self.assertNotEqual(command[0], "cargo")
            if command[0] == "git":
                return REAL_RUN(command, **options)
            if command[:2] == [sys.executable, str(self.root / "scripts/release-prebuild-manifest.py")]:
                return self.run_without_cargo(command, **options)
            self.assertEqual(command[0], str(self.prebuilt_root / "payload/clonk-app"))
            path = Path(command[command.index("--software-present-smoke") + 1])
            report = presented_report()
            report["display_backend"] = "appkit" if operating_system == "darwin" else "x11"
            path.write_text(json.dumps(report))
            write_captures(path)
            if mutate_manifest:
                manifest = self.prebuilt_root / "manifest.json"
                manifest.write_bytes(manifest.read_bytes() + b"\n")
            return SimpleNamespace(returncode=0)

        with (
            mock.patch.object(MODULE, "REPOSITORY", self.root),
            mock.patch.object(MODULE, "refuse_to_run_as_root"),
            mock.patch.object(MODULE, "source_identity", return_value=source),
            mock.patch.object(MODULE.sys, "platform", operating_system),
            mock.patch.object(MODULE.platform, "platform", return_value="Fixture OS"),
            mock.patch.object(MODULE.platform, "version", return_value="Fixture version"),
            mock.patch.object(MODULE.platform, "machine", return_value="arm64" if operating_system == "darwin" else "x86_64"),
            mock.patch.object(MODULE.subprocess, "check_output", side_effect=output),
            mock.patch.object(MODULE.subprocess, "run", side_effect=execute),
            redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(MODULE.main([
                "--release", "--prebuilt-root", str(self.prebuilt_root),
                "--artifact-dir", str(artifacts), "--no-xvfb",
            ]), 0)
        return json.loads((artifacts / "qualification.json").read_text())

    def test_prebuilt_qualification_records_producer_flags_separately_from_consumer_observations(self):
        with mock.patch.dict(MODULE.os.environ, {"RUSTFLAGS": "-Cdebuginfo=1"}):
            self.write_manifest()
        with mock.patch.dict(MODULE.os.environ, {"RUSTFLAGS": "-Cdebuginfo=0"}):
            evidence = self.qualify()
        self.assertEqual(evidence["build_origin"], "prebuilt")
        self.assertEqual(evidence["rustflags"], "-Cdebuginfo=1")
        self.assertEqual(evidence["build_target"], "x86_64-unknown-linux-gnu")
        self.assertEqual(evidence["qualification_environment"]["rustflags"], "-Cdebuginfo=0")
        self.assertEqual(evidence["prebuilt_manifest"]["schema"], 3)
        self.assertEqual(evidence["prebuilt_manifest"]["provenance"]["builder"],
                         {"run_id": "123456", "run_attempt": 2})
        self.assertEqual(evidence["prebuilt_manifest_sha256"], MODULE.file_digest(self.prebuilt_root / "manifest.json"))

    def test_prebuilt_manifest_changes_during_qualification_cannot_produce_evidence(self):
        self.write_manifest()
        with self.assertRaisesRegex(SystemExit, "manifest changed"):
            self.qualify(mutate_manifest=True)
        self.assertFalse((self.sandbox / "reports/qualification.json").exists())

    def test_prebuilt_runtime_is_verified_without_a_cargo_build(self):
        self.write_manifest()
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo) as run,
        ):
            self.assertEqual(
                MODULE.build_binary(self.root, release=True, prebuilt_root=self.prebuilt_root),
                self.prebuilt_root / "payload/clonk-app",
            )
        commands = [call.args[0] for call in run.call_args_list if call.args[0][:3] == [
            sys.executable, str(self.root / "scripts/release-prebuild-manifest.py"), "verify",
        ]]
        self.assertEqual(len(commands), 1)
        self.assertEqual(commands[0][commands[0].index("--provenance-root") + 1], str(self.root))

    def test_prebuilt_runtime_requires_production_provenance_even_for_intact_legacy_payloads(self):
        self.write_manifest(provenance=False)
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
            self.assertRaisesRegex(SystemExit, "schema must be 3"),
        ):
            MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_rejects_provenance_and_producer_mismatches(self):
        self.write_manifest()
        manifest = self.prebuilt_root / "manifest.json"
        original = manifest.read_text()
        for field, value, diagnosis in (
            ("recipe_sha", "0" * 64, "recipe_sha"),
            ("toolchain_sha", "1" * 64, "toolchain_sha"),
            ("content_sha", "2" * 40, "content_sha"),
            ("recipe_sha", None, "unexpected or missing fields"),
            ("unknown_context", "unsupported", "unexpected or missing fields"),
            ("builder", {"run_id": "654321", "run_attempt": 2}, "producer run ID"),
            ("builder", {"run_id": "123456", "run_attempt": 3}, "producer attempt"),
        ):
            with self.subTest(field=field, value=value):
                document = json.loads(original)
                if value is None:
                    del document["provenance"][field]
                else:
                    document["provenance"][field] = value
                manifest.write_text(json.dumps(document))
                with (
                    mock.patch.object(MODULE.sys, "platform", "linux"),
                    mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
                    mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
                    self.assertRaisesRegex(SystemExit, diagnosis),
                ):
                    MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_uses_the_current_compiler_and_workflow_retry_context(self):
        self.write_manifest()
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
        ):
            original = self.toolchain
            self.toolchain += b"LLVM version: changed\n"
            with self.assertRaisesRegex(SystemExit, "toolchain_sha"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)
            self.toolchain = original
            self.run_id = "654321"
            with self.assertRaisesRegex(SystemExit, "producer run ID"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)
            self.run_id = "123456"
            self.run_attempt = "3"
            self.assertEqual(
                MODULE.build_binary(self.root, True, self.prebuilt_root),
                self.prebuilt_root / "payload/clonk-app",
            )

    def test_prebuilt_root_option_reaches_the_binary_verifier(self):
        self.write_manifest()
        artifacts = self.sandbox / "reports"
        with (
            mock.patch.object(MODULE, "refuse_to_run_as_root"),
            mock.patch.object(MODULE, "source_identity", return_value={}),
            mock.patch.object(MODULE, "build_binary", side_effect=RuntimeError("binary selected")) as build,
            self.assertRaisesRegex(RuntimeError, "binary selected"),
        ):
            MODULE.main([
                "--release", "--prebuilt-root", str(self.prebuilt_root),
                "--artifact-dir", str(artifacts),
            ])
        build.assert_called_once_with(MODULE.REPOSITORY, True, self.prebuilt_root)

    def test_prebuilt_runtime_rejects_another_source_version_or_artifact_kind(self):
        self.write_manifest()
        manifest = self.prebuilt_root / "manifest.json"
        original = manifest.read_text()
        for field, value in (
            ("head_sha", "3" * 40), ("tree_sha", "4" * 40),
            ("version", "1.2.4"), ("kind", "tool"),
        ):
            with self.subTest(field=field):
                document = json.loads(original)
                document[field] = value
                manifest.write_text(json.dumps(document))
                with (
                    mock.patch.object(MODULE.sys, "platform", "linux"),
                    mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
                    mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
                    self.assertRaisesRegex(SystemExit, field.replace("_", "-")),
                ):
                    MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_rejects_same_size_binary_tampering(self):
        self.write_manifest()
        (self.prebuilt_root / "payload/clonk-app").write_bytes(b"tampered runtime\n")
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
            self.assertRaisesRegex(SystemExit, "sha256 mismatch: payload/clonk-app"),
        ):
            MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_requires_the_exact_shipped_payload_file_set(self):
        self.write_manifest()
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
        ):
            game = self.prebuilt_root / "payload/clonk-game"
            original = game.read_bytes()
            game.unlink()
            with self.assertRaisesRegex(SystemExit, "missing: payload/clonk-game"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)
            game.write_bytes(original)
            (self.prebuilt_root / "payload/undeclared").write_bytes(b"extra")
            with self.assertRaisesRegex(SystemExit, "extra: payload/undeclared"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_rejects_missing_malformed_or_unsupported_manifests(self):
        self.write_manifest()
        manifest = self.prebuilt_root / "manifest.json"
        original = manifest.read_text()
        manifest.unlink()
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
        ):
            with self.assertRaisesRegex(SystemExit, "cannot read prebuilt runtime identity"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)
            manifest.write_text("invalid json")
            with self.assertRaisesRegex(SystemExit, "cannot read prebuilt runtime identity"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)
            document = json.loads(original)
            document["schema"] = 0
            manifest.write_text(json.dumps(document))
            with self.assertRaisesRegex(SystemExit, "manifest schema"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_rejects_modified_or_untracked_source_inputs(self):
        self.write_manifest()
        tracked = self.root / "scripts/release-prebuild-manifest.py"
        original = tracked.read_bytes()
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
        ):
            tracked.write_bytes(original + b"\n# source changed\n")
            with self.assertRaisesRegex(SystemExit, "requires committed source inputs"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)
            tracked.write_bytes(original)
            (self.root / "new_source.rs").write_text("// untracked source\n")
            with self.assertRaisesRegex(SystemExit, "requires committed source inputs"):
                MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_requires_release_mode(self):
        self.write_manifest()
        with (
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
            self.assertRaisesRegex(SystemExit, "requires --release"),
        ):
            MODULE.build_binary(self.root, False, self.prebuilt_root)

    def test_prebuilt_runtime_rejects_cross_platform_and_cross_architecture_targets(self):
        self.write_manifest()
        manifest = self.prebuilt_root / "manifest.json"
        original = manifest.read_text()
        for operating_system, architecture, target in (
            ("linux", "x86_64", "universal-apple-darwin"),
            ("linux", "x86_64", "aarch64-unknown-linux-gnu"),
            ("win32", "AMD64", "x86_64-pc-windows-gnu"),
            ("darwin", "arm64", "x86_64-apple-darwin"),
            ("linux", "x86_64", "host"),
        ):
            with self.subTest(platform=operating_system, architecture=architecture, target=target):
                document = json.loads(original)
                document["target"] = target
                manifest.write_text(json.dumps(document))
                with (
                    mock.patch.object(MODULE.sys, "platform", operating_system),
                    mock.patch.object(MODULE.platform, "machine", return_value=architecture),
                    mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
                    self.assertRaisesRegex(SystemExit, "cannot run on"),
                ):
                    MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_rejects_platforms_without_a_shipped_runtime(self):
        self.write_manifest()
        for operating_system, architecture in (
            ("linux", "aarch64"), ("win32", "ARM64"), ("freebsd", "x86_64"),
        ):
            with (
                self.subTest(platform=operating_system, architecture=architecture),
                mock.patch.object(MODULE.sys, "platform", operating_system),
                mock.patch.object(MODULE.platform, "machine", return_value=architecture),
                mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
                self.assertRaisesRegex(SystemExit, "no shipped prebuilt runtime"),
            ):
                MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_runtime_accepts_the_native_windows_executable(self):
        self.write_manifest("x86_64-pc-windows-msvc")
        with (
            mock.patch.object(MODULE.sys, "platform", "win32"),
            mock.patch.object(MODULE.platform, "machine", return_value="AMD64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
        ):
            self.assertEqual(
                MODULE.build_binary(self.root, True, self.prebuilt_root),
                self.prebuilt_root / "payload/clonk-app.exe",
            )

    def test_prebuilt_runtime_accepts_the_fused_macos_runtime_on_both_native_architectures(self):
        self.write_manifest("universal-apple-darwin")
        for architecture in ("arm64", "x86_64"):
            with (
                self.subTest(architecture=architecture),
                mock.patch.object(MODULE.sys, "platform", "darwin"),
                mock.patch.object(MODULE.platform, "machine", return_value=architecture),
                mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
            ):
                self.assertEqual(
                    MODULE.build_binary(self.root, True, self.prebuilt_root),
                    self.prebuilt_root / "payload/clonk-app",
                )

    def test_fused_runtime_qualification_preserves_each_parent_build_instead_of_claiming_another_compile(self):
        self.write_manifest("universal-apple-darwin")
        with mock.patch.dict(MODULE.os.environ, {"RUSTFLAGS": "-Cdebuginfo=0"}):
            evidence = self.qualify(operating_system="darwin")
        self.assertEqual(evidence["build_origin"], "prebuilt")
        for field in ("build_profile", "build_target", "rustflags", "encoded_rustflags", "rustc"):
            self.assertIsNone(evidence[field])
        recipe = evidence["prebuilt_manifest"]["provenance"]["producer_recipe"]
        self.assertEqual(recipe["operation"], "transform")
        self.assertEqual(recipe["target"], "universal-apple-darwin")
        self.assertEqual([json.loads(entry["manifest_json"])["provenance"]["producer_recipe"]["target"]
                          for entry in recipe["parents"]], ["aarch64-apple-darwin", "x86_64-apple-darwin"])
        self.assertEqual(evidence["qualification_environment"]["rustflags"], "-Cdebuginfo=0")

    def test_prebuilt_probe_refuses_incomplete_producer_recipes_and_legacy_schema_two(self):
        self.write_manifest()
        manifest = self.prebuilt_root / "manifest.json"
        original = manifest.read_text()
        for mutation, diagnosis in (
            ("missing", "unexpected or missing fields"), ("profile", "profile/features mismatch"),
            ("target", "producer recipe target mismatch"), ("schema", "schema must be 3"),
        ):
            with self.subTest(mutation=mutation):
                document = json.loads(original)
                if mutation == "missing":
                    del document["provenance"]["producer_recipe"]
                elif mutation == "schema":
                    document["schema"] = 2
                else:
                    document["provenance"]["producer_recipe"][mutation] = "unprescribed"
                manifest.write_text(json.dumps(document))
                with (
                    mock.patch.object(MODULE.sys, "platform", "linux"),
                    mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
                    mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
                    self.assertRaisesRegex(SystemExit, diagnosis),
                ):
                    MODULE.build_binary(self.root, True, self.prebuilt_root)

    def test_prebuilt_probe_refuses_a_windows_runtime_with_dynamic_crt_provenance(self):
        self.write_manifest("x86_64-pc-windows-msvc")
        manifest = self.prebuilt_root / "manifest.json"
        document = json.loads(manifest.read_text())
        environment = document["provenance"]["producer_recipe"]["environment"]
        environment["RUSTFLAGS"] = environment["RUSTFLAGS"].replace("+crt-static", "-crt-static")
        manifest.write_text(json.dumps(document))
        with (
            mock.patch.object(MODULE.sys, "platform", "win32"),
            mock.patch.object(MODULE.platform, "machine", return_value="AMD64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
            self.assertRaisesRegex(SystemExit, "static CRT"),
        ):
            MODULE.build_binary(self.root, True, self.prebuilt_root)

    @unittest.skipIf(MODULE.os.name == "nt", "Windows does not have Unix execute permissions")
    def test_verified_unix_runtime_restores_execute_permissions_without_changing_bytes(self):
        self.write_manifest()
        binaries = list((self.prebuilt_root / "payload").iterdir())
        before = {binary: binary.read_bytes() for binary in binaries}
        for binary in binaries:
            binary.chmod(0o644)
        with (
            mock.patch.object(MODULE.sys, "platform", "linux"),
            mock.patch.object(MODULE.platform, "machine", return_value="x86_64"),
            mock.patch.object(MODULE.subprocess, "run", side_effect=self.run_without_cargo),
        ):
            MODULE.build_binary(self.root, True, self.prebuilt_root)
        for binary in binaries:
            self.assertEqual(binary.stat().st_mode & 0o111, 0o111)
            self.assertEqual(binary.read_bytes(), before[binary])


if __name__ == "__main__":
    unittest.main()
