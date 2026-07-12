#![allow(dead_code)]

use crate::support::real_scenario::{join_local_player, load_installed_scenario, load_tutorial};
use lc_engine::{
    math, EffectVarValue, ObjectId, COM_RIGHT, COM_THROW, FULL_CON, OWNER_NONE,
};
use lc_script::Value;

#[test]
fn tutorial_harness_boots_the_installed_cpp_global_script_layer() {
    let engine = load_tutorial(2, 0);

    // C++ loads planet/System.c4g before definitions and the scenario
    // (C4Game.cpp:2591-2607,2764-2788). Helpers.c supplies both functions
    // used by Tutorial02 and BALN; a direct Scenario::apply fixture does not.
    for function in ["Schedule", "ScheduleCall", "FxIntScheduleCallTimer"] {
        assert!(
            engine.debug_global_has_function(function),
            "virtual play must expose planet global `{function}`"
        );
    }
    assert_eq!(
        engine.materials().len(),
        21,
        "virtual play must use the installed Material.c4g library"
    );
}

#[test]
fn dragon_rock_mage_choice_redefines_the_real_knight_and_transfers_its_flag() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player(&mut engine, "Dragon Rock character parity");
    let knight = engine
        .crew_cursor(owner)
        .expect("Dragon Rock joins the Scenario.txt KNIG");

    // Choose normal difficulty through the real KNIG object menu. The shipped
    // InitializePlayer2 then creates FLAG in that KNIG and opens the shipped
    // KNIG/MAGE selection menu (Drachenfels.c4s/Script.c:86-103,112-128).
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("choose normal difficulty");
    let flag = engine
        .object_snapshot(knight)
        .and_then(|knight| {
            knight.contents.into_iter().find(|item| {
                engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "FLAG")
            })
        })
        .expect("normal difficulty gives the real KNIG a FLAG");
    let (_, choice) = engine
        .cursor_object_menu(owner)
        .expect("normal difficulty opens the real character menu");
    assert_eq!(
        choice
            .items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["KNIG", "MAGE"]
    );

    engine
        .player_in_com(owner, COM_RIGHT, 0)
        .expect("select MAGE");
    assert_eq!(
        engine
            .cursor_object_menu(owner)
            .expect("character menu remains open")
            .1
            .selection,
        1,
        "the physical Right control selects MAGE"
    );
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("execute Redefine3(MAGE)");

    // Redefine3 creates MAGE, immediately calls pNew->GrabContents(this()),
    // copies the live state, installs it as crew/cursor, then removes KNIG
    // (Drachenfels.c4s/Script.c:150-178). FnGrabContents is an engine-global
    // function found after MAGE's own script and transfers a copied contents
    // list through ordinary Enter calls (C4Aul.cpp:130-148;
    // C4Script.cpp:320-327; C4Object.cpp:6162-6171).
    let mage = engine
        .crew_cursor(owner)
        .expect("Redefine3 leaves a live crew cursor");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("replacement crew remains live")
            .definition_id,
        "MAGE"
    );
    assert!(
        !engine
            .object_snapshot(knight)
            .expect("the removal stays observable until cleanup")
            .status
            .is_active(),
        "Redefine3 marks the old KNIG deleted immediately"
    );
    assert_eq!(
        engine
            .object_snapshot(flag)
            .expect("FLAG survives the character replacement")
            .container,
        Some(mage),
        "MAGE receives KNIG's contents through the real GrabContents call"
    );
}

