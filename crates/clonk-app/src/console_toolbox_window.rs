//! Applying the developer toolbox's decisions to a real window.
//!
//! `C4DevmodeDlg` acts on its GTK window inside the call that decided to —
//! `AddPage` creates it, `SwitchPage` shows and retitles it, the
//! `delete-event` handler hides it (`C4DevmodeDlg.cpp:53-121`). winit cannot:
//! a window may only be built from the event loop's window target, and the
//! decisions are taken from console code that has none. So
//! [`crate::developer_toolbox::DeveloperToolbox`] records what it wants as
//! [`ToolboxEffect`]s and this module drains them once per pass, exactly as
//! [`crate::console_viewport_windows`] reconciles the viewport windows.

use crate::developer_host::DeveloperHost;
use crate::developer_toolbox::ToolboxEffect;
use crate::developer_windows::{DeveloperWindows, ToolboxPage, WindowId};
use crate::DeveloperPane;

/// The toolbox's registry key, if it has a window.
pub(crate) fn toolbox_window_key(windows: &DeveloperWindows<DeveloperHost>) -> Option<WindowId> {
    windows.find_key(|host| matches!(host, DeveloperHost::Toolbox(_)))
}

/// Apply every queued toolbox effect.
///
/// A window that fails to build is logged and the effect dropped, as a failed
/// viewport window is: the console keeps running without its toolbox rather
/// than taking the round down with it.
pub(crate) fn reconcile_developer_toolbox_window(
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
    next_key: &mut u64,
    target: &winit::event_loop::ActiveEventLoop,
) {
    use crate::developer_windows::HostPurpose;
    use crate::toolbox_window_host::build_toolbox_window;

    if app.developer_toolbox_effects.is_empty() {
        return;
    }
    for effect in std::mem::take(&mut app.developer_toolbox_effects) {
        let key = toolbox_window_key(windows);
        match effect {
            // Creation is deferred to the first `Show`: `AddPage` builds the
            // window before any page is switched to, but an empty window with
            // no current page has nothing to draw, and winit would flash it.
            ToolboxEffect::Create(_) => {}
            ToolboxEffect::Show {
                page,
                title,
                position,
            } => match key {
                Some(key) => {
                    windows.switch_page(key, page);
                    windows.show_and_focus(key);
                    if let Some(DeveloperHost::Toolbox(toolbox)) = windows.host_mut(key) {
                        toolbox.surface.window.set_title(&title);
                        // `SwitchPage` restores the remembered coordinates on
                        // the way back up (`C4DevmodeDlg.cpp:108-114`). This
                        // is the arm that has one: the create arm can only
                        // ever see `None`, because the position is recorded
                        // while the window is visible.
                        if let Some((x, y)) = position {
                            toolbox
                                .surface
                                .window
                                .set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                        }
                    }
                }
                None => {
                    let chrome = crate::developer_toolbox::ToolboxChrome::default();
                    match build_toolbox_window(
                        target,
                        &title,
                        chrome,
                        position,
                        crate::developer_toolbox_view::TOOLBOX_WIDTH,
                        crate::developer_toolbox_view::TOOLBOX_HEIGHT,
                    ) {
                        Ok(host) => {
                            let key = WindowId(*next_key);
                            *next_key += 1;
                            tracing::debug!(?page, "opened the developer toolbox window");
                            windows.insert(
                                key,
                                HostPurpose::Toolbox { page },
                                DeveloperHost::Toolbox(host),
                            );
                            windows.show_and_focus(key);
                        }
                        Err(error) => {
                            tracing::error!(%error, "failed to open the developer toolbox");
                            // The model believes it is up. Left that way it
                            // would answer every later `switch_page` with a
                            // bare retitle, and the toolbox could never be
                            // opened again in this session.
                            let effect = app.developer_toolbox.close(None);
                            app.developer_toolbox_effects.extend(effect);
                        }
                    }
                }
            },
            // The window hides and keeps every page — that is the whole point
            // of the `delete-event` handler returning TRUE.
            ToolboxEffect::Hide => {
                if let Some(key) = key {
                    windows.hide(key);
                }
            }
            ToolboxEffect::Title(title) => {
                if let Some(DeveloperHost::Toolbox(toolbox)) =
                    key.and_then(|key| windows.host_mut(key))
                {
                    toolbox.surface.window.set_title(&title);
                }
            }
            // Only the last page's removal destroys it, which nothing does
            // while a console round is up.
            ToolboxEffect::Destroy => {
                if let Some(key) = key {
                    windows.close(key);
                }
            }
        }
    }
}

