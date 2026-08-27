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
