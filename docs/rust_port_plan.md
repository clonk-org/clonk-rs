# LegacyClonk Rust Port Plan

## Status: Prototype - Core Systems Ported, Missing Critical Game Features

**Reality Check:** `cargo run` launches a **preview/demo** titled "LegacyClonk (Rust preview)" showing a hardcoded "Rust Sandbox" with synthetic Walker objects, placeholder graphics, and generated audio tones. This is **NOT the real game.**

**C++ Codebase:** 454 source files implementing the full game
**Rust Port:** 95 files implementing core subsystems only

**What Works:** Engine simulation, basic graphics, audio playback, network transport, C4Group file parsing, player registry feeding HUD overlays
**What's Missing:** Most of the actual game (see gaps below)

---

## Architecture

**Rust Crates:**
- `lc-engine` - Core game engine (physics, objects, landscape, actions, effects, recording/playback)
- `lc-script` - C4Aul script VM port
- `lc-graphics` - Surface rendering and pixel manipulation
- `lc-audio` - Audio decoder + mixer (music/SFX channels)
- `lc-frontend` - Startup menu, scenario browser, input dispatcher
- `lc-gui` - Widget system for UI overlays
- `lc-resources` - C4Group file loading and scenario discovery
- `lc-network` - Multiplayer transport (handshake, lobby, control dispatch, sync)
- `lc-platform` - Platform abstractions and path discovery
- `lc-core` - Shared types and config bridge
- `lc-app` - **Main game binary** (integrates all subsystems)
- `lc-game` - Launcher wrapper

**C++ Codebase:** Legacy implementation in `src/` no longer required for Rust runtime.

---

## Major Missing Features (C++ → Rust Parity Gaps)

### Category 1: Game Systems (NOT PORTED)
- ⚠️ **Player Management** (C4Player.cpp 🔗 Runtime APIs landed; strategic features pending)
  - ✅ Engine exposes full player registry (join/leave, wealth, knowledge, inventory, cursor, per-player viewports)
  - ✅ Simulation snapshots + HUD now surface player names/wealth/cursor for overlays
  - ✅ Team rule + home-base production synced across players (C4RULE_TeamHomebase parity)
  - ✅ Advanced control UX landed: Rust input dispatcher now drives cursor cycling, selection toggles, and serializes player menu commands like the C++ runtime
  - ✅ PlayerMenu command now toggles the Rust pause menu; inventory/build menus and scripted context submenus handled by the Rust UI

- ✅ **Viewport System** (C4Viewport.cpp 🔗 Camera parity improved)
  - ✅ Multiple simultaneous viewports per player with dynamic split-screen layout
  - ✅ Split-screen multiplayer rendering fed by engine player viewports
  - ✅ Zoom scaling, camera smoothing, and automatic letterboxing for aspect mismatches

- ✅ **Menu System** (C4Menu.h 🔗 Pause, inventory, and context menus implemented)
  - ✅ Player pause/main menu ported in Rust frontend (resume, quick save/load, abort)
  - ✅ Player inventory menu renders crew contents, surfaces selection feedback, and dispatches Focus/DropAll through definition `MenuCommand`
  - ✅ Build menu consumes home base stock, hands items to crew, and scripted context menus now flow through definition `MenuEntries` callbacks

- ✅ **Message System** (C4GameMessage.cpp 🔗 Implemented)
  - Global and target-attached messages render in the HUD with legacy lifetimes and per-player visibility
  - Per-player filtering matches C++ semantics
  - Remaining polish: frame decorations/portraits not yet drawn

- ⚠️ **Material System** (C4Material.cpp 🔗 runtime parity incomplete)
  - ✅ Material definitions now populate the Rust `MaterialSet` with density/friction metadata and canonical lookup
  - ✅ Landscapes retain per-column material ids and collisions now apply material friction to object velocity/vertex data
  - ✅ Conversion/reaction logic and temperature-driven evaluation mirrored in `MaterialSet::reaction` / `evaluate_temperature_conversion`, and landscapes now apply temperature-driven conversions each tick
  - ✅ ActMap `DigFree` values feed the Rust runtime; dig procedures now carve landscape columns when materials permit excavation

- ✅ **Weather & Sky** (C4Weather.cpp, C4Sky.cpp 🔗 Ported)
  - ✅ Dynamic precipitation honors scenario settings (rain vs. snow), wind variation, and emits lightning events for HUD feedback
  - ✅ Scenario sky textures render with parallax, wind-driven drift, and gradient fallback when no assets are present
  - ✅ Gamma/day-night curves blend seasonal ramps into sky lighting

- ✅ **Particles (PXS)** (C4PXS.cpp 🔗 Material pixel system landed)
  - Gravity- and wind-driven material pixels now emit during blasts, drift, settle, and obey Material.txt reactions (insert, poof, corrode, incinerate)
  - Landscape heights update from deposits/removals, and colliding objects inherit material friction to slow horizontal motion

