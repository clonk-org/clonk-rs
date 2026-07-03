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
- [x] **`Graphics.Scale` honoured (2026-06-12).** The shared config's
      `ResolutionX/ResolutionY/Scale` keys are read with the C++ names
      (C4Config.cpp:440-442; the Rust app previously read nonexistent
      `ResX/ResY` and ignored Scale at render time, so menus laid out 1:1
      over the physical window — the wrong zoom and dialog proportions).
      The window asks for `ResX*Scale` output pixels (C4Application.cpp:183),
      the app renders at `ceil(pixels/scale)` logical
      (C4Application::SetResolution), `lc-scaling::FramePresenter`
      bilinear-upscales the finished frame, and mouse input divides by the
      scale (C4MouseControl.cpp:185). KNOWN GAP at Scale != 100: C++
      rasterizes fonts at `size*scale` (C4Fonts.cpp:172) and stretches each
      texture directly to the output, so its text stays crisp; the Rust
      frame upscale is geometrically identical but softer. Per-renderer
      scale-aware rasterization is the follow-up if crispness matters.
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
- [x] **Startup main menu pixel-exact vs C++ (2026-06-10).** Measured against an
      F9 GL-backbuffer capture of the C++ engine at windowed 1280x720:
      **99.80% of pixels bit-identical**; the residual is ±1-LSB GPU filter
      rounding under the focus highlight (invisible) plus the mouse-cursor
      sprite baked into the C++ capture. What it took (all cited in code):
      * Exact `C4StartupMainDlg` integer geometry incl. fullscreen-dialog client
        margins (`main_menu_layout`, unit-tested: buttons at (842,201+44i,414,40)
        @720p); 3-slice `DrawBar` planks; additive `GUIButtonHighlight`
        focus/hover overlay; trademark line; `"Players: none selected"`;
        version string from `C4VERSION`.
      * **CStdFont-faithful fonts**: FreeType `FT_LOAD_RENDER|FT_LOAD_NO_HINTING`
        at `FT_Set_Pixel_Sizes(h,h)` (same rasterizer family as the vendored
        2.14.x), atlas-cell composition with the baked blur shadow + BltAlpha
        `>>8` quirk (`lc-graphics/src/clonk_font.rs`), sizes 12/13/14/16/22,
        `<c>` markup + `&` hotkey highlight (renders pale, NOT underlined),
        glyph-exact captions/labels.
      * **Gamma**: the blit shader's per-fragment ramp lookup (`CGammaControl`
        formula, NEAREST 1D-texture index = `floor(c*256/255)`, black floor
        0→1) — `lc-graphics/src/gamma.rs`.
      * **Blits**: `CStdDDraw::Blit`-faithful per-texture-tile quads
        (pow2-of-min-dim tiles, `GL_CLAMP_TO_EDGE` per C4Surface.cpp:1102),
        GL_LINEAR bilinear in f32, float blend, single store rounding.
- [x] Verification: `lc-app --dump-menu-frame <png>` renders the startup menu
      headlessly at 1280x720. Compare against `build/Screenshots/*.png` shifted
      **one row up** — C++ `C4Surface::SavePNG` has an off-by-one in its
      `glReadPixels` readback loop (`realHgt - y`, C4Surface.cpp:434), so every
      F9 screenshot is one row off and its top row is undefined. (Upstream C++
      bug worth fixing; the live window is NOT shifted.)
- [ ] Remaining menu polish: none known at the main menu beyond the ±1-LSB GPU
      filter residual.
- [x] **About, Scenario-Selection book, Network dialogs pixel-exact
      (2026-06-10).** App-level `--dump-menu-frame --menu-view about|scenarios|net`
      vs F9 refs (`build/Screenshots/ref-*.png`, row-shift + masks applied):
      about **96.15%**, scensel **95.56%**, net **97.36%** bit-identical, and in
      all three EVERY residual pixel is channel-delta 1 (GPU bilinear rounding
      on the stretched 800x600 backgrounds) — zero structural diffs.
      Renderers: `lc-frontend/src/startup_{about_dlg,scensel,netdlg}.rs`
      (spec-driven, every formula cited; specs in `target/parity-specs/`).
      New empirics baked in: `DrawLineDw` drops its end pixel (GL_LINES
      diamond-exit) so `Draw3DFrame` corners blend once; shadowless book fonts
      (`build_book_font_set`); zoomed `DrawBar` branch for the 23px GUICaption
      bar. Masks: cursor sprite baked into refs at ~(637,356); scensel list +
      right page (live entries differ from the ref's empty exe-dir scan); net
      list client (nondeterministic masterserver content).
- [x] **Options and Player-Selection dialogs pixel-exact (2026-06-10).**
      App-level dumps vs F9 refs: options **98.69%**, plrsel **95.63%**
      bit-identical, all residuals channel-delta 1; zero structural diffs
      outside masks (cursor sprite; plrsel list content — the app has no
      packed-`.c4p` player discovery yet, so the live dialog shows the empty
      state while the ref shows Tyler). Renderers
      `startup_options_dlg.rs` / `startup_plrsel.rs`. More engine quirks
      pinned with tests: `ReadPNG` and `SetPixDw` squash fully-transparent
      texels to BLACK (C4Surface.cpp:972,733 — GL tile padding stays
      transparent WHITE), which bleeds through GL_LINEAR on alpha-edged
      stretched assets; `fill_quad_dw` scanline ties are
      left-inclusive/right-exclusive.
- [ ] Player discovery for the plrsel dialog: read packed `.c4p` groups
      (portrait/BigIcon/Player.txt) in lc-resources, feed `PlrSelPlayer`s.
- [ ] Options interactivity: the parity sheet is display-only; control
      clicks still route to the old keyboard-rebind UI semantics (ESC works).
- [ ] Consolidate the three duplicated shadowless book-font builders and the
      per-dialog draw helpers (box/3D-frame/caption-bar/scrollbar) into one
      shared module (deliberate duplication from parallel agent ownership).
- [ ] Scen-sel interactivity beyond select/open/back clicks: search filter,
      scrollbar drag, right-page selection info, folder icons from
      Folder.txt/Icon assets (list rows currently use kind-default icons).

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
- [x] **Object faces anchored + con-scaled like C4Object::Draw** *(2026-07-03)* —
      sprites now take precedence over the debug vertex polygon (GoldRush trees
      rendered as flat hash-coloured triangles before); faces anchor at the
      shape top-left `x+Shape.x`/`y+Shape.y` (`C4Object.cpp:2231`) with
      `C4Shape::Stretch`/`Jolt` con-scaling (`C4Shape.cpp:103-128`); idle/
      FacetBase base faces implement DrawFace growth + construction display
      (`C4Object.cpp:438-467`); action facets place at `cox+FacetX`/`coy+FacetY`
      at full con and stretch over the con-scaled shape while growing
      (`C4Object.cpp:2450-2467`); FlipDir mirrors and rotation orbit the shape
      center; facet sources clamp to the sheet (Tree1 `Still` is 73x73 on a
      71px sheet). Base-graphics variants (`SetGraphics`) honored + pinned.
      Remaining: draw transforms are reduced to scale+offset (no full 2x3
      matrix), rotated shapes anchor with the unrotated shape bbox (C++
      rotates `Shape` in UpdateShape), MODE_Base overlays still draw the
      whole sheet centered.
- [ ] Object sprites — owner-colour only via the ColorByOwner mask.
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
