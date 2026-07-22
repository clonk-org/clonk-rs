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
