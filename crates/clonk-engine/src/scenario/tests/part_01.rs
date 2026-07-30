// Contiguous slice 1 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    // C4Game.cpp:2340-2345 + C4Object.cpp:363-386 — a successful reload
    // refreshes every object of that id against the rebuilt definition, and
    // touches nothing else about them.
    #[test]
    fn a_successful_reload_refreshes_live_objects_without_reinitialising_them() {
        let group = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/Objects.c4d/Animals.c4d/Wipf.c4d");
        if !group.is_dir() {
            return;
        }

        let mut engine = crate::Engine::new();
        let mut definition =
            crate::Definition::from_script("WIPF".to_string(), "placeholder".to_string(), "")
                .expect("script definition compiles");
        definition.set_source_path(Some(group.clone()));
        engine
            .register_definition(definition)
            .expect("register definition");
        let object = engine
            .spawn_object(crate::SpawnConfig::new("WIPF"))
            .expect("spawn a live object");
        let before = engine
            .snapshot()
            .object(object)
            .expect("the object is live")
            .clone();

        // A named graphic that the reloaded definition no longer supplies must
        // fall back to the object's own definition rather than being left
        // pointing at a name nothing provides
        // (`C4DefGraphicsPtrBackup::AssignUpdate`, C4DefGraphics.cpp:355-400).
        if let Some(index) = engine.find_object_index(object) {
            engine.objects[index].state.base_graphics = Some(crate::ObjectBaseGraphics {
                definition: crate::DefinitionId::from("WIPF"),
                graphics_name: Some("NoSuchVariant".to_string()),
                blit_mode: 0,
            });
        }

        assert!(engine.reload_definition("WIPF", false));

        let index = engine
            .find_object_index(object)
            .expect("the object is not removed: its own definition can serve it");
        assert_eq!(
            engine.objects[index]
                .state
                .base_graphics
                .as_ref()
                .and_then(|graphics| graphics.graphics_name.clone()),
            None,
            "a vanished named graphic falls back to the definition's own"
        );

        // The object survives with its own state intact: `UpdateFace` writes
        // only definition projections, so position, Con, rotation and colour
        // are untouched — a reload refreshes an object, it does not
        // reinitialise one.
        let snapshot = engine.snapshot();
        let live = snapshot
            .object(object)
            .expect("the object survives a successful reload");
        assert_eq!(live.position, before.position);
        assert_eq!(live.rotation, before.rotation);
        assert_eq!(live.energy, before.energy);
    }

    // The reload against a *real shipped group*, not a synthetic DefCore:
    // C4DefList::Reload re-opens the definition's own path and rebuilds it
    // through the same loader production uses (C4Def.cpp:1191-1213).
    #[test]
    fn reloading_a_shipped_definition_group_rebuilds_it_from_disk() {
        let group = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/Objects.c4d/Animals.c4d/Wipf.c4d");
        if !group.is_dir() {
            // The content submodule is not materialised in this checkout.
            return;
        }

        let mut engine = crate::Engine::new();
        let mut definition =
            crate::Definition::from_script("WIPF".to_string(), "placeholder".to_string(), "")
                .expect("script definition compiles");
        definition.set_source_path(Some(group.clone()));
        engine
            .register_definition(definition)
            .expect("register definition");
        assert_eq!(engine.definition("WIPF").map(crate::Definition::name), Some("placeholder"));

        assert!(
            engine.reload_definition("WIPF", false),
            "a real shipped group reloads"
        );
        let reloaded = engine.definition("WIPF").expect("the definition survives");
        assert_eq!(reloaded.source_path(), Some(group.as_path()));
        // The name came from DefCore.txt on disk, replacing the placeholder —
        // so the rebuild really re-read the group rather than keeping what was
        // registered.
        assert_ne!(reloaded.name(), "placeholder");
        // And the group is offered to the file monitor, since it is unpacked.
        assert_eq!(engine.monitored_definition_directories(), vec![group]);
    }

    // C4Def.cpp:547-560 — only unpacked definition groups are watched, and
    // each group is registered once.
    #[test]
    fn only_unpacked_definition_groups_are_offered_to_the_file_monitor() {
        let dir = tempfile::tempdir().expect("temp root");
        let unpacked = dir.path().join("Rock.c4d");
        std::fs::create_dir_all(&unpacked).expect("create unpacked group");
        let packed = dir.path().join("Packed.c4d");
        std::fs::write(&packed, b"packed group bytes").expect("write packed group");

        let mut engine = crate::Engine::new();
        for (id, path) in [
            ("ROCK", Some(unpacked.clone())),
            ("PACK", Some(packed.clone())),
            ("SCRP", None),
        ] {
            let mut definition =
                crate::Definition::from_script(id.to_string(), id.to_string(), "")
                    .expect("script definition compiles");
            definition.set_source_path(path);
            engine
                .register_definition(definition)
                .expect("register definition");
        }

        // A packed group has no directory to observe, and a script-only
        // definition has no group at all.
        assert_eq!(
            engine.monitored_definition_directories(),
            vec![unpacked.clone()]
        );

        // Two definitions sharing one group register it once — C++ skips a
        // location it already has.
        let mut sibling =
            crate::Definition::from_script("ROK2".to_string(), "Rock 2".to_string(), "")
                .expect("script definition compiles");
        sibling.set_source_path(Some(unpacked.clone()));
        engine
            .register_definition(sibling)
            .expect("register sibling definition");
        assert_eq!(engine.monitored_definition_directories(), vec![unpacked]);
    }

    // C4Def.cpp:1191-1213 + C4Game.cpp:2322-2367 — the reload re-opens the
    // definition's own stored group, and a failed load removes it outright.
    #[test]
    fn reloading_a_definition_reopens_its_group_and_removes_it_on_failure() {
        let dir = tempfile::tempdir().expect("temp group root");
        let group_path = dir.path().join("Rock.c4d");
        std::fs::create_dir_all(&group_path).expect("create definition group");
        std::fs::write(
            group_path.join("DefCore.txt"),
            "[DefCore]\nid=ROCK\nVersion=4,9,8\nName=Rock\n",
        )
        .expect("write DefCore");

        let mut engine = crate::Engine::new();
        let mut definition =
            crate::Definition::from_script("ROCK".to_string(), "Rock".to_string(), "")
                .expect("script definition compiles");
        definition.set_source_path(Some(group_path.clone()));
        engine
            .register_definition(definition)
            .expect("register definition");

        // The network refusal is the first line: nothing is re-opened and the
        // definition is untouched.
        assert!(!engine.reload_definition("ROCK", true));
        assert!(engine.definition("ROCK").is_some());

        // A definition with no stored group cannot reload, and nothing is
        // disturbed by the attempt.
        let mut pathless =
            crate::Definition::from_script("STON".to_string(), "Stone".to_string(), "")
                .expect("script definition compiles");
        pathless.set_source_path(None);
        engine
            .register_definition(pathless)
            .expect("register pathless definition");
        assert!(!engine.reload_definition("STON", false));
        assert!(
            engine.definition("STON").is_some(),
            "a definition with no group to re-open is refused, not removed"
        );

        // The group is real, so the reload succeeds and the definition keeps
        // its path for the next one.
        assert!(engine.reload_definition("ROCK", false));
        let reloaded = engine.definition("ROCK").expect("the definition survives");
        assert_eq!(reloaded.source_path(), Some(group_path.as_path()));

        // Now break the group. `C4Def::Clear` has already emptied the
        // definition by the time `Load` fails, so the failure arm removes it
        // rather than restoring anything.
        std::fs::remove_dir_all(&group_path).expect("remove the group");
        assert!(!engine.reload_definition("ROCK", false));
        assert!(
            engine.definition("ROCK").is_none(),
            "a failed reload removes the definition"
        );
    }

    // C4Game.cpp:2352-2360 — a failed reload removes the definition outright,
    // so removal must unwind every structure registration pushed into.
    #[test]
    fn removing_a_definition_unwinds_everything_registration_added() {
        let mut engine = crate::Engine::new();
        for id in ["AAAA", "BBBB"] {
            let definition =
                crate::Definition::from_script(id.to_string(), id.to_string(), "")
                    .expect("script definition compiles");
            engine
                .register_definition(definition)
                .expect("register definition");
        }
        assert!(engine.definition("AAAA").is_some());

        assert!(engine.remove_definition("AAAA"));
        assert!(engine.definition("AAAA").is_none());
        assert!(
            engine.definition("BBBB").is_some(),
            "removing one definition leaves its siblings alone"
        );
        // Removing the same id twice reports the miss rather than unwinding
        // anything a second time.
        assert!(!engine.remove_definition("AAAA"));

        // The id is free again, which it would not be if the map entry were
        // the only thing dropped.
        let definition = crate::Definition::from_script("AAAA".to_string(), "A".to_string(), "")
            .expect("script definition compiles");
        engine
            .register_definition(definition)
            .expect("the removed id can be registered again");
    }

    // C4Def.cpp:547-560 — `C4Def::Load` stores the group's own full name as
    // `Filename`. `C4DefList::Reload` re-opens exactly that, `C4Def::Clear`
    // deliberately preserves it ("Assume filename is being kept"), and
    // `AddDirectoryForMonitoring` watches it. A definition with no group
    // behind it carries none, which is the case a reload must refuse rather
    // than attempt.
    #[test]
    fn definitions_carry_the_group_they_were_loaded_from() {
        let mut definition =
            crate::Definition::from_script("TEST".to_string(), "Test".to_string(), "")
                .expect("script-only definition compiles");
        assert!(
            definition.source_path().is_none(),
            "a definition built from script alone has no group to reload from"
        );

        let group = std::path::PathBuf::from("/content/Objects.c4d/Rock.c4d");
        definition.set_source_path(Some(group.clone()));
        assert_eq!(definition.source_path(), Some(group.as_path()));

        // Clearing it is not the same as never having had one, but both refuse
        // a reload — C++ tests `if (!Filename[0])` (C4Particles.cpp:197 for the
        // particle sibling; the def path re-opens Filename directly).
        definition.set_source_path(None);
        assert!(definition.source_path().is_none());
    }

    #[test]
    fn legacy_string_table_reuses_identity_and_overwrites_repeated_line_id() {
        let directory = tempdir().expect("string-table directory");
        std::fs::write(directory.path().join("Strings.txt"), b"same\r\nsame\r\n")
            .expect("write Strings.txt");
        let group = Group::open(directory.path()).expect("open string-table group");
        let registrations = load_legacy_string_table(&group).expect("load Strings.txt");

        assert!(clonk_script::resolve_c4_string(&registrations, 0).is_none());
        let repeated = clonk_script::resolve_c4_string(&registrations, 1)
            .expect("later repeated line owns the shared ID");
        let repeated_again = clonk_script::resolve_c4_string(&registrations, 1)
            .expect("live shared identity remains resolvable");
        assert!(repeated.ptr_eq(&repeated_again));
        assert_eq!(repeated.as_ref(), "same");
        assert_eq!(
            clonk_script::resolve_c4_string(&registrations, 2)
                .expect("trailing LF creates an empty final line")
                .as_ref(),
            ""
        );

        let mut engine = Engine::new();
        engine.adopt_legacy_string_table(registrations);
        let literal =
            clonk_script::register_c4_literal_string(&engine.script_string_registrations, "same");
        assert!(repeated.ptr_eq(&literal));
    }

    #[test]
    fn legacy_string_table_stops_the_whole_scan_at_first_nul() {
        let directory = tempdir().expect("string-table directory");
        std::fs::write(
            directory.path().join("Strings.txt"),
            b"first\nsecond\0third\nignored",
        )
        .expect("write Strings.txt");
        let group = Group::open(directory.path()).expect("open string-table group");
        let registrations = load_legacy_string_table(&group).expect("load Strings.txt");

        assert_eq!(
            clonk_script::resolve_c4_string(&registrations, 0)
                .expect("first line")
                .as_ref(),
            "first"
        );
        assert_eq!(
            clonk_script::resolve_c4_string(&registrations, 1)
                .expect("prefix before NUL")
                .as_ref(),
            "second"
        );
        assert!(clonk_script::resolve_c4_string(&registrations, 2).is_none());
    }

    #[test]
    fn legacy_string_table_preserves_non_utf8_bytes_verbatim() {
        let directory = tempdir().expect("string-table directory");
        std::fs::write(directory.path().join("Strings.txt"), [0xe4])
            .expect("write raw Strings.txt");
        let group = Group::open(directory.path()).expect("open string-table group");
        let registrations = load_legacy_string_table(&group).expect("load Strings.txt");

        let value = clonk_script::resolve_c4_string(&registrations, 0)
            .expect("raw string-table entry resolves");
        assert_eq!(clonk_script::c4_string_bytes(&value), [0xe4]);
        assert_eq!(clonk_script::c4_string_byte_len(&value), 1);
    }

    const TEST_SCRIPT: &str = r#"
