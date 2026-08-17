//! Drawing and hit-testing the object list window.
//!
//! `C4ObjectListDlg` is a `GtkTreeView` over a custom `C4List` model
//! (`C4ObjectListDlg.cpp:726-787`) and exists **only** under
//! `WITH_DEVELOPER_MODE`: past that `#else` every method of the class is an
//! empty body (`:791-805`), so on the reference build `C4Console::EditObjects`
//! opens nothing at all.
//!
//! The tree's *shape* is portable and already ported —
//! [`clonk_engine::developer_inspection::object_tree`] builds the same nesting
//! the model does, contained objects appearing under their container rather
//! than at the root. What this module adds is the flattening into drawable
//! rows, the row a click resolves to, and the two-way selection binding that
//! `OnSelectionChanged` and `Update` implement between them.

use clonk_engine::developer_inspection::InspectionNode;
use clonk_engine::ObjectId;
use clonk_frontend::classic_gui::{draw_facet_stretch, IntRect};
use clonk_frontend::developer_chrome::{
    contains, draw_fitted_text, draw_sunken, fill, CONTROL_BACKGROUND, CONTROL_TEXT,
    SELECTED_BACKGROUND, SELECTED_TEXT, SMALL_FONT_SIZE, WINDOW_BACKGROUND,
};
use clonk_frontend::{GuiPoint, ImageData};
use clonk_graphics::{Surface, TextFont};

/// `gtk_window_set_default_size(GTK_WINDOW(window), 180, 300)`
/// (`C4ObjectListDlg.cpp:735`).
pub(crate) const OBJECT_LIST_WIDTH: u32 = 180;
pub(crate) const OBJECT_LIST_HEIGHT: u32 = 300;

/// The window title (`:734`). Not a resource string in C++ either — it is the
/// literal `"Objects"`.
pub(crate) const OBJECT_LIST_TITLE: &str = "Objects";

const PADDING: i32 = 4;
/// `ICON_SIZE` in C4ObjectListDlg.cpp:665 and the fixed-height tree rows at
/// `:773`: the pixbuf renderer makes every row tall enough for its icon.
const ICON_SIZE: i32 = 24;
const ROW_HEIGHT: i32 = ICON_SIZE;
/// A contained object is drawn under its container, indented by one step —
/// `GtkTreeView`'s expander column does the same.
const INDENT: i32 = 12;

/// One drawable row: an object, its depth, name, and definition picture.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ObjectListRow {
    pub(crate) id: ObjectId,
    pub(crate) depth: usize,
    pub(crate) name: String,
    pub(crate) icon: Option<ImageData>,
}

/// Flatten the ported tree into rows, parents before their contents.
///
/// `GtkTreeView` draws an expanded tree in exactly this order, and the port
/// has no expanders — the tree is always open, because a collapsed row would
/// hide a selection the edit cursor can still be holding.
pub(crate) fn object_list_rows(
    tree: &[InspectionNode],
    name_of: impl Fn(ObjectId) -> String,
    icon_of: impl Fn(ObjectId) -> Option<ImageData>,
) -> Vec<ObjectListRow> {
    fn walk(
        nodes: &[InspectionNode],
        depth: usize,
        name_of: &impl Fn(ObjectId) -> String,
        icon_of: &impl Fn(ObjectId) -> Option<ImageData>,
        rows: &mut Vec<ObjectListRow>,
    ) {
        for node in nodes {
            rows.push(ObjectListRow {
                id: node.id,
                depth,
                name: name_of(node.id),
                icon: icon_of(node.id),
            });
            walk(&node.contents, depth + 1, name_of, icon_of, rows);
        }
    }

    let mut rows = Vec::new();
    walk(tree, 0, &name_of, &icon_of, &mut rows);
    rows
}

/// How many rows fit, and which one is first, so the selection stays visible.
fn visible_window(rows: usize, selected_row: Option<usize>, height: u32) -> (usize, usize) {
    let capacity = (((height as i32 - PADDING * 2 - 2) / ROW_HEIGHT).max(1)) as usize;
    let first = selected_row
        .filter(|row| *row >= capacity)
        .map_or(0, |row| row + 1 - capacity)
        .min(rows.saturating_sub(capacity.min(rows)));
    (first, capacity)
}

/// The rectangle a visible row occupies.
fn row_rect(index: usize, first: usize, width: u32) -> IntRect {
    IntRect {
        x: PADDING + 1,
        y: PADDING + 1 + (index - first) as i32 * ROW_HEIGHT,
        w: (width as i32 - PADDING * 2 - 2).max(1),
        h: ROW_HEIGHT,
    }
}

