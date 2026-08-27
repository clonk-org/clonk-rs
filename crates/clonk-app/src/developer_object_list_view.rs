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
use clonk_frontend::developer_chrome::PaneScroll;
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
    /// Whether the row holds anything — an expander is drawn only for those.
    pub(crate) has_children: bool,
    /// Whether that expander is open. False without children, whatever the
    /// retained state says, because there is nothing to show.
    pub(crate) expanded: bool,
}

/// Which containers the user has opened.
///
/// `GtkTreeView` keys expansion by tree *path*, which a rebuild invalidates.
/// Keying by the object instead is what carries it through the rebuild
/// `C4ObjectListDlg::Update` performs on every object change, and through a
/// reparent that would move the path: an object put down elsewhere is still
/// the row the user opened.
///
/// A container that empties is not forgotten — GTK drops its expander through
/// `row_has_child_toggled` (`C4ObjectListDlg.cpp:504-517`) while the row
/// itself stays — so refilling it comes back open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObjectTreeExpansion {
    open: std::collections::HashSet<ObjectId>,
}

impl ObjectTreeExpansion {
    pub(crate) fn is_open(&self, id: ObjectId) -> bool {
        self.open.contains(&id)
    }

    /// Open a closed container, or close an open one.
    pub(crate) fn toggle(&mut self, id: ObjectId) {
        if !self.open.remove(&id) {
            self.open.insert(id);
        }
    }
}

/// Flatten the ported tree into the rows that are actually visible, parents
/// before their contents.
///
/// `GtkTreeView` draws an expanded tree in exactly this order, and draws
/// nothing under a closed one. The tree view is created with no
/// `expand_all` call (`C4ObjectListDlg.cpp:726-787`), so GTK's default holds
/// and a container starts closed.
pub(crate) fn object_list_rows(
    tree: &[InspectionNode],
    expansion: &ObjectTreeExpansion,
    name_of: impl Fn(ObjectId) -> String,
    icon_of: impl Fn(ObjectId) -> Option<ImageData>,
) -> Vec<ObjectListRow> {
    fn walk(
        nodes: &[InspectionNode],
        depth: usize,
        expansion: &ObjectTreeExpansion,
        name_of: &impl Fn(ObjectId) -> String,
        icon_of: &impl Fn(ObjectId) -> Option<ImageData>,
        rows: &mut Vec<ObjectListRow>,
    ) {
        for node in nodes {
            let has_children = !node.contents.is_empty();
            let expanded = has_children && expansion.is_open(node.id);
            rows.push(ObjectListRow {
                id: node.id,
                depth,
                name: name_of(node.id),
                icon: icon_of(node.id),
                has_children,
                expanded,
            });
            if expanded {
                walk(&node.contents, depth + 1, expansion, name_of, icon_of, rows);
            }
        }
    }

    let mut rows = Vec::new();
    walk(tree, 0, expansion, &name_of, &icon_of, &mut rows);
    rows
}

/// The list's retained scroll position.
///
/// C++ puts the tree in an automatic scrolled window
/// (`C4ObjectListDlg.cpp:747-780`), so the position is state of the widget:
/// `Update` rebuilds the model on every object change and the adjustment does
/// not move. Deriving the offset from the selection instead — which is what
/// this replaces — makes every rebuild jump the view and makes scrolling away
/// from the selection impossible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObjectListScroll {
    /// The shared retained position, over this list's own row metric.
    inner: PaneScroll,
}

impl ObjectListScroll {
    /// How many rows fit in a list of this height.
    pub(crate) fn capacity(height: u32) -> usize {
        (((height as i32 - PADDING * 2 - 2) / ROW_HEIGHT).max(1)) as usize
    }

    /// The first visible row and the capacity, for the tree as it stands now.
    pub(crate) fn window(&self, rows: usize, height: u32) -> (usize, usize) {
        let capacity = Self::capacity(height);
        (self.inner.window(rows, capacity), capacity)
    }

