# C++↔Rust primitive differential harness

This harness verifies that the Rust port (`crates/clonk-engine`) reproduces the
determinism-critical C++ primitives **bit-for-bit**. It is a true *differential*
against the C++ source snapshot pinned at
`7d43b47b7d789b533f32d005e64596e0a07019cd` in this repository's Git history,
not a Rust-vs-Rust regression check (that is
`cargo xtask engine-snapshots verify`).

The harness is the required, reproducible oracle for bounded behavior that can
be extracted from C++ without booting a complete scenario. The live
full-scenario complement is documented in [`bridge/README.md`](bridge/README.md).

## What it covers

The golden (`golden/parity_golden.json`) is generated from the **real engine
code** and the Rust side runs identical inputs and asserts byte-exact equality.
The keys in that file are the authoritative section inventory; the table below
highlights the highest-coupling sections rather than duplicating that evolving
inventory in prose:

| Section | C++ source of truth | Why it matters |
|---|---|---|
| `itofix` / `fixtoi` | `src/Fixed.h` C4Fixed | velocity/gravity precision (`SetXDir`, `FIXED256`) |
| `arith` | `src/Fixed.h` operators | velocity scaling, force redirection |
| `trig` | `src/Fixed.h` + `src/Fixed.cpp` `SineTable` | rotation, `SimFlight` |
| `rng_random` | `src/C4Random.h` LCG | network sync (`RandomHold`/`RandomCount`, incl. range-0) |
| `rng_randomize3` | `src/C4Random.cpp` `FRndBuf3` | mass-mover / `Rnd3` |
| `dig2object_rng` | complete `C4Object::DigOutMaterialCast` body | Dig2Object shape-bottom spawn and `Random(360)` plus the next 20 ledger draws |
| `material_corrode_rng` | `src/C4Material.cpp` corrosion branches | material reaction execution RNG ordering |
| `mass_mover_transfer_rng` | `src/C4MassMover.cpp` transfer calls | `Random(10)` before `Rnd3()` immediate-execution decision |
| `script_value_hash` | `src/C4Value.cpp` `hashCombine` / `std::hash<C4Value>` | map-key lookup for nested script values |
| `script_value_convert` | `src/C4Value.cpp:488-598` `C4ScriptCnvMap` + `ConvertTo` | type-coercion rules for `getInt`/`getStr`/… and parameter marshaling |
| `script_killer` | `src/C4ScriptKiller.h`, called by `src/C4Script.cpp:1333-1347` | GetKiller/SetKiller fallback target, player validation, direct assignment, foreign/arrow targeting |
| `eval_direct_exec_context` | complete `FnEval` + `C4AulScript::DirectExec` scope setup (`src/C4Script.cpp:4501-4513`; `src/C4AulExec.cpp:1674-1683`) | object-definition, definition-only, and `Game.Script` receiver selection; object `LocalNamed`, temporary Def, parent, and caller strictness |
| `effect_check` / `effect_execute` | complete `C4Effect::Check` and `C4Effect::Execute` (`src/C4Effect.cpp:271-363`) | AddTo negotiation, priority-1 bypass, upper-effect cycling, timer cadence, kill decisions, and dead-node unlinking |
| `effect_lifecycle` | complete `C4Effect` constructor, `GetCallbackScript`, `ClearPointers`, `Kill`, `ClearAll`, `DoDamage`, `TempRemoveUpperEffects`, and `TempReaddUpperEffects`, composed with the extracted `Check`, `Execute`, and `DoCall` bodies (`src/C4Effect.cpp:31-469`) | exact Start/Timer/Effect/Add/Damage/Stop arguments and receiver, number/order/priority/time state, object/global mutation, callback replacement/addition, false/error results, and synchronized RNG |
| `effect_callback_conversion` | `C4Effect::Execute` / `DoCall` callback entry plus `C4AulScriptFunc::Exec` conversion (`src/C4Effect.cpp:319-456`; `src/C4AulExec.cpp:1364-1397,1610-1656`) | warning-only pre-strict3 conversion versus strict3 rejection for Timer and custom EffectCall callbacks |
| `definition_commanded_effect_position` | complete `C4Effect::Execute` + `C4AulScriptFunc::Exec` forwarding + `C4AulExec::Exec` script-context setup + `FnGetX`/`FnGetY` (`src/C4Effect.cpp:319-363`; `src/C4AulExec.cpp:330-364,1638-1649`; `src/C4Script.cpp:1198-1202,1293-1297`) | an ID-commanded effect keeps its affected carrier argument while implicit position calls see the null command-object receiver |
| `landscape_path` | `src/C4LandscapePath.h`, called by `src/C4Landscape.cpp:890-915` | 17×15 PixCnt traversal and authoritative pixel-plane occupancy at cell edges |
| `action_direction` | `src/C4ActionDirection.h`, called by `C4Object::ExecAction`/`SetDir` | raw-C4Fixed facing, TurnAction fixed-position resync, and stale pre-transition phase ordering |
| `action_swim_direction` | `src/C4ActionDirection.h`, called by DFA_SWIM/`SetDir` | SwimAccel facing changes, TurnAction two-axis fixed-position resync, and stale Swim phase ordering |
| `action_push_pull_fight_direction` | mechanically extracted `C4Object.cpp` DFA_PUSH/DFA_PULL/DFA_FIGHT direction blocks | raw sub-pixel Push/Pull facing, target-relative Fight facing, TurnAction dispatch, and zero `SetDir` calls at equal x |
| `action_callbacks` | `src/C4ActionCallbacks.h`, called by `C4Object::SetAction` | synchronous callback count and Start-before-End/Abort ordering |
| `connect_missing_target_removal` | mechanically extracted `C4Object.cpp` DFA_CONNECT missing-target branch | `LineBreak(true)` before `AssignRemoval`/`Destruction`, with final deleted status |
| `connect_geometry_break_removal` | mechanically extracted `C4Shape::LineConnect` vertex guard + later DFA_CONNECT break branch | zero-argument `LineBreak()` before the same removal lifecycle |
| `line_connect_routing` | the complete `C4Shape::LineConnect` body past that guard, plus `InsertVertex`, `PathFree`/`PathFreeIgnoreVehicle` and `ForLine` (`src/C4Shape.cpp:273-341`; `src/C4Landscape.cpp:1670-1720,2034-2053`) | endpoint move, three-range bend search, old-endpoint fallback across vehicle material, the index-vs-density rule that decides whether a line can break at all, and the ordered vertex list |
| `solid_mask_graphics` | `src/C4SolidMaskBitmap.h`, called by `C4SolidMask` | active/default graphics selection and transparent/solid mask sampling after `SetGraphics` |
| `shake_objects` | complete `C4Game::ShakeObjects` + `C4Object::Fling` bodies | master-order gates, `Random(3)`/`Rnd3()` consumption, attachment material identity, and raw Fling fallback |
| `blast_free` | complete `C4Landscape::ClearPix`, `BlastFreePix`, and `BlastFree` bodies | exact circle scan, pre-mutation material counts, duplicate-slot BlastShiftTo/DefaultMatTex byte selection, IFT preservation, and RNG order |
| `network_rule_goal_placement` | complete `C4SGame::ConvertGoals` and `C4Game::InitRules`/`InitGoals` bodies (`src/C4Scenario.cpp:506-556`; `src/C4Game.cpp:4056-4076`) | HarpoonRace's authored RVLR plus default energy realism becomes authoritative RVLR+ENRG parameters; rules use `max(count, 1)`, goals use the exact count, and local scenario lists cannot replace synchronized JoinData lists |
| `player_join_capacity` | complete `C4PlayerList::GetCount` plus the mechanically extracted capacity block from `C4PlayerList::Join` (`src/C4PlayerList.cpp:172-178,288-294`) | all linked players count, zero is a closed limit, one remaining slot admits exactly one named player, and rejection leaves the ordered roster unchanged |
| `c4group_sort` | complete `C4Group::Sort` and `SortByList` through the linked `src/C4Group.cpp` | descending rank order, unlisted names sinking below listed ones, the case-insensitive tie-break, and the two ways C++ reaches "no sort at all" |
| `scenario_sections` | complete `C4GameSave::SaveScenarioSections` plus the `C4ScenarioSection` constructor and accessors that build the list it walks (`src/C4GameSave.cpp:111-137`; `src/C4Scenario.cpp:555-566,649-657`) | the exact-save section sweep runs in **reverse** construction order, deletes the current section without re-adding it, and discards `Add`'s result |
| `contents_list_order` | complete `C4ObjectList::Add`, `Remove`, `GetLink`, `RemoveLink`, `InsertLink`, and `ShiftContents` bodies (`src/C4ObjectList.cpp:110-268,310-318,614-636,815-831`) | exact category/id insertion, StaticBack/line/unsorted exceptions, tail-add, link-preserving rotation, fresh-link reinsertion, and iterator repair |
| `container_lifecycle` | complete `C4Object::Enter`, `Exit`, and `Collect` bodies (`src/C4Object.cpp:1532-1637,5693-5717`) | callback and reentrant mutation order, both containment directions, controller transfer, final status, and raw fixed motion |
| `breath_refill_callback_order` | mechanically extracted breathable-supply block (`src/C4Object.cpp:915-919`) | Goldwipfcaves' raw `physical=-2009260032`, `state=2147483647` pair both without its missing DeepBreath callback (pinning the overflowing final `+=`) and with a state-mutating probe (pinning the condition and callback-before-add ordering) |
| `set_graphics_missing_lookup` | complete `C4DefGraphics::Get`, base `C4Object::SetGraphics`, and `FnSetGraphics` bodies (`src/C4DefGraphics.cpp:221-229`; `src/C4Object.cpp:5894-5910`; `src/C4Script.cpp:4372-4442`) | a missing named base or overlay graphic returns false without changing an already-selected known named graphic or overlay |
| `contact_action_bottom_flight` | complete bottom `DFA_FLIGHT` arm of `C4Object::ContactAction` + action helpers | the `(OCF_HitSpeed4 \|\| fDisabled)` FlatUp gate, including low-speed disabled actions |
| `contact_action_top_side_flight` | complete top/left/right `DFA_FLIGHT` arms + action helpers + unresolved-flight tail | the `(OCF_HitSpeed3 \|\| fDisabled)` Tumble gates, exact transient wall kicks, enabled Hangle/Scale controls, and final slide-free state |
| `movement` | `src/C4Movement.cpp:260,627` accumulation | the Theme-C core: `fix += dir`, `ydir += gravity` |
| `shape_contact_check` / `target_bounds` | complete `C4Shape::ContactCheck` and `C4Object::TargetBounds` bodies | ordered vertex masks/materials and closed-border MCVehic rules before movement response |
| `contact_vtx_helpers` | complete `ContactVtxCNAT`, `ContactVtxWeight`, and `ContactVtxFriction` bodies | authored vertex ordering, including skipping a contacted zero-weight centre vertex |
| `do_movement_collision_matrix` | mechanically extracted unattached translation block plus `ContactCheck`, bounds, redirection, and friction helpers (`src/C4Movement.cpp:42-95,128-213,254-322`) | every pixel candidate and raw fixed state, both axes/directions, bounds and open/closed borders, multi-contact aggregation/friction, callbacks/RNG, and a live Rust solid-mask wall's MCVehic density |
| `do_movement_rotation_matrix` | mechanically extracted rotation block plus complete `C4Shape::Rotate` (`src/C4Movement.cpp:372-436`; `src/C4Shape.cpp:41-101`) | absolute-shape one-degree turns, rotation limits, contact rollback, rdir redirection, and raw `fix_r` accumulation |
| `do_movement_contact_action_handoff` | `src/C4Movement.cpp:467-472` handoff plus the exact bottom `DFA_FLIGHT` arm | the aggregate contact mask replaces the last-probe mask and changes Flight to Walk with bottom precedence |

