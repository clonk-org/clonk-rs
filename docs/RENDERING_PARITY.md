# Rendering and presentation parity

The C++ authorities are `src/StdDDraw2.*`, `src/StdGL.cpp`,
`src/C4Surface.cpp`, and the draw sites in `C4GraphicsSystem`, `C4Viewport`,
`C4Landscape`, and `C4GUI` at oracle commit
`7d43b47b7d789b533f32d005e64596e0a07019cd`.

## Contract

LegacyClonk uses a process-global OpenGL renderer. The Rust port can compose a
frame through wgpu or through the CPU software renderer and can present a CPU
frame through either wgpu or a wgpu-independent software presenter. Backend
choice is not a behavioral deviation. Every interactive route must preserve:

- painter order and the separation between logical and physical coordinate
  spaces;
- texture sampling, tiling, clipping, and projective coordinates;
- packed-C4 modulation, owner color, fog, blend equations, and destination
  alpha conventions;
- gamma placement and the pixels selected for screenshots and save thumbnails;
- visible resource selection, revisions, and frame-to-frame state; and
- simulation, control, and synchronized-RNG independence from presentation.

Cross-driver GL output is not generally bit-identical. Integer color formulas,
CPU output, command lowering, and resource transitions are exact. GPU sampling
and composed frames are checked by readback against the software oracle, with a
one-byte tolerance only where native GPU filtering is driver-dependent.

## Presentation routes

The primary application window has three interactive routes plus non-windowed
CPU uses:

| Route | Composition | Window presentation | Selection |
| --- | --- | --- | --- |
| Retained GPU | `RetainedGpuRenderer` consumes painter-ordered commands | wgpu `WindowSurface` | Normal graphical path on a usable adapter. |
| CPU through GPU | Software renderer writes the physical CPU frame | wgpu `WindowSurface` blits it | A retained source or shader exceeds the active device's supported limits. |
| Software | Software renderer writes the physical CPU frame | wgpu-independent `SoftwarePresenter` | Forced by `LC_SOFTWARE_PRESENTATION`, or automatic when no usable adapter meets the graphics floor. |

Auxiliary object-list, toolbox, component-editor, and scoreboard panes use
`clonk-app`'s separate `SoftwareWindow` host: they compose CPU pixels but
present them through their own GPU-backed `WindowSurface`. They are not another
primary-window selection mode and do not inherit its wgpu-independent fallback.

A console-mode dialog is one of these panes rather than a layer of the primary
window: `Dialog::Show` creates a child window for any non-viewport dialog when
the application is not fullscreen, and `Dialog::Close` destroys it again. Such a
dialog has no caption widget, title icon, or close button — `Dialog::SetTitle`
puts the text on the OS window bar and returns before allocating one — and it
sizes itself, so the window follows the dialog rather than the player.

Headless dumps, deterministic reference tests, resource preprocessing, and
private scratch surfaces also use CPU pixels but do not present a window.

The forced software route returns before constructing a wgpu instance, adapter,
or device. Automatic startup fallback records whether the cause was no adapter
or a below-floor adapter. A lost device on an established GPU route is different:
the app rebuilds the GPU surface and renderer and does not silently change the
session to software presentation. Fatal allocation, validation, internal, and
unrelated parity errors remain fatal.

The CPU-composition fallback for a retained source or shader limit is also not
a device-loss fallback. It uses the exact software oracle for the frame and may
still present that frame through the existing wgpu surface.

## Retained GPU command contract

1. `clonk-graphics::Surface` records painter-ordered `GpuCommand`s while the
   frontend draws. Textures carry a stable identity, revision, complete recovery
   backing, and optional dirty rectangles.
2. `clonk-frontend` lowers sky, landscape/liquid, sprites, particles, HUD, GUI,
   menus, text, and solid primitives into those commands.
3. `clonk-app` builds an ordered `RetainedGpuFrame`. Logical game/chrome layers
   use the scaled and cropped presentation transform; scale-native layers use an
   identity transform at the same physical extent.
4. `RetainedGpuRenderer::render_layers` validates the complete frame before GPU
   mutation, merges compatible retained resources, appends all layers to one
   command stream, and composes once into a physical `Rgba8Unorm` target.
