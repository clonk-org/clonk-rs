use super::*;
use std::path::PathBuf;

#[test]
fn resource_rank_extension_errors_remain_deferred_through_engine_definition() {
    let mut packed = clonk_resources::MutableGroup::new("DeferredRank.c4d");
    packed
        .add_file("DefCore.txt", b"[DefCore]\nid=DRNK\n".to_vec())
        .expect("add DefCore");
    packed
        .add_file("RankUS.txt", b"Recruit\r\n*Wrong %d\r\n".to_vec())
        .expect("add rank table");
    let group = clonk_resources::Group::from_memory(
        PathBuf::from("DeferredRank.c4d"),
        packed.pack().expect("pack definition"),
    )
    .expect("open packed definition");
    let resource = ResourceDefinitionData::load_with_languages(&group, &["US"])
        .expect("resource loading must not validate unused extensions");
    let compiled = Definition::from_resource(&resource)
        .expect("engine definition construction must preserve the lazy table");
    let mut engine = Engine::new();
    engine
        .register_definition(compiled)
        .expect("definition with dormant invalid extension registers");
    let _ = engine.host_definition_tables();

    let names = engine
        .definitions
        .get("DRNK")
        .and_then(Definition::rank_names)
        .expect("registered rank table");
    assert_eq!(names.get(0).as_deref(), Some("Recruit"));
    assert!(
        std::panic::catch_unwind(|| names.get(1)).is_err(),
            "requesting the malformed extended rank preserves native's uncaught boundary"
    );
}
