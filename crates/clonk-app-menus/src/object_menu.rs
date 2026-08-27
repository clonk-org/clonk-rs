use std::{cell::RefCell, collections::HashMap};

use clonk_engine::{
    CommandKind, ContextMenuEntry, ControlCommand, Engine, EngineError, ObjectId, ObjectMenuExtra,
    ObjectMenuSymbol, SimulationSnapshot, OWNER_NONE,
};
use clonk_frontend::{
    default_owner_color,
    hud::{draw_command_image_cell_with_gamma, HudFont},
    CommandImage, CommandOverlayIcon, GuiPoint,
};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{Color, GammaRamp, PixelFormat, Rect, Surface, TextFont};
use clonk_gui::ImageData;

use crate::ingame_menu::{
    draw_3d_frame, draw_caption_bar, draw_command_key, draw_image_region, draw_image_region_aspect,
    draw_ok_cancel, draw_tooltip, tooltip_position, tooltip_wrap_width, IngameMenuGraphics,
};
use clonk_app_core::pictures::definition_menu_picture;

const BACKDROP_COLOR: Color = Color::new(0, 0, 0, 172);
const PANEL_COLOR: Color = Color::new(18, 28, 48, 240);
const PANEL_BORDER: Color = Color::new(210, 224, 255, 220);
const HIGHLIGHT_COLOR: Color = Color::new(58, 92, 164, 220);
const TITLE_COLOR: Color = Color::opaque(240, 244, 255);
const TEXT_COLOR: Color = Color::opaque(214, 220, 235);
const EMPHASIS_TEXT_COLOR: Color = Color::opaque(255, 255, 255);
const MUTED_TEXT_COLOR: Color = Color::opaque(144, 152, 166);

const PANEL_WIDTH_MIN: i32 = 340;
const PANEL_WIDTH_MAX: i32 = 720;
const PANEL_PADDING: i32 = 24;
const TITLE_GAP: i32 = 28;
const ITEM_HEIGHT: i32 = 42;
const ITEM_SPACING: i32 = 4;
const TITLE_FONT_SIZE: f32 = 22.0;
const ITEM_FONT_SIZE: f32 = 18.0;
const DETAIL_FONT_SIZE: f32 = 14.0;
const MODE_HINT: &str = "Press ←/→ to switch menus";
const CLASSIC_ITEM_SIZE: i32 = 35;
const CLASSIC_FRAME_WIDTH: i32 = 2;
const CLASSIC_COMMAND_HEIGHT: i32 = 16;

/// `C4Menu::InitLocation`'s Context row height: `max(C4MN_SymbolSize,
/// FontRegular.GetLineHeight())` (C4Menu.cpp:650-652). ObjectRank rows size
/// their facet from it (C4Script.cpp:1721).
pub fn classic_context_item_height(font_line_height: i32) -> i32 {
    font_line_height.max(CLASSIC_COMMAND_HEIGHT).max(1)
}
use crate::scrollbar::SCROLLBAR_EXTENT as CLASSIC_SCROLLBAR_WIDTH;
const CLASSIC_TITLE_HEIGHT: i32 = 23;
const CLASSIC_INFO_DEFAULT_WIDTH: i32 = 270;
const CLASSIC_PICTURE_SIZE: i32 = 64;
const CLASSIC_DIALOG_LINES: i32 = 5;
const CLASSIC_DIALOG_LINE_MARGIN: i32 = 5;
const CLASSIC_DIALOG_OPTION_MARGIN: i32 = 3;
const CLASSIC_DIALOG_PORTRAIT_INDENT: i32 = 5;
const CLASSIC_BG_ALPHA: u8 = 255 - 0x5f;
const CLASSIC_SELECTION_COLOR: Color = Color::opaque(0xc8, 0, 0);
const CLASSIC_EXTRA_FRAME_COLOR: Color = Color::opaque(0x44, 0, 0);
const CLASSIC_CAPTION_COLOR: Color = Color::opaque(0xff, 0xff, 0xff);
const CLASSIC_CLOSE_ICON: u8 = 34;
const CLASSIC_TOOLTIP_BG_COLOR: Color = Color::opaque(0xf1, 0xea, 0x78);
const CLASSIC_TOOLTIP_TEXT_COLOR: Color = Color::opaque(0x48, 0x32, 0x22);
const CLASSIC_TOOLTIP_FRAME_COLOR: Color = Color::new(0, 0, 0, 255 - 0x7f);
/// `C4MN_Item_NoCount` (`src/C4Menu.h:67`): this sentinel suppresses the
/// count suffix even though it lives in the same field as real counts.
const MENU_ITEM_NO_COUNT: i32 = 12_345_678;

#[derive(Clone, Debug)]
struct ObjectMenuItem {
    label: String,
    definition_id: String,
    instances: Vec<ObjectId>,
    description: Option<String>,
    icon: Option<ImageData>,
}

impl ObjectMenuItem {
    fn new(
        label: impl Into<String>,
        definition_id: impl Into<String>,
        description: Option<String>,
        icon: Option<ImageData>,
        primary: ObjectId,
    ) -> Self {
        Self {
            label: label.into(),
            definition_id: definition_id.into(),
            instances: vec![primary],
            description,
            icon,
        }
    }

    fn push_instance(&mut self, id: ObjectId) {
        if !self.instances.contains(&id) {
            self.instances.push(id);
        }
    }

    fn count(&self) -> usize {
        self.instances.len()
    }

    fn primary_object(&self) -> Option<ObjectId> {
        self.instances.first().copied()
    }
}

trait MenuEntry {
    fn label(&self) -> &str;
    fn description(&self) -> Option<&str>;
    fn count(&self) -> usize;
    fn count_label(&self) -> Option<String> {
        let count = self.count();
        (count > 1).then(|| format!("x{count}"))
    }
    fn selectable(&self) -> bool {
        true
    }
    fn icon(&self) -> Option<&ImageData> {
        None
    }
}

impl MenuEntry for ObjectMenuItem {
    fn label(&self) -> &str {
        &self.label
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn count(&self) -> usize {
        self.count()
    }

    fn icon(&self) -> Option<&ImageData> {
        self.icon.as_ref()
    }
}

impl MenuEntry for clonk_engine::ObjectMenuItem {
    fn label(&self) -> &str {
        &self.caption
    }

    fn description(&self) -> Option<&str> {
        None
    }

    fn count(&self) -> usize {
        1
    }

    fn count_label(&self) -> Option<String> {
        (self.count != MENU_ITEM_NO_COUNT).then(|| format!("{}x", self.count))
    }

