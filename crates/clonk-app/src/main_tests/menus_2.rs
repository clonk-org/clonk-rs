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
fn default_z_dialog_order_tracks_show_raise_and_close() {
    let mut app = new_game_over_keyboard_app();
    assert_eq!(
        app.runtime_default_dialog_order_snapshot(),
        vec![RuntimeDefaultDialog::GameOver]
    );

    app.toggle_network_chart();
    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
    app.toggle_runtime_client_list().test_value();
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
    app.toggle_runtime_client_list().test_value();
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart));
    app.toggle_network_chart();
    assert!(app.runtime_default_dialog_order_snapshot().is_empty());
}

#[test]
fn non_left_runtime_dialog_hits_swallow_without_raising() {
    let mut app = new_game_over_keyboard_app();
    app.resize(1280, 720).test_value();
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
        .test_value();
    assert!(app.game_over_dialog_contains_point(game_over_only));
    assert!(!app.network_chart_contains_point(game_over_only));
    assert!(!app.game_over_pointer_route_hit(outside));
    let order = app.runtime_default_dialog_order_snapshot();
    app.running_pointer_position = Some(game_over_only);

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);
    app.test_right_button(ElementState::Pressed);
    app.test_right_button(ElementState::Released);
    app.handle_other_mouse_button(ElementState::Pressed)
        .test_value();
    app.handle_other_mouse_button(ElementState::Released)
        .test_value();
    assert_eq!(app.runtime_default_dialog_order_snapshot(), order);
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart));

    app.test_cursor(PhysicalPosition::new(
        f64::from(game_over_only.x),
        f64::from(game_over_only.y),
    ));
    app.handle_mouse_button_classified(ElementState::Pressed, false)
        .test_value();
    assert!(app.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver));
    app.handle_mouse_button_classified(ElementState::Released, false)
        .test_value();
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
        .test_value();
    let layout = f2.top_message_dialog_layout().test_value();
    let button = layout.buttons.first().test_value().rect;
    f2.test_cursor(PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    ));
    f2.test_left_button(ElementState::Pressed);
    assert!(f2.message_dialogs[0].state.has_pointer_capture());
    assert_eq!(f2.message_dialog_pointer_capture_index, Some(0));
    f2.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    assert_eq!(f2.running_chat_text(), Some(""));
    assert_eq!(f2.message_dialogs.len(), 1);
    assert!(f2.message_dialogs[0].state.has_pointer_capture());
    f2.test_left_button(ElementState::Released);
    assert!(f2.message_dialogs.is_empty());
    assert!(f2.running_chat_active());

    let mut focus_loss = boxed_running_sandbox_app();
    focus_loss
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let layout = focus_loss.top_message_dialog_layout().test_value();
    let button = layout.buttons.first().test_value().rect;
    focus_loss.test_cursor(PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    ));
    focus_loss.test_left_button(ElementState::Pressed);
    focus_loss.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    focus_loss.handle_focus_lost().test_value();
    assert!(!focus_loss.message_dialogs[0].state.has_pointer_capture());
    assert_eq!(focus_loss.message_dialog_pointer_capture_index, None);
    assert!(!focus_loss.primary_pointer_left_down);
    focus_loss.test_left_button(ElementState::Released);
    assert_eq!(focus_loss.message_dialogs.len(), 1);

    for (modifiers, expected) in [
        (ModifiersState::SHIFT, "/team "),
        (ModifiersState::ALT, "\""),
    ] {
        let mut app = boxed_running_sandbox_app();
        app.push_message_dialog(notice(), MessageDialogContinuation::None)
            .test_value();
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
        assert_eq!(app.running_chat_text(), Some(expected));
        assert_eq!(app.message_dialogs.len(), 1);
    }

    let mut bare_return = boxed_running_sandbox_app();
    bare_return
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    bare_return.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(bare_return.running_chat_text(), Some(""));
    assert_eq!(bare_return.message_dialogs.len(), 1);

    let lower_layout = bare_return.top_message_dialog_layout().test_value();
    let lower_point = PhysicalPosition::new(
        f64::from(lower_layout.bounds.x + 5),
        f64::from(lower_layout.bounds.y + 5),
    );
    bare_return.test_cursor(lower_point);
    bare_return.test_left_button(ElementState::Pressed);
    bare_return.test_left_button(ElementState::Released);
    assert!(!bare_return.running_chat_active());
    bare_return.test_text_input('x');
    assert_eq!(bare_return.running_chat_text(), Some(""));

    let chat_layout = bare_return.game_option_input_layout().test_value();
    let chat_point = PhysicalPosition::new(
        f64::from(chat_layout.edit.x + chat_layout.edit.w / 2),
        f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
    );
    bare_return.test_cursor(chat_point);
    bare_return.test_left_button(ElementState::Pressed);
    bare_return.test_left_button(ElementState::Released);
    assert!(bare_return.running_chat_active());
    bare_return.test_text_input('x');
    assert_eq!(bare_return.running_chat_text(), Some("x"));

    let mut inactive_return = boxed_running_sandbox_app();
    inactive_return.start_running_chat(RunningChatMode::All);
    inactive_return
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let lower_layout = inactive_return.top_message_dialog_layout().test_value();
    inactive_return.test_cursor(PhysicalPosition::new(
        f64::from(lower_layout.bounds.x + 5),
        f64::from(lower_layout.bounds.y + 5),
    ));
    inactive_return.test_left_button(ElementState::Pressed);
    inactive_return.test_left_button(ElementState::Released);
    inactive_return.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(inactive_return.message_dialogs.len(), 1);
    assert!(!inactive_return.running_chat_active());
    inactive_return.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert!(inactive_return.message_dialogs.is_empty());
    assert!(inactive_return.running_chat_active());

    let mut held_drag = boxed_running_sandbox_app();
    held_drag.start_running_chat(RunningChatMode::All);
    held_drag
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let lower_layout = held_drag.top_message_dialog_layout().test_value();
    let lower_button = lower_layout.buttons.first().test_value().rect;
    held_drag.test_cursor(PhysicalPosition::new(
        f64::from(lower_button.x + lower_button.w / 2),
        f64::from(lower_button.y + lower_button.h / 2),
    ));
    held_drag.test_left_button(ElementState::Pressed);
    assert!(!held_drag.running_chat_active());
    let chat_layout = held_drag.game_option_input_layout().test_value();
    held_drag.test_cursor(PhysicalPosition::new(
        f64::from(chat_layout.edit.x + chat_layout.edit.w / 2),
        f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
    ));
    assert!(held_drag.running_chat_active());
    held_drag.test_cursor(PhysicalPosition::new(
        f64::from(lower_button.x + lower_button.w / 2),
        f64::from(lower_button.y + lower_button.h / 2),
    ));
    held_drag.test_left_button(ElementState::Released);
    assert_eq!(held_drag.message_dialogs.len(), 1);

    let mut label_drag = boxed_running_sandbox_app();
    label_drag.start_running_chat(RunningChatMode::All);
    label_drag
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let chat_layout = label_drag.game_option_input_layout().test_value();
    let label_point = PhysicalPosition::new(
        f64::from(chat_layout.message.x + chat_layout.message.w / 2),
        f64::from(chat_layout.message.y + chat_layout.message.h / 2),
    );
    let message_layout = label_drag.top_message_dialog_layout().test_value();
    let lower_point = PhysicalPosition::new(
        f64::from(message_layout.bounds.x + 5),
        f64::from(message_layout.bounds.y + 5),
    );
    label_drag.test_cursor(label_point);
    label_drag.test_left_button(ElementState::Pressed);
    assert_eq!(label_drag.game_option_input_pointer_capture, None);
    assert!(label_drag.primary_pointer_left_down);
    label_drag.test_cursor(lower_point);
    assert!(!label_drag.running_chat_active());
    assert_eq!(label_drag.active_message_dialog_index(), Some(0));
    label_drag.test_left_button(ElementState::Released);
    assert!(!label_drag.primary_pointer_left_down);

    let mut touch_lower = boxed_running_sandbox_app();
    touch_lower.start_running_chat(RunningChatMode::All);
    touch_lower
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let message_layout = touch_lower.top_message_dialog_layout().test_value();
    let lower_touch = GuiPoint::new(
        (message_layout.bounds.x + 5) as f32,
        (message_layout.bounds.y + 5) as f32,
    );
    touch_lower.test_touch(TouchPhase::Started, lower_touch);
    assert!(!touch_lower.running_chat_active());
    assert_eq!(touch_lower.active_message_dialog_index(), Some(0));
    touch_lower.test_touch(TouchPhase::Ended, lower_touch);

    let mut release_hit = boxed_running_sandbox_app();
    release_hit.start_running_chat(RunningChatMode::All);
    release_hit
        .push_message_dialog(
            notice().with_checkbox("&Remember", false),
            MessageDialogContinuation::None,
        )
        .test_value();
    let message_layout = release_hit.top_message_dialog_layout().test_value();
    let checkbox = message_layout.checkbox.test_ref().square;
    let checkbox_point = PhysicalPosition::new(
        f64::from(checkbox.x + checkbox.w / 2),
        f64::from(checkbox.y + checkbox.h / 2),
    );
    let chat_layout = release_hit.game_option_input_layout().test_value();
    let edit_point = PhysicalPosition::new(
        f64::from(chat_layout.edit.x + 5),
        f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
    );
    release_hit.test_cursor(edit_point);
    release_hit.test_left_button(ElementState::Pressed);
    assert_eq!(
        release_hit.game_option_input_pointer_capture,
        Some(ContextMenuPointerButton::Left),
    );
    release_hit.test_cursor(checkbox_point);
    release_hit.test_left_button(ElementState::Released);
    assert_eq!(release_hit.game_option_input_pointer_capture, None);
    assert_eq!(
        release_hit.message_dialogs[0].state.checkbox_checked(),
        Some(true),
    );

    let mut close_active_chat = boxed_running_sandbox_app();
    close_active_chat.start_running_chat(RunningChatMode::All);
    close_active_chat
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let message_layout = close_active_chat.top_message_dialog_layout().test_value();
    let button = message_layout.buttons.first().test_value().rect;
    close_active_chat.test_cursor(PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    ));
    close_active_chat.test_left_button(ElementState::Pressed);
    let chat_layout = close_active_chat.game_option_input_layout().test_value();
    close_active_chat.test_cursor(PhysicalPosition::new(
        f64::from(chat_layout.edit.x + 5),
        f64::from(chat_layout.edit.y + chat_layout.edit.h / 2),
    ));
    assert!(close_active_chat.running_chat_active());
    assert_eq!(
        close_active_chat.message_dialog_pointer_capture_index,
        Some(0)
    );
    close_active_chat.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert!(close_active_chat.running_chat.is_none());
    assert_eq!(close_active_chat.message_dialog_pointer_capture_index, None);
    assert!(!close_active_chat.message_dialogs[0]
        .state
        .has_pointer_capture());

    let mut stacked_active = boxed_running_sandbox_app();
    stacked_active.start_running_chat(RunningChatMode::All);
    stacked_active
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let first_layout = stacked_active.top_message_dialog_layout().test_value();
    stacked_active.test_cursor(PhysicalPosition::new(
        f64::from(first_layout.bounds.x + 5),
        f64::from(first_layout.bounds.y + 5),
    ));
    stacked_active.test_left_button(ElementState::Pressed);
    stacked_active.test_left_button(ElementState::Released);
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
        .test_value();
    assert_eq!(stacked_active.active_message_dialog_index(), Some(0));
    stacked_active.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    stacked_active.test_key(VirtualKeyCode::Enter, ElementState::Released);
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
        .test_value();
    let layout = stacked_capture.top_message_dialog_layout().test_value();
    let button = layout.buttons.first().test_value().rect;
    let button_point = PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    );
    stacked_capture.test_cursor(button_point);
    stacked_capture.test_left_button(ElementState::Pressed);
    assert_eq!(
        stacked_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    stacked_capture
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .test_value();
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(0));
    let small_layout = stacked_capture.top_message_dialog_layout().test_value();
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

    stacked_capture.test_left_button(ElementState::Released);
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
        .test_value();
    let regular_layout = exposed_lower.top_message_dialog_layout().test_value();
    exposed_lower
        .push_message_dialog(small_vote(), MessageDialogContinuation::None)
        .test_value();
    let small_layout = exposed_lower.top_message_dialog_layout().test_value();
    let close = regular_layout.close_button.test_value();
    let exposed_point = PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    );
    assert!(!GameApp::point_in_message_dialog_bounds(
        GuiPoint::new(exposed_point.x as f32, exposed_point.y as f32),
        &small_layout,
    ));
    exposed_lower.test_cursor(exposed_point);
    exposed_lower.test_left_button(ElementState::Pressed);
    assert_eq!(exposed_lower.active_message_dialog_index(), Some(0));
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
    let top_point = PhysicalPosition::new(
        f64::from(small_layout.bounds.x + small_layout.bounds.w / 2),
        f64::from(small_layout.bounds.y + small_layout.bounds.h / 2),
    );
    exposed_lower.test_cursor(top_point);
    assert_eq!(exposed_lower.active_message_dialog_index(), Some(1));
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
    exposed_lower.test_cursor(exposed_point);
    assert_eq!(exposed_lower.active_message_dialog_index(), Some(1));
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, Some(0));
    exposed_lower.test_left_button(ElementState::Released);
    assert_eq!(exposed_lower.message_dialogs.len(), 2);
    assert_eq!(exposed_lower.message_dialog_pointer_capture_index, None);

    let mut inserted_capture = boxed_running_sandbox_app();
    inserted_capture
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let regular_layout = inserted_capture.top_message_dialog_layout().test_value();
    let close = regular_layout.close_button.test_value();
    let close_point = PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    );
    inserted_capture.test_cursor(close_point);
    inserted_capture.test_left_button(ElementState::Pressed);
    inserted_capture
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .test_value();
    assert_eq!(inserted_capture.active_message_dialog_index(), Some(1));
    assert_eq!(
        inserted_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    assert!(inserted_capture.message_dialogs[0]
        .state
        .has_pointer_capture());
    let small_layout = inserted_capture.top_message_dialog_layout().test_value();
    let top_point = PhysicalPosition::new(
        f64::from(small_layout.bounds.x + small_layout.bounds.w / 2),
        f64::from(small_layout.bounds.y + small_layout.bounds.h / 2),
    );
    inserted_capture.test_cursor(top_point);
    inserted_capture.test_left_button(ElementState::Released);
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
    inserted_capture.test_cursor(exposed_point);
    inserted_capture.test_left_button(ElementState::Pressed);
    assert_eq!(inserted_capture.active_message_dialog_index(), Some(0));
    inserted_capture.test_left_button(ElementState::Released);

    stacked_capture.remove_message_dialog_at(1).test_value();
    stacked_capture.test_cursor(button_point);
    stacked_capture.test_left_button(ElementState::Pressed);
    stacked_capture
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .test_value();
    stacked_capture.test_cursor(button_point);
    assert_eq!(
        stacked_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
    stacked_capture.test_cursor(a_only_point);
    assert_eq!(
        stacked_capture.message_dialog_pointer_capture_index,
        Some(0)
    );
    assert_eq!(stacked_capture.active_message_dialog_index(), Some(1));
    stacked_capture.test_left_button(ElementState::Released);
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
        .test_value();
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
        .test_value();
    vote_return.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert!(vote_return.running_chat.is_none());
    assert_eq!(vote_return.message_dialogs.len(), 1);
    vote_return.test_key(VirtualKeyCode::Enter, ElementState::Released);
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
            .test_value();
        app.test_modifiers(modifiers);
        app.test_key(key, ElementState::Pressed);
        app.test_key(key, ElementState::Released);
        assert_eq!(app.message_dialogs.len(), 1);
        assert!(app.running_chat.is_none());
    }

    let mut unmatched_vote_hotkey = boxed_classic_running_sandbox_app();
    unmatched_vote_hotkey
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .test_value();
    unmatched_vote_hotkey.test_modifiers(ModifiersState::ALT);
    unmatched_vote_hotkey.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    assert!(unmatched_vote_hotkey.external_irc_dialog_visible);
    unmatched_vote_hotkey.test_key(VirtualKeyCode::KeyC, ElementState::Released);
    assert_eq!(unmatched_vote_hotkey.message_dialogs.len(), 1);

    let mut handled_message_hotkey = boxed_running_sandbox_app();
    handled_message_hotkey
        .push_message_dialog(
            vote().with_checkbox("&Don't display again", false),
            MessageDialogContinuation::LeagueSurrender,
        )
        .test_value();
    handled_message_hotkey.test_modifiers(ModifiersState::ALT);
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
        .test_value();
    changed_release.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    changed_release.test_modifiers(ModifiersState::CONTROL);
    changed_release.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert_eq!(changed_release.message_dialogs.len(), 1);
    assert!(changed_release.running_chat.is_none());

    let mut exclusive_top_scope = boxed_running_sandbox_app();
    exclusive_top_scope
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let lower_layout = exclusive_top_scope.top_message_dialog_layout().test_value();
    exclusive_top_scope
        .push_message_dialog(small_vote(), MessageDialogContinuation::LeagueSurrender)
        .test_value();
    let exposed = PhysicalPosition::new(
        f64::from(lower_layout.bounds.x + 5),
        f64::from(lower_layout.bounds.y + 5),
    );
    exclusive_top_scope.test_cursor(exposed);
    exclusive_top_scope.test_left_button(ElementState::Pressed);
    exclusive_top_scope.test_left_button(ElementState::Released);
    assert_eq!(exclusive_top_scope.active_message_dialog_index(), Some(0));
    exclusive_top_scope.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    exclusive_top_scope.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert_eq!(exclusive_top_scope.message_dialogs.len(), 1);
    assert!(matches!(
        exclusive_top_scope.message_dialogs[0].continuation,
        MessageDialogContinuation::LeagueSurrender
    ));
    assert!(exclusive_top_scope.running_chat.is_none());

    let mut nonexclusive_top_scope = boxed_running_sandbox_app();
    nonexclusive_top_scope
        .push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
        .test_value();
    let lower_layout = nonexclusive_top_scope
        .top_message_dialog_layout()
        .test_value();
    nonexclusive_top_scope
        .push_message_dialog(small_notice(), MessageDialogContinuation::None)
        .test_value();
    let exposed = PhysicalPosition::new(
        f64::from(lower_layout.bounds.x + 5),
        f64::from(lower_layout.bounds.y + 5),
    );
    nonexclusive_top_scope.test_cursor(exposed);
    nonexclusive_top_scope.test_left_button(ElementState::Pressed);
    nonexclusive_top_scope.test_left_button(ElementState::Released);
    assert_eq!(
        nonexclusive_top_scope.active_message_dialog_index(),
        Some(0)
    );
    nonexclusive_top_scope.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(nonexclusive_top_scope.running_chat_text(), Some(""));
    assert_eq!(nonexclusive_top_scope.message_dialogs.len(), 2);

    for (key, modifiers, expected) in [
        (VirtualKeyCode::F2, ModifiersState::empty(), ""),
        (VirtualKeyCode::Enter, ModifiersState::SHIFT, "/team "),
        (VirtualKeyCode::Enter, ModifiersState::ALT, "\""),
    ] {
        let mut app = boxed_running_sandbox_app();
        app.push_message_dialog(vote(), MessageDialogContinuation::LeagueSurrender)
            .test_value();
        app.test_modifiers(modifiers);
        app.test_key(key, ElementState::Pressed);
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
        .test_value();
        app.test_modifiers(modifiers);
        app.test_key(key, ElementState::Pressed);
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

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    let surface_width = app.graphics.surface().width() as i32;
    let surface_height = app.graphics.surface().height() as i32;
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let layout = app.game_option_input_layout().test_value();
    let controller = app.running_chat_controller().test_value();
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
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
        app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
        assert!(app.context_menu.is_none());
    }
    app.test_modifiers(ModifiersState::empty());

    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    assert!(app.context_menu.is_some());
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert!(app.context_menu.is_none());
    assert_eq!(app.running_chat_text(), Some("/team "));
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert!(app.context_menu.is_none());
    assert_eq!(app.running_chat_text(), Some("\""));
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    assert!(app.context_menu.is_none());
    assert_eq!(app.running_chat_text(), Some(""));

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert!(app.running_chat.is_none());
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert_eq!(app.running_chat_text(), Some(""));
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(app.running_chat_text(), Some("/team "));
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(app.running_chat_text(), Some("\""));
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);

    for character in "alpha beta".chars() {
        app.test_text_input(character);
    }
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    app.test_gamepad_events([
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
    ]);
    assert!(app.ingame_menu.is_none());
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    let caret_before_alt_navigation = app.running_chat_controller().test_value().caret();
    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::CONTROL | ModifiersState::ALT,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
    ] {
        app.test_modifiers(modifiers);
        for key in [VirtualKeyCode::ArrowLeft, VirtualKeyCode::Backspace] {
            app.test_key(key, ElementState::Pressed);
            app.test_key(key, ElementState::Released);
        }
        assert_eq!(app.running_chat_text(), Some("alpha beta"));
        assert_eq!(
            app.running_chat_controller()
                .expect("chat remains open after Alt navigation probe")
                .caret(),
            caret_before_alt_navigation
        );
    }
    app.test_modifiers(ModifiersState::empty());
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    app.test_modifiers(ModifiersState::empty());
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(app.running_chat_text(), Some("alpha beta"));
    app.test_modifiers(ModifiersState::empty());
    assert_eq!(
        app.message_board_line(),
        board_before,
        "the message board remains a fading log instead of echoing edit text"
    );

    app.pressed_engine_keys.insert(VirtualKeyCode::KeyA);
    app.engine
        .test_player_mut(app.local_owner)
        .control
        .pressed_coms = 1 << clonk_engine::COM_LEFT;
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    assert!(app.context_menu.is_some());
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Notice",
            "The chat remains the higher input-z dialog.",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
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
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    app.test_text_input('!');
    assert_eq!(app.running_chat_text(), Some("alpha beta!"));
    assert_eq!(app.message_dialogs.len(), 1);
    let mut frame = vec![0_u8; (surface_width * surface_height * 4) as usize];
    app.test_render(&mut frame);
    assert!(frame.iter().any(|byte| *byte != 0));

    app.test_modifiers(ModifiersState::CONTROL | ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    assert!(app
        .running_chat_controller()
        .and_then(InputDialogController::selected_text)
        .is_some_and(|text| !text.is_empty()));
    app.test_modifiers(ModifiersState::empty());
    let keyboard_selection = app.running_chat_controller().test_value().selection();

    let start = PhysicalPosition::new(
        f64::from(layout.edit.x + 5),
        f64::from(layout.edit.y + layout.edit.h / 2),
    );
    let end = PhysicalPosition::new(
        f64::from(layout.edit.x + 35),
        f64::from(layout.edit.y + layout.edit.h / 2),
    );
    app.test_cursor(start);
    app.test_left_button(ElementState::Pressed);
    let selection_after_down = app.running_chat_controller().test_value().selection();
    assert!(selection_after_down.is_some_and(|(anchor, caret)| anchor == caret));
    assert_ne!(selection_after_down, keyboard_selection);
    app.test_cursor(end);
    app.test_left_button(ElementState::Released);
    assert!(app
        .running_chat_controller()
        .and_then(InputDialogController::selected_text)
        .is_some_and(|text| !text.is_empty()));
    app.test_right_button(ElementState::Pressed);
    assert!(app.context_menu.is_some());
    assert_eq!(app.message_dialogs.len(), 1);
    app.test_right_button(ElementState::Released);

    let text_before_context_key = app.running_chat_text().map(str::to_string);
    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Released);
    assert!(app.game_option_input_consumed_keys.is_empty());
    assert_eq!(app.running_chat_text(), text_before_context_key.as_deref());
    assert_eq!(
        app.running_chat.as_ref().map(|chat| chat.history_index),
        Some(-1)
    );

    let caret_before_ctrl_left = app.running_chat_controller().test_value().caret();
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    assert_eq!(
        app.running_chat_controller()
            .expect("chat remains open")
            .caret(),
        caret_before_ctrl_left
    );
    assert!(app.context_menu.is_some());
    app.test_modifiers(ModifiersState::empty());

    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    assert!(app.external_irc_dialog_visible);
    assert!(app.running_chat.is_none());
    assert!(app.context_menu.is_none());
    app.test_key(VirtualKeyCode::KeyC, ElementState::Released);
    app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    app.test_key(VirtualKeyCode::KeyC, ElementState::Released);
    assert!(!app.external_irc_dialog_visible);
    app.test_modifiers(ModifiersState::empty());
    assert!(app.game_option_input_dialog.is_none());
    assert!(app.context_menu.is_none());
    assert_eq!(app.message_dialogs.len(), 1);
    assert_eq!(app.message_board_line(), board_before);
}

