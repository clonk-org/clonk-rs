# HarpoonRace 24-player process benchmark

Run this opt-in benchmark from an interactive desktop session on otherwise
idle hardware:

```sh
cargo build --release --offline --locked -p clonk-app --bin clonk-app
PYTHONDONTWRITEBYTECODE=1 \
  python3 scripts/run_harpoonrace_24_player_benchmark.py
```

Use `--dry-run` to inspect the exact commands and port allocation without
opening windows. `--help` lists timeout, port, duration, resolution, and
acceptance overrides.

The runner performs this ordered route:

1. Start one real classic `/network /lobby /console` HarpoonRace host with no
   local player.
2. Wait for its exact HTTP game reference to report `State=Lobby`.
3. Send `/set maxplayer 24`, then wait for `MaxPlayers=24`.
4. Launch 24 rendered clients with isolated user/cache/temp/log directories,
   unique mesh TCP/UDP ports, unique client names, and 24 distinct
   directory-backed `.c4p` profiles. Each profile carries a real checked-in
   `BigIcon.png`, exercising the roster icon decode/upload path.
5. Wait until all 24 exact profile names appear as PlayerInfos in the host
   reference. Then retain the complete fleet for a ten-second default settling
   interval, recheck the exact roster immediately before GO, and only then
   send `/start 0`. This gives the preferred peer mesh time to converge
   without weakening the final full-mesh acceptance gate.
6. Measure every client after the built-in two-second warmup. Each must report
   24 runtime players, 24 synchronized PlayerInfos, 24 activated non-host
   clients, and exactly one live, active, owner-matched SF5B in every runtime
   player's Crew list. The aggregate live-crew-object count remains in the
   report as a diagnostic, but does not substitute for this per-player gate.
   The default local stress contract also requires at least 35 simulation and
   presentation FPS, an average graphics pass no slower than the native 28 ms
   game tick, and a graphics-pass p99 below 25 ms. The simulation threshold
   tracks C++'s approximately 35.7 Hz game cadence. The presentation and p99
   thresholds are intentionally stricter smoothness targets, not C++ parity
   invariants: native automatic frame skipping may present approximately every
   second simulation frame under sustained graphics pressure. Automatic skips
   are retained as evidence and are not an independent failure when the
   configured aggregate performance gates pass. At measurement completion,
   each client also takes one runtime-connection snapshot. The benchmark
   requires an exact preferred message route to every other client, a
   nonnegative ping and lag value for every preferred peer, and a maximum lag
   no greater than `--maximum-network-lag-ms` (100 ms by default). It retains
   TCP/UDP/unknown route counts, maximum packet loss, current PreSend, and the
   smoothed control-send time.
7. Keep clients connected after their individual measurements until all 24
   raw metric/context/network reports exist. The supervisor then applies
   acceptance, releases the fleet together, and closes the host with `/quit`.

This preserves the C++ lobby mechanism instead of modifying scenario content.
HarpoonRace ships with `MaxPlayer=12` (`Scenario.txt:1-5`). The C++ host maps
`/set maxplayer N` to synchronized `C4CVT_MaxPlayer`
(`src/C4MessageInput.cpp:475-490`) and applies it to
`Game.Parameters.MaxPlayers` (`src/C4Control.cpp:162-179`). All profiles must
therefore be admitted before GO: HarpoonRace calls parameterless
`SetMaxPlayer()` in `Script1` (`Script.c:14-18`), closing later player
admission.

Results are retained under
`target/network-benchmark/harpoonrace-24-<timestamp>/`. Important artifacts
include:

- `manifest.json`: commit/content/binary/scenario/machine/topology fingerprint;
- `events.jsonl`: raw launch, admission, `/start 0`, detected `Go!`, report
  completion, and shutdown timeline;
- `benchmark-timing.json`: the supervisor's exact `/start 0` send time,
  timestamped `INFO Go!` evidence when a process log exposes it, and the first
  supervisor observation/file-mtime evidence for each client report;
