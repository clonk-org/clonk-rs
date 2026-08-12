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

use crate::developer_tools_page::{tools_control_enabled, ToolsControl, TOOLS_PAGE_CONTROLS};

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

/// Everything the Tools page draws, resolved by the caller so the view never
/// reaches into the engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolsPageModel {
    /// `Game.Landscape.Mode`, which decides the whole page's enablement.
    pub(crate) mode: LandscapeMode,
    pub(crate) tool: Tool,
    pub(crate) grade: i32,
    pub(crate) ift: bool,
    pub(crate) material: String,
    pub(crate) texture: String,
    /// `C4ToolsDlg::InitMaterialCtrls`' combo contents (`:486-489`).
    pub(crate) materials: Vec<String>,
    /// `C4ToolsDlg::UpdateTextures`' combo contents (`:517-548`).
    pub(crate) textures: Vec<ToolTextureEntry>,
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
        let content = IntRect {
            x: PADDING,
            y: PADDING,
            w: (width - PADDING * 2).max(1),
            h: (height - PADDING * 2).max(1),
        };
        let mode_width = MODE_COLUMN_WIDTH.min((content.w - WIDE_GAP - 1).max(1));
        let mut slots = Vec::with_capacity(TOOLS_PAGE_CONTROLS.len());
        let mut push = |control: ToolsControl, rect: IntRect| {
            slots.push(ToolsPageSlot {
                control,
                rect,
                enabled: tools_control_enabled(control, self.mode, &self.material),
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
                IntRect {
                    x: content.x,
                    y: content.y + index as i32 * (BUTTON_HEIGHT + NARROW_GAP),
                    w: mode_width,
                    h: BUTTON_HEIGHT,
                },
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
                IntRect {
                    x: right_x + index as i32 * (tool_width + NARROW_GAP),
                    y: content.y,
                    w: tool_width,
                    h: BUTTON_HEIGHT,
                },
            );
        }

        // hbox(12, expands) — preview, grade, IFT, then the two lists.
        let lower_y = content.y + BUTTON_HEIGHT + WIDE_GAP;
        let lower_height = (content.y + content.h - lower_y).max(1);
        push(
            ToolsControl::Preview,
            IntRect {
                x: right_x,
                y: lower_y,
                w: PREVIEW_SIZE.min(right_width),
                h: PREVIEW_SIZE.min(lower_height),
            },
        );
        let grade_x = right_x + PREVIEW_SIZE + WIDE_GAP;
        push(
            ToolsControl::Grade,
            IntRect {
                x: grade_x,
                y: lower_y,
                w: GRADE_WIDTH,
                h: lower_height,
            },
        );
        let ift_x = grade_x + GRADE_WIDTH + WIDE_GAP;
        for (index, control) in [ToolsControl::Ift, ToolsControl::NoIft]
            .into_iter()
            .enumerate()
        {
            push(
                control,
                IntRect {
                    x: ift_x,
                    y: lower_y + index as i32 * (BUTTON_HEIGHT + NARROW_GAP),
                    w: IFT_COLUMN_WIDTH,
                    h: BUTTON_HEIGHT,
                },
            );
        }

        // vbox(6, expands) — the material list over the texture list.
        let list_x = ift_x + IFT_COLUMN_WIDTH + NARROW_GAP;
        let list_width = (content.x + content.w - list_x).max(1);
        let list_height = ((lower_height - NARROW_GAP) / 2).max(1);
        push(
            ToolsControl::Materials,
            IntRect {
                x: list_x,
                y: lower_y,
                w: list_width,
                h: list_height,
            },
        );
        push(
            ToolsControl::Textures,
            IntRect {
                x: list_x,
                y: lower_y + list_height + NARROW_GAP,
                w: list_width,
                h: list_height,
            },
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
            ToolsControl::Materials => {
                let names = self.material_names();
                list_index_at(slot.rect, point.1, &names, &self.material)
                    .map(|index| ToolsPageAction::SetMaterial(self.materials[index].clone()))
            }
            ToolsControl::Textures => {
                let names = self.texture_names();
                list_index_at(slot.rect, point.1, &names, &self.texture)
                    .map(|index| ToolsPageAction::SetTexture(self.textures[index].name.clone()))
            }
            // The preview is a picture, not a control.
            _ => None,
        }
    }

    fn material_names(&self) -> Vec<&str> {
        self.materials.iter().map(String::as_str).collect()
    }

    fn texture_names(&self) -> Vec<&str> {
        self.textures
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
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
                    // Every material in the catalogue is selectable; only the
                    // texture list has an invalid section (`:517-548`).
                    let entries = self
                        .material_names()
                        .into_iter()
                        .map(|name| (name, true))
                        .collect::<Vec<_>>();
                    self.render_list(surface, font, slot, &entries, &self.material);
                }
                ToolsControl::Textures => {
                    let entries = self
                        .textures
                        .iter()
                        .map(|entry| (entry.name.as_str(), entry.valid))
                        .collect::<Vec<_>>();
                    self.render_list(surface, font, slot, &entries, &self.texture);
                }
                control => self.render_button(surface, font, slot, self.label(control)),
            }
        }
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
    /// IFT (`C4ToolsDlg::UpdatePreview`, `:601-708`). That routine draws
    /// through the engine's own material colours; with no texture atlas here
    /// the swatch stands in as the selection read back in words.
    fn render_preview(&self, surface: &mut Surface, font: &dyn TextFont, rect: IntRect) {
        draw_sunken(surface, rect, CONTROL_BACKGROUND);
        draw_fitted_text(
            surface,
            font,
            IntRect {
                h: rect.h / 2,
                ..rect
            },
            &self.material,
            CONTROL_TEXT,
            SMALL_FONT_SIZE,
            3,
        );
        draw_fitted_text(
            surface,
            font,
            IntRect {
                y: rect.y + rect.h / 2,
                h: rect.h / 2,
                ..rect
            },
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
            IntRect {
                x: slot.rect.x + 2,
                y: slot.rect.y + offset,
                w: (slot.rect.w - 4).max(1),
                h: BUTTON_HEIGHT,
            },
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
            let rect = IntRect {
                x: slot.rect.x + 1,
                y: slot.rect.y + 1 + row as i32 * LIST_ROW_HEIGHT,
                w: (slot.rect.w - 2).max(1),
                h: LIST_ROW_HEIGHT,
            };
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

/// The property page: `C4PropertyDlg`'s read-only text box (`IDC_EDITOUTPUT`).
pub(crate) fn render_property_page(surface: &mut Surface, font: &dyn TextFont, text: &str) {
    surface.fill(WINDOW_BACKGROUND);
    let rect = IntRect {
        x: PADDING,
        y: PADDING,
        w: (surface.width() as i32 - PADDING * 2).max(1),
        h: (surface.height() as i32 - PADDING * 2).max(1),
    };
    draw_sunken(surface, rect, CONTROL_BACKGROUND);
    let rows = ((rect.h - 2) / LIST_ROW_HEIGHT).max(0) as usize;
    for (row, line) in text.lines().take(rows).enumerate() {
        draw_fitted_text(
            surface,
            font,
            IntRect {
                x: rect.x + 1,
                y: rect.y + 1 + row as i32 * LIST_ROW_HEIGHT,
                w: (rect.w - 2).max(1),
                h: LIST_ROW_HEIGHT,
            },
            line,
            CONTROL_TEXT,
            SMALL_FONT_SIZE,
            3,
        );
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use clonk_engine::developer_landscape::TOOL_SKY_MATERIAL;
    use clonk_graphics::{BitmapFont, PixelFormat};

    fn model() -> ToolsPageModel {
        ToolsPageModel {
            mode: LandscapeMode::Exact,
            tool: Tool::Brush,
            grade: 5,
            ift: true,
            material: "Earth".to_owned(),
            texture: "Rough".to_owned(),
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
        let materials = slot(&model, ToolsControl::Materials).rect;
        assert!(matches!(
            model.hit(
                TOOLBOX_WIDTH,
                TOOLBOX_HEIGHT,
                (materials.x + 2, materials.y + 2)
            ),
            Some(ToolsPageAction::SetMaterial(_))
        ));
        assert_eq!(click(&model, materials), None, "a click past the last row");
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

        // A list row resolves to the name it drew, and a click past the last
        // row selects nothing rather than the nearest.
        let materials = slot(&model, ToolsControl::Materials).rect;
        let first = model.hit(
            TOOLBOX_WIDTH,
            TOOLBOX_HEIGHT,
            (materials.x + 2, materials.y + 2),
        );
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
            render_property_page(&mut surface, &font, "Type: Rock (ROCK)\nOwner: Ada");
            // A hit anywhere is either a real action or nothing; it never
            // indexes past a catalogue.
            for point in [(0, 0), (width as i32 - 1, height as i32 - 1), (5, 5)] {
                let _ = model.hit(width, height, point);
            }
        }
    }
}