#[test]
fn observer_menu_lists_players_and_live_previews_selection() {
    let mut app = new_state_only_running_sandbox_app();
    let first = app.local_owner;
    let first_info = app.engine.test_player(first).player_info_id();
    let second = first + 1;
    let hidden = first + 2;
    let second_info = first_info + 10;
    let hidden_info = first_info + 20;
    app.engine
        .register_player(
            PlayerConfig::new(second, "Second visible").with_player_info_id(second_info),
        )
        .test_value();
    app.engine
        .register_player(
            PlayerConfig::new(hidden, "Hidden target").with_player_info_id(hidden_info),
        )
        .test_value();
    let info = |id, name: &[u8], flags| clonk_engine::ControlPlayerInfoEntry {
        id,
        name: LegacyCString::from_bytes(name.to_vec()).test_value(),
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

    let menu = app.ingame_menu.get(OWNER_NONE).test_value();
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
    let menu = app.ingame_menu.get(app.local_owner).test_value();
    assert_eq!(menu.page(), ingame_menu::MenuPage::TeamSelection);
    assert_eq!(
        menu.items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>(),
        [MenuAction::SelectTeam(1), MenuAction::SelectTeam(2)]
    );

    let local_owner = app.local_owner;
    let outcome = app
        .ingame_menu
        .get_mut(local_owner)
        .expect("team selection menu opens")
        .handle_command(ControlCommand::MenuEnter, CommandKind::Press)
        .test_value();
    app.execute_ingame_menu_outcome(outcome).test_value();

    let player = app.engine.test_player(app.local_owner);
    assert_eq!(player.status(), PlayerStatus::Active);
    assert_eq!(player.team(), Some(1));
    assert!(
        app.engine.crew_cursor(app.local_owner).is_some(),
        "Regicide selection must leave the player with usable crew"
    );
    assert!(app.ingame_menu.is_none());

    let owner = app.local_owner;
    app.activate_ingame_main_menu_for_player(owner).test_value();
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
        .test_value();
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
        .test_value();
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
    app.test_key(VirtualKeyCode::KeyZ, ElementState::Pressed);

    app.open_initial_team_selection(secondary);
    assert_eq!(
        app.ingame_menu.as_ref().and_then(IngameMenuState::player),
        Some(secondary)
    );

    // Keyboard set 2 Key4 is Throw; an active C4MainMenu converts it to
    // MenuEnter and selects the first team.
    app.test_key(VirtualKeyCode::Numpad4, ElementState::Pressed);

    let secondary_player = app.engine.test_player(secondary);
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
    let mut rule = test_definition("IRUL", "Integrated Rule", "#strict 3\n");
    rule.set_category(C4D_RULE);
    rule.set_description(Some("Keep to the rule".to_string()));
    app.engine.register_test_definition(rule);
    app.engine
        .spawn_test_object(clonk_engine::SpawnConfig::new("IRUL"));
    app.snapshot = app.engine.snapshot();

    app.apply_ingame_menu_action_for_player(player, MenuAction::ActivateRules)
        .test_value();
    let menu = app.ingame_menu.get(player).test_value();
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
    app.open_ingame_menu().test_value();
    app.apply_ingame_menu_action(MenuAction::ActivateOptions)
        .test_value();

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    assert!(
        app.ingame_menu_gfx
            .as_ref()
            .is_some_and(|gfx| gfx.show_close_button),
        "the controlling mouse player's title renders its close button"
    );

    let close_rect = |app: &GameApp| {
        let player = app.local_owner;
        let area = app.graphics.viewport_rect(player).test_value();
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
            .test_value()
            .close_button_rect(area, &font, &gfx)
    };
    let close_point = |app: &GameApp| {
        let close = close_rect(app);
        PhysicalPosition::new(
            f64::from(close.x) + f64::from(close.width) / 2.0,
            f64::from(close.y) + f64::from(close.height) / 2.0,
        )
    };

    app.test_cursor(close_point(&app));
    app.test_right_button(ElementState::Pressed);
    app.test_right_button(ElementState::Released);
    assert_eq!(
        app.ingame_menu
            .get(app.local_owner)
            .map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Options),
        "right-click must not invoke Dialog::OnUserClose"
    );
    assert!(commands.take_submitted_local().is_empty());

    let close = close_rect(&app);
    app.test_cursor(PhysicalPosition::new(
        f64::from(close.x - 1),
        f64::from(close.y) + f64::from(close.height) / 2.0,
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(close_point(&app));
    app.test_left_button(ElementState::Released);
    assert_eq!(
        app.ingame_menu
            .get(app.local_owner)
            .map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Options),
        "release-over must not close unless the close button retained left-down"
    );
    assert!(commands.take_submitted_local().is_empty());

    app.test_cursor(close_point(&app));
    app.test_left_button(ElementState::Pressed);
    assert_eq!(
        app.ingame_menu
            .get(app.local_owner)
            .map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Options),
        "IconButton closes on button-up, not button-down"
    );
    assert!(commands.take_submitted_local().is_empty());
    app.test_left_button(ElementState::Released);
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

    app.test_cursor(close_point(&app));
    app.test_left_button(ElementState::Pressed);
    assert!(app.ingame_menu.contains(app.local_owner));
    assert!(commands.take_submitted_local().is_empty());
    app.test_left_button(ElementState::Released);
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
    app.open_ingame_menu().test_value();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    assert!(app
        .ingame_menu_gfx
        .as_ref()
        .is_some_and(|gfx| gfx.show_close_button));

    app.ingame_menu.clear();
    app.ingame_menu.replace(
        owner + 1,
        IngameMenuState::main_menu(&MainMenuConditions::default(), &IngameMenuLabels::default()),
    );
    app.test_render(&mut frame);
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
    app.test_render(&mut frame);
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
    app.test_render(&mut frame);
    assert!(
        !app.ingame_menu_gfx
            .as_ref()
            .is_some_and(|gfx| gfx.show_close_button),
        "DisableMouse=1 suppresses the title close button"
    );

    let area = app.graphics.viewport_rect(owner).test_value();
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let close = app.ingame_menu.get(owner).test_value().close_button_rect(
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
    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_point.x),
        f64::from(menu_point.y),
    ));
    app.test_left_button(ElementState::Pressed);

    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_point.x + 4.0),
        f64::from(menu_point.y),
    ));
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Candidate { .. })
    ));
    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_point.x + MENU_DRAG_THRESHOLD),
        f64::from(menu_point.y),
    ));
    assert!(app.ingame_construction_drag_active());
    assert!(app.mouse_state.is_none());
    assert!(app.ingame_right_mouse_state.is_none());
    assert!(app.ingame_custom_cursor_active());

    app.handle_focus_lost().test_value();
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

    app.test_cursor(PhysicalPosition::new(
        f64::from(menu_point.x),
        f64::from(menu_point.y),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);

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
    app.test_cursor(PhysicalPosition::new(
        f64::from(invalid_point.x),
        f64::from(invalid_point.y),
    ));
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            site_valid: false,
            ..
        })
    ));

    app.test_left_button(ElementState::Released);
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
    app.test_update();
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
    let retained = app.ingame_viewport_mouse.test_value();
    assert!(matches!(
        app.construction_menu_drag.as_ref(),
        Some(ConstructionMenuDrag::Active {
            viewport_index: Some(index),
            ..
        }) if *index == retained.viewport_index
    ));

    app.engine
        .test_player_mut(owner)
        .set_view_offset(Vector2::new(7, 0));
    app.snapshot = app.engine.snapshot();
    let render_snapshot = app.snapshot.clone();
    let viewports = collect_viewport_inputs(&render_snapshot).test_value();
    app.graphics.render_frame(&render_snapshot, &viewports);
    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.index == retained.viewport_index)
        .test_value();
    let screen = GuiPoint::new(
        viewport.rect.x.saturating_add(retained.position.x) as f32,
        viewport.rect.y.saturating_add(retained.position.y) as f32,
    );
    let expected_pointer = app
        .graphics
        .viewport_output_point_for_index(viewport.index, screen)
        .test_value();
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
    app.test_left_button(ElementState::Released);
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
        let mut probe = test_definition(definition_id, "Menu race probe", script);
        probe.set_category(clonk_engine::CATEGORY_LIVING);
        probe.set_crew_member(true);
        app.engine.register_test_definition(probe);
        let cursor = app.engine.spawn_test_object(
            SpawnConfig::new(definition_id)
                .with_owner(owner)
                .with_crew_member(true),
        );
        let mut crew = app.engine.test_player(owner).crew().to_vec();
        crew.push(cursor);
        app.engine.test_player_mut(owner).set_crew(crew);
        app.engine.clear_crew_selection(owner);
        app.engine.select_crew(owner, [cursor]).test_value();
        app.engine.set_crew_cursor(owner, Some(cursor)).test_value();
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
        .test_value();
        let (_, converted, tick) = commands.take_submitted_local().pop().test_value();
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(None),
                    ..ObjectUpdate::default()
                },
            )
            .test_value();
        app.apply_ready_controls(
            tick,
            vec![NetworkControl::Player {
                owner,
                event: converted,
            }],
        )
        .test_value();
        let cursor = app.engine.test_object_snapshot(cursor);
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
        .test_value();
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
    let cursor = app.engine.test_crew_cursor(app.local_owner);
    let menu = two_item_script_menu(cursor);

    let mut baseline = vec![0u8; 320 * 200 * 4];
    app.test_render(&mut baseline);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    let mut with_menu = vec![0u8; 320 * 200 * 4];
    app.test_render(&mut with_menu);
    assert_ne!(
        with_menu, baseline,
        "an engine-created script menu must be visible"
    );
    let mut before_tooltip = with_menu.clone();
    for _ in 1..89 {
        app.test_render(&mut before_tooltip);
    }
    let mut with_tooltip = vec![0u8; 320 * 200 * 4];
    app.test_render(&mut with_tooltip);
    assert_ne!(
        with_tooltip, before_tooltip,
        "C4MN_InfoCaption_Delay shows the tooltip on draw 90"
    );

    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .test_value();
    app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
        .test_value();
    let menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor exists")
        .test_value();
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
    .test_value();
    app.dispatch_control_event(ControlEvent::Command {
        command: ControlCommand::Throw,
        kind: CommandKind::Release,
    })
    .test_value();
    assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));
}

