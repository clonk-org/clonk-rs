#![allow(dead_code)]

use crate::support::real_scenario::{join_local_player, load_installed_scenario, load_tutorial};
use lc_engine::{EffectVarValue, ObjectId, COM_RIGHT, COM_THROW};
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
