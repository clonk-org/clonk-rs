# LegacyClonk Rust Port — Status & GAP LIST

> Living document. Last updated 2026-06-05. The C++ engine in `../src/` is the
> **golden oracle**; parity = bit-for-bit match on simulation state. This file
> tracks every divergence from that goal.

## State: broadly scaffolded, not yet lockstep-parity-capable

The port reproduces the engine's *shape* (structs, enums, command dispatch, FFI,
~1217 tests) and the two original headline determinism breaks — fixed-point math
and the RNG — are now correct for the **currently ported paths**. Lockstep parity
is still blocked by downstream stateful systems (`CrossCheck`, script-values,
particles, materials) and residual movement systems outside the now-ported
per-pixel contact-loop slices.

### The two foundational breaks — current status

1. **Fixed-point math — DONE for current paths.** `C4Fixed`/`FixedVec2` in
   `math.rs` (16.16, `itofix`/`fixtoi`, Sin/Cos). Live objects carry private fixed
   position/velocity/rotation; the motion step accumulates fixed velocity and
   projects to integer pixels. Script velocity surface (`SetXDir`/`GetXDir`, prec
   10) and rotation (`SetRDir`/`GetRDir`, `fix_r += rdir*5`, half-circle wrap,
   `C4Movement.cpp:373-436`) carry true sub-pixel. Snapshot + JSON save/load
   preserve raw `C4Fixed` (emitted only when beyond `fixtoi`). Raw `fix_x/fix_y/
   xdir/ydir/fix_r/rdir` cross the C ABI (`LcEngine*`/`RustEngineBridge.cpp`).
   `Rotate=` → `Definition::rotateable`, `OCF_Rotate`, non-rotateable zeroing,
   `Rotateable>1` clamp. **Theme C complete:** gravity, friction, collision,
   walk/swim/float/scale/hangle/dig accel, push/pull/fight/lift, wind, and
   physics clamping all write authoritative `fixed_velocity`;
   `sync_fixed_velocity_components_from_public` deleted. **Open:** residual
   movement systems outside item 4 (notably rotated/update-lifetime solid masks).

2. **RNG — DONE for current callers.** `LcgRng` is the C++ LCG
   (`RandomHold*214013 + 2531011; (RandomHold>>16)%range`, `C4Random.h:52-60`) with
   `RandomHold`/`RandomCount`, `FixedRandom`, `SeededRandom`, `Randomize3`/`Rnd3`
   (500-entry buffer), serialized with state. Engine seeds `FixedRandom(seed);
   Randomize3();`. The old ChaCha proptest is replaced by an LCG parity test.
   **Open:** `SafeRandom` consumers, full network sync-check integration.

## Parity harnesses

- **`cargo xtask parity verify`** (also `cargo test -p lc-engine
  parity_differential_matches_cpp_golden`) — the **real C++↔Rust differential**:
  diffs `C4Fixed` math, LCG (`Random`/`RandomCount` incl. range-0, `Randomize3`/
  `Rnd3`), `Sin`/`Cos`, per-frame sub-pixel accumulation (`fix += dir`,
  `ydir += gravity`), the `C4Value` map-key hash (`script_value_hash`), and the
  `C4ScriptCnvMap` conversion table + `ConvertTo` dispatch (`script_value_convert`)
  byte-for-byte against a golden from the real engine
  (`src/Fixed.{h,cpp}`, `src/C4Random.h`, `src/C4Value.cpp`). Reports first mismatch; negative
  control confirms it fails on a corrupted golden. **Gates Theme C.** Regenerate:
  `cargo xtask parity record`. See `parity/README.md`.
- **`cargo xtask engine-snapshots verify`** — Rust-vs-Rust determinism
  *regression* check only (NOT a parity check).
- **Phase 2 (OPEN)** — live full-scenario shadow-diff. The per-pixel collision
  loop, landscape, and materials are uncovered. The `USE_RUST_ENGINE_VALIDATION`
  bridge compiles (`ffi` feature) and carries raw `C4Fixed` state across the ABI
  but still needs shadow execution + per-field divergence reporting.

## Gates

- **`cargo test --workspace`: GREEN** (~1217 pass, cargo exit 0). Known flaky:
  `lc-network session::tests::control_sync_and_reconnect_smoke` — passes in
  isolation/clean runs; can fail under heavy parallel load when a departing
  client's closing socket surfaces a transient `HostEvent::TransportError` that
  `wait_for_host_ready` (`session.rs:1264`) treats as fatal. Network-I/O churn,
  not a determinism issue. Fix: have `wait_for_*` tolerate transport errors from
  departing clients (as they already skip `ClientLeft`).
