use std::collections::HashMap;

use lc_engine::{
    CommandKind, ContextMenuEntry, ControlCommand, DefinitionPictureImage, Engine, ObjectId,
    ObjectMenuExtra, ObjectMenuSymbol, SimulationSnapshot, OWNER_NONE,
};
use lc_frontend::{
    default_owner_color,
    hud::{draw_command_image_cell, HudFont},
    CommandImage, CommandOverlayIcon, GuiPoint,
};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{Color, Rect, Surface, TextFont};
use lc_gui::ImageData;

use crate::ingame_menu::{
    draw_caption_bar, draw_command_key, draw_image_region_aspect, draw_ok_cancel, draw_3d_frame,
    draw_tooltip, IngameMenuGraphics,
};

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
const CLASSIC_TITLE_HEIGHT: i32 = 23;
const CLASSIC_BG_ALPHA: u8 = 255 - 0x5f;
const CLASSIC_SELECTION_COLOR: Color = Color::opaque(0xc8, 0, 0);
const CLASSIC_EXTRA_FRAME_COLOR: Color = Color::opaque(0x44, 0, 0);
const CLASSIC_CAPTION_COLOR: Color = Color::opaque(0xff, 0xff, 0xff);
const CLASSIC_CLOSE_ICON: u8 = 34;
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

