//! Pure model and geometry for the classic startup Options **Graphics** sheet.
//!
//! The owning options dialog is responsible for rendering, focus, sounds,
//! popups, and applying display changes to the host window. This module keeps
//! the directly-bound C++ configuration values and the scale-test proposal
//! separate so moving the scale slider cannot accidentally persist an
//! unconfirmed value.

use crate::classic_gui::IntRect;
use crate::GuiPoint;

/// `C4StartupOptionsDlg.cpp`'s scale-spinbox limits.
pub const MIN_GRAPHICS_SCALE_PERCENT: i32 = 100;
/// Deliberate divergence: C++ caps the scale spinbox and slider at 300
/// (`constexpr int maxScale = 300`, C4StartupOptionsDlg.cpp:132). The first-run
/// scale follows the monitor's pixel density, so a 4x panel clamps to the C++
/// ceiling and renders a smaller-than-classic logical layout. Raising the cap
/// to 400 only widens what the Options dialog can express; the slider mapping,
/// the spinbox clamp, and the scale-test flow are unchanged.
pub const MAX_GRAPHICS_SCALE_PERCENT: i32 = 400;
/// `Config.Graphics.SmokeLevel` range exposed by the Options slider.
pub const MIN_SMOKE_LEVEL: i32 = 0;
pub const MAX_SMOKE_LEVEL: i32 = 300;

/// `Config.Graphics.UseDisplayMode` in combo-box order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GraphicsDisplayMode {
    #[default]
    Fullscreen,
    Window,
}

impl GraphicsDisplayMode {
    pub const ALL: [Self; 2] = [Self::Fullscreen, Self::Window];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fullscreen => "Fullscreen",
            Self::Window => "Window",
        }
    }

    /// Native `StdEnumEntry` spelling written by `C4ConfigGraphics`.
    pub const fn config_value(self) -> &'static str {
        self.label()
    }

    /// Accept both the native names and the legacy numeric enum values.
    pub fn from_config(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fullscreen" | "0" => Some(Self::Fullscreen),
            "window" | "windowed" | "1" => Some(Self::Window),
            _ => None,
        }
    }
}

/// Every directly-bound Graphics checkbox, in native construction order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsCheckboxId {
    AddNewCrewPortraits,
    SaveDefaultPortraits,
    AutoFrameSkip,
    ShowFolderMaps,
    DisableGamma,
    FireParticles,
}

impl GraphicsCheckboxId {
    pub const ALL: [Self; 6] = [
        Self::AddNewCrewPortraits,
        Self::SaveDefaultPortraits,
        Self::AutoFrameSkip,
        Self::ShowFolderMaps,
        Self::DisableGamma,
        Self::FireParticles,
    ];

    pub const OPTIONS: [Self; 5] = [
        Self::AddNewCrewPortraits,
        Self::SaveDefaultPortraits,
        Self::AutoFrameSkip,
        Self::ShowFolderMaps,
        Self::DisableGamma,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::AddNewCrewPortraits => 0,
            Self::SaveDefaultPortraits => 1,
            Self::AutoFrameSkip => 2,
            Self::ShowFolderMaps => 3,
            Self::DisableGamma => 4,
            Self::FireParticles => 5,
        }
    }

    pub const fn config_key(self) -> &'static str {
        match self {
            Self::AddNewCrewPortraits => "AddNewCrewPortraits",
            Self::SaveDefaultPortraits => "SaveDefaultPortraits",
            Self::AutoFrameSkip => "AutoFrameSkip",
            Self::ShowFolderMaps => "ShowFolderMaps",
            Self::DisableGamma => "DisableGamma",
            Self::FireParticles => "FireParticles",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::AddNewCrewPortraits => "Add new portraits",
            Self::SaveDefaultPortraits => "Store portraits",
            Self::AutoFrameSkip => "Automatic frame skip",
            Self::ShowFolderMaps => "Show folder maps",
            Self::DisableGamma => "Disable gamma",
            Self::FireParticles => "Fire particles",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsSliderId {
    Scale,
    SmokeLevel,
}

/// A horizontal `C4GUI::ScrollBar` pointer region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsSliderPart {
    DecrementArrow,
    Track,
    IncrementArrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpinboxDirection {
    Increment,
    Decrement,
}

/// External effects emitted by pure Graphics-sheet mutations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsSheetAction {
    OpenDisplayModeCombo,
    DisplayModeChanged(GraphicsDisplayMode),
    CheckboxChanged {
        id: GraphicsCheckboxId,
        checked: bool,
    },
    /// The slider/spinbox proposal changed; native config is not changed yet.
    ScaleProposalChanged(i32),
    /// Apply the proposal temporarily and open the timed confirmation dialog.
    TestScale {
        old_percent: i32,
        new_percent: i32,
    },
    SmokeLevelChanged(i32),
}

