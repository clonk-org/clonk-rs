//! Per-viewport player lock, scrollbars and event routing.
//!
//! `C4Viewport::TogglePlayerLock` (`C4Viewport.cpp:250-267`) flips the lock and
//! shows or hides the window's scroll bars with it. Note the asymmetry: a
//! locked viewport always unlocks, but locking requires `ValidPlr(Player)` —
//! an ownerless (`NO_OWNER`) viewport can never be locked, and the call still
//! reports success.
//!
//! `ScrollBarsByViewPosition` (`:270-...`) refuses outright while locked, and
//! otherwise ranges each bar over the landscape with the view extent as its
//! page and the view origin as its position.

use crate::developer_cursor::CursorMode;

/// The result of toggling a viewport's player lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerLockState {
    pub locked: bool,
    /// Locked viewports hide their scroll bars (`WS_HSCROLL | WS_VSCROLL`
    /// cleared, `C4Viewport.cpp:264`).
    pub scrollbars_visible: bool,
}

/// `C4Viewport::TogglePlayerLock` (`C4Viewport.cpp:250-267`).
///
/// `has_valid_player` is `ValidPlr(Player)`. An ownerless viewport cannot lock,
/// so it stays unlocked with its bars shown.
pub fn toggle_player_lock(locked: bool, has_valid_player: bool) -> PlayerLockState {
    if locked {
        // Unlocking always succeeds and restores the bars.
        return PlayerLockState {
            locked: false,
            scrollbars_visible: true,
        };
    }
    if has_valid_player {
        return PlayerLockState {
            locked: true,
            scrollbars_visible: false,
        };
    }
    // `else if (ValidPlr(Player))` failed: nothing changed.
    PlayerLockState {
        locked: false,
        scrollbars_visible: true,
    }
}

/// One scroll bar's range (`SCROLLINFO` in `ScrollBarsByViewPosition`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrollRange {
    /// `nMin`, always zero.
    pub min: i32,
    /// `nMax` — the landscape extent.
    pub max: i32,
    /// `nPage` — the visible extent.
    pub page: i32,
    /// `nPos` — the view origin.
    pub position: i32,
}

/// Both scroll bars for a viewport, or `None` while the player lock is on —
/// `ScrollBarsByViewPosition` returns false immediately when locked
/// (`C4Viewport.cpp:272`).
pub fn scroll_ranges(
    locked: bool,
    view_x: i32,
    view_y: i32,
    view_width: i32,
    view_height: i32,
    landscape_width: i32,
    landscape_height: i32,
) -> Option<(ScrollRange, ScrollRange)> {
    if locked {
        return None;
    }
    Some((
        ScrollRange {
            min: 0,
            max: landscape_width,
            page: view_width,
            position: view_x,
        },
        ScrollRange {
            min: 0,
            max: landscape_height,
            page: view_height,
            position: view_y,
        },
    ))
}

/// A filled rectangle on the viewport surface, in surface pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BarRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// One bar: the channel it runs in, and the thumb inside it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrollBar {
    pub track: BarRect,
    pub thumb: BarRect,
}

/// Both bars of an unlocked viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrollBarLayout {
    pub horizontal: ScrollBar,
    pub vertical: ScrollBar,
}

/// The shortest thumb that still reads as a thumb rather than a mark.
const MINIMUM_THUMB: i32 = 4;

