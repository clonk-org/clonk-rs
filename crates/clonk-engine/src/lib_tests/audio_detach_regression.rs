use super::*;

fn sound_source_engine() -> (Engine, ObjectId, Vector2) {
    let mut engine = Engine::with_seed(0x4c_30_30_31);
    engine
        .register_definition(
            Definition::from_script("SND1", "Sound source", "").expect("sound definition compiles"),
        )
        .expect("sound definition registers");
    let position = Vector2::new(321, 654);
    let object = engine
        .spawn_object(SpawnConfig::new("SND1").with_position(position))
        .expect("sound source spawns");
    (engine, object, position)
}

fn impact_command(object: ObjectId) -> AudioCommand {
    AudioCommand::PlaySound {
        name: "Impact".into(),
        target: Some(object),
        volume: 100,
        looped: false,
        multiple: false,
        custom_falloff: None,
    }
}

fn fire_loop_command(object: ObjectId) -> AudioCommand {
    AudioCommand::PlaySound {
        name: "Fire".into(),
        target: Some(object),
        volume: 100,
        looped: true,
        multiple: false,
        custom_falloff: None,
    }
}

#[test]
fn native_destroy_detaches_loop_at_the_objects_final_position() {
    let (mut engine, object, position) = sound_source_engine();
    engine
        .audio_registry
        .play_sound("Fire", Some(object), 100, true, false, None);
    engine.audio_registry.take_events();
    engine.pending_audio.clear();

    let index = engine.find_object_index(object).expect("source remains");
    engine.objects[index].mark_destroyed();
    let snapshot = engine.tick().expect("native removal frame succeeds");

    assert!(snapshot.object(object).is_none());
    assert_eq!(
        snapshot.audio,
        vec![AudioCommand::DetachObjectSounds {
            target: object,
            position,
        }]
    );
}

#[test]
fn native_destroy_without_object_audio_emits_no_detach_command() {
    let (mut engine, object, _) = sound_source_engine();
    let index = engine.find_object_index(object).expect("source remains");
    engine.objects[index].mark_destroyed();

    let snapshot = engine.tick().expect("native removal frame succeeds");

    assert!(snapshot.audio.is_empty());
}

#[test]
fn same_frame_direct_one_shot_detaches_once_in_command_order() {
    let (mut engine, object, position) = sound_source_engine();
    let play = impact_command(object);
    engine.pending_audio.push(play.clone());
    let index = engine.find_object_index(object).expect("source remains");
    engine.objects[index].mark_destroyed();

    let snapshot = engine.tick().expect("native removal frame succeeds");

    assert_eq!(
        snapshot.audio,
        vec![
            play,
            AudioCommand::DetachObjectSounds {
                target: object,
                position,
            },
        ]
    );
    engine.audio_registry.detach_object_sounds(object, position);
    assert!(engine.audio_registry.take_events().is_empty());
}

#[test]
fn same_frame_direct_loop_detaches_once_in_command_order() {
    let (mut engine, object, position) = sound_source_engine();
    let play = fire_loop_command(object);
    engine.pending_audio.push(play.clone());
    let index = engine.find_object_index(object).expect("source remains");
    engine.objects[index].mark_destroyed();

    let snapshot = engine.tick().expect("native removal frame succeeds");

    assert_eq!(
        snapshot.audio,
        vec![
            play,
            AudioCommand::DetachObjectSounds {
                target: object,
                position,
            },
        ]
    );
}

#[test]
fn delivered_one_shot_target_is_remembered_until_later_removal() {
    let (mut engine, object, position) = sound_source_engine();
    let play = impact_command(object);
    engine.pending_audio.push(play.clone());
    assert_eq!(
        engine.tick().expect("delivery frame succeeds").audio,
        vec![play]
    );

    let index = engine.find_object_index(object).expect("source remains");
    engine.objects[index].mark_destroyed();
    assert_eq!(
        engine.tick().expect("removal frame succeeds").audio,
        vec![AudioCommand::DetachObjectSounds {
            target: object,
            position,
        }]
    );
}

#[test]
fn delivered_loop_target_is_remembered_until_later_removal() {
    let (mut engine, object, position) = sound_source_engine();
    let play = fire_loop_command(object);
    engine.pending_audio.push(play.clone());
    assert_eq!(
        engine.tick().expect("delivery frame succeeds").audio,
        vec![play]
    );

    let index = engine.find_object_index(object).expect("source remains");
    engine.objects[index].mark_destroyed();
    assert_eq!(
        engine.tick().expect("removal frame succeeds").audio,
        vec![AudioCommand::DetachObjectSounds {
            target: object,
            position,
        }]
    );
}

#[test]
fn restore_discards_sound_bindings_from_the_replaced_world() {
    let (mut engine, object, _) = sound_source_engine();
    let state = engine.capture_state();
    engine
        .audio_registry
        .play_sound("Impact", Some(object), 100, false, false, None);
    engine
        .pending_audio
        .extend(engine.audio_registry.take_events());

    engine.restore_state(&state).expect("state restores");
    let index = engine
        .find_object_index(object)
        .expect("restored source remains");
    engine.objects[index].mark_destroyed();
    let snapshot = engine.tick().expect("restored removal succeeds");

    assert_eq!(
        snapshot.audio,
        vec![
            AudioCommand::SetMusicPlaylist {
                playlist: None,
                restart: false,
            },
            AudioCommand::SetMusicLevel {
                level: DEFAULT_MUSIC_LEVEL,
            },
        ]
    );
}
