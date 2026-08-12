//! The viewport context menu (`C4EditCursor::DoContextMenu`,
//! `src/C4EditCursor.cpp:583-628`).
//!
//! C++ has two bodies for this menu and no third: `TrackPopupMenu` over
//! `IDR_CONTEXTMENUS`' `VIEWPORT` popup under `_WIN32`, a `GtkMenu` under
//! `WITH_DEVELOPER_MODE`, and past the `#endif` nothing at all. The reference
//! build compiles neither, so on macOS the whole menu — and with it Delete,
//! Duplicate and Grab contents — simply does not exist.
//!
//! Both bodies agree on the part that *is* portable, so that is what this
//! module keeps: the item set and its order (`res/engine.rc:287-295`,
//! `C4EditCursor.cpp:88-97`), which entry is enabled, and the caption the
//! Properties row swaps by mode. The enablement itself is not decided here —
//! [`clonk_engine::developer_cursor::context_menu`] owns it.
//!
//! The presentation is invented, because there is nothing to port it from. It
//! is drawn in the same Win9x chrome as the console shell
//! ([`crate::developer_chrome`]) and lands on the viewport's own framebuffer,
//! since a winit window cannot host an OS popup.

use clonk_engine::developer_cursor::{CursorContextMenu, PropertiesCaption};
use clonk_graphics::{Surface, TextFont};

use crate::classic_gui::IntRect;
use crate::developer_chrome::{
    contains, draw_fitted_text, draw_raised, fill, CONTROL_TEXT, DISABLED_TEXT, MENU_ITEM_HEIGHT,
    MENU_SEPARATOR_HEIGHT, MID_EDGE, SELECTED_BACKGROUND, SELECTED_TEXT, SMALL_FONT_SIZE,
    WINDOW_BACKGROUND,
};
use crate::GuiPoint;

/// The four commands the menu can issue (`IDM_VIEWPORT_*`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewportContextItem {
    /// `IDM_VIEWPORT_DELETE` -> `C4EditCursor::Delete`.
    Delete,
    /// `IDM_VIEWPORT_DUPLICATE` -> `C4EditCursor::Duplicate`.
    Duplicate,
    /// `IDM_VIEWPORT_CONTENTS` -> `C4EditCursor::GrabContents`.
    GrabContents,
    /// `IDM_VIEWPORT_PROPERTIES` -> `C4EditCursor::OpenPropTools`.
    Properties,
}

/// The resource strings the menu draws. The host resolves them, so the view
/// never reaches for a resource table of its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewportContextLabels {
    /// `IDS_MNU_DELETE`.
    pub delete: String,
    /// `IDS_MNU_DUPLICATE`.
    pub duplicate: String,
    /// `IDS_MNU_CONTENTS`.
    pub contents: String,
    /// `IDS_CNS_PROPERTIES`, shown in Edit mode.
    pub properties: String,
    /// `IDS_CNS_TOOLS`, shown in Play and Draw mode.
    pub tools: String,
}

impl Default for ViewportContextLabels {
    fn default() -> Self {
        Self {
            delete: "Delete".to_owned(),
            duplicate: "Duplicate".to_owned(),
            contents: "Grab contents".to_owned(),
            properties: "Properties".to_owned(),
            tools: "Tools".to_owned(),
        }
    }
}

/// One row: a command or the separator above Properties.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewportContextEntry {
    Item {
        item: ViewportContextItem,
        label: String,
        enabled: bool,
    },
    Separator,
}

impl ViewportContextEntry {
    /// The command this row issues, if it is not the separator.
    pub fn item(&self) -> Option<ViewportContextItem> {
        match self {
            Self::Item { item, .. } => Some(*item),
            Self::Separator => None,
        }
    }

    fn enabled(&self) -> bool {
        matches!(self, Self::Item { enabled: true, .. })
    }
}

/// One row's placement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportContextEntryLayout {
    pub index: usize,
    pub rect: IntRect,
}

/// What a click did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewportContextOutcome {
    /// Run this command and close the menu.
    Activate(ViewportContextItem),
    /// The menu swallowed the click and stays up. A greyed row does this in
    /// both toolkits: `MF_GRAYED` absorbs the click without dismissing the
    /// popup, and a GTK insensitive item never emits `activate` — only a
    /// click *outside* cancels.
    Ignored,
    /// Close the menu without running anything — a click outside it.
    Dismiss,
}

/// An open viewport context menu.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportContextMenu {
    entries: Vec<ViewportContextEntry>,
    /// Where the popup's top-left corner was asked for, in window-local
    /// pixels. `TPM_LEFTALIGN | TPM_TOPALIGN` at the cursor
    /// (`C4EditCursor.cpp:597-600`).
    origin: (i32, i32),
    width: i32,
    pointer: Option<GuiPoint>,
}

