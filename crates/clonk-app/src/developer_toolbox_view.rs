//! Drawing and hit-testing the two developer toolbox pages.
//!
//! `C4ToolsDlg` and `C4PropertyDlg` build native widget trees — a Win32 dialog
//! template under `_WIN32`, a GTK box tree under `WITH_DEVELOPER_MODE` — and
//! **neither is compiled on the reference build**: past `C4ToolsDlg::Open`'s
//! `#endif` (`C4ToolsDlg.cpp:397`) the function falls straight through to
//! `Active = true` and an ordered refresh, so the "dialog" there is a headless
//! flag. There is consequently no oracle for a pixel of this file.
//!
//! What *is* ported, and what this module is built on, is everything behind
//! the widgets: the page's control inventory and grouping
//! ([`crate::developer_tools_page`], from `C4ToolsDlg.cpp:289-377`), its
//! enablement (`:912-940`), the material and texture catalogues
//! ([`clonk_engine::developer_landscape`], `:482-548`) and the property
//! panel's text ([`clonk_engine::developer_property_text`],
//! `C4PropertyDlg.cpp:169-256`). The layout below only decides where those
//! answers are drawn.

use clonk_engine::developer_landscape::ToolTextureEntry;
use clonk_engine::developer_tools::{LandscapeMode, Tool, GRADE_MAX, GRADE_MIN};
use clonk_frontend::classic_gui::IntRect;
use clonk_frontend::developer_chrome::{
    contains, draw_fitted_text, draw_raised, draw_sunken, fill, CONTROL_BACKGROUND, CONTROL_TEXT,
    DISABLED_TEXT, MID_EDGE, SELECTED_BACKGROUND, SELECTED_TEXT, SMALL_FONT_SIZE,
    WINDOW_BACKGROUND,
};
use clonk_frontend::GuiPoint;
use clonk_graphics::{Surface, TextFont};

use crate::developer_tools_page::{
    landscape_mode_button_enabled, tools_control_enabled, ToolsControl, TOOLS_PAGE_CONTROLS,
};

/// The toolbox window's initial extent. Nothing to port — the GTK page is
/// sized by its box tree and the Win32 one by a dialog template neither of
/// which this build has — so it is the smallest extent that fits the box tree
/// below without clipping a list to fewer than a few rows.
pub(crate) const TOOLBOX_WIDTH: u32 = 320;
pub(crate) const TOOLBOX_HEIGHT: u32 = 260;

const PADDING: i32 = 8;
/// `hbox(12)` / `vbox(12)` in the box tree.
const WIDE_GAP: i32 = 12;
/// `vbox(6)` / `hbox(6)`.
const NARROW_GAP: i32 = 6;
const BUTTON_HEIGHT: i32 = 22;
const MODE_COLUMN_WIDTH: i32 = 72;
const PREVIEW_SIZE: i32 = 48;
const GRADE_WIDTH: i32 = 20;
const IFT_COLUMN_WIDTH: i32 = 52;
const LIST_ROW_HEIGHT: i32 = 14;
/// Wide enough for `IDS_BTN_RELOADDEF` at the page's small face.
const RELOAD_BUTTON_WIDTH: i32 = 88;

/// Everything the Tools page draws, resolved by the caller so the view never
/// reaches into the engine.
/// Which of the two combo selectors is open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolsCombo {
    Materials,
    Textures,
}

/// `Eq` is deliberately absent: the model now carries a rendered sample, and
/// `ImageData` compares structurally without claiming total equality.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolsPageModel {
    /// `Game.Landscape.Mode`, which decides the whole page's enablement.
    pub(crate) mode: LandscapeMode,
    /// `Game.Landscape.Map != nullptr` — the Static button's own gate
    /// (`C4ToolsDlg.cpp:805-807`).
    pub(crate) has_map: bool,
    pub(crate) tool: Tool,
    pub(crate) grade: i32,
    pub(crate) ift: bool,
    pub(crate) material: String,
    pub(crate) texture: String,
    /// `C4ToolsDlg::InitMaterialCtrls`' combo contents (`:486-489`).
    pub(crate) materials: Vec<String>,
    /// `C4ToolsDlg::UpdateTextures`' combo contents (`:517-548`).
    pub(crate) textures: Vec<ToolTextureEntry>,
    /// Which selector is showing its list, if either.
    ///
    /// The open list is an **overlay**, not a control: it is absent from
    /// [`Self::layout`] so the page's controls stay non-overlapping, and it is
    /// drawn and hit-tested after them so it can cover them the way a real
    /// combo does.
    pub(crate) open_combo: Option<ToolsCombo>,
    /// The rendered material sample `UpdatePreview` draws, already resolved
    /// against the material and texture catalogues.
    ///
    /// Resolved by the caller rather than here because the view has no
    /// material data of its own, and `None` when the pair does not resolve —
    /// which is the disabled-page case where C++ shows the sunken box alone.
    pub(crate) preview: Option<clonk_frontend::ImageData>,
}

/// One control's placement and whether it is live.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToolsPageSlot {
    pub(crate) control: ToolsControl,
    pub(crate) rect: IntRect,
    pub(crate) enabled: bool,
}

/// What clicking the page asked the console to do.
///
/// Every variant is a setter because every control on the page is one — the
/// names mirror `C4ToolsDlg`'s own `SetLandscapeMode`/`SetTool`/`SetIFT`/
/// `SetGrade`/`SetMaterial`/`SetTexture`.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ToolsPageAction {
    /// The only one that is a *control*: it goes through the queue as
    /// `EMDT_SetMode` (`C4ToolsDlg.cpp:875-879`).
    SetLandscapeMode(LandscapeMode),
    SetTool(Tool),
    SetIft(bool),
    SetGrade(i32),
    SetMaterial(String),
    SetTexture(String),
    /// Show a selector's list. C++'s combo does this itself; here the open
    /// state lives with the console so the model stays a fresh projection.
    OpenCombo(ToolsCombo),
    /// Dismiss it without selecting, which is what a click off the list does.
    CloseCombo,
}