/// Directly-bound Graphics values plus the unconfirmed scale proposal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphicsSheetState {
    pub display_mode: GraphicsDisplayMode,
    /// Last accepted `Config.Graphics.Scale` value.
    pub applied_scale_percent: i32,
    /// Slider/spinbox value. This becomes applied only after scale-test Yes.
    pub proposed_scale_percent: i32,
    pub add_new_crew_portraits: bool,
    pub save_default_portraits: bool,
    pub auto_frame_skip: bool,
    pub show_folder_maps: bool,
    pub disable_gamma: bool,
    pub smoke_level: i32,
    pub fire_particles: bool,
}

impl Default for GraphicsSheetState {
    fn default() -> Self {
        // C4ConfigGraphics::CompileFunc defaults (C4Config.cpp:438-505).
        Self {
            display_mode: GraphicsDisplayMode::Fullscreen,
            applied_scale_percent: MIN_GRAPHICS_SCALE_PERCENT,
            proposed_scale_percent: MIN_GRAPHICS_SCALE_PERCENT,
            add_new_crew_portraits: true,
            save_default_portraits: true,
            auto_frame_skip: true,
            show_folder_maps: true,
            disable_gamma: false,
            smoke_level: 200,
            fire_particles: true,
        }
    }
}

impl GraphicsSheetState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        display_mode: GraphicsDisplayMode,
        scale_percent: i32,
        add_new_crew_portraits: bool,
        save_default_portraits: bool,
        auto_frame_skip: bool,
        show_folder_maps: bool,
        disable_gamma: bool,
        smoke_level: i32,
        fire_particles: bool,
    ) -> Self {
        let proposed_scale_percent = clamp_scale(scale_percent);
        Self {
            display_mode,
            applied_scale_percent: scale_percent,
            proposed_scale_percent,
            add_new_crew_portraits,
            save_default_portraits,
            auto_frame_skip,
            show_folder_maps,
            disable_gamma,
            smoke_level: smoke_level.clamp(MIN_SMOKE_LEVEL, MAX_SMOKE_LEVEL),
            fire_particles,
        }
    }

    pub const fn checkbox(&self, id: GraphicsCheckboxId) -> bool {
        match id {
            GraphicsCheckboxId::AddNewCrewPortraits => self.add_new_crew_portraits,
            GraphicsCheckboxId::SaveDefaultPortraits => self.save_default_portraits,
            GraphicsCheckboxId::AutoFrameSkip => self.auto_frame_skip,
            GraphicsCheckboxId::ShowFolderMaps => self.show_folder_maps,
            GraphicsCheckboxId::DisableGamma => self.disable_gamma,
            GraphicsCheckboxId::FireParticles => self.fire_particles,
        }
    }

    pub fn set_checkbox(
        &mut self,
        id: GraphicsCheckboxId,
        checked: bool,
    ) -> Option<GraphicsSheetAction> {
        if self.checkbox(id) == checked {
            return None;
        }
        match id {
            GraphicsCheckboxId::AddNewCrewPortraits => self.add_new_crew_portraits = checked,
            GraphicsCheckboxId::SaveDefaultPortraits => self.save_default_portraits = checked,
            GraphicsCheckboxId::AutoFrameSkip => self.auto_frame_skip = checked,
            GraphicsCheckboxId::ShowFolderMaps => self.show_folder_maps = checked,
            GraphicsCheckboxId::DisableGamma => self.disable_gamma = checked,
            GraphicsCheckboxId::FireParticles => self.fire_particles = checked,
        }
        Some(GraphicsSheetAction::CheckboxChanged { id, checked })
    }

    pub fn toggle_checkbox(&mut self, id: GraphicsCheckboxId) -> GraphicsSheetAction {
        let checked = !self.checkbox(id);
        self.set_checkbox(id, checked)
            .expect("toggling a checkbox always changes it")
    }

    pub fn set_display_mode(
        &mut self,
        display_mode: GraphicsDisplayMode,
    ) -> Option<GraphicsSheetAction> {
        if self.display_mode == display_mode {
            return None;
        }
        self.display_mode = display_mode;
        Some(GraphicsSheetAction::DisplayModeChanged(display_mode))
    }

    /// Native scale-slider callback value (`0..=200`).
    pub const fn scale_slider_value(&self) -> i32 {
        self.proposed_scale_percent - MIN_GRAPHICS_SCALE_PERCENT
    }

    pub fn set_scale_slider_value(&mut self, slider_value: i32) -> Option<GraphicsSheetAction> {
        self.set_proposed_scale_percent(slider_value + MIN_GRAPHICS_SCALE_PERCENT)
    }

    /// Spinbox and slider share this single proposal, so either input updates
    /// the value the other control renders.
    pub fn set_scale_spinbox_value(&mut self, value: i32) -> Option<GraphicsSheetAction> {
        self.set_proposed_scale_percent(value)
    }

    pub fn step_scale_spinbox(&mut self, delta: i32) -> Option<GraphicsSheetAction> {
        self.set_proposed_scale_percent(self.proposed_scale_percent.saturating_add(delta))
    }

    pub fn set_proposed_scale_percent(&mut self, value: i32) -> Option<GraphicsSheetAction> {
        let value = clamp_scale(value);
        if self.proposed_scale_percent == value {
            return None;
        }
        self.proposed_scale_percent = value;
        Some(GraphicsSheetAction::ScaleProposalChanged(value))
    }

    /// Mirrors `SpinBox::OnTextChange`: non-digits are discarded and the
    /// interpreted value is clamped, while the caller may retain the sanitized
    /// pre-finish text for caret/edit rendering.
    pub fn set_scale_from_spinbox_text(
        &mut self,
        text: &str,
    ) -> (String, Option<GraphicsSheetAction>) {
        let sanitized = sanitize_scale_spinbox_text(text);
        let parsed = sanitized.parse::<i32>().unwrap_or(0);
        let action = self.set_scale_spinbox_value(parsed);
        (sanitized, action)
    }

    /// Returns no action when Apply/Enter is used without a changed proposal.
    pub const fn request_scale_test(&self) -> Option<GraphicsSheetAction> {
        if self.proposed_scale_percent == self.applied_scale_percent {
            None
        } else {
            Some(GraphicsSheetAction::TestScale {
                old_percent: self.applied_scale_percent,
                new_percent: self.proposed_scale_percent,
            })
        }
    }

    /// Accepts the currently proposed scale after the timed dialog returns Yes.
    pub fn commit_scale_test(&mut self) -> bool {
        if self.applied_scale_percent == self.proposed_scale_percent {
            return false;
        }
        self.applied_scale_percent = self.proposed_scale_percent;
        true
    }

    /// Restores both controls after No, dismissal, or timeout.
    pub fn revert_scale_test(&mut self) -> bool {
        let applied_proposal = clamp_scale(self.applied_scale_percent);
        if self.proposed_scale_percent == applied_proposal {
            return false;
        }
        self.proposed_scale_percent = applied_proposal;
        true
    }

    pub fn set_smoke_slider_value(&mut self, value: i32) -> Option<GraphicsSheetAction> {
        let value = value.clamp(MIN_SMOKE_LEVEL, MAX_SMOKE_LEVEL);
        if self.smoke_level == value {
            return None;
        }
        self.smoke_level = value;
        Some(GraphicsSheetAction::SmokeLevelChanged(value))
    }
}

