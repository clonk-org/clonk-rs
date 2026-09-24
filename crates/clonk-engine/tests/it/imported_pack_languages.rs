//! Player-visible text of the imported ClonkMars and E.P.I.C. packs in each
//! player's language.

use crate::support::real_scenario::load_installed_scenario_in_languages;
use crate::support::EngineTestExt;
use clonk_engine::{Engine, ObjectId, PlayerConfig, SpawnConfig};
use clonk_script::{c4_string_from_bytes, Value};

const PLAYER: i32 = 1;

fn call(engine: &mut Engine, object: ObjectId, function: &str, args: Vec<Value>) {
    let index = engine.test_object_index(object);
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function}: {error}"));
}

/// The engine's text for `text` written in ISO-8859-1, the encoding of the
/// packs' string tables: a script string keeps their bytes.
fn latin1(text: &str) -> String {
    let bytes = text
        .chars()
        .map(|character| u8::try_from(u32::from(character)).expect("ISO-8859-1 text"))
        .collect::<Vec<_>>();
    c4_string_from_bytes(&bytes)
}

/// clonk-org/clonk-rs-content#78: ClonkMars ran words together in its English
/// names. A definition's name is the `Names.txt` line for the player's
/// language (C4Def.cpp:637-638), so English players built a "Materialunit".
#[test]
fn the_mars_material_unit_is_named_in_the_players_language() {
    for (languages, name) in [(["US"], "Material unit"), (["DE"], "Materialeinheit")] {
        let engine =
            load_installed_scenario_in_languages("ClonkMars.c4f/01_Fossae.c4s", 0, &languages);
        assert_eq!(engine.definition_name("UNIT"), Some(name), "{languages:?}");
    }
}

/// clonk-org/clonk-rs-content#78: the ClonkMars power line had only an English
/// string table, so a German player's `$TxtLinebroke$` stayed unsubstituted
/// (C4LangStringTable.cpp:95-99). `Message` takes a `$...$` for a sound name
/// and shows only the text before its first `$` (C4Script.cpp:2420-2430), so a
/// German player whose line broke got an empty message.
#[test]
fn a_broken_mars_power_line_warns_in_the_players_language() {
    for (languages, warning) in [(["US"], "Line broke"), (["DE"], "Leitung gerissen")] {
        let mut engine =
            load_installed_scenario_in_languages("ClonkMars.c4f/01_Fossae.c4s", 0, &languages);
        let line = engine.spawn_test_object(SpawnConfig::new("PWRL"));
        call(&mut engine, line, "LineBreak", vec![Value::Bool(false)]);
        assert!(
            engine.message_line_contains(&latin1(warning)),
            "{languages:?}: {warning}"
        );
    }
}

/// clonk-org/clonk-rs-content#162: E.P.I.C. is written in English, and its
/// toolbelt had only an English string table. A German player's default
/// language sequence is just `DE` (C4Config.cpp:1472-1473), so `$MsgPuton$`
/// stayed unsubstituted and `PlayerMessage` showed only the text before its
/// first `$` (C4Script.cpp:2400-2410): nothing at all.
#[test]
fn an_epic_toolbelt_is_put_on_in_the_players_language() {
    for (languages, announcement) in [
        (["US"], "Toolbelt put on."),
        (["DE"], "Werkzeuggürtel angelegt."),
    ] {
        let mut engine =
            load_installed_scenario_in_languages("E.P.I.C.c4f/Death_Canyon.c4s", 0, &languages);
        engine.register_test_player(PlayerConfig::new(PLAYER, "Toolbelt tester"));
        let clonk = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(PLAYER)
                .with_controller(PLAYER),
        );
        let belt = engine.spawn_test_object(SpawnConfig::new("938Z"));
        call(
            &mut engine,
            belt,
            "Activate",
            vec![Value::Object(clonk.as_u64())],
        );
        assert!(
            engine.message_line_contains(&latin1(announcement)),
            "{languages:?}: {announcement}"
        );
    }
}
