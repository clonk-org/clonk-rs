use clonk_engine::AudioCommand;

use crate::support::real_scenario::{join_local_player_on_team, load_installed_scenario};

#[test]
fn dragon_rock_princess_scream_is_one_targeted_non_looping_effect() {
    // Drachenfels Script25 calls Sound("PrincessScream", false, g_pKing)
    // exactly once (content/Fantasy.c4f/Drachenfels.c4s/Script.c:307-320).
    // The omitted C4Aul loop and multiple parameters both default to zero,
    // so C++ creates a one-shot effect rather than looping scenario music
    // (C4Script.cpp:2297-2327; C4SoundSystem.cpp:321-355).
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    join_local_player_on_team(&mut engine, "Dragon Rock audio parity", 1);
    let mut screams = Vec::new();
    let mut audio_names = Vec::new();
    {
        let mut record_audio = |audio: Vec<AudioCommand>| {
            audio_names.extend(audio.iter().filter_map(|event| match event {
                AudioCommand::PlaySound { name, .. } | AudioCommand::PlayMusic { name, .. } => {
                    Some(name.clone())
                }
                _ => None,
            }));
            screams.extend(audio.into_iter().filter(|event| {
                matches!(
                    event,
                    AudioCommand::PlaySound { name, .. }
                        if name.eq_ignore_ascii_case("PrincessScream")
                )
            }));
        };

        // Script15 pauses the counter until the dragon's shipped IntDragonFree
        // effect calls this scenario callback. Invoke that real callback here so
        // this audio regression is independent of the dragon-flight route; from
        // here onward the ordinary Script20.. counter executes through engine ticks.
        for _ in 0..30 {
            let presentation =
                crate::support::TestValueExt::test_value(engine.tick_with_presentation());
            record_audio(presentation.audio);
        }
        crate::support::TestValueExt::test_value(
            engine.call_scenario_script_function("OnDragonReachTarget", Vec::new()),
        );

        for _ in 0..500 {
            let presentation =
                crate::support::TestValueExt::test_value(engine.tick_with_presentation());
            record_audio(presentation.audio);
        }
    }

    assert_eq!(
        screams.len(),
        1,
        "the shipped intro must emit its scream once, never as recurring music; audio={audio_names:?}; globals={:?}",
        engine.snapshot().script_globals,
    );
    let AudioCommand::PlaySound {
        target,
        volume,
        looped,
        multiple,
        custom_falloff,
        ..
    } = &screams[0]
    else {
        unreachable!("the filter retains only PlaySound events")
    };
    let king = target.expect("the intro scream is spatially targeted at g_pKing");
    assert_eq!(
        engine
            .object_snapshot(king)
            .expect("the king remains present after the intro")
            .definition_id,
        "KING"
    );
    assert_eq!(*volume, 100);
    assert!(!looped, "PrincessScream is a C++ one-shot effect");
    assert!(!multiple, "the omitted C++ fMultiple argument is false");
    assert_eq!(*custom_falloff, None);
}
