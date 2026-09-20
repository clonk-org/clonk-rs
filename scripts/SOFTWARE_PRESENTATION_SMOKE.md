# Software presentation smoke

This opt-in probe drives the shipped `clonk-app` event handler through a real
window presented without a wgpu adapter or device. It exercises
the fallback for environments below the retained GPU floor established in
clonk-org/clonk-rs#298. GLES 2.0-only Raspberry Pi 0–3 / VideoCore IV is one
intended route, but this smoke's X11/macOS evidence does not qualify those
boards; clonk-org/clonk-rs#1249 owns that hardware run.

It is the counterpart to `HEADED_SURFACE_TEARDOWN_SMOKE.md`, not a mode of it.
That runner validates GPU adapter and driver teardown and quotes
`adapter_info()` in its evidence, so it cannot speak for a path with no adapter
to report.

```sh
python3 scripts/run_software_presentation_smoke.py
```

The runner requires Python 3.11 or newer and Pillow. By default it sets
`LC_SOFTWARE_PRESENTATION=1` and verifies that no GPU startup was attempted.
`--automatic-fallback` instead removes that setting and supplies an explicitly
empty `WGPU_BACKEND`. The report must show failed GPU attempts with empty backend
sets followed by software selection. An attempt using another backend fails the
check. This exercises the real startup ladder on a machine with a working GPU.

Use `--release` for qualification of a committed release build. The runner honors
`CARGO_BUILD_TARGET`, including the shipped `x86_64-pc-windows-msvc` target, and
isolates configuration, user data, cache, logs and temporary files.

## What it exercises

1. Open the shell window and build a software presenter for it. Record the
   actual selection reason, attempted GPU backends and native window backend.
2. Paint a full frame and present it through that presenter.
3. Request a real window resize and wait for the ordinary OS event to resize
   the software drawable, frame buffer and input presenter. **Shrink** and
   present again. A shrink rather than a grow: a
   window manager can silently clamp a grow, which would let the resize phase
   pass without resizing anything.
4. With `--check-input`, request a native cursor move to physical `(128, 96)`
   at application scale 2. Require both the OS event and the application's
   mapped `(64, 48)` position. This is opt-in because some compositors prohibit
   cursor warping; a missing event fails qualification.
5. Grow the drawable to twice the window's extent **while holding the frame**,
   and present again. This is what a windowed-to-fullscreen transition does to
   the presenter: the renderer keeps drawing at its logical resolution and the
   destination changes underneath it.
6. Restore the windowed drawable and present a third time.
7. Request an F9 screenshot through the application and encode a 200×150 save
   thumbnail from the presented software frame. Decode both PNGs and check
   dimensions and every pixel. This covers thumbnail encoding, not insertion
   into a saved game or full savegame parity. F9 is dispatched through the
   application key handler; the native input check covers cursor events.
8. Exit the event loop and confirm the window registry is empty, so no window
   outlived the loop.

Resize is the phase worth having. A drawable that is not resized with its window
presents a stale or wrongly-sized frame, and unlike the GPU path there is no
surface reconfiguration underneath to catch it.

The transition phases exist because the resize cannot catch a wrong *scale* or
crop: it moves the frame and the drawable together, so the scale stays one and
nothing is ever letterboxed. Only a drawable that changes on its own produces a
scale above one, and the report records the resulting scale and clip rectangle
for each phase so a transform kept across a transition is visible rather than
inferred.

Steps 5 and 6 change the drawable directly. Their historical `fullscreen` phase
name describes the presenter's scale/clip exercise; it does not prove an OS
fullscreen transition. Step 3 does require an actual window resize.

## Running without a desktop session

With no `DISPLAY` or `WAYLAND_DISPLAY` the runner launches under `xvfb-run`, so
a headless machine can still qualify this path against a real X11 server. That
is not a lesser run for this particular probe: an X server with no GPU behind it
*is* the environment the software presenter exists for. Pass `--no-xvfb` to
refuse the fallback instead.

Xvfb needs `xvfb` and `xauth` installed; `xvfb-run` fails with
`xauth command not found` without the latter. On a Debian-derived image winit
also dlopens X libraries that a minimal container does not carry:

```sh
apt-get install --no-install-recommends \
  xvfb xauth libx11-6 libxcursor1 libxrandr2 libxi6 libxkbcommon0 \
  libxkbcommon-x11-0 libxcb1
```

Without those, the run fails early with `Failed to load one of xlib's shared
libraries` from winit rather than anything about presentation.

A container with no sound card also logs `failed to create audio stream` from
ALSA. That is not a failure of this probe — audio is not part of what it
qualifies, and the run continues.

`clonk-app` refuses to run as root, so the probe cannot either — in a container,
run it as an ordinary user. The runner checks this before spending a build.

