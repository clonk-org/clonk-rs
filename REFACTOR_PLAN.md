# clonk-app decomposition campaign

Directive (2026-07-21): faster builds, DRY, Rust best practices — decompose the
`clonk-app` monolith into smaller packages. Every step is STRUCTURAL (no
behavior change), lands one commit at a time through the full workspace gate,
and follows Tidy First. Prior build-time findings (2026-07-11, see git history)
still bind: the app crate's downstream cascade dominates rebuild times; crate
extraction is the standing "next lever". Do not re-try rejected experiments
(opt0 test profile, -Zthreads, cranelift).

## State after the first landings

- All packages renamed `lc-*` → `clonk-*` (extern "C" `lc_*` FFI symbols kept —
  they are the ABI the C++ bridge calls; `CMakeLists.txt` imports
  `libclonk_engine.*`).
- `clonk-app/src/main.rs` split: ~97.8k lines production + `main_tests.rs`
  (114.5k, `#[path]`-mounted `mod tests` — same test ids, same private access).
- Satellite modules and seam counts (crate:: refs) at split time:
  `gpu_renderer` 0, `draw_commands` 0, `network` 1, `object_menu` 2
  (→ingame_menu), `game_over` 4, `ingame_menu` 4; ~24 more small satellites.
- `impl GameApp` is a single ~63k-line block (main.rs ~27.8k→~91k).

## Step queue (each = one structural landing; re-measure build times after each)

1. **clonk-app-render**: move `gpu_renderer.rs` + `draw_commands.rs` (zero
   seams) into a new package; `pub(crate)` → `pub` inside the moved files;
   main.rs re-imports via `use clonk_app_render::{draw_commands, gpu_renderer};`
   so call sites don't churn. Their inline tests move with them (test ids gain
   the new package prefix; nextest overrides don't reference them).
2. **clonk-app-menus**: `ingame_menu.rs`, `object_menu.rs`, `game_over.rs`,
   `menu_controls.rs` (+ their small helpers). Cut the ~10 `crate::` seams by
   moving the shared types they touch into the new crate or an app-core crate
   (see 4) — never by widening visibility of GameApp internals.
3. **clonk-app-netplay**: `network.rs`, `network_host_preparation.rs`,
   `client_network_scenario.rs`, `client_start_barrier.rs`,
   `configured_client_players.rs`, `host_game_resource_sources.rs`,
   `prepared_host_bootstrap.rs`, `control_message.rs` (one seam:
   network→prepared_host_bootstrap stays internal to the new crate).
4. **clonk-app-core**: shared leaf types out of main.rs (AppMode, boundary
   enums, staging structs, resource-bundle structs, paths/config plumbing) so
   steps 2-3 and later peels depend on it instead of on main.rs.
   Step-2 deferrals to absorb here: the shared software picture pipeline that
   kept `object_menu.rs` in clonk-app (`object_menu_item_picture` and its
   `_with_text_spec_resources`/`_with_renderer_modes` variants,
   `cached_menu_object_picture*`, `menu_rank_picture`,
   `menu_object_rank_picture`, `compose_owned_menu_picture*`,
   `crop_menu_image`, the `inventory_*` picture helpers,
   `prepare_owned_menu_definition_pixels`, `centered_picture_transform`,
   `inventory_object_picture_with_allowed_modes`, `resolve_script_font_image`,
   `ScriptTextSpecResources`), plus the interim `clonk_app_menus::menu_images`
   leaves which can sink to app-core (or clonk-graphics) once that pipeline
   moves; finish by moving `object_menu.rs` into clonk-app-menus.
   Step-3 residue for this step: the generic native-config helpers
   (`configured_native_*`, `NativeConfigValue`,
   `update_configured_native_values`) ride along in
   `clonk_app_netplay::configured_client_players` but serve dozens of pure
   local-config call sites in clonk-app (language, gamepad, graphics, toasts);
   re-house them here. clonk-app's old lib target dissolved into
   clonk-app-netplay, so this crate starts from main.rs alone.
5. **main_tests.rs decomposition**: split by area into `tests/` submodule files
   mounted from the same `#[path]` root (keeps ids); move tests that only
   exercise an extracted crate INTO that crate.
6. **impl GameApp split** (the big one, LAST): break the 63k-line impl into
   per-area extension traits or free functions living in the area crates,
   keeping `GameApp` state in clonk-app-core. Design doc required before
   execution; expect several landings.