const fn clamp_scale(value: i32) -> i32 {
    if value < MIN_GRAPHICS_SCALE_PERCENT {
        MIN_GRAPHICS_SCALE_PERCENT
    } else if value > MAX_GRAPHICS_SCALE_PERCENT {
        MAX_GRAPHICS_SCALE_PERCENT
    } else {
        value
    }
}

/// Filters a spinbox edit to its native non-negative three-digit domain.
pub fn sanitize_scale_spinbox_text(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).take(3).collect()
}

/// Font/control measurements used by the source-faithful grid builder.
///
/// [`Default`] is the standard 1280x720 Endeavour-font geometry. The options
/// dialog can supply measured widths through [`graphics_sheet_layout_with_metrics`]
/// when rendering at another configured font size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicsSheetLayoutMetrics {
    pub outer_indent_x: i32,
    pub outer_indent_y: i32,
    pub inner_indent_x: i32,
    pub inner_indent_y: i32,
    pub group_title_line_height: i32,
    pub display_mode_label_width: i32,
    pub apply_button_width: i32,
    pub percent_label_width: i32,
    pub scale_edit_width: i32,
    pub checkbox_height: i32,
    pub low_label_width: i32,
    pub high_label_width: i32,
    pub label_height: i32,
}

impl Default for GraphicsSheetLayoutMetrics {
    fn default() -> Self {
        Self {
            outer_indent_x: 30,
            outer_indent_y: 3,
            inner_indent_x: 30,
            inner_indent_y: 1,
            group_title_line_height: 25,
            display_mode_label_width: 96,
            apply_button_width: 42,
            percent_label_width: 14,
            scale_edit_width: 50,
            checkbox_height: 22,
            low_label_width: 27,
            high_label_width: 31,
            label_height: 22,
        }
    }
}

