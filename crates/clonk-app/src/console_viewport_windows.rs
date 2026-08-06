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

/// Open and close console viewport windows so the registry matches the live
/// physical list.
///
/// `scale` is `Application.GetScale()` (`C4Application.h:119`), which sizes the
/// window as `ceilf(400 * scale)` by `ceilf(250 * scale)`
/// (`C4Viewport.cpp:1350`).
///
/// A window that fails to build is logged and skipped, not fatal:
/// `C4GraphicsSystem::CreateViewport` deletes the viewport and returns false
/// when `Init` fails (`:235-239`), leaving the console running.
pub(crate) fn reconcile_console_viewport_windows(
    app: &mut crate::GameApp,
    windows: &mut crate::developer_windows::DeveloperWindows<crate::developer_host::DeveloperHost>,
    next_key: &mut u64,
    scale: f32,
    target: &winit::event_loop::ActiveEventLoop,
) {
    use crate::developer_host::DeveloperHost;
    use crate::developer_windows::{HostPurpose, WindowId};
    use clonk_engine::developer_viewport::{viewport_window_spec, ViewportWindowTitle};

    let open = windows
        .keys()
        .filter_map(|key| {
            windows
                .host(key)
                .and_then(DeveloperHost::viewport_identity)
                .map(|identity| (key, identity))
        })
        .collect::<Vec<_>>();
    let physical = app
        .physical_viewports
        .iter()
        .map(|viewport| (viewport.physical_identity, viewport.displayed_player))
        .collect::<Vec<_>>();
    let open_identities = open
        .iter()
        .map(|(_, identity)| *identity)
        .collect::<Vec<_>>();
    let changes = console_viewport_window_changes(physical, &open_identities);
    if changes.is_empty() {
        return;
    }

    for change in changes {
        match change {
            ConsoleViewportWindowChange::Open { identity, player } => {
                let name = app
                    .engine
                    .player(player)
                    .map(|player| player.name().to_owned());
                let spec = viewport_window_spec(player, name.as_deref(), scale);
                let title = match spec.title {
                    // The shipped resource table has no IDS_CNS_VIEWPORT, so
                    // the English fallback carries it, as it does for every
                    // other console string.
                    ViewportWindowTitle::Resource => {
                        app.runtime_resource_text("IDS_CNS_VIEWPORT", "Viewport")
                    }
                    ViewportWindowTitle::PlayerName(name) => name,
                };
                match crate::viewport_window_host::build_viewport_window(
                    target,
                    &title,
                    spec.width.max(1) as u32,
                    spec.height.max(1) as u32,
                    identity,
                    scale,
                ) {
                    Ok(host) => {
                        let key = WindowId(*next_key);
                        *next_key += 1;
                        tracing::debug!(
                            identity,
                            player,
                            width = spec.width,
                            height = spec.height,
                            "opened a console viewport window"
                        );
                        windows.insert(
                            key,
                            HostPurpose::Viewport {
                                viewport: identity as u32,
                            },
                            DeveloperHost::Viewport(host),
                        );
                    }
                    Err(error) => {
                        tracing::error!(%error, identity, "failed to open a console viewport window");
                    }
                }
            }
            ConsoleViewportWindowChange::Close { identity } => {
                if let Some((key, _)) = open.iter().find(|(_, open)| *open == identity) {
                    windows.close(*key);
                }
            }
        }
    }
}

/// Which OS window an event names, if any.
pub(crate) fn event_window_id(
    event: &winit::event::Event<crate::NetworkEventWake>,
) -> Option<winit::window::WindowId> {
    match event {
        winit::event::Event::WindowEvent { window_id, .. } => Some(*window_id),
        _ => None,
    }
}

/// One console viewport window's own events.
///
/// Close routes through the pointer-keyed `CloseViewport(C4Viewport *)`
/// (`C4GraphicsSystem.cpp:205-224`) by way of this window's identity, so
/// closing one window never takes a sibling viewport of the same player with
/// it — that is what the player-keyed overload (`:314-331`) would do.
pub(crate) fn handle_console_viewport_event(
    key: crate::developer_windows::WindowId,
    event: &winit::event::Event<crate::NetworkEventWake>,
    app: &mut crate::GameApp,
    windows: &mut crate::developer_windows::DeveloperWindows<crate::developer_host::DeveloperHost>,
) {
    use crate::developer_host::DeveloperHost;
    use crate::developer_windows::DeveloperWindowPresenter;
    use winit::event::{Event, WindowEvent};

    match event {
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            if let Some(identity) = windows.host(key).and_then(DeveloperHost::viewport_identity) {
                app.close_physical_viewport_identity(identity);
            }
            windows.close(key);
        }
        Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } => {
            windows.resize(key, size.width.max(1), size.height.max(1));
            windows.request_redraw(key);
        }
        // `ScaleFactorChanged` no longer carries the proposed size. Winit
        // follows it with `Resized`, which is the authoritative surface size.
        Event::WindowEvent {
            event: WindowEvent::ScaleFactorChanged { .. },
            ..
        } => {
            windows.request_redraw(key);
        }
        // A focused viewport window is where the console's modifier state
        // comes from: the shell never sees these messages, and the edit
        // cursor reads Ctrl/Shift live on every click (`C4EditCursor.cpp:143,
        // 206`) while Alt drives the Draw-mode picker (`:773-792`).
        Event::WindowEvent {
            event: WindowEvent::ModifiersChanged(modifiers),
            ..
        } => {
            app.update_console_editor_modifiers(modifiers.state());
        }
        // `C4Viewport`'s pointer handlers convert the coordinates carried by
        // each message through this viewport's own ViewX/ViewY and scale
        // (`C4Viewport.cpp:181`). winit splits motion from button state, so
        // the position is remembered between the two.
        Event::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. },
            ..
        } => {
            let Some(DeveloperHost::Viewport(viewport)) = windows.host_mut(key) else {
                return;
            };
            viewport.last_pointer = (position.x as i32, position.y as i32);
            let (identity, local) = (viewport.identity, viewport.last_pointer);
            let modifiers = app.keyboard_modifiers;
            app.console_viewport_motion(
                identity,
                local,
                1.0,
                modifiers.control_key(),
                modifiers.shift_key(),
            );
        }
        Event::WindowEvent {
            event:
                WindowEvent::MouseInput {
                    state: winit::event::ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                },
            ..
        } => {
            app.console_viewport_release();
        }
        Event::WindowEvent {
            event:
                WindowEvent::MouseInput {
                    state: winit::event::ElementState::Pressed,
                    button: winit::event::MouseButton::Left,
                    ..
                },
            ..
        } => {
            let Some(DeveloperHost::Viewport(viewport)) = windows.host_mut(key) else {
                return;
            };
            let (identity, local) = (viewport.identity, viewport.last_pointer);
            // `LeftButtonDown(fControl)` and `Move`'s Shift arm read the
            // live modifier state (`C4EditCursor.cpp:143,206`).
            let modifiers = app.keyboard_modifiers;
            app.console_viewport_press(
                identity,
                local,
                1.0,
                modifiers.control_key(),
                modifiers.shift_key(),
            );
        }
        Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } => {
            if let Some(host) = windows.host_mut(key) {
                if let Err(error) = host.present(app) {
                    tracing::error!(%error, "console viewport window present failed");
                }
            }
        }
        _ => {}
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
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