impl ToolsPageModel {
    /// The page's controls in box-tree order, placed inside `width`×`height`.
    ///
    /// The tree is `C4ToolsDlg.cpp:305-372`: a mode column on the left, and a
    /// right column holding the tool row above a row of preview, grade, IFT
    /// and the two lists.
    pub(crate) fn layout(&self, width: u32, height: u32) -> Vec<ToolsPageSlot> {
        let width = width as i32;
        let height = height as i32;
        let content = IntRect::new(
            PADDING,
            PADDING,
            (width - PADDING * 2).max(1),
            (height - PADDING * 2).max(1),
        );
        let mode_width = MODE_COLUMN_WIDTH.min((content.w - WIDE_GAP - 1).max(1));
        let mut slots = Vec::with_capacity(TOOLS_PAGE_CONTROLS.len());
        let mut push = |control: ToolsControl, rect: IntRect| {
            // Two rules, not one: `EnableControls` covers the page, and
            // `UpdateLandscapeModeCtrls` separately gates the three mode
            // buttons — which `EnableControls` never disables.
            let enabled = tools_control_enabled(control, self.mode, &self.material)
                && control.landscape_mode().is_none_or(|button| {
                    landscape_mode_button_enabled(button, self.mode, self.has_map)
                });
            slots.push(ToolsPageSlot {
                control,
                rect,
                enabled,
            });
        };

        // vbox(6) — the three landscape mode buttons, never disabled.
        for (index, control) in [
            ToolsControl::ModeDynamic,
            ToolsControl::ModeStatic,
            ToolsControl::ModeExact,
        ]
        .into_iter()
        .enumerate()
        {
            push(
                control,
                IntRect::new(
                    content.x,
                    content.y + index as i32 * (BUTTON_HEIGHT + NARROW_GAP),
                    mode_width,
                    BUTTON_HEIGHT,
                ),
            );
        }

        // vbox(12, expands) — the right column.
        let right_x = content.x + mode_width + WIDE_GAP;
        let right_width = (content.x + content.w - right_x).max(1);
        // hbox(6) — the five tool buttons share the row evenly.
        let tools = [
            ToolsControl::Brush,
            ToolsControl::Line,
            ToolsControl::Rect,
            ToolsControl::Fill,
            ToolsControl::Picker,
        ];
        let tool_width = ((right_width - NARROW_GAP * 4) / tools.len() as i32).max(1);
        for (index, control) in tools.into_iter().enumerate() {
            push(
                control,
                IntRect::new(
                    right_x + index as i32 * (tool_width + NARROW_GAP),
                    content.y,
                    tool_width,
                    BUTTON_HEIGHT,
                ),
            );
        }

        // hbox(12, expands) — preview, grade, IFT, then the two lists.
        let lower_y = content.y + BUTTON_HEIGHT + WIDE_GAP;
        let lower_height = (content.y + content.h - lower_y).max(1);
        push(
            ToolsControl::Preview,
            IntRect::new(
                right_x,
                lower_y,
                PREVIEW_SIZE.min(right_width),
                PREVIEW_SIZE.min(lower_height),
            ),
        );
        let grade_x = right_x + PREVIEW_SIZE + WIDE_GAP;
        push(
            ToolsControl::Grade,
            IntRect::new(grade_x, lower_y, GRADE_WIDTH, lower_height),
        );
        let ift_x = grade_x + GRADE_WIDTH + WIDE_GAP;
        for (index, control) in [ToolsControl::Ift, ToolsControl::NoIft]
            .into_iter()
            .enumerate()
        {
            push(
                control,
                IntRect::new(
                    ift_x,
                    lower_y + index as i32 * (BUTTON_HEIGHT + NARROW_GAP),
                    IFT_COLUMN_WIDTH,
                    BUTTON_HEIGHT,
                ),
            );
        }

        // vbox(6, expands) — the material list over the texture list.
        let list_x = ift_x + IFT_COLUMN_WIDTH + NARROW_GAP;
        let list_width = (content.x + content.w - list_x).max(1);
        let list_height = ((lower_height - NARROW_GAP) / 2).max(1);
        push(
            ToolsControl::Materials,
            IntRect::new(list_x, lower_y, list_width, list_height),
        );
        push(
            ToolsControl::Textures,
            IntRect::new(
                list_x,
                lower_y + list_height + NARROW_GAP,
                list_width,
                list_height,
            ),
        );
        slots
    }

    /// What a click at `point` asked for, or `None` for dead space and every
    /// disabled control.
    pub(crate) fn hit(
        &self,
        width: u32,
        height: u32,
        point: (i32, i32),
    ) -> Option<ToolsPageAction> {
        let position = GuiPoint::new(point.0 as f32, point.1 as f32);
        // An open list covers the page, so it answers first. Anywhere off it
        // dismisses rather than acting on whatever it was covering — clicking
        // "away" from a combo should not also press the control underneath.
        if let (Some(combo), Some(popup)) = (self.open_combo, self.combo_popup(width, height)) {
            if !contains(popup, position) {
                return Some(ToolsPageAction::CloseCombo);
            }
            return match combo {
                ToolsCombo::Materials => self
                    .combo_row_at(popup, point.1, self.materials.len())
                    .map(|index| ToolsPageAction::SetMaterial(self.materials[index].clone())),
                ToolsCombo::Textures => self
                    .combo_row_at(popup, point.1, self.textures.len())
                    .map(|index| ToolsPageAction::SetTexture(self.textures[index].name.clone())),
            };
        }
        let slot = self
            .layout(width, height)
            .into_iter()
            .find(|slot| slot.enabled && contains(slot.rect, position))?;
        if let Some(mode) = slot.control.landscape_mode() {
            return Some(ToolsPageAction::SetLandscapeMode(mode));
        }
        if let Some(tool) = slot.control.tool() {
            return Some(ToolsPageAction::SetTool(tool));
        }
        match slot.control {
            ToolsControl::Ift => Some(ToolsPageAction::SetIft(true)),
            ToolsControl::NoIft => Some(ToolsPageAction::SetIft(false)),
            ToolsControl::Grade => Some(ToolsPageAction::SetGrade(grade_at(slot.rect, point.1))),
            // Closed, these are combo boxes: clicking one opens its list
            // rather than selecting whatever row happened to be under the
            // pointer.
            ToolsControl::Materials => Some(ToolsPageAction::OpenCombo(ToolsCombo::Materials)),
            ToolsControl::Textures => Some(ToolsPageAction::OpenCombo(ToolsCombo::Textures)),
            // The preview is a picture, not a control.
            _ => None,
        }
    }