#[test]
fn dragon_rock_initialize_player_grants_both_plan_knowledge_sets() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player(&mut engine, "Dragon Rock knowledge parity");

    // Dragon Rock calls WPPL->SetKnowledge(iPlr) and
    // CPPL->SetKnowledge(iPlr) before continuing player initialization
    // (Drachenfels.c4s/Script.c:63-103). Both shipped plan scripts use the
    // two-argument SetPlrKnowledge(player, id) form throughout
    // (Weapons/Plans/Script.c:10-65; Castle/Plans/Script.c:22-70).
    // C4Aul pads the omitted third slot with nil, which converts to false,
    // so FnSetPlrKnowledge validates the definition and grants count one
    // (C4AulParse.cpp:2339-2345; C4Value.h:161,325-330;
    // C4Script.cpp:2636-2650; C4IDList.cpp:85-103).
    // The persisted result is the union with Scenario.txt's positive
    // Player1 Knowledge entries. C4Player first ConsolidateValids that list
    // (C4Player.cpp:697-706; C4IDList.cpp:175-184), and each plan grant also
    // rejects an unloaded definition (C4Script.cpp:2646-2649). Thus PNON
    // from Scenario.txt and CODH from WPPL are deliberately absent.
    let expected = [
        "ADM1", "ADM3", "ANVL", "ARCH", "ARMR", "ARWP", "AXE1", "BALN", "BANP",
        "BARL", "BAS7", "BED1", "BLMP", "BOW1", "BRDG", "BRED", "BWRC", "CANN",
        "CATA", "CHEM", "CLD1", "CNDL", "CNKT", "COKI", "CPAL", "CPEL", "CPH1",
        "CPHC", "CPKT", "CPOF", "CPR1", "CPR2", "CPT1", "CPT2", "CPT3", "CPT4",
        "CPTL", "CPTR", "CPW1", "CPW2", "CPWK", "CPWL", "CPWR", "DCO3", "DCO4",
        "DOGH", "DPOT", "DRCK", "EFLN", "ELEV", "FARP", "FBMP", "FDRS", "FLNT",
        "FNDR", "FRGE", "GUNP", "HUT1", "HUT2", "HUT3", "KSDL", "LANC", "LNKT",
        "LORY", "OVEN", "PAL2", "PALS", "PFIR", "PHEA", "POWR", "PSTO", "PUMP",
        "RSRC", "SAWM", "SFLN", "SHIE", "SHRC", "SLBT", "SPER", "SPRC", "STFN",
        "SWOR", "SWRC", "TABL", "TENP", "TFLN", "THRN", "TWR2", "WDBR", "WGTW",
        "WMIL", "WODC", "WRKS", "WTWR", "WZKP", "XARP", "XBOW",
    ];
    let player = engine.player(owner).expect("joined player remains live");
    let mut actual = player
        .knowledge()
        .map(|definition| definition.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    assert_eq!(actual, expected, "both shipped plan sets persist exactly");

    // The difficulty menu is created after both definition calls. Its
    // presence proves InitializePlayer ran past every omitted remove flag
    // instead of aborting at the original argument-count warning.
    let (_, menu) = engine
        .cursor_object_menu(owner)
        .expect("InitializePlayer continues into the difficulty menu");
    assert_eq!(
        menu.items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["WIPF", "MONS"]
    );
}

#[test]
fn dragon_rock_real_schedule_enables_and_forces_player_fog_of_war() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player(&mut engine, "Dragon Rock fog parity");
    let player = engine.player(owner).expect("joined player remains live");
    assert!(!player.fog_of_war());
    assert!(!player.force_fog_of_war());

    // The shipped InitializePlayer schedules SetFoW through the installed
    // Helpers.c IntSchedule effect (Drachenfels.c4s/Script.c:56-71;
    // planet/System.c4g/Helpers.c:110-132). A failed eval aborts before the
    // callback's -1 return, so the one-shot effect staying alive would expose
    // the original unknown-function warning.
    let schedules = engine
        .global_effects()
        .iter()
        .filter(|effect| effect.name == "IntSchedule")
        .collect::<Vec<_>>();
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0].interval, 1);
    assert_eq!(
        schedules[0].var(0),
        EffectVarValue::String(format!("SetFoW(true, {owner})"))
    );
    assert_eq!(schedules[0].var(1), EffectVarValue::Int(1));

    engine
        .tick()
        .expect("the real IntSchedule callback evaluates SetFoW");

    // FnSetFoW accepts the live player and calls C4Player::SetFoW
    // (C4Script.cpp:3671-3678), which enables both the active FoW flag and
    // its initialized state and forces the script choice
    // (C4Player.cpp:815-824). The Rust save state exposes the two persistent
    // fields FogOfWar and ForceFogOfWar (C4Player.cpp:1580-1581).
    assert!(
        engine
            .global_effects()
            .iter()
            .all(|effect| effect.name != "IntSchedule"),
        "successful eval reaches Helpers.c's one-shot kill return"
    );
    let player = engine.player(owner).expect("joined player remains live");
    assert!(player.fog_of_war());
    assert!(player.force_fog_of_war());
    let persisted = player.to_state();
    assert!(persisted.fog_of_war);
    assert!(persisted.force_fog_of_war);
}

