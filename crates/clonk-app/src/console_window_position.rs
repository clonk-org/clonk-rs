//! The developer console's own remembered window position.
//!
//! `C4Console::GetPositionData` (`C4Console.cpp:1278-1284`) selects the `Main`
//! entry under the `Console` subkey and sets `storeSize = false`, so the console
//! persists a position and nothing else — separate from the game window's
//! geometry. `RestorePosition` runs right after the window is created
//! (`C4Console.cpp:296-305`) and `StorePosition` on destruction (:154-159).
//!
//! `StoreWindowPosition`/`RestoreWindowPosition` (`StdRegistry.cpp:283-327`)
//! define the stored grammar: the literal `Maximized` or `Minimized`, or a
//! comma-separated `x,y` — `x,y,w,h` only when `storeSize` is set, which the
//! console never does. A restore that finds fewer than four fields keeps the
//! window's current size.

/// The `[Console]` section and `Main` key `GetPositionData` names. The port
/// stores its configuration in an INI rather than the registry, but keeps the
/// same names so the entry is recognisable.
pub(crate) const CONSOLE_POSITION_SECTION: &str = "Console";
pub(crate) const CONSOLE_POSITION_KEY: &str = "Main";

/// What `RestoreWindowPosition` found stored (`StdRegistry.cpp:300-327`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConsoleWindowPlacement {
    /// The literal `"Maximized"` (:310-311).
    Maximized,
    /// The literal `"Minimized"` (:312-313).
    Minimized,
    /// `x,y` — the console's own form, which leaves the size alone (:317-322).
    Position { x: i32, y: i32 },
    /// `x,y,w,h`. The console never writes this, but a hand-edited or
    /// inherited entry can carry it, and C++ would honour it.
    PositionAndSize {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
}

impl ConsoleWindowPlacement {
    /// The position to move the window to, if the entry carries one.
    pub(crate) fn position(self) -> Option<(i32, i32)> {
        match self {
            Self::Position { x, y } | Self::PositionAndSize { x, y, .. } => Some((x, y)),
            Self::Maximized | Self::Minimized => None,
        }
    }
}

/// Parses a stored entry. Returns `None` for anything `sscanf` would not read
/// as a coordinate pair, which C++ treats as "cannot restore" (:307-308).
pub(crate) fn parse_console_position(stored: &str) -> Option<ConsoleWindowPlacement> {
    let stored = stored.trim();
    match stored {
        "Maximized" => return Some(ConsoleWindowPlacement::Maximized),
        "Minimized" => return Some(ConsoleWindowPlacement::Minimized),
        _ => {}
    }
    let fields: Vec<i32> = stored
        .split(',')
        .map(|field| field.trim().parse::<i32>())
        .collect::<Result<_, _>>()
        .ok()?;
    match fields[..] {
        [x, y] => Some(ConsoleWindowPlacement::Position { x, y }),
        [x, y, width, height] => Some(ConsoleWindowPlacement::PositionAndSize {
            x,
            y,
            width,
            height,
        }),
        // Three fields leave `fSetSize` true with an uninitialised height in
        // C++; the port declines rather than reproducing that read.
        _ => None,
    }
}

/// Formats the console's entry. `storeSize` is false for the console, so this
/// is always the two-field form (`StdRegistry.cpp:296-297`).
pub(crate) fn format_console_position(x: i32, y: i32) -> String {
    format!("{x},{y}")
}

/// The `Viewport{Player + 1}` key a detached viewport remembers its geometry
/// under (`C4ViewportWindow::GetPositionData`, `C4Viewport.cpp:217-222`).
///
/// The same `Console` subkey as the console's own `Main` entry, but keyed on
/// the *player*, not a list index: two viewports following the same player
/// then share one slot instead of colliding by position, and the ownerless
/// viewport (`OWNER_NONE`, which is -1) lands on `Viewport0`.
pub(crate) fn viewport_position_key(player: i32) -> String {
    format!("Viewport{}", player.saturating_add(1))
}

/// The `ConsoleGUI_{GetID()}` key a console **dialog** remembers its geometry
/// under (`DialogWindow::GetPositionData`, `C4GuiDialogs.cpp:285-297`).
///
/// The same `Console` subkey as the console's own `Main` entry and a
/// viewport's, keyed on the dialog's own `GetID()` — `"Scoreboard"` for
/// `C4ScoreboardDlg` (`C4Scoreboard.h:107`). A dialog whose `GetID()` is empty
/// gets no entry at all (`:287`), which is what `None` means here.
pub(crate) fn console_dialog_position_key(dialog_id: &str) -> Option<String> {
    (!dialog_id.is_empty()).then(|| format!("ConsoleGUI_{dialog_id}"))
}

