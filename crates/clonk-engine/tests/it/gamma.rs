use clonk_engine::{
    scenario::{LegacyC4SVal, LegacyWeatherInit},
    Definition, Engine, EngineState, EnvironmentSettings, GammaControlState, Recording,
    SpawnConfig,
};

use crate::support::real_scenario::{join_local_player, load_tutorial};

const TUTORIAL06_RAMP: [u32; 3] = [0x000000, 0x646464, 0xc8c8c8];
const COLD_WINTER_RAMP: [u32; 3] = [0x00000a, 0x75759a, 0xe5e5ff];

fn gamma_probe_definition() -> Definition {
    crate::support::TestValueExt::test_value(Definition::from_script(
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

        public func SetWinter()
        {
            SetSeason(0);
        }

        public func SetCold()
        {
            SetTemperature(-20);
        }

        public func SetNewClimate()
        {
            SetClimate(10);
        }

        public func SetSeasonThenExplicitGamma()
        {
            SetSeason(0);
            SetGamma(0x010203, 0x818283, 0xfefdfc, 1);
        }

        public func SetExplicitGammaThenSeason()
        {
            SetGamma(0x010203, 0x818283, 0xfefdfc, 1);
            SetSeason(0);
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
    ))
}

fn gamma_probe_engine() -> Engine {
    let mut engine = Engine::with_seed(0);
    crate::support::TestValueExt::test_value(engine.register_definition(gamma_probe_definition()));
    crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("GAMM")));
    engine
}

fn call_probe(engine: &mut Engine, function: &str) {
    engine
        .call_object_function(0, function, Vec::new())
        .unwrap_or_else(|error| panic!("{function} executes: {error}"));
}

fn cold_winter_weather_init(no_gamma: bool) -> LegacyWeatherInit {
    let flat = |value: i32| LegacyC4SVal::new(value, 0, -100, 100);
    LegacyWeatherInit {
        season: flat(0),
        year_speed: flat(0),
        climate: flat(70), // 100 - 70 - 50 = -20
        wind: flat(0),
        rain: flat(0),
        precipitation: "Water".to_string(),
        lightning: flat(0),
        meteorite: flat(0),
        volcano: flat(0),
        earthquake: flat(0),
        no_initialize: true,
        no_gamma,
    }
}

#[test]
fn weather_init_writes_exact_season_gamma_to_slot_one() {
    // C4Weather::Init calls SetSeasonGamma after assigning Season,
    // Temperature, Climate, and NoGamma (C4Weather.cpp:36-69). Season 0
    // selects the winter row; Temperature=-20 subtracts 10 from red/green
    // and adds 10 to blue (C4Weather.cpp:259-284).
    let init = cold_winter_weather_init(false);
    let mut engine = Engine::with_seed(0);
    engine.set_environment(EnvironmentSettings::new(0).with_gamma_enabled());

    crate::support::TestValueExt::test_value(engine.apply_weather_init(&init));

    assert_eq!(engine.gamma_controls().ramp(1), Some(COLD_WINTER_RAMP));
}

#[test]
fn no_gamma_leaves_the_existing_season_slot_untouched() {
    // SetSeasonGamma's first operation is `if (NoGamma) return;`
    // (C4Weather.cpp:259-261). Init and all three setters therefore retain
    // the previous C4GRI_SEASON value instead of resetting it.
    let mut engine = gamma_probe_engine();
    call_probe(&mut engine, "SetSecond");
    let expected = engine.gamma_controls().ramp(1);
    engine.set_environment(
        EnvironmentSettings::new(0)
            .with_season(50)
            .with_temperature(0)
            .with_gamma_disabled(),
    );

    crate::support::TestValueExt::test_value(
        engine.apply_weather_init(&cold_winter_weather_init(true)),
    );
    assert_eq!(engine.gamma_controls().ramp(1), expected);

    for function in ["SetWinter", "SetCold", "SetNewClimate"] {
        call_probe(&mut engine, function);
        assert_eq!(
            engine.gamma_controls().ramp(1),
            expected,
            "{function} leaves slot 1 untouched"
        );
    }
}

