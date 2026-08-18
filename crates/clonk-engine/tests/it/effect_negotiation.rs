use crate::support::real_scenario::content_root;
use crate::support::EngineTestExt;
use clonk_engine::{Definition, Engine, SpawnConfig};
use clonk_resources::Group;
use clonk_script::Value;

#[test]
fn construction_effect_command_target_callbacks_keep_the_inflight_object_as_this() {
    // C4Effect stores the command-target pointer and executes both local and
    // recursively resolved global callbacks with that pointer as `this`,
    // even while Construction is running before normal object-list insertion
    // (src/C4Effect.cpp:42-56,439-456).
    let script = r#"#strict 2
local local_started, global_started;

func Construction()
{
  AddEffect("BirthLocal", this(), 100, 0, this());
  AddEffect("BirthGlobal", this(), 100, 0, this());
}

func FxBirthLocalStart(object target, int number, int temp)
{
  if (this() != target) return -1;
  if (temp) return 0;
  local_started = 1;
  SetR(11);
  return 0;
}

global func FxBirthGlobalStart(object target, int number, int temp)
{
  if (this() != target || GetR() != 11) return -1;
  // A global func may not name a declaring host's local
  // (C4AulParse.cpp:2000-2004); LocalN is the route C4Aul leaves it.
  LocalN("global_started") = 1;
  SetR(37);
  return 0;
}
"#;

    let mut engine = Engine::new();
    engine.register_test_script_definition("BORN", "Construction effect target", script);

    let object = engine.spawn_test_object(SpawnConfig::new("BORN"));
    let snapshot = engine.test_object_snapshot(object);

    assert_eq!(snapshot.rotation, 37);
    assert_eq!(
        snapshot.local_vars.get("local_started"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        snapshot.local_vars.get("global_started"),
        Some(&Value::Int(1))
    );
    assert_eq!(snapshot.effects.len(), 2);
    assert!(snapshot.effects.iter().all(|effect| {
        effect.priority == 100
            && effect.command_target == Some(object.as_u64() as i32)
            && effect.command_id.as_deref() == Some("BORN")
    }));
}

#[test]
fn live_object_add_effect_checks_once_with_cpp_arguments() {
    // C4Effect::New inserts the pending effect and synchronously asks each
    // same/higher-priority checker through Fx<Name>Effect with
    // [new name, affected object, checker number, nil, rVal1..4]. A -1
    // answer makes AddEffect return zero without validating the pending
    // effect (src/C4Effect.cpp:97-116,271-285).
    let script = r#"#strict
local iChecker;
local iUpper;
local iChecks;
local iDenyExact;
local iPassExact;
local iStarts;
local iStartExact;
local iStartVarsClean;
local iStartInline;
local iStartOrder;
local iDeniedStops;
local iNested;

func Install()
{
  iChecks = iDenyExact = iPassExact = iStarts = iStartExact = iStartVarsClean = iStartInline = iStartOrder = iDeniedStops = 0;
  iChecker = AddEffect("Guard", this(), 200, 0, this());
  iUpper = AddEffect("Upper", this(), 300, 0, this());
  return(iChecker);
}

func AddDenied()
{
  return(AddEffect("Denied", this(), 100, 7, this(), 0, 11, 12, 13, 14));
}

func AddAllowed()
{
  var iResult = AddEffect("Allowed", this(), 100, 9, this(), 0, 21, 22, 23, 24);
  if(iStarts == 1 && iStartOrder == 123) iStartInline = 1;
  return(iResult);
}

func AddStartDenied()
{
  return(AddEffect("StartDenied", this(), 100, 0, this()));
}

func AddReentrantDeniedThenAfter()
{
  iNested = 0;
  AddEffect("ReentrantDeny", this(), 100, 0, this());
  var iAfter = AddEffect("AfterReentrantDeny", this(), 1, 0, this());
  return(iNested * 100 + iAfter);
}

func AddStartDeniedThenAfter()
{
  var iDenied = AddEffect("StartDenied", this(), 100, 0, this());
  var iAfter = AddEffect("AfterStartDeny", this(), 1, 0, this());
  return(iDenied * 100 + iAfter);
}

func FxGuardEffect(string szNew, object pTarget, int iNumber, int iNewNumber,
                   int iVal1, int iVal2, int iVal3, int iVal4)
{
  ++iChecks;
  if(szNew eq "ReentrantDeny")
  {
    iNested = AddEffect("Nested", this(), 1, 0, this());
    return(-1);
  }
  if(szNew eq "Denied")
  {
    if(pTarget == this() && iNumber == iChecker && !iNewNumber &&
       iVal1 == 11 && iVal2 == 12 && iVal3 == 13 && iVal4 == 14)
      ++iDenyExact;
    return(-1);
  }
  if(szNew eq "Allowed")
  {
    if(pTarget == this() && iNumber == iChecker && !iNewNumber &&
       iVal1 == 21 && iVal2 == 22 && iVal3 == 23 && iVal4 == 24)
      ++iPassExact;
  }
  return(0);
}

func FxUpperStop(object pTarget, int iNumber, int iReason, bool fTemp)
{
  if(iReason == 1 && fTemp) iStartOrder = iStartOrder * 10 + 1;
}

func FxUpperStart(object pTarget, int iNumber, int iTemp)
{
  if(iTemp == 1) iStartOrder = iStartOrder * 10 + 3;
}

func FxAllowedStart(object pTarget, int iNumber, int iTemp,
                    int iVal1, int iVal2, int iVal3, int iVal4)
{
  iStartOrder = iStartOrder * 10 + 2;
  if(!iTemp && iVal1 == 21 && iVal2 == 22 && iVal3 == 23 && iVal4 == 24)
    ++iStartExact;
  if(!EffectVar(0, pTarget, iNumber) && !EffectVar(1, pTarget, iNumber))
    ++iStartVarsClean;
  EffectVar(2, pTarget, iNumber) = 77;
  ++iStarts;
}

func FxStartDeniedStart() { return(-1); }
func FxStartDeniedStop() { ++iDeniedStops; }
"#;

    let mut engine = Engine::new();
    engine.register_test_script_definition("FXCK", "Effect checker", script);

    let denied = engine.spawn_test_object(SpawnConfig::new("FXCK"));
    let denied_index = engine.test_object_index(denied);
    let checker = engine.call_test_object_function(denied_index, "Install", Vec::new());
    assert!(matches!(checker, Value::Int(number) if number > 0));
    assert_eq!(
        engine.call_test_object_function(denied_index, "AddDenied", Vec::new()),
        Value::Int(0),
        "C4Fx_Effect_Deny maps to AddEffect's zero return"
    );
    let denied = engine.test_object_snapshot(denied);
    assert_eq!(
        denied
            .effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .map(|effect| effect.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Guard", "Upper"],
        "the denied pending effect never becomes live"
    );
    assert!(denied
        .effects
        .iter()
        .any(|effect| effect.name == "Denied" && effect.priority == 0));
    assert_eq!(denied.local_vars.get("iChecks"), Some(&Value::Int(1)));
    assert_eq!(denied.local_vars.get("iDenyExact"), Some(&Value::Int(1)));

    let allowed = engine.spawn_test_object(SpawnConfig::new("FXCK"));
    let allowed_index = engine.test_object_index(allowed);
    engine.call_test_object_function(allowed_index, "Install", Vec::new());
    assert!(matches!(
        engine
            .call_test_object_function(allowed_index, "AddAllowed", Vec::new()),
        Value::Int(number) if number > 0
    ));
    let allowed = engine.test_object_snapshot(allowed);
    assert_eq!(
        allowed.local_vars.get("iChecks"),
        Some(&Value::Int(1)),
        "the deferred Started event must not repeat the synchronous check"
    );
    assert_eq!(allowed.local_vars.get("iPassExact"), Some(&Value::Int(1)));
    assert_eq!(allowed.local_vars.get("iStartExact"), Some(&Value::Int(1)));
    assert_eq!(
        allowed.local_vars.get("iStartVarsClean"),
        Some(&Value::Int(1)),
        "constructor rVals reach Start but do not prepopulate EffectVars"
    );
    assert_eq!(
        allowed.local_vars.get("iStartInline"),
        Some(&Value::Int(1)),
        "the upper temp-stop, new Start, and upper temp-start all finish inside AddEffect"
    );
    assert_eq!(
        allowed.local_vars.get("iStarts"),
        Some(&Value::Int(1)),
        "the passing effect still receives exactly one Start callback"
    );
    let allowed_effect = crate::support::TestValueExt::test_value(
        allowed
            .effects
            .iter()
            .find(|effect| effect.name == "Allowed"),
    );
    assert_eq!(
        allowed_effect.vars(),
        &[
            clonk_engine::EffectVarValue::Nil,
            clonk_engine::EffectVarValue::Nil,
            clonk_engine::EffectVarValue::Int(77),
        ],
        "only the explicit EffectVar write persists"
    );

    let reentrant = engine.spawn_test_object(SpawnConfig::new("FXCK"));
    let reentrant_index = engine.test_object_index(reentrant);
    engine.call_test_object_function(reentrant_index, "Install", Vec::new());
    assert_eq!(
        engine
            .call_test_object_function(
                reentrant_index,
                "AddReentrantDeniedThenAfter",
                Vec::new(),
            ),
        Value::Int(405),
        "the pending outer node consumes #3 before Check, so its nested and subsequent adds get #4 and #5"
    );

    let start_denied = engine.spawn_test_object(SpawnConfig::new("FXCK"));
    let start_denied_index = engine.test_object_index(start_denied);
    engine.call_test_object_function(start_denied_index, "Install", Vec::new());
    assert_eq!(
        engine
            .call_test_object_function(start_denied_index, "AddStartDeniedThenAfter", Vec::new(),),
        Value::Int(304),
        "the denied #3 remains linked through the script call, so the next add gets #4"
    );
    let start_denied = engine.test_object_snapshot(start_denied);
    assert!(
        start_denied
            .effects
            .iter()
            .any(|effect| effect.name == "StartDenied" && effect.priority == 0),
        "C4Fx_Start_Deny leaves the unvalidated node linked dead"
    );
    assert_eq!(
        start_denied.local_vars.get("iDeniedStops"),
        Some(&Value::Nil),
        "a Start-denied effect dies without Stop"
    );
}

#[test]
fn live_object_remove_effect_finishes_kill_inline() {
    // FnRemoveEffect calls C4Effect::Kill synchronously: upper effects are
    // temp-stopped high-to-low, the victim Stop runs, and uppers are
    // restarted low-to-high before the caller continues. A Stop denial
    // restores the victim, and its RNG draw precedes the caller's next draw
    // (C4Script.cpp:5508-5511; C4Effect.cpp:365-405,473-510).
    let script = r#"#strict 3
local iOrder, iStopRandom, iAfterRandom, iSawVictimVar;

func Install()
{
  AddEffect("Victim", this(), 100, 0, this());
  AddEffect("Upper", this(), 200, 0, this());
  return(1);
}

func RemoveInline()
{
  iOrder = iStopRandom = iAfterRandom = iSawVictimVar = 0;
  var removed = RemoveEffect("Victim", this());
  iAfterRandom = Random(100);
  var victim = GetEffect("Victim", this());
  if(victim) iSawVictimVar = EffectVar(0, this(), victim);
  iOrder = iOrder * 10 + 4;
  return([removed, iStopRandom, iAfterRandom, iSawVictimVar, iOrder]);
}

func FxUpperStop(object target, int number, int reason, bool temp)
{
  if(reason == 1 && temp) iOrder = iOrder * 10 + 1;
}

func FxVictimStop(object target, int number, int reason)
{
  iOrder = iOrder * 10 + 2;
  iStopRandom = Random(100);
  EffectVar(0, target, number) = 77;
  return(-1);
}

func FxUpperStart(object target, int number, int temp)
{
  if(temp == 1) iOrder = iOrder * 10 + 3;
}
"#;

    let mut engine = Engine::with_seed(7);
    engine.register_test_script_definition("FXRM", "Inline effect remover", script);
    let target = engine.spawn_test_object(SpawnConfig::new("FXRM"));
    let target_index = engine.test_object_index(target);
    engine.call_test_object_function(target_index, "Install", Vec::new());

    let mut expected_rng = engine.debug_rng_clone();
    let expected_stop = expected_rng.random(100);
    let expected_after = expected_rng.random(100);
    assert_eq!(
        engine
            .call_test_object_function(target_index, "RemoveInline", Vec::new()),
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(expected_stop),
            Value::Int(expected_after),
            Value::Int(77),
            Value::Int(1234),
        ]),
        "the Kill bracket, denial recovery, and Stop RNG draw all finish before RemoveEffect returns"
    );

    assert_eq!(
        engine
            .test_object_snapshot(target)
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Victim", "Upper"],
        "FxVictimStop's -1 restores the same effect"
    );
}