/// A console dialog's entry. `GetPositionData` sets `storeSize = true`
/// (`C4GuiDialogs.cpp:291`), so it is the same four-field form a viewport
/// writes rather than the console's own two-field one.
///
/// The stored size is written but is not what the dialog ends up at: every
/// `C4ScoreboardDlg::Update` runs `Dialog::UpdateSize`, which resizes the
/// window from the dialog's recomputed bounds (`C4GuiDialogs.cpp:445-473`).
/// Restoring it still matters for the frame between window creation and the
/// first update, and for byte-parity with the config C++ writes.
pub(crate) fn format_console_dialog_position(x: i32, y: i32, width: i32, height: i32) -> String {
    format_viewport_position(x, y, width, height)
}

/// A viewport's entry. `GetPositionData` sets `storeSize = true`
/// (`C4Viewport.cpp:221`), so unlike the console this is the four-field form
/// (`StdRegistry.cpp:296-297`) and the remembered size comes back with the
/// position.
pub(crate) fn format_viewport_position(x: i32, y: i32, width: i32, height: i32) -> String {
    format!("{x},{y},{width},{height}")
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    // C4Viewport.cpp:220-222 — a detached viewport remembers its geometry
    // under the same `Console` subkey, keyed by player, and unlike the console
    // it sets `storeSize`, so the entry carries the size too.
    #[test]
    fn a_viewport_remembers_its_geometry_keyed_by_player() {
        // `Viewport{Player + 1}`: the ownerless viewport is `Viewport0` and
        // player 0 is `Viewport1`. Keying on the player rather than a list
        // index is what stops two viewports following the same player from
        // colliding — and what makes an ownerless one land in its own slot.
        assert_eq!(viewport_position_key(clonk_engine::OWNER_NONE), "Viewport0");
        assert_eq!(viewport_position_key(0), "Viewport1");
        assert_eq!(viewport_position_key(3), "Viewport4");

        // `storeSize` is set, so this is the four-field form — the one the
        // console never writes.
        assert_eq!(format_viewport_position(10, 20, 400, 250), "10,20,400,250");
        assert_eq!(
            parse_console_position(&format_viewport_position(10, 20, 400, 250)),
            Some(ConsoleWindowPlacement::PositionAndSize {
                x: 10,
                y: 20,
                width: 400,
                height: 250,
            }),
            "the size survives the round trip, which is the whole point of storeSize"
        );

        // The section is shared with the console, so a viewport entry must not
        // be able to land on the console's own key.
        assert_ne!(
            viewport_position_key(clonk_engine::OWNER_NONE),
            CONSOLE_POSITION_KEY
        );
    }

    // C4Console.cpp:1278-1284; StdRegistry.cpp:283-327 — the console round-trips
    // a position through its own `Console/Main` slot and never touches the game
    // window's geometry keys.
    #[test]
    fn console_window_position_round_trips_without_overwriting_game_display() {
        // Stored as two fields because the console sets storeSize = false.
        assert_eq!(format_console_position(120, 48), "120,48");
        assert_eq!(
            parse_console_position("120,48"),
            Some(ConsoleWindowPlacement::Position { x: 120, y: 48 })
        );
        assert_eq!(
            parse_console_position(&format_console_position(-40, -1024))
                .and_then(ConsoleWindowPlacement::position),
            Some((-40, -1024)),
            "a negative position is valid on a multi-monitor desktop"
        );

        // The two literals C++ writes for a zoomed or iconic window carry no
        // coordinates, so nothing is moved (:290-292,:310-313).
        assert_eq!(
            parse_console_position("Maximized"),
            Some(ConsoleWindowPlacement::Maximized)
        );
        assert_eq!(
            parse_console_position("Minimized"),
            Some(ConsoleWindowPlacement::Minimized)
        );
        assert_eq!(
            ConsoleWindowPlacement::Maximized.position(),
            None,
            "a maximized entry must not be applied as a position"
        );

        // A four-field entry is honoured for its position; the console never
        // writes one, and the extra fields never become game-window size.
        assert_eq!(
            parse_console_position("10,20,640,480"),
            Some(ConsoleWindowPlacement::PositionAndSize {
                x: 10,
                y: 20,
                width: 640,
                height: 480,
            })
        );
        assert_eq!(
            parse_console_position("10,20,640,480").and_then(ConsoleWindowPlacement::position),
            Some((10, 20))
        );

        // Unusable entries restore nothing rather than moving to a garbage
        // coordinate (:307-308).
        assert_eq!(parse_console_position(""), None);
        assert_eq!(parse_console_position("120"), None);
        assert_eq!(parse_console_position("120,"), None);
        assert_eq!(parse_console_position("120,48,640"), None);
        assert_eq!(parse_console_position("left,top"), None);
        assert_eq!(parse_console_position("Restored"), None);

        // The slot is the console's own, not the game window's.
        assert_eq!(CONSOLE_POSITION_SECTION, "Console");
        assert_eq!(CONSOLE_POSITION_KEY, "Main");
    }
}
