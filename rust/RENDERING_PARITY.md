# Rendering parity

> Current architecture and verification snapshot, 2026-07-21. The C++ oracle
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

1. `lc-graphics::Surface` records painter-ordered `GpuCommand`s while ordinary
   frontend code draws. Textures carry a stable id, current revision, complete
   CPU regeneration backing, and optional dirty rectangles.
2. `lc-frontend` lowers sky, landscape/liquid, sprites, particles, HUD, GUI,
   menus, and solid primitives into those commands.
3. `lc-app` builds an ordered `RetainedGpuFrame`. Logical game/chrome layers use
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

`pixels` already reconfigures the surface and retries acquisition once. A
`Lost` or `Outdated` error returned after that retry causes `lc-app` to rebuild
`Pixels`, then call `RetainedGpuRenderer::recreate` with the replacement device,
queue, and surface format. The renderer also records uncaptured wgpu validation
and allocation failures. Pinned wgpu 0.16 reports some native device-loss paths
by panicking in queue submission or polling; the presentation/readback boundary
catches only those recognized diagnostics and converts them to the same typed
recreation request, while unrelated panics resume unwinding. The next
self-contained scene repopulates every device resource. `Timeout` schedules
another presentation; `OutOfMemory` and unrelated validation/parity errors
remain fatal. There is no CPU-frame recovery fallback.

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
- App tests `m06_l033_all_graphical_modes_produce_retained_scenes`,
  `m06_l033_scale_native_text_keeps_logical_physical_painter_order`, and
  `m06_l033_surface_error_policy_rebuilds_or_retries_only_recoverable_errors`
  cover mode integration and recovery policy. The scale-one and every-scale
  startup-font tests ensure reachable retained text cannot fall back to a
  point-rasterized or mismatched atlas.

The measured Deep Sea retained-GPU reference is documented in `PERFORMANCE.md`:
Apple M4 Max/Metal, 800x600 at 100% scale, 20.002 seconds after warmup, 1,077
presentations, 5.794 ms average and 9.202 ms maximum graphics-pass time, with no
automatic graphics skips. This is a fingerprinted reference, not a universal
60 FPS claim.

## Remaining limits and review rules

- A true operating-system/device-loss injection test is not portable. Direct
  replacement-device recreation, uncaptured-error classification, recognized
  native device-loss conversion, and the app's surface-error policy test cover
  the deterministic portions; live recovery still needs platform smoke tests.
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