- `join-admission-samples.json`: all process-launch-to-PlayerInfo timings;
- `presentation-raw.json`: every process report and every graphics-pass sample;
- `summary.json`: pass/fail, p50/p95/p99 aggregates, per-process log volume,
  and the effective logging filter;
- `reference-before-start.txt`: evidence that all 24 PlayerInfos were admitted;
- per-process stdout, stderr, session log, configuration, and profile/core/icon
  evidence.

The timing artifact distinguishes source timestamps from supervisor
observations. The app's machine report currently has no wall-clock timestamp
and does not emit wall-clock markers for warmup or measurement start/end, so
`emitted_at_utc`, `warmup.started_at_utc`, and the measurement start/end fields
remain null. They are not inferred from `Go!`, report-file mtime, or the
supervisor's polling time. The reported warmup duration and measured elapsed
seconds are retained separately.

The topology is deliberately explicit: 24 GUI clients on one machine share its
CPU and GPU. That is a useful local contention/soak result, but it is not a
claim about 24 independent client machines. This rendered fleet is an idle
gameplay soak: it verifies that all 24 HarpoonRace crews stay live and
synchronized, but it does not inject simultaneous movement or firing input.
The rendered-client report exports a one-shot endpoint snapshot after the
measurement window. It proves route coverage and the current ping/lag/loss
state at that boundary; it does not prove that latency remained below the
threshold throughout the soak. Full preferred-message-route mesh is an
explicit benchmark topology requirement, not an engine invariant: C++ relay
fallback remains valid gameplay behavior. The retained
startup-to-PlayerInfo timing includes process and asset loading and must not be
reported as network latency.

The separate socket harness reports client-observed host-route RTT and control
contribution over a same-process Tokio loopback topology. Its cadence follows
the C++ 28 ms game tick times `ControlRate=2`, or 56 ms per control tick.
C++'s network `TargetFPS=38` is retained only for PreSend calculation and is
not used as a simulation clock. Its authoritative default run continuously
samples for 60 seconds and retains raw aggregate/per-client RTT and
control-completion series, so it provides sustained loopback-transport evidence
that the rendered clients' one-shot endpoint snapshot cannot. The socket
harness does not launch HarpoonRace, run rendered clients, or execute `/set`,
so those results are not general fleet/network RTT measurements.

Run the authoritative 24-player socket harness over exact full-mesh reliable
UDP with:

```sh
LC_NETWORK_LOAD_TOPOLOGY=udp \
  cargo test --locked -p clonk-network --test integration \
  network_load_24::harpoonrace_shaped_24_player_control_transport_sustains_lockstep \
  -- --ignored --exact --nocapture
```

Set `LC_NETWORK_LOAD_TOPOLOGY=tcp` for the original full TCP mesh, or
`LC_NETWORK_LOAD_TOPOLOGY=relay` for C++'s valid host-relay fallback. If the
new variable is unset, `LC_NETWORK_LOAD_DIRECT_MESH=0` continues to select
relay mode and every other legacy value continues to select TCP. Reports
record the requested topology and the exact preferred message protocol for
every directed endpoint-to-peer route.

Normal rendered clients do not accept console `/quit`. With
`LC_APP_PRESENTATION_BENCHMARK_KEEP_RUNNING=1`, the supervisor holds every
client in lockstep through the last report and then sends recorded, PID-specific
`SIGTERM`s after asking the host to `/quit`. In-process benchmark assertions
are intentionally disabled: they exit a failed client before slower peers
finish, changing the lockstep roster under measurement. A missing report, an
early exit, an error/desync log, any retained-GPU texture-creation INFO spam,
a forced `SIGKILL`, or any failed supervisor acceptance condition fails the
run. Ordinary application INFO remains enabled; only the noisy
`wgpu_core::device` target is raised to `warn`.