### Category 2: Game Loop & Integration (INCOMPLETE)
- ✅  **Scenario Loading** (real scenarios load when assets are present)
  - Legacy resolver now searches packed install groups and ancestor folders before falling back
  - Sandbox preview remains only as safety net when required assets are missing

- ⚠️  **Definition System** (partial - C4Def.cpp)
  - ✅ Sandbox fallback now loads install definitions (Clonk et al.) instead of the synthetic Walker
  - ✅ DefCore now surfaces value/mass data (and picture rect metadata) to the engine; inventory UI shows per-item stats
  - ✅ Definition graphics pipeline now decodes `Graphics*.png` assets, caches picture sprites in the engine, and feeds the frontend sprite map so objects render with real icons
  - ✅ Definition sound library registration now feeds definition groups into the audio resolver so real samples play instead of synthetic tone fallbacks

- ⚠️  **Graphics Resources** (C4GraphicsResource.cpp 🔗 Minimal)
  - Progress: Fonts, startup backgrounds, and button textures now sourced from Graphics.c4g; walker sprite mapped from crew assets
  - Missing: Comprehensive definition graphics pipeline, cursor graphics, broader GUI icon sets
  - Has: Basic surface rendering only

- ⚠️  **Landscape Features** (C4Landscape.cpp is 88KB, lc-engine has basics only)
  - Progress: Digging raises surface depths (DigFree); blasting now carves heightmap craters honoring BlastFree and tracking removal stats; incineration hooks respect Inflammable. Scripted deformation still outstanding.
  - Missing: Pixel-perfect material queries
  - Missing: Landscape script integration (Earthquake, Incinerate, etc.)
  - Has: Basic heightmap collision only

### Category 3: Multiplayer & Networking (INCOMPLETE)
- ⚠️  **Lobby System** (C4GameLobby.cpp 38KB 🔗 Not ported)
  - Pre-game lobby with player join, team selection
  - Scenario parameter configuration
  - Resource synchronization before game start

- ⚠️  **Network Clients** (C4Network2Client.cpp 🔗 Basic transport only)
  - Client lifecycle management
  - Client authentication and permissions
  - Client-specific state tracking

- ❌ **League/Ranking** (C4League.h 🔗 Not ported)
  - Online matchmaking, account system
  - Game statistics and player rankings
  - League game recording

### Category 4: Editor & Development (NOT PORTED)
- ❌ **In-Game Console** (C4Console.cpp 🔗 Not ported)
  - Script console for live debugging
  - Object inspection and manipulation
  - Command execution during gameplay

- ❌ **Edit Cursor** (C4EditCursor.cpp 🔗 Not ported)
  - Live object placement and editing
  - Landscape editing tools
  - Property modification

### Category 5: Demo/Fallback Systems
- ❌ **Synthetic Audio** (`lc-app/main.rs:601-617`) - Generates sine wave tones for missing sounds
- ❌ **Placeholder Previews** (`lc-app/main.rs:1186-1241`) - Procedural gradient images
- ❌ **Hardcoded Walker** (`lc-app/main.rs:2383-2414`) - Fixed spawn at (240, 180)
- ❌ **Fallback Ground Height** (`lc-app/main.rs:55+`) - Hardcoded 360px default
- ❌ **Sandbox Scenario** (`lc-app/main.rs:1371-1386`) - "Rust Sandbox" instead of real game

---

## Path to Real Game (NOT Fallback Removal)

The documentation previously focused on "removing fallbacks." **This is backwards.** The fallbacks exist *because* the real game features are missing. The path forward is:

### Phase 1: Core Game Integration
1. Port C4Def system to load real object definitions from Definition folders — ✅ Added `lc-resources::definition` (DefCore/ActMap/script parsing) with `Engine::Definition::from_resource`; sandbox now attempts to boot with real Clonk definitions when present (asset loading + scenario wiring still pending)
2. ✅ Port Graphics.c4g loading for object graphics, fonts, UI elements — startup menu and UI components now render real assets from Graphics.c4g and Endeavour.ttf; walker sprite mapping added as first object graphic
3. ✅ Port Material.txt system for terrain material definitions — `lc-resources::material` parses Material.txt/®.c4m files and `Engine` tracks a MaterialSet
4. ✅ Make real scenarios load with install definitions (gameplay still limited until player/viewport/menu systems land)

### Phase 2: Essential Gameplay Systems
1. Port C4Player for real player management — ✅ Teams/home-base parity, ✅ control UX including PlayerMenu pause toggle, ✅ inventory/build menus and scripted context menus handled in Rust UI via C4Menu
2. ✅ Port C4Viewport for proper camera, zoom, split-screen — smooth camera easing, zoom scaling, and letterboxing now mirror the C++ behaviour
3. Port C4Menu for object interaction menus — ✅ player pause menu landed; inventory/build flows now wired through Rust UI; scripted context/object menus run via definition callbacks
4. ✅ Port C4GameMessage for mission text and objectives — HUD now renders script-driven messages; decorative frames/portraits pending future polish