5. Fragment or monitor gamma is applied in the location selected by the C++
   renderer switches. The final composition is presented or read back for a
   screenshot/save thumbnail.

Renderer switches such as `Shader`, `UseShaderGamma`, `NoAlphaAdd`,
`PointFiltering`, and `NoBoxFades` are snapshotted by the frontend and reflected
by command preparation. The backend must not infer them from the commands it
happens to receive.

While this route is active, the `WindowSurface` CPU frame stays at 1x1 so it
cannot upload a full software frame ahead of `render_with`. Switching to CPU
composition restores the physical buffer extent first.

### Painter order and scale-native layers

One frame may contain several coordinate spaces without an offscreen-opacity
approximation. `GpuSceneLayer` associates each scene with its own
`GpuPresentation`; all layers share one physical target and one gamma snapshot.
Commands remain in the exact order supplied by the app.

C++ dialog fading is per-child packed `0xTTRRGGBB` modulation, not group
opacity. Retained commands must distinguish inheritance, combination through
`ModulateClr`, and explicit suppression for owner-color or nested overrides.
Replacement draws become ordinary blends when modulation adds transparency so
the painted underlay remains visible. A packed color that cannot be recovered
as exact bytes is a typed error, and validation must fail before mutating any
recorded command.

### Alpha and classic line coverage

Native GL does not read framebuffer alpha back, so the deterministic CPU
reference defines destination alpha. Primitive solids use source-over alpha;
sampled-fragment recovery uses the same non-separate factors as native GL; and
additive draws preserve destination alpha while adding source-alpha-weighted
RGB. `GpuSolidAlphaMode` keeps those producer conventions distinct through
recorder coalescing.

Classic frames are directed line sets, not full-length rectangle strips.
`DrawFrameDw` covers each corner once; `CStdDDraw::DrawFrame` excludes the
shared bottom-right endpoint. CPU rendering and retained capture must route
through the same line semantics.

## Resource validation and recovery

A `GpuScene` is a self-contained recovery unit. Every texture referenced by a
command must be declared in the current scene even if the renderer cache holds
the same identity. Before upload or cache mutation, validation checks:

- finite nonzero presentations and projected coordinates;
- unique, correctly sized resources and valid dirty rectangles;
- monotonic revision deltas and complete backing after revision/device gaps;
- declared texture formats, liquid pairs, owner masks, primitive counts,
  scissors, and vertex ranges; and
- compatible extent, gamma, layer state, and shared backing across the ordered
  frame.

A matching cached base revision receives only its dirty rectangles. Skipped
producer revisions, incompatible deltas, mode transitions, and a new device use
complete backing. Resize recreates the physical composition target without
discarding compatible source resources. Both the resident GPU cache and the
frontend's immutable content interning are bounded.

`clonk-surface` bounds swapchain acquisition rather than retrying inside the
event-loop callback:

- `Lost` asks the window owner to drop and rebuild its surface;
- `Outdated` reconfigures and retries once;
- `Suboptimal` reconfigures once, then may use the valid acquired frame;
- `Timeout` and `Occluded` skip presentation without running the render
  callback.

Every surface owner drops the old configured `WindowSurface` before building a
replacement. Device-loss notification takes precedence over a generic
validation error or a narrowly recognized submission/readback panic. After a
replacement device is created, `RetainedGpuRenderer::recreate` advances the
generation and the next self-contained scene repopulates device resources.
Repeated loss while no frame can present follows the normal graphics cadence
rather than requesting redraws without bound.

The deterministic parts of this policy have unit and GPU-readback coverage. A
real platform device-loss run remains tracked by clonk-org/clonk-rs#1241; unit
injection is not reported as platform qualification.

## Software composition and presentation

The CPU renderer is both the exact reference implementation and a supported
interactive path. It composes at the physical presentation extent selected by
the same `FramePresenter` geometry as the GPU route, applies monitor gamma to
the presented bytes, and hands tightly packed RGBA to either destination.

The wgpu-independent `SoftwarePresenter` converts those bytes to the platform
pixel words expected by `softbuffer`. It owns its buffer outright, so a present
either succeeds or returns an error; it has no device-rebuild state. Resize must
resize both the CPU frame and the live drawable before the next presentation.