7. **Best-practices pass per new crate** (after its extraction lands):
   clippy -D warnings clean, error types instead of stringly errors where the
   surrounding code already trends that way, dedupe helpers that the split
   makes visible. Behavioral changes stay OUT of structural commits.

## Rules

- One step per landing; full workspace gate green (fleet landing protocol).
- Never mix structural and behavioral changes; parity oracles are untouchable.
- Measure: `time cargo build -p clonk-app` (warm, body-edit loop) before/after
  each step and record here.

## Build-time log

- Baseline after rename+test-split (2026-07-21): body-edit loop ≈ 23.5s
  (`cargo check -p clonk-app --tests`, warm deps).
- Step 1 landed (clonk-app-render): focused 30/30; warm incremental
  `cargo check -p clonk-app --tests` after a touch of main.rs ≈ 2.2s
  (the 23.5s baseline above was a first-build, not incremental — treat the
  pair as cache-state bounds, not a strict before/after).
- Step 5 landed (main_tests.rs decomposition): spliced into 15 area part files
  under `crates/clonk-app/src/main_tests/` via `include!` — bare item
  sequences into the same `mod tests`, NOT child modules, so every test id
  stays `tests::<fn>` and nextest overrides / evidence citations are
  unaffected. Root keeps the prelude, shared helpers (144 units), and 9 items
  pinned by file-relative `include_bytes!`/`include_str!` paths (4,326 lines);
  parts run 2,069 (chat_messages) to 18,660 (netplay) lines, all under the
  20,000-line rustfmt-stdin limit. Id-stability proof:
  `cargo nextest list -E 'package(clonk-app)'` byte-identical pre/post
  (1,858 ids); full clonk-app battery 1858 run / 1858 passed / 3 ignored-skips
  on both sides; the original 114,489-line file byte-reconstructs exactly from
  the emitted files. Warm body-edit loop unchanged (≈2.3s; a part-file touch
  costs the same — this step buys navigability, not build time). Moving
  render-only tests into clonk-app-render was skipped: no test exercises that
  crate alone (both candidates also drive GameApp).
- Step 2 landed (clonk-app-menus): ingame_menu/game_over/menu_controls/
  clonk_fonts + the menu_images leaf compositors moved out of the app bin
  (object_menu.rs deferred on the shared picture pipeline — see step 4);
  focused 78/78 in the new crate + consumer sweep 170/170 in clonk-app; warm
  incremental `cargo check -p clonk-app --tests` after a touch of main.rs
  ≈ 2.0-2.3s (parity with step 1); menu-crate edits now re-typecheck only
  clonk-app-menus (≈18s full lib+lib-test worst case) instead of the
  212k-line bin+bin-test pair.
- Step 3 landed (clonk-app-netplay): netplay package suite 238/238; clonk-app
  join/lobby/network consumer sweep 257/257; warm incremental
  `cargo check -p clonk-app --tests` after a touch of main.rs ≈ 2.0s.
- Step 4 landed (clonk-app-core + object_menu move): the shared picture
  pipeline left main.rs for `clonk_app_core::pictures` (the
  object_menu_item_picture closure, the inventory_*/menu_rank_*/
  compose_owned_* helpers, ScriptTextSpecResources and the text-spec
  resolvers, plus object_menu's apply_definition_owner_color and
  definition_menu_picture); the clonk-app-menus menu_images leaves sank to
  `core::menu_images` (menus re-exports the old paths, the further
  clonk-graphics sink stays a step-7 candidate); netplay's generic
  configured_native_*/NativeConfigValue/update_configured_native_values
  machinery re-housed as `core::native_config` with its 8 tests (netplay
  re-exports the old paths and imports the shared decode/trim helpers);
  AppMode and the ClassicGameLobbyBoundary/Child +
  ClassicGuiBootstrap/ClassicStartupBootstrap Issue/Defect leaf enums moved
  to the core root. main.rs re-imports every moved name under its old path
  (test-only names via the main_tests prelude), so call sites and spliced
  test parts did not churn; the FrontendAssets-coupled constructors stayed
  behind as script_text_spec_resources_from_assets{,_and_hud} free
  functions. object_menu.rs then completed its step-2 deferral into
  clonk-app-menus (pub(crate)->pub; set_mode_for_parity_test behind a menus
  `test-hooks` feature mirroring netplay's; main.rs keeps
  `use clonk_app_menus::object_menu;` so game_message.rs's
  crate::object_menu::draw_menu_decoration seam and the test parts'
  object_menu:: paths still resolve). Focused: core 8/8, netplay
  configured/native 34/34, menus 115/115 (78 + 37 object_menu tests
  including both real-asset cases), clonk-app consumer sweep 61/61
  pre-move / 30/30 post-move (object_menu's ids now carry the menus
  prefix). Warm incremental `cargo check -p clonk-app --tests`: main.rs
  body edit ≈ 1.9s (parity with steps 1-3); a core body edit cascades
  core→menus→netplay→app in ≈ 2.1s, while the core-only
  `cargo check -p clonk-app-core --tests` loop is ≈ 0.16s.
