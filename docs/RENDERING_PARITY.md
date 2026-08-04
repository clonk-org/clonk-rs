# Rendering parity

> Current architecture and verification snapshot, 2026-08-02. The C++ oracle
> is `src/StdDDraw2.*`, `src/StdGL.cpp`, `src/C4Surface.cpp`, and the draw sites
> in `C4GraphicsSystem`, `C4Viewport`, `C4Landscape`, and `C4GUI`.

## What parity means

LegacyClonk uses a process-global OpenGL renderer. The Rust port uses wgpu, but
the backend API is not the behavior: parity requires the same painter order,
coordinates, sampling, packed-C4 modulation, blending, clipping, gamma
placement, and visible resources.

Cross-driver GL output is not generally bit-identical. Integer color formulas
and resource state transitions are therefore tested exactly; GPU sampling and
composed frames are checked by readback against the software oracle, allowing a
one-byte tolerance only where the native GL filter itself is driver-dependent.
The software renderer remains useful as an oracle, not as the normal windowed
presentation path.

## Current windowed architecture

Normal graphical presentation is retained and GPU-composed:

1. `clonk-graphics::Surface` records painter-ordered `GpuCommand`s while ordinary
   frontend code draws. Textures carry a stable id, current revision, complete
   CPU regeneration backing, and optional dirty rectangles.
2. `clonk-frontend` lowers sky, landscape/liquid, sprites, particles, HUD, GUI,
   menus, and solid primitives into those commands.
3. `clonk-app` builds an ordered `RetainedGpuFrame`. Logical game/chrome layers use
   the scaled and cropped presentation transform. Scale-native font layers use
   an identity transform at the same physical extent.
4. `RetainedGpuRenderer::render_layers` validates the complete frame before GPU
   mutation, merges compatible retained resources, appends every layer to one
   command stream, and composes once into a physical `Rgba8Unorm` target.
5. Fragment or monitor gamma is applied in the C++-selected location. The final
   composition is then presented, or copied for screenshots/save thumbnails.

`pixels` is deliberately kept at a 1x1 logical buffer during retained
presentation so it cannot upload a full software frame before `render_with`.
Menu, loading, running, and graphical console modes all enter the retained
capture path. A render error is recovered or reported; it does not silently
switch the current frame to software composition.

The CPU path is intentionally retained for headless dumps, deterministic
reference tests, resource preprocessing, and private scratch surfaces. Those
uses do not make the visible window a CPU-frame upload renderer.

Native GL never reads framebuffer alpha back (CStdGL uses no `GL_DST_ALPHA`
factor and screenshots drop alpha), so the deterministic CPU reference is the
authority for destination alpha. Its conventions are per producer and the
retained GPU target reproduces them exactly: primitive quads, boxes, lines,
and points blend alpha source-over; sampled-fragment recovery through
`blend_fragment` applies the same non-separate factor to alpha as to RGB; and
additive draws preserve destination alpha while adding source-alpha-weighted
RGB. Private premultiplied text scratch layers accumulate source-over and are
unpremultiplied once before retained upload. Every retained solid carries its
destination-alpha provenance (`GpuSolidAlphaMode`) so recorder coalescing
cannot mix the source-over and non-separate equations, and the classic
startup/loader rasterizers keep their byte-exact quantize-before-blend CPU
loops while capture reroutes the same fragments into painter-ordered retained
commands.

### Painter order and scale-native text

One frame may contain several coordinate spaces without introducing an
offscreen-opacity approximation. `GpuSceneLayer` associates each scene with its
own `GpuPresentation`, while all layers share one physical target and one gamma
snapshot. Commands remain in the exact layer order supplied by the app.

C++ dialog fading is not group opacity. `C4GUI::Dialog::Draw` activates packed
`0xTTRRGGBB` blit modulation while each child draws. The retained equivalent
records whether each texture inherits, combines, or suppresses the enclosing
state, matching the distinct native call sites:

- an ordinary unmodulated base blit inherits the active value directly, so an
  outer white stays 255 rather than being rounded to 254;
- local, fog, owner, solid, and semantic-text colors combine through
  `ModulateClr`: RGB is `(destination * source) >> 8` and transparency is
  `dst + src - ((dst * src) >> 8)`;
- `C4GFXBLIT_CLRSFC_OWNCLR` owner passes and explicit nested overrides suppress
  the enclosing value;
- byte-derived solid RGBA is converted to C4 transparency and back, while an
  already-filtered recovery fragment receives the equivalent float shader
  operation without byte quantization;
- replacement quads/solids become normal blends when modulation adds
  transparency, preserving the already-painted underlay;
- semantic text uses `modulate_rgba8_by_packed_c4` before retained glyph-quad
  capture.

