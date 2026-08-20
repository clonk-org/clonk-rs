//! Phase-1 C++↔Rust differential parity check.
//!
//! Runs the determinism-critical primitives (`C4Fixed`, the LCG RNG, and the
//! per-frame sub-pixel accumulation) through the Rust port and asserts they are
//! byte-for-byte identical to the C++ golden oracle in
//! `parity/golden/parity_golden.json`. That golden is produced from the REAL
//! engine code (`src/Fixed.h`, `src/Fixed.cpp`'s `SineTable`, `src/C4Random.h`,
//! `src/C4ScriptKiller.h`, `src/C4LandscapePath.h`, and
//! `src/C4ActionDirection.h`, `src/C4ActionCallbacks.h`, and
//! `src/C4SolidMaskBitmap.h`, mechanically extracted DFA_PUSH/DFA_PULL/DFA_FIGHT
//! direction blocks from `src/C4Object.cpp`, `C4PlayerList::GetCount` and
//! `Join`'s capacity block from `src/C4PlayerList.cpp`, plus complete `FnEval`,
//! DirectExec's temporary context setup, `C4Effect::Execute`, C4AulScriptFunc's engine-call
//! forwarding and script-context setup, `FnGetX`/`FnGetY`,
//! `C4Object::DigOutMaterialCast`,
//! `C4Game::ShakeObjects`, `C4Object::Fling`, `C4Landscape::ClearPix`,
//! `BlastFreePix`, `BlastFree`, `ExecuteScan`, and `DoScan` bodies and the
//! `C4SGame::ConvertGoals`, `C4Game::InitRules`/`InitGoals`, and
//! bottom/top/side-flight `C4Object::ContactAction` arms) by
//! `parity/oracle/gen_golden.sh` — so this is a genuine differential against
//! the C++ oracle, not a Rust-vs-Rust regression.
//!
//! This gates Theme C (wiring fixed precision through physics): the gravity /
//! velocity sub-pixel accumulation the harness exercises is exactly the
//! arithmetic Theme C extends. The C++ per-pixel collision loop (item 4) is out
//! of scope here and is the subject of a future live-bridge differential.
//!
//! On any divergence the test panics with the first mismatch (section, index,
//! field, C++ value vs Rust value).

use clonk_resources::Group;
use clonk_script::{c4_hash_combine, cnv_fn, C4VType, Value as ScriptValue, ValueMap};
use serde_json::Value;

use crate::compat::{cos_func, sin_func, sqrt_func, LandscapeOperation};
use crate::landscape::{Landscape, LandscapeRasterState, PixelGrid};
use crate::material::{consume_corrosion_effect_rng, evaluate_corrosion, MaterialSet};
use crate::math::{
    fixed10, fixed100, fixed256, fixtoi, fixtoi_prec, itofix, itofix_prec, C4Fixed, FixedVec2,
};
use crate::rng::LcgRng;
use crate::scenario::{
    GameParameterRuleGoalLists, LegacyDefinitionResolver, MapPixelClassifier, ScenarioError,
    ScenarioIdListEntry,
};
use crate::{
    contact_action_wall_tumble_x, ActionSpec, ActionState, CommandDirection, Definition,
    DefinitionPicture, DefinitionRect, DefinitionSpriteImage, DefinitionTargetRect, Direction,
    EffectVarValue, Engine, EngineError, JoinPlayerConfig, ObjectBaseGraphics, ObjectStatus,
    ObjectUpdate, PhysicalInfo, PhysicsSettings, PlayerConfig, Scenario, ShapeAttachRecord,
    SpawnConfig, CATEGORY_LIVING, CATEGORY_OBJECT, CATEGORY_VEHICLE, OWNER_NONE,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../parity/golden/parity_golden.json"
);

fn load_golden() -> Value {
    let text = std::fs::read_to_string(GOLDEN).unwrap_or_else(|e| {
        panic!(
            "could not read C++ golden at {GOLDEN}: {e}\n\
             Generate it with `parity/oracle/gen_golden.sh`."
        )
    });
    serde_json::from_str(&text).expect("golden parity JSON parses")
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("golden entry missing integer field `{key}`: {v}"))
}

fn u(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("golden entry missing unsigned field `{key}`: {v}"))
}

fn register_real_c4_effect_definition(engine: &mut Engine, id: &str, name: &str, source: &str) {
    let mut definition = Definition::from_script(id, name, source)
        .unwrap_or_else(|error| panic!("{id} effect fixture compiles: {error}"));
    // Production resource loading enables this on every real C4Script
    // definition (scenario/core.rs:303-307); the command-DSL proplist
    // convention is intentionally test-fixture-only.
    definition.set_c4_callback_convention(true);
    engine
        .register_definition(definition)
        .unwrap_or_else(|error| panic!("{id} effect fixture registers: {error}"));
}

/// Assert two values are equal, panicking with a precise first-divergence report.
fn expect_eq(section: &str, index: usize, field: &str, cpp: i64, rust: i64) {
    if cpp != rust {
        write_parity_diff_from_environment(
            section,
            index,
            field,
            serde_json::json!(cpp),
            serde_json::json!(rust),
        );
    }
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
}

fn expect_eq_u64(section: &str, index: usize, field: &str, cpp: u64, rust: u64) {
    if cpp != rust {
        write_parity_diff_from_environment(
            section,
            index,
            field,
            serde_json::json!(cpp),
            serde_json::json!(rust),
        );
    }
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
}

/// The 8x40 material grid `parity/oracle/oracle_main.cpp`'s `splash_effect`
/// scaffolds, and the `SplashHost` over it: water (liquid and instable), a
/// liquid that is NOT instable, and granite.
struct SplashProbe {
    grid: [[i32; SplashProbe::WIDTH as usize]; SplashProbe::HEIGHT as usize],
    rng: LcgRng,
    bubbles: Vec<[i32; 2]>,
    casts: Vec<[i32; 5]>,
    extractions: i64,
}

impl SplashProbe {
    const WIDTH: i32 = 8;
    const HEIGHT: i32 = 40;
    const MAP: [(i32, bool); 3] = [(25, true), (25, false), (50, false)];

    /// Water at or below `water_top`, granite at or below `floor_top`, sky
    /// above, using material `liquid_mat` for the water body.
    fn new(water_top: i32, floor_top: i32, liquid_mat: i32) -> Self {
        let mut grid = [[-1; Self::WIDTH as usize]; Self::HEIGHT as usize];
        for (y, row) in grid.iter_mut().enumerate() {
            let y = y as i32;
            row.fill(if y >= floor_top {
                2
            } else if y >= water_top {
                liquid_mat
            } else {
                -1
            });
        }
        Self {
            grid,
            rng: LcgRng::new(0),
            bubbles: Vec::new(),
            casts: Vec::new(),
            extractions: 0,
        }
    }

    fn water_column(water_top: i32) -> Self {
        Self::new(water_top, Self::HEIGHT, 0)
    }

    fn material(&self, x: i32, y: i32) -> Option<usize> {
        (0..Self::WIDTH).contains(&x).then_some(())?;
        (0..Self::HEIGHT).contains(&y).then_some(())?;
        usize::try_from(self.grid[y as usize][x as usize]).ok()
    }

    fn density(&self, x: i32, y: i32) -> i32 {
        self.material(x, y).map_or(0, |mat| Self::MAP[mat].0)
    }
}

impl crate::engine_splash::SplashHost for SplashProbe {
    type Error = std::convert::Infallible;

    fn splash_is_semi_solid(&self, x: i32, y: i32) -> bool {
        self.density(x, y) >= 25
    }

    fn splash_material_is_liquid(&self, x: i32, y: i32) -> bool {
        self.material(x, y)
            .map(|mat| Self::MAP[mat])
            .is_some_and(|(density, instable)| (25..50).contains(&density) && instable)
    }

    fn splash_is_liquid(&self, x: i32, y: i32) -> bool {
        (25..50).contains(&self.density(x, y))
    }

    fn splash_random(&mut self, upper_bound: i32) -> Result<i32, Self::Error> {
        Ok(self.rng.random(upper_bound))
    }

    fn splash_bubble_out(&mut self, x: i32, y: i32) -> Result<(), Self::Error> {
        self.bubbles.push([x, y]);
        Ok(())
    }

    /// C++ hands `PXS::Create` whatever `ExtractMaterial` returned, and
    /// `Create` drops an invalid material (C4PXS.cpp:210) — so the extraction
    /// is counted either way and only a real material casts.
    fn splash_extract_and_cast(
        &mut self,
        source: crate::Vector2,
        destination: crate::Vector2,
        velocity: FixedVec2,
    ) -> Result<(), Self::Error> {
        self.extractions += 1;
        let Some(material) = self
            .material(source.x, source.y)
            .filter(|mat| (25..50).contains(&Self::MAP[*mat].0))
        else {
            return Ok(());
        };
        self.grid[source.y as usize][source.x as usize] = -1;
        self.casts.push([
            material as i32,
            destination.x,
            destination.y,
            fixtoi_prec(velocity.x, 100),
            fixtoi_prec(velocity.y, 100),
        ]);
        Ok(())
    }
}

/// The 24x16 landscape `parity/oracle/oracle_main.cpp`'s `shape_contact`
/// scaffolds: sky above y=10, earth below, a water pocket at x=3..5 and a
/// pillar at x=17..18, with the border configuration under test. Installing it
/// on the engine is what resolves the grid's material names.
fn install_contact_oracle_landscape(
    engine: &mut Engine,
    left_open: i32,
    right_open: i32,
    top_open: bool,
    bottom_open: bool,
) {
    const WIDTH: u32 = 24;
    const HEIGHT: i32 = 16;

    let mut bytes = vec![0u8; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT {
        for x in 0..WIDTH as i32 {
            let mut byte = u8::from(y >= 10);
            if y >= 11 && (3..=5).contains(&x) {
                byte = 2;
            }
            if (17..=18).contains(&x) && y >= 6 {
                byte = 1;
            }
            bytes[y as usize * WIDTH as usize + x as usize] = byte;
        }
    }
    let mut densities = vec![0; 128];
    densities[1] = 50;
    densities[2] = 30;
    let mut material_names = vec![None; 128];
    material_names[1] = Some("Earth".to_string());
    material_names[2] = Some("Water".to_string());

    let mut landscape = Landscape::flat(WIDTH, HEIGHT);
    landscape.set_pixel_grid(PixelGrid::new(
        WIDTH,
        HEIGHT as u32,
        bytes,
        densities,
        material_names,
        vec![None; 128],
    ));
    landscape.set_border_open(left_open, right_open, top_open, bottom_open);
    let vehicle = engine
        .materials
        .id_of("Vehicle")
        .expect("the fixture declares Vehicle");
    landscape.set_vehicle_material(Some(vehicle));
    engine.set_landscape(landscape);
}

/// The material library the `shape_contact` grid's bytes map onto.
fn contact_oracle_materials() -> clonk_resources::MaterialLibrary {
    clonk_resources::MaterialLibrary::parse(
        r#"
        [Material Earth]
        Name=Earth
        Density=50

        [Material Water]
        Name=Water
        Density=30

        [Material Vehicle]
        Name=Vehicle
        Density=100
        "#,
    )
    .expect("contact oracle materials parse")
}

fn expect_json_eq(section: &str, index: usize, field: &str, cpp: Value, rust: Value) {
    if cpp != rust {
        write_parity_diff_from_environment(section, index, field, cpp.clone(), rust.clone());
    }
    assert_eq!(
        cpp, rust,
        "PARITY DIVERGENCE in `{section}` entry {index} field `{field}`: \
         C++ golden = {cpp}, Rust = {rust}"
    );
}

fn write_parity_diff_from_environment(
    section: &str,
    index: usize,
    field: &str,
    cpp: Value,
    rust: Value,
) {
    let directory = std::env::var_os("LC_TEST_ARTIFACT_DIR")
        .or_else(|| std::env::var_os("LC_DEV_CHECK_ARTIFACT_DIR"));
    let Some(directory) = directory else {
        return;
    };
    match write_parity_diff_artifact(
        std::path::Path::new(&directory),
        section,
        index,
        field,
        cpp,
        rust,
    ) {
        Ok(path) => eprintln!("C++/Rust parity diff: {}", path.display()),
        Err(error) => eprintln!("failed to preserve C++/Rust parity diff: {error}"),
    }
}

fn write_parity_diff_artifact(
    directory: &std::path::Path,
    section: &str,
    index: usize,
    field: &str,
    cpp: Value,
    rust: Value,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("cpp-rust-diff.json");
    let artifact = serde_json::json!({
        "schema": "legacyclonk.cpp-rust-diff.v1",
        "section": section,
        "entry": index,
        "field": field,
        "cpp": cpp,
        "rust": rust,
        "golden": "parity/golden/parity_golden.json",
        "reproduce": "cargo xtask parity verify",
    });
    let temporary = directory.join(format!(".cpp-rust-diff-{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&artifact).map_err(std::io::Error::other)?;
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, &path)?;
    Ok(path)
}

#[test]
fn parity_divergence_artifact_is_structured_and_reproducible() {
    let temp = tempfile::tempdir().expect("temporary artifact directory");
    let path = write_parity_diff_artifact(
        temp.path(),
        "movement[gravity]",
        7,
        "fix_y",
        serde_json::json!(65_536),
        serde_json::json!(65_535),
    )
    .expect("parity artifact writes");
    let artifact: Value =
        serde_json::from_reader(std::fs::File::open(path).expect("parity artifact opens"))
            .expect("parity artifact parses");

    assert_eq!(artifact["schema"], "legacyclonk.cpp-rust-diff.v1");
    assert_eq!(artifact["section"], "movement[gravity]");
    assert_eq!(artifact["entry"], 7);
    assert_eq!(artifact["field"], "fix_y");
    assert_eq!(artifact["cpp"], 65_536);
    assert_eq!(artifact["rust"], 65_535);
    assert_eq!(artifact["golden"], "parity/golden/parity_golden.json");
    assert_eq!(artifact["reproduce"], "cargo xtask parity verify");
}

/// Reconstruct the `script_value_convert` source value the oracle emitted for a
/// given case name (must stay in sync with `conv_cases` in oracle_main.cpp).
fn convert_case_value(name: &str) -> ScriptValue {
    match name {
        "nil" => ScriptValue::Nil,
        "int_0" => ScriptValue::Int(0),
        "int_5000" => ScriptValue::Int(5000),
        "int_9999" => ScriptValue::Int(9999),
        "int_10000" => ScriptValue::Int(10000),
        "int_neg1" => ScriptValue::Int(-1),
        "bool_true" => ScriptValue::Bool(true),
        "bool_false" => ScriptValue::Bool(false),
        "id_CLNK" => ScriptValue::C4Id("CLNK".to_string()),
        "string" => ScriptValue::String("x".to_string().into()),
        "array" => ScriptValue::Array(Vec::new()),
        "map" => ScriptValue::Proplist(ValueMap::new()),
        other => panic!("unknown script_value_convert case `{other}`"),
    }
}

fn action_direction_engine() -> (Engine, crate::ObjectId) {
    let mut definition =
        Definition::from_script("WIPF", "Wipf", "#strict\n").expect("oracle fixture compiles");
    definition.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([
            (
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_directions(2)
                    .with_length(18)
                    .with_delay(2)
                    .with_next("Walk")
                    .with_turn_action("Turn"),
            ),
            (
                "Turn".to_string(),
                ActionSpec::default()
                    .with_procedure("NONE")
                    .with_directions(2)
                    .with_length(6)
                    .with_delay(2)
                    .with_next("Walk"),
            ),
        ]),
    );
    definition.set_physical(PhysicalInfo {
        walk: 80_000,
        ..PhysicalInfo::default()
    });

    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine
        .register_definition(definition)
        .expect("oracle fixture registers");
    let id = engine
        .spawn_object(
            SpawnConfig::new("WIPF")
                .with_position(crate::Vector2::new(541, 629))
                .with_fixed_position(FixedVec2::new(
                    C4Fixed::from_raw(35_468_082),
                    C4Fixed::from_raw(41_222_142),
                ))
                .with_fixed_velocity(FixedVec2::new(
                    C4Fixed::from_raw(-52_430),
                    C4Fixed::from_raw(65_534),
                ))
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right)
                .with_category(CATEGORY_OBJECT)
                .with_mobile(true)
                .with_loaded(true),
        )
        .expect("oracle fixture spawns");
    (engine, id)
}

fn action_push_pull_fight_direction_engine(case: &Value) -> (Engine, crate::ObjectId) {
    let name = case["name"]
        .as_str()
        .expect("procedure-direction case has a name");
    let (action_name, procedure, walk) = match name {
        "push_positive_subpixel" => ("Push", "PUSH", 1),
        "pull_positive_subpixel" => ("Pull", "PULL", 1),
        "fight_target_right_negative_velocity" | "fight_equal_x_negative_velocity" => {
            ("Fight", "FIGHT", 35_000)
        }
        other => panic!("unknown procedure-direction case `{other}`"),
    };
    let actor_script = r#"#strict
local turn_starts, turn_start_dir;
protected func TurnStart()
{
    turn_starts = turn_starts + 1;
    turn_start_dir = GetDir();
    return true;
}
"#;
    let mut actor = Definition::from_script("ACTR", "Actor", actor_script)
        .expect("procedure-direction actor compiles");
    actor.set_c4_callback_convention(true);
    actor.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    actor.set_physical(PhysicalInfo {
        walk,
        push: 100_000,
        ..PhysicalInfo::default()
    });
    actor.configure_actions(
        Some(action_name.to_string()),
        HashMap::from([
            (
                action_name.to_string(),
                ActionSpec::default()
                    .with_procedure(procedure)
                    .with_directions(2)
                    .with_flip_dir(1)
                    .with_turn_action("Turn"),
            ),
            (
                "Turn".to_string(),
                ActionSpec::default()
                    .with_directions(2)
                    .with_flip_dir(1)
                    .with_start_call("TurnStart"),
            ),
        ]),
    );

    let mut target = Definition::from_script("TRGT", "Target", "#strict\n")
        .expect("procedure-direction target compiles");
    target.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    target.set_grab(1);
    target.set_mass(200);
    target.configure_actions(
        Some("Fight".to_string()),
        HashMap::from([(
            "Fight".to_string(),
            ActionSpec::default()
                .with_procedure("FIGHT")
                .with_directions(2),
        )]),
    );

    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine
        .register_definition(actor)
        .expect("procedure-direction actor registers");
    engine
        .register_definition(target)
        .expect("procedure-direction target registers");
    let target_id = engine
        .spawn_object(
            SpawnConfig::new("TRGT")
                .with_category(if procedure == "FIGHT" {
                    CATEGORY_OBJECT
                } else {
                    CATEGORY_VEHICLE
                })
                .with_position(crate::Vector2::new(i(case, "target_x") as i32, 0))
                .with_action(ActionState::new("Fight")),
        )
        .expect("procedure-direction target spawns");
    let mut action = ActionState::new(action_name);
    action.target = Some(target_id);
    let direction = match i(case, "initial_direction") {
        0 => Direction::Left,
        1 => Direction::Right,
        other => panic!("invalid procedure-direction fixture direction {other}"),
    };
    let initial_xdir = if procedure == "FIGHT" {
        C4Fixed::from_raw(i(case, "xdir_raw") as i32)
    } else {
        C4Fixed::ZERO
    };
    let actor_id = engine
        .spawn_object(
            SpawnConfig::new("ACTR")
                .with_category(CATEGORY_OBJECT)
                .with_position(crate::Vector2::new(i(case, "actor_x") as i32, 0))
                .with_action(action)
                .with_direction(direction)
                .with_command_direction(CommandDirection::Right)
                .with_fixed_velocity(FixedVec2::new(initial_xdir, C4Fixed::ZERO))
                .with_mobile(true),
        )
        .expect("procedure-direction actor spawns");
    let actor_idx = engine
        .find_object_index(actor_id)
        .expect("procedure-direction actor exists");
    engine.objects[actor_idx].state.draw_transform = None;
    (engine, actor_id)
}

fn swim_action_direction_engine() -> (Engine, crate::ObjectId) {
    let mut definition =
        Definition::from_script("FISH", "Fish", "#strict\n").expect("oracle fixture compiles");
    definition.configure_actions(
        Some("Swim".to_string()),
        HashMap::from([
            (
                "Swim".to_string(),
                ActionSpec::default()
                    .with_procedure("SWIM")
                    .with_directions(2)
                    .with_length(20)
                    .with_delay(1)
                    .with_next("Swim")
                    .with_turn_action("Turn"),
            ),
            (
                "Turn".to_string(),
                ActionSpec::default()
                    .with_procedure("SWIM")
                    .with_directions(2)
                    .with_length(15)
                    .with_delay(3)
                    .with_next("Swim"),
            ),
        ]),
    );
    definition.set_physical(PhysicalInfo {
        swim: 100_000,
        ..PhysicalInfo::default()
    });

    let mut engine = Engine::with_seed(0);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine
        .register_definition(definition)
        .expect("oracle fixture registers");
    let mut action = ActionState::new("Swim");
    action.phase = 3;
    action.time = 103;
    let id = engine
        .spawn_object(
            SpawnConfig::new("FISH")
                .with_position(crate::Vector2::new(873, 438))
                .with_fixed_position(FixedVec2::new(
                    C4Fixed::from_raw(57_212_928),
                    C4Fixed::from_raw(28_737_532),
                ))
                .with_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, C4Fixed::from_raw(-6_556)))
                .with_action(action)
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Left)
                .with_category(CATEGORY_OBJECT)
                .with_mobile(true)
                .with_loaded(true),
        )
        .expect("oracle fixture spawns");
    let idx = engine.find_object_index(id).expect("swimmer exists");
    engine.objects[idx].state.in_liquid = true;
    (engine, id)
}

fn action_callbacks_engine(case: &str) -> (Engine, crate::ObjectId) {
    let script = r#"#strict
local callbackOrder, startCount, oldCount;

protected func Activity()
{
    SetAction("New");
    return 1;
}

protected func OnStart()
{
    callbackOrder = callbackOrder * 10 + 1;
    startCount = startCount + 1;
    return 1;
}

protected func OnEnd()
{
    callbackOrder = callbackOrder * 10 + 2;
    oldCount = oldCount + 1;
    return 1;
}

protected func OnAbort()
{
    callbackOrder = callbackOrder * 10 + 3;
    oldCount = oldCount + 1;
    return 1;
}
"#;
    let mut definition =
        Definition::from_script("ACBK", "Action callbacks", script).expect("fixture compiles");
    definition.set_c4_callback_convention(true);
    let mut old = ActionSpec::default();
    match case {
        "script_start_only" => {
            definition.set_timer(1);
            definition.set_timer_call(Some("Activity".to_string()));
        }
        "script_start_abort" => {
            definition.set_timer(1);
            definition.set_timer_call(Some("Activity".to_string()));
            old = old.with_abort_call("OnAbort");
        }
        "natural_start_end" => {
            old = old
                .with_length(1)
                .with_delay(1)
                .with_next("New")
                .with_end_call("OnEnd");
        }
        other => panic!("unknown action_callbacks case `{other}`"),
    }
    definition.configure_actions(
        Some("Old".to_string()),
        HashMap::from([
            ("Old".to_string(), old),
            (
                "New".to_string(),
                ActionSpec::default().with_start_call("OnStart"),
            ),
        ]),
    );

    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("fixture registers");
    let id = engine
        .spawn_object(
            SpawnConfig::new("ACBK")
                .with_action(ActionState::new("Old"))
                .with_category(CATEGORY_OBJECT)
                .with_local_vars(HashMap::from([
                    ("callbackOrder".to_string(), ScriptValue::Int(0)),
                    ("startCount".to_string(), ScriptValue::Int(0)),
                    ("oldCount".to_string(), ScriptValue::Int(0)),
                ]))
                .with_loaded(true),
        )
        .expect("fixture spawns");
    (engine, id)
}

fn action_callback_local(engine: &Engine, id: crate::ObjectId, name: &str) -> i64 {
    engine
        .find_object_index(id)
        .and_then(|idx| engine.objects[idx].state.local_vars.get(name))
        .and_then(|value| match value {
            ScriptValue::Int(value) => Some(i64::from(*value)),
            _ => None,
        })
        .unwrap_or(0)
}