- **`cargo clippy`: NOT clean** (~275 lines). 7 `not_unsafe_ptr_arg_deref` errors
  (FFI entry points deref raw pointers without `unsafe` — fix first, mark them
  `unsafe fn`, no ABI change); ~30 `too_many_arguments`; ~6 `type_complexity`;
  ~230 auto-fixable style lints. Deferred because several clippy
  "simplifications" aren't proven behavior-preserving in determinism-critical
  `lc-engine`/`lc-script`. Order: (1) FFI `unsafe`; (2) `#[allow]` on engine/FFI
  signatures; (3) bulk `--fix` on non-critical crates + full test; (4)
  engine/script by hand, each verified.
- **Graphical parity: NOT achieved** (presentation layer). Asset loading/2D blit
  matches, but menu chrome, the GL 3D scenario book, and in-game rendering all
  diverge — `lc-graphics` is ~25% (per-pixel blit only; no transforms/GL/shaders/
  landscape rendering). Live in-game capture is blocked (x86_64 Rosetta C++ build
  had no linked scenarios; `lc-app` is a non-bundled winit binary computer-use
  can't drive).

## Known accepted divergence

- **No general comma operator in C++.** Rust's `lc-script` `parse_comma` accepts
  comma sequences in any expression context; C++ only allows them inside `return
  (...)` via `multi_params_hack` (`C4AulParse.cpp:2069`); `,` is absent from
  `C4ScriptOpMap`. Rust only *accepts* more, and real content uses the legal
  `return (...)` form, so risk is low — but should be narrowed to C++ semantics.

---

## Determinism-Critical GAP LIST

Sorted worst-first (stub → partial; within partial, by severity). 24 of 26
audited subsystems are determinism-critical.

| Subsystem | Coverage | Key Parity Risk | Rust Location |
|---|---|---|---|
| **script-values** | partial | **Done:** `C4ScriptCnvMap` 81-cell conversion table + `ConvertTo` dispatch (differential-locked `script_value_convert`); boost `hashCombine` + libc++ `std::hash<C4Value>` map-key hash (`script_value_hash`); typed `C4V_C4Object` identity as `Value::Object(u64)` through VM equality/truthiness/type/hash, FFI, host `this`, object-returning helpers, and effect vars; recursive FFI marshalling for `C4Id`/`Array`/`Proplist` through `LcScriptValueKind` + `LcScriptMapEntry`; VM-visible reference semantics for `&` params, `func &` returns, Local/Var slots, and array/map element refs. **Open:** `GuessType()` data-nonzero path (unreachable in the eager Rust value model — types are always known), C++ string-table interning/refcounts. Save/load + net sync still incomplete. | `lc-script/src/value.rs`, `lc-script/src/vm.rs`, `lc-engine/src/compat.rs` |
| **particles** | **stub (420 vs 808)** | `ActiveParticle::tick()` is `pos+=vel; life-=1` only — no gravity, wind, collision, alpha fade, animation, or `SafeRandom` variation. `Cast()`, `Push()`, all `fx*` procs, `C4ParticleDef::Load()` absent. Any particle scenario desyncs RNG + state. | `lib.rs:669-860,12136-12547`; `compat.rs:8355-8539` |
| **findobject-ocf** | **stub (35%, 280 vs 956)** | No `CreateByValue()` condition-tree factory (nested `C4FO_And/Or/Not` fail silently), no `C4SortObject` (`Random/Speed/Mass/Value` unsorted → desync), no `C4FO_AtRect`/`UseShapes()` beyond the legacy rectangle path. | `compat.rs:1667-1835,6784-6931`; `ocf.rs` |
| **movement-physics** | partial | Central motion accumulates sub-pixel fixed velocity, steps x/y per pixel, consumes DefCore/current owned vertices and `StretchGrowth`/Jolt construction shape updates, runs shape/vertex `ContactCheck`, dispatches ContactLeft/Right/Top/Bottom and Hit/Hit2/Hit3 in C++ order, applies redirect/friction, clamps landscape and layer `TargetBounds`, overlays active DefCore solid masks as `MCVehic` contact density with sprite-alpha bitmap transparency, supports `Shape.Attach`, forces Jump/default on attach loss, rolls back per-degree rotation, and uses C++ density levels for background/material/vehicle contact checks (`C4M_Background=0`, material `Density`, closed side bounds and solid masks `C4M_Vehicle=100`). **Missing:** rotated solid-mask put-buffer semantics, `SetSolidMask`/solid-mask update lifetime, attached-object pushback. | `lib.rs`, `landscape.rs` |
| **objects-core** | partial | `AtObject()` exists; collection auto-check is sector-backed. Full `CrossCheck()` (919-LOC inter-object loop: Tick3/5/10/35 incineration/fight/collection/hit-damage) absent. OCF computes ~8 vs ~30 checks (`ocf.rs:46-76` vs `lib.rs:527-666`); object list is `Vec` vs category/ID-sorted. | `lib.rs`, `ocf.rs`, `compat.rs` |
| **game-control-record** | partial (35%) | No varint frame-delta encoding (`C4Record.cpp:243-264`) — JSON snapshots instead. No `ControlRate`/`ControlTick` throttle, no `SyncRate` sync-check state machine, no `+37` end-marker (`:196`), no `Prepare()` pre-validation. | `lib.rs`, `control.rs`, `record.rs`, `ffi.rs` |
| **material** | partial (40%) | Reaction *execution* partly missing: `mrfInsertCheck` splash/slide physics (`C4Material.cpp:570-604`) and script reactions unported. `MaterialReactionKind` classifies; mass-mover path now executes corrode/poof (see item 9). No full `ExtractMaterial/InsertMaterial` semantics. | `lc-engine/material.rs`, `lc-resources/material.rs` |
| **pxs-massmover** | partial (296 vs 691) | Down/L/R corrosion, two-pass reverse exec, `Random(10)` before `Rnd3()` transfer order, and `LandscapeInsertThrust` plumbing landed. Remaining: exact `CreatePtr` fixed-slot reuse, richer per-pixel thrust/insert, step-by-step `_PathFree` (vs `first_collision_on_line`). | `mass_mover.rs`; `lib.rs:12211+` |
| **landscape** | partial (25%) | Batch `apply_temperature_conversions` vs C++ incremental `ExecuteScan/DoScan` with `ScanX` cursor (scan order desyncs). No `PRETTY_TEMP_CONV`, no map creation (`ChunkyRandom`/`MapToLandscape`), no `DigFree/BlastFree`, no pixel ops, no Save/Load. Liquid model is segment- vs pixel-based. | `landscape.rs`, `material.rs` |
| **effects** | partial (35%, 195 vs 921) | Builtin fire effect (300+ LOC) + helper effects (Splash/Smoke/Explosion/BubbleOut) missing. No `Check()` priority-conflict, no `TempRemove/TempReadd`. `advance_tick()` uses saturation vs modulo `iTime % iIntervall` → timing drift. Dispatch infra exists but never invoked for builtins. | `effect.rs`, `lib.rs:5175+,5272+` |
| **commands** | partial (55%) | AI determinism: MoveTo lacks Jump/Flight/Swim control; Get missing `Random(15)-7` offset (`C4Command.cpp:1290`) + side-jump (`:1272`). Tick2/5/35 throttling absent → continuous exec breaks tick-sync. Scale/Hangle let-go thresholds missing. | `command.rs` |
| **players-crew-teams** | partial (770 vs 5747) | Wealth clamp divergence (10k `adjust` vs 100k `set`, `player.rs:344,349`). Team home-base production sync missing (`C4RULE_TeamHombase`, `C4Player.cpp:1637`) → players advance independently. No `CheckElimination`, no hostility model, asset value is a caller stub. | `player.rs` |
| **definitions-id** | partial (4319 LOC) | `CrossMapActMap()` procedure→numeric NOT done (`definition.rs:35` keeps actions as strings) → runtime action behavior diverges. No `GetComponents` override, no `CalcDefValue()`. C4ID byte extraction differs. Many DefCore flags unparsed. | `lc-resources/definition.rs`; `compat.rs` |
| **weather-sky** | partial (65%) | All updates every tick vs Tick10/35/1000 gating → wrong seasonal/wind signature. Meteor/earthquake/volcano launching unimplemented (`lib.rs:7811` does lightning only). `Random(60)`/`Random(100)` replaced by `&&`-chained `gen_range`. Sky parallax `wind/100` vs FIXED100. | `lib.rs`, `sky.rs`, `compat.rs` |
| **config-info** | partial (49%) | `GetAName()` random name uses `Random()` — no Rust equivalent. No `PromotionUpdate()`. `RandomSeed = time(nullptr)` (`:425`) ties determinism to wall-clock. Default init differs (locale, control prefs). | `lc-core/std_config.rs`, `lc-app/settings.rs`, `scenario.rs` |
| **resources-groups** | partial (43%) | Read-only: no group write/create (`Save/Add/Move/Delete`), no gzip, no CRC32 at open (`C4Group.cpp:791`). Path normalization (Rust `components()`) and WalkDir order may differ from C++ `DirectoryIterator`. | `group.rs`, `scenario.rs` |
| **sectors-regions-rect** | partial | `C4LSectors`/`C4LArea` done in `sector.rs`: 50×50 point/shape lists, `SectorAt()` out-sector behavior, `C4LArea::Next()` row/pitch iteration with clipped edge cases; membership rebuilds on all current object-lifecycle paths. Consumers wired: `AtObject()`, bounded `FindObject`/`FindObjects`/`ObjectCount`, collection cross-check. **Open:** separate `C4Region` UI/input rectangles. | `sector.rs`, `lib.rs`, `compat.rs` |
| **pathfinder-transfer** | **full** but buggy | Ray exec order: C++ LIFO prepend (newest-first, `C4PathFinder.cpp:655`) vs Rust `insert(0,…)` + ordered iteration (FIFO) → different waypoint sequences. Zone lookup `sorted_by_key(owner)` vs C++ insertion order. | `pathfinder.rs`, `transfer.rs` |

## Non-Determinism-Critical (presentation-layer) Gaps

Flagged critical in the audit but in practice their *visual* output diverges
while simulation impact is secondary.

| Subsystem | Coverage | Key Risk | Rust Location |
|---|---|---|---|
| **graphics** | partial (25%, 1276 vs 5045) | No transforms/rotation (`CBltTransform`), texture mgmt/GL, shaders (`StdGL.cpp`), patterns, gamma, or landscape rendering. `blit_region` per-pixel only. | `lc-graphics/src` |
| **audio** | partial (35%) | Panning math differs (SDL 0–192 vs gain 0–1, `mixer.rs:775`). `C4SoundSystem`/`C4MusicSystem` high-level layers absent (object attach, falloff, `MaxSoundInstances`, `IsNear`, wildcard). `SetPosition` declared, never implemented. | `lc-audio/src` |
| **gui-menus** | partial (3237 vs 4467) | No rendering (`DrawElement`), `InitLocation` layout, text progression, hotkey markup, or portraits. Column wrap-around → modular arithmetic (diverges when `ItemCount % Columns != 0`). | `lc-app/object_menu.rs`, `ingame_menu.rs`, `lc-gui` |
| **startup-launcher** | partial (~60%) | Player-selection dialog missing (stub msg, `main.rs:6515`). No file validation, update check, first-start UX, or fades. Startup folded into game loop vs separate modal. | `lc-frontend/startup_*.rs`, `lc-app/main.rs` |
| **network** | partial (3590 vs 8379) | Control-coordination half is determinism-critical (above). Missing: password auth (`C4Network2.cpp:281-345`), voting, league, client status (NCS_*), save/restore join-data, protocol negotiation (`PROTOCOL_VERSION=1` hardcoded). Client-ID signed/unsigned mismatch. | `lc-network/src` |

## Silent Stubs Inventory

Functions that return plausibly but skip core C++ logic — the landmines that pass
review and desync in production. (Resolved subsystems — fixed-point, RNG,
sectors — omitted; see status above.)

**script-vm-aul** — `AssignmentTarget::{LocalSlot,VarSlot,EffectSlot,MethodSlot,
FunctionCall}` (`vm.rs:1072-1158`) string-mangled (`__local_/__var_`) keys, not
array indices/dispatch; `forward_rest` variadic TODO (`vm.rs:464,484`);
`invoke_host_function` (`:200-222`) no param/return validation; call dispatch
(`:493-496`) by-name, no inheritance/overload chain. (`Expr::This`, div-by-zero,
slots, stack limit — fixed; see Completed.)

**script-values** — `C4ScriptCnvMap`/`ConvertTo`, the map-key hash, typed
`C4V_C4Object` identity, VM-visible references for `&` params/returns and
container lvalues, and C4Id/Array/Proplist/Object FFI marshalling are no longer
stubs. Still stubbed: strings are owned Rust strings rather than C++ string-table
entries.

**objects-core** — `reset_action_to_default` (`lib.rs:10629`) no `SetActionByName`
enforcement; `apply_*_procedure` (`:10244+`) no ObjectCom transitions;
`compute_ocf` (`:3768`) no `ContactCheck`, no velocity HitSpeed, wrong
entrance-rotated check.

**movement-physics** — current slice has per-pixel x/y, `ContactCheck`,
`RedirectForce`, friction, `Shape.Attach`, border bounds, per-degree rotation
rollback, current-model density provider parity for background/material/vehicle
contact density, Contact*/Hit callbacks, and C++ `UpdateShape` construction
paths for definition/owned vertices plus `StretchGrowth`. Layer bounds now use
the C++ `TargetBounds` clamp path; active DefCore solid masks are parsed from
`SolidMask=`, sampled as `MCVehic`, and respect sprite-alpha transparency during
contact/attach/rotation checks. **Open:** rotated solid-mask put-buffer
semantics, `SetSolidMask`/solid-mask update lifetime, attached-object pushback.

**landscape** — `insert_material_at` (`:872-898`) no pathfinding/velocity/
collision; `remove_material_at` (`:900-919`) no extraction/spawn; `incinerate_at`
(`:921-926`) returns early; `blast_circle` (`:697-813`) no BlastFree layers/grade.

**material** — `MaterialReactionKind::{Convert,Poof,Corrode,Incinerate,Insert}`
(`material.rs:110-121`) variants defined; `reaction()`/`custom_reaction()`
(`:705-767`) classify only — non-mass-mover callers must implement physics.

**pxs-massmover** — `find_liquid_target()` (`mass_mover.rs:254-291`) no reaction
callbacks during slide; `tick_material_particles()` (`lib.rs:12259`)
`first_collision_on_line` skips step-by-step reactions.

**effects** — `advance_tick` (`effect.rs:86`) timer bool only; `set_var/var`
(`:100-112`) no callbacks; dispatch infra (`lib.rs:5175+,5272+`) never invoked for
builtin Fire.

**commands** — `MoveToState::step()` (`command.rs:6257-6307`) no flight/jump
control; `TransferState` no Tick5 throttle; `RetryState` (`:8890+`) decrement
only; `HomeState` no base-owner check; `FollowState` no Push/Ungrab; `PutState` no
failure-suppression flags.

**players-crew-teams** — `set_crew()/sort_crew()` (`player.rs:467-474,611-613`) by
ObjectId, no validation; `update_asset_value()` (`:386-395`) accepts pre-computed
value; `set_home_base_*()` (`:476-496`) no team sync; `advance_home_base_
production()` (`:558-589`) no team logic; `set_status()` (`:307-317`) no
evacuation/callbacks; ctor no Hostility init.

**definitions-id** — `Definition::load()` (`definition.rs:35`) no `CrossMapActMap`;
`parse_act_map()` (`:486-619`) no procedure→numeric, `next_action` stays string.

**game-control-record** — `Recorder::record` (`record.rs:69`) Vec push, no binary;
`Recording::to_writer` (`:34`) JSON only; `Playback::validate_snapshot`
(`:108-125`) post-hoc not streaming; `Game::tick` (`lib.rs:7837`) no
control/record lifecycle.

**findobject-ocf** — `find_object` (`compat.rs:6784`) linear/closest, no factory;
`find_object_closest`/`collect_closest_matches` (`:6826,6911`) distance sort, no
SortObject; `ocf compute` (`ocf.rs:46`) no dynamic updates.

**weather-sky** — `tick_weather_events` (`lib.rs:7811`) lightning only.

**particles** — `create_particle()` (`compat.rs:8355-8485`) registers, no exec;
`apply_particle_commands()` (`lib.rs:12163-12209`) add/remove only;
`tick_particles()` (`:12540-12548`) no environment interaction.
`CastParticles`/`CastBackParticles`/`PushParticles` unregistered (calling them
errors) → both particles and the `Random`/`RandomCount` stream desync.

**config-info** — `Audio/DisplayOptions::apply_config()` (`settings.rs:60-105,
331-371`) load subset, skip validation; `Config::get_bool()` (`std_config.rs:134`)
`true/1/yes` only; `ScenarioObjectives::from_legacy_game()` (`scenario.rs:186-217`)
create/clear only.

**presentation** — audio `SetPosition` (`mixer.rs:313-316`) declared only;
graphics `Surface::blit/blit_region` (`surface.rs:228-323`) per-pixel only;
`Color::blend_over` (`color.rs:36-57`) basic alpha; gui `ObjectMenuState::render/
handle_command` (`object_menu.rs:427-567`) backdrop only, returns `None`;
`IngameMenuState` no rendering; startup `PlayerSelection` (`main.rs:6515`) stub
text; resources `Group` no write/create.

**network** — `broadcast_packet` (`session.rs:705-712`) treats Queue/Sync/Decide
identically; Request handler (`:1387-1399`) no tick-range/rate limit;
`handle_accept` (`:469-548`) no password; `record_packet` (`resync.rs:29-34`) no
order/retransmit validation; `broadcast_exec_sync` (`:738-749`) no host-frozen
check.

---

## Top 15 Action Items

Determinism-critical first; items 1–3 gate almost everything. Status inline.

1. **PARTIAL** — `C4Fixed` type + replace `Vector2`. Core done (see foundational
   break #1 above). Remaining = residual non-item-4 movement systems plus other
   stateful subsystems.
2. **DONE** (lc-engine) — Replace ChaCha8 with the C++ LCG. (Break #2 above.)
3. **DONE** (current callers) — `Randomize3`/`Rnd3` circular buffer.
4. **DONE (requested contact-loop slices)** — Per-pixel stepping movement loop
   with sub-pixel accumulation.
   Done for current density model: DefCore vertices/`Attach`, shape/vertex
   `ContactCheck`, `RedirectForce`+friction, `BorderBound` clamp, `Shape.Attach`,
   Jump/default on attach loss, per-degree rotation rollback, and
   background/material/vehicle `GetDensity` levels for contact checks
   (`vehicle_density_boundary_below_contact_density_allows_motion_like_cpp`
   mirrors `C4Movement.cpp:260-281`, `C4Shape.cpp:389`,
   `C4Landscape.h:144-150`, `C4Material.h:200`), plus layer `TargetBounds`
   (`layer_border_bound_clamps_horizontal_target_like_cpp` mirrors
   `C4Movement.cpp:185-196` and `C4Movement.cpp:147-155`), and active
   solid-mask vehicle-density contact
   (`solid_mask_vehicle_density_blocks_per_pixel_contact_like_cpp` mirrors
   `C4Movement.cpp:260-282`, `C4SolidMask.cpp:66-104`, `C4Material.h:200`,
   `C4Movement.cpp:277`; resource parsing covered by
   `parse_def_core_solid_mask_target_rect`), solid-mask bitmap transparency
   (`solid_mask_transparent_bitmap_pixel_allows_motion_like_cpp` mirrors
   `C4SolidMask.cpp:80-104,401-411`), Contact*/Hit callback ordering, and full
   `UpdateShape` construction shape refresh for definition vertices, owned
   vertices across restore, and `StretchGrowth`
   (`construction_jolt_updates_vertices_and_preserves_bottom_like_cpp`,
   `construction_owned_vertices_survive_restore_like_cpp`,
   `construction_stretch_growth_scales_x_axis_like_cpp`). Residual movement gap
   tracked above: rotated solid-mask put-buffer/update lifetime and attached
   object pushback.
5. **DONE** (infra + current consumers) — `C4LSectors`/`C4LArea` (see GAP LIST).
   Open: separate `C4Region` UI rectangles.
6. **TODO** — Remaining `CrossCheck()` inter-object loop (919 LOC): Tick3/5/10/35
   scheduling, `RejectFight`/`CatchBlow`, hit energy/fling, contained-object fight,
   contact incineration, exact post-callback recheck.
7. **PARTIAL** — `script-values`. **Done:** `C4ScriptCnvMap` 81-cell table +
   `ConvertTo` dispatch (`C4Value.cpp:431-598`; differential-locked
   `script_value_convert` — 81-cell grid + per-(value,target,#strict) result);
   boost `hashCombine` + `std::hash<C4Value>` (`:923-1029`; `script_value_hash`);
   recursive C4Id/Array/Proplist FFI marshalling in `rust_value_to_lc()` +
   `lc_value_to_rust()` (`ffi.rs`); VM-visible reference semantics for `&`
   params, `func &` returns, Local/Var slots, and array/map element refs; typed
   `C4V_C4Object` identity as `Value::Object(u64)` through VM/FFI/host helpers.
   **Remaining:** C++ string-table interning/refcounts, full save/load + net
   sync wiring.
8. **DONE** — C4Script VM operator parity + `Expr::This` + Var/Local slots (see
   Completed).
9. **PARTIAL** — Material reaction execution. Mass-mover path runs
   `MaterialReactionKind` with event masks, `mrfCorrode` `Random(100)` ordering +
   effect RNG, `mrfPoof` `Rnd3()`, shared `ExtractMaterial`/`InsertMaterial`.
   Remaining: full `mrfInsertCheck` splash (8× damping) + slide for PXS
   (`C4Material.cpp:570-604`) and script reactions.
10. **PARTIAL** — Mass-mover parity. Down/L/R corrosion, two-pass reverse exec,
    immediate spawned-mover exec, `LandscapeInsertThrust` plumbing, `Random(10)`
    before `Rnd3()` order landed. Remaining: exact `CreatePtr` slot reuse, richer
    per-pixel thrust/insert.
11. **TODO** — `CrossMapActMap()` in definition loading (`definition.rs:35,
    486-619`): procedure→numeric, `next_action`→indices per `C4Def.cpp:773-799`.
12. **TODO** — Full particle physics processor: `fxStdExec` (`C4Particles.cpp:
    614-697`) gravity/wind+friction/alpha/collision/animation; `Cast()`, `Push()`,
    `fx*` maps, `C4ParticleDef::Load()`, `SafeRandom`.
13. **TODO** — Frame-tick gating: Tick10/35/1000 (weather), Tick2/5/35 (commands),
    `ControlRate`/`ControlTick`/`SyncRate` modulo (`ffi.rs:451-489`); + meteor/
    earthquake/volcano with `Random(60)`/`Random(100)`.
14. **TODO** — Sync-check state machine + binary record: `DoSync`/`SyncRate`,
    queue + `RemoveOldSyncChecks` (`C4GameControl.cpp:441-468`), varint frame-diff
    (`C4Record.cpp:243-264`), `+37` end-marker (`:196`).
15. **TODO** — `FindObject` condition-tree factory (`CreateByValue()`,
    `C4FindObject.cpp:37-162`) + `C4SortObject` (`Random/Speed/Mass/Value/Func`),
    full `C4FO_AtRect`/`UseShapes()`, deterministic sorted iteration; fix
    `ocf.rs:46` dynamic state.

---

## Completed (changelog)

**Script value object identity — item 7 (partial) (2026-06-05).** Replaced the
old object-reference proplist shim as the primary representation with typed
object values:
- Added `Value::Object(u64)` for `C4V_C4Object`, including VM equality,
  truthiness, `type_name() == "object"`, `c4v_type()`, and deterministic
  `std::hash<C4Value>` parity hashing under the object type tag.
- Extended FFI with `LcScriptValueKind::Object` and an object-id payload, while
  preserving the existing Array/Proplist marshalling fields.
- Changed engine object-returning helpers (`object_reference_value`, contents,
  find-object/content helpers, action targets, `Contained`, etc.) to return typed
  objects; host parsers accept typed objects and the legacy `{id = ...}` proplist
  shim for compatibility.
- Effect variables now preserve object identity as `EffectVarValue::Object(u64)`
  instead of collapsing objects into lossy integers.
- Covered by `this` identity tests, FFI nested round trips, object-returning host
  helper tests, effect-var object round trips, full `lc-script --features ffi`,
  and full `lc-engine`.

**Script value references — item 7 (partial) (2026-06-05).** Replaced the VM's
synthetic `__funcref_*` placeholder with internal lvalue handles:
- Environment bindings now use shared cells, so reference parameters alias caller
  variables/slots/elements instead of receiving value copies.
- `func &` returns now return an internal lvalue for variables, `Local()`/`Var()`,
  array indexes, map/proplist properties, and chained reference-returning calls.
- Nested script calls share object-local named storage and `Local(n)` slots during
  a call session, matching the C++ object-local shape that `C4Value` references
  target.
- `type_name()` now reports `"map"` for `Value::Proplist`, matching
  `GetC4VName(C4V_Map)`.
- Covered by runtime tests for `&` param mutation, `func &` Local-slot mutation,
  and array/map element mutation through both reference paths.

**Script value FFI Array/Proplist marshalling — item 7 (partial) (2026-06-05).**
Extended the C ABI value shape beyond Nil/Int/Bool/String:
- Added `LcScriptValueKind::{C4Id, Array, Proplist}` and `LcScriptMapEntry`;
  `rust_value_to_lc()` recursively exports nested arrays/proplists with sorted
  proplist keys for deterministic ABI order.
- `lc_value_to_rust()` imports the same nested structures, and
  `lc_script_value_free()` now recursively frees strings, arrays, map-entry keys,
  and nested values.
- Covered by focused ffi-feature roundtrip tests for nested arrays/proplists,
  empty containers, and C4Id values.

**`C4ScriptCnvMap` conversion table — item 7 (partial) (2026-06-04).** Ported the
9×9 type-conversion table and `ConvertTo` dispatch from `C4Value.cpp:431-598`:
- `C4VType` (the `C4V_Type` tag, C4Value.h:37-54), `CnvFn` (the six converter
  classes from the C4Value.cpp:481-486 macros; `Warn` derived since it is a pure
  function of the class), and the table transcribed cell-for-cell into
  `value.rs` (`C4_SCRIPT_CNV_MAP`).
- `Value::c4v_type()` (the eager Rust model only maps `Nil`→`C4V_Any`;
  `C4V_pC4Value` remains internal VM reference state) and
  `Value::convert_to(to, strict)` mirroring `ConvertTo`: `CnvOK`→true,
  `FnCnvError`→false,
  `FnCnvDirectOld`→`!strict`, `FnCnvInt2Id`→int in `0..=9999`, `FnCnvGuess`→true
  (nil "is every type except a reference"; the Game-dependent `GuessType`
  data-nonzero path is unreachable because types are always known),
  `FnCnvDeref`→unreachable (no Rust reference value).
- Locked by a new differential section `script_value_convert` (oracle
  `oracle_main.cpp` transcribes the table independently; golden regenerated):
  the full 81-cell classification grid + 216 per-(value, target type, #strict)
  `ConvertTo` results. Negative control verified (corrupting one cell fails with
  `cell [1][3]`). RED→GREEN per increment; `cargo test --workspace` +
  `cargo clippy -p lc-script -p lc-engine` green.

**Theme C — fixed precision through physics (2026-06-04).** All ported physics
paths write authoritative `fixed_velocity`, integer mirror derived via `fixtoi`;
`sync_fixed_velocity_components_from_public` deleted. Covered: gravity
(`ydir += GravAccel`, `FIXED100(20)/5` = raw 13107, `C4Movement.cpp:643`),
friction (`ContactVtxFriction`, `:569`), collision resolution (sub-pixel retained/
zeroed at fixed level, `:266-282`), walk/swim/float/scale/hangle/dig accel
(`C4Object.cpp:4776`, accels are `const C4Fixed`), push/pull/fight/lift, wind
(`FIXED100(iWind)`). Gated by `parity verify` + targeted tests.

**C4Script VM operator parity — item 8 (2026-06-04).** Bit-exact fixes from
reading `C4AulExec.cpp` directly (each RED→GREEN, full suite green):
- `x/0`, `x%0` → `Int(0)` (`:504-526`, was: threw).
- `&&`/`||` return the surviving **operand**, not a bool (`:999-1021`).
- Binary/unary int ops coerce `nil→0`, `bool→0/1` via `Value::as_c4_int()`
  (the `_getInt()` mirror; `None` for String/Array/Proplist — pointer data);
  `-` uses `wrapping_neg`, div/mod use `wrapping_div/rem` (`:460-470`, `C4Value.h:170`).
- `this` → current object: VM threads a host `this` (`Vm::with_this`); all 8
  object-context call sites pass `object_reference_value(object_id)` (was `Nil`).
- Non-nil String/Array/Proplist are truthy even when empty (`C4Value.h:185`→`:76`).
- `==`/`!=` honor `#strict` (each `Function` carries its level; `<strict 3`
  compares Int/Bool/nil by value: `0==nil`, `1==true`) (`C4Value.cpp:823`).
- `..`/`..=` concat operator: lex + parse (priority 10, between equality and
  comparison) + eval (string-join/array-append/map-merge) (`:594-657`).
- Call-depth 64 → **512** (`MAX_CONTEXT_STACK`, `:62`) via `stacker::maybe_grow`
  (~10 KiB/level; `cc`/`psm`/`stacker` pinned for Rust 1.87).
- `Var(n)`/`Local(n)` numeric slots wired: dedicated function-scoped
  `var_slots`/`local_slots`; reads routed; negative index clamped to 0; `Local(n)`
  persists via object `local_vars` (`"__local_{n}"`), `Var(n)` per-call like
  `NumVars` (`C4Script.cpp:3390,3408`). (Note: `Var/Local` are separate scratch
  arrays, NOT aliases of named storage — the earlier "aliasing" analysis was a
  misread.)

**Phase 0 arrival fixes (2026-05-30).** The suite didn't compile on arrival
(unvalidated commit `e94e5052` added `local_vars` to `ObjectSnapshot` without
updating 5 fixtures). Fixed: fixture compilation; `Initialize` non-proplist return
now discarded like C++ (`C4Object.cpp:1483`); an infinite-loop test hang
(`command.rs`); a boot/scenario state-machine stranding bug (`main.rs
poll_boot_loading`); host-function checklist; 2 app-integration test pumps;
removed stray `*.bak` files.
