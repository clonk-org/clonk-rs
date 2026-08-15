// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

#[test]
fn push_to_talk_key_falls_through_in_an_offline_menu() {
    let mut app = new_classic_running_sandbox_app();
    app.audio.test_mut().options.voice_enabled = true;
    app.mode = AppMode::Menu;

    assert!(!app.handle_voice_key(VirtualKeyCode::Backquote, ElementState::Pressed));
}

#[test]
fn push_to_talk_opens_capture_in_a_network_lobby() {
    struct TestVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for TestVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            vec![clonk_audio::VoiceInputFrame {
                payload: clonk_audio::encode_voice_frame(
                    &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
                ),
                level: 1.0,
            }]
        }
    }

    let mut app = new_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Observer".to_string(), false));
    app.control_clients.register(7, false, true);
    let (manager, _events, mut voice) = NetworkManager::test_stub_with_voice_for_client_id(7);
    app.network = Some(manager);
    app.audio.test_mut().options.voice_enabled = true;
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(|_| Ok(TestVoiceSource));

    app.handle_key(VirtualKeyCode::Backquote, ElementState::Pressed)
        .test_value();
    assert!(app.voice_chat.capture_active());
    assert!(app.key_event_suppresses_text);
    app.update_voice_chat();
    let outbound = voice.try_recv_outbound().test_value();
    assert_eq!(outbound.player_id, crate::voice_chat::LOBBY_VOICE_PLAYER_ID);
    assert!(app.voice_chat.capture_active());
    app.handle_key(VirtualKeyCode::Backquote, ElementState::Released)
        .test_value();
    assert!(!app.voice_chat.capture_active());
}

#[test]
fn network_lobby_voice_activation_uses_a_client_scoped_wire_identity() {
    use std::cell::RefCell;

    struct OneFrameVoiceSource {
        frames: RefCell<Vec<clonk_audio::VoiceInputFrame>>,
    }

    impl crate::voice_chat::VoiceFrameSource for OneFrameVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            std::mem::take(&mut *self.frames.borrow_mut())
        }
    }

    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    let (manager, _events, mut voice) = NetworkManager::test_stub_with_voice_for_client_id(0);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    options.voice_activation_threshold = 0.0;
    let payload = clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES]);
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |_| {
        Ok(OneFrameVoiceSource {
            frames: RefCell::new(vec![clonk_audio::VoiceInputFrame {
                payload,
                level: 1.0,
            }]),
        })
    });
    let simulation_before_voice = app.engine.snapshot();
    let admitted_at = Instant::now();

    app.update_voice_chat_at(admitted_at);

    assert!(app.voice_chat.capture_active());
    assert_eq!(app.voice_chat.capture_key(), None);
    let outbound = voice.try_recv_outbound().test_value();
    assert_eq!(outbound.player_id, crate::voice_chat::LOBBY_VOICE_PLAYER_ID);
    assert!(app
        .voice_chat
        .active_speakers(admitted_at)
        .contains(&(0, crate::voice_chat::LOBBY_VOICE_PLAYER_ID)));
    assert_eq!(app.engine.snapshot(), simulation_before_voice);
}

#[test]
fn network_lobby_voice_plays_authenticated_clients_non_positionally() {
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    app.audio.test_mut().system = clonk_audio::AudioSystem::new_manual_with_resampling(
        8,
        clonk_audio::ResamplingMode::Linear,
    );
    app.audio.test_mut().options.voice_enabled = true;
    let (manager, _events, voice) = NetworkManager::test_stub_with_voice_for_client_id(0);
    app.network = Some(manager);
    let remote_client = 7;
    app.control_clients.register(remote_client, false, true);
    let admitted_at = Instant::now();
    let simulation_before_voice = app.engine.snapshot();
    for sequence in [0, 0, 1, 2, 3] {
        voice
            .send_inbound(clonk_network::VoiceFrame {
                client_id: remote_client as u32,
                player_id: crate::voice_chat::LOBBY_VOICE_PLAYER_ID,
                stream_epoch: 3,
                sequence,
                payload: clonk_audio::encode_voice_frame(
                    &[2_000; clonk_audio::VOICE_FRAME_SAMPLES],
                )
                .to_vec(),
            })
            .test_value();
    }

    app.update_voice_chat_at(admitted_at);

    let stream_id =
        crate::voice_chat::voice_stream_id(remote_client, crate::voice_chat::LOBBY_VOICE_PLAYER_ID);
    assert_eq!(
        app.audio
            .test_ref()
            .system
            .voice_stream_stats(stream_id)
            .queued_frames,
        4,
    );
    let mut mixed = [0_i16; 2];
    app.audio.test_ref().system.mixer().mix_i16(&mut mixed);
    assert_ne!(mixed, [0, 0]);
    assert_eq!(mixed[0], mixed[1], "lobby voice is centered");
    assert!(app
        .voice_chat
        .active_speakers(admitted_at)
        .contains(&(remote_client, crate::voice_chat::LOBBY_VOICE_PLAYER_ID)));
    assert_eq!(app.engine.snapshot(), simulation_before_voice);

    app.mode = AppMode::Loading;
    app.update_voice_chat_at(admitted_at + Duration::from_millis(20));
    assert_eq!(
        app.audio
            .test_ref()
            .system
            .voice_stream_stats(stream_id)
            .queued_frames,
        0,
        "an inactive transition must remove already-queued lobby speech",
    );

    app.mode = AppMode::Menu;
    for sequence in 0..=4 {
        voice
            .send_inbound(clonk_network::VoiceFrame {
                client_id: remote_client as u32,
                player_id: crate::voice_chat::LOBBY_VOICE_PLAYER_ID,
                stream_epoch: 3,
                sequence,
                payload: clonk_audio::encode_voice_frame(
                    &[2_000; clonk_audio::VOICE_FRAME_SAMPLES],
                )
                .to_vec(),
            })
            .test_value();
    }
    app.update_voice_chat_at(admitted_at + Duration::from_millis(40));
    assert_eq!(
        app.audio
            .test_ref()
            .system
            .voice_stream_stats(stream_id)
            .queued_frames,
        0,
        "the retained route must not replay lobby frames after an inactive transition",
    );
    assert!(app.voice_chat.remote_streams.is_empty());

    for sequence in 0..4 {
        voice
            .send_inbound(clonk_network::VoiceFrame {
                client_id: remote_client as u32,
                player_id: crate::voice_chat::LOBBY_VOICE_PLAYER_ID,
                stream_epoch: 4,
                sequence,
                payload: clonk_audio::encode_voice_frame(
                    &[2_000; clonk_audio::VOICE_FRAME_SAMPLES],
                )
                .to_vec(),
            })
            .test_value();
    }
    app.update_voice_chat_at(admitted_at + Duration::from_millis(60));
    assert_eq!(
        app.audio
            .test_ref()
            .system
            .voice_stream_stats(stream_id)
            .queued_frames,
        4,
        "a fresh capture epoch may speak after the retained transition",
    );
}

#[test]
fn network_lobby_voice_rejects_unknown_clients_and_non_lobby_scopes() {
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    app.audio.test_mut().options.voice_enabled = true;
    let (manager, _events, voice) = NetworkManager::test_stub_with_voice_for_client_id(0);
    app.network = Some(manager);
    let known_client = 7;
    app.control_clients.register(known_client, false, true);
    let admitted_at = Instant::now();
    for (client_id, player_id) in [
        (8, crate::voice_chat::LOBBY_VOICE_PLAYER_ID),
        (known_client, 17),
    ] {
        voice
            .send_inbound(clonk_network::VoiceFrame {
                client_id: client_id as u32,
                player_id,
                stream_epoch: 1,
                sequence: 0,
                payload: clonk_audio::encode_voice_frame(
                    &[2_000; clonk_audio::VOICE_FRAME_SAMPLES],
                )
                .to_vec(),
            })
            .test_value();
    }

    app.update_voice_chat_at(admitted_at);

    assert!(app.voice_chat.remote_streams.is_empty());
    assert!(app.voice_chat.active_speakers(admitted_at).is_empty());
}

#[test]
fn voice_capture_does_not_cross_the_game_and_lobby_identity_boundary() {
    struct SilentVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for SilentVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            Vec::new()
        }
    }

    let mut app = new_classic_running_sandbox_app();
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(0));
    app.snapshot = app.engine.snapshot();
    let (manager, _events, _voice) = NetworkManager::test_stub_with_voice_for_client_id(0);
    app.network = Some(manager);
    app.audio.test_mut().options.voice_enabled = true;
    app.voice_chat =
        crate::voice_chat::VoiceChatState::with_source_opener(|_| Ok(SilentVoiceSource));
    app.update_voice_chat();
    assert!(app.handle_voice_key(VirtualKeyCode::Backquote, ElementState::Pressed));
    assert!(app.voice_chat.capture_active());

    app.mode = AppMode::Menu;
    install_test_classic_host_lobby(&mut app);
    app.update_voice_chat();

    assert!(
        !app.voice_chat.capture_active(),
        "a held in-game capture must not silently become lobby speech",
    );
}

#[test]
fn push_to_talk_and_remote_playback_cross_the_game_runtime_voice_seam() {
    use std::cell::RefCell;

    struct TestVoiceSource {
        frames: RefCell<Vec<clonk_audio::VoiceInputFrame>>,
    }

    impl crate::voice_chat::VoiceFrameSource for TestVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            std::mem::take(&mut *self.frames.borrow_mut())
        }
    }

    fn mix_frames(system: &clonk_audio::AudioSystem, frames: usize) -> Vec<i16> {
        let mut output = vec![0; frames.saturating_mul(2)];
        system.mixer().mix_i16(&mut output);
        output
    }

    let mut app = new_classic_running_sandbox_app();
    app.audio.test_mut().system = clonk_audio::AudioSystem::new_manual_with_resampling(
        8,
        clonk_audio::ResamplingMode::Linear,
    );
    let local_client = 7;
    let local_player = app.local_owner;
    app.engine
        .test_player_mut(local_player)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    let local_cursor = app
        .engine
        .player(local_player)
        .and_then(|player| player.cursor())
        .test_value();

    let remote_client = 3;
    let remote_player = 17;
    app.engine
        .register_player(PlayerConfig::new(remote_player, "Remote"))
        .test_value();
    app.engine
        .test_player_mut(remote_player)
        .set_at_client(clonk_engine::PlayerAtClient::new(remote_client));
    let remote_container = app.engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_owner(remote_player)
            .with_position(Vector2::new(160, 100)),
    );
    let remote_cursor = app.engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_owner(remote_player)
            .with_position(Vector2::new(160, 100))
            .with_container(remote_container),
    );
    app.engine
        .test_player_mut(remote_player)
        .set_cursor(Some(remote_cursor));
    app.snapshot = app.engine.snapshot();
    assert_eq!(
        app.snapshot
            .object(remote_cursor)
            .and_then(|object| object.container),
        Some(remote_container),
        "the runtime seam exercises a speaking selected crew inside a container",
    );
    let viewport_inputs = collect_viewport_inputs(&app.snapshot).test_value();
    app.graphics.render_frame(&app.snapshot, &viewport_inputs);
    let remote_position = crate::voice_chat::authenticated_selected_voice_crew(
        &app.snapshot,
        remote_client,
        remote_player,
    )
    .test_value()
    .position;
    let (remote_audibility, remote_pan) = compute_object_positional_mix(
        remote_position,
        &app.snapshot,
        &app.graphics.active_viewport_projections(),
    );
    let remote_volume = remote_audibility * app.audio.test_ref().options.voice_volume;
    let reference_audio = clonk_audio::AudioSystem::new_manual_with_resampling(
        8,
        clonk_audio::ResamplingMode::Linear,
    );
    let mut reference_voice = crate::voice_chat::VoiceChatState::default();

    let (manager, _events, mut voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.audio.test_mut().options.voice_enabled = true;
    let captured = clonk_audio::VoiceInputFrame {
        payload: clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES]),
        level: 1.0,
    };
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |_| {
        Ok(TestVoiceSource {
            frames: RefCell::new(vec![captured]),
        })
    });

    app.handle_key(VirtualKeyCode::Backquote, ElementState::Pressed)
        .test_value();
    assert!(app.voice_chat.capture_active());
    assert!(app.key_event_suppresses_text);
    app.update_voice_chat();

    let outbound = voice.try_recv_outbound().test_value();
    assert_eq!(outbound.player_id, local_player);
    assert_eq!(outbound.stream_epoch, 1);
    assert_eq!(outbound.sequence, 0);
    let local_activity = app.voice_chat.active_speakers(Instant::now());
    assert!(local_activity.contains(&(local_client, local_player)));
    assert_eq!(
        collect_speaking_overlay_objects(&app.snapshot, &local_activity),
        vec![local_cursor],
    );

    app.window_active = false;
    app.handle_focus_lost().test_value();
    assert!(
        !app.voice_chat.capture_active(),
        "focus loss must close an active push-to-talk capture",
    );

    let simulation_before_remote_voice = app.engine.snapshot();
    let admission_started_at = Instant::now();
    let batched_admission_at = admission_started_at + Duration::from_millis(65);
    for sequence in [0, 2, 1, 3] {
        let inbound = clonk_network::VoiceFrame {
            client_id: remote_client as u32,
            player_id: remote_player,
            stream_epoch: 4,
            sequence,
            payload: clonk_audio::encode_voice_frame(
                &[2_000 + sequence as i16; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .to_vec(),
        };
        assert!(reference_voice
            .accept_remote_frame(&app.snapshot, &inbound, batched_admission_at)
            .is_some());
        voice.send_inbound(inbound).test_value();
    }
    app.update_voice_chat_at(batched_admission_at);

    let stream_id = crate::voice_chat::voice_stream_id(remote_client, remote_player);
    let first_reference_frames = reference_voice.drain_remote_playout(
        remote_client,
        remote_player,
        batched_admission_at,
        clonk_audio::DEFAULT_VOICE_BUFFERED_FRAMES,
        0,
    );
    assert_eq!(
        first_reference_frames
            .iter()
            .map(|frame| (frame.sequence, frame.concealed))
            .collect::<Vec<_>>(),
        vec![(0, false), (1, false), (2, false), (3, false)],
    );
    for frame in first_reference_frames {
        reference_audio.queue_voice_stream_with_mix(
            stream_id,
            frame.samples,
            remote_volume,
            remote_pan,
        );
    }
    assert_eq!(
        app.audio
            .test_ref()
            .system
            .voice_stream_stats(stream_id)
            .queued_frames,
        4,
    );

    let sample_rate =
        usize::try_from(app.audio.test_ref().system.mixer().sample_rate()).unwrap_or(usize::MAX);
    let first_mix_frames = sample_rate.saturating_mul(35) / 1_000;
    let second_mix_frames = sample_rate.saturating_mul(20) / 1_000;
    let total_mix_frames = sample_rate.saturating_mul(120) / 1_000;
    let final_mix_frames = total_mix_frames
        .saturating_sub(first_mix_frames)
        .saturating_sub(second_mix_frames);
    let mut actual_output = mix_frames(&app.audio.test_ref().system, first_mix_frames);
    let mut expected_output = mix_frames(&reference_audio, first_mix_frames);

    let missing_successor = clonk_network::VoiceFrame {
        client_id: remote_client as u32,
        player_id: remote_player,
        stream_epoch: 4,
        sequence: 5,
        payload: clonk_audio::encode_voice_frame(&[2_005; clonk_audio::VOICE_FRAME_SAMPLES])
            .to_vec(),
    };
    assert!(reference_voice
        .accept_remote_frame(
            &app.snapshot,
            &missing_successor,
            admission_started_at + Duration::from_millis(100),
        )
        .is_some());
    voice.send_inbound(missing_successor).test_value();
    app.update_voice_chat_at(admission_started_at + Duration::from_millis(100));
    assert!(reference_voice
        .drain_remote_playout(
            remote_client,
            remote_player,
            admission_started_at + Duration::from_millis(100),
            2,
            3,
        )
        .is_empty());
    actual_output.extend(mix_frames(&app.audio.test_ref().system, second_mix_frames));
    expected_output.extend(mix_frames(&reference_audio, second_mix_frames));

    app.update_voice_chat_at(admission_started_at + Duration::from_millis(120));
    let final_reference_frames = reference_voice.drain_remote_playout(
        remote_client,
        remote_player,
        admission_started_at + Duration::from_millis(120),
        3,
        2,
    );
    assert_eq!(
        final_reference_frames
            .iter()
            .map(|frame| (frame.sequence, frame.concealed))
            .collect::<Vec<_>>(),
        vec![(4, true), (5, false)],
    );
    for frame in final_reference_frames {
        reference_audio.queue_voice_stream_with_mix(
            stream_id,
            frame.samples,
            remote_volume,
            remote_pan,
        );
    }
    actual_output.extend(mix_frames(&app.audio.test_ref().system, final_mix_frames));
    expected_output.extend(mix_frames(&reference_audio, final_mix_frames));

    assert_eq!(
        app.voice_chat
            .remote_playout_stats(remote_client, remote_player)
            .concealed_frames,
        1,
        "the runtime must consume the missing packet through loss concealment",
    );
    assert_eq!(
        actual_output, expected_output,
        "the runtime must queue the ordered and concealed PCM into the mixer",
    );
    let stereo_frames = expected_output.chunks_exact(2).collect::<Vec<_>>();
    assert!(stereo_frames
        .iter()
        .any(|frame| frame.iter().any(|&sample| sample != 0)));
    let playout_frame_samples = sample_rate.saturating_mul(20) / 1_000;
    assert!(stereo_frames
        .chunks(playout_frame_samples)
        .all(|frame| frame
            .iter()
            .any(|sample| sample.iter().any(|&value| value != 0))));
    let last_active_frame = stereo_frames
        .iter()
        .rposition(|frame| frame.iter().any(|&sample| sample != 0))
        .test_value();
    assert!(stereo_frames[..=last_active_frame].windows(2).all(|pair| {
        (i32::from(pair[1][0]) - i32::from(pair[0][0])).abs() <= 128
            && (i32::from(pair[1][1]) - i32::from(pair[0][1])).abs() <= 128
    }));
    assert_eq!(
        app.engine.snapshot(),
        simulation_before_remote_voice,
        "voice playout state must not become observable to the simulation",
    );
    let remote_activity = app
        .voice_chat
        .active_speakers(admission_started_at + Duration::from_millis(120));
    assert!(remote_activity.contains(&(remote_client, remote_player)));
    assert!(
        collect_speaking_overlay_objects(&app.snapshot, &remote_activity).contains(&remote_cursor)
    );

    app.snapshot
        .players
        .iter_mut()
        .find(|player| player.id == remote_player)
        .test_value()
        .cursor = None;
    app.update_voice_chat_at(admission_started_at + Duration::from_millis(200));
    assert!(
        !app.voice_chat
            .remote_streams
            .contains_key(&(remote_client, remote_player)),
        "invalidated ownership must discard pending remote playout",
    );

    app.audio.test_mut().options.voice_enabled = false;
    app.update_voice_chat();
    assert_eq!(
        app.audio.test_ref().system.voice_stream_stats(stream_id),
        clonk_audio::VoiceStreamStats::default(),
        "disabling voice live must remove buffered remote playback",
    );
    assert!(
        !app.voice_chat
            .active_speakers(Instant::now())
            .contains(&(remote_client, remote_player)),
        "disabling voice live must clear remote speaking activity",
    );
}

#[test]
fn voice_volume_action_scales_remote_playback_gain_above_unity() {
    use clonk_frontend::startup_options_dlg::{OptionsDlgAction, SoundSheetAction, SoundVolumeId};

    fn render_first_remote_sample(voice_volume: u8) -> i16 {
        let mut app = new_classic_running_sandbox_app();
        app.audio.test_mut().system = clonk_audio::AudioSystem::new_manual_with_resampling(
            8,
            clonk_audio::ResamplingMode::Linear,
        );
        let local_client = 7;
        let local_player = app.local_owner;
        app.engine
            .test_player_mut(local_player)
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        let local_position = app
            .engine
            .player(local_player)
            .and_then(|player| player.cursor())
            .and_then(|cursor| {
                app.engine
                    .snapshot()
                    .object(cursor)
                    .map(|object| object.position)
            })
            .test_value();

        let remote_client = 3;
        let remote_player = 17;
        app.engine
            .register_player(PlayerConfig::new(remote_player, "Remote"))
            .test_value();
        app.engine
            .test_player_mut(remote_player)
            .set_at_client(clonk_engine::PlayerAtClient::new(remote_client));
        let remote_cursor = app.engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(remote_player)
                .with_position(local_position),
        );
        app.engine
            .test_player_mut(remote_player)
            .set_cursor(Some(remote_cursor));
        app.snapshot = app.engine.snapshot();
        let viewport_inputs = collect_viewport_inputs(&app.snapshot).test_value();
        app.graphics.render_frame(&app.snapshot, &viewport_inputs);

        let (manager, _events, voice) =
            NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.audio.test_mut().options.voice_enabled = true;
        app.process_options_dialog_actions(vec![OptionsDlgAction::Sound(
            SoundSheetAction::VolumeChanged {
                id: SoundVolumeId::Voice,
                value: voice_volume,
            },
        )])
        .test_value();
        assert_eq!(
            app.audio.test_ref().options.voice_volume_percent(),
            i32::from(voice_volume),
        );

        let now = Instant::now();
        for sequence in 0..4 {
            voice
                .send_inbound(clonk_network::VoiceFrame {
                    client_id: remote_client as u32,
                    player_id: remote_player,
                    stream_epoch: 1,
                    sequence,
                    payload: clonk_audio::encode_voice_frame(
                        &[4_000; clonk_audio::VOICE_FRAME_SAMPLES],
                    )
                    .to_vec(),
                })
                .test_value();
        }
        app.update_voice_chat_at(now);

        let mut output = [0_i16; 2];
        app.audio.test_ref().system.mixer().mix_i16(&mut output);
        output[0]
    }

    let unity_volume = render_first_remote_sample(100);
    let boosted_volume = render_first_remote_sample(200);
    assert!(
        unity_volume > 0,
        "the remote frame must reach live playback"
    );
    assert!(
        (i32::from(boosted_volume) - i32::from(unity_volume) * 2).abs() <= 1,
        "the app must pass 200% as exactly twice the queued gain of 100%: \
         unity={unity_volume}, boosted={boosted_volume}",
    );
}

#[test]
fn voice_activation_opens_the_microphone_on_speech_and_leaves_the_key_to_the_game() {
    use std::cell::RefCell;

    struct TestVoiceSource {
        frames: RefCell<Vec<clonk_audio::VoiceInputFrame>>,
    }

    impl crate::voice_chat::VoiceFrameSource for TestVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            std::mem::take(&mut *self.frames.borrow_mut())
        }
    }

    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    let local_player = app.local_owner;
    app.engine
        .test_player_mut(local_player)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();

    let (manager, _events, mut voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    options.voice_activation_threshold = 0.5;
    options.voice_activation_hangover_ms = 0;

    let payload = clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES]);
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |_| {
        Ok(TestVoiceSource {
            frames: RefCell::new(vec![
                clonk_audio::VoiceInputFrame {
                    payload,
                    level: 0.1,
                },
                clonk_audio::VoiceInputFrame {
                    payload,
                    level: 0.9,
                },
            ]),
        })
    });

    assert!(
        !app.voice_chat.capture_active(),
        "no key has been pressed and no tick has run yet",
    );
    assert!(
        !app.handle_voice_key(VirtualKeyCode::Backquote, ElementState::Pressed),
        "voice activation must leave the push-to-talk key to the game",
    );

    app.update_voice_chat();
    assert!(app.voice_chat.capture_active());
    assert_eq!(
        app.voice_chat.capture_key(),
        None,
        "a voice-activated capture is owned by no key",
    );

    let outbound = voice.try_recv_outbound().test_value();
    assert_eq!(outbound.player_id, local_player);
    assert_eq!(outbound.sequence, 0);
    assert!(
        voice.try_recv_outbound().is_none(),
        "the frame below the threshold must never reach the wire",
    );
    assert!(app
        .voice_chat
        .active_speakers(Instant::now())
        .contains(&(local_client, local_player)));

    app.window_active = false;
    app.update_voice_chat();
    assert!(
        !app.voice_chat.capture_active(),
        "leaving the window must close a voice-activated capture too",
    );
}

#[test]
fn voice_activation_never_opens_a_microphone_the_player_did_not_opt_in_to() {
    let opens = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = opens.clone();

    struct UnreachableVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for UnreachableVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            Vec::new()
        }
    }

    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();

    let (manager, _events, _voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    // Voice activation selected, but the microphone opt-in itself never taken.
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = false;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    options.voice_activation_threshold = 0.0;
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |_| {
        observed.set(observed.get() + 1);
        Ok(UnreachableVoiceSource)
    });

    for _ in 0..5 {
        app.update_voice_chat();
    }

    assert_eq!(
        opens.get(),
        0,
        "Voice.Enabled is the microphone opt-in; the activation mode is only how an opted-in \
         microphone is keyed",
    );
    assert!(!app.voice_chat.capture_active());
}

#[test]
fn switching_back_to_push_to_talk_closes_a_voice_activated_capture() {
    struct SilentVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for SilentVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            Vec::new()
        }
    }

    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();

    let (manager, _events, _voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    // A stub source, never the real `VoiceCapture`: a test must not depend on
    // the host owning a microphone, and must certainly not open one.
    app.voice_chat =
        crate::voice_chat::VoiceChatState::with_source_opener(|_| Ok(SilentVoiceSource));

    app.update_voice_chat();
    assert!(app.voice_chat.capture_active());

    app.audio.test_mut().options.voice_activation_mode =
        crate::settings::VoiceActivationMode::PushToTalk;
    app.update_voice_chat();
    assert!(
        !app.voice_chat.capture_active(),
        "no key holds this capture open, so nothing else would ever close it",
    );
}

#[test]
fn capture_processing_follows_the_settings_without_reopening_the_microphone() {
    struct SilentVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for SilentVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            Vec::new()
        }
    }

    let opens = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed_opens = opens.clone();
    let switches: std::rc::Rc<std::cell::RefCell<Option<std::sync::Arc<_>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let observed_switches = switches.clone();
    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();

    let (manager, _events, _voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    options.voice_noise_suppression = false;
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |options| {
        observed_opens.set(observed_opens.get() + 1);
        *observed_switches.borrow_mut() = Some(options.processing.clone());
        Ok(SilentVoiceSource)
    });

    app.update_voice_chat();

    assert_eq!(opens.get(), 1);
    let switches = switches.borrow().clone().test_value();
    assert_eq!(
        switches.get(),
        clonk_audio::VoiceProcessingConfig {
            noise_suppression: false,
            ..clonk_audio::VoiceProcessingConfig::default()
        },
        "the capture opens with what the player configured",
    );

    app.audio.test_mut().options.voice_automatic_gain_control = false;
    app.update_voice_chat();

    assert_eq!(
        switches.get(),
        clonk_audio::VoiceProcessingConfig {
            noise_suppression: false,
            automatic_gain_control: false,
            ..clonk_audio::VoiceProcessingConfig::default()
        },
        "a stage switched off mid-call reaches the open microphone",
    );
    assert_eq!(
        opens.get(),
        1,
        "and does it without closing and reopening the device",
    );
}

#[test]
fn voice_activation_opens_the_exact_configured_input_device() {
    struct SilentVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for SilentVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            Vec::new()
        }
    }

    let selected = "coreaudio:built-in-microphone"
        .parse::<clonk_audio::VoiceInputDeviceId>()
        .expect("a persisted CPAL input device identity");
    let observed = std::rc::Rc::new(std::cell::RefCell::new(None));
    let observed_options = observed.clone();
    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();
    let (manager, _events, _voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    options.voice_input_device = Some(selected.clone());
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |options| {
        *observed_options.borrow_mut() = options.input_device;
        Ok(SilentVoiceSource)
    });

    app.update_voice_chat();

    assert_eq!(*observed.borrow(), Some(selected));
}

#[test]
fn clearing_the_live_network_session_drops_microphone_capture_without_another_tick() {
    struct DroppingVoiceSource(std::rc::Rc<std::cell::Cell<usize>>);

    impl crate::voice_chat::VoiceFrameSource for DroppingVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            Vec::new()
        }
    }

    impl Drop for DroppingVoiceSource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    let drops = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed_drops = drops.clone();
    let mut app = new_classic_running_sandbox_app();
    let (manager, _events, _voice) = NetworkManager::test_stub_with_voice_for_client_id(7);
    app.network = Some(manager);
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |_| {
        Ok(DroppingVoiceSource(observed_drops.clone()))
    });
    app.voice_chat
        .start_capture(Some(VirtualKeyCode::Backquote), None)
        .expect("the injected microphone opens");

    app.clear_live_network_session();

    assert_eq!(
        drops.get(),
        1,
        "session teardown closes capture synchronously"
    );
    assert!(!app.voice_chat.capture_active());
    assert!(app.network.is_none());
}

#[test]
fn changing_voice_input_keeps_the_network_and_simulation_session() {
    struct OneFrameVoiceSource {
        frame: std::cell::RefCell<Option<clonk_audio::VoiceInputFrame>>,
        drops: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl crate::voice_chat::VoiceFrameSource for OneFrameVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            self.frame.borrow_mut().take().into_iter().collect()
        }
    }

    impl Drop for OneFrameVoiceSource {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    let first = "coreaudio:first"
        .parse::<clonk_audio::VoiceInputDeviceId>()
        .expect("the first device identity");
    let second = "coreaudio:second"
        .parse::<clonk_audio::VoiceInputDeviceId>()
        .expect("the second device identity");
    let opens = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed_opens = opens.clone();
    let drops = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed_drops = drops.clone();
    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();
    let before = app.snapshot.clone();
    let (manager, _events, mut voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    options.voice_activation_threshold = 0.0;
    options.voice_input_device = Some(first.clone());
    let payload = clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES]);
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |options| {
        observed_opens.borrow_mut().push(options.input_device);
        Ok(OneFrameVoiceSource {
            frame: std::cell::RefCell::new(Some(clonk_audio::VoiceInputFrame {
                payload,
                level: 1.0,
            })),
            drops: observed_drops.clone(),
        })
    });
    let now = Instant::now();

    app.update_voice_chat_at(now);
    let before_hotplug = voice.try_recv_outbound().test_value();
    app.audio.test_mut().options.voice_input_device = Some(second.clone());
    app.update_voice_chat_at(now + Duration::from_millis(20));
    let after_hotplug = voice.try_recv_outbound().test_value();

    assert_eq!(opens.borrow().as_slice(), &[Some(first), Some(second)]);
    assert_eq!(drops.get(), 1, "only the replaced stream is closed");
    assert_eq!(
        (before_hotplug.stream_epoch, before_hotplug.sequence),
        (1, 0)
    );
    assert_eq!((after_hotplug.stream_epoch, after_hotplug.sequence), (2, 0));
    assert!(app.network.is_some(), "the game session remains connected");
    assert_eq!(app.snapshot, before, "voice never mutates lockstep state");
}

#[test]
fn every_capture_carries_the_reference_echo_cancellation_can_be_switched_on_with() {
    struct SilentVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for SilentVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            Vec::new()
        }
    }

    let referenced = std::rc::Rc::new(std::cell::Cell::new(None));
    let observed = referenced.clone();
    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();

    let (manager, _events, _voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    // Switched off at the moment the microphone opens, and switched on later.
    options.voice_echo_cancellation = false;
    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(move |options| {
        observed.set(Some(options.echo_reference.is_some()));
        Ok(SilentVoiceSource)
    });

    app.update_voice_chat();

    assert_eq!(
        referenced.get(),
        Some(true),
        "the reference is bound while the microphone opens and can never be added later, so a \
         capture that starts with echo cancellation off still has to carry it",
    );
}

#[test]
fn a_microphone_voice_activation_cannot_open_is_not_retried_every_tick() {
    let opens = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = opens.clone();
    let mut app = new_classic_running_sandbox_app();
    let local_client = 7;
    app.engine
        .test_player_mut(app.local_owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.snapshot = app.engine.snapshot();

    let (manager, _events, _voice) =
        NetworkManager::test_stub_with_voice_for_client_id(local_client as u32);
    app.network = Some(manager);
    let options = &mut app.audio.test_mut().options;
    options.voice_enabled = true;
    options.voice_activation_mode = crate::settings::VoiceActivationMode::VoiceActivated;
    struct UnreachableVoiceSource;

    impl crate::voice_chat::VoiceFrameSource for UnreachableVoiceSource {
        fn drain_frames(&self) -> Vec<clonk_audio::VoiceInputFrame> {
            unreachable!("this microphone never opens")
        }
    }

    app.voice_chat = crate::voice_chat::VoiceChatState::with_source_opener(
        move |_| -> Result<UnreachableVoiceSource, _> {
            observed.set(observed.get() + 1);
            Err(clonk_audio::VoiceCaptureError::NoInputDevice)
        },
    );

    for _ in 0..5 {
        app.update_voice_chat();
    }
    assert_eq!(opens.get(), 1, "a failed open must not be retried per tick");

    app.window_active = false;
    app.update_voice_chat();
    app.window_active = true;
    app.update_voice_chat();
    assert_eq!(
        opens.get(),
        2,
        "becoming eligible again is what clears the failure",
    );
}

#[test]
fn direct_runtime_repairs_urls_truncated_by_the_old_rust_parser() {
    // C++ defaults both fields to the complete HTTPS URL
    // (C4Config.h:35-38; C4Config.cpp:545-550). The old Rust parser
    // treated `//` as a comment and persisted only the scheme.
    let dir = tempdir();
    let path = dir.path().join("clonk-rust.config");
    fs::write(
        &path,
        "[Network]\nServerAddress = https:\nAlternateServerAddress = http:\n",
    )
    .test_value();

    assert!(repair_rust_truncated_masterserver_urls(&path).expect("repair config"));

    let config = Config::load(path).test_value();
    assert_eq!(
        config.get_in(Some("Network"), "ServerAddress"),
        Some(OFFICIAL_LEAGUE_SERVER)
    );
    assert_eq!(
        config.get_in(Some("Network"), "AlternateServerAddress"),
        Some(OFFICIAL_LEAGUE_SERVER)
    );
}

#[test]
fn reference_query_settings_use_cpp_configured_locale() {
    // C4HTTPClient canonicalizes General.LanguageCharset and sends the
    // in-memory General.LanguageEx sequence on every reference request
    // (pristine 9ffa0a5d src/C4HTTPClient.cpp:184-200;
    // src/C4Config.cpp:875-893).
    let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    let mut config = Config::new();
    config.set_in(Some("General"), "LanguageCharset", "RUSSIAN");
    config.set_in(Some("General"), "LanguageEx", "RU,US,DE");
    config.save(paths.config_file()).test_value();

    assert_eq!(
        load_reference_query_settings(Some(&paths)),
        clonk_network::ReferenceQueryConfig {
            language_charset: "RUSSIAN".to_string(),
            language_sequence: "RU,US,DE".to_string(),
            http_backend: Default::default(),
        }
    );
    assert_eq!(
        load_network_advertiser_settings(Some(&paths)).language_charset,
        "RUSSIAN"
    );
}

#[test]
fn client_network_settings_preserve_configured_ports_and_zero_disables_protocol() {
    let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    let mut config = Config::new();
    config.set_in(Some("General"), "Name", "Exact maker");
    config.set_in(Some("Network"), "PortTCP", "0");
    config.set_in(Some("Network"), "PortUDP", "22113");
    config.save(paths.config_file()).test_value();

    let settings = client_settings_for_paths(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client".to_string(),
        Some(&paths),
    );

    assert_eq!(settings.mesh_tcp_bind_address, None);
    assert_eq!(settings.group_maker.as_bytes(), b"Exact maker");
    assert_eq!(
        settings.mesh_udp_bind_address,
        Some(SocketAddr::from(([0_u16; 8], 22_113)))
    );
}

#[test]
fn network_port_sanitation_preserves_zero_and_disables_invalid_values() {
    assert_eq!(
        sanitized_network_ports(b""),
        NetworkPorts {
            tcp: DEFAULT_NETWORK_TCP_PORT,
            udp: DEFAULT_NETWORK_UDP_PORT,
            discovery: clonk_network::DEFAULT_DISCOVERY_PORT,
            reference: clonk_network::DEFAULT_REFERENCE_PORT,
        }
    );
    assert_eq!(
        sanitized_network_ports(
            b"[Network]\nPortTCP=0\nPortUDP=0\nPortDiscovery=0\nPortRefServer=0\n"
        ),
        NetworkPorts {
            tcp: 0,
            udp: 0,
            discovery: 0,
            reference: 0,
        }
    );
    assert_eq!(
                sanitized_network_ports(
                    b"[Network]\nPortTCP=70000\nPortUDP=-1\nPortDiscovery=not-a-port\nPortRefServer=65536\n"
                ),
                NetworkPorts {
                    tcp: 0,
                    udp: 0,
                    discovery: 0,
                    reference: 0,
                }
            );
}

#[test]
fn network_port_collisions_increment_secondary_ports_and_wrap_at_u16_max() {
    assert_eq!(
        sanitized_network_ports(
            b"[Network]\nPortTCP=23000\nPortRefServer=23000\nPortUDP=24000\nPortDiscovery=24000\n"
        ),
        NetworkPorts {
            tcp: 23_000,
            udp: 24_000,
            discovery: 24_001,
            reference: 23_001,
        }
    );
    assert_eq!(
        sanitized_network_ports(
            b"[Network]\nPortTCP=65535\nPortRefServer=65535\nPortUDP=65535\nPortDiscovery=65535\n"
        ),
        NetworkPorts {
            tcp: u16::MAX,
            udp: u16::MAX,
            discovery: clonk_network::DEFAULT_DISCOVERY_PORT,
            reference: clonk_network::DEFAULT_REFERENCE_PORT,
        }
    );
}

#[test]
fn zero_ports_flow_to_disabled_app_network_services() {
    let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        b"[Network]\nPortTCP=0\nPortUDP=0\nPortDiscovery=0\nPortRefServer=0\n",
    )
    .test_value();

    assert_eq!(load_network_startup_settings(Some(&paths)).1, 0);
    assert_eq!(load_network_reference_port(Some(&paths)), 0);
    assert_eq!(load_network_search_settings(Some(&paths)).discovery_port, 0);
    assert_eq!(
        load_network_advertiser_settings(Some(&paths)),
        clonk_network::NetworkGameAdvertiserConfig {
            discovery_port: 0,
            reference_port: None,
            language_charset: String::new(),
        }
    );
    let settings = client_settings_for_paths(
        SocketAddr::from(([127, 0, 0, 1], DEFAULT_NETWORK_TCP_PORT)),
        "Client".to_string(),
        Some(&paths),
    );
    assert_eq!(settings.mesh_tcp_bind_address, None);
    assert_eq!(settings.mesh_udp_bind_address, None);
}

#[test]
fn options_network_back_validates_both_port_pairs_and_alternate_notice_gate() {
    use clonk_frontend::message_dialog::{MessageDialogIcon, MessageDialogResult};
    use clonk_frontend::startup_options_dlg::OptionsDlgAction;
    use clonk_frontend::startup_options_network::{NetworkCheckboxId, NetworkPortId};

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    {
        let network = app
            .startup_options_dialog
            .as_mut()
            .test_value()
            .network_mut();
        let tcp = network.port(NetworkPortId::Tcp).port;
        network.port_mut(NetworkPortId::Reference).port = tcp;
    }
    app.process_options_dialog_actions(vec![OptionsDlgAction::Back])
        .test_value();
    assert_eq!(app.startup_view, StartupView::Options);
    let tcp_error = app.message_dialogs.last().test_value();
    assert_eq!(tcp_error.state.caption(), "Configuration error");
    assert_eq!(
        tcp_error.state.message(),
        "TCP port and reference port must be set to different values between 1 and 65535!"
    );
    assert_eq!(tcp_error.state.icon(), MessageDialogIcon::ERROR);
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    {
        let network = app
            .startup_options_dialog
            .as_mut()
            .test_value()
            .network_mut();
        network.port_mut(NetworkPortId::Reference).port = 11_111;
        let udp = network.port(NetworkPortId::Udp).port;
        network.port_mut(NetworkPortId::Discovery).port = udp;
    }
    app.process_options_dialog_actions(vec![OptionsDlgAction::Back])
        .test_value();
    assert_eq!(app.startup_view, StartupView::Options);
    let udp_error = app.message_dialogs.last().test_value();
    assert_eq!(udp_error.state.caption(), "Configuration error");
    assert_eq!(
        udp_error.state.message(),
        "UDP port and discovery port must be set to different values between 1 and 65535!"
    );
    assert_eq!(udp_error.state.icon(), MessageDialogIcon::ERROR);
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    app.process_options_dialog_actions(vec![OptionsDlgAction::NetworkCheckboxChanged {
        id: NetworkCheckboxId::UseAlternateServer,
        checked: true,
    }])
    .test_value();
    assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
        dialog.continuation,
        MessageDialogContinuation::OptionsAlternateServerNotice
    )));
    app.message_dialogs
        .last_mut()
        .test_value()
        .state
        .handle_hotkey('D');
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    assert!(
        app.startup_options_dialog
            .as_ref()
            .unwrap()
            .network()
            .hide_no_official_league_notice
    );
    app.process_options_dialog_actions(vec![OptionsDlgAction::NetworkCheckboxChanged {
        id: NetworkCheckboxId::UseAlternateServer,
        checked: true,
    }])
    .test_value();
    assert!(app.message_dialogs.is_empty());
}

#[test]
fn network_create_selects_a_scenario_before_binding_a_host() {
    // C4StartupNetDlg::CreateGame only switches to the network scenario
    // selector. The selected scenario is stored before OpenGame opens it
    // and calls InitNetworkHost, so no network socket/reference exists in
    // the selector (src/C4StartupNetDlg.cpp:1111-1114;
    // src/C4StartupScenSelDlg.cpp:1635-1666;
    // src/C4Game.cpp:421-438).
    let mut app = new_menu_app(1280, 720);
    app.open_network_game_dialog();

    app.process_network_dialog_actions(vec![
        clonk_frontend::startup_netdlg::NetDlgAction::CreateGame,
    ])
    .test_value();

    assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
    assert_eq!(
        app.scenario_selector_mode,
        ScenarioSelectorMode::NetworkHost
    );
    assert!(app.startup_network_connection.is_none());
    assert!(app.network.is_none());
    assert!(app.network_game_advertiser.is_none());

    app.scensel_do_back().test_value();
    assert_eq!(app.startup_view, StartupView::NetworkGame);
}

#[test]
fn network_host_preparation_keeps_cpp_configured_participant_order() {
    // C4Game freezes Config.General.Participants into PlayerFilenames and
    // C4ClientPlayerInfos walks those modules in order. The separately
    // sorted startup player-selection rows never reorder the host's
    // initial packet or NRT_Player IDs (pristine 9ffa0a5d
    // src/C4Game.cpp:361-364; src/C4PlayerInfo.cpp:70-104,357-395).
    let install = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let content = install.path().join("content");
    let scenario_path = content.join("Order.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    let players = install.path().join("Players");
    fs::create_dir_all(&players).test_value();
    let write_player = |filename: &str, name: &[u8]| {
        let path = players.join(filename);
        let mut group = clonk_resources::MutableGroup::new(filename);
        let mut player_core = b"[Player]\nName=".to_vec();
        player_core.extend_from_slice(name);
        player_core.extend_from_slice(b"\n[Preferences]\nColorDw=255\n");
        group
            .add_file_with_metadata("Player.txt", player_core, 1, false)
            .test_value();
        fs::write(&path, group.pack().test_value()).test_value();
        path
    };
    let bravo = write_player("Bravo.c4p", b"Br\xc3\xa4vo");
    let alpha = write_player("Alpha.c4p", b"Alpha");
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    let mut config = b"[General]\nName=\"M\xc3\xa4ker\"\nPlayerPath=Players\nParticipants=Players/Bravo.c4p;Players/Alpha.c4p\n\n[Network]\nLocalName=\"H\xc3\xa4st\"\nNick=\"N\xc3\xa4ck\"\nComment=\"".to_vec();
    for _ in 0..129 {
        config.extend_from_slice(b"\xc3\xa4");
    }
    config.extend_from_slice(
                b"\"\nControlRate=7\nControlMode=1\nPortTCP=12345\nPortUDP=12346\nMaxLoadFileSize=123456\nNoRuntimeJoin=0\nEnableUPnP=0\n",
            );
    fs::write(paths.config_file(), config).test_value();
    let app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    assert_eq!(
        app.startup_player_files
            .iter()
            .map(|player| player.path.as_path())
            .collect::<Vec<_>>(),
        vec![alpha.as_path(), bravo.as_path()],
        "the UI model supplies the deliberately opposite sorted order"
    );
    let mut scenario = FrontendScenario::fallback();
    scenario.title = "Order".to_string();
    scenario.path = Some(scenario_path);

    let preparation = build_network_host_preparation(
        &app,
        &scenario,
        &ScenarioDefinitionLoad::Seed {
            modules: Vec::new(),
            definition_root: None,
        },
        &[],
        &[],
        None,
        None,
    )
    .test_value();

    assert_eq!(
        clonk_resources::encode_legacy_script_text(&preparation.group_maker),
        Some(b"M\xc3\xa4ker".to_vec())
    );
    assert_eq!(
        clonk_resources::encode_legacy_script_text(&preparation.host_name),
        Some(b"H\xc3\xa4st".to_vec())
    );
    assert_eq!(
        clonk_resources::encode_legacy_script_text(&preparation.host_nick),
        Some(b"N\xc3\xa4ck".to_vec())
    );
    assert_eq!(
        clonk_resources::encode_legacy_script_text(&preparation.network_comment),
        Some(b"\xc3\xa4".repeat(128)),
        "VAL_Comment counts and truncates native bytes, not Unicode scalars"
    );
    assert_eq!(
        preparation.player_sources[0]
            .identity
            .as_ref()
            .expect("configured player identity")
            .player_name
            .as_bytes(),
        b"Br\xc3\xa4vo",
        "valid UTF-8-shaped player-name bytes remain native bytes"
    );
    assert_eq!(preparation.config.control_rate, 7);
    assert_eq!(preparation.config.control_mode, 1);
    assert_eq!(preparation.config.network_tcp_port, 12_345);
    assert_eq!(preparation.config.network_udp_port, 12_346);
    assert_eq!(preparation.config.max_load_file_size, 123_456);
    assert!(!preparation.config.no_runtime_join);
    assert!(!preparation.config.enable_upnp);

    let publication = preparation.league.test_ref();
    assert!(!publication.league_server_signup);
    assert_eq!(
        preparation
            .player_sources
            .iter()
            .map(|source| {
                (
                    source.resource.path.as_path(),
                    source.resource.wire_name.as_bytes(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (bravo.as_path(), b"Players/Bravo.c4p".as_slice()),
            (alpha.as_path(), b"Players/Alpha.c4p".as_slice()),
        ]
    );
}

#[test]
fn command_line_definition_selection_is_published_to_network_clients() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let content_root = content.path();
    fs::create_dir_all(content_root.join("Material.c4g")).test_value();
    let base = install_network_definition_pack(content_root, "Base.c4d", "BAS1");
    let extra_one = install_network_definition_pack(content_root, "ExtraOne.c4d", "EXT1");
    let extra_two = install_network_definition_pack(content_root, "ExtraTwo.c4d", "EXT2");
    let objects = install_network_definition_pack(content_root, "Objects.c4d", "OBJS");
    let scenario_path = content_root.join("CommandLine.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(scenario_path.join("Scenario.txt"), "[Head]\nTitle=Command Line Definitions\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nLocalOnly=1\n").test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content_root));
    persist_config_value(&paths, "General", "Definitions", "Base.c4d").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let classic = parse_classic_command_line(&[
        OsString::from("./ExtraOne.c4d"),
        OsString::from("ExtraTwo.c4d"),
    ]);
    app.apply_classic_command_line(&classic).test_value();
    persist_config_value(&paths, "General", "Definitions", "ChangedAfterParse.c4d").test_value();
    let definition_load = app.take_scenario_seed_definition_load();
    assert!(matches!(
        &definition_load,
        ScenarioDefinitionLoad::Seed { modules, .. }
            if modules == &["Base.c4d", "./ExtraOne.c4d", "ExtraTwo.c4d", "Objects.c4d"]
    ));
    let frontend = FrontendScenario {
        identifier: "CommandLine.c4s".to_string(),
        title: "Command Line Definitions".to_string(),
        path: Some(scenario_path),
        ..FrontendScenario::fallback()
    };
    let staged = app
        .prepare_network_host_scenario(frontend, definition_load)
        .test_value();
    assert!(matches!(
        app.scenario_seed_definition_load(),
        ScenarioDefinitionLoad::Seed { modules, .. }
            if modules == ["Objects.c4d"]
    ));
    assert_eq!(
        staged.scenario.definition_resource_paths(),
        &[
            base.clone(),
            extra_one.clone(),
            extra_two.clone(),
            objects.clone(),
        ]
    );
    let prepared = prepare_staged_network_host(&app, &staged);
    assert_eq!(
        published_definition_wire_names(&prepared),
        vec![
            b"Base.c4d".to_vec(),
            b"./ExtraOne.c4d".to_vec(),
            b"ExtraTwo.c4d".to_vec(),
            b"Objects.c4d".to_vec(),
        ]
    );

    let host = prepared.host_config();
    let snapshot = host.initial_join_snapshot.test_ref().clone();
    assert_eq!(
        snapshot
            .parameters
            .game_resources
            .iter()
            .filter(|core| {
                core.resource_type == clonk_network::HostResourceType::Definitions as u8
            })
            .map(|core| core.filename.as_bytes().to_vec())
            .collect::<Vec<_>>(),
        published_definition_wire_names(&prepared)
    );
    let dynamic_path = host
        .resource_files
        .iter()
        .find(|resource| {
            resource.core.resource_type == clonk_network::HostResourceType::Dynamic as u8
        })
        .test_value()
        .path
        .clone();
    let dynamic_scenario = Group::open(&dynamic_path)
        .test_value()
        .read_file("Scenario.txt")
        .test_value();
    let expected_definitions =
        b"Definitions=\"Base.c4d\",\"./ExtraOne.c4d\",\"ExtraTwo.c4d\",\"Objects.c4d\"";
    assert!(dynamic_scenario
        .windows(expected_definitions.len())
        .any(|window| window == expected_definitions));

    let host_files = host.resource_files.clone();
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 2,
        start_control_tick: snapshot.dynamic_tick,
        status: host.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    let complete_path = |core: &clonk_engine::NetworkResourceCore| {
        host_files
            .iter()
            .find(|resource| resource.core.id == core.id)
            .map(|resource| resource.path.clone())
    };
    let scenario_resources =
        resolve_client_scenario_resources(&join_data, complete_path).test_value();
    let game_resources = resolve_client_game_resources(&join_data, |core| {
        host_files
            .iter()
            .find(|resource| resource.core.id == core.id)
            .map(|resource| resource.path.clone())
    })
    .test_value();
    let client_directory = tempdir();
    let combined_path = client_directory.path().join("Combined2.c4s");
    let mut artifact = GameApp::run_lobby_preload_job(LobbyPreloadJob {
        graphics: LobbyPreloadGraphicsContext {
            app_paths: app.app_paths.clone(),
            fallback: app.startup_game_graphics_resources(),
            liquid_animation_enabled: app.assets.liquid_animation_enabled(),
        },
        source: LobbyPreloadJobSource::Client {
            join_data,
            scenario_resources: Some(scenario_resources),
            game_resources,
            resource_directory: client_directory.path().to_path_buf(),
            maker: "Exact Host".to_string(),
            scenario_path: combined_path,
            staging_path: None,
        },
    })
    .test_value();
    let mut client_definition_ids = Vec::new();
    artifact
        .client
        .as_mut()
        .and_then(|client| client.scenario.take())
        .test_value()
        .visit_definition_groups(|id, _| client_definition_ids.push(id.to_string()));
    assert_eq!(
        client_definition_ids,
        vec![
            "BAS1".to_string(),
            "EXT1".to_string(),
            "EXT2".to_string(),
            "OBJS".to_string(),
        ]
    );
    let mut host_definition_ids = Vec::new();
    prepared
        .claim_scenario()
        .test_value()
        .visit_definition_groups(|id, _| host_definition_ids.push(id.to_string()));
    assert_eq!(host_definition_ids, client_definition_ids);
}

#[test]
fn packed_scenario_alias_is_skipped_as_an_external_definition() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let content_root = content.path();
    fs::create_dir_all(content_root.join("Material.c4g")).test_value();
    let scenario_path = content_root.join("AliasScenario.c4s");
    let mut scenario_group = clonk_resources::MutableGroup::new("AliasScenario.c4s");
    scenario_group
                .add_file(
                    "Scenario.txt",
                    b"[Head]\nTitle=Scenario Alias\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nLocalOnly=1\n"
                        .to_vec(),
                ).test_value();
    scenario_group
        .add_child(
            "ScenarioDef.c4d",
            packed_network_definition("ScenarioDef.c4d", "SCEN"),
        )
        .test_value();
    fs::write(&scenario_path, scenario_group.pack().test_value()).test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content_root));
    let app = new_menu_app_with_paths(640, 480, &paths);
    let staged = app
        .prepare_network_host_scenario(
            FrontendScenario {
                identifier: "AliasScenario.c4s".to_string(),
                title: "Scenario Alias".to_string(),
                path: Some(scenario_path.clone()),
                ..FrontendScenario::fallback()
            },
            ScenarioDefinitionLoad::Fixed {
                modules: vec![path_as_legacy_text(&scenario_path)],
                definition_root: None,
            },
        )
        .test_value();
    assert_eq!(
        staged.scenario.definition_resource_paths(),
        std::slice::from_ref(&scenario_path)
    );

    let prepared = prepare_staged_network_host(&app, &staged);
    let snapshot = prepared.host_config().initial_join_snapshot.test_ref();
    assert_eq!(
        snapshot.parameters.game_resources[0].id,
        snapshot.parameters.scenario.id
    );
    assert_eq!(
        snapshot.parameters.game_resources[0].resource_type,
        clonk_network::HostResourceType::Scenario as u8
    );
    assert!(published_definition_wire_names(&prepared).is_empty());
    assert_eq!(
        prepared.definition_modules(),
        [path_as_legacy_text(&scenario_path)]
    );

    let scenario = prepared.claim_scenario().test_value();
    assert!(scenario.definition_resource_paths().is_empty());
    let mut definition_ids = Vec::new();
    scenario.visit_definition_groups(|id, _| definition_ids.push(id.to_string()));
    assert_eq!(definition_ids, ["SCEN"]);
}

#[test]
fn packed_system_alias_becomes_a_repeated_definition_row() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let content_root = content.path();
    let system_path = content_root.join("System.c4g");
    let mut system_group = clonk_resources::MutableGroup::new("System.c4g");
    system_group
        .add_child(
            "SystemDef.c4d",
            packed_network_definition("SystemDef.c4d", "SYSD"),
        )
        .test_value();
    fs::write(&system_path, system_group.pack().test_value()).test_value();
    fs::create_dir_all(content_root.join("Material.c4g")).test_value();
    let scenario_path = content_root.join("SystemAlias.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=System Alias\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nLocalOnly=1\n",
    )
    .test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content_root));
    let app = new_menu_app_with_paths(640, 480, &paths);
    let staged = app
        .prepare_network_host_scenario(
            FrontendScenario {
                identifier: "SystemAlias.c4s".to_string(),
                title: "System Alias".to_string(),
                path: Some(scenario_path),
                ..FrontendScenario::fallback()
            },
            ScenarioDefinitionLoad::Fixed {
                modules: vec!["System.c4g".to_string()],
                definition_root: None,
            },
        )
        .test_value();
    assert_eq!(
        staged.scenario.definition_resource_paths(),
        std::slice::from_ref(&system_path)
    );

    let prepared = prepare_staged_network_host(&app, &staged);
    let game_resources = &prepared
        .host_config()
        .initial_join_snapshot
        .test_ref()
        .parameters
        .game_resources;
    assert_eq!(game_resources[0], game_resources[1]);
    assert_eq!(
        game_resources[0].resource_type,
        clonk_network::HostResourceType::Definitions as u8
    );
    assert_eq!(
        published_definition_wire_names(&prepared),
        vec![b"System.c4g".to_vec(), b"System.c4g".to_vec()]
    );
    assert_eq!(prepared.definition_modules(), ["System.c4g"]);

    let scenario = prepared.claim_scenario().test_value();
    assert_eq!(
        scenario.definition_resource_paths(),
        [system_path.clone(), system_path]
    );
    let mut definition_ids = Vec::new();
    scenario.visit_definition_groups(|id, _| definition_ids.push(id.to_string()));
    assert_eq!(definition_ids, ["SYSD"]);
}

#[test]
fn packed_material_alias_removes_the_host_material_projection() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let content_root = content.path();
    fs::create_dir_all(content_root.join("System.c4g")).test_value();
    let material_path = content_root.join("Material.c4g");
    let mut material_group = clonk_resources::MutableGroup::new("Material.c4g");
    material_group
        .add_child(
            "MaterialDef.c4d",
            packed_network_definition("MaterialDef.c4d", "MATD"),
        )
        .test_value();
    fs::write(&material_path, material_group.pack().test_value()).test_value();
    let scenario_path = content_root.join("MaterialAlias.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Material Alias\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nLocalOnly=1\n",
    )
    .test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content_root));
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = app
        .prepare_network_host_scenario(
            FrontendScenario {
                identifier: "MaterialAlias.c4s".to_string(),
                title: "Material Alias".to_string(),
                path: Some(scenario_path),
                ..FrontendScenario::fallback()
            },
            ScenarioDefinitionLoad::Fixed {
                modules: vec!["Material.c4g".to_string()],
                definition_root: None,
            },
        )
        .test_value();

    let prepared = prepare_staged_network_host(&app, &staged);
    let game_resources = &prepared
        .host_config()
        .initial_join_snapshot
        .test_ref()
        .parameters
        .game_resources;
    assert_eq!(game_resources[0], game_resources[2]);
    assert_eq!(
        game_resources[2].resource_type,
        clonk_network::HostResourceType::Definitions as u8
    );
    assert!(prepared.material_resource_groups().is_empty());
    assert_eq!(prepared.definition_modules(), ["Material.c4g"]);
    let frozen_definition_paths = {
        let (executable, definitions) = prepared.definition_save_paths();
        (executable.to_owned(), definitions.to_owned())
    };
    assert_eq!(
        frozen_definition_paths,
        game_save_definition_paths(Some(&paths), &load_native_config_bytes(Some(&paths)),)
    );
    persist_config_value(&paths, "General", "DefinitionPath", "Changed/").test_value();
    assert_eq!(
        prepared.definition_save_paths(),
        (
            frozen_definition_paths.0.as_str(),
            frozen_definition_paths.1.as_str(),
        ),
        "runtime saves retain the process-loaded path pair frozen for the initial dynamic"
    );
    assert_eq!(
        game_save_definition_paths(Some(&paths), &load_native_config_bytes(Some(&paths)),).1,
        "Changed/",
        "the on-disk value really changed after preparation"
    );

    app.install_prepared_host_material_resources(&prepared);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_owned(),
        prepared: Some(prepared.clone()),
    }));
    let (authoritative_materials, reuse_preloaded) = network_material_load_plan(
        app.network_mode.as_ref(),
        app.network_material_resource_groups.as_deref(),
    );
    assert_eq!(authoritative_materials.map(<[Group]>::len), Some(0));
    assert!(
        !reuse_preloaded,
        "an authoritative empty host material vector bypasses staged local preload data"
    );

    let scenario = prepared.claim_scenario().test_value();
    app.prepare_recording_for(
        &staged.frontend,
        &scenario,
        None,
        Some(prepared.definition_modules()),
        Some(prepared.definition_save_paths()),
    )
    .test_value();
    let recording_seed = app.live_save_seed.test_ref();
    assert_eq!(recording_seed.definition_modules, ["Material.c4g"]);
    assert_eq!(
                (
                    recording_seed.definition_executable_path.as_str(),
                    recording_seed.definition_path.as_str(),
                ),
                (
                    frozen_definition_paths.0.as_str(),
                    frozen_definition_paths.1.as_str(),
                ),
                "initial and runtime record saves share the definition paths frozen before the config rewrite"
            );
    assert_eq!(
        recording_definition_modules(&scenario, Some(prepared.definition_modules())),
        ["Material.c4g"],
        "recording keeps Game.DefinitionFilenames instead of final cross-type resource rows"
    );
    assert_eq!(
                recording_definition_modules(&scenario, None),
                [
                    path_as_legacy_text(&material_path),
                    path_as_legacy_text(&material_path),
                ],
                "the fallback demonstrates the repeated physical projection that must not seed a prepared-host record"
            );
    assert_eq!(
        scenario.definition_resource_paths(),
        [material_path.clone(), material_path]
    );
    let mut definition_ids = Vec::new();
    scenario.visit_definition_groups(|id, _| definition_ids.push(id.to_string()));
    assert_eq!(definition_ids, ["MATD"]);
}

#[test]
fn reference_state_survives_initial_and_final_advertiser_bind_failure() {
    // C4Network2Reference is rebuilt from game state independently of the
    // optional reference server. A listener failure must not discard the
    // template or leave its retained value at GS_Lobby.
    let (snapshot, reference) = default_exact_host_reference();
    let parameters = snapshot.parameters.clone();
    let occupied = std::net::TcpListener::bind("[::]:0").test_value();
    let config = clonk_network::NetworkGameAdvertiserConfig {
        discovery_port: 0,
        reference_port: Some(occupied.local_addr().test_value().port()),
        language_charset: String::new(),
    };
    let mut app = new_state_only_menu_app(320, 200);

    app.start_network_game_advertiser_with_reference(config.clone(), reference);

    assert!(app.network_game_advertiser.is_none());
    assert_eq!(
        app.advertised_game_reference
            .as_ref()
            .expect("validated state survives bind failure")
            .summary()
            .state,
        "Lobby"
    );

    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.host_join_snapshot = Some(snapshot);
    app.control_clients
        .replace_snapshot(parameters.clients.clients.clone());
    app.engine.set_max_players(parameters.max_players);
    app.snapshot.game_time = 37;
    app.snapshot.frame = 91;

    app.publish_game_over_host_reference_with_config(config);

    assert!(app.network_game_advertiser.is_none());
    let retained = app.advertised_game_reference.test_ref();
    assert_eq!(retained.summary().state, "Running");
    assert!(!retained.summary().join_allowed);
    assert_eq!(
        (retained.metadata().time, retained.metadata().frame),
        (37, 91)
    );

    let retained = retained.clone();
    app.start_network_game_advertiser_with_reference(
        clonk_network::NetworkGameAdvertiserConfig {
            discovery_port: 0,
            reference_port: Some(0),
            language_charset: String::new(),
        },
        retained,
    );
    assert!(app.network_game_advertiser.is_some());
    app.change_network_control_to_local(0);
    assert!(app.network_game_advertiser.is_none());
    assert!(app.advertised_game_reference.is_none());
}

#[test]
fn netpuncher_assignment_refreshes_the_retained_host_reference() {
    let (_snapshot, reference) = default_exact_host_reference();
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.advertised_game_reference = Some(reference);
    let (manager, events, mut commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    app.network = Some(manager);
    let addresses = vec![
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Tcp,
            "198.51.100.7:11112".parse().unwrap(),
        ),
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Udp,
            "198.51.100.7:11113".parse().unwrap(),
        ),
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Udp,
            "198.51.100.7:43123".parse().unwrap(),
        ),
    ];
    events
        .send(NetworkEvent::NetpuncherStateChanged {
            game_ids: clonk_network::NetpuncherGameIds {
                ipv4: 0xaabb_ccdd,
                ipv6: 0,
            },
            local_addresses: addresses.clone(),
        })
        .test_value();

    app.test_network_events();

    let updated = app.advertised_game_reference.test_ref();
    assert_eq!(updated.summary().netpuncher_ipv4, 0xaabb_ccdd);
    assert_eq!(updated.metadata().netpuncher_ipv4, 0xaabb_ccdd);
    assert_eq!(updated.summary().addresses, addresses);
    assert_eq!(updated.metadata().addresses, addresses);
    assert_eq!(
        updated.summary().tcp_addresses,
        vec!["198.51.100.7:11112".parse::<SocketAddr>().unwrap()]
    );
    assert!(app.classic_host_lobby_active());
    assert_eq!(commands.take_league_update_effects().1, 1);
}

#[test]
fn recoverable_route_diagnostic_keeps_the_classic_host_lobby_open() {
    // Failed speculative/secondary connections are warnings and do not
    // remove the peer or close the lobby (src/C4Network2IO.cpp:252-264;
    // src/C4Network2.cpp:1761-1817). C4Log routes that warning into the
    // active lobby log instead of opening a child pane
    // (src/C4Log.cpp:227-239; src/C4GameLobby.cpp:738-753).
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    let warning = "could not connect to 127.0.0.1:32122 using TCP: Connection refused.";
    events
        .send(NetworkEvent::RecoverableRouteDiagnostic {
            client_id: None,
            error: warning.to_string(),
        })
        .test_value();

    app.test_network_events();

    assert!(app.classic_host_lobby_active());
    assert!(app.network.is_some());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("classic lobby remains")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some(warning)
    );
}

#[test]
fn an_unassociated_connection_failure_stays_out_of_the_classic_host_lobby() {
    // A probe, a cancelled join or a refused admission never names a client.
    // C4Network2 logs those at info — HandleConn's "connection by X blocked"
    // and OnConnectFail's "connection to X failed" — which is under the warn
    // C4LogSystem::GuiSink defaults to, so MainDlg::OnLog never receives them
    // (src/C4Network2.cpp:1361,1745-1747; src/C4Log.cpp:307). The warn-level
    // route diagnostic above is the contrast: it does reach the lobby.
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    let before = app
        .classic_host_lobby
        .as_ref()
        .expect("classic lobby is installed")
        .controller
        .logs()
        .len();
    events
        .send(NetworkEvent::UnassociatedConnectionFailed {
            error: "connection admission from 127.0.0.1:11113 failed: wrong password".to_string(),
        })
        .test_value();

    app.test_network_events();

    assert!(app.classic_host_lobby_active());
    assert!(app.network.is_some());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("classic lobby remains")
            .controller
            .logs()
            .len(),
        before,
        "an unassociated connection failure must not reach the lobby log"
    );
}

#[test]
fn peer_protocol_error_logs_and_keeps_the_classic_lobby_open() {
    // A malformed packet is logged and closes the offending connection,
    // but packet handling returns to the network loop; it does not abort
    // the host lobby (src/C4Network2IO.cpp:808-834;
    // src/C4Network2.cpp:1774-1824).
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    events
        .send(NetworkEvent::Error(
            "failed to decode direct control packet".to_string(),
        ))
        .test_value();

    app.test_network_events();

    assert!(app.classic_host_lobby_active());
    assert!(app.network.is_some());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("classic lobby remains")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some("failed to decode direct control packet")
    );
}

#[test]
fn typed_peer_transport_diagnostic_keeps_the_classic_lobby_open() {
    // A peer packet compiler failure closes that peer's route and returns
    // to the network scheduler. The associated error is written to the
    // lobby log without creating a fatal child presentation
    // (src/C4Network2IO.cpp:808-834;
    // src/C4Log.cpp:227-239; src/C4GameLobby.cpp:738-753).
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    events
        .send(NetworkEvent::TransportDiagnostic {
            client_id: Some(7),
            error: "failed to unpack forwarded packet".to_string(),
        })
        .test_value();

    app.test_network_events();

    assert!(app.classic_host_lobby_active());
    assert!(app.network.is_some());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("classic lobby remains")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some("client 7: failed to unpack forwarded packet")
    );
}

#[test]
fn network_diagnostics_are_visible_in_a_joined_client_lobby() {
    // Every fullscreen peer owns the same MainDlg, and the GUI log sink
    // forwards warnings/errors to that live lobby without a host-role
    // check (src/C4Network2.cpp:483-487;
    // src/C4Log.cpp:227-239; src/C4GameLobby.cpp:738-753).
    let mut app = new_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let (manager, events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    events
        .send(NetworkEvent::RecoverableRouteDiagnostic {
            client_id: Some(3),
            error: "secondary route unavailable".to_string(),
        })
        .test_value();
    events
        .send(NetworkEvent::TransportDiagnostic {
            client_id: Some(3),
            error: "malformed peer packet".to_string(),
        })
        .test_value();
    events
        .send(NetworkEvent::Error("network scheduler warning".to_string()))
        .test_value();

    app.test_network_events();

    assert!(app.joined_network_lobby_active());
    assert!(app.network.is_some());
    let logs = &app.network_lobby.test_ref().logs;
    assert_eq!(
        logs.iter()
            .rev()
            .take(3)
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "network scheduler warning",
            "client 3: malformed peer packet",
            "client 3: secondary route unavailable",
        ]
    );
}

#[test]
fn failed_client_connection_reaches_cleanup_and_keeps_classic_lobby_open() {
    // Total client connectivity loss notifies the league, queues a
    // synchronized ClientRemove, and leaves the host lobby running
    // (src/C4Network2.cpp:1802-1824).
    let mut app = new_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let failed_client_id = 24;
    app.pending_runtime_dynamic_request =
        Some(PendingRuntimeDynamicRequest::new(failed_client_id, 0));
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    events
        .send(NetworkEvent::PeerConnectionFailed {
            client_id: failed_client_id,
        })
        .test_value();
    events
        .send(NetworkEvent::PeerDisconnected {
            client_id: failed_client_id,
            reason: None,
        })
        .test_value();
    let diagnostic = "read failed: connection reset by peer";
    events
        .send(NetworkEvent::RecoverableRouteDiagnostic {
            client_id: Some(failed_client_id),
            error: diagnostic.to_string(),
        })
        .test_value();

    app.test_network_events();

    assert!(app.classic_host_lobby_active());
    assert!(app.network.is_some());
    assert!(
        app.pending_runtime_dynamic_request.is_none(),
        "the ordinary PeerConnectionFailed cleanup handler must run"
    );
    let expected_diagnostic = format!("client {failed_client_id}: {diagnostic}");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("classic lobby remains")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some(expected_diagnostic.as_str())
    );
}

#[test]
fn fatal_worker_failure_in_network_lobby_restores_startup_error_log() {
    // A fatal application-loop failure makes DoLobby clear the network and
    // return false; Game::Init then fails and QuitGame rebuilds startup
    // with the error flag retained (src/C4Network2.cpp:475-510;
    // src/C4Game.cpp:408-411; src/C4Application.cpp:373-400,438-449).
    let mut app = new_real_classic_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let (manager, events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    events
        .send(NetworkEvent::FatalError(
            "network worker stopped unexpectedly".to_string(),
        ))
        .test_value();

    app.test_network_events();

    assert_eq!(app.mode, AppMode::Menu);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.startup_network_dialog.is_some());
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_lobby.is_none());
    assert_startup_error_log(
        &app,
        "Unable to start network session: network worker stopped unexpectedly",
    );
    let engine_results = app.engine.snapshot().round_results;
    assert_eq!(
        engine_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        engine_results.network_result_message,
        b"network worker stopped unexpectedly"
    );
    assert_eq!(app.snapshot.round_results, engine_results);
    let mut frame = vec![0x4c; 320 * 200 * 4];
    app.test_render(&mut frame);
    assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn fatal_worker_failure_after_lobby_while_loading_changes_to_local_control() {
    // Once DoLobby has returned, the network control object is active.
    // Clear therefore invokes ChangeToLocal even while the rest of
    // Game::Init is still loading (src/C4Network2.cpp:493-525,748-775;
    // src/C4GameControl.cpp:94-127).
    let mut app = new_state_only_running_sandbox_app();
    app.mode = AppMode::Loading;
    let (events, _commands) = install_running_network_stub(&mut app, 7, 31, 4);
    app.engine.set_network_control_mode(true);
    app.network_lobby = None;
    let control_tick_before = app.engine.sync_check(7).control_tick;
    events
        .send(NetworkEvent::FatalError(
            "network worker stopped during start".to_string(),
        ))
        .test_value();

    app.test_network_events();

    assert_eq!(app.mode, AppMode::Loading);
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_lobby.is_none());
    assert_eq!(app.engine.control_rate, 1);
    assert_eq!(
        app.engine.sync_check(7).control_tick,
        control_tick_before,
        "ChangeToLocal preserves the current control tick"
    );
    let engine_results = app.engine.snapshot().round_results;
    assert_eq!(
        engine_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        engine_results.network_result_message,
        b"network worker stopped during start"
    );
    assert_eq!(app.snapshot.round_results, engine_results);
}

#[test]
fn prepared_network_loading_failure_clears_session_before_restoring_startup() {
    // OpenGame treats every failed Game::Init alike, including a failure
    // after DoLobby returned: QuitGame clears the partial game and live
    // network before reconstructing the remembered startup dialog
    // (src/C4Application.cpp:373-400,442-451;
    // src/C4Game.cpp:452-477).
    let mut app = new_real_classic_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    app.mode = AppMode::Loading;
    app.network_lobby = None;
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let (manager, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    let (_sender, receiver) = mpsc::channel();
    app.loading_state = Some(ScenarioLoadingState::new(
        FrontendScenario::fallback(),
        app.assets.loader_resources().test_value(),
        HashMap::new(),
        Vec::new(),
        receiver,
    ));

    app.finish_scenario_loading_failure(
        "Unable to activate synchronized scenario".to_string(),
        true,
    )
    .test_value();

    assert_eq!(app.mode, AppMode::Menu);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_lobby.is_none());
    assert!(
        app.loading_state.is_none(),
        "the failed load ticket must not suppress a later client start"
    );
    assert_startup_error_log(&app, "Unable to activate synchronized scenario");
}

#[test]
fn direct_join_loading_failure_clears_session_before_exit() {
    // /join disables UseStartupDialog, so failed OpenGame does not create
    // another startup generation. QuitGame still clears the live network
    // before the application exits (src/C4Application.cpp:373-400,
    // 442-451; src/C4Game.cpp:452-477).
    let mut app = new_real_classic_menu_app(320, 200);
    app.classic_command_line.direct_join = Some("127.0.0.1:11112".to_string());
    app.mode = AppMode::Loading;
    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Client);

    app.finish_scenario_loading_failure(
        "Unable to activate direct-join scenario".to_string(),
        true,
    )
    .test_value();

    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.exit_requested);
}

#[test]
fn post_go_client_preparation_failure_clears_session_and_presents_startup_error() {
    // A client can leave DoLobby before its combined scenario has been
    // opened. Any later RetrieveScenario/InitGame failure unwinds the
    // partial network game through QuitGame; it is not an invisible
    // status line behind a deleted lobby (src/C4Network2.cpp:493-525;
    // src/C4Application.cpp:373-400,442-451).
    let dir = tempdir();
    let mut app = new_real_classic_menu_app(320, 200);
    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Client);
    app.startup_view = StartupView::NetworkLobby;
    app.mode = AppMode::Loading;
    app.network_lobby = None;
    let snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    let go = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: 0,
        target_tick: snapshot.dynamic_tick,
    };
    app.pending_client_start_status = Some(go);
    app.pending_network_join_data = Some(clonk_network::JoinDataEnvelope {
        client_id: 3,
        start_control_tick: snapshot.dynamic_tick,
        status: go,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    });
    app.client_combined_scenario_path = Some(dir.path().join("MissingCombined3.c4s"));

    app.prepare_client_network_scenario_if_ready().test_value();

    assert_eq!(app.mode, AppMode::Menu);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_lobby.is_none());
    assert_eq!(app.message_dialogs.len(), 1);
    assert_eq!(app.message_dialogs[0].state.caption(), "Error Log");
    assert!(
        app.message_dialogs[0]
            .state
            .message()
            .starts_with("Unable to prepare network scenario:"),
        "the native startup log owns the concrete preparation failure"
    );
    assert!(app.status_text.is_empty());
}

#[test]
fn fatal_worker_failure_during_running_round_changes_to_local_control() {
    // Clearing a failed client network during an active round invokes
    // ChangeToLocal: the frame/tick are retained, remote clients are
    // removed, and the round continues at local control rate one
    // (src/C4Network2.cpp:1825-1833;
    // src/C4GameControl.cpp:93-127).
    let mut app = new_state_only_running_sandbox_app();
    let local_client = 7;
    let local_player = app.local_owner;
    app.engine
        .test_player_mut(local_player)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.engine.set_network_game(true);
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(31, 4).test_value(),
    );
    app.snapshot = app.engine.snapshot();
    let (manager, events) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_control_clock = Some(NetworkControlClock::new(31, 4));
    app.control_clients = ControlClientRegistry::default();
    app.control_clients.register(0, true, false);
    app.control_clients.register(local_client, true, false);
    let frame_before = app.engine.frame();
    let control_tick_before = app.engine.sync_check(local_client).control_tick;
    events
        .send(NetworkEvent::TransportDiagnostic {
            client_id: Some(0),
            error: "host transport read failed".to_string(),
        })
        .test_value();
    events
        .send(NetworkEvent::FatalError(
            "network worker stopped unexpectedly".to_string(),
        ))
        .test_value();

    app.test_network_events();

    assert!(matches!(app.mode, AppMode::Running));
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_control_running);
    assert_eq!(app.engine.frame(), frame_before);
    assert_eq!(
        app.engine.sync_check(local_client).control_tick,
        control_tick_before
    );
    assert_eq!(app.engine.control_rate, 1);
    assert!(app.engine.player(local_player).is_some());
    let engine_results = app.engine.snapshot().round_results;
    assert_eq!(
        engine_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        engine_results.network_result_message,
        b"network worker stopped unexpectedly"
    );
    assert_eq!(app.snapshot.round_results, engine_results);
}

#[test]
fn fatal_worker_failure_during_game_over_preserves_the_network_halt() {
    // ChangeToLocal must not clear the halt owned by a shown game-over
    // dialog; otherwise a client resumes when its host disconnects
    // (src/C4GameControl.cpp:94-127;
    // src/C4GameOverDlg.cpp:349-381).
    let mut app = new_game_over_keyboard_app();
    let (events, _commands) = install_running_network_stub(&mut app, 7, 31, 4);
    app.network_control_running = false;
    events
        .send(NetworkEvent::FatalError(
            "host disconnected during evaluation".to_string(),
        ))
        .test_value();

    app.test_network_events();

    assert!(app.game_over_dialog.is_some());
    assert!(app.network.is_none());
    assert!(
        !app.network_control_running,
        "ChangeToLocal must not resume beneath the game-over dialog"
    );
    assert_ne!(
        app.offline_halt_count, 0,
        "the dialog-owned network hold transfers to local control"
    );
    let engine_results = app.engine.snapshot().round_results;
    assert_eq!(
        engine_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        engine_results.network_result_message,
        b"host disconnected during evaluation"
    );
    assert_eq!(app.snapshot.round_results, engine_results);

    app.handle_game_over_action(GameOverAction::Continue)
        .test_value();
    assert!(app.game_over_dialog.is_none());
    assert_eq!(app.offline_halt_count, 0);
    assert!(app.network_control_running);
}

#[test]
fn back_history_and_fresh_app_match_native_dialog_memory() {
    let mut backed_out = new_menu_app(640, 480);
    backed_out.open_scenario_browser();
    backed_out.scensel_do_back().test_value();
    assert_eq!(backed_out.startup_view, StartupView::MainMenu);
    assert_eq!(
        backed_out.last_startup_dialog,
        StartupDialog::ScenarioBrowser(ScenarioSelectorMode::Local)
    );
    backed_out
        .start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    confirm_abort_dialog(&mut backed_out);
    assert_l038_browser_return(&backed_out, ScenarioSelectorMode::Local);
    backed_out.scensel_do_back().test_value();
    assert_eq!(backed_out.startup_view, StartupView::MainMenu);
    assert_eq!(backed_out.last_startup_dialog, StartupDialog::MainMenu);

    let mut previous_session = running_browser_sandbox(ScenarioSelectorMode::Local);
    confirm_abort_dialog(&mut previous_session);
    assert_l038_browser_return(&previous_session, ScenarioSelectorMode::Local);
    drop(previous_session);

    let mut fresh = new_menu_app(640, 480);
    assert_eq!(fresh.startup_view, StartupView::MainMenu);
    assert_eq!(fresh.last_startup_dialog, StartupDialog::MainMenu);
    fresh
        .start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    fresh
        .handle_game_over_action(GameOverAction::End)
        .test_value();
    assert_eq!(fresh.startup_view, StartupView::MainMenu);
    assert_eq!(fresh.last_startup_dialog, StartupDialog::MainMenu);
}

#[test]
fn immediate_relaunch_retains_destination_without_background_discovery() {
    let mut app = new_menu_app(640, 480);
    app.open_network_game_dialog();
    app.open_network_lobby();
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();

    app.restart_current_scenario().test_value();

    assert!(matches!(app.mode, AppMode::Running));
    assert_eq!(app.last_startup_dialog, StartupDialog::NetworkGame);
    assert!(
        app.startup_game_search.is_none(),
        "restart must not leave startup discovery behind the new round"
    );
    confirm_abort_dialog(&mut app);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert_eq!(app.last_startup_dialog, StartupDialog::NetworkGame);
}

#[test]
fn refresh_generation_ignores_results_queued_before_worker_clear() {
    let mut app = new_classic_menu_app(800, 600);
    app.startup_network_refresh_waiting_for_clear = true;
    let stale = clonk_network::NetworkGameReference {
        title: "Stale queued result".to_string(),
        ..Default::default()
    };

    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::ReferencesUpdated(
        vec![stale],
    ))
    .test_value();
    assert!(app.startup_network_refresh_waiting_for_clear);
    assert!(app.startup_game_references.is_empty());

    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::Cleared)
        .test_value();
    assert!(!app.startup_network_refresh_waiting_for_clear);
    assert!(app.status_text.is_empty());

    let fresh = clonk_network::NetworkGameReference {
        title: "Fresh result".to_string(),
        ..Default::default()
    };
    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::ReferencesUpdated(
        vec![fresh.clone()],
    ))
    .test_value();
    assert_eq!(app.startup_game_references, [fresh]);
    assert!(app.status_text.is_empty());
}

#[test]
fn network_search_results_render_only_in_native_rows() {
    let mut app = new_real_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::MasterserverReply(
        clonk_network::MasterserverReplyInfo {
            game_count: 3,
            player_count: 5,
            ..Default::default()
        },
    ))
    .test_value();
    let references = (0..3)
        .map(|index| clonk_network::NetworkGameReference {
            title: format!("Discovered game {index}"),
            host_name: format!("Host {index}"),
            version: clonk_network::CURRENT_GAME_VERSION,
            build: clonk_network::CURRENT_GAME_BUILD,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::ReferencesUpdated(
        references,
    ))
    .test_value();

    assert!(app.status_text.is_empty());
    let dialog = app.startup_network_dialog.test_ref();
    assert_eq!(dialog.games().len(), 3);
    assert_eq!(dialog.masterserver_entry().details, "3 game(s) found.");

    let mut frame = vec![0x4d; 800 * 600 * 4];
    app.test_render(&mut frame);
    assert!(frame.iter().any(|byte| *byte != 0x4d));

    app.status_text = "Querying game infos…".to_string();
    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::ReferencesUpdated(
        Vec::new(),
    ))
    .test_value();
    assert_eq!(app.status_text, "Querying game infos…");
    frame.fill(0x6e);
    let error = app
        .render(&mut frame)
        .expect_err("the removed query sentinel cannot bypass generic status rejection");
    assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::StartupStatusOverlay {
            view: StartupView::NetworkGame,
            status,
        }) if status == "Querying game infos…"
    ));
    assert!(frame.iter().all(|byte| *byte == 0x6e));
}

#[test]
fn discovery_failure_opens_abort_modal_without_leaving_network_dialog() {
    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    let detail = "LAN discovery is unavailable: no multicast interface";

    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::SearchError {
        source: Some(clonk_network::ReferenceQuerySource::GameDiscovery),
        message: detail.to_string(),
    })
    .test_value();

    assert_eq!(app.message_dialogs.len(), 1);
    let modal = &app.message_dialogs[0].state;
    assert_eq!(modal.caption(), "Search Error");
    assert_eq!(modal.message(), format!("Search failed: {detail}"));
    assert_eq!(
        modal.buttons(),
        clonk_frontend::message_dialog::MessageDialogButtons::CANCEL
    );
    assert_eq!(
        modal.focused_button(),
        Some(clonk_frontend::message_dialog::MessageDialogButton::Cancel)
    );
    assert_eq!(
        modal.icon(),
        clonk_frontend::message_dialog::MessageDialogIcon::ERROR
    );
    assert!(app.status_text.is_empty());
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(!app.take_exit_request());

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    assert!(app.message_dialogs.is_empty());
    assert_eq!(app.startup_view, StartupView::NetworkGame);

    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::SearchError {
        source: Some(clonk_network::ReferenceQuerySource::Masterserver),
        message: "masterserver unavailable".to_string(),
    })
    .test_value();
    assert!(app.message_dialogs.is_empty());
    assert!(app.status_text.is_empty());
    let masterserver = app.startup_network_dialog.test_ref().masterserver_entry();
    assert_eq!(masterserver.details, "masterserver unavailable");
    assert_eq!(
        masterserver.row_icon,
        clonk_frontend::startup_netdlg::NetDlgRowIcon::Error
    );
    assert!(!app.take_exit_request());
}

#[test]
fn join_progress_lists_logical_routes_once_without_collapsing_dial_attempts() {
    // C++ expands a local address over every interface for NetIO.Connect,
    // but appends the original C4Network2Address only once to the progress
    // message (oracle-src-pinned src/C4Network2.cpp:375-405,412-423).
    let original = "[fe80::cafe]:11112".parse().test_value();
    let scoped = |protocol, scope_id| {
        clonk_network::NetworkAddress::new(
            protocol,
            std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
                "fe80::cafe".parse().test_value(),
                11_112,
                0,
                scope_id,
            )),
        )
    };
    let attempts = vec![
        scoped(clonk_network::NetworkProtocol::Tcp, 3),
        scoped(clonk_network::NetworkProtocol::Tcp, 7),
        scoped(clonk_network::NetworkProtocol::Udp, 3),
        scoped(clonk_network::NetworkProtocol::Udp, 7),
    ];
    let settings = ClientSettings::new(original, "Player").with_join_route_plan(
        clonk_network::NetworkJoinRoutePlan {
            logical_addresses: vec![
                clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Tcp, original),
                clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Udp, original),
            ],
            dial_attempts: attempts.clone(),
        },
    );

    assert_eq!(
        settings.server_addresses, attempts,
        "the network worker must retain every scoped dial attempt"
    );
    assert_eq!(
        startup_network_connect_targets(&settings),
        "TCP:[fe80::cafe]:11112, UDP:[fe80::cafe]:11112",
        "the dialog must list each original logical route exactly once"
    );

    let mut tcp_only = settings;
    tcp_only.mesh_udp_bind_address = None;
    assert_eq!(
        startup_network_connect_targets(&tcp_only),
        "TCP:[fe80::cafe]:11112",
        "C++ names only protocols for which NetIO.Connect accepted an attempt"
    );
}

#[test]
fn join_progress_names_target_and_dismisses_on_resolution() {
    use clonk_frontend::message_dialog::{
        MessageDialogButton, MessageDialogButtons, MessageDialogIcon, MessageDialogSize,
    };

    let mut app = new_real_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    app.startup_network_dialog
        .test_mut()
        .set_join_address("stale.example:11112");
    let (sender, receiver) = mpsc::channel();
    let target = "UDP:192.0.2.10:11112, TCP:192.0.2.10:11112";

    app.begin_startup_network_connection(
        receiver,
        StartupNetworkPurpose::Join,
        None,
        Some(StartupJoinTarget::Addresses(target.to_string())),
    )
    .test_value();

    assert!(app.startup_network_transition_active());
    assert_eq!(app.message_dialogs.len(), 1);
    let progress = &app.message_dialogs[0].state;
    assert_eq!(
        progress.message(),
        format!("Connecting to host on {target}...")
    );
    assert_eq!(progress.caption(), "Joining network game");
    assert_eq!(progress.buttons(), MessageDialogButtons::CANCEL);
    assert_eq!(progress.focused_button(), Some(MessageDialogButton::Cancel));
    assert_eq!(progress.icon(), MessageDialogIcon::Standard(3));
    assert_eq!(progress.size(), MessageDialogSize::Regular);
    assert!(app.status_text.is_empty());

    sender
        .send(Err(NetworkStartError::Other(
            "controlled connection failure".to_string(),
        )))
        .test_value();
    app.poll_startup_network_connection().test_value();

    assert!(app.startup_network_connection.is_none());
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("fresh network dialog")
            .join_address(),
        ""
    );
    assert_startup_error_log(
        &app,
        "Unable to start network session: controlled connection failure",
    );
    let mut frame = vec![0x4c; 800 * 600 * 4];
    app.test_render(&mut frame);
    assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn a_reference_join_names_the_game_rather_than_its_transport_addresses() {
    // C++ formats IDS_NET_CONNECTHOST from every endpoint it dials
    // (oracle-src-pinned 7d43b47b src/C4Network2.cpp:410-419). A masterserver
    // reference routinely advertises a dozen routes, and SetSourceAddress
    // (src/C4Network2Reference.cpp:37-47) rewrites the host's null address onto
    // one the reference already lists, so the modal fills with duplicated
    // transport addresses. The player picked a game by name; name that
    // (clonk-org/clonk-rs#204). The endpoints keep their C++ wording in the log.
    let mut app = new_classic_menu_app(800, 600);

    app.activate_network_reference_join(clonk_network::NetworkGameReference {
        title: "Clonk Party".to_string(),
        host_name: "archlinux".to_string(),
        source_address: "127.0.0.1:1".parse().test_value(),
        ..Default::default()
    })
    .test_value();

    let progress = &app.message_dialogs.last().test_value().state;
    assert_eq!(
        progress.message(),
        "Connecting to Clonk Party on archlinux..."
    );
    assert_eq!(progress.caption(), "Joining network game");
}

#[test]
fn a_password_prompt_does_not_cost_the_join_the_name_of_its_game() {
    // The reference is consumed when the prompt opens, so the game's name has
    // to survive on the settings that the accepted password launches. Losing it
    // here would silently drop a passworded join back to the address wall
    // clonk-org/clonk-rs#204 is about.
    let mut app = new_classic_menu_app(800, 600);

    app.activate_network_reference_join(clonk_network::NetworkGameReference {
        title: "Clonk Party".to_string(),
        host_name: "archlinux".to_string(),
        password_needed: true,
        source_address: "127.0.0.1:1".parse().test_value(),
        ..Default::default()
    })
    .test_value();
    assert!(app.game_option_input_dialog.is_some());

    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "hunter2".to_string(),
    )])
    .test_value();

    let progress = &app.message_dialogs.last().test_value().state;
    assert_eq!(
        progress.message(),
        "Connecting to Clonk Party on archlinux..."
    );
}

#[test]
fn escape_aborts_inflight_join_and_keeps_network_dialog() {
    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    let address: SocketAddr = "192.0.2.20:11112".parse().test_value();
    app.pending_network_join = Some(ClientSettings::new(address, "Player"));
    let (sender, receiver) = mpsc::channel();

    app.begin_startup_network_connection(
        receiver,
        StartupNetworkPurpose::Join,
        None,
        Some(StartupJoinTarget::Addresses(address.to_string())),
    )
    .test_value();
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

    assert!(app.startup_network_connection.is_none());
    assert!(app.pending_network_join.is_none());
    assert!(app.message_dialogs.is_empty());
    assert!(app.status_text.is_empty());
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.startup_network_dialog.is_some());
    assert!(app
        .message_dialog_consumed_keys
        .contains(&VirtualKeyCode::Escape));
    assert!(sender
        .send(Err(NetworkStartError::Other("stale result".to_string())))
        .is_err());

    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    app.poll_startup_network_connection().test_value();
    assert!(app.message_dialog_consumed_keys.is_empty());
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.status_text.is_empty());
}

#[test]
fn cancel_button_aborts_inflight_join() {
    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    let (sender, receiver) = mpsc::channel();
    app.begin_startup_network_connection(
        receiver,
        StartupNetworkPurpose::Join,
        None,
        Some(StartupJoinTarget::Addresses(
            "198.51.100.40:11112".to_string(),
        )),
    )
    .test_value();
    let cancel = app
        .top_message_dialog_layout()
        .test_value()
        .buttons
        .first()
        .test_value()
        .rect;
    let point = PhysicalPosition::new(
        f64::from(cancel.x + cancel.w / 2),
        f64::from(cancel.y + cancel.h / 2),
    );

    app.test_cursor(point);
    app.test_left_button(ElementState::Pressed);
    assert!(app.startup_network_connection.is_some());
    app.test_left_button(ElementState::Released);

    assert!(app.startup_network_connection.is_none());
    assert!(app.message_dialogs.is_empty());
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(sender
        .send(Err(NetworkStartError::Other("stale result".to_string())))
        .is_err());
}

#[test]
fn network_connection_progress_cancel_interrupts_inflight_transport() {
    use std::io::Read;
    use std::net::{TcpListener, UdpSocket};

    fn stalled_handshake_server() -> (
        SocketAddr,
        Receiver<()>,
        Receiver<()>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).test_value();
        let address = listener.local_addr().test_value();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().test_value();
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .test_value();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut bytes = [0_u8; 512];
            loop {
                match stream.read(&mut bytes) {
                    Ok(0) => panic!("startup transport closed before its handshake"),
                    Ok(_) => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) && Instant::now() < deadline =>
                    {
                        continue;
                    }
                    Err(error) => panic!("startup handshake read failed: {error}"),
                }
            }
            accepted_tx.send(()).test_value();
            loop {
                match stream.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) && Instant::now() < deadline =>
                    {
                        continue;
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset
                        ) =>
                    {
                        break;
                    }
                    Err(error) => panic!("stalled startup transport read failed: {error}"),
                }
            }
            closed_tx.send(()).test_value();
        });
        (address, accepted_rx, closed_rx, worker)
    }

    fn reserve_mesh_addresses() -> (SocketAddr, SocketAddr) {
        let tcp = TcpListener::bind(("127.0.0.1", 0)).test_value();
        let tcp_address = tcp.local_addr().test_value();
        let udp = UdpSocket::bind(("127.0.0.1", 0)).test_value();
        let udp_address = udp.local_addr().test_value();
        (tcp_address, udp_address)
    }

    fn cancellable_settings(
        server: SocketAddr,
        mesh_tcp: SocketAddr,
        mesh_udp: SocketAddr,
    ) -> ClientSettings {
        let mut settings = ClientSettings::new(server, "Player").with_netpuncher(
            server.to_string(),
            clonk_network::NetpuncherGameIds { ipv4: 17, ipv6: 0 },
        );
        settings.mesh_tcp_bind_address = Some(mesh_tcp);
        settings.mesh_udp_bind_address = Some(mesh_udp);
        settings
    }

    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);

    let (first_server, first_accepted, first_closed, first_server_worker) =
        stalled_handshake_server();
    app.activate_network_join(first_server.to_string())
        .test_value();
    first_accepted
        .recv_timeout(Duration::from_secs(4))
        .test_value();

    // Cancel and Escape use this same continuation and are retained in
    // the adjacent raw-connect progress-dialog input-route regressions.
    let close = app
        .top_message_dialog_layout()
        .test_value()
        .close_button
        .test_value();
    let close_point = PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    );
    app.test_cursor(close_point);
    app.test_left_button(ElementState::Pressed);
    assert!(app.startup_network_connection.is_some());
    let cancellation_started = Instant::now();
    app.test_left_button(ElementState::Released);
    assert!(
        cancellation_started.elapsed() < Duration::from_secs(1),
        "transport cancellation must not wait for the stalled handshake"
    );

    first_closed
        .recv_timeout(Duration::from_secs(1))
        .test_value();
    first_server_worker.join().test_value();
    assert!(app.startup_network_connection.is_none());
    assert!(app.pending_network_join.is_none());
    assert!(app.message_dialogs.is_empty());
    assert!(app.status_text.is_empty());
    app.poll_startup_network_connection().test_value();
    assert!(app.message_dialogs.is_empty());

    let (second_server, second_accepted, second_closed, second_server_worker) =
        stalled_handshake_server();
    let (second_mesh_tcp, second_mesh_udp) = reserve_mesh_addresses();
    app.pending_network_join = Some(cancellable_settings(
        second_server,
        second_mesh_tcp,
        second_mesh_udp,
    ));
    app.launch_pending_network_join().test_value();
    second_accepted
        .recv_timeout(Duration::from_secs(4))
        .test_value();

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    second_closed
        .recv_timeout(Duration::from_secs(1))
        .test_value();
    second_server_worker.join().test_value();
    let rebound_tcp = TcpListener::bind(second_mesh_tcp).test_value();
    let rebound_udp = UdpSocket::bind(second_mesh_udp).test_value();
    drop((rebound_tcp, rebound_udp));
    app.poll_startup_network_connection().test_value();
    assert!(app.message_dialogs.is_empty());
    assert!(app.status_text.is_empty());
}

#[test]
fn network_reference_projects_five_lines_and_native_status_order() {
    use clonk_frontend::startup_netdlg::{NetDlgRowIcon, NetDlgStatusIcon};

    let reference = clonk_network::NetworkGameReference {
        icon: 7,
        title: "Rage".to_string(),
        host_name: "Ada".to_string(),
        state: "Running".to_string(),
        time: 3_723,
        comment: "Bring a friend".to_string(),
        join_allowed: true,
        password_needed: true,
        official_server: true,
        use_fair_crew: true,
        goals: vec!["Gold".to_string(), "Elimination".to_string()],
        league: "League game".to_string(),
        league_address: "https://league.example/".to_string(),
        max_players: 4,
        player_names: vec!["Ada".to_string(), "Bob".to_string()],
        game: "LegacyClonk".to_string(),
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        tcp_addresses: vec!["127.0.0.1:11112".parse().unwrap()],
        ..Default::default()
    };

    let row = GameApp::startup_network_reference_row(&reference);
    assert_eq!(row.title, "Rage on Ada");
    assert_eq!(
        row.details,
        "2/4 players - Goals: Gold, Elimination - Game is running - 01:02:03"
    );
    assert_eq!(
        row.extra_lines,
        [
            "Engine version: LegacyClonk 4.9.11.0 [362]",
            "Comment: Bring a friend",
            "Player: Ada, Bob",
        ]
    );
    assert_eq!(
        row.status_icons,
        [
            NetDlgStatusIcon::PasswordNeeded,
            NetDlgStatusIcon::League,
            NetDlgStatusIcon::Running,
            NetDlgStatusIcon::RuntimeJoin,
            NetDlgStatusIcon::FairCrew,
            NetDlgStatusIcon::OfficialServer,
        ]
    );
    assert_eq!(row.row_icon, NetDlgRowIcon::Scenario(7));
    assert_eq!(row.address.as_deref(), Some("127.0.0.1:11112"));
    assert!(row.joinable);

    let alternate = GameApp::startup_network_reference_row_with_config(
        &embedded_runtime_language_table().entries,
        true,
        &reference,
    );
    assert!(!alternate
        .status_icons
        .contains(&NetDlgStatusIcon::OfficialServer));
}

#[test]
fn lan_query_rows_resolve_fail_and_expire_without_modal() {
    use clonk_frontend::startup_netdlg::NetDlgRowIcon;

    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    let address: SocketAddr = "127.0.0.1:30111".parse().test_value();
    app.apply_startup_game_search_event(
        clonk_network::StartupGameSearchEvent::GameDiscoveryQueryStarted { address },
    )
    .test_value();
    assert_eq!(app.startup_discovery_reference_queries.len(), 1);
    let row = &app.startup_network_dialog.as_ref().test_value().games()[0];
    assert_eq!(row.title, "Local network on 127.0.0.1:30111");
    assert_eq!(row.row_icon, NetDlgRowIcon::Query);

    app.startup_direct_reference_queries
        .push(StartupDirectReferenceQuery {
            id: 40,
            address: "direct.example".to_string(),
            state: StartupDirectReferenceQueryState::Pending,
            expires_at: None,
        });
    app.sync_startup_network_game_rows();
    assert!(matches!(
        app.startup_network_join_target(0),
        Some(StartupNetworkJoinTarget::DirectAddress(target)) if target == address.to_string()
    ));
    assert!(matches!(
        app.startup_network_join_target(1),
        Some(StartupNetworkJoinTarget::DirectAddress(target)) if target == "direct.example"
    ));
    app.startup_direct_reference_queries.clear();

    app.apply_startup_game_search_event(
        clonk_network::StartupGameSearchEvent::GameDiscoveryQueryFailed {
            address,
            message: "reference connection refused".to_string(),
        },
    )
    .test_value();
    assert!(app.message_dialogs.is_empty());
    let row = &app.startup_network_dialog.as_ref().test_value().games()[0];
    assert_eq!(row.details, "reference connection refused");
    assert_eq!(row.row_icon, NetDlgRowIcon::Error);
    let expires_at = app.startup_discovery_reference_queries[0]
        .expires_at
        .test_value();
    app.tick_startup_network_query_rows_at(expires_at);
    assert!(app.startup_discovery_reference_queries.is_empty());
    assert!(app
        .startup_network_dialog
        .as_ref()
        .unwrap()
        .games()
        .is_empty());

    app.apply_startup_game_search_event(
        clonk_network::StartupGameSearchEvent::GameDiscoveryQueryStarted { address },
    )
    .test_value();
    // Repeated multicast replies do not duplicate a live native query.
    app.apply_startup_game_search_event(
        clonk_network::StartupGameSearchEvent::GameDiscoveryQueryStarted { address },
    )
    .test_value();
    assert_eq!(app.startup_discovery_reference_queries.len(), 1);
    let reference = clonk_network::NetworkGameReference {
        icon: 12,
        title: "LAN game".to_string(),
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        ..Default::default()
    };
    app.apply_startup_game_search_event(
        clonk_network::StartupGameSearchEvent::GameDiscoveryQueryResolved {
            address,
            references: vec![reference.clone()],
            selected_index: Some(0),
        },
    )
    .test_value();
    assert!(app.startup_discovery_reference_queries.is_empty());
    assert_eq!(app.startup_game_references, [reference]);
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().selected_game(),
        None,
        "game discovery must not explicitly select its returned reference"
    );

    app.apply_startup_game_search_event(
        clonk_network::StartupGameSearchEvent::GameDiscoveryQueryFailed {
            address,
            message: "late duplicate failure".to_string(),
        },
    )
    .test_value();
    assert!(app.startup_discovery_reference_queries.is_empty());
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().games().len(),
        1
    );

    let mut selection_app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut selection_app);
    selection_app.begin_startup_discovery_reference_query(address);
    selection_app
        .startup_direct_reference_queries
        .push(StartupDirectReferenceQuery {
            id: 41,
            address: "selected-direct.example".to_string(),
            state: StartupDirectReferenceQueryState::Pending,
            expires_at: None,
        });
    selection_app.sync_startup_network_game_rows();
    selection_app.focus_startup_direct_reference_query(41);
    assert_eq!(
        selection_app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .selected_game(),
        Some(1)
    );
    let second_address: SocketAddr = "127.0.0.1:30112".parse().test_value();
    selection_app.begin_startup_discovery_reference_query(second_address);
    assert_eq!(
        selection_app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .selected_game(),
        Some(2),
        "inserting a LAN row before direct rows retains direct-query identity"
    );
    selection_app.finish_startup_discovery_reference_query(
        second_address,
        vec![clonk_network::NetworkGameReference {
            title: "Second LAN game".to_string(),
            version: clonk_network::CURRENT_GAME_VERSION,
            build: clonk_network::CURRENT_GAME_BUILD,
            ..Default::default()
        }],
        true,
    );
    assert_eq!(
        selection_app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .selected_game(),
        Some(2),
        "resolving a LAN row retains selected direct-query identity"
    );
    assert!(matches!(
        selection_app.startup_network_join_target(2),
        Some(StartupNetworkJoinTarget::DirectAddress(target))
            if target == "selected-direct.example"
    ));

    let mut retry_app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut retry_app);
    retry_app.begin_startup_discovery_reference_query(address);
    let failed_id = retry_app.startup_discovery_reference_queries[0].id;
    retry_app.fail_startup_discovery_reference_query(address, "first request failed".to_string());
    retry_app.begin_startup_discovery_reference_query(address);
    let retry_id = retry_app.startup_discovery_reference_queries[1].id;
    assert_ne!(failed_id, retry_id);
    retry_app.focus_startup_discovery_reference_query(retry_id);
    retry_app
        .apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::ReferencesUpdated(
            vec![clonk_network::NetworkGameReference {
                title: "Other LAN game".to_string(),
                version: clonk_network::CURRENT_GAME_VERSION,
                build: clonk_network::CURRENT_GAME_BUILD,
                ..Default::default()
            }],
        ))
        .test_value();
    assert_eq!(
        retry_app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .selected_game(),
        Some(2),
        "a selected pending retry must not retarget to an older failed row for the same address"
    );
    assert_eq!(
        retry_app.selected_startup_discovery_reference_query_id(),
        Some(retry_id)
    );
}

#[test]
fn resolved_game_selection_tracks_host_and_address_identity() {
    let reference = |title: &str, host: &str, address: &str| clonk_network::NetworkGameReference {
        title: title.to_string(),
        host_name: host.to_string(),
        tcp_addresses: vec![address.parse().unwrap()],
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        ..Default::default()
    };
    let a = reference("A", "Host A", "127.0.0.1:31001");
    let b = reference("B", "Host B", "127.0.0.1:31002");
    let c = reference("C", "Host C", "127.0.0.1:31003");
    let mut b_later = b.clone();
    b_later.title = "B later".to_string();
    b_later.start_time = 2;
    let mut identity_app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut identity_app);
    identity_app.startup_game_references = vec![b_later, b.clone()];
    identity_app.sync_startup_network_game_rows();
    identity_app.focus_startup_game_reference(&b);
    assert_eq!(
        identity_app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .selected_game(),
        Some(1),
        "exact reference identity precedes the host/address fallback"
    );

    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    app.startup_game_references = vec![a.clone(), b.clone()];
    app.sync_startup_network_game_rows();
    app.startup_network_dialog
        .as_mut()
        .test_value()
        .focus_game(1);

    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::ReferencesUpdated(
        vec![b.clone(), a.clone()],
    ))
    .test_value();
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().selected_game(),
        Some(0)
    );
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().games()[0].title,
        "B on Host B"
    );

    let discovery_address: SocketAddr = "127.0.0.1:30113".parse().test_value();
    app.begin_startup_discovery_reference_query(discovery_address);
    app.finish_startup_discovery_reference_query(
        discovery_address,
        vec![a.clone(), b.clone(), c.clone()],
        true,
    );
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().selected_game(),
        Some(1),
        "LAN resolution keeps the selected resolved game"
    );

    app.startup_direct_reference_queries
        .push(StartupDirectReferenceQuery {
            id: 42,
            address: "direct-selection.example".to_string(),
            state: StartupDirectReferenceQueryState::Pending,
            expires_at: None,
        });
    app.sync_startup_network_game_rows();
    app.finish_startup_direct_reference_query(42, vec![c, b, a], Some(0));
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().selected_game(),
        Some(1),
        "an unselected direct query cannot hijack resolved-game selection"
    );

    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::ReferencesUpdated(
        vec![
            reference("A newer", "Host A", "127.0.0.1:31001"),
            reference("C newer", "Host C", "127.0.0.1:31003"),
        ],
    ))
    .test_value();
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().selected_game(),
        Some(1),
        "removing a selected resolved reference selects its next native sibling"
    );
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().games()[1].title,
        "C newer on Host C"
    );
}

#[test]
fn network_message_modal_freezes_search_events_and_expiry_until_close() {
    use clonk_frontend::startup_netdlg::NetDlgRowIcon;

    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    let nearly_expired_reference = clonk_network::NetworkGameReference {
        title: "Nearly expired game".to_string(),
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        ..Default::default()
    };
    app.startup_game_references = vec![nearly_expired_reference.clone()];
    let expired_query_id = 70;
    app.startup_direct_reference_queries
        .push(StartupDirectReferenceQuery {
            id: expired_query_id,
            address: "expired.example".to_string(),
            state: StartupDirectReferenceQueryState::Empty,
            expires_at: Instant::now().checked_sub(Duration::from_secs(1)),
        });
    app.sync_startup_network_game_rows();
    // clonk-network's fake-time coverage proves that the worker emits this
    // removal at the native 42-second reference deadline. Inject the
    // resulting event here so modal/backlog behavior is deterministic.
    app.startup_game_search_test_events.push_back(
        clonk_network::StartupGameSearchEvent::ReferencesUpdated(Vec::new()),
    );

    app.set_startup_masterserver_error("stale query error".to_string());
    let overdue_refresh = Instant::now()
        .checked_sub(clonk_network::GAME_SEARCH_INTERVAL + Duration::from_secs(1))
        .test_value();
    let overdue_masterserver_query = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .test_value();
    app.startup_network_last_refresh = Some(overdue_refresh);
    app.startup_masterserver_next_query_at = Some(overdue_masterserver_query);
    app.request_network_reference_join(clonk_network::NetworkGameReference {
        game: "LegacyClonk".to_string(),
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        join_allowed: false,
        ..Default::default()
    })
    .test_value();
    assert_eq!(app.message_dialogs.len(), 1);

    app.poll_startup_game_search().test_value();
    assert_eq!(app.startup_game_search_test_events.len(), 1);
    assert_eq!(
        app.startup_game_references.as_slice(),
        std::slice::from_ref(&nearly_expired_reference)
    );
    assert!(app
        .startup_direct_reference_queries
        .iter()
        .any(|query| query.id == expired_query_id));
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .games()
            .len(),
        2
    );
    assert_eq!(app.startup_network_last_refresh, Some(overdue_refresh));
    assert_eq!(
        app.startup_masterserver_next_query_at,
        Some(overdue_masterserver_query)
    );
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .masterserver_entry()
            .row_icon,
        NetDlgRowIcon::Error
    );

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();
    app.poll_startup_game_search().test_value();
    assert!(app.startup_game_search_test_events.is_empty());
    assert!(app.startup_game_references.is_empty());
    assert!(app
        .startup_network_dialog
        .as_ref()
        .expect("network dialog")
        .games()
        .is_empty());
    assert!(!app
        .startup_direct_reference_queries
        .iter()
        .any(|query| query.id == expired_query_id));
    assert_eq!(app.startup_network_last_refresh, Some(overdue_refresh));
    assert!(app.startup_masterserver_next_query_at.is_none());
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .masterserver_entry()
            .row_icon,
        NetDlgRowIcon::Query
    );

    // A search event can create the covering modal itself. Later events
    // must remain in the receiver rather than being pre-drained or lost.
    app.startup_game_references = vec![nearly_expired_reference.clone()];
    app.sync_startup_network_game_rows();
    app.startup_game_search_test_events.extend([
        clonk_network::StartupGameSearchEvent::SearchError {
            source: Some(clonk_network::ReferenceQuerySource::GameDiscovery),
            message: "discovery failed".to_string(),
        },
        clonk_network::StartupGameSearchEvent::ReferencesUpdated(Vec::new()),
    ]);
    app.poll_startup_game_search().test_value();
    assert_eq!(app.message_dialogs.len(), 1);
    assert_eq!(app.startup_game_search_test_events.len(), 1);
    assert_eq!(app.startup_game_references, [nearly_expired_reference]);

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    app.poll_startup_game_search().test_value();
    assert!(app.startup_game_search_test_events.is_empty());
    assert!(app.startup_game_references.is_empty());
}

#[test]
fn an_unflushed_internet_toggle_outranks_the_config_file() {
    // `OnBtnInternet` flips `Config.Network.MasterServerSignUp` in memory and
    // calls UpdateMasterserver; the file is written only by
    // `C4Application::Clear`. `OnShown` -> `UpdateMasterserver` and the
    // btnInternet icon both read that same in-memory value, so the toggle
    // survives every dialog switch (pinned oracle 7d43b47b
    // src/C4StartupNetDlg.cpp:710,771-777,838-845,851-866).
    let _lock = env_lock().lock();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(repository)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    if let Some(parent) = paths.config_file().parent() {
        fs::create_dir_all(parent).test_value();
    }
    fs::write(
        paths.config_file(),
        b"[Network]\r\nMasterServerSignUp=1\r\n",
    )
    .test_value();

    let mut app = new_classic_menu_app(800, 600);
    app.app_paths = Some(paths.clone());
    app.open_network_game_dialog();
    assert!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .config()
            .masterserver_signup
    );

    // The controller flips its own flag before emitting the action.
    app.startup_network_dialog
        .test_mut()
        .sync_masterserver_signup_from_config(false);
    app.process_network_dialog_actions(vec![
        clonk_frontend::startup_netdlg::NetDlgAction::MasterserverSignupChanged(false),
    ])
    .test_value();
    assert_eq!(
        app.deferred_config.get("Network", "MasterServerSignUp"),
        Some("0")
    );
    assert!(
        load_network_startup_settings(Some(&paths)).0,
        "the toggle is deliberately not written through, so the file still reads enabled"
    );

    app.refresh_retained_network_dialog_internet();
    assert!(
        !app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .config()
            .masterserver_signup,
        "re-showing the retained dialog must not resurrect the on-disk value"
    );

    app.open_network_game_dialog();
    assert!(
        !app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .config()
            .masterserver_signup,
        "rebuilding the dialog must not resurrect the on-disk value either"
    );

    // The scenario selector's Internet checkbox saves immediately, which is one
    // of the surfaces C++ also writes straight through, so it supersedes the
    // netdlg's unflushed change instead of being shadowed by it.
    app.persist_game_option_value("Network", "MasterServerSignUp", "1".to_string());
    assert_eq!(
        app.deferred_config.get("Network", "MasterServerSignUp"),
        None
    );
    assert!(app.masterserver_signup_setting());
}

#[test]
fn the_periodic_masterserver_requery_keeps_the_reply_it_is_refreshing() {
    use clonk_frontend::startup_netdlg::{NetDlgRowIcon, NetDlgTextLine};

    // The masterserver branch of `C4StartupNetListEntry::Execute` re-queries
    // without calling UpdateText or UpdateSmallState — the only call sites are
    // :161, :267, :280, :298-299, :359, :503 and :513 — so the labels keep the
    // previous reply's game count, message of the day and hyperlink, and only
    // the icon returns to the animated fctNetGetRef facet (pinned oracle
    // 7d43b47b src/C4StartupNetDlg.cpp:191-207).
    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::MasterserverReply(
        clonk_network::MasterserverReplyInfo {
            motd: "Update available.".to_string(),
            motd_url: "https://clonkspot.example/lc-en".to_string(),
            game_count: 3,
            player_count: 5,
            ..Default::default()
        },
    ))
    .test_value();

    let replied = app
        .startup_network_dialog
        .test_ref()
        .masterserver_entry()
        .clone();
    assert_eq!(replied.details, "3 game(s) found.");
    assert_eq!(replied.row_icon, NetDlgRowIcon::QueryStatic);
    assert!(matches!(
        replied.extra_lines.as_slice(),
        [NetDlgTextLine::Plain(_), NetDlgTextLine::Hyperlink { .. }]
    ));

    let requery_at = app.startup_masterserver_next_query_at.test_value();
    app.tick_startup_network_query_rows_at(requery_at);

    let requerying = app.startup_network_dialog.test_ref().masterserver_entry();
    assert_eq!(
        requerying.row_icon,
        NetDlgRowIcon::Query,
        "the re-query animates the icon"
    );
    assert_eq!(
        requerying.details, replied.details,
        "the re-query leaves the previous game count on screen"
    );
    assert_eq!(
        requerying.extra_lines, replied.extra_lines,
        "the re-query keeps the message of the day and its hyperlink"
    );
    assert_eq!(
        requerying.title, replied.title,
        "the re-query leaves sInfoText[0] alone"
    );
    // `QueryReferences` re-arms iRequestTimeout and the branch clears iTimeout
    // (src/C4StartupNetDlg.cpp:182,203-204).
    assert_eq!(
        app.startup_masterserver_request_timeout_at,
        Some(requery_at + clonk_network::REFERENCE_QUERY_TIMEOUT)
    );
    assert!(app.startup_masterserver_next_query_at.is_none());
}

#[test]
fn masterserver_row_reports_a_reference_request_timeout_when_no_reply_arrives() {
    use clonk_frontend::startup_netdlg::NetDlgRowIcon;

    // `C4StartupNetListEntry::Execute` bounds how long the masterserver row may
    // sit on IDS_NET_INFOQUERY: once the request has been outstanding for
    // `C4NetRefRequestTimeout` it becomes an IDS_NET_ERR_REFREQTIMEOUT error row
    // and `SetTimeout(TT_RefReqWait)` schedules the next query one
    // `C4NetMasterServerQueryInterval` later (pinned oracle 7d43b47b
    // src/C4StartupNetDlg.cpp:182,216-223,525; src/C4StartupNetDlg.h:27-28).
    let mut app = new_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    // The helper leaves `startup_game_search` unset, so no worker reply can ever
    // arrive — the state the native watchdog exists to recover from.
    let armed_at = Instant::now();
    app.reset_startup_masterserver_entry_at(armed_at);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .masterserver_entry()
            .row_icon,
        NetDlgRowIcon::Query
    );

    app.tick_startup_network_query_rows_at(
        armed_at + clonk_network::REFERENCE_QUERY_TIMEOUT - Duration::from_secs(1),
    );
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .masterserver_entry()
            .row_icon,
        NetDlgRowIcon::Query,
        "the row keeps querying until the native request timeout elapses"
    );

    let timed_out_at = armed_at + clonk_network::REFERENCE_QUERY_TIMEOUT;
    app.tick_startup_network_query_rows_at(timed_out_at);
    let entry = app
        .startup_network_dialog
        .test_ref()
        .masterserver_entry()
        .clone();
    assert_eq!(entry.row_icon, NetDlgRowIcon::Error);
    assert_eq!(entry.details, "Reference request timed out");
    assert_eq!(
        app.startup_masterserver_next_query_at,
        Some(timed_out_at + clonk_network::GAME_SEARCH_INTERVAL),
        "the timed-out row schedules the next masterserver query"
    );
}

#[test]
fn network_game_list_wheel_and_held_arrow_route_through_app() {
    use clonk_frontend::startup_netdlg::{
        net_dlg_layout, NetDlgConfig, NetDlgController, NetDlgFontMetrics, NetDlgGameEntry,
    };

    let mut app = new_real_classic_menu_app(640, 480);
    let metrics = NetDlgFontMetrics::from_fonts(app.assets.clonk_fonts.as_deref().test_value());
    let layout = net_dlg_layout(640, 480, &metrics);
    let mut controller = NetDlgController::new(
        NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    controller.resize(640, 480);
    controller.set_games(
        (0..32)
            .map(|index| NetDlgGameEntry {
                title: format!("Game {index}"),
                details: String::new(),
                extra_lines: Vec::new(),
                status_icons: Vec::new(),
                row_icon: clonk_frontend::startup_netdlg::NetDlgRowIcon::None,
                address: None,
                joinable: true,
            })
            .collect(),
    );
    assert!(controller.list_max_scroll() > 60);
    app.startup_view = StartupView::NetworkGame;
    app.startup_network_dialog = Some(controller);
    app.startup_game_search = None;

    app.test_cursor(PhysicalPosition::new(
        f64::from(layout.list_viewport.x + 4),
        f64::from(layout.list_viewport.y + 4),
    ));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .list_scroll_offset(),
        60
    );
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .list_scroll_offset(),
        0
    );

    app.test_cursor(PhysicalPosition::new(
        f64::from(layout.list_scrollbar.x + 8),
        f64::from(layout.list_scrollbar.y + layout.list_scrollbar.h - 8),
    ));
    app.test_left_button(ElementState::Pressed);
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);
    let first = app.startup_network_dialog.test_ref().list_scroll_offset();
    app.test_render(&mut frame);
    let second = app.startup_network_dialog.test_ref().list_scroll_offset();
    assert!(first > 0 && second > first);
    app.test_left_button(ElementState::Released);
    app.test_render(&mut frame);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .list_scroll_offset(),
        second
    );

    app.test_left_button(ElementState::Pressed);
    let before_transition = app.startup_network_dialog.test_ref().list_scroll_offset();
    let (_sender, receiver) = mpsc::channel();
    app.begin_startup_network_connection(
        receiver,
        StartupNetworkPurpose::Join,
        None,
        Some(StartupJoinTarget::Addresses("127.0.0.1:11112".to_string())),
    )
    .test_value();
    assert!(
        !app.startup_network_dialog
            .as_mut()
            .expect("network dialog")
            .tick_scrollbar(),
        "transition start must cancel the held arrow"
    );
    app.status_text.clear();
    app.test_render(&mut frame);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .list_scroll_offset(),
        before_transition
    );
    app.startup_network_connection = None;
    app.dismiss_startup_network_connect_progress();
    app.test_render(&mut frame);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .list_scroll_offset(),
        before_transition
    );
}

#[test]
fn network_join_without_reference_opens_the_classic_error_dialog() {
    // C4StartupNetDlg::DoOK shows a modal MessageDialog when no list
    // reference/direct address is selected (C4StartupNetDlg.cpp:992-1004).
    let _lock = env_lock().lock();
    let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    let mut app = GameApp::new(
        1280,
        720,
        AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        },
        Some(&paths),
        RuntimeConfig {
            player_owner: 1,
            player_name: "Player".to_string(),
            network: None,
            record_enabled: false,
        },
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_network_game_dialog();

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);

    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.startup_network_connection.is_none());
    assert_eq!(app.message_dialogs.len(), 1);
    let dialog = &app.message_dialogs[0].state;
    assert_eq!(dialog.caption(), "Cannot join game");
    assert_eq!(
        dialog.message(),
        "No reference selected. Select a game from the list or enter a direct join address below!"
    );
    assert_eq!(
        dialog.buttons(),
        clonk_frontend::message_dialog::MessageDialogButtons::OK
    );
    assert_eq!(
        dialog.icon(),
        clonk_frontend::message_dialog::MessageDialogIcon::ERROR
    );

    // Input outside the dialog must not leak to the underlying Back
    // button while the modal is active.
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let back = clonk_frontend::startup_netdlg::net_dlg_layout(1280, 720, &metrics).buttons[0];
    let back_point = PhysicalPosition::new(
        f64::from(back.x + back.w / 2),
        f64::from(back.y + back.h / 2),
    );
    app.test_cursor(back_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert_eq!(app.message_dialogs.len(), 1);

    let mut frame = vec![0_u8; 1280 * 720 * 4];
    app.test_render(&mut frame);

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert_eq!(
        app.message_dialogs.len(),
        1,
        "Return must show the button-down frame before activation"
    );
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert!(app.message_dialogs.is_empty());
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.startup_network_connection.is_none());
    reset_cached_app_paths();
}

#[test]
fn network_direct_address_enter_adds_query_row_and_focuses_list_without_joining() {
    let mut app = new_classic_menu_app(800, 600);
    app.open_network_game_dialog();
    let address = "127.0.0.1:9".to_string();
    let actions = {
        let dialog = app.startup_network_dialog.test_mut();
        dialog.set_join_address(address.clone());
        assert_eq!(
            dialog.handle_key_down(KeyCode::Tab),
            [clonk_frontend::startup_netdlg::NetDlgAction::FocusChanged(
                clonk_frontend::startup_netdlg::NetDlgControl::JoinAddress,
            )]
        );
        dialog.handle_key_down(KeyCode::Enter)
    };
    assert_eq!(
        actions,
        [
            clonk_frontend::startup_netdlg::NetDlgAction::QueryAddress {
                address: address.clone(),
            },
            clonk_frontend::startup_netdlg::NetDlgAction::FocusChanged(
                clonk_frontend::startup_netdlg::NetDlgControl::GameList,
            ),
        ]
    );

    app.process_network_dialog_actions(actions).test_value();

    let dialog = app.startup_network_dialog.as_ref().test_value();
    assert_eq!(
        dialog.focused_control(),
        clonk_frontend::startup_netdlg::NetDlgControl::GameList
    );
    assert_eq!(dialog.selected_game(), Some(0));
    assert_eq!(dialog.games().len(), 1);
    assert_eq!(dialog.games()[0].address.as_deref(), Some(address.as_str()));
    assert_eq!(app.startup_direct_reference_queries.len(), 1);
    assert_eq!(
        app.startup_direct_reference_queries[0].state,
        StartupDirectReferenceQueryState::Pending
    );
    assert!(app.pending_network_join.is_none());
    assert!(app.startup_network_connection.is_none());
    assert!(app.network.is_none());

    // Cancel the deliberately unreachable reference query, then exercise
    // the unresolved row's second-Enter raw-join fallback.
    app.startup_game_search = None;
    app.active_scenario = Some(FrontendScenario::fallback());
    app.active_definition_load = Some(ScenarioDefinitionLoad::Fixed {
        modules: vec!["Stale.c4d".to_string()],
        definition_root: None,
    });
    let actions = app
        .startup_network_dialog
        .as_mut()
        .test_value()
        .handle_key_down(KeyCode::Enter);
    assert_eq!(
        actions,
        [clonk_frontend::startup_netdlg::NetDlgAction::JoinGame {
            address: Some(address),
        }]
    );
    app.process_network_dialog_actions(actions).test_value();
    assert!(app.active_scenario.is_none());
    match app.active_definition_load.as_ref() {
        Some(ScenarioDefinitionLoad::Seed { modules, .. }) => {
            assert_eq!(modules, &["Objects.c4d".to_string()]);
        }
        other => panic!("direct join must install the Objects seed, got {other:?}"),
    }
}

#[test]
fn network_join_edit_routes_window_keys_pointer_selection_and_context() {
    let mut app = new_classic_menu_app(800, 600);
    app.open_network_game_dialog();
    app.startup_network_dialog
        .test_mut()
        .set_join_address("replace me");

    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .unwrap()
            .join_address_selection(),
        Some((0, "replace me".len()))
    );

    app.test_text_input('|');
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().join_address(),
        "\u{a6}"
    );

    app.startup_network_dialog
        .as_mut()
        .test_value()
        .set_join_address("alpha beta");
    app.test_key(VirtualKeyCode::Home, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Home, ElementState::Released);
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::ArrowRight, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowRight, ElementState::Released);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .unwrap()
            .join_address_caret(),
        6
    );

    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::ArrowRight, ElementState::Pressed);
    app.test_key(VirtualKeyCode::ArrowRight, ElementState::Released);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Released);
    app.test_modifiers(ModifiersState::empty());
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().join_address(),
        "alpha eta"
    );

    app.startup_network_dialog
        .as_mut()
        .test_value()
        .set_join_address("alpha beta");
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(&fonts);
    let layout = clonk_frontend::startup_netdlg::net_dlg_layout(800, 600, &metrics);
    let beta_x = layout.join_edit.x
        + 4
        + fonts.text.measure("alpha ", false).0
        + fonts.text.measure("b", false).0 / 2;
    let beta = PhysicalPosition::new(
        f64::from(beta_x),
        f64::from(layout.join_edit.y + layout.join_edit.h / 2),
    );
    app.test_cursor(beta);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    app.test_left_button(ElementState::Pressed);
    let dialog = app.startup_network_dialog.as_ref().test_value();
    let selection = dialog.join_address_selection().test_value();
    assert_eq!(&dialog.join_address()[selection.0..selection.1], "beta");
    assert!(app.netdlg_last_click.is_none());
    assert!(app.netdlg_join_edit_last_click.is_none());
    app.test_left_button(ElementState::Released);

    app.test_right_button(ElementState::Pressed);
    let popup = app.context_menu.test_ref();
    assert_eq!(
        popup.layout().panels[0].rows.len(),
        4 + usize::from(clipboard_text_available())
    );
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .unwrap()
            .join_address_selection(),
        Some(selection),
        "right click preserves the Edit selection"
    );
    app.test_right_button(ElementState::Released);
    app.close_context_menu_silently();

    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    assert!(app.context_menu.is_some());
    assert!(!app
        .netdlg_edit_consumed_keys
        .contains(&VirtualKeyCode::ContextMenu));
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    app.close_context_menu_silently();

    app.netdlg_join_edit_last_click = Some(Instant::now() - Duration::from_millis(450));
    app.test_left_button(ElementState::Pressed);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .unwrap()
            .join_address_selection(),
        None
    );
    app.test_left_button(ElementState::Released);

    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Released);

    let list = PhysicalPosition::new(
        f64::from(layout.game_list.x + layout.game_list.w / 2),
        f64::from(layout.game_list.y + layout.game_list.h / 2),
    );
    app.test_cursor(list);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .unwrap()
            .focused_control(),
        clonk_frontend::startup_netdlg::NetDlgControl::GameList
    );
    for (modifiers, key) in [
        (ModifiersState::CONTROL, VirtualKeyCode::ArrowLeft),
        (ModifiersState::SHIFT, VirtualKeyCode::ArrowLeft),
        (ModifiersState::ALT, VirtualKeyCode::ArrowLeft),
        (ModifiersState::CONTROL, VirtualKeyCode::Backspace),
    ] {
        app.test_modifiers(modifiers);
        app.test_key(key, ElementState::Pressed);
        app.test_key(key, ElementState::Released);
        assert_eq!(app.startup_view, StartupView::NetworkGame);
    }
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    assert_eq!(app.startup_view, StartupView::MainMenu);
}

#[test]
fn network_direct_query_ids_preserve_selection_across_out_of_order_results() {
    let mut app = new_classic_menu_app(800, 600);
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    dialog.resize(800, 600);
    app.startup_view = StartupView::NetworkGame;
    app.startup_network_dialog = Some(dialog);
    app.startup_direct_reference_queries = vec![
        StartupDirectReferenceQuery {
            id: 10,
            address: "first.invalid".to_string(),
            state: StartupDirectReferenceQueryState::Pending,
            expires_at: None,
        },
        StartupDirectReferenceQuery {
            id: 20,
            address: "second.invalid".to_string(),
            state: StartupDirectReferenceQueryState::Pending,
            expires_at: None,
        },
    ];
    app.sync_startup_network_game_rows();
    app.focus_startup_direct_reference_query(10);

    let second = clonk_network::NetworkGameReference {
        title: "Second".to_string(),
        game: "LegacyClonk".to_string(),
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        ..Default::default()
    };
    app.finish_startup_direct_reference_query(20, vec![second.clone()], Some(0));
    assert_eq!(
        app.startup_direct_reference_queries
            .iter()
            .map(|query| query.id)
            .collect::<Vec<_>>(),
        [10]
    );
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().selected_game(),
        Some(1),
        "the still-pending first query must stay selected after the second resolves"
    );

    let first = clonk_network::NetworkGameReference {
        title: "First".to_string(),
        game: "LegacyClonk".to_string(),
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        ..Default::default()
    };
    app.finish_startup_direct_reference_query(10, vec![second, first.clone()], Some(1));
    assert!(app.startup_direct_reference_queries.is_empty());
    assert_eq!(
        app.startup_network_dialog.as_ref().unwrap().selected_game(),
        Some(1)
    );
    match app.startup_network_join_target(1) {
        Some(StartupNetworkJoinTarget::Reference(reference)) => {
            assert_eq!(reference, first)
        }
        _ => panic!("resolved first query must select its own returned reference"),
    }
}

#[test]
fn network_version_mismatch_defers_to_runtime_join_policy() {
    // C++ host admission compares only PID_Conn's build to C4XVERBUILD,
    // not the four-part display version (oracle-src-pinned
    // src/C4Network2.cpp:1291-1299).
    let mut app = new_classic_menu_app(800, 600);
    let reference = clonk_network::NetworkGameReference {
        title: "Newer build".to_string(),
        game: "LegacyClonk".to_string(),
        version: [4, 9, 12, 0],
        build: clonk_network::CURRENT_GAME_BUILD + 1,
        join_allowed: false,
        ..Default::default()
    };
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    dialog.resize(800, 600);
    dialog.set_games(vec![GameApp::startup_network_reference_row(&reference)]);
    assert!(dialog.handle_key_down(KeyCode::Down).is_empty());
    let actions = dialog.handle_key_down(KeyCode::Enter);
    app.startup_view = StartupView::NetworkGame;
    app.startup_network_dialog = Some(dialog);
    app.startup_game_references = vec![reference];

    app.process_network_dialog_actions(actions).test_value();

    assert_eq!(app.message_dialogs.len(), 1);
    let modal = &app.message_dialogs[0].state;
    assert_eq!(modal.caption(), "Cannot join game");
    assert_eq!(
        modal.message(),
        "The game has started already and runtime join is not allowed! Try joining anyway?"
    );
    assert_eq!(
        modal.buttons(),
        clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
    );
    assert_eq!(
        modal.icon(),
        clonk_frontend::message_dialog::MessageDialogIcon::ERROR
    );
    assert!(app.pending_network_join.is_none());
    assert!(app.startup_network_connection.is_none());
    assert!(app.network.is_none());
}

#[test]
fn network_no_runtime_join_requires_yes_and_retains_the_exact_reference() {
    let mut app = new_classic_menu_app(800, 600);
    let global_tcp = clonk_network::NetworkAddress::new(
        clonk_network::NetworkProtocol::Tcp,
        "8.8.8.8:11112".parse().test_value(),
    );
    let private_udp = clonk_network::NetworkAddress::new(
        clonk_network::NetworkProtocol::Udp,
        "10.0.0.7:11113".parse().test_value(),
    );
    let reference = clonk_network::NetworkGameReference {
        title: "Running game".to_string(),
        game: "LegacyClonk".to_string(),
        version: clonk_network::CURRENT_GAME_VERSION,
        build: clonk_network::CURRENT_GAME_BUILD,
        join_allowed: false,
        password_needed: true,
        addresses: vec![private_udp, global_tcp],
        source_address: "127.0.0.1:11111".parse().test_value(),
        netpuncher_ipv4: 0x1122_3344,
        netpuncher_ipv6: 0x5566_7788,
        netpuncher_address: "puncher.invalid:11115".to_string(),
        ..Default::default()
    };
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    dialog.resize(800, 600);
    dialog.set_games(vec![GameApp::startup_network_reference_row(&reference)]);
    assert!(dialog.handle_key_down(KeyCode::Down).is_empty());
    let actions = dialog.handle_key_down(KeyCode::Enter);
    app.startup_view = StartupView::NetworkGame;
    app.startup_network_dialog = Some(dialog);
    app.startup_game_references = vec![reference];

    app.process_network_dialog_actions(actions.clone())
        .test_value();
    let modal = &app.message_dialogs[0].state;
    assert_eq!(modal.caption(), "Cannot join game");
    assert_eq!(
        modal.message(),
        "The game has started already and runtime join is not allowed! Try joining anyway?"
    );
    assert_eq!(
        modal.buttons(),
        clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
    );
    assert_eq!(
        modal.focused_button(),
        Some(clonk_frontend::message_dialog::MessageDialogButton::Yes)
    );
    assert!(app.pending_network_join.is_none());
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();
    assert!(app.pending_network_join.is_none());
    assert!(app.startup_network_connection.is_none());

    app.process_network_dialog_actions(actions).test_value();
    app.startup_game_references.clear();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();

    let settings = app.pending_network_join.test_ref();
    assert_eq!(settings.server_addresses, [global_tcp, private_udp]);
    assert_eq!(
        settings.netpuncher_address.as_deref(),
        Some("puncher.invalid:11115")
    );
    assert_eq!(settings.netpuncher_game_ids.ipv4, 0x1122_3344);
    assert_eq!(settings.netpuncher_game_ids.ipv6, 0x5566_7788);
    assert!(app.game_option_input_dialog.is_some());
    assert!(app.startup_network_connection.is_none());
}

#[test]
fn network_row_double_click_joins_another_cpp_build() {
    // The reference provides the exact build required by C++ admission
    // (oracle-src-pinned src/C4Network2Reference.cpp:79,100-102;
    // src/C4Network2.cpp:1291-1299).
    let mut app = new_classic_menu_app(640, 480);
    let reference = clonk_network::NetworkGameReference {
        title: "Wrong version".to_string(),
        game: "LegacyClonk".to_string(),
        version: [4, 9, 12, 0],
        build: clonk_network::CURRENT_GAME_BUILD + 1,
        password_needed: true,
        ..Default::default()
    };
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let layout = clonk_frontend::startup_netdlg::net_dlg_layout(640, 480, &metrics);
    let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    dialog.resize(640, 480);
    dialog.set_games(vec![GameApp::startup_network_reference_row(&reference)]);
    app.startup_view = StartupView::NetworkGame;
    app.startup_network_dialog = Some(dialog);
    app.startup_game_references = vec![reference];
    app.startup_game_search = None;
    let point = PhysicalPosition::new(
        f64::from(layout.list_entry.x + layout.list_entry.w / 2),
        f64::from(layout.list_entry.y + layout.list_entry.h / 2),
    );
    app.test_cursor(point);

    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    assert!(app.message_dialogs.is_empty());

    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    assert!(app.message_dialogs.is_empty());
    assert!(app.netdlg_last_click.is_none());
    assert_eq!(
        app.pending_network_join
            .as_ref()
            .expect("double-click prepares the reference join")
            .compatibility_build,
        clonk_network::CURRENT_GAME_BUILD + 1
    );
    assert!(app.game_option_input_dialog.is_some());
    assert!(app.startup_network_connection.is_none());
}

#[test]
fn client_join_flow_uses_cpp_reference_build_regardless_of_rust_version() {
    // C4GameVersion defaults the reference build to C4XVERBUILD, which the
    // host then requires in PID_Conn (oracle-src-pinned
    // src/C4GameVersion.h:35-37; src/C4Network2.cpp:1291-1299).
    let mut app = new_classic_menu_app(800, 600);
    let global_tcp = clonk_network::NetworkAddress::new(
        clonk_network::NetworkProtocol::Tcp,
        "8.8.8.8:11112".parse().test_value(),
    );
    let private_udp = clonk_network::NetworkAddress::new(
        clonk_network::NetworkProtocol::Udp,
        "10.0.0.7:11113".parse().test_value(),
    );
    let reference = clonk_network::NetworkGameReference {
        title: "Passworded game".to_string(),
        password_needed: true,
        addresses: vec![private_udp, global_tcp],
        source_address: "127.0.0.1:11111".parse().test_value(),
        netpuncher_ipv4: 0x1122_3344,
        netpuncher_ipv6: 0x5566_7788,
        netpuncher_address: "puncher.invalid:11115".to_string(),
        version: [4, 9, 12, 0],
        build: clonk_network::CURRENT_GAME_BUILD + 2,
        ..Default::default()
    };
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let mut network_dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    network_dialog.resize(800, 600);
    network_dialog.set_games(vec![clonk_frontend::startup_netdlg::NetDlgGameEntry {
        title: reference.title.clone(),
        details: String::new(),
        extra_lines: Vec::new(),
        status_icons: Vec::new(),
        row_icon: clonk_frontend::startup_netdlg::NetDlgRowIcon::None,
        address: None,
        joinable: true,
    }]);
    assert!(network_dialog.handle_key_down(KeyCode::Down).is_empty());
    let actions = network_dialog.handle_key_down(KeyCode::Enter);
    assert_eq!(
        actions,
        [clonk_frontend::startup_netdlg::NetDlgAction::JoinGame { address: None }]
    );
    app.startup_network_dialog = Some(network_dialog);
    app.startup_game_references = vec![reference];
    app.active_scenario = Some(FrontendScenario::fallback());
    app.active_definition_load = Some(ScenarioDefinitionLoad::Fixed {
        modules: vec!["Stale.c4d".to_string()],
        definition_root: None,
    });

    app.process_network_dialog_actions(actions).test_value();

    assert!(app.message_dialogs.is_empty());
    assert!(app.active_scenario.is_none());
    match app.active_definition_load.as_ref() {
        Some(ScenarioDefinitionLoad::Seed { modules, .. }) => {
            assert_eq!(modules, &["Objects.c4d".to_string()]);
        }
        other => panic!("reference join must install the Objects seed, got {other:?}"),
    }
    let settings = app.pending_network_join.test_ref();
    assert_eq!(
        settings.compatibility_build,
        clonk_network::CURRENT_GAME_BUILD + 2
    );
    assert_eq!(settings.server_addresses, [global_tcp, private_udp]);
    assert_eq!(
        settings.logical_server_addresses,
        [global_tcp, private_udp],
        "reference joins retain the C++ progress routes separately"
    );
    assert_eq!(
        settings.netpuncher_address.as_deref(),
        Some("puncher.invalid:11115")
    );
    assert_eq!(settings.netpuncher_game_ids.ipv4, 0x1122_3344);
    assert_eq!(settings.netpuncher_game_ids.ipv6, 0x5566_7788);
    assert!(settings.password.is_empty());
    let dialog = app.game_option_input_dialog.test_ref();
    assert_eq!(
        dialog.purpose,
        PendingInputDialogPurpose::NetworkJoinPassword
    );
    assert_eq!(dialog.controller.message(), "Enter password:");
    assert_eq!(dialog.controller.caption(), "Enter password:");
    assert_eq!(dialog.controller.icon(), InputDialogIcon::LOCKED);
    assert!(dialog.controller.text().is_empty());

    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
        .test_value();
    assert!(app.pending_network_join.is_none());
    assert!(app.game_option_input_dialog.is_none());
}

#[test]
fn abandoned_join_password_prompt_keeps_the_netdlg_search_running() {
    // C4StartupNetDlg owns DiscoverClient for the dialog's whole lifetime: it
    // is constructed with the dialog and closed only in the destructor, and a
    // join password dialog merely stops OnSec1Timer while it is up
    // (oracle-src-pinned src/C4StartupNetDlg.cpp:737-738,752,1116-1120).
    let mut app = new_classic_menu_app(800, 600);
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let mut network_dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    network_dialog.resize(800, 600);
    app.startup_view = StartupView::NetworkGame;
    app.startup_network_dialog = Some(network_dialog);
    app.startup_game_search = Some(
        clonk_network::StartupGameSearch::start(clonk_network::NetworkGameSearchConfig {
            internet_enabled: false,
            use_alternate_server: false,
            master_server_url: String::new(),
            discovery_port: 0,
        })
        .test_value(),
    );
    let reference = clonk_network::NetworkGameReference {
        title: "Passworded game".to_string(),
        password_needed: true,
        source_address: "127.0.0.1:11111".parse().test_value(),
        ..Default::default()
    };

    app.activate_network_reference_join(reference.clone())
        .test_value();
    assert!(app.game_option_input_dialog.is_some());

    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
        .test_value();

    assert!(app.pending_network_join.is_none());
    assert!(
        app.startup_game_search.is_some(),
        "a cancelled password prompt must leave the dialog's search running"
    );
    app.request_startup_network_refresh().test_value();
    assert!(
        app.message_dialogs.is_empty(),
        "refresh must not report a search that was never stopped"
    );

    // An empty password aborts the join exactly like Cancel does.
    app.activate_network_reference_join(reference).test_value();
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(String::new())])
        .test_value();

    assert!(app.pending_network_join.is_none());
    assert!(app.startup_network_connection.is_none());
    assert!(
        app.startup_game_search.is_some(),
        "an empty password must leave the dialog's search running"
    );
}

#[test]
fn cancelling_a_launched_join_restores_the_netdlg_search() {
    use clonk_frontend::startup_netdlg::NetDlgRowIcon;

    // Starting a join ends the C++ startup loop, so `DoStartup` destroys the
    // dialog together with its DiscoverClient and pMasterserverClient; aborting
    // the attempt re-enters `DoStartup`, which builds a fresh C4StartupNetDlg
    // whose OnShown -> UpdateMasterserver starts querying again (pinned oracle
    // 7d43b47b src/C4StartupNetDlg.cpp:737-738,752,771-777,851-866;
    // src/C4Startup.cpp:196-198). The port retains the dialog across that
    // round trip, so the search it owns has to come back with it.
    let mut app = new_classic_menu_app(800, 600);
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let mut network_dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig {
            masterserver_signup: false,
            record: false,
        },
        metrics,
    );
    network_dialog.resize(800, 600);
    app.startup_view = StartupView::NetworkGame;
    app.startup_network_dialog = Some(network_dialog);
    app.startup_game_search = Some(
        clonk_network::StartupGameSearch::start(clonk_network::NetworkGameSearchConfig {
            internet_enabled: false,
            use_alternate_server: false,
            master_server_url: String::new(),
            discovery_port: 0,
        })
        .test_value(),
    );

    app.activate_network_reference_join(clonk_network::NetworkGameReference {
        title: "Unreachable game".to_string(),
        source_address: "127.0.0.1:1".parse().test_value(),
        ..Default::default()
    })
    .test_value();
    assert!(
        app.startup_game_search.is_none(),
        "a join that actually launches stops the search"
    );
    assert!(matches!(
        app.message_dialogs
            .last()
            .map(|dialog| &dialog.continuation),
        Some(MessageDialogContinuation::StartupNetworkConnectProgress)
    ));

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();

    assert!(app.startup_network_connection.is_none());
    assert!(app.pending_network_join.is_none());
    assert!(
        app.startup_game_search.is_some(),
        "the netdlg must never be left on screen without its search"
    );
    app.request_startup_network_refresh().test_value();
    assert!(
        app.message_dialogs.is_empty(),
        "refresh must not report a search that was restored"
    );
    assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .expect("network dialog")
            .masterserver_entry()
            .row_icon,
        NetDlgRowIcon::Query
    );
}

#[test]
fn client_join_flow_wrong_password_reprompts_without_rebuilding_attempts() {
    let mut app = new_classic_menu_app(800, 600);
    let attempts = vec![
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Tcp,
            "127.0.0.1:30111".parse().unwrap(),
        ),
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Udp,
            "127.0.0.1:30112".parse().unwrap(),
        ),
    ];
    app.pending_network_join = Some(
        ClientSettings::new(attempts[0].endpoint, "Player")
            .with_join_attempts(attempts.clone())
            .with_password(
                clonk_engine::LegacyCString::from_bytes(b"rejected".to_vec()).test_value(),
            ),
    );
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Err(NetworkStartError::WrongPassword {
            message: clonk_engine::LegacyCString::from_bytes(b"wrong password".to_vec())
                .test_value(),
        }))
        .test_value();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));

    app.poll_startup_network_connection().test_value();

    assert!(app.startup_network_connection.is_none());
    assert_eq!(
        app.pending_network_join
            .as_ref()
            .expect("same join remains pending")
            .server_addresses,
        attempts
    );
    let prompt = app.game_option_input_dialog.test_ref();
    assert_eq!(
        prompt.purpose,
        PendingInputDialogPurpose::NetworkJoinPassword
    );
    assert!(prompt.controller.text().is_empty());

    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(String::new())])
        .test_value();
    assert!(app.pending_network_join.is_none());
    assert!(app.startup_network_connection.is_none());

    app.pending_network_join =
        Some(ClientSettings::new(attempts[0].endpoint, "Player").with_join_attempts(attempts));
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Err(NetworkStartError::Other("join denied".to_string())))
        .test_value();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));
    app.poll_startup_network_connection().test_value();
    assert!(app.pending_network_join.is_none());
    assert!(app.game_option_input_dialog.is_none());
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert_startup_error_log(&app, "Unable to start network session: join denied");
}

#[test]
fn client_join_flow_submitted_password_is_frozen_for_the_worker() {
    let mut app = new_classic_menu_app(800, 600);
    let closed = std::net::TcpListener::bind("127.0.0.1:0").test_value();
    let address = closed.local_addr().test_value();
    drop(closed);
    app.pending_network_join = Some(ClientSettings::new(address, "Player").with_join_attempts([
        clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Tcp, address),
    ]));
    app.open_network_join_password_dialog().test_value();

    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "typed secret".to_string(),
    )])
    .test_value();

    assert_eq!(
        app.pending_network_join
            .as_ref()
            .expect("settings stay frozen while the worker starts")
            .password
            .as_bytes(),
        b"typed secret"
    );
    assert!(app.startup_network_connection.is_some());
    for _ in 0..100 {
        app.poll_startup_network_connection().test_value();
        if app.startup_network_connection.is_none() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(app.startup_network_connection.is_none());
    assert!(app.pending_network_join.is_none());
}

#[test]
fn gamepad_enabled_uses_native_false_and_defaults_true() {
    assert!(configured_gamepads_enabled(b""));
    assert!(configured_gamepads_enabled(
        b"[General]\nGamepadEnabled=invalid\n"
    ));
    assert!(!configured_gamepads_enabled(
        b"[General]\nGamepadEnabled=false\n"
    ));
    assert!(!configured_gamepads_enabled(
        b"[General]\nGamepadEnabled=0\n"
    ));
}

#[test]
fn startup_group_maker_snapshot_preserves_configured_native_bytes() {
    assert_eq!(configured_process_group_maker(b"").as_bytes(), b"");
    assert_eq!(
        configured_process_group_maker(b"[General]\nName=\"M\\201ker\"\nName=Ignored\n").as_bytes(),
        b"M\x81ker"
    );
}

#[test]
fn dirty_axis_calibration_updates_cpp_keys_without_rewriting_other_bytes() {
    let source = b"[Vendor]\nOpaque=\x80\xff\n[Gamepad2]\nVendorKey=keep\nAxis4Min=7\n";
    assert_eq!(
        update_dirty_gamepad_axis_calibration_config(source, &GamepadBindings::default())
            .expect("clean calibration leaves config untouched"),
        source
    );

    let mut bindings = GamepadBindings::default();
    let mut calibrations = bindings.axis_calibrations();
    calibrations[2][4] = input::GamepadAxisCalibration::new(12, u32::MAX, true);
    bindings.replace_axis_calibrations(calibrations);
    let updated = update_dirty_gamepad_axis_calibration_config(source, &bindings).test_value();

    assert!(updated.windows(2).any(|bytes| bytes == b"\x80\xff"));
    assert_eq!(
        clonk_app_netplay::configured_native_value(&updated, "Gamepad2", "VendorKey")
            .expect("unrelated gamepad value survives")
            .as_bytes(),
        b"keep"
    );
    assert_eq!(
        clonk_app_netplay::configured_native_value(&updated, "Gamepad2", "Axis4Min")
            .expect("minimum persisted")
            .as_bytes(),
        b"12"
    );
    assert_eq!(
        clonk_app_netplay::configured_native_value(&updated, "Gamepad2", "Axis4Max")
            .expect("full u32 maximum persisted")
            .as_bytes(),
        b"4294967295"
    );
    assert_eq!(
        clonk_app_netplay::configured_native_boolean(&updated, "Gamepad2", "Axis4Calibrated"),
        Some(true)
    );
}

#[test]
fn startup_dialog_fade_preserves_ordered_native_text_at_scaled_output() {
    let scale = 1.5;
    let mut app = new_real_classic_menu_app(320, 200);
    app.graphics.set_runtime_sprite_filtering(scale, false);
    app.configure_native_startup_fonts(scale, false);
    let _ = render_ordered_test_frame(&mut app, scale, 480, 300);

    app.handle_main_menu_activation(MainMenuItem::About)
        .test_value();
    let mut frame_ten = None;
    for step in 1..=STARTUP_DIALOG_FADE_STEPS {
        let (_, output, plan) = render_ordered_test_frame(&mut app, scale, 480, 300);
        if step == 1 {
            assert_eq!(plan.batches.len(), 2, "outgoing then incoming layers");
            assert!(plan
                .batches
                .iter()
                .all(|batch| batch.logical_layer.is_some()));
            assert!(plan.batches.iter().all(|batch| !batch.text.is_empty()));
            assert!(
                plan.batches[0].fonts.is_some(),
                "outgoing retains its fonts"
            );
            assert!(plan.batches[0]
                .text
                .iter()
                .all(|command| command.color[3] <= 230));
            assert!(plan.batches[1]
                .text
                .iter()
                .all(|command| command.color[3] <= 26));
        }
        if step == STARTUP_DIALOG_FADE_STEPS {
            frame_ten = Some(output);
        }
    }
    assert!(app.startup_dialog_fade.is_none());
    let (_, settled, _) = render_ordered_test_frame(&mut app, scale, 480, 300);
    assert_eq!(
        frame_ten.expect("scaled frame ten"),
        settled,
        "scaled frame ten must already use the settled native-text path"
    );
}

#[test]
fn synchronized_player_file_policy_matches_native_suppression_matrix() {
    use clonk_engine::PlayerStatus;

    let policy = |status, script_player, at_client, league, max_players| {
        synchronized_player_file_policy(status, script_player, at_client, 7, league, max_players)
    };
    for status in [
        PlayerStatus::Inactive,
        PlayerStatus::Active,
        PlayerStatus::TeamSelection,
        PlayerStatus::TeamSelectionPending,
    ] {
        assert_eq!(
            policy(status, false, 7, true, Some(0)),
            SynchronizedPlayerFilePolicy::Persist {
                local_control: true
            },
            "local users persist regardless of remote-only gates"
        );
    }
    for status in [PlayerStatus::Eliminated, PlayerStatus::Surrendered] {
        assert_eq!(
            policy(status, false, 7, false, Some(4)),
            SynchronizedPlayerFilePolicy::Skip
        );
    }
    assert_eq!(
        policy(PlayerStatus::Active, true, 7, false, Some(4)),
        SynchronizedPlayerFilePolicy::Skip
    );
    for (league, max_players) in [(true, Some(4)), (false, Some(0)), (false, Some(-1))] {
        assert_eq!(
            policy(PlayerStatus::Active, false, 8, league, max_players),
            SynchronizedPlayerFilePolicy::BlockedRemote
        );
    }
    for max_players in [None, Some(1)] {
        assert_eq!(
            policy(PlayerStatus::Active, false, 8, false, max_players),
            SynchronizedPlayerFilePolicy::Persist {
                local_control: false
            }
        );
    }
}

#[test]
fn synchronized_player_file_remote_gates_leave_profile_untouched() {
    let directory = tempdir();
    let profile_path = directory.path().join("Remote.c4p");
    let sentinel = b"profile must not be opened";
    fs::write(&profile_path, sentinel).test_value();

    let mut app = new_state_only_synthetic_crew_running_sandbox_app();
    let player_number = app.local_owner;
    let info_id = 602;
    let mut state = app.engine.capture_state();
    let player = state
        .players
        .iter_mut()
        .find(|player| player.id == player_number)
        .test_value();
    player.player_info_id = info_id;
    player.at_client = clonk_engine::PlayerAtClient::new(8);
    player.status = clonk_engine::PlayerStatus::Active;
    player.script_player = false;
    app.engine.restore_state(&state).test_value();
    app.control_player_infos.replace_snapshot(
        info_id,
        [clonk_engine::PlayerInfoControlData {
            client_id: 8,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: info_id,
                filename: LegacyCString::from_bytes(b"Remote.c4p".to_vec())
                    .expect("player filename"),
                game_number: player_number,
                ..clonk_engine::ControlPlayerInfoEntry::default()
            }],
            by_client: 0,
            ..clonk_engine::PlayerInfoControlData::default()
        }],
    );
    app.local_player_profile_paths
        .insert(info_id, profile_path.clone());
    assert_eq!(
        app.offline_local_client_id(),
        0,
        "the fixture must classify client 8 as remote"
    );

    app.engine.set_max_players(4);
    app.network_is_league = true;
    assert!(!app.persist_synchronized_local_player_files());
    assert_eq!(fs::read(&profile_path).unwrap(), sentinel);

    app.network_is_league = false;
    app.engine.set_max_players(0);
    assert!(!app.persist_synchronized_local_player_files());
    assert_eq!(fs::read(&profile_path).unwrap(), sentinel);
}

#[test]
fn developer_console_latches_no_input_and_reflects_pending_network_pause() {
    let mut app = new_state_only_running_sandbox_app();
    app.console_mode = true;
    app.control_playback =
        Some(ControlRecordPlayback::from_bytes(&[0, clonk_engine::RCT_END]).test_value());
    app.sync_developer_console_view();
    app.control_playback = None;
    app.sync_developer_console_view();
    assert!(!app.developer_console.view_model().editing);

    let (_events, _commands) = install_running_network_stub(&mut app, 0, 0, 1);
    app.network_control_running = true;
    app.runtime_network_status_barrier = Some(RuntimeNetworkStatusBarrier {
        status: clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_PAUSE,
            control_mode: 0,
            target_tick: 0,
        },
        local_reached: false,
        actual_control_tick: None,
    });
    assert!(
        app.developer_console_view_model().halted,
        "IsPaused becomes true as soon as a network status barrier is pending"
    );
    app.runtime_network_status_barrier = None;
    assert!(!app.developer_console_view_model().halted);
}

#[test]
fn developer_console_player_and_net_menus_use_native_controls() {
    let mut app = new_state_only_running_sandbox_app();
    let player = app.local_owner;
    app.engine
        .test_player_mut(player)
        .set_at_client_name("Host");
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
    let mut remote = message_client(7, b"Remote");
    remote.activated = false;
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), remote]);

    assert_eq!(
        app.developer_console_player_menu_entries(true),
        vec![ConsolePlayerRow {
            number: player,
            quit_label: format!(
                "Remove {} (Host) ",
                c4_presentation_text(app.engine.player(player).expect("player remains").name())
            ),
            quit_enabled: true,
            viewport_label: format!(
                "New for {}",
                c4_presentation_text(app.engine.player(player).expect("player remains").name())
            ),
        }]
    );
    assert_eq!(
        app.developer_console_net_menu_entries(),
        vec![
            ConsoleClientRow {
                id: 0,
                menu_label: "Host Host (0)".to_string(),
                menu_enabled: true,
            },
            ConsoleClientRow {
                id: 7,
                menu_label: "Client Remote (7) deactivated".to_string(),
                menu_enabled: true,
            },
        ]
    );

    app.engine.set_control_host(false);
    assert!(
        app.developer_console_net_menu_entries()
            .iter()
            .all(|entry| entry.menu_enabled),
        "native keeps the rows enabled and rejects their callback"
    );
    assert!(!app
        .developer_console_kick_client(7)
        .expect("non-control host ignores kick"));
    assert!(commands.take_submitted_client_removes().is_empty());
    app.engine.set_control_host(true);
    assert!(app
        .developer_console_kick_client(7)
        .expect("control host queues kick"));
    assert_eq!(
        commands.take_submitted_client_removes(),
        vec![clonk_engine::ClientRemoveControlData {
            client_id: 7,
            reason: clonk_engine::LegacyCString::from_bytes(b"kicked from host menu".to_vec())
                .expect("fixture reason"),
            by_client: 0,
        }]
    );

    let tick = app.local_control_submission_tick();
    assert!(app
        .developer_console_quit_player(player, true)
        .expect("queue console player quit"));
    assert_eq!(
        commands.take_submitted_internal_player_scripts(),
        vec![(
            tick,
            clonk_engine::ControlPacket::EliminatePlayer(
                clonk_engine::EliminatePlayerControlData {
                    player,
                    by_client: 0,
                },
            ),
        )]
    );
    assert!(app.engine.player(player).is_some());
}

#[test]
fn developer_console_offline_join_and_quit_use_control_cadence() {
    let player_path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    let mut app = new_synthetic_running_sandbox_app();
    let before = app.engine.snapshot().players.len();

    assert_eq!(
        app.developer_console_join_players(&[player_path.to_path_buf()], true)
            .expect("join selected offline player"),
        1
    );
    assert_eq!(app.engine.snapshot().players.len(), before + 1);

    let player = app.local_owner;
    app.engine.set_control_rate(3);
    app.snapshot = app.engine.test_tick();
    assert!(app
        .developer_console_quit_player(player, true)
        .expect("queue local CID_EliminatePlayer"));
    assert_ne!(
        app.engine.player(player).map(clonk_engine::Player::status),
        Some(clonk_engine::PlayerStatus::Eliminated),
        "Game.Control.Input.Add must not execute the callback immediately"
    );
    app.test_update();
    app.test_update();
    assert_ne!(
        app.engine.player(player).map(clonk_engine::Player::status),
        Some(clonk_engine::PlayerStatus::Eliminated)
    );
    app.test_update();
    assert_eq!(
        app.engine.player(player).map(clonk_engine::Player::status),
        Some(clonk_engine::PlayerStatus::Eliminated)
    );
}

#[test]
fn developer_console_network_join_uses_join_local_player_route() {
    let player_path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _events, commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let mut settings = ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client");
    settings.group_maker = LegacyCString::from_bytes(b"Console maker".to_vec()).test_value();
    app.network_mode = Some(NetworkMode::Client(settings));
    app.control_clients.register(7, true, false);
    let wire_name = LegacyCString::from_bytes(path_to_legacy_bytes(player_path)).test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 7 << 16,
        loadable: true,
        filename: wire_name.clone(),
        ..Default::default()
    };
    let observer = thread::spawn(move || commands.complete_initial_client_join(vec![resource]));

    assert_eq!(
        app.developer_console_join_players(&[player_path.to_path_buf()], true)
            .expect("console network player joins"),
        1
    );
    drop(app.network.take());
    let (order, publications, updates, acknowledgements) = observer.test_join();
    assert_eq!(order, vec!["publish", "player-info"]);
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].source_path.as_path(), player_path);
    assert_eq!(publications[0].wire_name, wire_name);
    assert_eq!(publications[0].group_maker.as_bytes(), b"Console maker");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].client_id, 7);
    assert_eq!(
        updates[0].flags,
        clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
    );
    assert!(acknowledgements.is_empty());
}

#[test]
fn developer_console_unpacked_c4f_child_has_native_mother_group() {
    let directory = tempdir();
    let outer = directory.path().join("Outer.c4f");
    let inner_folder = outer.join("Inner.c4f");
    let scenario = inner_folder.join("Game.c4s");
    fs::create_dir_all(&scenario).test_value();

    let (physical, children) = scenario_logical_storage(&scenario).test_value();
    assert_eq!(physical, outer);
    assert_eq!(children, vec!["Inner.c4f", "Game.c4s"]);

    let standalone = directory.path().join("Standalone.c4s");
    fs::create_dir(&standalone).test_value();
    assert_eq!(
        scenario_logical_storage(&standalone).unwrap(),
        (standalone, Vec::new())
    );
}

#[test]
fn folder_live_save_replays_native_prefix_and_writes_packed_children() {
    let directory = tempdir();
    let destination = directory.path().join("Live.c4s");
    fs::create_dir(&destination).test_value();
    fs::write(destination.join("Remove.txt"), b"remove me").test_value();
    fs::write(destination.join("Untouched.dat"), b"keep me").test_value();
    fs::create_dir(destination.join("Blocked.txt")).test_value();

    let mut child = MutableGroup::new("SectNight.c4g");
    child
        .add_file("Scenario.txt", b"[Head]\nTitle=Night\n".to_vec())
        .test_value();
    let child = child.pack_raw().test_value();
    let mut journal = developer_console_save::FolderSaveJournal::default();
    journal.put_file(
        "Prefix.txt",
        b"committed prefix",
        developer_console_save::FolderSaveAddFailure::Fatal,
    );
    journal.delete_entry("Remove.txt");
    journal.put_child(
        "SectNight.c4g",
        child,
        developer_console_save::FolderSaveAddFailure::Fatal,
    );
    journal.put_file(
        "Blocked.txt",
        b"cannot replace a directory by truncating it",
        developer_console_save::FolderSaveAddFailure::Fatal,
    );
    journal.put_file(
        "After.txt",
        b"must not run",
        developer_console_save::FolderSaveAddFailure::Fatal,
    );

    replay_folder_save_journal(&journal, &destination, b"Folder maker")
        .expect_err("directory target makes the fourth mutation fail");

    assert_eq!(
        fs::read(destination.join("Prefix.txt")).unwrap(),
        b"committed prefix"
    );
    assert!(!destination.join("Remove.txt").exists());
    assert_eq!(
        fs::read(destination.join("Untouched.dat")).unwrap(),
        b"keep me"
    );
    assert!(!destination.join("After.txt").exists());
    let child_path = destination.join("SectNight.c4g");
    assert!(child_path.is_file(), "folder children stay packed files");
    let child = Group::open(&child_path).test_value();
    assert_eq!(child.maker(), Some("Folder maker"));
    assert_eq!(
        child.read_file("Scenario.txt").unwrap(),
        b"[Head]\nTitle=Night\n"
    );
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("lc-rewrite")));
}

#[test]
fn network_set_pre_send_applies_matching_local_target_fps_and_flash_without_fatal_boundary() {
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 7, 0, 2);
    app.control_clients.replace_snapshot([
        message_client(0, b"Host"),
        message_client(7, b"Client Alice"),
    ]);
    app.engine.set_network_game(true);
    app.engine.set_network_control_mode(true);
    app.engine
        .load_scenario_script_with_convention(
            "SetPreSend app fixture",
            concat!(
                "#strict\n",
                "func Mismatch() { return SetPreSend(55, \"Host*\"); }\n",
                "func Match() { return SetPreSend(76, \"client a?i*\"); }\n",
                "func Reset() { return SetPreSend(0, \"\"); }\n",
            ),
            true,
        )
        .test_value();
    app.set_runtime_flash_message("unchanged", RuntimeHelpCharset::Windows1252)
        .test_value();
    let unchanged_flash = app.runtime_flash_message.clone();

    app.engine
        .call_scenario_script_function("Mismatch", Vec::new())
        .test_value();
    app.apply_engine_network_target_fps_requests().test_value();
    assert_eq!(
        app.network_control_clock
            .expect("network clock")
            .target_fps(),
        38
    );
    assert_eq!(app.runtime_flash_message, unchanged_flash);

    app.engine
        .call_scenario_script_function("Match", Vec::new())
        .test_value();
    app.apply_engine_network_target_fps_requests().test_value();
    let clock = app.network_control_clock.test_value();
    assert_eq!(clock.target_fps(), 76);
    assert_eq!(
        clock.control_presend(),
        1,
        "setter does not recalculate inline"
    );
    let flash = app.runtime_flash_message.test_ref();
    assert_eq!(flash.text, "TargetFPS: 76");
    assert_eq!(flash.remaining_draws, 26);
    assert_eq!(flash.y, app.runtime_flash_y());

    app.engine
        .call_scenario_script_function("Reset", Vec::new())
        .test_value();
    app.apply_engine_network_target_fps_requests().test_value();
    assert_eq!(
        app.network_control_clock
            .expect("network clock")
            .target_fps(),
        38
    );
    assert_eq!(runtime_flash_text(&app), Some("TargetFPS: 38"));
}

#[test]
fn runtime_client_pause_and_go_drive_to_targets_before_acknowledging() {
    // CheckStatusReached keeps both Pause and Go control running until the
    // cadence boundary at/after the requested tick, stops first, and only
    // then sends PID_StatusAck (src/C4Network2.cpp:2017-2060,2081-2113).
    let mut app = new_running_sandbox_app();
    assert_eq!(
        app.engine.frame(),
        0,
        "fixture starts on a cadence boundary"
    );
    let (events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 2,
    };

    events
        .send(NetworkEvent::StatusRequested(pause))
        .test_value();
    app.test_network_events();
    assert!(
        app.network_control_running,
        "receipt alone must not halt control"
    );
    assert_eq!(app.pending_client_start_status, None);
    assert!(commands.take_framed_status_acknowledgements().is_empty());

    queue_empty_ready_tick(&app, &events);
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (1, 1)
    );
    app.test_update();
    queue_empty_ready_tick(&app, &events);
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (3, 2)
    );
    assert!(
        app.network_control_running,
        "tick target is not a cadence boundary yet"
    );
    app.test_update();
    app.test_update();

    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (4, 2)
    );
    assert!(!app.network_control_running);
    assert_eq!(
        commands.take_framed_status_acknowledgements(),
        vec![(pause, 4)]
    );
    app.test_update();
    app.sec1_timer().test_value();
    assert!(commands.take_framed_status_acknowledgements().is_empty());

    events
        .send(NetworkEvent::StatusCommitted(pause))
        .test_value();
    app.test_network_events();
    assert!(!app.network_control_running);

    let go = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        target_tick: 4,
        ..pause
    };
    events.send(NetworkEvent::StatusRequested(go)).test_value();
    app.test_network_events();
    assert!(
        app.network_control_running,
        "Go drives out of committed Pause"
    );

    queue_empty_ready_tick(&app, &events);
    app.test_update();
    app.test_update();
    queue_empty_ready_tick(&app, &events);
    app.test_update();
    app.test_update();
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (8, 4)
    );
    assert!(
        !app.network_control_running,
        "Go also waits halted for commit"
    );
    assert_eq!(
        commands.take_framed_status_acknowledgements(),
        vec![(go, 8)]
    );

    events.send(NetworkEvent::StatusCommitted(go)).test_value();
    app.test_network_events();
    assert!(app.network_control_running);
    queue_empty_ready_tick(&app, &events);
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (9, 5)
    );
}

#[test]
fn a_status_without_a_runtime_reach_condition_supersedes_the_pending_barrier() {
    // HandleStatus installs every received status and clears fStatusReached,
    // so a lobby status the client can never reach still replaces a pending
    // Pause barrier rather than leaving the client to acknowledge a barrier
    // its network layer has already dropped (src/C4Network2.cpp:1501-1511,
    // 2017-2041; clonk-org/clonk-rs#308).
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 2,
    };
    events
        .send(NetworkEvent::StatusRequested(pause))
        .test_value();
    app.test_network_events();
    assert_eq!(
        app.runtime_network_status_barrier
            .map(|pending| pending.status),
        Some(pause)
    );

    let lobby = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_LOBBY,
        control_mode: 1,
        target_tick: -1,
    };
    events
        .send(NetworkEvent::StatusRequested(lobby))
        .test_value();
    app.test_network_events();
    assert_eq!(app.runtime_network_status_barrier, None);

    for _ in 0..4 {
        assert_eq!(
            app.check_runtime_network_status_reached(),
            RuntimeStatusReachOutcome::NotReached
        );
    }
    assert!(commands.take_runtime_status_commands().is_empty());
}

#[test]
fn runtime_client_adopts_a_recorded_status_its_barrier_never_applied() {
    // A client's reach target and the status it acknowledges are one
    // C4Network2::Status value, so a PID_Status the app did not apply -- one it
    // could not arm, or one lost with the tail of an aborted event batch --
    // still becomes the barrier instead of wedging the game on an
    // acknowledgement the network layer rejects every frame
    // (src/C4Network2.cpp:1501-1511; clonk-org/clonk-rs#308).
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 2,
    };
    events
        .send(NetworkEvent::StatusRequested(pause))
        .test_value();
    app.test_network_events();

    let retargeted = clonk_network::NetworkStatus {
        target_tick: 4,
        ..pause
    };
    app.network
        .as_mut()
        .test_value()
        .test_receive_client_status_request(retargeted);

    assert_eq!(
        app.check_runtime_network_status_reached(),
        RuntimeStatusReachOutcome::NotReached
    );
    assert_eq!(
        app.runtime_network_status_barrier
            .map(|pending| pending.status),
        Some(retargeted)
    );
    assert!(
        app.network_control_running,
        "the adopted target is still ahead"
    );
    assert!(commands.take_framed_status_acknowledgements().is_empty());

    for _ in 0..4 {
        queue_empty_ready_tick(&app, &events);
        app.test_update();
        app.test_update();
    }
    // The reach probe runs before the frame it observes is stepped, so the
    // cadence boundary at the adopted target is tested on the next pass.
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (8, 4)
    );
    assert!(!app.network_control_running);
    assert_eq!(
        commands.take_framed_status_acknowledgements(),
        vec![(retargeted, 8)]
    );
    app.test_update();
    app.sec1_timer().test_value();
    assert!(
        commands.take_framed_status_acknowledgements().is_empty(),
        "the adopted barrier is acknowledged once"
    );
}

#[test]
fn runtime_client_barrier_follows_an_acknowledgement_already_on_the_wire() {
    // OnStatusReached stops control and the client then waits for the host's
    // PID_StatusAck for the exact status it acknowledged, so a barrier that
    // lost track of that status adopts it instead of blocking the commit
    // (src/C4Network2.cpp:2017-2060,1513-1550; clonk-org/clonk-rs#308).
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 0,
    };
    events
        .send(NetworkEvent::StatusRequested(pause))
        .test_value();
    app.test_network_events();
    app.test_update();
    assert_eq!(
        commands.take_framed_status_acknowledgements(),
        vec![(pause, 0)]
    );

    // Only the barrier forgets the acknowledged status; the network layer still
    // awaits its commit.
    app.runtime_network_status_barrier = Some(RuntimeNetworkStatusBarrier {
        status: clonk_network::NetworkStatus {
            target_tick: 6,
            ..pause
        },
        local_reached: false,
        actual_control_tick: None,
    });

    assert_eq!(
        app.check_runtime_network_status_reached(),
        RuntimeStatusReachOutcome::NotReached
    );
    assert_eq!(
        app.runtime_network_status_barrier,
        Some(RuntimeNetworkStatusBarrier {
            status: pause,
            local_reached: true,
            actual_control_tick: Some(0),
        })
    );
    assert!(!app.network_control_running);
    assert!(commands.take_framed_status_acknowledgements().is_empty());

    events
        .send(NetworkEvent::StatusCommitted(pause))
        .test_value();
    app.test_network_events();
    assert_eq!(app.runtime_network_status_barrier, None);
}

#[test]
fn runtime_host_commit_executes_sync_at_the_actual_overshoot_tick() {
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 2,
    };
    events
        .send(NetworkEvent::StatusRequested(pause))
        .test_value();
    for tick in 0..=2 {
        events
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: Vec::new(),
            })
            .test_value();
    }
    let mut status_commands = Vec::new();
    for _ in 0..7 {
        app.test_update();
        status_commands.extend(commands.take_runtime_status_commands());
    }
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (6, 3)
    );
    assert_eq!(
        status_commands,
        vec![network::TestRuntimeStatusCommand::Reached {
            status: pause,
            actual_control_tick: 3,
        }]
    );

    events
        .send(NetworkEvent::ScheduledSync {
            tick: 3,
            controls: vec![NetworkControl::Message(message_control(
                MESSAGE_TYPE_NORMAL,
                -1,
                -1,
                b"overshoot-sync",
                0,
            ))],
        })
        .test_value();
    app.test_network_events();
    assert!(app.network_sync.scheduled.contains_key(&3));
    assert!(app
        .message_board
        .log_history
        .iter()
        .all(|line| !line.contains("overshoot-sync")));
    events
        .send(NetworkEvent::StatusCommitted(pause))
        .test_value();
    app.test_network_events();

    assert!(app.network_sync.scheduled.is_empty());
    assert!(app
        .message_board
        .log_history
        .iter()
        .any(|line| line.contains("overshoot-sync")));
    assert!(!app.network_control_running);
}

#[test]
fn runtime_host_pause_retarget_and_go_report_each_local_reach_once() {
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);

    // Rust's already-incremented clock maps to native getNextControlTick:
    // after tick zero, both identify tick one even off cadence
    // (src/C4GameControl.cpp:325-365).
    queue_empty_ready_tick(&app, &events);
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (1, 1)
    );
    app.pause_host_for_league_vote();
    let status_commands = commands.take_runtime_status_commands();
    let [network::TestRuntimeStatusCommand::Change(pause)] = status_commands.as_slice() else {
        panic!("expected one Pause change, got {status_commands:?}");
    };
    let pause = *pause;
    assert_eq!(pause.state, clonk_network::NETWORK_STATE_PAUSE);
    assert_eq!(pause.target_tick, 1);
    assert!(app.network_control_running);

    app.test_update();
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (2, 1)
    );
    assert!(!app.network_control_running);
    assert_eq!(
        commands.take_runtime_status_commands(),
        vec![network::TestRuntimeStatusCommand::Reached {
            status: pause,
            actual_control_tick: 1,
        }]
    );
    app.sec1_timer().test_value();
    assert!(
        commands.take_runtime_status_commands().is_empty(),
        "local reach is sent once"
    );

    let retargeted = clonk_network::NetworkStatus {
        target_tick: 3,
        ..pause
    };
    events
        .send(NetworkEvent::StatusRequested(retargeted))
        .test_value();
    app.test_network_events();
    assert!(app.network_control_running);
    assert!(commands.take_runtime_status_commands().is_empty());
    queue_empty_ready_tick(&app, &events);
    app.test_update();
    app.test_update();
    queue_empty_ready_tick(&app, &events);
    app.test_update();
    app.test_update();
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (6, 3)
    );
    assert!(!app.network_control_running);
    assert_eq!(
        commands.take_runtime_status_commands(),
        vec![network::TestRuntimeStatusCommand::Reached {
            status: retargeted,
            actual_control_tick: 3,
        }]
    );

    events
        .send(NetworkEvent::StatusCommitted(retargeted))
        .test_value();
    app.test_network_events();
    assert!(!app.network_control_running);

    // A packet received during committed Pause is still raw, not
    // CtrlReady. Start must reach Go before re-enabling control and leave
    // this target batch for execution after the commit.
    queue_empty_ready_tick(&app, &events);
    app.test_network_events();
    app.finish_host_vote_pause(clonk_engine::VoteControlData {
        vote_type: clonk_engine::VOTE_TYPE_PAUSE,
        approve: true,
        data: 0,
        by_client: 0,
    });
    let status_commands = commands.take_runtime_status_commands();
    let [network::TestRuntimeStatusCommand::Change(go), network::TestRuntimeStatusCommand::Reached {
        status: reached_go,
        actual_control_tick,
    }] = status_commands.as_slice()
    else {
        panic!("expected Go change then immediate local reach, got {status_commands:?}");
    };
    let go = *go;
    assert_eq!(*reached_go, go);
    assert_eq!(*actual_control_tick, 3);
    assert_eq!(go.state, clonk_network::NETWORK_STATE_GO);
    assert_eq!(go.target_tick, 3);
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (6, 3)
    );
    assert!(!app.network_control_running);
    assert!(app.network_ticks.ready.contains_key(&3));
    events.send(NetworkEvent::StatusCommitted(go)).test_value();
    app.test_network_events();
    assert!(app.network_control_running);
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (7, 4)
    );
    assert!(!app.network_ticks.ready.contains_key(&3));
}

#[test]
fn runtime_host_rechecks_unreported_arrival_on_sec1_timer() {
    // C4Network2::OnSec1Timer calls Execute/CheckStatusReached even when
    // no simulation attempt is scheduled (src/C4Network2.cpp:674-690).
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 1);
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 0,
    };
    app.runtime_network_status_barrier = Some(RuntimeNetworkStatusBarrier {
        status: pause,
        local_reached: false,
        actual_control_tick: None,
    });

    app.sec1_timer().test_value();

    assert!(!app.network_control_running);
    assert_eq!(
        commands.take_runtime_status_commands(),
        vec![network::TestRuntimeStatusCommand::Reached {
            status: pause,
            actual_control_tick: 0,
        }]
    );
}

#[test]
fn message_board_change_mode_and_execute_match_native_faders() {
    let line_height = 15;
    let mut board = ClassicMessageBoardState::default();
    assert!(board.change_mode(MessageBoardMode::SingleLine, line_height));
    let first_frame = board.advance_frame(line_height, false);
    assert_eq!(first_frame.screen_fader, 90);
    assert_eq!(board.screen_fader, 95);
    assert!(board.empty);

    board.enqueue("first".to_string());
    board.enqueue("second".to_string());
    let _ = board.advance_frame(line_height, false);
    assert_eq!(board.fader, line_height - 1);
    assert_eq!(board.current_line().as_deref(), Some("second"));
    assert_eq!(board.screen_fader, 75);

    board.line_count = 7;
    assert!(board.change_mode(MessageBoardMode::Continuous, line_height));
    assert_eq!(board.mode, MessageBoardMode::Continuous);
    assert_eq!(board.line_count, 7);
    assert_eq!(board.back_scroll, -1);
    assert_eq!(board.fader, 0);
    let unchanged = board.clone();
    let _ = board.advance_frame(line_height, false);
    assert_eq!(board.mode, unchanged.mode);
    assert_eq!(board.screen_fader, unchanged.screen_fader);

    assert!(!board.change_mode(MessageBoardMode::Hidden, line_height));
    let _ = board.advance_frame(line_height, false);
    assert_eq!(board.screen_fader, 100);
    assert_eq!(board.back_scroll, -1);
    assert!(board.change_mode(MessageBoardMode::SingleLine, line_height));
    assert!(
        board.empty,
        "hidden-to-single keeps the native empty transition"
    );
}

#[test]
fn msgboard_command_uses_runtime_lines_but_persists_only_a_bool() {
    let _lock = env_lock().lock();
    let root = tempdir();
    let (_guard, paths, _) = loader_origin_fixture_paths(root.path());
    paths.ensure_user_dirs().test_value();
    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    let line_height = app.graphics.message_board_line_height();

    app.process_running_chat_text("/msgboard 4");
    assert_eq!(app.message_board.mode, MessageBoardMode::Continuous);
    assert_eq!(app.message_board.line_count, 4);
    for line in ["one", "two", "three", "four"] {
        app.enqueue_control_message_board_line(line.to_string());
    }
    let overlay = app.message_board_overlay();
    assert_eq!(overlay.mode, MessageBoardMode::Continuous);
    assert!(overlay.log_lines.ends_with(&[
        "one".to_string(),
        "two".to_string(),
        "three".to_string(),
        "four".to_string(),
    ]));
    let config = Config::load(paths.config_file()).test_value();
    assert_eq!(config.get_in(Some("Graphics"), "MsgBoard"), Some("1"));

    let mut reloaded = ClassicMessageBoardState::default();
    reloaded.initialize(load_message_board_enabled(Some(&paths)), line_height);
    assert_eq!(
        reloaded.mode,
        MessageBoardMode::SingleLine,
        "the bool-typed config cannot retain the runtime line count"
    );
    assert_eq!(reloaded.line_count, 1);

    app.process_running_chat_text("/msgboard 0");
    assert_eq!(app.message_board.mode, MessageBoardMode::Hidden);
    let config = Config::load(paths.config_file()).test_value();
    assert_eq!(config.get_in(Some("Graphics"), "MsgBoard"), Some("0"));
    let mut hidden_reloaded = ClassicMessageBoardState::default();
    hidden_reloaded.initialize(load_message_board_enabled(Some(&paths)), line_height);
    assert_eq!(hidden_reloaded.mode, MessageBoardMode::Hidden);
    assert_eq!(hidden_reloaded.line_count, 0);

    app.process_running_chat_text("/msgboard 21tail");
    assert_eq!(app.message_board.mode, MessageBoardMode::Continuous);
    assert_eq!(app.message_board.line_count, 20);
    app.process_running_chat_text("/msgboard 1");
    assert_eq!(app.message_board.mode, MessageBoardMode::SingleLine);
}

#[test]
fn running_set_comment_is_direct_host_effect() {
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    app.process_running_chat_text("/set comment live runtime comment");

    assert_eq!(
        Config::load(paths.config_file())
            .expect("load updated runtime config")
            .get_in(Some("Network"), "Comment"),
        Some("live runtime comment")
    );
    assert_eq!(
        app.advertised_game_reference
            .as_ref()
            .expect("updated running reference")
            .metadata()
            .comment
            .as_bytes(),
        b"live runtime comment"
    );
    assert_eq!(
        latest_message_board_logical_entry(&app).as_deref(),
        Some(clonk_frontend::game_option_buttons::COMMENT_CHANGED_LOG),
    );
    assert!(commands.take_submitted_decided_controls().is_empty());

    let mut client = new_state_only_running_sandbox_app();
    client.app_paths = Some(paths.clone());
    let (_events, mut client_commands) = install_running_network_stub(&mut client, 7, 0, 2);
    client.process_running_chat_text("/set comment rejected client comment");
    assert_eq!(
        Config::load(paths.config_file())
            .expect("load host-only runtime config")
            .get_in(Some("Network"), "Comment"),
        Some("live runtime comment")
    );
    assert!(client_commands.take_submitted_decided_controls().is_empty());
    assert!(message_board_logical_entries(&client)
        .iter()
        .all(|line| line != clonk_frontend::game_option_buttons::COMMENT_CHANGED_LOG));
}

#[test]
fn running_network_client_cannot_set_maxplayer_with_stale_engine_host_state() {
    // ProcessCommand authorizes the initialized control host. In a
    // network session that identity is the actual local client (host ID
    // zero), never a stale engine-side flag; C4ControlSet enforces the
    // same author again (src/C4GameControl.cpp:59-68;
    // src/C4MessageInput.cpp:475-492; src/C4Control.cpp:162-179).
    let mut app = new_state_only_running_sandbox_app();
    assert!(
        app.engine.is_control_host(),
        "fixture intentionally starts with stale local-host state"
    );
    let (_events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);

    app.process_running_chat_text("/set maxplayer 23");

    assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "an actual network client must not synthesize a host-authored Set"
    );
}

#[test]
fn running_dispatches_controls_modes_and_custom_commands() {
    let mut app = new_state_only_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);

    app.process_running_chat_text("/nodebug");
    let decided = commands.take_submitted_decided_controls();
    assert_eq!(decided.len(), 1);
    assert_eq!(decided[0].0, app.local_control_submission_tick());
    assert!(!decided[0].2, "an active running host queues CDT_Decide");
    assert_eq!(
        clonk_network::LegacyControlSet::from_control_packet(&decided[0].1),
        Some(clonk_network::LegacyControlSet {
            value_type: 1,
            data: 0,
            by_client: 0,
        })
    );
    app.process_running_chat_text("/set maxplayer 9");
    app.process_running_chat_text("/set faircrew off");
    let decided = commands.take_submitted_decided_controls();
    assert_eq!(decided.len(), 2);
    assert!(decided.iter().all(|(_, _, sync)| !sync));
    assert_eq!(
        decided
            .iter()
            .filter_map(|(_, control, _)| {
                clonk_network::LegacyControlSet::from_control_packet(control)
            })
            .collect::<Vec<_>>(),
        vec![
            clonk_network::LegacyControlSet {
                value_type: 2,
                data: 9,
                by_client: 0,
            },
            clonk_network::LegacyControlSet {
                value_type: 5,
                data: -1,
                by_client: 0,
            },
        ]
    );

    app.process_running_chat_text("/activate Remote");
    app.process_running_chat_text("/deactivate Remote");
    app.process_running_chat_text("/observer Remote");
    assert_eq!(
        commands.take_submitted_client_updates(),
        vec![
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 1,
                by_client: 0,
            },
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 0,
                by_client: 0,
            },
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                client_id: 7,
                data: 0,
                by_client: 0,
            },
        ]
    );

    app.engine.set_debug_mode(true);
    app.process_running_chat_text("/script return 1");
    let decided = commands.take_submitted_decided_controls();
    let [(tick, clonk_engine::ControlPacket::Script(script), false)] = decided.as_slice() else {
        panic!("expected one queued script command, got {decided:?}");
    };
    assert_eq!(*tick, app.local_control_submission_tick());
    assert_eq!(script.target_object, clonk_engine::SCRIPT_SCOPE_CONSOLE);
    assert_eq!(script.script.as_bytes(), b"return 1");

    assert!(app.engine.add_message_board_command(
        clonk_engine::InitialNetworkMessageBoardCommand {
            name: "probe".to_string(),
            script: "return true".to_string(),
            restriction: clonk_engine::MessageBoardCommandRestriction::Plain,
        },
    ));
    app.process_running_chat_text("/probe  exact tail");
    let decided = commands.take_submitted_decided_controls();
    let [(tick, clonk_engine::ControlPacket::CustomCommand(custom), false)] = decided.as_slice()
    else {
        panic!("expected one queued custom command, got {decided:?}");
    };
    assert_eq!(*tick, app.local_control_submission_tick());
    assert_eq!(custom.command.as_bytes(), b"probe");
    assert_eq!(custom.argument.as_bytes(), b" exact tail");
    assert_eq!(custom.player, app.local_owner);

    app.process_running_chat_text("/msgboard 12");
    assert_eq!(app.message_board.mode, MessageBoardMode::Continuous);
    assert_eq!(app.message_board.line_count, 12);
    app.process_running_chat_text("/msgboard 1");
    assert_eq!(app.message_board.mode, MessageBoardMode::SingleLine);
    app.process_running_chat_text("/msgboard 0");
    assert_eq!(app.message_board.mode, MessageBoardMode::Hidden);
    app.process_running_chat_text("/chart");
    assert!(app.network_chart_dialog.is_some());
    app.process_running_chat_text("/chart");
    assert!(app.network_chart_dialog.is_none());

    app.runtime_network_control_mode = Some(0);
    app.process_running_chat_text("/decentralctrl");
    assert!(commands.take_runtime_status_commands().is_empty());
    let (_snapshot, reference) = default_exact_host_reference();
    assert_eq!(reference.summary().control_mode, 0);
    app.advertised_game_reference = Some(reference);
    app.process_running_chat_text("/centralctrl");
    assert_eq!(
        commands.take_runtime_status_commands(),
        vec![
            network::TestRuntimeStatusCommand::Change(clonk_network::NetworkStatus {
                state: clonk_network::NETWORK_STATE_GO,
                control_mode: 1,
                target_tick: 0,
            }),
            network::TestRuntimeStatusCommand::Reached {
                status: clonk_network::NetworkStatus {
                    state: clonk_network::NETWORK_STATE_GO,
                    control_mode: 1,
                    target_tick: 0,
                },
                actual_control_tick: 0,
            },
        ]
    );
    assert_eq!(
        app.advertised_game_reference
            .as_ref()
            .expect("running reference survives control-mode publication")
            .summary()
            .control_mode,
        1
    );
    app.network_is_league = true;
    app.process_running_chat_text("/asyncctrl");
    assert!(commands.take_runtime_status_commands().is_empty());
    assert!(latest_message_board_logical_entry(&app)
        .as_deref()
        .is_some_and(|line| line.contains("not allowed in league")));
}

#[test]
fn network_chart_tracks_running_network_sandbox_and_toggles_as_singleton() {
    let mut app = new_running_sandbox_app();
    assert!(
        app.network_stats.is_some(),
        "running initialization owns live stats"
    );
    let (events, _commands) = install_running_network_stub(&mut app, 0, 0, 1);

    for _ in 0..3 {
        queue_empty_ready_tick(&app, &events);
        app.test_update();
    }
    app.sec1_timer().test_value();

    let cursor = app.engine.test_crew_cursor(app.local_owner);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate::new().with_status(clonk_engine::ObjectStatus::Inactive),
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    assert!(app
        .snapshot
        .objects
        .iter()
        .any(|object| object.status == clonk_engine::ObjectStatus::Inactive));
    let expected_object_count = app
        .snapshot
        .objects
        .iter()
        .filter(|object| object.status != clonk_engine::ObjectStatus::Deleted)
        .count() as f32;
    let object_sample_time = app.network_stats.test_ref().object_count_graph().end_time();
    app.record_network_stats_frame();
    assert_eq!(
        app.network_stats
            .as_ref()
            .expect("live stats")
            .object_count_graph()
            .value(object_sample_time),
        expected_object_count,
        "native object count includes inactive objects"
    );

    let second_counter = app.network_stats.test_ref().second_counter();
    app.mode = AppMode::Loading;
    app.record_network_stats_second();
    assert_eq!(
        app.network_stats
            .as_ref()
            .expect("stats survive the start barrier")
            .second_counter(),
        second_counter,
        "pre-GO loading must not append a per-second sample"
    );
    app.mode = AppMode::Running;

    app.process_running_chat_text("/chart");
    let dialog = app.network_chart_dialog.test_ref();
    assert_eq!(
        dialog.tab_names(),
        ["oc", "FPS", "NetIO", "Pings", "Control", "APM"]
    );
    assert!(
        dialog.tabs().iter().all(|tab| !tab.graph.is_empty()),
        "every live graph receives at least one frame/second/control sample"
    );
    let object_graph = dialog.active_graph().test_value();
    assert!(object_graph.end_time() - object_graph.start_time() >= 3);

    let chart_point = {
        let resources = app.assets.network_chart_resources().test_value();
        let preferred = scoreboard_preferred_rect(
            app.graphics
                .preferred_dialog_rect(app.mouse_control.then_some(app.local_owner)),
        );
        let layout = app
            .network_chart_dialog
            .test_ref()
            .layout(preferred, resources);
        GuiPoint::new(
            layout.caption.x.saturating_add(8) as f32,
            layout.caption.y.saturating_add(layout.caption.h / 2) as f32,
        )
    };
    app.running_pointer_position = Some(chart_point);
    assert!(app.handle_network_chart_pointer_button(ElementState::Pressed));
    assert!(app.network_chart_pointer_capture);
    app.ingame_pointer = None;
    app.ingame_edge_scroll = None;
    app.test_cursor(PhysicalPosition::new(10_000.0, 10_000.0));
    assert!(app.network_chart_pointer_capture);
    assert!(app.ingame_pointer.is_none());
    assert!(app.ingame_edge_scroll.is_none());
    app.handle_mouse_button_classified(ElementState::Released, false)
        .test_value();
    assert!(!app.network_chart_pointer_capture);

    app.process_running_chat_text("/chart");
    assert!(
        app.network_chart_dialog.is_none(),
        "a second activation closes the singleton"
    );
    assert!(
        app.network_stats.is_some(),
        "closing presentation retains sampling"
    );

    app.return_to_menu_for_relaunch();
    assert!(
        app.network_stats.is_none(),
        "game teardown drops live stats"
    );
    assert!(app.network_chart_dialog.is_none());
}

#[test]
fn chart_toggle_respects_reachable_native_key_priorities() {
    let configured = |binding: &str| {
        let mut app = new_classic_running_sandbox_app();
        app.runtime_key_config_cache = OnceLock::new();
        let source = format!("[Keys]\nChartToggle={binding}\n");
        app.runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(source.as_bytes()).test_value()))
            .test_value();
        app
    };

    for (name, key) in [
        ("F5", VirtualKeyCode::F5),
        ("F6", VirtualKeyCode::F6),
        ("F7", VirtualKeyCode::F7),
    ] {
        let mut app = configured(name);
        app.test_key(key, ElementState::Pressed);
        assert!(app.network_chart_dialog.is_some(), "{name}");
    }

    for (binding, key, modifiers) in [
        ("Left", VirtualKeyCode::ArrowLeft, ModifiersState::empty()),
        (
            "Shift+Right",
            VirtualKeyCode::ArrowRight,
            ModifiersState::SHIFT,
        ),
        (
            "Ctrl+Left",
            VirtualKeyCode::ArrowLeft,
            ModifiersState::CONTROL,
        ),
        (
            "Ctrl+Shift+Right",
            VirtualKeyCode::ArrowRight,
            ModifiersState::CONTROL | ModifiersState::SHIFT,
        ),
    ] {
        let mut chat = configured(binding);
        chat.start_running_chat(RunningChatMode::All);
        chat.keyboard_modifiers = modifiers;
        assert!(
            !chat.handle_runtime_chart_toggle_key(key, ElementState::Pressed),
            "focused chat Edit owns {binding}"
        );
        assert!(chat.network_chart_dialog.is_none());
    }

    let mut observer_menu = configured("Left");
    observer_menu.clear_physical_viewport_states();
    let observer = observer_menu.ownerless_physical_viewport_state();
    observer_menu.physical_viewports.push(observer);
    observer_menu.physical_viewports_authoritative = true;
    observer_menu.ingame_menu.replace(
        observer_menu.local_owner,
        IngameMenuState::main_menu(
            &MainMenuConditions {
                has_player: false,
                player_count: 0,
                ..MainMenuConditions::default()
            },
            &IngameMenuLabels::default(),
        ),
    );
    assert!(observer_menu.primary_physical_viewport_is_no_owner());
    assert!(!observer_menu
        .handle_runtime_chart_toggle_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed));

    let mut irc = configured("F8");
    irc.show_external_irc_dialog().test_value();
    irc.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(irc.external_irc_dialog_visible);
    assert!(irc.network_chart_dialog.is_some());
    assert!(irc.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart));

    let mut irc_unclaimed = configured("Alt+Z");
    irc_unclaimed.show_external_irc_dialog().test_value();
    irc_unclaimed.keyboard_modifiers = ModifiersState::ALT;
    irc_unclaimed.test_key(VirtualKeyCode::KeyZ, ElementState::Pressed);
    assert!(irc_unclaimed.network_chart_dialog.is_some());

    let mut irc_edit = configured("Ctrl+Shift+Left");
    irc_edit.show_external_irc_dialog().test_value();
    irc_edit
        .external_irc_dialog
        .test_mut()
        .force_chat_mode_and_focus();
    irc_edit.keyboard_modifiers = ModifiersState::CONTROL | ModifiersState::SHIFT;
    assert!(
        !irc_edit.handle_runtime_chart_toggle_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed)
    );

    let mut irc_connect = configured("Up");
    irc_connect.show_external_irc_dialog().test_value();
    irc_connect.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    assert!(irc_connect.network_chart_dialog.is_some());

    let mut game_over = new_game_over_keyboard_app();
    game_over.runtime_key_config_cache = OnceLock::new();
    game_over
        .runtime_key_config_cache
        .set(Ok(
            parse_runtime_key_config(b"[Keys]\nChartToggle=Alt+E\n").test_value()
        ))
        .test_value();
    game_over.keyboard_modifiers = ModifiersState::ALT;
    assert!(!game_over.handle_runtime_chart_toggle_key(VirtualKeyCode::KeyE, ElementState::Pressed));

    let mut game_over_list = new_game_over_keyboard_app();
    game_over_list.runtime_key_config_cache = OnceLock::new();
    game_over_list
        .runtime_key_config_cache
        .set(Ok(
            parse_runtime_key_config(b"[Keys]\nChartToggle=Up\n").test_value()
        ))
        .test_value();
    for _ in 0..2 {
        game_over_list.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        game_over_list.test_key(VirtualKeyCode::Tab, ElementState::Released);
    }
    assert!(matches!(
        game_over_list
            .game_over_dialog
            .as_ref()
            .and_then(GameOverState::focused),
        Some(GameOverFocus::PlayerList(_))
    ));
    assert!(!game_over_list
        .handle_runtime_chart_toggle_key(VirtualKeyCode::ArrowUp, ElementState::Pressed));

    let mut vote = configured("Alt+Y");
    vote.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::new(
            "Vote?",
            "Voting",
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
            clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
            clonk_frontend::message_dialog::MessageDialogSize::Regular,
            true,
        ),
        MessageDialogContinuation::LeagueSurrender,
    )
    .test_value();
    vote.keyboard_modifiers = ModifiersState::ALT;
    assert!(!vote.handle_runtime_chart_toggle_key(VirtualKeyCode::KeyY, ElementState::Pressed));

    let mut player_escape = configured("F8");
    player_escape
        .local_controls
        .remove(player_escape.local_owner);
    player_escape.local_controls.initialize(LocalControlInit {
        owner: player_escape.local_owner,
        preferred_set: 1,
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    assert!(player_escape.bindings.rebind_for_set(
        1,
        ControlBindingId::Left,
        VirtualKeyCode::Escape,
    ));
    player_escape.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    player_escape.test_key(VirtualKeyCode::F8, ElementState::Released);
    assert!(!player_escape.network_chart_owns_stronger_escape());
    player_escape.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert!(player_escape.network_chart_dialog.is_some());
    player_escape.test_key(VirtualKeyCode::Escape, ElementState::Released);
}

#[test]
fn chart_uses_native_placement_caption_drag_and_close_control() {
    let mut app = new_running_sandbox_app();
    app.resize(1280, 720).test_value();
    app.toggle_network_chart();
    let assets = Arc::clone(&app.assets);
    let resources = assets.network_chart_resources().test_value();
    let preferred = scoreboard_preferred_rect(
        app.graphics
            .preferred_dialog_rect(app.mouse_control.then_some(app.local_owner)),
    );
    let layout = app
        .network_chart_dialog
        .test_ref()
        .layout(preferred, resources);
    assert_eq!(
        (layout.bounds.x, layout.bounds.y),
        (preferred.x + 30, preferred.y + 30)
    );

    let caption = GuiPoint::new(
        (layout.caption.x + 8) as f32,
        (layout.caption.y + layout.caption.h / 2) as f32,
    );
    app.running_pointer_position = Some(caption);
    assert!(app.handle_network_chart_pointer_button(ElementState::Pressed));
    assert!(app.network_chart_pointer_capture);
    let moved = GuiPoint::new(caption.x + 37.0, caption.y + 19.0);
    app.running_pointer_position = Some(moved);
    assert!(app.handle_network_chart_pointer_move(moved));
    assert!(app.handle_network_chart_pointer_button(ElementState::Released));
    assert!(!app.network_chart_pointer_capture);
    let moved_layout = app
        .network_chart_dialog
        .test_ref()
        .layout(preferred, resources);
    assert_eq!(
        (moved_layout.bounds.x, moved_layout.bounds.y),
        (layout.bounds.x + 37, layout.bounds.y + 19)
    );

    let body = GuiPoint::new(
        (moved_layout.chart.x + moved_layout.chart.w / 2) as f32,
        (moved_layout.chart.y + moved_layout.chart.h / 2) as f32,
    );
    app.running_pointer_position = Some(body);
    assert!(app.handle_network_chart_pointer_button(ElementState::Pressed));
    assert!(
        !app.network_chart_pointer_capture,
        "chart body clicks are consumed without becoming a drag element"
    );
    assert!(app.handle_network_chart_pointer_button(ElementState::Released));

    let close = GuiPoint::new(
        (moved_layout.close_button.x + moved_layout.close_button.w / 2) as f32,
        (moved_layout.close_button.y + moved_layout.close_button.h / 2) as f32,
    );
    app.running_pointer_position = Some(close);
    assert!(app.handle_network_chart_pointer_button(ElementState::Pressed));
    assert!(app.network_chart_pointer_capture);
    assert!(app.handle_network_chart_pointer_button(ElementState::Released));
    assert!(app.network_chart_dialog.is_none());
}

#[test]
fn control_mode_targets_native_current_tick_after_cadence_consumption() {
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
    queue_empty_ready_tick(&app, &events);
    app.test_update();
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (1, 1)
    );

    app.runtime_network_control_mode = Some(0);
    app.process_running_chat_text("/centralctrl");

    let expected = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: 1,
        target_tick: 0,
    };
    assert_eq!(
        commands.take_runtime_status_commands(),
        vec![network::TestRuntimeStatusCommand::Change(expected)]
    );
}

#[test]
fn frozen_runtime_sync_executes_immediately_but_pending_barrier_queues() {
    let disable_debug = || {
        NetworkControl::Set(clonk_network::LegacyControlSet {
            value_type: 1,
            data: 0,
            by_client: 0,
        })
    };

    let mut frozen = new_state_only_running_sandbox_app();
    frozen.engine.set_debug_mode(true);
    frozen.engine.set_allow_debug(true);
    let (events, _commands) = install_running_network_stub(&mut frozen, 0, 0, 2);
    frozen.host_reference_paused = true;
    frozen.network_control_running = false;
    events
        .send(NetworkEvent::ScheduledSync {
            tick: 0,
            controls: vec![disable_debug()],
        })
        .test_value();

    frozen.test_network_events();
    assert!(!frozen.engine.debug_mode());
    assert!(frozen.network_sync.scheduled.is_empty());

    let mut transitioning = new_state_only_running_sandbox_app();
    transitioning.engine.set_debug_mode(true);
    transitioning.engine.set_allow_debug(true);
    let (events, _commands) = install_running_network_stub(&mut transitioning, 0, 0, 2);
    transitioning.host_reference_paused = true;
    transitioning.network_control_running = false;
    transitioning.runtime_network_status_barrier = Some(RuntimeNetworkStatusBarrier {
        status: clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 0,
        },
        local_reached: true,
        actual_control_tick: Some(0),
    });
    events
        .send(NetworkEvent::ScheduledSync {
            tick: 0,
            controls: vec![disable_debug()],
        })
        .test_value();

    transitioning.test_network_events();
    assert!(transitioning.engine.debug_mode());
    assert!(transitioning.network_sync.scheduled.contains_key(&0));
}

#[test]
fn menu_touch_title_drag_uses_touch_coordinates_through_release() {
    let mut app = new_menu_app(640, 480);
    let dialog = clonk_frontend::message_dialog::MessageDialogState::new(
        "Move this confirmation",
        "Caption",
        clonk_frontend::message_dialog::MessageDialogButtons::OK,
        clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        clonk_frontend::message_dialog::MessageDialogSize::Small,
        false,
    );
    app.push_message_dialog(dialog, MessageDialogContinuation::None)
        .test_value();
    let layout = app.top_message_dialog_layout().test_value();
    let caption = layout.caption.test_value();
    let start = GuiPoint::new((caption.x + 10) as f32, (caption.y + 10) as f32);
    let end = GuiPoint::new(start.x + 41.0, start.y + 27.0);
    app.running_pointer_position = Some(GuiPoint::new(1.0, 1.0));

    app.test_touch(TouchPhase::Started, start);
    assert!(app.message_dialogs[0].state.has_positional_pointer_drag());
    app.test_touch(TouchPhase::Ended, end);

    assert_eq!(app.message_dialogs[0].state.dialog_offset(), (41, 27));
    assert!(!app.message_dialogs[0].state.has_pointer_capture());
    assert_eq!(app.message_dialogs.len(), 1);
}

#[test]
fn message_control_sound_uses_global_cooldown_before_per_client_mute() {
    let mut app = new_state_only_running_sandbox_app();
    install_message_fixture(&mut app);
    app.control_messages = ControlMessageState::new(Duration::from_secs(5), false);
    let start = Instant::now();
    let mut attempts = 0;

    let first = app.execute_message_control_with_sound_at(
        message_control(MESSAGE_TYPE_SOUND, -1, -1, b"Ping", 7),
        start,
        |_, name| {
            attempts += 1;
            assert_eq!(name, "Ping");
            true
        },
    );
    assert!(first.sound_attempted && first.sound_played);

    let other_sender = app.execute_message_control_with_sound_at(
        message_control(MESSAGE_TYPE_SOUND, -1, -1, b"Ping", 8),
        start + Duration::from_secs(4),
        |_, _| {
            attempts += 1;
            true
        },
    );
    assert!(!other_sender.sound_attempted);

    app.control_messages.set_muted(8, true);
    let muted = app.execute_message_control_with_sound_at(
        message_control(MESSAGE_TYPE_SOUND, -1, -1, b"Ping", 8),
        start + Duration::from_secs(5),
        |_, _| {
            attempts += 1;
            true
        },
    );
    assert!(!muted.sound_attempted);
    app.control_messages.set_muted(8, false);
    let consumed = app.execute_message_control_with_sound_at(
        message_control(MESSAGE_TYPE_SOUND, -1, -1, b"Ping", 7),
        start + Duration::from_secs(6),
        |_, _| {
            attempts += 1;
            true
        },
    );
    assert!(!consumed.sound_attempted);
    assert_eq!(attempts, 1);
}

#[test]
fn message_control_host_system_and_inactive_attention_match_cpp() {
    let mut app = new_state_only_running_sandbox_app();
    install_message_fixture(&mut app);
    app.window_active = false;

    assert!(
        !app.execute_message_control(message_control(MESSAGE_TYPE_SYSTEM, -1, -1, b"forged", 7,))
            .displayed
    );
    assert!(
        app.execute_message_control(message_control(
            MESSAGE_TYPE_SYSTEM,
            -1,
            -1,
            b"host notice",
            0,
        ))
        .displayed
    );
    assert!(!app.take_user_attention_request());

    let alert = app.execute_message_control(message_control(MESSAGE_TYPE_ALERT, -1, -1, b"", 7));
    assert!(alert.attention_requested);
    assert!(app.take_user_attention_request());

    let mention =
        app.execute_message_control(message_control(MESSAGE_TYPE_NORMAL, 7, -1, b"hi aLi!", 7));
    assert!(mention.attention_requested);
    assert!(app.take_user_attention_request());

    let rejected_first_match = app.execute_message_control(message_control(
        MESSAGE_TYPE_NORMAL,
        7,
        -1,
        b"Malice, Ali!",
        7,
    ));
    assert!(!rejected_first_match.attention_requested);
    assert!(!app.take_user_attention_request());
}

#[test]
fn portrait_crew_label_decodes_native_info_name_for_presentation() {
    let mut app = new_state_only_running_sandbox_app();
    let crew = app.engine.test_crew_cursor(app.local_owner);
    let raw_name = clonk_script::c4_string_from_bytes(b"Ren\xe9");
    let mut state = app.engine.capture_state();
    let definition_id = app.snapshot.object(crew).test_value().definition_id.clone();
    assert!(
        app.engine
            .definition_portrait_graphics_image(&definition_id)
            .is_some(),
        "fixture definition must have a portrait so current=None is observable"
    );
    state.crew_object_infos.insert(
        crew,
        clonk_engine::CrewObjectInfo {
            core: Default::default(),
            definition_id,
            name: raw_name,
            death_message: String::new(),
            rank: 0,
            rank_name: "Clonk".to_string(),
            experience: 0,
            rounds: 0,
            death_count: 0,
            total_playing_time: 0,
            birthday: 0,
            age: 0,
            participation: 0,
            in_action_time: 0,
            extra_data: Vec::new(),
            portraits: Default::default(),
        },
    );
    app.engine.restore_state(&state).test_value();
    app.snapshot = app.engine.snapshot();

    let mut players = collect_player_overlays(
        &mut app.engine,
        &app.snapshot,
        Some(crew),
        &app.bindings,
        &app.gamepad_bindings,
    );
    app.display_flags.portraits = true;
    app.populate_crew_infos(&mut players);
    app.populate_crew_portraits(&mut players);

    let overlay = players
        .iter()
        .flat_map(|player| &player.crew)
        .find(|overlay| overlay.object_id == crew)
        .test_value();
    assert_eq!(overlay.info_name.as_deref(), Some("Ren\u{e9}"));
    assert_eq!(overlay.label, "Ren\u{e9}");
    assert!(
        overlay.portrait.is_none() && overlay.portrait_owner_overlay.is_none(),
        "an info with no current portrait must not draw the definition's first portrait"
    );
    assert_eq!(
        clonk_script::c4_string_bytes(
            &app.engine
                .crew_object_info(crew)
                .expect("crew info remains live")
                .name
        ),
        b"Ren\xe9"
    );
}

#[test]
fn viewport_selection_preserves_authoritative_local_order_slots_and_elimination() {
    // C++ creates one viewport for each LocalControl player and keeps
    // every viewport through elimination (C4Game.cpp:2736-2746;
    // C4Player.cpp:2015-2037). The snapshot projection must therefore
    // retain local-player order, duplicate-owner slots, centers and zoom.
    let mut app = new_running_sandbox_app();
    app.snapshot.hud.messages.clear();
    let mut snapshot = app.snapshot.clone();
    let local_owner = app.local_owner;
    let local = snapshot
        .players
        .iter()
        .find(|player| player.id == local_owner)
        .cloned()
        .test_value();
    let focus = local
        .viewports
        .first()
        .and_then(|viewport| viewport.focus)
        .or(local.cursor)
        .or_else(|| local.crew.first().copied())
        .test_value();
    let mut second = local.clone();
    second.id = local_owner + 1;
    second.name = "Second".to_string();
    second.viewports[0].center = Vector2::new(700, 800);
    second.viewports[0].zoom = 2.0;

    let mut local_with_split = local.clone();
    local_with_split.viewports.push(
        clonk_engine::PlayerViewport::new(Vector2::new(300, 400))
            .with_focus(Some(focus))
            .with_zoom(1.5),
    );

    // A remote player appears first and even shares focus; locality, not
    // global focus de-duplication or player-list order, decides selection.
    // Two local players may also retain the same focus object.
    snapshot.players = vec![second.clone(), local_with_split.clone()];
    snapshot.hud.local_players = vec![local_owner, second.id];
    let viewports = collect_viewport_inputs(&snapshot).test_value();
    assert_eq!(viewports.len(), 3);
    assert_eq!(
        viewports
            .iter()
            .map(|viewport| (
                viewport.owner,
                viewport.center,
                viewport.zoom,
                viewport.focus.expect("local viewport focus").id,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                local_owner,
                local_with_split.viewports[0].center,
                local_with_split.viewports[0].zoom,
                focus,
            ),
            (local_owner, Vector2::new(300, 400), 1.5, focus),
            (second.id, Vector2::new(700, 800), 2.0, focus),
        ]
    );
    let mut ordinary_frame = vec![0x73; app.graphics.surface().pixels().len()];
    app.render_running(&mut ordinary_frame, false).test_value();

    // An unset or deleted slot focus follows only the owning player's
    // live cursor and then first live crew entry. It never consults
    // app-global focus or an unrelated object in snapshot order.
    let mut inherited = local.clone();
    inherited.viewports = vec![clonk_engine::PlayerViewport::new(Vector2::new(17, 29))];
    inherited.viewports[0].focus = Some(ObjectId::new(u64::MAX));
    inherited.cursor = Some(focus);
    snapshot.players = vec![inherited.clone()];
    snapshot.hud.local_players = vec![local_owner];
    assert_eq!(
        collect_viewport_inputs(&snapshot).expect("live cursor supersedes a deleted slot focus")[0]
            .focus
            .expect("cursor focus")
            .id,
        focus
    );
    inherited.cursor = None;
    inherited.crew = vec![focus];
    snapshot.players = vec![inherited];
    assert_eq!(
        collect_viewport_inputs(&snapshot)
            .expect("first crew member supplies the local slot focus")[0]
            .focus
            .expect("crew focus")
            .id,
        focus
    );

    // Elimination does not close C++ viewports. Preserve the viewport's
    // own payload as proof this is not an observer or first-object
    // substitute, and prove the ordinary renderer accepts it.
    let mut eliminated = local.clone();
    eliminated.status = clonk_engine::PlayerStatus::Eliminated;
    eliminated.viewports[0].center = Vector2::new(123, 456);
    eliminated.viewports[0].zoom = 1.75;
    snapshot.players = vec![eliminated.clone()];
    snapshot.hud.local_players = vec![local_owner];
    let eliminated_views = collect_viewport_inputs(&snapshot).test_value();
    assert_eq!(eliminated_views.len(), 1);
    assert_eq!(eliminated_views[0].owner, local_owner);
    assert_eq!(eliminated_views[0].center, Vector2::new(123, 456));
    assert_eq!(eliminated_views[0].zoom, 1.75);

    app.snapshot.players = vec![eliminated];
    app.snapshot.hud.local_players = vec![local_owner];
    let mut frame = vec![0x91; app.graphics.surface().pixels().len()];
    app.render_running(&mut frame, false).test_value();
}

#[test]
fn viewport_selection_uses_cpp_keyboard_layout_order_across_joins() {
    assert_eq!(
        [-1, 0, 1, 2, 3, 4, 7].map(classic_viewport_layout_order),
        [-1, 0, 3, 1, 2, 4, 7],
        "non-keyboard controls pass through and Keyboard1/3/4/2 map to row-major order",
    );

    let app = new_state_only_running_sandbox_app();
    let mut snapshot = app.snapshot.clone();
    let template = snapshot
        .players
        .iter()
        .find(|player| player.id == app.local_owner)
        .cloned()
        .test_value();
    let make_player = |id, control_set, center| {
        let mut player = template.clone();
        player.id = id;
        player.name = format!("Keyboard {}", control_set + 1);
        player.control_set = control_set;
        player.viewports.truncate(1);
        player.viewports[0].center = center;
        player
    };

    // IDs deliberately oppose C++ layout order. Engine snapshots expose
    // local IDs in numeric order, while C4GraphicsSystem assigns cells by
    // Keyboard1, Keyboard3, Keyboard4, Keyboard2.
    let keyboard2 = make_player(10, 1, Vector2::new(100, 10));
    let keyboard4 = make_player(20, 3, Vector2::new(200, 20));
    let keyboard3 = make_player(30, 2, Vector2::new(300, 30));
    let keyboard1 = make_player(40, 0, Vector2::new(400, 40));
    let viewport_owners = |snapshot: &SimulationSnapshot| {
        collect_viewport_inputs(snapshot)
            .test_value()
            .into_iter()
            .map(|viewport| viewport.owner)
            .collect::<Vec<_>>()
    };

    snapshot.players = vec![keyboard2.clone(), keyboard1.clone()];
    snapshot.hud.local_players = vec![keyboard2.id, keyboard1.id];
    let initial = viewport_owners(&snapshot);
    assert_eq!(initial, vec![keyboard1.id, keyboard2.id]);

    // Joining Keyboard3 inserts its new cell between Keyboard1 and
    // Keyboard2 without reversing the existing players' relative order.
    snapshot.players.insert(1, keyboard3.clone());
    snapshot.hud.local_players.insert(1, keyboard3.id);
    let joined = viewport_owners(&snapshot);
    assert_eq!(joined, vec![keyboard1.id, keyboard3.id, keyboard2.id]);
    assert_eq!(
        joined
            .iter()
            .copied()
            .filter(|owner| initial.contains(owner))
            .collect::<Vec<_>>(),
        initial,
    );

    snapshot.players.insert(1, keyboard4.clone());
    snapshot.hud.local_players.insert(1, keyboard4.id);
    let all_keyboards = vec![keyboard1.id, keyboard3.id, keyboard4.id, keyboard2.id];
    for _ in 0..8 {
        assert_eq!(viewport_owners(&snapshot), all_keyboards);
    }
    snapshot.players.reverse();
    snapshot.hud.local_players.reverse();
    assert_eq!(
        viewport_owners(&snapshot),
        all_keyboards,
        "unique control layouts make slot assignment independent of source order",
    );
}

#[test]
fn pending_preflight_retains_the_exact_network_tick() {
    // C4Control::PreExecute inspects the complete list before execution;
    // a pending packet leaves iControlReady unchanged so the same tick is
    // retried intact (src/C4Control.cpp:73-90;
    // src/C4GameControlNetwork.cpp:687-719).
    let tick = 7;
    let controls = vec![NetworkControl::JoinPlayer(
        clonk_engine::JoinPlayerControlData::default(),
    )];
    let mut gate = NetworkTickGate::default();
    gate.queue(tick, tick, controls.clone());

    assert!(gate.take_exact_if_ready(tick, |_| false).is_none());
    assert_eq!(
        gate.take_exact_if_ready(tick, |_| true),
        Some(controls),
        "the retained tick executes once its preflight becomes ready"
    );
}

#[test]
fn synchronized_team_configuration_keeps_all_join_data_query_values() {
    let host_config = clonk_network::HostConfig::default();
    let mut parameters = host_config.initial_join_snapshot.test_value().parameters;
    parameters.teams.custom = 1;
    parameters.teams.active = 0;
    parameters.teams.allow_hostility_change = 1;
    parameters.teams.team_distribution = 4;
    parameters.teams.allow_team_switch = 1;
    parameters.teams.auto_generate_teams = 0;
    parameters.teams.team_colors = 1;

    assert_eq!(
        synchronized_team_configuration(&parameters),
        TeamConfiguration {
            custom: true,
            active: false,
            allow_hostility_change: true,
            distribution: 4,
            allow_team_switch: true,
            auto_generate_teams: false,
            team_colors: true,
        }
    );
}

#[test]
fn client_join_data_replaces_authoritative_control_registries() {
    // HandleJoinData deep-copies Game.Parameters. Game.Clients and
    // Game.PlayerInfos are references into that snapshot, so stale local
    // entries are replaced and the raw LastPlayerID counter is retained
    // (src/C4Network2.cpp:1574-1620; src/C4Game.cpp:64-70;
    // src/C4PlayerInfo.cpp:649-665).
    let mut app = new_state_only_menu_app(320, 200);
    app.control_clients.register(99, true, false);
    app.network_client_activity.mark_activated(99, 123);
    app.control_player_infos.apply(netplay_player_info_data(
        99,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: 3,
            ..Default::default()
        }],
    ));
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.clients = clonk_network::JoinClientRegistrySnapshot {
        clients: vec![
            clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: clonk_engine::LegacyCString::from_bytes(b"Host".to_vec())
                    .expect("valid host name"),
                lobby_ready: true,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 7,
                name: clonk_engine::LegacyCString::from_bytes(b"Joining client".to_vec())
                    .expect("valid client name"),
                nick: clonk_engine::LegacyCString::from_bytes(b"Joiner".to_vec())
                    .expect("valid client nick"),
                ..Default::default()
            },
        ],
        local_client_id: Some(7),
    };
    snapshot.parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: 40,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 12,
                ..Default::default()
            }],
        }],
    };
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 23,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    event_tx
        .send(NetworkEvent::JoinData(join_data.clone()))
        .test_value();

    app.test_network_events();

    assert_eq!(app.pending_network_join_data, Some(join_data));
    assert!(!app.control_clients.contains(99));
    assert_eq!(
        app.network_client_activity.last_frame,
        BTreeMap::from([(0, 0), (7, 0)])
    );
    let host = app.control_clients.state(0).test_value();
    assert!(host.activated);
    assert!(host.lobby_ready);
    let local = app.control_clients.state(7).test_value();
    assert_eq!(local.name.as_bytes(), b"Joining client");
    assert_eq!(local.nick.as_bytes(), b"Joiner");
    assert!(app.control_player_infos.get(3).is_none());
    assert!(app.control_player_infos.get(12).is_some());
    let admitted = app
        .control_player_infos
        .admit_request(
            clonk_engine::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
            },
            4,
        )
        .test_value();
    assert_eq!(admitted.players[0].id, 41);
}

#[test]
fn synchronized_player_derivation_registers_the_latest_core_before_event_drain() {
    // C4Player::Save looks the resource up by its stable mutable filename,
    // so every later save derives from the newest official resource. The
    // mutable path and its ownership remain the derived resource's file
    // (src/C4Player.cpp:452-461; src/C4Network2Res.cpp:718-823).
    let mut resources = AdmissionResourceStore::default();
    let root = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 40,
        loadable: true,
        ..Default::default()
    };
    let first = clonk_engine::NetworkResourceCore {
        id: 41,
        derived_id: root.id,
        ..root.clone()
    };
    let second = clonk_engine::NetworkResourceCore {
        id: 42,
        derived_id: first.id,
        ..root.clone()
    };
    resources.register_lobby_resource(&root);
    resources.mark_complete_with_locality(root.id, PathBuf::from("Alice.c4p"), true);
    resources.register_finished_derivation(
        &first,
        PathBuf::from("Alice.c4p"),
        clonk_network::ResourceFileOwnership::Persistent,
    );
    resources.register_finished_derivation(
        &second,
        PathBuf::from("Alice.c4p"),
        clonk_network::ResourceFileOwnership::Persistent,
    );

    assert_eq!(resources.derivation_target(root.id), Some(second.id));
    assert_eq!(
        resources.status(second.id),
        Some(&AdmissionResourceState::Complete {
            path: PathBuf::from("Alice.c4p"),
            removed: false,
            local: true,
        })
    );

    let remote = clonk_engine::NetworkResourceCore { id: 50, ..root };
    resources.register_lobby_resource(&remote);
    resources.mark_complete_with_locality(remote.id, PathBuf::from("Network/Bob.c4p"), false);
    let remote_derived = clonk_engine::NetworkResourceCore {
        id: 51,
        derived_id: remote.id,
        ..remote.clone()
    };
    resources.register_finished_derivation(
        &remote_derived,
        PathBuf::from("Network/Bob.c4p"),
        clonk_network::ResourceFileOwnership::Temporary,
    );
    assert_eq!(
        resources.derivation_target(remote.id),
        Some(remote_derived.id)
    );
    assert_eq!(
        resources.status(remote_derived.id),
        Some(&AdmissionResourceState::Complete {
            path: PathBuf::from("Network/Bob.c4p"),
            removed: false,
            local: false,
        })
    );
}

#[test]
fn main_menu_player_join_uses_active_network_max_players() {
    // ActivateMain compares the live Game.Players count with the host's
    // synchronized Game.Parameters.MaxPlayers before adding New Player;
    // it never substitutes the scenario default after JoinData
    // (pristine 9ffa0a5d src/C4MainMenu.cpp:643-686).
    let mut app = new_state_only_running_sandbox_app();
    app.network_max_players = 1;
    app.snapshot.players = vec![clonk_engine::PlayerState {
        id: app.local_owner,
        ..Default::default()
    }];

    let conditions = app.main_menu_conditions();
    let menu = IngameMenuState::main_menu(&conditions, &IngameMenuLabels::default()).test_value();

    assert_eq!(conditions.max_players, 1);
    assert!(!menu
        .items()
        .iter()
        .any(|item| item.action == MenuAction::ActivateNewPlayer));
}

#[test]
fn offline_runtime_join_player_local_no_network() {
    // JoinPlayer:<file> selects CtrlJoinLocalNoNetwork when networking is
    // disabled. That path loads one local AddPlayers record, applies it as
    // CID_PlrInfo, and lets LocalJoinUnjoinedPlayersInQueue issue the
    // filename-backed join for the local control client
    // (src/C4MainMenu.cpp:761-771; src/C4PlayerList.cpp:320-330;
    // src/C4PlayerInfo.cpp:693-733).
    let player_path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    let player_file = PlayerFile::load_from_path(player_path).test_value();
    let mut app = new_synthetic_running_sandbox_app();
    app.startup_player_files.push(StartupPlayerFile {
        path: player_path.to_path_buf(),
        file_name: player_path
            .file_name()
            .test_value()
            .to_string_lossy()
            .into_owned(),
        player_file: player_file.clone(),
        render_model: clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: player_file.name.clone(),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: player_file.normalized_preferred_color(),
            score: player_file.score,
            rounds: player_file.rounds,
            rounds_won: player_file.rounds_won,
            rounds_lost: player_file.rounds_lost,
            total_playing_time: player_file.total_playing_time,
            comment: String::new(),
        },
    });
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    let before_players = app.engine.snapshot().players.len();
    let before_info_ids = app.control_player_infos.client_info_ids(0);
    app.status_text = "offline join sentinel".to_string();
    let recording_directory = tempdir();
    let recording_path = recording_directory.path().join("001-OfflineRuntime.c4s");
    install_test_recording_template(&mut app, recording_path.clone());
    app.start_recording(true).test_value();
    let recorded_frame = u32::try_from(app.engine.frame()).test_value();

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();

    let joined_info_ids = app
        .control_player_infos
        .client_info_ids(0)
        .into_iter()
        .filter(|info_id| !before_info_ids.contains(info_id))
        .collect::<Vec<_>>();
    let [joined_info_id] = joined_info_ids.as_slice() else {
        panic!("expected one new local player info, got {joined_info_ids:?}");
    };
    let info = app.control_player_infos.get(*joined_info_id).test_value();
    assert_eq!(
        info.filename.as_bytes(),
        clonk_script::c4_string_bytes(player_path.to_string_lossy().as_ref())
    );
    assert_eq!(
        info.name.as_bytes(),
        clonk_script::c4_string_bytes(&player_file.name)
    );
    assert_eq!(
        (info.color, info.original_color),
        (
            player_file.normalized_preferred_color(),
            player_file.normalized_preferred_color(),
        )
    );
    assert_eq!(info.resource, None);
    assert_eq!(
        info.flags
            & (clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE | clonk_engine::PLAYER_INFO_FLAG_JOINED),
        clonk_engine::PLAYER_INFO_FLAG_JOINED
    );

    let snapshot = app.engine.snapshot();
    assert_eq!(snapshot.players.len(), before_players + 1);
    let joined = snapshot
        .players
        .iter()
        .find(|player| player.player_info_id == *joined_info_id)
        .test_value();
    assert_eq!(
        app.engine
            .player(joined.id)
            .expect("joined player remains live")
            .at_client(),
        clonk_engine::PlayerAtClient::HOST
    );
    assert!(snapshot.hud.local_players.contains(&joined.id));
    assert!(app.local_controls.assignment(joined.id).is_some());
    assert!(app
        .physical_viewports
        .iter()
        .any(|viewport| viewport.displayed_player == joined.id));
    assert_eq!(app.status_text, "offline join sentinel");
    app.apply_ingame_menu_action(MenuAction::ActivateNewPlayer)
        .test_value();
    assert!(app
        .ingame_menu
        .as_ref()
        .expect("other runtime player rows keep the menu open")
        .items()
        .iter()
        .all(|item| {
            item.action != MenuAction::JoinPlayer(player_path.to_string_lossy().into_owned())
        }));

    assert!(app.finish_recording().is_none());
    let recording = Group::open(&recording_path).test_value();
    let mut playback =
        ControlRecordPlayback::from_bytes(&recording.read_file("CtrlRec.c4b").test_value())
            .test_value();
    let recorded_controls = playback.take_controls(recorded_frame);
    assert!(recorded_controls.iter().any(|packet| {
        matches!(
            packet,
            clonk_engine::ControlPacket::PlayerInfo(info)
                if info.players.iter().any(|player| player.id == *joined_info_id)
        )
    }));
    assert!(recorded_controls.iter().any(|packet| {
        matches!(
            packet,
            clonk_engine::ControlPacket::JoinPlayer(join)
                if join.info_id == *joined_info_id
                    && matches!(
                        &join.source,
                        clonk_engine::JoinPlayerSource::Embedded(data) if !data.is_empty()
                    )
        )
    }));

    let malformed_directory = tempdir();
    let malformed = malformed_directory.path().join("Malformed.c4p");
    fs::write(&malformed, b"not a packed player group").test_value();
    let before_failure_players = app.engine.snapshot().players;
    let before_failure_infos = app.control_player_infos.retained_rows_snapshot();
    let error = app
        .apply_ingame_menu_action(MenuAction::JoinPlayer(
            malformed.to_string_lossy().into_owned(),
        ))
        .expect_err("malformed offline player returns a typed boundary");
    let detail = match error {
        EngineError::ClassicMenuParityBoundary { detail } => detail,
        other => panic!("offline failure returned the wrong error: {other}"),
    };
    assert!(detail.contains("classic offline in-game player join failed"));
    assert!(detail.contains("failed to load"));
    assert!(detail.contains(&malformed.to_string_lossy().into_owned()));
    assert_eq!(app.status_text, "offline join sentinel");
    assert_eq!(app.engine.snapshot().players, before_failure_players);
    assert_eq!(
        app.control_player_infos.retained_rows_snapshot(),
        before_failure_infos
    );
}

#[test]
fn retired_local_player_releases_profile_and_preferred_controls_for_runtime_rejoin() {
    // C4PlayerList::Retire routes through Remove, which marks the linked
    // C4PlayerInfo removed before deleting the live player; that makes the
    // profile available to ActivateNewPlayer again (src/C4PlayerList.cpp:219-267,
    // 398-409; src/C4MainMenu.cpp:59-121).
    let player_path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    let player_file = PlayerFile::load_from_path(player_path).test_value();
    let mut app = new_synthetic_running_sandbox_app();
    app.startup_player_files.push(StartupPlayerFile {
        path: player_path.to_path_buf(),
        file_name: player_path
            .file_name()
            .test_value()
            .to_string_lossy()
            .into_owned(),
        player_file: player_file.clone(),
        render_model: clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: player_file.name.clone(),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: player_file.normalized_preferred_color(),
            score: player_file.score,
            rounds: player_file.rounds,
            rounds_won: player_file.rounds_won,
            rounds_lost: player_file.rounds_lost,
            total_playing_time: player_file.total_playing_time,
            comment: String::new(),
        },
    });
    let before_info_ids = app.control_player_infos.client_info_ids(0);

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();

    let retired_info_id = app
        .control_player_infos
        .client_info_ids(0)
        .into_iter()
        .find(|info_id| !before_info_ids.contains(info_id))
        .test_value();
    let retired_owner = app
        .engine
        .players()
        .find(|player| player.player_info_id() == retired_info_id)
        .map(clonk_engine::Player::id)
        .test_value();
    let original_control = app.local_controls.assignment(retired_owner).test_value();

    assert!(app
        .engine
        .execute_eliminate_player_control(&clonk_engine::EliminatePlayerControlData {
            player: retired_owner,
            by_client: 0,
        })
        .expect("execute host elimination"));
    for _ in 0..60 {
        app.test_update();
    }

    assert!(app.engine.player(retired_owner).is_none());
    let retired_info = app.control_player_infos.get(retired_info_id).test_value();
    assert_ne!(
        retired_info.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED,
        0,
        "automatic retirement must release the profile from FileInUse"
    );
    assert!(app
        .available_runtime_player_files()
        .iter()
        .any(|entry| { entry.file == player_path.to_string_lossy() }));
    let info_ids_before_rejoin = app.control_player_infos.client_info_ids(0);

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();

    let rejoined_info_id = app
        .control_player_infos
        .client_info_ids(0)
        .into_iter()
        .find(|info_id| !info_ids_before_rejoin.contains(info_id))
        .test_value();
    let rejoined_owner = app
        .engine
        .players()
        .find(|player| player.player_info_id() == rejoined_info_id)
        .map(clonk_engine::Player::id)
        .test_value();
    let rejoined_control = app.local_controls.assignment(rejoined_owner).test_value();
    assert_eq!(rejoined_control.set, original_control.set);
    assert_eq!(rejoined_control.mouse, original_control.mouse);
}

#[test]
fn active_network_client_runtime_join_publishes_before_add_request() {
    // JoinPlayer:<file> calls JoinLocalPlayer(file, true). LoadFromLocalFile
    // publishes the NRT_Player resource before the client sends the
    // CIF_AddPlayers PID_PlayerInfoUpdReq to the host
    // (pristine 9ffa0a5d src/C4MainMenu.cpp:760-771;
    // src/C4PlayerInfo.cpp:70-104,357-395;
    // src/C4Network2Players.cpp:78-137).
    let directory = tempdir();
    let player_path = directory.path().join("Runtime.c4p");
    let mut player_group = clonk_resources::MutableGroup::new("Runtime.c4p");
    player_group
        .add_file_with_metadata(
            "Player.txt",
            b"[Player]\nName=Runtime\n[Preferences]\nColorDw=6636321\n".to_vec(),
            1,
            false,
        )
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();

    let mut app = new_running_sandbox_app();
    app.player_name = "Exact maker".to_string();
    let (manager, _event_tx, commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let mut client_settings =
        ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client");
    client_settings.group_maker = LegacyCString::from_bytes(b"Exact maker".to_vec()).test_value();
    app.network_mode = Some(NetworkMode::Client(client_settings));
    app.control_clients.register(7, true, false);
    let before_players = app.engine.snapshot().players;

    let wire_name = clonk_engine::LegacyCString::from_bytes(
        player_path.as_os_str().as_encoded_bytes().to_vec(),
    )
    .test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 7 << 16,
        loadable: true,
        filename: wire_name.clone(),
        ..Default::default()
    };
    let expected_resource = resource.clone();
    let command_observer =
        thread::spawn(move || commands.complete_initial_client_join(vec![resource]));

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();
    assert_eq!(
        app.engine.snapshot().players,
        before_players,
        "the client waits for the host's synchronized player-info echo before mutating players"
    );
    assert_eq!(
        app.admission_resources.complete_path(expected_resource.id),
        Some(player_path.as_path()),
        "the publishing client keeps its own player resource complete"
    );
    drop(app.network.take());

    let (order, publications, player_infos, acknowledgements) = command_observer.test_join();
    assert_eq!(order, vec!["publish", "player-info"]);
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].source_path, player_path);
    assert_eq!(publications[0].wire_name, wire_name.clone());
    assert_eq!(publications[0].group_maker.as_bytes(), b"Exact maker");
    assert_eq!(player_infos.len(), 1);
    assert_eq!(player_infos[0].client_id, 7);
    assert_eq!(
        player_infos[0].flags,
        clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
    );
    let player = &player_infos[0].players[0];
    assert_eq!(player.id, 0);
    assert_eq!(player.name.as_bytes(), b"Runtime");
    assert_eq!(player.filename, wire_name);
    assert_eq!(player.color, 0x65_43_21);
    assert_eq!(player.original_color, 0x65_43_21);
    assert_eq!(player.resource, Some(expected_resource));
    assert!(acknowledgements.is_empty());
}

#[test]
fn deactivated_network_client_can_request_runtime_player_join() {
    // JoinLocalPlayer permits an inactive non-observer to send its
    // CIF_AddPlayers PID_PlayerInfoUpdReq, then requests activation
    // (src/C4Network2Players.cpp:78-137).
    let directory = tempdir();
    let player_path = directory.path().join("Rejoin.c4p");
    let mut player_group = clonk_resources::MutableGroup::new("Rejoin.c4p");
    player_group
        .add_file_with_metadata(
            "Player.txt",
            b"[Player]\nName=Rejoin\n[Preferences]\nColorDw=6636321\n".to_vec(),
            1,
            false,
        )
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();

    let mut app = new_running_sandbox_app();
    let (manager, _event_tx, commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.control_clients.register(7, false, false);

    let wire_name = clonk_engine::LegacyCString::from_bytes(
        player_path.as_os_str().as_encoded_bytes().to_vec(),
    )
    .test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 7 << 16,
        loadable: true,
        filename: wire_name,
        ..Default::default()
    };
    let command_observer =
        thread::spawn(move || commands.complete_initial_client_join(vec![resource]));

    let result = app.submit_runtime_network_player(&player_path.to_string_lossy());
    drop(app.network.take());
    let (order, _, player_infos, _) = command_observer.test_join();

    result.test_value();
    assert_eq!(order, vec!["publish", "player-info"]);
    let [request] = player_infos.as_slice() else {
        panic!("expected one runtime player-info request");
    };
    assert_eq!(request.client_id, 7);
    assert_eq!(
        request.flags,
        clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
    );
}

#[test]
fn network_observer_cannot_request_lobby_player_join() {
    // JoinLocalPlayer rejects an observer before opening the selected player,
    // including the /joinplr route used while a lobby is active
    // (src/C4Network2Players.cpp:78-85; src/C4GameLobby.cpp:516-526).
    let directory = tempdir();
    let missing_player = directory.path().join("Observer.c4p");
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, _commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Observer",
    )));
    app.control_clients.register(7, false, true);

    assert_eq!(
        app.submit_network_player_path(&missing_player, "Observer.c4p", false)
            .expect_err("an observer cannot add a lobby player"),
        "network observers cannot join players"
    );
}

#[test]
fn active_network_host_runtime_join_publishes_admits_and_queues_join() {
    // JoinLocalPlayer(file, true) first lets LoadFromLocalFile publish an
    // NRT_Player. A host handles CIF_AddPlayers directly, assigns the next
    // player ID, broadcasts authoritative PlayerInfo with CDT_Direct, and
    // the running-host handler then queues JoinPlayer with the resource
    // file (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104;
    // src/C4Network2Players.cpp:78-137,160-239,245-270,353-388).
    let directory = tempdir();
    let player_path = directory.path().join("HostRuntime.c4p");
    let mut player_group = clonk_resources::MutableGroup::new("HostRuntime.c4p");
    player_group
        .add_file_with_metadata(
            "Player.txt",
            b"[Player]\nName=Host Runtime\n[Preferences]\nColorDw=1193046\n".to_vec(),
            1,
            false,
        )
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();

    let mut app = new_running_sandbox_app();
    app.player_name = "Exact host maker".to_string();
    app.control_clients.register(0, true, false);
    app.control_player_infos.replace_snapshot(40, []);
    let tick = app.local_control_submission_tick();
    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    let wire_name = clonk_engine::LegacyCString::from_bytes(
        player_path.as_os_str().as_encoded_bytes().to_vec(),
    )
    .test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: wire_name.clone(),
        ..Default::default()
    };
    let expected_resource = resource.clone();
    let (direct_ready, direct_wait) = std::sync::mpsc::channel();
    let command_observer = thread::spawn(move || {
        commands.complete_runtime_host_join(resource, event_tx, direct_ready)
    });

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();
    direct_wait
        .recv_timeout(Duration::from_secs(1))
        .test_value();
    app.test_network_events();
    drop(app.network.take());

    let (order, publications, player_infos, joins) = command_observer.test_join();
    assert_eq!(order, vec!["publish", "player-info", "join-player"]);
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].source_path, player_path);
    assert_eq!(publications[0].wire_name, wire_name.clone());
    assert_eq!(publications[0].group_maker.as_bytes(), b"Exact host maker");
    let [info] = player_infos.as_slice() else {
        panic!("expected one authoritative PlayerInfo");
    };
    assert_eq!((info.client_id, info.by_client), (0, 0));
    assert_eq!(
        info.flags,
        clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
    );
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted player");
    };
    assert_eq!(player.id, 41);
    assert_eq!(player.name.as_bytes(), b"Host Runtime");
    assert_eq!(player.resource.as_ref(), Some(&expected_resource));
    assert_eq!(joins.len(), 1);
    assert_eq!(joins[0].0, tick);
    assert_eq!(joins[0].1.at_client, 0);
    assert_eq!(joins[0].1.info_id, 41);
    assert_eq!(
        joins[0].1.filename.as_bytes(),
        player_path.as_os_str().as_encoded_bytes()
    );
    assert_eq!(
        joins[0].1.source,
        clonk_engine::JoinPlayerSource::Resource(expected_resource)
    );
}

#[test]
fn inactive_remote_client_readds_retired_profile_after_activation_go() {
    // C4PlayerList::Remove calls SetRemoved on the shared C4PlayerInfo before
    // deleting the live player on every peer. Rust mirrors that link outside
    // the engine, so the authoritative host must also repair a remote client's
    // retained list. An inactive requester is admitted before activation, then
    // OnStatusGoReached issues its deferred JoinPlayer
    // (src/C4PlayerList.cpp:219-267,398-409;
    // src/C4Network2Players.cpp:160-270,465-482).
    let mut app = new_running_sandbox_app();
    app.control_clients.register(0, true, false);
    app.control_clients.register(7, true, false);
    let wire_name = LegacyCString::from_bytes(b"ClientRejoin.c4p".to_vec()).test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 7 << 16,
        loadable: true,
        ..Default::default()
    };
    app.control_player_infos.replace_snapshot(
        41,
        [clonk_engine::PlayerInfoControlData {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 41,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                    | clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                name: LegacyCString::from_bytes(b"Client Rejoin".to_vec()).test_value(),
                filename: wire_name.clone(),
                resource: Some(resource.clone()),
                ..Default::default()
            }],
            by_client: 0,
        }],
    );
    let retiring = app.local_owner + 1;
    app.engine
        .register_player(PlayerConfig::new(retiring, "Client Rejoin").with_player_info_id(41))
        .test_value();
    app.engine
        .test_player_mut(retiring)
        .set_at_client(clonk_engine::PlayerAtClient::new(7));
    app.snapshot = app.engine.snapshot();

    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    assert!(app
        .engine
        .execute_eliminate_player_control(&clonk_engine::EliminatePlayerControlData {
            player: retiring,
            by_client: 0,
        })
        .expect("execute remote player elimination"));
    let players_before_tick = app.player_info_ids_by_player();
    for _ in 0..60 {
        app.snapshot = app.engine.test_tick();
    }
    app.mirror_retired_player_info(&players_before_tick);

    let retirement_updates = commands.take_preexecuted_player_infos();
    let [(info, join_players_on_echo)] = retirement_updates.as_slice() else {
        panic!("expected one authoritative remote-retirement PlayerInfo");
    };
    assert_eq!((info.client_id, info.by_client), (7, 0));
    assert!(join_players_on_echo.is_empty());
    let [player] = info.players.as_slice() else {
        panic!("expected the remote client's retained player row");
    };
    assert_eq!(player.id, 41);
    assert_ne!(
        player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED,
        0,
        "the remote client must receive the released profile before rejoining"
    );
    app.control_clients.register(7, false, false);
    let expected_resource = resource.clone();

    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 7,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    name: LegacyCString::from_bytes(b"Client Rejoin".to_vec()).test_value(),
                    filename: wire_name,
                    resource: Some(resource),
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .test_value();
    app.test_network_events();

    let readmission_updates = commands.take_preexecuted_player_infos();
    let [(info, join_players_on_echo)] = readmission_updates.as_slice() else {
        panic!("expected one authoritative remote-client readmission");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected the readmitted remote player row");
    };
    assert_eq!((info.client_id, player.id), (7, 42));
    assert!(
        join_players_on_echo.is_empty(),
        "the inactive client must wait for synchronized activation"
    );

    let frame = i32::try_from(app.engine.frame()).test_value();
    event_tx
        .send(NetworkEvent::ActivationRequest {
            client_id: 7,
            tick: frame,
            waited_for: true,
            ping_ms: 0,
        })
        .test_value();
    app.test_network_events();
    let activations = commands.take_submitted_client_updates();
    let [activation] = activations.as_slice() else {
        panic!("expected the host-authored activation control");
    };
    let activation = activation.clone();
    assert_eq!(
        activation,
        clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 7,
            data: 1,
            by_client: 0,
        }
    );
    let tick = app.local_control_submission_tick();
    app.network_control_running = false;
    event_tx
        .send(NetworkEvent::ScheduledSync {
            tick,
            controls: vec![NetworkControl::ClientUpdate(activation)],
        })
        .test_value();
    app.test_network_events();
    assert!(!app.control_clients.is_activated(7));
    assert!(app.network_sync.scheduled.contains_key(&tick));
    assert!(commands.take_submitted_join_players().is_empty());

    event_tx
        .send(NetworkEvent::StatusCommitted(
            clonk_network::NetworkStatus {
                state: clonk_network::NETWORK_STATE_GO,
                control_mode: 1,
                target_tick: i32::try_from(tick).test_value(),
            },
        ))
        .test_value();
    app.test_network_events();

    assert!(app.control_clients.is_activated(7));
    let joins = commands.take_submitted_join_players();
    let [(join_tick, join)] = joins.as_slice() else {
        panic!("expected one deferred remote-client JoinPlayer");
    };
    assert_eq!(*join_tick, tick);
    assert_eq!((join.at_client, join.info_id), (7, 42));
    assert_eq!(join.by_client, 0);
    assert_eq!(
        join.source,
        clonk_engine::JoinPlayerSource::Resource(expected_resource)
    );
}

#[test]
fn active_network_host_readmits_its_own_retired_profile_at_runtime() {
    // C4PlayerList::Retire routes through Remove, which calls
    // C4PlayerInfo::SetRemoved before deleting the live player, so the host's
    // own later CIF_AddPlayers request for that profile is no longer the
    // "double join" HandlePlayerInfoUpdRequest denies without message
    // (src/C4PlayerList.cpp:219-267,398-409; src/C4Network2Players.cpp:160-181;
    // src/C4PlayerInfo.cpp:568-580).
    let directory = tempdir();
    let player_path = directory.path().join("HostRejoin.c4p");
    let mut player_group = clonk_resources::MutableGroup::new("HostRejoin.c4p");
    player_group
        .add_file_with_metadata(
            "Player.txt",
            b"[Player]\nName=Host Rejoin\n[Preferences]\nColorDw=1193046\n".to_vec(),
            1,
            false,
        )
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();

    let mut app = new_running_sandbox_app();
    app.player_name = "Exact host maker".to_string();
    app.control_clients.register(0, true, false);

    let wire_name = clonk_engine::LegacyCString::from_bytes(
        player_path.as_os_str().as_encoded_bytes().to_vec(),
    )
    .test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: wire_name.clone(),
        ..Default::default()
    };
    // The host is already playing that profile: one joined PlayerInfo row for
    // its own client carrying the published NRT_Player, bound to a live player.
    app.control_player_infos.replace_snapshot(
        41,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 41,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                    | clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                filename: wire_name.clone(),
                resource: Some(resource.clone()),
                ..Default::default()
            }],
            by_client: 0,
        }],
    );
    let retiring = app.local_owner + 1;
    app.engine
        .register_player(PlayerConfig::new(retiring, "Host Rejoin").with_player_info_id(41))
        .test_value();
    app.snapshot = app.engine.snapshot();

    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    assert!(app
        .engine
        .execute_eliminate_player_control(&clonk_engine::EliminatePlayerControlData {
            player: retiring,
            by_client: 0,
        })
        .expect("execute host elimination"));
    // The network frame loop cannot drive the delay here: its per-tick
    // FinalizeTick command blocking-sends into the stub's bounded channel.
    // Retirement is the engine's own, so advance it and mirror the one frame
    // that removes the player exactly as the running app does.
    let players_before_tick = app.player_info_ids_by_player();
    for _ in 0..60 {
        app.snapshot = app.engine.test_tick();
    }
    app.mirror_retired_player_info(&players_before_tick);
    assert!(
        app.engine.player(retiring).is_none(),
        "the eliminated player retires after the 60-frame delay"
    );
    let retired = app.control_player_infos.get(41).test_value();
    assert_ne!(
        retired.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED,
        0,
        "retirement releases the profile before the host readmits it"
    );

    let (direct_ready, direct_wait) = std::sync::mpsc::channel();
    let command_observer = thread::spawn(move || {
        commands.complete_runtime_host_join(resource, event_tx, direct_ready)
    });

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();
    let broadcast = direct_wait.recv_timeout(Duration::from_secs(1));
    assert!(
        !app.status_text.starts_with("Unable to join player"),
        "the readmitted profile must not be refused: {}",
        app.status_text
    );
    broadcast.test_value();
    app.test_network_events();

    let (order, _publications, player_infos, joins) = command_observer.test_join();
    assert_eq!(order, vec!["publish", "player-info", "join-player"]);
    let [info] = player_infos.as_slice() else {
        panic!("expected one authoritative PlayerInfo");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted player");
    };
    assert_eq!(player.id, 42, "the readmitted profile gets a fresh ID");
    assert_eq!(joins.len(), 1);
    assert_eq!(joins[0].1.info_id, 42);

    let first_rejoin = joins[0].1.clone();
    let repeated_resource = match &first_rejoin.source {
        clonk_engine::JoinPlayerSource::Resource(resource) => resource.clone(),
        source => panic!("network rejoin used the wrong source: {source:?}"),
    };
    app.apply_join_player_control(first_rejoin).test_value();
    let rejoined = app
        .engine
        .players()
        .find(|player| player.player_info_id() == 42)
        .map(clonk_engine::Player::id)
        .test_value();

    assert!(app
        .engine
        .execute_eliminate_player_control(&clonk_engine::EliminatePlayerControlData {
            player: rejoined,
            by_client: 0,
        })
        .expect("execute second host elimination"));
    for _ in 0..60 {
        app.snapshot = app.engine.test_tick();
    }
    // A retained Joined row can outlive the one frame where its runtime
    // player disappears. Reconcile it from the current live roster, as C++'s
    // C4PlayerList::Remove has already called SetRemoved by this point
    // (src/C4PlayerList.cpp:219-267,398-409).
    let players_after_second_retirement = app.player_info_ids_by_player();
    app.mirror_retired_player_info(&players_after_second_retirement);
    assert!(app.engine.player(rejoined).is_none());
    assert_ne!(
        app.control_player_infos.get(42).test_value().flags
            & clonk_engine::PLAYER_INFO_FLAG_REMOVED,
        0,
        "the first rejoin must release the profile when it retires"
    );

    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let (direct_ready, direct_wait) = std::sync::mpsc::channel();
    let command_observer = thread::spawn(move || {
        commands.complete_runtime_host_join(repeated_resource, event_tx, direct_ready)
    });

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();
    let broadcast = direct_wait.recv_timeout(Duration::from_secs(1));
    assert!(
        !app.status_text.starts_with("Unable to join player"),
        "the profile's second readmission must not be refused: {}",
        app.status_text
    );
    broadcast.test_value();
    app.test_network_events();
    drop(app.network.take());

    let (order, _publications, player_infos, joins) = command_observer.test_join();
    assert_eq!(order, vec!["publish", "player-info", "join-player"]);
    let [info] = player_infos.as_slice() else {
        panic!("expected a second authoritative PlayerInfo");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected a second admitted player");
    };
    assert_eq!(player.id, 43, "the second rejoin gets another fresh ID");
    assert_eq!(joins.len(), 1);
    assert_eq!(joins[0].1.info_id, 43);
}

#[test]
fn rejoin_after_elimination_policy_reads_its_network_config_key() {
    // clonk-org/clonk-rs#240. The key is a clonk-rs extension with no C4Config
    // counterpart, so an absent one must leave the oracle behaviour in place.
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(640, 480, &paths);

    assert!(app.rejoin_after_elimination_allowed());

    persist_config_value(&paths, "Network", "NoRejoinAfterElimination", "1").test_value();
    assert!(!app.rejoin_after_elimination_allowed());

    persist_config_value(&paths, "Network", "NoRejoinAfterElimination", "0").test_value();
    assert!(app.rejoin_after_elimination_allowed());

    // A host that changed the policy for this session outranks the file, the
    // same way runtime join does.
    app.network_rejoin_after_elimination_allowed = Some(false);
    assert!(!app.rejoin_after_elimination_allowed());
}

#[test]
fn active_network_host_barring_rejoins_refuses_its_own_retired_profile() {
    // clonk-org/clonk-rs#240: the same runtime readmission a stock host grants,
    // refused because this host set Config.Network.NoRejoinAfterElimination.
    // Admission still denies without a synchronized message, exactly like the
    // double join it is treated as (src/C4Network2Players.cpp:160-181).
    let directory = tempdir();
    let player_path = directory.path().join("HostRejoin.c4p");
    let mut player_group = clonk_resources::MutableGroup::new("HostRejoin.c4p");
    player_group
        .add_file_with_metadata(
            "Player.txt",
            b"[Player]\nName=Host Rejoin\n[Preferences]\nColorDw=1193046\n".to_vec(),
            1,
            false,
        )
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();

    let mut app = new_running_sandbox_app();
    app.player_name = "Exact host maker".to_string();
    app.control_clients.register(0, true, false);
    app.network_rejoin_after_elimination_allowed = Some(false);

    let wire_name = clonk_engine::LegacyCString::from_bytes(
        player_path.as_os_str().as_encoded_bytes().to_vec(),
    )
    .test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: wire_name.clone(),
        ..Default::default()
    };
    app.control_player_infos.replace_snapshot(
        41,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 41,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                    | clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                filename: wire_name.clone(),
                resource: Some(resource.clone()),
                ..Default::default()
            }],
            by_client: 0,
        }],
    );
    let retiring = app.local_owner + 1;
    app.engine
        .register_player(PlayerConfig::new(retiring, "Host Rejoin").with_player_info_id(41))
        .test_value();
    app.snapshot = app.engine.snapshot();

    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    assert!(app
        .engine
        .execute_eliminate_player_control(&clonk_engine::EliminatePlayerControlData {
            player: retiring,
            by_client: 0,
        })
        .expect("execute host elimination"));
    let players_before_tick = app.player_info_ids_by_player();
    for _ in 0..60 {
        app.snapshot = app.engine.test_tick();
    }
    app.mirror_retired_player_info(&players_before_tick);
    assert!(
        app.engine.player(retiring).is_none(),
        "the eliminated player retires after the 60-frame delay"
    );

    let (direct_ready, _direct_wait) = std::sync::mpsc::channel();
    let command_observer = thread::spawn(move || {
        commands.complete_runtime_host_join(resource, event_tx, direct_ready)
    });

    app.apply_ingame_menu_action(MenuAction::JoinPlayer(
        player_path.to_string_lossy().into_owned(),
    ))
    .test_value();
    assert!(
        app.status_text.starts_with("Unable to join player"),
        "the eliminated profile must be refused: {}",
        app.status_text
    );
    drop(app.network.take());

    // The resource is published before admission runs, as in C++, but nothing
    // is broadcast and no player is queued to join.
    let (order, _publications, player_infos, joins) = command_observer.test_join();
    assert_eq!(order, vec!["publish"]);
    assert!(player_infos.is_empty());
    assert!(joins.is_empty());
    assert_eq!(
        app.control_player_infos.player_count(),
        1,
        "only the retired history row remains"
    );
}

#[test]
fn active_network_host_runtime_join_assigns_team_before_broadcast() {
    // The host handles its local CIF_AddPlayers packet directly, assigning
    // its ID and team before broadcasting authoritative PlayerInfo
    // (src/C4Network2Players.cpp:78-137,160-205;
    // src/C4Teams.cpp:53-81,474-542).
    let directory = tempdir();
    let player_path = directory.path().join("HostTeamRuntime.c4p");
    let mut player_group = clonk_resources::MutableGroup::new("HostTeamRuntime.c4p");
    player_group
                .add_file_with_metadata(
                    "Player.txt",
                    b"[Player]\nName=Host Team Runtime\n[Preferences]\nColorDw=1193046\nAlternateColorDw=6636321\n".to_vec(),
                    1,
                    false,
                ).test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();

    let team = |id, player_ids, color| clonk_engine::InitialNetworkTeam {
        id,
        name: clonk_engine::LegacyCString::from_bytes(format!("Team {id}").into_bytes())
            .test_value(),
        player_start_index: 0,
        player_ids,
        color,
        icon_spec: clonk_engine::LegacyCString::default(),
        max_players: 0,
    };
    let mut app = new_state_only_running_sandbox_app();
    app.player_name = "Exact host maker".to_string();
    app.control_clients.register(0, true, false);
    app.control_player_infos.replace_snapshot(
        1,
        [netplay_player_info_data(
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                team: 1,
                color: 0x0012_3456,
                original_color: 0x0012_3456,
                ..Default::default()
            }],
        )],
    );
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
        clonk_engine::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: clonk_engine::InitialNetworkTeamDistribution::Random,
            team_colors: false,
            max_script_players: 0,
            script_player_names: clonk_engine::LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![1], 0x00f4_0000),
                team(2, Vec::new(), 0x0000_c800),
            ],
        },
    ));
    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let wire_name = clonk_engine::LegacyCString::from_bytes(
        player_path.as_os_str().as_encoded_bytes().to_vec(),
    )
    .test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: wire_name,
        ..Default::default()
    };
    let (direct_ready, direct_wait) = std::sync::mpsc::channel();
    let command_observer = thread::spawn(move || {
        commands.complete_runtime_host_join(resource, event_tx, direct_ready)
    });

    app.submit_runtime_network_player(&player_path.to_string_lossy())
        .test_value();
    direct_wait
        .recv_timeout(Duration::from_secs(1))
        .test_value();
    app.test_network_events();
    drop(app.network.take());

    let (_, _, player_infos, _) = command_observer.test_join();
    let [info] = player_infos.as_slice() else {
        panic!("expected one authoritative PlayerInfo");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted player");
    };
    assert_eq!((player.id, player.team), (2, 2));
    assert_eq!(player.original_color, 0x0012_3456);
    assert_ne!(
            player.color, player.original_color,
            "native skips the alternate while current still equals original and enters its process-random fallback"
        );
    assert_eq!(
        app.host_local_alternate_colors_by_resource.get(&17),
        Some(&0x0065_4321),
        "a successful local runtime join extends the persistent sidecar"
    );
    assert!(app.host_local_player_info_ids.contains(&2));
    let teams = app.network_team_assignment.test_mut().teams_mut();
    assert_eq!(teams.teams[0].player_ids, vec![1]);
    assert_eq!(teams.teams[1].player_ids, vec![2]);
}

#[test]
fn client_join_data_submits_an_empty_initial_player_info_for_an_observer() {
    // JoinLocalPlayer sends even an empty CIF_Initial request so the host
    // marks the client as an observer and answers with all player infos;
    // it happens before DoLobby reaches and acknowledges GS_Lobby
    // (src/C4Network2Players.cpp:38-49,78-137;
    // src/C4Game.cpp:3840-3844).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Observer",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Observer".to_string(), false));
    app.selected_player_file = None;

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot
        .parameters
        .clients
        .clients
        .push(clonk_engine::ClientCoreControlData {
            client_id: 7,
            name: clonk_engine::LegacyCString::from_bytes(b"Observer".to_vec()).test_value(),
            ..Default::default()
        });
    snapshot.parameters.clients.local_client_id = Some(7);
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: snapshot.dynamic_tick,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    event_tx
        .send(NetworkEvent::JoinData(join_data))
        .test_value();

    app.test_network_events();

    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: Vec::new(),
        }]
    );
}

#[test]
fn startup_host_auth_players_stay_with_their_connection_and_cancel_cleanly() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "Network", "LeagueAutoLogin", "0").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let auth = clonk_network::LeagueAuthRequestHead {
        account: LegacyCString::from_bytes(b"account".to_vec()).test_value(),
        password: LegacyCString::from_bytes(b"password".to_vec()).test_value(),
        ..Default::default()
    };
    let player = |name: &[u8]| clonk_engine::ControlPlayerInfoEntry {
        name: LegacyCString::from_bytes(name.to_vec()).test_value(),
        ..Default::default()
    };

    let (first_manager, _events, mut first_commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    assert_eq!(
        app.begin_league_player_auth_exchange(
            LeaguePlayerAuthContinuation::StartupHost {
                mode: NetworkMode::Host(host_network_settings()),
                manager: first_manager,
                selected_scenario: None,
                purpose: StartupNetworkPurpose::StagedHost,
                players: vec![player(b"First Host")],
                index: 0,
                server_name: "league.example".to_string(),
            },
            auth.clone(),
            clonk_frontend::league_signup::LeagueSignupMode::Login,
        )
        .expect("begin first host Auth"),
        LeaguePlayerAuthStatus::Pending
    );
    assert!(first_commands.receive_league_player_auth().complete(Ok(
        clonk_network::decode_league_auth_response(
            b"[Response]\r\nStatus=Success\r\nAUID=first-token\r\n",
        ),
    )));
    app.poll_league_player_auth().test_value();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    let first_connection = app.startup_network_connection.take().test_value();
    let first_players = first_connection.authenticated_league_players.test_ref();
    assert_eq!(first_players[0].name.as_bytes(), b"First Host");
    assert_eq!(first_players[0].auth_id.as_bytes(), b"first-token");
    drop(first_connection);

    let (second_manager, _events, mut second_commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    assert_eq!(
        app.begin_league_player_auth_exchange(
            LeaguePlayerAuthContinuation::StartupHost {
                mode: NetworkMode::Host(host_network_settings()),
                manager: second_manager,
                selected_scenario: None,
                purpose: StartupNetworkPurpose::StagedHost,
                players: vec![player(b"Second Host")],
                index: 0,
                server_name: "league.example".to_string(),
            },
            auth,
            clonk_frontend::league_signup::LeagueSignupMode::Login,
        )
        .expect("begin replacement host Auth"),
        LeaguePlayerAuthStatus::Pending
    );
    let abandoned = second_commands.receive_league_player_auth();
    assert_eq!(abandoned.player.name.as_bytes(), b"Second Host");
    app.show_main_menu();
    assert!(app.pending_league_player_auth.is_none());
    assert!(app.startup_network_connection.is_none());
    assert!(!app.message_dialogs.iter().any(|dialog| matches!(
        dialog.continuation,
        MessageDialogContinuation::LeaguePlayerAuthWait
            | MessageDialogContinuation::LeaguePlayerAuthWelcome
            | MessageDialogContinuation::LeaguePlayerAuthError
            | MessageDialogContinuation::LeaguePlayerAuthCancelled
    )));
    assert!(!abandoned.complete(Ok(clonk_network::LeagueAuthResponse::default())));
}

#[test]
fn client_rejects_a_combined_scenario_without_network_game_flag() {
    // After RetrieveScenario and RetrieveFiles, C4Game aborts before
    // InitScriptEngine when the combined C4S Head.NetworkGame flag is false
    // (pristine 9ffa0a5d src/C4Game.cpp:2526-2564).
    let directory = tempdir();
    let scenario_path = directory.path().join("Combined7.c4s");
    let definition_path = scenario_path.join("Defs.c4d");
    fs::create_dir_all(&definition_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Offline payload\nNetworkGame=false\n\n[Definitions]\nDefinition1=Defs.c4d\n",
    )
    .test_value();
    fs::write(
        definition_path.join("DefCore.txt"),
        "[DefCore]\nid=TEST\nName=Test\nCategory=1\n",
    )
    .test_value();
    write_test_definition_graphics(&definition_path);
    let scenario =
        Scenario::load_from_path_with(&scenario_path, &InstallDefinitionResolver::new(None))
            .test_value();

    assert_eq!(
        validate_client_network_scenario(&scenario),
        Err("retrieved scenario is not marked as a network game".to_string())
    );
}

#[test]
fn host_player_info_request_queues_authoritative_direct_broadcast() {
    // HandlePlayerInfoUpdRequest assigns IDs, then sends one host-authored
    // C4ControlPlayerInfo with CDT_Direct and executes that host-authored
    // control synchronously (src/C4Network2Players.cpp:160-239;
    // src/C4Control.cpp:1264-1282).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 9,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 0,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .test_value();

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [info] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo broadcast");
    };
    assert_eq!((info.client_id, info.by_client), (3, 0));
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted player");
    };
    assert_eq!(player.id, 1);
    assert!(app.control_player_infos.get(1).is_some());
}

#[test]
fn same_client_add_echo_normalizes_and_issues_only_its_direct_snapshot() {
    let mut app = new_state_only_running_sandbox_app();
    app.control_clients.register(3, true, false);
    app.control_player_infos.replace_snapshot(0, []);
    let resources = [
        clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 17,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"First.c4p".to_vec()).test_value(),
            ..Default::default()
        },
        clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 18,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"Second.c4p".to_vec()).test_value(),
            ..Default::default()
        },
    ];
    for resource in &resources {
        app.admission_resources.register_lobby_resource(resource);
        app.admission_resources.mark_complete(
            resource.id,
            PathBuf::from(resource.filename.to_string_lossy().into_owned()),
        );
    }
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    for (index, (name, color)) in [
        (b"First".as_slice(), 0x00f4_0000),
        (b"Second".as_slice(), 0x0000_00f4),
    ]
    .into_iter()
    .enumerate()
    {
        event_tx
            .send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: 3,
                request: clonk_network::PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        name: clonk_engine::LegacyCString::from_bytes(name.to_vec()).unwrap(),
                        color,
                        original_color: color,
                        resource: Some(resources[index].clone()),
                        ..Default::default()
                    }],
                },
                by_host: false,
            })
            .test_value();
    }

    app.test_network_events();
    let controls = commands.take_preexecuted_player_infos();
    assert_eq!(controls.len(), 2);
    assert_eq!(
        controls[0]
            .1
            .iter()
            .map(|player| player.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(
        controls[1]
            .1
            .iter()
            .map(|player| player.id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    app.control_clients.register(3, false, false);

    let mut first_echo = controls[0].0.clone();
    first_echo.players[0].flags |= clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE;
    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: controls[0].0.clone(),
            info: first_echo,
            join_players_on_echo: controls[0].1.clone(),
        })
        .test_value();
    app.test_network_events();
    let first_joins = commands.take_submitted_join_players();
    assert_eq!(
        first_joins
            .iter()
            .map(|(_, join)| join.info_id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_ne!(
        app.control_player_infos.get(1).unwrap().flags
            & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0,
        "A Add normalization must survive the later B Add echo"
    );

    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: controls[1].0.clone(),
            info: controls[1].0.clone(),
            join_players_on_echo: controls[1].1.clone(),
        })
        .test_value();
    app.test_network_events();
    let second_joins = commands.take_submitted_join_players();
    assert_eq!(
        second_joins
            .iter()
            .map(|(_, join)| join.info_id)
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert_ne!(
        app.control_player_infos.get(1).unwrap().flags
            & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0
    );
}

#[test]
fn stale_add_echo_joins_its_snapshot_without_normalizing_later_replacement() {
    let mut app = new_state_only_running_sandbox_app();
    app.control_clients.register(3, true, false);
    app.control_player_infos.replace_snapshot(0, []);
    let resource = |id, filename: &[u8]| clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(filename.to_vec()).test_value(),
        ..Default::default()
    };
    let first_resource = resource(17, b"First.c4p");
    let replacement_resource = resource(18, b"Replacement.c4p");
    for core in [&first_resource, &replacement_resource] {
        app.admission_resources.register_lobby_resource(core);
        app.admission_resources.mark_complete(
            core.id,
            PathBuf::from(core.filename.to_string_lossy().into_owned()),
        );
    }
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    for request in [
        clonk_network::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                name: clonk_engine::LegacyCString::from_bytes(b"First".to_vec()).unwrap(),
                color: 0x00f4_0000,
                original_color: 0x00f4_0000,
                resource: Some(first_resource.clone()),
                ..Default::default()
            }],
        },
        clonk_network::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: clonk_engine::LegacyCString::from_bytes(b"Replacement".to_vec()).unwrap(),
                color: 0x0000_00f4,
                original_color: 0x0000_00f4,
                resource: Some(replacement_resource.clone()),
                ..Default::default()
            }],
        },
    ] {
        event_tx
            .send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: 3,
                request,
                by_host: false,
            })
            .test_value();
    }

    app.test_network_events();
    let controls = commands.take_preexecuted_player_infos();
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0].1[0].resource.as_ref(), Some(&first_resource));
    assert_eq!(
        controls[1].1[0].resource.as_ref(),
        Some(&replacement_resource)
    );
    assert_eq!(
        app.control_player_infos.get(1).unwrap().resource.as_ref(),
        Some(&replacement_resource)
    );

    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: controls[0].0.clone(),
            info: controls[0].0.clone(),
            join_players_on_echo: controls[0].1.clone(),
        })
        .test_value();
    app.test_network_events();
    let first_joins = commands.take_submitted_join_players();
    assert_eq!(first_joins.len(), 1);
    assert_eq!(
        first_joins[0].1.source,
        clonk_engine::JoinPlayerSource::Resource(first_resource)
    );
    assert_eq!(
        app.control_player_infos.get(1).unwrap().resource.as_ref(),
        Some(&replacement_resource),
        "a stale add echo must not overwrite the later replacement row"
    );

    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: controls[1].0.clone(),
            info: controls[1].0.clone(),
            join_players_on_echo: controls[1].1.clone(),
        })
        .test_value();
    app.test_network_events();
    assert!(commands.take_submitted_join_players().is_empty());
}

#[test]
fn preexecuted_join_snapshot_is_client_scoped_for_duplicate_info_id() {
    let resource = |id, filename: &[u8]| clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(filename.to_vec()).test_value(),
        ..Default::default()
    };
    let wrong_resource = resource(17, b"Wrong.c4p");
    let target_resource = resource(18, b"Target.c4p");
    let wrong = clonk_engine::ControlPlayerInfoEntry {
        id: 7,
        name: clonk_engine::LegacyCString::from_bytes(b"Wrong".to_vec()).test_value(),
        resource: Some(wrong_resource),
        ..Default::default()
    };
    let target = clonk_engine::ControlPlayerInfoEntry {
        id: 7,
        name: clonk_engine::LegacyCString::from_bytes(b"Target".to_vec()).test_value(),
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        resource: Some(target_resource.clone()),
        ..Default::default()
    };

    let mut app = new_state_only_running_sandbox_app();
    app.control_clients.register(3, true, false);
    app.control_player_infos.replace_snapshot(
        7,
        [
            netplay_player_info_data(4, vec![wrong]),
            netplay_player_info_data(3, vec![target.clone()]),
        ],
    );
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));

    app.broadcast_and_preexecute_player_info(
        clonk_engine::PlayerInfoControlData {
            client_id: 3,
            players: vec![target],
            by_client: 0,
            ..Default::default()
        },
        false,
        true,
    )
    .test_value();

    let controls = commands.take_preexecuted_player_infos();
    let [(_, captured)] = controls.as_slice() else {
        panic!("expected one preexecuted target-client control");
    };
    let [captured] = captured.as_slice() else {
        panic!("expected one target-client join snapshot row");
    };
    assert_eq!(captured.player_type, clonk_engine::PLAYER_INFO_TYPE_SCRIPT);
    assert_eq!(captured.resource.as_ref(), Some(&target_resource));
}

#[test]
fn host_direct_remote_player_info_refreshes_join_data_for_later_consumers() {
    // Game.PlayerInfos aliases Game.Parameters.PlayerInfos, and SendJoinData
    // copies those live parameters. A runtime admission therefore has to be
    // present in the next client's JoinData after its direct control runs
    // (src/C4Game.cpp:65-71; src/C4Network2.cpp:1820-1844;
    // src/C4Network2Players.cpp:233-239).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 9,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Runtime Remote".to_vec())
                        .expect("valid player name"),
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .test_value();
    app.test_network_events();
    let (broadcasts, published) = commands.take_team_control_updates();
    let [authoritative] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo control");
    };
    assert_eq!(authoritative.players[0].id, 1);
    let latest = published.last().test_value();
    assert_eq!(published.len(), 1);
    assert_eq!(latest.parameters.player_infos.last_player_id, 1);
    let [client] = latest.parameters.player_infos.clients.as_slice() else {
        panic!("runtime client row is retained in JoinData");
    };
    assert_eq!((client.client_id, client.flags), (3, 0));
    let [player] = client.players.as_slice() else {
        panic!("runtime player row is retained in JoinData");
    };
    assert_eq!(player.id, 1);
    assert_eq!(player.name.as_bytes(), b"Runtime Remote");
    assert_eq!(published.last(), app.host_join_snapshot.as_ref());
}

#[test]
fn host_synchronized_player_info_refreshes_join_data() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    let authoritative = app
        .control_player_infos
        .admit_request(
            clonk_network::PlayerInfoUpdateRequest {
                client_id: 4,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
            },
            app.network_max_players,
        )
        .test_value();

    app.apply_ready_controls(7, vec![NetworkControl::PlayerInfo(authoritative)])
        .test_value();

    let published = commands.take_published_join_snapshots();
    let latest = published.last().test_value();
    assert_eq!(latest.parameters.player_infos.last_player_id, 1);
    assert_eq!(latest.parameters.player_infos.clients[0].client_id, 4);
    assert_eq!(latest.parameters.player_infos.clients[0].players[0].id, 1);
    assert_eq!(published.last(), app.host_join_snapshot.as_ref());
}

#[test]
fn client_direct_player_info_does_not_rebalance_random_teams_or_echo_updates() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let mut metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10, 20], 0),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.control_player_infos.replace_snapshot(
        30,
        [netplay_player_info_data(
            3,
            vec![
                set_control_test_player(10, 1, 0),
                set_control_test_player(20, 1, 0),
            ],
        )],
    );

    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![
                    set_control_test_player(10, 1, 0),
                    set_control_test_player(20, 1, 0),
                    set_control_test_player(30, 1, 0),
                ],
                by_client: 0,
                ..Default::default()
            },
        )))
        .test_value();
    app.test_network_events();

    let teams = app.network_team_assignment.test_ref().teams();
    assert_eq!(teams.teams[0].player_ids, vec![10, 20, 30]);
    assert!(teams.teams[1].player_ids.is_empty());
    assert_eq!(app.control_player_infos.get(10).unwrap().team, 1);
    assert!(commands.take_broadcast_player_infos().is_empty());
}

#[test]
fn host_remote_player_info_assigns_the_unique_least_used_runtime_team() {
    // HandlePlayerInfoUpdRequest allocates the ID before AssignTeams, and
    // the host broadcasts that already-adjusted PlayerInfo. AddPlayer also
    // records the ID and forces the current team color
    // (src/C4Network2Players.cpp:160-205;
    // src/C4Teams.cpp:53-81,474-542).
    let team = |id, player_ids, color| clonk_engine::InitialNetworkTeam {
        id,
        name: clonk_engine::LegacyCString::from_bytes(format!("Team {id}").into_bytes())
            .test_value(),
        player_start_index: 0,
        player_ids,
        color,
        icon_spec: clonk_engine::LegacyCString::default(),
        max_players: 0,
    };
    let mut app = new_state_only_menu_app(320, 200);
    app.control_player_infos.replace_snapshot(
        1,
        [netplay_player_info_data(
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: clonk_engine::LegacyCString::from_bytes(b"Existing".to_vec())
                    .expect("valid existing player name"),
                team: 1,
                ..Default::default()
            }],
        )],
    );
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
        clonk_engine::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: clonk_engine::InitialNetworkTeamDistribution::Random,
            team_colors: true,
            max_script_players: 0,
            script_player_names: clonk_engine::LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![1], 0x00f4_0000),
                team(2, Vec::new(), 0x0000_c800),
            ],
        },
    ));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let original_color = 0x0012_3456;
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 9,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"New".to_vec())
                        .expect("valid new player name"),
                    color: original_color,
                    original_color,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .test_value();

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [info] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo broadcast");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted player");
    };
    assert_eq!((player.id, player.team), (2, 2));
    assert_eq!(
        (player.color, player.original_color),
        (0x0000_c800, original_color)
    );
    let teams = app.network_team_assignment.test_mut().teams_mut();
    assert_eq!(teams.teams[0].player_ids, vec![1]);
    assert_eq!(teams.teams[1].player_ids, vec![2]);
}

#[test]
fn host_authored_script_player_info_assigns_a_runtime_host_team() {
    // Host-authored requests still execute AssignTeams. A script player
    // cannot choose later at runtime, so TEAMDIST_Host assigns the least-
    // used existing team even after the lobby has ended
    // (src/C4Network2Players.cpp:146-153,189-205;
    // src/C4Teams.cpp:474-542).
    let team = |id, player_ids, color| clonk_engine::InitialNetworkTeam {
        id,
        name: clonk_engine::LegacyCString::from_bytes(format!("Team {id}").into_bytes())
            .test_value(),
        player_start_index: 0,
        player_ids,
        color,
        icon_spec: clonk_engine::LegacyCString::default(),
        max_players: 0,
    };
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos.replace_snapshot(
        1,
        [netplay_player_info_data(
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: clonk_engine::LegacyCString::from_bytes(b"Existing".to_vec())
                    .expect("valid existing player name"),
                team: 1,
                ..Default::default()
            }],
        )],
    );
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
        clonk_engine::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: clonk_engine::InitialNetworkTeamDistribution::Host,
            team_colors: true,
            max_script_players: 0,
            script_player_names: clonk_engine::LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![1], 0x00f4_0000),
                team(2, Vec::new(), 0x0000_c800),
            ],
        },
    ));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 0,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Script".to_vec())
                        .expect("valid script player name"),
                    player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                    ..Default::default()
                }],
            },
            by_host: true,
        })
        .test_value();

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [info] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo broadcast");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted script player");
    };
    assert_eq!((player.id, player.team), (2, 2));
    assert_eq!(player.color, 0x0000_c800);
    let teams = app.network_team_assignment.test_mut().teams_mut();
    assert_eq!(teams.teams[0].player_ids, vec![1]);
    assert_eq!(teams.teams[1].player_ids, vec![2]);
}

#[test]
fn host_player_info_request_uses_active_network_player_limit() {
    // AssignPlayerIDs computes free slots from
    // Game.Parameters.MaxPlayers, not the scenario format default
    // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:781-807;
    // src/C4Network2Players.cpp:160-194).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_max_players = 1;
    app.control_player_infos.replace_snapshot(
        1,
        [netplay_player_info_data(
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
        )],
    );
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 9,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
            },
            by_host: false,
        })
        .test_value();

    app.test_network_events();

    assert!(commands.take_broadcast_player_infos().is_empty());
}

#[test]
fn running_host_direct_script_info_queues_one_synchronized_join() {
    // On a running host, direct PlayerInfo executes first and then
    // JoinUnjoinedPlayersInControlQueue appends one script JoinPlayer to
    // the next control input (src/C4Network2Players.cpp:245-269,353-388).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let tick = app.local_control_submission_tick();
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 41,
                    player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                    ..Default::default()
                }],
                by_client: 0,
                ..Default::default()
            },
        )))
        .test_value();

    app.test_network_events();

    assert_eq!(
        commands.take_submitted_join_players(),
        vec![(
            tick,
            clonk_engine::JoinPlayerControlData {
                at_client: 0,
                info_id: 41,
                source: clonk_engine::JoinPlayerSource::Embedded(Vec::new()),
                by_client: 0,
                ..Default::default()
            },
        )]
    );
}

#[test]
fn offline_create_script_player_joins_through_player_info_control_path() {
    let mut app = new_state_only_running_sandbox_app();
    let existing_player_info_id = app.engine.test_player(app.local_owner).player_info_id();
    assert!(existing_player_info_id > 0);
    app.engine
        .install_scenario_script_with_convention(
            "CreateScriptPlayer fixture",
            r#"
            global func SpawnBot()
            {
                return CreateScriptPlayer("Bot", 0x445566, 2, 15, __AI);
            }
            "#,
            true,
        )
        .test_value();
    let before = app.engine.snapshot().players.len();
    app.engine
        .call_scenario_script_function("SpawnBot", Vec::new())
        .test_value();
    assert_eq!(
        app.engine.snapshot().players.len(),
        before,
        "CreateScriptPlayer must not join synchronously inside the VM call"
    );

    app.handle_script_player_info_updates().test_value();
    app.handle_script_player_info_updates().test_value();

    let infos = app
        .control_player_infos
        .client_info_ids(0)
        .into_iter()
        .filter_map(|id| app.control_player_infos.get(id))
        .filter(|info| info.name.as_bytes() == b"Bot")
        .collect::<Vec<_>>();
    assert_eq!(infos.len(), 1, "script PlayerInfo is admitted exactly once");
    let info = infos[0];
    assert_eq!(
        info.id,
        existing_player_info_id + 1,
        "PlayerInfo admission reserves IDs already owned by live low-level players"
    );
    assert_eq!(info.player_type, clonk_engine::PLAYER_INFO_TYPE_SCRIPT);
    assert_eq!(
        (info.color, info.original_color, info.team),
        (0x445566, 0x445566, 2)
    );
    assert_eq!(info.extra_data, *b"__AI");
    assert_eq!(
        info.flags
            & (clonk_engine::PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
                | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                | clonk_engine::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
                | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE),
        clonk_engine::PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT
            | clonk_engine::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
            | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE
    );
    let runtime = app
        .engine
        .snapshot()
        .players
        .into_iter()
        .find(|player| player.player_info_id == info.id)
        .test_value();
    assert_eq!(runtime.name, "Bot");
    assert_eq!(runtime.team, Some(2));
    assert_eq!(
        runtime.color,
        Some(clonk_engine::RgbColor::new(0x44, 0x55, 0x66))
    );
    assert!(
        app.snapshot
            .players
            .iter()
            .any(|player| player.player_info_id == info.id),
        "post-control app snapshot includes the joined player"
    );
}

#[test]
fn client_direct_cpp_sync_check_desync_continues_running_round_locally() {
    // An inactive C++ host sends CID_SyncCheck through
    // PID_ControlPkt/CDT_Direct, which HandleControlPkt executes at once
    // (src/C4GameControl.cpp:439-450;
    // src/C4GameControlNetwork.cpp:558-565). A mismatch clears C4Network2,
    // which invokes ChangeToLocal without aborting the round
    // (src/C4Control.cpp:469-519; src/C4Network2.cpp:746-789;
    // src/C4GameControl.cpp:93-127). Live frame-100 fixture:
    // ff 1a 00 00 00 42 02 85 64 00 32 00 6d 03 00 00 74 80 01 00
    // 00 00 00 00 98 01 99 01 56 02 00.
    let mut app = new_state_only_running_sandbox_app();
    let sound_dir = tempdir();
    let sound_scenario = sound_dir.path().join("DesyncAudio.c4s");
    fs::create_dir_all(&sound_scenario).test_value();
    fs::write(sound_scenario.join("SyncError.wav"), silent_pcm_wav(100)).test_value();
    let audio = app.audio.test_mut();
    audio.options.sound_enabled = true;
    audio.configure_scenario(Some(&sound_scenario));
    assert!(app.snapshot.audio.is_empty());
    app.ui_sound_log.clear();
    let local_player = app.local_owner;
    let local_client = 1;
    let remote_player = 17;
    app.engine
        .test_player_mut(local_player)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.engine
        .register_player(PlayerConfig::new(remote_player, "Host player"))
        .test_value();
    app.engine
        .test_player_mut(remote_player)
        .set_at_client(clonk_engine::PlayerAtClient::HOST);
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(37, 4).test_value(),
    );
    app.snapshot = app.engine.snapshot();
    let (manager, event_tx, _commands) =
        NetworkManager::test_stub_with_commands_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_control_clock = Some(NetworkControlClock::new(37, 4));
    app.control_clients = ControlClientRegistry::default();
    app.control_clients.register(0, true, false);
    app.control_clients.register(local_client, true, false);
    let remote = SyncCheckPacket {
        frame: 100,
        control_tick: 50,
        random3: 0,
        random_count: 877,
        crew_positions_sum: 16_500,
        pxs_count: 0,
        mass_mover_index: 0,
        object_count: 152,
        object_enumeration_index: 153,
        sector_shape_sum: 342,
        by_client: 0,
    };
    let mut local = remote.clone();
    local.random_count -= 1;
    app.sync_checks.record_local(local);
    let frame_before = app.engine.frame();
    let control_tick_before = app.engine.sync_check(local_client).control_tick;
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::SyncCheck(
            remote,
        )))
        .test_value();

    app.test_network_events();

    assert!(matches!(app.mode, AppMode::Running));
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert_eq!(app.engine.frame(), frame_before);
    assert_eq!(
        app.engine.sync_check(local_client).control_tick,
        control_tick_before
    );
    assert_eq!(app.engine.control_rate, 1);
    assert!(app.engine.player(local_player).is_some());
    assert!(app.engine.player(remote_player).is_none());
    assert_eq!(
        app.status_text,
        "Network desync detected; disconnected from host"
    );
    assert_eq!(
        app.engine.snapshot().round_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        app.engine
            .snapshot()
            .round_results
            .network_result_message
            .as_slice(),
        &b"Network: Synchronization loss!"[..]
    );
    assert_eq!(
        app.snapshot.round_results,
        app.engine.snapshot().round_results,
        "the presentation snapshot exposes the verdict immediately"
    );
    assert!(
        app.audio
            .as_ref()
            .expect("sandbox audio context remains")
            .loaded_sounds
            .keys()
            .any(|key| key.to_ascii_lowercase().contains("syncerror.wav")),
        "SyncError is eagerly decoded and played immediately"
    );
    assert_eq!(
        app.ui_sound_log
            .iter()
            .filter(|sound| sound.as_str() == "CloseViewport")
            .count(),
        0,
        "the normally removed remote host has no local physical viewport to close"
    );
    assert!(
        app.snapshot.audio.is_empty(),
        "the immediate UI cue is not replayed through engine audio"
    );
}

#[test]
fn host_ignores_remote_sync_check_without_desync_side_effects() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    let status_before = app.status_text.clone();
    let local = app.engine.sync_check(0);
    let mut remote = local.clone();
    remote.random_count = remote.random_count.wrapping_add(1);
    remote.by_client = 7;
    app.sync_checks.record_local(local);
    events
        .send(NetworkEvent::DirectControl(NetworkControl::SyncCheck(
            remote,
        )))
        .test_value();

    app.test_network_events();

    assert!(app.network.is_some());
    assert!(matches!(app.network_mode, Some(NetworkMode::Host(_))));
    assert_eq!(app.status_text, status_before);
    assert!(app.engine.snapshot().round_results.network_result.is_none());
    assert!(app.snapshot.round_results.network_result.is_none());
    assert!(app.sync_checks.remote.is_empty());
}

#[test]
fn synchronized_raw_player_control_counts_before_byte_narrowing() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;

    app.apply_ready_controls(
        12,
        vec![NetworkControl::PlayerControl(
            clonk_engine::PlayerControlData {
                player: owner,
                // CountControl observes 273, while InCom receives its
                // narrowed byte (17, a release command).
                command: 273,
                data: 4,
                by_client: 0,
            },
        )],
    )
    .test_value();

    let player = app.engine.test_player(owner);
    assert_eq!((player.control_count(), player.action_count()), (1, 1));
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn synchronized_player_command_executes_stack_data_and_count() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    let crew = app.engine.test_crew_cursor(owner);

    app.apply_ready_controls(
        12,
        vec![NetworkControl::PlayerCommand(PlayerCommandControlData {
            player: owner,
            command: CommandId::Wait as i32,
            x: 12,
            y: -7,
            target: 999_999,
            target2: 0,
            data: 23,
            add_mode: 1,
            by_client: 0,
        })],
    )
    .test_value();

    let commands = app
        .engine
        .test_object_snapshot(crew)
        .command_stack
        .command_views();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "Wait");
    assert_eq!(commands[0].target, None);
    assert_eq!(commands[0].tx, Some(12));
    assert_eq!(commands[0].ty, Some(-7));
    assert_eq!(commands[0].data, CommandData::Integer(23));
    let player = app.engine.test_player(owner);
    assert_eq!((player.control_count(), player.action_count()), (1, 1));
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn synchronized_custom_command_executes_only_while_game_is_running() {
    let mut app = new_state_only_running_sandbox_app();
    let command = NetworkControl::CustomCommand(clonk_engine::CustomCommandControlData {
        command: clonk_engine::LegacyCString::from_bytes(b"speed".to_vec()).test_value(),
        argument: clonk_engine::LegacyCString::from_bytes(b"100".to_vec()).test_value(),
        player: -1,
        by_client: 91,
    });
    let initial_delay = app.engine.game_tick_delay_ms();

    app.mode = AppMode::Loading;
    app.apply_ready_controls(11, vec![command.clone()])
        .test_value();
    assert_eq!(app.engine.game_tick_delay_ms(), initial_delay);

    app.mode = AppMode::Running;
    app.apply_ready_controls(12, vec![command]).test_value();
    assert_eq!(app.engine.game_tick_delay_ms(), 10);

    app.engine.set_debug_mode(true);
    let disabled_speed = NetworkControl::CustomCommand(clonk_engine::CustomCommandControlData {
        command: clonk_engine::LegacyCString::from_bytes(b"speed".to_vec()).test_value(),
        argument: clonk_engine::LegacyCString::from_bytes(b"50".to_vec()).test_value(),
        player: -1,
        by_client: 91,
    });
    app.apply_ready_controls(
        13,
        vec![
            NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type: 1,
                data: 0,
                by_client: 7,
            }),
            disabled_speed,
        ],
    )
    .test_value();
    assert_eq!(
        app.engine.game_tick_delay_ms(),
        10,
        "DisableDebug removes /speed before the following packet executes"
    );
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn synchronized_goal_menu_evaluates_before_opening_only_for_local_player() {
    let mut app = new_running_sandbox_app();
    let player = app.local_owner;
    let mut goal = test_definition(
        "IGOL",
        "Integrated Goal",
        "#strict 3\nfunc IsFulfilled() { return true; }",
    );
    goal.set_category(C4D_GOAL);
    goal.set_description(Some("Reach the target".to_string()));
    app.engine.register_test_definition(goal);
    app.engine
        .spawn_test_object(clonk_engine::SpawnConfig::new("IGOL"));
    let by_client = app.engine.test_player(player).at_client().get();
    let control =
        NetworkControl::ActivateGameGoalMenu(clonk_engine::ActivateGameGoalMenuControlData {
            player,
            by_client,
        });

    app.engine.set_local_players([player]);
    app.apply_ready_controls(12, vec![control.clone()])
        .test_value();
    let menu = app.ingame_menu.get(player).test_value();
    assert_eq!(menu.page(), ingame_menu::MenuPage::Goals);
    assert_eq!(menu.items().len(), 1);
    assert_eq!(
        menu.items()[0].action,
        MenuAction::GoalInfo("IGOL".to_string())
    );
    assert_eq!(
        menu.items()[0].info_caption.as_deref(),
        Some("Reach the target")
    );

    app.ingame_menu.replace(player, None);
    app.engine.set_local_players([]);
    app.apply_ready_controls(13, vec![control]).test_value();
    assert!(app.ingame_menu.get(player).is_none());
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn synchronized_message_board_answer_executes_only_for_the_owning_client() {
    let mut app = new_state_only_running_sandbox_app();
    let player = app.local_owner;
    app.engine
        .test_player_mut(player)
        .set_at_client(clonk_engine::PlayerAtClient::HOST);
    app.engine.register_test_definition(test_definition(
        "MBAT",
        "Message-board answer target",
        r#"#strict 2
        local callback_answer, callback_count;
        public func Open(int player) { return CallMessageBoard(this(), false, "answer", player); }
        protected func InputCallback(string answer, int player)
        {
            callback_answer = answer;
            callback_count = callback_count + 1;
            return 1;
        }
        "#,
    ));
    let target = app.engine.spawn_test_object(SpawnConfig::new("MBAT"));
    let target_index = app.engine.find_object_index(target).test_value();
    assert_eq!(
        app.engine
            .call_object_function(target_index, "Open", vec![Value::Int(player)])
            .expect("query opens"),
        Value::Bool(true)
    );
    for frame in 1..=35 {
        app.engine
            .tick()
            .unwrap_or_else(|error| panic!("query activation tick {frame} succeeds: {error}"));
    }
    assert!(app.engine.active_message_board_input().is_some());
    let object = i32::try_from(target.as_u64()).test_value();

    app.apply_ready_controls(
        11,
        vec![NetworkControl::MessageBoardAnswer(
            clonk_engine::MessageBoardAnswerControlData {
                object,
                answer: clonk_engine::LegacyCString::from_bytes(b"forged".to_vec())
                    .expect("answer is NUL-free"),
                player,
                by_client: 7,
            },
        )],
    )
    .test_value();
    assert_ne!(
        app.engine
            .object_snapshot(target)
            .expect("target remains live")
            .local_vars
            .get("callback_count"),
        Some(&Value::Int(1)),
        "the forged answer must not invoke InputCallback"
    );
    assert!(
        app.engine.active_message_board_input().is_some(),
        "synchronized execution does not own local dialog closure"
    );

    let answer = app
        .engine
        .prepare_message_board_answer_control(
            clonk_engine::LegacyCString::from_bytes(b"q\"\\z".to_vec())
                .expect("answer is NUL-free"),
            -1,
        )
        .test_value();
    assert!(app.engine.active_message_board_input().is_none());
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network
        .test_ref()
        .submit_message_board_answer(12, answer)
        .test_value();
    assert_ne!(
        app.engine
            .object_snapshot(target)
            .expect("target remains live")
            .local_vars
            .get("callback_count"),
        Some(&Value::Int(1)),
        "submission closes the input but does not run the callback"
    );
    let (_, answer) = commands
        .take_submitted_message_board_answers()
        .pop()
        .test_value();
    assert_eq!(answer.by_client, 0, "manager stamps the local client ID");

    app.apply_ready_controls(12, vec![NetworkControl::MessageBoardAnswer(answer)])
        .test_value();
    let target = app.engine.test_object_snapshot(target);
    assert_eq!(
        target.local_vars.get("callback_answer"),
        Some(&Value::String("q\"\\z".to_string().into()))
    );
    assert_eq!(
        target.local_vars.get("callback_count"),
        Some(&Value::Int(1))
    );
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn host_control_rate_set_changes_both_clocks_and_keeps_batch_order() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_control_clock = Some(NetworkControlClock::new(37, 4));
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(37, 4).test_value(),
    );
    let initial_pressed = app.engine.test_player(app.local_owner).control.pressed_coms;

    app.apply_ready_controls(
        37,
        vec![
            NetworkControl::Player {
                owner: app.local_owner,
                event: ControlEvent::Press(ControlButton::Right),
            },
            NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type: 0,
                data: 1,
                by_client: 0,
            }),
        ],
    )
    .test_value();

    assert_ne!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms,
        initial_pressed
    );
    assert_eq!(app.engine.control_rate(), 5);
    assert_eq!(
        app.network_control_clock
            .map(NetworkControlClock::control_rate),
        Some(5)
    );
    assert_eq!(
        app.network_control_clock
            .and_then(|clock| clock.tick_for_frame(4)),
        None
    );
    assert_eq!(
        app.network_control_clock
            .and_then(|clock| clock.tick_for_frame(5)),
        Some(37)
    );

    for (delta, expected) in [(i32::MAX, 20), (i32::MIN, 1)] {
        app.apply_ready_controls(
            38,
            vec![NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type: 0,
                data: delta,
                by_client: 0,
            })],
        )
        .test_value();
        assert_eq!(app.engine.control_rate(), expected);
        assert_eq!(
            app.network_control_clock
                .map(NetworkControlClock::control_rate),
            Some(expected)
        );
    }
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn host_parameter_sets_update_live_game_state_without_boundaries() {
    let mut app = new_state_only_running_sandbox_app();
    for (value_type, data) in [(2, 37), (3, 4), (4, -1), (5, 777)] {
        app.apply_ready_controls(
            9,
            vec![NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type,
                data,
                by_client: 0,
            })],
        )
        .unwrap_or_else(|error| panic!("Set type {value_type} executes: {error}"));
    }

    assert_eq!(app.network_max_players, 37);
    assert_eq!(app.engine.max_players(), Some(37));
    assert_eq!(app.engine.team_distribution(), 4);
    assert!(app.engine.team_colors());
    assert!(app.engine.use_fair_crew());
    assert_eq!(app.engine.fair_crew_strength(), 777);

    for (value_type, data) in [(3, 9), (4, 0)] {
        app.apply_ready_controls(
            9,
            vec![NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type,
                data,
                by_client: 0,
            })],
        )
        .test_value();
    }
    assert_eq!(app.engine.team_distribution(), 4);
    assert!(!app.engine.team_colors());

    app.apply_ready_controls(
        10,
        vec![NetworkControl::Set(clonk_network::LegacyControlSet {
            value_type: 5,
            data: -1,
            by_client: 0,
        })],
    )
    .test_value();
    assert!(!app.engine.use_fair_crew());
    assert_eq!(app.engine.fair_crew_strength(), 0);

    app.engine.set_fair_crew_forced(true);
    app.apply_ready_controls(
        11,
        vec![NetworkControl::Set(clonk_network::LegacyControlSet {
            value_type: 5,
            data: 999,
            by_client: 0,
        })],
    )
    .test_value();
    assert!(!app.engine.use_fair_crew());
    assert_eq!(app.engine.fair_crew_strength(), 0);

    app.network_is_league = true;
    app.apply_ready_controls(
        12,
        vec![NetworkControl::Set(clonk_network::LegacyControlSet {
            value_type: 2,
            data: 99,
            by_client: 0,
        })],
    )
    .test_value();
    assert_eq!(app.engine.max_players(), Some(37));

    for value_type in [-1, 6, i32::MAX] {
        app.apply_ready_controls(
            13,
            vec![NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type,
                data: 123,
                by_client: 0,
            })],
        )
        .test_value();
    }
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn network_host_distribution_reassigns_and_publishes_full_team_state() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    let metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10, 20], 1),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.control_player_infos.replace_snapshot(
        20,
        [netplay_player_info_data(
            3,
            vec![
                set_control_test_player(20, 1, 0),
                set_control_test_player(10, 1, clonk_engine::PLAYER_INFO_FLAG_JOINED),
            ],
        )],
    );

    app.execute_control_set(clonk_network::LegacyControlSet {
        value_type: 3,
        data: 2,
        by_client: 0,
    });

    assert_eq!(app.control_player_infos.get(20).unwrap().team, 2);
    assert_eq!(app.engine.team_distribution(), 2);
    let assignment = app.network_team_assignment.as_ref().test_value().teams();
    assert_eq!(assignment.teams[0].player_ids, vec![10]);
    assert_eq!(assignment.teams[1].player_ids, vec![20]);
    let snapshot = app.host_join_snapshot.as_ref().test_value();
    assert_eq!(snapshot.parameters.teams.teams[0].player_ids, vec![10]);
    assert_eq!(snapshot.parameters.teams.teams[1].player_ids, vec![20]);
    assert_eq!(
        snapshot.parameters.player_infos.clients[0].players[0].team,
        2
    );
    let (player_infos, snapshots) = commands.take_team_control_updates();
    assert_eq!(player_infos.len(), 1);
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0], *snapshot);
}

#[test]
fn generated_distribution_rebuilds_default_teams_and_publishes() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    let mut metadata = set_control_test_metadata(true, vec![set_control_test_team(1, vec![20], 0)]);
    metadata.random_team_count = 2;
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.control_player_infos.replace_snapshot(
        20,
        [netplay_player_info_data(
            3,
            vec![set_control_test_player(20, 1, 0)],
        )],
    );
    app.execute_control_set(clonk_network::LegacyControlSet {
        value_type: 3,
        data: 3,
        by_client: 0,
    });

    let assignment = app.network_team_assignment.as_ref().test_value().teams();
    assert_eq!(
        assignment.team_distribution,
        clonk_engine::InitialNetworkTeamDistribution::Random
    );
    assert_eq!(assignment.last_team_id, 2);
    assert_eq!(
        assignment
            .teams
            .iter()
            .map(|team| (team.id, team.name.as_bytes(), team.color))
            .collect::<Vec<_>>(),
        vec![
            (1, b"Team 1".as_slice(), 0x00f4_0000),
            (2, b"Team 2".as_slice(), 0x0000_c800),
        ]
    );
    let assigned_team = app.control_player_infos.get(20).test_value().team;
    assert!((1..=2).contains(&assigned_team));
    assert_eq!(
        assignment
            .teams
            .iter()
            .find(|team| team.id == assigned_team)
            .unwrap()
            .player_ids,
        vec![20]
    );
    assert_eq!(app.engine.team_distribution(), 3);
    let snapshot = app.host_join_snapshot.as_ref().test_value();
    assert_eq!(snapshot.parameters.teams.team_distribution, 3);
    assert_eq!(snapshot.parameters.teams.teams.len(), 2);
    let (player_infos, snapshots) = commands.take_team_control_updates();
    let [player_info] = player_infos.as_slice() else {
        panic!("expected one regenerated PlayerInfo packet");
    };
    assert_eq!(player_info.client_id, 3);
    assert_eq!(
        player_info.flags
            & (clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
                | clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED),
        0
    );
    assert_eq!(player_info.players[0].team, assigned_team);
    assert_eq!(snapshots, vec![snapshot.clone()]);
}

#[test]
fn invalid_host_team_distribution_is_a_silent_no_op() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    let metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10], 0),
            set_control_test_team(2, vec![20], 0),
        ],
    );
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.control_player_infos.replace_snapshot(
        20,
        [netplay_player_info_data(
            3,
            vec![set_control_test_player(20, 2, 0)],
        )],
    );
    app.status_text = "unchanged".to_string();
    let before_distribution = app.engine.team_distribution();
    let before_teams = app
        .network_team_assignment
        .as_ref()
        .test_value()
        .teams()
        .clone();
    let before_infos = app.control_player_infos.retained_rows_snapshot();
    let before_snapshot = app.host_join_snapshot.clone();

    app.execute_control_set(clonk_network::LegacyControlSet {
        value_type: 3,
        data: 9,
        by_client: 0,
    });

    assert_eq!(app.status_text, "unchanged");
    assert_eq!(app.engine.team_distribution(), before_distribution);
    assert_eq!(
        app.network_team_assignment.as_ref().unwrap().teams(),
        &before_teams
    );
    assert_eq!(
        app.control_player_infos.retained_rows_snapshot(),
        before_infos
    );
    assert_eq!(app.host_join_snapshot, before_snapshot);
    assert_eq!(
        commands.take_team_control_updates(),
        (Vec::new(), Vec::new())
    );
}

#[test]
fn network_host_team_colors_broadcasts_attributes_and_full_join_data() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    let metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10], 0),
            set_control_test_team(2, vec![20], 0),
        ],
    );
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let mut unjoined = set_control_test_player(20, 2, 0);
    unjoined.color = 0x0000_c800;
    unjoined.original_color = 0x0000_c800;
    let mut joined = set_control_test_player(10, 1, clonk_engine::PLAYER_INFO_FLAG_JOINED);
    joined.color = 0x00f4_0000;
    joined.original_color = 0x00f4_0000;
    app.control_player_infos
        .replace_snapshot(20, [netplay_player_info_data(3, vec![unjoined, joined])]);

    app.execute_control_set(clonk_network::LegacyControlSet {
        value_type: 4,
        data: 1,
        by_client: 0,
    });

    assert!(app.engine.team_colors());
    assert!(
        app.network_team_assignment
            .as_ref()
            .unwrap()
            .teams()
            .team_colors
    );
    assert_eq!(app.control_player_infos.get(20).unwrap().color, 0x0000_00f4);
    let snapshot = app.host_join_snapshot.as_ref().test_value();
    assert_eq!(snapshot.parameters.teams.team_colors, 1);
    assert_eq!(snapshot.parameters.teams.teams.len(), 2);
    assert_eq!(
        snapshot.parameters.player_infos.clients[0].players[0].color,
        0x0000_00f4
    );
    let (player_infos, snapshots) = commands.take_team_control_updates();
    assert_eq!(player_infos.len(), 1);
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0], *snapshot);
}

#[test]
fn wire_known_zero_color_and_name_conflicts_resolve_and_publish_team_colors() {
    for conflict_kind in ["color", "name"] {
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
        let metadata = set_control_test_metadata(
            false,
            vec![
                set_control_test_team(1, Vec::new(), 0),
                set_control_test_team(2, Vec::new(), 0),
            ],
        );
        app.network_team_assignment =
            Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
        let mut first = set_control_test_player(20, 0, 0);
        let mut second = set_control_test_player(21, 0, 0);
        if conflict_kind == "color" {
            first.color = 0x00f4_0000;
            first.original_color = 0x00f4_0000;
            second.color = 0x00f4_0000;
            second.original_color = 0x00f4_0000;
        } else {
            first.color = 0x00f4_0000;
            first.original_color = 0x00f4_0000;
            second.color = 0x0000_00f4;
            second.original_color = 0x0000_00f4;
            first.name = clonk_engine::LegacyCString::from_bytes(b"Same".to_vec()).test_value();
            second.name = clonk_engine::LegacyCString::from_bytes(b"same".to_vec()).test_value();
        }
        app.control_player_infos
            .replace_snapshot(21, [netplay_player_info_data(3, vec![first, second])]);
        app.execute_control_set(clonk_network::LegacyControlSet {
            value_type: 4,
            data: 1,
            by_client: 0,
        });

        assert!(app.engine.team_colors(), "{conflict_kind}");
        assert!(
            app.network_team_assignment
                .as_ref()
                .unwrap()
                .teams()
                .team_colors,
            "{conflict_kind}"
        );
        let first = app.control_player_infos.get(20).test_value();
        let second = app.control_player_infos.get(21).test_value();
        if conflict_kind == "color" {
            assert_ne!(first.color, second.color);
        } else {
            fn effective_name(player: &clonk_engine::ControlPlayerInfoEntry) -> &[u8] {
                if player.forced_name.is_empty() {
                    player.name.as_bytes()
                } else {
                    player.forced_name.as_bytes()
                }
            }
            assert!(!effective_name(first).eq_ignore_ascii_case(effective_name(second)));
        }
        let snapshot = app.host_join_snapshot.as_ref().test_value();
        assert_eq!(snapshot.parameters.teams.team_colors, 1);
        let (player_infos, snapshots) = commands.take_team_control_updates();
        assert_eq!(player_infos.len(), 1, "{conflict_kind}");
        assert_eq!(
            snapshots.as_slice(),
            std::slice::from_ref(snapshot),
            "{conflict_kind}"
        );
    }
}

#[test]
fn offline_host_reassigns_but_replay_only_changes_the_distribution_flag() {
    let metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10, 20], 1),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    let packet = netplay_player_info_data(
        0,
        vec![
            set_control_test_player(20, 1, 0),
            set_control_test_player(10, 1, clonk_engine::PLAYER_INFO_FLAG_JOINED),
        ],
    );

    let mut offline = new_state_only_running_sandbox_app();
    offline.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
        metadata.clone(),
    ));
    offline
        .control_player_infos
        .replace_snapshot(20, [packet.clone()]);
    offline.execute_control_set(clonk_network::LegacyControlSet {
        value_type: 3,
        data: 2,
        by_client: 0,
    });
    assert_eq!(offline.control_player_infos.get(20).unwrap().team, 2);

    let mut replay = new_state_only_running_sandbox_app();
    replay.engine.set_control_host(false);
    replay.network_team_assignment = None;
    replay.control_player_infos.replace_snapshot(20, [packet]);
    replay.execute_control_set(clonk_network::LegacyControlSet {
        value_type: 3,
        data: 3,
        by_client: 0,
    });
    assert_eq!(replay.engine.team_distribution(), 3);
    assert_eq!(replay.control_player_infos.get(20).unwrap().team, 1);
}

#[test]
fn client_player_info_rechecks_prepared_team_memberships_before_activation() {
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    let (_sender, receiver) = mpsc::channel();
    app.loading_state = Some(test_loading_state(
        FrontendScenario::fallback(),
        receiver,
        false,
        test_prepared_go(
            0,
            false,
            false,
            false,
            Vec::new(),
            Vec::new(),
            vec![
                clonk_engine::TeamInfo::new(1, "One", 0).with_player_ids(vec![20]),
                clonk_engine::TeamInfo::new(2, "Two", 0),
            ],
        ),
    ));
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            netplay_player_info_data(3, vec![set_control_test_player(20, 2, 0)]),
        )))
        .test_value();

    app.process_network_events().test_value();

    let teams = &app
        .loading_state
        .as_ref()
        .unwrap()
        .prepared_go
        .as_ref()
        .test_value()
        .team_registry;
    assert!(teams[0].player_ids.is_empty());
    assert_eq!(teams[1].player_ids, vec![20]);
}

#[test]
fn client_player_info_generates_missing_autogenerated_teams_before_activation() {
    // C4PlayerInfoList::AddInfo generates every nonzero team named by the
    // incoming packet before merging it into Game.PlayerInfos
    // (src/C4PlayerInfo.cpp:849-855).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    let mut metadata = set_control_test_metadata(true, Vec::new());
    metadata.custom = true;
    snapshot.parameters.teams = clonk_network::join_team_list_snapshot(metadata);
    event_tx
        .send(NetworkEvent::JoinData(clonk_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host_config.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        }))
        .test_value();

    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            netplay_player_info_data(3, vec![set_control_test_player(20, 4, 0)]),
        )))
        .test_value();

    app.process_network_events().test_value();

    let teams = &app
        .pending_network_join_data
        .test_ref()
        .parameters
        .teams
        .teams;
    assert_eq!(
        teams.iter().map(|team| team.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(teams[..3].iter().all(|team| team.player_ids.is_empty()));
    assert_eq!(teams[3].player_ids, vec![20]);
}

#[test]
fn offline_player_info_control_rechecks_teams_before_joining_unjoined_script_player() {
    let mut app = new_state_only_synthetic_crew_running_sandbox_app();
    let existing_info_id = app.engine.test_player(app.local_owner).player_info_id();
    let bot_info_id = existing_info_id + 1;
    let metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![existing_info_id, bot_info_id], 0),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));

    let existing =
        set_control_test_player(existing_info_id, 1, clonk_engine::PLAYER_INFO_FLAG_JOINED);
    let mut bot = set_control_test_player(bot_info_id, 1, 0);
    bot.player_type = clonk_engine::PLAYER_INFO_TYPE_SCRIPT;
    app.control_player_infos.replace_snapshot(
        bot_info_id,
        [netplay_player_info_data(0, vec![existing.clone(), bot])],
    );

    let mut moved_bot = set_control_test_player(bot_info_id, 2, 0);
    moved_bot.player_type = clonk_engine::PLAYER_INFO_TYPE_SCRIPT;
    app.apply_ready_controls(
        7,
        vec![NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![existing, moved_bot],
                ..Default::default()
            },
        )],
    )
    .test_value();

    let teams = app.network_team_assignment.test_ref().teams();
    assert_eq!(teams.teams[0].player_ids, vec![existing_info_id]);
    assert_eq!(teams.teams[1].player_ids, vec![bot_info_id]);
    let bot = app
        .engine
        .players()
        .find(|player| player.player_info_id() == bot_info_id)
        .test_value();
    assert_eq!(bot.team(), Some(2));
}

#[test]
fn mutable_set_parameters_round_trip_and_legacy_absence_preserves_bootstrap() {
    let mut engine = Engine::new();
    engine.set_fair_crew_forced(true);
    engine.set_allow_debug(false);
    engine.set_control_rate(7);
    let state = engine.capture_state();

    let mut restored = Engine::new();
    restored.restore_state(&state).test_value();
    assert!(restored.fair_crew_forced());
    assert!(!restored.allow_debug());
    assert_eq!(restored.control_rate(), 7);

    let mut legacy = state;
    legacy.fair_crew_forced = None;
    legacy.allow_debug = None;
    legacy.control_rate = None;
    let mut seeded = Engine::new();
    seeded.set_fair_crew_forced(true);
    seeded.set_allow_debug(false);
    seeded.set_control_rate(9);
    seeded.restore_state(&legacy).test_value();
    assert!(seeded.fair_crew_forced());
    assert!(!seeded.allow_debug());
    assert_eq!(seeded.control_rate(), 9);
}

#[test]
fn non_host_host_gated_control_sets_are_synchronized_no_ops() {
    for value_type in [0, 2, 3, 4, 5] {
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.network_control_clock = Some(NetworkControlClock::new(0, 3));
        app.engine.set_control_rate(3);
        app.control_clients.register(3, false, false);
        let before = (
            app.engine.control_rate(),
            app.network_control_clock
                .map(NetworkControlClock::control_rate),
            app.network_max_players,
            app.engine.max_players(),
            app.engine.team_distribution(),
            app.engine.team_colors(),
            app.engine.use_fair_crew(),
            app.engine.fair_crew_strength(),
        );

        app.apply_ready_controls(
            4,
            vec![
                NetworkControl::Set(clonk_network::LegacyControlSet {
                    value_type,
                    data: 99,
                    by_client: 7,
                }),
                NetworkControl::ClientUpdate(clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    client_id: 3,
                    data: 1,
                    by_client: 0,
                }),
            ],
        )
        .unwrap_or_else(|error| {
            panic!("non-host Set type {value_type} must be a C++ no-op: {error}")
        });

        assert!(app.control_clients.is_activated(3));
        assert_eq!(
            (
                app.engine.control_rate(),
                app.network_control_clock
                    .map(NetworkControlClock::control_rate),
                app.network_max_players,
                app.engine.max_players(),
                app.engine.team_distribution(),
                app.engine.team_colors(),
                app.engine.use_fair_crew(),
                app.engine.fair_crew_strength(),
            ),
            before,
            "non-host Set type {value_type} must not mutate game parameters"
        );
        assert!(app.runtime_flash_message.is_none());
        assert_eq!(app.executing_ready_tick, None);
    }
}

#[test]
fn disable_debug_set_executes_for_every_author_and_does_not_preflight_batch() {
    for by_client in [0, 7] {
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.engine.set_debug_mode(true);
        app.engine.set_allow_debug(true);
        app.graphics
            .set_debug_draw_flags(clonk_frontend::DebugDrawFlags {
                show_vertices: true,
                show_entrance: true,
                show_action: true,
                show_command: true,
                show_pathfinder: true,
                show_solid_mask: true,
                show_net_status: true,
            });
        let initial_pressed = app.engine.test_player(app.local_owner).control.pressed_coms;
        app.apply_ready_controls(
            5,
            vec![
                NetworkControl::Player {
                    owner: app.local_owner,
                    event: ControlEvent::Press(ControlButton::Right),
                },
                NetworkControl::Set(clonk_network::LegacyControlSet {
                    value_type: 1,
                    data: 123,
                    by_client,
                }),
            ],
        )
        .test_value();
        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms,
            initial_pressed
        );
        assert!(!app.engine.debug_mode());
        assert!(!app.engine.allow_debug());
        assert_eq!(
            app.graphics.debug_draw_flags(),
            clonk_frontend::DebugDrawFlags::default()
        );
        assert_eq!(app.executing_ready_tick, None);
    }
}

#[test]
fn immediate_control_sets_execute_for_host_and_non_host_disable_debug() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.engine.set_debug_mode(true);
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::Set(
            clonk_network::LegacyControlSet {
                value_type: 4,
                data: 1,
                by_client: 0,
            },
        )))
        .test_value();
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::Set(
            clonk_network::LegacyControlSet {
                value_type: 1,
                data: 0,
                by_client: 7,
            },
        )))
        .test_value();

    app.test_network_events();
    assert!(app.engine.team_colors());
    assert!(!app.engine.debug_mode());
    assert!(!app.engine.allow_debug());
}

#[test]
fn host_direct_elimination_closes_one_viewport_then_falls_back_silently() {
    // FnEliminatePlayer(plr, true) appends CID_RemovePlr to Game.Input;
    // it does not erase the player while the calling control/frame is
    // still executing (C4Script.cpp:2823-2833; C4PlayerList.cpp:480-484).
    let mut app = new_classic_lightweight_running_sandbox_app();
    let player = app.local_owner;
    assert_eq!(
        app.ui_sound_log
            .iter()
            .filter(|sound| sound.as_str() == "CloseViewport")
            .count(),
        1,
        "InitGameFinal creates the initial owned viewport non-silently"
    );
    app.ui_sound_log.clear();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let tick = u32::try_from(app.engine.frame()).test_value();
    let script = format!("EliminatePlayer({player}, true)");
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: vec![NetworkControl::Script(clonk_engine::ScriptControlData {
                target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                strictness: clonk_engine::ScriptStrictness::Strict3,
                script: clonk_engine::LegacyCString::from_bytes(script.into_bytes())
                    .expect("script has no NUL"),
                by_client: 0,
            })],
        })
        .test_value();

    app.test_update();

    assert!(app.engine.player(player).is_some());
    assert!(
        app.ui_sound_log.is_empty(),
        "elimination retains its viewport until the later RemovePlr"
    );
    let remove = clonk_engine::RemovePlayerControlData {
        player,
        disconnected: false,
        by_client: 0,
    };
    assert_eq!(
        commands.take_submitted_remove_players(),
        vec![(tick.saturating_add(1), remove)]
    );
    app.engine
        .replace_player_viewports(player, Vec::new())
        .test_value();

    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: tick.saturating_add(1),
            controls: vec![NetworkControl::RemovePlayer(remove)],
        })
        .test_value();
    app.test_update();

    assert!(app.engine.player(player).is_none());
    assert_eq!(
        app.ui_sound_log
            .iter()
            .filter(|sound| sound.as_str() == "CloseViewport")
            .count(),
        1,
        "closing all of one player's viewports requests one sound"
    );
    assert!(!app.snapshot.hud.local_players.contains(&player));

    app.ui_sound_log.clear();
    for _ in 0..2 {
        let viewports = collect_viewport_inputs(&app.snapshot).test_value();
        assert_eq!(viewports.len(), 1);
        assert_eq!(viewports[0].owner, OWNER_NONE);
    }
    app.execute_remove_player_control(remove).test_value();
    assert!(
        app.ui_sound_log.is_empty(),
        "the ownerless fallback and missing-player close are silent"
    );
}

#[test]
fn non_control_host_direct_elimination_returns_success_without_queuing() {
    let mut app = new_state_only_running_sandbox_app();
    let player = app.local_owner;
    app.engine.set_control_host(false);
    let script = format!("EliminatePlayer({player}, true)");
    let result = app
        .engine
        .execute_script_control(
            &clonk_engine::ScriptControlData {
                target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                strictness: clonk_engine::ScriptStrictness::Strict3,
                script: clonk_engine::LegacyCString::from_bytes(script.into_bytes())
                    .expect("script has no NUL"),
                by_client: 0,
            },
            ScriptControlPolicy::live(false),
        )
        .test_value();

    assert_eq!(result, Some(Value::Int(1)));
    assert!(app.engine.take_pending_remove_player_controls().is_empty());
    assert!(app.engine.player(player).is_some());
}

#[test]
fn offline_direct_elimination_waits_for_next_control_rate_frame() {
    let mut app = new_classic_running_sandbox_app();
    let player = app.local_owner;
    app.engine.set_control_rate(3);
    app.snapshot = app.engine.test_tick();
    let script = format!("EliminatePlayer({player}, true)");
    app.engine
        .execute_script_control(
            &clonk_engine::ScriptControlData {
                target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                strictness: clonk_engine::ScriptStrictness::Strict3,
                script: clonk_engine::LegacyCString::from_bytes(script.into_bytes())
                    .expect("script has no NUL"),
                by_client: 0,
            },
            ScriptControlPolicy::live(false),
        )
        .test_value();

    app.test_update();
    assert!(app.engine.player(player).is_some());
    app.test_update();
    assert!(app.engine.player(player).is_some());
    app.test_update();
    assert!(app.engine.player(player).is_none());
}

#[test]
fn remove_player_control_is_host_only_and_propagates_disconnected() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    for (player, info_id) in [(17, 7), (18, 8)] {
        app.engine
            .register_player(
                PlayerConfig::new(player, format!("Player {player}")).with_player_info_id(info_id),
            )
            .test_value();
    }
    let player_infos = vec![
        clonk_engine::ControlPlayerInfoEntry {
            id: 7,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            game_part_frame: -99,
            ..Default::default()
        },
        clonk_engine::ControlPlayerInfoEntry {
            id: 8,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            game_part_frame: -99,
            ..Default::default()
        },
    ];
    app.control_player_infos
        .replace_snapshot(8, [netplay_player_info_data(3, player_infos.clone())]);
    let mut host_snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    host_snapshot.parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: 8,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 3,
            flags: 0,
            players: player_infos,
        }],
    };
    app.host_join_snapshot = Some(host_snapshot);

    app.apply_ready_controls(
        0,
        vec![NetworkControl::RemovePlayer(
            clonk_engine::RemovePlayerControlData {
                player: 17,
                disconnected: true,
                by_client: 3,
            },
        )],
    )
    .test_value();
    assert!(app.engine.player(17).is_some());
    assert_eq!(
        app.control_player_infos
            .get(7)
            .expect("info retained")
            .flags
            & (clonk_engine::PLAYER_INFO_FLAG_REMOVED
                | clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED),
        0
    );
    assert_eq!(
        app.control_player_infos
            .get(7)
            .expect("info retained")
            .game_part_frame,
        -99
    );

    app.apply_ready_controls(
        1,
        vec![
            NetworkControl::RemovePlayer(clonk_engine::RemovePlayerControlData {
                player: 17,
                disconnected: true,
                by_client: 0,
            }),
            NetworkControl::RemovePlayer(clonk_engine::RemovePlayerControlData {
                player: 18,
                disconnected: false,
                by_client: 0,
            }),
        ],
    )
    .test_value();

    assert!(app.engine.player(17).is_none());
    assert!(app.engine.player(18).is_none());
    let disconnected = app.control_player_infos.get(7).test_value();
    assert_ne!(
        disconnected.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED,
        0
    );
    assert_ne!(
        disconnected.flags & clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED,
        0
    );
    assert_eq!(disconnected.game_part_frame, 0);
    let ordinary = app.control_player_infos.get(8).test_value();
    assert_ne!(ordinary.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED, 0);
    assert_eq!(
        ordinary.flags & clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED,
        0
    );
    assert_eq!(ordinary.game_part_frame, 0);
    let published = commands.take_published_join_snapshots();
    assert_eq!(published.len(), 2);
    assert_eq!(published.last(), app.host_join_snapshot.as_ref());
    let published_players = &published
        .last()
        .test_value()
        .parameters
        .player_infos
        .clients[0]
        .players;
    assert_ne!(
        published_players[0].flags & clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED,
        0
    );
    assert_eq!(
        published_players[1].flags & clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED,
        0
    );
    assert_eq!(
        published_players
            .iter()
            .map(|info| info.game_part_frame)
            .collect::<Vec<_>>(),
        vec![0, 0]
    );
}

#[test]
fn synchronized_activation_admits_later_remote_player_info() {
    // Client activation executes as a host-authored synchronized control.
    // A later direct PlayerInfo may immediately queue JoinPlayer only when
    // that synchronized client exists and is active
    // (src/C4Control.cpp:578-620;
    // src/C4Network2Players.cpp:245-269).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::ClientJoin(
            clonk_engine::ClientJoinControlData {
                core: clonk_engine::ClientCoreControlData {
                    client_id: 3,
                    activated: false,
                    observer: false,
                    name: clonk_engine::LegacyCString::from_bytes(b"Remote".to_vec()).test_value(),
                    nick: clonk_engine::LegacyCString::from_bytes(b"R".to_vec()).test_value(),
                    lobby_ready: false,
                },
                by_client: 0,
            },
        )))
        .test_value();
    app.test_network_events();
    assert!(app.control_clients.contains(3));
    assert!(!app.control_clients.is_activated(3));
    let tick = app.local_control_submission_tick();

    app.apply_ready_controls(
        tick,
        vec![NetworkControl::ClientUpdate(
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 3,
                data: 1,
                by_client: 0,
            },
        )],
    )
    .test_value();
    assert!(commands.take_submitted_join_players().is_empty());

    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 41,
                    player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                    ..Default::default()
                }],
                by_client: 0,
                ..Default::default()
            },
        )))
        .test_value();
    app.test_network_events();

    let joins = commands.take_submitted_join_players();
    assert_eq!(joins.len(), 1);
    assert_eq!((joins[0].1.at_client, joins[0].1.info_id), (3, 41));
}

#[test]
fn host_activation_request_submits_cpp_eligible_synchronized_update() {
    // HandleActivateReq accepts only a waited-for inactive non-observer;
    // its lag window uses Game.FrameCounter, measured Game.FPS and the
    // connection ping before queuing host-authored CUT_Activate via
    // CDT_Sync (pristine 9ffa0a5d src/C4Network2.cpp:1553-1571).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_clients.register(3, false, false);
    app.frames_per_second = 60;
    let frame = i32::try_from(app.engine.frame()).test_value();
    event_tx
        .send(NetworkEvent::ActivationRequest {
            client_id: 3,
            tick: frame,
            waited_for: true,
            ping_ms: 25,
        })
        .test_value();

    app.test_network_events();

    assert_eq!(
        commands.take_submitted_client_updates(),
        vec![clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 1,
            by_client: 0,
        }]
    );
}

#[test]
fn classic_host_lobby_activation_request_has_no_presentation_child() {
    // HandlePacket dispatches PID_ClientActReq directly to
    // HandleActivateReq, which queues CUT_Activate through CDT_Sync.
    // The request does not open a lobby child or otherwise mutate the
    // lobby presentation (pristine 9ffa0a5d
    // src/C4Network2.cpp:989-998,1569-1588).
    let mut app = new_state_only_running_sandbox_app();
    app.mode = AppMode::Menu;
    install_test_classic_host_lobby(&mut app);
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_clients.register(3, false, false);
    event_tx
        .send(NetworkEvent::ActivationRequest {
            client_id: 3,
            tick: -1,
            waited_for: true,
            ping_ms: 25,
        })
        .test_value();

    app.test_network_events();

    assert_eq!(
        commands.take_submitted_client_updates(),
        vec![clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 1,
            by_client: 0,
        }]
    );
    assert!(app.classic_host_lobby.is_some());
}

#[test]
fn classic_host_lobby_status_commit_has_no_presentation_child() {
    // CheckStatusAck commits the reached GS_Lobby status through
    // OnStatusAck. That transition updates network/control state and
    // retains the active lobby; it does not open a lobby child
    // (pristine 9ffa0a5d
    // src/C4Network2.cpp:1529-1550,2062-2110).
    let mut app = new_state_only_running_sandbox_app();
    app.mode = AppMode::Menu;
    install_test_classic_host_lobby(&mut app);
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.network_control_running = true;
    app.status_text = "lobby presentation sentinel".to_string();
    let lobby = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_LOBBY,
        control_mode: 0,
        target_tick: 0,
    };
    event_tx
        .send(NetworkEvent::StatusCommitted(lobby))
        .test_value();

    app.test_network_events();

    assert_eq!(app.runtime_network_committed_status, Some(lobby));
    assert!(!app.network_control_running);
    assert!(app.classic_host_lobby.is_some());
    assert_eq!(app.mode, AppMode::Menu);
    assert_eq!(app.status_text, "lobby presentation sentinel");
}

#[test]
fn network_client_activity_uses_player_presence_and_strict_frame_age() {
    // UpdateClientActivity refreshes from Game.Players.GetAtClient, not
    // from received controls. DeactivateInactiveClients uses a strict
    // `last + 500 < FrameCounter` comparison and excludes the local
    // client (src/C4Network2Client.cpp:648-654;
    // src/C4Network2.cpp:2148-2159).
    let mut activity = NetworkClientActivity::default();

    assert!(activity
        .deactivation_candidates([0, 3], [], 0, 500)
        .is_empty());
    assert_eq!(
        activity.deactivation_candidates([0, 3], [], 0, 501),
        vec![3]
    );

    activity.mark_activated(3, 700);
    assert!(activity
        .deactivation_candidates([0, 3], [], 0, 1_200)
        .is_empty());
    assert_eq!(
        activity.deactivation_candidates([0, 3], [], 0, 1_201),
        vec![3]
    );

    assert!(activity
        .deactivation_candidates([0, 3], [3], 0, 10_000)
        .is_empty());
}

#[test]
fn runtime_player_refreshes_inactive_client_even_when_eliminated() {
    // C4PlayerList::GetAtClient does not filter eliminated players. Any
    // extant runtime player refreshes iLastActivity and prevents this
    // delayed path from selecting its client (src/C4PlayerList.cpp:487-496).
    let mut app = new_running_sandbox_app();
    for _ in 0..501 {
        app.engine.test_tick();
    }
    app.engine
        .register_player(PlayerConfig::new(17, "Remote"))
        .test_value();
    app.engine
        .test_player_mut(17)
        .set_at_client(clonk_engine::PlayerAtClient::new(3));
    app.engine
        .set_player_status(17, clonk_engine::PlayerStatus::Eliminated)
        .test_value();
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.network_control_running = false;
    app.control_clients.register(3, true, false);

    app.test_update();

    assert!(commands.take_submitted_client_updates().is_empty());
    assert!(app.control_clients.is_activated(3));
}

#[test]
fn control_rate_two_client_executes_scheduled_activation_at_the_control_tick() {
    // PID_ExecSyncCtrl carries Game.Control.ControlTick, not
    // Game.FrameCounter. At ControlRate 2 the client must retain a sync
    // control queued at tick 3 while FrameCounter is already 5, then
    // execute it on the next cadence frame (src/C4GameControlNetwork.cpp:
    // 279-297,558-588,786-830; src/C4Control.cpp:578-606).
    let mut app = new_running_sandbox_app();
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(3);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.control_clients.register(3, false, false);
    app.network_control_clock = Some(NetworkControlClock::new(0, 2));
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(0, 2).test_value(),
    );

    for tick in 0..3 {
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: Vec::new(),
            })
            .test_value();
        app.test_update();
        if tick < 2 {
            app.test_update();
        }
    }
    assert_eq!(app.engine.frame(), 5);
    assert_eq!(app.expected_network_control_tick(), 3);

    let update = clonk_engine::ClientUpdateControlData {
        update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
        client_id: 3,
        data: 1,
        by_client: 0,
    };
    event_tx
        .send(NetworkEvent::ScheduledSync {
            tick: 3,
            controls: vec![NetworkControl::ClientUpdate(update.clone())],
        })
        .test_value();

    app.test_update();
    assert_eq!(app.engine.frame(), 6);
    assert!(!app.control_clients.is_activated(3));
    assert!(commands.take_executed_client_updates().is_empty());

    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 3,
            controls: Vec::new(),
        })
        .test_value();
    app.test_update();

    assert_eq!(app.engine.frame(), 7);
    assert!(app.control_clients.is_activated(3));
    assert_eq!(commands.take_executed_client_updates(), vec![update]);
}

#[test]
fn synchronized_client_remove_prunes_and_rechecks_teams_without_client_host_cascade() {
    // ClientRemove is host-authored synchronized state. OnClientPart
    // discards never-joined infos, retains joined history, and rechecks
    // team membership on every peer. Attribute/gain mutation and direct
    // PlayerInfo broadcasts remain host-only
    // (src/C4Control.cpp:637-680;
    // src/C4Network2Players.cpp:425-459).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(2);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    // The sandbox fixture starts from an offline team-assignment owner;
    // an actual network client does not retain that host/offline state.
    app.network_team_assignment = None;
    app.control_clients.register(3, true, false);
    let metadata =
        set_control_test_metadata(false, vec![set_control_test_team(1, vec![7, 8, 9], 0)]);
    app.engine
        .register_player(PlayerConfig::new(17, "Remote").with_player_info_id(7))
        .test_value();
    app.control_player_infos.apply(netplay_player_info_data(
        3,
        vec![
            clonk_engine::ControlPlayerInfoEntry {
                id: 7,
                team: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                ..Default::default()
            },
            clonk_engine::ControlPlayerInfoEntry {
                id: 8,
                team: 1,
                ..Default::default()
            },
        ],
    ));
    let retained_color = 0x0000_f400;
    app.control_player_infos.apply(netplay_player_info_data(
        4,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: 9,
            team: 1,
            color: retained_color,
            original_color: 0x00f4_0000,
            forced_name: clonk_engine::LegacyCString::from_bytes(b"Alias".to_vec()).unwrap(),
            league_projected_gain: 5,
            ..Default::default()
        }],
    ));
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));

    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 3,
                reason: clonk_engine::LegacyCString::default(),
                by_client: 0,
            },
        )],
    )
    .test_value();

    assert!(!app.control_clients.contains(3));
    assert!(app
        .engine
        .snapshot()
        .players
        .iter()
        .all(|player| player.player_info_id != 7));
    let retained = app.control_player_infos.get(7).test_value();
    assert_ne!(retained.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED, 0);
    assert_ne!(
        retained.flags & clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED,
        0
    );
    assert!(app.control_player_infos.get(8).is_none());
    assert_eq!(app.engine.teams()[0].player_ids, vec![9]);
    let unaffected = app.control_player_infos.get(9).test_value();
    assert_eq!(unaffected.color, retained_color);
    assert_eq!(unaffected.forced_name.as_bytes(), b"Alias");
    assert_eq!(unaffected.league_projected_gain, 5);
    assert!(commands.take_broadcast_player_infos().is_empty());
}

#[test]
fn synchronized_client_remove_without_player_info_skips_the_part_cascade() {
    // OnClientPart returns before team/attribute/league work when no
    // C4ClientPlayerInfos packet exists for the departing client
    // (src/C4Network2Players.cpp:425-430).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients.register(3, true, false);
    let metadata = set_control_test_metadata(false, vec![set_control_test_team(1, vec![77], 0)]);
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.control_player_infos.apply(netplay_player_info_data(
        4,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: 77,
            team: 1,
            color: 0x0000_f400,
            original_color: 0x00f4_0000,
            league_projected_gain: 5,
            ..Default::default()
        }],
    ));

    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 3,
                reason: clonk_engine::LegacyCString::default(),
                by_client: 0,
            },
        )],
    )
    .test_value();

    let retained = app.control_player_infos.get(77).test_value();
    assert_eq!(retained.color, 0x0000_f400);
    assert_eq!(retained.league_projected_gain, 5);
    assert_eq!(
        app.network_team_assignment.as_ref().unwrap().teams().teams[0].player_ids,
        vec![77],
    );
    assert!(commands.take_broadcast_player_infos().is_empty());
}

#[test]
fn network_host_without_control_host_still_runs_client_part_updates() {
    // OnClientPart's attribute/gain/send guard is network-host ownership;
    // only the nested random-team recheck requires control-host status
    // (src/C4Network2Players.cpp:449-459; src/C4Teams.cpp:688-692).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.engine.set_control_host(false);
    app.control_clients.register(3, true, false);
    app.control_player_infos.replace_snapshot(
        40,
        [
            netplay_player_info_data(3, vec![set_control_test_player(30, 0, 0)]),
            netplay_player_info_data(
                4,
                vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 40,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                        | clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                    league_projected_gain: 5,
                    ..Default::default()
                }],
            ),
        ],
    );

    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 3,
                reason: clonk_engine::LegacyCString::default(),
                by_client: 0,
            },
        )],
    )
    .test_value();

    assert_eq!(
        app.control_player_infos
            .get(40)
            .unwrap()
            .league_projected_gain,
        -1,
    );
    let broadcasts = commands.take_broadcast_player_infos();
    let [update] = broadcasts.as_slice() else {
        panic!("expected one gain-reset packet, got {broadcasts:?}");
    };
    assert_eq!(update.client_id, 4);
    assert_eq!(update.players[0].league_projected_gain, -1);
}

#[test]
fn client_host_socket_loss_continues_the_running_round_locally() {
    // When a client's only host connection is gone, OnClientDisconnect
    // clears C4Network2 and thereby executes C4GameControl::ChangeToLocal;
    // it does not abort or return to startup (pristine 9ffa0a5d
    // src/C4Network2.cpp:1758-1765,1786-1817;
    // src/C4GameControl.cpp:93-127).
    let mut app = new_running_sandbox_app();
    let local_player = app.local_owner;
    let local_client = 7;
    let remote_player = 17;
    let remote_info = 73;
    app.engine
        .test_player_mut(local_player)
        .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
    app.engine
        .register_player(
            PlayerConfig::new(remote_player, "Host player").with_player_info_id(remote_info),
        )
        .test_value();
    app.engine
        .test_player_mut(remote_player)
        .set_at_client(clonk_engine::PlayerAtClient::HOST);
    app.engine.set_network_game(true);
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(31, 4).test_value(),
    );
    app.snapshot = app.engine.snapshot();

    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_control_clock = Some(NetworkControlClock::new(31, 4));
    app.network_max_players = 9;
    app.network_is_league = true;
    app.control_clients = ControlClientRegistry::default();
    app.control_clients.register(0, true, false);
    app.control_clients.register(local_client, true, false);
    app.control_player_infos.apply(netplay_player_info_data(
        0,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: remote_info,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            ..Default::default()
        }],
    ));
    app.apply_ingame_menu_action(MenuAction::ActivateOptions)
        .test_value();
    let frame_before = app.engine.frame();
    let control_tick_before = app.engine.sync_check(local_client).control_tick;

    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id: 0,
            reason: Some("connection lost".to_string()),
        })
        .test_value();
    app.test_network_events();

    assert!(matches!(app.mode, AppMode::Running));
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert_eq!(app.engine.frame(), frame_before);
    assert_eq!(
        app.engine.sync_check(local_client).control_tick,
        control_tick_before
    );
    assert_eq!(app.engine.control_rate, 1);
    assert_eq!(app.network_max_players, 9);
    assert!(app.network_is_league);
    assert!(app.engine.player(local_player).is_some());
    assert!(app.engine.player(remote_player).is_none());
    assert_eq!(
        app.ingame_menu.as_ref().map(IngameMenuState::page),
        Some(ingame_menu::MenuPage::Options)
    );
    let engine_results = app.engine.snapshot().round_results;
    assert_eq!(
        engine_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        engine_results.network_result_message,
        b"Network: host Host disconnected!"
    );
    assert_eq!(app.snapshot.round_results, engine_results);
}

#[test]
fn client_follows_an_announced_host_restart_back_to_the_lobby() {
    // Same socket loss as the test above, but preceded by the host's restart
    // notice. Native has no such notice and therefore no way to tell the two
    // apart, so it drops every client to local control
    // (src/C4Network2.cpp:1826-1832) and the restarted lobby comes up empty.
    // With the intent stated, the client leaves the abandoned round and
    // reconnects to the address it already joined.
    let mut app = new_running_sandbox_app();
    let local_client = 7;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_control_clock = Some(NetworkControlClock::new(31, 4));
    app.control_clients.register(0, true, false);
    app.control_clients.register(local_client, true, false);

    event_tx
        .send(NetworkEvent::HostRestarting { rejoin_seconds: 30 })
        .test_value();
    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id: 0,
            reason: Some("connection lost".to_string()),
        })
        .test_value();
    app.test_network_events();

    assert_eq!(
        app.mode,
        AppMode::Menu,
        "the round the host abandoned is torn down, not continued locally"
    );
    assert!(app.network.is_none());
    assert!(
        app.pending_host_rejoin.is_some(),
        "the rejoin stays armed until the connection resolves"
    );
    assert_eq!(
        app.startup_network_connection
            .as_ref()
            .map(|connection| connection.purpose),
        Some(StartupNetworkPurpose::Join),
        "the client reconnects to the restarted host"
    );
}

#[test]
fn client_follows_a_session_preserving_restart_into_the_lobby() {
    // The cheaper half of the same intent. Here the host keeps its session up,
    // so there is no socket loss to observe and nothing to re-dial: the client
    // leaves the round and re-enters the lobby on the connection it already
    // holds. Contrast `client_follows_an_announced_host_restart_back_to_the_lobby`,
    // which pays for a whole new connection to reach the same place.
    let mut app = new_running_sandbox_app();
    let local_client = 7;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_control_clock = Some(NetworkControlClock::new(31, 4));
    app.control_clients.register(0, true, false);
    app.control_clients.register(local_client, true, false);

    event_tx.send(NetworkEvent::HostRestartLobby).test_value();
    app.test_network_events();

    assert_eq!(
        app.mode,
        AppMode::Menu,
        "the round the host restarted is torn down"
    );
    assert!(
        app.network.is_some(),
        "the session the host kept up is the one this client keeps"
    );
    assert!(
        app.startup_network_connection.is_none(),
        "nothing is re-dialled: that is the whole point of the notice"
    );
    assert!(
        app.pending_host_rejoin.is_none(),
        "a preserved session has no reconnect to arm"
    );
    assert!(
        app.network_lobby.is_some(),
        "the client lands in the lobby rather than the game list"
    );
    assert!(
        app.network_lobby
            .test_ref()
            .controller
            .rows()
            .iter()
            .any(|row| row.id() == LobbyRosterId::Client(0)),
        "the connected host must remain in the restarted lobby roster"
    );
}

#[test]
fn an_oversized_rejoin_window_is_clamped() {
    // The window is a number off the wire. Honoured literally, a hostile or
    // buggy host could hold this client in a once-a-second reconnect loop for
    // eighteen hours.
    let mut app = new_running_sandbox_app();
    let (manager, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    app.arm_pending_host_rejoin(u16::MAX);

    let deadline = app.pending_host_rejoin.test_ref().deadline;
    assert!(
        deadline
            <= Instant::now() + Duration::from_secs(u64::from(MAX_HOST_RESTART_REJOIN_SECONDS)),
        "a peer-supplied window must not exceed the local ceiling"
    );

    app.arm_pending_host_rejoin(0);
    assert!(
        app.pending_host_rejoin.is_some(),
        "a zero window declines to re-arm; it does not disarm what is already armed"
    );
}

#[test]
fn cancelling_the_reconnect_dialog_abandons_the_rejoin() {
    // Every attempt raises the same CANCEL modal. If Cancel left the rejoin
    // armed, the retry would put the dialog straight back and the player would
    // be held on the main menu until the window expired.
    let mut app = new_classic_menu_app(800, 600);
    app.pending_host_rejoin = Some(PendingHostRejoin {
        settings: ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client"),
        deadline: Instant::now() + Duration::from_secs(30),
        next_attempt_at: None,
    });
    let (_sender, receiver) = mpsc::channel();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Connecting to host".to_string(),
            "Network".to_string(),
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::StartupNetworkConnectProgress,
    )
    .test_value();

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();

    assert!(app.startup_network_connection.is_none());
    assert!(
        app.pending_host_rejoin.is_none(),
        "Cancel must end the rejoin, not just the attempt in flight"
    );

    app.poll_startup_network_connection().test_value();
    assert!(
        app.startup_network_connection.is_none(),
        "the cancelled rejoin must not reopen its dialog"
    );
}

#[test]
fn a_window_that_closes_mid_attempt_reports_one_failure() {
    // The deadline can pass while a connect is still in flight. The failing
    // attempt and the expiry must not both unwind the startup screen, or the
    // player gets two teardowns and two stacked error dialogs.
    let mut app = new_classic_menu_app(800, 600);
    app.pending_host_rejoin = Some(PendingHostRejoin {
        settings: ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client"),
        deadline: Instant::now() - Duration::from_secs(1),
        next_attempt_at: None,
    });
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Err(NetworkStartError::Other(
            "connection refused".to_string(),
        )))
        .test_value();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));

    app.poll_startup_network_connection().test_value();
    let after_failure = app.message_dialogs.len();
    app.poll_startup_network_connection().test_value();

    assert!(app.pending_host_rejoin.is_none());
    assert_eq!(
        app.message_dialogs.len(),
        after_failure,
        "the expired window must not raise a second failure behind the first"
    );
}

#[test]
fn an_armed_rejoin_survives_the_hosts_rebind_window_and_then_gives_up() {
    // The host is still tearing its own session down when the notice arrives,
    // so the first reconnect necessarily races an unbound port. A single
    // refused connection must not end the rejoin — but the window the host
    // named must, or a host that never comes back would spin forever.
    let mut app = new_classic_menu_app(800, 600);
    app.pending_host_rejoin = Some(PendingHostRejoin {
        settings: ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client"),
        deadline: Instant::now() + Duration::from_secs(30),
        next_attempt_at: None,
    });
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Err(NetworkStartError::Other(
            "connection refused".to_string(),
        )))
        .test_value();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));

    app.poll_startup_network_connection().test_value();

    assert!(app.startup_network_connection.is_none());
    let scheduled = app.pending_host_rejoin.test_ref();
    assert!(
        scheduled.next_attempt_at.is_some(),
        "the next attempt waits for the host to finish re-binding"
    );
    assert!(
        app.startup_restart_diagnostics == StartupRestartDiagnostics::default(),
        "a retryable reconnect must not present a startup failure"
    );

    app.pending_host_rejoin = Some(PendingHostRejoin {
        settings: ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client"),
        deadline: Instant::now() - Duration::from_secs(1),
        next_attempt_at: None,
    });

    app.poll_startup_network_connection().test_value();

    assert!(
        app.pending_host_rejoin.is_none(),
        "the rejoin stops at the window the host named"
    );
    assert_eq!(app.startup_view, StartupView::NetworkGame);
}

#[test]
fn a_rejoin_reuses_the_live_join_settings_rather_than_rebuilding_them() {
    // The reconnect has to be the same join, not a fresh one typed from the
    // address bar: a password-protected or netpuncher-brokered host is only
    // reachable with the credentials and routes this client already holds.
    // Rebuilding ClientSettings from config would drop all of it and the
    // reconnect would be refused for the whole window.
    let mut app = new_running_sandbox_app();
    let local_client = 7;
    let password = clonk_engine::LegacyCString::from_bytes(b"hunter2".to_vec()).test_value();
    let settings = ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client")
        .with_password(password.clone())
        .with_compatibility_build(42);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(settings.clone()));
    app.network_control_clock = Some(NetworkControlClock::new(31, 4));

    event_tx
        .send(NetworkEvent::HostRestarting { rejoin_seconds: 30 })
        .test_value();
    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id: 0,
            reason: None,
        })
        .test_value();
    app.test_network_events();

    let relaunched = app.pending_network_join.test_ref();
    assert_eq!(relaunched.password, password);
    assert_eq!(relaunched.compatibility_build, 42);
    assert_eq!(
        relaunched.logical_server_addresses,
        settings.logical_server_addresses
    );
}

#[test]
fn dropping_to_local_control_abandons_an_armed_rejoin() {
    // ChangeToLocal keeps the round running with no manager
    // (src/C4GameControl.cpp:93-127), which is the same `network.is_none()`
    // the rejoin poll waits for. Without an explicit abandon, a worker-level
    // failure would open a reconnect dialog over a live, simulating round.
    let mut app = new_running_sandbox_app();
    let local_client = 7;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_control_clock = Some(NetworkControlClock::new(31, 4));

    event_tx
        .send(NetworkEvent::HostRestarting { rejoin_seconds: 30 })
        .test_value();
    event_tx
        .send(NetworkEvent::FatalError("worker failed".to_string()))
        .test_value();
    app.test_network_events();
    app.poll_startup_network_connection().test_value();

    assert_eq!(app.mode, AppMode::Running, "the round continues locally");
    assert!(app.network.is_none());
    assert!(
        app.pending_host_rejoin.is_none(),
        "a round that dropped to local control is no longer following the host"
    );
    assert!(
        app.startup_network_connection.is_none(),
        "no reconnect dialog may open over a live round"
    );
}

#[test]
fn an_announced_restart_alone_does_not_disturb_the_running_round() {
    // The notice arrives while the host is still connected and the round is
    // still simulating; only the disconnect it predicts may act on it. A host
    // that announces and then does not go away must leave the round untouched.
    let mut app = new_running_sandbox_app();
    let local_client = 7;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_control_clock = Some(NetworkControlClock::new(31, 4));

    event_tx
        .send(NetworkEvent::HostRestarting { rejoin_seconds: 30 })
        .test_value();
    app.test_network_events();
    app.poll_startup_network_connection().test_value();

    assert!(app.pending_host_rejoin.is_some(), "the intent is recorded");
    assert!(
        app.startup_network_connection.is_none(),
        "no reconnect may start while the session it would replace is still live"
    );
    assert!(app.network.is_some());
    assert_eq!(app.mode, AppMode::Running);
}

#[test]
fn a_client_waiting_in_the_lobby_also_follows_an_announced_restart() {
    // The Restart button also exists on C4Network2StartWaitDlg
    // (src/C4Network2Dialogs.cpp:574-584), so a client can be sitting in the
    // lobby when the host restarts. Host loss there normally unwinds
    // C4Game::Init back to the startup dialog (src/C4Network2.cpp:477-515),
    // which for an announced restart would throw the player out of a lobby
    // that is about to exist again.
    let mut app = new_running_sandbox_app();
    let local_client = 7;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.mode = AppMode::Menu;
    app.startup_view = StartupView::NetworkLobby;
    app.control_clients.register(0, true, false);
    app.control_clients.register(local_client, true, false);

    event_tx
        .send(NetworkEvent::HostRestarting { rejoin_seconds: 30 })
        .test_value();
    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id: 0,
            reason: Some("connection lost".to_string()),
        })
        .test_value();
    app.test_network_events();

    assert_eq!(
        app.startup_network_connection
            .as_ref()
            .map(|connection| connection.purpose),
        Some(StartupNetworkPurpose::Join),
        "a lobby client reconnects instead of unwinding to the game list"
    );
    assert!(app.pending_host_rejoin.is_some());
}

#[test]
fn a_rejoin_disarms_when_it_resolves_or_the_player_leaves() {
    // An armed rejoin is a standing instruction to reconnect. Left armed after
    // it has done its job, its window would later expire underneath a healthy
    // lobby and tear that lobby down; left armed after the player quits, it
    // would dial the host again from the main menu.
    let mut app = new_real_classic_menu_app(800, 600);
    let armed = || PendingHostRejoin {
        settings: ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client"),
        deadline: Instant::now() + Duration::from_secs(30),
        next_attempt_at: Some(Instant::now() + HOST_REJOIN_RETRY_INTERVAL),
    };
    app.pending_host_rejoin = Some(armed());
    let (manager, _events) = NetworkManager::test_stub_for_client_id(7);
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Client(ClientSettings::new(
                SocketAddr::from(([127, 0, 0, 1], 11_112)),
                "Client",
            )),
            manager,
        )))
        .test_value();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));

    let _ = app.poll_startup_network_connection();

    assert!(
        app.pending_host_rejoin.is_none(),
        "a resolved rejoin must not keep its deadline running"
    );

    app.pending_host_rejoin = Some(armed());
    app.return_to_menu();

    assert!(
        app.pending_host_rejoin.is_none(),
        "leaving the round abandons the rejoin with it"
    );
}

#[test]
fn client_non_host_peer_loss_keeps_the_network_session() {
    // OnClientDisconnect clears a client's network only when the lost
    // C4Network2Client is the host. Another peer's eventual synchronized
    // removal remains host-owned (pristine 9ffa0a5d
    // src/C4Network2.cpp:1786-1817).
    let mut app = new_state_only_running_sandbox_app();
    let local_client = 7;
    let peer_client = 9;
    let peer_player = 17;
    app.engine
        .register_player(PlayerConfig::new(peer_player, "Peer"))
        .test_value();
    app.engine
        .test_player_mut(peer_player)
        .set_at_client(clonk_engine::PlayerAtClient::new(peer_client));
    app.snapshot = app.engine.snapshot();
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(local_client as u32);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.control_clients.register(0, true, false);
    app.control_clients.register(local_client, true, false);
    app.control_clients.register(peer_client, true, false);

    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id: peer_client as u32,
            reason: Some("peer transport lost".to_string()),
        })
        .test_value();
    app.test_network_events();

    assert!(app.network.is_some());
    assert!(matches!(app.network_mode, Some(NetworkMode::Client(_))));
    assert!(matches!(app.mode, AppMode::Running));
    assert!(app.engine.player(peer_player).is_some());
}

#[test]
fn removing_local_network_client_changes_to_local_control() {
    // C4ControlClientRemove never deletes the local client. It invokes
    // C4GameControl::ChangeToLocal, which clears networking, removes
    // remote client records, and activates the local client
    // (src/C4Control.cpp:651-660; src/C4GameControl.cpp:94-128).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx) = NetworkManager::test_stub_for_client_id(3);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11112)),
        "Client",
    )));
    app.control_clients = ControlClientRegistry::default();
    app.control_clients.register(0, true, false);
    app.control_clients.register(3, false, false);

    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 3,
                reason: clonk_engine::LegacyCString::from_bytes(b"Removed".to_vec())
                    .expect("valid reason"),
                by_client: 0,
            },
        )],
    )
    .test_value();

    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_lobby.is_none());
    assert!(app.control_clients.contains(3));
    assert!(app.control_clients.is_activated(3));
    assert!(!app.control_clients.contains(0));
}

#[test]
fn running_stale_lobby_state_cannot_execute_scheduled_sync_immediately() {
    // DoLobby deletes pLobby before returning from GO. Defensively, even
    // a stale adapter cannot make later PID_ExecSyncCtrl packets execute
    // through the frozen-lobby path (src/C4Network2.cpp:493-515;
    // src/C4GameControlNetwork.cpp:558-588).
    let mut app = new_state_only_running_sandbox_app();
    let (events, _commands) = install_running_network_stub(&mut app, 7, 0, 1);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    events
        .send(NetworkEvent::ScheduledSync {
            tick: 1,
            controls: vec![NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type: 2,
                data: 23,
                by_client: 0,
            })],
        })
        .test_value();

    app.test_network_events();

    assert_ne!(
        app.engine.max_players(),
        Some(23),
        "post-GO sync must not execute through the frozen-lobby path"
    );
    assert_eq!(app.network_sync.take_exact(1).len(), 1);
}

#[test]
fn final_go_applies_lifecycle_sync_before_active_client_sweep() {
    // CheckStatusAck executes pending sync controls before
    // OnStatusGoReached scans every active client, then starts control.
    // PID_ExecSyncCtrl tick 3 must remain valid when ControlRate 2 has
    // already advanced FrameCounter to 6 (src/C4Network2.cpp:2062-2110;
    // src/C4Network2Players.cpp:465-482).
    let directory = tempdir();
    let mut app = new_state_only_running_sandbox_app();
    install_test_recording_template(&mut app, directory.path().join("001-FinalGoSync.c4s"));
    app.network_control_clock = Some(NetworkControlClock::new(3, 2));
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(0, 2).test_value(),
    );
    for _ in 0..6 {
        app.engine.test_tick();
    }
    assert_eq!(app.engine.frame(), 6);
    assert_eq!(app.expected_network_control_tick(), 3);
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.network_control_running = false;
    app.control_clients.register(3, false, false);
    app.control_clients.register(4, true, false);
    app.control_player_infos.apply(netplay_player_info_data(
        3,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: 31,
            player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        }],
    ));
    app.control_player_infos.apply(netplay_player_info_data(
        4,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: 41,
            player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        }],
    ));

    event_tx
        .send(NetworkEvent::ScheduledSync {
            tick: 3,
            controls: vec![
                NetworkControl::Synchronize(clonk_engine::SynchronizeControlData {
                    save_player_files: false,
                    sync_clearance: true,
                    by_client: 0,
                }),
                NetworkControl::ClientUpdate(clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    client_id: 3,
                    data: 1,
                    by_client: 0,
                }),
                NetworkControl::ClientRemove(clonk_engine::ClientRemoveControlData {
                    client_id: 4,
                    reason: clonk_engine::LegacyCString::default(),
                    by_client: 0,
                }),
            ],
        })
        .test_value();
    event_tx
        .send(NetworkEvent::StatusCommitted(
            clonk_network::NetworkStatus {
                state: clonk_network::NETWORK_STATE_GO,
                control_mode: 1,
                target_tick: 3,
            },
        ))
        .test_value();

    app.test_network_events();

    assert!(app.control_clients.is_activated(3));
    assert!(!app.control_clients.contains(4));
    assert!(app.control_player_infos.get(41).is_none());
    let joins = commands.take_submitted_join_players();
    assert_eq!(joins.len(), 1);
    assert_eq!(joins[0].0, 3);
    assert_eq!((joins[0].1.at_client, joins[0].1.info_id), (3, 31));
    assert!(app.network_control_running);
    assert!(app.recording.is_none());
    assert!(app.recording_template.is_some());
}

#[test]
fn network_update_uses_join_control_tick_and_cpp_control_rate() {
    // Network control starts at JoinData::iStartCtrlTick and gates only
    // frames divisible by Parameters.ControlRate. A missing aggregate
    // retries that same frame/tick; non-control frames simulate without a
    // control packet (pristine 9ffa0a5d src/C4GameControl.cpp:245-329;
    // src/C4GameControlNetwork.cpp:48-60).
    let mut app = new_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_control_clock = Some(network::NetworkControlClock::new(9, 2));
    assert_eq!(app.engine.frame(), 0);

    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 9,
            controls: Vec::new(),
        })
        .test_value();
    app.test_update();
    assert_eq!(app.engine.frame(), 1);

    app.test_update();
    assert_eq!(app.engine.frame(), 2);

    app.test_update();
    assert_eq!(app.engine.frame(), 2, "stalled control frame is retried");
    assert_eq!(
        app.network_control_clock.map(|clock| clock.current_tick()),
        Some(10)
    );

    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 10,
            controls: Vec::new(),
        })
        .test_value();
    app.test_update();
    assert_eq!(app.engine.frame(), 3);
}

#[test]
fn joined_lobby_non_roster_network_batch_keeps_cached_player_raster() {
    // MainDlg's player list refreshes only from its explicit lobby
    // callbacks or one-second timer (src/C4GameLobby.cpp:669-680,
    // 766-777). DoLobby deletes MainDlg before running mode, so this
    // cache exists only in the joined lobby (src/C4Network2.cpp:493-515).
    let mut app = new_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    manager.set_test_lobby_client_telemetry(clonk_network::RuntimeLobbyClientTelemetry {
        connections: vec![clonk_network::RuntimeNetworkConnection {
            connection_id: 1,
            client_id: 0,
            usage: "Data/Msg".to_string(),
            protocol: clonk_network::NetworkProtocol::Tcp,
            peer_address: None,
            packet_loss: 0,
            ping_ms: 12,
            lag_ms: 12,
        }],
        resource_progress: vec![(0, 80)],
    });
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Client")]);
    app.network_control_clock = Some(NetworkControlClock::new(0, 2));
    let cached_player_raster = ImageData::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    let cached_rows = vec![
        LobbyRosterRow::Client(LobbyClientRow {
            id: 0,
            name: "Host".to_string(),
            nick: String::new(),
            color: [255, 255, 255, 255],
            status: LobbyClientStatus::Host,
            local: false,
            connected: false,
            resource_progress: None,
            ping_ms: None,
        }),
        LobbyRosterRow::Player(LobbyPlayerRow {
            id: 41,
            client_id: 7,
            name: "Cached raster".to_string(),
            color: [1, 2, 3, 255],
            icon: LobbyRosterIcon::Raster(cached_player_raster.clone()),
            joined_player_overlay: None,
            team: None,
            league_score: None,
            league_rank: None,
        }),
    ];
    let mut lobby = NetworkLobbyState::new(7, "Client".to_string(), false);
    lobby.roster_rows.clone_from(&cached_rows);
    lobby.roster_rows_authoritative = true;
    lobby.controller.set_rows(cached_rows.clone());
    app.network_lobby = Some(lobby);
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 77,
            controls: Vec::new(),
        })
        .test_value();
    event_tx
        .send(NetworkEvent::HostPingMeasured { round_trip_ms: 12 })
        .test_value();

    app.test_network_events();
    let clock = app.network_control_clock.test_mut();
    assert_eq!(clock.calculate_performance(), None);
    assert_eq!(
        clock.avg_control_send_time(),
        0,
        "host-only ping telemetry must not replace the consumed topology sample"
    );

    let lobby = app.network_lobby.test_ref();
    let host = lobby
        .roster_rows
        .iter()
        .find_map(|row| match row {
            LobbyRosterRow::Client(client) if client.id == 0 => Some(client),
            _ => None,
        })
        .test_value();
    assert!(!host.connected);
    assert_eq!(host.ping_ms, None);
    assert_eq!(host.resource_progress, None);
    assert!(
        lobby.roster_rows.iter().any(|row| {
            matches!(
                row,
                LobbyRosterRow::Player(player)
                    if player.id == 41
                        && matches!(
                            &player.icon,
                            LobbyRosterIcon::Raster(raster)
                                if raster == &cached_player_raster
                        )
            )
        }),
        "ordinary network traffic preserves the cached player raster"
    );
    // C4Network2's independent one-second timer refreshes connection and
    // resource telemetry in existing rows; HostPing does not reconstruct
    // C4PlayerInfoListBox (src/C4Network2.cpp:674-677;
    // src/C4Network2Dialogs.cpp:343-370).
    app.refresh_classic_lobby_client_telemetry();
    let lobby = app.network_lobby.test_mut();
    let host = lobby
        .roster_rows
        .iter()
        .find_map(|row| match row {
            LobbyRosterRow::Client(client) if client.id == 0 => Some(client),
            _ => None,
        })
        .test_value();
    assert!(host.connected);
    assert_eq!(host.ping_ms, Some(12));
    assert_eq!(host.resource_progress, Some(80));
    assert!(
        lobby.roster_rows.iter().any(|row| {
            matches!(
                row,
                LobbyRosterRow::Player(player)
                    if player.id == 41
                        && player.icon == LobbyRosterIcon::Raster(cached_player_raster.clone())
            )
        }),
        "the one-second telemetry refresh preserves the cached player raster"
    );
    lobby.sync_classic_controller();
    assert_eq!(lobby.controller.rows(), lobby.roster_rows.as_slice());
    let telemetry_rows = lobby.roster_rows.clone();

    // C4Network2ResDlg updates non-player transfer state independently;
    // completing an ordinary definition resource does not reconstruct
    // C4PlayerInfoListBox rows (src/C4GameLobby.cpp:766-797).
    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id: 88,
            core: clonk_engine::NetworkResourceCore {
                resource_type: clonk_network::HostResourceType::Definitions as u8,
                id: 88,
                loadable: true,
                ..Default::default()
            },
            path: PathBuf::from("Objects.c4d"),
            local: false,
        })
        .test_value();
    app.test_network_events();

    let lobby = app.network_lobby.test_ref();
    assert_eq!(lobby.roster_rows, telemetry_rows);
    assert_eq!(lobby.controller.rows(), telemetry_rows.as_slice());

    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id: 89,
            core: clonk_engine::NetworkResourceCore {
                resource_type: clonk_network::HostResourceType::Player as u8,
                id: 89,
                loadable: true,
                ..Default::default()
            },
            path: PathBuf::from("Player.c4p"),
            local: false,
        })
        .test_value();
    app.test_network_events();
    assert_ne!(
        app.network_lobby
            .as_ref()
            .expect("cached lobby fixture")
            .controller
            .rows(),
        cached_rows,
        "an NRT_Player completion still invalidates fallback/BigIcon rows"
    );

    {
        let lobby = app.network_lobby.test_mut();
        lobby.roster_rows.clone_from(&cached_rows);
        lobby.controller.set_rows(cached_rows.clone());
    }
    event_tx
        .send(NetworkEvent::ResourceLoadFailed { resource_id: 89 })
        .test_value();
    app.test_network_events();
    assert_ne!(
        app.network_lobby
            .as_ref()
            .expect("cached lobby fixture")
            .controller
            .rows(),
        cached_rows,
        "an NRT_Player failure still restores fallback-icon rows"
    );

    {
        let lobby = app.network_lobby.test_mut();
        lobby.roster_rows.clone_from(&cached_rows);
        lobby.controller.set_rows(cached_rows.clone());
    }
    // C4ControlClientJoin owns the corresponding explicit lobby callback
    // (src/C4Control.cpp:552-565), so lifecycle changes still invalidate
    // and replace the cached rows immediately.
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::ClientJoin(
            clonk_engine::ClientJoinControlData {
                core: message_client(3, b"Remote"),
                by_client: 0,
            },
        )))
        .test_value();
    app.test_network_events();

    let lobby = app.network_lobby.test_mut();
    lobby.sync_classic_controller();
    assert!(
        lobby
            .controller
            .rows()
            .iter()
            .any(|row| matches!(row, LobbyRosterRow::Client(client) if client.id == 3)),
        "the explicit client lifecycle callback refreshes the roster"
    );
    assert_eq!(lobby.roster_rows, lobby.controller.rows());
}

#[test]
fn classic_roster_sync_without_a_lobby_is_cpp_guarded_noop() {
    // Native lifecycle controls call MainDlg only while GetLobby returns a
    // live dialog (src/C4Control.cpp:563-565,670-672). After lobby teardown
    // there is no PlayerInfo-list projection or related UI mutation.
    let mut app = new_state_only_running_sandbox_app();
    app.set_context_menu_lobby_team_player(Some(41));
    assert!(app.classic_host_lobby.is_none());
    assert!(app.network_lobby.is_none());

    app.sync_classic_lobby_roster();

    assert_eq!(app.context_menu_lobby_team_player, Some(41));
}

#[test]
fn initial_network_client_registry_keeps_the_local_client_name() {
    let (manager, _event_tx) = NetworkManager::test_stub();
    let mode = NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name: "Named Host".to_string(),
        prepared: None,
    });
    let clients = initial_control_clients(Some(&manager), Some(&mode));

    assert_eq!(
        clients
            .state(0)
            .expect("local host client is registered")
            .name
            .to_string_lossy(),
        "Named Host"
    );
    assert_eq!(
        initial_control_clients(None, None)
            .state(0)
            .expect("offline local client is registered")
            .name
            .to_string_lossy(),
        "Local"
    );
}

#[test]
fn locally_authored_join_uses_filename_instead_of_embedded_data() {
    // LocalControl is selected solely by ByClient and loads Filename
    // before the embedded/resource branches (src/C4Control.cpp:43-46,
    // 726-744).
    let mut app = new_synthetic_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.engine.set_network_game(true);
    let tick = u32::try_from(app.engine.frame()).test_value();
    let player_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    );
    let info = clonk_engine::ControlPlayerInfoEntry {
        name: clonk_engine::LegacyCString::from_bytes(b"Local Tyler".to_vec()).test_value(),
        id: 8,
        ..Default::default()
    };
    let join = clonk_engine::JoinPlayerControlData {
        filename: clonk_engine::LegacyCString::from_bytes(player_path.as_bytes().to_vec())
            .test_value(),
        at_client: 0,
        info_id: 8,
        source: clonk_engine::JoinPlayerSource::Embedded(vec![0, 0]),
        by_client: 0,
    };
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: vec![
                NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    players: vec![info],
                    by_client: 0,
                    ..Default::default()
                }),
                NetworkControl::JoinPlayer(join),
            ],
        })
        .test_value();

    app.test_update();

    let joined = app
        .snapshot
        .players
        .iter()
        .find(|player| player.player_info_id == 8)
        .test_value();
    assert_eq!(joined.name, "Local Tyler");
    assert_eq!((joined.score, joined.total_playing_time), (42, 99));
    assert_eq!(
        app.local_controls.owner_for_set(1),
        Some(joined.id),
        "the joined file's missing Control field defaults to Keyboard2"
    );
}

#[test]
fn network_main_menu_is_local_and_clears_only_when_user_closes_it() {
    // COM_PlayerMenu and C4MainMenu navigation are local UI; closing the
    // menu queues only COM_ClearPressedComs for synchronized execution
    // (src/C4Game.cpp:3595-3615; src/C4MainMenu.cpp:319-329).
    let mut app = new_running_sandbox_app();
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let tick = u32::try_from(app.engine.frame()).test_value();
    let player_menu = ControlEvent::Command {
        command: ControlCommand::PlayerMenu,
        kind: CommandKind::Press,
    };

    app.dispatch_control_event(player_menu).test_value();
    assert!(app.ingame_menu.is_some(), "player menu opens immediately");
    assert!(
        commands.take_submitted_local().is_empty(),
        "opening C4MainMenu does not submit synchronized control"
    );

    app.dispatch_control_event(player_menu).test_value();
    assert!(app.ingame_menu.is_none(), "player menu closes immediately");
    assert_eq!(
        commands.take_submitted_local(),
        vec![(app.local_owner, ControlEvent::ClearPressed, tick)],
        "one user-driven close queues one clear for the still-open tick"
    );
}

#[test]
fn eliminated_owner_cannot_submit_cached_valid_construction_drop() {
    let (mut app, owner, menu_point, valid_point, _invalid, _world, _c4id) =
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

    app.engine
        .set_player_status(owner, PlayerStatus::Eliminated)
        .test_value();
    app.test_left_button(ElementState::Released);

    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    assert!(controls.is_empty());
    assert!(commands.is_empty());
    assert!(selections.is_empty());
    assert!(app.construction_menu_drag.is_none());
}

#[test]
fn network_cursor_menu_converts_exact_press_coms_before_submission() {
    // LocalPlayerControl applies the cursor menu's asynchronous
    // ConvertCom before Input.Add, so both the network packet and CtrlRec
    // contain menu coms rather than their raw gameplay inputs
    // (C4Game.cpp:3616-3623; C4Menu.cpp:1040-1069).
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    install_test_cursor_menu(&mut app, cursor, two_item_script_menu(cursor));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);

    for (raw, mapped, wire_com) in [
        (
            ControlEvent::Command {
                command: ControlCommand::Throw,
                kind: CommandKind::Press,
            },
            ControlCommand::MenuEnter,
            38,
        ),
        (
            ControlEvent::Command {
                command: ControlCommand::Dig,
                kind: CommandKind::Press,
            },
            ControlCommand::MenuClose,
            40,
        ),
        (
            ControlEvent::Command {
                command: ControlCommand::Special2,
                kind: CommandKind::Press,
            },
            ControlCommand::MenuEnterAll,
            39,
        ),
        (
            ControlEvent::Press(ControlButton::Left),
            ControlCommand::MenuLeft,
            52,
        ),
        (
            ControlEvent::Press(ControlButton::Right),
            ControlCommand::MenuRight,
            53,
        ),
        (
            ControlEvent::Press(ControlButton::Up),
            ControlCommand::MenuUp,
            54,
        ),
        (
            ControlEvent::Press(ControlButton::Down),
            ControlCommand::MenuDown,
            55,
        ),
    ] {
        let tick = app.local_control_submission_tick();
        app.dispatch_control_event_for_local_player(owner, raw)
            .test_value();
        let submitted = commands.take_submitted_local();
        assert_eq!(
            submitted,
            vec![(
                owner,
                ControlEvent::Command {
                    command: mapped,
                    kind: CommandKind::Press,
                },
                tick,
            )]
        );
        let packet = NetworkControl::Player {
            owner,
            event: submitted[0].1,
        }
        .into_packet()
        .test_value();
        let clonk_engine::ControlPacket::PlayerControl(packet) = packet else {
            panic!("cursor-menu event must encode as PlayerControl");
        };
        assert_eq!(packet.player, owner);
        assert_eq!(packet.command, wire_com);
        assert_eq!(packet.data, 0);
    }
}

#[test]
fn network_progressing_cursor_menu_submits_show_text_before_execution() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);
    let mut menu = two_item_script_menu(cursor);
    menu.text_progressing = true;
    for item in &mut menu.items {
        item.text_display_progress = 0;
    }
    install_test_cursor_menu(&mut app, cursor, menu);
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);

    app.dispatch_control_event_for_local_player(
        owner,
        ControlEvent::Command {
            command: ControlCommand::Throw,
            kind: CommandKind::Press,
        },
    )
    .test_value();
    let (_, event, tick) = commands.take_submitted_local().pop().test_value();
    assert_eq!(
        event,
        ControlEvent::Command {
            command: ControlCommand::MenuShowText,
            kind: CommandKind::Press,
        }
    );
    let packet = NetworkControl::Player { owner, event }
        .into_packet()
        .test_value();
    let clonk_engine::ControlPacket::PlayerControl(packet) = packet else {
        panic!("show-text event must encode as PlayerControl");
    };
    assert_eq!(packet.command, 42);
    assert!(
        app.engine
            .cursor_object_menu(owner)
            .is_some_and(|(_, menu)| menu.text_progressing),
        "submission alone must not reveal local-length text"
    );

    app.apply_ready_controls(tick, vec![NetworkControl::Player { owner, event }])
        .test_value();
    let (_, menu) = app.engine.cursor_object_menu(owner).test_value();
    assert_eq!(menu.selection, 0);
    assert!(!menu.text_progressing);
    assert!(menu
        .items
        .iter()
        .all(|item| item.text_display_progress == -1));
}

#[test]
fn runtime_network_role_requires_consistent_manager_identity_and_mode() {
    let mut app = new_state_only_running_sandbox_app();
    app.network = None;
    app.network_mode = None;
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Offline);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Offline);

    let (host_manager, _events) = NetworkManager::test_stub();
    app.network = Some(host_manager);
    app.network_mode = None;
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Ambiguous);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Ambiguous);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Host);

    let (client_manager, _events) = NetworkManager::test_stub_for_client_id(3);
    app.network = Some(client_manager);
    app.network_mode = None;
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Ambiguous);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Ambiguous);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    assert_eq!(app.runtime_network_role(), RuntimeNetworkRole::Client);
}

#[test]
fn saved_game_reapplies_current_player_info_identity_and_preferences() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let info_id = app.engine.test_player(owner).player_info_id();
    let text = |value: &str| {
        clonk_engine::LegacyCString::from_bytes(value.as_bytes().to_vec()).test_value()
    };
    app.control_player_infos.apply(netplay_player_info_data(
        0,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: info_id,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            name: text("Base name"),
            forced_name: text("Forced name"),
            league_account: clonk_engine::LegacyCString::from_bytes(b"League Ren\xe9".to_vec())
                .expect("valid native player name"),
            ..Default::default()
        }],
    ));
    let scenario = app
        .active_scenario
        .clone()
        .unwrap_or_else(FrontendScenario::fallback);
    let mut engine_state = app.engine.capture_state();
    let saved_player = engine_state
        .players
        .iter_mut()
        .find(|player| player.id == owner)
        .test_value();
    saved_player.name = "Stale saved name".to_string();
    saved_player.script_player = true;
    saved_player.no_elimination_check = true;
    saved_player.pref_control_style = true;
    saved_player.pref_auto_context_menu = true;
    let save = SavedGameFile {
        version: SAVE_FILE_VERSION,
        saved_at_seconds: 0,
        scenario: SavedScenarioInfo::from_frontend(
            &scenario,
            &app.scenario_label,
            app.fallback_ground,
        ),
        definition_load: app.active_definition_load.clone(),
        focus_id: app.focus_id,
        user_label: Some("current player info wins".to_string()),
        runtime_music_enabled: Some(app.runtime_music_enabled),
        source_save_player_infos: None,
        source_string_table: None,
        source_title_png: None,
        engine_state,
    };

    app.apply_loaded_game(save).test_value();

    let player = app.engine.test_player(owner);
    assert_eq!(
        clonk_script::c4_string_bytes(player.name()),
        b"League Ren\xe9"
    );
    assert_eq!(player.at_client_name(), "Local");
    assert!(!player.is_script_player());
    assert!(!player.no_elimination_check());
    assert_eq!(player.control_style_preferences(), (false, false));
}

#[test]
fn saved_game_promotes_unjoined_takeover_info_before_recreation_filter() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let saved_info_id = app.engine.test_player(owner).player_info_id();
    assert!(saved_info_id > 0);
    let text = |value: &str| {
        clonk_engine::LegacyCString::from_bytes(value.as_bytes().to_vec()).test_value()
    };
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    app.control_player_infos.apply(netplay_player_info_data(
        0,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: saved_info_id + 100,
            savegame_player: saved_info_id,
            name: text("Current takeover"),
            flags: 0,
            ..Default::default()
        }],
    ));
    let scenario = app
        .active_scenario
        .clone()
        .unwrap_or_else(FrontendScenario::fallback);
    let mut engine_state = app.engine.capture_state();
    let saved_player = engine_state
        .players
        .iter_mut()
        .find(|player| player.id == owner)
        .test_value();
    saved_player.name = "Saved identity".to_string();
    saved_player.no_elimination_check = true;
    let save = SavedGameFile {
        version: SAVE_FILE_VERSION,
        saved_at_seconds: 0,
        scenario: SavedScenarioInfo::from_frontend(
            &scenario,
            &app.scenario_label,
            app.fallback_ground,
        ),
        definition_load: app.active_definition_load.clone(),
        focus_id: app.focus_id,
        user_label: Some("unjoined takeover restore".to_string()),
        runtime_music_enabled: Some(app.runtime_music_enabled),
        source_save_player_infos: None,
        source_string_table: None,
        source_title_png: None,
        engine_state,
    };

    app.apply_loaded_game(save).test_value();

    let player = app.engine.test_player(owner);
    assert_eq!(player.player_info_id(), saved_info_id);
    assert_eq!(player.name(), "Current takeover");
    assert!(player.no_elimination_check());
    assert_eq!(
        app.control_player_infos.recreation_info_ids(),
        vec![saved_info_id]
    );
}

#[test]
fn network_restore_routes_associated_info_away_from_plain_join_queue() {
    let mut player_infos = ControlPlayerInfoRegistry::default();
    player_infos.replace_snapshot(
        92,
        [netplay_player_info_data(
            3,
            vec![
                clonk_engine::ControlPlayerInfoEntry {
                    id: 91,
                    savegame_player: 7,
                    ..Default::default()
                },
                clonk_engine::ControlPlayerInfoEntry {
                    id: 92,
                    player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                    ..Default::default()
                },
            ],
        )],
    );
    let restore = clonk_engine::ControlPlayerInfoEntry {
        id: 7,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
        color: 0x0012_3456,
        team: 2,
        ..Default::default()
    };
    let duplicate_restore = clonk_engine::ControlPlayerInfoEntry {
        id: restore.id,
        color: 0x00ff_0000,
        team: 9,
        ..restore.clone()
    };

    let plain_joins = player_infos.issue_unjoined_players(3, |_| None);
    let recreation =
        route_network_savegame_recreation(&mut player_infos, &[restore.clone(), duplicate_restore]);

    assert_eq!(recreation, vec![(3, 7)]);
    assert_eq!(plain_joins.len(), 1);
    assert_eq!(plain_joins[0].info_id, 92);
    let resumed = player_infos.get(7).test_value();
    assert_eq!((resumed.color, resumed.team), (restore.color, restore.team));
    assert_ne!(resumed.flags & clonk_engine::PLAYER_INFO_FLAG_JOINED, 0);
}

#[test]
fn ordinary_network_savegame_starts_associated_profile_wait_after_go_commit() {
    // Network::FinalInit reaches the GO barrier before InitPlayers enters
    // RecreatePlayers and starts RetrieveRes (C4Network2.cpp:558-616;
    // C4Game.cpp:459-483,2805-2850; C4PlayerInfo.cpp:1566-1576).
    let fixture = tempdir();
    let scenario_path = fixture.path().join("NetworkSave.c4s");
    fs::create_dir(&scenario_path).test_value();
    fs::write(scenario_path.join("Game.txt"), b"[Player7]\nStatus=1\nIndex=2\nID=7\nWealth=19\n\n[Player9]\nStatus=1\nIndex=3\nID=9\nWealth=23\n\n[Player11]\nStatus=1\nIndex=4\nID=11\nWealth=29\n").test_value();
    let player_path = fixture.path().join("Alice.c4p");
    fs::create_dir(&player_path).test_value();
    fs::write(
        player_path.join("Player.txt"),
        b"[Player]\nName=Alice profile\nScore=31\n",
    )
    .test_value();
    let script_player_path = fixture.path().join("ScriptResource.c4p");
    fs::create_dir(&script_player_path).test_value();
    fs::write(
        script_player_path.join("Player.txt"),
        b"[Player]\nName=Script resource profile\nScore=37\n",
    )
    .test_value();
    let embedded_script_path = scenario_path.join("ScriptPlr-11.c4p");
    fs::create_dir(&embedded_script_path).test_value();
    fs::write(
        embedded_script_path.join("Player.txt"),
        b"[Player]\nName=Embedded script profile\nScore=41\n",
    )
    .test_value();
    let mut app = new_running_sandbox_app();
    let provisional_owner = app.local_owner;
    app.remove_local_control_assignment(provisional_owner);
    app.engine.retain_restored_players([]);
    app.engine.set_local_players([]);
    app.mode = AppMode::Loading;
    let (network, events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    let native =
        |bytes: &[u8]| clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).test_value();
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: native(b"Current client"),
            ..Default::default()
        }]);
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: native(b"Alice.c4p"),
        ..Default::default()
    };
    let script_resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 19,
        loadable: true,
        filename: native(b"ScriptResource.c4p"),
        ..Default::default()
    };
    let current = clonk_engine::ControlPlayerInfoEntry {
        id: 91,
        savegame_player: 7,
        name: native(b"Alice"),
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        resource: Some(resource.clone()),
        ..Default::default()
    };
    let script_current = clonk_engine::ControlPlayerInfoEntry {
        id: 92,
        savegame_player: 9,
        name: native(b"Script resource"),
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        resource: Some(script_resource.clone()),
        ..Default::default()
    };
    let embedded_script_current = clonk_engine::ControlPlayerInfoEntry {
        id: 93,
        savegame_player: 11,
        name: native(b"Embedded script"),
        filename: native(b"ScriptPlr-11.c4p"),
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        resource: Some(clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 20,
            loadable: true,
            filename: native(b"StaleScriptResource.c4p"),
            ..Default::default()
        }),
        ..Default::default()
    };
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    app.control_player_infos.replace_snapshot(
        script_current.id,
        [netplay_player_info_data(
            0,
            vec![
                current.clone(),
                script_current.clone(),
                embedded_script_current,
            ],
        )],
    );
    app.admission_resources
        .register_player_info_resources(std::slice::from_ref(&current));
    app.admission_resources
        .register_player_info_resources(std::slice::from_ref(&script_current));
    let (_sender, receiver) = mpsc::channel();
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(scenario_path);
    app.loading_state = Some(test_loading_state(
        scenario,
        receiver,
        true,
        test_prepared_go(
            0,
            false,
            true,
            false,
            vec![
                clonk_engine::ControlPlayerInfoEntry {
                    id: 7,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                    ..Default::default()
                },
                clonk_engine::ControlPlayerInfoEntry {
                    id: 11,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                    ..Default::default()
                },
                clonk_engine::ControlPlayerInfoEntry {
                    id: 9,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                    ..Default::default()
                },
            ],
            Vec::new(),
            Vec::new(),
        ),
    ));

    app.try_reach_loaded_network_go_barrier().test_value();

    assert_eq!(commands.take_status_reached(), 1);
    assert_eq!(
        app.loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| prepared.local_reached),
        Some(true)
    );
    assert!(app.blocking_resource_wait.is_none());

    app.handle_status_committed(clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: 0,
        target_tick: 0,
    })
    .test_value();

    assert!(app.blocking_resource_wait.as_ref().is_some_and(|wait| {
        wait.scope == BlockingResourceScope::PlayerJoin
            && wait.resource_id == resource.id
            && wait.player_info_id == Some(7)
    }));
    assert_eq!(app.mode, AppMode::Loading);
    // RecreatePlayerFiles stages every embedded script profile and discards
    // its live resource before RecreatePlayers reaches the first user pRes
    // wait (C4Game.cpp:2841-2847; C4PlayerInfo.cpp:1448-1521).
    let staged_script = app.control_player_infos.get(11).test_value();
    assert!(staged_script.resource.is_none());
    assert_eq!(
        staged_script.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0
    );

    app.admission_resources
        .mark_complete(resource.id, player_path);
    app.clear_blocking_resource_wait();
    app.poll_loading().test_value();

    // A script player whose embedded extraction is absent still uses its live
    // pRes before Filename and waits in RecreatePlayers when that resource is
    // loading (C4PlayerInfo.cpp:124-130,1460-1512,1566-1577).
    assert!(app.blocking_resource_wait.as_ref().is_some_and(|wait| {
        wait.scope == BlockingResourceScope::PlayerJoin
            && wait.resource_id == script_resource.id
            && wait.player_info_id == Some(9)
    }));
    assert_eq!(app.mode, AppMode::Loading);
    let user = app.engine.test_player(2);
    assert_eq!(
        (user.player_info_id(), user.wealth(), user.score()),
        (7, 19, 31)
    );
    assert!(app.engine.player(3).is_none());

    app.poll_loading().test_value();
    assert_eq!(
        app.engine
            .players()
            .filter(|player| player.player_info_id() == 7)
            .count(),
        1
    );
    let ready_tick = app.expected_network_control_tick();
    events
        .send(NetworkEvent::ReadyTick {
            tick: ready_tick,
            controls: Vec::new(),
        })
        .test_value();
    app.test_network_events();
    assert!(app.network_ticks.ready.contains_key(&ready_tick));

    app.admission_resources
        .mark_complete(script_resource.id, script_player_path);
    app.clear_blocking_resource_wait();
    app.poll_loading().test_value();

    assert_eq!(app.mode, AppMode::Running);
    let player = app.engine.test_player(2);
    assert_eq!(
        (player.player_info_id(), player.wealth(), player.score()),
        (7, 19, 31)
    );
    let script = app.engine.test_player(3);
    assert_eq!(
        (script.player_info_id(), script.wealth(), script.score()),
        (9, 23, 37)
    );
    assert!(script.is_script_player());
    let embedded = app.engine.test_player(4);
    assert_eq!(
        (
            embedded.player_info_id(),
            embedded.wealth(),
            embedded.score()
        ),
        (11, 29, 41)
    );
    assert!(embedded.is_script_player());
}

#[test]
fn change_to_local_during_savegame_recreation_retains_completed_profile_files() {
    // ChangeToLocal clears the network while RecreatePlayers remains on its
    // native stack: completed pRes files stay usable and pending retrievals
    // fail (C4GameControl.cpp:93-127; C4PlayerInfo.cpp:1566-1577).
    let fixture = tempdir();
    let completed_path = fixture.path().join("Complete.c4p");
    fs::create_dir(&completed_path).test_value();
    let native =
        |bytes: &[u8]| clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let completed = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 31,
        loadable: true,
        filename: native(b"Complete.c4p"),
        ..Default::default()
    };
    let pending = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 32,
        loadable: true,
        filename: native(b"Pending.c4p"),
        ..Default::default()
    };
    let mut app = new_running_sandbox_app();
    let (network, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(network);
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 7,
            activated: true,
            name: native(b"Local survivor"),
            nick: native(b"Survivor nick"),
            ..Default::default()
        }]);
    app.network_savegame_recreation_progress = Some(NetworkSavegameRecreationProgress::default());
    app.admission_resources.register_lobby_resource(&completed);
    app.admission_resources
        .mark_complete(completed.id, completed_path.clone());
    app.admission_resources.register_lobby_resource(&pending);

    app.change_network_control_to_local(7);

    assert_eq!(
        app.admission_resources.complete_path(completed.id),
        Some(completed_path.as_path())
    );
    assert!(matches!(
        app.admission_resources.status(pending.id),
        Some(AdmissionResourceState::Unavailable(
            AdmissionResourceUnavailable::TransferFailed
        ))
    ));
    let retained_client = app.control_clients.state(7).test_value();
    assert_eq!(retained_client.name.as_bytes(), b"Local survivor");
    assert_eq!(retained_client.nick.as_bytes(), b"Survivor nick");
}

#[test]
fn ordinary_network_savegame_recreates_associated_user_player_with_local_control() {
    // Ordinary C++ savegame startup associates RestorePlayerInfos, reloads the
    // current lobby profile, then joins the saved runtime player and reruns
    // InitControl (C4Game.cpp:2827-2850; C4PlayerInfo.cpp:1524-1607;
    // C4Player.cpp:354-386).
    let fixture = tempdir();
    let scenario_path = fixture.path().join("NetworkSave.c4s");
    fs::create_dir(&scenario_path).test_value();
    fs::write(scenario_path.join("Game.txt"), b"[Player7]\nStatus=1\nAtClient=9\nAtClientName=stale\nIndex=2\nID=7\nWealth=19\n\n[Player9]\nStatus=1\nIndex=4\nID=9\n\n[Player10]\nStatus=1\nIndex=5\nID=10\nWealth=29\n\n[Player8]\nStatus=1\nIndex=3\nID=8\nWealth=23\n\n[Player12]\nStatus=1\nIndex=6\nID=12\n").test_value();
    let player_path = fixture.path().join("Alice.c4p");
    fs::create_dir(&player_path).test_value();
    fs::write(
        player_path.join("Player.txt"),
        b"[Player]\nName=Alice profile\nScore=31\n[Preferences]\nControl=0\nMouse=0\n",
    )
    .test_value();
    let filename_player_path = fixture.path().join("Bob.c4p");
    fs::create_dir(&filename_player_path).test_value();
    fs::write(
        filename_player_path.join("Player.txt"),
        b"[Player]\nName=Bob profile\nScore=37\n[Preferences]\nControl=0\nMouse=0\n",
    )
    .test_value();
    let script_player_path = scenario_path.join("ScriptPlr-10.c4p");
    fs::create_dir(&script_player_path).test_value();
    fs::write(
        script_player_path.join("Player.txt"),
        b"[Player]\nName=Script profile\nScore=41\n",
    )
    .test_value();
    let installed_script_player_path = fixture.path().join("InstalledScript.c4p");
    fs::create_dir(&installed_script_player_path).test_value();
    fs::write(
        installed_script_player_path.join("Player.txt"),
        b"[Player]\nName=Installed script profile\nScore=43\n",
    )
    .test_value();
    let packed_scenario_path = fixture.path().join("PackedNetworkSave.c4s");
    let scenario_group = Group::open(&scenario_path).test_value();
    fs::write(
        &packed_scenario_path,
        MutableGroup::from_group(&scenario_group)
            .test_value()
            .pack()
            .test_value(),
    )
    .test_value();

    let native =
        |bytes: &[u8]| clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: native(b"Alice.c4p"),
        ..Default::default()
    };
    let installed_script_resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 19,
        loadable: true,
        filename: native(b"InstalledScript.c4p"),
        ..Default::default()
    };
    let unavailable_filename_resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 21,
        loadable: false,
        filename: native(b"MissingPublishedBob.c4p"),
        ..Default::default()
    };
    let current = clonk_engine::ControlPlayerInfoEntry {
        id: 91,
        savegame_player: 7,
        name: native(b"Alice current"),
        filename: native(b"Alice.c4p"),
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        resource: Some(resource.clone()),
        ..Default::default()
    };
    let restore = clonk_engine::ControlPlayerInfoEntry {
        id: 7,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        ..Default::default()
    };
    let filename_current = clonk_engine::ControlPlayerInfoEntry {
        id: 92,
        savegame_player: 8,
        name: native(b"Bob current"),
        filename: clonk_engine::LegacyCString::from_bytes(clonk_resources::path_to_legacy_bytes(
            &filename_player_path,
        ))
        .test_value(),
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        resource: Some(unavailable_filename_resource),
        ..Default::default()
    };
    let filename_restore = clonk_engine::ControlPlayerInfoEntry {
        id: 8,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        ..Default::default()
    };
    let missing_filename_current = clonk_engine::ControlPlayerInfoEntry {
        id: 93,
        savegame_player: 12,
        name: native(b"Missing filename user"),
        filename: clonk_engine::LegacyCString::from_bytes(clonk_resources::path_to_legacy_bytes(
            &fixture.path().join("MissingUser.c4p"),
        ))
        .test_value(),
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        ..Default::default()
    };
    let missing_filename_restore = clonk_engine::ControlPlayerInfoEntry {
        id: 12,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        ..Default::default()
    };
    let missing_script_current = clonk_engine::ControlPlayerInfoEntry {
        id: 9,
        savegame_player: 9,
        name: native(b"Installed script player"),
        filename: native(b"wrong-installed-script-path.c4p"),
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        resource: Some(installed_script_resource.clone()),
        ..Default::default()
    };
    let missing_script_restore = clonk_engine::ControlPlayerInfoEntry {
        id: 9,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        ..Default::default()
    };
    let script_current = clonk_engine::ControlPlayerInfoEntry {
        id: 10,
        savegame_player: 10,
        name: native(b"Current script player"),
        filename: native(b"ScriptPlr-10.c4p"),
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        resource: Some(clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 18,
            loadable: true,
            filename: native(b"ScriptPlr-10.c4p"),
            ..Default::default()
        }),
        ..Default::default()
    };
    let script_restore = clonk_engine::ControlPlayerInfoEntry {
        id: 10,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        ..Default::default()
    };

    let mut app = new_state_only_running_sandbox_app();
    app.engine
        .register_test_definition(test_definition("UCRW", "Unassociated crew", ""));
    let provisional_owner = app.local_owner;
    app.remove_local_control_assignment(provisional_owner);
    app.engine.retain_restored_players([]);
    app.engine.set_local_players([]);
    let unassociated_crew = app.engine.spawn_test_object(
        SpawnConfig::new("UCRW")
            .with_owner(6)
            .with_category(1 << 18),
    );
    let (network, _events) = NetworkManager::test_stub();
    app.network = Some(network);
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: native(b"Current client"),
            ..Default::default()
        }]);
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    app.control_player_infos.replace_snapshot(
        missing_filename_current.id,
        [netplay_player_info_data(
            0,
            vec![
                current.clone(),
                missing_script_current.clone(),
                script_current,
                filename_current,
                missing_filename_current,
            ],
        )],
    );
    app.admission_resources
        .register_player_info_resources(std::slice::from_ref(&current));
    app.admission_resources
        .register_player_info_resources(std::slice::from_ref(&missing_script_current));
    app.admission_resources
        .mark_complete(resource.id, player_path);
    app.admission_resources
        .mark_complete(installed_script_resource.id, installed_script_player_path);
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(packed_scenario_path);
    let (_sender, receiver) = mpsc::channel();
    app.loading_state = Some(test_loading_state(
        scenario,
        receiver,
        false,
        test_prepared_go(
            0,
            true,
            true,
            false,
            vec![
                restore,
                missing_script_restore,
                script_restore,
                filename_restore,
                missing_filename_restore,
                clonk_engine::ControlPlayerInfoEntry {
                    id: 11,
                    game_number: 6,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                    ..Default::default()
                },
            ],
            Vec::new(),
            Vec::new(),
        ),
    ));
    let record_path = fixture.path().join("001-RecreatedPlayers.c4s");
    install_test_recording_template(&mut app, record_path.clone());
    app.start_recording(true).test_value();

    app.finalize_network_loaded_scenario(true).test_value();

    let player = app.engine.test_player(2);
    assert_eq!((player.player_info_id(), player.wealth()), (7, 19));
    assert_eq!((player.name(), player.score()), ("Alice current", 31));
    assert_eq!(player.at_client(), clonk_engine::PlayerAtClient::HOST);
    assert_eq!(player.at_client_name(), "Current client");
    let filename_player = app.engine.test_player(3);
    // LoadResource clears an unavailable pRes but retains Filename, which
    // GetLocalJoinFilename then uses for RecreatePlayers
    // (C4PlayerInfo.cpp:124-130,275-292,1566-1601).
    assert_eq!(
        (
            filename_player.player_info_id(),
            filename_player.name(),
            filename_player.wealth(),
            filename_player.score(),
        ),
        (8, "Bob current", 23, 37)
    );
    let filename_info = app.control_player_infos.get(8).test_value();
    assert!(filename_info.resource.is_none());
    assert_eq!(
        filename_info.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0
    );
    let installed_script = app.engine.test_player(4);
    assert_eq!(
        (
            installed_script.player_info_id(),
            installed_script.name(),
            installed_script.score(),
        ),
        (9, "Installed script player", 43)
    );
    assert!(installed_script.is_script_player());
    let script_player = app.engine.test_player(5);
    assert_eq!(
        (
            script_player.player_info_id(),
            script_player.name(),
            script_player.wealth(),
            script_player.score(),
        ),
        (10, "Current script player", 29, 41)
    );
    assert!(script_player.is_script_player());
    // GetLocalJoinFilename returns a nonempty raw filename even when the file
    // does not exist. RecreatePlayers attempts that join, whose failure marks
    // the retained info Removed (C4PlayerInfo.cpp:124-130,1566-1603;
    // C4PlayerList.cpp:302-314).
    assert!(app.engine.player(6).is_none());
    assert!(app
        .control_player_infos
        .get(12)
        .is_some_and(|info| { info.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0 }));
    assert!(app
        .engine
        .snapshot()
        .round_results
        .players
        .iter()
        .any(|result| result.player_info_id == 12 && result.score_old == 0));
    assert_eq!(
        app.engine
            .object_snapshot(unassociated_crew)
            .expect("unassociated crew tombstone")
            .status,
        clonk_engine::ObjectStatus::Deleted,
        "RemoveUnassociatedPlayers removes raw-category crew before recreation"
    );
    let script_info = app.control_player_infos.get(10).test_value();
    // RecreatePlayerFiles extracts script profiles to a temporary path and
    // DeleteTempFile clears that filename/resource after the attempted join
    // (C4PlayerInfo.cpp:1460-1504,1601-1603).
    assert!(script_info.filename.is_empty());
    assert!(script_info.resource.is_none());
    assert_eq!(
        script_info.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0
    );
    assert!(app.local_controls.assignment(2).is_some());
    assert!(app.local_controls.assignment(3).is_some());
    assert_eq!(app.engine.snapshot().hud.local_players, vec![2, 3]);
    assert_eq!(app.local_owner, 2);
    assert!(app.finish_recording().is_none());
    let record = Group::open(record_path).test_value();
    // RecreatePlayers records every filename-backed profile, including
    // embedded script players that the initial record cleanup removed
    // (C4PlayerInfo.cpp:1594-1598).
    assert!(record
        .open_child("Recreate-10.c4p")
        .expect("recorded embedded script profile")
        .exists("Player.txt"));
}

#[test]
fn regular_network_scenario_recreates_fileless_script_player_without_runtime_data() {
    // A non-savegame script player may have no [Player<ID>] runtime section;
    // C4Player::Init then retains the restore-info defaults instead of
    // failing recreation (C4Player.cpp:355-371).
    let fixture = tempdir();
    let scenario_path = fixture.path().join("NetworkScenario.c4s");
    fs::create_dir(&scenario_path).test_value();
    let native =
        |bytes: &[u8]| clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let restore = clonk_engine::ControlPlayerInfoEntry {
        id: 7,
        name: native(b"Scenario script player"),
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        game_number: 10,
        ..Default::default()
    };
    let mut current = restore.clone();
    current.savegame_player = restore.id;

    let mut app = new_state_only_running_sandbox_app();
    let provisional_owner = app.local_owner;
    app.remove_local_control_assignment(provisional_owner);
    app.engine.retain_restored_players([]);
    app.engine.set_local_players([]);
    let (network, _events) = NetworkManager::test_stub();
    app.network = Some(network);
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: native(b"Current client"),
            ..Default::default()
        }]);
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    app.control_player_infos
        .replace_snapshot(current.id, [netplay_player_info_data(0, vec![current])]);
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(scenario_path);
    let (_sender, receiver) = mpsc::channel();
    app.loading_state = Some(test_loading_state(
        scenario,
        receiver,
        false,
        test_prepared_go(0, true, false, false, vec![restore], Vec::new(), Vec::new()),
    ));

    app.finalize_network_loaded_scenario(false).test_value();

    let player = app.engine.test_player(10);
    assert!(player.is_script_player());
    assert_eq!(player.player_info_id(), 7);
    assert!(app.engine.snapshot().hud.local_players.is_empty());
    assert!(app.local_controls.assignment(10).is_none());
}

#[test]
fn dragon_rock_network_restore_makes_script_npcs_hostile_to_joined_users() {
    // Dragon Rock's SavePlayerInfos restores script player 10 on team 2 before
    // the synchronized user JoinPlayer runs. Native RecreatePlayers owns that
    // order, and the later ScenarioInit writes mutual team hostility
    // (C4Game.cpp:2827-2863; C4Player.cpp:756-757,1022-1033).
    let fixture = tempdir();
    let scenario_path = fixture.path().join("Drachenfels.c4s");
    let script_player_path = scenario_path.join("ScriptPlr-1.c4p");
    fs::create_dir_all(&script_player_path).test_value();
    fs::write(
        script_player_path.join("Player.txt"),
        b"[Player]\nName=Ala Kadabra\n",
    )
    .test_value();
    let native =
        |bytes: &[u8]| clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let restore = clonk_engine::ControlPlayerInfoEntry {
        id: 1,
        name: native(b"Ala Kadabra"),
        filename: native(b"ScriptPlr-1.c4p"),
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
            | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT
            | clonk_engine::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        game_number: 10,
        team: 2,
        ..Default::default()
    };
    let mut current_script = restore.clone();
    current_script.savegame_player = restore.id;
    let user_info_id = 2;
    let current_user = clonk_engine::ControlPlayerInfoEntry {
        id: user_info_id,
        name: native(b"Network user"),
        team: 1,
        ..Default::default()
    };

    let mut app = new_state_only_running_sandbox_app();
    let provisional_owner = app.local_owner;
    app.remove_local_control_assignment(provisional_owner);
    app.engine.retain_restored_players([]);
    app.engine.set_local_players([]);
    let (network, _events) = NetworkManager::test_stub_for_client_id(3);
    app.network = Some(network);
    app.control_clients.replace_snapshot([
        clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: native(b"Host"),
            ..Default::default()
        },
        clonk_engine::ClientCoreControlData {
            client_id: 3,
            activated: true,
            name: native(b"Network client"),
            ..Default::default()
        },
    ]);
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    app.control_player_infos.replace_snapshot(
        user_info_id,
        [
            netplay_player_info_data(0, vec![current_script]),
            netplay_player_info_data(3, vec![current_user]),
        ],
    );
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(scenario_path);
    let (_sender, receiver) = mpsc::channel();
    app.loading_state = Some(test_loading_state(
        scenario,
        receiver,
        false,
        test_prepared_go(2, true, false, false, vec![restore], Vec::new(), Vec::new()),
    ));

    app.finalize_network_loaded_scenario(false).test_value();
    let packed_player = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ))
    .test_value();
    app.apply_join_player_control(clonk_engine::JoinPlayerControlData {
        filename: native(b"NetworkUser.c4p"),
        at_client: 3,
        info_id: user_info_id,
        source: clonk_engine::JoinPlayerSource::Embedded(packed_player),
        by_client: 0,
    })
    .test_value();

    let user = app
        .engine
        .players()
        .find(|player| player.player_info_id() == user_info_id)
        .test_value();
    let script = app.engine.test_player(10);
    assert_eq!((user.team(), script.team()), (Some(1), Some(2)));
    assert!(user.is_hostile_towards(10));
    assert!(script.is_hostile_towards(user.id()));
}

#[test]
fn network_savegame_finalization_does_not_rerun_scenario_initialize() {
    let install_probe = |app: &mut GameApp| {
        app.engine.clear_scenario_script();
        app.engine
            .load_scenario_script_with_convention(
                "SavegameInitializeProbe.c",
                "#strict 3\nfunc Initialize() { SetGravity(77); }",
                true,
            )
            .test_value();
    };

    let mut savegame = new_state_only_running_sandbox_app();
    let restored_gravity = savegame.engine.physics().gravity;
    assert_ne!(restored_gravity, 77);
    install_probe(&mut savegame);
    savegame.finalize_network_loaded_scenario(true).test_value();
    assert_eq!(
        savegame.engine.physics().gravity,
        restored_gravity,
        "C4Game::InitGameFinal skips Script.Initialize for savegames"
    );

    let mut fresh = new_state_only_running_sandbox_app();
    install_probe(&mut fresh);
    fresh.finalize_network_loaded_scenario(false).test_value();
    assert_eq!(fresh.engine.physics().gravity, 77);
}

#[test]
fn runtime_network_client_join_reaches_running_render_after_dynamic_sync() {
    // A client that connects after GS_Go receives JoinData only after the
    // host's synchronized dynamic capture. It must then pass FinalInit,
    // reach/commit the GO barrier, execute the synchronized activation, and
    // render a live world instead of remaining on a frozen loader or lobby
    // (C4Network2.cpp:1099-1116,1574-1623,1820-1850,2017-2059,2081-2114;
    // C4Game.cpp:455-483,2805-2863).
    let mut host = new_state_only_running_sandbox_app();
    let (host_manager, host_event_tx, mut host_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(0);
    host.network = Some(host_manager);
    host.network_mode = Some(NetworkMode::Host(host_network_settings()));
    host.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    host_event_tx
        .send(NetworkEvent::JoinDataNeeded {
            client_id: 7,
            current_control_tick: 23,
        })
        .test_value();
    host.process_network_events().test_value();
    assert_eq!(
        host_commands.take_submitted_decided_controls(),
        vec![(
            23,
            clonk_engine::ControlPacket::Synchronize(clonk_engine::SynchronizeControlData {
                save_player_files: false,
                sync_clearance: true,
                by_client: 0,
            },),
            true,
        ),],
        "C++ SendJoinData must request one synchronized dynamic capture"
    );

    let directory = tempdir();
    let scenario_path = directory.path().join("Scenario.c4s");
    let dynamic_path = directory.path().join("Dynamic.c4s");
    let mut scenario_group = MutableGroup::new("Scenario.c4s");
    scenario_group
        .add_file(
            "Scenario.txt",
            b"[Head]\nTitle=Runtime join\nNetworkGame=1\nNetworkRuntimeJoin=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n"
                .to_vec(),
        )
        .test_value();
    scenario_group
        .add_child("Defs.c4d", packed_network_definition("Defs.c4d", "CLNK"))
        .test_value();
    fs::write(&scenario_path, scenario_group.pack().test_value()).test_value();

    let player_info = clonk_engine::ControlPlayerInfoEntry {
        id: 7,
        name: LegacyCString::from_bytes(b"Late client".to_vec()).test_value(),
        filename: LegacyCString::from_bytes(b"Late.c4p".to_vec()).test_value(),
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        ..Default::default()
    };
    let restore_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: player_info.id,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 7,
            flags: 0,
            players: vec![player_info.clone()],
        }],
    };
    let mut dynamic_group = MutableGroup::new("Dynamic.c4s");
    dynamic_group
        .add_file(
            "Game.txt",
            b"[Player7]\nStatus=1\nIndex=7\nID=7\nWealth=19\n".to_vec(),
        )
        .test_value();
    dynamic_group
        .add_file(
            "SavePlayerInfos.txt",
            clonk_network::encode_player_info_list_ini(&restore_infos).test_value(),
        )
        .test_value();
    let mut player_group = MutableGroup::new("Late.c4p");
    player_group
        .add_file(
            "Player.txt",
            b"[Player]\nName=Late client\n[Preferences]\nControl=0\nMouse=0\n".to_vec(),
        )
        .test_value();
    dynamic_group
        .add_child("Late.c4p", player_group)
        .test_value();
    fs::write(&dynamic_path, dynamic_group.pack().test_value()).test_value();

    let resource = |resource_type: clonk_network::HostResourceType, id: i32, filename: &[u8]| {
        clonk_engine::NetworkResourceCore {
            resource_type: resource_type as u8,
            id,
            loadable: true,
            filename: LegacyCString::from_bytes(filename.to_vec()).test_value(),
            ..Default::default()
        }
    };
    let scenario = resource(
        clonk_network::HostResourceType::Scenario,
        70,
        b"Scenario.c4s",
    );
    let dynamic = resource(clonk_network::HostResourceType::Dynamic, 71, b"Dynamic.c4s");
    let mut parameters = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value()
        .parameters;
    parameters.scenario = scenario.clone();
    parameters.clients.clients = vec![
        clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: LegacyCString::from_bytes(b"Host".to_vec()).test_value(),
            ..Default::default()
        },
        clonk_engine::ClientCoreControlData {
            client_id: 7,
            activated: false,
            name: LegacyCString::from_bytes(b"Late client".to_vec()).test_value(),
            ..Default::default()
        },
    ];
    parameters.clients.local_client_id = Some(7);
    parameters.player_infos = restore_infos.clone();
    parameters.restore_player_infos = restore_infos.clone();
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 23,
        status: clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: -1,
        },
        dynamic: dynamic.clone(),
        parameters,
    };
    let go = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: 2,
        target_tick: 23,
    };

    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let mut settings = ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client");
    settings.resource_directory = directory.path().to_path_buf();
    app.network_mode = Some(NetworkMode::Client(settings));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    app.startup_view = StartupView::NetworkLobby;

    event_tx
        .send(NetworkEvent::JoinData(join_data.clone()))
        .test_value();
    app.process_network_events().test_value();
    assert_eq!(
        commands.take_player_info_updates().len(),
        1,
        "JoinData must submit the local client's initial player-info packet before loading"
    );
    // The worker emits the transport connection notification after JoinData;
    // it updates lobby/start-wait presentation only and must not substitute
    // for the JoinData/resource/control transitions
    // (C4Network2.cpp:1574-1623; C4GameLobby.cpp:669-675).
    event_tx
        .send(NetworkEvent::PeerConnected {
            client_id: 7,
            name: "Client".to_string(),
            kind: clonk_network::ParticipantKind::Player,
        })
        .test_value();
    app.process_network_events().test_value();
    let removal_observer = thread::spawn(move || {
        let (resource_id, completion) = commands.receive_resource_removal();
        assert_eq!(resource_id, 71);
        completion.send(Ok(())).test_value();
        commands
    });
    // The chase status can arrive before either synchronized resource. The
    // C++ client records that target and only enters InitGame after the
    // scenario/dynamic retrievals complete (C4Network2.cpp:2017-2059;
    // C4Game.cpp:2551-2598).
    event_tx
        .send(NetworkEvent::StatusRequested(go))
        .test_value();
    app.process_network_events().test_value();
    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id: scenario.id,
            core: scenario.clone(),
            path: scenario_path,
            local: false,
        })
        .test_value();
    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id: dynamic.id,
            core: dynamic.clone(),
            path: dynamic_path,
            local: false,
        })
        .test_value();
    app.process_network_events().test_value();
    let mut commands = removal_observer.test_join();

    // The network worker's asynchronous loader is not the transition under
    // test. Once RetrieveScenario has produced Combined7.c4s, load that
    // exact artifact synchronously so this regression has no scheduler or
    // sleep dependency.
    let combined_path = directory.path().join("Combined7.c4s");
    assert!(combined_path.exists(), "runtime JoinData was not merged");
    let scenario_data = clonk_engine::Scenario::load_from_path_with(
        &combined_path,
        &InstallDefinitionResolver::new(None),
    )
    .test_value();
    let frontend = app.loading_state.as_ref().test_ref().scenario.clone();
    {
        let loading = app.loading_state.as_mut().test_value();
        let prepared = loading.prepared_go.as_mut().test_value();
        prepared.save_game = false;
        loading.finished = true;
    }
    app.activate_loaded_scenario(frontend, &scenario_data)
        .test_value();
    let prepared = app
        .loading_state
        .as_ref()
        .and_then(|loading| loading.prepared_go.as_ref())
        .test_value();
    assert!(prepared.network_runtime_join);
    assert_eq!(prepared.runtime_join_players.len(), 1);
    // `poll_loading` restores the prepared network state after activation
    // before it attempts the C++ FinalInit/GO barrier.
    app.mode = AppMode::Loading;
    app.try_reach_loaded_network_go_barrier().test_value();
    assert!(matches!(app.mode, AppMode::Loading));
    assert_eq!(commands.take_framed_status_acknowledgements().len(), 1);

    event_tx
        .send(NetworkEvent::StatusCommitted(go))
        .test_value();
    app.process_network_events().test_value();
    assert!(matches!(app.mode, AppMode::Running));
    assert!(app.network_control_running);
    assert!(app.network_start_wait.is_none());
    assert!(app.local_controls.assignment(7).is_some());
    assert!(!app.control_clients.is_activated(7));

    let initial_frame = app.engine.frame();
    let activate = clonk_engine::ClientUpdateControlData {
        update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
        client_id: 7,
        data: 1,
        by_client: 0,
    };
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 23,
            controls: vec![NetworkControl::ClientUpdate(activate.clone())],
        })
        .test_value();
    app.update().test_value();
    assert_eq!(app.engine.frame(), initial_frame + 1);
    assert!(app.control_clients.is_activated(7));
    assert_eq!(commands.take_executed_client_updates(), vec![activate]);

    let mut frame = vec![0x4c; 320 * 200 * 4];
    assert!(app.test_render(&mut frame));
    assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn runtime_join_combined_save_recreates_players_in_save_player_info_order() {
    let mut app = new_state_only_running_sandbox_app();
    let object = app
        .engine
        .capture_state()
        .objects
        .first()
        .test_value()
        .snapshot
        .id;
    app.engine.retain_restored_players([]);
    let native =
        |bytes: &[u8]| clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let player_info =
        |id: i32, filename: &[u8], name: &[u8]| clonk_engine::ControlPlayerInfoEntry {
            id,
            filename: native(filename),
            name: native(name),
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            ..Default::default()
        };
    let first = player_info(11, b"First.c4p", b"First player");
    let second = player_info(22, b"Second.c4p", b"Second player");
    let departed = player_info(33, b"Missing.c4p", b"Departed player");

    let mut first_group = MutableGroup::new("First.c4p");
    first_group
        .add_file(
            "Player.txt",
            b"[Player]\nName=First profile\nScore=111\n[Preferences]\nControl=0\nMouse=0\n"
                .to_vec(),
        )
        .test_value();
    let mut second_group = MutableGroup::new("Second.c4p");
    second_group
        .add_file(
            "Player.txt",
            b"[Player]\nName=Second profile\nScore=222\n[Preferences]\nControl=0\nMouse=0\n"
                .to_vec(),
        )
        .test_value();
    let mut crew_group = MutableGroup::new("Veteran.c4i");
    crew_group
        .add_file(
            "ObjectInfo.txt",
            b"[ObjectInfo]\nid=CLNK\nName=Veteran\nRank=2\n".to_vec(),
        )
        .test_value();
    second_group
        .add_child("Veteran.c4i", crew_group)
        .test_value();

    let game_txt = format!(
                "[Player11]\r\nStatus=1\r\nAtClient=71\r\nAtClientName=stale first\r\nIndex=7\r\nID=11\r\nWealth=1111\r\n\r\n[Player22]\r\nStatus=1\r\nAtClient=72\r\nAtClientName=stale second\r\nIndex=5\r\nID=22\r\nWealth=2222\r\nMsgBoardQueries=({},\"object survives\",1)\r\n",
                object.as_u64()
            );
    let combined_dir = tempdir();
    let combined_path = combined_dir.path().join("Combined.c4s");
    let mut combined = MutableGroup::new("Combined.c4s");
    combined
        .add_file("Game.txt", game_txt.into_bytes())
        .test_value();
    combined.add_child("First.c4p", first_group).test_value();
    combined.add_child("Second.c4p", second_group).test_value();
    fs::write(&combined_path, combined.pack().test_value()).test_value();

    let (network, _events) = NetworkManager::test_stub();
    app.network = Some(network);
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: native(b"Current client"),
            ..Default::default()
        }]);
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    let mut current_first = first.clone();
    // Parameters may still carry a savegame-takeover association. The
    // exclusive NetworkRuntimeJoin branch must not reinterpret it against
    // the dynamic-local SavePlayerInfos list.
    current_first.savegame_player = second.id;
    app.control_player_infos.replace_snapshot(
        22,
        [netplay_player_info_data(
            0,
            vec![current_first, second.clone()],
        )],
    );
    let sources = vec![
        clonk_engine::RuntimeJoinPlayerSource {
            client_id: 0,
            at_client_name: "Current client".to_string(),
            info: first.clone(),
            load_unnamed_portraits: true,
        },
        clonk_engine::RuntimeJoinPlayerSource {
            client_id: 0,
            at_client_name: "Current client".to_string(),
            info: second.clone(),
            load_unnamed_portraits: true,
        },
        clonk_engine::RuntimeJoinPlayerSource {
            client_id: 99,
            at_client_name: "Departed client".to_string(),
            info: departed.clone(),
            load_unnamed_portraits: false,
        },
    ];
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(combined_path);
    let (_sender, receiver) = mpsc::channel();
    app.loading_state = Some(test_loading_state(
        scenario,
        receiver,
        false,
        test_prepared_go(
            0,
            true,
            true,
            true,
            vec![first, second, departed],
            sources,
            Vec::new(),
        ),
    ));

    app.finalize_network_loaded_scenario(true).test_value();

    assert_eq!(
        app.control_player_infos
            .get(11)
            .expect("runtime join leaves main player infos untouched")
            .savegame_player,
        22,
        "NetworkRuntimeJoin must not execute ordinary RestoreSavegameInfos first"
    );
    let first_player = app.engine.test_player(7);
    let second_player = app.engine.test_player(5);
    assert_eq!((first_player.wealth(), first_player.score()), (1111, 111));
    assert_eq!((second_player.wealth(), second_player.score()), (2222, 222));
    assert_eq!(first_player.at_client(), clonk_engine::PlayerAtClient::HOST);
    assert_eq!(
        second_player.at_client(),
        clonk_engine::PlayerAtClient::HOST
    );
    assert_eq!(first_player.at_client_name(), "Current client");
    assert_eq!(second_player.at_client_name(), "Current client");
    assert!(
        app.engine
            .players()
            .all(|player| player.player_info_id() != 33),
        "the missing current client's whole SavePlayerInfos packet is skipped"
    );
    assert_eq!(
        app.engine
            .players()
            .map(|player| player.player_info_id())
            .collect::<Vec<_>>(),
        vec![11, 22],
        "players are installed by SavePlayerInfos packet/player order"
    );
    assert_eq!(
        second_player.message_board_queries()[0].target,
        Some(object),
        "runtime object pointers denumerate against the loaded object graph"
    );
    let restored_state = app.engine.capture_state();
    assert_eq!(
        restored_state.crew_info_rosters[&5][0].name, "Veteran",
        "the root embedded .c4p group and nested crew roster were loaded"
    );
    assert_eq!(
        app.local_controls
            .assignment(7)
            .expect("first local control")
            .set,
        0,
        "the first SavePlayerInfos row claims its preferred control set"
    );
    assert_eq!(
        app.local_controls
            .assignment(5)
            .expect("second local control")
            .set,
        1,
        "the second row observes the first row's assignment"
    );
}

#[test]
fn saved_raw_mouse_control_survives_a_failed_restore_preference_gate() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let scenario = app
        .active_scenario
        .clone()
        .unwrap_or_else(FrontendScenario::fallback);
    app.local_controls.toggle_mouse(owner).test_value();
    app.engine
        .set_player_mouse_control(owner, false)
        .test_value();
    let mut engine_state = app.engine.capture_state();
    let saved_player = engine_state
        .players
        .iter_mut()
        .find(|player| player.id == owner)
        .test_value();
    saved_player.mouse_control = 2;
    saved_player.pref_mouse = Some(false);
    let save = SavedGameFile {
        version: SAVE_FILE_VERSION,
        saved_at_seconds: 0,
        scenario: SavedScenarioInfo::from_frontend(
            &scenario,
            &app.scenario_label,
            app.fallback_ground,
        ),
        definition_load: app.active_definition_load.clone(),
        focus_id: app.focus_id,
        user_label: Some("loaded mouse survives InitControl gate".to_string()),
        runtime_music_enabled: Some(app.runtime_music_enabled),
        source_save_player_infos: None,
        source_string_table: None,
        source_title_png: None,
        engine_state,
    };

    app.apply_loaded_game(save).test_value();

    assert_eq!(
        app.engine
            .player(owner)
            .expect("restored sandbox player")
            .mouse_control(),
        2
    );
    assert_eq!(app.local_controls.mouse_owner(), Some(owner));
    assert!(app.mouse_control);
}

/// Run a test body on an explicitly sized thread. Debug builds keep one
/// O0 stack slot per by-value `GameApp` in a frame, so bodies that
/// accumulate a dozen apps sit right at libtest's default thread stack;
/// 16 MiB restores the headroom without touching the body itself.
fn run_on_multi_app_test_stack(body: fn()) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(body)
        .test_value()
        .test_join();
}

#[test]
fn debug_key_gates_remaps_and_native_priority_are_nonfatal() {
    // Thirteen sandbox apps live in this one debug-build frame; their
    // combined O0 stack slots sit right at the default test-thread stack.
    run_on_multi_app_test_stack(debug_key_gates_remaps_and_native_priority_body);
}

fn debug_key_gates_remaps_and_native_priority_body() {
    for key in [VirtualKeyCode::F6, VirtualKeyCode::F7, VirtualKeyCode::F8] {
        let mut app = new_running_sandbox_app();
        app.test_modifiers(ModifiersState::CONTROL);
        app.test_key(key, ElementState::Pressed);
        assert_eq!(runtime_flash_text(&app), Some("No debug mode!"));
        assert_eq!(
            app.graphics.debug_draw_flags(),
            clonk_frontend::DebugDrawFlags::default()
        );
        assert!(!app.exit_requested);
    }

    let mut denied = new_running_sandbox_app();
    denied.engine.set_allow_debug(false);
    denied.test_modifiers(ModifiersState::CONTROL);
    denied.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    assert!(!denied.engine.debug_mode());
    assert_eq!(runtime_flash_text(&denied), Some("Debug mode: not allowed"));

    let mut missing_resources = new_running_sandbox_app();
    missing_resources.runtime_key_config_cache = OnceLock::new();
    missing_resources
        .runtime_key_config_cache
        .set(Err("missing custom key list".to_string()))
        .test_value();
    missing_resources.runtime_flash_resources_cache = OnceLock::new();
    missing_resources
        .runtime_flash_resources_cache
        .set(Err("missing language table".to_string()))
        .test_value();
    missing_resources.test_modifiers(ModifiersState::CONTROL);
    missing_resources.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    assert!(missing_resources.engine.debug_mode());
    assert_eq!(
        runtime_flash_text(&missing_resources),
        Some("[Undefined: IDS_CTL_DEBUGMODE]: [Undefined: IDS_CTL_ON]")
    );

    let mut disabled_binding = new_running_sandbox_app();
    disabled_binding.runtime_key_config_cache = OnceLock::new();
    disabled_binding
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgModeToggle=None\n",
        )
        .expect("parse disabled debug binding")))
        .test_value();
    disabled_binding.test_modifiers(ModifiersState::CONTROL);
    disabled_binding.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    assert!(!disabled_binding.engine.debug_mode());
    assert!(disabled_binding.runtime_flash_message.is_none());

    let mut debug_collision = new_running_sandbox_app();
    debug_collision.engine.set_allow_debug(false);
    debug_collision.runtime_key_config_cache = OnceLock::new();
    debug_collision
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgModeToggle=G\nDbgShowVtxToggle=G\n",
        )
        .expect("parse same-priority debug collision")))
        .test_value();
    debug_collision.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert_eq!(runtime_flash_text(&debug_collision), Some("No debug mode!"));

    let mut modified_player = new_running_sandbox_app();
    modified_player
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F5);
    modified_player.test_modifiers(ModifiersState::CONTROL);
    modified_player.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    assert!(modified_player.engine.debug_mode());

    let mut context_priority = new_running_sandbox_app();
    context_priority.runtime_key_config_cache = OnceLock::new();
    context_priority
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(b"[Keys]\nDbgModeToggle=R\n")
            .expect("parse context/debug collision")))
        .test_value();
    context_priority
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new("Remain off").with_hotkey('R')],
            GuiPoint::new(20.0, 20.0),
        )
        .test_value();
    context_priority.test_key(VirtualKeyCode::KeyR, ElementState::Pressed);
    assert!(!context_priority.engine.debug_mode());
    assert!(context_priority.runtime_flash_message.is_none());

    let mut chat_priority = new_running_sandbox_app();
    chat_priority.runtime_key_config_cache = OnceLock::new();
    chat_priority
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgModeToggle=Return\n",
        )
        .expect("parse chat/debug collision")))
        .test_value();
    chat_priority.start_running_chat(RunningChatMode::All);
    chat_priority.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert!(!chat_priority.engine.debug_mode());
    assert!(!chat_priority.running_chat_active());

    let mut modified_chat_fallthrough = new_running_sandbox_app();
    modified_chat_fallthrough.runtime_key_config_cache = OnceLock::new();
    modified_chat_fallthrough
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgModeToggle=Ctrl+Alt+G\n",
        )
        .expect("parse modified chat/debug collision")))
        .test_value();
    modified_chat_fallthrough.start_running_chat(RunningChatMode::All);
    modified_chat_fallthrough.test_modifiers(ModifiersState::CONTROL | ModifiersState::ALT);
    modified_chat_fallthrough.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert!(modified_chat_fallthrough.engine.debug_mode());
    assert!(modified_chat_fallthrough.running_chat_active());

    let mut vote_priority = new_running_sandbox_app();
    vote_priority.runtime_key_config_cache = OnceLock::new();
    vote_priority
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgModeToggle=Return\n",
        )
        .expect("parse vote/debug collision")))
        .test_value();
    vote_priority
        .push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                "Vote?",
                "Voting",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                true,
            ),
            MessageDialogContinuation::LeagueSurrender,
        )
        .test_value();
    vote_priority.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert!(!vote_priority.engine.debug_mode());
    assert!(vote_priority.runtime_flash_message.is_none());
    assert_eq!(vote_priority.message_dialogs.len(), 1);
    vote_priority.test_key(VirtualKeyCode::Enter, ElementState::Released);
    assert!(vote_priority.message_dialogs.is_empty());
    assert_eq!(vote_priority.mode, AppMode::Running);

    let mut game_over_priority = new_game_over_keyboard_app();
    game_over_priority.runtime_key_config_cache = OnceLock::new();
    game_over_priority
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgModeToggle=Return\n",
        )
        .expect("parse game-over/debug collision")))
        .test_value();
    game_over_priority.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    assert!(!game_over_priority.engine.debug_mode());
    assert!(game_over_priority.running_chat_active());

    let mut disable = new_running_sandbox_app();
    disable.engine.set_allow_debug(false);
    disable.engine.set_debug_mode(true);
    disable
        .graphics
        .set_debug_draw_flags(clonk_frontend::DebugDrawFlags {
            show_vertices: true,
            show_entrance: true,
            show_action: true,
            show_command: true,
            show_pathfinder: true,
            show_solid_mask: true,
            show_net_status: true,
        });
    disable.test_modifiers(ModifiersState::CONTROL);
    disable.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    assert!(!disable.engine.debug_mode());
    assert_eq!(
        disable.graphics.debug_draw_flags(),
        clonk_frontend::DebugDrawFlags::default()
    );

    let mut later_collision = new_running_sandbox_app();
    later_collision.runtime_key_config_cache = OnceLock::new();
    later_collision
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgShowVtxToggle=G\nGameSpeedUp=G\n",
        )
        .expect("parse denied-debug/later-global collision")))
        .test_value();
    later_collision.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert!(!later_collision.engine.debug_mode());
    assert_eq!(later_collision.frame_skip, 2);
    assert!(later_collision.full_speed);
    assert_eq!(runtime_flash_text(&later_collision), Some("Speed: 2x"));

    let mut rebound = new_running_sandbox_app();
    rebound.runtime_key_config_cache = OnceLock::new();
    rebound
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nDbgModeToggle=G\nDbgShowVtxToggle=H\n",
        )
        .expect("parse rebound debug keys")))
        .test_value();
    rebound.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert!(rebound.engine.debug_mode());
    rebound.test_key(VirtualKeyCode::KeyH, ElementState::Pressed);
    assert!(rebound.graphics.debug_draw_flags().show_vertices);

    let mut player_collision = new_running_sandbox_app();
    player_collision.runtime_key_config_cache = OnceLock::new();
    player_collision
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nKbd1Key1=G\nDbgModeToggle=G\n",
        )
        .expect("parse player/debug collision")))
        .test_value();
    player_collision.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert!(!player_collision.engine.debug_mode());
    assert!(player_collision.runtime_flash_message.is_none());

    let mut global_collision = new_running_sandbox_app();
    global_collision.runtime_key_config_cache = OnceLock::new();
    global_collision
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nSoundToggle=G\nDbgModeToggle=G\n",
        )
        .expect("parse earlier-global/debug collision")))
        .test_value();
    let sound_enabled = global_collision.audio.test_ref().options.sound_enabled;
    global_collision.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert_eq!(
        global_collision
            .audio
            .as_ref()
            .expect("sandbox audio")
            .options
            .sound_enabled,
        !sound_enabled
    );
    assert!(!global_collision.engine.debug_mode());
    assert!(global_collision.runtime_flash_message.is_none());

    let mut game_over = new_game_over_keyboard_app();
    game_over.engine.set_debug_mode(true);
    game_over.test_modifiers(ModifiersState::CONTROL);
    game_over.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(game_over.graphics.debug_draw_flags().show_solid_mask);
    assert!(game_over.game_over_dialog.is_some());
    assert_eq!(runtime_flash_text(&game_over), Some("SolidMasks: on"));
    assert!(!game_over.exit_requested);
}

#[test]
fn renderer_config_loads_native_defaults_and_graphics_values() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();

    let defaults = load_display_flags(Some(&paths));
    assert!(defaults.show_player_hud_always);
    assert!(defaults.splitscreen_dividers);
    assert!(defaults.fire_particles);
    assert!(defaults.pxs_gfx);

    fs::write(
        paths.config_file(),
        "[Graphics]\nShowPlayerHUDAlways=0\nSplitscreenDividers=0\nFireParticles=0\nPXSGfx=0\n",
    )
    .test_value();
    let disabled = load_display_flags(Some(&paths));
    assert!(!disabled.show_player_hud_always);
    assert!(!disabled.splitscreen_dividers);
    assert!(!disabled.fire_particles);
    assert!(!disabled.pxs_gfx);

    fs::write(
        paths.config_file(),
        "[Graphics]\nShowPlayerHUDAlways=1\nSplitscreenDividers=-1\nFireParticles=1\nPXSGfx=1\n",
    )
    .test_value();
    let enabled = load_display_flags(Some(&paths));
    assert!(enabled.show_player_hud_always);
    assert!(
        enabled.splitscreen_dividers,
        "any nonzero divider integer follows C++ truthiness"
    );
    assert!(enabled.fire_particles);
    assert!(enabled.pxs_gfx);

    fs::write(
        paths.config_file(),
        "[Graphics]\nSplitscreenDividers=invalid\n",
    )
    .test_value();
    assert!(load_display_flags(Some(&paths)).splitscreen_dividers);
}

// C4Network2ClientDlg appends " (!ack)" only when Game.Network.isHost() and
// the row's C4Network2Client exists and is not NCS_Ready
// (src/C4Network2Dialogs.cpp:62,71; src/C4Network2Client.h:113).
#[test]
fn client_info_ack_marker_needs_a_host_and_an_unready_net_client() {
    let state = |status| network::RuntimeNetworkClientState {
        client_id: 7,
        status,
        control_ready: true,
        wait_ms: 0,
    };
    for unready in [
        clonk_network::RemoteBarrierState::Joining,
        clonk_network::RemoteBarrierState::Chasing,
        clonk_network::RemoteBarrierState::NotReady,
        clonk_network::RemoteBarrierState::Removing,
    ] {
        assert!(GameApp::runtime_client_row_unacknowledged(
            true,
            Some(&state(unready))
        ));
        assert!(
            !GameApp::runtime_client_row_unacknowledged(false, Some(&state(unready))),
            "a client never renders the host-only marker"
        );
    }
    assert!(!GameApp::runtime_client_row_unacknowledged(
        true,
        Some(&state(clonk_network::RemoteBarrierState::Ready))
    ));
    assert!(
        !GameApp::runtime_client_row_unacknowledged(true, None),
        "the local row has no C4Network2Client to interrogate"
    );
}

// `/netgetscen` copies the transferred scenario resource next to the
// executable, and only for a non-host network client outside the lobby - the
// lobby uses its Resources tab instead. Every other state, and every failure
// along the way, is C++'s `return false`, which surfaces as the ordinary
// unknown-command error (src/C4MessageInput.cpp:527-545).
#[test]
fn running_chat_netgetscen_saves_client_scenario_resource_like_cpp() {
    let transfer = tempdir();
    let source = transfer.path().join("Transferred.c4s");
    fs::write(&source, b"packed scenario bytes").test_value();

    // The sandbox app boots against the repository install root; only the
    // netgetscen destination is redirected afterwards.
    let mut app = new_classic_running_sandbox_app();
    // AppPaths::discover validates planet/System.c4g, so the redirected root
    // gets a minimal one; only the install root matters to netgetscen.
    let install_root = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install_root.path().join("planet/System.c4g")).test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    app.app_paths = Some(test_app_paths());
    let (network, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.scenario = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 9,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Scenarios/Remote.c4s".to_vec()).test_value(),
        ..Default::default()
    };
    app.pending_network_join_data = Some(clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 0,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    });

    // The resource has not arrived yet: C++ returns false and the command is
    // reported as unknown.
    assert!(app.save_joined_scenario_resource().is_none());

    app.admission_resources.resources.insert(
        9,
        AdmissionResourceState::Complete {
            path: source.clone(),
            removed: false,
            local: false,
        },
    );

    // `Game.Network.isHost()` is client-id based, so the host - which already
    // owns the file - never gets the command.
    let (host_network, _host_events) = NetworkManager::test_stub_for_client_id(0);
    app.network = Some(host_network);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.process_running_chat_text("/netgetscen");
    assert!(
        !install_root.path().join("Remote.c4s").exists(),
        "the network host has the resource already"
    );

    let (client_network, _client_events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(client_network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.process_running_chat_text("/netgetscen");

    // Saved under the scenario's own base name, next to the executable, not
    // under the resource's transfer name.
    let destination = install_root.path().join("Remote.c4s");
    assert_eq!(
        fs::read(&destination).expect("saved scenario"),
        b"packed scenario bytes"
    );

    // CreateItem erases an existing target first, so a repeat overwrites and
    // succeeds (src/C4Group.cpp:147; src/StdFile.cpp:660-670).
    fs::write(&source, b"newer scenario bytes").test_value();
    assert_eq!(
        app.save_joined_scenario_resource().as_deref(),
        Some(destination.as_path())
    );
    assert_eq!(
        fs::read(&destination).expect("overwritten scenario"),
        b"newer scenario bytes"
    );
}

// C4Network2::DrawStatus reads the live per-protocol accumulator through
// getProtIRate/getProtORate/getProtBCRate, and coalesces the two protocol
// lines into one "Msg/Data" entry when the message and data NetIO are the same
// object (src/C4Network2.cpp:1148-1181).
#[test]
fn network_status_overlay_displays_live_protocol_rate_samples() {
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    app.graphics
        .set_debug_draw_flags(clonk_frontend::DebugDrawFlags {
            show_net_status: true,
            ..clonk_frontend::DebugDrawFlags::default()
        });

    // Both NetIO objects are bound, which is what makes the protocol line
    // appear at all (src/C4Network2.cpp:1149-1150).
    app.network.test_ref().set_test_local_addresses([
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Tcp,
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
        ),
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Udp,
            SocketAddr::from(([127, 0, 0, 1], 11_113)),
        ),
    ]);

    // An unsampled accumulator reads zero, exactly like native startup.
    app.update_network_status_overlay();
    let text = app.graphics.network_status_text().test_value();
    assert!(text.contains("i0 o0 bc0"), "{text}");

    let network = app.network.test_ref();
    network.record_test_protocol_traffic(1, clonk_network::NetworkProtocol::Udp, 900, 300, 120);
    network.record_test_protocol_traffic(2, clonk_network::NetworkProtocol::Tcp, 400, 100, 40);
    // GenerateStatistics only consumes an edge strictly outside C++'s
    // inclusive one-second window, so close the sample past it.
    network.generate_test_statistics(2_000);

    app.update_network_status_overlay();
    let text = app.graphics.network_status_text().test_value();
    let protocols = text
        .split('|')
        .find(|line| line.starts_with("Protocols:"))
        .test_value();
    assert!(
        protocols.contains(" i") && protocols.contains(" o") && protocols.contains(" bc"),
        "{protocols}"
    );
    assert!(
        !protocols.contains("i0 o0 bc0"),
        "the overlay must show the sampled rates, not zero: {protocols}"
    );

    // Reading the overlay must not consume the interval the per-second chart
    // sampling owns: a second draw shows the same cached values.
    let first = protocols.to_string();
    app.update_network_status_overlay();
    let second = app
        .graphics
        .network_status_text()
        .expect("second network status text")
        .split('|')
        .find(|line| line.starts_with("Protocols:"))
        .test_value()
        .to_string();
    assert_eq!(first, second);
}

// NetStatsToggle is registered at KEY_Default in the "no default keys
// assigned" block (src/C4Game.cpp:3456, :3462), so only a custom
// `[Keys] NetStatsToggle=` binding can reach it. Unlike its
// `C4GraphicsSystem::ToggleShow*` neighbours, ToggleShowNetStatus has no
// Game.DebugMode guard and flashes no message
// (src/C4GraphicsSystem.cpp:811-815).
#[test]
fn net_stats_toggle_is_default_unbound_and_a_custom_chord_shows_the_overlay() {
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);

    app.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(!app.graphics.debug_draw_flags().show_net_status);

    let parsed = parse_runtime_key_config(b"[Keys]\nNetStatsToggle=F8\n").test_value();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache.set(Ok(parsed)).test_value();

    app.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(app.graphics.debug_draw_flags().show_net_status);
    assert!(
        !app.engine.debug_mode(),
        "ToggleShowNetStatus has no debug-mode guard"
    );
    assert!(
        app.runtime_flash_message.is_none(),
        "ToggleShowNetStatus flashes no message"
    );

    app.update_network_status_overlay();
    let text = app.graphics.network_status_text().test_value();
    assert!(text.contains("Local: Active host Host (ID 0)"), "{text}");

    app.test_key(VirtualKeyCode::F8, ElementState::Released);
    assert!(app.graphics.debug_draw_flags().show_net_status);

    app.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(!app.graphics.debug_draw_flags().show_net_status);
    app.update_network_status_overlay();
    assert!(app.graphics.network_status_text().is_none());
}

#[test]
fn runtime_client_list_maps_native_lifecycle_readiness_and_wait() {
    use clonk_frontend::runtime_client_list::RuntimeClientStatusIcon;

    let state = |status, control_ready, wait_ms| network::RuntimeNetworkClientState {
        client_id: 7,
        status,
        control_ready,
        wait_ms,
    };
    for lifecycle in [
        clonk_network::RemoteBarrierState::Joining,
        clonk_network::RemoteBarrierState::Chasing,
        clonk_network::RemoteBarrierState::NotReady,
    ] {
        assert_eq!(
            GameApp::runtime_client_row_network_state(false, Some(&state(lifecycle, true, -12)),),
            (RuntimeClientStatusIcon::Loading, Some(-12))
        );
    }
    assert_eq!(
        GameApp::runtime_client_row_network_state(
            false,
            Some(&state(clonk_network::RemoteBarrierState::Ready, false, 9)),
        ),
        (RuntimeClientStatusIcon::NetWait, Some(9))
    );
    assert_eq!(
        GameApp::runtime_client_row_network_state(
            false,
            Some(&state(clonk_network::RemoteBarrierState::Ready, true, 4)),
        ),
        (RuntimeClientStatusIcon::Ready, Some(4))
    );
    assert_eq!(
        GameApp::runtime_client_row_network_state(
            false,
            Some(&state(clonk_network::RemoteBarrierState::Removing, true, 2)),
        ),
        (RuntimeClientStatusIcon::Kick, Some(2))
    );
    assert_eq!(
        GameApp::runtime_client_row_network_state(
            true,
            Some(&state(clonk_network::RemoteBarrierState::Ready, false, 99)),
        ),
        (RuntimeClientStatusIcon::NetWait, None)
    );
}

#[test]
fn network_status_collector_uses_native_client_next_control_baselines() {
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    let mut inactive = message_client(9, b"Inactive");
    inactive.activated = false;
    app.control_clients.replace_snapshot([
        message_client(0, b"Host"),
        message_client(7, b"Remote"),
        inactive,
    ]);
    app.refresh_network_client_next_control_ticks();
    app.network_control_clock = Some(NetworkControlClock::new(44, 4));
    app.network.test_ref().set_test_runtime_client_states([
        network::RuntimeNetworkClientState {
            client_id: 7,
            status: clonk_network::RemoteBarrierState::Ready,
            control_ready: false,
            wait_ms: -12,
        },
        network::RuntimeNetworkClientState {
            client_id: 9,
            status: clonk_network::RemoteBarrierState::NotReady,
            control_ready: true,
            wait_ms: 5,
        },
    ]);
    app.graphics
        .set_debug_draw_flags(clonk_frontend::DebugDrawFlags {
            show_net_status: true,
            ..clonk_frontend::DebugDrawFlags::default()
        });

    app.update_network_status_overlay();

    let text = app.graphics.network_status_text().test_value();
    assert!(
        text.contains(
            "|- Active client Remote (ID 7) (wait -12 ms, behind 4) (ready to start) (!ctrl)"
        ),
        "{text}"
    );
    assert!(
        text.contains("|- Inactive client Inactive (ID 9) (wait 5 ms, behind 44) (!rdy)"),
        "{text}"
    );

    app.refresh_network_client_next_control_ticks();
    app.update_network_status_overlay();
    assert!(app
        .graphics
        .network_status_text()
        .expect("refreshed network status text")
        .contains(
            "|- Active client Remote (ID 7) (wait -12 ms, behind 0) (ready to start) (!ctrl)"
        ));
}

#[test]
fn runtime_f4_toggles_only_live_network_dialog_and_consumes_edges() {
    for (role, opens) in [
        (RuntimeNetworkRole::Offline, false),
        (RuntimeNetworkRole::Host, true),
        (RuntimeNetworkRole::Client, true),
        (RuntimeNetworkRole::Ambiguous, false),
    ] {
        let mut app = new_running_sandbox_app();
        configure_runtime_network_role(&mut app, role);
        app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
        assert_eq!(app.runtime_client_list.is_some(), opens, "role {role:?}");
        assert_eq!(app.mode, AppMode::Running);
        assert!(!app.exit_requested);

        let before_release = runtime_global_ui_snapshot(&app);
        app.test_key(VirtualKeyCode::F4, ElementState::Released);
        let mut after_release = runtime_global_ui_snapshot(&app);
        // `C4Game::DoKeyboardInput` clears its `PressedKeys` entry for every
        // key-up before any scope or dialog decision (C4Game.cpp:2143-2155),
        // so the held-key latch is the one thing a consumed release does
        // change. Nothing else may.
        assert!(
            after_release.pressed_engine_keys.is_empty(),
            "role {role:?}: the release clears the physical held-key latch"
        );
        after_release
            .pressed_engine_keys
            .clone_from(&before_release.pressed_engine_keys);
        assert_eq!(after_release, before_release);

        app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
        assert!(app.runtime_client_list.is_none());
    }
}

#[test]
fn runtime_client_list_host_actions_submit_native_controls() {
    use clonk_frontend::runtime_client_list::RuntimeClientListAction;

    let mut activate = new_running_sandbox_app();
    let (_events, mut activate_commands) = install_running_network_stub(&mut activate, 0, 40, 4);
    activate
        .control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    activate
        .handle_runtime_client_list_action(RuntimeClientListAction::ToggleActivate(7))
        .test_value();
    assert_eq!(
        activate_commands.take_submitted_client_updates(),
        vec![clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 7,
            data: 0,
            by_client: 0,
        }]
    );

    let mut kick = new_running_sandbox_app();
    let (_events, mut kick_commands) = install_running_network_stub(&mut kick, 0, 40, 4);
    kick.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    kick.handle_runtime_client_list_action(RuntimeClientListAction::Kick(7))
        .test_value();
    assert_eq!(
        kick_commands.take_submitted_client_removes(),
        vec![clonk_engine::ClientRemoveControlData {
            client_id: 7,
            reason: clonk_engine::LegacyCString::from_bytes(b"kicked from client list".to_vec())
                .expect("fixture reason"),
            by_client: 0,
        }]
    );
}

#[test]
fn f4_control_rate_dropdown_waits_for_authoritative_echo() {
    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let rate = {
        let dialog = app.runtime_client_list.test_ref();
        let layout = dialog.layout(preferred, line_height);
        let index = dialog
            .option_rows()
            .iter()
            .position(|row| row.kind == LobbyOptionKind::ControlRate)
            .test_value();
        layout.option_rows[index].value
    };
    let point = GuiPoint::new((rate.x + 2) as f32, (rate.y + 2) as f32);
    app.running_pointer_position = Some(point);
    assert!(app
        .handle_runtime_client_list_pointer_button(ElementState::Pressed)
        .expect("press control-rate combo"));
    assert!(app
        .handle_runtime_client_list_pointer_button(ElementState::Released)
        .expect("open control-rate dropdown"));
    assert!(app.context_menu.is_some());
    assert_eq!(
        app.context_menu_lobby_option,
        Some(LobbyOptionKind::ControlRate)
    );

    app.process_context_menu_outcome(ContextMenuOutcome {
        captured: true,
        pass_through: false,
        focus_suppressed: true,
        events: vec![
            ContextMenuEvent::Closed,
            ContextMenuEvent::Activated(AppContextMenuCommand::RuntimeClientOption {
                option: LobbyOptionKind::ControlRate,
                value: 7,
            }),
        ],
    })
    .test_value();
    assert_eq!(app.engine.control_rate(), 4);
    assert_eq!(
        app.network_control_clock
            .map(NetworkControlClock::control_rate),
        Some(4)
    );
    assert_eq!(
        app.runtime_client_list
            .as_ref()
            .expect("F4 dialog remains open")
            .option_rows()
            .iter()
            .find(|row| row.kind == LobbyOptionKind::ControlRate)
            .map(|row| row.value.as_str()),
        Some("4")
    );
    let decided = commands.take_submitted_decided_controls();
    assert_eq!(decided.len(), 1);
    let set = clonk_network::LegacyControlSet::from_control_packet(&decided[0].1).test_value();
    assert_eq!(
        set,
        clonk_network::LegacyControlSet {
            value_type: 0,
            data: 3,
            by_client: 0,
        }
    );

    app.apply_ready_controls(decided[0].0, vec![NetworkControl::Set(set)])
        .test_value();
    assert_eq!(app.engine.control_rate(), 7);
    assert_eq!(
        app.network_control_clock
            .map(NetworkControlClock::control_rate),
        Some(7)
    );
    assert!(app.sec1_timer().expect("refresh runtime options"));
    assert_eq!(
        app.runtime_client_list
            .as_ref()
            .expect("F4 dialog remains open")
            .option_rows()
            .iter()
            .find(|row| row.kind == LobbyOptionKind::ControlRate)
            .map(|row| row.value.as_str()),
        Some("7")
    );
}

#[test]
fn f4_runtime_join_waits_for_network_ack_and_flashes_state() {
    let mut app = new_classic_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    let labels = app.classic_lobby_option_labels();
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);

    let acknowledgement = thread::spawn(move || {
        let (allowed, completion) = commands.receive_join_allowed();
        assert!(allowed);
        completion.send(Ok(())).test_value();
    });
    app.apply_runtime_client_list_option(LobbyOptionKind::RuntimeJoin, 1)
        .test_value();
    acknowledgement.test_join();

    assert_eq!(app.runtime_network_join_allowed, Some(true));
    assert_eq!(
        app.runtime_client_list
            .as_ref()
            .expect("F4 dialog remains open")
            .option_rows()
            .iter()
            .find(|row| row.kind == LobbyOptionKind::RuntimeJoin)
            .map(|row| row.value.as_str()),
        Some(labels.runtime_join_free.as_str())
    );
    assert_eq!(
        app.runtime_flash_message
            .as_ref()
            .map(|message| message.text.as_str()),
        Some(labels.runtime_join_free.as_str())
    );
}

#[test]
fn running_f4_nonexclusive_scope_does_not_receive_tab() {
    let mut app = new_classic_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);

    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    assert_eq!(
        app.runtime_client_list
            .as_ref()
            .and_then(|dialog| dialog.focused()),
        None
    );
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    assert!(app.runtime_client_list.is_some());
}

#[test]
fn runtime_client_list_wheel_precedes_running_player_control() {
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    assert!(app.mouse_control);
    assert!(app.local_controls.mouse_owner().is_some());

    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let layout = app
        .runtime_client_list
        .test_ref()
        .layout(preferred, line_height);
    app.running_pointer_position = Some(GuiPoint::new(
        (layout.list.x + 4) as f32,
        (layout.list.y + 4) as f32,
    ));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    assert!(commands.take_submitted_local().is_empty());

    app.running_pointer_position = Some(GuiPoint::new(0.0, 0.0));
    app.test_mouse_wheel(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -120.0)),
        2.0,
    );
    let submitted = commands.take_submitted_local();
    assert_eq!(submitted.len(), 1);
    assert!(matches!(
        submitted[0].1,
        ControlEvent::RawPlayerControl {
            command: clonk_engine::COM_WHEEL_DOWN,
            data: 0,
        }
    ));
}

#[test]
fn standalone_client_info_routes_wheel_and_keyboard_to_overflow() {
    use clonk_frontend::runtime_client_list::{
        RuntimeClientListDialog, RuntimeClientRow, RuntimeClientStatusIcon,
    };

    let mut app = new_classic_running_sandbox_app();
    app.resize(640, 480).test_value();
    app.mode = AppMode::Menu;
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let row = RuntimeClientRow {
        client_id: 7,
        name: "Remote".to_string(),
        nick: "Nick".to_string(),
        host: false,
        local: false,
        activated: true,
        observer: false,
        muted: false,
        has_players: false,
        player_names: Vec::new(),
        addresses: (0..20)
            .map(|index| format!("198.51.100.{index}:1111"))
            .collect(),
        status: RuntimeClientStatusIcon::Ready,
        wait_ms: None,
        connections: Vec::new(),
        can_moderate: false,
        unacknowledged: false,
    };
    app.runtime_client_list = Some(RuntimeClientListDialog::new_info(
        "Client information",
        row.client_id,
        Some(row),
    ));
    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let info = app
        .runtime_client_list
        .as_ref()
        .and_then(|dialog| dialog.info_layout(preferred, line_height))
        .test_value();
    app.running_pointer_position = Some(GuiPoint::new(
        (info.text.x + 2) as f32,
        (info.text.y + info.text.h / 2) as f32,
    ));
    let initial = app
        .runtime_client_list
        .as_ref()
        .and_then(|dialog| dialog.info_scroll_metrics(preferred, &fonts.text))
        .test_value();
    assert!(initial.max_scroll > 0);

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    let wheeled = app
        .runtime_client_list
        .as_ref()
        .and_then(|dialog| dialog.info_scroll_metrics(preferred, &fonts.text))
        .test_value();
    assert!(wheeled.scroll_y > initial.scroll_y);

    app.test_key(VirtualKeyCode::End, ElementState::Pressed);
    let ended = app
        .runtime_client_list
        .as_ref()
        .and_then(|dialog| dialog.info_scroll_metrics(preferred, &fonts.text))
        .test_value();
    assert_eq!(ended.scroll_y, ended.max_scroll);
    app.test_key(VirtualKeyCode::End, ElementState::Released);
    app.test_key(VirtualKeyCode::Home, ElementState::Pressed);
    assert_eq!(
        app.runtime_client_list
            .as_ref()
            .and_then(|dialog| dialog.info_scroll_metrics(preferred, &fonts.text))
            .expect("Home scroll metrics")
            .scroll_y,
        0
    );
}

#[test]
fn runtime_client_list_consumption_cancels_world_mouse_gestures() {
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let bounds = app
        .runtime_client_list
        .test_ref()
        .layout(preferred, line_height)
        .bounds;
    let dialog_point = GuiPoint::new(
        (bounds.x + bounds.w / 2) as f32,
        (bounds.y + bounds.h / 2) as f32,
    );
    app.test_cursor(PhysicalPosition::new(
        f64::from(dialog_point.x),
        f64::from(dialog_point.y),
    ));

    let owner = app.local_owner;
    let retained = app.engine.test_crew_cursor(owner);
    let pointer = ViewportPointer {
        owner,
        world: FloatVector2::new(20.0, 20.0),
        screen: GuiPoint::new(20.0, 20.0),
    };
    let seed_world_gestures = |app: &mut GameApp| {
        let mut left = IngameButtonMouseState::new(pointer, Some(retained), false);
        left.motion.moved = true;
        let mut right = IngameButtonMouseState::new(pointer, Some(retained), false);
        right.motion.moved = true;
        app.mouse_state = Some(left);
        app.ingame_right_mouse_state = Some(right);
        app.ingame_dragged_objects = vec![retained];
        app.ingame_last_left_down = Some(Instant::now());
        app.ingame_ignore_left_up = true;
    };
    let assert_cancelled = |app: &GameApp| {
        assert!(app.mouse_state.is_none());
        assert!(app.ingame_right_mouse_state.is_none());
        assert!(app.ingame_dragged_objects.is_empty());
        assert!(app.ingame_last_left_down.is_none());
        assert!(!app.ingame_ignore_left_up);
    };

    seed_world_gestures(&mut app);
    app.test_left_button(ElementState::Released);
    assert_cancelled(&app);

    seed_world_gestures(&mut app);
    app.test_right_button(ElementState::Released);
    assert_cancelled(&app);

    seed_world_gestures(&mut app);
    app.test_touch(TouchPhase::Moved, dialog_point);
    assert_cancelled(&app);

    let (direct, player_commands, selections) = commands.take_submitted_mouse_controls();
    assert!(direct.is_empty());
    assert!(player_commands.is_empty());
    assert!(selections.is_empty());
}

#[test]
fn runtime_client_list_prevents_tick5_from_reviving_edge_scroll() {
    let mut app = new_running_sandbox_app();
    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
    let owner = app.local_owner;
    let focus = app.engine.test_crew_cursor(owner);
    app.engine
        .replace_player_viewports(
            owner,
            vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let rect = app.graphics.viewport_rect(owner).test_value();
    let edge = GuiPoint::new(rect.x as f32, (rect.y + rect.height as i32 / 2) as f32);
    app.test_cursor(PhysicalPosition::new(f64::from(edge.x), f64::from(edge.y)));
    assert!(app.ingame_edge_scroll.is_some());

    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let bounds = app
        .runtime_client_list
        .test_ref()
        .layout(preferred, line_height)
        .bounds;
    let dialog_point = GuiPoint::new(
        (bounds.x + bounds.w / 2) as f32,
        (bounds.y + bounds.h / 2) as f32,
    );
    let stopped = app.engine.player(owner).test_value().viewports()[0].center;
    app.test_cursor(PhysicalPosition::new(
        f64::from(dialog_point.x),
        f64::from(dialog_point.y),
    ));
    assert!(app.ingame_pointer.is_none());
    assert!(app.ingame_edge_scroll.is_none());
    assert!(
        app.ingame_viewport_mouse.is_some(),
        "native VpX/VpY remains retained for Tick5 reevaluation"
    );

    for _ in 0..2 {
        assert!(!app
            .refresh_ingame_edge_scroll_tick5()
            .expect("client-list Tick5 reevaluation"));
    }
    assert_eq!(
        app.engine.player(owner).unwrap().viewports()[0].center,
        stopped
    );
    assert!(app.ingame_pointer.is_none());
    assert!(app.ingame_edge_scroll.is_none());
}

#[test]
fn runtime_client_list_status_refreshes_only_on_the_one_second_timer() {
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    app.network
        .test_ref()
        .set_test_runtime_client_states([network::RuntimeNetworkClientState {
            client_id: 7,
            status: clonk_network::RemoteBarrierState::Joining,
            control_ready: true,
            wait_ms: -12,
        }]);
    let mut clock = NetworkControlClock::new(40, 4);
    clock.observe_control_send_time_ms(6_000);
    clock.calculate_performance();
    clock.complete_control_frame();
    app.network_control_clock = Some(clock);

    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    let initial = app.runtime_client_list.test_ref().status();
    assert_eq!(
        (
            initial.tick,
            initial.rate,
            initial.presend,
            initial.average_control_time
        ),
        // A six-second link saturates the 1..15 PreSend clamp; the ACT
        // beside it is still C++'s exact 1/150 EWMA of the same sample.
        (41, 4, 15, 40_000)
    );
    assert_eq!(
        app.runtime_client_list
            .as_ref()
            .expect("dialog open")
            .status_text(),
        initial.to_string()
    );
    let initial_remote = app
        .runtime_client_list
        .test_ref()
        .rows()
        .iter()
        .find(|row| row.client_id == 7)
        .test_value();
    assert_eq!(
        (initial_remote.status, initial_remote.wait_ms),
        (
            clonk_frontend::runtime_client_list::RuntimeClientStatusIcon::Loading,
            Some(-12)
        )
    );

    app.network_control_clock = Some(NetworkControlClock::new(50, 2));
    app.network
        .test_ref()
        .set_test_runtime_client_states([network::RuntimeNetworkClientState {
            client_id: 7,
            status: clonk_network::RemoteBarrierState::Ready,
            control_ready: false,
            wait_ms: 9,
        }]);
    assert_eq!(
        app.runtime_client_list
            .as_ref()
            .expect("dialog remains open")
            .status(),
        initial,
        "the visible status is a one-second snapshot"
    );
    let stale_remote = app
        .runtime_client_list
        .test_ref()
        .rows()
        .iter()
        .find(|row| row.client_id == 7)
        .test_value();
    assert_eq!(
        (stale_remote.status, stale_remote.wait_ms),
        (
            clonk_frontend::runtime_client_list::RuntimeClientStatusIcon::Loading,
            Some(-12)
        ),
        "client rows are also one-second snapshots"
    );
    assert!(app
        .sec1_timer()
        .expect("pulse one-second network dialog timer"));
    let refreshed = app.runtime_client_list.test_ref().status();
    assert_eq!(
        (
            refreshed.tick,
            refreshed.rate,
            refreshed.presend,
            refreshed.average_control_time
        ),
        (50, 2, 1, 0)
    );
    assert!(refreshed.to_string().contains("Behind "));
    let waiting_remote = app
        .runtime_client_list
        .test_ref()
        .rows()
        .iter()
        .find(|row| row.client_id == 7)
        .test_value();
    assert_eq!(
        (waiting_remote.status, waiting_remote.wait_ms),
        (
            clonk_frontend::runtime_client_list::RuntimeClientStatusIcon::NetWait,
            Some(9)
        )
    );

    app.network
        .test_ref()
        .set_test_runtime_client_states([network::RuntimeNetworkClientState {
            client_id: 7,
            status: clonk_network::RemoteBarrierState::Removing,
            control_ready: true,
            wait_ms: 4,
        }]);
    assert!(app.sec1_timer().expect("refresh removing client row"));
    let removing_remote = app
        .runtime_client_list
        .test_ref()
        .rows()
        .iter()
        .find(|row| row.client_id == 7)
        .test_value();
    assert_eq!(
        (removing_remote.status, removing_remote.wait_ms),
        (
            clonk_frontend::runtime_client_list::RuntimeClientStatusIcon::Kick,
            Some(4)
        )
    );
}

#[test]
fn runtime_pause_control_script_uses_the_executing_network_tick() {
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 7, 2);
    app.apply_ready_controls(
        7,
        vec![NetworkControl::Script(clonk_engine::ScriptControlData {
            target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: clonk_engine::ScriptStrictness::Strict3,
            script: clonk_engine::LegacyCString::from_bytes(b"PauseGame()".to_vec())
                .expect("script is NUL-free"),
            by_client: 0,
        })],
    )
    .test_value();

    let changes = commands
        .take_runtime_status_commands()
        .into_iter()
        .filter_map(|command| match command {
            network::TestRuntimeStatusCommand::Change(status) => Some(status),
            network::TestRuntimeStatusCommand::Reached { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].state, changes[0].target_tick),
        (clonk_network::NETWORK_STATE_PAUSE, 7),
        "PauseGame runs before Rust advances the executing control tick"
    );
    assert!(!app.take_exit_request());
}

#[test]
fn save_to_slot_writes_native_c4group_savegame() {
    let fixture = tempdir();
    let user_data = fixture.path().join("user-data");
    let save_root = fixture.path().join("Savegames.c4f");
    let (_guard, paths) = exact_loader_test_paths(&user_data, None);
    persist_config_value(
        &paths,
        "General",
        "SaveGameFolder",
        save_root.to_string_lossy().into_owned(),
    )
    .test_value();
    persist_config_value(&paths, "General", "Language", "US").test_value();

    let scenario_path = fixture.path().join("Missions.c4f").join("01.c4s");
    install_record_test_definitions(&fixture.path().join("Missions.c4f"));
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(scenario_path.join("Scenario.txt"), b"[Head]\nTitle=Source scenario\nIcon=4\nMaxPlayer=4\n\n[Definitions]\nDefinition1=Objects.c4d\n").test_value();
    fs::write(scenario_path.join("Source.bin"), b"copied source sentinel").test_value();
    fs::write(scenario_path.join("Title.bmp"), b"stale bitmap title").test_value();
    fs::write(scenario_path.join("Title.png"), b"stale png title").test_value();
    fs::write(scenario_path.join("Icon.bmp"), b"stale bitmap icon").test_value();
    fs::write(scenario_path.join("DescDE.rtf"), b"stale description").test_value();
    fs::write(scenario_path.join("TitleUS.txt"), b"US:Stale title").test_value();

    let title = "Höhlenübung";
    let frontend = FrontendScenario {
        identifier: "Missions.c4f/01.c4s".to_string(),
        title: title.to_string(),
        path: Some(scenario_path.clone()),
        source_paths: vec![scenario_path.clone()],
        ..FrontendScenario::fallback()
    };
    let scenario_data =
        Scenario::load_from_path_with(&scenario_path, &InstallDefinitionResolver::new(None))
            .test_value();
    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    app.active_scenario = Some(frontend.clone());
    let player_info_id = app.engine.test_player(app.local_owner).player_info_id();
    app.control_player_infos.apply(netplay_player_info_data(
        0,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: player_info_id,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            game_number: app.local_owner,
            name: LegacyCString::from_bytes(b"Slot player".to_vec()).expect("slot player name"),
            ..Default::default()
        }],
    ));
    app.prepare_recording_for(&frontend, &scenario_data, None, None, None)
        .test_value();
    app.save_description_language = b"US".to_vec();
    let mut landscape = clonk_engine::Landscape::flat(2, 1);
    assert!(landscape.set_mode(clonk_engine::LANDSCAPE_MODE_EXACT));
    landscape.set_pixel_grid(clonk_engine::landscape::PixelGrid::new(
        2,
        1,
        vec![0, 1],
        vec![0; 256],
        vec![None; 256],
        vec![None; 256],
    ));
    app.engine.set_landscape(landscape);
    let mut state = app.engine.capture_state();
    state.frame = 37;
    state.game_time = 12;
    app.engine.restore_state(&state).test_value();
    app.snapshot = app.engine.snapshot();

    let slot = save_root.join("Missions.c4f").join("Missions10.c4s");
    fs::create_dir_all(slot.parent().test_value()).test_value();
    let mut stale = MutableGroup::new("Missions10.c4s");
    stale
        .add_file("Stale.txt", b"must be erased".to_vec())
        .test_value();
    fs::write(&slot, stale.pack().test_value()).test_value();

    let player_infos = app.recording_player_info_snapshot();
    assert_eq!(player_infos.clients.len(), 1);
    assert_eq!(player_infos.clients[0].players.len(), 1);
    let restore_plan = runtime_join_save::set_as_live_save_restore_infos(
        &app.control_clients.snapshot(),
        &player_infos,
        false,
        clonk_engine::LiveC4SavePolicy::Savegame {
            target_group_name: "Missions10.c4s",
        }
        .player_policy(),
    );
    assert_eq!(restore_plan.restore_infos.clients.len(), 1);

    app.save_to_slot(10);

    assert!(
        !app.status_text.starts_with("Save failed:"),
        "{}",
        app.status_text
    );
    assert!(slot.is_file(), "numbered save must be a packed file");
    let saved = Group::open(&slot).test_value();
    assert_eq!(
        saved.read_file("Source.bin").expect("copied source entry"),
        b"copied source sentinel"
    );
    assert!(!saved.exists("Stale.txt"));
    for component in [
        "Parameters.txt",
        "Scenario.txt",
        "Game.txt",
        "SavePlayerInfos.txt",
        "DescUS.rtf",
        "Title.png",
    ] {
        assert!(saved.exists(component), "missing native {component}");
    }
    for stale in ["Title.bmp", "Icon.bmp", "DescDE.rtf", "TitleUS.txt"] {
        assert!(!saved.exists(stale), "stale component survived: {stale}");
    }
    let scenario = String::from_utf8(saved.read_file("Scenario.txt").test_value()).test_value();
    assert!(scenario.contains("SaveGame=1\r\n"));
    assert!(scenario.contains("NoInitialize=1\r\n"));
    assert!(scenario.contains("Icon=11\r\n"));
    let game =
        clonk_engine::parse_initial_network_game_data(&saved.read_file("Game.txt").test_value());
    assert_eq!(game.frame, 37);
    assert_eq!(game.time, 12);

    let title_png = saved.read_file("Title.png").test_value();
    let decoder = png::Decoder::new(io::Cursor::new(title_png));
    let reader = decoder.read_info().test_value();
    assert_eq!(
        (reader.info().width, reader.info().height),
        (SAVE_THUMBNAIL_WIDTH, SAVE_THUMBNAIL_HEIGHT)
    );
    assert!(
        !slot.with_extension("png").exists(),
        "native slot must not write a sidecar thumbnail"
    );
    assert_eq!(
        fs::read(save_root.join("Title.txt")).expect("read root save title"),
        b"US:Savegames"
    );
    assert_eq!(
        fs::read(save_root.join("Missions.c4f/Title.txt")).expect("read scenario save title"),
        b"US:H\xc3\xb6hlen\xc3\xbcbung"
    );
    assert_eq!(
        app.active_scenario
            .as_ref()
            .and_then(|scenario| scenario.path.as_deref()),
        Some(scenario_path.as_path()),
        "QuickSave must not retarget the running scenario"
    );

    app.retained_gpu_presentation_active = true;
    let gpu_slot = save_root.join("Missions.c4f").join("Missions9.c4s");
    app.save_to_slot(9);
    assert_eq!(app.pending_native_save_thumbnails.len(), 1);
    assert_eq!(app.pending_native_save_thumbnails[0].path, gpu_slot);
    assert!(
        gpu_slot.exists(),
        "the game state save must remain synchronous"
    );
    assert!(!app.savegame_slots()[8].free);
    let mut later_state = app.engine.capture_state();
    later_state.frame = 91;
    app.engine.restore_state(&later_state).test_value();
    let gpu_title =
        encode_presented_save_thumbnail(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).test_value();
    app.finish_pending_native_save_thumbnails(Some(&gpu_title));
    assert!(app.pending_native_save_thumbnails.is_empty());
    let gpu_saved = Group::open(&gpu_slot).test_value();
    assert_eq!(
        gpu_saved
            .read_file("Title.png")
            .expect("read retained GPU title"),
        gpu_title
    );
    assert_eq!(
        clonk_engine::parse_initial_network_game_data(
            &gpu_saved.read_file("Game.txt").expect("read GPU save game"),
        )
        .frame,
        37,
        "thumbnail completion must not recapture later simulation state"
    );

    let guarded_slot = save_root.join("Missions.c4f").join("Missions8.c4s");
    app.save_to_slot(8);
    let mut replacement = MutableGroup::new("Missions8.c4s");
    replacement
        .add_file("External.txt", b"new generation".to_vec())
        .test_value();
    fs::write(&guarded_slot, replacement.pack().test_value()).test_value();
    app.finish_pending_native_save_thumbnails(Some(&gpu_title));
    let guarded = Group::open(&guarded_slot).test_value();
    assert!(guarded.exists("External.txt"));
    assert!(!guarded.exists("Title.png"));

    let teardown_slot = save_root.join("Missions.c4f").join("Missions7.c4s");
    app.save_to_slot(7);
    assert_eq!(
        app.pending_native_save_thumbnails
            .front()
            .map(|request| request.path.as_path()),
        Some(teardown_slot.as_path())
    );
    app.return_to_menu();
    assert!(app.pending_native_save_thumbnails.is_empty());
    assert_eq!(
        Group::open(&teardown_slot)
            .expect("open teardown-flushed slot")
            .read_file("Title.png")
            .expect("read preserved source title"),
        b"stale png title"
    );
}

#[test]
fn native_save_rejects_a_joined_player_without_a_runtime_section() {
    // C4PlayerList::Remove marks the retained info removed before deleting the
    // live player. SetAsRestoreInfos therefore cannot emit an IsJoined row
    // without C4Game emitting its Player<ID> section
    // (src/C4PlayerList.cpp:219-247; src/C4PlayerInfo.cpp:1637-1665;
    // src/C4Game.cpp:1987-1994).
    let fixture = tempdir();
    let scenario_path = fixture.path().join("Source.c4s");
    install_record_test_definitions(fixture.path());
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        b"[Head]\nTitle=Source scenario\n\n[Definitions]\nDefinition1=Objects.c4d\n",
    )
    .test_value();
    let frontend = FrontendScenario {
        identifier: "Source.c4s".to_string(),
        title: "Source scenario".to_string(),
        path: Some(scenario_path.clone()),
        source_paths: vec![scenario_path.clone()],
        ..FrontendScenario::fallback()
    };
    let scenario_data =
        Scenario::load_from_path_with(&scenario_path, &InstallDefinitionResolver::new(None))
            .test_value();
    let mut app = new_state_only_running_sandbox_app();
    app.active_scenario = Some(frontend.clone());
    app.prepare_recording_for(&frontend, &scenario_data, None, None, None)
        .test_value();
    let mut landscape = clonk_engine::Landscape::flat(1, 1);
    assert!(landscape.set_mode(clonk_engine::LANDSCAPE_MODE_EXACT));
    landscape.set_pixel_grid(clonk_engine::landscape::PixelGrid::new(
        1,
        1,
        vec![0],
        vec![0; 256],
        vec![None; 256],
        vec![None; 256],
    ));
    app.engine.set_landscape(landscape);
    app.control_player_infos.apply(netplay_player_info_data(
        3,
        vec![clonk_engine::ControlPlayerInfoEntry {
            id: 313,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            game_number: 2,
            name: LegacyCString::from_bytes(b"Departed".to_vec()).unwrap(),
            ..Default::default()
        }],
    ));
    let destination = fixture.path().join("Broken.c4s");

    let error = app
        .save_native_c4_game(ConsoleSaveKind::Savegame, Some(&destination), false, None)
        .expect_err("a mismatched live save must fail closed");

    assert!(
        error.to_string().contains(
            "exact save has joined SavePlayerInfos IDs without matching Game.txt Player<ID> sections: 313"
        ),
        "{error:#}"
    );
    assert!(!destination.exists());
}

#[test]
fn network_cleanup_records_cpp_network_error_bytes_before_clearing() {
    // OnClientDisconnect evaluates the localized host-loss message before
    // C4Network2::Clear invokes ChangeToLocal. RoundResults keeps the
    // original process-language bytes for later evaluation/save output
    // (src/C4Network2.cpp:1825-1833;
    // src/C4RoundResults.cpp:315-323).
    let mut app = new_state_only_running_sandbox_app();
    app.runtime_language_charset = RuntimeHelpCharset::Windows1252;

    app.record_network_error_round_result("Network: host André disconnected!");

    let engine_results = app.engine.snapshot().round_results;
    assert_eq!(
        engine_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        engine_results.network_result_message,
        b"Network: host Andr\xe9 disconnected!"
    );
    assert_eq!(
        app.snapshot.round_results, engine_results,
        "cleanup presentation sees the network verdict before teardown"
    );
}

#[test]
fn a_lockstep_stall_announces_itself_once_after_a_grace_period() {
    // A control stall is silent in C++: `DrawHoldMessages` prints only "Pause",
    // and only for `HaltCount`, which a stall never sets, so the world freezes
    // while rendering carries on at full frame rate. That is indistinguishable
    // from a hang and is the symptom behind legacyclonk/LegacyClonk#28,
    // "network games stop randomly". There is no C++ behaviour to preserve, so
    // the port says something -- but only after a grace period, or jitter on a
    // bad link would flash constantly, and only once per stall.
    let mut app = new_running_sandbox_app();
    let start = Instant::now();

    app.announce_network_stall(start).test_value();
    assert!(
        app.runtime_flash_message.is_none(),
        "a stall must not be announced the instant it begins"
    );

    app.announce_network_stall(start + Duration::from_millis(1_400))
        .test_value();
    assert!(
        app.runtime_flash_message.is_none(),
        "1.4s is still inside the grace period"
    );

    app.announce_network_stall(start + Duration::from_millis(1_600))
        .test_value();
    assert!(
        app.runtime_flash_message.is_some(),
        "a stall lasting past the grace period must be announced"
    );

    app.runtime_flash_message = None;
    app.announce_network_stall(start + Duration::from_millis(5_000))
        .test_value();
    assert!(
        app.runtime_flash_message.is_none(),
        "one notice per stall, not one per frame"
    );

    // Clearing the stall arms the next one.
    app.network_stall_since = None;
    let resumed = start + Duration::from_millis(6_000);
    app.announce_network_stall(resumed).test_value();
    app.announce_network_stall(resumed + Duration::from_millis(1_600))
        .test_value();
    assert!(
        app.runtime_flash_message.is_some(),
        "a fresh stall must be announced again"
    );
}

#[test]
fn a_long_catch_up_still_draws_at_the_render_floor() {
    // C++ thins rendering during catch-up by (behind + 15) / 20, so at a large
    // backlog it draws one frame in twenty or worse. Because a pass coalesces
    // several frames, consecutive passes can each decide to draw nothing, and a
    // recovering client shows a completely static picture -- the same "is it
    // hung?" symptom a silent stall produces. Spring pins draw to 2 Hz while
    // fast-forwarding instead; NETWORK_RENDER_FLOOR_FRAMES is that 2 Hz at the
    // 28 ms tick.
    let mut app = new_running_sandbox_app();

    // A pass that skipped everything, just under the floor, stays skipped: the
    // floor must not steal frames from the simulation while it is catching up.
    app.frames_since_redraw = NETWORK_RENDER_FLOOR_FRAMES - 1;
    let mut outcome = SimulationPassOutcome {
        did_update: true,
        executed_frames: 0,
        skipped_render_frames: 0,
        skip_redraw: true,
        immediate_network_retry: false,
        yielded_for_render: false,
    };
    apply_render_floor(&mut app, &mut outcome);
    assert!(
        outcome.skip_redraw,
        "the floor must not fire before the backlog of undrawn frames reaches it"
    );

    // One more executed frame crosses it, and the pass draws.
    outcome.executed_frames = 1;
    outcome.skip_redraw = true;
    apply_render_floor(&mut app, &mut outcome);
    assert!(
        !outcome.skip_redraw,
        "a client that has gone {NETWORK_RENDER_FLOOR_FRAMES} frames without \
         drawing must draw regardless of how far behind it is"
    );
    assert_eq!(
        app.frames_since_redraw, 0,
        "drawing resets the counter, so the floor is a rate and not a one-shot"
    );
}

/// No native window backend clears player controls on focus loss: Win32
/// deactivation only minimizes a fullscreen window (C4FullScreen.cpp:139-145),
/// X11 FocusOut/Unmap only clears `Application.Active` (:310-315), and the SDL
/// branch does not handle focus at all (:432-447). A synchronized
/// `ClearPressed` belongs to the explicit modal flows
/// (C4PlayerList.cpp:588-595), so Alt-Tab must add nothing to the session.
#[test]
fn focus_loss_does_not_submit_cpp_player_control() {
    let mut app = new_running_sandbox_app();
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    // Drain whatever joining the sandbox already queued.
    let _ = commands.take_submitted_local();

    app.handle_focus_lost().test_value();

    assert!(
        commands.take_submitted_local().is_empty(),
        "focus loss must not submit a network player control"
    );
    // The nonfatal UI/pointer cleanup still runs.
    assert!(app.pressed_engine_keys.is_empty());
    assert_eq!(app.ingame_pointer, None);
}

/// `Config.Network.MaxResSearchRecursion` defaults to 1 (C4Config.cpp:527-533)
/// and bounds `C4Network2Res::SearchLocal`'s candidate walk
/// (C4Network2Res.cpp:460-490). The live application has to load it and hand it
/// to client bootstrap, not merely expose it in the advanced editor.
#[test]
fn configured_max_resource_search_recursion_reaches_client_candidates() {
    let load = |body: Option<&str>| {
        let root = tempdir();
        let user_data = tempdir();
        fs::create_dir_all(root.path().join("planet/System.c4g")).test_value();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = test_app_paths();
        paths.ensure_user_dirs().test_value();
        if let Some(body) = body {
            fs::write(paths.config_file(), body).test_value();
        }
        (
            load_max_resource_search_recursion(Some(&paths)),
            client_settings_for_paths(
                SocketAddr::from(([127, 0, 0, 1], 11_112)),
                "Depth tester".to_string(),
                Some(&paths),
            )
            .max_resource_search_recursion,
        )
    };

    // The C++ default survives an absent config, an absent key and a
    // non-numeric value.
    assert_eq!(load(None), (1, 1));
    assert_eq!(load(Some("[Network]\nName=Tester\n")), (1, 1));
    assert_eq!(
        load(Some("[Network]\nMaxResSearchRecursion=deep\n")),
        (1, 1)
    );
    // A configured depth is loaded and carried into the client settings.
    assert_eq!(load(Some("[Network]\nMaxResSearchRecursion=0\n")), (0, 0));
    assert_eq!(load(Some("[Network]\nMaxResSearchRecursion=3\n")), (3, 3));
    // A negative depth cannot deepen the search.
    assert_eq!(load(Some("[Network]\nMaxResSearchRecursion=-2\n")), (0, 0));

    // The settings value reaches the bootstrap candidate walk.
    let config = clonk_network::ClientConfig::new("Depth tester", ParticipantKind::Player)
        .with_max_resource_search_recursion(3);
    assert_eq!(config.bootstrap_local_candidates.max_search_recursion(), 3);
    assert_eq!(
        clonk_network::ClientConfig::new("Depth tester", ParticipantKind::Player)
            .bootstrap_local_candidates
            .max_search_recursion(),
        1,
        "an unconfigured client keeps the native single-folder default"
    );
}

/// `C4Network2Res` creates dynamic groups, received files and temporary
/// download artifacts beneath `Config.Network.WorkPath`
/// (C4Config.cpp:527-533,1369-1374; C4Network2Res.cpp:1709-1775), so a
/// configured value has to move the staging directory as well as the wire
/// names. The value is a relative name under the network cache; anything that
/// could address a directory outside it keeps the native default.
#[test]
fn configured_network_work_path_controls_resource_staging_directory() {
    let staging = |body: Option<&str>| {
        let root = tempdir();
        let user_data = tempdir();
        fs::create_dir_all(root.path().join("planet/System.c4g")).test_value();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = test_app_paths();
        paths.ensure_user_dirs().test_value();
        if let Some(body) = body {
            fs::write(paths.config_file(), body).test_value();
        }
        let cache = paths.cache_dir().to_path_buf();
        let name = network_work_directory_name(Some(&paths));
        let directory = network_work_directory(Some(&paths)).test_value();
        let client = client_settings_for_paths(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Work-path tester".to_string(),
            Some(&paths),
        )
        .resource_directory;
        // Host and client staging agree, and both stay inside the cache.
        assert_eq!(directory, cache.join(&name));
        assert_eq!(client, cache.join(&name));
        assert!(directory.starts_with(&cache));
        name
    };

    // The native default survives an absent config and an absent/empty key.
    assert_eq!(staging(None), "Network");
    assert_eq!(staging(Some("[Network]\nName=Tester\n")), "Network");
    assert_eq!(staging(Some("[Network]\nWorkPath=\n")), "Network");
    // A plain relative name moves the staging directory.
    assert_eq!(staging(Some("[Network]\nWorkPath=NetCache\n")), "NetCache");
    assert_eq!(
        staging(Some("[Network]\nWorkPath=shared/net\n")),
        "shared/net"
    );

    // Unsafe values cannot escape the cache and keep the default.
    assert_eq!(staging(Some("[Network]\nWorkPath=..\n")), "Network");
    assert_eq!(staging(Some("[Network]\nWorkPath=../escape\n")), "Network");
    assert_eq!(staging(Some("[Network]\nWorkPath=net/../..\n")), "Network");
    assert_eq!(
        staging(Some("[Network]\nWorkPath=/tmp/escape\n")),
        "Network"
    );
    assert_eq!(staging(Some("[Network]\nWorkPath=./here\n")), "Network");
}

/// `C4Application::DoInit` builds the global asynchronous pool with exactly
/// `Config.General.ThreadPoolThreadCount` workers on every non-Windows target,
/// defaulting to 8 (C4Config.cpp:406-408; C4Application.cpp:152-159). Windows
/// uses the system pool, so the key is not read there.
#[cfg(not(windows))]
#[test]
fn configured_thread_pool_count_builds_runtime_with_requested_workers() {
    let workers = |body: Option<&str>| {
        let root = tempdir();
        let user_data = tempdir();
        fs::create_dir_all(root.path().join("planet/System.c4g")).test_value();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = test_app_paths();
        paths.ensure_user_dirs().test_value();
        if let Some(body) = body {
            fs::write(paths.config_file(), body).test_value();
        }
        load_thread_pool_thread_count(Some(&paths))
    };

    // The native default survives an absent config, key and unparsable value.
    assert_eq!(workers(None), 8);
    assert_eq!(workers(Some("[General]\nName=Tester\n")), 8);
    assert_eq!(workers(Some("[General]\nThreadPoolThreadCount=many\n")), 8);
    // Zero and negative counts cannot ask tokio for an invalid pool.
    assert_eq!(workers(Some("[General]\nThreadPoolThreadCount=0\n")), 8);
    assert_eq!(workers(Some("[General]\nThreadPoolThreadCount=-4\n")), 8);
    // A configured count is honoured verbatim.
    assert_eq!(workers(Some("[General]\nThreadPoolThreadCount=1\n")), 1);
    assert_eq!(workers(Some("[General]\nThreadPoolThreadCount=16\n")), 16);

    // The count reaches the runtime builder, and a zero request still falls
    // back rather than reaching tokio.
    use clonk_app_netplay::network::{
        network_runtime_worker_threads, set_network_runtime_worker_threads,
        DEFAULT_NETWORK_RUNTIME_WORKER_THREADS,
    };
    let restore = network_runtime_worker_threads();
    set_network_runtime_worker_threads(3);
    assert_eq!(network_runtime_worker_threads(), 3);
    set_network_runtime_worker_threads(0);
    assert_eq!(
        network_runtime_worker_threads(),
        DEFAULT_NETWORK_RUNTIME_WORKER_THREADS
    );
    assert_eq!(DEFAULT_NETWORK_RUNTIME_WORKER_THREADS, 8);
    set_network_runtime_worker_threads(restore);
}

/// The `#ifndef NDEBUG` shortcuts in `C4Game::ParseCommandLine`
/// (C4Game.cpp:3288-3304): `/host` stands up a lobby on the fixed pair with
/// both signups off, and `/client:N` joins localhost on
/// TCP `11112 + 2 * (N + 1)` / UDP `11113 + 2 * (N + 1)` using `atoi`.
#[test]
fn debug_classic_host_client_arguments_apply_cpp_lobby_ports() {
    let parse = |argument: &str| parse_classic_command_line(&[argument.to_string().into()]);

    if !DEBUG_CLASSIC_SHORTCUTS {
        // A release build must not expose behaviour C++ compiles out.
        let host = parse("/host");
        assert_eq!(host.network_active, None);
        assert_eq!(host.tcp_port, None);
        let client = parse("/client:0");
        assert_eq!(client.direct_join, None);
        return;
    }

    let host = parse("/host");
    assert_eq!(host.network_active, Some(true));
    assert_eq!(host.lobby_timeout, Some(None));
    assert_eq!(host.tcp_port, Some(11_112));
    assert_eq!(host.udp_port, Some(11_113));
    assert_eq!(host.master_server_signup, Some(false));
    assert_eq!(host.league_server_signup, Some(false));
    // `/host` does not set a join address; it is the host.
    assert_eq!(host.direct_join, None);
    // Case-insensitive, like SEqualNoCase.
    assert_eq!(parse("/HOST").tcp_port, Some(11_112));

    // `/client:N` targets localhost with a lobby and the derived pair.
    for (index, tcp, udp) in [
        ("0", 11_114, 11_115),
        ("1", 11_116, 11_117),
        ("2", 11_118, 11_119),
        ("7", 11_128, 11_129),
    ] {
        let client = parse(&format!("/client:{index}"));
        assert_eq!(client.network_active, Some(true));
        assert_eq!(client.direct_join.as_deref(), Some("localhost"));
        assert_eq!(client.lobby_timeout, Some(None));
        assert_eq!(client.tcp_port, Some(tcp), "client {index} TCP");
        assert_eq!(client.udp_port, Some(udp), "client {index} UDP");
        // Unlike `/host`, signup state is left alone.
        assert_eq!(client.master_server_signup, None);
        assert_eq!(client.league_server_signup, None);
    }
    assert_eq!(parse("/CLIENT:1").tcp_port, Some(11_116));

    // `atoi` takes the leading decimal prefix and yields zero without one, so
    // these behave like index 0 rather than failing the argument.
    for value in ["", "abc", "0x2"] {
        let client = parse(&format!("/client:{value}"));
        assert_eq!(client.tcp_port, Some(11_114), "/client:{value}");
    }
    // A trailing suffix keeps the numeric prefix.
    assert_eq!(parse("/client:3rd").tcp_port, Some(11_120));
}

#[test]
fn network_host_own_join_binds_the_local_presentation_to_its_player() {
    // C4Game::JoinPlayer binds the local presentation to the number the join
    // actually produced: `if (pPlr->LocalControl) CreateViewport(pPlr->Number)`
    // (pristine 9ffa0a5d src/C4Game.cpp:3544-3556), and C4Player::FinalInit
    // runs `Game.MouseControl.Init(Number)` for a locally controlled player
    // (src/C4Player.cpp:784-791). A network host joins before every client, so
    // C4PlayerList::GetFreeNumber hands it player 0 while `local_owner` still
    // holds the process default. Mouse commands, HUD lookup and menu ownership
    // all read `local_owner`, so it has to follow the join.
    let directory = tempdir();
    let player_path = directory.path().join("Host.c4p");
    let mut player_group = MutableGroup::new("Host.c4p");
    player_group
        .add_file(
            "Player.txt",
            b"[Player]\nName=Host\n[Preferences]\nColorDw=255\nControl=0\n".to_vec(),
        )
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();

    let mut app = new_state_only_running_sandbox_app();
    // A real network host owns no runtime player until its own synchronized
    // JoinPlayer executes; the sandbox fixture pre-registers one.
    app.remove_local_control_assignment(app.local_owner);
    app.engine.remove_player(app.local_owner).test_value();
    app.engine.set_local_players([]);
    let (manager, _event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.engine.set_network_game(true);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_clients.register(0, true, false);

    let resource_id = 6;
    let core = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: resource_id,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Host.c4p".to_vec()).test_value(),
        ..clonk_engine::NetworkResourceCore::default()
    };
    app.admission_resources
        .mark_complete(resource_id, player_path.clone());
    let info_id = 1;
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: info_id,
                name: LegacyCString::from_bytes(b"Host".to_vec()).expect("valid player name"),
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            by_client: 0,
            ..Default::default()
        });

    app.apply_join_player_control(clonk_engine::JoinPlayerControlData {
        filename: LegacyCString::from_bytes(b"Host.c4p".to_vec()).expect("valid legacy filename"),
        at_client: 0,
        info_id,
        source: clonk_engine::JoinPlayerSource::Resource(core),
        by_client: 0,
    })
    .test_value();

    let joined = app
        .engine
        .players()
        .find(|player| player.player_info_id() == info_id)
        .map(|player| player.id())
        .test_value();
    assert_eq!(
        app.engine.snapshot().hud.local_players,
        vec![joined],
        "the joined host player is the only local player"
    );
    assert_eq!(
        app.local_owner, joined,
        "local presentation must follow the host's own joined player number"
    );
}

#[test]
fn network_client_routes_player_targeted_sound_only_to_its_local_player() {
    // C4Player::LocalControl is derived from the joined player's AtClient and
    // the process-local client (C4Player.cpp:1871-1877). A provisional owner
    // from scenario activation must not survive after that number is assigned
    // to a remote player, or Sound(..., iAtPlayer) leaks that player's global
    // loops onto this client (C4Script.cpp:2297-2309).
    let mut app = new_state_only_running_sandbox_app();
    let provisional_owner = app.local_owner;
    app.remove_local_control_assignment(provisional_owner);
    app.engine.remove_player(provisional_owner).test_value();
    app.engine.set_local_players([provisional_owner]);

    let (manager, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.engine.set_network_game(true);
    app.engine
        .load_scenario_script_with_convention(
            "player-targeted sound fixture",
            concat!(
                "#strict 3\n",
                "global func InitializePlayer(int plr) {\n",
                "  if (plr == 0) Sound(\"Join0\", true, nil, 100, 1);\n",
                "  if (plr == 1) Sound(\"Join1\", true, nil, 100, 2);\n",
                "  if (plr == 2) Sound(\"Join2\", true, nil, 100, 3);\n",
                "}\n",
                "global func ProbeRemote(int plr) { Sound(\"Warning_lowoxygen\", true, nil, 100, plr + 1, 1); }\n",
            ),
            true,
        ).test_value();
    app.engine.pending_audio.clear();
    for client in [0, 3, 7] {
        app.control_clients.register(client, true, false);
    }

    let packed_player = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ))
    .test_value();
    for (info_id, at_client, name) in [
        (10, 0, "Remote host"),
        (11, 3, "Remote client"),
        (12, 7, "Local client"),
    ] {
        app.control_player_infos
            .apply(clonk_engine::PlayerInfoControlData {
                client_id: at_client,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: info_id,
                    name: LegacyCString::from_bytes(name.as_bytes().to_vec())
                        .expect("valid player name"),
                    ..Default::default()
                }],
                by_client: 0,
                ..Default::default()
            });
        app.apply_join_player_control(clonk_engine::JoinPlayerControlData {
            filename: LegacyCString::from_bytes(format!("Player{info_id}.c4p").into_bytes())
                .expect("valid player filename"),
            at_client,
            info_id,
            source: clonk_engine::JoinPlayerSource::Embedded(packed_player.clone()),
            by_client: 0,
        })
        .test_value();
    }

    let player_by_info = |app: &GameApp, info_id| {
        app.engine
            .players()
            .find(|player| player.player_info_id() == info_id)
            .map(|player| player.id())
            .test_value()
    };
    let remote_player = player_by_info(&app, 11);
    let local_player = player_by_info(&app, 12);
    let join_sounds = app
        .engine
        .pending_audio
        .iter()
        .filter_map(|command| match command {
            clonk_engine::AudioCommand::PlaySound { name, .. } if name.starts_with("Join") => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        join_sounds,
        vec![format!("Join{local_player}")],
        "InitializePlayer audio must use the join's authoritative local assignment; all audio: {:?}",
        app.engine.pending_audio
    );
    assert_eq!(
        app.engine.snapshot().hud.local_players,
        vec![local_player],
        "remote players must not pass player-targeted sound's local-client gate"
    );

    app.engine.tick_without_snapshot().test_value();
    assert!(
        !app.engine
            .player(remote_player)
            .expect("remote player remains")
            .viewports()
            .is_empty(),
        "the regression must cover a remote player's logical simulation viewport"
    );
    app.engine.pending_audio.clear();
    app.engine
        .call_scenario_script_function("ProbeRemote", vec![Value::Int(remote_player)])
        .test_value();
    assert!(
        !app.engine.pending_audio.iter().any(|command| matches!(
            command,
            clonk_engine::AudioCommand::PlaySound { name, .. }
                if name == "Warning_lowoxygen"
        )),
        "a remote logical viewport is not a process-local graphics viewport"
    );
}

#[test]
fn losing_the_last_local_viewport_flashes_the_native_observer_hint() {
    // C4FullScreen::ViewportCheck's no-viewport case creates the ownerless
    // observer viewport and then, outside film mode, flashes
    // IDS_MSG_PRESSORPUSHANYGAMEPADBUTT with the FullscreenMenuOpen key name
    // wrapped in the yellow markup (pristine 9ffa0a5d
    // src/C4FullScreen.cpp:499-527). C4Game::InitKeyboard registers that key
    // on K_SPACE (src/C4Game.cpp:3428). Without the hint an eliminated player
    // only sees their controls stop working.
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let expected = format_resource_string(
        app.runtime_flash_resources()
            .test_value()
            .observer_menu
            .clone(),
        &["<c ffff00><Space></c>"],
    );
    app.runtime_flash_message = None;

    assert!(app.close_physical_viewports(owner, false, true));
    app.check_fullscreen_physical_viewports(true);

    assert!(
        app.primary_physical_viewport_is_no_owner(),
        "the fullscreen fallback owns an ownerless observer viewport"
    );
    assert_eq!(
        app.runtime_flash_message
            .as_ref()
            .map(|message| message.text.clone()),
        Some(expected)
    );
}
fn netplay_player_info_data(
    client_id: i32,
    players: Vec<clonk_engine::ControlPlayerInfoEntry>,
) -> clonk_engine::PlayerInfoControlData {
    clonk_engine::PlayerInfoControlData {
        client_id,
        players,
        ..Default::default()
    }
}

fn test_prepared_go(
    control_mode: i32,
    local_reached: bool,
    save_game: bool,
    network_runtime_join: bool,
    restore_player_infos: Vec<clonk_engine::ControlPlayerInfoEntry>,
    runtime_join_players: Vec<clonk_engine::RuntimeJoinPlayerSource>,
    team_registry: Vec<clonk_engine::TeamInfo>,
) -> PreparedGoLoadingState {
    PreparedGoLoadingState {
        status: clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode,
            target_tick: 0,
        },
        local_reached,
        save_game,
        network_runtime_join,
        restore_player_infos,
        runtime_join_players,
        pending_client_runtime_join: None,
        initial_game_data: None,
        random_seed: 0,
        use_fair_crew: false,
        fair_crew_strength: 0,
        fair_crew_forced: false,
        allow_debug: true,
        auto_frame_skip: true,
        synchronized_rule_goal_lists: clonk_engine::GameParameterRuleGoalLists::new(
            Vec::new(),
            Vec::new(),
        ),
        team_configuration: TeamConfiguration::default(),
        team_registry,
        definition_modules: None,
    }
}

fn test_loading_state(
    scenario: FrontendScenario,
    receiver: mpsc::Receiver<ScenarioLoadingEvent>,
    finished: bool,
    prepared_go: PreparedGoLoadingState,
) -> ScenarioLoadingState {
    ScenarioLoadingState {
        scenario,
        refreshed_resources: None,
        refreshed_tooltip_font: None,
        refreshed_native_font_source: None,
        refreshed_global_gui_failures: None,
        refreshed_gui_sheet_overrides: None,
        refresh_requested: false,
        receiver,
        finished,
        last_progress: 0,
        log: Vec::new(),
        prepared_go: Some(prepared_go),
        offline_startup_players: None,
        offline_savegame: None,
        offline_random_seed: None,
    }
}
