use super::*;

#[test]
fn loaded_object_preserves_negative_raw_construction() {
    // C4Object::CompileFunc assigns Objects.txt Size directly to Con without
    // clamping it in the compiler tail (C4Object.cpp:2777,2858-2891).
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition(
        "NEGC",
        "Negative construction",
        "",
    ));

    let object = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("NEGC")
                .with_loaded(true)
                .with_construction(-1),
        ),
    );

    assert_eq!(
        engine
            .object_snapshot(object)
            .expect("loaded object remains live")
            .construction,
        -1
    );
}

#[test]
fn initial_component_gain_scales_the_raw_definition_count_once() {
    // ComponentConGain reads the raw Def->Component count and applies
    // Con exactly once (C4Object.cpp:519-526). This is observable when
    // Construction changes a freshly initialized zero-count entry before
    // NewObject's partial initial DoCon.
    let mut engine = Engine::new();
    let mut definition = test_definition("PART", "Partial", "#strict\n");
    definition.set_components(vec![
        DefinitionComponent {
            id: "ROCK".to_owned(),
            count: 4,
        },
        DefinitionComponent {
            id: "NEGA".to_owned(),
            count: -3,
        },
    ]);
    crate::TestValueExt::test_value(engine.register_definition(definition));
    let object = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("PART").with_construction(0)),
    );
    let index = crate::TestValueExt::test_value(engine.find_object_index(object));

    assert_eq!(
        engine
            .object_snapshot(object)
            .and_then(|snapshot| snapshot.components.get("NEGA").copied()),
        Some(-3),
        "initial ComponentConCutoff keeps min(-3, 0)"
    );

    engine.do_initial_con(index, FULL_CON / 2);

    assert_eq!(
        engine
            .object_snapshot(object)
            .and_then(|snapshot| snapshot.components.get("ROCK").copied()),
        Some(2)
    );
    assert_eq!(
        engine
            .object_snapshot(object)
            .and_then(|snapshot| snapshot.components.get("NEGA").copied()),
        Some(-1),
        "growth uses max(-3, trunc(-3 * 50%))"
    );

    let partial = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("PART").with_construction(FULL_CON / 2)),
    );
    let partial = crate::TestValueExt::test_value(engine.object_snapshot(partial));
    assert_eq!(partial.components.get("ROCK"), Some(&2));
    assert_eq!(
        partial.components.get("NEGA"),
        Some(&-1),
        "fresh partial Con includes the initial ComponentConGain"
    );

    let below_first_step = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("PART").with_construction(FULL_CON / 100 - 1)),
    );
    let below_first_step =
        crate::TestValueExt::test_value(engine.object_snapshot(below_first_step));
    assert_eq!(below_first_step.components.get("ROCK"), Some(&0));
    assert_eq!(
        below_first_step.components.get("NEGA"),
        Some(&-3),
        "initial DoCon does not refresh components below its first one-percent step"
    );
}

#[test]
fn zero_requirement_still_consumes_toward_a_negative_live_count() {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition("ZERO", "Zero", ""));
    crate::TestValueExt::test_value(engine.register_script_definition("BLDR", "Builder", ""));
    let mut target = test_definition("SITE", "Site", "");
    target.set_components(vec![DefinitionComponent {
        id: "ZERO".to_owned(),
        count: 0,
    }]);
    crate::TestValueExt::test_value(engine.register_definition(target));

    let builder = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("BLDR")));
    let site = crate::TestValueExt::test_value(engine.spawn_object(
        SpawnConfig::new("SITE").with_ordered_components(vec![("ZERO".to_owned(), -1)]),
    ));
    crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("ZERO").with_container(builder)),
    );
    let builder_idx = crate::TestValueExt::test_value(engine.find_object_index(builder));
    let site_idx = crate::TestValueExt::test_value(engine.find_object_index(site));
    let required = crate::TestValueExt::test_value(engine.definitions.get("SITE"))
        .components()
        .to_vec();

    assert_eq!(
        engine
            .ensure_build_components(builder_idx, site_idx, &required)
            .expect("component pass succeeds"),
        None
    );
    assert_eq!(
        engine
            .object_snapshot(site)
            .and_then(|snapshot| snapshot.components.get("ZERO").copied()),
        Some(0)
    );
    assert!(engine
        .object_snapshot(builder)
        .expect("builder remains")
        .contents
        .is_empty());
}
