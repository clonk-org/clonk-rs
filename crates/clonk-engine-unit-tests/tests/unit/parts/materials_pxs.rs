    fn materials_pxs_test_materials(source: &str) -> MaterialSet {
        let library = MaterialLibrary::parse(source).test_value();
        MaterialSet::from_resource_library(&library)
    }

    fn materials_pxs_with_earth(source: &str) -> MaterialSet {
        materials_pxs_test_materials(&format!(
            "{source}\n[Material Earth]\nName=Earth\nDensity=100\nFriction=25\n"
        ))
    }

    fn pxs_grid(
        width: u32,
        height: u32,
        bytes: Vec<u8>,
        materials: &[(usize, i32, &str)],
    ) -> landscape::PixelGrid {
        let mut densities = vec![0; 128];
        let mut names = vec![None; 128];
        for &(index, density, name) in materials {
            densities[index] = density;
            names[index] = Some(name.to_owned());
        }
        landscape::PixelGrid::new(width, height, bytes, densities, names, vec![None; 128])
    }

    fn pxs_grid_world(
        width: u32,
        height: i32,
        surface: Vec<i32>,
        grid: landscape::PixelGrid,
    ) -> Landscape {
        let mut world = Landscape::new(width, surface).test_value();
        world.set_world_height(height);
        world.set_pixel_grid(grid);
        world
    }

    fn pxs_engine(seed: u64, materials: MaterialSet) -> Engine {
        let mut engine = Engine::with_seed(seed);
        engine.set_materials(materials);
        engine
    }

    fn pxs_fixture(seed: u64, definition: Definition, config: SpawnConfig) -> (Engine, ObjectId) {
        let mut engine = Engine::with_seed(seed);
        engine.register_test_definition(definition);
        let id = engine.spawn_test_object(config);
        (engine, id)
    }

    fn pxs_default_fixture(definition: Definition, config: SpawnConfig) -> (Engine, ObjectId) {
        let mut engine = Engine::new();
        engine.register_test_definition(definition);
        let id = engine.spawn_test_object(config);
        (engine, id)
    }

    fn no_other_action_fixture(seed: u64) -> (Engine, ObjectId) {
        let mut definition = test_definition(
            "Actor",
            "Actor",
            r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        "#,
        );
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                (
                    "Idle".to_string(),
                    ActionSpec::default().with_no_other_action(true),
                ),
                ("Run".to_string(), ActionSpec::default()),
            ]),
        );
        pxs_fixture(seed, definition, SpawnConfig::new("Actor"))
    }

    fn start_abort_definition(
        id: &str,
        name: &str,
        source: &str,
        hooks: DebuggerHooks,
    ) -> Definition {
        let mut definition = test_definition(id, name, source);
        definition.set_debugger_hooks(hooks);
        definition.set_c4_callback_convention(true);
        definition.configure_actions(
            Some("Old".to_string()),
            HashMap::from([
                (
                    "Old".to_string(),
                    ActionSpec::default().with_abort_call("OnOldAbort"),
                ),
                (
                    "New".to_string(),
                    ActionSpec::default().with_start_call("OnNewStart"),
                ),
            ]),
        );
        definition
    }

    fn create_test_pxs(
        engine: &mut Engine,
        material: MaterialId,
        x: i32,
        y: i32,
        xdir: i32,
        ydir: i32,
    ) -> bool {
        engine.pxs_system.create(
            material,
            math::itofix(x),
            math::itofix(y),
            math::itofix(xdir),
            math::itofix(ydir),
        )
    }

    fn pxs_material_ids<const N: usize>(
        materials: &MaterialSet,
        names: [&str; N],
    ) -> [MaterialId; N] {
        names.map(|name| materials.id_of(name).test_value())
    }

    fn pxs_call_hooks<T, F>(record: F) -> (Arc<Mutex<Vec<T>>>, DebuggerHooks)
    where
        T: Send + 'static,
        F: Fn(&str, &[Value]) -> Option<T> + Send + Sync + 'static,
    {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let call_log = Arc::clone(&calls);
        let hooks = DebuggerHooks::new().with_on_call(move |name, args| {
            if let Some(call) = record(name, args) {
                call_log.lock().test_value().push(call);
            }
        });
        (calls, hooks)
    }

    #[test]
    fn weather_disaster_rng_draw_order_matches_cpp() {
        // C4Weather::Execute disaster block (C4Weather.cpp:104-148): on every
        // Tick10 frame the gates Random(60) [meteorite], Random(35)
        // [lightning], Random(50) [earthquake], Random(60) [volcano] are
        // drawn UNCONDITIONALLY — the configured levels only gate the
        // follow-up Random(100) comparison, never the gate draw itself.
        let materials = materials_pxs_with_earth("");
        let earth = materials.id_of("Earth").test_value();

        let mut engine = pxs_engine(123, materials.clone());
        engine.set_landscape(Landscape::flat_with_material(64, 40, Some(earth)));
        let mut mirror = engine.rng.clone();

        engine.tick_weather_events(10).test_value();
        // levels all default to 0: each zero gate still draws Random(100)
        if mirror.random(60) == 0 {
            mirror.random(100);
        }
        if mirror.random(35) == 0 {
            mirror.random(100);
        }
        if mirror.random(50) == 0 {
            mirror.random(100);
        }
        if mirror.random(60) == 0 {
            mirror.random(100);
        }
        assert_eq!(engine.rng, mirror, "gate draws are level-independent");

        // Non-Tick10 frame: no draws at all (C4Weather.cpp:104).
        let before = engine.rng.clone();
        engine.tick_weather_events(11).test_value();
        assert_eq!(engine.rng, before);

        // With a level at 100, a zero gate launches: lightning consumes
        // Random(GBackWdt) for its position (C4Weather.cpp:125); earthquake
        // consumes Random(GBackHgt) then Random(GBackWdt) (:133-134);
        // volcano consumes Random(10) then Random(GBackWdt) (:142-143);
        // meteorite consumes Random(101) then Random(GBackWdt) (:114-115).
        // No FX definitions are registered, so object creation is skipped,
        // but the synced draws must still happen.
        let mut engine = pxs_engine(7, materials);
        engine.set_landscape(Landscape::flat_with_material(64, 40, Some(earth)));
        let mut environment = engine.environment();
        environment.meteorite = 100;
        environment.lightning = 100;
        environment.earthquake = 100;
        environment.volcano = 100;
        engine.set_environment(environment);
        let height = engine
            .landscape
            .as_ref()
            .map(|landscape| landscape.estimated_height())
            .unwrap_or(0);

        for frame in 1..=400u64 {
            let mut mirror = engine.rng.clone();
            engine.tick_weather_events(frame).test_value();
            if frame % 10 != 0 {
                assert_eq!(engine.rng, mirror);
                continue;
            }
            if mirror.random(60) == 0 && mirror.random(100) < 100 {
                mirror.random(100 + 1);
                mirror.random(64);
            }
            if mirror.random(35) == 0 && mirror.random(100) < 100 {
                mirror.random(64);
            }
            if mirror.random(50) == 0 && mirror.random(100) < 100 {
                mirror.random(height);
                mirror.random(64);
            }
            if mirror.random(60) == 0 && mirror.random(100) < 100 {
                mirror.random(10);
                mirror.random(64);
            }
            assert_eq!(engine.rng, mirror, "frame {frame}");
        }
    }

    #[test]
    fn weather_meteor_spawn_honors_open_and_closed_top_motion() {
        // C4Weather::Execute creates METO at y=-20 with zero ydir when
        // Landscape.TopOpen, but at y=5 with itofix(2) ydir in a closed cave.
        // The shared r2/r1 draws still precede object creation in both cases
        // (C4Weather.cpp:104-120). C4Object::Init installs those dirs and
        // marks the meteor Mobile before Construction/Initialize run.
        let spawn = |top_open: bool| {
            let mut engine = Engine::with_seed(18);
            let mut landscape = Landscape::flat(64, 40);
            landscape.set_border_open(0, 0, top_open, false);
            engine.set_landscape(landscape);
            let mut meteor_definition = test_definition(
                "METO",
                "Meteor",
                r#"#strict 2
            local construction_xdir, initialize_xdir;
            protected func Construction() { construction_xdir = GetXDir(); }
            protected func Initialize() { initialize_xdir = GetXDir(); }
            "#,
            );
            meteor_definition.set_category(CATEGORY_OBJECT);
            meteor_definition.set_rotateable(1);
            meteor_definition.set_c4_callback_convention(true);
            engine.register_test_definition(meteor_definition);
            let mut environment = engine.environment();
            environment.meteorite = 100;
            engine.set_environment(environment);

            let mut mirror = engine.rng.clone();
            assert_eq!(mirror.random(60), 0, "seed reaches the meteor gate");
            assert!(mirror.random(100) < 100);
            let r2 = mirror.random(101);
            let x = mirror.random(64);

            engine.tick_weather_events(10).test_value();
            let expected_xdir = C4Fixed::from_raw(itofix(r2 - 50).val() / 10);
            let expected_ydir = if top_open { C4Fixed::ZERO } else { itofix(2) };
            let expected_rdir = C4Fixed::from_raw(itofix(1).val() / 5);
            let meteor = engine
                .objects
                .iter()
                .find(|object| object.definition_id == "METO")
                .test_value();
            let meteor_id = meteor.id;
            assert_eq!(meteor.state.position.x, x);
            assert_eq!(meteor.state.owner, OWNER_NONE);
            assert_eq!(meteor.fixed_velocity.x, expected_xdir);
            assert_eq!(meteor.rotation_velocity, expected_rdir);
            assert_eq!(
                engine.debug_object_motion(meteor_id.as_u64()),
                Some((expected_xdir.val(), expected_ydir.val(), true)),
                "meteor has its C4Object::Init motion on the spawn frame"
            );
            for callback_local in ["construction_xdir", "initialize_xdir"] {
                assert!(
                    matches!(
                        meteor.state.local_vars.get(callback_local),
                        Some(Value::Int(value)) if *value != 0
                    ),
                    "{callback_local} must observe the initial nonzero xdir"
                );
            }

            let initial_fixed_y = meteor.fixed_position.y;
            let result = (meteor.state.position.y, meteor.fixed_velocity.y);
            if top_open {
                for _ in 0..5 {
                    engine.tick_without_snapshot().test_value();
                }
                let meteor = engine
                    .objects
                    .iter()
                    .find(|object| object.id == meteor_id)
                    .test_value();
                assert!(
                    meteor.fixed_position.y > initial_fixed_y,
                    "open-top meteor falls under gravity before the next Tick10 pulse"
                );
            }
            result
        };

        assert_eq!(spawn(true), (-20, C4Fixed::ZERO));
        assert_eq!(spawn(false), (5, itofix(2)));
    }

    #[test]
    fn weather_wind_sound_level_starts_updates_and_stops_one_global_loop() {
        // C4Weather::Execute first steps Wind on Tick10, then calls
        // SoundLevel("Wind", nullptr, max(Abs(Wind)-30, 0)*2), before the
        // disaster RNG block (C4Weather.cpp:94-104; C4SoundSystem.cpp:38-51).
        // Tutorial07's fixed Wind=50 therefore asks the frontend to update an
        // existing global instance or start one loop at volume 40, and keeps
        // issuing the unconditional stop attempt while wind is calm.
        let mut engine = Engine::with_seed(18);
        engine.set_environment(EnvironmentSettings::new(50));

        for _ in 0..9 {
            assert!(engine
                .tick()
                .expect("pre-Tick10 frame succeeds")
                .audio
                .is_empty());
        }
        assert_eq!(
            engine.tick().expect("Tick10 weather succeeds").audio,
            vec![AudioCommand::SetSoundVolume {
                name: "Wind".to_string(),
                target: None,
                volume: 40,
            }]
        );

        for _ in 0..9 {
            assert!(engine
                .tick()
                .expect("pre-Tick20 frame succeeds")
                .audio
                .is_empty());
        }
        let mut rising = EnvironmentSettings::new(49).with_wind_variation(1, 1_000);
        rising.wind_target = 50;
        engine.set_environment(rising);
        assert_eq!(
            engine.tick().expect("Tick20 weather succeeds").audio,
            vec![AudioCommand::SetSoundVolume {
                name: "Wind".to_string(),
                target: None,
                volume: 40,
            }],
            "SoundLevel observes the Tick10 wind step before updating volume"
        );

        engine.set_environment(EnvironmentSettings::new(30));
        for _ in 0..9 {
            assert!(engine
                .tick()
                .expect("pre-Tick30 frame succeeds")
                .audio
                .is_empty());
        }
        assert_eq!(
            engine.tick().expect("Tick30 weather succeeds").audio,
            vec![AudioCommand::StopSound {
                name: "Wind".to_string(),
                target: None,
            }]
        );

        for _ in 0..9 {
            assert!(engine
                .tick()
                .expect("pre-Tick40 calm frame succeeds")
                .audio
                .is_empty());
        }
        assert_eq!(
            engine.tick().expect("Tick40 calm weather succeeds").audio,
            vec![AudioCommand::StopSound {
                name: "Wind".to_string(),
                target: None,
            }],
            "SoundLevel(0) always attempts StopSoundEffect"
        );
    }

    #[test]
    fn maximum_wind_preserves_level_140_gain() {
        let mut engine = Engine::with_seed(19);
        engine.set_environment(EnvironmentSettings::new(100));

        for _ in 0..9 {
            assert!(engine
                .tick()
                .expect("pre-Tick10 frame succeeds")
                .audio
                .is_empty());
        }
        assert_eq!(
            engine.tick().expect("maximum-wind Tick10 succeeds").audio,
            vec![AudioCommand::SetSoundVolume {
                name: "Wind".to_string(),
                target: None,
                volume: 140,
            }]
        );
    }

    #[test]
    fn negative_material_reaction_flags_are_truthy_like_cpp() {
        use material::{evaluate_corrosion, MaterialReactionKind};

        let natural = MaterialSet::from_resource_library(
            &MaterialLibrary::parse(
                r#"
                [Material IL]
                Name=IL
                Density=10
                Incindiary=-1

                [Material EH]
                Name=EH
                Density=20
                Extinguisher=-2

                [Material EL]
                Name=EL
                Density=10
                Extinguisher=-3

                [Material IH]
                Name=IH
                Density=20
                Incindiary=-4

                [Material FH]
                Name=FH
                Density=20
                Inflammable=-5

                [Material FL]
                Name=FL
                Density=10
                Inflammable=-6

                [Material AC]
                Name=AC
                Density=10
                Corrosive=-7

                [Material RH]
                Name=RH
                Density=20
                Corrode=-8
                "#,
            )
            .test_value(),
        );
        let id = |name| natural.id_of(name).test_value();
        assert_eq!(
            natural.reaction(Some(id("IL")), Some(id("EH"))).kind,
            MaterialReactionKind::Poof,
        );
        assert_eq!(
            natural.reaction(Some(id("EL")), Some(id("IH"))).kind,
            MaterialReactionKind::Poof,
        );
        assert_eq!(
            natural.reaction(Some(id("IL")), Some(id("FH"))).kind,
            MaterialReactionKind::Incinerate,
        );
        assert_eq!(
            natural.reaction(Some(id("FL")), Some(id("IH"))).kind,
            MaterialReactionKind::Incinerate,
        );
        assert_eq!(
            natural.reaction(Some(id("AC")), Some(id("RH"))).kind,
            MaterialReactionKind::Corrode {
                corrosive_strength: -7,
                corrode_resistance: -8,
                corrosion_probability: None,
            },
        );

        // The flags select the reaction by raw C++ integer truthiness, but
        // their signed values remain the probability thresholds. A negative
        // first threshold short-circuits after one draw; a negative second
        // threshold is reached after a guaranteed first success.
        let mut negative_first = LcgRng::new(67);
        let mut one_draw = negative_first.clone();
        let _ = one_draw.random(100);
        assert!(!evaluate_corrosion(-7, -8, None, &mut negative_first));
        assert_eq!(negative_first, one_draw);

        let mut negative_second = LcgRng::new(68);
        let mut two_draws = negative_second.clone();
        let _ = two_draws.random(100);
        let _ = two_draws.random(100);
        assert!(!evaluate_corrosion(100, -8, None, &mut negative_second));
        assert_eq!(negative_second, two_draws);

        let categories = MaterialSet::from_resource_library(
            &MaterialLibrary::parse(
                r#"
                [Material RI]
                Name=RI
                Density=10

                [Reaction]
                Type=Poof
                TargetSpec=Incindiary

                [Material RE]
                Name=RE
                Density=10

                [Reaction]
                Type=Poof
                TargetSpec=Extinguisher

                [Material RF]
                Name=RF
                Density=10

                [Reaction]
                Type=Poof
                TargetSpec=Inflammable

                [Material RC]
                Name=RC
                Density=10

                [Reaction]
                Type=Poof
                TargetSpec=Corrosive

                [Material RR]
                Name=RR
                Density=10

                [Reaction]
                Type=Poof
                TargetSpec=Corrode

                [Material IV]
                Name=IV
                Density=10

                [Reaction]
                Type=Poof
                TargetSpec=Incindiary
                InverseSpec=1

                [Material Rate]
                Name=Rate
                Density=10

                [Reaction]
                Type=Corrode
                TargetSpec=Zero
                CorrosionRate=-9

                [Material Neg]
                Name=Neg
                Density=20
                Incindiary=-1
                Extinguisher=-2
                Inflammable=-3
                Corrosive=-4
                Corrode=-5

                [Material Zero]
                Name=Zero
                Density=20
                "#,
            )
            .test_value(),
        );
        let neg = categories.id_of("Neg").test_value();
        let zero = categories.id_of("Zero").test_value();
        for source_name in ["RI", "RE", "RF", "RC", "RR"] {
            let source = categories.id_of(source_name).test_value();
            let reaction = categories.reaction(Some(source), Some(neg));
            assert!(reaction.user_defined, "{source_name} matched its category");
            assert_eq!(reaction.kind, MaterialReactionKind::Poof);
        }

        let inverse = categories.id_of("IV").test_value();
        assert!(
            !categories.reaction(Some(inverse), Some(neg)).user_defined,
            "a nonzero negative flag is excluded from the inverse category",
        );
        assert!(
            categories.reaction(Some(inverse), Some(zero)).user_defined,
            "a zero flag matches the inverse category",
        );
        assert!(
            categories.reaction(Some(inverse), None).user_defined,
            "sky matches an inverse flag category",
        );

        let rate = categories.id_of("Rate").test_value();
        let rate_reaction = categories.reaction(Some(rate), Some(zero));
        assert_eq!(
            rate_reaction.kind,
            MaterialReactionKind::Corrode {
                corrosive_strength: -9,
                corrode_resistance: 100,
                corrosion_probability: Some(-9),
            },
            "custom CorrosionRate remains a raw signed threshold",
        );
        let mut custom_rng = LcgRng::new(69);
        let mut custom_one_draw = custom_rng.clone();
        let _ = custom_one_draw.random(100);
        assert!(!evaluate_corrosion(-9, 100, Some(-9), &mut custom_rng));
        assert_eq!(custom_rng, custom_one_draw);
    }

    #[test]
    fn mrf_insert_check_splash_matches_cpp() {
        // mrfInsertCheck splash (C4Material.cpp:572-579): with fYDir >
        // itofix(1) and SplashRate set, !Random(SplashRate) bounces the PXS:
        // fYDir = -fYDir/8 (raw int division), fXDir = fXDir/8 +
        // FIXED100(Random(200)-100), and the nonzero fYDir keeps it alive.
        let materials = materials_pxs_with_earth(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
            SplashRate=1
        "#,
        );
        let [water, earth] = pxs_material_ids(&materials, ["Water", "Earth"]);
        let mut engine = pxs_engine(7, materials);
        engine.set_landscape(Landscape::flat_with_material(9, 10, Some(earth)));

        let mut mirror = engine.rng.clone();
        assert_eq!(mirror.random(1), 0, "SplashRate=1 always splashes");
        let expected_xdir = math::itofix(8) / 8 + math::fixed100(mirror.random(200) - 100);

        let (mut x, mut y) = (4, 9);
        let mut xdir = math::itofix(8);
        let mut ydir = math::itofix(16);
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            water,
            Some(earth),
            &mut pos_changed,
        );
        assert!(!insert_ok, "splash keeps the PXS alive");
        assert!(pos_changed);
        assert_eq!(ydir, -math::itofix(16) / 8);
        assert_eq!(xdir, expected_xdir);
        assert_eq!((x, y), (4, 9), "splash does not move the pixel");
        assert_eq!(engine.rng, mirror, "exactly two synced draws");
    }

    #[test]
    fn mrf_insert_check_incendiary_smokes_and_allows_insert_like_cpp() {
        // mrfInsertCheck (C4Material.cpp:584-586): incendiary materials
        // consume Random(25) and, on zero, Rnd3() for the smoke level
        // (Smoke(x, y, 4+Rnd3()), C4Effect.cpp:859-863); with no slide
        // available the check returns true (insertion OK,
        // C4Material.cpp:608-609).
        let materials = materials_pxs_with_earth(
            r#"
            [Material Lava]
            Name=Lava
            Density=30
            Friction=20
            Incindiary=1
        "#,
        );
        let [lava, earth] = pxs_material_ids(&materials, ["Lava", "Earth"]);
        let mut engine = pxs_engine(2, materials);
        // Deep inside flat earth: no slide target anywhere.
        engine.set_landscape(Landscape::flat_with_material(9, 5, Some(earth)));
        // Force the Random(25) == 0 branch deterministically.
        let smoke_seed = (0u32..)
            .find(|&seed| LcgRng::new(seed).random(25) == 0)
            .test_value();
        engine.rng = LcgRng::new(smoke_seed);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Smoke".into(),
                    init_fn: "SmokeInit".into(),
                    exec_fn: "SmokeExec".into(),
                    draw_fn: "Smoke".into(),
                    min_lifetime: 10,
                    max_lifetime: 10,
                    ..Default::default()
                },
                4,
                1.0,
            )
            .test_value();

        let mut mirror = engine.rng.clone();
        assert_eq!(mirror.random(25), 0);
        let expected_level = 4 + mirror.rnd3();

        let (mut x, mut y) = (4, 20);
        let mut xdir = math::C4Fixed::ZERO;
        let mut ydir = math::C4Fixed::ZERO;
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            lava,
            Some(earth),
            &mut pos_changed,
        );
        assert!(insert_ok, "no slide target → insertion OK");
        assert_eq!(ydir, math::C4Fixed::ZERO);
        assert_eq!(engine.rng, mirror, "Random(25) then Rnd3 consumed");
        let smoke: Vec<_> = engine
            .particle_system()
            .particles()
            .iter()
            .filter(|particle| particle.def_name == "Smoke")
            .collect();
        assert_eq!(smoke.len(), 1, "smoke particle spawned");
        assert_eq!(smoke[0].x.to_bits(), 4.0f32.to_bits());
        assert_eq!(
            smoke[0].y.to_bits(),
            (20.0f32 - (expected_level / 2) as f32).to_bits()
        );
        assert_eq!(smoke[0].a.to_bits(), (expected_level as f32).to_bits());
    }

    #[test]
    fn mrf_insert_check_slide_accelerates_or_absorbs_like_cpp() {
        // mrfInsertCheck slide (C4Material.cpp:588-607): FindMatSlide with
        // Sign(GravAccel), the PXS material's density and MaxSlide. Same
        // material → absorb (move there, fXDir = 0). Different material →
        // fXDir = (fXDir*10 + Sign(slide_x - x))/11 + FIXED10(Random(5)-2),
        // with the direct jump only when the target is within |fixtoi(fXDir)|.
        let materials = materials_pxs_with_earth(
            r#"
            [Material Sand]
            Name=Sand
            Density=25
            Friction=10
            MaxSlide=2
        "#,
        );
        let [sand, earth] = pxs_material_ids(&materials, ["Sand", "Earth"]);

        // Slide target two columns left: |x - slide_x| = 2 > |fixtoi(xdir')|
        // → no direct jump; the acceleration stays observable.
        let mut engine = pxs_engine(13, materials.clone());
        engine.set_landscape(
            Landscape::with_default_material(5, vec![11, 10, 10, 10, 11], Some(earth)).test_value(),
        );
        let mut mirror = engine.rng.clone();
        let expected_xdir =
            math::C4Fixed::from_raw((math::itofix(1).val() * 10 + math::itofix(-1).val()) / 11)
                + math::fixed10(mirror.random(5) - 2);

        let (mut x, mut y) = (2, 9);
        let mut xdir = math::itofix(1);
        let mut ydir = math::C4Fixed::ZERO;
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            sand,
            Some(earth),
            &mut pos_changed,
        );
        assert!(!insert_ok, "slide keeps the PXS alive");
        assert_eq!((x, y), (2, 9), "target out of reach → no jump");
        assert_eq!(xdir, expected_xdir);
        assert_eq!(engine.rng, mirror, "exactly one Random(5) draw");

        // Same material at the slide target → absorb without any draw.
        let mut engine = pxs_engine(13, materials);
        engine.set_landscape(
            Landscape::with_default_material(3, vec![11, 10, 11], Some(earth)).test_value(),
        );
        let mirror = engine.rng.clone();
        let (mut x, mut y) = (1, 9);
        let mut xdir = math::itofix(1);
        let mut ydir = math::C4Fixed::ZERO;
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            sand,
            Some(sand),
            &mut pos_changed,
        );
        assert!(!insert_ok);
        assert_eq!((x, y), (0, 10), "absorbed at the slide target");
        assert_eq!(xdir, math::C4Fixed::ZERO);
        assert_eq!(engine.rng, mirror, "no synced draws on same-mat slide");
    }

    #[test]
    fn user_insert_reaction_with_check_slide_off_skips_insert_check_like_cpp() {
        // mrfUserCheck (C4Material.cpp:612-625): user-defined reactions run
        // the splash/slide check only when CheckSlide is set (fInsertionCheck,
        // default true); the mrfInsert body's own check is `!fUserDefined`-
        // gated (C4Material.cpp:783-787). With CheckSlide=0 the insert
        // proceeds even where a slide path would otherwise keep the PXS
        // alive — and no slide RNG is drawn.
        let materials = materials_pxs_with_earth(
            r#"
            [Material Sand]
            Name=Sand
            Density=25
            Friction=10
            MaxSlide=2
            SplashRate=0

            [Reaction]
            Type=Insert
            TargetSpec=Earth
            CheckSlide=0
        "#,
        );
        let [sand, earth] = pxs_material_ids(&materials, ["Sand", "Earth"]);

        // Same slide-available terrain as the builtin-insert test: without
        // CheckSlide the slide would keep the PXS alive; with CheckSlide=0
        // the user reaction inserts immediately.
        let mut engine = pxs_engine(21, materials);
        engine.set_landscape(
            Landscape::with_default_material(5, vec![11, 10, 10, 10, 11], Some(earth)).test_value(),
        );
        let mirror = engine.rng.clone();
        assert!(create_test_pxs(&mut engine, sand, 2, 9, 0, 1));
        engine.tick_pxs();
        assert_eq!(
            engine.pxs_system.iter().count(),
            0,
            "CheckSlide=0 inserts without the slide check"
        );
        assert_eq!(engine.rng, mirror, "no splash/slide draws");
    }

    #[test]
    fn masked_pxs_move_custom_reaction_keeps_pxs_moving_without_insert_or_rng() {
        // C++ installs one custom-reaction slot for the pair. ExecMask=4
        // excludes meePXSMove, so mrfUserCheck returns false and the PXS
        // continues through Water. Falling back to the natural equal-density
        // Insert reaction would instead kill the PXS and write Mist at y=2.
        let materials = materials_pxs_test_materials(
            r#"
            [Material Mist]
            Name=Mist
            Density=25
            Friction=0
            MaxSlide=0

            [Reaction]
            Type=Poof
            TargetSpec=Water
            ExecMask=4

            [Material Water]
            Name=Water
            Density=25
            Friction=0
            MaxSlide=0
            "#,
        );
        let [mist, water] = pxs_material_ids(&materials, ["Mist", "Water"]);
        let reaction = materials.reaction_for_event(
            Some(mist),
            Some(water),
            material::MaterialInteractionEvent::PxsMove,
        );
        assert_eq!(reaction.kind, material::MaterialReactionKind::None);
        assert!(
            reaction.user_defined,
            "the masked pair slot remains occupied"
        );

        let mut bytes = vec![0u8; 7 * 8];
        bytes[3 * 7 + 3] = 10;
        let grid = pxs_grid(7, 8, bytes, &[(10, 25, "Water"), (20, 25, "Mist")]);
        let world = pxs_grid_world(7, 8, vec![8; 7], grid);

        let mut engine = pxs_engine(22, materials);
        engine.set_landscape(world);
        engine.set_physics(PhysicsSettings::new(0, 12, -20));
        assert!(engine
            .pxs_system
            .create(mist, itofix(3), itofix(2), C4Fixed::ZERO, itofix(1),));
        let mirror = engine.rng.clone();

        engine.tick_pxs();

        let survivors = engine.pxs_system.iter().copied().collect::<Vec<_>>();
        assert_eq!(survivors.len(), 1, "masked reaction keeps the PXS alive");
        assert_eq!(survivors[0].mat, mist);
        assert_eq!((fixtoi(survivors[0].x), fixtoi(survivors[0].y)), (3, 3));
        assert_eq!(engine.rng, mirror, "masked mrfUserCheck draws no RNG");
        let world = engine.landscape().test_value();
        assert_eq!(world.material_at(3, 2), None, "no builtin InsertMaterial");
        assert_eq!(world.grid_byte_at(3, 2), Some(0));
        assert_eq!(world.material_at(3, 3), Some(water));
        assert_eq!(world.grid_byte_at(3, 3), Some(10));
    }

    #[test]
    fn failed_pxs_corrosion_uses_full_insert_material_slide_like_cpp() {
        // C++ mrfCorrode's failed-corrosion branch calls the full
        // C4Landscape::InsertMaterial routine (C4Material.cpp:737-740).
        // InsertMaterial follows FindMatSlide and re-creates a falling PXS
        // at an open ledge (C4Landscape.cpp:1179-1184); directly extending a
        // landscape column at the contact point loses that droplet.
        let materials = materials_pxs_with_earth(
            r#"
            [Material Acid]
            Name=Acid
            Density=25
            Friction=0
            MaxSlide=10

            [Reaction]
            Type=Corrode
            TargetSpec=Earth
            CheckSlide=0
            CorrosionRate=0
        "#,
        );
        let [acid, earth] = pxs_material_ids(&materials, ["Acid", "Earth"]);
        let mut engine = pxs_engine(21, materials);

        // Earth floor through x=6, with an open drop at x=7. The acid PXS
        // hits Earth at (3, 6); failed corrosion inserts from (3, 5), whose
        // nearest valid slide is the open ledge at x=7.
        let mut bytes = vec![0u8; 12 * 12];
        for y in 6..12 {
            for x in 0..=6 {
                bytes[y * 12 + x] = 30;
            }
        }
        let grid = pxs_grid(12, 12, bytes, &[(30, 100, "Earth")]);
        let mut world = Landscape::with_default_material(12, vec![6; 12], Some(earth)).test_value();
        world.set_world_height(12);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        assert!(create_test_pxs(&mut engine, acid, 3, 5, 0, 1));
        engine.tick_pxs();

        let survivors: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(survivors.len(), 1, "InsertMaterial re-created the droplet");
        assert_eq!(
            fixtoi(survivors[0].x),
            7,
            "failed-corrosion liquid slid to the open ledge"
        );
    }

    #[test]
    fn failed_pxs_incineration_uses_full_insert_material_and_preserves_ift() {
        // C++ mrfIncinerate calls the full C4Landscape::InsertMaterial when
        // a PXSMove ignition fails (C4Material.cpp:758-767). Equal-density
        // Oil makes InsertMaterial climb to the sky pixel at (3, 5), where
        // it performs exactly one SetPix while preserving that pixel's IFT.
        // The column helper instead raises the surface to y=8 and rewrites
        // the intervening raster band.
        let materials = materials_pxs_test_materials(
            r#"
            [Material Lava]
            Name=Lava
            Density=25
            Friction=0
            MaxSlide=0
            SplashRate=100
            Incindiary=1

            [Material Oil]
            Name=Oil
            Density=25
            Friction=0
            MaxSlide=0
            Inflammable=1
        "#,
        );
        let [lava, oil] = pxs_material_ids(&materials, ["Lava", "Oil"]);
        let mut engine = pxs_engine(23, materials);
        engine.register_test_script_definition(FIRE_DEFINITION_ID, "Fire", "");
        let blocking_flame = engine.spawn_test_object(
            SpawnConfig::new(FIRE_DEFINITION_ID)
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(3, 7)),
        );
        assert!(engine
            .object_snapshot(blocking_flame)
            .expect("blocking FLAM remains live")
            .status
            .is_active());

        let mut bytes = vec![0u8; 9 * 12];
        for y in 6..12 {
            for x in 0..9 {
                bytes[y * 9 + x] = 20;
            }
        }
        bytes[5 * 9 + 3] = 0x80;
        let grid = pxs_grid(9, 12, bytes, &[(10, 25, "Lava"), (20, 25, "Oil")]);
        let mut world = Landscape::with_default_material(9, vec![6; 9], Some(oil)).test_value();
        world.set_world_height(12);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        assert!(create_test_pxs(&mut engine, lava, 3, 7, 0, 2));
        let mut mirror = engine.rng.clone();
        assert_eq!(mirror.random(100), 63, "failed splash gate draw");
        assert_eq!(mirror.random(25), 0, "incendiary smoke gate draw");
        assert_eq!(mirror.rnd3(), 1, "smoke level draw");

        engine.tick_pxs();

        assert_eq!(engine.rng, mirror, "exact mrfInsertCheck RNG ledger");
        assert_eq!(engine.pxs_system.count(), 0, "original PXS is dead");
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| {
                    !object.destroyed
                        && object.state.status.is_active()
                        && object.definition_id == FIRE_DEFINITION_ID
                })
                .count(),
            1,
            "the existing FLAM blocks a second ignition"
        );

        let world = engine.landscape().test_value();
        let grid = world.pixel_grid().test_value();
        assert_eq!(
            grid.bytes()[5 * 9 + 3],
            10 | 0x80,
            "single SetPix keeps IFT"
        );
        assert_eq!(grid.bytes()[6 * 9 + 3], 20, "Oil above contact is intact");
        assert_eq!(grid.bytes()[7 * 9 + 3], 20, "contact Oil is intact");
        assert_eq!(world.material_at(3, 5), Some(lava));
        assert_eq!(world.surface_height(3), Some(6), "no column-band rewrite");
    }

    #[test]
    fn insert_material_reaction_probe_follows_negative_gravity_like_cpp() {
        // C4Landscape::InsertMaterial probes the reaction at
        // `ty + Sign(GravAccel)` and passes that same coordinate to the
        // reaction (C4Landscape.cpp:1185-1193). With negative gravity the
        // material ABOVE the insertion point reacts; the material below
        // must not be substituted for it.
        let materials = materials_pxs_with_earth(
            r#"
            [Material Acid]
            Name=Acid
            Density=25
            Friction=0
            MaxSlide=0

            [Reaction]
            Type=Poof
            TargetSpec=Oil
            CheckSlide=0

            [Material Oil]
            Name=Oil
            Density=100
            Friction=20
            MaxSlide=0
        "#,
        );
        let [acid, oil, earth] = pxs_material_ids(&materials, ["Acid", "Oil", "Earth"]);
        let mut engine = pxs_engine(7, materials);
        engine.set_physics(PhysicsSettings::new(-1, 12, -20));

        // Oil at (3,4), sky insertion point (3,5), Earth below at (3,6).
        // MaxSlide=0 and the solid floor force InsertMaterial into its
        // reaction step without first re-creating a falling PXS.
        let mut bytes = vec![0u8; 7 * 10];
        bytes[4 * 7 + 3] = 20;
        for y in 6..10 {
            for x in 0..7 {
                bytes[y * 7 + x] = 30;
            }
        }
        let grid = pxs_grid(7, 10, bytes, &[(20, 100, "Oil"), (30, 100, "Earth")]);
        let mut world = Landscape::with_default_material(7, vec![6; 7], Some(earth)).test_value();
        world.set_world_height(10);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        assert_eq!(
            engine.landscape().and_then(|world| world.material_at(3, 4)),
            Some(oil)
        );
        engine.apply_landscape_operations(vec![LandscapeOperation::InsertMaterial {
            material: acid.index() as i32,
            position: Vector2::new(3, 5),
            velocity: Vector2::new(0, 0),
        }]);

        let world = engine.landscape().test_value();
        assert_eq!(world.material_at(3, 4), None, "Oil above was poofed");
        assert_eq!(
            world.material_at(3, 5),
            None,
            "the reacting Acid was not inserted"
        );
        assert_eq!(engine.pxs_system.count(), 0);
    }

    fn insert_material_convert_fixture(convert_to: &str) -> (Engine, MaterialId, MaterialId) {
        let library = MaterialLibrary::parse(&format!(
            r#"
            [Material Snow]
            Name=Snow
            Density=25
            Friction=0
            MaxSlide=0
            InMatConvert=Water
            InMatConvertTo={convert_to}
            InMatConvertDepth=2

            [Material Water]
            Name=Water
            Density=100
            Friction=0
            MaxSlide=0
            "#
        ))
        .test_value();
        let materials = MaterialSet::from_resource_library(&library);
        let [snow, water] = pxs_material_ids(&materials, ["Snow", "Water"]);
        let mut engine = pxs_engine(21, materials);
        engine.set_physics(PhysicsSettings::new(1, 12, -20));

        // Insert at (3,5): Water below selects the reaction and Water at
        // y-depth satisfies hardcoded InMatConvert's depth check.
        let mut bytes = vec![0u8; 7 * 10];
        bytes[3 * 7 + 3] = 20;
        bytes[6 * 7 + 3] = 20;
        let grid = pxs_grid(7, 10, bytes, &[(10, 25, "Snow"), (20, 100, "Water")]);
        let world = pxs_grid_world(7, 10, vec![10; 7], grid);
        engine.set_landscape(world);

        (engine, snow, water)
    }

    fn insert_material_thrust_fixture(
        landscape_insert_thrust: bool,
    ) -> (Engine, MaterialId, MaterialId, MaterialId) {
        let materials = materials_pxs_test_materials(
            r#"
            [Material Source]
            Name=Source
            Density=50
            Friction=0
            MaxSlide=0

            [Material Old]
            Name=Old
            Density=25
            Friction=0
            MaxSlide=0

            [Material Support]
            Name=Support
            Density=100
            Friction=0
            MaxSlide=0
            "#,
        );
        let [source, old, support] = pxs_material_ids(&materials, ["Source", "Old", "Support"]);

        // Source overwrites lower-density Old at (3,5). Dense Support at
        // (3,6) prevents the FindMatSlide/PXS path, so the only optional
        // write is Old recursively reinserted at (3,4).
        let mut bytes = vec![0u8; 7 * 10];
        bytes[5 * 7 + 3] = 20;
        bytes[6 * 7 + 3] = 30;
        let grid = pxs_grid(
            7,
            10,
            bytes,
            &[(10, 50, "Source"), (20, 25, "Old"), (30, 100, "Support")],
        );
        let world = pxs_grid_world(7, 10, vec![10; 7], grid);

        let mut engine = pxs_engine(24, materials);
        engine.set_physics(PhysicsSettings::new(1, 12, -20));
        engine.set_landscape_insert_thrust(landscape_insert_thrust);
        engine.set_landscape(world);
        (engine, source, old, support)
    }

    fn insert_source_over_old(engine: &mut Engine, source: MaterialId) {
        engine.apply_landscape_operations(vec![LandscapeOperation::InsertMaterial {
            material: source.index() as i32,
            position: Vector2::new(3, 5),
            velocity: Vector2::ZERO,
        }]);
    }

    #[test]
    fn insert_material_without_landscape_insert_thrust_drops_displaced_pixel() {
        let (mut engine, source, old, support) = insert_material_thrust_fixture(false);

        insert_source_over_old(&mut engine, source);

        let world = engine.landscape().test_value();
        assert_eq!(world.material_at(3, 5), Some(source));
        assert_eq!(world.grid_byte_at(3, 5), Some(10));
        assert_eq!(
            world.material_at(3, 4),
            None,
            "LandscapeInsertThrust=0 must not recursively reinsert Old"
        );
        assert_eq!(world.grid_byte_at(3, 4), Some(0));
        assert_eq!(world.material_at(3, 6), Some(support));
        assert_ne!(world.material_at(3, 4), Some(old));
        assert_eq!(engine.pxs_system.count(), 0);
    }

    #[test]
    fn insert_material_with_landscape_insert_thrust_reinserts_displaced_pixel() {
        let (mut engine, source, old, support) = insert_material_thrust_fixture(true);

        insert_source_over_old(&mut engine, source);

        let world = engine.landscape().test_value();
        assert_eq!(world.material_at(3, 5), Some(source));
        assert_eq!(world.grid_byte_at(3, 5), Some(10));
        assert_eq!(
            world.material_at(3, 4),
            Some(old),
            "LandscapeInsertThrust=1 preserves the recursive thrust behavior"
        );
        assert_eq!(world.grid_byte_at(3, 4), Some(20));
        assert_eq!(world.material_at(3, 6), Some(support));
        assert_eq!(engine.pxs_system.count(), 0);
    }

    #[test]
    fn insert_material_convert_writes_back_material_before_dead_pixel_insert() {
        // mrfConvert returns false after changing iPxsMat, so InsertMaterial's
        // final SetPix must use the converted material passed back by ref
        // (C4Landscape.cpp:1198-1218; C4Material.cpp:636-660).
        let (mut engine, snow, water) = insert_material_convert_fixture("Water");
        let mirror = engine.rng.clone();
        engine.apply_landscape_operations(vec![LandscapeOperation::InsertMaterial {
            material: snow.index() as i32,
            position: Vector2::new(3, 5),
            velocity: Vector2::new(0, 0),
        }]);

        let world = engine.landscape().test_value();
        assert_eq!(world.material_at(3, 5), Some(water));
        assert_eq!(
            world.grid_byte_at(3, 5),
            Some(20),
            "Water default mattex byte"
        );
        assert_eq!(
            engine.pxs_system.count(),
            0,
            "converted material is dead-inserted"
        );
        assert_eq!(engine.rng, mirror, "conversion and write-back draw no RNG");
    }

    #[test]
    fn insert_material_script_writes_back_position_before_dead_pixel_insert() {
        // A falsy mrfScript result keeps the material but writes X/Y back by
        // reference before InsertMaterial captures omat and calls SetPix.
        let materials = materials_pxs_test_materials(
            r#"
            [Material Sand]
            Name=Sand
            Density=25
            Friction=0
            MaxSlide=0

            [Reaction]
            Type=Script
            ScriptFunc=MoveInsertedMaterial
            TargetSpec=Earth
            CheckSlide=0
            ExecMask=1

            [Material Earth]
            Name=Earth
            Density=100
            Friction=0
            MaxSlide=0

            [Material Old]
            Name=Old
            Density=25
            Friction=0
            MaxSlide=0
            "#,
        );
        let [sand, old] = pxs_material_ids(&materials, ["Sand", "Old"]);
        let mut engine = pxs_engine(22, materials);
        engine.set_physics(PhysicsSettings::new(1, 12, -20));
        engine.set_landscape_insert_thrust(true);
        engine
            .install_scenario_script(
                "Scenario",
                r#"
                global func MoveInsertedMaterial(&x, &y, lsx, lsy, &xdir, &ydir, &pxs_mat, ls_mat, event)
                {
                    if (event == 0) { x = 4; y = 4; }
                    return 0;
                }
                "#,
            ).test_value();

        let mut bytes = vec![0u8; 7 * 10];
        bytes[6 * 7 + 2] = 30;
        bytes[4 * 7 + 4] = 20;
        let grid = pxs_grid(
            7,
            10,
            bytes,
            &[(10, 25, "Sand"), (20, 25, "Old"), (30, 100, "Earth")],
        );
        let world = pxs_grid_world(7, 10, vec![10; 7], grid);
        engine.set_landscape(world);
        let mirror = engine.rng.clone();

        engine.apply_landscape_operations(vec![LandscapeOperation::InsertMaterial {
            material: sand.index() as i32,
            position: Vector2::new(2, 5),
            velocity: Vector2::new(0, 0),
        }]);

        let world = engine.landscape().test_value();
        assert_eq!(
            world.material_at(2, 5),
            None,
            "original position stays empty"
        );
        assert_eq!(world.grid_byte_at(2, 5), Some(0));
        assert_eq!(world.material_at(4, 4), Some(sand));
        assert_eq!(
            world.grid_byte_at(4, 4),
            Some(10),
            "Sand default mattex byte"
        );
        assert_eq!(
            world.material_at(4, 3),
            Some(old),
            "thrust captures old material at the script-assigned destination"
        );
        assert_eq!(
            engine.pxs_system.count(),
            0,
            "script result is dead-inserted"
        );
        assert_eq!(engine.rng, mirror, "falsy script write-back draws no RNG");
    }

    #[test]
    fn insert_material_convert_to_unloaded_or_sky_kills_without_dead_insert() {
        // An unloaded/Sky conversion target makes mrfConvert return true;
        // InsertMaterial must exit before the dead-pixel write.
        for convert_to in ["Sky", "Missing"] {
            let (mut engine, snow, _water) = insert_material_convert_fixture(convert_to);
            let mirror = engine.rng.clone();
            engine.apply_landscape_operations(vec![LandscapeOperation::InsertMaterial {
                material: snow.index() as i32,
                position: Vector2::new(3, 5),
                velocity: Vector2::new(0, 0),
            }]);

            let world = engine.landscape().test_value();
            assert_eq!(world.material_at(3, 5), None, "target {convert_to}");
            assert_eq!(world.grid_byte_at(3, 5), Some(0), "target {convert_to}");
            assert_eq!(engine.pxs_system.count(), 0, "target {convert_to}");
            assert_eq!(engine.rng, mirror, "target {convert_to} draws no RNG");
        }
    }

    fn pxs_pos_material_refresh_engine(
        reaction: &str,
        trigger_at_pixel: bool,
    ) -> (Engine, MaterialId, MaterialId) {
        let library = MaterialLibrary::parse(&format!(
            r#"
            [Material Source]
            Name=Source
            Density=1
            WindDrift=0
            Friction=0

            {reaction}

            [Material Target]
            Name=Target
            Density=25
            WindDrift=40
            Friction=0

            [Material Trigger]
            Name=Trigger
            Density=10
            Friction=0
            "#
        ))
        .test_value();
        let materials = MaterialSet::from_resource_library(&library);
        let [source, target] = pxs_material_ids(&materials, ["Source", "Target"]);

        let mut engine = pxs_engine(21, materials);
        engine.set_physics(PhysicsSettings::new(0, 12, -20));
        engine.set_environment(EnvironmentSettings::new(80));

        // The PXS sits at (2,2); its current cell optionally triggers the
        // PXSPos reaction, while the cell below always has density 10.
        let mut bytes = vec![0u8; 5 * 6];
        if trigger_at_pixel {
            bytes[2 * 5 + 2] = 10;
        }
        bytes[3 * 5 + 2] = 10;
        let grid = pxs_grid(5, 6, bytes, &[(10, 10, "Trigger")]);
        let world = pxs_grid_world(5, 6, vec![6; 5], grid);
        engine.set_landscape(world);
        assert!(create_test_pxs(&mut engine, source, 2, 2, 0, 0));

        (engine, source, target)
    }

    #[test]
    fn pxs_pos_convert_refreshes_material_physics_for_same_tick() {
        let (mut engine, _source, target) = pxs_pos_material_refresh_engine(
            r#"
            [Reaction]
            Type=Convert
            TargetSpec=Trigger
            ConvertMat=Target
            CheckSlide=0
            ExecMask=1
            "#,
            true,
        );
        let mut mirror = engine.rng.clone();
        let random_x = mirror.random(1200);
        let random_y = mirror.random(1200);
        let txdir = math::itofix_prec(80, 15) + math::fixed256(random_x - 600);
        let tydir = math::fixed256(random_y - 600);
        let factor = math::itofix_prec(1, 800);

        engine.tick_pxs();

        let pixels: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(pixels.len(), 1);
        assert_eq!(pixels[0].mat, target, "PXSPos converted the material");
        assert_eq!(
            pixels[0].xdir,
            txdir * 20 * factor,
            "same-tick drift uses Target WindDrift=40"
        );
        assert_eq!(pixels[0].ydir, tydir * 20 * factor);
        assert_eq!(
            engine.rng, mirror,
            "Target Density=25 over density 10 takes both Random(1200) draws"
        );
    }

    #[test]
    fn pxs_pos_no_conversion_keeps_source_material_physics() {
        let (mut engine, source, _target) = pxs_pos_material_refresh_engine(
            r#"
            [Reaction]
            Type=Convert
            TargetSpec=Trigger
            ConvertMat=Target
            CheckSlide=0
            ExecMask=1
            "#,
            false,
        );
        let mirror = engine.rng.clone();

        engine.tick_pxs();

        let pixels: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(pixels.len(), 1);
        assert_eq!(pixels[0].mat, source);
        assert_eq!(pixels[0].xdir, math::C4Fixed::ZERO);
        assert_eq!(pixels[0].ydir, math::C4Fixed::ZERO);
        assert_eq!(
            engine.rng, mirror,
            "Source Density=1 over density 10 stays out of free fall"
        );
    }

    #[test]
    fn pxs_pos_system_global_script_writeback_refreshes_material_physics_for_same_tick() {
        let (mut engine, _source, target) = pxs_pos_material_refresh_engine(
            r#"
            [Reaction]
            Type=Script
            ScriptFunc=RewritePxsMat
            TargetSpec=Trigger
            CheckSlide=0
            ExecMask=1
            "#,
            true,
        );
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/PxsPosReaction.c".to_string(),
                format!(
                    r#"
                    global func RewritePxsMat(&x, &y, lsx, lsy, &xdir, &ydir, &pxs_mat, ls_mat, event) {{
                        if (event == 0) {{ pxs_mat = {}; }}
                        return 0;
                    }}
                    "#,
                    target.index()
                ),
            )]),
            1,
            "System.c4g PxsPos callback installs without a scenario host"
        );
        let mut mirror = engine.rng.clone();
        let random_x = mirror.random(1200);
        let random_y = mirror.random(1200);
        let txdir = math::itofix_prec(80, 15) + math::fixed256(random_x - 600);
        let tydir = math::fixed256(random_y - 600);
        let factor = math::itofix_prec(1, 800);

        engine.tick_pxs();

        let pixels: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(pixels.len(), 1);
        assert_eq!(pixels[0].mat, target, "script PxsMat writes back");
        assert_eq!(pixels[0].xdir, txdir * 20 * factor);
        assert_eq!(pixels[0].ydir, tydir * 20 * factor);
        assert_eq!(
            engine.rng, mirror,
            "script Target Density=25 takes both same-tick jitter draws"
        );
    }

    #[test]
    fn user_convert_reaction_fires_on_pxs_move_like_cpp() {
        // mrfConvert: hardcoded InMatConvert has no collision proc, but
        // USER-defined conversions also convert upon hitting materials —
        // meePXSMove falls through to the conversion logic
        // (C4Material.cpp:629-634). On success the dirs zero and the pixel
        // lives on as the converted material (pfPosChanged snaps it,
        // C4PXS.cpp:106-112).
        let materials = materials_pxs_with_earth(
            r#"
            [Material Slime]
            Name=Slime
            Density=25
            Friction=10

            [Reaction]
            Type=Convert
            TargetSpec=Earth
            ConvertMat=Water
            CheckSlide=0

            [Material Water]
            Name=Water
            Density=25
            Friction=0
        "#,
        );
        let [slime, water, earth] = pxs_material_ids(&materials, ["Slime", "Water", "Earth"]);

        let mut engine = pxs_engine(21, materials);
        // World bottom below the ground surface: the y=10 contact row is
        // EARTH — at the world bottom it would be the closed border, which
        // reads Vehicle in C++ (GetPix, C4Landscape.h:157-159).
        let mut world = Landscape::flat_with_material(5, 10, Some(earth));
        world.set_world_height(20);
        engine.set_landscape(world);
        let mirror = engine.rng.clone();
        assert!(create_test_pxs(&mut engine, slime, 2, 9, 0, 1));
        engine.tick_pxs();
        let survivors: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(survivors.len(), 1, "converted pixel lives on");
        assert_eq!(survivors[0].mat, water, "converted to Water on hit");
        assert_eq!(survivors[0].xdir, math::C4Fixed::ZERO);
        assert_eq!(survivors[0].ydir, math::C4Fixed::ZERO);
        assert_eq!(engine.rng, mirror, "no draws on the conversion path");
    }

    #[test]
    fn pxs_reacts_with_the_vehicle_border_at_closed_sides_like_cpp() {
        // GBackMat reads MCVehic → Vehicle past a closed side
        // (C4Landscape.h:144-161, GetMat :173-176), so a PXS pushing into
        // the border hits DefReactInsert vs Vehicle (liquid density 25 <=
        // vehicle 100 → mrfInsert, C4Material.cpp:773-798): mrfInsertCheck
        // finds no slide against the wall and InsertMaterial deactivates
        // the PXS in place — it must NOT walk out of bounds and die
        // draw-free like against sky.
        let materials = materials_pxs_test_materials(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25

            [Material Water]
            Name=Water
            Density=25
            Friction=0
            SplashRate=0
            MaxSlide=3

            [Material Vehicle]
            Name=Vehicle
            Density=100
            Friction=100
        "#,
        );
        let [water, earth] = pxs_material_ids(&materials, ["Water", "Earth"]);
        let mut engine = pxs_engine(9, materials);
        engine.set_landscape(Landscape::flat_with_material(6, 6, Some(earth)));

        // Per-pixel world: earth from y=6 down, sky above (the audit bug
        // lives on the grid path — material_at answers None past the
        // sides there).
        let mut bytes = vec![0u8; 6 * 12];
        for y in 6..12 {
            for x in 0..6 {
                bytes[y * 6 + x] = 30;
            }
        }
        let grid = pxs_grid(6, 12, bytes, &[(20, 25, "Water"), (30, 100, "Earth")]);
        let mut world = Landscape::with_default_material(6, vec![6; 6], Some(earth)).test_value();
        world.set_world_height(12);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        // Sitting on the ground in the border column, pushing left.
        let mirror = engine.rng.clone();
        assert!(create_test_pxs(&mut engine, water, 0, 5, -2, 0));
        engine.tick_pxs();
        assert_eq!(
            engine.pxs_system.iter().count(),
            0,
            "border contact inserts and deactivates the PXS"
        );
        let landscape = engine.landscape().test_value();
        assert_eq!(
            landscape.material_at(0, 5),
            Some(water),
            "InsertMaterial landed the pixel against the border"
        );
        assert_eq!(engine.rng, mirror, "no synced draws on this path");
    }

    #[test]
    fn pxs_created_mid_execute_never_takes_the_executing_slot() {
        // C4PXSSystem::Execute runs each PXS IN PLACE (C4PXS.cpp:218-240):
        // while a PXS executes, its slot still carries Mat != MNone, so a
        // PXS created inside a reaction (InsertMaterial's slide loop
        // re-creates the droplet, C4Landscape.cpp:1192-1196) can never be
        // handed that slot by New() (C4PXS.cpp:195-202). Only after the
        // reaction kills the pixel does Deactivate free it — the droplet
        // must land in the NEXT slot, keeping the deterministic
        // chunk-major execution order.
        let materials = materials_pxs_test_materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
            SplashRate=0
            MaxSlide=10
        "#,
        );
        let water = materials.id_of("Water").test_value();
        let mut engine = pxs_engine(3, materials);

        // Pixel world 12x12: a water pool in columns 0..=6 (rows 6..11)
        // with a step down to open air at column 7 — the insert slide
        // finds the ledge and re-creates the pixel as a droplet there.
        let mut bytes = vec![0u8; 12 * 12];
        for y in 6..12 {
            for x in 0..=6 {
                bytes[y * 12 + x] = 20;
            }
        }
        let grid = pxs_grid(12, 12, bytes, &[(20, 25, "Water")]);
        let mut world = Landscape::with_default_material(12, vec![6; 12], Some(water)).test_value();
        world.set_world_height(12);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        // Slot 0: a submerged water PXS moving down — the water-vs-water
        // move contact inserts (killing it) and the insert slides to the
        // ledge, creating the droplet DURING slot 0's execution.
        assert!(create_test_pxs(&mut engine, water, 3, 7, 0, 1));
        engine.tick_pxs();

        let survivors: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(survivors.len(), 1, "only the droplet survives");
        assert_eq!(fixtoi(survivors[0].x), 7, "droplet sits on the ledge");
        assert_eq!(
            engine.pxs_system.count(),
            1,
            "chunk counts stay exact through the mid-execute create"
        );

        // The executing slot 0 freed on death; the droplet occupies slot 1
        // (C++ New() skipped the still-live slot 0). The next create must
        // reuse slot 0 and execute BEFORE the droplet.
        assert!(create_test_pxs(&mut engine, water, 9, 2, 0, 0));
        let order: Vec<i32> = engine.pxs_system.iter().map(|pxs| fixtoi(pxs.x)).collect();
        assert_eq!(
            order,
            [9, 7],
            "slot 0 was free during the droplet's creation only in C++ terms — \
             the droplet must sit in slot 1"
        );
    }

    #[test]
    fn snapshot_restore_preserves_pxs_slot_layout_verbatim() {
        // C4PXSSystem::Save writes whole chunks including their MNone gaps
        // (C4PXS.cpp:346-349) and Load re-counts them in place
        // (C4PXS.cpp:383-397): slot POSITIONS survive save/load, so slot
        // reuse — and with it the deterministic execution order — stays
        // lockstep across a save/load boundary. Compacting the layout on
        // restore hands later creates different slots than C++.
        let materials = materials_pxs_test_materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
        "#,
        );
        let water = materials.id_of("Water").test_value();
        let mut engine = pxs_engine(5, materials);

        for x in 0..3 {
            assert!(create_test_pxs(&mut engine, water, x, 0, 0, 0));
        }
        // Kill the middle pixel: slot 1 becomes an MNone gap.
        engine.pxs_system.clear_slot(0, 1);

        let state = engine.capture_state();
        engine.restore_state(&state).test_value();

        // The gap must survive: the next create reuses slot 1, keeping
        // chunk-major order [slot0, slot1, slot2] = [0, 9, 2].
        assert!(create_test_pxs(&mut engine, water, 9, 0, 0, 0));
        let order: Vec<i32> = engine.pxs_system.iter().map(|pxs| fixtoi(pxs.x)).collect();
        assert_eq!(order, [0, 9, 2], "restore keeps the MNone gap at slot 1");
    }

    #[test]
    fn script_reaction_calls_system_global_function_and_kills_on_truthy_return() {
        // mrfScript (C4Material.cpp:800-835): the function gets the 9-int
        // parameter set — X, Y, LSPosX, LSPosY, fixtoi(XDir,100),
        // fixtoi(YDir,100), PxsMat, LsMat, Event — and a truthy return
        // deactivates the PXS (C4Material.cpp:818). An unresolvable name is
        // a no-op (null pScriptFunc, C4Material.cpp:809-811).
        let materials = materials_pxs_with_earth(
            r#"
            [Material Goo]
            Name=Goo
            Density=25
            Friction=10

            [Reaction]
            Type=Script
            ScriptFunc=GooHitsEarth
            TargetSpec=Earth
            CheckSlide=0

            [Material Ooze]
            Name=Ooze
            Density=25
            Friction=10

            [Reaction]
            Type=Script
            ScriptFunc=NoSuchFunction
            TargetSpec=Earth
            CheckSlide=0
        "#,
        );
        let [goo, ooze, earth] = pxs_material_ids(&materials, ["Goo", "Ooze", "Earth"]);

        let mut engine = pxs_engine(21, materials);
        // World bottom below the ground surface: the y=10 contact row is
        // EARTH — at the world bottom it would be the closed border, which
        // reads Vehicle in C++ (GetPix, C4Landscape.h:157-159).
        let mut world = Landscape::flat_with_material(5, 10, Some(earth));
        world.set_world_height(20);
        engine.set_landscape(world);
        // The reaction function records its parameters in a global effect
        // variable store via AddEffect... keep it simpler: return the kill
        // flag computed from the parameters so the call is observable both
        // ways (kill for goo's column, survive elsewhere is not reachable
        // in this fixture — the unresolvable arm covers the no-op path).
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/MaterialReaction.c".to_string(),
                r#"
                global func GooHitsEarth(x, y, lsx, lsy, xdir, ydir, pxs_mat, ls_mat, event) {
                    // meePXSMove = 1; falling straight down: xdir 0, ydir 100
                    if (event != 1) { return 0; }
                    if (xdir != 0) { return 0; }
                    if (ydir != 100) { return 0; }
                    if (ls_mat < 0) { return 0; }
                    return 1;
                }
                "#
                .to_string(),
            )]),
            1,
            "System.c4g reaction script installs without a scenario host"
        );
        engine
            .install_scenario_script("Scenario", "func NoSuchFunction() { return 1; }")
            .test_value();

        let mirror = engine.rng.clone();
        assert!(create_test_pxs(&mut engine, goo, 2, 9, 0, 1));
        engine.tick_pxs();
        assert_eq!(
            engine.pxs_system.iter().count(),
            0,
            "a truthy script return kills the PXS"
        );
        assert_eq!(engine.rng, mirror, "no synced draws on this path");

        // An ordinary scenario-local function is not owned by
        // Game.ScriptEngine, so GetSFuncWarn cannot resolve it: null
        // pScriptFunc leaves the PXS alive.
        assert!(create_test_pxs(&mut engine, ooze, 2, 9, 0, 1));
        engine.tick_pxs();
        assert_eq!(
            engine.pxs_system.iter().count(),
            1,
            "an unresolvable ScriptFunc leaves the PXS alive"
        );
    }

    #[test]
    fn script_reaction_writes_back_ref_params_like_cpp() {
        // mrfScript write-back (C4Material.cpp:814-832): X/Y/XDir/YDir/PxsMat
        // are passed by reference; after a falsy return, PxsMat writes back
        // UNCONDITIONALLY, and a change to any of pos/speed writes all four
        // back (dirs through the lossy FIXED100 round trip) and sets
        // pfPosChanged (→ the fStopMovement snap, C4PXS.cpp:106-112).
        let materials = materials_pxs_with_earth(
            r#"
            [Material Goo]
            Name=Goo
            Density=25
            Friction=10

            [Reaction]
            Type=Script
            ScriptFunc=Deflect
            TargetSpec=Earth
            CheckSlide=0

            [Material Water]
            Name=Water
            Density=25
            Friction=0
        "#,
        );
        let [goo, water, earth] = pxs_material_ids(&materials, ["Goo", "Water", "Earth"]);

        let mut engine = pxs_engine(21, materials);
        // World bottom below the ground surface: the y=10 contact row is
        // EARTH — at the world bottom it would be the closed border, which
        // reads Vehicle in C++ (GetPix, C4Landscape.h:157-159).
        let mut world = Landscape::flat_with_material(5, 10, Some(earth));
        world.set_world_height(20);
        engine.set_landscape(world);
        engine
            .install_scenario_script(
                "Scenario",
                r#"
                global func Deflect(&x, &y, lsx, lsy, &xdir, &ydir, &pxs_mat, ls_mat, event) {
                    xdir = 150;
                    ydir = -100;
                    pxs_mat = 1; // Water's material index
                    return 0;    // keep the pixel
                }
                "#,
            )
            .test_value();

        assert!(create_test_pxs(&mut engine, goo, 2, 9, 0, 1));
        engine.tick_pxs();
        let survivors: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(survivors.len(), 1, "falsy return keeps the pixel");
        assert_eq!(
            survivors[0].mat, water,
            "PxsMat writes back unconditionally"
        );
        assert_eq!(
            survivors[0].xdir,
            math::fixed100(150),
            "XDir writes back via FIXED100"
        );
        assert_eq!(
            survivors[0].ydir,
            math::fixed100(-100),
            "YDir writes back via FIXED100"
        );
        // pos_changed → fStopMovement: the pixel snapped to its int position
        assert_eq!(survivors[0].x, math::itofix(2));
        assert_eq!(survivors[0].y, math::itofix(9));
    }

    #[test]
    fn pxs_insert_reaction_runs_insert_check_like_cpp() {
        // mrfInsert on meePXSMove runs mrfInsertCheck before inserting
        // (C4Material.cpp:781-790): a PXS with a slide path keeps existing
        // (snapped to its int position, fStopMovement C4PXS.cpp:106-112);
        // an enclosed PXS inserts and dies.
        let materials = materials_pxs_with_earth(
            r#"
            [Material Sand]
            Name=Sand
            Density=25
            Friction=10
            MaxSlide=2
            SplashRate=0
        "#,
        );
        let [sand, earth] = pxs_material_ids(&materials, ["Sand", "Earth"]);

        // Slide available: the step loop hits earth below, mrfInsertCheck
        // finds the two-column slide, the PXS survives snapped to its int
        // position with the accelerated xdir (fStopMovement, C4PXS.cpp:106-112).
        // Default gravity stays on: Sign(GravAccel) feeds the slide direction
        // (C4Material.cpp:590) and the added ydir is small enough not to
        // shift fixtoi. SplashRate=0 keeps the splash branch draw-free.
        let mut engine = pxs_engine(21, materials.clone());
        engine.set_landscape(
            Landscape::with_default_material(5, vec![11, 10, 10, 10, 11], Some(earth)).test_value(),
        );
        let mut mirror = engine.rng.clone();
        // fXDir = (0*10 + Sign(0-2))/11 + FIXED10(Random(5)-2) (C4Material.cpp:597)
        let expected_xdir = math::C4Fixed::from_raw(math::itofix(-1).val() / 11)
            + math::fixed10(mirror.random(5) - 2);

        assert!(create_test_pxs(&mut engine, sand, 2, 9, 0, 1));
        engine.tick_pxs();
        let survivors: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(survivors.len(), 1, "slide path keeps the PXS alive");
        assert_eq!(survivors[0].x, math::itofix(2), "snapped to int position");
        assert_eq!(survivors[0].y, math::itofix(9));
        assert_eq!(survivors[0].xdir, expected_xdir);
        assert_eq!(survivors[0].ydir, math::C4Fixed::ZERO, "contact stops ydir");
        assert_eq!(engine.rng, mirror, "exactly one Random(5) draw");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.surface_height(2)),
            Some(10),
            "nothing inserted while sliding"
        );

        // Enclosed: no slide anywhere → insertion proceeds and the PXS dies
        // (C4Material.cpp:788-790). The liquid column only extends the world
        // height so the buried PXS stays in bounds (C4PXS.cpp:45-49).
        let mut engine = pxs_engine(21, materials);
        let mut landscape = Landscape::flat_with_material(5, 10, Some(earth));
        landscape.set_liquid_column(0, vec![LiquidSegment::new(25, 28)]);
        engine.set_landscape(landscape);
        let mirror = engine.rng.clone();
        assert!(create_test_pxs(&mut engine, sand, 2, 20, 0, 1));
        engine.tick_pxs();
        assert_eq!(
            engine.pxs_system.count(),
            0,
            "enclosed PXS inserts and dies"
        );
        assert_eq!(engine.rng, mirror, "no synced draws while enclosed");
    }

    #[test]
    fn apply_landscape_operations_executes_shake_circle() -> Result<(), EngineError> {
        let materials = materials_pxs_test_materials(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            DigFree=1
        "#,
        );
        let earth = materials.id_of("Earth").test_value();
        let mut engine = pxs_engine(9, materials);
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(earth)));

        engine.apply_landscape_operations(vec![LandscapeOperation::ShakeCircle {
            center: Vector2::new(8, 35),
            radius: 3,
        }]);

        let snapshot = engine.snapshot();
        assert!(
            snapshot
                .particles
                .iter()
                .any(|particle| particle.definition_id == "material/pxs/earth"),
            "shake operation should release earth particles"
        );
        Ok(())
    }

    #[test]
    fn shake_circle_clears_only_dig_free_grid_pixels_like_cpp() {
        // C4Landscape::ShakeFreePix clears DigFree material and creates a
        // PXS, but preserves other material (C4Landscape.cpp:928-938).
        let materials = materials_pxs_test_materials(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            DigFree=1

            [Material Granite]
            Name=Granite
            Density=100
        "#,
        );
        let [earth, granite] = pxs_material_ids(&materials, ["Earth", "Granite"]);
        let mut engine = pxs_engine(9, materials);

        let mut bytes = vec![0; 25];
        bytes[2 * 5 + 2] = 30;
        bytes[2 * 5 + 3] = 40;
        let grid = pxs_grid(5, 5, bytes, &[(30, 100, "Earth"), (40, 100, "Granite")]);
        let landscape = pxs_grid_world(5, 5, vec![5; 5], grid);
        engine.set_landscape(landscape);

        engine.execute_shake_circle_operation(Vector2::new(2, 2), 2);

        let landscape = engine.landscape().test_value();
        assert_eq!(landscape.material_at(2, 2), None, "DigFree Earth clears");
        assert_eq!(
            landscape.material_at(3, 2),
            Some(granite),
            "non-DigFree Granite survives"
        );
        assert_eq!(engine.pxs_system.count(), 1);
        assert_eq!(
            engine.pxs_system.iter().next().map(|pxs| pxs.mat),
            Some(earth),
            "the cleared Earth becomes one zero-velocity PXS"
        );
    }

    #[test]
    fn blast_circle_shifts_materials_with_blast_shift_to() -> Result<(), EngineError> {
        let materials = materials_pxs_test_materials(
            r#"
            [Material Granite]
            Name=Granite
            Density=110
            Friction=35
            BlastShiftTo=Earth

            [Material Earth]
            Name=Earth
            Density=90
            Friction=25
        "#,
        );
        let [granite, earth] = pxs_material_ids(&materials, ["Granite", "Earth"]);
        let mut engine = pxs_engine(29, materials);
        engine.set_landscape(Landscape::flat_with_material(25, 40, Some(granite)));

        engine
            .blast_circle(Vector2::new(12, 40), 10, None)
            .test_value();

        let landscape = engine.landscape().test_value();
        let mut shifted_columns = 0;
        for x in 0..landscape.width() as i32 {
            if landscape.solid_material_at(x) == Some(earth) {
                shifted_columns += 1;
            }
        }
        assert!(
            shifted_columns > 0,
            "expected blast to shift some columns to target material"
        );
        Ok(())
    }

    #[test]
    fn column_blast_shift_consumes_cpp_random_per_pixel_and_matches_raster() {
        // C4Landscape::BlastFreePix consumes exactly one
        // Random(BlastMatCount[mat]) for every BlastShiftTo source pixel,
        // even after an earlier pixel shifts (C4Landscape.cpp:941-960).
        // These equivalent column/raster worlds expose ten Granite pixels
        // in the complete r=2 scan, so both ledgers must advance by ten
        // identical LCG draws.
        let materials = materials_pxs_test_materials(
            r#"
            [Material Granite]
            Name=Granite
            Density=110
            Friction=35
            BlastShiftTo=Earth

            [Material Earth]
            Name=Earth
            Density=90
            Friction=25
        "#,
        );
        let granite = materials.id_of("Granite").test_value();

        let mut column_engine = Engine::with_seed(29);
        column_engine.set_materials(materials.clone());
        let mut column_world = Landscape::flat_with_material(7, 0, Some(granite));
        column_world.set_world_height(7);
        column_engine.set_landscape(column_world);

        let mut raster_engine = Engine::with_seed(29);
        raster_engine.set_materials(materials);
        let mut bytes = vec![0; 7 * 7];
        for y in 0..7 {
            bytes[y * 7..y * 7 + 7].fill(1);
        }
        let grid = landscape::PixelGrid::new(
            7,
            7,
            bytes,
            vec![0, 110, 90],
            vec![None, Some("Granite".to_owned()), Some("Earth".to_owned())],
            vec![None; 3],
        );
        let raster_world = pxs_grid_world(7, 7, vec![0; 7], grid);
        raster_engine.set_landscape(raster_world);

        let count_before = column_engine.rng.count;
        let mut mirror = column_engine.rng.clone();
        for _ in 0..10 {
            mirror.random(10);
        }

        let center = Vector2::new(3, 3);
        let column_result = column_engine.blast_circle(center, 2, None).test_value();
        let raster_result = raster_engine.blast_circle(center, 2, None).test_value();

        assert_eq!(
            column_result.pixel_count_by_material.get(&granite),
            Some(&10)
        );
        assert_eq!(
            column_result
                .shift_candidates
                .iter()
                .map(|candidate| candidate.pixel_count)
                .sum::<i32>(),
            10,
            "column approximation represents the same ten source pixels"
        );
        assert_eq!(
            column_result
                .shift_candidates
                .iter()
                .map(|candidate| candidate.column)
                .collect::<Vec<_>>(),
            vec![3, 2, 3, 1, 2, 3, 4, 2, 3, 3],
            "shift draws retain the C++ y/x scan order"
        );
        assert_eq!(
            raster_result.pixel_count_by_material.get(&granite),
            Some(&10)
        );
        assert_eq!(
            column_engine.rng.count - count_before,
            10,
            "one synced draw per BlastShiftTo pixel"
        );
        assert_eq!(
            column_engine.rng, mirror,
            "column ledger matches C++ Random"
        );
        assert_eq!(
            raster_engine.rng, mirror,
            "column and faithful raster paths finish on the same ledger"
        );
    }

    #[test]
    fn column_blast_shift_draws_for_non_mutating_shift_sources() {
        // BlastFreePix calls Random before it clears a BlastFree pixel, and
        // also calls Random when BlastShiftTo resolves to the source material.
        // Radius zero has threshold zero, so both cases still consume one draw.
        for (case, material_properties, expected_surface) in [
            ("blast-free", "BlastShiftTo=Earth\nBlastFree=1", 1),
            ("same-target", "BlastShiftTo=Granite", 0),
        ] {
            let library = MaterialLibrary::parse(&format!(
                r#"
                [Material Granite]
                Name=Granite
                Density=110
                Friction=35
                {material_properties}

                [Material Earth]
                Name=Earth
                Density=90
                Friction=25
                "#
            ))
            .test_value();
            let materials = MaterialSet::from_resource_library(&library);
            let granite = materials.id_of("Granite").test_value();

            let mut engine = Engine::with_seed(31);
            engine.set_materials(materials);
            let mut world = Landscape::flat_with_material(3, 0, Some(granite));
            world.set_world_height(3);
            engine.set_landscape(world);

            let mut mirror = engine.rng.clone();
            mirror.random(1);

            let result = engine
                .blast_circle(Vector2::new(1, 0), 0, None)
                .test_value();

            assert_eq!(
                result.pixel_count_by_material.get(&granite),
                Some(&1),
                "{case}: source pixel is pre-counted"
            );
            assert_eq!(engine.rng, mirror, "{case}: one C++ Random draw");
            let landscape = engine.landscape().test_value();
            assert_eq!(
                landscape.surface_height(1),
                Some(expected_surface),
                "{case}: BlastFree behavior is preserved"
            );
            assert_eq!(
                landscape.solid_material_at(1),
                Some(granite),
                "{case}: a non-mutating shift must not recolor the remaining column"
            );
        }
    }

    #[test]
    fn incendiary_particles_spawn_fire_without_eroding_surface() -> Result<(), EngineError> {
        let materials = materials_pxs_test_materials(
            r#"
            [Material Flame]
            Name=Flame
            Density=60
            Friction=10
            SplashRate=0
            Incindiary=100

            [Material Wood]
            Name=Wood
            Density=90
            Friction=25
            Inflammable=100
        "#,
        );
        let [flame, wood] = pxs_material_ids(&materials, ["Flame", "Wood"]);

        // Ignition happens via the meePXSPos check (C4PXS.cpp:51-57): when a
        // flame PXS's rounded position lies inside inflammable material,
        // mrfIncinerate calls Landscape.Incinerate(x, y), which reads the
        // material AT the position (C4Landscape.cpp:1430-1440). A contact
        // from above inserts at the air cell instead, in C++ too. The liquid
        // column only extends the estimated world height so the embedded PXS
        // stays in bounds (the column model has no separate map height).
        let mut engine = pxs_engine(31, materials);
        engine.register_test_definition(simple_definition(FIRE_DEFINITION_ID));
        let mut landscape = Landscape::flat_with_material(17, 80, Some(wood));
        landscape.set_liquid_column(0, vec![LiquidSegment::new(150, 160)]);
        engine.set_landscape(landscape);

        let column_x = 8;
        let before_height = engine
            .landscape()
            .test_value()
            .surface_height(column_x)
            .test_value();

        engine.pxs_system.create(
            flame,
            math::itofix(column_x),
            math::ftofix(before_height as f32 + 0.25),
            math::C4Fixed::ZERO,
            math::C4Fixed::ZERO,
        );

        engine.tick_pxs();
        let flame_spawned = engine.objects.iter().any(|object| {
            !object.destroyed
                && object.state.status.is_active()
                && object.definition_id == FIRE_DEFINITION_ID
        });
        assert!(
            flame_spawned,
            "expected a flame to spawn from the embedded PXS"
        );
        assert_eq!(engine.pxs_system.count(), 0, "ignited PXS deactivates");

        let after_height = engine
            .landscape()
            .test_value()
            .surface_height(column_x)
            .test_value();
        assert_eq!(
            after_height, before_height,
            "incineration should not erode the landscape surface"
        );

        engine.pxs_system.create(
            flame,
            math::itofix(column_x),
            math::ftofix(before_height as f32 + 0.25),
            math::C4Fixed::ZERO,
            math::C4Fixed::ZERO,
        );
        for _ in 0..3 {
            engine.tick_pxs();
        }

        let capped_flame_count = engine
            .objects
            .iter()
            .filter(|object| {
                !object.destroyed
                    && object.state.status.is_active()
                    && object.definition_id == FIRE_DEFINITION_ID
            })
            .count();
        assert_eq!(
            capped_flame_count, 1,
            "incineration should respect the fire density cap"
        );

        Ok(())
    }

    #[test]
    fn material_particles_settle_into_landscape() -> Result<(), EngineError> {
        let materials = materials_pxs_test_materials(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1
            Blast2PXSRatio=1
            SplashRate=15
        "#,
        );
        let earth = materials.id_of("Earth").test_value();
        let mut engine = pxs_engine(19, materials);
        engine.set_landscape(Landscape::flat_with_material(12, 30, Some(earth)));

        engine
            .blast_circle(Vector2::new(6, 30), 3, None)
            .test_value();

        let post_blast_surface = {
            let snapshot = engine.snapshot();
            let landscape = snapshot.landscape.as_ref().test_value();
            landscape.surface()[6]
        };
        assert!(
            post_blast_surface > 30,
            "blast should lower the surface before particles settle"
        );

        for _ in 0..24 {
            engine.tick_without_snapshot().test_value();
        }

        let snapshot = engine.snapshot();
        let landscape = snapshot.landscape.test_value();
        let final_surface = landscape.surface()[6];
        assert!(
            final_surface <= post_blast_surface + 1,
            "expected particles to prevent the crater from deepening"
        );
        assert!(
            final_surface >= 30,
            "expected final surface to remain at or above the original baseline"
        );
        Ok(())
    }

    // NOTE: the former `material_particles_apply_friction_to_objects` test
    // was removed with the C4PXS port: C++ PXS never interact with objects
    // (C4PXS.cpp has no object coupling), so the friction behavior it pinned
    // was an invention of the placeholder particle loop.

    const PASSIVE_PLAYER_SCRIPT: &str = r#"
