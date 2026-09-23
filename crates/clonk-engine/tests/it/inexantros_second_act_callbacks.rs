//! InExantros' second act opened `func Initialize() {` at line 43 of its
//! scenario script and never closed it. In C4Aul the bare `RelaunchPlayer:`
//! label that followed ended the function, and that one label declared
//! nothing (C4AulParse.cpp:2216-2238; pinned on a synthetic script in
//! `clonk-script`'s `test_old_style_functions`), so a hero who died was never
//! relaunched under either engine. clonk-org/clonk-rs-content#81 closed
//! `Initialize`.

use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
use clonk_engine::SpawnConfig;
use clonk_script::Value;

/// Relaunches a hero the way its death does and answers the experience it
/// keeps.
const RELAUNCH_PROBE: &str = r#"#strict
public func Relaunch(object hero)
{
    LocalN("pExp", hero) = 100;
    GameCall("RelaunchPlayer", hero);
    return LocalN("pExp", hero);
}
"#;

#[test]
fn the_second_act_relaunches_a_fallen_hero() {
    let mut engine =
        load_installed_scenario("Collection.c4f/Adventures.c4f/InExantros.c4f/2.Akt.c4s", 0);
    let hero = engine.spawn_test_object(SpawnConfig::new("KNIG"));
    engine
        .register_script_definition("IXPR", "Relaunch probe", RELAUNCH_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("IXPR"));
    let index = engine.test_object_index(probe);

    let experience = engine
        .call_object_function(index, "Relaunch", vec![Value::Object(hero.as_u64())])
        .expect("the relaunch runs");
    // `RelaunchPlayer` takes `60 + gGrad*6` experience, and `Initialize` sets
    // `gGrad` to 1 (`2.Akt.c4s/Script.c:56,388-403`).
    assert_eq!(experience.as_c4_int(), Some(100 - 66));
}
