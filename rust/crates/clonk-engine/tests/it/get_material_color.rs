use clonk_engine::{Definition, SpawnConfig};
use clonk_script::Value;

use crate::support::real_scenario::PreparedInstalledScenario;

pub(super) fn get_material_color_reads_earth_palette_and_system_rgb_wrapper(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert!(engine.debug_global_has_function("GetMaterialColorX"));
    engine
        .register_definition(
            Definition::from_script(
                "MCOL",
                "Material color probe",
                r#"#strict
local result;

public func Probe()
{
    var earth = Material("Earth");
    var ore = Material("Ore");
    var brick = Material("Brick");
    result = [
        GetMaterialColor(earth, 0, 0),
        GetMaterialColor(earth, 0, 1),
        GetMaterialColor(earth, 0, 2),
        GetMaterialColor(-1, 0, 0),
        GetMaterialColorX(earth, 0),
        GetMaterialColor(ore, 0, 0),
        GetMaterialColor(brick, 1, 0),
        GetMaterialColor(earth, 3, 0),
        GetMaterialColor(earth, 0, 3)
    ];
    return(result);
}
"#,
            )
            .expect("the material color probe compiles"),
        )
        .expect("the material color probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("MCOL"))
        .expect("the material color probe spawns");
    let index = engine.find_object_index(probe).expect("the probe exists");

    let expected = Value::Array(vec![
        Value::Int(127),
        Value::Int(95),
        Value::Int(63),
        Value::Nil,
        Value::Int(0x007f_5f3f),
        Value::Int(114),
        Value::Int(0),
        Value::Nil,
        Value::Int(147),
    ]);
    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("GetMaterialColor and GetMaterialColorX execute"),
        expected
    );
    assert_eq!(
        engine
            .object_snapshot(probe)
            .expect("the probe remains active")
            .local_vars
            .get("result"),
        Some(&expected)
    );
}
