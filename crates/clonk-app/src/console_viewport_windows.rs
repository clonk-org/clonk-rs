//! Which console viewport windows must open and close this pass.
//!
//! C++ has no reconciliation step: `C4GraphicsSystem::CreateViewport` creates
//! the `C4ViewportWindow` inside the same call that appends the viewport
//! (`C4GraphicsSystem.cpp:229-240`), and `CloseViewport(C4Viewport *)` destroys
//! exactly the one whose pointer it was handed (`:205-224`). The port cannot
//! open an OS window from those call sites — winit needs the event loop's
//! window target — so the same decisions are taken once per pass instead, from
//! the physical viewport list that `create_physical_viewport` and
//! `close_physical_viewports` already maintain.
//!
//! The identity *is* the C++ pointer. Addressing by owner would conflate two
//! windows showing the same player, which is exactly what
//! `CloseViewport(C4Viewport *)` refuses to do — its player-keyed sibling
//! (`:314-331`) is the one that erases every match, and it is not the path a
//! window's close button takes.

/// One window to open or close so the open set matches the physical list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConsoleViewportWindowChange {
    /// A physical viewport with no window yet. `player` selects the title:
    /// `IDS_CNS_VIEWPORT` for `NO_OWNER`, the player's name otherwise
    /// (`C4Viewport.cpp:1351`).
    Open { identity: u64, player: i32 },
    /// A window whose viewport is gone. Only this one closes.
    Close { identity: u64 },
}

/// Reconcile the open viewport windows against the live physical list.
///
/// `physical` is in `Game.GraphicsSystem::Viewports` order and `open` is the
/// set of identities that already own a window. Opens are emitted in physical
/// order so a batch of new viewports materialises in the order C++ appended
/// them; closes follow, so a pass that replaces one viewport with another
/// never leaves the new window waiting behind the old one's teardown.
pub(crate) fn console_viewport_window_changes(
    physical: impl IntoIterator<Item = (u64, i32)>,
    open: &[u64],
) -> Vec<ConsoleViewportWindowChange> {
    let mut live = Vec::new();
    let mut changes = Vec::new();
    for (identity, player) in physical {
        live.push(identity);
        if !open.contains(&identity) {
            changes.push(ConsoleViewportWindowChange::Open { identity, player });
        }
    }
    changes.extend(
        open.iter()
            .filter(|identity| !live.contains(identity))
            .map(|&identity| ConsoleViewportWindowChange::Close { identity }),
    );
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4GraphicsSystem.cpp:205-224,226-251 — one window per viewport, and a
    // close that erases exactly the viewport it names.
    #[test]
    fn console_viewport_windows_open_per_identity_and_close_only_their_own() {
        // Nothing open yet: every physical viewport materialises, in list
        // order, carrying the player its title comes from.
        assert_eq!(
            console_viewport_window_changes([(41, -1), (42, 3)], &[]),
            vec![
                ConsoleViewportWindowChange::Open {
                    identity: 41,
                    player: -1,
                },
                ConsoleViewportWindowChange::Open {
                    identity: 42,
                    player: 3,
                },
            ]
        );

        // Steady state costs nothing.
        assert_eq!(
            console_viewport_window_changes([(41, -1), (42, 3)], &[41, 42]),
            vec![]
        );

        // Closing one viewport closes exactly its window. This is the
        // pointer-keyed `CloseViewport`, not the player-keyed one.
        assert_eq!(
            console_viewport_window_changes([(42, 3)], &[41, 42]),
            vec![ConsoleViewportWindowChange::Close { identity: 41 }]
        );

        // Two windows on the *same* player are distinct windows: the second
        // opens and the first is untouched. Keying on the owner would either
        // skip the open or close the wrong window.
        assert_eq!(
            console_viewport_window_changes([(41, 3), (42, 3)], &[41]),
            vec![ConsoleViewportWindowChange::Open {
                identity: 42,
                player: 3,
            }]
        );

        // A closed game drops every window.
        assert_eq!(
            console_viewport_window_changes([], &[41, 42]),
            vec![
                ConsoleViewportWindowChange::Close { identity: 41 },
                ConsoleViewportWindowChange::Close { identity: 42 },
            ]
        );

        // A replacement pass opens before it closes, so the surviving window
        // is never the last thing created.
        assert_eq!(
            console_viewport_window_changes([(43, 0)], &[41]),
            vec![
                ConsoleViewportWindowChange::Open {
                    identity: 43,
                    player: 0,
                },
                ConsoleViewportWindowChange::Close { identity: 41 },
            ]
        );
    }
}
