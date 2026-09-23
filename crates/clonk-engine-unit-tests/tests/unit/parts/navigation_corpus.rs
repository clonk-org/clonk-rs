// Construction material trips through the shipped Frontier flow (Build,
// Acquire, Get, MoveTo) under the normal profile's navigation AI
// (`sim-navigation-ai`, docs/COMPAT_PROFILE.md). Each world is flat ground
// at y=140 with one terrain feature; the builder stands at a castle site at
// x=360, placed the way CreateConstruction places one, and fetches the one
// rock the site still needs. C4Command, which the LegacyClonk profile keeps,
// paces forever in the slit, island and sealed-cave worlds.

const CORPUS_GROUND: i32 = 140;
const CORPUS_WIDTH: usize = 480;
const CORPUS_HEIGHT: usize = 260;
const CORPUS_SITE_X: i32 = 360;
/// Every reachable rock here arrives within about 330 frames.
const CORPUS_FRAMES: usize = 900;

type CorpusRect = (i32, i32, i32, i32, bool);

/// Ground below `CORPUS_GROUND`, then each (x0, y0, x1, y1, solid) rect.
/// Vehicle is in the material table, as in all real content, so the site's
/// SolidMask is baked into the pixel plane the planner reads.
fn corpus_landscape(rects: &[CorpusRect]) -> Landscape {
    let mut solid = vec![false; CORPUS_WIDTH * CORPUS_HEIGHT];
    let mut set = |x0: i32, y0: i32, x1: i32, y1: i32, value: bool| {
        for y in y0.max(0)..=y1.min(CORPUS_HEIGHT as i32 - 1) {
            for x in x0.max(0)..=x1.min(CORPUS_WIDTH as i32 - 1) {
                solid[y as usize * CORPUS_WIDTH + x as usize] = value;
            }
        }
    };
    set(
        0,
        CORPUS_GROUND,
        CORPUS_WIDTH as i32 - 1,
        CORPUS_HEIGHT as i32 - 1,
        true,
    );
    for &(x0, y0, x1, y1, value) in rects {
        set(x0, y0, x1, y1, value);
    }
    let heights = (0..CORPUS_WIDTH)
        .map(|x| {
            (0..CORPUS_HEIGHT)
                .find(|&y| solid[y * CORPUS_WIDTH + x])
                .unwrap_or(CORPUS_HEIGHT) as i32
        })
        .collect();
    let grid = clonk_engine::landscape::PixelGrid::new(
        CORPUS_WIDTH as u32,
        CORPUS_HEIGHT as u32,
        solid.iter().map(|&value| u8::from(value)).collect(),
        vec![0, 80, 100],
        vec![None, Some("Earth".to_string()), Some("Vehicle".to_string())],
        vec![None; 3],
    );
    let mut landscape = Landscape::new(CORPUS_WIDTH as u32, heights).test_value();
    landscape.set_world_height(CORPUS_HEIGHT as i32);
    landscape.set_pixel_grid(grid);
    landscape
}

const CORPUS_CLEAR_SCRIPT: &str = "#strict 2
global func CorpusClear()
{
  for (var obj in FindObjects(Find_Not(Find_Or(Find_ID(CLNK), Find_Category(C4D_Rule), Find_Category(C4D_Goal), Find_Category(C4D_Environment)))))
    RemoveObject(obj);
}
";

fn corpus_script(engine: &mut Engine, source: &str) {
    let control = ScriptControlData {
        script: LegacyCString::from_bytes(source.as_bytes().to_vec()).test_value(),
        by_client: -1,
        ..Default::default()
    };
    let policy = ScriptControlPolicy {
        is_replay: false,
        console_active: true,
        allow_scripting_in_replays: false,
    };
    engine.execute_script_control(&control, policy).test_value();
}

#[derive(Debug, PartialEq, Eq)]
enum FetchOutcome {
    /// The site advanced at `frame`, built from the rock with index `rock`.
    Delivered { rock: Option<usize>, frame: usize },
    /// The builder's command stack emptied at `frame`.
    GaveUp { frame: usize },
    /// Still at it when the frames ran out: a loop.
    StillTrying,
}