impl GraphicsSheetLayoutMetrics {
    /// Responsive fallback when exact font measurements are not available.
    pub fn for_sheet(sheet: IntRect) -> Self {
        let mut metrics = Self::default();
        metrics.outer_indent_x = if sheet.w < 390 {
            20
        } else {
            (sheet.w / 21).max(20)
        };
        metrics.outer_indent_y = (sheet.h / 154).max(1);
        metrics.inner_indent_x = metrics.outer_indent_x;
        metrics.inner_indent_y = (metrics.outer_indent_y / 2).max(1);
        metrics
    }
}

/// Geometry in the coordinate space of the supplied sheet rectangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphicsSheetLayout {
    pub sheet: IntRect,
    pub display_group: IntRect,
    pub options_group: IntRect,
    pub effects_group: IntRect,
    pub display_mode_label: IntRect,
    pub display_mode_combo: IntRect,
    pub scale_label: IntRect,
    pub apply_button: IntRect,
    pub percent_label: IntRect,
    pub scale_edit: IntRect,
    pub scale_spin_increment: IntRect,
    pub scale_spin_decrement: IntRect,
    pub scale_slider: IntRect,
    pub checkboxes: [IntRect; 6],
    pub low_label: IntRect,
    pub high_label: IntRect,
    pub smoke_slider: IntRect,
}

impl GraphicsSheetLayout {
    pub const fn checkbox(&self, id: GraphicsCheckboxId) -> IntRect {
        self.checkboxes[id.index()]
    }

    pub const fn slider(&self, id: GraphicsSliderId) -> IntRect {
        match id {
            GraphicsSliderId::Scale => self.scale_slider,
            GraphicsSliderId::SmokeLevel => self.smoke_slider,
        }
    }
}

/// Builds the sheet with standard-font measurements.
pub fn graphics_sheet_layout(sheet: IntRect) -> GraphicsSheetLayout {
    graphics_sheet_layout_with_metrics(sheet, GraphicsSheetLayoutMetrics::for_sheet(sheet))
}

/// Mirrors `C4StartupOptionsDlg.cpp:794-919` using C++ integer grid math.
pub fn graphics_sheet_layout_with_metrics(
    sheet: IntRect,
    metrics: GraphicsSheetLayoutMetrics,
) -> GraphicsSheetLayout {
    let display_group = grid_cell(
        sheet,
        metrics.outer_indent_x,
        metrics.outer_indent_y,
        0,
        1,
        0,
        3,
        -1,
        -1,
        false,
        1,
        1,
    );
    let options_group = grid_cell(
        sheet,
        metrics.outer_indent_x,
        metrics.outer_indent_y,
        0,
        2,
        1,
        3,
        -1,
        -1,
        false,
        1,
        1,
    );
    let effects_group = grid_cell(
        sheet,
        metrics.outer_indent_x,
        metrics.outer_indent_y,
        1,
        2,
        1,
        3,
        -1,
        -1,
        false,
        1,
        1,
    );

    let display_controls = titled_group_client(display_group, metrics.group_title_line_height);
    let display_mode_row = centered_grid_row_with_margins(
        display_controls,
        metrics.inner_indent_x,
        metrics.inner_indent_y,
        0,
        2,
        26,
    );
    let (display_mode_label, display_mode_combo) =
        split_left(display_mode_row, metrics.display_mode_label_width);

    let scale_row = centered_grid_row_with_margins(
        display_controls,
        metrics.inner_indent_x,
        metrics.inner_indent_y,
        1,
        2,
        25,
    );
    let (scale_label, mut scale_remainder) =
        split_left(scale_row, metrics.display_mode_label_width);
    let (next, apply_button) = split_right(scale_remainder, metrics.apply_button_width);
    scale_remainder = next;
    scale_remainder.w = (scale_remainder.w - 16).max(0);
    let (next, percent_label) = split_right(scale_remainder, metrics.percent_label_width);
    let (next, scale_edit) = split_right(next, metrics.scale_edit_width);
    let scale_slider = centered_rect(next, next.w, 16);
    let spin_width = 13.min(scale_edit.w.max(0));
    let spin_x = scale_edit.x + scale_edit.w - spin_width - 1;
    let spin_height = 8.min((scale_edit.h - 4).max(0));
    let scale_spin_increment = IntRect {
        x: spin_x,
        y: scale_edit.y + 2,
        w: spin_width,
        h: spin_height,
    };
    let scale_spin_decrement = IntRect {
        x: spin_x,
        y: scale_edit.y + scale_edit.h - 2 - spin_height,
        w: spin_width,
        h: spin_height,
    };

    let options_client = titled_group_client(options_group, metrics.group_title_line_height);
    let mut checkboxes = [IntRect::default(); 6];
    for (row, id) in GraphicsCheckboxId::OPTIONS.into_iter().enumerate() {
        checkboxes[id.index()] = centered_grid_row_with_margins(
            options_client,
            metrics.inner_indent_x,
            metrics.inner_indent_y,
            row as i32,
            5,
            metrics.checkbox_height,
        );
    }

    let effects_client = titled_group_client(effects_group, metrics.group_title_line_height);
    let effects_level_row = grid_cell(
        effects_client,
        metrics.inner_indent_x,
        metrics.inner_indent_y,
        0,
        1,
        0,
        2,
        -1,
        -1,
        false,
        1,
        1,
    );
    let (low_label, effects_remainder) = split_left(effects_level_row, metrics.low_label_width);
    let (effects_remainder, high_label) = split_right(effects_remainder, metrics.high_label_width);
    let low_label = centered_rect(low_label, low_label.w, metrics.label_height);
    let high_label = centered_rect(high_label, high_label.w, metrics.label_height);
    let smoke_slider = centered_rect(effects_remainder, effects_remainder.w, 16);
    checkboxes[GraphicsCheckboxId::FireParticles.index()] = centered_grid_row_with_margins(
        effects_client,
        metrics.inner_indent_x,
        metrics.inner_indent_y,
        1,
        2,
        metrics.checkbox_height,
    );

    GraphicsSheetLayout {
        sheet,
        display_group,
        options_group,
        effects_group,
        display_mode_label,
        display_mode_combo,
        scale_label,
        apply_button,
        percent_label,
        scale_edit,
        scale_spin_increment,
        scale_spin_decrement,
        scale_slider,
        checkboxes,
        low_label,
        high_label,
        smoke_slider,
    }
}

