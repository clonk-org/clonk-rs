use std::env;
use std::path::PathBuf;

use crate::support::real_scenario::load_installed_scenario;
use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::{
    ocf, CommandDirection, Definition, DefinitionTargetRect, Direction, Engine, JoinPlayerConfig,
    Landscape, ObjectUpdate, PhysicalsUpdate, Scenario, ScenarioError, SpawnConfig, Vector2,
    CATEGORY_STATIC_BACK, CNAT_TOP, COM_DIG, COM_LEFT, COM_RIGHT, COM_THROW, COM_UP,
};
use clonk_resources::Group;
use clonk_script::Value;

struct ContentResolver {
    root: PathBuf,
}

#[test]
fn tutorial01_real_clonk_subcases_batch() {
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
    let subcases: &[(&str, fn(&Scenario))] = &[
        (
            "clonk_dig_control_starts_the_real_dig_action_like_cpp",
            tutorial_clonk_dig_control_starts_the_real_dig_action_like_cpp,
        ),
        (
            "hut_keeps_its_defcore_entrance_for_up_control",
            tutorial_hut_keeps_its_defcore_entrance_for_up_control,
        ),
        (
            "flag_throw_assigns_base_and_unlocks_digging",
            tutorial_flag_throw_assigns_base_and_unlocks_digging,
        ),
        (
            "clonk_jumps_into_a_ceiling_and_hangles_like_cpp",
            tutorial_clonk_jumps_into_a_ceiling_and_hangles_like_cpp,
        ),
        (
            "clonk_flight_keeps_accelerating_past_twelve_pixels_per_tick",
            tutorial_clonk_flight_keeps_accelerating_past_twelve_pixels_per_tick,
        ),
    ];
    let mut failures = Vec::new();

    for &(name, subcase) in subcases {
        eprintln!("running shared Tutorial01 real-Clonk subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subcase(&scenario))).is_err() {
            eprintln!("shared Tutorial01 real-Clonk subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    assert!(
        failures.is_empty(),
        "Tutorial01 real-Clonk subcase(s) failed: {}",
        failures.join(", ")
    );
}

fn tutorial_clonk_dig_control_starts_the_real_dig_action_like_cpp(scenario: &Scenario) {
    // C4Player::Execute flushes a buffered COM_Dig as COM_Dig_S after
    // C4DoubleClick frames (C4Player.cpp:1215-1229). In DFA_WALK that calls
    // ObjectComDig and selects the real CLNK "Dig" action
    // (C4Object.cpp:3422-3434; C4ObjectCom.cpp:353-362).
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine).expect("Tutorial01 applies");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Dig tester".to_string(),
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
        .expect("Tutorial01 player joins")
        .initialized()
        .expect("Tutorial01 player initializes");
    let clonk = engine
        .crew_cursor(joined.number)
        .expect("Tutorial01 joins one selected CLNK");

    for _ in 0..100 {
        engine.tick_without_snapshot().expect("settling frame");
    }
    let settled = engine.object_snapshot(clonk).expect("settled CLNK");
    assert_eq!(
        settled.action.name, "Walk",
        "the tutorial CLNK must be walking before the dig control"
    );
    assert_eq!(
        settled
            .temporary_physical
            .expect("InitializePlayer installs temporary physicals")
            .can_dig,
        0,
        "Tutorial01 intentionally locks digging until Script160"
    );

    // Tutorial01 Script160 calls ResetPhysical(GetCrew()) before teaching
    // the dig key (Script.c:134-141). Model that exact milestone so this
    // regression tests COM_Dig after the intentional tutorial lock expires.
    let mut reset = ObjectUpdate::new();
    reset.physicals = Some(PhysicalsUpdate {
        info: settled.info_physical,
        temporary: None,
        changes: Vec::new(),
    });
    engine
        .apply_object_update(clonk, reset)
        .expect("Script160 ResetPhysical milestone");

    engine
        .player_in_com(joined.number, COM_DIG, 0)
        .expect("normal player Dig control");
    for _ in 0..=10 {
        engine
            .tick_without_snapshot()
            .expect("dig single-click timeout frame");
    }

    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK after Dig")
            .action
            .name,
        "Dig"
    );
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
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content"))
}

