use super::*;

fn command(name: &[u8], argument: &[u8], player: i32, by_client: i32) -> CustomCommandControlData {
    CustomCommandControlData {
        command: LegacyCString::from_bytes(name.to_vec()).expect("name is NUL-free"),
        argument: LegacyCString::from_bytes(argument.to_vec()).expect("argument is NUL-free"),
        player,
        by_client,
    }
}

fn registered(
    name: &str,
    script: &str,
    restriction: MessageBoardCommandRestriction,
) -> InitialNetworkMessageBoardCommand {
    InitialNetworkMessageBoardCommand {
        name: name.to_string(),
        script: script.to_string(),
        restriction,
    }
}

fn probe_engine() -> Engine {
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[(
            "CustomCommandProbe.c".to_string(),
            "static CustomCommandProbe;\n\
                 global func Capture(value) { CustomCommandProbe = value; return true; }"
                .to_string(),
        )]),
        1
    );
    engine
}

fn probe(engine: &Engine) -> Value {
    let cell = engine
        .script_globals
        .borrow()
        .get("CustomCommandProbe")
        .cloned()
        .expect("probe global is linked");
    let value = cell.borrow().clone();
    value
}

fn execute(engine: &mut Engine, control: &CustomCommandControlData, game_running: bool) -> bool {
    engine
        .execute_custom_command_control(control, game_running)
        .expect("custom-command execution is not a fatal engine error")
}

fn call_script(engine: &mut Engine, function: &str) -> Value {
    engine
        .call_scenario_script_value(function, &[])
        .expect("message-board registration script executes")
        .expect("message-board registration function exists")
}

#[test]
fn custom_command_registry_is_first_wins_and_enters_join_data() {
    let mut already_disabled = Engine::new();
    already_disabled.disable_debug();
    assert_eq!(
        already_disabled.message_board_commands(),
        &[InitialNetworkMessageBoardCommand::speed()],
        "C++ removes /speed only when DebugMode was active"
    );

    let mut engine = Engine::new();
    assert_eq!(
        engine.message_board_commands(),
        &[InitialNetworkMessageBoardCommand::speed()]
    );
    assert!(engine.add_message_board_command(registered(
        "probe",
        "Capture(1)",
        MessageBoardCommandRestriction::Plain,
    )));
    assert!(!engine.add_message_board_command(registered(
        "probe",
        "Capture(2)",
        MessageBoardCommandRestriction::Identifier,
    )));

    let snapshot = InitialNetworkGameData::from_engine(&engine)
        .expect("command-only engine is representable in JoinData");
    assert_eq!(
        snapshot.message_board_commands,
        engine.message_board_commands()
    );
    assert_eq!(snapshot.message_board_commands[1].script, "Capture(1)");

    let encoded = engine
        .capture_state()
        .to_json_string()
        .expect("command registry serializes");
    let state = EngineState::from_json_str(&encoded).expect("command registry deserializes");
    let mut restored = Engine::new();
    restored
        .restore_state(&state)
        .expect("command registry restores");
    assert_eq!(
        restored.message_board_commands(),
        engine.message_board_commands()
    );

    engine.set_debug_mode(true);
    engine.disable_debug();
    assert!(engine
        .message_board_commands()
        .iter()
        .all(|command| command.name != "speed"));
    assert!(engine
        .message_board_commands()
        .iter()
        .any(|command| command.name == "probe"));
}

