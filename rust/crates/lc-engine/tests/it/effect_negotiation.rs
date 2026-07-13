use lc_engine::{Definition, Engine, SpawnConfig};
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
local iChecks;
local iDenyExact;
local iPassExact;
local iStarts;

func Install()
{
  iChecks = iDenyExact = iPassExact = iStarts = 0;
  iChecker = AddEffect("Guard", this(), 200, 0, this());
  return(iChecker);
}

func AddDenied()
{
  return(AddEffect("Denied", this(), 100, 7, this(), nil, 11, 12, 13, 14));
}

func AddAllowed()
{
  return(AddEffect("Allowed", this(), 100, 9, this(), nil, 21, 22, 23, 24));
}

func FxGuardEffect(string szNew, object pTarget, int iNumber, int iNewNumber,
                   int iVal1, int iVal2, int iVal3, int iVal4)
{
  ++iChecks;
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

func FxAllowedStart() { ++iStarts; }
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
        vec!["Guard"],
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
        allowed.local_vars.get("iStarts"),
        Some(&Value::Int(1)),
        "the passing effect still receives exactly one Start callback"
    );
}
