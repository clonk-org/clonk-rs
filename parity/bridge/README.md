# Shadow-diff bridge ABI

`lc_engine_ffi.h` is the C ABI the pinned oracle's `USE_RUST_ENGINE_VALIDATION`
bridge (`src/rust/RustEngineBridge.cpp`) calls. It starts from the header in
oracle commit `7d43b47b7d789b533f32d005e64596e0a07019cd`
(`rust/include/lc_engine_ffi.h`) and carries the current tree's reviewed live
validation extensions. It is vendored here so the contract is versioned next
to the implementation instead of living only in a C++ checkout —
`crates/clonk-engine/src/ffi.rs` (feature `ffi`) is what satisfies it.

This bridge complements the committed primitive golden in
[`../README.md`](../README.md) by running the current Rust engine beside a real
C++ scenario.

## Building the artifacts

```sh
cargo xtask ffi --profile debug     # or --release
```

That emits `target/<profile>/libclonk_engine.a` and the matching dynamic
library — the exact paths `CMakeLists.txt:97-107` imports and
`:404-406` links for `USE_RUST_ENGINE_VALIDATION`. The oracle runs the same
command itself (`CMakeLists.txt:138-143`).

The crate types are emitted by that command rather than declared in
`clonk-engine`'s manifest, as the pinned tree did it: a `crate-type` entry
would make every ordinary build pay the staticlib archive and the cdylib link.

## Running the shadow diff

```sh
git -C <oracle-repo> worktree add <path> 7d43b47b7d789b533f32d005e64596e0a07019cd
parity/bridge/build-oracle-validation.sh --oracle-root <path>
```

That builds the pinned oracle with `-DUSE_RUST_ENGINE_VALIDATION=ON` linking
**this** tree, rather than the Rust snapshot bundled at the oracle commit. The
script is not a convenience wrapper — four things have to be true at once or
the build silently uses the wrong engine, or does not configure at all:

- **The pinned source predates this tree's weather transport.** The script
  applies `parity/bridge/oracle-weather.patch` on top of the unchanged pinned
  commit. The patch is validation instrumentation only: it captures the
  already-evaluated rain gate and supplies the native weather payload. The
  script accepts either a wholly unapplied or a wholly applied patch and
  rejects partial or otherwise drifted source.
- **The pinned `CMakeLists.txt` cannot configure this option as shipped.** It
  carries a literal backspace (`0x08`) glued to the `clonk_engine_static` target
  name in all three places it appears, so CMake rejects the name as invalid
  while *printing* the clean one — which reads like a missing-artifact path
  problem and is why the option has never been usable. This is a typo in build
  plumbing, not engine behaviour, so it cannot affect determinism.
- **Every Rust path is hardcoded to `${CMAKE_SOURCE_DIR}/rust`** with no
  override variable, and `add_dependencies(clonk rust_build)` runs
  `cargo xtask ffi` *in that directory*. Pointing that entry at this checkout is
  what makes the oracle build your tree; `RUST_INCLUDE_DIR` then needs
  `include/lc_engine_ffi.h` to exist here, which the script creates untracked.
- The pin vendors fmt 11 headers on the zlib/curl include path while
  `find_package(fmt)` links a newer fmt, so they must be shadowed.

### Arming it — silence does not mean agreement

`EnsureInitialised` ends with `if (!g_recorder && !g_playback &&
!g_runtime_requested) g_disabled = true;`, after which `OnFrame` returns
immediately. **A run with no `LC_RUST_ENGINE_*` variable set produces a clean
log and compares nothing**, which is indistinguishable from a passing diff. Arm
it explicitly:

```sh
LC_RUST_ENGINE_RUNTIME=1 ./clonk        # live lockstep diff
LC_RUST_ENGINE_RECORD=<path> ./clonk    # C++ snapshots as JSON, for triage
```

Divergences are reported as `Rust runtime parity mismatch: ...`.

To prove that independently transported weather is active even when no object
reads it, compare a normal armed run with one that perturbs only the native
payload's wind value:

```sh
LC_RUST_ENGINE_RUNTIME=1 ./clonk

LC_RUST_ENGINE_RUNTIME=1 \
LC_RUST_ENGINE_RUNTIME_WEATHER_FAULT=wind \
./clonk
```

The normal run leaves the payload untouched. On an otherwise matching run, the
fault-injected command must stop at its first comparison with a diagnostic of
the form
`frame N: weather wind rust X, cpp Y`; it is a wiring check, not a scenario
parity result.

### Counting events across a run

The harness stops stepping Rust at the first divergence while C++ remains the
host and may continue. Whole-run event totals therefore measure different frame
spans after a mismatch. To compare counts, either:

- use a scenario that reports **no** divergence, so both engines execute the
  same frames; or
- log each side's frame number and truncate the C++ series to the Rust
  engine's last stepped frame before comparing.

### Probe placement — the same event logs different state on each side

A probe in C++'s `C4Object::SetDir` and one in the port's
`write_object_direction` do **not** observe the same moment: `SetDir` runs
`SetActionByName(TurnAction)` between them (`C4Object.cpp:4243-4248`), so C++
reports the pre-turn action and the port the post-turn one for the identical
event. Comparing the action names side by side shows a divergence that is
entirely instrumentation.

Prefer a metric that does not depend on where the probe sits — counting only
writes that actually *change* a value, for instance — or place both probes
relative to the same landmark.

### Passing the scenario — both engines must load the same `System.c4g`

The port resolves its install root by walking the **ancestors of the scenario
path** for `planet/System.c4g`
(`crates/clonk-engine/src/ffi.rs`, `load_scenario_into_runtime`), while C++
resolves it from its own working directory. Pass an absolute scenario path
inside this repository and the two engines silently load **different** system
groups: the port picks up `planet/System.c4g` from the checkout, whose
port-authored `#appendto` scripts (`BirdFlight.c`, `EkeAirbikeSteering.c`,
`GatherTask.c`, `FoWReveal.c`, ... — the set the compatibility profile
withholds, `crates/clonk-app/src/compat_readiness.rs`) never run under the
oracle.

The diff then reports content that only one side was given. Because scenarios
that do not load a port-authored append can still agree, mixing path forms can
invalidate only part of a batch without producing an obvious harness failure.

Pass the scenario **through the build tree** instead, so the ancestor walk finds
the group C++ is using:

```sh
cd <oracle>/build-validation
printf '/open %s/Races.c4f/Goldrace.c4s %s/Tyler.c4p\n' "$PWD" "$PWD" \
  | LC_RUST_ENGINE_RUNTIME=1 ./clonk
```

Comparing an install-root-sensitive scenario any other way is not a parity
result.

## Comparison boundary

The normal, non-authoritative loop compares the frame number, synchronized RNG
ledger, independently executing weather/environment state, ordered live-object
snapshots and definition histogram, global effects, particles, crew selection
and roles, eliminated/known crew ownership, per-player HUD core, controls, and
network-packet snapshots. Object comparison includes raw fixed position,
velocity, and rotation state; do not replace those fields with their whole-pixel
mirrors.

### Weather/environment handoff

The C++ bridge captures `Game.Weather` from `RustEngineBridge::OnFrame`. That
call is the end-of-frame boundary of a successful `C4Game::Execute`: the
frame's `C4Weather::Execute` has completed, as have the later landscape,
player, script, input, rule, game-over, and sync-check work. Immediately before
the ordinary snapshot comparison, C++ supplies that post-weather/end-of-frame
payload with `lc_engine_runtime_supply_weather_snapshot`; Rust advances through
the same frame and compares the payload before its object snapshot.

The versioned, fixed-width payload carries:

- the frame's `iTick10`, `iTick35`, and `iTick1000` phases;
- live `Season`, `YearSpeed`, `SeasonDelay`, `Wind`, `TargetWind`,
  `Temperature`, `TemperatureRange`, `Climate`, `MeteoriteLevel`,
  `VolcanoLevel`, `EarthquakeLevel`, `LightningLevel`, and `NoGamma`;
- every future-driving `C4SVal` member (`Std`, `Rnd`, `Min`, and `Max`) for
  `StartSeason`, `YearSpeed`, `Climate`, `Rain`, `Lightning`, `Wind`,
  `Meteorite`, `Volcano`, and `Earthquake`;
- scenario `NoInitialize` and weather `NoGamma`, the fixed 16-byte
  precipitation material name and its length, and the one-time initial rain
  gate plus a validity flag; and
- ABI version, the fixed 240-byte structure size, and reserved transport bytes.
  The version and size are validated before any semantic field is read; the
  material length and reserved bytes are transport metadata, while the fixed
  material bytes are the compared value.

The bridge also checks Rust-only storage against the exact legacy invariants it
represents. `base_wind`, variation and bounds come from the scenario wind
driver; a nonzero wind random range implies period `2000` and update interval
`1000` (otherwise both are zero), and the update timer is zero. Season bounds
come from `StartSeason`. Temperature variation/period/phase and time-of-day/
speed are zero, and legacy weather has no separate sky color. Precipitation
strength is the bounded scenario rain base; initialized precipitation is tied
to the captured rain gate, while `NoInitialize` retains the strength directly.
This makes those extra Rust fields assertions about C++ state rather than
uncompared extensions.

