// Port-only presentation UI: opening setup must never transmit speech.
#[test]
fn voice_options_are_part_of_the_options_book_without_a_floating_launcher() {
    let mut app = new_classic_running_sandbox_app();
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    app.mode = AppMode::Menu;
    app.open_options_menu();
    assert!(app.voice_setup_launcher().is_none());
    assert!(app
        .startup
        .options_dialog
        .as_ref()
        .unwrap()
        .voice()
        .is_some());
}

#[test]
fn voice_options_use_native_navigation_and_close_the_microphone_on_tab_exit() {
    use clonk_frontend::startup_options_dlg::{
        OptionsDlgAction, OptionsSheet, VoiceOptionsAction, VoiceOptionsControl,
    };
    let mut app = new_classic_running_sandbox_app();
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    app.mode = AppMode::Menu;
    app.open_options_menu();
    app.open_voice_setup().unwrap();
    assert_eq!(
        app.startup.options_dialog.as_ref().unwrap().active_sheet(),
        OptionsSheet::Voice
    );
    assert!(!app.voice_setup_is_modal());
    assert!(app.voice_setup.as_ref().unwrap().test.is_none());
    assert!(!app
        .voice_setup_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .unwrap());
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    app.voice_setup.as_mut().unwrap().test = Some(Box::new(LocalTestProbe(cancelled.clone())));
    app.process_voice_options_action(VoiceOptionsAction::Activate(VoiceOptionsControl::Noise))
        .unwrap();
    assert!(cancelled.get());
    assert_eq!(
        app.startup
            .options_dialog
            .as_ref()
            .unwrap()
            .voice()
            .unwrap()
            .noise,
        app.test_audio_mut().options.voice_noise_suppression
    );
    app.process_voice_options_action(VoiceOptionsAction::SetVolume(173))
        .unwrap();
    assert_eq!(app.test_audio_mut().options.voice_volume_percent(), 173);
    cancelled.set(false);
    app.voice_setup.as_mut().unwrap().test = Some(Box::new(LocalTestProbe(cancelled.clone())));
    let actions = app
        .startup
        .options_dialog
        .as_mut()
        .unwrap()
        .handle_ctrl_tab(false);
    assert_eq!(
        actions,
        [OptionsDlgAction::SheetChanged(OptionsSheet::Keyboard)]
    );
    app.process_options_dialog_actions(actions).unwrap();
    assert!(cancelled.get());
    assert!(app.voice_setup.is_none());
}

#[test]
fn voice_options_pointer_test_and_key_binding_use_the_native_dialog() {
    use clonk_frontend::startup_options_dlg::{VoiceOptionsAction, VoiceOptionsControl};
    let mut app = new_classic_running_sandbox_app();
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    app.mode = AppMode::Menu;
    app.resize(800, 600).unwrap();
    app.open_options_menu();
    app.open_voice_setup().unwrap();
    app.startup.dialog_fade = None;
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = cancelled.clone();
    app.voice_setup.as_mut().unwrap().start_test =
        Box::new(move |_, _| Ok(Box::new(LocalTestProbe(seen.clone()))));
    let rect = app
        .startup
        .options_dialog
        .as_ref()
        .unwrap()
        .voice_control_bounds(VoiceOptionsControl::Test)
        .unwrap();
    app.handle_cursor_moved(PhysicalPosition::new(
        (rect.x + rect.w / 2) as f64,
        (rect.y + rect.h / 2) as f64,
    ))
    .unwrap();
    app.handle_mouse_button(ElementState::Pressed).unwrap();
    app.handle_mouse_button(ElementState::Released).unwrap();
    assert!(app.voice_setup.as_ref().unwrap().test.is_some());
    app.process_voice_options_action(VoiceOptionsAction::Activate(
        VoiceOptionsControl::PushToTalk,
    ))
    .unwrap();
    assert!(cancelled.get());
    app.handle_key(VirtualKeyCode::KeyB, ElementState::Pressed)
        .unwrap();
    app.handle_key(VirtualKeyCode::KeyB, ElementState::Released)
        .unwrap();
    assert_eq!(
        app.test_audio_mut().options.voice_push_to_talk,
        VirtualKeyCode::KeyB
    );
    assert!(app.dialogs.messages.is_empty());
    app.update_voice_setup();
    // Closing the key-capture dialog can start a return fade; capture the
    // settled options page, independently of asset-loading timing.
    app.startup.dialog_fade = None;
    let mut frame = vec![0; 800 * 600 * 4];
    app.render(&mut frame).unwrap();
    assert!(
        frame[(400 * 800 + 400) * 4] > 140,
        "the options paper must be visible"
    );
    if let Ok(path) = std::env::var("CLONK_VOICE_OPTIONS_APP_IMAGE") {
        image::save_buffer(path, &frame, 800, 600, image::ColorType::Rgba8).unwrap();
    }
    app.process_voice_options_action(VoiceOptionsAction::Activate(VoiceOptionsControl::Input))
        .unwrap();
    assert!(app.context_menus.open.is_some());
    assert!(!app
        .voice_setup_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .unwrap());
}