#[test]
fn first_local_menu_press_reveals_progressive_text_before_navigation() {
    // C4Game::LocalPlayerControl performs the asynchronous ConvertCom
    // pass before offline dispatch/network submission. Only this local
    // raw press may become COM_MenuShowText; synchronized controls must
    // not recalculate the choice from client-specific text progress.
    let mut app = new_state_only_running_sandbox_app();
    let cursor = app.engine.test_crew_cursor(app.local_owner);
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
        .test_value();

    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .test_value();
    let menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor exists")
        .test_value();
    assert_eq!(menu.selection, 0, "reveal must not navigate");
    assert!(!menu.text_progressing);
    assert!(menu
        .items
        .iter()
        .all(|item| item.text_display_progress == -1));

    app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
        .test_value();
    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .test_value();
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
fn normal_menu_render_draws_no_symbol_for_an_unresolved_item_picture() {
    // C4MenuItem::DrawElement blits a row symbol only while its facet holds a
    // surface (C4Menu.cpp:166), so a picture that never resolved leaves the
    // cell empty and the round continues. A refill can outlive the object a
    // row was built from; failing the frame there ends the event loop and
    // drops the client out of a running network game instead.
    fn render_first_item_recipe(image: clonk_engine::ObjectMenuImage) -> Vec<u8> {
        let mut app = new_classic_running_sandbox_app();
        let cursor = app.engine.test_crew_cursor(app.local_owner);
        let mut menu = two_item_script_menu(cursor);
        menu.style = 0;
        menu.items[0].item_id = "MISS".to_string();
        menu.items[0].image = image;
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
            "both fixtures must leave the row without a resolved symbol"
        );
        install_test_cursor_menu(&mut app, cursor, menu);

        let mut frame = vec![0_u8; app.graphics.surface().pixels().len()];
        app.test_render(&mut frame);
        frame
    }

    let unresolved = render_first_item_recipe(clonk_engine::ObjectMenuImage::Definition);
    let empty = render_first_item_recipe(clonk_engine::ObjectMenuImage::None);
    assert!(
        unresolved.iter().any(|&channel| channel != 0),
        "the menu around the empty cell must still be drawn"
    );
    assert_eq!(
        unresolved, empty,
        "an unresolved picture must draw exactly like an empty C++ symbol facet"
    );
}

