//! Deep Sea construction-start chain (M03-P2-L225).
//!
//! User report: in FarWorlds.c4f/Deep.c4s construction never starts. The
//! whole loop is underwater: CNKT from the TRTS shell -> dig-double
//! Activate -> CXCN knowledge menu -> CreateConstructionSite (gated on the
//! clonk WALKING on the sea floor) -> double-Down on the site ->
//! C4CMD_Build -> Build action -> Con rises consuming materials
//! (Conkit.c4d/Script.c; C4ObjectCom.cpp:531-539,573-589,690-698;
//! C4Command.cpp:874-895; C4Landscape.cpp:2125-2169; C4Object.cpp:
//! 1682-1775,5010-5055).
//!
//! Chain audit against the native binary (2026-07-21, seed 0, this
//! repository's arm64 C++ build on a Deep.c4s probe copy): landscape
//! floors and all 267 sampled ConstructionCheck columns are byte-identical
//! (see `deep_sea_construction_site_survey_matches_cpp_oracle`); the one
//! divergent link was the missing ConstructionCheck failure feedback,
//! pinned in the reject leg of the full-loop test below.

use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::{
    ObjectUpdate, SpawnConfig, Vector2, COM_DIG, COM_DOWN, COM_MENU_SELECT, COM_THROW, FULL_CON,
};
use clonk_script::Value;

/// `GameMsgObject(..., FRed)`: 0xff000000 | C4.PAL entry FColors[FRed]=47
/// (C4GameMessage.cpp:280-282; C4Surface.cpp:1304; StdColors.h:32).
const MESSAGE_RED: u32 = 0xfff4_0000;

/// Keep the real Deep Sea world-tick sample bounded while exercising the
/// C4Object::Build path below (C4Object.cpp:1682-1775,5010-5055).
const MAX_BUILD_TICKS: i32 = 32;

