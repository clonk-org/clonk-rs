use clonk_engine::{Definition, SpawnConfig};
use clonk_script::Value;

use crate::support::real_scenario::PreparedInstalledScenario;

pub(super) fn set_material_color_matches_native_modulation_formula_and_invalid_gate(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    crate::support::TestValueExt::test_value(engine.register_definition(
        crate::support::TestValueExt::test_value(Definition::from_script(
            "SMCL",
            "SetMaterialColor probe",
            r#"#strict
                local result;

                public func Probe()
                {
                var earth = Material("Earth");
                var changed = SetMaterialColor(earth, 63, 190, 0, 1, 2, 3, 4, 5, 6);
                var changed_modulation = GetMatAdjust();
                var identity = SetMaterialColor(earth, 127, 95, 63);
                var identity_modulation = GetMatAdjust();
                var black = SetMaterialColor(earth, 0, 0, 0);
                var black_modulation = GetMatAdjust();
                var invalid = SetMaterialColor(-1, 255, 255, 255);
                var after_invalid = GetMatAdjust();
                result = [changed, changed_modulation,
              identity, identity_modulation,
              black, black_modulation,
              invalid, after_invalid];
                return(result);
                }
                "#,
        )),
    ));
    let probe =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("SMCL")));
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(probe));

    // Earth starts at Color=127,95,63. Native GetClrModulation therefore
    // computes min(target*256/max(source,1),255) per channel: 126,255,0.
    let expected = Value::Array(vec![
        Value::Bool(true),
        Value::Int(0x007e_ff00),
        Value::Bool(true),
        Value::Int(0),
        Value::Bool(true),
        Value::Int(1),
        Value::Bool(false),
        Value::Int(1),
    ]);
    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("SetMaterialColor executes"),
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
    assert_eq!(
        engine
            .snapshot()
            .landscape
            .expect("the probe landscape remains installed")
            .modulation(),
        1,
        "the invalid material leaves the preceding black sentinel unchanged"
    );
}
