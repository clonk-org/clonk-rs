use super::*;

#[test]
fn initial_component_gain_scales_the_raw_definition_count_once() {
        // ComponentConGain reads the raw Def->Component count and applies
        // Con exactly once (C4Object.cpp:519-526). This is observable when
        // Construction changes a freshly initialized zero-count entry before
        // NewObject's partial initial DoCon.
    let mut engine = Engine::new();
    let mut definition =
        Definition::from_script("PART", "Partial", "#strict\n").expect("definition compiles");
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
    engine
        .register_definition(definition)
        .expect("definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("PART").with_construction(0))
        .expect("zero-con object spawns");
    let index = engine.find_object_index(object).expect("object exists");

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

    let partial = engine
        .spawn_object(SpawnConfig::new("PART").with_construction(FULL_CON / 2))
        .expect("partial-con object spawns");
    let partial = engine
        .object_snapshot(partial)
        .expect("partial object exists");
    assert_eq!(partial.components.get("ROCK"), Some(&2));
    assert_eq!(
        partial.components.get("NEGA"),
        Some(&-1),
            "fresh partial Con includes the initial ComponentConGain"
    );

    let below_first_step = engine
        .spawn_object(SpawnConfig::new("PART").with_construction(FULL_CON / 100 - 1))
        .expect("sub-step object spawns");
    let below_first_step = engine
        .object_snapshot(below_first_step)
        .expect("sub-step object exists");
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
    engine
        .register_definition(
            Definition::from_script("ZERO", "Zero", "").expect("component compiles"),
        )
        .expect("component registers");
    engine
        .register_definition(
            Definition::from_script("BLDR", "Builder", "").expect("builder compiles"),
        )
        .expect("builder registers");
    let mut target = Definition::from_script("SITE", "Site", "").expect("site compiles");
    target.set_components(vec![DefinitionComponent {
        id: "ZERO".to_owned(),
        count: 0,
    }]);
    engine.register_definition(target).expect("site registers");

    let builder = engine
        .spawn_object(SpawnConfig::new("BLDR"))
        .expect("builder spawns");
    let site = engine
        .spawn_object(
            SpawnConfig::new("SITE").with_ordered_components(vec![("ZERO".to_owned(), -1)]),
        )
        .expect("site spawns");
    engine
        .spawn_object(SpawnConfig::new("ZERO").with_container(builder))
        .expect("material spawns");
    let builder_idx = engine.find_object_index(builder).expect("builder exists");
    let site_idx = engine.find_object_index(site).expect("site exists");
    let required = engine
        .definitions
        .get("SITE")
        .expect("site definition exists")
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