#[test]
fn engine_dialog_menu_renders_classic_style_instead_of_fallback() {
    let mut app = new_classic_running_sandbox_app();
    let cursor = app.engine.test_crew_cursor(app.local_owner);
    let mut menu = two_item_script_menu(cursor);
    menu.caption.clear();
    menu.style = 3;
    menu.columns = 1;
    for item in &mut menu.items {
        item.image = clonk_engine::ObjectMenuImage::None;
    }
    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut baseline);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    let mut rendered = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut rendered);
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
    let cursor = app.engine.test_crew_cursor(app.local_owner);
    let mut menu = two_item_script_menu(cursor);
    menu.caption = "Hut".to_string();
    menu.identification = serde_json::from_value(serde_json::json!({ "Int": 14 })).test_value();
    menu.style = 1;
    menu.permanent = true;
    menu.user_menu = false;
    menu.columns = 1;

    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut baseline);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    let mut with_menu = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut with_menu);
    assert_ne!(with_menu, baseline, "style-1 context menu must be visible");

    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .test_value();
    app.dispatch_control_event(ControlEvent::Release(ControlButton::Right))
        .test_value();
    let menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor exists")
        .test_value();
    assert_eq!(menu.selection, 1);
    let context_identification =
        serde_json::from_value(serde_json::json!({ "Int": 14 })).test_value();
    assert_eq!(menu.identification, context_identification);
}

