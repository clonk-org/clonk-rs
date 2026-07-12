use lc_engine::AudioCommand;

use crate::support::real_scenario::load_tutorial;

#[test]
fn tutorial07_tick10_starts_cpp_wind_loop_at_level_40() {
    // Tutorial07 fixes Wind=50 (Scenario.txt:74). C4Weather::Execute steps
    // wind and then runs SoundLevel("Wind", nullptr, (abs(Wind)-30)*2) on
    // Tick10 (C4Weather.cpp:94-104), yielding one global loop at level 40.
    let mut engine = load_tutorial(7, 0);
    assert_eq!(engine.environment().wind, 50);

    for _ in 0..9 {
        let frame = engine.tick().expect("pre-Tick10 tutorial frame succeeds");
        assert!(
            frame.audio.iter().all(|event| !is_wind_event(event)),
            "Tutorial07 must not start wind audio before Tick10"
        );
    }

    let wind_events = engine
        .tick()
        .expect("Tutorial07 Tick10 succeeds")
        .audio
        .into_iter()
        .filter(is_wind_event)
        .collect::<Vec<_>>();
    assert_eq!(
        wind_events,
        vec![AudioCommand::PlaySound {
            name: "Wind".to_string(),
            target: None,
            volume: 40,
            looped: true,
            multiple: false,
            custom_falloff: None,
        }]
    );
}

fn is_wind_event(event: &AudioCommand) -> bool {
    match event {
        AudioCommand::PlaySound { name, .. }
        | AudioCommand::StopSound { name, .. }
        | AudioCommand::SetSoundVolume { name, .. } => name.eq_ignore_ascii_case("Wind"),
        AudioCommand::PlayMusic { .. }
        | AudioCommand::StopMusic
        | AudioCommand::SetMusicLevel { .. } => false,
    }
}