#[test]
fn voice_setup_stops_live_capture_without_leaving_the_game() {
    let (mut app, mut voice) = n2_classic_voice_app(0);
    n2_enable_voice_activation(&mut app);
    app.voice_chat = crate::voice_service::VoiceChatService::with_source_opener(|_| {
        Ok(N2VoiceFrames::new(Vec::new()))
    });
    app.update_voice_chat();
    assert!(app.voice_chat.capture_active());
    app.open_voice_setup().unwrap();
    assert_eq!(app.mode, AppMode::Running);
    assert!(!app.voice_chat.capture_active());
    app.update_voice_chat();
    assert!(
        !app.voice_chat.capture_active(),
        "setup owns the microphone privacy boundary"
    );
    assert!(voice.try_recv_outbound().is_none());
}

struct LocalTestProbe(std::rc::Rc<std::cell::Cell<bool>>);
impl crate::game_app_voice_setup::LocalMicrophoneCheck for LocalTestProbe {
    fn cancel(&self) {
        self.0.set(true);
    }
    fn status(&self) -> clonk_audio::VoiceMicrophoneTestStatus {
        clonk_audio::VoiceMicrophoneTestStatus {
            state: clonk_audio::VoiceMicrophoneTestState::Recording,
            level: 0.6,
            remaining: Duration::from_secs(2),
        }
    }
}

#[test]
fn voice_setup_local_test_is_explicit_offline_and_cancelled_by_focus_loss() {
    use clonk_frontend::voice_setup::VoiceSetupControl;
    let mut app = new_classic_running_sandbox_app();
    app.test_audio_mut().options.voice_enabled = false;
    app.open_voice_setup().unwrap();
    assert!(app.voice_setup.as_ref().unwrap().test.is_none());
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = cancelled.clone();
    app.voice_setup.as_mut().unwrap().start_test =
        Box::new(move |_, _| Ok(Box::new(LocalTestProbe(seen.clone()))));
    app.voice_setup_action(VoiceSetupControl::Test).unwrap();
    assert!(app.voice_setup.as_ref().unwrap().test.is_some());
    assert!(!app.test_audio_mut().options.voice_enabled);
    assert!(!cancelled.get());
    app.handle_focus_lost().unwrap();
    assert!(cancelled.get());
    assert!(app.voice_setup.as_ref().unwrap().test.is_none());
    app.window_active = true;
    app.update_voice_chat();
    assert!(app.voice_setup.as_ref().unwrap().test.is_none());
}

