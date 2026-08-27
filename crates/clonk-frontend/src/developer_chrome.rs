//! The software chrome the developer windows are drawn in.
//!
//! The native console is Win32 or GTK widgets, so its look is the toolkit's,
//! not the engine's — and on the reference build there is no console dialog at
//! all (`C4Console::Init`'s bare `#else` arm, `C4Console.cpp:348-350`). The
//! port therefore draws its own, and every developer surface has to draw the
//! *same* one: a console whose menus, a viewport whose context menu and a
//! toolbox whose buttons each invented their own palette would read as three
//! unrelated programs.
//!
//! These are the Win9x common-control primitives that vocabulary needs — a
//! raised button face, a sunken client area, a single-pixel separator and text
//! clipped to its cell. They are deliberately not `C4GUI`: that is the *game's*
//! skin, drawn from the graphics pack, and the editor is not part of the game.

use clonk_graphics::{Color, Surface, TextFont};
use clonk_gui::Rect as GuiRect;

use crate::classic_gui::IntRect;
use crate::{fill_rect, GuiPoint};

/// One row of a dropdown or context menu.
/// A pane's retained first visible line or row.
///
/// Both scrolled developer panes need the same three operations and the same
/// clamping rule, and had a copy each: the property output
/// (`C4PropertyDlg.cpp:257-262`, which reads `EM_GETFIRSTVISIBLELINE` and
/// scrolls back to it) and the object tree (`C4ObjectListDlg.cpp:747-780`,
/// whose position lives in the scrolled window around a model that is rebuilt
/// on every object change).
///
/// Expressed in **lines and capacity** rather than pixels, so a pane supplies
/// its own row metric and this stays the same type for both.
///
/// The position is kept **unclamped**: content that shrinks does not throw
/// away where the user was, so content that grows again comes back to it. That
/// is the Win32 edit control's own behaviour, where `EM_LINESCROLL` clamps the
/// scroll it performs without changing what a later, longer text can reach.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneScroll {
    first: usize,
}

impl PaneScroll {
    /// The first visible line for the content as it stands now.
    pub fn window(&self, lines: usize, capacity: usize) -> usize {
        self.first.min(Self::last_top(lines, capacity))
    }

    /// Scroll by whole lines, as a wheel notch or a bar arrow does.
    pub fn scroll_by(&mut self, delta: i32, lines: usize, capacity: usize) {
        let last = Self::last_top(lines, capacity);
        let current = i64::try_from(self.first.min(last)).unwrap_or(i64::MAX);
        let target = current.saturating_add(i64::from(delta)).max(0);
        self.first = usize::try_from(target).unwrap_or(usize::MAX).min(last);
    }

    /// Put an absolute first line, as a thumb drag does.
    pub fn scroll_to(&mut self, first: usize, lines: usize, capacity: usize) {
        self.first = first.min(Self::last_top(lines, capacity));
    }

    /// Scroll one line into view, moving as little as possible.
    ///
    /// `gtk_tree_view_set_cursor` scrolls a row into view rather than centring
    /// it, so a row one past the bottom edge moves the window by one.
    pub fn reveal(&mut self, line: usize, lines: usize, capacity: usize) {
        let last = Self::last_top(lines, capacity);
        let first = self.first.min(last);
        self.first = if line < first {
            line
        } else if line >= first + capacity {
            (line + 1 - capacity).min(last)
        } else {
            first
        };
    }

    /// The highest first line that still fills the view.
    fn last_top(lines: usize, capacity: usize) -> usize {
        lines.saturating_sub(capacity)
    }
}

/// One pane scroll bar: the track it runs in and the thumb inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneScrollBar {
    pub track: IntRect,
    pub thumb: IntRect,
    /// The content and page the thumb was built from, so a press can be
    /// answered without the caller re-deriving them.
    pub lines: usize,
    pub capacity: usize,
}

/// The region of a pane bar a press landed on.
///
/// The same four a Win32 scroll bar divides into, and the same names the
/// detached viewport bars use — the panes are a different widget but not a
/// different vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneScrollPart {
    LineBack,
    LineForward,
    PageBack,
    PageForward,
    Thumb,
}