/// `TrackPopupMenu`'s own minimum: a popup narrower than its longest label is
/// useless, and one wider than the window cannot be clicked.
const MIN_WIDTH: i32 = 140;
/// Per-character advance for the width estimate, matched to the console's own
/// dropdown metric so the two menus size alike.
const LABEL_ADVANCE: i32 = 8;
const LABEL_PADDING: i32 = 30;

impl ViewportContextMenu {
    /// Build the menu `DoContextMenu` would pop up.
    ///
    /// `enablement` is [`clonk_engine::developer_cursor::context_menu`]'s
    /// answer, so the four `SetMenuItemEnable` calls (`:588-591`) and the
    /// caption swap (`:595`) are the port's, not re-derived here.
    pub fn new(
        enablement: CursorContextMenu,
        labels: &ViewportContextLabels,
        origin: (i32, i32),
    ) -> Self {
        let properties_label = match enablement.properties_caption {
            PropertiesCaption::Properties => labels.properties.clone(),
            PropertiesCaption::Tools => labels.tools.clone(),
        };
        // res/engine.rc:289-294 and C4EditCursor.cpp:93-97 build the same five
        // rows in the same order, separator included.
        let entries = vec![
            ViewportContextEntry::Item {
                item: ViewportContextItem::Delete,
                label: labels.delete.clone(),
                enabled: enablement.delete,
            },
            ViewportContextEntry::Item {
                item: ViewportContextItem::Duplicate,
                label: labels.duplicate.clone(),
                enabled: enablement.duplicate,
            },
            ViewportContextEntry::Item {
                item: ViewportContextItem::GrabContents,
                label: labels.contents.clone(),
                enabled: enablement.contents,
            },
            ViewportContextEntry::Separator,
            ViewportContextEntry::Item {
                item: ViewportContextItem::Properties,
                label: properties_label,
                enabled: enablement.properties,
            },
        ];
        let longest = entries
            .iter()
            .filter_map(|entry| match entry {
                ViewportContextEntry::Item { label, .. } => Some(label.chars().count()),
                ViewportContextEntry::Separator => None,
            })
            .max()
            .unwrap_or(0);
        Self {
            entries,
            origin,
            width: (longest as i32 * LABEL_ADVANCE + LABEL_PADDING).max(MIN_WIDTH),
            pointer: None,
        }
    }

    pub fn entries(&self) -> &[ViewportContextEntry] {
        &self.entries
    }

    /// The popup's rows, shifted so the whole menu stays inside the window.
    ///
    /// Both toolkits reposition a popup that would fall off the screen; here
    /// the window *is* the screen, because the menu is painted onto the
    /// viewport's framebuffer rather than into a window of its own. A row
    /// outside it could never be clicked.
    pub fn layout(&self, width: u32, height: u32) -> Vec<ViewportContextEntryLayout> {
        let total = self.entries.iter().map(entry_height).sum::<i32>();
        let menu_width = self.width.min(width as i32).max(1);
        let x = self.origin.0.min(width as i32 - menu_width).max(0);
        let y = self.origin.1.min(height as i32 - total).max(0);
        self.entries
            .iter()
            .enumerate()
            .scan(y, |top, (index, entry)| {
                let rect = IntRect {
                    x,
                    y: *top,
                    w: menu_width,
                    h: entry_height(entry),
                };
                *top += rect.h;
                Some(ViewportContextEntryLayout { index, rect })
            })
            .collect()
    }

    /// Track the pointer so the row under it highlights.
    pub fn handle_pointer_move(&mut self, position: GuiPoint) {
        self.pointer = Some(position);
    }

    /// Releasing a button over the popup.
    ///
    /// A row that is not a live command — greyed, or the separator — swallows
    /// the click and leaves the menu up; only a click outside cancels it.
    /// Both toolkits behave that way, and it matters here because the popup
    /// is not modal: a greyed row that dismissed would make the menu feel
    /// like it had acted.
    pub fn handle_pointer_up(
        &mut self,
        position: GuiPoint,
        width: u32,
        height: u32,
    ) -> ViewportContextOutcome {
        self.pointer = Some(position);
        let Some(row) = self
            .layout(width, height)
            .into_iter()
            .find(|row| contains(row.rect, position))
        else {
            return ViewportContextOutcome::Dismiss;
        };
        self.entries
            .get(row.index)
            .filter(|entry| entry.enabled())
            .and_then(ViewportContextEntry::item)
            .map_or(ViewportContextOutcome::Ignored, |item| {
                ViewportContextOutcome::Activate(item)
            })
    }

