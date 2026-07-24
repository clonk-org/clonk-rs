// Contiguous slice 3 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: sound, object state, menus and messages.

    #[test]
    fn custom_message_empty_string_registers_clear_command_like_cpp() {
        // FnCustomMessage forwards a non-null empty C4String to
        // C4GameMessageList::New, which performs a clear-only operation and
        // returns true (C4Script.cpp:5995-6039; C4GameMessage.cpp:290-305).
        let args = [
            Value::String(String::new().into()),
            Value::Nil,
            Value::Int(0),
            Value::Int(10),
            Value::Int(-30),
            Value::Int(0x00ff_ffff),
            Value::Nil,
            Value::Nil,
            Value::Int((FLAG_BOTTOM | FLAG_LEFT | FLAG_X_REL | FLAG_WIDTH_REL) as i32),
            Value::Int(35),
        ];
        let (result, outcome) = with_object_host_context(|| custom_message(&args));

        assert_eq!(
            result.expect("empty CustomMessage succeeds"),
            Value::Bool(true)
        );
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert!(spec.text.is_empty());
                assert_eq!(spec.kind, MessageKind::GlobalPlayer);
                assert_eq!(spec.player, Some(0));
                assert_eq!(
                    spec.flags,
                    FLAG_BOTTOM | FLAG_LEFT | FLAG_X_REL | FLAG_WIDTH_REL
                );
            }
            MessageCommand::PendingSpeech(_) => panic!("CustomMessage cannot defer speech"),
        }
    }

    #[test]
    fn custom_message_falsy_portrait_becomes_a_null_string_pointer() {
        // Legacy callers eagerly turn every falsy argument into C4V_Any nil
        // before converting a C4String* parameter (C4AulExec.cpp:1364-1396).
        // Western's ExtraLog passes integer zero in the portrait slot.
        for portrait in [Value::Int(0), Value::Bool(false)] {
            let args = [
                Value::String("Log line".into()),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                portrait,
            ];
            let (result, outcome) = with_object_host_context(|| custom_message(&args));
            assert_eq!(result.expect("CustomMessage succeeds"), Value::Bool(true));
            assert_eq!(outcome.messages.len(), 1);
            match &outcome.messages[0] {
                MessageCommand::Add(spec) => assert!(spec.portrait.is_none()),
                MessageCommand::PendingSpeech(_) => panic!("CustomMessage cannot defer speech"),
            }
        }
    }

    #[test]
    fn custom_message_c4id_decoration_must_resolve_like_cpp() {
        // C++ returns false before creating a message when idDeco is nonzero
        // but C4Id2Def cannot resolve it (C4Script.cpp:6002).
        let world =
            HostWorldContext::default().with_definition_metadata(Rc::new(HashMap::from([(
                "DECO".into(),
                DefinitionMetadata::default(),
            )])));
        let args = [
            Value::String("Welcome".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::C4Id("NOPE".into()),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || custom_message(&args));

        assert_eq!(
            result.expect("unknown decoration is not an error"),
            Value::Bool(false)
        );
        assert!(outcome.messages.is_empty());
    }

    #[test]
    fn custom_message_rejects_string_in_c4id_decoration_slot() {
        // String -> C4ID is an unconditional conversion error in the C++
        // conversion table (C4Value.cpp:550-561), even for old syntax.
        let args = [
            Value::String("Welcome".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::String("DECO".into()),
        ];
        let (result, outcome) = with_object_host_context(|| custom_message(&args));

        let error = result.expect_err("string decoration must not coerce to C4ID");
        assert!(error.message().contains("expected C4ID"));
        assert!(outcome.messages.is_empty());
    }

    #[test]
    fn message_family_speech_uses_cpp_anchor_precedence() {
        // FnMessage ignores pObj for speech and uses cthr->Obj;
        // FnPlayerMessage prefers pObj; FnPlrMessage uses cthr->Obj
        // unconditionally (C4Script.cpp:2395-2463).
        let mut audio = AudioRegistry::new();
        audio.set_available_samples(["messagespeech.WAV", "PlayerSpeech.wav", "PlrSpeech.wav"]);
        let audio_guard = enter_audio_context(audio);
        let caller = ObjectId::new(1);
        let target = ObjectId::new(2);
        let (result, outcome) = with_object_host_context(|| {
            let message_result = message(&[
                // Successful speech bypasses FnStringFormat entirely, so
                // this missing %d argument must not abort the call.
                Value::String("Hello %d$MessageSpeech".into()),
                object_reference_value(target),
            ])?;
            let player_result = player_message(&[
                Value::Int(0),
                Value::String("Hello$PlayerSpeech".into()),
                object_reference_value(target),
            ])?;
            let plr_result =
                plr_message(&[Value::String("Hello$PlrSpeech".into()), Value::Int(0)])?;
            Ok::<_, RuntimeError>((message_result, player_result, plr_result))
        });
        let _ = audio_guard.finish();
        assert_eq!(
            result.expect("message speech calls succeed"),
            (Value::Bool(true), Value::Bool(true), Value::Bool(true))
        );
        assert_eq!(outcome.messages.len(), 2);
        assert!(matches!(
            &outcome.messages[0],
            MessageCommand::PendingSpeech(SpeechFallback { message, .. })
                if message.kind == MessageKind::TargetPlayer
                    && message.target == Some(target)
                    && message.text == "Hello"
        ));
        assert!(matches!(
            &outcome.messages[1],
            MessageCommand::PendingSpeech(SpeechFallback { message, .. })
                if message.kind == MessageKind::Global
                    && message.target.is_none()
                    && message.player.is_none()
                    && message.text == "Hello"
        ));
        assert!(matches!(
            outcome.audio.events.as_slice(),
            [
                AudioCommand::PlaySpeech {
                    name: message_name,
                    target: Some(message_target),
                    fallback: None,
                },
                AudioCommand::PlaySpeech {
                    name: player_name,
                    target: Some(player_target),
                    fallback: Some(_),
                },
                AudioCommand::PlaySpeech {
                    name: plr_name,
                    target: Some(plr_target),
                    fallback: Some(_),
                },
            ] if message_name == "MessageSpeech"
                && *message_target == caller
                && player_name == "PlayerSpeech"
                && *player_target == target
                && plr_name == "PlrSpeech"
                && *plr_target == caller
        ));
    }

    #[test]
    fn message_family_speech_keeps_null_definition_caller_anchor() {
        // Definition-commanded effects carry a mutable affected object while
        // cthr->Obj remains null. Speech follows the latter, not the carrier.
        let mut audio = AudioRegistry::new();
        audio.set_available_samples(["MessageSpeech.wav", "PlayerSpeech.wav", "PlrSpeech.wav"]);
        let audio_guard = enter_audio_context(audio);
        let target = ObjectId::new(2);
        let (result, outcome) = with_effect_context_with_state_and_definition(
            Some(object_host_context_with_physical_energy(100, 100)),
            Some(DefinitionId::from("CALL")),
            None,
            &[],
            HostWorldContext::default(),
            2,
            false,
            || {
                Ok::<_, RuntimeError>((
                    message(&[
                        Value::String("Hello$MessageSpeech".into()),
                        object_reference_value(target),
                    ])?,
                    player_message(&[Value::Int(0), Value::String("Hello$PlayerSpeech".into())])?,
                    plr_message(&[Value::String("Hello$PlrSpeech".into()), Value::Int(0)])?,
                ))
            },
        );
        let _ = audio_guard.finish();

        assert_eq!(
            result.expect("definition message speech calls succeed"),
            (Value::Bool(true), Value::Bool(true), Value::Bool(true))
        );
        assert!(matches!(
            outcome.audio.events.as_slice(),
            [
                AudioCommand::PlaySpeech { target: None, .. },
                AudioCommand::PlaySpeech { target: None, .. },
                AudioCommand::PlaySpeech { target: None, .. },
            ]
        ));
    }

    #[test]
    fn message_family_missing_speech_falls_back_to_text() {
        // StartSoundEffect fails when C4SoundSystem has no matching sample;
        // all three natives then format and display segment zero instead of
        // emitting an audio request (C4Script.cpp:2395-2463;
        // C4SoundSystem.cpp:301-320).
        let mut audio = AudioRegistry::new();
        audio.set_available_samples(std::iter::empty::<&str>());
        let audio_guard = enter_audio_context(audio);

        let mut player = PlayerState::default();
        player.id = 7;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_object_host_context_with_world(world, || {
            Ok::<_, RuntimeError>((
                message(&[Value::String("Hello 1$NoMessageSpeech$".into())])?,
                player_message(&[
                    Value::Int(7),
                    Value::String("Hello 2$NoPlayerSpeech$".into()),
                ])?,
                plr_message(&[Value::String("Hello 3$NoPlrSpeech$".into()), Value::Int(7)])?,
            ))
        });
        let _ = audio_guard.finish();

        assert_eq!(
            result.expect("missing speech falls back"),
            (Value::Bool(true), Value::Bool(true), Value::Bool(true))
        );
        assert!(outcome.audio.events.is_empty());
        assert_eq!(outcome.messages.len(), 3);
        assert!(matches!(
            &outcome.messages[0],
            MessageCommand::Add(MessageSpec {
                kind: MessageKind::Global,
                text,
                target: None,
                player: None,
                ..
            }) if text == "Hello 1"
        ));
        assert!(matches!(
            &outcome.messages[1],
            MessageCommand::Add(MessageSpec {
                kind: MessageKind::GlobalPlayer,
                text,
                target: None,
                player: Some(7),
                ..
            }) if text == "Hello 2"
        ));
        assert!(matches!(
            &outcome.messages[2],
            MessageCommand::Add(MessageSpec {
                kind: MessageKind::GlobalPlayer,
                text,
                target: None,
                player: Some(7),
                ..
            }) if text == "Hello 3"
        ));

        for (command, expected) in outcome
            .messages
            .iter()
            .cloned()
            .zip(["Hello 1", "Hello 2", "Hello 3"])
        {
            let mut messages = crate::message::MessageManager::new();
            messages.apply_command(command);
            let snapshot = messages.snapshot();
            assert_eq!(snapshot.len(), 1);
            assert_eq!(snapshot[0].lines, vec![expected.to_string()]);
        }
    }

    #[test]
    fn message_speech_catalog_matches_cpp_prepared_filenames() {
        let invalid_star_matches = HashSet::from(["foo.wav".to_string(), "foo12.wav".to_string()]);
        assert!(!sound_sample_available(&invalid_star_matches, "Foo*"));
        let mut valid_star_matches = invalid_star_matches;
        valid_star_matches.insert("foo1.wav".to_string());
        assert!(sound_sample_available(&valid_star_matches, "Foo*"));

        let samples = HashSet::from([
            "voice.wav".to_string(),
            "encoded.ogg".to_string(),
            "blast1.wav".to_string(),
        ]);
        assert!(sound_sample_available(&samples, "VOICE"));
        assert!(sound_sample_available(&samples, "encoded.ogg"));
        assert!(!sound_sample_available(&samples, "encoded"));
        assert!(sound_sample_available(&samples, "Blast*"));
        assert!(!sound_sample_available(&samples, "Blast??"));
        assert!(!sound_sample_available(&samples, "sub/VOICE"));
    }

    #[test]
    fn sound_threads_the_cpp_multiple_flag_to_playback() {
        // FnSound passes fMultiple through the sound-system decision after its
        // exact-object IsSoundPlaying check (C4Script.cpp:2297, 2317-2322).
        let args = [
            Value::String("HorseWalk*".into()),
            Value::Bool(false),
            Value::Nil,
            Value::Int(100),
            Value::Nil,
            Value::Int(0),
            Value::Bool(true),
        ];
        let (result, outcome) = with_object_host_context(|| sound(&args));

        assert_eq!(result.expect("Sound succeeds"), Value::Bool(true));
        assert!(matches!(
            outcome.audio.events.as_slice(),
            [AudioCommand::PlaySound { multiple: true, .. }]
        ));
    }

    #[test]
    fn sound_preserves_negative_custom_falloff() {
        // FnSound forwards every nonzero signed falloff distance unchanged;
        // zero alone selects C4SoundSystem's ordinary audibility radius
        // (C4Script.cpp:2297-2323; C4SoundSystem.cpp:194-200).
        let (result, outcome) = with_object_host_context(|| {
            let call = |name: &str, custom_falloff: i32| {
                sound(&[
                    Value::String(name.into()),
                    Value::Bool(false),
                    Value::Nil,
                    Value::Int(100),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Bool(false),
                    Value::Int(custom_falloff),
                ])
            };
            assert_eq!(call("NegativeFalloff", -700)?, Value::Bool(true));
            assert_eq!(call("DefaultFalloff", 0)?, Value::Bool(true));
            Ok::<Value, RuntimeError>(Value::Nil)
        });

        result.expect("Sound custom-falloff probes run");
        assert_eq!(
            outcome.audio.events,
            vec![
                AudioCommand::PlaySound {
                    name: "NegativeFalloff".into(),
                    target: Some(ObjectId::new(1)),
                    volume: 100,
                    looped: false,
                    multiple: false,
                    custom_falloff: Some(-700),
                },
                AudioCommand::PlaySound {
                    name: "DefaultFalloff".into(),
                    target: Some(ObjectId::new(1)),
                    volume: 100,
                    looped: false,
                    multiple: false,
                    custom_falloff: None,
                },
            ]
        );
    }

    #[test]
    fn sound_at_player_gates_playback_to_local_players_and_viewports() {
        // iAtPlayer is one-based and gates playback on each client. A valid
        // remote player remains a successful sync-safe no-op unless this
        // client owns a viewport for it (C4Script.cpp:2297-2309).
        let players = vec![
            PlayerState {
                id: 0,
                ..PlayerState::default()
            },
            PlayerState {
                id: 1,
                ..PlayerState::default()
            },
            PlayerState {
                id: 2,
                viewports: vec![PlayerViewport::new(Vector2::ZERO)],
                ..PlayerState::default()
            },
        ];
        let world =
            HostWorldContext::from_objects_with_players(Vec::<HostWorldObject>::new(), players)
                .with_local_players([0]);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            let call = |name: &str, at_player: i32| {
                sound(&[
                    Value::String(name.into()),
                    Value::Bool(true),
                    Value::Nil,
                    Value::Int(100),
                    Value::Int(at_player),
                ])
            };

            assert_eq!(call("DingInvalid", 100)?, Value::Bool(false));
            assert_eq!(call("DingRemote", 2)?, Value::Bool(true));
            assert_eq!(call("DingLocal", 1)?, Value::Bool(true));
            assert_eq!(call("DingViewport", 3)?, Value::Bool(true));
            Ok::<Value, RuntimeError>(Value::Nil)
        });

        result.expect("Sound at-player probes run");
        let played = outcome
            .audio
            .events
            .iter()
            .filter_map(|event| match event {
                AudioCommand::PlaySound { name, target, .. } => {
                    assert_eq!(*target, None);
                    Some(name.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(played, vec!["DingLocal", "DingViewport"]);
    }

    #[test]
    fn sound_emits_pending_requests_for_frontend_dedup_and_negative_level_is_a_noop() {
        let (result, outcome) = with_object_host_context(|| {
            assert_eq!(sound(&[Value::String("Hit".into())])?, Value::Bool(true));
            assert_eq!(sound(&[Value::String("Hit".into())])?, Value::Bool(true));

            let start_loop = [
                Value::String("Loop".into()),
                Value::Bool(false),
                Value::Nil,
                Value::Int(100),
                Value::Int(0),
                Value::Int(1),
            ];
            assert_eq!(sound(&start_loop)?, Value::Bool(true));
            let negative_level_stop = [
                Value::String("Loop".into()),
                Value::Bool(false),
                Value::Nil,
                Value::Int(-1),
                Value::Int(0),
                Value::Int(-1),
            ];
            assert_eq!(sound(&negative_level_stop)?, Value::Bool(true));
            Ok::<Value, RuntimeError>(Value::Nil)
        });

        result.expect("Sound command probes run");
        let played = outcome
            .audio
            .events
            .iter()
            .filter_map(|event| match event {
                AudioCommand::PlaySound { name, .. } => Some(name.as_str()),
                AudioCommand::StopSound { .. } => panic!("negative level must not stop audio"),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(played, vec!["Hit", "Hit", "Loop"]);
    }

    #[test]
    fn sound_level_uses_global_default_and_stops_any_playing_instance() {
        let target = ObjectId::new(1);
        let target_value = object_reference_value(target);
        let (result, outcome) = with_object_host_context(|| {
            assert_eq!(
                sound_level(&[Value::String("Global".into()), Value::Int(50)])?,
                Value::Nil
            );
            sound(&[
                Value::String("Shot".into()),
                Value::Bool(false),
                target_value.clone(),
                Value::Int(100),
            ])?;
            assert_eq!(
                sound_level(&[Value::String("Shot".into()), Value::Int(0), target_value,])?,
                Value::Nil
            );
            Ok::<_, RuntimeError>(Value::Nil)
        });

        result.expect("SoundLevel probes run");
        assert_eq!(
            outcome.audio.events,
            vec![
                AudioCommand::SetSoundVolume {
                    name: "Global".into(),
                    target: None,
                    volume: 50,
                },
                AudioCommand::PlaySound {
                    name: "Shot".into(),
                    target: Some(target),
                    volume: 100,
                    looped: false,
                    multiple: false,
                    custom_falloff: None,
                },
                AudioCommand::StopSound {
                    name: "Shot".into(),
                    target: Some(target),
                },
            ]
        );
    }

    #[test]
    fn sound_level_preserves_positive_values_above_100() {
        let (result, outcome) = with_object_host_context(|| {
            sound_level(&[Value::String("Wind".into()), Value::Int(140)])
        });

        assert_eq!(result.expect("SoundLevel accepts level 140"), Value::Nil);
        assert_eq!(
            outcome.audio.events,
            vec![AudioCommand::SetSoundVolume {
                name: "Wind".into(),
                target: None,
                volume: 140,
            }]
        );
    }

    #[test]
    fn sound_level_controls_prior_frame_one_shots_without_phantom_loops() {
        let mut audio = AudioRegistry::new();
        audio.play_sound("VolumeShot", None, 100, false, false, None);
        audio.play_sound("StopShot", None, 100, false, false, None);
        assert_eq!(audio.take_events().len(), 2, "one-shots leave the frame");

        audio.sound_level("VolumeShot", None, 50);
        audio.sound_level("StopShot", None, 0);

        assert_eq!(
            audio.take_events(),
            vec![
                AudioCommand::SetSoundVolume {
                    name: "VolumeShot".into(),
                    target: None,
                    volume: 50,
                },
                AudioCommand::StopSound {
                    name: "StopShot".into(),
                    target: None,
                },
            ]
        );
    }

    #[test]
    fn sound_level_and_followup_sound_are_emitted_in_frontend_order() {
        let mut audio = AudioRegistry::new();
        audio.sound_level("Shot", None, 50);
        audio.play_sound("Shot", None, 100, true, false, None);

        assert_eq!(
            audio.take_events(),
            vec![
                AudioCommand::SetSoundVolume {
                    name: "Shot".into(),
                    target: None,
                    volume: 50,
                },
                AudioCommand::PlaySound {
                    name: "Shot".into(),
                    target: None,
                    volume: 100,
                    looped: true,
                    multiple: false,
                    custom_falloff: None,
                },
            ]
        );
    }

    #[test]
    fn sound_aliases_and_wildcards_are_always_emitted_for_frontend_arbitration() {
        let (result, outcome) = with_object_host_context(|| {
            let call = |name: &str, loop_flag: i32| {
                sound(&[
                    Value::String(name.into()),
                    Value::Bool(false),
                    Value::Nil,
                    Value::Int(100),
                    Value::Int(0),
                    Value::Int(loop_flag),
                ])
            };

            call("Fire", 1)?;
            call("Fire.wav", -1)?;
            call("Fire", 1)?;
            call("Fire.wav", 1)?;
            call("Blast*", 1)?;
            call("Blast*", 1)?;
            Ok::<_, RuntimeError>(Value::Nil)
        });

        result.expect("alias and wildcard Sound calls succeed");
        let target = Some(ObjectId::new(1));
        let play = |name: &str| AudioCommand::PlaySound {
            name: name.into(),
            target,
            volume: 100,
            looped: true,
            multiple: false,
            custom_falloff: None,
        };
        assert_eq!(
            outcome.audio.events,
            vec![
                play("Fire"),
                AudioCommand::StopSound {
                    name: "Fire.wav".into(),
                    target,
                },
                play("Fire"),
                play("Fire.wav"),
                play("Blast*"),
                play("Blast*"),
            ]
        );
    }

    #[test]
    fn remove_object_detaches_target_sounds_in_native_event_order() {
        let target = ObjectId::new(1);
        let (result, outcome) = with_object_host_context(|| {
            sound(&[
                Value::String("GlobalLoop".into()),
                Value::Bool(true),
                Value::Nil,
                Value::Int(100),
                Value::Int(0),
                Value::Int(1),
            ])?;
            sound(&[
                Value::String("Fire".into()),
                Value::Bool(false),
                Value::Nil,
                Value::Int(100),
                Value::Int(0),
                Value::Int(1),
            ])?;
            sound(&[Value::String("Impact".into())])?;
            remove_object(&[])
        });

        assert_eq!(result.expect("RemoveObject succeeds"), Value::Bool(true));
        assert_eq!(
            outcome.audio.events,
            vec![
                AudioCommand::PlaySound {
                    name: "GlobalLoop".into(),
                    target: None,
                    volume: 100,
                    looped: true,
                    multiple: false,
                    custom_falloff: None,
                },
                AudioCommand::PlaySound {
                    name: "Fire".into(),
                    target: Some(target),
                    volume: 100,
                    looped: true,
                    multiple: false,
                    custom_falloff: None,
                },
                AudioCommand::PlaySound {
                    name: "Impact".into(),
                    target: Some(target),
                    volume: 100,
                    looped: false,
                    multiple: false,
                    custom_falloff: None,
                },
                AudioCommand::DetachObjectSounds {
                    target,
                    position: Vector2::ZERO,
                },
            ]
        );
    }

    #[test]
    fn tutorial_music_hosts_emit_play_level_and_stop_like_cpp() {
        // FnMusic plays/stops immediately; FnMusicLevel clamps to 0..100 and
        // returns the stored level (C4Script.cpp:2329-2346; C4Game.cpp:
        // 4385-4389). Excess parameters are ignored after the parser warning
        // (C4AulParse.cpp:2339-2344), and bool arguments use C4Value truthiness.
        let (result, outcome) = with_object_host_context(|| {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    r#"
                    #strict
                    func Probe() {
                        Music("Frontend", "loop", "ignored");
                        var level = MusicLevel(130);
                        Music(0);
                        return level;
                    }
                    "#,
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(result.expect("tutorial music hosts run"), Value::Int(100));
        assert_eq!(outcome.audio.state.music_level(), 100);
        assert_eq!(
            outcome.audio.events,
            vec![
                AudioCommand::PlayMusic {
                    name: "Frontend".to_string(),
                    looped: true,
                },
                AudioCommand::SetMusicLevel { level: 100 },
                AudioCommand::StopMusic,
            ]
        );
    }

    #[test]
    fn set_playlist_filters_restarts_and_hides_only_the_sync_count_like_cpp() {
        // C4MusicSystem disables all records, then enables the union of raw
        // semicolon-separated, case-insensitive wildcard matches. An entry
        // matched by both patterns counts once, while duplicate records count
        // independently (C4MusicSystem.cpp:181-228). FnSetPlayList performs
        // those side effects before suppressing its local count in SyncMode.
        for sync in [false, true] {
            let mut audio = AudioRegistry::new();
            audio.set_available_music([
                "Pack/Theme.mid",
                "theme-extra.ogg",
                "Other.mid",
                "Duplicate/Theme.mid",
                "Credits.ogg",
            ]);
            let audio_guard = enter_audio_context(audio);
            let world = HostWorldContext::default().with_control_sync_mode(sync);
            let (result, outcome) = with_object_host_context_with_world(world, || {
                let mut script = clonk_script::Engine::new();
                register_host_functions(&mut script);
                script
                    .load_script(
                        r#"
                        #strict 3
                        func Probe() {
                            return SetPlayList("*.mid;THEME*", true, "ignored");
                        }
                        "#,
                    )
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
                script
                    .call("Probe", &[])
                    .map_err(|error| RuntimeError::new(error.to_string()))
            });
            let retained = audio_guard.finish();

            assert_eq!(
                result.expect("SetPlayList probe runs"),
                if sync { Value::Nil } else { Value::Int(4) }
            );
            assert_eq!(retained.music_playlist(), Some("*.mid;THEME*"));
            assert_eq!(outcome.audio.state.music_playlist(), Some("*.mid;THEME*"));
            assert_eq!(
                outcome.audio.events,
                vec![AudioCommand::SetMusicPlaylist {
                    playlist: Some("*.mid;THEME*".to_string()),
                    restart: true,
                }]
            );
        }
    }

    #[test]
    fn set_playlist_count_preserves_legacy_music_name_bytes() {
        let first = clonk_script::c4_string_from_bytes(b"Tune\x80.ogg");
        let second = clonk_script::c4_string_from_bytes(b"Tune\x81.ogg");
        let mut audio = AudioRegistry::new();
        audio.set_available_music([first, second]);

        assert_eq!(
            audio.set_music_playlist(clonk_script::c4_string_from_bytes(b"Tune\x80.*"), false),
            1,
            "literal high bytes must not collide through a lossy filename"
        );
        assert_eq!(
            audio.set_music_playlist(clonk_script::c4_string_from_bytes(b"Tune?.ogg"), false),
            2,
            "one native '?' consumes one legacy filename byte"
        );
        assert_eq!(
            audio.set_music_playlist(
                clonk_script::c4_string_from_bytes(b"Tune\x80.*\0Tune\x81.*"),
                false,
            ),
            1,
            "playlist matching stops at the native C-string terminator"
        );
        assert_eq!(
            audio
                .music_playlist()
                .map(clonk_script::c4_string_bytes)
                .as_deref(),
            Some(b"Tune\x80.*".as_slice()),
            "the retained playlist cannot expose bytes hidden after the terminator"
        );
    }

    #[test]
    fn set_next_mission_returns_nil_like_cpp() {
        // FnSetNextMission is a void host function; a non-empty path records
        // the next scenario metadata (C4Script.cpp:6053-6081).
        let (result, _) = with_object_host_context(|| {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    r#"
                    #strict
                    func Probe() {
                        return SetNextMission("Tutorial.c4f\\Tutorial01.c4s", "Repeat", "Again");
                    }
                    "#,
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(result.expect("SetNextMission runs"), Value::Nil);
    }

    #[test]
    fn set_next_mission_preserves_order_defaults_and_explicit_empty_strings() {
        // FnSetNextMission distinguishes omitted string pointers from an
        // explicit empty C4String, and clears path/text without touching the
        // description (C4Script.cpp:6053-6081).
        let (result, outcome) = with_object_host_context(|| {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    r#"
                    #strict
                    func Probe() {
                        SetNextMission("First", "", "");
                        SetNextMission("Second");
                        SetNextMission(0);
                        return 1;
                    }
                    "#,
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(result.expect("ordered calls run"), Value::Int(1));
        assert_eq!(
            outcome.next_mission_commands,
            vec![
                NextMissionCommand::Set {
                    path: "First".to_string(),
                    text: String::new(),
                    description: String::new(),
                },
                NextMissionCommand::Set {
                    path: "Second".to_string(),
                    text: DEFAULT_NEXT_MISSION_TEXT.to_string(),
                    description: DEFAULT_NEXT_MISSION_DESCRIPTION.to_string(),
                },
                NextMissionCommand::Clear,
            ]
        );
    }

    #[test]
    fn set_restore_infos_stores_raw_mask_in_order_and_returns_nil() {
        let error = set_restore_infos(&[Value::String("not an int".into())])
            .expect_err("typed integer conversion precedes the write");
        assert!(error.message().contains("expected integer"));

        let script = r#"#strict
public func StoreMask(value)
{
    return SetRestoreInfos(value, 123);
}

public func Sequence()
{
    return [
        SetRestoreInfos(RESTORE_ScriptPlayers | RESTORE_PlayerTeams),
        SetRestoreInfos(-1)
    ];
}

public func UseDefault()
{
    return SetRestoreInfos();
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(caller_index, "StoreMask", vec![Value::Bool(true)])
                .expect("bool mask call runs"),
            Value::Nil
        );
        assert_eq!(engine.restart_restore_info_mask(), 1);

        let unknown_bits = 0x4000_0003;
        assert_eq!(
            engine
                .call_object_function(caller_index, "StoreMask", vec![Value::Int(unknown_bits)],)
                .expect("unknown mask bits are retained"),
            Value::Nil
        );
        assert_eq!(engine.restart_restore_info_mask(), unknown_bits);

        assert_eq!(
            engine
                .call_object_function(caller_index, "Sequence", Vec::new())
                .expect("ordered mask writes run"),
            Value::Array(vec![Value::Nil, Value::Nil])
        );
        assert_eq!(
            engine.restart_restore_info_mask(),
            -1,
            "the raw signed final write wins without enum validation"
        );

        assert_eq!(
            engine
                .call_object_function(caller_index, "UseDefault", Vec::new())
                .expect("default mask call runs"),
            Value::Nil
        );
        assert_eq!(
            engine.restart_restore_info_mask(),
            0,
            "an omitted typed integer is nil-filled with zero"
        );

        let captured = engine.capture_state();
        engine
            .call_object_function(caller_index, "StoreMask", vec![Value::Int(-1)])
            .expect("post-save runtime mask write runs");
        engine
            .restore_state(&captured)
            .expect("in-place state restore succeeds");
        assert_eq!(
            engine.restart_restore_info_mask(),
            -1,
            "C++ save restoration does not overwrite the live restart handoff"
        );

        let mut fresh = crate::Engine::with_seed(1);
        fresh
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script)
                    .expect("fresh caller compiles"),
            )
            .expect("fresh caller registers");
        fresh
            .restore_state(&captured)
            .expect("captured state restores into a fresh engine");
        assert_eq!(
            fresh.restart_restore_info_mask(),
            0,
            "the runtime-only mask is absent from save and snapshot state"
        );
    }

    #[test]
    fn gain_mission_access_persists_and_goal_flow_reaches_game_over_return() {
        let mut engine = crate::Engine::with_seed(7);
        let definition = crate::Definition::from_script(
            "GOAL",
            "Goal",
            r#"#strict 2
public func Grant(password) { return GainMissionAccess(password); }
public func HasAccess(password) { return GetMissionAccess(password); }
public func CheckGoals()
{
    var passwords = ["goal-pass"];
    for (var missionPassword in passwords)
    {
        if (!missionPassword) return 1;
        GainMissionAccess(missionPassword);
    }
    return 0;
}
"#,
        )
        .expect("goal-style script compiles");
        engine
            .register_definition(definition)
            .expect("goal definition registers");
        let goal = engine
            .spawn_object(SpawnConfig::new("GOAL"))
            .expect("goal spawns");
        let goal_index = engine.find_object_index(goal).expect("goal exists");

        assert_eq!(
            engine
                .call_object_function(
                    goal_index,
                    "HasAccess",
                    vec![Value::String(String::new().into())],
                )
                .expect("explicit empty query executes"),
            Value::Bool(true),
            "SGetModule exposes the initially empty module"
        );
        assert_eq!(
            engine
                .call_object_function(goal_index, "Grant", vec![Value::String("pw".into())],)
                .expect("GainMissionAccess executes"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(goal_index, "Grant", vec![Value::String("PW".into())],)
                .expect("duplicate grant remains successful"),
            Value::Bool(true),
            "SAddModule de-duplicates case-insensitively"
        );
        assert_eq!(
            engine
                .call_object_function(
                    goal_index,
                    "HasAccess",
                    vec![Value::String(String::new().into())],
                )
                .expect("post-grant empty query executes"),
            Value::Bool(false)
        );
        assert_eq!(
            engine
                .call_object_function(goal_index, "CheckGoals", Vec::new())
                .expect("fulfilled Goal.c4d-style loop continues"),
            Value::Nil,
            "the caller reaches its game-over return"
        );
        for password in ["pw", "PW", "goal-pass", "GOAL-PASS"] {
            assert_eq!(
                engine
                    .call_object_function(
                        goal_index,
                        "HasAccess",
                        vec![Value::String(password.into())],
                    )
                    .expect("GetMissionAccess executes"),
                Value::Bool(true),
                "granted password {password:?} remains queryable"
            );
        }

        // Existing list is "pw;goal-pass" (12 bytes). The C++ guard allows
        // exactly 1009 more password bytes because 12 + 1009 + 3 == 1024;
        // after SAddModule adds the separator, any further non-empty grant
        // exceeds CFG_MaxString and returns false.
        let boundary_password = "x".repeat(1009);
        assert_eq!(
            engine
                .call_object_function(
                    goal_index,
                    "Grant",
                    vec![Value::String(boundary_password.clone().into())],
                )
                .expect("boundary grant executes"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(
                    goal_index,
                    "HasAccess",
                    vec![Value::String(boundary_password.into())],
                )
                .expect("boundary grant remains queryable"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(goal_index, "Grant", vec![Value::String("z".into())],)
                .expect("oversized grant reports failure without aborting"),
            Value::Bool(false)
        );
    }

    #[test]
    fn mission_access_uses_native_bytes_for_equality_and_length() {
        assert!(c4_bytes_equal_no_case(&[0xe4], &[0xc4]));
        assert_eq!(
            clonk_script::c4_string_bytes(&normalize_sound_name(
                &clonk_script::c4_string_from_bytes(&[0xe4]),
            )),
            [0xc4]
        );
        assert_eq!(
            clonk_script::c4_string_bytes(&make_valid_crew_name(
                &clonk_script::c4_string_from_bytes(&[0xe4]),
                &[clonk_script::c4_string_from_bytes(&[0xc4])],
            )),
            [0xe4, b'2']
        );

        let access = Rc::new(RefCell::new(String::new()));
        let world = HostWorldContext::default().with_mission_access(Rc::clone(&access));
        let split_utf8 = format!(
            "{}{}",
            clonk_script::c4_string_from_bytes(&[0xc3]),
            clonk_script::c4_string_from_bytes(&[0xbf])
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                gain_mission_access(&[Value::String(split_utf8.into())])?,
                Value::Bool(true)
            );
            get_mission_access(&[Value::String("\u{ff}".into())])
        });
        assert_eq!(
            result.expect("mission access calls succeed"),
            Value::Bool(true)
        );
        assert_eq!(
            clonk_script::c4_string_bytes(&access.borrow()),
            "\u{ff}".as_bytes()
        );
    }

    #[test]
    fn get_mission_access_reads_config_modules_and_warns_only_for_sync_queries() {
        fn query(access: Rc<RefCell<String>>, sync: bool, args: &[Value]) -> Value {
            let world = HostWorldContext::default()
                .with_mission_access(access)
                .with_control_sync_mode(sync);
            with_effect_context(None, &[], world, 1, || get_mission_access(args))
                .0
                .expect("GetMissionAccess query succeeds")
        }

        let access = Rc::new(RefCell::new("Alpha; Beta ;Gamma".to_string()));
        for password in ["alpha", "BETA", "Gamma"] {
            assert_eq!(
                query(Rc::clone(&access), false, &[Value::String(password.into())]),
                Value::Bool(true)
            );
        }
        assert_eq!(
            query(
                Rc::clone(&access),
                false,
                &[Value::String("missing".into())]
            ),
            Value::Bool(false)
        );
        assert_eq!(
            query(Rc::clone(&access), false, &[Value::Nil]),
            Value::Bool(false)
        );

        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));
        subscriber::with_default(subscriber, || {
            assert_eq!(
                query(Rc::clone(&access), true, &[Value::String("Alpha".into())]),
                Value::Bool(true)
            );
            assert_eq!(
                query(Rc::clone(&access), true, &[Value::Nil]),
                Value::Bool(false)
            );
        });

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, Level::WARN);
        assert_eq!(records[0].target, "clonk-script");
        assert_eq!(
            records[0].message,
            "using GetMissionAccess may cause desyncs when playing records!"
        );
    }

    #[test]
    fn player_message_targets_valid_player() {
        let mut player = PlayerState::default();
        player.id = 1;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(1), Value::String("Hi there".into())];
        let (result, outcome) =
            with_object_host_context_with_world(world, || player_message(&args));
        assert_eq!(result.expect("PlayerMessage succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.kind, MessageKind::GlobalPlayer);
                assert_eq!(spec.player, Some(1));
                assert_eq!(spec.text, "Hi there");
            }
            MessageCommand::PendingSpeech(_) => panic!("plain PlayerMessage cannot defer speech"),
        }
    }

    #[test]
    fn player_message_preserves_missing_player_scope() {
        // FnPlayerMessage has no ValidPlr branch: GameMsgPlayer and
        // GameMsgObjectPlayer retain the raw id, so C4GameMessage::Draw's
        // player equality check renders raw id 42 to no normal viewport.
        let target = ObjectId::new(1);
        let (result, outcome) = with_object_host_context(|| {
            Ok::<_, RuntimeError>((
                player_message(&[Value::Int(42), Value::String("Global secret".into())])?,
                player_message(&[
                    Value::Int(42),
                    Value::String("Target secret".into()),
                    object_reference_value(target),
                ])?,
            ))
        });

        assert_eq!(
            result.expect("PlayerMessage succeeds for a missing player"),
            (Value::Bool(true), Value::Bool(true))
        );
        assert_eq!(outcome.messages.len(), 2);
        for (command, expected_kind, expected_target, expected_text) in [
            (
                &outcome.messages[0],
                MessageKind::GlobalPlayer,
                None,
                "Global secret",
            ),
            (
                &outcome.messages[1],
                MessageKind::TargetPlayer,
                Some(target),
                "Target secret",
            ),
        ] {
            let MessageCommand::Add(spec) = command else {
                panic!("plain PlayerMessage cannot defer speech");
            };
            assert_eq!(spec.kind, expected_kind);
            assert_eq!(spec.target, expected_target);
            assert_eq!(spec.player, Some(42));
            assert_eq!(spec.text, expected_text);
        }
    }

    #[test]
    fn add_message_sets_multiple_flag() {
        let args = [Value::String("Queued".into())];
        let (result, outcome) = with_object_host_context(|| add_message(&args));
        assert_eq!(result.expect("AddMessage succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.flags & FLAG_MULTIPLE, FLAG_MULTIPLE);
                assert_eq!(spec.text, "Queued");
            }
            MessageCommand::PendingSpeech(_) => panic!("AddMessage cannot defer speech"),
        }
    }

    #[test]
    fn plr_message_degrades_to_global_when_player_missing() {
        let args = [Value::String("Warning".into()), Value::Int(42)];
        let (result, outcome) = with_object_host_context(|| plr_message(&args));
        assert_eq!(result.expect("PlrMessage succeeds"), Value::Bool(true));
        assert_eq!(outcome.messages.len(), 1);
        match &outcome.messages[0] {
            MessageCommand::Add(spec) => {
                assert_eq!(spec.kind, MessageKind::Global);
                assert!(spec.player.is_none());
                assert_eq!(spec.text, "Warning");
            }
            MessageCommand::PendingSpeech(_) => panic!("plain PlrMessage cannot defer speech"),
        }
    }

    #[test]
    fn format_applies_legacy_placeholders() {
        let args = [
            Value::String("Crew %03d %i %s %v %%".into()),
            Value::Int(7),
            Value::String("CLNK".into()),
            Value::String("Ready".into()),
            Value::Int(5),
        ];
        let result = format_string(&args).expect("Format succeeds");
        assert_eq!(result, Value::String("Crew 007 CLNK Ready 5 %".into()));
    }

    #[test]
    fn format_v_observes_strict_nil_and_legacy_parameter_reuse() {
        assert_eq!(
            format_string(&[
                Value::String("%v %v".into()),
                Value::Bool(false),
                Value::Int(5),
            ])
            .expect("direct Format succeeds"),
            Value::String("0 0".into()),
            "without a script caller, CalledWithStrictNil is false"
        );

        for (strict, expected) in [
            (
                2,
                Value::Array(vec![
                    Value::String("0 0".into()),
                    Value::String("0".into()),
                    Value::String("0".into()),
                    Value::String("7".into()),
                ]),
            ),
            (
                3,
                Value::Array(vec![
                    Value::String("0 5".into()),
                    Value::String("false".into()),
                    Value::String("nil".into()),
                    Value::String("7".into()),
                ]),
            ),
        ] {
            let mut script = ScriptEngine::new();
            register_host_functions(&mut script);
            script
                .load_script(&format!(
                    "#strict {strict}\nfunc Probe() {{ var unset; return [Format(\"%v %v\", 0, 5), Format(\"%v\", false), Format(\"%v\", unset), Format(\"%5v\", 7)]; }}"
                ))
                .expect("Format strictness probe compiles");

            assert_eq!(
                script.call("Probe", &[]).expect("Format probe runs"),
                expected,
                "#strict {strict}"
            );
        }
    }

    #[test]
    fn format_v_renders_live_object_name_and_number() {
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script(
                    "ACTR",
                    "Actor",
                    "#strict 3\npublic func Render() { return Format(\"%v\", this()); }",
                )
                .expect("actor definition compiles"),
            )
            .expect("actor definition registers");
        let actor = engine
            .spawn_object(crate::SpawnConfig::new("ACTR"))
            .expect("actor spawns");
        let actor_index = engine.find_object_index(actor).expect("actor exists");

        assert_eq!(
            engine
                .call_object_function(actor_index, "Render", Vec::new())
                .expect("object Format probe runs"),
            Value::String(format!("Actor #{}", actor.as_u64()).into())
        );
    }

    #[test]
    fn format_width_precision_and_c4id_use_native_byte_counts() {
        let utf8 = Value::String("\u{ff}".into());
        let projected =
            Value::String(clonk_script::c4_string_from_bytes("\u{ff}".as_bytes()).into());
        let format = Value::String("%.1s|%3s|%c".into());
        let render = |value: Value| {
            let result = format_string(&[format.clone(), value.clone(), value, Value::Int(0xff)])
                .expect("byte-oriented Format succeeds");
            let Value::String(result) = result else {
                panic!("Format returns a string");
            };
            clonk_script::c4_string_bytes(&result)
        };

        let expected = vec![0xc3, b'|', b' ', 0xc3, 0xbf, b'|', 0xff];
        assert_eq!(render(utf8), expected);
        assert_eq!(render(projected), expected);

        let raw_id = i32::from_le_bytes([0xff, b'A', 0, 0]);
        let Value::String(rendered_id) =
            format_string(&[Value::String("%i".into()), Value::Int(raw_id)])
                .expect("raw C4ID formats")
        else {
            panic!("Format returns a string");
        };
        assert_eq!(clonk_script::c4_string_bytes(&rendered_id), [0xff, b'A']);

        let Value::String(nul_truncated) =
            format_string(&[Value::String("A%cB".into()), Value::Int(0)])
                .expect("embedded NUL formats")
        else {
            panic!("Format returns a string");
        };
        assert_eq!(clonk_script::c4_string_bytes(&nul_truncated), b"A");

        let raw = [0xff, b'A', b'B', b'C'];
        let id = c4_id(&[Value::String(
            clonk_script::c4_string_from_bytes(&raw).into(),
        )])
        .expect("raw C4ID converts");
        assert_eq!(
            cast_int(std::slice::from_ref(&id)).expect("raw C4ID payload casts"),
            Value::Int(-1),
            "C4Id casts signed char 0xff to unsigned long before OR-ing"
        );
        let Value::String(formatted) =
            format_string(&[Value::String("%i".into()), id]).expect("raw C4ID formats")
        else {
            panic!("Format returns a string");
        };
        assert_eq!(clonk_script::c4_string_bytes(&formatted), [0xff; 4]);
        assert_eq!(
            c4_id(&[Value::String("CLNKtail".into())]).expect("long C4ID converts"),
            Value::C4Id("CLNK".into())
        );
    }

    #[test]
    fn typed_c4id_storage_preserves_identity_at_definition_and_command_boundaries() {
        let numeric = "12345";
        let stored_numeric = clonk_script::c4_id_from_raw(12345);
        assert_eq!(
            definition_id_for_c4id(numeric),
            definition_id_for_c4id(&stored_numeric)
        );
        assert_eq!(definition_id_for_c4id("ROCKtail").as_deref(), Some("ROCK"));

        let packed_digits = clonk_script::c4_id_from_raw(u32::from_le_bytes(*b"1111") as usize);
        assert_ne!(
            definition_id_for_c4id("1111"),
            definition_id_for_c4id(&packed_digits),
            "numeric 1111 and the packed bytes '1111' are distinct C4ID payloads"
        );
        for zero in ["NONE", "0000", "00000"] {
            assert_eq!(definition_id_for_c4id(zero), None);
        }

        let request = parse_command_request(
            CommandId::Call,
            &[
                Value::String("Call".into()),
                Value::Nil,
                Value::C4Id(stored_numeric.clone()),
                Value::Nil,
                Value::Nil,
                Value::String(String::new().into()),
            ],
            CommandArgLayout::Set,
            "SetCommand",
        )
        .expect("typed command Tx parses");
        assert_eq!(request.tx, Some(12345));
        assert_eq!(
            request
                .tx_definition
                .as_deref()
                .map(clonk_script::c4_id_raw),
            Some(12345)
        );
    }

    #[test]
    fn add_menu_item_keeps_equal_looking_c4id_payloads_distinct() {
        let script = r#"#strict 2
public func Open()
{
    CreateMenu(0, this(), this(), 0, "IDs");
    AddMenuItem("packed", "Choose", CastC4ID(825307441), this());
    AddMenuItem("numeric", "Choose", C4Id("1111"), this());
    SelectMenuItem(1, this());
    return true;
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("ACTR", "Actor", script)
                    .expect("menu fixture compiles"),
            )
            .expect("menu fixture registers");
        let actor = engine
            .spawn_object(crate::SpawnConfig::new("ACTR"))
            .expect("menu fixture spawns");
        let actor_index = engine.find_object_index(actor).expect("actor exists");

        assert_eq!(
            engine
                .call_object_function(actor_index, "Open", Vec::new())
                .expect("menu fixture runs"),
            Value::Bool(true)
        );
        let menu = engine
            .debug_object_menu(actor.as_u64())
            .expect("actor remains")
            .expect("menu opens");
        let packed_raw = u32::from_le_bytes(*b"1111") as usize;
        assert_eq!(menu.selection, 1);
        assert_eq!(clonk_script::c4_id_raw(&menu.items[0].item_id), packed_raw);
        assert_eq!(clonk_script::c4_id_raw(&menu.items[1].item_id), 1111);
        assert_ne!(menu.items[0].item_id, menu.items[1].item_id);
        assert_eq!(clonk_script::c4_id_text(&menu.items[0].item_id), "1111");
        assert_eq!(clonk_script::c4_id_text(&menu.items[1].item_id), "1111");
        // Generated menu commands are the intentional C4IdText/source
        // boundary; only the stored menu IDs retain the payload distinction.
        assert_eq!(menu.items[0].command, menu.items[1].command);
    }

    #[test]
    fn byte_limited_names_and_string_bit_eval_decode_raw_projection() {
        let thirty_one = clonk_script::c4_string_from_bytes(&[0xff; 31]);
        let truncated = truncate_c4_max_name(&thirty_one);
        assert_eq!(clonk_script::c4_string_bytes(&truncated), [0xff; 30]);
        assert_eq!(
            string_bit_eval(&clonk_script::c4_string_from_bytes(&[0xff, b'_', 0x80])),
            0b101
        );
    }

    #[test]
    fn get_type_reports_basic_value_kinds() {
        assert_eq!(
            get_type(&[Value::Nil]).expect("GetType succeeds"),
            Value::Int(C4V_ANY)
        );
        assert_eq!(
            get_type(&[Value::Int(7)]).expect("GetType succeeds"),
            Value::Int(C4V_INT)
        );
        assert_eq!(
            get_type(&[Value::Int(0)]).expect("direct GetType succeeds"),
            Value::Int(C4V_INT),
            "without cthr->Caller, falsy values retain their concrete type"
        );
        assert_eq!(
            get_type(&[Value::Bool(true)]).expect("GetType succeeds"),
            Value::Int(C4V_BOOL)
        );
        assert_eq!(
            get_type(&[Value::Bool(false)]).expect("direct GetType succeeds"),
            Value::Int(C4V_BOOL),
            "without cthr->Caller, falsy values retain their concrete type"
        );
        assert_eq!(
            get_type(&[Value::String("Hi".into())]).expect("GetType succeeds"),
            Value::Int(C4V_STRING)
        );
        assert_eq!(
            get_type(&[Value::Array(vec![Value::Int(1)])]).expect("GetType succeeds"),
            Value::Int(C4V_ARRAY)
        );
        let mut map = ValueMap::new();
        map.insert("key".into(), Value::Int(1));
        assert_eq!(
            get_type(&[Value::Proplist(map.into_iter().collect())]).expect("GetType succeeds"),
            Value::Int(C4V_MAP)
        );
    }

    #[test]
    fn cast_any_reconstructs_add_menu_item_nil_parameters() {
        // AddMenuItem writes an untyped null as CastAny(0)
        // (C4Script.cpp:1513-1546); executing that generated command must
        // recover C4V_Any/null rather than fail on an unknown helper.
        assert_eq!(
            cast_any(&[Value::Int(0)]).expect("CastAny succeeds"),
            Value::Nil
        );
    }

    #[test]
    fn cast_builtins_retag_payloads_and_drive_construction_paths() {
        let mut engine = crate::Engine::with_seed(3);
        let builder = crate::Definition::from_script(
            "ACLD",
            "Builder",
            r#"#strict 2
public func CastValues()
{
    var unset;
    return [CastC4ID(1279546187), CastInt(KSDL), CastBool(0), CastBool(7), CastInt(true), CastInt(unset), CastC4ID(0), CastC4ID(CastInt(GetID()) + 201135119), CastBool(C4Id("4294967296")), CastInt(C4Id("4294967297")), CastC4ID(C4Id("4294967296")), CastInt(CastC4ID(65536))];
}
public func WideCastValues()
{
    var id = C4Id("4294967296"), boolean = CastBool(id);
    return [boolean && true, CastInt(boolean), Equal(boolean, id), CastC4ID(boolean)];
}
public func MakePacked() { return CreateContents(CastC4ID(1279546187)); }
public func MakeRacesOffset(object container)
{
    return CreateContents(CastC4ID(CastInt(GetID()) + 201135119), container);
}
public func ControlCommand(command, target, tx, ty, target2, data)
{
    if (command == "Construct")
        if (CastC4ID(data)->~RejectConstruction(tx - GetX(), ty - GetY(), this()))
            return true;
    return false;
}
"#,
        )
        .expect("builder script compiles");
        engine
            .register_definition(builder)
            .expect("builder registers");
        let target = crate::Definition::from_script(
            "KSDL",
            "Packed target",
            r#"#strict 2
public func RejectConstruction(x, y, builder)
{
    if (!builder) return false;
    return x == 5 && y == 7;
}
"#,
        )
        .expect("packed target script compiles");
        engine
            .register_definition(target)
            .expect("packed target registers");
        engine
            .register_definition(
                crate::Definition::from_script("PWIP", "Races offset target", "#strict 2")
                    .expect("offset target script compiles"),
            )
            .expect("offset target registers");

        let builder = engine
            .spawn_object(SpawnConfig::new("ACLD").with_position(Vector2::new(20, 30)))
            .expect("builder spawns");
        let builder_index = engine.find_object_index(builder).expect("builder exists");

        #[cfg(all(not(target_os = "windows"), target_pointer_width = "64"))]
        let wide_bool = Value::from_c4_bool_data_raw(1_usize << 32);
        #[cfg(not(all(not(target_os = "windows"), target_pointer_width = "64")))]
        let wide_bool = Value::Bool(false);
        #[cfg(all(not(target_os = "windows"), target_pointer_width = "64"))]
        let wide_id = Value::C4Id(clonk_script::c4_id_from_raw(1_usize << 32));
        #[cfg(not(all(not(target_os = "windows"), target_pointer_width = "64")))]
        let wide_id = Value::Nil;

        assert_eq!(
            engine
                .call_object_function(builder_index, "CastValues", Vec::new())
                .expect("cast builtins execute"),
            Value::Array(vec![
                Value::C4Id("KSDL".into()),
                Value::Int(1_279_546_187),
                Value::Bool(false),
                Value::RawBool(7),
                Value::Int(1),
                Value::Int(0),
                Value::Nil,
                Value::C4Id("PWIP".into()),
                wide_bool,
                Value::Int(1),
                wide_id.clone(),
                Value::Int(65_536),
            ])
        );

        #[cfg(all(not(target_os = "windows"), target_pointer_width = "64"))]
        assert_eq!(
            engine
                .call_object_function(builder_index, "WideCastValues", Vec::new())
                .expect("wide casts execute"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(0),
                Value::Bool(true),
                wide_id,
            ]),
            "truthiness reads all C4V_Data bits while CastInt reads the low word and retagging preserves the raw payload"
        );

        let created = engine
            .call_object_function(builder_index, "MakePacked", Vec::new())
            .expect("packed id reaches CreateContents");
        let created = object_id_from_value(&created).expect("CreateContents returns an object");
        assert_eq!(
            engine
                .object_snapshot(created)
                .expect("created object survives")
                .definition_id,
            "KSDL"
        );

        let offset = engine
            .call_object_function(
                builder_index,
                "MakeRacesOffset",
                vec![object_reference_value(created)],
            )
            .expect("MonsterRescue-style packed id reaches CreateContents");
        let offset = object_id_from_value(&offset).expect("CreateContents returns an object");
        let offset = engine
            .object_snapshot(offset)
            .expect("offset object survives");
        assert_eq!(offset.definition_id, "PWIP");
        assert_eq!(offset.container, Some(created));

        let rejected = engine
            .call_object_function(
                builder_index,
                "ControlCommand",
                vec![
                    Value::String("Construct".into()),
                    Value::Nil,
                    Value::Int(25),
                    Value::Int(37),
                    Value::Nil,
                    Value::Int(1_279_546_187),
                ],
            )
            .expect("construction command executes without a script error");
        assert!(
            rejected.as_bool(),
            "the packed definition receives RejectConstruction"
        );
    }

    #[test]
    fn create_array_allocates_nil_initialised_values() {
        assert_eq!(
            create_array(&[]).expect("bare CreateArray succeeds"),
            Value::Array(Vec::new())
        );
        let result = create_array(&[Value::Int(3)]).expect("CreateArray succeeds");
        assert_eq!(
            result,
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil])
        );
    }

    #[test]
    fn create_array_rejects_out_of_range_sizes() {
        let error = create_array(&[Value::Int(-1)]).expect_err("CreateArray rejects negative");
        assert!(error
            .message()
            .starts_with("CreateArray: invalid array size"));

        let error = create_array(&[Value::Int(LEGACY_MAX_ARRAY_SIZE + 1)])
            .expect_err("CreateArray rejects oversized");
        assert!(error
            .message()
            .starts_with("CreateArray: invalid array size"));
    }

    #[test]
    fn set_length_resizes_the_callers_array_reference() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict
                func Grow() {
                    var values = CreateArray(2);
                    values[0] = 10;
                    values[1] = 20;
                    var result = SetLength(values, 5);
                    return [GetLength(values), values[0], values[1], values[2], values[4], result];
                }
                func Truncate() {
                    var values = [10, 20, 30];
                    SetLength(values, 1);
                    return [GetLength(values), values[0], values[1]];
                }
                func Negative() {
                    var values = [];
                    SetLength(values, -1);
                }
                func TooLarge() {
                    var values = [];
                    SetLength(values, 1000001);
                }
                func Nested() {
                    var outer = [[1, 2, 3]];
                    SetLength(outer[0], 1);
                    return [GetLength(outer[0]), outer[0][0], outer[0][1]];
                }
                func Alias() {
                    var values = [4, 5];
                    var alias = values;
                    SetLength(values, 1);
                    return [GetLength(values), GetLength(alias), alias[1]];
                }
                func RValue() { SetLength([1, 2], 1); }
                "#,
            )
            .expect("SetLength fixture compiles");

        assert_eq!(
            script.call("Grow", &[]).expect("array grows"),
            Value::Array(vec![
                Value::Int(5),
                Value::Int(10),
                Value::Int(20),
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );
        assert_eq!(
            script.call("Truncate", &[]).expect("array truncates"),
            Value::Array(vec![Value::Int(1), Value::Int(10), Value::Nil])
        );
        let error = script
            .call("Negative", &[])
            .expect_err("negative array lengths must fail");
        assert_eq!(
            error.to_string(),
            "runtime error: SetLength: invalid array size (-1)"
        );
        let error = script
            .call("TooLarge", &[])
            .expect_err("oversized arrays must fail");
        assert_eq!(
            error.to_string(),
            "runtime error: SetLength: invalid array size (1000001)"
        );
        assert_eq!(
            script.call("Nested", &[]).expect("indexed array resizes"),
            Value::Array(vec![Value::Int(1), Value::Int(1), Value::Nil])
        );
        assert_eq!(
            script
                .call("Alias", &[])
                .expect("array copy-on-write resizes"),
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(5)])
        );
        let error = script
            .call("RValue", &[])
            .expect_err("SetLength requires an array lvalue");
        assert_eq!(
            error.to_string(),
            "runtime error: call to \"SetLength\" parameter 1: got \"array\", but expected \"&\"!"
        );
    }

    #[test]
    fn get_length_returns_lengths_for_supported_types() {
        let result = get_length(&[Value::String("abc".into())]).expect("GetLength succeeds");
        assert_eq!(result, Value::Int(3));

        let result =
            get_length(&[Value::Array(vec![Value::Int(1), Value::Int(2)])]).expect("array length");
        assert_eq!(result, Value::Int(2));

        let mut map = ValueMap::new();
        map.insert("a".into(), Value::Int(1));
        map.insert("b".into(), Value::Bool(true));
        let result = get_length(&[Value::Proplist(map.into_iter().collect())]).expect("map length");
        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn get_length_returns_nil_for_falsey_values() {
        assert_eq!(get_length(&[Value::Nil]).expect("nil handled"), Value::Nil);
        assert_eq!(
            get_length(&[Value::Bool(false)]).expect("false handled"),
            Value::Nil
        );
        assert_eq!(
            get_length(&[Value::Int(0)]).expect("zero handled"),
            Value::Nil
        );
    }

    #[test]
    fn get_length_errors_for_unsupported_types() {
        let error = get_length(&[Value::Int(5)]).expect_err("GetLength rejects unsupported");
        assert_eq!(
            error.message(),
            "func \"GetLength\" par 0 cannot be converted to string or array or map"
        );
    }

    #[test]
    fn get_char_indexes_native_bytes_and_keeps_cpp_bounds() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict
                func Probe() {
                    var unset;
                    return [
                        GetChar("äb", 0), GetChar("äb", 1),
                        GetChar("abc", -1), GetChar("abc"), GetChar("abc", true),
                        GetChar("abc", 2), GetChar("abc", 3), GetChar("", -1),
                        GetChar(unset, 0)
                    ];
                }
                "#,
            )
            .expect("GetChar fixture compiles");

        assert_eq!(
            script.call("Probe", &[]).expect("GetChar probe executes"),
            Value::Array(vec![
                Value::Int(195),
                Value::Int(164),
                Value::Int(97),
                Value::Int(97),
                Value::Int(98),
                Value::Int(99),
                Value::Int(0),
                Value::Int(0),
                Value::Nil,
            ])
        );
    }

    #[test]
    fn get_index_of_strict2_and_older_use_cpp_scalar_equality() {
        let mut nonstrict = ScriptEngine::new();
        register_host_functions(&mut nonstrict);
        nonstrict
            .load_script("func Probe(needle, values) { return GetIndexOf(needle, values); }")
            .expect("NONSTRICT scalar probe loads without array syntax");
        for (needle, values) in [
            (Value::Nil, vec![Value::Int(0)]),
            (Value::Bool(false), vec![Value::Int(0)]),
            (Value::Int(0), vec![Value::Bool(false)]),
            (Value::Bool(true), vec![Value::Int(1)]),
            (
                Value::C4Id("ROCK".into()),
                vec![Value::Int(i32::from_le_bytes(*b"ROCK"))],
            ),
        ] {
            assert_eq!(
                nonstrict
                    .call("Probe", &[needle, Value::Array(values)])
                    .expect("NONSTRICT scalar equality probe runs"),
                Value::Int(0)
            );
        }

        for directive in ["#strict\n", "#strict 2\n"] {
            let bool_id_index = if directive == "#strict 2\n" { -1 } else { 0 };
            let wide_id_mismatch = if cfg!(target_pointer_width = "64") {
                -1
            } else {
                0
            };
            let mut script = ScriptEngine::new();
            register_host_functions(&mut script);
            script
                .load_script(&format!(
                    r#"{directive}
                    func Probe() {{
                        var unset;
                        var id = C4Id("ROCK");
                        var packed = CastInt(id);
                        var wide_id = C4Id("4294967297");
                        return [
                            GetIndexOf(0, CreateArray(3)),
                            GetIndexOf(unset, [0]),
                            GetIndexOf(false, [0]),
                            GetIndexOf(0, [false]),
                            GetIndexOf(true, [1]),
                            GetIndexOf(1, [true]),
                            GetIndexOf(packed, [id]),
                            GetIndexOf(id, [packed]),
                            GetIndexOf(true, [C4Id("0001")]),
                            GetIndexOf(C4Id("0001"), [true]),
                            GetIndexOf(2, [true]),
                            GetIndexOf(wide_id, [C4Id("0001")]),
                            GetIndexOf(1, [wide_id]),
                            GetIndexOf(wide_id, [wide_id])
                        ];
                    }}
                    "#
                ))
                .expect("GetIndexOf scalar probe loads");

            assert_eq!(
                script.call("Probe", &[]).expect("scalar probe runs"),
                Value::Array(vec![
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(bool_id_index),
                    Value::Int(bool_id_index),
                    Value::Int(-1),
                    Value::Int(wide_id_mismatch),
                    Value::Int(wide_id_mismatch),
                    Value::Int(0),
                ]),
                "directive {directive:?}"
            );
        }
    }

    #[test]
    fn get_index_of_strict3_checks_outer_type_only() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script.register_host_function("RetainedZeroIdArray", |_| {
            Ok(Value::Array(vec![Value::C4Id("NONE".into())]))
        });
        script
            .load_script(
                r#"
                #strict 3
                func Probe() {
                    var id = C4Id("ROCK");
                    var packed = CastInt(id);
                    return [
                        GetIndexOf(0, [nil]),
                        GetIndexOf(nil, [0]),
                        GetIndexOf(1, [true]),
                        GetIndexOf(true, [1]),
                        GetIndexOf(packed, [id]),
                        GetIndexOf(id, [packed]),
                        GetIndexOf(nil, [nil]),
                        GetIndexOf(false, [false]),
                        GetIndexOf(0, [0]),
                        GetIndexOf(id, [id]),
                        GetIndexOf([1], [[true]]),
                        GetIndexOf(C4Id("4294967297"), [C4Id("0001")]),
                        GetIndexOf(C4Id("4294967297"), [C4Id("4294967297")]),
                        GetIndexOf(C4Id("NONE"), [nil])
                    ];
                }
                func ManualZeroId(value) { return GetIndexOf(value, [nil]); }
                func ManualZeroEntry(value) { return GetIndexOf(false, [value]); }
                func RetainedZeroIdEntry() {
                    return GetIndexOf(nil, RetainedZeroIdArray());
                }
                "#,
            )
            .expect("strict3 GetIndexOf probe loads");

        assert_eq!(
            script.call("Probe", &[]).expect("strict3 probe runs"),
            Value::Array(vec![
                Value::Int(-1),
                Value::Int(-1),
                Value::Int(-1),
                Value::Int(-1),
                Value::Int(-1),
                Value::Int(-1),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(if cfg!(target_pointer_width = "64") {
                    -1
                } else {
                    0
                }),
                Value::Int(0),
                Value::Int(0),
            ])
        );
        assert_eq!(
            script
                .call("ManualZeroId", &[Value::C4Id("NONE".into())])
                .expect("external scalar copy canonicalizes zero-payload ID"),
            Value::Int(0)
        );
        assert_eq!(
            script
                .call("ManualZeroEntry", &[Value::C4Id("NONE".into())])
                .expect("strict3 keeps false distinct from canonical nil"),
            Value::Int(-1)
        );
        assert_eq!(
            script
                .call("RetainedZeroIdEntry", &[])
                .expect("container entry retains its zero C4ID tag"),
            Value::Int(-1)
        );
    }

    #[test]
    fn get_index_of_preserves_object_string_and_array_matching() {
        for level in [2, 3] {
            let mut script = ScriptEngine::new();
            register_host_functions(&mut script);
            script
                .load_script(&format!(
                    r#"
                    #strict {level}
                    func Probe(object first, object other) {{
                        var dynamic = "tar" .. "get";
                        return [
                            GetIndexOf(first, [other, first]),
                            GetIndexOf(first, [other]),
                            GetIndexOf(dynamic, ["target"]),
                            GetIndexOf(dynamic, ["other"]),
                            GetIndexOf([1, "x"], [[0], [1, "x"]]),
                            GetIndexOf([1, "x"], [[1, "y"]])
                        ];
                    }}
                    "#
                ))
                .expect("content matching probe loads");

            assert_eq!(
                script
                    .call("Probe", &[Value::Object(41), Value::Object(42)])
                    .expect("content matching probe runs"),
                Value::Array(vec![
                    Value::Int(1),
                    Value::Int(-1),
                    Value::Int(0),
                    Value::Int(-1),
                    Value::Int(1),
                    Value::Int(-1),
                ]),
                "strict level {level}"
            );
        }
    }

    #[test]
    fn get_index_of_pre_strict2_uses_backing_identity() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict
                func Probe() {
                    var array = [1], array_alias = array, other_array = [1];
                    var string = "x" .. "y", string_alias = string, other_string = "x" .. "y";
                    return [
                        GetIndexOf(array, [array_alias]),
                        GetIndexOf(array, [other_array]),
                        GetIndexOf(string, [string_alias]),
                        GetIndexOf(string, [other_string])
                    ];
                }
                "#,
            )
            .expect("identity probe loads");

        assert_eq!(
            script.call("Probe", &[]).expect("identity probe runs"),
            Value::Array(vec![
                Value::Int(0),
                Value::Int(-1),
                Value::Int(0),
                Value::Int(-1),
            ])
        );
    }

    #[test]
    fn get_index_of_uses_calling_functions_origin_strictness() {
        let mut strict3_source = ScriptEngine::new();
        strict3_source
            .load_script("#strict 3\nfunc Probe() { return GetIndexOf(0, [nil]); }")
            .expect("strict3 source loads");

        let mut strict2_destination = ScriptEngine::new();
        register_host_functions(&mut strict2_destination);
        strict2_destination
            .load_script("#strict 2\nfunc Own() { return 1; }")
            .expect("strict2 destination loads");
        strict2_destination.merge_from(&strict3_source);
        assert_eq!(
            strict2_destination
                .call("Probe", &[])
                .expect("included strict3 function runs"),
            Value::Int(-1)
        );

        let mut strict2_source = ScriptEngine::new();
        strict2_source
            .load_script("#strict 2\nfunc Probe() { var unset; return GetIndexOf(0, [unset]); }")
            .expect("strict2 source loads");

        let mut strict3_destination = ScriptEngine::new();
        register_host_functions(&mut strict3_destination);
        strict3_destination
            .load_script("#strict 3\nfunc Own() { return 1; }")
            .expect("strict3 destination loads");
        strict3_destination.merge_from(&strict2_source);
        assert_eq!(
            strict3_destination
                .call("Probe", &[])
                .expect("included strict2 function runs"),
            Value::Int(0)
        );
    }

    #[test]
    fn get_index_of_keeps_cpp_array_parameter_conversion() {
        let mut strict2 = ScriptEngine::new();
        register_host_functions(&mut strict2);
        strict2
            .load_script(
                r#"
                #strict 2
                func Missing() { return GetIndexOf(1); }
                func Zero() { return GetIndexOf(1, 0); }
                func False() { return GetIndexOf(1, false); }
                func Wrong() { return GetIndexOf(1, true); }
                func Passed(value) { return GetIndexOf(1, value); }
                func Entry(value) { return GetIndexOf(false, [value]); }
                "#,
            )
            .expect("strict2 parameter probe loads");
        for function in ["Missing", "Zero", "False"] {
            assert_eq!(
                strict2.call(function, &[]).expect("nil array is accepted"),
                Value::Int(-1)
            );
        }
        assert!(strict2
            .call("Wrong", &[])
            .expect_err("truthy non-array is rejected")
            .to_string()
            .contains("expected \"array\""));
        assert_eq!(
            strict2
                .call("Passed", &[Value::C4Id("NONE".into())])
                .expect("zero-payload ID array is nil"),
            Value::Int(-1)
        );
        for value in [Value::C4Id("NONE".into()), Value::Object(0)] {
            assert_eq!(
                strict2
                    .call("Entry", &[value])
                    .expect("zero-payload entry is canonical nil"),
                Value::Int(0)
            );
        }

        let mut strict3 = ScriptEngine::new();
        register_host_functions(&mut strict3);
        strict3
            .load_script(
                "#strict 3\nfunc Zero() { return GetIndexOf(1, 0); }\n\
                 func Passed(value) { return GetIndexOf(1, value); }",
            )
            .expect("strict3 parameter probe loads");
        assert!(strict3
            .call("Zero", &[])
            .expect_err("strict3 retains typed zero")
            .to_string()
            .contains("expected \"array\""));
        assert_eq!(
            strict3
                .call("Passed", &[Value::C4Id("0000".into())])
                .expect("engine-entry Set copy canonicalizes the zero ID"),
            Value::Int(-1)
        );
    }

    #[test]
    fn log_message_emits_info_event_with_script_target() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let layer = RecordingLayer::new(Arc::clone(&records));
        let subscriber = Registry::default().with(layer);
        subscriber::with_default(subscriber, || {
            let args = [Value::String("Log %02d".into()), Value::Int(3)];
            let result = log_message(&args).expect("Log succeeds");
            assert_eq!(result, Value::Nil);
        });
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.level, Level::INFO);
        assert_eq!(record.target, "clonk-script");
        assert_eq!(record.message, "Log 03");
    }

    #[test]
    fn debug_log_message_emits_debug_event_with_script_target() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let layer = RecordingLayer::new(Arc::clone(&records));
        let subscriber = Registry::default().with(layer);
        subscriber::with_default(subscriber, || {
            let args = [Value::String("Debug %d".into()), Value::Int(42)];
            let result = debug_log_message(&args).expect("DebugLog succeeds");
            assert_eq!(result, Value::Nil);
        });
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.level, Level::DEBUG);
        assert_eq!(record.target, "clonk-script");
        assert_eq!(record.message, "Debug 42");
    }

    #[test]
    fn log_returns_nil_and_takes_the_false_branch() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict 2
                func Probe() {
                    var result = Log("first");
                    if (Log("second")) return 42;
                    return [result, 7];
                }
                "#,
            )
            .expect("Log return probe compiles");

        assert_eq!(
            script.call("Probe", &[]).expect("Log return probe runs"),
            Value::Array(vec![Value::Nil, Value::Int(7)])
        );
    }

    #[test]
    fn start_call_trace_is_registered_and_returns_nil() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script("#strict 3\nfunc Probe() { return StartCallTrace(); }")
            .expect("StartCallTrace probe compiles");

        assert_eq!(
            script.call("Probe", &[]).expect("StartCallTrace executes"),
            Value::Nil
        );
    }

    #[test]
    fn debug_builtin_call_trace_follows_nested_calls_until_arming_frame_returns() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Arm() { StartCallTrace(); return Outer(); }\n\
                 func Outer() { return Inner(); }\n\
                 func Inner() { return 7; }\n\
                 func Untraced() { return 8; }",
            )
            .expect("call-trace probes compile");
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));

        subscriber::with_default(subscriber, || {
            assert_eq!(
                script.call("Arm", &[]).expect("call trace executes"),
                Value::Int(7)
            );
            assert_eq!(
                script
                    .call("Untraced", &[])
                    .expect("later top-level call executes"),
                Value::Int(8)
            );
        });

        let records = records.lock().unwrap();
        assert!(records.iter().all(|record| record.level == Level::INFO));
        assert!(records
            .iter()
            .all(|record| record.target == "clonk-script-trace"));
        assert_eq!(
            records
                .iter()
                .map(|record| record.message.as_str())
                .collect::<Vec<_>>(),
            vec![
                "T>Outer()",
                "T>>Inner()",
                "T>>Inner returned 7",
                "T>Outer returned 7",
                "TArm returned 7",
            ]
        );
    }

    #[test]
    fn direct_exec_arms_trace_and_reports_direct_exec_profile_entry() {
        let mut script = ScriptEngine::new();
        script.set_script_name("Test.Script");
        register_host_functions(&mut script);
        script.register_host_function("ProfilerDelay", |_| {
            std::thread::sleep(std::time::Duration::from_millis(3));
            Ok(Value::Nil)
        });
        script
            .load_script(
                "#strict 2\n\
                 func Helper() { return 7; }\n\
                 func EvalProfileFail() { return eval(\"ProfilerDelay() || Missing()\"); }\n\
                 func EndProfile() { return StopScriptProfiler(); }",
            )
            .expect("DirectExec diagnostic probes compile");

        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));
        subscriber::with_default(subscriber, || {
            let (value, _) = script
                .direct_exec_with_locals_and_this_in_context(
                    "StartCallTrace() || Helper()",
                    &HashMap::new(),
                    Value::Nil,
                    "console script",
                )
                .expect("traced DirectExec succeeds");
            assert_eq!(value, Value::Int(7));
            assert_eq!(
                script.call("Helper", &[]).expect("later call succeeds"),
                Value::Int(7),
                "the DirectExec frame unwind ends its trace"
            );

            clonk_script::start_script_profiler(None);
            for context in ["console script", "MenuCommand"] {
                let (value, _) = script
                    .direct_exec_with_locals_and_this_in_context(
                        "ProfilerDelay() || 1",
                        &HashMap::new(),
                        Value::Nil,
                        context,
                    )
                    .expect("profiled DirectExec succeeds");
                assert_eq!(value, Value::Int(1));
            }
            assert!(
                script
                    .direct_exec_with_locals_and_this_in_context(
                        "ProfilerDelay() || Missing()",
                        &HashMap::new(),
                        Value::Nil,
                        "console script",
                    )
                    .is_err(),
                "the host catches this fPassErrors=false failure after DirectExec"
            );
            assert_eq!(
                script.call("EndProfile", &[]).expect("profiler stops"),
                Value::Nil
            );

            clonk_script::start_script_profiler(None);
            assert!(
                script.call("EvalProfileFail", &[]).is_err(),
                "eval propagates its runtime error"
            );
            assert!(
                clonk_script::stop_script_profiler()
                    .expect("eval error leaves profiler active")
                    .iter()
                    .all(|entry| !entry.direct_exec),
                "fPassErrors=true eval never reaches StopDirectExec"
            );
        });

        let records = records.lock().unwrap();
        let trace = records
            .iter()
            .filter(|record| record.target == "clonk-script-trace")
            .map(|record| record.message.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            trace,
            vec!["T>Helper()", "T>Helper returned 7", "T returned 7"]
        );

        let profiler_rows = records
            .iter()
            .filter(|record| {
                record.target == "clonk-script-profiler"
                    && record.message.ends_with("\tDirect exec")
            })
            .collect::<Vec<_>>();
        assert_eq!(profiler_rows.len(), 1, "DirectExec timings aggregate");
        assert!(!profiler_rows[0].message.contains("global Direct exec"));
        let elapsed_ms = profiler_rows[0]
            .message
            .split_once("ms\t")
            .expect("native profiler row format")
            .0
            .parse::<u128>()
            .expect("native profiler duration");
        assert!(
            elapsed_ms >= 9,
            "success and swallowed-error DirectExec calls all contribute"
        );
    }

    #[test]
    fn eval_direct_exec_frames_use_cpp_names_and_balance_on_errors() {
        let mut script = ScriptEngine::new();
        script.set_script_name("Scenario");
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Helper() { return 7; }\n\
                 func Arm() { StartCallTrace(); return eval(\"Helper()\"); }\n\
                 func Fail() { StartCallTrace(); return eval(\"Missing()\"); }\n\
                 func NestedFail() { StartCallTrace(); return eval(\"eval(\\\"Missing()\\\")\"); }\n\
                 func ParseFail() { StartCallTrace(); return eval(\"(\"); }\n\
                 func GlobalEval() { StartCallTrace(); return global->eval(\"1\"); }",
            )
            .expect("eval diagnostic probes compile");

        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));
        subscriber::with_default(subscriber, || {
            let take_trace = || {
                let mut records = records.lock().unwrap();
                let trace = records
                    .iter()
                    .filter(|record| record.target == "clonk-script-trace")
                    .map(|record| record.message.clone())
                    .collect::<Vec<_>>();
                records.clear();
                trace
            };

            assert_eq!(
                script.call("Arm", &[]).expect("eval succeeds"),
                Value::Int(7)
            );
            assert_eq!(
                take_trace(),
                [
                    "T>eval in Scenario",
                    "T>>Helper()",
                    "T>>Helper returned 7",
                    "T> returned 7",
                    "TArm returned 7",
                ]
            );

            let error = script
                .call("Fail", &[])
                .expect_err("runtime errors propagate");
            assert_eq!(error.call_frames().len(), 2);
            assert_eq!(
                error.call_frames()[0].direct_exec_display(),
                Some("eval in Scenario")
            );
            assert_eq!(error.call_frames()[1].function(), "Fail");
            assert_eq!(error.call_frames()[1].direct_exec_display(), None);
            assert_eq!(take_trace(), ["T>eval in Scenario"]);

            let error = script
                .call("NestedFail", &[])
                .expect_err("nested runtime errors propagate");
            assert_eq!(error.call_frames().len(), 3);
            assert_eq!(
                error.call_frames()[0].direct_exec_display(),
                Some("eval in Scenario")
            );
            assert_eq!(
                error.call_frames()[1].direct_exec_display(),
                Some("eval in Scenario")
            );
            assert_eq!(error.call_frames()[2].function(), "NestedFail");
            assert_eq!(take_trace(), ["T>eval in Scenario", "T>>eval in Scenario"]);

            assert_eq!(
                script
                    .call("ParseFail", &[])
                    .expect("parse errors become nil"),
                Value::Nil
            );
            assert_eq!(take_trace(), ["TParseFail returned nil"]);

            assert_eq!(
                script
                    .call("GlobalEval", &[])
                    .expect("global eval succeeds"),
                Value::Int(1)
            );
            assert_eq!(
                take_trace(),
                [
                    "T>eval in Scenario",
                    "T> returned 1",
                    "TGlobalEval returned 1",
                ]
            );
        });
    }

    #[test]
    fn direct_exec_stack_display_and_targeted_profiler_match_cpp() {
        let mut script = ScriptEngine::new();
        script.set_script_name("TEST");
        script.set_game_script_name("Scenario.c4s/Script.c");
        script.set_definition_context(true);
        register_host_functions(&mut script);
        script.register_host_function("CaptureFrames", |_| {
            Ok(Value::Array(
                clonk_script::active_direct_exec_diagnostic_frames()
                    .into_iter()
                    .map(Value::from)
                    .collect(),
            ))
        });
        script.register_host_function("MissingObject", |_| Ok(Value::Object(999)));
        script.register_host_function("ProfilerDelay", |_| {
            std::thread::sleep(std::time::Duration::from_millis(3));
            Ok(Value::Int(1))
        });
        script.register_host_function("StopProfilerWithoutDirect", |_| {
            let entries = clonk_script::stop_script_profiler().unwrap_or_default();
            Ok(Value::Bool(entries.iter().all(|entry| !entry.direct_exec)))
        });
        let direct_trace = Arc::new(Mutex::new(Vec::new()));
        let direct_trace_sink = Arc::clone(&direct_trace);
        script.register_host_function("CaptureDirectTrace", move |_| {
            let direct_trace = Arc::clone(&direct_trace_sink);
            clonk_script::start_call_trace(move |message| {
                direct_trace.lock().unwrap().push(message.to_string());
            });
            Ok(Value::Nil)
        });
        script
            .load_script(
                "#strict 3\n\
                 global func GlobalFrame() { return eval(\"CaptureFrames()\"); }\n\
                 global func NestedDefinitionFrame() { return eval(\"CaptureFrames()\"); }\n\
                 func DefinitionFrame() { return eval(\"CaptureFrames()\"); }\n\
                 func DefinitionThroughEval() { return eval(\"NestedDefinitionFrame()\"); }\n\
                 func ProbeGlobal() { return global->GlobalFrame(); }",
            )
            .expect("stack-display host compiles");

        let (frames, _) = with_effect_context(
            Some(object_host_context_with_physical_energy(100, 100).with_definition_id("Clonk")),
            &[],
            HostWorldContext::default(),
            2,
            || {
                script.direct_exec_with_locals_and_this_in_context(
                    "eval(\"CaptureFrames()\")",
                    &HashMap::new(),
                    Value::Object(1),
                    "MenuCommand",
                )
            },
        )
        .0
        .expect("nested DirectExec stack capture succeeds");
        assert_eq!(
            frames,
            Value::Array(vec![
                Value::String("MenuCommand in TEST (obj Clonk #1)".into()),
                Value::String("eval in TEST (obj Clonk #1)".into()),
            ])
        );
        let renamed_error = with_effect_context(
            Some(object_host_context_with_physical_energy(100, 100).with_definition_id("Clonk")),
            &[],
            HostWorldContext::default(),
            2,
            || {
                script.direct_exec_with_locals_and_this_in_context(
                    "SetName(\"Renamed\") && Missing()",
                    &HashMap::new(),
                    Value::Object(1),
                    "MenuCommand",
                )
            },
        )
        .0
        .expect_err("renamed-object probe raises a runtime error");
        assert_eq!(
            renamed_error.call_frames()[0].direct_exec_display(),
            Some("MenuCommand in TEST (obj Renamed #1)"),
            "DirectExec stack dumps render the object's live error-time name"
        );
        let borrowed_context_error = with_effect_context(
            Some(object_host_context_with_physical_energy(100, 100).with_definition_id("Clonk")),
            &[],
            HostWorldContext::default(),
            2,
            || {
                script.direct_exec_with_locals_and_this_in_context(
                    "SetTransferZone(0, 0, 1, 1, MissingObject())",
                    &HashMap::new(),
                    Value::Object(1),
                    "MenuCommand",
                )
            },
        )
        .0
        .expect_err("missing transfer-zone owner raises a runtime error");
        assert_eq!(
            borrowed_context_error.call_frames()[0].direct_exec_display(),
            Some("MenuCommand in TEST (obj Clonk #1)"),
            "errors raised under a mutable host-context borrow retain the entry display"
        );
        let (returned_object, _) = with_effect_context(
            Some(object_host_context_with_physical_energy(100, 100).with_definition_id("Clonk")),
            &[],
            HostWorldContext::default(),
            2,
            || {
                script.direct_exec_with_locals_and_this_in_context(
                    "CaptureDirectTrace() || Object(1)",
                    &HashMap::new(),
                    Value::Object(1),
                    "MenuCommand",
                )
            },
        )
        .0
        .expect("object-return DirectExec succeeds");
        assert_eq!(returned_object, Value::Object(1));
        assert_eq!(
            *direct_trace.lock().unwrap(),
            ["T returned Clonk #1"],
            "anonymous DirectExec returns use C4Value::GetDataString"
        );
        assert_eq!(
            script
                .call("GlobalFrame", &[])
                .expect("callerless global frame eval succeeds"),
            Value::Array(vec![Value::String("eval in Scenario.c4s/Script.c".into())]),
            "a callerless global function is owned by Game.ScriptEngine"
        );
        assert_eq!(
            script
                .call("DefinitionFrame", &[])
                .expect("definition frame eval succeeds"),
            Value::Array(vec![Value::String("eval in TEST".into())]),
            "a retained Def context selects the definition script"
        );
        assert_eq!(
            script
                .call("DefinitionThroughEval", &[])
                .expect("nested definition eval succeeds"),
            Value::Array(vec![
                Value::String("eval in TEST".into()),
                Value::String("eval in Scenario.c4s/Script.c".into()),
            ]),
            "nil-object DirectExec clears Def for nested calls"
        );
        assert_eq!(
            script
                .call("ProbeGlobal", &[])
                .expect("global frame eval succeeds"),
            Value::Array(vec![Value::String("eval in Scenario.c4s/Script.c".into())]),
            "a null Obj/Def global frame selects Game.Script"
        );

        let unrelated_target = ScriptEngine::new().host_identity();
        clonk_script::start_script_profiler(Some(unrelated_target));
        script
            .direct_exec_with_locals_and_this_in_context(
                "ProfilerDelay()",
                &HashMap::new(),
                Value::Nil,
                "console script",
            )
            .expect("target-independent DirectExec profiling succeeds");
        let entries = clonk_script::stop_script_profiler().expect("profiler was active");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].direct_exec);
        assert_eq!(entries[0].function, "Direct exec");
        assert_eq!(entries[0].host_identity, None);
        assert!(entries[0].elapsed >= std::time::Duration::from_millis(3));

        script
            .direct_exec_with_locals_and_this_in_context(
                "(StartScriptProfiler() && ProfilerDelay()) || 1",
                &HashMap::new(),
                Value::Nil,
                "internal script",
            )
            .expect("profiling can start inside DirectExec");
        let entries = clonk_script::stop_script_profiler().expect("inner profiler remains active");
        let direct = entries
            .iter()
            .find(|entry| entry.direct_exec)
            .expect("active DirectExec starts timing at profiler start");
        assert!(direct.elapsed >= std::time::Duration::from_millis(3));

        clonk_script::start_script_profiler(None);
        assert_eq!(
            script
                .direct_exec_with_locals_and_this_in_context(
                    "ProfilerDelay() && StopProfilerWithoutDirect()",
                    &HashMap::new(),
                    Value::Nil,
                    "console script",
                )
                .expect("profiling can stop inside DirectExec")
                .0,
            Value::Bool(true),
            "stopping inside DirectExec excludes its still-active frame"
        );
        assert!(clonk_script::stop_script_profiler().is_none());

        clonk_script::start_script_profiler(None);
        script
            .direct_exec_with_locals_and_this_at_strict_in_context_diagnostics(
                "ProfilerDelay()",
                &HashMap::new(),
                Value::Nil,
                Some(3),
                "ObjectMenuValue",
                false,
            )
            .expect("Rust-only expression adapter executes");
        assert!(
            clonk_script::stop_script_profiler()
                .expect("adapter profiler was active")
                .is_empty(),
            "Rust-only expression adapters do not contaminate DirectExec totals"
        );
        assert!(crate::is_cpp_direct_exec_context("MenuCommand"));
        assert!(!crate::is_cpp_direct_exec_context("ObjectMenuValue"));

        let engine = crate::Engine::new();
        assert_eq!(
            engine.script_control_global_host().script_name(),
            "System.c4g"
        );
    }

    #[test]
    fn script_profiler_builtins_validate_definition_and_execute() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func StartAll() { return StartScriptProfiler(); }\n\
                 func StartKnown() { return StartScriptProfiler(GOOD); }\n\
                 func StartUnknown() { return StartScriptProfiler(MISS); }\n\
                 func Stop() { return StopScriptProfiler(); }",
            )
            .expect("script-profiler probes compile");
        let script = Arc::new(script);
        assert_eq!(
            script
                .call("StartAll", &[])
                .expect("global profiler starts"),
            Value::Bool(true)
        );
        assert_eq!(
            script
                .call("StartUnknown", &[])
                .expect("unknown definition is rejected without a world"),
            Value::Bool(false)
        );
        assert_eq!(
            script.call("Stop", &[]).expect("global profiler stops"),
            Value::Nil
        );

        let world = HostWorldContext::default()
            .with_definition_scripts(HashMap::from([("GOOD".into(), Arc::clone(&script))]));

        let (result, _) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                script
                    .call("StartAll", &[])
                    .expect("global profiler starts"),
                Value::Bool(true)
            );
            assert_eq!(
                script
                    .call("StartKnown", &[])
                    .expect("known definition profiler starts"),
                Value::Bool(true)
            );
            assert_eq!(
                script
                    .call("StartUnknown", &[])
                    .expect("unknown definition is rejected"),
                Value::Bool(false)
            );
            assert_eq!(
                script.call("Stop", &[]).expect("definition profiler stops"),
                Value::Nil
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("script-profiler builtins execute");
    }

