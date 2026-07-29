//! The classic `C4GUI::ScrollBar`, shared by every overflowing menu.
//!
//! Two independent copies of this existed: the drawing half in
//! `game_over.rs` (`draw_classic_scrollbar`, pinned by the evaluation-dialog
//! tests) and the interaction half in `clonk-frontend`'s startup chat
//! transcript. They agreed on the arithmetic, so this module is the promotion
//! of both rather than new pixel logic.
//!
//! `C4GuiContainers.cpp:309-470` draws the facet's three 16px cells — up arrow,
//! tiled shaft, down arrow — with the pin taken from column 16 of the shaft
//! row. `:477-623` defines the pointer regions: the two arrow buttons, the
//! draggable pin, and the pageable track between them.

use clonk_frontend::classic_gui::IntRect;

/// `C4GUI_ScrollArrowHgt` — the facet cell size, and the arrow button extent.
pub const SCROLLBAR_EXTENT: i32 = 16;

/// Builds a bar rectangle from plain coordinates, so callers holding a
/// different rectangle type need not depend on `IntRect`'s shape.
pub fn bar_rect(x: i32, y: i32, w: i32, h: i32) -> IntRect {
    IntRect { x, y, w, h }
}

/// Which part of the bar a pointer landed on (`C4GuiContainers.cpp:477-623`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollbarHit {
    /// The upper arrow button: steps back one line.
    ScrollUp,
    /// The lower arrow button: steps forward one line.
    ScrollDown,
    /// The pin itself: begins a drag.
    Pin,
    /// The bare track: pages toward the pointer.
    Track,
}

/// The arrow-button extent for a bar of this height. A bar too short for two
/// full arrows splits evenly, which is what the chat transcript already did.
fn arrow_extent(height: i32) -> i32 {
    SCROLLBAR_EXTENT.min(height / 2)
}

/// The distance the pin can travel: the shaft less the two arrows and the pin
/// itself (`C4GUI::ScrollBar::Update`).
pub fn pin_travel(height: i32) -> i32 {
    (height - 3 * arrow_extent(height)).max(0)
}

/// The pin's offset from the top of the shaft, proportional to the scrolled
/// fraction.
pub fn pin_offset(height: i32, scroll: i32, max_scroll: i32) -> i32 {
    let travel = pin_travel(height);
    if max_scroll <= 0 || travel <= 0 {
        return 0;
    }
    let clamped = scroll.clamp(0, max_scroll);
    (i64::from(clamped) * i64::from(travel) / i64::from(max_scroll)) as i32
}

/// The pin's rectangle within `bar`.
pub fn pin_rect(bar: IntRect, scroll: i32, max_scroll: i32) -> IntRect {
    let arrow = arrow_extent(bar.h);
    IntRect {
        x: bar.x,
        y: bar.y + arrow + pin_offset(bar.h, scroll, max_scroll),
        w: bar.w,
        h: arrow,
    }
}

/// Which region `point` falls in. `None` when the bar is absent — C++ only
/// shows it while the content overflows — or the pointer is outside it.
pub fn hit(bar: IntRect, point: (i32, i32), scroll: i32, max_scroll: i32) -> Option<ScrollbarHit> {
    let (x, y) = point;
    if max_scroll <= 0
        || bar.w <= 0
        || bar.h <= 0
        || x < bar.x
        || x >= bar.x + bar.w
        || y < bar.y
        || y >= bar.y + bar.h
    {
        return None;
    }
    let arrow = arrow_extent(bar.h);
    if y < bar.y + arrow {
        return Some(ScrollbarHit::ScrollUp);
    }
    if y >= bar.y + bar.h - arrow {
        return Some(ScrollbarHit::ScrollDown);
    }
    let pin = pin_rect(bar, scroll, max_scroll);
    Some(if y >= pin.y && y < pin.y + pin.h {
        ScrollbarHit::Pin
    } else {
        ScrollbarHit::Track
    })
}

