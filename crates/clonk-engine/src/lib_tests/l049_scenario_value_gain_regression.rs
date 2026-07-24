use super::*;

#[test]
fn l049_scenario_value_gain_uses_cpp_integer_truthiness() {
    let mut engine = Engine::new();
    for (value_gain, expected) in [(0, false), (1, true), (-1, true)] {
        engine.set_scenario_values(scenario::ScenarioValueStore::with_value_gain_for_test(
            value_gain,
        ));
        assert_eq!(
            engine.scenario_value_gain_enabled(),
            expected,
                "ValueGain={value_gain}"
        );
    }
}
