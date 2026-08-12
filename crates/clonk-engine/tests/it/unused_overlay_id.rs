use clonk_engine::{
    Definition, Engine, GraphicsOverlayMode, ObjectGraphicsOverlay, ObjectUpdate, SpawnConfig,
};
use clonk_script::Value;

#[test]
fn get_unused_overlay_id_is_registered_and_matches_search_semantics() {
    let mut engine = Engine::new();
    crate::support::TestValueExt::test_value(engine.register_definition(
        crate::support::TestValueExt::test_value(Definition::from_script(
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
        )),
    ));

    let caller =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("OVLY")));
    let empty =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("OVLY")));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
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
        ),
    );

    let caller_index = crate::support::TestValueExt::test_value(engine.find_object_index(caller));
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
