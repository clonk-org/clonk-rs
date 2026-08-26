# Shadow-diff bridge ABI

`lc_engine_ffi.h` is the C ABI the pinned oracle's `USE_RUST_ENGINE_VALIDATION`
bridge (`src/rust/RustEngineBridge.cpp`) calls, copied verbatim from the oracle
commit `7d43b47b7d789b533f32d005e64596e0a07019cd`
(`rust/include/lc_engine_ffi.h`). It is vendored here so the contract is
versioned next to the implementation instead of living only in a C++ checkout —
`crates/clonk-engine/src/ffi.rs` (feature `ffi`) is what satisfies it.

This is Phase 2 of the parity harness, clonk-org/clonk-rs#585.

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
script is not a convenience wrapper — three things have to be true at once or
the build silently uses the wrong engine, or does not configure at all:

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

### Counting events across a run — the two engines do not run equally long

The harness **stops stepping the Rust engine at its first divergence**; C++ is
the host and carries on to the end of the run. So any statistic gathered by
instrumenting both sides and counting events over a whole run measures *how long
each engine ran*, not how often it did something.

Measured on `Massif` with a per-frame probe on each side:

```
CPPCOM   distinct frames=60   last frame=60   events=360
RUSTCOM  distinct frames=1    last frame=1    events=6
```

That run diverged at frame 1, so the Rust engine executed one frame against
C++'s sixty. A 60:1 ratio in the raw counts is the harness, not the engine.
This produced a published-then-retracted "the port turns animals 5-10x less
often" finding (clonk-org/clonk-rs#1123) — the shape of the error is a ratio
extreme enough that it should prompt a check of the frame spans first.

To count anything across a run, either:

- use a scenario that reports **no** divergence, so both engines execute the
  same frames — the tutorials qualify; or
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
path** for `planet/System.c4g` (`crates/clonk-engine/src/ffi.rs:2609`), while
C++ resolves it from its own working directory. Pass an absolute scenario path
inside this repository and the two engines silently load **different** system
groups: the port picks up `planet/System.c4g` from the checkout, whose
port-authored `#appendto` scripts (`BirdFlight.c`, `EkeAirbikeSteering.c`,
`GatherTask.c`, `FoWReveal.c`, ... — the set the compatibility profile
withholds, `crates/clonk-app/src/compat_readiness.rs`) never run under the
oracle.

The diff then reports content that only one side was given. It looks like a
port defect and is not one: on `Races.c4f/Goldrace.c4s` it surfaced as

```
frame 1: ... effects rust [BirdFlight(prio 1 int 1 t 1)], cpp []
```

with the velocity and command-direction differences that `BirdFlight.c`'s own
`SetXDir`/`SetCommand` calls produce. Scenarios that pull in no port append
(the tutorials) are unaffected, so a sweep can be half valid.

Pass the scenario **through the build tree** instead, so the ancestor walk finds
the group C++ is using:

```sh
cd <oracle>/build-validation
printf '/open %s/Races.c4f/Goldrace.c4s %s/Tyler.c4p\n' "$PWD" "$PWD" \
  | LC_RUST_ENGINE_RUNTIME=1 ./clonk
```

Comparing an install-root-sensitive scenario any other way is not a parity
result.

## What is and is not wired

The loop runs. On Tutorial01 with a fresh player it reports exactly one
divergence, reproducible byte-for-byte:

```
frame 1: object 90 energy rust 55000, cpp 50000
```

That one is **not** a simulation defect — it is clonk-org/clonk-rs#1049, the
bridge having no field for the C++ fair-crew game parameters, so the Rust
runtime keeps its own defaults and promotes a rank-0 crew member to rank 1.
Aligning the parameter drops the count to zero, so the port matches C++ across
every object, effect, particle, crew and control the bridge compares, for the
whole scenario. Any C++ game parameter absent from the header is a false
positive of exactly this shape; check the header before blaming the port.

Still open:

- **No gate runs this.** It needs an oracle checkout and builds only where the
  oracle builds, so it is a local investigation tool, not CI coverage.
- All four scenario classes clonk-org/clonk-rs#585 asks for have now been swept
  — movement (`Goldrace`, `Skyrace`), landscape (`Greed`, `Canyon`, `Massif`),
  script/effect (`Tutorial03/05/07/09/10`) and network-record (see below). Each
  starts and reports a first divergence.
- **Weather is not compared, and cannot be**
  (clonk-org/clonk-rs#1083). `lc_engine_runtime_compare_snapshot` takes no
  environment parameters, so wind, season and climate never reach the
  comparison; the header carries `LcEngineRuntimeEnvironmentState` only for
  `lc_engine_runtime_export_environment`, which reads state *out of* the Rust
  runtime. Extending the compared set does not help — the values never arrive,
  and adding a parameter would break link compatibility with the bridge the
  oracle builds. **A clean diff says nothing about weather**, which is on the
  determinism-critical list; a weather divergence reaches the report only if
  some object script happens to branch on it, as in
  clonk-org/clonk-rs#1077. The same structural gap as
  clonk-org/clonk-rs#1049.
- The other bridges (`USE_RUST_CONFIG`, `USE_RUST_GROUP_VALIDATION`,
  `USE_RUST_GUI_VALIDATION`, `USE_RUST_PLATFORM_PATHS`) link their own `lc_*`
  libraries and need the `ffi` modules of the other crates, which are not
  restored. They are off by default, so engine validation does not wait on them.

So `parity verify` remains the primitive-section differential, and a green run
of it is still not evidence of full-scenario parity.

## Replaying a record

The network-record class needs a record to replay, and nothing in the tree
produces one. The engine records only when `Config.General.Record` is set, and
that lives in a config file the engine writes itself — so seed it rather than
hand-authoring one, because the key is `Record=false` inside `[General]` and a
guessed path is silently ignored (on macOS the default is
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

On `001-Tutorial` the replay reports one divergence, byte-identical across
repeated runs:

```
frame 1: synced RNG ledger diverged:
  rust hold 2460695438 count 501, cpp hold 3481787565 count 506
```

Unlike the landscape sweeps, this one is stable rather than intermittent, which
makes it the better starting point for the frame-1 draw-count family
(clonk-org/clonk-rs#1139 is the other member).

## Faithfulness

`ffi.rs` is a port-forward of the pinned implementation rather than a rewrite,
deliberately: `OnFrame` hands the Rust side a complete `SnapshotBuffer`
collected from `C4Game`, and the Rust side decides what counts as a divergence,
so the comparison semantics are only correct if they match the ones the bridge
was written against. Reconciling drift against the current engine took two
changes:

- `ObjectSnapshot::components` is a `ComponentList` now (`C4IDList` ordering)
  rather than a `HashMap`.
- `tracing-subscriber` is an optional dependency behind the `ffi` feature; the
  pinned tree took it the same way.

Keep it that way. A divergence introduced here is indistinguishable from an
engine divergence when the loop finally runs.