/// The object list's registry key, if it has a window.
pub(crate) fn object_list_window_key(
    windows: &DeveloperWindows<DeveloperHost>,
) -> Option<WindowId> {
    windows.find_key(|host| matches!(host, DeveloperHost::ObjectList(_)))
}

/// Open or close the object list window to match the console's request.
///
/// `C4ObjectListDlg::Open` creates the window only when it has none
/// (`C4ObjectListDlg.cpp:728`), so a second Objects click on an open list does
/// nothing — which is what makes this a reconcile rather than a command.
pub(crate) fn reconcile_developer_object_list_window(
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
    next_key: &mut u64,
    target: &winit::event_loop::ActiveEventLoop,
) {
    use crate::developer_windows::HostPurpose;
    use crate::object_list_window_host::build_object_list_window;

    let key = object_list_window_key(windows);
    match (app.developer_object_list_open, key) {
        (true, None) => match build_object_list_window(target) {
            Ok(host) => {
                let key = WindowId(*next_key);
                *next_key += 1;
                tracing::debug!("opened the developer object list window");
                windows.insert(
                    key,
                    HostPurpose::ObjectList,
                    DeveloperHost::ObjectList(host),
                );
            }
            Err(error) => {
                tracing::error!(%error, "failed to open the developer object list");
                // Leaving the request set would retry the failed build on
                // every pass for the rest of the round.
                app.close_developer_object_list();
            }
        },
        (false, Some(key)) => {
            windows.close(key);
        }
        _ => {}
    }
}

/// The object list window's own events.
pub(crate) fn handle_developer_object_list_event(
    key: WindowId,
    event: &winit::event::Event<crate::NetworkEventWake>,
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
) {
    use crate::developer_windows::DeveloperWindowPresenter;
    use winit::event::{Event, WindowEvent};

    match event {
        // The `"destroy"` handler nulls the window and the model; this list is
        // destroyed rather than hidden (`C4ObjectListDlg.cpp:592-597`).
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            app.close_developer_object_list();
            windows.close(key);
        }
        Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } => {
            windows.resize(key, size.width.max(1), size.height.max(1));
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::ScaleFactorChanged { .. },
            ..
        } => {
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. },
            ..
        } => {
            let Some(list) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_object_list_mut)
            else {
                return;
            };
            list.surface.last_pointer = (position.x as i32, position.y as i32);
            let (point, extent) = (list.surface.last_pointer, list.surface_extent());
            if app.developer_pane_scroll_drag(DeveloperPane::ObjectList, point, extent) {
                windows.request_redraw(key);
            }
        }
        // Ctrl and Shift decide what a click on a row means, and the shell
        // never sees these messages, so this window has to track them itself.
        Event::WindowEvent {
            event: WindowEvent::ModifiersChanged(modifiers),
            ..
        } => {
            app.keyboard_modifiers = modifiers.state();
        }
        // The tree view has focus while its window does, so GTK's own key
        // handling applies: the cursor walks the visible rows, Left/Right work
        // the disclosure, and Ctrl/Shift separate the cursor from the
        // selection (`C4ObjectListDlg.cpp:726-787`).
        Event::WindowEvent {
            event:
                WindowEvent::KeyboardInput {
                    event: input @ winit::event::KeyEvent { state, .. },
                    ..
                },
            ..
        } => {
            if *state != winit::event::ElementState::Pressed {
                return;
            }
            let Some(list) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_object_list_mut)
            else {
                return;
            };
            let height = list.surface_extent().1;
            let modifiers = app.keyboard_modifiers;
            let legacy = crate::legacy_virtual_key_from_event(input, modifiers);
            let claimed = match object_list_navigation_key(legacy) {
                Some(navigation) => app.navigate_developer_object_list(
                    navigation,
                    modifiers.control_key(),
                    modifiers.shift_key(),
                    height,
                ),
                // Ctrl+Space is GTK's "select what the cursor is on" without
                // disturbing the rest of a multiple selection.
                None if modifiers.control_key() && legacy == Some(crate::VirtualKeyCode::Space) => {
                    app.toggle_developer_object_list_cursor_selection()
                }
                None => false,
            };
            if claimed {
                windows.request_redraw(key);
            }
        }
        // The tree sits in an automatic scrolled window
        // (`C4ObjectListDlg.cpp:747-780`), so the wheel moves the view and
        // leaves the selection alone.
        Event::WindowEvent {
            event: WindowEvent::MouseWheel { delta, .. },
            ..
        } => {
            use clonk_engine::developer_viewport::{wheel_scroll_step, WheelDelta};

            let Some(list) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_object_list_mut)
            else {
                return;
            };
            let height = list.surface_extent().1;
            let delta = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    WheelDelta::Lines { x: *x, y: *y }
                }
                winit::event::MouseScrollDelta::PixelDelta(position) => WheelDelta::Pixels {
                    x: position.x as f32,
                    y: position.y as f32,
                },
            };
            let (_, rows_delta) = wheel_scroll_step(delta);
            if rows_delta != 0 && app.scroll_developer_object_list(rows_delta, height) {
                windows.request_redraw(key);
            }
        }
        // A release ends a held thumb wherever the pointer finished, which is
        // what makes a drag that leaves the bar still end cleanly.
        Event::WindowEvent {
            event:
                WindowEvent::MouseInput {
                    state: winit::event::ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                },
            ..
        } => {
            if app.developer_pane_scroll_release() {
                windows.request_redraw(key);
            }
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
            let Some(list) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_object_list_mut)
            else {
                return;
            };
            let (point, extent) = (list.surface.last_pointer, list.surface_extent());
            // The bar is a sibling widget inside the scrolled window, so a
            // press it takes never reaches the tree.
            if app.developer_pane_scroll_press(DeveloperPane::ObjectList, point, extent) {
                windows.request_redraw(key);
                return;
            }
            app.developer_object_list_click(point, extent);
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } => {
            if let Some(host) = windows.host_mut(key) {
                if let Err(error) = host.present(app) {
                    tracing::error!(%error, "developer object list present failed");
                    // A lost surface is not recoverable here: the window has
                    // hidden itself, and the list keeps nothing worth waiting
                    // for, so it closes as its own close button would.
                    app.close_developer_object_list();
                }
            }
        }
        _ => {}
    }
}

