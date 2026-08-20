use super::*;

trait TestEngineExt {
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str);
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
}

impl TestEngineExt for Engine {
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str) {
        crate::TestValueExt::test_value(self.register_script_definition(id, name, script));
    }

    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
        crate::TestValueExt::test_value(self.spawn_object(config))
    }
}

/// The gather order requires a clear path **both ways**, and each half is
/// pinned separately (clonk-org/clonk-rs#334).
///
/// Nothing in CI executes content script, so the shipped
/// `planet/System.c4g/GatherTask.c` is compiled and driven here rather than
/// trusted. Both halves matter and a single fixture cannot show it: an item
/// beyond the wall is already excluded by the outward check, so a return-path
/// bug hides behind it. The two cases below fail independently — deleting
/// either `PathFree` in the script turns exactly one of them red.
#[test]
fn the_gather_order_requires_a_clear_path_both_ways() {
    let mut engine = Engine::new();
    let sources = vec![(
        "GatherTask.c".to_owned(),
        include_str!("../../../../planet/System.c4g/GatherTask.c").to_owned(),
    )];
    assert_eq!(engine.install_global_scripts(&sources), 1);
    engine.resolve_appends();

    // Open ground with one solid column at x = 32, floor to ceiling.
    let mut landscape =
        crate::Landscape::with_default_material(64, vec![40; 64], None).expect("test landscape");
    landscape.set_world_height(40);
    let mut bytes = vec![0_u8; 64 * 40];
    for row in 0..40 {
        bytes[row * 64 + 32] = 1;
    }
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        64,
        40,
        bytes,
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);

    engine.register_test_script_definition("GOLD", "Nugget", "");
    engine.register_test_script_definition("CLNK", "Clonk", "");
    engine.register_test_script_definition("BASE", "Base", "");

    // Clonk and one nugget on the left of the wall; a second nugget beyond it.
    let clonk =
        engine.spawn_test_object(SpawnConfig::new("CLNK").with_position(Vector2::new(10, 20)));
    let near =
        engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(20, 20)));
    let _beyond =
        engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(50, 20)));

    let candidates = |engine: &mut Engine, base: Value| {
        let arguments = vec![
            Value::Object(clonk.as_u64()),
            Value::C4Id("GOLD".to_owned()),
            base,
        ];
        let value = crate::TestValueExt::test_value(
            engine.call_engine_global_function("ClonkRsGatherCandidates", &arguments),
        );
        let Value::Array(items) = value else {
            panic!("candidates must be an array, got {value:?}");
        };
        items
    };

    // Outward path: with no base to return to, only the reachable nugget is
    // offered. The one beyond the wall fails `PathFree` from the Clonk.
    let reachable = candidates(&mut engine, Value::Nil);
    assert_eq!(
        reachable.len(),
        1,
        "the nugget beyond the wall is not reachable"
    );
    assert_eq!(reachable[0], Value::Object(near.as_u64()));

    // Return path: the same nugget, now with a base on the far side of the
    // wall. The Clonk can still walk to it, but could not carry it home, so it
    // drops out — this is the half the outward check cannot cover.
    let across =
        engine.spawn_test_object(SpawnConfig::new("BASE").with_position(Vector2::new(50, 20)));
    let stranded = candidates(&mut engine, Value::Object(across.as_u64()));
    assert!(
        stranded.is_empty(),
        "an item the Clonk cannot carry home must not be offered, got {stranded:?}"
    );

    // And with a base it *can* reach, the same nugget is offered again, so the
    // exclusion above is the return path rather than the base merely existing.
    let home =
        engine.spawn_test_object(SpawnConfig::new("BASE").with_position(Vector2::new(4, 20)));
    let ordered = crate::TestValueExt::test_value(engine.call_engine_global_function(
        "ClonkRsGatherOrder",
        &[
            Value::Object(clonk.as_u64()),
            Value::C4Id("GOLD".to_owned()),
            Value::Object(home.as_u64()),
        ],
    ));
    assert_eq!(
        ordered,
        Value::Int(1),
        "one order for the one fetchable nugget"
    );
}