    /// Paint the popup over a finished viewport frame.
    pub fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        let rows = self.layout(surface.width(), surface.height());
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return;
        };
        draw_raised(
            surface,
            IntRect {
                x: first.rect.x,
                y: first.rect.y,
                w: first.rect.w,
                h: last.rect.y + last.rect.h - first.rect.y,
            },
            WINDOW_BACKGROUND,
        );
        for row in &rows {
            let Some(entry) = self.entries.get(row.index) else {
                continue;
            };
            match entry {
                ViewportContextEntry::Separator => fill(
                    surface,
                    IntRect {
                        x: row.rect.x + 4,
                        y: row.rect.y + row.rect.h / 2,
                        w: row.rect.w - 8,
                        h: 1,
                    },
                    MID_EDGE,
                ),
                ViewportContextEntry::Item { label, enabled, .. } => {
                    let highlighted = self.pointer.is_some_and(|point| contains(row.rect, point));
                    if highlighted && *enabled {
                        fill(surface, row.rect, SELECTED_BACKGROUND);
                    }
                    draw_fitted_text(
                        surface,
                        font,
                        row.rect,
                        label,
                        if !*enabled {
                            DISABLED_TEXT
                        } else if highlighted {
                            SELECTED_TEXT
                        } else {
                            CONTROL_TEXT
                        },
                        SMALL_FONT_SIZE,
                        8,
                    );
                }
            }
        }
    }
}