/// Pointer targets owned by the Graphics sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsHitTarget {
    DisplayModeCombo,
    ScaleEdit,
    ScaleSpinbox(SpinboxDirection),
    ApplyScale,
    Slider {
        id: GraphicsSliderId,
        part: GraphicsSliderPart,
    },
    Checkbox(GraphicsCheckboxId),
}

pub fn graphics_hit_test(
    layout: &GraphicsSheetLayout,
    point: GuiPoint,
) -> Option<GraphicsHitTarget> {
    if rect_contains(layout.scale_spin_increment, point) {
        return Some(GraphicsHitTarget::ScaleSpinbox(SpinboxDirection::Increment));
    }
    if rect_contains(layout.scale_spin_decrement, point) {
        return Some(GraphicsHitTarget::ScaleSpinbox(SpinboxDirection::Decrement));
    }
    if rect_contains(layout.apply_button, point) {
        return Some(GraphicsHitTarget::ApplyScale);
    }
    if rect_contains(layout.display_mode_combo, point) {
        return Some(GraphicsHitTarget::DisplayModeCombo);
    }
    if rect_contains(layout.scale_edit, point) {
        return Some(GraphicsHitTarget::ScaleEdit);
    }
    for id in GraphicsCheckboxId::ALL {
        let bounds = layout.checkbox(id);
        let square = IntRect {
            w: bounds.h + 1,
            ..bounds
        };
        if rect_contains(square, point) {
            return Some(GraphicsHitTarget::Checkbox(id));
        }
    }
    for id in [GraphicsSliderId::Scale, GraphicsSliderId::SmokeLevel] {
        let slider = layout.slider(id);
        if !rect_contains(slider, point) {
            continue;
        }
        let local_x = point.x.floor() as i32 - slider.x;
        let part = if local_x < 16 {
            GraphicsSliderPart::DecrementArrow
        } else if local_x >= slider.w - 16 {
            GraphicsSliderPart::IncrementArrow
        } else {
            GraphicsSliderPart::Track
        };
        return Some(GraphicsHitTarget::Slider { id, part });
    }
    None
}