    /// Scroll by whole rows, as a wheel notch or a bar arrow does.
    pub(crate) fn scroll_by(&mut self, delta: i32, rows: usize, height: u32) {
        self.inner.scroll_by(delta, rows, Self::capacity(height));
    }

    /// Put an absolute first row, as a thumb drag does.
    pub(crate) fn scroll_to(&mut self, row: usize, rows: usize, height: u32) {
        self.inner.scroll_to(row, rows, Self::capacity(height));
    }

    /// Scroll a row into view, moving as little as possible.
    pub(crate) fn reveal(&mut self, row: usize, rows: usize, height: u32) {
        self.inner.reveal(row, rows, Self::capacity(height));
    }
}

/// The rectangle a visible row occupies.
fn row_rect(index: usize, first: usize, width: u32) -> IntRect {
    IntRect::new(
        PADDING + 1,
        PADDING + 1 + (index - first) as i32 * ROW_HEIGHT,
        (width as i32 - PADDING * 2 - 2).max(1),
        ROW_HEIGHT,
    )
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
    // The expander column comes first at every depth, whether or not this row
    // draws one — `GtkTreeView` reserves it so sibling rows line up.
    let x = row.x + depth as i32 * INDENT + EXPANDER_SIZE;
    Some(IntRect::new(
        x,
        row.y + (ROW_HEIGHT - height) / 2,
        width,
        height,
    ))
}

/// The expander occupies the indent step before the row's own content, which
/// is where `GtkTreeView` puts it.
const EXPANDER_SIZE: i32 = 9;

/// Where a row's disclosure control sits, if it has one.
///
/// `GtkTreeView` prepends the expander to the first column, so it takes the
/// space the row's contents would otherwise be indented into. A row with
/// nothing inside has none: GTK drops it through `row_has_child_toggled` the
/// moment a container empties (`C4ObjectListDlg.cpp:504-517`).
pub(crate) fn expander_rect(row: IntRect, depth: usize) -> Option<IntRect> {
    let x = row.x + depth as i32 * INDENT;
    (x + EXPANDER_SIZE <= row.x + row.w).then(|| {
        IntRect::new(
            x,
            row.y + (ROW_HEIGHT - EXPANDER_SIZE) / 2,
            EXPANDER_SIZE,
            EXPANDER_SIZE,
        )
    })
}

/// A visible row's rectangle at the top of an unscrolled list, for a test that
/// needs to aim at one.
#[cfg(test)]
pub(crate) fn object_list_row_rect_for_test(index: usize, width: u32) -> IntRect {
    row_rect(index, 0, width)
}

/// A navigation key the tree understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectListKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// Close the row, or step out to its parent.
    Left,
    /// Open the row, or step into it.
    Right,
}

/// What a navigation key asked the list to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectListNavigation {
    /// Put the cursor on this row.
    MoveCursor(ObjectId),
    Expand(ObjectId),
    Collapse(ObjectId),
}

/// Resolve a navigation key against the rows that are on screen.
///
/// `C4ObjectListDlg` builds a stock `GtkTreeView` (`C4ObjectListDlg.cpp:
/// 726-787`), so its key handling is GTK's: the cursor walks the *visible*
/// rows, stepping over a closed container rather than into it, and Left/Right
/// work the disclosure before they work the cursor.
///
/// Returns `None` when the key asks for something the list cannot do — the end
/// of the list, or a leaf asked to open — which is what leaves the key
/// unclaimed.
pub(crate) fn object_list_navigate(
    rows: &[ObjectListRow],
    cursor: Option<ObjectId>,
    key: ObjectListKey,
    page: usize,
) -> Option<ObjectListNavigation> {
    if rows.is_empty() {
        return None;
    }
    // A cursor whose row is no longer drawn — its container was closed, or the
    // object left the world — is not a position, so the first key starts over.
    let Some(index) = cursor.and_then(|id| rows.iter().position(|row| row.id == id)) else {
        return Some(ObjectListNavigation::MoveCursor(rows[0].id));
    };
    let last = rows.len() - 1;
    let page = page.max(1);
    let row = &rows[index];

    let target = match key {
        ObjectListKey::Up => index.checked_sub(1)?,
        ObjectListKey::Down => (index < last).then_some(index + 1)?,
        ObjectListKey::Home => 0,
        ObjectListKey::End => last,
        ObjectListKey::PageUp => index.saturating_sub(page),
        ObjectListKey::PageDown => (index + page).min(last),
        ObjectListKey::Right => {
            return match (row.has_children, row.expanded) {
                (true, false) => Some(ObjectListNavigation::Expand(row.id)),
                // The child of an open row is the next visible row, by
                // construction: contents follow their container.
                (true, true) => rows
                    .get(index + 1)
                    .map(|child| ObjectListNavigation::MoveCursor(child.id)),
                (false, _) => None,
            };
        }
        ObjectListKey::Left => {
            if row.expanded {
                return Some(ObjectListNavigation::Collapse(row.id));
            }
            // Step out: the parent is the nearest earlier row one step
            // shallower, which is what a path walk would find.
            return rows[..index]
                .iter()
                .rposition(|candidate| candidate.depth + 1 == row.depth)
                .map(|parent| ObjectListNavigation::MoveCursor(rows[parent].id));
        }
    };

    (target != index).then(|| ObjectListNavigation::MoveCursor(rows[target].id))
}