fn entry_height(entry: &ViewportContextEntry) -> i32 {
    match entry {
        ViewportContextEntry::Separator => MENU_SEPARATOR_HEIGHT,
        ViewportContextEntry::Item { .. } => MENU_ITEM_HEIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_engine::developer_cursor::{context_menu, CursorMode};
    use clonk_graphics::{BitmapFont, PixelFormat};

    fn labels() -> ViewportContextLabels {
        ViewportContextLabels::default()
    }

    fn items(menu: &ViewportContextMenu) -> Vec<Option<ViewportContextItem>> {
        menu.entries()
            .iter()
            .map(ViewportContextEntry::item)
            .collect()
    }

    fn enabled(menu: &ViewportContextMenu) -> Vec<ViewportContextItem> {
        menu.entries()
            .iter()
            .filter(|entry| entry.enabled())
            .filter_map(ViewportContextEntry::item)
            .collect()
    }

    // res/engine.rc:287-295 and C4EditCursor.cpp:88-97,588-595 — the same five
    // rows in the same order, the same four enablement rules, and the caption
    // Properties swaps by mode.
    #[test]
    fn viewport_context_menu_mirrors_the_native_item_set_and_enablement() {
        let editing = context_menu(CursorMode::Edit, true, true, 2);
        let menu = ViewportContextMenu::new(editing, &labels(), (0, 0));
        assert_eq!(
            items(&menu),
            vec![
                Some(ViewportContextItem::Delete),
                Some(ViewportContextItem::Duplicate),
                Some(ViewportContextItem::GrabContents),
                None,
                Some(ViewportContextItem::Properties),
            ],
            "the separator sits above Properties, as it does in both toolkits"
        );
        assert_eq!(
            enabled(&menu),
            vec![
                ViewportContextItem::Delete,
                ViewportContextItem::Duplicate,
                ViewportContextItem::GrabContents,
                ViewportContextItem::Properties,
            ]
        );

        // An empty container greys Grab contents alone (`:590`).
        let empty = ViewportContextMenu::new(
            context_menu(CursorMode::Edit, true, true, 0),
            &labels(),
            (0, 0),
        );
        assert_eq!(
            enabled(&empty),
            vec![
                ViewportContextItem::Delete,
                ViewportContextItem::Duplicate,
                ViewportContextItem::Properties,
            ]
        );

        // Nothing selected: only Properties survives, because it is gated on
        // the mode alone (`:591`).
        let unselected = ViewportContextMenu::new(
            context_menu(CursorMode::Edit, true, false, 0),
            &labels(),
            (0, 0),
        );
        assert_eq!(enabled(&unselected), vec![ViewportContextItem::Properties]);

        // Play mode greys Properties and renames it. The other three are not
        // gated on the mode at all — `SetMenuItemEnable` asks only
        // `fObjectSelected && Console.Editing` for them (`:588-590`), so a
        // selection made in Edit mode is still deletable after switching to
        // Play. The caption follows `Mode == C4CNS_ModeEdit`, so Draw reads
        // "Tools" as well.
        let playing = ViewportContextMenu::new(
            context_menu(CursorMode::Play, true, true, 3),
            &labels(),
            (0, 0),
        );
        assert_eq!(
            enabled(&playing),
            vec![
                ViewportContextItem::Delete,
                ViewportContextItem::Duplicate,
                ViewportContextItem::GrabContents,
            ]
        );

        // A network client without edit rights loses all three object
        // commands, whatever the mode (`Console.Editing`).
        let spectating = ViewportContextMenu::new(
            context_menu(CursorMode::Edit, false, true, 3),
            &labels(),
            (0, 0),
        );
        assert_eq!(enabled(&spectating), vec![ViewportContextItem::Properties]);
        let caption = |menu: &ViewportContextMenu| match &menu.entries()[4] {
            ViewportContextEntry::Item { label, .. } => label.clone(),
            ViewportContextEntry::Separator => unreachable!("row 4 is Properties"),
        };
        assert_eq!(caption(&playing), "Tools");
        assert_eq!(caption(&menu), "Properties");
        let drawing = ViewportContextMenu::new(
            context_menu(CursorMode::Draw, true, true, 3),
            &labels(),
            (0, 0),
        );
        assert_eq!(caption(&drawing), "Tools");
        assert_eq!(
            enabled(&drawing),
            vec![
                ViewportContextItem::Delete,
                ViewportContextItem::Duplicate,
                ViewportContextItem::GrabContents,
                ViewportContextItem::Properties,
            ]
        );
    }

    // The popup is painted onto the viewport's own framebuffer, so a row that
    // fell outside it could never be clicked.
    #[test]
    fn viewport_context_menu_stays_inside_the_window_and_activates_only_live_rows() {
        let menu = ViewportContextMenu::new(
            context_menu(CursorMode::Edit, true, true, 1),
            &labels(),
            (10, 20),
        );
        let rows = menu.layout(400, 250);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].rect.x, 10);
        assert_eq!(rows[0].rect.y, 20);
        // Rows stack in order, the separator shorter than an item.
        assert_eq!(rows[1].rect.y, 20 + MENU_ITEM_HEIGHT);
        assert_eq!(rows[3].rect.h, MENU_SEPARATOR_HEIGHT);
        assert_eq!(rows[4].rect.y, rows[3].rect.y + MENU_SEPARATOR_HEIGHT);

        // A corner click shifts the whole popup back inside rather than
        // clipping it.
        let corner = ViewportContextMenu::new(
            context_menu(CursorMode::Edit, true, true, 1),
            &labels(),
            (399, 249),
        );
        let rows = corner.layout(400, 250);
        let last = rows.last().expect("five rows");
        assert!(rows[0].rect.x >= 0 && rows[0].rect.x + rows[0].rect.w <= 400);
        assert!(rows[0].rect.y >= 0 && last.rect.y + last.rect.h <= 250);

        // A window narrower than the popup still gets a menu, clamped to it.
        let rows = corner.layout(40, 30);
        assert_eq!(rows[0].rect.x, 0);
        assert_eq!(rows[0].rect.w, 40);
        assert_eq!(rows[0].rect.y, 0);
    }

    #[test]
    fn viewport_context_menu_click_activates_enabled_rows_only() {
        let mut menu = ViewportContextMenu::new(
            context_menu(CursorMode::Edit, true, true, 0),
            &labels(),
            (10, 20),
        );
        let rows = menu.layout(400, 250);
        let center = |row: &ViewportContextEntryLayout| {
            GuiPoint::new(
                (row.rect.x + row.rect.w / 2) as f32,
                (row.rect.y + row.rect.h / 2) as f32,
            )
        };
        assert_eq!(
            menu.handle_pointer_up(center(&rows[0]), 400, 250),
            ViewportContextOutcome::Activate(ViewportContextItem::Delete)
        );
        // Grab contents is grey with an empty container: the click is
        // swallowed and the menu stays up, as a `MF_GRAYED` item and a GTK
        // insensitive one both do.
        assert_eq!(
            menu.handle_pointer_up(center(&rows[2]), 400, 250),
            ViewportContextOutcome::Ignored
        );
        // The separator is not a command either, and does not dismiss.
        assert_eq!(
            menu.handle_pointer_up(center(&rows[3]), 400, 250),
            ViewportContextOutcome::Ignored
        );
        // Only a click outside the popup cancels it.
        assert_eq!(
            menu.handle_pointer_up(GuiPoint::new(390.0, 240.0), 400, 250),
            ViewportContextOutcome::Dismiss
        );
    }

    #[test]
    fn viewport_context_menu_renders_without_panicking_at_any_extent() {
        let font = BitmapFont::new();
        let mut menu = ViewportContextMenu::new(
            context_menu(CursorMode::Draw, true, true, 4),
            &labels(),
            (300, 200),
        );
        for (width, height) in [(400u32, 250u32), (1, 1), (40, 30)] {
            let mut surface = Surface::new(width, height, PixelFormat::Rgba8888);
            menu.handle_pointer_move(GuiPoint::new(10.0, 10.0));
            menu.render(&mut surface, &font);
        }
    }
}
