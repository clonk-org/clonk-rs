# Rust Port Status

The pinned C++ source snapshot at
`7d43b47b7d789b533f32d005e64596e0a07019cd` and `content/` are read-only parity
oracles; commits are the record of completed slices. The source snapshot
remains reachable in this repository's Git history, and
`LEGACYCLONK_ORACLE_ROOT` selects an external checkout for live differential
work.

## Focus

Engine virtual-play completes 01–03, 05, and 07–10; exact saved-local RNG now
invalidates the 04/06 route checkpoints and app 04/06/07 wrappers. Other app
keyboard routes and selected 08 are covered. Tutorial 09 pins seed-zero
System-name RNG placement, breath depletion/refill, the cyan HUD bar, and local/foreign
`DoBreath`. Pinned Gold Rush seed 424242 matches through frame 14,415 after C++
fixed-point script trigonometry; the next mismatch is unknown. Network status
barriers, admission/lifecycle, prepared observer hosts, selected-player/resource
publication, exact advertised references, C4Group cores, and initial
parameter/scenario/game/dynamic serialization cover fileless and resource-backed
players; the retained network dialog feeds recursive host selection, and typed
lobby countdown/ready-check traffic is live. Lobby projection preserves recursive
scenario/parameter/team defaults and overrides, clients, save/replay flags, and
definition-resolution boundaries. Reliable UDP fragmentation/reassembly,
missing-fragment recovery, multi-route promotion, pre-join forwarding, and
postmortem replay are modeled. Ordinary offline startup freezes the raw
participant list, admits every
valid module against the effective Parameters capacity before landscape creation,
queues joins before `Initialize`, and executes them before frame-one simulation.
Duplicate real paths retain separate infos but the later join is rejected like
`FileInUse`; distinct local players receive their `.c4p` keyboard sets and route
controls independently. Focus loss runs only the nonfatal UI/pointer
cleanup: no native backend clears player controls there
(C4FullScreen.cpp:139-145,310-315,432-447), so Alt-Tab adds nothing to a
synchronized session, and the window now keeps drawing while it is unfocused so
the picture cannot fall behind the round it is still playing. Scenario
definition lists use classic quoted/numbered parsing and load explicit global
packs before ancestor-local packs; later folder-local ID collisions overload
the global definition, while packed parent graphics and materials resolve
inner-first. Hazard's US-localized Script1 raises the live max-player limit
before emitting the host's exact `Drones` AddPlayers request; Script65 updates
live and persisted crew experience without a rank promotion. Alchemy (ALCO+NMGE)
intentionally replaces mana with ingredients. Its seeded bag follows C++
exit→collect→DigDouble→hidden-bag transfer; `ContextMagic`, MGUP/MGDW
global-effect merging, ABLA aim/release/Airblast, POSE selector/Possession,
MFBL→FRBL collection, MFFW's seven linked FCWS segments with synchronous
stuck-crew ejection, phase-mask rebakes, and damage/timer expiry,
native MVLC→FXV1, MWP2 paired portals/base transfer with exact fixed-slot
RemoveVertex/AddVertex metadata and mid-Warp save/restore, MTNL terrain opening,
FRCS timer audio, and direct CBMU MGUP casting are pinned. Learned MLGT aims,
launches LGTS, and advances its particle line with C++ wrapping arithmetic;
MICS preserves ICEB aim, non-crew cursor,
steering, impact, and Frostwave freeze; FRFS→FSHW→FLAM consumes inflammable
landscape fuel; MQKE consumes IROC, finds ground, launches FXQ1, shakes the
landscape/camera, and expires; MART configures AIR1→LGCN hit artefacts through
its real menus and casts LGCN from an enchanted ROCK impact; XCRS consumes its
recipe, sacrifices energy, and intercepts `AssignDeath` into delayed burning
reincarnation. Learned GGHG sustains Magic and heals nearby crew;
definition-owned effect `FindObject` uses global coordinates. Broader
combo/spell effects remain.
`Set/GetVisibility`, saved `Visibility=`/numbered `Locals=`, all C++ masks,
layers/local bits,
base/object-overlay rendering (including contained overlay-only targets and
`TargetPos` parallax/top faces), mouse picking, and target-message suppression
are live; construction sites draw the global sign at the Con-scaled shape bottom-left,
with real Tutorial 05 ELEV coverage; shipped MINV pins start/stop restoration and native
`ModulateColor` math.
In-game mouse matches left MoveTo, 400 ms carryable LeftDouble→Get, >5 px
landscape frames with `CRed`/candidate marks, 20-item carryable Drop/Throw,
Control-container Put, Grab=1 vehicle PushTo, HUD-region right-up, and
inventory-region same-ID Set→Append ordering. HUD inventory preserves contiguous
same-ID chunks and exact `CanConcatPictureWith` groups; Buy refills preserve the
numeric selection. DefCore and Objects.txt retain all 30 shape slots; saves
preserve dormant slots and OwnVertices backups. DefCore
retains raw five-part versions; runtime definitions apply C++'s 4.9.10.7 fallback.
Contained and pushed-target controls respect C++'s 4.9.1.3/4.9.5 early/late
callback boundaries in classic and auto-stop modes.
Physical PUSH/PULL copies the operator Controller after every successful
vehicle force and before range loss; Tutorial 05 pins sustained CATA ownership.
Rotation hosts target live self/foreign/same-call-created objects with exact
signed `rdir`; Gold Rush `_STA` fragments and same-angle solid-mask rebakes are pinned.
`CreateContents`, `ComposeContents`, and `Split2Components` now run the C++
creation/Enter/removal order synchronously, including custom recipes, live
same-call inventory, component RNG, controller transfer, and real anvil/fish paths.
Far Worlds construction now honors the terrain flag: footprints clear, nearby
ground rises, and granite basements exist before `Construction` runs.
`ConstructionCheck` rejections leave the C++ red feedback on the calling
object for every branch (undefined id, unbuildable definition, no room, no
level ground, blocker name) through both script `CreateConstruction` and the
Construct command; Deep Sea's underwater conkit loop is pinned end to end
against the native binary, including a byte-identical 267-column site survey.
`GetDefCoreVal` reflects raw DefCore `CollectionLimit`/`GrabPutGet` with C++
section/index rules. Legacy loading retains vehicle/base/component flags and
`Exclusive`/`Edible`/`Prey`/`AttractLightning`/`NoFight`. Arctic's occupied
kayak falls through to contained Throw below capacity and opens its grouped
C++ ID-6 Activate menu at capacity; refill filtering/cadence and old-style Get
ID 13 match C++. INUK reads the live double-Down latch, so selected harpoons
take the hardcoded Drop path instead of `ThrowHarpoon`.
Deep Sea AIRL pumping now observes repeated `ExtractLiquid` mutations within
one callback, conserves source material, clears column-model FindMatTop
surfaces, and rejects `MNone` insertion. HCLK finds exact cargo inside submerged
`GrabGet` lorries. Jungle AMUL upgrades resolve their post-`ChangeDef`
`this()->Initialize` on AMPH/AMPO/AMMA immediately, including effects,
temporary physicals, and AMMA action/local initialization. AMPO's live
effect negotiation rejects shipped PARW `PoisonCurse` before validation.
Live-object `AddEffect` reserves its number before Check and completes upper
temp cycling, Start, and Start-deny inline; fire-only outcomes persist.
Arctic LGT2 now launches three native creatorless FXL1 bolts with exact
arguments/RNG; weather lightning records unconditional C++ success, and
lightning/volcano effects start at the native `(50,50)` default.
Loaded `Objects.txt` sector ranks follow the C++ forward master list before
`FixObjectOrder`; runtime sector insertion follows live master order. Dragon
Rock restores saved-open entrances; TENT walk+Up and endboss
`Kill(g_pDragon)` are pinned. `Kill`, `DoEnergy`, `Punch`, and `Blast`
complete the central `AssignDeath` path synchronously before the invoking
C4Script statement continues, including guarded controller credit, effect
revival/force, action callbacks, inventory ejection, crew/cursor/FoW cleanup,
and final OCF refresh. Sky Race starts with one LOAM
bridge chunk; deaths/relaunch, 100% progress, rivalry elimination/retirement,
GOAL-delayed game over, and winner evaluation are pinned. Real CLNK ceiling
contact, attached Hangle traversal, auto-stop release, and let-go match C++.
Movement reacquires the live object and landscape after every `Contact*`
callback, so each callback's complete outcome—including death, revival,
removal, definition/mask changes, and foreign-object writes—is visible to the
next contact direction, collision response, rotation, and movement tail.
Movement removes unbounded side/bottom crossings in the same tick with exact
Border, DFA_ATTACH, and C4D_Parallax/Local[0] exemptions.
`BlastFree` has the void/padded four-int C++ ABI; its landscape mutation still
folds after the invoking callback rather than synchronously.
Tutorial 05's real CATA follows its launched payload through `SetPlrView`; the
next regular non-menu press resets the camera to ViewCursor/Cursor like C++.
Weather uses real material PXS; Tutorial 07 pins rain cadence, fixed
trajectories, and pixels;
lightning has no synthetic launch-frame flash. Regular CONNECT lines use the
C++ PathFree walk, 4/8/12-pixel terrain-bend search, and old-endpoint
PathFreeIgnoreVehicle fallback across solid masks and closed borders.
Power/source/drain/rope/colored/vertex rendering uses absolute live vertices,
C4.PAL colors/locals, and half-open start-marked segments; Lightning
`DrawBolt` uses C++'s per-axis cull, four unsynced jitter draws, CWhite, and raw
triangle strip.

C4Script `&` parameters now bind through `->` object and definition calls:
`Parse_Params` keeps an argument's reference whenever any engine function of
that name declares `C4V_pC4Value` at the slot, so the world bridge carries the
callee's settled parameter slots back to the caller. `SetVertex`'s own-vertex
modes write the `C4Shape` backup half seeded from the definition shape, and
`VTX_SetPermanentUpd` restores the live vertices through `UpdateShape`. Hazard
aiming rides on both: its HCRH crosshair orbits the aiming Clonk at
`CH_Distance` off the `WeaponAt` muzzle vertex, and a left click while aiming
runs `ControlCommand`/`DoMouseAiming`/`FireAimWeapon` through to the launched
projectile.

Menu parity is tracked recursively in `docs/MENU_PARITY.md`. It covers every C++
startup/in-game/object/script/modal screen and nested transition found in the
source and shipped content; top-level visual similarity is not treated as full
menu parity. The classic scoreboard and F1 help now have C++ layout, input
priority, resources, and z-order. Timed flash messages and the startup Sound
sheet's four toggles, two sliders, and six persisted keys are live; unresolved
dependencies and other unported descendants fail at typed boundaries instead
of showing Rust fallbacks.

## Gates

```sh
cargo nextest run -p clonk-engine-integration-tests --test engine_it -E 'test(/^(real_tutorial(0[1-9]|10)_(virtual_play|route)|real_tutorial02_balloon_platform)::/)'
cargo nextest run -p clonk-engine-integration-tests --test engine_it -E 'test(/^virtual_player_harness::/)'
cargo nextest run -p clonk-app -E 'test(/app_virtual_keyboard_(completes|flings|opens)/)'
cargo nextest run --workspace --no-fail-fast
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Only the real engine Tutorial04 and Tutorial06 end-to-end drivers are accepted
over-constraint skips. Manual timing probes, live-service checks, and C++
differential executables remain explicit opt-ins; the remaining ignored app
Tutorial04/06/07 and engine Tutorial05/07 routes are temporary test defects,
not an accepted parity baseline.

Behavior changes also require the relevant scenario sweep/audit and rebuilt
live comparison.

## Low-power hardware

Reference machine: M4 Max. Simulation cost is measured with
`cargo run --release -p clonk-engine --example scenario_profile -- <scenario> <frames>`,
which boots real installed content and times `Engine::tick`. Where a profile is
cited it is macOS `sample` at 1 ms, aggregated over the `advance_tick` subtree
only, so scenario loading is excluded.

The 300-frame runs in the table below are the startup burst and are noisy by
±20% on a shared machine; the 6000-frame `03_Chaos` figure is the one to
regress against.

One environment trap that will otherwise be misread as a regression:
`content/` in a worktree is an empty directory unless the submodule is checked
out; with it empty, six `clonk-network::integration initial_network_dynamic`
tests fail on missing oracle content and look like code faults. A host that
refuses `IPV6_JOIN_GROUP` on the default interface (`EADDRNOTAVAIL`) no longer
fails `startup_lan_reference_query_reports_address_lifecycle`,
`disabled_reference_server_keeps_discovery_only_advertiser_clean` or
`selected_network_scenario_installs_prepared_host_before_admission`:
clonk-org/clonk-rs#107 made the join non-fatal, so those three are once again
ordinary regressions wherever they fail. Separately,
`synchronized_tick::inactive_joined_client_does_not_block_host_lockstep`, plus
`clonk-app`'s `real_hazard_scenario_gui_sheet_overrides_apply_and_reach_running`
(a 480-attempt timeout), are load-sensitive and pass in isolation. Re-run the
crate alone before attributing either to a change.

Per-frame simulation cost by scenario, before against after, 2000 frames each.
Both binaries are run **interleaved** in the same pass and repeated twice, so
machine drift hits both arms; single 300-frame runs swing ±20% on a shared
machine and must not be compared across time. `before` is the profiler built
from `4ad017765`, `after` is the same example built from this branch.

| scenario | objects | mean before | mean after | delta | p50 before | p50 after | delta |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `EkeReloaded/InterplanetaryCivilwar/MeltMe` | 22 | 0.466 ms | 0.418 ms | -10.2% | 179 us | 148 us | -17.1% |
| `ClonkMars/01_Fossae` | 41 | 1.227 ms | 1.065 ms | -13.2% | 818 us | 703 us | -14.1% |
| `ClonkMars/08_Phobos` | 51 | 1.616 ms | 1.303 ms | -19.4% | 1.222 ms | 0.989 ms | -19.1% |
| `ClonkMars/03_Chaos` | 130 | 2.962 ms | 2.212 ms | -25.3% | 2.699 ms | 1.971 ms | -27.0% |
| `ClonkMars/07_Abyss` | 202 | 3.417 ms | 2.754 ms | -19.4% | 2.577 ms | 1.980 ms | -23.2% |
| `ClonkMars/06_Chasma` | 290 | 6.012 ms | 4.685 ms | -22.1% | 3.393 ms | 2.501 ms | -26.3% |

Repeats agree within ~1% on a quiet machine. p99 moves too, though less:
`03_Chaos` 8.23 -> 6.48 ms, `07_Abyss` 21.41 -> 19.53 ms.

Cost is roughly linear in object count, not quadratic, so the wall is
per-object work rather than an all-pairs scan, and the saving grows with object
count because more objects mean more script callbacks.

Projected onto the stated Pi multipliers (`K_sim` 9 for a Pi 4). **These are
arithmetic on reference-machine measurements, not runs on Pi hardware — no Pi
was in the loop for any number in this file.** `03_Chaos` mean goes from
2.96 ms x 9 = 26.7 ms to 2.21 ms x 9 = 19.9 ms against the 28 ms
`INGAME_FRAME_INTERVAL`, and a two-frame control tick from 53 ms to 40 ms
against a 55 ms period — from no headroom to roughly 25% headroom at the mean.
p99 remains over budget on a Pi 4, which is what the degradation levers exist
for.

Where the frame actually goes (`03_Chaos`, 122 objects, `advance_tick` = 100%):
`tick_global_effects` 47.7% and `dispatch_object_effect_events` 27.1% — about
three quarters of the tick is script-driven effect callbacks — against
`exec_mobile_object_movement` 6.5%. Roughly a third of the tick is
`malloc`/`free`. Snapshot construction is **not** a factor: `tick` minus
`tick_without_snapshot` is 0.09 ms at 128 objects (2.7%).

Scenario **loading** is the other half of the problem and shares the same
bottleneck. Over a whole `03_Chaos` process (23563 samples),
`Scenario::apply` is 51% against `advance_tick`'s 44%, and 99% of that
`apply` subtree is `run_legacy_init_placements` -> `init_create_object` ->
`call_object_function` -> `Vm::invoke_script_function` — object-placement
script callbacks running through the same VM the frame loop uses. Wall time is
13.8-15.7 s for `03_Chaos` on the reference machine, so a Pi 4 (K_sim ~9)
spends roughly two minutes before the first frame. That is not addressed here
and is tracked under Open.

Inside `Vm::invoke_script_function` the cost is representation, not dispatch:
`memcmp` 13.4% + `DYLD-STUB$$memcmp` 5.9% = **19.3% string comparison**, and
`sip::Hasher::write` 10.7% + `hash_one::<&str>` 6.1% = **16.9% string
hashing** — about 36% of VM time spent hashing and comparing names, against
`Value::clone` at 3.5%. The AST tree-walk being slower than C++'s 84-opcode
stack VM is therefore real but is not primarily a dispatch-shape problem.

Two earlier hypotheses were measured and killed. `benches/engine_tick.rs` is
not evidence for any of this — its Bouncer fixture exercises the command-DSL
marshalling shortcut rather than gameplay. Landscape uploads to the GPU were
already dirty-rect, not full-surface (`texture_upload_plan`,
`crates/clonk-app-render/src/gpu_renderer.rs`); a full 1.23 MB upload happens
only on first sight, on an extent/format change, above 75% dirty coverage,
above 128 accumulated rects, or after a skipped revision.

Landed optimizations, each with its measurement:

- **Read-only terrain queries borrow the landscape instead of copying it**
  (`LazyHostWorldProvider::with_landscape_borrow`,
  `crates/clonk-engine/src/compat/world.rs`;
  `Engine::lazy_host_world_landscape_borrow`,
  `crates/clonk-engine/src/engine/host_tables.rs`). `GBackSolid` and the rest
  of C4Wrappers.h:66-92 read one pixel; the lazy host world answered them from
  a deep copy of the entire landscape, once per callback that touched terrain.
  `03_Chaos`, 6000 frames: mean 2.645 ms -> **2.223 ms (-16.0%)**, p50
  2.328 ms -> **1.910 ms (-18.0%)**, wall 15.87 s -> 13.34 s. In the profile
  `Landscape::clone` (7.6% inclusive) and `drop_glue::<Landscape>` (7.0%)
  disappear entirely and the `advance_tick` sample count falls 12316 -> 10258.
  Pure representation: reads see the same bytes and a terrain *write* still
  materializes the private copy, so simulation state and ordering are
  unchanged. `cargo xtask parity verify` and `cargo xtask engine-snapshots
  verify` pass. Pinned by `read_only_terrain_queries_never_clone_the_landscape`;
  `lazy_host_world_call_object_materializes_only_on_world_access` and
  `lazy_host_world_contact_materialization_is_deferred_until_query` had their
  materialization counts updated from 1 to 0 — those assertions pin an
  implementation cost, not C++ behavior.

- **Per-frame and script name tables hash without a per-process seed**
  (`CommandObjectSnapshots`, `crates/clonk-engine/src/command/snapshot.rs`;
  `SlotMap`/`NamedLocalMap`/`FunctionVarMap` and the host-function maps,
  `crates/clonk-script/src/vm.rs`, `engine.rs`). String hashing and string
  comparison were about 36% of `Vm::invoke_script_function` (see the profile
  above). The hot key-probed maps now use `rustc_hash`. Safety argument, and it
  is pinned as a test rather than left as prose:
  `std::hash::RandomState` reseeds per process, so any simulation outcome that
  read HashMap iteration order would already desync between two runs of one
  seed — it does not, and `refresh_command_master_list_order` exists precisely
  so iteration order cannot decide a command target. A fixed-seed hasher is
  therefore strictly *more* reproducible than what it replaces. Pinned by
  `per_frame_lookup_tables_hash_without_a_per_process_seed`
  (`crates/clonk-engine/src/engine/exec_order.rs`), which asserts the engine
  hasher is stable across instances while `RandomState` is not.
  The argument was not taken on faith; every iteration site over a converted
  map was traced and none reaches simulation state or ordering:
  `command/machine.rs:5159,:5663` are `.any()` booleans; `:5180`
  `find_candidate` ranks on the total order (distance, master-list rank, id);
  `:1655` `find_spawned_construction` is `min_by_key((master_list_order, id))`
  where ties are impossible; `engine/player_view.rs:695` is a per-entry
  `values_mut()` write; `clonk-script/src/vm.rs:9490,:9532,:9562,:9571` iterate
  the scope *Vec* and `contains_key` each, folding into another name-keyed map;
  `clonk-script/src/engine.rs:1266,:1270` remove by name, `:1795` sorts, and
  `:1364` `link_script` carries physical declaration order separately in
  `local_function_order`/`global_function_order` — the design already treats
  those maps as unordered; `compat/world.rs:1592` is a commutative `|=` and
  `:1623` builds a set consumed only by `.contains()`.
  Measured in isolation (only this commit reverted, so the other work is in
  both arms), `03_Chaos` 6000 frames: p50 1.631 -> 1.550 ms and wall 11.494 ->
  10.884 s, **-5.0% p50 / -5.3% wall**, min-of-6.
  **Estimator caveat, carried deliberately:** round-by-round was 3-3 under load
  average 10-53, so this is min-of-N, not clean per-round separation.
  Min-of-N is the right estimator because contention only ever adds time, and
  it agrees with an independent earlier run (-4.7%) and with the profile
  (~4.9% of tick was in the converted maps) — but it is not a clean A/B.

- **`c4_names_equal` compares folded byte streams instead of building owned
  keys** (`crates/clonk-resources/src/material.rs`). `c4_name_key` allocates
  twice per call and was the comparison primitive, so comparing two material
  names cost four allocations; it was 1.6% of `advance_tick` self time.
  Equality is now an iterator comparison. Pinned by
  `name_equality_matches_the_owned_key_projection`, which asserts the new
  predicate agrees with the old owned-key one across case, padding, embedded
  NULs and latin1 spellings. Measured in isolation against commit `3824cea5e`,
  `03_Chaos` 6000 frames: p50 1.810 -> 1.631 ms and wall 12.645 -> 11.461 s,
  **-9.9% p50 / -9.4% wall**, winning all 8 paired rounds.
  Do **not** add this to the hasher figure below: the two were baselined
  against different trees, so they do not compose arithmetically. The combined
  effect is the whole-branch table above.

- **The per-viewport pixel plane is deferred until something touches pixels**
  (`PixelPlane`, `crates/clonk-graphics/src/surface.rs`). `Surface::new`
  unconditionally allocated and zeroed `width * height * 4` bytes, and
  `render_viewport` built one per viewport per frame — but under GPU scene
  capture every primitive records a command instead of rasterizing, so those
  bytes were never read or written. The plane is now a `OnceCell` materialized
  inside the byte accessor, so a missed path allocates rather than reading
  garbage. Measured by `gpu_capture_frames_materialize_no_viewport_pixel_planes`
  over 60 steady-state 640x480 capture frames: **1,228,800 deferred bytes per
  frame, 0 materializations**, and the removed allocate+zero+free costs
  0.030 ms/frame on the reference machine — memory-bandwidth-bound work, so the
  stated `K_blit` 10/25/75 multipliers put it near 0.3 ms on a Pi 4 and 2.3 ms
  on a Pi 1. `deferred_pixel_plane_rasterizes_identically_to_an_eager_one`
  pins that a non-capture surface still produces identical pixels.

- **The landscape render cache stops re-cloning the grid it is already anchored
  to** (`crates/clonk-frontend/src/graphics_system.rs`). The COW dirty-lineage
  invariant documented at `landscape.rs:550-554` is real, so the cache still
  holds a grid handle; only the provably redundant re-anchor is skipped, when
  the cache was reused or freshly rebuilt from this same grid *and*
  `cache.grid.bytes().as_ptr() == grid.bytes().as_ptr()`, i.e. it already holds
  that exact `Arc`. Rebuild frames were cloning twice (`:4277` and again at
  `:4579`); that is gone too. `PixelGrid::clone` with 128 texture + 128
  material names measures **4.3-5.6 us**, once per viewport per frame, now zero
  on static-landscape frames.

- **The texmap name tables are identified, not compared, every frame**
  (`texmap_identity`, `PixelGrid::texmap_tables_match`,
  `crates/clonk-engine/src/landscape.rs`). `render_dirty_rects_since` runs once
  per presented frame and compared `material_names` and `texture_names` — two
  128-entry `Vec<Option<String>>` — element by element to answer one yes/no
  question. Both tables move only in `sync_runtime_texmap`, so an FNV-1a
  identity over them (the same scheme `render_token` already uses) answers it
  with a `u64`. Measured on the shipped 128-slot table over 10,000 checks:
  **1.870 us -> 0.007 us per frame**, i.e. ~1.86 us saved per presented frame
  on the reference machine. That is a small absolute win — well under 0.01% of
  the 28 ms budget, ~17-37 us on the stated Pi multipliers — and is recorded as
  such rather than as a headline.
  The identity is content-derived rather than a counter because two grids that
  never synced a texmap would share any naive generation, and the frontend can
  be handed grids from unrelated landscapes. It is runtime-only: not
  serialized, equal to every other value under `PartialEq` so a save round-trip
  stays equal, and zero means "not computed", which makes
  `texmap_tables_match` fall back to the original compare for a grid restored
  from a save. Pinned by `texmap_identity_agrees_with_the_name_table_compare`
  (which includes the save round-trip case) and
  `texmap_identity_costs_less_than_the_name_table_compare`.

- **Particles are grouped by layer once per object pass**
  (`ParticleLayerIndex`, `crates/clonk-frontend/src/graphics_system.rs`)
  instead of `draw_definition_particles` filtering the whole particle slice on
  each of its two calls per object, which made a pass O(objects x particles).
  At 40 objects x 200 particles: **16,000 -> 200 particle examinations per
  pass**, wall time **54.0/77.9/64.8 us -> 27.9/37.4/28.1 us** over three runs
  each. Draw order is unchanged — the index is built by reverse traversal, so
  each layer list is in the same newest-first order the linear scan produced —
  and the index is rebuilt from, and invalidated at the end of, the single
  object pass that owns it, with a slice-identity check that falls back to the
  linear scan rather than trusting a stale list.

Evaluated and deliberately **not** built, so it is not re-derived:
memoizing `RuntimeTexMapState::default_material_entry`
(`crates/clonk-engine/src/landscape.rs`), an allocating linear scan whose
answer is a load-time constant. After the name-comparison fix above its whole
ceiling is `c4_names_equal` 0.55% + `ocf_solid_mask_overlay` 0.50% inclusive of
`advance_tick`. A stale-proof memo needs `default_material_entries` wrapped in
a newtype, because `RuntimeTexMapState` derives `PartialEq`/`Serialize` and the
field is writable from ~18 sites — that is a cache-staleness desync class taken
on for at most 0.5%. Not worth it.

Measured next tier, for whoever picks this up. The large remaining target is
**effect-dispatch cloning**: `dispatch_object_effect_events` is 47% inclusive,
and nearly all of the ~20% malloc/free self time sits under it —
`EffectState::to_vec`, `ObjectState::clone`, `Value::clone` (4.6% self) and
`drop_glue::<Value>` (2.6% self). Remaining unconverted hashing is much
smaller: `Engine::definitions` via `active_solid_mask_indices` 2.0%,
`ActionLibrary::spec_for_entry` 1.0%, `HostWorldContext::get` 1.0%,
`Engine::snapshot` 0.7%, `SectorMap::update` 0.6%.

## Open

- Open: **macOS `-[NSApplication terminate:]` skips the league `Action=End`.**
  `C4Application::Quit` reaches `C4Game::Clear` → `Network.Clear()` →
  `LeagueEnd(); DeinitLeague();` (`src/C4Game.cpp:581`;
  `src/C4Network2.cpp:746-763`) for *every* way the loop unwinds, so a native
  host always de-registers.

  The port sends the `End` from `NetworkManager::drop`, and reaches it two ways:
  `GameApp::request_exit` drops the session at the quit latch, and — for
  anything that never touches that latch, including the event loop's
  `event_target.exit()` error arms — the handler closure owns the `GameApp`
  (`crates/clonk-app/src/main.rs`, the `move` closure the initializer returns)
  and is dropped when `run_app` returns. So Windows, X11 and Wayland always
  de-register, and so does macOS whenever `exit()` was the trigger.

  The exception is macOS `terminate:` — Cmd+Q, the Dock's Quit item and log-out.
  It never delivers `CloseRequested`, and `run_app` never returns, so neither
  route runs: winit 0.30.13 implements only `applicationWillTerminate:`
  (`winit-0.30.13/src/platform_impl/macos/app_state.rs:69-72`), never
  `applicationShouldTerminate:`, so the app first hears about the quit from
  inside AppKit's own terminate — where joining worker threads is exactly what
  `Event::LoopExiting` is restricted from doing. SDL cancels that terminate and
  re-posts it as `SDL_QUIT` (`src/StdAppUnix.cpp:809-815`), which is why C++ is
  unaffected; matching it needs the port to own an `applicationShouldTerminate:`
  that returns `NSTerminateCancel` and routes into `request_exit`, the same
  shape `CStdApp::Quit`'s `fQuitMsgReceived` latch has
  (`src/StdAppUnix.cpp:256-259`). Note the Cmd+Q in question is winit's own
  default menu item (`winit-0.30.13/src/platform_impl/macos/menu.rs:66-73`).

  A macOS host quitting that way leaves its game registered until the server
  times it out. The consequence is bounded: since
  `fix: keep hosting when the league server refuses the registration`, a refused
  `Start` on the next host is a dismissible dialog, not a failed host. No gate
  covers this — neither `parity verify` nor the snapshots reach process
  shutdown.

- Open: **A `global func` body may read and write its declaring host's named
  `local`s.** C4Aul rejects that outright at parse time, in both the lvalue and
  the rvalue path — `else if (a->LocalNamed.GetItemNr(Idtf) != -1) { if
  (Fn->Owner == &Game.ScriptEngine) throw C4AulParseError(this, "using local
  variable in global function!"); }` (`C4AulParse.cpp:2000-2004`,
  `:2731-2737`) — so the enclosing function never links and every call raises.
  Engine-owned *function* resolution now matches (`global func` bodies resolve
  identifiers against the engine table), but the named-`local` rejection has no
  equivalent, because the port resolves object locals at run time against the
  supplied cells rather than against a per-script `LocalNamed` table checked
  during parsing. No shipped content depends on it. `parity verify` does not
  cover it: the golden has no `global func`.

- Open: **An unresolvable hard `inherited()` is reported at link time but does
  not truncate the function, and there is no `errCnt` summary.**
  `C4AulParse.cpp:2799` throws under `Type == PARSER`; `C4AulScript::Parse`
  catches it, logs it, counts it into `Game.ScriptEngine.errCnt`, truncates
  that function's bytecode *at the offending token*, redirects every unresolved
  forward jump there and appends `AB_ERR` (`C4AulParse.cpp:3563-3586`). The
  port now reports the same condition once at link time
  (`Engine::unresolved_inherited_diagnostics`, logged by
  `report_unresolved_inherited`, with C++'s pure-`#appendto` log suppression
  from `C4AulParse.cpp:3566`), and the call still raises when it runs — but the
  statements *before* the offending token continue to execute, where C++ would
  have discarded them with the rest of the body. Reproducing that needs
  expression spans and a partial-body AST node the parser does not carry.
  `C4AulScriptEngine::Link`'s `"linked - N errors"` summary
  (`C4AulLink.cpp:299-301`) has no port either.

  Survey trap: `content/` scripts are latin-1, so plain `grep -rn`/`grep -rI`
  classifies them as binary and silently skips them — use
  `--binary-files=text`, or an earlier survey's "8 hits" becomes 100. The ~99
  shipped functions carrying a hard `inherited()` are exactly why the link
  check resolves through the same rule the VM uses (own-owner list, then the
  engine hop, then the same-name native) rather than a chain walk.

  `parity verify` covers none of it: the golden has no `global func`.

- Open: **A `Game.pGlobalEffects` callback still runs with no ambient object
  even when the effect has a command target.** Object effects now execute on
  their command target, so `cthr->Obj` — the fallback every implicit-object
  native uses — is that object (`C4Effect.cpp:129,282,345,392,434`;
  `C4AulExec.cpp:1638-1648`). `dispatch_global_effect_callback`
  (`crates/clonk-engine/src/lib.rs`) still passes `None` for the host object
  context, so `AddEffect("X", 0, prio, iv, this)` reaches a global effect whose
  `Fx*` callbacks see `this()` but resolve bare `GetAlive`/`GetAction`/`GetX`
  against nothing. Closing it needs the object-scope builder that
  `Definition::dispatch_effect_callback_with_parameter_conversion_policy` owns
  to become reachable without a `Definition` receiver — the global dispatch
  deliberately has none, because C++ runs System/scenario callbacks in a game
  with no loaded definitions. `parity verify` does not cover it: the golden has
  no global effect with a command target.

- Open: **An omitted `content/` entry cannot yet be classified as release-owned
  or user-added.** Component archives are complete snapshots, but installed
  state records only their digest and version, not the top-level names the
  archive owned. The updater therefore preserves omitted content packs to avoid
  deleting user-installed scenarios or definitions; if a later release removes
  an official pack entirely, that pack can remain as hybrid content. Engine and
  planet swaps are exact and do not have this ambiguity. Closing it requires
  package/installed-state ownership inventory so the applier can retain only
  names that were never owned by the previous release.

- Open: **A hard power loss during the engine directory's two-rename swap can
  leave no launcher available to start recovery.** The component updater keeps
  a durable journal before every live-tree mutation and ordinary process
  failures either roll back immediately or resume on the next launch. The
  engine component still replaces the whole `bin` directory (or
  `Contents/MacOS`): after the installed directory has moved to its backup and
  before the staged directory takes its name, neither `clonk-game` nor
  `clonk-app` exists at the path a shortcut opens. The journal and backup are
  intact, but a reboot in that narrow window has no automatic executable entry
  point from which to consume them. Closing this requires either a stable
  bootstrap outside the replaced directory, with package/shortcut changes on
  every platform, or a separately designed file-level engine transaction; it
  is not covered by `parity verify` or the normal interrupted-process recovery
  tests.

- Open: **Bundle recovery is keyed by pathname, not a stable installation
  identity.** The external journal namespace and stored install root isolate
  sibling bundles and reject a different canonical path, but an unrelated
  bundle installed later at the same path can still accept stale recovery
  state. Conversely, moving or renaming an interrupted bundle leaves its
  sidecar in the old parent namespace, so startup from the new path cannot find
  it. Closing this requires a persistent installation identity (or equivalent
  filesystem identity) plus a sidecar lookup/migration design that survives a
  bundle move without admitting same-path replacements.

- Open: **Runtime JoinData cleanup is tied solely to a per-host one-second
  Tokio interval.** Native `C4Network2::Execute` removes an outdated dynamic
  whenever `ControlTick > iDynamicTick` (`src/C4Network2.cpp:679-696`).
  `C4Application` calls `C4Game::Execute` every running game cycle
  (`src/C4Application.cpp:451-460`), which calls `Network.Execute` before
  control preparation (`src/C4Game.cpp:776-782`); after
  `C4GameControl::Ticks` advances `ControlTick` (`src/C4GameControl.cpp:325-330`),
  the next game execution can remove the dynamic without waiting for a second.
  Rust currently calls `remove_stale_host_runtime_dynamic` only from the
  per-host `runtime_dynamic_timer` in `session/host_loop.rs`. Map and add the
  post-frame/next-`C4Network2::Execute` equivalent; do not infer a guaranteed
  one-second grace from the timer-path test.

- Open: **`CtrlRateDown`, `CtrlRateUp` and `NetAllowJoinToggle` are accepted by
  the key parser but dispatched nowhere.** All three sit in the same
  `KEY_Default` "no default keys assigned" block as `ChartToggle`,
  `NetObsNextPlayer` and `NetStatsToggle` (`src/C4Game.cpp:3456-3462`), and
  `RUNTIME_REGISTERED_GLOBAL_KEYS` (`crates/clonk-app/src/main_parts/assets.rs`)
  accepts a `[Keys]` override for each, so a configured chord is stored and then
  silently ignored. `NetStatsToggle` was closed for clonk-org/clonk-rs#117;
  these three still need their callbacks — `C4GameControl::KeyAdjustControlRate`
  (`src/C4GameControl.h:124`) and `C4Network2::ToggleAllowJoin`
  (`src/C4Network2.cpp:799`) — routed at the same PRIO_Base position, after
  `handle_runtime_chart_toggle_key` and in native registration order.

- Closed 2026-07-30: **Terrain saved under a put SolidMask survives a blast —
  oracle-faithful, not a port gap.** Reported as clonk-org/clonk-rs#43: a flint
  thrown at an elevator case parked on stone blows up everything visible, but
  moving the case reveals untouched stone under its floor. C++ does the same.
  `C4Game::Explosion` reaches `C4Landscape::BlastFree` directly
  (`C4Effect.cpp:919`), and unlike `ClearRect`
  (`C4Landscape.cpp:2171-2181`) `BlastFree` carries no
  `PrepareChange`/`FinishChange` bracket (`C4Landscape.cpp:1022-1062`), so it
  scans the *masked* Surface8. Every put mask pixel reads `MCVehic`, i.e.
  material `MVehic`, and `BlastFreePix` clears only when
  `Game.Material.Map[mat].BlastFree` is set (`C4Landscape.cpp:941-960`) —
  `Material.c4g/Vehicle.c4m` sets neither `BlastFree` nor `BlastShiftTo`, and
  `C4Material.cpp:105` defaults `BlastFree` to 0, so the masked pixels are
  counted into `BlastMatCount[MVehic]` and otherwise skipped, consuming no
  `Random()`. `C4SolidMask::Remove` then restores the background byte saved
  before the blast (`C4SolidMask.cpp:241-260`). `DigFree` shields the same
  pixels by the same mechanism, which is why a lift floor is undiggable.
  Bracketing the blast would free that material, shift `Random()` call order for
  every `BlastShiftTo` material in the crater, and desync against the oracle.
  Pinned by `blast_free_leaves_the_landscape_under_a_solid_mask_intact_like_cpp`
  next to the existing `dig_free_runs_before_movers_own_baked_mask_is_removed`
  (`crates/clonk-engine-unit-tests/tests/unit/parts/solidmask_shape.rs`).

- Closed 2026-07-30: **Every preplaced object whose saved `DrawTransform`
  carried the FlipDir mirror rendered exactly backwards.** Reported as the
  Dragon Rock intro dragon facing right while it flew left. `C4Object::
  UpdateFlipDir` (`C4Object.cpp:410-442`) is the *sole* owner of the mirror:
  it folds the sign into `pDrawTransform->mat[0]` via `SetFlipDir`
  (`C4Facet.h:79-88`), and `C4Object::Draw` hands that matrix straight to
  `DrawT`/`DrawXT` without mirroring anything itself
  (`C4Object.cpp:2506-2515`). The port had no `UpdateFlipDir` at all. Instead
  the loader faithfully restored C++'s already-mirrored matrix *and* the
  renderer independently re-derived the mirror from `Action.Dir`, so the two
  cancelled: `DRGN` #202 and `KING` #5129 ship
  `Dir=1` / `DrawTransform=-1,0,0,0,1,0,-1`, and every such object drew with
  its facing inverted. The mirror now lives in the engine — at SetDir
  (`C4Object.cpp:4276-4279`), at SetAction guarded on the FlipDir *value*
  changing (`:4183-4184`), and once per active object after `Objects.txt`
  load (`C4GameObjects.cpp:665-674`) — and `resolve_draw_direction` returns
  only `Action.DrawDir`, the facet row. Three consequences are C++-correct
  convergence rather than regressions: 6 preplaced objects that mirror with
  no saved transform (OilWars `PLM2`/`SNKE`, Paxhill `HORS`) now gain
  `new C4DrawTransform(-1)`; Goldrush #439 and Hammerfest #1019 (`CCAN`,
  saved FlipDir `-1` under an ActMap with no FlipDir) now draw unmirrored;
  and 173 preplaced exactly-identity transforms are deleted at load, so a
  re-save omits the key while engine-created facing objects start emitting
  it. The three `testdata/dev-replays` goldens moved for the same reason —
  a right-facing crew Clonk now carries the transform.

