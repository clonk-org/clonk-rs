use super::*;

#[test]
fn scripted_music_level_roundtrips_engine_state_and_restore_event() {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.load_scenario_script_with_convention(
        "MusicLevel.c",
        r#"#strict 3
    func Probe() {
    var low = MusicLevel(-1);
    var high = MusicLevel(130);
    var saved = MusicLevel(25);
    return [low, high, saved];
    }
    "#,
        true,
    ));

    assert_eq!(
        engine
            .call_scenario_script_value("Probe", &[])
            .expect("MusicLevel probe executes"),
        Some(Value::Array(vec![
            Value::Int(0),
            Value::Int(100),
            Value::Int(25),
        ]))
    );
    assert_eq!(
        engine.pending_audio.last(),
        Some(&AudioCommand::SetMusicLevel { level: 25 })
    );

    let state = engine.capture_state();
    assert_eq!(state.music_level, 25);
    assert_eq!(
        InitialNetworkGameData::from_engine(&engine)
            .expect("music-level-only engine is representable in Game.txt")
            .music_level,
        25
    );
    let encoded = crate::TestValueExt::test_value(state.to_json_string());
    let decoded = crate::TestValueExt::test_value(EngineState::from_json_str(&encoded));
    let mut restored = Engine::new();
    crate::TestValueExt::test_value(restored.restore_state(&decoded));

    assert_eq!(restored.capture_state().music_level, 25);
    assert_eq!(
        restored.pending_audio.last(),
        Some(&AudioCommand::SetMusicLevel { level: 25 })
    );
}

#[test]
fn legacy_engine_state_without_music_level_defaults_to_100() {
    let mut value =
        crate::TestValueExt::test_value(serde_json::to_value(Engine::new().capture_state()));
    crate::TestValueExt::test_value(value.as_object_mut()).remove("music_level");
    let decoded: EngineState = crate::TestValueExt::test_value(serde_json::from_value(value));
    assert_eq!(decoded.music_level, DEFAULT_MUSIC_LEVEL);

    let mut restored = Engine::new();
    crate::TestValueExt::test_value(restored.restore_state(&decoded));
    assert_eq!(restored.capture_state().music_level, DEFAULT_MUSIC_LEVEL);
    assert_eq!(
        restored.pending_audio.last(),
        Some(&AudioCommand::SetMusicLevel {
            level: DEFAULT_MUSIC_LEVEL,
        })
    );
}

#[test]
fn resume_reconciliation_folds_music_commands_without_mutating_saved_fields() {
    let mut engine = Engine::new();
    engine
        .audio_registry
        .restore_music_playlist(Some("Theme*".to_string()));
    engine.audio_registry.restore_music_level(25);
    engine.pending_audio = vec![
        AudioCommand::PlayMusic {
            name: "Intro".to_string(),
            looped: false,
        },
        AudioCommand::StopMusic,
        AudioCommand::SetMusicPlaylist {
            playlist: Some("FinalInit*".to_string()),
            restart: true,
        },
        AudioCommand::SetMusicLevel { level: 25 },
    ];

    assert!(!engine.reconcile_music_after_restore(false));
    assert!(engine.pending_audio.is_empty());
    let state = engine.capture_state();
    assert_eq!(state.play_list.as_deref(), Some("Theme*"));
    assert_eq!(state.music_level, 25);

    engine.pending_audio = vec![
        AudioCommand::StopMusic,
        AudioCommand::PlayMusic {
            name: "Final".to_string(),
            looped: true,
        },
    ];
    assert!(engine.reconcile_music_after_restore(false));
    assert!(engine.pending_audio.is_empty());
}

#[test]
fn set_playlist_roundtrips_engine_state_and_initial_game_data() {
    let mut engine = Engine::new();
    engine.configure_music_tracks([
        "Pack/Theme.mid",
        "theme-extra.ogg",
        "Other.mid",
        "Duplicate/Theme.mid",
        "Credits.ogg",
    ]);
    crate::TestValueExt::test_value(engine.load_scenario_script_with_convention(
        "SetPlayList.c",
        "#strict 3\nfunc Probe() { return SetPlayList(\"*.mid;THEME*\", true); }\n",
        true,
    ));

    assert_eq!(
        engine
            .call_scenario_script_value("Probe", &[])
            .expect("SetPlayList probe executes"),
        Some(Value::Int(4))
    );
    assert_eq!(engine.music_playlist(), "*.mid;THEME*");
    assert_eq!(
        engine.pending_audio.last(),
        Some(&AudioCommand::SetMusicPlaylist {
            playlist: Some("*.mid;THEME*".to_owned()),
            restart: true,
        })
    );

    let state = engine.capture_state();
    assert_eq!(state.play_list.as_deref(), Some("*.mid;THEME*"));
    assert_eq!(
        InitialNetworkGameData::from_engine(&engine)
            .expect("playlist-only engine is representable in JoinData")
            .play_list,
        "*.mid;THEME*"
    );

    let encoded = crate::TestValueExt::test_value(state.to_json_string());
    let decoded = crate::TestValueExt::test_value(EngineState::from_json_str(&encoded));
    let mut restored = Engine::new();
    restored.configure_music_tracks(["Theme.mid", "Other.ogg"]);
    crate::TestValueExt::test_value(restored.restore_state(&decoded));
    assert_eq!(restored.music_playlist(), "*.mid;THEME*");
    assert_eq!(
        restored.pending_audio,
        vec![
            AudioCommand::SetMusicPlaylist {
                playlist: Some("*.mid;THEME*".to_owned()),
                restart: false,
            },
            AudioCommand::SetMusicLevel {
                level: DEFAULT_MUSIC_LEVEL,
            },
        ]
    );
    assert_eq!(
        InitialNetworkGameData::from_engine(&restored)
            .expect("restored playlist is representable in JoinData")
            .play_list,
        "*.mid;THEME*"
    );
}

#[test]
fn explicit_empty_playlist_remains_distinct_from_default_after_restore() {
    let mut engine = Engine::new();
    engine.configure_music_tracks(["Theme.mid"]);
    crate::TestValueExt::test_value(engine.load_scenario_script_with_convention(
        "EmptySetPlayList.c",
        "#strict 3\nfunc Probe() { return SetPlayList(nil, false); }\n",
        true,
    ));
    assert_eq!(
        engine
            .call_scenario_script_value("Probe", &[])
            .expect("empty SetPlayList probe executes"),
        Some(Value::Int(0))
    );
    assert_eq!(engine.capture_state().play_list, Some(String::new()));

    let mut restored = Engine::new();
    crate::TestValueExt::test_value(restored.restore_state(&engine.capture_state()));
    assert_eq!(restored.capture_state().play_list, Some(String::new()));
    assert_eq!(
        restored.pending_audio,
        vec![
            AudioCommand::SetMusicPlaylist {
                playlist: Some(String::new()),
                restart: false,
            },
            AudioCommand::SetMusicLevel {
                level: DEFAULT_MUSIC_LEVEL,
            },
        ]
    );
}
