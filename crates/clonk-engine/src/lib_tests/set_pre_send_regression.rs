use super::*;

fn network_engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_network_game(true);
    engine.set_network_control_mode(true);
    engine
}

#[test]
fn network_set_pre_send_returns_normally_and_retains_ordered_requests() {
    let mut engine = network_engine();
    engine
        .load_scenario_script_with_convention(
                "SetPreSend.c",
            concat!(
                    "#strict\nfunc Probe() { return [",
                    "SetPreSend(-1), SetPreSend(0), ",
                    "SetPreSend(76, \"Client A?i*\"), SetPreSend(55, \"Host*\")]; }\n",
            ),
            true,
        )
        .expect("SetPreSend is registered during script linking");

    assert_eq!(
        engine
            .call_scenario_script_value("Probe", &[])
            .expect("network SetPreSend never raises a parity boundary"),
        Some(Value::Array(vec![
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
        ]))
    );
    assert_eq!(
        engine.take_network_target_fps_requests(),
        vec![
            NetworkTargetFpsRequest {
                target_fps: 38,
                client_pattern: None,
            },
            NetworkTargetFpsRequest {
                target_fps: 76,
                client_pattern: Some("Client A?i*".into()),
            },
            NetworkTargetFpsRequest {
                target_fps: 55,
                client_pattern: Some("Host*".into()),
            },
        ]
    );
}

#[test]
fn offline_set_pre_send_is_not_misclassified_as_a_script_failure() {
    let mut engine = Engine::new();
    engine
        .load_scenario_script_with_convention(
                "SetPreSend.c",
                "#strict\nfunc Probe() { SetPreSend(0); SetPreSend(-1); return 1; }\n",
            true,
        )
        .expect("SetPreSend is registered during script linking");

    engine
        .call_scenario_script_function("Probe", Vec::new())
        .expect("offline SetPreSend follows the native no-op path");
    assert!(engine.take_network_target_fps_requests().is_empty());

    engine.set_network_game(true);
    engine
        .call_scenario_script_function("Probe", Vec::new())
        .expect("preserved IsNetworkGame is still local control");
    assert!(engine.take_network_target_fps_requests().is_empty());
}

#[test]
fn fail_safe_initialize_keeps_the_local_pacing_request() {
    let mut engine = network_engine();
    engine
        .register_definition(
            Definition::from_script(
                    "PRES",
                    "PreSend probe",
                    "#strict\nfunc Initialize() { SetPreSend(30); }\n",
            )
            .expect("definition compiles"),
        )
        .expect("definition registers");

    engine
        .spawn_object(SpawnConfig::new("PRES"))
        .expect("Initialize SetPreSend succeeds");
    assert_eq!(
        engine.take_network_target_fps_requests(),
        vec![NetworkTargetFpsRequest {
            target_fps: 30,
            client_pattern: None,
        }]
    );
}

#[test]
fn nested_creation_keeps_the_local_pacing_request() {
    let mut engine = network_engine();
    engine
        .register_definition(
            Definition::from_script(
                    "PRES",
                    "Nested PreSend probe",
                    "#strict\nfunc Construction() { SetPreSend(30); }\n",
            )
            .expect("definition compiles"),
        )
        .expect("definition registers");
    engine
        .load_scenario_script_with_convention(
                "NestedSetPreSend.c",
                "#strict\nfunc Probe() { CreateObject(PRES); return 1; }\n",
            true,
        )
        .expect("scenario script compiles");

    engine
        .call_scenario_script_function("Probe", Vec::new())
        .expect("nested Construction SetPreSend succeeds");
    assert_eq!(
        engine.take_network_target_fps_requests(),
        vec![NetworkTargetFpsRequest {
            target_fps: 30,
            client_pattern: None,
        }]
    );
}

#[test]
fn initialize_def_keeps_the_local_pacing_request() {
    let mut engine = network_engine();
    engine
        .register_definition(
            Definition::from_script(
                    "PDEF",
                    "InitializeDef PreSend probe",
                    "#strict\nfunc InitializeDef() { SetPreSend(30); }\n",
            )
            .expect("definition compiles"),
        )
        .expect("definition registers");

    engine
        .initialize_definition_scripts()
        .expect("InitializeDef SetPreSend succeeds");
    assert_eq!(
        engine.take_network_target_fps_requests(),
        vec![NetworkTargetFpsRequest {
            target_fps: 30,
            client_pattern: None,
        }]
    );
}