- Open: **Graphics overlays in `MODE_Action`/`MODE_Base` still take their
  facet row and mirror from the host object's facing.**
  `C4GraphicsOverlay::Draw` blits with `iPhaseY = 0` and the overlay's *own*
  `C4DrawTransform`, never the host's `pDrawTransform`
  (`C4DefGraphics.cpp:820-826`). The port's behaviour is unchanged and now
  quarantined behind `GraphicsSystem::resolve_overlay_action_flip`; no test
  covers it, so closing it needs its own oracle first. `MODE_ExtraGraphics`
  is unaffected — it *does* inherit the host transform in C++
  (`C4DefGraphics.cpp:794-806`), which the port already reproduces.

- Open: **`Action.DrawDir` is derived at draw time rather than stored.**
  C++ writes it in `UpdateFlipDir`/`SetDir` and reads it back; it is
  `// NoSave` (`C4Object.h:98`) and absent from `C4Action::CompileFunc`, so
  keeping it derived costs no savegame fidelity and keeps `ObjectSnapshot`'s
  serialized shape unchanged. The one unreproducible case is C++'s stale
  window: `SetAction` skips the refresh when old and new FlipDir are equal
  and never clamps `DrawDir` to the new action's `Directions`.

- Open: **The `transformed` predicate in `blit_face` has no `Def->Rotateable`
  gate.** C++ takes the untransformed blit iff
  `(!Def->Rotateable || r == 0) && !pDrawTransform` (`C4Object.cpp:477`); the
  port tests `rotation_degrees.abs() > EPSILON` on a `rem_euclid(360)` value,
  so a non-rotateable object with a scripted `r` rotates where C++ does not,
  and `r == 360` normalizes to `0` where C++ reads it as nonzero. Pre-existing
  and deliberately left alone while the FlipDir change landed, so a facing
  regression stays distinguishable from a rotation one.

- Open: **The `DFA_PUSH`/`DFA_PULL`/`DFA_FIGHT` fallback arms in
  `engine/movement.rs` face by the whole-pixel velocity mirror** where C++
  tests the raw `C4Fixed` xdir, so a sub-pixel velocity leaves `Dir` unchanged
  in Rust and calls `SetDir` in C++. The same `Fight` arm faces by velocity
  where `C4Object.cpp:5276-5277` faces the *target* and calls `SetDir` zero
  times when the two share an x. Pre-existing; surfaced while mapping every
  direction write for the FlipDir port.

- Closed 2026-07-30: **The Options Gamepad sheet printed `invalid` where the
  oracle prints nothing.** With no `[Gamepad1]` section in the config,
  `C4ConfigGamepad::CompileFunc` defaults every `Button[i]` to `-1`
  (`C4Config.cpp:591-602`), and `KeySelButton::DrawElement` displays
  `C4KeyCodeEx::KeyCode2String(key, true, false)`
  (`C4StartupOptionsDlg.cpp:243`). `-1` is not a gamepad code (`Key_IsGamepad`
  wants `0x42` in bits 16-23, `C4KeyboardInput.h:83-86`), so it falls into the
  `USE_SDL_MAINLOOP` branch. That branch calls **`SDL_GetScancodeName`**, and
  returns `"invalid"` only when the returned pointer is NULL
  (`C4KeyboardInput.cpp:375-381`) — the earlier note here described
  `SDL_GetKeyName(SDL_GetKeyFromScancode(...))`, a pair that does not appear
  anywhere in that file at the pinned snapshot, which is where the `"invalid"`
  reading came from. `SDL_GetScancodeName` never returns NULL: for an
  out-of-range scancode it sets an error and returns the empty string, verified
  by probe against the locally installed SDL2 for `-1`, `0xFFFFFFFF` and `0`.
  The oracle therefore prints an empty second line, which is exactly what the
  live 1280x720 F9 capture showed. `legacy_gamepad_key_label`
  (`crates/clonk-app/src/input.rs`) now returns the empty string for the
  unassigned default, names the scancode for any other non-gamepad code, and
  reproduces the wrapping `uint8` button index for gamepad codes outside the
  axis ranges rather than inventing a sentinel — so `Undefined` and `invalid`
  are both gone from the control sheets and the player overlay.

- Closed 2026-07-30: **Options control-sheet behaviour.** The sheets rendered
  correctly but did not yet behave like `ControlConfigArea`. The capture modal
  substituted a second, hand-written English name (`"Cursor Left"` where the
  button beside it said `"Select left"`) instead of the `IDS_CTL_*` string C++
  puts in `IDS_MSG_PRESSKEY`/`IDS_MSG_PRESSBTN`
  (`C4StartupOptionsDlg.cpp:176-177`, `:243`). Space, Return and the gamepad low
  button did nothing on a focused selector, key cap or reset button, so the
  sheets were unreachable without a mouse; they now latch `fDown` and activate on
  release with `Button`'s `ArrowHit`/`Click` pair
  (`C4GuiButton.cpp:36-43,112-128,183-200`), and the reset button no longer plays
  an invented `Command` — `OnResetKeysBtn` is silent (`:416-427`). The GUI
  checkbox accepted clicks across its whole caption where `CheckBox::MouseInput`
  gates on `Inside(iX, 0, rcBounds.Hgt)` (`C4GuiCheckBox.cpp:87`), and toggled on
  Return where C++ binds K_SPACE only, as the *down* callback (`:44-51`).
  Toggling it only faded the dialog; `OnGUIGamepadCheckChange` ends in
  `RecreateDialog(false)` (`:437`), which rebuilds through `SwitchDialog`
  (`:1331`) — so both areas return to set 0 and the config is re-read, with only
  the sheet index restored (`:1332`). And `connected_count` tallied gilrs events
  rather than enumerating, so a pad attached before launch stayed invisible until
  first touched, hiding its bindings and mis-sizing the selector row;
  `GetGamePadCount()` is `SDL_NumJoysticks()` (`C4GamePadCon.cpp:437-440`).
  Pinned by `gamepad_sheet_render_matches_the_cpp_draw_model_at_1280x720`,
  `selected_control_set_button_is_additively_highlighted` and
  `highlighting_a_key_button_undims_its_glyph_and_reddens_its_labels`, which are
  the first tests to render either control sheet with the shipped
  `Control.png`/`Gamepad.png` rather than the facet-less fallback that cannot
  exist in C++ (`C4GraphicsResource.cpp:200,229` both `return false`).
  **Still open:** `C4GamePadOpener` is claimed only while the Gamepad sheet is
  visible where C++ holds it for the whole dialog lifetime
  (`C4StartupOptionsDlg.cpp:347-352,384-388`); `is_supported_key` refuses
  keycodes the config codec cannot encode, leaving the modal open, where C++
  stores any raw code; reset writes 48 `ButtonN=-1` plus 72 `Axis*` lines where
  C++ omits defaults entirely; more than four attached pads are clamped to four,
  where C++ shows one selector per pad and then reads `Config.Gamepads[4]` out of
  bounds (a deliberate, non-determinism-affecting divergence); and no captured
  C++ pixel baseline exists for either sheet, so those render tests are Rust
  self-consistency only.

- **`C4GameObjects::ValidateOwners` is not ported.**
  `InitGameFinal` resets `Owner` to `NO_OWNER` for every object whose player
  never joined, before the scenario constructor runs
  (`C4Game.cpp:2741`, `C4ObjectList.cpp:576`); `C4PlayerList::Remove` repeats it
  (`C4PlayerList.cpp:264`). There is no Rust counterpart, so an object loaded
  from `Objects.txt` keeps a dangling owner when its restore row is dropped.
  Newly relevant now that regular scenarios shipping `SavePlayerInfos.txt` take
  the recreation branch: Drachenfels carries 27 objects with `Owner=10` that
  are only valid because its `GameNumber=10` script player is restored.

- **`C4PlayerInfoList::RemoveUnassociatedPlayers` is not ported.**
  After `RestoreSavegameInfos`' association passes, C++ drops the savegame rows
  no participant claimed (`C4PlayerInfo.cpp:1424-1441`), logging
  `IDS_PRC_RESUMENOPLRASSOCIATION`. Unreachable for both shipped restore-info
  scenarios (their only rows are script players, which are always claimed by
  `CreateRestoreInfosForJoinedScriptPlayers`), but a real gap for savegames.

- **The restored player-number sentinel differs from C++.**
  `C4Player::Number` compiles from `Index` with default `C4P_Number_None`
  (`-5`, `C4Player.cpp:1598`, `C4Player.h:31`) and falls back to the number
  `RecreatePlayers` passed in. `runtime_join_player_restore.rs`'s
  `parse_player_state` defaults `Index` to `-1` and then allocates the lowest
  free number instead. Both shipped restore-info scenarios carry an explicit
  `GameNumber`, so neither sentinel is reached; a savegame written by C++ with
  no `Index` would diverge.

- **Automatic user↔savegame association is unverified against C++.**
  No shipped scenario sets `Head.SaveGame`, so the four `RestoreSavegameInfos`
  matching passes (`C4PlayerInfo.cpp:1373-1391`, mirrored in
  `offline_savegame.rs`) are reachable only from runtime-written saves. Their
  fidelity rests on unit fixtures, not on a differential run.

- **`C4PlayerList::Join`'s max-player rejection is not ported.**
  C++ refuses a join outright when `GetCount() + 1 > Game.Parameters.MaxPlayers`,
  logs `IDS_PRC_TOOMANYPLRS` and returns no `C4Player`
  (`C4PlayerList.cpp:288-294`). The Rust execution chain
  (`apply_join_player_control` -> `join_player_at_client_with_semantics` ->
  `register_joining_player`) never consults `max_players`, so it is strictly
  more permissive. This is reachable in shipped content, not theoretical:
  HarpoonRace's `Script1` calls parameterless `SetMaxPlayer()`
  (`content/EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s/Script.c:14-18`),
  which both engines resolve to `MaxPlayers = 0`, closing later admission in
  C++ only. Initial joins are unaffected — they are issued at the Go tick,
  before `Initialize()` and long before the Tick10-gated `Script1` — so this
  can only diverge on a **runtime** join into such a round, where C++ drops the
  player and Rust seats them. Closing it means adding the count gate to the
  synchronized join path, with an audit of the fixtures that currently join
  past a scenario's declared `MaxPlayer`.

- **`C4Player::Eliminate`'s early client deactivation is missing.**
  When the control host eliminates a player belonging to a *non-host* client
  and no unbeaten player is left at that client, C++ submits
  `CID_ClientUpdate`/`CUT_Activate(false)` for it
  (`C4Player.cpp:2075-2088`). `player.rs::eliminate` has no equivalent, so a
  fully eliminated remote client keeps its activated slot. The branch is gated
  on `AtClient > C4ClientIDHost`, so an eliminated host is unaffected either
  way; the visible effect is confined to lobby/roster activation state and
  control-tick participation of wiped-out clients.

- **Property-panel composition landed; the surfaces open.**
  `clonk-engine::developer_property_text` ports `C4PropertyDlg::Update`'s body
  (`C4PropertyDlg.cpp:169-256`): the 0/1/many switch, the fixed section order
  (type, owner, contents, action, locals, effects), and the `fFirstLocal`-style
  headers that appear **once** before their first entry and not at all when the
  section is empty. Section *values* are supplied by the caller, so this is
  independent of the value formatting, which the object-inspection read model
  supplies. Pinned by
  `object_list_and_property_dialog_share_edit_cursor_selection_order`.
  **Still open:** the panel and object-list surfaces (which need the keyed
  developer window host), the script input's `EMMO_Script` fan-out, and the
  refresh cadence.

- **Object-inspection read model landed; the windows that consume it are open.**
  `clonk-engine::developer_inspection` and `::developer_locals` supply the
  native ordering the console needs and nothing else exposes.
  `object_tree` reverses `SimulationSnapshot::render_order` — which is the draw
  direction, `Last -> Prev` (`C4ObjectList.cpp:390-395`) — to recover
  `Game.Objects` First -> Next, then skips contained objects at the top level
  and recurses through each `Contents` list, matching `C4ObjectListDlg`'s
  repeated "Skip Contained Objects in the main list"
  (`C4ObjectListDlg.cpp:100-101,557-560`). `name_list` ports
  `GetListID`/`GetNameList` including the fixed 500-slot id buffer and the
  separator keyed on the *index* rather than on what was emitted, so a skipped
  unknown leading definition still leaves its comma
  (`C4ObjectList.cpp:55-83,536-574`). `locals_in_emission_order` ports
  `C4PropertyDlg.cpp:210-234`'s two asymmetric loops: indexed `Local[n]` slots
  ascending and **truthy-only**, then **every** declared named local, assigned
  or not. Declaration order is the *definition's* — `SetNameList` /
  `OnNameListChanged` (`C4ValueMap.cpp`) re-map a loaded object's `LocalNamed=`
  onto the definition's name list and drop anything it no longer declares — so
  it comes from `Script::var_decls`, not from the object's map. Numbered slots
  already exist in the port as `__local_{n}` keys, the same keys
  `Local(n)`/`SetLocal` use. `clonk_script::data_string` is now the public
  `C4Value::GetDataString`. Completion keeps C++'s two *different* rules:
  the engine list tests `GetPublic()`, so every global script function shows
  even when declared private, while a definition function must be exactly
  `AA_PUBLIC` (`C4PropertyDlg.cpp:337-358`). Pinned by
  `developer_object_inspection_preserves_master_contents_local_and_effect_order`,
  `developer_object_inspection_exposes_data_strings_and_public_def_functions`
  and `locals_split_into_indexed_then_named_like_the_property_panel`.
  **Still open:** the tree and property windows themselves (the property and
  object-list dialogs, plus the keyed developer window host), and wiring the
  engine's live function tables into `completion_functions`.

- **The watcher itself landed.** `clonk-platform::file_monitor::DirectoryMonitor`
  is the reference backend's behaviour, not a richer one. `C4FileMonitor`'s
  macOS backend is FSEvents with **latency 1.0 s and flags 0**
  (`C4FileMonitor.cpp:287`) — flags 0 is `kFSEventStreamCreateFlagNone`, *not*
  `kFileEvents`, so events are **directory-granular**: the path handed to the
  callback is always a directory, never the file that changed. Linux inotify is
  the same, pushing `watchDescriptors[event->wd]` and ignoring `event->name`
  (`:80-126`); only Windows reports a child file path. A one-second poll
  therefore reproduces it exactly, and no file-watching dependency was added —
  which also keeps `clonk-engine`'s dependency mirroring in
  `clonk-engine-unit-tests` untouched.
  Two behaviours it keeps deliberately: **a directory registered after the
  monitor starts is silently dropped** (`if (!started) paths.emplace_back(...)`,
  `:299-305`), which is safe only because the lifecycle is create in
  `InitGame`, register while definitions load, start in `InitGameFinal`
  (`C4Game.cpp:2413-2424,2738,4445`); and **dropped events are not recovered** —
  the callback skips the `UserDropped|KernelDropped` flags and does nothing
  else (`:256-273`), so adding a rescan would be stricter than C++. Pinned by
  `file_monitor_reports_directories_and_refuses_late_registration`.
  **Now wired end to end.** `GameApp::arm_developer_file_monitor` runs as
  `C4Game::InitGameFinal`'s last act — after viewports exist and definitions
  have loaded (`C4Game.cpp:2738`) — because registration closes at the start.
  It arms only when `Config.Developer.AutoFileReload` (default **true**,
  `C4Config.cpp:434`) is set, the app is windowed, and no monitor is already
  running; `Engine::monitored_definition_directories` supplies only **unpacked**
  groups, each once. `poll_developer_file_monitor` then feeds
  `changed_file_route`, which refuses in a network game, routes a matched
  definition to `reload_definition`, and offers everything else to the script
  host. Pinned end to end by
  `developer_file_monitor_arms_registers_then_dispatches_definition_reloads`,
  which corrupts a real group on disk and watches the definition disappear
  through the failure arm.
  The dependency this note previously recorded was real and is now discharged: `C4Def::Load` registers
  `Filename`, the group's own full name (`C4Def.cpp:547-560`), which is exactly
  the source provenance the **source-backed definition and script reload core**
  exists to retain: the port resolves a definition group to a `Group`, builds
  the runtime definition and drops the path. So the watcher's registration
  genuinely blocks on that reload core, unlike the
  particle half above, whose stated dependency on it was false. Do the
  provenance thread first and registration becomes a two-line consequence of
  it; do the wiring first and there is nothing to register.
  The delivery half is independent: a poll on the event loop's own cadence
  feeding `developer_reload::changed_file_route`, whose dispatch, network
  refusal and script-host fallback are already ported.

- **File-monitor arming and the external reload trigger landed; the watcher is
  open.** `clonk-engine::developer_file_monitor` ports the two gates.
  A monitor starts only when `Developer.AutoFileReload` is set, the app is
  **not** fullscreen, and none is already running (`C4Game.cpp:2414`) — a
  fullscreen session never watches however the key is set. The external
  `WM_USER_RELOADFILE` payload is accepted only when its **last byte is NUL**
  (`C4Console.cpp:243-249`); C++ tests that byte alone and `break`s otherwise,
  so an unterminated buffer is silently ignored rather than truncated. An
  embedded NUL earlier in the buffer still passes, and the path simply ends
  there, because C++ then reads it as a C string. Pinned by
  `console_auto_file_reload_watches_unpacked_sources_and_dispatches_paths` and
  `external_reload_trigger_validates_path_and_reload_particle_is_name_based`.
  **Still open:** the watcher itself, its app-thread delivery, and
  `ReloadParticle` — the last of which needs the definition reload from the
  source-backed reload core.
  Which definitions get registered once it is armed is ported too
  (`C4Def.cpp:547-560`): only **unpacked** groups — a packed `.c4d` has no
  directory to observe — and only a **new** location, so reloading from the path
  a definition already has re-registers nothing. The ordering is the trap: C++
  computes the flag *before* `SCopy` overwrites `Filename`, and evaluating it
  afterwards would compare the group's name against itself, always be false, and
  silently watch nothing at all.

