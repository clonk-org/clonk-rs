use clonk_engine::{
    Definition, Engine, EngineError, PlayerConfig, ShowCommandsRequestStore, SpawnConfig, COM_THROW,
};
use clonk_script::Value;

#[test]
fn set_plr_show_command_stages_runtime_flash_and_enables_command_display() -> Result<(), EngineError>
{
    let requests = ShowCommandsRequestStore::default();
    let mut engine = Engine::new();
    engine.set_show_commands_request_store(requests.clone());
    engine.register_player(PlayerConfig::new(3, "Player 3"))?;
    engine.register_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "SHCM",
            "Show-command probe",
            r#"#strict 3
        func Probe(int player, int command)
        {
            return SetPlrShowCommand(player, command);
        }
        "#,
        ),
    ))?;
    let probe = engine.spawn_object(SpawnConfig::new("SHCM"))?;
    let probe_index = crate::support::TestValueExt::test_value(engine.find_object_index(probe));

    assert_eq!(
        engine.call_object_function(
            probe_index,
            "Probe",
            vec![Value::Int(3), Value::Int(i32::from(COM_THROW))],
        )?,
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .player(3)
            .expect("player remains live")
            .flash_command(),
        i32::from(COM_THROW)
    );
    assert!(requests.take_enable_request());
    assert!(
        !requests.take_enable_request(),
        "the config request is one-shot"
    );

    assert_eq!(
        engine.call_object_function(probe_index, "Probe", vec![Value::Int(99), Value::Int(23)],)?,
        Value::Bool(false)
    );
    assert_eq!(
        engine
            .player(3)
            .expect("player remains live")
            .flash_command(),
        i32::from(COM_THROW),
        "an invalid player leaves the prior flash command unchanged"
    );
    assert!(!requests.take_enable_request());

    assert_eq!(
        engine.call_object_function(probe_index, "Probe", vec![Value::Int(3), Value::Int(0)],)?,
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .player(3)
            .expect("player remains live")
            .flash_command(),
        0
    );
    assert!(
        requests.take_enable_request(),
        "COM_None still force-enables ShowCommands"
    );

    engine.call_object_function(probe_index, "Probe", vec![Value::Int(3), Value::Int(17)])?;
    assert_eq!(
        engine
            .player(3)
            .expect("player remains live")
            .flash_command(),
        17
    );
    let saved = engine.capture_state();
    engine.restore_state(&saved)?;
    assert_eq!(
        engine
            .player(3)
            .expect("restored player exists")
            .flash_command(),
        0,
        "C4Player::FlashCom is NoSave"
    );
    Ok(())
}
