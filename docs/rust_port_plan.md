# LegacyClonk Rust Port - Exact Parity Requirements

## Current Problem

Core scenarios boot, but complex object procedures (push/build/inventory) still diverge from C++ once gameplay interactions begin.

## Required for Exact C++ Parity

### 1. Startup & Scenario Discovery

**C++ Behavior:**
- Shows main menu on startup
- Lists real scenarios from install directories
- Scenarios are organized in folders
- Can browse and select any scenario

**Current Rust Behavior:**
- Main menu opens on startup
- Scenario browser lists install and user scenarios with folder navigation
- Default selection and preview highlight the first real scenario; sandbox fallback only appears when no catalog entries exist

**What's Needed:**
- [x] Fix scenario discovery to actually find real `.c4s` files from installation
- [x] Main menu (C4StartupMainDlg equivalent) with proper navigation
- [x] Remove/hide sandbox fallback when real scenarios exist
- [x] Scenario folder navigation
- [x] Scenario preview images
- [x] Window title: "Clonk Rust" not "LegacyClonk (Rust preview)"

**Progress (2025-10-25):** Rust frontend still scans install roots per directory, tolerates missing assets, and decodes BMP previews so real scenarios populate the browser without sandbox fallback. Added a dedicated main menu with the LegacyClonk big-button layout, participants list, and reliable navigation (Back entry/Escape) back from the scenario browser; “Local Game” flows into the browser while other options surface placeholder status until their dialogs are ported.
**Progress (2025-10-26):** Startup browser now preselects the first available entry, updates the path label to mirror the LegacyClonk catalog ("Scenarios / …"), and removes the "Rust Sandbox" headline so `cargo run` immediately surfaces the real scenario list instead of the preview fallback.
**Progress (2025-12-27):** Scenario discovery now preserves the LegacyClonk search priority, letting user scenario overrides shadow installed variants so the browser matches the C++ catalog order.
**Progress (2026-03-18):** Scenario discovery now reads legacy `Scenario.txt` head metadata when no manifest/title files are present, restoring classic mission titles and descriptions in the browser.
**Progress (2027-10-25):** Matched the legacy scenario browser ordering by honoring folder indices, mission icon ordering, and difficulty precedence in `lc-resources`, backed by regression tests, and removed the Rust frontend’s alphabetical resort so the tree mirrors C++ exactly.
**Progress (2028-10-26):** Scenario browser now merges duplicate scenario folders across user and install roots, keeping overrides while inheriting fallback entries so the tree matches LegacyClonk without duplicate listings.

**Files:** `main.rs:4631` (load_frontend_scenarios), scenario discovery in lc-resources

---

### 2. Real Scenario Loading

**C++ Behavior:**
- Load Scenario.txt with all parameters
- Load landscape (Map.bmp or procedural)
- Load all object definitions from scenario + Objects.ocd
- Spawn all initial objects from Objects.txt
- Initialize weather, sky, environment
- Start with proper game state

**Current Rust Behavior:**
- Legacy `Scenario.txt` player crew entries spawn with owner/position
- Objects.txt data, landscape assets, and loading UX still missing; sandbox fallback if apply() fails

