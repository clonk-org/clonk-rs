use clonk_engine::AudioCommand;

use crate::support::real_scenario::load_tutorial;

#[test]
fn tutorial07_real_acid_rain_starts_on_each_cpp_phase_call() {
    // The shipped FXP1 Process action has Delay=2 and PhaseCall=Precipitation
    // (Objects.c4d/Effects.c4d/Precipitation.c4d/ActMap.txt). Each callback
    // performs three InsertMaterial calls, and Tutorial07's fixed strength 77
    // makes `Random(50) > iStrength` impossible (its Scenario.txt and the
    // shipped Precipitation Script.c). C4Object::ExecAction invokes PhaseCall
    // after advancing Phase on every second execution (C4Object.cpp:5466-5476).
    let mut engine = load_tutorial(7, 0);

    let acid_rain_count = |engine: &clonk_engine::Engine| {
        engine
            .snapshot()
            .particles
            .into_iter()
            .filter(|particle| particle.definition_id == "material/pxs/acidrain")
            .count()
    };

    let expected = [0, 3, 3, 6];
    for (tick, expected_count) in (1..=4).zip(expected) {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        assert_eq!(
            acid_rain_count(&engine),
            expected_count,
            "shipped Tutorial07 precipitation count after tick {tick}",
        );
    }
}

#[test]
fn tutorial07_tick10_starts_cpp_wind_loop_at_level_40() {
    // Tutorial07 fixes Wind=50 (Scenario.txt:74). C4Weather::Execute steps
    // wind and then runs SoundLevel("Wind", nullptr, (abs(Wind)-30)*2) on
    // Tick10 (C4Weather.cpp:94-104). The frontend resolves this atomically by
    // updating a live instance or starting one global loop at level 40.
    let mut engine = load_tutorial(7, 0);
    assert_eq!(engine.environment().wind, 50);

    for _ in 0..9 {
        let frame = crate::support::TestValueExt::test_value(engine.tick());
        assert!(
            frame.audio.iter().all(|event| !is_wind_event(event)),
            "Tutorial07 must not start wind audio before Tick10"
        );
    }

    let wind_events = crate::support::TestValueExt::test_value(engine.tick())
        .audio
        .into_iter()
        .filter(is_wind_event)
        .collect::<Vec<_>>();
    assert_eq!(
        wind_events,
        vec![AudioCommand::SetSoundVolume {
            name: "Wind".to_string(),
            target: None,
            volume: 40,
        }]
    );
}

fn is_wind_event(event: &AudioCommand) -> bool {
    match event {
        AudioCommand::PlaySound { name, .. }
        | AudioCommand::PlaySpeech { name, .. }
        | AudioCommand::PlaySoundAt { name, .. }
        | AudioCommand::StopSound { name, .. }
        | AudioCommand::SetSoundVolume { name, .. } => name.eq_ignore_ascii_case("Wind"),
        AudioCommand::DetachObjectSounds { .. }
        | AudioCommand::PlayMusic { .. }
        | AudioCommand::StopMusic
        | AudioCommand::SetMusicLevel { .. }
        | AudioCommand::SetMusicPlaylist { .. } => false,
    }
}