#[test]
fn deep_sea_conkit_site_starts_building_underwater() {
    let mut engine = load_installed_scenario("FarWorlds.c4f/Deep.c4s", 0);
    let owner = join_local_player(&mut engine, "Deep Sea construction parity");
    let clonk = engine
        .crew_cursor(owner)
        .expect("Deep Sea joins with a selected HCLK");
    let clonk_state = engine.object_snapshot(clonk).expect("cursor clonk lives");
    assert_eq!(clonk_state.definition_id, "HCLK");
    // Seed-zero ScenarioInit: Position=26,35 zoomed by MapZoom=17, crew
    // spread (C4Player.cpp:710-763). The shore under the shell is walkable
    // at boot; the C++ oracle run joins identically.
    assert_eq!(clonk_state.position, Vector2::new(471, 585));
    assert_eq!(clonk_state.action.name, "Walk");
    assert!(
        engine.debug_landscape_is_liquid(clonk_state.position.x, clonk_state.position.y),
        "the Deep Sea start is underwater"
    );

    // Deep.c4s InitializePlayer fills the TRTS shell with 5 CNKT + 3 LNKT.
    let base_state = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "TRTS")
        .expect("Deep Sea places the player's TRTS shell");
    assert_eq!(base_state.position, Vector2::new(449, 575));
    let count_contents = |ids: &[clonk_engine::ObjectId], definition: &str| {
        ids.iter()
            .filter(|id| {
                engine
                    .object_snapshot(**id)
                    .is_some_and(|object| object.definition_id == definition)
            })
            .count()
    };
    assert_eq!(count_contents(&base_state.contents, "CNKT"), 5);
    assert_eq!(count_contents(&base_state.contents, "LNKT"), 3);

    // The scenario's own Deep.c4d overloads CNKT: GCOR=2;METL=1 components
    // replace the Objects.c4d WOOD=2;METL=1 and Activate loses the
    // CanConstruct gate (Deep.c4d/Items.c4d/Conkit.c4d).
    let conkit = base_state
        .contents
        .iter()
        .copied()
        .find(|id| {
            engine
                .object_snapshot(*id)
                .is_some_and(|object| object.definition_id == "CNKT")
        })
        .expect("a shell conkit");
    let kit_components = engine
        .object_snapshot(conkit)
        .expect("CNKT lives")
        .components;
    assert_eq!(kit_components.get("GCOR"), Some(&2));
    assert_eq!(kit_components.get("METL"), Some(&1));

    // Take one CNKT from the shell (the interactive contents-menu route is
    // pinned above ground by the Tutorial04 suites).
    engine
        .apply_object_update(conkit, ObjectUpdate::new().with_container(clonk))
        .expect("take a CNKT from the TRTS shell");
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("clonk lives")
            .contents
            .first(),
        Some(&conkit),
        "dig-double activates the first carried object (C4ObjectCom.cpp:537-539)"
    );

    // Link: dig-double opens CNKT::Activate's CXCN menu underwater, filled
    // from GetPlrKnowledge(owner, 0, i++, C4D_Structure()).
    engine.player_in_com(owner, COM_DIG, 0).expect("first Dig");
    engine
        .player_in_com(owner, COM_DIG, 0)
        .expect("second Dig inside the double-click window");
    {
        let (_, menu) = engine
            .cursor_object_menu(owner)
            .expect("CNKT::Activate opens its CXCN menu underwater");
        assert_eq!(menu.identification, Value::C4Id("CXCN".to_string()));
        assert_eq!(menu.symbol_id, "CXCN");
        assert_eq!(menu.command_object, Some(conkit));
        assert_eq!(menu.extra, clonk_engine::ObjectMenuExtra::Components);
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<Vec<_>>(),
            ["FDR2", "PWR2", "DRSC"],
            "Deep Sea knowledge lists its three C4D_Structure plans in scenario order"
        );
        assert_eq!(
            menu.items[1].components,
            vec![
                clonk_engine::ObjectMenuComponent {
                    definition_id: "GCOR".to_string(),
                    count: 6,
                },
                clonk_engine::ObjectMenuComponent {
                    definition_id: "GLAS".to_string(),
                    count: 2,
                },
            ],
            "the PWR2 row retains its C++ component order"
        );
    }

    // Reject leg: the boot shore cannot support a 96x80 PWR2 (support strip
    // below (471,595) is water; C++ oracle column x=470 pwr2=0).
    // ConstructionCheck fails AFTER the solid test on the support test and
    // must leave the C++ red feedback on the calling kit
    // (C4Landscape.cpp:2152-2157; C4GameMessage.cpp:280-282). The failed
    // CreateConstruction returns nil, so CreateConstructionSite keeps the
    // kit (Conkit.c4d/Script.c).
    engine
        .player_in_com(owner, COM_MENU_SELECT, 1)
        .expect("select the PWR2 row");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("menu enter runs CreateConstructionSite");
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "PWR2"),
        "the shore reject must not place a site"
    );
    assert_eq!(
        engine
            .object_snapshot(conkit)
            .expect("kit outlives the reject")
            .container,
        Some(clonk),
        "a rejected CreateConstruction keeps the kit"
    );
    assert!(
        engine.cursor_object_menu(owner).is_none(),
        "the one-shot CXCN menu closes on selection"
    );
    let reject_messages = engine
        .snapshot()
        .hud
        .messages
        .into_iter()
        .filter(|message| message.target == Some(conkit))
        .collect::<Vec<_>>();
    assert_eq!(
        reject_messages.len(),
        1,
        "ConstructionCheck leaves exactly one kit-targeted failure message"
    );
    assert_eq!(
        reject_messages[0].lines,
        vec!["No level ground!".to_string()],
        "the support-strip failure uses IDS_OBJ_NOLEVEL"
    );
    assert_eq!(
        reject_messages[0].color, MESSAGE_RED,
        "ConstructionCheck feedback is FRed"
    );

    // Link: swim -> floor-walk. Lift the builder into open water over the
    // first legal floor stretch (C++ oracle x=730/740 accept PWR2) and hold
    // Down: the sinking HCLK lands into Walk through the ACLK ContactBottom
    // contact call (AquaClonk.c4d/Script.c; C4Movement.cpp:166-182;
    // HCLK's own ActMap strips InLiquidAction from Walk).
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(721, 500))
                .with_velocity(Vector2::ZERO)
                .with_action("Swim"),
        )
        .expect("swim above the buildable floor");
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("clonk lives")
            .action
            .name,
        "Swim"
    );
    engine
        .player_in_com(owner, COM_DOWN, 0)
        .expect("hold Down toward the sea floor");
    let mut landed = false;
    for _ in 0..40 {
        engine.tick_without_snapshot().expect("landing advances");
        if engine
            .object_snapshot(clonk)
            .expect("clonk survives the descent")
            .action
            .name
            == "Walk"
        {
            landed = true;
            break;
        }
    }
    let clonk_state = engine.object_snapshot(clonk).expect("clonk lives");
    assert!(
        landed,
        "holding Down must land the swimming HCLK into Walk; action={} pos={:?}",
        clonk_state.action.name, clonk_state.position
    );
    assert_eq!(clonk_state.position, Vector2::new(721, 532));
    assert!(
        engine.debug_landscape_is_liquid(clonk_state.position.x, clonk_state.position.y),
        "the landed builder still walks underwater"
    );

    // Link: the same menu route places the site on the legal floor.
    engine.player_in_com(owner, COM_DIG, 0).expect("first Dig");
    engine
        .player_in_com(owner, COM_DIG, 0)
        .expect("second Dig inside the double-click window");
    assert!(
        engine.cursor_object_menu(owner).is_some(),
        "the CXCN menu reopens over the buildable floor"
    );
    engine
        .player_in_com(owner, COM_MENU_SELECT, 1)
        .expect("select the PWR2 row");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("menu enter runs CreateConstructionSite");
    let site = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "PWR2" && object.status.is_active())
        .expect("CreateConstruction places the PWR2 site underwater");
    // CreateConstruction(idType, 0, 10, owner, 1, 1, 1): kit offsets ride on
    // the carrying clonk, one percent Con (C4Script.cpp:1905-1933).
    assert_eq!(site.position, Vector2::new(721, 542));
    assert_eq!(site.construction, FULL_CON / 100);
    assert_eq!(site.owner, owner);
    assert_ne!(
        site.ocf & 0x2,
        0,
        "an incomplete upright site carries OCF_Construct (C4Object.cpp:549-554)"
    );
    assert!(
        engine
            .object_snapshot(conkit)
            .is_none_or(|kit| !kit.status.is_active()),
        "CreateConstructionSite consumes the kit"
    );
    assert!(
        engine.snapshot().hud.messages.iter().any(|message| {
            message.target == Some(clonk)
                && message
                    .lines
                    .iter()
                    .any(|line| line.starts_with("Construction:"))
        }),
        "the kit script announces the new site on the clonk"
    );
    let site = site.id;

    // The builder carries PWR2's full component demand (GCOR=6;GLAS=2).
    // HCLK's MaxContentsCount(3) is a collection-control gate; script
    // CreateContents ignores it the same way.
    for definition in [
        "GCOR", "GCOR", "GCOR", "GCOR", "GCOR", "GCOR", "GLAS", "GLAS",
    ] {
        let material = engine
            .spawn_object(SpawnConfig::new(definition))
            .expect("component spawns");
        engine
            .apply_object_update(material, ObjectUpdate::new().with_container(clonk))
            .expect("component enters the builder");
    }

    // Deep Sea spawns 7 SHRK predators; a stationary builder is prey in
    // either engine, and a real player would fight or flee. Retire the
    // wildlife so the frame-exact Con pins below observe only the build loop.
    let wildlife = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| matches!(object.definition_id.as_str(), "SHRK" | "FISH"))
        .map(|object| object.id)
        .collect::<Vec<_>>();
    for animal in wildlife {
        engine
            .apply_object_update(
                animal,
                ObjectUpdate::new().with_status(clonk_engine::ObjectStatus::Deleted),
            )
            .expect("retire Deep Sea wildlife for the deterministic build leg");
    }

    // Link: double-Down over the OCF_Construct site starts Build through
    // PlayerObjectCommand(C4CMD_Build) -> C4Command::Build's at-target arm
    // -> ObjectComBuild -> SetActionByName("Build")
    // (C4ObjectCom.cpp:573-589,690-698; C4Command.cpp:874-895).
    engine
        .player_in_com(owner, COM_DOWN, 0)
        .expect("first Down");
    engine
        .player_in_com(owner, COM_DOWN, 0)
        .expect("second Down inside the double-click window");

    // DFA_BUILD per-frame progression (C4Object.cpp:5010-5055,1682-1775):
    // Build(iLevel=10) grabs at most one carried object per needed id per
    // frame, then DoCon(10 * 100 * 150 / Mass=6000 = 25). HCLK's own ActMap
    // Build entry has no InLiquidAction, so the action persists underwater.
    // Six stock-speed frames cover every distinct component-transfer state:
    // both GLAS are consumed by frame 2 and all six GCOR by frame 6.
    const STOCK_BUILD_TICKS: i32 = 6;
    const STOCK_CON_PER_TICK: i32 = 25;
    const ACCELERATED_BUILD_TICKS: i32 = 10;
    let sampled_build_ticks = STOCK_BUILD_TICKS + ACCELERATED_BUILD_TICKS;
    assert!(
        sampled_build_ticks <= MAX_BUILD_TICKS,
        "the real-scenario construction sample must stay within {MAX_BUILD_TICKS} world ticks"
    );
    let total_frames = (FULL_CON - FULL_CON / 100) / STOCK_CON_PER_TICK;
    assert_eq!(total_frames, 3960);
    for frame in 1..=STOCK_BUILD_TICKS {
        engine
            .tick_without_snapshot()
            .expect("stock-speed Build frames advance");
        let site_state = engine.object_snapshot(site).expect("site survives");
        assert_eq!(
            site_state.construction,
            FULL_CON / 100 + frame * STOCK_CON_PER_TICK,
            "frame {frame}: DoCon rises exactly 25 per stock-speed Build frame"
        );
        let builder_state = engine.object_snapshot(clonk).expect("builder survives");
        assert_eq!(
            builder_state.action.name, "Build",
            "frame {frame}: the underwater Build action persists"
        );
        let expected_gcor = frame.min(6);
        let expected_glas = frame.min(2);
        assert_eq!(
            site_state.components.get("GCOR").copied().unwrap_or(0),
            expected_gcor,
            "frame {frame}: the site grabs one carried GCOR per frame"
        );
        assert_eq!(
            site_state.components.get("GLAS").copied().unwrap_or(0),
            expected_glas,
            "frame {frame}: the site grabs carried GLAS until its demand is met"
        );
        assert_eq!(
            builder_state.contents.len() as i32,
            8 - expected_gcor - expected_glas,
            "frame {frame}: consumed components leave the builder's inventory"
        );
    }

    // Keep executing the native DFA_BUILD -> Build -> DoCon path, but raise
    // the builder's temporary physical through C++ SetPhysical semantics
    // (C4Script.cpp:557-601). Ten equal accelerated ticks cover the
    // material-complete steady state and land exactly on FullCon without
    // paying for another 3954 identical full-world ticks.
    let accelerated_delta = (FULL_CON - (FULL_CON / 100 + STOCK_BUILD_TICKS * STOCK_CON_PER_TICK))
        / ACCELERATED_BUILD_TICKS;
    assert_eq!(accelerated_delta, 9_885);
    let accelerated_can_construct = accelerated_delta * 4;
    assert_eq!(accelerated_can_construct, 39_540);
    engine
        .register_definition(
            clonk_engine::Definition::from_script(
                "BSPD",
                "Build-speed probe",
                r#"#strict 2
public func Refill(object target)
{
  return DoBreath(700000, target);
}

public func Accelerate(object target, int speed)
{
  return SetPhysical("CanConstruct", speed, PHYS_Temporary, target);
}
"#,
            )
            .expect("build-speed probe compiles"),
        )
        .expect("build-speed probe registers");
    let accelerator = engine
        .spawn_object(SpawnConfig::new("BSPD").with_position(Vector2::new(0, 0)))
        .expect("build-speed probe spawns");
    let accelerator_index = engine
        .find_object_index(accelerator)
        .expect("build-speed probe stays live");
    assert_eq!(
        engine
            .call_object_function(
                accelerator_index,
                "Refill",
                vec![Value::Object(clonk.as_u64())],
            )
            .expect("the diver refreshes breath before the accelerated build"),
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("builder survives the breath refill")
            .breath,
        700_000,
        "DoBreath retains the former long-build refill seam (C4Script.cpp:508-514)"
    );
    assert_eq!(
        engine
            .call_object_function(
                accelerator_index,
                "Accelerate",
                vec![
                    Value::Object(clonk.as_u64()),
                    Value::Int(accelerated_can_construct),
                ],
            )
            .expect("temporary CanConstruct acceleration succeeds"),
        Value::Bool(true)
    );

    for frame in 1..=ACCELERATED_BUILD_TICKS {
        engine
            .tick_without_snapshot()
            .expect("accelerated Build frames advance");
        let site_state = engine.object_snapshot(site).expect("site survives");
        assert_eq!(
            site_state.construction,
            FULL_CON / 100 + STOCK_BUILD_TICKS * STOCK_CON_PER_TICK + frame * accelerated_delta,
            "accelerated frame {frame}: DoCon follows the temporary physical"
        );
        let builder_state = engine.object_snapshot(clonk).expect("builder survives");
        assert_eq!(
            builder_state.action.name, "Build",
            "accelerated frame {frame}: the underwater Build action persists"
        );
        assert_eq!(
            site_state.components.get("GCOR").copied().unwrap_or(0),
            6,
            "accelerated frame {frame}: transferred GCOR remain on the site"
        );
        assert_eq!(
            site_state.components.get("GLAS").copied().unwrap_or(0),
            2,
            "accelerated frame {frame}: transferred GLAS remain on the site"
        );
        assert_eq!(
            builder_state.contents.len(),
            0,
            "accelerated frame {frame}: the consumed inventory stays empty"
        );
    }

    // Completion: Con caps at FullCon on the exact frame, PSF_Completion +
    // PSF_Initialize fire, and OCF swaps Construct for FullCon
    // (C4Object.cpp:1489-1516,1758-1767).
    let complete = engine.object_snapshot(site).expect("completed site");
    assert_eq!(complete.construction, FULL_CON);
    // DoCon anchors straight-con growth on the bottom edge while ordinary
    // physics settles the growing structure into the chunky floor; the
    // exact settle drift belongs to the movement suites, but the finished
    // 80px building must still stand on the same support stretch.
    assert!(
        (complete.position.x - 721).abs() <= 8 && (542..=548).contains(&(complete.position.y + 40)),
        "the finished PWR2 stands on the surveyed floor: {:?}",
        complete.position
    );
    assert_eq!(
        complete.ocf & 0x2,
        0,
        "a finished building loses OCF_Construct"
    );
    assert_ne!(
        complete.ocf & 0x40,
        0,
        "a finished building gains OCF_FullCon"
    );
    assert_eq!(complete.components.get("GCOR"), Some(&6));
    assert_eq!(complete.components.get("GLAS"), Some(&2));

    // The builder stops: DFA_BUILD sees the complete target, ObjectComStop
    // ends the action, and C4Command::Build finishes without queueing
    // C4CMD_Energy for the power *generator* (C4Object.cpp:5033-5045;
    // C4Command.cpp:838-880).
    for _ in 0..5 {
        engine.tick_without_snapshot().expect("builder stops");
    }
    let builder = engine.object_snapshot(clonk).expect("builder lives");
    assert_ne!(builder.action.name, "Build");
    assert!(
        builder.command_queue.is_empty(),
        "the finished Build command leaves no follow-up commands: {:?}",
        builder.command_queue
    );
}