- Step 7 first pass over the extracted crates (dead code + mechanical
  clippy): per-crate `clippy --all-targets -W clippy::all` warnings
  core 6→1, render 9→0, menus 15→11, netplay 23→18 (53→30 total).
  Deleted the extraction-flagged dead code (menus GAP_BEFORE_FOOTER,
  netplay `fn source`); NetworkWorkerReady.local_addresses stays — the
  host_worker_binds_and_advertises… unit test asserts on it. Remaining
  warnings are deliberate skips: 16× too_many_arguments, 4×
  large_enum_variant (boxing), 2× type_complexity, netplay's
  frame%control_rate is_multiple_of pair (divisor-zero panic semantics
  in the control cadence), one collapsible_match whose fix hoists a
  mutating commit() into a match guard, and cfg(test)-region lints
  (test edits out of scope for the pass). Manifest sweep: no unused
  deps; the dev-dep feature-union comments were already uniform.
  Dedupe visible from the split: only the test-helper repository_root
  ×3 (all in test code — deferred with the menu_images→clonk-graphics
  sink). game_over.rs, gpu_renderer.rs and prepared_host_bootstrap.rs
  are rustfmt-clean again; ingame_menu/object_menu/network.rs keep
  their pre-extraction deviations (many sit inside test modules).
  Focused suites after the pass: core 8, render 27, menus 115,
  netplay 230 — identical counts, all green.
- Step 6a landed (impl GameApp split into per-area files): the single
  62,789-line / 1,389-method `impl GameApp` block became 14 `#[path]`-mounted
  area files under `crates/clonk-app/src/game_app/` (each `use super::*;`
  plus its own `impl GameApp`), methods moved byte-verbatim in original
  order: input 12,569 / network 7,683 / lobby 7,535 / startup 5,721 /
  menu 4,697 / render 4,635 / config 2,364 / console_record 2,102 /
  scenario 1,892 / player 1,710 / scensel 1,657 / saves 1,426 / chat 1,353 /
  sound 1,169 lines; 170 methods (6.4k lines, incl. the constructors) stay
  in the root impl; main.rs 96,243 → 39,889 lines. 893 of 1,219 moved
  methods needed private → pub(crate) (compiler-enumerated cross-module
  callers, incl. the `mod tests` splice). Byte-partition proof: the original
  impl body reconstructs exactly from the root + area chunks (each byte
  once, order preserved per file) plus the enumerated pub(crate) insertions.
  nextest id list byte-identical pre/post (1,496 ids); battery 1,496/1,496
  both sides; menus+netplay sweep 345/345. Warm incremental
  `cargo check -p clonk-app --tests`: area-file touch ≈ 1.9s, main.rs touch
  ≈ 2.0s (parity with steps 1-4 — same crate, so this step buys
  navigability and the 6b seam, not check time). Area files inherit
  main.rs's legacy formatting verbatim; rustfmt normalization of
  `src/game_app/` is a deliberate follow-up, kept out of the move commit.

## Step 6b design

After 6a, GameApp methods live in per-area files but all state still sits on
the one struct. 6b extracts per-area sub-state into the area crates so area
logic leaves the app crate entirely; `GameApp` becomes a composition of area
states (`render: RenderState`, `sound: SoundState`, ...). One area per
landing, render first (clonk-app-render already exists):

1. Introduce `<Area>State` in the area crate by moving the GameApp fields
   only that area touches; `GameApp` holds it as a field and call sites
   become `self.<area>.<field>`.
