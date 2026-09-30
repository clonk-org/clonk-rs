#[test]
fn unified_settings_retains_the_original_options_backdrop_title_paper_and_back_button() {
    use clonk_frontend::classic_gui::IntRect;
    use clonk_frontend::settings_overlay::SettingsCategory;
    use clonk_frontend::startup_options_dlg::{
        options_dlg_layout, OptionsDlgScreen, OptionsDlgState, ProgramSheetState,
    };
    let mut app = new_real_classic_menu_app(1280, 720);
    app.app_paths = None;
    let fonts = app.assets.clonk_fonts.clone().unwrap();
    let book = app.assets.options_book_fonts.clone().unwrap();
    let assets = app
        .assets
        .options_dlg_assets(crate::settings::CompatProfile::Normal)
        .unwrap();
    let mut original = Surface::new(1280, 720, PixelFormat::Rgba8888);
    OptionsDlgScreen::render_state(
        &mut original,
        &assets,
        &fonts,
        &book,
        &OptionsDlgState::new(ProgramSheetState::default()),
        None,
    );
    app.open_unified_settings(SettingsCategory::Interface)
        .unwrap();
    let mut unified = Surface::new(1280, 720, PixelFormat::Rgba8888);
    assert!(app.render_unified_settings_to_surface(&mut unified, None));
    let layout = options_dlg_layout(1280, 720, &fonts, &book);
    // C4GuiDialogs.cpp:834-849, C4StartupOptionsDlg.cpp:655-657 and
    // C4GuiTabular.cpp:455 pin the original title, Back button and paper.
    for (name, rect) in [
        ("mine background", IntRect::new(24, 180, 120, 220)),
        ("Options title", layout.title_label),
        ("Back button", layout.back_button),
        (
            "paper edge",
            IntRect::new(
                layout.paper.x + layout.paper.w - 18,
                layout.paper.y + 24,
                18,
                layout.paper.h - 48,
            ),
        ),
    ] {
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let offset = (y as usize * 1280 + x as usize) * 4;
                assert_eq!(
                    &unified.pixels()[offset..offset + 4],
                    &original.pixels()[offset..offset + 4],
                    "{name} must retain its native artwork at {x},{y}"
                );
            }
        }
    }
}

#[test]
fn unified_settings_book_keeps_controls_clear_of_tabs_in_compact_windows() {
    use clonk_frontend::settings_overlay::SettingsCategory;
    let mut app = new_real_classic_menu_app(640, 480);
    app.app_paths = None;
    app.open_unified_settings(SettingsCategory::Interface)
        .unwrap();
    for (width, height) in [(640, 480), (800, 600), (1280, 720)] {
        app.resize(width, height).unwrap();
        let mut frame = vec![0; width as usize * height as usize * 4];
        app.render(&mut frame).unwrap();
        let controller = &mut app.unified_settings.as_mut().unwrap().controller;
        let layout = controller.layout();
        // StartupTabClip's active metal clasp extends across the paper's edge.
        assert!(
            layout.list.x >= layout.tabs[0].x + 120 + 8,
            "controls must clear the clasp at {width}x{height}"
        );
        assert!(
            layout.list.h >= 4 * 36,
            "at least four settings remain visible"
        );
        assert!(layout.footer.y + layout.footer.h < layout.back.y);
        for (category, tab) in SettingsCategory::ALL.into_iter().zip(layout.tabs) {
            let point = GuiPoint::new((tab.x + tab.w / 2) as f32, (tab.y + tab.h / 2) as f32);
            controller.pointer(point, true);
            controller.pointer(point, false);
            assert_eq!(
                controller.category, category,
                "every illustrated tab must be clickable"
            );
        }
        controller.select_audio_page(clonk_frontend::settings_overlay::AudioPage::Voice);
        assert_eq!(controller.visible_indices().len(), 6);
        assert!(
            controller.layout().list.h >= 6 * 28,
            "the complete basic microphone setup must fit at {width}x{height}"
        );
    }
}

