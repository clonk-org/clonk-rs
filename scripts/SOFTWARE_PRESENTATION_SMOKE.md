# Software presentation smoke

This opt-in probe drives the shipped `clonk-app` event handler through a real
window that has **no wgpu instance, adapter or device behind it**. It exercises
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

The runner sets `LC_SOFTWARE_PRESENTATION`, which the probe requires: without it
the probe refuses to start rather than quietly qualifying the GPU presenter on a
machine that has a working adapter.

## What it exercises

1. Open the shell window and build a software presenter for it, with no wgpu
   instance created for the presentation path.
2. Paint a full frame and present it through that presenter.
3. **Shrink** the drawable and present again. A shrink rather than a grow: a
   window manager can silently clamp a grow, which would let the resize phase
   pass without resizing anything.
4. Grow the drawable to twice the window's extent **while holding the frame**,
   and present again. This is what a windowed-to-fullscreen transition does to
   the presenter: the renderer keeps drawing at its logical resolution and the
   destination changes underneath it.
5. Restore the windowed drawable and present a third time.
6. Exit the event loop and confirm the window registry is empty, so no window
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

Steps 4 and 5 change the drawable directly rather than asking the window manager
to go fullscreen. That is deliberate and is the same reasoning as the shrink in
step 3: a compositor may refuse or defer a fullscreen request, which would make
the phase pass without transitioning anything. What the presenter sees during a
real transition is a drawable that changed without the frame, and that is what
is reproduced here. Driving a real compositor through the transition is platform
breadth, owned by clonk-org/clonk-rs#1249, clonk-org/clonk-rs#1254 and
clonk-org/clonk-rs#1255.

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
  "schema_version": 2,
  "kind": "clonk_software_present_smoke",
  "success": true,
  "failure": null,
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

Run and passing on:

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
platform. Windows qualification is tracked by clonk-org/clonk-rs#1254.

The native Wayland run above was taken on a board with no monitor attached, so
the compositor had no physical output. It exercises the Wayland protocol path,
the drawable lifecycle and the transition transforms; it does not exercise
modesetting, vsync or scanout, and it says nothing about a compositor other
than weston. Fractional scale is likewise untested — weston's headless output
runs at scale 1.
