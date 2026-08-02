// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

#[inline(never)]
fn boxed_running_sandbox_app() -> Box<GameApp> {
    Box::new(new_running_sandbox_app())
}

#[inline(never)]
fn boxed_classic_running_sandbox_app() -> Box<GameApp> {
    Box::new(new_classic_running_sandbox_app())
}

#[test]
fn l143_default_z_dialog_order_tracks_show_raise_and_close() {
    let mut app = new_game_over_keyboard_app();
    assert_eq!(
        app.runtime_default_dialog_order_snapshot(),
        vec![RuntimeDefaultDialog::GameOver]
    );

    app.toggle_network_chart();
    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
    app.toggle_runtime_client_list().expect("open client list");
    app.external_irc_dialog_visible = true;
    app.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
    assert_eq!(
        app.runtime_default_dialog_order_snapshot(),
        vec![
            RuntimeDefaultDialog::GameOver,
            RuntimeDefaultDialog::NetworkChart,
            RuntimeDefaultDialog::ClientList,
            RuntimeDefaultDialog::ExternalIrc,
        ]
    );
    assert!(app.runtime_client_list_above_game_over);
    assert!(app.runtime_top_default_dialog_is_exclusive());

    app.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::GameOver);
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver));
    assert!(!app.runtime_client_list_above_game_over);
    app.dismiss_game_over_dialog();
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc));
    app.external_irc_dialog_visible = false;
    app.hide_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::ClientList));
    app.toggle_runtime_client_list().expect("close client list");
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart));
    app.toggle_network_chart();
    assert!(app.runtime_default_dialog_order_snapshot().is_empty());
}

#[test]
fn l143_non_left_runtime_dialog_hits_swallow_without_raising() {
    let mut app = new_game_over_keyboard_app();
    app.resize(1280, 720)
        .expect("resize pointer-routing fixture");
    let outside = GuiPoint::new(0.0, 0.0);
    assert!(!app.game_over_dialog_contains_point(outside));
    assert!(app.game_over_pointer_route_hit(outside));

    app.toggle_network_chart();
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    let game_over_only = (0..height)
        .step_by(4)
        .find_map(|y| {
            (0..width)
                .step_by(4)
                .map(|x| GuiPoint::new(x as f32, y as f32))
                .find(|point| {
                    app.game_over_dialog_contains_point(*point)
                        && !app.network_chart_contains_point(*point)
                })
        })
        .expect("evaluation has an exposed point outside the chart");
    assert!(app.game_over_dialog_contains_point(game_over_only));
    assert!(!app.network_chart_contains_point(game_over_only));
    assert!(!app.game_over_pointer_route_hit(outside));
    let order = app.runtime_default_dialog_order_snapshot();
    app.running_pointer_position = Some(game_over_only);

    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0)
        .expect("lower game-over swallows an in-bounds wheel");
    app.handle_right_mouse_button(ElementState::Pressed)
        .expect("lower game-over swallows an in-bounds right press");
    app.handle_right_mouse_button(ElementState::Released)
        .expect("lower game-over swallows an in-bounds right release");
    app.handle_other_mouse_button(ElementState::Pressed)
        .expect("lower game-over swallows an in-bounds middle press");
    app.handle_other_mouse_button(ElementState::Released)
        .expect("lower game-over swallows an in-bounds middle release");
    assert_eq!(app.runtime_default_dialog_order_snapshot(), order);
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart));

    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(game_over_only.x),
        f64::from(game_over_only.y),
    ))
    .expect("move reaches the exposed lower game-over chassis");
    app.handle_mouse_button_classified(ElementState::Pressed, false)
        .expect("left press activates the exposed lower game-over dialog");
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver));
    app.handle_mouse_button_classified(ElementState::Released, false)
        .expect("release the game-over activation gesture");
}