2. Move that 6a file's methods onto `impl <Area>State`, verbatim modulo the
   receiver; tests that only exercise the area state move with them.
3. Cross-area methods take explicit multi-state signatures
   (`fn x(render: &mut RenderState, net: &NetworkState)`) — never
   `&mut GameApp` inside an area crate; shared leaf types sink to
   clonk-app-core first.
4. Direction rule: area crates never depend on clonk-app, nor on each other
   except through clonk-app-core. Order after render: sound, saves, chat
   (smallest coupling surface), then lobby/network last (widest overlap).

## Wave 2 — remaining monolith decomposition (2026-07-22)

Targets (lines at wave start): clonk-engine/compat.rs 85,672;
clonk-engine-unit-tests/tests/unit/main.rs 78,217; clonk-engine/lib.rs 74,275;
clonk-engine/scenario.rs 36,731; clonk-frontend/lib.rs 34,918;
clonk-engine/command.rs 29,144; clonk-network/session.rs 26,488;
clonk-engine/direct_com.rs 20,165.

Recipes (structural only, byte-verbatim moves, full gate per landing):
- PRODUCTION monoliths → real child modules (`<name>/` directory modules),
  byte-verbatim item moves, compiler-enumerated minimal pub(crate)
  promotions, parent re-imports children into the original scope so call
  sites and `super::*` in tests do not churn. Inline `#[cfg(test)] mod`
  blocks STAY in the parent file — running test ids must never change
  (frontend inline tests are mounted by clonk-frontend-unit-tests; nextest
  overrides and queue evidence cite `tests::` ids). Engine files are
  determinism-critical: moves only, zero expression edits.
- TEST monoliths → `include!`-spliced part files (the main_tests.rs
  mechanism): same module, ids byte-stable, byte-partition proof required.
- After each area lands: DRY/clippy follow-up pass may consolidate
  duplication the split exposes (separate commits; parity-sensitive lints
  stay untouched, as in step 7).

Order: wave A parallel across disjoint crates (compat.rs, frontend lib.rs,
network session.rs, engine-unit-tests main.rs); wave B engine siblings
(scenario.rs, command.rs, direct_com.rs — parallel, disjoint files, module
file→dir conversions leave lib.rs decls untouched), then engine lib.rs
last (crate root, biggest care); wave C per-crate DRY/clippy. GameApp 6b
state extraction remains queued behind wave 2.

## Wave 2 landing log

- compat.rs test splice landed: the 34,767-line `compat::tests` body left the
  parent file for 11 byte-verbatim contiguous parts under
  `crates/clonk-engine/src/compat/tests/`, spliced back with `include!` so the
  module — and every `compat::tests::*` id — is unchanged. compat.rs
  34,889 → 151 lines (production was already only ~117 lines; the file was
  99.6% test body). Prelude `use` items stay in the parent so the parts are
  bare item sequences. Byte-partition proof: the parts concatenate to the
  original module body exactly, line for line. `cargo nextest list --workspace`
  byte-identical pre/post (8,861 lines); full gate 8,828 run / 8,828 passed /
  11 skipped. Parts run 2,903-5,411 lines. Known follow-up: `xtask` dev_check
  routes engine checks by file basename, so edits to a part file no longer
  match the `"compat.rs"` arm — the parent still does.
- command.rs test splice landed: the 16,882-line `command::tests` body moved to
  7 byte-verbatim contiguous parts under `crates/clonk-engine/src/command/tests/`
  (2,255-2,468 lines each), spliced with `include!` from the unchanged inline
  module. command.rs 29,109 → 12,246 lines; the remaining body is production
  (snapshots, the command model, `impl CommandStack`, the per-command state
  machines) and is the next split. Byte-partition proof + `nextest list`
  byte-identical (8,861); gate 8,828/8,828, 11 skipped.
- scenario.rs test splice landed: the 18,832-line `scenario::tests` body moved
  to 8 byte-verbatim parts under `crates/clonk-engine/src/scenario/tests/`
  (753-2,797 lines), spliced with `include!`. Two items pinned by file-relative
  `include_str!`/`include_bytes!` paths into `content/` stay in the parent (162
  lines) — the same carve-out main_tests.rs needed. The splitter now validates
  every candidate boundary against a Rust code mask, because scenario.rs
  embeds C4Script raw strings whose column-0 `}` lines otherwise read as item
  ends. `mod game_start_sync` (854 lines) stays inline. scenario.rs
  36,967 → 18,320 lines; the ~17.3k-line production half (Scenario, the legacy
  C4S parsers, LegacyObjectRecord, the map-pixel classifier) is the next split.