impl MenuEntry for lc_engine::ObjectMenuItem {
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

fn engine_script_menu_title(menu: &lc_engine::ObjectMenuState) -> String {
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
        title.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EngineScriptMenuLayout {
    pub bounds: Rect,
    pub title: Rect,
    pub client_x: i32,
    pub client_y: i32,
    pub columns: i32,
    pub lines: i32,
    pub item_width: i32,
    pub item_height: i32,
    pub first_index: usize,
    pub visible: usize,
}

impl EngineScriptMenuLayout {
    pub fn item_rect(self, index: usize) -> Option<Rect> {
        let slot = index.checked_sub(self.first_index)?;
        if slot >= self.visible {
            return None;
        }
        let slot = i32::try_from(slot).ok()?;
        Some(Rect::new(
            self.client_x + (slot % self.columns) * self.item_width,
            self.client_y + (slot / self.columns) * self.item_height,
            self.item_width as u32,
            self.item_height as u32,
        ))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EngineScriptMenuPointerTarget {
    Close,
    Item(usize),
    Background,
}

fn rect_contains_point(rect: Rect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.width as i32) as f32
        && point.y < (rect.y + rect.height as i32) as f32
}

pub(crate) fn engine_script_menu_pointer_target(
    area: Rect,
    font: &HudFont<'_>,
    menu: &lc_engine::ObjectMenuState,
    show_commands: bool,
    show_close_button: bool,
    point: GuiPoint,
) -> Option<EngineScriptMenuPointerTarget> {
    if !matches!(menu.style, 0 | 1) {
        return None;
    }
    let layout = engine_script_menu_layout(area, font, menu, show_commands);
    if show_close_button && rect_contains_point(layout.close_button_rect(), point) {
        return Some(EngineScriptMenuPointerTarget::Close);
    }
    let item = menu
        .items
        .iter()
        .enumerate()
        .skip(layout.first_index)
        .take(layout.visible)
        .find_map(|(index, _)| {
            rect_contains_point(layout.item_rect(index)?, point)
                .then_some(EngineScriptMenuPointerTarget::Item(index))
        });
    item.or_else(|| {
        rect_contains_point(layout.bounds, point)
            .then_some(EngineScriptMenuPointerTarget::Background)
    })
}

fn apply_default_menu_owner_color(pixels: &mut [u8], mask: &[u8]) {
    // C4Def::Picture2Facet calls Graphics.GetBitmap(0); C4Surface::SetClr
    // maps zero to 0xff, the engine's default blue owner color
    // (C4Def.cpp:1374-1378; C4DefGraphics.h:49; C4Surface.h:110).
    for (index, mask_value) in mask.iter().copied().enumerate() {
        let offset = index * 4;
        let Some(pixel) = pixels.get_mut(offset..offset + 4) else {
            break;
        };
        let mask = u16::from(mask_value);
        if mask == 0 {
            continue;
        }
        let inverse = 255_u16 - mask;
        pixel[0] = (u16::from(pixel[0]) * inverse / 255) as u8;
        pixel[1] = (u16::from(pixel[1]) * inverse / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * inverse + 255 * mask) / 255) as u8;
    }
}

pub(crate) fn definition_menu_picture(image: DefinitionPictureImage) -> ImageData {
    let width = image.width();
    let height = image.height();
    let mask = image.color_mask();
    let pixels = image.into_pixels();
    let Some(mask) = mask else {
        return ImageData::from_arc(width, height, pixels);
    };
    let mut pixels = pixels.to_vec();
    apply_default_menu_owner_color(&mut pixels, &mask);
    ImageData::new(width, height, pixels)
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
        ObjectMenuSymbol::Info => CommandImage::InfoMenu { picture },
        ObjectMenuSymbol::Exit => CommandImage::Exit,
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
    components: &[lc_engine::ObjectMenuComponent],
) -> Vec<ComponentFooterCell> {
    // C4IDList::Draw asks GetSectionCount before applying
    // Right|Triple|Half to each TruncateSection. Keep that slightly unusual
    // capacity rule: a 16px-high strip reports width/16 sections, while each
    // actual component cell consumes 16*3/2 = 24 pixels from the right
    // (C4IDList.cpp:207-227; C4Facet.cpp:38-42,182-213).
    let section_count = if remaining.height == 0 {
        0
    } else {
        remaining.width / remaining.height
    } as usize;
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

pub(crate) fn engine_script_menu_layout(
    area: Rect,
    font: &HudFont<'_>,
    menu: &lc_engine::ObjectMenuState,
    show_commands: bool,
) -> EngineScriptMenuLayout {
    // Normal menus are a fixed 35px icon grid. Context menus are compact
    // captioned rows: height=max(C4MN_SymbolSize, FontRegular), width is
    // the widest title/item text plus its square symbol (C4Menu.cpp:
    // 642-665). InitMenu normally gives Context one column (:359-365).
    let columns = menu.columns.max(1);
    let (item_width, item_height) = if menu.style == 1 {
        let item_height = font.line_height().max(CLASSIC_COMMAND_HEIGHT);
        let title_width = font
            .text_width(&menu.caption)
            .saturating_add(item_height)
            .saturating_add(CLASSIC_COMMAND_HEIGHT);
        let item_width = menu
            .items
            .iter()
            .map(|item| font.text_width(&item.caption).saturating_add(item_height))
            .fold(title_width, i32::max)
            .saturating_add(3);
        (item_width.max(1), item_height.max(1))
    } else {
        (CLASSIC_ITEM_SIZE, CLASSIC_ITEM_SIZE)
    };
    let item_count = i32::try_from(menu.items.len()).unwrap_or(i32::MAX);
    let natural_lines = (item_count / columns) + i32::from(item_count % columns != 0);
    let max_lines = ((area.height as i32 - 100) / item_height).max(1);
    let lines = natural_lines.max(1).min(max_lines);
    let title_height = font.line_height().max(CLASSIC_TITLE_HEIGHT);
    let command_height = i32::from(show_commands || menu.extra != ObjectMenuExtra::None)
        * CLASSIC_COMMAND_HEIGHT;
    let width = columns * item_width + 2 * CLASSIC_FRAME_WIDTH;
    let height = lines * item_height
        + title_height
        + command_height
        + CLASSIC_FRAME_WIDTH;

    // Default C4Menu alignment is Right|Bottom with one C4SymbolSize (35)
    // below and two at the right (C4Menu.cpp:298, 727-745).
    let mut x = area.width as i32 - 2 * CLASSIC_ITEM_SIZE - width;
    let mut y = area.height as i32 - CLASSIC_ITEM_SIZE - height;
    if width > area.width as i32 - 2 * CLASSIC_ITEM_SIZE {
        x = (area.width as i32 - width) / 2;
    }
    if height > area.height as i32 - 2 * CLASSIC_ITEM_SIZE {
        y = (area.height as i32 - height) / 2;
    }
    x += area.x;
    y += area.y;

    let visible = (columns * lines) as usize;
    let first_index = usize::try_from(menu.selection)
        .ok()
        .filter(|selection| *selection >= visible && lines > 1)
        .map(|selection| {
            ((selection / columns as usize) + 1 - lines as usize) * columns as usize
        })
        .unwrap_or(0);
    EngineScriptMenuLayout {
        bounds: Rect::new(x, y, width as u32, height as u32),
        title: Rect::new(x, y, width as u32, title_height as u32),
        client_x: x + CLASSIC_FRAME_WIDTH,
        client_y: y + title_height,
        columns,
        lines,
        item_width,
        item_height,
        first_index,
        visible,
    }
}

/// Draws a script-created `C4ObjectMenu` from the engine's live runtime
/// state. The engine remains the sole owner of selection and item state; this
/// is deliberately a read-only presentation view.
pub fn render_engine_script_menu(
    surface: &mut Surface,
    area: Rect,
    font: &HudFont<'_>,
    fallback_font: &dyn TextFont,
    tiny_font: Option<&HudFont<'_>>,
    menu: &lc_engine::ObjectMenuState,
    gfx: &IngameMenuGraphics,
    title_icon: Option<&ImageData>,
    item_icons: &[Option<ImageData>],
    selected_component_icons: &[Option<ImageData>],
    show_close_button: bool,
    time_on_selection: u32,
) {
    if surface.width() == 0 || surface.height() == 0 || area.width == 0 || area.height == 0 {
        return;
    }

    let selected = usize::try_from(menu.selection)
        .ok()
        .filter(|selection| *selection < menu.items.len());
    if matches!(menu.style, 0 | 1) {
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
        );
        return;
    }
    ObjectMenuState::render_entries(
        surface,
        fallback_font,
        &menu.items,
        selected,
        &engine_script_menu_title(menu),
        "No menu entries.",
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_engine_normal_menu(
    surface: &mut Surface,
    area: Rect,
    font: &HudFont<'_>,
    tiny_font: Option<&HudFont<'_>>,
    menu: &lc_engine::ObjectMenuState,
    gfx: &IngameMenuGraphics,
    title_icon: Option<&ImageData>,
    item_icons: &[Option<ImageData>],
    selected_component_icons: &[Option<ImageData>],
    selected: Option<usize>,
    show_close_button: bool,
    time_on_selection: u32,
) {
    let layout = engine_script_menu_layout(area, font, menu, gfx.show_commands);
    let bounds = layout.bounds;
    let x = bounds.x;
    let y = bounds.y;
    let width = bounds.width as i32;
    let height = bounds.height as i32;
    let title_height = layout.title.height as i32;
    let columns = layout.columns;

    fill_rect(
        surface,
        bounds,
        Color::new(0, 0, 0, CLASSIC_BG_ALPHA),
    );
    draw_3d_frame(surface, bounds);

    let title_rect = layout.title;
    if let Some(caption_bar) = gfx.caption_bar.as_ref() {
        draw_caption_bar(surface, title_rect, caption_bar);
    }
    let icon_indent = if menu.title_symbol == ObjectMenuSymbol::Definition {
        title_icon.map_or(0, |icon| {
            let side = (title_height - 2) as u32;
            draw_image_region_aspect(
                surface,
                icon,
                Rect::new(0, 0, icon.width(), icon.height()),
                Rect::new(x + 1, y + 1, side, side),
                false,
            );
            title_height
        })
    } else {
        let side = (title_height - 2) as u32;
        let image = command_image_for_menu_symbol(
            menu.title_symbol,
            title_icon.cloned(),
            gfx,
        );
        draw_command_image_cell(
            surface,
            &gfx.hud,
            Rect::new(x + 1, y + 1, side, side),
            &image,
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
        x + icon_indent,
        title_rect.y,
        title_text_right.saturating_sub(x + icon_indent).max(0) as u32,
        title_rect.height,
    );
    let nested_clip = previous_clip
        .map(|clip| {
            clip.intersection(title_text_clip)
                .unwrap_or(Rect::new(0, 0, 0, 0))
        })
        .unwrap_or(title_text_clip);
    surface.set_clip(nested_clip);
    font.draw(
        surface,
        x + icon_indent + 5,
        y + (title_height - font.line_height()) / 2 - 1,
        &engine_script_menu_title(menu),
        CLASSIC_CAPTION_COLOR,
        TextAlign::Left,
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
            );
        }
    }

    let client_x = layout.client_x;
    let client_y = layout.client_y;
    let visible = layout.visible;
    let first_index = layout.first_index;
    for (slot, (index, item)) in menu
        .items
        .iter()
        .enumerate()
        .skip(first_index)
        .take(visible)
        .enumerate()
    {
        let cell_x = client_x + (slot as i32 % columns) * layout.item_width;
        let cell_y = client_y + (slot as i32 / columns) * layout.item_height;
        let cell = Rect::new(
            cell_x,
            cell_y,
            layout.item_width as u32,
            layout.item_height as u32,
        );
        if selected == Some(index) && menu.text_progress != Some(0) {
            fill_rect(surface, cell, CLASSIC_SELECTION_COLOR);
        }
        let symbol_cell = if menu.style == 1 {
            Rect::new(
                cell_x,
                cell_y,
                layout.item_height as u32,
                layout.item_height as u32,
            )
        } else {
            cell
        };
        let picture = item_icons.get(index).cloned().flatten();
        let image = command_image_for_menu_symbol(item.symbol, picture, gfx);
        draw_command_image_cell(surface, &gfx.hud, symbol_cell, &image);
        if menu.style == 1 {
            font.draw(
                surface,
                cell_x + layout.item_height,
                cell_y,
                &item.caption,
                CLASSIC_CAPTION_COLOR,
                TextAlign::Left,
            );
        }
        if item.count != MENU_ITEM_NO_COUNT {
            font.draw(
                surface,
                cell_x + layout.item_width - 1,
                cell_y + layout.item_height - 1 - font.line_height(),
                &format!("{}x", item.count),
                CLASSIC_CAPTION_COLOR,
                TextAlign::Right,
            );
        }
    }

    if gfx.show_commands || menu.extra != ObjectMenuExtra::None {
        let extra = Rect::new(
            x + 1,
            y + height - CLASSIC_COMMAND_HEIGHT - 1,
            (width - 2) as u32,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        draw_border(surface, extra, CLASSIC_EXTRA_FRAME_COLOR);
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
                );
            }
            if let Some(cell) = truncate_control() {
                draw_ok_cancel(surface, gfx, cell.x, cell.y, cell.width, 0, 0);
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
                    );
                }
                if let Some(cell) = truncate_control() {
                    draw_ok_cancel(surface, gfx, cell.x, cell.y, cell.width, 2, 1);
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
                );
            }
            if let Some(cell) = truncate_control() {
                if menu
                    .items
                    .iter()
                    .any(|item| item.symbol == ObjectMenuSymbol::Exit)
                {
                    draw_command_image_cell(surface, &gfx.hud, cell, &CommandImage::Exit);
                } else {
                    draw_ok_cancel(surface, gfx, cell.x, cell.y, cell.width, 1, 0);
                }
            }
        }
        if menu.extra == ObjectMenuExtra::Components {
            if let Some(components) = selected
                .and_then(|selection| menu.items.get(selection))
                .map(|item| item.components.as_slice())
            {
                for cell in component_footer_cells(remaining, components) {
                    let picture = selected_component_icons
                        .get(cell.component_index)
                        .cloned()
                        .flatten();
                    draw_command_image_cell(
                        surface,
                        &gfx.hud,
                        cell.rect,
                        &CommandImage::Picture(picture),
                    );
                    font.draw(
                        surface,
                        cell.rect.x + cell.rect.width as i32 - 1,
                        cell.rect.y + cell.rect.height as i32 - 1 - font.line_height(),
                        &cell.count_label,
                        CLASSIC_CAPTION_COLOR,
                        TextAlign::Right,
                    );
                }
            }
        } else if menu.extra == ObjectMenuExtra::Value {
            if let Some(value) = selected
                .and_then(|selection| menu.items.get(selection))
                .and_then(|item| item.value)
            {
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
                    );
                }
                font.draw(
                    surface,
                    right,
                    remaining.y,
                    &value,
                    CLASSIC_CAPTION_COLOR,
                    TextAlign::Right,
                );
            }
        }
    }

    if time_on_selection >= 90 {
        if let Some((selection, item)) = selected
            .and_then(|selection| menu.items.get(selection).map(|item| (selection, item)))
            .filter(|(_, item)| !item.info_caption.is_empty())
        {
            let slot = selection.saturating_sub(first_index);
            let cell_x = client_x + (slot as i32 % columns) * layout.item_width;
            let cell_y = client_y + (slot as i32 / columns) * layout.item_height;
            draw_tooltip(surface, font, cell_x, cell_y, &item.info_caption);
        }
    }
}

