# Live GPU recovery qualification

`scripts/run_device_loss_probe.py` exercises device destruction and recovery
inside the shipped application's window event loop. It leaves the startup
screen idle with an isolated copy of the real player-profile fixture (avoiding
the first-start name editor and its blinking caret), waits for 30 retained
presentations, captures the last presented composition, then calls
`wgpu::Device::destroy`. This is a backend-authoritative
injection, not an operating-system driver reset or a synthetic surface error.

```sh
python3 -m venv /tmp/clonk-probe-venv
/tmp/clonk-probe-venv/bin/python -m pip install Pillow
/tmp/clonk-probe-venv/bin/python scripts/run_device_loss_probe.py --release \
  --backend metal --artifact-dir /tmp/clonk-device-loss-metal
```

Choose `metal`, `dx12`, `vulkan`, or `gl` explicitly. The runner clears inherited
presentation overrides and uses isolated configuration and user-data paths.
On Linux without a display, it uses Xvfb; install Xvfb, xauth, and the Mesa
Vulkan/GL drivers. Windows uses the shipped three-package build graph and must
use `scripts/configure-msvc-runtime.sh` before release qualification, followed
by `scripts/validate-msvc-runtime.sh`. A software GPU adapter such as lavapipe
or WARP exercises its named graphics backend; it does not qualify a hardware
vendor's driver or the application's separate software presenter.

Success requires all of the following:

- The retained renderer receives a `Destroyed` device-loss diagnosis and the
  ordinary event loop successfully rebuilds it at a newer generation.
- wgpu's live surface count falls by one after dropping the old configured
  surface and before constructing the replacement.
- The first recovered frame uses the same source texture identities as the
  reference scene, recreates every one, and uploads their complete contents.
  No pre-loss GPU texture is reused. Cached art from earlier screens is counted
  separately; it is restored only when a later scene needs it.
- Readback of that first actual presentation is byte-identical to the
  pre-loss reference, with nonuniform textured content in the captured image.
- Three frames present on the replacement generation, without an unrequested
  switch to CPU composition or the software presenter.

The image comparison requires a static screen. Do not use an animated scenario
as this probe's reference: an intentional scene change would correctly fail
the before/after comparison. This startup-screen fixture qualifies the live
recovery path and its retained source textures, not every gameplay shader or
the behavior of a physical GPU reset. Existing renderer tests separately cover
landscape resources and preserving configured rendering options.

Both startup and recovery have 15-second deadlines. A locked or occluded
display is a failed run, not qualification and not evidence that its backend
cannot inject device loss. The outer runner also bounds process execution.

`report.json` contains adapter/driver details, generations, callback diagnosis,
surface counts, texture/upload counts, presentation counts, and elapsed recovery
time. `report.before.png` and `report.after.png` retain the presented pixels.
`qualification.json` binds those files and the log to the source and pinned
content commits, source-input hash, executable hash, OS, toolchain, and build
flags. Release mode refuses dirty source inputs and any source or executable
change during the run.

`.github/workflows/device-loss-qualification.yml` runs Linux Vulkan and GL,
Windows DX12, and macOS Metal. Release candidates invoke it from exact-SHA
qualification; developers can dispatch `rust.yml` with `device_loss=true` on
their branch. Each platform retains failure diagnostics as well as successful
evidence. These rows must pass before any platform is recorded as qualified.

The [accepted platform evidence](evidence/live-gpu-recovery/README.md) records
successful Vulkan, GL, DX12, and Metal runs at source
`496ae5bd269dd3622d529f73f1266023e15d9eb9` for clonk-org/clonk-rs#1241.
It retains the source/binary identities, reports, matching image pairs, upload
counts, and logs, with the software and paravirtual adapter scope stated for
each platform.