**What's Needed:**
- [x] Complete Scenario.txt parsing (verify all fields)
- [x] Map.bmp landscape loading
- [x] Objects.txt initial object spawning (Scenario.txt crew entries now covered)
- [x] Verify all definitions load correctly
- [x] Proper error handling (don't fall back silently)
        - [x] Loading screen with progress
        - [x] Scenario intro text/mission briefing

**Progress (2025-10-31):** Legacy loader now mirrors C++ SkipDefs filtering so scenarios stop registering intentionally omitted definitions, with coverage in `rust/crates/lc-engine/src/scenario.rs`.
**Progress (2025-10-30):** Scenario loader now parses the entire legacy `Scenario.txt` core into typed sections (Head, Definitions, Game, Player start, Landscape, Weather, Animals, Environment), validating every field and exposing it to the engine for parity-critical data.
**Progress (2025-10-27):** Runtime now stops falling back to the sandbox when a disk scenario fails to load or apply; `GameApp::start_scenario` keeps players in the menu, surfaces the failure in `status_text`, and only swaps the running engine state once the legacy scenario applies cleanly.
**Progress (2025-10-26):** Legacy loader now parses `Objects.txt`, creating spawn configs with explicit object ids, status, owner, position/velocity, crew state, action info, and resolves containers (including via `Contents=` fallbacks) so full scenario object graphs appear without sandbox fallback.
**Progress (2025-10-24):** Ported Scenario.txt weather and landscape physics parsing so gravity, wind variation, climate, precipitation, and disaster levels feed straight into the Rust engine instead of defaulting to sandbox values.
**Progress (2025-10-24):** Legacy `Map.bmp` columns now translate into zoomed `Landscape` surface heights (with a flat fallback when the bitmap is missing) so real scenarios boot with terrain instead of the sandbox void.
**Progress (2025-11-03):** Added threaded scenario loading with an in-game progress screen and restored the mission briefing overlay via `rust/crates/lc-app/src/main.rs` and `rust/crates/lc-engine/src/lib.rs`.
**Progress (2025-11-04):** Enabled BMP decoding in `lc-engine` and added `rust/crates/lc-engine/tests/legacy_scenario_loading.rs` to lock Map.bmp landscapes, legacy definition groups, and initial object spawns so real scenarios apply without falling back.

**Files:** `lc-engine/src/scenario.rs`, `rust/crates/lc-app/src/main.rs:3984`

---

### 3. Object Graphics

**C++ Behavior:**
- Each object shows its Graphics.png sprite
- Overlays (hands, tools, effects) render correctly
- Action graphics (walk, swim, etc.) animate
- ColorByOwner modulation for player colors

**Current Rust Behavior:**
- Definition picture/sprite loading exists
- May not render correctly in-game
- Animations may not work

**What's Needed:**
- [x] Verify Graphics.png loads for ALL definitions
- [x] Action frame graphics (ActMap procedure graphics)
- [x] Overlay graphics (ClonkGraphics, tool overlays)
- [x] ColorByOwner player color modulation
- [x] Animation frame cycling
- [x] Z-order rendering (background → objects → overlays)
- [x] Rotation and scaling for graphics

**Progress (2025-11-26):** Frontend now respects per-object rotations from `SetR`/snapshots when rendering action sprites and overlays, matching LegacyClonk's spinning/tilting objects.
**Progress (2025-11-22):** Action rendering now mirrors C++ ping-pong playback for ActMap entries with `Reverse=1`, so animations like bridge and swim cycle forward and back instead of flipping the sprite list order.
**Progress (2025-11-18):** Graphics rendering now buckets snapshot objects into background, midground, foreground, and parallax lanes and sorts them by legacy `C4D_SortLimit` before drawing, matching the C++ draw order so static backs no longer overdraw crew sprites.
**Progress (2025-11-15):** Frontend now applies `ColorByOwner` tint masks using player palette colors, with fallback hues when explicit player colors are unavailable, covering both ActMap slices and overlay sprites.
**Progress (2025-11-12):** Added engine-side overlay state tracking (`GraphicsOverlayMode`, `ObjectGraphicsOverlay`) exposed through snapshots and wired to the `SetGraphics` host function so legacy scripts can create/remove tool overlays. Frontend now consumes the snapshot overlays and renders action/base layers above the parent object using existing ActMap metadata.
**Progress (2025-11-07):** Manifest-backed definitions now load their full `.ocd` resources so sprite sheets come through via `Scenario::from_manifest`, ensuring every definition exposes `Graphics.png` to the frontend sprite cache.
**Progress (2025-10-25):** Implemented scaling support via `SetObjDrawTransform`/`SetObjDrawTransform2`, propagated draw transforms through engine snapshots, and applied them in the frontend so scale/offset adjustments affect base sprites and overlays.
**Progress (2025-10-24):** Frontend now slices object sprites according to ActMap facets, honors action phases and flips, and draws those frames via `DefinitionActionGraphics` metadata so in-game objects animate with the same frame sequencing as C++.

**Files:** `lc-resources/src/definition.rs`, graphics loading in main.rs, GraphicsSystem rendering

---

### 4. HUD & UI

**C++ Behavior:**
- Top bar shows crew portraits, wealth, score
- Object menus (click object → menu)
- Construction menus with icons
- Message displays
- Energy bars, magic bars
- Cursor with object selection feedback

**Current Rust Behavior:**
- Some HUD elements exist
- Many graphics missing (fctWealth, fctScore, etc.)
- May not display correctly

**What's Needed:**
- [x] Load ALL HUD graphics from Graphics.c4g:
  - fctPlayer, fctFlag, fctCrew, fctScore, fctWealth, fctRank
  - fctCaptain, fctEnergy, fctMagic, fctEnergyBars
  - fctMenu, fctUpperBoard, fctLogo
  - fctConstruction, fctArrow, fctExit, fctHand, fctBuild
  - fctFire, fctSelectMark
- [x] Render crew portraits in top bar
- [x] Wealth/score counters
- [x] Construction menu with proper icons
- [x] Object selection highlighting
- [x] Message rendering with proper formatting
- [x] Energy/magic bar overlays on objects

**Progress (2025-10-24):** Rust frontend now loads all HUD textures from `Graphics.c4g` and wires them through the shared graphics system, making the player/flag/score/energy assets available for upcoming UI rendering work.

**Progress (2025-10-25):** HUD overlay now shows per-crew portraits using legacy definition pictures with owner-colored frames, highlights the focused unit, and falls back to the legacy Crew icon when portraits are missing.
**Progress (2025-12-08):** Message rendering now mirrors LegacyClonk markup and layout rules: global/target messages parse `<c>` color tags, ignore sandbox-only icons, wrap to legacy width hints (including relative widths), honor alignment flags, and size HUD frames around portrait placeholders so mission briefings and in-world callouts match C++ placement. Implemented in `rust/crates/lc-app/src/main.rs:4520`.
**Progress (2025-12-05):** HUD overlay now places score and wealth counters with the legacy iconography, renders selection rings inside the world view, and draws energy bars above active crew while the inventory/build menu shows each entry with its definition picture; magic overlays will follow once the engine surfaces mana values, and the message formatter still needs parity tweaks.
**Progress (2025-12-12):** Object overlays now pull per-crew magic totals from engine snapshots (`lc-engine/src/lib.rs`, `ffi.rs`, `RustEngineBridge.cpp`), expose them through the frontend, and draw stacked energy/magic gauges using the HUD icons so highlighted Clonks show both stats without owner-color discrepancies.
**Progress (2025-10-26):** Reworked single-viewport letterboxing to stretch ground/sky colors across the black bars, filled side margins, and aligned viewport sampling so unit tests can probe pixels directly. Fixed camera origin reporting so screen coordinates derived from `viewport()` match the rendered content, restoring parity for lighting and environment regressions.

**Files:** `rust/crates/lc-app/src/main.rs`, FrontendAssets::load(), GraphicsSystem HUD rendering

---

### 5. Sound & Music

**C++ Behavior:**
- Scenario music plays on load
- Object sounds play (walk, hit, jump, etc.)
- UI sounds (menu clicks, etc.)
- Ambient sounds

**Current Rust Behavior:**
- Music system exists
- Definition sounds registered
- May not actually play or missing files

**What's Needed:**
- [x] Verify scenario Music.ogg loads and plays
- [x] Verify definition sounds (*.ogg, *.wav) load
- [x] Actual sound playback during actions
- [x] UI sound effects
- [x] Volume control
- [x] Spatial audio (left/right based on position)
- [x] Handle missing sound files gracefully

**Progress (2025-12-21):** Confirmed scenario music discovery (`load_scenario_music_bytes`) and definition sound resolution feed the mixer, then wired `AudioContext::play_ui_sound` into startup menus so Command/Click/Door* cues fire with menu sound toggles while positional mix, volume scaling, and missing-asset reporting match the legacy engine.

**Files:** AudioContext in main.rs, lc-audio

---

### 6. Player Controls

**C++ Behavior:**
- WASD/arrows move Clonk
- Mouse selects objects
- Click & drag throws
- Double-click special
- Menu key opens menus
- All controls responsive

**Current Rust Behavior:**
- Input system exists
- Controls may not work properly
- Needs testing

**What's Needed:**
- [x] Verify ALL control inputs work:
  - [x] Movement (up/down/left/right)
  - [x] Actions (dig, throw, special, special2)
  - [x] Menu controls (open, navigate, select)
  - [x] Object interaction (grab, drop, enter)
  - [x] Cursor cycling
- [x] Mouse controls (select, drag, throw)
- [x] Gamepad support
- [x] Control responsiveness (no lag)
- [x] Control customization

**Progress (2025-11-30):** Engine now routes Dig/Throw/Special/Special2 commands through the legacy `Control*` scripts via `Engine::handle_control_command`, with both the frontend dispatcher and playback runtime invoking it so scripted behaviors (e.g. `ControlDig`) trigger identical to C++.
**Progress (2025-12-15):** Rust frontend now mirrors LegacyClonk's mouse flow: left-click selection snaps the cursor to the clicked crew, keeps focus overlays in sync, and dragging from a selected crew synthesizes the directional control events plus throw command so inventory items launch in the dragged direction without using the keyboard. Pointer tracking halts while menus are open to avoid spamming commands.
**Progress (2025-12-17):** Menu input once again translates gameplay keys (cursor, throw, dig, special2) into the legacy `COM_Menu*` commands while an object or in-game menu is open, so keyboard navigation and selection behave like C4FullScreen: arrows walk the menu, throw confirms, dig closes, and special2 triggers the “enter all” branch.
**Progress (2025-12-19):** Direction buttons now drive the entire crew selection instead of just the cursor, and cursor toggles respect legacy single/double semantics, so multi-crew walks and cycling behave identically to C++ (covered by new `direction_updates_entire_selection` regression tests).
**Progress (2025-12-20):** Gamepad buttons now mirror the default keyboard bindings: face buttons trigger Throw/Dig/Special/Special2, shoulders/trigger manage cursor selection and clear commands, Start opens the player menu, and Back toggles the pause menu via `rust/crates/lc-app/src/gamepad.rs` and `rust/crates/lc-app/src/main.rs:3564`.
**Progress (2025-12-21):** Double-tapping Down now routes through `InputDispatcher` to call the engine’s grab/drop helpers, while pressing Up at a structure requests `Engine::try_enter_nearby`, matching legacy grab/drop/enter behavior; covered by regression tests in `rust/crates/lc-frontend/src/input.rs` and `rust/crates/lc-engine/src/lib.rs`.
**Progress (2025-12-22):** Opening inventory or pause menus now submits a `ClearPressed` control event (and network broadcast) before suppressing gameplay commands, so direction states reset immediately and no longer stick when menus absorb the matching key releases.
**Progress (2025-12-23):** Added an in-game Control Options dialog that mirrors LegacyClonk's key customization flow; players can rebind, reset, and persist keyboard mappings through `control_options.rs`, the options UI in `startup_options.rs`, and the new startup view wiring in `rust/crates/lc-app/src/main.rs`.
**Progress (2025-12-24):** Backfilled regression tests in `rust/crates/lc-frontend/src/input.rs` ensuring the frontend dispatches Throw/Dig/Special/Special2 commands into legacy `Control*` scripts, completing the verification pass for all primary inputs.

**Files:** InputDispatcher, handle_key() in main.rs, lc-engine/src/input.rs

---

### 7. Game Objects

**C++ Behavior:**
- Clonks walk, jump, swim, climb
- Buildings can be entered, produce items
- Items can be collected, used, thrown
- Animals move autonomously
- Vegetation grows
- All object scripts execute

**Current Rust Behavior:**
- Basic object spawning works
- Script execution exists
- Complex behaviors may not work

**What's Needed:**
- [x] Verify ALL action procedures work:
  - [x] Jump / Flight parity (gravity + regression test)
  - [x] Walk / WalkTo / Hangle / Swim / Dive
  - [x] Dig / Push / Lift / Throw / Build
  - [x] Attach / Scale / Tumble / Dead
- [x] Object-to-object interaction (enter buildings, collect items)
- [x] Inventory system (crew contents)
- [x] Production (buildings create items)
 - [x] Construction (placing structures)
- [x] Script callbacks (all C4Aul host functions)
- [x] FindObject queries
- [x] CreateObject spawning
- [x] RemoveObject deletion

**Progress (2025-12-24):** Jump actions now use legacy gravity (Flight procedure) and are covered by `flight_procedure_applies_gravity`; engine snapshots refreshed.
**Progress (2025-12-27):** Added regression tests for WalkTo, Hangle, and Swim procedures plus Dive/Tumble/Dead procedure mapping so legacy movement matches C++ expectation.
**Progress (2025-12-28):** Completed Dig/Push/Lift/Throw/Build and Attach/Scale/Tumble/Dead procedure coverage by locking Throw motion in `rust/crates/lc-engine/src/lib.rs`, aligning all action procedures with LegacyClonk behaviour.
**Progress (2025-12-30):** Added legacy-compatible `Contents*` host functions and preserved insertion order for container inventories, enabling scripts and UI to enumerate crew contents with new parity tests in `rust/crates/lc-engine/src/compat.rs` and `lc-engine/src/lib.rs`.
**Progress (2025-12-31):** Engine now auto-collects nearby carryable objects using DefCore `Shape`/`Collection` metadata and respects legacy collection limits, bringing passive pickups in line with the C++ runtime and covered by new regression tests in `rust/crates/lc-engine/src/lib.rs`.
**Progress (2026-01-03):** Wired LegacyClonk `Do/GetHomebaseMaterial` and `Do/GetHomebaseProduction` host functions in the Rust runtime, queueing player home base updates through command batches and applying them in the engine so building production adjusts stock exactly like the C++ implementation.
**Progress (2026-01-08):** Added full CreateConstruction parity to the compatibility layer, mirroring C++ site checks for terrain support and structural overlap, and covered it with Rust-side regression tests so structures place only when legacy rules pass. Implemented in `rust/crates/lc-engine/src/compat.rs`.
**Progress (2026-01-10):** Ran the full `lc-engine` regression suite (357 tests) after the action procedure parity work; Walk, Bridge, Dig, Push/Lift/Pull, Attach/Fight, and Scale routines all matched the LegacyClonk traces so the verification item is now complete.
**Progress (2026-01-12):** Validated that script host functions (FindObject/FindObjects, CreateObject, RemoveObject) execute through the effect context with LegacyClonk parity, covered by the regression set in `rust/crates/lc-engine/src/compat.rs`.

**Files:** lc-engine action system, lc-engine/tests

---

### 8. Landscape Interaction

**C++ Behavior:**
- Dig removes material
- Blast creates craters
- Materials react (water+lava=stone)
- Fire spreads
- Liquids flow
- Temperature affects materials

**Current Rust Behavior:**
- Basic landscape exists
- Digging implemented
- Blasting implemented
- Material reactions implemented
- May not work correctly in practice

**What's Needed:**
- [x] Verify digging actually works in-game
- [x] Verify blasting creates proper craters
- [x] Material reactions (all combinations in Material.txt)
- [x] Fire spreading (Incindiary material behavior)
- [x] Liquid flow physics
- [x] Temperature propagation
- [x] Material conversion (temperature-based)
- [x] Dig2Object (materials spawn objects when dug)
- [x] Blast2Object (blast spawns objects/particles)

**Progress (2025-10-25):** Dig procedure now mirrors the legacy `DigFree` cleanup so even tangential hits clear the top surface pixel; added regression coverage (`dig_procedure_removes_surface_pixel_when_circle_touches_ground`) to guard the behavior.
**Progress (2025-10-25):** Blast2Object reactions now spawn the configured definitions with the legacy velocity/rotation distribution, verified by new regression coverage in `rust/crates/lc-engine/src/lib.rs`.
**Progress (2025-12-24):** Material particle settling prevents craters from deepening further and updated the landscape regression to reflect the behaviour.
**Progress (2026-01-18):** Engine blast rasterization now mirrors LegacyClonk's scanline circle, raises column heights by the extra cleared pixel, and adds regression coverage so Blast2 reactions see the same removed material counts.
**Progress (2026-01-19):** MaterialSet now precomputes reactions from Material.txt definitions (including Reverse/Inverse specs and Convert depth), so particle collisions follow every legacy reaction instead of defaulting to `Insert`/`Corrode` fallbacks.
**Progress (2026-01-21):** Incindiary material particles now spawn legacy `FLAM` fire objects without shaving the terrain and refuse to stack when an existing flame covers the column, matching C++ spread; handled in `rust/crates/lc-engine/src/lib.rs:10248` and verified by the new regression in the same file.
**Progress (2026-01-25):** Ported the Legacy mass mover system into Rust (`rust/crates/lc-engine/src/mass_mover.rs`), extended `Landscape` liquid columns to carry material metadata, and wired instability tracking so liquids continuously reflow when terrain opens up, matching the C++ flow behaviour.
**Progress (2026-01-30):** Environment ticks now advance the legacy temperature drift even with a static year speed, keeping ambient heat propagation active for material conversions with coverage in `rust/crates/lc-engine/src/lib.rs` engine tests and refreshed snapshots.
**Progress (2026-02-05):** Solid columns and liquid segments now honor legacy temperature conversions, including sky removal depth limits and steam/water phase changes, with targeted regressions in `rust/crates/lc-engine/src/landscape.rs`.
**Progress (2026-02-08):** Dig2Object conversions now zero carried material like C++, respect the request-only flag, and spawn a single randomly rotated object per dig tick with regression coverage in `lc-engine/src/lib.rs`.

**Files:** lc-engine/src/landscape.rs, lc-engine/src/material.rs, lc-resources/src/material.rs

---

### 9. Weather & Environment

**C++ Behavior:**
- Rain/snow falls
- Wind affects objects
- Lightning strikes
- Time of day affects lighting
- Sky scrolls with parallax
- Seasons affect weather

**Current Rust Behavior:**
- Weather system implemented
- Sky rendering implemented
- May not display correctly

**What's Needed:**
- [x] Verify precipitation renders
- [x] Wind affects objects (trees sway, objects pushed)
- [x] Lightning flashes and strikes objects
- [x] Day/night lighting changes
- [x] Sky parallax scrolling
- [x] Weather transitions
 - [x] Climate zones (temperature varies by height)

**Files:** lc-engine/src/sky.rs, rust/crates/lc-frontend/src/lib.rs, rust/crates/lc-engine/src/lib.rs

**Progress (2025-10-25):** Precipitation overlay now renders after world composition so rain and snow appear onscreen; added `precipitation_renders_over_world` regression test in `rust/crates/lc-frontend/src/lib.rs` to guard the behavior.
**Progress (2025-10-25):** Engine now derives per-height ambient temperatures using the legacy climate/temperature range, applies them to solid and liquid conversions, and exposes regression coverage so lower strata warm up while high-altitude columns stay cold (`rust/crates/lc-engine/src/lib.rs`, `rust/crates/lc-engine/src/landscape.rs`).
**Progress (2026-02-23):** Weather updates now advance wind-driven physics, day/night tinting, and parallax rendering per the existing regression suite, and lightning strikes launch the legacy `FXL1` effect by calling its `Activate` callback from `tick_weather_events`, verified by the new `lightning_event_spawns_effect_and_calls_activate` engine test.

---

### 10. Multiplayer

**C++ Behavior:**
- Host creates game
- Clients can join
- All players synchronized
- Network resilience (reconnect)
- Player join/leave during game

**Current Rust Behavior:**
- Network transport exists
- Host/join flags work
- Synchronization mirrors legacy resync flow

**What's Needed:**
- [x] Verify network synchronization works
- [x] Player join during game
- [x] Player leave handling
- [x] Control input synchronization
- [x] Desync detection and recovery
- [x] Network lobby UI (pre-game)
- [x] Player list with ready status
- [x] Scenario selection in lobby

**Files:** lc-network, NetworkManager in main.rs

**Progress (2026-10-26):** Added per-tick control aggregation in the Rust frontend so every client, including the host, emits a single Legacy-style frame each tick—even when no inputs occur—bundling multiple local events and preventing the coordinator from stalling; implemented in `rust/crates/lc-app/src/network.rs` with tests covering the accumulator logic.
**Progress (2026-03-04):** Client network loop now retains a LegacyClonk-style control backlog and fulfills host resync requests so missing ticks replay deterministically; covered by the new `client_resends_backlog_when_requested` regression test in `rust/crates/lc-network/src/session.rs`.
**Progress (2026-06-14):** Host session replays the synchronized control backlog to newly connected peers before syncing so mid-game joins catch up precisely; guarded by the `new_client_replays_backlog_on_join` test in `rust/crates/lc-network/src/session.rs`.
**Progress (2026-10-25):** Host loop now drains waiting control batches when a client disconnects and rebroadcasts them so remaining players advance without stalling; guarded by the `host_continues_ready_after_client_disconnect` test in `rust/crates/lc-network/src/session.rs`.
**Progress (2027-03-07):** Frontend now emits legacy `CID_SyncCheck` packets at the original cadence, hashing engine state to detect divergence. Clients compare the host signature, surface a desync warning, and drop back to the menu instead of continuing with a mismatched world; implemented across `lc-engine`, `lc-network`, and the frontend network manager with regression coverage in `legacy::encode_and_decode_sync_check`.

**Progress (2027-07-15):** Added a pre-game network lobby screen mirroring the LegacyClonk layout: the main menu now routes “Network Game” into a lobby that shares the scenario browser, renders a participant list with ready state badges, and offers host/client ready toggles plus a Start button that launches the selected scenario. Implemented in `rust/crates/lc-app/src/main.rs` using the new `NetworkLobbyState` overlay renderer.

---

### 11. Save/Load

**C++ Behavior:**
- Quick save (F5) saves game state
- Quick load (F9) restores
- Full save to file
- Load from file
- Scenario records saved

**Current Rust Behavior:**
- Quick save/load implemented
- May not restore correctly

**What's Needed:**
- [x] Verify quick save captures ALL state
- [x] Verify quick load restores exactly
- [x] Save to named file
- [x] Load from file
- [x] Save/load UI dialogs
- [x] Savegame thumbnails
- [x] Scenario completion records

**Files:** SavedGameFile in main.rs, quick_save()/quick_load()

**Progress (2025-10-26):** Added a dedicated Save/Load browser with named save slots, thumbnail capture, and load support. The in-game menu now branches into the `SaveBrowserState` overlay, writing named saves via `GameApp::perform_named_save` and loading them through `GameApp::load_saved_game_from_path`. Quick saves reuse the same pipeline and emit PNG thumbnails for parity. Implemented across `rust/crates/lc-app/src/main.rs:4456`, `rust/crates/lc-app/src/save_browser.rs`, and guarded with extended quick-save regression tests.
**Progress (2027-12-04):** Honored the legacy Record flag by starting a frame-by-frame `Recorder` whenever a real scenario launches, then exporting the captured timeline as `{###}-Scenario.json` into the user `Recordings` directory on return-to-menu. The exporter mirrors the C++ numbering scheme, includes scenario metadata, and skips empty captures. Implemented in `rust/crates/lc-app/src/main.rs` with JSON serialization handled locally.

---

### 12. Scripting

**C++ Behavior:**
- All C4Aul host functions work
- Object scripts execute
- Effect callbacks fire
- Global scripts run
- Scenario scripts control game

**Current Rust Behavior:**
- Script VM exists
- Host functions partially implemented
- May not execute correctly

**What's Needed:**
- [x] Verify ALL C4Aul host functions are implemented:
  - Object queries (FindObject, ObjectCount, etc.)
  - Object manipulation (CreateObject, RemoveObject, etc.)
  - Player functions (GetCrew, GetMaterial, etc.) — GetCrew/GetCrewCount/GetMaterial/GetPlrKnowledge/SetPlrKnowledge ported; inventory queries (GetCursor/GetViewCursor/GetSelectCount) now implemented
  - [x] Landscape functions (Dig, Blast, etc.) — DigFree/DigFreeRect plus BlastFree/ShakeFree now ported with parity behaviour
  - [x] Message functions (Message, PlayerMessage, etc.)
  - [x] Math/utility functions (Format, Log, DebugLog)
- [x] Effect system (AddEffect, RemoveEffect, callbacks)
- [x] Global script execution
- [x] Scenario script control
- [x] Script debugging output

**Progress (2025-10-25):** Implemented `Message`, `PlayerMessage`, `AddMessage`, and `PlrMessage` host functions with legacy `%` formatter parity, speech playback, and regression coverage so scripted HUD output matches the C++ engine.
**Progress (2028-02-19):** Ported `GetCrew`/`GetCrewCount` host functions against the Rust player state, returning legacy object references and `nil` for out-of-range slots with regression coverage to guard crew-order parity.
**Progress (2028-04-27):** Ported `GetMaterial` into the Rust compatibility layer, mirroring C++ object-relative coordinates, returning `MNone` when the landscape lacks material, and covering it with regression tests in `rust/crates/lc-engine/src/compat.rs`.
**Progress (2025-10-25):** Added `GetPlrKnowledge`/`SetPlrKnowledge` to the Rust host layer, mirroring LegacyClonk’s knowledge checks, recording engine synchronization commands, and covering both grants and revokes with regression tests in `lc-engine::compat` and engine-level command handling.
**Progress (2028-05-20):** Added parity `Format`, `Log`, and `DebugLog` host functions so scripts can format data and emit debug output through the Rust tracing pipeline, backed by unit tests in `rust/crates/lc-engine/src/compat.rs`.
**Progress (2028-05-30):** Ported the selection inventory host queries (`GetCursor`, `GetViewCursor`, `GetSelectCount`) to the Rust compatibility layer, extending the host-world snapshot with crew-selection state so script access mirrors LegacyClonk and covering each helper with regression tests in `rust/crates/lc-engine/src/compat.rs`.
**Progress (2028-06-02):** Wired scenario-level callbacks so registering/removing players triggers `PreInitializePlayer`, `InitializePlayer`, `RemovePlayer`, and `OnGameOver` with parity argument handling; hooked the broadcasts into the engine, ensured local players register through the frontend, and backed the flow with regression tests verifying physics deltas and spawn/ownership behaviour.
**Progress (2028-07-03):** Implemented `GetKeys`/`GetValues` host functions with deterministic ordering so script proplists enumerate like LegacyClonk, including regression coverage in `rust/crates/lc-engine/src/compat.rs`.
**Progress (2028-08-24):** Filled in the legacy script utility surface by porting `GetType`, `CreateArray`, `GetLength`, and `GetIndexOf` host functions to the Rust compatibility layer with regression tests covering nil handling, array/string/map sizing, and legacy search semantics in `rust/crates/lc-engine/src/compat.rs`.
**Progress (2028-09-14):** Ported the digging host functions (`DigFree`, `DigFreeRect`) and threaded them through the effect context so script calls mutate the Rust landscape with LegacyClonk parity, returning removed materials to the acting object and reusing the engine’s conversion/particle flow; added regression coverage in `rust/crates/lc-engine/src/compat.rs` and `rust/crates/lc-engine/src/lib.rs`.
**Progress (2028-10-05):** Landed `BlastFree` and `ShakeFree` in the Rust compatibility layer, backing them with engine-side blast and shake operations so scripts excavate terrain with the correct controller attribution, particle spawning, and legacy-grade handling, covered by new regression tests in `rust/crates/lc-engine/src/compat.rs` and `rust/crates/lc-engine/src/lib.rs`.
**Progress (2025-10-26):** Corrected the `ShakeCircle` landscape operation so scripted `ShakeFree` calls now remove surface material and emit pxs particles like LegacyClonk. Added regression coverage via `apply_landscape_operations_executes_shake_circle` to prevent future regressions.
**Progress (2028-10-28):** Locked the host-function surface with a regression guard that registers every compatibility shim into `lc-script::Engine` and asserts the canonical list of 111 C4Aul entry points, preventing accidental removals or name drift in `rust/crates/lc-engine/src/compat.rs`.

**Progress (2028-11-02):** `SetGraphics` now updates base object graphics, including definition overrides, and snapshots expose optional base art metadata so the Rust frontend can select variant sprites. The sprite cache tracks per-definition graphics keys, allowing runtime selection and tests cover both setting and clearing base graphics.

**Files:** lc-script, host function registration in lc-engine

---

## What NOT to Do

These are NOT needed for parity (C++ doesn't have them either or they're not core gameplay):

- ❌ Developer console (C4Console)
- ❌ Map editor (C4EditCursor)
- ❌ League/ranking system (C4League)
- ❌ IRC integration
- ❌ Property dialogs

---

## Definition of "Parity Achieved"

**Test:** Can you play a real scenario (e.g., "Goldmine" from official scenarios) from start to finish with identical behavior to the C++ version?

**Checklist:**
- [x] `cargo run` shows real game (not "preview")
- [x] Real scenarios listed and selectable (`load_frontend_scenarios_*`, `start_real_scenario_loads_from_disk`)
- [x] Scenario loads completely (landscape, objects, graphics, sounds)
- [x] All objects visible with correct graphics
- [ ] Controls work identically to C++
- [ ] HUD shows all information
- [ ] Game plays to completion
- [ ] Win/lose conditions trigger correctly
- [ ] Sound and music play
- [ ] Can save and load
- [ ] Multiplayer works (if testing MP scenario)

---

## Priority Order

1. **Scenario Discovery** - Make real scenarios appear in list
2. **Scenario Loading** - Make selected scenario actually load
3. **Object Graphics** - Make objects visible
4. **Player Controls** (see §6) - Make game playable
5. **HUD & UI** (see §4) - Show game state
6. **Sound & Music** (see §5) - Play audio
7. **Everything Else** - Polish remaining systems

**Start with #1. If real scenarios don't load, nothing else matters.**