#[test]
fn running_chat_global_bindings_open_above_lower_messages_and_contexts() {
    let notice = || {
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Lower notice",
            "Message",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        )
    };

    let mut f2 = boxed_running_sandbox_app();
    f2.push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push lower message");
    let layout = f2.top_message_dialog_layout().expect("message layout");
    let button = layout.buttons.first().expect("message button").rect;
    f2.handle_cursor_moved(PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    ))
    .expect("hover lower message button");
    f2.handle_mouse_button(ElementState::Pressed)
        .expect("capture lower message button");
    assert!(f2.message_dialogs[0].state.has_pointer_capture());
    assert_eq!(f2.message_dialog_pointer_capture_index, Some(0));
    f2.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
        .expect("F2 opens chat above lower message");
    assert_eq!(f2.running_chat_text(), Some(""));
    assert_eq!(f2.message_dialogs.len(), 1);
    assert!(f2.message_dialogs[0].state.has_pointer_capture());
    f2.handle_mouse_button(ElementState::Released)
        .expect("release retained lower-message capture through chat");
    assert!(f2.message_dialogs.is_empty());
    assert!(f2.running_chat_active());

    let mut focus_loss = boxed_running_sandbox_app();
    focus_loss
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push lower message for focus-loss capture");
    let layout = focus_loss
        .top_message_dialog_layout()
        .expect("message layout");
    let button = layout.buttons.first().expect("message button").rect;
    focus_loss
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(button.x + button.w / 2),
            f64::from(button.y + button.h / 2),
        ))
        .expect("hover lower focus-loss button");
    focus_loss
        .handle_mouse_button(ElementState::Pressed)
        .expect("capture lower focus-loss button");
    focus_loss
        .handle_key(VirtualKeyCode::F2, ElementState::Pressed)
        .expect("open chat over retained focus-loss capture");
    focus_loss
        .handle_focus_lost()
        .expect("focus loss clears captures below active chat");
    assert!(!focus_loss.message_dialogs[0].state.has_pointer_capture());
    assert_eq!(focus_loss.message_dialog_pointer_capture_index, None);
    assert!(!focus_loss.primary_pointer_left_down);
    focus_loss
        .handle_mouse_button(ElementState::Released)
        .expect("post-focus release cannot activate lower message");
    assert_eq!(focus_loss.message_dialogs.len(), 1);

    for (modifiers, expected) in [
        (ModifiersState::SHIFT, "/team "),
        (ModifiersState::ALT, "\""),
    ] {
        let mut app = boxed_running_sandbox_app();
        app.push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("push lower message");
        app.handle_modifiers_changed(modifiers)
            .expect("set chat-open modifier");
        app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
            .expect("modified Return falls through lower message to chat");
        assert_eq!(app.running_chat_text(), Some(expected));
        assert_eq!(app.message_dialogs.len(), 1);
    }

    let mut bare_return = boxed_running_sandbox_app();
    bare_return
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push lower message for bare Return");
    bare_return
        .handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("bare Return opens chat above nonexclusive lower message");
    assert_eq!(bare_return.running_chat_text(), Some(""));
    assert_eq!(bare_return.message_dialogs.len(), 1);

    let lower_layout = bare_return
        .top_message_dialog_layout()
        .expect("lower message layout under chat");
    let lower_point = PhysicalPosition::new(
        f64::from(lower_layout.bounds.x + 5),
        f64::from(lower_layout.bounds.y + 5),
    );
    bare_return
        .handle_cursor_moved(lower_point)
        .expect("hover lower message outside compact chat");
    bare_return
        .handle_mouse_button(ElementState::Pressed)
        .expect("activate lower shared-screen message");
    bare_return
        .handle_mouse_button(ElementState::Released)
        .expect("release lower shared-screen message");
    assert!(!bare_return.running_chat_active());
    bare_return
        .handle_text_input('x')
        .expect("inactive chat ignores text while lower message owns keys");
    assert_eq!(bare_return.running_chat_text(), Some(""));

    let chat_layout = bare_return.game_option_input_layout().expect("chat layout");
    let chat_point = PhysicalPosition::new(
        f64::from(chat_layout.edit.x + chat_layout.edit.w / 2),
        f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
    );
    bare_return
        .handle_cursor_moved(chat_point)
        .expect("hover chat above lower message");
    bare_return
        .handle_mouse_button(ElementState::Pressed)
        .expect("reactivate compact chat");
    bare_return
        .handle_mouse_button(ElementState::Released)
        .expect("release compact chat click");
    assert!(bare_return.running_chat_active());
    bare_return
        .handle_text_input('x')
        .expect("reactivated chat accepts text");
    assert_eq!(bare_return.running_chat_text(), Some("x"));

    let mut inactive_return = boxed_running_sandbox_app();
    inactive_return.start_running_chat(RunningChatMode::All);
    inactive_return
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push message below visible chat for active-key routing");
    let lower_layout = inactive_return
        .top_message_dialog_layout()
        .expect("inactive-key lower message layout");
    inactive_return
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(lower_layout.bounds.x + 5),
            f64::from(lower_layout.bounds.y + 5),
        ))
        .expect("hover lower message for active-key routing");
    inactive_return
        .handle_mouse_button(ElementState::Pressed)
        .expect("activate lower message for Return routing");
    inactive_return
        .handle_mouse_button(ElementState::Released)
        .expect("release lower-message activation click");
    inactive_return
        .handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("active lower message owns Return down");
    assert_eq!(inactive_return.message_dialogs.len(), 1);
    assert!(!inactive_return.running_chat_active());
    inactive_return
        .handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("active lower message owns Return up");
    assert!(inactive_return.message_dialogs.is_empty());
    assert!(inactive_return.running_chat_active());

    let mut held_drag = boxed_running_sandbox_app();
    held_drag.start_running_chat(RunningChatMode::All);
    held_drag
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push message below chat for held-pointer activation");
    let lower_layout = held_drag
        .top_message_dialog_layout()
        .expect("held-pointer lower message layout");
    let lower_button = lower_layout.buttons.first().expect("lower OK button").rect;
    held_drag
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(lower_button.x + lower_button.w / 2),
            f64::from(lower_button.y + lower_button.h / 2),
        ))
        .expect("hover lower button for held-pointer activation");
    held_drag
        .handle_mouse_button(ElementState::Pressed)
        .expect("press lower button while chat is visible");
    assert!(!held_drag.running_chat_active());
    let chat_layout = held_drag
        .game_option_input_layout()
        .expect("held chat layout");
    held_drag
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(chat_layout.edit.x + chat_layout.edit.w / 2),
            f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
        ))
        .expect("held left movement activates the hit chat dialog");
    assert!(held_drag.running_chat_active());
    held_drag
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(lower_button.x + lower_button.w / 2),
            f64::from(lower_button.y + lower_button.h / 2),
        ))
        .expect("active chat retains held routing outside its bounds");
    held_drag
        .handle_mouse_button(ElementState::Released)
        .expect("lower button cannot re-arm after chat activation");
    assert_eq!(held_drag.message_dialogs.len(), 1);

    let mut label_drag = boxed_running_sandbox_app();
    label_drag.start_running_chat(RunningChatMode::All);
    label_drag
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push lower message for noncapturing chat-label drag");
    let chat_layout = label_drag
        .game_option_input_layout()
        .expect("label chat layout");
    let label_point = PhysicalPosition::new(
        f64::from(chat_layout.message.x + chat_layout.message.w / 2),
        f64::from(chat_layout.message.y + chat_layout.message.h / 2),
    );
    let message_layout = label_drag
        .top_message_dialog_layout()
        .expect("label-drag lower message layout");
    let lower_point = PhysicalPosition::new(
        f64::from(message_layout.bounds.x + 5),
        f64::from(message_layout.bounds.y + 5),
    );
    label_drag
        .handle_cursor_moved(label_point)
        .expect("hover the inert chat label");
    label_drag
        .handle_mouse_button(ElementState::Pressed)
        .expect("press the inert chat label");
    assert_eq!(label_drag.game_option_input_pointer_capture, None);
    assert!(label_drag.primary_pointer_left_down);
    label_drag
        .handle_cursor_moved(lower_point)
        .expect("held label drag activates the hit lower message");
    assert!(!label_drag.running_chat_active());
    assert_eq!(label_drag.active_message_dialog_index(), Some(0));
    label_drag
        .handle_mouse_button(ElementState::Released)
        .expect("release the noncapturing label drag");
    assert!(!label_drag.primary_pointer_left_down);

    let mut touch_lower = boxed_running_sandbox_app();
    touch_lower.start_running_chat(RunningChatMode::All);
    touch_lower
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push lower message for shared touch routing");
    let message_layout = touch_lower
        .top_message_dialog_layout()
        .expect("touch lower message layout");
    let lower_touch = GuiPoint::new(
        (message_layout.bounds.x + 5) as f32,
        (message_layout.bounds.y + 5) as f32,
    );
    touch_lower
        .handle_touch(TouchPhase::Started, lower_touch)
        .expect("touch starts on the exposed lower message");
    assert!(!touch_lower.running_chat_active());
    assert_eq!(touch_lower.active_message_dialog_index(), Some(0));
    touch_lower
        .handle_touch(TouchPhase::Ended, lower_touch)
        .expect("touch ends on the lower message");

    let mut release_hit = boxed_running_sandbox_app();
    release_hit.start_running_chat(RunningChatMode::All);
    release_hit
        .push_message_dialog(
            notice().with_checkbox("&Remember", false),
            MessageDialogContinuation::None,
        )
        .expect("push checkbox message below captured chat edit");
    let message_layout = release_hit
        .top_message_dialog_layout()
        .expect("checkbox message layout");
    let checkbox = message_layout
        .checkbox
        .as_ref()
        .expect("checkbox layout")
        .square;
    let checkbox_point = PhysicalPosition::new(
        f64::from(checkbox.x + checkbox.w / 2),
        f64::from(checkbox.y + checkbox.h / 2),
    );
    let chat_layout = release_hit
        .game_option_input_layout()
        .expect("edit chat layout");
    let edit_point = PhysicalPosition::new(
        f64::from(chat_layout.edit.x + 5),
        f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
    );
    release_hit
        .handle_cursor_moved(edit_point)
        .expect("hover chat edit");
    release_hit
        .handle_mouse_button(ElementState::Pressed)
        .expect("chat edit installs pDragElement");
    assert_eq!(
        release_hit.game_option_input_pointer_capture,
        Some(ContextMenuPointerButton::Left),
    );
    release_hit
        .handle_cursor_moved(checkbox_point)
        .expect("chat edit capture retains held motion over checkbox");
    release_hit
        .handle_mouse_button(ElementState::Released)
        .expect("release clears chat capture before checkbox hit-testing");
    assert_eq!(release_hit.game_option_input_pointer_capture, None);
    assert_eq!(
        release_hit.message_dialogs[0].state.checkbox_checked(),
        Some(true),
    );

    let mut close_active_chat = boxed_running_sandbox_app();
    close_active_chat.start_running_chat(RunningChatMode::All);
    close_active_chat
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push lower message for active-chat close cleanup");
    let message_layout = close_active_chat
        .top_message_dialog_layout()
        .expect("close-cleanup message layout");
    let button = message_layout
        .buttons
        .first()
        .expect("close-cleanup button")
        .rect;
    close_active_chat
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(button.x + button.w / 2),
            f64::from(button.y + button.h / 2),
        ))
        .expect("hover cleanup button");
    close_active_chat
        .handle_mouse_button(ElementState::Pressed)
        .expect("capture cleanup button");
    let chat_layout = close_active_chat
        .game_option_input_layout()
        .expect("close-cleanup chat layout");
    close_active_chat
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(chat_layout.edit.x + 5),
            f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
        ))
        .expect("held move activates chat above retained capture");
    assert!(close_active_chat.running_chat_active());
    assert_eq!(
        close_active_chat.message_dialog_pointer_capture_index,
        Some(0)
    );
    close_active_chat
        .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("closing active chat releases all mouse elements");
    assert!(close_active_chat.running_chat.is_none());
    assert_eq!(close_active_chat.message_dialog_pointer_capture_index, None);
    assert!(!close_active_chat.message_dialogs[0]
        .state
        .has_pointer_capture());

    let mut stacked_active = boxed_running_sandbox_app();
    stacked_active.start_running_chat(RunningChatMode::All);
    stacked_active
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push first lower message");
    let first_layout = stacked_active
        .top_message_dialog_layout()
        .expect("first lower message layout");
    stacked_active
        .handle_cursor_moved(PhysicalPosition::new(
            f64::from(first_layout.bounds.x + 5),
            f64::from(first_layout.bounds.y + 5),
        ))
        .expect("hover first lower message");
    stacked_active
        .handle_mouse_button(ElementState::Pressed)
        .expect("activate first lower message");
    stacked_active
        .handle_mouse_button(ElementState::Released)
        .expect("release first lower activation");
    assert_eq!(stacked_active.active_message_dialog_index(), Some(0));

    let vote = || {
        clonk_frontend::message_dialog::MessageDialogState::new(
            "Vote?",
            "Voting",
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
            clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
            clonk_frontend::message_dialog::MessageDialogSize::Regular,
            true,
        )
    };
    let small_vote = || {
        clonk_frontend::message_dialog::MessageDialogState::new(
            "Vote?",
            "Voting",
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
            clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
            clonk_frontend::message_dialog::MessageDialogSize::Small,
            true,
        )
    };
    let small_notice = || {
        clonk_frontend::message_dialog::MessageDialogState::new(
            "Top notice",
            "Message",
            clonk_frontend::message_dialog::MessageDialogButtons::OK,
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            clonk_frontend::message_dialog::MessageDialogSize::Small,
            false,
        )
    };
    stacked_active
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("insert second message below inactive chat");
    assert_eq!(stacked_active.active_message_dialog_index(), Some(0));
    stacked_active
        .handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("previous lower active dialog owns Return down");
    stacked_active
        .handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("previous lower active dialog owns Return up");
    assert_eq!(stacked_active.message_dialogs.len(), 1);
    assert!(matches!(
        stacked_active.message_dialogs[0].continuation,
        MessageDialogContinuation::LeagueSurrender
    ));
    assert!(stacked_active.running_chat_active());

    let mut stacked_capture = boxed_running_sandbox_app();
    stacked_capture.start_running_chat(RunningChatMode::All);
    stacked_capture
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push captured dialog A below chat");
    let layout = stacked_capture
        .top_message_dialog_layout()
        .expect("captured dialog A layout");
    let button = layout.buttons.first().expect("dialog A button").rect;
    let button_point = PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    );
    stacked_capture
        .handle_cursor_moved(button_point)
        .expect("hover dialog A button");
    stacked_capture
        .handle_mouse_button(ElementState::Pressed)
        .expect("dialog A acquires global drag capture");
    assert_eq!(
        stacked_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    stacked_capture
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("insert dialog B above captured A but below chat");
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(0));
    let small_layout = stacked_capture
        .top_message_dialog_layout()
        .expect("smaller dialog B layout");
    let button_gui_point = GuiPoint::new(button_point.x as f32, button_point.y as f32);
    assert!(GameApp::point_in_message_dialog_bounds(
        button_gui_point,
        &small_layout,
    ));
    let a_only_point = PhysicalPosition::new(
        f64::from(layout.bounds.x + 5),
        f64::from(layout.bounds.y + 5),
    );
    assert!(!GameApp::point_in_message_dialog_bounds(
        GuiPoint::new(a_only_point.x as f32, a_only_point.y as f32),
        &small_layout,
    ));

    stacked_capture
        .handle_mouse_button(ElementState::Released)
        .expect("release hit-tests B after clearing A's global capture");
    assert_eq!(stacked_capture.message_dialogs.len(), 2);
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(0));
    assert_eq!(stacked_capture.message_dialog_pointer_capture_index, None);
    assert!(stacked_capture
        .message_dialogs
        .iter()
        .all(|dialog| !dialog.state.has_pointer_capture()));

    let mut exposed_lower = boxed_running_sandbox_app();
    exposed_lower
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push regular shared-screen dialog A");
    let regular_layout = exposed_lower
        .top_message_dialog_layout()
        .expect("regular dialog A layout");
    exposed_lower
        .push_message_dialog(small_vote(), MessageDialogContinuation::None)
        .expect("push smaller shared-screen dialog B");
    let small_layout = exposed_lower
        .top_message_dialog_layout()
        .expect("smaller dialog B layout");
    let close = regular_layout
        .close_button
        .expect("regular dialog A close button");
    let exposed_point = PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    );
    assert!(!GameApp::point_in_message_dialog_bounds(
        GuiPoint::new(exposed_point.x as f32, exposed_point.y as f32),
        &small_layout,
    ));
    exposed_lower
        .handle_cursor_moved(exposed_point)
        .expect("hover the exposed lower dialog A");
    exposed_lower
        .handle_mouse_button(ElementState::Pressed)
        .expect("left-down activates and captures exposed lower dialog A");
    assert_eq!(exposed_lower.active_message_dialog_index(), Some(0));
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
    let top_point = PhysicalPosition::new(
        f64::from(small_layout.bounds.x + small_layout.bounds.w / 2),
        f64::from(small_layout.bounds.y + small_layout.bounds.h / 2),
    );
    exposed_lower
        .handle_cursor_moved(top_point)
        .expect("held move into B activates it without transferring A capture");
    assert_eq!(exposed_lower.active_message_dialog_index(), Some(1));
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
    exposed_lower
        .handle_cursor_moved(exposed_point)
        .expect("active B blocks the lower A-only hit while capture remains");
    assert_eq!(exposed_lower.active_message_dialog_index(), Some(1));
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
    exposed_lower
        .handle_mouse_button(ElementState::Released)
        .expect("A-only release clears A capture without closing it");
    assert_eq!(exposed_lower.message_dialogs.len(), 2);
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, None);

    let mut inserted_capture = boxed_running_sandbox_app();
    inserted_capture
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push dialog A before an asynchronous insertion");
    let regular_layout = inserted_capture
        .top_message_dialog_layout()
        .expect("asynchronous dialog A layout");
    let close = regular_layout
        .close_button
        .expect("asynchronous dialog A close button");
    let close_point = PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    );
    inserted_capture
        .handle_cursor_moved(close_point)
        .expect("hover dialog A close before insertion");
    inserted_capture
        .handle_mouse_button(ElementState::Pressed)
        .expect("dialog A captures before insertion");
    inserted_capture
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("insert exclusive dialog B without releasing A capture");
    assert_eq!(inserted_capture.active_message_dialog_index(), Some(1));
    assert_eq!(
        inserted_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    assert!(inserted_capture.message_dialogs[0]
        .state
        .has_pointer_capture());
    let small_layout = inserted_capture
        .top_message_dialog_layout()
        .expect("asynchronous dialog B layout");
    let top_point = PhysicalPosition::new(
        f64::from(small_layout.bounds.x + small_layout.bounds.w / 2),
        f64::from(small_layout.bounds.y + small_layout.bounds.h / 2),
    );
    inserted_capture
        .handle_cursor_moved(top_point)
        .expect("active B owns held motion after insertion");
    inserted_capture
        .handle_mouse_button(ElementState::Released)
        .expect("B hit clears the retained A capture");
    assert_eq!(inserted_capture.message_dialogs.len(), 2);
    assert_eq!(inserted_capture.message_dialog_pointer_capture_index, None);

    let exposed_point = PhysicalPosition::new(
        f64::from(regular_layout.bounds.x + 5),
        f64::from(regular_layout.bounds.y + 5),
    );
    assert!(!GameApp::point_in_message_dialog_bounds(
        GuiPoint::new(exposed_point.x as f32, exposed_point.y as f32),
        &small_layout,
    ));
    inserted_capture
        .handle_cursor_moved(exposed_point)
        .expect("hover A outside the smaller exclusive B");
    inserted_capture
        .handle_mouse_button(ElementState::Pressed)
        .expect("exclusive B still permits shared-screen A hit-testing");
    assert_eq!(inserted_capture.active_message_dialog_index(), Some(0));
    inserted_capture
        .handle_mouse_button(ElementState::Released)
        .expect("release the exposed A click");

    stacked_capture
        .remove_message_dialog_at(1)
        .expect("remove B to press A again");
    stacked_capture
        .handle_cursor_moved(button_point)
        .expect("hover A before a second gesture");
    stacked_capture
        .handle_mouse_button(ElementState::Pressed)
        .expect("dialog A reacquires capture");
    stacked_capture
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("insert B above A during the second drag");
    stacked_capture
        .handle_cursor_moved(button_point)
        .expect("captured A drags first, then overlapping B activates");
    assert_eq!(
        stacked_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
    stacked_capture
        .handle_cursor_moved(a_only_point)
        .expect("active B blocks a lower A-only hit while capture remains");
    assert_eq!(
        stacked_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
    stacked_capture
        .handle_mouse_button(ElementState::Released)
        .expect("A-only release clears A capture without reactivating its button");
    assert_eq!(stacked_capture.message_dialogs.len(), 2);
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
    assert_eq!(stacked_capture.message_dialog_pointer_capture_index, None);
    assert!(stacked_capture
        .message_dialogs
        .iter()
        .all(|dialog| !dialog.state.has_pointer_capture()));

    let mut vote_pointer = boxed_running_sandbox_app();
    vote_pointer
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("push exclusive vote for outside-pointer routing");
    vote_pointer.running_pointer_position = Some(GuiPoint::new(0.0, 0.0));
    assert!(!vote_pointer.handle_message_dialog_pointer_move(GuiPoint::new(0.0, 0.0)));
    assert!(!vote_pointer
        .handle_message_dialog_pointer_button(ElementState::Pressed)
        .expect("outside vote hit-test falls through to shared Screen scanning"));
    assert!(!vote_pointer
        .handle_message_dialog_pointer_button(ElementState::Released)
        .expect("outside vote release falls through to shared Screen scanning"));

    let mut vote_return = boxed_running_sandbox_app();
    vote_return
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("push exclusive vote for bare Return");
    vote_return
        .handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("bare Return remains owned by exclusive vote");
    assert!(vote_return.running_chat.is_none());
    assert_eq!(vote_return.message_dialogs.len(), 1);
    vote_return
        .handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("focused No rejects vote on Return release");
    assert!(vote_return.message_dialogs.is_empty());
    assert_eq!(vote_return.mode, AppMode::Running);

    for (key, modifiers) in [
        (VirtualKeyCode::Enter, ModifiersState::CONTROL),
        (VirtualKeyCode::Space, ModifiersState::CONTROL),
        (VirtualKeyCode::Space, ModifiersState::SHIFT),
        (VirtualKeyCode::Escape, ModifiersState::CONTROL),
        (
            VirtualKeyCode::KeyY,
            ModifiersState::CONTROL | ModifiersState::ALT,
        ),
    ] {
        let mut app = boxed_running_sandbox_app();
        app.push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push vote for exact modifier routing");
        app.handle_modifiers_changed(modifiers)
            .expect("set nonmatching GUI modifiers");
        app.handle_key(key, ElementState::Pressed)
            .expect("nonmatching GUI key down is inert");
        app.handle_key(key, ElementState::Released)
            .expect("nonmatching GUI key up is inert");
        assert_eq!(app.message_dialogs.len(), 1);
        assert!(app.running_chat.is_none());
    }

    let mut unmatched_vote_hotkey = boxed_classic_running_sandbox_app();
    unmatched_vote_hotkey
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("push exclusive vote for unmatched Alt mnemonic");
    unmatched_vote_hotkey
        .handle_modifiers_changed(ModifiersState::ALT)
        .expect("hold Alt over vote");
    unmatched_vote_hotkey
        .handle_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("unmatched vote mnemonic falls through to global Alt+C");
    assert!(unmatched_vote_hotkey.external_irc_dialog_visible);
    unmatched_vote_hotkey
        .handle_key(VirtualKeyCode::KeyC, ElementState::Released)
        .expect("global Alt+C release also falls through the vote");
    assert_eq!(unmatched_vote_hotkey.message_dialogs.len(), 1);

    let mut handled_message_hotkey = boxed_running_sandbox_app();
    handled_message_hotkey
        .push_message_dialog(
            vote().with_checkbox("&Don't display again", false),
            MessageDialogContinuation::LeagueSurrender,
        )
        .expect("push checkbox message for down-only mnemonic");
    handled_message_hotkey
        .handle_modifiers_changed(ModifiersState::ALT)
        .expect("hold Alt over checkbox mnemonic");
    assert!(handled_message_hotkey
        .handle_message_dialog_key(VirtualKeyCode::KeyD, ElementState::Pressed)
        .expect("checkbox mnemonic down is handled"));
    assert_eq!(
        handled_message_hotkey.message_dialogs[0]
            .state
            .checkbox_checked(),
        Some(true)
    );
    assert!(!handled_message_hotkey
        .message_dialog_consumed_keys
        .contains(&VirtualKeyCode::KeyD));
    assert!(!handled_message_hotkey
        .handle_message_dialog_key(VirtualKeyCode::KeyD, ElementState::Released)
        .expect("mnemonic release is not owned by the dialog"));

    let mut changed_release = boxed_running_sandbox_app();
    changed_release
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("push vote for modifier-changed release");
    changed_release
        .handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("bare Return presses focused No");
    changed_release
        .handle_modifiers_changed(ModifiersState::CONTROL)
        .expect("change modifiers before Return up");
    changed_release
        .handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("modified Return up does not match the bare button binding");
    assert_eq!(changed_release.message_dialogs.len(), 1);
    assert!(changed_release.running_chat.is_none());

    let mut exclusive_top_scope = boxed_running_sandbox_app();
    exclusive_top_scope
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .expect("push ordinary lower A");
    let lower_layout = exclusive_top_scope
        .top_message_dialog_layout()
        .expect("ordinary lower A layout");
    exclusive_top_scope
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("push smaller exclusive top B");
    let exposed = PhysicalPosition::new(
        f64::from(lower_layout.bounds.x + 5),
        f64::from(lower_layout.bounds.y + 5),
    );
    exclusive_top_scope
        .handle_cursor_moved(exposed)
        .expect("hover exposed ordinary A");
    exclusive_top_scope
        .handle_mouse_button(ElementState::Pressed)
        .expect("activate ordinary A under exclusive B");
    exclusive_top_scope
        .handle_mouse_button(ElementState::Released)
        .expect("release ordinary A activation");
    assert_eq!(exclusive_top_scope.active_message_dialog_index(), Some(0));
    exclusive_top_scope
        .handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("top exclusive B supplies GUI scope to active A");
    exclusive_top_scope
        .handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("active ordinary A accepts Return under B's GUI scope");
    assert_eq!(exclusive_top_scope.message_dialogs.len(), 1);
    assert!(matches!(
        exclusive_top_scope.message_dialogs[0].continuation,
        MessageDialogContinuation::LeagueSurrender
    ));
    assert!(exclusive_top_scope.running_chat.is_none());

    let mut nonexclusive_top_scope = boxed_running_sandbox_app();
    nonexclusive_top_scope
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .expect("push exclusive lower A");
    let lower_layout = nonexclusive_top_scope
        .top_message_dialog_layout()
        .expect("exclusive lower A layout");
    nonexclusive_top_scope
        .push_message_dialog(small_notice(), MessageDialogContinuation::None)
        .expect("push smaller nonexclusive top B");
    let exposed = PhysicalPosition::new(
        f64::from(lower_layout.bounds.x + 5),
        f64::from(lower_layout.bounds.y + 5),
    );
    nonexclusive_top_scope
        .handle_cursor_moved(exposed)
        .expect("hover exposed lower vote A");
    nonexclusive_top_scope
        .handle_mouse_button(ElementState::Pressed)
        .expect("activate lower vote A");
    nonexclusive_top_scope
        .handle_mouse_button(ElementState::Released)
        .expect("release lower vote A activation");
    assert_eq!(
        nonexclusive_top_scope.active_message_dialog_index(),
        Some(0)
    );
    nonexclusive_top_scope
        .handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("nonexclusive top B leaves bare Return in global chat scope");
    assert_eq!(nonexclusive_top_scope.running_chat_text(), Some(""));
    assert_eq!(nonexclusive_top_scope.message_dialogs.len(), 2);

    for (key, modifiers, expected) in [
        (VirtualKeyCode::F2, ModifiersState::empty(), ""),
        (VirtualKeyCode::Enter, ModifiersState::SHIFT, "/team "),
        (VirtualKeyCode::Enter, ModifiersState::ALT, "\""),
    ] {
        let mut app = boxed_running_sandbox_app();
        app.push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .expect("push exclusive vote for global chat binding");
        app.handle_modifiers_changed(modifiers)
            .expect("set vote chat-open modifier");
        app.handle_key(key, ElementState::Pressed)
            .expect("unhandled global chat binding falls through exclusive vote");
        assert_eq!(app.running_chat_text(), Some(expected));
        assert_eq!(app.message_dialogs.len(), 1);
    }

    for (key, modifiers, expected) in [
        (VirtualKeyCode::F2, ModifiersState::empty(), ""),
        (VirtualKeyCode::Enter, ModifiersState::SHIFT, "/team "),
        (VirtualKeyCode::Enter, ModifiersState::ALT, "\""),
    ] {
        let mut app = boxed_running_sandbox_app();
        app.open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new("Unrelated")],
            GuiPoint::new(20.0, 20.0),
        )
        .expect("open unrelated context");
        app.handle_modifiers_changed(modifiers)
            .expect("set context chat-open modifier");
        app.handle_key(key, ElementState::Pressed)
            .expect("global chat binding opens underneath unrelated context");
        assert_eq!(app.running_chat_text(), Some(expected));
        assert!(app.context_menu.is_some());
    }
}