    fn selectable(&self) -> bool {
        self.selectable
    }
}

fn engine_script_presentation_text(text: &str) -> String {
    clonk_resources::decode_legacy_script_text(&clonk_script::c4_string_bytes(text))
}

fn engine_script_menu_title(menu: &clonk_engine::ObjectMenuState) -> String {
    let title = (menu.style == 0)
        .then(|| {
            usize::try_from(menu.selection)
                .ok()
                .and_then(|selection| menu.items.get(selection))
                .map(|item| item.caption.as_str())
        })
        .flatten()
        .unwrap_or(&menu.caption);
    if title.is_empty() {
        " ".to_string()
    } else {
        engine_script_presentation_text(title)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineScriptMenuLayout {
    pub bounds: Rect,
    pub title: Rect,
    /// Visible `ScrollWindow` client area. Item rectangles are translated by
    /// [`Self::scroll_y`] and clipped to this rectangle.
    pub client: Rect,
    pub client_x: i32,
    pub client_y: i32,
    pub columns: i32,
    pub lines: i32,
    pub item_width: i32,
    pub item_height: i32,
    /// Persistent logical-pixel `C4GUI::ScrollWindow::iScrollY`.
    pub scroll_y: i32,
    pub max_scroll_y: i32,
    /// The overflow scrollbar's column, present only while the menu overflows
    /// — `C4GUI::ScrollWindow` shows the bar on the same condition that
    /// reserves its width (`C4GuiContainers.cpp:477-480`).
    pub scrollbar: Option<Rect>,
    pub first_index: usize,
    /// Number of item slots which can intersect the client. A partial first
    /// row exposes one additional row at the bottom.
    pub visible: usize,
}

impl EngineScriptMenuLayout {
    fn item_unclipped_rect(self, index: usize) -> Option<Rect> {
        let index = i32::try_from(index).ok()?;
        let column = index % self.columns;
        let row = index / self.columns;
        Some(Rect::new(
            self.client_x + column * self.item_width,
            self.client_y + row * self.item_height - self.scroll_y,
            self.item_width as u32,
            self.item_height as u32,
        ))
    }

    pub fn item_rect(self, index: usize) -> Option<Rect> {
        let rect = self.item_unclipped_rect(index)?;
        rect.intersection(self.client).map(|_| rect)
    }

    pub fn item_visible_rect(self, index: usize) -> Option<Rect> {
        self.item_unclipped_rect(index)?.intersection(self.client)
    }

    pub fn close_button_rect(self) -> Rect {
        // Dialog::SetTitle places Ico_Close in the wooden label's 16x16
        // top-right corner with 4px indents (C4GuiDialogs.cpp:386-421;
        // Element::GetToprightCornerRect, C4Gui.cpp:363-370).
        Rect::new(
            self.title.x + self.title.width as i32 - 20,
            self.title.y + 4,
            16,
            16,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DialogMenuRowLayout {
    index: usize,
    rect: Rect,
    symbol_rect: Option<Rect>,
    text_rect: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DialogMenuLayout {
    bounds: Rect,
    title: Option<Rect>,
    client: Rect,
    portrait: Option<(usize, Rect)>,
    rows: Vec<DialogMenuRowLayout>,
}

fn translate_rect(mut rect: Rect, dx: i32, dy: i32) -> Rect {
    rect.x = rect.x.saturating_add(dx);
    rect.y = rect.y.saturating_add(dy);
    rect
}

fn relocate_dialog_menu_layout(
    mut layout: DialogMenuLayout,
    location: Option<(i32, i32)>,
) -> DialogMenuLayout {
    let Some((x, y)) = location else {
        return layout;
    };
    let dx = x.saturating_sub(layout.bounds.x);
    let dy = y.saturating_sub(layout.bounds.y);
    layout.bounds = translate_rect(layout.bounds, dx, dy);
    layout.title = layout.title.map(|title| translate_rect(title, dx, dy));
    layout.client = translate_rect(layout.client, dx, dy);
    layout.portrait = layout
        .portrait
        .map(|(index, rect)| (index, translate_rect(rect, dx, dy)));
    for row in &mut layout.rows {
        row.rect = translate_rect(row.rect, dx, dy);
        row.symbol_rect = row.symbol_rect.map(|rect| translate_rect(rect, dx, dy));
        row.text_rect = translate_rect(row.text_rect, dx, dy);
    }
    layout
}

fn clamp_dialog_menu_layout_to_free_anchor(
    area: Rect,
    layout: DialogMenuLayout,
    free_location: (i32, i32),
) -> DialogMenuLayout {
    let width = layout.bounds.width as i32;
    let height = layout.bounds.height as i32;
    let x = if width > area.width as i32 - 2 * CLASSIC_ITEM_SIZE {
        area.x + (area.width as i32 - width) / 2
    } else {
        free_location
            .0
            .clamp(area.x, area.x + area.width as i32 - width)
    };
    let y = if height > area.height as i32 - 2 * CLASSIC_ITEM_SIZE {
        area.y + (area.height as i32 - height) / 2
    } else {
        free_location
            .1
            .clamp(area.y, area.y + area.height as i32 - height)
    };
    relocate_dialog_menu_layout(layout, Some((x, y)))
}

#[cfg(test)]
fn dialog_script_menu_layout(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
) -> DialogMenuLayout {
    dialog_script_menu_layout_with_images(area, font, menu, item_icons, &HashMap::new())
}

fn dialog_script_menu_layout_with_images(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
    font_images: &HashMap<String, ImageData>,
) -> DialogMenuLayout {
    let item_has_symbols = item_icons.iter().map(Option::is_some).collect::<Vec<_>>();
    dialog_script_menu_layout_with_symbols(area, font, menu, &item_has_symbols, font_images)
}

fn dialog_script_menu_layout_with_symbols(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_has_symbols: &[bool],
    font_images: &HashMap<String, ImageData>,
) -> DialogMenuLayout {
    let line_height = font.line_height().max(1);
    let menu_caption = engine_script_presentation_text(&menu.caption);
    let mut item_width = (area.width as i32 - 2 * CLASSIC_FRAME_WIDTH)
        .min(
            text_spec_width(font, &menu_caption, font_images)
                .saturating_add(2 * CLASSIC_COMMAND_HEIGHT + CLASSIC_FRAME_WIDTH)
                .max(CLASSIC_INFO_DEFAULT_WIDTH),
        )
        .max(1);
    let has_portrait = menu
        .items
        .first()
        .is_some_and(|item| item.caption.is_empty());
    if has_portrait && item_width > 2 * CLASSIC_PICTURE_SIZE && area.height > area.width {
        item_width = item_width
            .saturating_sub(CLASSIC_PICTURE_SIZE + CLASSIC_DIALOG_PORTRAIT_INDENT)
            .max(40);
    }

    let first_text = usize::from(has_portrait);
    let natural_rows = menu
        .items
        .iter()
        .enumerate()
        .skip(first_text)
        .map(|(index, item)| {
            let has_symbol = item_has_symbols.get(index).copied().unwrap_or(false);
            let mut assumed_height = line_height;
            let mut available_width = item_width;
            let natural_height = loop {
                let symbol_width = if has_symbol {
                    assumed_height.min(available_width / 2)
                } else {
                    0
                };
                available_width = item_width.saturating_sub(symbol_width).max(1);
                let caption = engine_script_presentation_text(&item.caption);
                let text = layout_info_text(font, &caption, available_width, font_images);
                let height = line_height.saturating_mul(text.lines.len() as i32).max(1);
                if symbol_width == 0 || height <= assumed_height {
                    break height;
                }
                assumed_height = height;
            };
            (index, natural_height, has_symbol)
        })
        .collect::<Vec<_>>();
    let equal_symbol_height = menu.equal_item_height.then(|| {
        natural_rows
            .iter()
            .filter_map(|(_, height, has_symbol)| has_symbol.then_some(*height))
            .max()
            .unwrap_or(0)
    });

    let mut relative_rows = Vec::with_capacity(natural_rows.len());
    let mut y = 0;
    let mut previous_selectable = false;
    for (index, natural_height, has_symbol) in natural_rows {
        let selectable = menu.items[index].selectable;
        y += if !relative_rows.is_empty() && previous_selectable && selectable {
            CLASSIC_DIALOG_OPTION_MARGIN
        } else {
            CLASSIC_DIALOG_LINE_MARGIN
        };
        let height = if has_symbol {
            equal_symbol_height
                .unwrap_or(natural_height)
                .max(natural_height)
        } else {
            natural_height
        };
        relative_rows.push((index, y, height, has_symbol));
        y += height;
        previous_selectable = selectable;
    }

    let max_lines = ((area.height as i32 - 100) / line_height).max(1);
    let lines = CLASSIC_DIALOG_LINES.min(max_lines).max(1);
    let baseline_client_height = lines * line_height;
    let rows_bottom = relative_rows
        .last()
        .map(|(_, y, height, _)| y + height + CLASSIC_DIALOG_LINE_MARGIN)
        .unwrap_or(0);
    let client_height = baseline_client_height.max(rows_bottom);
    let portrait_width =
        i32::from(has_portrait) * (CLASSIC_PICTURE_SIZE + CLASSIC_DIALOG_PORTRAIT_INDENT);
    let title_height = if menu.caption.is_empty() {
        0
    } else {
        line_height.max(CLASSIC_TITLE_HEIGHT)
    };
    let extra_height = i32::from(menu.extra != ObjectMenuExtra::None) * CLASSIC_COMMAND_HEIGHT;
    let (margin_top, margin_left, margin_right, margin_bottom) = menu
        .decoration
        .as_ref()
        .map(|decoration| {
            (
                decoration.border_top,
                CLASSIC_FRAME_WIDTH + decoration.border_left,
                CLASSIC_FRAME_WIDTH + decoration.border_right,
                CLASSIC_FRAME_WIDTH + decoration.border_bottom,
            )
        })
        .unwrap_or((
            0,
            CLASSIC_FRAME_WIDTH,
            CLASSIC_FRAME_WIDTH,
            CLASSIC_FRAME_WIDTH,
        ));
    let width = item_width + portrait_width + margin_left + margin_right;
    let height = margin_top + title_height + client_height + extra_height + margin_bottom;
    let x = area.x + (area.width as i32 - width) / 2;
    let mut y = area.y + CLASSIC_ITEM_SIZE;
    if height > area.height as i32 - 2 * CLASSIC_ITEM_SIZE {
        y = area.y + (area.height as i32 - height) / 2;
    }
    let bounds = Rect::new(x, y, width as u32, height as u32);
    let title = (title_height > 0)
        .then(|| Rect::new(x, y + margin_top, width.max(0) as u32, title_height as u32));
    let client_x = x + margin_left;
    let client_y = y + margin_top + title_height;
    let client = Rect::new(
        client_x,
        client_y,
        (item_width + portrait_width) as u32,
        client_height as u32,
    );
    let portrait = has_portrait.then(|| {
        let height = if item_has_symbols.first().copied().unwrap_or(false) {
            CLASSIC_PICTURE_SIZE
        } else {
            0
        };
        (
            0,
            Rect::new(client_x, client_y, portrait_width as u32, height as u32),
        )
    });
    let rows = relative_rows
        .into_iter()
        .map(|(index, relative_y, height, has_symbol)| {
            let row_x = client_x + portrait_width;
            let row_y = client_y + relative_y;
            let rect = Rect::new(row_x, row_y, item_width as u32, height as u32);
            let symbol_width = if has_symbol && height <= item_width {
                height
            } else {
                0
            };
            let symbol_rect = (symbol_width > 0)
                .then(|| Rect::new(row_x, row_y, symbol_width as u32, height as u32));
            let text_rect = Rect::new(
                row_x + symbol_width,
                row_y,
                item_width.saturating_sub(symbol_width) as u32,
                height as u32,
            );
            DialogMenuRowLayout {
                index,
                rect,
                symbol_rect,
                text_rect,
            }
        })
        .collect();
    DialogMenuLayout {
        bounds,
        title,
        client,
        portrait,
        rows,
    }
}

fn dialog_visible_caption(item: &clonk_engine::ObjectMenuItem) -> String {
    let bytes = clonk_script::c4_string_bytes(&item.caption);
    let end = if item.text_display_progress < 0 {
        bytes.len()
    } else {
        usize::try_from(item.text_display_progress)
            .unwrap_or_default()
            .min(bytes.len())
    };
    clonk_resources::decode_legacy_script_text(&bytes[..end])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineScriptMenuPointerTarget {
    Close,
    Title,
    Item(usize),
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineScriptMenuPresentationGeometry {
    pub bounds: Rect,
    pub title: Option<Rect>,
    pub client: Option<Rect>,
    pub scroll_y: i32,
    pub max_scroll_y: i32,
}

fn rect_contains_point(rect: Rect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.width as i32) as f32
        && point.y < (rect.y + rect.height as i32) as f32
}

pub fn engine_script_menu_pointer_target(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
    show_commands: bool,
    show_close_button: bool,
    point: GuiPoint,
) -> Option<EngineScriptMenuPointerTarget> {
    engine_script_menu_pointer_target_with_info(
        area,
        font,
        menu,
        item_icons,
        show_commands,
        show_close_button,
        point,
        &HashMap::new(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn engine_script_menu_pointer_target_with_info(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
    show_commands: bool,
    show_close_button: bool,
    point: GuiPoint,
    font_images: &HashMap<String, ImageData>,
    free_location: Option<(i32, i32)>,
) -> Option<EngineScriptMenuPointerTarget> {
    if !rect_contains_point(area, point) {
        return None;
    }
    if menu.style == 3 {
        let item_has_symbols = item_icons.iter().map(Option::is_some).collect::<Vec<_>>();
        let mut layout = dialog_script_menu_layout_with_symbols(
            area,
            font,
            menu,
            &item_has_symbols,
            font_images,
        );
        if let Some(free_location) = free_location {
            layout = clamp_dialog_menu_layout_to_free_anchor(area, layout, free_location);
        }
        return dialog_script_menu_pointer_target(&layout, show_close_button, point);
    }
    if !matches!(menu.style, 0..=2) {
        return None;
    }
    let layout = engine_script_menu_layout_with_images(
        area,
        font,
        menu,
        show_commands,
        font_images,
        free_location,
    );
    engine_script_menu_pointer_target_for_layout(menu, layout, show_close_button, point)
}

fn dialog_script_menu_pointer_target(
    layout: &DialogMenuLayout,
    show_close_button: bool,
    point: GuiPoint,
) -> Option<EngineScriptMenuPointerTarget> {
    if let Some(title) = layout.title {
        if show_close_button {
            let close = Rect::new(
                layout.bounds.x + layout.bounds.width as i32 - 20,
                title.y + 4,
                16,
                16,
            );
            if rect_contains_point(close, point) {
                return Some(EngineScriptMenuPointerTarget::Close);
            }
        }
        if rect_contains_point(title, point) {
            return Some(EngineScriptMenuPointerTarget::Title);
        }
    }
    if let Some((index, portrait)) = layout.portrait {
        if rect_contains_point(portrait, point) {
            return Some(EngineScriptMenuPointerTarget::Item(index));
        }
    }
    if let Some(index) = layout
        .rows
        .iter()
        .find(|row| rect_contains_point(row.rect, point))
        .map(|row| row.index)
    {
        return Some(EngineScriptMenuPointerTarget::Item(index));
    }
    rect_contains_point(layout.bounds, point).then_some(EngineScriptMenuPointerTarget::Background)
}

fn engine_script_menu_pointer_target_for_layout(
    menu: &clonk_engine::ObjectMenuState,
    layout: EngineScriptMenuLayout,
    show_close_button: bool,
    point: GuiPoint,
) -> Option<EngineScriptMenuPointerTarget> {
    if show_close_button && rect_contains_point(layout.close_button_rect(), point) {
        return Some(EngineScriptMenuPointerTarget::Close);
    }
    if rect_contains_point(layout.title, point) {
        return Some(EngineScriptMenuPointerTarget::Title);
    }
    let item = menu
        .items
        .iter()
        .enumerate()
        .skip(layout.first_index)
        .take(layout.visible)
        .find_map(|(index, _)| {
            rect_contains_point(layout.item_visible_rect(index)?, point)
                .then_some(EngineScriptMenuPointerTarget::Item(index))
        });
    item.or_else(|| {
        rect_contains_point(layout.bounds, point)
            .then_some(EngineScriptMenuPointerTarget::Background)
    })
}

/// Pointer hit-test using an initialized title position and a retained
/// ScrollWindow offset. Unlike the legacy wrapper, this does not adjust the
/// scroll to the current selection; wheel scrolling may hide the selection.
#[allow(clippy::too_many_arguments)]
pub fn engine_script_menu_pointer_target_with_presentation(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
    show_commands: bool,
    show_close_button: bool,
    point: GuiPoint,
    font_images: &HashMap<String, ImageData>,
    location: Option<(i32, i32)>,
    scroll_y: i32,
    // `C4Menu::Lines` as last written by `C4Menu::SetSize` and not yet
    // overwritten by `InitLocation` (C4Menu.cpp:635-640,713-721); `None`
    // recomputes the row count from the item count.
    explicit_lines: Option<i32>,
) -> Option<EngineScriptMenuPointerTarget> {
    if !rect_contains_point(area, point) {
        return None;
    }
    if menu.style == 3 {
        let item_has_symbols = item_icons.iter().map(Option::is_some).collect::<Vec<_>>();
        let layout = relocate_dialog_menu_layout(
            dialog_script_menu_layout_with_symbols(
                area,
                font,
                menu,
                &item_has_symbols,
                font_images,
            ),
            location,
        );
        return dialog_script_menu_pointer_target(&layout, show_close_button, point);
    }
    if !matches!(menu.style, 0..=2) {
        return None;
    }
    let layout = engine_script_menu_layout_with_presentation(
        area,
        font,
        menu,
        show_commands,
        font_images,
        location,
        scroll_y,
        false,
        explicit_lines,
    );
    engine_script_menu_pointer_target_for_layout(menu, layout, show_close_button, point)
}

/// Hit-test a not-yet-initialized free-alignment anchor with the same
/// one-time clamping used by `C4Menu::InitLocation`.
#[allow(clippy::too_many_arguments)]
pub fn engine_script_menu_pointer_target_with_free_anchor(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
    show_commands: bool,
    show_close_button: bool,
    point: GuiPoint,
    font_images: &HashMap<String, ImageData>,
    free_location: (i32, i32),
    scroll_y: i32,
    // `C4Menu::Lines` as last written by `C4Menu::SetSize` and not yet
    // overwritten by `InitLocation` (C4Menu.cpp:635-640,713-721); `None`
    // recomputes the row count from the item count.
    explicit_lines: Option<i32>,
) -> Option<EngineScriptMenuPointerTarget> {
    if !rect_contains_point(area, point) {
        return None;
    }
    if menu.style == 3 {
        let item_has_symbols = item_icons.iter().map(Option::is_some).collect::<Vec<_>>();
        let layout = clamp_dialog_menu_layout_to_free_anchor(
            area,
            dialog_script_menu_layout_with_symbols(
                area,
                font,
                menu,
                &item_has_symbols,
                font_images,
            ),
            free_location,
        );
        return dialog_script_menu_pointer_target(&layout, show_close_button, point);
    }
    if !matches!(menu.style, 0..=2) {
        return None;
    }
    let layout = engine_script_menu_layout_with_free_anchor(
        area,
        font,
        menu,
        show_commands,
        font_images,
        free_location,
        scroll_y,
        false,
        explicit_lines,
    );
    engine_script_menu_pointer_target_for_layout(menu, layout, show_close_button, point)
}

/// Resolve a not-yet-initialized free-alignment anchor for either native
/// menu presentation style. The returned origin can be retained as an exact
/// location for redraws, refills, and title dragging.
#[allow(clippy::too_many_arguments)]
pub fn engine_script_menu_presentation_geometry_with_free_anchor(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
    show_commands: bool,
    font_images: &HashMap<String, ImageData>,
    free_location: (i32, i32),
    scroll_y: i32,
    adjust_selection: bool,
    // `C4Menu::Lines` as last written by `C4Menu::SetSize` and not yet
    // overwritten by `InitLocation` (C4Menu.cpp:635-640,713-721); `None`
    // recomputes the row count from the item count.
    explicit_lines: Option<i32>,
) -> Option<EngineScriptMenuPresentationGeometry> {
    if menu.style == 3 {
        let item_has_symbols = item_icons.iter().map(Option::is_some).collect::<Vec<_>>();
        let layout = clamp_dialog_menu_layout_to_free_anchor(
            area,
            dialog_script_menu_layout_with_symbols(
                area,
                font,
                menu,
                &item_has_symbols,
                font_images,
            ),
            free_location,
        );
        return Some(EngineScriptMenuPresentationGeometry {
            bounds: layout.bounds,
            title: layout.title,
            client: Some(layout.client),
            scroll_y: 0,
            max_scroll_y: 0,
        });
    }
    if !matches!(menu.style, 0..=2) {
        return None;
    }
    let layout = engine_script_menu_layout_with_free_anchor(
        area,
        font,
        menu,
        show_commands,
        font_images,
        free_location,
        scroll_y,
        adjust_selection,
        explicit_lines,
    );
    Some(EngineScriptMenuPresentationGeometry {
        bounds: layout.bounds,
        title: Some(layout.title),
        client: Some(layout.client),
        scroll_y: layout.scroll_y,
        max_scroll_y: layout.max_scroll_y,
    })
}

/// Style-independent initialized rectangles used by app-level wheel and
/// title-drag routing. Dialog-style menus have no ScrollWindow offset, while
/// normal/context/info menus expose the same clamped pixel state as render.
#[allow(clippy::too_many_arguments)]
pub fn engine_script_menu_presentation_geometry(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    item_icons: &[Option<ImageData>],
    show_commands: bool,
    font_images: &HashMap<String, ImageData>,
    location: Option<(i32, i32)>,
    scroll_y: i32,
    // `C4Menu::Lines` as last written by `C4Menu::SetSize` and not yet
    // overwritten by `InitLocation` (C4Menu.cpp:635-640,713-721); `None`
    // recomputes the row count from the item count.
    explicit_lines: Option<i32>,
) -> Option<EngineScriptMenuPresentationGeometry> {
    if menu.style == 3 {
        let item_has_symbols = item_icons.iter().map(Option::is_some).collect::<Vec<_>>();
        let layout = relocate_dialog_menu_layout(
            dialog_script_menu_layout_with_symbols(
                area,
                font,
                menu,
                &item_has_symbols,
                font_images,
            ),
            location,
        );
        return Some(EngineScriptMenuPresentationGeometry {
            bounds: layout.bounds,
            title: layout.title,
            client: Some(layout.client),
            scroll_y: 0,
            max_scroll_y: 0,
        });
    }
    if !matches!(menu.style, 0..=2) {
        return None;
    }
    let layout = engine_script_menu_layout_with_presentation(
        area,
        font,
        menu,
        show_commands,
        font_images,
        location,
        scroll_y,
        false,
        explicit_lines,
    );
    Some(EngineScriptMenuPresentationGeometry {
        bounds: layout.bounds,
        title: Some(layout.title),
        client: Some(layout.client),
        scroll_y: layout.scroll_y,
        max_scroll_y: layout.max_scroll_y,
    })
}

fn precompose_definition_menu_title_icon(icon: &ImageData) -> ImageData {
    // CreateMenu first draws the definition into a square C4SymbolSize facet;
    // WoodenLabel later scales that square facet into the title bar. Keeping
    // both stages matters for non-square definition pictures such as CLNK
    // (C4Script.cpp:1420-1450; C4Def.cpp:813-837;
    // C4GuiLabels.cpp:168-208).
    thread_local! {
        static TITLE_ICONS: RefCell<HashMap<clonk_graphics::GpuTextureId, ImageData>> =
            RefCell::new(HashMap::new());
    }
    TITLE_ICONS.with(|icons| {
        if let Some(icon) = icons.borrow().get(&icon.gpu_texture_id()).cloned() {
            return icon;
        }
        let side = CLASSIC_ITEM_SIZE as u32;
        let mut symbol = Surface::new(side, side, PixelFormat::Rgba8888);
        draw_image_region_aspect(
            &mut symbol,
            icon,
            Rect::new(0, 0, icon.width(), icon.height()),
            Rect::new(0, 0, side, side),
            false,
            None,
        );
        let composed = ImageData::new(side, side, symbol.pixels().to_vec());
        icons
            .borrow_mut()
            .insert(icon.gpu_texture_id(), composed.clone());
        composed
    })
}

fn command_image_for_menu_symbol(
    symbol: ObjectMenuSymbol,
    picture: Option<ImageData>,
    gfx: &IngameMenuGraphics,
) -> CommandImage {
    match symbol {
        ObjectMenuSymbol::Definition => CommandImage::Picture(picture),
        ObjectMenuSymbol::Put => CommandImage::Composite {
            picture,
            icon: CommandOverlayIcon::Hand(0),
        },
        ObjectMenuSymbol::Buy { owner } => CommandImage::BuyMenu {
            owner_color: gfx
                .owner_colors
                .get(&owner)
                .copied()
                .unwrap_or_else(|| default_owner_color(owner)),
        },
        ObjectMenuSymbol::Sell { owner } => CommandImage::SellMenu {
            owner_color: gfx
                .owner_colors
                .get(&owner)
                .copied()
                .unwrap_or_else(|| default_owner_color(owner)),
        },
        ObjectMenuSymbol::Info | ObjectMenuSymbol::InfoTitle => CommandImage::InfoMenu { picture },
        ObjectMenuSymbol::Exit => CommandImage::Exit,
        ObjectMenuSymbol::Construction => CommandImage::Picture(gfx.hud.construction.clone()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComponentFooterCell {
    component_index: usize,
    rect: Rect,
    count_label: String,
}

fn component_footer_cells(
    mut remaining: Rect,
    components: &[clonk_engine::ObjectMenuComponent],
) -> Vec<ComponentFooterCell> {
    // C4IDList::Draw asks GetSectionCount before applying
    // Right|Triple|Half to each TruncateSection. Keep that slightly unusual
    // capacity rule: a 16px-high strip reports width/16 sections, while each
    // actual component cell consumes 16*3/2 = 24 pixels from the right
    // (C4IDList.cpp:207-227; C4Facet.cpp:38-42,182-213).
    let section_count = remaining.width.checked_div(remaining.height).unwrap_or(0) as usize;
    let cell_width = remaining.height.saturating_mul(3) / 2;
    components
        .iter()
        .take(section_count)
        .enumerate()
        .map_while(|(component_index, component)| {
            (cell_width != 0 && cell_width <= remaining.width).then(|| {
                remaining.width -= cell_width;
                ComponentFooterCell {
                    component_index,
                    rect: Rect::new(
                        remaining.x + remaining.width as i32,
                        remaining.y,
                        cell_width,
                        remaining.height,
                    ),
                    count_label: format!("{}x", component.count),
                }
            })
        })
        .collect()
}

fn draw_component_footer(
    surface: &mut Surface,
    font: &HudFont<'_>,
    gfx: &IngameMenuGraphics,
    remaining: Rect,
    components: &[clonk_engine::ObjectMenuComponent],
    selected_component_icons: &[Option<ImageData>],
    gamma: Option<&GammaRamp>,
) {
    for cell in component_footer_cells(remaining, components) {
        let picture = selected_component_icons
            .get(cell.component_index)
            .cloned()
            .flatten();
        draw_command_image_cell_with_gamma(
            surface,
            &gfx.hud,
            cell.rect,
            &CommandImage::Picture(picture),
            gamma,
        );
        font.draw_with_gamma(
            surface,
            cell.rect.x + cell.rect.width as i32 - 1,
            cell.rect.y + cell.rect.height as i32 - 1 - font.line_height(),
            &cell.count_label,
            CLASSIC_CAPTION_COLOR,
            TextAlign::Right,
            gamma,
        );
    }
}

/// Draws `C4Facet::DrawValue2(..., C4FCT_Right)` and returns the icon's
/// left edge. Components+magic menus use that edge to recover the part of
/// the footer that DrawValue2 consumed (C4Menu.cpp:900-912;
/// C4Facet.cpp:265-290).
fn draw_magic_value_footer(
    surface: &mut Surface,
    font: &HudFont<'_>,
    gfx: &IngameMenuGraphics,
    remaining: Rect,
    value: i32,
    available: i32,
    gamma: Option<&GammaRamp>,
) -> i32 {
    let label = format!("{value}/{available}");
    let label_width = font.text_width(&label);
    let magic_width = gfx
        .hud
        .magic
        .as_ref()
        .map_or(0, |magic| magic.width() as i32);
    let icon_x = remaining.x + remaining.width as i32 - label_width - magic_width - 3;
    if let Some(magic) = gfx.hud.magic.as_ref() {
        draw_image_region_aspect(
            surface,
            magic,
            Rect::new(0, 0, magic.width(), magic.height()),
            Rect::new(
                icon_x,
                remaining.y,
                remaining.height.saturating_mul(2),
                remaining.height,
            ),
            false,
            gamma,
        );
    }
    font.draw_with_gamma(
        surface,
        remaining.x + remaining.width as i32 - 1,
        remaining.y,
        &label,
        CLASSIC_CAPTION_COLOR,
        TextAlign::Right,
        gamma,
    );
    icon_x
}

/// Resolve the definition-backed item value and the live source object's
/// current magic exactly where C4Menu::DrawElement does so. This operates on
/// the app's per-frame presentation clone; engine-owned menu state remains
/// untouched.
pub fn resolve_engine_script_menu_footer(
    engine: &mut Engine,
    menu: &mut clonk_engine::ObjectMenuState,
) -> Result<(), EngineError> {
    let needs_value = matches!(
        menu.extra,
        ObjectMenuExtra::Value
            | ObjectMenuExtra::MagicValue
            | ObjectMenuExtra::ComponentsMagic
            | ObjectMenuExtra::LiveMagicValue
            | ObjectMenuExtra::ComponentsLiveMagic
    );
    if needs_value {
        if let Some(item) = usize::try_from(menu.selection)
            .ok()
            .and_then(|selection| menu.items.get_mut(selection))
        {
            if item.value.is_none() {
                item.value = engine.calculated_definition_value(&item.item_id, None, OWNER_NONE)?;
            }
        }
    }
    if matches!(
        menu.extra,
        ObjectMenuExtra::LiveMagicValue | ObjectMenuExtra::ComponentsLiveMagic
    ) {
        menu.extra_data = u64::try_from(menu.extra_data)
            .ok()
            .and_then(|number| engine.object_snapshot(ObjectId::new(number)))
            .map_or(0, |object| object.magic_energy / 1_000);
    }
    Ok(())
}

#[derive(Clone, Debug)]
enum InfoTextToken {
    Character {
        raw: String,
        width: i32,
    },
    Markup {
        raw: String,
        name: String,
        opening: bool,
    },
    Image {
        spec: String,
        width: i32,
    },
    Break,
}

impl InfoTextToken {
    fn width(&self) -> i32 {
        match self {
            Self::Character { width, .. } | Self::Image { width, .. } => *width,
            Self::Markup { .. } | Self::Break => 0,
        }
    }

    fn break_kind(&self) -> Option<bool> {
        match self {
            Self::Character { raw, .. } if raw == "-" => Some(true),
            Self::Character { raw, .. }
                if raw.len() == 1 && raw.as_bytes()[0].is_ascii_whitespace() =>
            {
                Some(false)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct InfoTextLine {
    tokens: Vec<InfoTextToken>,
    width: i32,
}

#[derive(Clone, Debug, Default)]
struct InfoTextLayout {
    lines: Vec<InfoTextLine>,
    width: i32,
}

fn valid_color_markup_parameters(parameters: &str) -> bool {
    !parameters.is_empty()
        && parameters.len() <= 8
        && parameters
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn tokenize_info_text(
    font: &HudFont<'_>,
    text: &str,
    images: &HashMap<String, ImageData>,
) -> Vec<InfoTextToken> {
    let mut tokens = Vec::new();
    let mut markup_stack: Vec<String> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let raw = &rest[..=end];
                let contents = &rest[1..end];
                let markup = if let Some(name) = contents.strip_prefix('/') {
                    (contents.find(' ').is_none()
                        && markup_stack.last().is_some_and(|open| open == name))
                    .then(|| (name.to_string(), false))
                } else if contents == "i" {
                    Some(("i".to_string(), true))
                } else if let Some(parameters) = contents.strip_prefix("c ") {
                    valid_color_markup_parameters(parameters).then(|| ("c".to_string(), true))
                } else {
                    None
                };
                if let Some((name, opening)) = markup {
                    if opening {
                        markup_stack.push(name.clone());
                    } else {
                        markup_stack.pop();
                    }
                    tokens.push(InfoTextToken::Markup {
                        raw: raw.to_string(),
                        name,
                        opening,
                    });
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        if let Some(after_open) = rest.strip_prefix("{{") {
            if let Some(end) = after_open.find("}}") {
                let spec = &after_open[..end];
                if !spec.is_empty() && !spec.starts_with('{') {
                    let width = images.get(spec).map_or(0, |image| {
                        let height = image.height().max(1);
                        i32::try_from(
                            u64::from(image.width())
                                * u64::try_from(font.graphics_line_height().max(0)).unwrap_or(0)
                                / u64::from(height),
                        )
                        .unwrap_or(i32::MAX)
                    });
                    tokens.push(InfoTextToken::Image {
                        spec: spec.to_string(),
                        width,
                    });
                    rest = &after_open[end + 2..];
                    continue;
                }
            }
        }
        let character = rest.chars().next().expect("non-empty text remainder");
        rest = &rest[character.len_utf8()..];
        if character == '\n' || character == '|' {
            tokens.push(InfoTextToken::Break);
        } else {
            tokens.push(InfoTextToken::Character {
                raw: character.to_string(),
                width: font.character_advance(character),
            });
        }
    }
    tokens
}

fn info_line_metrics(tokens: &[InfoTextToken]) -> (i32, Option<(usize, bool)>) {
    let mut width = 0_i32;
    let mut last_break = None;
    for (index, token) in tokens.iter().enumerate() {
        width = width.saturating_add(token.width());
        if let Some(include) = token.break_kind() {
            last_break = Some((index, include));
        }
    }
    (width, last_break)
}

fn layout_info_text(
    font: &HudFont<'_>,
    text: &str,
    max_width: i32,
    images: &HashMap<String, ImageData>,
) -> InfoTextLayout {
    let max_width = max_width.max(1);
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut line_width = 0_i32;
    let mut last_break: Option<(usize, bool)> = None;
    let mut visible_tokens = 0_usize;

    let push_line = |lines: &mut Vec<InfoTextLine>, tokens: Vec<InfoTextToken>| {
        let width = tokens
            .iter()
            .fold(0_i32, |width, token| width.saturating_add(token.width()));
        lines.push(InfoTextLine { tokens, width });
    };

    for token in tokenize_info_text(font, text, images) {
        if matches!(token, InfoTextToken::Break) {
            push_line(&mut lines, std::mem::take(&mut line));
            line_width = 0;
            last_break = None;
            visible_tokens = 0;
            continue;
        }
        let token_width = token.width();
        let token_break = token.break_kind();
        line.push(token);
        if token_width == 0 {
            continue;
        }
        visible_tokens += 1;
        line_width = line_width.saturating_add(token_width);
        if line_width <= max_width || visible_tokens == 1 {
            if let Some(include) = token_break {
                // BreakMessage preserves whitespace when it is the first
                // character on a line; later break spaces are replaced by
                // the inserted newline (StdFont.cpp:684-699,735-746).
                last_break = Some((line.len() - 1, include || visible_tokens == 1));
            }
            continue;
        }

        let current_is_space = token_break == Some(false);
        let (split_at, skip_after_split) = if current_is_space {
            (line.len() - 1, 1)
        } else if let Some((break_index, include)) = last_break {
            if include {
                (break_index + 1, 0)
            } else {
                (break_index, 1)
            }
        } else {
            (line.len() - 1, 0)
        };
        let mut remainder = line.split_off(split_at);
        if skip_after_split > 0 && !remainder.is_empty() {
            remainder.remove(0);
        }
        push_line(&mut lines, std::mem::take(&mut line));
        line = remainder;
        (line_width, last_break) = info_line_metrics(&line);
        visible_tokens = line.iter().filter(|token| token.width() > 0).count();
    }
    push_line(&mut lines, line);
    if lines.is_empty() {
        lines.push(InfoTextLine::default());
    }
    let width = lines.iter().map(|line| line.width).max().unwrap_or(0);
    InfoTextLayout { lines, width }
}

fn text_spec_layout(
    font: &HudFont<'_>,
    text: &str,
    images: &HashMap<String, ImageData>,
) -> InfoTextLayout {
    layout_info_text(font, text, i32::MAX, images)
}

fn text_spec_width(font: &HudFont<'_>, text: &str, images: &HashMap<String, ImageData>) -> i32 {
    if !tokenize_info_text(font, text, images)
        .iter()
        .any(|token| matches!(token, InfoTextToken::Image { .. }))
    {
        return font.text_width_markup(text);
    }
    text_spec_layout(font, text, images).width
}

// Keep the token layout, inline-image table, draw origin, color, and gamma
// explicit at this renderer boundary so markup parity remains easy to audit.
#[allow(clippy::too_many_arguments)]
fn render_info_text(
    surface: &mut Surface,
    font: &HudFont<'_>,
    layout: &InfoTextLayout,
    images: &HashMap<String, ImageData>,
    x: i32,
    y: i32,
    color: Color,
    gamma: Option<&GammaRamp>,
) {
    let mut active_markup: Vec<(String, String)> = Vec::new();
    for (line_index, line) in layout.lines.iter().enumerate() {
        let mut draw_x = x;
        let line_y = y + line_index as i32 * font.line_height();
        let mut text = String::new();
        let mut text_width = 0_i32;
        let flush_text =
            |surface: &mut Surface, text: &mut String, text_width: &mut i32, draw_x: &mut i32| {
                if !text.is_empty() {
                    font.draw_markup_with_gamma(
                        surface,
                        *draw_x,
                        line_y,
                        text,
                        color,
                        TextAlign::Left,
                        gamma,
                    );
                    *draw_x = draw_x.saturating_add(*text_width);
                    text.clear();
                    *text_width = 0;
                }
            };
        for token in &line.tokens {
            match token {
                InfoTextToken::Image { spec, width } => {
                    flush_text(surface, &mut text, &mut text_width, &mut draw_x);
                    if let Some(image) = images.get(spec).filter(|_| *width > 0) {
                        draw_image_region_aspect(
                            surface,
                            image,
                            Rect::new(0, 0, image.width(), image.height()),
                            Rect::new(
                                draw_x,
                                line_y,
                                *width as u32,
                                font.graphics_line_height().max(0) as u32,
                            ),
                            false,
                            gamma,
                        );
                    }
                    draw_x = draw_x.saturating_add(*width);
                }
                InfoTextToken::Character { raw, width } => {
                    if text.is_empty() {
                        for (_, opening) in &active_markup {
                            text.push_str(opening);
                        }
                    }
                    text.push_str(raw);
                    text_width = text_width.saturating_add(*width);
                }
                InfoTextToken::Markup { raw, name, opening } => {
                    if text.is_empty() {
                        for (_, opening) in &active_markup {
                            text.push_str(opening);
                        }
                    }
                    text.push_str(raw);
                    if *opening {
                        active_markup.push((name.clone(), raw.clone()));
                    } else if active_markup.last().is_some_and(|(open, _)| open == name) {
                        active_markup.pop();
                    }
                }
                InfoTextToken::Break => {}
            }
        }
        flush_text(surface, &mut text, &mut text_width, &mut draw_x);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_text_spec(
    surface: &mut Surface,
    font: &HudFont<'_>,
    text: &str,
    images: &HashMap<String, ImageData>,
    x: i32,
    y: i32,
    color: Color,
    gamma: Option<&GammaRamp>,
) {
    if !tokenize_info_text(font, text, images)
        .iter()
        .any(|token| matches!(token, InfoTextToken::Image { .. }))
    {
        font.draw_markup_with_gamma(surface, x, y, text, color, TextAlign::Left, gamma);
        return;
    }
    let layout = text_spec_layout(font, text, images);
    render_info_text(surface, font, &layout, images, x, y, color, gamma);
}

// Tooltip placement and rich-text resources are independent classic renderer
// inputs; a parameter bundle would only obscure their parity mapping.
#[allow(clippy::too_many_arguments)]
fn draw_text_spec_tooltip(
    surface: &mut Surface,
    font: &HudFont<'_>,
    facet: Rect,
    x: i32,
    y: i32,
    text: &str,
    images: &HashMap<String, ImageData>,
    gamma: Option<&GammaRamp>,
) {
    if !tokenize_info_text(font, text, images)
        .iter()
        .any(|token| matches!(token, InfoTextToken::Image { .. }))
    {
        draw_tooltip(surface, font, facet, x, y, text, gamma);
        return;
    }
    let layout = layout_info_text(font, text, tooltip_wrap_width(facet), images);
    let text_width = layout.width;
    let text_height = font.line_height() * layout.lines.len() as i32;
    let width = text_width + 6;
    let height = text_height + 4;
    let (tooltip_x, tooltip_y) = tooltip_position(facet, x, y, width, height);
    fill_rect(
        surface,
        Rect::new(
            tooltip_x,
            tooltip_y,
            width.max(0) as u32,
            height.saturating_sub(1).max(0) as u32,
        ),
        CLASSIC_TOOLTIP_BG_COLOR,
        gamma,
    );
    draw_border(
        surface,
        Rect::new(
            tooltip_x,
            tooltip_y,
            width.max(0) as u32,
            height.max(0) as u32,
        ),
        CLASSIC_TOOLTIP_FRAME_COLOR,
        gamma,
    );
    render_info_text(
        surface,
        font,
        &layout,
        images,
        tooltip_x + 3,
        tooltip_y + 1,
        CLASSIC_TOOLTIP_TEXT_COLOR,
        gamma,
    );
}

fn collect_inline_image_specs(text: &str, specs: &mut Vec<String>) {
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after_open) = rest.strip_prefix("{{") {
            if let Some(end) = after_open.find("}}") {
                let spec = &after_open[..end];
                if !spec.is_empty() && !spec.starts_with('{') {
                    if !specs.iter().any(|old| old == spec) {
                        specs.push(spec.to_string());
                    }
                    rest = &after_open[end + 2..];
                    continue;
                }
            }
        }
        let character = rest.chars().next().expect("non-empty text remainder");
        rest = &rest[character.len_utf8()..];
    }
}

pub fn engine_script_menu_inline_image_specs(menu: &clonk_engine::ObjectMenuState) -> Vec<String> {
    let mut specs = Vec::new();
    collect_inline_image_specs(&engine_script_presentation_text(&menu.caption), &mut specs);
    collect_inline_image_specs(&engine_script_menu_title(menu), &mut specs);
    for item in &menu.items {
        collect_inline_image_specs(&engine_script_presentation_text(&item.caption), &mut specs);
        collect_inline_image_specs(
            &engine_script_presentation_text(&item.info_caption),
            &mut specs,
        );
    }
    specs
}

pub fn engine_script_menu_layout(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    show_commands: bool,
) -> EngineScriptMenuLayout {
    engine_script_menu_layout_with_images(area, font, menu, show_commands, &HashMap::new(), None)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EngineScriptMenuLocation {
    Aligned,
    /// Uninitialized `C4MN_Align_Free` anchor. `InitLocation` clamps it.
    FreeAnchor(i32, i32),
    /// Already initialized/dragged dialog origin. Native dragging applies it
    /// verbatim until a later `ResetLocation`.
    Exact(i32, i32),
}

fn engine_script_menu_layout_with_images(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    show_commands: bool,
    font_images: &HashMap<String, ImageData>,
    free_location: Option<(i32, i32)>,
) -> EngineScriptMenuLayout {
    let location = free_location.map_or(EngineScriptMenuLocation::Aligned, |(x, y)| {
        EngineScriptMenuLocation::FreeAnchor(x, y)
    });
    engine_script_menu_layout_impl(
        area,
        font,
        menu,
        show_commands,
        font_images,
        location,
        0,
        true,
        None,
    )
}

/// Computes the initialized presentation geometry for a live script menu.
///
/// `location` is an already initialized top-left position (including a raw
/// title drag), while `scroll_y` is the retained logical-pixel ScrollWindow
/// offset. Callers set `adjust_selection` only for the native sites which
/// invoke `C4Menu::AdjustPosition`; wheel scrolling and redraws pass `false`.
#[allow(clippy::too_many_arguments)]
pub fn engine_script_menu_layout_with_presentation(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    show_commands: bool,
    font_images: &HashMap<String, ImageData>,
    location: Option<(i32, i32)>,
    scroll_y: i32,
    adjust_selection: bool,
    explicit_lines: Option<i32>,
) -> EngineScriptMenuLayout {
    let location = location.map_or(EngineScriptMenuLocation::Aligned, |(x, y)| {
        EngineScriptMenuLocation::Exact(x, y)
    });
    engine_script_menu_layout_impl(
        area,
        font,
        menu,
        show_commands,
        font_images,
        location,
        scroll_y,
        adjust_selection,
        explicit_lines,
    )
}

/// Resolve a not-yet-initialized `C4MN_Align_Free` anchor once. The returned
/// bounds may then be retained and supplied to the ordinary presentation
/// helper as an exact dragged/initialized location.
#[allow(clippy::too_many_arguments)]
pub fn engine_script_menu_layout_with_free_anchor(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    show_commands: bool,
    font_images: &HashMap<String, ImageData>,
    free_location: (i32, i32),
    scroll_y: i32,
    adjust_selection: bool,
    explicit_lines: Option<i32>,
) -> EngineScriptMenuLayout {
    engine_script_menu_layout_impl(
        area,
        font,
        menu,
        show_commands,
        font_images,
        EngineScriptMenuLocation::FreeAnchor(free_location.0, free_location.1),
        scroll_y,
        adjust_selection,
        explicit_lines,
    )
}

#[allow(clippy::too_many_arguments)]
fn engine_script_menu_layout_impl(
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    show_commands: bool,
    font_images: &HashMap<String, ImageData>,
    location: EngineScriptMenuLocation,
    scroll_y: i32,
    adjust_selection: bool,
    explicit_lines: Option<i32>,
) -> EngineScriptMenuLayout {
    // Normal menus are a fixed 35px icon grid. Context menus are compact
    // captioned rows: height=max(C4MN_SymbolSize, FontRegular), width is
    // the widest title/item text plus its square symbol (C4Menu.cpp:
    // 642-665). InitMenu normally gives Context one column (:359-365).
    let columns = menu.columns.max(1);
    let (item_width, item_height) = match menu.style {
        1 => {
            let item_height = classic_context_item_height(font.line_height());
            let title = engine_script_presentation_text(&menu.caption);
            let title_width = text_spec_width(font, &title, font_images)
                .saturating_add(item_height)
                .saturating_add(CLASSIC_COMMAND_HEIGHT);
            let item_width = menu
                .items
                .iter()
                .map(|item| {
                    let caption = engine_script_presentation_text(&item.caption);
                    text_spec_width(font, &caption, font_images).saturating_add(item_height)
                })
                .fold(title_width, i32::max)
                .saturating_add(3);
            (item_width.max(1), item_height.max(1))
        }
        2 => {
            // C4MN_Style_Info first wraps against a 270px default text
            // column (capped by the viewport), shrinks that column to the
            // widest actual title/line, adds 3px breathing room, and finally
            // appends a 64px picture column (C4Menu.cpp:666-693).
            let title = engine_script_presentation_text(&menu.caption);
            let mut largest_text_width = text_spec_width(font, &title, font_images)
                .saturating_add(2 * CLASSIC_COMMAND_HEIGHT)
                .saturating_add(CLASSIC_FRAME_WIDTH);
            let wrap_width = (area.width as i32 - 2 * CLASSIC_FRAME_WIDTH)
                .min(largest_text_width.max(CLASSIC_INFO_DEFAULT_WIDTH))
                .max(1);
            let mut text_height = 0;
            for item in &menu.items {
                let info_caption = engine_script_presentation_text(&item.info_caption);
                let text = layout_info_text(font, &info_caption, wrap_width, font_images);
                largest_text_width = largest_text_width.max(text.width);
                text_height = text_height.max(font.line_height() * text.lines.len() as i32);
            }
            (
                wrap_width
                    .min(largest_text_width)
                    .saturating_add(3 + CLASSIC_PICTURE_SIZE)
                    .max(1),
                text_height.max(CLASSIC_PICTURE_SIZE),
            )
        }
        _ => (CLASSIC_ITEM_SIZE, CLASSIC_ITEM_SIZE),
    };
    let item_count = i32::try_from(menu.items.len()).unwrap_or(i32::MAX);
    let natural_lines = (item_count / columns) + i32::from(item_count % columns != 0);
    let max_lines = ((area.height as i32 - 100) / item_height).max(1);
    // `InitLocation` derives Lines from the item count and clamps it to the
    // viewport (C4Menu.cpp:713-719). `C4Menu::SetSize` instead assigns Lines
    // outright and reruns only `InitSize`, which applies no viewport clamp
    // (C4Menu.cpp:635-640,755-780), so an explicit row count from
    // `SetMenuSize` is used as given.
    let lines = explicit_lines
        .filter(|lines| *lines > 0)
        .unwrap_or_else(|| natural_lines.max(1).min(max_lines))
        .max(1);
    let title_height = font.line_height().max(CLASSIC_TITLE_HEIGHT);
    let command_height =
        i32::from(show_commands || menu.extra != ObjectMenuExtra::None) * CLASSIC_COMMAND_HEIGHT;
    let (margin_top, margin_left, margin_right, margin_bottom) = menu
        .decoration
        .as_ref()
        .map(|decoration| {
            (
                decoration.border_top,
                CLASSIC_FRAME_WIDTH + decoration.border_left,
                CLASSIC_FRAME_WIDTH + decoration.border_right,
                CLASSIC_FRAME_WIDTH + decoration.border_bottom,
            )
        })
        .unwrap_or((
            0,
            CLASSIC_FRAME_WIDTH,
            CLASSIC_FRAME_WIDTH,
            CLASSIC_FRAME_WIDTH,
        ));
    let scrollbar_width = i32::from(item_count > columns * lines) * CLASSIC_SCROLLBAR_WIDTH;
    let width = columns * item_width + margin_left + margin_right + scrollbar_width;
    let height = lines * item_height + margin_top + title_height + command_height + margin_bottom;

    // Default C4Menu alignment is Right|Bottom with one C4SymbolSize (35)
    // below and two at the right (C4Menu.cpp:298, 727-745). A free anchor is
    // clamped only during InitLocation. An initialized title drag is raw.
    let default_x = area.x + area.width as i32 - 2 * CLASSIC_ITEM_SIZE - width;
    let default_y = area.y + area.height as i32 - CLASSIC_ITEM_SIZE - height;
    let (x, y) = match location {
        EngineScriptMenuLocation::Aligned => (
            if width > area.width as i32 - 2 * CLASSIC_ITEM_SIZE {
                area.x + (area.width as i32 - width) / 2
            } else {
                default_x
            },
            if height > area.height as i32 - 2 * CLASSIC_ITEM_SIZE {
                area.y + (area.height as i32 - height) / 2
            } else {
                default_y
            },
        ),
        EngineScriptMenuLocation::FreeAnchor(free_x, free_y) => (
            if width > area.width as i32 - 2 * CLASSIC_ITEM_SIZE {
                area.x + (area.width as i32 - width) / 2
            } else {
                free_x.clamp(area.x, area.x + area.width as i32 - width)
            },
            if height > area.height as i32 - 2 * CLASSIC_ITEM_SIZE {
                area.y + (area.height as i32 - height) / 2
            } else {
                free_y.clamp(area.y, area.y + area.height as i32 - height)
            },
        ),
        EngineScriptMenuLocation::Exact(x, y) => (x, y),
    };

    let client_x = x + margin_left;
    let client_y = y + margin_top + title_height;
    let client_height = lines.saturating_mul(item_height);
    let content_height = natural_lines.saturating_mul(item_height);
    let max_scroll_y = content_height.saturating_sub(client_height).max(0);
    let mut scroll_y = scroll_y.clamp(0, max_scroll_y);

    // C4Menu::AdjustPosition delegates to ScrollRangeInView only for menus
    // with more than one visible line. It moves the minimum number of pixels
    // and treats exact top/bottom equality as already visible.
    if adjust_selection && lines > 1 {
        if let Some(selection) = usize::try_from(menu.selection)
            .ok()
            .filter(|selection| *selection < menu.items.len())
        {
            let row = i32::try_from(selection)
                .unwrap_or(i32::MAX)
                .checked_div(columns)
                .unwrap_or_default();
            let item_y = row.saturating_mul(item_height);
            let item_bottom = item_y.saturating_add(item_height);
            scroll_y = if item_bottom > content_height {
                max_scroll_y
            } else if scroll_y > item_y {
                item_y
            } else if scroll_y.saturating_add(client_height) < item_bottom {
                item_bottom.saturating_sub(client_height)
            } else {
                scroll_y
            }
            .clamp(0, max_scroll_y);
        }
    }

    let first_row = scroll_y / item_height;
    let first_index = usize::try_from(first_row.saturating_mul(columns)).unwrap_or(usize::MAX);
    let visible_rows = lines.saturating_add(i32::from(scroll_y % item_height != 0));
    let visible = usize::try_from(visible_rows.saturating_mul(columns)).unwrap_or(usize::MAX);
    let client = Rect::new(
        client_x,
        client_y,
        columns.saturating_mul(item_width).max(0) as u32,
        client_height.max(0) as u32,
    );
    EngineScriptMenuLayout {
        bounds: Rect::new(x, y, width as u32, height as u32),
        title: Rect::new(x, y + margin_top, width as u32, title_height as u32),
        client,
        client_x,
        client_y,
        columns,
        lines,
        item_width,
        item_height,
        scroll_y,
        max_scroll_y,
        scrollbar: (scrollbar_width > 0).then(|| {
            Rect::new(
                client_x + columns * item_width,
                client_y,
                scrollbar_width as u32,
                (lines * item_height) as u32,
            )
        }),
        first_index,
        visible,
    }
}

/// Which part of the engine object menu's overflow scrollbar `point` hits, if
/// any. Routes through the shared `C4GUI::ScrollBar` model so the object menu,
/// the evaluation dialog and the startup chat agree
/// (`C4GuiContainers.cpp:477-623`).
pub fn engine_menu_scrollbar_hit(
    layout: &EngineScriptMenuLayout,
    point: (i32, i32),
) -> Option<crate::scrollbar::ScrollbarHit> {
    let bar = layout.scrollbar?;
    crate::scrollbar::hit(
        crate::scrollbar::bar_rect(bar.x, bar.y, bar.width as i32, bar.height as i32),
        point,
        layout.scroll_y,
        layout.max_scroll_y,
    )
}

/// The scroll a pin drag to `pointer_y` selects in the engine object menu.
pub fn engine_menu_scroll_from_pointer(
    layout: &EngineScriptMenuLayout,
    pointer_y: i32,
) -> Option<i32> {
    let bar = layout.scrollbar?;
    Some(crate::scrollbar::scroll_from_pointer(
        crate::scrollbar::bar_rect(bar.x, bar.y, bar.width as i32, bar.height as i32),
        pointer_y,
        layout.max_scroll_y,
    ))
}

/// Draws a script-created `C4ObjectMenu` from the engine's live runtime
/// state. The engine remains the sole owner of selection and item state; this
/// is deliberately a read-only presentation view.
// Preserve this public non-gamma wrapper's established flat API; it mirrors
// the full renderer below and forwards the inputs unchanged with gamma off.
#[allow(clippy::too_many_arguments)]
pub fn render_engine_script_menu(
    surface: &mut Surface,
    area: Rect,
    font: &HudFont<'_>,
    fallback_font: &dyn TextFont,
    tiny_font: Option<&HudFont<'_>>,
    menu: &clonk_engine::ObjectMenuState,
    gfx: &IngameMenuGraphics,
    title_icon: Option<&ImageData>,
    item_icons: &[Option<ImageData>],
    selected_component_icons: &[Option<ImageData>],
    show_close_button: bool,
    time_on_selection: u32,
) {
    render_engine_script_menu_with_gamma(
        surface,
        area,
        font,
        fallback_font,
        tiny_font,
        menu,
        gfx,
        title_icon,
        item_icons,
        selected_component_icons,
        show_close_button,
        time_on_selection,
        None,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn render_engine_script_menu_with_gamma(
    surface: &mut Surface,
    area: Rect,
    font: &HudFont<'_>,
    _fallback_font: &dyn TextFont,
    tiny_font: Option<&HudFont<'_>>,
    menu: &clonk_engine::ObjectMenuState,
    gfx: &IngameMenuGraphics,
    title_icon: Option<&ImageData>,
    item_icons: &[Option<ImageData>],
    selected_component_icons: &[Option<ImageData>],
    show_close_button: bool,
    time_on_selection: u32,
    gamma: Option<&GammaRamp>,
    // `C4Menu::Lines` from `C4Menu::SetSize`; see the layout helpers.
    explicit_lines: Option<i32>,
) {
    if surface.width() == 0 || surface.height() == 0 || area.width == 0 || area.height == 0 {
        return;
    }

    // External dialogs are owned by one viewport. Their raw dragged bounds
    // may leave it, but C4Viewport's primary clipper prevents painting into a
    // sibling viewport or outside the output facet.
    let previous_clip = surface.clip();
    let viewport_clip = previous_clip
        .map(|clip| clip.intersection(area).unwrap_or(Rect::new(0, 0, 0, 0)))
        .unwrap_or(area);
    surface.set_clip(viewport_clip);
    let selected = usize::try_from(menu.selection)
        .ok()
        .filter(|selection| *selection < menu.items.len());
    if matches!(menu.style, 0..=2) {
        render_engine_normal_menu(
            surface,
            area,
            font,
            tiny_font,
            menu,
            gfx,
            title_icon,
            item_icons,
            selected_component_icons,
            selected,
            show_close_button,
            time_on_selection,
            gamma,
            explicit_lines,
        );
    } else if menu.style == 3 {
        render_engine_dialog_menu(
            surface,
            area,
            font,
            menu,
            gfx,
            title_icon,
            item_icons,
            selected,
            show_close_button,
            gamma,
        );
    } else {
        panic!(
            "classic script menu style {} is unavailable; refusing generic Rust fallback",
            menu.style
        );
    }
    match previous_clip {
        Some(clip) => surface.set_clip(clip),
        None => surface.clear_clip(),
    }
}

// Source facet and destination geometry intentionally remain separate to
// mirror C4Facet's clipped edge/corner drawing operations at each call site.
#[allow(clippy::too_many_arguments)]
fn draw_decoration_facet(
    surface: &mut Surface,
    image: &ImageData,
    facet: &clonk_engine::DefinitionActionFacet,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    gamma: Option<&GammaRamp>,
) {
    if width <= 0 || height <= 0 {
        return;
    }
    draw_image_region(
        surface,
        image,
        Rect::new(facet.x, facet.y, width as u32, height as u32),
        Rect::new(x, y, width as u32, height as u32),
        gamma,
    );
}

pub fn draw_menu_decoration(
    surface: &mut Surface,
    bounds: Rect,
    decoration: &clonk_engine::ObjectMenuFrameDecoration,
    image: Option<&ImageData>,
    gamma: Option<&GammaRamp>,
) {
    clonk_frontend::classic_gui::draw_engine_box(
        surface,
        bounds.x,
        bounds.y,
        bounds.x + bounds.width as i32 - 1,
        bounds.y + bounds.height as i32 - 1,
        decoration.background_color,
        gamma,
    );
    let Some(image) = image else {
        return;
    };
    let width = bounds.width as i32;
    let height = bounds.height as i32;
    if let Some(facet) = decoration.top.as_ref().filter(|facet| facet.width > 0) {
        let mut x = decoration.border_left;
        while x < width - decoration.border_right {
            let draw_width = facet.width.min(width - decoration.border_right - x);
            draw_decoration_facet(
                surface,
                image,
                facet,
                bounds.x + x,
                bounds.y + facet.target_y,
                draw_width,
                facet.height,
                gamma,
            );
            x += facet.width;
        }
    }
    if let Some(facet) = decoration.left.as_ref().filter(|facet| facet.height > 0) {
        let mut y = decoration.border_top;
        while y < height - decoration.border_bottom {
            let draw_height = facet.height.min(height - decoration.border_bottom - y);
            draw_decoration_facet(
                surface,
                image,
                facet,
                bounds.x + facet.target_x,
                bounds.y + y,
                facet.width,
                draw_height,
                gamma,
            );
            y += facet.height;
        }
    }
    if let Some(facet) = decoration.right.as_ref().filter(|facet| facet.height > 0) {
        let mut y = decoration.border_top;
        while y < height - decoration.border_bottom {
            let draw_height = facet.height.min(height - decoration.border_bottom - y);
            draw_decoration_facet(
                surface,
                image,
                facet,
                bounds.x + width - decoration.border_right + facet.target_x,
                bounds.y + y,
                facet.width,
                draw_height,
                gamma,
            );
            y += facet.height;
        }
    }
    if let Some(facet) = decoration.bottom.as_ref().filter(|facet| facet.width > 0) {
        let mut x = decoration.border_left;
        while x < width - decoration.border_right {
            let draw_width = facet.width.min(width - decoration.border_right - x);
            draw_decoration_facet(
                surface,
                image,
                facet,
                bounds.x + x,
                bounds.y + height - decoration.border_bottom + facet.target_y,
                draw_width,
                facet.height,
                gamma,
            );
            x += facet.width;
        }
    }
    for (facet, x, y) in [
        (decoration.top_left.as_ref(), bounds.x, bounds.y),
        (
            decoration.top_right.as_ref(),
            bounds.x + width - decoration.border_right,
            bounds.y,
        ),
        (
            decoration.bottom_left.as_ref(),
            bounds.x,
            bounds.y + height - decoration.border_bottom,
        ),
        (
            decoration.bottom_right.as_ref(),
            bounds.x + width - decoration.border_right,
            bounds.y + height - decoration.border_bottom,
        ),
    ] {
        if let Some(facet) = facet {
            draw_decoration_facet(
                surface,
                image,
                facet,
                x + facet.target_x,
                y + facet.target_y,
                facet.width,
                facet.height,
                gamma,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_engine_dialog_menu(
    surface: &mut Surface,
    area: Rect,
    font: &HudFont<'_>,
    menu: &clonk_engine::ObjectMenuState,
    gfx: &IngameMenuGraphics,
    title_icon: Option<&ImageData>,
    item_icons: &[Option<ImageData>],
    selected: Option<usize>,
    show_close_button: bool,
    gamma: Option<&GammaRamp>,
) {
    let layout = relocate_dialog_menu_layout(
        dialog_script_menu_layout_with_images(area, font, menu, item_icons, &gfx.font_images),
        gfx.menu_location,
    );
    if let Some(decoration) = menu.decoration.as_ref() {
        draw_menu_decoration(
            surface,
            layout.bounds,
            decoration,
            gfx.frame_decoration.as_ref(),
            gamma,
        );
    } else {
        fill_rect(
            surface,
            layout.bounds,
            Color::new(0, 0, 0, CLASSIC_BG_ALPHA),
            gamma,
        );
        draw_3d_frame(surface, layout.bounds, gamma);
    }

    if let Some(title) = layout.title {
        if let Some(caption_bar) = gfx.caption_bar.as_ref() {
            draw_caption_bar(surface, title, caption_bar, gamma);
        }
        let title_height = title.height as i32;
        let icon_indent = title_icon.map_or(0, |icon| {
            let icon = precompose_definition_menu_title_icon(icon);
            let side = (title_height - 2).max(0) as u32;
            draw_image_region_aspect(
                surface,
                &icon,
                Rect::new(0, 0, icon.width(), icon.height()),
                Rect::new(title.x + 1, title.y + 1, side, side),
                false,
                gamma,
            );
            title_height
        });
        let text_right = if show_close_button {
            title.x + title.width as i32 - 20
        } else {
            title.x + title.width as i32
        };
        let previous_clip = surface.clip();
        let title_clip = Rect::new(
            title.x + icon_indent,
            title.y,
            text_right.saturating_sub(title.x + icon_indent).max(0) as u32,
            title.height,
        );
        let nested_clip = previous_clip
            .map(|clip| {
                clip.intersection(title_clip)
                    .unwrap_or(Rect::new(0, 0, 0, 0))
            })
            .unwrap_or(title_clip);
        surface.set_clip(nested_clip);
        let title_text = engine_script_presentation_text(&menu.caption);
        render_text_spec(
            surface,
            font,
            &title_text,
            &gfx.font_images,
            title.x + icon_indent + 5,
            title.y + (title_height - font.line_height()) / 2 - 1,
            CLASSIC_CAPTION_COLOR,
            gamma,
        );
        match previous_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }
        if show_close_button {
            if let Some(gui_icons) = gfx.gui_icons.as_ref() {
                let source_x = i32::from(CLASSIC_CLOSE_ICON % 6) * 40;
                let source_y = i32::from(CLASSIC_CLOSE_ICON / 6) * 40;
                draw_image_region_aspect(
                    surface,
                    gui_icons,
                    Rect::new(source_x, source_y, 40, 40),
                    Rect::new(title.x + title.width as i32 - 20, title.y + 4, 16, 16),
                    false,
                    gamma,
                );
            }
        }
    }

    if let Some((index, portrait)) = layout.portrait {
        if let Some(image) = item_icons.get(index).and_then(Option::as_ref) {
            draw_image_region_aspect(
                surface,
                image,
                Rect::new(0, 0, image.width(), image.height()),
                Rect::new(
                    portrait.x,
                    portrait.y,
                    CLASSIC_PICTURE_SIZE as u32,
                    portrait.height,
                ),
                true,
                gamma,
            );
        }
    }

    for row in &layout.rows {
        let item = &menu.items[row.index];
        if selected == Some(row.index) && item.text_display_progress != 0 {
            fill_rect(surface, row.rect, CLASSIC_SELECTION_COLOR, gamma);
        }
        if item.text_display_progress != 0 {
            if let (Some(symbol), Some(image)) = (
                row.symbol_rect,
                item_icons.get(row.index).and_then(Option::as_ref),
            ) {
                draw_image_region_aspect(
                    surface,
                    image,
                    Rect::new(0, 0, image.width(), image.height()),
                    symbol,
                    true,
                    gamma,
                );
            }
        }

        let previous_clip = surface.clip();
        let text_clip = previous_clip
            .map(|clip| {
                clip.intersection(row.text_rect)
                    .unwrap_or(Rect::new(0, 0, 0, 0))
            })
            .unwrap_or(row.text_rect);
        surface.set_clip(text_clip);
        let visible = dialog_visible_caption(item);
        let text = layout_info_text(font, &visible, row.text_rect.width as i32, &gfx.font_images);
        render_info_text(
            surface,
            font,
            &text,
            &gfx.font_images,
            row.text_rect.x,
            row.text_rect.y,
            CLASSIC_CAPTION_COLOR,
            gamma,
        );
        match previous_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }

        if item.count != MENU_ITEM_NO_COUNT {
            font.draw_with_gamma(
                surface,
                row.rect.x + row.rect.width as i32 - 1,
                row.rect.y + row.rect.height as i32 - 1 - font.line_height(),
                &format!("{}x", item.count),
                CLASSIC_CAPTION_COLOR,
                TextAlign::Right,
                gamma,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_engine_normal_menu(
    surface: &mut Surface,
    area: Rect,
    font: &HudFont<'_>,
    tiny_font: Option<&HudFont<'_>>,
    menu: &clonk_engine::ObjectMenuState,
    gfx: &IngameMenuGraphics,
    title_icon: Option<&ImageData>,
    item_icons: &[Option<ImageData>],
    selected_component_icons: &[Option<ImageData>],
    selected: Option<usize>,
    show_close_button: bool,
    time_on_selection: u32,
    gamma: Option<&GammaRamp>,
    // `C4Menu::Lines` as last written by `C4Menu::SetSize` and not yet
    // overwritten by `InitLocation` (C4Menu.cpp:635-640,713-721); `None`
    // recomputes the row count from the item count.
    explicit_lines: Option<i32>,
) {
    let layout = engine_script_menu_layout_with_presentation(
        area,
        font,
        menu,
        gfx.show_commands,
        &gfx.font_images,
        gfx.menu_location,
        gfx.menu_scroll_y,
        false,
        explicit_lines,
    );
    let bounds = layout.bounds;
    let x = bounds.x;
    let y = bounds.y;
    let width = bounds.width as i32;
    let height = bounds.height as i32;
    let title_height = layout.title.height as i32;

    if let Some(decoration) = menu.decoration.as_ref() {
        draw_menu_decoration(
            surface,
            bounds,
            decoration,
            gfx.frame_decoration.as_ref(),
            gamma,
        );
    } else {
        fill_rect(
            surface,
            bounds,
            Color::new(0, 0, 0, CLASSIC_BG_ALPHA),
            gamma,
        );
        draw_3d_frame(surface, bounds, gamma);
    }

    let title_rect = layout.title;
    if let Some(caption_bar) = gfx.caption_bar.as_ref() {
        draw_caption_bar(surface, title_rect, caption_bar, gamma);
    }
    let icon_indent = if menu.title_symbol == ObjectMenuSymbol::Definition {
        title_icon.map_or(0, |icon| {
            let icon = precompose_definition_menu_title_icon(icon);
            let side = (title_height - 2) as u32;
            draw_image_region_aspect(
                surface,
                &icon,
                Rect::new(0, 0, icon.width(), icon.height()),
                Rect::new(title_rect.x + 1, title_rect.y + 1, side, side),
                false,
                gamma,
            );
            title_height
        })
    } else if menu.title_symbol == ObjectMenuSymbol::InfoTitle {
        let side = (title_height - 2) as u32;
        draw_ok_cancel(
            surface,
            gfx,
            title_rect.x + 1,
            title_rect.y + 1,
            side,
            0,
            1,
            gamma,
        );
        title_height
    } else {
        let side = (title_height - 2) as u32;
        let image = command_image_for_menu_symbol(menu.title_symbol, title_icon.cloned(), gfx);
        draw_command_image_cell_with_gamma(
            surface,
            &gfx.hud,
            Rect::new(title_rect.x + 1, title_rect.y + 1, side, side),
            &image,
            gamma,
        );
        title_height
    };
    // WoodenLabel restricts its title text to the label bounds and reserves
    // a 20px right indent whenever Dialog::SetTitle adds the mouse close
    // button. Store/restore the caller's primary clipper around that child
    // draw (C4GuiLabels.cpp:168-209; C4GuiDialogs.cpp:386-421).
    let previous_clip = surface.clip();
    let title_text_right = if show_close_button {
        layout.close_button_rect().x
    } else {
        x + width
    };
    let title_text_clip = Rect::new(
        title_rect.x + icon_indent,
        title_rect.y,
        title_text_right
            .saturating_sub(title_rect.x + icon_indent)
            .max(0) as u32,
        title_rect.height,
    );
    let nested_clip = previous_clip
        .map(|clip| {
            clip.intersection(title_text_clip)
                .unwrap_or(Rect::new(0, 0, 0, 0))
        })
        .unwrap_or(title_text_clip);
    surface.set_clip(nested_clip);
    render_text_spec(
        surface,
        font,
        &engine_script_menu_title(menu),
        &gfx.font_images,
        title_rect.x + icon_indent + 5,
        title_rect.y + (title_height - font.line_height()) / 2 - 1,
        CLASSIC_CAPTION_COLOR,
        gamma,
    );
    match previous_clip {
        Some(clip) => surface.set_clip(clip),
        None => surface.clear_clip(),
    }
    if show_close_button {
        if let Some(gui_icons) = gfx.gui_icons.as_ref() {
            let source_x = i32::from(CLASSIC_CLOSE_ICON % 6) * 40;
            let source_y = i32::from(CLASSIC_CLOSE_ICON / 6) * 40;
            draw_image_region_aspect(
                surface,
                gui_icons,
                Rect::new(source_x, source_y, 40, 40),
                layout.close_button_rect(),
                false,
                gamma,
            );
        }
    }

    let visible = layout.visible;
    let first_index = layout.first_index;
    let previous_client_clip = surface.clip();
    let client_clip = previous_client_clip
        .map(|clip| {
            clip.intersection(layout.client)
                .unwrap_or(Rect::new(0, 0, 0, 0))
        })
        .unwrap_or(layout.client);
    surface.set_clip(client_clip);
    for (index, item) in menu
        .items
        .iter()
        .enumerate()
        .skip(first_index)
        .take(visible)
    {
        let Some(cell) = layout.item_rect(index) else {
            continue;
        };
        let cell_x = cell.x;
        let cell_y = cell.y;
        if menu.style != 2 && selected == Some(index) && item.text_display_progress != 0 {
            fill_rect(surface, cell, CLASSIC_SELECTION_COLOR, gamma);
        }
        let picture = item_icons.get(index).cloned().flatten();
        let symbol_width = match menu.style {
            1 => layout.item_height,
            2 => picture.as_ref().map_or_else(
                || i32::from(item.symbol != ObjectMenuSymbol::Definition) * CLASSIC_PICTURE_SIZE,
                |image| {
                    i32::try_from(image.width())
                        .unwrap_or(i32::MAX)
                        .min(CLASSIC_PICTURE_SIZE)
                },
            ),
            _ => layout.item_width,
        };
        let symbol_cell = Rect::new(
            cell_x,
            cell_y,
            symbol_width.max(0) as u32,
            layout.item_height as u32,
        );
        let image = command_image_for_menu_symbol(item.symbol, picture, gfx);
        if symbol_width > 0 {
            draw_command_image_cell_with_gamma(surface, &gfx.hud, symbol_cell, &image, gamma);
        }
        let caption = engine_script_presentation_text(&item.caption);
        let info_caption = engine_script_presentation_text(&item.info_caption);
        match menu.style {
            1 => render_text_spec(
                surface,
                font,
                &caption,
                &gfx.font_images,
                cell_x + symbol_width,
                cell_y,
                CLASSIC_CAPTION_COLOR,
                gamma,
            ),
            2 => {
                let text_rect = Rect::new(
                    cell_x + symbol_width,
                    cell_y,
                    (layout.item_width - symbol_width).max(0) as u32,
                    layout.item_height as u32,
                );
                let previous_clip = surface.clip();
                let nested_clip = previous_clip
                    .map(|clip| {
                        clip.intersection(text_rect)
                            .unwrap_or(Rect::new(0, 0, 0, 0))
                    })
                    .unwrap_or(text_rect);
                surface.set_clip(nested_clip);
                let info_layout = layout_info_text(
                    font,
                    &info_caption,
                    text_rect.width as i32,
                    &gfx.font_images,
                );
                render_info_text(
                    surface,
                    font,
                    &info_layout,
                    &gfx.font_images,
                    text_rect.x,
                    text_rect.y,
                    CLASSIC_CAPTION_COLOR,
                    gamma,
                );
                match previous_clip {
                    Some(clip) => surface.set_clip(clip),
                    None => surface.clear_clip(),
                }
            }
            _ => {}
        }
        if item.count != MENU_ITEM_NO_COUNT {
            font.draw_with_gamma(
                surface,
                cell_x + layout.item_width - 1,
                cell_y + layout.item_height - 1 - font.line_height(),
                &format!("{}x", item.count),
                CLASSIC_CAPTION_COLOR,
                TextAlign::Right,
                gamma,
            );
        }
    }
    match previous_client_clip {
        Some(clip) => surface.set_clip(clip),
        None => surface.clear_clip(),
    }

    if gfx.show_commands || menu.extra != ObjectMenuExtra::None {
        let extra = Rect::new(
            x + 1,
            y + height - CLASSIC_COMMAND_HEIGHT - 1,
            (width - 2) as u32,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        // C4Menu::DrawFrame divider (C4Menu.cpp:846-849,932-935);
        // CStdDDraw::DrawFrame never rasterizes the bottom-right corner
        // (capture: Drachenfels divider (1208,662) stays background).
        draw_hv_border(surface, extra, CLASSIC_EXTRA_FRAME_COLOR, gamma);
        let mut remaining = extra;
        if gfx.show_commands {
            let mut truncate_control = || {
                // C4Facet::TruncateSection(C4FCT_Left) returns an empty facet
                // without changing the source once another square no longer
                // fits (C4Facet.cpp:182-217). A five-column normal menu can
                // therefore show at most five of the six controls requested by
                // an item with Command2.
                let size = remaining.height;
                (size <= remaining.width && size != 0).then(|| {
                    let cell = Rect::new(remaining.x, remaining.y, size, size);
                    remaining.x += size as i32;
                    remaining.width -= size;
                    cell
                })
            };
            let tiny = tiny_font.unwrap_or(font);
            if menu.style != 2 {
                if let Some(cell) = truncate_control() {
                    draw_command_key(
                        surface,
                        gfx,
                        tiny,
                        cell.x,
                        cell.y,
                        cell.width,
                        3,
                        &gfx.throw_key,
                        gamma,
                    );
                }
                if let Some(cell) = truncate_control() {
                    draw_ok_cancel(surface, gfx, cell.x, cell.y, cell.width, 0, 0, gamma);
                }
                if selected
                    .and_then(|selection| menu.items.get(selection))
                    .is_some_and(|item| !item.command2.is_empty())
                {
                    if let Some(cell) = truncate_control() {
                        draw_command_key(
                            surface,
                            gfx,
                            tiny,
                            cell.x,
                            cell.y,
                            cell.width,
                            11,
                            &gfx.special2_key,
                            gamma,
                        );
                    }
                    if let Some(cell) = truncate_control() {
                        draw_ok_cancel(surface, gfx, cell.x, cell.y, cell.width, 2, 1, gamma);
                    }
                }
            }
            if let Some(cell) = truncate_control() {
                draw_command_key(
                    surface,
                    gfx,
                    tiny,
                    cell.x,
                    cell.y,
                    cell.width,
                    5,
                    &gfx.dig_key,
                    gamma,
                );
            }
            if let Some(cell) = truncate_control() {
                if menu
                    .items
                    .iter()
                    .any(|item| item.symbol == ObjectMenuSymbol::Exit)
                {
                    draw_command_image_cell_with_gamma(
                        surface,
                        &gfx.hud,
                        cell,
                        &CommandImage::Exit,
                        gamma,
                    );
                } else {
                    draw_ok_cancel(surface, gfx, cell.x, cell.y, cell.width, 1, 0, gamma);
                }
            }
        }
        let selected_item = selected.and_then(|selection| menu.items.get(selection));
        match menu.extra {
            ObjectMenuExtra::Components => {
                if let Some(item) = selected_item {
                    draw_component_footer(
                        surface,
                        font,
                        gfx,
                        remaining,
                        &item.components,
                        selected_component_icons,
                        gamma,
                    );
                }
            }
            ObjectMenuExtra::Value => {
                if let Some(value) = selected_item.and_then(|item| item.value) {
                    let value = value.to_string();
                    let value_width = font.text_width(&value);
                    let right = remaining.x + remaining.width as i32 - 1;
                    if let Some(wealth) = gfx.hud.wealth.as_ref() {
                        let wealth_rect = Rect::new(
                            right - value_width - 2 * CLASSIC_COMMAND_HEIGHT,
                            remaining.y,
                            (2 * CLASSIC_COMMAND_HEIGHT) as u32,
                            CLASSIC_COMMAND_HEIGHT as u32,
                        );
                        draw_image_region_aspect(
                            surface,
                            wealth,
                            Rect::new(0, 0, wealth.width(), wealth.height()),
                            wealth_rect,
                            false,
                            gamma,
                        );
                    }
                    font.draw_with_gamma(
                        surface,
                        right,
                        remaining.y,
                        &value,
                        CLASSIC_CAPTION_COLOR,
                        TextAlign::Right,
                        gamma,
                    );
                }
            }
            ObjectMenuExtra::MagicValue | ObjectMenuExtra::LiveMagicValue => {
                if let Some(value) = selected_item.and_then(|item| item.value) {
                    draw_magic_value_footer(
                        surface,
                        font,
                        gfx,
                        remaining,
                        value,
                        menu.extra_data,
                        gamma,
                    );
                }
            }
            ObjectMenuExtra::ComponentsMagic | ObjectMenuExtra::ComponentsLiveMagic => {
                if let Some(item) = selected_item {
                    if let Some(value) = item.value {
                        let magic_x = draw_magic_value_footer(
                            surface,
                            font,
                            gfx,
                            remaining,
                            value,
                            menu.extra_data,
                            gamma,
                        );
                        let component_width = magic_x.saturating_sub(remaining.x + 5) as u32;
                        draw_component_footer(
                            surface,
                            font,
                            gfx,
                            Rect::new(
                                remaining.x,
                                remaining.y,
                                component_width.min(remaining.width),
                                remaining.height,
                            ),
                            &item.components,
                            selected_component_icons,
                            gamma,
                        );
                    }
                }
            }
            ObjectMenuExtra::None | ObjectMenuExtra::Info | ObjectMenuExtra::Unknown(_) => {}
        }
    }

    if menu.style != 2 && time_on_selection >= 90 {
        if let Some((selection, item)) = selected
            .and_then(|selection| menu.items.get(selection).map(|item| (selection, item)))
            .filter(|(_, item)| !item.info_caption.is_empty())
        {
            let Some(cell) = layout.item_unclipped_rect(selection) else {
                return;
            };
            let cell_x = cell.x;
            let cell_y = cell.y;
            let info_caption = engine_script_presentation_text(&item.info_caption);
            draw_text_spec_tooltip(
                surface,
                font,
                area,
                cell_x,
                cell_y,
                &info_caption,
                &gfx.font_images,
                gamma,
            );
        }
    }

    // `C4GUI::ScrollWindow` draws its bar after the client contents
    // (C4GuiContainers.cpp:309-470). A missing facet leaves it undrawn, as a
    // null `C4Facet` does in C++.
    if let (Some(bar), Some(scroll)) = (layout.scrollbar, gfx.scroll.as_ref()) {
        crate::scrollbar::draw_classic_scrollbar(
            surface,
            crate::scrollbar::bar_rect(bar.x, bar.y, bar.width as i32, bar.height as i32),
            scroll,
            crate::scrollbar::pin_offset(bar.height as i32, layout.scroll_y, layout.max_scroll_y),
            layout.max_scroll_y,
            gamma,
        );
    }
}

#[derive(Clone, Debug)]
struct ContextMenuItem {
    entry: ContextMenuEntry,
}

impl ContextMenuItem {
    fn new(mut entry: ContextMenuEntry) -> Self {
        entry.label = engine_script_presentation_text(&entry.label);
        entry.description = entry
            .description
            .map(|description| engine_script_presentation_text(&description));
        Self { entry }
    }

    fn selection(&self, crew_id: ObjectId) -> ContextMenuSelection {
        ContextMenuSelection {
            crew_id,
            function: self.entry.function.clone(),
            label: self.entry.label.clone(),
            description: self.entry.description.clone(),
        }
    }
}

impl MenuEntry for ContextMenuItem {
    fn label(&self) -> &str {
        &self.entry.label
    }

    fn description(&self) -> Option<&str> {
        self.entry.description.as_deref()
    }

    fn count(&self) -> usize {
        1
    }
}

#[derive(Clone, Debug)]
struct BuildMenuItem {
    definition_id: String,
    label: String,
    description: Option<String>,
    available: u32,
    icon: Option<ImageData>,
}

impl BuildMenuItem {
    fn available(&self) -> u32 {
        self.available
    }
}

impl MenuEntry for BuildMenuItem {
    fn label(&self) -> &str {
        &self.label
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn count(&self) -> usize {
        self.available as usize
    }

    fn icon(&self) -> Option<&ImageData> {
        self.icon.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuMode {
    Inventory,
    Container,
    Context,
    Build,
}

impl MenuMode {
    fn title_suffix(self) -> &'static str {
        match self {
            MenuMode::Inventory => "Inventory",
            MenuMode::Container => "Contents",
            MenuMode::Context => "Actions",
            MenuMode::Build => "Build",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectMenuCommand {
    Focus,
    DropAll,
    Take,
    TakeAll,
}

#[derive(Clone, Debug)]
pub enum ObjectMenuAction {
    Close,
    Execute {
        command: ObjectMenuCommand,
        selection: ObjectMenuSelection,
    },
    Build {
        selection: BuildMenuSelection,
        amount: u32,
    },
    Context {
        selection: ContextMenuSelection,
    },
}

#[derive(Clone, Debug)]
pub struct ObjectMenuSelection {
    pub crew_id: ObjectId,
    pub primary_id: ObjectId,
    pub instances: Vec<ObjectId>,
    pub definition_id: String,
    pub label: String,
    pub source_container: Option<ObjectId>,
}

impl ObjectMenuSelection {
    pub fn count(&self) -> usize {
        self.instances.len()
    }
}

#[derive(Clone, Debug)]
pub struct BuildMenuSelection {
    pub crew_id: ObjectId,
    pub owner: i32,
    pub definition_id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct ContextMenuSelection {
    pub crew_id: ObjectId,
    pub function: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
struct ContainerState {
    id: ObjectId,
    label: String,
    items: Vec<ObjectMenuItem>,
    known_empty: bool,
}

#[derive(Clone, Debug)]
pub struct ObjectMenuState {
    crew_id: ObjectId,
    crew_label: String,
    owner: i32,
    mode: MenuMode,
    inventory: Vec<ObjectMenuItem>,
    container: Option<ContainerState>,
    context: Vec<ContextMenuItem>,
    build: Vec<BuildMenuItem>,
    inventory_selected: Option<usize>,
    container_selected: Option<usize>,
    context_selected: Option<usize>,
    build_selected: Option<usize>,
    inventory_known_empty: bool,
}

impl ObjectMenuState {
    pub fn mode(&self) -> MenuMode {
        self.mode
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_mode_for_parity_test(&mut self, mode: MenuMode) {
        self.mode = mode;
    }

    pub fn for_player(
        owner: i32,
        engine: &mut Engine,
        snapshot: &SimulationSnapshot,
    ) -> Option<Self> {
        let cursor = engine.crew_cursor(owner)?;
        Self::new(engine, snapshot, cursor)
    }

    pub fn new(
        engine: &mut Engine,
        snapshot: &SimulationSnapshot,
        crew_id: ObjectId,
    ) -> Option<Self> {
        let crew = snapshot.object(crew_id)?.clone();
        let owner = crew.owner;
        let crew_label = engine
            .definition_name(&crew.definition_id)
            .unwrap_or(&crew.definition_id)
            .to_string();
        let inventory = collect_inventory(engine, snapshot, &crew);
        let inventory_known_empty = inventory.is_empty();
        let context = collect_context_items(engine, &crew);
        let build = collect_build_items(engine, &crew);
        let inventory_selected = if inventory.is_empty() { None } else { Some(0) };
        let context_selected = if context.is_empty() { None } else { Some(0) };
        let build_selected = if build.is_empty() { None } else { Some(0) };
        let mut mode = MenuMode::Inventory;
        if inventory.is_empty() && !context.is_empty() {
            mode = MenuMode::Context;
        }
        if mode == MenuMode::Inventory && inventory.is_empty() && !build.is_empty() {
            mode = MenuMode::Build;
        }
        Some(Self {
            crew_id,
            crew_label,
            owner,
            mode,
            inventory,
            container: None,
            context,
            build,
            inventory_selected,
            container_selected: None,
            context_selected,
            build_selected,
            inventory_known_empty,
        })
    }

    pub fn focus_inventory_mode(&mut self) {
        self.mode = MenuMode::Inventory;
        if self.inventory_selected.is_none() && !self.inventory.is_empty() {
            self.inventory_selected = Some(0);
        }
    }

    pub fn focus_context_mode(&mut self) {
        if self.context.is_empty() {
            return;
        }
        self.mode = MenuMode::Context;
        if self.context_selected.is_none() {
            self.context_selected = Some(0);
        }
    }

    pub fn focus_container_mode(
        &mut self,
        engine: &mut Engine,
        snapshot: &SimulationSnapshot,
        container_id: ObjectId,
    ) -> bool {
        let container = match snapshot.object(container_id) {
            Some(container) => container,
            None => {
                self.container = None;
                self.container_selected = None;
                self.ensure_valid_mode();
                return false;
            }
        };

        let label = engine
            .definition_name(&container.definition_id)
            .unwrap_or(&container.definition_id)
            .to_string();
        let items = collect_container_items(engine, snapshot, container);
        let known_empty = items.is_empty();
        self.container = Some(ContainerState {
            id: container_id,
            label,
            items,
            known_empty,
        });

        if let Some(state) = self.container.as_ref() {
            self.container_selected = if state.items.is_empty() {
                None
            } else {
                Some(0)
            };
        } else {
            self.container_selected = None;
        }

        self.mode = MenuMode::Container;
        self.ensure_selection_for_mode();
        true
    }

    pub fn refresh(&mut self, engine: &mut Engine, snapshot: &SimulationSnapshot) -> bool {
        let crew = match snapshot.object(self.crew_id) {
            Some(crew) => crew,
            None => return false,
        };
        self.owner = crew.owner;
        self.crew_label = engine
            .definition_name(&crew.definition_id)
            .unwrap_or(&crew.definition_id)
            .to_string();
        self.inventory = collect_inventory(engine, snapshot, crew);
        self.context = collect_context_items(engine, crew);
        self.build = collect_build_items(engine, crew);
        self.inventory_known_empty = self.inventory.is_empty();
        clamp_selection(&mut self.inventory_selected, self.inventory.len());
        if let Some(state) = self.container.as_mut() {
            if let Some(container) = snapshot.object(state.id) {
                state.label = engine
                    .definition_name(&container.definition_id)
                    .unwrap_or(&container.definition_id)
                    .to_string();
                state.items = collect_container_items(engine, snapshot, container);
                state.known_empty = state.items.is_empty();
                clamp_selection(&mut self.container_selected, state.items.len());
                if self.container_selected.is_none() && !state.items.is_empty() {
                    self.container_selected = Some(0);
                }
            } else {
                self.container = None;
                self.container_selected = None;
                if self.mode == MenuMode::Container {
                    self.mode = MenuMode::Inventory;
                }
            }
        }
        clamp_selection(&mut self.context_selected, self.context.len());
        clamp_selection(&mut self.build_selected, self.build.len());
        if self.inventory_selected.is_none() && !self.inventory.is_empty() {
            self.inventory_selected = Some(0);
        }
        if self.context_selected.is_none() && !self.context.is_empty() {
            self.context_selected = Some(0);
        }
        if self.build_selected.is_none() && !self.build.is_empty() {
            self.build_selected = Some(0);
        }
        self.ensure_valid_mode();
        true
    }

    pub fn handle_command(
        &mut self,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Option<ObjectMenuAction> {
        if !matches!(
            kind,
            CommandKind::Press | CommandKind::Single | CommandKind::Double
        ) {
            return None;
        }

        match command {
            ControlCommand::MenuUp => {
                self.advance_selection(-1);
                None
            }
            ControlCommand::MenuDown => {
                self.advance_selection(1);
                None
            }
            ControlCommand::MenuLeft => {
                if self.step_mode(-1, false) {
                    return None;
                }
                self.advance_selection(-1);
                None
            }
            ControlCommand::MenuRight => {
                if self.step_mode(1, false) {
                    return None;
                }
                self.advance_selection(1);
                None
            }
            ControlCommand::MenuSelect | ControlCommand::MenuEnter => match self.mode {
                MenuMode::Inventory => self.activation_action(ObjectMenuCommand::Focus),
                MenuMode::Container => self.container_action(ObjectMenuCommand::Take),
                MenuMode::Context => self.context_action(),
                MenuMode::Build => self.build_action(1),
            },
            ControlCommand::MenuEnterAll => match self.mode {
                MenuMode::Inventory => self.activation_action(ObjectMenuCommand::DropAll),
                MenuMode::Context => self.context_action(),
                MenuMode::Container => self.container_action(ObjectMenuCommand::TakeAll),
                MenuMode::Build => {
                    let amount = self
                        .build_selected
                        .and_then(|index| self.build.get(index))
                        .map(|item| item.available())
                        .unwrap_or(0);
                    self.build_action(amount)
                }
            },
            ControlCommand::MenuClose => Some(ObjectMenuAction::Close),
            ControlCommand::MenuShowText => None,
            _ => None,
        }
    }

    pub fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        self.render_with_gamma(surface, font, None);
    }

    pub fn render_with_gamma(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        gamma: Option<&GammaRamp>,
    ) {
        let width = surface.width() as i32;
        let height = surface.height() as i32;
        if width <= 0 || height <= 0 {
            return;
        }

        fill_rect(surface, surface.bounds(), BACKDROP_COLOR, gamma);

        let hint = if self.available_modes().len() >= 2 {
            Some(MODE_HINT)
        } else {
            None
        };

        match self.mode {
            MenuMode::Inventory => {
                let title = format!("{} {}", self.crew_label, self.mode.title_suffix());
                let empty_message = if self.inventory_known_empty {
                    "Inventory is empty."
                } else {
                    "Inventory unavailable."
                };
                Self::render_entries(
                    surface,
                    font,
                    &self.inventory,
                    self.inventory_selected,
                    &title,
                    empty_message,
                    hint,
                    gamma,
                );
            }
            MenuMode::Container => {
                let mut title = format!("{} {}", self.crew_label, self.mode.title_suffix());
                let mut empty_message = "Container unavailable.";
                if let Some(state) = self.container.as_ref() {
                    title = format!("{} {}", state.label, self.mode.title_suffix());
                    if state.known_empty {
                        empty_message = "Container is empty.";
                    }
                    Self::render_entries(
                        surface,
                        font,
                        state.items.as_slice(),
                        self.container_selected,
                        &title,
                        empty_message,
                        hint,
                        gamma,
                    );
                } else {
                    let empty: &[ObjectMenuItem] = &[];
                    Self::render_entries(
                        surface,
                        font,
                        empty,
                        None,
                        &title,
                        empty_message,
                        hint,
                        gamma,
                    );
                }
            }
            MenuMode::Context => {
                let title = format!("{} {}", self.crew_label, self.mode.title_suffix());
                Self::render_entries(
                    surface,
                    font,
                    &self.context,
                    self.context_selected,
                    &title,
                    "No actions available.",
                    hint,
                    gamma,
                );
            }
            MenuMode::Build => {
                let title = format!("{} {}", self.crew_label, self.mode.title_suffix());
                Self::render_entries(
                    surface,
                    font,
                    &self.build,
                    self.build_selected,
                    &title,
                    "No home base supplies available.",
                    hint,
                    gamma,
                );
            }
        }
    }

    // This fallback renderer keeps each piece of presentation state explicit;
    // it is the single boundary shared by every menu mode.
    #[allow(clippy::too_many_arguments)]
    fn render_entries<E: MenuEntry>(
        surface: &mut Surface,
        font: &dyn TextFont,
        items: &[E],
        selected: Option<usize>,
        title: &str,
        empty_message: &str,
        hint: Option<&str>,
        gamma: Option<&GammaRamp>,
    ) {
        let width = surface.width() as i32;
        let height = surface.height() as i32;

        let mut panel_width = (width as f32 * 0.42).round() as i32;
        panel_width = panel_width.clamp(PANEL_WIDTH_MIN, PANEL_WIDTH_MAX);
        panel_width = panel_width
            .min(width - PANEL_PADDING * 2)
            .max(PANEL_WIDTH_MIN);

        let list_height = if items.is_empty() {
            ITEM_HEIGHT
        } else {
            (items.len() as i32).saturating_mul(ITEM_HEIGHT + ITEM_SPACING) - ITEM_SPACING
        };
        let mut panel_height = (PANEL_PADDING * 2) + TITLE_GAP + list_height.max(ITEM_HEIGHT);
        if hint.is_some() {
            panel_height += ITEM_SPACING + DETAIL_FONT_SIZE as i32 + 6;
        }

        let panel_x = (width - panel_width) / 2;
        let panel_y = (height - panel_height) / 2;

        let panel_rect = Rect::new(panel_x, panel_y, panel_width as u32, panel_height as u32);
        fill_rect(surface, panel_rect, PANEL_COLOR, gamma);
        draw_border(surface, panel_rect, PANEL_BORDER, gamma);

        let mut cursor_y = panel_y + PANEL_PADDING;
        clonk_frontend::draw_text_with_gamma(
            font,
            surface,
            (panel_x + PANEL_PADDING) as f32,
            cursor_y as f32,
            title,
            TITLE_FONT_SIZE,
            TITLE_COLOR,
            gamma,
        );

        cursor_y += TITLE_GAP;
        if items.is_empty() {
            clonk_frontend::draw_text_with_gamma(
                font,
                surface,
                (panel_x + PANEL_PADDING) as f32,
                (cursor_y + 10) as f32,
                empty_message,
                ITEM_FONT_SIZE,
                MUTED_TEXT_COLOR,
                gamma,
            );
            return;
        }

        for (index, item) in items.iter().enumerate() {
            let row_rect = Rect::new(
                panel_x + PANEL_PADDING,
                cursor_y,
                (panel_width - PANEL_PADDING * 2) as u32,
                ITEM_HEIGHT as u32,
            );
            if Some(index) == selected {
                fill_rect(surface, row_rect, HIGHLIGHT_COLOR, gamma);
            }

            let primary_color = if !item.selectable() {
                MUTED_TEXT_COLOR
            } else if Some(index) == selected {
                EMPHASIS_TEXT_COLOR
            } else {
                TEXT_COLOR
            };

            let label_text = item
                .count_label()
                .map(|count| format!("{} ({count})", item.label()))
                .unwrap_or_else(|| item.label().to_string());

            let mut text_x = row_rect.x + 12;
            if let Some(icon) = item.icon() {
                let icon_size = (ITEM_HEIGHT - 10).max(12) as u32;
                let icon_rect = Rect::new(
                    row_rect.x + 8,
                    row_rect.y + (ITEM_HEIGHT - icon_size as i32) / 2,
                    icon_size,
                    icon_size,
                );
                draw_menu_icon(surface, icon_rect, icon, gamma);
                text_x = icon_rect.x + icon_rect.width as i32 + 8;
            }
            clonk_frontend::draw_text_with_gamma(
                font,
                surface,
                text_x as f32,
                (row_rect.y + 8) as f32,
                &label_text,
                ITEM_FONT_SIZE,
                primary_color,
                gamma,
            );

            if let Some(description) = item.description() {
                clonk_frontend::draw_text_with_gamma(
                    font,
                    surface,
                    text_x as f32,
                    (row_rect.y + 22) as f32,
                    description,
                    DETAIL_FONT_SIZE,
                    MUTED_TEXT_COLOR,
                    gamma,
                );
            }

            cursor_y += ITEM_HEIGHT + ITEM_SPACING;
        }

        if let Some(hint) = hint {
            clonk_frontend::draw_text_with_gamma(
                font,
                surface,
                (panel_x + PANEL_PADDING) as f32,
                (panel_y + panel_height - PANEL_PADDING - 18) as f32,
                hint,
                DETAIL_FONT_SIZE,
                MUTED_TEXT_COLOR,
                gamma,
            );
        }
    }

    fn advance_selection(&mut self, delta: i32) {
        let (selected, len) = self.current_selection_mut();
        if len == 0 {
            return;
        }
        let len = len as i32;
        let next = match selected {
            Some(index) => (*index as i32 + delta).rem_euclid(len),
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        *selected = Some(next as usize);
    }

    fn activation_action(&self, command: ObjectMenuCommand) -> Option<ObjectMenuAction> {
        let index = self.inventory_selected?;
        let item = self.inventory.get(index)?;
        let primary_id = item.primary_object()?;
        Some(ObjectMenuAction::Execute {
            command,
            selection: ObjectMenuSelection {
                crew_id: self.crew_id,
                primary_id,
                instances: item.instances.clone(),
                definition_id: item.definition_id.clone(),
                label: item.label.clone(),
                source_container: None,
            },
        })
    }

    fn container_action(&self, command: ObjectMenuCommand) -> Option<ObjectMenuAction> {
        let state = self.container.as_ref()?;
        let index = self.container_selected?;
        let item = state.items.get(index)?;
        let primary_id = item.primary_object()?;
        let instances = match command {
            ObjectMenuCommand::TakeAll => item.instances.clone(),
            _ => vec![primary_id],
        };
        if instances.is_empty() {
            return None;
        }
        Some(ObjectMenuAction::Execute {
            command,
            selection: ObjectMenuSelection {
                crew_id: self.crew_id,
                primary_id,
                instances,
                definition_id: item.definition_id.clone(),
                label: item.label.clone(),
                source_container: Some(state.id),
            },
        })
    }

    fn context_action(&self) -> Option<ObjectMenuAction> {
        let index = self.context_selected?;
        let item = self.context.get(index)?;
        Some(ObjectMenuAction::Context {
            selection: item.selection(self.crew_id),
        })
    }

    fn build_action(&self, amount: u32) -> Option<ObjectMenuAction> {
        if amount == 0 {
            return None;
        }
        let index = self.build_selected?;
        let item = self.build.get(index)?;
        let available = item.available();
        if available == 0 {
            return None;
        }
        Some(ObjectMenuAction::Build {
            selection: BuildMenuSelection {
                crew_id: self.crew_id,
                owner: self.owner,
                definition_id: item.definition_id.clone(),
                label: item.label.clone(),
            },
            amount: amount.min(available),
        })
    }

    fn current_selection_mut(&mut self) -> (&mut Option<usize>, usize) {
        match self.mode {
            MenuMode::Inventory => (&mut self.inventory_selected, self.inventory.len()),
            MenuMode::Container => {
                let len = self
                    .container
                    .as_ref()
                    .map(|state| state.items.len())
                    .unwrap_or(0);
                (&mut self.container_selected, len)
            }
            MenuMode::Context => (&mut self.context_selected, self.context.len()),
            MenuMode::Build => (&mut self.build_selected, self.build.len()),
        }
    }

    fn ensure_valid_mode(&mut self) {
        if !self.mode_available(self.mode) {
            if let Some(mode) = self.available_modes().first().copied() {
                self.mode = mode;
            } else {
                self.mode = MenuMode::Inventory;
            }
        }
        self.ensure_selection_for_mode();
    }

    fn mode_available(&self, mode: MenuMode) -> bool {
        match mode {
            MenuMode::Inventory => !self.inventory.is_empty() || self.inventory_known_empty,
            MenuMode::Container => self
                .container
                .as_ref()
                .map(|state| !state.items.is_empty() || state.known_empty)
                .unwrap_or(false),
            MenuMode::Context => !self.context.is_empty(),
            MenuMode::Build => !self.build.is_empty(),
        }
    }

    fn available_modes(&self) -> Vec<MenuMode> {
        let mut modes = Vec::new();
        if self.mode_available(MenuMode::Inventory) {
            modes.push(MenuMode::Inventory);
        }
        if self.mode_available(MenuMode::Container) {
            modes.push(MenuMode::Container);
        }
        if self.mode_available(MenuMode::Context) {
            modes.push(MenuMode::Context);
        }
        if self.mode_available(MenuMode::Build) {
            modes.push(MenuMode::Build);
        }
        if modes.is_empty() {
            modes.push(MenuMode::Inventory);
        }
        modes
    }

    fn ensure_selection_for_mode(&mut self) {
        match self.mode {
            MenuMode::Inventory => {
                if self.inventory_selected.is_none() && !self.inventory.is_empty() {
                    self.inventory_selected = Some(0);
                }
            }
            MenuMode::Container => {
                if let Some(state) = self.container.as_ref() {
                    if self.container_selected.is_none() && !state.items.is_empty() {
                        self.container_selected = Some(0);
                    }
                }
            }
            MenuMode::Context => {
                if self.context_selected.is_none() && !self.context.is_empty() {
                    self.context_selected = Some(0);
                }
            }
            MenuMode::Build => {
                if self.build_selected.is_none() && !self.build.is_empty() {
                    self.build_selected = Some(0);
                }
            }
        }
    }

    fn step_mode(&mut self, delta: i32, wrap: bool) -> bool {
        let modes = self.available_modes();
        if modes.len() <= 1 {
            return false;
        }
        let current_index = modes
            .iter()
            .position(|mode| *mode == self.mode)
            .unwrap_or(0) as i32;
        let len = modes.len() as i32;
        let next_index = if wrap {
            (current_index + delta).rem_euclid(len) as usize
        } else {
            let candidate = current_index + delta;
            if candidate < 0 || candidate >= len {
                return false;
            }
            candidate as usize
        };
        if modes[next_index] == self.mode {
            return false;
        }
        self.mode = modes[next_index];
        self.ensure_selection_for_mode();
        true
    }
}

fn clamp_selection(selection: &mut Option<usize>, len: usize) {
    if len == 0 {
        *selection = None;
    } else if let Some(index) = selection {
        if *index >= len {
            *selection = Some(len - 1);
        }
    }
}

fn collect_inventory(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    crew: &clonk_engine::ObjectSnapshot,
) -> Vec<ObjectMenuItem> {
    if crew.contents.is_empty() {
        return Vec::new();
    }
    collect_contents(engine, snapshot, &crew.contents)
}

fn collect_container_items(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    container: &clonk_engine::ObjectSnapshot,
) -> Vec<ObjectMenuItem> {
    if container.contents.is_empty() {
        return Vec::new();
    }
    collect_contents(engine, snapshot, &container.contents)
}

fn collect_contents(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    contents: &[ObjectId],
) -> Vec<ObjectMenuItem> {
    let mut order: Vec<ObjectMenuItem> = Vec::new();
    let mut lookup: HashMap<String, usize> = HashMap::new();

    for child_id in contents {
        let child = match snapshot.object(*child_id) {
            Some(child) => child,
            None => continue,
        };
        let name = engine
            .definition_name(&child.definition_id)
            .unwrap_or(&child.definition_id);
        let description = build_definition_summary(engine, &child.definition_id);
        let icon = definition_icon(engine, &child.definition_id);
        if let Some(index) = lookup.get(&child.definition_id).copied() {
            if let Some(entry) = order.get_mut(index) {
                entry.push_instance(child.id);
            }
        } else {
            let index = order.len();
            let entry =
                ObjectMenuItem::new(name, &child.definition_id, description, icon, child.id);
            order.push(entry);
            lookup.insert(child.definition_id.clone(), index);
        }
    }
    order
}

fn collect_context_items(
    engine: &mut Engine,
    crew: &clonk_engine::ObjectSnapshot,
) -> Vec<ContextMenuItem> {
    match engine.context_menu_entries(crew.id) {
        Ok(entries) => entries.into_iter().map(ContextMenuItem::new).collect(),
        Err(err) => {
            tracing::warn!(object = ?crew.id, error = ?err, "failed to build context menu");
            Vec::new()
        }
    }
}

fn collect_build_items(engine: &Engine, crew: &clonk_engine::ObjectSnapshot) -> Vec<BuildMenuItem> {
    if crew.owner == OWNER_NONE {
        return Vec::new();
    }
    let Some(player) = engine.player(crew.owner) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for (definition_id, count) in player.home_base_material() {
        if *count == 0 {
            continue;
        }
        let label = engine
            .definition_name(definition_id)
            .unwrap_or(definition_id)
            .to_string();
        let description = build_definition_summary(engine, definition_id);
        let icon = definition_icon(engine, definition_id);
        entries.push(BuildMenuItem {
            definition_id: definition_id.clone(),
            label,
            description,
            available: *count,
            icon,
        });
    }
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries
}

fn definition_icon(engine: &Engine, definition_id: &str) -> Option<ImageData> {
    engine
        .definition_picture_image(definition_id)
        .map(definition_menu_picture)
}

fn build_definition_summary(engine: &Engine, definition_id: &str) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(value) = engine.definition_value(definition_id) {
        if value > 0 {
            parts.push(format!("Value {value}"));
        }
    }
    if let Some(mass) = engine.definition_mass(definition_id) {
        if mass > 0 {
            parts.push(format!("Mass {mass}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" • "))
    }
}

fn draw_menu_icon(surface: &mut Surface, rect: Rect, icon: &ImageData, gamma: Option<&GammaRamp>) {
    clonk_frontend::classic_gui::draw_facet_nearest(
        surface,
        icon,
        Rect::new(0, 0, icon.width(), icon.height()),
        rect,
        gamma,
    );
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    clonk_frontend::draw_color_rect(surface, rect, color, gamma);
}

fn pack_engine_color(color: Color) -> u32 {
    (u32::from(255 - color.a) << 24)
        | (u32::from(color.r) << 16)
        | (u32::from(color.g) << 8)
        | u32::from(color.b)
}

/// `CStdDDraw::DrawFrameDw` (StdDDraw2.cpp:1181-1187): the directed line loop
/// covers every corner exactly once. Verified against a real C++ GL capture
/// (Drachenfels tooltip frame, 2026-07-21): the former full-length strips
/// double-blended the corners, which no C++ frame does.
fn draw_border(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    clonk_frontend::classic_gui::draw_engine_frame(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.width as i32 - 1,
        rect.y + rect.height as i32 - 1,
        pack_engine_color(color),
        gamma,
    );
}

/// `CStdDDraw::DrawFrame` (StdDDraw2.cpp:1173-1179) as reached from
/// `C4Menu::DrawFrame` (C4Menu.cpp:932-935): two horizontals plus two
/// verticals whose shared excluded endpoint leaves the bottom-right corner
/// unpainted on render targets.
fn draw_hv_border(surface: &mut Surface, rect: Rect, color: Color, gamma: Option<&GammaRamp>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    clonk_frontend::classic_gui::draw_engine_frame_hv(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.width as i32 - 1,
        rect.y + rect.height as i32 - 1,
        pack_engine_color(color),
        gamma,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_app_core::pictures::apply_default_menu_owner_color;
    use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
    use clonk_engine::ComponentList;
    use clonk_engine::{
        CommandStackSnapshot, Definition, Engine, JoinPlayerConfig, MovementProfile,
        ObjectSnapshot, ObjectStatus, ObjectUpdate, PlayerConfig, Scenario, ScenarioError,
        SpawnConfig, Vector2,
    };
    use clonk_resources::{Group, MaterialLibrary};
    use std::collections::HashMap;
    use std::path::PathBuf;

    struct RepositoryContentResolver {
        root: PathBuf,
    }

    impl LegacyDefinitionResolver for RepositoryContentResolver {
        fn resolve_definition_groups(
            &self,
            _scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            Group::open(self.root.join(identifier.replace('\\', "/")))
                .map(|group| vec![group])
                .map_err(ScenarioError::Resources)
        }
    }

    // C++ GL capture oracle (2026-07-21, Drachenfels choice-menu
    // tooltip at (942,580) 182x26 in Screenshot001.png): every frame pixel
    // over the opaque #F1EA78 fill — corners included — reads (121,117,60):
    // one DrawLineDw blend of 0x7f000000 with GL round-to-nearest and the
    // gamma black floor. Full strips double-blended corners to (61,59,30).
    #[test]
    fn classic_tooltip_border_corners_blend_once_per_cpp_capture() {
        let gamma = GammaRamp::from_control_points([0x000000, 0x808080, 0xffffff]);
        let mut surface = Surface::new(16, 12, PixelFormat::Rgba8888);
        surface.fill(CLASSIC_TOOLTIP_BG_COLOR);
        draw_border(
            &mut surface,
            Rect::new(2, 2, 12, 8),
            CLASSIC_TOOLTIP_FRAME_COLOR,
            Some(&gamma),
        );
        let frame = Some(Color::opaque(121, 117, 60));
        for (x, y) in [(2, 2), (13, 2), (2, 9), (13, 9)] {
            assert_eq!(
                surface.get_pixel(x, y),
                frame,
                "corner ({x},{y}) must blend once like the C++ capture"
            );
        }
        assert_eq!(surface.get_pixel(7, 2), frame);
        assert_eq!(surface.get_pixel(2, 5), frame);
    }

    // C++ GL capture oracle (2026-07-21, Drachenfels extra-bar
    // divider (1032,647)-(1208,662) in Screenshot001.png): three corners and
    // the edges paint (68,1,1) while the bottom-right corner stays at the
    // (1,1,1) background — both `CStdDDraw::DrawFrame` lines exclude it.
    #[test]
    fn classic_divider_skips_bottom_right_corner_per_cpp_capture() {
        let gamma = GammaRamp::from_control_points([0x000000, 0x808080, 0xffffff]);
        let mut surface = Surface::new(16, 12, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(1, 1, 1));
        draw_hv_border(
            &mut surface,
            Rect::new(2, 2, 12, 8),
            CLASSIC_EXTRA_FRAME_COLOR,
            Some(&gamma),
        );
        let divider = Some(Color::opaque(68, 1, 1));
        assert_eq!(surface.get_pixel(2, 2), divider);
        assert_eq!(surface.get_pixel(13, 2), divider);
        assert_eq!(surface.get_pixel(2, 9), divider);
        assert_eq!(
            surface.get_pixel(13, 9),
            Some(Color::opaque(1, 1, 1)),
            "CStdDDraw::DrawFrame never rasterizes the shared bottom-right endpoint"
        );
        assert_eq!(surface.get_pixel(7, 9), divider);
        assert_eq!(surface.get_pixel(13, 5), divider);
    }

    use clonk_app_core::test_support::repository_root;

    fn solid_image(width: u32, height: u32, color: Color) -> ImageData {
        ImageData::new(
            width,
            height,
            [color.r, color.g, color.b, color.a].repeat((width * height) as usize),
        )
    }

    fn solid_rgba_image(width: u32, height: u32, rgba: [u8; 4]) -> ImageData {
        ImageData::new(width, height, rgba.repeat((width * height) as usize))
    }

    fn script_menu_fixture(
        identifier: &str,
        name: &str,
        script: &str,
    ) -> clonk_engine::ObjectMenuState {
        let mut engine = Engine::new();
        engine
            .register_script_definition(identifier, name, script)
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new(identifier))
            .expect("menu object spawns");
        engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("Initialize created its menu")
    }

    fn engine_script_menu_fixture(style: i32, item_count: usize) -> clonk_engine::ObjectMenuState {
        let script = format!(
            r#"
            func Initialize()
            {{
                CreateMenu(MENU, this(), this(), 0, "Scrollable", 0, {style});
                AddMenuItem("Row 0", "Choose()", MENU, this());
            }}
            "#
        );
        let mut menu = script_menu_fixture("MENU", "Menu", &script);
        let template = menu.items[0].clone();
        menu.items = (0..item_count)
            .map(|index| {
                let mut item = template.clone();
                item.caption = format!("Row {index}");
                item
            })
            .collect();
        menu.selection = 0;
        menu.columns = 1;
        menu
    }

    /// `planet/System.c4g/MenuRangeRow.c` greys the step a range cannot take
    /// instead of dropping it, and relies on colour markup costing no width so
    /// a Context row never resizes as its value moves. Pin that property here,
    /// where the width is actually measured (C4Menu.cpp:650-664 through
    /// CStdFont::GetTextExtent's markup skip, StdFont.cpp:571-601).
    #[test]
    fn colour_markup_in_a_context_caption_costs_no_row_width() {
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let images = HashMap::new();
        let width_of = |caption: &str| {
            let mut menu = engine_script_menu_fixture(1, 1);
            menu.items[0].caption = caption.to_string();
            engine_script_menu_layout_with_images(
                Rect::new(0, 0, 640, 480),
                &font,
                &menu,
                false,
                &images,
                None,
            )
            .item_width
        };

        let plain = width_of("Metal - 4 (+1/-1)");
        for greyed in [
            "Metal - 4 (+1/<c 888888>-1</c>)",
            "Metal - 4 (<c 888888>+1</c>/-1)",
            "Metal - 4 (<c 888888>+1</c>/<c 888888>-1</c>)",
        ] {
            assert_eq!(
                width_of(greyed),
                plain,
                "greying a step must not move the row: {greyed}"
            );
        }
    }

    fn surface_rect_contains_color(surface: &Surface, rect: Rect, color: Color) -> bool {
        (rect.y..rect.y + rect.height as i32).any(|y| {
            (rect.x..rect.x + rect.width as i32)
                .any(|x| surface.get_pixel(x as u32, y as u32) == Some(color))
        })
    }

    fn load_repository_dragon_rock() -> (Engine, i32) {
        let repository = repository_root();
        let content = repository.join("content");
        let scenario_path = content.join("Fantasy.c4f/Drachenfels.c4s");
        let scenario = Scenario::load_from_path_with(
            &scenario_path,
            &RepositoryContentResolver {
                root: content.clone(),
            },
        )
        .unwrap_or_else(|error| panic!("scenario `{}` loads: {error}", scenario_path.display()));
        let material_group =
            Group::open(content.join("Material.c4g")).expect("installed Material.c4g opens");
        let materials =
            MaterialLibrary::from_group(&material_group).expect("installed materials load");
        let system_group =
            Group::open(repository.join("planet/System.c4g")).expect("System.c4g opens");
        let system_scripts = load_system_scripts(&system_group).expect("system scripts load");
        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&materials);
        engine.install_global_scripts(&system_scripts);
        engine.set_standard_names(
            system_group
                .read_file("Names.txt")
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        );
        scenario.apply(&mut engine).expect("Dragon Rock applies");
        let owner = engine
            .join_player(JoinPlayerConfig {
                name: "Dragon Rock menu parity".to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                // Drachenfels has an active custom team list. A C++ player
                // with no selected team stops before ScenarioInit; this menu
                // fixture models the lobby's Helden selection (team 1).
                team: Some(1),
                color_dw: 0xff_00_00,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("Dragon Rock player joins")
            .initialized()
            .expect("Dragon Rock team selection initializes the player")
            .number;
        (engine, owner)
    }

    fn make_object(id: u64, definition: &str) -> ObjectSnapshot {
        ObjectSnapshot {
            id: ObjectId::new(id),
            definition_id: definition.to_string(),
            custom_name: None,
            position: Vector2::new(0, 0),
            velocity: Vector2::new(0, 0),
            rotation: 0,
            energy: 100,
            need_energy: false,
            construction: clonk_engine::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
            action: Default::default(),
            direction: Default::default(),
            command_direction: Default::default(),
            action_procedure: None,
            effects: Vec::new(),
            vertices: Vec::new(),
            current_shape: None,
            current_fire_top: None,
            contact_density: 50,
            own_vertices: None,
            vertex_contacts: Vec::new(),
            solid_mask_override: None,
            container: None,
            layer: None,
            visibility: 0,
            blit_mode: 0,
            color: 0,
            color_modulation: 0,
            picture_rect: Default::default(),
            contents: Vec::new(),
            components: ComponentList::new(),
            component_order: Vec::new(),
            status: ObjectStatus::Normal,
            owner: 1,
            controller: 1,
            category: 0,
            crew_member: false,
            plr_view_range: 0,
            selected: false,
            alive: true,
            base_graphics: None,
            graphics_overlays: Vec::new(),
            draw_transform: None,
            command_queue: Vec::new(),
            command_stack: CommandStackSnapshot::default(),
            local_vars: HashMap::new(),
            in_liquid: false,
            mobile: false,
            ocf: 0,
            timer: 0,
            own_mass: 0,
            on_fire: false,
            fire_phase: 0,
            fire_caused_by: -1,
            info_physical: None,
            temporary_physical: None,
            physical_changes: Vec::new(),
            breath: 0,
            last_energy_loss_cause: -1,
            base: -1,
            fixed_position: None,
            fixed_velocity: None,
            rotation_velocity: None,
            fixed_rotation: None,
        }
    }

    fn make_snapshot(
        mut crew: ObjectSnapshot,
        contents: Vec<ObjectSnapshot>,
        players: Vec<clonk_engine::PlayerState>,
    ) -> SimulationSnapshot {
        let mut objects = Vec::new();
        let mut crew_contents = Vec::new();
        for object in contents {
            crew_contents.push(object.id);
            objects.push(object);
        }
        crew.contents = crew_contents;
        crew.crew_member = true;
        objects.insert(0, crew);
        SimulationSnapshot {
            objects,
            players,
            rng: clonk_engine::LcgRng::seed_from_u64(42),
            ..Default::default()
        }
    }

    #[test]
    fn inventory_groups_by_definition() {
        let mut engine = Engine::new();
        let crew = make_object(1, "Clonk");
        let contents = vec![
            make_object(2, "Shovel"),
            make_object(3, "Shovel"),
            make_object(4, "Hammer"),
        ];
        let snapshot = make_snapshot(crew.clone(), contents, Vec::new());
        let mut menu =
            ObjectMenuState::new(&mut engine, &snapshot, crew.id).expect("menu should exist");
        assert_eq!(menu.inventory.len(), 2);
        assert_eq!(menu.inventory[0].definition_id, "Shovel");
        assert_eq!(menu.inventory[0].count(), 2);
        assert_eq!(menu.inventory[1].definition_id, "Hammer");
        assert_eq!(menu.inventory[1].count(), 1);

        // Simulate removing an item and refreshing.
        let mut snapshot_updated = snapshot.clone();
        if let Some(crew_obj) = snapshot_updated.objects.get_mut(0) {
            crew_obj.contents.pop();
        }
        assert!(menu.refresh(&mut engine, &snapshot_updated));
        assert_eq!(menu.inventory.len(), 1);
        assert_eq!(menu.inventory[0].count(), 2);
    }

    #[test]
    fn menu_enter_all_emits_drop_action() {
        let mut engine = Engine::new();
        let crew = make_object(1, "Clonk");
        let contents = vec![
            make_object(2, "Shovel"),
            make_object(3, "Shovel"),
            make_object(4, "Hammer"),
        ];
        let snapshot = make_snapshot(crew.clone(), contents, Vec::new());
        let mut menu =
            ObjectMenuState::new(&mut engine, &snapshot, crew.id).expect("menu should exist");
        let action = menu
            .handle_command(ControlCommand::MenuEnterAll, CommandKind::Press)
            .expect("drop action");
        match action {
            ObjectMenuAction::Execute { command, selection } => {
                assert_eq!(command, ObjectMenuCommand::DropAll);
                assert_eq!(selection.crew_id, crew.id);
                assert_eq!(selection.label, "Shovel");
                assert_eq!(selection.definition_id, "Shovel");
                assert_eq!(selection.count(), 2);
                assert_eq!(selection.instances.len(), 2);
                assert_eq!(selection.primary_id, ObjectId::new(2));
                assert!(selection.source_container.is_none());
            }
            _ => panic!("expected execute action"),
        }
    }

    #[test]
    fn container_mode_emits_take_actions() {
        let mut engine = Engine::new();
        let crew = make_object(1, "Clonk");
        let mut container = make_object(2, "Chest");
        let item_a = make_object(3, "Shovel");
        let item_b = make_object(4, "Shovel");

        container.contents = vec![item_a.id, item_b.id];

        let mut snapshot = make_snapshot(crew.clone(), Vec::new(), Vec::new());
        snapshot.objects.push(container.clone());
        snapshot.objects.push(ObjectSnapshot {
            container: Some(container.id),
            ..item_a.clone()
        });
        snapshot.objects.push(ObjectSnapshot {
            container: Some(container.id),
            ..item_b.clone()
        });

        let mut menu =
            ObjectMenuState::new(&mut engine, &snapshot, crew.id).expect("menu should exist");
        assert!(menu.focus_container_mode(&mut engine, &snapshot, container.id));
        assert_eq!(menu.mode, MenuMode::Container);
        assert!(menu.available_modes().contains(&MenuMode::Container));

        let take_action = menu
            .handle_command(ControlCommand::MenuSelect, CommandKind::Press)
            .expect("take action");
        match take_action {
            ObjectMenuAction::Execute { command, selection } => {
                assert_eq!(command, ObjectMenuCommand::Take);
                assert_eq!(selection.source_container, Some(container.id));
                assert_eq!(selection.instances.len(), 1);
                assert_eq!(selection.definition_id, "Shovel");
            }
            other => panic!("unexpected action: {:?}", other),
        }

        let take_all = menu
            .handle_command(ControlCommand::MenuEnterAll, CommandKind::Press)
            .expect("take all action");
        match take_all {
            ObjectMenuAction::Execute { command, selection } => {
                assert_eq!(command, ObjectMenuCommand::TakeAll);
                assert_eq!(selection.source_container, Some(container.id));
                assert_eq!(selection.instances.len(), 2);
            }
            other => panic!("unexpected action: {:?}", other),
        }
    }

    #[test]
    fn inventory_item_uses_definition_metadata() {
        let mut engine = Engine::new();
        let mut shovel =
            Definition::from_script("Shovel", "Shovel", "func Initialize() {}").unwrap();
        shovel.set_movement_profile(MovementProfile::default());
        shovel.set_value(75);
        shovel.set_mass(18);
        engine
            .register_definition(shovel)
            .expect("register shovel definition");

        let crew = make_object(1, "Clonk");
        let contents = vec![make_object(2, "Shovel")];
        let snapshot = make_snapshot(crew.clone(), contents, Vec::new());
        let menu =
            ObjectMenuState::new(&mut engine, &snapshot, crew.id).expect("menu should exist");
        assert_eq!(menu.inventory.len(), 1);
        assert_eq!(
            menu.inventory[0].description.as_deref(),
            Some("Value 75 • Mass 18")
        );
    }

    #[test]
    fn context_menu_generates_actions() {
        let script = r#"
        #strict 3
        global func Initialize(state, random) { return nil; }
        global func MenuEntries(state)
        {
            return [ { label = "Wave", callback = "MenuWave", description = "Greet nearby" } ];
        }
        global func MenuWave(state) { return true; }
        "#;

        let mut engine = Engine::with_seed(0);
        let mut definition = Definition::from_script("Clonk", "Clonk", script).unwrap();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("register definition");

        let crew_id = engine
            .spawn_object(
                SpawnConfig::new("Clonk")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn crew");
        let snapshot = engine.snapshot();
        let mut menu =
            ObjectMenuState::new(&mut engine, &snapshot, crew_id).expect("menu should exist");

        assert_eq!(menu.mode, MenuMode::Context);
        assert_eq!(menu.context.len(), 1);
        assert_eq!(
            menu.available_modes(),
            vec![MenuMode::Inventory, MenuMode::Context]
        );

        let action = menu
            .handle_command(ControlCommand::MenuSelect, CommandKind::Press)
            .expect("context action");
        match action {
            ObjectMenuAction::Context { selection } => {
                assert_eq!(selection.function, "MenuWave");
                assert_eq!(selection.label, "Wave");
                assert_eq!(selection.description.as_deref(), Some("Greet nearby"));
            }
            other => panic!("unexpected action: {:?}", other),
        }
    }

    #[test]
    fn context_menu_labels_decode_native_bytes_for_presentation() {
        let raw_label = clonk_script::c4_string_from_bytes(b"Wav\xe9");
        let raw_description = clonk_script::c4_string_from_bytes(b"Gr\xfc\xdfe");
        let item = ContextMenuItem::new(ContextMenuEntry {
            function: "MenuWave".into(),
            label: raw_label.clone(),
            description: Some(raw_description.clone()),
        });

        assert_eq!(item.label(), "Wav\u{e9}");
        assert_eq!(item.description(), Some("Gr\u{fc}\u{df}e"));
        assert_eq!(clonk_script::c4_string_bytes(&raw_label), b"Wav\xe9");
        assert_eq!(
            clonk_script::c4_string_bytes(&raw_description),
            b"Gr\xfc\xdfe"
        );
    }

    #[test]
    fn build_menu_lists_home_base_supplies() {
        let mut engine = Engine::new();
        engine
            .register_player(
                PlayerConfig::new(1, "Player")
                    .with_home_base_material(HashMap::from([("Hammer".to_string(), 3_u32)])),
            )
            .expect("register player");
        let mut hammer =
            Definition::from_script("Hammer", "Hammer", "func Initialize() {}").unwrap();
        hammer.set_movement_profile(MovementProfile::default());
        engine.register_definition(hammer).expect("register hammer");

        let mut crew = make_object(1, "Clonk");
        crew.owner = 1;
        let snapshot = make_snapshot(crew.clone(), Vec::new(), Vec::new());
        let mut menu =
            ObjectMenuState::new(&mut engine, &snapshot, crew.id).expect("menu should exist");
        assert_eq!(menu.build.len(), 1);
        assert_eq!(menu.build[0].definition_id, "Hammer");
        assert_eq!(menu.build[0].available(), 3);

        // Switch to build mode.
        assert!(menu
            .handle_command(ControlCommand::MenuRight, CommandKind::Press)
            .is_none());
        assert_eq!(menu.mode, MenuMode::Build);
        let action = menu
            .handle_command(ControlCommand::MenuSelect, CommandKind::Press)
            .expect("build action");
        match action {
            ObjectMenuAction::Build { selection, amount } => {
                assert_eq!(selection.definition_id, "Hammer");
                assert_eq!(selection.owner, 1);
                assert_eq!(selection.crew_id, crew.id);
                assert_eq!(amount, 1);
            }
            other => panic!("unexpected action: {:?}", other),
        }
    }

    // C4GuiContainers.cpp:477-623 — an overflowing engine menu shows the bar in
    // the column its layout already reserves, and routes pointer input through
    // the shared C4GUI::ScrollBar model.
    #[test]
    fn engine_menu_overflow_scrollbar_routes_arrows_track_and_thumb() {
        use crate::scrollbar::ScrollbarHit;

        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let images = HashMap::new();
        let mut menu = engine_script_menu_fixture(1, 12);
        menu.columns = 3;
        // Two visible rows of four leaves the rest scrollable.
        let layout = engine_script_menu_layout_with_presentation(
            Rect::new(0, 0, 640, 480),
            &font,
            &menu,
            false,
            &images,
            Some((0, 0)),
            0,
            false,
            Some(2),
        );

        let bar = layout
            .scrollbar
            .expect("an overflowing menu reserves the bar");
        assert_eq!(
            bar.width as i32,
            crate::scrollbar::SCROLLBAR_EXTENT,
            "the drawn bar must fill the reserved column"
        );
        assert_eq!(
            bar.x,
            layout.client_x + layout.columns * layout.item_width,
            "the bar sits in the column the layout reserved, not over the items"
        );
        assert!(
            layout.max_scroll_y > 0,
            "the fixture must actually overflow"
        );

        let centre = bar.x + bar.width as i32 / 2;
        let top = bar.y;
        let bottom = bar.y + bar.height as i32 - 1;
        assert_eq!(
            engine_menu_scrollbar_hit(&layout, (centre, top)),
            Some(ScrollbarHit::ScrollUp)
        );
        assert_eq!(
            engine_menu_scrollbar_hit(&layout, (centre, bottom)),
            Some(ScrollbarHit::ScrollDown)
        );
        // Outside the bar the menu items keep the pointer.
        assert_eq!(engine_menu_scrollbar_hit(&layout, (bar.x - 1, top)), None);

        // This menu's bar is two 16px rows tall, so the two arrows consume it
        // entirely and the pin has nowhere to travel — `rect.h - 3 * extent` is
        // negative. C++ behaves the same way for a bar this short, so a drag
        // selects nothing; the shared model's own test covers a bar with travel.
        assert_eq!(crate::scrollbar::pin_travel(bar.height as i32), 0);
        assert_eq!(engine_menu_scroll_from_pointer(&layout, bottom), Some(0));
        assert_eq!(engine_menu_scroll_from_pointer(&layout, top), Some(0));

        // A menu that fits shows no bar at all and swallows no input.
        let fits = engine_script_menu_layout_with_presentation(
            Rect::new(0, 0, 640, 480),
            &font,
            &menu,
            false,
            &images,
            Some((0, 0)),
            0,
            false,
            None,
        );
        assert_eq!(fits.max_scroll_y, 0);
        assert!(fits.scrollbar.is_none());
        assert_eq!(engine_menu_scrollbar_hit(&fits, (centre, top)), None);
    }

    // `C4Menu::SetSize` assigns Lines and reruns only `InitSize`, which applies
    // no viewport clamp, while `InitLocation` recomputes Lines from the item
    // count whenever it runs (C4Menu.cpp:635-640,713-721,755-780).
    #[test]
    fn set_menu_size_rows_control_visible_grid_and_scrollbar() {
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let images = HashMap::new();
        let area = Rect::new(0, 0, 640, 480);
        let mut menu = engine_script_menu_fixture(1, 12);
        menu.columns = 3;

        let derived = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            Some((0, 0)),
            0,
            false,
            None,
        );
        assert_eq!(derived.columns, 3);
        assert_eq!(derived.lines, 4, "12 items over 3 columns is four rows");

        // An explicit two-row grid shrinks the client, leaves the remaining
        // rows scrollable and reserves the native scrollbar.
        let explicit = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            Some((0, 0)),
            0,
            false,
            Some(2),
        );
        assert_eq!(explicit.lines, 2);
        assert_eq!(explicit.columns, 3);
        assert_eq!(
            explicit.client.height,
            (2 * explicit.item_height).max(0) as u32
        );
        assert_eq!(
            explicit.bounds.height + 2 * explicit.item_height as u32,
            derived.bounds.height
        );
        assert_eq!(explicit.max_scroll_y, 2 * explicit.item_height);
        assert_eq!(derived.max_scroll_y, 0);
        assert_eq!(explicit.visible, 6);

        // Item hit geometry follows the same grid: row three is scrolled out.
        assert!(explicit.item_rect(0).is_some());
        assert!(explicit.item_rect(5).is_some());
        assert!(explicit.item_rect(6).is_none());

        // InitSize applies no viewport clamp, so an oversized explicit grid is
        // used as given while a derived one is capped.
        let tall = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            Some((0, 0)),
            0,
            false,
            Some(40),
        );
        assert_eq!(tall.lines, 40);
        let many = engine_script_menu_fixture(1, 400);
        let capped = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &many,
            false,
            &images,
            Some((0, 0)),
            0,
            false,
            None,
        );
        assert_eq!(capped.lines, ((480 - 100) / capped.item_height).max(1));

        // Zero keeps the derived axis (SetSize ignores a zero argument).
        let zero = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            Some((0, 0)),
            0,
            false,
            Some(0),
        );
        assert_eq!(zero.lines, derived.lines);
    }

    #[test]
    fn engine_script_menu_command_strip_uses_cpp_menu_symbol_height() {
        // DrawMenuControls reserves C4MN_SymbolSize (16px), not the
        // unrelated 35px C4SymbolSize used by normal-menu items
        // (src/C4Menu.h:32-35,262; src/C4Menu.cpp:843-880).
        assert_eq!(CLASSIC_COMMAND_HEIGHT, 16);
    }

    #[test]
    fn script_menu_scroll_range_is_minimal_pixel_persistent_and_column_aware() {
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let images = HashMap::new();
        let area = Rect::new(0, 0, 320, 200);
        let mut menu = engine_script_menu_fixture(1, 20);
        menu.columns = 2;

        let base = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            Some((-25, -9)),
            0,
            false,
            None,
        );
        assert_eq!((base.bounds.x, base.bounds.y), (-25, -9));
        assert_eq!(base.lines, 6);
        assert_eq!(base.max_scroll_y, 4 * base.item_height);

        let retained_scroll = base.item_height + 5;
        menu.selection = 2 * menu.columns;
        let retained = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            Some((-25, -9)),
            retained_scroll,
            true,
            None,
        );
        assert_eq!(retained.scroll_y, retained_scroll);
        assert_eq!(retained.first_index, 2);
        assert_eq!(retained.visible, 2 * (retained.lines as usize + 1));

        menu.selection = menu.columns;
        let above = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            None,
            retained_scroll,
            true,
            None,
        );
        assert_eq!(above.scroll_y, base.item_height);

        menu.selection = (base.lines - 1) * menu.columns;
        let bottom_equality = engine_script_menu_layout_with_presentation(
            area, &font, &menu, false, &images, None, 0, true, None,
        );
        assert_eq!(bottom_equality.scroll_y, 0);

        menu.selection = (base.lines + 1) * menu.columns + 1;
        let below = engine_script_menu_layout_with_presentation(
            area, &font, &menu, false, &images, None, 0, true, None,
        );
        assert_eq!(below.scroll_y, 2 * base.item_height);

        let low = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            None,
            i32::MIN,
            false,
            None,
        );
        let high = engine_script_menu_layout_with_presentation(
            area,
            &font,
            &menu,
            false,
            &images,
            None,
            i32::MAX,
            false,
            None,
        );
        assert_eq!(low.scroll_y, 0);
        assert_eq!(high.scroll_y, high.max_scroll_y);

        let one_line_area = Rect::new(0, 0, 320, 100);
        menu.selection = i32::try_from(menu.items.len() - 1).expect("small fixture");
        let one_line = engine_script_menu_layout_with_presentation(
            one_line_area,
            &font,
            &menu,
            false,
            &images,
            None,
            base.item_height + 3,
            true,
            None,
        );
        assert_eq!(one_line.lines, 1);
        assert_eq!(one_line.scroll_y, base.item_height + 3);
    }

    #[test]
    fn partial_scroll_clips_render_and_item_hits_below_title() {
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let images = HashMap::new();
        let area = Rect::new(0, 0, 320, 200);
        let menu = engine_script_menu_fixture(1, 20);
        let location = Some((40, 30));
        let layout = engine_script_menu_layout_with_presentation(
            area, &font, &menu, false, &images, location, 5, false, None,
        );

        assert_eq!(layout.scroll_y, 5);
        assert_eq!(layout.first_index, 0);
        assert_eq!(layout.visible, layout.lines as usize + 1);
        let first = layout.item_rect(0).expect("partial first row intersects");
        let first_visible = layout
            .item_visible_rect(0)
            .expect("partial first row is clipped");
        assert_eq!(first.y, layout.client.y - 5);
        assert_eq!(first_visible.y, layout.client.y);
        assert_eq!(
            first_visible.height,
            u32::try_from(layout.item_height - 5).expect("positive visible height")
        );
        let last_partial = usize::try_from(layout.lines).expect("positive line count");
        assert_eq!(
            layout
                .item_visible_rect(last_partial)
                .expect("partial bottom row")
                .height,
            5
        );

        let hit = |point| {
            engine_script_menu_pointer_target_with_presentation(
                area,
                &font,
                &menu,
                &[],
                false,
                true,
                point,
                &images,
                location,
                5,
                None,
            )
        };
        assert_eq!(
            hit(GuiPoint::new(
                layout.client.x as f32 + 1.0,
                layout.client.y as f32 + 0.5,
            )),
            Some(EngineScriptMenuPointerTarget::Item(0))
        );
        assert_eq!(
            hit(GuiPoint::new(
                layout.client.x as f32 + 1.0,
                layout.client.y as f32 - 0.5,
            )),
            Some(EngineScriptMenuPointerTarget::Title),
            "the clipped-off part of row zero must not steal title hits"
        );
        assert_eq!(
            hit(GuiPoint::new(
                layout.client.x as f32 + 1.0,
                (layout.client.y + layout.client.height as i32) as f32 - 0.5,
            )),
            Some(EngineScriptMenuPointerTarget::Item(last_partial))
        );
        let close = layout.close_button_rect();
        assert_eq!(
            hit(GuiPoint::new(close.x as f32 + 1.0, close.y as f32 + 1.0,)),
            Some(EngineScriptMenuPointerTarget::Close),
            "close remains topmost over the title drag target"
        );

        let gfx = IngameMenuGraphics {
            menu_location: location,
            menu_scroll_y: 5,
            ..IngameMenuGraphics::default()
        };
        let mut surface = Surface::new(320, 200, clonk_graphics::PixelFormat::Rgba8888);
        let item_icons = vec![None; menu.items.len()];
        render_engine_script_menu(
            &mut surface,
            area,
            &font,
            &fallback,
            None,
            &menu,
            &gfx,
            None,
            &item_icons,
            &[],
            true,
            0,
        );
        assert_eq!(
            surface.get_pixel(layout.client.x as u32 + 1, layout.client.y as u32),
            Some(CLASSIC_SELECTION_COLOR)
        );
        assert_ne!(
            surface.get_pixel(
                layout.client.x as u32 + 1,
                u32::try_from(layout.client.y - 1).expect("fixture is on-screen"),
            ),
            Some(CLASSIC_SELECTION_COLOR),
            "selection fill is clipped at the client top"
        );
        assert_eq!(surface.clip(), None, "render restores the caller clip");
    }

    #[test]
    fn dialog_geometry_and_title_hits_follow_exact_presentation_location() {
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let images = HashMap::new();
        let area = Rect::new(0, 0, 640, 480);
        let menu = engine_script_menu_fixture(3, 2);
        let natural = engine_script_menu_presentation_geometry(
            area,
            &font,
            &menu,
            &[],
            false,
            &images,
            None,
            0,
            None,
        )
        .expect("dialog geometry");
        let location = Some((-17, -23));
        let moved = engine_script_menu_presentation_geometry(
            area,
            &font,
            &menu,
            &[],
            false,
            &images,
            location,
            i32::MAX,
            None,
        )
        .expect("moved dialog geometry");

        assert_eq!((moved.bounds.x, moved.bounds.y), (-17, -23));
        assert_eq!(moved.bounds.width, natural.bounds.width);
        assert_eq!(moved.bounds.height, natural.bounds.height);
        assert_eq!(moved.scroll_y, 0);
        assert_eq!(moved.max_scroll_y, 0);
        let title = moved.title.expect("caption creates a title");
        let client = moved.client.expect("dialog has a client");
        assert_eq!(title.x - moved.bounds.x, 0);
        assert_eq!(
            client.x - moved.bounds.x,
            natural.client.expect("natural client").x - natural.bounds.x
        );

        assert_eq!(
            engine_script_menu_pointer_target_with_presentation(
                area,
                &font,
                &menu,
                &[],
                false,
                true,
                GuiPoint::new(title.x as f32 + 1.0, title.y as f32 + 1.0),
                &images,
                location,
                i32::MAX,
                None,
            ),
            None,
            "external-dialog input is clipped to its owning viewport"
        );
        let close = Rect::new(
            moved.bounds.x + moved.bounds.width as i32 - 20,
            title.y + 4,
            16,
            16,
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_presentation(
                area,
                &font,
                &menu,
                &[],
                false,
                true,
                GuiPoint::new(close.x as f32 + 1.0, close.y as f32 + 1.0),
                &images,
                location,
                i32::MAX,
                None,
            ),
            None,
            "an off-viewport close button is not interactive"
        );

        let visible_location = Some((17, 23));
        let visible = engine_script_menu_presentation_geometry(
            area,
            &font,
            &menu,
            &[],
            false,
            &images,
            visible_location,
            i32::MAX,
            None,
        )
        .expect("visible moved dialog geometry");
        let visible_title = visible.title.expect("caption creates a title");
        assert_eq!(
            engine_script_menu_pointer_target_with_presentation(
                area,
                &font,
                &menu,
                &[],
                false,
                true,
                GuiPoint::new(visible_title.x as f32 + 1.0, visible_title.y as f32 + 1.0,),
                &images,
                visible_location,
                i32::MAX,
                None,
            ),
            Some(EngineScriptMenuPointerTarget::Title)
        );
        let visible_close = Rect::new(
            visible.bounds.x + visible.bounds.width as i32 - 20,
            visible_title.y + 4,
            16,
            16,
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_presentation(
                area,
                &font,
                &menu,
                &[],
                false,
                true,
                GuiPoint::new(visible_close.x as f32 + 1.0, visible_close.y as f32 + 1.0,),
                &images,
                visible_location,
                i32::MAX,
                None,
            ),
            Some(EngineScriptMenuPointerTarget::Close)
        );
    }

    #[test]
    fn dialog_hit_geometry_ignores_unresolved_item_picture() {
        // C4MenuItem::GetSymbolWidth reserves a Dialog symbol column only
        // when the resolved facet has a surface (C4Menu.cpp:138).
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let area = Rect::new(0, 0, 640, 480);
        let font_images = HashMap::new();
        let mut menu = engine_script_menu_fixture(3, 1);
        menu.items[0].image = clonk_engine::ObjectMenuImage::Definition;

        let mut differing_caption = None;
        for length in 1..=256 {
            menu.items[0].caption = "W".repeat(length);
            let drawn =
                dialog_script_menu_layout_with_images(area, &font, &menu, &[None], &font_images);
            let recipe =
                dialog_script_menu_layout_with_symbols(area, &font, &menu, &[true], &font_images);
            if drawn.bounds != recipe.bounds {
                differing_caption = Some((drawn, recipe));
                break;
            }
        }
        let (drawn, recipe) = differing_caption.expect("fixture crosses a symbol-wrap boundary");
        assert_ne!(drawn.bounds, recipe.bounds);

        let hit_geometry = engine_script_menu_presentation_geometry(
            area,
            &font,
            &menu,
            &[None],
            false,
            &font_images,
            None,
            0,
            None,
        )
        .expect("Dialog geometry");
        assert_eq!(
            hit_geometry.bounds, drawn.bounds,
            "input geometry must match drawing when the picture does not resolve"
        );
    }

    #[test]
    fn engine_script_context_menu_uses_free_location_and_cpp_clamping() {
        // C4Menu::InitMenu gives context style 1 one column
        // (src/C4Menu.cpp:359-365), then the same classic menu location,
        // size, drawing, and GUI element path handles its items
        // (src/C4Menu.cpp:642-880). C4Object::ActivateMenu selects that
        // style for C4MN_Context (src/C4Object.cpp:1961-1980).
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Context", 0, 1);
            AddMenuItem("Action", "Choose()", MENU, this());
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        assert_eq!(menu.style, 1);
        assert_eq!(menu.columns, 1);

        let font = clonk_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &hud_font, &menu, false);
        let item = layout.item_rect(0).expect("context item is visible");
        let item_height = hud_font.line_height().max(CLASSIC_COMMAND_HEIGHT);
        let item_width = (hud_font.text_width("Context") + item_height + 16)
            .max(hud_font.text_width("Action") + item_height)
            + 3;
        assert_eq!(
            (item.width, item.height),
            (item_width as u32, item_height as u32)
        );
        assert_eq!(
            layout.bounds.width,
            (item_width + 2 * CLASSIC_FRAME_WIDTH) as u32
        );

        let free_area = Rect::new(41, 27, 240, 160);
        let at_click = engine_script_menu_layout_with_free_anchor(
            free_area,
            &hud_font,
            &menu,
            false,
            &HashMap::new(),
            (free_area.x + 12, free_area.y + 18),
            0,
            false,
            None,
        );
        assert_eq!(
            (at_click.bounds.x, at_click.bounds.y),
            (free_area.x + 12, free_area.y + 18),
            "a fitting free menu keeps the viewport-local click as its outer top-left"
        );
        let above_left = engine_script_menu_layout_with_free_anchor(
            free_area,
            &hud_font,
            &menu,
            false,
            &HashMap::new(),
            (free_area.x - 100, free_area.y - 100),
            0,
            false,
            None,
        );
        assert_eq!(
            (above_left.bounds.x, above_left.bounds.y),
            (free_area.x, free_area.y),
            "negative free coordinates clamp to the viewport origin"
        );
        let below_right = engine_script_menu_layout_with_free_anchor(
            free_area,
            &hud_font,
            &menu,
            false,
            &HashMap::new(),
            (i32::MAX / 2, i32::MAX / 2),
            0,
            false,
            None,
        );
        assert_eq!(
            (below_right.bounds.x, below_right.bounds.y),
            (
                free_area.x + free_area.width as i32 - below_right.bounds.width as i32,
                free_area.y + free_area.height as i32 - below_right.bounds.height as i32,
            ),
            "right/bottom free coordinates clamp by the fully sized outer dialog"
        );

        let mut overflowing_menu = menu.clone();
        while overflowing_menu.items.len() < 5 {
            overflowing_menu.items.push(menu.items[0].clone());
        }
        let overflowing = engine_script_menu_layout_with_free_anchor(
            free_area,
            &hud_font,
            &overflowing_menu,
            false,
            &HashMap::new(),
            (i32::MAX / 2, free_area.y),
            0,
            false,
            None,
        );
        assert_eq!(
            overflowing.bounds.width,
            layout.bounds.width + CLASSIC_SCROLLBAR_WIDTH as u32,
            "InitSize reserves C4GUI_ScrollBarWdt before free-position clamping"
        );
        assert_eq!(
            overflowing.bounds.x,
            free_area.x + free_area.width as i32 - overflowing.bounds.width as i32,
        );

        let mut replacement_dialog = menu.clone();
        replacement_dialog.style = 3;
        let dialog_area = Rect::new(41, 27, 640, 480);
        let dialog_click = (dialog_area.x + 48, dialog_area.y + 36);
        let replacement_geometry = engine_script_menu_presentation_geometry_with_free_anchor(
            dialog_area,
            &hud_font,
            &replacement_dialog,
            &[],
            false,
            &HashMap::new(),
            dialog_click,
            0,
            false,
            None,
        )
        .expect("dialog style has presentation geometry");
        assert_eq!(
            (replacement_geometry.bounds.x, replacement_geometry.bounds.y),
            dialog_click,
            "SetLocation applies to a dialog that replaces Context during ActivateMenu"
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_free_anchor(
                dialog_area,
                &hud_font,
                &replacement_dialog,
                &[],
                false,
                true,
                GuiPoint::new(dialog_click.0 as f32 + 1.0, dialog_click.1 as f32 + 1.0),
                &HashMap::new(),
                dialog_click,
                0,
                None,
            ),
            Some(EngineScriptMenuPointerTarget::Title)
        );

        let point = GuiPoint::new(item.x as f32 + 1.0, item.y as f32 + 1.0);
        assert_eq!(
            engine_script_menu_pointer_target(area, &hud_font, &menu, &[], false, true, point,),
            Some(EngineScriptMenuPointerTarget::Item(0))
        );

        let gfx = IngameMenuGraphics::default();
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &hud_font,
            &font,
            None,
            &menu,
            &gfx,
            None,
            &[None],
            &[],
            true,
            0,
        );
        assert_eq!(
            surface
                .get_pixel(item.x as u32 + 1, item.y as u32 + 1)
                .expect("selected context cell pixel"),
            CLASSIC_SELECTION_COLOR
        );
    }

    #[test]
    fn engine_script_menu_collects_every_inline_image_spec_in_first_use_order() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Base", 0, 0);
            AddMenuItem("One", "", NONE, this());
            AddMenuItem("Two", "", NONE, this());
        }
        "#;
        let mut menu = script_menu_fixture("MENU", "Menu", script);
        menu.selection = 0;
        menu.caption = "{{CAPTION}} {{DUPLICATE}}".to_string();
        menu.items[0].caption = "{{{TITLE}}} {{DUPLICATE}}".to_string();
        menu.items[0].info_caption = "{{INFO_FIRST}} {{DUPLICATE}}".to_string();
        menu.items[1].caption = "{{ITEM_SECOND}} {{CAPTION}}".to_string();
        menu.items[1].info_caption = "{{INFO_SECOND}} {{TITLE}}".to_string();

        assert_eq!(engine_script_menu_title(&menu), "{{{TITLE}}} {{DUPLICATE}}");
        assert_eq!(
            engine_script_menu_inline_image_specs(&menu),
            vec![
                "CAPTION".to_string(),
                "DUPLICATE".to_string(),
                "TITLE".to_string(),
                "INFO_FIRST".to_string(),
                "ITEM_SECOND".to_string(),
                "INFO_SECOND".to_string(),
            ],
            "menu caption, computed title, every caption and every info caption are scanned recursively and deduplicated"
        );
    }

    #[test]
    fn goldrush_context_text_specs_share_render_and_pointer_geometry() {
        // Western.c4f/Goldrush.c4s/System.c4g/Trade.c ships this exact
        // Context-caption TextSpec. It must reserve and paint the SBTR facet
        // instead of displaying the braces as ordinary text.
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "{{CONTEXT_TITLE}} Trade", 0, 1);
            AddMenuItem("{{SBTR:8}} $MsgBuyIndian$", "BuyIndian", NONE, this());
        }
        "#;
        let mut menu = script_menu_fixture("MENU", "Menu", script);
        menu.selection = -1;

        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let title_color = Color::opaque(221, 31, 191);
        let item_color = Color::opaque(17, 213, 229);
        let images = HashMap::from([
            ("CONTEXT_TITLE".to_string(), solid_image(80, 8, title_color)),
            ("SBTR:8".to_string(), solid_image(120, 8, item_color)),
        ]);
        let area = Rect::new(0, 0, 1_000, 480);
        let empty_images = HashMap::new();
        let mapped =
            engine_script_menu_layout_with_images(area, &font, &menu, false, &images, None);
        let unmapped =
            engine_script_menu_layout_with_images(area, &font, &menu, false, &empty_images, None);
        let mapped_row = mapped.item_rect(0).expect("mapped Context row");
        let unmapped_row = unmapped.item_rect(0).expect("unmapped Context row");
        assert!(mapped.bounds.width > unmapped.bounds.width);
        assert!(
            mapped_row.x < unmapped_row.x,
            "right alignment exposes added width on the left"
        );

        let probe = GuiPoint::new(mapped_row.x as f32 + 1.0, mapped_row.y as f32 + 1.0);
        assert_eq!(
            engine_script_menu_pointer_target_with_info(
                area,
                &font,
                &menu,
                &[],
                false,
                false,
                probe,
                &images,
                None,
            ),
            Some(EngineScriptMenuPointerTarget::Item(0)),
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_info(
                area,
                &font,
                &menu,
                &[],
                false,
                false,
                probe,
                &empty_images,
                None,
            ),
            None,
            "the probe is only inside when hit-testing uses the rendering image map"
        );

        let gfx = IngameMenuGraphics {
            font_images: images,
            ..IngameMenuGraphics::default()
        };
        let mut surface = Surface::new(1_000, 480, PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &font,
            &fallback,
            None,
            &menu,
            &gfx,
            None,
            &[None],
            &[],
            false,
            0,
        );
        assert!(surface_rect_contains_color(
            &surface,
            mapped.title,
            title_color
        ));
        assert!(surface_rect_contains_color(
            &surface, mapped_row, item_color
        ));
    }

    #[test]
    fn engine_script_info_menu_uses_info_text_and_picture_rows() {
        // C4MN_Style_Info sizes a single-column menu from wrapped
        // InfoCaption text, then adds a 64px picture column and enforces a
        // 64px row height. It never paints the red selection box and its
        // items do not execute (src/C4Menu.cpp:141-181,498-503,666-693).
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Information", 0, 2);
            AddMenuItem("Caption is not rendered", "Choose()", MENU, this(), 0, 0,
                        "Wrapped information shown beside the picture");
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        assert_eq!(menu.style, 2);

        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &font, &menu, true);
        let row = layout.item_rect(0).expect("info row is visible");
        assert_eq!(layout.columns, 1);
        assert_eq!(row.height, 64);
        assert!(row.width >= 64);

        let blue = Color::opaque(23, 67, 211);
        let icon = ImageData::new(8, 8, [blue.r, blue.g, blue.b, blue.a].repeat(64));
        let gfx = IngameMenuGraphics {
            show_commands: true,
            ..IngameMenuGraphics::default()
        };
        let render = |menu: &clonk_engine::ObjectMenuState| {
            let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
            render_engine_script_menu(
                &mut surface,
                area,
                &font,
                &fallback,
                None,
                menu,
                &gfx,
                None,
                &[Some(icon.clone())],
                &[],
                false,
                0,
            );
            surface
        };
        let rendered = render(&menu);
        assert!(
            (row.y..row.y + row.height as i32).any(|y| {
                (row.x..row.x + 64).any(|x| rendered.get_pixel(x as u32, y as u32) == Some(blue))
            }),
            "the definition picture occupies the 64px left column"
        );
        assert_ne!(
            rendered.get_pixel(row.x as u32 + 63, row.y as u32 + 63),
            Some(CLASSIC_SELECTION_COLOR),
            "Info menus never draw a selection mark"
        );

        let mut changed_caption = menu.clone();
        changed_caption.items[0].caption = "A completely different hidden caption".to_string();
        assert_eq!(
            render(&changed_caption).snapshot(),
            rendered.snapshot(),
            "Info rendering uses InfoCaption rather than Caption"
        );
        let mut changed_info = menu.clone();
        changed_info.items[0].info_caption = "Different visible information".to_string();
        assert_ne!(render(&changed_info).snapshot(), rendered.snapshot());

        let mut at_tooltip_delay = Surface::new(640, 480, PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut at_tooltip_delay,
            area,
            &font,
            &fallback,
            None,
            &menu,
            &gfx,
            None,
            &[Some(icon)],
            &[],
            false,
            90,
        );
        assert_eq!(
            at_tooltip_delay.snapshot(),
            rendered.snapshot(),
            "Info rows never grow a delayed tooltip"
        );
        assert_eq!(
            engine_script_menu_pointer_target(
                area,
                &font,
                &menu,
                &[],
                true,
                false,
                GuiPoint::new(row.x as f32 + 1.0, row.y as f32 + 1.0),
            ),
            Some(EngineScriptMenuPointerTarget::Item(0)),
            "Info rows still participate in hover selection"
        );
    }

    #[test]
    fn engine_script_info_menu_wraps_markup_and_inline_images() {
        let font_bytes = std::fs::read(repository_root().join("planet/System.c4g/Endeavour.ttf"))
            .expect("Endeavour.ttf reads");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(&font_bytes)
            .expect("Endeavour fonts build");
        let font = HudFont::Clonk(&fonts.text);
        let red = Color::opaque(240, 20, 20);
        let blue = Color::opaque(20, 70, 240);
        let inline = ImageData::new(12, 6, [red.r, red.g, red.b, red.a].repeat(72));
        let title_inline = solid_image(300, 10, blue);
        let images = HashMap::from([
            ("TEST".to_string(), inline.clone()),
            ("TITLE".to_string(), title_inline),
        ]);
        let rich = layout_info_text(
            &font,
            "<c 00ff00>green</c>|{{TEST}}supercalifragilistic",
            60,
            &images,
        );
        assert!(
            rich.lines.len() >= 3,
            "manual and emergency breaks are retained"
        );
        assert!(rich
            .lines
            .iter()
            .flat_map(|line| &line.tokens)
            .any(|token| matches!(token, InfoTextToken::Image { spec, width } if spec == "TEST" && *width == font.graphics_line_height() * 2)));
        assert!(rich.lines.iter().all(|line| line.width <= 60));
        let leading_space = layout_info_text(&font, " abc", 1, &images);
        assert!(matches!(
            leading_space.lines[0].tokens.as_slice(),
            [InfoTextToken::Character { raw, .. }] if raw == " "
        ));

        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "{{TITLE}} Information", 0, 2);
            AddMenuItem("Hidden", "", MENU, this(), 0, 0,
                        "<c 00ff00>green</c>|{{TEST}}");
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        let mut nested_image_menu = menu.clone();
        nested_image_menu.items[0].info_caption = "{{{TEST}}}".to_string();
        assert_eq!(
            engine_script_menu_inline_image_specs(&nested_image_menu),
            vec!["TITLE".to_string(), "TEST".to_string()]
        );
        let fallback = clonk_graphics::BitmapFont::new();
        let gfx = IngameMenuGraphics {
            font_images: images,
            ..IngameMenuGraphics::default()
        };
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout_with_images(
            area,
            &font,
            &menu,
            false,
            &gfx.font_images,
            None,
        );
        let unmapped_layout =
            engine_script_menu_layout_with_images(area, &font, &menu, false, &HashMap::new(), None);
        let row = layout.item_rect(0).expect("info row");
        assert!(layout.bounds.width > unmapped_layout.bounds.width);
        assert!(
            row.x < unmapped_layout.item_rect(0).expect("unmapped info row").x,
            "right-aligned Info geometry includes title TextSpec width"
        );
        let probe = GuiPoint::new(
            row.x.max(area.x) as f32 + 1.0,
            row.y.max(area.y) as f32 + 1.0,
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_info(
                area,
                &font,
                &menu,
                &[],
                false,
                false,
                probe,
                &gfx.font_images,
                None,
            ),
            Some(EngineScriptMenuPointerTarget::Item(0)),
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_info(
                area,
                &font,
                &menu,
                &[],
                false,
                false,
                probe,
                &HashMap::new(),
                None,
            ),
            None
        );
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &font,
            &fallback,
            None,
            &menu,
            &gfx,
            None,
            &[None],
            &[],
            false,
            0,
        );
        let mut colors = Vec::new();
        for y in row.y..row.y + row.height as i32 {
            for x in row.x..row.x + row.width as i32 {
                if let Some(color) = surface.get_pixel(x as u32, y as u32) {
                    colors.push(color);
                }
            }
        }
        assert!(colors.iter().any(|color| color.g > 150 && color.r < 80));
        assert!(colors.contains(&red));
        assert!(surface_rect_contains_color(&surface, layout.title, blue));
    }

    #[test]
    fn free_info_menu_centers_safely_when_larger_than_the_viewport() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Information", 0, 2);
            AddMenuItem("Hidden", "", MENU, this(), 0, 0,
                        "A long information row that cannot fit a tiny viewport");
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let area = Rect::new(10, 20, 120, 80);

        let layout = engine_script_menu_layout_with_images(
            area,
            &font,
            &menu,
            false,
            &HashMap::new(),
            Some((i32::MAX, i32::MAX)),
        );

        assert_eq!(
            layout.bounds.x,
            area.x + (area.width as i32 - layout.bounds.width as i32) / 2
        );
        assert_eq!(
            layout.bounds.y,
            area.y + (area.height as i32 - layout.bounds.height as i32) / 2
        );
    }

    #[test]
    #[should_panic(expected = "refusing generic Rust fallback")]
    fn unavailable_script_menu_styles_never_render_the_generic_pane() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Unknown", 0, 99);
            AddMenuItem("Line", "", MENU, this());
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let gfx = IngameMenuGraphics::default();
        render_engine_script_menu(
            &mut Surface::new(640, 480, PixelFormat::Rgba8888),
            Rect::new(0, 0, 640, 480),
            &font,
            &fallback,
            None,
            &menu,
            &gfx,
            None,
            &[None],
            &[],
            false,
            0,
        );
    }

    #[test]
    fn dialog_layout_matches_cpp_portrait_and_variable_row_geometry() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "", 0, 3);
            AddMenuItem("", "", NONE, this());
            AddMenuItem("Narration", "", NONE, this());
            AddMenuItem("Continue", "Choose", MENU, this());
            AddMenuItem("Cancel", "Choose", MENU, this());
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        assert_eq!(
            font.line_height(),
            14,
            "fixture pins classic fallback metrics"
        );
        let icon = ImageData::new(1, 1, vec![255, 255, 255, 255]);
        let icons = vec![Some(icon.clone()), None, Some(icon.clone()), Some(icon)];

        let layout = dialog_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &menu, &icons);
        assert_eq!(layout.bounds, Rect::new(148, 35, 343, 72));
        assert_eq!(layout.title, None, "empty Dialog captions remove the title");
        assert_eq!(layout.client, Rect::new(150, 35, 339, 70));
        assert_eq!(layout.portrait, Some((0, Rect::new(150, 35, 69, 64))));
        assert_eq!(
            layout
                .rows
                .iter()
                .map(|row| (row.index, row.rect))
                .collect::<Vec<_>>(),
            vec![
                (1, Rect::new(219, 40, 270, 14)),
                (2, Rect::new(219, 59, 270, 14)),
                (3, Rect::new(219, 76, 270, 14)),
            ]
        );

        let portrait_layout =
            dialog_script_menu_layout(Rect::new(0, 0, 320, 480), &font, &menu, &icons);
        assert_eq!(portrait_layout.bounds, Rect::new(23, 35, 274, 72));
        assert!(portrait_layout.rows.iter().all(|row| row.rect.width == 201));
        assert_eq!(
            engine_script_menu_pointer_target(
                Rect::new(0, 0, 640, 480),
                &font,
                &menu,
                &icons,
                true,
                true,
                GuiPoint::new(480.0, 39.0),
            ),
            Some(EngineScriptMenuPointerTarget::Background),
            "an empty Dialog title never exposes a close target"
        );
        assert_eq!(
            engine_script_menu_pointer_target(
                Rect::new(0, 0, 640, 480),
                &font,
                &menu,
                &icons,
                true,
                true,
                GuiPoint::new(300.0, 62.0),
            ),
            Some(EngineScriptMenuPointerTarget::Item(2))
        );
    }

    #[test]
    fn dialog_text_specs_share_measurement_render_and_pointer_geometry() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "{{DIALOG_TITLE}} Question", 0, 3);
            AddMenuItem("{{DIALOG_ROW}} Answer", "Choose", NONE, this());
        }
        "#;
        let mut menu = script_menu_fixture("MENU", "Menu", script);
        menu.items[0].text_display_progress = -1;

        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let title_color = Color::opaque(239, 94, 31);
        let row_color = Color::opaque(41, 225, 105);
        let images = HashMap::from([
            (
                "DIALOG_TITLE".to_string(),
                solid_image(260, 10, title_color),
            ),
            ("DIALOG_ROW".to_string(), solid_image(180, 10, row_color)),
        ]);
        let area = Rect::new(0, 0, 800, 480);
        let icons = vec![None];
        let mapped = dialog_script_menu_layout_with_images(area, &font, &menu, &icons, &images);
        let empty_images = HashMap::new();
        let unmapped =
            dialog_script_menu_layout_with_images(area, &font, &menu, &icons, &empty_images);
        let mapped_row = &mapped.rows[0];
        let unmapped_row = &unmapped.rows[0];
        assert!(mapped.bounds.width > unmapped.bounds.width);
        assert!(mapped_row.rect.x < unmapped_row.rect.x);

        let probe = GuiPoint::new(
            mapped_row.rect.x as f32 + 1.0,
            mapped_row.rect.y as f32 + 1.0,
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_info(
                area,
                &font,
                &menu,
                &[],
                false,
                false,
                probe,
                &images,
                None,
            ),
            Some(EngineScriptMenuPointerTarget::Item(0))
        );
        assert_eq!(
            engine_script_menu_pointer_target_with_info(
                area,
                &font,
                &menu,
                &[],
                false,
                false,
                probe,
                &empty_images,
                None,
            ),
            None,
            "Dialog hit-testing must receive the same image map as rendering"
        );

        let gfx = IngameMenuGraphics {
            font_images: images,
            ..IngameMenuGraphics::default()
        };
        let mut surface = Surface::new(800, 480, PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &font,
            &fallback,
            None,
            &menu,
            &gfx,
            None,
            &icons,
            &[],
            false,
            0,
        );
        assert!(surface_rect_contains_color(
            &surface,
            mapped.title.expect("Dialog title"),
            title_color,
        ));
        assert!(surface_rect_contains_color(
            &surface,
            mapped_row.rect,
            row_color,
        ));
    }

    #[test]
    fn dialog_equal_item_height_only_equalizes_symbol_rows() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "", 0, 131);
            AddMenuItem("A", "Choose", MENU, this());
            AddMenuItem("A|B", "Choose", NONE, this());
            AddMenuItem("WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW", "Choose", MENU, this());
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        assert!(menu.equal_item_height);
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let icon = ImageData::new(1, 1, vec![255, 255, 255, 255]);
        let icons = vec![Some(icon.clone()), None, Some(icon)];
        let mut unequal = menu.clone();
        unequal.equal_item_height = false;
        let unequal = dialog_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &unequal, &icons);
        let equal = dialog_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &menu, &icons);

        assert!(unequal.rows[2].rect.height > unequal.rows[0].rect.height);
        assert_eq!(
            equal.rows[0].rect.height, equal.rows[2].rect.height,
            "all symbol-bearing rows take the largest natural icon-row height"
        );
        assert_eq!(
            equal.rows[1].rect.height, unequal.rows[1].rect.height,
            "symbol-free multiline rows retain their natural height"
        );
        assert!(equal.bounds.height > unequal.bounds.height);
    }

    #[test]
    fn dialog_progress_prefix_is_byte_based_and_markup_preserving() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "", 0, 3);
            AddMenuItem("<c ff0000>AB</c>|CD", "", NONE, this());
            AddMenuItem("éZ", "", NONE, this());
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        let mut markup = menu.items[0].clone();
        markup.text_display_progress = 0;
        assert_eq!(dialog_visible_caption(&markup), "");
        markup.text_display_progress = 11;
        assert_eq!(dialog_visible_caption(&markup), "<c ff0000>A");
        markup.text_display_progress = 12;
        assert_eq!(dialog_visible_caption(&markup), "<c ff0000>AB");
        markup.text_display_progress = -1;
        assert_eq!(dialog_visible_caption(&markup), "<c ff0000>AB</c>|CD");

        let mut unicode = menu.items[1].clone();
        unicode.text_display_progress = 1;
        assert_eq!(dialog_visible_caption(&unicode), "Ã");
        unicode.text_display_progress = 2;
        assert_eq!(dialog_visible_caption(&unicode), "é");

        unicode.caption = clonk_script::c4_string_from_bytes(&[0xff, b'Z']);
        unicode.text_display_progress = 1;
        assert_eq!(
            dialog_visible_caption(&unicode),
            clonk_resources::decode_legacy_script_text(&[0xff])
        );
        unicode.text_display_progress = -1;
        assert_eq!(
            dialog_visible_caption(&unicode),
            clonk_resources::decode_legacy_script_text(&[0xff, b'Z'])
        );
    }

    #[test]
    fn normal_menu_presentation_decodes_native_title_caption_and_tooltip_bytes() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Title", 0, 1);
            AddMenuItem("Entry", "Choose", MENU, this(), 0, 0, "Details");
        }
        "#;
        let mut raw = script_menu_fixture("MENU", "Menu", script);
        raw.caption = clonk_script::c4_string_from_bytes(b"Tit\xe9");
        raw.items[0].caption = clonk_script::c4_string_from_bytes(b"Entr\xe9");
        raw.items[0].info_caption = clonk_script::c4_string_from_bytes(b"D\xe9tails");
        raw.selection = 0;

        let mut presented = raw.clone();
        presented.caption = "Tit\u{e9}".into();
        presented.items[0].caption = "Entr\u{e9}".into();
        presented.items[0].info_caption = "D\u{e9}tails".into();

        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let area = Rect::new(0, 0, 640, 480);
        assert_eq!(engine_script_menu_title(&raw), "Tit\u{e9}");
        assert_eq!(
            engine_script_menu_layout(area, &font, &raw, false),
            engine_script_menu_layout(area, &font, &presented, false)
        );

        let render = |menu: &clonk_engine::ObjectMenuState| {
            let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
            render_engine_script_menu(
                &mut surface,
                area,
                &font,
                &fallback,
                None,
                menu,
                &IngameMenuGraphics::default(),
                None,
                &[None],
                &[],
                false,
                90,
            );
            surface.snapshot()
        };
        assert_eq!(render(&raw), render(&presented));
        assert_eq!(clonk_script::c4_string_bytes(&raw.caption), b"Tit\xe9");
        assert_eq!(
            clonk_script::c4_string_bytes(&raw.items[0].info_caption),
            b"D\xe9tails"
        );
    }

    #[test]
    fn dialog_menu_presentation_decodes_native_title_and_row_bytes() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Title", 0, 3);
            AddMenuItem("Entry", "Choose", NONE, this());
        }
        "#;
        let mut raw = script_menu_fixture("MENU", "Menu", script);
        raw.caption = clonk_script::c4_string_from_bytes(b"Tit\xe9");
        raw.items[0].caption = clonk_script::c4_string_from_bytes(b"Entr\xff");
        raw.items[0].text_display_progress = -1;

        let mut presented = raw.clone();
        presented.caption = "Tit\u{e9}".into();
        presented.items[0].caption = "Entr\u{ff}".into();

        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let area = Rect::new(0, 0, 640, 480);
        assert_eq!(dialog_visible_caption(&raw.items[0]), "Entr\u{ff}");
        assert_eq!(
            dialog_script_menu_layout(area, &font, &raw, &[None]),
            dialog_script_menu_layout(area, &font, &presented, &[None])
        );

        let render = |menu: &clonk_engine::ObjectMenuState| {
            let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
            render_engine_script_menu(
                &mut surface,
                area,
                &font,
                &fallback,
                None,
                menu,
                &IngameMenuGraphics::default(),
                None,
                &[None],
                &[],
                false,
                0,
            );
            surface.snapshot()
        };
        assert_eq!(render(&raw), render(&presented));
        assert_eq!(clonk_script::c4_string_bytes(&raw.caption), b"Tit\xe9");
        assert_eq!(
            clonk_script::c4_string_bytes(&raw.items[0].caption),
            b"Entr\xff"
        );
    }

    #[test]
    fn classic_dialog_renderer_suppresses_hidden_selection_and_symbol() {
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "", 0, 3);
            AddMenuItem("Choice", "Choose", MENU, this());
        }
        "#;
        let mut menu = script_menu_fixture("MENU", "Menu", script);
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let gfx = IngameMenuGraphics::default();
        let icon = ImageData::new(1, 1, vec![255, 255, 255, 255]);
        let icons = vec![Some(icon)];
        let area = Rect::new(0, 0, 640, 480);
        let layout = dialog_script_menu_layout(area, &font, &menu, &icons);
        let probe = (
            (layout.rows[0].rect.x + layout.rows[0].rect.width as i32 - 2) as u32,
            (layout.rows[0].rect.y + layout.rows[0].rect.height as i32 - 2) as u32,
        );

        menu.items[0].text_display_progress = 0;
        let mut hidden = Surface::new(640, 480, PixelFormat::Rgba8888);
        render_engine_dialog_menu(
            &mut hidden,
            area,
            &font,
            &menu,
            &gfx,
            None,
            &icons,
            Some(0),
            false,
            None,
        );
        assert_ne!(
            hidden.get_pixel(probe.0, probe.1),
            Some(CLASSIC_SELECTION_COLOR)
        );

        menu.items[0].text_display_progress = -1;
        let mut shown = Surface::new(640, 480, PixelFormat::Rgba8888);
        render_engine_dialog_menu(
            &mut shown,
            area,
            &font,
            &menu,
            &gfx,
            None,
            &icons,
            Some(0),
            false,
            None,
        );
        assert_eq!(
            shown.get_pixel(probe.0, probe.1),
            Some(CLASSIC_SELECTION_COLOR)
        );
    }

    #[test]
    fn dialog_decoration_replaces_frame_and_uses_captured_margins() {
        let decoration = clonk_engine::ObjectMenuFrameDecoration {
            source_definition: "DECO".to_string(),
            background_color: 0x803f3f00,
            border_top: 10,
            border_left: 10,
            border_right: 10,
            border_bottom: 10,
            top: Some(clonk_engine::DefinitionActionFacet {
                x: 0,
                y: 0,
                width: 3,
                height: 1,
                target_x: 0,
                target_y: 0,
            }),
            top_right: None,
            right: None,
            bottom_right: None,
            bottom: None,
            bottom_left: None,
            left: None,
            top_left: Some(clonk_engine::DefinitionActionFacet {
                x: 3,
                y: 0,
                width: 1,
                height: 1,
                target_x: -1,
                target_y: -1,
            }),
        };
        let mut surface = Surface::new(16, 16, PixelFormat::Rgba8888);
        let image = ImageData::new(
            4,
            1,
            vec![
                0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255,
            ],
        );
        let mut draw_decoration = decoration.clone();
        draw_decoration.border_top = 2;
        draw_decoration.border_left = 2;
        draw_decoration.border_right = 2;
        draw_decoration.border_bottom = 2;
        draw_menu_decoration(
            &mut surface,
            Rect::new(5, 5, 9, 9),
            &draw_decoration,
            Some(&image),
            None,
        );
        assert_eq!(
            surface.get_pixel(4, 4),
            Some(Color::opaque(255, 0, 0)),
            "negative corner targets intentionally protrude outside bounds"
        );
        assert_eq!(
            surface.get_pixel(7, 7),
            Some(Color::new(31, 31, 0, 127)),
            "the half-opaque background stays premultiplied on a transparent layer"
        );
        assert_eq!(surface.get_pixel(11, 5), Some(Color::opaque(0, 255, 0)));
        assert_eq!(
            surface.get_pixel(12, 5),
            Some(Color::new(31, 31, 0, 127)),
            "the final top tile is truncated before the right border"
        );

        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "", 0, 3);
            AddMenuItem("Line", "", NONE, this());
        }
        "#;
        let mut engine = Engine::new();
        engine
            .register_script_definition("MENU", "Menu", script)
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("MENU"))
            .expect("menu object spawns");
        let mut menu = engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("menu exists");
        menu.decoration = Some(decoration);
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Fallback(&fallback);
        let layout = dialog_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &menu, &[None]);
        assert_eq!(layout.bounds, Rect::new(173, 35, 294, 92));
        assert_eq!(layout.client, Rect::new(185, 45, 270, 70));

        menu.style = 1;
        let layout = engine_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &menu, false);
        assert_eq!(layout.title.y, layout.bounds.y + 10);
        assert_eq!(layout.client_x, layout.bounds.x + 12);
        assert_eq!(layout.client_y, layout.title.y + layout.title.height as i32);
        assert_eq!(
            engine_script_menu_pointer_target(
                Rect::new(0, 0, 640, 480),
                &font,
                &menu,
                &[],
                false,
                true,
                GuiPoint::new((layout.client_x + 1) as f32, (layout.client_y + 1) as f32,),
            ),
            Some(EngineScriptMenuPointerTarget::Item(0))
        );
    }

    /// `FrameDecoration::Draw` keeps whatever `SetByDef` read from the
    /// definition's callbacks and draws with it: the background box first, the
    /// edge helpers returning only on `Wdt <= 0` / `Hgt <= 0`, and the four
    /// corners handed straight to `C4Facet::Draw`
    /// (src/C4GuiDialogs.cpp:150-196). No border or target value is rejected
    /// — a negative border simply starts the edge loop outside the frame, and
    /// borders wider than the frame make `x < rcBounds.Wdt - iBorderRight`
    /// false immediately (clonk-org/clonk-rs#1207).
    #[test]
    fn out_of_range_decoration_geometry_draws_instead_of_refusing() {
        use clonk_engine::{DefinitionActionFacet, ObjectMenuFrameDecoration};

        let bounds = Rect::new(8, 8, 32, 24);
        let sheet = ImageData::new(8, 8, vec![255_u8; 8 * 8 * 4]);
        let facet = |target_x: i32, target_y: i32| {
            Some(DefinitionActionFacet {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
                target_x,
                target_y,
            })
        };
        let decoration = |borders: [i32; 4], target: i32| ObjectMenuFrameDecoration {
            source_definition: "DECO".to_string(),
            background_color: 0x0011_2233,
            border_top: borders[0],
            border_left: borders[1],
            border_right: borders[2],
            border_bottom: borders[3],
            top: facet(target, target),
            top_right: facet(target, target),
            right: facet(target, target),
            bottom_right: facet(target, target),
            bottom: facet(target, target),
            bottom_left: facet(target, target),
            left: facet(target, target),
            top_left: facet(target, target),
        };
        let paint = |decoration: &ObjectMenuFrameDecoration| {
            let mut surface = Surface::new(64, 48, PixelFormat::Rgba8888);
            surface.fill(Color::opaque(0, 0, 0));
            draw_menu_decoration(&mut surface, bounds, decoration, Some(&sheet), None);
            surface
        };

        for (label, borders, target) in [
            ("negative borders", [-6, -6, -6, -6], 0),
            ("borders wider than the frame", [40, 40, 40, 40], 0),
            ("targets far outside the frame", [4, 4, 4, 4], -500),
            ("targets far past the frame", [4, 4, 4, 4], 500),
            ("mixed signs", [-3, 40, -3, 40], -20),
        ] {
            let decoration = decoration(borders, target);
            // Drawing it is well defined and clipped to the surface.
            let surface = paint(&decoration);
            let inside = surface
                .get_pixel(bounds.x as u32 + 1, bounds.y as u32 + 1)
                .expect("a pixel inside the frame");
            assert!(
                (inside.r, inside.g, inside.b) == (0x11, 0x22, 0x33)
                    || (inside.r, inside.g, inside.b) == (255, 255, 255),
                "{label}: the frame holds either its background or a facet, not {inside:?}"
            );
        }

        // Borders wider than the frame make every edge loop skip, so only the
        // background box and the four unconditional corners can appear.
        let wide = decoration([40, 40, 40, 40], 0);
        let narrow_edges = paint(&wide);
        let inside = narrow_edges
            .get_pixel(bounds.x as u32 + 8, bounds.y as u32 + 8)
            .expect("a pixel inside the frame");
        assert_eq!(
            (inside.r, inside.g, inside.b),
            (0x11, 0x22, 0x33),
            "no edge run reaches the middle of the frame"
        );
    }

    /// `SetByDef` ignores every `SetFacetByAction` result, so a facet whose
    /// `FrameDeco*` action the definition never declared — or whose
    /// definition carries no bitmap at all — stays `Default()` and
    /// `C4Facet::Draw` returns immediately on it. The background box is drawn
    /// before any of them and is unaffected
    /// (src/C4GuiDialogs.cpp:95-124, 150-196).
    #[test]
    fn an_unresolved_decoration_facet_draws_nothing_and_keeps_the_background() {
        use clonk_engine::{DefinitionActionFacet, ObjectMenuFrameDecoration};

        let bounds = Rect::new(4, 6, 40, 30);
        let background = 0x0033_6699;
        let facet = || {
            Some(DefinitionActionFacet {
                x: 0,
                y: 0,
                width: 6,
                height: 6,
                target_x: 0,
                target_y: 0,
            })
        };
        let base = ObjectMenuFrameDecoration {
            source_definition: "DECO".to_string(),
            background_color: background,
            border_top: 6,
            border_left: 6,
            border_right: 6,
            border_bottom: 6,
            top: None,
            top_right: None,
            right: None,
            bottom_right: None,
            bottom: None,
            bottom_left: None,
            left: None,
            top_left: None,
        };

        // What the background alone paints, with no facet declared at all.
        let paint = |decoration: &ObjectMenuFrameDecoration| {
            let mut surface = Surface::new(64, 48, PixelFormat::Rgba8888);
            surface.fill(Color::opaque(0, 0, 0));
            draw_menu_decoration(&mut surface, bounds, decoration, None, None);
            surface.pixels().to_vec()
        };
        let background_only = paint(&base);

        // Every position in turn: declared, drawable, and unresolvable.
        for name in [
            "top",
            "top-right",
            "right",
            "bottom-right",
            "bottom",
            "bottom-left",
            "left",
            "top-left",
        ] {
            let mut decoration = base.clone();
            match name {
                "top" => decoration.top = facet(),
                "top-right" => decoration.top_right = facet(),
                "right" => decoration.right = facet(),
                "bottom-right" => decoration.bottom_right = facet(),
                "bottom" => decoration.bottom = facet(),
                "bottom-left" => decoration.bottom_left = facet(),
                "left" => decoration.left = facet(),
                _ => decoration.top_left = facet(),
            }
            assert_eq!(
                paint(&decoration),
                background_only,
                "an unresolvable {name} facet contributes no pixels"
            );
        }

        // The background itself still reaches the pixels it owns.
        let mut surface = Surface::new(64, 48, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(0, 0, 0));
        draw_menu_decoration(&mut surface, bounds, &base, None, None);
        let inside = surface
            .get_pixel(bounds.x as u32 + 1, bounds.y as u32 + 1)
            .expect("a pixel inside the frame");
        assert_eq!(
            (inside.r, inside.g, inside.b),
            (0x33, 0x66, 0x99),
            "the background box is drawn before any facet"
        );
    }

    #[test]
    fn real_last_will_assets_render_a_decorated_progressive_dialog() {
        let repository = repository_root();
        let last_will = repository.join("content/Missions.c4f/LastWill.c4s");
        let mut engine = Engine::new();
        let system_group =
            Group::open(repository.join("planet/System.c4g")).expect("System.c4g opens");
        let system_scripts = load_system_scripts(&system_group).expect("system scripts load");
        engine.install_global_scripts(&system_scripts);
        for relative in ["Dlg.c4d/MenuDeco.c4d", "Farmer.c4d"] {
            let group = Group::open(last_will.join(relative))
                .unwrap_or_else(|error| panic!("{relative} opens: {error}"));
            let resource = clonk_resources::ResourceDefinition::load(&group)
                .unwrap_or_else(|error| panic!("{relative} loads: {error}"));
            engine
                .register_definition(
                    Definition::from_resource(&resource)
                        .unwrap_or_else(|error| panic!("{relative} compiles: {error}")),
                )
                .unwrap_or_else(|error| panic!("{relative} registers: {error}"));
        }
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "", 0, 3, false, MENU);
            AddMenuItem("Portrait:_LEI::ffccaa::1", "", NONE, this(), 0, 0, "", 5, 0, 0);
            AddMenuItem("<c ff>Farmer:</c> The last will.", "", NONE, this(), 0, 0, "", 512, 0, 0);
            SetMenuDecoration(MD69, this());
            SetMenuTextProgress(7, this());
        }
        "#;
        engine
            .register_script_definition("MENU", "Last Will dialog fixture", script)
            .expect("fixture registers");
        let target = engine
            .spawn_object(SpawnConfig::new("MENU"))
            .expect("fixture spawns");
        let menu = engine
            .debug_object_menu(target.as_u64())
            .expect("target exists")
            .expect("dialog exists");
        assert_eq!(menu.style, 3);
        assert!(menu.text_progressing);
        assert_eq!(menu.items.len(), 2);
        assert!(matches!(
            menu.items[0].image,
            clonk_engine::ObjectMenuImage::TextSpec { .. }
        ));
        let decoration = menu.decoration.as_ref().expect("MD69 decoration");
        assert_eq!(
            (
                decoration.source_definition.as_str(),
                decoration.background_color,
                decoration.border_top,
                decoration.border_left,
                decoration.border_right,
                decoration.border_bottom,
            ),
            ("MD69", 0x803f3f00, 10, 10, 10, 10)
        );
        assert_eq!(
            decoration.top_left,
            Some(clonk_engine::DefinitionActionFacet {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                target_x: -10,
                target_y: -10,
            })
        );
        assert_eq!(
            decoration.top,
            Some(clonk_engine::DefinitionActionFacet {
                x: 20,
                y: 0,
                width: 88,
                height: 20,
                target_x: 0,
                target_y: -10,
            })
        );

        let snapshot = engine.snapshot();
        let item_icons = menu
            .items
            .iter()
            .map(|item| {
                clonk_app_core::pictures::object_menu_item_picture(
                    &engine,
                    &snapshot,
                    item,
                    0,
                    &clonk_frontend::HudGraphics::default(),
                    menu.style,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            item_icons[0]
                .as_ref()
                .map(|image| (image.width(), image.height())),
            Some((150, 150))
        );
        assert!(item_icons[1].is_none());

        // C4ObjectMenu::RefillInternal owns the facet before a later tick can
        // remove its source object (C4ObjectMenu.cpp:311-313, C4Menu.cpp:388-398).
        // The empty frame snapshot models that deletion; the retained
        // definition/picture inputs must still produce the row symbol.
        let mut native_item = menu.items[0].clone();
        native_item.image = clonk_engine::ObjectMenuImage::Definition;
        native_item.item_id = "_LEI".to_string();
        native_item.picture_object = Some(clonk_engine::ObjectId::new(999_999));
        native_item.picture_snapshot = Some(clonk_engine::ObjectMenuPictureSnapshot {
            definition_id: "_LEI".into(),
            symbol_size: 35,
            base_graphics: None,
            graphics_overlays: Vec::new(),
            blit_mode: 0,
            color: 0,
            color_modulation: 0,
            picture_rect: clonk_engine::DefinitionRect::default(),
            rank: None,
        });
        let frame_after_source_removal = engine.snapshot();
        assert!(
            clonk_app_core::pictures::object_menu_item_picture(
                &engine,
                &frame_after_source_removal,
                &native_item,
                0,
                &clonk_frontend::HudGraphics::default(),
                menu.style,
            )
            .is_some(),
            "the refill snapshot keeps the native row drawable after deletion",
        );
        let decoration_image = engine
            .definition_sprite_image("MD69", None)
            .expect("MD69 sprite sheet");
        assert_eq!(
            (decoration_image.width(), decoration_image.height()),
            (128, 128)
        );
        let decoration_image = ImageData::from_arc(
            decoration_image.width(),
            decoration_image.height(),
            decoration_image.into_pixels(),
        );
        let font_bytes = std::fs::read(repository.join("planet/System.c4g/Endeavour.ttf"))
            .expect("Endeavour.ttf reads");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(&font_bytes)
            .expect("Endeavour fonts build");
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Clonk(&fonts.text);
        let layout =
            dialog_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &menu, &item_icons);
        assert_eq!(layout.bounds, Rect::new(138, 35, 363, 132));
        assert_eq!(layout.client, Rect::new(150, 45, 339, 110));
        assert_eq!(layout.portrait, Some((0, Rect::new(150, 45, 69, 64))));

        let gfx = IngameMenuGraphics {
            frame_decoration: Some(decoration_image),
            ..IngameMenuGraphics::default()
        };
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(12, 24, 40));
        render_engine_script_menu(
            &mut surface,
            Rect::new(0, 0, 640, 480),
            &font,
            &fallback,
            None,
            &menu,
            &gfx,
            None,
            &item_icons,
            &[],
            false,
            0,
        );
        assert_eq!(surface.snapshot().to_string(), "640x480#e03235c1");
    }

    #[test]
    fn engine_context_menu_draws_cpp_composite_construction_and_exit_symbols() {
        // C4ObjectMenu::RefillInternal composes carried picture+Hand(0) for
        // Put, a definition picture for Contents, DrawMenuSymbol for
        // Buy/Sell, fctConstruction for BuildInfo, target+OKCancel(0,1) for
        // Info and fctExit for Exit (src/C4ObjectMenu.cpp:335-427;
        // src/C4Menu.cpp:43-70). AutoContextMenu's close command contains
        // "Exit", so the command strip also draws fctExit (:874-880).

        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Hut", 0, 1);
            AddMenuItem("Contents", "Choose()", MENU, this());
        }
        "#;
        let mut menu = script_menu_fixture("MENU", "Menu", script);
        let template = menu.items[0].clone();
        let make_item = |caption: &str, item_id: &str, symbol: clonk_engine::ObjectMenuSymbol| {
            let mut item = template.clone();
            item.caption = caption.to_string();
            item.item_id = item_id.to_string();
            item.symbol = symbol;
            item
        };
        menu.identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        menu.selection = -1;
        menu.items = vec![
            make_item("Put", "HUT3", clonk_engine::ObjectMenuSymbol::Put),
            make_item(
                "Contents",
                "HUT3",
                clonk_engine::ObjectMenuSymbol::Definition,
            ),
            make_item(
                "Buy",
                "NONE",
                clonk_engine::ObjectMenuSymbol::Buy { owner: 7 },
            ),
            make_item(
                "Sell",
                "NONE",
                clonk_engine::ObjectMenuSymbol::Sell { owner: 7 },
            ),
            make_item(
                "Construction material",
                "NONE",
                clonk_engine::ObjectMenuSymbol::Construction,
            ),
            make_item("Info", "HUT3", clonk_engine::ObjectMenuSymbol::Info),
            make_item("Exit", "NONE", clonk_engine::ObjectMenuSymbol::Exit),
        ];

        let gray = Color::opaque(80, 80, 80);
        let red = Color::opaque(240, 20, 20);
        let green = Color::opaque(20, 220, 20);
        let yellow = Color::opaque(240, 220, 20);
        let magenta = Color::opaque(220, 20, 220);
        let orange = Color::opaque(240, 120, 20);
        let cyan = Color::opaque(20, 220, 220);
        let purple = Color::opaque(160, 20, 220);
        let brown = Color::opaque(170, 90, 30);
        let mut arrow = vec![0_u8; 16 * 8 * 4];
        for y in 0..8 {
            for x in 0..16 {
                let offset = (y * 16 + x) * 4;
                let color = if x < 8 { yellow } else { magenta };
                arrow[offset..offset + 4].copy_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        }
        let mut control = vec![0_u8; 224 * 164 * 4];
        for y in 132..164 {
            for x in 128..160 {
                let offset = (y * 224 + x) * 4;
                control[offset..offset + 4]
                    .copy_from_slice(&[orange.r, orange.g, orange.b, orange.a]);
            }
        }
        let control = ImageData::new(224, 164, control);
        let gfx = IngameMenuGraphics {
            hud: clonk_frontend::HudGraphics {
                flag: Some(solid_rgba_image(8, 8, [0, 0, 255, 255])),
                wealth: Some(solid_rgba_image(8, 8, [green.r, green.g, green.b, green.a])),
                arrow: Some(ImageData::new(16, 8, arrow)),
                exit: Some(solid_rgba_image(8, 8, [cyan.r, cyan.g, cyan.b, cyan.a])),
                hand: Some(solid_rgba_image(
                    8,
                    8,
                    [purple.r, purple.g, purple.b, purple.a],
                )),
                construction: Some(solid_rgba_image(8, 8, [brown.r, brown.g, brown.b, brown.a])),
                control: Some(control.clone()),
                ..clonk_frontend::HudGraphics::default()
            },
            owner_colors: HashMap::from([(7, red)]),
            control: Some(control),
            show_commands: true,
            ..IngameMenuGraphics::default()
        };
        let picture = solid_rgba_image(8, 8, [gray.r, gray.g, gray.b, gray.a]);
        let item_icons = vec![
            Some(picture.clone()),
            Some(picture.clone()),
            None,
            None,
            None,
            Some(picture),
            None,
        ];
        let font = clonk_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &hud_font, &menu, true);
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &hud_font,
            &font,
            None,
            &menu,
            &gfx,
            None,
            &item_icons,
            &[],
            false,
            0,
        );

        let row = |index| layout.item_rect(index).expect("context row is visible");
        for color in [gray, purple] {
            assert!(
                surface_rect_contains_color(&surface, row(0), color),
                "missing Put {color:?}"
            );
        }
        assert!(surface_rect_contains_color(&surface, row(1), gray));
        for color in [red, green, yellow] {
            assert!(
                surface_rect_contains_color(&surface, row(2), color),
                "missing Buy {color:?}"
            );
        }
        for color in [red, green, magenta] {
            assert!(
                surface_rect_contains_color(&surface, row(3), color),
                "missing Sell {color:?}"
            );
        }
        assert!(surface_rect_contains_color(&surface, row(4), brown));
        for color in [gray, orange] {
            assert!(
                surface_rect_contains_color(&surface, row(5), color),
                "missing Info {color:?}"
            );
        }
        assert!(surface_rect_contains_color(&surface, row(6), cyan));
        let command_strip = Rect::new(
            layout.bounds.x,
            layout.bounds.y + layout.bounds.height as i32 - CLASSIC_COMMAND_HEIGHT,
            layout.bounds.width,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        assert!(surface_rect_contains_color(&surface, command_strip, cyan));
    }

    #[test]
    fn engine_buy_menu_draws_cpp_title_symbol_and_value_footer() {
        // C4Object::ActivateMenu(C4MN_Buy) supplies a composed Buy title
        // facet and C4MN_Extra_Value (C4Object.cpp:1919-1928). C4Menu keeps
        // one 16px footer whenever Extra is set, independently of command
        // hints, then DrawValue places a 32x16 wealth facet immediately left
        // of the selected value at the footer's right edge
        // (C4Menu.h:248-264; C4Menu.cpp:843-907; C4Facet.cpp:240-260).

        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Nothing to buy");
            AddMenuItem("Buy Lorry", "Choose()", LORY, this());
        }
        "#;
        let mut menu = script_menu_fixture("MENU", "Menu", script);
        menu.title_symbol = clonk_engine::ObjectMenuSymbol::Buy { owner: 7 };
        menu.extra = clonk_engine::ObjectMenuExtra::Value;
        menu.selection = 0;
        menu.items[0].value = Some(25);

        let red = Color::opaque(240, 20, 20);
        let green = Color::opaque(20, 220, 20);
        let yellow = Color::opaque(240, 220, 20);
        let magenta = Color::opaque(220, 20, 220);
        let mut arrow = vec![0_u8; 16 * 8 * 4];
        for y in 0..8 {
            for x in 0..16 {
                let offset = (y * 16 + x) * 4;
                let color = if x < 8 { yellow } else { magenta };
                arrow[offset..offset + 4].copy_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        }
        let gfx = IngameMenuGraphics {
            hud: clonk_frontend::HudGraphics {
                flag: Some(solid_rgba_image(8, 8, [0, 0, 255, 255])),
                wealth: Some(solid_rgba_image(8, 8, [green.r, green.g, green.b, green.a])),
                arrow: Some(ImageData::new(16, 8, arrow)),
                ..clonk_frontend::HudGraphics::default()
            },
            owner_colors: HashMap::from([(7, red)]),
            show_commands: false,
            ..IngameMenuGraphics::default()
        };
        let font = clonk_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &hud_font, &menu, false);
        let mut menu_without_extra = menu.clone();
        menu_without_extra.extra = clonk_engine::ObjectMenuExtra::None;
        let layout_without_extra =
            engine_script_menu_layout(area, &hud_font, &menu_without_extra, false);
        assert_eq!(
            layout.bounds.height,
            layout_without_extra.bounds.height + CLASSIC_COMMAND_HEIGHT as u32,
            "C4Menu::GetMarginBottom reserves the footer for Extra without DrawMenuControls"
        );

        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &hud_font,
            &font,
            None,
            &menu,
            &gfx,
            None,
            &[None],
            &[],
            false,
            0,
        );

        let title_symbol = Rect::new(
            layout.title.x + 1,
            layout.title.y + 1,
            layout.title.height - 2,
            layout.title.height - 2,
        );
        for color in [red, green, yellow] {
            assert!(
                surface_rect_contains_color(&surface, title_symbol, color),
                "missing Buy title component {color:?}"
            );
        }
        assert!(
            !surface_rect_contains_color(&surface, title_symbol, magenta),
            "Buy uses arrow phase 0, not Sell phase 1"
        );

        let footer = Rect::new(
            layout.bounds.x + 1,
            layout.bounds.y + layout.bounds.height as i32 - CLASSIC_COMMAND_HEIGHT - 1,
            layout.bounds.width - 2,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        let value_width = hud_font.text_width("25");
        let wealth = Rect::new(
            footer.x + footer.width as i32 - 1 - value_width - 2 * CLASSIC_COMMAND_HEIGHT,
            footer.y,
            (2 * CLASSIC_COMMAND_HEIGHT) as u32,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        assert!(surface_rect_contains_color(&surface, wealth, green));
        let value_text = Rect::new(
            footer.x + footer.width as i32 - 1 - value_width,
            footer.y,
            value_width.max(1) as u32,
            footer.height,
        );
        assert!(surface_rect_contains_color(
            &surface,
            value_text,
            CLASSIC_CAPTION_COLOR
        ));
    }

    #[test]
    fn menu_value_footer_runs_shipped_tendon_calc_def_value() {
        // C4Menu::DrawElement asks C4Def::GetValue(nil, NO_OWNER) for an
        // item without fOwnValue. Western's TEND is the shipped distinguishing
        // case: DefCore Value=2, while CalcDefValue returns 4.
        let repository = repository_root();
        let group =
            Group::open(repository.join("content/Western.c4d/Items.c4d/Materials.c4d/Tendon.c4d"))
                .expect("shipped Tendon definition opens");
        let resource = clonk_resources::ResourceDefinition::load(&group)
            .expect("shipped Tendon definition loads");
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_resource(&resource).expect("Tendon script compiles"),
            )
            .expect("Tendon definition registers");
        assert_eq!(engine.definition_value("TEND"), Some(2));

        let mut menu = engine_script_menu_fixture(0, 1);
        menu.extra = ObjectMenuExtra::Value;
        menu.selection = 0;
        menu.items[0].item_id = "TEND".to_string();
        menu.items[0].value = None;
        resolve_engine_script_menu_footer(&mut engine, &mut menu)
            .expect("Tendon footer value resolves");
        assert_eq!(menu.items[0].value, Some(4));

        let mut own_value_menu = menu.clone();
        own_value_menu.items[0].value = Some(2);
        resolve_engine_script_menu_footer(&mut engine, &mut own_value_menu)
            .expect("explicit menu value remains valid");
        assert_eq!(
            own_value_menu.items[0].value,
            Some(2),
            "C4MenuItem::fOwnValue suppresses definition-price recomputation"
        );

        let render = |menu: &clonk_engine::ObjectMenuState| {
            let font = clonk_graphics::BitmapFont::new();
            let hud_font = HudFont::Fallback(&font);
            let area = Rect::new(0, 0, 320, 200);
            let mut surface = Surface::new(320, 200, PixelFormat::Rgba8888);
            render_engine_script_menu(
                &mut surface,
                area,
                &hud_font,
                &font,
                None,
                menu,
                &IngameMenuGraphics::default(),
                None,
                &[None],
                &[],
                false,
                0,
            );
            surface.pixels().to_vec()
        };
        let calculated_pixels = render(&menu);
        let mut raw_value_menu = menu.clone();
        raw_value_menu.items[0].value = Some(2);
        assert_ne!(
            calculated_pixels,
            render(&raw_value_menu),
            "the painted CalcDefValue result must differ from raw DefCore Value=2"
        );
    }

    #[test]
    fn engine_magic_menu_draws_cpp_spell_cost_and_available_energy_footer() {
        // C4MN_Extra_MagicValue draws Magic.png followed by the selected
        // spell's `value/available` pair. Alchemy uses the components-only
        // variant because NMGE removes mana, while ordinary spell menus use
        // this same footer geometry (C4Menu.cpp:883-912;
        // C4Facet.cpp:265-290).

        let script = r#"
        func Initialize()
        {
            CreateMenu(MAGE, this(), this(), 0, "No spells");
            AddMenuItem("Cast Bridge", "DoMagic", MAGE, this());
        }
        "#;
        let mut engine = Engine::new();
        let mut definition =
            Definition::from_script("MAGE", "Mage", script).expect("script compiles");
        definition.set_value(10);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("MAGE"))
            .expect("mage spawns");
        let mut menu = engine
            .debug_object_menu(object.as_u64())
            .expect("mage exists")
            .expect("Initialize created its spell menu");
        menu.extra = ObjectMenuExtra::MagicValue;
        menu.extra_data = 30;
        menu.selection = 0;
        menu.items[0].value = Some(10);
        assert_eq!(menu.extra, ObjectMenuExtra::MagicValue);
        assert_eq!(menu.extra_data, 30);
        assert_eq!(menu.items[0].value, Some(10));

        let magic = Color::opaque(207, 35, 231);
        let gfx = IngameMenuGraphics {
            hud: clonk_frontend::HudGraphics {
                magic: Some(solid_rgba_image(
                    25,
                    35,
                    [magic.r, magic.g, magic.b, magic.a],
                )),
                ..clonk_frontend::HudGraphics::default()
            },
            show_commands: false,
            ..IngameMenuGraphics::default()
        };
        let font = clonk_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &hud_font, &menu, false);
        let footer = Rect::new(
            layout.bounds.x + 1,
            layout.bounds.y + layout.bounds.height as i32 - CLASSIC_COMMAND_HEIGHT - 1,
            layout.bounds.width - 2,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &hud_font,
            &font,
            None,
            &menu,
            &gfx,
            None,
            &[None],
            &[],
            false,
            0,
        );

        assert!(
            surface_rect_contains_color(&surface, footer, magic),
            "C++ spell menus draw Magic.png in the extra strip"
        );
        assert!(
            surface_rect_contains_color(&surface, footer, CLASSIC_CAPTION_COLOR),
            "C++ spell menus draw the selected spell's 10/30 cost/mana label"
        );

        let mut live_menu = menu.clone();
        live_menu.extra = ObjectMenuExtra::LiveMagicValue;
        live_menu.extra_data = object.as_u64() as i32;
        live_menu.items[0].value = None;
        engine
            .apply_object_update(object, ObjectUpdate::new().with_magic_energy(30_000))
            .expect("set initial live mana");
        resolve_engine_script_menu_footer(&mut engine, &mut live_menu)
            .expect("static spell value resolves");
        assert_eq!(live_menu.items[0].value, Some(10));
        assert_eq!(live_menu.extra_data, 30);

        live_menu.extra_data = object.as_u64() as i32;
        engine
            .apply_object_update(object, ObjectUpdate::new().with_magic_energy(20_000))
            .expect("spend live mana");
        resolve_engine_script_menu_footer(&mut engine, &mut live_menu)
            .expect("live spell value resolves");
        assert_eq!(live_menu.extra_data, 20, "live footer updates every frame");
    }

    #[test]
    fn engine_components_menu_draws_cached_requirements_right_to_left() {
        // C4MenuItem snapshots the selected definition's components, then
        // C4MN_Extra_Components draws those C4IDList entries from the footer's
        // right edge in stored order. Right|Triple|Half turns each 16px-high
        // section into a 24x16 cell; counts are literal "Nx" labels at the
        // cell's bottom-right (C4Menu.cpp:92-97,843-899;
        // C4IDList.cpp:207-227; C4Facet.cpp:182-213).

        let script = r#"
        func Initialize()
        {
            CreateMenu(CXCN, this(), this(), 1, "No construction plans available");
            AddMenuItem("Construction: Elevator", "CreateConstructionSite", ELEV, this());
        }
        "#;
        let mut engine = Engine::new();
        engine
            .register_script_definition("CXCN", "Construction", script)
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("CXCN"))
            .expect("menu object spawns");
        let mut menu = engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("Initialize created its menu");
        menu.extra = clonk_engine::ObjectMenuExtra::Components;
        menu.selection = 0;
        menu.items[0].components = vec![
            clonk_engine::ObjectMenuComponent {
                definition_id: "WOOD".to_string(),
                count: 4,
            },
            clonk_engine::ObjectMenuComponent {
                definition_id: "METL".to_string(),
                count: 2,
            },
        ];

        let font = clonk_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &hud_font, &menu, true);
        assert_eq!(layout.bounds, Rect::new(391, 369, 179, 76));
        let footer = Rect::new(
            layout.bounds.x + 1,
            layout.bounds.y + layout.bounds.height as i32 - CLASSIC_COMMAND_HEIGHT - 1,
            layout.bounds.width - 2,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        assert_eq!(footer, Rect::new(392, 428, 177, 16));
        let remaining_after_six_controls = Rect::new(
            footer.x + 6 * CLASSIC_COMMAND_HEIGHT,
            footer.y,
            footer.width - 6 * CLASSIC_COMMAND_HEIGHT as u32,
            footer.height,
        );
        assert_eq!(
            component_footer_cells(remaining_after_six_controls, &menu.items[0].components)
                .into_iter()
                .map(|cell| (cell.rect, cell.count_label))
                .collect::<Vec<_>>(),
            vec![
                (Rect::new(545, 428, 24, 16), "4x".to_string()),
                (Rect::new(521, 428, 24, 16), "2x".to_string()),
            ],
            "stored WOOD,METL order must render first/rightmost, hence visually METL,WOOD"
        );

        let wood = Color::opaque(146, 92, 45);
        let metal = Color::opaque(92, 112, 132);
        let component_icons = vec![
            Some(solid_rgba_image(8, 8, [wood.r, wood.g, wood.b, wood.a])),
            Some(solid_rgba_image(8, 8, [metal.r, metal.g, metal.b, metal.a])),
        ];
        let gfx = IngameMenuGraphics {
            show_commands: true,
            ..IngameMenuGraphics::default()
        };
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            area,
            &hud_font,
            &font,
            None,
            &menu,
            &gfx,
            None,
            &[None],
            &component_icons,
            false,
            0,
        );

        let metal_cell = Rect::new(521, 428, 24, 16);
        let wood_cell = Rect::new(545, 428, 24, 16);
        assert!(surface_rect_contains_color(&surface, metal_cell, metal));
        assert!(!surface_rect_contains_color(&surface, metal_cell, wood));
        assert!(surface_rect_contains_color(&surface, wood_cell, wood));
        assert!(!surface_rect_contains_color(&surface, wood_cell, metal));
        assert!(
            surface_rect_contains_color(&surface, metal_cell, CLASSIC_CAPTION_COLOR),
            "METL cell must overlay the white literal 2x count"
        );
        assert!(
            surface_rect_contains_color(&surface, wood_cell, CLASSIC_CAPTION_COLOR),
            "WOOD cell must overlay the white literal 4x count"
        );
    }

    #[test]
    fn engine_script_normal_menu_uses_cpp_grid_geometry_and_selection_color() {
        // CreateMenu/AddMenuItem populate the runtime menu drawn by
        // C4Viewport::DrawMenu (C4Viewport.cpp:983-995). An item without a
        // command is not selectable and a forced non-sentinel count is drawn
        // beside its caption (C4Menu.cpp:152-210).
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Choose");
            AddMenuItem("Information", "", MENU, this(), 0);
            AddMenuItem("Selectable", "Choose()", MENU, this(), 3, 0, "Details");
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);

        assert_eq!(engine_script_menu_title(&menu), "Selectable");
        assert_eq!(menu.symbol_id, "MENU");
        let mut menu_without_caption = menu.clone();
        menu_without_caption.caption.clear();
        menu_without_caption.selection = -1;
        assert_eq!(engine_script_menu_title(&menu_without_caption), " ");
        let entries = &menu.items;
        assert_eq!(entries[0].label(), "Information");
        assert!(!entries[0].selectable());
        assert_eq!(entries[0].count_label(), None);
        assert_eq!(entries[1].label(), "Selectable");
        assert_eq!(entries[1].info_caption, "Details");
        assert!(entries[1].selectable());
        assert_eq!(entries[1].count_label().as_deref(), Some("3x"));
        assert_eq!(menu.selection, 1);

        let font = clonk_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let mut gui_icons = vec![0_u8; 240 * 240 * 4];
        for y in 200..240 {
            for x in 160..200 {
                let offset = (y * 240 + x) * 4;
                gui_icons[offset..offset + 4].copy_from_slice(&[17, 238, 51, 255]);
            }
        }
        let mut control = vec![0_u8; 224 * 164 * 4];
        for y in 100..132 {
            for x in 160..192 {
                let offset = (y * 224 + x) * 4;
                control[offset..offset + 4].copy_from_slice(&[239, 23, 71, 255]);
            }
        }
        let gfx = IngameMenuGraphics {
            show_commands: true,
            gui_icons: Some(ImageData::new(240, 240, gui_icons)),
            control: Some(ImageData::new(224, 164, control)),
            ..IngameMenuGraphics::default()
        };
        let icons = vec![None, None];
        // DrawMenuControls reserves one C4MN_SymbolSize (16px) below the
        // 35px item grid (src/C4Menu.h:32-35,262), and C4Menu::DrawElement
        // uses that complete strip for its square command cells
        // (src/C4Menu.cpp:843-880). InitSize includes the strip in the menu
        // bounds before bottom alignment (src/C4Menu.cpp:755-777).
        let layout = engine_script_menu_layout(Rect::new(0, 0, 640, 480), &hud_font, &menu, true);
        assert_eq!(CLASSIC_COMMAND_HEIGHT, 16);
        assert_eq!(layout.bounds, Rect::new(391, 369, 179, 76));
        assert_eq!(layout.item_rect(0), Some(Rect::new(393, 392, 35, 35)));
        assert_eq!(
            engine_script_menu_pointer_target(
                Rect::new(0, 0, 640, 480),
                &hud_font,
                &menu,
                &[],
                true,
                true,
                GuiPoint::new(411.0, 393.0),
            ),
            Some(EngineScriptMenuPointerTarget::Item(0)),
            "pointer hit cells must move with the C++-sized command strip"
        );
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        render_engine_script_menu(
            &mut surface,
            Rect::new(0, 0, 640, 480),
            &hud_font,
            &font,
            None,
            &menu,
            &gfx,
            None,
            &icons,
            &[],
            true,
            0,
        );

        let mut narrow_menu = menu.clone();
        narrow_menu.columns = 1;
        let narrow_layout =
            engine_script_menu_layout(Rect::new(0, 0, 640, 480), &hud_font, &narrow_menu, true);
        let mut overflow_surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        let overflow_probe = (
            u32::try_from(narrow_layout.bounds.x + narrow_layout.bounds.width as i32 + 50)
                .expect("overflow probe x is positive"),
            u32::try_from(narrow_layout.bounds.y + narrow_layout.bounds.height as i32 - 10)
                .expect("overflow probe y is positive"),
        );
        let overflow_before = overflow_surface
            .get_pixel(overflow_probe.0, overflow_probe.1)
            .expect("overflow probe starts on the surface");
        render_engine_script_menu(
            &mut overflow_surface,
            Rect::new(0, 0, 640, 480),
            &hud_font,
            &font,
            None,
            &narrow_menu,
            &gfx,
            None,
            &icons,
            &[],
            true,
            0,
        );

        // WoodenLabel clips title text to its bounds and reserves a 20px
        // right indent for a mouse close button (C4GuiLabels.cpp:168-209;
        // C4GuiDialogs.cpp:386-421). Even an overlong selected caption must
        // not paint outside a narrow menu title.
        let title_right = narrow_layout.title.x + narrow_layout.title.width as i32;
        let title_overflow = (narrow_layout.title.y
            ..narrow_layout.title.y + narrow_layout.title.height as i32)
            .find_map(|y| {
                (title_right..title_right + 80).find_map(|x| {
                    overflow_surface
                        .get_pixel(x as u32, y as u32)
                        .filter(|color| *color != Color::new(0, 0, 0, 0))
                        .map(|color| (x, y, color))
                })
            });
        assert_eq!(
            title_overflow, None,
            "C++ WoodenLabel clipping must keep narrow menu captions inside the title"
        );

        // A one-column menu cannot fit all six requested controls.
        // C4Facet::TruncateSection returns an empty facet once another 16px
        // square no longer fits, so later phases must not leak past the menu
        // edge (C4Menu.cpp:857-880; C4Facet.cpp:182-217).
        assert_eq!(
            overflow_surface
                .get_pixel(overflow_probe.0, overflow_probe.1)
                .expect("overflow probe remains on the surface"),
            overflow_before,
            "command cells must be truncated to the C++ menu strip"
        );

        // Normal style is always a five-column 35px icon grid. With the
        // 23px wooden title, 16px command bar and 2px frame, this two-item
        // menu is 179x76 and aligns Right|Bottom at (391,369) in 640x480
        // (C4Menu.h:32-35,262; C4Menu.cpp:642-777). Item 1 starts at client
        // (428,392), and the
        // selected cell is filled with palette CRed (#c80000) before its
        // icon is drawn (C4Menu.cpp:147-154).
        assert_eq!(
            surface.get_pixel(429, 393).expect("selected cell pixel"),
            Color::opaque(0xc8, 0, 0),
            "script menus must use the C++ five-column icon-grid geometry"
        );
        assert_eq!(
            surface.get_pixel(558, 381).expect("close icon pixel"),
            Color::opaque(17, 238, 51),
            "mouse-controlled C4Menu title bars show Ico_Close at their top-right corner"
        );
        assert_eq!(
            engine_script_menu_pointer_target(
                Rect::new(0, 0, 640, 480),
                &hud_font,
                &menu,
                &[],
                true,
                true,
                GuiPoint::new(558.0, 381.0),
            ),
            Some(EngineScriptMenuPointerTarget::Close)
        );
        assert_eq!(
            engine_script_menu_pointer_target(
                Rect::new(0, 0, 640, 480),
                &hud_font,
                &menu,
                &[],
                true,
                true,
                GuiPoint::new(411.0, 400.0),
            ),
            Some(EngineScriptMenuPointerTarget::Item(0)),
            "even non-selectable cells consume GUI pointer input"
        );
    }

    #[test]
    fn definition_menu_pictures_use_cpp_default_blue_owner_color() {
        // Graphics.GetBitmap(0) calls SetClr(0), which substitutes 0xff;
        // that is blue in C4's packed surface color (C4DefGraphics.h:49;
        // C4Surface.h:110). Dragon Rock's Knight and Mage menu pictures
        // therefore use blue, not their raw red/black overlay pixels.
        let mut pixels = vec![0, 0, 0, 255, 100, 50, 10, 255];
        apply_default_menu_owner_color(&mut pixels, &[136, 0]);
        assert_eq!(&pixels[..4], &[0, 0, 136, 255]);
        assert_eq!(&pixels[4..], &[100, 50, 10, 255]);

        let mut pixels = vec![0, 255, 0, 255, 100, 50, 10, 255];
        apply_default_menu_owner_color(&mut pixels, &[128, 128, 128, 128, 0, 0, 255, 255]);
        assert_eq!(&pixels[..4], &[0, 127, 64, 255]);
        assert_eq!(&pixels[4..], &[0, 0, 255, 255]);

        let mut transparent_base = vec![0, 0, 0, 0];
        apply_default_menu_owner_color(&mut transparent_base, &[128, 128, 128, 128]);
        assert_eq!(
            transparent_base,
            [0, 0, 128, 128],
            "flattening keeps straight RGB so the eventual draw applies alpha only once"
        );
    }

    #[test]
    fn real_dragon_rock_choice_menus_match_cpp_geometry_assets_and_timing() {
        let (mut engine, owner) = load_repository_dragon_rock();
        let (crew, difficulty) = engine
            .cursor_object_menu(owner)
            .map(|(crew, menu)| (crew, menu.clone()))
            .expect("Dragon Rock difficulty menu");
        assert_eq!(difficulty.caption, "Select difficulty");
        assert_eq!(difficulty.symbol_id, "WIPF");
        assert_eq!(difficulty.title_symbol, ObjectMenuSymbol::Definition);
        assert_eq!(difficulty.style, 0);
        assert!(!difficulty.permanent);
        assert_eq!(difficulty.extra, ObjectMenuExtra::None);
        assert_eq!(difficulty.selection, 0);
        assert!(difficulty.user_menu);
        assert_eq!(difficulty.command_object, Some(crew));
        assert_eq!(difficulty.columns, 5);
        assert_eq!(difficulty.lines, 0);
        assert_eq!(engine_script_menu_title(&difficulty), "Difficulty: Normal");
        assert_eq!(
            difficulty
                .items
                .iter()
                .map(|item| (
                    item.item_id.as_str(),
                    item.caption.as_str(),
                    item.info_caption.as_str(),
                    item.command.as_str(),
                    item.command2.as_str(),
                    item.count,
                    item.symbol,
                    item.selectable,
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "WIPF",
                    "Difficulty: Normal",
                    "Each player receives a flag.",
                    "SetDifficulty(1)",
                    "SetDifficulty(1)",
                    MENU_ITEM_NO_COUNT,
                    ObjectMenuSymbol::Definition,
                    true,
                ),
                (
                    "MONS",
                    "Difficulty: Hard",
                    "Players start without flags.",
                    "SetDifficulty(2)",
                    "SetDifficulty(2)",
                    MENU_ITEM_NO_COUNT,
                    ObjectMenuSymbol::Definition,
                    true,
                ),
            ]
        );

        engine
            .player_in_com(owner, clonk_engine::COM_THROW, 0)
            .expect("choose normal difficulty");
        let (_, character_knight) = engine
            .cursor_object_menu(owner)
            .map(|(crew, menu)| (crew, menu.clone()))
            .expect("Dragon Rock character menu");
        assert_eq!(character_knight.caption, "Choose character");
        assert_eq!(character_knight.symbol_id, "CLNK");
        assert_eq!(character_knight.title_symbol, ObjectMenuSymbol::Definition);
        assert_eq!(character_knight.style, 0);
        assert_eq!(character_knight.selection, 0);
        assert_eq!(character_knight.columns, 5);
        assert_eq!(character_knight.lines, 0);
        assert_eq!(
            engine_script_menu_title(&character_knight),
            "Character: Knight"
        );
        assert_eq!(
            character_knight
                .items
                .iter()
                .map(|item| (
                    item.item_id.as_str(),
                    item.caption.as_str(),
                    item.info_caption.as_str(),
                    item.command.as_str(),
                    item.command2.as_str(),
                    item.count,
                    item.symbol,
                    item.selectable,
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "KNIG",
                    "Character: Knight",
                    "Play as Knight.",
                    "Redefine3(KNIG)",
                    "Redefine3(KNIG)",
                    MENU_ITEM_NO_COUNT,
                    ObjectMenuSymbol::Definition,
                    true,
                ),
                (
                    "MAGE",
                    "Character: Mage",
                    "Play as Mage.",
                    "Redefine3(MAGE)",
                    "Redefine3(MAGE)",
                    MENU_ITEM_NO_COUNT,
                    ObjectMenuSymbol::Definition,
                    true,
                ),
            ]
        );
        engine
            .player_in_com(owner, clonk_engine::COM_RIGHT, 0)
            .expect("select mage");
        let (_, character_mage) = engine
            .cursor_object_menu(owner)
            .map(|(crew, menu)| (crew, menu.clone()))
            .expect("Dragon Rock mage selection");
        assert_eq!(character_mage.selection, 1);
        assert_eq!(engine_script_menu_title(&character_mage), "Character: Mage");

        let picture_snapshot = |id: &str| {
            let picture = definition_menu_picture(
                engine
                    .definition_picture_image(id)
                    .unwrap_or_else(|| panic!("{id} picture")),
            );
            let mut surface =
                Surface::new(picture.width(), picture.height(), PixelFormat::Rgba8888);
            surface.pixels_mut().copy_from_slice(picture.pixels());
            surface.snapshot().to_string()
        };
        // Snapshot hashes include transparent texels; C4Surface loads those
        // as transparent black before the definition picture is cropped.
        //
        // The dimensions follow the shipped art rather than being fixed: a
        // Picture rect is authored in game units and multiplied by DefCore
        // `Scale` when it is cropped (C4Def.cpp:745, `Scale / 100.0f`). CLNK
        // and KNIG now ship crew sheets rendered at `Scale=300`, so their
        // 32x40 and 38x44 rects crop to 96x120 and 114x132; WIPF, MONS and
        // MAGE have no high-resolution pack and stay at `Scale=100`. Each of
        // these is that arithmetic, so the C++ rule is still what is pinned
        // here — no capture is involved. On-screen size is unchanged: the menu
        // fits every picture into a fixed cell, so a larger crop is sharper
        // rather than bigger.
        assert_eq!(
            ["WIPF", "MONS", "CLNK", "KNIG", "MAGE"].map(picture_snapshot),
            [
                "64x64#90fd40b3",
                "64x64#9fc301d8",
                "96x120#4803e685",
                "114x132#d62b476a",
                "32x40#965dc12f",
            ]
        );

        let graphics_resource = clonk_resources::graphics::GraphicsResource::open(
            repository_root().join("planet/Graphics.c4g"),
        )
        .expect("Graphics.c4g opens");
        let image = |name: &str| {
            let image = graphics_resource
                .load_image(name)
                .unwrap_or_else(|error| panic!("{name} loads: {error}"));
            ImageData::new(image.width(), image.height(), image.pixels().to_vec())
        };
        let font_bytes = std::fs::read(repository_root().join("planet/System.c4g/Endeavour.ttf"))
            .expect("Endeavour.ttf reads");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(&font_bytes)
            .expect("Endeavour fonts build");
        let fallback = clonk_graphics::BitmapFont::new();
        let font = HudFont::Clonk(&fonts.text);
        let tiny = HudFont::Clonk(&fonts.mini);
        let gfx = IngameMenuGraphics {
            control: Some(image("Control.png")),
            caption_bar: Some(image("GUICaption.png")),
            show_commands: true,
            show_command_keys: true,
            throw_key: "A".to_string(),
            special2_key: "F".to_string(),
            dig_key: "D".to_string(),
            ..IngameMenuGraphics::default()
        };
        let gamma_points = engine
            .snapshot()
            .environment
            .gamma
            .combined_control_points();
        assert_eq!(gamma_points, [0x000000, 0x808080, 0xffffff]);
        let gamma = GammaRamp::from_control_points(gamma_points);
        let expected_layout = EngineScriptMenuLayout {
            bounds: Rect::new(391, 369, 179, 76),
            title: Rect::new(391, 369, 179, 23),
            client: Rect::new(393, 392, 175, 35),
            client_x: 393,
            client_y: 392,
            columns: 5,
            lines: 1,
            item_width: 35,
            item_height: 35,
            scroll_y: 0,
            max_scroll_y: 0,
            scrollbar: None,
            first_index: 0,
            visible: 5,
        };
        assert_eq!(
            engine_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &difficulty, true),
            expected_layout
        );
        assert_eq!(
            engine_script_menu_layout(Rect::new(0, 0, 640, 480), &font, &character_knight, true,),
            expected_layout
        );

        let render = |menu: &clonk_engine::ObjectMenuState, time_on_selection: u32| {
            let title_icon = engine
                .definition_picture_image(&menu.symbol_id)
                .map(definition_menu_picture);
            let item_icons = menu
                .items
                .iter()
                .map(|item| {
                    engine
                        .definition_picture_image(&item.item_id)
                        .map(definition_menu_picture)
                })
                .collect::<Vec<_>>();
            let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
            surface.fill(Color::opaque(12, 24, 40));
            render_engine_script_menu_with_gamma(
                &mut surface,
                Rect::new(0, 0, 640, 480),
                &font,
                &fallback,
                Some(&tiny),
                menu,
                &gfx,
                title_icon.as_ref(),
                &item_icons,
                &[],
                false,
                time_on_selection,
                Some(&gamma),
                None,
            );
            surface
        };
        let difficulty_1 = render(&difficulty, 1);
        let difficulty_89 = render(&difficulty, 89);
        let difficulty_90 = render(&difficulty, 90);
        let character_knight_1 = render(&character_knight, 1);
        let character_mage_1 = render(&character_mage, 1);
        let character_mage_90 = render(&character_mage, 90);
        // Re-pinned against the 2026-07-21 Drachenfels C++ GL
        // capture (Screenshot001.png): the extra-bar divider leaves its
        // bottom-right corner unpainted (capture (1208,662) = background,
        // CStdDDraw::DrawFrame end-drop) and the delayed tooltip frame blends
        // every corner exactly once ((942,580)/(1123,580) = (121,117,60), not
        // the double-blended (61,59,30) the former full strips produced).
        assert_eq!(difficulty_1.snapshot().to_string(), "640x480#fbce6a84");
        assert_eq!(difficulty_89.snapshot(), difficulty_1.snapshot());
        assert_eq!(difficulty_90.snapshot().to_string(), "640x480#5cf97238");
        // The three character-menu hashes were refreshed when CLNK and KNIG
        // started shipping crew sheets rendered at DefCore `Scale=300`, which
        // changes the two definition pictures the menu draws.
        //
        // The C++ GL capture above verified menu CHROME — the divider's dropped
        // end pixel and the single-blend tooltip corners — and that chrome is
        // untouched here. Rendering these same three menus against the previous
        // low-resolution art and diffing gives 821 / 820 / 707 differing pixels,
        // every one of them inside the title symbol or the KNIG item cell
        // (bounding box x 396..423): zero chrome pixels move, and the MAGE cell,
        // whose art did not change, is bit-identical. The layout assertions
        // above still pass unchanged, so what the capture pinned still holds.
        assert_eq!(
            [
                character_knight_1.snapshot().to_string(),
                character_mage_1.snapshot().to_string(),
                character_mage_90.snapshot().to_string(),
            ],
            ["640x480#067a8001", "640x480#f445457b", "640x480#34adb1cd"]
        );
    }

    /// Markup is free; images are not (clonk-org/clonk-rs#563).
    ///
    /// `CMarkup` tags carry no advance of their own — `CMarkupTagItalic::Apply`
    /// only shears the blit transform (`StdMarkup.cpp:24-28`), and
    /// `GetTextExtent` measures with markup stripped — so wrapping an Info row
    /// in `<i>`, nesting it, or coloring it must leave the line widths, the
    /// wrap points and therefore the row's hit rectangle exactly where the
    /// plain text put them.
    ///
    /// An inline `{{image}}` is the deliberate counter-case: it *does* occupy
    /// width, so it must move them. Asserting only the first half would pass
    /// against a layout that ignored every markup construct including images.
    #[test]
    fn info_markup_leaves_row_geometry_alone_while_inline_images_move_it() {
        let font_bytes = std::fs::read(repository_root().join("planet/System.c4g/Endeavour.ttf"))
            .expect("Endeavour.ttf reads");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(&font_bytes)
            .expect("Endeavour fonts build");
        let font = HudFont::Clonk(&fonts.text);
        let images = HashMap::from([(
            "TEST".to_string(),
            solid_image(12, 6, Color::opaque(240, 20, 20)),
        )]);

        // One wrap width for every form, chosen narrow enough that the text
        // actually wraps — a width that fits on one line would make the
        // "wrap points are unchanged" half vacuous.
        let width = 60;
        let plain = layout_info_text(&font, "supercalifragilistic expialidocious", width, &images);
        assert!(
            plain.lines.len() > 1,
            "the sample must wrap for this test to say anything about wrap points"
        );
        let shape = |layout: &InfoTextLayout| {
            (
                layout.width,
                layout
                    .lines
                    .iter()
                    .map(|line| line.width)
                    .collect::<Vec<_>>(),
            )
        };

        for markup in [
            "<i>supercalifragilistic expialidocious</i>",
            "<i><i>supercalifragilistic expialidocious</i></i>",
            "<c 00ff00>supercalifragilistic expialidocious</c>",
            "<i><c 00ff00>supercalifragilistic expialidocious</c></i>",
        ] {
            let decorated = layout_info_text(&font, markup, width, &images);
            assert_eq!(
                shape(&decorated),
                shape(&plain),
                "{markup} must not move a single wrap point"
            );
        }

        let with_image = layout_info_text(
            &font,
            "{{TEST}}supercalifragilistic expialidocious",
            width,
            &images,
        );
        assert_ne!(
            shape(&with_image),
            shape(&plain),
            "an inline image occupies width and must reflow the row"
        );

        // The same claim where the player actually feels it: the row rectangle
        // the pointer is tested against.
        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Information", 0, 2);
            AddMenuItem("Row", "", MENU, this(), 0, 0, "plain instruction text");
        }
        "#;
        let menu = script_menu_fixture("MENU", "Menu", script);
        let area = Rect::new(0, 0, 320, 200);
        let rect_for = |caption: &str| {
            let mut variant = menu.clone();
            variant.items[0].info_caption = caption.to_string();
            engine_script_menu_layout_with_images(area, &font, &variant, false, &images, None)
                .item_rect(0)
        };
        let plain_rect = rect_for("plain instruction text").expect("the row is laid out");
        for markup in [
            "<i>plain instruction text</i>",
            "<i><i>plain instruction text</i></i>",
            "<c 00ff00>plain instruction text</c>",
        ] {
            assert_eq!(
                rect_for(markup),
                Some(plain_rect),
                "{markup} must leave the row's hit rectangle identical"
            );
        }
        // The rectangle is content-derived, not a fixed grid cell, so the
        // equalities above are a real constraint rather than a tautology.
        assert_ne!(
            rect_for("{{TEST}}plain instruction text"),
            Some(plain_rect),
            "an inline image widens the row, proving the rect tracks content"
        );
    }
}
