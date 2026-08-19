import tomllib
import unittest

from _repo import REPOSITORY

# The crates a dedicated server must not need. `winit` is the window, `wgpu`
# the GPU device, `cpal` the audio output device; a headless run
# (clonk-org/clonk-rs#120) initialises none of them.
PRESENTATION_DEPENDENCIES = ("winit", "wgpu", "cpal")

# Exactly the crates allowed to name one today. This is a ceiling, not a
# target: clonk-org/clonk-rs#391 wants a server build whose graph contains none
# of them, and every crate added here is one more that has to be gated first.
PRESENTATION_CRATES = {
    "clonk-app": {"wgpu", "winit"},
    "clonk-app-render": {"wgpu"},
    "clonk-audio": {"cpal"},
    "clonk-launcher-shell": {"winit"},
    "clonk-surface": {"wgpu"},
}


def _dependency_names(manifest):
    """Every dependency a manifest names, including per-target tables."""
    names = set()
    for table in ("dependencies", "dev-dependencies", "build-dependencies"):
        names.update(manifest.get(table, {}))
    for target in manifest.get("target", {}).values():
        for table in ("dependencies", "dev-dependencies", "build-dependencies"):
            names.update(target.get(table, {}))
    return names


def _crate_manifests():
    for path in sorted((REPOSITORY / "crates").glob("*/Cargo.toml")):
        yield path.parent.name, tomllib.loads(path.read_text(encoding="utf-8"))


class HeadlessDependencyContractTests(unittest.TestCase):
    def test_only_the_presentation_crates_name_a_window_gpu_or_audio_device(self):
        """The simulation half is already server-clean; keep it that way.

        A dedicated server links the renderer today, which is what
        clonk-org/clonk-rs#391 is about. That split only stays feasible while
        the engine, networking, scripting and resource crates name none of
        these — a single new edge from one of them turns a packaging change
        into a redesign, and nothing else would report it.
        """
        actual = {}
        for crate, manifest in _crate_manifests():
            named = _dependency_names(manifest) & set(PRESENTATION_DEPENDENCIES)
            if named:
                actual[crate] = named

        self.assertEqual(
            actual,
            PRESENTATION_CRATES,
            "the set of crates naming a window, GPU or audio device changed; "
            "if that is deliberate, update PRESENTATION_CRATES and say why in "
            "the pull request",
        )

    def test_the_audio_backend_can_be_dropped_from_a_build(self):
        """`cpal` is the audio *output* device, and a server never opens one.

        A headless run already forces `AudioOptions::silenced`, mirroring
        C++ compiling `ENABLE_SOUND` off for `USE_CONSOLE`
        (`CMakeLists.txt:183-185`). Keeping the backend optional is what lets a
        server build drop it from the graph rather than merely not call it.
        The decoders are deliberately not optional: they are pure Rust and
        parse content, not devices.
        """
        manifest = tomllib.loads(
            (REPOSITORY / "crates" / "clonk-audio" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        cpal = manifest["dependencies"]["cpal"]
        self.assertTrue(
            cpal.get("optional", False),
            "clonk-audio must be buildable without an audio output device",
        )
        self.assertIn(
            "cpal",
            manifest["features"]["default"],
            "the interactive build must keep audio on by default",
        )


if __name__ == "__main__":
    unittest.main()
