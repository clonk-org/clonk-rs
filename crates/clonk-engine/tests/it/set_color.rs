use crate::support::EngineTestExt;
use clonk_engine::{Engine, SpawnConfig};
use clonk_script::Value;

#[test]
fn set_color_maps_legacy_palette_indices_and_rejects_out_of_range_values() {
    let script = r#"#strict 3
func Apply(int color)
{
    return [SetColor(color), GetColorDw()];
}

func ApplyWithNilTarget(int color)
{
    return [SetColor(color, nil), GetColorDw()];
}
"#;
    let mut engine = Engine::new();
    engine.register_test_script_definition("SCLR", "SetColor probe", script);
    let probe = engine.spawn_test_object(SpawnConfig::new("SCLR"));
    let probe_index = engine.test_object_index(probe);

    for (color_index, expected) in [
        0x0000e8, 0xf40000, 0x00c800, 0xfcf41c, 0xdc9850, 0x784830, 0xb05000, 0xfcb490, 0x747474,
        0xe8e8e8, 0x30c4fc, 0xbc00c0,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            engine.call_test_object_function(
                probe_index,
                "Apply",
                vec![Value::Int(color_index as i32)]
            ),
            Value::Array(vec![Value::Int(1), Value::Int(expected)]),
            "legacy color index {color_index}"
        );
    }

    for invalid in [-1, 12, i32::MAX] {
        assert_eq!(
            engine.call_test_object_function(probe_index, "Apply", vec![Value::Int(invalid)]),
            Value::Array(vec![Value::Int(0), Value::Int(0xbc00c0)]),
            "out-of-range index {invalid} leaves the previous color unchanged"
        );
    }

    assert_eq!(
        engine.call_test_object_function(probe_index, "ApplyWithNilTarget", vec![Value::Int(1)],),
        Value::Array(vec![Value::Int(1), Value::Int(0xf40000)])
    );
    assert_eq!(engine.test_object_snapshot(probe).color, 0xf40000);
}

#[test]
fn set_color_updates_an_explicit_foreign_target() {
    let script = r#"#strict 3
func ApplyTo(int color, object target)
{
    return [SetColor(color, target), GetColorDw()];
}
"#;
    let mut engine = Engine::new();
    engine.register_test_script_definition("SCLR", "SetColor probe", script);
    let caller = engine.spawn_test_object(SpawnConfig::new("SCLR"));
    let target = engine.spawn_test_object(SpawnConfig::new("SCLR"));
    let caller_index = engine.test_object_index(caller);

    assert_eq!(
        engine.call_test_object_function(
            caller_index,
            "ApplyTo",
            vec![Value::Int(3), Value::Object(target.as_u64())],
        ),
        Value::Array(vec![Value::Int(1), Value::Int(0)])
    );
    assert_eq!(engine.test_object_snapshot(target).color, 0xfcf41c);
    assert_eq!(engine.test_object_snapshot(caller).color, 0);
}