#[test]
fn unified_settings_keeps_the_screen_and_releases_only_its_own_offline_pause() {
    use clonk_frontend::settings_overlay::SettingsCategory;
    let mut app = new_classic_running_sandbox_app();
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    let screen = app.startup.view;
    app.netplay.offline_halt_count = 1;
    app.open_unified_settings(SettingsCategory::Quick).unwrap();
    assert!(app.unified_settings.is_some());
    assert_eq!(app.mode, AppMode::Running);
    assert_eq!(app.startup.view, screen);
    assert_eq!(app.netplay.offline_halt_count, 2);
    app.close_unified_settings();
    assert_eq!(app.netplay.offline_halt_count, 1);
}

#[test]
fn unified_settings_recording_default_updates_the_game_about_to_start() {
    use clonk_frontend::settings_overlay::{SettingsAction, SettingsCategory};
    use clonk_frontend::startup_options_advanced::AdvancedConfigValue;
    let mut app = new_real_classic_menu_app(800, 600);
    app.app_paths = None;
    app.open_scenario_browser();
    app.records.directory = Some(PathBuf::from("Records"));
    app.records.enabled = false;
    app.open_unified_settings(SettingsCategory::Game).unwrap();
    let index = app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .settings
        .iter()
        .position(|s| s.id.section == "General" && s.id.key == "Record")
        .unwrap();
    app.process_unified_settings_actions(vec![SettingsAction::Change(
        index,
        AdvancedConfigValue::Bool(true),
    )])
    .unwrap();
    assert!(
        app.records.enabled,
        "the next game must use the new recording default"
    );
    assert!(
        app.scenario_game_options.values().record,
        "the selector must show the same preference"
    );
}

#[test]
fn unified_settings_cancels_the_underlying_pointer_capture() {
    use clonk_frontend::settings_overlay::SettingsCategory;
    let mut app = new_real_classic_menu_app(800, 600);
    app.app_paths = None;
    let button = clonk_frontend::main_menu_layout(800, 600).buttons[5];
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    ))
    .unwrap();
    app.handle_mouse_button(ElementState::Pressed).unwrap();
    assert!(!app.take_exit_request());
    app.open_unified_settings(SettingsCategory::Quick).unwrap();
    app.close_unified_settings();
    app.handle_mouse_button(ElementState::Released).unwrap();
    assert!(
        !app.take_exit_request(),
        "releasing a pre-overlay press must not activate Exit"
    );
}

#[test]
fn unified_settings_microphone_test_stays_local_and_stops_when_leaving_audio() {
    use clonk_frontend::settings_overlay::{SettingsAction, SettingsCategory};
    let mut app = new_classic_running_sandbox_app();
    app.app_paths = None;
    app.test_audio_mut().options.voice_enabled = false;
    app.open_unified_settings(SettingsCategory::Audio).unwrap();
    assert!(app.voice_setup.as_ref().unwrap().test.is_none());
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = cancelled.clone();
    app.voice_setup.as_mut().unwrap().start_test =
        Box::new(move |_, _| Ok(Box::new(LocalTestProbe(observed.clone()))));
    app.process_unified_settings_actions(vec![SettingsAction::TestMicrophone])
        .unwrap();
    assert!(app.voice_setup.as_ref().unwrap().test.is_some());
    assert!(!app.test_audio_mut().options.voice_enabled);
    assert!(app.runtime_gui_has_keyboard_focus());
    app.unified_settings
        .as_mut()
        .unwrap()
        .controller
        .select_category(SettingsCategory::Display);
    app.update_voice_setup();
    assert!(cancelled.get());
    assert!(app.voice_setup.as_ref().unwrap().test.is_none());
    assert!(app.unified_settings.is_some());
}