Packed colors that cannot be recovered as exact bytes are rejected with a
typed error. The recorder validates the whole stream before changing any
command, so failure cannot leave a partially faded scene.

## Implemented retained semantics

| Area | Current retained behavior |
| --- | --- |
| Textures | Stable identities, copy-on-write forks, complete recovery backing, sparse revision deltas, bounded resident cache, and bounded content interning for immutable derived images. |
| Sampling | Nearest and linear sampling, native independent `C4TexRef` tile clamps/padding, stretch and projective transforms. |
| Color | Packed-C4 modulation, MOD2, normal/replace/additive blending, owner-color passes, spatial fog modulation. |
| Geometry | Triangles plus physical fragments for OpenGL 2.1 points and lines: center-clipped odd/even point snapping, directed clip-volume clipping, diamond-exit coverage, endpoint exclusion, rounded-width replication, and perspective color interpolation. Logical clips retain C++'s independently rounded, clip-relative projection and physical scissor. |
| World | Sky gradients/images, textured or fallback landscape, liquid mask animation, objects, PXS and definition particles. |
| UI | Startup/loading chrome, options/player/scenario screens, in-game HUD and menus, dialogs, cursors, scoreboard markup, and stable scale-native glyph/inline-image quads. |
| Gamma | Disabled, per-fragment lookup, and completed-frame monitor postpass follow the active renderer switches. |
| Output | Physical presentation, resize, screenshot readback, save-thumbnail readback, sRGB-surface byte preservation. |

Advanced renderer switches (`Shader`, `UseShaderGamma`, `NoAlphaAdd`,
`PointFiltering`, `NoBoxFades`, and related texture behavior) are snapshotted by
the frontend and reflected by command preparation rather than guessed in the
backend.

## Validation, cache, and recovery contract

A `GpuScene` is a self-contained recovery unit. Every texture referenced by a
command must be declared in that scene even when the renderer cache happens to
contain the same id. Before uploads or cache mutation, validation checks:

- nonzero finite presentations and finite projected coordinates;
- unique, correctly sized resources and every dirty rectangle;
- revision advancement for deltas;
- declared texture formats and complete liquid pairs;
- lowered owner masks, valid primitive counts, scissors, and vertex ranges;
- compatible physical extent, gamma snapshot, and shared texture backing across
  ordered layers.

A cache hit at the declared base revision receives only the dirty rectangles.
Skipped producer revisions, mode transitions, incompatible deltas, and a new
device use the complete backing instead. Resize recreates only the physical
composition target and preserves the renderer generation and source cache. The
cache is bounded to 256 MiB and 4096 entries, evicting least-recently-used
resources that are not referenced by the current frame. Separately, immutable
frontend images are content-interned with dimensions and collision-checked RGBA
bytes, bounded to 16,384 entries/128 MiB, so repeatedly derived carets, game-over
facets, refresh phases, inventory chrome, glyphs, and inline images retain one
GPU identity instead of forcing uploads every frame.

The locally patched `pixels` 0.17.2 owns bounded transient surface recovery
(`third_party/pixels/src/lib.rs`). `Lost` returns a typed `SurfaceLost` error so
each window owner can reconstruct `Pixels`; `Outdated` reconfigures and retries
once before skipping the frame; and `Suboptimal` reconfigures once before using
the still-valid acquired frame if it remains suboptimal. `Timeout` and
`Occluded` also return success without invoking the render callback. This bounds
the upstream retry loop tracked by parasyte/pixels#460. `clonk-app` records
whether the callback ran, reports a skipped presentation when it did not, and
leaves screenshots and save-thumbnail requests queued for the next drawable
frame.

Every `SurfaceLost` owner moves its old `Pixels` value out of an explicit slot
and drops it before the replacement builder runs. Dropping wgpu's surface
synchronously unconfigures the old swapchain, so Vulkan and DX12 never see two
configured swapchains for one native window. A successful rebuild may request
one prompt redraw; further losses rebuild only on the owner's normal graphics
cadence until an actually presented frame rearms that prompt. Skipped frames do
not rearm it. The game surface also restores its prior buffer extent and bytes,
so an unchanged cached CPU frame survives device recreation. The launcher's
otherwise-unconditional redraw loop additionally paces continued
loss/occlusion retries at 250 ms.

wgpu 29 reports device loss through the device-loss callback. The retained
renderer converts that notification into a typed recreation request. The app
checks renderer health before presentation, after a successful return, and
when an error or panic escapes (`present_retained_gpu_frame`,
`crates/clonk-app/src/main_parts/audio.rs`). Device loss recorded at any of
those checkpoints takes precedence over a generic `pixels::Error::Validation`
or panic; `clonk-app` rebuilds `Pixels`, then calls
`RetainedGpuRenderer::recreate` with the replacement device, queue, and surface
format. A narrowly recognized device-loss panic from submission or readback
remains a compatibility fallback for backends that fail before dispatching the
callback, while unrelated panics with healthy renderer state resume unwinding.
The next self-contained scene repopulates every device resource. Uncaptured
allocation, internal, and unrelated validation/parity errors remain fatal.
Surface and device errors never silently select CPU-frame recovery.