fn connect_removal_engine(geometry_break: bool) -> (Engine, crate::ObjectId) {
    let script = r#"#strict
local callbackOrder, lineBreakCount, lineBreakArgumentPresent, lineBreakAutomatic, destructionCount;

protected func LineBreak(automatic)
{
    callbackOrder = callbackOrder * 10 + 1;
    lineBreakCount = lineBreakCount + 1;
    if (GetType(automatic) != 0) lineBreakArgumentPresent = 1;
    if (automatic) lineBreakAutomatic = 1;
    return 1;
}

protected func Destruction()
{
    callbackOrder = callbackOrder * 10 + 2;
    destructionCount = destructionCount + 1;
    return 1;
}
"#;
    let mut definition =
        Definition::from_script("RPLN", "CONNECT removal line", script).expect("fixture compiles");
    definition.set_c4_callback_convention(true);
    definition.set_line(1);
    if geometry_break {
        definition.set_shape_vertices(vec![crate::ObjectVertex::new(0, 0)]);
    }
    definition.configure_actions(
        Some("Connect".to_string()),
        HashMap::from([(
            "Connect".to_string(),
            ActionSpec::default().with_procedure("CONNECT"),
        )]),
    );

    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("fixture registers");
    if geometry_break {
        engine
            .register_script_definition("CEND", "CONNECT endpoint", "#strict\n")
            .expect("endpoint fixture registers");
    }
    let mut action = ActionState::new("Connect");
    if geometry_break {
        action.target = Some(
            engine
                .spawn_object(SpawnConfig::new("CEND").with_position(crate::Vector2::new(10, 0)))
                .expect("first endpoint spawns"),
        );
        action.target2 = Some(
            engine
                .spawn_object(SpawnConfig::new("CEND").with_position(crate::Vector2::new(20, 0)))
                .expect("second endpoint spawns"),
        );
    }
    let id = engine
        .spawn_object(
            SpawnConfig::new("RPLN")
                .with_action(action)
                .with_category(CATEGORY_OBJECT)
                .with_local_vars(HashMap::from([
                    ("callbackOrder".to_string(), ScriptValue::Int(0)),
                    ("lineBreakCount".to_string(), ScriptValue::Int(0)),
                    ("lineBreakArgumentPresent".to_string(), ScriptValue::Int(0)),
                    ("lineBreakAutomatic".to_string(), ScriptValue::Int(0)),
                    ("destructionCount".to_string(), ScriptValue::Int(0)),
                ]))
                .with_loaded(true),
        )
        .expect("fixture spawns");
    (engine, id)
}

fn expect_connect_removal_case(golden: &Value, section: &str, geometry_break: bool) {
    let case = &golden[section];
    let (mut engine, id) = connect_removal_engine(geometry_break);
    let idx = engine.find_object_index(id).expect("line exists");
    assert!(!engine
        .exec_connect_line(idx)
        .expect("CONNECT break branch executes"));
    expect_eq(
        section,
        0,
        "line_break_count",
        i(case, "line_break_count"),
        action_callback_local(&engine, id, "lineBreakCount"),
    );
    expect_eq(
        section,
        0,
        "line_break_argument_count",
        i(case, "line_break_argument_count"),
        action_callback_local(&engine, id, "lineBreakArgumentPresent"),
    );
    expect_eq(
        section,
        0,
        "line_break_automatic",
        i(case, "line_break_automatic"),
        action_callback_local(&engine, id, "lineBreakAutomatic"),
    );
    expect_eq(
        section,
        0,
        "destruction_count",
        i(case, "destruction_count"),
        action_callback_local(&engine, id, "destructionCount"),
    );
    expect_eq(
        section,
        0,
        "callback_order",
        i(case, "callback_order"),
        action_callback_local(&engine, id, "callbackOrder"),
    );
    let object = &engine.objects[idx];
    expect_eq(
        section,
        0,
        "status",
        i(case, "status"),
        i64::from(object.state.status.to_script_value()),
    );
}

fn solid_mask_sprite(alpha: u8) -> DefinitionSpriteImage {
    const WIDTH: u32 = 220;
    const HEIGHT: u32 = 87;
    const SOURCE_X: usize = 219;
    const SOURCE_Y: usize = 86;
    let mut pixels = vec![0; (WIDTH * HEIGHT * 4) as usize];
    pixels[(SOURCE_Y * WIDTH as usize + SOURCE_X) * 4 + 3] = alpha;
    DefinitionSpriteImage {
        width: WIDTH,
        height: HEIGHT,
        pixels: Arc::from(pixels.into_boxed_slice()),
        color_mask: None,
    }
}

/// Sprite whose pixels encode their own coordinates (R = x, G = y), so the
/// first pixel of a cropped facet recovers the source rect's origin.
fn coordinate_sprite(size: u32) -> DefinitionSpriteImage {
    let mut pixels = vec![0; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let base = ((y * size + x) * 4) as usize;
            pixels[base] = x as u8;
            pixels[base + 1] = y as u8;
            pixels[base + 3] = 255;
        }
    }
    DefinitionSpriteImage {
        width: size,
        height: size,
        pixels: Arc::from(pixels.into_boxed_slice()),
        color_mask: None,
    }
}