/// How much of each end of a pane track is an arrow button.
const PANE_ARROW_EXTENT: i32 = 10;

/// The shortest thumb that still reads as a thumb rather than a mark.
const PANE_MINIMUM_THUMB: i32 = 6;

/// A bar for content already scrolled to `first`, or `None` when it fits.
///
/// Both panes sit in `GTK_POLICY_AUTOMATIC` scrolled windows
/// (`C4ObjectListDlg.cpp:748`; `C4PropertyDlg.cpp:128-140`), so the bar exists
/// only when there is something it could reach.
pub fn pane_scroll_bar_at(
    track: IntRect,
    lines: usize,
    capacity: usize,
    first: usize,
) -> Option<PaneScrollBar> {
    if capacity == 0 || lines <= capacity || track.h <= 0 || track.w <= 0 {
        return None;
    }
    let reach = lines - capacity;
    let extent = (i64::from(track.h) * capacity as i64 / lines as i64) as i32;
    let extent = extent.clamp(PANE_MINIMUM_THUMB.min(track.h), track.h);
    let travel = (track.h - extent).max(0);
    let offset = (i64::from(travel) * first.min(reach) as i64 / reach as i64) as i32;
    Some(PaneScrollBar {
        track,
        thumb: IntRect::new(track.x, track.y + offset, track.w, extent),
        lines,
        capacity,
    })
}

/// A bar for content at the top — the shape a caller wants when it only needs
/// to know whether a bar exists at all.
pub fn pane_scroll_bar(track: IntRect, lines: usize, capacity: usize) -> Option<PaneScrollBar> {
    pane_scroll_bar_at(track, lines, capacity, 0)
}

/// Which part of a bar a press landed on, or `None` for the pane itself.
pub fn pane_scroll_bar_press(bar: &PaneScrollBar, point: (i32, i32)) -> Option<PaneScrollPart> {
    let track = bar.track;
    let inside = point.0 >= track.x
        && point.0 < track.x + track.w
        && point.1 >= track.y
        && point.1 < track.y + track.h;
    if !inside {
        return None;
    }
    // The arrows never take more than a third of the bar each, so a short
    // track stays usable as a track.
    let arrow = PANE_ARROW_EXTENT.min(track.h / 3);
    if arrow > 0 && point.1 < track.y + arrow {
        return Some(PaneScrollPart::LineBack);
    }
    if arrow > 0 && point.1 >= track.y + track.h - arrow {
        return Some(PaneScrollPart::LineForward);
    }
    if point.1 >= bar.thumb.y && point.1 < bar.thumb.y + bar.thumb.h {
        return Some(PaneScrollPart::Thumb);
    }
    Some(if point.1 < bar.thumb.y {
        PaneScrollPart::PageBack
    } else {
        PaneScrollPart::PageForward
    })
}

/// The first line a thumb at this pointer position names.
///
/// The position follows the pointer rather than accumulating a delta, so a
/// drag that leaves the bar and comes back lands where the pointer is.
pub fn pane_scroll_bar_line(bar: &PaneScrollBar, y: i32) -> usize {
    let reach = bar.lines.saturating_sub(bar.capacity);
    let span = bar.track.h.max(1);
    let along = (y - bar.track.y).clamp(0, span);
    (i64::from(along) * reach as i64 / i64::from(span)) as usize
}

/// The two arrow boxes of a pane bar, in drawing order.
pub fn pane_scroll_bar_arrows(bar: &PaneScrollBar) -> [IntRect; 2] {
    let track = bar.track;
    let arrow = PANE_ARROW_EXTENT.min(track.h / 3).max(0);
    [
        IntRect::new(track.x, track.y, track.w, arrow),
        IntRect::new(track.x, track.y + track.h - arrow, track.w, arrow),
    ]
}

pub const MENU_ITEM_HEIGHT: i32 = 22;
/// A separator row, which is shorter than an item and carries no text.
pub const MENU_SEPARATOR_HEIGHT: i32 = 8;
pub const FONT_SIZE: f32 = 13.0;
pub const SMALL_FONT_SIZE: f32 = 11.0;