Each golden section is deliberately bounded to the named function bodies and
fixtures. Cross-system behavior must be checked with the live `RustEngineBridge`
shadow diff. The remaining direct attached-movement differential is tracked by
clonk-org/clonk-rs#516, and live landscape comparison by
clonk-org/clonk-rs#1240.

## Accepted safety divergences

- **PXS `SyncClearance` gap compaction:** C++ copies a surviving chunk pointer
  downward without clearing the moved-from slot (C4PXS.cpp:406-424). If an
  empty lower chunk precedes a live one, two slots alias the same allocation;
  subsequent execution can process it twice and cleanup can `delete[]` it
  twice. Rust intentionally transfers unique ownership and clears the tail.
  Golden or live-shadow equality is therefore not expected at this undefined-
  behavior boundary; Rust's single-copy survivor order is authoritative.
- **S2 map-generator terminal parameters:** a negative Mandel alpha becomes a
  huge `uint32_t` iteration budget in C++, Gradient with `Wdt=0` performs
  integer division by zero, and Random with `alpha=-2` performs remainder by
  zero (C4MapCreatorS2.cpp:1357-1361,1422-1447). Rust bounds negative Mandel
  alpha to ten iterations, substitutes a denominator of one for Gradient's
  zero width, and returns false from Random's raw algorithm (before normal
  overlay inversion). These inputs are excluded from C++ differential runs.
  Mandel zero width or height is not excluded: its floating division is
  emulated with the same IEEE-754 inf/NaN propagation, and safe parameters
  remain formula-identical.
