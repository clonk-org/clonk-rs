use std::fs;
use std::path::Path;

use clonk_engine::{Definition, Engine, SpawnConfig};
use clonk_resources::{Group, ResourceDefinition};
use clonk_script::Value;
use tempfile::tempdir;

fn write_definition(path: &Path, id: &str, script: &str) {
    fs::create_dir_all(path).expect("definition directory creates");
    fs::write(
        path.join("DefCore.txt"),
        format!("[DefCore]\nid={id}\nName={id}\n"),
    )
    .expect("DefCore writes");
    fs::write(path.join("Script.c"), script).expect("Script writes");
}

fn engine_with(definitions: &[(&str, &str)]) -> Engine {
    let root = tempdir().expect("resource root creates");
    let mut engine = Engine::new();
    for (id, script) in definitions {
        let path = root.path().join(format!("{id}.c4d"));
        write_definition(&path, id, script);
        let resource = ResourceDefinition::load(&Group::open(&path).expect("group opens"))
            .expect("definition loads");
        engine
            .register_definition(Definition::from_resource(&resource).expect("definition compiles"))
            .expect("definition registers");
    }
    // The tempdir must outlive definition loading only; leak it so the engine
    // keeps working after this helper returns.
    std::mem::forget(root);
    engine
}

/// `C4AulParse::Parse_Params` keeps an argument's reference bytecode whenever
/// any same-named engine function declares `C4V_pC4Value` at that slot, and
/// the arrow-call path routes through the same `Parse_Params`
/// (oracle-src-pinned src/C4AulParse.cpp:3244-3250, :2318-2331). The callee
/// then aliases the caller's slot, so a `func(&x, &y)` writes back through an
/// `obj->~Fn(x, y)` call. Hazard's aiming and firing chain is built on this:
/// `UpdateVertices` calls `this->~WeaponAt(x, y, r)` and feeds the results
/// into `SetVertex`, and the weapon's `Shoot` does the same
/// (Hazard.c4d/Libraries.c4d/Functionalities.c4d/CanAim.c4d/Script.c:220-226;
/// Crew.c4d/HazardClonk.c4d/Script.c:930-940).
#[test]
fn arrow_call_arguments_alias_the_callee_reference_parameters() {
    let mut engine = engine_with(&[(
        "SELF",
        r#"#strict 2
func Probe()
{
  var x, y, plain;
  var result = this->~Fill(x, y, plain);
  return [result, x, y, plain];
}

func Fill(&a, &b, c)
{
  a = 7;
  b = "set";
  c = 99;
  return 1;
}
"#,
    )]);
    let object = engine
        .spawn_object(SpawnConfig::new("SELF").with_loaded(true))
        .expect("object spawns");
    let index = engine.find_object_index(object).expect("object exists");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", vec![])
            .expect("probe runs"),
        Value::Array(vec![
            Value::Int(1),
            Value::Int(7),
            Value::String("set".to_string().into()),
            // A plain parameter gets a dereferenced copy, so the caller's
            // variable stays nil (src/C4Value.cpp:586-597).
            Value::Nil,
        ])
    );
}

/// The same across two definitions: `AB_CALL` resolves the callee on the
/// target object, and `GetFirstFunc` made the caller push references before
/// the target was known (src/C4AulParse.cpp:3225).
#[test]
fn cross_object_arrow_call_writes_back_through_reference_parameters() {
    let mut engine = engine_with(&[
        (
            "CALR",
            r#"#strict 2
func Probe(object target)
{
  var x, y;
  target->~Fill(x, y);
  return [x, y];
}
"#,
        ),
        (
            "CALE",
            r#"#strict 2
func Fill(&a, &b)
{
  a = 3;
  b = 4;
}
"#,
        ),
    ]);
    let caller = engine
        .spawn_object(SpawnConfig::new("CALR").with_loaded(true))
        .expect("caller spawns");
    let callee = engine
        .spawn_object(SpawnConfig::new("CALE").with_loaded(true))
        .expect("callee spawns");
    let index = engine.find_object_index(caller).expect("caller exists");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", vec![Value::Object(callee.as_u64())])
            .expect("probe runs"),
        Value::Array(vec![Value::Int(3), Value::Int(4)])
    );
}