#[test]
fn running_chat_uses_compact_bottom_third_dialog_above_log_and_message_dialogs() {
    let mut app = new_classic_running_sandbox_app();
    install_message_fixture(&mut app);
    assert!(
        app.execute_message_control(message_control(
            MESSAGE_TYPE_NORMAL,
            7,
            -1,
            b"before chat",
            7,
        ))
        .displayed
    );
    let board_before = app.message_board_line();

    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("open running chat");
    let surface_width = app.graphics.surface().width() as i32;
    let surface_height = app.graphics.surface().height() as i32;
    let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
    let layout = app.game_option_input_layout().expect("chat layout");
    let controller = app.running_chat_controller().expect("chat controller");
    let edit_height = (fonts.text.line_height + 3).max(23);
    let width = surface_width * 4 / 5;
    let height = edit_height + 2;
    let label_width = fonts.text.measure("Chat:", true).0 + 4;

    assert!(controller.is_chat_layout());
    assert_eq!(controller.message(), "Chat:");
    assert_eq!(controller.caption(), "");
    assert_eq!(controller.icon(), InputDialogIcon::None);
    assert_eq!(
        controller.focused_control(),
        clonk_frontend::input_dialog::InputDialogControl::Edit
    );
    assert_eq!(layout.caption, None);
    assert_eq!(layout.close_button, None);
    assert_eq!(layout.bounds.w, width);
    assert_eq!(layout.bounds.h, height);
    assert_eq!(layout.bounds.x, (surface_width - width) / 2);
    assert_eq!(
        layout.bounds.y,
        (surface_height - height) / 2 + surface_height / 3
    );
    assert_eq!(layout.message.w, label_width);
    assert_eq!((layout.icon.w, layout.icon.h), (0, 0));
    assert_eq!((layout.ok_button.w, layout.ok_button.h), (0, 0));
    assert_eq!((layout.cancel_button.w, layout.cancel_button.h), (0, 0));

    for modifiers in [ModifiersState::SHIFT, ModifiersState::ALT] {
        app.handle_modifiers_changed(modifiers)
            .expect("hold modifier over the context-menu key");
        app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
            .expect("modified Apps is not the exact context binding");
        app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Released)
            .expect("release modified Apps probe");
        assert!(app.context_menu.is_none());
    }
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release context-menu modifier");

    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
        .expect("open context over empty chat");
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Released)
        .expect("release context-menu key");
    assert!(app.context_menu.is_some());
    app.handle_modifiers_changed(ModifiersState::SHIFT)
        .expect("hold Shift over empty chat context");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("global allies binding reopens empty chat through its context");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("release allies binding");
    assert!(app.context_menu.is_none());
    assert_eq!(app.running_chat_text(), Some("/team "));
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Shift");
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("close allies chat");

    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("reopen empty chat for context say binding");
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
        .expect("open context for say binding");
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Released)
        .expect("release context-menu key");
    app.handle_modifiers_changed(ModifiersState::ALT)
        .expect("hold Alt over empty chat context");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("global say binding reopens empty chat through its context");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("release say binding");
    assert!(app.context_menu.is_none());
    assert_eq!(app.running_chat_text(), Some("\""));
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Alt");
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("close say chat");

    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("reopen empty chat for context F2 binding");
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
        .expect("open context for F2 binding");
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Released)
        .expect("release context-menu key");
    app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
        .expect("global all-chat binding reopens empty chat through its context");
    assert!(app.context_menu.is_none());
    assert_eq!(app.running_chat_text(), Some(""));

    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("Escape closes chat without sending");
    assert!(app.running_chat.is_none());
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("reopen running chat");
    app.handle_modifiers_changed(ModifiersState::SHIFT)
        .expect("hold Shift");
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("Shift+Escape does not cancel the exact bare binding");
    assert_eq!(app.running_chat_text(), Some(""));
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("Shift+Return replaces empty chat with allies mode");
    assert_eq!(app.running_chat_text(), Some("/team "));
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Shift");
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("close allies chat");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("reopen ordinary running chat");
    app.handle_modifiers_changed(ModifiersState::ALT)
        .expect("hold Alt");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("Alt+Return replaces empty chat with say mode");
    assert_eq!(app.running_chat_text(), Some("\""));
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Alt");
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("close say chat");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("reopen ordinary chat for editing");

    for character in "alpha beta".chars() {
        app.handle_text_input(character).expect("type chat text");
    }
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    app.process_gamepad_event_batch([
        GamepadEvent::GuiButton {
            slot: GamepadSlot::new(0),
            class: GuiButtonClass::High,
            state: ElementState::Pressed,
        },
        GamepadEvent::Action {
            slot: GamepadSlot::new(0),
            action: GamepadActionType::MenuToggle,
            state: ElementState::Pressed,
        },
        GamepadEvent::Button {
            slot: GamepadSlot::new(0),
            button: LegacyGamepadButton::new(8),
            state: ElementState::Pressed,
        },
    ])
    .expect("chat owns the raw gamepad Select cluster");
    assert!(app.ingame_menu.is_none());
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    let caret_before_alt_navigation = app
        .running_chat_controller()
        .expect("chat controller before Alt navigation probe")
        .caret();
    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::CONTROL | ModifiersState::ALT,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
    ] {
        app.handle_modifiers_changed(modifiers)
            .expect("hold an Alt modifier mask over chat edit");
        for key in [VirtualKeyCode::ArrowLeft, VirtualKeyCode::Backspace] {
            app.handle_key(key, ElementState::Pressed)
                .expect("Alt navigation is not an Edit cursor binding");
            app.handle_key(key, ElementState::Released)
                .expect("release Alt navigation probe");
        }
        assert_eq!(app.running_chat_text(), Some("alpha beta"));
        assert_eq!(
            app.running_chat_controller()
                .expect("chat remains open after Alt navigation probe")
                .caret(),
            caret_before_alt_navigation
        );
    }
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Alt navigation modifier");
    app.handle_modifiers_changed(ModifiersState::SHIFT)
        .expect("hold Shift over nonempty chat");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("Shift+Return leaves nonempty chat unchanged");
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Shift");
    app.handle_modifiers_changed(ModifiersState::ALT)
        .expect("hold Alt over nonempty chat");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("Alt+Return leaves nonempty chat unchanged");
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Alt");
    assert_eq!(
        app.message_board_line(),
        board_before,
        "the message board remains a fading log instead of echoing edit text"
    );

    app.pressed_engine_keys.insert(VirtualKeyCode::KeyA);
    app.engine
        .player_mut(app.local_owner)
        .expect("local sandbox player")
        .control
        .pressed_coms = 1 << clonk_engine::COM_LEFT;
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
        .expect("open chat context before lower message");
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Released)
        .expect("release chat context key");
    assert!(app.context_menu.is_some());
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Notice",
            "The chat remains the higher input-z dialog.",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .expect("push message below chat");
    assert!(app.context_menu.is_some());
    assert!(app.pressed_engine_keys.contains(&VirtualKeyCode::KeyA));
    assert_ne!(
        app.engine
            .player(app.local_owner)
            .expect("local sandbox player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT),
        0
    );
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("close chat context above lower message");
    app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
        .expect("release context close key");
    app.handle_text_input('!')
        .expect("chat receives text above message dialog");
    assert_eq!(app.running_chat_text(), Some("alpha beta!"));
    assert_eq!(app.message_dialogs.len(), 1);
    let mut frame = vec![0_u8; (surface_width * surface_height * 4) as usize];
    app.render(&mut frame)
        .expect("render chat above the lower message dialog");
    assert!(frame.iter().any(|byte| *byte != 0));

    app.handle_modifiers_changed(ModifiersState::CONTROL | ModifiersState::SHIFT)
        .expect("hold Ctrl+Shift");
    app.handle_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed)
        .expect("select previous word in chat edit");
    assert!(app
        .running_chat_controller()
        .and_then(InputDialogController::selected_text)
        .is_some_and(|text| !text.is_empty()));
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release modifiers");
    let keyboard_selection = app
        .running_chat_controller()
        .expect("chat controller after keyboard selection")
        .selection();

    let start = PhysicalPosition::new(
        f64::from(layout.edit.x + 5),
        f64::from(layout.edit.y + layout.edit.h / 2),
    );
    let end = PhysicalPosition::new(
        f64::from(layout.edit.x + 35),
        f64::from(layout.edit.y + layout.edit.h / 2),
    );
    app.handle_cursor_moved(start)
        .expect("point into chat above message dialog");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("start chat selection");
    let selection_after_down = app
        .running_chat_controller()
        .expect("chat receives pointer down")
        .selection();
    assert!(selection_after_down.is_some_and(|(anchor, caret)| anchor == caret));
    assert_ne!(selection_after_down, keyboard_selection);
    app.handle_cursor_moved(end).expect("drag chat selection");
    app.handle_mouse_button(ElementState::Released)
        .expect("finish chat selection");
    assert!(app
        .running_chat_controller()
        .and_then(InputDialogController::selected_text)
        .is_some_and(|text| !text.is_empty()));
    app.handle_right_mouse_button(ElementState::Pressed)
        .expect("open chat edit context menu");
    assert!(app.context_menu.is_some());
    assert_eq!(app.message_dialogs.len(), 1);
    app.handle_right_mouse_button(ElementState::Released)
        .expect("release context-menu button");

    let text_before_context_key = app.running_chat_text().map(str::to_string);
    app.handle_key(VirtualKeyCode::ArrowUp, ElementState::Pressed)
        .expect("context menu outranks chat history");
    app.handle_key(VirtualKeyCode::ArrowUp, ElementState::Released)
        .expect("release context-menu navigation");
    assert!(app.game_option_input_consumed_keys.is_empty());
    assert_eq!(app.running_chat_text(), text_before_context_key.as_deref());
    assert_eq!(
        app.running_chat.as_ref().map(|chat| chat.history_index),
        Some(-1)
    );

    let caret_before_ctrl_left = app
        .running_chat_controller()
        .expect("chat remains under its context menu")
        .caret();
    app.handle_modifiers_changed(ModifiersState::CONTROL)
        .expect("hold Ctrl over chat context menu");
    app.handle_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed)
        .expect("context makes the parent chat edit inactive");
    assert_eq!(
        app.running_chat_controller()
            .expect("chat remains open")
            .caret(),
        caret_before_ctrl_left
    );
    assert!(app.context_menu.is_some());
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Ctrl");

    app.handle_modifiers_changed(ModifiersState::ALT)
        .expect("hold Alt over chat context menu");
    app.handle_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("global IRC chord replaces compact chat with the standalone dialog");
    assert!(app.external_irc_dialog_visible);
    assert!(app.running_chat.is_none());
    assert!(app.context_menu.is_none());
    app.handle_key(VirtualKeyCode::KeyC, ElementState::Released)
        .expect("consume global IRC chord release");
    app.handle_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("second global IRC chord closes the standalone dialog");
    app.handle_key(VirtualKeyCode::KeyC, ElementState::Released)
        .expect("consume closing IRC chord release");
    assert!(!app.external_irc_dialog_visible);
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("release Alt");
    assert!(app.game_option_input_dialog.is_none());
    assert!(app.context_menu.is_none());
    assert_eq!(app.message_dialogs.len(), 1);
    assert_eq!(app.message_board_line(), board_before);
}

