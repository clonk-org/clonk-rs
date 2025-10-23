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
  - ✅ PlayerMenu command now toggles the Rust pause menu; object/build menus still rely on future C4Menu port

- ✅ **Viewport System** (C4Viewport.cpp 🔗 Camera parity improved)
  - ✅ Multiple simultaneous viewports per player with dynamic split-screen layout
  - ✅ Split-screen multiplayer rendering fed by engine player viewports
  - ✅ Zoom scaling, camera smoothing, and automatic letterboxing for aspect mismatches

- ⚠️ **Menu System** (C4Menu.h 🔗 Partial — player pause menu implemented)
  - ✅ Player pause/main menu ported in Rust frontend (resume, quick save/load, abort)
  - ❌ Context menus, build/inventory menus, and script-driven callbacks still unported

- ❌ **Message System** (C4GameMessage.cpp 🔗 No message display)
  - In-game messages, tutorials, mission objectives
  - Per-player message queues
  - Message positioning and formatting

- ⚠️ **Material System** (C4Material.cpp 🔗 lc-engine missing materials)
  - Terrain materials (earth, rock, gold, water, lava, acid, etc.)
  - Material physics (density, friction, temperature reactions)
  - Material conversion and reactions
  - ✅ Definitions now load from install groups; runtime behaviour still missing

- ❌ **Weather & Sky** (C4Weather.cpp, C4Sky.cpp 🔗 Not ported)
  - Dynamic weather (rain, snow, wind, lightning)
  - Sky gradients, stars, celestial bodies
  - Day/night cycles with lighting

- ❌ **Particles (PXS)** (C4PXS.cpp 🔗 Basic particle system only)
  - Material pixels (smoke, fire, water droplets, sparks)
  - Particle-terrain interaction
  - Particle-object collision

### Category 2: Game Loop & Integration (INCOMPLETE)
- ⚠️  **Scenario Loading** (works but defaults to demo)
  - `Scenario::load_from_path()` exists but fails in practice
  - Falls back to hardcoded "Rust Sandbox" with synthetic Walker
  - Real scenario discovery exists but doesn't integrate properly

- ⚠️  **Definition System** (partial - C4Def.cpp)
  - Can load object definitions but limited to hardcoded Walker
  - Missing definition graphics, sounds, ActMap integration
  - No DefCore.txt parsing for real game objects

- ⚠️  **Graphics Resources** (C4GraphicsResource.cpp 🔗 Minimal)
  - Progress: Fonts, startup backgrounds, and button textures now sourced from Graphics.c4g; walker sprite mapped from crew assets
  - Missing: Comprehensive definition graphics pipeline, cursor graphics, broader GUI icon sets
  - Has: Basic surface rendering only

- ⚠️  **Landscape Features** (C4Landscape.cpp is 88KB, lc-engine has basics only)
  - Missing: Landscape modification (digging, blasting, incineration)
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
1. Port C4Player for real player management — ✅ Teams/home-base parity, ✅ control UX including PlayerMenu pause toggle (object/build menus still depend on C4Menu)
2. ✅ Port C4Viewport for proper camera, zoom, split-screen — smooth camera easing, zoom scaling, and letterboxing now mirror the C++ behaviour
3. Port C4Menu for object interaction menus — ⚠️ player pause menu landed; context/object menus pending
4. Port C4GameMessage for mission text and objectives

### Phase 3: Game Content & Polish
1. Port full C4Landscape material modification system
2. Port C4Weather and C4Sky for atmosphere
3. Port C4PXS particle system for effects
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
- ✅ Material definitions load from install archives (lc-resources::material + Engine MaterialSet)
- ✅ Player registry + HUD overlay surface real player metadata (names, wealth, cursor) sourced from engine snapshots
- ✅ Cursor cycling and selection toggles now respect classic COM_Single/COM_Double timing with network serialization parity
- ✅ Team home-base rule produces and syncs materials between teammates (C4Player::ExecHomeBaseProduction)
- ⚠️ In-game pause menu implemented (resume, quick save/load, abort) while gameplay continues; object/build menus still pending
- ⚠️ Experimental: `.c4s` scenarios boot with real definitions and scripts when install data is available (lands in running state, but full player UX still limited)

**What Doesn't Work (Real Game Requirements):**
- ❌ `cargo run` shows **demo only** (window title: "LegacyClonk (Rust preview)")
- ❌ No real scenarios load (falls back to hardcoded "Rust Sandbox")
- ❌ No real object definitions (only synthetic Walker)
- ❌ No real in-game definition graphics (walker sprite mapping only; most objects still fall back)
- ❌ No real audio (synthetic tones for missing sounds)
- ⚠️ In-game menus limited to new pause menu (no context/build/object menus yet)
- ❌ Materials still unused at runtime (no terrain types, no mining, no reactions despite definitions loading)
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
