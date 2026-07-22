use clonk_engine::{
    Definition, Engine, GraphicsOverlayMode, ObjectGraphicsOverlay, ObjectUpdate, SpawnConfig,
};
use clonk_script::Value;

#[test]
fn get_unused_overlay_id_is_registered_and_matches_search_semantics() {
    let mut engine = Engine::new();
    engine
        .register_definition(
            Definition::from_script(
                "OVLY",
                "Overlay ID probe",
                r#"
                #strict

                public func Probe(object empty)
                {
                    return [
                        GetUnusedOverlayID(0, empty),
                        GetUnusedOverlayID(1, empty),
                        GetUnusedOverlayID(1, 0),
                        GetUnusedOverlayID(1),
                        GetUnusedOverlayID(-1)
                    ];
                }

                public func ProbeWithoutObjectContext(target_definition)
                {
                    return DefinitionCall(target_definition, "WithoutObjectContext");
                }

                func WithoutObjectContext()
                {
                    return GetUnusedOverlayID(1, 0);
                }
                "#,
            )
            .expect("overlay ID probe compiles"),
        )
        .expect("overlay ID probe registers");

    let caller = engine
        .spawn_object(SpawnConfig::new("OVLY"))
        .expect("caller spawns");
    let empty = engine
        .spawn_object(SpawnConfig::new("OVLY"))
        .expect("empty target spawns");
    engine
        .apply_object_update(
            caller,
            ObjectUpdate {
                graphics_overlays: Some(
                    [1, 2, -1, -2]
                        .into_iter()
                        .map(|id| ObjectGraphicsOverlay::new(id, GraphicsOverlayMode::Picture))
                        .collect(),
                ),
                ..ObjectUpdate::default()
            },
        )
        .expect("occupied overlay slots install");

    let caller_index = engine
        .find_object_index(caller)
        .expect("caller remains present");
    assert_eq!(
        engine
            .call_object_function(caller_index, "Probe", vec![Value::Object(empty.as_u64())],)
            .expect("GetUnusedOverlayID probe succeeds"),
        Value::Array(vec![
            Value::Nil,
            Value::Int(1),
            Value::Int(3),
            Value::Int(3),
            Value::Int(-3),
        ])
    );
    assert_eq!(
        engine
            .call_object_function(
                caller_index,
                "ProbeWithoutObjectContext",
                vec![Value::C4Id("OVLY".to_string())],
            )
            .expect("definition-context probe succeeds"),
        Value::Nil,
        "nil target without a calling object has no fallback object"
    );
}