#[derive(Clone, Debug)]
struct ContextMenuItem {
    entry: ContextMenuEntry,
}

impl ContextMenuItem {
    fn new(entry: ContextMenuEntry) -> Self {
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
enum MenuMode {
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
        let width = surface.width() as i32;
        let height = surface.height() as i32;
        if width <= 0 || height <= 0 {
            return;
        }

        fill_rect(surface, surface.bounds(), BACKDROP_COLOR);

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
                    );
                } else {
                    let empty: &[ObjectMenuItem] = &[];
                    Self::render_entries(surface, font, empty, None, &title, empty_message, hint);
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
                );
            }
        }
    }

    fn render_entries<E: MenuEntry>(
        surface: &mut Surface,
        font: &dyn TextFont,
        items: &[E],
        selected: Option<usize>,
        title: &str,
        empty_message: &str,
        hint: Option<&str>,
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
        fill_rect(surface, panel_rect, PANEL_COLOR);
        draw_border(surface, panel_rect, PANEL_BORDER);

        let mut cursor_y = panel_y + PANEL_PADDING;
        font.draw_text(
            surface,
            (panel_x + PANEL_PADDING) as f32,
            cursor_y as f32,
            title,
            TITLE_FONT_SIZE,
            TITLE_COLOR,
        );

        cursor_y += TITLE_GAP;
        if items.is_empty() {
            font.draw_text(
                surface,
                (panel_x + PANEL_PADDING) as f32,
                (cursor_y + 10) as f32,
                empty_message,
                ITEM_FONT_SIZE,
                MUTED_TEXT_COLOR,
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
                fill_rect(surface, row_rect, HIGHLIGHT_COLOR);
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
                draw_menu_icon(surface, icon_rect, icon);
                text_x = icon_rect.x + icon_rect.width as i32 + 8;
            }
            font.draw_text(
                surface,
                text_x as f32,
                (row_rect.y + 8) as f32,
                &label_text,
                ITEM_FONT_SIZE,
                primary_color,
            );

            if let Some(description) = item.description() {
                font.draw_text(
                    surface,
                    text_x as f32,
                    (row_rect.y + 22) as f32,
                    description,
                    DETAIL_FONT_SIZE,
                    MUTED_TEXT_COLOR,
                );
            }

            cursor_y += ITEM_HEIGHT + ITEM_SPACING;
        }

        if let Some(hint) = hint {
            font.draw_text(
                surface,
                (panel_x + PANEL_PADDING) as f32,
                (panel_y + panel_height - PANEL_PADDING - 18) as f32,
                hint,
                DETAIL_FONT_SIZE,
                MUTED_TEXT_COLOR,
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
    crew: &lc_engine::ObjectSnapshot,
) -> Vec<ObjectMenuItem> {
    if crew.contents.is_empty() {
        return Vec::new();
    }
    collect_contents(engine, snapshot, &crew.contents)
}

fn collect_container_items(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    container: &lc_engine::ObjectSnapshot,
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
    crew: &lc_engine::ObjectSnapshot,
) -> Vec<ContextMenuItem> {
    match engine.context_menu_entries(crew.id) {
        Ok(entries) => entries.into_iter().map(ContextMenuItem::new).collect(),
        Err(err) => {
            tracing::warn!(object = ?crew.id, error = ?err, "failed to build context menu");
            Vec::new()
        }
    }
}

fn collect_build_items(engine: &Engine, crew: &lc_engine::ObjectSnapshot) -> Vec<BuildMenuItem> {
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
    engine.definition_picture_image(definition_id).map(|image| {
        let width = image.width();
        let height = image.height();
        ImageData::from_arc(width, height, image.into_pixels())
    })
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

fn draw_menu_icon(surface: &mut Surface, rect: Rect, icon: &ImageData) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let bounds = surface.bounds();
    let src_width = icon.width();
    let src_height = icon.height();
    if src_width == 0 || src_height == 0 {
        return;
    }
    let pixels = icon.pixels();
    for dy in 0..rect.height {
        let target_y = rect.y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }
        let src_y = ((dy as u64) * src_height as u64 / rect.height as u64) as u32;
        for dx in 0..rect.width {
            let target_x = rect.x + dx as i32;
            if target_x < bounds.x || target_x >= bounds.x + bounds.width as i32 {
                continue;
            }
            let src_x = ((dx as u64) * src_width as u64 / rect.width as u64) as u32;
            let idx = ((src_y * src_width + src_x) * 4) as usize;
            if idx + 3 >= pixels.len() {
                continue;
            }
            let color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );
            if color.a == 0 {
                continue;
            }
            let result = if color.a == 255 {
                surface.set_pixel(target_x as u32, target_y as u32, color)
            } else {
                surface.blend_pixel(target_x as u32, target_y as u32, color)
            };
            if result.is_err() {
                break;
            }
        }
    }
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color) {
    if let Some(clipped) = rect.intersection(surface.bounds()) {
        for y in clipped.y..(clipped.y + clipped.height as i32) {
            for x in clipped.x..(clipped.x + clipped.width as i32) {
                let result = if color.a == 255 {
                    surface.set_pixel(x as u32, y as u32, color)
                } else {
                    surface.blend_pixel(x as u32, y as u32, color)
                };
                if result.is_err() {
                    break;
                }
            }
        }
    }
}

