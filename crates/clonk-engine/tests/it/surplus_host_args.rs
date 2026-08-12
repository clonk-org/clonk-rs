use clonk_engine::{Definition, Engine, Landscape, PlayerConfig, SpawnConfig, Vector2};
use clonk_script::Value;

#[test]
fn listed_native_functions_evaluate_and_ignore_surplus_arguments() {
    let source = r#"#strict 3
static Marks;

func Mark()
{
    Marks = Marks + 1;
    return "surplus is not typechecked";
}

func Probe(player)
{
    Marks = 0;
    SetTemperature(37);

    GetTemperature(Mark());
    GetWealth(player, Mark());
    GetViewCursor(player, Mark());
    Hostile(player, 2, false, Mark());
    GetWind(0, 0, true, Mark());
    GetVertexNum(nil, Mark());
    GetValue(nil, nil, nil, player, Mark());
    IncinerateLandscape(0, 0, Mark());
    Incinerate(nil, Mark());
    Extinguish(nil, Mark());

    return [GetTemperature(), GetWealth(player), Marks];
}
"#;

    let mut engine = Engine::with_seed(0);
    crate::support::TestValueExt::test_value(
        engine.register_player(PlayerConfig::new(1, "Caller").with_wealth(75)),
    );
    crate::support::TestValueExt::test_value(
        engine.register_player(PlayerConfig::new(2, "Opponent")),
    );
    engine.set_landscape(Landscape::flat(20, 10));

    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "XARG",
        "Surplus argument probe",
        source,
    ));
    definition.set_value(12);
    crate::support::TestValueExt::test_value(engine.register_definition(definition));
    let object = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("XARG").with_position(Vector2::new(5, 5))),
    );
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(object));

    assert_eq!(
        engine
            .call_object_function(index, "Probe", vec![Value::Int(1)])
            .expect("surplus arguments are discarded after evaluation"),
        Value::Array(vec![Value::Int(37), Value::Int(75), Value::Int(10)])
    );
}