/// Where an unlocked viewport's bars land on a surface of this size.
///
/// `None` for a locked viewport — this takes what [`scroll_ranges`] returned
/// so the two answers cannot drift apart — and also for a window too small to
/// hold a track, which would otherwise ask for a negative-width rectangle.
///
/// The proportions are `ScrollBarsByViewPosition`'s: the thumb carries `nPage`
/// as its extent against `nMax`, and `nPos` as its offset. The thickness and
/// the colours are not portable — the reference macOS build compiles
/// `ScrollBarsByViewPosition` as `{ return false; }` (`C4Viewport.cpp:634-635`),
/// so there is no C++ presentation to mirror.
pub fn scroll_bar_layout(
    ranges: Option<(ScrollRange, ScrollRange)>,
    surface_width: i32,
    surface_height: i32,
    thickness: i32,
) -> Option<ScrollBarLayout> {
    let (horizontal, vertical) = ranges?;
    // The bars give up their shared corner, so neither draws over the other.
    let horizontal_track = surface_width.checked_sub(thickness)?;
    let vertical_track = surface_height.checked_sub(thickness)?;
    if thickness <= 0 || horizontal_track <= 0 || vertical_track <= 0 {
        return None;
    }

    let horizontal_thumb = thumb_extent(&horizontal, horizontal_track);
    let vertical_thumb = thumb_extent(&vertical, vertical_track);
    Some(ScrollBarLayout {
        horizontal: ScrollBar {
            track: BarRect {
                x: 0,
                y: surface_height - thickness,
                width: horizontal_track,
                height: thickness,
            },
            thumb: BarRect {
                x: horizontal_thumb.0,
                y: surface_height - thickness,
                width: horizontal_thumb.1,
                height: thickness,
            },
        },
        vertical: ScrollBar {
            track: BarRect {
                x: surface_width - thickness,
                y: 0,
                width: thickness,
                height: vertical_track,
            },
            thumb: BarRect {
                x: surface_width - thickness,
                y: vertical_thumb.0,
                width: thickness,
                height: vertical_thumb.1,
            },
        },
    })
}

/// The `(offset, extent)` of one thumb within a track of `track` pixels.
///
/// A landscape no larger than the view has nothing to scroll, so the thumb
/// fills the track — the bar then says "all of it is visible" rather than
/// showing a stub that cannot move.
fn thumb_extent(range: &ScrollRange, track: i32) -> (i32, i32) {
    let max = i64::from(range.max);
    let page = i64::from(range.page).clamp(0, max.max(0));
    let track_len = i64::from(track);
    if max <= 0 || page >= max {
        return (0, track);
    }
    let extent = (page * track_len / max).clamp(i64::from(MINIMUM_THUMB), track_len);
    let scrollable = max - page;
    let position = i64::from(range.position).clamp(0, scrollable);
    let offset = position * (track_len - extent) / scrollable;
    (offset as i32, extent as i32)
}

/// The window a console viewport materialises with (`C4Viewport.cpp:1350-1351`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewportWindowSpec {
    /// `ceilf(400 * scale)` — **ceiling**, not rounding or truncation, so a
    /// fractional scale always rounds the window up.
    pub width: i32,
    /// `ceilf(250 * scale)`.
    pub height: i32,
    /// `IDS_CNS_VIEWPORT` for an ownerless viewport, otherwise the player's
    /// name — so a per-player window is titled after its player, not "Viewport".
    pub title: ViewportWindowTitle,
    /// `std::format("Viewport{}", cvp->Player + 1)`
    /// (`C4ViewportWindow::GetPositionData`). Note the `+ 1`: an ownerless
    /// viewport is `Viewport0`, player 0 is `Viewport1`.
    pub position_id: String,
    /// `Config.GetSubkeyPath("Console")` — viewport geometry lives under the
    /// console's config subkey, not the game's.
    pub position_subkey: &'static str,
    /// `storeSize = true`: the size is remembered along with the position.
    pub store_size: bool,
}

/// Which title an opening viewport window takes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewportWindowTitle {
    /// `IDS_CNS_VIEWPORT`, for `NO_OWNER`.
    Resource,
    /// `Game.Players.Get(Player)->GetName()`.
    PlayerName(String),
}

/// `Config.GetSubkeyPath("Console")`.
pub const VIEWPORT_POSITION_SUBKEY: &str = "Console";

/// The native default viewport window extent before scaling.
pub const VIEWPORT_DEFAULT_WIDTH: f32 = 400.0;
pub const VIEWPORT_DEFAULT_HEIGHT: f32 = 250.0;