#[test]
fn observer_menu_lists_players_and_live_previews_selection() {
    let mut app = new_state_only_running_sandbox_app();
    let first = app.local_owner;
    let first_info = app
        .engine
        .player(first)
        .expect("sandbox player")
        .player_info_id();
    let second = first + 1;
    let hidden = first + 2;
    let second_info = first_info + 10;
    let hidden_info = first_info + 20;
    app.engine
        .register_player(
            PlayerConfig::new(second, "Second visible").with_player_info_id(second_info),
        )
        .expect("register second visible observer target");
    app.engine
        .register_player(
            PlayerConfig::new(hidden, "Hidden target").with_player_info_id(hidden_info),
        )
        .expect("register invisible observer target");
    let info = |id, name: &[u8], flags| clonk_engine::ControlPlayerInfoEntry {
        id,
        name: LegacyCString::from_bytes(name.to_vec()).expect("valid player-info name"),
        flags,
        ..clonk_engine::ControlPlayerInfoEntry::default()
    };
    app.control_player_infos.replace_snapshot(
        hidden_info,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            players: vec![
                info(first_info, b"Player", 0),
                info(second_info, b"Second visible", 0),
                info(
                    hidden_info,
                    b"Hidden target",
                    clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                ),
            ],
            ..clonk_engine::PlayerInfoControlData::default()
        }],
    );

    app.clear_physical_viewport_states();
    let observer = app.ownerless_physical_viewport_state();
    let physical_identity = observer.physical_identity;
    app.physical_viewports.push(observer);
    app.physical_viewports_authoritative = true;
    assert!(app.set_physical_film_view(first));

    let open_observer_menu = |app: &mut GameApp| {
        app.ingame_menu.replace(
            OWNER_NONE,
            IngameMenuState::main_menu(
                &MainMenuConditions {
                    has_player: false,
                    player_count: 3,
                    ..MainMenuConditions::default()
                },
                &IngameMenuLabels::default(),
            ),
        );
        assert!(app
            .handle_menu_command(OWNER_NONE, ControlCommand::MenuEnter, CommandKind::Press,)
            .expect("open observer target page"));
    };
    open_observer_menu(&mut app);

    let menu = app.ingame_menu.get(OWNER_NONE).expect("observer menu");
    assert_eq!(menu.page(), ingame_menu::MenuPage::Observer);
    assert_eq!(
        menu.items()
            .iter()
            .map(|item| (item.caption.as_str(), item.action.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("free view", MenuAction::Observe(ObserverTarget::Free)),
            ("Player", MenuAction::Observe(ObserverTarget::Player(first)),),
            (
                "Second visible",
                MenuAction::Observe(ObserverTarget::Player(second)),
            ),
        ]
    );
    assert_eq!(menu.selection(), 1, "current followed player is selected");
    assert!(menu
        .items()
        .iter()
        .all(|item| item.caption != "Hidden target"));

    assert!(app
        .handle_menu_command(OWNER_NONE, ControlCommand::MenuDown, CommandKind::Press,)
        .expect("moving selection previews the next player"));
    assert_eq!(app.physical_viewports[0].displayed_player, second);
    assert_eq!(app.film_view_player, Some(second));
    assert!(app.set_physical_film_view(first));
    assert_eq!(
        app.ingame_menu
            .get(OWNER_NONE)
            .map(IngameMenuState::selection),
        Some(2),
        "camera perturbation does not change the highlighted row"
    );
    assert!(app
        .handle_menu_command(OWNER_NONE, ControlCommand::MenuEnter, CommandKind::Press,)
        .expect("Enter dispatches the highlighted player target"));
    assert!(!app.ingame_menu.contains(OWNER_NONE));
    assert_eq!(app.physical_viewports[0].displayed_player, second);

    open_observer_menu(&mut app);
    assert!(app
        .handle_menu_command(OWNER_NONE, ControlCommand::MenuDown, CommandKind::Press,)
        .expect("last player wraps to free view"));
    assert_eq!(app.physical_viewports[0].displayed_player, OWNER_NONE);
    assert!(app.set_physical_film_view(first));
    assert!(app
        .handle_menu_command(OWNER_NONE, ControlCommand::MenuEnter, CommandKind::Press,)
        .expect("Enter dispatches free view through the same path"));
    assert_eq!(app.physical_viewports[0].displayed_player, OWNER_NONE);
    assert_eq!(app.film_view_player, Some(OWNER_NONE));
    assert_eq!(
        app.physical_viewports[0].physical_identity,
        physical_identity
    );
    assert!(app.physical_viewports[0].is_no_owner_viewport);
}

#[test]
fn real_regicide_opens_initial_team_menu_and_hides_disabled_switch() {
    // Regicide's custom active Teams.txt leaves the initial user
    // teamless. C4Player::Execute opens C4MN_TeamSelection with both
    // ordered teams before the player's ScenarioInit can run
    // (C4Player.cpp:159-173,1762-1772; C4MainMenu.cpp:175-236).
    let mut app = real_installed_scenario_app("Knights.c4f/Regicide.c4s", "Regicide team chooser");
    wait_for_running(&mut app);

    assert!(
        !app.engine.team_configuration().allow_team_switch,
        "Regicide's parsed Teams.txt keeps mid-round switching disabled"
    );
    assert_eq!(
        app.engine
            .player(app.local_owner)
            .map(clonk_engine::Player::status),
        Some(PlayerStatus::TeamSelection)
    );
    let menu = app
        .ingame_menu
        .as_ref()
        .expect("team selection opens automatically");
    assert_eq!(menu.page(), ingame_menu::MenuPage::TeamSelection);
    assert_eq!(
        menu.items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>(),
        [MenuAction::SelectTeam(1), MenuAction::SelectTeam(2)]
    );

    let outcome = app
        .ingame_menu
        .as_mut()
        .expect("team menu remains open")
        .handle_command(ControlCommand::MenuEnter, CommandKind::Press)
        .expect("first team activates");
    app.execute_ingame_menu_outcome(outcome)
        .expect("team selection executes");

    let player = app
        .engine
        .player(app.local_owner)
        .expect("selected player remains registered");
    assert_eq!(player.status(), PlayerStatus::Active);
    assert_eq!(player.team(), Some(1));
    assert!(
        app.engine.crew_cursor(app.local_owner).is_some(),
        "Regicide selection must leave the player with usable crew"
    );
    assert!(app.ingame_menu.is_none());

    let owner = app.local_owner;
    app.activate_ingame_main_menu_for_player(owner)
        .expect("open post-selection main menu");
    assert!(!app
        .ingame_menu
        .as_ref()
        .expect("main menu")
        .items()
        .iter()
        .any(|item| item.action == MenuAction::ActivateTeamSelection));
}

#[test]
fn secondary_local_player_controls_own_initial_team_menu() {
    // C4Player stores one C4MainMenu per player. LocalPlayerControl looks
    // up the keyboard-set owner, converts through that player's menu, and
    // TeamSel dispatches DoTeamSelection on the menu's Player
    // (pristine 9ffa0a5d src/C4Player.h:85;
    // src/C4Game.cpp:3572-3624; src/C4MainMenu.cpp:899-908).
    let mut app = new_synthetic_running_sandbox_app();
    let primary = app.local_owner;
    let primary_before = app
        .engine
        .player(primary)
        .map(|player| (player.status(), player.team()))
        .expect("primary local player");
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "West", 0xff0000),
        clonk_engine::TeamInfo::new(2, "East", 0x0000ff),
    ]);
    let secondary = app
        .engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: "Secondary".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0000ff,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .expect("secondary waits for a team");
    app.engine.set_local_players([primary, secondary]);
    app.local_controls = LocalControlRegistry::default();
    app.local_controls.initialize(LocalControlInit {
        owner: primary,
        preferred_set: 0,
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.local_controls.initialize(LocalControlInit {
        owner: secondary,
        preferred_set: 1,
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.handle_key(VirtualKeyCode::KeyZ, ElementState::Pressed)
        .expect("primary holds left");

    app.open_initial_team_selection(secondary);
    assert_eq!(
        app.ingame_menu.as_ref().and_then(IngameMenuState::player),
        Some(secondary)
    );

    // Keyboard set 2 Key4 is Throw; an active C4MainMenu converts it to
    // MenuEnter and selects the first team.
    app.handle_key(VirtualKeyCode::Numpad4, ElementState::Pressed)
        .expect("secondary enters selected team");

    let secondary_player = app.engine.player(secondary).expect("secondary remains");
    assert_eq!(secondary_player.status(), PlayerStatus::Active);
    assert_eq!(secondary_player.team(), Some(1));
    assert!(
        app.engine.crew_cursor(secondary).is_some(),
        "team activation spawns the default native crew"
    );
    assert_eq!(
        app.engine
            .player(primary)
            .map(|player| (player.status(), player.team())),
        Some(primary_before),
        "secondary menu control must not mutate the primary player"
    );
    assert_ne!(
        app.engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == primary)
            .expect("primary snapshot")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT),
        0,
        "closing the secondary menu must clear only secondary controls"
    );
    assert!(app.ingame_menu.is_none());
}

#[test]
fn rules_menu_uses_engine_definition_description_as_tooltip() {
    let mut app = new_running_sandbox_app();
    let player = app.local_owner;
    let mut rule = Definition::from_script("IRUL", "Integrated Rule", "#strict 3\n")
        .expect("rule definition compiles");
    rule.set_category(C4D_RULE);
    rule.set_description(Some("Keep to the rule".to_string()));
    app.engine
        .register_definition(rule)
        .expect("rule definition registers");
    app.engine
        .spawn_object(clonk_engine::SpawnConfig::new("IRUL"))
        .expect("rule object spawns");
    app.snapshot = app.engine.snapshot();

    app.apply_ingame_menu_action_for_player(player, MenuAction::ActivateRules)
        .expect("open rules menu");
    let menu = app.ingame_menu.get(player).expect("rules menu opens");
    assert_eq!(menu.page(), ingame_menu::MenuPage::Rules);
    assert_eq!(
        menu.items()[0].info_caption.as_deref(),
        Some("Keep to the rule")
    );
}

#[test]
fn player_menu_title_close_routes_submenu_back_and_main_closed() {
    // Dialog's Ico_Close calls C4Menu::TryClose on left-up. Submenus run
    // their ActivateMenu:Main close command; the Main page stays closed.
    // Every C4MainMenu::OnClosed queues one synchronized ClearPressed
    // (C4GuiDialogs.cpp:386-425; C4MainMenu.cpp:313-329).
    let mut app = new_classic_running_sandbox_app();
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    app.open_ingame_menu().expect("open player menu");
    app.apply_ingame_menu_action(MenuAction::ActivateOptions)
        .expect("open Options submenu");

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.render(&mut frame).expect("establish local viewport");
    assert!(
        app.ingame_menu_gfx
            .as_ref()
            .is_some_and(|gfx| gfx.show_close_button),
        "the controlling mouse player's title renders its close button"
    );

    let close_rect = |app: &GameApp| {
        let player = app.local_owner;
        let area = app.graphics.viewport_rect(player).expect("local viewport");
        let fallback = app.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            app.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let gfx = IngameMenuGraphics {
            show_commands: app.display_flags.show_commands,
            show_close_button: true,
            ..IngameMenuGraphics::default()
        };
        app.ingame_menu
            .get(player)
            .expect("player menu")
            .close_button_rect(area, &font, &gfx)
    };
    let close_point = |app: &GameApp| {
        let close = close_rect(app);
        PhysicalPosition::new(
            f64::from(close.x) + f64::from(close.width) / 2.0,
            f64::from(close.y) + f64::from(close.height) / 2.0,
        )
    };

    app.handle_cursor_moved(close_point(&app))
        .expect("hover Options close");
    app.handle_right_mouse_button(ElementState::Pressed)
        .expect("right-down is consumed by close control");
    app.handle_right_mouse_button(ElementState::Released)
        .expect("right-up is consumed by close control");
    assert_eq!(
        app.ingame_menu
            .get(app.local_owner)
            .map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Options),
        "right-click must not invoke Dialog::OnUserClose"
    );
    assert!(commands.take_submitted_local().is_empty());

    let close = close_rect(&app);
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(close.x - 1),
        f64::from(close.y) + f64::from(close.height) / 2.0,
    ))
    .expect("hover title background beside close");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("title background mouse down");
    app.handle_cursor_moved(close_point(&app))
        .expect("move onto close after background down");
    app.handle_mouse_button(ElementState::Released)
        .expect("release over close without close capture");
    assert_eq!(
        app.ingame_menu
            .get(app.local_owner)
            .map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Options),
        "release-over must not close unless the close button retained left-down"
    );
    assert!(commands.take_submitted_local().is_empty());

    app.handle_cursor_moved(close_point(&app))
        .expect("re-hover the close button after title dragging moved the dialog");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("Options close mouse down");
    assert_eq!(
        app.ingame_menu
            .get(app.local_owner)
            .map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Options),
        "IconButton closes on button-up, not button-down"
    );
    assert!(commands.take_submitted_local().is_empty());
    app.handle_mouse_button(ElementState::Released)
        .expect("Options close mouse up");
    assert_eq!(
        app.ingame_menu
            .get(app.local_owner)
            .map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Main),
        "Options close command reactivates Main"
    );
    assert_eq!(
        commands.take_submitted_local(),
        vec![(app.local_owner, ControlEvent::ClearPressed, tick)]
    );

    app.handle_cursor_moved(close_point(&app))
        .expect("hover Main close");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("Main close mouse down");
    assert!(app.ingame_menu.contains(app.local_owner));
    assert!(commands.take_submitted_local().is_empty());
    app.handle_mouse_button(ElementState::Released)
        .expect("Main close mouse up");
    assert!(
        !app.ingame_menu.contains(app.local_owner),
        "Main has no close action and remains closed"
    );
    assert_eq!(
        commands.take_submitted_local(),
        vec![(app.local_owner, ControlEvent::ClearPressed, tick)]
    );
}

#[test]
fn player_menu_title_close_visibility_follows_mouse_owner_and_disable_mouse() {
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    app.open_ingame_menu().expect("open mouse owner's menu");
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.render(&mut frame).expect("render mouse owner's menu");
    assert!(app
        .ingame_menu_gfx
        .as_ref()
        .is_some_and(|gfx| gfx.show_close_button));

    app.ingame_menu.clear();
    app.ingame_menu.replace(
        owner + 1,
        IngameMenuState::main_menu(&MainMenuConditions::default(), &IngameMenuLabels::default()),
    );
    app.render(&mut frame)
        .expect("render menu not owned by the mouse player");
    assert!(
        !app.ingame_menu_gfx
            .as_ref()
            .is_some_and(|gfx| gfx.show_close_button),
        "a non-controlling player's C4Menu::HasMouse is false"
    );
    app.local_controls = LocalControlRegistry::default();
    app.local_controls.initialize(LocalControlInit {
        owner: owner + 1,
        preferred_set: 1,
        prefers_mouse: true,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.mouse_control = true;
    app.render(&mut frame)
        .expect("render reassigned mouse owner's menu");
    assert!(
        app.ingame_menu_gfx
            .as_ref()
            .is_some_and(|gfx| gfx.show_close_button),
        "close visibility follows the assigned mouse owner, not local_owner"
    );

    app.ingame_menu.clear();
    app.ingame_menu.replace(
        owner,
        IngameMenuState::main_menu(&MainMenuConditions::default(), &IngameMenuLabels::default()),
    );
    app.local_controls = LocalControlRegistry::default();
    let assignment = app.local_controls.initialize(LocalControlInit {
        owner,
        preferred_set: 0,
        prefers_mouse: true,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: true,
    });
    assert!(!assignment.mouse);
    app.mouse_control_allowed = false;
    app.mouse_control = false;
    app.render(&mut frame).expect("render DisableMouse menu");
    assert!(
        !app.ingame_menu_gfx
            .as_ref()
            .is_some_and(|gfx| gfx.show_close_button),
        "DisableMouse=1 suppresses the title close button"
    );

    let area = app.graphics.viewport_rect(owner).expect("local viewport");
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let close = app
        .ingame_menu
        .get(owner)
        .expect("disabled-mouse player menu")
        .close_button_rect(
            area,
            &font,
            &IngameMenuGraphics {
                show_commands: app.display_flags.show_commands,
                show_close_button: true,
                ..IngameMenuGraphics::default()
            },
        );
    let point = GuiPoint::new(
        (close.x + close.width as i32 / 2) as f32,
        (close.y + close.height as i32 / 2) as f32,
    );
    assert_eq!(
        app.ingame_menu_pointer_target(point),
        None,
        "DisableMouse leaves no invisible close hit target"
    );
}

#[test]
fn construction_menu_drag_uses_five_pixel_gate_and_focus_loss_clears_capture() {
    let (mut app, _owner, menu_point, _valid, _invalid, _world, _c4id) =
        construction_drag_fixture();
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(menu_point.x),
        f64::from(menu_point.y),
    ))
    .expect("move over constructable row");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("arm menu drag");

    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(menu_point.x + 4.0),
        f64::from(menu_point.y),
    ))
    .expect("move four pixels");
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Candidate { .. })
    ));
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(menu_point.x + MENU_DRAG_THRESHOLD),
        f64::from(menu_point.y),
    ))
    .expect("move exactly five pixels");
    assert!(app.ingame_construction_drag_active());
    assert!(app.mouse_state.is_none());
    assert!(app.ingame_right_mouse_state.is_none());
    assert!(app.ingame_custom_cursor_active());

    app.handle_focus_lost().expect("lose window focus");
    assert!(app.construction_menu_drag.is_none());
    assert!(!app.ingame_custom_cursor_active());
}