global func Initialize(state, random)
{
    return 0;
}

global func Step(state, frame, random)
{
    return 0;
}
"#;

    const EFFECT_HOST_SCRIPT: &str = r#"#strict 3
    global func Initialize(state, random)
    {
        if (!GetEffect("Glow", this()))
        {
            return { effects = [ { op = "add", name = "Glow", priority = 150, interval = 4 } ] };
        }
        return nil;
    }

    global func Step(state, frame, random)
    {
        if (frame == 1)
        {
            return { effects = [ { op = "add", name = "Spark", priority = 60 } ] };
        }
        if (frame == 2)
        {
            var glow_number = GetEffect("Glow", this());
            var glow_priority = GetEffect("Glow", this(), 0, 2);
            var spark_priority = GetEffect("Spark", this(), 0, 2);
            var interval = GetEffect("Glow", this(), 0, 3);
            var filtered = GetEffect("Glow", this(), 0, 2, 100);
            var allowed = GetEffect("Glow", this(), 0, 2, 200);
            if (filtered)
            {
                return { energy = -1 };
            }
            return { energy = glow_number + glow_priority + spark_priority + interval + allowed };
        }
        return nil;
    }
    "#;

    const EFFECT_HOST_ADD_REMOVE_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        AddEffect("Glow", this(), 120, 3);
        AddEffect("Spark", this(), 100);
        return 0;
    }

    global func Step(state, frame, random)
    {
        if (frame == 1)
        {
            RemoveEffect("Glow", this());
        }
        if (frame == 2)
        {
            var spark_id = GetEffect("Spark", this());
            if (spark_id)
            {
                var no_name;
                RemoveEffect(no_name, this(), spark_id);
            }
        }
        return 0;
    }
    "#;

    const GLOBAL_EFFECT_HELPER_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        var no_target;
        AddEffect("WorldPulse", no_target, 80, 3);
        return 0;
    }

    global func Step(state, frame, random)
    {
        if (frame == 1)
        {
            var no_target;
            RemoveEffect("WorldPulse", no_target);
        }
        return 0;
    }
    "#;

    const MENU_COMMAND_SCRIPT: &str = r#"#strict 2