Screenshots come from the selected presenter's physical, gamma-resolved frame.
Save thumbnails follow C++ `C4Game::SaveGameTitle`: they encode the image the
player saw after gamma and reduce it to 200x150. Presentation choice must not
change simulation cadence, RNG, replay hashes, or saved game state.

The reproducible real-window qualification is in
[`../scripts/SOFTWARE_PRESENTATION_SMOKE.md`](../scripts/SOFTWARE_PRESENTATION_SMOKE.md).
GPU surface ownership and teardown use
[`../scripts/HEADED_SURFACE_TEARDOWN_SMOKE.md`](../scripts/HEADED_SURFACE_TEARDOWN_SMOKE.md).

## Required evidence and review rules

Keep all three evidence layers when changing this boundary:

1. exact CPU/reference tests for integer formulas, coverage, and output bytes;
2. retained-scene tests for lowering, validation, revision/cache behavior, and
   recovery decisions; and
3. real GPU readback or headed presenter smoke coverage for behavior that mocks
   cannot establish.

Canonical C++/Rust presentation captures use 1280x720 at 100% scale. Both sides
of a comparison must use the same geometry; comparing different resolutions or
scales is not visual-parity evidence. The required screen set, the measured
blockers and approved masks are machine-readable in
`compat/presentation_captures.json`.

### Capturing a startup pair

Both halves are byte-reproducible, so a difference between them is a real one.
Verified 2026-08-26 for all six startup views (clonk-org/clonk-rs#587).

**Rust half** — headless, one process per view:

```sh
LC_USER_DATA_DIR=<scratch> clonk-app --dump-menu-frame <out>.png \
  --menu-view {main|scenarios|net|plrsel|options|about}
```

The scratch config needs `[Graphics] DisableGamma=0` and `Shader=true`, or
`startup_fragment_gamma` falls back to identity and every pure-black text pixel
comes out 0 where the oracle's gamma LUT floors it to 1 — a capture-config
artifact that reads as a large parity loss. It also needs a player: with none,
the first-run new-player form opens over the menu and picks its colour from
`SafeRandom`, so the capture is neither comparable nor reproducible.
`two_menu_dumps_are_byte_identical_once_the_capture_procedure_has_a_player`
pins that.

**C++ half** — the pinned oracle has no headless dump, so it needs a headed
windowed run driven to each screen and F9 on each. Its own screenshot is
already exactly 1280x720 and is byte-identical across repeats of one screen, so
no OS screen capture is involved. Traps, each of which cost a run:

- the app reads `~/Library/Preferences/legacyclonk.config`, not the port's
  config; back it up, since the engine rewrites it on a clean exit;
- set `DisplayMode=Window`, 1280x720, `Scale=100`, and
  `[Network] EnableAutomaticUpdate=0` or a modal update dialog blocks startup;
- the bundle's content symlinks must resolve, or it segfaults right after
  `Loading graphics...`;
- a build linked against an older homebrew `fmt` will not launch at all once the
  `opt/fmt` symlink moves; repoint it with `install_name_tool -change` rather
  than rebuilding;
- park the mouse in a dead corner before F9. The engine draws its own cursor
  into the frame, so wherever it sits becomes a masked region.

**Comparison** — `C4Surface::SavePNG` reads the framebuffer one row low, so the
C++ capture's rows 1..719 line up with the Rust dump's rows 0..718. Compare on
that shift, not on row 0.

New draw code must not read destination CPU pixels during active retained
capture. It must emit a blend command, use an isolated scratch resource before
capture, or return a typed parity error. A test that inspects only the completed
software oracle does not prove retained command ordering.

Do not add a generic CPU-frame fallback for validation, internal, allocation,
or parity errors. Only an explicit presentation selection or a classified
retained source/shader limit may choose CPU composition. Keep visual/content
parity in the owning feature issue: a missing sprite, menu action, or simulation
state is not evidence that the presenter selected the wrong renderer.

The following are explicit scope boundaries, not open parity gaps:

- byte-identical output across unrelated GPU drivers is not promised beyond the
  documented one-byte filtering tolerance;
- the engine-state shadow-diff ABI does not compare render surfaces; rendering
  uses the independent CPU/GPU oracle described here; and
- headless output is not an interactive presentation backend.