/// The console scoreboard's registry key, if its dialog has a window.
pub(crate) fn scoreboard_window_key(windows: &DeveloperWindows<DeveloperHost>) -> Option<WindowId> {
    windows.find_key(|host| matches!(host, DeveloperHost::Scoreboard(_)))
}

/// Open, resize or close the console scoreboard window to match its dialog.
///
/// `Dialog::Show` creates the window and `Dialog::Close` destroys it
/// (`C4GuiDialogs.cpp:659-661,677`), so the window's lifetime is exactly the
/// dialog's — this reconciles against `GameApp::console_scoreboard_window_open`
/// rather than being commanded, the same shape as the object list above.
/// `CreateConsoleWindow` returns early when the dialog already has a window
/// (`:308`), which is why a repeated show is not a second window.
///
/// While it is open the window also follows `Dialog::UpdateSize`
/// (`C4GuiDialogs.cpp:445-473`): a live `SetScoreboardData` that grows the
/// dialog grows the window with it.
pub(crate) fn reconcile_console_scoreboard_window(
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
    next_key: &mut u64,
    target: &winit::event_loop::ActiveEventLoop,
) {
    use crate::developer_windows::HostPurpose;
    use crate::scoreboard_window_host::build_scoreboard_window;

    let key = scoreboard_window_key(windows);
    let chrome = app.console_scoreboard_window_chrome();
    match (chrome, key) {
        (Some((title, width, height)), None) => {
            // `CStdWindow::RestorePosition` reads the stored entry as the
            // window is created (`StdRegistry.cpp:300-327`); a missing or
            // unparsable one simply leaves the platform default.
            let restored = crate::load_console_dialog_window_position(
                app.app_paths.as_ref(),
                crate::scoreboard_window_host::SCOREBOARD_DIALOG_ID,
            )
            .and_then(crate::console_window_position::ConsoleWindowPlacement::position);
            match build_scoreboard_window(target, &title, width, height, restored) {
                Ok(host) => {
                    let key = WindowId(*next_key);
                    *next_key += 1;
                    tracing::debug!("opened the console scoreboard window");
                    windows.insert(
                        key,
                        HostPurpose::Scoreboard,
                        DeveloperHost::Scoreboard(host),
                    );
                }
                Err(error) => {
                    tracing::error!(%error, "failed to open the console scoreboard");
                    // `Dialog::Show` returns false without showing when
                    // `CreateConsoleWindow` fails, and `ShowRemoveDlg` then deletes
                    // the dialog (`C4GuiDialogs.cpp:661`, `:1091-1101`). Leaving the
                    // dialog open would retry the failed build every pass.
                    app.close_scoreboard_dialog();
                }
            }
        }
        (Some((title, width, height)), Some(key)) => {
            if let Some(board) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_scoreboard_mut)
            {
                board.set_chrome(&title, width, height);
            }
        }
        // No dialog, or a board that cannot be laid out — both are states in
        // which C++ has no window either.
        (None, Some(key)) => {
            store_console_scoreboard_geometry(app, windows, key);
            windows.close(key);
            // `Dialog::Close` hands focus back to the parent console.
            windows.show_and_focus(crate::developer_windows::SHELL_WINDOW);
        }
        (None, None) => {}
    }
}

