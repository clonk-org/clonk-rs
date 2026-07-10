use std::env;
use std::path::PathBuf;

use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{
    ocf, CommandDirection, Definition, DefinitionTargetRect, Direction, Engine, JoinPlayerConfig,
    Landscape, ObjectUpdate, Scenario, ScenarioError, SpawnConfig, Vector2, CATEGORY_STATIC_BACK,
    CNAT_TOP, COM_UP,
};
use lc_resources::Group;

struct ContentResolver {
    root: PathBuf,
}

impl LegacyDefinitionResolver for ContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let path = self.root.join(identifier.replace('\\', "/"));
        Group::open(path)
            .map(|group| vec![group])
            .map_err(ScenarioError::Resources)
    }
}

fn content_root() -> PathBuf {
    env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../content"))
}

#[test]
fn tutorial_hut_keeps_its_defcore_entrance_for_up_control() {
    // HUT2's DefCore Entrance=-18,8,16,17 must reach SetOCF's full-Con
    // OCF_Entrance bit (C4Object.cpp:586-589). ObjectComUp at the real
    // entrance then queues Enter before considering Jump
    // (C4ObjectCom.cpp:335-348).
    let content = content_root();
    let tutorial = content.join("Tutorial.c4f/Tutorial01.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&tutorial, &resolver)
        .expect("Tutorial01 and the real Objects.c4d load");
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine).expect("Tutorial01 applies");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Entrance tester".to_string(),
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            startup_player_count: 1,
        })
        .expect("Tutorial01 player joins");
    let clonk = engine
        .crew_cursor(joined.number)
        .expect("Tutorial01 joins one selected CLNK");
    let hut = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id.as_str() == "HUT2")
        .expect("Tutorial01 creates HUT2");

    assert_ne!(
        hut.ocf & ocf::ENTRANCE,
        0,
        "full-con HUT2 exposes its DefCore entrance"
    );

    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(570, 170))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .with_command_direction(CommandDirection::Stop),
        )
        .expect("place CLNK in the real hut entrance");
    engine
        .player_in_com(joined.number, COM_UP, 0)
        .expect("normal player Up control");

    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK after input")
            .command_stack
            .command_names(),
        vec!["Enter".to_string()]
    );
}

#[test]
fn tutorial_clonk_jumps_into_a_ceiling_and_hangles_like_cpp() {
    // C4PhysicalInfo::PromotionUpdate enables CanHangle for every ranked
    // crew member (C4InfoCore.cpp:207-213). A low-speed DFA_FLIGHT contact
    // through the CLNK top vertex then enters Hangle without changing its
    // facing (C4Object.cpp:4369-4404; C4ObjectCom.cpp:112-118).
    let content = content_root();
    let tutorial = content.join("Tutorial.c4f/Tutorial01.c4s");
    assert!(
        tutorial.is_dir(),
        "Tutorial01 content is required at {}; set LC_CONTENT_ROOT for an isolated worktree",
        content.display()
    );

    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&tutorial, &resolver)
        .expect("Tutorial01 and the real Objects.c4d load");
    let mut engine = Engine::with_seed(0);
    scenario
        .apply_before_players(&mut engine)
        .expect("Tutorial01 definitions apply");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Ceiling tester".to_string(),
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            startup_player_count: 1,
        })
        .expect("Tutorial01 player joins");
    let clonk = engine
        .crew_cursor(joined.number)
        .expect("Tutorial01 joins one selected CLNK");

    let loaded = engine.object_snapshot(clonk).expect("joined CLNK exists");
    assert_eq!(loaded.definition_id.as_str(), "CLNK");
    assert!(
        loaded
            .vertices
            .iter()
            .any(|vertex| vertex.x == 0 && vertex.y == -7 && vertex.cnat & CNAT_TOP != 0),
        "the real CLNK DefCore top-contact vertex must survive loading"
    );

    // The real CLNK stands at (30,20): its bottom vertex is y=29 beside
    // the floor at y=30, while its top vertex starts below the y=5 ceiling.
    // The gap guarantees one observable Jump/FLIGHT frame before contact.
    engine.set_landscape(Landscape::flat(60, 30));
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(30, 20))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Stop),
        )
        .expect("place the real CLNK in the fixture");
    let mut ceiling = Definition::from_script("TSTC", "Test ceiling", "#strict\n")
        .expect("ceiling definition compiles");
    ceiling.set_category(CATEGORY_STATIC_BACK);
    ceiling.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 60, 1, 0, 0)));
    engine
        .register_definition(ceiling)
        .expect("ceiling definition registers");
    engine
        .spawn_object(SpawnConfig::new("TSTC").with_position(Vector2::new(0, 5)))
        .expect("ceiling solid mask spawns");

    engine
        .player_in_com(joined.number, COM_UP, 0)
        .expect("normal player Up control");
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK after input")
            .command_stack
            .command_names(),
        vec!["Jump".to_string()],
        "COM_Up must take the normal WALK -> Jump command path"
    );

    engine.tick().expect("first jump frame");
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK in flight")
            .action
            .name,
        "Jump",
        "the real action map must reach DFA_FLIGHT before ceiling contact"
    );

    for _ in 0..4 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            break;
        }
        engine.tick().expect("ceiling approach frame");
    }
    let hangle = engine.object_snapshot(clonk).expect("CLNK after contact");
    assert_eq!(hangle.action.name, "Hangle");
    assert_eq!(hangle.direction, Direction::Right);
    assert_eq!(hangle.velocity, Vector2::ZERO);
}

#[test]
fn tutorial_clonk_flight_keeps_accelerating_past_twelve_pixels_per_tick() {
    // DFA_FLIGHT calls DoGravity every frame (C4Object.cpp:4893-4904), whose
    // free-fall branch only adds GravAccel (C4Object.cpp:4672-4674). C++ has
    // no generic terminal-velocity clamp, so a Clonk falling through enough
    // open space must accelerate past 12 px/tick.
    let content = content_root();
    let tutorial = content.join("Tutorial.c4f/Tutorial01.c4s");
    assert!(
        tutorial.is_dir(),
        "Tutorial01 content is required at {}; set LC_CONTENT_ROOT for an isolated worktree",
        content.display()
    );

    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&tutorial, &resolver)
        .expect("Tutorial01 and the real Objects.c4d load");
    let mut engine = Engine::with_seed(0);
    scenario
        .apply_before_players(&mut engine)
        .expect("Tutorial01 definitions apply");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Flight tester".to_string(),
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            startup_player_count: 1,
        })
        .expect("Tutorial01 player joins");
    let clonk = engine
        .crew_cursor(joined.number)
        .expect("Tutorial01 joins one selected CLNK");

    engine.set_landscape(Landscape::flat(80, 400));
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(40, 100))
                .with_velocity(Vector2::new(0, 11))
                .with_action("Jump")
                .with_command_direction(CommandDirection::Stop),
        )
        .expect("place the real CLNK in open flight");

    for _ in 0..9 {
        engine.tick().expect("open flight frame");
    }

    let falling = engine.object_snapshot(clonk).expect("CLNK after flight");
    assert_eq!(falling.action.name, "Jump");
    assert_eq!(falling.command_direction, CommandDirection::Stop);
    assert_eq!(
        falling.velocity.y, 13,
        "eleven plus nine 0.2px gravity steps rounds to 13 without a C++-foreign terminal clamp"
    );
    assert_eq!(
        falling.position.y, 208,
        "the unbounded fixed-point velocities must also drive the C++ flight trajectory"
    );
}
