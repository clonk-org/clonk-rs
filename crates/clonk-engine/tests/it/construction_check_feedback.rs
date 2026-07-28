//! ConstructionCheck failure feedback (C4Landscape.cpp:2125-2169).
//!
//! Every rejected branch reports through `GameMsgObject(szMsg, pByObj,
//! FRed)` when a calling object exists: IDS_OBJ_UNDEF, IDS_OBJ_NOCON,
//! IDS_OBJ_NOROOM, IDS_OBJ_NOLEVEL and IDS_OBJ_NOOTHER. Both the script
//! `CreateConstruction(..., fCheckSite)` path (C4Script.cpp:1918-1921) and
//! the C4CMD_Construct site check (C4Command.cpp:1797-1801) must leave the
//! same red target message.

use clonk_engine::landscape::PixelGrid;
use clonk_engine::{
    Definition, Engine, Landscape, MessageKind, ObjectUpdate, PhysicalInfo, SpawnConfig, Vector2,
    CATEGORY_STATIC_BACK,
};
use clonk_resources::{Group, ResourceDefinition};
use clonk_script::Value;
use std::fs;

const WIDTH: u32 = 300;
const HEIGHT: u32 = 200;
const FLOOR_Y: i32 = 150;
/// `0xff000000 | Pal.GetClr(FColors[FRed])` (C4GameMessage.cpp:280-282).
const MESSAGE_RED: u32 = 0xfff4_0000;

/// Flat granite floor from `FLOOR_Y` down; open air above.
fn flat_floor_landscape() -> Landscape {
    let bytes = (0..HEIGHT as i32)
        .flat_map(|y| (0..WIDTH as i32).map(move |_| u8::from(y >= FLOOR_Y)))
        .collect();
    let grid = PixelGrid::new(
        WIDTH,
        HEIGHT,
        bytes,
        vec![0, 50],
        vec![None, Some("Granite".to_string())],
        vec![None; 2],
    );
    let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
    landscape.set_pixel_grid(grid);
    landscape.set_world_height(HEIGHT as i32);
    landscape
}

fn structure_definition(
    root: &std::path::Path,
    id: &str,
    name: &str,
    constructable: bool,
) -> Definition {
    let path = root.join(format!("{id}.c4d"));
    fs::create_dir(&path).expect("definition dir");
    fs::write(
        path.join("DefCore.txt"),
        format!(
            "[DefCore]\nid={id}\nName={name}\nCategory=C4D_Structure\nMass=100\n\
             Width=40\nHeight=40\nOffset=-20,-20\nConstruction={}\n",
            i32::from(constructable)
        ),
    )
    .expect("DefCore.txt");
    fs::write(path.join("Script.c"), "#strict 2\n").expect("Script.c");
    let resource =
        ResourceDefinition::load(&Group::open(&path).expect("group opens")).expect("loads");
    Definition::from_resource(&resource).expect("definition converts")
}

fn feedback_engine() -> (Engine, clonk_engine::ObjectId, tempfile::TempDir) {
    let temp = tempfile::tempdir().expect("definition tempdir");
    let mut engine = Engine::new();
    engine.set_landscape(flat_floor_landscape());
    engine
        .register_definition(structure_definition(temp.path(), "STRC", "Testhouse", true))
        .expect("STRC registers");
    engine
        .register_definition(structure_definition(
            temp.path(),
            "NOCN",
            "Ruinshell",
            false,
        ))
        .expect("NOCN registers");
    let mut caller = Definition::from_script(
        "CALR",
        "Caller",
        r#"#strict 2
public func Place(id def, int x, int y)
{
    var s = CreateConstruction(def, x, y, -1, 1, 0, 1);
    if (s) { return 1; }
    return 0;
}
public func Order(id def, int x, int y)
{
    return SetCommand(this, "Construct", 0, x, y, 0, def);
}
"#,
    )
    .expect("caller compiles");
    caller.set_category(CATEGORY_STATIC_BACK);
    caller.set_physical(PhysicalInfo {
        can_construct: 1,
        ..PhysicalInfo::default()
    });
    engine.register_definition(caller).expect("CALR registers");
    engine.register_script_definition("CNKT", "Conkit", "#strict 2\n").expect("CNKT registers");
    let caller = engine
        .spawn_object(SpawnConfig::new("CALR").with_position(Vector2::new(0, 0)))
        .expect("caller spawns");
    (engine, caller, temp)
}

fn place(engine: &mut Engine, caller: clonk_engine::ObjectId, id: &str, x: i32, y: i32) -> i32 {
    let index = engine.find_object_index(caller).expect("caller index");
    match engine
        .call_object_function(
            index,
            "Place",
            vec![Value::C4Id(id.to_string()), Value::Int(x), Value::Int(y)],
        )
        .expect("Place runs")
    {
        Value::Int(value) => value,
        // Below #strict 3 the literal `0` is nil (C++ C4Aul semantics).
        Value::Nil => 0,
        other => panic!("int expected: {other:?}"),
    }
}