global func Initialize(state, random)
{
    return 0;
}

global func Step(state, frame, random)
{
    return 0;
}
"#;

    #[test]
    fn loaded_actidle_name_uses_the_builtin_sentinel() {
        let state = build_action_state(
            Some("ActIdle".to_string()),
            Some(4),
            Some(5),
            Some(6),
            None,
            None,
            None,
        )
        .expect("saved action builds");
        assert_eq!(state.name, "Idle");
        assert_eq!(state.act_map_index, None);
    }

    #[test]
    fn loaded_action_name_buffer_stops_at_nul_before_lookup() {
        let state = build_action_state(
            Some("Walk\0ignored".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("saved action builds");

        assert_eq!(state.name, "Walk");
        assert_eq!(
            clonk_script::c4_string_bytes(state.compiled_name()),
            b"Walk"
        );
    }

    #[test]
    fn legacy_scenario_action_conversion_keeps_exact_first_reflection() {
        let mut first = ResourceActionDefinition::default();
        first.length = Some(7);
        first.sound = Some("Zap".to_string());
        first.disabled = true;
        first.reflected_ints.insert("ObjectDisabled".to_string(), 7);
        first.reflected_ints.insert("Length".to_string(), -3);
        let mut duplicate = ResourceActionDefinition::default();
        duplicate.length = Some(99);
        let converted = convert_action_map(&ResourceActionMap {
            default_action: None,
            actions: vec![
                ("Probe".to_string(), first),
                ("Probe".to_string(), duplicate),
            ],
        });
        let spec = converted
            .specs
            .get("Probe")
            .expect("runtime action retained");
        assert_eq!(
            spec.length,
            Some(7),
            "runtime action selection keeps the first physical slot"
        );
        let reflection = converted
            .reflections
            .get("Probe")
            .expect("exact compiler view retained");
        assert_eq!(
            reflection.get("Length", 0),
            Some(clonk_script::Value::Int(-3))
        );
        assert_eq!(
            reflection.get("Sound", 0),
            Some(clonk_script::Value::String("Zap".into()))
        );
        assert_eq!(
            reflection.get("ObjectDisabled", 0),
            Some(clonk_script::Value::Int(7))
        );
    }

    #[test]
    fn legacy_scenario_action_conversion_retains_signed_runtime_fields() {
        let mut action = ResourceActionDefinition::default();
        action.length = Some(-4);
        action.delay = Some(-6);
        action.step = Some(-11);
        action.directions = Some(-2);
        action.flip_dir = Some(-3);
        let converted = convert_action_map(&ResourceActionMap {
            default_action: None,
            actions: vec![("Odd".to_string(), action)],
        });

        let spec = converted.specs.get("Odd").expect("runtime action retained");
        assert_eq!(spec.length, Some(-4));
        assert_eq!(spec.delay, Some(-6));
        assert_eq!(spec.step, Some(-11));
        assert_eq!(spec.directions, Some(-2));
        let graphics = converted
            .graphics
            .get("Odd")
            .expect("runtime action graphics retained");
        assert_eq!(graphics.length, Some(-4));
        assert_eq!(graphics.directions, -2);
        assert_eq!(graphics.flip_dir, Some(-3));
    }

    #[test]
    fn legacy_objects_size_preserves_oversize_construction() {
        // C4Object::CompileFunc reads Size directly into Con. Loaded object
        // state may therefore remain above FullCon even independently of a
        // later DoCon clamp decision (C4Object.cpp:2763).
        let records =
            parse_legacy_objects("[Object]\nid=OVSZ\nSize=150000\n").expect("Objects.txt parses");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].construction, Some(150_000));
    }

    #[test]
    fn scenario_value_store_follows_c4value_compiler_indexing_and_types() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\n\
             Version=4,9\n\
             SaveGame=2\n\
             Replay=-3\n\
             \n\
             [Definitions]\n\
             Definition1=Objects.c4d\n\
             Definition2= \"Foo\\Bar.c4d\"  \n\
             \n\
             [Player1]\n\
             StandardCrew=NONE\n\
             EnforcePosition=7\n\
             HomeBaseMaterial=WOOD=5;ROCK;\n\
             \n\
             [Player2]\n\
             StandardCrew=0000\n\
             EnforcePosition=8\n\
             \n\
             [Player3]\n\
             StandardCrew=clnkextra\n\
             \n\
             [Player4]\n\
             StandardCrew=CL-Nextra\n\
             \n\
             [Landscape]\n\
             MapWidth=64,2,32,250\n\
             MapZoom=8,2,5,15\n\
             Sky=Clouds1,Clouds2\n\
             \n\
             [Disasters]\n\
             Volcano=12,3,0,100\n",
        )
        .expect("scenario core");
        let group = Group::open(directory.path()).expect("scenario group");
        let manifest = parse_legacy_scenario_manifest(&group).expect("parsed scenario core");
        let values = ScenarioValueStore::from_runtime_core(&manifest.core, false);

        assert_eq!(
            values.get("MapZoom", Some("Landscape"), 0),
            Some(&ScenarioValue::Int(8))
        );
        assert_eq!(
            values.get("MapZoom", Some("Landscape"), 1),
            Some(&ScenarioValue::Int(2))
        );
        assert_eq!(
            values.get("MapZoom", Some("Landscape"), 2),
            Some(&ScenarioValue::Int(5))
        );
        assert_eq!(
            values.get("MapZoom", Some("Landscape"), 3),
            Some(&ScenarioValue::Int(15))
        );
        assert_eq!(values.get("MapZoom", Some("Landscape"), 4), None);
        assert_eq!(values.get("MapZoom", Some("Landscape"), -1), None);

        assert_eq!(
            values.get("HomeBaseMaterial", Some("Player1"), 0),
            Some(&ScenarioValue::C4Id("WOOD".to_string()))
        );
        assert_eq!(
            values.get("HomeBaseMaterial", Some("Player1"), 1),
            Some(&ScenarioValue::Int(5))
        );
        assert_eq!(
            values.get("HomeBaseMaterial", Some("Player1"), 2),
            Some(&ScenarioValue::C4Id("ROCK".to_string()))
        );
        assert_eq!(
            values.get("HomeBaseMaterial", Some("Player1"), 3),
            Some(&ScenarioValue::Int(0))
        );
        assert_eq!(values.get("HomeBaseMaterial", Some("Player1"), 4), None);

        assert_eq!(
            values.get("SaveGame", Some("Head"), 0),
            Some(&ScenarioValue::Int(2))
        );
        assert_eq!(
            values.get("Replay", Some("Head"), 0),
            Some(&ScenarioValue::Int(-3))
        );
        assert_eq!(
            values.get("EnforcePosition", Some("Player1"), 0),
            Some(&ScenarioValue::Int(7))
        );
        assert_eq!(
            values.get("EnforcePosition", None, 1),
            Some(&ScenarioValue::Int(8)),
            "a no-section lookup carries entry_nr across repeated names"
        );
        assert_eq!(
            values.get("Version", Some("Head"), 1),
            Some(&ScenarioValue::Int(9))
        );
        assert_eq!(values.get("Version", Some("Head"), 2), None);
        assert_eq!(
            values.get("Definitions", Some("Definitions"), 0),
            Some(&ScenarioValue::String("Objects.c4d".to_string()))
        );
        assert_eq!(
            values.get("Definitions", Some("Definitions"), 1),
            Some(&ScenarioValue::String("\"Foo\\Bar.c4d\"  ".to_string()))
        );
        assert_eq!(
            values.get("Definitions", None, 0),
            None,
            "the root [Definitions] match shadows its same-name child"
        );
        assert_eq!(
            values.get("StandardCrew", Some("Player1"), 0),
            Some(&ScenarioValue::C4Id(String::new()))
        );
        assert_eq!(
            values.get("StandardCrew", Some("Player2"), 0),
            Some(&ScenarioValue::C4Id(String::new()))
        );
        assert_eq!(
            values.get("StandardCrew", Some("Player3"), 0),
            Some(&ScenarioValue::C4Id("clnk".to_string()))
        );
        assert_eq!(
            values.get("StandardCrew", Some("Player4"), 0),
            Some(&ScenarioValue::C4Id("CL-N".to_string()))
        );

        // C4Landscape::Init mutates these three Game.C4S fields before any
        // scenario callback can observe them.
        assert_eq!(
            values.get("MapWidth", Some("Landscape"), 3),
            Some(&ScenarioValue::Int(10_000))
        );
        assert_eq!(
            values.get("MapHeight", Some("Landscape"), 3),
            Some(&ScenarioValue::Int(10_000))
        );
        assert_eq!(
            values.get("NewStyleLandscape", Some("Landscape"), 0),
            Some(&ScenarioValue::Int(2))
        );
        assert_eq!(
            values.get("Sky", Some("Landscape"), 0),
            Some(&ScenarioValue::String("Clouds1;Clouds2".to_string()))
        );
        let values_with_sky_surface = ScenarioValueStore::from_runtime_core(&manifest.core, true);
        assert_eq!(
            values_with_sky_surface.get("Sky", Some("Landscape"), 0),
            Some(&ScenarioValue::String("Clouds1,Clouds2".to_string()))
        );
        assert_eq!(
            values.get("Volcano", Some("Disasters"), 0),
            Some(&ScenarioValue::Int(12))
        );
        assert_eq!(values.get("Volcano", Some("Weather"), 0), None);
        assert_eq!(values.get("StartupPlayerCount", Some("Head"), 0), None);
        assert_eq!(values.get("mapzoom", Some("Landscape"), 0), None);

        let mut engine = Engine::new();
        engine.set_scenario_values(values.clone());
        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("scenario values serialize with engine state");
        let state = crate::EngineState::from_json_str(&encoded)
            .expect("scenario values deserialize with engine state");
        let mut restored = Engine::new();
        restored
            .restore_state(&state)
            .expect("scenario values restore into a fresh engine");
        assert_eq!(restored.scenario_values.as_ref(), &values);

        let defaults = ScenarioValueStore::default();
        assert_eq!(
            defaults.get("MapZoom", Some("Landscape"), 0),
            Some(&ScenarioValue::Int(10))
        );
    }

    #[test]
    fn section_scenario_core_resets_compiled_fields_and_ignores_main_only_entries() {
        let main = parse_legacy_scenario_text(
            "[Head]\n\
             Title=Main title\n\
             Version=4,6,4\n\
             MaxPlayer=7\n\
             MaxPlayerLeague=5\n\
             SaveGame=1\n\
             NoInitialize=9\n\
             RandomSeed=123\n\
             ForcedAutoContextMenu=1\n\
             ForcedAutoStopControl=0\n\
             \n\
             [Definitions]\n\
             Definition1=Main.c4d\n\
             \n\
             [Game]\n\
             Goals=MELE=1\n\
             StructNeedEnergy=0\n\
             ValueOverloads=FISH=20\n\
             \n\
             [Player1]\n\
             Wealth=42,0,0,250\n\
             \n\
             [Landscape]\n\
             MapWidth=2,0,2,2\n\
             MapHeight=3,0,3,3\n\
             MapZoom=5,0,5,5\n\
             ShadeMaterials=1\n\
             \n\
             [Weather]\n\
             Wind=100,0,-100,100\n",
        )
        .expect("main core parses");
        let section = parse_legacy_scenario_text(
            "[Head]\n\
             Title=Ignored title\n\
             Version=9,9,9\n\
             MaxPlayer=99\n\
             MaxPlayerLeague=98\n\
             SaveGame=0\n\
             NoInitialize=2\n\
             RandomSeed=7\n\
             \n\
             [Definitions]\n\
             Definition1=Ignored.c4d\n\
             \n\
             [Game]\n\
             ValueOverloads=ROCK=99\n\
             \n\
             [Landscape]\n\
             Sky=Clouds\n",
        )
        .expect("section core parses");
        let compiled = overlay_legacy_scenario_manifest(&main, section)
            .expect("section compiles over main core");

        assert_eq!(compiled.title.as_deref(), Some("Main title"));
        assert_eq!(compiled.core.head.title, "Main title");
        assert_eq!(compiled.core.head.version, [4, 6, 4, 0, 0]);
        assert_eq!(compiled.core.head.max_player, 7);
        assert_eq!(compiled.core.head.max_player_league, 5);
        assert_eq!(compiled.core.head.save_game, 1);
        assert_eq!(compiled.core.head.no_initialize, 2);
        assert_eq!(compiled.core.head.random_seed, 7);
        assert_eq!(compiled.core.head.forced_auto_context_menu, 1);
        assert_eq!(compiled.core.head.forced_control_style, 0);
        assert_eq!(compiled.definition_specs, vec!["Main.c4d"]);
        assert_eq!(
            compiled.core.game.realism.value_overloads,
            vec![LegacyIdEntry {
                id: "FISH".to_string(),
                count: Some(20),
            }]
        );

        assert!(compiled.core.game.goals.is_empty());
        assert!(compiled.core.game.rules.is_empty());
        assert!(compiled.core.game.realism.structures_need_energy);
        assert_eq!(compiled.core.players.len(), MAX_PLAYER_STARTS);
        assert_eq!(
            compiled.core.players[0].wealth,
            LegacyC4SVal::new(0, 0, 0, 250)
        );
        assert_eq!(
            compiled.core.landscape.map_width,
            LegacyC4SVal::new(100, 0, 64, 250)
        );
        assert_eq!(
            compiled.core.landscape.map_height,
            LegacyC4SVal::new(50, 0, 40, 250)
        );
        assert_eq!(
            compiled.core.landscape.map_zoom,
            LegacyC4SVal::new(10, 0, 5, 15)
        );
        assert_eq!(compiled.core.landscape.sky.as_deref(), Some("Clouds"));
        assert!(
            !compiled.core.landscape.shade_materials,
            "the absent default uses retained main Version=4.6.4"
        );
        assert_eq!(
            compiled.core.weather.wind,
            LegacyC4SVal::new(0, 70, -100, 100)
        );
        assert_eq!(
            compiled.sections.get("landscape"),
            Some(&vec![("Sky".to_string(), "Clouds".to_string())]),
            "raw main landscape entries must not leak into section consumers"
        );

        let values = ScenarioValueStore::from_runtime_core(&compiled.core, false);
        assert_eq!(values.get("Goals", Some("Game"), 0), None);
        assert_eq!(
            values.get("Rules", Some("Game"), 0),
            Some(&ScenarioValue::C4Id("ENRG".to_string()))
        );
        assert_eq!(
            values.get("Rules", Some("Game"), 1),
            Some(&ScenarioValue::Int(1))
        );
        assert_eq!(
            values.get("StructNeedEnergy", Some("Game"), 0),
            Some(&ScenarioValue::Bool(false))
        );
        assert_eq!(
            values.get("ValueOverloads", Some("Game"), 0),
            Some(&ScenarioValue::C4Id("FISH".to_string()))
        );
        assert_eq!(
            values.get("MapWidth", Some("Landscape"), 0),
            Some(&ScenarioValue::Int(100))
        );
        assert_eq!(
            values.get("MapWidth", Some("Landscape"), 3),
            Some(&ScenarioValue::Int(10_000))
        );
        assert_eq!(
            values.get("Wind", Some("Weather"), 0),
            Some(&ScenarioValue::Int(0))
        );
        assert_eq!(
            values.get("Wind", Some("Weather"), 1),
            Some(&ScenarioValue::Int(70))
        );
        assert_eq!(
            values.get("ShadeMaterials", Some("Landscape"), 0),
            Some(&ScenarioValue::Bool(false))
        );

        let directory = tempdir().expect("section directory");
        std::fs::write(
            directory.path().join("Landscape.txt"),
            "map Next { seed=11; };",
        )
        .expect("section landscape script");
        let group = Group::open(directory.path()).expect("section group opens");
        let mut classifier = MapPixelClassifier::from_slots(
            [0; 128],
            vec![None; 128],
            vec![None; 128],
            vec![None; 128],
        );
        let landscape =
            load_legacy_landscape_body_for_test(&group, &compiled, Some(&mut classifier), 0, 1)
                .expect("section landscape loads")
                .expect("section landscape exists");
        let raster = landscape.raster_state().expect("section raster state");
        let map = raster.map().expect("section retains its generated map");
        assert_eq!((map.width, map.height), (100, 50));
        assert_eq!(raster.map_zoom(), 10);
    }

    // Clean differential captured from the stock pre-port C++ merge-base
    // 9ffa0a5d. SHA-256 of the pristine 1302-byte Scenario.txt:
    // 99351a3dede2076f8e4666d62c71362db25ddc3c953bcf60b711960100c80914.
    const TUTORIAL01_PRISTINE_CPP_INITIAL_NETWORK_SCENARIO: &str = concat!(
        "[Head]\r\n",
        "Icon=2\r\n",
        "Title=A Clonk\r\n",
        "Version=4,9,11\r\n",
        "Difficulty=1\r\n",
        "MaxPlayer=1\r\n",
        "DisableMouse=1\r\n",
        "NetworkGame=true\r\n",
        "ForcedGfxMode=1\r\n",
        "Origin=Tutorial.c4f/Tutorial01.c4s\r\n",
        "\r\n",
        "[Definitions]\r\n",
        "Definitions=\"Objects.c4d\",\"Tutorial.c4f\"\r\n",
        "SkipDefs=ERTH=1\r\n",
        "\r\n",
        "[Game]\r\n",
        "StructNeedEnergy=false\r\n",
        "Rules=SURR=1\r\n",
        "\r\n",
        "[Player1]\r\n",
        "Position=32,20\r\n",
        "Crew=CLNK=1\r\n",
        "Magic=MWND=0;MWP2=0;MGWP=0;MVLC=0;RVLT=0;RMMG=0;MMTR=0;MLGT=0;MINV=0;MGHL=0;MGUP=0;MGDW=0;MFFW=0;MFFS=0;MBRG=0;MFFA=0;FRFS=0;EXTG=0;ETFL=0;MQKE=0;CMFG=0\r\n",
        "\r\n",
        "[Player2]\r\n",
        "Crew=CLNK=1\r\n",
        "Magic=MWND=0;MWP2=0;MGWP=0;MVLC=0;RVLT=0;RMMG=0;MMTR=0;MLGT=0;MINV=0;MGHL=0;MGUP=0;MGDW=0;MFFW=0;MFFS=0;MBRG=0;MFFA=0;FRFS=0;EXTG=0;ETFL=0;MQKE=0;CMFG=0\r\n",
        "\r\n",
        "[Player3]\r\n",
        "Crew=CLNK=1\r\n",
        "Magic=MWND=0;MWP2=0;MGWP=0;MVLC=0;RVLT=0;RMMG=0;MMTR=0;MLGT=0;MINV=0;MGHL=0;MGUP=0;MGDW=0;MFFW=0;MFFS=0;MBRG=0;MFFA=0;FRFS=0;EXTG=0;ETFL=0;MQKE=0;CMFG=0\r\n",
        "\r\n",
        "[Player4]\r\n",
        "Crew=CLNK=1\r\n",
        "Magic=MWND=0;MWP2=0;MGWP=0;MVLC=0;RVLT=0;RMMG=0;MMTR=0;MLGT=0;MINV=0;MGHL=0;MGUP=0;MGDW=0;MFFW=0;MFFS=0;MBRG=0;MFFA=0;FRFS=0;EXTG=0;ETFL=0;MQKE=0;CMFG=0\r\n",
        "\r\n",
        "[Landscape]\r\n",
        "VegetationLevel=0,30,0,100\r\n",
        "Sky=Clouds2\r\n",
        "AutoScanSideOpen=false\r\n",
        "MapWidth=64,0,64,250\r\n",
        "MapHeight=47,0,40,250\r\n",
        "Liquid=Water-Smooth\r\n",
        "Layers=Earth-Rough=100;Earth-Smooth2=100\r\n",
        "SkyScrollMode=2\r\n",
        "\r\n",
        "[Weather]\r\n",
        "Climate=30,0,0,100\r\n",
        "YearSpeed=20,10,0,100\r\n",
        "Wind=1,30,-100,100\r\n",
    );

    fn json_scenario_without_legacy_core() -> Scenario {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Scenario.json"),
            r#"{"definitions":[{"id":"TEST","script":"Script.c"}]}"#,
        )
        .expect("write JSON fixture");
        std::fs::write(dir.path().join("Script.c"), TEST_SCRIPT).expect("write test script");
        Scenario::load_from_path(dir.path()).expect("JSON fixture loads")
    }

    fn scenario_with_retained_legacy_core(source: &str) -> Scenario {
        let mut scenario = json_scenario_without_legacy_core();
        scenario.legacy_core = Some(
            parse_legacy_scenario_text(source)
                .expect("legacy core parses")
                .core,
        );
        scenario
    }

    #[test]
    fn offline_startup_preflight_reads_effective_max_players_without_loading_resources() {
        // OpenScenario loads C4S first, then Parameters.txt overrides the
        // scenario-derived MaxPlayers default before InitLocal admits players
        // (pristine 9ffa0a5d src/C4Game.cpp:162-166,231-248;
        // src/C4GameParameters.cpp:408-422,553-558).
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Scenario.txt"),
            "[Head]\nMaxPlayer=4\n\n[Definitions]\nDefinition1=Missing.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            dir.path().join("Parameters.txt"),
            "[Parameters]\nRandomSeed=73\nMaxPlayers=2\n",
        )
        .expect("write parameters");
        let group = Group::open(dir.path()).expect("open scenario group");

        let expected = OfflineScenarioStartupPreflight {
            max_players: 2,
            random_seed: Some(73),
            save_game: false,
            restore_player_infos: false,
        };
        assert_eq!(
            Scenario::preflight_offline_startup_from_group(&group)
                .expect("group preflight succeeds"),
            expected,
        );
        assert_eq!(
            Scenario::preflight_offline_startup_from_path(dir.path())
                .expect("path preflight succeeds"),
            expected,
        );

        std::fs::remove_file(dir.path().join("Parameters.txt"))
            .expect("remove parameter component");
        let group = Group::open(dir.path()).expect("reopen scenario without parameters");
        let expected = OfflineScenarioStartupPreflight {
            max_players: 4,
            random_seed: None,
            save_game: false,
            restore_player_infos: false,
        };
        assert_eq!(
            Scenario::preflight_offline_startup_from_group(&group)
                .expect("missing-parameters preflight succeeds"),
            expected,
            "only a missing Parameters.txt requests the time/LC_PIN_SEED default",
        );
        assert_eq!(
            Scenario::preflight_offline_startup_from_path(dir.path())
                .expect("path preflight succeeds"),
            expected,
        );

        let savegame = dir.path().join("Savegame.c4s");
        std::fs::create_dir(&savegame).expect("create savegame fixture");
        std::fs::write(
            savegame.join("Scenario.txt"),
            "[Head]\nSaveGame=1\nMaxPlayer=4\n",
        )
        .expect("write savegame core");
        assert_eq!(
            Scenario::preflight_offline_startup_from_path(&savegame)
                .expect("offline savegame preflight succeeds"),
            OfflineScenarioStartupPreflight {
                max_players: 4,
                random_seed: None,
                save_game: true,
                restore_player_infos: false,
            }
        );

        let replay = dir.path().join("Replay.c4s");
        std::fs::create_dir(&replay).expect("create replay fixture");
        std::fs::write(
            replay.join("Scenario.txt"),
            "[Head]\nReplay=1\nMaxPlayer=4\n",
        )
        .expect("write replay core");
        assert!(matches!(
            Scenario::preflight_offline_startup_from_path(&replay),
            Err(ScenarioError::OfflineStartupReplayUnsupported)
        ));

        let json = dir.path().join("Json.c4s");
        std::fs::create_dir(&json).expect("create JSON fixture");
        std::fs::write(json.join("Scenario.json"), "{\"definitions\":[]}")
            .expect("write JSON manifest");
        assert!(matches!(
            Scenario::preflight_offline_startup_from_path(&json),
            Err(ScenarioError::OfflineStartupJsonUnsupported)
        ));
    }

    #[test]
    fn offline_startup_preflight_admits_restore_infos_without_a_savegame() {
        // C4GameParameters::Load reads SavePlayerInfos.txt into
        // RestorePlayerInfos whenever the entry exists; Head.SaveGame guards
        // only the historical Game.txt [PlayerFiles] fallback in the else-arm
        // (pristine 9ffa0a5d src/C4GameParameters.cpp:378-399). InitPlayers
        // then enters the restore branch on
        // RestorePlayerInfos.GetActivePlayerCount(true) alone, "for savegames
        // or regular scenarios with restore infos" (src/C4Game.cpp:2841-2843).
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Scenario.txt"), "[Head]\nMaxPlayer=4\n")
            .expect("write scenario core");
        std::fs::write(
            dir.path().join("SavePlayerInfos.txt"),
            "[PlayerInfoList]\nLastPlayerID=1\n",
        )
        .expect("write restore infos");

        assert_eq!(
            Scenario::preflight_offline_startup_from_path(dir.path())
                .expect("a regular scenario shipping restore infos preflights"),
            OfflineScenarioStartupPreflight {
                max_players: 4,
                random_seed: None,
                save_game: false,
                restore_player_infos: true,
            },
        );
    }

    #[test]
    fn legacy_network_game_flag_is_preserved_for_client_safety_check() {
        // After opening the retrieved client scenario, C4Game rejects it when
        // C4S.Head.NetworkGame is false (C4Game.cpp:2551-2564).
        let network = scenario_with_retained_legacy_core("[Head]\nNetworkGame=true\n");
        let offline = scenario_with_retained_legacy_core("[Head]\nNetworkGame=false\n");

        assert!(network.network_game());
        assert!(!offline.network_game());
    }

    fn legacy_cstring(bytes: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(bytes.to_vec()).expect("fixture has no interior NUL")
    }