#[test]
fn tutorial03_auto_context_menu_reaches_buy_and_contents() {
    // Tutorial03 waits for C4MN_Context=14, C4MN_Buy=4, then
    // C4MN_Contents=18 (Script.c:120-184). These are the engine-owned
    // permanent menus created/refilled by C4Object/C4ObjectMenu
    // (C4Object.cpp:1919-1980,2044-2062; C4ObjectMenu.cpp:207-435).
    let mut engine = load_installed_scenario("Tutorial.c4f/Tutorial03.c4s", 0);
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Building-menu tester".to_string(),
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
            auto_context_menu: true,
            startup_player_count: 1,
        })
        .expect("Tutorial03 player joins")
        .initialized()
        .expect("Tutorial03 player initializes");
    let clonk = engine
        .crew_cursor(joined.number)
        .expect("Tutorial03 joins one selected CLNK");
    // ReadyMaterial FLAG enters the ready HUT3 during ScenarioInit, then
    // C4Object::ExecBase assigns Base on Tick10 (C4Object.cpp:1000-1018).
    // Do not enter early: the pre-base context correctly lacks Buy/Sell.
    // Exit spends its first C++ Execute in InitEvaluation, so Tick10 may
    // assign Base one frame before the ready crew actually leaves.
    for _ in 0..20 {
        engine
            .tick_without_snapshot()
            .expect("ready-base initialization frame");
        let base_ready = engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "HUT3" && object.base == joined.number);
        let crew_exited = engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none());
        if base_ready && crew_exited {
            break;
        }
    }
    let hut = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "HUT3" && object.base == joined.number)
        .expect("Tick10 assigns the ready HUT3 as this player's base");
    assert_ne!(
        hut.ocf & ocf::ENTRANCE,
        0,
        "full-con HUT3 exposes its DefCore entrance"
    );
    // PlaceReadyCrew immediately queues Exit after entering the ready base
    // (C4Player.cpp:551-564). The loop above waits through its separate
    // InitEvaluation and execution frames. The entrance control path has its
    // own regression below; put the CLNK back inside here to isolate the
    // tutorial's subsequent Context -> Buy -> Contents sequence.
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("joined Tutorial03 clonk")
            .container,
        None,
        "ready crew must have executed the C++ Exit command"
    );
    engine
        .apply_object_update(clonk, ObjectUpdate::new().with_container(hut.id))
        .expect("model the completed enter-building step");

    engine.tick_without_snapshot().expect("auto-context frame");
    let context = engine
        .debug_object_menu(clonk.as_u64())
        .expect("clonk exists")
        .unwrap_or_else(|| {
            let clonk = engine.object_snapshot(clonk).expect("clonk snapshot");
            panic!(
                "HUT3 opens C4MN_Context: container={:?}, crew={}, commands={:?}",
                clonk.container,
                clonk.crew_member,
                clonk.command_stack.command_names()
            )
        });
    assert_eq!(
        engine.object_snapshot(hut.id).expect("hut survives").base,
        joined.number,
        "the auto-context frame must retain the ready-base owner"
    );
    assert_eq!(context.identification, Value::Int(14));
    assert_eq!(
        context
            .items
            .iter()
            .map(|item| item.caption.as_str())
            .collect::<Vec<_>>(),
        vec!["Contents", "Buy", "Sell", "Info", "Exit"]
    );
    // C4ObjectMenu::RefillInternal composes the context symbols from the
    // target picture, owner-colored Buy/Sell recipes, target+OKCancel Info,
    // and fctExit respectively (C4ObjectMenu.cpp:361-427; C4Menu.cpp:43-70).
    assert_eq!(
        context
            .items
            .iter()
            .map(|item| item.symbol)
            .collect::<Vec<_>>(),
        vec![
            clonk_engine::ObjectMenuSymbol::Definition,
            clonk_engine::ObjectMenuSymbol::Buy {
                owner: joined.number,
            },
            clonk_engine::ObjectMenuSymbol::Sell {
                owner: joined.number,
            },
            clonk_engine::ObjectMenuSymbol::Info,
            clonk_engine::ObjectMenuSymbol::Exit,
        ]
    );

    engine
        .player_in_com(joined.number, COM_RIGHT, 0)
        .expect("select Buy");
    engine
        .player_in_com(joined.number, COM_THROW, 0)
        .expect("open Buy");
    let buy = engine
        .debug_object_menu(clonk.as_u64())
        .expect("clonk exists")
        .expect("Buy menu opens");
    assert_eq!(buy.identification, Value::Int(4));
    assert_eq!(
        buy.items
            .iter()
            .map(|item| (item.item_id.as_str(), item.count, item.value))
            .collect::<Vec<_>>(),
        vec![("LORY", 1, Some(20))]
    );

    engine
        .player_in_com(joined.number, COM_THROW, 0)
        .expect("buy LORY");
    assert_eq!(engine.player(joined.number).expect("player").wealth(), 5);
    engine
        .player_in_com(joined.number, COM_DIG, 0)
        .expect("close Buy");
    engine
        .tick_without_snapshot()
        .expect("auto-context reopens");
    assert_eq!(
        engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("Context reopens")
            .identification,
        Value::Int(14)
    );
    engine
        .player_in_com(joined.number, COM_THROW, 0)
        .expect("open Contents");
    let contents = engine
        .debug_object_menu(clonk.as_u64())
        .expect("clonk exists")
        .expect("Contents menu opens");
    assert_eq!(contents.identification, Value::Int(18));
    assert!(contents.items.iter().any(|item| item.item_id == "LORY"));
}