#[test]
fn live_object_kill_of_temp_removed_effect_restarts_for_removal() {
    // C4Effect::Kill on an inactive upper effect first runs
    // Fx*Start(C4FxCall_TempAddForRemoval), then the ordinary Fx*Stop. This
    // happens when the lower effect removes the upper one from its Start
    // callback while AddEffect's temp bracket is active
    // (C4Effect.cpp:376-400).
    let script = r#"#strict 3
local iOrder, iStartTemp, iStopReason;

func Install()
{
  AddEffect("Upper", this(), 200, 0, this());
  return(1);
}

func Trigger()
{
  iOrder = 0;
  iStartTemp = iStopReason = -1;
  AddEffect("Lower", this(), 100, 0, this());
  return([iOrder, iStartTemp, iStopReason]);
}

func FxLowerStart(object target)
{
  RemoveEffect("Upper", target);
}

func FxUpperStart(object target, int number, int temp)
{
  if(temp == 2)
  {
    iOrder = iOrder * 10 + 1;
    iStartTemp = temp;
  }
}

func FxUpperStop(object target, int number, int reason, bool temp)
{
  if(!temp)
  {
    iOrder = iOrder * 10 + 2;
    iStopReason = reason;
  }
}
"#;

    let mut engine = Engine::new();
    engine.register_test_script_definition("FXTR", "Temp-removed effect killer", script);
    let target = engine.spawn_test_object(SpawnConfig::new("FXTR"));
    let target_index = engine.test_object_index(target);
    engine.call_test_object_function(target_index, "Install", Vec::new());

    assert_eq!(
        engine.call_test_object_function(target_index, "Trigger", Vec::new()),
        Value::Array(vec![Value::Int(12), Value::Int(2), Value::Int(0)]),
        "the inactive upper effect receives Start(2) before Stop(0)"
    );
}

