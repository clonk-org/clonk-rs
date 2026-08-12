use super::*;

fn idle_crew(id: &str, name: &str, experience: i32) -> player_file::CrewInfo {
    player_file::CrewInfo {
        id: id.to_string(),
        name: name.to_string(),
        rank_name: "Clonk".to_string(),
        experience,
        ..Default::default()
    }
}

#[test]
fn empty_id_get_idle_excludes_no_standard_crew_definitions() {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition("STND", "Standard crew", ""));
    let mut excluded = test_definition("SPEC", "Special crew", "");
    excluded.no_standard_crew = -2;
    crate::TestValueExt::test_value(engine.register_definition(excluded));

    engine.crew_rosters.insert(
        4,
        vec![
            idle_crew("STND", "Standard", 100),
            idle_crew("SPEC", "Special", 900),
        ],
    );
    engine.crew_info_order.insert(4, vec![1, 0]);

    assert_eq!(engine.idle_crew_info_index(4, ""), Some(0));
    assert_eq!(engine.idle_crew_info_index(4, "SPEC"), Some(1));

    let (standard_index, _) = crate::TestValueExt::test_value(engine.recruit_crew_info(4, ""));
    assert_eq!(standard_index, 0);
    assert!(engine.crew_rosters[&4][0].in_action);
    assert!(!engine.crew_rosters[&4][1].in_action);

    let (special_index, _) = crate::TestValueExt::test_value(engine.recruit_crew_info(4, "SPEC"));
    assert_eq!(special_index, 1);
    assert!(engine.crew_rosters[&4][1].in_action);
}
