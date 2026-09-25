# Raspberry Pi 4 retained GPU qualification

Recorded on 2026-09-25 for clonk-org/clonk-rs#1250, on a board with no monitor
attached, under weston's headless backend. These files are copies of the run's
reports and the relevant log lines. The full run directories stayed on the board.

| Property | Value |
| --- | --- |
| Source commit | `aaa33a3382e189ea829b3836c18a1dcecc218cce` |
| Content commit | `0888b4f3bd10762c976c2fe93aa650d7909c1c6f` |
| Board | Raspberry Pi 4 Model B rev 1.4, 8 GB |
| OS | Debian 13 (trixie), kernel `6.12.47+rpt-rpi-v8`, aarch64 |
| Driver | Mesa `26.2.0-1~bpo13+0~rpt2` (`V3DV Mesa`, Vulkan 1.3.354), Vulkan loader 1.4.309 |
| Window system | Wayland, weston 14.0.2 headless, 1280×720 output |
| Build | Release, on the board, Rust 1.98.1 / LLVM 22.1.8 |
| Executable SHA-256 | `0e9625c5e4b8b55b65c0a83e65204309b8e487c82af062457ca6121f3d0f9acd` |

The checkout was a shallow clone of the exact commit, with `content/` at the
gitlink. [identity.txt](identity.txt) records the rest: the CPU governor, the
load at the start, and `vcgencmd get_throttled`. That reported `0x80000`: the
soft temperature limit had been reached at some point since boot, probably
during the builds, but no throttling was active.

## What ran

1. **The surface lifecycle and a resize on V3D.** weston ran with `--use-gl`,
   which makes V3D surface-compatible. The command was
   `run_headed_surface_teardown_smoke.py --wiring-only --backend vulkan --binary target/release/clonk-app`,
   and it passed; see [headed-smoke/](headed-smoke/):
   - Two real windows presented on `V3D 4.2.14.0`.
   - The child closed while the shell survived.
   - The shell was maximized from 800×600 to 1280×653. Its retained surface
     followed, and it presented again at that extent.
   - `LoopExiting` released exactly the shell.

   `wiring-only` because the authoritative mode exists for the NVIDIA crashes
   and accepts only that hardware. The runner records
   `source_clean_before_and_after: false` for one reason only: the application
   creates an empty `.clonk-update.lock` in its install root, which is the
   checkout root here. `git status` lists nothing else.
2. **The cadence.** `scripts/run-deep-sea-gpu-benchmark.sh 20`, under the same
   GL renderer. [deep-sea-gpu.txt](deep-sea-gpu.txt) holds the machine line: 348
   presentations, all through the retained GPU path. The native-tick assertion
   fails (exit 2), as it must at 17.4 frames per second.
3. **The compositor offering no hardware adapter.** Under weston's default
   renderer, V3D is not surface-compatible; see
   [vulkaninfo-default-renderer.txt](vulkaninfo-default-renderer.txt). The
   application runs on `llvmpipe` and says why; see
   [default-renderer-adapter.txt](default-renderer-adapter.txt).
4. **The floor not met.** The app was started to its menu in a 4400 px wide
   window under the GL renderer. Both hardware attempts refused V3D: "the
   requested 4400px buffer exceeds the adapter's max_texture_dimension_2d of
   4096px". The fallback-adapter attempt then ran on `llvmpipe`, with its
   warning; see [floor-4400px.txt](floor-4400px.txt).
5. **No usable GPU backend.** `run_software_presentation_smoke.py --release --automatic-fallback --no-xvfb`
   passed; see [software-fallback/](software-fallback/):
   - Two GPU attempts with empty backend sets, then software presentation with
     reason `no-adapter`.
   - A maximize from 800×600 to 1280×653, followed by the drawable, frame and
     input presenter.
   - The windowed, fullscreen and windowed-again transition phases.
   - The F9 screenshot and the save thumbnail.
   - An empty window registry at exit.