/// What a click on the list asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectListClick {
    /// The row's own content: select it.
    Select(ObjectId),
    /// The row's expander: open or close it, without changing the selection.
    Toggle(ObjectId),
}

/// What a click landed on, or `None` past the last row.
///
/// A click on empty space selects nothing rather than the nearest object —
/// `GtkTreeSelection` reports no path there, and `OnSelectionChanged` then
/// clears the edit cursor's selection, which is a real command the user can
/// give.
pub(crate) fn object_list_hit(
    rows: &[ObjectListRow],
    scroll: ObjectListScroll,
    width: u32,
    height: u32,
    point: (i32, i32),
) -> Option<ObjectListClick> {
    let (first, capacity) = scroll.window(rows.len(), height);
    let position = GuiPoint::new(point.0 as f32, point.1 as f32);
    rows.iter()
        .enumerate()
        .skip(first)
        .take(capacity)
        .find(|(index, _)| contains(row_rect(*index, first, width), position))
        .map(|(index, row)| {
            let expander = row
                .has_children
                .then(|| expander_rect(row_rect(index, first, width), row.depth))
                .flatten();
            match expander {
                Some(rect) if contains(rect, position) => ObjectListClick::Toggle(row.id),
                _ => ObjectListClick::Select(row.id),
            }
        })
}