/// The C++ cell-data callback scales each `PictureRect` facet to 24 pixels on
/// its longer side while retaining aspect ratio (`C4ObjectListDlg.cpp:701-714`).
fn icon_extent(icon: &ImageData) -> Option<(i32, i32)> {
    let width = u64::from(icon.width());
    let height = u64::from(icon.height());
    if width == 0 || height == 0 {
        return None;
    }
    let (target_width, target_height) = if width >= height {
        (
            ICON_SIZE,
            ((ICON_SIZE as u64) * height / width).max(1) as i32,
        )
    } else {
        (
            ((ICON_SIZE as u64) * width / height).max(1) as i32,
            ICON_SIZE,
        )
    };
    Some((target_width, target_height))
}

fn icon_rect(row: IntRect, depth: usize, icon: &ImageData) -> Option<IntRect> {
    let (width, height) = icon_extent(icon)?;
    let x = row.x + depth as i32 * INDENT;
    Some(IntRect {
        x,
        y: row.y + (ROW_HEIGHT - height) / 2,
        w: width,
        h: height,
    })
}

/// Which object a click landed on, or `None` past the last row.
///
/// A click on empty space selects nothing rather than the nearest object —
/// `GtkTreeSelection` reports no path there, and `OnSelectionChanged` then
/// clears the edit cursor's selection, which is a real command the user can
/// give.
pub(crate) fn object_list_hit(
    rows: &[ObjectListRow],
    selected_row: Option<usize>,
    width: u32,
    height: u32,
    point: (i32, i32),
) -> Option<ObjectId> {
    let (first, capacity) = visible_window(rows.len(), selected_row, height);
    let position = GuiPoint::new(point.0 as f32, point.1 as f32);
    rows.iter()
        .enumerate()
        .skip(first)
        .take(capacity)
        .find(|(index, _)| contains(row_rect(*index, first, width), position))
        .map(|(_, row)| row.id)
}

