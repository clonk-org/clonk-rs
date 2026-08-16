use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportPointer {
    pub owner: i32,
    pub world: FloatVector2,
    pub screen: GuiPoint,
}

/// The themed C4MouseControl/C4GUI cursor cells.
/// Their numeric atlas phases are fixed in src/C4MouseControl.cpp:43-76.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseCursorPhase {
    Region,
    Crosshair,
    Enter,
    Grab,
    Chop,
    Dig,
    Build,
    Select,
    Object,
    Ungrab,
    Up,
    Down,
    Left,
    Right,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
    JumpLeft,
    JumpRight,
    Drop,
    ThrowRight,
    Put,
    Vehicle,
    VehiclePut,
    ThrowLeft,
    Point,
    DigObject,
    Help,
    DigMaterial,
    Add,
    Construct,
    Attack,
    Nothing,
}

impl MouseCursorPhase {
    pub const fn atlas_phase(self) -> i32 {
        match self {
            Self::Region => 0,
            Self::Crosshair => 1,
            Self::Enter => 2,
            Self::Grab => 3,
            Self::Chop => 4,
            Self::Dig => 5,
            Self::Build => 6,
            Self::Select => 7,
            Self::Object => 8,
            Self::Ungrab => 9,
            Self::Up => 10,
            Self::Down => 11,
            Self::Left => 12,
            Self::Right => 13,
            Self::UpLeft => 14,
            Self::UpRight => 15,
            Self::DownLeft => 16,
            Self::DownRight => 17,
            Self::JumpLeft => 18,
            Self::JumpRight => 19,
            Self::Drop => 20,
            Self::ThrowRight => 21,
            Self::Put => 22,
            Self::Vehicle => 24,
            Self::VehiclePut => 25,
            Self::ThrowLeft => 26,
            Self::Point => 27,
            Self::DigObject => 28,
            Self::Help => 29,
            Self::DigMaterial => 30,
            Self::Add => 31,
            Self::Construct => 32,
            Self::Attack => 33,
            Self::Nothing => 34,
        }
    }

