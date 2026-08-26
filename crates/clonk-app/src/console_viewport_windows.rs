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
/// What a detached viewport window does with one keyboard event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewportKeyRoute {
    /// `VK_SCROLL` is bound to this viewport's own lock and deliberately kept
    /// out of `Game.DoKeyboardInput` (`C4Viewport.cpp:82-86`).
    TogglePlayerLock,
    /// Escape closes the port's console popup without running anything, as it
    /// does for both native menus. The popup is port-only, so C++ has no arm
    /// here — but it owns the key only while it is open.
    DismissPopup,
    /// Everything else reaches the process-global scope/priority dispatcher,
    /// which is what `WM_KEYDOWN`/`WM_KEYUP` do for every other key
    /// (`C4Viewport.cpp:88-96`).
    Dispatch,
    /// Nothing to do: a key this process has no legacy code for, or the
    /// autorepeat of the viewport-local lock.
    Ignore,
}

/// `C4ViewportWindow::WndProc`'s keyboard switch (`C4Viewport.cpp:79-100`).
///
/// `WM_KEYUP` has no `VK_SCROLL` case, so only the *press* is viewport-local;
/// the release goes to the dispatcher with every other key.
pub(crate) fn viewport_key_route(
    key: Option<crate::VirtualKeyCode>,
    state: winit::event::ElementState,
    repeat: bool,
    popup_open: bool,
) -> ViewportKeyRoute {
    let pressed = state == winit::event::ElementState::Pressed;
    match key {
        Some(crate::VirtualKeyCode::ScrollLock) if pressed => {
            if repeat {
                // Pre-existing choice: an autorepeated lock toggle would
                // flicker the lock rather than express an intent.
                ViewportKeyRoute::Ignore
            } else {
                ViewportKeyRoute::TogglePlayerLock
            }
        }
        Some(crate::VirtualKeyCode::Escape) if pressed && popup_open => {
            ViewportKeyRoute::DismissPopup
        }
        Some(_) => ViewportKeyRoute::Dispatch,
        None => ViewportKeyRoute::Ignore,
    }
}

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
                // The popup is painted on this window's frame, so it dies with
                // it rather than waiting for a click that can no longer arrive.
                app.dismiss_console_viewport_context_menu_for(identity);
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
        // `WM_KEYDOWN`/`WM_KEYUP` reach `Game.DoKeyboardInput` for every key
        // but the viewport-local lock (`C4Viewport.cpp:79-96`).
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput { event: input, .. },
            ..
        } => {
            let legacy = crate::legacy_virtual_key_from_event(input, app.keyboard_modifiers);
            match viewport_key_route(
                legacy,
                input.state,
                input.repeat,
                app.console_viewport_context_menu_open(),
            ) {
                ViewportKeyRoute::TogglePlayerLock => {
                    if let Some(identity) =
                        windows.host(key).and_then(DeveloperHost::viewport_identity)
                    {
                        app.toggle_console_viewport_player_lock(identity);
                        windows.request_redraw(key);
                    }
                }
                ViewportKeyRoute::DismissPopup => {
                    if app.dismiss_console_viewport_context_menu() {
                        windows.request_redraw(key);
                    }
                }
                ViewportKeyRoute::Dispatch => {
                    if let Some(legacy) = legacy {
                        if let Err(error) = app.handle_key(legacy, input.state) {
                            tracing::error!(%error, "detached viewport key dispatch failed");
                        }
                    }
                }
                ViewportKeyRoute::Ignore => {}
            }
        }
        // Scrolling stands in for the native scroll bars this window does not
        // have — see `developer_viewport::wheel_scroll_step`.
        Event::WindowEvent {
            event: WindowEvent::MouseWheel { delta, .. },
            ..
        } => {
            use clonk_engine::developer_viewport::{wheel_scroll_step, WheelDelta};

            let delta = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    WheelDelta::Lines { x: *x, y: *y }
                }
                winit::event::MouseScrollDelta::PixelDelta(position) => WheelDelta::Pixels {
                    x: position.x as f32,
                    y: position.y as f32,
                },
            };
            let (dx, dy) = wheel_scroll_step(delta);
            if let Some(identity) = windows.host(key).and_then(DeveloperHost::viewport_identity) {
                if app.scroll_console_viewport(identity, dx, dy) {
                    windows.request_redraw(key);
                }
            }
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
            // `Application.IsControlDown()`/`IsShiftDown()` are a
            // process-global state that `DoKeyboardInput` reads for whichever
            // window delivered the key (`C4Viewport.cpp:89`), so a detached
            // window's modifiers have to reach the dispatcher's too.
            app.keyboard_modifiers = modifiers.state();
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
            let surface_local = viewport.surface_pointer();
            // The popup owns the pointer while it is up — the row under the
            // cursor highlights and the edit cursor beneath sees nothing.
            if app.console_viewport_context_menu_motion(identity, surface_local) {
                windows.request_redraw(key);
                return;
            }
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
            let Some(DeveloperHost::Viewport(viewport)) = windows.host_mut(key) else {
                return;
            };
            // The popup owns the whole click, press and release: C++'s menu is
            // modal, so `LeftButtonUp` never runs while it is up. Letting the
            // release through would clear the `Hold` that Grab contents sets
            // one line before its control goes out — and the press that chose
            // the item may already have closed the menu, so the grab is what
            // is asked, not whether a popup is still open.
            let identity = viewport.identity;
            if app.take_console_viewport_pointer_grab(identity)
                || app.console_viewport_context_menu_owns_pointer(identity)
            {
                return;
            }
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
            let (surface_local, extent) = (viewport.surface_pointer(), viewport.surface_extent());
            // An open popup takes the click: `TrackPopupMenu` is modal and the
            // GTK menu holds a pointer grab, so neither ever lets one through
            // to the viewport underneath.
            if app.console_viewport_context_menu_click(identity, surface_local, extent) {
                windows.request_redraw(key);
                return;
            }
            // A press the popup did not take starts an ordinary gesture, so
            // any grab left behind by a click whose release never arrived —
            // the window lost focus between the two — ends here rather than
            // swallowing this gesture's release instead.
            app.console_viewport_context_menu_grab = None;
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
        // `C4EditCursor::RightButtonDown` settles the selection and
        // `RightButtonUp` opens the menu over it (`C4EditCursor.cpp:244-274,
        // 332-340`) — two messages, so a drag between them cannot change what
        // the menu was built for.
        Event::WindowEvent {
            event:
                WindowEvent::MouseInput {
                    state: winit::event::ElementState::Pressed,
                    button: winit::event::MouseButton::Right,
                    ..
                },
            ..
        } => {
            let Some(DeveloperHost::Viewport(viewport)) = windows.host_mut(key) else {
                return;
            };
            let (identity, local) = (viewport.identity, viewport.last_pointer);
            // A second right-click while the popup is up cancels it and
            // touches nothing underneath — the modal menu would have eaten
            // the message. Without this the selection is re-picked *behind*
            // the menu the user is still reading.
            if app.console_viewport_context_menu_owns_pointer(identity) {
                app.dismiss_console_viewport_context_menu();
                app.console_viewport_context_menu_grab = Some(identity);
                windows.request_redraw(key);
                return;
            }
            let modifiers = app.keyboard_modifiers;
            app.console_viewport_right_press(identity, local, 1.0, modifiers.control_key());
        }
        Event::WindowEvent {
            event:
                WindowEvent::MouseInput {
                    state: winit::event::ElementState::Released,
                    button: winit::event::MouseButton::Right,
                    ..
                },
            ..
        } => {
            let Some(DeveloperHost::Viewport(viewport)) = windows.host_mut(key) else {
                return;
            };
            let (identity, local) = (viewport.identity, viewport.surface_pointer());
            // The release that completes a cancelling right-click opens
            // nothing: `RightButtonUp` runs `DoContextMenu` once per press.
            if app.take_console_viewport_pointer_grab(identity) {
                return;
            }
            app.open_console_viewport_context_menu(identity, local);
            windows.request_redraw(key);
        }
        // `WM_DROPFILES` — the editor's only way to create an object without
        // typing script (`C4Viewport.cpp:106-109,225-240`). winit delivers one
        // path per event where Win32 hands over a whole `HDROP`, so each is
        // its own `DropFiles` call.
        Event::WindowEvent {
            event: WindowEvent::DroppedFile(path),
            ..
        } => {
            let Some(DeveloperHost::Viewport(viewport)) = windows.host_mut(key) else {
                return;
            };
            // `DragQueryPoint` gives the drop point; winit's `DroppedFile`
            // carries none, so the position the pointer was last seen at is
            // the only one there is.
            let (identity, local) = (viewport.identity, viewport.surface_pointer());
            app.drop_file_on_console_viewport(identity, path, local);
            windows.request_redraw(key);
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

    // `C4ViewportWindow::WndProc` sends every key but the viewport-local lock
    // to `Game.DoKeyboardInput` (`C4Viewport.cpp:79-100`), and its `WM_KEYUP`
    // arm has no `VK_SCROLL` case at all.
    #[test]
    fn detached_viewport_keys_reach_the_dispatcher_except_the_viewport_lock() {
        use winit::event::ElementState::{Pressed, Released};

        // The lock is the one key the dialog keeps, and only on the press.
        assert_eq!(
            viewport_key_route(
                Some(crate::VirtualKeyCode::ScrollLock),
                Pressed,
                false,
                false
            ),
            ViewportKeyRoute::TogglePlayerLock
        );
        assert_eq!(
            viewport_key_route(
                Some(crate::VirtualKeyCode::ScrollLock),
                Released,
                false,
                false
            ),
            ViewportKeyRoute::Dispatch,
            "WM_KEYUP has no VK_SCROLL case"
        );
        assert_eq!(
            viewport_key_route(
                Some(crate::VirtualKeyCode::ScrollLock),
                Pressed,
                true,
                false
            ),
            ViewportKeyRoute::Ignore,
            "an autorepeated lock toggle expresses no intent"
        );

        // Everything else goes to the dispatcher, down and up alike, repeated
        // or not — the repeat flag is an argument `DoKeyboardInput` carries,
        // not a reason to drop the event.
        for key in [
            crate::VirtualKeyCode::KeyA,
            crate::VirtualKeyCode::F5,
            crate::VirtualKeyCode::AltLeft,
            crate::VirtualKeyCode::ShiftLeft,
            crate::VirtualKeyCode::Space,
        ] {
            for state in [Pressed, Released] {
                for repeat in [false, true] {
                    assert_eq!(
                        viewport_key_route(Some(key), state, repeat, false),
                        ViewportKeyRoute::Dispatch,
                        "{key:?} {state:?} repeat={repeat}"
                    );
                }
            }
        }

        // A key this process has no legacy code for is dropped rather than
        // dispatched as something else.
        assert_eq!(
            viewport_key_route(None, Pressed, false, false),
            ViewportKeyRoute::Ignore
        );
    }

    // The port-only console popup owns Escape while it is open, and only
    // then; C++ has no popup here, so an unowned Escape is an ordinary key.
    #[test]
    fn detached_viewport_escape_belongs_to_an_open_popup_and_otherwise_dispatches() {
        use winit::event::ElementState::{Pressed, Released};

        assert_eq!(
            viewport_key_route(Some(crate::VirtualKeyCode::Escape), Pressed, false, true),
            ViewportKeyRoute::DismissPopup
        );
        assert_eq!(
            viewport_key_route(Some(crate::VirtualKeyCode::Escape), Pressed, false, false),
            ViewportKeyRoute::Dispatch,
            "with no popup open Escape is the dispatcher's"
        );
        assert_eq!(
            viewport_key_route(Some(crate::VirtualKeyCode::Escape), Released, false, true),
            ViewportKeyRoute::Dispatch,
            "the popup consumes the press, not the release"
        );
        // The popup never claims another key.
        assert_eq!(
            viewport_key_route(Some(crate::VirtualKeyCode::KeyA), Pressed, false, true),
            ViewportKeyRoute::Dispatch
        );
        assert_eq!(
            viewport_key_route(
                Some(crate::VirtualKeyCode::ScrollLock),
                Pressed,
                false,
                true
            ),
            ViewportKeyRoute::TogglePlayerLock,
            "the viewport lock outranks the popup, as it outranks the dispatcher"
        );
    }

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
