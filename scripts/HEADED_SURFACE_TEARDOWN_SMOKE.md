# Headed GPU surface teardown smoke

This opt-in gate drives the shipped `clonk-app` event handler through two real
windows and two real GPU surfaces. It exists for the NVIDIA Wayland crashes in
clonk-org/clonk-rs#53 and clonk-org/clonk-rs#54, whose ordinary unit tests can
only pin the underlying ownership rules.

The authoritative run requires an interactive Linux Wayland session on a
discrete NVIDIA GPU with the proprietary NVIDIA Vulkan driver active. It also
requires a clean Git checkout and builds the release binary itself so the
evidence is bound to that exact commit. The runner forces a fresh Cargo target
directory, removes ambient compiler, flags, wrapper, target and profile
overrides, copies the executable path reported by Cargo's compiler-artifact
record into the evidence directory, and launches that exact copy. It also
requires the clean `content/` checkout to match the commit's gitlink. From a
terminal inside that session:

```sh
python3 scripts/run_headed_surface_teardown_smoke.py
```

Do not run it over SSH without access to that live session. The runner requires
`WAYLAND_DISPLAY` and `XDG_SESSION_TYPE=wayland`, forces
`WGPU_BACKEND=vulkan`, and rejects any adapter that is not reported by the
actual surface device as discrete NVIDIA hardware using the proprietary
`NVIDIA` driver rather than Mesa NVK. `--artifact-dir PATH`
selects a fresh destination; otherwise evidence is retained below
`target/headed-surface-smoke/`.

The runner uses an isolated windowed 800×600 configuration and artifact-local
user, cache, log and temporary directories. It removes ambient Clonk diagnostic
and GPU-selection overrides before launch, while retaining the Wayland/Vulkan
driver environment needed to create the real surfaces.

The hidden application mode stays inside the normal `RuntimeApplication` and
production event-handler closure. It performs this sequence:

1. Build the shell's real window and surface through the ordinary framebuffer
   builder.
2. Build and register a real viewport window through that same path.
3. Present both surfaces from their real redraw callbacks.
4. Destroy the viewport through `DeveloperWindows::close`, prove its weak
   window handle died while the shell survived, then present the shell again.
5. request event-loop exit and let `Event::LoopExiting` reach the ordinary
   shutdown code unchanged.
6. Write `app-report.json` only after production
   `DeveloperWindows::release_all` has returned and the shell weak handle has
   died.

The Python runner independently rejects the report unless the two distinct
windows used the exact same retained-instance registry entry, the acquisition
sequence was one creation followed by one reuse, the survivor presented after
the child closed, and `LoopExiting` released exactly the remaining shell. A
crash, timeout, stale or partial report, unknown schema, duplicate JSON key,
missing driver identity, or nonzero process exit fails the run. Removing the
retained-instance funnel breaks the surface-to-entry evidence; removing the
production `release_all` call leaves the shell alive and prevents a successful
report.

The artifact directory contains the exact tested `clonk-app` executable,
release-build logs, raw application report, stdout, stderr, isolated
configuration and user data, plus `qualification.json`. The last file records
the clean Git commit, in-run build command, matching Cargo/copy/pre/post-run
binary SHA-256, controlled environment, platform, exact launch command,
elapsed time, adapter identity and complete lifecycle report. Preserve that
directory with any headed-hardware result.

## Wiring-only runs

Another headed backend can check the real-window/event-loop wiring without
claiming that the NVIDIA Wayland faults are retired. For example, on macOS:

```sh
cargo build --release --locked -p clonk-app --bin clonk-app
python3 scripts/run_headed_surface_teardown_smoke.py \
  --binary target/release/clonk-app \
  --wiring-only \
  --backend metal
```

The windows must be compositor-visible: an occluded or unavailable drawable is
reported as `Skipped` by wgpu and the gate intentionally times out rather than
calling it a presentation. A wiring-only result is useful lifecycle evidence,
but it is not Linux, Wayland, Vulkan or NVIDIA crash evidence.

There is currently no repository or organization self-hosted runner, so no
workflow pretends to provide this hardware lane. Once a headed
Linux/Wayland/proprietary-NVIDIA runner is provisioned, invoke this same default
command from it and retain `qualification.json`; do not replace it with an
Xvfb, software Vulkan or headless compositor pass.