#[test]
fn engine_info_menu_renders_the_classic_style_instead_of_a_fallback() {
    clonk_logging::init();
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(app.local_owner);
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
    app.test_render(&mut baseline);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    let mut with_menu = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut with_menu);
    assert_ne!(with_menu, baseline);
    let initial_location = app
        .script_menu_presentations
        .get(&owner)
        .and_then(|state| state.location)
        .test_value();

    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate::default().with_position(Vector2::new(280, 160)),
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    app.refresh_focus();
    app.test_render(&mut with_menu);
    assert_eq!(
        app.script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location),
        Some(initial_location),
        "C4Menu::SetLocation is one-shot; the menu must not follow a moving target"
    );
}

#[test]
fn context_style_script_menu_reaches_command2_by_right_click_and_special2() {
    // The Menu2 order page is C4MN_Style_Context with one column, and -1 lives
    // on Command2. Coverage for the secondary activation was all either
    // engine-level or on internal Contents menus, so nothing exercised the
    // shape ClonkMars actually ships through the real input layer. Both routes
    // C4Menu offers a player are checked here: right-up (C4Menu.cpp:228-232)
    // and Special2 (C4Menu.cpp:1053).
    clonk_logging::init();
    for use_keyboard in [false, true] {
        let mut app = new_classic_running_sandbox_app();
        let cursor = app.engine.test_crew_cursor(app.local_owner);
        let mut menu = two_item_script_menu(cursor);
        menu.style = 1; // C4MN_Style_Context (C4Menu.h:40)
        menu.items[1].command2 = "SetComDir(COMD_Right())".to_string();
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(menu.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .test_value();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.test_render(&mut frame);

        let second_item = {
            let fallback = app.assets.font_arc();
            let font = clonk_frontend::hud::HudFont::from_set(
                app.assets.clonk_fonts.as_deref(),
                fallback.as_ref(),
            );
            let area = app.graphics.viewport_rect(app.local_owner).test_value();
            object_menu::engine_script_menu_layout(
                area,
                &font,
                &menu,
                app.display_flags.show_commands,
            )
            .item_rect(1)
            .test_value()
        };
        let second_point = PhysicalPosition::new(
            f64::from(second_item.x) + 4.0,
            f64::from(second_item.y) + 4.0,
        );
        app.test_cursor(second_point);
        assert_eq!(
            app.engine
                .debug_object_menu(cursor.as_u64())
                .expect("cursor")
                .expect("menu")
                .selection,
            1,
            "a one-column Context row is hit-tested like any other"
        );

        if use_keyboard {
            AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyF);
        } else {
            app.test_right_button(ElementState::Pressed);
            app.test_right_button(ElementState::Released);
        }

        assert_eq!(
            app.engine
                .object_snapshot(cursor)
                .expect("cursor survives the secondary activation")
                .command_direction,
            CommandDirection::Right,
            "Command2 must run (keyboard: {use_keyboard})"
        );
    }
}

#[test]
fn engine_script_menu_pointer_selects_enters_and_closes_like_cpp() {
    // C4MenuItem::MouseEnter selects a selectable item, left-up enters
    // it, and Dialog's Ico_Close queues COM_MenuClose
    // (C4Menu.cpp:213-242, 1237-1262; C4ObjectMenu.cpp:461-478).
    clonk_logging::init();
    let mut app = new_classic_running_sandbox_app();
    let cursor = app.engine.test_crew_cursor(app.local_owner);
    let menu = two_item_script_menu(cursor);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu.clone())),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);

    let (second_item, close_button) = {
        let fallback = app.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            app.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let area = app.graphics.viewport_rect(app.local_owner).test_value();
        let layout = object_menu::engine_script_menu_layout(
            area,
            &font,
            &menu,
            app.display_flags.show_commands,
        );
        (layout.item_rect(1).test_value(), layout.close_button_rect())
    };
    let second_point = PhysicalPosition::new(
        f64::from(second_item.x) + 8.0,
        f64::from(second_item.y) + 8.0,
    );
    app.test_cursor(second_point);
    assert_eq!(
        app.engine
            .debug_object_menu(cursor.as_u64())
            .expect("cursor")
            .expect("menu")
            .selection,
        1,
        "hover must select the item under the pointer"
    );
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
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
        .test_value();
    app.test_cursor(second_point);
    app.test_right_button(ElementState::Pressed);
    app.test_right_button(ElementState::Released);
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
        .test_value();
    let close_point = PhysicalPosition::new(
        f64::from(close_button.x) + 8.0,
        f64::from(close_button.y) + 8.0,
    );
    app.test_cursor(close_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    assert_eq!(app.engine.debug_object_menu(cursor.as_u64()), Some(None));
}

#[test]
fn script_menu_pre_first_draw_discards_explicit_rows() {
    // C4Menu::SetSize writes Lines without clearing LocationSet, but a newly
    // created menu still draws through InitLocation first, which derives the
    // row count from the item set (C4Menu.cpp:635-640,713-721,796-797).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    let mut menu = long_script_menu(cursor, 4);
    menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, menu.clone());
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);
    let (_, layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("layout resources")
        .test_value();
    let area = app.graphics.viewport_rect(owner).test_value();
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let derived_layout =
        object_menu::engine_script_menu_layout(area, &font, &menu, app.display_flags.show_commands);
    assert_eq!(layout.lines, derived_layout.lines);
    assert_eq!(layout.visible, derived_layout.visible);
    assert_eq!(layout.client.height, derived_layout.client.height);
    assert_eq!(layout.scrollbar, derived_layout.scrollbar);
    assert!(layout.item_rect(3).is_some());
    assert_eq!(app.script_menu_presentations[&owner].explicit_lines, None);
}