/// Remember where the scoreboard window was before it is destroyed.
///
/// `CStdWindow::StorePosition` writes the entry on the way down
/// (`StdRegistry.cpp:290-298`), and `GetPositionData` sets `storeSize` for a
/// dialog (`C4GuiDialogs.cpp:291`), so the size goes with the position even
/// though the dialog resizes itself again on its next Update.
fn store_console_scoreboard_geometry(
    app: &crate::GameApp,
    windows: &DeveloperWindows<DeveloperHost>,
    key: WindowId,
) {
    let Some(paths) = app.app_paths.as_ref() else {
        return;
    };
    let Some(board) = windows.host(key).and_then(|host| match host {
        DeveloperHost::Scoreboard(board) => Some(board),
        _ => None,
    }) else {
        return;
    };
    let Some((x, y)) = board.position() else {
        return;
    };
    let (width, height) = board.surface_extent();
    if let Err(error) = crate::store_console_dialog_window_position(
        paths,
        crate::scoreboard_window_host::SCOREBOARD_DIALOG_ID,
        x,
        y,
        width as i32,
        height as i32,
    ) {
        tracing::warn!(%error, "failed to store the console scoreboard geometry");
    }
}

/// The console scoreboard window's own events.
pub(crate) fn handle_console_scoreboard_event(
    key: WindowId,
    event: &winit::event::Event<crate::NetworkEventWake>,
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
) {
    use crate::developer_windows::DeveloperWindowPresenter;
    use winit::event::{Event, WindowEvent};

    match event {
        // `DialogWinProc`'s `WM_CLOSE` arm is `dialog->Close(false)`
        // (`C4GuiDialogs.cpp:236`) — the window's own close button closes the
        // dialog exactly as the in-dialog close button does. The reconcile
        // pass then removes the window and returns focus to the console.
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            store_console_scoreboard_geometry(app, windows, key);
            app.close_scoreboard_dialog();
            windows.close(key);
            windows.show_and_focus(crate::developer_windows::SHELL_WINDOW);
        }
        Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } => {
            windows.resize(key, size.width.max(1), size.height.max(1));
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::ScaleFactorChanged { .. },
            ..
        } => {
            windows.request_redraw(key);
        }
        // `DialogWinProc` forwards the dialog window's keys to
        // `Game.DoKeyboardInput` (`C4GuiDialogs.cpp:219-228`), and
        // `ScoreboardToggle` is registered at `KEYSCOPE_Generic`
        // (`C4Game.cpp:3427`), so Tab on this window closes the board.
        //
        // Its pointer events are deliberately not routed: `C4ScoreboardDlg`
        // overrides `IsMouseControlled()` to false (`C4Scoreboard.h:109`), and
        // the console dialog has no close button or title bar of its own to
        // hit (`C4GuiDialogs.cpp:390-395`).
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput {
                event: key_event, ..
            },
            ..
        } => {
            if let Some(legacy) =
                crate::legacy_virtual_key_from_event(key_event, app.keyboard_modifiers)
            {
                if let Err(error) = app.handle_key(legacy, key_event.state) {
                    tracing::error!(%error, "console scoreboard key dispatch failed");
                }
            }
        }
        Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } => {
            if let Some(host) = windows.host_mut(key) {
                if let Err(error) = host.present(app) {
                    tracing::error!(%error, "console scoreboard present failed");
                    app.close_scoreboard_dialog();
                }
            }
        }
        _ => {}
    }
}

/// The console chart's registry key, if its dialog has a window.
pub(crate) fn network_chart_window_key(
    windows: &DeveloperWindows<DeveloperHost>,
) -> Option<WindowId> {
    windows.find_key(|host| matches!(host, DeveloperHost::NetworkChart(_)))
}

