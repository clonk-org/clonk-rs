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
| Native Wayland software | Not yet qualified; the Xvfb run does not exercise Wayland. Tracked by clonk-org/clonk-rs#1255. |
| Raspberry Pi 4 / Pi 5 retained GPU | Their nominal APIs avoid the GLES 2 rejection, but neither generation has real-board evidence. Tracked separately by clonk-org/clonk-rs#1250. |
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