pub const WINDOW_BACKGROUND: Color = Color::opaque(0xd4, 0xd0, 0xc8);
pub const CONTROL_BACKGROUND: Color = Color::opaque(0xff, 0xff, 0xff);
pub const CONTROL_TEXT: Color = Color::opaque(0x10, 0x10, 0x10);
pub const DISABLED_TEXT: Color = Color::opaque(0x78, 0x78, 0x78);
pub const SELECTED_BACKGROUND: Color = Color::opaque(0x31, 0x6a, 0xc5);
pub const SELECTED_TEXT: Color = Color::opaque(0xff, 0xff, 0xff);
pub const LIGHT_EDGE: Color = Color::opaque(0xff, 0xff, 0xff);
pub const DARK_EDGE: Color = Color::opaque(0x60, 0x60, 0x60);
pub const MID_EDGE: Color = Color::opaque(0x9a, 0x9a, 0x9a);

/// Whether `point` lands inside `rect`, right and bottom edges exclusive.
pub fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < rect.x.saturating_add(rect.w) as f32
        && point.y < rect.y.saturating_add(rect.h) as f32
}

pub fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(
        rect.x as f32,
        rect.y as f32,
        rect.w.max(0) as f32,
        rect.h.max(0) as f32,
    )
}

pub fn fill(surface: &mut Surface, rect: IntRect, color: Color) {
    if rect.w > 0 && rect.h > 0 {
        fill_rect(surface, &gui_rect(rect), color);
    }
}

pub fn draw_bottom_line(surface: &mut Surface, rect: IntRect, color: Color) {
    fill(
        surface,
        IntRect::new(rect.x, rect.y + rect.h - 1, rect.w, 1),
        color,
    );
}

/// A button face: light on the top and left, dark on the bottom and right.
pub fn draw_raised(surface: &mut Surface, rect: IntRect, color: Color) {
    fill(surface, rect, color);
    fill(surface, IntRect::new(rect.x, rect.y, rect.w, 1), LIGHT_EDGE);
    fill(surface, IntRect::new(rect.x, rect.y, 1, rect.h), LIGHT_EDGE);
    fill(
        surface,
        IntRect::new(rect.x, rect.y + rect.h - 1, rect.w, 1),
        DARK_EDGE,
    );
    fill(
        surface,
        IntRect::new(rect.x + rect.w - 1, rect.y, 1, rect.h),
        DARK_EDGE,
    );
}

/// A client area: the raised edges inverted, which is what makes a pressed
/// button and an editable field look the same way in.
pub fn draw_sunken(surface: &mut Surface, rect: IntRect, color: Color) {
    fill(surface, rect, color);
    fill(surface, IntRect::new(rect.x, rect.y, rect.w, 1), DARK_EDGE);
    fill(surface, IntRect::new(rect.x, rect.y, 1, rect.h), DARK_EDGE);
    fill(
        surface,
        IntRect::new(rect.x, rect.y + rect.h - 1, rect.w, 1),
        LIGHT_EDGE,
    );
    fill(
        surface,
        IntRect::new(rect.x + rect.w - 1, rect.y, 1, rect.h),
        LIGHT_EDGE,
    );
}