#[test]
fn dragon_rock_objects_keep_their_multidirectional_action_rows() {
    let engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);

    // C4Action::CompileFunc reads Dir as an unrestricted int32
    // (C4Action.cpp:45-54). Loading resolves the action name without replacing
    // that field (C4Object.cpp:2867-2876), then UpdateFlipDir derives DrawDir
    // from it (C4GameObjects.cpp:665-674; C4Object.cpp:404-430).
    for (number, definition, action, direction) in [
        (294, "BANR", "FlyBack", 13),
        (293, "BANR", "FlyBack", 13),
        (292, "BANR", "Fly", 13),
        (290, "BANR", "Fly", 13),
        (1159, "FLAG", "FlyBase", 4),
        (4447, "MUSH", "Exist", 3),
        (548, "BANR", "Fly", 7),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock object #{number} loads"));
        assert_eq!(object.definition_id, definition, "object #{number}");
        assert_eq!(object.action.name, action, "object #{number}");
        assert_eq!(
            object.direction.to_script_value(),
            direction,
            "object #{number} must retain its Objects.txt Dir"
        );
    }

    // These are valid graphic rows, not malformed two-way facing values.
    // BANR's FlipDir=7 maps raw 13 to row 0 mirrored and raw 7 to row 6
    // mirrored; FLAG and MUSH draw their raw rows directly.
    for (definition, action, directions, flip_dir) in [
        ("BANR", "Fly", 14, Some(7)),
        ("BANR", "FlyBack", 14, Some(7)),
        ("FLAG", "FlyBase", 9, None),
        ("MUSH", "Exist", 4, None),
    ] {
        let graphics = engine
            .definition_action_graphics(definition)
            .unwrap_or_else(|| panic!("{definition} action graphics load"));
        let graphics = graphics
            .get(action)
            .unwrap_or_else(|| panic!("{definition}::{action} action loads"));
        assert_eq!(graphics.directions, directions);
        assert_eq!(graphics.flip_dir, flip_dir);
    }
}

#[test]
fn dragon_rock_objects_restore_serialized_c4id_named_locals() {
    let engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);

    // GetC4VID assigns uppercase `I` to C4V_C4ID (C4Value.cpp:368-410).
    // C4Value::CompileFunc persists the ID's signed 32-bit payload verbatim
    // (C4Value.cpp:717-766), and C4ID converts that payload to its four
    // little-endian text bytes (C4Id.cpp:26-52). These are IDs—not integer or
    // object-reference locals—so definition lookup must not gate restoration.
    for (number, definition) in [
        (1758, "MAGE"),
        (1781, "SCRL"),
        (3714, "SCRL"),
        (5064, "SCRL"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock object #{number} loads"));
        assert_eq!(object.definition_id, definition);
        assert!(object.status.is_active(), "object #{number} is live");
    }

    for (number, local, id) in [
        (1758, "idLastSpell", "EH69"),
        (1781, "idSpell", "MFBL"),
        (3714, "idSpell", "ELX2"),
        (5064, "idSpell", "MFRB"),
        (4410, "idLastSpell", "_MWP"),
        (3886, "idShield", "SHIE"),
        (3883, "idShield", "SHIE"),
        (3818, "idShield", "SHIE"),
        (2541, "idShield", "SHIE"),
        (2555, "idShield", "SHIE"),
        (1128, "idShield", "SHIE"),
        (1128, "ai_idFirstEncounterCB", "BAND"),
        (1779, "idSpell", "XCRS"),
        (1780, "idSpell", "XCRS"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock object #{number} loads"));
        assert_eq!(
            object.local_vars.get(local),
            Some(&Value::C4Id(id.to_string())),
            "object #{number} local {local}"
        );
    }

    // C4Value arrays recurse through the same compiler and retain their
    // declared order/size (C4Value.cpp:801-805). Cover every Dragon Rock
    // ai_aSpells array that generated the original per-element warnings.
    for (number, ids) in [
        (
            3893,
            &[
                "GGHG", "GZ9Z", "ABLA", "MBOT", "MBLS", "MFRB", "MFBL", "FRFS",
                "MBRG", "EH69", "CMFG",
            ][..],
        ),
        (
            4410,
            &["GZ9Z", "CMFG", "MFFW", "MBRG", "EH69", "EXTG", "ELX2"][..],
        ),
        (
            2550,
            &[
                "CMFG", "MFFW", "ABLA", "MBRG", "EXTG", "MGHL", "MLGT", "ETFL",
                "MFRB", "MDBT", "MFBL", "RUND", "MBLS", "CPAN", "CFAL", "MGBW",
                "MICS", "ELX1", "GZ9Z",
            ][..],
        ),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock spellcaster #{number} loads"));
        let expected = Value::Array(
            ids.iter()
                .map(|id| Value::C4Id((*id).to_string()))
                .collect(),
        );
        assert_eq!(
            object.local_vars.get("ai_aSpells"),
            Some(&expected),
            "object #{number} spell order"
        );
    }
}

#[test]
fn dragon_rock_scroll_transfer_zone_callbacks_persist_cpp_names() {
    let engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);

    // C4Game::Synchronize reaches UpdateTransferZone through
    // TransferZones::Synchronize and Game.Objects.UpdateTransferZones
    // (C4Game.cpp:3727-3729; C4TransferZone.cpp:110-114;
    // C4ObjectList.cpp:734-739). Game.Objects excludes Status=2 objects
    // (C4GameObjects.cpp:54-58), so only Dragon Rock's three active SCRL
    // objects execute the shipped UpdateName call (Scroll.c4d/Script.c:
    // 141-153); the two inactive scrolls retain their serialized names.
    //
    // SetName resolves to the engine-global function after the definition
    // scope (C4Aul.cpp:130-148). With no explicit target it writes the
    // calling object, and GetName then observes CustomName before the
    // definition fallback (C4Script.cpp:993-1005,1008-1060;
    // C4Object.cpp:2103-2115). Because SetName is UpdateName's final call,
    // these changed names prove every relevant shipped callback completed.
    for (number, expected_name) in [
        (1781, "Scroll: Fiery lump"),
        (3714, "Scroll: Recovery"),
        (5064, "Scroll: Fireball"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("active Dragon Rock scroll #{number} loads"));
        assert!(object.status.is_active(), "scroll #{number} is active");
        assert_eq!(
            object.custom_name.as_deref(),
            Some(expected_name),
            "scroll #{number} persists UpdateName's SetName result"
        );
    }

    for (number, saved_name) in [
        (1779, "Schriftrolle: Reinkarnation"),
        (1780, "Schriftrolle: Reinkarnation"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("inactive Dragon Rock scroll #{number} loads"));
        assert!(!object.status.is_active(), "scroll #{number} is inactive");
        assert_eq!(
            object.custom_name.as_deref(),
            Some(saved_name),
            "inactive scroll #{number} is outside C++ Game.Objects broadcast"
        );
    }
}