## Reading the report

The probe writes a JSON report to `target/software-present-smoke/report.json`,
or to `--artifact-dir`. A passing run:

```json
{
  "schema_version": 3,
  "kind": "clonk_software_present_smoke",
  "success": true,
  "failure": null,
  "software_reason": "forced",
  "gpu_attempt_backends": [],
  "display_backend": "windows",
  "target_os": "windows",
  "target_arch": "x86_64",
  "input_mapping": {
    "window_position": [128, 96],
    "gui_position": [64, 48],
    "scale": 2
  },
  "initial_extent": [800, 600],
  "resized_extent": [760, 560],
  "presented_before_resize": true,
  "presented_after_resize": true,
  "phases": [
    {
      "name": "windowed",
      "frame_extent": [760, 560],
      "drawable_extent": [760, 560],
      "scale": 1,
      "clip_rect": [0, 0, 760, 560],
      "presented": true
    },
    {
      "name": "fullscreen",
      "frame_extent": [760, 560],
      "drawable_extent": [1520, 1120],
      "scale": 2,
      "clip_rect": [0, 0, 1520, 1120],
      "presented": true
    },
    {
      "name": "windowed-again",
      "frame_extent": [760, 560],
      "drawable_extent": [760, 560],
      "scale": 1,
      "clip_rect": [0, 0, 760, 560],
      "presented": true
    }
  ],
  "registry_empty_at_exit": true
}
```

The example includes `--check-input`; otherwise `input_mapping` is null. The
runner also writes `run.log`, `report.screenshot.png`, `report.thumbnail.png`,
and `qualification.json`. The latter records the commit, pinned content
revision, source/binary/artifact SHA-256 hashes, OS version, architecture,
compiler, build profile/target/flags, native backend and checked mode. It rejects
changed content, source changes during the run, executable replacement, and
uncommitted source inputs for a release qualification.

The runner treats the process exit code as authoritative and additionally
rejects:

- a report whose `initial_extent` equals its `resized_extent`, because a resize
  that did not change the extent proves nothing about the drawable following the
  window;
- a phase sequence that is not `windowed`, `fullscreen`, `windowed-again`, or
  any phase that presented nothing;
- a phase whose `clip_rect` does not fit its own `drawable_extent` — which is
  what a transform kept across the transition produces;
- a `fullscreen` phase whose `scale` is still one, because a transition that did
  not scale could not have caught a wrong scale.

## Coverage

Earlier schema-2 reference runs passed on:

- **Linux / X11 under Xvfb** — `aarch64`, Debian-based `rust:1.98.0` container,
  no GPU present.
- **macOS** — an ordinary desktop session.
- **Linux / native Wayland** — `aarch64`, Raspberry Pi 4 Model B rev 1.4,
  Debian 13 (trixie), kernel 6.12.47, weston 14.0.2. No XWayland: the app
  reports `display_backend: wayland`. A V3D GPU is present and unused, which is
  the point — the software presenter has to work where an adapter exists but is
  not being asked for. Schema 2, all three transition phases presented, scale
  1 → 2 → 1, no window left behind.

These are path-specific reference runs, not a claim about every `softbuffer`
platform.

**Windows schema-3 qualification passed on 2026-09-20:** Windows Server 2025
(`10.0.26100`), AMD64, Win32, shipped `x86_64-pc-windows-msvc` release build with
static CRT and LLD ThinLTO. Source `9c774f82275f4595938ad950e7f9b71fc159280d`,
content `9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`. Both forced and automatic
fallback passed with `--check-input`; every stage above completed. The executable
hash matches the workflow's successful static-CRT validation. The
[reports, capture images, logs and qualification metadata](../docs/evidence/windows-software-presentation/README.md)
are retained in the repository, with links to the original run and artifact.

The schema-3 procedure can be dispatched against a branch using the existing
**Main validation** workflow with `software_presentation=true`. Its Windows
release tooling job configures and validates the shipped static-CRT MSVC build,
then runs both modes with `--release --check-input`. It uploads the
`windows-software-presentation` artifact even on failure. Run the same commands
on a Windows desktop after configuring the shipped build:

```sh
python scripts/run_software_presentation_smoke.py --release --check-input \
  --artifact-dir target/windows-software-presentation/forced
python scripts/run_software_presentation_smoke.py --release --check-input \
  --automatic-fallback --artifact-dir target/windows-software-presentation/automatic
```

The native Wayland run above was taken on a board with no monitor attached, so
the compositor had no physical output. It exercises the Wayland protocol path,
the drawable lifecycle and the transition transforms; it does not exercise
modesetting, vsync or scanout, and it says nothing about a compositor other
than weston. Fractional scale is likewise untested — weston's headless output
runs at scale 1.