## Executable evidence

Renderer and scene tests pin the backend boundary:

- `gpu_renderer_matches_cpu_reference_frame` exercises textured normal,
  replace, additive, MOD2, clipping, projective coordinates, native tiled
  linear filtering, landscape/liquid, owner overlays, solids, points, gamma,
  resize, dirty upload, cache residency, and explicit device recreation.
- `layered_presentations_preserve_physical_painter_order` performs real GPU
  readback with a scaled logical layer and an overlapping identity-space layer.
- `recovery_validation_requires_every_command_texture_in_the_current_scene`,
  `recovery_validation_checks_all_deltas_before_gpu_mutation`,
  `recovery_validation_rejects_projection_overflow_before_gpu_mutation`, and
  `mode_and_device_generation_gaps_choose_safe_texture_uploads` pin recovery
  preconditions.
- `semantic_text_style_combines_with_cpp_shift_and_transparency_screen`,
  `direct_textured_inherit_uses_outer_color_without_white_rounding`,
  `combined_texture_fog_and_owner_use_cpp_modulate_clr`, and
  `recorder_modulation_validates_all_commands_before_mutating_any` pin exact
  dialog/text modulation and atomic failure.
- `source_over_normal_blend_matches_cpu_reference_alpha`,
  `non_separate_normal_blend_shares_color_factors_with_alpha`,
  `additive_blend_preserves_destination_alpha_for_both_modes`,
  `fractional_clipper_rounds_viewport_then_projects_relative_coordinates`, and
  `logical_line_pair_expands_to_cpp_application_scale_in_physical_space` pin
  the CPU-reference alpha translation, fractional clip projection, and scaled
  line width.
- `line_clipping_preserves_directed_entry_and_exit_endpoints`,
  `diagonal_line_color_uses_cpp_window_space_projection_parameter`,
  `wide_point_is_center_clipped_before_physical_rasterization`, and the real-GPU
  reference frame pin directed OpenGL 2.1 line/point fragments and interpolation.
- `native_gpu_text_is_one_stable_textured_quad_per_glyph` prevents regression
  to CPU-sampled, point-per-pixel scale-native text.
- `retained_fogged_cursor_text_is_one_textured_gamma_quad_per_glyph`,
  `retained_fogged_markup_text_is_a_stable_textured_draw_not_point_coverage`,
  `retained_moving_pxs_is_one_gpu_line_not_one_point_per_covered_pixel`, and the
  retained CONNECT/selection/bolt tests pin formerly rasterized running draws.
- `identical_immutable_images_reuse_retained_gpu_identity`, the scoreboard
  texture/markup tests, and the loader/sky/caret/refresh resource tests pin
  stable derived resources and revisioned dynamic uploads.
- Surface tests pin retained alpha/additive fragments, gamma capture,
  copy-on-write revisions, projective rejection, child clipping, and semantic
  text capture.
- App tests `all_graphical_modes_produce_retained_scenes`,
  `scale_native_text_keeps_logical_physical_painter_order`, and
  `pixels_handles_surface_recovery_and_app_handles_renderer_failures`
  cover mode integration and recovery policy. The scale-one and every-scale
  startup-font tests ensure reachable retained text cannot fall back to a
  point-rasterized or mismatched atlas.

The measured Deep Sea retained-GPU reference is documented in `PERFORMANCE.md`:
Apple M4 Max/Metal, 800x600 at 100% scale, 20.002 seconds after warmup, 1,077
presentations, 5.794 ms average and 9.202 ms maximum graphics-pass time, with no
automatic graphics skips. This is a fingerprinted reference, not a universal
60 FPS claim.

## Menu frame rasterization verified against a real C++ GL capture

The one question the retained-GPU work left open — whether the CPU menu-frame
bytes match native `DrawFrameDw`/`DrawFrame` line output — was settled on
2026-07-21 against a live C++ capture, and the CPU paths were corrected to
line-accurate coverage.

Capture procedure (reproducible in ~2 minutes):

1. Harness: a scratch directory containing a copy of
   `build-arm64-native/clonk.app` (binary of 2026-07-12; the rendering
   sources `StdGL.cpp`/`StdDDraw2.cpp`/`C4Menu.cpp`/`C4Gui.cpp` last changed
   2024-12-03, so any current build is equivalent), symlinks to
   `planet/Graphics.c4g`, `planet/System.c4g`, `content/{Objects,Knights,
   Fantasy}.c4d`, `content/{Material,Music,Sound}.c4g`, `content/Fantasy.c4f`,
   and a copy of a player file (`Neuling.c4p`). The engine resolves its data
   directory from the app-bundle parent, not the cwd.