#[test]
fn subthreshold_constructable_menu_click_still_enters_item() {
    let (mut app, owner, menu_point, _valid, _invalid, _world, _c4id) = construction_drag_fixture();
    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();

    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(menu_point.x),
        f64::from(menu_point.y),
    ))
    .expect("hover constructable row");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("press constructable row");
    app.handle_mouse_button(ElementState::Released)
        .expect("release without crossing drag sensitivity");

    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    assert_eq!(
        controls,
        vec![(
            owner,
            ControlEvent::RawPlayerControl {
                command: clonk_engine::COM_MENU_ENTER,
                data: 0,
            },
            tick,
        )]
    );
    assert!(commands.is_empty());
    assert!(selections.is_empty());
    assert!(app.construction_menu_drag.is_none());
}

#[test]
fn invalid_construction_menu_drop_sends_nothing_and_clears_drag() {
    let (mut app, _owner, menu_point, valid_point, invalid_point, _world, _c4id) =
        construction_drag_fixture();
    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);

    begin_construction_drag(&mut app, menu_point, valid_point);
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            site_valid: true,
            ..
        })
    ));
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(invalid_point.x),
        f64::from(invalid_point.y),
    ))
    .expect("move to invalid site");
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            site_valid: false,
            ..
        })
    ));

    app.handle_mouse_button(ElementState::Released)
        .expect("release invalid construction drag");
    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    assert!(controls.is_empty());
    assert!(commands.is_empty());
    assert!(selections.is_empty());
    assert!(app.construction_menu_drag.is_none());
}

#[test]
fn construction_menu_drag_refreshes_site_check_without_pointer_motion() {
    let (mut app, _owner, menu_point, valid_point, _invalid, _world, _c4id) =
        construction_drag_fixture();
    begin_construction_drag(&mut app, menu_point, valid_point);
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            site_valid: true,
            ..
        })
    ));

    let mut filled = Landscape::flat(480, 0);
    filled.set_world_height(220);
    app.engine.set_landscape(filled);
    app.update()
        .expect("advance C4MouseControl construction check");
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            site_valid: false,
            ..
        })
    ));
}

#[test]
fn construction_menu_drag_reprojects_stationary_pointer_after_camera_motion() {
    let (mut app, owner, menu_point, valid_point, _invalid, _world, raw_c4id) =
        construction_drag_fixture();
    begin_construction_drag(&mut app, menu_point, valid_point);
    let before = match app.construction_menu_drag.as_ref() {
        Some(ConstructionMenuDrag::Active {
            pointer: Some(pointer),
            ..
        }) => ingame_pointer_world_pixel(*pointer),
        state => panic!("active drag pointer missing: {state:?}"),
    };
    let retained = app
        .ingame_viewport_mouse
        .expect("construction drag retains native VpX/VpY");
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            viewport_index: Some(index),
            ..
        }) if *index == retained.viewport_index
    ));

    app.engine
        .player_mut(owner)
        .expect("construction owner remains live")
        .set_view_offset(Vector2::new(7, 0));
    app.snapshot = app.engine.snapshot();
    let render_snapshot = app.snapshot.clone();
    let viewports =
        collect_viewport_inputs(&render_snapshot).expect("camera move keeps a local viewport");
    app.graphics.render_frame(&render_snapshot, &viewports);
    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.index == retained.viewport_index)
        .expect("retained physical viewport survives camera move");
    let screen = GuiPoint::new(
        viewport.rect.x.saturating_add(retained.position.x) as f32,
        viewport.rect.y.saturating_add(retained.position.y) as f32,
    );
    let expected_pointer = app
        .graphics
        .viewport_output_point_for_index(viewport.index, screen)
        .expect("stationary VpX/VpY reprojects");
    let expected_world = ingame_pointer_world_pixel(expected_pointer);
    assert_ne!(
        expected_world, before,
        "camera motion changes the drop site"
    );
    let mut shifted_ground = Landscape::flat(480, expected_world.y);
    shifted_ground.set_world_height(expected_world.y.saturating_add(40));
    app.engine.set_landscape(shifted_ground);
    assert!(
        app.engine.construction_site_valid("BLD1", expected_world),
        "reprojected camera site is buildable"
    );

    app.refresh_construction_menu_drag();
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            pointer: Some(pointer),
            site_valid: true,
            ..
        }) if ingame_pointer_world_pixel(*pointer) == expected_world
    ));

    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    app.handle_mouse_button(ElementState::Released)
        .expect("release stationary construction drag");
    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    assert!(controls.is_empty());
    assert_eq!(
        commands,
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::Construct as i32,
                x: expected_world.x,
                y: expected_world.y,
                target: 0,
                target2: 0,
                data: raw_c4id,
                add_mode: 1,
                by_client: 7,
            },
        )]
    );
    assert!(selections.is_empty());
}

#[test]
fn queued_cursor_menu_actions_cannot_fire_after_the_menu_closes() {
    // A converted menu action may execute after another synchronized
    // control has closed the menu. It must never reappear as the raw
    // Throw/Dig action that produced it (C4Object.cpp:3369-3371).
    for (definition_id, raw, callback) in [
        ("QTHR", ControlCommand::Throw, "throw_count"),
        ("QDIG", ControlCommand::Dig, "dig_count"),
    ] {
        let mut app = new_state_only_running_sandbox_app();
        let owner = app.local_owner;
        let script = r#"#strict
local throw_count, dig_count;
func ControlThrow() { throw_count = 1; return(1); }
func ControlDig() { dig_count = 1; return(1); }
"#;
        let mut probe = Definition::from_script(definition_id, "Menu race probe", script)
            .expect("probe definition compiles");
        probe.set_category(clonk_engine::CATEGORY_LIVING);
        probe.set_crew_member(true);
        app.engine
            .register_definition(probe)
            .expect("register menu race probe");
        let cursor = app
            .engine
            .spawn_object(
                SpawnConfig::new(definition_id)
                    .with_owner(owner)
                    .with_crew_member(true),
            )
            .expect("spawn menu race probe");
        let mut crew = app
            .engine
            .player(owner)
            .expect("sandbox player remains live")
            .crew()
            .to_vec();
        crew.push(cursor);
        app.engine
            .player_mut(owner)
            .expect("sandbox player remains live")
            .set_crew(crew);
        app.engine.clear_crew_selection(owner);
        app.engine
            .select_crew(owner, [cursor])
            .expect("select menu race probe");
        app.engine
            .set_crew_cursor(owner, Some(cursor))
            .expect("make menu race probe the cursor");
        install_test_cursor_menu(&mut app, cursor, two_item_script_menu(cursor));

        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        app.dispatch_control_event_for_local_player(
            owner,
            ControlEvent::Command {
                command: raw,
                kind: CommandKind::Press,
            },
        )
        .expect("queue cursor-menu action");
        let (_, converted, tick) = commands
            .take_submitted_local()
            .pop()
            .expect("converted control was queued");
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(None),
                    ..ObjectUpdate::default()
                },
            )
            .expect("close menu before control execution");
        app.apply_ready_controls(
            tick,
            vec![NetworkControl::Player {
                owner,
                event: converted,
            }],
        )
        .expect("execute converted control after close");
        let cursor = app
            .engine
            .object_snapshot(cursor)
            .expect("menu race probe survives");
        for name in ["throw_count", "dig_count"] {
            assert!(
                cursor
                    .local_vars
                    .get(name)
                    .is_none_or(|value| value == &Value::Nil),
                "converted {raw:?} must leave {name} unset"
            );
        }

        // Prove the fixture would catch the old raw packet: a second,
        // deliberately unconverted press reaches the corresponding
        // ControlThrow/ControlDig callback immediately.
        app.apply_ready_controls(
            tick.saturating_add(1),
            vec![NetworkControl::Player {
                owner,
                event: ControlEvent::Command {
                    command: raw,
                    kind: CommandKind::Press,
                },
            }],
        )
        .expect("execute deliberate raw gameplay control");
        assert_eq!(
            app.engine
                .object_snapshot(cursor.id)
                .expect("menu race probe survives raw control")
                .local_vars
                .get(callback),
            Some(&Value::Int(1)),
            "the fixture must observe an unconverted {raw:?} action"
        );
    }
}

#[test]
fn engine_script_menu_is_visible_and_consumes_raw_player_controls() {
    // C4Viewport draws the cursor object's menu (C4Viewport.cpp:
    // 983-995), while C4Player::InCom converts raw controls before
    // gameplay (C4Player.cpp:1502-1513). This is the app half of the
    // mandatory Dragon Rock difficulty/type menu path.
    clonk_logging::init();
    let mut app = new_classic_running_sandbox_app();
    let cursor = app
        .engine
        .crew_cursor(app.local_owner)
        .expect("sandbox cursor");
    let menu = two_item_script_menu(cursor);

    let mut baseline = vec![0u8; 320 * 200 * 4];
    app.render(&mut baseline).expect("baseline render");
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .expect("install script menu");
    let mut with_menu = vec![0u8; 320 * 200 * 4];
    app.render(&mut with_menu).expect("menu render");
    assert_ne!(
        with_menu, baseline,
        "an engine-created script menu must be visible"
    );
    let mut before_tooltip = with_menu.clone();
    for _ in 1..89 {
        app.render(&mut before_tooltip).expect("pre-tooltip render");
    }
    let mut with_tooltip = vec![0u8; 320 * 200 * 4];
    app.render(&mut with_tooltip).expect("90th menu render");
    assert_ne!(
        with_tooltip, before_tooltip,
        "C4MN_InfoCaption_Delay shows the tooltip on draw 90"
    );

    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .expect("right press");
    app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
        .expect("right release");
    let menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor exists")
        .expect("menu open");
    assert_eq!(menu.selection, 1, "release must not navigate twice");
    assert_eq!(
        app.engine
            .object_snapshot(cursor)
            .expect("cursor snapshot")
            .command_direction,
        CommandDirection::Stop,
        "menu navigation must not steer the crew"
    );

    app.dispatch_control_event(ControlEvent::Command {
        command: ControlCommand::Throw,
        kind: CommandKind::Press,
    })
    .expect("enter press");
    app.dispatch_control_event(ControlEvent::Command {
        command: ControlCommand::Throw,
        kind: CommandKind::Release,
    })
    .expect("enter release");
    assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));
}

#[test]
fn first_local_menu_press_reveals_progressive_text_before_navigation() {
    // C4Game::LocalPlayerControl performs the asynchronous ConvertCom
    // pass before offline dispatch/network submission. Only this local
    // raw press may become COM_MenuShowText; synchronized controls must
    // not recalculate the choice from client-specific text progress.
    let mut app = new_state_only_running_sandbox_app();
    let cursor = app
        .engine
        .crew_cursor(app.local_owner)
        .expect("sandbox cursor");
    let mut menu = two_item_script_menu(cursor);
    menu.text_progressing = true;
    for item in &mut menu.items {
        item.text_display_progress = 0;
    }
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .expect("install progressive script menu");

    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .expect("first right press reveals text");
    let menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor exists")
        .expect("menu stays open");
    assert_eq!(menu.selection, 0, "reveal must not navigate");
    assert!(!menu.text_progressing);
    assert!(menu
        .items
        .iter()
        .all(|item| item.text_display_progress == -1));

    app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
        .expect("right release");
    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .expect("second right press navigates");
    assert_eq!(
        app.engine
            .debug_object_menu(cursor.as_u64())
            .expect("cursor exists")
            .expect("menu stays open")
            .selection,
        1
    );
}

#[test]
fn normal_menu_render_rejects_an_unresolved_non_textspec_item_picture() {
    let mut app = new_classic_running_sandbox_app();
    let cursor = app
        .engine
        .crew_cursor(app.local_owner)
        .expect("sandbox cursor");
    let mut menu = two_item_script_menu(cursor);
    menu.style = 0;
    menu.items[0].item_id = "MISS".to_string();
    menu.items[0].image = clonk_engine::ObjectMenuImage::Definition;
    menu.items[0].presentation_definition_id = Some("MISS".to_string());
    assert!(
        object_menu_item_picture(
            &app.engine,
            &app.snapshot,
            &menu.items[0],
            0,
            &HudGraphics::default(),
            menu.style,
        )
        .is_none(),
        "fixture must exercise the unresolved non-TextSpec branch"
    );
    install_test_cursor_menu(&mut app, cursor, menu);

    let mut frame = vec![0_u8; app.graphics.surface().pixels().len()];
    let error = app
        .render(&mut frame)
        .expect_err("Normal menu must fail closed on an unresolved definition image");
    assert!(
        error
            .to_string()
            .contains("unresolved classic menu image at item 0"),
        "unexpected error: {error:#}"
    );
    assert!(
        error.to_string().contains("Definition"),
        "unexpected recipe: {error:#}"
    );
}

#[test]
fn engine_dialog_menu_renders_classic_style_instead_of_fallback() {
    let mut app = new_classic_running_sandbox_app();
    let cursor = app
        .engine
        .crew_cursor(app.local_owner)
        .expect("sandbox cursor");
    let mut menu = two_item_script_menu(cursor);
    menu.caption.clear();
    menu.style = 3;
    menu.columns = 1;
    for item in &mut menu.items {
        item.image = clonk_engine::ObjectMenuImage::None;
    }
    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.render(&mut baseline).expect("baseline render");
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .expect("install dialog menu");
    let mut rendered = vec![0_u8; 320 * 200 * 4];
    app.render(&mut rendered).expect("classic Dialog render");
    assert_ne!(rendered, baseline);
}

#[test]
fn engine_context_menu_is_visible_and_navigable_through_the_app() {
    // C4Player::Execute installs C4MN_Context as style 1 on the cursor;
    // C4Viewport draws that engine-owned menu and C4Player::InCom routes
    // navigation to it before gameplay (C4Object.cpp:1961-1980,
    // 2044-2062; C4Viewport.cpp:983-995; C4Player.cpp:1502-1513).
    clonk_logging::init();
    let mut app = new_classic_running_sandbox_app();
    let cursor = app
        .engine
        .crew_cursor(app.local_owner)
        .expect("sandbox cursor");
    let mut menu = two_item_script_menu(cursor);
    menu.caption = "Hut".to_string();
    menu.identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
        .expect("integer menu identification deserializes");
    menu.style = 1;
    menu.permanent = true;
    menu.user_menu = false;
    menu.columns = 1;

    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.render(&mut baseline).expect("baseline render");
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .expect("install context menu");
    let mut with_menu = vec![0_u8; 320 * 200 * 4];
    app.render(&mut with_menu).expect("context render");
    assert_ne!(with_menu, baseline, "style-1 context menu must be visible");

    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .expect("right press");
    app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
        .expect("right release");
    let menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor exists")
        .expect("context remains open");
    assert_eq!(menu.selection, 1);
    let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
        .expect("integer menu identification deserializes");
    assert_eq!(menu.identification, context_identification);
}

#[test]
fn engine_info_menu_renders_the_classic_style_instead_of_a_fallback() {
    clonk_logging::init();
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app
        .engine
        .crew_cursor(app.local_owner)
        .expect("sandbox cursor");
    let mut menu = two_item_script_menu(cursor);
    menu.caption = "Information".to_string();
    menu.style = 2;
    menu.columns = 1;
    menu.items.truncate(1);
    menu.items[0].caption = "Hidden caption".to_string();
    menu.items[0].info_caption = "<c 00ff00>Classic wrapped information</c>".to_string();
    menu.items[0].command.clear();
    menu.items[0].command2.clear();
    menu.items[0].selectable = false;
    menu.items[0].picture_object = Some(cursor);
    menu.selection = -1;
    menu.user_menu = false;

    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.render(&mut baseline).expect("baseline render");
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .expect("install Info menu");
    let mut with_menu = vec![0_u8; 320 * 200 * 4];
    app.render(&mut with_menu)
        .expect("classic style-2 Info menu renders");
    assert_ne!(with_menu, baseline);
    let initial_location = app
        .script_menu_presentations
        .get(&owner)
        .and_then(|state| state.location)
        .expect("internal Info latches its target-relative location");

    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate::default().with_position(Vector2::new(280, 160)),
        )
        .expect("move Info target");
    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    app.render(&mut with_menu)
        .expect("render after target move");
    assert_eq!(
        app.script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location),
        Some(initial_location),
        "C4Menu::SetLocation is one-shot; the menu must not follow a moving target"
    );
}

