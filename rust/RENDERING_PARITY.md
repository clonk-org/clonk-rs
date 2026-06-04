# Rendering-layer parity plan

> Goal: bring the Rust render path to parity with the C++ engine. Living doc.
> Started 2026-05-30.

## What "parity" means here (and what it can't)

The C++ engine renders through **OpenGL** (`src/StdGL.cpp`) — GPU output that is
not bit-identical across drivers/GPUs. The Rust port renders through a **CPU
framebuffer** (`pixels`/wgpu in `lc-app`, software blits in `lc-graphics`).
Therefore *exact pixel* parity is undefined by construction. The achievable,
meaningful target is **visual/functional parity**: the same surfaces/facets/
sprites/landscape drawn at the same positions, with the same transforms, color
modulation, blend modes, and clipping — verified two ways:
1. **Code-level fidelity** to the C++ draw algorithm (cite `file:line`).
2. **Visual spot-checks** (screenshot the Rust frame, compare to the C++ frame).

A pixel-exact differential test against the GL renderer is unsound (GPU-dependent),
so rendering changes are verified by (1) exactness unit tests against the C++
*formulas* where they are integer/deterministic (modulation, blend math), and (2)
visual comparison for composed scenes.

## Empirical starting state (2026-05-30, measured side-by-side)

- 2D asset blitting works (menu background + logo render correctly).
- Menu/scenario-browser chrome **diverges** (C++ large wooden buttons + GL 3D book;
  Rust flat panel). See `PORT_STATUS.md` "Graphical parity (empirical)".
- The core blit (`lc-graphics/src/surface.rs::blit_region`) was a bare 1:1
  alpha-over copy — **no modulation, blend modes, stretch, transform, or clip**.
- `lc-graphics` ≈ 1,276 LOC vs the C++ render core
  (`StdDDraw2`+`StdGL`+`C4Surface`+facets+graphics system) ≈ 6,800 LOC, plus
  scene drawing scattered across viewport/object/landscape code.

## Prioritized phases

### R1 — rendering primitives (foundation; everything draws through these)
All done + unit-tested against the C++ formulas, 2026-05-30 (`lc-graphics`, 33 tests).
- [x] **Color modulation** (`dwModClr`): `Color::modulate_clr` / `modulate_clr_mod2`
      mirroring C++ `ModulateClr`/`ModulateClrMOD2` (`src/StdColors.h:159,183`),
      incl. the `(255*255)>>8=254` `>>8` quirk. Opaque-white = GL identity.
- [x] **Blit modes**: `BlitMode::{Normal,Additive,Mod2}` (`src/C4Surface.h:39`).
      Additive = `dst + src·srcAlpha` per `glBlendFunc(GL_SRC_ALPHA, GL_ONE)`
      (`StdGL.cpp:908`); Mod2 = alpha-weighted `modulate_clr_mod2` combine.
      `Surface::blit_region_ex(src, rect, dest, modulation, mode)`.
- [x] **Stretched blit** (`blit_stretched`) — nearest-neighbour facet scaling.
- [x] **`CBltTransform`** affine transform — `transform.rs::Transform` ports
      `SetRotate`/`SetMoveScale`/`TransformPoint`/`*=`/inverse exactly
      (`src/StdDDraw2.{h,cpp}`); `Surface::blit_transformed` inverse-maps
      destination pixels (rotation/scale/mirror).
- [x] **Clipping rect** (`set_clip`/`clear_clip`, `SetPrimaryClipper`) honoured by
      every blit + `fill_rect`.
- [x] **Filled box** (`fill_rect`, `DrawBoxDw`) — alpha-blended, clip-aware.
- [ ] **Lines / gradient quads** (`DrawLineDw`, `DrawQuadDw` with per-vertex
      colour) — needs the C++ DDA/scanline formula verified before asserting;
      lower priority (used for debug overlays + a few HUD gradients).

### R2 — GUI / menu chrome (most visible divergence; no simulation dependency)
- [x] Main-menu labels match C++ (`startup_main_menu.rs`: "Start Game" /
      "Start Network Game"), verified live via screenshot 2026-05-30.
- [x] Startup main-menu layout matches `C4StartupMainDlg` (`C4StartupMainDlg.cpp:44`):
      buttons fill the right 2/5 (`GetFromRight(Wdt*2/5)`), inset `Wdt/26` /
      `40+Hgt/8`, stacked from the top; the blue panel backdrop + footer box were
      removed (C++ has neither) and the participants label is plain right-aligned
      text at `(Wdt*39/40, Hgt*9/10)`. Button captions centred. Verified live
      2026-05-30 — full-width wooden planks on the loader background.
- [x] Button caption colour matches C++ exactly: `C4GUI_ButtonFontClr = 0xffffff00`
      (yellow) when active, `C4GUI_InactCaptionFontClr = 0xffafafaf` when disabled
      (`src/C4Gui.h:53-56`). Verified live 2026-05-30.
- [ ] Remaining menu polish: exact plank texture/bolt detail (asset-level), hotkey
      underline from the `&` marker, version string under the logo.
- [ ] Other startup dialogs: scenario-selection "book", options, player select,
      about — layouts to match their `C4Startup*Dlg`.
- [ ] Scenario-selection "book" (`C4StartupScenSelDlg`) — the parallax 3D book.
- [ ] Font rendering + markup (`StdFont`, `StdMarkup`) — hotkeys, colored text.

### R3 — in-game scene (largest; depends on simulation correctness)
- [x] **Verification capability**: `lc-app --dump-frame <png> [--sandbox] --test-frames N`
      renders one in-game frame headlessly (no window) to PNG, sidestepping the
      window-focus problem. Plus `--sandbox` boots straight into the sandbox.
      *Done 2026-05-30 — this unblocks all R3 visual checks.*

The first headless dump (1280x720, sandbox) shows the current Rust in-game render,
and confirms R3 is substantially incomplete vs C++:
- [ ] Sky / parallax not visible in-game (scene is mostly black). `draw_sky`
      exists but the sandbox frame shows no gradient/ground — investigate camera
      framing + sky for the flat sandbox landscape.
- [ ] Landscape rendering — flat/segment model, no material/texture-mapped terrain
      (`C4Landscape::Draw`). Sandbox ground not visibly drawn.
- [ ] **Command/HUD icons render as solid owner-colour (red) silhouettes** — the
      ColorByOwner/`dwModClr` modulation is applied to the whole icon instead of
      only the team-colour mask pixels. Concrete bug, ties to R1 modulation
      (`blend_color_by_owner` `lib.rs:133`; sprite path `lib.rs:1854`).
- [ ] Object sprites — wire `draw_transform`/rotation through `blit_transformed`
      and owner-colour only via the ColorByOwner mask.
- [ ] Particles (`C4Particles` draw).
- [ ] HUD: Rust draws a debug overlay (FRAME/POS/VEL); C++ has the upper board with
      crew portraits + wealth + scoreboard.

**Caveat:** even once these are drawn correctly, the in-game frame cannot match
C++ pixel-for-position until the determinism-critical simulation gaps (integer
coords, ChaCha8 RNG — see `PORT_STATUS.md`) are fixed, since those decide where
every object/material is.

## Verification capability still TODO

- A deterministic **frame-dump** (`lc-app` render-state → PNG) so any scene can be
  captured headlessly and compared to a C++ reference frame without window/grant
  issues. (Blocked the live in-game capture this session; see `PORT_STATUS.md`.)
- `~/Applications/ClonkRust.app` exists (bundled `lc-app` with content paths in
  `Info.plist`); on a fresh session it is grantable to computer-use, enabling
  automated side-by-side capture of both engines.
