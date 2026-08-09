// Contiguous slice 6 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    #[test]
    fn non_fixed_definition_seed_expands_through_root_before_originals() {
        fn write_pack(root: &Path, id: &str) {
            let definition = root.join("Objects.c4d").join(format!("{id}.c4d"));
            std::fs::create_dir_all(&definition).expect("definition dir");
            std::fs::write(
                definition.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write defcore");
            write_test_definition_graphics(&definition);
        }

        let dir = tempdir().expect("tempdir");
        let normal_root = dir.path().join("normal");
        let definition_root = dir.path().join("rooted");
        write_pack(&normal_root, "NORM");
        write_pack(&definition_root, "ROOT");
        let scenario_dir = dir.path().join("SeededRoot.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Seeded root\n\n[Definitions]\nLocalOnly=1\nDefinition1=Ignored.c4d\n",
        )
        .expect("write scenario core");

        let scenario = Scenario::load_from_path_with_languages_and_definition_seed_in_root(
            &scenario_dir,
            &FileSystemResolver {
                roots: vec![normal_root.clone()],
            },
            &["US"],
            &["Objects.c4d"],
            &definition_root,
        )
        .expect("seeded rooted scenario loads");
        assert_eq!(
            scenario
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["ROOT", "NORM"]
        );
        assert_eq!(
            scenario.definition_resource_paths(),
            [
                definition_root.join("Objects.c4d"),
                normal_root.join("Objects.c4d"),
            ]
        );
    }

    #[test]
    fn rooted_definition_path_crosses_packed_groups_case_insensitively() {
        let dir = tempdir().expect("tempdir");
        let definition_root = dir.path().join("rooted");
        std::fs::create_dir_all(&definition_root).expect("definition root");

        let rooted_core = b"[DefCore]\nid=PACK\nName=Packed\nCategory=0\n";
        let rooted_script = b"// packed rooted definition\n";
        let rooted_graphics = encode_indexed_bmp(&[&[0x83]]);
        let nested = packed_test_group(&[
            ("DefCore.txt", false, rooted_core.as_slice()),
            ("Script.c", false, rooted_script.as_slice()),
            ("Graphics.bmp", false, rooted_graphics.as_slice()),
        ]);
        let outer = packed_test_group_file(&[("NeStEd.C4D", true, nested.as_slice())]);
        std::fs::write(definition_root.join("PACK.C4D"), outer).expect("write outer packed group");

        let normal_root = dir.path().join("normal");
        let original = normal_root.join("pack.c4d/nested.c4d");
        std::fs::create_dir_all(&original).expect("original group");
        std::fs::write(
            original.join("DefCore.txt"),
            "[DefCore]\nid=ORIG\nName=Original\nCategory=0\n",
        )
        .expect("write original defcore");
        write_test_definition_graphics(&original);

        let scenario_dir = dir.path().join("PackedRoot.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Packed root\n\n[Definitions]\n\
             Definitions=\"pack.c4d\\\\nested.c4d\"\n",
        )
        .expect("write scenario core");

        let scenario = Scenario::load_from_path_with_languages_and_definition_seed_in_root(
            &scenario_dir,
            &FileSystemResolver {
                roots: vec![normal_root],
            },
            &["US"],
            &["IgnoredSeed.c4d"],
            &definition_root,
        )
        .expect("case-insensitive packed root path resolves");
        assert_eq!(
            scenario
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["PACK", "ORIG"]
        );
        assert_eq!(
            scenario.definition_resource_paths()[0],
            definition_root.join("PACK.C4D/NeStEd.C4D")
        );
    }

    /// Minimal legacy scenario fixture: one definition pack plus a
    /// `Scenario.txt` with the given `[Landscape]` section body.
    fn write_legacy_sky_fixture(dir: &Path, landscape_section: &str) -> PathBuf {
        let defs_root = dir.join("Defs.c4d");
        let foo_core = defs_root.join("Foo.c4d");
        std::fs::create_dir_all(&foo_core).expect("definition dir");
        std::fs::write(
            foo_core.join("DefCore.txt"),
            "[DefCore]\nid=FOOO\nName=Foo\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        write_test_definition_graphics(&foo_core);

        let scenario_dir = dir.join("SkyTest.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            format!(
                "[Head]\nTitle=Sky Test\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\n{landscape_section}"
            ),
        )
        .expect("write legacy scenario core");
        scenario_dir
    }

    fn load_legacy_sky(dir: &Path, scenario_dir: &Path) -> Scenario {
        load_legacy_sky_with_seed(dir, scenario_dir, 0)
    }

    fn load_legacy_sky_with_seed(dir: &Path, scenario_dir: &Path, random_seed: u64) -> Scenario {
        let resolver = FileSystemResolver {
            roots: vec![dir.to_path_buf()],
        };
        Scenario::load_from_path_with_seed(scenario_dir, &resolver, random_seed)
            .expect("legacy scenario loads")
    }

    #[test]
    fn legacy_scenario_loads_sky_bitmap_like_c4sky_init() {
        // C4Sky::Init loads the scenario's `Sky` bitmap via LoadAny
        // (C4Sky.cpp:82-84; extension search png/bmp/jpeg/jpg,
        // C4Surface.cpp:855), sets the fade colors to white (C4Sky.cpp:109),
        // tiles tiny surfaces up to 128x128 (SurfaceEnsureSize,
        // C4Sky.cpp:28-52,110-111) and maps SkyScrollMode=2 to
        // ParX=ParY=20 without touching ParallaxMode (C4Sky.cpp:122-124).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "SkyScrollMode=2\n");

        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(
                &[80u8, 120, 200].repeat(16),
                4,
                4,
                ColorType::Rgb8.into(),
            )
            .expect("encode sky jpeg");
        std::fs::write(scenario_dir.join("Sky.jpg"), jpeg).expect("write Sky.jpg");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert!(sky.settings.has_surface);
        assert_eq!((sky.settings.width, sky.settings.height), (128, 128));
        let surface = sky.surface.as_ref().expect("sky surface loaded");
        assert_eq!((surface.width(), surface.height()), (128, 128));
        assert_eq!(sky.settings.fade_top, RgbColor::new(255, 255, 255));
        assert_eq!(sky.settings.fade_bottom, RgbColor::new(255, 255, 255));
        assert_eq!(sky.settings.parallax_x, 20);
        assert_eq!(sky.settings.parallax_y, 20);
        assert_eq!(sky.settings.parallax_mode, SkyParallaxMode::Fixed);
    }

    #[test]
    fn legacy_sky_load_stops_after_an_unreadable_first_extension() {
        // LoadAny picks the first matching extension before decoding it. A
        // matching child/unreadable entry therefore cannot fall through to a
        // valid lower-priority extension (C4Surface.cpp:846-865).
        let dir = tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("Sky.png")).expect("unreadable png entry");
        std::fs::write(dir.path().join("Sky.bmp"), encode_indexed_bmp(&[&[0x83]]))
            .expect("lower-priority bmp");
        let group = Group::open(dir.path()).expect("sky group");

        assert!(load_legacy_sky_surface(&group, "Sky").is_none());
    }

    #[test]
    fn legacy_sky_def_list_uses_seeded_random_and_lookup_order() {
        // C4Sky::Init selects exactly one SkyDef section with the stateless
        // C4Random.h formula, searches the scenario before Graphics.c4g, and
        // does not consume the synchronized Random() ledger (C4Sky.cpp:88-105).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "Sky=A;B;C\n");
        let graphics_dir = dir.path().join("Graphics.c4g");
        std::fs::create_dir_all(&graphics_dir).expect("graphics group");

        let write_png = |path: &Path, pixel: [u8; 4]| {
            use image::ImageEncoder as _;
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&pixel, 1, 1, ColorType::Rgba8.into())
                .expect("encode named sky png");
            std::fs::write(path, png).expect("write named sky png");
        };
        write_png(&scenario_dir.join("A.png"), [1, 2, 3, 255]);
        write_png(&scenario_dir.join("B.png"), [20, 40, 60, 255]);
        write_png(&graphics_dir.join("B.png"), [200, 210, 220, 255]);
        write_png(&graphics_dir.join("C.png"), [70, 80, 90, 255]);

        let independent_index = |seed: u32| {
            let stepped = seed.wrapping_mul(214_013).wrapping_add(2_531_011);
            ((stepped >> 16) % 3) as usize
        };

        assert_eq!(independent_index(7), 1);
        let scenario = load_legacy_sky_with_seed(dir.path(), &scenario_dir, 7);
        let surface = scenario
            .sky()
            .and_then(|sky| sky.surface.as_ref())
            .expect("seed 7 selects scenario B sky");
        assert_eq!(&surface.pixels()[..4], &[20, 40, 60, 255]);

        assert_eq!(independent_index(0), 2);
        let scenario = load_legacy_sky_with_seed(dir.path(), &scenario_dir, 0);
        let surface = scenario
            .sky()
            .and_then(|sky| sky.surface.as_ref())
            .expect("seed 0 selects Graphics.c4g C sky");
        assert_eq!(&surface.pixels()[..4], &[70, 80, 90, 255]);
    }

    #[test]
    fn legacy_scenario_without_sky_bitmap_falls_back_to_palette_fade() {
        // No sky bitmap: C4Sky::Init drops the surface and takes the fade
        // colors from SkyDefFade (C4Sky.cpp:129-134); an all-zero fade
        // selects game palette entries 104 and 123 — RGB(28,64,152) and
        // RGB(192,196,252) after the <<2 load scaling (C4Sky.cpp:56-62,
        // C4GraphicsResource.cpp:183-184, planet/Graphics.c4g/C4.PAL).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "TopOpen=1\n");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert!(!sky.settings.has_surface);
        assert!(sky.surface.is_none());
        assert_eq!(sky.settings.fade_top, RgbColor::new(28, 64, 152));
        assert_eq!(sky.settings.fade_bottom, RgbColor::new(192, 196, 252));
        assert_eq!(sky.settings.parallax_x, 10);
        assert_eq!(sky.settings.parallax_y, 10);
    }

    #[test]
    fn legacy_sky_def_fade_overrides_the_palette_default() {
        // Non-zero SkyDefFade — the `SkyFade` key (C4Scenario.cpp:344) —
        // is two explicit RGB triplets (C4Sky::SetFadePalette,
        // C4Sky.cpp:63-68).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "SkyFade=1,2,3,4,5,6\n");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert_eq!(sky.settings.fade_top, RgbColor::new(1, 2, 3));
        assert_eq!(sky.settings.fade_bottom, RgbColor::new(4, 5, 6));
    }

    #[test]
    fn legacy_sky_fade_channels_wrap_to_low_byte_like_c4rgb() {
        // C4Sky::SetFadePalette passes raw signed SkyFade values to C4RGB,
        // which retains each channel's low byte (C4Sky.cpp:63-68;
        // StdColors.h:52).
        let dir = tempdir().expect("tempdir");
        let scenario_dir =
            write_legacy_sky_fixture(dir.path(), "SkyFade=-1,256,511,513,-258,1024\n");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert_eq!(sky.settings.fade_top, RgbColor::new(255, 0, 255));
        assert_eq!(sky.settings.fade_bottom, RgbColor::new(1, 254, 0));
    }

    #[test]
    fn legacy_sky_scroll_mode_wind_maps_to_wind_parallax() {
        // SkyScrollMode=1: wind-driven xdir plus ParY=20 (C4Sky.cpp:118-121).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "SkyScrollMode=1\n");

        let mut png = Vec::new();
        {
            use image::ImageEncoder as _;
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(
                    &[10u8, 20, 30, 255],
                    1,
                    1,
                    ColorType::Rgba8.into(),
                )
                .expect("encode sky png");
        }
        std::fs::write(scenario_dir.join("Sky.png"), png).expect("write Sky.png");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert_eq!(sky.settings.parallax_mode, SkyParallaxMode::Wind);
        assert_eq!(sky.settings.parallax_x, 10);
        assert_eq!(sky.settings.parallax_y, 20);
        // 1x1 tile enlarged to the 128x128 minimum (C4Sky.cpp:110-111).
        assert_eq!((sky.settings.width, sky.settings.height), (128, 128));
    }

    #[test]
    fn legacy_skipdefs_excludes_specified_definitions() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let foo_core = defs_root.join("Foo.c4d");
        std::fs::create_dir_all(&foo_core).expect("foo definition dir");
        std::fs::write(
            foo_core.join("DefCore.txt"),
            "[DefCore]\nid=FOOO\nName=Foo\nCategory=0\nCrewMember=0\n",
        )
        .expect("write foo defcore");
        std::fs::write(foo_core.join("Script.c"), "// foo script\n").expect("write foo script");
        std::fs::write(
            foo_core.join("ActMap.txt"),
            "[Action]\nDefault=MissingButSkipped\n",
        )
        .expect("write malformed skipped actmap");

        let bar_core = defs_root.join("Bar.c4d");
        std::fs::create_dir_all(&bar_core).expect("bar definition dir");
        std::fs::write(
            bar_core.join("DefCore.txt"),
            "[DefCore]\nid=BARR\nName=Bar\nCategory=0\nCrewMember=0\n",
        )
        .expect("write bar defcore");
        std::fs::write(bar_core.join("Script.c"), "// bar script\n").expect("write bar script");
        write_test_definition_graphics(&bar_core);

        let scenario_dir = dir.path().join("SkipDefsScenario.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=SkipDefs\n\n[Definitions]\nDefinition1=Defs.c4d\nSkipDefs=FOOO\n\n[Player1]\nCrew=BARR\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "global func Initialize(state, random) { return 0; }\n",
        )
        .expect("write scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let ids: Vec<String> = scenario
            .definitions
            .iter()
            .map(|def| def.id.clone())
            .collect();
        assert!(
            ids.iter().any(|id| id == "BARR"),
            "expected non-skipped definition to be present"
        );
        assert!(
            !ids.iter().any(|id| id == "FOOO"),
            "expected skipped definition to be filtered out"
        );
    }

    #[test]
    fn parse_c4fixed_reads_the_cpp_serialization_formats() {
        // CompileFunc(C4Fixed&) (Fixed.h:247-266): prefix 'f' = the int32
        // is FLOAT BITS run through FLOAT_TO_FIXED (ftofix truncates);
        // 'F' or no prefix = the raw fixed-point value. GoldRush saves
        // YDir=f1067030938 (bits of 1.2f) on its hanging stalactites.
        assert_eq!(
            parse_c4fixed("f1067030938").expect("parses").val(),
            78643, // trunc(1.2 * 65536)
        );
        assert_eq!(
            parse_c4fixed("f-1063256064").expect("parses").val(),
            -5 * 65536, // bits of -5.0f as negative int32
        );
        assert_eq!(parse_c4fixed("F78643").expect("parses").val(), 78643);
        assert_eq!(parse_c4fixed("123").expect("parses").val(), 123);
        assert_eq!(
            parse_c4fixed("x78643")
                .expect("unknown alphabetic format stays raw")
                .val(),
            78643
        );
        assert!(
            parse_c4fixed("ff1067030938").is_err(),
            "Character consumes exactly one format byte before the integer"
        );
    }

    #[test]
    fn legacy_unsigned_object_words_accept_signed_cpp_spellings() {
        let mut records = parse_legacy_objects(
            "[Object]\nid=GOOD\nNumber=1\nOCF=-1\nColorDw=-2\nColorMod=-3\nBlitMode=-4\n",
        )
        .expect("signed uint32 spellings preserve their bit patterns");
        let record = records.remove(0);
        assert_eq!(record.ocf, Some((-1_i32) as u32));
        assert_eq!(record.color, Some((-2_i32) as u32));
        assert_eq!(record.color_modulation, Some((-3_i32) as u32));
        assert_eq!(record.blit_mode, Some((-4_i32) as u32));
    }

    #[test]
    fn legacy_command_versions_follow_cpp_field_layouts() {
        // Shipped 4.9 scenarios use `$1`: it already carries BaseMode, but
        // retains the pre-v2 compatibility rule that textual `0` is empty.
        let version_one =
            parse_legacy_object_command("$1,MoveTo,i517,177,0,0,0,0,1,0,0,0,0,0,1,0", 7)
                .expect("shipped $1 command parses");
        assert_eq!(version_one.name, "MoveTo");
        assert_eq!(
            version_one.tx,
            SerializedC4Value::Value(clonk_script::Value::Int(517))
        );
        assert_eq!(version_one.ty, 177);
        assert_eq!(version_one.evaluated, 1);
        assert_eq!(version_one.base_mode, 1);
        assert!(version_one.text.is_empty());

        let unversioned =
            parse_legacy_object_command("MoveTo,i5,6,7,8,9,10,11,12,13,14,15,16,0", 8)
                .expect("unversioned command parses");
        assert_eq!(unversioned.base_mode, 0);
        assert!(unversioned.text.is_empty());

        let explicit_zero =
            parse_legacy_object_command("$0,MoveTo,i5,6,7,8,9,10,11,12,13,14,15,16,0", 9)
                .expect("explicit version zero command parses");
        assert_eq!(explicit_zero.base_mode, 0);
        assert!(explicit_zero.text.is_empty());

        let version_two =
            parse_legacy_object_command("$2,MoveTo,i5,6,7,8,9,10,11,12,13,14,15,16,0,0", 10)
                .expect("current command parses");
        assert_eq!(version_two.base_mode, 0);
        assert_eq!(version_two.text, "0");
    }

    #[test]
    fn legacy_object_property_and_section_names_are_exact_case() {
        let mut wrong_case = parse_legacy_objects(
            "[Object]\nid=GOOD\nNumber=1\nx=9\ny=10\nxdir=F1\nYDIR=F2\nFixx=F3\nfixY=F4\nFIXR=F5\nRdir=F6\nmobile=true\nrotation=7\n",
        )
        .expect("wrong-case properties are ignored like unexpected INI names");
        let wrong_case = wrong_case.remove(0);
        assert_eq!(wrong_case.x, None);
        assert_eq!(wrong_case.y, None);
        assert_eq!(wrong_case.xdir, None);
        assert_eq!(wrong_case.ydir, None);
        assert_eq!(wrong_case.fix_x, None);
        assert_eq!(wrong_case.fix_y, None);
        assert_eq!(wrong_case.fix_r, None);
        assert_eq!(wrong_case.rdir, None);
        assert_eq!(wrong_case.mobile, None);
        assert_eq!(wrong_case.rotation, None);

        let mut exact = parse_legacy_objects(
            "[Object]\nid=GOOD\nNumber=1\nX=9\nY=10\nXDir=F1\nYDir=F2\nFixX=F3\nFixY=F4\nFixR=F5\nRDir=F6\nMobile=true\nRotation=7\n",
        )
        .expect("canonical motion properties parse");
        let exact = exact.remove(0);
        assert_eq!(exact.x, Some(9));
        assert_eq!(exact.y, Some(10));
        assert_eq!(exact.xdir.map(crate::math::C4Fixed::val), Some(1));
        assert_eq!(exact.ydir.map(crate::math::C4Fixed::val), Some(2));
        assert_eq!(exact.fix_x.map(crate::math::C4Fixed::val), Some(3));
        assert_eq!(exact.fix_y.map(crate::math::C4Fixed::val), Some(4));
        assert_eq!(exact.fix_r.map(crate::math::C4Fixed::val), Some(5));
        assert_eq!(exact.rdir.map(crate::math::C4Fixed::val), Some(6));
        assert_eq!(exact.mobile, Some(true));
        assert_eq!(exact.rotation, Some(7));

        let wrong_object = parse_legacy_objects(
            "[object]\nid=GOOD\nNumber=1\n[Object]\nid=GOOD\nNumber=2\nowner=7\nEnergy=4\nENERGY=9\n[Commands]\nCommand01=$2,Wait,A0,0,0,0,0,0,0,0,0,0,0,0,0,\n",
        )
        .expect("unknown-case naming environments are ignored");
        assert_eq!(wrong_object.len(), 1);
        assert_eq!(wrong_object[0].number, Some(2));
        assert_eq!(wrong_object[0].owner, None);
        assert_eq!(wrong_object[0].energy, Some(4));
        assert!(wrong_object[0].commands.is_empty());
    }

    #[test]
    fn legacy_object_rct_all_values_preserve_slashes_and_trailing_spaces() {
        let mut records = parse_legacy_objects(concat!(
            "[Object]\n",
            "id=GOOD\n",
            "Number=1\n",
            "Info=Captain // veteran  \n",
            "[Commands]\n",
            "Command1=$2,Call,A0,0,0,0,0,0,0,0,0,0,0,0,0,Foo//Bar  \n",
        ))
        .expect("RCT_All values parse");
        let record = records.remove(0);
        assert_eq!(record.info_name.as_deref(), Some("Captain // veteran  "));
        assert_eq!(
            record.commands.get(&1).map(|command| command.text.as_str()),
            Some("Foo//Bar  ")
        );
    }

    #[test]
    fn legacy_object_components_preserve_bare_zero_and_negative_counts() {
        assert_eq!(
            parse_legacy_object_components("WOOD;ZERO=0;NEGA=-3", 7)
                .expect("component list parses"),
            vec![
                ("WOOD".to_owned(), 0),
                ("ZERO".to_owned(), 0),
                ("NEGA".to_owned(), -3),
            ]
        );
    }

    #[test]
    fn legacy_objects_restore_temporary_physicals_without_parent_field_leakage() {
        let source = concat!(
            "[Object]\n",
            "id=GOOD\n",
            "Number=1\n",
            "Energy=9001\n",
            "Breath=9002\n",
            "Damage=9003\n",
            "PhysicalTemporary=1\n",
            // The first matching naming node wins, including the flag.
            "PhysicalTemporary=false\n",
            "[Physical]\n",
            "Energy=101\n",
            "Breath=102\n",
            "Walk=103\n",
            "Jump=104\n",
            "Scale=105\n",
            "Hangle=106\n",
            "Dig=107\n",
            "Swim=108\n",
            "Throw=109\n",
            "Push=110\n",
            "Fight=111\n",
            "Magic=112\n",
            "Float=113\n",
            "CanScale=114\n",
            "CanHangle=115\n",
            "CanDig=116\n",
            "CanConstruct=117\n",
            "CanChop=118\n",
            "CanFly=119\n",
            "CorrosionResist=120\n",
            "BreatheWater=121\n",
            "Energy=999999\n",
            "Changes=Walk=10,Energy=20,Walk=30\n",
            "Changes=Energy=999\n",
            // Not a C4PhysicalInfo naming: it must not reach the parent.
            "Damage=9999\n",
            // FollowName consumes only the first sibling Physical section.
            "[Physical]\n",
            "Energy=not-an-integer\n",
        );
        let mut records = parse_legacy_objects(source).expect("temporary physical object parses");
        assert_eq!(records.len(), 1);
        let record = records.remove(0);
        let expected = crate::PhysicalInfo {
            energy: 101,
            breath: 102,
            walk: 103,
            jump: 104,
            scale: 105,
            hangle: 106,
            dig: 107,
            swim: 108,
            throw: 109,
            push: 110,
            fight: 111,
            magic: 112,
            float: 113,
            can_scale: 114,
            can_hangle: 115,
            can_dig: 116,
            can_construct: 117,
            can_chop: 118,
            can_fly: 119,
            corrosion_resist: 120,
            breathe_water: 121,
        };
        assert_eq!(
            (record.energy, record.breath, record.damage),
            (Some(9001), Some(9002), Some(9003))
        );
        assert_eq!(record.temporary_physical, Some(expected));
        assert_eq!(
            record.physical_changes,
            vec![
                ("Walk".to_string(), 10),
                ("Energy".to_string(), 20),
                ("Walk".to_string(), 30),
            ]
        );

        let definition_ids = HashSet::from(["GOOD"]);
        let object_numbers = HashSet::from([1_u64]);
        let string_registrations = clonk_script::new_string_registrations();
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &string_registrations,
        };
        let config = record
            .into_spawn(&definition_ids, &resolution)
            .expect("record converts")
            .expect("known definition spawns")
            .config;
        assert_eq!(
            (config.energy, config.breath, config.damage),
            (Some(9001), Some(9002), Some(9003))
        );
        assert_eq!(config.temporary_physical, Some(expected));

        let mut definition =
            Definition::from_script("GOOD", "Good", "").expect("definition compiles");
        definition.set_physical(crate::PhysicalInfo {
            energy: 77_001,
            breath: 77_002,
            walk: 77_003,
            ..crate::PhysicalInfo::default()
        });
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");
        let object_id = engine.spawn_object(config).expect("loaded object spawns");
        let object_index = engine
            .find_object_index(object_id)
            .expect("loaded object remains live");
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.id == object_id)
            .expect("loaded object is snapshotted");
        assert_eq!(
            (object.energy, object.breath, object.damage),
            (9001, 9002, 9003)
        );
        assert_eq!(object.temporary_physical, Some(expected));
        assert_eq!(
            object.physical_changes,
            vec![
                ("Walk".to_string(), 10),
                ("Energy".to_string(), 20),
                ("Walk".to_string(), 30),
            ]
        );
        assert_eq!(engine.object_physical(object_index), expected);
    }

    #[test]
    fn legacy_objects_ignore_disabled_and_out_of_scope_physical_sections() {
        let source = concat!(
            "[Object]\n",
            "id=GOOD\n",
            "Number=2\n",
            "Energy=2001\n",
            "Breath=2002\n",
            "PhysicalTemporary=0\n",
            "PhysicalTemporary=true\n",
            "[Physical]\n",
            "Energy=not-an-integer\n",
            "[Object]\n",
            "id=GOOD\n",
            "Number=3\n",
            "Energy=3001\n",
            "Breath=3002\n",
            "[Physical]\n",
            "Changes=NoSuchPhysical=7\n",
            "[Object]\n",
            "id=GOOD\n",
            "Number=4\n",
            "Energy=4001\n",
            "Breath=4002\n",
            "PhysicalTemporary=true\n",
            // A child section is not the FollowName sibling.
            "  [Physical]\n",
            "  Energy=still-not-an-integer\n",
            // This same-level sibling consumes the position Physical needed.
            "[Unrelated]\n",
            "[Physical]\n",
            "Changes=also-not-compiled\n",
            "[Object]\n",
            "id=GOOD\n",
            "Number=5\n",
            "PhysicalTemporary=true\n",
            "[Object]\n",
            "id=GOOD\n",
            "Number=6\n",
            // Boolean does not skip whitespace after `=`; the default adaptor
            // turns this malformed first value into false.
            "PhysicalTemporary= true\n",
            "PhysicalTemporary=1\n",
            "[Physical]\n",
            "Energy=not-an-integer\n",
            "[Object]\n",
            "id=GOOD\n",
            "Number=7\n",
            // Scenario.txt's extended boolean words are malformed here and
            // likewise default to false instead of failing the object load.
            "PhysicalTemporary=yes\n",
            "[Physical]\n",
            "Changes=not-compiled\n",
            "[Object]\n",
            "id=GOOD\n",
            "Number=8\n",
            // Outdenting a value leaves the child naming and resumes Object.
            "  [Child]\n",
            "  PhysicalTemporary=false\n",
            // Spaces are part of the native naming key; this is unused.
            "PhysicalTemporary =false\n",
            "PhysicalTemporary=true\n",
            // A malformed would-be section creates no naming node and cannot
            // consume Object's next root sibling.
            "[123]\n",
            // The child section did not consume Object's root-level sibling.
            "[Physical]\tignored trailing text\n",
            "Walk =999999\n",
            "Walk=808\n",
            "  [Nested]\n",
            "  Breath=999999\n",
            // The same indentation rule resumes the Physical naming.
            "Breath=809\n",
        );
        let records = parse_legacy_objects(source).expect("unused physical sections are ignored");
        let definition_ids = HashSet::from(["GOOD"]);
        let object_numbers = HashSet::from([2_u64, 3, 4, 5, 6, 7, 8]);
        let string_registrations = clonk_script::new_string_registrations();
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &string_registrations,
        };
        let configs = records
            .into_iter()
            .map(|record| {
                record
                    .into_spawn(&definition_ids, &resolution)
                    .expect("record converts")
                    .expect("known definition spawns")
                    .config
            })
            .collect::<Vec<_>>();
        assert_eq!(configs.len(), 7);
        assert_eq!(
            (configs[0].energy, configs[0].breath),
            (Some(2001), Some(2002))
        );
        assert_eq!(
            (configs[1].energy, configs[1].breath),
            (Some(3001), Some(3002))
        );
        assert_eq!(
            (configs[2].energy, configs[2].breath),
            (Some(4001), Some(4002))
        );
        assert_eq!(configs[0].temporary_physical, None);
        assert_eq!(configs[1].temporary_physical, None);
        assert_eq!(
            configs[2].temporary_physical,
            Some(crate::PhysicalInfo::default())
        );
        assert_eq!(
            configs[3].temporary_physical,
            Some(crate::PhysicalInfo::default())
        );
        assert_eq!(configs[4].temporary_physical, None);
        assert_eq!(configs[5].temporary_physical, None);
        assert_eq!(
            configs[6].temporary_physical,
            Some(crate::PhysicalInfo {
                walk: 808,
                breath: 809,
                ..crate::PhysicalInfo::default()
            })
        );
        assert!(configs
            .iter()
            .all(|config| config.physical_changes.is_empty()));
    }

    #[test]
    fn legacy_objects_default_malformed_temporary_physical_values_like_cpp() {
        let source = concat!(
            // Raw indentation may decrease between root siblings; their
            // common name-tree parent, not equal columns, controls FollowName.
            "  [Object]\n",
            "  id=GOOD\n",
            "  Number=9\n",
            "  PhysicalTemporary=true\n",
            "[Physical]\n",
            // The default adaptor consumes this first naming as zero, so the
            // later duplicate cannot replace it.
            "Energy=not-an-integer\n",
            "Energy=123\n",
            // The STL adaptor retains its valid prefix and stops on the first
            // invalid element. The duplicate Changes naming is then unused.
            "Changes=Walk=10,NoSuchPhysical=20,Energy=30\n",
            "Changes=Energy=40\n",
        );
        let mut records = parse_legacy_objects(source).expect("default adaptors do not abort");
        assert_eq!(records.len(), 1);
        let record = records.remove(0);
        assert_eq!(
            record.temporary_physical,
            Some(crate::PhysicalInfo::default())
        );
        assert_eq!(record.physical_changes, vec![("Walk".to_string(), 10)]);
    }

    #[test]
    fn live_objects_txt_parser_retains_full_object_and_nested_state() {
        let source = concat!(
            "[Object]\n",
            "id=GOOD\n",
            "Number=1\n",
            "Info=Captain\n",
            "Contained=0\n",
            "ActionTarget1=999\n",
            "ActionTarget2=1000000042\n",
            "Layer=-7\n",
            "LastEngLossPlr=8\n",
            "LastSolidAtchFrame=77\n",
            "NoCollectDelay=9\n",
            "Base=2\n",
            "OwnMass=11\n",
            "Mass=29\n",
            "Damage=23\n",
            "Breath=41\n",
            "FirePhase=3\n",
            "AttachX=12\n",
            "AttachY=-4\n",
            "AttachVtx=2\n",
            "OnFire=true\n",
            "PhysicalTemporary=true\n",
            "OCF=31\n",
            "PlrViewRange=444\n",
            "CrewDisabled=true\n",
            "Graphics=GOOD::Alternate\n",
            "DrawTransform=1,0,0,0,1,0,1\n",
            "Effects=Fire(4,100,8,1,0,NONE)[3;i3,i7,S0]\n",
            "GfxOverlay=7,GOOD::Alternate,6,,132,4,(1,0,0,0,1,0,1),11259375,2\n",
            "Contents=3;2;3\n",
            "[Physical]\n",
            "Energy=50\n",
            "Breath=60\n",
            "Walk=70\n",
            "Changes=Walk=10,Energy=20,Walk=30\n",
            // This is not an Object-section property and must not overwrite
            // the parent object's Damage=23.
            "Damage=999\n",
            "[Commands]\n",
            "Command1=$2,Call,a[3;i7,S0,I0],0,2,0,23,5,1,0,0,2,3,0,1,raw,text\n",
            // Native stores this signed word verbatim. A negative interval
            // skips the lifetime countdown but does not reject the command.
            "Command2=$2,MoveTo,i12,-3,2,0,7,-4,0,1,0,0,1,0,0,\n",
            // C4Command::Tx is a tagged C4Value for every command. Runtime
            // command code observes a nonnumeric value as zero, but the
            // original value remains part of the exact save projection.
            "Command3=$2,Wait,a[2;i4,i5],0,0,0,0,0,0,0,0,0,0,0,0,\n",
            // Likewise, a stray nested key cannot leak into [Object].
            "Damage=1000\n",
            "[Object]\n",
            "id=GOOD\n",
            "Number=2\n",
        );

        let mut records = parse_legacy_objects(source).expect("live Objects.txt parses");
        assert_eq!(records.len(), 2);
        let first = records.remove(0);
        assert_eq!(first.info_name.as_deref(), Some("Captain"));
        assert_eq!(first.damage, Some(23));
        assert_eq!(first.contents, vec![3, 2, 3]);
        assert_eq!(
            first.physical_changes,
            vec![
                ("Walk".to_string(), 10),
                ("Energy".to_string(), 20),
                ("Walk".to_string(), 30),
            ]
        );
        let physical = first
            .temporary_physical
            .as_ref()
            .expect("[Physical] was captured");
        assert_eq!(
            (physical.energy, physical.breath, physical.walk),
            (50, 60, 70)
        );
        assert_eq!(first.commands.len(), 3);
        assert_eq!(first.commands.get(&1).expect("Command1").text, "raw,text");
        assert_eq!(first.draw_transform, Some(crate::DrawTransform::identity()));
        assert!(matches!(
            first
                .effects
                .as_ref()
                .and_then(|effects| effects.first())
                .and_then(|effect| effect.vars.get(2)),
            Some(SerializedC4Value::StringTableIndex(0))
        ));

        let definition_ids = HashSet::from(["GOOD"]);
        let object_numbers = HashSet::from([1_u64, 2]);
        let string_registrations = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&string_registrations, 0, "saved text");
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &string_registrations,
        };
        let spawn = first
            .into_spawn(&definition_ids, &resolution)
            .expect("record converts")
            .expect("known definition spawns");
        assert_eq!(spawn.info_name.as_deref(), Some("Captain"));
        assert_eq!(spawn.contents_handles, ["3", "2", "3"]);
        let config = spawn.config;
        assert_eq!(
            config.compiler_cache,
            crate::ObjectCompilerCache {
                info: "Captain".to_string(),
                contained: 0,
                action_target1: 999,
                action_target2: 1_000_000_042,
                layer: -7,
            },
            "raw compiler caches survive parsing independently of denumeration",
        );
        assert_eq!(config.last_energy_loss_cause, Some(8));
        assert_eq!(config.last_attach_movement_frame, Some(77));
        assert_eq!(config.no_collect_delay, Some(9));
        assert_eq!(config.base, Some(2));
        assert_eq!(config.own_mass, Some(11));
        assert_eq!(config.compiled_mass, Some(29));
        assert_eq!(config.damage, Some(23));
        assert_eq!(config.breath, Some(41));
        assert_eq!(config.fire_phase, Some(3));
        assert_eq!(config.on_fire, Some(true));
        assert_eq!(config.fire_caused_by, Some(7));
        assert_eq!(config.compiled_ocf, Some(31));
        assert_eq!(config.plr_view_range, Some(444));
        assert_eq!(config.crew_disabled, Some(true));
        assert_eq!(
            config.shape_attach,
            Some(crate::ShapeAttachRecord {
                mat_valid: false,
                mat_vehicle: false,
                x: 12,
                y: -4,
                vtx: 2,
            })
        );
        assert_eq!(
            config
                .base_graphics
                .as_ref()
                .and_then(|graphics| graphics.graphics_name.as_deref()),
            Some("Alternate")
        );
        assert_eq!(
            config.draw_transform,
            Some(crate::DrawTransform::identity())
        );
        let overlay = config.graphics_overlays.first().expect("overlay restored");
        assert_eq!(overlay.id, 7);
        assert_eq!(overlay.overlay_object, Some(ObjectId::new(2)));
        assert_eq!(overlay.transform, Some(crate::DrawTransform::identity()));
        assert_eq!(
            config.effects[0].vars[2],
            EffectVarValue::String("saved text".to_string().into())
        );
        let temporary = config
            .temporary_physical
            .as_ref()
            .expect("temporary physical restored");
        assert_eq!(
            (temporary.energy, temporary.breath, temporary.walk),
            (50, 60, 70)
        );
        assert_eq!(config.physical_changes.len(), 3);
        let command_stack = config.command_stack.as_ref().expect("commands restored");
        assert_eq!(command_stack.command_names(), ["Call", "MoveTo", "Wait"]);
        let commands = command_stack.command_views();
        assert_eq!(commands[0].target, Some(ObjectId::new(2)));
        assert_eq!(
            commands[0].tx_value,
            Some(clonk_script::Value::Array(vec![
                clonk_script::Value::Int(7),
                clonk_script::Value::String("saved text".to_string().into()),
                clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(0)),
            ]))
        );
        assert_eq!(commands[0].legacy_data, Some(23));
        assert_eq!(
            commands[0].data,
            crate::command::CommandData::Text("raw,text".to_string())
        );
        assert_eq!(commands[1].tx, Some(12));
        assert_eq!(commands[1].ty, Some(-3));
        assert_eq!(command_stack.legacy_save_commands()[1].update_interval, -4);
        assert_eq!(
            commands[2].tx_value,
            Some(clonk_script::Value::Array(vec![
                clonk_script::Value::Int(4),
                clonk_script::Value::Int(5),
            ]))
        );

        let defaulted = records
            .remove(0)
            .into_spawn(&definition_ids, &resolution)
            .expect("minimal record converts")
            .expect("known definition spawns")
            .config;
        assert!(defaulted.loaded);
        assert!(defaulted.native_compiled_object_defaults);
        assert_eq!(defaulted.energy, None);
        assert_eq!(defaulted.breath, None);
        assert_eq!(defaulted.category, None);
        assert_eq!(defaulted.compiled_mass, None);
    }

    #[test]
    fn legacy_objects_restore_ordered_command_stack() {
        let dir = tempdir().expect("tempdir");
        let definition = dir.path().join("Defs.c4d/Command.c4d");
        std::fs::create_dir_all(&definition).expect("definition directory");
        std::fs::write(
            definition.join("DefCore.txt"),
            "[DefCore]\nid=CMND\nName=Command object\nCategory=17\n",
        )
        .expect("definition core");
        std::fs::write(definition.join("Script.c"), "#strict\n").expect("definition script");
        write_test_definition_graphics(&definition);

        let scenario_dir = dir.path().join("Commands.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario directory");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            concat!(
                "[Head]\nTitle=Commands\nSaveGame=1\nNoInitialize=1\n\n",
                "[Definitions]\nDefinition1=Defs.c4d\n",
            ),
        )
        .expect("scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            concat!(
                "[Object]\n",
                "id=CMND\nNumber=100\nStatus=1\nCategory=17\n",
                "[Commands]\n",
                "Command1=$2,Call,a[5;i7,O101,O102,O999,I0],-3,101,102,23,-4,-2,3,0,-5,-6,-7,9,raw,text // exact  \n",
                "Command2=$2,MoveTo,O102,44,999,101,-55,-66,0,-9,2,7,8,9,3,move,text  \n",
                // Native stops at the first missing number, even if a later
                // naming exists in the same [Commands] section.
                "Command4=$2,Wait,i0,0,0,0,0,0,0,0,0,0,0,0,0,ignored\n",
                "\n[Object]\n",
                "id=CMND\nNumber=101\nStatus=1\nCategory=17\n",
                "\n[Object]\n",
                "id=CMND\nNumber=102\nStatus=2\nCategory=17\n",
            ),
        )
        .expect("Objects.txt");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(&scenario_dir, &resolver)
            .expect("command scenario loads");
        let mut engine = Engine::with_seed(221);
        scenario
            .apply(&mut engine)
            .expect("command scenario applies");

        let assert_stack = |engine: &Engine| {
            let actor_index = engine
                .find_object_index(ObjectId::new(100))
                .expect("command actor remains loaded");
            let saved = engine.objects[actor_index].commands.legacy_save_commands();
            assert_eq!(
                saved.len(),
                2,
                "Command4 after the numbering gap is ignored"
            );
            assert_eq!(
                saved
                    .iter()
                    .map(|command| command.view.name.as_str())
                    .collect::<Vec<_>>(),
                ["Call", "MoveTo"],
                "Command1 is the executable head and Command2 its tail",
            );

            let call = &saved[0];
            assert_eq!(call.view.target, Some(ObjectId::new(101)));
            assert_eq!(
                call.view.target2,
                Some(ObjectId::new(102)),
                "inactive objects remain valid command targets",
            );
            assert_eq!(
                call.view.tx_value,
                Some(clonk_script::Value::Array(vec![
                    clonk_script::Value::Int(7),
                    clonk_script::Value::Object(101),
                    clonk_script::Value::Object(102),
                    clonk_script::Value::Nil,
                    clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(0)),
                ])),
                "tagged Tx recursively denumerates active, inactive, and missing objects",
            );
            assert_eq!(call.view.ty, Some(-3));
            assert_eq!(call.view.legacy_data, Some(23));
            assert_eq!(call.update_interval, -4);
            assert_eq!(
                (call.evaluated, call.path_checked, call.finished),
                (-2, 3, 0)
            );
            assert_eq!((call.failures, call.retries, call.permit), (-5, -6, -7));
            assert_eq!(call.base_mode, 9, "unknown int32 BaseMode is retained");
            assert_eq!(call.text, "raw,text // exact  ");

            let move_to = &saved[1];
            assert_eq!(
                move_to.view.target, None,
                "missing Target denumerates to null"
            );
            assert_eq!(move_to.view.target2, Some(ObjectId::new(101)));
            assert_eq!(
                move_to.view.tx_value,
                Some(clonk_script::Value::Object(102)),
            );
            assert_eq!(move_to.view.ty, Some(44));
            assert_eq!(move_to.view.data, crate::command::CommandData::Integer(-55));
            assert_eq!(move_to.update_interval, -66);
            assert_eq!(
                (move_to.evaluated, move_to.path_checked, move_to.finished),
                (0, -9, 2),
            );
            assert_eq!(
                (move_to.failures, move_to.retries, move_to.permit),
                (7, 8, 9)
            );
            assert_eq!(move_to.base_mode, 3);
            assert_eq!(
                move_to.text, "move,text  ",
                "non-Call Text is not discarded"
            );
        };

        assert_eq!(
            engine
                .object_snapshot(ObjectId::new(102))
                .expect("inactive target remains addressable")
                .status,
            ObjectStatus::Inactive,
        );
        assert_stack(&engine);

        let encoded = serde_json::to_string(&engine.capture_state())
            .expect("command-bearing engine state serializes");
        let restored = serde_json::from_str(&encoded).expect("engine state deserializes");
        engine
            .restore_state(&restored)
            .expect("engine state restores");
        assert_stack(&engine);

        // A section's Objects.Load resolves against objects retained from the
        // previous section too, including inactive objects outside this file.
        let section_dir = dir.path().join("Retained.c4g");
        std::fs::create_dir_all(&section_dir).expect("section directory");
        std::fs::write(
            section_dir.join("Objects.txt"),
            concat!(
                "[Object]\n",
                "id=CMND\nNumber=200\nStatus=1\nCategory=17\n",
                "[Commands]\n",
                "Command1=$2,Call,O102,0,102,999,0,0,0,0,0,0,0,0,0,Retained\n",
            ),
        )
        .expect("section Objects.txt");
        let group = Group::open(&section_dir).expect("section group opens");
        let spawns = collect_legacy_objects_with_definition_ids(
            &group,
            &HashSet::from(["CMND"]),
            &clonk_script::new_string_registrations(),
            &HashSet::from([102]),
        )
        .expect("section objects compile");
        let retained = spawns[0]
            .config
            .command_stack
            .as_ref()
            .expect("section command stack")
            .legacy_save_commands();
        assert_eq!(retained[0].view.target, Some(ObjectId::new(102)));
        assert_eq!(retained[0].view.target2, None);
        assert_eq!(
            retained[0].view.tx_value,
            Some(clonk_script::Value::Object(102)),
        );
    }

    #[test]
    fn legacy_objects_restore_remaining_runtime_fields_and_info_links() {
        let dir = tempdir().expect("tempdir");
        let defs_root = dir.path().join("Defs.c4d");
        for (folder, id, crew_member, category) in
            [("Clonk.c4d", "CLNK", 1, 17), ("Thing.c4d", "THNG", 0, 17)]
        {
            let definition = defs_root.join(folder);
            std::fs::create_dir_all(&definition).expect("definition directory");
            std::fs::write(
                definition.join("DefCore.txt"),
                format!(
                    "[DefCore]\nid={id}\nName={id}\nCategory={category}\nCrewMember={crew_member}\nMass=50\n"
                ),
            )
            .expect("definition core");
            std::fs::write(definition.join("Script.c"), "#strict\n").expect("definition script");
            write_test_definition_graphics(&definition);
        }

        let scenario_dir = dir.path().join("RuntimeFields.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario directory");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Runtime Fields\nSaveGame=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            concat!(
                "[Object]\n",
                "id=CLNK\nNumber=100\nStatus=1\nOwner=0\nController=9\n",
                "Category=17\nSize=100000\nAlive=true\nInfo=Captain\n",
                "LastEngLossPlr=-7\nMotionX=-13\nMotionY=17\n",
                "LastSolidAtchFrame=-23\nNoCollectDelay=-29\nBase=0\n",
                "OwnMass=-31\nMass=37\nDamage=41\nEnergy=47\nBreath=43\n",
                "FirePhase=5\nOnFire=true\nPlrViewRange=444\nCrewDisabled=true\n",
                "\n[Object]\n",
                "id=THNG\nNumber=101\nStatus=1\nOwner=0\nCategory=17\n",
                "Size=100000\nDamage=53\nEnergy=59\nPlrViewRange=222\n",
            ),
        )
        .expect("Objects.txt");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(&scenario_dir, &resolver)
            .expect("savegame scenario loads");
        let mut engine = Engine::with_seed(73);
        scenario
            .apply_before_players(&mut engine)
            .expect("objects load before restored players");
        engine
            .register_player(crate::PlayerConfig::new(0, "Restored"))
            .expect("restored player registers");

        let selected_physical = crate::PhysicalInfo {
            energy: 77_000,
            breath: 66_000,
            walk: 55_000,
            can_scale: 1,
            ..crate::PhysicalInfo::default()
        };
        let crew_info = |name: &str, experience: i32, physical: crate::PhysicalInfo| {
            crate::player_file::CrewInfo {
                id: "CLNK".to_string(),
                name: name.to_string(),
                core: Default::default(),
                rank_name: "Clonk".to_string(),
                experience,
                physical,
                portraits: Default::default(),
                ..Default::default()
            }
        };
        engine.crew_rosters.insert(
            0,
            vec![
                crew_info("Higher experience fallback", 999, Default::default()),
                crew_info("Captain", 1, selected_physical),
            ],
        );
        engine.crew_info_order.insert(0, vec![0, 1]);
        engine
            .finalize_restored_players(false)
            .expect("saved Info links after player restoration");

        let crew_id = ObjectId::new(100);
        let crew_index = engine.find_object_index(crew_id).expect("loaded crew");
        let crew = &engine.objects[crew_index];
        assert_eq!(crew.compiler_cache.info, "Captain");
        assert_eq!((crew.motion_x, crew.motion_y), (-13, 17));
        assert_eq!(crew.last_attach_movement_frame, -23);
        assert_eq!(crew.last_energy_loss_cause, -7);
        assert_eq!(crew.state.no_collect_delay, -29);
        assert_eq!(crew.state.base, 0);
        assert_eq!(crew.state.own_mass, -31);
        assert_eq!(crew.compiled_mass, Some(37));
        assert_eq!(crew.state.damage, 41);
        assert_eq!(crew.state.energy, 47);
        assert_eq!(crew.state.breath, 43);
        assert_eq!(crew.state.fire_phase, 5);
        assert!(crew.state.on_fire);
        assert_eq!(crew.state.plr_view_range, 444);
        assert!(crew.state.crew_disabled);
        assert_eq!(crew.state.controller, 0, "AssignInfo controls the crew");
        assert_eq!(crew.state.info_physical, Some(selected_physical));
        assert_ne!(crew.state.ocf & crate::ocf::ON_FIRE, 0);
        assert_eq!(crew.state.effects.len(), 1, "bare OnFire gets Fire");
        assert_eq!(crew.state.effects[0].name, crate::C4FX_FIRE);
        assert_eq!(crew.state.effects[0].number, 1);
        assert_eq!(
            crew.state.effects[0].priority, 0,
            "fDoCalls=false leaves the compatibility node dead"
        );
        assert_eq!(
            crew.state.effects[0].interval,
            crate::C4FX_FIRE_TIMER_INTERVAL
        );
        assert_eq!(crew.state.effects[0].timer, 0);
        assert!(crew.state.effects[0].start_dispatched);

        let info = engine.crew_object_info(crew_id).expect("live crew info");
        assert_eq!(info.name, "Captain");
        let link = engine.crew_info_links[&crew_id];
        assert_eq!((link.player_id, link.roster_index), (0, 1));
        assert!(!engine.crew_rosters[&0][0].in_action);
        assert!(engine.crew_rosters[&0][1].in_action);
        assert_eq!(engine.player(0).expect("player").crew(), [crew_id]);

        let non_info_id = ObjectId::new(101);
        let view_objects = engine.player(0).expect("player").fow_view_objects();
        assert!(view_objects.contains(&crew_id), "Info object restores FoW");
        assert!(
            view_objects.contains(&non_info_id),
            "non-Info object restores FoW"
        );
        let before = (
            engine.objects[crew_index].state.on_fire,
            engine.objects[crew_index].state.fire_phase,
            engine.objects[crew_index].state.construction,
            engine.objects[crew_index].state.damage,
            engine.objects[crew_index].state.energy,
        );
        let rng_before = engine.rng.clone();

        engine.tick_without_snapshot().expect("first tick succeeds");

        let crew = &engine.objects[crew_index];
        assert!(crew.state.effects.is_empty(), "dead fallback is unlinked");
        assert_eq!(
            (
                crew.state.on_fire,
                crew.state.fire_phase,
                crew.state.construction,
                crew.state.damage,
                crew.state.energy,
            ),
            before,
            "a priority-zero Fire node performs no fire execution"
        );
        assert_eq!(engine.rng, rng_before, "dead Fire consumes no random draw");
    }

    #[test]
    fn restored_object_links_clear_invalid_owners_even_when_every_player_restore_failed() {
        // InitGameFinal always validates object Owner/Base/Controller after
        // InitPlayers, including when RecreatePlayers joined nobody
        // (C4Game.cpp:2724-2729,3157-3165).
        let mut engine = Engine::new();
        engine
            .register_definition(
                Definition::from_script("THNG", "Thing", "").expect("compile definition"),
            )
            .expect("register definition");
        let object = engine
            .spawn_object(SpawnConfig::new("THNG").with_owner(7))
            .expect("spawn saved-owner fixture");
        let object_index = engine.find_object_index(object).expect("fixture object");
        assert_eq!(engine.objects[object_index].state.owner, 7);

        engine
            .finalize_restored_players(false)
            .expect("finalize empty restore result");

        assert_eq!(engine.objects[object_index].state.owner, crate::OWNER_NONE);
    }

    #[test]
    fn unassociated_joined_savegame_player_removes_only_its_raw_player_objects() {
        // RestoreSavegameInfos removes each joined restore row without a
        // current SavegamePlayer association in packet/row order; the saved
        // GameNumber selects FLAG or raw C4D_CrewMember objects by live Owner
        // (C4PlayerInfo.cpp:1422-1439,1610-1633; C4PlayerList.cpp:208-216;
        // C4Object.cpp:6267-6291).
        let mut engine = Engine::new();
        for id in ["FLAG", "CREW", "KEEP"] {
            engine
                .register_definition(
                    Definition::from_script(id, id, "").expect("compile definition"),
                )
                .expect("register definition");
        }
        let flag = engine
            .spawn_object(SpawnConfig::new("FLAG").with_owner(7))
            .expect("spawn unassociated flag");
        let crew = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(7)
                    .with_category(1 << 18),
            )
            .expect("spawn unassociated raw-category crew");
        let ordinary = engine
            .spawn_object(SpawnConfig::new("KEEP").with_owner(7))
            .expect("spawn unassociated ordinary object");
        let associated = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(8)
                    .with_category(1 << 18),
            )
            .expect("spawn associated crew");
        let ownerless = engine
            .spawn_object(SpawnConfig::new("FLAG"))
            .expect("spawn ownerless flag");

        let mut current = crate::ControlPlayerInfoRegistry::default();
        current.apply(crate::PlayerInfoControlData {
            client_id: 4,
            players: vec![crate::ControlPlayerInfoEntry {
                id: 91,
                savegame_player: 8,
                ..Default::default()
            }],
            ..Default::default()
        });
        let restore = [
            crate::ControlPlayerInfoEntry {
                id: 7,
                game_number: 7,
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                ..Default::default()
            },
            crate::ControlPlayerInfoEntry {
                id: 8,
                game_number: 8,
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                ..Default::default()
            },
            crate::ControlPlayerInfoEntry {
                id: 9,
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                ..Default::default()
            },
        ];

        engine
            .remove_unassociated_savegame_player_objects(&current, &restore)
            .expect("remove unassociated saved-player objects");

        assert_eq!(
            engine
                .object_snapshot(flag)
                .expect("retained tombstone")
                .status,
            crate::ObjectStatus::Deleted
        );
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("retained tombstone")
                .status,
            crate::ObjectStatus::Deleted
        );
        assert_eq!(
            engine
                .object_snapshot(ordinary)
                .expect("ordinary object")
                .status,
            crate::ObjectStatus::Normal
        );
        assert_eq!(
            engine
                .object_snapshot(associated)
                .expect("associated crew")
                .status,
            crate::ObjectStatus::Normal
        );
        assert_eq!(
            engine
                .object_snapshot(ownerless)
                .expect("ownerless flag")
                .status,
            crate::ObjectStatus::Normal
        );
    }

    #[test]
    fn legacy_object_name_decodes_cpp_escaped_strings() {
        // StdCompilerINIRead::ReadEscapedChar (StdCompiler.cpp:1006-1062).
        assert_eq!(
            parse_legacy_object_name(r#""Script \"Wipf\" \\ \101\x42""#, 7)
                .expect("escaped name parses")
                .as_deref(),
            Some("Script \"Wipf\" \\ AB")
        );
    }

    #[test]
    fn loads_legacy_objects_txt_spawns_initial_objects() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let box_core = defs_root.join("Box.c4d");
        std::fs::create_dir_all(&box_core).expect("box definition dir");
        std::fs::write(
            box_core.join("DefCore.txt"),
            "[DefCore]\nid=BOX1\nName=Box\nCategory=0\nCrewMember=0\n",
        )
        .expect("write box defcore");
        std::fs::write(box_core.join("Script.c"), "// box script\n").expect("box script");
        write_test_definition_graphics(&box_core);

        let gem_core = defs_root.join("Gem.c4d");
        std::fs::create_dir_all(&gem_core).expect("gem definition dir");
        std::fs::write(
            gem_core.join("DefCore.txt"),
            "[DefCore]\nid=GEM1\nName=Gem\nCategory=0\nCrewMember=0\n",
        )
        .expect("write gem defcore");
        std::fs::write(gem_core.join("Script.c"), "// gem script\n").expect("gem script");
        write_test_definition_graphics(&gem_core);

        let scenario_dir = dir.path().join("LegacyObjects.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Objects\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            // XDir/YDir are float-bit C4Fixed like real saves write them
            // (Fixed.h:247-266): f-1063256064 = -5.0, f1077936128 = 3.0.
            // BOX1 must not be StaticBack: C4GameObjects::Load deliberately
            // zeroes loaded velocity for that category after denumeration.
            "[Object]\nid=BOX1\nNumber=100\nName=Scroll: Alchemist's bag\nStatus=1\nCategory=16\nOwner=1\nController=2\nX=10\nY=20\nXDir=F45875\nYDir=F-78643\nContents=101\n\n[Object]\nid=GEM1\nNumber=101\nName=\"ScriptWipf\"\nStatus=1\nCategory=0\nLayer=100\nVisibility=13\nBlitMode=132\nColorDw=1122867\nColorMod=1146447479\nPicture=-5,6,70,80\nX=30\nY=40\nXDir=f-1063256064\nYDir=f1077936128\nEnergy=77\nNeedEnergy=1\nSelected=1\nMagicEnergy=192000\nAlive=false\nDir=1\nComDir=3\nAction=Idle\nActionTime=6\nPhase=2\nActionData=5\nActionTarget1=100\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");

        assert_eq!(scenario.initial_spawns.len(), 2);

        let first = &scenario.initial_spawns[0];
        assert_eq!(first.handle.as_deref(), Some("100"));
        assert!(first.container_handle.is_none());
        assert_eq!(first.config.definition_id, "BOX1");
        assert_eq!(first.config.owner, 1);
        // Controller compiles verbatim (C4Object.cpp:2739).
        assert_eq!(first.config.controller, Some(2));
        assert_eq!(first.config.position, Vector2::new(10, 20));
        let first_fixed_velocity = first.config.fixed_velocity.expect("loaded fixed velocity");
        assert_eq!(first_fixed_velocity.x.val(), 45_875);
        assert_eq!(first_fixed_velocity.y.val(), -78_643);
        assert_eq!(first.config.id, Some(ObjectId::new(100)));
        assert_eq!(
            first.config.custom_name.as_deref(),
            Some("Scroll: Alchemist's bag")
        );

        let second = &scenario.initial_spawns[1];
        assert_eq!(second.handle.as_deref(), Some("101"));
        assert_eq!(second.container_handle.as_deref(), Some("100"));
        assert_eq!(second.config.definition_id, "GEM1");
        assert_eq!(second.config.custom_name.as_deref(), Some("ScriptWipf"));
        assert_eq!(second.config.layer, Some(ObjectId::new(100)));
        assert_eq!(second.config.visibility, Some(13));
        assert_eq!(second.config.blit_mode, Some(132));
        assert_eq!(second.config.color, Some(0x0011_2233));
        assert_eq!(second.config.color_modulation, Some(0x4455_6677));
        assert_eq!(
            second.config.picture_rect,
            Some(DefinitionRect::new(-5, 6, 70, 80))
        );
        assert_eq!(second.config.position, Vector2::new(30, 40));
        assert_eq!(second.config.velocity, Vector2::new(-5, 3));
        assert_eq!(second.config.energy, Some(77));
        assert_eq!(second.config.need_energy, Some(true));
        assert_eq!(second.config.selected, Some(true));
        // MagicEnergy compiles verbatim (C4Object.cpp:2768) — the
        // Drachenfels wizards carry six-digit stores.
        assert_eq!(second.config.magic_energy, Some(192_000));
        assert_eq!(second.config.alive, Some(false));
        assert_eq!(second.config.category, Some(0));
        assert_eq!(second.config.direction, Direction::Right);
        assert_eq!(second.config.command_direction, CommandDirection::Right);
        let action = second.config.action.as_ref().expect("action state present");
        assert_eq!(action.name, "Idle");
        // ActionTime= is Action.Time (C4Object.cpp:2745), not the
        // intra-phase PhaseDelay counter.
        assert_eq!(action.time, 6);
        assert_eq!(action.ticks, 0);
        assert_eq!(action.phase, 2);
        assert_eq!(action.data, 5);
        assert_eq!(action.target, Some(ObjectId::new(100)));

        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");
        assert_eq!(
            engine.debug_exec_order(),
            [ObjectId::new(101), ObjectId::new(100)],
            "FixObjectOrder category-sorts the native execution order"
        );

        let box_snapshot = engine
            .object_snapshot(ObjectId::new(100))
            .expect("box object");
        assert_eq!(box_snapshot.definition_id, "BOX1");
        assert_eq!(box_snapshot.owner, 1);
        assert_eq!(box_snapshot.controller, 2, "loaded Controller= sticks");
        assert_eq!(box_snapshot.position, Vector2::new(10, 20));
        let box_index = engine
            .find_object_index(ObjectId::new(100))
            .expect("box object index");
        assert_eq!(engine.objects[box_index].fixed_velocity.x.val(), 45_875);
        assert_eq!(engine.objects[box_index].fixed_velocity.y.val(), -78_643);
        assert_eq!(
            box_snapshot.custom_name.as_deref(),
            Some("Scroll: Alchemist's bag")
        );

        let gem_snapshot = engine
            .object_snapshot(ObjectId::new(101))
            .expect("gem object");
        let gem_index = engine
            .find_object_index(ObjectId::new(101))
            .expect("gem object index");
        assert_eq!(
            engine.objects[gem_index].compiler_cache,
            crate::ObjectCompilerCache {
                info: String::new(),
                // Contents repair resolved the live container to #100, but
                // no Contained naming was compiled for this row.
                contained: 0,
                action_target1: 100,
                action_target2: 0,
                layer: 100,
            },
            "pointer denumeration must not overwrite raw compiler words",
        );
        let captured = engine.capture_state();
        assert_eq!(
            captured.objects[gem_index].compiler_cache, engine.objects[gem_index].compiler_cache,
            "exact engine snapshots preserve stale compiler caches",
        );
        assert_eq!(gem_snapshot.definition_id, "GEM1");
        assert_eq!(gem_snapshot.custom_name.as_deref(), Some("ScriptWipf"));
        assert_eq!(gem_snapshot.layer, Some(ObjectId::new(100)));
        assert_eq!(gem_snapshot.visibility, 13);
        assert_eq!(gem_snapshot.blit_mode, 132);
        assert_eq!(gem_snapshot.color, 0x0011_2233);
        assert_eq!(gem_snapshot.color_modulation, 0x4455_6677);
        assert_eq!(
            gem_snapshot.picture_rect,
            DefinitionRect::new(-5, 6, 70, 80)
        );
        assert_eq!(gem_snapshot.position, Vector2::new(30, 40));
        // Loads denumerate Contained without the Enter transfer
        // (C4Object.cpp:1582 never runs): compile default NO_OWNER.
        assert_eq!(gem_snapshot.controller, crate::OWNER_NONE);
        // FixObjectOrder repairs the missing Category=0 sort bit to
        // C4D_StaticBack after the load-time speed check. Game-start
        // SyncClearance subsequently zeroes its speed (C4GameObjects.cpp:
        // 640-663,773-830; C4Object.cpp:3830-3850).
        assert_eq!(gem_snapshot.category, crate::CATEGORY_STATIC_BACK);
        assert_eq!(gem_snapshot.velocity, Vector2::ZERO);
        assert_eq!(gem_snapshot.energy, 77);
        assert!(gem_snapshot.need_energy);
        assert!(gem_snapshot.selected);
        assert_eq!(gem_snapshot.magic_energy, 192_000);
        assert!(!gem_snapshot.alive);
        assert_eq!(gem_snapshot.container, Some(ObjectId::new(100)));
        assert_eq!(gem_snapshot.direction, Direction::Right);
        assert_eq!(gem_snapshot.command_direction, CommandDirection::Right);
        assert_eq!(gem_snapshot.action.name, "Idle");
        // Load enters ActIdle, then restores saved Time/Phase/PhaseDelay for
        // any successful SetActionByName call (C4Object.cpp:2862-2876).
        assert_eq!(gem_snapshot.action.ticks, 0);
        assert_eq!(gem_snapshot.action.phase, 2);
        assert_eq!(gem_snapshot.action.time, 6);
        assert_eq!(gem_snapshot.action.data, 5);
        assert_eq!(gem_snapshot.action.target, Some(ObjectId::new(100)));

        let saved = engine
            .serialize_live_c4_save_with_policy(
                crate::LiveC4SaveSpec {
                    title: "Loaded velocity",
                    definition_modules: &[],
                    definition_executable_path: "",
                    definition_path: "",
                    origin: "LegacyObjects.c4s",
                    music_enabled: false,
                    copied_material_group_is_file: false,
                    title_component: crate::LiveC4ComponentHost::Unmodified,
                    info_component: crate::LiveC4ComponentHost::Unmodified,
                    script_component: crate::LiveC4ComponentHost::Unmodified,
                },
                crate::LiveC4SavePolicy::Scenario {
                    force_exact_landscape: false,
                },
            )
            .expect("loaded object state serializes");
        let objects_txt = String::from_utf8(saved.objects_txt).unwrap();
        assert!(objects_txt.contains("XDir=F45875\r\n"));
        assert!(objects_txt.contains("YDir=F-78643\r\n"));

        engine
            .restore_state(&captured)
            .expect("captured legacy state restores");
        let restored_gem_index = engine
            .find_object_index(ObjectId::new(101))
            .expect("restored gem object index");
        assert_eq!(
            engine.objects[restored_gem_index].compiler_cache,
            captured.objects[gem_index].compiler_cache,
            "restore_state preserves raw compiler-cache words",
        );
    }

    #[test]
    fn scenario_initialize_finds_and_removes_placed_objects_like_cpp() {
        // GoldRush's DoInitialize culls placed editor leftovers:
        //   if(FindObject(_ETG)) RemoveObject(FindObject(_ETG));
        // (Goldrush.c4s/Script.c:28) and re-runs the placed cannon's
        // Initialize via FindObject(CCAN). The scenario script must see
        // Objects.txt placements through FindObject.
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let box_core = defs_root.join("Box.c4d");
        std::fs::create_dir_all(&box_core).expect("box definition dir");
        std::fs::write(
            box_core.join("DefCore.txt"),
            "[DefCore]\nid=BOX1\nName=Box\nCategory=0\nCrewMember=0\n",
        )
        .expect("write box defcore");
        std::fs::write(box_core.join("Script.c"), "// box\n").expect("box script");
        write_test_definition_graphics(&box_core);

        let scenario_dir = dir.path().join("LegacyObjects.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Objects\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=BOX1\nNumber=100\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nprotected func InitializePlayer(int iPlr) {\n\
                 if(FindObject(BOX1)) RemoveObject(FindObject(BOX1));\n\
                 return 1;\n\
             }\n",
        )
        .expect("write scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");
        join_test_player(&mut engine);

        // AssignRemoval clears Status immediately (C4Object.cpp); the
        // carcass is purged at frame end.
        let count = engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| {
                object.definition_id == "BOX1" && object.status != ObjectStatus::Deleted
            })
            .count();
        assert_eq!(
            count, 0,
            "the scenario script's FindObject saw the placed object and removed it"
        );
    }

    #[test]
    fn objects_txt_placements_do_not_fire_construction_callbacks_like_cpp() {
        // C4GameObjects::Load (C4GameObjects.cpp:535-618) only compiles the
        // entries and denumerates pointers — Construction/Initialize fire
        // for NEW objects only (C4Object::Init). GoldRush depends on this:
        // its placed Cauldrons would otherwise create fresh CampFires and
        // its placed Bubbles would Remove() themselves at load.
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let box_core = defs_root.join("Box.c4d");
        std::fs::create_dir_all(&box_core).expect("box definition dir");
        std::fs::write(
            box_core.join("DefCore.txt"),
            "[DefCore]\nid=BOX1\nName=Box\nCategory=0\nCrewMember=0\n",
        )
        .expect("write box defcore");
        std::fs::write(
            box_core.join("Script.c"),
            "#strict\nlocal iMark;\n\
             protected func Construction() { iMark = 1; }\n\
             protected func Initialize() { iMark = 2; CreateObject(GEM1, 5, 5, -1); }\n",
        )
        .expect("box script");
        write_test_definition_graphics(&box_core);

        let gem_core = defs_root.join("Gem.c4d");
        std::fs::create_dir_all(&gem_core).expect("gem definition dir");
        std::fs::write(
            gem_core.join("DefCore.txt"),
            "[DefCore]\nid=GEM1\nName=Gem\nCategory=0\nCrewMember=0\n",
        )
        .expect("write gem defcore");
        std::fs::write(gem_core.join("Script.c"), "// gem script\n").expect("gem script");
        write_test_definition_graphics(&gem_core);

        let scenario_dir = dir.path().join("LegacyObjects.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Objects\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=BOX1\nNumber=100\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");

        let snapshot = engine.snapshot();
        let placed = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "BOX1")
            .expect("placed object exists");
        assert!(
            matches!(
                placed.local_vars.get("iMark"),
                None | Some(&clonk_script::Value::Nil)
            ),
            "neither Construction nor Initialize ran for the loaded object \
             (got {:?})",
            placed.local_vars.get("iMark")
        );
        assert!(
            !snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "GEM1"),
            "Initialize side effects (CreateObject) must not happen at load"
        );
    }

    /// A minimal uncompressed bottom-up 8-bit BMP from top-down rows.
    fn encode_indexed_bmp(rows: &[&[u8]]) -> Vec<u8> {
        let height = rows.len() as u32;
        let width = rows[0].len() as u32;
        let stride = ((width as usize) + 3) & !3;
        let data_offset = 14 + 40 + 256 * 4;
        let file_size = data_offset + stride * height as usize;
        let mut bytes = Vec::with_capacity(file_size);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(file_size as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(data_offset as u32).to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&(width as i32).to_le_bytes());
        bytes.extend_from_slice(&(height as i32).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        for _ in 0..4 {
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        bytes.extend_from_slice(&256u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.resize(data_offset, 0);
        for row in rows.iter().rev() {
            bytes.extend_from_slice(row);
            bytes.resize(bytes.len() + (stride - row.len()), 0);
        }
        bytes
    }

    fn write_test_texture(group: &Path, name: &str) {
        std::fs::write(
            group.join(format!("{name}.bmp")),
            encode_indexed_bmp(&[&[0u8]]),
        )
        .expect("write test texture");
    }

    fn build_material_enumeration_classifier(
        mat_map: Option<&[u8]>,
    ) -> Result<MapPixelClassifier, ScenarioError> {
        let dir = tempdir().expect("tempdir");
        let materials = dir.path().join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "# dynamic slots only\n")
            .expect("write texmap");
        for (name, density) in [("A", 60), ("B", 70), ("C", 80)] {
            std::fs::write(
                materials.join(format!("{name}.c4m")),
                format!("[Material]\nName={name}\nDensity={density}\nTextureOverlay=Smooth\n"),
            )
            .expect("write material");
        }
        write_test_texture(&materials, "Smooth");
        if let Some(mat_map) = mat_map {
            std::fs::write(dir.path().join("MatMap.txt"), mat_map).expect("write MatMap");
        }

        let group = Group::open(dir.path()).expect("scenario group opens");
        let resolver = FileSystemResolver { roots: Vec::new() };
        build_map_pixel_classifier(&group, &resolver)?.ok_or_else(|| {
            ScenarioError::InvalidLandscape("material classifier was not built".to_string())
        })
    }

    #[test]
    fn in_liquid_is_the_cached_object_flag_like_cpp() {
        // C4Object::InLiquid is a CACHED flag: loaded from Objects.txt
        // (default false, C4Object.cpp:2775), updated only inside movement
        // (DoMovement, C4Movement.cpp:443-460) — FnInLiquid reads the flag,
        // never the landscape (C4Script.cpp:1864-1868). A freshly loaded
        // object in water therefore reads InLiquid()==false until its
        // first movement frame, and a stale loaded flag on dry land clears
        // on the first frame.
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(
            good.join("Script.c"),
            "#strict\nlocal iWet;\npublic func Probe() { iWet = InLiquid(); return 1; }\n",
        )
        .expect("write script");
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Liquid.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Liquid\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 30, 30, 0],
                &[0, 20, 20, 0],
                &[30, 20, 20, 0],
                &[30, 30, 30, 0],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(
            materials.join("TexMap.txt"),
            "20=Water-Liquid\n30=Earth-Smooth\n",
        )
        .expect("write texmap");
        std::fs::write(
            materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        )
        .expect("write water");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");
        write_test_texture(&materials, "Liquid");
        write_test_texture(&materials, "Smooth");
        // A sits in the cave water without the flag; B sits in dry air
        // above column 0 with a stale InLiquid=1. Category 16 (C4D_Object):
        // ExecMovement skips C4D_StaticBack objects entirely
        // (C4Movement.cpp:564), so static placements would keep their
        // loaded flag forever.
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=80\nStatus=1\nCategory=16\nX=15\nY=15\n\n\
             [Object]\nid=GOOD\nNumber=81\nStatus=1\nCategory=16\nX=5\nY=5\nInLiquid=1\n",
        )
        .expect("write objects");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nfunc Initialize() {\n\
                 var pWet;\n\
                 while(pWet = FindObject(GOOD, 0,0,0,0, 0, 0, 0, 0, pWet)) pWet->Probe();\n\
                 return 1;\n\
             }\n",
        )
        .expect("write scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let flag = |engine: &Engine, number: u64| {
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.id == ObjectId::new(number))
                .map(|object| object.in_liquid)
                .expect("object exists")
        };
        let probed = |engine: &Engine, number: u64| {
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.id == ObjectId::new(number))
                .and_then(|object| object.local_vars.get("iWet").cloned())
                .expect("probe ran")
        };

        assert!(!flag(&engine, 80), "loaded default is false even in water");
        assert!(flag(&engine, 81), "loaded InLiquid=1 sticks until movement");
        assert_eq!(
            probed(&engine, 80),
            clonk_script::Value::Bool(false),
            "InLiquid() reads the stale flag, not the landscape"
        );
        assert_eq!(
            probed(&engine, 81),
            clonk_script::Value::Bool(true),
            "InLiquid() reads the stale loaded flag on dry land too"
        );

        // Loaded placements rest with Mobile=false (C4Object.cpp:2772), so
        // DoMovement — and with it the InLiquid update — never runs until
        // the Tick10 gravity mobilization (C4Movement.cpp:576-587): frames
        // 1-9 keep the stale flags, frame 10 re-mobilizes with zeroed dirs,
        // and frame 11 runs the first DoMovement that refreshes the flag.
        for _ in 0..9 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert!(
            !flag(&engine, 80),
            "immobile objects keep the stale flag (C4Movement.cpp:567)"
        );
        assert!(flag(&engine, 81), "stale flag survives while demobilized");
        engine
            .tick_without_snapshot()
            .expect("mobilization tick succeeds");
        engine
            .tick_without_snapshot()
            .expect("first movement tick succeeds");
        assert!(
            flag(&engine, 80),
            "movement sets the flag in liquid (C4Movement.cpp:443-460)"
        );
        assert!(
            !flag(&engine, 81),
            "movement clears the stale flag on dry land"
        );
    }

    #[test]
    fn nested_script_global_compatibility_sections_stage_for_restore() {
        let data = crate::parse_initial_network_game_data(
            b"[Script]\r\nGo=true\r\n  [Globals]\r\n  0=17\r\n  2=b1\r\n  [GlobalNamed]\r\n  saved=23\r\n",
        );
        let runtime =
            InitialNetworkRuntimeState::parse(&data).expect("nested compatibility spelling stages");
        let string_registrations = clonk_script::new_string_registrations();
        let (globals, effects) =
            runtime.resolve_post_object_state(&HashSet::new(), &string_registrations);
        assert!(effects.is_empty());
        assert_eq!(
            globals.numbered.get(&0),
            Some(&clonk_script::Value::Int(17))
        );
        assert_eq!(globals.numbered.get(&1), Some(&clonk_script::Value::Nil));
        assert_eq!(
            globals.numbered.get(&2),
            Some(&clonk_script::Value::Bool(true))
        );
        assert_eq!(
            globals.named.get("saved"),
            Some(&clonk_script::Value::Int(23))
        );
    }

    #[test]
    fn compiled_sky_runs_before_bitmap_scroll_mode_like_cpp() {
        let data = crate::parse_initial_network_game_data(
            b"[Sky]\r\n\r\n[Effects]\r\n\r\n[Scoreboard]\r\n",
        );
        let mut runtime =
            InitialNetworkRuntimeState::parse(&data).expect("header-only compiler blocks stage");
        assert!(runtime.global_effects.is_empty());
        assert_eq!(
            (
                runtime.scoreboard.row_count(),
                runtime.scoreboard.column_count()
            ),
            (0, 0)
        );

        let mut scenario_settings = SkySettings::default().with_surface(128, 64);
        scenario_settings.parallax_mode = SkyParallaxMode::Wind;
        scenario_settings.parallax_x = 37;
        scenario_settings.parallax_y = 41;
        scenario_settings.modulation = Some(0x1234_5678);
        scenario_settings.back_color = Some(0x1122_3344);
        scenario_settings.back_color_raw = 0x1122_3344;
        let mut engine = Engine::with_seed(0);
        engine.set_sky(scenario_settings.clone());
        let frame = runtime
            .sky
            .take()
            .expect("explicit empty Sky remains present")
            .into_frame(scenario_settings, true, 1);
        engine.apply_initial_network_sky_frame(&frame);

        let restored = engine
            .sky
            .as_ref()
            .expect("sky remains initialized")
            .snapshot();
        assert!(
            restored.settings.has_surface,
            "scenario surface is retained"
        );
        assert_eq!(restored.fixed, Some([0, 0, 0, 0]));
        assert_eq!(restored.settings.parallax_mode, SkyParallaxMode::Wind);
        assert_eq!(
            (restored.settings.parallax_x, restored.settings.parallax_y),
            (10, 20),
            "bitmap SkyScrollMode runs after the compiled member defaults"
        );
        assert_eq!(restored.settings.modulation, Some(0x00ff_ffff));
        assert_eq!(restored.settings.back_color_raw, 0);
        assert_eq!(restored.settings.back_color, None);

        let data = crate::parse_initial_network_game_data(
            b"[Sky]\r\nX=65536\r\nY=-65536\r\nXDir=32768\r\nYDir=-32768\r\nModulation=305419896\r\nParX=37\r\nParY=41\r\nParMode=1\r\nBackClr=287454020\r\nBackClrEnabled=true\r\n",
        );
        let mut runtime = InitialNetworkRuntimeState::parse(&data).expect("compiled sky stages");
        let scenario_settings = SkySettings::default().with_surface(128, 64);
        let fresh = runtime
            .sky
            .take()
            .expect("compiled Sky remains present")
            .into_frame(scenario_settings, false, 2);
        assert_eq!(fresh.fixed, Some([0, 0, 0, 0]));
        assert_eq!(fresh.settings.parallax_mode, SkyParallaxMode::Fixed);
        assert_eq!(
            (fresh.settings.parallax_x, fresh.settings.parallax_y),
            (20, 20),
            "fresh Sky::Init resets runtime parallax before mode 2 applies"
        );
        assert_eq!(fresh.settings.modulation, Some(0x1234_5678));
        assert_eq!(fresh.settings.back_color, Some(0x1122_3344));
    }

    #[test]
    fn initial_runtime_denumeration_matches_any_maps_and_effect_targets() {
        let objects = HashSet::from([7_u64, 1_001_000_001_u64]);
        let string_registrations = clonk_script::new_string_registrations();
        let resolution = SerializedC4ValueResolution {
            object_numbers: &objects,
            string_registrations: &string_registrations,
        };

        assert_eq!(
            parse_serialized_c4value("b7", 1)
                .expect("raw C4V_Bool parses")
                .resolve(&resolution),
            clonk_script::Value::RawBool(7),
            "C4Value bool parsing retains the Data.Int payload"
        );

        assert_eq!(
            parse_serialized_c4value("A1000000007", 1)
                .expect("legacy any parses")
                .resolve(&resolution),
            clonk_script::Value::Object(7),
            "C4V_Any words in the legacy pointer range denumerate"
        );
        assert_eq!(
            parse_serialized_c4value("A1000000009", 1)
                .expect("missing legacy any parses")
                .resolve(&resolution),
            clonk_script::Value::Int(1_000_000_009),
            "a missing C4V_Any pointer is guessed back to int"
        );
        assert_eq!(
            parse_serialized_c4value("A1001000001", 1)
                .expect("raw any parses")
                .resolve(&resolution),
            clonk_script::Value::Int(1_001_000_001),
            "words above C4EnumPointer2 are never shifted"
        );
        let packed_id = i32::from_le_bytes(*b"TEST");
        assert_eq!(
            parse_serialized_c4value(&format!("A{packed_id}"), 1)
                .expect("packed any ID parses")
                .resolve(&resolution),
            clonk_script::Value::C4Id("TEST".to_string()),
            "GuessType recognizes packed IDs before its integer fallback"
        );
        let old_slots =
            parse_local_slots("1000000007,17", 1).expect("old pre-size C4ValueList parses");
        assert_eq!(old_slots.len(), 10);
        assert_eq!(
            old_slots
                .into_iter()
                .take(2)
                .map(|value| value.resolve(&resolution))
                .collect::<Vec<_>>(),
            vec![clonk_script::Value::Object(7), clonk_script::Value::Int(17)],
            "old slot zero and untyped following values retain C4V_Any semantics"
        );

        let resolved = parse_serialized_c4value("m[4;i1=O9;O9=i2;i3=a[1;O9];i4=A1000000007]", 1)
            .expect("map parses")
            .resolve(&resolution);
        let clonk_script::Value::Proplist(resolved) = resolved else {
            panic!("expected resolved map");
        };
        assert_eq!(resolved.len(), 2);
        assert_eq!(
            resolved.get_key(&clonk_script::Value::Int(3)),
            Some(&clonk_script::Value::Array(vec![clonk_script::Value::Nil])),
            "nested missing objects become nil without removing the parent entry"
        );
        assert_eq!(
            resolved.get_key(&clonk_script::Value::Int(4)),
            Some(&clonk_script::Value::Object(7))
        );

        assert_eq!(
            parse_serialized_c4value("a[1;i3,O9]", 1)
                .expect("array ignores extra elements")
                .resolve(&resolution),
            clonk_script::Value::Array(vec![clonk_script::Value::Int(3)])
        );
        let extra_map = parse_serialized_c4value("m[1;i5=i6;broken]", 1)
            .expect("map ignores entries after its declared count")
            .resolve(&resolution);
        let clonk_script::Value::Proplist(extra_map) = extra_map else {
            panic!("expected one-entry map");
        };
        assert_eq!(extra_map.len(), 1);
        assert_eq!(
            extra_map.get_key(&clonk_script::Value::Int(5)),
            Some(&clonk_script::Value::Int(6))
        );

        let effect = |command_target| SerializedEffectState {
            number: 1,
            name: "Probe".to_string(),
            priority: 1,
            interval: 1,
            timer: 0,
            command_target,
            command_id: None,
            vars: Vec::new(),
        };
        assert_eq!(
            effect(1_000_000_007).resolve(&resolution).command_target,
            Some(7)
        );
        assert_eq!(
            effect(1_001_000_001).resolve(&resolution).command_target,
            Some(1_001_000_001),
            "modern raw effect targets above C4EnumPointer2 stay raw"
        );
    }

    #[test]
    fn serialized_map_denumeration_preserves_native_string_slot_lifetimes() {
        let object_numbers = HashSet::new();

        // S0=o999 compiles the string key before the missing value clears its
        // entry. The key itself is destroyed, so no C4String reference remains.
        let registrations = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&registrations, 0, "loaded");
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &registrations,
        };
        let clonk_script::Value::Proplist(map) = parse_serialized_c4value("m[1;S0=o999]", 1)
            .expect("missing-value map parses")
            .resolve(&resolution)
        else {
            panic!("expected resolved map");
        };
        assert!(map.is_empty());
        assert!(clonk_script::resolve_c4_string(&registrations, 0).is_none());

        // Compilation resolves every map value before denumeration removes
        // any entry. The sibling therefore claims S0 while the doomed key is
        // still alive and retains that exact loaded identity afterwards.
        let registrations = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&registrations, 0, "loaded");
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &registrations,
        };
        let clonk_script::Value::Proplist(map) = parse_serialized_c4value("m[2;S0=o999;i1=S0]", 1)
            .expect("sibling-string map parses")
            .resolve(&resolution)
        else {
            panic!("expected resolved map");
        };
        let Some(clonk_script::Value::String(sibling)) = map.get_key(&clonk_script::Value::Int(1))
        else {
            panic!("resolved sibling string remains visible");
        };
        let registered = clonk_script::resolve_c4_string(&registrations, 0)
            .expect("visible sibling keeps S0 registered");
        assert!(sibling.ptr_eq(&registered));

        // o999=S0 removes the key but C4ValueHash::emptyValues retains its
        // mapped slot. Reusing that slot for a later insertion releases S0.
        let registrations = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&registrations, 0, "loaded");
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &registrations,
        };
        let clonk_script::Value::Proplist(mut map) =
            parse_serialized_c4value("m[2;o999=S0;i1=i2]", 1)
                .expect("missing-key map parses")
                .resolve(&resolution)
        else {
            panic!("expected resolved map");
        };
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get_key(&clonk_script::Value::Int(1)),
            Some(&clonk_script::Value::Int(2)),
            "a slot compiled after the doomed key does not reuse emptyValues"
        );
        assert!(clonk_script::resolve_c4_string(&registrations, 0).is_some());
        map.insert_key(clonk_script::Value::Int(7), clonk_script::Value::Int(8));
        assert_eq!(
            map.get_key(&clonk_script::Value::Int(7)),
            Some(&clonk_script::Value::Int(8))
        );
        assert!(clonk_script::resolve_c4_string(&registrations, 0).is_none());
    }

    #[test]
    fn serialized_map_compile_keeps_only_the_final_duplicate_assignment() {
        let object_numbers = HashSet::new();

        let registrations = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&registrations, 0, "loaded");
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &registrations,
        };
        let clonk_script::Value::Proplist(map) = parse_serialized_c4value("m[2;i1=S0;i1=o999]", 1)
            .expect("duplicate-key map parses")
            .resolve(&resolution)
        else {
            panic!("expected resolved map");
        };
        assert!(map.is_empty(), "the final missing value removes the slot");
        assert_eq!(
            map.hidden_values().cloned().collect::<Vec<_>>(),
            vec![clonk_script::Value::Nil]
        );
        assert!(clonk_script::resolve_c4_string(&registrations, 0).is_none());

        let registrations = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&registrations, 0, "loaded");
        let resolution = SerializedC4ValueResolution {
            object_numbers: &object_numbers,
            string_registrations: &registrations,
        };
        let clonk_script::Value::Proplist(map) = parse_serialized_c4value("m[2;i1=o999;i1=S0]", 1)
            .expect("reverse duplicate-key map parses")
            .resolve(&resolution)
        else {
            panic!("expected resolved map");
        };
        assert_eq!(
            map.get_key(&clonk_script::Value::Int(1)),
            Some(&clonk_script::Value::String("loaded".into()))
        );
        assert_eq!(map.hidden_values().count(), 0);
    }

    #[test]
    fn local_named_consumes_exactly_the_declared_count() {
        let entries =
            parse_local_named("1;kept=i1,ignored=i2", 1).expect("trailing values are ignored");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "kept");
        assert!(parse_local_named("0;ignored=i2", 1)
            .expect("zero count returns before payload")
            .is_empty());
        assert!(parse_local_named("2;only=i1", 1).is_err());
    }

    #[test]
    fn compiled_scoreboard_requires_every_allocated_cell_field() {
        for source in [
            b"[Scoreboard]\r\nRows=1\r\nCols=1\r\nCell0_0Value=7\r\n".as_slice(),
            b"[Scoreboard]\r\nRows=1\r\nCols=1\r\nCell0_0String=Title\r\n".as_slice(),
        ] {
            let data = crate::parse_initial_network_game_data(source);
            assert!(
                InitialNetworkRuntimeState::parse(&data).is_err(),
                "C++ rejects a partially compiled scoreboard matrix"
            );
        }

        let data = crate::parse_initial_network_game_data(
            b"[Scoreboard]\r\nRows=1\r\nCols=1\r\nCell0_0String=Title\r\nCell0_0Value=7\r\n",
        );
        let runtime = InitialNetworkRuntimeState::parse(&data)
            .expect("complete compiled scoreboard validates");
        assert_eq!(
            runtime.scoreboard.cell(0, 0).and_then(|cell| cell.text()),
            Some("Title")
        );
        assert_eq!(
            runtime.scoreboard.cell(0, 0).map(|cell| cell.value()),
            Some(7)
        );
    }

    // Objects.txt `LocalNamed=` (C4Object.cpp:2788; C4ValueMapData::
    // CompileFunc, C4ValueMap.cpp:236-295): per-object script locals load
    // verbatim with the C4Value type-char encoding (GetC4VID,
    // C4Value.cpp:368-394) — A=any (zero data reads back nil), i=int,
    // b=bool, I=C4ID, O=enumerated object number, a[size;elems]=array with
    // trailing nils omitted. The I payloads below are verbatim C++-written
    // Dragon Rock values (C4Value.cpp:717-800; C4Id.cpp:26-45).
    #[test]
    fn serialized_c4ids_preserve_raw_payload_collisions_and_sign_extension() {
        for (encoded, expected) in [("I825307441", 825_307_441usize), ("I-1", usize::MAX)] {
            let SerializedC4Value::Value(clonk_script::Value::C4Id(id)) =
                parse_serialized_c4value(encoded, 1).expect("serialized C4ID parses")
            else {
                panic!("expected a typed C4ID");
            };
            assert_eq!(clonk_script::c4_id_raw(&id), expected);
        }
    }

    #[test]
    fn objects_txt_restores_named_locals_like_cpp() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\n",
        )
        .expect("write defcore");
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Locals.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Locals\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=95\nStatus=1\nX=5\nY=5\n\
             LocalNamed=10;iNum=i17,fFlag=b1,pRef=O80,junk=A0,aList=a[4;i1,i2],\
             idSpell=I1112688205,aiFirst=I1145979202,numeric=I1337,none=I0,\
             aSpells=a[3;I959858757,I1145979202]\n\n\
             [Object]\nid=GOOD\nNumber=80\nStatus=1\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let idx = engine
            .find_object_index(ObjectId::new(95))
            .expect("object exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(locals.get("iNum"), Some(&clonk_script::Value::Int(17)));
        assert_eq!(locals.get("fFlag"), Some(&clonk_script::Value::Bool(true)));
        assert_eq!(
            locals.get("pRef"),
            Some(&clonk_script::Value::Object(80)),
            "O-typed refs resolve after every object has loaded"
        );
        assert_eq!(
            locals.get("junk"),
            Some(&clonk_script::Value::Nil),
            "C4V_Any with zero data reads back nil"
        );
        assert_eq!(
            locals.get("aList"),
            Some(&clonk_script::Value::Array(vec![
                clonk_script::Value::Int(1),
                clonk_script::Value::Int(2),
                clonk_script::Value::Nil,
                clonk_script::Value::Nil,
            ])),
            "arrays restore the declared size; trailing nils are omitted on write"
        );
        assert_eq!(
            locals.get("idSpell"),
            Some(&clonk_script::Value::C4Id("MFRB".to_string())),
            "C++'s signed int32 C4ID payload decodes in little-endian byte order"
        );
        assert_eq!(
            locals.get("aiFirst"),
            Some(&clonk_script::Value::C4Id("BAND".to_string())),
            "callback-suffix IDs survive even when they are not definitions"
        );
        assert_eq!(
            locals.get("numeric"),
            Some(&clonk_script::Value::C4Id("1337".to_string())),
            "numeric C4IDs use the four-digit C4IdText form"
        );
        assert_eq!(
            locals.get("none"),
            Some(&clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(0))),
            "C4ID_None preserves its tag while remaining falsey"
        );
        assert!(!locals.get("none").expect("I0 local exists").as_bool());
        assert_eq!(
            locals.get("aSpells"),
            Some(&clonk_script::Value::Array(vec![
                clonk_script::Value::C4Id("EH69".to_string()),
                clonk_script::Value::C4Id("BAND".to_string()),
                clonk_script::Value::Nil,
            ])),
            "nested C4IDs restore through the recursive array decoder"
        );
    }

    // C4Value::DenumeratePointer subtracts the legacy enumeration offset,
    // searches both active and inactive object lists, and clears a missing
    // object (C4Value.cpp:693-713; C4ObjectList.h:32-34). It recurses through
    // containers, and C4Object applies it to LocalNamed after every object is
    // loaded (C4Value.cpp:686-690; C4Object.cpp:2914-2923).
    // Serialized strings resolve by the IDs loaded from Strings.txt; duplicate
    // text reuses the existing C4String and moves its enum ID to the later line
    // (C4StringTable.cpp:201-216; C4Value.cpp:783-798).
    #[test]
    fn objects_txt_denumerates_named_local_identities_like_cpp() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\n",
        )
        .expect("write defcore");
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Identities.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Identities\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Strings.txt"),
            b"first\r\nM\xfcnkelburg\r\nsame\r\nsame\r\n",
        )
        .expect("write string table");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\n\
             LocalNamed=9;pOffset=O1000000419,pPlain=O419,pMissing=O1000000999,\
             aRefs=a[3;O1000000419,O1000000999],sFirst=S0,sUmlaut=S1,\
             sOldDuplicate=S2,sDuplicate=S3,sMissing=S7\n\n\
             [Object]\nid=GOOD\nNumber=419\nStatus=2\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let holder = engine
            .object_snapshot(ObjectId::new(10))
            .expect("holder exists");
        let locals = &holder.local_vars;
        assert_eq!(
            locals.get("pOffset"),
            Some(&clonk_script::Value::Object(419)),
            "C4EnumPointer1 is removed before lookup"
        );
        assert_eq!(
            locals.get("pPlain"),
            Some(&clonk_script::Value::Object(419)),
            "unoffset enum values resolve too"
        );
        assert_eq!(locals.get("pMissing"), Some(&clonk_script::Value::Nil));
        assert_eq!(
            locals.get("aRefs"),
            Some(&clonk_script::Value::Array(vec![
                clonk_script::Value::Object(419),
                clonk_script::Value::Nil,
                clonk_script::Value::Nil,
            ])),
            "container denumeration is recursive"
        );
        assert_eq!(
            locals.get("sFirst"),
            Some(&clonk_script::Value::String("first".to_string().into()))
        );
        let Some(clonk_script::Value::String(umlaut)) = locals.get("sUmlaut") else {
            panic!("raw string-table identity denumerates to a string");
        };
        assert_eq!(
            clonk_script::c4_string_bytes(umlaut),
            b"M\xfcnkelburg",
            "native C4String bytes remain lossless instead of being recoded as UTF-8"
        );
        assert_eq!(
            locals.get("sOldDuplicate"),
            Some(&clonk_script::Value::Nil),
            "a duplicate string moves the shared C4String to the later enum ID"
        );
        assert_eq!(
            locals.get("sDuplicate"),
            Some(&clonk_script::Value::String("same".to_string().into()))
        );
        assert_eq!(
            locals.get("sFirst").and_then(|value| match value {
                clonk_script::Value::String(value) => Some(value.enum_id()),
                _ => None,
            }),
            Some(0)
        );
        assert_eq!(
            locals.get("sUmlaut").and_then(|value| match value {
                clonk_script::Value::String(value) => Some(value.enum_id()),
                _ => None,
            }),
            Some(1)
        );
        assert_eq!(
            locals.get("sDuplicate").and_then(|value| match value {
                clonk_script::Value::String(value) => Some(value.enum_id()),
                _ => None,
            }),
            Some(3),
            "the resolved C4Value retains the duplicate line's overwritten native ID"
        );
        assert_eq!(locals.get("sMissing"), Some(&clonk_script::Value::Nil));

        let persisted = serde_json::to_string(&holder).expect("snapshot serializes");
        let restored: crate::ObjectSnapshot =
            serde_json::from_str(&persisted).expect("snapshot restores");
        assert_eq!(restored.local_vars, holder.local_vars);
    }

    // C4Weather::Init at scenario start draws from the SYNCED ledger
    // (C4Weather.cpp:36-70): Season, YearSpeed, Climate, Wind (the value
    // trees read through GetWind), the NoInitialize-gated rain block, then
    // Lightning and the Disasters. Every C4SVal::Evaluate draws
    // Random(2*Rnd+1) even for Rnd=0 (C4Scenario.cpp:43-46), so the whole
    // RNG stream shifts if any draw is skipped.
    #[test]
    fn fresh_legacy_initialize_def_and_placements_observe_default_weather_before_weather_init() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "PROB",
                "#strict\n\
                 static initialize_def_weather;\n\
                 static initialize_def_random;\n\
                 static placement_construction_weather;\n\
                 static placement_completion_weather;\n\
                 static placement_weather;\n\
                 static placement_random;\n\
                 func CurrentWeather() {\n\
                     return [GetWind(0, 0, true), GetTemperature(), GetClimate(), GetSeason()];\n\
                 }\n\
                 func InitializeDef() {\n\
                     initialize_def_weather = CurrentWeather();\n\
                     initialize_def_random = Random(1000000);\n\
                     return 1;\n\
                 }\n\
                 func Construction() {\n\
                     placement_construction_weather = CurrentWeather();\n\
                     return 1;\n\
                 }\n\
                 func Completion() {\n\
                     placement_completion_weather = CurrentWeather();\n\
                     return 1;\n\
                 }\n\
                 func Initialize() {\n\
                     placement_weather = CurrentWeather();\n\
                     placement_random = Random(1000000);\n\
                     return 1;\n\
                 }\n",
            )),
            "// no scenario script\n",
        );
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Weather callback order\n\n\
             [Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=10\nGravity=120,3,10,200\n\
             VegetationLevel=20,4,0,100\nInEarthLevel=30,2,0,100\n\n\
             [Environment]\nObjects=PROB=1;\n\n\
             [Weather]\nStartSeason=30,10,0,100\nYearSpeed=45,5,0,100\n\
             Climate=60,10,0,100\nWind=10,5,-20,20\nRain=25,5,0,100\n\
             Lightning=12,5,0,100\nNoGamma=0\n\n\
             [Disasters]\nMeteorite=25,4,0,100\nVolcano=15,3,0,100\n\
             Earthquake=5,2,0,100\n",
        )
        .expect("write scenario core");
        // The loader expands this to its 100x100 static-map minimum:
        // LandscapeLoaded enables the placement block while the world stays
        // below the 500-pixel threshold for rain-cloud creation.
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0u8, 0][..], &[0u8, 0][..]]),
        )
        .expect("write landscape");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        assert_ne!(
            scenario.environment().expect("scenario environment"),
            EnvironmentSettings::default(),
            "the fixture must carry non-default scenario weather"
        );
        assert_eq!(
            scenario.environment_before_weather_init(false),
            EnvironmentSettings::default(),
            "every staged live-weather field starts at C4Weather::Default"
        );

        let mut engine = Engine::with_seed(9);
        scenario
            .apply_before_network_final_init(&mut engine)
            .expect("fresh scenario applies without the final RNG re-fix");

        let global = |name: &str| {
            engine
                .script_globals
                .borrow()
                .get(name)
                .map(|cell| cell.borrow().clone())
        };
        let default_getters = clonk_script::Value::Array(vec![clonk_script::Value::Int(0); 4]);
        assert_eq!(
            global("initialize_def_weather"),
            Some(default_getters.clone()),
            "InitializeDef sees C4Weather::Default"
        );
        assert_eq!(
            global("placement_construction_weather"),
            Some(default_getters.clone()),
            "InitEnvironment object Construction sees C4Weather::Default"
        );
        assert_eq!(
            global("placement_completion_weather"),
            Some(default_getters.clone()),
            "InitEnvironment object Completion sees C4Weather::Default"
        );
        assert_eq!(
            global("placement_weather"),
            Some(default_getters),
            "InitEnvironment object Initialize sees C4Weather::Default"
        );

        // Replay the live synced ledger through the exact C++ boundary:
        // Gravity, InitializeDef, the two unconditional placement-level
        // evaluates, the placed object's Initialize, then Weather.Init.
        let mut replay = crate::rng::LcgRng::seed_from_u64(9);
        LegacyC4SVal::new(120, 3, 10, 200).evaluate(&mut replay);
        let initialize_def_random = replay.random(1_000_000);
        LegacyC4SVal::new(20, 4, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(30, 2, 0, 100).evaluate(&mut replay);
        let placement_random = replay.random(1_000_000);
        let season = LegacyC4SVal::new(30, 10, 0, 100).evaluate(&mut replay);
        let year_speed = LegacyC4SVal::new(45, 5, 0, 100).evaluate(&mut replay);
        let climate = 100 - LegacyC4SVal::new(60, 10, 0, 100).evaluate(&mut replay) - 50;
        let wind = LegacyC4SVal::new(10, 5, -20, 20).evaluate(&mut replay);
        let rain = LegacyC4SVal::new(25, 5, 0, 100).evaluate(&mut replay);
        let lightning = LegacyC4SVal::new(12, 5, 0, 100).evaluate(&mut replay);
        let meteorite = LegacyC4SVal::new(25, 4, 0, 100).evaluate(&mut replay);
        let volcano = LegacyC4SVal::new(15, 3, 0, 100).evaluate(&mut replay);
        let earthquake = LegacyC4SVal::new(5, 2, 0, 100).evaluate(&mut replay);

        assert_eq!(
            global("initialize_def_random"),
            Some(clonk_script::Value::Int(initialize_def_random))
        );
        assert_eq!(
            global("placement_random"),
            Some(clonk_script::Value::Int(placement_random))
        );
        let environment = engine.environment();
        assert_eq!(
            (
                environment.season,
                environment.year_speed,
                environment.climate,
                environment.temperature,
                environment.wind,
                environment.wind_target,
            ),
            (season, year_speed, climate, climate, wind, wind)
        );
        assert_eq!(
            (
                environment.lightning,
                environment.meteorite,
                environment.volcano,
                environment.earthquake,
            ),
            (lightning, meteorite, volcano, earthquake)
        );
        assert_eq!(environment.precipitation, rain);
        assert_eq!(environment.precipitation_strength, 25);
        assert_eq!((environment.season_min, environment.season_max), (0, 100));
        assert_eq!(
            (
                environment.base_wind,
                environment.wind_variation,
                environment.wind_min,
                environment.wind_max,
            ),
            (10, 5, -20, 20)
        );
        assert!(!environment.no_gamma);
        assert_eq!(engine.debug_rng_clone(), replay);
    }