#[test]
fn engine_script_menu_pointer_selects_enters_and_closes_like_cpp() {
    // C4MenuItem::MouseEnter selects a selectable item, left-up enters
    // it, and Dialog's Ico_Close queues COM_MenuClose
    // (C4Menu.cpp:213-242, 1237-1262; C4ObjectMenu.cpp:461-478).
    clonk_logging::init();
    let mut app = new_classic_running_sandbox_app();
    let cursor = app
        .engine
        .crew_cursor(app.local_owner)
        .expect("sandbox cursor");
    let menu = two_item_script_menu(cursor);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu.clone())),
                ..ObjectUpdate::default()
            },
        )
        .expect("install script menu");
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.render(&mut frame).expect("establish viewport layout");

    let (second_item, close_button) = {
        let fallback = app.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            app.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let area = app
            .graphics
            .viewport_rect(app.local_owner)
            .expect("local viewport");
        let layout = object_menu::engine_script_menu_layout(
            area,
            &font,
            &menu,
            app.display_flags.show_commands,
        );
        (
            layout.item_rect(1).expect("second item rect"),
            layout.close_button_rect(),
        )
    };
    let second_point = PhysicalPosition::new(
        f64::from(second_item.x) + 8.0,
        f64::from(second_item.y) + 8.0,
    );
    app.handle_cursor_moved(second_point)
        .expect("hover second item");
    assert_eq!(
        app.engine
            .debug_object_menu(cursor.as_u64())
            .expect("cursor")
            .expect("menu")
            .selection,
        1,
        "hover must select the item under the pointer"
    );
    app.handle_mouse_button(ElementState::Pressed)
        .expect("item mouse down");
    app.handle_mouse_button(ElementState::Released)
        .expect("item mouse up");
    assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));

    let mut right_menu = menu.clone();
    right_menu.items[1].command2 = "SetComDir(COMD_Right())".to_string();
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(right_menu)),
                ..ObjectUpdate::default()
            },
        )
        .expect("reinstall script menu for right enter");
    app.handle_cursor_moved(second_point)
        .expect("hover second item for right enter");
    app.handle_right_mouse_button(ElementState::Pressed)
        .expect("right item mouse down");
    app.handle_right_mouse_button(ElementState::Released)
        .expect("right item mouse up");
    assert_eq!(
        app.engine
            .object_snapshot(cursor)
            .expect("cursor survives right enter")
            .command_direction,
        CommandDirection::Right,
        "right-up must dispatch COM_MenuEnterAll and execute Command2"
    );
    assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));

    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .expect("reinstall script menu");
    let close_point = PhysicalPosition::new(
        f64::from(close_button.x) + 8.0,
        f64::from(close_button.y) + 8.0,
    );
    app.handle_cursor_moved(close_point)
        .expect("hover close button");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("close mouse down");
    app.handle_mouse_button(ElementState::Released)
        .expect("close mouse up");
    assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));
}

#[test]
fn l065_running_menu_wheels_are_pixel_persistent_and_never_reach_gameplay() {
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.crew_cursor(owner).expect("sandbox cursor");
    let menu = long_script_menu(cursor, 12);
    install_test_cursor_menu(&mut app, cursor, menu);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.render(&mut frame).expect("seed script presentation");
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);

    let (_, layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("script layout resources")
        .expect("open normal script menu");
    assert!(layout.max_scroll_y >= 60);
    let client_point = GuiPoint::new((layout.client.x + 4) as f32, (layout.client.y + 4) as f32);
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(client_point.x),
        f64::from(client_point.y),
    ))
    .expect("hover script ScrollWindow");
    let selection = app
        .engine
        .cursor_object_menu(owner)
        .expect("script menu open")
        .1
        .selection;
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
        .expect("script menu consumes wheel down");
    assert_eq!(
        app.script_menu_presentations
            .get(&owner)
            .expect("script presentation")
            .scroll_y,
        60
    );
    assert_eq!(
        app.engine
            .cursor_object_menu(owner)
            .expect("wheel leaves menu open")
            .1
            .selection,
        selection,
        "wheel must not move the synchronized menu selection"
    );
    assert!(commands.take_submitted_local().is_empty());
    app.render(&mut frame)
        .expect("render preserves wheel displacement");
    assert_eq!(
        app.script_menu_presentations
            .get(&owner)
            .expect("script presentation")
            .scroll_y,
        60,
        "redraw must not pin an unchanged selection back into view"
    );

    let (_, geometry) = app
        .script_menu_geometry_for_owner(owner)
        .expect("script geometry resources")
        .expect("script geometry");
    let title = geometry.title.expect("normal menu title");
    let title_point = GuiPoint::new((title.x + 24) as f32, (title.y + 5) as f32);
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(title_point.x),
        f64::from(title_point.y),
    ))
    .expect("hover script title");
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
        .expect("external dialog consumes title wheel");
    assert_eq!(
        app.script_menu_presentations
            .get(&owner)
            .expect("script presentation")
            .scroll_y,
        60,
        "only the ScrollWindow client scrolls"
    );
    assert!(commands.take_submitted_local().is_empty());

    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(None),
                ..ObjectUpdate::default()
            },
        )
        .expect("close script menu");
    app.script_menu_presentations.remove(&owner);
    let players = (0..12)
        .map(|index| NewPlayerEntry {
            file: format!("Player{index}.c4p"),
            name: format!("Player {index}"),
        })
        .collect::<Vec<_>>();
    app.ingame_menu.replace(
        owner,
        Some(IngameMenuState::new_player_menu(
            &players,
            &IngameMenuLabels::default(),
        )),
    );
    app.render(&mut frame).expect("render long player menu");
    let area = app.ingame_menu_area(owner).expect("player viewport");
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let gfx = IngameMenuGraphics {
        show_commands: app.display_flags.show_commands,
        show_close_button: true,
        ..IngameMenuGraphics::default()
    };
    let bounds = app
        .ingame_menu
        .get(owner)
        .expect("player menu")
        .bounds(area, &font, &gfx);
    let player_client = GuiPoint::new((bounds.x + 6) as f32, (bounds.y + 30) as f32);
    assert!(app
        .ingame_menu
        .get(owner)
        .expect("player menu")
        .client_contains(area, &font, &gfx, player_client));
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(player_client.x),
        f64::from(player_client.y),
    ))
    .expect("hover player-menu ScrollWindow");
    let selection = app.ingame_menu.get(owner).expect("player menu").selection();
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
        .expect("player menu consumes wheel down");
    let player_menu = app.ingame_menu.get(owner).expect("player menu");
    assert_eq!(player_menu.scroll_y(), 60);
    assert_eq!(player_menu.selection(), selection);
    assert!(commands.take_submitted_local().is_empty());
    app.render(&mut frame)
        .expect("player-menu redraw preserves wheel displacement");
    assert_eq!(app.ingame_menu.get(owner).unwrap().scroll_y(), 60);
}