fn fetch_for_construction(rects: &[CorpusRect], rocks: &[Vector2]) -> FetchOutcome {
    let (mut engine, owner, clonk) = frontier_crew_engine(true);
    engine.set_landscape(corpus_landscape(rects));
    engine
        .apply_scenario_script_edit("NavigationCorpus", CORPUS_CLEAR_SCRIPT)
        .test_value();
    corpus_script(&mut engine, "CorpusClear()");
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new().with_position(Vector2::new(CORPUS_SITE_X, CORPUS_GROUND - 10)),
        )
        .test_value();
    corpus_script(
        &mut engine,
        &format!("CreateConstruction(CST1, {CORPUS_SITE_X}, {CORPUS_GROUND}, {owner}, 90, true)"),
    );
    let site = engine
        .objects
        .iter()
        .find(|object| object.definition_id.as_str() == "CST1")
        .map(|object| object.id)
        .test_value();
    let rocks: Vec<ObjectId> = rocks
        .iter()
        .map(|&at| engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(at)))
        .collect();
    engine.spawn_test_object(SpawnConfig::new("WOOD").with_container(clonk));
    engine
        .execute_player_command(
            owner,
            CommandId::Build as i32,
            0,
            0,
            site.as_u64() as i32,
            0,
            0,
            1,
        )
        .test_value();

    let mut carried = None;
    for frame in 0..CORPUS_FRAMES {
        let snapshot = engine.test_tick();
        carried = carried.or_else(|| {
            rocks.iter().position(|&rock| {
                snapshot
                    .object(rock)
                    .is_some_and(|rock| rock.container == Some(clonk))
            })
        });
        if snapshot
            .object(site)
            .is_some_and(|site| site.construction > 90_000)
        {
            return FetchOutcome::Delivered {
                rock: carried,
                frame,
            };
        }
        if snapshot
            .object(clonk)
            .is_some_and(|builder| builder.command_stack.command_names().is_empty())
        {
            return FetchOutcome::GaveUp { frame };
        }
    }
    FetchOutcome::StillTrying
}

fn undelivered(cases: &[(&str, Vec<CorpusRect>, Vector2)]) -> Vec<(String, FetchOutcome)> {
    cases
        .iter()
        .map(|(name, rects, rock)| (name.to_string(), fetch_for_construction(rects, &[*rock])))
        .filter(|(_, outcome)| !matches!(outcome, FetchOutcome::Delivered { .. }))
        .collect()
}

#[test]
fn navigation_fetches_construction_material_across_flat_ground_and_pits() {
    let g = CORPUS_GROUND;
    let cases = [
        ("flat ground", vec![], Vector2::new(260, g - 4)),
        ("far along flat ground", vec![], Vector2::new(60, g - 4)),
        (
            "in a 30px pit",
            vec![(220, g, 280, g + 29, false)],
            Vector2::new(250, g + 26),
        ),
        (
            "in a 60px pit",
            vec![(220, g, 280, g + 59, false)],
            Vector2::new(250, g + 56),
        ),
    ];
    let failures = undelivered(&cases);
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn navigation_fetches_construction_material_over_cliffs_and_chasms() {
    let g = CORPUS_GROUND;
    let cases = [
        (
            "atop a 30px cliff",
            vec![(0, g - 30, 279, g - 1, true)],
            Vector2::new(200, g - 34),
        ),
        (
            "atop a 60px cliff",
            vec![(0, g - 60, 279, g - 1, true)],
            Vector2::new(200, g - 64),
        ),
        (
            "across a 30px chasm",
            vec![(250, g, 280, g + 99, false)],
            Vector2::new(200, g - 4),
        ),
    ];
    let failures = undelivered(&cases);
    assert!(failures.is_empty(), "{failures:?}");
}

/// A wall down to 10px above the ground, a slit no Clonk fits through.
fn slit_wall() -> Vec<CorpusRect> {
    let g = CORPUS_GROUND;
    vec![(270, 0, 281, g - 1, true), (270, g - 10, 281, g - 1, false)]
}

#[test]
fn navigation_gives_up_on_material_behind_a_body_high_slit() {
    let outcome = fetch_for_construction(&slit_wall(), &[Vector2::new(150, CORPUS_GROUND - 4)]);
    assert!(
        matches!(outcome, FetchOutcome::GaveUp { frame } if frame < 200),
        "C4PathFinder routes a point through the slit; the builder must not pace at it: {outcome:?}"
    );
}

#[test]
fn navigation_fetches_construction_material_through_a_tunnel_under_the_slit() {
    let g = CORPUS_GROUND;
    let mut detour = slit_wall();
    detour.extend([
        (240, g + 10, 312, g + 33, false),
        (292, g, 312, g + 9, false),
        (240, g, 260, g + 9, false),
    ]);
    let outcome = fetch_for_construction(&detour, &[Vector2::new(150, g - 4)]);
    assert!(
        matches!(outcome, FetchOutcome::Delivered { .. }),
        "{outcome:?}"
    );
}

#[test]
fn navigation_fetches_the_reachable_rock_over_a_nearer_unreachable_one() {
    let g = CORPUS_GROUND;
    let far = Vector2::new(60, g - 4);
    let island = [(300, g - 80, 340, g - 75, true)];
    let sealed = [(280, g + 30, 300, g + 45, false)];
    for (name, rects, near) in [
        ("on a floating island", &island, Vector2::new(320, g - 84)),
        ("in a sealed cave", &sealed, Vector2::new(290, g + 41)),
    ] {
        let outcome = fetch_for_construction(rects, &[near, far]);
        assert!(
            matches!(outcome, FetchOutcome::Delivered { rock: Some(1), .. }),
            "the nearer rock is {name}: {outcome:?}"
        );
    }
}
