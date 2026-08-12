use super::*;

fn say_engine() -> (Engine, ObjectId, ObjectId) {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition("VIEW", "View object", ""));
    crate::TestValueExt::test_value(engine.register_script_definition("CURS", "Cursor object", ""));
    let view = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("VIEW")));
    let cursor = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("CURS")
                .with_custom_name("Speaker")
                .with_color(0),
        ),
    );
    crate::TestValueExt::test_value(engine.register_player(
        PlayerConfig::new(3, "Alice").with_color(Some(RgbColor::new(0x12, 0x34, 0x56))),
    ));
    let player = crate::TestValueExt::test_value(engine.player_mut(3));
    player.set_at_client(PlayerAtClient::new(2));
    player.set_cursor(Some(cursor));
    player.set_view_target(Some(view));
    (engine, view, cursor)
}

fn say(message: Vec<u8>, by_client: i32) -> MessageControlData {
    MessageControlData {
        message_type: MESSAGE_TYPE_SAY,
        player: 3,
        to_player: -1,
        message: crate::TestValueExt::test_value(LegacyCString::from_bytes(message)),
        by_client,
    }
}

#[test]
fn say_rechecks_owner_and_targets_raw_view_target_with_player_color() {
    let (mut engine, view, _) = say_engine();
    let raw = vec![b'h', b'i', 0x80];

    assert!(!engine.execute_message_control_say(&say(raw.clone(), 7)));
    assert!(engine.snapshot().hud.messages.is_empty());

    assert!(engine.execute_message_control_say(&say(raw.clone(), 2)));
    let messages = engine.snapshot().hud.messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].kind, MessageKind::Target);
    assert_eq!(messages[0].target, Some(view));
    assert_eq!(
        messages[0].lines,
        vec![clonk_script::c4_string_from_bytes(&raw)]
    );
    assert_eq!(messages[0].color, 0xff12_3456);
}

#[test]
fn cinematic_say_uses_cursor_name_and_zero_color_fallback() {
    let (mut engine, view, _) = say_engine();
    engine.set_scenario_values(scenario::ScenarioValueStore::with_film_for_test(2));

    assert!(engine.execute_message_control_say(&say(b"action".to_vec(), 2)));
    let messages = engine.snapshot().hud.messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].target, Some(view));
    assert_eq!(messages[0].lines, vec!["<Speaker> action"]);
    assert_eq!(messages[0].color, 0xff00_00ff);
}

#[test]
fn film_view_scope_requires_raw_replay_and_nonzero_film() {
    let mut engine = Engine::new();
    for (replay, film, expected) in [
        (0, 0, false),
        (1, 0, false),
        (0, 1, false),
        (1, 1, true),
        (1, 2, true),
    ] {
        engine.set_scenario_values(scenario::ScenarioValueStore::with_replay_film_for_test(
            replay, film,
        ));
        assert_eq!(
            engine.film_replay(),
            expected,
            "Replay={replay}, Film={film}"
        );
    }
}

#[test]
fn fullscreen_film_fallback_requires_replay_and_nonzero_film_mode() {
    let mut engine = Engine::new();
    engine.set_scenario_values(scenario::ScenarioValueStore::with_replay_film_for_test(
        0, 1,
    ));
    assert!(!engine.is_replay_film());

    engine.set_scenario_values(scenario::ScenarioValueStore::with_replay_film_for_test(
        1, 1,
    ));
    assert!(engine.is_replay_film());

    engine.set_scenario_values(scenario::ScenarioValueStore::with_replay_film_for_test(
        1, 0,
    ));
    assert!(!engine.is_replay_film());

    engine.set_scenario_values(scenario::ScenarioValueStore::with_replay_film_for_test(
        -1, 2,
    ));
    engine.set_replay_control(true);
    crate::TestValueExt::test_value(engine.finish_replay());
    assert!(
        engine.is_replay_film(),
        "ViewportCheck keeps using persistent Head.Replay after ChangeToLocal"
    );
}