fn def_picture_scale_engine(scale_percent: u32, picture: DefinitionPicture) -> Engine {
    let mut definition =
        Definition::from_script("PSCL", "Picture Scale", "#strict\n").expect("fixture compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    definition.set_picture(Some(picture));
    // C4Def.cpp:745 `Scale = C4DefCore::Scale / 100.0f`, as wired at lib.rs:12841.
    definition.set_graphics_scale(scale_percent as f32 / 100.0);
    definition.set_sprite_image(Some(coordinate_sprite(256)));

    let mut engine = Engine::with_seed(7);
    engine
        .register_definition(definition)
        .expect("definition registers");
    engine
}

fn solid_mask_graphics_engine() -> (Engine, crate::ObjectId) {
    let mut definition =
        Definition::from_script("CTWR", "Castle Tower", "#strict\n").expect("fixture compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    definition.set_solid_mask(Some(DefinitionTargetRect::new(219, 86, 1, 1, 0, 0)));
    definition.set_sprite_image(Some(solid_mask_sprite(0)));
    definition.set_sprite_variants(HashMap::from([("2".to_string(), solid_mask_sprite(255))]));

    let mut engine = Engine::with_seed(7);
    let grid = PixelGrid::new(
        3,
        3,
        vec![0; 9],
        vec![0, 100, 100],
        vec![None, Some("Earth".into()), Some("Vehicle".into())],
        vec![None; 3],
    );
    let mut landscape = Landscape::flat(3, 3);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let id = engine
        .spawn_object(
            SpawnConfig::new("CTWR")
                .with_position(crate::Vector2::new(1, 1))
                .with_loaded(true),
        )
        .expect("tower spawns");
    (engine, id)
}

struct RuleGoalParityResolver {
    roots: Vec<PathBuf>,
}

impl LegacyDefinitionResolver for RuleGoalParityResolver {
    fn resolve_definition_groups(
        &self,
        scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let mut groups = Vec::new();
        let normalized = identifier.replace('\\', "/");
        let path = Path::new(&normalized);

        if let Ok(child) = scenario.open_child(path) {
            groups.push(child);
        }
        for root in &self.roots {
            let candidate = root.join(path);
            if !candidate.exists() {
                continue;
            }
            let group = Group::open(&candidate)?;
            if groups
                .iter()
                .all(|existing| existing.root() != group.root())
            {
                groups.push(group);
            }
        }
        if groups.is_empty() {
            Err(ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            })
        } else {
            Ok(groups)
        }
    }
}

fn golden_scenario_id_list(case: &Value, key: &str) -> Vec<ScenarioIdListEntry> {
    case[key]
        .as_array()
        .unwrap_or_else(|| panic!("network rule/goal case field `{key}` is an array"))
        .iter()
        .map(|entry| {
            ScenarioIdListEntry::new(
                entry["id"]
                    .as_str()
                    .unwrap_or_else(|| panic!("`{key}` entry has an id")),
                i(entry, "count") as i32,
            )
        })
        .collect()
}

fn scenario_id_list_text(entries: &[ScenarioIdListEntry]) -> String {
    entries
        .iter()
        .map(|entry| format!("{}={};", entry.id, entry.count))
        .collect()
}

fn indexed_bmp_2x2() -> Vec<u8> {
    const WIDTH: u32 = 2;
    const HEIGHT: u32 = 2;
    const STRIDE: usize = 4;
    const DATA_OFFSET: usize = 14 + 40 + 256 * 4;
    let file_size = DATA_OFFSET + STRIDE * HEIGHT as usize;
    let mut bytes = Vec::with_capacity(file_size);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(file_size as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(DATA_OFFSET as u32).to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&(WIDTH as i32).to_le_bytes());
    bytes.extend_from_slice(&(HEIGHT as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8u16.to_le_bytes());
    for _ in 0..4 {
        bytes.extend_from_slice(&0u32.to_le_bytes());
    }
    bytes.extend_from_slice(&256u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.resize(file_size, 0);
    bytes
}

fn rust_network_rule_goal_placement(case: &Value, case_index: usize) {
    let name = case["name"]
        .as_str()
        .expect("network rule/goal case has a name");
    let scenario_rules = golden_scenario_id_list(case, "scenario_rules");
    let scenario_goals = golden_scenario_id_list(case, "scenario_goals");
    let parameter_rules = golden_scenario_id_list(case, "parameter_rules");
    let parameter_goals = golden_scenario_id_list(case, "parameter_goals");

    let fixture = tempfile::tempdir().expect("network rule/goal parity fixture");
    let definitions_root = fixture.path().join("Defs.c4d");
    let goal_ids = scenario_goals
        .iter()
        .chain(parameter_goals.iter())
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let mut definition_ids = scenario_rules
        .iter()
        .chain(scenario_goals.iter())
        .chain(parameter_rules.iter())
        .chain(parameter_goals.iter())
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>();
    definition_ids.insert("GOAL".to_string());
    for id in definition_ids {
        let definition = definitions_root.join(format!("{id}.c4d"));
        std::fs::create_dir_all(&definition).expect("definition directory");
        let category = if goal_ids.contains(id.as_str()) {
            4096
        } else {
            8192
        };
        std::fs::write(
            definition.join("DefCore.txt"),
            format!("[DefCore]\nid={id}\nName={id}\nCategory={category}\n"),
        )
        .expect("definition core writes");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(definition.join("Graphics.png"))
            .expect("definition graphics writes");
    }

    let scenario_directory = fixture.path().join("RuleGoalParity.c4s");
    std::fs::create_dir_all(&scenario_directory).expect("scenario directory");
    let energy_default = if name == "harpoonrace_join_data" {
        String::new()
    } else {
        "StructNeedEnergy=0\n".to_string()
    };
    std::fs::write(
        scenario_directory.join("Scenario.txt"),
        format!(
            "[Head]\nTitle=RuleGoalParity\n\n\
             [Definitions]\nDefinition1=Defs.c4d\n\n\
             [Game]\n{energy_default}Goals={}\nRules={}\n\n\
             [Landscape]\nMapZoom=10\n",
            scenario_id_list_text(&scenario_goals),
            scenario_id_list_text(&scenario_rules),
        ),
    )
    .expect("scenario core writes");
    std::fs::write(scenario_directory.join("Landscape.bmp"), indexed_bmp_2x2())
        .expect("scenario landscape writes");

    let resolver = RuleGoalParityResolver {
        roots: vec![fixture.path().to_path_buf()],
    };
    let scenario =
        Scenario::load_from_path_with(&scenario_directory, &resolver).expect("scenario loads");

    if name == "harpoonrace_join_data" {
        let defaults = scenario
            .lobby_metadata()
            .expect("legacy scenario has lobby metadata")
            .game_parameter_defaults();
        let rust_rules = defaults
            .rules()
            .iter()
            .map(|entry| serde_json::json!({"id": entry.id(), "count": entry.count()}))
            .collect::<Vec<_>>();
        let rust_goals = defaults
            .goals()
            .iter()
            .map(|entry| serde_json::json!({"id": entry.id(), "count": entry.count()}))
            .collect::<Vec<_>>();
        expect_json_eq(
            "network_rule_goal_placement",
            case_index,
            "parameter_rules",
            case["parameter_rules"].clone(),
            Value::Array(rust_rules),
        );
        expect_json_eq(
            "network_rule_goal_placement",
            case_index,
            "parameter_goals",
            case["parameter_goals"].clone(),
            Value::Array(rust_goals),
        );
    }

    let synchronized =
        GameParameterRuleGoalLists::new(parameter_rules.clone(), parameter_goals.clone());
    let mut engine = Engine::with_seed(7);
    scenario
        .apply_before_players_for_game_start(
            &mut engine,
            true,
            None,
            None,
            None,
            Some(&synchronized),
            None,
        )
        .expect("network scenario applies");
    let snapshot = engine.snapshot();
    let rule_ids = parameter_rules
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let goal_ids = parameter_goals
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let rust_rule_objects = snapshot
        .objects
        .iter()
        .filter(|object| rule_ids.contains(object.definition_id.as_str()))
        .map(|object| Value::String(object.definition_id.clone()))
        .collect();
    let rust_goal_objects = snapshot
        .objects
        .iter()
        .filter(|object| goal_ids.contains(object.definition_id.as_str()))
        .map(|object| Value::String(object.definition_id.clone()))
        .collect();
    expect_json_eq(
        "network_rule_goal_placement",
        case_index,
        "rule_objects",
        case["rule_objects"].clone(),
        Value::Array(rust_rule_objects),
    );
    expect_json_eq(
        "network_rule_goal_placement",
        case_index,
        "goal_objects",
        case["goal_objects"].clone(),
        Value::Array(rust_goal_objects),
    );
}

fn player_join_capacity_config(name: &str, player_info_id: i32) -> JoinPlayerConfig {
    JoinPlayerConfig {
        name: name.to_string(),
        player_info_id,
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
    }
}

fn player_names(engine: &Engine) -> Value {
    Value::Array(
        engine
            .players()
            .map(|player| Value::String(player.name().to_string()))
            .collect(),
    )
}

fn savegame_matching_entry(case: &Value, side: &str) -> crate::control::ControlPlayerInfoEntry {
    let name = case[format!("{side}_name")]
        .as_array()
        .expect("savegame_player_matching name is a C++ oracle byte array")
        .iter()
        .map(|byte| byte.as_u64().expect("name byte is a number") as u8)
        .collect::<Vec<_>>();
    // The oracle emits Latin-1 bytes, which is what the engine stores.
    let filename = case[format!("{side}_filename")]
        .as_str()
        .expect("savegame_player_matching filename is a string")
        .as_bytes()
        .to_vec();
    crate::control::ControlPlayerInfoEntry {
        name: crate::control::LegacyCString::from_bytes(name)
            .expect("oracle names carry no interior NUL"),
        filename: crate::control::LegacyCString::from_bytes(filename)
            .expect("oracle filenames carry no interior NUL"),
        original_color: i(case, &format!("{side}_color")) as u32,
        ..Default::default()
    }
}

fn rust_savegame_player_matching(case: &Value, case_index: usize) {
    const SECTION: &str = "savegame_player_matching";
    let current = savegame_matching_entry(case, "current");
    let saved = savegame_matching_entry(case, "saved");
    let cpp = case["matches"]
        .as_array()
        .expect("savegame_player_matching matches is a C++ oracle array");
    assert_eq!(
        cpp.len(),
        4,
        "{SECTION} case {case_index} records one result per MatchingLevel"
    );
    for (level, expected) in cpp.iter().enumerate() {
        let expected = expected
            .as_bool()
            .expect("savegame_player_matching result is a bool");
        expect_json_eq(
            SECTION,
            case_index,
            &format!("matches[{level}]"),
            Value::Bool(expected),
            Value::Bool(crate::savegame_association::savegame_players_match(
                &current,
                &saved,
                level as u8,
            )),
        );
    }
}

/// C4IDList.cpp:33-103 — component order, which participates in the replay
/// hash but had no comparable field on the C++ side.
///
/// The list is a `std::vector<Entry>`, so position is meaningful and the same
/// ID may appear more than once with independent counts — the shipped Bazooka
/// `DefCore` does exactly that. A comparator that only checked ID/count pairs
/// would pass a model that collapsed the repeat, which is why the rows carry
/// the ordered entries themselves.
fn rust_component_order(case: &Value, case_index: usize) {
    const SECTION: &str = "component_order";

    let entries = |key: &str| -> Vec<(String, i32)> {
        case[key]
            .as_array()
            .unwrap_or_else(|| panic!("{SECTION} {key} is a C++ oracle array"))
            .iter()
            .map(|entry| {
                (
                    entry["id"]
                        .as_str()
                        .expect("component id is a string")
                        .to_owned(),
                    i(entry, "count") as i32,
                )
            })
            .collect()
    };

    // Built with `push`, not `set`: the parsed DefCore appends every entry it
    // reads, which is the only way a repeat can exist at all.
    let mut list = entries("initial")
        .into_iter()
        .map(|(id, count)| (crate::DefinitionId::from(id.as_str()), count))
        .collect::<crate::ComponentList>();

    if let Some(set) = case.get("set").filter(|set| !set.is_null()) {
        list.set(
            crate::DefinitionId::from(set["id"].as_str().expect("set id is a string")),
            i(set, "count") as i32,
        );
    }

    expect_json_eq(
        SECTION,
        case_index,
        "entries",
        serde_json::json!(entries("entries")
            .into_iter()
            .map(|(id, count)| serde_json::json!({"id": id, "count": count}))
            .collect::<Vec<_>>()),
        serde_json::json!(list
            .iter()
            .map(|(id, count)| serde_json::json!({"id": id.as_str(), "count": count}))
            .collect::<Vec<_>>()),
    );
    expect_eq(
        SECTION,
        case_index,
        "number_of_ids",
        i(case, "number_of_ids"),
        list.len() as i64,
    );

    // `GetIDCount` resolves through `findId`, which returns the **first**
    // matching entry; a later repeat is unreachable by ID.
    for lookup in case["lookups"]
        .as_array()
        .expect("component_order lookups is a C++ oracle array")
    {
        let id = lookup["id"].as_str().expect("lookup id is a string");
        expect_eq(
            SECTION,
            case_index,
            &format!("lookups[{id}]"),
            i(lookup, "count"),
            list.get(id).unwrap_or(0) as i64,
        );
    }
}

/// C4PlayerInfo.cpp:1373-1391 — the pass loop the four matching levels run
/// inside, which decides *which* savegame player each join ends up with.
///
/// The per-level predicate is already compared by `savegame_player_matching`;
/// what this adds is the loop's own semantics — pass ordering, first-accepting
/// candidate, the eligibility test, and which associations C++ calls "wild".
fn rust_savegame_association(case: &Value, case_index: usize) {
    const SECTION: &str = "savegame_association";

    let players = |key: &str| -> Vec<crate::control::ControlPlayerInfoEntry> {
        case[key]
            .as_array()
            .unwrap_or_else(|| panic!("{SECTION} {key} is a C++ oracle array"))
            .iter()
            .map(|player| {
                let name = player["name"]
                    .as_array()
                    .expect("player name is a C++ oracle byte array")
                    .iter()
                    .map(|byte| byte.as_u64().expect("name byte is a number") as u8)
                    .collect::<Vec<_>>();
                crate::control::ControlPlayerInfoEntry {
                    id: i(player, "id") as i32,
                    name: crate::control::LegacyCString::from_bytes(name)
                        .expect("oracle names carry no interior NUL"),
                    filename: crate::control::LegacyCString::from_bytes(
                        player["filename"]
                            .as_str()
                            .expect("player filename is a string")
                            .as_bytes()
                            .to_vec(),
                    )
                    .expect("oracle filenames carry no interior NUL"),
                    original_color: u(player, "color") as u32,
                    ..Default::default()
                }
            })
            .collect()
    };

    let mut participants = players("participants");
    let savegame_players = players("savegame_players");
    let wild = crate::savegame_association::associate_savegame_players(
        &mut participants,
        &savegame_players,
    );

    expect_json_eq(
        SECTION,
        case_index,
        "associations",
        case["associations"].clone(),
        serde_json::json!(participants
            .iter()
            .map(|player| player.savegame_player)
            .collect::<Vec<_>>()),
    );
    expect_json_eq(
        SECTION,
        case_index,
        "wild",
        case["wild"].clone(),
        serde_json::json!(wild
            .iter()
            .map(|takeover| serde_json::json!({
                "participant": takeover.participant,
                "savegame_player": takeover.savegame_player,
            }))
            .collect::<Vec<_>>()),
    );
}

fn rust_player_join_capacity(case: &Value, case_index: usize) {
    const SECTION: &str = "player_join_capacity";
    let initial_names = case["names_before"]
        .as_array()
        .expect("player join capacity names_before is a C++ oracle array");
    let mut engine = Engine::with_seed(0);
    for (index, name) in initial_names.iter().enumerate() {
        let name = name
            .as_str()
            .expect("player join capacity initial name is a string");
        engine
            .join_player(player_join_capacity_config(name, index as i32 + 1))
            .unwrap_or_else(|error| panic!("initial player `{name}` joins: {error}"));
    }

    expect_eq(
        SECTION,
        case_index,
        "count_before",
        i(case, "count_before"),
        engine.players().count() as i64,
    );
    expect_json_eq(
        SECTION,
        case_index,
        "names_before",
        case["names_before"].clone(),
        player_names(&engine),
    );

    let maximum = i(case, "max_players") as i32;
    let joining_name = case["joining_name"]
        .as_str()
        .expect("player join capacity joining_name is a string");
    engine.set_max_players(maximum);
    let result = engine.join_player(player_join_capacity_config(
        joining_name,
        initial_names.len() as i32 + 1,
    ));
    let accepted = match result {
        Ok(_) => true,
        Err(EngineError::TooManyPlayers { .. }) => false,
        Err(error) => panic!("unexpected player join error for `{joining_name}`: {error}"),
    };

    expect_json_eq(
        SECTION,
        case_index,
        "accepted",
        case["accepted"].clone(),
        serde_json::json!(accepted),
    );
    expect_eq(
        SECTION,
        case_index,
        "count_after",
        i(case, "count_after"),
        engine.players().count() as i64,
    );
    expect_json_eq(
        SECTION,
        case_index,
        "names_after",
        case["names_after"].clone(),
        player_names(&engine),
    );
}

#[test]
fn parity_differential_matches_cpp_golden() {
    let golden = load_golden();

    // C4SGame::ConvertGoals and C4Game::InitRules/InitGoals
    // (oracle-src-pinned src/C4Scenario.cpp:506-556;
    // src/C4Game.cpp:4056-4076). HarpoonRace drives the same authored lists
    // through both converters; the count-edge case then makes local
    // Scenario.txt leakage observable while applying the synchronized lists.
    for (case_index, case) in golden["network_rule_goal_placement"]
        .as_array()
        .expect("network_rule_goal_placement is a C++ oracle array")
        .iter()
        .enumerate()
    {
        rust_network_rule_goal_placement(case, case_index);
    }

    // C4PlayerList.cpp:172-178,288-294. The C++ oracle compiles the exact
    // linked-list count and Join capacity gate; Rust seeds and attempts every
    // row through Engine::join_player, including the zero-is-closed boundary.
    let player_join_capacity_cases = golden["player_join_capacity"]
        .as_array()
        .expect("player_join_capacity is a C++ oracle array");
    let player_join_capacity_names = player_join_capacity_cases
        .iter()
        .map(|case| {
            case["name"]
                .as_str()
                .expect("player_join_capacity case has a name")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        player_join_capacity_names,
        [
            "zero_rejects_empty",
            "below_limit_accepts",
            "at_limit_rejects",
        ],
        "player_join_capacity must retain its exact ordered three-row matrix"
    );
    for (case_index, case) in player_join_capacity_cases.iter().enumerate() {
        rust_player_join_capacity(case, case_index);
    }

    // C4PlayerInfo.cpp:1102-1118. The four MatchingLevel passes
    // RestoreSavegameInfos runs (:1373-1391) when it associates joining players
    // with a savegame's stored players. The C++ oracle compiles the extracted
    // switch, including PML_PlrFileName's fallthrough into PML_PlrName, so a
    // file-name match alone never associates.
    let savegame_matching_cases = golden["savegame_player_matching"]
        .as_array()
        .expect("savegame_player_matching is a C++ oracle array");
    assert_eq!(
        savegame_matching_cases.len(),
        9,
        "savegame_player_matching must retain its exact nine-row matrix"
    );
    for (case_index, case) in savegame_matching_cases.iter().enumerate() {
        rust_savegame_player_matching(case, case_index);
    }

    // C4PlayerInfo.cpp:1373-1391. The pass loop those levels run inside: no
    // shipped scenario sets Head.SaveGame, so this path is reachable only from
    // runtime-written saves and had no differential coverage at all.
    let savegame_association_cases = golden["savegame_association"]
        .as_array()
        .expect("savegame_association is a C++ oracle array");
    assert_eq!(
        savegame_association_cases.len(),
        6,
        "savegame_association must retain its exact six-row matrix"
    );
    for (case_index, case) in savegame_association_cases.iter().enumerate() {
        rust_savegame_association(case, case_index);
    }

    // C4IDList.cpp:33-103. Component order is inside the replay hash, so a
    // model that reordered or collapsed entries is a desync this comparator
    // can now see directly rather than only as an eventual hash mismatch.
    let component_order_cases = golden["component_order"]
        .as_array()
        .expect("component_order is a C++ oracle array");
    assert_eq!(
        component_order_cases.len(),
        6,
        "component_order must retain its exact six-row matrix"
    );
    for (case_index, case) in component_order_cases.iter().enumerate() {
        rust_component_order(case, case_index);
    }

    // 0. C4PXSSystem slot allocation (C4PXS.cpp:181-204, 426-437). The order a
    //    freed slot is handed back in decides the chunk-major execution order
    //    of every later pass, so it is compared against the real `New` rather
    //    than assumed. The golden frees high-index-first, which a
    //    most-recently-freed allocator would answer differently on the very
    //    next call.
    {
        let mut system = crate::pxs::PxsSystem::default();
        let mut live: Vec<(usize, usize)> = Vec::new();
        let material = crate::material::MaterialId::new(1).expect("material 1");
        // The oracle locates a returned pointer; the port has no pointer to
        // hand back, so each pixel carries a unique x and is located by it.
        let locate = |system: &crate::pxs::PxsSystem, tag: i32| {
            for chunk in 0..crate::pxs::PXS_MAX_CHUNK {
                for slot in 0..crate::pxs::PXS_CHUNK_SIZE {
                    if let Some(pxs) = system.peek_slot(chunk, slot) {
                        if pxs.x == itofix(tag) {
                            return Some((chunk, slot));
                        }
                    }
                }
            }
            None
        };

        for (idx, e) in golden["pxs_allocation"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let step = e["step"].as_str().unwrap_or_default();
            let (chunk, slot) = (i(e, "chunk"), i(e, "slot"));
            if let Some(freed) = step.strip_prefix("free") {
                let which: usize = freed.parse().expect("a free step names an index");
                let (chunk_at, slot_at) = live[which];
                expect_eq("pxs_allocation", idx, "chunk", chunk, chunk_at as i64);
                expect_eq("pxs_allocation", idx, "slot", slot, slot_at as i64);
                system.clear_slot(chunk_at, slot_at);
                live.remove(which);
                continue;
            }
            let tag = idx as i32;
            assert!(
                system.create(material, itofix(tag), itofix(0), itofix(0), itofix(0)),
                "the golden sequence never exhausts the chunk table"
            );
            let placed = locate(&system, tag).expect("the created pixel is in a slot");
            expect_eq("pxs_allocation", idx, "chunk", chunk, placed.0 as i64);
            expect_eq("pxs_allocation", idx, "slot", slot, placed.1 as i64);
            live.push(placed);
        }
    }

    // 0b. mrfPoof's synchronised-draw discipline (C4Material.cpp:663-688). The
    //     arm extracts the landscape material, then draws Rnd3 twice: smoke on
    //     the first zero, a positional sound on the second. Both draws happen
    //     unconditionally and in that order, and — the parity fact worth
    //     pinning — neither touches the synchronised ledger, because Rnd3 reads
    //     the Randomize3 table rather than `Random`. A port that skipped the
    //     sound's draw when it had no sound to play, or that routed either
    //     through `Random`, would desynchronise everything downstream.
    for (idx, e) in golden["material_poof_reaction"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let seed = i(e, "seed") as i32;
        let mut rng = crate::LcgRng::new(seed as u32);
        rng.randomize3();
        expect_eq(
            "material_poof_reaction",
            idx,
            "random_count",
            i(e, "random_count"),
            i64::from(rng.count),
        );
        expect_eq(
            "material_poof_reaction",
            idx,
            "random_hold",
            i(e, "random_hold"),
            i64::from(rng.hold),
        );

        // The two draws the arm makes, in order.
        let smoke = i32::from(rng.rnd3() == 0);
        let sound = i32::from(rng.rnd3() == 0);
        expect_eq(
            "material_poof_reaction",
            idx,
            "smoke",
            i(e, "smoke"),
            smoke as i64,
        );
        expect_eq(
            "material_poof_reaction",
            idx,
            "sound",
            i(e, "sound"),
            sound as i64,
        );

        // And neither draw moved the synchronised ledger.
        expect_eq(
            "material_poof_reaction",
            idx,
            "random_count after rnd3",
            i(e, "random_count"),
            i64::from(rng.count),
        );
    }

    // 0c. C4MassMoverSet::Create's slot scan (C4MassMover.cpp:67-94). The
    //     search starts AFTER `CreatePtr` and wraps at the chunk end, so a slot
    //     freed behind the cursor is not reused until the cursor comes round to
    //     it — the opposite of the PXS allocator above, which always hands back
    //     the lowest free slot. Where a mover lands decides whether the frame's
    //     descending `Execute` pass reaches it again this pass or only the next,
    //     so the sequence of chosen slots is parity state.
    //
    //     The oracle stubs `Init` to succeed, which also holds its `Count` at
    //     zero, so `Create`'s `Count == C4MassMoverChunk` gate never fires there
    //     and this section pins the scan alone; the gate has its own test
    //     (`create_gate_is_exact_equality_on_count`).
    {
        let mut set = crate::mass_mover::MassMoverSet::default();
        let material = crate::material::MaterialId::new(1).expect("material 1");
        let take = |set: &mut crate::mass_mover::MassMoverSet| {
            set.find_free_slot()
                .map(|index| {
                    set.fill_slot(
                        index,
                        crate::mass_mover::MassMover {
                            mat: material,
                            x: 7,
                            y: 9,
                        },
                    );
                })
                .is_some()
        };

        for (idx, e) in golden["mass_mover_allocation"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let step = e["step"].as_str().unwrap_or_default();
            let ok = match step {
                "free_behind" | "free_for_wrap" => {
                    set.cease(1);
                    true
                }
                "free_behind_again" => {
                    set.cease(2);
                    true
                }
                // Fill the chunk, then record where the cursor stopped.
                "full" => {
                    while take(&mut set) {}
                    false
                }
                _ => take(&mut set),
            };
            expect_eq(
                "mass_mover_allocation",
                idx,
                "ok",
                i(e, "ok"),
                i64::from(ok),
            );
            expect_eq(
                "mass_mover_allocation",
                idx,
                "create_ptr",
                i(e, "create_ptr"),
                i64::from(set.create_ptr()),
            );
        }
    }

    // 0d. Splash's draw stream (C4Effect.cpp:801-836), the liquid-entry effect
    //     that `C4Object::UpdateInLiquid` and the movement InLiquid check fire
    //     on entry. Two things make it worth pinning against the real body
    //     rather than a restatement:
    //
    //     * both `Random` pairs are written with an explicit r2-before-r1
    //       temporary to force the evaluation order, so a port that draws them
    //       left to right swaps every bubble's x and y offset; and
    //     * the extraction inside the loop empties the very pixel the liquid
    //       test reads, so the first iteration takes four draws and every later
    //       one takes two. The draw COUNT is landscape-dependent, which is what
    //       makes a wrong one desynchronise everything downstream rather than
    //       merely move some spray.
    for (idx, e) in golden["splash_effect"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        // The grid the oracle scaffolds for each case.
        let mut probe = match e["case"].as_str().unwrap_or_default() {
            "roofed" => SplashProbe::water_column(4),
            "not_instable" => SplashProbe::new(18, SplashProbe::HEIGHT, 1),
            "in_sky" => SplashProbe::water_column(30),
            "shallow" => SplashProbe::new(19, 22, 0),
            _ => SplashProbe::water_column(18),
        };
        probe.rng = LcgRng::new(i(e, "seed") as u32);
        crate::engine_splash::run_splash(&mut probe, 4, 20, i(e, "amt") as i32)
            .expect("the probe is infallible");

        expect_json_eq(
            "splash_effect",
            idx,
            "bubbles",
            e["bubbles"].clone(),
            serde_json::json!(probe.bubbles),
        );
        expect_json_eq(
            "splash_effect",
            idx,
            "casts",
            e["casts"].clone(),
            serde_json::json!(probe.casts),
        );
        expect_eq(
            "splash_effect",
            idx,
            "extractions",
            i(e, "extractions"),
            probe.extractions,
        );
        expect_eq(
            "splash_effect",
            idx,
            "random_count",
            i(e, "random_count"),
            i64::from(probe.rng.count),
        );
        expect_eq(
            "splash_effect",
            idx,
            "random_hold",
            i(e, "random_hold"),
            i64::from(probe.rng.hold),
        );
    }

    // 0e. C4Object::UpdateInLiquid (C4Object.cpp:6093-6110) and the probe it
    //     reads through (:5632-5635), driven through the same helpers both live
    //     call sites use (`engine/movement.rs`, `compat/object_state.rs`).
    //     Entry is edge-triggered and carries the splash; leaving is a bare flag
    //     clear. The probe sits at `y + Float * Con / FullCon - 1`, so a
    //     half-built object starts swimming at a different pixel — while the
    //     splash still originates at the object's own `y + 1`, which is why
    //     `float_reaches_water` enters the liquid and splashes nothing.
    for (idx, e) in golden["in_liquid_transition"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        // (water_top, y, was_in_liquid, con, float, mass, hit_speed, wdt, hgt)
        let full = crate::FULL_CON;
        let case = e["case"].as_str().unwrap_or_default();
        let (water_top, y, was, con, float_line, mass, hit, wdt, hgt) = match case {
            "enter_splash" => (18, 20, false, full, 0, 10, true, 8, 10),
            "enter_no_hitspeed" => (18, 20, false, full, 0, 10, false, 8, 10),
            "enter_mass_boundary" => (18, 20, false, full, 0, 3, true, 8, 10),
            "enter_mass_above" => (18, 20, false, full, 0, 4, true, 8, 10),
            "stays_wet" => (18, 20, true, full, 0, 10, true, 8, 10),
            "stays_dry" => (30, 20, false, full, 0, 10, true, 8, 10),
            "leaves" => (30, 20, true, full, 0, 10, true, 8, 10),
            "float_reaches_water" => (18, 14, false, full, 6, 10, true, 8, 10),
            "half_con_falls_short" => (18, 14, false, full / 2, 6, 10, true, 8, 10),
            "large_object_clamps" => (18, 20, false, full, 0, 10, true, 40, 40),
            "small_object_amount" => (18, 20, false, full, 0, 10, true, 5, 6),
            other => panic!("unhandled in_liquid_transition case `{other}`"),
        };

        let mut probe = SplashProbe::water_column(water_top);
        probe.rng = LcgRng::new(i(e, "seed") as u32);

        let probe_y = crate::engine_splash::liquid_probe_y(y, float_line, con);
        let wet = crate::engine_splash::SplashHost::splash_is_liquid(&probe, 4, probe_y);

        let mut in_liquid = was;
        if crate::engine_splash::entered_liquid(wet, was) {
            let ocf = if hit { crate::ocf::HIT_SPEED2 } else { 0 };
            if crate::engine_splash::should_splash(wet, was, ocf, mass) {
                let amount = crate::engine_splash::splash_amount(wdt, hgt);
                crate::engine_splash::run_splash(&mut probe, 4, y + 1, amount)
                    .expect("the probe is infallible");
            }
            in_liquid = true;
        } else if !wet && was {
            in_liquid = false;
        }

        expect_eq(
            "in_liquid_transition",
            idx,
            "probe_y",
            i(e, "probe_y"),
            i64::from(probe_y),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "wet",
            i(e, "wet"),
            i64::from(wet),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "in_liquid",
            i(e, "in_liquid"),
            i64::from(in_liquid),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "bubbles",
            i(e, "bubbles"),
            probe.bubbles.len() as i64,
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "casts",
            i(e, "casts"),
            probe.casts.len() as i64,
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "random_count",
            i(e, "random_count"),
            i64::from(probe.rng.count),
        );
        expect_eq(
            "in_liquid_transition",
            idx,
            "random_hold",
            i(e, "random_hold"),
            i64::from(probe.rng.hold),
        );
    }

    // 0f. C4Weather::Execute's disaster block (C4Weather.cpp:104-148). Four
    //     gates in a fixed order, and each gate spends its `Random(100)` level
    //     test EVEN AT LEVEL ZERO — so `all_levels_zero` draws 1629 times over
    //     400 ticks and fires nothing, while the same seed at full levels draws
    //     1696 and fires 37 disasters. A port that skipped the test when the
    //     level was zero, or reordered the gates, would desynchronise from the
    //     first tick a gate opens.
    //
    //     The launch helpers create an object and call Activate; the oracle
    //     records their arguments instead. The fixture registers those four
    //     definitions with no script functions, so both sides spend exactly the
    //     draws `Execute` itself makes.
    for (case_index, case) in golden["weather_execute"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let width = i(case, "width") as i32;
        let height = i(case, "height") as i32;

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        // C4Weather's Launch helpers create the effect object and call
        // Activate on it (C4Weather.cpp:153-165, 196-203, 178-184); the port
        // only records the weather event when that call succeeds, so the
        // fixture's definitions answer it. The body draws nothing, which is
        // what keeps the ledger comparable.
        for id in ["METO", "FXL1", "FXQ1", "FXV1"] {
            engine
                .register_definition(
                    Definition::from_script(
                        id,
                        id,
                        "#strict\npublic func Activate() { return 1; }\n",
                    )
                    .expect("weather effect compiles"),
                )
                .expect("weather effect registers");
        }
        let mut landscape = Landscape::flat(width as u32, height);
        landscape.set_world_height(height);
        // C4Landscape's TopOpen decides where the meteor spawns and whether it
        // gets a downward ydir (C4Weather.cpp:117-119).
        landscape.set_border_open(0, 0, i(case, "top_open") != 0, false);
        engine.landscape = Some(landscape);

        let mut environment = crate::EnvironmentSettings::new(0);
        environment.meteorite = i(case, "meteorite") as i32;
        environment.lightning = i(case, "lightning") as i32;
        environment.earthquake = i(case, "earthquake") as i32;
        environment.volcano = i(case, "volcano") as i32;
        engine.set_environment(environment);
        engine.weather_events.clear();

        let rows = case["ticks"].as_array().unwrap();
        let mut row_index = 0;
        for tick in 0..400_i64 {
            let before = engine.weather_events.len();
            engine
                .tick_weather_events((tick as u64 + 1) * 10)
                .expect("weather tick runs");
            let fired = engine.weather_events[before..]
                .iter()
                .map(|event| match event {
                    // The oracle records the meteorite's spawn arguments; the
                    // port carries only its x on the event and the rest on the
                    // object it spawned, so only x is compared here.
                    crate::WeatherEvent::Meteorite { x } => ("meteorite", *x),
                    crate::WeatherEvent::Lightning { position } => ("lightning", *position),
                    crate::WeatherEvent::Earthquake { x, .. } => ("earthquake", *x),
                    crate::WeatherEvent::Volcano { x, .. } => ("volcano", *x),
                })
                .collect::<Vec<_>>();

            let Some(row) = rows.get(row_index).filter(|row| i(row, "tick") == tick) else {
                assert!(
                    fired.is_empty(),
                    "PARITY DIVERGENCE in `weather_execute` case {case_index}: tick {tick} fired \
                     {fired:?} where the C++ golden recorded nothing"
                );
                continue;
            };
            row_index += 1;

            let expected = row["events"]
                .as_array()
                .unwrap()
                .iter()
                // `meteorite_rdir` is the oracle's continuation row for the
                // meteor's rotation velocity, not a second event.
                .filter(|event| event["kind"].as_str() != Some("meteorite_rdir"))
                .map(|event| {
                    (
                        event["kind"].as_str().unwrap_or_default().to_owned(),
                        i(event, "a") as i32,
                    )
                })
                .collect::<Vec<_>>();
            let actual = fired
                .iter()
                .map(|(kind, x)| ((*kind).to_owned(), *x))
                .collect::<Vec<_>>();
            assert_eq!(
                expected, actual,
                "PARITY DIVERGENCE in `weather_execute` case {case_index} tick {tick} events"
            );
            expect_eq(
                "weather_execute",
                case_index,
                "random_count",
                i(row, "random_count"),
                i64::from(engine.rng.count),
            );
            expect_eq_u64(
                "weather_execute",
                case_index,
                "random_hold",
                u(row, "random_hold"),
                u64::from(engine.rng.hold),
            );
        }
        assert_eq!(
            row_index,
            rows.len(),
            "PARITY DIVERGENCE in `weather_execute` case {case_index}: \
             the port never reached every recorded tick"
        );
    }

    // 0g. C4Shape::ContactCheck (C4Shape.cpp:370-406), the per-pixel probe every
    //     step of C4Object::DoMovement runs — the explicit Phase 2 per-pixel
    //     collision gap in parity/README.md, for this bounded matrix. It decides
    //     ContactCNAT, ContactCount and the per-vertex VtxContactCNAT, so a
    //     vertex that answers differently by one pixel moves the object
    //     differently for the rest of the frame.
    //
    //     Its density reads go through GetPix's border rules
    //     (C4Landscape.h:163-180), where a CLOSED border answers MCVehic —
    //     solid — rather than sky. That is what stops an object at the edge of
    //     the map instead of letting it walk out of the world, and the
    //     `*_border` cases pin it from both sides.
    {
        let library = contact_oracle_materials();

        for (idx, case) in golden["shape_contact_check"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let mut engine = Engine::with_seed(0);
            engine.configure_materials_from_library(&library);
            install_contact_oracle_landscape(
                &mut engine,
                i(case, "left_open") as i32,
                i(case, "right_open") as i32,
                i(case, "top_open") != 0,
                i(case, "bottom_open") != 0,
            );
            let landscape = engine
                .landscape()
                .expect("contact oracle landscape remains");

            let rows = case["vertices"].as_array().expect("case vertices");
            let vertices = rows
                .iter()
                .map(|row| {
                    crate::ObjectVertex::new(i(row, "x") as i32, i(row, "y") as i32)
                        .with_cnat(i(row, "cnat") as u32)
                })
                .collect::<Vec<_>>();
            let position = crate::Vector2::new(i(case, "at_x") as i32, i(case, "at_y") as i32);
            let contact = crate::shape_contact_check(
                &vertices,
                position,
                landscape,
                &engine.materials,
                &[],
                None,
                i(case, "contact_density") as i32,
            );

            expect_eq(
                "shape_contact_check",
                idx,
                "any",
                i(case, "any"),
                i64::from(u8::from(contact.is_contact())),
            );
            expect_eq(
                "shape_contact_check",
                idx,
                "contact_cnat",
                i(case, "contact_cnat"),
                i64::from(contact.contact_cnat),
            );
            expect_eq(
                "shape_contact_check",
                idx,
                "contact_count",
                i(case, "contact_count"),
                i64::from(contact.count()),
            );
            for (vertex_index, row) in rows.iter().enumerate() {
                expect_eq(
                    "shape_contact_check",
                    idx,
                    "vertex contact_cnat",
                    i(row, "contact_cnat"),
                    i64::from(contact.vertex_contacts[vertex_index]),
                );
                // C4Shape stores VtxContactMat, which the port does not carry on
                // ShapeContact — so the material is compared through the
                // landscape probe both sides read, GBackMat
                // (C4Wrappers.h:179-182). A CNAT_NoCollision vertex is skipped
                // before that assignment, so its golden value is the fixture's
                // own initialiser rather than an engine answer.
                if i(row, "cnat") & 64 != 0 {
                    continue;
                }
                let expected = match i(row, "mat") {
                    -1 => None,
                    1 => Some("Earth"),
                    2 => Some("Water"),
                    3 => Some("Vehicle"),
                    other => panic!("unmapped oracle material index {other}"),
                };
                let actual = landscape
                    .border_material_at(
                        position.x + i(row, "x") as i32,
                        position.y + i(row, "y") as i32,
                    )
                    .and_then(|id| engine.materials.get_by_id(id))
                    .map(|material| material.name().to_owned());
                assert_eq!(
                    expected,
                    actual.as_deref(),
                    "PARITY DIVERGENCE in `shape_contact_check` entry {idx} vertex \
                     {vertex_index} material"
                );
            }
        }
    }

    // 0h. C4Object::TargetBounds (C4Movement.cpp:128-164), the clamp
    //     SideBounds and VerticalBounds run every movement target through. Both
    //     comparisons are strict, so sitting exactly on a limit is not a
    //     crossing; and when the limits cross each other, clamping to the low
    //     one puts the target above the high one, so BOTH bounds fire with the
    //     low contact first.
    //
    //     The port splits the C++ body: `target_bounds` returns which bounds
    //     fired, and its callers clear `fixed_velocity.x` for the side pair and
    //     `.y` for the vertical one. The golden records the C++ zeroing for the
    //     record; what is compared here is the clamp and the contact order the
    //     shared function decides.
    for (idx, case) in golden["target_bounds"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut target = i(case, "target") as i32;
        let contacts = crate::target_bounds(
            &mut target,
            i(case, "low") as i32,
            i(case, "high") as i32,
            i(case, "cnat_low") as u32,
            i(case, "cnat_hi") as u32,
        );

        expect_eq(
            "target_bounds",
            idx,
            "bounded",
            i(case, "bounded"),
            i64::from(target),
        );
        let expected = case["contacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_i64().unwrap())
            .collect::<Vec<_>>();
        let actual = contacts
            .into_iter()
            .flatten()
            .map(i64::from)
            .collect::<Vec<_>>();
        assert_eq!(
            expected, actual,
            "PARITY DIVERGENCE in `target_bounds` entry {idx} contacts"
        );
    }

    // 0i. C4Shape::Attach (C4Shape.cpp:165-271), the search attached movement
    //     runs instead of the ordinary collision loop. The two branches differ
    //     in a way that shows up in play: the old-style search loops vertices
    //     OUTSIDE and the range inside, so a second matching vertex starts from
    //     the position the first already moved to — `two_vertices_old_style`
    //     ends up BELOW the surface at y=11 — while CNAT_MultiAttach loops the
    //     range outside and takes the nearest attachment across all vertices,
    //     landing on the surface at y=9. That is the "stucking" the C++ comment
    //     at C4Shape.cpp:179-194 describes, and it is why both branches exist.
    //
    //     `closed_border_no_attach` pins the other asymmetry worth knowing: a
    //     closed border answers solid to a density probe, but Attach also
    //     requires `ax >= 0`, so an object can CONTACT the edge of the map
    //     without attaching to it.
    {
        let library = contact_oracle_materials();

        for (idx, case) in golden["shape_attach"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let mut engine = Engine::with_seed(0);
            engine.configure_materials_from_library(&library);
            install_contact_oracle_landscape(
                &mut engine,
                i(case, "left_open") as i32,
                i(case, "right_open") as i32,
                i(case, "top_open") != 0,
                i(case, "bottom_open") != 0,
            );
            let landscape = engine.landscape().expect("attach oracle landscape remains");

            let vertices = case["vertices"]
                .as_array()
                .expect("case vertices")
                .iter()
                .map(|row| {
                    crate::ObjectVertex::new(i(row, "x") as i32, i(row, "y") as i32)
                        .with_cnat(i(row, "cnat") as u32)
                })
                .collect::<Vec<_>>();
            let mut position = crate::Vector2::new(i(case, "at_x") as i32, i(case, "at_y") as i32);
            let mut record = crate::ShapeAttachRecord::default();
            let attached = crate::shape_attach(
                &vertices,
                &mut position,
                i(case, "attach") as u32,
                landscape,
                &engine.materials,
                &[],
                None,
                50,
                &mut record,
            );

            expect_eq(
                "shape_attach",
                idx,
                "attached",
                i(case, "attached"),
                i64::from(u8::from(attached)),
            );
            expect_eq(
                "shape_attach",
                idx,
                "x",
                i(case, "x"),
                i64::from(position.x),
            );
            expect_eq(
                "shape_attach",
                idx,
                "y",
                i(case, "y"),
                i64::from(position.y),
            );
            // C4Shape keeps AttachMat itself; the port keeps only whether the
            // attachment landed on a valid material and whether that material
            // is Vehicle, so the oracle's index is compared through those two.
            expect_eq(
                "shape_attach",
                idx,
                "attach_mat valid",
                i64::from(i(case, "attach_mat") >= 0),
                i64::from(u8::from(record.mat_valid)),
            );
            expect_eq(
                "shape_attach",
                idx,
                "attach_mat vehicle",
                i64::from(i(case, "attach_mat") == 3),
                i64::from(u8::from(record.mat_vehicle)),
            );
            // The position fields only overwrite on success
            // (C4Shape.cpp:217-219, 253-255).
            if attached {
                expect_eq(
                    "shape_attach",
                    idx,
                    "attach_x",
                    i(case, "attach_x"),
                    i64::from(record.x),
                );
                expect_eq(
                    "shape_attach",
                    idx,
                    "attach_y",
                    i(case, "attach_y"),
                    i64::from(record.y),
                );
                expect_eq(
                    "shape_attach",
                    idx,
                    "attach_vtx",
                    i(case, "attach_vtx"),
                    i64::from(record.vtx),
                );
            }
        }
    }

    // 0j. The container lifecycle: C4Object::Enter, Exit and Collect
    //     (C4Object.cpp:1532-1563, 1566-1637, 5693-5717), all three compiled
    //     from mechanically extracted bodies. What is pinned is the ORDER of
    //     their script calls and the re-checks between them:
    //
    //       * the recursion guard runs AFTER RejectEntrance, and
    //         RejectCollection only when the caller asked for the flag;
    //       * a Collection2 that removes the object abandons Entrance;
    //       * the re-check after Entrance tests the CONTAINER's status, not the
    //         entering object's, so an Entrance that removes the object itself
    //         still reaches the base auto-sell tail while one that removes the
    //         container does not;
    //       * Exit reports failure when a Departure callback put the object
    //         back into a container, having already done everything; and
    //       * Collect's three Hit calls are gated on their own OCF bits and
    //         stop at the first that removes the object.
    //
    //     The oracle's `calls` list also records bookkeeping the port does not
    //     expose (SetOCF, UpdateMass, CloseMenu, UpdateFace); those entries
    //     document where the mutations sit between the script calls, and what
    //     is compared here is the script calls, which both engines can name.
    {
        // Base-11 digits, one per script callback, in the order they ran. Eight
        // calls is the longest sequence in the matrix, so the encoding stays
        // inside i32.
        let digit_of = |call: &str| -> Option<i64> {
            Some(match call {
                // The oracle records the PSF_ names verbatim, `~` and all.
                "~RejectEntrance" => 1,
                "~RejectCollect" => 2,
                "~Collection2" => 3,
                "~Entrance" => 4,
                "~Collection" => 5,
                "~Ejection" => 6,
                "~Departure" => 7,
                "~Hit" => 8,
                "~Hit2" => 9,
                "~Hit3" => 10,
                _ => return None,
            })
        };

        for (idx, case) in golden["container_lifecycle"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let name = case["case"].as_str().unwrap_or_default();
            let op = case["op"].as_str().unwrap_or_default();

            // Each configured callback's port-side effect, mirroring the
            // oracle's Effect for this case.
            let reject_entrance =
                i64::from(name == "enter_rejected" || name == "collect_enter_refused");
            let reject_collection = i64::from(name == "collect_rejected_by_container");
            let entrance_body = match name {
                "enter_entrance_clears_own_status" => "RemoveObject();",
                "enter_entrance_clears_container" => "Exit();",
                _ => "",
            };
            let departure_body = match name {
                "exit_reentered_by_script" => "Enter(FindObject(OUTS));",
                _ => "",
            };
            let hit_body = match name {
                "collect_hit_kills" => "RemoveObject();",
                _ => "",
            };
            let collection2_body = match name {
                "enter_collection2_kills" => "Exit(pObj);",
                _ => "",
            };

            let object_script = format!(
                "#strict\n\
                 static callback_log;\n\
                 protected func RejectEntrance(pTarget) {{ callback_log = callback_log * 11 + 1; return {reject_entrance}; }}\n\
                 protected func Entrance(pContainer) {{ callback_log = callback_log * 11 + 4; {entrance_body} }}\n\
                 protected func Departure(pContainer) {{ callback_log = callback_log * 11 + 7; {departure_body} }}\n\
                 protected func Hit() {{ callback_log = callback_log * 11 + 8; {hit_body} }}\n\
                 protected func Hit2() {{ callback_log = callback_log * 11 + 9; }}\n\
                 protected func Hit3() {{ callback_log = callback_log * 11 + 10; }}\n\
                 public func DoEnterNull() {{ return Enter(0); }}\n\
                 public func DoEnterSelf() {{ return Enter(this()); }}\n\
                 public func DoEnter(pTarget) {{ return Enter(pTarget); }}\n\
                 public func DoExit() {{ return Exit(this(), 11, 22, 33, 1, 2, 3); }}\n"
            );
            let container_script = format!(
                "#strict\n\
                 static callback_log;\n\
                 protected func RejectCollect(idDef, pObj) {{ callback_log = callback_log * 11 + 2; return {reject_collection}; }}\n\
                 protected func Collection2(pObj) {{ callback_log = callback_log * 11 + 3; {collection2_body} }}\n\
                 protected func Collection(pObj) {{ callback_log = callback_log * 11 + 5; }}\n\
                 protected func Ejection(pObj) {{ callback_log = callback_log * 11 + 6; }}\n\
                 public func DoCollect(pItem) {{ return Collect(pItem); }}\n\
                 public func ReadLog() {{ return callback_log; }}\n\
                 public func ResetLog() {{ callback_log = 0; return 1; }}\n"
            );

            let mut engine = Engine::with_seed(0);
            // The script-level Collect needs the collector to carry
            // OCF_Collection before it will reach C4Object::Collect at all
            // (C4Script.cpp:391-413), which a DefCore collection rect is what
            // grants.
            let mut container_definition =
                Definition::from_script("CTCN", "CTCN", container_script.as_str())
                    .expect("container lifecycle fixture compiles");
            container_definition
                .set_collection_rect(Some(crate::DefinitionRect::new(-12, -10, 24, 12)));
            engine
                .register_definition(container_definition)
                .expect("container lifecycle fixture registers");
            for (id, script) in [
                ("CTOB", object_script.as_str()),
                // The old container an already-contained object exits from.
                // It needs the same recorder: the oracle logs every call, so a
                // silent OUTS would drop Ejection from the sequence.
                (
                    "OUTS",
                    "#strict\n\
                     static callback_log;\n\
                     protected func Ejection(pObj) { callback_log = callback_log * 11 + 6; }\n\
                     protected func Collection2(pObj) { callback_log = callback_log * 11 + 3; }\n",
                ),
            ] {
                engine
                    .register_definition(
                        Definition::from_script(id, id, script)
                            .expect("container lifecycle fixture compiles"),
                    )
                    .expect("container lifecycle fixture registers");
            }

            let object = engine
                .spawn_object(SpawnConfig::new("CTOB").with_controller(5))
                .expect("lifecycle object spawns");
            let container = engine
                .spawn_object(SpawnConfig::new("CTCN").with_controller(9))
                .expect("lifecycle container spawns");
            let outside = engine
                .spawn_object(SpawnConfig::new("OUTS").with_controller(2))
                .expect("lifecycle outside container spawns");

            // `exit_not_contained` is the one case that must start free.
            if name == "enter_from_container" || (op == "exit" && name != "exit_not_contained") {
                let index = engine.find_object_index(object).expect("object exists");
                engine.objects[index].state.container = Some(outside);
            }
            if name == "enter_recursive" {
                let index = engine
                    .find_object_index(container)
                    .expect("container exists");
                engine.objects[index].state.container = Some(object);
            }
            // The oracle sets the hit-speed OCF bits directly; the port derives
            // them from raw speed (|xdir| + |ydir| >= 1.5 / 2 / 6, see
            // `movement_hit_speed_flags`), and Collect defers its CopyMotion
            // until after the Hit calls precisely so they are still live there.
            if name.starts_with("collect_hit") {
                let index = engine.find_object_index(object).expect("object exists");
                let speed = if name == "collect_hit_speeds" { 7 } else { 3 };
                engine.objects[index].fixed_velocity = FixedVec2::new(itofix(speed), C4Fixed::ZERO);
                engine.objects[index].state.ocf |=
                    crate::movement_hit_speed_flags(engine.objects[index].fixed_velocity);
            }

            let object_index = engine.find_object_index(object).expect("object exists");
            let container_index = engine
                .find_object_index(container)
                .expect("container exists");
            engine
                .call_object_function(container_index, "ResetLog", Vec::new())
                .expect("the log resets");

            let target_value = crate::compat::object_reference_value(container);
            let (runner_index, function, arguments) = match op {
                "enter" => match name {
                    "enter_null_target" => (object_index, "DoEnterNull", Vec::new()),
                    "enter_self" => (object_index, "DoEnterSelf", Vec::new()),
                    _ => (object_index, "DoEnter", vec![target_value]),
                },
                "exit" => (object_index, "DoExit", Vec::new()),
                _ => (
                    container_index,
                    "DoCollect",
                    vec![crate::compat::object_reference_value(object)],
                ),
            };
            let result = engine
                .call_object_function(runner_index, function, arguments)
                .expect("the lifecycle operation runs");

            expect_eq(
                "container_lifecycle",
                idx,
                "result",
                i(case, "result"),
                i64::from(
                    matches!(result, ScriptValue::Bool(true))
                        || matches!(result, ScriptValue::Int(value) if value != 0),
                ),
            );

            // The script-call order, encoded the same way on both sides.
            let expected_log = case["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|call| digit_of(call.as_str().unwrap_or_default()))
                .fold(0_i64, |log, digit| log * 11 + digit);
            // `callback_log = 0` folds its literal zero to nil below strict 3
            // (see the `zero_literal` section), so an untouched log reads as
            // Nil rather than Int(0).
            let log = match engine
                .call_object_function(container_index, "ReadLog", Vec::new())
                .expect("the log reads back")
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected callback log value {other:?}"),
            };
            expect_eq(
                "container_lifecycle",
                idx,
                "callback order",
                expected_log,
                log,
            );
        }
    }

    // 0k. C4Effect::Check (C4Effect.cpp:271-316), the negotiation every
    //     AddEffect runs before a new effect exists. Three effects sit in the
    //     list at priorities 100, 60 and 20 and each case configures what their
    //     checker callbacks answer:
    //
    //       * priority 1 is always allowed and asks nobody;
    //       * only effects of AT LEAST the incoming priority are asked, so a
    //         low-priority denier cannot stop anything, and dead or
    //         callback-less effects are skipped;
    //       * a Deny short-circuits the walk, while an Annul only NOMINATES its
    //         effect — the walk continues and the LAST annulling effect is the
    //         one that absorbs, so `last_annul_wins` comes back with the third
    //         effect's number;
    //       * the AnnulCalls form brackets the FxAdd in temp-remove/temp-readd
    //         of the effects above the absorber, and both halves of that
    //         bracket test `pNext`, so an absorber at the end of the list gets
    //         no bracket at all; and
    //       * an FxAdd that answers Start_Deny kills the absorber and reports
    //         Annul rather than a number.
    //
    //     The port shows the bracket as temp Stop/Start callbacks on the upper
    //     effects rather than as one call, so the trace is normalised to the
    //     oracle's markers: the fixture logs a temp Stop/Start only from the
    //     middle effect, which is the one above the absorber in every bracketed
    //     case here.
    {
        let digit_of = |call: &str| -> Option<i64> {
            Some(match call {
                "EffectA" => 1,
                "EffectB" => 2,
                "EffectC" => 3,
                "Add" => 4,
                "TempRemoveUpper" => 5,
                "TempReaddUpper" => 6,
                "Kill" => 7,
                _ => return None,
            })
        };

        for (idx, case) in golden["effect_check"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let name = case["case"].as_str().unwrap_or_default();
            let priority = i(case, "priority") as i32;

            // What each existing effect's checker answers, and what the
            // absorbing effect's Add returns, recovered from the case name the
            // oracle emitted.
            let (results, add_result, dead, functionless) = match name {
                "priority_one_asks_nobody" => ([-1, -1, -1], 0, [false; 3], false),
                "all_accept" => ([0, 0, 0], 0, [false; 3], false),
                "first_denies" => ([-1, 0, 0], 0, [false; 3], false),
                "second_denies" => ([0, -1, 0], 0, [false; 3], false),
                "low_priority_denier_ignored" => ([-1, -1, -1], 0, [false; 3], false),
                "dead_effect_skipped" => ([-1, 0, 0], 0, [true, false, false], false),
                "functionless_effect_skipped" => ([-1, 0, 0], 0, [false; 3], true),
                "annul_absorbs" => ([-2, 0, 0], 0, [false; 3], false),
                "last_annul_wins" => ([-2, 0, -2], 0, [false; 3], false),
                "deny_after_annul" => ([-2, -1, 0], 0, [false; 3], false),
                "annul_calls_brackets_add" => ([-3, 0, 0], 0, [false; 3], false),
                "annul_calls_on_last_effect" => ([0, 0, -3], 0, [false; 3], false),
                "add_denies_kills_absorber" => ([-2, 0, 0], -1, [false; 3], false),
                "annul_calls_add_denies" => ([-3, 0, 0], -1, [false; 3], false),
                other => panic!("unhandled effect_check case `{other}`"),
            };

            // Only the middle effect reports its temp bracket, matching the
            // oracle's single TempRemoveUpper/TempReaddUpper markers.
            let mut script = String::from("#strict 2\nstatic fx_log, fx_armed;\n");
            for (index, id) in ["A", "B", "C"].into_iter().enumerate() {
                let digit = index + 1;
                let checker = if functionless && index == 0 {
                    String::new()
                } else {
                    format!(
                        "func FxEffect{id}Effect(string name, object target, int number) {{ if (!fx_armed) return 0; fx_log = fx_log * 11 + {digit}; return {}; }}\n",
                        results[index]
                    )
                };
                script.push_str(&checker);
                script.push_str(&format!(
                    "func FxEffect{id}Add(object target, int number, string name, int interval) {{ fx_log = fx_log * 11 + 4; return {add_result}; }}\n"
                ));
                if index == 1 {
                    script.push_str(&format!(
                        "func FxEffect{id}Stop(object target, int number, int reason, bool temp) {{ if (temp) fx_log = fx_log * 11 + 5; return 0; }}\n"
                    ));
                    script.push_str(&format!(
                        "func FxEffect{id}Start(object target, int number, int temp) {{ if (temp) fx_log = fx_log * 11 + 6; return 0; }}\n"
                    ));
                } else {
                    // The absorber's own non-temp Stop is how a Kill shows.
                    script.push_str(&format!(
                        "func FxEffect{id}Stop(object target, int number, int reason, bool temp) {{ if (!temp) fx_log = fx_log * 11 + 7; return 0; }}\n"
                    ));
                }
            }
            script.push_str(
                "func Arm() { AddEffect(\"EffectA\", this(), 100, 0, this()); AddEffect(\"EffectB\", this(), 60, 0, this()); AddEffect(\"EffectC\", this(), 20, 0, this()); fx_log = 0; fx_armed = 1; return 1; }\n",
            );
            script.push_str(&format!(
                "func Run() {{ return CheckEffect(\"Newcomer\", this(), {priority}, 35); }}\n"
            ));
            script.push_str("func ReadLog() { return fx_log; }\n");

            let mut engine = Engine::with_seed(0);
            engine
                .register_definition(
                    Definition::from_script("EFCK", "Effect check", &script)
                        .expect("effect check fixture compiles"),
                )
                .expect("effect check fixture registers");
            let object = engine
                .spawn_object(SpawnConfig::new("EFCK"))
                .expect("effect check object spawns");
            let index = engine.find_object_index(object).expect("object exists");
            engine
                .call_object_function(index, "Arm", Vec::new())
                .expect("the three effects are added");
            // A dead effect is one whose priority is zero (C4Effects.h:110),
            // in both engines.
            if dead[0] {
                if let Some(effect) = engine.objects[index]
                    .state
                    .effects
                    .iter_mut()
                    .find(|effect| effect.name == "EffectA")
                {
                    effect.priority = 0;
                }
            }

            let result = engine
                .call_object_function(index, "Run", Vec::new())
                .expect("CheckEffect runs");
            let result = match result {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected CheckEffect result {other:?}"),
            };
            expect_eq("effect_check", idx, "result", i(case, "result"), result);

            let expected_log = case["trace"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|call| digit_of(call.as_str().unwrap_or_default()))
                .fold(0_i64, |log, digit| log * 11 + digit);
            let log = match engine
                .call_object_function(index, "ReadLog", Vec::new())
                .expect("the log reads back")
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected effect log value {other:?}"),
            };
            expect_eq("effect_check", idx, "callback order", expected_log, log);
        }
    }

    // 0l. C4Effect::Execute (C4Effect.cpp:319-363), the per-frame effect pass.
    //     It walks the list unlinking dead effects as it goes, advances each
    //     survivor's clock FIRST, and only then tests `iTime % iIntervall` — so
    //     an effect created this frame with interval 1 fires immediately, and
    //     one with a non-zero starting time lands on different frames. An
    //     interval with no timer function at all is killed the moment the
    //     boundary arrives (:355-357), and a timer answering
    //     `C4Fx_Execute_Kill` finishes its effect, which the NEXT pass unlinks.
    for (idx, case) in golden["effect_execute"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let name = case["case"].as_str().unwrap_or_default();
        // (priority, interval, has_timer, timer_result, start_time) per effect,
        // recovered from the case the oracle emitted.
        let rows: [(i32, i32, bool, i32, i32); 3] = match name {
            "interval_zero_never_fires" => [
                (100, 0, true, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "interval_two_fires_every_other" => [
                (100, 2, true, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "interval_one_fires_every_frame" => [
                (100, 1, true, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "start_time_shifts_boundary" => [
                (100, 3, true, 0, 1),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "timer_kills_then_unlinks" => [
                (100, 1, true, -1, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "interval_without_timer_dies" => [
                (100, 2, false, 0, 0),
                (60, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "dead_head_unlinked" => [
                (100, 0, true, 0, 0),
                (60, 0, true, 0, 0),
                (0, 0, true, 0, 0),
            ],
            "dead_middle_unlinked" => [
                (100, 0, true, 0, 0),
                (0, 0, true, 0, 0),
                (20, 0, true, 0, 0),
            ],
            "dead_tail_unlinked" => [(0, 0, true, 0, 0), (60, 0, true, 0, 0), (20, 0, true, 0, 0)],
            "all_dead_unlinked" => [(0, 0, true, 0, 0), (0, 0, true, 0, 0), (0, 0, true, 0, 0)],
            other => panic!("unhandled effect_execute case `{other}`"),
        };

        let mut script = String::from("#strict 2\nstatic fx_log;\n");
        for (index, id) in ["A", "B", "C"].into_iter().enumerate() {
            let (_, _, has_timer, timer_result, _) = rows[index];
            let digit = index + 1;
            if has_timer {
                script.push_str(&format!(
                    "func FxEffect{id}Timer(object target, int number, int time) {{ fx_log = fx_log * 11 + {digit}; return {timer_result}; }}\n"
                ));
            }
            script.push_str(&format!(
                "func FxEffect{id}Start(object target, int number, int temp) {{ return 0; }}\n"
            ));
        }
        script.push_str("func Arm() {\n");
        for (index, id) in ["A", "B", "C"].into_iter().enumerate() {
            let (priority, interval, ..) = rows[index];
            // A zero priority would be refused outright, so every effect is
            // added alive and the dead ones are zeroed afterwards.
            let add_priority = if priority == 0 {
                10 * (index as i32 + 1)
            } else {
                priority
            };
            script.push_str(&format!(
                "  AddEffect(\"Effect{id}\", this(), {add_priority}, {interval}, this());\n"
            ));
        }
        script.push_str("  fx_log = 0; return 1;\n}\n");
        script.push_str("func ReadLog() { return fx_log; }\n");
        script.push_str("func ResetLog() { fx_log = 0; return 1; }\n");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("EFEX", "Effect execute", &script)
                    .expect("effect execute fixture compiles"),
            )
            .expect("effect execute fixture registers");
        let object = engine
            .spawn_object(SpawnConfig::new("EFEX"))
            .expect("effect execute object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        engine
            .call_object_function(index, "Arm", Vec::new())
            .expect("the three effects are added");
        for (row, id) in rows.iter().zip(["A", "B", "C"]) {
            let (priority, _, _, _, start_time) = *row;
            let effect_name = format!("Effect{id}");
            if let Some(effect) = engine.objects[index]
                .state
                .effects
                .iter_mut()
                .find(|effect| effect.name == effect_name)
            {
                if priority == 0 {
                    effect.priority = 0;
                }
                effect.timer = start_time;
            }
        }

        for pass in case["passes"].as_array().unwrap() {
            engine
                .call_object_function(index, "ResetLog", Vec::new())
                .expect("the log resets");
            engine.tick().expect("the effect frame runs");

            let expected_log = pass["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|call| match call.as_str().unwrap_or_default() {
                    "EffectA" => Some(1_i64),
                    "EffectB" => Some(2),
                    "EffectC" => Some(3),
                    // The oracle records the Kill the pass performed; the port
                    // shows it as the effect being gone on the next pass, which
                    // the live list below compares.
                    _ => None,
                })
                .fold(0_i64, |log, digit| log * 11 + digit);
            let log = match engine
                .call_object_function(index, "ReadLog", Vec::new())
                .expect("the log reads back")
            {
                ScriptValue::Int(value) => i64::from(value),
                ScriptValue::Nil | ScriptValue::Bool(false) => 0,
                other => panic!("unexpected effect log value {other:?}"),
            };
            let frame = i(pass, "frame");
            expect_eq("effect_execute", idx, "timer calls", expected_log, log);

            let expected_live = pass["live"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>();
            let live = engine.objects[index]
                .state
                .effects
                .iter()
                .map(|effect| effect.name.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                expected_live, live,
                "PARITY DIVERGENCE in `effect_execute` entry {idx} frame {frame} live effects"
            );
        }
    }

    // 0m. C4Object::AssignRemoval (C4Object.cpp:240-320), the object teardown.
    //     The order is what this pins:
    //
    //       * the CONTAINER's ContentsDestruction runs before the object's own
    //         Destruction, and each is followed by a `Status` re-check because
    //         the callback may already have removed the object — a callback
    //         that does so stops everything after it;
    //       * the object's contents are torn down BEFORE it leaves its own
    //         container, so a dying object's cargo still sees it as their
    //         container; and
    //       * `fExitContents` decides whether that cargo is Exited (spilled) or
    //         removed recursively, each one running its own full teardown.
    //
    //     The oracle also records bookkeeping the port does not expose
    //     (SetOCF, UpdateMass, SetActionIdle, ClearPointers, particles, the
    //     info retire); those entries document where the mutations sit between
    //     the script calls, and the comparison here is over the script calls
    //     plus the end state both engines can name.
    for (idx, case) in golden["object_removal"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let name = case["case"].as_str().unwrap_or_default();
        let contents = i(case, "own_contents");
        let _ = contents;

        // Which fixture shape this case needs.
        let contained = matches!(
            name,
            "contained" | "contents_destruction_deletes" | "contained_with_contents"
        );
        let cargo = match name {
            "already_deleted"
            | "contents_removed_recursively"
            | "contents_exited"
            | "contained_with_contents" => 2,
            "contents_destruction_deletes" | "destruction_deletes" => 1,
            _ => 0,
        };
        let exit_contents = name == "contents_exited";
        let destruction_body = if name == "destruction_deletes" {
            "if (!rm_fired) { rm_fired = 1; RemoveObject(); }"
        } else {
            ""
        };
        let contents_destruction_body = if name == "contents_destruction_deletes" {
            "if (!rm_fired) { rm_fired = 1; RemoveObject(pObj); }"
        } else {
            ""
        };

        let object_script = format!(
            "#strict 2\n\
             static rm_log, rm_fired;\n\
             protected func Destruction() {{ rm_log = rm_log * 11 + 2; {destruction_body} }}\n\
             protected func ContentsDestruction(pObj) {{ rm_log = rm_log * 11 + 1; }}\n\
             public func ReadLog() {{ return rm_log; }}\n"
        );
        let container_script = format!(
            "#strict 2\n\
             static rm_log, rm_fired;\n\
             protected func ContentsDestruction(pObj) {{ rm_log = rm_log * 11 + 1; {contents_destruction_body} }}\n\
             protected func Destruction() {{ rm_log = rm_log * 11 + 2; }}\n\
             public func ReadLog() {{ return rm_log; }}\n\
             public func ResetLog() {{ rm_log = 0; return 1; }}\n"
        );
        // The cargo carries the same recorders, so a recursive teardown shows.
        let cargo_script = "#strict 2\n\
             static rm_log;\n\
             protected func Destruction() { rm_log = rm_log * 11 + 2; }\n\
             protected func ContentsDestruction(pObj) { rm_log = rm_log * 11 + 1; }\n";

        let mut engine = Engine::with_seed(0);
        for (id, script) in [
            ("RMOB", object_script.as_str()),
            ("RMCN", container_script.as_str()),
            ("RMCG", cargo_script),
        ] {
            engine
                .register_definition(
                    Definition::from_script(id, id, script).expect("removal fixture compiles"),
                )
                .expect("removal fixture registers");
        }

        let container = engine
            .spawn_object(SpawnConfig::new("RMCN"))
            .expect("removal container spawns");
        let object = engine
            .spawn_object(SpawnConfig::new("RMOB"))
            .expect("removal object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        if contained {
            engine.objects[index].state.container = Some(container);
        }
        let mut cargo_ids = Vec::new();
        for _ in 0..cargo {
            let id = engine
                .spawn_object(SpawnConfig::new("RMCG").with_container(object))
                .expect("cargo spawns");
            cargo_ids.push(id);
        }
        if name == "inactive_reactivated_first" {
            engine.objects[index].state.status = ObjectStatus::Inactive;
        }
        // The oracle's `already_deleted` row is an object whose status is
        // already zero while its cargo is still attached — a state a real first
        // removal cannot leave behind, so it is set directly.
        if name == "already_deleted" {
            engine.objects[index].state.status = ObjectStatus::Deleted;
        }

        let container_index = engine
            .find_object_index(container)
            .expect("container exists");
        engine
            .call_object_function(container_index, "ResetLog", Vec::new())
            .expect("the log resets");

        // `already_deleted` is the second removal of an object already gone.
        let runner_script = format!(
            "#strict 2\npublic func Run(object pTarget) {{ RemoveObject(pTarget, {}); return 1; }}\n",
            i32::from(exit_contents)
        );
        engine
            .register_definition(
                Definition::from_script("RMRN", "RMRN", &runner_script)
                    .expect("removal runner compiles"),
            )
            .expect("removal runner registers");
        let runner = engine
            .spawn_object(SpawnConfig::new("RMRN"))
            .expect("removal runner spawns");
        let runner_index = engine.find_object_index(runner).expect("runner exists");
        let target = crate::compat::object_reference_value(object);
        let _ = &target;
        engine
            .call_object_function(runner_index, "Run", vec![target])
            .expect("the removal runs");

        let expected_log = case["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|call| match call.as_str().unwrap_or_default() {
                "~ContentsDestruction" => Some(1_i64),
                "~Destruction" => Some(2),
                _ => None,
            })
            .fold(0_i64, |log, digit| log * 11 + digit);
        let log = match engine
            .call_object_function(container_index, "ReadLog", Vec::new())
            .expect("the log reads back")
        {
            ScriptValue::Int(value) => i64::from(value),
            ScriptValue::Nil | ScriptValue::Bool(false) => 0,
            other => panic!("unexpected removal log value {other:?}"),
        };
        expect_eq("object_removal", idx, "callback order", expected_log, log);

        // The cargo's fate: removed with the object, or spilled into the world.
        let surviving_cargo = cargo_ids
            .iter()
            .filter(|id| {
                engine
                    .find_object_index(**id)
                    .is_some_and(|index| engine.objects[index].state.status.is_active())
            })
            .count();
        let expected_cargo = case["content_status"]
            .as_array()
            .unwrap()
            .iter()
            .take(cargo as usize)
            .filter(|status| status.as_i64() != Some(0))
            .count();
        assert_eq!(
            expected_cargo, surviving_cargo,
            "PARITY DIVERGENCE in `object_removal` entry {idx} surviving cargo"
        );
    }

    // 1. itofix (whole-integer + precision-denominated).
    for (idx, e) in golden["itofix"].as_array().unwrap().iter().enumerate() {
        let (x, prec, raw) = (i(e, "x") as i32, i(e, "prec") as i32, i(e, "raw"));
        let rust = if prec == 1 {
            itofix(x).val()
        } else {
            itofix_prec(x, prec).val()
        };
        expect_eq("itofix", idx, "raw", raw, rust as i64);
    }

    // 2. fixtoi (rounding back to integer, whole + precision-multiplied).
    for (idx, e) in golden["fixtoi"].as_array().unwrap().iter().enumerate() {
        let (raw, prec, result) = (i(e, "raw") as i32, i(e, "prec") as i32, i(e, "result"));
        let f = C4Fixed::from_raw(raw);
        let rust = if prec == 1 {
            fixtoi(f)
        } else {
            fixtoi_prec(f, prec)
        };
        expect_eq("fixtoi", idx, "result", result, rust as i64);
    }

    // 3. arithmetic (+, -, *, /) and the FIXED100/256/10 helper constants.
    for (idx, e) in golden["arith"].as_array().unwrap().iter().enumerate() {
        if e.get("a").is_some() {
            let (a, b) = (i(e, "a") as i32, i(e, "b") as i32);
            let (fa, fb) = (itofix(a), itofix(b));
            expect_eq("arith", idx, "add", i(e, "add"), (fa + fb).val() as i64);
            expect_eq("arith", idx, "sub", i(e, "sub"), (fa - fb).val() as i64);
            expect_eq("arith", idx, "mul", i(e, "mul"), (fa * fb).val() as i64);
            expect_eq("arith", idx, "div", i(e, "div"), (fa / fb).val() as i64);
        } else {
            expect_eq(
                "arith",
                idx,
                "fixed100_10",
                i(e, "fixed100_10"),
                fixed100(10).val() as i64,
            );
            expect_eq(
                "arith",
                idx,
                "fixed256_10",
                i(e, "fixed256_10"),
                fixed256(10).val() as i64,
            );
            expect_eq(
                "arith",
                idx,
                "fixed10_10",
                i(e, "fixed10_10"),
                fixed10(10).val() as i64,
            );
        }
    }

    // 4. trig (Sin/Cos via the shared SineTable).
    for (idx, e) in golden["trig"].as_array().unwrap().iter().enumerate() {
        let deg = i(e, "deg") as i32;
        let angle = itofix(deg);
        expect_eq(
            "trig",
            idx,
            "sin",
            i(e, "sin"),
            angle.sin_deg().val() as i64,
        );
        expect_eq(
            "trig",
            idx,
            "cos",
            i(e, "cos"),
            angle.cos_deg().val() as i64,
        );
    }

    // 4b. Script FnSin/FnCos default radius: omitted integer parameters are
    // zero-filled and only precision is corrected to one (C4Script.cpp:
    // 3224-3238).
    for (idx, e) in golden["script_trig_default_radius"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let deg = i(e, "deg") as i32;
        let ScriptValue::Int(rust_sin) =
            sin_func(&[ScriptValue::Int(deg)]).expect("script Sin oracle input succeeds")
        else {
            panic!("script Sin did not return int")
        };
        let ScriptValue::Int(rust_cos) =
            cos_func(&[ScriptValue::Int(deg)]).expect("script Cos oracle input succeeds")
        else {
            panic!("script Cos did not return int")
        };
        expect_eq(
            "script_trig_default_radius",
            idx,
            "sin",
            i(e, "sin"),
            i64::from(rust_sin),
        );
        expect_eq(
            "script_trig_default_radius",
            idx,
            "cos",
            i(e, "cos"),
            i64::from(rust_cos),
        );
    }

    // 4c. Script FnSqrt: the two correction steps around the truncated double
    // root, whose `iSqrt * iSqrt` products are C4ValueInt and wrap above
    // 46340^2 (C4Script.cpp:3240-3247, C4Value.h:62).
    for (idx, e) in golden["script_sqrt"].as_array().unwrap().iter().enumerate() {
        let value = i(e, "value") as i32;
        let ScriptValue::Int(rust_root) =
            sqrt_func(&[ScriptValue::Int(value)]).expect("script Sqrt oracle input succeeds")
        else {
            panic!("script Sqrt did not return int")
        };
        expect_eq(
            "script_sqrt",
            idx,
            "root",
            i(e, "root"),
            i64::from(rust_root),
        );
    }

    // 5. RNG: the LCG sequence and RandomCount semantics (incl. range 0).
    {
        let rr = &golden["rng_random"];
        let seed = i(rr, "seed") as u32;
        let mut rng = LcgRng::new(seed);
        for (idx, e) in rr["sequence"].as_array().unwrap().iter().enumerate() {
            let range = i(e, "range") as i32;
            let val = i(e, "val");
            expect_eq("rng_random", idx, "val", val, rng.random(range) as i64);
        }
        expect_eq(
            "rng_random",
            0,
            "count_after",
            i(rr, "count_after"),
            rng.count as i64,
        );
        rng.random(0); // range 0: returns 0 but still increments count
        expect_eq(
            "rng_random",
            0,
            "count_after_zero",
            i(rr, "count_after_zero"),
            rng.count as i64,
        );
    }

    // 5b. Stateless SeededRandom, including zero range and u32 overflow.
    for (idx, entry) in golden["rng_seeded_random"]
        .as_array()
        .expect("rng_seeded_random is an array")
        .iter()
        .enumerate()
    {
        expect_eq_u64(
            "rng_seeded_random",
            idx,
            "val",
            u(entry, "val"),
            u64::from(LcgRng::seeded_random(
                u(entry, "seed") as u32,
                u(entry, "range") as u32,
            )),
        );
    }

    // 6. Randomize3 buffer values + the Rnd3 circular-buffer sequence.
    {
        let rr = &golden["rng_randomize3"];
        let seed = i(rr, "seed") as u32;
        // Buffer values are `random(3) - 1` ×500 (what randomize3 fills).
        let mut builder = LcgRng::new(seed);
        for (idx, b) in rr["buffer"].as_array().unwrap().iter().enumerate() {
            let cpp = b.as_i64().unwrap();
            expect_eq(
                "rng_randomize3.buffer",
                idx,
                "entry",
                cpp,
                (builder.random(3) - 1) as i64,
            );
        }
        // Rnd3 sequence exercises randomize3() + rnd3() end to end.
        let mut rng = LcgRng::new(seed);
        rng.randomize3();
        for (idx, b) in rr["rnd3_sequence"].as_array().unwrap().iter().enumerate() {
            let cpp = b.as_i64().unwrap();
            expect_eq(
                "rng_randomize3.rnd3_sequence",
                idx,
                "entry",
                cpp,
                rng.rnd3() as i64,
            );
        }
    }

    // 6b. C4Object::DigOutMaterialCast: drive a real Rust DigRect through a
    // Dig2ObjectRatio material, then compare the cast and twenty subsequent
    // draws with the mechanically extracted C++ body/LC_RNG_TRACE ledger.
    {
        let case = &golden["dig2object_rng"];
        let object_x = i(case, "object_x") as i32;
        let object_y = i(case, "object_y") as i32;
        let shape_y = i(case, "shape_y") as i32;
        let shape_height = i(case, "shape_height") as i32;

        let mut digger =
            Definition::from_script("DGRR", "Digger", "").expect("digger fixture compiles");
        digger.set_shape_rect(Some(DefinitionRect::new(-2, shape_y, 4, shape_height)));
        let mut gem = Definition::from_script("GEM_", "Gem", "").expect("gem fixture compiles");
        gem.set_rotateable(1);

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
        "#;
        let library = clonk_resources::MaterialLibrary::parse(material_source)
            .expect("Dig2Object material fixture parses");
        let materials = MaterialSet::from_resource_library(&library);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(digger)
            .expect("digger fixture registers");
        engine
            .register_definition(gem)
            .expect("gem fixture registers");
        engine.set_materials(materials);

        let mut pixels = vec![0_u8; 25];
        pixels[object_y as usize * 5 + object_x as usize] = 10;
        let mut densities = vec![0_i32; 128];
        densities[10] = 80;
        let mut material_names = vec![None; 128];
        material_names[10] = Some("Earth".to_string());
        let grid = PixelGrid::new(5, 5, pixels, densities, material_names, vec![None; 128]);
        let mut landscape = Landscape::flat(5, 5);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let digger_id = engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(crate::Vector2::new(object_x, object_y))
                    .with_loaded(true),
            )
            .expect("digger fixture spawns");
        engine.rng = LcgRng::new(i(case, "seed") as u32);
        expect_eq(
            "dig2object_rng.rng_before",
            0,
            "count",
            i(&case["rng_before"], "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "dig2object_rng.rng_before",
            0,
            "hold",
            u(&case["rng_before"], "hold"),
            u64::from(engine.rng.hold),
        );

        engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
            origin: crate::Vector2::new(object_x, object_y),
            width: 1,
            height: 1,
            requested: false,
            by_object: Some(digger_id),
        }]);

        let snapshot = engine.snapshot();
        let gems: Vec<_> = snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "GEM_")
            .collect();
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "count",
            i(&case["spawn"], "count"),
            gems.len() as i64,
        );
        let gem = gems.first().expect("Dig2Object fixture spawned one gem");
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "x",
            i(&case["spawn"], "x"),
            i64::from(gem.position.x),
        );
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "y",
            i(&case["spawn"], "y"),
            i64::from(gem.position.y),
        );
        expect_eq(
            "dig2object_rng.spawn",
            0,
            "rotation",
            i(&case["spawn"], "rotation"),
            i64::from(gem.rotation),
        );
        expect_eq(
            "dig2object_rng.rng_after_cast",
            0,
            "count",
            i(&case["rng_after_cast"], "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "dig2object_rng.rng_after_cast",
            0,
            "hold",
            u(&case["rng_after_cast"], "hold"),
            u64::from(engine.rng.hold),
        );

        for (index, draw) in case["next"]
            .as_array()
            .expect("dig2object_rng.next is an array")
            .iter()
            .enumerate()
        {
            let range = i(draw, "range") as i32;
            expect_eq(
                "dig2object_rng.next",
                index,
                "value",
                i(draw, "value"),
                i64::from(engine.rng.random(range)),
            );
        }
        expect_eq(
            "dig2object_rng.rng_after",
            0,
            "count",
            i(&case["rng_after"], "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "dig2object_rng.rng_after",
            0,
            "hold",
            u(&case["rng_after"], "hold"),
            u64::from(engine.rng.hold),
        );
    }

    // 6c. C4Game::ShakeObjects master-list selection, RNG consumption, and
    // raw C4Object::Fling fallback. The oracle compiles the complete method
    // bodies mechanically extracted from C4Game.cpp and C4Object.cpp.
    {
        let case = &golden["shake_objects"];
        let objects = case["objects"]
            .as_array()
            .expect("shake_objects.objects is an array");
        let caller_row = objects
            .iter()
            .find(|row| row["name"].as_str() == Some("caller"))
            .expect("shake_objects oracle includes caller row");
        let caused_by = i(case, "caused_by") as i32;
        let script = format!(
            "#strict\npublic func Shake() {{ SetController({caused_by}); ShakeObjects({}, {}, {}); SetController(-1); }}\n",
            i(case, "x"),
            i(case, "y"),
            i(case, "range")
        );
        let mut caller = Definition::from_script("SHKO", "Shake oracle", &script)
            .expect("shake oracle caller compiles");
        caller.set_category(CATEGORY_OBJECT);
        let mut target = Definition::from_script("SHKT", "Shake target", "#strict\n")
            .expect("shake oracle target compiles");
        target.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        engine
            .register_player(PlayerConfig::new(caused_by, "Shake cause"))
            .expect("shake cause player registers");

        let spawn_row = |engine: &mut Engine,
                         row: &Value,
                         definition_id: &str,
                         container: Option<crate::ObjectId>| {
            let config = SpawnConfig::new(definition_id)
                .with_custom_name(row["name"].as_str().expect("row name"))
                .with_position(crate::Vector2::new(i(row, "x") as i32, i(row, "y") as i32))
                .with_fixed_velocity(FixedVec2::new(
                    C4Fixed::from_raw(i(row, "xdir_before") as i32),
                    C4Fixed::from_raw(i(row, "ydir_before") as i32),
                ))
                .with_category(i(row, "category") as i32)
                .with_controller(OWNER_NONE)
                .with_alive(i(row, "ocf") as u32 & crate::ocf::ALIVE != 0);
            let id = engine
                .spawn_object(config)
                .expect("shake oracle row spawns");
            let index = engine.find_object_index(id).expect("shake row exists");
            let attach_mat = i(row, "attach_mat");
            engine.objects[index].state.status =
                ObjectStatus::from_script_value(i(row, "status") as i32)
                    .expect("valid C4Object status");
            engine.objects[index].state.container = container;
            engine.objects[index].state.t_attach = i(row, "t_attach_before") as u32;
            engine.objects[index].frame_t_attach = i(row, "t_attach_before") as u32;
            engine.objects[index].state.shape_attach = ShapeAttachRecord {
                mat_valid: attach_mat >= 0,
                mat_vehicle: attach_mat == 1,
                x: i(row, "x") as i32,
                y: i(row, "y") as i32,
                vtx: 0,
            };
            engine.objects[index].state.mobile = false;
            id
        };

        let caller_id = spawn_row(&mut engine, caller_row, "SHKO", None);
        let mut ids = HashMap::from([("caller".to_string(), caller_id)]);
        for row in objects {
            let name = row["name"].as_str().expect("row name");
            if name == "caller" {
                continue;
            }
            let container = (i(row, "contained") != 0).then_some(caller_id);
            ids.insert(
                name.to_string(),
                spawn_row(&mut engine, row, "SHKT", container),
            );
        }
        let master_order = objects
            .iter()
            .map(|row| ids[row["name"].as_str().expect("row name")])
            .collect::<Vec<_>>();
        engine.exec_list = master_order.iter().rev().copied().collect();

        let rng_before = &case["rng_before"];
        expect_eq(
            "shake_objects.rng_before",
            0,
            "count",
            i(rng_before, "count"),
            engine.rng.count as i64,
        );
        expect_eq_u64(
            "shake_objects.rng_before",
            0,
            "hold",
            u(rng_before, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "shake_objects.rng_before",
            0,
            "rnd3_ptr",
            i(rng_before, "rnd3_ptr"),
            engine.rng.rnd3_ptr() as i64,
        );

        let caller_index = engine
            .find_object_index(caller_id)
            .expect("shake caller exists");
        engine
            .call_object_function(caller_index, "Shake", Vec::new())
            .expect("ShakeObjects executes");

        let rng_after = &case["rng_after"];
        expect_eq(
            "shake_objects.rng_after",
            0,
            "count",
            i(rng_after, "count"),
            engine.rng.count as i64,
        );
        expect_eq_u64(
            "shake_objects.rng_after",
            0,
            "hold",
            u(rng_after, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "shake_objects.rng_after",
            0,
            "rnd3_ptr",
            i(rng_after, "rnd3_ptr"),
            engine.rng.rnd3_ptr() as i64,
        );

        for (index, row) in objects.iter().enumerate() {
            let name = row["name"].as_str().expect("row name");
            let object_index = engine
                .find_object_index(ids[name])
                .unwrap_or_else(|| panic!("shake oracle row `{name}` remains"));
            let object = &engine.objects[object_index];
            expect_eq(
                "shake_objects.objects",
                index,
                "xdir_after",
                i(row, "xdir_after"),
                object.fixed_velocity.x.val() as i64,
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "ydir_after",
                i(row, "ydir_after"),
                object.fixed_velocity.y.val() as i64,
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "t_attach_after",
                i(row, "t_attach_after"),
                i64::from(object.state.t_attach),
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "mobile_after",
                i(row, "mobile_after"),
                i64::from(u8::from(object.state.mobile)),
            );
            expect_eq(
                "shake_objects.objects",
                index,
                "controller_after",
                i(row, "controller_after"),
                i64::from(object.state.controller),
            );
        }
    }

    // 6c-2. C4Game::BlastObjects (C4Game.cpp:1265-1319) and C4Object::Blast
    // (C4Object.cpp:1416-1426), both compiled from the mechanically extracted
    // bodies. Three things are pinned that nothing else in the tree covers:
    //
    //   * the two independent hit tests — a direct hit widens the shape by five
    //     pixels on every side, while the shock wave is a plain `<= level`
    //     square around the object's own position, so `far_out_of_range` takes
    //     neither and `boundary_in`/`boundary_out` straddle the shock wave
    //     alone;
    //   * the shock-wave gate's shape (category, NoHorizontalMove, then a Grab
    //     of exactly 1 excusing vehicles and DFA_FLOAT actors); and
    //   * that the whole call consumes NOTHING from the synchronised stream and
    //     one Rnd3 per fling. `RandomCount` and `RandomHold` are unchanged
    //     across it while the Rnd3 pointer advances once per flung object, so a
    //     port that routed the fling's sign through `Random` would desynchronise
    //     every later frame while looking correct in isolation.
    //
    // `blast_incinerate_gate` runs the same fixture for Blast's
    // `Damage >= Def->BlastIncinerate` arm. It is a separate case, and one
    // without an RNG comparison, because the oracle records the `Incinerate`
    // call where the port starts the real fire effect — and that effect draws.
    for section in ["blast_objects", "blast_incinerate_gate"] {
        let case = &golden[section];
        let rows = case["objects"].as_array().expect("blast objects array");
        let blast_x = i(case, "x") as i32;
        let blast_y = i(case, "y") as i32;
        let level = i(case, "level") as i32;
        let caused_by = i(case, "caused_by") as i32;

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        let caller_script = format!(
            "#strict\npublic func Boom() {{ SetController({caused_by}); BlastObjects({blast_x}, {blast_y}, {level}); SetController(-1); }}\n"
        );
        let mut caller = Definition::from_script("BLSO", "Blast oracle", &caller_script)
            .expect("blast oracle caller compiles");
        caller.set_category(CATEGORY_OBJECT);
        engine
            .register_definition(caller)
            .expect("blast caller registers");
        engine
            .register_player(PlayerConfig::new(caused_by, "Blast cause"))
            .expect("blast cause player registers");

        // One definition per row: Grab, NoHorizontalMove, BlastIncinerate, mass
        // and the shape rect are all C4Def state, and the DFA_FLOAT row needs
        // an action map of its own.
        let mut ids = HashMap::new();
        let mut master_order = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let name = row["name"].as_str().expect("row name");
            let definition_id = format!("BL{index:02}");
            let mut definition = Definition::from_script(&definition_id, name, "#strict\n")
                .expect("blast row definition compiles");
            definition.set_category(i(row, "category") as i32);
            definition.set_mass(i(row, "mass") as i32);
            definition.set_grab(i(row, "grab") as i32);
            definition.set_no_horizontal_move(i(row, "no_horizontal_move") as i32);
            definition.set_blast_incinerate(i(row, "blast_incinerate") as i32);
            definition.set_shape_rect(Some(crate::DefinitionRect::new(
                i(row, "shape_x") as i32,
                i(row, "shape_y") as i32,
                i(row, "wdt") as i32,
                i(row, "hgt") as i32,
            )));
            // The oracle's `procedure` column is the C4Def ActMap entry the
            // object's action points at; -1 is ActIdle.
            if i(row, "procedure") >= 0 {
                definition.configure_actions(
                    Some("Float".to_owned()),
                    HashMap::from([(
                        "Float".to_owned(),
                        crate::ActionSpec::default().with_procedure("FLOAT"),
                    )]),
                );
            }
            engine
                .register_definition(definition)
                .expect("blast row definition registers");

            let id = engine
                .spawn_object(
                    SpawnConfig::new(&definition_id)
                        .with_custom_name(name)
                        .with_category(i(row, "category") as i32)
                        .with_controller(OWNER_NONE)
                        .with_alive(i(row, "alive") != 0),
                )
                .expect("blast oracle row spawns");
            let object_index = engine.find_object_index(id).expect("blast row exists");
            // Set the position rather than spawning at it: a spawn y is the
            // object's BOTTOM (C4Game::CreateObject), and these rows carry a
            // shape offset, so passing the oracle's y through SpawnConfig would
            // place the object eight pixels off its own coordinate.
            engine.objects[object_index].state.position =
                crate::Vector2::new(i(row, "x") as i32, i(row, "y") as i32);
            engine.objects[object_index].state.status =
                ObjectStatus::from_script_value(i(row, "status") as i32)
                    .expect("valid C4Object status");
            engine.objects[object_index].state.mobile = false;
            ids.insert(name.to_owned(), id);
            master_order.push(id);
        }
        // The oracle's contained row sits inside the first row's object, which
        // is what keeps it out of the uncontained arm entirely.
        if let Some(contained) = ids.get("contained").copied() {
            let container = ids["living_center"];
            let contained_index = engine
                .find_object_index(contained)
                .expect("contained row exists");
            engine.objects[contained_index].state.container = Some(container);
        }

        // The caller is not one of the oracle's rows; it sits far outside both
        // hit tests so it can run the blast without taking part in it.
        let caller_id = engine
            .spawn_object(
                SpawnConfig::new("BLSO")
                    .with_position(crate::Vector2::new(5_000, 5_000))
                    .with_controller(OWNER_NONE),
            )
            .expect("blast caller spawns");
        master_order.push(caller_id);
        engine.exec_list = master_order.iter().rev().copied().collect();

        let compares_rng = case.get("rng_before").is_some();
        if compares_rng {
            let rng_before = &case["rng_before"];
            expect_eq(
                "blast_objects.rng_before",
                0,
                "count",
                i(rng_before, "count"),
                engine.rng.count as i64,
            );
            expect_eq_u64(
                "blast_objects.rng_before",
                0,
                "hold",
                u(rng_before, "hold"),
                u64::from(engine.rng.hold),
            );
            expect_eq(
                "blast_objects.rng_before",
                0,
                "rnd3_ptr",
                i(rng_before, "rnd3_ptr"),
                engine.rng.rnd3_ptr() as i64,
            );
        }

        let caller_index = engine
            .find_object_index(caller_id)
            .expect("blast caller exists");
        engine
            .call_object_function(caller_index, "Boom", Vec::new())
            .expect("BlastObjects executes");

        if compares_rng {
            let rng_after = &case["rng_after"];
            expect_eq(
                "blast_objects.rng_after",
                0,
                "count",
                i(rng_after, "count"),
                engine.rng.count as i64,
            );
            expect_eq_u64(
                "blast_objects.rng_after",
                0,
                "hold",
                u(rng_after, "hold"),
                u64::from(engine.rng.hold),
            );
            expect_eq(
                "blast_objects.rng_after",
                0,
                "rnd3_ptr",
                i(rng_after, "rnd3_ptr"),
                engine.rng.rnd3_ptr() as i64,
            );
        }

        for (index, row) in rows.iter().enumerate() {
            let name = row["name"].as_str().expect("row name");
            let object_index = engine
                .find_object_index(ids[name])
                .unwrap_or_else(|| panic!("blast oracle row `{name}` remains"));
            let object = &engine.objects[object_index];
            expect_eq(
                section,
                index,
                "xdir_after",
                i(row, "xdir_after"),
                object.fixed_velocity.x.val() as i64,
            );
            expect_eq(
                section,
                index,
                "ydir_after",
                i(row, "ydir_after"),
                object.fixed_velocity.y.val() as i64,
            );
            expect_eq(
                section,
                index,
                "mobile_after",
                i(row, "mobile_after"),
                i64::from(u8::from(object.state.mobile)),
            );
            expect_eq(
                section,
                index,
                "controller_after",
                i(row, "controller_after"),
                i64::from(object.state.controller),
            );
            // The oracle records DoDamage's arguments; the port runs the real
            // body, which for a plain fixture with no rules or effects lands on
            // exactly that sum.
            expect_eq(
                section,
                index,
                "damage_sum",
                i(row, "damage_sum"),
                i64::from(object.state.damage),
            );
            // Likewise Incinerate: the oracle counts the call, the port sets
            // the flag the real effect leads to.
            expect_eq(
                section,
                index,
                "incinerate_calls",
                i(row, "incinerate_calls"),
                i64::from(u8::from(object.state.on_fire)),
            );
        }
    }

    // 6c. C4Landscape::BlastFree (C4Landscape.cpp:881-888, 941-960,
    // 1022-1062): the oracle mechanically compiles the complete ClearPix,
    // BlastFreePix, and BlastFree bodies. A 7x7 authoritative Surface8 plane
    // mixes Earth/Granite and IFT pixels; Earth clears to sky/Tunnel+IFT,
    // Granite probabilistically shifts to Rock while preserving IFT. Compare
    // the pre-mutation BlastMatCount, every final byte, and exact RNG state.
    {
        let case = &golden["blast_free"];
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            BlastFree=1

            [Material Granite]
            Name=Granite
            Density=100
            BlastShiftTo=Rock-Rough

            [Material Rock]
            Name=Rock
            Density=100
            BlastFree=1

            [Material Tunnel]
            Name=Tunnel
            Density=0
            TextureOverlay=Smooth2
        "#,
        )
        .expect("BlastFree oracle materials parse");

        let width = i(case, "width") as u32;
        let height = i(case, "height") as u32;
        let initial_bytes = case["initial_bytes"]
            .as_array()
            .expect("blast_free.initial_bytes is an array")
            .iter()
            .map(|byte| byte.as_u64().expect("pixel byte") as u8)
            .collect::<Vec<_>>();
        let mut densities = [0; 128];
        densities[1] = 100;
        densities[2] = 100;
        densities[3] = 100;
        densities[5] = 100;
        let mut names = vec![None; 128];
        names[1] = Some("Earth".to_string());
        names[2] = Some("Granite".to_string());
        names[3] = Some("Rock".to_string());
        names[4] = Some("Tunnel".to_string());
        names[5] = Some("Rock".to_string());
        names[6] = Some("Tunnel".to_string());
        let mut textures = vec![None; 128];
        textures[1] = Some("Smooth".to_string());
        textures[2] = Some("Smooth".to_string());
        textures[3] = Some("Smooth".to_string());
        textures[4] = Some("Rough".to_string());
        textures[5] = Some("Rough".to_string());
        textures[6] = Some("Smooth2".to_string());
        let grid = PixelGrid::new(
            width,
            height,
            initial_bytes,
            densities.to_vec(),
            names.clone(),
            textures.clone(),
        );

        let classifier = MapPixelClassifier::from_slots_with_library(
            densities,
            names,
            textures,
            vec![None; 128],
            library.clone(),
            vec![
                "Smooth".to_string(),
                "Rough".to_string(),
                "Smooth2".to_string(),
            ],
        );
        let mut texmap = classifier.into_runtime_state();
        texmap.set_default_material_entry("Earth", 1);
        texmap.set_default_material_entry("Granite", 2);
        texmap.set_default_material_entry("Rock", 3);
        texmap.set_default_material_entry("Tunnel", 6);
        let zero_texmap = texmap.clone();

        let mut engine = Engine::with_seed(i(case, "seed") as u64);
        engine.configure_materials_from_library(&library);
        let mut landscape = Landscape::flat(width, height as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_raster_state(LandscapeRasterState::new(1, 0, texmap));
        engine.set_landscape(landscape);

        let rng_before = &case["rng_before"];
        expect_eq(
            "blast_free.rng_before",
            0,
            "count",
            i(rng_before, "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.rng_before",
            0,
            "hold",
            u(rng_before, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "blast_free.rng_before",
            0,
            "rnd3_ptr",
            i(rng_before, "rnd3_ptr"),
            i64::from(engine.rng.rnd3_ptr()),
        );

        let result = engine
            .blast_circle(
                crate::Vector2::new(i(case, "x") as i32, i(case, "y") as i32),
                i(case, "radius") as i32,
                Some(i(case, "controller") as i32),
            )
            .expect("BlastFree oracle blast applies");

        let counts = &case["pre_counts"];
        for (index, name) in ["Earth", "Granite", "Rock", "Tunnel"]
            .into_iter()
            .enumerate()
        {
            let material = engine
                .materials()
                .id_of(name)
                .unwrap_or_else(|| panic!("BlastFree oracle material `{name}` exists"));
            let rust = result
                .pixel_count_by_material
                .get(&material)
                .copied()
                .unwrap_or_default();
            expect_eq(
                "blast_free.pre_counts",
                index,
                &name.to_ascii_lowercase(),
                i(counts, &name.to_ascii_lowercase()),
                i64::from(rust),
            );
        }

        let expected_bytes = case["final_bytes"]
            .as_array()
            .expect("blast_free.final_bytes is an array");
        let landscape = engine.landscape().expect("BlastFree landscape remains");
        for (index, expected) in expected_bytes.iter().enumerate() {
            let x = index as i32 % width as i32;
            let y = index as i32 / width as i32;
            expect_eq(
                "blast_free.final_bytes",
                index,
                "byte",
                expected.as_i64().expect("golden pixel byte"),
                i64::from(
                    landscape
                        .grid_byte_at(x, y)
                        .unwrap_or_else(|| panic!("BlastFree pixel ({x},{y}) exists")),
                ),
            );
        }

        let rng_after = &case["rng_after"];
        expect_eq(
            "blast_free.rng_after",
            0,
            "count",
            i(rng_after, "count"),
            i64::from(engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.rng_after",
            0,
            "hold",
            u(rng_after, "hold"),
            u64::from(engine.rng.hold),
        );
        expect_eq(
            "blast_free.rng_after",
            0,
            "rnd3_ptr",
            i(rng_after, "rnd3_ptr"),
            i64::from(engine.rng.rnd3_ptr()),
        );

        let zero = &case["zero_radius"];
        let zero_x = i(zero, "x") as i32;
        let zero_y = i(zero, "y") as i32;
        let mut zero_bytes = vec![0; width as usize * height as usize];
        zero_bytes[zero_y as usize * width as usize + zero_x as usize] =
            i(zero, "initial_byte") as u8;
        let zero_grid = PixelGrid::new(
            width,
            height,
            zero_bytes,
            zero_texmap.densities.clone(),
            zero_texmap.material_names.clone(),
            zero_texmap.texture_names.clone(),
        );
        let mut zero_landscape = Landscape::flat(width, height as i32);
        zero_landscape.set_pixel_grid(zero_grid);
        zero_landscape.set_raster_state(LandscapeRasterState::new(1, 0, zero_texmap));
        let mut zero_engine = Engine::with_seed(i(zero, "seed") as u64);
        zero_engine.configure_materials_from_library(&library);
        zero_engine.set_landscape(zero_landscape);

        expect_eq(
            "blast_free.zero_radius.rng_before",
            0,
            "count",
            i(&zero["rng_before"], "count"),
            i64::from(zero_engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.zero_radius.rng_before",
            0,
            "hold",
            u(&zero["rng_before"], "hold"),
            u64::from(zero_engine.rng.hold),
        );
        let zero_result = zero_engine
            .blast_circle(crate::Vector2::new(zero_x, zero_y), 0, Some(7))
            .expect("zero-radius BlastFree oracle blast applies");
        let earth = zero_engine
            .materials()
            .id_of("Earth")
            .expect("zero-radius oracle Earth exists");
        expect_eq(
            "blast_free.zero_radius",
            0,
            "pre_count",
            i(zero, "pre_count"),
            i64::from(
                zero_result
                    .pixel_count_by_material
                    .get(&earth)
                    .copied()
                    .unwrap_or_default(),
            ),
        );
        expect_eq(
            "blast_free.zero_radius",
            0,
            "final_byte",
            i(zero, "final_byte"),
            i64::from(
                zero_engine
                    .landscape()
                    .and_then(|landscape| landscape.grid_byte_at(zero_x, zero_y))
                    .expect("zero-radius center pixel remains addressable"),
            ),
        );
        expect_eq(
            "blast_free.zero_radius.rng_after",
            0,
            "count",
            i(&zero["rng_after"], "count"),
            i64::from(zero_engine.rng.count),
        );
        expect_eq_u64(
            "blast_free.zero_radius.rng_after",
            0,
            "hold",
            u(&zero["rng_after"], "hold"),
            u64::from(zero_engine.rng.hold),
        );
    }

    // 6d. C4Landscape::ExecuteScan / DoScan (C4Landscape.cpp:89-230). The
    // C++ oracle mechanically compiles both complete production bodies. Its
    // 6x8 Surface8 fixture has six Water pixels in every column, scans two
    // columns per frame, and freezes at four pixels per conversion pass
    // (`TempConvStrength=3` includes the starting pixel). Compare the exact
    // material counts and wrapping ScanX cursor after every Engine::tick.
    {
        let case = &golden["landscape_scan"];
        let width = i(case, "width") as u32;
        let height = i(case, "height") as u32;
        let water_depth = i(case, "water_depth") as u32;
        let water_byte = i(case, "water_byte") as u8;
        let ice_byte = i(case, "ice_byte") as u8;
        expect_eq(
            "landscape_scan",
            0,
            "scan_speed",
            i(case, "scan_speed"),
            i64::from((width as i32 / 500).clamp(2, 15)),
        );
        let library = clonk_resources::MaterialLibrary::parse(&format!(
            r#"
            [Material Water]
            Name=Water
            Density=30
            BelowTempConvert={}
            BelowTempConvertDir={}
            BelowTempConvertTo=Ice
            TempConvStrength={}

            [Material Ice]
            Name=Ice
            Density=80
            "#,
            i(case, "below_temperature"),
            i(case, "direction"),
            i(case, "strength"),
        ))
        .expect("landscape scan oracle materials parse");

        let mut bytes = vec![0; width as usize * height as usize];
        for y in 0..water_depth {
            bytes[y as usize * width as usize..(y + 1) as usize * width as usize].fill(water_byte);
        }
        let mut densities = vec![0; 128];
        densities[water_byte as usize] = 30;
        densities[ice_byte as usize] = 80;
        let mut material_names = vec![None; 128];
        material_names[water_byte as usize] = Some("Water".to_string());
        material_names[ice_byte as usize] = Some("Ice".to_string());
        let grid = PixelGrid::new(
            width,
            height,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        );

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        engine.set_environment(
            crate::EnvironmentSettings::new(0)
                .with_temperature(i(case, "temperature") as i32)
                .with_climate(0)
                .with_temperature_range(0),
        );
        let mut landscape = Landscape::flat(width, height as i32);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let states = case["states"]
            .as_array()
            .expect("landscape_scan.states is an array");
        for (index, state) in states.iter().enumerate() {
            expect_eq(
                "landscape_scan.states",
                index,
                "frame",
                i(state, "frame"),
                index as i64,
            );
            let landscape = engine
                .landscape()
                .expect("landscape scan oracle landscape remains");
            let grid = landscape
                .pixel_grid()
                .expect("landscape scan oracle pixel grid remains");
            let water = grid
                .bytes()
                .iter()
                .filter(|&&byte| byte & 0x7f == water_byte)
                .count();
            let ice = grid
                .bytes()
                .iter()
                .filter(|&&byte| byte & 0x7f == ice_byte)
                .count();
            expect_eq(
                "landscape_scan.states",
                index,
                "scan_x",
                i(state, "scan_x"),
                i64::from(landscape.scan_x()),
            );
            expect_eq(
                "landscape_scan.states",
                index,
                "water",
                i(state, "water"),
                water as i64,
            );
            expect_eq(
                "landscape_scan.states",
                index,
                "ice",
                i(state, "ice"),
                ice as i64,
            );
            if index + 1 < states.len() {
                engine
                    .tick_without_snapshot()
                    .expect("landscape scan oracle frame executes");
            }
        }
    }

    // 6e. C4Object::ContactAction's bottom DFA_FLIGHT arm
    // (C4Object.cpp:4336-4351). The C++ oracle mechanically compiles that
    // complete switch arm and the real ObjectActionFlat helper. In particular,
    // a low-speed action with ObjectDisabled=1 takes the same FlatUp path as
    // OCF_HitSpeed4; a low-speed enabled action falls through to Walk.
    for (index, case) in golden["contact_action_bottom_flight"]
        .as_array()
        .expect("contact_action_bottom_flight is an array")
        .iter()
        .enumerate()
    {
        let mut definition = Definition::from_script("CFLI", "Contact flight oracle", "#strict\n")
            .expect("contact flight oracle compiles");
        definition.configure_actions(
            Some("Flight".to_string()),
            HashMap::from([
                (
                    "Flight".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_disabled(i(case, "disabled") != 0),
                ),
                ("FlatUp".to_string(), ActionSpec::default()),
                ("KneelDown".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("contact flight oracle registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("CFLI")
                    .with_action(ActionState::new("Flight"))
                    .with_direction(Direction::Right)
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(i(case, "xdir_before") as i32),
                        C4Fixed::from_raw(i(case, "ydir_before") as i32),
                    ))
                    .with_category(CATEGORY_OBJECT)
                    .with_loaded(true),
            )
            .expect("contact flight oracle object spawns");
        let object_index = engine
            .find_object_index(id)
            .expect("contact flight oracle object exists");
        engine.objects[object_index].state.ocf = i(case, "ocf") as u32;
        let definition_id = engine.objects[object_index].definition_id.clone();
        engine
            .exec_contact_action(object_index, crate::CNAT_BOTTOM, &definition_id)
            .expect("bottom flight ContactAction executes");

        let object = &engine.objects[object_index];
        let action_after = match object.state.action.name.as_str() {
            "Flight" => 0,
            "FlatUp" => 1,
            "KneelDown" => 2,
            "Walk" => 3,
            action => panic!("unexpected contact-flight action `{action}`"),
        };
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "action_after",
            i(case, "action_after"),
            action_after,
        );
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "direction_after",
            i(case, "direction_after"),
            i64::from(object.state.direction.to_script_value()),
        );
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "xdir_after",
            i(case, "xdir_after"),
            i64::from(object.fixed_velocity.x.val()),
        );
        expect_eq(
            "contact_action_bottom_flight",
            index,
            "ydir_after",
            i(case, "ydir_after"),
            i64::from(object.fixed_velocity.y.val()),
        );
    }

    // 6f. C4Object::ContactAction's ceiling and wall DFA_FLIGHT arms
    // (C4Object.cpp:4400-4500), including the common unresolved-flight tail.
    // The enabled controls take Hangle/Scale. At the same low speed, a
    // disabled action must take Tumble instead; the tail then slides it free
    // and zeroes the transient +/-FIXED100(150) wall velocity.
    for (index, case) in golden["contact_action_top_side_flight"]
        .as_array()
        .expect("contact_action_top_side_flight is an array")
        .iter()
        .enumerate()
    {
        let mut definition =
            Definition::from_script("CFTS", "Contact top/side oracle", "#strict\n")
                .expect("contact top/side oracle compiles");
        definition.configure_actions(
            Some("Flight".to_string()),
            HashMap::from([
                (
                    "Flight".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_disabled(i(case, "disabled") != 0),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                (
                    "Scale".to_string(),
                    ActionSpec::default().with_procedure("SCALE"),
                ),
                (
                    "Hangle".to_string(),
                    ActionSpec::default().with_procedure("HANGLE"),
                ),
            ]),
        );
        definition.set_physical(PhysicalInfo {
            can_scale: i(case, "can_scale") as i32,
            can_hangle: i(case, "can_hangle") as i32,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("contact top/side oracle registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("CFTS")
                    .with_position(crate::Vector2::new(
                        i(case, "x_before") as i32,
                        i(case, "y_before") as i32,
                    ))
                    .with_action(ActionState::new("Flight"))
                    .with_direction(Direction::Right)
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(i(case, "xdir_before") as i32),
                        C4Fixed::from_raw(i(case, "ydir_before") as i32),
                    ))
                    .with_category(CATEGORY_OBJECT)
                    .with_loaded(true),
            )
            .expect("contact top/side oracle object spawns");
        let object_index = engine
            .find_object_index(id)
            .expect("contact top/side oracle object exists");
        engine.objects[object_index].state.ocf = i(case, "ocf") as u32;
        let definition_id = engine.objects[object_index].definition_id.clone();
        let contact = i(case, "contact") as u32;
        engine
            .exec_contact_action(object_index, contact, &definition_id)
            .expect("top/side flight ContactAction executes");

        let object = &engine.objects[object_index];
        let action_after = match object.state.action.name.as_str() {
            "Flight" => 0,
            "Tumble" => 4,
            "Scale" => 5,
            "Hangle" => 6,
            action => panic!("unexpected top/side contact action `{action}`"),
        };
        let xdir_before_flight_stuck = if i(case, "disabled") != 0 {
            i64::from(contact_action_wall_tumble_x(contact).val())
        } else {
            0
        };
        for (field, actual) in [
            ("action_after", action_after),
            (
                "direction_after",
                i64::from(object.state.direction.to_script_value()),
            ),
            ("xdir_before_flight_stuck", xdir_before_flight_stuck),
            ("ydir_before_flight_stuck", 0),
            ("x_after", i64::from(object.state.position.x)),
            ("y_after", i64::from(object.state.position.y)),
            ("xdir_after", i64::from(object.fixed_velocity.x.val())),
            ("ydir_after", i64::from(object.fixed_velocity.y.val())),
        ] {
            expect_eq(
                "contact_action_top_side_flight",
                index,
                field,
                i(case, field),
                actual,
            );
        }
    }

    // 7. Material corrosion execution RNG ordering.
    for (idx, e) in golden["material_corrode_rng"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let seed = i(e, "seed") as u32;
        let custom = i(e, "custom") != 0;
        let mut rng = LcgRng::new(seed);
        let success = if custom {
            evaluate_corrosion(0, 0, Some(i(e, "rate") as i32), &mut rng)
        } else {
            evaluate_corrosion(
                i(e, "corrosive") as i32,
                i(e, "corrode") as i32,
                None,
                &mut rng,
            )
        };
        if success {
            consume_corrosion_effect_rng(&mut rng);
        }
        expect_eq(
            "material_corrode_rng",
            idx,
            "success",
            i(e, "success"),
            success as i64,
        );
        expect_eq(
            "material_corrode_rng",
            idx,
            "count",
            i(e, "count"),
            rng.count as i64,
        );
        expect_eq(
            "material_corrode_rng",
            idx,
            "hold",
            i(e, "hold"),
            rng.hold as i64,
        );
    }

    // 8. Mass-mover transfer RNG ordering: Random(10) before Rnd3().
    for (case_idx, e) in golden["mass_mover_transfer_rng"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let seed = i(e, "seed") as u32;
        let mut rng = LcgRng::new(seed);
        rng.randomize3();
        for (idx, call) in e["calls"].as_array().unwrap().iter().enumerate() {
            let random10 = rng.random(10);
            let rnd3 = rng.rnd3();
            let label = format!("mass_mover_transfer_rng[{case_idx}]");
            expect_eq(
                &label,
                idx,
                "random10",
                i(call, "random10"),
                random10 as i64,
            );
            expect_eq(&label, idx, "rnd3", i(call, "rnd3"), rnd3 as i64);
            expect_eq(
                &label,
                idx,
                "execute_immediately",
                i(call, "execute_immediately"),
                (rnd3 == 0) as i64,
            );
        }
        expect_eq(
            "mass_mover_transfer_rng",
            case_idx,
            "count",
            i(e, "count"),
            rng.count as i64,
        );
        expect_eq(
            "mass_mover_transfer_rng",
            case_idx,
            "hold",
            i(e, "hold"),
            rng.hold as i64,
        );
    }

    // 9. C4Value map-key hash: C4Value.cpp:923-1029.
    {
        let section = &golden["script_value_hash"];
        expect_eq_u64(
            "script_value_hash",
            0,
            "sizeof_size_t",
            u(section, "sizeof_size_t"),
            std::mem::size_of::<usize>() as u64,
        );

        for (idx, e) in section["hash_combine"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let seed = u(e, "seed") as usize;
            let next = u(e, "next") as usize;
            expect_eq_u64(
                "script_value_hash.hash_combine",
                idx,
                "hash",
                u(e, "hash"),
                c4_hash_combine(seed, next) as u64,
            );
        }

        let mut map = ValueMap::new();
        map.insert("a".to_string(), ScriptValue::Int(1));
        map.insert(
            "b".to_string(),
            ScriptValue::Array(vec![ScriptValue::Int(2), ScriptValue::Int(3)]),
        );
        let mut reversed = ValueMap::new();
        reversed.insert(
            "b".to_string(),
            ScriptValue::Array(vec![ScriptValue::Int(2), ScriptValue::Int(3)]),
        );
        reversed.insert("a".to_string(), ScriptValue::Int(1));

        let mixed_entries = [
            (ScriptValue::Int(42), ScriptValue::String("int".into())),
            (ScriptValue::Bool(true), ScriptValue::Int(7)),
            (ScriptValue::C4Id("CLNK".into()), ScriptValue::Bool(false)),
            (
                ScriptValue::Object(77),
                ScriptValue::String("object".into()),
            ),
            (
                ScriptValue::Array(vec![ScriptValue::Int(1), ScriptValue::Bool(true)]),
                ScriptValue::C4Id("1337".into()),
            ),
        ];
        let mixed = ValueMap::from(mixed_entries.clone());
        let mixed_reversed = mixed_entries.into_iter().rev().collect();

        let cases = [
            ("nil", ScriptValue::Nil),
            ("int_zero", ScriptValue::Int(0)),
            ("int_42", ScriptValue::Int(42)),
            ("int_minus_one", ScriptValue::Int(-1)),
            ("bool_false", ScriptValue::Bool(false)),
            ("bool_true", ScriptValue::Bool(true)),
            ("id_CLNK", ScriptValue::C4Id("CLNK".to_string())),
            ("id_1337", ScriptValue::C4Id("1337".to_string())),
            ("string_empty", ScriptValue::String(String::new().into())),
            ("string_alpha", ScriptValue::String("alpha".to_string().into())),
            (
                "string_16",
                ScriptValue::String("abcdefghijklmnop".to_string().into()),
            ),
            (
                "string_24",
                ScriptValue::String("abcdefghijklmnopqrstuvwx".to_string().into()),
            ),
            (
                "string_40",
                ScriptValue::String("abcdefghijklmnopqrstuvwxyz0123456789ABCD".to_string().into()),
            ),
            (
                "string_80",
                ScriptValue::String(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        .to_string()
                        .into(),
                ),
            ),
            (
                "array_1_true_x",
                ScriptValue::Array(vec![
                    ScriptValue::Int(1),
                    ScriptValue::Bool(true),
                    ScriptValue::String("x".to_string().into()),
                ]),
            ),
            ("map_a1_b23", ScriptValue::Proplist(map)),
            ("map_b23_a1", ScriptValue::Proplist(reversed)),
            ("map_mixed_keys", ScriptValue::Proplist(mixed)),
            (
                "map_mixed_keys_reversed",
                ScriptValue::Proplist(mixed_reversed),
            ),
        ];
        for (idx, (name, value)) in cases.iter().enumerate() {
            let entry = section["values"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["name"].as_str() == Some(*name))
                .unwrap_or_else(|| panic!("missing script_value_hash case `{name}`"));
            expect_eq_u64(
                "script_value_hash.values",
                idx,
                "hash",
                u(entry, "hash"),
                value.c4_value_hash() as u64,
            );
        }
    }

    // 9b. C4ScriptCnvMap conversion table + ConvertTo dispatch: C4Value.cpp:488-598.
    {
        let section = &golden["script_value_convert"];
        expect_eq(
            "script_value_convert",
            0,
            "type_count",
            i(section, "type_count"),
            C4VType::ALL.len() as i64,
        );

        // The 81-cell classification grid, source row × destination column.
        for (row, row_str) in section["table"].as_array().unwrap().iter().enumerate() {
            for (col, code) in row_str.as_str().unwrap().chars().enumerate() {
                let rust = cnv_fn(C4VType::ALL[row], C4VType::ALL[col]).code();
                assert_eq!(
                    code, rust,
                    "PARITY DIVERGENCE in `script_value_convert.table` cell [{row}][{col}]: \
                     C++ golden = {code}, Rust = {rust}"
                );
            }
        }

        // Per-(value, target type, #strict) ConvertTo results.
        for (idx, e) in section["convert"].as_array().unwrap().iter().enumerate() {
            let value = convert_case_value(e["name"].as_str().unwrap());
            expect_eq(
                "script_value_convert.convert",
                idx,
                "from",
                i(e, "from"),
                value.c4v_type().index() as i64,
            );
            let to = C4VType::ALL[i(e, "to") as usize];
            let strict = i(e, "strict") != 0;
            expect_eq(
                "script_value_convert.convert",
                idx,
                "result",
                i(e, "result"),
                value.convert_to(to, strict) as i64,
            );
        }
    }

    // 10. FnGetKiller/FnSetKiller (C4Script.cpp:1333-1347), whose C++
    // implementation delegates to the production C4ScriptKiller helper used
    // by the oracle. Drive the Rust HOST FUNCTIONS through the real script VM
    // so registration, default-self behavior, foreign/arrow dispatch and the
    // pending-update seam all participate in the differential.
    {
        let section = &golden["script_killer"];
        let caller_script = r#"#strict
local iInitial, iSetSelf, iReadSelf, iInvalid, iAfterInvalid;
local iClearSelf, iReadCleared, iSetForeign, iReadForeign;
local iArrowClear, iArrowRead;
func Trigger(object pOther) {
    iInitial = GetKiller();
    iSetSelf = SetKiller(1);
    iReadSelf = GetKiller();
    iInvalid = SetKiller(9);
    iAfterInvalid = GetKiller();
    iClearSelf = SetKiller(-1);
    iReadCleared = GetKiller();
    iSetForeign = SetKiller(1, pOther);
    iReadForeign = GetKiller(pOther);
    iArrowClear = pOther->SetKiller(-1);
    iArrowRead = pOther->GetKiller();
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(1, "P1"))
            .expect("killer differential player registers");
        engine
            .register_script_definition("CALL", "Caller", caller_script)
            .expect("killer differential caller registers");
        engine
            .register_script_definition("OTHR", "Other", "#strict\n")
            .expect("killer differential target registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("killer differential caller spawns");
        let other_id = engine
            .spawn_object(SpawnConfig::new("OTHR").with_category(CATEGORY_OBJECT))
            .expect("killer differential target spawns");
        let caller_idx = engine
            .find_object_index(caller_id)
            .expect("killer differential caller exists");
        engine
            .call_object_function(
                caller_idx,
                "Trigger",
                vec![ScriptValue::Object(other_id.as_u64())],
            )
            .expect("killer differential script runs");

        let caller_idx = engine
            .find_object_index(caller_id)
            .expect("killer differential caller remains");
        let locals = &engine.objects[caller_idx].state.local_vars;
        let rust_local = |name: &str| match locals.get(name) {
            Some(ScriptValue::Int(value)) => i64::from(*value),
            Some(ScriptValue::Bool(value)) => i64::from(*value),
            value => panic!("killer differential local `{name}` has unexpected value {value:?}"),
        };
        for (idx, (golden_key, local_name)) in [
            ("initial", "iInitial"),
            ("set_self", "iSetSelf"),
            ("read_self", "iReadSelf"),
            ("set_invalid", "iInvalid"),
            ("after_invalid", "iAfterInvalid"),
            ("clear_self", "iClearSelf"),
            ("read_cleared", "iReadCleared"),
            ("set_foreign", "iSetForeign"),
            ("read_foreign", "iReadForeign"),
            ("arrow_clear", "iArrowClear"),
            ("arrow_read", "iArrowRead"),
        ]
        .into_iter()
        .enumerate()
        {
            expect_eq(
                "script_killer",
                idx,
                golden_key,
                i(section, golden_key),
                rust_local(local_name),
            );
        }
        expect_eq(
            "script_killer",
            11,
            "self_final",
            i(section, "self_final"),
            i64::from(engine.objects[caller_idx].last_energy_loss_cause),
        );
        let other_idx = engine
            .find_object_index(other_id)
            .expect("killer differential target remains");
        expect_eq(
            "script_killer",
            12,
            "foreign_final",
            i(section, "foreign_final"),
            i64::from(engine.objects[other_idx].last_energy_loss_cause),
        );

        // No C4Aul object context: invoke the same registered Rust hosts from
        // a bare clonk-script engine. This matches C4ScriptKiller's null/null
        // oracle cases and pins the NO_OWNER/false fallbacks.
        let mut bare = clonk_script::Engine::new();
        crate::compat::register_host_functions(&mut bare);
        bare.add_script(
            clonk_script::Script::compile(
                "global func ReadNoContext() { return GetKiller(); }\n\
                 global func WriteNoContext() { return SetKiller(1); }\n",
            )
            .expect("bare killer differential script compiles"),
        );
        let bare_result = |function: &str, bare: &mut clonk_script::Engine| match bare
            .call(function, &[])
            .unwrap_or_else(|error| panic!("bare killer call `{function}` failed: {error}"))
        {
            ScriptValue::Int(value) => i64::from(value),
            ScriptValue::Bool(value) => i64::from(value),
            value => panic!("bare killer call `{function}` returned {value:?}"),
        };
        expect_eq(
            "script_killer",
            13,
            "get_no_context",
            i(section, "get_no_context"),
            bare_result("ReadNoContext", &mut bare),
        );
        expect_eq(
            "script_killer",
            14,
            "set_no_context",
            i(section, "set_no_context"),
            bare_result("WriteNoContext", &mut bare),
        );
        expect_eq(
            "script_killer",
            15,
            "no_owner_constant",
            i(section, "get_no_context"),
            i64::from(OWNER_NONE),
        );
    }

    // 10b. FnEval -> DirectExec context selection (C4Script.cpp:4501-4513;
    // C4AulExec.cpp:1674-1683). The C++ oracle executes both mechanically
    // extracted production blocks. Rust drives the same three contexts through
    // real C4Script: the object sentinel requires both its named local and its
    // definition-owned function, while DefinitionCall supplies Def without Obj
    // and global->eval clears both so Game.Script owns the expression.
    {
        let object_script = r#"#strict 2
local power;
func Probe()
{
    power = 50;
    return eval("Explode(power)");
}
func Explode(value) { return value + 1; }
"#;
        let definition_script = r#"#strict
func DefinitionProbe() { return eval("DefinitionHelper()"); }
func DefinitionHelper() { return 62; }
"#;
        let definition_caller_script = r#"#strict
func Probe() { return DefinitionCall(DEFV, "DefinitionProbe"); }
"#;
        let global_caller_script = r#"#strict 3
func Probe() { return global->eval("ScenarioHelper()"); }
"#;
        let scenario_script = r#"#strict 3
func ScenarioHelper() { return 73; }
"#;

        let mut engine = Engine::with_seed(29);
        for (id, name, script) in [
            ("OBJV", "Eval object receiver", object_script),
            ("DEFV", "Eval definition receiver", definition_script),
            ("CALL", "Eval definition caller", definition_caller_script),
            ("GEVL", "Eval game caller", global_caller_script),
        ] {
            engine
                .register_definition(
                    Definition::from_script(id, name, script)
                        .unwrap_or_else(|error| panic!("{name} compiles: {error}")),
                )
                .unwrap_or_else(|error| panic!("{name} registers: {error}"));
        }
        engine
            .install_scenario_script_with_convention("Scenario", scenario_script, true)
            .expect("eval differential scenario script installs");
        let object = engine
            .spawn_object(SpawnConfig::new("OBJV"))
            .expect("eval differential object receiver spawns");
        let definition_caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("eval differential definition caller spawns");
        let global_caller = engine
            .spawn_object(SpawnConfig::new("GEVL"))
            .expect("eval differential game caller spawns");

        let call_probe = |engine: &mut Engine, id| {
            let index = engine
                .find_object_index(id)
                .expect("eval differential caller remains");
            match engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("eval differential probe runs")
            {
                ScriptValue::Int(value) => i64::from(value),
                value => panic!("eval differential probe returned {value:?}"),
            }
        };
        let rust_results = HashMap::from([
            (
                "object_definition",
                (1_i64, 1_i64, 2_i64, 1_i64, call_probe(&mut engine, object)),
            ),
            (
                "definition_only",
                (
                    0_i64,
                    1_i64,
                    1_i64,
                    2_i64,
                    call_probe(&mut engine, definition_caller),
                ),
            ),
            (
                "game_script",
                (
                    0_i64,
                    0_i64,
                    3_i64,
                    3_i64,
                    call_probe(&mut engine, global_caller),
                ),
            ),
        ]);

        for (index, case) in golden["eval_direct_exec_context"]
            .as_array()
            .expect("eval_direct_exec_context is a C++ oracle array")
            .iter()
            .enumerate()
        {
            let name = case["name"]
                .as_str()
                .expect("eval_direct_exec_context case has a name");
            let &(has_object, has_definition, caller_strict, receiver, result) = rust_results
                .get(name)
                .unwrap_or_else(|| panic!("unknown eval_direct_exec_context case `{name}`"));
            for (field, rust) in [
                ("has_object", has_object),
                ("has_definition", has_definition),
                ("caller_strict", caller_strict),
                ("expected_receiver", receiver),
                ("receiver", receiver),
                ("scope_valid", 1),
                ("direct_strict", caller_strict),
                ("result", result),
            ] {
                expect_eq(
                    "eval_direct_exec_context",
                    index,
                    field,
                    i(case, field),
                    rust,
                );
            }
        }
    }

    // 10c. C4Effect::Execute passes pCommandTarget—not the affected pForObj—
    // to C4AulFunc::Exec (oracle-src-pinned src/C4Effect.cpp:319-363).
    // With only idCommandTarget set, the mechanically extracted C++ path
    // therefore gives FnGetX/FnGetY a null cthr->Obj while retaining the
    // carrier as the timer's first argument (src/C4AulExec.cpp:330-364,
    // 1638-1649; src/C4Script.cpp:1198-1202,1293-1297).
    {
        let section = &golden["definition_commanded_effect_position"];
        let carrier_script = r#"#strict 2
func Arm()
{
    return AddEffect("Origin", this(), 100, 1, 0, PROB);
}
"#;
        let callback_script = r#"#strict 2
func FxOriginTimer(object target, int number, int time)
{
    EffectVar(0, target, number) = GetX();
    EffectVar(1, target, number) = GetY();
    EffectVar(2, target, number) = GetX(target);
    EffectVar(3, target, number) = GetY(target);
    EffectVar(4, target, number) = time;
    EffectVar(5, target, number) = !this();
    EffectVar(6, target, number) = GetID(target) == CARR;
    EffectVar(7, target, number) = number;
    return 0;
}
"#;

        let mut carrier = Definition::from_script("CARR", "Effect carrier", carrier_script)
            .expect("effect receiver differential carrier compiles");
        carrier.set_c4_callback_convention(true);
        let mut callback = Definition::from_script("PROB", "Effect callback", callback_script)
            .expect("effect receiver differential callback compiles");
        callback.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(carrier)
            .expect("effect receiver differential carrier registers");
        engine
            .register_definition(callback)
            .expect("effect receiver differential callback registers");
        let carrier = engine
            .spawn_object(
                SpawnConfig::new("CARR")
                    .with_position(crate::Vector2::new(
                        i(section, "carrier_x") as i32,
                        i(section, "carrier_y") as i32,
                    ))
                    .with_mobile(false),
            )
            .expect("effect receiver differential carrier spawns");
        let carrier_index = engine
            .find_object_index(carrier)
            .expect("effect receiver differential carrier exists");
        engine
            .call_object_function(carrier_index, "Arm", Vec::new())
            .expect("definition-commanded effect installs");
        engine
            .tick_without_snapshot()
            .expect("definition-commanded effect timer runs");

        let carrier_index = engine
            .find_object_index(carrier)
            .expect("effect receiver differential carrier remains");
        let carrier_state = &engine.objects[carrier_index].state;
        let effect = carrier_state
            .effects
            .iter()
            .find(|effect| effect.name == "Origin")
            .expect("definition-commanded effect remains active");
        let var_i64 = |index: usize, field: &str| match effect.var(index) {
            EffectVarValue::Int(value) => i64::from(value),
            EffectVarValue::Bool(value) => i64::from(value),
            EffectVarValue::RawBool(value) => i64::from(value != 0),
            value => panic!(
                "definition_commanded_effect_position `{field}` has unexpected value {value:?}"
            ),
        };
        let position_var = |index: usize, field: &str| match effect.var(index) {
            EffectVarValue::Nil => Value::Null,
            EffectVarValue::Int(value) => Value::from(value),
            value => panic!(
                "definition_commanded_effect_position `{field}` has unexpected value {value:?}"
            ),
        };

        for (index, (field, rust)) in [
            ("carrier_x", i64::from(carrier_state.position.x)),
            ("carrier_y", i64::from(carrier_state.position.y)),
            (
                "has_id_command_target",
                i64::from(effect.command_id.as_deref() == Some("PROB")),
            ),
            (
                "command_target_is_null",
                i64::from(effect.command_target.is_none()),
            ),
            (
                "callback_ran",
                i64::from(!matches!(effect.var(4), EffectVarValue::Nil)),
            ),
            (
                "callback_receiver_is_null",
                var_i64(5, "callback_receiver_is_null"),
            ),
            (
                "callback_target_is_carrier",
                var_i64(6, "callback_target_is_carrier"),
            ),
            ("number", var_i64(7, "number")),
            ("time", var_i64(4, "time")),
        ]
        .into_iter()
        .enumerate()
        {
            expect_eq(
                "definition_commanded_effect_position",
                index,
                field,
                i(section, field),
                rust,
            );
        }
        for (index, (field, effect_var)) in [
            ("implicit_x", 0_usize),
            ("implicit_y", 1),
            ("explicit_x", 2),
            ("explicit_y", 3),
        ]
        .into_iter()
        .enumerate()
        {
            expect_json_eq(
                "definition_commanded_effect_position",
                index,
                field,
                section[field].clone(),
                position_var(effect_var, field),
            );
        }
    }

    // 10d. C4Effect routes the warning-only conversion marker at its
    // callback boundary only. The golden is emitted by the pinned C++
    // Execute and DoCall bodies together with their extracted script-function
    // conversion entry, so verify both deferred Timer and EffectCall paths.
    {
        let section = &golden["effect_callback_conversion"];
        let raw_fixed_x = |engine: &Engine, object| {
            let snapshot = engine
                .object_snapshot(object)
                .expect("effect conversion carrier remains live");
            snapshot
                .fixed_velocity
                .unwrap_or_else(|| FixedVec2::from_ints(snapshot.velocity.x, snapshot.velocity.y))
                .x
                .val()
        };

        let mut timer_pre_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut timer_pre_strict3,
            "TMHP",
            "Timer warning conversion host",
            r#"#strict 2
func Arm()
{
  return(AddEffect("Oracle", this(), 100, 1, 0, TMCP));
}
func Read()
{
  return(ReadTimerPreStrict3Value());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut timer_pre_strict3,
            "TMCP",
            "Timer warning conversion callback",
            r#"#strict 2
static callback_value;
func FxOracleTimer(int target, int number, int time)
{
  callback_value = GetType(target) == 4;
  return(0);
}
global func ReadTimerPreStrict3Value() { return(callback_value); }
"#,
        );
        let timer_pre_object = timer_pre_strict3
            .spawn_object(SpawnConfig::new("TMHP"))
            .expect("pre-strict3 timer carrier spawns");
        let timer_pre_index = timer_pre_strict3
            .find_object_index(timer_pre_object)
            .expect("pre-strict3 timer carrier exists");
        timer_pre_strict3
            .call_object_function(timer_pre_index, "Arm", Vec::new())
            .expect("pre-strict3 timer installs");
        timer_pre_strict3
            .tick_without_snapshot()
            .expect("pre-strict3 timer warns and runs");
        let timer_pre_reader = timer_pre_strict3
            .spawn_object(SpawnConfig::new("TMCP"))
            .expect("pre-strict3 timer callback reader spawns");
        let timer_pre_index = timer_pre_strict3
            .find_object_index(timer_pre_reader)
            .expect("pre-strict3 timer callback reader remains live");
        let timer_pre_value = timer_pre_strict3
            .call_object_function(timer_pre_index, "ReadTimerPreStrict3Value", Vec::new())
            .expect("pre-strict3 timer callback value reads");
        let mut timer_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut timer_strict3,
            "TMHS",
            "Strict timer conversion host",
            r#"#strict 3
func Arm()
{
  return(AddEffect("Oracle", this(), 100, 1, nil, TMCS));
}
func Read()
{
  return(ReadTimerStrict3Value());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut timer_strict3,
            "TMCS",
            "Strict timer conversion callback",
            r#"#strict 3
static callback_value;
func FxOracleTimer(int target, int number, int time)
{
  callback_value = 1;
  return(0);
}
global func ReadTimerStrict3Value() { return(callback_value); }
"#,
        );
        let timer_strict_object = timer_strict3
            .spawn_object(SpawnConfig::new("TMHS"))
            .expect("strict timer carrier spawns");
        let timer_strict_index = timer_strict3
            .find_object_index(timer_strict_object)
            .expect("strict timer carrier exists");
        timer_strict3
            .call_object_function(timer_strict_index, "Arm", Vec::new())
            .expect("strict timer installs");
        timer_strict3
            .tick_without_snapshot()
            .expect("strict timer conversion fails safe");
        let timer_strict_reader = timer_strict3
            .spawn_object(SpawnConfig::new("TMCS"))
            .expect("strict timer callback reader spawns");
        let timer_strict_index = timer_strict3
            .find_object_index(timer_strict_reader)
            .expect("strict timer callback reader remains live");
        let timer_strict_value = timer_strict3
            .call_object_function(timer_strict_index, "ReadTimerStrict3Value", Vec::new())
            .expect("strict timer callback value reads");

        let mut timer_strict3_reference = Engine::new();
        register_real_c4_effect_definition(
            &mut timer_strict3_reference,
            "TMHR",
            "Strict timer reference host",
            r#"#strict 3
func Arm()
{
  return(AddEffect("Oracle", this(), 100, 1, nil, TMCR));
}
func Read()
{
  return(ReadTimerStrict3ReferenceValue());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut timer_strict3_reference,
            "TMCR",
            "Strict timer reference callback",
            r#"#strict 3
static callback_value;
func FxOracleTimer(int &target, int number, int time)
{
  SetXDir(17, target);
  callback_value = 1;
  return(0);
}
global func ReadTimerStrict3ReferenceValue() { return(callback_value); }
"#,
        );
        let timer_reference_object = timer_strict3_reference
            .spawn_object(SpawnConfig::new("TMHR"))
            .expect("strict reference timer carrier spawns");
        let timer_reference_index = timer_strict3_reference
            .find_object_index(timer_reference_object)
            .expect("strict reference timer carrier exists");
        timer_strict3_reference
            .call_object_function(timer_reference_index, "Arm", Vec::new())
            .expect("strict reference timer installs");
        timer_strict3_reference
            .tick_without_snapshot()
            .expect("strict reference timer conversion fails safe");
        let timer_reference_reader = timer_strict3_reference
            .spawn_object(SpawnConfig::new("TMCR"))
            .expect("strict reference timer callback reader spawns");
        let timer_reference_index = timer_strict3_reference
            .find_object_index(timer_reference_reader)
            .expect("strict reference timer callback reader remains live");
        let timer_reference_value = timer_strict3_reference
            .call_object_function(
                timer_reference_index,
                "ReadTimerStrict3ReferenceValue",
                Vec::new(),
            )
            .expect("strict reference timer callback value reads");

        let mut call_pre_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut call_pre_strict3,
            "ECHP",
            "EffectCall warning conversion host",
            r#"#strict 2
func Probe()
{
  var number = AddEffect("Oracle", this(), 100, 0, 0, ECCP);
  return(EffectCall(this(), number, "Probe", this()));
}
"#,
        );
        register_real_c4_effect_definition(
            &mut call_pre_strict3,
            "ECCP",
            "EffectCall warning conversion callback",
            r#"#strict 2
func FxOracleProbe(object target, int number, int declared_but_unused)
{
  var id_matches = GetID(target) == ECHP;
  var same_object = target == declared_but_unused;
  var type_is_object = GetType(target) == 4;
  GetNeededMatStr(target);
  SetXDir(17, target);
  return([id_matches, same_object, type_is_object]);
}
"#,
        );
        let call_pre_object = call_pre_strict3
            .spawn_object(SpawnConfig::new("ECHP"))
            .expect("pre-strict3 EffectCall carrier spawns");
        let call_pre_index = call_pre_strict3
            .find_object_index(call_pre_object)
            .expect("pre-strict3 EffectCall carrier exists");
        let call_pre_result = call_pre_strict3
            .call_object_function(call_pre_index, "Probe", Vec::new())
            .expect("pre-strict3 EffectCall warns and runs");

        let mut call_strict3 = Engine::new();
        register_real_c4_effect_definition(
            &mut call_strict3,
            "ECHS",
            "Strict EffectCall conversion host",
            r#"#strict 3
func Probe()
{
  var number = AddEffect("Oracle", this(), 100, 0, nil, ECCS);
  return(EffectCall(this(), number, "Probe", this()));
}
func Read()
{
  return(ReadEffectCallStrict3Value());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut call_strict3,
            "ECCS",
            "Strict EffectCall conversion callback",
            r#"#strict 3
static callback_value;
func FxOracleProbe(object target, int number, int declared_but_unused)
{
  callback_value = 1;
  return(0);
}
global func ReadEffectCallStrict3Value() { return(callback_value); }
"#,
        );
        let call_strict_object = call_strict3
            .spawn_object(SpawnConfig::new("ECHS"))
            .expect("strict EffectCall carrier spawns");
        let call_strict_index = call_strict3
            .find_object_index(call_strict_object)
            .expect("strict EffectCall carrier exists");
        let call_strict_rejected = call_strict3
            .call_object_function(call_strict_index, "Probe", Vec::new())
            .is_err();
        let call_strict_reader = call_strict3
            .spawn_object(SpawnConfig::new("ECCS"))
            .expect("strict EffectCall callback reader spawns");
        let call_strict_index = call_strict3
            .find_object_index(call_strict_reader)
            .expect("strict EffectCall callback reader remains live");
        let call_strict_value = call_strict3
            .call_object_function(call_strict_index, "ReadEffectCallStrict3Value", Vec::new())
            .expect("strict EffectCall callback value reads");

        let mut call_strict3_reference = Engine::new();
        register_real_c4_effect_definition(
            &mut call_strict3_reference,
            "ECHR",
            "Strict EffectCall reference host",
            r#"#strict 3
func Probe()
{
  var number = AddEffect("Oracle", this(), 100, 0, nil, ECCR);
  return(EffectCall(this(), number, "Probe", this()));
}
func Read()
{
  return(ReadEffectCallStrict3ReferenceValue());
}
"#,
        );
        register_real_c4_effect_definition(
            &mut call_strict3_reference,
            "ECCR",
            "Strict EffectCall reference callback",
            r#"#strict 3
static callback_value;
func FxOracleProbe(object target, int number, int &declared_but_unused)
{
  SetXDir(17, target);
  callback_value = 1;
  return(0);
}
global func ReadEffectCallStrict3ReferenceValue() { return(callback_value); }
"#,
        );
        let call_reference_object = call_strict3_reference
            .spawn_object(SpawnConfig::new("ECHR"))
            .expect("strict reference EffectCall carrier spawns");
        let call_reference_index = call_strict3_reference
            .find_object_index(call_reference_object)
            .expect("strict reference EffectCall carrier exists");
        let call_reference_rejected = call_strict3_reference
            .call_object_function(call_reference_index, "Probe", Vec::new())
            .is_err();
        let call_reference_reader = call_strict3_reference
            .spawn_object(SpawnConfig::new("ECCR"))
            .expect("strict reference EffectCall callback reader spawns");
        let call_reference_index = call_strict3_reference
            .find_object_index(call_reference_reader)
            .expect("strict reference EffectCall callback reader remains live");
        let call_reference_value = call_strict3_reference
            .call_object_function(
                call_reference_index,
                "ReadEffectCallStrict3ReferenceValue",
                Vec::new(),
            )
            .expect("strict reference EffectCall callback value reads");

        let fields = [
            (
                "pre_strict3_callback_ran",
                i64::from(!matches!(&timer_pre_value, ScriptValue::Nil)),
            ),
            (
                "pre_strict3_original_object",
                i64::from(matches!(&timer_pre_value, ScriptValue::Bool(true))),
            ),
            (
                "strict3_rejected",
                i64::from(matches!(&timer_strict_value, ScriptValue::Nil)),
            ),
            (
                "strict3_callback_ran",
                i64::from(!matches!(&timer_strict_value, ScriptValue::Nil)),
            ),
            (
                "strict3_reference_rejected",
                i64::from(matches!(&timer_reference_value, ScriptValue::Nil)),
            ),
            (
                "strict3_reference_callback_ran",
                i64::from(!matches!(&timer_reference_value, ScriptValue::Nil)),
            ),
            (
                "strict3_reference_object_mutated",
                i64::from(raw_fixed_x(&timer_strict3_reference, timer_reference_object) != 0),
            ),
            (
                "effect_call_pre_strict3_callback_ran",
                i64::from(matches!(&call_pre_result, ScriptValue::Array(_))),
            ),
            (
                "effect_call_pre_strict3_type_is_object",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.get(2), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_identity_matches",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.get(1), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_id_matches",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.first(), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_target_equals_extra",
                i64::from(matches!(
                    &call_pre_result,
                    ScriptValue::Array(values)
                        if matches!(values.get(1), Some(ScriptValue::Bool(true)))
                )),
            ),
            (
                "effect_call_pre_strict3_object_mutated",
                i64::from(
                    raw_fixed_x(&call_pre_strict3, call_pre_object) == itofix_prec(17, 10).val(),
                ),
            ),
            (
                "effect_call_strict3_rejected",
                i64::from(call_strict_rejected),
            ),
            (
                "effect_call_strict3_callback_ran",
                i64::from(!matches!(&call_strict_value, ScriptValue::Nil)),
            ),
            (
                "effect_call_strict3_reference_rejected",
                i64::from(call_reference_rejected),
            ),
            (
                "effect_call_strict3_reference_callback_ran",
                i64::from(!matches!(&call_reference_value, ScriptValue::Nil)),
            ),
            (
                "effect_call_strict3_reference_object_mutated",
                i64::from(raw_fixed_x(&call_strict3_reference, call_reference_object) != 0),
            ),
        ];
        for (index, (field, rust)) in fields.into_iter().enumerate() {
            expect_eq(
                "effect_callback_conversion",
                index,
                field,
                i(section, field),
                rust,
            );
        }
    }

    // 11. C4Landscape::_PathFree (C4Landscape.cpp:890-915): PixCnt scans the
    //     authoritative Surface8 bytes. The second case is the minimized
    //     Goldrush frame-143 divergence: one water pixel at the right edge of
    //     a 17x15 cell must make the whole coarse cell occupied.
    for (idx, case) in golden["landscape_path"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut bytes = vec![0; 17 * 15];
        let pixel_x = i(case, "pixel_x") as i32;
        let pixel_y = i(case, "pixel_y") as i32;
        if pixel_x >= 0 && pixel_y >= 0 {
            bytes[pixel_y as usize * 17 + pixel_x as usize] = 1;
        }
        let mut densities = vec![0; 2];
        densities[1] = i(case, "density") as i32;
        let grid = PixelGrid::new(17, 15, bytes, densities, vec![None; 2], vec![None; 2]);
        let mut landscape = Landscape::flat(17, 15);
        landscape.set_pixel_grid(grid);
        expect_eq(
            "landscape_path",
            idx,
            "free",
            i(case, "free"),
            i64::from(landscape.path_free(0, 0, 16, 14, &crate::MaterialSet::new())),
        );
    }

    // 12. C4Object::ExecAction DFA_WALK + SetDir ordering
    //     (C4Object.cpp:4796-4826, 4249-4265, 4100-4187). This is the
    //     minimized Goldrush frame-170 WIPF case: Right ComDir accelerates a
    //     negative residual xdir to raw -19662, which rounds to zero but must
    //     still request Left, fire TurnAction and snap fix_x before movement.
    {
        let section = &golden["action_direction"];

        let (mut exec_action, id) = action_direction_engine();
        let idx = exec_action
            .find_object_index(id)
            .expect("oracle object exists");
        expect_eq(
            "action_direction",
            0,
            "returned_early",
            0,
            i64::from(
                exec_action
                    .apply_physics_at_index(idx)
                    .expect("action-direction physics applies"),
            ),
        );
        let object = &exec_action.objects[idx];
        expect_eq(
            "action_direction",
            0,
            "steered_xdir",
            i(section, "steered_xdir"),
            i64::from(object.fixed_velocity.x.val()),
        );
        expect_eq(
            "action_direction",
            0,
            "action_is_turn",
            i(section, "action_is_turn"),
            i64::from(object.state.action.name == "Turn"),
        );
        expect_eq(
            "action_direction",
            0,
            "direction",
            i(section, "direction"),
            i64::from(object.state.direction.to_script_value()),
        );
        expect_eq(
            "action_direction",
            0,
            "command_direction",
            i(section, "command_direction"),
            i64::from(object.state.command_direction.to_script_value()),
        );
        expect_eq(
            "action_direction",
            0,
            "fix_x_after_set_dir",
            i(section, "fix_x_after_set_dir"),
            i64::from(object.fixed_position.x.val()),
        );

        let (mut full_frame, id) = action_direction_engine();
        full_frame
            .tick_without_snapshot()
            .expect("oracle frame executes");
        let object = &full_frame.objects[full_frame
            .find_object_index(id)
            .expect("oracle object survives")];
        expect_eq(
            "action_direction",
            0,
            "action_phase",
            i(section, "action_phase"),
            i64::from(object.state.action.phase),
        );
        expect_eq(
            "action_direction",
            0,
            "action_time",
            i(section, "action_time"),
            i64::from(object.state.action.time),
        );
        expect_eq(
            "action_direction",
            0,
            "fix_x_after_move",
            i(section, "fix_x_after_move"),
            i64::from(object.fixed_position.x.val()),
        );
    }

    // 12b. Exact DFA_PUSH/PULL raw-xdir direction blocks and DFA_FIGHT's
    //      target-relative direction block (C4Object.cpp:5106-5108,
    //      5189-5192,5241-5243). These run through the full Rust executor so
    //      a later integer-velocity direction tail cannot mask the result.
    let procedure_direction_cases = golden["action_push_pull_fight_direction"]
        .as_array()
        .expect("procedure-direction golden is an array");
    let expected_procedure_direction_names = [
        "push_positive_subpixel",
        "pull_positive_subpixel",
        "fight_target_right_negative_velocity",
        "fight_equal_x_negative_velocity",
    ];
    assert_eq!(
        procedure_direction_cases.len(),
        expected_procedure_direction_names.len(),
        "procedure-direction golden must retain the complete extracted matrix"
    );
    for (case, expected_name) in procedure_direction_cases
        .iter()
        .zip(expected_procedure_direction_names)
    {
        assert_eq!(
            case["name"].as_str(),
            Some(expected_name),
            "procedure-direction golden row order/name drifted"
        );
    }
    for (idx, case) in procedure_direction_cases.iter().enumerate() {
        let name = case["name"]
            .as_str()
            .expect("procedure-direction case has a name");
        let (mut engine, actor_id) = action_push_pull_fight_direction_engine(case);
        let actor_idx = engine
            .find_object_index(actor_id)
            .expect("procedure-direction actor exists");
        let returned_early = engine
            .apply_physics_at_index(actor_idx)
            .expect("procedure-direction physics applies");
        assert!(
            !returned_early,
            "procedure-direction case `{name}` must reach the native phase tail"
        );
        let actor_idx = engine
            .find_object_index(actor_id)
            .expect("procedure-direction actor survives");
        let actor = &engine.objects[actor_idx];
        let turn_starts = match actor.state.local_vars.get("turn_starts") {
            Some(ScriptValue::Int(count)) => i64::from(*count),
            _ => 0,
        };
        let turn_start_dir = match actor.state.local_vars.get("turn_start_dir") {
            Some(ScriptValue::Int(direction)) => i64::from(*direction),
            _ => -1,
        };
        let action_is_turn = i64::from(actor.state.action.name == "Turn");
        // FlipDir=1 plus the deliberately cleared transform is controlled
        // instrumentation for zero-versus-one SetDir calls. The golden field
        // itself remains the C++ scaffold's explicit call count.
        let set_dir_call_probe = i64::from(actor.state.draw_transform.is_some());

        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "set_dir_calls",
            i(case, "set_dir_calls"),
            set_dir_call_probe,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "runs_turn_action",
            i(case, "runs_turn_action"),
            action_is_turn,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "turn_starts",
            i(case, "runs_turn_action"),
            turn_starts,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "turn_start_dir",
            i(case, "turn_start_dir"),
            turn_start_dir,
        );
        expect_eq(
            "action_push_pull_fight_direction",
            idx,
            "direction",
            i(case, "direction"),
            i64::from(actor.state.direction.to_script_value()),
        );
        if matches!(name, "push_positive_subpixel" | "pull_positive_subpixel") {
            expect_eq(
                "action_push_pull_fight_direction",
                idx,
                "xdir_raw",
                i(case, "xdir_raw"),
                i64::from(actor.fixed_velocity.x.val()),
            );
            expect_eq(
                "action_push_pull_fight_direction",
                idx,
                "xdir_pixel",
                i(case, "xdir_pixel"),
                i64::from(actor.state.velocity.x),
            );
        }
    }

    // 13. C4Object::ExecAction DFA_SWIM + SetDir ordering
    //     (C4Object.cpp:4946-4984, 4235-4254, 4168-4169). This is the
    //     minimized Goldrush frame-219 FISH case: Left ComDir creates a raw
    //     negative xdir, which must fire Swim.TurnAction and snap both fixed
    //     coordinates before movement while stale Swim advances Turn's phase.
    {
        let section = &golden["action_swim_direction"];

        let (mut exec_action, id) = swim_action_direction_engine();
        let idx = exec_action
            .find_object_index(id)
            .expect("oracle swimmer exists");
        expect_eq(
            "action_swim_direction",
            0,
            "returned_early",
            0,
            i64::from(
                exec_action
                    .apply_physics_at_index(idx)
                    .expect("swim-direction physics applies"),
            ),
        );
        let object = &exec_action.objects[idx];
        expect_eq(
            "action_swim_direction",
            0,
            "steered_xdir",
            i(section, "steered_xdir"),
            i64::from(object.fixed_velocity.x.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "steered_ydir",
            i(section, "steered_ydir"),
            i64::from(object.fixed_velocity.y.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "action_is_turn",
            i(section, "action_is_turn"),
            i64::from(object.state.action.name == "Turn"),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "direction",
            i(section, "direction"),
            i64::from(object.state.direction.to_script_value()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "command_direction",
            i(section, "command_direction"),
            i64::from(object.state.command_direction.to_script_value()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_x_after_set_dir",
            i(section, "fix_x_after_set_dir"),
            i64::from(object.fixed_position.x.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_y_after_set_dir",
            i(section, "fix_y_after_set_dir"),
            i64::from(object.fixed_position.y.val()),
        );

        let (mut full_frame, id) = swim_action_direction_engine();
        full_frame
            .tick_without_snapshot()
            .expect("oracle frame executes");
        let object = &full_frame.objects[full_frame
            .find_object_index(id)
            .expect("oracle swimmer survives")];
        expect_eq(
            "action_swim_direction",
            0,
            "action_phase",
            i(section, "action_phase"),
            i64::from(object.state.action.phase),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "action_time",
            i(section, "action_time"),
            i64::from(object.state.action.time),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_x_after_move",
            i(section, "fix_x_after_move"),
            i64::from(object.fixed_position.x.val()),
        );
        expect_eq(
            "action_swim_direction",
            0,
            "fix_y_after_move",
            i(section, "fix_y_after_move"),
            i64::from(object.fixed_position.y.val()),
        );
    }

    // 14. C4Object::SetAction callback dispatch (C4Object.cpp:4172-4208).
    //     Minimized from Goldrush frame 192, WIPF #565: script SetAction
    //     synchronously fires the new StartCall exactly once and before the
    //     old AbortCall; natural phase wraps likewise fire Start before End.
    for (idx, case) in golden["action_callbacks"]
        .as_array()
        .expect("action_callbacks is an array")
        .iter()
        .enumerate()
    {
        let name = case["name"]
            .as_str()
            .expect("action_callbacks case has a name");
        let (mut engine, id) = action_callbacks_engine(name);
        engine
            .tick_without_snapshot()
            .expect("callback fixture frame executes");
        expect_eq(
            "action_callbacks",
            idx,
            "completed",
            i(case, "completed"),
            i64::from(engine.find_object_index(id).is_some()),
        );
        expect_eq(
            "action_callbacks",
            idx,
            "callback_order",
            i(case, "callback_order"),
            action_callback_local(&engine, id, "callbackOrder"),
        );
        expect_eq(
            "action_callbacks",
            idx,
            "start_count",
            i(case, "start_count"),
            action_callback_local(&engine, id, "startCount"),
        );
        expect_eq(
            "action_callbacks",
            idx,
            "old_count",
            i(case, "old_count"),
            action_callback_local(&engine, id, "oldCount"),
        );
    }

    // 14b. C4Object.cpp DFA_CONNECT missing-target branch (5368-5376 in the
    //      pinned oracle): LineBreak(true) runs before AssignRemoval, whose
    //      Destruction callback runs while the line is still live. Call the
    //      real Engine procedure directly so its deleted object's callback
    //      locals remain observable before end-of-frame tombstone cleanup.
    expect_connect_removal_case(&golden, "connect_missing_target_removal", false);

    // 14c. The later geometry-break branch (pinned C4Object.cpp:5435-5441)
    //      calls LineBreak with no argument before the same AssignRemoval.
    //      A one-vertex line makes real C4Shape::LineConnect fail its pinned
    //      C4Shape.cpp:275 guard in both oracle and Rust fixtures.
    expect_connect_removal_case(&golden, "connect_geometry_break_removal", true);

    // 15. C4SolidMask constructor bitmap selection (C4SolidMask.cpp:400-412,
    //     C4Object.cpp:5908-5923). Minimized from Goldrush frame 184, CTWR
    //     #1351: source pixel (219,86) is transparent in Graphics.png but
    //     opaque in Graphics2.png. SetGraphics selects Graphics2 and rebuilds
    //     the put solid mask immediately.
    {
        let cases = golden["solid_mask_graphics"]
            .as_array()
            .expect("solid_mask_graphics is an array");
        let (mut engine, id) = solid_mask_graphics_engine();
        let vehicle = engine
            .landscape()
            .and_then(Landscape::grid_vehicle_byte)
            .expect("vehicle material exists");

        for (idx, case) in cases.iter().enumerate() {
            if i(case, "selected_variant") != 0 {
                let mut update = ObjectUpdate::new();
                update.base_graphics = Some(Some(ObjectBaseGraphics {
                    definition: "CTWR".to_string(),
                    graphics_name: Some("2".to_string()),
                    blit_mode: 0,
                }));
                engine
                    .apply_object_update(id, update)
                    .expect("SetGraphics update applies");
            }

            let object =
                &engine.objects[engine.find_object_index(id).expect("tower object survives")];
            let active_variant = object
                .state
                .base_graphics
                .as_ref()
                .and_then(|graphics| graphics.graphics_name.as_deref())
                == Some("2");
            expect_eq(
                "solid_mask_graphics",
                idx,
                "active_variant",
                i(case, "active_variant"),
                i64::from(active_variant),
            );
            expect_eq(
                "solid_mask_graphics",
                idx,
                "source_x",
                i(case, "source_x"),
                219,
            );
            expect_eq(
                "solid_mask_graphics",
                idx,
                "source_y",
                i(case, "source_y"),
                86,
            );
            let mask_pixel = engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(1, 1))
                .map_or(0, |pixel| if pixel == vehicle { 0xff } else { 0x00 });
            expect_eq(
                "solid_mask_graphics",
                idx,
                "mask_pixel",
                i(case, "mask_pixel"),
                mask_pixel,
            );
        }
    }

    // 15b. DefCore Scale -> Picture facet rect (C4Def.cpp:745 percent->float,
    //      C4Def.cpp:1341 Picture2Facet, C4Rect.cpp:37-44 Scaled). The Picture
    //      rect is authored in GAME units; the phase offset is composed there
    //      and only the resulting rect is scaled into bitmap space, so the
    //      truncation applies to the already-offset x. This is the contract any
    //      HD (Scale != 100) content depends on.
    for (idx, case) in golden["def_picture_scale"]
        .as_array()
        .expect("def_picture_scale is an array")
        .iter()
        .enumerate()
    {
        let engine = def_picture_scale_engine(
            u(case, "scale_percent") as u32,
            DefinitionPicture {
                x: i(case, "picture_x") as i32,
                y: i(case, "picture_y") as i32,
                width: i(case, "picture_wdt") as i32,
                height: i(case, "picture_hgt") as i32,
            },
        );
        let image = engine
            .definition_picture_phase_image("PSCL", i(case, "phase") as i32)
            .expect("scaled picture facet");
        expect_eq(
            "def_picture_scale",
            idx,
            "wdt",
            i(case, "wdt"),
            i64::from(image.width()),
        );
        expect_eq(
            "def_picture_scale",
            idx,
            "hgt",
            i(case, "hgt"),
            i64::from(image.height()),
        );
        // R/G of the first pixel are the source coordinates the crop started at.
        let pixels = image.pixels();
        expect_eq(
            "def_picture_scale",
            idx,
            "x",
            i(case, "x"),
            i64::from(pixels[0]),
        );
        expect_eq(
            "def_picture_scale",
            idx,
            "y",
            i(case, "y"),
            i64::from(pixels[1]),
        );
    }

    // 16. Movement: per-frame sub-pixel accumulation (the Theme-C core).
    //    fix_x += xdir; fix_y += (ydir += gravity); matching C4Movement.cpp.
    for scn in golden["movement"].as_array().unwrap() {
        let name = scn["name"].as_str().unwrap_or("?");
        let mut fix_x = itofix(0);
        let mut fix_y = itofix(0);
        let xdir = C4Fixed::from_raw(i(scn, "xdir") as i32);
        let mut ydir = C4Fixed::from_raw(i(scn, "ydir0") as i32);
        let grav = C4Fixed::from_raw(i(scn, "grav") as i32);
        for (frame, fr) in scn["frames"].as_array().unwrap().iter().enumerate() {
            ydir += grav;
            fix_x += xdir;
            fix_y += ydir;
            let label = format!("movement[{name}]");
            expect_eq(&label, frame, "fix_x", i(fr, "fix_x"), fix_x.val() as i64);
            expect_eq(&label, frame, "fix_y", i(fr, "fix_y"), fix_y.val() as i64);
            expect_eq(&label, frame, "xdir", i(fr, "xdir"), xdir.val() as i64);
            expect_eq(&label, frame, "ydir", i(fr, "ydir"), ydir.val() as i64);
            expect_eq(&label, frame, "x", i(fr, "x"), fixtoi(fix_x) as i64);
            expect_eq(&label, frame, "y", i(fr, "y"), fixtoi(fix_y) as i64);
        }
    }

    // 17. DFA_FLOAT clamps raw C4Fixed directions to FIXED100(Physical.Float),
    // including the zero default for a real resource without [Physical]
    // (C4InfoCore.cpp:239-242; C4Object.cpp:5291-5310). Resource provenance
    // and the FXP1-shaped fixture are covered by the focused engine test; this
    // bounded oracle keeps the raw clamp itself in the C++ golden.
    for (idx, case) in golden["native_float"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let limit = fixed100(i(case, "physical_float") as i32);
        let mut xdir = C4Fixed::from_raw(i(case, "xdir_before") as i32);
        let mut ydir = C4Fixed::from_raw(i(case, "ydir_before") as i32);
        if ydir < -limit {
            ydir = -limit;
        }
        if ydir > limit {
            ydir = limit;
        }
        if xdir > limit {
            xdir = limit;
        }
        if xdir < -limit {
            xdir = -limit;
        }
        expect_eq(
            "native_float",
            idx,
            "xdir_after",
            i(case, "xdir_after"),
            i64::from(xdir.val()),
        );
        expect_eq(
            "native_float",
            idx,
            "ydir_after",
            i(case, "ydir_after"),
            i64::from(ydir.val()),
        );
    }
}
