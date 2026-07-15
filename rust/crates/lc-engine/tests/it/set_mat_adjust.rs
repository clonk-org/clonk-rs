use lc_engine::{Definition, SpawnConfig};
use lc_script::Value;

use crate::support::real_scenario::load_installed_scenario;

#[test]
fn set_mat_adjust_accepts_rgb_and_the_calling_script_continues() {
    // RGB is supplied by the installed System.c4g layer, just as it is for
    // the shipped Western scripts which call SetMatAdjust(RGB(...)).
    let mut engine = load_installed_scenario("Western.c4f/Goldrush.c4s", 0);
    engine
        .register_definition(
            Definition::from_script(
                "MADJ",
                "SetMatAdjust probe",
                r#"#strict
local iBefore, iAfter;

public func Probe()
{
    iBefore = 1;
    SetMatAdjust(RGB(50, 50, 50));
    iAfter = 2;
    return(iAfter);
}
"#,
            )
            .expect("the SetMatAdjust probe compiles"),
        )
        .expect("the SetMatAdjust probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("MADJ"))
        .expect("the SetMatAdjust probe spawns");
    let index = engine.find_object_index(probe).expect("the probe exists");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("SetMatAdjust(RGB(50, 50, 50)) executes"),
        Value::Int(2)
    );
    let locals = &engine
        .object_snapshot(probe)
        .expect("the probe remains active")
        .local_vars;
    assert_eq!(locals.get("iBefore"), Some(&Value::Int(1)));
    assert_eq!(
        locals.get("iAfter"),
        Some(&Value::Int(2)),
        "SetMatAdjust must not abort the calling script"
    );
    assert_eq!(
        engine
            .snapshot()
            .landscape
            .expect("the probe landscape remains installed")
            .modulation(),
        0x0032_3232,
        "RGB(50, 50, 50) becomes the live C4Landscape modulation dword"
    );
}

#[test]
fn gold_rush_global_fade_timer_reaches_its_completion_check() {
    let mut engine = load_installed_scenario("Western.c4f/Goldrush.c4s", 0);
    assert!(engine.debug_global_has_function("GlobalFadeTo"));
    assert!(engine.debug_global_has_function("FxIntGlobalClrModTimer"));

    // GetMatAdjust is tracked separately by CLO-163. This narrow shim lets
    // the shipped fade save its initial value so this CLO-159 regression can
    // isolate the later SetMatAdjust call and completion check.
    engine
        .register_definition(
            Definition::from_script(
                "FADM",
                "GoldRush material-fade driver",
                r#"#strict
global func GetMatAdjust() { return(0); }
public func StartFade() { GlobalFadeTo(245, 5); return(1); }
"#,
            )
            .expect("the GoldRush fade driver compiles"),
        )
        .expect("the GoldRush fade driver registers");
    let driver = engine
        .spawn_object(SpawnConfig::new("FADM"))
        .expect("the GoldRush fade driver spawns");
    let index = engine.find_object_index(driver).expect("the driver exists");
    assert_eq!(
        engine
            .call_object_function(index, "StartFade", Vec::new())
            .expect("the shipped global fade starts through SetMatAdjust"),
        Value::Int(1)
    );

    let fade = engine
        .global_effects()
        .iter()
        .find(|effect| effect.name == "IntGlobalClrMod")
        .expect("GlobalFadeTo installs its landscape/sky effect");
    assert_eq!(fade.interval, 4);

    for frame in 1..=4 {
        engine
            .tick()
            .unwrap_or_else(|error| panic!("GoldRush fade tick {frame} succeeds: {error}"));
    }

    let fade = engine
        .global_effects()
        .iter()
        .find(|effect| effect.name == "IntGlobalClrMod")
        .expect("an abnormal completed fade remains installed");
    assert_eq!(
        fade.interval, 0,
        "the shipped timer reaches ChangeEffect after SetMatAdjust"
    );
}
