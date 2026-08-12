use clonk_engine::{compat, Engine, SpawnConfig};
use clonk_script::Value;

fn call_modulate_color(strict_level: u8, first_color: &str) -> Value {
    let mut script = clonk_script::Engine::new();
    compat::register_host_functions(&mut script);
    crate::support::TestValueExt::test_value(script.load_script(&format!(
        "#strict {strict_level}\nfunc Probe() {{ return ModulateColor({first_color}, -1); }}"
    )));
    crate::support::TestValueExt::test_value(script.call("Probe", &[]))
}

fn custom_message_color(strict_level: u8, color: &str) -> u32 {
    let script = format!(
        "#strict {strict_level}\nfunc Probe() {{ var unset; return CustomMessage(\"probe\", unset, unset, unset, unset, {color}); }}"
    );
    let mut engine = Engine::new();
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "CMST",
        "CustomMessage strictness probe",
        &script,
    ));
    let object =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("CMST")));
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(object));
    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("CustomMessage probe runs"),
        Value::Bool(true)
    );

    let messages = engine.snapshot().hud.messages;
    assert_eq!(messages.len(), 1);
    messages[0].color
}

#[test]
fn modulate_color_optional_default_follows_caller_strictness() {
    for falsy in ["false", "0"] {
        assert_eq!(
            call_modulate_color(2, falsy),
            Value::Int(-65_794),
            "below strict 3, {falsy} is absent and defaults to white"
        );
        assert_eq!(
            call_modulate_color(3, falsy),
            Value::Int(-16_777_216),
            "at strict 3, {falsy} is an explicit zero color"
        );
    }
}

#[test]
fn custom_message_optional_color_default_follows_caller_strictness() {
    for falsy in ["false", "0"] {
        assert_eq!(
            custom_message_color(2, falsy),
            0xffff_ffff,
            "below strict 3, {falsy} is absent and defaults to white"
        );
        assert_eq!(
            custom_message_color(3, falsy),
            0xff00_0000,
            "at strict 3, {falsy} is an explicit zero color"
        );
    }

    assert_eq!(custom_message_color(2, "true"), 0xff00_0001);
    assert_eq!(custom_message_color(3, "true"), 0xff00_0001);
}