#[test]
fn add_msg_board_cmd_validates_the_caller_and_drives_custom_command_execution() {
    let mut engine = probe_engine();

    for (name, restriction) in [("direct-escaped", 0), ("direct-plain", 1)] {
        let source = format!("AddMsgBoardCmd(\"{name}\", \"Capture(11)\", {restriction})");
        assert_eq!(
            engine
                .direct_exec_script_control_global(&source, "internal script", Some(3))
                .expect("the rejected DirectExec registration returns normally"),
            Value::Bool(false)
        );
    }
    assert_eq!(
        engine
            .direct_exec_script_control_global(
                "AddMsgBoardCmd(\"direct-identifier\", \"Capture(12)\", C4MSGCMDR_Identifier)",
                "internal script",
                Some(3),
            )
            .expect("Identifier-restricted DirectExec registration succeeds"),
        Value::Bool(true)
    );
    assert!(engine
        .message_board_commands()
        .iter()
        .any(|command| command.name == "direct-identifier"));
    assert!(engine
        .message_board_commands()
        .iter()
        .all(|command| { command.name != "direct-escaped" && command.name != "direct-plain" }));

    engine
        .load_scenario_script_with_convention(
            "AddMsgBoardCmd.c",
            r#"#strict 2
func RegisterCommands()
{
    var unset;
    return [
        AddMsgBoardCmd(unset, "Capture(1)", C4MSGCMDR_Identifier),
        AddMsgBoardCmd("nil-script", unset, C4MSGCMDR_Identifier),
        AddMsgBoardCmd("invalid-low", "Capture(2)", -1),
        AddMsgBoardCmd("invalid-high", "Capture(3)", 3),
        AddMsgBoardCmd("probe", "Capture(%d)", C4MSGCMDR_Escaped),
        AddMsgBoardCmd("probe", "Capture(999)", C4MSGCMDR_Identifier)
    ];
}

func RegisterFromEval()
{
    return eval("AddMsgBoardCmd(\"eval-command\", \"Capture(8)\", C4MSGCMDR_Plain)");
}
"#,
            true,
        )
        .expect("AddMsgBoardCmd scenario probe compiles");
    assert_eq!(
        call_script(&mut engine, "RegisterCommands"),
        Value::Array(vec![
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
        ])
    );
    assert_eq!(
        call_script(&mut engine, "RegisterFromEval"),
        Value::Bool(false),
        "the unnamed eval frame may not install a Plain command"
    );
    assert!(engine
        .message_board_commands()
        .iter()
        .all(|command| command.name != "eval-command"));

    let probe_commands = engine
        .message_board_commands()
        .iter()
        .filter(|command| command.name == "probe")
        .collect::<Vec<_>>();
    assert_eq!(probe_commands.len(), 1, "the first registration wins");
    assert_eq!(probe_commands[0].script, "Capture(%d)");

    assert!(execute(&mut engine, &command(b"probe", b"73", -1, 9), true));
    assert_eq!(
        probe(&engine),
        Value::Int(73),
        "the registered template executes through CID_CustomCommand"
    );
}

#[test]
fn custom_command_checks_player_running_and_exact_registration_before_execution() {
    let mut engine = probe_engine();
    engine
        .register_player(PlayerConfig::new(3, "Owner"))
        .expect("host player registers");
    engine
        .player_mut(3)
        .expect("player remains registered")
        .set_at_client(PlayerAtClient::HOST);
    assert!(engine.add_message_board_command(registered(
        "who",
        "Capture(\"%player%/%player%\")",
        MessageBoardCommandRestriction::Escaped,
    )));

    let matching = command(b"who", b"ignored", 3, 0);
    assert!(execute(&mut engine, &matching, true));
    assert_eq!(probe(&engine), Value::String("3/3".to_string().into()));

    for rejected in [
        command(b"who", b"ignored", 3, 7),
        command(b"who", b"ignored", 99, 0),
        command(b"WHO", b"ignored", 3, 0),
        command(b"missing", b"ignored", 3, 0),
    ] {
        assert!(!execute(&mut engine, &rejected, true));
        assert_eq!(probe(&engine), Value::String("3/3".to_string().into()));
    }
    assert!(!execute(&mut engine, &matching, false));

    let ownerless = command(b"who", b"ignored", -1, 91);
    assert!(execute(&mut engine, &ownerless, true));
    assert_eq!(probe(&engine), Value::String("-1/-1".to_string().into()));
}