/// The scroll a pin drag to `pointer_y` selects, clamped to the range. The
/// pointer is taken to hold the pin's centre, as `C4GUI::ScrollBar::MouseInput`
/// does while captured.
pub fn scroll_from_pointer(bar: IntRect, pointer_y: i32, max_scroll: i32) -> i32 {
    let travel = pin_travel(bar.h);
    if max_scroll <= 0 || travel <= 0 {
        return 0;
    }
    let arrow = arrow_extent(bar.h);
    let offset = (pointer_y - bar.y - arrow - arrow / 2).clamp(0, travel);
    ((i64::from(offset) * i64::from(max_scroll)) / i64::from(travel)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: IntRect = IntRect {
        x: 100,
        y: 40,
        w: SCROLLBAR_EXTENT,
        h: 160,
    };

    // C4GuiContainers.cpp:309-470,477-623 — the arrow buttons, the pageable
    // track, and the proportional pin, including their boundaries.
    #[test]
    fn overflow_menu_scrollbar_arrows_track_and_thumb_match_cpp() {
        // No bar while the content fits: every pointer misses (:477-480).
        assert_eq!(hit(BAR, (105, 100), 0, 0), None);
        // Outside the bar in either axis.
        assert_eq!(hit(BAR, (99, 100), 0, 40), None);
        assert_eq!(hit(BAR, (105, 39), 0, 40), None);
        assert_eq!(hit(BAR, (105, 200), 0, 40), None);

        // The arrows own the first and last `SCROLLBAR_EXTENT` rows, and the
        // boundary row belongs to the shaft.
        assert_eq!(hit(BAR, (105, 40), 0, 40), Some(ScrollbarHit::ScrollUp));
        assert_eq!(hit(BAR, (105, 55), 0, 40), Some(ScrollbarHit::ScrollUp));
        assert_eq!(hit(BAR, (105, 56), 0, 40), Some(ScrollbarHit::Pin));
        assert_eq!(hit(BAR, (105, 199), 0, 40), Some(ScrollbarHit::ScrollDown));
        assert_eq!(hit(BAR, (105, 184), 0, 40), Some(ScrollbarHit::ScrollDown));

        // At scroll 0 the pin sits directly under the up arrow; below it the
        // bare track pages.
        let pin = pin_rect(BAR, 0, 40);
        assert_eq!(pin.y, BAR.y + SCROLLBAR_EXTENT);
        assert_eq!(pin.h, SCROLLBAR_EXTENT);
        assert_eq!(
            hit(BAR, (105, pin.y + pin.h), 0, 40),
            Some(ScrollbarHit::Track)
        );

        // The pin travels the shaft proportionally and reaches the bottom
        // exactly at max scroll.
        let travel = pin_travel(BAR.h);
        assert_eq!(travel, 160 - 3 * SCROLLBAR_EXTENT);
        assert_eq!(pin_offset(BAR.h, 0, 40), 0);
        assert_eq!(pin_offset(BAR.h, 20, 40), travel / 2);
        assert_eq!(pin_offset(BAR.h, 40, 40), travel);
        // Out-of-range scroll clamps rather than running off the shaft.
        assert_eq!(pin_offset(BAR.h, 400, 40), travel);
        assert_eq!(pin_offset(BAR.h, -10, 40), 0);

        // A drag maps the pointer back to a scroll and clamps at both ends.
        assert_eq!(scroll_from_pointer(BAR, BAR.y, 40), 0);
        assert_eq!(scroll_from_pointer(BAR, BAR.y + BAR.h, 40), 40);
        let middle = BAR.y + SCROLLBAR_EXTENT + SCROLLBAR_EXTENT / 2 + travel / 2;
        assert_eq!(scroll_from_pointer(BAR, middle, 40), 20);

        // Round trip: a pin placed from a scroll maps back to that scroll.
        for scroll in [0, 7, 20, 33, 40] {
            let placed = pin_rect(BAR, scroll, 40);
            let recovered = scroll_from_pointer(BAR, placed.y + placed.h / 2, 40);
            assert!(
                (recovered - scroll).abs() <= 1,
                "scroll {scroll} recovered as {recovered}"
            );
        }

        // A bar too short for two full arrows splits evenly and never produces
        // a negative travel.
        let short = IntRect { h: 10, ..BAR };
        assert_eq!(pin_travel(short.h), 0);
        assert_eq!(pin_offset(short.h, 5, 40), 0);
        assert_eq!(scroll_from_pointer(short, short.y + 5, 40), 0);
        assert_eq!(
            hit(short, (105, short.y), 0, 40),
            Some(ScrollbarHit::ScrollUp)
        );
    }
}