    pub(crate) fn hotspot(self, cell: i32) -> (i32, i32) {
        let center = cell / 2;
        if cell == 13 {
            return match self {
                Self::Region | Self::Select => (0, 0),
                Self::Dig | Self::DigMaterial => (0, cell),
                _ => (center, center),
            };
        }
        match self {
            Self::Up => (center, 0),
            Self::Down => (center, center.saturating_add(cell / 2)),
            Self::Left => (0, center),
            Self::Right => (center.saturating_add(cell / 2), center),
            Self::UpLeft => (0, 0),
            Self::UpRight => (center.saturating_add(cell / 2), 0),
            Self::DownLeft => (0, center.saturating_add(cell / 2)),
            Self::DownRight => {
                let edge = center.saturating_add(cell / 2);
                (edge, edge)
            }
            _ => (center, center),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportEdgeScroll {
    pub delta: Vector2,
    pub cursor: MouseCursorPhase,
    pub(crate) edge_mask: u8,
}

impl ViewportEdgeScroll {
    /// Yield the native independent `ScrollView` calls in left, up, right,
    /// down order. Applying bounds after every item matters when opposite
    /// edges coincide, such as a one-pixel viewport.
    pub fn steps(self) -> ViewportEdgeScrollSteps {
        ViewportEdgeScrollSteps {
            edge_mask: self.edge_mask,
            next_edge: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ViewportEdgeScrollSteps {
    edge_mask: u8,
    next_edge: u8,
}

impl Iterator for ViewportEdgeScrollSteps {
    type Item = Vector2;

    fn next(&mut self) -> Option<Self::Item> {
        const STEPS: [Vector2; 4] = [
            Vector2 { x: -10, y: 0 },
            Vector2 { x: 0, y: -10 },
            Vector2 { x: 10, y: 0 },
            Vector2 { x: 0, y: 10 },
        ];
        while usize::from(self.next_edge) < STEPS.len() {
            let edge = self.next_edge;
            self.next_edge += 1;
            if self.edge_mask & (1 << edge) != 0 {
                return Some(STEPS[usize::from(edge)]);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let consumed = if self.next_edge >= 4 {
            u8::MAX
        } else {
            (1 << self.next_edge) - 1
        };
        let remaining = (self.edge_mask & !consumed).count_ones() as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ViewportEdgeScrollSteps {
    fn len(&self) -> usize {
        self.size_hint().0
    }
}

/// Resolve an already-stored C4MouseControl viewport coordinate without
/// clamping it to the current dimensions. This is the `Execute` repeat path:
/// a viewport resize can make the old `VpX`/`VpY` cease to be an edge.
pub fn viewport_edge_scroll_at(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Option<ViewportEdgeScroll> {
    if width <= 0 || height <= 0 {
        return None;
    }

    let max_x = width - 1;
    let max_y = height - 1;
    let left = x == 0;
    let top = y == 0;
    let right = x == max_x;
    let bottom = y == max_y;
    let edge_mask =
        u8::from(left) | (u8::from(top) << 1) | (u8::from(right) << 2) | (u8::from(bottom) << 3);

    // UpdateScrolling executes all four independent axis checks in this
    // order. Preserve that behavior even for degenerate one-pixel extents.
    let mut delta = Vector2::ZERO;
    let mut cursor = None;
    if left {
        delta.x = delta.x.saturating_sub(10);
        cursor = Some(MouseCursorPhase::Left);
    }
    if top {
        delta.y = delta.y.saturating_sub(10);
        cursor = Some(MouseCursorPhase::Up);
    }
    if right {
        delta.x = delta.x.saturating_add(10);
        cursor = Some(MouseCursorPhase::Right);
    }
    if bottom {
        delta.y = delta.y.saturating_add(10);
        cursor = Some(MouseCursorPhase::Down);
    }

    // The four exact-corner checks then overwrite the single-axis cursor.
    if left && top {
        cursor = Some(MouseCursorPhase::UpLeft);
    }
    if right && top {
        cursor = Some(MouseCursorPhase::UpRight);
    }
    if left && bottom {
        cursor = Some(MouseCursorPhase::DownLeft);
    }
    if right && bottom {
        cursor = Some(MouseCursorPhase::DownRight);
    }

    cursor.map(|cursor| ViewportEdgeScroll {
        delta,
        cursor,
        edge_mask,
    })
}

/// Resolve C4MouseControl's exact one-pixel viewport edge zones. Input is
/// first rounded upward and clamped to the inclusive viewport output bounds,
/// matching `C4GraphicsSystem::MouseMoveToViewport` at fractional scales.
pub fn viewport_edge_scroll(
    viewport: SurfaceRect,
    pointer: GuiPoint,
) -> Option<ViewportEdgeScroll> {
    if viewport.width == 0 || viewport.height == 0 {
        return None;
    }

    let width = i32::try_from(viewport.width).unwrap_or(i32::MAX);
    let height = i32::try_from(viewport.height).unwrap_or(i32::MAX);
    let max_x = width - 1;
    let max_y = height - 1;
    let x = (pointer.x.ceil() as i32)
        .saturating_sub(viewport.x)
        .clamp(0, max_x);
    let y = (pointer.y.ceil() as i32)
        .saturating_sub(viewport.y)
        .clamp(0, max_y);
    viewport_edge_scroll_at(x, y, width, height)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceRect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl SourceRect {
    pub(crate) fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// `C4Math::Angle(0, 0, x, y)`: zero points up, clockwise positive.
pub(crate) fn c4_particle_angle(x: i32, y: i32) -> i32 {
    let absolute_x = x.wrapping_abs() as f32;
    let absolute_y = y.wrapping_abs() as f32;
    let angle = (180.0_f64
        * f64::from(absolute_y.atan2(absolute_x))
        * f64::from(std::f32::consts::FRAC_1_PI)) as i32;
    if x > 0 {
        if y < 0 {
            90 - angle
        } else {
            90 + angle
        }
    } else if y < 0 {
        270 + angle
    } else {
        270 - angle
    }
}

pub(crate) fn lower_bounded_surface_clip(surface: &Surface, left: i32, top: i32) -> SurfaceRect {
    let width = (i64::from(surface.width()) - i64::from(left)).clamp(0, i64::from(u32::MAX)) as u32;
    let height =
        (i64::from(surface.height()) - i64::from(top)).clamp(0, i64::from(u32::MAX)) as u32;
    SurfaceRect::new(left, top, width, height)
}

/// Floating-point source geometry used by C4Object face blits and
/// `C4Facet::DrawXFloat`. C++ forwards `GetGraphics()->pDef->Scale` into
/// `Blit` without quantizing object source rectangles
/// (C4Object.cpp:438-467,2639-2670), while DrawXFloat proportionally crops its
/// facet before the same float-source blit (C4Facet.cpp:306-319).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FloatSourceRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl FloatSourceRect {
    pub(crate) fn scaled(source: SourceRect, scale: f32) -> Self {
        Self {
            x: source.x as f32 * scale,
            y: source.y as f32 * scale,
            width: source.width as f32 * scale,
            height: source.height as f32 * scale,
        }
    }

    pub(crate) fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }

    /// `CStdDDraw::Blit` insets ordinary source facets by half a texel on
    /// each side when the application scale is not 100%
    /// (src/StdDDraw2.cpp:676-688). This happens before exactness, tile
    /// intersection, and texture-coordinate calculation.
    ///
    /// C++'s own `noScalingCorrection` argument already suppresses the inset
    /// per call site; `Graphics.HDExactBlits` additionally suppresses it for
    /// blits that map one authored texel to one device pixel, where the inset
    /// is a pure drift (see `GraphicsSystem::runtime_sprite_blit`).
    pub(crate) fn with_scaling_correction(mut self, enabled: bool) -> Self {
        if enabled {
            if self.width > 1.0 {
                self.x += 0.5;
                self.width -= 1.0;
            }
            if self.height > 1.0 {
                self.y += 0.5;
                self.height -= 1.0;
            }
        }
        self
    }

    pub(crate) fn source_edge(
        self,
        normalized_x: f32,
        normalized_y: f32,
        flip_x: bool,
    ) -> (f32, f32) {
        let x = if flip_x {
            self.x + self.width * (1.0 - normalized_x)
        } else {
            self.x + self.width * normalized_x
        };
        (x, self.y + self.height * normalized_y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CameraKey {
    /// Legacy/snapshot-derived identity for a player's stable camera slot.
    Player { owner: i32, slot: usize },
    /// App-owned identity for one concrete native `C4Viewport` object.
    Physical { identity: u64, slot: usize },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CameraState {
    pub(crate) d_view_x: C4Fixed,
    pub(crate) d_view_y: C4Fixed,
    pub(crate) view_x: i32,
    pub(crate) view_y: i32,
    pub(crate) view_width: i32,
    pub(crate) view_height: i32,
}

impl CameraState {
    /// `CreateViewport` calls `CenterPosition` after setting the output size,
    /// while dViewX/Y retain C4Viewport's negative initialization sentinel.
    pub(crate) fn new(
        world_width: i32,
        world_height: i32,
        view_width: i32,
        view_height: i32,
    ) -> Self {
        Self {
            d_view_x: itofix(CAMERA_UNINITIALIZED),
            d_view_y: itofix(CAMERA_UNINITIALIZED),
            view_x: (world_width - view_width) / 2,
            view_y: (world_height - view_height) / 2,
            view_width,
            view_height,
        }
    }

    pub(crate) fn update(
        &mut self,
        center_x: i32,
        center_y: i32,
        view_width: i32,
        view_height: i32,
        world_width: i32,
        world_height: i32,
        scroll_border: i32,
        scrolling: bool,
        scroll_smooth: i32,
    ) -> (i32, i32) {
        // SetOutputSize keeps the previous visible center. It adjusts the
        // integer ViewX/Y but deliberately does not rewrite dViewX/Y.
        self.resize_output(view_width, view_height);

        let scroll_range = if scrolling {
            0
        } else {
            (view_width / 10).min(view_height / 10)
        };
        let target_x = classic_camera_target_axis(
            self.view_x,
            center_x,
            view_width,
            world_width,
            scroll_range,
            scroll_border,
            scrolling,
        );
        let target_y = classic_camera_target_axis(
            self.view_y,
            center_y,
            view_height,
            world_height,
            scroll_range,
            scroll_border,
            scrolling,
        );
        let divisor = scroll_smooth.clamp(1, 50);

        // C4Viewport uses the sign of both fixed coordinates as its coupled
        // initialization test. This also means a negative border position
        // takes the snap branch on every graphics pass.
        if self.d_view_x >= 0 && self.d_view_y >= 0 {
            self.d_view_x += (itofix(target_x) - self.d_view_x) / divisor;
            self.d_view_y += (itofix(target_y) - self.d_view_y) / divisor;
            self.view_x = fixtoi(self.d_view_x);
            self.view_y = fixtoi(self.d_view_y);
        } else {
            self.view_x = target_x;
            self.view_y = target_y;
            self.d_view_x = itofix(target_x);
            self.d_view_y = itofix(target_y);
        }

        (self.view_x, self.view_y)
    }

    pub(crate) fn resize_output(&mut self, view_width: i32, view_height: i32) {
        if self.view_width != view_width {
            self.view_x += (self.view_width - view_width) / 2;
            self.view_width = view_width;
        }
        if self.view_height != view_height {
            self.view_y += (self.view_height - view_height) / 2;
            self.view_height = view_height;
        }
    }

    /// No-owner fullscreen viewports are not player-locked. Without an
    /// explicit FreeScroll input they retain their centered position, and
    /// UpdateViewPosition hard-clamps large worlds while centering small ones.
    /// `C4Viewport::UpdateViewPosition`'s ownerless arm
    /// (`C4Viewport.cpp:1234-1254`).
    ///
    /// Centring an undersized map is gated on `Application.isFullScreen`
    /// (`:1237`, `:1246`). A detached console viewport window takes the other
    /// arm — `min(ViewX, GBackWdt - ViewWdt)` then `max(ViewX, 0)` — which
    /// pins the origin at 0 rather than scrolling the map into the middle of
    /// a window larger than the world.
    pub(crate) fn no_owner_position(
        &mut self,
        view_width: i32,
        view_height: i32,
        world_width: i32,
        world_height: i32,
        fullscreen: bool,
    ) -> (i32, i32) {
        self.resize_output(view_width, view_height);
        self.view_x = if fullscreen && world_width < view_width {
            (world_width - view_width) / 2
        } else {
            // `min` then `max`, as C++ writes it: `clamp` would panic once the
            // world is narrower than the view.
            self.view_x.min(world_width - view_width).max(0)
        };
        self.view_y = if fullscreen && world_height < view_height {
            (world_height - view_height) / 2
        } else {
            self.view_y.min(world_height - view_height).max(0)
        };
        (self.view_x, self.view_y)
    }

    /// A temporary `SetFilmView(NO_OWNER)` on an owned viewport disables
    /// player tracking without turning it into a classified observer
    /// viewport. Preserve its current center across output-size changes.
    pub(crate) fn stationary_position(&mut self, view_width: i32, view_height: i32) -> (i32, i32) {
        self.resize_output(view_width, view_height);
        (self.view_x, self.view_y)
    }
}

/// C4Viewport::AdjustPosition's per-axis dead-zone and progressive edge
/// bounds (src/C4Viewport.cpp:1165-1201). Inputs and the result are whole
/// world pixels; the 16.16 filter is applied afterwards.
fn classic_camera_target_axis(
    current_view: i32,
    center: i32,
    view_extent: i32,
    world_extent: i32,
    scroll_range: i32,
    scroll_border: i32,
    scrolling: bool,
) -> i32 {
    let mut extra_bound = if scrolling { scroll_border } else { 0 };
    if !scrolling {
        if center < scroll_border {
            extra_bound = (scroll_border - center).min(scroll_border);
        } else if center >= world_extent - scroll_border {
            extra_bound = (center - world_extent).min(0) + scroll_border;
        }
    }
    extra_bound = extra_bound.max((view_extent - world_extent) / 2 + 1);

    let desired = center - view_extent / 2;
    let target = current_view.clamp(desired - scroll_range, desired + scroll_range);
    let min_view = -extra_bound;
    let max_view = world_extent - view_extent + extra_bound;
    if min_view <= max_view {
        target.clamp(min_view, max_view)
    } else {
        // The oversized-world rule above normally prevents an inverted
        // range. Keep the centered C++ fallback for defensive malformed
        // dimensions rather than panicking in `i32::clamp`.
        (world_extent - view_extent) / 2
    }
}

pub(crate) fn scaled_camera_border(border: i32, zoom: f32, output_extent: u32) -> u32 {
    (border.max(0) as f32 * zoom)
        .round()
        .clamp(0.0, output_extent as f32) as u32
}

/// Which sized cursor sheet a display resolution selects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorTiers {
    /// `C4GraphicsResource::ReloadResolutionDependentFiles` exactly: the
    /// sheet only ever grows with `Graphics.Scale`, never with the panel's
    /// own pixel count (src/C4GraphicsResource.cpp:468-491).
    #[default]
    Classic,
    /// Remaster divergence: select by physical width so the pointer keeps
    /// the angular size C++ gives it at 1280 (a 50px cell, 3.90625% of the
    /// screen width) all the way up the shipped ladder. The eight sized
    /// sheets are authored at exactly that ratio — 28/40/50/75/100/150/225/338
    /// px cells — so every tier is an exact 1:1 blit of existing art.
    HighDpi,
}

#[derive(Debug, Clone, Default)]
pub struct CursorAtlas {
    images: Vec<Option<ImageData>>,
}

impl CursorAtlas {
    pub fn new(images: Vec<Option<ImageData>>) -> Self {
        Self { images }
    }

    pub fn empty() -> Self {
        Self { images: Vec::new() }
    }

    /// PreInit's sized cursor path succeeds only when all eight classic
    /// resolution sheets loaded; a partial set is not a usable fallback.
    pub fn is_complete(&self) -> bool {
        self.images.len() == 8 && self.images.iter().all(Option::is_some)
    }

    pub fn image_for_resolution(&self, width: u32) -> Option<ImageData> {
        self.image_for_scaled_resolution(width, 1.0)
    }

    /// Select the resolution-dependent cursor sheet like
    /// `C4GraphicsResource::ReloadResolutionDependentFiles`
    /// (src/C4GraphicsResource.cpp:468-504).
    pub fn image_for_scaled_resolution(&self, logical_width: u32, scale: f32) -> Option<ImageData> {
        self.image_for_tiers(logical_width, scale, CursorTiers::Classic)
    }

    pub fn image_for_tiers(
        &self,
        logical_width: u32,
        scale: f32,
        tiers: CursorTiers,
    ) -> Option<ImageData> {
        let index = Self::index_for_tiers(logical_width, scale, tiers);
        self.images.get(index).and_then(Clone::clone)
    }

    pub(crate) fn index_for_tiers(logical_width: u32, scale: f32, tiers: CursorTiers) -> usize {
        if tiers == CursorTiers::Classic {
            return Self::index_for_scaled_resolution(logical_width, scale);
        }
        // Physical width at which each sheet's cell first reaches C++'s
        // 50px-at-1280 ratio: 1920/2560/3840/5760/8653 for the 75/100/150/
        // 225/338px cells. Below the C++ breakpoint the classic ladder
        // already tracks the panel, so it is kept verbatim.
        const LADDER: [(f32, usize); 5] = [
            (8653.0, 0),
            (5760.0, 1),
            (3840.0, 2),
            (2560.0, 3),
            (1920.0, 4),
        ];
        let scale = if scale.is_finite() {
            scale.max(f32::EPSILON)
        } else {
            1.0
        };
        let physical_width = logical_width as f32 * scale;
        if physical_width <= 1280.0 {
            return Self::index_for_scaled_resolution(logical_width, scale);
        }
        LADDER
            .iter()
            .find(|(width, _)| physical_width >= *width)
            .map_or(5, |&(_, index)| index)
    }

    pub(crate) fn index_for_scaled_resolution(logical_width: u32, scale: f32) -> usize {
        const DEFAULT_INDEX: usize = 5;
        const BREAKPOINTS: [f32; 2] = [1280.0, 800.0];

        let scale = if scale.is_finite() {
            scale.max(f32::EPSILON)
        } else {
            1.0
        };
        let physical_width = logical_width as f32 * scale;
        let mut index = DEFAULT_INDEX;
        if physical_width > BREAKPOINTS[0] {
            let scale_shift = (scale.max(1.0) - 0.5) as usize;
            index -= index.min(scale_shift);
        } else {
            for &bp in &BREAKPOINTS {
                if physical_width >= bp {
                    break;
                }
                index += 1;
            }
        }
        index
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyRenderState {
    pub(crate) settings: SkySettings,
    image: Option<ImageData>,
    image_is_fully_opaque: bool,
}

impl SkyRenderState {
    pub fn new(settings: SkySettings, image: Option<ImageData>) -> Self {
        let image_is_fully_opaque = image.as_ref().is_some_and(|image| {
            if image.width() == 0 || image.height() == 0 {
                return false;
            }
            let Some(expected_len) = (image.width() as usize)
                .checked_mul(image.height() as usize)
                .and_then(|pixels| pixels.checked_mul(4))
            else {
                return false;
            };
            image.pixels().len() == expected_len
                && image.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255)
        });
        Self {
            settings,
            image,
            image_is_fully_opaque,
        }
    }

    pub fn settings(&self) -> &SkySettings {
        &self.settings
    }

    pub fn image(&self) -> Option<&ImageData> {
        self.image.as_ref()
    }

    pub(crate) fn image_is_fully_opaque(&self) -> bool {
        self.image_is_fully_opaque
    }
}

/// `C4MessageBoard::iMode` (src/C4MessageBoard.cpp:65-125).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageBoardMode {
    /// The standard one-line board whose current message slides and fades.
    #[default]
    SingleLine,
    /// The runtime-only continuous board selected by `/msgboard <n>` for
    /// `n >= 2`.
    Continuous,
    /// No board output, except for the native type-in fallthrough.
    Hidden,
}

/// Immutable `C4MessageBoard::Draw` input assembled by the application.
///
/// `log_lines` are stored oldest-to-newest. `back_scroll == 0` addresses the
/// newest physical line, matching `C4LogBuffer::GetLine(-1)`; positive values
/// walk toward older lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageBoardOverlay {
    pub mode: MessageBoardMode,
    pub line_count: i32,
    pub log_lines: Vec<String>,
    pub back_scroll: i32,
    pub fader: i32,
    pub screen_fader: i32,
    pub type_in: bool,
}

impl Default for MessageBoardOverlay {
    fn default() -> Self {
        Self {
            mode: MessageBoardMode::SingleLine,
            line_count: 4,
            log_lines: Vec::new(),
            back_scroll: -1,
            fader: 0,
            screen_fader: 0,
            type_in: false,
        }
    }
}

impl MessageBoardOverlay {
    /// `MessageBoard.Output.Hgt` after `ChangeMode`: one FontRegular line in
    /// standard mode, `(iLines + 1)` lines in continuous mode, and zero while
    /// hidden (src/C4MessageBoard.cpp:65-125).
    pub fn output_height(&self, line_height: i32) -> i32 {
        let line_height = line_height.max(0);
        match self.mode {
            MessageBoardMode::SingleLine => line_height,
            MessageBoardMode::Continuous => self
                .line_count
                .max(2)
                .saturating_add(1)
                .saturating_mul(line_height),
            MessageBoardMode::Hidden => 0,
        }
    }
}

pub struct GraphicsOverlay<'a> {
    /// Developer-only FRAME/POS/VEL text. It is additive and must never carry
    /// classic help, flash, network, or error presentation.
    pub frame_text: &'a str,
    /// Developer-only ENERGY/DAMAGE/OWNER text. It is additive and must never
    /// replace classic status presentation.
    pub status_text: &'a str,
    /// Opt-in developer HUD lines (not part of the C++-faithful overlay).
    /// Callers keep this false for parity/compatibility launches.
    pub debug_hud: bool,
    /// `false` for `Head.Film && Head.Replay`: suppresses the per-viewport
    /// player HUD and world cursor/select marks while leaving global chrome
    /// to its independent `C4GraphicsSystem` pass.
    pub viewport_overlays_visible: bool,
    pub players: Vec<PlayerOverlay>,
    /// Precomposed `C4Object::DrawTopFace` crew labels. The app owns the
    /// player-info/invisibility and hostility policy; the renderer owns the
    /// object-space gates, placement and color (src/C4Object.cpp:2582-2612).
    pub crew_name_labels: Vec<CrewNameOverlay>,
    /// Process-local voice activity projected onto selected crew objects.
    /// This is independent of `players`: ordinary owned viewports retain only
    /// their own [`PlayerOverlay`], while every visible speaking crew member
    /// needs a world-space indicator.
    pub speaking: SpeakingOverlay,
    /// `Game.Time` seconds for the upper board clock
    /// (C4Game::Sec1Timer, src/C4Game.cpp:1737-1741).
    pub game_time_seconds: u64,
    /// Complete `C4MessageBoard::Draw` projection, including continuous-mode
    /// history and the two native faders (src/C4MessageBoard.cpp:243-306).
    pub message_board: MessageBoardOverlay,
    /// Local wall-clock text (`[%H:%M:%S]`) shown by `C4UpperBoard::Draw`.
    /// `None` is `Config.Graphics.ShowClock == false`.
    pub clock_text: Option<String>,
    /// Sampled `C4Game::FPS`; `None` is `Config.General.FPS == false`.
    pub frames_per_second: Option<i32>,
    /// `Config.Graphics.UpperBoard`, including its viewport/message split.
    pub upper_board_mode: hud::UpperBoardMode,
    /// `Config.Graphics.ShowPortraits` (src/C4Config.cpp:448) — shifts the
    /// viewport bars down ten pixels when portraits are enabled.
    pub show_portraits: bool,
    /// `Config.Graphics.ShowCommands` (src/C4Config.cpp:449) — gates the
    /// per-viewport command rows (src/C4Viewport.cpp:948).
    pub show_commands: bool,
    /// `Config.Graphics.ShowCommandKeys` (src/C4Config.cpp:450) — key names
    /// on the command key caps (src/C4ObjectCom.cpp:942).
    pub show_command_keys: bool,
}

/// One world-space crew-name label after the app has applied the two display
/// toggles and player-level visibility rules. `visible_to` contains viewport
/// player numbers; include [`OWNER_NONE`] for the fullscreen observer view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrewNameOverlay {
    pub object_id: ObjectId,
    pub text: String,
    pub visible_to: Vec<i32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpeakingOverlay {
    pub object_ids: Vec<ObjectId>,
    /// Complete Graphics.c4g/GUIIcons.png sheet. The renderer extracts the
    /// classic `Ico_Sound` phase so runtime graphics overloads remain active.
    pub gui_icons: Option<ImageData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerOverlay {
    pub owner: i32,
    pub name: String,
    pub wealth: i32,
    pub score: i32,
    /// Raw `C4Player::ViewWealth`; this requests the wealth item even when
    /// `Config.Graphics.ShowPlayerHUDAlways` is disabled.
    pub view_wealth: bool,
    /// Effective `Game.C4S.Game.ValueGain && C4Player::ViewValue`; this
    /// requests the score item when the global always-visible HUD is disabled.
    pub view_value: bool,
    /// Effective HUD cursor (`ViewCursor ?: Cursor`). Commands still come
    /// from the player's real cursor (src/C4Viewport.cpp:891-897,947-961).
    pub cursor: Option<ObjectId>,
    /// Saved `C4Player::Captain`; the cursor-info star is shown only when
    /// this exact object is the active cursor (src/C4Viewport.cpp:904-907).
    pub captain: Option<ObjectId>,
    pub eliminated: bool,
    pub owner_color: Color,
    /// `C4Player::SelectCount` for the crew display value
    /// (src/C4Viewport.cpp:1320).
    pub select_count: i32,
    /// `C4Player::ShowStartup` — device hint + name until the first control
    /// com (src/C4Player.cpp:1376, src/C4Viewport.cpp:1450).
    pub show_startup: bool,
    /// Effective `C4Player::Control` set used to select the keyboard/gamepad
    /// startup phase. Keyboard1-4 are 0..=3; GamePad1-4 are 4..=7.
    pub control_set: i32,
    /// C++ truthiness of the raw `C4Player::MouseControl` integer. The mouse
    /// startup symbol co-renders with the selected keyboard/gamepad phase.
    pub mouse_control: bool,
    /// `C4Player::ShowControl` and `ShowControlPos`, consumed by
    /// `C4Viewport::DrawPlayerControls` (src/C4Viewport.cpp:1394-1441).
    pub show_control: i32,
    pub show_control_position: i32,
    /// Raw `C4Player::LastCom`; `Com2Control` selects the pressed hint.
    pub last_com: i32,
    /// Short `PlrControlKeyName` values in CON_* order.
    pub control_key_labels: Vec<String>,
    /// Actual player crew count; `crew` may additionally carry a non-roster
    /// ViewCursor so its HUD data can be presented without inflating this.
    pub crew_count: i32,
    pub crew: Vec<CrewOverlay>,
    /// The cursor object's contextual command icons
    /// (C4Object::DrawCommands, src/C4Object.cpp:2940-3098), resolved by
    /// the app; drawn into the viewport command rows when ShowCommands.
    pub commands: Vec<CommandIcon>,
    /// C4Player::FlashCom for the owner of the object producing `commands`.
    pub flash_command: i32,
}

pub(crate) const fn player_fixed_item_visibility(
    show_player_hud_always: bool,
    view_wealth: bool,
    view_value: bool,
) -> (bool, bool, bool) {
    (
        show_player_hud_always || view_wealth,
        show_player_hud_always || view_value,
        show_player_hud_always,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrewOverlay {
    pub object_id: ObjectId,
    /// The crew member's name (`C4ObjectInfo::sName`).
    pub label: String,
    /// Raw `C4Object::Energy` and resolved `GetPhysical()->Energy`.
    pub energy: i32,
    pub energy_capacity: i32,
    /// `C4Object::ViewEnergy`, the transient bar timer. `DrawCursorInfo` draws
    /// the three status bars only while it is live or
    /// `Config.Graphics.ShowPlayerHUDAlways` is set (C4Viewport.cpp:921).
    pub view_energy: i32,
    /// Raw `C4Object::MagicEnergy` and resolved `GetPhysical()->Magic`.
    /// A non-zero level inserts the optional middle HUD bar
    /// (src/C4Viewport.cpp:934-938; src/C4Object.cpp:2722-2726).
    pub magic_energy: i32,
    pub magic_capacity: i32,
    /// Raw `C4Object::Breath` and resolved `GetPhysical()->Breath`.
    /// C++ draws this bar only while breath is non-zero and below capacity
    /// (src/C4Viewport.cpp:939-943; src/C4Object.cpp:2728-2731).
    pub breath: i32,
    pub breath_capacity: i32,
    pub is_focus: bool,
    /// Raw cursor definition `HideHUDElements`/`HideHUDBars` masks.
    pub hide_hud_elements: i32,
    pub hide_hud_bars: i32,
    pub portrait: Option<ImageData>,
    /// Raw owner-color surface paired with `portrait`, drawn as C++'s second
    /// filtered pass before applying `portrait_owner_color`.
    pub portrait_owner_overlay: Option<ImageData>,
    /// Packed C4 player `ColorDw`, applied after filtering the owner surface.
    pub portrait_owner_color: u32,
    /// `C4ObjectInfo::Rank` (src/C4ObjectInfo.cpp:330).
    pub rank: i32,
    /// The def's own rank symbols (`pDef->pRankSymbols`,
    /// src/C4ObjectInfo.cpp:334-341); falls back to the global Rank.png.
    pub rank_symbols: Option<ImageData>,
    /// Extension-adjusted base phase count (`pDef->iNumRankSymbols`).
    /// `None` uses the selected strip's raw phase count.
    pub rank_symbol_count: Option<u32>,
    /// `cursor->Info` presence + `Info->sName`: the red cursor label above
    /// the flashing mark draws only for crew with an object info
    /// (C4Game::DrawCursors, src/C4Game.cpp:1873-1887).
    pub info_name: Option<String>,
    /// `Info->sRankName` for the extra rank line when `Rank > 0`
    /// (src/C4Game.cpp:1877-1881).
    pub rank_name: Option<String>,
    /// The grouped sections of `cursor->Contents.DrawIDList`
    /// (src/C4Viewport.cpp:911-917; src/C4ObjectList.cpp:343-372).
    pub inventory: Vec<InventoryOverlay>,
}

/// Presentation data for one grouped cursor-inventory section. The first
/// object represents the group, matching `C4ObjectListIterator::GetNext`
/// (src/C4ObjectList.cpp:849-903).
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryOverlay {
    pub object_id: ObjectId,
    pub definition_id: DefinitionId,
    pub picture: Option<ImageData>,
    /// Direct `C4ObjectList::DrawIDList` keeps the object's additive bit for
    /// the final draw onto the viewport framebuffer.
    pub additive: bool,
    /// Prepared picture overlays retain their individual framebuffer blend
    /// bit so mixed normal/additive object pictures can be drawn in order.
    pub picture_overlays: Vec<InventoryPictureOverlay>,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InventoryPictureOverlay {
    pub picture: ImageData,
    pub additive: bool,
}

#[derive(Debug)]
pub struct ViewportInput<'a> {
    pub owner: i32,
    pub center: Vector2,
    pub offset: Vector2,
    pub zoom: f32,
    /// Object anchor used for player selection presentation. Native
    /// `NO_OWNER` observer viewports do not require one.
    pub focus: Option<&'a ObjectSnapshot>,
    /// Stable physical viewport identity. `SetFilmView` changes the player
    /// assigned to a viewport without replacing that viewport or resetting
    /// its smoothing state.
    pub(crate) camera_identity: Option<CameraKey>,
    /// C4Viewport::fIsNoOwnerViewport is independent from its temporary
    /// Player assignment. Film view switches preserve this classification.
    pub(crate) is_no_owner_viewport: bool,
    /// `C4PVM_Scrolling` removes the normal camera dead zone and enables the
    /// fixed fullscreen scroll border in C4Viewport::AdjustPosition.
    pub(crate) scrolling: bool,
    /// `C4Viewport::PlayerLock`, which starts set (`C4Viewport::Default`,
    /// `C4Viewport.cpp:1272`). Only a console viewport window ever clears it,
    /// and clearing it is what stops the view following its player.
    pub(crate) player_lock: bool,
}

impl<'a> ViewportInput<'a> {
    pub fn new(owner: i32, center: Vector2, zoom: f32, focus: &'a ObjectSnapshot) -> Self {
        Self {
            owner,
            center,
            offset: Vector2::ZERO,
            zoom,
            focus: Some(focus),
            camera_identity: None,
            is_no_owner_viewport: owner == OWNER_NONE,
            scrolling: false,
            player_lock: true,
        }
    }

    /// Construct C4FullScreen's object-independent `NO_OWNER` observer
    /// viewport. Its camera is centered and clamped from landscape geometry;
    /// no live object is required merely to anchor rendering.
    pub fn ownerless(center: Vector2, zoom: f32) -> Self {
        Self {
            owner: OWNER_NONE,
            center,
            offset: Vector2::ZERO,
            zoom,
            focus: None,
            camera_identity: None,
            is_no_owner_viewport: true,
            scrolling: false,
            player_lock: true,
        }
    }

    /// Construct a player-owned physical viewport whose view center exists
    /// independently of a live cursor/crew object. This is required while a
    /// focusless player remains in `C4PVM_Scrolling`.
    pub fn owned_without_focus(owner: i32, center: Vector2, zoom: f32) -> Self {
        Self {
            owner,
            center,
            offset: Vector2::ZERO,
            zoom,
            focus: None,
            camera_identity: None,
            is_no_owner_viewport: false,
            scrolling: false,
            player_lock: true,
        }
    }

    pub fn with_offset(mut self, offset: Vector2) -> Self {
        self.offset = offset;
        self
    }

    /// Bind this input to an existing physical viewport across temporary
    /// player retargets.
    pub fn with_camera_identity(mut self, owner: i32, slot: usize) -> Self {
        self.camera_identity = Some(CameraKey::Player { owner, slot });
        self
    }

    /// Bind this input to one concrete physical viewport. Player numbers may
    /// be reused while an older film-retargeted viewport remains alive, so
    /// they cannot identify native viewport-owned smoothing state.
    pub fn with_physical_camera_identity(mut self, identity: u64, slot: usize) -> Self {
        self.camera_identity = Some(CameraKey::Physical { identity, slot });
        self
    }

    pub fn with_scrolling(mut self, scrolling: bool) -> Self {
        self.scrolling = scrolling;
        self
    }

    pub fn set_scrolling(&mut self, scrolling: bool) {
        self.scrolling = scrolling;
    }

    /// `C4Viewport::PlayerLock`. An unlocked viewport stops following its
    /// player and keeps the position its scroll bars left
    /// (`C4Viewport.cpp:1162`).
    pub fn with_player_lock(mut self, player_lock: bool) -> Self {
        self.player_lock = player_lock;
        self
    }

    pub fn from_focus(focus: &'a ObjectSnapshot) -> Self {
        Self {
            owner: focus.owner,
            center: Vector2::new(focus.position.x, focus.position.y),
            offset: Vector2::ZERO,
            zoom: 1.0,
            focus: Some(focus),
            camera_identity: None,
            is_no_owner_viewport: focus.owner == OWNER_NONE,
            scrolling: false,
            player_lock: true,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveViewport {
    pub(crate) owner: i32,
    pub(crate) focus: Option<ObjectId>,
    pub(crate) rect: SurfaceRect,
    pub(crate) content_rect: SurfaceRect,
    pub(crate) target_x: i32,
    pub(crate) target_y: i32,
    pub(crate) logical_width: i32,
    pub(crate) logical_height: i32,
    pub(crate) world_width: i32,
    pub(crate) world_height: i32,
    pub(crate) viewport_x: f32,
    pub(crate) viewport_y: f32,
    pub(crate) zoom: f32,
    pub(crate) camera_key: CameraKey,
    pub(crate) is_no_owner_viewport: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudibilityFacet {
    pub(crate) target_x: i32,
    pub(crate) target_y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// Immutable projection data for one exact active viewport record.
///
/// Owners are not unique: split-screen can create multiple viewports for one
/// player, so callers that need C4Viewport-faithful routing must iterate these
/// records instead of looking a viewport up by owner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveViewportProjection {
    pub index: usize,
    pub owner: i32,
    /// The concrete `C4Viewport`'s app-owned identity, when it has one.
    ///
    /// This is the only stable handle a detached console window has:
    /// [`Self::index`] is the *rendered layout* order and moves whenever the
    /// layout is recalculated, and [`Self::owner`] repeats when two viewports
    /// follow the same player. Address a viewport by this, not by either of
    /// those (`C4Viewport.cpp` gives each window its own object).
    pub identity: Option<u64>,
    /// Physical `C4Viewport::fIsNoOwnerViewport` classification. Temporary
    /// film-view player assignment does not change it.
    pub is_no_owner_viewport: bool,
    pub rect: SurfaceRect,
    pub content_rect: SurfaceRect,
    pub target_x: i32,
    pub target_y: i32,
    pub logical_width: i32,
    pub logical_height: i32,
    pub content_origin_x: f32,
    pub content_origin_y: f32,
    pub zoom: f32,
}

/// One ordered draw-time `C4Object::SetAudibilityAt` call. The application
/// reduces these after the completed graphics pass, when every active
/// viewport/listener is available, and retains that result across skipped
/// graphics passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedAudibilityCall {
    /// A non-parallax line endpoint, mixed through all active viewports.
    World { point: Vector2 },
    /// A parallax object/line point and the center of its exact draw facet.
    Parallax {
        point: Vector2,
        rendered_center: Vector2,
    },
}

pub type RenderedObjectAudibilityCalls = HashMap<ObjectId, Vec<RenderedAudibilityCall>>;

impl ActiveViewportProjection {
    /// C4Facet logical output bounds (`TargetX/Y`, `Wdt/Hgt`).
    pub fn contains_logical_point(self, position: Vector2) -> bool {
        let x = position.x - self.target_x;
        let y = position.y - self.target_y;
        x >= 0 && y >= 0 && x < self.logical_width && y < self.logical_height
    }

    /// Map a C4Facet-space point into this record's physical output.
    pub fn logical_to_output(self, position: Vector2) -> (f32, f32) {
        (
            (position.x as f32 - self.content_origin_x) * self.zoom + self.content_rect.x as f32,
            (position.y as f32 - self.content_origin_y) * self.zoom + self.content_rect.y as f32,
        )
    }

    pub fn contains_output_point(self, point: (f32, f32)) -> bool {
        point.0 >= self.rect.x as f32
            && point.1 >= self.rect.y as f32
            && point.0 < self.rect.x as f32 + self.rect.width as f32
            && point.1 < self.rect.y as f32 + self.rect.height as f32
    }

    /// This viewport's own pointer projection.
    ///
    /// `C4Viewport`'s window handlers convert a window-local pointer through
    /// *that viewport's* `ViewX`/`ViewY` and the application scale
    /// (`C4Viewport.cpp:112,181,192`), never through the last globally
    /// rendered layout.
    pub fn pointer_projection(self, scale: f32) -> crate::viewport_projection::ViewportProjection {
        crate::viewport_projection::ViewportProjection {
            view_x: self.target_x,
            view_y: self.target_y,
            scale,
        }
    }
}

/// One console viewport window's completed frame.
///
/// `C4Viewport::Execute` selects that viewport's own rendering context, draws
/// into a `cgo` covering the whole window, and blits it
/// (`C4Viewport.cpp:1126-1155`). The port has no per-window GL context, so the
/// drawn pixels and the projection they were drawn with travel together.
#[derive(Debug, Clone)]
pub struct DetachedViewportFrame {
    /// The window-sized target the viewport was drawn into.
    pub surface: Surface,
    /// The projection this frame was drawn with. Pointer input for the window
    /// must be converted through this, not through the fullscreen layout.
    pub projection: ActiveViewportProjection,
}