#[test]
fn shipped_hazard_jumper_bite_check_uses_strict1_raw_string_identity() {
    let group = crate::support::TestValueExt::test_value(Group::open(
        content_root().join("Hazard.c4d/Enemies.c4d/Jumper.c4d"),
    ));
    let resource = crate::support::TestValueExt::test_value(
        clonk_resources::definition::Definition::load(&group),
    );

    let mut engine = Engine::new();
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_resource(&resource),
    ));
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "FXDR",
            "Effect driver",
            r#"#strict 3
        func Probe(object target)
        {
          AddEffect("Bite", target, 99, 1, target);
          return AddEffect("Bite", target, 99, 1, target);
        }
        "#,
        ),
    ));

    let jumper = engine.spawn_test_object(SpawnConfig::new("ALN2").with_loaded(true));
    let driver = engine.spawn_test_object(SpawnConfig::new("FXDR"));
    let driver_index = engine.test_object_index(driver);
    assert_eq!(
        engine.call_test_object_function(
            driver_index,
            "Probe",
            vec![Value::Object(jumper.as_u64())]
        ),
        Value::Int(2),
        "fresh FxBiteEffect name must not raw-equal its interned strict1 literal"
    );

    let mut bite_numbers = engine
        .test_object_snapshot(jumper)
        .effects
        .iter()
        .filter(|effect| effect.name == "Bite")
        .map(|effect| effect.number)
        .collect::<Vec<_>>();
    bite_numbers.sort_unstable();
    assert_eq!(bite_numbers, [1, 2]);
}

