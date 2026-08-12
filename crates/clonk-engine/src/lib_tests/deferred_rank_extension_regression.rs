use super::*;
use std::path::PathBuf;

#[test]
fn resource_rank_extension_errors_remain_deferred_through_engine_definition() {
    let mut packed = clonk_resources::MutableGroup::new("DeferredRank.c4d");
    crate::TestValueExt::test_value(
        packed.add_file("DefCore.txt", b"[DefCore]\nid=DRNK\n".to_vec()),
    );
    crate::TestValueExt::test_value(
        packed.add_file("RankUS.txt", b"Recruit\r\n*Wrong %d\r\n".to_vec()),
    );
    let group = crate::TestValueExt::test_value(clonk_resources::Group::from_memory(
        PathBuf::from("DeferredRank.c4d"),
        crate::TestValueExt::test_value(packed.pack()),
    ));
    let resource = crate::TestValueExt::test_value(ResourceDefinitionData::load_with_languages(
        &group,
        &["US"],
    ));
    let compiled = crate::TestValueExt::test_value(Definition::from_resource(&resource));
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_definition(compiled));
    let _ = engine.host_definition_tables();

    let names = crate::TestValueExt::test_value(
        engine
            .definitions
            .get("DRNK")
            .and_then(Definition::rank_names),
    );
    assert_eq!(names.get(0).as_deref(), Some("Recruit"));
    assert!(
        std::panic::catch_unwind(|| names.get(1)).is_err(),
        "requesting the malformed extended rank preserves native's uncaught boundary"
    );
}
