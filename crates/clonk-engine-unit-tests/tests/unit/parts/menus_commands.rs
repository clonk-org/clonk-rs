    #[test]
    fn close_menu_and_menu_query_cancel_follow_cpp_close_semantics() {
        // FnCloseMenu (C4Script.cpp:4309-4314) forces the close —
        // C4Menu::TryClose(fOK=true) skips IsCloseDenied (C4Menu.cpp:
        // 317-320). FnCreateMenu's clear of the OLD menu is soft
        // (CloseMenu(false), C4Script.cpp:1447): a truthy MenuQueryCancel
        // (C4ObjectMenu::IsCloseDenied, C4ObjectMenu.cpp:56-75) keeps the
        // old menu and fails the new one.
        let script = r#"
        local deny;
        func SetDeny(flag) { deny = flag; }
        func MenuQueryCancel() { return deny; }
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func OpenOther() { return CreateMenu(MENU, this(), this(), 0, "Other"); }
        func Shut() { return CloseMenu(this()); }
        func ReadMenu() { return GetMenu(this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, Vec::new())
                .expect("call succeeds")
        };

        // CloseMenu without a menu still succeeds (C4Object::CloseMenu
        // returns true when Menu is null, C4Object.cpp:2009-2016).
        assert_eq!(call(&mut engine, "Shut"), Value::Bool(true));

        // Open, then force-close despite MenuQueryCancel denying.
        assert_eq!(call(&mut engine, "OpenMenu"), Value::Bool(true));
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .call_object_function(idx, "SetDeny", vec![Value::Int(1)])
            .expect("SetDeny succeeds");
        assert_eq!(
            call(&mut engine, "Shut"),
            Value::Bool(true),
            "forced close skips MenuQueryCancel"
        );
        assert_eq!(call(&mut engine, "ReadMenu"), Value::Int(0));

        // Open again; the denied SOFT close makes a second CreateMenu fail
        // and keeps the old menu.
        assert_eq!(call(&mut engine, "OpenMenu"), Value::Bool(true));
        assert_eq!(
            call(&mut engine, "OpenOther"),
            Value::Bool(false),
            "MenuQueryCancel denies replacing the menu"
        );
        assert_eq!(
            call(&mut engine, "ReadMenu"),
            Value::C4Id("WIPF".into()),
            "the old menu survives the denied replace"
        );

        // Allow the close: the replace goes through.
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .call_object_function(idx, "SetDeny", vec![Value::Int(0)])
            .expect("SetDeny succeeds");
        assert_eq!(call(&mut engine, "OpenOther"), Value::Bool(true));
        assert_eq!(call(&mut engine, "ReadMenu"), Value::C4Id("MENU".into()));
    }

    #[test]
    fn add_menu_item_composes_commands_and_counts_like_cpp() {
        // FnAddMenuItem (C4Script.cpp:1471-1734): no menu -> false; new-style
        // commands (any non-identifier char) go through the %d->%s sprintf
        // hack (:1560-1571), old-style function names compose
        // "Fn(ID,param[,1][,value])" (:1573-1597); a zero count becomes
        // C4MN_Item_NoCount unless C4MN_Add_ForceCount (:1726); items without
        // a command are not selectable (:1729) and the first selectable item
        // grabs the initial selection (C4Menu::AddItem, C4Menu.cpp:424).
        let script = r#"
        func TryEarly() { return AddMenuItem("x", "Cmd", WIPF, this()); }
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func AddPlain() { return AddMenuItem("Info", "", WIPF, this()); }
        func AddNew() { return AddMenuItem("Easy", "SetDifficulty(0)", WIPF, this()); }
        func AddFmt() { return AddMenuItem("Fmt", "Choose(%d,%d)", WIPF, this(), 0, 5); }
        func AddOld() { return AddMenuItem("Old %s", "Choose", CLNK, this(), 3, 7, "info"); }
        func AddValued() { return AddMenuItem("Val", "Choose", CLNK, this(), 0, "txt", 0, 384, 0, 42); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, Vec::new())
                .expect("call succeeds")
        };

        assert_eq!(
            call(&mut engine, "TryEarly"),
            Value::Bool(false),
            "no menu -> false (C4Script.cpp:1475)"
        );
        assert_eq!(call(&mut engine, "OpenMenu"), Value::Bool(true));
        for adder in ["AddPlain", "AddNew", "AddFmt", "AddOld", "AddValued"] {
            assert_eq!(call(&mut engine, adder), Value::Bool(true), "{adder}");
        }

        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.items.len(), 5);

        // Command-less item: never selectable, no-count sentinel.
        let plain = &menu.items[0];
        assert_eq!(plain.command, "");
        assert!(!plain.selectable);
        assert_eq!(plain.count, 12_345_678, "C4MN_Item_NoCount");

        // New style without %d: both commands are the literal text.
        let easy = &menu.items[1];
        assert_eq!(easy.command, "SetDifficulty(0)");
        assert_eq!(easy.command2, "SetDifficulty(0)");
        assert!(easy.selectable);

        // New style with %d: the FIRST %d takes the parameter, the second
        // gets 0 (left) / 1 (right) (C4Script.cpp:1563-1570).
        let fmt = &menu.items[2];
        assert_eq!(fmt.command, "Choose(5,0)");
        assert_eq!(fmt.command2, "Choose(5,1)");

        // Old style: Fn(ID,param) / Fn(ID,param,1); caption %s takes the
        // item def's name (C4Script.cpp:1492-1505); explicit count kept.
        let old = &menu.items[3];
        assert_eq!(old.caption, "Old Clonk");
        assert_eq!(old.command, "Choose(CLNK,7)");
        assert_eq!(old.command2, "Choose(CLNK,7,1)");
        assert_eq!(old.count, 3);
        assert_eq!(old.item_id, "CLNK");

        // C4MN_Add_PassValue (128) + C4MN_Add_ForceCount (256): string
        // parameters are quoted, the value rides along, count 0 stays 0.
        let valued = &menu.items[4];
        assert_eq!(valued.command, "Choose(CLNK,\"txt\",0,42)");
        assert_eq!(valued.command2, "Choose(CLNK,\"txt\",1,42)");
        assert_eq!(valued.count, 0);
        assert_eq!(valued.value, Some(42));

        // The first SELECTABLE item took the initial selection
        // (item 0 is not selectable, so index 1).
        assert_eq!(menu.selection, 1);
    }

    fn attach_one_pixel_portrait(
        engine: &mut Engine,
        definition: &mut Definition,
        name: &str,
    ) {
        let mut portrait_image = Definition::from_script("PIMG", "Portrait image", "")
            .expect("image definition compiles");
        portrait_image.set_picture(Some(DefinitionPicture {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }));
        portrait_image.set_sprite_image(Some(DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: std::sync::Arc::from([0xff, 0xff, 0xff, 0xff]),
            color_mask: None,
        }));
        engine
            .register_definition(portrait_image)
            .expect("image definition registers");
        let image = engine
            .definition_picture_phase_image("PIMG", 0)
            .expect("portrait fixture image");
        definition.set_portrait_graphics(vec![(name.to_string(), image)]);
    }

    #[test]
    fn add_menu_item_preserves_cpp_image_recipes() {
        // FnAddMenuItem creates each C4MN_Add_Img* symbol before adding the
        // row (C4Script.cpp:1597-1729). Keep the source recipe in engine
        // state so Dialog/style 3 can lay out portraits and row images with
        // the same semantics instead of guessing from the cleared caption.
        let script = r#"
        func OpenDialog() { return CreateMenu(CLNK, this(), this(), 0, "", 0, 3); }
        func AddNone() { return AddMenuItem("None", "", NONE, this()); }
        func AddDefinition() { return AddMenuItem("Definition", "", CLNK, this()); }
        func AddFallback() { return AddMenuItem("Fallback %s", "", MISS, this()); }
        func AddPortrait() { return AddMenuItem("Portrait:CLNK::0000ff::1", "", NONE, this(), 0, 0, "", 5, 0, 0); }
        func AddMissingPortrait() { return AddMenuItem("Portrait:CLNK::0000ff::Missing", "", NONE, this(), 0, 0, "", 5, 0, 0); }
        func AddColoredTextSpec() { return AddMenuItem("CLNK", "", NONE, this(), 0, 0, "", 5, 0x112233, 0); }
        func AddUnknownPortrait() { return AddMenuItem("MISS", "", NONE, this(), 0, 0, "", 5, 0, 0); }
        func AddBadObjectRank() { return AddMenuItem("Bad object rank", "", NONE, this(), 0, 0, "", 3, 5, 0); }
        func AddBadObject() { return AddMenuItem("Bad object", "", NONE, this(), 0, 0, "", 4, 5, 0); }
        func AddRank() { return AddMenuItem("Rank", "", CLNK, this(), 4, 0, "", 1, 0, 0); }
        func AddIndexed() { return AddMenuItem("Indexed", "", CLNK, this(), 0, 0, "", 2, 3, 0); }
        func AddObjectRank() { return AddMenuItem("Object rank", "", NONE, this(), 0, 0, "", 3, this(), 0); }
        func AddObject() { return AddMenuItem("Object", "", NONE, this(), 0, 0, "", 4, this(), 0); }
        func Recolor() { return SetColorDw(0xabcdef); }
        func AddColor() { return AddMenuItem("Color", "", CLNK, this(), 0, 0, "", 6, 0x112233, 0); }
        func AddIndexedColor() { return AddMenuItem("Indexed color", "", CLNK, this(), 0, 0, "", 7, 2, 0x445566); }
        "#;
        let mut engine = Engine::with_seed(7);
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("script compiles");
        attach_one_pixel_portrait(&mut engine, &mut definition, "1");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, Vec::new())
                .expect("call succeeds")
        };

        assert_eq!(call(&mut engine, "OpenDialog"), Value::Bool(true));
        assert_eq!(call(&mut engine, "AddNone"), Value::Bool(true));
        assert_eq!(call(&mut engine, "AddDefinition"), Value::Bool(true));
        assert_eq!(call(&mut engine, "AddFallback"), Value::Bool(true));
        assert_eq!(call(&mut engine, "AddPortrait"), Value::Bool(true));
        assert_eq!(
            call(&mut engine, "AddMissingPortrait"),
            Value::Bool(false)
        );
        assert_eq!(call(&mut engine, "AddColoredTextSpec"), Value::Bool(true));
        assert_eq!(call(&mut engine, "AddUnknownPortrait"), Value::Bool(false));
        assert_eq!(call(&mut engine, "AddBadObjectRank"), Value::Bool(false));
        assert_eq!(call(&mut engine, "AddBadObject"), Value::Bool(false));
        for adder in [
            "AddRank",
            "AddIndexed",
            "AddObjectRank",
            "AddObject",
            "AddColor",
            "AddIndexedColor",
        ] {
            assert_eq!(call(&mut engine, adder), Value::Bool(true), "{adder}");
        }

        assert_eq!(call(&mut engine, "Recolor"), Value::Bool(true));

        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.items.len(), 11, "failed image recipes must not append");

        assert_eq!(menu.items[0].image, ObjectMenuImage::None);
        assert_eq!(menu.items[1].image, ObjectMenuImage::Definition);
        assert_eq!(menu.items[1].presentation_definition_id.as_deref(), Some("CLNK"));
        let fallback = &menu.items[2];
        assert_eq!(fallback.caption, "Fallback Clonk");
        assert_eq!(fallback.item_id, "MISS", "the command ID remains unchanged");
        assert_eq!(fallback.image, ObjectMenuImage::Definition);
        assert_eq!(fallback.presentation_definition_id.as_deref(), Some("CLNK"));
        let portrait = &menu.items[3];
        assert_eq!(portrait.caption, "", "TextSpec consumes the caption");
        assert_eq!(
            portrait.image,
            ObjectMenuImage::TextSpec {
                spec: "Portrait:CLNK::0000ff::1".to_string(),
                color: 0xff,
            }
        );
        assert!(!portrait.selectable);
        assert_eq!(
            menu.items[4].image,
            ObjectMenuImage::TextSpec {
                spec: "CLNK".to_string(),
                color: 0x112233,
            }
        );

        let rank = &menu.items[5];
        assert_eq!(rank.image, ObjectMenuImage::Rank { rank: 4 });
        assert_eq!(rank.count, 12_345_678, "rank consumes the item count");
        assert_eq!(
            menu.items[6].image,
            ObjectMenuImage::Indexed { index: 3 }
        );
        assert_eq!(
            menu.items[7].image,
            ObjectMenuImage::ObjectRank { object: clonk }
        );
        assert_eq!(
            menu.items[8].image,
            ObjectMenuImage::Object { object: clonk }
        );
        let cached_picture = menu.items[8]
            .picture_snapshot
            .as_ref()
            .expect("Object captures its picture source while the row is added");
        assert_eq!(cached_picture.definition_id, "CLNK");
        assert_eq!(cached_picture.color, 0, "later SetColorDw must not mutate the icon");
        assert_eq!(
            menu.items[9].image,
            ObjectMenuImage::Color { color: 0x112233 }
        );
        assert_eq!(
            menu.items[10].image,
            ObjectMenuImage::IndexedColor {
                index: 2,
                color: 0x445566,
            }
        );
    }

    #[test]
    fn add_menu_text_spec_uses_the_shared_cpp_grammar() {
        let script = r#"
        func OpenDialog() { return CreateMenu(CLNK, this(), this(), 0, "", 0, 3); }
        func AddBare() { return AddMenuItem("AB_D", "", NONE, this(), 0, 0, "", 5); }
        func AddIndexed() { return AddMenuItem("AB_D:  +12 trailing", "", NONE, this(), 0, 0, "", 5); }
        func AddDecimalPrefix() { return AddMenuItem("AB_D:0x10", "", NONE, this(), 0, 0, "", 5); }
        func AddNegativeZero() { return AddMenuItem("AB_D:-0tail", "", NONE, this(), 0, 0, "", 5); }
        func AddPortrait() { return AddMenuItem("Portrait:cowb::nope::captain1", "", NONE, this(), 0, 0, "", 5, 0x123456); }
        func AddIcon() { return AddMenuItem("Ico:LockedTrailing", "", NONE, this(), 0, 0, "", 5); }
        func AddLowercase() { return AddMenuItem("abcd", "", NONE, this(), 0, 0, "", 5); }
        func AddNegative() { return AddMenuItem("AB_D:-1", "", NONE, this(), 0, 0, "", 5); }
        func AddMissingPortrait() { return AddMenuItem("Portrait:cowb::missing", "", NONE, this(), 0, 0, "", 5); }
        func AddEmptyPortrait() { return AddMenuItem("Portrait:cowb::", "", NONE, this(), 0, 0, "", 5); }
        func AddLowercaseIcon() { return AddMenuItem("ico:Locked", "", NONE, this(), 0, 0, "", 5); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("AB_D", "Uppercase", "").expect("definition registers");
        engine.register_script_definition("abcd", "Lowercase", "").expect("definition registers");
        let mut portrait_definition =
            Definition::from_script("cowb", "Portrait", "").expect("definition compiles");
        attach_one_pixel_portrait(&mut engine, &mut portrait_definition, "Captain1");
        engine
            .register_definition(portrait_definition)
            .expect("definition registers");
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, Vec::new())
                .expect("call succeeds")
        };

        assert_eq!(call(&mut engine, "OpenDialog"), Value::Bool(true));
        for adder in [
            "AddBare",
            "AddIndexed",
            "AddDecimalPrefix",
            "AddNegativeZero",
            "AddPortrait",
            "AddIcon",
        ] {
            assert_eq!(call(&mut engine, adder), Value::Bool(true), "{adder}");
        }
        for adder in [
            "AddLowercase",
            "AddNegative",
            "AddMissingPortrait",
            "AddEmptyPortrait",
            "AddLowercaseIcon",
        ] {
            assert_eq!(call(&mut engine, adder), Value::Bool(false), "{adder}");
        }

        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.items.len(), 6, "invalid TextSpecs must not append");
        assert_eq!(
            menu.items[4].image,
            ObjectMenuImage::TextSpec {
                spec: "Portrait:cowb::nope::captain1".to_string(),
                color: 0x123456,
            },
            "an invalid inline portrait color retains the caller fallback"
        );
    }

    #[test]
    fn menu_definition_picture_phase_preserves_index_and_clips_out_of_bounds() {
        // C4Def::Picture2Facet fixes the source phase when AddMenuItem runs;
        // it does not validate the phase. Drawing later clips the facet
        // against the graphics surface instead of silently substituting
        // phase zero (C4Def.cpp:1374-1378).
        let mut definition =
            Definition::from_script("PHAS", "Phases", "").expect("definition compiles");
        definition.set_picture(Some(DefinitionPicture {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }));
        definition.set_sprite_image(Some(DefinitionSpriteImage {
            width: 3,
            height: 1,
            pixels: std::sync::Arc::from([
                0xff, 0, 0, 0xff, 0, 0xff, 0, 0xff, 0, 0, 0xff, 0xff,
            ]),
            color_mask: None,
        }));
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");

        let phase_two = engine
            .definition_picture_phase_image("PHAS", 2)
            .expect("phase two image");
        assert_eq!((phase_two.width(), phase_two.height()), (1, 1));
        assert_eq!(&*phase_two.pixels(), &[0, 0, 0xff, 0xff]);

        let outside = engine
            .definition_picture_phase_image("PHAS", 5)
            .expect("out-of-range phases still retain a facet");
        assert_eq!((outside.width(), outside.height()), (1, 1));
        assert_eq!(&*outside.pixels(), &[0, 0, 0, 0]);
    }

    #[test]
    fn real_clonk_category_agrees_across_definition_object_and_reflection() {
        // C4DefCore::Load adds C4D_CrewMember for any nonzero CrewMember
        // before validating the low-five-bit sort category (C4Def.cpp:
        // 206-233). The shipped CLNK is Living + SelectHomebase in text, so
        // every runtime and reflection view must expose the derived bit too.
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..");
        let group = clonk_resources::Group::open(
            repository.join("content/Objects.c4d/Crew.c4d/Clonk.c4d"),
        )
        .expect("open shipped Clonk definition");
        let resource =
            ResourceDefinitionData::load(&group).expect("load shipped Clonk definition");
        let expected = CATEGORY_LIVING | (1 << 11) | (1 << 18);
        assert_eq!(resource.core.category, expected);

        let clonk_definition =
            Definition::from_resource(&resource).expect("compile shipped Clonk definition");
        assert_eq!(clonk_definition.category(), expected);
        let probe_definition = Definition::from_script(
            "CATP",
            "Category probe",
            r#"#strict 2
func Probe(object target)
{
    var no_target;
    return [GetCategory(target), GetCategory(no_target, CLNK), GetDefCoreVal("Category", "DefCore", CLNK)];
}
"#,
        )
        .expect("category probe compiles");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(clonk_definition)
            .expect("Clonk definition registers");
        engine
            .register_definition(probe_definition)
            .expect("probe definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK").with_loaded(true))
            .expect("loaded Clonk spawns");
        let probe = engine
            .spawn_object(SpawnConfig::new("CATP").with_loaded(true))
            .expect("loaded probe spawns");

        assert_eq!(
            engine.object_snapshot(clonk).expect("Clonk exists").category,
            expected
        );
        let probe_index = engine.find_object_index(probe).expect("probe exists");
        assert_eq!(
            engine
                .call_object_function(
                    probe_index,
                    "Probe",
                    vec![Value::Object(clonk.as_u64())],
                )
                .expect("category probe succeeds"),
            Value::Array(vec![
                Value::Int(expected),
                Value::Int(expected),
                Value::Int(expected),
            ])
        );
    }

    #[test]
    fn knight_include_chain_inherits_clonk_rank_strip_and_base_count() {
        // C4Def::IncludeDefinition forwards non-owned rank graphics through
        // CLNK -> KNIG -> KING. The two shipped ImgRank menus in Kingdoms use
        // these derived crew IDs and must not fall back to the global strip.
        let paths = [
            "content/Objects.c4d/Crew.c4d/Clonk.c4d",
            "content/Knights.c4d/Crew.c4d/Knight.c4d",
            "content/Knights.c4d/Crew.c4d/King.c4d",
        ];
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..");
        let mut engine = Engine::new();
        for path in paths {
            let group = clonk_resources::Group::open(repository.join(path))
                .expect("open shipped crew definition");
            let resource = ResourceDefinitionData::load(&group)
                .expect("load shipped crew definition");
            engine
                .register_definition(
                    Definition::from_resource(&resource).expect("compile crew definition"),
                )
                .expect("register crew definition");
        }
        engine.resolve_includes().expect("resolve crew includes");

        for definition_id in ["CLNK", "KNIG", "KING"] {
            let image = engine
                .definition_rank_symbols_image(definition_id)
                .expect("custom rank strip inherited");
            assert_eq!((image.width(), image.height()), (464, 16));
            assert_eq!(
                engine.definition_rank_symbol_count(definition_id),
                Some(24)
            );
        }
    }

    #[test]
    fn object_menu_picture_snapshot_captures_same_call_mutation_and_round_trips() {
        let script = r#"
        func Open() { return CreateMenu(CLNK, this(), this(), 0, "Choose"); }
        func Capture() {
            SetColorDw(0x123456);
            SetClrModulation(0x70402010);
            ChangeDef(NEWW);
            return AddMenuItem("Object", "", NONE, this(), 0, 0, "", 4, this(), 0);
        }
        "#;
        let mut engine = Engine::new();
        engine.register_script_definition("NEWW", "New", "").expect("new definition registers");
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine
            .apply_object_update(
                clonk,
                ObjectUpdate {
                    base_graphics: Some(Some(ObjectBaseGraphics {
                        definition: "CLNK".to_string(),
                        graphics_name: Some("Old".to_string()),
                        blit_mode: 0,
                    })),
                    ..ObjectUpdate::new()
                },
            )
            .expect("custom graphics apply");
        engine.tick_without_snapshot().expect("tick succeeds");
        let call = |engine: &mut Engine, function: &str| {
            let index = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(index, function, Vec::new())
                .expect("script call succeeds")
        };
        assert_eq!(call(&mut engine, "Open"), Value::Bool(true));
        assert_eq!(call(&mut engine, "Capture"), Value::Bool(true));

        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu exists");
        let item = menu.items.first().expect("captured row");
        assert_eq!(item.presentation_definition_id.as_deref(), Some("NEWW"));
        let picture = item.picture_snapshot.as_ref().expect("picture snapshot");
        assert_eq!(picture.definition_id, "NEWW");
        assert_eq!(picture.base_graphics, None);
        assert_eq!(picture.color, 0, "non-CBO ChangeDef clears object color");
        assert_eq!(picture.color_modulation, 0x70402010);

        let encoded = serde_json::to_string(item).expect("menu item serializes");
        let decoded: ObjectMenuItem =
            serde_json::from_str(&encoded).expect("menu item deserializes");
        assert_eq!(decoded, *item);
    }

    #[test]
    fn object_menu_picture_caches_foreign_temporary_overlay_before_removal() {
        // Hazard's Weapon/Chooser menus create a temporary icon object,
        // configure a Picture overlay and transform from the caller, add an
        // ImgObject row, then remove the source object in the same call.
        let script = r#"
        func Configure() {
            var icon = CreateObject(TEMP, 0, 0, NO_OWNER);
            CreateMenu(CTRL, this(), this(), 0, "Choose");
            SetGraphics(0, icon, PICT, 1, GFXOV_MODE_Picture);
            SetObjDrawTransform(650, 0, 5000, 0, 650, 5000, icon, 1);
            var result = AddMenuItem("Icon", "", NONE, this(), 0, 0, "", 4, icon, 0);
            RemoveObject(icon);
            return result;
        }
        "#;
        let mut engine = Engine::new();
        for (id, definition_script) in [("TEMP", ""), ("PICT", ""), ("CTRL", script)] {
            engine
                .register_script_definition(id, id, definition_script)
                .expect("definition registers");
        }
        let controller = engine
            .spawn_object(SpawnConfig::new("CTRL"))
            .expect("controller spawns");
        engine.tick_without_snapshot().expect("tick succeeds");
        let index = engine
            .find_object_index(controller)
            .expect("controller exists");
        assert_eq!(
            engine
                .call_object_function(index, "Configure", Vec::new())
                .expect("configure succeeds"),
            Value::Bool(true)
        );

        let menu = engine
            .debug_object_menu(controller.as_u64())
            .expect("controller exists")
            .expect("menu exists");
        let picture = menu.items[0]
            .picture_snapshot
            .as_ref()
            .expect("temporary picture captured");
        assert_eq!(picture.graphics_overlays.len(), 1);
        let overlay = &picture.graphics_overlays[0];
        assert_eq!(overlay.definition.as_deref(), Some("PICT"));
        assert_eq!(overlay.mode, GraphicsOverlayMode::Picture);
        assert_eq!(
            overlay.transform,
            Some(DrawTransform::from_components(0.65, 0.65, 5.0, 5.0))
        );
    }

    #[test]
    fn menu_item_caches_custom_components_from_the_menu_target_builder_like_cpp() {
        // C4MenuItem resolves and caches components at construction time with
        // pObjInstance=null and pBuilder=Menu->GetParentObject(), i.e. the
        // MENU TARGET rather than the command object (C4Menu.cpp:76-97;
        // C4Def.cpp:1266-1275,1322-1355). CreateMenu forwards iExtra=1 as
        // C4MN_Extra_Components (C4Script.cpp:1420-1448).
        let builder_script = r#"
        local component_mode;
        func SetComponentMode(value) { component_mode = value; }
        func MenuComponentMode() { return component_mode; }
        "#;
        let command_script = r#"
        func MenuComponentMode() { return 0; }
        func Open(builder) {
            CreateMenu(CXCN, builder, this(), C4MN_Extra_Components, "Build");
            return AddMenuItem("Build", "Choose", DYNA, builder);
        }
        "#;
        let item_script = r#"#strict
        protected func GetCustomComponents(builder) {
            if (builder->~MenuComponentMode()) return [WOOD, WOOD, METL];
            return [GOLD];
        }
        "#;

        let mut engine = Engine::with_seed(7);
        for id in ["WOOD", "METL", "GOLD", "ROCK"] {
            engine.register_script_definition(id, id, "").expect("component definition registers");
        }
        engine
            .register_script_definition("BULD", "Builder", builder_script)
            .expect("builder registers");
        engine
            .register_script_definition("CMND", "Command", command_script)
            .expect("command registers");
        let mut item =
            Definition::from_script("DYNA", "Dynamic", item_script).expect("item compiles");
        item.set_components(vec![DefinitionComponent {
            id: "ROCK".to_string(),
            count: 9,
        }]);
        engine.register_definition(item).expect("item registers");
        let builder = engine
            .spawn_object(SpawnConfig::new("BULD"))
            .expect("builder spawns");
        let command = engine
            .spawn_object(SpawnConfig::new("CMND"))
            .expect("command spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine,
                    object: ObjectId,
                    function: &str,
                    args: Vec<Value>| {
            let index = engine.find_object_index(object).expect("object exists");
            engine
                .call_object_function(index, function, args)
                .expect("script call succeeds")
        };
        call(
            &mut engine,
            builder,
            "SetComponentMode",
            vec![Value::Int(1)],
        );
        assert_eq!(
            call(
                &mut engine,
                command,
                "Open",
                vec![object_reference_value(builder)],
            ),
            Value::Bool(true)
        );
        // The C4MenuItem owns the resolved C4IDList. A later builder-state
        // change must not re-run GetCustomComponents during presentation.
        call(
            &mut engine,
            builder,
            "SetComponentMode",
            vec![Value::Int(0)],
        );

        let menu = engine
            .debug_object_menu(builder.as_u64())
            .expect("builder exists")
            .expect("builder owns the menu");
        assert_eq!(menu.command_object, Some(command));
        assert_eq!(menu.extra, ObjectMenuExtra::Components);
        assert_eq!(
            menu.items[0].components,
            vec![
                ObjectMenuComponent {
                    definition_id: "WOOD".to_string(),
                    count: 2,
                },
                ObjectMenuComponent {
                    definition_id: "METL".to_string(),
                    count: 1,
                },
            ],
            "the target BULD, not command CMND, is the custom-component builder"
        );
    }

    #[test]
    fn menu_item_custom_component_fallbacks_and_force_no_desc_match_cpp() {
        // GetCustomComponents overrides DefCore components only when it
        // returns an array: an EMPTY array is still an override, while a
        // missing function or non-array result falls back to Def->Component
        // (C4Def.cpp:1266-1275,1322-1355). An omitted AddMenuItem info
        // caption falls back to pDef->GetDesc unless ForceNoDesc is set
        // (C4Script.cpp:1590-1594).
        let command_script = r#"
        func Open(builder) {
            CreateMenu(CXCN, builder, this(), C4MN_Extra_Components, "Build");
            AddMenuItem("Empty", "Choose", EMPT, builder);
            AddMenuItem("Non-array", "Choose", NARR, builder);
            AddMenuItem("Missing", "Choose", MISS, builder);
            return AddMenuItem("No description", "Choose", MISS, builder,
                               0, 0, 0, C4MN_Add_ForceNoDesc);
        }
        "#;

        let mut engine = Engine::with_seed(7);
        for id in ["WOOD", "METL", "GOLD"] {
            engine.register_script_definition(id, id, "").expect("component definition registers");
        }
        engine.register_script_definition("BULD", "Builder", "").expect("builder registers");
        engine
            .register_script_definition("CMND", "Command", command_script)
            .expect("command registers");

        let register_item = |engine: &mut Engine,
                             id: &str,
                             script: &str,
                             component: &str,
                             count: i32,
                             description: &str| {
            let mut item = Definition::from_script(id, id, script).expect("item compiles");
            item.set_components(vec![DefinitionComponent {
                id: component.to_string(),
                count,
            }]);
            item.set_description(Some(description.to_string()));
            engine.register_definition(item).expect("item registers");
        };
        register_item(
            &mut engine,
            "EMPT",
                        "#strict\nprotected func GetCustomComponents(builder) { return []; }",
            "WOOD",
            5,
            "Empty custom description.",
        );
        register_item(
            &mut engine,
            "NARR",
            "protected func GetCustomComponents(builder) { return 17; }",
            "METL",
            3,
            "Non-array description.",
        );
        register_item(
            &mut engine,
            "MISS",
            "",
            "GOLD",
            2,
            "Missing-hook description.",
        );

        let builder = engine
            .spawn_object(SpawnConfig::new("BULD"))
            .expect("builder spawns");
        let command = engine
            .spawn_object(SpawnConfig::new("CMND"))
            .expect("command spawns");
        engine.tick_without_snapshot().expect("tick succeeds");
        let command_index = engine.find_object_index(command).expect("command exists");
        assert_eq!(
            engine
                .call_object_function(
                    command_index,
                    "Open",
                    vec![object_reference_value(builder)],
                )
                .expect("Open succeeds"),
            Value::Bool(true)
        );

        let menu = engine
            .debug_object_menu(builder.as_u64())
            .expect("builder exists")
            .expect("builder owns the menu");
        assert_eq!(menu.extra, ObjectMenuExtra::Components);
        assert_eq!(menu.items.len(), 4);
        assert!(
            menu.items[0].components.is_empty(),
            "an empty custom array overrides EMPT's static WOOD component"
        );
        assert_eq!(menu.items[0].info_caption, "Empty custom description.");
        assert_eq!(
            menu.items[1].components,
            vec![ObjectMenuComponent {
                definition_id: "METL".to_string(),
                count: 3,
            }],
            "a non-array result falls back to NARR's DefCore components"
        );
        assert_eq!(menu.items[1].info_caption, "Non-array description.");
        assert_eq!(
            menu.items[2].components,
            vec![ObjectMenuComponent {
                definition_id: "GOLD".to_string(),
                count: 2,
            }],
            "a missing hook falls back to MISS's DefCore components"
        );
        assert_eq!(menu.items[2].info_caption, "Missing-hook description.");
        assert_eq!(menu.items[3].components, menu.items[2].components);
        assert_eq!(
            menu.items[3].info_caption, "",
            "C4MN_Add_ForceNoDesc suppresses the omitted-caption fallback"
        );
    }

    #[test]
    fn context_menu_overlay_write_keeps_the_objects_other_overlays() {
        // execute_context_menu's non-legacy branch dispatches through
        // call_menu_callback, whose carrier context is built separately from
        // the object's primary scope. C4Object::GetGraphicsOverlay splices one
        // node into the live pGfxOverlay list (src/C4Object.cpp:5962-5977), so
        // a context-menu callback that sets one overlay must leave the rest.
        let script = r#"
        func Arm() {
            return SetGraphics(0, this(), PICT, 1, GFXOV_MODE_Action, "O20");
        }
        func ContextPaint(menuObj) {
            SetGraphics(0, this(), PICT, 9, GFXOV_MODE_Action, "Pointer");
            return 1;
        }
        "#;
        let mut engine = Engine::with_seed(7);
        for (id, definition_script) in [("PICT", ""), ("CLNK", script)] {
            engine
                .register_script_definition(id, id, definition_script)
                .expect("definition registers");
        }
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let index = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .call_object_function(index, "Arm", Vec::new())
            .expect("overlay 1 installs");
        engine
            .execute_context_menu(clonk, "ContextPaint")
            .expect("context menu entry runs");

        let overlays = engine
            .object_snapshot(clonk)
            .expect("clonk survives")
            .graphics_overlays
            .iter()
            .map(|overlay| overlay.id)
            .collect::<Vec<_>>();
        assert_eq!(
            overlays,
            vec![1, 9],
            "the context-menu callback wrote overlay 9; overlay 1 must survive"
        );
    }

    #[test]
    fn menu_callback_overlay_write_keeps_the_objects_other_overlays() {
        // C4Object::GetGraphicsOverlay splices one node into the live
        // pGfxOverlay list (src/C4Object.cpp:5962-5977), so a menu callback
        // that sets one overlay leaves the object's others alone. The Rust
        // scope publishes its whole overlay list, so the menu callback carrier
        // context must be seeded with the object's real overlays -- the same
        // omission that truncated ClonkMars' MHUD from an effect timer.
        let script = r#"
        func OnMenuSelection(sel, menuObj) {
            SetGraphics(0, this(), PICT, 9, GFXOV_MODE_Action, "Pointer");
        }
        func OpenMenu() {
            CreateMenu(WIPF, this(), this(), 0, "Choose");
            AddMenuItem("A", "CmdA", WIPF, this());
            AddMenuItem("B", "CmdB", WIPF, this());
            return 1;
        }
        func Arm() {
            return SetGraphics(0, this(), PICT, 1, GFXOV_MODE_Action, "O20");
        }
        func Sel(i) { return SelectMenuItem(i, this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        for (id, definition_script) in [("PICT", ""), ("CLNK", script)] {
            engine
                .register_script_definition(id, id, definition_script)
                .expect("definition registers");
        }
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };
        call(&mut engine, "Arm", Vec::new());
        call(&mut engine, "OpenMenu", Vec::new());
        call(&mut engine, "Sel", vec![Value::Int(1)]);

        let overlays = engine
            .object_snapshot(clonk)
            .expect("clonk survives")
            .graphics_overlays
            .iter()
            .map(|overlay| overlay.id)
            .collect::<Vec<_>>();
        assert_eq!(
            overlays,
            vec![1, 9],
            "the menu callback wrote overlay 9; overlay 1 must survive"
        );
    }

    #[test]
    fn select_menu_item_moves_selection_and_fires_on_menu_selection_like_cpp() {
        // FnSelectMenuItem (C4Script.cpp:1736-1741) -> C4Menu::SetSelection
        // (C4Menu.cpp:557-594): only SELECTABLE items move the selection,
        // but the call returns true whenever a menu is active, and
        // fDoCalls fires OnMenuSelection(Selection, ParentObject) on the
        // command object EITHER way (C4ObjectMenu::OnSelectionChanged,
        // C4ObjectMenu.cpp:93-104) — with the (possibly unchanged) final
        // selection.
        let script = r#"
        local lastSel;
        func OnMenuSelection(sel, menuObj) { lastSel = sel; }
        func NoMenu() { return SelectMenuItem(0, this()); }
        func OpenMenu() {
            CreateMenu(WIPF, this(), this(), 0, "Choose");
            AddMenuItem("A", "CmdA", WIPF, this());
            AddMenuItem("B", "CmdB", WIPF, this());
            AddMenuItem("C", "", WIPF, this());
            return 1;
        }
        func Sel(i) { return SelectMenuItem(i, this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };
        let last_sel = |engine: &Engine| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine.objects[idx]
                .state
                .local_vars
                .get("lastSel")
                .cloned()
                .unwrap_or(Value::Nil)
        };

        assert_eq!(
            call(&mut engine, "NoMenu", Vec::new()),
            Value::Bool(false),
            "no menu -> false (C4Script.cpp:1739)"
        );
        assert_eq!(call(&mut engine, "OpenMenu", Vec::new()), Value::Int(1));

        // Selectable item: selection moves, callback sees the new index.
        assert_eq!(
            call(&mut engine, "Sel", vec![Value::Int(1)]),
            Value::Bool(true)
        );
        assert_eq!(last_sel(&engine), Value::Int(1));
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.selection, 1);

        // Non-selectable item: selection stays, call still true, callback
        // fires with the OLD selection.
        assert_eq!(
            call(&mut engine, "Sel", vec![Value::Int(2)]),
            Value::Bool(true)
        );
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.selection, 1, "item without command is not selectable");
        assert_eq!(last_sel(&engine), Value::Int(1));

        // Out of range behaves the same.
        assert_eq!(
            call(&mut engine, "Sel", vec![Value::Int(9)]),
            Value::Bool(true)
        );
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.selection, 1);
    }

    #[test]
    fn clear_menu_items_resets_without_callback_and_keeps_the_menu_open() {
        // FnClearMenuItems -> C4Menu::ClearItems(true) (C4Script.cpp:
        // 5149-5159; C4Menu.cpp:975-988): `true` resets the selection, but
        // SetSelection receives fDoCalls=false, so OnMenuSelection does NOT
        // run. The menu allocation and identity remain active.
        let script = r#"#strict
local selection_calls, last_selection, cancel_calls;
func OnMenuSelection(selection, menu_object) {
    selection_calls += 1;
    last_selection = selection;
}
func MenuQueryCancel() { cancel_calls += 1; return true; }
func ClearWithoutObject() { return ClearMenuItems(); }
func NoObject() { return DefinitionCall(CLNK, "ClearWithoutObject"); }
func NoMenu() { return ClearMenuItems(); }
func OpenMenu() {
    CreateMenu(WIPF, this(), this(), 0, "Choose");
    AddMenuItem("A", "CmdA", WIPF, this());
    AddMenuItem("B", "CmdB", WIPF, this());
    SelectMenuItem(1, this());
    return true;
}
func ResetCallbacks() {
    selection_calls = 0;
    last_selection = 99;
    cancel_calls = 0;
    return true;
}
func Clear() { return ClearMenuItems(); }
func AddAgain() { return AddMenuItem("C", "CmdC", WIPF, this()); }
func ReadMenu() { return GetMenu(); }
"#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str| {
            let index = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(index, name, Vec::new())
                .expect("call succeeds")
        };
        assert_eq!(call(&mut engine, "NoObject"), Value::Bool(false));
        assert_eq!(call(&mut engine, "NoMenu"), Value::Bool(false));
        assert_eq!(call(&mut engine, "OpenMenu"), Value::Bool(true));
        assert_eq!(call(&mut engine, "ResetCallbacks"), Value::Bool(true));
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu is open");
        assert_eq!(menu.items.len(), 2);
        assert_eq!(menu.selection, 1);

        assert_eq!(call(&mut engine, "Clear"), Value::Bool(true));
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("ClearMenuItems keeps the menu open");
        assert!(menu.items.is_empty());
        assert_eq!(menu.selection, -1);
        assert_eq!(call(&mut engine, "ReadMenu"), Value::C4Id("WIPF".into()));
        let index = engine.find_object_index(clonk).expect("clonk exists");
        let locals = &engine.objects[index].state.local_vars;
        assert_eq!(locals.get("selection_calls"), Some(&Value::Nil));
        assert_eq!(locals.get("last_selection"), Some(&Value::Int(99)));
        assert_eq!(locals.get("cancel_calls"), Some(&Value::Nil));

        assert_eq!(call(&mut engine, "AddAgain"), Value::Bool(true));
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("same menu remains open");
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.selection, 0);
        assert_eq!(call(&mut engine, "Clear"), Value::Bool(true));
    }

    #[test]
    fn exit_closes_the_object_menu_like_cpp() {
        // C4Object::Exit (C4Object.cpp:1530-1562) force-closes the exiting
        // object's menu among its "Misc updates" (CloseMenu(true),
        // C4Object.cpp:1555) — synchronously, so a GetMenu later in the
        // same script call already sees it gone.
        let script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func LeaveAndRead() { Exit(); return GetMenu(this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        engine.register_script_definition("HUT1", "Hut", "").expect("hut registers");
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("hut spawns");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK").with_container(hut))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "OpenMenu", Vec::new())
                .expect("OpenMenu succeeds"),
            Value::Bool(true)
        );
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "LeaveAndRead", Vec::new())
                .expect("LeaveAndRead succeeds"),
            Value::Int(0),
            "Exit closes the menu before the same-call GetMenu (C4Object.cpp:1555)"
        );
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(engine.objects[idx].state.container, None, "exit folded");
        assert_eq!(
            engine.debug_object_menu(clonk.as_u64()),
            Some(None),
            "the menu stays closed after the fold"
        );
    }

    #[test]
    fn enter_closes_the_object_menu_and_a_later_create_menu_survives_like_cpp() {
        // C4Object::Enter (C4Object.cpp:1565-1614) force-closes the entering
        // object's menu among its "Failsafe updates" (CloseMenu(true),
        // C4Object.cpp:1594). The close happens AT the Enter — a CreateMenu
        // later in the same call opens a fresh menu that stays.
        let script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func BoardAndRead(hut) { Enter(hut); return GetMenu(this()); }
        func BoardThenReopen(hut) { Enter(hut); Exit(); Enter(hut); CreateMenu(MENU, this(), this(), 0, "After"); return GetMenu(this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        engine.register_script_definition("HUT1", "Hut", "").expect("hut registers");
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("hut spawns");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "OpenMenu", Vec::new())
                .expect("OpenMenu succeeds"),
            Value::Bool(true)
        );
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "BoardAndRead", vec![object_reference_value(hut)])
                .expect("BoardAndRead succeeds"),
            Value::Int(0),
            "Enter closes the menu before the same-call GetMenu (C4Object.cpp:1594)"
        );
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(engine.objects[idx].state.container, Some(hut));
        assert_eq!(engine.debug_object_menu(clonk.as_u64()), Some(None));

        // Exit+Enter then CreateMenu in ONE call: the new menu must survive
        // the container-change fold (C++ closed at Enter time, then the
        // script reopened — the reopened menu stays).
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(idx, "BoardThenReopen", vec![object_reference_value(hut)])
                .expect("BoardThenReopen succeeds"),
            Value::C4Id("MENU".into())
        );
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("menu reopened after Enter survives the fold");
        assert_eq!(menu.identification, Value::C4Id("MENU".into()));
    }

    #[test]
    fn enter_and_exit_force_close_the_moving_objects_menu_before_callbacks() {
        let moving_script = r#"
        local callback_order, callback_menu_order, query_calls;
        public func RecordCallback(int step)
        {
            callback_order = callback_order * 10 + step;
            if (GetMenu(this())) callback_menu_order = callback_menu_order * 10 + step;
            return(1);
        }
        public func OpenMenu()
        {
            return CreateMenu(WIPF, this(), this(), 0, "Choose");
        }
        public func ResetAndOpen()
        {
            callback_order = 0;
            callback_menu_order = 0;
            query_calls = 0;
            return OpenMenu();
        }
        public func Board(object container) { return Enter(container); }
        public func Leave() { return Exit(); }
        protected func Entrance(object container) { return RecordCallback(2); }
        protected func Departure(object container) { return RecordCallback(4); }
        protected func MenuQueryCancel()
        {
            query_calls++;
            return(1);
        }
        "#;
        let container_script = r#"
        protected func Collection2(object item) { return item->RecordCallback(1); }
        protected func Ejection(object item) { return item->RecordCallback(3); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine
            .register_script_definition("CLNK", "Clonk", moving_script)
            .expect("moving object registers");
        engine
            .register_script_definition("HUT1", "Hut", container_script)
            .expect("container registers");
        let container = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("container spawns");
        let moving = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("moving object spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let index = engine
                .find_object_index(moving)
                .expect("moving object remains");
            engine
                .call_object_function(index, name, args)
                .expect("callback probe call succeeds")
        };
        assert_eq!(call(&mut engine, "ResetAndOpen", Vec::new()), Value::Bool(true));
        assert_eq!(
            call(
                &mut engine,
                "Board",
                vec![object_reference_value(container)],
            ),
            Value::Bool(true)
        );

        let index = engine
            .find_object_index(moving)
            .expect("moving object entered");
        assert_eq!(engine.objects[index].state.container, Some(container));
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_order"),
            Some(&Value::Int(12)),
            "Collection2 runs before Entrance"
        );
        assert!(
            !engine.objects[index]
                .state
                .local_vars
                .get("callback_menu_order")
                .is_some_and(Value::as_bool),
            "both Enter callbacks observe the old menu already closed"
        );
        assert!(
            !engine.objects[index]
                .state
                .local_vars
                .get("query_calls")
                .is_some_and(Value::as_bool),
            "forced Enter close bypasses a denying MenuQueryCancel"
        );

        assert_eq!(call(&mut engine, "OpenMenu", Vec::new()), Value::Bool(true));
        assert_eq!(call(&mut engine, "Leave", Vec::new()), Value::Bool(true));

        let index = engine
            .find_object_index(moving)
            .expect("moving object exited");
        assert_eq!(engine.objects[index].state.container, None);
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_order"),
            Some(&Value::Int(1234)),
            "Ejection runs before Departure"
        );
        assert!(
            !engine.objects[index]
                .state
                .local_vars
                .get("callback_menu_order")
                .is_some_and(Value::as_bool),
            "both Exit callbacks observe the old menu already closed"
        );
        assert!(
            !engine.objects[index]
                .state
                .local_vars
                .get("query_calls")
                .is_some_and(Value::as_bool),
            "forced Exit close bypasses a denying MenuQueryCancel"
        );
        assert_eq!(engine.debug_object_menu(moving.as_u64()), Some(None));
    }

    #[test]
    fn engine_internal_container_moves_close_the_object_menu_like_cpp() {
        // Engine-internal container moves (collection cross-check
        // lib.rs `with_container` update, grab/enter DirectCom arms) are
        // C4Object::Enter/Exit too — both force-close the MOVING object's
        // menu (CloseMenu(true), C4Object.cpp:1555/:1594).
        let script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        engine.register_script_definition("HUT1", "Hut", "").expect("hut registers");
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("hut spawns");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let idx = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .call_object_function(idx, "OpenMenu", Vec::new())
            .expect("OpenMenu succeeds");
        assert!(engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .is_some());

        // Host-driven Enter, like the collection cross-check
        // (C4Object::Collect -> Enter, C4Object.cpp:5698 -> :1552).
        engine
            .apply_object_update(clonk, ObjectUpdate::new().with_container(hut))
            .expect("enter applies");
        assert_eq!(
            engine.debug_object_menu(clonk.as_u64()),
            Some(None),
            "the entering object's menu closes (C4Object.cpp:1594)"
        );

        // Reopen, then a host-driven Exit (e.g. drop) closes it again.
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .call_object_function(idx, "OpenMenu", Vec::new())
            .expect("OpenMenu succeeds");
        assert!(engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .is_some());
        engine
            .apply_object_update(clonk, ObjectUpdate::new().clear_container())
            .expect("exit applies");
        assert_eq!(
            engine.debug_object_menu(clonk.as_u64()),
            Some(None),
            "the exiting object's menu closes (C4Object.cpp:1555)"
        );
    }

    #[test]
    fn sync_clearance_closes_the_object_menu_like_cpp() {
        // C4Object::SyncClearance (C4Object.cpp:3829-3850) force-closes any
        // open menu among its no-save safeties (CloseMenu(true),
        // C4Object.cpp:3842) — menus never survive a Synchronize.
        let script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let idx = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .call_object_function(idx, "OpenMenu", Vec::new())
            .expect("OpenMenu succeeds");
        assert!(engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .is_some());

        engine
            .game_start_synchronize()
            .expect("game-start synchronization succeeds");
        assert_eq!(
            engine.debug_object_menu(clonk.as_u64()),
            Some(None),
            "SyncClearance closes menus (C4Object.cpp:3842)"
        );
    }

    #[test]
    fn control_set_command_soft_closes_the_menu_and_a_denial_aborts_like_cpp() {
        // C4Object::SetCommand with fControl (C4Object.cpp:3938-3981):
        // ClearCommands runs first (:3941), then the menu must agree to a
        // SOFT close — `if (!CloseMenu(false)) return;` (:3944-3946). A
        // MenuQueryCancel denial (C4ObjectMenu::IsCloseDenied,
        // C4ObjectMenu.cpp:57-76) keeps the menu open and aborts the whole
        // SetCommand: no ControlCommand overload, no command push — but the
        // stack stays cleared.
        let script = r#"
        local deny;
        local queried;
        func SetDeny(flag) { deny = flag; }
        func MenuQueryCancel(sel, menuObj) { queried = queried + 1; return deny; }
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        "#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        definition.configure_actions(Some("Idle".to_string()), actions);
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player registers");
        let clonk = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle")),
            )
            .expect("clonk spawns");
        engine.set_crew_cursor(1, Some(clonk)).expect("cursor set");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };
        let queried = |engine: &Engine| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine.objects[idx]
                .state
                .local_vars
                .get("queried")
                .cloned()
                .unwrap_or(Value::Nil)
        };

        // Undenied menu: the control command closes it and pushes.
        call(&mut engine, "OpenMenu", Vec::new());
        engine
            .player_object_command(1, CommandId::Dig, None, 10, 20)
            .expect("command routes");
        assert_eq!(
            engine.debug_object_menu(clonk.as_u64()),
            Some(None),
            "SetCommand(fControl) closed the undenied menu (C4Object.cpp:3945)"
        );
        assert_eq!(queried(&engine), Value::Int(1), "the soft close queried");
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine.objects[idx].commands.snapshot().command_names(),
            vec!["Dig".to_string()],
            "the command was set after the close"
        );

        // Denied menu: it survives, and the SetCommand aborts AFTER the
        // ClearCommands — the old Dig is gone, nothing new is pushed.
        // (Throw/Drop route as C4P_Command_Add and never touch the menu,
        // C4ObjectCom.cpp:1020-1036 — use another Set-mode command.)
        call(&mut engine, "OpenMenu", Vec::new());
        call(&mut engine, "SetDeny", vec![Value::Int(1)]);
        engine
            .player_object_command(1, CommandId::Dig, None, 30, 40)
            .expect("command routes");
        assert!(
            engine
                .debug_object_menu(clonk.as_u64())
                .expect("clonk exists")
                .is_some(),
            "the denied menu stays open (C4Object.cpp:3946)"
        );
        assert_eq!(queried(&engine), Value::Int(2));
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert!(
            engine.objects[idx].commands.snapshot().is_empty(),
            "the abort still cleared the stack (ClearCommands ran first, :3941)"
        );
    }

    #[test]
    fn get_menu_selection_reads_the_selection_or_minus_one_like_cpp() {
        // FnGetMenuSelection (C4Script.cpp:4310-4316): no object, no menu or
        // inactive menu -> -1; otherwise C4Menu::GetSelection() — the raw
        // Selection index (C4Menu.cpp:612-615), which itself is -1 while
        // nothing is selected (C4Menu::Default, C4Menu.cpp:284).
        let script = r#"
        func Read() { return GetMenuSelection(this()); }
        func ReadSelf() { return GetMenuSelection(); }
        func OpenEmpty() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func Fill() {
            AddMenuItem("Info", "", WIPF, this());
            AddMenuItem("A", "CmdA", WIPF, this());
            AddMenuItem("B", "CmdB", WIPF, this());
            return SelectMenuItem(2, this());
        }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, Vec::new())
                .expect("call succeeds")
        };

        assert_eq!(
            call(&mut engine, "Read"),
            Value::Int(-1),
            "no menu -> -1 (C4Script.cpp:4314)"
        );
        assert_eq!(call(&mut engine, "OpenEmpty"), Value::Bool(true));
        assert_eq!(
            call(&mut engine, "ReadSelf"),
            Value::Int(-1),
            "open menu without a selection reports its raw -1"
        );
        assert_eq!(call(&mut engine, "Fill"), Value::Bool(true));
        assert_eq!(
            call(&mut engine, "Read"),
            Value::Int(2),
            "the selected index is reported (C4Script.cpp:4315)"
        );
    }

    #[test]
    fn set_menu_size_clamps_and_keeps_zero_axes_like_cpp() {
        // FnSetMenuSize (C4Script.cpp:4483-4492): false without an active
        // menu; cols/rows clamp through BoundBy(0..50) and feed
        // C4Menu::SetSize (C4Menu.cpp:635-640), where a ZERO axis keeps the
        // previous value (`if (iToWdt) Columns = iToWdt;`). Menus start at
        // Columns = Lines = 0 (C4Menu::Default, C4Menu.cpp:299).
        let script = r#"
        func Resize(c, r) { return SetMenuSize(c, r, this()); }
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };
        let size = |engine: &Engine| {
            let menu = engine
                .debug_object_menu(clonk.as_u64())
                .expect("clonk exists")
                .expect("menu is open");
            (menu.columns, menu.lines)
        };

        assert_eq!(
            call(&mut engine, "Resize", vec![Value::Int(3), Value::Int(4)]),
            Value::Bool(false),
            "no menu -> false (C4Script.cpp:4489)"
        );
        call(&mut engine, "OpenMenu", Vec::new());
        assert_eq!(
            size(&engine),
            (5, 0),
            "InitMenu gives normal menus five columns (C4Menu.cpp:359-365)"
        );
        assert_eq!(
            call(&mut engine, "Resize", vec![Value::Int(3), Value::Int(4)]),
            Value::Bool(true)
        );
        assert_eq!(size(&engine), (3, 4));
        // Zero keeps the previous axis (C4Menu.cpp:637-638); a negative
        // clamps to 0 and thus also keeps; oversize clamps to 50.
        assert_eq!(
            call(&mut engine, "Resize", vec![Value::Int(0), Value::Int(7)]),
            Value::Bool(true)
        );
        assert_eq!(size(&engine), (3, 7));
        assert_eq!(
            call(&mut engine, "Resize", vec![Value::Int(99), Value::Int(-5)]),
            Value::Bool(true)
        );
        assert_eq!(size(&engine), (50, 7));
    }

    #[test]
    fn set_menu_text_progress_requires_an_explicit_menu_object_like_cpp() {
        // FnSetMenuTextProgress (C4Script.cpp:1750-1754): unlike most menu
        // fns there is NO cthr->Obj fallback — `if (!pMenuObj ||
        // !pMenuObj->Menu) return false;`. With an active menu it returns
        // C4Menu::SetTextProgress(n, false) (C4Menu.cpp:1079-1111), which
        // is true whenever the menu is active. An empty menu immediately
        // clears fTextProgressing because it has no unfinished rows.
        let script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func NoObj() { return SetMenuTextProgress(0); }
        func Prog(n) { return SetMenuTextProgress(n, this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };
        let progressing = |engine: &Engine| {
            engine
                .debug_object_menu(clonk.as_u64())
                .expect("clonk exists")
                .expect("menu is open")
                .text_progressing
        };

        assert_eq!(
            call(&mut engine, "Prog", vec![Value::Int(0)]),
            Value::Bool(false),
            "no menu -> false (C4Script.cpp:1752)"
        );
        call(&mut engine, "OpenMenu", Vec::new());
        assert_eq!(
            call(&mut engine, "NoObj", Vec::new()),
            Value::Bool(false),
            "nil menu object -> false even with a scope object (C4Script.cpp:1752)"
        );
        assert_eq!(
            call(&mut engine, "Prog", vec![Value::Int(5)]),
            Value::Bool(true)
        );
        assert!(
            !progressing(&engine),
            "an empty menu has no unfinished text"
        );
        assert_eq!(
            call(&mut engine, "Prog", vec![Value::Int(-1)]),
            Value::Bool(true)
        );
        assert!(
            !progressing(&engine),
            "negative n disables text progress (fTextProgressing = false)"
        );
    }

    #[test]
    fn menu_text_progress_distributes_a_shared_cpp_byte_budget() {
        // C4Menu::SetTextProgress and C4MenuItem::DoTextProgress
        // (C4Menu.cpp:105-126,1079-1111): the first empty-caption row is a
        // portrait; recognized markup costs no budget; option rows reveal
        // without consuming budget; ordinary text advances raw bytes.
        let script = r#"
        func OpenDialog() {
            CreateMenu(CLNK, this(), this(), 0, "", 0, 3);
            AddMenuItem("Portrait:CLNK::0000ff::1", "", NONE, this(), 0, 0, "", 5);
            AddMenuItem("<i>AB</i>", "", NONE, this());
            AddMenuItem("Continue", "Choose", CLNK, this());
            return AddMenuItem("éZ", "", NONE, this());
        }
        func Prog(n) { return SetMenuTextProgress(n, this()); }
        func AddLate() { return AddMenuItem("Late", "", NONE, this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("script compiles");
        attach_one_pixel_portrait(&mut engine, &mut definition, "1");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };
        let menu = |engine: &Engine| {
            engine
                .debug_object_menu(clonk.as_u64())
                .expect("clonk exists")
                .expect("menu is open")
        };

        assert_eq!(call(&mut engine, "OpenDialog", Vec::new()), Value::Bool(true));
        assert_eq!(
            call(&mut engine, "Prog", vec![Value::Int(0)]),
            Value::Bool(true)
        );
        let state = menu(&engine);
        assert!(state.text_progressing);
        assert_eq!(
            state
                .items
                .iter()
                .map(|item| item.text_display_progress)
                .collect::<Vec<_>>(),
            vec![-1, 0, 0, 0],
            "portrait is excluded and every text row starts hidden"
        );

        call(&mut engine, "Prog", vec![Value::Int(1)]);
        assert_eq!(
            menu(&engine).items[1].text_display_progress,
            4,
            "<i> is skipped and the A byte consumes the budget"
        );

        call(&mut engine, "Prog", vec![Value::Int(3)]);
        let state = menu(&engine);
        assert_eq!(state.items[1].text_display_progress, -1);
        assert_eq!(state.items[2].text_display_progress, -1);
        assert_eq!(
            state.items[3].text_display_progress, 1,
            "the remaining byte enters the two-byte UTF-8 character"
        );

        assert_eq!(call(&mut engine, "AddLate", Vec::new()), Value::Bool(true));
        assert_eq!(menu(&engine).items[4].text_display_progress, 0);

        call(&mut engine, "Prog", vec![Value::Int(0)]);
        engine.tick_without_snapshot().expect("menu progress tick succeeds");
        assert_eq!(
            menu(&engine).items[1].text_display_progress,
            4,
            "C4Menu::Execute advances one shared byte per object tick"
        );

        call(&mut engine, "Prog", vec![Value::Int(-1)]);
        let state = menu(&engine);
        assert!(!state.text_progressing);
        assert!(
            state
                .items
                .iter()
                .all(|item| item.text_display_progress == -1)
        );
    }

    #[test]
    fn set_menu_decoration_requires_a_known_def_and_a_menu_like_cpp() {
        // FnSetMenuDecoration (C4Script.cpp:1737-1748): no cthr->Obj
        // fallback — `if (!pMenuObj || !pMenuObj->Menu) return false;`.
        // FrameDecoration::SetByDef (C4GuiDialogs.cpp:110-142) fails on an
        // unknown def (C4Id2Def null, :113-114); with a known def it stores
        // the deco and returns true (the FrameDeco* facet/border queries
        // are presentation).
        let script = r#"
        func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        func NoObj() { return SetMenuDecoration(DECO); }
        func Deco(decoration) { return SetMenuDecoration(decoration, this()); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let mut decoration = Definition::from_script(
            "DECO",
            "Deco",
            r#"
            protected func FrameDecorationBackClr() { return 123456; }
            protected func FrameDecorationBorderTop() { return 1; }
            protected func FrameDecorationBorderLeft() { return 2; }
            protected func FrameDecorationBorderRight() { return 3; }
            protected func FrameDecorationBorderBottom() { return 4; }
            "#,
        )
        .expect("script compiles");
        decoration.configure_action_graphics(HashMap::from([(
            "FrameDecoTop".to_string(),
            DefinitionActionGraphics {
                facet: Some(DefinitionActionFacet {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                    target_x: -2,
                    target_y: -3,
                }),
                ..DefinitionActionGraphics::default()
            },
        )]));
        engine
            .register_definition(decoration)
            .expect("deco registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };

        assert_eq!(
            call(&mut engine, "Deco", vec![Value::C4Id("DECO".into())]),
            Value::Bool(false),
            "no menu -> false (C4Script.cpp:1739)"
        );
        call(&mut engine, "OpenMenu", Vec::new());
        assert_eq!(
            call(&mut engine, "NoObj", Vec::new()),
            Value::Bool(false),
            "nil menu object -> false even with a scope object (C4Script.cpp:1739)"
        );
        assert_eq!(
            call(&mut engine, "Deco", vec![Value::C4Id("GOLD".into())]),
            Value::Bool(false),
            "unknown deco def -> SetByDef fails (C4GuiDialogs.cpp:113-114)"
        );
        assert_eq!(
            engine
                .debug_object_menu(clonk.as_u64())
                .expect("clonk exists")
                .expect("menu is open")
                .decoration,
            None
        );
        assert_eq!(
            call(&mut engine, "Deco", vec![Value::C4Id("DECO".into())]),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .debug_object_menu(clonk.as_u64())
                .expect("clonk exists")
                .expect("menu is open")
                .decoration,
            Some(ObjectMenuFrameDecoration {
                source_definition: "DECO".to_string(),
                background_color: 123456,
                border_top: 1,
                border_left: 2,
                border_right: 3,
                border_bottom: 4,
                top: Some(DefinitionActionFacet {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                    target_x: -2,
                    target_y: -3,
                }),
                top_right: None,
                right: None,
                bottom_right: None,
                bottom: None,
                bottom_left: None,
                left: None,
                top_left: None,
            })
        );
    }

    #[test]
    fn menu_user_enter_executes_the_item_command_and_closes_like_cpp() {
        // C4Menu::Enter (C4Menu.cpp:498-523): no active menu -> false;
        // Style_Info refuses (:502); without a selected item non-dialogs
        // keep the menu and report true (:504-510); otherwise the selected
        // item's command (Command2 on right enter with one set, :514) is
        // copied, a non-permanent menu closes BEFORE the exec (:517), and
        // the string runs as script on the menu's command object
        // (C4ObjectMenu::MenuCommand -> C4Object::MenuCommand DirectExec,
        // C4ObjectMenu.cpp:505-527 / C4Object.cpp:3756-3760).
        let script = r#"
        local hit, text;
        func Mark(a, b) { hit = a + b; return 7; }
        func Keep(value) { text = value; return 1; }
        func OpenWith(cmd, par, style, perm) {
            CreateMenu(WIPF, this(), this(), 0, "Choose", 0, style, perm);
            AddMenuItem("A", cmd, WIPF, this(), 0, par);
            return SelectMenuItem(0, this());
        }
        func OpenEmpty() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine
                .call_object_function(idx, name, args)
                .expect("call succeeds")
        };
        let hit = |engine: &Engine| {
            let idx = engine.find_object_index(clonk).expect("clonk exists");
            engine.objects[idx]
                .state
                .local_vars
                .get("hit")
                .cloned()
                .unwrap_or(Value::Nil)
        };

        // No menu -> false (C4Menu::Enter !IsActive, C4Menu.cpp:501).
        assert!(!engine.menu_user_enter(clonk, false).expect("enter runs"));

        // Left enter: command with %d — the second %d gets 0 for left
        // (C4Script.cpp:1563-1570). The non-permanent menu closes.
        call(
            &mut engine,
            "OpenWith",
            vec![
                Value::String("Mark(%d,%d)".into()),
                Value::Int(40),
                Value::Int(0),
                Value::Int(0),
            ],
        );
        assert!(engine.menu_user_enter(clonk, false).expect("enter runs"));
        assert_eq!(hit(&engine), Value::Int(40), "Mark(40,0) ran");
        assert_eq!(
            engine.debug_object_menu(clonk.as_u64()),
            Some(None),
            "non-permanent menu closed before the exec (C4Menu.cpp:517)"
        );

        // Right enter takes Command2 (:514): the second %d gets 1.
        call(
            &mut engine,
            "OpenWith",
            vec![
                Value::String("Mark(%d,%d)".into()),
                Value::Int(40),
                Value::Int(0),
                Value::Int(0),
            ],
        );
        assert!(engine.menu_user_enter(clonk, true).expect("enter runs"));
        assert_eq!(hit(&engine), Value::Int(41), "Mark(40,1) ran");

        // MenuCommand's C4AulScript::DirectExec consumes the copied raw
        // C4 string bytes. It must not reinterpret an invalid UTF-8 byte in
        // the command as the UTF-8 encoding of our internal projection.
        call(
            &mut engine,
            "OpenWith",
            vec![
                Value::String(clonk_script::c4_string_from_bytes(&[
                    b'K', b'e', b'e', b'p', b'(', b'\"', 0xff, b'\"', b')',
                ]).into()),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
            ],
        );
        assert!(engine.menu_user_enter(clonk, false).expect("enter runs"));
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("text"),
            Some(&Value::String(clonk_script::c4_string_from_bytes(&[0xff]).into()))
        );

        // A PERMANENT menu survives its own execution (C4Menu.cpp:517).
        call(
            &mut engine,
            "OpenWith",
            vec![
                Value::String("Mark(%d,%d)".into()),
                Value::Int(50),
                Value::Int(0),
                Value::Int(1),
            ],
        );
        assert!(engine.menu_user_enter(clonk, false).expect("enter runs"));
        assert_eq!(hit(&engine), Value::Int(50));
        assert!(
            engine
                .debug_object_menu(clonk.as_u64())
                .expect("clonk exists")
                .is_some(),
            "permanent menu stays open"
        );

        // Style_Info menus refuse Enter outright (C4Menu.cpp:502).
        call(
            &mut engine,
            "OpenWith",
            vec![
                Value::String("Mark(%d,%d)".into()),
                Value::Int(60),
                Value::Int(2),
                Value::Int(0),
            ],
        );
        assert!(!engine.menu_user_enter(clonk, false).expect("enter runs"));
        assert_eq!(hit(&engine), Value::Int(50), "info menu ran nothing");

        // No selected item in a non-dialog menu: true, menu stays, nothing
        // runs (C4Menu.cpp:504-510).
        call(&mut engine, "OpenEmpty", Vec::new());
        assert!(engine.menu_user_enter(clonk, false).expect("enter runs"));
        assert_eq!(hit(&engine), Value::Int(50));
        assert!(engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .is_some());
    }

    #[test]
    fn landscape_width_and_height_report_gback_dimensions_like_cpp() {
        // FnLandscapeWidth/FnLandscapeHeight (C4Script.cpp:3077-3085):
        // GBackWdt/GBackHgt — the wild-horse ContactRight turn check reads
        // them (Goldrush WildHorse.c).
        let script = r#"
        func ReadWidth() { return LandscapeWidth(); }
        func ReadHeight() { return LandscapeHeight(); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("HORS", "Horse", script).expect("definition registers");
        engine.set_landscape(Landscape::flat_with_material(23, 41, None));
        let horse = engine
            .spawn_object(SpawnConfig::new("HORS"))
            .expect("horse spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let idx = engine.find_object_index(horse).expect("horse exists");
        assert_eq!(
            engine
                .call_object_function(idx, "ReadWidth", Vec::new())
                .expect("ReadWidth succeeds"),
            Value::Int(23)
        );
        assert_eq!(
            engine
                .call_object_function(idx, "ReadHeight", Vec::new())
                .expect("ReadHeight succeeds"),
            Value::Int(41)
        );
    }

    #[test]
    fn punch_follows_object_com_punch_semantics_like_cpp() {
        // FnPunch (C4Script.cpp:328-332) -> ObjectComPunch
        // (C4ObjectCom.cpp:735-767): zero punch derives from the Fight
        // physicals (clamp(5*attacker/target, 0, 10)); QueryCatchBlow
        // halves punch > 1 and stops the blow (return false, no tumble);
        // energy drops -punch% and ComDir stops either way; punch >= 10
        // tries Tumble (xdir FIXED100(150)*tdir, ydir -2), else GetPunched
        // (xdir FIXED100(250)*tdir, ydir 0), each firing CatchBlow(punch,
        // attacker) on success.
        let attacker_script = r#"
        func Bite(target, strength) { return Punch(target, strength); }
        "#;
        let victim_script = r#"
        local catchBlow;
        local stopBlows;
        func QueryCatchBlow(byObj) { return stopBlows; }
        func CatchBlow(level, byObj) { catchBlow = level; }
        "#;
        let mut attacker_def = Definition::from_script("SNKE", "Snake", attacker_script)
            .expect("attacker compiles");
        attacker_def.set_physical(PhysicalInfo {
            fight: 50_000,
            ..PhysicalInfo::default()
        });
        let mut victim_def =
            Definition::from_script("CLNK", "Clonk", victim_script).expect("victim compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("GetPunched".to_string(), ActionSpec::default());
        actions.insert("Tumble".to_string(), ActionSpec::default());
        actions.insert(
            "Dead".to_string(),
            ActionSpec::default().with_no_other_action(true),
        );
        victim_def.configure_actions(Some("Idle".to_string()), actions);
        victim_def.set_physical(PhysicalInfo {
            fight: 25_000,
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        // A victim without Tumble/GetPunched AND without Fight: the derived
        // punch stays zero -> Punch is a no-op success (C4ObjectCom.cpp:741).
        let pillow_def =
            Definition::from_script("PILW", "Pillow", victim_script).expect("pillow compiles");

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(attacker_def)
            .expect("attacker registers");
        engine
            .register_definition(victim_def)
            .expect("victim registers");
        engine
            .register_definition(pillow_def)
            .expect("pillow registers");
        let snake = engine
            .spawn_object(SpawnConfig::new("SNKE").with_position(Vector2::new(50, 50)))
            .expect("snake spawns");
        let spawn_victim = |engine: &mut Engine| {
            engine
                .spawn_object(
                    SpawnConfig::new("CLNK")
                        .with_position(Vector2::new(52, 50))
                        .with_alive(true)
                        .with_energy(50_000),
                )
                .expect("victim spawns")
        };
        let v_regular = spawn_victim(&mut engine);
        let v_hard = spawn_victim(&mut engine);
        let v_catcher = spawn_victim(&mut engine);
        let v_derived = spawn_victim(&mut engine);
        let dead_velocity = FixedVec2::new(fixed100(37), itofix(-3));
        let spawn_dead_victim = |engine: &mut Engine| {
            engine
                .spawn_object(
                    SpawnConfig::new("CLNK")
                        .with_position(Vector2::new(52, 50))
                        .with_alive(true)
                        .with_energy(50_000)
                        .with_action(ActionState::new("Dead"))
                        .with_command_direction(CommandDirection::Right)
                        .with_fixed_velocity(dead_velocity),
                )
                .expect("dead victim spawns")
        };
        let v_dead_regular = spawn_dead_victim(&mut engine);
        let v_dead_hard = spawn_dead_victim(&mut engine);
        let pillow = engine
            .spawn_object(
                SpawnConfig::new("PILW")
                    .with_position(Vector2::new(52, 50))
                    .with_alive(true)
                    .with_energy(50_000),
            )
            .expect("pillow spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        // tdir = +1 for a right-facing attacker (C4ObjectCom.cpp:745).
        let snake_idx = engine.find_object_index(snake).expect("snake exists");
        engine.objects[snake_idx].state.direction = Direction::Right;
        // A pre-punch ComDir so the COMD_Stop write is observable.
        let idx = engine.find_object_index(v_regular).expect("victim exists");
        engine.objects[idx].state.command_direction = CommandDirection::Right;

        let bite = |engine: &mut Engine, target: ObjectId, strength: Option<i32>| {
            let idx = engine.find_object_index(snake).expect("snake exists");
            let args = vec![
                Value::Object(target.as_u64()),
                strength.map(Value::Int).unwrap_or(Value::Nil),
            ];
            engine
                .call_object_function(idx, "Bite", args)
                .expect("Bite succeeds")
        };

        // Regular punch (8 < 10): GetPunched, xdir FIXED100(250), ydir 0.
        assert_eq!(bite(&mut engine, v_regular, Some(8)), Value::Bool(true));
        let idx = engine.find_object_index(v_regular).expect("victim exists");
        let victim = &engine.objects[idx];
        assert_eq!(victim.state.energy, 42_000, "-8% of C4MaxPhysical");
        assert_eq!(victim.state.action.name, "GetPunched");
        assert_eq!(victim.fixed_velocity.x, fixed100(250));
        assert_eq!(victim.fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(victim.state.command_direction, CommandDirection::Stop);
        assert_eq!(
            victim.state.local_vars.get("catchBlow"),
            Some(&Value::Int(8)),
            "CatchBlow(level, byObj) fired"
        );

        // Hard punch (>= 10): Tumble with the tumble fling.
        assert_eq!(bite(&mut engine, v_hard, Some(12)), Value::Bool(true));
        let idx = engine.find_object_index(v_hard).expect("victim exists");
        let victim = &engine.objects[idx];
        assert_eq!(victim.state.energy, 38_000, "-12%");
        assert_eq!(victim.state.action.name, "Tumble");
        assert_eq!(victim.fixed_velocity.x, fixed100(150));
        assert_eq!(victim.fixed_velocity.y, itofix(-2));

        // A NoOtherAction Dead victim takes damage and stops its ComDir, but
        // ordinary ObjectActionGetPunched/ObjectActionTumble transitions
        // fail without changing action or motion (C4Object.cpp:4111-4115).
        assert_eq!(
            bite(&mut engine, v_dead_regular, Some(8)),
            Value::Bool(false)
        );
        let idx = engine
            .find_object_index(v_dead_regular)
            .expect("dead victim exists");
        let victim = &engine.objects[idx];
        assert_eq!(victim.state.energy, 42_000);
        assert_eq!(victim.state.action.name, "Dead");
        assert_eq!(victim.fixed_velocity, dead_velocity);
        assert_eq!(victim.state.command_direction, CommandDirection::Stop);
        assert!(
            !victim
                .state
                .local_vars
                .get("catchBlow")
                .is_some_and(Value::as_bool),
            "failed GetPunched does not fire CatchBlow"
        );

        assert_eq!(
            bite(&mut engine, v_dead_hard, Some(12)),
            Value::Bool(false)
        );
        let idx = engine
            .find_object_index(v_dead_hard)
            .expect("dead victim exists");
        let victim = &engine.objects[idx];
        assert_eq!(victim.state.energy, 38_000);
        assert_eq!(victim.state.action.name, "Dead");
        assert_eq!(victim.fixed_velocity, dead_velocity);
        assert_eq!(victim.state.command_direction, CommandDirection::Stop);
        assert!(
            !victim
                .state
                .local_vars
                .get("catchBlow")
                .is_some_and(Value::as_bool),
            "failed Tumble and GetPunched do not fire CatchBlow"
        );

        // Caught blow: halved damage, no tumble, Punch returns false.
        let idx = engine.find_object_index(v_catcher).expect("victim exists");
        engine.objects[idx]
            .state
            .local_vars
            .insert("stopBlows".to_string(), Value::Int(1));
        assert_eq!(bite(&mut engine, v_catcher, Some(8)), Value::Bool(false));
        let idx = engine.find_object_index(v_catcher).expect("victim exists");
        let victim = &engine.objects[idx];
        assert_eq!(victim.state.energy, 46_000, "halved to -4%");
        assert_ne!(victim.state.action.name, "GetPunched", "no fling");

        // Zero punch derives from the Fight physicals:
        // clamp(5*50000/25000, 0, 10) = 10 -> hard punch.
        assert_eq!(bite(&mut engine, v_derived, None), Value::Bool(true));
        let idx = engine.find_object_index(v_derived).expect("victim exists");
        assert_eq!(engine.objects[idx].state.action.name, "Tumble");

        // No Fight physical on the target: punch stays 0 -> no-op success.
        assert_eq!(bite(&mut engine, pillow, None), Value::Bool(true));
        let idx = engine.find_object_index(pillow).expect("pillow exists");
        assert_eq!(engine.objects[idx].state.energy, 50_000, "untouched");
    }

    #[test]
    fn set_command_dispatches_to_a_foreign_object_like_cpp() {
        // FnSetCommand (C4Script.cpp:840-868): pObj is the FIRST parameter
        // and may be ANY object — GoldRush's StopClonk helper halts OTHER
        // clonks (SetCommand(pTarget, "None"), Helpers.c:94) and dialogue
        // NPCs get MoveTo orders. An unknown command name clears the
        // target's command stack and returns false (:847-851).
        let script = r#"
        func Order(target) { return SetCommand(target, "MoveTo", 0, 44, 55); }
        func Halt(target) { return SetCommand(target, "None"); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("BOSS", "Boss", script).expect("boss registers");
        engine.register_script_definition("MNON", "Minion", "").expect("minion registers");
        let boss = engine
            .spawn_object(SpawnConfig::new("BOSS"))
            .expect("boss spawns");
        let minion = engine
            .spawn_object(SpawnConfig::new("MNON"))
            .expect("minion spawns");
        engine.tick_without_snapshot().expect("tick succeeds");

        let boss_idx = engine.find_object_index(boss).expect("boss exists");
        assert_eq!(
            engine
                .call_object_function(
                    boss_idx,
                    "Order",
                    vec![Value::Object(minion.as_u64())],
                )
                .expect("Order succeeds"),
            Value::Bool(true)
        );
        let minion_idx = engine.find_object_index(minion).expect("minion exists");
        assert_eq!(
            engine.objects[minion_idx].commands.command_names(),
            vec!["MoveTo".to_string()],
            "the foreign target carries the command"
        );

        let boss_idx = engine.find_object_index(boss).expect("boss exists");
        assert_eq!(
            engine
                .call_object_function(boss_idx, "Halt", vec![Value::Object(minion.as_u64())])
                .expect("Halt succeeds"),
            Value::Bool(false),
            "unknown command name -> ClearCommands + false"
        );
        let minion_idx = engine.find_object_index(minion).expect("minion exists");
        assert!(
            engine.objects[minion_idx].commands.command_names().is_empty(),
            "\"None\" cleared the foreign stack"
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    struct PlayerObjectCommandDiagnostic {
        level: tracing::Level,
        target: String,
        message: String,
        error: Option<String>,
    }

    #[derive(Clone)]
    struct PlayerObjectCommandDiagnosticLayer {
        records: std::sync::Arc<std::sync::Mutex<Vec<PlayerObjectCommandDiagnostic>>>,
    }

    impl<S> tracing_subscriber::Layer<S> for PlayerObjectCommandDiagnosticLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = PlayerObjectCommandMessageVisitor::default();
            event.record(&mut visitor);
            self.records
                .lock()
                .expect("diagnostic records lock")
                .push(PlayerObjectCommandDiagnostic {
                    level: *event.metadata().level(),
                    target: event.metadata().target().to_string(),
                    message: visitor.message.unwrap_or_default(),
                    error: visitor.error,
                });
        }
    }

    #[derive(Default)]
    struct PlayerObjectCommandMessageVisitor {
        message: Option<String>,
        error: Option<String>,
    }

    impl tracing::field::Visit for PlayerObjectCommandMessageVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                let mut text = format!("{value:?}");
                if let Some(stripped) = text
                    .strip_prefix('"')
                    .and_then(|inner| inner.strip_suffix('"'))
                {
                    text = stripped.to_string();
                }
                self.message = Some(text);
            } else if field.name() == "error" {
                self.error = Some(format!("{value:?}"));
            }
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "message" {
                self.message = Some(value.to_string());
            } else if field.name() == "error" {
                self.error = Some(value.to_string());
            }
        }
    }

    fn player_object_command_fixture() -> (Engine, ObjectId, ObjectId, ObjectId) {
        let caller_script = r#"#strict 3
        func Seed(target) { return SetCommand(target, "Wait"); }
        func PutInto(target) { return PlayerObjectCommand(1, "Put", target, 0, 0); }
        func BadPlayer(target) { return PlayerObjectCommand(77, "Put", target, 0, 0); }
        func BadName(target) { return PlayerObjectCommand(1, "NoSuch", target, 0, 0); }
        func BadCall() { return PlayerObjectCommand(1, "Call"); }
        func IntData() { return PlayerObjectCommand(1, "Wait", nil, 0, 0, nil, 4711); }
        func IdData() { return PlayerObjectCommand(1, "Wait", nil, 0, 0, nil, ITEM); }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_script_definition("CALL", "Caller", caller_script)
            .expect("caller registers");
        let mut crew_definition =
            Definition::from_script("CREW", "Crew", "").expect("crew compiles");
        crew_definition.set_ocf_base(ocf::CONTAINER);
        engine
            .register_definition(crew_definition)
            .expect("crew registers");
        engine.register_script_definition("ITEM", "Item", "").expect("item registers");
        let mut container_definition =
            Definition::from_script("CONT", "Container", "").expect("container compiles");
        container_definition.set_ocf_base(ocf::CONTAINER);
        engine
            .register_definition(container_definition)
            .expect("container registers");
        engine
            .register_player(PlayerConfig::new(1, "Player"))
            .expect("player registers");

        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let crew = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("crew spawns");
        engine.select_crew(1, [crew]).expect("crew selected");
        engine
            .set_crew_cursor(1, Some(crew))
            .expect("crew cursor set");
        engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(crew))
            .expect("crew inventory spawns");
        let container = engine
            .spawn_object(SpawnConfig::new("CONT"))
            .expect("container spawns");
        engine.tick_without_snapshot().expect("fixture initializes");

        (engine, caller, crew, container)
    }

    fn call_player_object_command_fixture(
        engine: &mut Engine,
        caller: ObjectId,
        function: &str,
        args: Vec<Value>,
    ) -> Result<Value, EngineError> {
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        engine.call_object_function(caller_index, function, args)
    }

    #[test]
    fn player_object_command_call_warns_below_strict3_and_errors_at_strict3() {
        // StrictError reads cthr->Caller->Func->Owner->Strict. Below STRICT3
        // it diagnoses Call but still performs C4P_Command_Set with Data=0;
        // STRICT3 throws before C4Player::ObjectCommand (C4Script.cpp:62-75,
        // 961-985).
        let (mut engine, strict3_caller, crew, container) = player_object_command_fixture();
        let mut legacy_callers = Vec::new();
        for (definition, strict_directive) in
            [("NSCL", ""), ("S1CL", "#strict"), ("S2CL", "#strict 2")]
        {
            let script = format!(
                r#"{strict_directive}
                func Issue(target, target2) {{
                    return PlayerObjectCommand(1, "Call", target, 17, 19, target2, 4711);
                }}
                "#
            );
            engine
                .register_script_definition(definition, definition, &script)
                .expect("legacy caller registers");
            let caller = engine
                .spawn_object(SpawnConfig::new(definition))
                .expect("legacy caller spawns");
            legacy_callers.push((strict_directive, caller));
        }
        engine
            .tick_without_snapshot()
            .expect("legacy callers initialize");

        for (strict_directive, caller) in legacy_callers {
            assert_eq!(
                call_player_object_command_fixture(
                    &mut engine,
                    strict3_caller,
                    "Seed",
                    vec![Value::Object(crew.as_u64())],
                )
                .expect("old Wait command seeds"),
                Value::Bool(true)
            );

            let records = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let subscriber = tracing_subscriber::layer::SubscriberExt::with(
                tracing_subscriber::Registry::default(),
                PlayerObjectCommandDiagnosticLayer {
                    records: std::sync::Arc::clone(&records),
                },
            );
            let result = tracing::subscriber::with_default(subscriber, || {
                call_player_object_command_fixture(
                    &mut engine,
                    caller,
                    "Issue",
                    vec![
                        Value::Object(container.as_u64()),
                        Value::Object(strict3_caller.as_u64()),
                    ],
                )
            });

            assert_eq!(
                result.expect("legacy Call continues after its diagnostic"),
                Value::Bool(true),
                "caller {strict_directive:?}"
            );
            assert_eq!(
                records.lock().expect("diagnostic records lock").as_slice(),
                &[PlayerObjectCommandDiagnostic {
                    level: tracing::Level::WARN,
                    target: "clonk-script".to_string(),
                    message: "PlayerObjectCommand: Command \"Call\" not supported".to_string(),
                    error: None,
                }],
                "caller {strict_directive:?}"
            );

            let views = engine
                .object_snapshot(crew)
                .expect("crew exists")
                .command_stack
                .command_views();
            assert_eq!(views.len(), 1, "Call replaces the seeded Wait command");
            assert_eq!(views[0].name, "Call");
            assert_eq!(views[0].target, Some(container));
            assert_eq!(views[0].tx, Some(17));
            assert_eq!(views[0].ty, Some(19));
            assert_eq!(views[0].target2, Some(strict3_caller));
            assert_eq!(
                views[0].data,
                CommandData::Integer(0),
                "Call ignores the supplied 4711 data value"
            );
        }

        call_player_object_command_fixture(
            &mut engine,
            strict3_caller,
            "Seed",
            vec![Value::Object(crew.as_u64())],
        )
        .expect("strict-3 baseline Wait command seeds");
        let before = engine
            .object_snapshot(crew)
            .expect("crew exists")
            .command_stack;
        let records = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::layer::SubscriberExt::with(
            tracing_subscriber::Registry::default(),
            PlayerObjectCommandDiagnosticLayer {
                records: std::sync::Arc::clone(&records),
            },
        );
        let error = tracing::subscriber::with_default(subscriber, || {
            call_player_object_command_fixture(
                &mut engine,
                strict3_caller,
                "BadCall",
                Vec::new(),
            )
        })
        .expect_err("strict-3 Call errors");
        match error {
            EngineError::Script { source, .. } => assert!(
                source
                    .to_string()
                    .contains("PlayerObjectCommand: Command \"Call\" not supported"),
                "unexpected strict error: {source}"
            ),
            other => panic!("expected script error, got {other:?}"),
        }
        assert!(
            records
                .lock()
                .expect("diagnostic records lock")
                .is_empty(),
            "strict-3 throws instead of warning"
        );
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew exists")
                .command_stack,
            before,
            "strict-3 rejection precedes every command mutation"
        );
    }

    #[test]
    fn player_object_command_sets_put_and_converts_int_or_id_data() {
        // FnPlayerObjectCommand delegates to C4Player::ObjectCommand with
        // C4P_Command_Set, so the selected crew's old stack is replaced.
        // Its Data slot uses C4Value::getIntOrID for every non-Call command
        // (C4Script.cpp:961-985; C4Player.cpp:1397-1451).
        let (mut engine, caller, crew, container) = player_object_command_fixture();
        let crew_ref = Value::Object(crew.as_u64());
        let container_ref = Value::Object(container.as_u64());

        assert_eq!(
            call_player_object_command_fixture(&mut engine, caller, "Seed", vec![crew_ref])
                .expect("old command seeds"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew exists")
                .command_stack
                .command_names(),
            vec!["Wait".to_string()]
        );

        assert_eq!(
            call_player_object_command_fixture(
                &mut engine,
                caller,
                "PutInto",
                vec![container_ref],
            )
            .expect("Put command succeeds"),
            Value::Bool(true)
        );
        let views = engine
            .object_snapshot(crew)
            .expect("crew exists")
            .command_stack
            .command_views();
        assert_eq!(views.len(), 1, "Set mode replaces the old Wait stack");
        assert_eq!(views[0].name, "Put");
        assert_eq!(views[0].target, Some(container));
        assert_eq!(views[0].target2, None);
        assert_eq!(views[0].data, CommandData::Integer(0));

        assert_eq!(
            call_player_object_command_fixture(&mut engine, caller, "IntData", Vec::new())
                .expect("integer Data succeeds"),
            Value::Bool(true)
        );
        let views = engine
            .object_snapshot(crew)
            .expect("crew exists")
            .command_stack
            .command_views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].data, CommandData::Integer(4711));

        assert_eq!(
            call_player_object_command_fixture(&mut engine, caller, "IdData", Vec::new())
                .expect("C4ID Data succeeds"),
            Value::Bool(true)
        );
        let views = engine
            .object_snapshot(crew)
            .expect("crew exists")
            .command_stack
            .command_views();
        assert_eq!(views.len(), 1);
        assert_eq!(
            views[0].data,
            CommandData::Integer(i32::from_le_bytes(*b"ITEM"))
        );
    }

    #[test]
    fn player_object_command_rejections_leave_selected_crew_unchanged() {
        // Player and command validation precede C4Player::ObjectCommand.
        // C4CMD_Call reaches StrictError before the Set path and is fatal to
        // a #strict 3 caller (C4Script.cpp:961-985).
        let (mut engine, caller, crew, container) = player_object_command_fixture();
        call_player_object_command_fixture(
            &mut engine,
            caller,
            "Seed",
            vec![Value::Object(crew.as_u64())],
        )
        .expect("old command seeds");
        let before = engine
            .object_snapshot(crew)
            .expect("crew exists")
            .command_stack;

        for function in ["BadPlayer", "BadName"] {
            assert_eq!(
                call_player_object_command_fixture(
                    &mut engine,
                    caller,
                    function,
                    vec![Value::Object(container.as_u64())],
                )
                .expect("invalid request returns normally"),
                Value::Bool(false)
            );
            assert_eq!(
                engine
                    .object_snapshot(crew)
                    .expect("crew exists")
                    .command_stack,
                before,
                "{function} must not alter the selected crew's stack"
            );
        }

        let error = call_player_object_command_fixture(&mut engine, caller, "BadCall", Vec::new())
            .expect_err("Call is a strict-3 error");
        match error {
            EngineError::Script { source, .. } => assert!(
                source
                    .to_string()
                    .contains("PlayerObjectCommand: Command \"Call\" not supported"),
                "unexpected strict error: {source}"
            ),
            other => panic!("expected script error, got {other:?}"),
        }
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew exists")
                .command_stack,
            before,
            "strict-3 Call rejection must happen before the Set path"
        );
    }

    fn reject_grabbed_test_engine() -> Engine {
        let actor_script = r#"#strict
local order, seen_action, seen_target, finished;
local finished_front, finished_target;
local after_execute, after_action;
local remove_on_jump;
local jump_xdir, jump_ydir, jump_by_com;

public func ResetGrabProbe()
{
  var no_value;
  order = 0;
  seen_action = no_value;
  seen_target = no_value;
  finished = no_value;
  finished_front = no_value;
  finished_target = no_value;
  after_execute = no_value;
  after_action = no_value;
  remove_on_jump = false;
  jump_xdir = no_value;
  jump_ydir = no_value;
  jump_by_com = no_value;
  return true;
}

public func RemoveOnJump()
{
  remove_on_jump = true;
  return true;
}

public func NoteReject(target)
{
  order = order * 10 + 1;
  seen_action = GetAction();
  seen_target = target;
  return true;
}

public func RunGrab(target)
{
  ResetGrabProbe();
  SetCommand(this(), "Grab", target);
  ExecuteCommand();
  after_execute = order;
  after_action = GetAction();
  return order;
}

public func QueueNullGrab() { return AddCommand(this(), "Grab"); }
public func RunOneCommand() { return ExecuteCommand(); }

protected func PushStart()
{
  order = order * 10 + 2;
  return true;
}

protected func JumpStart()
{
  order = order * 10 + 3;
  return true;
}

protected func OnActionJump(xdir, ydir, by_com)
{
  jump_xdir = xdir;
  jump_ydir = ydir;
  jump_by_com = by_com;
  if (!remove_on_jump) return false;
  RemoveObject(this());
  return true;
}

protected func ControlCommandFinished(command, target)
{
  finished = command;
  finished_front = GetCommand(0);
  finished_target = target;
  return true;
}
"#;
        let mut actor =
            Definition::from_script("RGAC", "Grab actor", actor_script).expect("actor compiles");
        actor.set_c4_callback_convention(true);
        actor.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        actor.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Push".to_string(),
                    ActionSpec::default()
                        .with_procedure("PUSH")
                        .with_start_call("PushStart"),
                ),
                (
                    "Scale".to_string(),
                    ActionSpec::default().with_procedure("SCALE"),
                ),
                (
                    "Hangle".to_string(),
                    ActionSpec::default().with_procedure("HANGLE"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_start_call("JumpStart"),
                ),
            ]),
        );

        let veto_script = r#"#strict
protected func RejectGrabbed(clonk)
{
  clonk->NoteReject(this());
  return true;
}
"#;
        let pass_script = r#"#strict
protected func RejectGrabbed(clonk)
{
  clonk->NoteReject(this());
  return false;
}
"#;
        let add_command_script = r#"#strict
protected func RejectGrabbed(clonk)
{
  clonk->NoteReject(this());
  AddCommand(clonk, "Wait", 0, 0, 0, 0, 1);
  return true;
}
"#;
        let zero_id_script = r#"#strict
protected func RejectGrabbed(clonk)
{
  clonk->NoteReject(this());
  return NONE;
}
"#;
        let clear_target_script = r#"#strict
protected func RejectGrabbed(clonk)
{
  clonk->NoteReject(this());
  SetObjectStatus(2, this(), true);
  return false;
}
"#;
        let clear_then_replace_script = r#"#strict
protected func RejectGrabbed(clonk)
{
  clonk->NoteReject(this());
  SetObjectStatus(2, this(), true);
  SetCommand(clonk, "Wait", 0, 1);
  return false;
}
"#;
        let replace_then_clear_script = r#"#strict
protected func RejectGrabbed(clonk)
{
  clonk->NoteReject(this());
  SetCommand(clonk, "Wait", 0, 1);
  SetObjectStatus(2, this(), true);
  return false;
}
"#;
        let removed_actor_script = r#"#strict
local reject_calls;
protected func RejectGrabbed(clonk)
{
  reject_calls = 1;
  return true;
}
"#;

        let mut engine = Engine::with_seed(15);
        engine
            .register_definition(actor)
            .expect("actor definition registers");
        for (id, name, script) in [
            ("RGVT", "Veto target", veto_script),
            ("RGPS", "Pass target", pass_script),
            ("RGPL", "Plain target", "#strict\n"),
            ("RGAD", "Command-adding target", add_command_script),
            ("RGNO", "Zero-ID target", zero_id_script),
            ("RGCP", "Pointer-clearing target", clear_target_script),
            (
                "RGCR",
                "Clear-then-replace target",
                clear_then_replace_script,
            ),
            (
                "RGRC",
                "Replace-then-clear target",
                replace_then_clear_script,
            ),
            ("RGRM", "Removed-actor target", removed_actor_script),
        ] {
            let mut target = Definition::from_script(id, name, script).expect("target compiles");
            target.set_c4_callback_convention(true);
            target.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
            target.set_grab(1);
            engine
                .register_definition(target)
                .expect("target definition registers");
        }
        engine
    }

    fn spawn_grab_probe(
        engine: &mut Engine,
        target_definition: &str,
        action: &str,
        x: i32,
    ) -> (ObjectId, ObjectId) {
        let position = Vector2::new(x, 100);
        let actor = engine
            .spawn_object(
                SpawnConfig::new("RGAC")
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_position(position)
                    .with_action(ActionState::new(action))
                    .with_command_direction(CommandDirection::Right)
                    .with_alive(true),
            )
            .expect("actor spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new(target_definition)
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(position),
            )
            .expect("target spawns");
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_index, "ResetGrabProbe", Vec::new())
            .expect("probe resets");
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        (actor, target)
    }

    #[test]
    fn script_add_command_null_grab_ungrabs_then_reports_finished() {
        let mut engine = reject_grabbed_test_engine();
        let (actor, pushed_target) = spawn_grab_probe(&mut engine, "RGPL", "Push", 0);
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_index].commands.clear();
        engine.objects[actor_index].state.action.target = Some(pushed_target);
        engine.objects[actor_index]
            .commands
            .push_back(
                CommandRequest::new(CommandId::Wait)
                    .with_retries(1)
                    .with_mode(CommandMode::Base),
            )
            .expect("base Wait queues");

        assert_eq!(
            engine
                .call_object_function(actor_index, "QueueNullGrab", Vec::new())
                .expect("script AddCommand returns"),
            Value::Bool(true),
            "C4Object::AddCommand validates the command id, not Target"
        );
        assert_eq!(
            engine
                .object_snapshot(actor)
                .expect("actor remains")
                .command_stack
                .command_names(),
            vec!["Grab".to_string(), "Wait".to_string()]
        );

        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_index, "RunOneCommand", Vec::new())
            .expect("targetless Grab executes");
        assert_eq!(
            engine
                .object_snapshot(actor)
                .expect("actor remains")
                .command_stack
                .command_names(),
            vec!["UnGrab".to_string(), "Grab".to_string(), "Wait".to_string()],
            "the pushing actor queues UnGrab before checking the null target"
        );

        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_index, "RunOneCommand", Vec::new())
            .expect("UnGrab executes");
        let after_ungrab = engine.object_snapshot(actor).expect("actor remains");
        assert_eq!(after_ungrab.action.name, "Walk");
        assert_eq!(
            after_ungrab.command_stack.command_names(),
            vec!["Grab".to_string(), "Wait".to_string()]
        );
        assert_eq!(
            after_ungrab.local_vars.get("finished"),
            Some(&Value::String("UnGrab".to_string().into()))
        );
        assert_eq!(
            after_ungrab.local_vars.get("finished_front"),
            Some(&Value::String("UnGrab".to_string().into())),
            "ControlCommandFinished observes the finished UnGrab front"
        );

        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_index, "RunOneCommand", Vec::new())
            .expect("uncovered targetless Grab fails");
        let after_grab = engine.object_snapshot(actor).expect("actor remains");
        assert_eq!(
            after_grab.command_stack.command_names(),
            vec!["Wait".to_string()]
        );
        assert_eq!(
            after_grab.local_vars.get("finished"),
            Some(&Value::String("Grab".to_string().into()))
        );
        assert_eq!(
            after_grab.local_vars.get("finished_front"),
            Some(&Value::String("Grab".to_string().into())),
            "the failed Grab remains visible during ControlCommandFinished"
        );
        assert_eq!(
            after_grab.local_vars.get("finished_target"),
            Some(&Value::Nil)
        );
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        let failed_base = serde_json::to_value(engine.objects[actor_index].commands.snapshot())
            .expect("command stack serializes");
        assert_eq!(failed_base["commands"][0]["failures"], serde_json::json!(1));
        assert_eq!(failed_base["commands"][0]["retries"], serde_json::json!(1));

        engine
            .call_object_function(actor_index, "RunOneCommand", Vec::new())
            .expect("base consumes the delegated failure");
        let during_retry = engine.object_snapshot(actor).expect("actor remains");
        assert_eq!(
            during_retry.command_stack.command_names(),
            vec!["Retry".to_string(), "Wait".to_string()]
        );
    }

    #[test]
    fn grab_calls_reject_grabbed_before_push_and_honors_veto() {
        let mut engine = reject_grabbed_test_engine();
        let (veto_actor, veto_target) = spawn_grab_probe(&mut engine, "RGVT", "Walk", 0);
        let (pass_actor, pass_target) = spawn_grab_probe(&mut engine, "RGPS", "Walk", 100);
        let (plain_actor, plain_target) = spawn_grab_probe(&mut engine, "RGPL", "Walk", 200);
        let (scale_actor, _) = spawn_grab_probe(&mut engine, "RGVT", "Scale", 300);
        let (hangle_actor, _) = spawn_grab_probe(&mut engine, "RGVT", "Hangle", 400);
        let scale_actor_index = engine
            .find_object_index(scale_actor)
            .expect("scaler exists");
        engine.objects[scale_actor_index].state.direction = Direction::Left;
        let hangle_actor_index = engine
            .find_object_index(hangle_actor)
            .expect("hangler exists");
        engine.objects[hangle_actor_index].state.direction = Direction::Right;
        let (mutating_actor, _) = spawn_grab_probe(&mut engine, "RGAD", "Walk", 500);
        let (zero_id_actor, zero_id_target) =
            spawn_grab_probe(&mut engine, "RGNO", "Walk", 600);
        let (clear_target_actor, _) = spawn_grab_probe(&mut engine, "RGCP", "Walk", 700);
        let (clear_then_replace_actor, _) =
            spawn_grab_probe(&mut engine, "RGCR", "Walk", 800);
        let (replace_then_clear_actor, replace_then_clear_target) =
            spawn_grab_probe(&mut engine, "RGRC", "Walk", 900);
        let (removed_actor, removed_actor_target) =
            spawn_grab_probe(&mut engine, "RGRM", "Scale", 1000);
        let removed_actor_index = engine
            .find_object_index(removed_actor)
            .expect("removal actor exists");
        engine
            .call_object_function(removed_actor_index, "RemoveOnJump", Vec::new())
            .expect("jump removal arms");

        engine.tick_without_snapshot().expect("Grab commands execute");

        let veto = engine.object_snapshot(veto_actor).expect("veto actor remains");
        assert_eq!(veto.action.name, "Walk");
        assert_eq!(veto.command_direction, CommandDirection::Right);
        assert_eq!(veto.local_vars.get("order"), Some(&Value::Int(1)));
        assert_eq!(
            veto.local_vars.get("seen_action"),
            Some(&Value::String("Walk".to_string().into()))
        );
        assert_eq!(
            veto.local_vars.get("seen_target"),
            Some(&object_reference_value(veto_target))
        );
        assert_eq!(
            veto.local_vars.get("finished"),
            Some(&Value::String("Grab".to_string().into()))
        );
        assert!(veto.command_stack.is_empty(), "vetoed Grab finishes now");

        let pass = engine.object_snapshot(pass_actor).expect("pass actor remains");
        assert_eq!(pass.action.name, "Push");
        assert_eq!(pass.action.target, Some(pass_target));
        assert_eq!(pass.command_direction, CommandDirection::Stop);
        assert_eq!(pass.local_vars.get("order"), Some(&Value::Int(12)));
        assert_eq!(
            pass.command_stack.command_names(),
            vec!["Grab".to_string()]
        );

        let plain = engine
            .object_snapshot(plain_actor)
            .expect("plain actor remains");
        assert_eq!(plain.action.name, "Push");
        assert_eq!(plain.action.target, Some(plain_target));
        assert_eq!(plain.local_vars.get("order"), Some(&Value::Int(2)));

        let zero_id = engine
            .object_snapshot(zero_id_actor)
            .expect("zero-ID actor remains");
        assert_eq!(zero_id.action.name, "Push");
        assert_eq!(zero_id.action.target, Some(zero_id_target));
        assert_eq!(zero_id.local_vars.get("order"), Some(&Value::Int(12)));

        let clear_target = engine
            .object_snapshot(clear_target_actor)
            .expect("pointer-cleared actor remains");
        assert_eq!(clear_target.action.name, "Walk");
        assert_eq!(clear_target.command_direction, CommandDirection::Stop);
        assert_eq!(clear_target.local_vars.get("order"), Some(&Value::Int(1)));

        let clear_then_replace = engine
            .object_snapshot(clear_then_replace_actor)
            .expect("clear-then-replace actor remains");
        assert_eq!(clear_then_replace.action.name, "Walk");
        assert_eq!(
            clear_then_replace.command_stack.command_names(),
            vec!["Wait".to_string()]
        );

        let replace_then_clear = engine
            .object_snapshot(replace_then_clear_actor)
            .expect("replace-then-clear actor remains");
        assert_eq!(replace_then_clear.action.name, "Push");
        assert_eq!(
            replace_then_clear.action.target,
            Some(replace_then_clear_target)
        );
        assert_eq!(
            replace_then_clear.command_stack.command_names(),
            vec!["Wait".to_string()]
        );

        let removed_target = engine
            .object_snapshot(removed_actor_target)
            .expect("removed actor's target remains");
        assert_eq!(
            removed_target.local_vars.get("reject_calls"),
            Some(&Value::Int(1)),
            "RejectGrabbed still runs after scale let-go removes the actor"
        );

        for (actor, expected_xdir) in [(scale_actor, 100), (hangle_actor, -100)] {
            let actor = engine.object_snapshot(actor).expect("climber remains");
            assert_eq!(actor.local_vars.get("order"), Some(&Value::Int(31)));
            assert_eq!(
                actor.local_vars.get("seen_action"),
                Some(&Value::String("Jump".to_string().into())),
                "let-go and its Jump StartCall precede RejectGrabbed"
            );
            assert_eq!(
                actor.local_vars.get("jump_xdir"),
                Some(&Value::Int(expected_xdir)),
                "ObjectComLetGo jumps opposite the climber's facing"
            );
            assert_eq!(actor.local_vars.get("jump_ydir"), Some(&Value::Nil));
            assert_eq!(
                actor.local_vars.get("jump_by_com"),
                Some(&Value::Bool(true))
            );
            assert_ne!(actor.action.name, "Push", "the veto prevents grabbing");
        }

        let mutating = engine
            .object_snapshot(mutating_actor)
            .expect("command-mutating actor remains");
        assert_eq!(mutating.action.name, "Walk");
        assert_eq!(
            mutating.command_stack.command_names(),
            vec!["Wait".to_string(), "Grab".to_string()]
        );
        engine.tick_without_snapshot().expect("callback-added Wait completes");
        let mutating = engine
            .object_snapshot(mutating_actor)
            .expect("command-mutating actor remains");
        assert!(
            mutating.command_stack.is_empty(),
            "the finished original Grab must not resume below the callback-added command"
        );
        assert_eq!(mutating.action.name, "Walk");
        assert!(
            engine
                .object_snapshot(clear_target_actor)
                .expect("pointer-cleared actor remains")
                .command_stack
                .is_empty(),
            "the next Grab execute observes its cleared target and fails"
        );
    }

    #[test]
    fn execute_command_runs_reject_grabbed_before_returning_to_script() {
        let mut engine = reject_grabbed_test_engine();
        let (veto_actor, veto_target) = spawn_grab_probe(&mut engine, "RGVT", "Walk", 0);
        let (pass_actor, pass_target) = spawn_grab_probe(&mut engine, "RGPS", "Walk", 100);
        let (scale_actor, scale_target) = spawn_grab_probe(&mut engine, "RGVT", "Scale", 200);
        let (mutating_actor, mutating_target) =
            spawn_grab_probe(&mut engine, "RGAD", "Walk", 300);
        let (zero_id_actor, zero_id_target) =
            spawn_grab_probe(&mut engine, "RGNO", "Walk", 400);
        let (clear_target_actor, clear_target) =
            spawn_grab_probe(&mut engine, "RGCP", "Walk", 500);
        let (far_scale_actor, far_scale_target) =
            spawn_grab_probe(&mut engine, "RGVT", "Scale", 600);
        let far_scale_target_index = engine
            .find_object_index(far_scale_target)
            .expect("far scale target exists");
        engine.objects[far_scale_target_index].set_position(Vector2::new(660, 100));

        for (actor, target, expected_order, expected_action, expected_commands) in [
            (veto_actor, veto_target, 1, "Walk", Vec::new()),
            (
                pass_actor,
                pass_target,
                12,
                "Push",
                vec!["Grab".to_string()],
            ),
            (scale_actor, scale_target, 31, "Jump", Vec::new()),
            (
                mutating_actor,
                mutating_target,
                1,
                "Walk",
                vec!["Wait".to_string(), "Grab".to_string()],
            ),
            (
                zero_id_actor,
                zero_id_target,
                12,
                "Push",
                vec!["Grab".to_string()],
            ),
            (
                clear_target_actor,
                clear_target,
                1,
                "Walk",
                vec!["Grab".to_string()],
            ),
        ] {
            let actor_index = engine.find_object_index(actor).expect("actor exists");
            assert_eq!(
                engine
                    .call_object_function(
                        actor_index,
                        "RunGrab",
                        vec![object_reference_value(target)],
                    )
                    .expect("RunGrab executes"),
                Value::Int(expected_order)
            );
            let actor = engine.object_snapshot(actor).expect("actor remains");
            assert_eq!(
                actor.local_vars.get("after_execute"),
                Some(&Value::Int(expected_order))
            );
            assert_eq!(
                actor.local_vars.get("after_action"),
                Some(&Value::String(expected_action.to_string().into()))
            );
            assert_eq!(actor.command_stack.command_names(), expected_commands);
        }

        let far_scale_actor_index = engine
            .find_object_index(far_scale_actor)
            .expect("far scaler exists");
        assert_eq!(
            engine
                .call_object_function(
                    far_scale_actor_index,
                    "RunGrab",
                    vec![object_reference_value(far_scale_target)],
                )
                .expect("far Scale RunGrab executes"),
            Value::Nil
        );
        let far_scale = engine
            .object_snapshot(far_scale_actor)
            .expect("far scaler remains");
        assert_eq!(far_scale.action.name, "Scale");
        assert_eq!(far_scale.local_vars.get("jump_xdir"), Some(&Value::Nil));
        assert_eq!(
            far_scale.command_stack.command_names(),
            vec!["MoveTo".to_string(), "Grab".to_string()],
            "Scale only lets go inside Grab's at-target branch"
        );

        engine.tick_without_snapshot().expect("callback-added Wait completes");
        assert!(
            engine
                .object_snapshot(mutating_actor)
                .expect("command-mutating actor remains")
                .command_stack
                .is_empty(),
            "ExecuteCommand must finish the original Grab below the callback-added command"
        );
        assert!(
            engine
                .object_snapshot(clear_target_actor)
                .expect("pointer-cleared actor remains")
                .command_stack
                .is_empty(),
            "ExecuteCommand must preserve the cleared target for the next failure"
        );
    }

    fn object_com_grab_test_engine() -> Engine {
        let actor_script = r#"#strict
local order, grab_target, grab_flag, target_controller_in_grab;
local grabbed_target, grabbed_flag, grabbed_controller, deletion_mode;
local stop_clear_target, finished;
local push_start_comdir, push_start_action, push_start_target;
local stop_to_dig, stop_walk_starts, stop_pointer_mode;
local reject_action;
local stop_order, stop_abort_action, stop_walk_start_action;

public func ResetGrabCallbacks()
{
  var no_value;
  order = 0;
  grab_target = no_value;
  grab_flag = no_value;
  target_controller_in_grab = no_value;
  grabbed_target = no_value;
  grabbed_flag = no_value;
  grabbed_controller = no_value;
  deletion_mode = 0;
  stop_clear_target = no_value;
  finished = no_value;
  push_start_comdir = no_value;
  push_start_action = no_value;
  push_start_target = no_value;
  stop_to_dig = no_value;
  stop_walk_starts = no_value;
  stop_pointer_mode = no_value;
  reject_action = no_value;
  stop_order = no_value;
  stop_abort_action = no_value;
  stop_walk_start_action = no_value;
  return true;
}

public func ArmGrabDeletion(mode)
{
  deletion_mode = mode;
  return true;
}

public func ArmStopTargetClear(target)
{
  stop_clear_target = target;
  return true;
}

public func ArmStopToDig()
{
  stop_to_dig = true;
  stop_walk_starts = 0;
  return true;
}

public func ArmStopPointerOrder(target, mode)
{
  stop_clear_target = target;
  stop_pointer_mode = mode;
  return true;
}

public func DeactivateTarget(target)
{
  return SetObjectStatus(2, target, false);
}

public func ReleaseAfterGrabbed()
{
  return deletion_mode == 4;
}

public func DetachBeforeRejectRemoval()
{
  return deletion_mode == 5;
}

public func NoteGrabReject(target)
{
  order = order * 10 + 1;
  reject_action = GetAction();
  return true;
}

protected func PushStart()
{
  order = order * 10 + 2;
  push_start_comdir = GetComDir();
  push_start_action = GetAction();
  push_start_target = GetActionTarget();
  return true;
}

protected func StopWalkStart()
{
  stop_order = stop_order * 10 + 2;
  stop_walk_start_action = GetAction();
  if (!stop_to_dig) return true;
  stop_walk_starts++;
  if (stop_walk_starts == 1) SetAction("Dig");
  return true;
}

protected func StopAbort()
{
  var no_target;
  stop_order = stop_order * 10 + 1;
  stop_abort_action = GetAction();
  if (stop_clear_target)
  {
    if (stop_pointer_mode == 1)
    {
      SetObjectStatus(2, stop_clear_target, true);
      SetCommand(this(), "Wait", 0, 1);
    }
    else if (stop_pointer_mode == 2)
    {
      SetCommand(this(), "Wait", 0, 1);
      RemoveObject(stop_clear_target);
    }
    else
    {
      SetObjectStatus(2, stop_clear_target, true);
    }
    stop_clear_target = no_target;
  }
  return true;
}

protected func Grab(target, grab)
{
  if (!grab) return true;
  order = order * 10 + 3;
  grab_target = target;
  grab_flag = grab;
  target_controller_in_grab = GetController(target);
  SetController(7);
  if (deletion_mode == 1) RemoveObject(this());
  if (deletion_mode == 2) RemoveObject(target);
  if (deletion_mode == 3) SetCommand(this(), "Wait", 0, 1);
  return false;
}

public func NoteGrabbed(target, grab, controller)
{
  order = order * 10 + 4;
  grabbed_target = target;
  grabbed_flag = grab;
  grabbed_controller = controller;
  return true;
}

public func RunGrabNow(target, mode)
{
  ResetGrabCallbacks();
  deletion_mode = mode;
  SetCommand(this(), "Grab", target);
  ExecuteCommand();
  return order;
}


public func RunStopClearGrabNow(target)
{
  ResetGrabCallbacks();
  stop_clear_target = target;
  SetCommand(this(), "Grab", target);
  ExecuteCommand();
  return finished;
}

public func RunStopPointerGrabNow(target, mode)
{
  ResetGrabCallbacks();
  ArmStopPointerOrder(target, mode);
  SetCommand(this(), "Grab", target);
  ExecuteCommand();
  return true;
}

public func RunStopToDigGrabNow(target)
{
  ResetGrabCallbacks();
  ArmStopToDig();
  SetCommand(this(), "Grab", target);
  ExecuteCommand();
  return order;
}

protected func ControlCommandFinished(command)
{
  finished = command;
  return true;
}
"#;
        let mut actor =
            Definition::from_script("OGAC", "ObjectComGrab actor", actor_script)
                .expect("actor compiles");
        actor.set_c4_callback_convention(true);
        actor.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        actor.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_start_call("StopWalkStart"),
                ),
                (
                    "LockedWalk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_no_other_action(true),
                ),
                (
                    "LockedBuild".to_string(),
                    ActionSpec::default()
                        .with_procedure("BUILD")
                        .with_no_other_action(true),
                ),
                (
                    "Push".to_string(),
                    ActionSpec::default()
                        .with_procedure("PUSH")
                        .with_start_call("PushStart"),
                ),
                (
                    "Build".to_string(),
                    ActionSpec::default()
                        .with_procedure("BUILD")
                        .with_abort_call("StopAbort"),
                ),
                (
                    "Chop".to_string(),
                    ActionSpec::default().with_procedure("CHOP"),
                ),
                (
                    "Dig".to_string(),
                    ActionSpec::default().with_procedure("DIG"),
                ),
                (
                    "Flight".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                (
                    "Swim".to_string(),
                    ActionSpec::default().with_procedure("SWIM"),
                ),
            ]),
        );

        let target_script = r#"#strict