#[test]
fn weather_setters_refresh_slot_one_like_cpp() {
    // C4Weather::SetTemperature, SetClimate, and SetSeason each call
    // SetSeasonGamma after updating their field (C4Weather.cpp:223-243).
    let mut engine = gamma_probe_engine();
    engine.set_environment(
        EnvironmentSettings::new(0)
            .with_season(50)
            .with_temperature(0)
            .with_gamma_enabled(),
    );

    call_probe(&mut engine, "SetSecond");
    call_probe(&mut engine, "SetWinter");
    assert_eq!(
        engine.gamma_controls().ramp(1),
        Some([0x000000, 0x7f7f90, 0xefefff]),
        "SetSeason refreshes the weather slot"
    );

    call_probe(&mut engine, "SetSecond");
    call_probe(&mut engine, "SetCold");
    assert_eq!(
        engine.gamma_controls().ramp(1),
        Some(COLD_WINTER_RAMP),
        "SetTemperature refreshes the weather slot"
    );

    call_probe(&mut engine, "SetSecond");
    call_probe(&mut engine, "SetNewClimate");
    assert_eq!(
        engine.gamma_controls().ramp(1),
        Some(COLD_WINTER_RAMP),
        "SetClimate refreshes the weather slot even though climate is not a ramp input"
    );
}

#[test]
fn season_rollover_refreshes_gamma_before_temperature_drift() {
    // C4Weather::Execute calls SetSeasonGamma only inside the rollover
    // branch, before the separate Tick35 temperature drift
    // (C4Weather.cpp:72-93). At -10 the winter curve shifts by five even
    // though this same frame then raises Temperature to -9.
    let mut environment = EnvironmentSettings::new(0)
        .with_season(0)
        .with_season_bounds(0, 100)
        .with_year_speed(1)
        .with_climate(0)
        .with_temperature_range(0)
        .with_temperature(-10)
        .with_gamma_enabled();
    environment.season_delay = 199;
    let mut engine = Engine::with_seed(0);
    engine.set_environment(environment);

    for _ in 0..34 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.gamma_controls().ramp(1),
        Some(GammaControlState::DEFAULT_RAMP),
        "non-Tick35 frames do not rewrite the season control"
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(engine.environment().season, 1);
    assert_eq!(engine.environment().temperature, -9);
    assert_eq!(
        engine.gamma_controls().ramp(1),
        Some([0x000005, 0x7a7a95, 0xeaeaff]),
        "gamma uses the pre-drift -10 temperature"
    );
}

#[test]
fn weather_and_explicit_gamma_writes_retain_script_call_order() {
    // C++ applies both calls immediately: SetSeason delegates to
    // SetSeasonGamma (C4Weather.cpp:229-233), while SetGamma writes the
    // requested slot directly (C4Script.cpp:4998-5006). The later call
    // therefore owns C4GRI_SEASON when both occur in one callback.
    let mut engine = gamma_probe_engine();
    let summer = EnvironmentSettings::new(0)
        .with_season(50)
        .with_temperature(0)
        .with_gamma_enabled();
    engine.set_environment(summer);
    call_probe(&mut engine, "SetSeasonThenExplicitGamma");
    assert_eq!(
        engine.gamma_controls().ramp(1),
        Some([0x010203, 0x818283, 0xfefdfc]),
        "later explicit SetGamma wins"
    );

    engine.set_environment(summer);
    call_probe(&mut engine, "SetExplicitGammaThenSeason");
    assert_eq!(
        engine.gamma_controls().ramp(1),
        Some([0x000000, 0x7f7f90, 0xefefff]),
        "later SetSeasonGamma wins"
    );
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

    let serialized =
        crate::support::TestValueExt::test_value(engine.capture_state().to_json_string());
    let state = crate::support::TestValueExt::test_value(EngineState::from_json_str(&serialized));
    assert_eq!(state.gamma, expected);

    let mut restored = Engine::with_seed(99);
    crate::support::TestValueExt::test_value(
        restored.register_definition(gamma_probe_definition()),
    );
    crate::support::TestValueExt::test_value(restored.restore_state(&state));
    assert_eq!(*restored.gamma_controls(), expected);

    let snapshot = restored.snapshot();
    assert_eq!(snapshot.environment.gamma, expected);
    let mut recording = Recording::new();
    recording.push(snapshot);
    let serialized = crate::support::TestValueExt::test_value(recording.to_string());
    let recording = crate::support::TestValueExt::test_value(Recording::from_str(&serialized));
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