#[test]
fn voice_setup_device_changes_close_tests_and_session_changes_close_setup() {
    use clonk_frontend::voice_setup::VoiceSetupControl;
    let mut app = new_classic_running_sandbox_app();
    app.open_voice_setup().unwrap();
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    app.voice_setup.as_mut().unwrap().test = Some(Box::new(LocalTestProbe(cancelled.clone())));
    app.voice_setup_action(VoiceSetupControl::Output).unwrap();
    assert!(cancelled.get());
    cancelled.set(false);
    app.voice_setup.as_mut().unwrap().test = Some(Box::new(LocalTestProbe(cancelled.clone())));
    app.clear_live_network_session();
    assert!(cancelled.get());
    assert!(app.voice_setup.is_none());
}

#[test]
fn voice_setup_routes_keyboard_without_transmitting_or_changing_game_mode() {
    let (mut app, mut voice) = n2_classic_voice_app(0);
    app.test_audio_mut().options.voice_enabled = true;
    app.open_voice_setup().unwrap();
    app.handle_key(VirtualKeyCode::Backquote, ElementState::Pressed)
        .unwrap();
    app.handle_key(VirtualKeyCode::Backquote, ElementState::Released)
        .unwrap();
    app.update_voice_chat();
    assert!(!app.voice_chat.capture_active());
    assert!(voice.try_recv_outbound().is_none());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .unwrap();
    assert!(app.voice_setup.is_none());
    assert_eq!(app.mode, AppMode::Running);
}

#[test]
fn voice_setup_renders_over_the_game_at_small_resolution() {
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).unwrap();
    let mut before = vec![0; 640 * 480 * 4];
    app.render(&mut before).unwrap();
    app.open_voice_setup().unwrap();
    let mut after = before.clone();
    app.render(&mut after).unwrap();
    assert_ne!(before, after);
    assert_eq!(app.mode, AppMode::Running);
    if let Ok(path) = std::env::var("CLONK_VOICE_SETUP_TEST_IMAGE") {
        image::save_buffer(path, &after, 640, 480, image::ColorType::Rgba8).unwrap();
    }
}

#[test]
fn closing_voice_setup_refreshes_the_underlying_audio_options() {
    use clonk_frontend::startup_options_dlg::SoundCheckboxId;
    use clonk_frontend::voice_setup::VoiceSetupControl;
    let mut app = new_classic_running_sandbox_app();
    app.test_audio_mut().options.voice_enabled = false;
    app.open_options_menu();
    app.open_voice_setup().unwrap();
    app.voice_setup_action(VoiceSetupControl::Enabled).unwrap();
    app.close_voice_setup();
    assert!(app
        .startup
        .options_dialog
        .as_ref()
        .unwrap()
        .sound()
        .checkbox(SoundCheckboxId::VoiceEnabled));
}

#[test]
fn a_client_negotiates_voice_before_microphone_opt_in() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let host = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        clonk_network::start_host(
            listener,
            clonk_network::HostConfig {
                udp_bind_address: Some("127.0.0.1:0".parse().unwrap()),
                ..clonk_network::HostConfig::default()
            },
        )
        .await
        .unwrap()
    });
    let address = host.udp_local_addr().unwrap();
    let files = tempfile::tempdir().unwrap();
    let mut settings = ClientSettings::new(address, "Voice setup test").with_join_attempts([
        clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Udp, address),
    ]);
    settings.resource_directory = files.path().to_owned();
    let app = GameApp::new(
        640,
        480,
        AudioOptions {
            voice_enabled: false,
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        },
        None,
        RuntimeConfig {
            player_owner: 1,
            player_name: "Voice setup test".into(),
            network: Some(NetworkMode::Client(settings)),
            record_enabled: false,
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !app.netplay.manager.as_ref().unwrap().voice_available() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let negotiated = app.netplay.manager.as_ref().unwrap().voice_available();
    assert!(!app.voice_chat_enabled());
    assert!(!app.voice_chat.capture_active());
    drop(app);
    runtime.block_on(host.shutdown()).unwrap();
    assert!(
        negotiated,
        "local microphone opt-in must not require reconnecting to negotiate UDP voice"
    );
}