#[test]
fn unified_settings_stops_the_local_microphone_test_when_returning_to_sound() {
    use clonk_frontend::settings_overlay::{AudioPage, SettingsAction, SettingsFocus};
    let mut app = new_classic_running_sandbox_app();
    app.app_paths = None;
    app.test_audio_mut().options.voice_enabled = false;
    app.open_unified_voice_settings().unwrap();
    let controller = &mut app.unified_settings.as_mut().unwrap().controller;
    controller.set_focus(SettingsFocus::TestMicrophone);
    assert!(controller
        .key(clonk_frontend::KeyCode::Enter, false, false)
        .is_empty());
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = cancelled.clone();
    app.voice_setup.as_mut().unwrap().start_test =
        Box::new(move |_, _| Ok(Box::new(LocalTestProbe(observed.clone()))));
    app.process_unified_settings_actions(vec![SettingsAction::TestMicrophone])
        .unwrap();
    app.update_voice_setup();
    assert!(
        !cancelled.get(),
        "the visible test panel owns the recording"
    );
    app.update_unified_settings(Instant::now());
    assert!(
        app.unified_settings
            .as_ref()
            .unwrap()
            .controller
            .view
            .microphone_testing
    );
    capture_unified_settings_fixture(&mut app, "game-microphone-recording");
    app.unified_settings
        .as_mut()
        .unwrap()
        .controller
        .select_audio_page(AudioPage::Sound);
    app.update_voice_setup();
    assert!(
        cancelled.get(),
        "leaving the test must stop local capture even within Audio"
    );
    assert!(app.voice_setup.as_ref().unwrap().test.is_none());
    assert!(!app.test_audio_mut().options.voice_enabled);
}

#[test]
fn unified_settings_controller_category_button_consumes_all_of_its_aliases() {
    use clonk_frontend::settings_overlay::SettingsCategory;
    let mut app = new_classic_running_sandbox_app();
    app.app_paths = None;
    app.config.gamepad_input_enabled = true;
    app.open_unified_settings(SettingsCategory::Audio).unwrap();
    let slot = GamepadSlot::from_index(0).unwrap();
    app.process_sourced_gamepad_event_batch(
        [
            SourcedGamepadEvent {
                gamepad: 0,
                cluster: 1,
                event: GamepadEvent::Button {
                    slot,
                    button: LegacyGamepadButton::new(4),
                    state: ElementState::Pressed,
                },
            },
            SourcedGamepadEvent {
                gamepad: 0,
                cluster: 1,
                event: GamepadEvent::GuiButton {
                    slot,
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                },
            },
        ],
        true,
    )
    .unwrap();
    assert_eq!(
        app.unified_settings
            .as_ref()
            .expect("alias must not close settings")
            .controller
            .category,
        SettingsCategory::Display
    );
}

#[test]
fn unconfirmed_display_options_never_reach_the_config_file() {
    let files = tempfile::tempdir().unwrap();
    let (_guard, paths) = exact_loader_test_paths(files.path(), None);
    paths.ensure_user_dirs().unwrap();
    let original = "[Graphics]\nScale=100\nResolutionX=800\nResolutionY=600\nDisplayMode=Window\n";
    fs::write(paths.config_file(), original).unwrap();
    let mut options = DisplayOptions::load(Some(&paths));
    options.begin_settings_preview();
    options.record_scale_percent(150, 1200, 900);
    options.persist_if_dirty(&paths);
    assert_eq!(fs::read_to_string(paths.config_file()).unwrap(), original);
    options.finish_settings_preview(false);
    assert_eq!(options.scale_percent(), 100);
    assert_eq!(options.actual_size(), (800, 600));
    options.begin_settings_preview();
    options.record_scale_percent(200, 1280, 960);
    options.finish_settings_preview(true);
    options.persist_if_dirty(&paths);
    assert_eq!(
        Config::load(paths.config_file())
            .unwrap()
            .get_in(Some("Graphics"), "Scale"),
        Some("200")
    );
}

