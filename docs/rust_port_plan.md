# LegacyClonk Rust Port - Exact Parity Requirements

## Current Problem

Running `cargo run` shows ONLY a "Sandbox Scenario" fallback. Real scenarios from installation are not discovered/loaded properly. The game is not playable with real content.

## Required for Exact C++ Parity

### 1. Startup & Scenario Discovery

**C++ Behavior:**
- Shows main menu on startup
- Lists real scenarios from install directories
- Scenarios are organized in folders
- Can browse and select any scenario

**Current Rust Behavior:**
- Shows scenario browser immediately (no main menu)
- Shows "Rust Sandbox" fallback only
- Real scenarios not appearing in list

**What's Needed:**
- [x] Fix scenario discovery to actually find real `.c4s` files from installation
- [x] Main menu (C4StartupMainDlg equivalent) with proper navigation
- [x] Remove/hide sandbox fallback when real scenarios exist
- [x] Scenario folder navigation
- [x] Scenario preview images
- [x] Window title: "Clonk Rust" not "LegacyClonk (Rust preview)"

**Progress (2025-10-25):** Rust frontend still scans install roots per directory, tolerates missing assets, and decodes BMP previews so real scenarios populate the browser without sandbox fallback. Added a dedicated main menu with the LegacyClonk big-button layout, participants list, and reliable navigation (Back entry/Escape) back from the scenario browser; “Local Game” flows into the browser while other options surface placeholder status until their dialogs are ported.

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

**Progress (2025-11-26):** Frontend now respects per-object rotations from `SetR`/snapshots when rendering action sprites and overlays, matching LegacyClonk's spinning/tilting objects (scaling still pending).
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
- [ ] Verify ALL control inputs work:
  - [x] Movement (up/down/left/right)
  - [x] Actions (dig, throw, special, special2)
  - [x] Menu controls (open, navigate, select)
  - [ ] Object interaction (grab, drop, enter)
  - [x] Cursor cycling
- [x] Mouse controls (select, drag, throw)
- [x] Gamepad support
- [ ] Control responsiveness (no lag)
- [ ] Control customization

**Progress (2025-11-30):** Engine now routes Dig/Throw/Special/Special2 commands through the legacy `Control*` scripts via `Engine::handle_control_command`, with both the frontend dispatcher and playback runtime invoking it so scripted behaviors (e.g. `ControlDig`) trigger identical to C++.
**Progress (2025-12-15):** Rust frontend now mirrors LegacyClonk's mouse flow: left-click selection snaps the cursor to the clicked crew, keeps focus overlays in sync, and dragging from a selected crew synthesizes the directional control events plus throw command so inventory items launch in the dragged direction without using the keyboard. Pointer tracking halts while menus are open to avoid spamming commands.
**Progress (2025-12-17):** Menu input once again translates gameplay keys (cursor, throw, dig, special2) into the legacy `COM_Menu*` commands while an object or in-game menu is open, so keyboard navigation and selection behave like C4FullScreen: arrows walk the menu, throw confirms, dig closes, and special2 triggers the “enter all” branch.
**Progress (2025-12-19):** Direction buttons now drive the entire crew selection instead of just the cursor, and cursor toggles respect legacy single/double semantics, so multi-crew walks and cycling behave identically to C++ (covered by new `direction_updates_entire_selection` regression tests).
**Progress (2025-12-20):** Gamepad buttons now mirror the default keyboard bindings: face buttons trigger Throw/Dig/Special/Special2, shoulders/trigger manage cursor selection and clear commands, Start opens the player menu, and Back toggles the pause menu via `rust/crates/lc-app/src/gamepad.rs` and `rust/crates/lc-app/src/main.rs:3564`.

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
- [ ] Verify ALL action procedures work:
  - Walk, Jump, WalkTo, Hangle, Swim, Dive
  - Dig, Push, Lift, Throw, Build
  - Attach, Scale, Tumble, Dead
- [ ] Object-to-object interaction (enter buildings, collect items)
- [ ] Inventory system (crew contents)
- [ ] Production (buildings create items)
- [ ] Construction (placing structures)
- [ ] Script callbacks (all C4Aul host functions)
- [ ] FindObject queries
- [ ] CreateObject spawning
- [ ] RemoveObject deletion

**Files:** lc-engine action system, lc-script host functions

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
- [ ] Verify digging actually works in-game
- [ ] Verify blasting creates proper craters
- [ ] Material reactions (all combinations in Material.txt)
- [ ] Fire spreading (Incindiary material behavior)
- [ ] Liquid flow physics
- [ ] Temperature propagation
- [ ] Material conversion (temperature-based)
- [ ] Dig2Object (materials spawn objects when dug)
- [ ] Blast2Object (blast spawns objects/particles)

**Files:** lc-engine/src/landscape.rs, lc-engine/src/material.rs

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
- [ ] Verify precipitation renders
- [ ] Wind affects objects (trees sway, objects pushed)
- [ ] Lightning flashes and strikes objects
- [ ] Day/night lighting changes
- [ ] Sky parallax scrolling
- [ ] Weather transitions
- [ ] Climate zones (temperature varies by height)

**Files:** lc-engine/src/sky.rs, SkyRenderState in lc-frontend

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
- Synchronization may not work correctly

**What's Needed:**
- [ ] Verify network synchronization works
- [ ] Player join during game
- [ ] Player leave handling
- [ ] Control input synchronization
- [ ] Desync detection and recovery
- [ ] Network lobby UI (pre-game)
- [ ] Player list with ready status
- [ ] Scenario selection in lobby

**Files:** lc-network, NetworkManager in main.rs

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
- [ ] Verify quick save captures ALL state
- [ ] Verify quick load restores exactly
- [ ] Save to named file
- [ ] Load from file
- [ ] Save/load UI dialogs
- [ ] Savegame thumbnails
- [ ] Scenario completion records

**Files:** SavedGameFile in main.rs, quick_save()/quick_load()

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
- [ ] Verify ALL C4Aul host functions are implemented:
  - Object queries (FindObject, ObjectCount, etc.)
  - Object manipulation (CreateObject, RemoveObject, etc.)
  - Player functions (GetCrew, GetMaterial, etc.)
  - Landscape functions (Dig, Blast, etc.)
  - Message functions (Message, PlayerMessage, etc.)
  - Math/utility functions
- [ ] Effect system (AddEffect, RemoveEffect, callbacks)
- [ ] Global script execution
- [ ] Scenario script control
- [ ] Script debugging output

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
- [ ] `cargo run` shows real game (not "preview")
- [ ] Real scenarios listed and selectable
- [ ] Scenario loads completely (landscape, objects, graphics, sounds)
- [ ] All objects visible with correct graphics
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
