use super::*;

fn speed_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_scenario_script_with_convention(
                "SetGameSpeed.c",
                r#"
#strict
func SetSpeed(int speed) { return SetGameSpeed(speed); }
func ResetSpeed() { return SetGameSpeed(); }
func Helper() { return SetGameSpeed(38); }
func EvalSpeed() { return eval("SetGameSpeed(76)"); }
"#,
            true,
        )
        .expect("SetGameSpeed probe compiles");
    engine
}

fn call(engine: &mut Engine, function: &str, args: Vec<Value>) -> Value {
    engine
        .call_scenario_script_value(function, &args)
        .expect("speed probe executes")
        .expect("speed probe function exists")
}

#[test]
fn set_game_speed_validates_and_restarts_the_application_timer() {
    let mut engine = speed_engine();
    assert_eq!(engine.game_tick_delay_ms(), DEFAULT_GAME_TICK_DELAY_MS);

    assert_eq!(
        call(&mut engine, "SetSpeed", vec![Value::Int(76)]),
        Value::Bool(true)
    );
    assert_eq!(engine.game_tick_delay_ms(), 13);
    let first_revision = engine.game_tick_delay_revision();

    assert_eq!(
        call(&mut engine, "SetSpeed", vec![Value::Int(76)]),
        Value::Bool(true)
    );
    assert_eq!(engine.game_tick_delay_ms(), 13);
    assert_ne!(engine.game_tick_delay_revision(), first_revision);

    for invalid in [-5, 1001] {
        let revision = engine.game_tick_delay_revision();
        assert_eq!(
            call(&mut engine, "SetSpeed", vec![Value::Int(invalid)]),
            Value::Bool(false)
        );
        assert_eq!(engine.game_tick_delay_ms(), 13);
        assert_eq!(engine.game_tick_delay_revision(), revision);
    }

    assert_eq!(
        call(&mut engine, "ResetSpeed", Vec::new()),
        Value::Bool(true)
    );
    assert_eq!(engine.game_tick_delay_ms(), 26);

    for (speed, delay) in [(1, 1000), (1000, 1)] {
        assert_eq!(
            call(&mut engine, "SetSpeed", vec![Value::Int(speed)]),
            Value::Bool(true)
        );
        assert_eq!(engine.game_tick_delay_ms(), delay);
    }
}

#[test]
fn league_rejects_only_an_immediate_temporary_script_caller() {
    let mut engine = speed_engine();
    engine.set_league_game(true);

    assert_eq!(
        call(&mut engine, "SetSpeed", vec![Value::Int(76)]),
        Value::Bool(true)
    );
    assert_eq!(engine.game_tick_delay_ms(), 13);

    assert_eq!(
        engine
            .direct_exec_script_control_global("SetGameSpeed(38)", "internal script", Some(3),)
            .expect("direct league expression executes its false branch"),
        Value::Bool(false)
    );
    assert_eq!(engine.game_tick_delay_ms(), 13);

    assert_eq!(
        engine
            .direct_exec_scenario_script("Helper()", "internal script", Some(3))
            .expect("ordinary helper called from DirectExec is allowed"),
        Value::Bool(true)
    );
    assert_eq!(engine.game_tick_delay_ms(), 26);

    assert_eq!(
        call(&mut engine, "EvalSpeed", Vec::new()),
        Value::Bool(false)
    );
    assert_eq!(engine.game_tick_delay_ms(), 26);
}

#[test]
fn synthesized_speed_command_resolves_through_direct_exec() {
    let mut engine = speed_engine();
    let source = InitialNetworkMessageBoardCommand::speed()
        .script
        .replace("%d", "76");
    assert_eq!(source, "SetGameSpeed(76)");
    assert_eq!(
        engine
            .direct_exec_script_control_global(&source, "internal script", Some(3))
            .expect("stock speed command executes"),
        Value::Bool(true)
    );
    assert_eq!(engine.game_tick_delay_ms(), 13);
}