fn caller_messages(engine: &Engine, caller: clonk_engine::ObjectId) -> Vec<(Vec<String>, u32)> {
    engine
        .snapshot()
        .hud
        .messages
        .into_iter()
        .filter(|message| message.target == Some(caller))
        .map(|message| (message.lines, message.color))
        .collect()
}

#[test]
fn construction_check_reports_each_failed_branch_on_the_caller() {
    let (mut engine, caller, _temp) = feedback_engine();

    // Support strip below (150,100) is open air -> IDS_OBJ_NOLEVEL
    // (C4Landscape.cpp:2152-2157).
    assert_eq!(place(&mut engine, caller, "STRC", 150, 100), 0);
    assert_eq!(
        caller_messages(&engine, caller),
        vec![(vec!["No level ground!".to_string()], MESSAGE_RED)]
    );

    // Body rect buried in granite -> IDS_OBJ_NOROOM (:2144-2150). The
    // non-Multiple target message replaces the NOLEVEL line.
    assert_eq!(place(&mut engine, caller, "STRC", 150, 190), 0);
    assert_eq!(
        caller_messages(&engine, caller),
        vec![(vec!["Not enough room!".to_string()], MESSAGE_RED)]
    );

    // Constructable=0 -> IDS_OBJ_NOCON with the definition name and the
    // C++ line break (:2139-2143).
    assert_eq!(place(&mut engine, caller, "NOCN", 150, FLOOR_Y), 0);
    assert_eq!(
        caller_messages(&engine, caller),
        vec![(
            vec!["Ruinshell cannot".to_string(), "be built.".to_string()],
            MESSAGE_RED
        )]
    );

    // A valid floor spot places the site and leaves no failure feedback.
    assert_eq!(place(&mut engine, caller, "STRC", 150, FLOOR_Y), 1);
    assert_eq!(
        caller_messages(&engine, caller),
        vec![(
            vec!["Ruinshell cannot".to_string(), "be built.".to_string()],
            MESSAGE_RED
        )],
        "a successful CreateConstruction leaves the previous message alone"
    );

    // Game.OverlapObject compares con-scaled live shapes, so the fresh 1%
    // site cannot block anything yet; a full-con neighbor does ->
    // IDS_OBJ_NOOTHER named after the blocker (C4Game.cpp:1298-1313;
    // C4Landscape.cpp:2159-2163).
    engine
        .spawn_object(SpawnConfig::new("STRC").with_position(Vector2::new(220, FLOOR_Y - 20)))
        .expect("full-con blocker spawns");
    assert_eq!(place(&mut engine, caller, "STRC", 230, FLOOR_Y), 0);
    assert_eq!(
        caller_messages(&engine, caller),
        vec![(vec!["Testhouse is in the way.".to_string()], MESSAGE_RED)]
    );

    // An unknown id resolves through ConstructionCheck's own C4Id2Def
    // first: IDS_OBJ_UNDEF with the raw id text, still no site
    // (C4Landscape.cpp:2131-2138).
    assert_eq!(place(&mut engine, caller, "UNDF", 150, FLOOR_Y), 0);
    assert_eq!(
        caller_messages(&engine, caller),
        vec![(vec!["Structure UNDF undefined.".to_string()], MESSAGE_RED)]
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "UNDF"),
        "an undefined id never places a site"
    );
}

#[test]
fn construct_command_site_rejection_messages_the_actor() {
    let (mut engine, caller, _temp) = feedback_engine();
    // C4CMD_Construct requires a carried conkit before the site check
    // (C4Command.cpp:1783-1795).
    let kit = engine
        .spawn_object(SpawnConfig::new("CNKT"))
        .expect("kit spawns");
    engine
        .apply_object_update(kit, ObjectUpdate::new().with_container(caller))
        .expect("kit enters the caller");

    // The requested mid-map site (150,100) is inside the actor's at-site
    // range but hangs in open air -> the check rejects with
    // IDS_OBJ_NOLEVEL before C4Command::Fail (C4Command.cpp:1797-1801).
    let index = engine.find_object_index(caller).expect("caller index");
    engine
        .call_object_function(
            index,
            "Order",
            vec![
                Value::C4Id("STRC".to_string()),
                Value::Int(150),
                Value::Int(100),
            ],
        )
        .expect("Order runs");
    engine
        .apply_object_update(
            caller,
            ObjectUpdate::new().with_position(Vector2::new(150, 100)),
        )
        .expect("stand the actor at the requested site");
    for _ in 0..3 {
        engine.tick_without_snapshot().expect("command executes");
    }

    let messages = engine
        .snapshot()
        .hud
        .messages
        .into_iter()
        .filter(|message| {
            message.target == Some(caller)
                && message.kind == MessageKind::Target
                && message.lines == vec!["No level ground!".to_string()]
        })
        .collect::<Vec<_>>();
    assert_eq!(
        messages.len(),
        1,
        "the Construct command's rejected site check leaves the C++ feedback"
    );
    assert_eq!(messages[0].color, MESSAGE_RED);
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "STRC"),
        "the rejected Construct command must not place a site"
    );
}