/// A closed or open disclosure triangle, in the flat style the rest of these
/// windows use.
///
/// `GtkTreeView` draws its own themed expander; the shape is the port's, the
/// two states are not.
fn draw_expander(surface: &mut Surface, rect: IntRect, expanded: bool) {
    for step in 0..(EXPANDER_SIZE / 2 + 1) {
        let (x, y, w, h) = if expanded {
            // Pointing down: a wide top row narrowing as it descends.
            (
                rect.x + step,
                rect.y + EXPANDER_SIZE / 4 + step,
                (EXPANDER_SIZE - step * 2).max(1),
                1,
            )
        } else {
            // Pointing right.
            (
                rect.x + EXPANDER_SIZE / 4 + step,
                rect.y + step,
                1,
                (EXPANDER_SIZE - step * 2).max(1),
            )
        };
        fill(surface, IntRect::new(x, y, w, h), CONTROL_TEXT);
    }
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
    scroll: ObjectListScroll,
) {
    surface.fill(WINDOW_BACKGROUND);
    let (width, height) = (surface.width(), surface.height());
    let client = IntRect::new(
        PADDING,
        PADDING,
        (width as i32 - PADDING * 2).max(1),
        (height as i32 - PADDING * 2).max(1),
    );
    draw_sunken(surface, client, CONTROL_BACKGROUND);
    let (first, capacity) = scroll.window(rows.len(), height);
    for (index, row) in rows.iter().enumerate().skip(first).take(capacity) {
        let rect = row_rect(index, first, width);
        // The expander comes before the cell renderers, so it is drawn first
        // and is never covered by the icon or the name.
        if row.has_children {
            if let Some(expander) = expander_rect(rect, row.depth) {
                draw_expander(surface, expander, row.expanded);
            }
        }
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
            rect.with_horizontal(text_x, (rect.w - text_x + rect.x).max(1)),
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

    /// The list keeps where the user put it.
    ///
    /// C++ puts the tree in an automatic scrolled window
    /// (`C4ObjectListDlg.cpp:747-780`), whose adjustment is state of the
    /// widget rather than something recomputed from the selection. A rebuild —
    /// and `Update` rebuilds on every object change — leaves it alone.
    #[test]
    fn the_object_list_scroll_offset_survives_a_rebuild_and_clamps_when_the_tree_shrinks() {
        let height = 8 * ROW_HEIGHT as u32 + (PADDING * 2 + 2) as u32;
        let mut scroll = ObjectListScroll::default();
        let capacity = ObjectListScroll::capacity(height);
        assert_eq!(capacity, 8);

        scroll.scroll_by(5, 100, height);
        assert_eq!(scroll.window(100, height), (5, 8));

        // A rebuild is not a reason to move.
        assert_eq!(scroll.window(100, height), (5, 8));

        // Shrinking past the offset pins it to the last full page rather than
        // leaving the view scrolled past the end.
        assert_eq!(scroll.window(10, height), (2, 8));
        // A tree shorter than one page starts at the top.
        assert_eq!(scroll.window(3, height), (0, 8));
        assert_eq!(scroll.window(0, height), (0, 8));
        // Clamping for a short tree is not a *write*: the retained offset
        // comes back when the tree grows again, which is what makes a live
        // mutation non-destructive.
        assert_eq!(scroll.window(100, height), (5, 8));
    }

    /// Selecting an offscreen row brings it into view, and no further.
    ///
    /// `gtk_tree_view_set_cursor` scrolls the row into view rather than
    /// centring it, so a row just below the last visible one moves the window
    /// by exactly one.
    #[test]
    fn revealing_a_row_moves_the_window_the_minimum_in_either_direction() {
        let height = 8 * ROW_HEIGHT as u32 + (PADDING * 2 + 2) as u32;
        let mut scroll = ObjectListScroll::default();
        scroll.scroll_by(20, 100, height);
        assert_eq!(scroll.window(100, height).0, 20);

        // Already visible: nothing moves.
        for row in [20, 24, 27] {
            scroll.reveal(row, 100, height);
            assert_eq!(scroll.window(100, height).0, 20, "row {row} was visible");
        }

        // One past the bottom scrolls by one.
        scroll.reveal(28, 100, height);
        assert_eq!(scroll.window(100, height).0, 21);

        // Above the top puts the row first.
        scroll.reveal(4, 100, height);
        assert_eq!(scroll.window(100, height).0, 4);

        // Far below lands the row on the last visible line.
        scroll.reveal(99, 100, height);
        assert_eq!(scroll.window(100, height).0, 92);
    }

    use clonk_graphics::{BitmapFont, PixelFormat};

    fn node(id: u64, contents: Vec<InspectionNode>) -> InspectionNode {
        InspectionNode {
            id: ObjectId::new(id),
            contents,
        }
    }

    /// Arrow keys walk the rows that are actually on screen.
    ///
    /// `GtkTreeView`'s cursor moves through the *visible* rows, so a closed
    /// container's contents are stepped over rather than descended into — the
    /// same rows `object_list_rows` produces (`C4ObjectListDlg.cpp:726-787`
    /// builds a stock tree view, whose key handling is GTK's).
    #[test]
    fn arrow_keys_walk_the_visible_rows_and_stop_at_the_ends() {
        let tree = vec![
            node(1, vec![node(2, vec![]), node(3, vec![])]),
            node(4, vec![]),
        ];
        let name = |id: ObjectId| format!("Object {}", id.as_u64());
        let icon = |_: ObjectId| None;
        let closed = object_list_rows(&tree, &ObjectTreeExpansion::default(), name, icon);

        // Closed: rows are 1 and 4, so Down from 1 reaches 4.
        assert_eq!(
            object_list_navigate(&closed, None, ObjectListKey::Down, 8),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(1))),
            "with no cursor, the first key lands on the first row"
        );
        assert_eq!(
            object_list_navigate(&closed, Some(ObjectId::new(1)), ObjectListKey::Down, 8),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(4)))
        );
        assert_eq!(
            object_list_navigate(&closed, Some(ObjectId::new(4)), ObjectListKey::Down, 8),
            None,
            "the last row is the end"
        );
        assert_eq!(
            object_list_navigate(&closed, Some(ObjectId::new(1)), ObjectListKey::Up, 8),
            None,
            "and the first row is the other end"
        );

        // Open: the contents are now rows, so Down descends into them.
        let mut expansion = ObjectTreeExpansion::default();
        expansion.toggle(ObjectId::new(1));
        let open = object_list_rows(&tree, &expansion, name, icon);
        assert_eq!(
            object_list_navigate(&open, Some(ObjectId::new(1)), ObjectListKey::Down, 8),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(2)))
        );

        // A cursor on a row that is no longer visible starts over.
        assert_eq!(
            object_list_navigate(&closed, Some(ObjectId::new(2)), ObjectListKey::Down, 8),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(1))),
            "a cursor inside a closed container is not a position"
        );

        // Nothing to walk.
        assert_eq!(
            object_list_navigate(&[], None, ObjectListKey::Down, 8),
            None
        );
        assert_eq!(object_list_navigate(&[], None, ObjectListKey::End, 8), None);
    }

    /// Home, End and the page keys address the visible list.
    #[test]
    fn home_end_and_page_keys_move_by_the_list_and_by_the_page() {
        let tree = (0..20).map(|id| node(id, vec![])).collect::<Vec<_>>();
        let rows = object_list_rows(
            &tree,
            &ObjectTreeExpansion::default(),
            |id| format!("Object {}", id.as_u64()),
            |_| None,
        );
        let page = 6;

        assert_eq!(
            object_list_navigate(&rows, Some(ObjectId::new(9)), ObjectListKey::Home, page),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(0)))
        );
        assert_eq!(
            object_list_navigate(&rows, Some(ObjectId::new(9)), ObjectListKey::End, page),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(19)))
        );
        assert_eq!(
            object_list_navigate(&rows, Some(ObjectId::new(9)), ObjectListKey::PageDown, page),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(15)))
        );
        assert_eq!(
            object_list_navigate(&rows, Some(ObjectId::new(9)), ObjectListKey::PageUp, page),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(3)))
        );
        // A page that would run off either end stops at it rather than
        // refusing to move.
        assert_eq!(
            object_list_navigate(&rows, Some(ObjectId::new(2)), ObjectListKey::PageUp, page),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(0)))
        );
        assert_eq!(
            object_list_navigate(
                &rows,
                Some(ObjectId::new(17)),
                ObjectListKey::PageDown,
                page
            ),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(19)))
        );
        // Already there: nothing to do.
        assert_eq!(
            object_list_navigate(&rows, Some(ObjectId::new(0)), ObjectListKey::Home, page),
            None
        );
    }

    /// Left and right work the disclosure before they work the cursor.
    ///
    /// GTK's tree view gives Right two jobs: open a closed row, or step into
    /// an already-open one. Left is the mirror — close an open row, or step
    /// out to the parent.
    #[test]
    fn left_and_right_expand_collapse_then_move() {
        let tree = vec![
            node(1, vec![node(2, vec![node(3, vec![])])]),
            node(4, vec![]),
        ];
        let name = |id: ObjectId| format!("Object {}", id.as_u64());
        let icon = |_: ObjectId| None;

        let mut expansion = ObjectTreeExpansion::default();
        let closed = object_list_rows(&tree, &expansion, name, icon);
        assert_eq!(
            object_list_navigate(&closed, Some(ObjectId::new(1)), ObjectListKey::Right, 8),
            Some(ObjectListNavigation::Expand(ObjectId::new(1))),
            "right opens a closed container"
        );

        expansion.toggle(ObjectId::new(1));
        let open = object_list_rows(&tree, &expansion, name, icon);
        assert_eq!(
            object_list_navigate(&open, Some(ObjectId::new(1)), ObjectListKey::Right, 8),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(2))),
            "right on an open container steps into it"
        );
        assert_eq!(
            object_list_navigate(&open, Some(ObjectId::new(1)), ObjectListKey::Left, 8),
            Some(ObjectListNavigation::Collapse(ObjectId::new(1))),
            "left closes it again"
        );
        assert_eq!(
            object_list_navigate(&open, Some(ObjectId::new(2)), ObjectListKey::Left, 8),
            Some(ObjectListNavigation::MoveCursor(ObjectId::new(1))),
            "left on a closed child steps out to its parent"
        );
        assert_eq!(
            object_list_navigate(&open, Some(ObjectId::new(4)), ObjectListKey::Left, 8),
            None,
            "a top-level leaf has nowhere to step out to"
        );
        assert_eq!(
            object_list_navigate(&open, Some(ObjectId::new(4)), ObjectListKey::Right, 8),
            None,
            "and nothing to open"
        );
    }

    /// The expander is its own click target, ahead of the row's own.
    ///
    /// `GtkTreeView`'s expander column sits before the cell renderers and
    /// consumes the click that opens or closes a row — the selection does not
    /// change with it (`C4ObjectListDlg.cpp:757-773` builds the one column the
    /// expander is prepended to).
    #[test]
    fn a_click_on_the_expander_toggles_instead_of_selecting() {
        let tree = vec![node(1, vec![node(2, vec![])]), node(3, vec![])];
        let expansion = ObjectTreeExpansion::default();
        let rows = object_list_rows(&tree, &expansion, |_| String::new(), |_| None);
        let extent = (OBJECT_LIST_WIDTH, OBJECT_LIST_HEIGHT);
        let scroll = ObjectListScroll::default();

        let container = row_rect(0, 0, extent.0);
        let expander =
            expander_rect(container, rows[0].depth).expect("a container carries an expander");
        let middle = (expander.x + expander.w / 2, expander.y + expander.h / 2);
        assert_eq!(
            object_list_hit(&rows, scroll, extent.0, extent.1, middle),
            Some(ObjectListClick::Toggle(ObjectId::new(1)))
        );

        // Past the expander, the same row selects as before.
        let name = (expander.x + expander.w + 4, middle.1);
        assert_eq!(
            object_list_hit(&rows, scroll, extent.0, extent.1, name),
            Some(ObjectListClick::Select(ObjectId::new(1)))
        );

        // A childless row draws no expander, so the same spot selects it: the
        // column is reserved for alignment but claims nothing.
        let leaf = row_rect(1, 0, extent.0);
        assert!(!rows[1].has_children);
        assert_eq!(
            object_list_hit(
                &rows,
                scroll,
                extent.0,
                extent.1,
                (leaf.x + 2, leaf.y + ROW_HEIGHT / 2)
            ),
            Some(ObjectListClick::Select(ObjectId::new(3)))
        );
    }

    /// A container's contents are hidden until it is opened.
    ///
    /// The tree is a plain `GtkTreeView` over a hierarchical model
    /// (`C4ObjectListDlg.cpp:726-787`): it never calls
    /// `gtk_tree_view_expand_all`, so GTK's default stands and a row with
    /// children starts closed behind an expander. The port drew the whole
    /// hierarchy open, which is what this replaces.
    #[test]
    fn collapsed_containers_hide_their_contents_and_carry_an_expander() {
        let tree = vec![
            node(1, vec![node(2, vec![node(3, vec![])]), node(4, vec![])]),
            node(5, vec![]),
        ];
        let name = |id: ObjectId| format!("Object {}", id.as_u64());
        let icon = |_: ObjectId| None;

        let mut expansion = ObjectTreeExpansion::default();
        let closed = object_list_rows(&tree, &expansion, name, icon);
        assert_eq!(
            closed.iter().map(|row| row.id.as_u64()).collect::<Vec<_>>(),
            vec![1, 5],
            "only the roots are drawn while everything is closed"
        );
        assert!(closed[0].has_children, "object 1 contains two objects");
        assert!(!closed[0].expanded);
        assert!(
            !closed[1].has_children,
            "an object with nothing inside gets no expander"
        );

        expansion.toggle(ObjectId::new(1));
        let one_open = object_list_rows(&tree, &expansion, name, icon);
        assert_eq!(
            one_open
                .iter()
                .map(|row| row.id.as_u64())
                .collect::<Vec<_>>(),
            vec![1, 2, 4, 5],
            "opening a container shows its own contents, not its grandchildren"
        );
        assert!(one_open[0].expanded);
        assert!(one_open[1].has_children, "object 2 still contains object 3");

        expansion.toggle(ObjectId::new(2));
        let both_open = object_list_rows(&tree, &expansion, name, icon);
        assert_eq!(
            both_open
                .iter()
                .map(|row| (row.id.as_u64(), row.depth))
                .collect::<Vec<_>>(),
            vec![(1, 0), (2, 1), (3, 2), (4, 1), (5, 0)],
            "depth is the nesting, whatever is open"
        );

        // Closing the outer one hides the inner one too, without forgetting
        // that it was open: GTK keeps a collapsed subtree's own expansion.
        expansion.toggle(ObjectId::new(1));
        assert_eq!(
            object_list_rows(&tree, &expansion, name, icon)
                .iter()
                .map(|row| row.id.as_u64())
                .collect::<Vec<_>>(),
            vec![1, 5]
        );
        expansion.toggle(ObjectId::new(1));
        assert_eq!(
            object_list_rows(&tree, &expansion, name, icon)
                .iter()
                .map(|row| row.id.as_u64())
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5],
            "reopening restores the subtree the user had opened"
        );
    }

    /// Expansion is keyed by the object, so a rebuild that moves it keeps it.
    ///
    /// `Update` rebuilds the model on every object change, and a container
    /// that empties loses its expander through `row_has_child_toggled`
    /// (`C4ObjectListDlg.cpp:504-517`) without the view forgetting the row.
    #[test]
    fn expansion_follows_the_object_through_reparenting_and_emptying() {
        let name = |id: ObjectId| format!("Object {}", id.as_u64());
        let icon = |_: ObjectId| None;
        let mut expansion = ObjectTreeExpansion::default();
        expansion.toggle(ObjectId::new(2));

        // Object 2 starts inside 1.
        let nested = vec![node(1, vec![node(2, vec![node(3, vec![])])])];
        expansion.toggle(ObjectId::new(1));
        assert_eq!(
            object_list_rows(&nested, &expansion, name, icon)
                .iter()
                .map(|row| row.id.as_u64())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        // It leaves the container: still open, now at the top level.
        let reparented = vec![node(1, vec![]), node(2, vec![node(3, vec![])])];
        let rows = object_list_rows(&reparented, &expansion, name, icon);
        assert_eq!(
            rows.iter().map(|row| row.id.as_u64()).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "reparenting does not close it"
        );
        assert!(
            !rows[0].has_children,
            "the emptied container loses its expander"
        );

        // Emptied itself, it keeps no expander and draws nothing under it.
        let emptied = vec![node(1, vec![]), node(2, vec![])];
        let rows = object_list_rows(&emptied, &expansion, name, icon);
        assert_eq!(
            rows.iter().map(|row| row.id.as_u64()).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(!rows[1].has_children);
        assert!(
            !rows[1].expanded,
            "a row with nothing to show is not drawn open"
        );
    }

    /// Every container open, which is what the older tests assume: they were
    /// written when the port drew the whole hierarchy unconditionally.
    fn open_tree() -> ObjectTreeExpansion {
        let mut expansion = ObjectTreeExpansion::default();
        for id in [1, 2, 4] {
            expansion.toggle(ObjectId::new(id));
        }
        expansion
    }

    fn rows() -> Vec<ObjectListRow> {
        object_list_rows(
            &[
                node(1, vec![node(2, vec![node(3, vec![])]), node(4, vec![])]),
                node(5, vec![]),
            ],
            &open_tree(),
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
        assert!(object_list_rows(
            &[],
            &ObjectTreeExpansion::default(),
            |_| String::new(),
            |_| None
        )
        .is_empty());
    }

    // C4ObjectListDlg.cpp:669-724 — each row's icon is sourced from that
    // object's definition PictureRect and then installed on its renderer.
    #[test]
    fn object_list_rows_capture_each_definition_picture_icon() {
        let first = ImageData::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let second = ImageData::new(1, 2, vec![0, 255, 0, 255, 255, 255, 0, 255]);
        let rows = object_list_rows(
            &[node(1, vec![]), node(2, vec![])],
            &ObjectTreeExpansion::default(),
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
            Some(IntRect::new(
                row.x + INDENT + EXPANDER_SIZE,
                row.y + (ROW_HEIGHT - ICON_SIZE / 2) / 2,
                ICON_SIZE,
                ICON_SIZE / 2
            ))
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
                ObjectListScroll::default(),
                extent.0,
                extent.1,
                // Past the expander: object 1 is a container, and its left
                // edge belongs to the disclosure control.
                (first.x + EXPANDER_SIZE + 2, first.y + ROW_HEIGHT / 2)
            ),
            Some(ObjectListClick::Select(ObjectId::new(1)))
        );
        // The third row is the doubly-contained object, indented twice; the
        // row is still full width, so a click at its left edge finds it.
        let third = row_rect(2, 0, extent.0);
        assert_eq!(
            object_list_hit(
                &rows,
                ObjectListScroll::default(),
                extent.0,
                extent.1,
                (third.x + 2, third.y + ROW_HEIGHT / 2)
            ),
            Some(ObjectListClick::Select(ObjectId::new(3)))
        );
        // Past the last row, and outside the client area, select nothing.
        assert_eq!(
            object_list_hit(
                &rows,
                ObjectListScroll::default(),
                extent.0,
                extent.1,
                (10, extent.1 as i32 - 8)
            ),
            None
        );
        assert_eq!(
            object_list_hit(
                &rows,
                ObjectListScroll::default(),
                extent.0,
                extent.1,
                (0, 0)
            ),
            None
        );
        assert_eq!(
            object_list_hit(
                &[],
                ObjectListScroll::default(),
                extent.0,
                extent.1,
                (10, 10)
            ),
            None
        );
    }

    // The row a click resolves to is the row that was drawn there, wherever
    // the retained offset has put the window.
    #[test]
    fn object_list_hit_testing_follows_the_retained_scroll_offset() {
        let rows = object_list_rows(
            &(0..100).map(|id| node(id, vec![])).collect::<Vec<_>>(),
            &ObjectTreeExpansion::default(),
            |id| format!("Object {}", id.as_u64()),
            |_| None,
        );
        let (width, height) = (OBJECT_LIST_WIDTH, 100);
        let mut scroll = ObjectListScroll::default();
        scroll.reveal(80, rows.len(), height);
        let (first, capacity) = scroll.window(rows.len(), height);
        assert_eq!(capacity, 3, "24px icons determine the fixed row metric");
        assert_eq!(
            first + capacity - 1,
            80,
            "revealing puts the row on the last visible line"
        );
        let rect = row_rect(80, first, width);
        assert_eq!(
            object_list_hit(&rows, scroll, width, height, (rect.x + 2, rect.y + 2)),
            Some(ObjectListClick::Select(ObjectId::new(80)))
        );
        // The window never scrolls past the end of the list.
        scroll.reveal(99, rows.len(), height);
        let (first, capacity) = scroll.window(rows.len(), height);
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
            let mut scroll = ObjectListScroll::default();
            scroll.reveal(2, rows.len(), height);
            render_object_list(&mut surface, &font, &rows, &[ObjectId::new(3)], scroll);
            let _ = object_list_hit(&rows, scroll, width, height, (5, 5));
        }
    }
}
