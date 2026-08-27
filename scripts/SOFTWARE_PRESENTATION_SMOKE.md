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
4. Exit the event loop and confirm the window registry is empty, so no window
   outlived the loop.

Resize is the phase worth having. A drawable that is not resized with its window
presents a stale or wrongly-sized frame, and unlike the GPU path there is no
surface reconfiguration underneath to catch it.

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
  "schema_version": 1,
  "kind": "clonk_software_present_smoke",
  "success": true,
  "failure": null,
  "initial_extent": [800, 600],
  "resized_extent": [760, 560],
  "presented_before_resize": true,
  "presented_after_resize": true,
  "registry_empty_at_exit": true
}
```

The runner treats the process exit code as authoritative and additionally
rejects a report whose `initial_extent` equals its `resized_extent`, because a
resize that did not change the extent proves nothing about the drawable
following the window.

## Coverage

Run and passing on:

- **Linux / X11 under Xvfb** — `aarch64`, Debian-based `rust:1.98.0` container,
  no GPU present.
- **macOS** — an ordinary desktop session.

These are path-specific reference runs, not a claim about every `softbuffer`
platform. Windows qualification is tracked by clonk-org/clonk-rs#1254, native
Wayland by clonk-org/clonk-rs#1255, fullscreen/input-coordinate proof by
clonk-org/clonk-rs#1262, and presenter-specific performance evidence by
clonk-org/clonk-rs#1263.