    /// Where an open combo's list is drawn, if one is open.
    ///
    /// Anchored under its own control and clamped to the page, so a long
    /// catalogue scrolls rather than spilling off the window.
    pub(crate) fn combo_popup(&self, width: u32, height: u32) -> Option<IntRect> {
        let combo = self.open_combo?;
        let control = match combo {
            ToolsCombo::Materials => ToolsControl::Materials,
            ToolsCombo::Textures => ToolsControl::Textures,
        };
        let anchor = self
            .layout(width, height)
            .into_iter()
            .find(|slot| slot.control == control)?;
        let rows = match combo {
            ToolsCombo::Materials => self.materials.len(),
            ToolsCombo::Textures => self.textures.len(),
        };
        let wanted = (rows as i32).max(1) * LIST_ROW_HEIGHT + 2;
        let top = anchor.rect.y;
        let available = (height as i32 - top).max(LIST_ROW_HEIGHT + 2);
        Some(IntRect::new(
            anchor.rect.x,
            top,
            anchor.rect.w,
            wanted.min(available),
        ))
    }

    /// The row an open combo's list puts under a point, if any.
    fn combo_row_at(&self, popup: IntRect, y: i32, rows: usize) -> Option<usize> {
        if rows == 0 {
            return None;
        }
        let offset = y - (popup.y + 1);
        if offset < 0 {
            return None;
        }
        let index = (offset / LIST_ROW_HEIGHT) as usize;
        (index < rows).then_some(index)
    }

    fn material_names(&self) -> Vec<&str> {
        self.materials.iter().map(String::as_str).collect()
    }

    /// Whether `control` currently reads as pressed. C++ uses radio buttons
    /// for all three groups, so exactly one of each is down at any time.
    fn pressed(&self, control: ToolsControl) -> bool {
        match control {
            ToolsControl::Ift => self.ift,
            ToolsControl::NoIft => !self.ift,
            control => {
                control.landscape_mode() == Some(self.mode) || control.tool() == Some(self.tool)
            }
        }
    }