#[test]
fn unified_settings_keeps_unsaved_edits_available_after_a_save_failure() {
    use clonk_frontend::settings_overlay::{SettingsAction, SettingsCategory};
    use clonk_frontend::startup_options_advanced::AdvancedConfigValue;
    let mut app = new_classic_running_sandbox_app();
    let files = tempfile::tempdir().unwrap();
    let (_guard, paths) = exact_loader_test_paths(files.path(), None);
    paths.ensure_user_dirs().unwrap();
    fs::remove_file(paths.config_file()).unwrap();
    fs::create_dir(paths.config_file()).unwrap();
    app.app_paths = Some(paths.clone());
    app.open_unified_settings(SettingsCategory::Audio).unwrap();
    let index = app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .settings
        .iter()
        .position(|row| row.id.section == "Sound" && row.id.key == "SoundVolume")
        .unwrap();
    app.process_unified_settings_actions(vec![SettingsAction::Change(
        index,
        AdvancedConfigValue::Integer {
            value: 17,
            min: 0,
            max: 100,
        },
    )])
    .unwrap();
    app.close_unified_settings();
    assert!(app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .view
        .message
        .contains("Could not save"));
    assert_eq!(app.config.deferred.get("Sound", "SoundVolume"), Some("17"));
    fs::remove_dir(paths.config_file()).unwrap();
    app.close_unified_settings();
    assert!(app.unified_settings.is_none());
    assert_eq!(
        Config::load(paths.config_file())
            .unwrap()
            .get_in(Some("Sound"), "SoundVolume"),
        Some("17")
    );
}

