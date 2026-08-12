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
use clonk_frontend::classic_gui::IntRect;
use clonk_frontend::developer_chrome::{
    contains, draw_fitted_text, draw_sunken, fill, CONTROL_BACKGROUND, CONTROL_TEXT,
    SELECTED_BACKGROUND, SELECTED_TEXT, SMALL_FONT_SIZE, WINDOW_BACKGROUND,
};
use clonk_frontend::GuiPoint;
use clonk_graphics::{Surface, TextFont};

/// `gtk_window_set_default_size(GTK_WINDOW(window), 180, 300)`
/// (`C4ObjectListDlg.cpp:735`).
pub(crate) const OBJECT_LIST_WIDTH: u32 = 180;
pub(crate) const OBJECT_LIST_HEIGHT: u32 = 300;

/// The window title (`:734`). Not a resource string in C++ either — it is the
/// literal `"Objects"`.
pub(crate) const OBJECT_LIST_TITLE: &str = "Objects";

const PADDING: i32 = 4;
const ROW_HEIGHT: i32 = 16;
/// A contained object is drawn under its container, indented by one step —
/// `GtkTreeView`'s expander column does the same.
const INDENT: i32 = 12;

/// One drawable row: an object, its depth, and the name to show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectListRow {
    pub(crate) id: ObjectId,
    pub(crate) depth: usize,
    pub(crate) name: String,
}

/// Flatten the ported tree into rows, parents before their contents.
///
/// `GtkTreeView` draws an expanded tree in exactly this order, and the port
/// has no expanders — the tree is always open, because a collapsed row would
/// hide a selection the edit cursor can still be holding.
pub(crate) fn object_list_rows(
    tree: &[InspectionNode],
    name_of: impl Fn(ObjectId) -> String,
) -> Vec<ObjectListRow> {
    fn walk(
        nodes: &[InspectionNode],
        depth: usize,
        name_of: &impl Fn(ObjectId) -> String,
        rows: &mut Vec<ObjectListRow>,
    ) {
        for node in nodes {
            rows.push(ObjectListRow {
                id: node.id,
                depth,
                name: name_of(node.id),
            });
            walk(&node.contents, depth + 1, name_of, rows);
        }
    }

    let mut rows = Vec::new();
    walk(tree, 0, &name_of, &mut rows);
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
        draw_fitted_text(
            surface,
            font,
            IntRect {
                x: rect.x + row.depth as i32 * INDENT,
                w: (rect.w - row.depth as i32 * INDENT).max(1),
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
        assert!(object_list_rows(&[], |_| String::new()).is_empty());
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
        );
        let (width, height) = (OBJECT_LIST_WIDTH, 100);
        let (first, capacity) = visible_window(rows.len(), Some(80), height);
        assert!(capacity > 0 && capacity < rows.len());
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
