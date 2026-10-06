"""Static guards for verified content and exact release-input handoffs."""

import re
import unittest

from _repo import REPOSITORY


WORKFLOWS = REPOSITORY / ".github" / "workflows"
ACTIONS = REPOSITORY / ".github" / "actions"
ACTION = ACTIONS / "verify-cache-handoff" / "action.yml"
VERIFIED_CONTENT = ACTIONS / "verified-content" / "action.yml"
CACHE_ACTION = "actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9"
UPLOAD_ACTION = "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
DOWNLOAD_ACTION = "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c"


def steps(text):
    """Split a workflow or action into its verbatim step blocks."""
    boundaries = [match.start() for match in re.finditer(r"(?m)^ *- ", text)]
    return [
        text[start : boundaries[index + 1] if index + 1 < len(boundaries) else len(text)]
        for index, start in enumerate(boundaries)
    ]


class CacheHandoffVerificationTests(unittest.TestCase):
    def test_the_handoff_verifier_polls_before_declaring_a_save_lost(self):
        action = ACTION.read_text(encoding="utf-8")

        self.assertIn("using: composite", action)
        for name in ("path", "key"):
            with self.subTest(input_name=name):
                self.assertRegex(
                    action, rf"(?m)^  {name}:\n(?:    .*\n)*?    required: true\n"
                )

        lookups = [step for step in steps(action) if CACHE_ACTION in step]
        self.assertGreaterEqual(len(lookups), 3)
        for lookup in lookups:
            with self.subTest(lookup=lookup):
                self.assertIn("lookup-only: true", lookup)
                self.assertIn("path: ${{ inputs.path }}", lookup)
                self.assertIn("key: ${{ inputs.key }}", lookup)

        # Only the last attempt fails the row; the earlier ones feed the retry.
        self.assertNotIn("fail-on-cache-miss", "".join(lookups[:-1]))
        self.assertIn("fail-on-cache-miss: true", lookups[-1])

        waits = [step for step in steps(action) if "sleep " in step]
        self.assertGreaterEqual(len(waits), 2)
        self.assertGreaterEqual(
            sum(int(match) for match in re.findall(r"sleep (\d+)", action)), 10
        )

        # Every retry is gated on all of its predecessors: a skipped step
        # reports no cache-hit, so a partial condition would retry after a hit.
        attempts = re.findall(r"(?m)^    - name: .*\n(?:      .*\n)*", action)
        seen = 0
        for attempt in attempts:
            condition = re.search(r"if: (?P<condition>.*)", attempt)
            if condition is None:
                seen += 1
                continue
            self.assertEqual(
                condition.group("condition").count("!= 'true'"),
                seen,
                msg=attempt,
            )
            if CACHE_ACTION in attempt:
                seen += 1

    def test_no_workflow_verifies_its_own_save_with_a_single_lookup(self):
        sources = sorted(WORKFLOWS.glob("*.yml")) + sorted(ACTIONS.glob("*/action.yml"))
        for source in sources:
            if source == ACTION:
                continue  # Its final lookup follows the bounded retries above.
            text = source.read_text(encoding="utf-8")
            for step in steps(text):
                if CACHE_ACTION not in step:
                    continue
                with self.subTest(source=str(source.relative_to(REPOSITORY)), step=step):
                    self.assertNotIn(
                        "lookup-only: true",
                        step if "fail-on-cache-miss: true" in step else "",
                    )

    def test_content_publication_hands_verified_objects_to_the_retrying_verifier(self):
        content = VERIFIED_CONTENT.read_text(encoding="utf-8")
        save = next(step for step in steps(content) if "actions/cache/save@" in step)
        verifier = next(
            step
            for step in steps(content)
            if "uses: ./.github/actions/verify-cache-handoff" in step
        )
        materialize = next(step for step in steps(content) if "id: materialize" in step)
        self.assertLess(content.index(materialize), content.index(save))
        self.assertLess(content.index(save), content.index(verifier))
        self.assertNotIn("continue-on-error", materialize)
        self.assertNotIn("continue-on-error", verifier)
        for step in (save, verifier):
            self.assertIn(
                "if: inputs.publish == 'true' && github.ref == 'refs/heads/main' "
                "&& steps.cache.outputs.cache-hit != 'true'",
                step,
            )
            self.assertIn("path: .git/modules/content", step)
            self.assertIn(
                "key: clonk-content-git-v2-${{ hashFiles('.gitmodules') }}-"
                "${{ steps.identity.outputs.revision }}",
                step,
            )
        self.assertNotIn("restore-keys:", content)

        main = (WORKFLOWS / "rust.yml").read_text(encoding="utf-8")
        producer = main[
            main.index("  content-landing-cache:") : main.index("  linux-landing-cache:")
        ]
        self.assertIn("uses: ./.github/actions/verified-content", producer)
        self.assertIn("publish: 'true'", producer)
        self.assertIn("cancel-in-progress: false", producer)

    def test_exhaustive_workspace_bootstrap_verifies_the_same_saved_paths_and_key(self):
        main = (WORKFLOWS / "rust.yml").read_text(encoding="utf-8")
        producer = main[
            main.index("  linux-landing-cache:") : main.index("  diagnostic-admission:")
        ]
        publish = next(
            step for step in steps(producer)
            if "uses: ./.github/actions/workspace-cache" in step and "operation: save" in step
        )
        record = next(
            step for step in steps(producer) if "scripts/ci-workspace-cache.py record " in step
        )
        verify = next(
            step for step in steps(producer)
            if "uses: ./.github/actions/verify-cache-handoff" in step
        )
        self.assertLess(producer.index(record), producer.index(publish))
        self.assertLess(producer.index(publish), producer.index(verify))
        self.assertIn("if: github.event_name == 'workflow_dispatch' && inputs.cache_only", verify)
        self.assertNotIn("continue-on-error", verify)
        identifier = re.search(r"(?m)^        id: (\S+)\n", publish).group(1)
        self.assertIn(f"key: ${{{{ steps.{identifier}.outputs.key }}}}", verify)
        paths = re.search(
            r"(?m)^          path: \|\n((?:            .*\n)+)", verify
        )
        self.assertIsNotNone(paths, "cache version requires both saved paths during lookup")
        # actions/cache versions include the complete path list, even for
        # lookup-only. Omitting the external ledger makes an exact key miss.
        expected = [
            re.search(rf"(?m)^          {name}: (\S+)\n", publish).group(1)
            for name in ("target", "ledger")
        ]
        self.assertEqual(paths.group(1).split(), expected)

    def test_release_artifacts_keep_the_exact_source_and_producing_run_identity(self):
        prebuild = (WORKFLOWS / "release-prebuild.yml").read_text(encoding="utf-8")
        package = (WORKFLOWS / "release-build.yml").read_text(encoding="utf-8")
        uploads = [step for step in steps(prebuild) if UPLOAD_ACTION in step]
        self.assertEqual(len(uploads), 2)  # Packaging tool and native runtime.
        for upload in uploads:
            self.assertIn(
                "name: ${{ matrix.artifact }}-${{ inputs.source-sha }}-${{ github.run_id }}",
                upload,
            )
            self.assertIn("path: target/release-prebuild/${{ matrix.artifact }}", upload)
            self.assertIn("if-no-files-found: error", upload)
            self.assertNotIn("continue-on-error", upload)

        downloads = [step for step in steps(package) if DOWNLOAD_ACTION in step]
        self.assertEqual(len(downloads), 3)  # Tool, runtime, macOS's second target.
        for field in ("tool_artifact", "runtime_artifact", "runtime_artifact_2"):
            download = next(
                step for step in downloads
                if f"path: target/release-prebuild/${{{{ matrix.{field} }}}}\n" in step
            )
            with self.subTest(artifact=field):
                self.assertIn(
                    f"name: ${{{{ matrix.{field} }}}}-${{{{ inputs.source-sha }}}}-${{{{ github.run_id }}}}",
                    download,
                )
                self.assertNotIn("continue-on-error", download)
                self.assertNotIn("pattern:", download)
                self.assertNotIn("run-id:", download)
                self.assertNotIn("restore-keys:", download)
        self.assertNotIn("actions/cache/save@", prebuild)
        self.assertNotIn("key: release-prebuild-", package)

        # Each platform packages only after its own exact inputs finish. The
        # outer platform matrix need not wait for an unrelated platform's build.
        pipeline = (WORKFLOWS / "release-platform.yml").read_text(encoding="utf-8")
        producer = pipeline[pipeline.index("  prebuild:") : pipeline.index("  package:")]
        consumer = pipeline[pipeline.index("  package:") :]
        self.assertIn("uses: ./.github/workflows/release-prebuild.yml", producer)
        self.assertIn("uses: ./.github/workflows/release-build.yml", consumer)
        self.assertIn("needs: prebuild", consumer)
        for argument in ("source-sha", "tree-sha", "version", "platform"):
            for job in (producer, consumer):
                self.assertIn(f"{argument}: ${{{{ inputs.{argument} }}}}", job)

    def test_prebuilt_payload_manifests_are_verified_before_any_input_is_consumed(self):
        prebuild = (WORKFLOWS / "release-prebuild.yml").read_text(encoding="utf-8")
        package = (WORKFLOWS / "release-build.yml").read_text(encoding="utf-8")
        writes = [
            step for step in steps(prebuild)
            if "scripts/release-prebuild-manifest.py write" in step
        ]
        self.assertEqual(len(writes), 2)
        verify = next(
            step for step in steps(package)
            if "- name: Verify and install the prebuilt inputs" in step
        )
        self.assertEqual(verify.count("scripts/release-prebuild-manifest.py verify"), 3)
        self.assertNotIn("continue-on-error", verify)
        for step in writes + [verify]:
            self.assertNotIn("continue-on-error", step)
            for argument in (
                '--provenance-root "$GITHUB_WORKSPACE"',
                '--head-sha "$SOURCE_SHA"',
                '--tree-sha "$TREE_SHA"',
                '--version "$VERSION"',
            ):
                self.assertIn(argument, step)
            for name, source in (
                ("SOURCE_SHA", "source-sha"),
                ("TREE_SHA", "tree-sha"),
                ("VERSION", "version"),
            ):
                self.assertIn(f"{name}: ${{{{ inputs.{source} }}}}", step)

        for kind, name in (("tool", "packaging tool"), ("runtime", "runtime")):
            write = next(step for step in writes if f"--kind {kind}" in step)
            upload = next(
                step for step in steps(prebuild) if f"- name: Hand off the {name}\n" in step
            )
            self.assertLess(prebuild.index(write), prebuild.index(upload))

        self.assertIn("--kind tool", verify)
        self.assertIn("--kind runtime", verify)
        self.assertIn('--file "payload/${{ matrix.tool_filename }}"', verify)
        for root, target in (
            ("tool_root", "host"),
            ("runtime_root", '"${{ matrix.runtime_target }}"'),
            ("runtime_root_2", '"${{ matrix.runtime_target_2 }}"'),
        ):
            command = next(
                command for command in verify.split("scripts/release-prebuild-manifest.py verify")[1:]
                if f'--root "${root}"' in command
            )
            self.assertIn(f'--manifest "${root}/manifest.json"', command)
            self.assertIn(f"--target {target}", command)
            self.assertLess(
                verify.index(f'--root "${root}"'),
                verify.index(f'cp "${root}'),
            )
        for binary in ("c4group", "clonk-app", "clonk-game"):
            self.assertIn(f'--file "payload/{binary}${{{{ matrix.runtime_suffix }}}}"', verify)
            self.assertIn(f"--file payload/{binary}", verify)
        for name in ("Package Linux", "Package Windows installer", "Package macOS"):
            self.assertLess(package.index(verify), package.index(f"- name: {name}"))


if __name__ == "__main__":
    unittest.main()