fn tutorial_hut_keeps_its_defcore_entrance_for_up_control(scenario: &Scenario) {
    // HUT2's DefCore Entrance=-18,8,16,17 must reach SetOCF's full-Con
    // OCF_Entrance bit (C4Object.cpp:586-589). ObjectComUp at the real
    // entrance then queues Enter before considering Jump
    // (C4ObjectCom.cpp:335-348).
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine).expect("Tutorial01 applies");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Entrance tester".to_string(),
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
        .expect("Tutorial01 player joins")
        .initialized()
        .expect("Tutorial01 player initializes");
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

    engine
        .tick_without_snapshot()
        .expect("first entrance command frame");
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK while the door opens")
            .container,
        None,
        "C4Command::Enter waits outside a closed entrance (C4Command.cpp:600-609)"
    );
    assert_eq!(
        engine
            .object_snapshot(hut.id)
            .expect("HUT2 while opening")
            .action
            .name,
        "OpenDoor",
        "the closed entrance calls ActivateEntrance before entering"
    );

    for _ in 0..20 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut.id))
        {
            break;
        }
        engine.tick_without_snapshot().expect("door-opening frame");
    }
    let clonk_after_open = engine
        .object_snapshot(clonk)
        .expect("CLNK after the door opens");
    let hut_after_open = engine
        .object_snapshot(hut.id)
        .expect("HUT2 after opening frames");
    assert_eq!(
        clonk_after_open.container,
        Some(hut.id),
        "C4Command::Enter enters once EntranceStatus becomes nonzero; hut action={:?}, clonk position={:?}, commands={:?}",
        hut_after_open.action,
        clonk_after_open.position,
        clonk_after_open.command_stack.command_names(),
    );
}