/// Open or close the console chart window to match its dialog.
///
/// The same shape as the scoreboard above: `Dialog::Show` creates the window
/// and `Dialog::Close` destroys it (`C4GuiDialogs.cpp:659-661,677`), so this
/// reconciles against `GameApp::console_network_chart_window_open` rather than
/// being commanded. The chart's bounds are fixed, so the resize arm only
/// restores a size something else changed.
pub(crate) fn reconcile_console_network_chart_window(
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
    next_key: &mut u64,
    target: &winit::event_loop::ActiveEventLoop,
) {
    use crate::developer_windows::HostPurpose;
    use crate::network_chart_window_host::build_network_chart_window;

    let key = network_chart_window_key(windows);
    let chrome = app.console_network_chart_window_chrome();
    match (chrome, key) {
        (Some((title, width, height)), None) => {
            let restored = crate::load_console_dialog_window_position(
                app.app_paths.as_ref(),
                crate::network_chart_window_host::NETWORK_CHART_DIALOG_ID,
            )
            .and_then(crate::console_window_position::ConsoleWindowPlacement::position);
            match build_network_chart_window(target, &title, width, height, restored) {
                Ok(host) => {
                    let key = WindowId(*next_key);
                    *next_key += 1;
                    tracing::debug!("opened the console network chart window");
                    windows.insert(
                        key,
                        HostPurpose::NetworkChart,
                        DeveloperHost::NetworkChart(host),
                    );
                }
                Err(error) => {
                    tracing::error!(%error, "failed to open the console network chart");
                    // `Dialog::Show` returns false without showing when
                    // `CreateConsoleWindow` fails, and the dialog is deleted
                    // (`C4GuiDialogs.cpp:661`). Leaving it open would retry the
                    // failed build every pass.
                    app.toggle_network_chart();
                }
            }
        }
        (Some((title, width, height)), Some(key)) => {
            if let Some(chart) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_network_chart_mut)
            {
                chart.set_chrome(&title, width, height);
            }
        }
        (None, Some(key)) => {
            store_console_network_chart_geometry(app, windows, key);
            windows.close(key);
            // `Dialog::Close` hands focus back to the parent console.
            windows.show_and_focus(crate::developer_windows::SHELL_WINDOW);
        }
        (None, None) => {}
    }
}

/// Remember where the chart window was before it is destroyed.
fn store_console_network_chart_geometry(
    app: &crate::GameApp,
    windows: &DeveloperWindows<DeveloperHost>,
    key: WindowId,
) {
    let Some(paths) = app.app_paths.as_ref() else {
        return;
    };
    let Some(chart) = windows.host(key).and_then(|host| match host {
        DeveloperHost::NetworkChart(chart) => Some(chart),
        _ => None,
    }) else {
        return;
    };
    let Some((x, y)) = chart.position() else {
        return;
    };
    let (width, height) = chart.surface_extent();
    if let Err(error) = crate::store_console_dialog_window_position(
        paths,
        crate::network_chart_window_host::NETWORK_CHART_DIALOG_ID,
        x,
        y,
        width as i32,
        height as i32,
    ) {
        tracing::warn!(%error, "failed to store the console network chart geometry");
    }
}

/// The console chart window's own events.
///
/// Unlike the scoreboard, this dialog *is* mouse-controlled: `C4GUI::Tabular`
/// selects a sheet on button down, and that is the one element the window
/// chrome does not already own.
pub(crate) fn handle_console_network_chart_event(
    key: WindowId,
    event: &winit::event::Event<crate::NetworkEventWake>,
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
) {
    use crate::developer_windows::DeveloperWindowPresenter;
    use winit::event::{Event, WindowEvent};

    match event {
        // `DialogWinProc`'s `WM_CLOSE` arm is `dialog->Close(false)`
        // (`C4GuiDialogs.cpp:236`), and closing the chart dialog is what the
        // toggle already does.
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            store_console_network_chart_geometry(app, windows, key);
            app.toggle_network_chart();
            windows.close(key);
            windows.show_and_focus(crate::developer_windows::SHELL_WINDOW);
        }
        Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } => {
            windows.resize(key, size.width.max(1), size.height.max(1));
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::ScaleFactorChanged { .. },
            ..
        } => {
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. },
            ..
        } => {
            if let Some(chart) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_network_chart_mut)
            {
                chart.surface.last_pointer = (position.x as i32, position.y as i32);
            }
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
            let Some(chart) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_network_chart_mut)
            else {
                return;
            };
            let (x, y) = chart.surface.last_pointer;
            if app.console_network_chart_pointer_down(clonk_frontend::GuiPoint::new(
                x as f32, y as f32,
            )) {
                windows.request_redraw(key);
            }
        }
        // `DialogWinProc` forwards the dialog window's keys to
        // `Game.DoKeyboardInput` (`C4GuiDialogs.cpp:219-228`), so the chart
        // toggle pressed on this window closes it.
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput {
                event: key_event, ..
            },
            ..
        } => {
            if let Some(legacy) =
                crate::legacy_virtual_key_from_event(key_event, app.keyboard_modifiers)
            {
                if let Err(error) = app.handle_key(legacy, key_event.state) {
                    tracing::error!(%error, "console network chart key dispatch failed");
                }
            }
        }
        Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } => {
            if let Some(host) = windows.host_mut(key) {
                if let Err(error) = host.present(app) {
                    tracing::error!(%error, "console network chart present failed");
                    app.toggle_network_chart();
                }
            }
        }
        _ => {}
    }
}