/// Builds the spec for a viewport window. `player_name` is consulted only for
/// an owned viewport; an owner with no player row falls back to the resource
/// title rather than an empty one.
pub fn viewport_window_spec(
    player: i32,
    player_name: Option<&str>,
    scale: f32,
) -> ViewportWindowSpec {
    let scaled = |extent: f32| (extent * scale).ceil() as i32;
    ViewportWindowSpec {
        width: scaled(VIEWPORT_DEFAULT_WIDTH),
        height: scaled(VIEWPORT_DEFAULT_HEIGHT),
        title: match (player, player_name) {
            (crate::OWNER_NONE, _) => ViewportWindowTitle::Resource,
            (_, Some(name)) => ViewportWindowTitle::PlayerName(name.to_owned()),
            (_, None) => ViewportWindowTitle::Resource,
        },
        position_id: format!("Viewport{}", player + 1),
        position_subkey: VIEWPORT_POSITION_SUBKEY,
        store_size: true,
    }
}

/// `ViewportScrollSpeed` (`C4Viewport.cpp:57`) — one scroll-bar line step,
/// and GTK's `step_increment` for the same bars (`:316,328`).
pub const VIEWPORT_SCROLL_SPEED: i32 = 10;

/// The scroll step one mouse-wheel message asks for.
///
/// This is presentation the port has to **invent**: C++ scrolls a console
/// viewport through its window's native scroll bars, and the reference macOS
/// build has neither — `TogglePlayerLock` and `ScrollBarsByViewPosition` are
/// both `{ return false; }` there (`C4Viewport.cpp:634-635`). A wheel notch is
/// therefore mapped onto the one step size C++ does define, `ViewportScrollSpeed`,
/// so the feel matches the Win32 and GTK bars' line buttons.
///
/// Lines scroll vertically and a horizontal wheel scrolls horizontally, both
/// away from the reader: a notch "down" raises `ViewY`, matching `SB_LINEDOWN`
/// (`:141`). Pixel deltas — trackpads — arrive pre-scaled and pass through.
pub fn wheel_scroll_step(delta: WheelDelta) -> (i32, i32) {
    match delta {
        WheelDelta::Lines { x, y } => (
            (-x * VIEWPORT_SCROLL_SPEED as f32) as i32,
            (-y * VIEWPORT_SCROLL_SPEED as f32) as i32,
        ),
        WheelDelta::Pixels { x, y } => (-x as i32, -y as i32),
    }
}

/// One wheel message, in whichever unit the platform reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelDelta {
    Lines { x: f32, y: f32 },
    Pixels { x: f32, y: f32 },
}

/// Where a viewport window's input goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewportEventRoute {
    /// Ordinary player mouse control.
    MouseControl,
    /// The editor sink — the edit cursor and drawing tools.
    EditorSink,
}

