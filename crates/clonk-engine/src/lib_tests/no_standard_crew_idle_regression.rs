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
    engine
        .register_definition(
            Definition::from_script("STND", "Standard crew", "")
                .expect("standard definition compiles"),
        )
        .expect("standard definition registers");
    let mut excluded =
        Definition::from_script("SPEC", "Special crew", "").expect("special definition compiles");
    excluded.no_standard_crew = -2;
    engine
        .register_definition(excluded)
        .expect("special definition registers");

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

    let (standard_index, _) = engine
        .recruit_crew_info(4, "")
        .expect("empty id recruits standard crew");
    assert_eq!(standard_index, 0);
    assert!(engine.crew_rosters[&4][0].in_action);
    assert!(!engine.crew_rosters[&4][1].in_action);

    let (special_index, _) = engine
        .recruit_crew_info(4, "SPEC")
        .expect("explicit id recruits special crew");
    assert_eq!(special_index, 1);
    assert!(engine.crew_rosters[&4][1].in_action);
}
