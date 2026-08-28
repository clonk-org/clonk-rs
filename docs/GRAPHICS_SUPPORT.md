# Graphics presentation support

The primary interactive window has two presentation paths:

- retained GPU presentation through wgpu, which is the normal path;
- CPU composition presented by the wgpu-independent `SoftwarePresenter`, used
  when the GPU path cannot start or when an operator sets
  `LC_SOFTWARE_PRESENTATION=1`.

The graphics floor below applies only to retained GPU presentation. Falling
below it no longer means that interactive play necessarily fails: the
application next tries the software presenter. Headless server mode
(`--headless`) initializes neither presenter.

## Retained GPU floor

This table and `crates/clonk-surface/src/capability.rs` are one contract stated
twice. The tests
`the_declared_floor_is_what_the_renderer_actually_requires` and
`the_device_request_asks_for_nothing_beyond_the_published_floor` fail if the
renderer silently starts requiring more.

| Requirement | Value | Why |
| --- | --- | --- |
| Graphics API | Vulkan 1.0, Metal, DX12, desktop GL, or GLES 3.0 | wgpu's GLES backend rejects contexts below GLES 3.0 |
| Optional wgpu features | none (`Features::empty()`) | timestamp queries are opt-in and degrade to unavailable |
| Surface format | at least one sRGB format | composition relies on the surface encode |
| Frame-buffer format usages | `TEXTURE_BINDING \| COPY_DST` | the frame buffer is uploaded and sampled |
| `max_texture_dimension_2d` | at least 2048 and at least the requested extent | GLES 3.0 and WebGL2 guarantee 2048 |
| Compute shaders | not required | presentation is a render-pipeline blit |

The device request uses `adapter.limits()`, so a stronger adapter remains
available to the renderer. The table is the minimum the GPU path may require,
not a cap on what it may use.

## Selection and fallback

Unless `WGPU_BACKEND` is set, GPU discovery tries `Backends::PRIMARY` first,
then all backends to include GL/GLES, and finally asks the widest set for a
software wgpu adapter such as llvmpipe, lavapipe, or WARP. An explicit
`WGPU_BACKEND` is authoritative: every attempt stays inside the named backend.
`WGPU_ADAPTER_NAME` selects among adapters for the surface and falls back to
the default adapter if no name matches.

If the complete GPU startup ladder fails, the primary window tries
`SoftwarePresenter`. That fallback owns a CPU RGBA frame and presents it
through `softbuffer`; it creates no wgpu instance, adapter, or device. A forced
`LC_SOFTWARE_PRESENTATION=1` request takes this path immediately, which makes
the fallback reproducible on a GPU-capable machine.

The software path is currently bounded to the primary application/game window.
Developer and editor secondary windows retain their existing GPU-backed paths;
software fallback for those tools is an explicit non-goal, not implied support.
Their registry and headed teardown tests independently prove that closing a
child leaves the primary shell alive. clonk-org/clonk-rs#1262 owns the remaining
fullscreen/input-coordinate proof, and clonk-org/clonk-rs#1263 owns
presenter-specific performance evidence. Presentation choice does not change
simulation, lockstep, RNG, save, or replay state.

## Failure classes

GPU startup preserves the concrete `SurfaceError`: no adapter, a below-floor
capability report, device creation, surface creation, or an invalid extent.
During rendering, surface loss requests a rebuild; validation and callback
failures remain fatal to that GPU presentation attempt. A startup failure in
any GPU class triggers the software attempt for the primary window.

Software presentation can fail because the platform window cannot be attached,
the drawable is empty, the CPU frame cannot be allocated, or compositing or
presenting fails. If both paths fail, the final diagnostic retains the complete
GPU error chain and the software-presenter error instead of reporting only the
last failure.

## Qualification status

API availability is not hardware qualification. In particular, a board
advertising GLES 3.1 has not proved its driver limits, surface formats, or frame
budget.

| Route | Evidence and current scope |
| --- | --- |
| Desktop retained GPU | The pure capability tests cover the declared floor. `scripts/run_headed_surface_teardown_smoke.py` is the live adapter/device/surface qualification procedure and records its exact backend and adapter. |
| Linux/X11 software | A no-GPU Xvfb reference run is recorded in `scripts/SOFTWARE_PRESENTATION_SMOKE.md`; it covers attach, present, resize, and shutdown. |
| macOS software | A reference run is recorded in `scripts/SOFTWARE_PRESENTATION_SMOKE.md`. |
| Windows software | Not yet qualified; clonk-org/clonk-rs#1254 owns the missing platform evidence. |
| Native Wayland software | Qualified on a Raspberry Pi 4 under weston 14.0.2 with no XWayland: presented, resized, ran the windowed/fullscreen/windowed transition sequence, and left no window behind. The reference run is recorded in `scripts/SOFTWARE_PRESENTATION_SMOKE.md`, including what it does not cover — no scanout, one compositor, scale 1 only. |
| Raspberry Pi 4 retained GPU | Runs, but does not sustain the cadence. Real-board evidence below: V3D presents every frame through the retained path, and the complete frame is 46 ms at p50 against a 28 ms budget. **Implemented and correct, not playable at native speed.** |
| Raspberry Pi 5 retained GPU | Still no real-board evidence. A Pi 4 result says nothing about it — different GPU generation, different driver limits. Tracked by clonk-org/clonk-rs#1250. |
| Raspberry Pi 0–3 / VideoCore IV software | The GPU path cannot use their GLES 2-only adapter. The wgpu-free fallback is a possible from-source route, not a support claim; real-board windowing and cadence remain unqualified in clonk-org/clonk-rs#1249. |

