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
use crate::developer_windows::{DeveloperWindows, WindowId};

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
    use crate::developer_windows::{DeveloperWindowHost, HostPurpose};
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
                    if let Some(host) = windows.host_mut(key) {
                        host.set_visible(true);
                        host.request_redraw();
                    }
                    if let Some(DeveloperHost::Toolbox(toolbox)) = windows.host_mut(key) {
                        toolbox.window.set_title(&title);
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
                        }
                        Err(error) => {
                            tracing::error!(%error, "failed to open the developer toolbox");
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
                    toolbox.window.set_title(&title);
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
            let effect = app.developer_toolbox.close(position);
            app.developer_toolbox_effects.extend(effect);
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
            if let Some(toolbox) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_toolbox_mut)
            {
                toolbox.last_pointer = (position.x as i32, position.y as i32);
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
            let (point, extent) = (toolbox.last_pointer, toolbox.surface_extent());
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
