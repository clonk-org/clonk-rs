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
- [ ] Complete Scenario.txt parsing (verify all fields)
- [x] Map.bmp landscape loading
- [x] Objects.txt initial object spawning (Scenario.txt crew entries now covered)
- [ ] Verify all definitions load correctly
- [x] Proper error handling (don't fall back silently)
- [ ] Loading screen with progress
- [ ] Scenario intro text/mission briefing

**Progress (2025-10-27):** Runtime now stops falling back to the sandbox when a disk scenario fails to load or apply; `GameApp::start_scenario` keeps players in the menu, surfaces the failure in `status_text`, and only swaps the running engine state once the legacy scenario applies cleanly.
**Progress (2025-10-26):** Legacy loader now parses `Objects.txt`, creating spawn configs with explicit object ids, status, owner, position/velocity, crew state, action info, and resolves containers (including via `Contents=` fallbacks) so full scenario object graphs appear without sandbox fallback.
**Progress (2025-10-24):** Ported Scenario.txt weather and landscape physics parsing so gravity, wind variation, climate, precipitation, and disaster levels feed straight into the Rust engine instead of defaulting to sandbox values.
**Progress (2025-10-24):** Legacy `Map.bmp` columns now translate into zoomed `Landscape` surface heights (with a flat fallback when the bitmap is missing) so real scenarios boot with terrain instead of the sandbox void.

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
- [ ] Verify Graphics.png loads for ALL definitions
- [ ] Action frame graphics (ActMap procedure graphics)
- [ ] Overlay graphics (ClonkGraphics, tool overlays)
- [ ] ColorByOwner player color modulation
- [ ] Animation frame cycling
- [ ] Z-order rendering (background → objects → overlays)
- [ ] Rotation and scaling for graphics

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
- [ ] Load ALL HUD graphics from Graphics.c4g:
  - fctPlayer, fctFlag, fctCrew, fctScore, fctWealth, fctRank
  - fctCaptain, fctEnergy, fctMagic, fctEnergyBars
  - fctMenu, fctUpperBoard, fctLogo
  - fctConstruction, fctArrow, fctExit, fctHand, fctBuild
  - fctFire, fctSelectMark
- [ ] Render crew portraits in top bar
- [ ] Wealth/score counters
- [ ] Construction menu with proper icons
- [ ] Object selection highlighting
- [ ] Message rendering with proper formatting
- [ ] Energy/magic bar overlays on objects

**Files:** FrontendAssets::load(), GraphicsSystem HUD rendering

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
- [ ] Verify scenario Music.ogg loads and plays
- [ ] Verify definition sounds (*.ogg, *.wav) load
- [ ] Actual sound playback during actions
- [ ] UI sound effects
- [ ] Volume control
- [ ] Spatial audio (left/right based on position)
- [ ] Handle missing sound files gracefully

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
  - Movement (up/down/left/right)
  - Actions (dig, throw, special, special2)
  - Menu controls (open, navigate, select)
  - Object interaction (grab, drop, enter)
  - Cursor cycling
- [ ] Mouse controls (select, drag, throw)
- [ ] Gamepad support
- [ ] Control responsiveness (no lag)
- [ ] Control customization

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
4. **Controls** - Make game playable
5. **HUD** - Show game state
6. **Sound** - Play audio
7. **Everything Else** - Polish remaining systems

**Start with #1. If real scenarios don't load, nothing else matters.**