#[test]
fn l065_script_menu_scroll_and_drag_state_is_per_viewport_owner() {
    let mut app = new_classic_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = primary + 1;
    let primary_cursor = app.engine.crew_cursor(primary).expect("primary cursor");
    let primary_state = app
        .engine
        .object_snapshot(primary_cursor)
        .expect("primary cursor state");

    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary"))
        .expect("register secondary player");
    let secondary_position = Vector2::new(
        primary_state.position.x.saturating_add(24),
        primary_state.position.y,
    );
    let secondary_cursor = app
        .engine
        .spawn_object(
            SpawnConfig::new(primary_state.definition_id)
                .with_position(secondary_position)
                .with_owner(secondary)
                .with_crew_member(true),
        )
        .expect("spawn secondary cursor");
    app.engine
        .select_crew(secondary, [secondary_cursor])
        .expect("select secondary cursor");
    app.engine
        .set_crew_cursor(secondary, Some(secondary_cursor))
        .expect("set secondary cursor");
    app.engine
        .replace_player_viewports(
            secondary,
            vec![clonk_engine::PlayerViewport::new(secondary_position)
                .with_focus(Some(secondary_cursor))],
        )
        .expect("set secondary viewport");
    app.engine.set_local_players([primary, secondary]);
    app.local_controls = LocalControlRegistry::default();
    for (owner, preferred_set, prefers_mouse) in [(primary, 0, false), (secondary, 1, true)] {
        app.local_controls.initialize(LocalControlInit {
            owner,
            preferred_set,
            prefers_mouse,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
    }
    app.mouse_control = true;
    install_test_cursor_menu(
        &mut app,
        primary_cursor,
        long_script_menu(primary_cursor, 12),
    );
    install_test_cursor_menu(
        &mut app,
        secondary_cursor,
        long_script_menu(secondary_cursor, 12),
    );
    app.snapshot = app.engine.snapshot();

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.render(&mut frame)
        .expect("render both viewport-owned script menus");
    assert!(app.script_menu_presentations.contains_key(&primary));
    assert!(app.script_menu_presentations.contains_key(&secondary));

    let (_, secondary_layout) = app
        .script_menu_layout_for_owner(secondary, false)
        .expect("secondary layout resources")
        .expect("secondary script menu");
    assert!(secondary_layout.max_scroll_y >= 60);
    let client = PhysicalPosition::new(
        f64::from(secondary_layout.client.x + 4),
        f64::from(secondary_layout.client.y + 4),
    );
    app.handle_cursor_moved(client)
        .expect("hover secondary ScrollWindow");
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
        .expect("scroll secondary script menu");
    assert_eq!(app.script_menu_presentations[&primary].scroll_y, 0);
    assert_eq!(app.script_menu_presentations[&secondary].scroll_y, 60);
    app.render(&mut frame)
        .expect("retain independent viewport scroll state");
    assert_eq!(app.script_menu_presentations[&primary].scroll_y, 0);
    assert_eq!(app.script_menu_presentations[&secondary].scroll_y, 60);

    let (_, geometry) = app
        .script_menu_geometry_for_owner(secondary)
        .expect("secondary geometry resources")
        .expect("secondary geometry");
    let title = geometry.title.expect("secondary wooden title");
    let start = PhysicalPosition::new(f64::from(title.x + 3), f64::from(title.y + 5));
    app.handle_cursor_moved(start)
        .expect("hover secondary title");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("capture secondary title");
    let destination = PhysicalPosition::new(start.x + 11.0, start.y + 7.0);
    app.handle_cursor_moved(destination)
        .expect("drag secondary title");
    app.handle_mouse_button(ElementState::Released)
        .expect("release secondary title");
    assert_eq!(app.script_menu_presentations[&primary].location, None);
    assert_eq!(
        app.script_menu_presentations[&secondary].location,
        Some((geometry.bounds.x + 11, geometry.bounds.y + 7)),
    );
}

#[test]
fn runtime_music_flash_recurses_through_every_player_and_engine_menu_screen() {
    let every_player_menu_page = || {
        let entry = GoalRuleEntry {
            definition_id: "CLNK".to_string(),
            name: "Entry".to_string(),
            description: None,
            fulfilled: false,
        };
        vec![
            IngameMenuState::main_menu(
                &MainMenuConditions::default(),
                &IngameMenuLabels::default(),
            )
            .expect("default player main menu"),
            IngameMenuState::hostility_menu(&[], &IngameMenuLabels::default()),
            IngameMenuState::observer_menu(&[], ObserverTarget::Free, &IngameMenuLabels::default()),
            IngameMenuState::team_selection_menu(
                &[TeamSelectionEntry {
                    id: 1,
                    caption: "Team".to_string(),
                    icon_spec: None,
                    color: 0,
                    has_participants: false,
                }],
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::goals_menu(std::slice::from_ref(&entry), &IngameMenuLabels::default()),
            IngameMenuState::rules_menu(std::slice::from_ref(&entry), &IngameMenuLabels::default()),
            IngameMenuState::new_player_menu(
                &[ingame_menu::NewPlayerEntry {
                    file: "Player.c4p".to_string(),
                    name: "Player".to_string(),
                }],
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::savegame_menu(
                &[SaveSlotState { free: true }; 10],
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::options_menu(
                &OptionFlags {
                    sound: true,
                    music: true,
                    mouse_shown: true,
                    mouse: true,
                },
                0,
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::display_menu(
                &DisplayFlags::default(),
                0,
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::surrender_menu(&IngameMenuLabels::default()),
            IngameMenuState::client_disconnect_menu(&IngameMenuLabels::default()),
            IngameMenuState::host_disconnect_menu(
                &[HostDisconnectClientEntry {
                    client_id: 0,
                    caption: "Host (Host)".to_string(),
                    activated: true,
                }],
                &IngameMenuLabels::default(),
            ),
        ]
    };
    let default_pages = every_player_menu_page();
    let rebound_pages = every_player_menu_page();
    let sound_pages = every_player_menu_page();
    assert_eq!(default_pages.len(), 13);
    let page_index = |page: ingame_menu::MenuPage| match page {
        ingame_menu::MenuPage::Main => 0,
        ingame_menu::MenuPage::Hostility => 1,
        ingame_menu::MenuPage::Observer => 2,
        ingame_menu::MenuPage::TeamSelection => 3,
        ingame_menu::MenuPage::Goals => 4,
        ingame_menu::MenuPage::Rules => 5,
        ingame_menu::MenuPage::NewPlayer => 6,
        ingame_menu::MenuPage::Savegame => 7,
        ingame_menu::MenuPage::Options => 8,
        ingame_menu::MenuPage::Display => 9,
        ingame_menu::MenuPage::Surrender => 10,
        ingame_menu::MenuPage::ClientDisconnect => 11,
        ingame_menu::MenuPage::HostDisconnect => 12,
    };
    let test_music_bytes = silent_pcm_wav(10);
    let load_test_music = |app: &GameApp| {
        app.audio
            .as_ref()
            .expect("test audio")
            .system
            .load_music(&test_music_bytes)
            .expect("load lightweight runtime music fixture")
    };
    let prime_music_toggle_off = |app: &mut GameApp, music: &MusicHandle| {
        app.audio
            .as_ref()
            .expect("test audio")
            .system
            .play_music(music, true)
            .expect("start lightweight runtime music fixture");
        app.runtime_music_enabled = true;
    };

    let mut default_app = new_classic_lightweight_running_sandbox_app();
    let default_music = load_test_music(&default_app);
    let mut rebound_app = new_classic_lightweight_running_sandbox_app();
    rebound_app
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
    rebound_app
        .engine
        .player_mut(rebound_app.local_owner)
        .expect("local player")
        .control
        .control_style = true;
    let mut sound_app = new_classic_lightweight_running_sandbox_app();
    let mut covered = [false; 13];
    for ((default_menu, rebound_menu), sound_menu) in default_pages
        .into_iter()
        .zip(rebound_pages)
        .zip(sound_pages)
    {
        let page = default_menu.page();
        covered[page_index(page)] = true;
        assert_eq!(rebound_menu.page(), page);
        assert_eq!(sound_menu.page(), page);

        default_app
            .ingame_menu
            .replace(default_app.local_owner, Some(default_menu));
        prime_music_toggle_off(&mut default_app, &default_music);
        default_app
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("music producer reaches every player-menu page");
        let draws_before = default_app
            .runtime_flash_message
            .as_ref()
            .expect("localized flash")
            .remaining_draws;
        let mut frame = vec![0_u8; 320 * 200 * 4];
        default_app
            .render(&mut frame)
            .unwrap_or_else(|error| panic!("render flash over {page:?}: {error:#}"));
        assert_eq!(
            default_app
                .runtime_flash_message
                .as_ref()
                .expect("music text lasts more than one draw")
                .remaining_draws,
            draws_before - 1,
            "page {page:?}"
        );
        assert_eq!(
            default_app.ingame_menu.as_ref().map(IngameMenuState::page),
            Some(page)
        );
        default_app
            .handle_key(VirtualKeyCode::F3, ElementState::Released)
            .expect("release music producer");

        rebound_app
            .ingame_menu
            .replace(rebound_app.local_owner, Some(rebound_menu));
        rebound_app
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("player priority owns F3 on every page");
        assert!(rebound_app.runtime_flash_message.is_none(), "page {page:?}");
        assert!(rebound_app.ingame_menu.is_some(), "page {page:?}");
        rebound_app
            .handle_key(VirtualKeyCode::F3, ElementState::Released)
            .expect("release rebound player control");
        assert!(!rebound_app
            .pressed_engine_keys
            .contains(&VirtualKeyCode::F3));
        assert_eq!(
            rebound_app
                .engine
                .player(rebound_app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0
        );

        sound_app
            .ingame_menu
            .replace(sound_app.local_owner, Some(sound_menu));
        let sound_before = sound_app
            .audio
            .as_ref()
            .expect("test audio")
            .options
            .sound_enabled;
        sound_app
            .handle_modifiers_changed(ModifiersState::CONTROL)
            .expect("set Ctrl");
        sound_app
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("Ctrl+F3 reaches every player-menu page");
        assert_eq!(
            sound_app
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .sound_enabled,
            !sound_before,
            "page {page:?}"
        );
        assert!(sound_app.runtime_flash_message.is_none(), "page {page:?}");
        assert!(sound_app.ingame_menu.is_some(), "page {page:?}");
        sound_app
            .handle_key(VirtualKeyCode::F3, ElementState::Released)
            .expect("release sound producer");
        sound_app
            .handle_modifiers_changed(ModifiersState::empty())
            .expect("release Ctrl");
    }
    assert!(covered.into_iter().all(|covered| covered));

    let mut default_app = new_classic_lightweight_running_sandbox_app();
    let default_music = load_test_music(&default_app);
    let mut rebound = new_classic_lightweight_running_sandbox_app();
    rebound
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
    rebound
        .engine
        .player_mut(rebound.local_owner)
        .expect("local player")
        .control
        .control_style = true;
    let mut sound = new_classic_lightweight_running_sandbox_app();
    for style in 0..=3 {
        for text_progressing in [false, true] {
            let install_menu = |app: &mut GameApp| {
                let cursor = app
                    .engine
                    .crew_cursor(app.local_owner)
                    .expect("sandbox cursor");
                let mut menu = two_item_script_menu(cursor);
                menu.style = style;
                menu.text_progressing = text_progressing;
                app.engine
                    .apply_object_update(
                        cursor,
                        ObjectUpdate {
                            menu: Some(Some(menu)),
                            ..ObjectUpdate::default()
                        },
                    )
                    .expect("install engine menu style");
                app.snapshot = app.engine.snapshot();
            };

            install_menu(&mut default_app);
            prime_music_toggle_off(&mut default_app, &default_music);
            default_app
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("music producer reaches every engine menu style");
            let draws_before = default_app
                .runtime_flash_message
                .as_ref()
                .expect("localized flash")
                .remaining_draws;
            let mut frame = vec![0_u8; 320 * 200 * 4];
            default_app.render(&mut frame).unwrap_or_else(|error| {
                panic!("render style {style}, progress {text_progressing}: {error:#}")
            });
            assert_eq!(
                default_app
                    .runtime_flash_message
                    .as_ref()
                    .expect("music text lasts more than one draw")
                    .remaining_draws,
                draws_before - 1
            );
            assert!(default_app
                .engine
                .cursor_object_menu(default_app.local_owner)
                .is_some());
            default_app
                .handle_key(VirtualKeyCode::F3, ElementState::Released)
                .expect("release music producer");

            install_menu(&mut rebound);
            rebound
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("player F3 owns every engine menu style");
            assert!(rebound.runtime_flash_message.is_none());
            assert!(rebound
                .engine
                .cursor_object_menu(rebound.local_owner)
                .is_some());
            rebound
                .handle_key(VirtualKeyCode::F3, ElementState::Released)
                .expect("release rebound player control");
            assert!(!rebound.pressed_engine_keys.contains(&VirtualKeyCode::F3));
            assert_eq!(
                rebound
                    .engine
                    .player(rebound.local_owner)
                    .expect("local player")
                    .control
                    .pressed_coms
                    & (1 << clonk_engine::COM_LEFT),
                0
            );

            install_menu(&mut sound);
            let before = sound
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .sound_enabled;
            sound
                .handle_modifiers_changed(ModifiersState::CONTROL)
                .expect("set Ctrl");
            sound
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("Ctrl+F3 reaches every engine menu style");
            assert_eq!(
                sound
                    .audio
                    .as_ref()
                    .expect("test audio")
                    .options
                    .sound_enabled,
                !before
            );
            assert!(sound.runtime_flash_message.is_none());
            assert!(sound.engine.cursor_object_menu(sound.local_owner).is_some());
            sound
                .handle_key(VirtualKeyCode::F3, ElementState::Released)
                .expect("release sound producer");
            sound
                .handle_modifiers_changed(ModifiersState::empty())
                .expect("release Ctrl");
        }
    }
}

#[test]
fn runtime_flash_draws_above_f1_help_and_below_recursive_context_gui() {
    let mut help = new_classic_running_sandbox_app();
    help.status_text.clear();
    help.snapshot.hud.messages.clear();
    help.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("show help beneath flash");
    help.set_runtime_flash_message("AAAA", RuntimeHelpCharset::Windows1252)
        .expect("install flash above help");
    let flash = help.runtime_flash_message.take().expect("flash state");
    let mut help_only = vec![0_u8; 320 * 200 * 4];
    help.render(&mut help_only).expect("render help-only frame");
    let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
    expected.pixels_mut().copy_from_slice(&help_only);
    let gamma = help
        .graphics
        .active_gamma_ramp(&help.snapshot.environment.gamma);
    let fonts = help.assets.clonk_fonts.clone().expect("FontRegular");
    clonk_frontend::flash_message::render_flash_message(
        &mut expected,
        &fonts.text,
        &flash.text,
        flash.y,
        Some(&gamma),
        &MessageFontImages::default(),
    );
    help.runtime_flash_message = Some(flash);
    let mut actual = vec![0_u8; 320 * 200 * 4];
    help.render(&mut actual).expect("render help then flash");
    assert_eq!(actual, expected.pixels());

    let mut context = new_classic_running_sandbox_app();
    context.status_text.clear();
    context.snapshot.hud.messages.clear();
    context
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new("Root")
                .with_submenu(vec![ContextMenuEntry::new("Child")
                    .with_submenu(vec![ContextMenuEntry::new("Context above flash")])])],
            GuiPoint::new(120.0, 55.0),
        )
        .expect("open overlapping recursive context");
    for depth in 0..2 {
        context
            .handle_key(VirtualKeyCode::ArrowRight, ElementState::Pressed)
            .unwrap_or_else(|error| panic!("open context depth {depth}: {error}"));
        context
            .handle_key(VirtualKeyCode::ArrowRight, ElementState::Released)
            .unwrap_or_else(|error| panic!("release context depth {depth}: {error}"));
    }
    context
        .set_runtime_flash_message("AAAAAAAAAAAA", RuntimeHelpCharset::Windows1252)
        .expect("install flash beneath context");
    let menu = context.context_menu.take().expect("detach context");
    let flash = context.runtime_flash_message.clone().expect("flash state");
    let mut flash_only = vec![0_u8; 320 * 200 * 4];
    context.render(&mut flash_only).expect("render flash only");
    let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
    expected.pixels_mut().copy_from_slice(&flash_only);
    let gamma = context
        .graphics
        .active_gamma_ramp(&context.snapshot.environment.gamma);
    menu.render(&mut expected, Some(&gamma))
        .expect("compose topmost context");
    context.context_menu = Some(menu);
    context.runtime_flash_message = Some(flash);
    let mut actual = vec![0_u8; 320 * 200 * 4];
    context
        .render(&mut actual)
        .expect("render flash below recursive context");
    assert_eq!(actual, expected.pixels());
}

#[test]
fn runtime_f1_recurses_through_every_player_menu_page_and_priority_layer() {
    let every_player_menu_page = || {
        let entry = GoalRuleEntry {
            definition_id: "CLNK".to_string(),
            name: "Entry".to_string(),
            description: None,
            fulfilled: false,
        };
        vec![
            IngameMenuState::main_menu(
                &MainMenuConditions::default(),
                &IngameMenuLabels::default(),
            )
            .expect("default player main menu"),
            IngameMenuState::hostility_menu(&[], &IngameMenuLabels::default()),
            IngameMenuState::observer_menu(&[], ObserverTarget::Free, &IngameMenuLabels::default()),
            IngameMenuState::team_selection_menu(
                &[TeamSelectionEntry {
                    id: 1,
                    caption: "Team".to_string(),
                    icon_spec: None,
                    color: 0,
                    has_participants: false,
                }],
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::goals_menu(std::slice::from_ref(&entry), &IngameMenuLabels::default()),
            IngameMenuState::rules_menu(std::slice::from_ref(&entry), &IngameMenuLabels::default()),
            IngameMenuState::new_player_menu(
                &[ingame_menu::NewPlayerEntry {
                    file: "Player.c4p".to_string(),
                    name: "Player".to_string(),
                }],
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::savegame_menu(
                &[SaveSlotState { free: true }; 10],
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::options_menu(
                &OptionFlags {
                    sound: true,
                    music: true,
                    mouse_shown: true,
                    mouse: true,
                },
                0,
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::display_menu(
                &DisplayFlags::default(),
                0,
                &IngameMenuLabels::default(),
            ),
            IngameMenuState::surrender_menu(&IngameMenuLabels::default()),
            IngameMenuState::client_disconnect_menu(&IngameMenuLabels::default()),
            IngameMenuState::host_disconnect_menu(
                &[HostDisconnectClientEntry {
                    client_id: 0,
                    caption: "Host (Host)".to_string(),
                    activated: true,
                }],
                &IngameMenuLabels::default(),
            ),
        ]
    };
    let default_pages = every_player_menu_page();
    let rebound_pages = every_player_menu_page();
    assert_eq!(default_pages.len(), 13, "all native C4MainMenu page roots");
    let page_index = |page: ingame_menu::MenuPage| match page {
        ingame_menu::MenuPage::Main => 0,
        ingame_menu::MenuPage::Hostility => 1,
        ingame_menu::MenuPage::Observer => 2,
        ingame_menu::MenuPage::TeamSelection => 3,
        ingame_menu::MenuPage::Goals => 4,
        ingame_menu::MenuPage::Rules => 5,
        ingame_menu::MenuPage::NewPlayer => 6,
        ingame_menu::MenuPage::Savegame => 7,
        ingame_menu::MenuPage::Options => 8,
        ingame_menu::MenuPage::Display => 9,
        ingame_menu::MenuPage::Surrender => 10,
        ingame_menu::MenuPage::ClientDisconnect => 11,
        ingame_menu::MenuPage::HostDisconnect => 12,
    };
    let mut default_app = new_classic_running_sandbox_app();
    let mut rebound_app = new_running_sandbox_app();
    rebound_app
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    rebound_app
        .engine
        .player_mut(rebound_app.local_owner)
        .expect("local player")
        .control
        .control_style = true;
    let mut covered_pages = [false; 13];

    for (default_menu, rebound_menu) in default_pages.into_iter().zip(rebound_pages) {
        let page = default_menu.page();
        covered_pages[page_index(page)] = true;
        assert_eq!(rebound_menu.page(), page);

        default_app
            .ingame_menu
            .replace(default_app.local_owner, Some(default_menu));
        default_app
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("default F1 toggles above every player-menu page");
        assert!(default_app.runtime_help_visible, "page {page:?}");
        assert_eq!(
            default_app.ingame_menu.as_ref().map(IngameMenuState::page),
            Some(page)
        );
        default_app
            .handle_key(VirtualKeyCode::F1, ElementState::Released)
            .expect("release default help key");
        default_app
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("hide default help before the next page");
        default_app
            .handle_key(VirtualKeyCode::F1, ElementState::Released)
            .expect("release default help reset");
        assert!(!default_app.runtime_help_visible, "page {page:?}");

        rebound_app
            .ingame_menu
            .replace(rebound_app.local_owner, Some(rebound_menu));
        rebound_app
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("PRIO_PlrControl owns F1 across every player-menu page");
        assert!(!rebound_app.runtime_help_visible, "page {page:?}");
        assert!(rebound_app.ingame_menu.is_some(), "page {page:?}");
        rebound_app
            .handle_key(VirtualKeyCode::F1, ElementState::Released)
            .expect("release rebound player control");
        assert!(!rebound_app
            .pressed_engine_keys
            .contains(&VirtualKeyCode::F1));
        assert_eq!(
            rebound_app
                .engine
                .player(rebound_app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0
        );
    }
    assert!(covered_pages.into_iter().all(|covered| covered));

    let mut observer = new_classic_running_sandbox_app();
    observer
        .engine
        .remove_player(observer.local_owner)
        .expect("remove local player for ownerless observer menu");
    observer.snapshot = observer.engine.snapshot();
    observer.ingame_menu.replace(
        observer.local_owner,
        IngameMenuState::main_menu(
            &MainMenuConditions {
                has_player: false,
                player_count: 0,
                ..MainMenuConditions::default()
            },
            &IngameMenuLabels::default(),
        ),
    );
    observer
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    observer
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("ownerless observer menu suppresses player scope, not Generic help");
    assert!(observer.runtime_help_visible);
    assert!(observer.ingame_menu.is_some());

    let mut object = new_running_sandbox_app();
    assert!(object.open_object_menu().expect("open object menu"));
    object
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    object
        .engine
        .player_mut(object.local_owner)
        .expect("local player")
        .control
        .control_style = true;
    object
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("PRIO_PlrControl owns F1 over object menus");
    assert!(!object.runtime_help_visible);
    assert!(object.object_menu.is_some());

    let mut message = new_running_sandbox_app();
    message
        .push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Help",
                "Nonexclusive",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("push nonexclusive message");
    message
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    message
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("player priority remains above a nonexclusive message");
    assert!(!message.runtime_help_visible);
    assert_eq!(message.message_dialogs.len(), 1);

    let mut context = new_running_sandbox_app();
    context
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(24.0, 24.0),
        )
        .expect("open nonexclusive context");
    context
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    context
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("player priority remains above a context callback");
    assert!(!context.runtime_help_visible);
    assert!(context.context_menu.is_some());

    let board_script = r#"global func Initialize()
        {
            SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
        }"#;
    let mut default_scoreboard = new_classic_scoreboard_test_app(board_script);
    toggle_scoreboard(&mut default_scoreboard, ModifiersState::empty());
    let mut scoreboard_only = vec![0_u8; 320 * 200 * 4];
    default_scoreboard
        .render(&mut scoreboard_only)
        .expect("render scoreboard before help");
    default_scoreboard
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("default F1 toggles beneath scoreboard");
    let mut scoreboard_and_help = vec![0_u8; 320 * 200 * 4];
    default_scoreboard
        .render(&mut scoreboard_and_help)
        .expect("render help beneath scoreboard");
    assert!(default_scoreboard.runtime_help_visible);
    assert!(default_scoreboard.scoreboard_dialog.is_some());
    assert_ne!(scoreboard_and_help, scoreboard_only);

    let mut scoreboard = new_scoreboard_test_app(
        r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
            }"#,
    );
    toggle_scoreboard(&mut scoreboard, ModifiersState::empty());
    assert!(scoreboard.scoreboard_dialog.is_some());
    scoreboard
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    scoreboard
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("player priority remains above the nonexclusive scoreboard");
    assert!(!scoreboard.runtime_help_visible);
    assert!(scoreboard.scoreboard_dialog.is_some());

    let mut save_browser = new_classic_running_sandbox_app();
    save_browser
        .open_save_browser()
        .expect("open app save browser state");
    save_browser
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("default F1 toggles over save browser");
    assert!(save_browser.runtime_help_visible);
    assert!(save_browser.save_browser.is_some());

    let mut rebound_save_browser = new_running_sandbox_app();
    rebound_save_browser
        .open_save_browser()
        .expect("open rebound save browser state");
    rebound_save_browser
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    rebound_save_browser
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("player priority remains active over save browser");
    assert!(!rebound_save_browser.runtime_help_visible);
    assert!(rebound_save_browser.save_browser.is_some());

    let mut game_over = new_game_over_keyboard_app();
    game_over
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    game_over
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("exclusive evaluation suppresses player control but not Generic help");
    assert!(game_over.runtime_help_visible);
}

#[test]
fn running_context_menu_renders_above_runtime_f1_help() {
    let mut app = new_classic_running_sandbox_app();
    app.status_text.clear();
    app.snapshot.hud.messages.clear();
    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.render(&mut baseline).expect("render running baseline");
    app.open_context_menu_at(
        vec![ContextMenuEntry::<AppContextMenuCommand>::new(
            "Context above help",
        )],
        GuiPoint::new(120.0, 105.0),
    )
    .expect("open running context menu");
    let mut context_only = vec![0_u8; 320 * 200 * 4];
    app.render(&mut context_only)
        .expect("render visible running context");
    assert_ne!(context_only, baseline, "running context must draw pixels");

    app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect("toggle help beneath context");
    let context = app.context_menu.take().expect("detach running context");
    let mut help_only = vec![0_u8; 320 * 200 * 4];
    app.render(&mut help_only)
        .expect("render help without context");
    let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
    expected.pixels_mut().copy_from_slice(&help_only);
    let gamma = app
        .graphics
        .active_gamma_ramp(&app.snapshot.environment.gamma);
    context
        .render(&mut expected, Some(&gamma))
        .expect("compose expected topmost context");
    app.context_menu = Some(context);
    let mut help_and_context = vec![0_u8; 320 * 200 * 4];
    app.render(&mut help_and_context)
        .expect("render help below running context");
    assert_ne!(
        help_and_context, context_only,
        "help remains visible outside the panel"
    );
    assert_eq!(
        help_and_context,
        expected.pixels(),
        "running render must compose the context after F1 help"
    );
}

#[test]
fn runtime_f1_recurses_through_all_engine_menu_styles_and_progress_states() {
    let mut app = new_classic_running_sandbox_app();
    let mut rebound = new_classic_running_sandbox_app();
    rebound
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    rebound
        .engine
        .player_mut(rebound.local_owner)
        .expect("local rebound player")
        .control
        .control_style = true;
    let mut menu_only = vec![0_u8; 320 * 200 * 4];
    let mut menu_and_help = vec![0_u8; 320 * 200 * 4];
    for style in 0..=3 {
        for text_progressing in [false, true] {
            let cursor = app
                .engine
                .crew_cursor(app.local_owner)
                .expect("sandbox cursor");
            let mut menu = two_item_script_menu(cursor);
            menu.style = style;
            menu.text_progressing = text_progressing;
            app.engine
                .apply_object_update(
                    cursor,
                    ObjectUpdate {
                        menu: Some(Some(menu)),
                        ..ObjectUpdate::default()
                    },
                )
                .expect("install engine menu style");
            app.snapshot = app.engine.snapshot();
            menu_only.fill(0);
            app.render(&mut menu_only)
                .expect("render engine menu before F1");
            app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("default F1 toggles over engine menu");
            menu_and_help.fill(0);
            app.render(&mut menu_and_help)
                .expect("render F1 above engine menu");
            assert!(
                app.runtime_help_visible,
                "style {style}, progress {text_progressing}"
            );
            assert_ne!(menu_and_help, menu_only);
            assert!(app.engine.cursor_object_menu(app.local_owner).is_some());
            app.handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("release default help key");
            app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("hide default help before the next engine menu");
            app.handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("release default help reset");
            assert!(!app.runtime_help_visible);

            let rebound_cursor = rebound
                .engine
                .crew_cursor(rebound.local_owner)
                .expect("rebound sandbox cursor");
            let mut rebound_menu = two_item_script_menu(rebound_cursor);
            rebound_menu.style = style;
            rebound_menu.text_progressing = text_progressing;
            rebound
                .engine
                .apply_object_update(
                    rebound_cursor,
                    ObjectUpdate {
                        menu: Some(Some(rebound_menu)),
                        ..ObjectUpdate::default()
                    },
                )
                .expect("install rebound engine menu style");
            rebound.snapshot = rebound.engine.snapshot();
            rebound
                .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("player F1 owns every engine menu style");
            assert!(!rebound.runtime_help_visible);
            assert!(rebound
                .engine
                .cursor_object_menu(rebound.local_owner)
                .is_some());
            rebound
                .handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("release rebound player control");
            assert!(!rebound.pressed_engine_keys.contains(&VirtualKeyCode::F1));
            assert_eq!(
                rebound
                    .engine
                    .player(rebound.local_owner)
                    .expect("local rebound player")
                    .control
                    .pressed_coms
                    & (1 << clonk_engine::COM_LEFT),
                0
            );
        }
    }
}

#[test]
fn runtime_f4_gamepad_high_requires_active_dialog_and_other_input_reaches_gameplay() {
    let mut active = new_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut active, 0, 40, 4);
    route_primary_gamepad_to_local_owner(&mut active);
    active
        .handle_key(VirtualKeyCode::F4, ElementState::Pressed)
        .expect("open active runtime F4 dialog");
    assert!(active.runtime_client_list_strong_gamepad_callback_is_active());
    assert!(active.runtime_client_list_draw_active());

    active
        .process_gamepad_event_batch([
            GamepadEvent::Axis {
                slot: GamepadSlot::new(0),
                axis: LegacyGamepadAxis::new(0, true),
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Right,
                state: ElementState::Pressed,
            },
        ])
        .expect("normal F4 leaves player-control gamepad directions in base scope");
    let submitted = commands.take_submitted_local();
    assert_eq!(submitted.len(), 1);
    assert!(matches!(
        submitted[0].1,
        ControlEvent::Press(ControlButton::Right)
    ));

    active
        .process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::MenuToggle,
                state: ElementState::Pressed,
            },
        ])
        .expect("active F4 strong High callback owns its raw alias cluster");
    assert!(active.runtime_client_list.is_none());
    assert!(active.ingame_menu.is_none());
    assert!(commands.take_submitted_local().is_empty());

    let mut inactive = new_running_sandbox_app();
    configure_runtime_network_role(&mut inactive, RuntimeNetworkRole::Host);
    inactive
        .push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Notice",
                "The older F4 dialog remains inactive",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("show ordinary dialog before F4");
    inactive
        .handle_key(VirtualKeyCode::F4, ElementState::Pressed)
        .expect("insert F4 below the existing z+1 message");
    assert!(!inactive.runtime_client_list_strong_gamepad_callback_is_active());
    assert!(!inactive.runtime_client_list_draw_active());
    inactive
        .process_gamepad_event_batch([GamepadEvent::GuiButton {
            slot: GamepadSlot::new(0),
            class: GuiButtonClass::High,
            state: ElementState::Pressed,
        }])
        .expect("inactive F4 has no strong High callback");
    assert!(inactive.runtime_client_list.is_some());
    assert_eq!(inactive.message_dialogs.len(), 1);
}

#[test]
fn running_only_globals_are_excluded_from_menu_and_loading_modes() {
    let mut menu = new_menu_app(320, 200);
    for key in [
        VirtualKeyCode::F1,
        VirtualKeyCode::F4,
        VirtualKeyCode::Pause,
    ] {
        menu.handle_key(key, ElementState::Pressed)
            .expect("running-only global key is not registered in Menu mode");
        menu.handle_key(key, ElementState::Released)
            .expect("release remains outside the running-only global helper");
    }
    menu.handle_modifiers_changed(ModifiersState::ALT)
        .expect("set menu Alt modifier");
    menu.handle_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("runtime IRC frontend is not registered in Menu mode");
    menu.handle_key(VirtualKeyCode::KeyC, ElementState::Released)
        .expect("menu IRC chord release remains outside the runtime helper");

    let mut loading = new_running_sandbox_app();
    loading.mode = AppMode::Loading;
    for key in [
        VirtualKeyCode::F1,
        VirtualKeyCode::F4,
        VirtualKeyCode::Pause,
    ] {
        loading
            .handle_key(key, ElementState::Pressed)
            .expect("running-only global key is not registered in Loading mode");
        loading
            .handle_key(key, ElementState::Released)
            .expect("release remains outside the running-only global helper");
    }
    loading
        .handle_modifiers_changed(ModifiersState::ALT)
        .expect("set loading Alt modifier");
    loading
        .handle_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("runtime IRC frontend is not registered in Loading mode");
    loading
        .handle_key(VirtualKeyCode::KeyC, ElementState::Released)
        .expect("loading IRC chord release remains outside the runtime helper");
}

#[test]
fn l019_window_close_confirms_running_round_and_nonrunning_close_exits() {
    let mut app = new_running_sandbox_app();
    app.update().expect("advance round before declining close");
    let running_frame = app.engine.frame();
    let running_scenario = app
        .active_scenario
        .as_ref()
        .expect("active sandbox scenario")
        .identifier
        .clone();

    app.handle_window_close_requested();
    assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
        dialog.continuation,
        MessageDialogContinuation::AbortGame { .. }
    )));
    assert!(!app.take_exit_request());
    finish_abort_dialog(
        &mut app,
        clonk_frontend::message_dialog::MessageDialogResult::No,
    );
    assert!(matches!(app.mode, AppMode::Running));
    assert_eq!(app.engine.frame(), running_frame);
    assert_eq!(
        app.active_scenario
            .as_ref()
            .map(|scenario| scenario.identifier.as_str()),
        Some(running_scenario.as_str())
    );

    app.handle_window_close_requested();
    finish_abort_dialog(
        &mut app,
        clonk_frontend::message_dialog::MessageDialogResult::Yes,
    );
    assert!(matches!(app.mode, AppMode::Menu));
    assert!(app.active_scenario.is_none());
    assert!(
        !app.take_exit_request(),
        "Yes ends the round, not the process"
    );

    app.handle_window_close_requested();
    assert!(
            app.take_exit_request(),
            "the window-event footer turns this into ControlFlow::Exit so dirty display options persist"
        );

    let mut loading = new_running_sandbox_app();
    loading.mode = AppMode::Loading;
    loading.handle_window_close_requested();
    assert!(loading.take_exit_request());
    assert!(loading.ingame_menu.is_none());
    assert!(loading.message_dialogs.is_empty());
}

#[test]
fn l019_window_close_uses_observer_owner_and_never_exits_on_dialog_refusal() {
    let mut observer = new_running_sandbox_app();
    let removed_owner = observer.local_owner;
    observer
        .engine
        .remove_player(removed_owner)
        .expect("remove local player for passive observer");
    observer.engine.set_local_players([]);
    observer.local_controls = LocalControlRegistry::default();
    observer.snapshot = observer.engine.snapshot();
    observer.refresh_non_authoritative_physical_viewports();
    assert!(observer.primary_physical_viewport_is_no_owner());

    observer.handle_window_close_requested();
    observer.handle_window_close_requested();
    assert!(observer.ingame_menu.is_none());
    assert_eq!(observer.message_dialogs.len(), 1);
    assert!(matches!(
        observer.message_dialogs[0].continuation,
        MessageDialogContinuation::AbortGame { .. }
    ));
    assert!(!observer.take_exit_request());

    let mut game_over = new_game_over_keyboard_app();
    game_over.handle_window_close_requested();
    assert!(game_over.game_over_dialog.is_some());
    assert!(game_over.ingame_menu.is_none());
    assert!(game_over.message_dialogs.is_empty());
    assert!(!game_over.take_exit_request());
}

#[test]
fn l002_bare_escape_opens_abort_confirmation_without_exiting() {
    clonk_logging::init();
    let mut app = new_running_sandbox_app();
    app.status_text.clear();
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("bare Escape opens C4AbortGameDialog");

    assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
        dialog.continuation,
        MessageDialogContinuation::AbortGame { .. }
    )));
    assert!(app.object_menu.is_none());
    assert!(matches!(app.mode, AppMode::Running));
    assert!(!app.take_exit_request());
    assert!(app.status_text.is_empty());
    assert!(!app.show_abort_dialog(app.local_owner));
    assert_eq!(app.message_dialogs.len(), 1);
}