global func Initialize(state, random)
{
    return 0;
}

global func MenuCommand(state, kind, selection)
{
    if (kind == "focus")
    {
        SetR(42);
        return true;
    }
    return false;
}
"#;

    const PROCEDURE_STATE_SCRIPT: &str = r#"#strict 3
    global func Initialize(state, random)
    {
        if (state.action && state.action.procedure == "flight")
        {
            return { energy = 7 };
        }
        return { energy = -1 };
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    #[test]
    fn script_set_wind_holds_target_until_next_tick1000_evaluation() {
        let script = r#"#strict
func ForceWind()
{
    SetWind(80);
    return(GetWind(0, 0, true));
}

func ReadWind()
{
    return(GetWind(0, 0, true));
}
"#;
        let mut engine = Engine::with_seed(17);
        engine.register_test_script_definition("WIND", "Wind probe", script);

        // Keep nonzero scenario variation so the legacy runtime-field
        // compatibility path cannot mask a stale TargetWind. SetWind must
        // change only the live Wind/TargetWind pair, not scenario Wind.Std.
        let mut weather = EnvironmentSettings::new(5).with_wind_variation(4, 2_000);
        weather.wind_target = -20;
        engine.set_environment(weather);

        let probe = engine.spawn_test_object(SpawnConfig::new("WIND"));
        let probe_index = engine.test_object_index(probe);
        assert_eq!(
            engine
                .call_object_function(probe_index, "ForceWind", Vec::new())
                .expect("SetWind executes"),
            Value::Int(80)
        );
        assert_eq!(
            (engine.environment().wind, engine.environment().wind_target),
            (80, 80)
        );
        assert_eq!(engine.environment().base_wind, 5);

        for expected_frame in 1..1_000 {
            engine.tick_without_snapshot().test_value();
            let probe_index = engine.test_object_index(probe);
            assert_eq!(
                engine
                    .call_object_function(probe_index, "ReadWind", Vec::new())
                    .expect("GetWind executes"),
                Value::Int(80),
                "script-set wind decayed on frame {expected_frame}"
            );
            assert_eq!(engine.environment().wind_target, 80);
        }
        assert_eq!(engine.frame(), 999);

        engine.tick_without_snapshot().test_value();
        let weather = engine.environment();
        assert_eq!(weather.base_wind, 5);
        assert!(
            (1..=9).contains(&weather.wind_target),
            "Tick1000 must evaluate around scenario Std 5 +/- Rnd 4, got {}",
            weather.wind_target
        );
        assert_eq!(
            weather.wind, 79,
            "Tick1000 evaluates the scenario target, then steps toward it"
        );
    }

    #[test]
    fn wind_variation_adjusts_over_time() {
        // C4Weather::Execute (C4Weather.cpp:94-100): TargetWind re-evaluates
        // only on Tick1000 frames with ONE synced draw
        // (BoundBy(Std + Random(2*Rnd+1) - Rnd, Min, Max), C4SVal::Evaluate,
        // C4Scenario.cpp:43-46); the wind steps ±1 toward the target on
        // Tick10 frames.
        let mut settings = EnvironmentSettings::new(5).with_wind_variation(4, 40);
        let mut rng = LcgRng::seed_from_u64(1234);
        let mut probe = rng.clone();
        let rnd = settings.wind_variation.max(0);
        let expected_target = (settings.base_wind + probe.random(2 * rnd + 1) - rnd)
            .clamp(settings.wind_min, settings.wind_max);

        // Off-gate frames consume no draws and leave the wind unchanged.
        let before = settings;
        settings.advance_frame(&mut rng, 7);
        assert_eq!(settings.wind, before.wind);
        assert_eq!(settings.wind_target, before.wind_target);

        settings.advance_frame(&mut rng, 1000);
        assert_eq!(
            settings.wind_target, expected_target,
            "Tick1000 target evaluation"
        );
        assert_eq!(rng, probe, "exactly one synced draw");
        let stepped = (before.wind + (expected_target - before.wind).signum())
            .clamp(settings.wind_min, settings.wind_max);
        assert_eq!(settings.wind, stepped, "Tick10 step toward the target");
    }

    #[test]
    fn zero_base_wind_survives_the_initial_random_evaluation() {
        // C4Weather::Init evaluates C4S.Weather.Wind into the current Wind but
        // leaves the scenario C4SVal::Std untouched; Tick1000 evaluates around
        // that same Std even when it is zero (C4Weather.cpp:40-48,94-100;
        // C4Scenario.cpp:43-46).
        let mut settings = EnvironmentSettings::new(0).with_wind_variation(75, 2_000);
        settings.wind = -61;
        settings.wind_target = -61;
        let mut rng = LcgRng::seed_from_u64(424_242);
        let mut probe = rng.clone();
        let expected_target = (probe.random(151) - 75).clamp(-100, 100);

        settings.advance_frame(&mut rng, 1_000);

        assert_eq!(settings.base_wind, 0, "scenario Std remains authoritative");
        assert_eq!(settings.wind_target, expected_target);
        assert_eq!(rng, probe, "Tick1000 consumes exactly one synced draw");
    }

    #[test]
    fn zero_tick1000_wind_target_is_not_rewritten_during_decay() {
        // Seed 57's first post-Randomize3 draw is 70 in range 141, so the
        // scenario value 0 ± 70 evaluates to exactly zero on Tick1000.
        let mut settings = EnvironmentSettings::new(0).with_wind_variation(70, 1_000);
        settings.wind = 40;
        settings.wind_target = 40;
        let mut rng = LcgRng::seed_from_u64(57);
        let mut probe = rng.clone();
        assert_eq!(
            probe.random(141),
            70,
            "fixture evaluates TargetWind to zero"
        );

        settings.advance_frame(&mut rng, 1_000);
        assert_eq!(settings.wind_target, 0, "Tick1000 replaces TargetWind");
        assert_eq!(settings.wind, 39, "coincident Tick10 uses the new target");
        assert_eq!(settings.base_wind, 0, "scenario Wind.Std stays immutable");
        assert_eq!(rng, probe, "Tick1000 consumes exactly one synced draw");

        for frame in 1_001..=1_390 {
            // Engine construction, replacement, and restore normalize legacy
            // fixture fields here. They must not repair a legitimate zero
            // TargetWind back to the current Wind.
            settings.refresh_runtime_fields();
            assert_eq!(settings.wind_target, 0, "TargetWind before frame {frame}");
            assert_eq!(settings.base_wind, 0, "Wind.Std before frame {frame}");

            let previous_wind = settings.wind;
            settings.advance_frame(&mut rng, frame);
            if frame % 10 == 0 {
                assert_eq!(settings.wind, previous_wind - 1, "Tick10 frame {frame}");
            } else {
                assert_eq!(settings.wind, previous_wind, "off-gate frame {frame}");
            }
            assert_eq!(settings.wind_target, 0, "TargetWind on frame {frame}");
            assert_eq!(settings.base_wind, 0, "Wind.Std on frame {frame}");
        }
        assert_eq!(settings.wind, 0, "Wind decays all the way to TargetWind");
    }

    #[test]
    fn fixed_scenario_wind_std_survives_script_wind_and_drives_decay() {
        // C4Weather::SetWind changes Wind and TargetWind, never the scenario
        // C4S.Weather.Wind.Std. The next Tick1000 evaluation therefore
        // restores TargetWind to Std and Tick10 begins the decay immediately.
        let mut settings = EnvironmentSettings::new(0);
        settings.wind = 80;
        settings.wind_target = 80;
        settings.refresh_runtime_fields();
        assert_eq!(
            settings.base_wind, 0,
            "script wind cannot replace scenario Std"
        );
        assert_eq!(
            settings.wind_target, 80,
            "normalization preserves TargetWind"
        );

        let mut rng = LcgRng::seed_from_u64(9_001);
        settings.advance_frame(&mut rng, 1_000);
        assert_eq!(settings.wind_target, 0, "Tick1000 evaluates scenario Std");
        assert_eq!(settings.wind, 79, "Tick10 starts decay on the same frame");

        for frame in 1_001..=1_790 {
            settings.refresh_runtime_fields();
            assert_eq!(settings.base_wind, 0, "Wind.Std before frame {frame}");
            assert_eq!(settings.wind_target, 0, "TargetWind before frame {frame}");
            settings.advance_frame(&mut rng, frame);
        }
        assert_eq!(settings.wind, 0, "script Wind decays back to scenario Std");
        assert_eq!(settings.wind_target, 0);
        assert_eq!(settings.base_wind, 0);
    }

    #[test]
    fn scenario_wind_c4sval_tick1000_uses_raw_rnd_and_bounds() {
        // C4Weather::Execute re-evaluates the scenario C4SVal verbatim on
        // Tick1000. Rnd=70 therefore means Random(141), even though the
        // [-30,30] bounds make the effective spread much smaller.
        let mut settings = EnvironmentSettings::new(0).with_wind_variation(70, 2_000);
        settings.wind_min = -30;
        settings.wind_max = 30;
        let mut rng = LcgRng::seed_from_u64(0xC4);
        let mut mirror = rng.clone();

        for frame in [1_000, 2_000, 3_000] {
            let raw = mirror.random(141) - 70;
            let expected = if raw < -30 {
                -30
            } else if raw > 30 {
                30
            } else {
                raw
            };

            settings.advance_frame(&mut rng, frame);

            assert_eq!(settings.wind_target, expected, "frame {frame}");
            assert_eq!(rng, mirror, "one Random(141) draw at frame {frame}");
        }
    }

    #[test]
    fn scenario_wind_c4sval_tick10_clamps_after_set_wind() {
        // Wind=(50,10,0,20) always evaluates to the Max bound. SetWind uses
        // its own hard [-100,100] clamp and sets both runtime fields; the
        // next Tick10 step then applies the narrower scenario bounds.
        let mut settings = EnvironmentSettings::new(50).with_wind_variation(10, 2_000);
        settings.wind_min = 0;
        settings.wind_max = 20;
        let mut delta = clonk_engine::compat::EnvironmentDelta::default();
        delta.wind = Some(-50);
        delta.apply(&mut settings);
        assert_eq!((settings.wind, settings.wind_target), (-50, -50));
        let mut rng = LcgRng::seed_from_u64(0xC4);
        let before_tick10 = rng.clone();

        settings.advance_frame(&mut rng, 10);

        assert_eq!(settings.wind, 0, "scenario Min clamps the SetWind value");
        assert_eq!(settings.wind_target, -50, "Tick10 does not reroll target");
        assert_eq!(rng, before_tick10, "Tick10 consumes no wind RNG");

        let mut mirror = rng.clone();
        let _ = mirror.random(21);

        settings.advance_frame(&mut rng, 1_000);

        assert_eq!(settings.wind_target, 20, "Std 50 always bounds to Max 20");
        assert_eq!(
            settings.wind, 1,
            "Tick1000 rerolls to 20, then steps once up from the Min bound"
        );
        assert_eq!(rng, mirror, "Tick1000 preserves the one-draw ledger");
    }

    const SET_BRIDGE_ACTION_DATA_SCRIPT: &str = r#"#strict 3
    global func Initialize(state, random)
    {
        if (SetBridgeActionData(200, true, false, 7))
        {
            return { energy = 1 };
        }
        return { energy = 0 };
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    const SET_BRIDGE_ACTION_DATA_FAILURE_SCRIPT: &str = r#"#strict 3
    global func Initialize(state, random)
    {
        if (SetBridgeActionData(120, false, false, -1))
        {
            return { energy = 1 };
        }
        return { energy = 0 };
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    const RANDOM_HELPER_SCRIPT: &str = r#"#strict 3
    global func Initialize(state, random)
    {
        return nil;
    }

    global func Step(state, frame, random)
    {
        return { energy = Random(10) };
    }
    "#;

    const PROCEDURE_MOVEMENT_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        return 0;
    }

    global func Step(state, frame, random)
    {
        return 0;
    }
    "#;

    const PATHFINDING_HELPER_SCRIPT: &str = r#"#strict 3
    global func Initialize(state, random)
    {
        var success = PathFree(0, 0, 10, 0);
        var failure = PathFree(0, 0, 10, 12);
        var value = 0;
        if (success)
        {
            value = value + 1;
        }
        if (failure)
        {
            value = value + 2;
        }
        return { energy = value };
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    fn build_lift_definition(id: &str) -> Definition {
        let mut definition = test_definition(id, id, PROCEDURE_MOVEMENT_SCRIPT);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Lift".to_string(), ActionSpec::for_procedure("lift"));
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition
    }

    fn build_idle_definition(id: &str) -> Definition {
        test_definition(id, id, PROCEDURE_MOVEMENT_SCRIPT)
    }

    fn build_definition() -> Definition {
        let source = r#"#strict 3
        global func Initialize(state, random) {
            return { energy = 100 };
        }

        global func Step(state, frame, random) {
            var vx = state.velocity[0] + 1;
            return { velocity = [vx, state.velocity[1]] };
        }
        "#;
        test_definition("Test", "Test", source)
    }

    #[test]
    fn initialize_returning_non_proplist_is_ignored_like_cpp() {
        // C++ parity: C4Object.cpp:1483 invokes `Call(PSF_Initialize)` as a bare
        // statement and DISCARDS the return value. Real Clonk definitions return
        // an int (or anything) from Initialize; the engine must not reject such a
        // return. The command-delta proplist convention is an additive Rust
        // convenience for synthetic fixtures, not a requirement on real content.
        let source = r#"
        global func Initialize(state, random) { return 1; }
        "#;
        let definition = test_definition("CLNK", "Clonk", source);
        let mut engine = Engine::new();
        engine.register_test_definition(definition);
        let spawned = engine.spawn_object(SpawnConfig::new("CLNK").with_energy(50));
        assert!(
            spawned.is_ok(),
            "Initialize returning an int must not error (C++ discards the return): {spawned:?}"
        );
    }

    #[test]
    fn home_base_production_shared_across_team_when_rule_enabled() {
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(1, "Ordered", 0).with_player_ids(vec![1, 2])
        ]);
        engine.set_team_home_base_rule(true);

        let mut crew = build_definition();
        crew.set_crew_member(true);
        engine.register_test_definition(crew);
        for owner in [1, 2] {
            engine.spawn_test_object(
                SpawnConfig::new("Test")
                    .with_owner(owner)
                    .with_alive(true)
                    .with_crew_member(true),
            );
        }

        let mut production = HashMap::new();
        production.insert("Brick".to_string(), 10);

        let leader = PlayerConfig::new(1, "Leader")
            .with_team(Some(1))
            .with_home_base_production(production.clone());
        let follower = PlayerConfig::new(2, "Follower")
            .with_team(Some(1))
            .with_home_base_production(production.clone());

        engine.register_test_player(leader);
        engine.register_test_player(follower);
        for (id, name) in [(1, "Leader"), (2, "Follower")] {
            engine
                .reinitialize_player_after_restore(
                    id,
                    PlayerAtClient::HOST,
                    "Local",
                    name,
                    PlayerRuntimeControl::NONE,
                    false,
                    true,
                    false,
                    false,
                )
                .test_value();
            let player = engine.player_mut(id).test_value();
            player.set_production_delay(0);
            player.set_production_unit(0);
            player.set_home_base_material_entries(Vec::new());
        }

        for _ in 0..2099 {
            engine.tick_without_snapshot().test_value();
        }
        assert_eq!(engine.frame(), 2099);

        let leader = engine.player(1).test_value();
        let follower = engine.player(2).test_value();
        assert!(leader.home_base_material().get("Brick").is_none());
        assert!(follower.home_base_material().get("Brick").is_none());
        assert_eq!(
            (leader.production_delay(), follower.production_delay()),
            (59, 59)
        );

        engine.tick_without_snapshot().test_value();
        assert_eq!(engine.frame(), 2100);

        let leader = engine.player(1).test_value();
        let follower = engine.player(2).test_value();
        assert_eq!(leader.home_base_material().get("Brick"), Some(&1));
        assert_eq!(follower.home_base_material().get("Brick"), Some(&1));
        assert_eq!(
            (leader.production_delay(), follower.production_delay()),
            (0, 60)
        );
        assert_eq!(
            (leader.production_unit(), follower.production_unit()),
            (1, 0)
        );
    }

    #[test]
    fn team_home_base_leader_uses_team_player_info_order_not_runtime_number() {
        // C4Team stores C4PlayerInfo IDs. GetFirstActivePlayerID walks that
        // order and resolves each ID to a runtime player; C4Player::Number is
        // a separate, reusable index (C4Teams.cpp:126-137;
        // C4Player.cpp:1637-1664).
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(1, "Ordered", 0).with_player_ids(vec![99, 20, 10])
        ]);
        engine.set_team_home_base_rule(true);

        let mut crew = build_definition();
        crew.set_crew_member(true);
        engine.register_test_definition(crew);
        for owner in [1, 5] {
            engine.spawn_test_object(
                SpawnConfig::new("Test")
                    .with_owner(owner)
                    .with_alive(true)
                    .with_crew_member(true),
            );
        }
        let production = HashMap::from([("Brick".to_string(), 10)]);
        engine.register_test_player(
            PlayerConfig::new(1, "Lower runtime number")
                .with_player_info_id(10)
                .with_team(Some(1))
                .with_home_base_production(production.clone()),
        );
        engine.register_test_player(
            PlayerConfig::new(5, "Team-order leader")
                .with_player_info_id(20)
                .with_team(Some(1))
                .with_home_base_production(production),
        );
        engine.player_mut(1).test_value().set_production_delay(59);
        engine.player_mut(5).test_value().set_production_delay(59);

        for _ in 0..34 {
            engine.tick_without_snapshot().test_value();
        }
        assert_eq!(
            (
                engine.player(5).expect("leader").production_delay(),
                engine.player(1).expect("follower").production_delay(),
            ),
            (59, 59),
        );
        engine.tick_without_snapshot().test_value();

        let follower = engine.player(1).test_value();
        let leader = engine.player(5).test_value();
        assert_eq!(
            (leader.production_delay(), follower.production_delay()),
            (0, 60)
        );
        assert_eq!(leader.home_base_material().get("Brick"), Some(&1));
        assert_eq!(follower.home_base_material().get("Brick"), Some(&1));
    }

    #[test]
    fn home_base_production_updates_empty_and_nonleader_delay_bookkeeping() {
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(1, "Ordered", 0).with_player_ids(vec![20, 10])
        ]);
        engine.set_team_home_base_rule(true);

        let mut crew = build_definition();
        crew.set_crew_member(true);
        engine.register_test_definition(crew);
        for owner in [1, 5] {
            engine.spawn_test_object(
                SpawnConfig::new("Test")
                    .with_owner(owner)
                    .with_alive(true)
                    .with_crew_member(true),
            );
        }
        engine.register_test_player(
            PlayerConfig::new(1, "Follower")
                .with_player_info_id(10)
                .with_team(Some(1)),
        );
        engine.register_test_player(
            PlayerConfig::new(5, "Leader")
                .with_player_info_id(20)
                .with_team(Some(1)),
        );
        engine.player_mut(1).test_value().set_production_delay(59);
        engine.player_mut(5).test_value().set_production_delay(59);

        for _ in 0..35 {
            engine.tick_without_snapshot().test_value();
        }

        let follower = engine.player(1).test_value();
        let leader = engine.player(5).test_value();
        assert!(leader.home_base_production_entries().is_empty());
        assert!(follower.home_base_production_entries().is_empty());
        assert_eq!(
            (leader.production_delay(), follower.production_delay()),
            (0, 60)
        );
        assert_eq!(
            (leader.production_unit(), follower.production_unit()),
            (1, 0)
        );
    }

    #[test]
    fn home_base_production_pauses_during_team_selection() {
        let mut engine = Engine::new();
        engine.register_test_player(
            PlayerConfig::new(1, "Choosing")
                .with_status(PlayerStatus::TeamSelection)
                .with_home_base_production(HashMap::from([("Brick".to_string(), 10)]))
                .with_production_delay(59),
        );

        for _ in 0..70 {
            engine.tick_without_snapshot().test_value();
        }
        engine
            .set_player_status(1, PlayerStatus::TeamSelectionPending)
            .test_value();
        for _ in 0..70 {
            engine.tick_without_snapshot().test_value();
        }

        let player = engine.player(1).test_value();
        assert_eq!(player.production_delay(), 59);
        assert_eq!(player.production_unit(), 0);
        assert!(player.home_base_material().get("Brick").is_none());
    }

    #[test]
    fn missing_team_definition_does_not_gate_home_base_production() {
        let mut engine = Engine::new();
        engine.set_team_home_base_rule(true);
        let mut crew = build_definition();
        crew.set_crew_member(true);
        engine.register_test_definition(crew);
        for owner in [1, 2] {
            engine.spawn_test_object(
                SpawnConfig::new("Test")
                    .with_owner(owner)
                    .with_alive(true)
                    .with_crew_member(true),
            );
            engine.register_test_player(
                PlayerConfig::new(owner, format!("Player {owner}")).with_team(Some(99)),
            );
            engine
                .reinitialize_player_after_restore(
                    owner,
                    PlayerAtClient::HOST,
                    "Local",
                    format!("Player {owner}"),
                    PlayerRuntimeControl::NONE,
                    false,
                    true,
                    false,
                    false,
                )
                .test_value();
            let player = engine.player_mut(owner).test_value();
            player.set_home_base_production(HashMap::from([("Brick".to_string(), 10)]));
            player.set_home_base_material_entries(Vec::new());
            player.set_production_delay(59);
            player.set_production_unit(0);
        }

        for _ in 0..35 {
            engine.tick_without_snapshot().test_value();
        }

        for owner in [1, 2] {
            let player = engine.player(owner).test_value();
            assert_eq!(player.production_delay(), 0);
            assert_eq!(player.production_unit(), 1);
            assert_eq!(player.home_base_material().get("Brick"), Some(&1));
        }
    }

    #[test]
    fn home_base_production_respects_rule_toggle() {
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(2, "Toggle", 0).with_player_ids(vec![1, 2])
        ]);
        engine.set_team_home_base_rule(false);

        let mut crew = build_definition();
        crew.set_crew_member(true);
        engine.register_test_definition(crew);
        for owner in [1, 2] {
            engine.spawn_test_object(
                SpawnConfig::new("Test")
                    .with_owner(owner)
                    .with_alive(true)
                    .with_crew_member(true),
            );
        }

        let mut production = HashMap::new();
        production.insert("Brick".to_string(), 10);

        let leader = PlayerConfig::new(1, "Leader")
            .with_team(Some(2))
            .with_home_base_production(production.clone());
        let follower = PlayerConfig::new(2, "Follower").with_team(Some(2));

        engine.register_test_player(leader);
        engine.register_test_player(follower);

        for _ in 0..60 {
            engine.tick_player_systems().test_value();
        }

        {
            let leader = engine.player(1).test_value();
            let follower = engine.player(2).test_value();
            assert_eq!(leader.home_base_material().get("Brick"), Some(&1));
            assert!(
                follower.home_base_material().get("Brick").is_none(),
                "follower should not receive materials when rule disabled"
            );
        }

        engine.set_team_home_base_rule(true);
        let leader_material = engine.player(1).test_value().home_base_material().clone();
        engine
            .set_player_home_base_material(1, leader_material)
            .test_value();

        let follower_after = engine.player(2).test_value();
        assert_eq!(follower_after.home_base_material().get("Brick"), Some(&1));
    }

    #[test]
    fn apply_player_commands_updates_home_base_material() {
        let mut engine = Engine::new();
        engine.register_test_player(PlayerConfig::new(1, "Leader"));

        engine
            .apply_player_commands(vec![PlayerCommand::AdjustHomeBaseMaterial {
                player_id: 1,
                definition_id: "Brick".to_string(),
                delta: 3,
            }])
            .test_value();

        let player = engine.player(1).test_value();
        assert_eq!(player.home_base_material().get("Brick"), Some(&3));
    }

    #[test]
    fn apply_player_commands_synchronizes_the_mutating_team_members_ordered_materials() {
        let mut engine = Engine::new();
        engine.set_team_home_base_rule(true);

        engine.register_test_player(PlayerConfig::new(1, "Leader").with_team(Some(1)));
        engine.register_test_player(PlayerConfig::new(2, "Follower").with_team(Some(1)));
        let starting = vec![("ZINC".into(), 7), ("BRIK".into(), 0)];
        engine
            .player_mut(1)
            .test_value()
            .set_home_base_material_entries(starting.clone());
        engine
            .player_mut(2)
            .test_value()
            .set_home_base_material_entries(starting);

        engine
            .apply_player_commands(vec![PlayerCommand::AdjustHomeBaseMaterial {
                player_id: 2,
                definition_id: "ROCK".to_string(),
                delta: 2,
            }])
            .test_value();

        let leader = engine.player(1).test_value();
        let follower = engine.player(2).test_value();
        let expected = &[
            ("ZINC".to_string(), 7),
            ("BRIK".to_string(), 0),
            ("ROCK".to_string(), 2),
        ];
        assert_eq!(leader.home_base_material_entries(), expected);
        assert_eq!(follower.home_base_material_entries(), expected);
    }

    #[test]
    fn apply_player_commands_grants_player_knowledge() {
        let mut engine = Engine::new();
        engine.register_test_player(PlayerConfig::new(1, "Scholar"));

        engine
            .apply_player_commands(vec![PlayerCommand::GrantKnowledge {
                player_id: 1,
                definition_id: "BRIK".to_string(),
            }])
            .test_value();

        let player = engine.player(1).test_value();
        assert!(
            player.knowledge().any(|id| id == "BRIK"),
            "player gains requested knowledge"
        );
    }

    #[test]
    fn player_magic_host_calls_preserve_order_and_same_call_writes() {
        // FnGetPlrMagic/FnSetPlrMagic (C4Script.cpp:2723-2748) query and
        // mutate C4Player::Magic in list order. Indexed reads filter by
        // C4D_Magic, while an ID read is a boolean membership check; a
        // successful SetPlrMagic is visible to the very next host call.
        let mut engine = Engine::with_seed(7);
        for (id, category) in [
            ("HIGH", CATEGORY_MAGIC),
            ("OBJE", CATEGORY_OBJECT),
            ("LOWM", CATEGORY_MAGIC),
            ("NEWM", CATEGORY_MAGIC),
        ] {
            let mut definition = test_definition(id, id, "");
            definition.set_category(category);
            engine.register_test_definition(definition);
        }
        engine.register_test_player(PlayerConfig::new(7, "Mage").with_magic(vec![
            "HIGH".into(),
            "OBJE".into(),
            "LOWM".into(),
        ]));
        engine.register_test_definition(test_definition(
            "CALL",
            "Caller",
            r#"#strict
        func Probe(player, high, new_magic) {
            var no_magic;
            return [
                GetPlrMagic(player, high),
                GetPlrMagic(player, no_magic, 0),
                GetPlrMagic(player, no_magic, 1),
                GetPlrMagic(player, no_magic, -1),
                GetPlrMagic(99, high),
                SetPlrMagic(player, new_magic),
                GetPlrMagic(player, new_magic),
                GetPlrMagic(player, no_magic, 2),
                SetPlrMagic(player, high, true),
                GetPlrMagic(player, high),
                SetPlrMagic(player, high, true)
            ];
        }
        "#,
        ));
        let caller = engine.spawn_test_object(SpawnConfig::new("CALL"));
        let caller_index = engine.test_object_index(caller);

        let result = engine.call_test_object_function(
            caller_index,
            "Probe",
            vec![
                Value::Int(7),
                Value::C4Id("HIGH".into()),
                Value::C4Id("NEWM".into()),
            ],
        );

        assert_eq!(
            result,
            Value::Array(vec![
                Value::Bool(true),
                Value::C4Id("HIGH".into()),
                Value::C4Id("LOWM".into()),
                Value::Nil,
                Value::Nil,
                Value::Int(1),
                Value::Bool(true),
                Value::C4Id("NEWM".into()),
                Value::Int(1),
                Value::Bool(false),
                Value::Int(0),
            ])
        );
        assert_eq!(
            engine
                .player(7)
                .expect("player persists")
                .magic()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["OBJE".to_string(), "LOWM".to_string(), "NEWM".to_string()]
        );
    }

    #[test]
    fn value_host_reads_raw_defcore_value_and_returns_nil_for_unknown_id() {
        // FnValue is deliberately narrower than GetValue: it returns the raw
        // C4Def::Value for a loaded ID and null for an unloaded/zero ID
        // (C4Script.cpp:1385-1389,6896). MagiClonk uses this exact helper for
        // both mana checks and the post-Activate energy deduction.
        let mut spell = test_definition("MBRG", "Bridge spell", "");
        spell.set_value(10);
        let caller = test_definition(
            "CALL",
            "Caller",
            r#"#strict
        func Probe(known, missing)
        {
          return [Value(known), Value(missing), Value()];
        }
        "#,
        );

        let mut engine = Engine::new();
        engine.register_test_definition(spell);
        engine.register_test_definition(caller);
        let object = engine.spawn_test_object(SpawnConfig::new("CALL"));
        let index = engine.test_object_index(object);

        assert_eq!(
            engine
                .call_object_function(
                    index,
                    "Probe",
                    vec![
                        Value::C4Id("MBRG".to_string()),
                        Value::C4Id("MISS".to_string()),
                    ],
                )
                .expect("Value host calls run"),
            Value::Array(vec![Value::Int(10), Value::Nil, Value::Nil])
        );
    }

    #[test]
    fn check_effect_matches_empty_priority_and_callback_merge_semantics() {
        // FnCheckEffect returns null before C4Effect::Check when the selected
        // list has no head; with a list, priority 1 bypasses callbacks. Other
        // priorities synchronously run Fx<Name>Effect and an annul result
        // (-2) calls the accepting effect's Fx<Name>Add, returning its number
        // (C4Script.cpp:5546-5556; C4Effect.cpp:271-317).
        let definition = test_definition(
            "CALL",
            "Caller",
            r#"#strict 2
        func EmptyGlobal() { var no_target; return CheckEffect("Probe", no_target, 100, 0); }
        func Install()
        {
          var no_target;
          AddEffect("World", no_target, 200, 0, this());
          return AddEffect("Shield", this(), 200, 0, this());
        }
        func Probe()
        {
          var no_target;
          return [
            CheckEffect("PriorityOne", no_target, 1, 0),
            CheckEffect("GlobalDenied", no_target, 100, 0),
            CheckEffect("Denied", this(), 100, 7, 42),
            CheckEffect("Merge", this(), 100, 9, 6),
            CheckEffect("Clean", this(), 300, 0)
          ];
        }
        func FxWorldEffect() { SetR(99); return(-1); }
        func FxShieldEffect(szNew, pTarget, iNumber, iUnused, iValue)
        {
          if (szNew == "Denied" && pTarget == this() && iNumber > 0 && !iUnused && iValue == 42) return(-1);
          if (szNew == "Merge") return(-2);
          return(0);
        }
        func FxShieldAdd(pTarget, iNumber, szNew, iInterval, iValue)
        {
          if (pTarget == this() && szNew == "Merge" && iInterval == 9) SetR(iValue);
          return(0);
        }
        "#,
        );

        let (mut engine, object) =
            pxs_default_fixture(definition, SpawnConfig::new("CALL").with_owner(1));
        let index = engine.test_object_index(object);

        assert_eq!(
            engine
                .call_object_function(index, "EmptyGlobal", vec![])
                .expect("empty global check runs"),
            Value::Nil,
            "a missing effect-list head returns null, not integer zero"
        );
        let shield = engine.call_test_object_function(index, "Install", vec![]);
        let shield_number = match shield {
            Value::Int(number) if number > 0 => number,
            other => panic!("AddEffect returns the Shield number, got {other:?}"),
        };

        assert_eq!(
            engine
                .call_object_function(index, "Probe", vec![])
                .expect("CheckEffect callback chain runs"),
            Value::Array(vec![
                Value::Int(0),
                Value::Int(-1),
                Value::Int(-1),
                Value::Int(shield_number),
                Value::Int(0),
            ])
        );
        assert_eq!(
            engine
                .object_snapshot(object)
                .expect("caller remains live")
                .rotation,
            6,
            "FxShieldAdd received the proposed interval/value and ran synchronously"
        );
    }

    #[test]
    fn removed_effects_stay_linked_dead_until_the_next_execute() {
        let definition = test_definition(
            "CALL",
            "Caller",
            r#"#strict 2
        local silent_number;

        func RemoveAndReplace()
        {
          var old_number = AddEffect("Old", this(), 100, 0, this());
          EffectVar(0, this(), old_number) = 77;
          var removed = RemoveEffect(0, this(), old_number);
          var old_lookup = GetEffect(0, this(), old_number);
          var old_var = EffectVar(0, this(), old_number);
          var replacement = AddEffect("New", this(), 200, 0, this());
          return [old_number, replacement, old_var, old_lookup, removed];
        }

        func RemoveWithoutCalls()
        {
          silent_number = AddEffect("Silent", this(), 100, 0, this());
          EffectVar(0, this(), silent_number) = 88;
          var removed = RemoveEffect(0, this(), silent_number, true);
          return [silent_number, removed, GetEffect(0, this(), silent_number), EffectVar(0, this(), silent_number)];
        }

        func ProbeSilent()
        {
          return [GetEffect(0, this(), silent_number), EffectVar(0, this(), silent_number)];
        }
        "#,
        );

        let (mut engine, object) = pxs_default_fixture(definition, SpawnConfig::new("CALL"));
        let index = engine.test_object_index(object);

        assert_eq!(
            engine
                .call_object_function(index, "RemoveAndReplace", vec![])
                .expect("remove-and-replace call runs"),
            Value::Array(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(77),
                Value::Int(1),
                Value::Bool(true),
            ]),
            "the dead number and vars remain visible and reserve the next number"
        );
        assert_eq!(
            engine
                .call_object_function(index, "RemoveWithoutCalls", vec![])
                .expect("no-callback removal runs"),
            Value::Array(vec![
                Value::Int(3),
                Value::Bool(true),
                Value::Int(3),
                Value::Int(88),
            ])
        );
        assert_eq!(
            engine
                .call_object_function(index, "ProbeSilent", vec![])
                .expect("dead effect remains visible before Execute"),
            Value::Array(vec![Value::Int(3), Value::Int(88)])
        );

        engine.tick_without_snapshot().test_value();
        assert_eq!(
            engine
                .call_object_function(index, "ProbeSilent", vec![])
                .expect("post-Execute probe runs"),
            Value::Array(vec![Value::Nil, Value::Nil])
        );
        assert_eq!(
            engine
                .object_snapshot(object)
                .expect("caller remains live")
                .effects
                .iter()
                .map(|effect| (effect.name.clone(), effect.number))
                .collect::<Vec<_>>(),
            vec![("New".to_owned(), 2)]
        );
    }

    #[test]
    fn deferred_named_remove_preserves_the_selected_same_name_effect_number() {
        let script = r#"#strict 3
global func Initialize(state, random)
{
  AddEffect("Twin", this(), 100, 0);
  AddEffect("Twin", this(), 200, 0);
  return nil;
}

global func Step(state, frame, random)
{
  if (frame == 1) RemoveEffect("Twin", this(), 1);
  return nil;
}

global func FxTwinStop(state, int number, int reason) { return nil; }
global func FirstTwin() { return GetEffect("Twin", this(), 0, 0); }
"#;

        let (calls, hooks) = pxs_call_hooks(|name, args| Some((name.to_owned(), args.to_vec())));

        let mut definition = test_definition("CALL", "Caller", script);
        definition.set_debugger_hooks(hooks);
        let (mut engine, object) = pxs_default_fixture(definition, SpawnConfig::new("CALL"));

        let initial = engine.test_object_snapshot(object);
        let first_number = initial
            .effects
            .iter()
            .find(|effect| effect.name == "Twin" && effect.priority == 100)
            .test_value()
            .number;
        let second_number = initial
            .effects
            .iter()
            .find(|effect| effect.name == "Twin" && effect.priority == 200)
            .test_value()
            .number;
        assert!(first_number < second_number);

        engine.tick_without_snapshot().test_value();
        let after_remove = engine.test_object_snapshot(object);
        assert!(after_remove.effects.iter().any(|effect| {
            effect.number == first_number && effect.name == "Twin" && effect.priority == 100
        }));
        assert!(after_remove.effects.iter().any(|effect| {
            effect.number == second_number && effect.name == "Twin" && effect.priority == 0
        }));
        let index = engine.test_object_index(object);
        assert_eq!(
            engine
                .call_object_function(index, "FirstTwin", vec![])
                .expect("live name lookup succeeds"),
            Value::Int(first_number)
        );

        let stop_calls = calls
            .lock()
            .test_value()
            .iter()
            .filter(|(name, _)| name == "FxTwinStop")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(stop_calls.len(), 1);
        assert_eq!(stop_calls[0].1.get(1), Some(&Value::Int(second_number)));
        assert_eq!(
            stop_calls[0].1.get(2),
            None,
            "C4Effect::Kill supplies only target and number; the typed script parameter nil-fills internally"
        );

        engine.tick_without_snapshot().test_value();
        assert_eq!(
            engine
                .object_snapshot(object)
                .expect("caller remains live")
                .effects
                .iter()
                .map(|effect| effect.number)
                .collect::<Vec<_>>(),
            vec![first_number]
        );
    }

    #[test]
    fn check_effect_preserves_dead_head_zero_and_skips_removed_later_checker() {
        // C4Effect nodes are only unlinked by Execute. Within one VM call a
        // removed final node still means FnCheckEffect had a non-null head,
        // so Check returns integer zero; a truly empty list returns nil. The
        // checker walk also re-tests IsDead at each node, so Killer removing
        // the later Victim prevents Victim's callback from running.
        let definition = test_definition(
            "CALL",
            "Caller",
            r#"
        func RemoveOnlyThenCheck()
        {
          var no_name;
          var iOnly = AddEffect("Only", this(), 200, 0, this());
          RemoveEffect(no_name, this(), iOnly, true);
          return CheckEffect("AfterRemove", this(), 100, 0);
        }
        func InstallWalk()
        {
          AddEffect("Killer", this(), 100, 0, this());
          return AddEffect("Victim", this(), 200, 0, this());
        }
        func Walk() { return CheckEffect("Probe", this(), 50, 0); }
        func FxKillerEffect(szNew, pTarget)
        {
          RemoveEffect("Victim", this(), 0, true);
          return(0);
        }
        func FxVictimEffect(szNew) { if (szNew == "Probe") { SetR(9); return(-1); } return(0); }
        "#,
        );

        let mut engine = Engine::new();
        engine.register_test_definition(definition);
        let removed = engine.spawn_test_object(SpawnConfig::new("CALL"));
        let removed_index = engine.test_object_index(removed);
        assert_eq!(
            engine
                .call_object_function(removed_index, "RemoveOnlyThenCheck", vec![])
                .expect("same-call removal check runs"),
            Value::Int(0),
            "a dead-but-not-yet-cleaned list head reaches C4Effect::Check"
        );
        assert_eq!(
            engine
                .call_object_function(removed_index, "Walk", vec![])
                .expect("pre-Execute dead-list check runs"),
            Value::Int(0),
            "command folding keeps the dead list head linked"
        );
        engine.tick_without_snapshot().test_value();
        assert_eq!(
            engine
                .call_object_function(removed_index, "Walk", vec![])
                .expect("post-Execute empty-list check runs"),
            Value::Nil,
            "the next Execute unlinks the dead final node"
        );

        let walked = engine.spawn_test_object(SpawnConfig::new("CALL").with_owner(1));
        let walked_index = engine.test_object_index(walked);
        engine.call_test_object_function(walked_index, "InstallWalk", vec![]);
        assert_eq!(
            engine
                .call_object_function(walked_index, "Walk", vec![])
                .expect("live checker walk runs"),
            Value::Int(0)
        );
        assert_eq!(
            engine
                .object_snapshot(walked)
                .expect("caller remains live")
                .rotation,
            0,
            "the removed later Victim checker was not dispatched from a stale snapshot"
        );
    }

    #[test]
    fn check_effect_add_deny_kills_last_same_name_acceptor_once_by_number() {
        // Multiple same-name effects coexist. The last -2 checker wins; when
        // its FxAdd returns -1, Check performs one full Kill of THAT numbered
        // acceptor and returns -2. The lower same-name peer must survive and
        // FxStop must run exactly once.
        let definition = test_definition(
            "CALL",
            "Caller",
            r#"#strict 2
        func Install()
        {
          SetR(1);
          var iFirst = AddEffect("Shield", this(), 100, 0, this());
          var iSecond = AddEffect("Shield", this(), 200, 0, this());
          return [iFirst, iSecond];
        }
        func Probe() { return CheckEffect("Merge", this(), 50, 0); }
        func FxShieldEffect(szNew) { if (szNew == "Merge") return(-2); return(0); }
        func FxShieldAdd() { return(-1); }
        func FxShieldStop() { SetR(GetR() + 1); return(0); }
        "#,
        );
        let (mut engine, object) = pxs_default_fixture(definition, SpawnConfig::new("CALL"));
        let index = engine.test_object_index(object);
        let installed = engine.call_test_object_function(index, "Install", vec![]);
        let Value::Array(numbers) = installed else {
            panic!("Install returns effect numbers")
        };
        let first = match numbers.first() {
            Some(Value::Int(number)) => *number,
            other => panic!("first effect number missing: {other:?}"),
        };
        let second = match numbers.get(1) {
            Some(Value::Int(number)) => *number,
            other => panic!("second effect number missing: {other:?}"),
        };

        assert_eq!(
            engine
                .call_object_function(index, "Probe", vec![])
                .expect("annul/add-deny check runs"),
            Value::Int(-2)
        );
        let snapshot = engine.test_object_snapshot(object);
        assert_eq!(snapshot.rotation, 2, "the acceptor Stop ran exactly once");
        assert_eq!(
            snapshot
                .effects
                .iter()
                .filter(|effect| effect.priority != 0)
                .map(|effect| effect.number)
                .collect::<Vec<_>>(),
            vec![first],
            "last-annul winner {second} is dead; its same-name peer survives"
        );
        assert!(
            snapshot
                .effects
                .iter()
                .any(|effect| effect.number == second && effect.priority == 0),
            "Kill leaves the annulled acceptor linked until Execute"
        );
        engine.tick_without_snapshot().test_value();
        assert_eq!(
            engine
                .object_snapshot(object)
                .expect("caller remains live")
                .effects
                .iter()
                .map(|effect| effect.number)
                .collect::<Vec<_>>(),
            vec![first]
        );
    }

    #[test]
    fn check_effect_annul_calls_temp_brackets_add_in_cpp_order() {
        let definition = test_definition(
            "CALL",
            "Caller",
            r#"
        func Install()
        {
          var iShield = AddEffect("Shield", this(), 100, 0, this());
          AddEffect("Upper", this(), 200, 0, this());
          SetR(0);
          return iShield;
        }
        func Probe() { return CheckEffect("Merge", this(), 50, 0); }
        func FxShieldEffect() { return(-3); }
        func FxShieldAdd()
        {
          if (GetEffectCount("*", this(), -100) != 1) return(-1);
          SetR(GetR() * 10 + GetEffectCount("*", this(), 100));
          return(0);
        }
        func FxUpperStop(pTarget, iNumber, iTemp, fTemp)
        {
          if (iTemp == 1 && fTemp) SetR(GetR() * 10 + 1);
          return(0);
        }
        func FxUpperStart(pTarget, iNumber, iTemp)
        {
          if (iTemp == 1) SetR(GetR() * 10 + 3);
          return(0);
        }
        "#,
        );
        let (mut engine, object) = pxs_default_fixture(definition, SpawnConfig::new("CALL"));
        let index = engine.test_object_index(object);
        let shield = engine.call_test_object_function(index, "Install", vec![]);
        assert_eq!(
            engine
                .call_object_function(index, "Probe", vec![])
                .expect("annul-calls check runs"),
            shield
        );
        assert_eq!(
            engine
                .object_snapshot(object)
                .expect("caller remains live")
                .rotation,
            123,
            "AnnulCalls orders Upper Stop, signed-priority count 2, Upper Start"
        );
    }

    #[test]
    fn scenario_init_builds_and_persists_cpp_ordered_magic_lists() {
        // C4Player::ScenarioInit copies the scenario list, removes unknown
        // definitions, loads every C4D_Magic definition when that result is
        // empty, then stable-sorts by DefCore Value (C4Player.cpp:705-708;
        // C4IDList.cpp:177-205). C4Player::CompileFunc persists Magic
        // verbatim (C4Player.cpp:1610-1612).
        let mut engine = Engine::with_seed(7);
        for (id, category, value) in [
            ("HIGM", CATEGORY_MAGIC, 100),
            ("LOWM", CATEGORY_MAGIC, 10),
            ("MIDM", CATEGORY_MAGIC, 10),
            ("NEWM", CATEGORY_MAGIC, 50),
            ("OBJE", CATEGORY_OBJECT, 5),
        ] {
            let mut definition = test_definition(id, id, "");
            definition.set_category(category);
            definition.set_value(value);
            engine.register_test_definition(definition);
        }
        let mut explicit = PlayerStart::default();
        explicit.magic = vec![
            ("HIGM".into(), 7),
            ("MISS".into(), 99),
            ("MIDM".into(), -2),
            ("LOWM".into(), 0),
            ("OBJE".into(), 3),
        ];
        engine.set_player_starts(vec![explicit, PlayerStart::default()]);

        for name in ["Explicit", "Default"] {
            engine
                .join_player(JoinPlayerConfig {
                    name: name.to_string(),
                    player_info_id: 0,
                    score: 0,
                    rounds: 0,
                    rounds_won: 0,
                    rounds_lost: 0,
                    total_playing_time: 0,
                    team: None,
                    color_dw: 0xff0000,
                    pref_color: 0,
                    pref_position: 0,
                    crew: Vec::new(),
                    control_style: false,
                    auto_context_menu: false,
                    startup_player_count: 1,
                })
                .test_value();
        }

        assert_eq!(
            engine
                .player(0)
                .expect("explicit player exists")
                .magic()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["OBJE", "MIDM", "LOWM", "HIGM"]
        );
        assert_eq!(
            engine
                .player(1)
                .expect("default player exists")
                .magic()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["MIDM", "LOWM", "NEWM", "HIGM"]
        );

        let captured = engine.capture_state();
        assert_eq!(
            captured
                .players
                .iter()
                .find(|player| player.id == 0)
                .expect("explicit state exists")
                .magic_entries,
            vec![
                ("OBJE".into(), 3),
                ("MIDM".into(), -2),
                ("LOWM".into(), 0),
                ("HIGM".into(), 7),
            ]
        );
        assert_eq!(
            captured
                .players
                .iter()
                .find(|player| player.id == 1)
                .expect("default state exists")
                .magic_entries,
            vec![
                ("MIDM".into(), 0),
                ("LOWM".into(), 0),
                ("NEWM".into(), 0),
                ("HIGM".into(), 0),
            ]
        );
        let encoded = captured.to_json_string().test_value();
        let decoded = EngineState::from_json_str(&encoded).test_value();
        engine.restore_state(&decoded).test_value();
        assert_eq!(
            engine
                .player(0)
                .expect("restored player exists")
                .magic()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["OBJE", "MIDM", "LOWM", "HIGM"]
        );
        assert_eq!(
            engine
                .capture_state()
                .players
                .into_iter()
                .find(|player| player.id == 0)
                .expect("restored state exists")
                .magic_entries,
            vec![
                ("OBJE".into(), 3),
                ("MIDM".into(), -2),
                ("LOWM".into(), 0),
                ("HIGM".into(), 7),
            ]
        );
    }

    #[test]
    fn apply_player_commands_revokes_player_knowledge() {
        let mut engine = Engine::new();
        engine.register_test_player(
            PlayerConfig::new(1, "Scholar").with_knowledge(vec!["BRIK".to_string()]),
        );

        engine
            .apply_player_commands(vec![PlayerCommand::RevokeKnowledge {
                player_id: 1,
                definition_id: "BRIK".to_string(),
            }])
            .test_value();

        let player = engine.player(1).test_value();
        assert!(
            player.knowledge().all(|id| id != "BRIK"),
            "player no longer knows revoked definition"
        );
    }

    #[test]
    fn enabling_team_rule_synchronizes_existing_members() {
        let mut engine = Engine::new();

        let mut material = HashMap::new();
        material.insert("Brick".to_string(), 5);

        let leader = PlayerConfig::new(1, "Leader")
            .with_team(Some(3))
            .with_home_base_material(material.clone());
        let follower = PlayerConfig::new(2, "Follower").with_team(Some(3));

        engine.register_test_player(leader);
        engine.register_test_player(follower);

        let follower_before = engine.player(2).test_value();
        assert!(
            follower_before.home_base_material().is_empty(),
            "rule disabled keeps member inventory separate"
        );

        engine.set_team_home_base_rule(true);

        let follower_after = engine.player(2).test_value();
        assert_eq!(follower_after.home_base_material().get("Brick"), Some(&5));
    }

    #[test]
    fn path_free_host_function_queries_landscape() {
        let mut definition = test_definition("PathTester", "PathTester", PATHFINDING_HELPER_SCRIPT);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine.register_test_definition(definition);
        engine.set_landscape(Landscape::flat(32, 8));

        let id = engine.spawn_test_object(SpawnConfig::new("PathTester"));

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.energy, 1);
    }

    #[test]
    fn clo_162_pow_nil_pads_the_exponent_and_wraps_overflow() {
        let script = r#"#strict
local short_result, overflow_result;

func Initialize()
{
    short_result = Pow(5);
    overflow_result = Pow(2, 31);
}
"#;
        let definition = test_definition("P162", "Pow short arguments", script);
        let (engine, object) = pxs_default_fixture(definition, SpawnConfig::new("P162"));
        let index = engine.test_object_index(object);
        let locals = &engine.objects[index].state.local_vars;

        assert_eq!(locals.get("short_result"), Some(&Value::Int(1)));
        assert_eq!(
            locals.get("overflow_result"),
            Some(&Value::Int(i32::MIN)),
            "2^31 wraps to the signed int32 minimum like C++"
        );
    }

    #[test]
    fn clo_162_path_free_nil_pads_the_destination_to_zero_zero() {
        let script = r#"#strict
local short_path;

func Initialize()
{
    short_path = PathFree(6, 6);
}
"#;
        let definition = test_definition("F162", "PathFree short arguments", script);
        let mut engine = Engine::new();
        engine.register_test_definition(definition);

        // The nil-padded destination is (0,0), so the (6,6)->(0,0) ray
        // crosses the only solid pixel at (3,3).
        let mut pixels = vec![0_u8; 7 * 7];
        pixels[3 * 7 + 3] = 1;
        let mut densities = vec![0_i32; 2];
        densities[1] = 100;
        let grid = landscape::PixelGrid::new(7, 7, pixels, densities, vec![None; 2], vec![None; 2]);
        let landscape = pxs_grid_world(7, 7, vec![0; 7], grid);
        engine.set_landscape(landscape);

        let object = engine.spawn_test_object(SpawnConfig::new("F162"));
        let index = engine.test_object_index(object);

        assert_eq!(
            engine.objects[index].state.local_vars.get("short_path"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn clo_162_object_distance_without_arguments_returns_nil() {
        let script = r#"#strict
local distance;

func Initialize()
{
    distance = ObjectDistance();
}
"#;
        let definition = test_definition("D162", "ObjectDistance no arguments", script);
        let (engine, object) = pxs_default_fixture(definition, SpawnConfig::new("D162"));
        let index = engine.test_object_index(object);

        assert_eq!(
            engine.objects[index].state.local_vars.get("distance"),
            Some(&Value::Nil)
        );
    }

    #[test]
    fn advances_actions_using_definition_map() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default()
                .with_length(2)
                .with_delay(1)
                .with_next("Idle"),
        );
        actions.insert("Idle".to_string(), ActionSpec::default().with_length(1));
        definition.configure_actions(Some("Walk".to_string()), actions);

        let (mut engine, id) = pxs_fixture(0, definition, SpawnConfig::new("Test"));

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);

        let snapshot = engine.test_tick();
        let object = snapshot.object(id).test_value();
        assert_eq!(object.action.name, "Walk");
        assert_eq!(object.action.phase, 1);
        assert_eq!(object.action.ticks, 0);

        let snapshot = engine.test_tick();
        let object = snapshot.object(id).test_value();
        assert_eq!(object.action.name, "Idle");
        assert_eq!(object.action.phase, 0);
        assert_eq!(object.action.ticks, 0);
    }

    #[test]
    fn menu_command_invokes_definition_script() {
        let mut definition = test_definition("Crew", "Crew", MENU_COMMAND_SCRIPT);
        definition.set_crew_member(true);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let (mut engine, id) = pxs_fixture(0, definition, SpawnConfig::new("Crew").with_owner(1));

        let selection = MenuCommandSelection {
            primary_id: id,
            instances: vec![id],
            definition_id: "Crew".to_string(),
            label: "Crew".to_string(),
        };

        let handled = engine
            .menu_command(id, MenuCommandKind::Focus, selection)
            .test_value();
        assert!(handled, "script should report handled command");

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(
            snapshot.rotation, 42,
            "script should update object rotation via SetR"
        );
    }

    #[test]
    fn action_delay_requires_multiple_ticks() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert(
            "Loop".to_string(),
            ActionSpec::default().with_length(3).with_delay(2),
        );
        definition.configure_actions(Some("Loop".to_string()), actions);

        let (mut engine, id) = pxs_fixture(0, definition, SpawnConfig::new("Test"));

        let initial = engine.test_object_snapshot(id);
        assert_eq!(initial.action.phase, 0);
        assert_eq!(initial.action.ticks, 0);

        let after_first = engine.test_tick();
        let object = after_first.object(id).test_value();
        assert_eq!(object.action.phase, 0);
        assert_eq!(object.action.ticks, 1);

        let after_second = engine.test_tick();
        let object = after_second.object(id).test_value();
        assert_eq!(object.action.phase, 1);
        assert_eq!(object.action.ticks, 0);

        let after_third = engine.test_tick();
        let object = after_third.object(id).test_value();
        assert_eq!(object.action.phase, 1);
        assert_eq!(object.action.ticks, 1);

        let after_fourth = engine.test_tick();
        let object = after_fourth.object(id).test_value();
        assert_eq!(object.action.phase, 2);
        assert_eq!(object.action.ticks, 0);
    }

    #[test]
    fn action_start_and_end_callbacks_fire() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func OnIdleStart(state, action) { return 0; }
        global func OnIdleEnd(state, action) { return 0; }
        global func OnWalkStart(state, action) { return 0; }
        "#;

        let (call_log, hooks) = pxs_call_hooks(|name, _args| Some(name.to_string()));

        let mut definition = test_definition("Actor", "Actor", script);
        definition.set_debugger_hooks(hooks);

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default()
                .with_length(1)
                .with_delay(1)
                .with_next("Walk")
                .with_start_call("OnIdleStart")
                .with_end_call("OnIdleEnd"),
        );
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_start_call("OnWalkStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(5);
        engine.register_test_definition(definition);

        engine.spawn_test_object(SpawnConfig::new("Actor"));

        {
            let calls = call_log.lock().test_value().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_end, 0);
            assert_eq!(walk_start, 0);
        }

        engine.tick_without_snapshot().test_value();

        {
            let calls = call_log.lock().test_value().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_end, 1);
            assert_eq!(walk_start, 1);
        }

        engine.tick_without_snapshot().test_value();

        {
            let calls = call_log.lock().test_value().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_end, 1);
            assert_eq!(walk_start, 1);
        }
    }

    #[test]
    fn initialize_set_action_runs_one_start_then_abort_pair() {
        let script = r#"#strict
protected func Initialize()
{
    SetAction("New");
    return 1;
}

protected func OnNewStart()
{
    return 1;
}

protected func OnOldAbort()
{
    return 1;
}
"#;
        let (call_log, hooks) = pxs_call_hooks(|name, _args| {
            matches!(name, "OnNewStart" | "OnOldAbort").then(|| name.to_string())
        });
        let definition = start_abort_definition("ACBI", "Action callback init", script, hooks);
        let mut engine = Engine::with_seed(0);
        engine.register_test_definition(definition);
        engine.spawn_test_object(SpawnConfig::new("ACBI"));

        assert_eq!(
            call_log.lock().unwrap().as_slice(),
            ["OnNewStart", "OnOldAbort"],
            "C4Object::SetAction runs one StartCall/AbortCall pair during Initialize"
        );
    }

    #[test]
    fn initial_effect_set_action_runs_one_start_then_abort_pair() {
        let script = r#"#strict 3
func FxSwitchStart(object target, int number, int temp)
{
    SetAction("New");
    return 1;
}

func OnNewStart() { return 1; }
func OnOldAbort() { return 1; }
"#;
        let (call_log, hooks) = pxs_call_hooks(|name, _args| {
            matches!(name, "OnNewStart" | "OnOldAbort").then(|| name.to_string())
        });
        let definition = start_abort_definition("ACEF", "Action callback effect", script, hooks);
        let mut engine = Engine::with_seed(0);
        engine.register_test_definition(definition);
        let object_id = ObjectId::new(77);
        engine.spawn_test_object(
            SpawnConfig::new("ACEF").with_id(object_id).add_effect(
                EffectState::new("Switch")
                    .with_priority(100)
                    .with_command_target(Some(object_id.as_u64() as i32)),
            ),
        );

        assert_eq!(
            call_log.lock().unwrap().as_slice(),
            ["OnNewStart", "OnOldAbort"],
            "initial FxStart SetAction is not replayed after insertion"
        );
    }

    #[test]
    fn engine_set_action_same_name_dispatches_start_then_abort_with_saved_phase(
    ) -> Result<(), EngineError> {
        // C4Object::SetAction saves iLastPhase before resetting Phase even for
        // a same-action call (C4Object.cpp:4104-4105, 4132-4146), then runs
        // StartCall before AbortCall and passes that saved phase
        // (C4Object.cpp:4171-4193; default call mask C4Object.h:307-309).
        let (calls, hooks) = pxs_call_hooks(|name, args| {
            matches!(name, "OnLoopStart" | "OnLoopAbort").then(|| (name.to_string(), args.to_vec()))
        });
        let mut definition = test_definition("LOOP", "Loop actor", "#strict\npublic func ResetLoop() { return SetAction(\"Loop\"); }\npublic func ResetLoopForced() { var no_value; return SetAction(\"Loop\", no_value, no_value, true); }\nprotected func OnLoopStart() { return(1); }\nprotected func OnLoopAbort(int iPhase) { return(1); }\n");
        definition.set_c4_callback_convention(true);
        definition.set_debugger_hooks(hooks);
        definition.configure_actions(
            Some("Loop".to_string()),
            HashMap::from([(
                "Loop".to_string(),
                ActionSpec::default()
                    .with_start_call("OnLoopStart")
                    .with_abort_call("OnLoopAbort"),
            )]),
        );

        let mut action = ActionState::new("Loop");
        action.phase = 7;
        action.ticks = 6;
        action.time = 42;
        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("LOOP")
                .with_action(action)
                .with_loaded(true),
        )?;
        let index = engine.test_object_index(id);
        assert_eq!(
            engine.call_object_function(index, "ResetLoop", Vec::new())?,
            Value::Bool(true)
        );

        let first_calls = calls.lock().test_value().clone();
        assert_eq!(first_calls.len(), 2, "observed callbacks: {first_calls:?}");
        assert_eq!(first_calls[0].0, "OnLoopStart");
        assert_eq!(first_calls[1].0, "OnLoopAbort");
        assert_eq!(first_calls[1].1.first(), Some(&Value::Int(7)));
        let action = &engine.objects[index].state.action;
        assert_eq!(action.phase, 0, "same-name SetAction resets Phase");
        assert_eq!(action.ticks, 0, "same-name SetAction resets PhaseDelay");
        assert_eq!(action.time, 42, "same-name SetAction preserves Time");

        calls.lock().test_value().clear();
        assert_eq!(
            engine.call_object_function(index, "ResetLoopForced", Vec::new())?,
            Value::Bool(true)
        );
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[("OnLoopStart".to_string(), Vec::new())],
            "fDirect/fForce suppresses AbortCall but still runs StartCall"
        );
        Ok(())
    }

    #[test]
    fn set_action_callback_chain_is_not_truncated_at_sixteen() -> Result<(), EngineError> {
        // A natural Root -> A01 transition runs Start(A01), whose nested
        // SetActions form a 19-level depth-first chain. C++ completes every
        // nested Start/Abort before resuming the outer End(Root); it has no
        // SetAction-specific depth cap (C4Object.cpp:4171-4197, 5485).
        const LAST_ACTION: usize = 20;
        let mut script = String::from("#strict 2\nprotected func EndRoot() { return 1; }\n");
        let mut actions = HashMap::from([(
            "Root".to_string(),
            ActionSpec::default()
                .with_length(1)
                .with_delay(1)
                .with_next("A01")
                .with_end_call("EndRoot"),
        )]);
        for index in 1..=LAST_ACTION {
            let action = format!("A{index:02}");
            let start = format!("Start{index:02}");
            let abort = format!("Abort{index:02}");
            let body = if index < LAST_ACTION {
                format!("SetAction(\"A{:02}\"); ", index + 1)
            } else {
                String::new()
            };
            script.push_str(&format!(
                "protected func {start}() {{ {body}return 1; }}\n\
                 protected func {abort}(int phase) {{ return 1; }}\n"
            ));
            actions.insert(
                action,
                ActionSpec::default()
                    .with_start_call(start)
                    .with_abort_call(abort),
            );
        }

        let (callback_log, hooks) = pxs_call_hooks(|name, _args| {
            (name.starts_with("Start") || name.starts_with("Abort") || name == "EndRoot")
                .then(|| name.to_string())
        });

        let mut definition = test_definition("SACD", "SetAction depth", &script);
        definition.set_c4_callback_convention(true);
        definition.set_debugger_hooks(hooks);
        definition.configure_actions(Some("Root".to_string()), actions);

        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        let object = engine.spawn_object(
            SpawnConfig::new("SACD")
                .with_action(ActionState::new("Root"))
                .with_loaded(true),
        )?;
        engine.tick_without_snapshot()?;

        let mut expected = (1..=LAST_ACTION)
            .map(|index| format!("Start{index:02}"))
            .collect::<Vec<_>>();
        expected.extend(
            (1..LAST_ACTION)
                .rev()
                .map(|index| format!("Abort{index:02}")),
        );
        expected.push("EndRoot".to_string());
        assert_eq!(callback_log.lock().unwrap().as_slice(), expected);
        let index = engine.test_object_index(object);
        assert_eq!(engine.objects[index].state.action.name, "A20");
        Ok(())
    }

    #[test]
    fn runaway_set_action_callbacks_report_the_vm_limit() -> Result<(), EngineError> {
        // C4Aul's fail-safe callback Exec reports and unwinds recursion once
        // the shared VM stack is exhausted. SetAction itself still returns
        // true; it must not silently skip the callback at an arbitrary host
        // depth (C4AulExec.cpp:1318-1342).
        let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut hooks = DebuggerHooks::new();
        {
            let callback_count = Arc::clone(&callback_count);
            hooks.set_on_call(move |name, _args| {
                if name == "LoopStart" {
                    callback_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            });
        }
        let mut definition = test_definition(
            "SACR",
            "SetAction recursion",
            r#"#strict 2
        public func Trigger() { return SetAction("Loop"); }
        public func Healthy() { return 73; }
        protected func LoopStart() { SetAction("Loop"); return 1; }
        protected func LoopAbort(int phase) { return 1; }
        "#,
        );
        definition.set_c4_callback_convention(true);
        definition.set_debugger_hooks(hooks);
        definition.configure_actions(
            Some("Loop".to_string()),
            HashMap::from([(
                "Loop".to_string(),
                ActionSpec::default()
                    .with_start_call("LoopStart")
                    .with_abort_call("LoopAbort"),
            )]),
        );

        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        let object = engine.spawn_object(
            SpawnConfig::new("SACR")
                .with_action(ActionState::new("Loop"))
                .with_loaded(true),
        )?;
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::layer::SubscriberExt::with(
            tracing_subscriber::Registry::default(),
            PlayerObjectCommandDiagnosticLayer {
                records: Arc::clone(&records),
            },
        );
        let index = engine.test_object_index(object);
        let result = tracing::subscriber::with_default(subscriber, || {
            engine.call_object_function(index, "Trigger", Vec::new())
        })?;

        assert_eq!(result, Value::Bool(true));
        assert!(
            callback_count.load(std::sync::atomic::Ordering::Relaxed) > 16,
            "callbacks continue beyond the removed Rust-only limit"
        );
        let records = records.lock().test_value();
        assert!(records.iter().any(|record| {
            record.message == "SetAction callback error; continuing like the C++ fail-safe exec"
                && record
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("internal error: value stack overflow!"))
        }));
        assert!(!records
            .iter()
            .any(|record| record.message.contains("recursion backstop")));
        drop(records);
        assert_eq!(
            engine.call_object_function(index, "Healthy", Vec::new())?,
            Value::Int(73),
            "the fail-safe unwind releases the shared VM stack"
        );
        Ok(())
    }

    #[test]
    fn phase_call_set_phase_suppresses_same_tick_next_action_like_cpp() -> Result<(), EngineError> {
        let script = r#"#strict 2
local seen_action;
protected func OnPhase()
{
    seen_action = GetAction();
    SetPhase(0);
    return 1;
}
func ReadSeen() { return seen_action; }
"#;
        let mut definition = test_definition("PHCL", "Phase callback", script);
        definition.set_c4_callback_convention(true);
        definition.configure_actions(
            None,
            HashMap::from([
                (
                    "Loop".to_string(),
                    ActionSpec::default()
                        .with_length(1)
                        .with_delay(1)
                        .with_phase_call("OnPhase")
                        .with_next("Done"),
                ),
                ("Done".to_string(), ActionSpec::default()),
            ]),
        );
        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        let object = engine.spawn_object(
            SpawnConfig::new("PHCL")
                .with_action(ActionState::new("Loop"))
                .with_loaded(true),
        )?;

        engine.tick_without_snapshot()?;

        let index = engine.test_object_index(object);
        assert_eq!(engine.objects[index].state.action.name, "Loop");
        assert_eq!(engine.objects[index].state.action.phase, 0);
        assert_eq!(
            engine.call_object_function(index, "ReadSeen", Vec::new())?,
            Value::String("Loop".to_string().into())
        );
        Ok(())
    }

    #[test]
    fn engine_set_action_stops_callbacks_after_start_invalidates_receiver(
    ) -> Result<(), EngineError> {
        // After each callback C++ stops the SetAction sequence if the object
        // was removed or its definition changed (C4Object.cpp:4178-4198).
        // RemoveObject clears Status (C4Script.cpp:456-460); ChangeDef swaps
        // Def in place (C4Object.cpp:1207-1227).
        let (calls, hooks) = pxs_call_hooks(|name, _args| {
            matches!(
                name,
                "RemoveOnStart" | "OldAbort" | "ChangeOnStart" | "AbortAfterChange"
            )
            .then(|| name.to_string())
        });

        let mut remove_def = test_definition("RMOV", "Remove on start", "#strict\npublic func Trigger() { return SetAction(\"New\"); }\nprotected func RemoveOnStart() { RemoveObject(); return(1); }\nprotected func OldAbort(int iPhase) { return(1); }\n");
        remove_def.set_c4_callback_convention(true);
        remove_def.set_debugger_hooks(hooks.clone());
        remove_def.configure_actions(
            Some("Old".to_string()),
            HashMap::from([
                (
                    "Old".to_string(),
                    ActionSpec::default().with_abort_call("OldAbort"),
                ),
                (
                    "New".to_string(),
                    ActionSpec::default().with_start_call("RemoveOnStart"),
                ),
            ]),
        );
        let mut remove_engine = Engine::new();
        remove_engine.register_definition(remove_def)?;
        let removed = remove_engine.spawn_object(
            SpawnConfig::new("RMOV")
                .with_action(ActionState::new("Old"))
                .with_loaded(true),
        )?;
        let remove_index = remove_engine.test_object_index(removed);
        assert_eq!(
            remove_engine.call_object_function(remove_index, "Trigger", Vec::new())?,
            Value::Bool(true)
        );
        assert!(remove_engine.objects[remove_index].destroyed);
        assert_eq!(calls.lock().unwrap().as_slice(), ["RemoveOnStart"]);

        calls.lock().test_value().clear();
        let mut changed_def = test_definition(
            "NEWD",
            "Changed definition",
            "#strict\nprotected func AbortAfterChange(int iPhase) { return(1); }\n",
        );
        changed_def.set_c4_callback_convention(true);
        changed_def.set_debugger_hooks(hooks.clone());
        changed_def.configure_actions(
            Some("Rest".to_string()),
            HashMap::from([
                ("Rest".to_string(), ActionSpec::default()),
                (
                    "Old".to_string(),
                    ActionSpec::default().with_abort_call("AbortAfterChange"),
                ),
            ]),
        );
        let mut swap_def = test_definition("SWAP", "Change on start", "#strict\npublic func Trigger() { return SetAction(\"New\"); }\nprotected func ChangeOnStart() { ChangeDef(NEWD); return(1); }\n");
        swap_def.set_c4_callback_convention(true);
        swap_def.set_debugger_hooks(hooks);
        swap_def.configure_actions(
            Some("Old".to_string()),
            HashMap::from([
                ("Old".to_string(), ActionSpec::default()),
                (
                    "New".to_string(),
                    ActionSpec::default().with_start_call("ChangeOnStart"),
                ),
            ]),
        );
        let mut swap_engine = Engine::new();
        swap_engine.register_definition(changed_def)?;
        swap_engine.register_definition(swap_def)?;
        let swapped = swap_engine.spawn_object(
            SpawnConfig::new("SWAP")
                .with_action(ActionState::new("Old"))
                .with_loaded(true),
        )?;
        let swap_index = swap_engine.test_object_index(swapped);
        assert_eq!(
            swap_engine.call_object_function(swap_index, "Trigger", Vec::new())?,
            Value::Bool(true)
        );
        assert_eq!(swap_engine.objects[swap_index].definition_id, "NEWD");
        assert_eq!(calls.lock().unwrap().as_slice(), ["ChangeOnStart"]);
        Ok(())
    }

    #[test]
    fn foreign_set_dir_runs_full_turn_action_transition_like_cpp() -> Result<(), EngineError> {
        let target_script = r#"#strict 2
local starts, abort_phase;
func Read() { return [starts, abort_phase]; }
protected func TurnStart() { starts += 1; return 1; }
protected func WalkAbort(int phase) { abort_phase = phase; return 1; }
"#;
        let mut target = test_definition("SDTG", "SetDir target", target_script);
        target.set_c4_callback_convention(true);
        target.configure_actions(
            None,
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_directions(2)
                        .with_turn_action("Turn")
                        .with_abort_call("WalkAbort"),
                ),
                (
                    "Turn".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_directions(2)
                        .with_start_call("TurnStart"),
                ),
            ]),
        );
        let caller = test_definition(
            "SDCL",
            "SetDir caller",
            "#strict 2\nfunc TurnOther(object target) { return SetDir(1, target); }\n",
        );
        let mut engine = Engine::new();
        engine.register_definition(target)?;
        engine.register_definition(caller)?;

        let target = engine.spawn_object(
            SpawnConfig::new("SDTG")
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Left)
                .with_loaded(true),
        )?;
        let target_index = engine.test_object_index(target);
        {
            let action = &mut engine.objects[target_index].state.action;
            action.phase = 7;
            action.ticks = 6;
            action.time = 42;
            action.data = 9;
        }
        let caller = engine.spawn_object(SpawnConfig::new("SDCL"))?;
        let caller_index = engine.test_object_index(caller);

        assert_eq!(
            engine.call_object_function(
                caller_index,
                "TurnOther",
                vec![Value::Object(target.as_u64())],
            )?,
            Value::Bool(true)
        );
        let target_index = engine.test_object_index(target);
        let state = &engine.objects[target_index].state;
        assert_eq!(state.direction, Direction::Right);
        assert_eq!(state.action.name, "Turn");
        assert_eq!(
            (
                state.action.phase,
                state.action.ticks,
                state.action.time,
                state.action.data
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(
            engine.call_object_function(target_index, "Read", Vec::new())?,
            Value::Array(vec![Value::Int(1), Value::Int(7)])
        );
        Ok(())
    }

    #[test]
    fn forced_action_change_triggers_abort_callbacks() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func OnIdleStart(state, action) { return 0; }
        global func OnIdleEnd(state, action) { return 0; }
        global func OnIdleAbort(state, action) { return 0; }
        global func OnRunStart(state, action) { return 0; }
        "#;

        let (call_log, hooks) = pxs_call_hooks(|name, _args| Some(name.to_string()));

        let mut definition = test_definition("Actor", "Actor", script);
        definition.set_debugger_hooks(hooks);

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default()
                .with_length(20)
                .with_start_call("OnIdleStart")
                .with_end_call("OnIdleEnd")
                .with_abort_call("OnIdleAbort"),
        );
        actions.insert(
            "Run".to_string(),
            ActionSpec::default().with_start_call("OnRunStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let (mut engine, id) = pxs_fixture(11, definition, SpawnConfig::new("Actor"));

        {
            let calls = call_log.lock().test_value().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_abort = calls.iter().filter(|name| *name == "OnIdleAbort").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let run_start = calls.iter().filter(|name| *name == "OnRunStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_abort, 0);
            assert_eq!(idle_end, 0);
            assert_eq!(run_start, 0);
        }

        engine
            .apply_object_update(id, ObjectUpdate::new().with_action("Run"))
            .test_value();

        {
            let calls = call_log.lock().test_value().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_abort = calls.iter().filter(|name| *name == "OnIdleAbort").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let run_start = calls.iter().filter(|name| *name == "OnRunStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_abort, 1);
            assert_eq!(idle_end, 0);
            assert_eq!(run_start, 1);
        }
    }

    #[test]
    fn real_content_action_callbacks_use_cpp_argument_convention() {
        // C++ ActMap callbacks: StartCall/EndCall run with no parameters
        // (C4Object.cpp:4154,4168); AbortCall receives the last phase as its
        // only parameter, `Exec(this, {C4VInt(iLastPhase)})`
        // (C4Object.cpp:4182). Content like COWB's AbortJumpDrawGun(int
        // iPhase) feeds that straight into SetPhase.
        let script = r#"
        global func OnIdleStart(a) { return 0; }
        global func OnIdleAbort(iPhase) { return 0; }
        "#;

        let (call_log, hooks) =
            pxs_call_hooks(|name, args| Some((name.to_string(), args.to_vec())));

        let mut definition = test_definition("Actor", "Actor", script);
        definition.set_c4_callback_convention(true);
        definition.set_debugger_hooks(hooks);

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default()
                .with_length(20)
                .with_start_call("OnIdleStart")
                .with_abort_call("OnIdleAbort"),
        );
        actions.insert("Run".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let (mut engine, id) = pxs_fixture(11, definition, SpawnConfig::new("Actor"));
        engine
            .apply_object_update(id, ObjectUpdate::new().with_action("Run"))
            .test_value();

        let calls = call_log.lock().test_value().clone();
        let start_args = calls
            .iter()
            .find(|(name, _)| name == "OnIdleStart")
            .map(|(_, args)| args.clone())
            .test_value();
        assert!(
            start_args
                .iter()
                .all(|arg| matches!(arg, clonk_script::Value::Nil)),
            "StartCall passes no parameters, got {start_args:?}"
        );
        let abort_args = calls
            .iter()
            .find(|(name, _)| name == "OnIdleAbort")
            .map(|(_, args)| args.clone())
            .test_value();
        assert_eq!(
            abort_args.first(),
            Some(&clonk_script::Value::Nil),
            "AbortCall passes phase 0, normalized to nil by the nonstrict callee, got {abort_args:?}"
        );
    }

    #[test]
    fn non_forced_action_update_respects_no_other_action() {
        let (mut engine, id) = no_other_action_fixture(7);

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_action_update(ActionUpdate::default().with_name("Run").with_force(false)),
            )
            .test_value();

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.action.name, "Idle");
    }

    #[test]
    fn forced_action_update_overrides_no_other_action() {
        let (mut engine, id) = no_other_action_fixture(13);

        engine
            .apply_object_update(id, ObjectUpdate::new().with_action("Run"))
            .test_value();

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.action.name, "Run");
    }

    #[test]
    fn action_phase_callbacks_fire() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func OnIdlePhase(state, action) { return 0; }
        global func OnWalkStart(state, action) { return 0; }
        "#;

        let (call_log, hooks) = pxs_call_hooks(|name, _args| Some(name.to_string()));

        let mut definition = test_definition("Actor", "Actor", script);
        definition.set_debugger_hooks(hooks);

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default()
                .with_length(3)
                .with_delay(1)
                .with_next("Walk")
                .with_phase_call("OnIdlePhase"),
        );
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_start_call("OnWalkStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let (mut engine, id) = pxs_fixture(2, definition, SpawnConfig::new("Actor"));

        {
            let calls = call_log.lock().test_value().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            assert_eq!(idle_phase, 0);
        }

        engine.tick_without_snapshot().test_value();

        {
            let calls = call_log.lock().test_value().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            assert_eq!(idle_phase, 1);
        }

        engine.tick_without_snapshot().test_value();

        {
            let calls = call_log.lock().test_value().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            assert_eq!(idle_phase, 2);
        }

        engine.tick_without_snapshot().test_value();

        {
            let calls = call_log.lock().test_value().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_phase, 3);
            assert_eq!(walk_start, 1);
        }

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);
    }

    #[test]
    fn action_step_advances_multiple_phases() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        "#;

        let mut definition = test_definition("Stepper", "Stepper", script);

        let mut actions = HashMap::new();
        actions.insert(
            "Pulse".to_string(),
            ActionSpec::default()
                .with_length(5)
                .with_delay(1)
                .with_step(2)
                .with_next("Pulse"),
        );
        definition.configure_actions(Some("Pulse".to_string()), actions);

        let (mut engine, id) = pxs_fixture(7, definition, SpawnConfig::new("Stepper"));

        let after_first = engine.test_tick();
        let object = after_first.object(id).test_value();
        assert_eq!(object.action.phase, 2);

        let after_second = engine.test_tick();
        let object = after_second.object(id).test_value();
        assert_eq!(object.action.phase, 4);

        let after_third = engine.test_tick();
        let object = after_third.object(id).test_value();
        assert_eq!(object.action.phase, 0);
    }

    #[test]
    fn host_get_effect_queries_effect_stack() {
        let definition = test_definition("EffectUser", "Effect User", EFFECT_HOST_SCRIPT);

        let (mut engine, id) = pxs_fixture(0, definition, SpawnConfig::new("EffectUser"));

        engine.tick_without_snapshot().test_value();
        engine.tick_without_snapshot().test_value();

        let snapshot = engine.test_object_snapshot(id);
        // C++ list order ascends by |priority| (C4Effect.cpp:80-94).
        assert_eq!(snapshot.effects.len(), 2);
        assert_eq!(snapshot.effects[0].name, "Spark");
        assert_eq!(snapshot.effects[0].priority, 60);
        assert_eq!(snapshot.effects[1].name, "Glow");
        assert_eq!(snapshot.effects[1].priority, 150);
        assert_eq!(snapshot.energy, 365);
    }

    #[test]
    fn host_add_effect_and_remove_effect_via_helpers() {
        let definition = test_definition(
            "EffectBridge",
            "Effect Bridge",
            EFFECT_HOST_ADD_REMOVE_SCRIPT,
        );

        let (mut engine, id) = pxs_fixture(0, definition, SpawnConfig::new("EffectBridge"));

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.effects.len(), 2);
        assert_eq!(snapshot.effects[0].name, "Spark");
        assert_eq!(snapshot.effects[1].name, "Glow");

        let first_tick = engine.test_tick();
        let object = first_tick.object(id).test_value();
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Spark" && effect.priority != 0));
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Glow" && effect.priority == 0));

        let second_tick = engine.test_tick();
        let object = second_tick.object(id).test_value();
        assert_eq!(object.effects.len(), 1);
        assert_eq!(object.effects[0].name, "Spark");
        assert_eq!(object.effects[0].priority, 0);

        let third_tick = engine.test_tick();
        let object = third_tick.object(id).test_value();
        assert!(object.effects.is_empty());
    }

    #[test]
    fn host_helpers_modify_global_effects() {
        let definition =
            test_definition("GlobalEffect", "Global Effect", GLOBAL_EFFECT_HELPER_SCRIPT);

        let mut engine = Engine::with_seed(0);
        engine.register_test_definition(definition);

        engine.spawn_test_object(SpawnConfig::new("GlobalEffect"));

        assert_eq!(engine.global_effects().len(), 1);
        assert_eq!(engine.global_effects()[0].name, "WorldPulse");

        engine.tick_without_snapshot().test_value();

        assert!(engine.global_effects().is_empty());
    }

    #[test]
    fn inactive_objects_skip_physics_and_step() {
        let mut definition = build_definition();
        definition.configure_actions(Some("Idle".to_string()), HashMap::new());

        let (mut engine, id) = pxs_fixture(
            0,
            definition,
            SpawnConfig::new("Test")
                .with_velocity(Vector2::new(3, 0))
                .with_energy(50),
        );

        engine.tick_without_snapshot().test_value();

        engine
            .apply_object_update(id, ObjectUpdate::new().with_status(ObjectStatus::Inactive))
            .test_value();

        let before = engine.test_object_snapshot(id);

        engine.tick_without_snapshot().test_value();

        let after = engine.test_object_snapshot(id);

        assert_eq!(after.velocity, before.velocity);
        assert_eq!(after.position, before.position);
        assert_eq!(after.energy, before.energy);
        assert_eq!(after.status, ObjectStatus::Inactive);
    }

    #[test]
    fn engine_state_persists_object_status() {
        let mut definition = build_definition();
        definition.configure_actions(Some("Idle".to_string()), HashMap::new());

        let (mut engine, id) = pxs_fixture(
            0,
            definition,
            SpawnConfig::new("Test")
                .with_status(ObjectStatus::Inactive)
                .with_owner(1)
                .with_crew_member(true),
        );

        engine
            .apply_object_update(id, ObjectUpdate::new().with_status(ObjectStatus::Inactive))
            .test_value();

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(0);
        restored.register_test_definition(build_definition());
        restored.restore_state(&state).test_value();

        let snapshot = restored.test_object_snapshot(id);
        assert_eq!(snapshot.status, ObjectStatus::Inactive);
        assert!(restored.crew_members(1).is_empty());
        // Elimination is Tick35-gated (C4Player.cpp:225-235): the restored
        // crewless owner eliminates once the game runs to the boundary.
        assert!(!restored.is_owner_eliminated(1));
        for _ in 0..35 {
            restored.tick_without_snapshot().test_value();
        }
        assert!(restored.is_owner_eliminated(1));
    }

    #[test]
    fn host_random_consumes_engine_rng() {
        let definition = test_definition("RandomUser", "Random User", RANDOM_HELPER_SCRIPT);

        let (mut engine, id) = pxs_fixture(0, definition, SpawnConfig::new("RandomUser"));

        let mut expected_rng = LcgRng::seed_from_u64(0);
        let _ = expected_rng.random(i32::MAX); // Initialize random argument
        let _ = expected_rng.random(i32::MAX); // First tick random argument
        let first_expected = expected_rng.random(10);

        let first_tick = engine.test_tick();
        let object = first_tick.object(id).test_value();
        assert_eq!(object.energy, first_expected);

        let _ = expected_rng.random(i32::MAX); // Second tick random argument
        let second_expected = expected_rng.random(10);

        let second_tick = engine.test_tick();
        let object = second_tick.object(id).test_value();
        assert_eq!(object.energy, second_expected);
    }

    #[test]
    fn action_procedure_surfaces_in_state_value() {
        let mut definition = test_definition("Airborne", "Airborne", PROCEDURE_STATE_SCRIPT);
        let mut actions = HashMap::new();
        actions.insert("Fly".to_string(), ActionSpec::for_procedure("flight"));
        definition.configure_actions(Some("Fly".to_string()), actions);

        let (engine, id) = pxs_fixture(0, definition, SpawnConfig::new("Airborne"));

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.energy, 7);
    }

    #[test]
    fn snapshot_includes_action_procedure() {
        let mut definition = test_definition("Airborne", "Airborne", PROCEDURE_STATE_SCRIPT);
        let mut actions = HashMap::new();
        actions.insert("Fly".to_string(), ActionSpec::for_procedure("flight"));
        definition.configure_actions(Some("Fly".to_string()), actions);

        let (engine, id) = pxs_fixture(0, definition, SpawnConfig::new("Airborne"));

        let snapshot = engine.test_object_snapshot(id);
        assert_eq!(snapshot.action_procedure.as_deref(), Some("flight"));
    }