fn tutorial_flag_throw_assigns_base_and_unlocks_digging(scenario: &Scenario) {
    // Tutorial01's real sequence carries FLAG through Script60, teaches
    // contained COM_Throw in Script110, observes C4Object::Base through
    // GetBase in Script120, then unlocks digging in Script160. Contained
    // Throw is synchronous (C4Object.cpp:3280-3282; C4Command.cpp:966-970)
    // and ExecBase attaches the flag on Tick10 (C4Object.cpp:1000-1018).
    let content = content_root();
    let mut engine = Engine::with_seed(0);
    let material_group =
        Group::open(content.join("Material.c4g")).expect("the real global Material.c4g opens");
    let materials = clonk_resources::MaterialLibrary::from_group(&material_group)
        .expect("the real global material definitions load");
    engine.configure_materials_from_library(&materials);
    scenario.apply(&mut engine).expect("Tutorial01 applies");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Flag tester".to_string(),
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
        .expect("Tutorial01 player joins")
        .initialized()
        .expect("Tutorial01 player initializes");
    assert_eq!(joined.number, 0, "Tutorial01 scripts address player zero");
    let clonk = engine
        .crew_cursor(joined.number)
        .expect("Tutorial01 joins one selected CLNK");
    let hut = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id.as_str() == "HUT2")
        .expect("Tutorial01 creates HUT2")
        .id;

    let flag = (0..700)
        .find_map(|_| {
            engine
                .tick_without_snapshot()
                .expect("tutorial lead-in frame");
            engine
                .snapshot()
                .objects
                .into_iter()
                .find(|object| object.definition_id.as_str() == "FLAG")
                .map(|object| object.id)
        })
        .expect("Script50 creates the tutorial flag");
    engine
        .apply_object_update(flag, ObjectUpdate::new().with_container(clonk))
        .expect("collect the real tutorial flag");
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_container(hut)
                .with_action("Walk")
                .with_command_direction(CommandDirection::Stop),
        )
        .expect("stand inside the tutorial hut");

    let mut mask = engine.snapshot().players[0].show_control;
    let mut mask_changes = 0;
    for _ in 0..400 {
        engine
            .tick_without_snapshot()
            .expect("tutorial flag instruction frame");
        let next = engine.snapshot().players[0].show_control;
        if next != mask {
            mask = next;
            mask_changes += 1;
            if mask_changes == 2 {
                break;
            }
        }
    }
    assert_eq!(
        mask_changes, 2,
        "Script60 must accept the carried flag and reach Script110"
    );

    engine
        .player_in_com(joined.number, COM_THROW, 0)
        .expect("contained Throw control");
    assert_eq!(
        engine
            .object_snapshot(flag)
            .expect("FLAG after Throw")
            .container,
        Some(hut),
        "COM_Throw puts FLAG into HUT2 before returning"
    );
    for _ in 0..20 {
        engine.tick_without_snapshot().expect("ExecBase frame");
        if engine
            .object_snapshot(hut)
            .is_some_and(|object| object.base == joined.number)
        {
            break;
        }
    }
    let hut_after_flag = engine.object_snapshot(hut).expect("HUT2 after FLAG");
    let flag_after_base = engine.object_snapshot(flag).expect("attached FLAG");
    assert_eq!(hut_after_flag.base, joined.number);
    assert_eq!(flag_after_base.container, None);
    assert_eq!(flag_after_base.action.name, "FlyBase");
    assert_eq!(flag_after_base.action.target, Some(hut));

    let script110_mask = engine.snapshot().players[0].show_control;
    for _ in 0..150 {
        engine.tick_without_snapshot().expect("Script120 frame");
        if engine.snapshot().players[0].show_control != script110_mask {
            break;
        }
    }
    assert_ne!(
        engine.snapshot().players[0].show_control,
        script110_mask,
        "Script120 must call GetBase(HUT2) and advance without a script error"
    );

    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .clear_container()
                .with_position(Vector2::new(220, 271))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .with_command_direction(CommandDirection::Stop),
        )
        .expect("leave HUT2 for the digging lesson");
    for _ in 0..500 {
        engine
            .apply_object_update(
                clonk,
                ObjectUpdate::new()
                    .with_position(Vector2::new(220, 271))
                    .with_velocity(Vector2::ZERO),
            )
            .expect("keep CLNK in Script160's lesson area");
        engine.tick_without_snapshot().expect("Script160 frame");
        let script160_message = engine.snapshot().hud.messages.iter().any(|message| {
            message
                .lines
                .iter()
                .any(|line| line.contains("Use this key to start a digging process"))
        });
        if script160_message
            && engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.temporary_physical.is_none())
        {
            break;
        }
    }
    let script160_clonk = engine.object_snapshot(clonk).expect("CLNK at Script160");
    assert!(
        script160_clonk.temporary_physical.is_none()
            && engine.snapshot().hud.messages.iter().any(|message| {
                message
                    .lines
                    .iter()
                    .any(|line| line.contains("Use this key to start a digging process"))
            }),
        "Script160 must teach and unlock digging; frame={}, position={:?}, container={:?}, show_control={}, messages={:?}",
        engine.snapshot().frame,
        script160_clonk.position,
        script160_clonk.container,
        engine.snapshot().players[0].show_control,
        engine.snapshot().hud.messages,
    );
    let gold = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id.as_str() == "GOLD")
        .expect("Script150 creates the tutorial gold")
        .id;
    let (width, height, before_dig) = engine
        .debug_landscape_plane()
        .expect("Tutorial01 keeps its authoritative Surface8 plane");
    assert!(
        engine.debug_landscape_is_solid(220, 299),
        "the lesson floor starts solid before DigFree"
    );
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(220, 289))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .with_direction(Direction::Left)
                .with_command_direction(CommandDirection::Stop),
        )
        .expect("place the unlocked CLNK on the real lesson floor");
    engine
        .player_in_com(joined.number, COM_DIG, 0)
        .expect("normal player Dig control");
    for _ in 0..100 {
        engine
            .tick_without_snapshot()
            .expect("diagonal tutorial digging frame");
    }
    let diagonal_digger = engine.object_snapshot(clonk).expect("CLNK after Dig");
    assert_eq!(diagonal_digger.action.name, "Dig");
    assert_eq!(
        diagonal_digger.command_direction,
        CommandDirection::DownLeft
    );
    // DoMovement applies an active action's DigFree circle to the predicted
    // position on Surface8 (C4Movement.cpp:227-245). This is the real pixel
    // underneath the lesson start, not a replacement heightfield fixture.
    assert!(
        !engine.debug_landscape_is_solid(220, 299),
        "the real tutorial floor pixel must be excavated"
    );
    let (dug_width, dug_height, after_diagonal_dig) = engine
        .debug_landscape_plane()
        .expect("Tutorial01 Surface8 remains authoritative after digging");
    assert_eq!((dug_width, dug_height), (width, height));
    assert_ne!(
        after_diagonal_dig, before_dig,
        "DigFree must mutate the authoritative landscape byte plane"
    );

    engine
        .player_in_com(joined.number, COM_LEFT, 0)
        .expect("steer the active dig left toward the gold");
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK after steering")
            .command_direction,
        CommandDirection::Left
    );
    // Tick3 cross-check collection enters carryable objects into the crew
    // (C4GameObjects.cpp:143-194; C4Object.cpp:5693-5713). Follow the real
    // tutorial tunnel until its Script150 GOLD enters the real CLNK.
    for _ in 0..140 {
        engine
            .tick_without_snapshot()
            .expect("horizontal tutorial digging frame");
        if engine
            .object_snapshot(gold)
            .is_some_and(|object| object.container == Some(clonk))
        {
            break;
        }
    }
    let collected_gold = engine.object_snapshot(gold).expect("tutorial GOLD remains");
    assert_eq!(
        collected_gold.container,
        Some(clonk),
        "the excavated tunnel must let the real CLNK collect Script150's GOLD; clonk={:?}, gold={:?}",
        engine.object_snapshot(clonk),
        collected_gold,
    );

    // Script200 must observe GOLD while it is still carried. The natural
    // route spends many frames climbing back to HUT2; this bounded test
    // teleports only after Script206's message proves the same Tick10 script
    // observation happened (Tutorial01 Script.c:151-169).
    for _ in 0..250 {
        if engine
            .snapshot()
            .hud
            .messages
            .iter()
            .any(|message| message.lines.iter().any(|line| line.contains("Wonderful!")))
        {
            break;
        }
        engine
            .tick_without_snapshot()
            .expect("Script200 GOLD observation frame");
    }
    assert!(
        engine.snapshot().hud.messages.iter().any(|message| {
            message
                .lines
                .iter()
                .any(|line| line.contains("Wonderful!"))
        }),
        "Script200 must observe carried GOLD before the bounded route enters HUT2; frame={}, messages={:?}, clonk={:?}, gold={:?}",
        engine.frame(),
        engine.snapshot().hud.messages,
        engine.object_snapshot(clonk),
        engine.object_snapshot(gold),
    );

    // Tutorial01 Script210/215 asks the player to carry GOLD into HUT2,
    // then fulfills SCRG and selects Tutorial02 (Script.c:171-182). The
    // normal Up control must take C4ObjectCom's entrance path before Jump
    // (C4ObjectCom.cpp:335-348). Once Enter's callbacks finish, HUT2
    // synchronously auto-sells nested BaseAutoSell GOLD
    // (C4Object.cpp:1625-1634,970-995).
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(570, 170))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .with_command_direction(CommandDirection::Stop),
        )
        .expect("place the gold-carrying CLNK at HUT2's entrance");

    // C4Game::Ticks only raises TimeGo; the external Sec1Timer consumes it
    // (C4Game.cpp:1755-1759,1899-1913). Let seven deterministic seconds
    // pass at Tick35 cadence while Script215 waits for the player to enter
    // (C4Game.cpp:1908; Tutorial01 Script.c:176-181).
    for _ in 0..7 {
        for _ in 0..35 {
            engine
                .tick_without_snapshot()
                .expect("pre-completion clock frame");
        }
        engine.sec1_timer();
    }
    assert_eq!(engine.game_time(), 7);

    engine
        .player_in_com(joined.number, COM_UP, 0)
        .expect("normal player Up control at HUT2");
    for _ in 0..30 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
        {
            break;
        }
        engine
            .tick_without_snapshot()
            .expect("final HUT2 entrance frame");
    }
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK after entering HUT2")
            .container,
        Some(hut),
        "the normal Up control must carry the CLNK into HUT2"
    );
    assert_eq!(
        engine
            .player(joined.number)
            .expect("player exists")
            .wealth(),
        5,
        "HUT2 sells GOLD in the successful Enter call"
    );
    assert!(
        engine.object_snapshot(gold).is_none(),
        "Sell2Home removes the nested GOLD synchronously"
    );

    // Script215 runs on the scenario host's Tick10 cadence after the Clonk
    // enters (C4ScriptHost.cpp:222-230).
    let mut reached_tutorial02 = false;
    for _ in 0..20 {
        engine
            .tick_without_snapshot()
            .expect("Script215 approach frame");
        if engine.next_mission().path == r"Tutorial.c4f\Tutorial02.c4s" {
            reached_tutorial02 = true;
            break;
        }
    }
    let script215_snapshot = engine.snapshot();
    assert!(
        reached_tutorial02,
        "Script215 must fulfill SCRG and select Tutorial02; frame={}, next_mission={:?}, show_control={}, messages={:?}, clonk={:?}, gold={:?}",
        engine.frame(),
        engine.next_mission(),
        script215_snapshot.players[0].show_control,
        script215_snapshot.hud.messages,
        engine.object_snapshot(clonk),
        engine.object_snapshot(gold),
    );

    // SCRG's GOAL controller checks fulfillment every 250 frames, changes
    // to the 30-frame Wait4End action, then calls GameOver
    // (Goal.c4d DefCore.txt:7-8; ActMap.txt:9-15; Script.c:52-81).
    let mut completed = None;
    for _ in 0..300 {
        let snapshot = engine.tick().expect("GOAL completion frame");
        if snapshot.game_over {
            completed = Some(snapshot);
            break;
        }
    }
    let completed = completed.unwrap_or_else(|| {
        panic!(
            "fulfilled Tutorial01 must reach GameOver; frame={}, next_mission={:?}",
            engine.frame(),
            engine.next_mission(),
        )
    });

    // C4Game evaluates players before C4RoundResults freezes the goal list
    // and Game.Time (C4Game.cpp:845-854,1832-1856;
    // C4RoundResults.cpp:280-313). The final owned assets are CLNK 25,
    // FLAG 100, HUT2 35 and GOLD 5: 140 gain over the initial CLNK, plus
    // C4Player::Evaluate's 100-point winner bonus (C4Player.cpp:84-105,
    // 930-968).
    assert_eq!(
        completed
            .round_results
            .goals
            .iter()
            .map(|goal| goal.as_str())
            .collect::<Vec<_>>(),
        vec!["SCRG"]
    );
    assert_eq!(
        completed
            .round_results
            .fulfilled_goals
            .iter()
            .map(|goal| goal.as_str())
            .collect::<Vec<_>>(),
        vec!["SCRG"]
    );
    assert_eq!(completed.round_results.playing_time_seconds, 7);

    let player = completed
        .players
        .iter()
        .find(|player| player.id == joined.number)
        .expect("Tutorial01 evaluated player");
    assert!(player.won);
    assert!(player.evaluated);
    assert_eq!(player.value, 165);
    assert_eq!(player.value_gain, 140);
    assert_eq!(player.score, 240);
    assert_eq!(player.total_playing_time, 7);

    let result = completed
        .round_results
        .players
        .iter()
        .find(|result| result.player_info_id == player.player_info_id)
        .expect("Tutorial01 frozen player result");
    assert_eq!(result.total_playing_time, 7);
    assert_eq!(result.score_old, 0);
    assert_eq!(result.score_new, Some(240));
}

