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
    engine
        .register_script_definition("SCLR", "SetColor probe", script)
        .expect("SetColor probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("SCLR"))
        .expect("SetColor probe spawns");
    let probe_index = engine
        .find_object_index(probe)
        .expect("SetColor probe remains live");

    for (color_index, expected) in [
        0x0000e8, 0xf40000, 0x00c800, 0xfcf41c, 0xdc9850, 0x784830, 0xb05000, 0xfcb490, 0x747474,
        0xe8e8e8, 0x30c4fc, 0xbc00c0,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            engine
                .call_object_function(probe_index, "Apply", vec![Value::Int(color_index as i32)])
                .expect("SetColor executes"),
            Value::Array(vec![Value::Int(1), Value::Int(expected)]),
            "legacy color index {color_index}"
        );
    }

    for invalid in [-1, 12, i32::MAX] {
        assert_eq!(
            engine
                .call_object_function(probe_index, "Apply", vec![Value::Int(invalid)])
                .expect("out-of-range SetColor executes"),
            Value::Array(vec![Value::Int(0), Value::Int(0xbc00c0)]),
            "out-of-range index {invalid} leaves the previous color unchanged"
        );
    }

    assert_eq!(
        engine
            .call_object_function(probe_index, "ApplyWithNilTarget", vec![Value::Int(1)],)
            .expect("nil target falls back to the calling object"),
        Value::Array(vec![Value::Int(1), Value::Int(0xf40000)])
    );
    assert_eq!(
        engine
            .object_snapshot(probe)
            .expect("SetColor probe remains live")
            .color,
        0xf40000
    );
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
    engine
        .register_script_definition("SCLR", "SetColor probe", script)
        .expect("SetColor probe registers");
    let caller = engine
        .spawn_object(SpawnConfig::new("SCLR"))
        .expect("SetColor caller spawns");
    let target = engine
        .spawn_object(SpawnConfig::new("SCLR"))
        .expect("SetColor target spawns");
    let caller_index = engine
        .find_object_index(caller)
        .expect("SetColor caller remains live");

    assert_eq!(
        engine
            .call_object_function(
                caller_index,
                "ApplyTo",
                vec![Value::Int(3), Value::Object(target.as_u64())],
            )
            .expect("explicit-target SetColor executes"),
        Value::Array(vec![Value::Int(1), Value::Int(0)])
    );
    assert_eq!(
        engine
            .object_snapshot(target)
            .expect("SetColor target remains live")
            .color,
        0xfcf41c
    );
    assert_eq!(
        engine
            .object_snapshot(caller)
            .expect("SetColor caller remains live")
            .color,
        0
    );
}