/// Landscape + ConstructionCheck survey pinned against the native C++
/// binary (arm64 build, LC_PIN_SEED=0, probe copy of Deep.c4s, 2026-07-21).
/// Every row is `(x, first solid y from the top, FDR2/PWR2/DRSC site checks
/// at that floor)` captured from the shipped `CreateConstruction(id, x, y,
/// -1, 1, 0, 1)` script path. Static-map ChunkOZoom jitter, GBackSolid
/// density reads, and all three ConstructionCheck predicates must stay
/// byte-identical (C4Landscape.cpp:337-380,2125-2169).
#[test]
fn deep_sea_construction_site_survey_matches_cpp_oracle() {
    const CPP_ORACLE: &[(i32, i32, i32, i32, i32)] = &[
        (60, 67, 0, 0, 0),
        (70, 67, 0, 0, 0),
        (80, 74, 0, 0, 0),
        (90, 114, 0, 0, 0),
        (100, 117, 0, 0, 0),
        (110, 135, 0, 0, 0),
        (120, 135, 0, 0, 0),
        (130, 145, 0, 0, 0),
        (140, 152, 0, 0, 0),
        (150, 151, 0, 0, 0),
        (160, 164, 0, 0, 0),
        (170, 167, 0, 0, 0),
        (180, 167, 0, 0, 0),
        (190, 167, 0, 1, 1),
        (200, 165, 1, 1, 1),
        (210, 167, 1, 1, 1),
        (220, 167, 1, 1, 1),
        (230, 163, 0, 0, 0),
        (240, 168, 1, 1, 1),
        (250, 167, 1, 1, 1),
        (260, 165, 1, 1, 1),
        (270, 166, 1, 1, 1),
        (280, 167, 1, 1, 1),
        (290, 168, 0, 0, 0),
        (300, 164, 0, 0, 0),
        (310, 168, 0, 0, 0),
        (320, 153, 1, 1, 1),
        (330, 149, 1, 1, 1),
        (340, 149, 1, 1, 1),
        (350, 152, 0, 1, 0),
        (360, 150, 0, 0, 0),
        (370, 149, 0, 0, 0),
        (380, 152, 0, 0, 0),
        (390, 132, 1, 1, 1),
        (400, 131, 1, 1, 1),
        (410, 133, 0, 0, 0),
        (420, 135, 0, 0, 0),
        (430, 126, 0, 0, 0),
        (440, 118, 1, 1, 1),
        (450, 113, 0, 0, 0),
        (460, 118, 0, 0, 0),
        (470, 117, 1, 0, 0),
        (480, 115, 1, 1, 1),
        (490, 105, 0, 0, 0),
        (500, 99, 0, 0, 0),
        (510, 99, 0, 0, 0),
        (520, 114, 1, 1, 1),
        (530, 117, 1, 0, 0),
        (540, 117, 1, 1, 1),
        (550, 123, 0, 0, 0),
        (560, 150, 0, 0, 0),
        (570, 149, 0, 0, 0),
        (580, 150, 0, 0, 0),
        (590, 152, 0, 0, 0),
        (600, 149, 1, 1, 1),
        (610, 149, 1, 1, 1),
        (620, 167, 0, 0, 0),
        (630, 168, 0, 0, 0),
        (640, 164, 0, 0, 0),
        (650, 171, 0, 0, 0),
        (660, 185, 0, 0, 0),
        (670, 181, 0, 0, 0),
        (680, 151, 0, 0, 0),
        (690, 152, 0, 0, 0),
        (700, 132, 0, 0, 0),
        (710, 133, 0, 0, 0),
        (720, 134, 0, 0, 0),
        (730, 135, 0, 1, 0),
        (740, 140, 0, 1, 1),
        (750, 168, 0, 0, 0),
        (760, 167, 0, 0, 0),
        (770, 207, 0, 0, 0),
        (780, 219, 0, 0, 0),
        (790, 593, 0, 0, 0),
        (800, 596, 0, 0, 0),
        (810, 611, 0, 0, 0),
        (820, 613, 0, 0, 0),
        (830, 625, 0, 0, 0),
        (840, 628, 0, 0, 0),
        (850, 626, 0, 1, 0),
        (860, 637, 0, 0, 0),
        (870, 644, 0, 0, 0),
        (880, 643, 0, 0, 0),
        (890, 640, 0, 0, 0),
        (900, 643, 0, 0, 0),
        (910, 633, 0, 1, 1),
        (920, 626, 1, 1, 1),
        (930, 625, 1, 1, 1),
        (940, 628, 0, 0, 0),
        (950, 627, 0, 0, 0),
        (960, 617, 1, 1, 1),
        (970, 609, 1, 1, 1),
        (980, 611, 1, 1, 1),
        (990, 607, 0, 1, 1),
        (1000, 609, 1, 1, 1),
        (1010, 610, 1, 1, 1),
        (1020, 608, 1, 1, 1),
        (1030, 608, 1, 1, 1),
        (1040, 610, 1, 1, 1),
        (1050, 611, 1, 1, 1),
        (1060, 607, 1, 1, 1),
        (1070, 609, 1, 1, 1),
        (1080, 611, 1, 1, 1),
        (1090, 608, 1, 1, 1),
        (1100, 608, 1, 1, 1),
        (1110, 610, 1, 1, 1),
        (1120, 611, 1, 1, 1),
        (1130, 617, 0, 0, 0),
        (1140, 609, 1, 1, 1),
        (1150, 611, 1, 1, 1),
        (1160, 607, 1, 1, 1),
        (1170, 609, 1, 1, 1),
        (1180, 610, 1, 1, 1),
        (1190, 608, 1, 1, 1),
        (1200, 608, 1, 1, 1),
        (1210, 610, 1, 1, 1),
        (1220, 611, 1, 1, 1),
        (1230, 607, 1, 1, 1),
        (1240, 609, 1, 1, 1),
        (1250, 611, 0, 0, 0),
        (1260, 608, 0, 1, 0),
        (1270, 608, 0, 0, 0),
        (1280, 604, 0, 0, 0),
        (1290, 594, 0, 1, 0),
        (1300, 593, 0, 0, 0),
        (1310, 593, 0, 0, 0),
        (1320, 586, 0, 0, 0),
        (1330, 576, 1, 1, 1),
        (1340, 575, 1, 1, 1),
        (1350, 572, 0, 0, 0),
        (1360, 575, 1, 1, 1),
        (1370, 575, 1, 1, 1),
        (1380, 575, 1, 1, 1),
        (1390, 573, 1, 1, 1),
        (1400, 575, 0, 0, 0),
        (1410, 575, 0, 0, 0),
        (1420, 565, 1, 1, 1),
        (1430, 559, 1, 1, 1),
        (1440, 560, 1, 1, 1),
        (1450, 557, 1, 1, 1),
        (1460, 557, 1, 1, 1),
        (1470, 561, 1, 1, 1),
        (1480, 558, 1, 1, 1),
        (1490, 557, 1, 1, 1),
        (1500, 560, 1, 1, 1),
        (1510, 559, 1, 1, 1),
        (1520, 557, 1, 1, 1),
        (1530, 557, 1, 1, 1),
        (1540, 560, 1, 1, 1),
        (1550, 558, 1, 1, 1),
        (1560, 557, 1, 1, 1),
        (1570, 560, 1, 1, 1),
        (1580, 558, 1, 1, 1),
        (1590, 557, 1, 1, 1),
        (1600, 558, 0, 1, 1),
        (1610, 560, 0, 0, 0),
        (1620, 557, 0, 0, 0),
        (1630, 557, 0, 0, 0),
        (1640, 535, 0, 0, 0),
        (1650, 526, 0, 0, 0),
        (1660, 525, 0, 0, 0),
        (1670, 529, 0, 0, 0),
        (1680, 543, 0, 0, 0),
        (1690, 539, 1, 0, 1),
        (1700, 542, 0, 0, 0),
        (1710, 560, 0, 0, 0),
        (1720, 558, 0, 0, 0),
        (1730, 550, 0, 0, 0),
        (1740, 542, 0, 0, 0),
        (1750, 491, 0, 0, 0),
        (1760, 480, 0, 0, 0),
        (1770, 472, 0, 0, 0),
        (1780, 361, 0, 0, 0),
        (1790, 356, 0, 0, 0),
        (1800, 288, 1, 1, 1),
        (1810, 289, 0, 1, 0),
        (1820, 286, 0, 0, 0),
        (1830, 285, 0, 0, 0),
        (1840, 288, 0, 0, 0),
        (1850, 270, 1, 1, 1),
        (1860, 267, 0, 1, 1),
        (1870, 269, 0, 0, 0),
        (1880, 271, 0, 0, 0),
        (1890, 253, 0, 1, 0),
        (1900, 253, 0, 0, 0),
        (1910, 250, 0, 0, 0),
        (1920, 253, 0, 0, 0),
        (1930, 226, 0, 1, 1),
        (1940, 218, 0, 0, 0),
        (1950, 217, 0, 0, 0),
        (1960, 220, 0, 1, 0),
        (1970, 219, 0, 0, 0),
        (1980, 263, 0, 0, 0),
        (1990, 254, 0, 0, 0),
        (2000, 253, 0, 0, 0),
        (2010, 251, 0, 0, 0),
        (2020, 252, 0, 0, 0),
        (2030, 263, 0, 0, 0),
        (2040, 336, 0, 0, 0),
        (2050, 336, 0, 0, 0),
        (2060, 340, 0, 0, 0),
        (2070, 557, 0, 0, 0),
        (2080, 564, 0, 0, 0),
        (2090, 575, 0, 0, 0),
        (2100, 628, 0, 0, 0),
        (2110, 626, 0, 0, 0),
        (2120, 601, 0, 0, 0),
        (2130, 590, 0, 0, 0),
        (2140, 593, 1, 0, 1),
        (2150, 593, 0, 0, 0),
        (2160, 596, 0, 1, 0),
        (2170, 628, 0, 0, 0),
        (2180, 625, 0, 0, 0),
        (2190, 625, 0, 0, 0),
        (2200, 628, 0, 0, 0),
        (2210, 626, 1, 1, 1),
        (2220, 658, 0, 0, 0),
        (2230, 661, 0, 0, 0),
        (2240, 661, 0, 0, 0),
        (2250, 658, 0, 0, 0),
        (2260, 661, 0, 0, 0),
        (2270, 693, 0, 0, 0),
        (2280, 694, 0, 0, 0),
        (2290, 596, 0, 0, 0),
        (2300, 590, 0, 0, 0),
        (2310, 593, 0, 0, 0),
        (2320, 567, 0, 0, 0),
        (2330, 540, 0, 0, 0),
        (2340, 540, 0, 0, 0),
        (2350, 321, 0, 1, 0),
        (2360, 321, 0, 0, 0),
        (2370, 317, 0, 0, 0),
        (2380, 322, 0, 0, 0),
        (2390, 293, 0, 0, 0),
        (2400, 286, 0, 0, 0),
        (2410, 285, 0, 0, 0),
        (2420, 300, 0, 0, 0),
        (2430, 219, 1, 1, 1),
        (2440, 220, 1, 1, 1),
        (2450, 218, 1, 1, 1),
        (2460, 217, 1, 1, 1),
        (2470, 220, 0, 1, 0),
        (2480, 219, 0, 0, 0),
        (2490, 217, 0, 0, 0),
        (2500, 217, 0, 0, 0),
        (2510, 220, 0, 0, 0),
        (2520, 185, 1, 1, 1),
        (2530, 185, 1, 1, 1),
        (2540, 202, 0, 0, 0),
        (2550, 186, 1, 1, 1),
        (2560, 185, 1, 1, 1),
        (2570, 184, 1, 1, 1),
        (2580, 184, 1, 1, 1),
        (2590, 185, 1, 1, 1),
        (2600, 185, 1, 1, 1),
        (2610, 181, 0, 0, 0),
        (2620, 188, 1, 1, 1),
        (2630, 233, 0, 0, 0),
        (2640, 235, 0, 0, 0),
        (2650, 235, 0, 0, 0),
        (2660, 271, 0, 0, 0),
        (2670, 271, 0, 0, 0),
        (2680, 278, 0, 0, 0),
        (2690, 304, 0, 0, 0),
        (2700, 303, 0, 0, 0),
        (2710, 464, 0, 0, 0),
        (2720, 475, 0, 0, 0),
    ];

    let mut engine = load_installed_scenario("FarWorlds.c4f/Deep.c4s", 0);
    let _owner = join_local_player(&mut engine, "Deep Sea oracle survey");
    engine
        .register_definition(
            clonk_engine::Definition::from_script(
                "PRBE",
                "Construction probe",
                r#"#strict 2
public func ProbeFloor(int x)
{
    var y = 0;
    while (!GBackSolid(x, y) && y < LandscapeHeight()) ++y;
    return y;
}
public func ProbeSite(id def, int x, int y)
{
    var s = CreateConstruction(def, x, y, -1, 1, 0, 1);
    if (s) { RemoveObject(s); return 1; }
    return 0;
}
"#,
            )
            .expect("probe compiles"),
        )
        .expect("probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("PRBE").with_position(Vector2::new(0, 0)))
        .expect("probe spawns");
    let probe_index = engine.find_object_index(probe).expect("probe index");

    // Below #strict 3 the literal `0` is nil (C++ C4Aul semantics), so
    // ProbeSite's miss arm surfaces as Nil; the C++ log's `%d` formats the
    // same nil as 0.
    let as_int = |value: Value| match value {
        Value::Int(v) => v,
        Value::Nil => 0,
        other => panic!("int expected: {other:?}"),
    };

    let mut mismatches = Vec::new();
    for &(x, floor, fdr2, pwr2, drsc) in CPP_ORACLE {
        let rust_floor = as_int(
            engine
                .call_object_function(probe_index, "ProbeFloor", vec![Value::Int(x)])
                .expect("floor probe runs"),
        );
        let mut site = |def: &str| {
            as_int(
                engine
                    .call_object_function(
                        probe_index,
                        "ProbeSite",
                        vec![
                            Value::C4Id(def.to_string()),
                            Value::Int(x),
                            Value::Int(rust_floor),
                        ],
                    )
                    .expect("site probe runs"),
            )
        };
        let row = (x, rust_floor, site("FDR2"), site("PWR2"), site("DRSC"));
        if row != (x, floor, fdr2, pwr2, drsc) {
            mismatches.push((row, (x, floor, fdr2, pwr2, drsc)));
        }
    }
    assert!(
        mismatches.is_empty(),
        "survey diverged from the C++ oracle (rust, cpp): {mismatches:?}"
    );
}