fn tutorial_clonk_jumps_into_a_ceiling_and_hangles_like_cpp(scenario: &Scenario) {
    // C4PhysicalInfo::PromotionUpdate enables CanHangle for every ranked
    // crew member (C4InfoCore.cpp:207-213). A low-speed DFA_FLIGHT contact
    // through the CLNK top vertex then enters Hangle without changing its
    // facing (C4Object.cpp:4369-4404; C4ObjectCom.cpp:112-118).
    let mut engine = Engine::with_seed(0);
    scenario
        .apply_before_players(&mut engine)
        .expect("Tutorial01 definitions apply");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Ceiling tester".to_string(),
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
        .expect("Tutorial01 player joins")
        .initialized()
        .expect("Tutorial01 player initializes");
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

    engine.tick_without_snapshot().expect("first jump frame");
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
        engine
            .tick_without_snapshot()
            .expect("ceiling approach frame");
    }
    let hangle = engine.object_snapshot(clonk).expect("CLNK after contact");
    assert_eq!(hangle.action.name, "Hangle");
    assert_eq!(hangle.direction, Direction::Right);
    assert_eq!(hangle.velocity, Vector2::ZERO);
}

fn tutorial_clonk_flight_keeps_accelerating_past_twelve_pixels_per_tick(scenario: &Scenario) {
    // DFA_FLIGHT calls DoGravity every frame (C4Object.cpp:4893-4904), whose
    // free-fall branch only adds GravAccel (C4Object.cpp:4672-4674). C++ has
    // no generic terminal-velocity clamp, so a Clonk falling through enough
    // open space must accelerate past 12 px/tick.
    let mut engine = Engine::with_seed(0);
    scenario
        .apply_before_players(&mut engine)
        .expect("Tutorial01 definitions apply");
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Flight tester".to_string(),
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
        .expect("Tutorial01 player joins")
        .initialized()
        .expect("Tutorial01 player initializes");
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
        engine.tick_without_snapshot().expect("open flight frame");
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