    fn label(&self, control: ToolsControl) -> &'static str {
        match control {
            ToolsControl::ModeDynamic => "Dynamic",
            ToolsControl::ModeStatic => "Static",
            ToolsControl::ModeExact => "Exact",
            ToolsControl::Brush => "Brush",
            ToolsControl::Line => "Line",
            ToolsControl::Rect => "Rect",
            ToolsControl::Fill => "Fill",
            ToolsControl::Picker => "Pick",
            ToolsControl::Preview => "",
            ToolsControl::Grade => "",
            ToolsControl::Ift => "IFT",
            ToolsControl::NoIft => "No IFT",
            ToolsControl::Materials => "",
            ToolsControl::Textures => "",
        }
    }

    pub(crate) fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        surface.fill(WINDOW_BACKGROUND);
        for slot in self.layout(surface.width(), surface.height()) {
            match slot.control {
                ToolsControl::Preview => self.render_preview(surface, font, slot.rect),
                ToolsControl::Grade => self.render_grade(surface, slot),
                ToolsControl::Materials => {
                    self.render_closed_combo(surface, font, slot, &self.material)
                }
                ToolsControl::Textures => {
                    self.render_closed_combo(surface, font, slot, &self.texture)
                }
                control => self.render_button(surface, font, slot, self.label(control)),
            }
        }
        // Last, and over everything: an open list is an overlay, so it draws
        // after the controls it covers.
        self.render_open_combo(surface, font);
    }

    /// A closed selector: the current value and the drop marker beside it.
    fn render_closed_combo(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        slot: ToolsPageSlot,
        value: &str,
    ) {
        draw_sunken(
            surface,
            slot.rect,
            if slot.enabled {
                CONTROL_BACKGROUND
            } else {
                WINDOW_BACKGROUND
            },
        );
        let marker = LIST_ROW_HEIGHT;
        draw_fitted_text(
            surface,
            font,
            slot.rect.with_width((slot.rect.w - marker).max(1)),
            value,
            if slot.enabled {
                CONTROL_TEXT
            } else {
                DISABLED_TEXT
            },
            SMALL_FONT_SIZE,
            3,
        );
        // A plain wedge rather than a glyph: the page's font has no arrow, and
        // a letter would read as part of the value.
        let center_x = slot.rect.x + slot.rect.w - marker / 2;
        let center_y = slot.rect.y + slot.rect.h / 2;
        for step in 0..(marker / 4).max(1) {
            let half = (marker / 4).max(1) - step;
            for x in (center_x - half)..=(center_x + half) {
                let _ = surface.set_pixel(
                    x.max(0) as u32,
                    (center_y - 1 + step).max(0) as u32,
                    if slot.enabled {
                        CONTROL_TEXT
                    } else {
                        DISABLED_TEXT
                    },
                );
            }
        }
    }

    /// The open list, drawn over the page.
    fn render_open_combo(&self, surface: &mut Surface, font: &dyn TextFont) {
        let (Some(combo), Some(popup)) = (
            self.open_combo,
            self.combo_popup(surface.width(), surface.height()),
        ) else {
            return;
        };
        let (entries, selected) = match combo {
            ToolsCombo::Materials => (
                self.material_names()
                    .into_iter()
                    .map(|name| (name, true))
                    .collect::<Vec<_>>(),
                self.material.as_str(),
            ),
            ToolsCombo::Textures => (
                self.textures
                    .iter()
                    .map(|entry| (entry.name.as_str(), entry.valid))
                    .collect::<Vec<_>>(),
                self.texture.as_str(),
            ),
        };
        self.render_list(
            surface,
            font,
            ToolsPageSlot {
                control: match combo {
                    ToolsCombo::Materials => ToolsControl::Materials,
                    ToolsCombo::Textures => ToolsControl::Textures,
                },
                rect: popup,
                enabled: true,
            },
            &entries,
            selected,
        );
    }

    fn render_button(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        slot: ToolsPageSlot,
        label: &str,
    ) {
        if self.pressed(slot.control) {
            draw_sunken(surface, slot.rect, WINDOW_BACKGROUND);
        } else {
            draw_raised(surface, slot.rect, WINDOW_BACKGROUND);
        }
        draw_fitted_text(
            surface,
            font,
            slot.rect,
            label,
            if slot.enabled {
                CONTROL_TEXT
            } else {
                DISABLED_TEXT
            },
            SMALL_FONT_SIZE,
            4,
        );
    }

    /// `IDC_PREVIEW` is a rendered swatch of the current material, grade and
    /// IFT (`C4ToolsDlg::UpdatePreview`, `:601-708`): a patterned disc whose
    /// radius is the grade. The names are drawn only when the pair does not
    /// resolve to one, which is the disabled-page case.
    fn render_preview(&self, surface: &mut Surface, font: &dyn TextFont, rect: IntRect) {
        draw_sunken(surface, rect, CONTROL_BACKGROUND);
        if let Some(sample) = self.preview.as_ref() {
            blit_preview_sample(surface, rect, sample);
            return;
        }
        draw_fitted_text(
            surface,
            font,
            rect.with_height(rect.h / 2),
            &self.material,
            CONTROL_TEXT,
            SMALL_FONT_SIZE,
            3,
        );
        draw_fitted_text(
            surface,
            font,
            rect.with_vertical(rect.y + rect.h / 2, rect.h / 2),
            &self.texture,
            CONTROL_TEXT,
            SMALL_FONT_SIZE,
            3,
        );
    }

    /// The grade scale. C++ sets its value to `C4TLS_GradeMax - Grade`
    /// (`InitGradeCtrl`, `:719`), so the scale runs *inverted*: the fine end
    /// is at the bottom.
    fn render_grade(&self, surface: &mut Surface, slot: ToolsPageSlot) {
        draw_sunken(surface, slot.rect, CONTROL_BACKGROUND);
        if !slot.enabled {
            return;
        }
        let travel = (slot.rect.h - BUTTON_HEIGHT).max(1);
        let offset =
            (GRADE_MAX - self.grade.clamp(GRADE_MIN, GRADE_MAX)) * travel / (GRADE_MAX - GRADE_MIN);
        fill(
            surface,
            IntRect::new(
                slot.rect.x + 2,
                slot.rect.y + offset,
                (slot.rect.w - 4).max(1),
                BUTTON_HEIGHT,
            ),
            MID_EDGE,
        );
    }

    /// A combo box's drop-down, drawn open: the list is the control here,
    /// because there is no toolkit to pop one up over the page.
    fn render_list(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        slot: ToolsPageSlot,
        entries: &[(&str, bool)],
        selected: &str,
    ) {
        draw_sunken(
            surface,
            slot.rect,
            if slot.enabled {
                CONTROL_BACKGROUND
            } else {
                WINDOW_BACKGROUND
            },
        );
        if !slot.enabled {
            return;
        }
        let rows = visible_rows(slot.rect);
        let names = entries.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        for (row, (name, valid)) in entries
            .iter()
            .skip(list_scroll(&names, selected, rows))
            .take(rows)
            .enumerate()
        {
            let rect = IntRect::new(
                slot.rect.x + 1,
                slot.rect.y + 1 + row as i32 * LIST_ROW_HEIGHT,
                (slot.rect.w - 2).max(1),
                LIST_ROW_HEIGHT,
            );
            let chosen = *name == selected;
            if chosen {
                fill(surface, rect, SELECTED_BACKGROUND);
            }
            draw_fitted_text(
                surface,
                font,
                rect,
                name,
                match (chosen, valid) {
                    (true, _) => SELECTED_TEXT,
                    // The invalid section is the one C++ parks at the bottom
                    // of the combo (`:517-548`); greying it says so.
                    (false, true) => CONTROL_TEXT,
                    (false, false) => DISABLED_TEXT,
                },
                SMALL_FONT_SIZE,
                3,
            );
        }
    }
}

/// How many rows the list box shows at once.
fn visible_rows(rect: IntRect) -> usize {
    (rect.h / LIST_ROW_HEIGHT).max(1) as usize
}

/// The first entry drawn, chosen so the selection is always on screen. A
/// native combo scrolls its drop-down to the selected item for the same
/// reason.
fn list_scroll(names: &[&str], selected: &str, rows: usize) -> usize {
    names
        .iter()
        .position(|name| *name == selected)
        .map_or(0, |index| (index + 1).saturating_sub(rows))
}

/// The row a click landed on, or `None` past the last entry.
///
/// The scroll offset is the one [`list_scroll`] gave the renderer: computing
/// it differently here would resolve a click to a name the user cannot see.
fn list_index_at(rect: IntRect, y: i32, names: &[&str], selected: &str) -> Option<usize> {
    let rows = visible_rows(rect);
    let row = ((y - rect.y - 1).max(0) / LIST_ROW_HEIGHT) as usize;
    let index = row + list_scroll(names, selected, rows);
    (row < rows && index < names.len()).then_some(index)
}