/// The component editor's registry key, if one is open.
pub(crate) fn component_editor_window_key(
    windows: &DeveloperWindows<DeveloperHost>,
) -> Option<WindowId> {
    windows.find_key(|host| matches!(host, DeveloperHost::ComponentEditor(_)))
}

/// Open or close the component editor window to match the console's request.
pub(crate) fn reconcile_developer_component_editor_window(
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
    next_key: &mut u64,
    target: &winit::event_loop::ActiveEventLoop,
) {
    use crate::component_editor_window_host::build_component_editor_window;
    use crate::developer_windows::HostPurpose;

    let key = component_editor_window_key(windows);
    match (app.developer_component_editor.is_some(), key) {
        (true, None) => {
            let title = app
                .developer_component_editor
                .as_ref()
                .map(|edit| edit.host.filename().to_owned())
                .unwrap_or_default();
            match build_component_editor_window(target, &title) {
                Ok(host) => {
                    let key = WindowId(*next_key);
                    *next_key += 1;
                    tracing::debug!(%title, "opened the developer component editor");
                    windows.insert(
                        key,
                        HostPurpose::ComponentEditor,
                        DeveloperHost::ComponentEditor(host),
                    );
                }
                Err(error) => {
                    tracing::error!(%error, "failed to open the developer component editor");
                    // Cancel rather than retry: `C4ComponentHost::Cancel`
                    // mutates nothing, so the component is untouched.
                    app.cancel_developer_component_editor();
                }
            }
        }
        (false, Some(key)) => {
            windows.close(key);
        }
        _ => {}
    }
}

/// The component editor window's own events.
///
/// The Win32 dialog is modal with OK and Cancel buttons; this window has a
/// keyboard equivalent, because there is no dialog template to port and a
/// text surface that could not be committed would be pointless.
pub(crate) fn handle_developer_component_editor_event(
    key: WindowId,
    event: &winit::event::Event<crate::NetworkEventWake>,
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
    editor_modifiers: &mut winit::keyboard::ModifiersState,
) {
    use crate::developer_component_editor::ComponentEditorKey;
    use crate::developer_windows::DeveloperWindowPresenter;
    use winit::event::{Event, WindowEvent};
    use winit::keyboard::{Key, NamedKey};

    match event {
        Event::WindowEvent {
            event: WindowEvent::ModifiersChanged(modifiers),
            ..
        } => *editor_modifiers = modifiers.state(),
        // Closing the window is Cancel, which mutates nothing.
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            app.cancel_developer_component_editor();
            windows.close(key);
        }
        Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } => {
            windows.resize(key, size.width.max(1), size.height.max(1));
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::ScaleFactorChanged { .. },
            ..
        } => {
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput {
                event: key_event, ..
            },
            ..
        } if key_event.state == winit::event::ElementState::Pressed => {
            // This window's own modifier state. The shell never sees these
            // messages and neither does a viewport, so a handler that read the
            // shared field would find whatever the last *other* window left
            // there — which is what made the newline unreachable.
            let commit = editor_modifiers.control_key() || editor_modifiers.super_key();
            let Some(edit) = app.developer_component_editor.as_mut() else {
                return;
            };
            // Enter is a **newline**, not OK. The Win32 dialog's edit
            // control is multi-line and its OK is a button; a script editor
            // whose most common key closed it would be unusable, so the
            // commit takes the modifier instead.
            let editing = match &key_event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => Some(ComponentEditorKey::Left),
                Key::Named(NamedKey::ArrowRight) => Some(ComponentEditorKey::Right),
                Key::Named(NamedKey::ArrowUp) => Some(ComponentEditorKey::Up),
                Key::Named(NamedKey::ArrowDown) => Some(ComponentEditorKey::Down),
                Key::Named(NamedKey::Home) => Some(ComponentEditorKey::Home),
                Key::Named(NamedKey::End) => Some(ComponentEditorKey::End),
                Key::Named(NamedKey::Backspace) => Some(ComponentEditorKey::Backspace),
                Key::Named(NamedKey::Delete) => Some(ComponentEditorKey::Delete),
                Key::Named(NamedKey::Enter) if !commit => Some(ComponentEditorKey::Enter),
                _ => None,
            };
            if let Some(editing) = editing {
                // Held shift extends a selection instead of dropping it. Read
                // from this window's own modifiers for the same reason the
                // commit is: a handler reading the shared field would find
                // whatever the last other window left there.
                edit.text
                    .key_extending(editing, editor_modifiers.shift_key());
                windows.request_redraw(key);
                return;
            }
            match &key_event.logical_key {
                Key::Named(NamedKey::Enter) => {
                    app.commit_developer_component_editor();
                    windows.close(key);
                }
                Key::Named(NamedKey::Escape) => {
                    app.cancel_developer_component_editor();
                    windows.close(key);
                }
                Key::Named(NamedKey::Space) => {
                    edit.text.insert(' ');
                    windows.request_redraw(key);
                }
                // The editing shortcuts, which are the reason a modified
                // character is not typed. Cut, copy and paste move text
                // between the editor and the system clipboard; the editor
                // itself only ever handles strings, so a clipboard that is
                // unavailable costs the edit rather than the session.
                Key::Character(text) if commit => {
                    let shortcut = text.chars().next().map(|c| c.to_ascii_lowercase());
                    match shortcut {
                        Some('c') => {
                            if let Some(selected) = edit.text.copy_selection() {
                                set_clipboard_text(&selected);
                            }
                        }
                        Some('x') => {
                            if let Some(selected) = edit.text.cut_selection() {
                                set_clipboard_text(&selected);
                            }
                        }
                        Some('v') => {
                            if let Some(text) = clipboard_text() {
                                edit.text.paste(&text);
                            }
                        }
                        // Shift-Z is redo on the same key, which is what a
                        // macOS editor does; Ctrl-Y is the Windows spelling.
                        Some('z') if editor_modifiers.shift_key() => {
                            edit.text.redo();
                        }
                        Some('z') => {
                            edit.text.undo();
                        }
                        Some('y') => {
                            edit.text.redo();
                        }
                        _ => return,
                    }
                    windows.request_redraw(key);
                }
                // A modified character is a shortcut, not text: without this
                // Ctrl-S would type an `s` into the component.
                Key::Character(text) if !commit => {
                    for character in text.chars() {
                        edit.text.insert(character);
                    }
                    windows.request_redraw(key);
                }
                _ => {}
            }
        }
        Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } => {
            if let Some(host) = windows.host_mut(key) {
                if let Err(error) = host.present(app) {
                    tracing::error!(%error, "developer component editor present failed");
                    app.cancel_developer_component_editor();
                }
            }
        }
        _ => {}
    }
}