/// Draw `text` inside `rect`, clipped to whole characters that fit.
///
/// Truncation is by measurement rather than an ellipsis: a Win32 static
/// control clips, and a label that grew an ellipsis would change width with
/// the font.
pub fn draw_fitted_text(
    surface: &mut Surface,
    font: &dyn TextFont,
    rect: IntRect,
    text: &str,
    color: Color,
    size: f32,
    padding: i32,
) {
    if rect.w <= padding * 2 || rect.h <= 0 {
        return;
    }
    let available = (rect.w - padding * 2) as f32;
    let mut fitted = String::new();
    for character in text.chars() {
        let mut candidate = fitted.clone();
        candidate.push(character);
        if font.measure_text(&candidate, size).width > available {
            break;
        }
        fitted.push(character);
    }
    font.draw_text(
        surface,
        (rect.x + padding) as f32,
        (rect.y + ((rect.h as f32 - size) / 2.0).max(1.0) as i32) as f32,
        &fitted,
        size,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pane whose content fits shows no bar at all.
    ///
    /// `gtk_scrolled_window_set_policy(..., GTK_POLICY_AUTOMATIC,
    /// GTK_POLICY_AUTOMATIC)` (`C4ObjectListDlg.cpp:748`) is exactly that
    /// policy: the bar appears only when there is something it could reach.
    #[test]
    fn a_pane_bar_appears_only_when_the_content_overflows() {
        let track = IntRect::new(100, 0, 10, 200);
        assert_eq!(pane_scroll_bar(track, 6, 10), None, "six lines in ten fit");
        assert_eq!(pane_scroll_bar(track, 10, 10), None, "exactly a page fits");
        let bar = pane_scroll_bar(track, 40, 10).expect("forty lines in ten do not");
        assert!(bar.thumb.h > 0 && bar.thumb.h <= track.h);
        assert!(bar.thumb.y >= track.y);
        assert!(bar.thumb.y + bar.thumb.h <= track.y + track.h);
    }

    /// The thumb carries the visible proportion and the position.
    #[test]
    fn a_pane_thumb_reports_the_page_and_the_position() {
        let track = IntRect::new(0, 0, 10, 100);
        let top = pane_scroll_bar_at(track, 100, 10, 0).expect("overflowing");
        let bottom = pane_scroll_bar_at(track, 100, 10, 90).expect("overflowing");
        assert_eq!(top.thumb.y, track.y, "the top of the content is the top");
        assert_eq!(
            bottom.thumb.y + bottom.thumb.h,
            track.y + track.h,
            "and the last page reaches the bottom"
        );
        assert!(
            top.thumb.h < track.h / 2,
            "a tenth of the content is a short thumb: {}",
            top.thumb.h
        );
    }

    /// A press resolves to the same four regions the viewport bars use.
    #[test]
    fn a_pane_bar_press_resolves_to_a_line_page_or_thumb() {
        let track = IntRect::new(0, 0, 10, 100);
        let bar = pane_scroll_bar_at(track, 100, 10, 40).expect("overflowing");

        assert_eq!(
            pane_scroll_bar_press(&bar, (5, track.y + 1)),
            Some(PaneScrollPart::LineBack)
        );
        assert_eq!(
            pane_scroll_bar_press(&bar, (5, track.y + track.h - 2)),
            Some(PaneScrollPart::LineForward)
        );
        assert_eq!(
            pane_scroll_bar_press(&bar, (5, bar.thumb.y + bar.thumb.h / 2)),
            Some(PaneScrollPart::Thumb)
        );
        assert_eq!(
            pane_scroll_bar_press(&bar, (5, bar.thumb.y - 1)),
            Some(PaneScrollPart::PageBack)
        );
        assert_eq!(
            pane_scroll_bar_press(&bar, (5, bar.thumb.y + bar.thumb.h + 1)),
            Some(PaneScrollPart::PageForward)
        );
        assert_eq!(
            pane_scroll_bar_press(&bar, (-5, bar.thumb.y)),
            None,
            "outside the track is the pane's, not the bar's"
        );
    }

    /// A drag names an absolute first line, clamped at both ends.
    #[test]
    fn a_pane_thumb_drag_names_a_first_line() {
        let track = IntRect::new(0, 0, 10, 100);
        let bar = pane_scroll_bar_at(track, 100, 10, 0).expect("overflowing");
        assert_eq!(pane_scroll_bar_line(&bar, track.y), 0);
        assert_eq!(
            pane_scroll_bar_line(&bar, track.y + track.h),
            90,
            "the bottom is as far as a full page can reach"
        );
        assert_eq!(pane_scroll_bar_line(&bar, track.y - 50), 0, "clamped above");
        assert_eq!(
            pane_scroll_bar_line(&bar, track.y + track.h + 50),
            90,
            "and below"
        );
    }
}