/// A `Stop` callback answering `C4Fx_Stop_Deny` leaves the effect exactly where
/// it was.
///
/// `C4Effect::Kill` remembers `iPrevPrio`, calls `SetDead()` (which zeroes the
/// priority), and on `C4Fx_Stop_Deny` restores `iPriority = iPrevPrio`
/// (`C4Effect.cpp:365-396`). The effect is never unlinked and relinked, so its
/// list position, number and variables are all untouched — which is what makes
/// a denial different from a remove followed by a fresh add.
///
/// The port unlinks and re-inserts on this path, so the recovered effect has to
/// land back in the same slot. Priority-ordered insertion only does that if the
/// recovered effect still carries its original priority rather than the zero
/// `SetDead` would leave.
#[test]
fn a_denied_stop_keeps_its_list_position_number_and_variables() {
    let mut engine = Engine::new();
    engine.register_test_script_definition(
        "DENY",
        "Denied stop retains its slot",
        r#"#strict 2
local iStops;

func Install()
{
  iStops = 0;
  AddEffect("Low", this(), 100, 0, this());
  AddEffect("Mid", this(), 200, 0, this());
  AddEffect("High", this(), 300, 0, this());
  return(0);
}

func FxMidStart(object target, int number, int temp)
{
  if (!temp) EffectVar(0, target, number) = 4242;
  return(0);
}

// Refuse the removal. The temp calls that bracket an unrelated removal must
// not be refused, or the effect would never be re-added.
func FxMidStop(object target, int number, int reason, bool temp)
{
  if (temp) return(0);
  ++iStops;
  return(-1);
}

func Deny()
{
  RemoveEffect("Mid", this());
  return(iStops);
}

func ReadMidVar()
{
  return(EffectVar(0, this(), GetEffect("Mid", this())));
}
"#,
    );

    let object = engine.spawn_test_object(SpawnConfig::new("DENY"));
    let index = engine.test_object_index(object);
    engine.call_test_object_function(index, "Install", Vec::new());

    let before = engine
        .test_object_snapshot(object)
        .effects
        .iter()
        .map(|effect| (effect.name.clone(), effect.number, effect.priority))
        .collect::<Vec<_>>();
    assert_eq!(before.len(), 3, "three effects are installed: {before:?}");

    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "Deny", Vec::new()),
        Value::Int(1),
        "the Stop callback ran once and refused"
    );

    let after = engine
        .test_object_snapshot(object)
        .effects
        .iter()
        .map(|effect| (effect.name.clone(), effect.number, effect.priority))
        .collect::<Vec<_>>();
    assert_eq!(
        after, before,
        "a denied removal restores the effect in place, with its priority and \
         number, rather than re-adding it at the head or tail"
    );

    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "ReadMidVar", Vec::new()),
        Value::Int(4242),
        "and its EffectVars survive the denial"
    );
}