#[test]
fn dragon_rock_object_lookup_carries_script1_state_into_script3() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    join_local_player(&mut engine, "Dragon Rock intro object parity");

    // InitializePlayer starts the ordinary C4GameScriptHost counter. Its
    // every-tenth-frame Execute post-increments Counter before calling
    // Script%d (C4ScriptHost.cpp:222-230), so Script1 runs on frame 20.
    for _ in 0..20 {
        engine.tick().expect("Dragon Rock reaches shipped Script1");
    }

    // GetEndboss calls Object(EVIL_MAGE_OBJ), where EVIL_MAGE_OBJ is the
    // Objects.txt Number 1758 (Drachenfels.c4s/Script.c:10,194-198,243-279).
    // FnObject resolves that exact number through SafeObjectPointer
    // (C4Script.cpp:3327-3330), whose Game.Objects override searches both
    // active and inactive lists and whose final guard rejects only Status=0
    // (C4GameObjects.cpp:270-276; C4ObjectList.cpp:544-557).
    let globals = &engine.snapshot().script_globals.named;
    for (name, number) in [
        ("g_pEndboss", 1758),
        ("g_pDragon", 202),
        ("g_pKing", 5129),
        ("g_pPrincess", 1777),
    ] {
        assert_eq!(
            globals.get(name),
            Some(&Value::Object(number)),
            "Script1 persists {name} for later callbacks"
        );
    }
    for (number, definition) in [(1758, "MAGE"), (202, "DRGN")] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Script1 target #{number} remains live"));
        assert_eq!(object.definition_id, definition);
        assert_eq!(object.position.x, 1000, "Script1 moved #{number}");
        assert_eq!(object.position.y, 800, "Script1 moved #{number}");
    }

    // Script3 runs on frame 40 and dereferences the Script1 result through
    // g_pDragon->IntroControl(400, 1050) (Drachenfels.c4s/Script.c:281-284).
    // The shipped DRGN append writes all three globals before returning true
    // (Drachenfels.c4s/System.c4g/Dragon.c:17,26-32). If Script1 aborted at
    // Object(), this is the original "target is zero" failure instead.
    for _ in 0..20 {
        engine.tick().expect("Dragon Rock reaches shipped Script3");
    }
    let globals = &engine.snapshot().script_globals.named;
    assert_eq!(globals.get("DRGN_ctrl_tx"), Some(&Value::Int(400)));
    assert_eq!(globals.get("DRGN_ctrl_ty"), Some(&Value::Int(1050)));
    assert!(matches!(
        globals.get("DRGN_ctrl_stop"),
        None | Some(Value::Nil) | Some(Value::Bool(false)) | Some(Value::Int(0))
    ));
}