/// The grade a click at `y` selects, inverted like `InitGradeCtrl`'s scale.
fn grade_at(rect: IntRect, y: i32) -> i32 {
    let travel = (rect.h - BUTTON_HEIGHT).max(1);
    let offset = (y - rect.y - BUTTON_HEIGHT / 2).clamp(0, travel);
    GRADE_MAX - offset * (GRADE_MAX - GRADE_MIN) / travel
}

/// A retained first-visible-line, for a pane whose content is replaced wholesale.
///
/// `C4PropertyDlg::Update` reads `EM_GETFIRSTVISIBLELINE`, replaces the text
/// and scrolls back to it (`C4PropertyDlg.cpp:257-262`). It runs on every
/// Tick35 and every selection change, so the position has to survive the
/// replacement or the pane snaps to the top several times a second.
///
/// The line is kept **unclamped**: an object with less to say does not throw
/// away where the user was, so re-selecting a longer one comes back to it.
/// That is the same property the Win32 edit control has, where `EM_LINESCROLL`
/// clamps the scroll without changing what a later, longer text can reach.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineScroll {
    first: usize,
}

impl LineScroll {
    /// The first visible line for the content as it stands now.
    pub(crate) fn window(&self, lines: usize, capacity: usize) -> usize {
        self.first.min(Self::last_top(lines, capacity))
    }

    /// Scroll by whole lines, as a wheel notch or a bar arrow does.
    pub(crate) fn scroll_by(&mut self, delta: i32, lines: usize, capacity: usize) {
        let last = Self::last_top(lines, capacity);
        let current = i64::try_from(self.first.min(last)).unwrap_or(i64::MAX);
        let target = current.saturating_add(i64::from(delta)).max(0);
        self.first = usize::try_from(target).unwrap_or(usize::MAX).min(last);
    }

    /// The highest first line that still fills the view.
    fn last_top(lines: usize, capacity: usize) -> usize {
        lines.saturating_sub(capacity)
    }
}

/// How many lines the output box shows at this page height.
pub(crate) fn property_output_capacity(height: u32) -> usize {
    let layout = property_page_layout(1, height);
    ((layout.output.h - 2) / LIST_ROW_HEIGHT).max(1) as usize
}

/// The slice of output the box shows: its first line and how many.
pub(crate) fn property_output_window(
    lines: usize,
    scroll: LineScroll,
    height: u32,
) -> (usize, usize) {
    let capacity = property_output_capacity(height);
    (scroll.window(lines, capacity), capacity)
}

/// The one control on the property page that a click can reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PropertyPageAction {
    /// `IDC_BUTTONRELOADDEF` (`C4PropertyDlg.cpp:74-76`).
    ReloadDef,
}

/// Where the property page's two controls sit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PropertyPageLayout {
    /// `IDC_EDITOUTPUT`, the read-only text box.
    pub(crate) output: IntRect,
    /// `IDC_BUTTONRELOADDEF`.
    pub(crate) reload: IntRect,
}

/// `C4PropertyDlg` is a native dialog resource, so its geometry does not
/// survive into this port; only the fact that the page carries the output box
/// and the reload button does. The button takes the bottom row and the box
/// keeps the rest, and on a page too short for both the button wins — it is
/// the only thing here a click can reach.
pub(crate) fn property_page_layout(width: u32, height: u32) -> PropertyPageLayout {
    let width = width.max(1) as i32;
    let height = height.max(1) as i32;
    // A page smaller than its own padding still has to place both controls
    // inside it: the toolbox window has no minimum size of its own.
    let origin_x = PADDING.min(width - 1);
    let origin_y = PADDING.min(height - 1);
    let content = IntRect::new(
        origin_x,
        origin_y,
        (width - origin_x - PADDING).clamp(1, width - origin_x),
        (height - origin_y - PADDING).clamp(1, height - origin_y),
    );
    let button_height = BUTTON_HEIGHT.min(content.h);
    let button_width = RELOAD_BUTTON_WIDTH.min(content.w);
    let reload = IntRect::new(
        content.x + content.w - button_width,
        content.y + content.h - button_height,
        button_width,
        button_height,
    );
    let output_height = (content.h - button_height - NARROW_GAP).max(1);
    PropertyPageLayout {
        output: IntRect::new(content.x, content.y, content.w, output_height),
        reload,
    }
}

/// Which control a click lands on, if any.
pub(crate) fn property_page_hit(
    extent: (u32, u32),
    point: (i32, i32),
) -> Option<PropertyPageAction> {
    let layout = property_page_layout(extent.0, extent.1);
    contains(layout.reload, GuiPoint::new(point.0 as f32, point.1 as f32))
        .then_some(PropertyPageAction::ReloadDef)
}

/// The property page: `C4PropertyDlg`'s read-only text box (`IDC_EDITOUTPUT`)
/// and its `IDC_BUTTONRELOADDEF` button.
///
/// The button is enabled on `Console.Editing` alone (`C4PropertyDlg.cpp:117`),
/// with no selection condition of its own.
pub(crate) fn render_property_page(
    surface: &mut Surface,
    font: &dyn TextFont,
    text: &str,
    scroll: LineScroll,
    reload_enabled: bool,
    reload_label: &str,
) {
    surface.fill(WINDOW_BACKGROUND);
    let layout = property_page_layout(surface.width(), surface.height());
    let rect = layout.output;
    draw_sunken(surface, rect, CONTROL_BACKGROUND);
    let (first, rows) = property_output_window(text.lines().count(), scroll, surface.height());
    for (row, line) in text.lines().skip(first).take(rows).enumerate() {
        draw_fitted_text(
            surface,
            font,
            IntRect::new(
                rect.x + 1,
                rect.y + 1 + row as i32 * LIST_ROW_HEIGHT,
                (rect.w - 2).max(1),
                LIST_ROW_HEIGHT,
            ),
            line,
            CONTROL_TEXT,
            SMALL_FONT_SIZE,
            3,
        );
    }
    draw_raised(surface, layout.reload, WINDOW_BACKGROUND);
    draw_fitted_text(
        surface,
        font,
        layout.reload,
        reload_label,
        if reload_enabled {
            CONTROL_TEXT
        } else {
            DISABLED_TEXT
        },
        SMALL_FONT_SIZE,
        4,
    );
}

