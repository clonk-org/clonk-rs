use crate::support::EngineTestExt;
use clonk_engine::{Definition, EffectVarValue, SpawnConfig};
use clonk_script::Value;

use crate::support::real_scenario::PreparedInstalledScenario;

pub(super) fn get_mat_adjust_tracks_default_and_same_call_set_value(
    prepared: &PreparedInstalledScenario,
) {
    // RGB is supplied by the installed System.c4g layer, just as it is for
    // the shipped Western scripts which call SetMatAdjust(RGB(...)).
    let mut engine = prepared.instantiate();
    engine.register_test_definition(crate::support::TestValueExt::test_value(
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
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("MADJ"));
    let index = engine.test_object_index(probe);

    assert_eq!(
        engine.call_test_object_function(index, "Probe", Vec::new()),
        Value::Int(2)
    );
    let locals = &engine.test_object_snapshot(probe).local_vars;
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
    engine.register_test_definition(crate::support::TestValueExt::test_value(
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
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("FADR"));
    let index = engine.test_object_index(probe);

    assert_eq!(
        engine.call_test_object_function(index, "StartFade", Vec::new()),
        Value::Int(0x0010_2131)
    );
    let locals = &engine.test_object_snapshot(probe).local_vars;
    assert_eq!(locals.get("iBefore"), Some(&Value::Int(0x0011_2233)));
    assert_eq!(locals.get("iDuring"), Some(&Value::Int(0x0010_2131)));

    let fade = crate::support::TestValueExt::test_value(
        engine
            .global_effects()
            .iter()
            .find(|effect| effect.name == "IntGlobalClrMod" && effect.priority != 0),
    );
    assert_eq!(fade.var(3), EffectVarValue::Int(0x0011_2233));

    assert_eq!(
        engine.call_test_object_function(index, "StopFade", Vec::new()),
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
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "FADM",
            "GoldRush material-fade driver",
            r#"#strict
        global func GetMatAdjust() { return(0); }
        public func StartFade() { GlobalFadeTo(245, 5); return(1); }
        "#,
        ),
    ));
    let driver = engine.spawn_test_object(SpawnConfig::new("FADM"));
    let index = engine.test_object_index(driver);
    assert_eq!(
        engine.call_test_object_function(index, "StartFade", Vec::new()),
        Value::Int(1)
    );

    let fade = crate::support::TestValueExt::test_value(
        engine
            .global_effects()
            .iter()
            .find(|effect| effect.name == "IntGlobalClrMod"),
    );
    assert_eq!(fade.interval, 4);

    for frame in 1..=4 {
        engine
            .tick_without_snapshot()
            .unwrap_or_else(|error| panic!("GoldRush fade tick {frame} succeeds: {error}"));
    }

    let fade = crate::support::TestValueExt::test_value(
        engine
            .global_effects()
            .iter()
            .find(|effect| effect.name == "IntGlobalClrMod"),
    );
    assert_eq!(
        fade.interval, 0,
        "the shipped timer reaches ChangeEffect after SetMatAdjust"
    );
}