`C4Weather::Init` evaluates the initial `Rain` gate before it evaluates any
per-cloud strengths. The oracle instrumentation records that already-produced
gate value at the call site and transports it later; it never calls
`Rain.Evaluate()` again, so observation consumes no synchronized RNG draw. If
`NoInitialize` skips the evaluation, the validity flag remains false.

The handoff is deliberately fail-closed and one-shot. A missing payload, an ABI
or size mismatch, a duplicate pending payload, or a payload tagged for another
frame fails validation. The normal comparison consumes exactly one same-frame
payload. A semantic mismatch reports the compared frame and the first differing
weather field, for example `frame 0: weather wind rust 0, cpp 1`.
Authoritative mode is unchanged and does not use this comparison handoff.

One determinism-critical plane remains outside the normal comparison:

- **Landscape/material state:** each engine generates and mutates its own
  landscape in a normal run, but `runtime_snapshot_mismatch` has no landscape
  checksum or byte-plane comparison. `LC_RUST_ENGINE_RUNTIME_AUTHORITATIVE`
  pushes Rust's landscape into C++ and therefore cannot prove independent
  agreement. This is tracked by clonk-org/clonk-rs#1240.

Render-surface equivalence is outside this engine-state ABI. Rendering has its
own contract in
[`../../docs/RENDERING_PARITY.md`](../../docs/RENDERING_PARITY.md).

### Separate oracle validation bridges

The pinned oracle also defines validation bridges that this engine-state ABI
does not exercise. Their Rust implementations are not present in the current
tree, so an engine shadow-diff result must not be treated as evidence for them:

- `USE_RUST_CONFIG`: clonk-org/clonk-rs#1264
- `USE_RUST_GROUP_VALIDATION`: clonk-org/clonk-rs#1265
- `USE_RUST_GUI_VALIDATION`: clonk-org/clonk-rs#1266
- `USE_RUST_PLATFORM_PATHS`: clonk-org/clonk-rs#1267

No required gate runs the live bridge: it needs a separately built oracle
checkout and is intentionally an opt-in investigation tool. `cargo xtask parity
verify` remains the reproducible primitive differential. A green result from
either harness is evidence only for the fields that harness actually compares.

## Replaying a record

To exercise the network-record path, first have the C++ engine produce a record.
Recording is enabled only when `Config.General.Record` is set, and that lives in
a config file the engine writes itself — so seed it rather than hand-authoring
one, because the key is `Record=false` inside `[General]` and a guessed path is
silently ignored (on macOS the default is
`$HOME/Library/Preferences/legacyclonk.config`, which a `HOME` override does not
create on its own).

```sh
cd <oracle>/build-validation

# 1. Let the engine write a config, then enable recording in it.
printf '/quit\n' | ./clonk "/config:$PWD/rec.config"
sed -i '' 's/^Record=false/Record=true/' rec.config

# 2. Record a run. The record lands in `Records.c4f` beside the build tree.
printf '/open %s/Tutorial.c4f/Tutorial01.c4s %s/Tyler.c4p\n' "$PWD" "$PWD" \
  | ./clonk "/config:$PWD/rec.config"

# 3. Replay it with the diff ARMED.
printf '/open %s/Records.c4f/001-Tutorial.c4s\n' "$PWD" \
  | LC_RUST_ENGINE_RUNTIME=1 ./clonk "/config:$PWD/rec.config"
```

**Check that it actually replayed.** The log looks the same as an ordinary run —
it still says `Player join`, and nothing says "replay" — so the reliable tell is
that `C4Game::Execute` skips recording for a replay (`C4Game.cpp:2935`,
`if (!C4S.Head.Replay && ...)`). Leave `Record=true` set for step 3: if
`Records.c4f` gains no `002-` entry, the engine took the replay path. If it does
gain one, the record was opened as an ordinary scenario and the run says nothing
about control or record.

## Faithfulness

`ffi.rs` is a port-forward of the pinned implementation rather than a rewrite.
`OnFrame` hands the Rust side a `SnapshotBuffer` collected from `C4Game`, and
the Rust side decides what counts as a divergence, so the ABI and normalization
rules must remain aligned with the bridge at the pinned commit. In particular:

- `ObjectSnapshot::components` uses `ComponentList` so the ABI preserves
  `C4IDList` ordering; a `HashMap` is not equivalent.
- `tracing-subscriber` is an optional dependency behind the `ffi` feature; the
  pinned tree took it the same way.

Do not silently widen, reorder, or normalize the ABI to fit current Rust types.
A bridge-translation divergence is indistinguishable from an engine divergence
once the loop runs.
