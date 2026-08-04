//! Reusable scrolling text state for classic `C4GUI::InfoDialog`-shaped modals.

use std::cell::{Cell, RefCell};

use crate::classic_gui::IntRect;
use crate::message_dialog::{break_message, break_message_with_options, BreakMessageOptions};
use crate::{GuiPoint, KeyCode};

pub const INFO_SCROLLBAR_EXTENT: i32 = 16;
const TEXT_MARGIN_LEFT: i32 = 10;
const TEXT_MARGIN_RIGHT: i32 = 5;
const TEXT_MARGIN_TOP: i32 = 8;
const TEXT_MARGIN_BOTTOM: i32 = 8;
const INFO_DIALOG_VERTICAL_ROOM: i32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollingInfoGeometry {
    pub frame: IntRect,
    pub viewport: IntRect,
    pub scrollbar: IntRect,
    pub line_height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollingInfoMetrics {
    pub viewport_height: i32,
    pub content_height: i32,
    pub max_scroll: i32,
    pub scroll_y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleInfoLine {
    pub text: String,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoScrollTarget {
    Up,
    Down,
    Track,
}

/// Text/controller portion of the classic information-dialog chassis.
///
/// The stored offset is an absolute logical-pixel Y position. Replacing text
/// retains it until the next metrics pass clamps it against the new content,
/// matching `BeginUpdateText`/`EndUpdateText` rather than preserving a ratio.
#[derive(Clone, Debug)]
pub struct ScrollingInfoDialog {
    caption: String,
    requested_line_count: usize,
    live_updates: bool,
    lines: Vec<String>,
    wrapped_width: Cell<Option<i32>>,
    wrapped_lines: RefCell<Vec<String>>,
    scroll_y: Cell<i32>,
}

impl ScrollingInfoDialog {
    pub fn new(
        caption: impl Into<String>,
        requested_line_count: usize,
        live_updates: bool,
    ) -> Self {
        Self {
            caption: caption.into(),
            requested_line_count: requested_line_count.max(1),
            live_updates,
            lines: Vec::new(),
            wrapped_width: Cell::new(None),
            wrapped_lines: RefCell::new(Vec::new()),
            scroll_y: Cell::new(0),
        }
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub fn set_caption(&mut self, caption: impl Into<String>) {
        self.caption = caption.into();
    }

    pub const fn requested_line_count(&self) -> usize {
        self.requested_line_count
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn reset_lines(&mut self, lines: Vec<String>) {
        self.lines = lines;
        self.clear_wrapping();
        self.scroll_y.set(0);
    }

    pub fn reset_scroll(&self) {
        self.scroll_y.set(0);
    }

    pub fn replace_lines_preserving_scroll(&mut self, lines: Vec<String>) {
        self.lines = lines;
        self.clear_wrapping();
    }

    /// Scheduler hook for dynamic InfoDialog consumers. Static callers set
    /// `live_updates` to false and retain their original text.
    pub fn on_sec1_timer(&mut self, update: impl FnOnce() -> Vec<String>) -> bool {
        if !self.live_updates {
            return false;
        }
        self.replace_lines_preserving_scroll(update());
        true
    }

    /// Native total dialog height before available-screen clamping.
    pub fn preferred_dialog_height(&self, line_height: i32) -> i32 {
        i32::try_from(self.requested_line_count)
            .unwrap_or(i32::MAX)
            .saturating_mul(line_height.max(1))
            .saturating_add(INFO_DIALOG_VERTICAL_ROOM)
    }

    /// Height of the native text-window allocation before the dialog's outer
    /// space for its close controls.
    pub fn preferred_text_window_height(&self, line_height: i32) -> i32 {
        i32::try_from(self.requested_line_count)
            .unwrap_or(i32::MAX)
            .saturating_mul(line_height.max(1))
            .saturating_add(TEXT_MARGIN_TOP + TEXT_MARGIN_BOTTOM)
    }

    /// `TextWindow` client geometry: L10/T8/R5/B8 margins followed by the
    /// permanently reserved 16-pixel `ScrollWindow` scrollbar column.
    pub fn geometry(&self, frame: IntRect, line_height: i32) -> ScrollingInfoGeometry {
        let client = IntRect {
            x: frame.x.saturating_add(TEXT_MARGIN_LEFT),
            y: frame.y.saturating_add(TEXT_MARGIN_TOP),
            w: frame
                .w
                .saturating_sub(TEXT_MARGIN_LEFT + TEXT_MARGIN_RIGHT)
                .max(1),
            h: frame
                .h
                .saturating_sub(TEXT_MARGIN_TOP + TEXT_MARGIN_BOTTOM)
                .max(1),
        };
        let scrollbar_width = INFO_SCROLLBAR_EXTENT.min(client.w).max(1);
        ScrollingInfoGeometry {
            frame,
            viewport: IntRect {
                x: client.x,
                y: client.y,
                w: client.w.saturating_sub(scrollbar_width).max(1),
                h: client.h,
            },
            scrollbar: IntRect {
                x: client
                    .x
                    .saturating_add(client.w.saturating_sub(scrollbar_width)),
                y: client.y,
                w: scrollbar_width,
                h: client.h,
            },
            line_height: line_height.max(1),
        }
    }

    pub fn physical_lines(&self) -> Vec<String> {
        if self.wrapped_width.get().is_some() {
            self.wrapped_lines.borrow().clone()
        } else {
            self.lines.clone()
        }
    }

    /// Rebuilds TextWindow-style physical rows for the current viewport.
    /// Pipe separation is handled by the static InfoDialog constructor;
    /// AddTextLine additionally honors CR/LF and wraps long logical rows.
    pub fn prepare_wrapped_lines(&self, font: &clonk_graphics::clonk_font::ClonkFont, width: i32) {
        let width = width.max(1);
        if self.wrapped_width.get() == Some(width) {
            return;
        }
        let mut physical = Vec::new();
        for logical in &self.lines {
            for paragraph in logical.split(['\r', '\n']).filter(|line| !line.is_empty()) {
                // C4LogBuffer first gives the unindented row the complete
                // width, then wraps the untouched suffix against the width
                // remaining after its two-space continuation prefix.
                let first_break = break_message_with_options(
                    font,
                    paragraph,
                    width,
                    BreakMessageOptions {
                        max_lines: 1,
                        ..BreakMessageOptions::default()
                    },
                );
                let first_separator = first_break.char_indices().find_map(|(index, character)| {
                    matches!(character, '\r' | '\n').then_some(index)
                });
                let Some(separator) = first_separator else {
                    if !first_break.is_empty() {
                        physical.push(first_break);
                    }
                    continue;
                };
                let first = &first_break[..separator];
                if !first.is_empty() {
                    physical.push(first.to_string());
                }
                let suffix_start = separator
                    + first_break[separator..]
                        .chars()
                        .next()
                        .map(char::len_utf8)
                        .unwrap_or_default();
                let suffix = &first_break[suffix_start..];
                if suffix.is_empty() {
                    continue;
                }
                let indent_width = font.measure("  ", true).0.max(0);
                let wrapped_suffix =
                    break_message(font, suffix, width.saturating_sub(indent_width));
                for line in wrapped_suffix
                    .split(['\r', '\n'])
                    .filter(|line| !line.is_empty())
                {
                    physical.push(format!("  {line}"));
                }
            }
        }
        *self.wrapped_lines.borrow_mut() = physical;
        self.wrapped_width.set(Some(width));
    }

    fn clear_wrapping(&self) {
        self.wrapped_width.set(None);
        self.wrapped_lines.borrow_mut().clear();
    }

    pub fn metrics(&self, geometry: &ScrollingInfoGeometry) -> ScrollingInfoMetrics {
        let physical_count = i32::try_from(self.physical_lines().len()).unwrap_or(i32::MAX);
        let content_height = physical_count.saturating_mul(geometry.line_height).max(1);
        let max_scroll = content_height.saturating_sub(geometry.viewport.h).max(0);
        let scroll_y = self.scroll_y.get().clamp(0, max_scroll);
        self.scroll_y.set(scroll_y);
        ScrollingInfoMetrics {
            viewport_height: geometry.viewport.h,
            content_height,
            max_scroll,
            scroll_y,
        }
    }

    pub fn visible_lines(&self, geometry: &ScrollingInfoGeometry) -> Vec<VisibleInfoLine> {
        let metrics = self.metrics(geometry);
        let viewport_bottom = geometry.viewport.y.saturating_add(geometry.viewport.h);
        self.physical_lines()
            .into_iter()
            .enumerate()
            .filter_map(|(index, text)| {
                let y = geometry
                    .viewport
                    .y
                    .saturating_add(
                        i32::try_from(index)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(geometry.line_height),
                    )
                    .saturating_sub(metrics.scroll_y);
                (y.saturating_add(geometry.line_height) > geometry.viewport.y
                    && y < viewport_bottom)
                    .then_some(VisibleInfoLine { text, y })
            })
            .collect()
    }

    pub fn handle_wheel(&self, delta: i32, geometry: &ScrollingInfoGeometry) {
        self.scroll_by(delta.saturating_neg(), geometry);
    }

    /// Keyboard scrolling is an accessibility requirement of the scrolling-info
    /// chassis, not a port of C++: native TextWindow has no focus element and
    /// only handles pointer input.
    pub fn handle_key(&self, key: KeyCode, geometry: &ScrollingInfoGeometry) -> bool {
        match key {
            KeyCode::Up => self.scroll_by(-geometry.line_height, geometry),
            KeyCode::Down => self.scroll_by(geometry.line_height, geometry),
            KeyCode::PageUp => self.scroll_by(-geometry.viewport.h, geometry),
            KeyCode::PageDown => self.scroll_by(geometry.viewport.h, geometry),
            KeyCode::Home => self.set_scroll_y(0, geometry),
            KeyCode::End => {
                let max_scroll = self.metrics(geometry).max_scroll;
                self.set_scroll_y(max_scroll, geometry);
            }
            _ => return false,
        }
        true
    }

    pub fn scroll_target_at(
        &self,
        point: GuiPoint,
        geometry: &ScrollingInfoGeometry,
    ) -> Option<InfoScrollTarget> {
        if self.metrics(geometry).max_scroll == 0 || !contains(geometry.scrollbar, point) {
            return None;
        }
        if point.y < geometry.scrollbar.y.saturating_add(INFO_SCROLLBAR_EXTENT) as f32 {
            Some(InfoScrollTarget::Up)
        } else if point.y
            >= geometry
                .scrollbar
                .y
                .saturating_add(geometry.scrollbar.h)
                .saturating_sub(INFO_SCROLLBAR_EXTENT) as f32
        {
            Some(InfoScrollTarget::Down)
        } else {
            Some(InfoScrollTarget::Track)
        }
    }

    pub fn activate_scroll_target(
        &self,
        target: InfoScrollTarget,
        point: GuiPoint,
        geometry: &ScrollingInfoGeometry,
    ) {
        match target {
            InfoScrollTarget::Up => self.scroll_by(-geometry.line_height, geometry),
            InfoScrollTarget::Down => self.scroll_by(geometry.line_height, geometry),
            InfoScrollTarget::Track => self.set_scroll_from_pointer(point, geometry),
        }
    }

    pub fn set_scroll_from_pointer(&self, point: GuiPoint, geometry: &ScrollingInfoGeometry) {
        let metrics = self.metrics(geometry);
        let max_pin = geometry
            .scrollbar
            .h
            .saturating_sub(3 * INFO_SCROLLBAR_EXTENT)
            .max(0);
        if metrics.max_scroll == 0 || max_pin == 0 {
            self.scroll_y.set(0);
            return;
        }
        let pin = (point.y.floor() as i32)
            .saturating_sub(geometry.scrollbar.y)
            .saturating_sub(INFO_SCROLLBAR_EXTENT + INFO_SCROLLBAR_EXTENT / 2)
            .clamp(0, max_pin);
        self.scroll_y
            .set(metrics.max_scroll.saturating_mul(pin) / max_pin);
    }

    pub fn scrollbar_pin(&self, geometry: &ScrollingInfoGeometry) -> i32 {
        let metrics = self.metrics(geometry);
        let max_pin = geometry
            .scrollbar
            .h
            .saturating_sub(3 * INFO_SCROLLBAR_EXTENT)
            .max(0);
        if metrics.max_scroll == 0 || max_pin == 0 {
            0
        } else {
            max_pin.saturating_mul(metrics.scroll_y) / metrics.max_scroll
        }
    }

    fn scroll_by(&self, amount: i32, geometry: &ScrollingInfoGeometry) {
        let current = self.metrics(geometry).scroll_y;
        self.set_scroll_y(current.saturating_add(amount), geometry);
    }

    fn set_scroll_y(&self, scroll_y: i32, geometry: &ScrollingInfoGeometry) {
        let max_scroll = self.metrics(geometry).max_scroll;
        self.scroll_y.set(scroll_y.clamp(0, max_scroll));
    }
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x < rect.x.saturating_add(rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < rect.y.saturating_add(rect.h) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(dialog: &ScrollingInfoDialog) -> ScrollingInfoGeometry {
        dialog.geometry(
            IntRect {
                x: 0,
                y: 0,
                w: 200,
                h: 116,
            },
            10,
        )
    }

    #[test]
    fn scrolling_info_reaches_every_line_with_wheel_and_keyboard() {
        let mut dialog = ScrollingInfoDialog::new("Information", 10, true);
        assert_eq!(dialog.requested_line_count(), 10);
        dialog.reset_lines((0..17).map(|index| format!("line-{index:02}")).collect());
        let geometry = geometry(&dialog);

        assert_eq!(
            dialog
                .visible_lines(&geometry)
                .last()
                .map(|line| line.text.as_str()),
            Some("line-09")
        );
        dialog.handle_wheel(-60, &geometry);
        assert!(dialog
            .visible_lines(&geometry)
            .iter()
            .any(|line| line.text == "line-15"));
        assert!(dialog.handle_key(KeyCode::End, &geometry));
        assert_eq!(
            dialog
                .visible_lines(&geometry)
                .last()
                .map(|line| line.text.as_str()),
            Some("line-16")
        );
        assert!(dialog.handle_key(KeyCode::Home, &geometry));
        assert_eq!(dialog.metrics(&geometry).scroll_y, 0);
        assert!(dialog.handle_key(KeyCode::PageDown, &geometry));
        assert!(dialog.metrics(&geometry).scroll_y > 0);
        assert!(dialog.handle_key(KeyCode::PageUp, &geometry));
        assert_eq!(dialog.metrics(&geometry).scroll_y, 0);
    }

    #[test]
    fn scrolling_info_refresh_preserves_absolute_offset_then_clamps() {
        let mut dialog = ScrollingInfoDialog::new("Information", 10, true);
        dialog.reset_lines((0..20).map(|index| format!("old-{index:02}")).collect());
        let geometry = geometry(&dialog);
        dialog.handle_wheel(-37, &geometry);
        assert_eq!(dialog.metrics(&geometry).scroll_y, 37);

        assert!(
            dialog.on_sec1_timer(|| { (0..20).map(|index| format!("new-{index:02}")).collect() })
        );
        assert_eq!(dialog.metrics(&geometry).scroll_y, 37);
        assert!(dialog.lines()[0].starts_with("new-"));

        dialog.replace_lines_preserving_scroll(vec!["short".to_string()]);
        assert_eq!(dialog.metrics(&geometry).scroll_y, 0);

        let mut static_dialog = ScrollingInfoDialog::new("Static", 10, false);
        static_dialog.reset_lines(vec!["original".to_string()]);
        assert!(!static_dialog.on_sec1_timer(|| vec!["changed".to_string()]));
        assert_eq!(static_dialog.lines(), ["original"]);
    }
}
