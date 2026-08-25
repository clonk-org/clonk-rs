use super::*;

#[test]
fn a_weather_meteor_runs_the_creation_lifecycle_native_gives_it() {
    // C4Weather::Execute reaches the meteor through Game.CreateObject
    // (oracle-src-pinned src/C4Weather.cpp:116-120), so the object gets the
    // whole creation lifecycle -- and C4Object::DoCon calls Completion when it
    // crosses full Con (src/C4Object.cpp:1506-1510).
    //
    // That callback is load-bearing for the synchronized ledger, not just for
    // object state. METO carries exactly one action, whose ActMap entry is
    // `StartCall=SmokeTrail`, and the shipped script reaches it from there:
    //
    //     public func Completion() { explosion_base = 10; SetAction("Evaporate"); }
    //
    // SmokeTrail draws (Random(7), Random(50), then Random(41)/Random(11)
    // twice per particle). Spawning the meteor raw skips Completion, so the
    // port made none of those draws and its ledger fell behind a stock C++
    // client from the first meteor onwards -- a desync at frame 810 of
    // ClonkMars 01_Fossae (clonk-org/clonk-rs#1085).
    //
    // The observable pinned here is that Completion ran at all; the draws are
    // the shipped script's business, and asserting a draw count would pin the
    // other disaster gates' short-circuiting rather than this defect.
    let mut engine = Engine::with_seed(0x1085);
    engine.set_landscape(Landscape::flat(400, 200));
    crate::TestValueExt::test_value(engine.register_script_definition(
        "METO",
        "Meteor",
        "#strict\n\nfunc Completion() { SetPosition(7, 9); }\n",
    ));
    // Random(100) < 100 always passes, so Random(60) is the only gate left.
    engine.environment.meteorite = 100;

    let mut frame = 0;
    while !engine
        .objects
        .iter()
        .any(|object| object.definition_id == "METO")
    {
        frame += 10;
        crate::TestValueExt::test_value(engine.tick_weather_events(frame).ok());
        assert!(frame < 10_000, "the Random(60) meteor gate never opened");
    }

    let meteor = engine
        .objects
        .iter()
        .find(|object| object.definition_id == "METO")
        .expect("the weather created a meteor");
    assert_eq!(
        (meteor.state.position.x, meteor.state.position.y),
        (7, 9),
        "Completion must run during weather-driven meteor creation"
    );
}