#[test]
fn script_menu_explicit_rows_survive_stable_live_draws() {
    // C4Menu::SetSize reruns only InitSize and leaves LocationSet set, so a
    // row count issued after the first draw controls the live client, visible
    // count, scrollbar and hit grid (C4Menu.cpp:635-640,755-780).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    install_test_cursor_menu(&mut app, cursor, long_script_menu(cursor, 4));
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);

    let mut explicit_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    explicit_menu.lines = 2;
    install_test_cursor_menu(&mut app, cursor, explicit_menu.clone());
    app.test_render(&mut frame);
    let (_, first_live_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("first live layout resources")
        .test_value();
    assert_eq!(first_live_layout.lines, 2);
    assert_eq!(first_live_layout.visible, 2);
    assert!(first_live_layout.scrollbar.is_some());
    assert!(first_live_layout.item_rect(0).is_some());
    assert!(first_live_layout.item_rect(1).is_some());

    explicit_menu.selection = 2;
    install_test_cursor_menu(&mut app, cursor, explicit_menu.clone());
    app.test_render(&mut frame);
    let (_, stable_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("stable live layout resources")
        .test_value();
    assert_eq!(stable_layout.lines, 2);
    assert_eq!(stable_layout.visible, 2);
    assert_eq!(stable_layout.client.height, first_live_layout.client.height);
    assert!(stable_layout.scrollbar.is_some());
    assert!(stable_layout.item_rect(0).is_none());
    assert!(stable_layout.item_rect(2).is_some());
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines,
        Some(2)
    );

    // Normal-menu shrink does not clear LocationSet, so the live explicit
    // row count remains even when the derived one-row item set would fit in a
    // smaller grid (C4Menu.cpp:961-969).
    explicit_menu.items.truncate(1);
    explicit_menu.selection = 0;
    install_test_cursor_menu(&mut app, cursor, explicit_menu);
    app.test_render(&mut frame);
    let (_, stable_shrink_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("stable shrink layout resources")
        .test_value();
    assert_eq!(stable_shrink_layout.lines, 2);
    assert_eq!(stable_shrink_layout.visible, 2);
    assert!(stable_shrink_layout.scrollbar.is_none());
    assert!(stable_shrink_layout.item_rect(0).is_some());
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines,
        Some(2)
    );
}

#[test]
fn script_menu_growth_refill_recomputes_explicit_rows_and_visible_grid() {
    // C4Menu::RefillInternal clears LocationSet whenever a refill grows the
    // item set (C4Menu.cpp:947-970). The next Draw therefore reruns
    // InitLocation, which recomputes Lines and VisibleCount from the new item
    // count instead of retaining a live SetMenuSize row count
    // (C4Menu.cpp:713-721,755-780,796-797).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    install_test_cursor_menu(&mut app, cursor, long_script_menu(cursor, 2));
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);

    let mut explicit_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    explicit_menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, explicit_menu);
    app.test_render(&mut frame);
    let (_, explicit_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("explicit layout resources")
        .test_value();
    assert_eq!(explicit_layout.lines, 1);
    assert_eq!(explicit_layout.visible, 1);
    assert!(explicit_layout.scrollbar.is_some());

    let mut grown_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    let grown_items = long_script_menu(cursor, 4).items;
    grown_menu.items = grown_items;
    grown_menu.location_reset_generation = grown_menu.location_reset_generation.wrapping_add(1);
    // A refill callback may issue SetMenuSize while it adds rows. C++ clears
    // LocationSet after the refill, so this changed value is discarded by
    // the same draw that observes the growth (C4Menu.cpp:635-640,947-970).
    grown_menu.lines = 2;
    grown_menu.selection = 3;
    install_test_cursor_menu(&mut app, cursor, grown_menu.clone());
    app.test_render(&mut frame);
    let (_, grown_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("grown layout resources")
        .test_value();
    let area = app.graphics.viewport_rect(owner).test_value();
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let derived_layout = object_menu::engine_script_menu_layout(
        area,
        &font,
        &grown_menu,
        app.display_flags.show_commands,
    );
    assert_eq!(grown_layout.lines, derived_layout.lines);
    assert_eq!(grown_layout.visible, derived_layout.visible);
    assert_eq!(grown_layout.client.height, derived_layout.client.height);
    assert_eq!(
        grown_layout.scrollbar.is_some(),
        derived_layout.scrollbar.is_some()
    );
    assert!(
        grown_layout.item_rect(3).is_some(),
        "selection remains visible"
    );
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines, None,
        "growth must invalidate the live explicit row count"
    );
}

#[test]
fn script_menu_pointer_hit_test_invalidates_growth_before_redraw() {
    // Input can arrive after the simulation has rebuilt a menu but before the
    // next presentation. Native C4Menu has already cleared LocationSet at
    // that point, so the hit grid must use the grown natural row count rather
    // than the prior explicit SetMenuSize count (C4Menu.cpp:947-970).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    install_test_cursor_menu(&mut app, cursor, long_script_menu(cursor, 2));
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);

    let mut explicit_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    explicit_menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, explicit_menu);
    app.test_render(&mut frame);

    let mut grown_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    grown_menu.items = long_script_menu(cursor, 4).items;
    grown_menu.location_reset_generation = grown_menu.location_reset_generation.wrapping_add(1);
    grown_menu.lines = 1;
    grown_menu.selection = 0;
    let area = app.graphics.viewport_rect(owner).test_value();
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let natural_layout = object_menu::engine_script_menu_layout(
        area,
        &font,
        &grown_menu,
        app.display_flags.show_commands,
    );
    let fourth_item = natural_layout.item_rect(3).test_value();
    install_test_cursor_menu(&mut app, cursor, grown_menu);

    app.test_cursor(PhysicalPosition::new(
        f64::from(fourth_item.x) + 8.0,
        f64::from(fourth_item.y) + 8.0,
    ));
    assert_eq!(
        app.engine
            .debug_object_menu(cursor.as_u64())
            .expect("cursor")
            .expect("grown menu")
            .selection,
        3,
        "pointer hit-testing must observe refill invalidation before redraw"
    );
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines, None,
        "pointer input must not retain the stale explicit row count"
    );
}

#[test]
fn script_menu_live_add_item_preserves_explicit_rows() {
    // C4Menu::AddItem updates the live grid but does not clear LocationSet;
    // only C4ObjectMenu::RefillInternal owns the count-based invalidation
    // (C4Menu.cpp:401-430; C4ObjectMenu.cpp:947-970).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    install_test_cursor_menu(&mut app, cursor, long_script_menu(cursor, 2));
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);

    let mut explicit_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    explicit_menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, explicit_menu.clone());
    app.test_render(&mut frame);
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines,
        Some(1)
    );

    explicit_menu.items.push(explicit_menu.items[0].clone());
    explicit_menu.selection = 0;
    install_test_cursor_menu(&mut app, cursor, explicit_menu);
    app.test_render(&mut frame);
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines,
        Some(1),
        "ordinary AddMenuItem growth must not mimic native refill invalidation"
    );
}

#[test]
fn script_menu_viewport_reset_does_not_restore_same_frame_set_size() {
    // C4Viewport resets LocationSet before the next Draw; a SetMenuSize made
    // in that interval is overwritten by InitLocation, not retained as a
    // live explicit row count (C4Menu.h:203; C4Menu.cpp:635-640,713-721).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    install_test_cursor_menu(&mut app, cursor, long_script_menu(cursor, 4));
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);

    let mut explicit_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    explicit_menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, explicit_menu.clone());
    app.test_render(&mut frame);
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines,
        Some(1)
    );

    app.resize(320, 200).test_value();
    explicit_menu.lines = 2;
    install_test_cursor_menu(&mut app, cursor, explicit_menu);
    let mut resized_frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut resized_frame);
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines, None,
        "viewport reset must dominate a same-frame SetMenuSize"
    );
}

#[test]
fn context_menu_shrink_refill_recomputes_explicit_rows_and_scrollbar() {
    // C4Menu::RefillInternal resizes a context menu when its item count
    // decreases, while ordinary menus keep their retained location
    // (C4Menu.cpp:961-969). The invalidation reruns InitLocation's derived
    // row count and InitSize's VisibleCount/scrollbar geometry
    // (C4Menu.cpp:713-721,755-780).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    let mut initial_menu = long_script_menu(cursor, 4);
    initial_menu.style = 1;
    install_test_cursor_menu(&mut app, cursor, initial_menu);
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);

    let mut explicit_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    explicit_menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, explicit_menu);
    app.test_render(&mut frame);
    let (_, explicit_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("explicit layout resources")
        .test_value();
    assert_eq!(explicit_layout.lines, 1);
    assert!(explicit_layout.scrollbar.is_some());

    let mut shrunk_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    shrunk_menu.items.truncate(2);
    shrunk_menu.location_reset_generation = shrunk_menu.location_reset_generation.wrapping_add(1);
    shrunk_menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, shrunk_menu.clone());
    app.test_render(&mut frame);
    let (_, shrunk_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("shrunk layout resources")
        .test_value();
    let area = app.graphics.viewport_rect(owner).test_value();
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let derived_layout = object_menu::engine_script_menu_layout(
        area,
        &font,
        &shrunk_menu,
        app.display_flags.show_commands,
    );
    assert_eq!(shrunk_layout.lines, derived_layout.lines);
    assert_eq!(shrunk_layout.visible, derived_layout.visible);
    assert_eq!(shrunk_layout.client.height, derived_layout.client.height);
    assert_eq!(shrunk_layout.scrollbar, derived_layout.scrollbar);
    assert!(shrunk_layout.item_rect(1).is_some());
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines, None,
        "a Context shrink must invalidate the live explicit row count"
    );
}