/// Centres a rendered sample inside the sunken preview box, clipped to it.
///
/// The sample is produced at the box's own size, so this is a copy rather than
/// a scale — scaling a patterned disc would blur exactly the texture detail the
/// swatch exists to show.
fn blit_preview_sample(surface: &mut Surface, rect: IntRect, sample: &clonk_frontend::ImageData) {
    let width = sample.width() as i32;
    let height = sample.height() as i32;
    if width <= 0 || height <= 0 {
        return;
    }
    let origin_x = rect.x + (rect.w - width).max(0) / 2;
    let origin_y = rect.y + (rect.h - height).max(0) / 2;
    let pixels = sample.pixels();
    for y in 0..height.min(rect.h) {
        for x in 0..width.min(rect.w) {
            let index = ((y * width + x) * 4) as usize;
            let Some(channels) = pixels.get(index..index + 4) else {
                return;
            };
            let _ = surface.set_pixel(
                (origin_x + x).max(0) as u32,
                (origin_y + y).max(0) as u32,
                clonk_graphics::Color::new(channels[0], channels[1], channels[2], channels[3]),
            );
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    /// The output box keeps the line it was showing across a rebuild.
    ///
    /// `C4PropertyDlg::Update` reads `EM_GETFIRSTVISIBLELINE`, replaces the
    /// whole text, and scrolls back to it (`C4PropertyDlg.cpp:257-262`) — and
    /// it runs on every Tick35 and every selection change, so without that the
    /// pane would snap to the top three times a second.
    #[test]
    fn the_property_output_keeps_its_first_visible_line_across_a_rebuild() {
        let capacity = 6;
        let mut scroll = LineScroll::default();
        assert_eq!(scroll.window(40, capacity), 0);

        scroll.scroll_by(9, 40, capacity);
        assert_eq!(scroll.window(40, capacity), 9);
        // The text is replaced wholesale; the position is not part of it.
        assert_eq!(scroll.window(40, capacity), 9);

        // A shorter object pins the view to the last full page.
        assert_eq!(scroll.window(12, capacity), 6);
        assert_eq!(scroll.window(4, capacity), 0);
        // Selecting nothing at all is one line, and is not a scroll *write*:
        // the retained line comes back when the output grows again, which is
        // what makes a Tick35 rebuild non-destructive.
        assert_eq!(scroll.window(1, capacity), 0);
        assert_eq!(scroll.window(40, capacity), 9);

        // Scrolling clamps rather than running off either end.
        scroll.scroll_by(-100, 40, capacity);
        assert_eq!(scroll.window(40, capacity), 0);
        scroll.scroll_by(100, 40, capacity);
        assert_eq!(scroll.window(40, capacity), 34);
    }

    /// The output box shows the retained slice, not always the first one.
    #[test]
    fn the_property_output_window_reports_the_retained_slice() {
        let extent = (240u32, 160u32);
        let capacity = property_output_capacity(extent.1);
        assert!(capacity > 1, "the test page has room for several lines");

        let mut scroll = LineScroll::default();
        scroll.scroll_by(3, 100, capacity);
        assert_eq!(property_output_window(100, scroll, extent.1), (3, capacity));
        // Output that no longer reaches the retained line falls back to what
        // it can show.
        assert_eq!(property_output_window(2, scroll, extent.1), (0, capacity));
    }

    /// The page is the read-only output box plus one button, and the box gives
    /// up the row the button needs rather than being drawn under it.
    ///
    /// `C4PropertyDlg` is `IDC_EDITOUTPUT` and `IDC_BUTTONRELOADDEF` in one
    /// dialog (`C4PropertyDlg.cpp:74-76,117`); only where they sit is this
    /// port's own decision.
    #[test]
    fn the_property_page_reserves_its_reload_button_below_the_output_box() {
        let extent = (240, 160);
        let layout = property_page_layout(extent.0, extent.1);

        assert!(
            layout.output.y + layout.output.h <= layout.reload.y,
            "the output box ends before the button starts: {layout:?}"
        );
        assert!(layout.reload.h > 0 && layout.reload.w > 0);
        assert!(
            layout.reload.y + layout.reload.h <= extent.1 as i32,
            "the button stays inside the page"
        );

        let inside = (
            layout.reload.x + layout.reload.w / 2,
            layout.reload.y + layout.reload.h / 2,
        );
        assert_eq!(
            property_page_hit(extent, inside),
            Some(PropertyPageAction::ReloadDef)
        );
        assert_eq!(
            property_page_hit(extent, (layout.output.x + 1, layout.output.y + 1)),
            None,
            "the output box is read-only and claims no click"
        );
    }

    /// A page too short for both keeps the button rather than drawing it off
    /// the edge, because the button is the only thing on the page a click can
    /// reach at all.
    #[test]
    fn a_short_property_page_still_places_its_button_inside() {
        for height in [1_u32, 8, 20, 32] {
            let layout = property_page_layout(120, height);
            assert!(
                layout.reload.y >= 0 && layout.reload.y + layout.reload.h <= height.max(1) as i32,
                "height {height} put the button at {:?}",
                layout.reload
            );
            assert!(layout.output.h >= 1, "height {height} left no output box");
        }
    }

    use clonk_engine::developer_landscape::TOOL_SKY_MATERIAL;
    use clonk_graphics::{BitmapFont, PixelFormat};

    fn model() -> ToolsPageModel {
        ToolsPageModel {
            mode: LandscapeMode::Exact,
            has_map: true,
            tool: Tool::Brush,
            grade: 5,
            ift: true,
            material: "Earth".to_owned(),
            texture: "Rough".to_owned(),
            open_combo: None,
            // The layout and hit-test tests do not need a rendered sample; the
            // swatch itself is pinned in clonk-frontend where it is composed.
            preview: None,
            materials: vec![
                TOOL_SKY_MATERIAL.to_owned(),
                "Earth".to_owned(),
                "Granite".to_owned(),
            ],
            textures: vec![
                ToolTextureEntry {
                    name: "Smooth".to_owned(),
                    valid: true,
                },
                ToolTextureEntry {
                    name: "Rough".to_owned(),
                    valid: true,
                },
            ],
        }
    }

    fn slot(model: &ToolsPageModel, control: ToolsControl) -> ToolsPageSlot {
        model
            .layout(TOOLBOX_WIDTH, TOOLBOX_HEIGHT)
            .into_iter()
            .find(|slot| slot.control == control)
            .expect("every control is laid out")
    }

    fn click(model: &ToolsPageModel, rect: IntRect) -> Option<ToolsPageAction> {
        model.hit(
            TOOLBOX_WIDTH,
            TOOLBOX_HEIGHT,
            (rect.x + rect.w / 2, rect.y + rect.h / 2),
        )
    }

    /// `C4ToolsDlg` builds the material and texture selectors as **combo
    /// boxes** (`:305-372`), so a closed one shows the current selection and
    /// clicking it opens the list; the open list is an overlay, not a control
    /// in the box tree (clonk-org/clonk-rs#398).
    #[test]
    fn a_closed_combo_opens_and_an_open_one_selects_or_closes() {
        let model = model();
        assert_eq!(model.open_combo, None, "the page opens with both closed");
        assert_eq!(
            click(&model, slot(&model, ToolsControl::Materials).rect),
            Some(ToolsPageAction::OpenCombo(ToolsCombo::Materials)),
            "a closed combo opens rather than selecting whatever was under the pointer"
        );

        let mut open = model.clone();
        open.open_combo = Some(ToolsCombo::Materials);
        let popup = open
            .combo_popup(TOOLBOX_WIDTH, TOOLBOX_HEIGHT)
            .expect("an open combo has a popup");

        // The second row of the open list selects the second material.
        let second = (popup.x + popup.w / 2, popup.y + LIST_ROW_HEIGHT + 1);
        assert_eq!(
            open.hit(TOOLBOX_WIDTH, TOOLBOX_HEIGHT, second),
            Some(ToolsPageAction::SetMaterial(open.materials[1].clone())),
            "an open combo's rows select"
        );

        // Anywhere outside the popup dismisses it instead of acting on the
        // control that happens to sit there.
        let outside = (popup.x + popup.w / 2, popup.y - 2);
        assert_eq!(
            open.hit(TOOLBOX_WIDTH, TOOLBOX_HEIGHT, outside),
            Some(ToolsPageAction::CloseCombo),
            "a click off an open combo closes it and does nothing else"
        );
    }

    /// The popup overlays the page, so it must not join the box tree — the
    /// no-overlap invariant below is about controls, and an open list is
    /// allowed to cover them.
    #[test]
    fn an_open_combo_popup_is_not_a_control_in_the_box_tree() {
        let mut open = model();
        open.open_combo = Some(ToolsCombo::Textures);
        assert_eq!(
            open.layout(TOOLBOX_WIDTH, TOOLBOX_HEIGHT).len(),
            TOOLS_PAGE_CONTROLS.len(),
            "opening a combo adds no control"
        );
        assert!(open.combo_popup(TOOLBOX_WIDTH, TOOLBOX_HEIGHT).is_some());
        assert!(
            model().combo_popup(TOOLBOX_WIDTH, TOOLBOX_HEIGHT).is_none(),
            "a closed page has no popup"
        );
    }

    // C4ToolsDlg.cpp:289-377 — every control in the box tree is placed, once,
    // inside the page, and none of them overlaps another.
    #[test]
    fn tools_page_places_every_control_inside_the_page_without_overlap() {
        let model = model();
        let slots = model.layout(TOOLBOX_WIDTH, TOOLBOX_HEIGHT);
        assert_eq!(slots.len(), TOOLS_PAGE_CONTROLS.len());
        assert_eq!(
            slots.iter().map(|slot| slot.control).collect::<Vec<_>>(),
            TOOLS_PAGE_CONTROLS.to_vec(),
            "the page is built in box-tree order"
        );
        for slot in &slots {
            assert!(
                slot.rect.x >= 0
                    && slot.rect.y >= 0
                    && slot.rect.x + slot.rect.w <= TOOLBOX_WIDTH as i32
                    && slot.rect.y + slot.rect.h <= TOOLBOX_HEIGHT as i32,
                "{:?} is inside the page: {:?}",
                slot.control,
                slot.rect
            );
            assert!(slot.rect.w > 0 && slot.rect.h > 0);
        }
        for (index, first) in slots.iter().enumerate() {
            for second in &slots[index + 1..] {
                let disjoint = first.rect.x + first.rect.w <= second.rect.x
                    || second.rect.x + second.rect.w <= first.rect.x
                    || first.rect.y + first.rect.h <= second.rect.y
                    || second.rect.y + second.rect.h <= first.rect.y;
                assert!(
                    disjoint,
                    "{:?} {:?} overlaps {:?} {:?}",
                    first.control, first.rect, second.control, second.rect
                );
            }
        }
    }

    // C4ToolsDlg.cpp:912-940 — the enablement is the ported one, and a
    // disabled control swallows its click rather than acting.
    #[test]
    fn tools_page_click_is_refused_by_every_disabled_control() {
        let mut model = model();
        assert_eq!(
            click(&model, slot(&model, ToolsControl::Fill).rect),
            Some(ToolsPageAction::SetTool(Tool::Fill)),
            "Fill is live in an exact landscape"
        );

        // Dynamic leaves only the three mode buttons live, which is what lets
        // a user get out of a mode where nothing else works.
        model.mode = LandscapeMode::Dynamic;
        assert_eq!(click(&model, slot(&model, ToolsControl::Fill).rect), None);
        assert_eq!(click(&model, slot(&model, ToolsControl::Brush).rect), None);
        assert_eq!(
            click(&model, slot(&model, ToolsControl::ModeExact).rect),
            Some(ToolsPageAction::SetLandscapeMode(LandscapeMode::Exact))
        );

        // Static keeps everything but Fill.
        model.mode = LandscapeMode::Static;
        assert_eq!(click(&model, slot(&model, ToolsControl::Fill).rect), None);
        assert_eq!(
            click(&model, slot(&model, ToolsControl::Line).rect),
            Some(ToolsPageAction::SetTool(Tool::Line))
        );
        // Sky has no textures to choose between, and only that list goes. The
        // material list beside it stays live — a click on its first row still
        // resolves, where one below the last entry selects nothing at all.
        model.material = TOOL_SKY_MATERIAL.to_owned();
        let textures = slot(&model, ToolsControl::Textures).rect;
        assert_eq!(
            model.hit(
                TOOLBOX_WIDTH,
                TOOLBOX_HEIGHT,
                (textures.x + 2, textures.y + 2)
            ),
            None
        );
        // The material selector beside it stays live. It is a combo now, so a
        // click on it opens its list wherever it lands — there are no longer
        // rows on the closed control to miss.
        let materials = slot(&model, ToolsControl::Materials).rect;
        assert_eq!(
            model.hit(
                TOOLBOX_WIDTH,
                TOOLBOX_HEIGHT,
                (materials.x + 2, materials.y + 2)
            ),
            Some(ToolsPageAction::OpenCombo(ToolsCombo::Materials))
        );
        assert_eq!(
            click(&model, materials),
            Some(ToolsPageAction::OpenCombo(ToolsCombo::Materials))
        );
    }

    // C4ToolsDlg.cpp:719 — `TBM_SETPOS` receives `C4TLS_GradeMax - Grade`, so
    // the scale is inverted: the coarse end is at the top.
    #[test]
    fn grade_scale_runs_inverted_and_clamps_to_its_range() {
        let rect = slot(&model(), ToolsControl::Grade).rect;
        assert_eq!(grade_at(rect, rect.y), GRADE_MAX);
        assert_eq!(grade_at(rect, rect.y + rect.h), GRADE_MIN);
        // Off either end is clamped rather than wrapped or overflowing.
        assert_eq!(grade_at(rect, rect.y - 10_000), GRADE_MAX);
        assert_eq!(grade_at(rect, rect.y + 10_000), GRADE_MIN);
        let middle = grade_at(rect, rect.y + rect.h / 2);
        assert!(
            (GRADE_MIN..=GRADE_MAX).contains(&middle),
            "a mid-scale click is in range: {middle}"
        );
    }

    #[test]
    fn tools_page_ift_and_list_clicks_name_what_they_selected() {
        let model = model();
        assert_eq!(
            click(&model, slot(&model, ToolsControl::Ift).rect),
            Some(ToolsPageAction::SetIft(true))
        );
        assert_eq!(
            click(&model, slot(&model, ToolsControl::NoIft).rect),
            Some(ToolsPageAction::SetIft(false))
        );
        // The preview is a picture: clicking it selects nothing.
        assert_eq!(
            click(&model, slot(&model, ToolsControl::Preview).rect),
            None
        );

        // Selection lives in the open list now: the closed control opens, and a
        // row of the list resolves to the name it drew.
        let materials = slot(&model, ToolsControl::Materials).rect;
        assert_eq!(
            model.hit(
                TOOLBOX_WIDTH,
                TOOLBOX_HEIGHT,
                (materials.x + 2, materials.y + 2),
            ),
            Some(ToolsPageAction::OpenCombo(ToolsCombo::Materials))
        );
        let mut model = model.clone();
        model.open_combo = Some(ToolsCombo::Materials);
        let popup = model
            .combo_popup(TOOLBOX_WIDTH, TOOLBOX_HEIGHT)
            .expect("an open combo has a popup");
        let first = model.hit(TOOLBOX_WIDTH, TOOLBOX_HEIGHT, (popup.x + 2, popup.y + 2));
        assert!(
            matches!(first, Some(ToolsPageAction::SetMaterial(ref name)) if model.materials.contains(name)),
            "a material row selects one of the catalogue's own names: {first:?}"
        );
        let names = model.material_names();
        assert_eq!(
            list_index_at(materials, materials.y + 10_000, &names, &model.material),
            None
        );
        assert_eq!(list_index_at(materials, materials.y, &[], ""), None);
        // The row a click resolves to is the row the renderer drew there,
        // which is what the shared scroll offset guarantees.
        let rows = visible_rows(materials);
        let scroll = list_scroll(&names, &model.material, rows);
        assert_eq!(
            list_index_at(materials, materials.y + 2, &names, &model.material),
            Some(scroll)
        );
    }

    #[test]
    fn toolbox_pages_render_without_panicking_at_any_extent() {
        let font = BitmapFont::new();
        let model = model();
        for (width, height) in [
            (TOOLBOX_WIDTH, TOOLBOX_HEIGHT),
            (1, 1),
            (64, 48),
            (900, 700),
        ] {
            let mut surface = Surface::new(width, height, PixelFormat::Rgba8888);
            model.render(&mut surface, &font);
            render_property_page(
                &mut surface,
                &font,
                "Type: Rock (ROCK)\nOwner: Ada",
                LineScroll::default(),
                true,
                "Reload def",
            );
            let _ = property_page_hit((width, height), (5, 5));
            // A hit anywhere is either a real action or nothing; it never
            // indexes past a catalogue.
            for point in [(0, 0), (width as i32 - 1, height as i32 - 1), (5, 5)] {
                let _ = model.hit(width, height, point);
            }
        }
    }
}
