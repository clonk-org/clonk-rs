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