- **Scenario-section discovery order:** C++ builds one `C4ScenarioSection` per
  discovered `Sect*.c4g` in `C4Group` entry order and prepends each
  (C4Game.cpp:3325; C4Scenario.cpp:557-566), so its list is reverse entry
  order — stored order for a packed group, host `readdir` order for an open
  folder. Rust sorts the discovered names case-insensitively before building
  the same reversed list, so the order is host-independent. This is kept rather
  than matched, because the ordering effect that matters most is port-only:
  Rust re-serializes each modified section against one shared string table as
  it walks the list, so the walk order decides which section's values receive
  which `S<n>` ID, and C++ has nothing to match there — it adds temp files
  written at section-switch time instead of re-serializing. Following `readdir`
  order would make Rust's own saved bytes host-dependent for no parity gain.
  The closed group is identical either way; only a partially written one
  differs. Section lookup can differ only between two sections whose names
  differ solely by case, which the port cannot hold distinctly in any event
  because it keys sections by lowercase name.

- **Folder-group entry order:** C++ leaves a `GRPF_Folder` scan in host
  `readdir` order (C4Group.cpp:1177-1207; StdFile.cpp:823-836), and
  `C4MaterialMap::Load` assigns material slots straight from that scan
  (C4Material.cpp:263-299). Because the dynamic texture map is allocated in
  material order, an unpacked `Material.c4g` makes every generated landscape a
  function of the filesystem, so two peers on one content revision desync
  (clonk-org/clonk-rs#1455). Rust instead enumerates a folder as
  `C4Group::Sort` would order its packed image: rank by the group's `C4FLS_*`
  list, then `stricmp`, with raw bytes breaking the remaining tie
  (C4Group.cpp:2300-2336; C4Application.cpp:122 installs the list). That is
  the order shipped packed content already has — `c4group -p` on
  `content/Material.c4g` stores `Acid.c4m` before `ASHES.c4m` and `Oil.c4m`
  before `ORE.c4m`, which raw byte order inverts — so an unpacked local load,
  a packed network load and C++ reading either all assign the same slots.
  Packed groups keep their stored entry order on both sides.

## How the oracle stays honest

`oracle/gen_golden.sh` uses the actual engine code, not a hand-rewrite. Selected
construction rules and examples:

- `oracle_fixed.h` is **mechanically stripped** from `src/Fixed.h` (only the
  `StdCompiler`/`StdAdaptors` includes and the serialization `CompileFunc` are
  removed — the `C4Fixed` arithmetic is byte-identical).
- `SineTable` is lifted verbatim from `src/Fixed.cpp`.
- `src/C4Random.h` is included unmodified (its sole heavy include, `C4Record.h`,
  is `#ifdef DEBUGREC`, which the oracle does not define).
- `Randomize3`/`Rnd3` are reproduced verbatim from `src/C4Random.cpp` (10 trivial
  lines around the real `Random()`).
- `dig2object_rng` mechanically extracts the complete production
  `C4Object::DigOutMaterialCast` body. C++ records its `CreateObject` arguments
  and twenty following `Random` draws; Rust digs an identical one-pixel
  `Dig2ObjectRatio` material and compares the same spawn and ledger.
- Material corrosion and mass-mover transfer sections are small source-aligned
  RNG traces copied from the branch order in `src/C4Material.cpp` and
  `src/C4MassMover.cpp`; they intentionally avoid full engine setup while still
  pinning sync-critical `Random()` call order.
- `script_value_hash` is a source-aligned standalone copy of the small
  `hashCombine` / recursive `std::hash<C4Value>` path in `src/C4Value.cpp`.
- `script_value_convert` transcribes the 9×9 `C4ScriptCnvMap` table and the
  `ConvertTo` dispatch (`src/C4Value.cpp:431-598`) cell-for-cell — the real table
  is a private static of function pointers that cannot be linked without all of
  Game/C4Object. The oracle's copy and the Rust port's are *independent*, so a
  transcription slip on either side surfaces as a divergence. The Game-dependent
  `FnCnvGuess`/`GuessType` branch only runs for a non-zero `C4V_Any`; every input
  is a concrete type or nil, so no engine setup is needed.
- `script_killer` calls the production `C4ScriptKiller.h` helper verbatim.
  `C4Script.cpp` delegates both static engine functions to this same helper, so
  the oracle can vary context/target pointers and the player-validity predicate
  without copying the decision logic or linking the full game executable. The
  Rust checker drives its registered host functions through real C4Script calls,
  including explicit foreign and arrow targets plus a context-free call.
- `eval_direct_exec_context` mechanically extracts the complete production
  `FnEval` body and DirectExec's exact object Def/`LocalNamed`/parent setup.
  Minimal surrounding C++ contexts make each receiver and temporary child
  observable. The Rust checker drives the same three paths through the real
  C4Script VM: its object case can return 51 only by resolving both the live
  named local `power` and the target definition's `Explode` function; the
  other cases enter definition-only DirectExec through `DefinitionCall` and
  `Game.Script` through `global->eval`. The focused C++ scaffold validates
  receiver/setup/strict/source forwarding but does not run C++ `ParseFn` or
  expression execution. Scheduled expressions are exercised as part of the
  native scenario differential described in [`bridge/README.md`](bridge/README.md).
- `definition_commanded_effect_position` mechanically extracts the complete
  production `C4Effect::Execute`, `C4AulScriptFunc::Exec` engine-call
  forwarding, `C4AulExec::Exec` script-context setup, and `FnGetX`/`FnGetY`.
  Its scaffold gives an ID-commanded effect a carrier at `(320,-50)` but no
  command object, then records that the timer receives the carrier as its
  explicit target while its implicit position receiver remains null. Rust
  drives the same case through real `AddEffect` and effect-timer dispatch;
  callback lookup by command ID is also pinned by the focused engine
  regression.
- `effect_lifecycle` mechanically extracts every callback-bearing constructor,
  list walk, removal and command-pointer body from `C4Effect.cpp` and compiles
  them beside one recording callback scaffold. Eleven rows cover object and
  global lists; definition-ID and live object command receivers; priority 1 and raw
  negative insertion; Start denial with number reservation; Effect/Add temp
  cycling, including a no-callback kill of a lower recursive frame while its
  TempStop call is suspended; Timer/Kill/Stop; recursive ClearAll with callback
  replacement, insertion and Stop denial; live Damage deletion/recreation; silent command
  target loss; and fail-safe callback error side effects. Every callback makes
  one `Random(17)` draw, so its exact receiver, arguments, mutation order,
  effect-chain state, `RandomCount`, and `RandomHold` are compared together.
  This section composes with `effect_check`, `effect_execute`,
  `effect_callback_conversion`, and `definition_commanded_effect_position`:
  the older sections retain their deeper negotiation, cadence, conversion and
  implicit-position matrices while this lifecycle row set joins the remaining
  callback kinds and mutation seams end to end.
- `landscape_path` calls the production `C4LandscapePath.h` traversal used by
  `_PathFree`. Its edge-water input is the minimized Goldrush frame-143 live
  divergence; Rust runs the same density plane through a real `PixelGrid`.
- `action_direction` calls the production `C4ActionDirection.h` raw-xdir and
  TurnAction decisions used by `C4Object`. Its input is the minimized Goldrush
  frame-170 WIPF state; Rust runs the same Walk/Turn ActMap through a real
  engine frame and compares raw velocity/position plus action, facing, phase,
  and time.
- `action_swim_direction` drives the same production direction/TurnAction
  decisions with the minimized Goldrush frame-219 FISH state. Rust runs a real
  Swim/Turn ActMap frame and compares raw velocity/position plus action, facing,
  phase, and time; the decisive fixed-y snap is observable only when internal
  DFA_SWIM facing goes through `SetDir`.
- `action_push_pull_fight_direction` compiles the exact production direction
  branches mechanically lifted from DFA_PUSH, DFA_PULL, and DFA_FIGHT. Rust
  drives the corresponding real procedures through `Engine::apply_physics_at_index`
  and compares raw sub-pixel velocity, whole-pixel mirrors, facing, TurnAction
  dispatch, and the equal-x Fight case where C++ makes no `SetDir` call.
- `action_callbacks` calls the production `C4ActionCallbacks.h` dispatcher
  used by `C4Object::SetAction`. Its Start-only case is the minimized Goldrush
  frame-192 WIPF double-`Sitting` divergence; real Rust script fixtures also
  cover script Start/Abort and natural Start/End ordering.
- `connect_missing_target_removal` compiles the exact production target-check
  and `if (fBroke)` block lifted from DFA_CONNECT. A minimal C++ lifecycle
  scaffold records `LineBreak(true)`, `AssignRemoval`'s `Destruction`, and final
  status. `connect_geometry_break_removal` additionally compiles the exact
  `C4Shape::LineConnect` one-vertex failure guard and the later DFA_CONNECT
  `LineBreak()`/removal block. Rust drives both through the real
  `Engine::exec_connect_line` method and inspects each deleted line before frame
  cleanup. Other golden sections cover the general removal lifecycle.
- `line_connect_routing` compiles the **rest** of `C4Shape::LineConnect`
  (C4Shape.cpp:273-331) — the endpoint move, the three-range bend search seeded
  from `ForLine`'s reported intersection, the old-endpoint fallback and the
  insertion — together with the `InsertVertex`, `PathFree`/
  `PathFreeIgnoreVehicle` and `ForLine` bodies underneath it. Only the pixel
  plane below `GBackPix`/`GBackSolid` is scaffolding. It pins one C++ detail
  that decides how often lines break at all: `PathFreeIgnoreVehiclePix`
  compares a material **index** against `C4M_Solid = 50`
  (C4Landscape.cpp:2044-2048; C4Wrappers.h:68-71; C4Material.h:201), so every
  material a real scenario declares reads as non-solid there, the old-endpoint
  fallback almost always succeeds, and only a material whose index is itself
  ≥ 50 can make `LineConnect` return false. The callback trace and object
  status of the DFA_CONNECT handoff remain with
  clonk-org/clonk-rs#1243.
- `solid_mask_graphics` calls the production `C4SolidMaskBitmap.h` helpers used
  by `C4SolidMask`. Its decisive `(219,86)` input is the minimized Goldrush
  frame-184 CTWR Graphics2/SNKE contact: default graphics are transparent,
  Graphics2 is opaque. Rust runs that selection through a real mask bake and
  also tests cross-definition `SetGraphics` plus immediate remove/re-put.
- `shake_objects` mechanically extracts and compiles the complete production
  `C4Game::ShakeObjects` and `C4Object::Fling` bodies. Minimal object/action
  stubs force the raw fallback while preserving the real selection and RNG
  order; Rust drives the registered script host function over the same master
  list and compares every resulting velocity, attachment, mobile, and cause.
- `blast_free` mechanically extracts and compiles the complete production
  `C4Landscape::ClearPix`, `BlastFreePix`, and `BlastFree` bodies. A 7×7
  Surface8 fixture mixes Earth and Granite with/without IFT; Granite shifts to
  an explicit second Rock texture while Earth clears to sky or Tunnel's
  second/default texture+IFT. Rust blasts an identical real
  `PixelGrid` and compares pre-mutation `BlastMatCount`, every final byte, and
  `RandomHold`/`RandomCount`/`FRndPtr3` before and after the scan. A second
  mechanically extracted call pins the inclusive radius-zero center clear.
- `network_rule_goal_placement` mechanically extracts and compiles the complete
  production `C4SGame::ConvertGoals`, `C4Game::InitRules`, and
  `C4Game::InitGoals` bodies plus the `C4IDList` methods they invoke. The first
  case uses HarpoonRace's real `Rules=RVLR=1`, `Goals=RACE=1`, and omitted
  `StructNeedEnergy` default to record RVLR+ENRG and RACE placement. The second
  deliberately gives local Scenario.txt different counts from the synchronized
  parameters, proving that Rust applies the authoritative lists and preserves
  C++'s rule/goal zero-count asymmetry. This is a focused startup-method
  differential, not a full native network session; the production method
  bodies are exact while object creation and `UpdateRules` are recording
  scaffolds.
- `player_join_capacity` mechanically extracts and compiles the complete
  production `C4PlayerList::GetCount` body and the capacity block bounded inside
  `C4PlayerList::Join`. The generator also fails closed unless that block has
  exactly one count comparison, too-many-player log call, and null return. A
  three-row linked-list scaffold records zero, below-limit, and exact-full
  admission. Rust seeds and attempts the same named players exclusively through
  public `Engine::join_player` calls, then compares the real result and ordered
  roster. The C++ scaffold executes and validates the diagnostic call, but the
  Rust differential deliberately makes no logging claim: application-level
  presentation is covered by the join-control tests.
- `c4group_sort` is the first section driven through **linked** engine sources
  rather than an extracted snippet: `src/C4Group.cpp` and the file layer beneath
  it (`C4Strings`, `StdFile`, `CStdFile`, `StdGzCompressedFile`, `StdBuf`,
  `C4InputValidation`) are compiled into the oracle whole, so `Sort`,
  `SortRank`, `SortByList` and `WildcardMatch` are the real ones. That link is
  also why the C4Strings helpers are no longer lifted — the copies would be
  duplicate symbols beside the real translation unit — and why the oracle is
  built at `-std=c++23` with `-fwrapv` and `-DZLIB_CONST`. The wrapping flag
  defines LegacyClonk's intentional signed-overflow arithmetic for GCC/Clang
  invocations that honor it. SHA1 is stubbed,
  reached only by the `Original` author signature that no section verifies.
  `Sort` reorders the in-memory entry list, so this section needs no file bytes
  and touches none of the three host-dependent values a group rewrite would —
  the creation stamp, a file mtime, and the `#ifdef __linux__` executable bit.
  A name byte above 0x7f is deliberately excluded for the same reason:
  `stricmp` is locale-dependent there and the golden has no platform axis.
  The golden carries each fixture's insertion order as well as its result,
  because a comparator that reconstructs the input cannot distinguish "sorted
  correctly" from "never sorted at all" — an injection proved exactly that hole
  before the input was emitted.
- `scenario_sections` compiles the complete production
  `C4GameSave::SaveScenarioSections` body beside the real `C4ScenarioSection`
  constructor, both of its accessors, and the `C4Strings` helpers that splice a
  section name over the `*` in the real `Sect*.c4g` — so the composed entry
  name and the list order both come from engine code rather than a restatement.
  The destination group is a recorder: what a C4Group write *does* is not this
  section's subject, its call order is. The decisive rule is that the
  constructor **prepends**, so the sweep runs in reverse construction order and
  the implicit node the first section switch creates is reached first; the
  current section is deleted and never re-added even when modified; and `Add`'s
  result is discarded, so the sweep has no failure exit. Rust drives the same
  six section lists through a real `Engine` and compares the ordered
  destination mutations, expanding its single `Replace` back into the
  delete-then-add pair C++ emits. The constructor's folding of an empty or
  case-insensitive `main` onto `C4ScenSect_Main` is **not** exercised, and
  neither is section *discovery*: C++ takes C4Group entry order there while the
  port normalizes to a host-independent sort, so both sides are handed the same
  construction order.
- `contents_list_order` compiles the complete production linked-list bodies
  that allocate, remove, insert, and rotate `C4ObjectLink`s. Constructor
  serials make link identity observable without relying on allocator address
  reuse; Rust independently normalizes each object's incarnation counter to
  the same allocation sequence.
- `container_lifecycle` compiles the complete production `Enter`, `Exit`, and
  `Collect` bodies against a callback recorder. Rust drives the matching public
  script functions and compares callback order plus final containment,
  controller, status, mobility, liquid state, and raw fixed motion.
- `contact_action_bottom_flight` mechanically extracts the complete first
  `DFA_FLIGHT` switch arm from `C4Object::ContactAction` and the production
  `ObjectActionWalk`, `ObjectActionKneel`, and `ObjectActionFlat` helpers. Its
  three-case OR-gate matrix proves that low-speed `ObjectDisabled=1` reaches
  FlatUp exactly like `OCF_HitSpeed4`, while enabled low-speed flight walks.
  Rust drives the matching ActMaps through `Engine::exec_contact_action` and
  compares action, direction, and raw fixed velocities.
- `contact_action_top_side_flight` mechanically extracts the ceiling and both
  wall `DFA_FLIGHT` arms, `ObjectActionTumble`/`Scale`/`Hangle`, and the shared
  unresolved-flight tail. Enabled controls enter Hangle/Scale; matching
  low-speed disabled cases enter Tumble, bypass those fallbacks, and compare
  the pre-tail raw Tumble velocity plus final action, direction, position, and
  raw fixed velocity after slide-free.

If a divergence is ever a *bug in the golden* rather than the Rust port, fix the
C++ source and regenerate.

## Usage

```sh
# Verify every registered comparator (the wrapper runs each package separately
# and fails if any package has no matching test):
cargo xtask parity verify
# For a focused engine-only run:
cargo nextest run -p clonk-engine-unit-tests --test engine_inline \
  --no-tests=fail -E 'test(/(^|::)parity_differential_matches_cpp_golden$/)'

# Regenerate the golden after changing the C++ primitives or oracle coverage
# (requires a C++23 compiler; honours $CXX, defaults to clang++):
parity/oracle/gen_golden.sh
#   or:
cargo xtask parity record
```

The generator defaults to this repository and archives the pinned C++ source
snapshot at commit `7d43b47b7d789b533f32d005e64596e0a07019cd` from its Git
history into the disposable `.gen` directory before extraction and
compilation. A shallow clone must fetch that history before recording. Set
`LEGACYCLONK_ORACLE_ROOT` to use a separate C++ checkout, or
`LEGACYCLONK_ORACLE_REVISION` for an intentional source revision override.

The Rust checker is `crates/clonk-engine/src/parity_differential.rs`. On any
mismatch it panics with `PARITY DIVERGENCE in <section> entry <i> field <f>:
C++ golden = <x>, Rust = <y>` — i.e. the first divergence, fully localized.

## Historical full-scenario evidence

The [Gold Rush seed 424242 report](reports/goldrush_seed_424242.json) records a
continuous historical shadow differential through frame 15,000. It uses the
Rust revision that established the previous 14,415-frame horizon and the exact
scenario, content, and player inputs recovered from that run. The pinned
oracle's base C++ `src` tree is byte-identical at the execution revision. The
report records both base tree IDs and the diagnostic-only bridge/ABI patch that
makes synchronized `FRndPtr3`/`RandomHold`/`RandomCount` mismatches fail closed
on every frame without changing simulation logic.

This is historical evidence for clonk-org/clonk-rs#394, not a claim that the
current Rust engine matches the full C++ scenario. It remains tied to the Rust
revision recorded in the artifact. Use the live bridge below for the current
tree.

## Layout

```
parity/
  oracle/
    oracle_main.cpp     # the golden generator (emits JSON)
    gen_golden.sh       # strips src/ headers, compiles, runs -> golden
    .gen/               # generated build inputs (oracle_fixed.h, sine_table.cpp) — disposable
  golden/
    parity_golden.json  # committed C++ golden output (the oracle)
  reports/
    goldrush_seed_424242.json             # historical scenario horizon
    goldrush_seed_424242_rng_ledger.diff  # fail-closed RNG diagnostic
  bridge/                                 # current-tree full-scenario shadow diff
```

## Live full-scenario shadow diff

The current tree exposes the C ABI consumed by the pinned oracle's
`src/rust/RustEngineBridge.cpp`. Build and arming instructions, comparison
boundaries, and the traps that can make a run compare nothing are in
[`bridge/README.md`](bridge/README.md).

The two harnesses answer different questions. `cargo xtask parity verify`
compares the committed bounded C++ fixtures and is a required gate. The live
bridge boots real content and finds cross-system drift, but requires a local
oracle build and is intentionally an investigation tool rather than a CI gate.
A green result from either one must not be reported as evidence for state the
other harness alone observes.