fn draw_border(surface: &mut Surface, rect: Rect, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let top = Rect::new(rect.x, rect.y, rect.width, 1);
    let bottom = Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1);
    let left = Rect::new(rect.x, rect.y, 1, rect.height);
    let right = Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height);
    fill_rect(surface, top, color);
    fill_rect(surface, bottom, color);
    fill_rect(surface, left, color);
    fill_rect(surface, right, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{
        CommandStackSnapshot, Definition, Engine, MovementProfile, ObjectSnapshot, ObjectStatus,
        PlayerConfig, SpawnConfig, Vector2,
    };
    use std::collections::HashMap;

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
            construction: lc_engine::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
            action: Default::default(),
            direction: Default::default(),
            command_direction: Default::default(),
            action_procedure: None,
            effects: Vec::new(),
            vertices: Vec::new(),
            own_vertices: None,
            container: None,
            layer: None,
            blit_mode: 0,
            contents: Vec::new(),
            components: HashMap::new(),
            status: ObjectStatus::Normal,
            owner: 1,
            controller: 1,
            category: 0,
            crew_member: false,
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
        players: Vec<lc_engine::PlayerState>,
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
            frame: 0,
            game_time: 0,
            game_over: false,
            round_results: Default::default(),
            physics: None,
            objects,
            environment: Default::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            script_globals: Default::default(),
            particles: Vec::new(),
            players,
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners: Vec::new(),
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: lc_engine::LcgRng::seed_from_u64(42),
            surfaces: Vec::new(),
            hud: Default::default(),
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            transfer_zones: Vec::new(),
            menu_requests: Vec::new(),
            audio: Vec::new(),
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

    #[test]
    fn engine_script_menu_command_strip_uses_cpp_menu_symbol_height() {
        // DrawMenuControls reserves C4MN_SymbolSize (16px), not the
        // unrelated 35px C4SymbolSize used by normal-menu items
        // (src/C4Menu.h:32-35,262; src/C4Menu.cpp:843-880).
        assert_eq!(CLASSIC_COMMAND_HEIGHT, 16);
    }

    #[test]
    fn engine_script_context_menu_uses_classic_geometry_and_pointer_targeting() {
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
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_script("MENU", "Menu", script).expect("script compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("MENU"))
            .expect("menu object spawns");
        let menu = engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("Initialize created its menu");
        assert_eq!(menu.style, 1);
        assert_eq!(menu.columns, 1);

        let font = lc_graphics::BitmapFont::new();
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
        assert_eq!(layout.bounds.width, (item_width + 2 * CLASSIC_FRAME_WIDTH) as u32);
        let point = GuiPoint::new(item.x as f32 + 1.0, item.y as f32 + 1.0);
        assert_eq!(
            engine_script_menu_pointer_target(
                area,
                &hud_font,
                &menu,
                false,
                true,
                point,
            ),
            Some(EngineScriptMenuPointerTarget::Item(0))
        );

        let gfx = IngameMenuGraphics::default();
        let mut surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
    fn engine_context_menu_draws_cpp_composite_symbols_and_exit_close_icon() {
        // C4ObjectMenu::RefillInternal composes carried picture+Hand(0) for
        // Put, a definition picture for Contents, DrawMenuSymbol for
        // Buy/Sell, target+OKCancel(0,1) for Info and fctExit for Exit
        // (src/C4ObjectMenu.cpp:335-427;
        // src/C4Menu.cpp:43-70). AutoContextMenu's close command contains
        // "Exit", so the command strip also draws fctExit (:874-880).
        fn solid(width: u32, height: u32, rgba: [u8; 4]) -> ImageData {
            ImageData::new(width, height, rgba.repeat((width * height) as usize))
        }

        fn contains_color(surface: &Surface, rect: Rect, color: Color) -> bool {
            (rect.y..rect.y + rect.height as i32).any(|y| {
                (rect.x..rect.x + rect.width as i32).any(|x| {
                    surface.get_pixel(x as u32, y as u32) == Some(color)
                })
            })
        }

        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Hut", 0, 1);
            AddMenuItem("Contents", "Choose()", MENU, this());
        }
        "#;
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_script("MENU", "Menu", script).expect("script compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("MENU"))
            .expect("menu object spawns");
        let mut menu = engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("Initialize created its menu");
        let template = menu.items[0].clone();
        let make_item = |caption: &str,
                         item_id: &str,
                         symbol: lc_engine::ObjectMenuSymbol| {
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
            make_item("Put", "HUT3", lc_engine::ObjectMenuSymbol::Put),
            make_item(
                "Contents",
                "HUT3",
                lc_engine::ObjectMenuSymbol::Definition,
            ),
            make_item(
                "Buy",
                "NONE",
                lc_engine::ObjectMenuSymbol::Buy { owner: 7 },
            ),
            make_item(
                "Sell",
                "NONE",
                lc_engine::ObjectMenuSymbol::Sell { owner: 7 },
            ),
            make_item("Info", "HUT3", lc_engine::ObjectMenuSymbol::Info),
            make_item("Exit", "NONE", lc_engine::ObjectMenuSymbol::Exit),
        ];

        let gray = Color::opaque(80, 80, 80);
        let red = Color::opaque(240, 20, 20);
        let green = Color::opaque(20, 220, 20);
        let yellow = Color::opaque(240, 220, 20);
        let magenta = Color::opaque(220, 20, 220);
        let orange = Color::opaque(240, 120, 20);
        let cyan = Color::opaque(20, 220, 220);
        let purple = Color::opaque(160, 20, 220);
        let mut arrow = vec![0_u8; 16 * 8 * 4];
        for y in 0..8 {
            for x in 0..16 {
                let offset = (y * 16 + x) * 4;
                let color = if x < 8 { yellow } else { magenta };
                arrow[offset..offset + 4]
                    .copy_from_slice(&[color.r, color.g, color.b, color.a]);
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
            hud: lc_frontend::HudGraphics {
                flag: Some(solid(8, 8, [0, 0, 255, 255])),
                wealth: Some(solid(8, 8, [green.r, green.g, green.b, green.a])),
                arrow: Some(ImageData::new(16, 8, arrow)),
                exit: Some(solid(8, 8, [cyan.r, cyan.g, cyan.b, cyan.a])),
                hand: Some(solid(
                    8,
                    8,
                    [purple.r, purple.g, purple.b, purple.a],
                )),
                control: Some(control.clone()),
                ..lc_frontend::HudGraphics::default()
            },
            owner_colors: HashMap::from([(7, red)]),
            control: Some(control),
            show_commands: true,
            ..IngameMenuGraphics::default()
        };
        let picture = solid(8, 8, [gray.r, gray.g, gray.b, gray.a]);
        let item_icons = vec![
            Some(picture.clone()),
            Some(picture.clone()),
            None,
            None,
            Some(picture),
            None,
        ];
        let font = lc_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &hud_font, &menu, true);
        let mut surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
            assert!(contains_color(&surface, row(0), color), "missing Put {color:?}");
        }
        assert!(contains_color(&surface, row(1), gray));
        for color in [red, green, yellow] {
            assert!(contains_color(&surface, row(2), color), "missing Buy {color:?}");
        }
        for color in [red, green, magenta] {
            assert!(contains_color(&surface, row(3), color), "missing Sell {color:?}");
        }
        for color in [gray, orange] {
            assert!(contains_color(&surface, row(4), color), "missing Info {color:?}");
        }
        assert!(contains_color(&surface, row(5), cyan));
        let command_strip = Rect::new(
            layout.bounds.x,
            layout.bounds.y + layout.bounds.height as i32 - CLASSIC_COMMAND_HEIGHT,
            layout.bounds.width,
            CLASSIC_COMMAND_HEIGHT as u32,
        );
        assert!(contains_color(&surface, command_strip, cyan));
    }

    #[test]
    fn engine_buy_menu_draws_cpp_title_symbol_and_value_footer() {
        // C4Object::ActivateMenu(C4MN_Buy) supplies a composed Buy title
        // facet and C4MN_Extra_Value (C4Object.cpp:1919-1928). C4Menu keeps
        // one 16px footer whenever Extra is set, independently of command
        // hints, then DrawValue places a 32x16 wealth facet immediately left
        // of the selected value at the footer's right edge
        // (C4Menu.h:248-264; C4Menu.cpp:843-907; C4Facet.cpp:240-260).
        fn solid(width: u32, height: u32, rgba: [u8; 4]) -> ImageData {
            ImageData::new(width, height, rgba.repeat((width * height) as usize))
        }

        fn contains_color(surface: &Surface, rect: Rect, color: Color) -> bool {
            (rect.y..rect.y + rect.height as i32).any(|y| {
                (rect.x..rect.x + rect.width as i32).any(|x| {
                    surface.get_pixel(x as u32, y as u32) == Some(color)
                })
            })
        }

        let script = r#"
        func Initialize()
        {
            CreateMenu(MENU, this(), this(), 0, "Nothing to buy");
            AddMenuItem("Buy Lorry", "Choose()", LORY, this());
        }
        "#;
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_script("MENU", "Menu", script).expect("script compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("MENU"))
            .expect("menu object spawns");
        let mut menu = engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("Initialize created its menu");
        menu.title_symbol = lc_engine::ObjectMenuSymbol::Buy { owner: 7 };
        menu.extra = lc_engine::ObjectMenuExtra::Value;
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
                arrow[offset..offset + 4]
                    .copy_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        }
        let gfx = IngameMenuGraphics {
            hud: lc_frontend::HudGraphics {
                flag: Some(solid(8, 8, [0, 0, 255, 255])),
                wealth: Some(solid(8, 8, [green.r, green.g, green.b, green.a])),
                arrow: Some(ImageData::new(16, 8, arrow)),
                ..lc_frontend::HudGraphics::default()
            },
            owner_colors: HashMap::from([(7, red)]),
            show_commands: false,
            ..IngameMenuGraphics::default()
        };
        let font = lc_graphics::BitmapFont::new();
        let hud_font = HudFont::Fallback(&font);
        let area = Rect::new(0, 0, 640, 480);
        let layout = engine_script_menu_layout(area, &hud_font, &menu, false);
        let mut menu_without_extra = menu.clone();
        menu_without_extra.extra = lc_engine::ObjectMenuExtra::None;
        let layout_without_extra =
            engine_script_menu_layout(area, &hud_font, &menu_without_extra, false);
        assert_eq!(
            layout.bounds.height,
            layout_without_extra.bounds.height + CLASSIC_COMMAND_HEIGHT as u32,
            "C4Menu::GetMarginBottom reserves the footer for Extra without DrawMenuControls"
        );

        let mut surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
                contains_color(&surface, title_symbol, color),
                "missing Buy title component {color:?}"
            );
        }
        assert!(
            !contains_color(&surface, title_symbol, magenta),
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
        assert!(contains_color(&surface, wealth, green));
        let value_text = Rect::new(
            footer.x + footer.width as i32 - 1 - value_width,
            footer.y,
            value_width.max(1) as u32,
            footer.height,
        );
        assert!(contains_color(
            &surface,
            value_text,
            CLASSIC_CAPTION_COLOR
        ));
    }

    #[test]
    fn engine_components_menu_draws_cached_requirements_right_to_left() {
        // C4MenuItem snapshots the selected definition's components, then
        // C4MN_Extra_Components draws those C4IDList entries from the footer's
        // right edge in stored order. Right|Triple|Half turns each 16px-high
        // section into a 24x16 cell; counts are literal "Nx" labels at the
        // cell's bottom-right (C4Menu.cpp:92-97,843-899;
        // C4IDList.cpp:207-227; C4Facet.cpp:182-213).
        fn solid(width: u32, height: u32, rgba: [u8; 4]) -> ImageData {
            ImageData::new(width, height, rgba.repeat((width * height) as usize))
        }

        fn contains_color(surface: &Surface, rect: Rect, color: Color) -> bool {
            (rect.y..rect.y + rect.height as i32).any(|y| {
                (rect.x..rect.x + rect.width as i32).any(|x| {
                    surface.get_pixel(x as u32, y as u32) == Some(color)
                })
            })
        }

        let script = r#"
        func Initialize()
        {
            CreateMenu(CXCN, this(), this(), 1, "No construction plans available");
            AddMenuItem("Construction: Elevator", "CreateConstructionSite", ELEV, this());
        }
        "#;
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_script("CXCN", "Construction", script).expect("script compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("CXCN"))
            .expect("menu object spawns");
        let mut menu = engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("Initialize created its menu");
        menu.extra = lc_engine::ObjectMenuExtra::Components;
        menu.selection = 0;
        menu.items[0].components = vec![
            lc_engine::ObjectMenuComponent {
                definition_id: "WOOD".to_string(),
                count: 4,
            },
            lc_engine::ObjectMenuComponent {
                definition_id: "METL".to_string(),
                count: 2,
            },
        ];

        let font = lc_graphics::BitmapFont::new();
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
            Some(solid(8, 8, [wood.r, wood.g, wood.b, wood.a])),
            Some(solid(8, 8, [metal.r, metal.g, metal.b, metal.a])),
        ];
        let gfx = IngameMenuGraphics {
            show_commands: true,
            ..IngameMenuGraphics::default()
        };
        let mut surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
        assert!(contains_color(&surface, metal_cell, metal));
        assert!(!contains_color(&surface, metal_cell, wood));
        assert!(contains_color(&surface, wood_cell, wood));
        assert!(!contains_color(&surface, wood_cell, metal));
        assert!(
            contains_color(&surface, metal_cell, CLASSIC_CAPTION_COLOR),
            "METL cell must overlay the white literal 2x count"
        );
        assert!(
            contains_color(&surface, wood_cell, CLASSIC_CAPTION_COLOR),
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
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_script("MENU", "Menu", script).expect("script compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("MENU"))
            .expect("menu object spawns");
        let menu = engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .expect("Initialize created its menu");

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

        let font = lc_graphics::BitmapFont::new();
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
        let layout = engine_script_menu_layout(
            Rect::new(0, 0, 640, 480),
            &hud_font,
            &menu,
            true,
        );
        assert_eq!(CLASSIC_COMMAND_HEIGHT, 16);
        assert_eq!(layout.bounds, Rect::new(391, 369, 179, 76));
        assert_eq!(layout.item_rect(0), Some(Rect::new(393, 392, 35, 35)));
        assert_eq!(
            engine_script_menu_pointer_target(
                Rect::new(0, 0, 640, 480),
                &hud_font,
                &menu,
                true,
                true,
                GuiPoint::new(411.0, 393.0),
            ),
            Some(EngineScriptMenuPointerTarget::Item(0)),
            "pointer hit cells must move with the C++-sized command strip"
        );
        let mut surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
        let narrow_layout = engine_script_menu_layout(
            Rect::new(0, 0, 640, 480),
            &hud_font,
            &narrow_menu,
            true,
        );
        let mut overflow_surface = Surface::new(640, 480, lc_graphics::PixelFormat::Rgba8888);
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
    }
}