/// Play routes to mouse control; Edit and Draw route to the editor
/// (`C4Viewport.cpp:150-193` dispatches on `Console.EditCursor.GetMode()`).
pub fn route_viewport_event(mode: CursorMode) -> ViewportEventRoute {
    match mode {
        CursorMode::Play => ViewportEventRoute::MouseControl,
        CursorMode::Edit | CursorMode::Draw => ViewportEventRoute::EditorSink,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bars are drawn from `ScrollBarsByViewPosition`'s own numbers, so
    /// the thumb has to carry `nPage` as its extent and `nPos` as its offset
    /// against `nMax`. The reference macOS build compiles the bars away
    /// (`C4Viewport.cpp:634-635`), so only these proportions are portable —
    /// the thickness and the colours are this port's own.
    #[test]
    fn scroll_bars_track_the_page_and_position_they_are_given() {
        const THICKNESS: i32 = 5;
        let surface = (200, 100);
        let landscape = (1000, 500);
        let view = (200, 100);

        let ranges =
            |x: i32, y: i32| scroll_ranges(false, x, y, view.0, view.1, landscape.0, landscape.1);

        // The horizontal track gives up its last `THICKNESS` pixels to the
        // vertical bar, so the two never overlap in the corner.
        let layout = scroll_bar_layout(ranges(0, 0), surface.0, surface.1, THICKNESS)
            .expect("an unlocked viewport has bars");
        assert_eq!(layout.horizontal.track.width, surface.0 - THICKNESS);
        assert_eq!(layout.vertical.track.height, surface.1 - THICKNESS);

        // `nPage / nMax` of the track: 200/1000 of 195.
        assert_eq!(layout.horizontal.thumb.width, 39);
        assert_eq!(layout.horizontal.thumb.x, layout.horizontal.track.x);
        // 100/500 of 95.
        assert_eq!(layout.vertical.thumb.height, 19);
        assert_eq!(layout.vertical.thumb.y, layout.vertical.track.y);

        // Scrolled fully right, the thumb ends flush with the track — that is
        // what makes the bar read as "this is the end of the landscape".
        let layout = scroll_bar_layout(
            ranges(landscape.0 - view.0, landscape.1 - view.1),
            surface.0,
            surface.1,
            THICKNESS,
        )
        .expect("an unlocked viewport has bars");
        assert_eq!(
            layout.horizontal.thumb.x + layout.horizontal.thumb.width,
            layout.horizontal.track.x + layout.horizontal.track.width
        );
        assert_eq!(
            layout.vertical.thumb.y + layout.vertical.thumb.height,
            layout.vertical.track.y + layout.vertical.track.height
        );

        // Halfway along, the thumb is halfway along.
        let layout = scroll_bar_layout(ranges(400, 0), surface.0, surface.1, THICKNESS)
            .expect("an unlocked viewport has bars");
        assert_eq!(layout.horizontal.thumb.x, layout.horizontal.track.x + 78);
    }

    /// The issue's own criterion: the bars disappear exactly when
    /// `scroll_ranges` refuses, which is the locked case
    /// (`C4Viewport.cpp:272`).
    #[test]
    fn scroll_bars_are_absent_exactly_when_the_ranges_are() {
        let locked = scroll_ranges(true, 0, 0, 200, 100, 1000, 500);
        assert!(locked.is_none(), "the lock is what refuses");
        assert!(
            scroll_bar_layout(locked, 200, 100, 5).is_none(),
            "a locked viewport draws no bars"
        );

        let unlocked = scroll_ranges(false, 0, 0, 200, 100, 1000, 500);
        assert!(unlocked.is_some());
        assert!(
            scroll_bar_layout(unlocked, 200, 100, 5).is_some(),
            "an unlocked one does"
        );
    }

    /// A window too small to hold a track draws nothing rather than a
    /// negative-width rectangle.
    #[test]
    fn a_viewport_smaller_than_its_bars_draws_none() {
        let ranges = scroll_ranges(false, 0, 0, 200, 100, 1000, 500);
        assert!(scroll_bar_layout(ranges, 4, 100, 5).is_none());
        assert!(scroll_bar_layout(ranges, 200, 4, 5).is_none());
    }

    // C4Viewport.cpp:250-272 — the lock's asymmetry, the scrollbar gate, and
    // the per-mode input route.
    #[test]
    fn console_viewport_windows_route_redraw_resize_close_and_input_by_window_id() {
        // Locking needs a valid player; unlocking never does.
        assert_eq!(
            toggle_player_lock(false, true),
            PlayerLockState {
                locked: true,
                scrollbars_visible: false
            }
        );
        assert_eq!(
            toggle_player_lock(true, true),
            PlayerLockState {
                locked: false,
                scrollbars_visible: true
            }
        );
        // A locked viewport still unlocks even if its player went away.
        assert_eq!(
            toggle_player_lock(true, false),
            PlayerLockState {
                locked: false,
                scrollbars_visible: true
            }
        );
        // An ownerless viewport cannot lock at all — the `else if` fails and
        // nothing changes (:261).
        assert_eq!(
            toggle_player_lock(false, false),
            PlayerLockState {
                locked: false,
                scrollbars_visible: true
            },
            "NO_OWNER viewports keep free scroll"
        );

        // A locked viewport has no scroll bars at all (:272).
        assert_eq!(scroll_ranges(true, 10, 20, 400, 250, 4000, 1200), None);

        // Unlocked: each bar spans the landscape, pages by the view extent and
        // sits at the view origin.
        let (horizontal, vertical) =
            scroll_ranges(false, 10, 20, 400, 250, 4000, 1200).expect("free scroll");
        assert_eq!(
            horizontal,
            ScrollRange {
                min: 0,
                max: 4000,
                page: 400,
                position: 10
            }
        );
        assert_eq!(
            vertical,
            ScrollRange {
                min: 0,
                max: 1200,
                page: 250,
                position: 20
            }
        );

        // A wheel notch is worth one scroll-bar line step, away from the
        // reader: scrolling down raises ViewY like `SB_LINEDOWN` (:141).
        assert_eq!(
            wheel_scroll_step(WheelDelta::Lines { x: 0.0, y: -1.0 }),
            (0, VIEWPORT_SCROLL_SPEED)
        );
        assert_eq!(
            wheel_scroll_step(WheelDelta::Lines { x: 1.0, y: 0.0 }),
            (-VIEWPORT_SCROLL_SPEED, 0)
        );
        // Trackpad deltas already carry their own distance.
        assert_eq!(
            wheel_scroll_step(WheelDelta::Pixels { x: 0.0, y: -7.0 }),
            (0, 7)
        );

        // C4Viewport.cpp:1350-1351 — the window a viewport materialises with.
        let ownerless = viewport_window_spec(crate::OWNER_NONE, None, 1.0);
        assert_eq!((ownerless.width, ownerless.height), (400, 250));
        assert_eq!(ownerless.title, ViewportWindowTitle::Resource);
        assert_eq!(
            ownerless.position_id, "Viewport0",
            "the id is Player + 1, so an ownerless viewport is Viewport0"
        );
        assert_eq!(ownerless.position_subkey, "Console");
        assert!(ownerless.store_size);

        // An owned viewport takes its player's name and the next id.
        let owned = viewport_window_spec(0, Some("Red"), 1.0);
        assert_eq!(
            owned.title,
            ViewportWindowTitle::PlayerName("Red".to_owned())
        );
        assert_eq!(owned.position_id, "Viewport1");
        // Duplicate owners still get distinct ids by player, not by list index.
        assert_eq!(
            viewport_window_spec(3, Some("Blue"), 1.0).position_id,
            "Viewport4"
        );
        // An owner with no player row falls back rather than titling it empty.
        assert_eq!(
            viewport_window_spec(2, None, 1.0).title,
            ViewportWindowTitle::Resource
        );

        // The extent is `ceilf`, so a fractional scale always rounds up.
        let scaled = viewport_window_spec(crate::OWNER_NONE, None, 1.5);
        assert_eq!((scaled.width, scaled.height), (600, 375));
        let awkward = viewport_window_spec(crate::OWNER_NONE, None, 1.01);
        assert_eq!(
            (awkward.width, awkward.height),
            (404, 253),
            "ceil, not round: 400*1.01 = 404.0 and 250*1.01 = 252.5 -> 253"
        );

        // Input routes by cursor mode; the concrete edit behaviour belongs to
        // the edit cursor's selection/drag/context-menu work.
        assert_eq!(
            route_viewport_event(CursorMode::Play),
            ViewportEventRoute::MouseControl
        );
        assert_eq!(
            route_viewport_event(CursorMode::Edit),
            ViewportEventRoute::EditorSink
        );
        assert_eq!(
            route_viewport_event(CursorMode::Draw),
            ViewportEventRoute::EditorSink
        );
    }
}
