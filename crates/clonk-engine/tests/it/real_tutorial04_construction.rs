use std::env;
use std::path::PathBuf;

use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::{
    Engine, JoinPlayerConfig, ObjectMenuComponent, ObjectMenuExtra, ObjectUpdate, Scenario,
    ScenarioError, COM_DIG,
};
use clonk_resources::Group;
use clonk_script::Value;

struct ContentResolver {
    root: PathBuf,
}

impl LegacyDefinitionResolver for ContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        Group::open(self.root.join(identifier.replace('\\', "/")))
            .map(|group| vec![group])
            .map_err(ScenarioError::Resources)
    }
}

fn load_tutorial04() -> (Engine, i32) {
    let content = env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content"));
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario =
        Scenario::load_from_path_with(content.join("Tutorial.c4f/Tutorial04.c4s"), &resolver)
            .expect("Tutorial04 loads");
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine).expect("Tutorial04 applies");
    let player = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial04 construction".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("Tutorial04 player joins");
    (engine, player.number())
}

#[test]
fn tutorial04_conkit_opens_the_real_elevator_construction_menu() {
    // CNKT::Activate creates CXCN on its containing clonk and fills it from
    // GetPlrKnowledge (Objects/.../Conkit.c4d/Script.c:5-21). Classic double
    // Dig activates the first carried object (C4ObjectCom.cpp:531-539).
    let (mut engine, player) = load_tutorial04();
    let clonk = engine
        .crew_cursor(player)
        .expect("Tutorial04 joins one selected CLNK");
    let conkit = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "CNKT")
        .expect("Tutorial04 supplies a construction kit")
        .id;

    for _ in 0..160 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        {
            break;
        }
        engine
            .tick_without_snapshot()
            .expect("ready crew Exit frame");
    }
    engine
        .apply_object_update(conkit, ObjectUpdate::new().with_container(clonk))
        .expect("take the real tutorial construction kit");

    engine
        .player_in_com(player, COM_DIG, 0)
        .expect("first Dig press");
    engine
        .player_in_com(player, COM_DIG, 0)
        .expect("second Dig press");

    let clonk_after_activation = engine.object_snapshot(clonk).expect("CLNK survives");
    let conkit_after_activation = engine.object_snapshot(conkit).expect("CNKT survives");
    let (_, menu) = engine.cursor_object_menu(player).unwrap_or_else(|| {
        panic!(
            "CNKT activation opens its script menu; clonk={clonk_after_activation:?}; \
             conkit={conkit_after_activation:?}"
        )
    });
    assert_eq!(menu.identification, Value::C4Id("CXCN".to_string()));
    assert_eq!(menu.symbol_id, "CXCN");
    assert_eq!(menu.style, 0, "C4MN_Style_Normal");
    assert!(!menu.permanent);
    assert_eq!(menu.command_object, Some(conkit));
    assert_eq!(menu.columns, 5);
    assert_eq!(
        menu.extra,
        ObjectMenuExtra::Components,
        "CNKT passes C4MN_Extra_Components to CreateMenu (C4Script.cpp:1420-1448)"
    );
    assert_eq!(menu.items.len(), 1, "Tutorial04 knows only ELEV");
    assert_eq!(menu.items[0].item_id, "ELEV");
    assert_eq!(menu.items[0].caption, "Construction: Elevator");
    assert_eq!(
        menu.items[0].info_caption,
        "The elevator will automatically move to pick up waiting clonks, but may also be switched to permanent movement.",
        "AddMenuItem falls back to ELEV's localized description (C4Script.cpp:1590-1594)"
    );
    assert_eq!(
        menu.items[0].components,
        vec![
            ObjectMenuComponent {
                definition_id: "WOOD".to_string(),
                count: 4,
            },
            ObjectMenuComponent {
                definition_id: "METL".to_string(),
                count: 2,
            },
        ],
        "C4MenuItem caches ELEV's ordered components at AddMenuItem time (C4Menu.cpp:92-97; C4Def.cpp:1322-1355)"
    );
    assert_eq!(menu.selection, 0);
}