fn capture_unified_settings_fixture(app: &mut GameApp, name: &str) {
    let Ok(directory) = std::env::var("CLONK_SETTINGS_IMAGES") else {
        return;
    };
    fs::create_dir_all(&directory).unwrap();
    for (width, height) in [(640, 480), (800, 600), (1280, 720)] {
        app.resize(width, height).unwrap();
        let mut frame = vec![0; width as usize * height as usize * 4];
        app.render(&mut frame).unwrap();
        image::save_buffer(
            Path::new(&directory).join(format!("{name}-{width}x{height}.png")),
            &frame,
            width,
            height,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}

#[test]
fn unified_settings_exposes_current_match_values_without_giving_clients_host_authority() {
    use clonk_frontend::settings_overlay::{SettingsAction, SettingsCategory};
    use clonk_frontend::startup_options_advanced::AdvancedConfigValue;
    let mut app = new_classic_running_sandbox_app();
    app.app_paths = None;
    let (network, _events) = NetworkManager::test_stub_for_client_id(1);
    app.netplay.manager = Some(network);
    app.netplay.mode = Some(NetworkMode::Client(ClientSettings::new(
        "127.0.0.1:11111".parse().unwrap(),
        "Settings client",
    )));
    app.engine.set_control_host(false);
    let rate = app.engine.control_rate();
    app.open_unified_settings(SettingsCategory::Game).unwrap();
    assert_eq!(app.netplay.offline_halt_count, 0);
    let rows = &app.unified_settings.as_ref().unwrap().controller.settings;
    let index = rows
        .iter()
        .position(|row| row.id.section == "Session" && row.id.key == "ControlRate")
        .expect("current match control rate");
    assert!(rows[index].details.unavailable.is_some());
    let mut proposed = rows[index].value.clone();
    if let AdvancedConfigValue::Choice { value, .. } = &mut proposed {
        *value = "9".into();
    }
    app.process_unified_settings_actions(vec![SettingsAction::Change(index, proposed)])
        .unwrap();
    assert_eq!(app.engine.control_rate(), rate);
    assert!(app.config.deferred.get("Session", "ControlRate").is_none());
}

#[test]
fn unified_settings_display_preview_reverts_on_timeout_without_saving_the_candidate() {
    use clonk_frontend::settings_overlay::{SettingsAction, SettingsCategory};
    use clonk_frontend::startup_options_advanced::AdvancedConfigValue;
    let mut app = new_classic_running_sandbox_app();
    app.app_paths = None;
    app.resize(1280, 960).unwrap();
    app.open_unified_settings(SettingsCategory::Display)
        .unwrap();
    let index = app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .settings
        .iter()
        .position(|s| s.id.section == "Graphics" && s.id.key == "Scale")
        .unwrap();
    let before = app.unified_settings.as_ref().unwrap().controller.settings[index]
        .value
        .clone();
    app.process_unified_settings_actions(vec![SettingsAction::Change(
        index,
        AdvancedConfigValue::Integer {
            value: 125,
            min: 100,
            max: 400,
        },
    )])
    .unwrap();
    assert!(app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .view
        .display_confirmation
        .is_some());
    assert!(app.config.deferred.get("Graphics", "Scale").is_none());
    app.update_unified_settings(Instant::now() + Duration::from_secs(30));
    let controller = &app.unified_settings.as_ref().unwrap().controller;
    assert_eq!(controller.settings[index].value, before);
    assert!(controller.view.display_confirmation.is_none());
    assert!(app.config.deferred.get("Graphics", "Scale").is_none());
}

#[test]
fn unified_settings_entry_points_open_the_same_overlay_without_changing_context() {
    use clonk_frontend::settings_overlay::{AudioPage, SettingsCategory, SettingsFocus};
    let mut app = new_real_classic_menu_app(800, 600);
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    app.app_paths = None;
    let before = app.startup.view;
    app.handle_main_menu_activation(MainMenuItem::Options)
        .unwrap();
    assert!(app.unified_settings.is_some());
    assert_eq!(
        app.unified_settings.as_ref().unwrap().controller.category,
        SettingsCategory::Interface
    );
    assert_eq!(app.startup.view, before);
    capture_unified_settings_fixture(&mut app, "menu");
    app.unified_settings
        .as_mut()
        .unwrap()
        .controller
        .select_audio_page(AudioPage::Sound);
    capture_unified_settings_fixture(&mut app, "menu-sound");
    app.unified_settings
        .as_mut()
        .unwrap()
        .controller
        .select_audio_page(AudioPage::Voice);
    capture_unified_settings_fixture(&mut app, "menu-voice");
    app.unified_settings
        .as_mut()
        .unwrap()
        .controller
        .show_advanced = true;
    capture_unified_settings_fixture(&mut app, "menu-voice-advanced");
    let controller = &mut app.unified_settings.as_mut().unwrap().controller;
    controller.show_advanced = false;
    controller.set_focus(SettingsFocus::TestMicrophone);
    controller.key(clonk_frontend::KeyCode::Enter, false, false);
    capture_unified_settings_fixture(&mut app, "menu-microphone-test");
    app.close_unified_settings();
    app.apply_classic_startup_screen("options");
    assert!(app.unified_settings.is_some());
    assert_eq!(app.startup.view, before);
    let mut app = new_classic_running_sandbox_app();
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    app.app_paths = None;
    app.apply_ingame_menu_action_for_player(0, MenuAction::VoiceSetup)
        .unwrap();
    assert_eq!(
        app.unified_settings.as_ref().unwrap().controller.category,
        SettingsCategory::Audio
    );
    assert_eq!(
        app.unified_settings.as_ref().unwrap().controller.audio_page,
        AudioPage::Voice,
        "voice entry points must go straight to microphone setup"
    );
    capture_unified_settings_fixture(&mut app, "game-audio");
}

#[test]
fn unified_settings_binding_capture_rejects_conflicts_and_consumes_the_new_key() {
    use clonk_frontend::settings_overlay::{SettingsAction, SettingsCategory};
    let mut app = new_classic_running_sandbox_app();
    app.app_paths = None;
    app.open_unified_settings(SettingsCategory::Controls)
        .unwrap();
    let index = app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .settings
        .iter()
        .position(|s| s.id.section == "Controls" && s.id.key == "Kbd1Key1")
        .unwrap();
    let duplicate = app
        .bindings
        .key_for_set(0, ControlBindingId::CursorRight)
        .unwrap();
    app.process_unified_settings_actions(vec![SettingsAction::CaptureBinding(index)])
        .unwrap();
    app.handle_key(duplicate, ElementState::Pressed).unwrap();
    assert_eq!(app.unified_settings.as_ref().unwrap().binding, Some(index));
    assert!(app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .view
        .message
        .contains("Already assigned"));
    app.handle_key(VirtualKeyCode::F12, ElementState::Pressed)
        .unwrap();
    assert_eq!(app.unified_settings.as_ref().unwrap().binding, None);
    assert_eq!(
        app.bindings.key_for_set(0, ControlBindingId::CursorLeft),
        Some(VirtualKeyCode::F12)
    );
    assert!(app.pending_screenshots.is_empty());
}

#[test]
fn unified_settings_applies_live_values_and_merges_saved_changes_without_losing_extensions() {
    use clonk_frontend::settings_overlay::{SettingsAction, SettingsCategory};
    use clonk_frontend::startup_options_advanced::AdvancedConfigValue;
    let mut app = new_classic_running_sandbox_app();
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    let files = tempfile::tempdir().unwrap();
    let (_guard, paths) = exact_loader_test_paths(files.path(), None);
    paths.ensure_user_dirs().unwrap();
    fs::write(paths.config_file(), "[Vendor]\nExtension=keep\n").unwrap();
    app.app_paths = Some(paths.clone());
    app.open_unified_settings(SettingsCategory::Audio).unwrap();
    let index = app
        .unified_settings
        .as_ref()
        .unwrap()
        .controller
        .settings
        .iter()
        .position(|s| s.id.section == "Sound" && s.id.key == "SoundVolume")
        .unwrap();
    app.process_unified_settings_actions(vec![SettingsAction::Change(
        index,
        AdvancedConfigValue::Integer {
            value: 23,
            min: 0,
            max: 100,
        },
    )])
    .unwrap();
    assert_eq!(app.test_audio_mut().options.sound_volume_percent(), 23);
    assert_eq!(
        Config::load(paths.config_file())
            .unwrap()
            .get_in(Some("Sound"), "SoundVolume"),
        None
    );
    fs::write(
        paths.config_file(),
        "[Vendor]\nExtension=changed externally\n",
    )
    .unwrap();
    app.close_unified_settings();
    assert!(app.unified_settings.is_none());
    let saved = Config::load(paths.config_file()).unwrap();
    assert_eq!(saved.get_in(Some("Sound"), "SoundVolume"), Some("23"));
    assert_eq!(
        saved.get_in(Some("Vendor"), "Extension"),
        Some("changed externally")
    );
    app.open_unified_settings(SettingsCategory::Quick).unwrap();
    assert_eq!(
        app.unified_settings.as_ref().unwrap().controller.settings[index]
            .value
            .serialized(),
        "23"
    );
}

#[test]
fn unified_settings_shortcut_search_and_escape_stay_in_the_current_screen() {
    let mut app = new_classic_running_sandbox_app();
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    app.app_paths = None;
    app.handle_modifiers_changed(ModifiersState::CONTROL)
        .unwrap();
    app.handle_key(VirtualKeyCode::Comma, ElementState::Pressed)
        .unwrap();
    assert!(app.unified_settings.is_some());
    app.handle_modifiers_changed(ModifiersState::empty())
        .unwrap();
    app.unified_settings
        .as_mut()
        .unwrap()
        .controller
        .focus_search();
    for character in "microphone".chars() {
        app.handle_text_input(character).unwrap();
    }
    let controller = &app.unified_settings.as_ref().unwrap().controller;
    assert_eq!(controller.query, "microphone");
    assert!(!controller.visible_indices().is_empty());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .unwrap();
    assert!(app.unified_settings.is_some());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .unwrap();
    assert!(app.unified_settings.is_none());
    assert_eq!(app.netplay.offline_halt_count, 0);
    assert_eq!(app.mode, AppMode::Running);
    assert!(!app.running_chat_active());
}
