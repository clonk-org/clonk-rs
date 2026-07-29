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

        // Input routes by cursor mode; concrete edit behaviour is L043's.
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
