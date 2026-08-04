use clonk_engine::{Definition, EffectVarValue, SpawnConfig};
use clonk_script::Value;

use crate::support::real_scenario::PreparedInstalledScenario;

pub(super) fn get_mat_adjust_tracks_default_and_same_call_set_value(
    prepared: &PreparedInstalledScenario,
) {
    // RGB is supplied by the installed System.c4g layer, just as it is for
    // the shipped Western scripts which call SetMatAdjust(RGB(...)).
    let mut engine = prepared.instantiate();
    engine
        .register_definition(
            Definition::from_script(
                "MADJ",
                "SetMatAdjust probe",
                r#"#strict
local iBefore, iDefault, iCurrent, iAfter;

public func Probe()
{
    iBefore = 1;
    iDefault = GetMatAdjust();
    SetMatAdjust(RGB(50, 50, 50));
    iCurrent = GetMatAdjust();
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
        locals.get("iDefault"),
        Some(&Value::Int(0)),
        "C4Landscape::Default uses zero for normal, unmodulated drawing"
    );
    assert_eq!(locals.get("iCurrent"), Some(&Value::Int(0x0032_3232)));
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

pub(super) fn western_global_fade_restores_pre_fade_material_modulation(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    engine
        .register_definition(
            Definition::from_script(
                "FADR",
                "Western material-fade restoration probe",
                r#"#strict
local iBefore, iDuring, iAfter;

public func StartFade()
{
    SetMatAdjust(RGB(17, 34, 51));
    iBefore = GetMatAdjust();
    GlobalFadeTo(245, 5);
    iDuring = GetMatAdjust();
    return(iDuring);
}

public func StopFade()
{
    RemoveEffect("IntGlobalClrMod");
    iAfter = GetMatAdjust();
    return(iAfter);
}
"#,
            )
            .expect("the Western fade restoration probe compiles"),
        )
        .expect("the Western fade restoration probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("FADR"))
        .expect("the Western fade restoration probe spawns");
    let index = engine.find_object_index(probe).expect("the probe exists");

    assert_eq!(
        engine
            .call_object_function(index, "StartFade", Vec::new())
            .expect("the shipped Western fade starts"),
        Value::Int(0x0010_2131)
    );
    let locals = &engine
        .object_snapshot(probe)
        .expect("the fade probe remains active")
        .local_vars;
    assert_eq!(locals.get("iBefore"), Some(&Value::Int(0x0011_2233)));
    assert_eq!(locals.get("iDuring"), Some(&Value::Int(0x0010_2131)));

    let fade = engine
        .global_effects()
        .iter()
        .find(|effect| effect.name == "IntGlobalClrMod" && effect.priority != 0)
        .expect("GlobalFadeTo installs its landscape/sky effect");
    assert_eq!(fade.var(3), EffectVarValue::Int(0x0011_2233));

    assert_eq!(
        engine
            .call_object_function(index, "StopFade", Vec::new())
            .expect("the shipped Western fade stop restores modulation"),
        Value::Int(0x0011_2233)
    );
    assert_eq!(
        engine
            .snapshot()
            .landscape
            .expect("the probe landscape remains installed")
            .modulation(),
        0x0011_2233
    );
    assert!(engine
        .global_effects()
        .iter()
        .all(|effect| effect.name != "IntGlobalClrMod" || effect.priority == 0));
}

pub(super) fn gold_rush_global_fade_timer_reaches_its_completion_check(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert!(engine.debug_global_has_function("GlobalFadeTo"));
    assert!(engine.debug_global_has_function("FxIntGlobalClrModTimer"));

    // The native GetMatAdjust reads the live landscape modulation
    // (C4Script.cpp:4638-4642). A script-level global shadows a host function
    // (`Vm::invoke_engine_raw`), so this narrow shim pins the value the
    // shipped fade saves at the engine default and keeps the regression on
    // the later SetMatAdjust call and its completion check.
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
            .tick_without_snapshot()
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