#[test]
fn dragon_rock_script25_casts_cpp_sparks_and_completes_intro_step() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    join_local_player(&mut engine, "Dragon Rock CastObjects parity");

    // Let the shipped counter reach Script15's pause, then resume it through
    // the real dragon-arrival callback (Drachenfels.c4s/Script.c:286-294).
    // Counter 20 is intentionally empty; Script21 runs at frame 180 and
    // Script25 naturally runs at frame 220.
    for _ in 0..160 {
        engine.tick().expect("Dragon Rock reaches Script15 pause");
    }
    engine
        .call_scenario_script_function("OnDragonReachTarget", Vec::new())
        .expect("real dragon arrival resumes the intro counter");
    for _ in 0..59 {
        engine.tick().expect("Dragon Rock approaches Script25");
    }
    assert_eq!(engine.snapshot().frame, 219);

    let princess_before = engine
        .object_snapshot(ObjectId::new(1777))
        .expect("Dragon Rock princess remains live before Script25");
    let old_sparks = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "SPRK")
        .map(|object| object.id)
        .collect::<Vec<_>>();

    let frame = engine.tick().expect("natural Script25 callback succeeds");
    assert_eq!(frame.frame, 220);
    let sparks = frame
        .objects
        .iter()
        .filter(|object| object.definition_id == "SPRK" && !old_sparks.contains(&object.id))
        .collect::<Vec<_>>();

    // Script25 calls the shipped Sparkle(5, fx, fy), which casts
    // 5/3+2 == three SPRK objects (Script.c:307-320;
    // Objects.c4d/Effects.c4d/Spark.c4d/Script.c:25-28). FnCastObjects
    // applies scenario-global coordinates and NO_OWNER/NO_OWNER, while
    // C4Game::CastObjects samples rdir, ydir, xdir, rotation in that exact
    // order for every object (C4Script.cpp:2476-2480;
    // C4Game.cpp:1727-1739). SPRK is not rotateable, so Init clears the
    // sampled rotation/rdir but preserves both FIXED10 velocity components
    // (C4Object.cpp:153-187).
    assert_eq!(sparks.len(), 3, "Sparkle(5) casts exactly three sparks");
    let allowed_velocity = (-5..=5).map(math::fixed10).collect::<Vec<_>>();
    for spark in sparks {
        assert_eq!(spark.owner, OWNER_NONE);
        assert_eq!(spark.controller, OWNER_NONE);
        assert_eq!(spark.position.x, princess_before.position.x);
        assert_eq!(spark.position.y, princess_before.position.y - 3);
        let fixed_velocity = spark.fixed_velocity.unwrap_or_else(|| {
            math::FixedVec2::from_ints(spark.velocity.x, spark.velocity.y)
        });
        assert!(allowed_velocity.contains(&fixed_velocity.x));
        assert!(allowed_velocity.contains(&fixed_velocity.y));
        assert_eq!(spark.rotation, 0);
        assert_eq!(spark.rotation_velocity, None);
        assert_eq!(spark.action.name, "Sparkle");
        assert!(
            (FULL_CON..FULL_CON * 2).contains(&spark.construction),
            "SPRK Completion applies DoCon(Random(100)) after initial FullCon; got {}",
            spark.construction
        );
    }

    // These statements follow Sparkle in Script25. They prove CastObjects
    // returned normally instead of aborting the callback at the old unknown
    // function warning.
    let princess = engine
        .object_snapshot(ObjectId::new(1777))
        .expect("princess survives Script25");
    assert_eq!((princess.position.x, princess.position.y), (2145, 485));
    assert_eq!(princess.action.name, "Walk");
    assert_eq!(princess.direction.to_script_value(), 0);
    let endboss = engine
        .object_snapshot(ObjectId::new(1758))
        .expect("endboss survives Script25");
    assert_eq!(endboss.action.name, "RideMagic");
    assert_eq!(endboss.action.target, Some(ObjectId::new(202)));
}