/// The toolbox window's own events.
pub(crate) fn handle_developer_toolbox_event(
    key: WindowId,
    event: &winit::event::Event<crate::NetworkEventWake>,
    app: &mut crate::GameApp,
    windows: &mut DeveloperWindows<DeveloperHost>,
) {
    use crate::developer_windows::DeveloperWindowPresenter;
    use winit::event::{Event, WindowEvent};

    match event {
        // Closing hides and remembers where it was, so the next open comes
        // back to the same place (`C4DevmodeDlg.cpp:36-42,91-115`).
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            let position = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_toolbox_mut)
                .and_then(|toolbox| toolbox.position());
            app.close_developer_toolbox(position);
            windows.hide(key);
        }
        Event::WindowEvent {
            event: WindowEvent::Resized(size),
            ..
        } => {
            windows.resize(key, size.width.max(1), size.height.max(1));
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::ScaleFactorChanged { .. },
            ..
        } => {
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. },
            ..
        } => {
            let Some(toolbox) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_toolbox_mut)
            else {
                return;
            };
            toolbox.surface.last_pointer = (position.x as i32, position.y as i32);
            let (point, extent) = (toolbox.surface.last_pointer, toolbox.surface_extent());
            if app.developer_toolbox.current_page() == Some(ToolboxPage::Property)
                && app.developer_pane_scroll_drag(DeveloperPane::PropertyOutput, point, extent)
            {
                windows.request_redraw(key);
            }
        }
        // `IDC_COMBOINPUT` has the keyboard while the property page is up, and
        // it is enabled on `Console.Editing` alone
        // (`C4PropertyDlg.cpp:117`). A key it takes is the entry's, not the
        // window's.
        Event::WindowEvent {
            event:
                WindowEvent::KeyboardInput {
                    event: input @ winit::event::KeyEvent { state, .. },
                    ..
                },
            ..
        } => {
            if *state != winit::event::ElementState::Pressed
                || app.developer_toolbox.current_page() != Some(ToolboxPage::Property)
            {
                return;
            }
            let claimed = match crate::legacy_virtual_key_from_event(input, app.keyboard_modifiers)
            {
                Some(crate::VirtualKeyCode::Enter) | Some(crate::VirtualKeyCode::NumpadEnter) => {
                    match app.submit_developer_property_script() {
                        Ok(submitted) => submitted,
                        Err(error) => {
                            tracing::error!(%error, "property script submission failed");
                            true
                        }
                    }
                }
                Some(crate::VirtualKeyCode::Backspace) => app.backspace_developer_property_script(),
                // Everything else is text if it produced any: winit resolves
                // the layout, so this needs no keycode table of its own.
                _ => input
                    .text
                    .as_ref()
                    .map(|text| text.as_str())
                    .filter(|text| !text.chars().any(char::is_control))
                    .is_some_and(|text| app.type_developer_property_script(text)),
            };
            if claimed {
                windows.request_redraw(key);
            }
        }
        // The property output is a scrolled text view
        // (`C4PropertyDlg.cpp:128-140`); the tools page has nothing to scroll.
        Event::WindowEvent {
            event: WindowEvent::MouseWheel { delta, .. },
            ..
        } => {
            use clonk_engine::developer_viewport::{wheel_scroll_step, WheelDelta};

            if app.developer_toolbox.current_page() != Some(ToolboxPage::Property) {
                return;
            }
            let Some(toolbox) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_toolbox_mut)
            else {
                return;
            };
            let height = toolbox.surface_extent().1;
            let delta = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    WheelDelta::Lines { x: *x, y: *y }
                }
                winit::event::MouseScrollDelta::PixelDelta(position) => WheelDelta::Pixels {
                    x: position.x as f32,
                    y: position.y as f32,
                },
            };
            let (_, lines) = wheel_scroll_step(delta);
            if lines != 0 && app.scroll_developer_property_page(lines, height) {
                windows.request_redraw(key);
            }
        }
        // A release ends a held thumb wherever the pointer finished, which is
        // what makes a drag that leaves the bar still end cleanly.
        Event::WindowEvent {
            event:
                WindowEvent::MouseInput {
                    state: winit::event::ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                },
            ..
        } => {
            if app.developer_pane_scroll_release() {
                windows.request_redraw(key);
            }
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
            let Some(toolbox) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_toolbox_mut)
            else {
                return;
            };
            let (point, extent) = (toolbox.surface.last_pointer, toolbox.surface_extent());
            if app.developer_toolbox.current_page() == Some(ToolboxPage::Property)
                && app.developer_pane_scroll_press(DeveloperPane::PropertyOutput, point, extent)
            {
                windows.request_redraw(key);
                return;
            }
            app.developer_toolbox_click(point, extent);
            windows.request_redraw(key);
        }
        Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } => {
            if let Some(host) = windows.host_mut(key) {
                if let Err(error) = host.present(app) {
                    tracing::error!(%error, "developer toolbox present failed");
                }
            }
        }
        _ => {}
    }
}