#[test]
fn abort_dialog_uses_stacked_halt_and_preserves_prior_pause() {
    let mut unpaused = new_running_sandbox_app();
    assert_eq!(unpaused.offline_halt_count, 0);
    assert!(unpaused.show_abort_dialog(unpaused.local_owner));
    assert_eq!(unpaused.offline_halt_count, 1);
    assert!(
        !unpaused.show_abort_dialog(unpaused.local_owner),
        "the singleton abort dialog cannot acquire a second halt lease"
    );
    assert_eq!(unpaused.offline_halt_count, 1);
    finish_abort_dialog(
        &mut unpaused,
        clonk_frontend::message_dialog::MessageDialogResult::No,
    );
    assert_eq!(unpaused.offline_halt_count, 0);

    let mut app = new_running_sandbox_app();
    app.set_runtime_pause(true);
    assert_eq!(app.offline_halt_count, 1);
    app.engine
        .player_mut(app.local_owner)
        .expect("local player")
        .control
        .pressed_coms = 1 << clonk_engine::COM_LEFT;
    let frozen_frame = app.engine.frame();

    assert!(app.show_abort_dialog(app.local_owner));
    assert_eq!(app.offline_halt_count, 2);
    assert!(app.runtime_halt_active());
    app.update().expect("stacked halt keeps the app loop live");
    assert_eq!(app.engine.frame(), frozen_frame);

    finish_abort_dialog(
        &mut app,
        clonk_frontend::message_dialog::MessageDialogResult::No,
    );
    assert_eq!(app.offline_halt_count, 1);
    assert!(app.runtime_halt_active(), "the prior pause remains owned");
    assert_eq!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms,
        0,
        "decline clears every local player's pressed commands"
    );

    assert!(app.show_abort_dialog(app.local_owner));
    assert_eq!(app.offline_halt_count, 2);
    let index = app.message_dialogs.len() - 1;
    app.remove_message_dialog_at(index)
        .expect("silent dialog removal releases its captured lease");
    assert_eq!(app.offline_halt_count, 1);
    app.set_runtime_pause(false);
    assert_eq!(app.offline_halt_count, 0);

    let mut network = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut network, 0, 0, 1);
    assert!(network.show_abort_dialog(network.local_owner));
    assert_eq!(network.offline_halt_count, 0);
    finish_abort_dialog(
        &mut network,
        clonk_frontend::message_dialog::MessageDialogResult::No,
    );
    assert_eq!(network.offline_halt_count, 0);
}