#[test]
fn script_menu_viewport_resize_recomputes_explicit_rows_and_hit_regions() {
    // C4Viewport marks menu positions for reset whenever its output size
    // changes (C4Viewport.cpp:780-803,1482-1494), then ResetLocation makes
    // the next Draw rerun InitLocation (C4Menu.cpp:713-721,796-797).
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    install_test_cursor_menu(&mut app, cursor, long_script_menu(cursor, 4));
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);

    let mut explicit_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    explicit_menu.lines = 1;
    install_test_cursor_menu(&mut app, cursor, explicit_menu);
    app.test_render(&mut frame);
    let (_, explicit_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("explicit layout resources")
        .test_value();
    assert_eq!(explicit_layout.lines, 1);
    assert!(explicit_layout.scrollbar.is_some());

    app.resize(320, 200).test_value();
    let mut resized_frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut resized_frame);
    let (_, resized_layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("resized layout resources")
        .test_value();
    let area = app.graphics.viewport_rect(owner).test_value();
    let fallback = app.assets.font_arc();
    let font = clonk_frontend::hud::HudFont::from_set(
        app.assets.clonk_fonts.as_deref(),
        fallback.as_ref(),
    );
    let resized_menu = app
        .engine
        .debug_object_menu(cursor.as_u64())
        .expect("cursor menu")
        .expect("menu remains open");
    let derived_layout = object_menu::engine_script_menu_layout(
        area,
        &font,
        &resized_menu,
        app.display_flags.show_commands,
    );
    assert_eq!(resized_layout.lines, derived_layout.lines);
    assert_eq!(resized_layout.visible, derived_layout.visible);
    assert_eq!(resized_layout.client.height, derived_layout.client.height);
    assert_eq!(resized_layout.scrollbar, derived_layout.scrollbar);
    assert!(resized_layout.item_rect(0).is_some());
    assert_eq!(
        app.script_menu_presentations[&owner].explicit_lines, None,
        "viewport reset must discard the old live row count"
    );
}

#[test]
fn running_menu_wheels_are_pixel_persistent_and_never_reach_gameplay() {
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    let menu = long_script_menu(cursor, 12);
    install_test_cursor_menu(&mut app, cursor, menu);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);

    let (_, layout) = app
        .script_menu_layout_for_owner(owner, false)
        .expect("script layout resources")
        .test_value();
    assert!(layout.max_scroll_y >= 60);
    let client_point = GuiPoint::new((layout.client.x + 4) as f32, (layout.client.y + 4) as f32);
    app.test_cursor(PhysicalPosition::new(
        f64::from(client_point.x),
        f64::from(client_point.y),
    ));
    let selection = app
        .engine
        .cursor_object_menu(owner)
        .test_value()
        .1
        .selection;
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
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
    app.test_render(&mut frame);
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
        .test_value();
    let title = geometry.title.test_value();
    let title_point = GuiPoint::new((title.x + 24) as f32, (title.y + 5) as f32);
    app.test_cursor(PhysicalPosition::new(
        f64::from(title_point.x),
        f64::from(title_point.y),
    ));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
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
        .test_value();
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
    app.test_render(&mut frame);
    let area = app.ingame_menu_area(owner).test_value();
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
        .test_value()
        .bounds(area, &font, &gfx);
    let player_client = GuiPoint::new((bounds.x + 6) as f32, (bounds.y + 30) as f32);
    assert!(app
        .ingame_menu
        .get(owner)
        .expect("player menu")
        .client_contains(area, &font, &gfx, player_client));
    app.test_cursor(PhysicalPosition::new(
        f64::from(player_client.x),
        f64::from(player_client.y),
    ));
    let selection = app.ingame_menu.get(owner).test_value().selection();
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    let player_menu = app.ingame_menu.get(owner).test_value();
    assert_eq!(player_menu.scroll_y(), 60);
    assert_eq!(player_menu.selection(), selection);
    assert!(commands.take_submitted_local().is_empty());
    app.test_render(&mut frame);
    assert_eq!(app.ingame_menu.get(owner).unwrap().scroll_y(), 60);
}

#[test]
fn script_menu_scroll_and_drag_state_is_per_viewport_owner() {
    let mut app = new_classic_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = primary + 1;
    let primary_cursor = app.engine.test_crew_cursor(primary);
    let primary_state = app.engine.test_object_snapshot(primary_cursor);

    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary"))
        .test_value();
    let secondary_position = Vector2::new(
        primary_state.position.x.saturating_add(24),
        primary_state.position.y,
    );
    let secondary_cursor = app.engine.spawn_test_object(
        SpawnConfig::new(primary_state.definition_id)
            .with_position(secondary_position)
            .with_owner(secondary)
            .with_crew_member(true),
    );
    app.engine
        .select_crew(secondary, [secondary_cursor])
        .test_value();
    app.engine
        .set_crew_cursor(secondary, Some(secondary_cursor))
        .test_value();
    app.engine
        .replace_player_viewports(
            secondary,
            vec![clonk_engine::PlayerViewport::new(secondary_position)
                .with_focus(Some(secondary_cursor))],
        )
        .test_value();
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
    app.test_render(&mut frame);
    assert!(app.script_menu_presentations.contains_key(&primary));
    assert!(app.script_menu_presentations.contains_key(&secondary));

    let (_, secondary_layout) = app
        .script_menu_layout_for_owner(secondary, false)
        .expect("secondary layout resources")
        .test_value();
    assert!(secondary_layout.max_scroll_y >= 60);
    let client = PhysicalPosition::new(
        f64::from(secondary_layout.client.x + 4),
        f64::from(secondary_layout.client.y + 4),
    );
    app.test_cursor(client);
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    assert_eq!(app.script_menu_presentations[&primary].scroll_y, 0);
    assert_eq!(app.script_menu_presentations[&secondary].scroll_y, 60);
    app.test_render(&mut frame);
    assert_eq!(app.script_menu_presentations[&primary].scroll_y, 0);
    assert_eq!(app.script_menu_presentations[&secondary].scroll_y, 60);

    let (_, geometry) = app
        .script_menu_geometry_for_owner(secondary)
        .expect("secondary geometry resources")
        .test_value();
    let title = geometry.title.test_value();
    let start = PhysicalPosition::new(f64::from(title.x + 3), f64::from(title.y + 5));
    app.test_cursor(start);
    app.test_left_button(ElementState::Pressed);
    let destination = PhysicalPosition::new(start.x + 11.0, start.y + 7.0);
    app.test_cursor(destination);
    app.test_left_button(ElementState::Released);
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
            .test_ref()
            .system
            .load_music(&test_music_bytes)
            .test_value()
    };
    let prime_music_toggle_off = |app: &mut GameApp, music: &MusicHandle| {
        app.audio
            .test_ref()
            .system
            .play_music(music, true)
            .test_value();
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
        .test_player_mut(rebound_app.local_owner)
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
        default_app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
        let draws_before = default_app.runtime_flash_message.test_ref().remaining_draws;
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
        default_app.test_key(VirtualKeyCode::F3, ElementState::Released);

        rebound_app
            .ingame_menu
            .replace(rebound_app.local_owner, Some(rebound_menu));
        rebound_app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
        assert!(rebound_app.runtime_flash_message.is_none(), "page {page:?}");
        assert!(rebound_app.ingame_menu.is_some(), "page {page:?}");
        rebound_app.test_key(VirtualKeyCode::F3, ElementState::Released);
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
        let sound_before = sound_app.audio.test_ref().options.sound_enabled;
        sound_app.test_modifiers(ModifiersState::CONTROL);
        sound_app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
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
        sound_app.test_key(VirtualKeyCode::F3, ElementState::Released);
        sound_app.test_modifiers(ModifiersState::empty());
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
        .test_player_mut(rebound.local_owner)
        .control
        .control_style = true;
    let mut sound = new_classic_lightweight_running_sandbox_app();
    for style in 0..=3 {
        for text_progressing in [false, true] {
            let install_menu = |app: &mut GameApp| {
                let cursor = app.engine.test_crew_cursor(app.local_owner);
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
                    .test_value();
                app.snapshot = app.engine.snapshot();
            };

            install_menu(&mut default_app);
            prime_music_toggle_off(&mut default_app, &default_music);
            default_app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
            let draws_before = default_app.runtime_flash_message.test_ref().remaining_draws;
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
            default_app.test_key(VirtualKeyCode::F3, ElementState::Released);

            install_menu(&mut rebound);
            rebound.test_key(VirtualKeyCode::F3, ElementState::Pressed);
            assert!(rebound.runtime_flash_message.is_none());
            assert!(rebound
                .engine
                .cursor_object_menu(rebound.local_owner)
                .is_some());
            rebound.test_key(VirtualKeyCode::F3, ElementState::Released);
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
            let before = sound.audio.test_ref().options.sound_enabled;
            sound.test_modifiers(ModifiersState::CONTROL);
            sound.test_key(VirtualKeyCode::F3, ElementState::Pressed);
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
            sound.test_key(VirtualKeyCode::F3, ElementState::Released);
            sound.test_modifiers(ModifiersState::empty());
        }
    }
}