/// Puts text on the system clipboard, logging rather than failing the edit.
///
/// The editor's own cut and copy are pure string operations; this is the only
/// part that touches the machine, which is what keeps the editor testable
/// without reading or writing the developer's clipboard.
fn set_clipboard_text(text: &str) {
    if let Err(error) =
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.to_string()))
    {
        tracing::warn!(%error, "failed to copy component editor text");
    }
}

/// Reads the system clipboard, or `None` when there is nothing to paste.
fn clipboard_text() -> Option<String> {
    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
        Ok(text) => Some(text),
        Err(error) => {
            tracing::warn!(%error, "failed to paste into the component editor");
            None
        }
    }
}

/// Which navigation the list understands a key as.
///
/// Only the keys `GtkTreeView` binds itself: everything else is left for the
/// window's other owners rather than swallowed here.
fn object_list_navigation_key(
    key: Option<crate::VirtualKeyCode>,
) -> Option<crate::developer_object_list_view::ObjectListKey> {
    use crate::developer_object_list_view::ObjectListKey;
    use crate::VirtualKeyCode;

    match key? {
        VirtualKeyCode::ArrowUp => Some(ObjectListKey::Up),
        VirtualKeyCode::ArrowDown => Some(ObjectListKey::Down),
        VirtualKeyCode::ArrowLeft => Some(ObjectListKey::Left),
        VirtualKeyCode::ArrowRight => Some(ObjectListKey::Right),
        VirtualKeyCode::Home => Some(ObjectListKey::Home),
        VirtualKeyCode::End => Some(ObjectListKey::End),
        VirtualKeyCode::PageUp => Some(ObjectListKey::PageUp),
        VirtualKeyCode::PageDown => Some(ObjectListKey::PageDown),
        _ => None,
    }
}
