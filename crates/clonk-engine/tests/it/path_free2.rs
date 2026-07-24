use clonk_engine::landscape::PixelGrid;
use clonk_engine::{Definition, Engine, EngineError, Landscape, SpawnConfig};
use clonk_script::Value;

fn call_probe_result(script: &str, landscape: Landscape) -> Result<Value, EngineError> {
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(landscape);
    engine
        .register_definition(
            Definition::from_script("PF2T", "PathFree2 probe", script)
                .expect("probe script compiles"),
        )
        .expect("probe definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("PF2T"))
        .expect("probe object spawns");
    let index = engine.find_object_index(object).expect("probe exists");
    engine.call_object_function(index, "Probe", Vec::new())
}

fn raster_landscape(width: u32, height: u32, pixel: impl Fn(i32, i32) -> u8) -> Landscape {
    let mut bytes = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            bytes.push(pixel(x, y));
        }
    }
    let grid = PixelGrid::new(
        width,
        height,
        bytes,
        vec![0, 100],
        vec![None; 2],
        vec![None; 2],
    );
    let mut landscape = Landscape::flat(width, height as i32);
    landscape.set_pixel_grid(grid);
    landscape.set_world_height(height as i32);
    landscape
}

#[test]
fn path_free2_preserves_clear_refs_and_writes_the_cpp_canonical_blocker() {
    let script = r#"#strict 3
global func Probe()
{
    var clear_x = 1, clear_y = 0;
    var clear = PathFree2(clear_x, clear_y, 9, 0);

    var forward_x = 1, forward_y = 1;
    var forward = PathFree2(forward_x, forward_y, 9, 5);

    var reverse_x = 9, reverse_y = 5;
    var reverse = PathFree2(reverse_x, reverse_y, 1, 1);

    var alias = 1;
    var aliased = PathFree2(alias, alias, 9, 5);
    return [clear, clear_x, clear_y,
            forward, forward_x, forward_y,
            reverse, reverse_x, reverse_y,
            aliased, alias];
}
"#;
    // ForLine normalizes this x-major segment to (1,1)->(9,5). The caller-
    // reversed path therefore hits (3,2), not the nearer-from-caller (7,4).
    let landscape = raster_landscape(12, 8, |x, y| u8::from((x, y) == (3, 2) || (x, y) == (7, 4)));

    assert_eq!(
        call_probe_result(script, landscape).expect("PathFree2 probe runs"),
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(1),
            Value::Int(0),
            Value::Bool(false),
            Value::Int(3),
            Value::Int(2),
            Value::Bool(false),
            Value::Int(3),
            Value::Int(2),
            Value::Bool(false),
            // C++ assigns X first and Y second, so aliased refs retain Y.
            Value::Int(2),
        ])
    );
}

#[test]
fn path_free2_rejects_non_lvalue_reference_slots_without_crashing() {
    for script in [
        r#"#strict 3
        global func Probe()
        {
            var y = 0;
            return PathFree2(0, y, 2, 2);
        }
        "#,
        r#"#strict 3
        global func Probe()
        {
            var x = 0;
            return PathFree2(x, 0, 2, 2);
        }
        "#,
    ] {
        let error = call_probe_result(script, raster_landscape(4, 4, |_, _| 0))
            .expect_err("C++ native reference parameters reject non-lvalues");
        assert!(
            matches!(error, EngineError::Script { .. }),
            "unexpected PathFree2 argument error: {error:?}"
        );
    }
}

#[test]
fn path_free2_reads_unconvertible_referents_as_zero_before_writeback() {
    let script = r#"#strict 3
global func Probe()
{
    var x = "not an integer", y = true;
    var result = PathFree2(x, y, 2, 1);
    return [result, x, y];
}
"#;
    let landscape = raster_landscape(4, 4, |x, y| u8::from((x, y) == (0, 1)));

    assert_eq!(
        call_probe_result(script, landscape).expect("PathFree2 coercion probe runs"),
        Value::Array(vec![Value::Bool(false), Value::Int(0), Value::Int(1)])
    );
}

#[test]
fn path_free2_native_integer_slots_apply_legacy_falsy_conversion() {
    fn script_engine(source: &str) -> clonk_script::Engine {
        let mut engine = clonk_script::Engine::new();
        clonk_engine::compat::register_host_functions(&mut engine);
        engine.register_host_function("ZeroId", |_| Ok(Value::C4Id("NONE".into())));
        engine.load_script(source).expect("probe script loads");
        engine
    }

    // Validate the former [result, x, y] tuple inside one NONSTRICT call;
    // array syntax itself is unavailable below STRICT 1.
    let nonstrict = r#"
global func Probe()
{
    var x = 1, y = 1;
    if (!PathFree2(x, y, ZeroId(), ZeroId())) return -1;
    if (x != 1) return -2;
    if (y != 1) return -3;
    return 1;
}
"#;
    let nonstrict = script_engine(nonstrict);
    assert_eq!(
        nonstrict
            .call("Probe", &[])
            .expect("nonstrict native conversion runs"),
        Value::Int(1)
    );

    let strict_three = r#"#strict 3
global func Probe()
{
    var x = 1, y = 0;
    return PathFree2(x, y, ZeroId(), ZeroId());
}
"#;
    assert_eq!(
        script_engine(strict_three)
            .call("Probe", &[])
            .expect("zero-argument AB_FUNC materializes each result"),
        Value::Bool(true),
        "AB_FUNC pushes a zero-argument native return through C4Value::Set into a fresh stack slot, so C4ID(0) is already canonical nil before PathFree2 converts its integer parameters"
    );
}