The repository currently publishes Linux only for x86_64, so no Raspberry Pi
generation has a shipped release asset. Do not infer support for one Pi
generation from another, or from API version alone. Until the linked hardware
issues close, the honest statement is that the paths are implemented but the
boards are unqualified.

Run the software qualification procedure with:

```sh
python3 scripts/run_software_presentation_smoke.py
```

Run the retained GPU procedure with an explicit backend as documented in
`scripts/HEADED_SURFACE_TEARDOWN_SMOKE.md`. Preserve the generated JSON report,
commit, content revision, OS, architecture, driver/backend, window system, and
hardware with every qualification result.

Note that `scripts/run_headed_surface_teardown_smoke.py` cannot qualify other
hardware: it rejects any adapter that is not discrete NVIDIA on the proprietary
driver, because it exists for the NVIDIA Wayland crashes in
clonk-org/clonk-rs#53 and clonk-org/clonk-rs#54. Qualifying a different board
means driving `--headed-surface-smoke` directly and recording the report by
hand, as the Pi 4 run below did.

## Raspberry Pi 4 retained GPU, qualified

Commit `7497af24799d`, content `b9214cafb46a`, built with the pinned 1.98.0
toolchain on the board itself (51m57s, release profile).

| | |
| --- | --- |
| Board | Raspberry Pi 4 Model B rev 1.4, 8 GB |
| OS | Debian 13 (trixie), kernel 6.12.47 aarch64 |
| Adapter | `V3D 4.2.14.0`, integrated GPU |
| Driver | `V3DV Mesa`, Mesa 26.2.0-1~bpo13+0~rpt2, Vulkan 1.3.354, conformance 1.3.8.3 |
| Backend | vulkan |
| Window system | Wayland (weston 14.0.2, headless backend, GL renderer on V3D) |

**The surface lifecycle passes.** Two real windows and two real surfaces:
both presented, the child closed while the shell survived, the child released,
the shell presented again afterwards, and the registry was empty at loop exit.

**The cadence does not.** A 20-second Deep Sea measurement, 348 presentations,
every one of them through the retained GPU path and none through the CPU
presenter:

| | p50 | p95 | max |
| --- | ---: | ---: | ---: |
| complete frame | 46.0 ms | 101.1 ms | 117.6 ms |
| simulation | 26.6 ms | 82.5 ms | 99.9 ms |
| platform present | 18.5 ms | 19.6 ms | 27.4 ms |
| CPU raster | 0.09 ms | 0.12 ms | 0.24 ms |

That is 17.4 presented frames per second against the 28 ms tick's 35.7, and the
run fails the native-tick assertion. The split says where it goes: simulation
alone spends the whole budget at p50, the platform present spends most of
another one, and CPU composition is free at 0.09 ms. `surface_reallocations=0`,
so no setup cost is hiding inside those steady-state numbers.

**Two limits worth naming.** `max_texture_dimension_2d` is 4096 on V3D, where
the desktop adapters this port is developed against report 16384 or more. The
floor compares that against the requested buffer extent rather than a fixed
threshold, so it passes below 4096 px and fails above — a 4K output would not
present here. wgpu also reports V3D as downlevel: "does not support enough
features to be a fully compliant implementation of WebGPU".

**What this run does not cover.** The board had no monitor attached — both HDMI
connectors read `disconnected` — so the compositor ran headless and nothing was
scanned out to a display. Modesetting, vsync and real display output remain
unqualified on this board. Pi 5 remains entirely unqualified.

**The compositor decides which adapter you get, and nothing says so.** With
weston's headless backend on its default renderer, V3D is not among the
adapters compatible with the surface — the only one offered is `llvmpipe`, a
CPU Vulkan device — and the run still reports success with no indication it was
not on the GPU. Passing `--use-gl`, which brings up the GL renderer on V3D and
advertises dmabuf, is what makes V3D surface-compatible. **Any Pi qualification
must read the adapter out of the report rather than assume the board's GPU was
used.** The missing diagnostic is clonk-org/clonk-rs#1381.