- lib.rs test extraction landed: 41 of the crate root's 42 inline
  `#[cfg(test)] mod <name>` regression modules became `#[path]`-mounted files
  under `crates/clonk-engine/src/lib_tests/`, keeping each module name — and
  therefore every `<name>::*` test id — identical. Bodies are dedented by one
  level with a mask-aware pass that leaves any line whose first character sits
  inside a string literal untouched, so the embedded C4Script raw strings keep
  their exact contents; the dedent is proved invertible per module.
  `mod material_colorization_regression` stays inline (file-relative
  `include_bytes!`). lib.rs 74,930 → 62,458 lines. The remaining bulk is the
  40,003-line `impl Engine` block — the 6a-style area split is next.
- lib.rs `impl Engine` split landed (the engine's 6a): the single 40,001-line /
  435-method block became 19 `#[path]`-mounted area files under
  `crates/clonk-engine/src/engine/`, each `use super::*;` plus its own
  `impl Engine`, methods moved byte-verbatim in original order: state 4,681 /
  procedures 4,343 / tick 3,554 / economy 3,425 / players 2,832 /
  solid_mask 2,726 / landscape_ops 2,671 / command_results 2,046 /
  movement 1,908 / definitions 1,710 / exec_order 1,514 / spawn_queue 1,411 /
  script_exec 1,254 / crew 1,242 / host_tables 1,153 / player_view 1,129 /
  world 909 / config 790 / game_over 512 lines. Only the two constructors
  (`new`, `with_seed`) stay in the root impl; lib.rs 62,458 → 22,690 lines.
  260 private method declarations needed `pub(crate)` (statically enumerated
  from cross-module references, then confirmed by a clean compile). Byte-
  partition proof: the root impl plus the 19 area impls reconstruct the
  original 40,001-line body line for line, with every difference being one of
  the 260 enumerated `pub(crate)` insertions. nextest id list byte-identical
  (8,861); gate 8,828/8,828, 11 skipped.
- compat host-context prologue collapse (wave C, DRY): 270 of the ~790
  `HOST_CONTEXT.with(|cell| { .. })` host-function bodies opened with the same
  four-line borrow-and-bail preamble. Two private helpers in compat.rs —
  `with_host_context` / `with_host_context_mut`, taking the inert fallback and
  a closure over the context — replace it, so each wrapper now starts at its
  own logic. Every fallback is a constant expression, so moving it from the
  bail arm to an argument does not change what is evaluated. 6 sites whose
  bodies still name `borrow`/`cell` were skipped by the transform, as were the
  `match`-form and `and_then`-chain variants (next pass). Net -1,067 lines with
  the wrappers' C++ `file:line` citations untouched.
- compat host-context collapse, second pass: the `match borrow.as_ref() { .. }`
  form (51 sites) and the `cell.borrow().as_ref().map(..).unwrap_or(..)` form
  (9 sites) now go through the same two helpers. Net -300 lines. Left for a
  later pass: the `.ok_or_else(|| RuntimeError::new(".."))?` chain form (~66
  sites), which needs a lazy-error helper so the message is not allocated on
  every call, and the multi-step `and_then` chains, which are real expression
  rewrites rather than a preamble swap.
- compat host-context collapse, third pass: the 63 `.ok_or_else(|| RuntimeError
  ::new(".."))?` chain sites now use `try_with_host_context` /
  `try_with_host_context_mut`, which take the message as `&str` and only build
  the error on the missing-context path — so the success path still allocates
  nothing. Net -252 lines. Remaining in this family: the multi-step
  `and_then` chains (~146 two-line prefixes), which are expression rewrites
  rather than preamble swaps, and 6 sites whose bodies name `borrow`/`cell`.

## Deferred, with reasons (2026-07-24)

- **Engine field sub-structs** (`struct Engine`, 156 fields): every candidate
  group — base rules (12 fields), teams (8), crew info (7), pathfinder (3),
  scenario sections (5) — is read *and written* directly by test code
  (41/178/62/5/10 references across 7-17 test files). Regrouping them into
  sub-structs is a field-path rename that necessarily edits those test bodies,
  which the current landing rule forbids for rename commits. Doing it needs
  either an explicit exception, or a preparatory commit that migrates the tests
  onto accessors first — itself a test-touching change. Not attempted.
- **Dead-code deletion**: the workspace is `dead_code`-clean under
  `clippy -D warnings`, so nothing is compiler-detectably dead. A cross-crate
  scan finds 147 `pub` items never named outside their own definition, but
  spot-checking shows they are deliberate C++-mirror surface — `set_button_text`
  (C4GUI::Button::SetText), `keep_portrait` (the `Keep` half of a
  clear/keep pair), `mean_value` (documented as the oracle's misnamed
  `GetMedianValue`) — not dead code. Deleting them is a decision about the
  port's intended API surface, not a mechanical tidy, so the inventory is
  recorded here rather than acted on. Breakdown by crate: engine 55, core 26,
  frontend 24, network 11, script 8, gui 6, app-menus 5, app 3, resources 3,
  graphics 2, platform/audio/launcher-ui/app-core 1 each.
- **clonk-app/src/main.rs** (40,420 lines) is untouched: unlike the engine
  files it has no single monolith left after step 6a — it is ~1,077 top-level
  items whose largest block is the 6,441-line root `impl GameApp`. Splitting it
  is the queued step 6b (per-area state extraction), which is a design change,
  not a byte-verbatim move.
- command.rs production split landed: the 12,186-line production half became
  `snapshot.rs` 419 / `model.rs` 894 / `geometry.rs` 240 / `machine.rs` 6,845
  and `machine/stack.rs` 3,825, with command.rs down to 69 lines (imports,
  consts, the module decls and the id-frozen `mod tests` splice).

  Two boundary lessons, both forced by the compiler:
  * `impl CommandStack` reaches into every per-command state struct's private
    fields (713 accesses). Rather than publish those fields, `stack` became a
    **child** of `machine` — a child can see its parent's privates, so the
    state machine keeps its encapsulation and only the module nesting changed.
    `CommandSnapshot` and `ActiveCommand` are likewise mutually private, so the
    old `runtime` group was merged into `machine` instead of being its own file.
  * The inline `mod tests` stays at `command::tests` (ids are frozen), which
    makes it a *sibling* of the new children and unable to see their internals.
    The fix is `pub(in crate::command)`, not `pub(crate)`: it restores exactly
    the visibility those items had before the split — the whole `command`
    subtree — with no widening. 235 declarations use it; only 73 needed real
    `pub(crate)` because something outside `command` already called them.

  Byte-partition proof: all 12,186 production lines reconstruct from the five
  files line for line, with 239 lines differing solely by a visibility prefix.
- scenario.rs production split landed: the 17,220-line production half became
  `core.rs` 4,944 / `legacy_parse.rs` 4,127 / `sections.rs` 3,034 /
  `map.rs` 1,748 / `definitions.rs` 1,163 / `values.rs` 940 / `c4value.rs` 899 /
  `legacy_types.rs` 413, with scenario.rs down to 1,120 lines (imports, the
  module decls, and the id-frozen `mod tests` splice plus its two
  file-relative-macro tests and `mod game_start_sync`).

  Visibility follows the command.rs rule — `pub(in crate::scenario)` restores
  the exact pre-split scope, `pub(crate)` only where an outside caller already
  existed; each module's `pub use` re-export is set to that module's widest item
  visibility so no glob claims more than it re-exports. 567 declarations changed
  visibility; the 17,220 production lines are otherwise byte-identical.

  Two tooling lessons for the next split:
  * Promoting by *name* is wrong. Making a private inherent `fn serialize`
    visible let it win method resolution over `serde::Serialize::serialize` at
    call sites in sibling modules — E0061/E0308, not a privacy error. Promote
    only declarations rustc actually names, via JSON diagnostic spans, and
    verify the span really is a declaration (an early version spliced
    `pub(in ...)` into the middle of expressions).
  * Conversely, span-driven promotion alone is not sufficient: when a private
    method is shadowed by a trait method in scope, rustc reports the *arity*
    mismatch instead of E0624, so the loop must consume every error class, not
    just E0616/E0624/E0603/E0451.