### Phase 3: Game Content & Polish
1. ✅ Port full C4Landscape material modification system — DigFree terrain carving plus BlastFree/incineration parity landed; Blast2Object + Blast2PXS reactions now emit definition-driven spawns and particles alongside scripted landscape commands
2. ✅ Port C4Weather and C4Sky for atmosphere — sky assets now load from scenarios/Graphics.c4g with wind-driven parallax, precipitation reacts to temperature (rain vs. snow), and lightning events tie into HUD lighting
3. ✅ Port C4PXS particle system for effects — engine now simulates material PXS with landscape reactions and object friction passthrough
4. Integrate real game assets (objects, scenarios, graphics, sounds)

### Phase 4: Remove Fallbacks
**Only after Phases 1-3:** Remove synthetic audio, placeholder previews, sandbox scenario, hardcoded defaults

**Current fallbacks are symptoms, not the disease.** Fix the missing game systems first.

---

## Current Implementation Status

**What Actually Works:**
- ✅ `cargo test` passes (unit tests for ported subsystems)
- ✅ `cargo xtask engine-snapshots verify` (engine simulation determinism)
- ✅ Basic engine ticks with physics simulation
- ✅ C4Group file format parsing (can read .c4g/.c4s/.c4f files)
- ✅ Network transport layer (`--host`/`--join` flags exist)
- ✅ Audio playback (music/SFX mixing)
- ✅ Window creation and basic input handling
- ✅ Startup menu renders Graphics.c4g backgrounds and buttons with Endeavour.ttf fonts (no longer flat placeholders)
- ✅ Scenario loader resolves legacy definition paths from packed install groups before falling back to the sandbox preview
- ✅ Sandbox fallback auto-loads install object definitions when install data is available (spawns real Clonks instead of the synthetic Walker)
- ✅ Material definitions load from install archives (lc-resources::material + Engine MaterialSet) and collisions now consume material friction metadata
- ✅ Landscape blast operations carve heightmap craters respecting BlastFree and expose per-material removal totals for follow-up effects
- ✅ Player registry + HUD overlay surface real player metadata (names, wealth, cursor) sourced from engine snapshots
- ✅ Cursor cycling and selection toggles now respect classic COM_Single/COM_Double timing with network serialization parity
- ✅ Team home-base rule produces and syncs materials between teammates (C4Player::ExecHomeBaseProduction)
- ✅ In-game pause menu implemented (resume, quick save/load, abort) while gameplay continues; scripted context menus now surface definition-driven actions
- ⚠️ Player inventory menu mirrors crew contents with navigation/selection feedback, shows value/mass stats, and now routes Focus/DropAll/build deliveries through definition `MenuCommand`
- ⚠️ Experimental: `.c4s` scenarios boot with real definitions and scripts when install data is available (lands in running state, but full player UX still limited)
- ✅ HUD renders global and object-attached messages using script `CustomMessage` calls with legacy timeouts
- ✅ Sky background in the Rust frontend now blends a day/night gradient derived from engine time-of-day and ambient temperature

**What Doesn't Work (Real Game Requirements):**
- ❌ `cargo run` shows **demo only** (window title: "LegacyClonk (Rust preview)")
- ⚠️ Definition runtime still incomplete (no scripted context menus, limited metadata, missing graphics/sound wiring)
- ❌ No real in-game definition graphics (walker sprite mapping only; most objects still fall back)
- ⚠️ Audio assets still incomplete (missing samples fall back to synthesized tones even though definition libraries now register)
- ⚠️ Message frames/portraits still missing (text-only rendering today)
- ⚠️ Material runtime still misses mining reactions such as BlastShiftTo shifts, Dig2Object conversions, and particle-to-terrain feedback despite Blast2Object/PXS reactions now firing
- ❌ No multiplayer lobby (transport exists but no game coordination)

---

## Success Criteria

**Real game achieved when `cargo run` shows:**

1. **Actual Startup Menu**
   - Real scenario list from installation directory
   - Actual scenario preview screenshots (not procedural gradients)
   - Working scenario folders and navigation

2. **Real Scenarios Load and Play**
   - Load object definitions from scenario/system folders
   - Display object graphics from Graphics.c4g
   - Play actual scenario music and sound effects
   - Spawn real game objects (Clonks, structures, animals, etc.)

3. **Full Game Loop**
   - Player management (join, crew selection, inventory)
   - In-game menus (build, buy, interaction)
   - Material terrain system (dig earth, mine gold, etc.)
   - Weather and environmental effects
   - Mission objectives and game messages

4. **Multiplayer Works**
   - Pre-game lobby with player join/leave
   - Synchronized object spawning and control
   - Network resilience and reconnection
   - Spectator mode and replays

5. **No Synthetic Content**
   - Zero fallback scenarios, objects, graphics, or audio
   - All assets loaded from real game files
   - Behavior indistinguishable from C++ version
