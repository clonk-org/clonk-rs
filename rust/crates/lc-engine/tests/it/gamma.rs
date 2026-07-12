use lc_engine::{Definition, Engine, EngineState, GammaControlState, Recording, SpawnConfig};

use crate::support::real_scenario::{join_local_player, load_tutorial};

const TUTORIAL06_RAMP: [u32; 3] = [0x000000, 0x646464, 0xc8c8c8];

fn gamma_probe_definition() -> Definition {
    Definition::from_script(
        "GAMM",
        "Gamma probe",
        r#"
        #strict

        public func SetTutorial()
        {
            SetGamma(0x000000, 0x646464, 0xc8c8c8);
        }

        public func SetSecond()
        {
            SetGamma(0x010203, 0x818283, 0xfefdfc, 1);
        }

        public func ResetDefault()
        {
            ResetGamma();
        }

        public func SetInvalidLow()
        {
            SetGamma(0x111111, 0x222222, 0x333333, -1);
        }

        public func SetInvalidHigh()
        {
            SetGamma(0x111111, 0x222222, 0x333333, 9);
        }
        "#,
    )
    .expect("gamma probe compiles")
}

fn gamma_probe_engine() -> Engine {
    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(gamma_probe_definition())
        .expect("gamma probe registers");
    engine
        .spawn_object(SpawnConfig::new("GAMM"))
        .expect("gamma probe spawns");
    engine
}

fn call_probe(engine: &mut Engine, function: &str) {
    engine
        .call_object_function(0, function, Vec::new())
        .unwrap_or_else(|error| panic!("{function} executes: {error}"));
}

#[test]
fn gamma_controls_match_cpp_index_defaults_and_additive_curve() {
    let mut engine = gamma_probe_engine();
    assert!(engine.gamma_controls().is_default());
    assert_eq!(
        engine.effective_gamma_control_points(),
        GammaControlState::DEFAULT_RAMP
    );

    // Tutorial06 omits iRampIndex, selecting C4GRI_SCENARIO (slot 0).
    call_probe(&mut engine, "SetTutorial");
    assert_eq!(engine.gamma_controls().ramp(0), Some(TUTORIAL06_RAMP));
    assert_eq!(
        engine.snapshot().environment.gamma,
        *engine.gamma_controls()
    );

    // C4GraphicsSystem::ApplyGamma adds each slot's displacement from the
    // default independently per RGB channel (C4GraphicsSystem.cpp:787-809).
    call_probe(&mut engine, "SetSecond");
    assert_eq!(
        engine.effective_gamma_control_points(),
        [0x010203, 0x656667, 0xc7c6c5]
    );

    let before_invalid = *engine.gamma_controls();
    call_probe(&mut engine, "SetInvalidLow");
    call_probe(&mut engine, "SetInvalidHigh");
    assert_eq!(*engine.gamma_controls(), before_invalid);

    // ResetGamma also defaults its omitted index to slot 0; slot 1 remains.
    call_probe(&mut engine, "ResetDefault");
    assert_eq!(
        engine.gamma_controls().ramp(0),
        Some(GammaControlState::DEFAULT_RAMP)
    );
    assert_eq!(
        engine.effective_gamma_control_points(),
        [0x010203, 0x818283, 0xfefdfc]
    );
}

#[test]
fn gamma_controls_survive_save_snapshot_and_recording_roundtrips() {
    let mut engine = gamma_probe_engine();
    call_probe(&mut engine, "SetTutorial");
    call_probe(&mut engine, "SetSecond");
    let expected = *engine.gamma_controls();

    let serialized = engine
        .capture_state()
        .to_json_string()
        .expect("engine state serializes");
    let state = EngineState::from_json_str(&serialized).expect("engine state parses");
    assert_eq!(state.gamma, expected);

    let mut restored = Engine::with_seed(99);
    restored
        .register_definition(gamma_probe_definition())
        .expect("restored gamma probe registers");
    restored
        .restore_state(&state)
        .expect("engine state restores");
    assert_eq!(*restored.gamma_controls(), expected);

    let snapshot = restored.snapshot();
    assert_eq!(snapshot.environment.gamma, expected);
    let mut recording = Recording::new();
    recording.push(snapshot);
    let serialized = recording.to_string().expect("recording serializes");
    let recording = Recording::from_str(&serialized).expect("recording parses");
    assert_eq!(recording.frames()[0].environment.gamma, expected);

    assert!(Engine::new().gamma_controls().is_default());
}

#[test]
fn real_tutorial06_initialize_player_sets_cpp_scenario_gamma() {
    let mut engine = load_tutorial(6, 0);
    assert!(engine.gamma_controls().is_default());
    join_local_player(&mut engine, "Tutorial06 gamma oracle");

    // Tutorial06/Script.c:11 uses RGB(0), RGB(100), RGB(200) and omits the
    // index; C++ therefore stores these exact packed values in slot 0.
    assert_eq!(engine.gamma_controls().ramp(0), Some(TUTORIAL06_RAMP));
    assert_eq!(engine.effective_gamma_control_points(), TUTORIAL06_RAMP);
}