- **Viewport lock, scroll ranges and input routing landed; windows open.**
  `clonk-engine::developer_viewport` ports `C4Viewport::TogglePlayerLock`
  (`C4Viewport.cpp:250-267`) with its asymmetry intact: unlocking always
  succeeds, but locking requires `ValidPlr(Player)`, so an **ownerless
  (`NO_OWNER`) viewport can never be locked** and keeps free scroll — the call
  still reports success, so a caller cannot detect the refusal from its return
  value. Locked viewports hide their scroll bars, and
  `ScrollBarsByViewPosition` refuses outright while locked (`:272`); unlocked,
  each bar spans the landscape with the view extent as its page and the view
  origin as its position. Input routes by cursor mode — Play to ordinary mouse
  control, Edit and Draw to the editor sink. Pinned by
  `console_viewport_windows_route_redraw_resize_close_and_input_by_window_id`.
  The window a viewport materialises with is ported too
  (`C4Viewport.cpp:1350-1351`): `ceilf(400 * scale)` by `ceilf(250 * scale)` —
  **ceiling**, so a fractional scale always rounds the window up — titled
  `IDS_CNS_VIEWPORT` when ownerless and after the **player's name** otherwise.
  Its remembered geometry is keyed `Viewport{Player + 1}`, so an ownerless
  viewport is `Viewport0` and player 0 is `Viewport1`, stored under the
  **`Console`** config subkey with `storeSize` set. Keying on the player rather
  than a list index is what keeps duplicate-owner viewports from colliding.
  The lock and the scroll are now **wired**. `PhysicalViewportState` carries
  `C4Viewport::PlayerLock`, which starts set (`C4Viewport::Default`, `:1272`)
  and travels into `ViewportInput`, and an unlocked *owned* viewport skips the
  player-follow — exactly C++'s `if (PlayerLock && ValidPlr(Player))` gate
  (`:1162`) — and keeps the position it already had. It is **not** clamped
  afterwards: `UpdateViewPosition`'s clamp block is gated on
  `fIsNoOwnerViewport` (`:1234-1236`), so an owned viewport is allowed a view
  outside the landscape and grows its borders around it instead (`:1256-1260`).
  Clamping there would snap the view the moment the lock came off, which is how
  the first cut of this got it wrong. `GraphicsSystem::scroll_detached_viewport`
  moves that camera's `ViewX`/`ViewY` the way `WM_HSCROLL`/`WM_VSCROLL` assign
  them (`:125-146`), carrying `dViewX`/`dViewY` with it so a later locked frame
  does not interpolate the scroll away, and the step is applied **unclamped**,
  as the line buttons do. `scroll_ranges` is the refusal: it returns `None`
  exactly when `ScrollBarsByViewPosition` returns false. Pinned by
  `console_viewport_scrolls_only_once_its_player_lock_is_off`.
  The **presentation is invented**, and has to be: the reference macOS build
  compiles `TogglePlayerLock` and `ScrollBarsByViewPosition` as
  `{ return false; }` (`:634-635`), so there are no bars to port. A wheel notch
  is mapped onto the one step size C++ does define, `ViewportScrollSpeed = 10`
  (`:57`, and GTK's `step_increment` at `:316,328`), and Scroll Lock toggles the
  lock as the Win32 handler binds `VK_SCROLL` (`:83-86`). Pinned by
  `developer_viewport::wheel_scroll_step`'s cases in
  `console_viewport_windows_route_redraw_resize_close_and_input_by_window_id`.
  **Still open:** drawn scroll bars, and the `Viewport{Player + 1}` geometry
  persistence, which is Windows-only behaviour (see the frame-selection card).

- **Component-host edit model landed; the editors themselves open.**
  `clonk-engine::developer_components` ports `C4ComponentHost`'s commit and save
  rules. OK replaces the bytes and sets `Modified`
  (`C4ComponentHost.cpp:330-334`) — including when the text is unchanged, since
  C++ does not compare; Cancel mutates nothing, not even the flag. Saving
  (`:231-236`) has two behaviours a naive port loses: an **unmodified host is
  skipped entirely**, which is what stops a save touching components the user
  never opened, and an **emptied host deletes the component** rather than
  writing a zero-byte file. Only the Script editor relinks the script tree
  (`C4Console.cpp:1328-1351`), and all three are refused outright in a network
  game.
  The Script commit is now wired through the engine.
  `Engine::apply_scenario_script_edit` reproduces `C4Console::EditScript`
  (`C4Console.cpp:1335-1342`), where two details are easy to lose: it must
  **not** re-run `Initialize` — C++ only replaces the host's `Data` and relinks,
  and the scenario is already running, so re-initialising would recreate its
  objects — and the relink is **unconditional**, because
  `Game.ScriptEngine.ReLink(&Game.Defs)` sits outside the `#ifdef _WIN32` and
  runs even when the dialog was cancelled or never opened.
  `Engine::relink_after_component_edit` is that second arm. Pinned by
  `console_component_editors_commit_bytes_and_relink_script` and
  `console_component_editors_commit_bytes_and_relink_script_through_the_engine`,
  which drives a live engine through two edits and a bare relink.
  The accepted bytes now reach the save.
  `developer_console_save::component_save_mutations` projects each host onto the
  group journal three ways: an **unmodified** host contributes nothing — which
  is what stops a save rewriting components the user never opened — an
  **emptied** one contributes a `DeleteEntry` rather than a zero-byte write, and
  only a modified non-empty host is written, as `PutFile` with
  `FolderSaveAddFailure::Fatal`, because silently dropping a component the user
  just edited would lose their edit. Pinned by
  `edited_component_hosts_reach_the_scenario_save_as_group_mutations`.
  **Still open:** the editor surfaces — which do not exist on the reference
  build; see the console-dialog note above.

- **The edit cursor's overlay draw list landed.**
  Unlike the console's dialogs, `C4EditCursor::Draw` is *not* a native widget —
  it draws through the engine's own rasterizer, so it has an exact pixel oracle.
  `clonk-engine::developer_overlay` ports it: a selection mark per selected
  object in selection order, then the drag frame, then the drag line, then the
  drop-target icon. `DrawSelectMark` is the fiddly part — **twelve individual
  pixels** forming an L at each corner, not a rectangle outline, and nothing at
  all when the shape is under a pixel wide or tall. The drag frame **normalises**
  its corners (`min`/`max`) so dragging up-left frames the same rectangle as
  down-right, while the drag line does not and keeps its direction. Holding
  Shift interleaves an additive re-draw of each object after its mark
  (`ColorMod = 0xffffff`, `C4GFXBLIT_CLRSFC_MOD2 | C4GFXBLIT_ADDITIVE`, both
  restored), rather than appending a second pass. The drop-target icon is
  centred horizontally on the target and rests on top of its shape. Nothing is
  emitted in a fullscreen game, because `C4Viewport::Draw` only calls the cursor
  when windowed. Pinned by
  `console_overlay_emits_marks_frame_line_and_drop_target_in_cpp_order`.

- **Edit-cursor mode, context enablement and gestures landed; overlays open.**
  `clonk-engine::developer_cursor` ports `C4EditCursor::ToggleMode`
  (`C4EditCursor.cpp:540-556`) — Play -> Edit -> Draw -> Play, gated on
  `EditingOK()`, which is just `Console.Editing` (`:683-692`); a refused toggle
  leaves the mode alone *and clears `Hold`*. It also ports the viewport context
  entries (`:594-605`): Delete and Duplicate need a selection **and** editing
  rights, Contents additionally needs the *first* selected object to hold
  something, and **Properties is gated on mode alone** — enabled with no
  selection and without editing rights, disabled only in Play. Its caption also
  switches: outside Edit mode the entry reads `IDS_CNS_TOOLS`, not
  `IDS_CNS_PROPERTIES` (`:605`).
  The pointer gestures are now ported too. `edit_target` mirrors
  `C4EditCursor::Move`'s `do`/`while` (`:143-151`): the target is picked at
  least once, Shift resumes after `Selection.Last` rather than from the top and
  keeps advancing past anything already selected, and — easy to get wrong —
  there is **no wrap-around**, so an all-selected stack ends at `nullptr`.
  `edit_move` gives a held non-frame drag `MoveSelection(xoff, yoff)` while a
  rubber-band drag keeps re-targeting, and `edit_tick_move` reproduces
  `Execute`'s **zero-offset** `EMMO_Move` re-issued every tick while `Hold` is
  set (`:65-69`) — a stationary held selection still emits control traffic.
  `drop_target` ports `UpdateDropTarget` (`:653-670`): Ctrl plus a non-empty
  selection, then the first object in `Game.Objects` order whose shape rectangle
  contains the cursor, skipping deleted, contained and selected ones; the
  rectangle is half-open (`Inside(.., 0, Wdt - 1)`), so a zero-area shape
  contains nothing. `edit_release` emits `FrameSelection` before the
  `EMMO_Enter` of `PutContents`, both optional and in that order (`:672-682`).
  `EditCursor()` already reaches the hovered object —
  `Engine::set_edit_cursor_target` feeds the host world context. Pinned by
  `console_edit_cursor_selects_cycles_drags_and_emits_cpp_ordered_controls`.
  The mode-change publication is ported too (`C4EditCursor::SetMode`). Three
  details: `Console.UpdateModeCtrls` runs **before** the unchanged-mode early
  return, so it fires even when nothing changed; entering Draw clears the
  **Property** page while entering Edit or Play clears the **Tools** page; and
  the toolbox reopens *only* when one of the two was already active — a mode
  switch never opens it from nothing. Play shows the mouse cursor, Edit and Draw
  hide it, and the focused window is saved and restored around the switch so it
  is never stolen from the console.

- **The viewport draw order and the console overlay's place in it landed.**
  The console's edit cursor draws *inside* the ordinary viewport pass, not on a
  finished frame, so where its hook sits is a parity question.
  `clonk-frontend::viewport_draw_order` ports `C4Viewport::Draw`'s phase
  sequence (`C4Viewport.cpp:1023-1119`). Three things a from-scratch version
  gets wrong: the hook goes **after** the foreground and custom-GUI objects but
  **before** `DrawOverlay`, the per-player HUD — last would draw over the HUD,
  earlier would let the HUD cover it; it runs **after the border inset is
  undone** (`:1093-1099`), so the console draws across the whole viewport
  including the border strips the world was clipped out of; and it is gated on
  `!Application.isFullScreen`, *not* on `fDrawOverlay` — so a full-map
  screenshot pass still draws it while dropping borders, clipper and HUD. Fog of
  war is disabled before both, so neither is modulated. Cursors are skipped only
  when a film **and** a replay (`if (!Film || !Replay)`). Pinned by
  `detached_viewport_overlay_hook_precedes_player_hud`.
  Addressing by identity landed with it. `ActiveViewportProjection` now carries
  the concrete viewport's `identity`, and `viewport_projection_for_identity`
  resolves it. That matters because both handles a caller previously had are
  wrong for a detached window: `index` is the *rendered layout* order and moves
  whenever the layout is recalculated, and `owner` repeats when two viewports
  follow the same player — exactly what a console second window on an
  already-viewed player produces. The camera store was already keyed by
  `CameraKey::Physical`; what was missing was exposing that identity to the
  caller. Pinned by
  `detached_viewport_projection_is_addressable_by_physical_identity`, which
  renders two same-owner viewports, swaps their layout order, and checks the
  indices move while the identities do not.

- **Identity-addressed detached rendering landed, and is complete.**
  `GraphicsSystem::render_detached_viewport` draws exactly one physical
  identity into a window-sized target and hands back the pixels plus the
  `ActiveViewportProjection` they were drawn with, the way `C4Viewport::Execute`
  selects that viewport's own context, sets `cgo` from its own six numbers and
  blits it (`C4Viewport.cpp:1126-1155`). Selecting a context is literally what
  the Rust surface swap models — `CStdGLCtx::Select` rewrites the primary
  surface's `Wdt`/`Hgt` to the window's own extent (`StdGLCtx.cpp:467-476`).
  An identity that is not in the supplied list draws **nothing** rather than
  falling back to the first viewport, so a closed viewport's window goes blank
  instead of showing somebody else's view. The pass saves and restores the
  fullscreen records, which is C++-faithful rather than merely defensive:
  fullscreen and console viewports are mutually exclusive in one process
  (`C4GraphicsSystem.cpp:231-234`).
  Three fullscreen-only behaviours had leaked into it and are now gated on
  `Application.isFullScreen`, all three verified against the oracle:
  `C4GraphicsSystem::RecalculateViewports` — the sole writer of the
  landscape-extent cap, the layout cell and `DrawX`/`DrawY` — opens with
  `if (!Application.isFullScreen) return;` (`C4GraphicsSystem.cpp:335-336`), so
  a console viewport is never capped to the landscape and always draws at its
  target's origin; the message board and upper board are inside the same gate
  (`:171-183`), so a detached window reserves no height for them; and
  `C4Viewport::UpdateViewPosition` centres an ownerless view on an undersized
  map only `if (Application.isFullScreen)` (`C4Viewport.cpp:1237,1246`) —
  otherwise it runs `min` then `max` and pins the origin at 0. Pinned by
  `detached_viewport_render_targets_only_requested_physical_identity` and
  `detached_viewport_window_is_never_capped_or_centred_on_a_small_map`.
  `ActiveViewportProjection::pointer_projection` closes the loop: the frame a
  window drew is the frame its pointer input converts through.
  **Still open:** the OS windows that would consume it (console viewports
  materialised as detached windows).

- **Per-viewport pointer projection landed.**
  `clonk-frontend::viewport_projection` ports `C4Viewport`'s local-to-world
  conversion (`C4Viewport.cpp:112,181,192`):
  `ViewX + static_cast<int32_t>(local / scale)`. Two details a from-scratch
  implementation gets wrong: the division is floating point and the cast
  truncates **toward zero**, not floors — so a pointer above or left of the
  window projects differently — and the view origin is added *after*
  truncation. A non-finite or non-positive scale yields the view origin rather
  than a wild coordinate. Pinned by
  `detached_viewport_pointer_projection_uses_window_identity_and_scale`.

- **Console viewport windows now open (creation, reconciliation, teardown).** The
  console's Viewport menu already created the *logical* physical viewport; it
  now materialises as a real OS window. `clonk-app::console_viewport_windows`
  reconciles the open windows against the physical list each pass and
  `viewport_window_host` is the port's `C4ViewportWindow`. C++ has no
  reconciliation step — `CreateViewport` builds the window inside the same call
  that appends the viewport (`C4GraphicsSystem.cpp:229-240`) — but winit can
  only create a window from the event loop's target, so the same decisions are
  taken once per pass instead. Details worth keeping:
  - **Identity is the C++ pointer.** Opens and closes address one viewport by
    `physical_identity`, never by owner, so two windows on the same player stay
    distinct. `GameApp::close_physical_viewport_identity` is
    `CloseViewport(C4Viewport *)` (`:205-224`): it erases exactly one, and it
    has no `fSilent` parameter at all, so it always plays — unlike the
    player-keyed overload (`:314-331`) that erases every match.
  - **Redraws ride the graphics tick**, the way `C4GraphicsSystem::Execute`
    runs `cvp->Execute()` for every viewport inside one pass (`:167-169`).
    Redrawing per event-loop pass instead ignores the frame schedule, the
    automatic frame skip and the repaint floor, and spins — that was written
    and fixed before landing.
  - **The window is not a child.** `C4Viewport::Init` passes the console shell
    as `pParent` (`C4Viewport.cpp:1351`), and the reference `CStdWindow::Init`
    accepts it and ignores it entirely (`StdSDLWindow.cpp:52-66`).
  - The buffer extent is `ceilf(drawable / scale)` (`C4Viewport.cpp:798`),
    pinned by
    `viewport_logical_extent_is_a_ceiling_division_by_the_application_scale`;
    the open/close decision is pinned by
    `console_viewport_windows_open_per_identity_and_close_only_their_own`; the
    draw by `console_viewport_render_uses_the_windows_own_extent_and_identity`.
  **Verified live, drawing real world.** `/console <scenario> <player.c4p>`
  opens a window titled after the player at exactly 400x250 and presents the
  revealed landscape, sky, structures and per-viewport HUD every graphics tick.
  An earlier `--sandbox` run showed only a uniform `fow_color` fill, which was
  correct — that player has no crew, so nothing is revealed — but it meant the
  content was unconfirmed until a real player joined. A player file needs no
  tooling to make: it is an unpacked directory holding `Player.txt` with a
  `[Player]` section, plus one `<Name>.c4i/ObjectInfo.txt` per crew member.
  That is worth knowing, because "no player file available" is what blocked
  every headed check of this subsystem.
  **Still open on this card:** routing the window's pointer and key input into
  the edit-cursor sink. The window delivers events and
  `ActiveViewportProjection::pointer_projection` converts them, but nothing
  consumes them yet — see the next entry.

- **Clicking inside a console viewport window now selects (the edit-cursor
  gesture, reached from a real window).** `GameApp::console_viewport_press`
  joins the four pieces: a window-local pointer converted through *that*
  viewport's own `ViewX`/`ViewY` (`C4Viewport.cpp:181`), the target picked by
  `edit_target` over `EditCursorHitTest` (`C4EditCursor.cpp:150`), the press
  applied by `edit_press` (`:201-229`), and the result written to the shared
  `DeveloperSelection` as `SelectionWriter::EditCursor`. Pinned end to end by
  `console_viewport_pointer_gestures_select_move_and_frame`.
  One design point the port had to settle: `viewport_projection_for_identity`
  reads `GraphicsSystem::active_viewports`, which is the **fullscreen** layout
  and is never populated in console mode — so a detached window had no source
  for its own view origin. `GameApp::console_viewport_projections` now retains
  the projection each window was last drawn with, keyed by physical identity.
  That is the port's form of C++ getting it for free from the `C4Viewport`
  object that both draws and handles input.
  The overlay hook is wired too: `render_console_viewport` calls
  `developer_overlay`'s draw list where `C4Viewport::Draw` calls
  `Console.EditCursor.Draw(cgo)` — after the foreground objects, before the
  per-player HUD, gated on `!Application.isFullScreen`
  (`C4Viewport.cpp:1102-1108`). Only `SelectMark` is painted; the other
  commands need the drag gestures, and drawing half a rubber band would be
  worse than drawing none.
  Resolving the mark's rectangle needed one addition. `DrawSelectMark` frames
  the object's **live** `C4Shape` — stretched by `Con`, rotated by `r` — and
  `ObjectSnapshot::current_shape` carries that only when it is not
  reconstructible, so it is usually `None`. `EditCursorHitTest::shape_rect`
  resolves it through the *same* world view the hit test uses, which is what
  makes the mark and the click agree about what was clicked.
  Verifying that answered itself and found a bug. The invisible mark was two
  separate things: the test's ownerless viewport had `ViewX = 864` while the
  object sat at `x = 240`, so the mark was correctly computed and legitimately
  off-screen — *and* `object_live_shape_rect` returns the shape in **world**
  coordinates (`cobj->x + cobj->Shape.x`, the whole left-hand side of C++'s
  expression), so adding the object's position again double-counted it and put
  the mark an object-width from where it belonged. Both are fixed and the
  coordinate convention is now pinned, including that the corner Ls point
  *outward* — they reach one pixel beyond the shape on each side and no
  further, which is what catches a displaced mark.
  **The mark reaches pixels.** `a_selected_object_draws_its_mark_into_the_viewport_frame`
  renders a console viewport, selects an object inside the view, renders again
  and asserts the frames differ — then clears the selection and asserts the
  frame goes back to *exactly* the unmarked bytes, so the difference is the
  mark and not per-frame drift. The viewport has to be an **owned** one for
  this: an ownerless viewport is centred on the map, and a mark on an object
  outside that view is legitimately clipped away, which is what made an earlier
  attempt at this test pass for the wrong reason.
  **The rubber band is complete.** `console_viewport_motion` and
  `console_viewport_release` carry `C4EditCursor::Move`'s Edit arm
  (`C4EditCursor.cpp:129-152`) and `LeftButtonUp`'s (`:287-341`): a press on
  empty space arms the band with **both** corners at the press (`X2 = X;
  Y2 = Y`), motion drags the live corner while the anchor stays put, and the
  release runs `FrameSelection` over `Game.Objects` master order — the reverse
  of the snapshot's draw order — then clears `Hold` and `DragFrame`
  *regardless*, as C++ does. Covered by
  `console_viewport_pointer_gestures_select_move_and_frame`, which now walks
  the whole gesture.
  **Dragging moves the selection, as a control.** A held non-frame drag routes
  `edit_move`'s `MoveSelection(xoff, yoff)` into `EMMO_Move` through the same
  `submit_or_execute_editor_selection_script` path `EMMO_Script` already used
  — editing is a *control*, not a direct mutation, so a network game stays in
  lockstep. The offset is the delta from the previous pointer message, and a
  motion that moved nothing emits nothing: the zero-offset re-issue is
  `Execute`'s per-tick path (`edit_tick_move`), not this one.
  **The edit cursor's input side is complete.** `UpdateDropTarget` recomputes
  on every motion, before the drag arms decide anything
  (`C4EditCursor.cpp:653-670`), and the release emits `PutContents`'
  `EMMO_Enter` after `FrameSelection`, both optional and in that order
  (`:674-677`). `edit_tick_move`'s zero-offset re-issue is wired too, with one
  port-specific guard: `C4Console::Execute` runs `EditCursor.Execute()` once
  per application tick, while the port's event loop wakes far more often, so
  the emit is keyed to the engine frame. Without that it would flood the
  control queue — the kind of divergence that looks like a performance bug
  rather than a parity one.
  So every `developer_cursor` entry point now has a production caller:
  `edit_press`, `edit_target`, `edit_move`, `edit_tick_move`, `edit_release`,
  `drop_target` and `frame_selection`, plus `developer_selection`'s mutators
  and `developer_overlay`'s mark. `EMMO_Move` and `EMMO_Enter` both reach the
  control queue through the path `EMMO_Script` already used.

- **The edit-cursor interaction layer is wired (this entry is kept for the
  history of how it read before).** Worth
  stating on its own, because every card above reads as "landed" and the
  editor still cannot edit. `grep` for the modules outside their own files
  returned one hit before the click path above landed. `edit_target`,
  `edit_press`, `DeveloperSelection::replace`/`toggle`/`clear` now have a real
  caller; `edit_move`, `edit_tick_move`, `edit_release`, `drop_target`,
  `frame_selection`, `select_frame` and all of `developer_overlay` still do
  not. `developer_tools` is reached only by
  `clonk-app::developer_tools_page`, which is itself a specification of the
  page rather than a rendered one. On the control side only `EMMO_Script` is
  wired (`clonk-app::main.rs`, the console's script input); `EMMO_Move`,
  `EMMO_Enter` and `EMMO_Remove` have no emitter.
  This is not a gap in any one piece — each ported the behaviour it was scoped
  for, and each is pinned by tests. The missing piece is the *caller*, and
  there is exactly one reason there is no caller: a console viewport window is
  where all of it would be driven from, and no such window is ever created.
  Materialising those windows is what turns this group
  from a tested library into a working editor; nothing else in the group is
  blocked on anything but it.
  The window half is now done, so what remains is the bridge. Two concrete
  pieces, both scouted:
  - **`C4EditCursor::LeftButtonDown`'s Edit arm is ported** as
    `developer_cursor::edit_press` (`C4EditCursor.cpp:201-229`), pinned by
    `edit_press_selects_toggles_and_arms_the_rubber_band_like_cpp`. Two details
    it keeps: a plain click on an *already selected* object changes nothing
    (C++ guards the replace on `!Selection.GetLink(Target)`), which is what lets
    a multi-object selection be dragged as a unit; and the whole Ctrl branch is
    inside `if (Target)`, so Ctrl-clicking empty space neither clears nor starts
    a rubber band where a plain click there does both.
  - **The hit test now reaches outside the script host.**
    `EditCursorHitTest::new(&snapshot).object_at(x, y, after)` supplies
    `edit_target`'s `find_next`, which C++ writes as
    `Game.FindObject(0, X, Y, 0, 0, OCF_NotContained, …, ANY_OWNER, Target)`
    (`C4EditCursor.cpp:150`). It runs the **same** query script content calls
    — `compat::objects::find_object_linear` with `FindObjectParams` — rather
    than a second hit test that could disagree with it. The blocker was that
    `find_object_linear` needs a `WorldAccessor` and only `with_host_context`
    supplied one; `HostWorldContext` is itself a `WorldAccessor`, and
    `host_world_context_from_snapshot` builds one without entering the script
    host, which is correct because the console hit-tests *between* ticks, not
    during a script call. It is a struct rather than a bare function on
    purpose: `edit_target` calls `find_next` repeatedly to walk a shift-click
    stack, so the world view is built once per gesture. Pinned against a live
    fixture world by
    `edit_target_walks_the_live_object_stack_through_the_hit_test`, which also
    covers the no-wrap-around rule — a fully selected stack ends at `None`.
    **Still open:** the app-side glue — a viewport window's pointer events
    projected through `pointer_projection`, fed to `edit_press`/`edit_target`,
    applied to `DeveloperSelection`, and emitted as `EMMO_Move`/`EMMO_Enter`.
    Every piece it needs now exists and is tested; none of them are called yet.

- **Frame-selection membership landed; three editor gaps remain untracked.**
  `DeveloperSelection::select_frame` took the framed objects *from its caller*
  and nothing computed them, so a rubber-band drag drew a band and selected
  nothing even once wired. `developer_cursor::frame_selection` ports
  `C4EditCursor::FrameSelection` (`C4EditCursor.cpp:460-471`). Three details a
  from-scratch version gets wrong: the test is on the object's **own `x`/`y`**,
  not its shape rectangle — a wide object centred outside the band is not
  framed even though its graphic covers it, which is why the candidate type
  carries no shape at all; `Inside` is `>= lbound && <= rbound`
  (`C4Math.h:22`), so an object exactly on an edge is admitted and a zero-area
  band still frames what sits under the cursor; and the band is normalised per
  axis inside the `Inside` call, so every drag direction frames the same set.
  `cobj->OCF & OCF_NotContained` is just `!Contained` — that bit is set from
  nothing else (`C4Object.cpp:636-637,735-736`). Objects are appended with
  `C4ObjectList::stNone`, which does not sort, so master order is preserved.
  Pinned by
  `frame_selection_admits_master_order_positions_inside_the_normalised_band`.
  **Still open, and owned by no queue card:**
  (a) **closed** — `MessageManager::update_def` ports
  `C4GameMessageList::UpdateDef` (`C4GameMessage.cpp:233-244,340-345`), which
  `C4Game::ReloadDef` runs as its **last** act after *either* arm
  (`C4Game.cpp:2364`). A decoration the definition still supplies is
  re-resolved and kept; one it no longer supplies is **deleted** rather than
  left drawing from a definition that is gone, and decorations sourced from
  other definitions are untouched. Pinned by
  `a_removed_definition_drops_the_frame_decorations_it_supplied`;
  (b) **closed** — the console frame-tick order is ported as
  `developer_cursor::console_tick_steps` (`C4Console.cpp:1630-1639`). The order
  is not the obvious one: in console mode the **graphics pass is driven by the
  console tick and runs last**, after the edit cursor, which is why a selection
  resolved this tick shows in the same frame's overlay rather than the next. In
  fullscreen the driver is `C4FullScreen::Execute` and this sequence does not
  run at all. `PropertyDlg.Execute()` sits inside `#ifdef _WIN32`, so the
  reference build runs four steps, not five. Pinned by
  `console_tick_runs_the_edit_cursor_before_the_graphics_pass`;
  (c) `C4ViewportWindow::GetPositionData` is `#ifdef _WIN32` (`C4Viewport.h:39-49`;
  `StorePosition`/`RestorePosition` exist only in the HWND `StdWindow.cpp`), so
  the arm64 macOS reference build never remembers viewport geometry. The landed
  `developer_viewport::viewport_window_spec` fields `position_id`,
  `position_subkey` and `store_size` therefore describe **Windows-only**
  behaviour — wiring them to config on macOS would be a divergence, not parity.

- **`ReloadParticle`'s engine half landed (the particle steps of dev-mode
  reload).** `Engine::reload_particle` ports `C4Game::ReloadParticle`
  (`C4Game.cpp:2369-2394`). It was recorded as depending on the source-backed
  definition reload core; that is false — `C4ParticleDef::Reload` needs only `Filename` plus
  `C4Group::Open` and `Load` (`C4Particles.cpp:194-205`), all of which existed.
  What was missing was the filename: `ParticleDef` now carries a `source_path`,
  set by `register_resource_from`.
  Four behaviours a plausible port softens, all pinned by
  `reload_particle_refuses_network_and_clears_everything_on_failure`:
  the **network refusal is the first line**, before the name check and any
  lookup; an **unknown name is a plain `false`** that reloads nothing and
  clears nothing; a **failed reload clears every particle in the system**, not
  just this definition's, and then removes the definition; and
  `C4ParticleDef::Reload` **refusing for want of a filename is itself a failed
  reload**, so it takes that same destructive arm rather than returning early.
  One ordering trap: `Reload` mutates the definition **in place**, so its
  position in `pDef0..pDefL` is unchanged. The port's registration pushes to
  the tail, so `restore_def_order` puts it back — otherwise every later
  definition shifts and `GetDef` finds a different one for a duplicate name.
  **Landed.** `FnReloadParticle` (`C4Script.cpp:5161-5165`, not the
  `:4992-4996` the port's comment claimed) returns `Game.ReloadParticle`'s
  result **synchronously**, which the staged-command channel cannot do — it
  applies after the call has returned. The answer therefore comes from state
  seeded *before* the call, the same shape the port already uses to let
  `CreateObject` return a reference to an object the engine has not made yet
  (`next_object_id`). `HostWorldContext` now carries the definitions that could
  reload — those holding a `Filename`, since `C4ParticleDef::Reload` refuses
  without one (`C4Particles.cpp:197`) — alongside the network flag it already
  had. The builtin answers from those, stages the accepted name on the existing
  `host_requests` channel (the one `PauseGame` uses), and
  `Engine::apply_particle_reload_requests` does the work afterwards, drained
  once per pass beside `apply_engine_pause_game_requests`.
  No `EffectContextOutcome` plumbing was needed. An earlier note here proposed
  it and was **wrong**: `host_requests` is purpose-built for exactly this and
  cost one field, one builder and one drain.
  Every `false` C++ produces is reproduced exactly — network game, nil name,
  unknown name, and a definition with no `Filename`. **The residual divergence
  is one case:** a reload that passes all four checks and then fails on I/O
  reports `true` to the script where C++ reports `false`. The engine still runs
  the full failure arm (every particle cleared, the definition removed); only
  the value the script already received is optimistic.
  Pinned by `reload_particle_answers_synchronously_and_the_engine_applies_it`
  (a live script call through a real host context) on top of the frozen
  `reload_particle_reports_false_for_every_name_cpp_cannot_reload`, which still
  passes — so the change could only ever turn a successful reload from `false`
  into `true`, which is what it did.
  `FnReloadDef` (`C4Script.cpp:5143-5159`) landed with it, on the same channel
  and with two details of its own: with **no id** the caller reloads its *own*
  definition (`ctx->Obj->Def`, `:5146-5151`), and a missing definition is a
  plain `false` rather than an error. Pinned by
  `reload_def_answers_synchronously_and_defaults_to_the_callers_definition`.
  One divergence applies to both and is worth stating plainly: C++ reloads
  *inside* the call, while the port does the work on the next pass, drained
  beside `apply_engine_pause_game_requests`. The script's answer is unaffected —
  only the moment the definition changes moves, by at most one pass, and a
  console reload is not a synchronised operation.

- **Live-reload path matching landed; the reload itself is open.**
  `clonk-engine::developer_reload` ports `C4DefList::GetByPath`
  (`C4Def.cpp:1137-1152`), which decides whether a changed file names a loaded
  definition. The rule is narrower than it looks: a path matches only the
  definition **root** or **exactly one component below it** — a grandchild such
  as `Rock.c4d/Graphics/Overlay.png` does *not* match and falls through to the
  generic script-host reload (`C4ScriptHost.cpp:135-149`). Comparison is
  case-insensitive (`SEqual2NoCase`), and a prefix that stops mid-component
  (`Rock.c4d` against `Rock.c4dx`) is rejected because the following byte is
  neither NUL nor a separator. Pinned by
  `definition_path_matches_only_the_root_or_one_immediate_child` and
  `definition_lookup_returns_the_first_match_in_list_order`. **Note the ticket
  cites `C4Def.cpp:1158-1175` for this, which is `CheckRequireDef` — the wrong
  function.**
  `C4Game::ReloadDef`'s surrounding policy is ported too. The network refusal is
  its *first* line, so a network game never reloads whatever changed on disk;
  the synchronise that follows is `Synchronize(false)`, which closes menus
  holding dead surfaces but deliberately does **not** write player files back.
  The two outcomes are symmetric sweeps over every object of that id in master
  order, and both are blunter than they look: on success **all** of them get
  `UpdateFace(true)` — C++ does not work out which are affected, because an
  object can use another definition's graphics — and on failure **all** of them
  are removed, the script profiler is aborted, and the definition itself is
  dropped from the list. `Messages.UpdateDef(id)` runs after either arm. Pinned
  by `console_definition_reload_refuses_network_and_sweeps_every_matching_object`.
  The watcher's dispatch is ported too. `C4Game::ReloadFile`
  (`C4Game.cpp:2306-2319`) refuses in a network game, converts the path with
  `Config.AtExeRelativePath` **before** matching — so an absolute watcher path
  never reaches `GetByPath` — and falls through to
  `ScriptEngine.ReloadScript` for anything no definition owns; the script host
  is the fallback, not a sibling branch. `C4Game::ReloadParticle` is blunter
  than it looks: an unknown name reloads nothing, and a *failed* reload clears
  **every particle in the system**, not just that definition's, then deletes the
  definition. Pinned by
  `external_reload_routes_by_definition_and_clears_particles_on_failure`.
  `C4DefList::Reload`'s sequence is ported as well, and its order is
  load-bearing in three places. `SortByID` rebuilds the quick-access table
  **before** the relink, so the relink sees the definition at its final
  position; `ReLink` runs **before** graphics are restored and "will also do
  include callbacks", so a script inspecting graphics from an include callback
  sees the *backed-up* set, not the reloaded one; and graphics are restored last
  via `AssignUpdate`, which remaps live pointers rather than reassigning. On any
  early return — the group failing to open, or `Load` failing — the graphics
  backup's destructor resets every graphic to default, and `Clear` deliberately
  keeps the filename, which is what lets the reload re-open the group it came
  from. Pinned by `definition_reload_relinks_before_restoring_graphics`.
  **Source provenance is retained now (the reload core's first step).** `Definition`
  carries a `source_path`, set at the install site from the
  `ScenarioDefinition::resource_group` the loader already held — the group's
  own root, which is what `C4Def::Load` stores as `Filename` (`C4Def.cpp:550`).
  That one field is what `C4DefList::Reload` re-opens, what `C4Def::Clear`
  deliberately preserves ("Assume filename is being kept"), and what
  `AddDirectoryForMonitoring` watches — so it unblocks the watcher's
  registration as well as the reload. A definition built from script alone
  carries none, which is the case a reload must refuse rather than attempt.
  Pinned by `definitions_carry_the_group_they_were_loaded_from` for the
  accessor contract and, against real content, by
  `reloading_a_shipped_definition_group_rebuilds_it_from_disk`: it points a
  definition at the shipped `Wipf.c4d`, registers it under a placeholder name,
  reloads, and asserts the name came back from `DefCore.txt` — so the rebuild
  demonstrably re-read the group rather than keeping what was registered. That
  closes the coverage this note previously recorded as owed.
  **The reload body landed.** `Engine::reload_definition` re-opens the group
  from the definition's own stored path, rebuilds it through the same
  `ResourceDefinition::load` + `Definition::from_resource` pair production
  loading uses — so DefCore, ActMap, scripts, graphics, portraits and ranks all
  come back by construction rather than through a second, drifting code path —
  and relinks. Three orderings are load-bearing: the re-open uses the stored
  `Filename`, which is exactly why `C4Def::Clear` preserves it; the relink runs
  with the definition back at its final position (C++ calls `SortByID()` before
  `ReLink` for the same reason); and **a failed load removes the definition
  entirely** rather than leaving the old one, because `Clear()` has already
  emptied it and there is nothing intact to keep. A definition with no stored
  group is refused without disturbing anything. Pinned by
  `reloading_a_definition_reopens_its_group_and_removes_it_on_failure`, which
  drives a real on-disk group and then deletes it.
  **The failure sweep is applied.** A failed reload now assigns every object of
  that id for removal before dropping the definition, through
  `definition_reload_outcome`'s plan. It is blunt on purpose: C++ filters on the
  id **alone**, not on `Status` (`C4Game.cpp:2352-2360`), unlike
  `C4ObjectList::UpdateFaces` which does check it — so `object_ids_of_definition`
  deliberately does not.
  **The success sweep landed, additively.** `refresh_object_face_from_definition`
  ports `C4Object::UpdateFace(true)` (`C4Object.cpp:363-386`) as its own engine
  operation, and `reload_definition` runs it over *every* object of that id —
  not a computed subset, because C++'s own comment says why: an object can use
  another definition's graphics, so "better update everything"
  (`C4Game.cpp:2340-2345`).
  It writes only definition projections — shape template, solid-mask override,
  compiled mass — and deliberately leaves `Con`, rotation, position, colour, the
  action index, energy, contents, effects and commands alone: a reload
  *refreshes* an object, it does not reinitialise one. The last argument to
  `refresh_shape_after_state_change` is `false`, which is `UpdateSolidMask`'s
  `fRestoreAttachedObjects` (`:371`) — a reload must not re-attach riders the
  C++ path leaves alone. Pinned by
  `a_successful_reload_refreshes_live_objects_without_reinitialising_them`,
  against the real shipped `Wipf.c4d`.
  This was written **additively** rather than by extracting the ChangeDef
  block it resembles. Extraction would have changed a path
  (`refresh_shape_after_state_change`) that feeds movement and contact for
  every existing caller; building the reload's own operation from the same
  primitives leaves those callers untouched, and the two can be unified later
  under a differential test rather than under time pressure.
  `C4DefGraphicsPtrBackup::AssignUpdate` landed with it
  (`reassign_graphics_after_reload`). Re-resolution is **by name**, not pointer
  patching (`C4DefGraphics.cpp:355-400`): a named graphic that survives the
  reload keeps the object on it; one that is gone falls back to the object's own
  definition; and an object that can do neither is removed rather than left
  holding a name nothing supplies — leaving a dangling name is the divergence.
  It runs *before* the face refresh so the refresh sees settled graphics.
  With that, **the source-backed definition and script reload core is
  complete**: provenance, the rebuild through the
  production loader, removal, the failure sweep, the success sweep and the
  graphics re-resolution. Historical notes on where the work started:
  `UpdateFace(true)` has no callable primitive — but its pieces are not
  scattered: they sit inside the **ChangeDef** path in
  `engine/economy.rs` (around the `object.shape_template = template` assignment),
  which already does the definition-derived refresh for a *different* reason —
  new `shape_template`, `solid_mask_override` reset, the non-rotateable
  rotation reset, and `refresh_shape_after_state_change(..., false)` whose
  `false` is `UpdateFace`'s own `fRestoreAttachedObjects`. Extracting that
  block as one operation both call sites share is the piece of work, and it is
  a **refactor of a determinism-adjacent path** — `refresh_shape_after_state_change`
  feeds movement and contact — so it wants a session with room to run the full
  gates, not a tail-end increment. Left undone deliberately rather than
  attempted badly.
  What must come with it: `C4DefGraphicsPtrBackup::AssignUpdate`'s graphics
  re-resolution is **name-based**, not pointer patching
  (`C4DefGraphics.cpp:355-400`) — a live object whose `graphics_name` is gone
  from the reloaded sprite variants falls back to its own definition, and is
  removed if that also fails. Silently leaving a dangling name is the
  divergence. Until then a successful
  reload replaces the definition and leaves live objects on their old graphics
  and shape. Also still open: the watcher's registration, now a short step
  rather than a blocked one.
  `Engine::remove_definition` landed with it — the exact inverse of
  `register_definition`, unwinding the map, the load order, the id-sorted
  runtime order and the script link source, and invalidating the same caches.
  `C4Game::ReloadDef`'s failure arm removes the definition outright after
  assigning every object of that type for removal (`C4Game.cpp:2352-2360`), so
  a failed reload must not leave the old definition in place. Missing any one
  structure leaves `relink_scripts` walking a host with no definition behind
  it, which is why the test also re-registers the removed id. Pinned by
  `removing_a_definition_unwinds_everything_registration_added`.

- **Deferred runtime config save: mechanism landed, most callers still write
  through.** C++ mutates its process-wide `Config` for ordinary runtime toggles
  and writes once in `C4Application::Clear` (`C4Application.cpp:351-367`); the
  port wrote every toggle straight to disk, so a transient change survived a
  crash C++ would have discarded and each toggle rewrote the whole file.
  `clonk-app::deferred_config` now holds pending `(section, key) -> value`
  writes, replacing rather than queueing a repeated key, and `main` flushes them
  grouped by section on the clean-exit path only — an aborted run discards them,
  which is the behaviour the ticket asks for. Pinned by
  `runtime_config_mutations_remain_process_local_until_shutdown_save`.
  **Migrated so far, each with an oracle citation:**
  `Network.MasterServerSignUp` and `General.Record`
  (`C4StartupNetDlg::OnBtnInternet`/`OnBtnRecord`, `C4StartupNetDlg.cpp:840-850`).
  `General.MissionAccess` was migrated here too and has since been pulled back
  out — see *Earned mission access is written when it is earned* under
  [Deliberate divergences](#deliberate-divergences-from-the-oracle). What it
  contributed and keeps: the live list, not the config *file*, is what the gate
  tests (`C4StartupScenSelDlg.cpp:743`); the network branch used to re-read the
  file instead, so every password earned this session stayed locked in the
  network selector while the local selector already honoured it. Pinned by
  `network_mission_access_gate_honours_memory_only_grant`.
  The four `[Sound]` toggles defer too: `C4SoundSystem::ToggleOnOff` is
  `enabled = !enabled` with no save (`C4SoundSystem.cpp:138-142`), and neither
  `C4SoundSystem.cpp` nor `C4MainMenu.cpp` contains a `Config.Save()`.
  The `[Startup]` hide-message flags defer too: `ShowMessageModal` takes
  `Config.Startup.HideMsg*` **by pointer** and writes it in memory
  (`C4ChatDlg.cpp:624`), and none of `C4Gui.cpp`, `C4GuiDialogs.cpp` or
  `C4ChatDlg.cpp` contains a `Config.Save()`. Six tests asserted the file was
  written immediately; two now assert the pending value and four flush
  explicitly before reloading, since their subject is the written *content*
  rather than the timing. All had pinned the divergence. The IRC preference is
  deliberately left writing eagerly — it goes through
  `persist_irc_warning_preference`, a different native path.
  **There are two flush points, not one.** `C4StartupOptionsDlg::SaveConfig`
  ends with an outright `Config.Save()` — "make sure config is saved, in case
  the game crashes later on" (`C4StartupOptionsDlg.cpp:1188-1189`) — so leaving
  the Options dialog is an explicit save surface, not something to defer.
  `close_options_menu` now flushes the pending store alongside its existing
  options write, and the clean-shutdown path flushes the same store. Deferring
  the Options save to shutdown would have lost exactly the crash protection that
  comment describes. **Still open:** every other
  `persist_config_value` caller needs its own C++ site read before being moved —
  Participants, sound toggles, ServerAddress and the Startup
  checkboxes are *not* settled by the oracle lines this ticket cites, and the
  many `main_tests` callers are fixture set-up that must keep writing
  immediately.

- **A proven C++ defect: `c4group -g` produces update packages `c4group -y`
  cannot apply.** Reproduced end-to-end against the pinned oracle build
  (`build-arm64-native/c4group`, arm64 macOS). `C4UpdatePackage::MkUp` builds
  each `GRPUP_Entries.txt` record with
  `std::format("{}={}", strItemName, ...)` where `strItemName` is a
  `char[_MAX_PATH]` whose only initialised byte is `[0]`; the format writes the
  **whole array**, so every record carries about a kilobyte of uninitialised
  stack memory between the name and its `=`. `DoGrpUpdate` then matches those
  names against real entries with `SEqual`, matches nothing, and **deletes every
  entry of the target group**, so the update fails. The same omission is in
  `C4UpdatePackageCore`'s constructor, which initialises `GrpChks1` but not
  `GrpContentsCRC1`/`GrpContentsCRC2`, leaving fifty uninitialised words in
  `AutoUpdate.txt`; `Check` compares against them and only works by falling
  through to its `GrpChks1` comparison.
  Two consequences for the `c4group` CLI port, both of which overturn an
  earlier note on it. **Byte-identical output cannot be the acceptance
  criterion** — three runs of C++ `-g` over identical inputs produce three
  different files, because the garbage differs per run. And writing a correct
  manifest is a **fix**, not a divergence needing justification: an update
  package is not simulation state, so it cannot affect determinism.
  `clonk-c4group::update_entries` writes the manifest `MkUp` intends and reads
  the corrupted form tolerantly, since C++-produced packages exist in the wild.
  `clonk-c4group::update_core` does the same for `AutoUpdate.txt`, checked
  against a package the oracle's own `c4group -g` produced. Two traps there:
  the `UpGrpCnt` member is serialised under the key **`TargetCount`**, so
  reading a key named after the member silently yields a zero-target package;
  and only the first `TargetCount` array entries are meaningful, which is what
  discards the uninitialised `GrpContentsCRC1` tail on read. Writing pads
  nothing, so the same inputs give the same bytes every time — unlike C++.
  Pinned by
  `update_entry_manifest_round_trips_and_tolerates_cpp_uninitialised_names`,
  which also asserts that parsing those records *literally* would delete the
  whole group — the observed C++ behaviour.
  `clonk-c4group::make_update` ports `MkUp`'s diff, which decides **two**
  separate things that are easy to conflate. Which entries to *copy*: changed
  when `EntrySize` **or** `EntryCRC32` differs — size-then-CRC, never a byte
  compare — or when there is no source group. And whether the group is written
  at all (`includeInUpdate`): set by a copied entry, but *also* by a header
  difference or an **entry-order** difference on its own, so two groups with
  identical entries in a different order still produce an update. `group_file_crc`
  is verified numerically against the oracle's own output. Pinned by
  `update_plan_copies_changed_entries_and_lists_every_target_entry`.
  **`c4group -g` now works, and works better than C++'s.** Generating a package
  from the same two groups and handing it to the *oracle's own* `c4group -y`
  applies correctly: `a.txt` updated, `added.txt` added, `keep.txt` kept,
  `removed.txt` deleted — where C++'s own package deletes every entry, because
  its manifest is corrupt. Its core matches C++'s field for field
  (`GrpChks1=1686362931`, `GrpChks2=1194512086`).
  **And byte-identical repacking is not required after all** — which finally
  settles a claim this file carried in two earlier forms.
  `C4UpdatePackage::Execute`'s verdict is
  `if ((!GrpContentsCRC2 || GrpContentsCRC2 != iResContentsChks) && iResChks != GrpChks2) return false;`
  — success needs the result's *contents* CRC to match `GrpContentsCRC2` **or**
  its *file* CRC to match `GrpChks2`. The contents CRC
  (`C4Group_GetFileContentsCRC` -> `C4Group::EntryCRC32(nullptr)`) is the
  **XOR** of every entry's CRC, so it is order- and packing-independent: that is
  the escape hatch that lets an equivalent repack pass.
  One trap in it, and it is the whole reason a first attempt fails: an entry's
  CRC is **not** a CRC of its data. C++ computes `crc32(0, data)` and then
  *continues the same CRC over the entry's filename bytes*. `entry_crc` and
  `group_contents_crc` reproduce that, verified numerically —
  `GrpContentsCRC2=3949291798` is exactly that XOR over the fixture's three
  entries.
  With those written, the oracle's own `c4group -y` reports **`Ok`** on a
  Rust-generated package, where its own package fails.
  `-y` is implemented too (`clonk-c4group::apply_update`), reproducing
  `Execute`'s ladder rather than tidying it: already-updated is a *success*, not
  a refusal; the source check consults `GrpContentsCRC1[i]` first — guarded by
  `GrpContentsCRC1[i] &&`, which is the only thing stopping C++'s garbage
  matching by accident — and falls back to `GrpChks1[i]`, which is what makes
  real packages work.
  Verified in **both** directions against the reference tool: Rust `-g` then
  C++ `-y` is `Ok`, and Rust `-y` applies a **C++-generated** package correctly
  — the very package C++ itself cannot apply, because the tolerant manifest
  parse recovers the entry names from its corrupted records. Caveat: reproduced on one
  toolchain; `std::format` over `char[N]` may differ elsewhere.

- **Actionable ready-check toasts: the concurrency core landed, backends open.**
  The lobby ready check already runs as an in-window Yes/No dialog with a
  countdown; making the desktop notification beside it *actionable* means an
  answer can arrive from a backend callback thread while the same question is
  still answerable in-window. That race, not the API shape, is the hard part —
  getting it wrong double-submits to a live protocol request.
  `clonk-app::ready_check_notification::ReadyCheckContinuation` claims the
  answer with a single atomic swap, so exactly one of the dialog thread and any
  number of activation callbacks may resolve it; every later one is dropped.
  Whatever wins also hides the toast, so a stale notification can never answer a
  question that no longer exists, and a `Default` activation (clicking the body)
  closes *without* submitting an answer — it means "come back to the game", not
  "yes". Backend show and hide failures are logged and ignored, because a
  missing notification daemon must not take the lobby down. Pinned by
  `ready_check_notification_actions_answer_and_dialog_close_hides_toast` against
  a fake sink, and by
  `concurrent_ready_check_resolution_submits_exactly_one_answer`, which races
  four threads 64 times and asserts a single winner and a single hide.
  The freedesktop wire encoding is ported too, because it is the part of a Linux
  backend that is both easy to get wrong and testable without a bus.
  `org.freedesktop.Notifications.Notify` takes actions as a **flat
  `key, label, key, label` array**, not as pairs — swapping that shows the key
  as the button text — and `"default"` is the reserved key that fires on a body
  click with no button of its own, which is where
  `NotificationActivation::Default` comes from. An unrecognised `ActionInvoked`
  key is **ignored rather than guessed at**, so another application's action can
  never be read as an answer. For `NotificationClosed`, only reason 3
  (`CloseNotification`) is our own doing and leaves the continuation alone;
  expiry, user dismissal and undefined all end the prompt. Pinned by
  `freedesktop_actions_interleave_keys_and_labels_with_a_default_entry`.
  **Still open:** the D-Bus and WinRT plumbing itself — the `Notify` call, the
  signal listener thread and `CloseNotification`. No Linux target is installed
  here (`rustup target list --installed` shows only darwin and windows), so that
  code could not be compiled, let alone run; writing a signal listener that
  cannot even be type-checked is exactly where the concurrency bugs this core
  exists to prevent would enter.

- **`Network.UseCurl` now selects the HTTP backend, as a policy rather than a
  second stack.** `C4Network2HTTPClient` picks one of two implementations at
  construction (`C4Network2Reference.cpp:410-413`), and they differ on the wire,
  not just internally. curl follows `Location` (`CURLOPT_FOLLOWLOCATION`), keeps
  an in-memory cookie jar (`CURLOPT_COOKIEFILE ""`), reuses connections
  (`CURLOPT_SHARE`) and bounds the connect phase plus a stalled transfer
  (`C4HTTPClient.cpp:189-198`). NetIO does none of that: it writes `HTTP/1.0`
  with `Connection: Close`, has **no `Location` handling at all**, no cookie
  state, and one 20-second query timeout (`C4Network2Reference.cpp:404-405,
  825-856`). `clonk-network::HttpBackend` is that difference, applied to a
  `reqwest` client builder and threaded through `ReferenceQueryConfig` and
  `LeagueHttpTransportConfig` so both reference and league traffic follow the
  configured backend; the key defaults to true (`C4Config.cpp:561`), which is
  the behaviour that already shipped.
  **Deliberate divergence, with a reason.** The alternative reading — port
  `C4Network2HTTPClientImplNetIO` literally — means a hand-written HTTP header
  and gzip parser reading straight off the reference and league sockets,
  duplicating `reqwest` on exactly the paths this repo forbids panicking on.
  The acceptance criterion is observable request semantics, so the policy is
  what is reproduced. Two residuals: `reqwest` cannot emit an `HTTP/1.0`
  request line, so a version-sensitive server still sees `HTTP/1.1` with
  `Connection: close`, and this `reqwest` has no happy-eyeballs builder, though
  its connector default already matches C++'s 300 ms. Pinned by
  `use_curl_false_selects_netio_compatible_http_transport`.

- **Keyed developer window host landed, with the console shell as a live record.**
  The runner owns exactly one Window/Pixels/FramePresenter, so console
  viewports, the Tools/Property toolbox and the object-list window had nowhere
  to live. `clonk-app::developer_windows` is the registry: records keyed by
  `WindowId`, each owning one host plus a `HostPurpose`, and every operation
  addresses exactly one record. Close semantics follow the oracle rather than
  one rule — a viewport or the object list is destroyed
  (`C4Viewport.cpp:775-834`), the toolbox is only *hidden* so its notebook
  pages survive (`C4DevmodeDlg.cpp:79-101`), and the shell ignores a
  child-style close entirely, so closing a child can never take the console
  down. `request_redraw_visible` skips hidden hosts, and `present_visible`
  reports each failure against its own `WindowId`, so one host's lost surface is
  never attributed to another. Pinned by
  `developer_window_host_routes_resize_redraw_hide_and_close_by_window_id`.
  Presenting needed a design fix before any live record was possible. C++ gets
  its drawing state for free — `C4Viewport::Execute` reads the global `Game`, so
  a viewport appears to present itself — while the port passes that state
  explicitly and it differs per purpose. `present` therefore moved to a separate
  `DeveloperWindowPresenter<Ctx>` trait: mocks present with `()`, and
  `shell_window_host::ShellWindowHost` presents with `GameApp` through the
  retained GPU pipeline. Without that split the shell simply could not implement
  the host contract.
  The runner now registers the shell under the reserved `SHELL_WINDOW` key. Its
  window, pixel surface, frame presenter and retained GPU renderer used to be
  four separate locals; they are one record's worth of state — the renderer is
  built from the surface's own device, queue and format — and bundling them is
  what makes them addressable by id. The event loop destructures the record once
  per event, so the per-site borrows are unchanged. This is deliberately pure
  indirection today: with one window it changes no behaviour, and it pays off
  when the console opens its second. **Still open:** the feature-specific
  surfaces themselves — the Tools/Property toolbox, the property and
  object-list windows, and console viewports.

- **The reference C++ build has no console dialog windows at all.** Worth
  stating plainly, because it reframes the whole console-surface group.
  `C4ToolsDlg::Open` (and its Property/object-list siblings) creates a window
  only under `_WIN32` or `WITH_DEVELOPER_MODE`. `WITH_DEVELOPER_MODE` defaults
  to **OFF** (`CMakeLists.txt:205-206`), and the pinned oracle's own arm64 macOS
  build is compiled `WITH_DEVELOPER_MODE:BOOL=OFF`, `USE_SDL_MAINLOOP:BOOL=ON`.
  On that build `Open` falls straight through to `Active = true` plus an ordered
  refresh — `InitGradeCtrl`, `UpdateLandscapeModeCtrls`, `UpdateToolCtrls`,
  `UpdateIFTControls`, `InitMaterialCtrls`, `EnableControls` — and `Clear` drops
  `Active` and nothing else, which is why re-opening restores the previous
  selection rather than the defaults. `C4ToolsDlg::Default`'s starting state is
  Brush, grade 5, IFT on, material `Earth`, texture `Rough`. All of that is now
  in `clonk-engine::developer_tools` (`open`, `clear`, `active`, `material`,
  `texture`), pinned by
  `console_draw_mode_routes_pointer_gestures_through_tools_state`.
  So the "native separate-window/notebook behavior" the Tools-dialog scope asks
  for describes the Win32 and GTK builds, neither of which is the reference
  build — and against the reference build the ported state *is* the dialog. The
  same reading applies to the property and object-list surfaces. Whether those
  count as done on that basis is a scoping decision, not a worker's call.

- **The toolbox notebook landed; its rendered controls are open.**
  `C4DevmodeDlg` (`C4DevmodeDlg.cpp:28-121`) is one shared utility window
  holding the Tools and Property pages in a **tabless** notebook
  (`gtk_notebook_set_show_tabs(FALSE)`), so a page is never picked by clicking —
  the console switches it. `clonk-app::developer_toolbox` ports the four
  behaviours a port loses first: the close button **hides** (its `delete-event`
  handler calls `SwitchPage(nullptr)` and returns `TRUE`, suppressing GTK's
  destroy), the window position is **remembered across hides** in `static` x/y
  and restored on the next show rather than re-centring, the title follows the
  current page's *invisible* tab label, and the window is destroyed only when
  its **last page** is removed — not when it is closed. Capturing the position
  is guarded on visibility, which is what stops a hidden window's stale
  coordinates overwriting a good one. Window chrome (utility type hint,
  `"toolbox"` role, transient-for the console, centre-on-parent) is carried in
  `ToolboxChrome` so the platform layer applies C++'s hints rather than
  inventing its own. Pinned by
  `developer_toolbox_hides_on_close_and_remembers_its_position`.
  The Tools page's own contract is in `clonk-app::developer_tools_page`: the
  fourteen controls in the order `C4ToolsDlg`'s box tree builds them
  (`C4ToolsDlg.cpp:289-377`), and `EnableControls`' rules (`:912-940`), which
  are three rules rather than one. Nearly everything needs
  `Mode >= C4LSC_Static`, but **Fill needs `>= C4LSC_Exact`** — it is the only
  tool absent from a static landscape — and the **texture list additionally
  requires the material not to be Sky**. The three landscape-mode buttons are
  never disabled, which is what stops a dynamic landscape being a dead end: in
  that mode they are the *only* live controls. Win32 selects a disabled bitmap
  from the same predicates, so the enablement answer picks the artwork too.
  Pinned by `tools_page_enables_fill_only_in_exact_and_textures_only_off_sky`.
  **Still open:** rendering the page — the button artwork, the grade scale, the
  material and texture pickers and the preview — and window focus.

- **Developer draw-tool state machine and mode control landed; dialog open.**
  `clonk-engine::developer_tools` carries `C4ToolsDlg`'s retained state and
  `C4EditCursor`'s gesture cadence. `ToggleTool` is `(Tool + 1) % 4`
  (`C4ToolsDlg.h:148`), which never lands on Picker — including from Picker
  itself, where it goes to Line. Grade clamps to 1..50 with five-unit key steps
  (`C4ToolsDlg.cpp:732-737`). The per-tool cadence matches
  `C4EditCursor.cpp:74,159,234,301-304` exactly: Brush emits on click *and*
  every drag step, Line/Rect record an anchor and emit once on release with both
  coordinate pairs **in C++'s argument order** — the *live* cursor first and the
  press anchor second, because `Move` overwrites `X`/`Y` on every pointer
  message before it dispatches on the mode (`:119-121`) while `LeftButtonDown`
  freezes `X2`/`Y2` (`:225-226`), and `ApplyToolLine` passes `(X, Y, X2, Y2)`
  (`:558`). The port had the two pairs the other way round until 2026-08-06;
  nothing drew differently, because `ForLine` and `DrawBox` normalize their
  endpoints, but the bytes on the wire and in a record did. The same fix moved
  Fill onto the live cursor — `ApplyToolFill` reads the same `X`/`Y` (`:579`),
  so a held fill follows the pointer instead of staying at the press. Fill emits
  nothing on the click — it arms `Hold` and
  repeats from `Execute` every frame while the game runs, refusing while halted
  or when the console is not editing. Alt selects the Picker temporarily in Draw
  mode only and restores the previous tool on release
  (`C4EditCursor.cpp:773-792`).
  Landscape-mode changes are now modelled as the *control* they are
  (`C4ToolsDlg::SetLandscapeMode`, `C4ToolsDlg.cpp:865-894`). The local path
  changes nothing: it asks `IDS_CNS_EXACTTOSTATIC` only for Exact -> Static,
  and on confirmation enqueues `EMDT_SetMode` as `CDT_Decide`. All the state
  lives in the `fThroughControl` arm — `landscape_mode_change` sets the mode,
  redraws from the map on Exact -> Static, and corrects the tool, because Fill
  exists only in Exact mode and any other mode falls back to Brush. No other
  tool is corrected, and a mode arriving through the queue is never
  re-confirmed. Pinned by
  `console_draw_mode_routes_pointer_gestures_through_tools_state`. The material
  and texture catalogue and the picker itself already exist in
  `developer_landscape`.
  Draw mode is now **reachable**. `GameApp` holds the `DeveloperTools` — away
  from Win32 and GTK that state *is* the dialog, since `C4ToolsDlg::Open`
  creates no window at all on the reference build (`C4ToolsDlg.cpp:262`) — and
  a console viewport window's press, motion and release route into it whenever
  the console is in Draw mode, alongside the Edit arm they already fed. Each
  `DrawControl` is packed into `C4ControlEMDrawTool` carrying the live landscape
  mode (the executor refuses a packet whose mode no longer matches,
  `C4Control.cpp:1015-1016`) and submitted through the same decided-control seam
  `EMMO_Script` uses, so a network game applies every stroke in lockstep. Four
  details ride with it: the per-frame Fill repeat runs from the console tick,
  once per **engine frame** rather than once per event-loop wake; a halted game
  refuses the Fill click with `IDS_CNS_FILLNOHALT` and never arms `Hold`
  (`C4EditCursor.cpp:227-231`); `ApplyToolPicker` writes its sample back into the
  tools and ends with `Hold = false` (`:731`), so a picker click never starts a
  drag; and Alt drives that picker from a viewport window's own
  `ModifiersChanged`, which the shell never sees.
  Two shared-state details are easy to lose and are ported deliberately.
  `EditingOK` is **not a predicate**: refusing a stroke drops `Hold` and reports
  itself (`:673-682`), so a drag the console may not make stops at the first
  stroke instead of re-asking on every pointer message — `C4Console::Message`
  shows nothing at all on the reference build (`C4Console.cpp:841-853`), so the
  console log is the port's own choice of surface. And `LeftButtonUp` clears
  `Hold`/`DragFrame`/`DragLine` **unconditionally** after dispatching its finish
  on the current mode (`:300-304`); because C++ has one `Hold` for both arms and
  the port has two, a mode change between press and release would otherwise
  strand the tools' gesture. Pinned by
  `console_viewport_draw_gestures_emit_landscape_tool_controls`,
  `console_draw_fill_refuses_while_halted_and_otherwise_repeats_at_the_cursor`,
  `console_draw_alt_picks_the_landscape_into_the_tools_without_drawing` and
  `a_refused_draw_stroke_and_a_mode_change_both_clear_the_held_gesture`.
  **Still open:** the dialog chrome and its window host, gated on the keyed
  developer window-host registry — so tool, grade, IFT, material and texture are
  reachable only through the Alt picker and the ported defaults (Brush, grade 5,
  IFT on, Earth over Rough), and `EMDT_SetMode` still has no emitter, which
  leaves a Dynamic landscape undrawable from the console.

- Closed 2026-07-29: **Options control sheets draw the classic facets.**
  `C4StartupOptionsDlg` draws the Keyboard/Gamepad pages from facets, not text
  buttons (`C4StartupOptionsDlg.cpp:215-345`). Two of the three pieces are in:
  `startup_options_controls::key_button_facets` ports
  `KeySelButton::DrawElement`'s geometry exactly — `fctKey` at phase `fDown`,
  then `fctCommand` at phase `iKeyID` inset by a fifth of the button width
  either side, three quarters of that above, and nudged down half an indent
  while held — and the twelve action labels now resolve through the
  `IDS_CTL_*` table in C++'s order (`:166-169`) via `OptionsLabels::control_keys`
  instead of baked English, falling back to the shipped US text per key. Pinned
  by `startup_options_control_sheets_render_classic_facets_and_resource_text`.
  The key buttons now blit the real facets: `OptionsDlgAssets` carries optional
  `control` (`Control.png`, source of `fctKeyboard` 0,0,80,36; `fctCommand`
  0,36,32,32; `fctKey` 0,100,64,64) and `gamepad` (`Gamepad.png`, its own image
  with an 80px phase width) per `C4GraphicsResource.cpp:200-203,229`, and
  `draw_control_sheet` blits `fctKey` then the inset `fctCommand`. Both assets
  are optional, so a headless run or missing data keeps the text buttons rather
  than failing the dialog. The device set selectors blit `fctKeyboard`/`fctGamepad`
  phases too — `fctCtrlPic = fGamepad ? fctGamepad : fctKeyboard` with the set
  index as phase (`C4StartupOptionsDlg.cpp:271`) — and the `Keyboard` and
  `Gamepad` `MENU_PARITY.md` rows record the change.

- Closed 2026-07-29: **Overflow menu scrollbars.**
  `C4GUI::ScrollBar` had two independent implementations — the drawing half in
  `clonk-app-menus::game_over` (`draw_classic_scrollbar`, pinned by the
  evaluation-dialog tests) and the interaction half in `clonk-frontend`'s
  startup chat transcript. They agreed on the arithmetic, so
  `clonk-app-menus::scrollbar` is the promotion of both: the arrow/pin/track hit
  regions, the proportional pin placement, and the drag-to-scroll inverse
  (`C4GuiContainers.cpp:309-470,477-623`). `game_over` now calls it rather than
  keeping its own copy, and all 123 menu tests still pass. Pinned by
  `overflow_menu_scrollbar_arrows_track_and_thumb_match_cpp`, which covers the
  arrow/shaft boundaries, out-of-range clamping, the pin/scroll round trip, and
  a bar too short for two full arrows. Both menu draw paths now render it:
  `IngameMenuGraphics` carries the `scroll` facet and the in-game and
  engine-script draws call the shared `draw_classic_scrollbar` after their client
  contents, leaving it undrawn when the facet is absent as a null `C4Facet` does.
  The `MENU_PARITY.md` row is updated. Arrow auto-repeat and track clicks are wired: a held arrow steps one unit per
  drawn frame exactly as `ScrollBar::DrawElement` does, and a track click
  **jumps** the thumb to the pointer rather than paging — C++ has no paging
  behaviour here at all (`C4GuiContainers.cpp:414-423`), which corrects an
  earlier note on this work. A held arrow draws from its pressed
  facet cell and the `ArrowHit`/`Command` sounds are raised on C++'s exact
  transitions, so the `Mouse hit testing/scrollbars/tooltips` parity row is now
  **Complete**. This replaces the earlier note about
  ticket's first criterion asks for — the **engine object menu is now wired**:
  its layout exposes the reserved column as a `scrollbar` rect (present only
  while the menu overflows, the same condition that reserves its width), and
  `engine_menu_scrollbar_hit`/`engine_menu_scroll_from_pointer` route pointer
  input through the shared model. Pinned by
  `engine_menu_overflow_scrollbar_routes_arrows_track_and_thumb`. Note that
  menu's bar is only two 16px rows tall, so the two arrows consume it entirely
  and the pin has no travel — `rect.h - 3 * extent` is negative — which is what
  C++ does for a bar that short; the shared model's own test covers a bar with
  travel. `draw_classic_scrollbar` moved into the shared module too, so the
  drawing, hit-testing and pin arithmetic now have exactly one implementation
  between them. **The in-game menu is wired too, and it was a parity fix, not just
  plumbing:** `C4Menu::InitSize` widens an overflowing menu by
  `C4GUI_ScrollBarWdt` (`C4Menu.cpp:776`), and the port's in-game menu did not —
  it was 16px too narrow whenever it overflowed, with no test covering that
  case. It now widens, exposes the bar rect, and routes pointer input through
  the shared model via `scrollbar_hit`/`scrollbar_scroll_from_pointer`, pinned by
  `ingame_menu_overflow_widens_for_the_scroll_bar`. Note `C4Menu.cpp:765-771`
  gives Dialog-style menus vertical auto-enlargement and explicitly **no** bar;
  the ticket's first criterion lists Dialog among the barred styles, which is
  wrong against the oracle. This chassis has only Normal and Context, so the
  distinction does not bite yet. One hazard recorded on the ticket still
  stands: `object_menu.rs` already reserves a scrollbar column, so check the
  reserved extent matches `SCROLLBAR_EXTENT` before drawing into it. A second
  one — "the in-game menu render is cached and version-gated, so a newly
  interactive element must bump `menu_render_version`" — was never true of the
  in-game menu and is now false everywhere; see the frame-cache removal below.

- Closed 2026-07-29: **`c4group` command-line utility.** The C++ product builds
  and installs a standalone `c4group` (`CMakeLists.txt:431-437,749-750`); the
  port had no binary. `crates/clonk-c4group` provides one, and it is installed
  by `xtask` alongside the runtime. The argument parser is complete — the whole
  matrix from `c4group_ng.cpp:146-400`, the leading options (:545-576, including
  `-x:<command>` reading from `argv+3` and `fQuiet` defaulting to true),
  multi-group dispatch, and the rule that a `-` argument ends the previous
  command's argument list. Implemented: the default listing (:120-124),
  `-l`/`-v` with wildcard filtering (:270-284), `-k` (:346-348), `-e`/`-et`,
  `-a`/`-as`, `-m` (which deletes its sources only after the rewrite succeeds,
  :181-200), `-d`, `-r`, `-o`, and in-place `-p`/`-u`, which replace the path
  with the other representation as C++ does (:289-326).
  **Mutation strategy:** this port's writer builds groups rather than opening
  them for mutation, so a mutating command rebuilds the group into a
  `MutableGroup` and repacks. The rebuild preserves each entry's timestamp and
  executable bit and re-adds nested groups *already packed* with their stored
  CRC, so children are never unpacked and repacked;
  `untouched_rebuild_round_trips_byte_for_byte` pins that a no-op rebuild
  reproduces the file byte for byte, without which every mutating command would
  silently rewrite unrelated entries. Pinned by six parser tests, three rebuild
  tests, and the end-to-end `c4group_cli_round_trips_native_command_matrix`.
  Also implemented: `-s` (the sort list, ranked by first matching `|` segment
  then case-insensitively by name, `C4Group.cpp:2290-2340`), `-x` (explode,
  which unpacks and then explodes each child group), `-z`, `-w`, the
  `-p`/`-x:` end-of-run prompt and detached execute (`c4group_ng.cpp:680-704`),
  and `-i`/`-u` shell registration and unregistration on Windows, which reuse
  the registry table from `clonk-platform::file_classes`. Unregistration deletes
  deepest key first, because `RegDeleteKey` refuses a key that still has
  subkeys, and treats an absent key as success so it is idempotent over a
  partly-registered machine.
  **Still unimplemented, reporting themselves and exiting non-zero rather than
  silently succeeding:** `-g` and `-y`, update generation and application. They
  depend on the `C4UpdatePackage` format (`C4Update.cpp`, 909 lines), which
  **nothing in this port implements**. It needs no binary diffing — updates are
  whole-file replacement — but `C4GroupEx` reaches into `C4Group`'s private
  header and entry cores (`C4Update.cpp:149-200`), and `clonk-resources`
  exposes no equivalent. That is a separate subsystem port, not a CLI concern.

- Closed 2026-07-29: **Live `UserPath` re-expansion.** `AppPaths` resolved
  `General.UserPath` once at discovery and cached everything derived from it,
  while `C4Config::AtUserPath` re-reads and re-expands on **every** call
  (`C4Config.cpp:1351-1357`), so a `UserPath` or environment change made while
  the game runs moves later lookups in C++ but not here.
  `AppPaths::at_user_path` now mirrors that. **Blast radius is two C++ files,
  not the whole path system** — an earlier note on this ticket implied
  otherwise. `AtUserPath` is called only from the startup directory creation
  (:1337-1338, once, so re-expansion is unobservable) and from
  `C4FileSelDlg`'s default-portrait extraction (`C4FileSelDlg.cpp:614-622`).
  The port's counterpart, `extract_default_startup_portraits_once`, was the one
  real consumer and now goes through `at_user_path`. `user_data_dir()` stays
  cached deliberately: the session log and cache must not move mid-session.
  Pinned by `at_user_path_reexpands_live_user_path_and_environment`, which
  changes both the config text and `$HOME` after discovery.

- Closed 2026-07-29: **Developer ordered edit selection.** The engine retained
  only a single hovered `edit_cursor_target`, so nothing owned the ordered
  selection the edit cursor, property panel and object tree share.
  `clonk-engine::developer_selection` now does. Entries append at the tail the
  way `C4ObjectList::stNone` does (`C4ObjectList.cpp:110-135`) and never
  duplicate; a plain click replaces (`C4EditCursor.cpp:219`), ctrl-click
  removes-or-appends (:213-214) so a re-added object moves to the tail, and a
  frame drag takes the enumerated order with duplicates collapsed. Pruning drops
  removed or unknown objects without reordering survivors, in one revision
  rather than one per removal. The hovered object stays a separate scalar, as
  C++ keeps `Target` outside the list (`C4EditCursor.h:39`) — setting it never
  advances the revision, though a vanished hover is pruned. Every mutator
  returns a snapshot only when something actually changed, so a no-op notifies
  nobody, and each snapshot carries its `SelectionWriter` so a subscriber can
  suppress its own echo (`C4ObjectListDlg.cpp:599-646`). Pinned by
  `developer_selection_preserves_toggle_frame_and_tree_order` and
  `developer_selection_prunes_removed_objects_and_notifies_once`.
  The console's script input and its refresh cadence now sit here too.
  `EMMO_Script` builds **one** `C4ControlScript` at `SCOPE_Global` and executes
  it **once per selected object**, re-pointing only the target
  (`C4Control.cpp:932-944`); an empty selection runs nothing, because C++
  returns on `!pObjects` rather than falling back to a single global run.
  The refresh is deferred and coalesced: `OnSelectionChanged` merely raises
  `fSelectionChanged`, and `Execute` consumes it once per frame before updating
  the property dialog and object list (`C4EditCursor.cpp:80-86,196-199`), so a
  multi-object edit refreshes the panel once, not once per object.
  **There is no periodic refresh to pair with it.** `PropertyDlg::Update` has
  exactly five callers in the pinned source and every one is selection-driven;
  `Tick35` never appears near the console (only `C4Viewport` and `C4Object`
  use it). The property-dialog scope asks for a "Windows-only Tick35 periodic
  refresh" that does not exist — adding one would be an invention. Pinned by
  `script_input_fans_out_over_the_selection_and_refresh_is_coalesced`.
  Hit testing, gestures and dialog content stay out by design; the parity row
  flips when the dependent edit/object UI lands.

- Closed 2026-07-29: **Developer landscape-tool read model.**
  `clonk-engine::developer_landscape` exposes the material/texture catalog and
  tool picker the native `C4ToolsDlg` reads, without the console reconstructing
  engine internals. The material list is `Sky` then the material map in its own
  order (`C4ToolsDlg.cpp:486-489`); the texture list puts invalid
  material/texture pairs at the bottom and valid ones above, and Exact mode
  contributes no invalid section because every texture is selectable there
  (:517-548). `corrected_tool_texture` mirrors `AssertValidTexture` — Static
  mode only, sky exempt, first valid texture wins (:965-983).
  `Engine::developer_landscape_tool_state` supplies mode, `MapZoom` and map
  availability, and `Engine::developer_tool_pick` mirrors
  `C4EditCursor::ApplyToolPicker` (`C4EditCursor.cpp:698-731`): Static divides
  by `MapZoom` and decodes the retained map byte into tex-map index (`& 0x7F`)
  and IFT bit (`& 0x80`), Exact reads the live material and IFT, and an empty
  byte, unresolved slot, off-map coordinate or invalid material all resolve to
  sky. Pinned by `developer_landscape_tool_catalog_partitions_valid_pairs` and
  `developer_landscape_picker_reads_static_mapzoom_and_exact_ift`. Dialog state,
  rendering, shortcuts and `EMDrawTool` emission remain out of scope by design
  (they belong to the Tools/draw dialog); the parity row flips when that lands.

- Closed 2026-07-29: **Windows file associations and the `clonk:` protocol.**
  The runtime accepted classic launch arguments but nothing registered the
  shell entries that deliver them. `clonk-platform::file_classes` now carries
  `SetC4FileClasses`' full table (`C4FileClasses.cpp:46-71`): all eleven classes
  with their names, icon ordinals — including the deliberately skipped 5 and 12 —
  extension mappings and content types (three of which are not the group type),
  the `clonk:` URL protocol, the default-made `Update` verb for `.c4u`, and the
  `AppUserModelId` display name. Key and value shapes follow
  `StdRegistry.cpp:224-279`; everything is written as `REG_SZ` under
  `HKEY_CLASSES_ROOT`. `main` registers it after the window exists, graphical
  Windows builds only, best-effort with the result logged at debug — C++ ignores
  the result outright because it "will only work if we have administrator
  rights" (`C4Application.cpp:219-223`). The composition is host-independent and
  pinned everywhere by `windows_file_classes_match_the_native_registry_entries`;
  the registry write is Windows-gated and was verified to compile and clippy
  clean against `x86_64-pc-windows-msvc`. **Not carried over:** the deletion of
  the stale `HKLM\...\App Paths\Clonk.exe` key (`C4FileClasses.cpp:68`) — the
  constant is recorded as `STALE_APP_PATHS_KEY` but nothing deletes it, since
  this port never created that key and removing an `HKEY_LOCAL_MACHINE` entry
  belonging to a different product is not something to do unasked.

- Closed 2026-07-29: **Developer console Help > About.** The menu item existed
  but only appended a log line. It now opens `ConsoleAboutModal`, owned and laid
  out by the console itself the way `C4Console::HelpAbout` opens its dialog
  directly (`C4Console.cpp:1193-1200`), carrying the caption, the running
  version, and `"Copyright (c) 2008 RedWolf Design GmbH"` from the
  `C4COPYRIGHT_YEAR`/`C4COPYRIGHT_COMPANY` defines (:1190-1191). The version
  comes from `clonk-core`, the single source — specifically `PORT_VERSION`,
  since that names the running build, whereas C++'s `C4VERSION` names the C++
  engine. The dialog is task-modal like C++'s `MB_TASKMODAL`: it swallows other
  keys until Enter or Escape acknowledges it (`MB_OK`), then returns focus to
  the command line without touching the running game or the log. Pinned by
  `console_about_action_opens_versioned_modal`. `DeveloperConsoleAction::ShowAbout`
  and its host arm were removed — the console no longer needs a host round trip
  for this, and a never-emitted variant would be dead API.

- Closed 2026-07-29: **Native startup-failure dialog.** Failures before the
  window existed were only returned and logged, so a packaged graphical launch
  could vanish with no visible explanation. `main` is now a thin wrapper over
  `run()`: an error returned before `note_window_created()` is additionally
  reported through `clonk-platform::startup_dialog`, under the `STD_PRODUCT`
  caption this port already uses as its window title, with `MB_ICONERROR |
  MB_OK` on Windows (`C4WinMain.cpp:97-117`). The original diagnostic still
  reaches stderr and Clonk.log and the exit status still fails — the dialog is
  an addition, exactly as C++ both prints and shows the message
  (`C4WinMain.cpp:274-289`). Pinned by
  `startup_failure_uses_native_error_dialog_before_window_exists`. **Platform
  coverage:** only Windows has a real sink, mirroring C++ where the Unix dialog
  exists solely under `WITH_DEVELOPER_MODE`; macOS and Linux select
  `NoStartupDialog` and stay stderr-only, which is deterministic and cannot
  block. `report_startup_failure` also takes a `headless` gate. It was passed a
  constant `false` until 2026-08-06, when `--headless` gave the port a signal
  resolved early enough to latch: `Cli::parse` now calls
  `startup_dialog::note_headless_run`, and `main` reads it back, so a dedicated
  server never waits on a modal acknowledgement no operator is there to give.

- Closed 2026-07-29: **Recording and screenshot output-folder semantics.**
  `clonk-app::output_folders` now carries C++'s rules. The record root gains the
  language-prefixed `Title.txt` that `C4ConfigGeneral::CreateSaveFolder` writes
  (`C4Config.cpp:1397-1412`), taken from `IDS_GAME_RECORDSTITLE` and the existing
  two-character `classic_save_folder_language`; an existing title is never
  overwritten, matching C++'s `FileExists` guard. Screenshot handling drops the
  non-native `trim()` on the configured `ScreenshotFolder`, so the value composes
  verbatim as `C4Config.cpp:1326-1332` leaves it, and
  `prepare_numbered_screenshot_path` now attempts a single directory creation
  with the ExePath fallback instead of `create_dir_all`, matching
  `C4Config::AtScreenshotPath` (:1381-1390) — a failed creation no longer builds
  intermediate directories. Pinned by
  `recording_root_writes_localized_title_component` and
  `screenshot_folder_matches_native_raw_single_mkdir_fallback`. **Note:** the
  record root still uses the port's `create_dir` on a path already rooted at the
  install root, so its single-level creation matches C++; `Network.WorkPath` is
  untouched, as the card requires.

- Closed 2026-07-29: **Developer console window position.** Console mode forced
  `position = None` at startup and the exit path explicitly declined to persist
  anything, so the console reopened at the OS default every run.
  `clonk-app::console_window_position` now carries the stored grammar from
  `StdRegistry.cpp:283-327` — the literals `Maximized`/`Minimized`, or `x,y`
  (`x,y,w,h` only when `storeSize` is set, which the console never does, though a
  four-field entry is still honoured for its position). Startup restores from the
  `Console`/`Main` slot `C4Console::GetPositionData` names
  (`C4Console.cpp:1278-1284`), and shutdown writes the position alone, beside but
  separate from the game window's `persist_if_dirty` so the two never share keys.
  The 320x320 console default size stands; a `Maximized`/`Minimized` entry is
  logged and falls back to platform placement, since the port has no console
  equivalent of `ShowWindow(SW_MAXIMIZE)`. Unparseable entries restore nothing
  rather than moving to a garbage coordinate. Pinned by
  `console_window_position_round_trips_without_overwriting_game_display`.

- Closed 2026-07-29: **Window application icon.** Both shells built iconless
  windows. `startup_window_builder` now attaches a decoded product icon, which
  covers the game window and the developer console alike — the port routes both
  through one builder, matching C++ assigning one resource to both window
  classes (`C4FullScreen.cpp:196-211`; `C4Console.cpp:297-310`). **Deliberate
  source divergence:** C++ uses `src/res/lc.ico`, which carries LegacyClonk's
  branding; this port ships as a separate product whose release bundle icon is
  already derived from `planet/Graphics.c4g/Logo.png` (`xtask/src/main.rs:31-32`),
  so the window icon comes from that same file and the bundle and window chrome
  keep one identity. The logo is embedded rather than read from the data root so
  the window still has an icon when content is missing. A decode failure leaves
  the platform default, as C++ does with a null `HICON`. Pinned by
  `classic_window_icon_decodes_and_is_attached_to_both_shells`.

- Closed 2026-07-30: **The product had no icon anywhere it is actually drawn.**
  Three independent defects behind one symptom, plus the artwork that made the
  fourth invisible. `clonk-icon` now owns one derivation for every consumer.

  - **`cargo run` on macOS had no Dock icon at all.** winit 0.28.7 accepts a
    window icon on macOS and discards it: its platform impl is an empty body
    (`winit-0.28.7/src/platform_impl/macos/window.rs:1152-1162`) and the macOS
    builder never reads the attribute, so `window_icon.rs` was dead code on
    darwin and only a packaged `.app` ever had a tile. `dock_icon.rs` calls
    `-[NSApplication setApplicationIconImage:]` instead, after the event loop is
    built — winit installs its own `NSApplication` subclass by being the first
    sender of `sharedApplication`. No C++ counterpart: `WM_SETICON` is
    Windows-only and the SDL build relies on the bundle.
  - **No Windows executable carried an icon resource.** There was no `.rc`, no
    `.ico` and no resource-compiler step anywhere, so Explorer, the Start Menu
    shortcut and Add/Remove Programs all fell back to the toolchain stub — while
    `clonk-platform`'s `file_classes` had been registering `DefaultIcon` values
    pointing at exe icon ordinals 1..13 that did not exist. `clonk-icon::build`
    now emits the port's `src/res/engine.rc`: 14 `ICON` entries at ids 4000..4013
    in engine.rc order, embedded in `clonk-app` only, as CMake appends that
    script to the `clonk` target alone (`cmake/filelists/EngineWin32.cmake`).
    Verified by cross-compiling: 14 `RT_GROUP_ICON` resources over 84 `RT_ICON`
    images, ids ascending, so every ordinal resolves. The thirteen file-class
    icons are the engine's own, recovered from `src/res` at the pinned snapshot
    (`crates/clonk-icon/res/windows`, licence in `res/COPYING`) — there is no
    Clonk Rust artwork for a scenario or a definition. Slot 0 is the one
    divergence and is generated from the logo. `clonk-game` carries the
    application icon alone: C++ has no launcher, but it is the binary
    `scripts/windows-installer.nsi:72,79` points the shortcut and `DisplayIcon`
    at. The installer and uninstaller take it through an optional `-DICON`,
    which `package` writes beside the staged payload.
  - **The `.app` icon existed and was unusable.** The `.icns` was valid, sealed
    and correctly named all along; the source was the 972x440 two-line wordmark,
    and squaring it left 18% of a 16px tile inked — a beige smear that reads as
    no icon. The icon is now cut from the wordmark's leading stone "C"
    (`APP_MARK`, 170x176, near-square), which lifts that to 65%. Consequence
    worth stating: the mark is 176px, so every slot above it is an upscale;
    Lanczos holds up to 512 and the 1024 `.icns` slot is soft. Pinned by
    `the_app_icon_fills_its_smallest_tile` and, so an artwork edit fails loudly
    rather than shipping a clipped glyph, `the_crop_rectangle_is_tight_around_the_mark`.
  - **Both resize paths filtered straight alpha**, so the transparent margin's
    `RGB(0,0,0)` averaged into every antialiased edge — a visible edge pixel
    measured 17/255 before the fix. Now premultiplied. Pinned by
    `downscaling_does_not_bleed_the_transparent_margin_into_the_edge`.
  - Two smaller gaps closed with them: winit's `with_window_icon` fills
    `ICON_SMALL` only, so the Windows taskbar button stretched the 64px
    title-bar image — `with_taskbar_icon` now supplies 256; and
    `clonk-launcher-shell` built its window with no icon on any platform.
  - **The updater threw the icon away.** `clonk-update` swaps only
    `Contents/MacOS`, so the `.icns` the engine component ships was extracted
    and discarded and an install updated in place kept its original icon for
    ever. `install_bundle_icon` renames it in before `resign_bundle`, so the new
    seal covers it, with the old one moved outside the `.app` for rollback.

  Gap: `crates/clonk-network/src/client_mesh.rs:1` has a pre-existing
  Windows-only `unused_imports` warning, unrelated to the icons but the only
  thing standing between a Windows `cargo check` and clean output.

- **Windows taskbar loading progress landed, with a real COM backend.**
  `clonk-platform::taskbar_progress` now carries C++'s logic: `LoaderTaskbarProgress`
  applies `C4Game::SetInitProgress`'s strictly-increasing gate
  (`C4Game.cpp:4094-4106`) and `CStdWindow::SetProgress`'s branch — 100 clears the
  indicator, anything else sets `TBPF_INDETERMINATE` plus the value
  (`StdWindow.cpp:183-196`) — and entering startup clears it and re-arms the gate
  (`C4Application.cpp:422-426`). The app drives it from
  `apply_scenario_loader_frame` and clears it when the loader consumes
  `Finished`. Pinned by `windows_loader_progress_updates_and_clears_taskbar_state`
  against an injected recording sink.
  `Win32TaskbarProgress` is the real sink: `CoCreateInstance(TaskbarList)`,
  `HrInit()`, then `SetProgressState`/`SetProgressValue` exactly as
  `CStdWindow` calls them, with every call best-effort because C++ simply skips
  them when its interface pointer is null. A failed create or `HrInit` yields
  `None` so the caller falls back to `NoTaskbarProgress` — a machine with no
  shell taskbar shows no progress rather than failing to start.
  The vtable is **generated**, not hand-written. An earlier note here proposed
  counting slot indices by hand because `windows-sys` types `ITaskbarList3` as
  an opaque `*mut c_void`; that was unnecessary — `clonk-app` already depends on
  the `windows` crate at 0.54 with `Win32_UI_Shell`, which generates safe
  bindings, so `clonk-platform` now does too. Hand-counted indices would have
  been unverifiable without a Windows machine and an off-by-one calls the wrong
  method with the wrong signature; generated bindings remove the risk entirely.
  Verified with `cargo check -p clonk-platform --target x86_64-pc-windows-msvc
  --all-targets`.
  The app now installs it. `GameApp::taskbar_progress` holds a boxed sink so the
  backend can be chosen once the platform window exists — which is also when
  C++ first has an `ITaskbarList3`, since `CStdWindow` needs a handle — and
  `run()` swaps it in right after the window is created. The handle is extracted
  **unconditionally**, because `RawWindowHandle::Win32` exists on every
  platform; only the two-line sink construction is target-gated. That is
  deliberate: `clonk-app` cannot be cross-checked for Windows (stacker's C build
  needs an MSVC toolchain), so the untestable surface is kept to those two
  lines and everything around them is compiled and linted on every host.
  **Still open:** observing it on a real taskbar.

- Closed 2026-08-04: **ClonkMars drew the base game's logo on its upper board.**
  `C4UpperBoard::Execute` centers the `Logo` facet on the board strip
  (`C4UpperBoard.cpp:88-92`), and `C4GraphicsResource` resolves that facet over
  the registered `Graphics.c4g` groups, so a scenario folder's own copy outranks
  `planet/` (`C4GraphicsResource.cpp:418-470`). Two bundled packs ship one:
  Hazard's is its own total-conversion wordmark and stays, but ClonkMars carried
  a 220x85 copy of the *base game's* logo, which this port rebranded — so every
  Mars scenario ran under the old product name while every other scenario showed
  the new one. Fixed in the data, not the engine: the override mechanism is
  correct and load-bearing for Hazard, so the redundant file is simply gone from
  the content submodule and Mars falls through to `planet/Graphics.c4g/Logo.png`.
  Height-neutral — both logos land on the same 67px `UPPER_BOARD_LOGO_MAX_HEIGHT`
  clamp. Pinned by `real_mars_upper_board_keeps_the_product_logo`, which resolves
  the real catalog entry through the real group set.

- Closed 2026-07-29: **Loading-screen GUI log capture.** The loader's log box
  showed only its own hard-coded phase labels: `ScenarioLoadingReporter` kept a
  private `VecDeque` that `report` replaced wholesale, so engine diagnostics
  emitted while a scenario loaded reached Clonk.log and stderr but never the
  only screen the player can see. `clonk-logging` now owns a bounded ordered
  loader buffer that both the GUI log sink and the reporter's milestones append
  to under one mutex, so a worker-thread event landing between two milestones
  keeps its position instead of either source replacing the other
  (`src/C4Log.cpp:208-243`). The sink is permanently attached and gated by an
  active flag rather than rebuilt per round; it reuses `GuiSinkFormat`, so lines
  carry the C++ GUI severity prefix rather than raw tracing metadata. The buffer
  opens with the reporter (`src/C4MessageBoard.cpp:223-251`) and is released
  when the loader consumes `Finished`. Pinned by
  `loader_captures_runtime_log_lines_in_gui_order`, which interleaves a
  worker-thread event between two phase updates and covers the capacity and
  before/after-loader boundaries.

- Closed 2026-07-29: **`DebugLog` routing.** `DebugLog` emitted an ordinary
  `tracing::debug!` on the same target as `Log`, so the one process-start
  `EnvFilter` discarded it everywhere at default verbosity — script diagnostics
  never reached Clonk.log — while a verbose session pushed debug-only lines to
  the message board in rounds where C++ suppresses them. `DebugLog` now has its
  own target (`clonk-core::log_target::SCRIPT_DEBUG_LOG_TARGET`), the registry
  filter admits it unconditionally so the session log always keeps it, stderr
  re-applies the operator's verbosity, and the GUI sinks take it only while the
  round has debug mode on. The engine publishes that gate from every site that
  mutates `debug_mode` — `set_debug_mode` (round setup and Ctrl+F5),
  `disable_debug` (synchronized DisableDebug) and the state reset (clear) — so
  all four transitions are covered without rebuilding any sink. Pinned by
  `debug_log_file_and_gui_routes_follow_runtime_debug_mode`. Per the standing
  directive that `clonk-logging` is judged on best practice rather than
  `C4Log.cpp` parity, this reproduces the *behaviour* with tracing's own layer
  and writer filters instead of mirroring C++'s spdlog sink tree.
  `debug_log_message_emits_debug_event_with_script_target` pinned the old shared
  target and was renamed and updated as part of this behaviour change.

- Closed 2026-07-29: **`Graphics.VerboseObjectLoading` diagnostics.** The level
  had an editor row but no runtime consumer. `clonk-engine`'s
  `scenario::verbose_loading` now carries it as a process global — the same
  shape as C++'s `Config` global, which the definition loader reads far from
  where the app parses configuration — published by `main` from the config with
  C++'s default of 0 (`C4Config.cpp:453`). Level 3 logs each definition's group
  full name (`C4Def.cpp:555-556`), level 1 logs `IDS_PRC_DEFOVERLOAD` for
  definition and particle overloads (`C4Def.cpp:1051`; `C4Particles.cpp:182`),
  and level 2 adds the `Old def at`/`Overload by` lines (:1055-1058); the levels
  are floors, so level 3 emits all three. Pinned by
  `verbose_object_loading_levels_gate_definition_diagnostics`. The overload
  bookkeeping is skipped entirely below level 1 so the default path adds no
  allocation to scenario load. **Not covered:** the overload template is the
  shipped US `IDS_PRC_DEFOVERLOAD` text — the engine has no resource-string
  table of its own, and unlike `NeededMaterialStrings`/`ConstructionCheckStrings`
  the app does not yet overwrite this one, so a German session logs the US
  wording. The seam to do so exists.

- Closed 2026-07-29: **Windows `/allocconsole` bootstrap.** `C4WinMain.cpp:72-93`
  allocates a console before normal initialization — unconditionally for debug
  GUI builds, only for `/allocconsole` in release GUI builds — aborts startup
  with `C4XRV_Failure` when allocation fails, and reopens stdin on `CONIN$` and
  stdout/stderr on `CONOUT$`. `clonk-platform::alloc_console` now carries that
  policy and `main` applies it before any output, ahead of the existing
  `attach_parent_console` (which solves the different problem of a terminal
  launch under `windows_subsystem = "windows"`). **Deliberate mechanism
  divergence:** C++ reattaches the CRT `FILE` streams with `freopen`, but Rust's
  `std::io` reads the process standard handles rather than the CRT, so the port
  opens the console devices and publishes them with `SetStdHandle` to reach the
  same observable state. A process that already owns a console makes
  `AllocConsole` fail with `ERROR_ACCESS_DENIED`; that arm is treated as success
  and only the streams are reattached, matching where C++'s `freopen` calls
  leave such a process. Pinned by `console_policy_matches_the_cpp_build_gates`
  on every platform and by the Windows-gated
  `windows_release_allocconsole_attaches_standard_streams`, which asserts all
  three standard handles are console devices via `GetConsoleMode`.

- Closed 2026-07-29: **Unix effective-root startup refusal.** `C4WinMain.cpp:251-255`
  refuses `geteuid() == 0` before the debug facilities and application
  initialization. `clonk-platform::privileges` now supplies that guard and
  `main` consults it immediately after the macOS translocation chdir and before
  the crash handlers, printing `Do not run <argv[0]> as root!` to stdout — with
  C++'s `"this program"` fallback when `argv[0]` is absent — and exiting
  `C4XRV_Failure` (1). Non-root and non-Unix startup are untouched. Pinned by
  `unix_effective_root_is_rejected_before_bootstrap`, which drives the guard
  with an explicit effective UID: an unprivileged test run cannot spawn a
  privileged child, so the decision function is exercised directly and `main`
  feeds it the real `geteuid()`.

- Closed 2026-07-29: **Windows unhandled-exception diagnostics.**
  `clonk-platform::crash_win32` installs the one-shot
  `SetUnhandledExceptionFilter` before application initialization
  (`C4WinMain.cpp:68-70`; `C4CrashHandlerWin32.cpp:644`). A first crash composes
  the `SafeTextDump` report — banner, exception code and sentence,
  continuability, access-violation/page-error detail, the x86_64 register block
  and the EFLAGS letters (`C4CrashHandlerWin32.cpp:86-202`) — writes it to the
  session log descriptor, writes a `LegacyClonk-crash-<UTC>.dmp` minidump under
  `Config.General.UserPath` via `CreateFileA(CREATE_NEW)` + `MiniDumpWriteDump`
  (:390,410,417,455-464), shows the `MessageBoxA` naming both artifacts
  (:427-447), and returns `EXCEPTION_CONTINUE_SEARCH` so the OS keeps its own
  processing (:467-468). The user path and log descriptor are published as they
  become known, mirroring C++ reading them from inside the filter. Report
  composition is host-independent and pinned by unit tests on every platform;
  the artifact path is pinned on Windows by
  `windows_unhandled_exception_writes_log_minidump_and_dialog`, which drives the
  same code the filter does — the OS-invoked filter itself cannot run in-process
  without killing the harness. **Not covered:** the symbolised stack walk and
  loaded-module list (`C4CrashHandlerWin32.cpp:280-350`), which need DbgHelp
  `StackWalk64`/`SymFromAddr` and a `Module32First` snapshot; the report carries
  registers but no frames.

- Closed 2026-07-29: **Unix fatal-signal diagnostics.** `clonk-platform::crash`
  installs the classic handler set — SIGBUS, SIGILL, SIGSEGV, SIGABRT, SIGINT,
  SIGQUIT, SIGFPE, SIGTERM, in `C4WinMain.cpp:257-264` order — before
  application initialization. A handled signal writes `<product>: Caught signal
  <NAME>` and up to 100 `backtrace_symbols_fd` frames to stderr and to the
  session log's raw descriptor, then restores `SIG_DFL` and reraises so the
  process keeps its original signal exit status and core-dump behaviour
  (`C4WinMain.cpp:179-213`). The handler uses only async-signal-safe calls —
  `backtrace` only after `install` resolves it once, because glibc loads it out
  of libgcc on first use and that allocates. `clonk-logging` `dup(2)`s the log
  descriptor out from behind the buffered tracing writer so the banner never
  touches it. Because the log does not exist when the handlers are installed,
  an early crash is stderr-only, exactly as C++'s `GetLogFD` sentinel yields.
  Pinned by the subprocess test
  `unix_fatal_signal_writes_diagnostics_then_reraises`, which asserts the child
  dies *from* SIGABRT rather than exiting.

  Amended 2026-08-05: the handlers go in with `sigaction(2)` and `SA_ONSTACK`
  on a 128 KiB alternate stack, **not** C++'s `signal(2)`
  (`C4WinMain.cpp:257-264`). A handler with no alternate stack cannot be
  entered once the stack is exhausted, so `signal(2)` made a stack overflow
  kill the process having written nothing at all — and worse than inheriting
  C++'s behaviour, it *replaced* the `SA_ONSTACK` handler the Rust runtime
  installs before `main`, so the port also lost the `has overflowed its stack`
  line stock Rust prints. Reproduced against the shipped crate: with the
  handlers installed an overflow wrote zero bytes and died on a signal; without
  them the runtime reported it. The signal set, the banner and the reraise are
  unchanged. Pinned by `unix_stack_overflow_still_reaches_the_crash_banner`.
  Threads not created by Rust — an audio or GPU-driver callback thread — have
  no alternate stack from anyone, so their overflows stay mute; that limit is
  std's too. Signals outside the C++ set of 8 (`SIGHUP`, `SIGTRAP`, `SIGXCPU`,
  `SIGSYS`) still terminate with no banner, deliberately.

- Closed 2026-08-06: **Headless dedicated-server mode** (clonk-org/clonk-rs#120).
  The port could already host a player-less network round and drive it from
  stdin, but `/console` still built a winit window and a wgpu device before the
  stdin reader existed, so it could not run where there is no display server or
  usable adapter. `--headless` now boots `run_headless_server`
  (`main_parts/assets.rs`) instead: no window, no device, `AudioOptions::silenced`
  (C++ forces `ENABLE_SOUND` off for `USE_CONSOLE`, `CMakeLists.txt:183-185`),
  handing straight to the already-complete-but-uncalled `run_console_event_loop`.
  `poll_boot_loading` no longer holds a server in `Loading` waiting on a loader
  screen it never builds — `C4Application::PreInit` builds one only for a
  startup-dialog run (`C4Application.cpp:239`) and a `USE_CONSOLE` build has no
  `C4FontLoader` at all (`C4Game.h:132-135`).

  **The signal is a `Cli` switch, deliberately not `Graphics.Engine = NoGfx`.**
  C++ has two mechanisms and they are not interchangeable: `USE_CONSOLE` is a
  build (`CMakeLists.txt:178`) and `GFXENGN_NOGFX` is only the runtime renderer
  `DDrawInit` picks (`StdDDraw2.cpp:1301-1310`). The stdin console is compiled
  in *solely* for `USE_CONSOLE` (`StdAppUnix.cpp:413-449`;
  `StdAppWin32.cpp:77-79`), so a graphical build carrying `Engine=3` is a blind
  interactive app with no way to command it — not a server. A build flag is also
  fixed for the life of the process, which a `Cli` field is and a classic
  argument is not: `/open <params>` re-parses a classic command line into the
  running process (`game_app/console_record.rs`). `headless` is likewise a
  separate `GameApp` field from `console_mode`, which carries developer-console
  authority — it is the `Console.Active` argument behind `ScriptControlPolicy`,
  i.e. "execute remote console-scope script from any client" — that a server on
  the internet must not inherit. C++ reads the two separately as well, and
  either alone makes the lobby a console lobby (`C4Network2.cpp:463`).

  Unattended round handling came with it. At countdown zero a host with no lobby
  dialog that is short of `C4Scenario::GetMinPlayer` logs
  `IDS_MSG_NOTENOUGHPLAYERSFORTHISRO` and quits rather than starting
  (`C4GameLobby.cpp:1163-1168`); the minimum rides on `ClassicHostLobbyProjection`
  beside the maximum it already carried, and an undetermined minimum never quits.
  Round end takes `C4Game::ShowGameOverDlg`'s console arm — quit directly
  (`C4Game.cpp:3679-3690`) — instead of pausing behind an evaluation dialog that
  a headless process cannot draw and no input can dismiss. The countdown itself
  reaches the log when no dialog consumes it (`C4GameLobby.cpp:1118-1127`,
  `:1150-1157`, `:1183-1190`).

  **Still open.** (a) `C4Game::ShowGameOverDlg`'s console arm first drains a
  pending network stream before quitting (`C4Game.cpp:3680-3687`); the port
  quits without that wait, so a stream can lose its tail. (b) No auto-rehost or
  scenario rotation — C++ has none either, parking in `C4AS_Startup` for the
  operator's next `/open` (`C4Application.cpp:428`), and the port's process
  simply exits, so a persistent server needs a supervisor restart.
  `clonk-network::host_restart`'s `PID_PORT_HOST_RESTARTING` is unused by this
  path. (c) The masterserver/league host signup is not exercised headlessly by
  any test. (d) The remaining `!console_mode` gates that a headless run also
  arguably wants — `failed_open_game_returns_to_startup` (`game_app/startup.rs`),
  `failed_record_stream_exits` (`game_app/console_record.rs`) and the savegame
  thumbnail capture (`game_app/saves.rs`) — are unchanged; each is reachable only
  from a render or interactive path a headless process does not take, and they
  were left rather than changed blind. (e) The graphics stack is still linked
  into the server binary; only a Cargo feature or a separate `clonk-server`
  binary would drop winit/wgpu/audio from its dependency graph.

- Closed 2026-08-05: **Session-shutdown diagnostics** (clonk-org/clonk-rs#40).
  Nothing marked a shutdown, so a session log that stopped mid-stream read
  identically whether the player quit on purpose or the process was destroyed —
  and every "the game just vanished" report stalled on exactly that fork.
  `clonk-logging::log_shutdown_banner` now writes `stopping clonk` with a
  reason as the last line of the `Event::LoopExiting` pass, after the config
  and console persistence that can still `warn!`; `GameApp::request_exit` takes
  a `&'static str` naming which of its 12 exits ran, so a bare Escape on the
  main menu is distinguishable from a window close or an update hand-off.
  `log_fatal_error` routes an `Err` out of `run()` through the log before
  `main` returns it: the Rust runtime prints a returned error to stderr alone,
  which a windowed build has nowhere to show, and winit's Wayland loop reaches
  that path after a failed `Connection::flush` without logging anything itself
  (`winit-0.30.13/src/platform_impl/linux/wayland/event_loop/mod.rs:284-287`),
  so a lost compositor connection used to end the session with an empty-looking
  log. It also covers a startup-initializer failure, where the handler is never
  installed and the `LoopExiting` teardown — and so the banner — never runs.
  Pinned by `the_shutdown_banner_records_that_the_session_ended_on_purpose`,
  `a_fatal_error_reaches_the_session_log` and
  `quitting_from_the_main_menu_records_why_the_session_ended`. Still silent by
  construction: `SIGKILL` (OOM killer), and the `process::exit` in the
  `LC_APP_PRESENTATION_BENCHMARK` path and the root-privilege refusal, both of
  which run before or outside the session log.

- Closed 2026-07-29: **macOS app translocation.** A quarantined bundle runs from
  a read-only `AppTranslocation` mount whose siblings are absent, so resource
  discovery saw a copy with no `Contents/Resources`. `clonk-platform` now
  resolves `SecTranslocateIsTranslocatedURL` /
  `SecTranslocateCreateOriginalPathForURL` out of Security.framework at run time
  the way C++ does (`MacAppTranslocation.cpp:27-63`) — dynamically, so a system
  without those symbols simply reports "not translocated" — and `main` chdirs to
  the directory holding the `.app` before path discovery
  (`C4WinMain.cpp:233-238`). Non-translocated bundles and explicit
  `LC_INSTALL_ROOT`/`/config` overrides are unchanged; the recovery returns the
  original path only when the probe positively reports translocation. Unlike
  C++, an unusual path encoding recovers through `CFStringGetCString` instead of
  throwing on a null `CFStringGetCStringPtr`. Pinned by the platform-gated
  `macos_translocated_bundle_uses_original_root_and_cwd`.

- Open gap (found 2026-07-28, not closed): **point and line raster width does
  not track world zoom.** `DrawProjection::line_width` is
  `presentation.scale` alone (`draw_projection`,
  `crates/clonk-app-render/src/gpu_renderer.rs`), and `rounded_raster_width`
  turns that into the physical point/line footprint. Because a world vertex's
  *position* is projected through the same application scale, a PXS point is
  exactly one world pixel at every `Graphics.Scale` — which is why the
  "rain is a scatter of single dead pixels at 4K" reading is a symptom of the
  world being small on a large panel, not of the point size. It becomes a real
  defect the moment viewport zoom is unpinned from 1.0: the world would
  magnify while rain, spray, dug-material sparks and every debug line stayed
  at the unzoomed width. Any zoom work must add the zoom term here, and the
  frontend must then stop being the only place that knows the zoom.

- Open UI gap (found 2026-07-28, not closed): **the custom raster frontend
  exposes neither platform accessibility semantics nor IME composition.**
  Scenario-search counts and no-result guidance are visible pixels but are not
  announced through an accessibility status node, and the search field has no
  semantic role or programmatic name. Window input handles committed
  characters but no IME preedit/commit lifecycle or candidate positioning.
  Closing this requires a platform accessibility bridge plus explicit IME
  enablement, composition state, and rendering. Classic keyboard parity remains
  covered, but screen-reader and international text-entry completeness must not
  be claimed.
- Open content gap (found 2026-07-28, CI treatment added 2026-07-29):
  **material slots follow host `readdir` order, so raw material-index and
  landscape goldens are host-specific.** `C4Group`'s folder scan is unsorted —
  `DirectoryIterator`
  wraps `DIR *`/`dirent` on Unix and `_finddata_t` on Windows (StdFile.h:102-126)
  — and `C4MaterialMap::Load` takes material slots straight from that scan
  (C4Material.cpp:263-299). `directory_entries` (`clonk-resources/src/group.rs`)
  mirrors this faithfully via `WalkDir` with no sort, and
  `MaterialLibrary::from_group` (`material.rs:204-219`) consumes that order. With
  `content/` checked out **unpacked**, material indices therefore depend on the
  filesystem: the APFS recording host and ext4 CI enumerate `Material.c4g`
  differently. Frame 0 of `tutorial01-idle` diverges in 277 leaves, all under
  `landscape/` — `liquids/*/material` reads 1 on macOS and 10 on Linux, and the
  texmap material names rotate (`Ice`/`Ashes`/`Vehicle`/`Tunnel`). The mismatch
  only surfaced once the `cargo fmt` gate stopped short-circuiting the rest of
  the parity job.
  Sorting the folder listing was measured and **rejected**: it makes Rust
  disagree with the C++ oracle on the recording host, breaking
  `elevator_motion_oracle::tutorial07_seed_zero_landscape_matches_cpp_surface8`
  (a whole-plane Surface8 hash, i.e. per-pixel material indices),
  `real_tutorial_seven_acid_rain_matches_cpp_animated_pxs_sequence`, and three
  real-scenario routes. C++ is right, so the engine keeps the unsorted scan.
  Simply packing the current global group is not a compatible shortcut:
  `c4group -p` applies the stock `C4FLS_Material` archive sort and changes the
  committed replay checkpoints too. Reproducing the recording order in a
  packed temporary group works, but the content tree contains 29 unpacked
  `Material.c4g` directories that would each need an explicit pinned order.
  CI therefore separates portable determinism from recording-host oracle
  values: Ubuntu repeats real replays under its native order and synthetic
  order tests use explicitly ordered packed groups; the raw real-content
  oracles carry a named non-macOS ignore reason and run in the required
  `Recording-host material-order oracles (macOS)` job. A repository script test
  requires every such ignored oracle to appear in that job, so the platform
  gate cannot become a silent skip. The underlying content gap remains open;
  closing it means shipping pinned packed groups and re-recording Rust and C++
  oracles together under that archive order.
- Test portability note (2026-07-29): the full-frame value in
  `loader_screen::tests::real_graphics_and_endeavour_frame_hash_is_stable` is a
  **Rust raster regression, not a captured C++ framebuffer oracle**. Its
  `LoaderSky1.jpg` input already differs before rendering: `jpeg-decoder`
  0.3.2 selects SSSE3 IDCT/color conversion on capable x86 CPUs and its scalar
  path elsewhere. The test therefore pins both decoder-specific input and
  composed-frame hashes; treating the scalar value as a macOS oracle, or the
  SSSE3 value as a Linux oracle, would be false because the split follows the
  decoder backend rather than the OS. C++ is not a portable pixel authority
  here either: the pinned build selects WIC or system libjpeg
  (`CMakeLists.txt:202-203,353-359,529-533`), rasterizes through runtime
  FreeType, and filters through OpenGL. Keep this test in the portable suite,
  separate from the APFS material-order oracle job.
- Open gap (found 2026-07-27, not closed): **scenario load is ~half the
  process cost and is unoptimized.** `ClonkMars/03_Chaos` takes 13.8-15.7 s to
  load on the reference machine — roughly two minutes on a Pi 4 — and 99% of
  that is object-placement script callbacks
  (`Engine::run_legacy_init_placements` -> `init_create_object` ->
  `Vm::invoke_script_function`), i.e. the same VM the frame loop uses. Anything
  that makes script name resolution cheaper (interning definition/function
  names instead of hashing and comparing `String` keys) pays here twice.
- Open gap (found 2026-07-27, not closed): **Raspberry Pi 0-3 cannot start.**
  wgpu-hal's GLES backend requests GLES 3.0 or higher
  (wgpu-hal-29.0.4 src/gles/egl.rs:463-474), and VideoCore IV is GLES 2.0,
  so no adapter is produced on any backend — `build_framebuffer`
  (`crates/clonk-app/src/main_parts/audio.rs`) now widens to `Backends::all()`
  and reports this explicitly instead of failing opaquely, but it cannot
  create a device that does not exist. There is no CPU presentation fallback
  either: the CPU rasterizer branch in `crates/clonk-app/src/main.rs` is
  unreachable because the retained-GPU path matches all three `AppMode`
  variants, and even reaching it would not help, since `pixels` needs a wgpu
  device to blit a CPU buffer to the window. Closing this means replacing
  `pixels` with a CPU presenter (`softbuffer` or equivalent) and re-enabling
  that branch; Pi 4/5 are unaffected.
- Closed 2026-08-02: script `AddMessage` now emits
  `C4GameMessageList::Append(..., fNoDuplicates=false)` semantics
  (`compat/menus_messages.rs`, `message.rs`), matching
  (`C4Script.cpp:2435-2441`, `C4GameMessage.cpp:315-329`). Target messages
  retain `NO_OWNER` and global messages retain `ANY_OWNER` (-2), so an
  ownerless `CustomMessage` cannot become the wrong append candidate.
  C++-cited regressions cover repeated messages, empty fallback text, and the
  owner distinction (`compat/tests/part_03.rs`).
- Open gap (found 2026-07-25, not closed): a DFA_FLIGHT object can land one
  frame later than C++. Reproduce with EkeReloaded `TheStippelAge/Invasion`
  under `LC_PIN_SEED=777` and a `#appendto ST5B` per-frame `Log` of
  action/position/`GetXDir(0,1000)`: Stippel `o739` reaches
  `Jump pFLIGHT x2730 y710 U400 V200` identically in both engines, then C++
  runs `ContactAction`'s bottom-`DFA_FLIGHT` arm (`C4Object.cpp:4360-4377`,
  `last_xdir`/`ObjectActionWalk`/restore) that frame and reports
  `Walk pWALK x2731 y710 U-300`, while Rust stays airborne one more frame
  (`Jump pFLIGHT x2731 y710 U400 V400`) and reconverges by the third frame.
  Only the frame the contact fires differs, so the cause is sub-pixel `fix_y`
  drift entering the frame rather than the transition itself; script cannot
  observe `fix_y` (`FnGetX`/`FnGetY` return whole pixels, `C4Script.cpp:1249`),
  so isolating it needs an oracle-side dump. Scale: 3 of 37 314 trace lines on
  that seed; the same run is otherwise bit-exact, and `LC_PIN_SEED=12345` is
  bit-exact over all 37 161 lines. This is independent of the FindObject
  ordering fix landed the same day — every `Find`-driven event (all `Bite`s)
  matches on both seeds.
- Closed 2026-08-02: script `SetPosition` now runs the trailing
  `UpdateInLiquid` path after `ForcePosition` (`C4Script.cpp:479`),
  including the `Float * Con / FullCon - 1` probe and entry `Splash`
  (`C4Object.cpp:5632-5635,6093-6110`). The cached flag is staged
  synchronously so a same-callback `InLiquid()` observes it, while the
  random bubble/PXS operations remain ordered. C++-cited regressions cover the
  flag refresh and heavy fast-object entry splash (`compat/tests/part_08.rs`).
- Closed 2026-08-02: the Rust-only `Landscape::resolve_collision` column
  fallback and all of its engine call sites were removed. C++ resolves contact
  per vertex and per pixel in `C4Object::ContactCheck` /
  `C4Object::DoMovement` (`C4Movement.cpp:165-181`, `:231`), so a
  pixel-less fixture no longer teleports an object to `surface_height(x)` or
  zeroes its velocity. The C++-cited `ObjectUpdate` regression is in
  `lib_tests/command_contact_regression.rs`; real terrain movement remains
  responsible for pixel-grid contact.
- Closed 2026-08-04: the **startup-menu frame cache** (`MenuFrameCache`,
  `menu_render_version`, `mark_menu_dirty`, 209 production call sites and 19
  invalidations) was removed. It never engaged in the shipping presentation
  path: `render_retained_gpu_frame` enters either the ordered-native branch,
  which begins clonk-text capture, or `begin_gpu_scene_capture`, and
  `cache_eligible` requires *neither* flag. That disqualifier is load-bearing
  rather than an oversight — the retained path passes a 4-byte
  `ignored_cpu_pixel` as its `frame`, so a cache write there would have stored
  a 4-byte "frame" whose length then matched on the next pass and short-
  circuited all GPU scene recording. The CPU presenter does not reach it
  either: `ordered_native_text` is true in `AppMode::Menu` whenever the
  scale-matched native atlas exists. Measured on this tree (test profile,
  `render_retained_gpu_frame` p50 over 40 samples) the recompose the cache
  would have saved is 52 µs at 1280x720 and 62 µs at 3840x2160 for the main
  menu, 25 µs for the scenario browser, 29/40 µs for player selection, and
  269 µs for About (1118 commands) — against a CPU-path *cached replay* of
  46 µs at 720p and 435 µs at 4K. Under retained capture the composition is
  command recording, not the software blit the original 15-21 ms → 0.7-2.1 ms
  figures were measured against, so restoring the cache was worth at most
  1.6% of one frame while making every future interactive menu element
  depend on remembering to invalidate. Do not reintroduce it; if menu
  presentation cost ever matters again, the lead is the renderer's persistent
  `composition.texture`, which could re-run only the present pass.
- Intentional divergence from C++ (2026-07-24), not a gap to close: the port
  ships **no trademark notice**. C++ draws `FANPROJECTTEXT " " TRADEMARKTEXT`
  (`C4Version.h:21-22`) in the main-menu and About footers
  (`C4StartupMainDlg.cpp:72-74`, `C4StartupAboutDlg.cpp:274-275`); Rust now
  draws the `FANPROJECTTEXT` half alone. The About licenses list drops its
  `Clonk Trademark` page, leaving one entry (`COPYING`), which widens the
  already-recorded gap in `docs/MENU_PARITY.md` that Rust does not ingest the
  `deps/licenses.cmake` corpus — with one row, list navigation has no second
  row, so `license_list_pointer_and_keyboard_selection_follow_listbox_rules`
  no longer covers inter-row selection. `IDS_DEV_LICENSE` keeps its CC BY-NC
  content half and loses its trademark half; `IDS_DLG_LICENSE` is now
  `Clonk Game Content License`. Label anchors are unchanged — both labels are
  right-aligned, so their geometry is text-length independent and no startup
  pixel baseline needed re-recording. The one re-recorded golden is
  `loader_screen::tests::real_graphics_and_endeavour_frame_hash_is_stable`,
  whose fixture lost a log line; its renderer is untouched. Four About-dialog
  tests that pinned C4GUI ListBox/ScrollWindow rules across a two-row list were
  deleted outright, since a one-row list gives them no subject:
  `l056_content_becoming_non_scrollable_clears_held_arrow_silently`,
  `l056_tiny_bar_has_no_track_drag_but_arrows_use_synthetic_range`,
  `l059_noop_scroll_updates_preserve_thumb_pin_residuals`, and
  `license_list_wheel_scrolls_its_second_scrollwindow_at_tiny_heights`. That
  list-widget coverage is simply gone; restoring it needs a second list entry
  or an `AboutDlgState` that can be driven over a test-supplied license set.
  Do **not** "restore parity" here by re-adding the notice; this is a
  deliberate licensing decision by the maintainer, not a port defect.

- Test gap (2026-07-25): `advance_game_clock_from_elapsed` now coalesces a
  one-second-timer backlog instead of replaying one `sec1_timer` pass per
  elapsed second, matching C++ (`seconds != LastExecute.tv_sec` fires
  `Sec1Timer()` at most once per Execute, LegacyClonk 7d43b47
  src/StdAppUnix.cpp:288-291; Win32 never queues WM_TIMER twice,
  StdAppWin32.cpp:132). The behavior is verified against that C++ source, but
  the accompanying test only pins that the sub-second phase survives -- it does
  NOT distinguish one pulse from sixty, because the drained accumulator is
  identical either way and `sec1_timer` exposes no call counter. Pinning the
  coalescing itself needs a counting seam on that call.

- Test gap (2026-07-25): the `RELIABLE_UDP_SEND_BUDGET` fix below has no test
  pinning "a congested peer does not delay a different peer", because forcing a
  UDP socket to block deterministically is OS-dependent and this suite already
  carries timing-flaky socket tests that should be root-caused rather than
  retried. The normal path is covered by the 943 `clonk-network` tests. A real
  test wants an injectable send seam so the congested case can be simulated
  without a real blocked socket.

- Flaky test (fixed 2026-07-29): `clonk-network`
  `session::tests::dual_client_reconnects_a_missing_tcp_route` started its
  reconnect deadline as soon as it asked the proxy task to cut TCP, before
  that task had been scheduled to abort and await its copier and thereby drop
  both sockets. It then polled the host through its command channel on every
  scheduler yield; those queued inspection commands deliberately take priority
  over network arms and could starve the very disconnect/admission events the
  test awaited. The proxy now acknowledges completed cancellation, an
  event-driven host barrier observes route removal and replacement without
  command flooding, and the test proves UDP traffic remains live while TCP is
  held absent. Its only lifecycle deadline is the native 30-second ping-timeout
  horizon, not a Rust-only immediate-redial requirement; no retry was added.
- Flaky test (observed 2026-07-24, not fixed): `clonk-network`
  `session::tests::sync_controls_wait_for_status_barrier_and_keep_fifo_order`
  failed once in a full workspace run at `session.rs:13493`, asserting that no
  client event arrives within a 50 ms `timeout`. It passed 5/5 in isolation and
  the immediately following full run was 8834/8834. A negative assertion with a
  50 ms budget cannot survive a loaded scheduler; the barrier it is pinning
  should be observed by a deterministic signal rather than by absence-within-a
  -deadline. Same family as the entry above — root-cause, do not add a retry.
- Tutorial/UI: exact menus, HUD, evaluation, audio, and startup/options; 2×/3×
  startup-main text is native, while fractional/other scaled text remains blurred.
- ActMap `Sound=` (C4Object.cpp:4149-4152, 4186-4190) now emits the looping,
  object-attached start/stop pair. C++ emits it inside `SetAction`; the port
  reconciles the frame's resulting action slot in `reconcile_action_sounds`
  (`engine/tick.rs`) because Rust mutates action state from a dozen sites and a
  per-site hook would leak a stuck loop wherever one was missed. Sound is
  client-local presentation, so this costs no determinism. Residual: an
  A→B→A round trip completed *inside* one frame is not heard, where C++ would
  stop and restart within that same 1/36 s. Latent for shipped content — of the
  160 actions declaring `Sound=`, the 10 that live a single frame are all either
  self-looping (`NextAction` = own name) or `HOLD`, so none vanish intra-frame.
- Gameplay: exact landscape/material/PXS behavior, liquids, blasts, weather,
  movement/collision/attachment, vehicles, containers, and callback order;
  remaining spell effects/combo casts;
  mouse-context target refill, special cursors, and networking.
- Systems: strict C4Value/save semantics, remaining multiplayer transport and
  resync, exact C4Teams/SafeRandom assignment, configuration/localization, and
  group I/O.
- Graphics overlays: all seven modes now dispatch, and the walk is exhaustive so
  a new mode cannot be lost to a catch-all. MODE_Base selects the source
  definition's `(0, 0, Shape.Wdt, Shape.Hgt)` facet, applies `Shape.x/y` at the
  destination, and uses the source definition's `Scale`
  (`C4DefGraphics.cpp:636-637,815-821`). This keeps Hazard's floating
  spawnpoint items on their world-face geometry instead of drawing the embedded
  64×64 inventory picture. Residual divergences:
  - MODE_Action passes a source scale of 1.0 instead of the source definition's
    `Scale`, which C++ hands to `DrawT` for every facet mode
    (`C4DefGraphics.cpp:821`). Latent: no shipped content sets `Scale`.
  - `draw_object_face` reads geometry (Shape, GrowthType, ActMap) from the same
    sprite it blits from, so a cross-definition `SetGraphics(name, obj, idSrcDef, 0)`
    takes the SOURCE definition's geometry. C++ `SetGraphics(gfx, fTemp)` swaps
    only the bitmap (`C4Object.cpp:377-382`, `:4230-4245`). Not on the
    MODE_ExtraGraphics path: the shipped Knights shield passes `GetID()`, so host
    and source share a definition.
  - `C4Object::UpdateGraphics` does not forward `fTemp` to `UpdateFace`
    (`C4Object.cpp:406`), so each C++ MODE_ExtraGraphics draw runs `UpdatePos`
    and `UpdateSolidMask` twice per frame from inside the render loop.
    Deliberately not ported — the Rust frontend draws from an immutable
    snapshot. Inert for shipped content; do not read it as a Rust bug.

Comparator caveats: presentation RNG is opt-in; fields compare only when both
bridges expose them (the C++ bridge omits layer/visibility/player hostility).
Tutorial 07's seed-zero Surface8 is byte-identical; broader
same-seed landscape coverage remains incomplete. Component order is replay-
hashed but not exported by the C++ bridge; unequal-count duplicate IDs remain
an ordered-map model gap.

- Open: **A `!`-led expression loses any lexer error found past its unary
  operand, and `!Foo(<oversized literal>)` compiles.** `Parser::parse_unary`
  (`crates/clonk-script/src/parser.rs`) runs a speculative full-precedence pass
  whose AST is discarded, keeping only its error as a fallback for the narrow
  operand parse that follows. A lexer error never becomes a token, so
  `reset_speculative` has nothing to replay for the text the preflight already
  scanned past, and the lexer cursor stays beyond it. Measured on
  `func T() { return !Foo(99999999999999999999999); }`: with the preflight the
  script compiles as though the argument were absent; without it the parse
  fails with `integer literal out of range` at column 24. `!x = <oversized>`
  and `!x && <oversized>` likewise degrade to `unexpected token in expression`
  at the wrong column. Deleting the preflight restored the correct error and
  column in every probed shape with the whole `clonk-script` suite still green,
  but that is not the close: C++ never raises this error at all —
  `C4AulParse.cpp:704-744` scans literals through `%SCNdPTR`/`%SCNxPTR` and
  truncates at the `AB_INT` push — so what the oracle does with a literal wider
  than 64 bits has to be established before choosing between deleting the
  preflight and widening the truncation. Latent for shipped content: across the
  2,132 `.c` files under `content/` the widest literals are 10 decimal digits
  and 8 hex digits, none overflowing the lexer's `i64`/`u64` scan. Not covered
  by `parity verify`, whose golden compiles no such script.

- Open: **Nothing automated exercises a real window, event loop or GPU surface,
  so neither NVIDIA/Wayland segfault fix has a regression gate or a confirmed
  repro.** `--integration-test`, `--dump-frame` and `run_console_event_loop` all
  drive `GameApp` directly and render into the CPU surface; no test in the
  workspace constructs a `winit::EventLoop`, a `Window` or a `Pixels`. Both
  fixes are argued from source rather than observed: one retained
  `wgpu::Instance` per backend set, so closing a window never runs
  `vkDestroyInstance` while another holds a swapchain (clonk-org/clonk-rs#53,
  landed in clonk-org/clonk-rs#171), and `DeveloperWindows::release_all` on
  `Event::LoopExiting`, so no surface is destroyed after `run_app` has consumed
  the loop that owned its display (clonk-org/clonk-rs#54). Their unit tests pin
  the invariants — one instance per backend set, viewport windows destroyed
  before the shell — not the crash, and nothing pins that the runner still
  *calls* the teardown, because an `ActiveEventLoop` cannot be constructed in a
  test. Both were developed on macOS/Metal, where neither crash reproduces, so
  only a headed Linux/Wayland/NVIDIA run on the reporting hardware can retire
  either issue.

  Note for anyone tempted to simplify the second one by dropping the whole
  event-handler closure instead of just the window registry: on macOS
  `Event::LoopExiting` is dispatched from inside `applicationWillTerminate:`
  (`winit-0.30.13/src/platform_impl/macos/app_state.rs:166-172`), an AppKit
  callback that never returns to `run_app`. That would newly run
  `NetworkManager::drop`'s unbounded `blocking_send` + `join`
  (`crates/clonk-app-netplay/src/network.rs:4856-4865`) and the lobby preload
  worker's cancellation-free join inside the OS's own quit, where a slow worker
  hangs termination and a panicking drop unwinds across `extern "C"` and aborts.
  `clonk-launcher-shell` releases its runtime from the same event
  (clonk-org/clonk-rs#174) and *may* drop the whole thing there, where the game
  may not: it owns no threads, channels or sockets, so the heaviest drop on
  that path flushes a `LineWriter`.

  Open in the launcher: it builds its framebuffer with plain `Pixels::new`, so
  the surface-lost rebuild in `LauncherRuntime::rebuild_framebuffer` destroys a
  `wgpu::Instance` and builds another, rather than reusing a retained one. Left
  that way deliberately — the process opens one window and holds one instance,
  so nothing else is presenting when that `vkDestroyInstance` runs, which is not
  the shape of clonk-org/clonk-rs#53 — and because the registry lives inside the
  `clonk-app` *binary* (`crates/clonk-app/src/gpu_instance.rs`), out of reach of
  another crate: `clonk-app-render` is the only shared crate that depends on
  `pixels`, and it would pull `clonk-engine` and `clonk-frontend` into the
  launcher with it. Sharing it needs a new `pixels`-only crate, which is worth
  doing if a second window, or a driver that cannot re-initialise after its last
  instance goes, ever makes the launcher present through two of them.

## Deliberate divergences from the oracle

- **A one-column script menu may claim the horizontal controls as a step**
  (`Engine::object_menu_step`, `crates/clonk-engine/src/direct_com.rs`; no key,
  opt-in per menu; C++ `C4Menu::Control`, LegacyClonk 7d43b47
  src/C4Menu.cpp:433-457). Approved 2026-08-04, follows
  clonk-org/clonk-rs#119. Every style but `C4MN_Style_Normal` forces
  `Columns = 1` (`C4Menu.cpp:359-365`), and at one column `COM_MenuLeft` and
  `COM_MenuRight` compute *exactly* the deltas `COM_MenuUp`/`Down` already
  carry, wrap included — so the horizontal pair is dead weight in every Context
  menu C++ has. Before turning one into a selection move the port now offers it
  to the menu's own command object (or the scenario script, matching
  `C4ObjectMenu::OnSelectionChanged`'s target choice, `C4ObjectMenu.cpp:78-104`)
  as `~OnMenuStep(iDelta, pMenuObject)`; a truthy return consumes it. Anything
  else — a menu that is not script-created, more than one column, no such
  function, a falsy return, a script error — runs the shipped move unchanged,
  so the observable behaviour of every menu in every pack that does not
  implement `OnMenuStep` is byte-identical. That is the whole existing corpus:
  the callback is new, so nothing outside this repository can be reached by it.
  This buys the horizontal-axis-adjusts-the-selected-value convention that list
  UIs are expected to have, which is what a quantity row needs and what a
  hidden right mouse button cannot give a keyboard or gamepad player.
  **Blast radius.** Menu coms are synchronized like any other control, and the
  callback runs on the same tick in the same order for every peer, so this
  cannot desync a port-to-port game; against a stock LegacyClonk client the
  scripts that implement it do not exist, because they ship in
  `planet/System.c4g`, which is the port's own engine data. `parity verify`
  and `engine-snapshots verify` execute no menu control at all. Pinned by
  `a_one_column_script_menu_offers_left_and_right_as_a_step_before_moving`,
  `an_unclaimed_step_still_moves_the_selection_exactly_as_it_did` and
  `a_multi_column_menu_never_offers_a_step`.

- **A capsule order is priced before it is delivered**
  (`planet/System.c4g/MarsOrderCapsule.c`, `#appendto BASE`; departs from
  `content/ClonkMars.c4d/Structures.c4d/Base.c4d/Script.c:133-155`). Approved
  2026-08-04, follows clonk-org/clonk-rs#119. The shipped commit spends the
  player's money one item at a time with no check and no report: `Buy` is
  called without `fShowErrors`, so `C4Player::Buy` takes its silent branch and
  neither `IDS_PLR_NOWEALTH` nor the Error sound is produced
  (`C4Player.cpp:849-853`); the first item that does not fit makes the loop
  `return true`, abandoning every product still to come; and because the hash
  iterates in bucket order (`ClonkMars.c4d/System.c4g/HashTable.c:173-189`),
  *which* half of the order arrives is unrelated to the order on screen. The
  capsule is created before any of that, so the one-capsule allowance and its
  five-minute cooldown are spent either way. Cerberus Fossae starts the player
  on `Wealth=30` against 186 clunkers of stock
  (`01_Fossae.c4s/Scenario.txt:17,21`), so filling the order page is the
  ordinary outcome, not an edge case. The append prices the order first —
  through the same `GetValue` the rows are captioned with, under the same stock
  re-clamp the commit applies — refuses it whole when it cannot be paid for and
  says so, reports what was sent when it can, and passes `fShowErrors` so
  anything that still slips through gets the engine's own message. `Buy`'s own
  arithmetic, `CapsuleCheck` and its refusals, `CreateCapsule` and the SellOnly
  branch are untouched.
  **Blast radius.** Wealth and homebase stock are synchronized, and this
  changes *how many* `Buy` calls happen in the refused case — from a partial
  run to none — so a port peer and a stock LegacyClonk peer would diverge on
  an unaffordable order. `planet/System.c4g` never reaches a stock client. No
  draw site is added or removed. Pinned by
  `an_order_over_the_players_wealth_is_refused_whole` and
  `an_affordable_order_is_delivered`.

- **A Menu2 range is one row that adds on a primary and takes back on a
  secondary activation** (`planet/System.c4g/MenuRangeRow.c`, `#appendto MS4C`;
  departs from `content/ClonkMars.c4d/Helpers.c4d/Menu2.c4d/Menu.c4d/
  Script.c:108-143`). Approved 2026-08-04, clonk-org/clonk-rs#119. The shipped
  `ShowRange` expands one range into three `AddMenuItem` rows — the value, an
  `Increase by 1` and a `Decrease by 1` whose captions name no product because
  the pack's authors commented out the per-product wording that its own
  `StringTbl*.txt` still carries (`Menu.c4d/Script.c:112-115`). Cerberus Fossae
  therefore listed 16 rows for five products, in a menu the engine draws as one
  narrow column of single-line rows: `C4MN_Style_Context` forces `Columns = 1`
  (`C4Menu.cpp:359-365`) and `InitLocation` gives each row one line of height
  (`C4Menu.cpp:650-664`), so there is no wider layout for those rows to use.
  The append collapses a range to one row and spends the engine's two
  activations on the two directions: `C4Menu::Enter` runs `C4MenuItem::Command2`
  on a right enter (`C4Menu.cpp:512-514`), which the engine composes itself out
  of the same command string (`C4Script.cpp:1630-1670`) and the player reaches
  with the right mouse button (`C4Menu.cpp:228-232`), with `[Special2]`
  (`C4Menu.cpp:1053`) or through `COM_MenuEnterAll` (`C4Menu.cpp:440`) — the
  same two activations the bottom key strip already advertises
  (`C4Menu.cpp:846-880`). This is not a new interaction for a purchase menu:
  the engine's own `C4MN_Buy` rows are one per product and already put a
  different quantity on `Command2` (`C4ObjectMenu.cpp:246-271`, ported at
  `crates/clonk-engine/src/direct_com.rs:5060-5069`). The direction differs
  deliberately — that menu buys immediately, where this one composes a pending
  order that nothing else can reduce. Because a hidden secondary action is only
  usable if the row says it is there, each row spells out both steps from its
  first frame and greys the one its limits forbid rather than dropping it — so a
  value on a limit says so instead of promising a click the shipped `BoundBy`
  would discard, and because colour markup carries no width the row is exactly
  as wide at every value and the menu never resizes under a pointer that is
  clicking it. The info caption, previously a verbatim duplicate of the row it
  covers, states the current value and both bindings, localized from a new
  `planet/System.c4g/StringTbl{US,DE}.txt`.
  **Blast radius.** Presentation of a script-built menu: `Increase`, `Decrease`
  and their clamp are untouched (`Menu.c4d/Script.c:178-198`), so a committed
  order is unchanged. `parity verify` and `engine-snapshots verify` cannot see
  it — neither executes content C4Script — and the replay goldens run no
  ClonkMars scenario. One piece of synchronized state does move: the shipped
  non-selectable branch creates and removes three dummy objects per range row
  and this one creates one, so the object-number counter advances differently
  once a greyed range is drawn. No draw site is added or removed on any branch,
  so `RandomCount` is untouched. Cross-play against a stock LegacyClonk client
  would diverge on that counter; `planet/System.c4g` is the port's own engine
  data and never reaches one. Both Menu2 clients are covered — ClonkMars' Base
  order page (`Base.c4d/Script.c:115-131`) and the viewport size chooser, whose
  ranges use a string key and no item id (`Viewport.c4d/Script.c:61-62`).
  Pinned by `mars_order_page_collapses_each_product_to_a_single_row`,
  `mars_order_row_offers_both_steps_on_the_product_row`,
  `mars_order_row_adds_on_a_left_enter_and_takes_back_on_a_right_one`,
  `mars_order_row_stops_offering_a_step_it_cannot_take`,
  `mars_order_row_moves_only_its_own_product_and_keeps_the_selection`,
  `menu2_range_rows_collapse_for_a_string_key_without_a_symbol_too`,
  `a_range_whose_condition_fails_collapses_to_one_inert_row`,
  `collapsing_the_rows_spends_no_synchronized_draw`,
  `colour_markup_in_a_context_caption_costs_no_row_width` and
  `the_row_hint_is_localized_from_the_system_group`.
  **Extended 2026-08-04** after the reporter could not reach `-1` at all in
  play. Nothing on screen names the right mouse button, so five further
  changes make the page usable without it, all in the same append:
  the left/right controls step the selected range through the `OnMenuStep`
  callback above; an **undo row** appears above the closing row as soon as a
  value moves, names what it will take back, and is reached by an ordinary
  activation, so no part of composing an order needs a hidden control; the
  quantity stays on screen at zero via `C4MN_Add_ForceCount`, where
  `C4Script.cpp:1726` would substitute `C4MN_Item_NoCount` and hide the column
  entirely; the closing row says **Back** on a submenu and **Finished** at the
  root, which shipped Menu2 captions identically although one leaves a page and
  the other spends money (`Menu.c4d/Script.c:54,206-228`); Escape abandons the
  order from any page, where `MenuQueryCancel` used to pop one level so the
  ordering UI could not be dismissed from the Order page at all; and an
  unchosen enum row is left blank instead of taking the whole red-cross cell
  from Menu2's sheet (`Menu.c4d/Script.c:255-261`), which beside "Only Sell"
  read as forbidden rather than as the other half of a radio pair. `ShowMenu`
  and `ShowEnum` are now rebuilt rather than inherited, so the closing row and
  the radio symbols are ours; the MS4C index constants are spelled as literals
  with citations, because Menu2's own `System.c4g` is not registered when
  `planet/System.c4g` parses.
  **Replacing a shipped function is a silent-loss risk, and it is guarded.**
  An `#appendto` override always wins, so a content bump that changed one of
  the replaced functions would be discarded with nothing failing.
  `crates/clonk-engine/tests/it/mars_menu_override_drift.rs` reads the shipped
  source straight out of the submodule — through `clonk_resources::Group`,
  since `Menu.c4d` is a packed C4Group — and pins the exact text of every
  function these appends replace (`ShowMenu`, `ShowEnum`, `ShowRange`,
  `MenuQueryCancel`, `OrderCapsule`), every function they call and lean on
  (`IncreaseRange`, `DecreaseRange`, `Finished`, `CreateDummy`,
  `GetMenuValues`, `CapsuleCheck`, `CreateCapsule`, `ContainedUp`), and the
  `MS4C_*` index declarations the literals stand for. A bump that touches one
  fails with the new source in the message. That failure is not a defect: read
  the new function, decide whether this entry still says what we want, re-pin.
  One trap this uncovered and the append guards:
  re-rendering a page with `CreateMenu` while the menu is **open** asks the
  owner whether the close is denied (`C4Script.cpp:1525`), and Menu2 answers by
  aborting the template — row commands never hit it because `C4Menu::Enter`
  closes a non-permanent menu first (`C4Menu.cpp:517`), but a step control does.
  Also pinned by `mars_order_row_steps_with_the_left_and_right_controls`,
  `mars_order_arrows_still_navigate_off_a_product_row`,
  `the_order_page_offers_undo_only_once_there_is_something_to_undo`,
  `the_undo_row_stays_on_the_page_its_change_belongs_to`,
  `every_product_shows_its_quantity_even_at_zero`,
  `the_closing_row_says_which_of_its_two_jobs_it_is_doing`,
  `escape_abandons_the_order_from_the_order_page`,
  `an_unchosen_mode_row_is_blank_rather_than_crossed_out` and, through the real
  app input layer,
  `context_style_script_menu_reaches_command2_by_right_click_and_special2`.
  **Extended again 2026-08-04** with the running total that was left open.
  `ShowMenu` asks the menu's owner for a figure through
  `~MenuFooterValue(values, ExtraData)` — the same shape the commit callback
  receives — and passes `C4MN_Extra_Value` to `CreateMenu` when it gets one,
  where shipped Menu2 passes `iExtra = 0` and opts out of the only money footer
  the engine has. That draws the order total beside the wealth symbol *and*
  arms the player's wealth HUD for the duration (`C4Menu.cpp:898-906`;
  `C4Viewport.cpp:1286-1296` otherwise keeps the counter hidden), so both
  halves of "can I afford this" are on screen while the order is composed
  rather than only in the refusal afterwards. Because the footer draws the
  *selected* item's value (`C4Menu.cpp:830-841`), every row carries the same
  figure through `C4MN_Add_PassValue` or the total would blink out as the
  selection moved; that flag only rewrites OLD-style command composition
  (`C4Script.cpp:1556-1597`) and every command this append builds is new-style,
  so the two compose safely. A menu whose owner does not answer keeps the
  shipped footer-less page, which is what the viewport size chooser does.
  **A latent layout defect surfaced with it and is worked around here.** A
  Context row is sized from its caption and symbol only (`C4Menu.cpp:648-662`)
  while the count is drawn right-aligned at the row's right edge regardless
  (`C4Menu.cpp:198-207`), so the widest caption on a page ends exactly where
  its own count starts and the two overprint — visible as `(+1/2)x` on the
  construction-kit row once `C4MN_Add_ForceCount` gave every row a count. Only
  the caption is measured, so only the caption can reserve the room: each range
  row now appends three spaces per digit of its maximum plus three. This is a
  C++ defect, not a port one — stock ClonkMars overprints the same way whenever
  its widest row carries a quantity — and sizing the row from the count instead
  would change `ItemWidth` for every Context menu in every pack, which is why
  it is worked around in content. Pinned by
  `the_order_page_shows_what_it_will_cost`,
  `a_menu_whose_owner_prices_nothing_keeps_the_shipped_footer` and
  `a_product_row_reserves_the_column_its_quantity_is_drawn_in`.

- **A refused default-interface multicast join falls back to per-interface
  joins** (`join_discovery_multicast`, `multicast_targets`,
  `multicast_interface_indices`, `crates/clonk-network/src/search.rs`; no key;
  C++ `C4NetIOSimpleUDP::InitBroadcast`, LegacyClonk 7d43b47
  src/C4NetIO.cpp:1620-1631). Approved 2026-08-03, clonk-org/clonk-rs#107. C++
  joins `ff02::1` with `ipv6mr_interface = 0` and sends to the group with the
  destination scope unset, under an acknowledged `// TODO: do multicast on all
  interfaces?` (src/C4NetIO.cpp:1623); where the kernel's default multicast
  route has no IPv6-capable interface — a Mac whose only LAN NIC has IPv6
  switched off is enough — the join returns `EADDRNOTAVAIL` and every send
  `EHOSTUNREACH`, so LAN discovery is dead in both directions and the reporter
  could neither see games nor be seen. The port keeps the C++ attempt first and
  reaches for `if_nameindex` only after the kernel has refused it, joining every
  interface that accepts the group and giving each its own destination scope;
  wherever the C++ join succeeds the wire behaviour is byte-identical, one
  unscoped datagram, and no interface is ever enumerated. Nothing
  simulation-facing reads discovery — it selects which game to join, before any
  control is exchanged, and the reference it fetches is the same document either
  way — so this cannot desync. Two consequences are accepted: on a host that
  needs the fallback, probes reach interfaces that cannot carry a reply and
  those queries expire through the ordinary reference-query timeout, and a host
  and client on the *same* such machine list one game once per shared interface,
  because the host rewrites its advertised addresses per source scope. That
  second case also loops each announce back once per shared interface: measured
  at 49 of the 64 `MAX_LAN_DISCOVERS` slots on the reporter's 22-interface Mac,
  so a machine with nine or more loopback-capable joined interfaces could spend
  the cap on itself — bounded, reset at each 30 s probe, and only reachable by
  running a host and the network dialog as two processes on one such machine,
  which the app itself never does because every host path drops the searcher
  first. Windows keeps the C++ behaviour unchanged — enumerating there needs
  `GetAdaptersAddresses`, which no required gate compiles for this crate. Pinned
  by `discovery_multicast_target_uses_cpp_default_interface`,
  `an_accepted_default_multicast_join_enumerates_no_interfaces`,
  `a_refused_default_multicast_join_keeps_every_joinable_interface`,
  `a_scoped_join_set_sends_one_probe_per_joined_interface`,
  `an_unjoinable_host_still_probes_the_cpp_default_interface`,
  `the_cpp_default_interface_never_sets_ipv6_multicast_if` and
  `enumerated_multicast_interfaces_do_not_depend_on_kernel_listing_order`.

- **An explicit refresh rebuilds a discovery socket that reached no group**
  (`discovery_needs_rebuild`, `crates/clonk-network/src/search.rs`; no key; C++
  `C4StartupNetDlg::DoRefresh`, LegacyClonk 7d43b47
  src/C4StartupNetDlg.cpp:1093-1105). Approved 2026-08-03,
  clonk-org/clonk-rs#107. C++ builds `DiscoverClient` once in the dialog
  constructor and discards the result (src/C4StartupNetDlg.cpp:737);
  `DoRefresh` only re-sends the probe byte, so a machine that had no network
  when the dialog opened stays blind until the dialog is reopened. The port
  re-runs socket construction on an explicit refresh, and only when the
  existing socket joined nothing at all — a socket
  that reached a group keeps its buffered replies exactly as C++ does. This is
  matchmaking state outside the lockstep simulation and cannot desync. Pinned by
  `only_an_unusable_discovery_socket_is_rebuilt_on_refresh`.

- **Earned mission access is written when it is earned**
  (`GameApp::persist_mission_access_if_changed`,
  `crates/clonk-app/src/game_app/config.rs`; no key). Approved 2026-07-31,
  clonk-org/clonk-rs#50. C++ mutates `Config.General.MissionAccess` in memory
  at both of its sites — `FnGainMissionAccess` (`C4Script.cpp:2368-2373`) and
  the cheat-code add/remove (`C4StartupScenSelDlg.cpp:1828-1839`) — and writes
  it only when the whole config is saved, on a clean quit
  (`C4Application.cpp:367`); a round that ends any other way silently relocks
  the mission the player just finished. The reporter hit exactly that. Unlike
  the runtime toggles that share the deferral, a password is earned progress,
  not a preference, so the port writes it as soon as the shared list changes:
  the event loop compares the store against the
  value on disk once per iteration, ahead of every early exit. Nothing
  simulation-facing reads the timing — script sees the same in-memory list
  either way — so this cannot desync. The value is written in C++'s escaped
  form (`MissionAccess="…"`, `C4Config.cpp:379`); the previous unquoted
  `RawAscii` write was unreadable to a LegacyClonk install sharing the file,
  since `StdCompilerINIRead` requires the quotes for an `RCT_Escaped` field.
  Pinned by `script_earned_mission_access_reaches_the_saved_config` and
  `earned_mission_access_survives_an_aborted_session`.

- **The fullscreen loader background may be aspect-filled**
  (`LoaderRenderConfig::with_aspect_fill`, `crates/clonk-frontend/src/loader_screen.rs`;
  opt-in `Graphics.LoaderAspect`). Approved 2026-07-28. `C4LoaderScreen::Draw`
  reaches `C4Facet::DrawFullScreen`, which stretches the source across the
  whole target with no aspect preservation, so the shipped 3840x2880 (4:3)
  loaders are squashed into 16:9 and bilinearly minified on a 4K panel. With
  the key on the image is centre-cropped to the target's aspect instead —
  which on a 16:9 4K panel makes it an unscaled 1:1 blit. Off, the blit stays
  bit-identical to C++.

- **The fog-of-war modulation grid may be subdivided**
  (`fine_fog_cell_divisor`, `crates/clonk-frontend/src/fog_modulation.rs`;
  opt-in `Graphics.FineFogOfWar`). Approved 2026-07-28. `Landscape.FoWRes`
  defaults to 64 world pixels and the boundary is interpolated only at quad
  corners, so the visibility edge shows 64px polygonal facets — very obvious at
  4K. The value feeds only `ClrModMap::reset` and nothing in the simulation
  reads it back, so subdividing it renderer-side (to 16px cells) is
  presentation-only and cannot desync; the snapshot's own value is untouched.

- **Higher-resolution GUI sheets are recognised by their dimensions**
  (`GuiArtScale::detect`, `crates/clonk-frontend/src/hud.rs`; no key — the
  opt-in is the presence of the art). Approved 2026-07-28. Graphics.c4g carries
  no per-sheet scale metadata (DefCore `Scale=` covers object definitions
  only), C4Facet derives cell sizes straight from the loaded surface while the
  HUD hard-codes sub-rects into the same sheets, so a larger replacement sheet
  used to magnify the layout or mis-slice the grid. A sheet that is an exact
  integer multiple (2x-8x) of the oracle's dimensions is now treated as art at
  that scale: hard-coded source rects are multiplied, dimension-derived cells
  divided, so logical destination geometry stays bit-identical. Resolution also
  prefers an `@4x`/`@3x`/`@2x` stem. With stock 1x content every path is the
  oracle blit byte-for-byte.

- **`Graphics.Monitor` selects the fullscreen monitor**
  (`crates/clonk-app/src/main_parts/assets.rs`). Approved 2026-07-28.
  `C4Config.h` declares the field and `C4Config.cpp` defaults it, but the
  oracle's SDL/GL build never reads it back, so the row is inert in C++. The
  port honours it: borderless fullscreen opens on the Nth enumerated monitor.
  Out-of-range and non-positive values keep the previous behaviour, so the
  default configuration is unchanged.

- **HD definition art may blit one authored texel per device pixel**
  (`GraphicsSystem::set_hd_exact_blits`; opt-in `Graphics.HDExactBlits`).
  Approved 2026-07-28. `stdgl_blit_sampling` forces `Linear` whenever the
  application scale is not 1 and `CStdDDraw` then insets the source by half a
  texel (src/StdGL.cpp:527, src/StdDDraw2.cpp:676-688), and exactness is tested
  in logical space. A `DefCore Scale=200` sheet drawn at `Graphics.Scale=200`
  is therefore texel-perfect in PHYSICAL pixels yet is resampled with a
  half-texel drift — which is what makes upscaled art look like a photograph of
  pixel art. With the key on, exactness is tested against
  `destination * presentation_scale` and both the correction and the forced
  Linear are skipped. The default path still satisfies the C++ pinning test
  unchanged.

- **Save thumbnails are area-averaged rather than 2-tap sampled**
  (`downsample_rgba_box`, `crates/clonk-graphics/src/surface.rs`; no key).
  Approved 2026-07-28. C++ reduces the frame with `CStdDDraw::Blit`'s two-tap
  sampler; at a 4K frame into a 200x150 box that reads 2 of ~20 source pixels
  per axis, i.e. point sampling, so thin scenery aliases into noise. The port
  averages every source pixel that falls in a destination cell, in
  premultiplied space so transparent regions do not darken the result. The
  thumbnail is metadata, not simulation state, and nothing pins its bytes.

- **The scale-native glyph atlas serves any resolved font recipe**
  (`ClassicNativeFontSource::sizes`; `Graphics.SnapTextToPixels` for the
  fractional-scale half). Approved 2026-07-28. The device-resolution atlas is a
  port enhancement with no C++ counterpart, and it previously only built for
  the literal 22/16/14/13/12 role map, so any other `General.FontSize` or
  scenario `Head.Font` silently dropped back to blurred logical raster — and at
  Scale>100 the classic loader refused outright. It now carries the resolved
  per-role sizes. Separately, `Graphics.SnapTextToPixels` rasterizes at
  `round(logical * scale)` and blits on whole physical pixels so fractional
  scales stop resampling every glyph.

- **The landscape may be magnified with alpha-weighted reconstruction**
  (`sample_landscape_smooth` in `LANDSCAPE_SHADER`, `crates/clonk-app-render`;
  opt-in `Graphics.SmoothLandscape`). Approved 2026-07-28. C++ blits the
  landscape surface with GL_NEAREST, so above 1:1 the largest surface on
  screen becomes hard colour blocks — increasingly so now that the first-run
  application scale follows the display density. Plain bilinear cannot be used
  here: the cache stores sky as `RGBA(0,0,0,0)` against opaque material
  (`output.fill(0)`, `crates/clonk-frontend/src/graphics_system.rs`), so an
  ordinary tap rings every silhouette with a dark halo. Weighting colour by
  coverage takes colour only from texels that have any while alpha still ramps
  across the boundary, which is what turns a magnified one-game-pixel step
  into an antialiased edge. At 1:1 the reconstruction degenerates to the
  nearest texel, so nothing changes until the frame is actually magnified.
  Pinned on a real device by
  `smooth_landscape_magnification_antialiases_without_a_sky_halo`, which also
  asserts the default path stays a hard nearest step.

- **Retained art may be minified through a mip chain**
  (`wants_mipmaps`/`generate_mip_chain` + `linear_mip_sampler`,
  `crates/clonk-app-render`; opt-in `Graphics.Mipmaps`). Approved 2026-07-28.
  C++ binds GL_LINEAR with no mip levels, so every minified draw is a single
  bilinear tap and aliases — which penalises exactly the higher-resolution art
  a `DefCore Scale=` pack would ship, and shimmers on the 3840x2880 loader
  backgrounds. Levels are box-filtered on the CPU from the complete backing a
  resource always carries, in premultiplied space so transparent surrounds do
  not bleed into minified sprite edges, and only for resources that never
  change (`base_revision.is_none() && dirty.is_empty()`) — the landscape cache
  and liquid animation keep one level and bind Nearest regardless. Pinned by
  `mip_chain_averages_in_premultiplied_space_and_halves_to_one_texel` and
  `only_unchanging_sources_get_a_mip_chain`.

- **All presentation-only divergences share one master switch**
  (`Graphics.Remaster`, `configured_remaster_feature`,
  `crates/clonk-app/src/main_parts/assets.rs`). Approved 2026-07-28. It only
  supplies the default for `HighDpiCursor`, `SkyDither` and `Mipmaps`; a key
  the player wrote explicitly still wins in both directions, and with nothing
  configured every one of them is off and the renderer stays C++-exact. Pinned
  by `the_remaster_switch_supplies_a_default_that_each_key_can_override`.

- **Presentation may run at the display's refresh period instead of the
  oracle's 30 ms ceiling** (`configured_smooth_presentation` +
  `effective_max_refresh_delay_ms` + `refresh_interval_for_tick`,
  `crates/clonk-app`; opt-in `Graphics.SmoothPresentation`, default off).
  Approved 2026-07-29. C++ defaults `Graphics.MaxRefreshDelay` to 30
  (C4Config.cpp:485) against a 28 ms game tick, so `C4Application` leaves that
  tick undivided (C4Application.cpp:510-531) and presents once per tick; the
  startup timer is a flat 16 ms. That is correct for world content, which
  really does advance only once per tick — but the mouse pointer is composited
  *into* the frame while the platform cursor is hidden
  (`classic_platform_cursor_visible`), so the refresh period is also the
  pointer's update period: measured 35.7 Hz in game and 62.9 Hz in the startup
  menu against a 120 Hz panel whose GPU pass costs 0.83 ms and whose event loop
  is idle 96 % of the time. When enabled, the panel period (clamped so it can
  never be slower than the oracle default) replaces only the *ceiling* of the
  **startup timer**; the divisor applied to it is C++'s own, and the 16 ms logic
  tick keeps its exact rate, so menu animation ages identically.
  The **game timer keeps the oracle ceiling unconditionally** (`RefreshCeilings`),
  which is why this is safe: all four C++-mirrored per-render behaviours (the
  C4Viewport camera smoother, `C4MessageBoard::Execute` plus the screen fader,
  flash-message `remaining_draws`, and the object-audibility cache) live in the
  running path and never see a changed cadence. Subdividing the game timer was
  measured and rejected: on an M4 Max at Scale=300 fullscreen a 7 ms ceiling
  moved presentation from 35.66 to 36.30 FPS while the average graphics pass
  grew 10.49 -> 18.17 ms and automatic frame skips went 2 -> 98, because in game
  the pass cost and swapchain back-pressure bind long before the timer does.
  In-game pointer smoothness is therefore still bounded by graphics-pass cost,
  not by this key. **This supersedes the unlogged
  `DEFAULT_MAX_REFRESH_DELAY_MS = 16` divergence** that `469eca304`
  (2026-07-20) introduced and `d9315f876` (2026-07-24) correctly reverted for
  being unlogged — the default now stays at the oracle's 30 permanently and the
  faster cadence is reachable only through this key or `Graphics.Remaster`.
  Pinned by `smooth_presentation_substitutes_the_display_period_for_the_native_ceiling`
  and `startup_refresh_honours_the_same_refresh_ceiling_as_the_game_timer`;
  `max_refresh_delay_defaults_to_cpp_30_ms_and_honors_positive_config` and
  `max_refresh_delay_missing_or_invalid_matches_cpp_thirty_ms` still hold the
  default path unchanged.

- **A diagnostics overlay may report the presentation rate the oracle's FPS
  counter cannot see** (`PresentationStats` +
  `GameApp::update_diagnostics_overlay` +
  `GraphicsSystem::draw_diagnostics_overlay`; opt-in `Graphics.ShowStats` plus
  a default-unbound `StatsToggle` key, both off). Approved 2026-08-05, follows
  clonk-org/clonk-rs#118. `C4UpperBoard` draws one frame rate under
  `Config.General.FPS` (src/C4UpperBoard.cpp:81-86), and it is `C4Game::FPS`:
  `cFPS++` counts executed *game* frames (C4Game.cpp:1915-1916) and
  `C4Game::Sec1Timer` samples it (C4Game.cpp:1758-1762). C++ presents once per
  tick, so there that single number is also the render rate. This port already
  diverges on exactly that point — `Graphics.SmoothPresentation`, the
  presentation-detail governor, automatic graphics skips and the
  refuse-to-draw-while-inactive gate all move the present rate independently of
  `frames_since_second` — so the ported counter kept a label that is no longer
  true here, and a presentation stall cannot reach the screen at all. Measured
  on `content/mods/Super_Mega_Ultra_Extrem_Wettlauf.c4s` at `fd3465c0e`: 35.7
  simulation FPS held steady across a 9.03 -> 0.93 collapse in
  `presentation_submission_fps`, and establishing which half was slow cost a
  two-hour investigation (causes filed as clonk-org/clonk-rs#158 and #159). The
  overlay carries numbers the tree already computes and previously emitted only
  as one `LC_APP_PRESENTATION_BENCHMARK` line at process exit: both frame rates,
  the last and p95 graphics-pass cost, automatic graphics skips per second, and
  — in a network round — the worst route's ping and loss beside control behind,
  PreSend, measured lateness and the `ControlLatencyEstimator` envelope PreSend
  is actually sized from. That last pair needed new read-only accessors; the
  script- and dialog-visible `ACT` field stays C++'s ping-derived mean and is
  untouched. `StatsToggle` is registered after every C++ action and yields the
  chord to all of them, so no shipped binding changes meaning.
  **Blast radius.** Presentation-only in both directions. Nothing on this path
  reads or writes `C4Fixed`, `C4Random`, movement, control ordering or anything
  else determinism-critical, so two clients with the key set differently stay in
  lockstep and cross-play against a stock LegacyClonk client is unaffected.
  Composing no text *is* the gate, so with the key unset there is no draw site
  and the frame is byte-identical to the one shipped today —
  `C4Network2::DrawStatus` keeps its oracle-pinned (+20,+50) anchor
  unconditionally, and the port-only panel is the one that shifts below it when
  both are visible. `parity verify` and `engine-snapshots verify` cannot observe
  it; neither renders. Pinned by
  `presentation_stats_count_the_present_rate_the_game_tick_counter_cannot_see`,
  `presentation_stats_summarize_what_the_graphics_pass_cost_that_second`,
  `presentation_stats_bound_the_samples_one_second_may_retain`,
  `show_stats_is_opt_in_and_follows_the_native_boolean_grammar`,
  `the_diagnostics_overlay_reports_both_frame_rates_and_stays_off_by_default`,
  `the_diagnostics_overlay_reports_the_horizon_a_stalling_client_is_sized_from`,
  `stats_toggle_is_default_unbound_and_a_custom_chord_flips_the_overlay`,
  `the_measured_horizon_inputs_are_readable_after_a_control_tick`,
  `the_diagnostics_overlay_is_inert_until_it_is_given_text` and
  `the_diagnostics_overlay_stands_clear_of_the_network_status_block`.

- **An unfocused window keeps drawing, and a hidden one stops**
  (`load_render_inactive_mask` + `render_inactive_allows_drawing` +
  `GameApp::window_occluded`, `crates/clonk-app`; the `Graphics.RenderInactive`
  default becomes Fullscreen|Console, and `WindowEvent::Occluded` gates every
  mask). Approved 2026-08-05, fixes clonk-org/clonk-rs#57.
  `C4GraphicsSystem::StartDrawing` refuses to draw an inactive window unless the
  active shell's bit is set (C4GraphicsSystem.cpp:96-106) and `C4Config` adapts
  the default to `Console` alone (C4Config.cpp:481), so Alt-Tab stops a
  fullscreen game's picture. Only the *graphics* half of
  `C4Application::Execute` is gated on activity, though — `Game.Execute()` is
  not (C4Application.cpp:451-478) — so the round runs on, and in a network game
  it must: lockstep means every other peer is waiting on this client's control.
  The oracle default therefore freezes the picture at the moment of
  deactivation, advances the world behind it, and snaps forward on refocus,
  which reads as the fast-forward filed as clonk-org/clonk-rs#56 over a session
  that never stalled. Measured before this change on macOS: an inactive shell
  presented **1 frame in 197 s while the simulation executed 7062**. The
  divergence is the *default* only; a `RenderInactive` the player writes is
  honoured verbatim in both directions, and `RenderInactive=2` restores C++
  exactly. C++ needed no notion of a hidden window because Win32 deactivation
  minimizes its fullscreen window (C4FullScreen.cpp:139-145) — its inactive gate
  already covered that case, and once the port draws while unfocused the two
  come apart, so occlusion now refuses on its own (macOS occlusion state, X11
  `VisibilityFullyObscured`; Wayland and Windows do not report it and keep
  drawing). Both refusals re-arm `RenderFloor::note_refused_presentation`, so
  neither can bank the graphics-deadline debt that `2dd4d6a65` removed.
  **Blast radius.** Presentation-only, and in the safe direction: this adds
  frames, never removes a simulation step. Nothing on the path reads or writes
  `C4Fixed`, `C4Random`, movement or control ordering, and the gate is
  per-client local state that no peer can observe, so two clients configured
  differently stay in lockstep and cross-play against a stock LegacyClonk client
  is unaffected. `parity verify` and `engine-snapshots verify` cannot see it;
  neither presents. The composited in-frame pointer is still withheld while
  inactive (`platform_cursor_visible`), so an unfocused window draws the world
  without claiming to own the cursor. Pinned by
  `the_shipped_default_keeps_an_unfocused_game_window_drawing` and
  `a_hidden_window_draws_no_frames_however_the_mask_is_set`;
  `render_inactive_bitmask_gates_unfocused_fullscreen_and_console_redraw`,
  `the_inactive_gate_never_withholds_the_first_frame` and
  `startup_activity_matches_native_rather_than_the_windowing_systems_focus_report`
  keep the per-bit behaviour, the first-frame exemption and the seeded activity
  flag unchanged.

- **The sky gradient may be dithered below the 8-bit step**
  (`GpuSolidStyle::dither` + `SOLID_SHADER`, `crates/clonk-app-render`; opt-in
  `Graphics.SkyDither`, default off). Approved 2026-07-28. C++ emits the sky
  fade as one interpolated quad into an 8-bit target, so the number of visible
  bands equals the channel delta spread over the viewport height: the shipped
  default fade `RGB(28,64,152)→RGB(192,196,252)` spans 100 blue steps, i.e. a
  band every ~22 rows at 2160p, and it gets strictly worse as panels grow. The
  divergence adds interleaved-gradient noise on a triangular PDF spanning one
  step before the framebuffer quantizes; the mean is unchanged, so the result
  is closer to the exact ramp than the banded output. It is set only on a quad
  whose corner colours actually differ (`no_box_fades` flattens its quad first)
  and only on the sky path, and it is presentation-only. Pinned by
  `sky_dither_marks_a_real_gradient_only_when_enabled`,
  `solid_triangle_vertices_carry_gamma_and_dither_in_separate_flag_channels`,
  and `sky_dither_defaults_off_and_reads_the_native_boolean`; the default path
  stays byte-identical under `gpu_renderer_matches_cpu_reference_frame`.

- **A first launch seeds the application scale from the display's density**
  (`DisplayOptions::apply_first_run_display_scale`,
  `crates/clonk-app/src/settings.rs`). Approved 2026-07-28. C++ starts every
  install at `Scale=100` (src/C4Config.cpp:480), which on a 2x panel produces
  an 800x600 *device pixel* window and a 14px font — the setting a player is
  most likely to want changed and least likely to find. Because `Scale`
  divides the physical extent into the logical layout (`logical_size_for`,
  `crates/clonk-scaling/src/lib.rs:12-17`), seeding it from the monitor keeps
  the classic 800x600 logical layout exactly and gives it the panel's real
  pixel density, so the window covers the same physical area it did on a 1x
  display. It applies only when no configuration file exists — every existing
  install and every value the player has ever saved is untouched — and rounds
  to an integer scale because a fractional application scale routes every
  glyph through a bilinear resample of the native atlas
  (`requires_resampling`, `crates/clonk-frontend/src/clonk_fonts.rs:102-113`).
  Presentation-only; pinned by
  `first_run_display_scale_follows_the_monitor_density_only_without_a_config`.

- **The mouse cursor may size itself from the panel's pixel count**
  (`CursorTiers::HighDpi`, `crates/clonk-frontend/src/viewport.rs`; opt-in
  `Graphics.HighDpiCursor`, default off). Approved 2026-07-28.
  `C4GraphicsResource::ReloadResolutionDependentFiles`
  (src/C4GraphicsResource.cpp:468-491) pins the sheet at index 5 for every
  width above 1280 and only steps up with `Graphics.Scale`, so a 4K panel at
  Scale=100 draws a 50px pointer while the shipped 75/100/150/225/338px sheets
  are never loaded. Those eight sizes are authored on an exact 50/1280 ratio,
  so selecting by physical width reproduces C++'s angular size at every
  resolution and each tier stays a 1:1 blit of existing art. Selection is
  presentation-only — the cell size feeds `C4MouseControl`'s draw offsets
  (src/C4MouseControl.cpp:333-344), never a control or a game coordinate — so
  two clients on different tiers stay in lockstep. `Classic` remains the
  default and keeps the C++ ladder byte-for-byte, pinned by
  `cursor_atlas_matches_cpp_scale_selection`; the divergence is pinned by
  `high_dpi_cursor_tiers_climb_the_shipped_ladder_by_physical_width` and
  `high_dpi_cursor_tiers_reach_the_drawn_cursor_cell`.

- **Scenario search uses a live catalog-wide product matcher while retaining
  the C++ matcher as a testable oracle** (`MenuState::apply_enhanced_search`,
  `MenuState::submit_search`; C++
  `C4StartupScenSelDlg::OnSearchBarEnter`/`UpdateList`). Approved 2026-07-28.
  C++ waits for Enter and searches only immediate-folder display titles; Rust
  deliberately searches loaded catalog metadata live, ranks results
  deterministically, and presents ancestor context and recovery feedback.
  `scensel_search_does_not_recurse_into_unopened_folders` and
  `scensel_search_applies_on_submit_case_insensitively` continue to pin the
  oracle path independently of enhanced-search tests. This affects startup
  discovery and presentation only; ordinary activation and simulation state
  are unchanged.
- **Guided missiles turn only while a turn key is held, and key-ups are
  synchronized in both control styles**
  (`planet/System.c4g/EkeGuidedMissile.c`, `planet/System.c4g/EkeSftRelease.c`,
  `LocalControlRegistry::route_keyboard_candidates`,
  `GameApp::dispatch_control_event_for_local_player`; C++
  `C4Game::LocalControlKeyUp`, LegacyClonk 7d43b47 src/C4Game.cpp:3592-3605).
  Approved 2026-07-28. Shipped Eke content latches the RL5B remote-guidance
  turn into the missile's `command` local and only `[Down]`/`[Up]` clears it
  (`EkeReloaded.c4d/Weapons.c4d/RocketLauncher.c4d/Script.c:9-49`), so a tapped
  turn key spins the missile until the player straightens it by hand. Two
  changes make it hold-to-steer:
  1. Two `#appendto` scripts in the port's own `planet/System.c4g` complete the
     release pair the content never had — SF5B forwards
     `ControlLeftReleased`/`ControlRightReleased` to the selected item, and
     RL5B straightens the missile. A release only clears the direction it owns,
     so rolling from `[Left]` onto `[Right]` keeps the newer turn.
  2. The app synchronizes the key-up for classic players too, because C++ routes
     a key-up only for AutoStopControl players and otherwise declines, leaving
     classic control with no `Control*Released` at all. A classic set is the
     *lowest priority* release handler: an AutoStop set still wins the key
     exactly as in C++, and an eventless classic candidate still declines.
  Classic movement is untouched — `C4Object::DirectCom`'s procedure switch has
  no release arm (C4Object.cpp:3405-3556) — so a released direction key still
  leaves the crew walking. What does change beyond the missile: every shipped
  `Control*Released` handler now also fires in classic control (ridden
  Knights/Western horses and coaches stop on release; the Fantasy Icestrike
  ball stops steering; Hazard's lift handler is a `Method=None` no-op), and
  `C4Player::PressedComs` now tracks the physical keys in classic control
  instead of latching set bits forever.
  Pinned by `eke_missile_stops_turning_when_the_turn_key_is_released`,
  `eke_missile_keeps_the_newer_turn_when_the_previous_key_is_released`,
  `eke_missile_down_and_up_still_straighten_the_guided_missile`,
  `classic_release_is_emitted_only_when_no_autostop_set_claims_the_key` and
  `selected_player_classic_control_synchronizes_horizontal_key_release`.

- **Accepted divergence: the repeated-key flag is set on every target,
  including macOS.** C++ decides this per windowing backend, chosen at build
  time. Win32 reads the hardware bit (`!!(lParam & 0x40000000)`,
  `C4Viewport.cpp:89,100`, `C4FullScreen.cpp:59,64`, `C4GuiDialogs.cpp:231,240`);
  X11 passes `false` and `C4Game::DoKeyboardInput` re-derives it from its own
  `PressedKeys` map, but only inside `#ifdef USE_X11` (`C4Game.cpp:2143-2154`);
  **SDL passes a literal `false` for every keydown and keyup**
  (`C4FullScreen.cpp:387-400`) and gets no synthesis, and SDL is the default
  main loop on Apple with `USE_X11` excluded there outright
  (`CMakeLists.txt:198-200`). So the pinned C++ macOS build cannot tell a held
  key from a tapped one.
  `game_app::input::BACKEND_SYNTHESIZES_KEY_REPEAT` is `true` unconditionally,
  which deliberately does **not** reproduce that. The SDL branch is not a rule
  about repeats, it is C++ lacking the information; the port synthesizes the
  flag from its own pressed-key set exactly as the X11 branch does, and that set
  is just as available on macOS. Modelling the absence promotes a host
  preference to gameplay: `C4Game::LocalControlKey` swallows a repeat for
  AutoStopControl players (`C4Game.cpp:3566-3570`) and `C4Player::InCom` raises
  a second identical com to `COM_Double` (`C4Player.cpp:1532-1533`), so with the
  flag unset a *held* direction key manufactures `Control*Double` at the host's
  auto-repeat rate. On stock macOS settings (~417 ms to the first repeat, then
  ~100 ms) that beats the 10-frame `C4DoubleClick` window at the 28 ms tick, so
  roughly half a second of holding Left or Right fires the ClonkMars Jetbelt
  (`Jetbelt.c4d/Script.c:38-41`) and the Eke Airbike Hyperfly boost
  (`Airbike.c4d/Script.c:33,55`), and holding Down arms the `COM_Down_D`
  `DFA_PUSH` ungrab. Reported from play on 2026-08-04 against `0677f3aef`, which
  had made the macOS target SDL-faithful; Linux players on the X11 branch never
  saw it.
  This cannot desync: repeat delivery is local input, upstream of the control
  queue, and the repeat rate is a per-machine setting no lockstep peer can
  observe.
  Pinned by `every_target_reports_the_repeated_key_flag`,
  `autostop_ignores_repeated_physical_keydown_until_release` and
  `app_virtual_keyboard_flings_tutorial05_wood_to_the_right_hill`; the latter
  two press the same key twice, which is exactly what the operating system
  delivers.

- Open gap (found 2026-08-02, not closed): **keyboard identity and delivery
  still differ at platform boundaries.** On X11, C++ explicitly asks XKB for
  group 0, level 0 on both press and release, ignoring the active layout group
  (`C4FullScreen.cpp:227-238`); winit instead derives its modifier-free key from
  the active XKB group. This already happened in winit 0.28's
  `lookup_keysym` path. On macOS, C++ forwards SDL's platform scancode directly
  (`C4FullScreen.cpp:387-400`), whose Cocoa table is not identical to winit's
  physical `KeyCode`: examples include F13/F14/F15 versus
  PrintScreen/ScrollLock/Pause, unidentified JIS/keypad keys, and the ISO grave
  swap. Those differences also predate this migration.

  On Windows, winit 0.30 newly consumes Alt+F4 before application key dispatch;
  winit 0.28 delivered it, while C++ offers `WM_SYSKEYDOWN`/`VK_F4` to
  `DoKeyboardInput` first (`C4FullScreen.cpp:62-71`). Recovering that event
  requires upstream support or native interception and is an accepted migration
  gap. The 0.30 port preserves the existing supported mappings and restores
  raw-VK identity for delivered supported Windows keys; it does not make
  Windows fully exact or close the Linux/macOS gaps. Releases target Windows,
  Linux, and macOS; the pre-existing BSD target/codec inconsistency remains out
  of scope.

- **A catch-up burst reserves wall-clock for drawing, and a repaint floor
  outranks every frame skip** (`RenderFloor`,
  `crates/clonk-app/src/main_parts/app_state.rs`; `RENDER_RESERVE_PERCENT` and
  `MAX_TIME_BETWEEN_RENDERS`, `crates/clonk-app/src/main_parts/assets.rs`;
  C++ `C4Application::Execute`/`Game.DoSkipFrame`, LegacyClonk 7d43b47
  src/C4Application.cpp:463-476). Approved 2026-07-27.
  C++ degrades only by thinning whole *graphics opportunities*, which assumes
  the simulation itself fits its tick. On Pi-class hardware it does not: one
  `advance_simulation_pass` drains the whole clamped 250 ms backlog
  (`MAX_ACCUMULATED_TIME`) without ever returning to the event loop, so the
  window can freeze for the entire burst. The ported `AutoFrameSkip` cannot
  help — it is a one-shot latch consumed at a graphics opportunity that never
  arrives, and `network_control_pacing` returns a no-op offline
  (`crates/clonk-app/src/game_app/network.rs`), so single-player has no lever
  at all.
  Two presentation-only rules are added. `simulation_burst_budget` bounds one
  pass at `(100 - RENDER_RESERVE_PERCENT)/RENDER_RESERVE_PERCENT` times the
  last measured graphics pass, floored at one simulation period and capped at
  the repaint floor; `must_present` forces a repaint after 500 ms (~2 Hz)
  whatever `skip_redraw`, `/fast N` or the automatic latch decided.
  Worked example (arithmetic from the constants, not a measurement — no Pi
  was in the loop): at a Pi-4-like 35 ms per simulation frame and a 10 ms
  graphics pass, one pass previously drained the full 250 ms backlog, i.e. 9
  frames / ~315 ms with no repaint, so the window updated at best ~3 Hz and,
  under `/fast N` or network thinning, arbitrarily less. The budget becomes
  `max(28 ms, 5.67 x 10 ms) = 57 ms`, so a pass runs ~2 frames and yields,
  putting repaints at roughly 14 Hz for about 15% of the CPU.
  Determinism is unaffected: the budget is checked only *after* a frame
  executed, unspent backlog stays in the accumulator, and the same simulation
  frames therefore run in the same order — only spread across more application
  passes. Nothing here is visible to script or to the control stream.
  **The graphics deadline bounds the burst as well**
  (`simulation_burst_budget_before`; added for clonk-org/clonk-rs#159).
  The reservation above is derived from the *last graphics pass*, so it is the
  right bound only while the simulation is the expensive half. When drawing is,
  it inverts: an expensive draw buys the simulation more wall clock, not less.
  Measured on `content/mods/Super_Mega_Ultra_Extrem_Wettlauf.c4s` at a 33 ms
  graphics pass, the reservation was 187 ms of simulation per draw and
  presentation fell to 0.93 FPS — the `must_present` floor — while
  `simulation_fps` held 35.7. C++ cannot reach that state: `C4Application::
  Execute` runs at most one `Game.Execute()` per pass and draws in the same pass
  (C4Application.cpp:451-478), so drawing gets a slot every pass and an
  overloaded machine runs the *game* slow instead. A pass therefore also stops
  once the next graphics opportunity is due, which keeps the catch-up (every
  frame that fits before the draw still coalesces into the one pass) while
  restoring the oracle's ordering. Same determinism argument: still checked only
  after a frame executed.
  Pinned by `a_simulation_burst_yields_to_the_event_loop_once_its_budget_is_spent`,
  `render_floor_reserves_a_share_of_the_wall_clock_for_drawing`,
  `a_simulation_burst_yields_when_the_next_graphics_opportunity_is_due` and
  `render_floor_forces_a_repaint_at_two_hertz_however_deep_the_skip`.
  **Relationship to the frame-counted floor below.** The two were developed
  independently against the same Spring precedent and both survive, because
  they bound different variables. `apply_render_floor` counts 18 *simulation
  frames*, which is 2 Hz only while frames cost their nominal 28 ms; it is the
  deterministic, testable one and it fires on a machine that is keeping up.
  `RenderFloor` bounds *wall time*, which is what actually degrades here: at a
  Pi-4-like 70 ms per frame those same 18 frames are 1.26 s of frozen window,
  and no frame count can see that. `RenderFloor` also bounds the burst itself,
  which the frame-counted floor does not — it only rewrites the decision a
  fully-drained pass already made. Neither can suppress a draw, so having both
  is monotone: they only ever add repaints. The event-loop path zeroes
  `frames_since_redraw` when it forces a repaint, so the frame counter does not
  keep climbing against a screen that did update.

- **Presentation detail is chosen from measured graphics cost instead of
  static config only** (`PresentationDetailGovernor` / `PresentationDetail`,
  `crates/clonk-app/src/main_parts/app_state.rs`; C++ `Graphics.FireParticles`
  and `Graphics.DisableGamma`, LegacyClonk 7d43b47 src/C4Config.cpp).
  Approved 2026-07-27. C++ exposes both as static `[Graphics]` switches the
  player sets once. A Pi needs them chosen for it, because the player cannot
  know in advance which scenario will exceed the budget.
  After `DETAIL_STEP_DOWN_PASSES` (30) consecutive graphics passes over the
  simulation interval the governor drops fire particles — `GraphicsSystem`
  skips the `Fire`/`Fire2` draws belonging to a *burning* object, one
  unbatched call each, which is a real saving now that the engine emits them.
  The predicate stops short of "every `Fire`/`Fire2` particle" because
  shipped content builds standalone flames from the same defs on objects that
  are not alight (the Western falling stars globally, the ClonkMars oil rig's
  `Overburn` flare on itself), and blacking those out is not a detail
  reduction. A particle carries no provenance, so script fire on an object
  that *is* alight goes with the rest of that blaze. Then the monitor-gamma
  resolve pass — a second full-screen fill doing three dependent texture
  fetches per pixel. It restores one step at a time after
  `DETAIL_STEP_UP_PASSES` (120) passes under half the budget; the deadband
  between the two thresholds is what stops it oscillating. It engages only
  while the round's frozen `Graphics.AutoFrameSkip` allows automatic
  degradation, and clearing that flag restores full detail immediately.
  Both steps are presentation-only, so two clients running at different detail
  levels stay in lockstep. That is why the fire rung suppresses the *draw*
  rather than driving `Engine::set_fire_particles`: an engine gate keyed to
  measured frame cost would put wall-clock timing inside `Engine::snapshot()`,
  whose particle list feeds the dev-replay hash. The static
  `Config.Graphics.FireParticles` is a separate signal and does reach the
  engine, where C++ applies it. Pinned by
  `presentation_detail_steps_down_only_on_a_sustained_overrun`,
  `presentation_detail_recovers_only_with_real_headroom` and
  `the_fire_detail_rung_skips_only_a_burning_objects_own_flames`.

- **Framebuffer creation widens the GPU backend set instead of aborting**
  (`build_framebuffer` / `framebuffer_backend_attempts`,
  `crates/clonk-app/src/main_parts/audio.rs`). Approved 2026-07-27. This has no
  C++ counterpart: the oracle uses SDL/OpenGL directly. The port asked wgpu for
  `Backends::PRIMARY` only (VULKAN | METAL | DX12 | BROWSER_WEBGPU — no GL at
  all), chosen because the GL backend probes for libEGL and logs a spurious
  "Unable to open libEGL" on macOS. On a board whose only usable driver is
  GLES that produced no adapter, and `PixelsBuilder::build` then failed out of
  `main` with no retry and no fallback. `PRIMARY` is still tried first, so
  desktop behavior and the macOS log are unchanged; only the failure path is
  new. An explicit `WGPU_BACKEND` is honoured exactly and never widened.
  Pinned by `framebuffer_backends_widen_to_gl_before_giving_up`.

- **The new-player dialog seeds its name from the localized rank ladder**
  (`GameApp::new_player_default_name`, `crates/clonk-app/src/game_app/startup.rs`;
  C++ `C4PlayerInfoCore::Default`, LegacyClonk 7d43b47 src/C4InfoCore.cpp:69).
  Approved 2026-07-27. C++ hardcodes the German `"Neuling"` and prefills it in
  the new-player dialog for English players too, so the very first screen a new
  player sees is German. `"Neuling"` is rank 0 of `IDS_RANKS_PLAYER`, whose
  shipped English ladder starts `"Novice"` (LanguageUS.txt:1280), so the port
  reads the seed from the localized ladder instead. A DE language pack still
  yields exactly `"Neuling"`, so German players are unaffected.
  Presentation only — `Player.txt` still round-trips byte-identically with C++
  in both directions. The omit-if-equal serialization default
  (`live_c4_player.rs`, `push_c4_string(..., "Name", ..., "Neuling", 30)`), the
  missing-`Name=` read fallback (`player_file.rs`, `default_player_name`) and
  the netplay fallback (`configured_client_players.rs`) all remain `"Neuling"`,
  so a profile written by either engine is read identically by the other.
  Pinned by `new_player_dialog_seeds_the_name_from_the_localized_first_player_rank`
  and `new_player_dialog_still_seeds_neuling_from_the_german_rank_ladder`.

- **`Network.ControlMode` defaults to 2 (`CNM_Async`), not 0 (`CNM_Decentral`)**
  (`crates/clonk-app/src/main_parts/resources.rs`,
  `crates/clonk-app/src/advanced_config.rs`; C++ `C4Config.cpp` ships 0 and
  labels async `"[!]Asynchroner Netzwerkmodus (experimentell!)"`,
  `C4GameOptions.cpp:93`). Approved 2026-07-25. The async *mechanism* itself is
  already a faithful port -- `force_expired_async_control` mirrors
  `PackCompleteCtrl` (LegacyClonk 7d43b47 src/C4GameControlNetwork.cpp:741-784)
  and the deadline mirrors :754 exactly; only the default changes.
  In lockstep the host cannot publish tick T until every client's control for T
  arrives, so the slowest link paces the whole session and one bad peer stalls
  everyone. Async bounds that wait at
  `ControlRate * AsyncMaxWait * 1000 / TargetFPS` (106 ms at defaults), then
  packs whichever clients arrived. The absent client's input is dropped, not
  deferred.
  Determinism is unaffected: only the host decides the timeout and broadcasts
  one authoritative aggregate, so every client executes the identical control.
  The straggler's late packet is rejected as stale rather than replayed on a
  later tick (`ControlCoordinator::ingest`, `tick < current_tick`), which is now
  pinned by
  `control_arriving_after_its_tick_was_forced_is_stale_and_never_executes`.
  Measured with `cargo run -p clonk-network --example async_control_mode`, 16
  seeds x 400 ticks, 4 clients with one impaired, *with PreSend active* (that
  pairing is the whole basis for this default -- see the numbers below).
  p99/max shared-tick lateness, decentral -> async:
  250 ms peer 232/281 ms -> 190/206 ms (32 packets dropped);
  +-150 ms jitter peer 278/302 ms -> 233/262 ms (45);
  8 clients with one at 400 ms 346/408 ms -> 311/337 ms (66);
  60 ms/10% loss peer 93/106 ms -> 93/106 ms (0 -- the timeout never fires, so
  enabling it is free where it is not needed).
  The cost is what changed the decision. Without PreSend the same runs drop
  1006-2162 packets; with the envelope estimator delivering most control on
  time, async only fires on genuine outliers and drops 32-66, roughly 0.5-1% of
  ticks -- and most ticks carry no keypress at all, so real input loss is well
  below that. PreSend still does the bulk of the work (mean lateness
  192 -> 63 ms, 216 -> 60 ms, 297 -> 98 ms); async is a tail/hitch fix layered
  on top, not a replacement for it.
  Known residuals: the drop is silent, with no client-side signal that a
  player's input was discarded; and a peer whose latency *consistently* exceeds
  the budget is dropped on nearly every tick (`AsyncMaxWait` 1 dropped on 6490
  of 6400 ticks against a 250 ms peer without PreSend), so the budget must stay
  above ordinary delivery time rather than being tuned down to chase the tail.

- **One chunk in flight per peer while a game is running, thirty in the lobby**
  (`crates/clonk-network/src/resource_catalog.rs`,
  `ResourceCatalog::set_max_loads_per_peer`, narrowed at the game-start
  transition in `session/host_dispatch.rs` and `session/client_loop.rs`; C++
  `C4NetResMaxLoadPerPeerPerFile` = 3 always). Approved 2026-07-27, lobby value
  rescaled 2026-07-29.
  The lobby cap is thirty rather than C++'s three because a chunk here is a tenth
  of `C4NetResChunkSize`: the cap counts chunks, but what it buys is a byte
  window, and 30 x 10 KiB is exactly C++'s 3 x 100 KiB. `RESOURCE_MAX_LOADS` is
  scaled the same way (200 against C++'s 20), preserving C++'s ratio between the
  two. Left at three, the smaller chunk would have divided C++'s
  bandwidth-delay product by ten and held one resource to 30 KiB per round trip
  — 375 KiB/s on an 80 ms link, minutes for a definition pack, which is what a
  joining player experienced as "still loading". The equivalence is pinned by
  `the_lobby_load_caps_hold_the_cpp_byte_window`; the two tests that assert C++'s
  literal 3/20 thresholds now set them explicitly via `set_max_loads_per_peer` /
  `set_max_loads` so they remain true C++ oracles.
  **Not re-measured:** the lobby window now queues the same bytes ahead of
  control as C++'s configuration, so lobby control latency should be the
  `100 KiB x3` row below rather than the `10 KiB x3` row. That is the accepted
  trade — the lobby has no lockstep control to protect — but the figure is
  inferred from equal byte counts, not measured. In-game is unaffected and stays
  at one.
  Until 2026-07-29 the in-game narrowing was a **no-op on the real download
  path**: it was applied to `ClientResourceState::catalog` / `HostState::
  resource_catalog`, but `dispatch_client_resource_packet` schedules through the
  *backend's* catalog whenever a backend exists and only falls back to the bare
  one when there is none. Both sites now also narrow the backend
  (`ResourceTransferBackend::set_max_loads_per_peer`), pinned by
  `narrowing_the_window_reaches_the_scheduling_catalog`.
  This cap, not `C4NetResMaxLoad`, is what governs head-of-line blocking: bulk
  outstanding *on one connection* is this times the chunk size, and the global
  cap only spreads work across different peers, which are different connections.
  Measured through the real reliable-UDP layer at 80 ms / +-20 ms / 2% loss,
  with a chunk pushed down the same ordered stream as control (8 seeds x 300
  ticks, `sim::bulk_stream_tests`). Control latency, mean and worst:
  no bulk at all 49.7 ms / 80 ms;
  100 KiB x3, C++'s configuration, 110.1 ms / 892 ms;
  10 KiB x3, the former lobby value, 63.1 ms / 445 ms;
  10 KiB x1, the in-game value, 53.1 ms / 393 ms.
  So the narrower window recovers most of the remaining gap to an unloaded link.
  It is *not* set to one everywhere, because it also divides transfer throughput
  by three — a peer can only have one chunk in flight per round trip, and on a
  300 ms link that turns a multi-megabyte scenario download into minutes. The
  blocking only costs anything while there is control to block, so the lobby
  keeps C++'s three, where a fast join is the only thing the player is waiting
  for. Purely local request scheduling either way: a serving peer cannot tell how
  many chunks we chose to have outstanding, so this is invisible to a stock C++
  peer.

- **The catch-up test brackets the lookahead the client asked for**
  (`GameApp::network_control_catch_up_limit`,
  `crates/clonk-app/src/game_app/network.rs`; `NETWORK_CONTROL_OVERFLOW_LIMIT`
  stays C++'s 3 and becomes a floor). Approved 2026-08-05, fixes
  clonk-org/clonk-rs#90.
  C++ `CtrlOverflow` tests the ready frontier against the executing tick alone —
  `iControlReady >= iTick + C4ControlOverflowLimit`, limit 3
  (C4GameControlNetwork.h:124) — and `Game.GameGo` then short-circuits the
  application timer for as long as it holds (C4GameControl.cpp:334-342,
  C4Game.cpp:1919). That is a backlog measure only while the lookahead is
  shallow. `CtrlNeeded` submits local control through `getCtrlTick(FrameCounter
  + PreSend)` (C4GameControlNetwork.cpp:147-155), so a client running PreSend
  `p` at ControlRate `r` has *itself* asked for `p / r` ticks beyond the one it
  is about to execute, and the host cannot complete a tick nobody submitted for.
  Once `p / r` reaches 3 the client's own jitter buffer reads as permanent
  backlog and every frame runs unpaced.
  Arithmetic, from the constants: with a delivery delay `D` and a wall-clock
  frame period `fp`, the ready frontier sits `p / r - D / (r * fp)` ticks ahead,
  so the client keeps fast-forwarding until `fp = D / (p - 2r)`. `PreSend` is
  `38 * budget + 1` (C4GameControlNetwork.cpp:437), so at ControlRate 2 the
  threshold is a delivery budget of ~132 ms, and a link that then delivers in
  80 ms settles at 10 ms per frame against the nominal 28 — the round runs at
  ~2.8x speed until the budget decays back under the threshold. That is the
  sporadic fast-forward reported in clonk-org/clonk-rs#90, and one peer on a bad
  link is enough to put *everyone* over the threshold, because the horizon is
  sized from `max(ping, measured lateness)` and the host's wait for a straggler
  is charged to every client's lateness.
  So the port is on the wrong side of this inequality far more often than C++
  is: this port deliberately sizes PreSend from the delivery-time *tail* rather
  than C++'s mean (see the `ControlLatencyEstimator` entry), which is precisely
  a request for a buffer deeper than typical delivery — and C++'s test then
  burns it. On a steady link both engines cross the threshold at the same ping;
  what is new here is how readily the tail-sized horizon gets there. The
  measurement is already on record two entries below: on the impaired chaos
  profiles the median horizon is **~378 ms**, i.e. PreSend at or near the 1..15
  clamp, which is a lookahead of 7 ticks against a limit of 3. One bad peer is
  enough to put a *healthy* client there, because the horizon takes
  `max(ping, measured lateness)` and the host's wait for a straggler is charged
  to every other client's lateness.
  `clonk_network::sim`'s own playout model never executes before
  `CONTROL_PERIOD * tick + lookahead` (`replay_lockstep`, sim.rs:519-543), so
  every frozen-time and input-latency figure recorded for the PreSend divergence
  was measured against a client that *holds* its horizon. This makes the engine
  behave the way those measurements assumed.
  The limit therefore becomes `max(3, p / r + 1)` — the largest ready queue the
  client's own submissions can explain, `+ 1` because `behind` counts the tick
  about to execute. `p / r + 1` is 1 or 2 for every PreSend up to 5 at
  ControlRate 2, so wherever C++'s shallow lookahead applies the number is
  unchanged and every pre-existing catch-up test passes untouched; it relaxes
  only where the buffer is one this client already paid input latency for.
  Substituting the allowance back into the inequality above leaves
  `-D / (r * fp) > 0`, so a *legitimate* horizon can no longer trigger a
  fast-forward at any PreSend, while control the client never submitted for —
  the shipped async mode packs a tick without a straggler, a deactivated client
  submits none at all, a rejoining client starts behind — still does.
  **Blast radius.** Local wall-clock pacing only, in the same family as
  `RenderFloor` and the `RenderInactive` default: it changes *when* this client
  executes a frame, never which frame, in what order, or with what content.
  Nothing on the path reads or writes `C4Fixed`, `C4Random`, movement or control
  ordering, the reported `behind` stays C++'s inclusive `GetBehind` for the F4
  list and the diagnostics overlay, and the decision is per-client local state no
  peer can observe — so a mixed session with a stock LegacyClonk client stays in
  lockstep, and in fact paces that client correctly too, since its speed is
  bounded by our submissions. `parity verify` and `engine-snapshots verify`
  cannot see it; neither runs a network session.
  Pinned by `control_buffered_inside_the_presend_horizon_is_not_a_catch_up_backlog`
  and `a_backlog_beyond_the_presend_horizon_still_catches_up`. Before the change
  the first of those executed 8 frames in a pass where 1 was due.

- **Drawing has a floor while catching up**
  (`crates/clonk-app/src/main_parts/app_state.rs`, `apply_render_floor`;
  `NETWORK_RENDER_FLOOR_FRAMES` = 18). Approved 2026-07-27. No C++ equivalent.
  C++ thins rendering during catch-up by `(behind + 15) / 20`
  (C4GameControl.cpp:334-342), so at a large backlog it draws one frame in
  twenty or worse, and because the port coalesces several simulation frames into
  one pass, consecutive passes can each decide to draw nothing at all. A
  recovering client then shows a completely static picture — the same "is it
  hung?" symptom a silent control stall produces, and the reason
  legacyclonk/LegacyClonk#28 reads the way it does.
  A pass that would draw nothing draws anyway once 18 simulation frames have
  gone by undrawn: 2 Hz at the 28 ms in-game tick, which is the floor Spring
  pins while fast-forwarding rather than giving the simulation everything.
  Counted in frames rather than wall time so the behaviour is deterministic and
  testable. Applied after the pass has decided, so the per-frame accounting that
  mirrors C++ is untouched.
  This covers the case where frames are cheap and the *decision* starves
  drawing. It does not bound wall time, so on hardware slow enough that 18
  frames take over a second it cannot fire soon enough; the wall-clock
  `RenderFloor` above covers that case and also bounds the burst. Both are
  live and neither can suppress a draw — see the cross-reference there.

- **Adaptive `ControlRate` was investigated and rejected — it cannot help a
  CPU-bound client.** Not a divergence; recorded so it is not attempted again.
  Widening the control cadence is the classic Age of Empires answer to a slow
  participant, and it does not apply to this failure. A control tick costs
  `ControlRate` simulation frames *and lasts* `ControlRate` frames, so the rate
  cancels out: a machine whose per-frame cost exceeds the per-frame budget is
  overloaded at every rate. `ControlRate` buys packet rate and jitter tolerance,
  both genuinely useful on a narrow link, but not one millisecond of CPU. The
  only lever that would help is a slower *frame* rate, which slows the game for
  everyone — precisely the outcome the straggler work exists to avoid.
  Pinned by `sim_session::control_rate_tests`. Finding it required fixing a
  fidelity bug in the harness: `control_period` now derives from the rate rather
  than being hardcoded at rate 2's 55 ms, without which a higher rate measured as
  strictly worse because the cost per control tick rose while its budget did not.

- **Reliable-UDP data now uses C++'s one-send policy**
  (`crates/clonk-network/src/udp.rs`, `reliable_udp_redundant_copies`;
  C++ `C4NetIOUDP::Peer::Send` and `SendDirect`, LegacyClonk 7d43b47
  `src/C4NetIO.cpp:2789-2809`, `:3128`). Approved 2026-08-08.
  Every data fragment is sent once and a missing fragment is repaired after a
  `Check`. Immediate byte-identical copies hid random loss on fast links, but
  their UDP/IP framing became positive feedback on a narrow shared uplink: two
  copies of the benchmark control plus its background load already offer more
  than 33.6 kbit/s. Re-ask counts cannot safely distinguish that congestion
  from random wire loss, because the extra copies both create the queue and
  censor the loss signal. Fresh peers and protected/unprotected review windows
  therefore oscillated between overloaded states.

  The deterministic acceptance profile sends one real client `PID_Control`
  every 56 ms through a 33,600 bit/s, 300 ms RTT link with independent 2% loss,
  a 4,200-byte drop-tail queue, 32 charged UDP/IP bytes per datagram and a paced
  20,000 charged-wire-bit/s client upload. It runs 256 warm-up controls and
  2,049 measured controls for each of 20 fixed seeds. Against the checked-in
  medians recorded by the byte-identical pre-change harness, pooled
  client-to-host total-delay p50 falls 878 ms -> 220 ms
  (74.9%) and propagation-excluded p50 falls 728 ms -> 70 ms (90.4%). All
  46,100 controls arrive exactly once and in order with matching payload digest,
  both peers remain `Working`, and no disconnect occurs. Every paired seed also
  clears the 50% total-delay target. The ordinary test
  `single_copy_halves_each_paired_dialup_seed` pins the pooled and per-seed
  thresholds. The ignored `dialup_20_seed_report` emits all 40,980 raw measured
  samples.

  The background is deliberately a paced charged-wire load, not a simulated
  resource-protocol transfer: it shares queue and serialization capacity but
  does not enter reliable packet numbering or repair. Resource head-of-line
  behavior remains covered separately by the bulk-stream tests. Packet numbers,
  delivered bytes, ordering, packet logs and the public API are unchanged; this
  removes a Rust-only physical-send divergence and leaves the existing 250 ms
  re-ask damping in place.

- **The host stops extending the async deadline for a persistent straggler**
  (`crates/clonk-network/src/session/host_loop.rs`, `force_expired_async_control`;
  `HostConfig::straggler_patience`, default 4). Approved 2026-07-27. No C++
  equivalent: C++ has no control-lag drop of any kind, and its only kick is 30 s
  of unanswered ping.
  `CNM_Async` bounds the host's wait *per tick*, which is the right answer for a
  peer that hiccups. It does nothing for a peer that is late on *every* tick — a
  machine that cannot sustain the cadence — because the host then pays the whole
  budget (106 ms at defaults) every single tick and every other participant pays
  it too. Once a client has missed the full budget on four consecutive ticks the
  host stops waiting for it; it rejoins the waited-for set the moment it
  delivers. This is the same move C++ already makes for `NCS_Chasing` clients,
  which `isWaitedFor()` excludes from `AllClientsReady`.
  Determinism is unaffected for the same reason `CNM_Async` itself is: only the
  host decides, and it still broadcasts one authoritative aggregate that every
  participant executes identically.
  Two details that took measurement to get right, both worth preserving:
  lateness is counted **only when the full budget actually expired**, never on a
  fast-path pack, or a client merely in flight when the host gave up on somebody
  else accumulates marks and is eventually written off itself; and the fast path
  fires only when **every** client the coordinator is still missing is a known
  straggler, since "the peer sent it" is not "the host has it".
  Measured with `cargo xtask chaos`, 16 committed seeds x 200 ticks. Healthy
  participants, before -> after, against the ping-sized-PreSend baseline this
  and the entry below replace:
  one Pi-class machine on a good link 975 -> 0 permille blocked and
  10348 -> 394 ms drift;
  the same machine on 33.6k dial-up 970 -> 0 permille and 10384 -> 394 ms;
  a Pi 4 on congested hotel wifi 930 -> 15 permille and 8995 -> 524 ms;
  a dial-up link on a good machine 530 -> 5 permille and 3491 -> 447 ms.
  Every impaired profile now costs the healthy players about what an all-healthy
  session does (368 ms).
  The cost is the straggler's input, and it is small: at patience 4 the healthy
  participants lose 38 inputs out of 4800 and the straggler 1.6% more than
  before, while an all-healthy session is completely unaffected (65 dropped, the
  same as with the feature off). Patience 2 buys the same drift but starts
  costing an all-healthy session input (65 -> 120), because ordinary loss makes a
  good client miss twice in a row often enough to be written off.
  What this does **not** fix: the straggler's own experience. It is CPU-bound, so
  its drift is unchanged at 20.5 s over an 11 s session. Nothing in the network
  layer can help a machine that cannot execute ticks fast enough; that needs
  adaptive `ControlRate` or a smaller scenario.

- **Published resources advertise 10 KiB chunks, not 100 KiB**
  (`crates/clonk-network/src/host_resource_core.rs`, `STOCK_CHUNK_SIZE`; C++
  `C4NetResChunkSize`, LegacyClonk 7d43b47 src/C4Network2Res.h:27). Approved
  2026-07-27. OpenClonk's value; LegacyClonk raised it to 100 KiB in `2557ff3d`
  to "better utilize available upload speed".
  Chunk size is carried per resource in the core and honoured by whoever
  downloads it, so this is a local publishing choice rather than a protocol
  change and a stock C++ peer follows it unmodified.
  The reason it is worth losing transfer throughput for: resource chunks and
  control share one **strictly-ordered** reliable-UDP sequence space whenever a
  peer has no TCP route, which is the ordinary internet-play topology because NAT
  punch-through is UDP-only and `GetDataConnection` falls back to the message
  connection. At the 499-byte payload limit a 100 KiB chunk is **206 datagrams**,
  so it puts 206 sequence numbers ahead of every later control packet, and one
  lost fragment withholds all of them from the game loop until the repair lands —
  which proceeds at ten fragment asks per check packet. Three concurrent chunks
  to one peer queue 618 fragments ahead of control. 10 KiB is 21 datagrams,
  cutting that head-of-line window by an order of magnitude.
  **This divergence delivered none of that until 2026-07-29**, and cost a
  ten-times *slowdown* instead. `ResourceFileStore::read_chunk` derived the chunk
  offset from the core's chunk size but capped the chunk *length* with the
  hardcoded 100 KiB literal — a faithful copy of
  `src/C4Network2Res.cpp:1268-1269`, which is self-consistent in C++ only because
  every core C++ publishes carries `ChunkSize = C4NetResChunkSize`
  (`src/C4Network2Res.cpp:81`, `:89`). Against a 10 KiB core each chunk therefore
  overlapped the following nine: the host served roughly ten times the file to
  deliver it once, the reliable-UDP burst stayed 206 fragments so the head-of-line
  window was never actually reduced, and the client credited one chunk per
  response. Fixed by sizing from the core's own stride, which is identical for
  every core C++ can publish and also stops wasting a stock C++ client's
  bandwidth. Pinned by `serving_every_chunk_moves_the_file_exactly_once` and
  `cpp_chunk_reads_offset_and_size_by_the_core_chunk_size`, which asserts the two
  forms still coincide at C++'s own chunk size.
  `RESOURCE_MAX_LOADS` is scaled with the chunk size (see the per-peer entry
  above), so the maximum outstanding bulk stays C++'s 2 MB.
  The reliable-UDP one-send policy does not help here: repair recovers *loss*,
  but it cannot move control ahead of bulk already queued on the same ordered
  packet-number stream.
  Scope: the stock size applies only once a core becomes loadable. C++ decodes an
  unloadable core by substituting its compiled-in defaults for size, CRC and
  chunk size alike, so a custom value could not round-trip there and would mean
  nothing if it did.
  Not changed, having been examined: the control loop still blocks on a pending
  player-file resource (`crates/clonk-app/src/game_app/sound.rs:649-687`). That
  wait is a correctness requirement — the tick carries a `JoinPlayer` that needs
  the file — and unlike a plain control stall it is already surfaced to the
  player through `begin_blocking_resource_wait_at` ("player file for %s").
  `RESOURCE_MAX_LOADS` was likewise left at C++'s 20 rather than OpenClonk's 5,
  because the swarm behaviour is pinned by tests against C++ and the chunk-size
  change already carries the benefit.

- **PreSend is sized from measured control lateness, not from ping alone**
  (`crates/clonk-app-netplay/src/network.rs`, `observe_control_lateness_ms` and
  `update_control_presend`; C++ `C4GameControlNetwork::CalcPerformance`,
  LegacyClonk 7d43b47 src/C4GameControlNetwork.cpp:404-430). Approved
  2026-07-27.
  C++ derives the horizon from `pConn->getPingTime()` and nothing else, and
  `iTargetFPS` is a hardcoded 38 rather than a measurement. A client that is
  slow rather than *distant* — a weak machine, a saturated uplink queue, a host
  that waited on somebody else — therefore never buys itself any headroom, and
  its input misses the async deadline on essentially every tick and is dropped
  silently. C++ already computes the right quantity in the same function,
  `AddPerf(pCtrl->getTime() - iWaitStart)`, and spends it only on the F7 "wait
  N ms" display string.
  The port keeps that ping sample and takes `max(ping, measured lateness)` for
  the PreSend decision, where lateness is the interval from reaching the control
  tick to consuming it — arrival against the cadence, the same quantity the host
  records as `ClientPerformanceStats::wait_ms`. Taking the maximum rather than
  replacing means a punctual client keeps exactly C++'s horizon, so the extra
  input latency is charged only where it buys something. The script- and
  dialog-visible `avg_control_send_time` (ACT) remains C++'s exact ping-derived
  1/150 EWMA.
  Determinism is untouched for the same reason as the entry below: PreSend
  selects only which tick a client stamps its *own* input for. Every participant
  still executes that tick at that tick, the wire format is unchanged, and
  PreSend already varies per client in C++.
  Measured with `cargo xtask chaos run --presend ping|measured`, 16 committed
  seeds x 200 ticks x 6 profiles. Healthy-participant blocked ticks and drift,
  ping -> measured:
  four good machines 125 -> 0 permille and 919 -> 368 ms;
  one dial-up link on a good machine 530 -> 10 permille and 3491 -> 529 ms;
  a Pi 4 on congested hotel wifi 930 -> 440 permille and 8995 -> 2874 ms.
  The cost is input latency: the median horizon rises from 78 ms to 226 ms on
  the healthy profile and to ~378 ms on the impaired ones, the latter close to
  the 1..15 frame clamp. AoE's 0-250 ms "unnoticeable" band therefore covers the
  healthy case but not the worst ones.
  Known limit, deliberately not papered over: this does **not** rescue a machine
  that simply cannot execute ticks fast enough. At `K_sim` 20 a control tick
  costs 156 ms against a 55 ms period, so no lookahead can help; the healthy
  players still lose 10 s over an 11 s session because the host pays the full
  `AsyncMaxWait` budget on every tick before giving up on it. That residual
  needs adaptive `ControlRate` or demotion, not a wider horizon.

- **PreSend is sized from the delivery-time envelope, not the mean**
  (`crates/clonk-network/src/control_latency.rs`, `ControlLatencyEstimator`;
  C++ `C4GameControlNetwork::CalcPerformance`, LegacyClonk 7d43b47
  src/C4GameControlNetwork.cpp:382-447). Approved 2026-07-24.
  C++ derives the PreSend horizon from a 1/150 EWMA of the *mean* control send
  time. Two consequences, both of which stall every participant rather than the
  one slow peer: the mean sits below the delivery times of roughly half of all
  control packets, so a link with any jitter stalls on about half its ticks
  forever; and the 150-sample time constant is ~8 s at ControlRate 2, so a link
  that gets slower stalls on *every* tick for that whole span before the horizon
  reacts. The port instead tracks a decaying peak envelope (immediate attack,
  C++'s slow decay) plus a mean-absolute-deviation margin over upward surprises
  only. On a steady link the deviation collapses and the envelope equals the
  mean, so the budget converges on exactly C++'s value and healthy connections
  are unaffected.
  The script- and dialog-visible `avg_control_send_time` (ACT) still uses C++'s
  exact 1/150 EWMA; only the PreSend decision reads the new budget.
  Determinism is untouched: PreSend selects which tick a client stamps its *own*
  input for. Every participant still executes that tick's control at that tick,
  the wire format and the delivered control stream are unchanged, and PreSend
  already varies per client in C++ (each adapts from its own ping, and
  `SetPreSend` is script-settable), so a C++ peer needs no knowledge of this.
  Measured with `cargo run -p clonk-network --example link_impairment`
  (`LC_PRESEND=cpp` vs `adaptive`, `LC_DUP=2`), 24 seeds x 400 control ticks
  (a 22 s session), paired with the historical redundancy experiment below.
  `LC_DUP=2` is an explicit harness setting and is not the current runtime
  policy. Frozen time and the worst single hitch, C++ -> port:
  80 ms RTT / +-20 ms jitter / 1% loss, 27.19% -> 0.18% and 231 ms -> 31 ms;
  150 ms / +-40 ms / 3%, 82.06% -> 0.81% and 502 ms -> 119 ms;
  40 ms / +-8 ms / 0.5%, 6.47% -> 0.02% and 101 ms -> 4 ms.
  The cost is input latency, and it is charged only where it buys something:
  mean horizon 57 ms -> 81 ms on the typical link, 28 ms -> 52 ms on the good
  one. The deviation weight is 1 by measurement, not by analogy: RFC 6298's 4
  bought 0.02 percentage points of frozen time and charged 65% more latency.
  The two tests that pinned the mean-only ramp now pin the ACT average and the
  1..15 clamp, which remain C++'s, and two new tests pin the divergent sizing.

- **One datagram may hold the reliable-UDP hub for at most 2 ms**
  (`crates/clonk-network/src/udp_runtime.rs`, `RELIABLE_UDP_SEND_BUDGET`;
  C++ `C4NetIOSimpleUDP::Send`, LegacyClonk 7d43b47 src/C4NetIO.cpp:1772-1790).
  Approved 2026-07-25. This restores C++'s failure mode, with one bounded
  softening.
  C++ issues a single non-blocking `sendto` and, on EWOULDBLOCK/EINPROGRESS,
  resets the error and reports success: the datagram is dropped and the
  reliable layer repairs it for that one peer. The port awaited writability
  instead, and because a single hub task owns the UDP socket for *every* peer,
  suspending there held up control delivery to all of them behind whichever
  peer was congested -- one bad uplink stalled the entire session. That is a
  port artifact, not C++ behavior, and it mattered more while control datagrams
  went out redundantly.
  The divergence is only that C++ drops at the *first* sign of back-pressure
  while the port allows 2 ms first, which is strictly more conservative (it
  drops less) and still bounds the hub. A writable socket never reaches the
  budget: `send_to` completes on its first poll and the timer is dropped
  unfired, so the healthy path is unchanged. A timeout is reported as a
  successful send, exactly like C++'s `return true`, so it cannot be mistaken
  for an unreachable peer and tear the connection down.
  `try_send_to` is not a valid expression of this and was tried and reverted:
  tokio's `try_*` does not register interest, so it answers WouldBlock until
  readiness has been established and silently drops every early datagram (it
  made 6 `udp_runtime`/`udp_session` tests time out).

- **Immediate reliable-UDP redundancy was measured, then reverted for narrow
  shared links.** Historical divergence, active from 2026-07-24 through
  2026-08-08. Sending three byte-identical control datagrams reduced repair
  stalls on otherwise uncongested 5-10% independent-loss links, and peers safely
  discarded the duplicates by packet number. It was not free: each physical
  copy carried another UDP/IP frame, and the later per-peer re-ask controller
  could only observe losses that survived every active copy. On a 33.6 kbit/s
  link shared with an upload, the controller therefore used congestion as its
  loss signal while its own copies created that congestion. The current
  single-send entry above supersedes this experiment. The measured fast-link
  benefit remains useful evidence for a future capacity-aware scheduler, but a
  re-ask-only controller must not restore it.

- **Reliable-UDP re-ask damping, 1 s -> 250 ms** (`crates/clonk-network/src/udp.rs`,
  `RELIABLE_UDP_RECHECK_INTERVAL`; C++ `C4NetIOUDP::Peer::iReCheckInterval`,
  LegacyClonk 7d43b47 src/C4NetIO.cpp:1914). Approved 2026-07-24.
  The first repair request is immediate in both engines, so this interval only
  governs the case where a repair request is itself lost. C++ then waits a full
  second, which in a lockstep session freezes every participant rather than only
  the peer that dropped a datagram.
  Measured with `cargo run -p clonk-network --example link_impairment` at 60 ms
  RTT, +0..20 ms jitter, 400 control packets: at 2% loss the two intervals are
  identical (44.50 ms mean, 171 ms p99); at 5% loss p99 falls 1.009 s -> 352 ms
  and the worst case 1.229 s -> 462 ms, for about 7% more datagrams.
  Simulation state cannot observe this: only the timing of a repeated repair
  request changes, while the delivered packet stream, its ordering and the wire
  format are untouched, and a C++ peer answers the extra asks unchanged.
  The three tests that pinned the one-second constant now pin the new interval
  and name the C++ line they depart from; the damping *shape* they cover
  (quiet inside the window, strictly higher holes continue immediately, the
  first ask's deadline survives continuations) remains C++'s and is still
  asserted.

- **Restarting a network round returns every client to the lobby**
  (`clonk-network/src/host_restart.rs` PID `0x71`,
  `GameApp::announce_network_round_restart`,
  `GameApp::begin_pending_host_rejoin`,
  `GameApp::poll_pending_host_rejoin`; C++ `C4Application::QuitGame`
  src/C4Application.cpp:373-405, `C4Network2::Clear` src/C4Network2.cpp:748-796,
  `C4Network2::OnClientDisconnect` src/C4Network2.cpp:1802-1834). Approved
  2026-07-29. A restart re-hosts from scratch — C++ backs up only
  `NetworkActive` and the password, runs `Game.Clear()` (which closes `NetIO`
  and drops every connection), then re-enters `Game::Init` with `fLobby` set.
  Clients therefore observe nothing but a closed socket, which
  `OnClientDisconnect` reads as a dead host: it records `NR_NetError` and calls
  `Clear()`, so each client is left alone in the abandoned round with
  `ChangeToLocal` while the host's new lobby comes up empty. Nothing in C++ can
  distinguish this from a crash — there is no restart packet, and `C4Game::Clear`
  zeroes `DirectJoinAddress` (src/C4Game.cpp:648-651), so a native client can
  only rejoin by hand. The port states the intent on the wire instead: the host
  broadcasts `PID_PortHostRestarting` before tearing down, and a client that
  receives it leaves the round and reconnects to the address it already joined,
  retrying once a second across the host's re-bind window (30 s by default,
  carried in the packet and clamped locally to 120 s) before falling back to the
  ordinary join failure. The reconnect repeats the *same join* — the retained
  `ClientSettings`, so the password, netpuncher brokerage and full route list
  survive — rather than rebuilding one from config. This is a
  lobby/session-lifecycle change only: the notice never enters the control
  queue, so control ticks, `RandomCount` and simulation state are untouched.
  **Note on the port-only ID range:** the rationale in
  `crates/clonk-network/src/capabilities.rs` — that C++ silently ignores an
  unknown packet ID — is wrong. `C4IDPacket::CompileFunc` `excCorrupt`s on an ID
  with no `FnUnpack` and `C4Network2IO::HandlePacket` catches that and closes
  the connection in a release build (src/C4Network2IO.cpp:820-834,
  src/C4Packet2.cpp:210-217). That is harmless for *this* packet, because the
  host sends it immediately before closing the session anyway, so a C++ client
  loses the connection it was about to lose and keeps native behavior. It is not
  a general licence, and `capabilities.rs`'s own claim should be revisited.
  Because a relayed `PID_FwdReq` reaches its target on the host's route and is
  indistinguishable from a host-authored packet, the host refuses to relay the
  whole `0x7x` port-only range
  (`a_client_cannot_forge_a_restart_notice_through_the_forward_relay`).
  Without the notice the client path is byte-identical to today's, pinned by
  `client_host_socket_loss_continues_the_running_round_locally`.

- **Birds fly a steered heading instead of re-rolling a pure-axis ComDir once
  a second** (`planet/System.c4g/BirdFlight.c`, `#appendto BIRD`; departs from
  `content/Objects.c4d/Animals.c4d/Bird.c4d/Script.c:25-91,240-284`).
  Approved 2026-07-30. The shipped bird's whole steering policy is four
  independent coin flips per `Activity`, and `Activity` runs on the default
  35-frame `TimerCall` (`C4Def.cpp:298`) because `DefCore.txt` sets no
  `Timer=`. Every decision snaps ComDir to a pure axis — `COMD_Up`/`COMD_Down`
  to climb, `COMD_Left`/`COMD_Right` to turn — so the bird never uses any of
  the four diagonal ComDirs. Against DFA_FLOAT's actuation model that is a
  decision period shorter than the control horizon: `FLOAT_ACCEL` is
  `FIXED100(10)` and the per-axis bound is `FIXED100(Float)` = 2.0 px/frame
  (`C4Object.cpp:5268-5286`), so the bird needs 21 frames to reach terminal
  and 41 to reverse an axis, and `COMD_Stop` has no deceleration case at all.
  The result is a permanently mid-transient sawtooth. The append keeps a
  heading and flies it with per-frame `SetXDir`/`SetYDir` writes at precision
  100; every write stays inside the clamp, so the script is the sole velocity
  authority and the engine adds nothing. On top of that it adds what the
  shipped script has no form of at all: `PathFree` terrain feelers, separation
  and weak alignment against nearby birds, a startle response to crew, and a
  flap-glide speed cycle. It also fixes a shipped copy-paste defect —
  `ContactRight` is a verbatim copy of `ContactLeft`, so its
  `COMD_Right + Random(2)*2-1` yields `COMD_UpRight` or `COMD_DownRight`
  (`COMD_Right` is 3) and steers back into the wall that raised the callback,
  on the one-in-five `!Random(5)` branch.
  **Blast radius.** `parity verify` and `engine-snapshots verify` cannot see
  this — neither executes content C4Script (the golden is 31 engine-primitive
  sections; `SNAPSHOT_SCENARIOS` is synthetic) — and the `tutorial01` replay
  goldens run a scenario with no `[Animals]` section. What does change is the
  RNG ledger *position* in the ~15 bird-bearing scenarios: the append
  reproduces all ten shipped draw sites in the shipped order under the shipped
  conditions and adds none of its own (per-bird variation comes from
  `ObjectNumber()`, not the synchronized stream), but `Activity`'s branches
  read world state the controller changes, so `RandomCount` drifts once the
  flight path does. Scenario *init* is unaffected — the controller installs
  itself lazily from `Survive`/`Activity` rather than from `Initialize`, so
  worldgen and animal placement stay draw-for-draw identical. Cross-play
  against a stock LegacyClonk client desyncs in any scenario containing a
  bird; `planet/System.c4g` is the port's own engine data and never reaches
  one. `ActMap.txt`, `DefCore.txt`, the possession block and the
  bait/nest/reproduction logic are untouched, and every AI entry point keeps
  its `GetEffect("PossessionSpell")` guard.
  Pinned by `bird_flight_controller_installs_itself_on_every_placed_bird`,
  `bird_heading_turns_continuously_instead_of_snapping_to_an_axis`,
  `bird_velocity_stays_inside_the_float_physical_clamp`,
  `bird_flight_controller_adds_no_draw_site_and_leaves_scenario_init_untouched`,
  `bird_flight_is_reproducible_from_a_fixed_seed`,
  `contact_right_reflects_away_from_the_wall_instead_of_back_into_it`,
  `birds_startle_and_flee_when_a_clonk_comes_close` and
  `birds_separate_from_neighbours_that_start_on_top_of_each_other`.

## Preserve

Preserve fixed-point sync boundaries, shared RNG state/count, reverse
execution, IDs and list ordering, callback/effect timing, movement rules,
script include order, save state, masks, scenario-init RNG, and crew/find ties.