/// Draw the list.
///
/// `gtk_tree_selection_set_mode(selection, GTK_SELECTION_MULTIPLE)` (`:779`),
/// so any number of rows can be marked at once — the list mirrors the edit
/// cursor's whole selection, not a single cursor row.
pub(crate) fn render_object_list(
    surface: &mut Surface,
    font: &dyn TextFont,
    rows: &[ObjectListRow],
    selected: &[ObjectId],
) {
    surface.fill(WINDOW_BACKGROUND);
    let (width, height) = (surface.width(), surface.height());
    let client = IntRect {
        x: PADDING,
        y: PADDING,
        w: (width as i32 - PADDING * 2).max(1),
        h: (height as i32 - PADDING * 2).max(1),
    };
    draw_sunken(surface, client, CONTROL_BACKGROUND);
    let first_selected = rows
        .iter()
        .position(|row| selected.first().is_some_and(|id| row.id == *id));
    let (first, capacity) = visible_window(rows.len(), first_selected, height);
    for (index, row) in rows.iter().enumerate().skip(first).take(capacity) {
        let rect = row_rect(index, first, width);
        let chosen = selected.contains(&row.id);
        if chosen {
            fill(surface, rect, SELECTED_BACKGROUND);
        }
        if let Some(image) = row.icon.as_ref() {
            if let Some(icon) = icon_rect(rect, row.depth, image) {
                draw_facet_stretch(
                    surface,
                    image,
                    (0.0, 0.0, image.width() as f32, image.height() as f32),
                    (icon.x as f32, icon.y as f32, icon.w as f32, icon.h as f32),
                    None,
                );
            }
        }
        let text_x = rect.x + row.depth as i32 * INDENT + ICON_SIZE;
        draw_fitted_text(
            surface,
            font,
            IntRect {
                x: text_x,
                w: (rect.w - text_x + rect.x).max(1),
                ..rect
            },
            &row.name,
            if chosen { SELECTED_TEXT } else { CONTROL_TEXT },
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
    use clonk_graphics::{BitmapFont, PixelFormat};

    fn node(id: u64, contents: Vec<InspectionNode>) -> InspectionNode {
        InspectionNode {
            id: ObjectId::new(id),
            contents,
        }
    }

    fn rows() -> Vec<ObjectListRow> {
        object_list_rows(
            &[
                node(1, vec![node(2, vec![node(3, vec![])]), node(4, vec![])]),
                node(5, vec![]),
            ],
            |id| format!("Object {}", id.as_u64()),
            |id| Some(ImageData::new(1, 1, vec![id.as_u64() as u8, 0, 0, 255])),
        )
    }

    // C4ObjectListDlg.cpp:726-787 — a contained object is drawn under its
    // container, which is the shape `object_tree` already builds.
    #[test]
    fn object_list_rows_nest_contents_under_their_container() {
        let rows = rows();
        assert_eq!(
            rows.iter()
                .map(|row| (row.id.as_u64(), row.depth))
                .collect::<Vec<_>>(),
            vec![(1, 0), (2, 1), (3, 2), (4, 1), (5, 0)],
            "parents come before their contents, at one step less depth"
        );
        assert_eq!(rows[0].name, "Object 1");
        assert!(object_list_rows(&[], |_| String::new(), |_| None).is_empty());
    }

    // C4ObjectListDlg.cpp:669-724 — each row's icon is sourced from that
    // object's definition PictureRect and then installed on its renderer.
    #[test]
    fn object_list_rows_capture_each_definition_picture_icon() {
        let first = ImageData::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let second = ImageData::new(1, 2, vec![0, 255, 0, 255, 255, 255, 0, 255]);
        let rows = object_list_rows(
            &[node(1, vec![]), node(2, vec![])],
            |id| format!("Object {}", id.as_u64()),
            |id| match id.as_u64() {
                1 => Some(first.clone()),
                2 => Some(second.clone()),
                _ => None,
            },
        );

        assert_eq!(rows[0].icon.as_ref(), Some(&first));
        assert_eq!(rows[1].icon.as_ref(), Some(&second));
    }

    // C4ObjectListDlg.cpp:701-714 — the icon column preserves the
    // PictureRect aspect ratio while fitting its longer side to ICON_SIZE.
    #[test]
    fn object_list_icons_fit_their_picture_aspect_ratio() {
        let wide = ImageData::new(4, 2, vec![255; 4 * 2 * 4]);
        let tall = ImageData::new(2, 4, vec![255; 2 * 4 * 4]);
        let row = row_rect(0, 0, OBJECT_LIST_WIDTH);

        assert_eq!(icon_extent(&wide), Some((ICON_SIZE, ICON_SIZE / 2)));
        assert_eq!(icon_extent(&tall), Some((ICON_SIZE / 2, ICON_SIZE)));
        assert_eq!(
            icon_rect(row, 1, &wide),
            Some(IntRect {
                x: row.x + INDENT,
                y: row.y + (ROW_HEIGHT - ICON_SIZE / 2) / 2,
                w: ICON_SIZE,
                h: ICON_SIZE / 2,
            })
        );
    }

    #[test]
    fn object_list_click_resolves_a_row_and_empty_space_selects_nothing() {
        let rows = rows();
        let extent = (OBJECT_LIST_WIDTH, OBJECT_LIST_HEIGHT);
        let first = row_rect(0, 0, extent.0);
        assert_eq!(
            object_list_hit(
                &rows,
                None,
                extent.0,
                extent.1,
                (first.x + 2, first.y + ROW_HEIGHT / 2)
            ),
            Some(ObjectId::new(1))
        );
        // The third row is the doubly-contained object, indented twice; the
        // row is still full width, so a click at its left edge finds it.
        let third = row_rect(2, 0, extent.0);
        assert_eq!(
            object_list_hit(
                &rows,
                None,
                extent.0,
                extent.1,
                (third.x + 2, third.y + ROW_HEIGHT / 2)
            ),
            Some(ObjectId::new(3))
        );
        // Past the last row, and outside the client area, select nothing.
        assert_eq!(
            object_list_hit(&rows, None, extent.0, extent.1, (10, extent.1 as i32 - 8)),
            None
        );
        assert_eq!(
            object_list_hit(&rows, None, extent.0, extent.1, (0, 0)),
            None
        );
        assert_eq!(
            object_list_hit(&[], None, extent.0, extent.1, (10, 10)),
            None
        );
    }

    // A list longer than the window scrolls to keep the selection on screen,
    // and the row a click resolves to is the row that was drawn there.
    #[test]
    fn object_list_scrolls_to_the_selection_and_hit_testing_follows_it() {
        let rows = object_list_rows(
            &(0..100).map(|id| node(id, vec![])).collect::<Vec<_>>(),
            |id| format!("Object {}", id.as_u64()),
            |_| None,
        );
        let (width, height) = (OBJECT_LIST_WIDTH, 100);
        let (first, capacity) = visible_window(rows.len(), Some(80), height);
        assert_eq!(capacity, 3, "24px icons determine the fixed row metric");
        assert_eq!(
            first + capacity - 1,
            80,
            "the selected row is the last one on screen"
        );
        let rect = row_rect(80, first, width);
        assert_eq!(
            object_list_hit(&rows, Some(80), width, height, (rect.x + 2, rect.y + 2)),
            Some(ObjectId::new(80))
        );
        // A selection already on screen does not scroll at all.
        assert_eq!(visible_window(rows.len(), Some(0), height).0, 0);
        assert_eq!(visible_window(rows.len(), None, height).0, 0);
        // The window never scrolls past the end of the list.
        let (first, capacity) = visible_window(rows.len(), Some(99), height);
        assert!(first + capacity <= rows.len());
    }

    #[test]
    fn object_list_renders_without_panicking_at_any_extent() {
        let font = BitmapFont::new();
        let rows = rows();
        for (width, height) in [
            (OBJECT_LIST_WIDTH, OBJECT_LIST_HEIGHT),
            (1, 1),
            (40, 24),
            (600, 800),
        ] {
            let mut surface = Surface::new(width, height, PixelFormat::Rgba8888);
            render_object_list(&mut surface, &font, &rows, &[ObjectId::new(3)]);
            let _ = object_list_hit(&rows, Some(2), width, height, (5, 5));
        }
    }
}