2. Private config via `LC_CONFIG_FILE` (C4Config.cpp:768): `DisplayMode=Window`,
   `ResolutionX=1280`, `ResolutionY=720`, `Scale=100`, `Shader=true`,
   `DisableGamma=false`, `ShowCommands=true`, `ShowCommandKeys=true`,
   `Participants="Neuling.c4p"`, sound/music/signups off.
3. `./clonk.app/Contents/MacOS/clonk "Fantasy.c4f/Drachenfels.c4s"` — the
   scenario script opens the "Select difficulty" object menu automatically.
   Leave mouse input untouched (tooltips draw only while
   `!Mouse.IsActiveInput()`, C4Menu.cpp:808-820); the delayed tooltip appears
   after 90 frames. Post F9 with a `CGEventPostToPid` helper; the engine
   writes `Screenshots/Screenshot001.png` next to the data directory.
4. Decode without color management (pure-zlib PNG reader) and probe bytes.

Measured facts (Drachenfels, 1280x720, default gamma ramp
`[0x000000, 0x808080, 0xffffff]`):

- Tooltip (`C4GUI::Screen::DrawToolTip`, C4Gui.cpp:907-925) at (942,580)
  182x26: fill `#F1EA78` stores exactly (241,234,120) and text `#483222`
  stores (72,50,34) — the identity ramp does not alter nonzero channels.
  Every `DrawFrameDw` frame pixel over the fill, the four corners included,
  reads (121,117,60): exactly one source-over blend of 0x7f000000 with GL
  round-to-nearest on store and the gamma shader's black floor (0 -> 1;
  120.53 rounding up to 121 proves both). The directed line loop
  (StdDDraw2.cpp:1181-1187) covers every corner exactly once; the former Rust
  full-length strips double-blended corners to (61,59,30) and diverged.
- Extra-bar divider (`C4Menu::DrawFrame` -> `CStdDDraw::DrawFrame`,
  C4Menu.cpp:846-849,932-935, StdDDraw2.cpp:1173-1179) at (1032,647)-(1208,662):
  palette color 80 stores (68,1,1) (black-floored `#440000`); the top and
  left corners paint, the right column stops at y=661 and the bottom row at
  x=1207, and the shared excluded endpoint (1208,662) — the bottom-right
  corner — stays at the (1,1,1) background. The former strips painted it.
- Dialog `Draw3DFrame` (C4Gui.cpp:264-279) borders over the (1,1,1) fog:
  single-blend values (38,11,1) outer `#772200@0xaf`, (17,6,1) inner
  `#331100@0xaf`, (54,22,1) `#AA4400@0xaf`, and the never-covered bottom-right
  pixels stay background — matching the end-exclusive strip/line coverage the
  Rust `draw_3d_frame` already used (the June startup captures had pinned the
  same corner semantics bit-exactly across six screens).

Consequences now in code: `classic_gui::draw_engine_frame_hv` models
`CStdDDraw::DrawFrame`'s horizontal/vertical line set (and records those four
native segments during retained capture — the previous strip-era recording
used the `DrawFrameDw` loop, which wrongly painted the divider's bottom-right
corner); `ingame_menu`/`object_menu` frame outlines route through the engine
line rasterizer for both CPU and capture, so the pinned software oracle now
carries line-identical bytes (the `*_per_cpp_capture` tests pin the capture
values, and
the re-pinned Dragon Rock hashes embed them end to end).

## Remaining limits and review rules

- A true operating-system/device-loss injection test is not portable. Direct
  replacement-device recreation, wgpu 29 callback and uncaptured-error
  classification, recognized compatibility-panic conversion, and the app's
  presentation-recovery policy test cover the deterministic portions; the
  vendored Pixels tests pin bounded `Lost`, `Outdated`, and `Suboptimal`
  acquisition behavior. Live device-loss recovery still needs platform smoke
  tests.
- New draw code must not read the destination CPU pixels during active capture.
  It must emit a blend command, use an isolated CPU scratch resource before
  capture, or return a typed parity error. Tests that only inspect a completed
  software oracle do not prove the retained path.
- Visual/content parity remains an oracle-by-oracle concern. A missing sprite,
  menu behavior, or simulation state should be tracked in its owning milestone;
  it is not evidence that the window has fallen back from GPU composition.

When changing this boundary, retain both the software oracle and real GPU
readback coverage. Do not add a generic CPU-frame fallback to make an
unsupported command appear to work.