#[test]
fn custom_command_numeric_format_matches_from_chars_prefix_rules() {
    let mut engine = probe_engine();
    assert!(engine.add_message_board_command(registered(
        "number",
        "Capture(%d)",
        MessageBoardCommandRestriction::Identifier,
    )));

    for (argument, expected) in [
        (&b"+17tail"[..], 17),
        (&b"-8junk"[..], -8),
        (&b"12.bad"[..], 12),
        (&b"junk"[..], 0),
        (&b" 12"[..], 0),
        (&b"+"[..], 0),
        (&b"++12"[..], 0),
        (&b"2147483648"[..], 0),
    ] {
        assert!(
            execute(&mut engine, &command(b"number", argument, -1, 55), true,),
            "argument {argument:?}"
        );
        assert_eq!(
            probe(&engine),
            Value::Int(expected),
            "argument {argument:?}"
        );
    }
    assert_eq!(Engine::custom_command_integer(b"-2147483648tail"), i32::MIN);

    assert!(engine.add_message_board_command(registered(
        "percent",
        "Capture(\"%d/%%\")",
        MessageBoardCommandRestriction::Plain,
    )));
    assert!(execute(
        &mut engine,
        &command(b"percent", b"17", -1, 55),
        true,
    ));
    assert_eq!(probe(&engine), Value::String("17/%".to_string().into()));

    for (name, script) in [
        ("repeated", "Capture(%d); Capture(%d)"),
        ("malformed", "Capture(%d) %q"),
        ("mixed", "Capture(%s); Capture(%d)"),
    ] {
        assert!(engine.add_message_board_command(registered(
            name,
            script,
            MessageBoardCommandRestriction::Plain,
        )));
        assert!(execute(
            &mut engine,
            &command(name.as_bytes(), b"91", -1, 55),
            true,
        ));
        assert_eq!(
            probe(&engine),
            Value::String("17/%".to_string().into()),
            "invalid fmt template {script:?} must not reach DirectExec"
        );
    }
}

#[test]
fn custom_command_string_restrictions_match_cpp_escaping_and_prefix_filter() {
    let mut engine = probe_engine();
    for entry in [
        registered(
            "escaped",
            "Capture(\"%s\")",
            MessageBoardCommandRestriction::Escaped,
        ),
        registered(
            "plain",
            "Capture(%s)",
            MessageBoardCommandRestriction::Plain,
        ),
        registered(
            "identifier",
            "Capture(\"%s\")",
            MessageBoardCommandRestriction::Identifier,
        ),
    ] {
        assert!(engine.add_message_board_command(entry));
    }

    assert!(execute(
        &mut engine,
        &command(b"escaped", b"a\\\"b", -1, 4),
        true,
    ));
    assert_eq!(probe(&engine), Value::String("a\\\"b".to_string().into()));

    assert!(execute(&mut engine, &command(b"plain", b"37", -1, 4), true,));
    assert_eq!(probe(&engine), Value::Int(37));

    assert!(execute(
        &mut engine,
        &command(b"identifier", b"AZaz09_~+- space\t.bad", -1, 4),
        true,
    ));
    assert_eq!(
        probe(&engine),
        Value::String("AZaz09_~+- space\t".to_string().into())
    );

    assert!(execute(
        &mut engine,
        &command(b"identifier", b".discarded", -1, 4),
        true,
    ));
    assert_eq!(probe(&engine), Value::String(String::new().into()));

    let legacy_name = clonk_script::c4_string_from_bytes(&[0xff]);
    assert!(engine.add_message_board_command(registered(
        &legacy_name,
        "Capture(73)",
        MessageBoardCommandRestriction::Plain,
    )));
    assert!(execute(&mut engine, &command(&[0xff], b"", -1, 4), true,));
    assert_eq!(probe(&engine), Value::Int(73));
}
