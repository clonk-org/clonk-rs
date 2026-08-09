use super::*;

#[test]
fn native_fire_shadow_probe_uses_script_tables_without_building_host_world(
) -> Result<(), EngineError> {
    // C4Effect::AssignCallbackFunctions resolves a commandless internal Fire
    // effect through Game.ScriptEngine (src/C4Effect.cpp:31-57), while
    // C4Object::ExecFire only dispatches the script timer when that lookup
    // shadows the native callback (src/C4Object.cpp:1257-1267).
    let mut engine = Engine::new();
    engine.register_definition(Definition::from_script("HUT1", "Hut", "")?)?;
    let fire = EffectState::new(C4FX_FIRE);
    let fallback = DefinitionId::from("HUT1");

    HOST_WORLD_CONTEXT_BASE_MATERIALIZATIONS.with(|count| count.set(0));
    assert!(!engine.effect_has_script_callback(&fire, &fallback, "Timer"));

    engine.register_definition(Definition::from_script(
        "OVRD",
        "Override",
        "#strict\nglobal func FxFireTimer(target, number, time) { return 0; }\n",
    )?)?;
    assert!(engine.effect_has_script_callback(&fire, &fallback, "Timer"));

    assert!(engine.remove_definition("OVRD"));
    assert!(
        !engine.effect_has_script_callback(&fire, &fallback, "Timer"),
        "a removed LinkedTo host must not leave a stale shadow"
    );
    assert_eq!(
        HOST_WORLD_CONTEXT_BASE_MATERIALIZATIONS.with(Cell::get),
        0,
        "a no-target Fire shadow probe needs script tables, not a callback world"
    );
    Ok(())
}
