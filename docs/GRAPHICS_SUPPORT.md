# Graphics support matrix

What a machine must offer before interactive play can start, what the port
requires of it, and which devices are therefore supported.

This document and the constants in `crates/clonk-surface/src/capability.rs` are
one statement made twice. `the_declared_floor_is_what_the_renderer_actually_requires`
and `the_device_request_asks_for_nothing_beyond_the_published_floor` fail when
they drift, so a dependency bump that needs more than this cannot land without
changing both.

## The floor

| Requirement | Value | Why |
|---|---|---|
| Graphics API | Vulkan 1.0, Metal, DX12, desktop GL, **or GLES 3.0** | wgpu-hal's GLES backend rejects any context below GLES 3.0 (`wgpu-hal/src/gles/egl.rs`) |
| wgpu features | none (`Features::empty()`) | timestamp queries are opt-in and the renderer runs without them |
| Surface format | at least one sRGB format | presentation composites in byte space and relies on the surface encode to restore those bytes |
| Frame-buffer format usages | `TEXTURE_BINDING \| COPY_DST` | the frame buffer is uploaded to and sampled, and nothing else |
| `max_texture_dimension_2d` | ≥ 2048, and ≥ the presentation extent | GLES 3.0 and WebGL2 both guarantee 2048 |
| Compute shaders | not required | presentation is a blit through a render pipeline |

The device is created with `required_limits: adapter.limits()`, so an adapter
offering more than the floor is used to the full. The floor bounds what the port
may *require*, not what it may use.

## Backend selection

`framebuffer_backend_attempts` tries `Backends::PRIMARY` (Vulkan, Metal, DX12,
WebGPU) and then `Backends::all()`, which adds GL/GLES. PRIMARY comes first
because the GL backend probes for libEGL and logs a spurious "Unable to open
libEGL" on macOS; widening on failure is what lets a board whose only usable
driver is GLES start at all.

An explicit `WGPU_BACKEND` is an operator instruction: it is honoured exactly and
**never** widened past, which
`framebuffer_backends_widen_to_gl_before_giving_up` pins. `WGPU_ADAPTER_NAME`
selects among the adapters that support the surface, and a name matching none of
them logs and falls back to the default adapter rather than failing startup.

## Devices

| Device | Interactive play | Notes |
|---|---|---|
| Desktop Windows / macOS / Linux with a current driver | Supported | Vulkan, Metal or DX12 |
| Software adapters (llvmpipe, lavapipe, WARP) | Supported when they report GLES 3.0/Vulkan 1.0 and an sRGB surface | slow, but above the floor; this is the lowest tier the probe can be exercised against without hardware |
| Raspberry Pi 4 / Pi 5 | Supported | V3D exposes GLES 3.1; not blocked by the GLES 2 limit, though a driver may still fail device creation |
| Raspberry Pi Zero 2 W / Pi 3 | **Unsupported** | VideoCore IV exposes GLES 2.0 only |
| Raspberry Pi 0 / Pi 1 / Pi 2 | **Unsupported** | VideoCore IV exposes GLES 2.0 only |

### The Pi 0–3 decision

Pi 0–3 are **explicitly unsupported for interactive play**, and software
presentation does not change that. The blocker is not the speed of the
rasteriser: wgpu-hal refuses to create a GLES context below 3.0, so those boards
produce no adapter on any backend, and the presentation path needs a wgpu device
even to blit a CPU buffer. A software presenter (clonk-org/clonk-rs#299) is
therefore scoped to machines that *have* a usable adapter but cannot drive the
retained pipeline well — not to recovering VideoCore IV.

Headless server mode (`--headless`) is unaffected: it initializes no device and
runs anywhere the rest of the engine does.

## Startup failure classes

The four are distinguished so a user is told which of them happened:

| Class | Surfaced as |
|---|---|
| No adapter at all (no installed driver, or a GLES 2-only board) | `SurfaceError::AdapterNotFound`, after every backend set has been tried |
| An adapter below the floor | `SurfaceError::BelowGraphicsFloor`, carrying one diagnostic naming **every** unmet requirement |
| Device creation failed on an adapter that passed | `SurfaceError::Device`, carrying wgpu's `RequestDeviceError` |
| Surface lost while running | `SurfaceError::Lost` — the one presentation failure callers recover from, by rebuilding the surface rather than reconfiguring it |

## Qualification

The probe is a pure function over capability *data*, so every requirement above
is exercised by unit tests on every host, including the tiers no runner has.
Live adapters are covered as follows:

- **Desktop backends** — the headed surface smoke test on the macOS, Linux and
  Windows jobs.
- **Software adapters** — runnable anywhere with `WGPU_BACKEND=gl` plus a
  llvmpipe/lavapipe ICD; this is the lowest tier a runner can host.
- **GLES 3.0 hardware and Raspberry Pi 4/5** — no runner exists, so these are
  qualified by hand before a release: install the release build on a Pi 4 or Pi
  5, start a scenario, and confirm the game presents frames and reports no
  below-floor diagnostic. Record the result in the release checklist.

A device that fails manual qualification moves to **Unsupported** in the table
above rather than lowering the floor: the floor may only drop if there is a
renderer path that presents correct frames below it.