#[test]
fn runtime_flash_draws_above_f1_help_and_below_recursive_context_gui() {
    let mut help = new_classic_running_sandbox_app();
    help.status_text.clear();
    help.snapshot.hud.messages.clear();
    help.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    help.set_runtime_flash_message("AAAA", RuntimeHelpCharset::Windows1252)
        .test_value();
    let flash = help.runtime_flash_message.take().test_value();
    let mut help_only = vec![0_u8; 320 * 200 * 4];
    help.test_render(&mut help_only);
    let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
    expected.pixels_mut().copy_from_slice(&help_only);
    let gamma = help
        .graphics
        .active_gamma_ramp(&help.snapshot.environment.gamma);
    let fonts = help.assets.clonk_fonts.clone().test_value();
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
    help.test_render(&mut actual);
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
        .test_value();
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
        .test_value();
    let menu = context.context_menu.take().test_value();
    let flash = context.runtime_flash_message.clone().test_value();
    let mut flash_only = vec![0_u8; 320 * 200 * 4];
    context.test_render(&mut flash_only);
    let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
    expected.pixels_mut().copy_from_slice(&flash_only);
    let gamma = context
        .graphics
        .active_gamma_ramp(&context.snapshot.environment.gamma);
    menu.render(&mut expected, Some(&gamma)).test_value();
    context.context_menu = Some(menu);
    context.runtime_flash_message = Some(flash);
    let mut actual = vec![0_u8; 320 * 200 * 4];
    context.test_render(&mut actual);
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
        .test_player_mut(rebound_app.local_owner)
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
        default_app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
        assert!(default_app.runtime_help_visible, "page {page:?}");
        assert_eq!(
            default_app.ingame_menu.as_ref().map(IngameMenuState::page),
            Some(page)
        );
        default_app.test_key(VirtualKeyCode::F1, ElementState::Released);
        default_app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
        default_app.test_key(VirtualKeyCode::F1, ElementState::Released);
        assert!(!default_app.runtime_help_visible, "page {page:?}");

        rebound_app
            .ingame_menu
            .replace(rebound_app.local_owner, Some(rebound_menu));
        rebound_app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
        assert!(!rebound_app.runtime_help_visible, "page {page:?}");
        assert!(rebound_app.ingame_menu.is_some(), "page {page:?}");
        rebound_app.test_key(VirtualKeyCode::F1, ElementState::Released);
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
        .test_value();
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
    observer.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(observer.runtime_help_visible);
    assert!(observer.ingame_menu.is_some());

    let mut object = new_running_sandbox_app();
    assert!(object.open_object_menu().expect("open object menu"));
    object
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    object
        .engine
        .test_player_mut(object.local_owner)
        .control
        .control_style = true;
    object.test_key(VirtualKeyCode::F1, ElementState::Pressed);
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
        .test_value();
    message
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    message.test_key(VirtualKeyCode::F1, ElementState::Pressed);
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
        .test_value();
    context
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    context.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(!context.runtime_help_visible);
    assert!(context.context_menu.is_some());

    let board_script = r#"global func Initialize()
        {
            SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
        }"#;
    let mut default_scoreboard = new_classic_scoreboard_test_app(board_script);
    toggle_scoreboard(&mut default_scoreboard, ModifiersState::empty());
    let mut scoreboard_only = vec![0_u8; 320 * 200 * 4];
    default_scoreboard.test_render(&mut scoreboard_only);
    default_scoreboard.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    let mut scoreboard_and_help = vec![0_u8; 320 * 200 * 4];
    default_scoreboard.test_render(&mut scoreboard_and_help);
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
    scoreboard.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(!scoreboard.runtime_help_visible);
    assert!(scoreboard.scoreboard_dialog.is_some());

    let mut game_over = new_game_over_keyboard_app();
    game_over
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    game_over.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(game_over.runtime_help_visible);
}

#[test]
fn running_context_menu_renders_above_runtime_f1_help() {
    let mut app = new_classic_running_sandbox_app();
    app.status_text.clear();
    app.snapshot.hud.messages.clear();
    let mut baseline = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut baseline);
    app.open_context_menu_at(
        vec![ContextMenuEntry::<AppContextMenuCommand>::new(
            "Context above help",
        )],
        GuiPoint::new(120.0, 105.0),
    )
    .test_value();
    let mut context_only = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut context_only);
    assert_ne!(context_only, baseline, "running context must draw pixels");

    app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    let context = app.context_menu.take().test_value();
    let mut help_only = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut help_only);
    let mut expected = Surface::new(320, 200, PixelFormat::Rgba8888);
    expected.pixels_mut().copy_from_slice(&help_only);
    let gamma = app
        .graphics
        .active_gamma_ramp(&app.snapshot.environment.gamma);
    context.render(&mut expected, Some(&gamma)).test_value();
    app.context_menu = Some(context);
    let mut help_and_context = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut help_and_context);
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
        .test_player_mut(rebound.local_owner)
        .control
        .control_style = true;
    let mut menu_only = vec![0_u8; 320 * 200 * 4];
    let mut menu_and_help = vec![0_u8; 320 * 200 * 4];
    for style in 0..=3 {
        for text_progressing in [false, true] {
            let cursor = app.engine.test_crew_cursor(app.local_owner);
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
                .test_value();
            app.snapshot = app.engine.snapshot();
            menu_only.fill(0);
            app.test_render(&mut menu_only);
            app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
            menu_and_help.fill(0);
            app.test_render(&mut menu_and_help);
            assert!(
                app.runtime_help_visible,
                "style {style}, progress {text_progressing}"
            );
            assert_ne!(menu_and_help, menu_only);
            assert!(app.engine.cursor_object_menu(app.local_owner).is_some());
            app.test_key(VirtualKeyCode::F1, ElementState::Released);
            app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
            app.test_key(VirtualKeyCode::F1, ElementState::Released);
            assert!(!app.runtime_help_visible);

            let rebound_cursor = rebound.engine.test_crew_cursor(rebound.local_owner);
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
                .test_value();
            rebound.snapshot = rebound.engine.snapshot();
            rebound.test_key(VirtualKeyCode::F1, ElementState::Pressed);
            assert!(!rebound.runtime_help_visible);
            assert!(rebound
                .engine
                .cursor_object_menu(rebound.local_owner)
                .is_some());
            rebound.test_key(VirtualKeyCode::F1, ElementState::Released);
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
    active.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    assert!(active.runtime_client_list_strong_gamepad_callback_is_active());
    assert!(active.runtime_client_list_draw_active());

    active.test_gamepad_events([
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
    ]);
    let submitted = commands.take_submitted_local();
    assert_eq!(submitted.len(), 1);
    assert!(matches!(
        submitted[0].1,
        ControlEvent::Press(ControlButton::Right)
    ));

    active.test_gamepad_events([
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
    ]);
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
        .test_value();
    inactive.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    assert!(!inactive.runtime_client_list_strong_gamepad_callback_is_active());
    assert!(!inactive.runtime_client_list_draw_active());
    inactive.test_gamepad_events([GamepadEvent::GuiButton {
        slot: GamepadSlot::new(0),
        class: GuiButtonClass::High,
        state: ElementState::Pressed,
    }]);
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
        menu.test_key(key, ElementState::Pressed);
        menu.test_key(key, ElementState::Released);
    }
    menu.test_modifiers(ModifiersState::ALT);
    menu.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    menu.test_key(VirtualKeyCode::KeyC, ElementState::Released);

    let mut loading = new_running_sandbox_app();
    loading.mode = AppMode::Loading;
    for key in [
        VirtualKeyCode::F1,
        VirtualKeyCode::F4,
        VirtualKeyCode::Pause,
    ] {
        loading.test_key(key, ElementState::Pressed);
        loading.test_key(key, ElementState::Released);
    }
    loading.test_modifiers(ModifiersState::ALT);
    loading.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    loading.test_key(VirtualKeyCode::KeyC, ElementState::Released);
}

#[test]
fn window_close_confirms_running_round_and_nonrunning_close_exits() {
    let mut app = new_running_sandbox_app();
    app.test_update();
    let running_frame = app.engine.frame();
    let running_scenario = app.active_scenario.test_ref().identifier.clone();

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
fn window_close_uses_observer_owner_and_never_exits_on_dialog_refusal() {
    let mut observer = new_running_sandbox_app();
    let removed_owner = observer.local_owner;
    observer.engine.remove_player(removed_owner).test_value();
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
fn bare_escape_opens_abort_confirmation_without_exiting() {
    clonk_logging::init();
    let mut app = new_running_sandbox_app();
    app.status_text.clear();
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

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
        .test_player_mut(app.local_owner)
        .control
        .pressed_coms = 1 << clonk_engine::COM_LEFT;
    let frozen_frame = app.engine.frame();

    assert!(app.show_abort_dialog(app.local_owner));
    assert_eq!(app.offline_halt_count, 2);
    assert!(app.runtime_halt_active());
    app.test_update();
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
    app.remove_message_dialog_at(index).test_value();
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