local reject_calls, grabbed_calls, grabbed_controller;

protected func RejectGrabbed(clonk)
{
  reject_calls++;
  clonk->NoteGrabReject(this());
  if (clonk->DetachBeforeRejectRemoval())
  {
    SetCommand(clonk, "Wait", 0, 1);
    RemoveObject(this());
  }
  return false;
}

protected func Grabbed(clonk, grab)
{
  grabbed_calls++;
  grabbed_controller = GetController();
  clonk->NoteGrabbed(this(), grab, grabbed_controller);
  if (clonk->ReleaseAfterGrabbed()) clonk->SetAction("Walk");
}
"#;
        let mut target =
            Definition::from_script("OGTG", "ObjectComGrab target", target_script)
                .expect("target compiles");
        target.set_c4_callback_convention(true);
        target.set_shape_rect(Some(DefinitionRect::new(-10, -12, 20, 24)));
        target.set_rotateable(1);
        // Deliberately no DefCore Grab bit: C4Command passes OCF_All to At.
        target.set_grab(0);
        let no_reject_script = r#"#strict
local grabbed_calls, grabbed_controller;
protected func Grabbed(clonk, grab)
{
  grabbed_calls++;
  grabbed_controller = GetController();
  clonk->NoteGrabbed(this(), grab, grabbed_controller);
  if (clonk->ReleaseAfterGrabbed()) clonk->SetAction("Walk");
}
"#;
        let mut no_reject =
            Definition::from_script("OGNR", "Grabbed-only target", no_reject_script)
                .expect("Grabbed-only target compiles");
        no_reject.set_c4_callback_convention(true);
        no_reject.set_shape_rect(Some(DefinitionRect::new(-10, -12, 20, 24)));
        no_reject.set_grab(0);

        let mut engine = Engine::with_seed(176);
        engine.register_definition(actor).expect("actor registers");
        engine.register_definition(target).expect("target registers");
        engine
            .register_definition(no_reject)
            .expect("Grabbed-only target registers");
        for player in [1, 2, 7] {
            engine
                .register_player(PlayerConfig::new(player, format!("Player {player}")))
                .expect("player registers");
        }
        engine
    }

    fn spawn_object_com_grab_probe(
        engine: &mut Engine,
        action: &str,
        x: i32,
    ) -> (ObjectId, ObjectId) {
        spawn_object_com_grab_probe_with_target(engine, action, x, "OGTG")
    }

    fn spawn_object_com_grab_probe_with_target(
        engine: &mut Engine,
        action: &str,
        x: i32,
        target_definition: &str,
    ) -> (ObjectId, ObjectId) {
        let position = Vector2::new(x, 100);
        let actor = engine
            .spawn_object(
                SpawnConfig::new("OGAC")
                    .with_owner(1)
                    .with_alive(true)
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_position(position)
                    .with_action(ActionState::new(action))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("actor spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new(target_definition)
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(position),
            )
            .expect("target spawns");
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_index, "ResetGrabCallbacks", Vec::new())
            .expect("probe resets");
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        (actor, target)
    }

    #[test]
    fn object_com_grab_matches_cpp_callbacks_controller_walk_and_stop_gates() {
        let mut engine = object_com_grab_test_engine();
        let (walk_actor, walk_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 0);
        let stopped = [
            spawn_object_com_grab_probe(&mut engine, "Build", 100),
            spawn_object_com_grab_probe(&mut engine, "Chop", 200),
            spawn_object_com_grab_probe(&mut engine, "Dig", 300),
        ];
        let stopped_build_actor = stopped[0].0;
        let (flight_actor, flight_target) =
            spawn_object_com_grab_probe(&mut engine, "Flight", 400);
        let (swim_actor, swim_target) =
            spawn_object_com_grab_probe(&mut engine, "Swim", 450);
        let (removed_actor, removed_actor_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 500);
        let removed_actor_index = engine
            .find_object_index(removed_actor)
            .expect("removal actor exists");
        engine
            .call_object_function(
                removed_actor_index,
                "ArmGrabDeletion",
                vec![Value::Int(1)],
            )
            .expect("self-removal arms");
        let (target_remover, removed_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 600);
        let target_remover_index = engine
            .find_object_index(target_remover)
            .expect("target remover exists");
        engine
            .call_object_function(
                target_remover_index,
                "ArmGrabDeletion",
                vec![Value::Int(2)],
            )
            .expect("target-removal arms");
        let (far_builder, far_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 700);
        let far_target_index = engine
            .find_object_index(far_target)
            .expect("far target exists");
        engine.objects[far_target_index].set_position(Vector2::new(760, 100));
        let (command_replacer, replacement_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 800);
        let command_replacer_index = engine
            .find_object_index(command_replacer)
            .expect("command replacer exists");
        engine
            .call_object_function(
                command_replacer_index,
                "ArmGrabDeletion",
                vec![Value::Int(3)],
            )
            .expect("command replacement arms");
        let (cleared_builder, cleared_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 900);
        let cleared_builder_index = engine
            .find_object_index(cleared_builder)
            .expect("cleared builder exists");
        engine
            .call_object_function(
                cleared_builder_index,
                "ArmStopTargetClear",
                vec![object_reference_value(cleared_target)],
            )
            .expect("stop target clear arms");
        let (sequential_builder, sequential_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 1_000);
        let sequential_builder_index = engine
            .find_object_index(sequential_builder)
            .expect("sequential builder exists");
        engine
            .call_object_function(sequential_builder_index, "ArmStopToDig", Vec::new())
            .expect("Build-to-Dig stop arms");
        let (mutated_actor, mutated_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 1_100);
        let mutated_actor_index = engine
            .find_object_index(mutated_actor)
            .expect("mutated actor exists");
        engine
            .call_object_function(
                mutated_actor_index,
                "ArmGrabDeletion",
                vec![Value::Int(4)],
            )
            .expect("Grabbed mutation arms");
        let (locked_actor, locked_target) =
            spawn_object_com_grab_probe(&mut engine, "LockedWalk", 1_200);
        let (locked_build_actor, locked_build_target) =
            spawn_object_com_grab_probe(&mut engine, "LockedBuild", 1_250);
        let (inactive_actor, inactive_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 1_300);
        let inactive_actor_index = engine
            .find_object_index(inactive_actor)
            .expect("inactive-target actor exists");
        engine
            .call_object_function(
                inactive_actor_index,
                "DeactivateTarget",
                vec![object_reference_value(inactive_target)],
            )
            .expect("target becomes inactive without pointer clearing");
        let (clear_then_detach, clear_then_detach_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 1_400);
        let clear_then_detach_index = engine
            .find_object_index(clear_then_detach)
            .expect("clear-then-detach actor exists");
        engine
            .call_object_function(
                clear_then_detach_index,
                "ArmStopPointerOrder",
                vec![object_reference_value(clear_then_detach_target), Value::Int(1)],
            )
            .expect("clear-then-detach arms");
        let (detach_then_remove, detach_then_remove_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 1_500);
        let detach_then_remove_index = engine
            .find_object_index(detach_then_remove)
            .expect("detach-then-remove actor exists");
        engine
            .call_object_function(
                detach_then_remove_index,
                "ArmStopPointerOrder",
                vec![object_reference_value(detach_then_remove_target), Value::Int(2)],
            )
            .expect("detach-then-remove arms");
        let (reject_detacher, reject_removed_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 1_600);
        let reject_detacher_index = engine
            .find_object_index(reject_detacher)
            .expect("Reject detacher exists");
        engine
            .call_object_function(
                reject_detacher_index,
                "ArmGrabDeletion",
                vec![Value::Int(5)],
            )
            .expect("Reject detach-before-remove arms");

        engine.tick_without_snapshot().expect("Grab commands execute");

        for (actor, target) in std::iter::once((walk_actor, walk_target)).chain(stopped) {
            let actor = engine.object_snapshot(actor).expect("grabber remains");
            assert_eq!(actor.action.name, "Push");
            assert_eq!(actor.action.target, Some(target));
            assert_eq!(actor.command_direction, CommandDirection::Stop);
            assert_eq!(actor.local_vars.get("order"), Some(&Value::Int(1234)));
            assert_eq!(
                actor.local_vars.get("push_start_comdir"),
                Some(&Value::Int(CommandDirection::Stop.to_script_value()))
            );
            assert_eq!(
                actor.local_vars.get("push_start_action"),
                Some(&Value::String("Push".to_string().into()))
            );
            assert_eq!(
                actor.local_vars.get("push_start_target"),
                Some(&object_reference_value(target))
            );
            assert_eq!(
                actor.local_vars.get("grab_target"),
                Some(&object_reference_value(target))
            );
            assert_eq!(actor.local_vars.get("grab_flag"), Some(&Value::Bool(true)));
            assert_eq!(
                actor.local_vars.get("target_controller_in_grab"),
                Some(&Value::Int(2)),
                "Grab runs before Controller propagation"
            );
            assert_eq!(
                actor.local_vars.get("grabbed_controller"),
                Some(&Value::Int(7)),
                "Grabbed sees the actor's post-Grab Controller"
            );
            assert_eq!(
                engine.object_snapshot(target).expect("target remains").controller,
                7
            );
        }
        let stopped_build = engine
            .object_snapshot(stopped_build_actor)
            .expect("stopped builder remains");
        assert_eq!(stopped_build.local_vars.get("stop_order"), Some(&Value::Int(12)));
        assert_eq!(
            stopped_build.local_vars.get("stop_abort_action"),
            Some(&Value::String("Idle".to_string().into()))
        );
        assert_eq!(
            stopped_build.local_vars.get("stop_walk_start_action"),
            Some(&Value::String("Walk".to_string().into()))
        );

        for (actor, target, expected_action) in [
            (flight_actor, flight_target, "Flight"),
            (swim_actor, swim_target, "Swim"),
        ] {
            let actor = engine.object_snapshot(actor).expect("air/water actor remains");
            assert_eq!(actor.action.name, expected_action);
            assert_eq!(actor.command_direction, CommandDirection::Stop);
            assert_eq!(actor.local_vars.get("order"), Some(&Value::Int(1)));
            assert_eq!(
                engine
                    .object_snapshot(target)
                    .expect("air/water target remains")
                    .controller,
                2,
                "non-Walk ObjectComGrab cannot propagate Controller"
            );
        }

        let removed_actor_target = engine
            .object_snapshot(removed_actor_target)
            .expect("self-removal target remains");
        assert_eq!(removed_actor_target.controller, 2);
        assert_eq!(
            removed_actor_target.local_vars.get("grabbed_calls"),
            Some(&Value::Nil),
            "Grabbed is suppressed when Grab removes the actor"
        );

        let target_remover = engine
            .object_snapshot(target_remover)
            .expect("target remover remains");
        assert_eq!(target_remover.local_vars.get("order"), Some(&Value::Int(123)));
        assert!(
            engine
                .object_snapshot(removed_target)
                .is_none_or(|target| target.status == ObjectStatus::Deleted),
            "Grab may remove the target before the survival gate"
        );

        let far_builder = engine
            .object_snapshot(far_builder)
            .expect("far builder remains");
        assert_eq!(
            far_builder.action.name, "Walk",
            "Build runs the full ObjectComStop before testing At"
        );
        assert_eq!(
            far_builder.command_stack.command_names(),
            vec!["MoveTo".to_string(), "Grab".to_string()],
            "the live post-stop At result inserts MoveTo in the same command pass"
        );

        let command_replacer = engine
            .object_snapshot(command_replacer)
            .expect("command replacer remains");
        assert_eq!(command_replacer.local_vars.get("order"), Some(&Value::Int(1234)));
        assert_eq!(
            command_replacer.command_stack.command_names(),
            vec!["Wait".to_string()],
            "Grab callback command replacement survives the remaining callbacks"
        );
        assert_eq!(
            engine
                .object_snapshot(replacement_target)
                .expect("replacement target remains")
                .controller,
            7
        );

        let cleared_builder = engine
            .object_snapshot(cleared_builder)
            .expect("cleared builder remains");
        assert_eq!(cleared_builder.action.name, "Walk");
        assert_eq!(cleared_builder.local_vars.get("order"), Some(&Value::Nil));
        assert_eq!(
            cleared_builder.local_vars.get("finished"),
            Some(&Value::String("Grab".to_string().into())),
            "post-stop null Target fails in the same command execution"
        );
        assert!(cleared_builder.command_stack.is_empty());
        let cleared_target = engine
            .object_snapshot(cleared_target)
            .expect("inactive target remains");
        assert_eq!(cleared_target.status, ObjectStatus::Inactive);
        assert_eq!(
            cleared_target.local_vars.get("reject_calls"),
            None,
            "RejectGrabbed is after the post-stop null-target check"
        );

        let sequential_builder = engine
            .object_snapshot(sequential_builder)
            .expect("sequential builder remains");
        assert_eq!(sequential_builder.action.name, "Push");
        assert_eq!(sequential_builder.action.target, Some(sequential_target));
        assert_eq!(
            sequential_builder.local_vars.get("stop_walk_starts"),
            Some(&Value::Int(2)),
            "BUILD stop is followed by an independent DIG stop"
        );
        assert_eq!(sequential_builder.local_vars.get("order"), Some(&Value::Int(1234)));

        let mutated_actor = engine
            .object_snapshot(mutated_actor)
            .expect("Grabbed-mutated actor remains");
        assert_eq!(mutated_actor.action.name, "Walk");
        assert_eq!(mutated_actor.local_vars.get("order"), Some(&Value::Int(1234)));
        assert_eq!(
            mutated_actor.command_stack.command_names(),
            vec!["Grab".to_string()],
            "Grabbed's action mutation survives and the unfinished Grab remains"
        );
        assert_eq!(
            engine
                .object_snapshot(mutated_target)
                .expect("mutation target remains")
                .controller,
            7
        );

        let locked_actor = engine
            .object_snapshot(locked_actor)
            .expect("locked-Walk actor remains");
        assert_eq!(locked_actor.action.name, "LockedWalk");
        assert_eq!(locked_actor.command_direction, CommandDirection::Stop);
        assert_eq!(locked_actor.local_vars.get("order"), Some(&Value::Int(1)));
        assert_eq!(
            engine
                .object_snapshot(locked_target)
                .expect("locked-Walk target remains")
                .controller,
            2,
            "ObjectActionPush is non-forced and respects NoOtherAction"
        );

        let locked_build_actor = engine
            .object_snapshot(locked_build_actor)
            .expect("locked-Build actor remains");
        assert_eq!(
            locked_build_actor.local_vars.get("reject_action"),
            Some(&Value::String("LockedBuild".to_string().into())),
            "Grab's ObjectComStop cannot bypass NoOtherAction before RejectGrabbed"
        );
        assert_eq!(locked_build_actor.command_direction, CommandDirection::Stop);
        assert_eq!(locked_build_actor.local_vars.get("order"), Some(&Value::Int(1)));
        assert_eq!(
            engine
                .object_snapshot(locked_build_target)
                .expect("locked-Build target remains")
                .controller,
            2,
            "Grab's ObjectComStop is non-forced and cannot bypass NoOtherAction"
        );

        let inactive_actor = engine
            .object_snapshot(inactive_actor)
            .expect("inactive-target actor remains");
        assert_eq!(inactive_actor.action.name, "Push");
        assert_eq!(inactive_actor.action.target, Some(inactive_target));
        assert_eq!(inactive_actor.local_vars.get("order"), Some(&Value::Int(1234)));
        let inactive_target = engine
            .object_snapshot(inactive_target)
            .expect("inactive target remains addressable");
        assert_eq!(inactive_target.status, ObjectStatus::Inactive);
        assert_eq!(inactive_target.controller, 7);

        let clear_then_detach = engine
            .object_snapshot(clear_then_detach)
            .expect("clear-then-detach actor remains");
        assert_eq!(clear_then_detach.action.name, "Walk");
        assert_eq!(
            clear_then_detach.command_stack.command_names(),
            vec!["Wait".to_string()],
            "a pointer cleared before detachment remains null"
        );
        assert_eq!(clear_then_detach.local_vars.get("order"), Some(&Value::Nil));

        let detach_then_remove = engine
            .object_snapshot(detach_then_remove)
            .expect("detach-then-remove actor remains");
        assert_eq!(detach_then_remove.action.name, "Walk");
        assert_eq!(
            detach_then_remove.command_stack.command_names(),
            vec!["MoveTo".to_string(), "Wait".to_string()],
            "a pointer frozen by detachment still queues MoveTo after Status becomes zero"
        );
        assert!(
            engine
                .object_snapshot(detach_then_remove_target)
                .is_none_or(|target| target.status == ObjectStatus::Deleted)
        );

        let reject_detacher = engine
            .object_snapshot(reject_detacher)
            .expect("Reject detacher remains");
        assert_eq!(reject_detacher.action.name, "Walk");
        assert_eq!(reject_detacher.action.target, Some(reject_removed_target));
        assert_eq!(reject_detacher.local_vars.get("order"), Some(&Value::Int(123)));
        assert_eq!(
            reject_detacher.command_stack.command_names(),
            vec!["Wait".to_string(), "Wait".to_string()],
            "ObjectComGrab runs before same-frame PUSH notices the status-zero target and adds its delay"
        );
    }

    #[test]
    fn execute_command_runs_object_com_grab_callbacks_before_returning() {
        let mut engine = object_com_grab_test_engine();
        let (actor, target) = spawn_object_com_grab_probe(&mut engine, "Walk", 0);
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    actor_index,
                    "RunGrabNow",
                    vec![object_reference_value(target), Value::Int(0)],
                )
                .expect("RunGrabNow executes"),
            Value::Int(1234)
        );
        let actor = engine.object_snapshot(actor).expect("actor remains");
        assert_eq!(actor.action.name, "Push");
        assert_eq!(actor.action.target, Some(target));
        assert_eq!(actor.local_vars.get("grabbed_controller"), Some(&Value::Int(7)));
        assert_eq!(
            actor.local_vars.get("push_start_comdir"),
            Some(&Value::Int(CommandDirection::Stop.to_script_value()))
        );
        assert_eq!(
            actor.local_vars.get("push_start_action"),
            Some(&Value::String("Push".to_string().into()))
        );
        assert_eq!(
            actor.local_vars.get("push_start_target"),
            Some(&object_reference_value(target))
        );
        assert_eq!(
            engine.object_snapshot(target).expect("target remains").controller,
            7
        );

        let (self_remover, self_remover_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 50);
        let self_remover_index = engine
            .find_object_index(self_remover)
            .expect("self-removing actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    self_remover_index,
                    "RunGrabNow",
                    vec![object_reference_value(self_remover_target), Value::Int(1)],
                )
                .expect("self-removing Grab executes"),
            Value::Int(123)
        );
        let self_remover_target = engine
            .object_snapshot(self_remover_target)
            .expect("self-removal target remains");
        assert_eq!(self_remover_target.controller, 2);
        assert_eq!(
            self_remover_target.local_vars.get("grabbed_calls"),
            Some(&Value::Nil),
            "status-zero grabber suppresses Controller propagation and Grabbed"
        );

        let (remover, doomed) = spawn_object_com_grab_probe(&mut engine, "Walk", 100);
        let remover_index = engine.find_object_index(remover).expect("remover exists");
        assert_eq!(
            engine
                .call_object_function(
                    remover_index,
                    "RunGrabNow",
                    vec![object_reference_value(doomed), Value::Int(2)],
                )
                .expect("target-removing Grab executes"),
            Value::Int(123)
        );
        let remover = engine.object_snapshot(remover).expect("remover remains");
        assert_eq!(remover.local_vars.get("grabbed_target"), Some(&Value::Nil));

        let (plain_actor, plain_target) =
            spawn_object_com_grab_probe_with_target(&mut engine, "Walk", 200, "OGNR");
        let plain_actor_index = engine
            .find_object_index(plain_actor)
            .expect("plain actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    plain_actor_index,
                    "RunGrabNow",
                    vec![object_reference_value(plain_target), Value::Int(0)],
                )
                .expect("missing RejectGrabbed is accepted"),
            Value::Int(234)
        );
        assert_eq!(
            engine
                .object_snapshot(plain_target)
                .expect("plain target remains")
                .controller,
            7,
            "host preview creates a target scope before propagating Controller"
        );

        let (stop_actor, stop_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 300);
        let stop_actor_index = engine
            .find_object_index(stop_actor)
            .expect("stop actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    stop_actor_index,
                    "RunStopClearGrabNow",
                    vec![object_reference_value(stop_target)],
                )
                .expect("synchronous stop-clear executes"),
            Value::String("Grab".to_string().into())
        );
        let stop_actor = engine.object_snapshot(stop_actor).expect("stop actor remains");
        assert!(stop_actor.command_stack.is_empty());
        assert_eq!(stop_actor.local_vars.get("order"), Some(&Value::Nil));
        assert_eq!(stop_actor.local_vars.get("stop_order"), Some(&Value::Int(12)));
        assert_eq!(
            stop_actor.local_vars.get("stop_abort_action"),
            Some(&Value::String("Idle".to_string().into()))
        );
        assert_eq!(
            stop_actor.local_vars.get("stop_walk_start_action"),
            Some(&Value::String("Walk".to_string().into()))
        );
        let stop_target = engine
            .object_snapshot(stop_target)
            .expect("inactive stop target remains");
        assert_eq!(stop_target.status, ObjectStatus::Inactive);
        assert_eq!(stop_target.local_vars.get("reject_calls"), None);

        let (sequential_actor, sequential_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 400);
        let sequential_actor_index = engine
            .find_object_index(sequential_actor)
            .expect("sequential actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    sequential_actor_index,
                    "RunStopToDigGrabNow",
                    vec![object_reference_value(sequential_target)],
                )
                .expect("synchronous Build-to-Dig Grab executes"),
            Value::Int(1234)
        );
        let sequential_actor = engine
            .object_snapshot(sequential_actor)
            .expect("sequential actor remains");
        assert_eq!(sequential_actor.action.name, "Push");
        assert_eq!(
            sequential_actor.local_vars.get("stop_walk_starts"),
            Some(&Value::Int(2))
        );

        let (mutated_actor, mutated_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 500);
        let mutated_actor_index = engine
            .find_object_index(mutated_actor)
            .expect("mutated actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    mutated_actor_index,
                    "RunGrabNow",
                    vec![object_reference_value(mutated_target), Value::Int(4)],
                )
                .expect("Grabbed action mutation executes"),
            Value::Int(1234)
        );
        let mutated_actor = engine
            .object_snapshot(mutated_actor)
            .expect("mutated actor remains");
        assert_eq!(mutated_actor.action.name, "Walk");
        assert_eq!(
            mutated_actor.command_stack.command_names(),
            vec!["Grab".to_string()]
        );

        let (locked_actor, locked_target) =
            spawn_object_com_grab_probe(&mut engine, "LockedWalk", 600);
        let locked_actor_index = engine
            .find_object_index(locked_actor)
            .expect("locked actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    locked_actor_index,
                    "RunGrabNow",
                    vec![object_reference_value(locked_target), Value::Int(0)],
                )
                .expect("locked-Walk Grab executes"),
            Value::Int(1)
        );
        let locked_actor = engine
            .object_snapshot(locked_actor)
            .expect("locked actor remains");
        assert_eq!(locked_actor.action.name, "LockedWalk");
        assert_eq!(locked_actor.command_direction, CommandDirection::Stop);
        assert_eq!(
            engine
                .object_snapshot(locked_target)
                .expect("locked target remains")
                .controller,
            2
        );

        let (locked_build_actor, locked_build_target) =
            spawn_object_com_grab_probe(&mut engine, "LockedBuild", 650);
        let locked_build_actor_index = engine
            .find_object_index(locked_build_actor)
            .expect("locked-Build actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    locked_build_actor_index,
                    "RunGrabNow",
                    vec![object_reference_value(locked_build_target), Value::Int(0)],
                )
                .expect("locked-Build Grab executes"),
            Value::Int(1)
        );
        let locked_build_actor = engine
            .object_snapshot(locked_build_actor)
            .expect("locked-Build actor remains");
        assert_eq!(locked_build_actor.action.name, "LockedBuild");
        assert_eq!(locked_build_actor.command_direction, CommandDirection::Stop);
        assert_eq!(
            engine
                .object_snapshot(locked_build_target)
                .expect("locked-Build target remains")
                .controller,
            2
        );

        let (inactive_actor, inactive_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 700);
        let inactive_actor_index = engine
            .find_object_index(inactive_actor)
            .expect("inactive-target actor exists");
        engine
            .call_object_function(
                inactive_actor_index,
                "DeactivateTarget",
                vec![object_reference_value(inactive_target)],
            )
            .expect("host target becomes inactive without pointer clearing");
        assert_eq!(
            engine
                .call_object_function(
                    inactive_actor_index,
                    "RunGrabNow",
                    vec![object_reference_value(inactive_target), Value::Int(0)],
                )
                .expect("inactive-target Grab executes"),
            Value::Int(1234)
        );
        assert_eq!(
            engine
                .object_snapshot(inactive_target)
                .expect("inactive host target remains")
                .controller,
            7
        );

        let (clear_then_detach, clear_then_detach_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 800);
        let clear_then_detach_index = engine
            .find_object_index(clear_then_detach)
            .expect("host clear-then-detach actor exists");
        engine
            .call_object_function(
                clear_then_detach_index,
                "RunStopPointerGrabNow",
                vec![object_reference_value(clear_then_detach_target), Value::Int(1)],
            )
            .expect("host clear-then-detach executes");
        assert_eq!(
            engine
                .object_snapshot(clear_then_detach)
                .expect("host clear-then-detach actor remains")
                .command_stack
                .command_names(),
            vec!["Wait".to_string()]
        );

        let (detach_then_remove, detach_then_remove_target) =
            spawn_object_com_grab_probe(&mut engine, "Build", 900);
        let detach_then_remove_index = engine
            .find_object_index(detach_then_remove)
            .expect("host detach-then-remove actor exists");
        engine
            .call_object_function(
                detach_then_remove_index,
                "RunStopPointerGrabNow",
                vec![object_reference_value(detach_then_remove_target), Value::Int(2)],
            )
            .expect("host detach-then-remove executes");
        assert_eq!(
            engine
                .object_snapshot(detach_then_remove)
                .expect("host detach-then-remove actor remains")
                .command_stack
                .command_names(),
            vec!["MoveTo".to_string(), "Wait".to_string()]
        );

        let (reject_detacher, reject_removed_target) =
            spawn_object_com_grab_probe(&mut engine, "Walk", 1_000);
        let reject_detacher_index = engine
            .find_object_index(reject_detacher)
            .expect("host Reject detacher exists");
        assert_eq!(
            engine
                .call_object_function(
                    reject_detacher_index,
                    "RunGrabNow",
                    vec![object_reference_value(reject_removed_target), Value::Int(5)],
                )
                .expect("host Reject detach-before-remove executes"),
            Value::Int(123)
        );
        let reject_detacher = engine
            .object_snapshot(reject_detacher)
            .expect("host Reject detacher remains");
        assert_eq!(reject_detacher.action.name, "Push");
        assert_eq!(reject_detacher.action.target, Some(reject_removed_target));
        assert_eq!(
            reject_detacher.command_stack.command_names(),
            vec!["Wait".to_string()]
        );
    }

    #[test]
    fn execute_command_grab_at_uses_live_construction_rotation_and_addtop_shape() {
        let mut engine = object_com_grab_test_engine();
        let spawn_actor = |engine: &mut Engine, position: Vector2| {
            let actor = engine
                .spawn_object(
                    SpawnConfig::new("OGAC")
                        .with_owner(1)
                        .with_alive(true)
                        .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                        .with_position(position)
                        .with_action(ActionState::new("Walk")),
                )
                .expect("shape-probe actor spawns");
            let actor_index = engine
                .find_object_index(actor)
                .expect("shape-probe actor exists");
            engine.force_object_position(actor_index, position);
            actor
        };
        let run_grab = |engine: &mut Engine, actor: ObjectId, target: ObjectId| {
            let actor_index = engine.find_object_index(actor).expect("shape actor exists");
            engine
                .call_object_function(
                    actor_index,
                    "RunGrabNow",
                    vec![object_reference_value(target), Value::Int(0)],
                )
                .expect("shape-probe Grab executes")
        };

        let short_target = engine
            .spawn_object(
                SpawnConfig::new("OGTG")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(0, 100))
                    .with_construction(FULL_CON / 2),
            )
            .expect("short target spawns");
        let short_position = engine
            .object_snapshot(short_target)
            .expect("short target exists")
            .position;
        let addtop_actor = spawn_actor(
            &mut engine,
            Vector2::new(short_position.x, short_position.y - 10),
        );
        assert_eq!(
            run_grab(&mut engine, addtop_actor, short_target),
            Value::Int(1234),
            "C4Object::addtop expands a short construction shape upward"
        );

        let outside_target = engine
            .spawn_object(
                SpawnConfig::new("OGTG")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(100, 100))
                    .with_construction(FULL_CON / 2),
            )
            .expect("outside target spawns");
        let outside_position = engine
            .object_snapshot(outside_target)
            .expect("outside target exists")
            .position;
        let outside_actor = spawn_actor(
            &mut engine,
            Vector2::new(outside_position.x, outside_position.y + 8),
        );
        assert_eq!(run_grab(&mut engine, outside_actor, outside_target), Value::Nil);
        let outside_actor = engine
            .object_snapshot(outside_actor)
            .expect("outside actor remains");
        assert_eq!(outside_actor.action.name, "Walk");
        assert_eq!(
            outside_actor.command_stack.command_names(),
            vec!["MoveTo".to_string(), "Grab".to_string()],
            "the unscaled definition rect must not make At succeed"
        );

        let rotated_target = engine
            .spawn_object(
                SpawnConfig::new("OGTG")
                    .with_owner(2)
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(200, 100))
                    .with_rotation(90),
            )
            .expect("rotated target spawns");
        let rotated_position = engine
            .object_snapshot(rotated_target)
            .expect("rotated target exists")
            .position;
        let rotated_actor = spawn_actor(
            &mut engine,
            Vector2::new(rotated_position.x - 15, rotated_position.y),
        );
        assert_eq!(
            run_grab(&mut engine, rotated_actor, rotated_target),
            Value::Int(1234),
            "At uses the target's rotated live shape"
        );
    }

    #[test]
    fn failed_call_feedback_observes_old_comdir_and_honors_live_tail_gates() {
        let actor_script = r#"#strict
local finished_calls, finished_dir;

public func RunNow() { return ExecuteCommand(); }

protected func ControlCommandFinished()
{
  finished_calls++;
  finished_dir = GetComDir();
}
"#;
        let target_script = |handled: &str| {
            format!(
                r#"#strict
local calls, caller_seen, tx_seen, ty_seen, target2_seen, pre_dir;

protected func WorkFailed(caller, tx, ty, other)
{{
  calls++;
  caller_seen = caller;
  tx_seen = tx;
  ty_seen = ty;
  target2_seen = other;
  pre_dir = GetComDir(caller);
  return {handled};
}}
"#
            )
        };

        let mut engine = Engine::with_seed(311);
        for (id, silent, crew) in [
            ("AEXE", false, true),
            ("ACRW", false, true),
            ("ASLT", true, true),
            ("ANCR", false, false),
        ] {
            let mut definition =
                Definition::from_script(id, id, actor_script).expect("actor script compiles");
            definition.set_crew_member(crew);
            definition.set_silent_commands(silent);
            engine
                .register_definition(definition)
                .expect("actor definition registers");
        }
        engine
            .register_script_definition("TGTF", "Falsy target", &target_script("0"))
            .expect("falsy target registers");
        engine
            .register_script_definition("TGTT", "Truthy target", &target_script("WOOD"))
            .expect("truthy target registers");
        engine.register_script_definition("MARK", "Marker", "#strict").expect("marker registers");

        let falsy_target = engine
            .spawn_object(SpawnConfig::new("TGTF"))
            .expect("falsy target spawns");
        let truthy_target = engine
            .spawn_object(SpawnConfig::new("TGTT"))
            .expect("truthy target spawns");
        let marker = engine
            .spawn_object(SpawnConfig::new("MARK"))
            .expect("marker spawns");

        let queue_failed_call = |engine: &mut Engine, actor: ObjectId, target: ObjectId| {
            let index = engine.find_object_index(actor).expect("actor exists");
            engine.objects[index]
                .commands
                .push_front(
                    CommandRequest::new(CommandId::Call)
                        .with_target(Some(target))
                        .with_target2(Some(marker))
                        .with_tx_definition("WOOD".into())
                        .with_ty(Some(17))
                        .with_data(CommandData::Text("Work".into()))
                        .with_mode(CommandMode::Base),
                )
                .expect("Call queues");
            assert!(engine.objects[index]
                .commands
                .fail_front_if(CommandId::Call));
            engine.refresh_object_ocf(index);
        };

        // Script ExecuteCommand must run the tail inside the current VM call.
        let execute_actor = engine
            .spawn_object(
                SpawnConfig::new("AEXE")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("ExecuteCommand actor spawns");
        queue_failed_call(&mut engine, execute_actor, falsy_target);
        let execute_index = engine
            .find_object_index(execute_actor)
            .expect("ExecuteCommand actor exists");
        assert_eq!(
            engine
                .call_object_function(execute_index, "RunNow", Vec::new())
                .expect("ExecuteCommand succeeds"),
            Value::Bool(true)
        );
        let execute = engine
            .object_snapshot(execute_actor)
            .expect("ExecuteCommand actor remains");
        assert_eq!(execute.command_direction, CommandDirection::Stop);
        assert_eq!(
            execute.local_vars.get("finished_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            execute.local_vars.get("finished_dir"),
            Some(&Value::Int(CommandDirection::Stop.to_script_value()))
        );
        let falsy = engine
            .object_snapshot(falsy_target)
            .expect("falsy target remains");
        assert_eq!(falsy.local_vars.get("calls"), Some(&Value::Int(1)));
        assert_eq!(
            falsy.local_vars.get("caller_seen"),
            Some(&Value::Object(execute_actor.as_u64()))
        );
        assert_eq!(
            falsy.local_vars.get("tx_seen"),
            Some(&Value::C4Id("WOOD".into()))
        );
        assert_eq!(falsy.local_vars.get("ty_seen"), Some(&Value::Int(17)));
        assert_eq!(
            falsy.local_vars.get("target2_seen"),
            Some(&Value::Object(marker.as_u64()))
        );
        assert_eq!(
            falsy.local_vars.get("pre_dir"),
            Some(&Value::Int(CommandDirection::Right.to_script_value())),
            "CallFailed runs before the common ComDir stop"
        );

        let normal_actor = engine
            .spawn_object(
                SpawnConfig::new("ACRW")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("normal actor spawns");
        let silent_actor = engine
            .spawn_object(
                SpawnConfig::new("ASLT")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("silent actor spawns");
        let noncrew_actor = engine
            .spawn_object(
                SpawnConfig::new("ANCR")
                    .with_alive(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("noncrew actor spawns");
        queue_failed_call(&mut engine, normal_actor, truthy_target);
        queue_failed_call(&mut engine, silent_actor, falsy_target);
        queue_failed_call(&mut engine, noncrew_actor, falsy_target);

        engine.tick_without_snapshot().expect("failure-tail tick succeeds");

        let normal = engine
            .object_snapshot(normal_actor)
            .expect("normal actor remains");
        assert_eq!(
            normal.command_direction,
            CommandDirection::Right,
            "truthy CallFailed suppresses the entire common tail"
        );
        assert_eq!(
            normal.local_vars.get("finished_dir"),
            Some(&Value::Int(CommandDirection::Right.to_script_value()))
        );
        let silent = engine
            .object_snapshot(silent_actor)
            .expect("silent actor remains");
        assert_eq!(
            silent.command_direction,
            CommandDirection::Right,
            "SilentCommands suppresses only the common tail"
        );
        let noncrew = engine
            .object_snapshot(noncrew_actor)
            .expect("noncrew actor remains");
        assert_eq!(noncrew.command_direction, CommandDirection::Right);

        let falsy = engine
            .object_snapshot(falsy_target)
            .expect("falsy target remains");
        assert_eq!(
            falsy.local_vars.get("calls"),
            Some(&Value::Int(2)),
            "silent crew still gets CallFailed; the noncrew does not"
        );
        let truthy = engine
            .object_snapshot(truthy_target)
            .expect("truthy target remains");
        assert_eq!(truthy.local_vars.get("calls"), Some(&Value::Int(1)));
    }

    #[test]
    fn call_command_preserves_arbitrary_tx_value_through_get_execute_failure_and_restore() {
        let actor_script = r#"#strict 3
local finished_count, finished_tx, finished_ty, finished_target2, finished_data;

public func Queue(target, tx, target2)
{
  return AddCommand(this(), "Call", target, tx, 17, target2, 0, "Work", 0, 1);
}

public func QueueData(target, tx, target2, data)
{
  return AddCommand(this(), "Call", target, tx, 17, target2, 0, data, 0, 1);
}

public func ReadTx() { return GetCommand(this(), 2, 0); }

protected func ControlCommandFinished(command, target, tx, ty, target2, data)
{
  finished_count++;
  finished_tx = tx;
  finished_ty = ty;
  finished_target2 = target2;
  finished_data = data;
}
"#;
        let target_script = r#"#strict 3
local success_count, success_tx, failed_count, failed_tx, last_ty, last_target2;

protected func Work(caller, tx, ty, target2)
{
  success_count++;
  success_tx = tx;
  last_ty = ty;
  last_target2 = target2;
  return true;
}

protected func WorkFailed(caller, tx, ty, target2)
{
  failed_count++;
  failed_tx = tx;
  last_ty = ty;
  last_target2 = target2;
  return true;
}
"#;
        let register = |engine: &mut Engine| {
            let mut actor =
                Definition::from_script("CTXA", "Call actor", actor_script).expect("actor compiles");
            actor.set_crew_member(true);
            engine
                .register_definition(actor)
                .expect("actor definition registers");
            engine
                .register_script_definition("CTXT", "Call target", target_script)
                .expect("target definition registers");
            engine
                .register_definition(simple_definition("CTXM"))
                .expect("marker definition registers");
            engine
                .register_definition(simple_definition("CTXD"))
                .expect("doomed definition registers");
        };

        let mut engine = Engine::with_seed(312);
        register(&mut engine);
        let actor = engine
            .spawn_object(
                SpawnConfig::new("CTXA")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("actor spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("CTXT").with_status(ObjectStatus::Inactive))
            .expect("inactive target spawns");
        let marker = engine
            .spawn_object(SpawnConfig::new("CTXM"))
            .expect("marker spawns");
        let doomed = engine
            .spawn_object(SpawnConfig::new("CTXD"))
            .expect("doomed object spawns");

        let queue = |engine: &mut Engine, tx: Value| {
            let actor_index = engine.find_object_index(actor).expect("actor exists");
            assert_eq!(
                engine
                    .call_object_function(
                        actor_index,
                        "Queue",
                        vec![
                            object_reference_value(target),
                            tx,
                            object_reference_value(marker),
                        ],
                    )
                    .expect("Call queues"),
                Value::Bool(true)
            );
        };
        let read_tx = |engine: &mut Engine| {
            let actor_index = engine.find_object_index(actor).expect("actor exists");
            engine
                .call_object_function(actor_index, "ReadTx", Vec::new())
                .expect("GetCommand reads Tx")
        };
        let round_trip = |engine: &Engine| {
            let json = engine
                .capture_state()
                .to_json_string()
                .expect("engine state serializes");
            let state = EngineState::from_json_str(&json).expect("engine state deserializes");
            let mut restored = Engine::with_seed(0);
            register(&mut restored);
            restored.restore_state(&state).expect("engine state restores");
            restored
        };

        let mut map = clonk_script::ValueMap::new();
        map.insert_key(
            Value::Object(marker.as_u64()),
            Value::Object(target.as_u64()),
        );
        map.insert(
            "nested".into(),
            Value::Array(vec![Value::Object(marker.as_u64()), Value::Bool(false)]),
        );
        let payloads = vec![
            Value::Nil,
            Value::Int(0),
            Value::Bool(false),
            Value::RawBool(7),
            Value::C4Id("WOOD".into()),
            Value::String("Call Tx".into()),
            Value::Object(marker.as_u64()),
            Value::Array(vec![
                Value::Object(marker.as_u64()),
                Value::Nil,
                Value::Int(0),
                Value::Bool(false),
            ]),
            Value::Proplist(map),
        ];

        for (index, payload) in payloads.into_iter().enumerate() {
            queue(&mut engine, payload.clone());
            assert_eq!(read_tx(&mut engine), payload, "live GetCommand tag {index}");
            engine = round_trip(&engine);
            assert_eq!(
                read_tx(&mut engine),
                payload,
                "restored GetCommand tag {index}"
            );

            engine
                .tick_without_snapshot()
                .expect("successful restored Call executes");
            let target_state = engine.object_snapshot(target).expect("target remains");
            assert_eq!(
                target_state.local_vars.get("success_count"),
                Some(&Value::Int(index as i32 + 1))
            );
            assert_eq!(target_state.local_vars.get("success_tx"), Some(&payload));
            assert_eq!(target_state.local_vars.get("last_ty"), Some(&Value::Int(17)));
            assert_eq!(
                target_state.local_vars.get("last_target2"),
                Some(&Value::Object(marker.as_u64()))
            );
            let actor_state = engine.object_snapshot(actor).expect("actor remains");
            assert_eq!(
                actor_state.local_vars.get("finished_count"),
                Some(&Value::Int(index as i32 * 2 + 1))
            );
            assert_eq!(actor_state.local_vars.get("finished_tx"), Some(&payload));
            assert_eq!(actor_state.local_vars.get("finished_data"), Some(&Value::Nil));
            assert_eq!(
                actor_state.command_direction,
                CommandDirection::Right,
                "successful Call does not stop ComDir"
            );

            queue(&mut engine, payload.clone());
            engine = round_trip(&engine);
            assert_eq!(
                read_tx(&mut engine),
                payload,
                "failure-side restored GetCommand tag {index}"
            );
            let actor_index = engine.find_object_index(actor).expect("actor exists");
            assert!(engine.objects[actor_index]
                .commands
                .fail_front_if(CommandId::Call));
            engine.refresh_object_ocf(actor_index);
            engine
                .tick_without_snapshot()
                .expect("failed restored Call executes");

            let target_state = engine.object_snapshot(target).expect("target remains");
            assert_eq!(
                target_state.local_vars.get("failed_count"),
                Some(&Value::Int(index as i32 + 1))
            );
            assert_eq!(target_state.local_vars.get("failed_tx"), Some(&payload));
            let actor_state = engine.object_snapshot(actor).expect("actor remains");
            assert_eq!(
                actor_state.local_vars.get("finished_count"),
                Some(&Value::Int(index as i32 * 2 + 2))
            );
            assert_eq!(actor_state.local_vars.get("finished_tx"), Some(&payload));
            assert_eq!(actor_state.local_vars.get("finished_data"), Some(&Value::Nil));
            assert_eq!(actor_state.command_direction, CommandDirection::Right);
        }

        let textless_tx = Value::Array(vec![Value::String("textless".into()), Value::Int(0)]);
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        assert_eq!(
            engine
                .call_object_function(
                    actor_index,
                    "QueueData",
                    vec![
                        object_reference_value(target),
                        textless_tx.clone(),
                        object_reference_value(marker),
                        Value::Int(99),
                    ],
                )
                .expect("non-string Call data queues"),
            Value::Bool(true)
        );
        assert_eq!(read_tx(&mut engine), textless_tx);
        engine = round_trip(&engine);
        assert_eq!(read_tx(&mut engine), textless_tx);
        engine
            .tick_without_snapshot()
            .expect("textless Call follows the normal failure path");
        let target_state = engine.object_snapshot(target).expect("target remains");
        assert_eq!(target_state.local_vars.get("success_count"), Some(&Value::Int(9)));
        assert_eq!(target_state.local_vars.get("failed_count"), Some(&Value::Int(9)));
        let actor_state = engine.object_snapshot(actor).expect("actor remains");
        assert_eq!(
            actor_state.local_vars.get("finished_count"),
            Some(&Value::Int(19))
        );
        assert_eq!(actor_state.local_vars.get("finished_tx"), Some(&textless_tx));
        assert_eq!(actor_state.local_vars.get("finished_data"), Some(&Value::Nil));
        assert_eq!(
            actor_state.command_direction,
            CommandDirection::Stop,
            "an unhandled empty-name failure runs the common failure tail"
        );

        let missing_payload = Value::Array(vec![
            Value::Object(doomed.as_u64()),
            Value::Object(marker.as_u64()),
        ]);
        queue(&mut engine, missing_payload);
        let mut state = engine.capture_state();
        state
            .objects
            .retain(|object| object.snapshot.id != doomed);
        let json = state.to_json_string().expect("trimmed state serializes");
        let state = EngineState::from_json_str(&json).expect("trimmed state deserializes");
        let mut restored = Engine::with_seed(0);
        register(&mut restored);
        restored.restore_state(&state).expect("trimmed state restores");
        assert_eq!(
            read_tx(&mut restored),
            Value::Array(vec![Value::Nil, Value::Object(marker.as_u64())]),
            "missing saved object references denumerate recursively without changing survivors"
        );
    }

    #[test]
    fn build_without_can_construct_reports_cantbuild_message() {
        let script = r#"#strict 3
local needs_material_called;

public func RunNow() { return ExecuteCommand(); }

protected func BuildNeedsMaterial()
{
  needs_material_called = 1;
  return 1;
}
"#;
        let mut builder =
            Definition::from_script("BLDR", "Builder", script).expect("builder compiles");
        builder.set_crew_member(true);
        builder.set_physical(PhysicalInfo {
            can_construct: 0,
            ..PhysicalInfo::default()
        });
        let mut site = Definition::from_script("SITE", "Site", "#strict").expect("site compiles");
        site.set_constructable(true);

        let mut engine = Engine::with_seed(313);
        engine
            .register_definition(builder)
            .expect("builder registers");
        engine.register_definition(site).expect("site registers");
        let target = engine
            .spawn_object(SpawnConfig::new("SITE").with_construction(1_000))
            .expect("site spawns");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("builder spawns");
        let actor_index = engine.find_object_index(actor).expect("builder exists");
        engine.objects[actor_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Base),
            )
            .expect("Build queues");
        engine.refresh_object_ocf(actor_index);

        let snapshot = engine.tick().expect("Build failure tick succeeds");
        let actor_snapshot = snapshot.object(actor).expect("builder remains");
        assert_eq!(actor_snapshot.command_direction, CommandDirection::Stop);
        assert_eq!(
            actor_snapshot.local_vars.get("needs_material_called"),
            Some(&Value::Int(1)),
            "the explicit CANTBUILD message does not skip BuildNeedsMaterial"
        );
        assert_eq!(snapshot.hud.messages.len(), 1);
        assert_eq!(snapshot.hud.messages[0].kind, MessageKind::Target);
        assert_eq!(snapshot.hud.messages[0].target, Some(actor));
        assert_eq!(snapshot.hud.messages[0].lines, vec!["Builder can't build."]);

        let sync_actor = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("synchronous builder spawns");
        let sync_index = engine
            .find_object_index(sync_actor)
            .expect("synchronous builder exists");
        engine.objects[sync_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Base),
            )
            .expect("synchronous Build queues");
        engine.refresh_object_ocf(sync_index);
        assert_eq!(
            engine
                .call_object_function(sync_index, "RunNow", Vec::new())
                .expect("synchronous Build failure runs"),
            Value::Bool(true)
        );
        let sync_snapshot = engine.snapshot();
        assert_eq!(
            sync_snapshot
                .object(sync_actor)
                .expect("synchronous builder remains")
                .command_direction,
            CommandDirection::Stop
        );
        assert!(sync_snapshot.hud.messages.iter().any(|message| {
            message.kind == MessageKind::Target
                && message.target == Some(sync_actor)
                && message.lines == vec!["Builder can't build."]
        }));
    }

    #[test]
    fn synchronous_failed_build_appends_generated_material_message() {
        // FnExecuteCommand reaches C4Command::Fail synchronously. An ordinary
        // Build failure assigns GetNeededMatStr to failMessage and appends it
        // to the existing target message (C4Script.cpp:835-838;
        // C4Command.cpp:2185-2194,2229-2235).
        let script = r#"#strict 3
public func SeedMessage() { return Message("Working", this()); }
public func RunNow() { return ExecuteCommand(); }
"#;
        let mut builder =
            Definition::from_script("BLDR", "Builder", script).expect("builder compiles");
        builder.set_crew_member(true);
        builder.set_physical(PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        });
        let mut site = Definition::from_script("SITE", "Site", "#strict").expect("site compiles");
        site.set_constructable(true);
        site.set_components(vec![DefinitionComponent {
            id: "WOOD".to_owned(),
            count: 1,
        }]);

        let mut engine = Engine::with_seed(72);
        engine
            .register_definition(builder)
            .expect("builder registers");
        engine.register_definition(site).expect("site registers");
        engine.register_script_definition("WOOD", "Wood", "#strict").expect("wood registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("SITE")
                    .with_construction(1_000)
                    .with_ordered_components(vec![("WOOD".to_owned(), 0)]),
            )
            .expect("site spawns");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_controller(4),
            )
            .expect("builder spawns");
        let actor_index = engine.find_object_index(actor).expect("builder exists");
        engine.refresh_object_ocf(actor_index);
        assert_eq!(
            engine
                .call_object_function(actor_index, "SeedMessage", Vec::new())
                .expect("seed message succeeds"),
            Value::Bool(true)
        );
        let seeded = engine.snapshot();
        assert_eq!(seeded.hud.messages.len(), 1);
        let seeded_id = seeded.hud.messages[0].id;
        assert_eq!(seeded.hud.messages[0].lines, vec!["Working"]);

        engine.objects[actor_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Base),
            )
            .expect("Build queues");
        assert!(engine.objects[actor_index]
            .commands
            .fail_front_if(CommandId::Build));
        assert_eq!(
            engine
                .call_object_function(actor_index, "RunNow", Vec::new())
                .expect("synchronous Build failure runs"),
            Value::Bool(true)
        );

        let failed = engine.snapshot();
        assert_eq!(
            failed.hud.messages.len(),
            1,
            "C++ reuses the existing target message"
        );
        assert_eq!(failed.hud.messages[0].id, seeded_id);
        assert_eq!(
            failed.hud.messages[0].lines,
            vec!["Working", "Site", "needs", "1x Wood"]
        );

        engine.objects[actor_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Base),
            )
            .expect("second Build queues");
        assert!(engine.objects[actor_index]
            .commands
            .fail_front_if(CommandId::Build));
        assert_eq!(
            engine
                .call_object_function(actor_index, "RunNow", Vec::new())
                .expect("second synchronous Build failure runs"),
            Value::Bool(true)
        );
        assert_eq!(
            engine.snapshot().hud.messages,
            failed.hud.messages,
            "C4GameMessage::Append suppresses the repeated material text"
        );
    }

    #[test]
    fn completed_builders_queue_only_one_energy_command_in_same_tick() {
        let mut builder =
            Definition::from_script("BLDR", "Builder", "#strict").expect("builder compiles");
        builder.set_physical(PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        });
        let mut site = Definition::from_script("SITE", "Site", "#strict").expect("site compiles");
        site.set_line_connect(LINE_CONNECT_POWER_INPUT);

        let mut engine = Engine::with_seed(314);
        engine.set_structures_need_energy(true);
        engine
            .register_definition(builder)
            .expect("builder registers");
        engine.register_definition(site).expect("site registers");
        let target = engine
            .spawn_object(SpawnConfig::new("SITE"))
            .expect("site spawns");
        let builders = [
            engine
                .spawn_object(SpawnConfig::new("BLDR"))
                .expect("first builder spawns"),
            engine
                .spawn_object(SpawnConfig::new("BLDR"))
                .expect("second builder spawns"),
        ];
        for builder_id in builders {
            let index = engine
                .find_object_index(builder_id)
                .expect("builder exists");
            engine.objects[index]
                .commands
                .push_front(
                    CommandRequest::new(CommandId::Build)
                        .with_target(Some(target))
                        .with_mode(CommandMode::Base),
                )
                .expect("Build queues");
        }

        let snapshot = engine.tick().expect("builders execute");
        let energy_count = builders
            .iter()
            .filter_map(|id| snapshot.object(*id))
            .flat_map(|object| object.command_stack.command_names())
            .filter(|name| name == "Energy")
            .count();
        assert_eq!(
            energy_count, 1,
            "the later builder must see the earlier builder's live Energy command"
        );
    }

    #[test]
    fn failed_build_feedback_uses_live_component_zero_and_truthy_still_stops() {
        let actor_script = r#"#strict 3
local needed_id, needed_count, needed_id_is_nil, needed_count_is_int;
local pre_dir, finished_dir;

public func RunNow() { return ExecuteCommand(); }

protected func BuildNeedsMaterial(component_id, count)
{
  needed_id = component_id;
  needed_count = count;
  needed_id_is_nil = GetType(component_id) == 0;
  needed_count_is_int = GetType(count) == C4V_Int;
  pre_dir = GetComDir();
  return 1;
}

protected func ControlCommandFinished() { finished_dir = GetComDir(); }
"#;
        let mut builder =
            Definition::from_script("BLDR", "Builder", actor_script).expect("builder compiles");
        builder.set_crew_member(true);
        let mut site = Definition::from_script("SITE", "Site", "#strict").expect("site compiles");
        // Deliberately differs from the live object Component list below.
        site.set_components(vec![DefinitionComponent {
            id: "WOOD".into(),
            count: 99,
        }]);

        let mut engine = Engine::with_seed(312);
        engine
            .register_definition(builder)
            .expect("builder registers");
        engine.register_definition(site).expect("site registers");
        engine.register_script_definition("WOOD", "Wood", "#strict").expect("wood registers");
        engine.register_script_definition("METL", "Metal", "#strict").expect("metal registers");

        let target = engine
            .spawn_object(
                SpawnConfig::new("SITE")
                    .with_ordered_components(vec![("METL".into(), 3), ("WOOD".into(), 8)]),
            )
            .expect("site spawns");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("builder spawns");
        let actor_index = engine.find_object_index(actor).expect("builder exists");
        engine.objects[actor_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Base),
            )
            .expect("Build queues");
        assert!(engine.objects[actor_index]
            .commands
            .fail_front_if(CommandId::Build));
        engine.refresh_object_ocf(actor_index);

        engine.tick_without_snapshot().expect("Build failure tick succeeds");

        let actor = engine.object_snapshot(actor).expect("builder remains");
        assert_eq!(
            actor.local_vars.get("needed_id"),
            Some(&Value::C4Id("METL".into()))
        );
        assert_eq!(actor.local_vars.get("needed_count"), Some(&Value::Int(3)));
        assert_eq!(
            actor.local_vars.get("pre_dir"),
            Some(&Value::Int(CommandDirection::Right.to_script_value()))
        );
        assert_eq!(actor.command_direction, CommandDirection::Stop);
        assert_eq!(
            actor.local_vars.get("finished_dir"),
            Some(&Value::Int(CommandDirection::Stop.to_script_value()))
        );

        let empty_target = engine
            .spawn_object(
                SpawnConfig::new("SITE")
                    .with_ordered_components(Vec::<(String, i32)>::new()),
            )
            .expect("component-free site spawns");
        let empty_actor = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("component-free builder spawns");
        let empty_actor_index = engine
            .find_object_index(empty_actor)
            .expect("component-free builder exists");
        engine.objects[empty_actor_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(empty_target))
                    .with_mode(CommandMode::Base),
            )
            .expect("component-free Build queues");
        assert!(engine.objects[empty_actor_index]
            .commands
            .fail_front_if(CommandId::Build));
        engine.refresh_object_ocf(empty_actor_index);
        assert_eq!(
            engine
                .call_object_function(empty_actor_index, "RunNow", Vec::new())
                .expect("component-free Build failure succeeds"),
            Value::Bool(true)
        );

        let empty_actor = engine
            .object_snapshot(empty_actor)
            .expect("component-free builder remains");
        assert_eq!(
            empty_actor.local_vars.get("pre_dir"),
            Some(&Value::Int(CommandDirection::Right.to_script_value())),
            "BuildNeedsMaterial must run even when Component[0] is empty: {:?}",
            empty_actor.local_vars
        );
        assert_eq!(empty_actor.command_direction, CommandDirection::Stop);
        assert_eq!(empty_actor.local_vars.get("needed_id"), Some(&Value::Nil));
        assert_eq!(
            empty_actor.local_vars.get("needed_count"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            empty_actor.local_vars.get("needed_id_is_nil"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            empty_actor.local_vars.get("needed_count_is_int"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn tutorial_special2_keeps_context_location_before_control_returns() {
        // FnExecuteCommand dispatches synchronously to C4Object::ExecuteCommand
        // (C4Script.cpp:835-838). ExecuteCommand calls
        // ~ControlCommandFinished while the finished command is still the
        // stack front, then clears all finished fronts (C4Object.cpp:3997-4007).
        let script = r#"#strict
local callback_name, callback_front, callback_comdir;

protected func ControlSpecial2()
{
  SetCommand(this(), "Context", 0, 17, 23, this());
  return ExecuteCommand();
}

protected func ControlCommandFinished(command)
{
  callback_name = command;
  callback_front = GetCommand(0);
  callback_comdir = GetComDir();
}
"#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        engine
            .register_player(PlayerConfig::new(1, "Player"))
            .expect("player registers");
        let clonk = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_alive(true),
            )
            .expect("clonk spawns");
        engine.select_crew(1, vec![clonk]).expect("crew selected");
        engine
            .set_crew_cursor(1, Some(clonk))
            .expect("cursor selected");
        let index = engine.find_object_index(clonk).expect("clonk exists");
        engine.objects[index].state.command_direction = CommandDirection::Right;

        assert!(
            engine
                .handle_control_command(1, ControlCommand::Special2, CommandKind::Press)
                .expect("ControlSpecial2 succeeds")
        );

        let index = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_name"),
            Some(&Value::String("Context".to_string().into()))
        );
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_front"),
            Some(&Value::String("Context".to_string().into())),
            "the callback observes the finished command before it is cleared"
        );
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_comdir"),
            Some(&Value::Int(CommandDirection::Stop.to_script_value()))
        );
        assert!(
            engine.objects[index].commands.is_empty(),
            "the finished front is cleared after the callback"
        );
        assert!(
            engine.pending_menu_requests.is_empty(),
            "C4MN_Context is installed by the engine, not deferred to app UI"
        );
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("Context installs its classic object menu");
        assert_eq!(menu.identification, Value::Int(14));
        assert_eq!(menu.style, 1);
        assert!(!menu.permanent);
        assert_eq!(menu.location, Some(Vector2::new(17, 23)));
        assert_eq!(menu.command_object, Some(clonk));
    }

    #[test]
    fn synchronous_float_move_to_uses_subpixel_position_like_cpp() {
        // FnExecuteCommand reads the live object's fix_x/fix_y. With target
        // (110,130), x=100.25 makes the strict 3:1 threshold choose Down;
        // reconstructing x=100 lands on the boundary and chooses DownRight
        // (C4Command.cpp:393-410).
        let script = r#"#strict
public func Steer()
{
  SetCommand(this(), "MoveTo", 0, 110, 130);
  ExecuteCommand();
  return ExecuteCommand();
}
"#;
        let mut floater =
            Definition::from_script("FLTR", "Floater", script).expect("script compiles");
        floater.set_physical(PhysicalInfo {
            float: 100,
            ..PhysicalInfo::default()
        });
        floater.configure_actions(
            Some("Float".to_string()),
            HashMap::from([(
                "Float".to_string(),
                ActionSpec::default().with_procedure("FLOAT"),
            )]),
        );
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(floater)
            .expect("definition registers");
        let object = engine
            .spawn_object(
                SpawnConfig::new("FLTR")
                    .with_position(Vector2::new(100, 100))
                    .with_fixed_position(FixedVec2::new(
                        itofix(100) + fixed100(25),
                        itofix(100),
                    ))
                    .with_action(ActionState::new("Float"))
                    .with_alive(true),
            )
            .expect("floater spawns");
        let index = engine.find_object_index(object).expect("floater exists");

        engine
            .call_object_function(index, "Steer", Vec::new())
            .expect("synchronous command runs");

        let object = engine.object_snapshot(object).expect("floater remains");
        assert_eq!(object.command_direction, CommandDirection::Down);
    }

    #[test]
    fn synchronous_move_to_stops_work_actions_before_same_execute_steering() {
        let script = r#"#strict
local stop_order;

public func RunNow() { return ExecuteCommand(); }
protected func WorkAbort() { stop_order = stop_order * 10 + 1; }
protected func ClearAbort()
{
  stop_order = stop_order * 10 + 1;
  SetCommand(this(), "Wait", 0, 1);
}
protected func WalkStart() { stop_order = stop_order * 10 + 2; }
"#;
        let mut definition =
            Definition::from_script("MVST", "MoveTo stopper", script).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default()
                .with_procedure("WALK")
                .with_start_call("WalkStart"),
        );
        for (name, procedure) in [
            ("Dig", "DIG"),
            ("Chop", "CHOP"),
            ("Build", "BUILD"),
            ("Bridge", "BRIDGE"),
        ] {
            actions.insert(
                name.to_string(),
                ActionSpec::default()
                    .with_procedure(procedure)
                    .with_abort_call("WorkAbort"),
            );
        }
        actions.insert(
            "DigClear".to_string(),
            ActionSpec::default()
                .with_procedure("DIG")
                .with_abort_call("ClearAbort"),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);

        let mut engine = Engine::with_seed(319);
        engine
            .register_definition(definition)
            .expect("definition registers");

        for (offset, action) in ["Dig", "Chop", "Build", "Bridge"]
            .into_iter()
            .enumerate()
        {
            let object = engine
                .spawn_object(
                    SpawnConfig::new("MVST")
                        .with_position(Vector2::new(100, 100 + offset as i32 * 30))
                        .with_fixed_velocity(FixedVec2::new(itofix(2), itofix(-3)))
                        .with_action(ActionState::new(action))
                        .with_command_direction(CommandDirection::Left)
                        .with_alive(true),
                )
                .expect("worker spawns");
            let index = engine.find_object_index(object).expect("worker exists");
            engine.objects[index]
                .commands
                .push_front(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(200))
                        .with_ty(Some(100 + offset as i32 * 30))
                        .with_evaluated(true),
                )
                .expect("MoveTo queues");

            assert_eq!(
                engine
                    .call_object_function(index, "RunNow", Vec::new())
                    .expect("MoveTo executes"),
                Value::Bool(true)
            );

            let live_index = engine.find_object_index(object).expect("worker remains");
            assert_eq!(engine.objects[live_index].fixed_velocity, FixedVec2::ZERO, "{action}");
            let object = engine.object_snapshot(object).expect("worker remains");
            assert_eq!(object.action.name, "Walk", "{action}");
            assert_eq!(
                object.command_direction,
                CommandDirection::Right,
                "{action}: steering must resume after ObjectComStop in the same Execute"
            );
            assert_eq!(
                object.local_vars.get("stop_order"),
                Some(&Value::Int(12)),
                "{action}: Idle transition abort precedes Walk start"
            );
            assert_eq!(object.command_stack.command_names(), vec!["MoveTo"]);
        }

        // ClearCommands/SetCommand only detaches an executing native
        // command (iExec=2); its current MoveTo body still resumes after the
        // callback and steers the object before being deleted.
        let replaced = engine
            .spawn_object(
                SpawnConfig::new("MVST")
                    .with_position(Vector2::new(100, 220))
                    .with_action(ActionState::new("DigClear"))
                    .with_command_direction(CommandDirection::Left)
                    .with_alive(true),
            )
            .expect("replacement worker spawns");
        let replaced_index = engine
            .find_object_index(replaced)
            .expect("replacement worker exists");
        engine.objects[replaced_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(200))
                    .with_ty(Some(220))
                    .with_evaluated(true),
            )
            .expect("replacement MoveTo queues");
        engine
            .call_object_function(replaced_index, "RunNow", Vec::new())
            .expect("replacement MoveTo executes");
        let replaced = engine
            .object_snapshot(replaced)
            .expect("replacement worker remains");
        assert_eq!(replaced.action.name, "Walk");
        assert_eq!(replaced.command_direction, CommandDirection::Right);
        assert_eq!(replaced.local_vars.get("stop_order"), Some(&Value::Int(12)));
        assert_eq!(replaced.command_stack.command_names(), vec!["Wait"]);

        // The ordinary object-tick path applies the same live command event
        // before ExecAction later in the frame.
        let tick_worker = engine
            .spawn_object(
                SpawnConfig::new("MVST")
                    .with_position(Vector2::new(100, 250))
                    .with_action(ActionState::new("Dig"))
                    .with_command_direction(CommandDirection::Left)
                    .with_alive(true),
            )
            .expect("tick worker spawns");
        let tick_index = engine
            .find_object_index(tick_worker)
            .expect("tick worker exists");
        engine.objects[tick_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(200))
                    .with_ty(Some(250))
                    .with_evaluated(true),
            )
            .expect("tick MoveTo queues");
        engine.tick_without_snapshot().expect("MoveTo tick succeeds");
        let tick_worker = engine
            .object_snapshot(tick_worker)
            .expect("tick worker remains");
        assert_eq!(tick_worker.action.name, "Walk");
        assert_eq!(tick_worker.command_direction, CommandDirection::Right);
        assert_eq!(
            tick_worker.local_vars.get("stop_order"),
            Some(&Value::Int(12))
        );

        // The auto-inserted bare Idle slot is inactive and fails an
        // out-of-range MoveTo; it is not confused with a real action name.
        let idle = engine
            .spawn_object(
                SpawnConfig::new("MVST")
                    .with_position(Vector2::new(100, 300))
                    .with_action(ActionState::new("Idle"))
                    .with_alive(true),
            )
            .expect("idle worker spawns");
        let idle_index = engine.find_object_index(idle).expect("idle worker exists");
        engine.objects[idle_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(200))
                    .with_ty(Some(300))
                    .with_evaluated(true),
            )
            .expect("idle MoveTo queues");
        engine
            .call_object_function(idle_index, "RunNow", Vec::new())
            .expect("idle MoveTo executes");
        assert!(
            engine
                .object_snapshot(idle)
                .expect("idle worker remains")
                .command_stack
                .is_empty(),
            "out-of-range ActIdle MoveTo fails and clears after its finished callback tail"
        );
    }

    #[test]
    fn normal_command_tick_calls_finished_before_clearing_like_cpp() {
        // Every C4Object::Execute runs ExecuteCommand first
        // (C4Object.cpp:1085,3997-4007), so the ordinary per-frame path
        // owes the same callback-before-clear ordering as the script host.
        let script = r#"#strict
local callback_name, callback_front;
public func Arm() { return SetCommand(this(), "Context", 0, 0, 0, this()); }
protected func ControlCommandFinished(command)
{
  callback_name = command;
  callback_front = GetCommand(0);
}
"#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        engine
            .register_player(PlayerConfig::new(1, "Player"))
            .expect("player registers");
        let clonk = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_alive(true),
            )
            .expect("clonk spawns");
        let index = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(index, "Arm", Vec::new())
                .expect("command arms"),
            Value::Bool(true)
        );

        engine.tick_without_snapshot().expect("command tick succeeds");

        let index = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_name"),
            Some(&Value::String("Context".to_string().into()))
        );
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_front"),
            Some(&Value::String("Context".to_string().into()))
        );
        assert!(engine.objects[index].commands.is_empty());
    }

    #[test]
    fn execute_command_targets_foreign_objects_and_empty_stacks_like_cpp() {
        // FnExecuteCommand accepts an explicit object and falls back to the
        // calling object only for null (C4Script.cpp:835-838). The object
        // method returns true even when its command stack is empty
        // (C4Object.cpp:3997-4007).
        let caller_script = r#"#strict
public func OpenOther(other)
{
  SetCommand(other, "Context", 0, 0, 0, other);
  return ExecuteCommand(other);
}
public func ExecuteEmpty(other) { return ExecuteCommand(other); }
"#;
        let target_script = r#"#strict
local finished;
protected func ControlCommandFinished(command) { finished = command; }
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_script_definition("CALL", "Caller", caller_script)
            .expect("caller registers");
        engine
            .register_script_definition("TARG", "Target", target_script)
            .expect("target registers");
        engine
            .register_player(PlayerConfig::new(1, "Player"))
            .expect("player registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_alive(true))
            .expect("caller spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("TARG")
                    .with_owner(1)
                    .with_alive(true),
            )
            .expect("target spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "OpenOther",
                    vec![Value::Object(target.as_u64())],
                )
                .expect("foreign ExecuteCommand succeeds"),
            Value::Bool(true)
        );
        let target_index = engine.find_object_index(target).expect("target exists");
        assert_eq!(
            engine.objects[target_index].state.local_vars.get("finished"),
            Some(&Value::String("Context".to_string().into()))
        );
        assert!(engine.objects[target_index].commands.is_empty());

        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "ExecuteEmpty",
                    vec![Value::Object(target.as_u64())],
                )
                .expect("empty ExecuteCommand succeeds"),
            Value::Bool(true)
        );
    }

    #[test]
    fn execute_command_dig_failure_is_synchronous_and_localized() {
        // C4Command::Dig calls ObjectComDig inside the same ExecuteCommand
        // invocation. The NODIG message and failed-command removal therefore
        // happen before the enclosing script call returns.
        let script = r#"#strict
local after_action, after_command;
public func RunNow()
{
  SetCommand(this(), "Dig", 0, 0, 100, 0, 1);
  ExecuteCommand();
  after_action = GetAction();
  after_command = GetCommand(0);
  return true;
}
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("definition compiles");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_no_other_action(true),
                ),
                (
                    "Dig".to_string(),
                    ActionSpec::default().with_procedure("DIG"),
                ),
            ]),
        );
        definition.set_physical(PhysicalInfo {
            can_dig: 1,
            ..Default::default()
        });
        let mut engine = Engine::new();
        engine.set_object_no_dig_resource_string("%s kann|nicht graben.");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_action(ActionState::new("Walk"))
                    .with_custom_name("Skript"),
            )
            .expect("actor spawns");
        let index = engine.find_object_index(actor).expect("actor exists");

        assert_eq!(
            engine
                .call_object_function(index, "RunNow", Vec::new())
                .expect("RunNow succeeds"),
            Value::Bool(true)
        );

        let index = engine.find_object_index(actor).expect("actor survives");
        assert_eq!(
            engine.objects[index].state.local_vars.get("after_action"),
            Some(&Value::String("Walk".to_string().into()))
        );
        assert_eq!(
            engine.objects[index]
                .state
                .local_vars
                .get("after_command"),
            Some(&Value::Nil)
        );
        assert!(engine.objects[index].commands.is_empty());
        let messages = engine.snapshot().hud.messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].target, Some(actor));
        assert_eq!(messages[0].lines, vec!["Skript kann", "nicht graben."]);
    }

    #[test]
    fn execute_command_dig_success_applies_data_and_steering_synchronously() {
        let script = r#"#strict
local after_action, after_data, after_dir;
public func RunNow()
{
  SetCommand(this(), "Dig", 0, 0, 100, 0, 1);
  ExecuteCommand();
  after_action = GetAction();
  after_data = GetActionData();
  after_dir = GetComDir();
  return true;
}
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("definition compiles");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Dig".to_string(),
                    ActionSpec::default().with_procedure("DIG"),
                ),
            ]),
        );
        definition.set_physical(PhysicalInfo {
            can_dig: 1,
            ..Default::default()
        });
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("CLNK").with_action(ActionState::new("Walk")),
            )
            .expect("actor spawns");
        let index = engine.find_object_index(actor).expect("actor exists");

        assert_eq!(
            engine
                .call_object_function(index, "RunNow", Vec::new())
                .expect("RunNow succeeds"),
            Value::Bool(true)
        );

        let index = engine.find_object_index(actor).expect("actor survives");
        assert_eq!(
            engine.objects[index].state.local_vars.get("after_action"),
            Some(&Value::String("Dig".to_string().into()))
        );
        assert_eq!(
            engine.objects[index].state.local_vars.get("after_data"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine.objects[index].state.local_vars.get("after_dir"),
            Some(&Value::Int(CommandDirection::DownLeft.to_script_value()))
        );
        assert_eq!(engine.objects[index].state.action.name, "Dig");
        assert_eq!(engine.objects[index].state.action.data, 1);
        assert_eq!(
            engine.objects[index].state.command_direction,
            CommandDirection::DownLeft
        );
        assert!(engine.snapshot().hud.messages.is_empty());
    }

    #[test]
    fn finished_callback_can_replace_the_front_before_clear_like_cpp() {
        // The clear loop re-reads `Command` after the callback
        // (C4Object.cpp:4001-4005). A callback SetCommand therefore
        // replaces the finished entry with an unfinished one that survives.
        let script = r#"#strict
public func Run()
{
  SetCommand(this(), "Context", 0, 0, 0, this());
  return ExecuteCommand();
}
protected func ControlCommandFinished() { SetCommand(this(), "Wait", 0, 5); }
"#;
        let mut engine = Engine::with_seed(3);
        engine.register_script_definition("CLNK", "Clonk", script).expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK").with_alive(true))
            .expect("clonk spawns");
        let index = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine
                .call_object_function(index, "Run", Vec::new())
                .expect("Run succeeds"),
            Value::Bool(true)
        );
        let index = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine.objects[index].commands.command_names(),
            vec!["Wait".to_string()]
        );
    }

    #[test]
    fn do_damage_asks_effects_for_non_living_and_fires_callback() {
        // C4Object::DoDamage (C4Object.cpp:1330-1343): NON-living things ask
        // their effects first (the inverse of DoEnergy's Alive gate), the
        // damage clamps at zero, and the Damage script callback fires with
        // (change, causedBy).
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Shell", priority = 100, interval = 0 } ] };
        }

        global func FxShellDamage(state, effect, damage, damage_type, cause_plr) {
            return damage / 2;
        }

        func Damage(change, caused_by) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }
        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");

        // Non-living: the effect halves the damage.
        let crate_id = engine
            .spawn_object(SpawnConfig::new("Actor").with_alive(false))
            .expect("spawn succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(crate_id).expect("object exists");
        engine
            .change_object_damage(idx, 10, C4FX_CALL_DMG_SCRIPT, 3)
            .expect("damage change succeeds");
        assert_eq!(engine.objects[idx].state.damage, 5);
        let calls = call_log.lock().unwrap().clone();
        assert!(
            calls.iter().any(|name| name == "Damage"),
            "the Damage script callback fires (C4Object.cpp:1342)"
        );

        // Living: effects are NOT asked for damage (C4Object.cpp:1333).
        let clonk_id = engine
            .spawn_object(SpawnConfig::new("Actor").with_alive(true))
            .expect("spawn succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(clonk_id).expect("object exists");
        engine
            .change_object_damage(idx, 10, C4FX_CALL_DMG_SCRIPT, 3)
            .expect("damage change succeeds");
        assert_eq!(engine.objects[idx].state.damage, 10);
    }

    #[test]
    fn do_energy_asks_fx_damage_effects_first() {
        // C4Object::DoEnergy asks living things' effects before applying
        // (C4Object.cpp:1355-1359); C4Effect::DoDamage walks the effects in
        // list order — each Fx<Name>Damage return REPLACES the damage
        // (getInt), and a zeroed damage aborts both the walk and DoEnergy
        // (C4Effect.cpp:312-322).
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [
                { op = "add", name = "Armor", priority = 200, interval = 0 },
                { op = "add", name = "Ward", priority = 100, interval = 0 }
            ] };
        }

        global func FxWardDamage(state, effect, damage, damage_type, cause_plr) {
            if (damage_type == 35) {
                return 0;
            }
            return damage;
        }

        global func FxArmorDamage(state, effect, damage, damage_type, cause_plr) {
            // C4Effect::DoDamage reads the callback through getInt(); keep
            // the noncanonical raw Bool payload instead of folding to one.
            return CastBool(damage / 2);
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Actor")
                    .with_alive(true)
                    .with_energy(50_000),
            )
            .expect("spawn succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(id).expect("object exists");

        // List order is ascending priority: Ward (100) runs before Armor
        // (200). A script-cause hit of -10 passes Ward untouched and is
        // -10% = -10000 raw (C4Object.cpp:1347), halved by Armor: -5000.
        engine
            .change_object_energy(idx, -10, C4FX_CALL_ENG_SCRIPT, 3)
            .expect("energy change succeeds");
        assert_eq!(engine.objects[idx].state.energy, 45_000);

        // A fire-cause hit is zeroed by Ward; the zero aborts the walk AND
        // DoEnergy (C4Object.cpp:1358) — Armor never halves, energy keeps.
        engine
            .change_object_energy(idx, -10, C4FX_CALL_ENG_FIRE, 3)
            .expect("energy change succeeds");
        assert_eq!(engine.objects[idx].state.energy, 45_000);
    }

    #[test]
    fn zero_obj_hit_energy_runs_the_head_damage_effect_once_like_cpp() {
        // C4Effect::DoDamage is a do/while (C4Effect.cpp:427-437): an
        // initial zero still visits the list head once. EngObjHit is the
        // documented zero-change DoEnergy case and records its striker
        // before the effect turns that zero into a real raw-energy change.
        let script = r#"#strict
local iDamageCalls, iSeenChange, iSeenCause, iSeenBy;
public func Arm()
{
  iDamageCalls = 0;
  return AddEffect("Amplifier", this(), 100, 0, this());
}
func FxAmplifierDamage(pTarget, iNumber, iChange, iCause, iCausedBy)
{
  iDamageCalls = iDamageCalls + 1;
  iSeenChange = iChange + 1;
  iSeenCause = iCause;
  iSeenBy = iCausedBy;
  return -1000;
}
"#;
        let mut definition =
            Definition::from_script("PING", "Zero-hit amplifier", script)
                .expect("definition compiles");
        definition.set_category(CATEGORY_LIVING);
        definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(144);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("PING")
                    .with_category(CATEGORY_LIVING)
                    .with_alive(true)
                    .with_energy(50_000),
            )
            .expect("object spawns");
        let index = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine
                .call_object_function(index, "Arm", Vec::new())
                .expect("effect installs"),
            Value::Int(1)
        );

        engine
            .change_object_energy(index, 0, C4FX_CALL_ENG_OBJ_HIT, 9)
            .expect("zero-hit DoEnergy succeeds");

        let index = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[index].state.energy, 49_000);
        assert_eq!(engine.objects[index].last_energy_loss_cause, 9);
        let locals = &engine.objects[index].state.local_vars;
        assert_eq!(locals.get("iDamageCalls"), Some(&Value::Int(1)));
        assert_eq!(
            locals.get("iSeenChange"),
            Some(&Value::Int(1)),
            "the callback received C4Script's falsy zero change"
        );
        assert_eq!(
            locals.get("iSeenCause"),
            Some(&Value::Int(C4FX_CALL_ENG_OBJ_HIT))
        );
        assert_eq!(locals.get("iSeenBy"), Some(&Value::Int(9)));
    }

    #[test]
    fn do_energy_clamps_to_physical_energy_ceiling() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Clonk", "Clonk", script).unwrap();
        // DoEnergy bounds energy by GetPhysical()->Energy (C4Object.cpp:1361);
        // 50000 on the 0..C4MaxPhysical scale is 50 percent points.
        definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        let zero_physical_definition =
            Definition::from_script("Crate", "Crate", script).unwrap();

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .register_definition(zero_physical_definition)
            .expect("zero-physical definition registers");

        let clonk_id = engine
            .spawn_object(SpawnConfig::new("Clonk").with_energy(40_000))
            .expect("clonk spawns");
        let clonk_idx = engine.find_object_index(clonk_id).expect("clonk exists");
        engine
            .change_object_energy(clonk_idx, 30, C4FX_CALL_ENG_SCRIPT, -1)
            .expect("energy change succeeds");
        assert_eq!(
            engine.objects[clonk_idx].state.energy, 50_000,
            "gain (+30% = +30000 raw) clamps to GetPhysical()->Energy"
        );

        // C4PhysicalInfo::Default zeroes Energy. BoundBy applies that zero
        // ceiling just like every positive one (C4Object.cpp:1388).
        let crate_id = engine
            .spawn_object(SpawnConfig::new("Crate"))
            .expect("crate spawns");
        let crate_idx = engine.find_object_index(crate_id).expect("crate exists");
        assert_eq!(engine.objects[crate_idx].state.energy, 0);
        engine
            .change_object_energy(crate_idx, 30, C4FX_CALL_ENG_SCRIPT, -1)
            .expect("energy change succeeds");
        assert_eq!(engine.objects[crate_idx].state.energy, 0);
    }

    // Hazard's in-round rule chooser is the ONLY way its NoFriendlyFire rule
    // ever exists: no scenario lists `NOFF` in `[Game] Rules=`, and DefCore
    // MaxUserSelect is parsed but never read (C4Def.cpp:169,297). The chooser
    // (Hazard.c4d/Rules.c4d/Chooser.c4d) identifies each rule purely by its
    // GetDefinition(i, Chooser_Cat) index: OpenRuleMenu hands that index to
    // AddMenuItem as the command Parameter, ChangeRuleConf records it in
    // aRules, and ConfigurationFinished2 re-resolves the same index to
    // CreateObject. So the index has to survive being encoded into the
    // command's source text (C4Script.cpp:1513-1546,1556-1597) and re-parsed
    // by the DirectExec that a menu Enter performs on the menu's COMMAND
    // object rather than on the clonk holding the menu (C4Menu.cpp:498-523;
    // C4ObjectMenu.cpp:505-527). A break anywhere along that chain creates
    // the wrong rule or none, which presents exactly as the reported
    // "Hazard ignores the no-friendly-fire setting".
    #[test]
    fn rule_chooser_creates_the_rule_whose_menu_index_was_toggled() {
        let chooseable = "#strict\npublic func IsChooseable() { return(1); }\n";
        let chooser_script = r#"#strict 2
static const Chooser_Cat = 524384;
local aRules;

func Boot() { aRules = CreateArray(); return 1; }

func OpenRuleMenu(object pClonk)
{
  CreateMenu(GetID(), pClonk);
  for(var i=0, idR, def ; idR = GetDefinition(i, Chooser_Cat) ; i++)
    if(DefinitionCall(idR, "IsChooseable") && !GetLength(FindObjects(Find_ID(idR))))
      {
      // C4MN_Add_ImgObject REJECTS the row unless XPar is a real object
      // (C4Script.cpp:1670-1678), so the chooser draws each rule from a
      // throwaway instance and removes it again.
      def = CreateObject(idR, 0,0, -1);
      AddMenuItem("%s", "ChangeRuleConf", idR, pClonk, 0, i, 0, 4, def);
      RemoveObject(def);
      }
  return 1;
}

func ChangeRuleConf(id dummy, int i)
{
  if(!aRules[i])
    aRules[i] = true;
  else
    aRules[i] = false;
  return 1;
}

func ConfigurationFinished2()
{
  var i = 0;
  for(var check in aRules)
  {
    if(check)
      CreateObject(GetDefinition(i, Chooser_Cat), 10, 10, -1);
    i++;
  }
  return 1;
}

func IndexOfNoff()
{
  for(var i=0 ; i < 20 ; i++)
    if(GetDefinition(i, Chooser_Cat) == NOFF)
      return i;
  return -1;
}

func Pick(object pClonk, int item) { return SelectMenuItem(item, pClonk); }
func RuleCounts() { return [ObjectCount(NOFF), ObjectCount(IGIB)]; }
"#;
        let mut engine = Engine::with_seed(5);
        // Two chooseable rules, so an off-by-one or reordered enumeration
        // resolves to the WRONG rule instead of merely to nothing. Category
        // mirrors NoFriendlyFire.c4d/DefCore.txt: C4D_StaticBack|C4D_Rule.
        for id in ["IGIB", "NOFF"] {
            let mut rule = Definition::from_script(id, id, chooseable).expect("rule compiles");
            rule.set_category(1 | 524_288);
            engine.register_definition(rule).expect("rule registers");
        }
        engine
            .register_script_definition("CHOS", "Chooser", chooser_script)
            .expect("chooser registers");
        engine
            .register_script_definition("CLNK", "Clonk", "#strict\n")
            .expect("clonk registers");

        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        let chooser = engine
            .spawn_object(SpawnConfig::new("CHOS").with_owner(-1))
            .expect("chooser spawns");
        let call = |engine: &mut Engine, name: &str, args: Vec<Value>| {
            let index = engine.find_object_index(chooser).expect("chooser exists");
            engine
                .call_object_function(index, name, args)
                .expect("call succeeds")
        };
        call(&mut engine, "Boot", Vec::new());
        let noff_index = call(&mut engine, "IndexOfNoff", Vec::new());
        assert!(
            matches!(noff_index, Value::Int(index) if index >= 0),
            "NOFF is enumerated under Chooser_Cat, got {noff_index:?}"
        );
        let Value::Int(noff_index) = noff_index else {
            unreachable!("checked above")
        };

        call(
            &mut engine,
            "OpenRuleMenu",
            vec![object_reference_value(clonk)],
        );
        let menu = engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk exists")
            .expect("the rule menu opened on the clonk, not on the chooser");
        assert_eq!(
            menu.command_object,
            Some(chooser),
            "CreateMenu's omitted command object defaults to cthr->Obj — the \
             chooser — so ChangeRuleConf resolves there (C4Script.cpp:1431)"
        );
        let item = menu
            .items
            .iter()
            .position(|item| item.item_id == "NOFF")
            .expect("NOFF has a menu item");
        assert_eq!(
            menu.items[item].command,
            format!("ChangeRuleConf(NOFF,{noff_index})"),
            "the item command carries the definition index as source text"
        );

        call(
            &mut engine,
            "Pick",
            vec![object_reference_value(clonk), Value::Int(item as i32)],
        );
        assert!(engine
            .menu_user_enter(clonk, false)
            .expect("menu enter runs"));
        call(&mut engine, "ConfigurationFinished2", Vec::new());

        assert_eq!(
            call(&mut engine, "RuleCounts", Vec::new()),
            Value::Array(vec![Value::Int(1), Value::Int(0)]),
            "only the toggled rule is created, and ObjectCount sees it the \
             way Hazard's NoFriendlyFire() reads it"
        );
    }
