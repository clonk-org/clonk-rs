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
            if let Some(list) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_object_list_mut)
            {
                list.surface.last_pointer = (position.x as i32, position.y as i32);
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
                edit.text.key(editing);
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
            if let Some(toolbox) = windows
                .host_mut(key)
                .and_then(DeveloperHost::as_toolbox_mut)
            {
                toolbox.surface.last_pointer = (position.x as i32, position.y as i32);
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
