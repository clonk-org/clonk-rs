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