fn rect_contains(rect: IntRect, point: GuiPoint) -> bool {
    let (x, y) = (point.x.floor() as i32, point.y.floor() as i32);
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

#[allow(clippy::too_many_arguments)]
fn grid_cell(
    area: IntRect,
    margin_x: i32,
    margin_y: i32,
    section_x: i32,
    section_x_count: i32,
    section_y: i32,
    section_y_count: i32,
    requested_width: i32,
    requested_height: i32,
    center: bool,
    span_x: i32,
    span_y: i32,
) -> IntRect {
    let max_width = (area.w - margin_x) / section_x_count - margin_x;
    let max_height = (area.h - margin_y) / section_y_count - margin_y;
    let cell_width = if requested_width < 0 || center {
        max_width
    } else {
        requested_width.min(max_width)
    };
    let cell_height = if requested_height < 0 || center {
        max_height
    } else {
        requested_height.min(max_height)
    };
    let mut rect = IntRect {
        x: area.x + section_x * (cell_width + margin_x) + margin_x,
        y: area.y + section_y * (cell_height + margin_y) + margin_y,
        w: cell_width * span_x + margin_x * (span_x - 1),
        h: cell_height * span_y + margin_y * (span_y - 1),
    };
    if requested_width >= 0 && center {
        rect.x += (cell_width - requested_width) / 2;
        rect.w = requested_width;
    }
    if requested_height >= 0 && center {
        rect.y += (cell_height - requested_height) / 2;
        rect.h = requested_height;
    }
    rect
}

fn centered_grid_row_with_margins(
    area: IntRect,
    margin_x: i32,
    margin_y: i32,
    row: i32,
    row_count: i32,
    height: i32,
) -> IntRect {
    grid_cell(
        area, margin_x, margin_y, 0, 1, row, row_count, -1, height, true, 1, 1,
    )
}

fn titled_group_client(group: IntRect, title_line_height: i32) -> IntRect {
    IntRect {
        x: group.x + 4,
        y: group.y + 4 + title_line_height,
        w: (group.w - 8).max(0),
        h: (group.h - 8 - title_line_height).max(0),
    }
}

fn split_left(rect: IntRect, width: i32) -> (IntRect, IntRect) {
    let width = width.clamp(0, rect.w);
    let left = IntRect { w: width, ..rect };
    let remainder = IntRect {
        x: rect.x + width,
        w: rect.w - width,
        ..rect
    };
    (left, remainder)
}

fn split_right(rect: IntRect, width: i32) -> (IntRect, IntRect) {
    let width = width.clamp(0, rect.w);
    let right = IntRect {
        x: rect.x + rect.w - width,
        w: width,
        ..rect
    };
    let remainder = IntRect {
        w: rect.w - width,
        ..rect
    };
    (remainder, right)
}

fn centered_rect(area: IntRect, width: i32, height: i32) -> IntRect {
    IntRect {
        x: area.x + (area.w - width) / 2,
        y: area.y + (area.h - height) / 2,
        w: width,
        h: height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(rect: IntRect) -> GuiPoint {
        GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
    }

    #[test]
    fn native_defaults_and_config_spellings_are_retained() {
        let state = GraphicsSheetState::default();
        assert_eq!(state.display_mode, GraphicsDisplayMode::Fullscreen);
        assert_eq!(state.applied_scale_percent, 100);
        assert_eq!(state.proposed_scale_percent, 100);
        assert_eq!(state.smoke_level, 200);
        assert!(state.add_new_crew_portraits);
        assert!(state.save_default_portraits);
        assert!(state.auto_frame_skip);
        assert!(state.show_folder_maps);
        assert!(!state.disable_gamma);
        assert!(state.fire_particles);

        assert_eq!(
            GraphicsDisplayMode::from_config("Fullscreen"),
            Some(GraphicsDisplayMode::Fullscreen)
        );
        assert_eq!(
            GraphicsDisplayMode::from_config("1"),
            Some(GraphicsDisplayMode::Window)
        );
        assert_eq!(GraphicsDisplayMode::Window.config_value(), "Window");
    }

    #[test]
    fn checkbox_mutations_are_typed_and_no_op_when_authoritative_value_matches() {
        let mut state = GraphicsSheetState::default();
        assert_eq!(
            state.set_checkbox(GraphicsCheckboxId::FireParticles, true),
            None
        );
        assert_eq!(
            state.toggle_checkbox(GraphicsCheckboxId::FireParticles),
            GraphicsSheetAction::CheckboxChanged {
                id: GraphicsCheckboxId::FireParticles,
                checked: false,
            }
        );
        assert!(!state.fire_particles);
        for id in GraphicsCheckboxId::ALL {
            assert_eq!(GraphicsCheckboxId::ALL[id.index()], id);
            assert!(!id.config_key().is_empty());
            assert!(!id.label().is_empty());
        }
    }

    #[test]
    fn slider_and_spinbox_share_one_clamped_scale_proposal() {
        let mut state = GraphicsSheetState::default();
        assert_eq!(
            state.set_scale_slider_value(50),
            Some(GraphicsSheetAction::ScaleProposalChanged(150))
        );
        assert_eq!(state.proposed_scale_percent, 150);
        assert_eq!(state.scale_slider_value(), 50);
        // Out-of-domain spinbox input clamps to the ceiling, whatever it is
        // (`SpinBox` clamps to its maximum, C4StartupOptionsDlg.cpp:135).
        assert_eq!(
            state.set_scale_spinbox_value(999),
            Some(GraphicsSheetAction::ScaleProposalChanged(
                MAX_GRAPHICS_SCALE_PERCENT
            ))
        );
        assert_eq!(
            state.scale_slider_value(),
            MAX_GRAPHICS_SCALE_PERCENT - MIN_GRAPHICS_SCALE_PERCENT
        );
        assert_eq!(
            state.step_scale_spinbox(-500),
            Some(GraphicsSheetAction::ScaleProposalChanged(100))
        );

        let (text, action) = state.set_scale_from_spinbox_text("x2a50z9");
        assert_eq!(text, "250");
        assert_eq!(action, Some(GraphicsSheetAction::ScaleProposalChanged(250)));
        assert_eq!(sanitize_scale_spinbox_text("12-34"), "123");
    }

    #[test]
    fn high_dpi_scale_ceiling_extends_past_the_cpp_spinbox_maximum() {
        // Deliberate divergence from C4StartupOptionsDlg.cpp:131-132, which
        // pins `constexpr int maxScale = 300` on both the spinbox
        // (`Base{rtBounds, fFocusEdit, minScale, maxScale}`, :135) and the
        // slider range (`maxScale - minScale + 1`, :858). The first-run scale
        // is seeded from the monitor density, so a 4x panel needs 400% to keep
        // the classic 800x600 logical layout; at the C++ ceiling it would be
        // truncated to a value the panel cannot express.
        assert_eq!(MIN_GRAPHICS_SCALE_PERCENT, 100);
        assert_eq!(MAX_GRAPHICS_SCALE_PERCENT, 400);

        let mut state = GraphicsSheetState::default();
        assert_eq!(
            state.set_scale_spinbox_value(400),
            Some(GraphicsSheetAction::ScaleProposalChanged(400))
        );
        assert_eq!(state.proposed_scale_percent, 400);
        assert_eq!(state.scale_slider_value(), 300);
        assert_eq!(
            state.set_scale_spinbox_value(999),
            None,
            "the raised ceiling still clamps"
        );

        // The slider spans exactly the spinbox domain in both directions, and
        // the widened three-digit edit domain still reaches the new maximum.
        assert_eq!(
            state.set_scale_slider_value(0),
            Some(GraphicsSheetAction::ScaleProposalChanged(100))
        );
        let (text, action) = state.set_scale_from_spinbox_text("400");
        assert_eq!(text, "400");
        assert_eq!(action, Some(GraphicsSheetAction::ScaleProposalChanged(400)));
    }

    #[test]
    fn scale_test_is_no_op_until_changed_then_commits_or_reverts() {
        let mut state = GraphicsSheetState::default();
        assert_eq!(state.request_scale_test(), None);
        state.set_scale_spinbox_value(175);
        assert_eq!(
            state.request_scale_test(),
            Some(GraphicsSheetAction::TestScale {
                old_percent: 100,
                new_percent: 175,
            })
        );
        assert!(state.commit_scale_test());
        assert_eq!(state.applied_scale_percent, 175);
        assert!(!state.commit_scale_test());

        state.set_scale_slider_value(100);
        assert_eq!(state.proposed_scale_percent, 200);
        assert!(state.revert_scale_test());
        assert_eq!(state.proposed_scale_percent, 175);
        assert!(!state.revert_scale_test());
    }

    #[test]
    fn loaded_subunit_scale_is_preserved_while_controls_stay_bounded() {
        let mut state = GraphicsSheetState::new(
            GraphicsDisplayMode::Fullscreen,
            50,
            true,
            true,
            true,
            true,
            false,
            200,
            true,
        );
        assert_eq!(state.applied_scale_percent, 50);
        assert_eq!(state.proposed_scale_percent, 100);
        assert_eq!(state.scale_slider_value(), 0);
        assert_eq!(
            state.request_scale_test(),
            Some(GraphicsSheetAction::TestScale {
                old_percent: 50,
                new_percent: 100,
            })
        );

        state.set_proposed_scale_percent(150);
        assert!(state.revert_scale_test());
        assert_eq!(state.applied_scale_percent, 50);
        assert_eq!(state.proposed_scale_percent, 100);
    }

    #[test]
    fn smoke_slider_clamps_to_native_range() {
        let mut state = GraphicsSheetState::default();
        assert_eq!(
            state.set_smoke_slider_value(999),
            Some(GraphicsSheetAction::SmokeLevelChanged(300))
        );
        assert_eq!(state.smoke_level, 300);
        assert_eq!(state.set_smoke_slider_value(300), None);
        assert_eq!(
            state.set_smoke_slider_value(-1),
            Some(GraphicsSheetAction::SmokeLevelChanged(0))
        );
    }

    #[test]
    fn standard_sheet_layout_matches_cpp_group_grid_and_translates_with_origin() {
        let sheet = IntRect {
            x: 356,
            y: 108,
            w: 644,
            h: 462,
        };
        let layout =
            graphics_sheet_layout_with_metrics(sheet, GraphicsSheetLayoutMetrics::default());
        assert_eq!(
            layout.display_group,
            IntRect {
                x: 386,
                y: 111,
                w: 584,
                h: 150
            }
        );
        assert_eq!(
            layout.options_group,
            IntRect {
                x: 386,
                y: 264,
                w: 277,
                h: 150
            }
        );
        assert_eq!(
            layout.effects_group,
            IntRect {
                x: 693,
                y: 264,
                w: 277,
                h: 150
            }
        );
        assert_eq!(layout.scale_slider.h, 16);
        assert_eq!(layout.smoke_slider.h, 16);
        assert_eq!(
            layout.checkbox(GraphicsCheckboxId::AddNewCrewPortraits).y,
            294
        );
        assert_eq!(layout.checkbox(GraphicsCheckboxId::DisableGamma).y, 386);
        assert_eq!(layout.checkbox(GraphicsCheckboxId::FireParticles).y, 369);

        let moved = graphics_sheet_layout_with_metrics(
            IntRect {
                x: 10,
                y: 20,
                ..sheet
            },
            GraphicsSheetLayoutMetrics::default(),
        );
        assert_eq!(moved.display_group.x - 10, layout.display_group.x - sheet.x);
        assert_eq!(moved.display_group.y - 20, layout.display_group.y - sheet.y);
    }

    #[test]
    fn hit_test_distinguishes_spin_arrows_slider_parts_and_checkbox_square() {
        let layout = graphics_sheet_layout_with_metrics(
            IntRect {
                x: 356,
                y: 108,
                w: 644,
                h: 462,
            },
            GraphicsSheetLayoutMetrics::default(),
        );
        assert_eq!(
            graphics_hit_test(&layout, center(layout.display_mode_combo)),
            Some(GraphicsHitTarget::DisplayModeCombo)
        );
        assert_eq!(
            graphics_hit_test(&layout, center(layout.apply_button)),
            Some(GraphicsHitTarget::ApplyScale)
        );
        assert_eq!(
            graphics_hit_test(&layout, center(layout.scale_spin_increment)),
            Some(GraphicsHitTarget::ScaleSpinbox(SpinboxDirection::Increment))
        );
        assert_eq!(
            graphics_hit_test(&layout, center(layout.scale_spin_decrement)),
            Some(GraphicsHitTarget::ScaleSpinbox(SpinboxDirection::Decrement))
        );

        let slider = layout.scale_slider;
        for (x, part) in [
            (slider.x, GraphicsSliderPart::DecrementArrow),
            (slider.x + 16, GraphicsSliderPart::Track),
            (slider.x + slider.w - 1, GraphicsSliderPart::IncrementArrow),
        ] {
            assert_eq!(
                graphics_hit_test(&layout, GuiPoint::new(x as f32, (slider.y + 1) as f32)),
                Some(GraphicsHitTarget::Slider {
                    id: GraphicsSliderId::Scale,
                    part
                })
            );
        }

        let checkbox = layout.checkbox(GraphicsCheckboxId::ShowFolderMaps);
        assert_eq!(
            graphics_hit_test(
                &layout,
                center(IntRect {
                    w: checkbox.h,
                    ..checkbox
                })
            ),
            Some(GraphicsHitTarget::Checkbox(
                GraphicsCheckboxId::ShowFolderMaps
            ))
        );
        assert_eq!(
            graphics_hit_test(
                &layout,
                GuiPoint::new((checkbox.x + checkbox.h + 1) as f32, checkbox.y as f32)
            ),
            None,
            "the caption is not part of native CheckBox::MouseInput"
        );
    }
}
