use crate::support::real_scenario::content_root;
use lc_engine::{Definition, Engine, SpawnConfig};
use lc_resources::Group;
use lc_script::Value;

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
local iStartInline;
local iStartOrder;
local iDeniedStops;
local iNested;

func Install()
{
  iChecks = iDenyExact = iPassExact = iStarts = iStartInline = iStartOrder = iDeniedStops = 0;
  iChecker = AddEffect("Guard", this(), 200, 0, this());
  iUpper = AddEffect("Upper", this(), 300, 0, this());
  return(iChecker);
}

func AddDenied()
{
  return(AddEffect("Denied", this(), 100, 7, this(), nil, 11, 12, 13, 14));
}

func AddAllowed()
{
  var iResult = AddEffect("Allowed", this(), 100, 9, this(), nil, 21, 22, 23, 24);
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

func FxAllowedStart()
{
  iStartOrder = iStartOrder * 10 + 2;
  ++iStarts;
}

func FxStartDeniedStart() { return(-1); }
func FxStartDeniedStop() { ++iDeniedStops; }
"#;

    let mut engine = Engine::new();
    engine
        .register_definition(
            Definition::from_script("FXCK", "Effect checker", script)
                .expect("effect checker script compiles"),
        )
        .expect("effect checker definition registers");

    let denied = engine
        .spawn_object(SpawnConfig::new("FXCK"))
        .expect("denial probe spawns");
    let denied_index = engine
        .find_object_index(denied)
        .expect("denial probe remains live");
    let checker = engine
        .call_object_function(denied_index, "Install", Vec::new())
        .expect("checker installs");
    assert!(matches!(checker, Value::Int(number) if number > 0));
    assert_eq!(
        engine
            .call_object_function(denied_index, "AddDenied", Vec::new())
            .expect("denied AddEffect completes"),
        Value::Int(0),
        "C4Fx_Effect_Deny maps to AddEffect's zero return"
    );
    let denied = engine
        .object_snapshot(denied)
        .expect("denial probe remains live");
    assert_eq!(
        denied
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Guard", "Upper"],
        "the denied pending effect never becomes live"
    );
    assert_eq!(denied.local_vars.get("iChecks"), Some(&Value::Int(1)));
    assert_eq!(denied.local_vars.get("iDenyExact"), Some(&Value::Int(1)));

    let allowed = engine
        .spawn_object(SpawnConfig::new("FXCK"))
        .expect("passing probe spawns");
    let allowed_index = engine
        .find_object_index(allowed)
        .expect("passing probe remains live");
    engine
        .call_object_function(allowed_index, "Install", Vec::new())
        .expect("checker installs");
    assert!(matches!(
        engine
            .call_object_function(allowed_index, "AddAllowed", Vec::new())
            .expect("passing AddEffect completes"),
        Value::Int(number) if number > 0
    ));
    let allowed = engine
        .object_snapshot(allowed)
        .expect("passing probe remains live");
    assert_eq!(
        allowed.local_vars.get("iChecks"),
        Some(&Value::Int(1)),
        "the deferred Started event must not repeat the synchronous check"
    );
    assert_eq!(allowed.local_vars.get("iPassExact"), Some(&Value::Int(1)));
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

    let reentrant = engine
        .spawn_object(SpawnConfig::new("FXCK"))
        .expect("reentrant-number probe spawns");
    let reentrant_index = engine
        .find_object_index(reentrant)
        .expect("reentrant-number probe remains live");
    engine
        .call_object_function(reentrant_index, "Install", Vec::new())
        .expect("checker installs");
    assert_eq!(
        engine
            .call_object_function(
                reentrant_index,
                "AddReentrantDeniedThenAfter",
                Vec::new(),
            )
            .expect("reentrant denial completes"),
        Value::Int(405),
        "the pending outer node consumes #3 before Check, so its nested and subsequent adds get #4 and #5"
    );

    let start_denied = engine
        .spawn_object(SpawnConfig::new("FXCK"))
        .expect("Start-denial probe spawns");
    let start_denied_index = engine
        .find_object_index(start_denied)
        .expect("Start-denial probe remains live");
    engine
        .call_object_function(start_denied_index, "Install", Vec::new())
        .expect("checker installs");
    assert_eq!(
        engine
            .call_object_function(
                start_denied_index,
                "AddStartDeniedThenAfter",
                Vec::new(),
            )
            .expect("Start-denied AddEffect completes"),
        Value::Int(304),
        "the denied #3 remains linked through the script call, so the next add gets #4"
    );
    let start_denied = engine
        .object_snapshot(start_denied)
        .expect("Start-denial probe remains live");
    assert!(
        start_denied
            .effects
            .iter()
            .all(|effect| effect.name != "StartDenied"),
        "C4Fx_Start_Deny leaves no validated effect"
    );
    assert_eq!(
        start_denied.local_vars.get("iDeniedStops"),
        Some(&Value::Nil),
        "a Start-denied effect dies without Stop"
    );
}

#[test]
fn shipped_hazard_jumper_bite_check_uses_strict1_raw_string_identity() {
    let group = Group::open(content_root().join("Hazard.c4d/Enemies.c4d/Jumper.c4d"))
        .expect("shipped Hazard Jumper group opens");
    let resource = lc_resources::definition::Definition::load(&group)
        .expect("shipped Hazard Jumper definition loads");

    let mut engine = Engine::new();
    engine
        .register_definition(
            Definition::from_resource(&resource).expect("Hazard Jumper script compiles"),
        )
        .expect("Hazard Jumper registers");
    engine
        .register_definition(
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
            )
            .expect("effect driver compiles"),
        )
        .expect("effect driver registers");

    let jumper = engine
        .spawn_object(SpawnConfig::new("ALN2").with_loaded(true))
        .expect("loaded Hazard Jumper spawns without Initialize dependencies");
    let driver = engine
        .spawn_object(SpawnConfig::new("FXDR"))
        .expect("effect driver spawns");
    let driver_index = engine
        .find_object_index(driver)
        .expect("driver remains live");
    assert_eq!(
        engine
            .call_object_function(driver_index, "Probe", vec![Value::Object(jumper.as_u64())])
            .expect("two same-frame Bite additions complete"),
        Value::Int(2),
        "fresh FxBiteEffect name must not raw-equal its interned strict1 literal"
    );

    let mut bite_numbers = engine
        .object_snapshot(jumper)
        .expect("Hazard Jumper remains live")
        .effects
        .iter()
        .filter(|effect| effect.name == "Bite")
        .map(|effect| effect.number)
        .collect::<Vec<_>>();
    bite_numbers.sort_unstable();
    assert_eq!(bite_numbers, [1, 2]);
}
