use clonk_engine::{
    Definition, DefinitionComponent, Engine, EngineState, ObjectId, PlayerConfig, SpawnConfig,
};
use clonk_script::Value;
use std::collections::HashMap;

const SCRIPT: &str = r#"
#strict

func SeedAndRead()
{
  SetComponent(ZERO, 0);
  SetComponent(IROC, 3);
  SetComponent(ZERO, 2);
  return [GetComponent(0, 0), GetComponent(0, 1),
          GetComponent(ZERO), GetComponent(IROC)];
}

func SeedAndReadBoolIndexes()
{
  SetComponent(ZERO, 0);
  SetComponent(IROC, 3);
  return [GetComponent(0, false), GetComponent(0, true)];
}
"#;

fn engine_with_bag() -> (Engine, ObjectId) {
    let mut engine = Engine::new();
    engine
        .register_definition(Definition::from_script("BAG_", "Bag", SCRIPT).expect("compile"))
        .expect("register bag");
    let bag = engine
        .spawn_object(SpawnConfig::new("BAG_"))
        .expect("spawn bag");
    (engine, bag)
}

#[test]
fn dynamic_object_components_keep_cpp_insertion_order_and_zero_entries() {
    // C4IDList::SetIDCount(..., true) appends a missing ID even at count 0;
    // later count writes do not reorder it. FnGetComponent's indexed form
    // reads that runtime list, not DefCore order (C4IDList.cpp:38-45,85-103;
    // C4Script.cpp:2653-2709).
    let (mut engine, bag) = engine_with_bag();
    let bag_index = engine.find_object_index(bag).expect("bag index");
    assert_eq!(
        engine
            .call_object_function(bag_index, "SeedAndRead", Vec::new())
            .expect("component script runs"),
        Value::Array(vec![
            Value::C4Id("ZERO".to_string()),
            Value::C4Id("IROC".to_string()),
            Value::Int(2),
            Value::Int(3),
        ])
    );

    let snapshot = engine.object_snapshot(bag).expect("bag snapshot");
    assert_eq!(snapshot.component_order, ["ZERO", "IROC"]);
    assert_eq!(snapshot.components.get("ZERO"), Some(&2));
    assert_eq!(snapshot.components.get("IROC"), Some(&3));

    let state: EngineState = serde_json::from_str(
        &serde_json::to_string(&engine.capture_state()).expect("component state serializes"),
    )
    .expect("component state deserializes");
    let (mut restored, _) = engine_with_bag();
    restored.restore_state(&state).expect("restore component list");
    let restored = restored.object_snapshot(bag).expect("restored bag");
    assert_eq!(restored.component_order, ["ZERO", "IROC"]);
}

#[test]
fn optional_integer_builtin_parameters_coerce_bool_to_zero_or_one() {
    let (mut engine, bag) = engine_with_bag();
    let bag_index = engine.find_object_index(bag).expect("bag index");

    assert_eq!(
        engine
            .call_object_function(bag_index, "SeedAndReadBoolIndexes", Vec::new())
            .expect("bool values convert to optional integer indexes"),
        Value::Array(vec![
            Value::C4Id("ZERO".to_owned()),
            Value::C4Id("IROC".to_owned()),
        ])
    );
}

#[test]
fn definition_order_and_duplicate_component_slots_survive_restore() {
    // C4IDList is an ordered vector, not a map. DefCore and Objects.txt may
    // contain duplicate IDs; the shipped Bazooka has ENAP twice. CompileFunc
    // preserves both slots (C4IDList.cpp:239-260).
    let script = r#"
#strict
func ReadOrder()
{
  return [GetComponent(0, 0), GetComponent(0, 1), GetComponent(0, 2)];
}
"#;
    let mut engine = Engine::new();
    let mut definition =
        Definition::from_script("ORDR", "Ordered", script).expect("definition compiles");
    definition.set_components(vec![
        DefinitionComponent {
            id: "ZZZZ".to_owned(),
            count: 2,
        },
        DefinitionComponent {
            id: "AAAA".to_owned(),
            count: 1,
        },
        DefinitionComponent {
            id: "AAAA".to_owned(),
            count: 1,
        },
    ]);
    engine
        .register_definition(definition.clone())
        .expect("definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("ORDR").with_components(HashMap::from([
            ("AAAA".to_owned(), 1),
            ("ZZZZ".to_owned(), 2),
        ])))
        .expect("ordered object spawns");
    let index = engine.find_object_index(object).expect("object index");
    assert_eq!(
        engine
            .call_object_function(index, "ReadOrder", Vec::new())
            .expect("indexed component read succeeds"),
        Value::Array(vec![
            Value::C4Id("ZZZZ".to_owned()),
            Value::C4Id("AAAA".to_owned()),
            Value::C4Id("AAAA".to_owned()),
        ])
    );

    let state: EngineState = serde_json::from_str(
        &serde_json::to_string(&engine.capture_state()).expect("ordered state serializes"),
    )
    .expect("ordered state deserializes");
    let mut restored = Engine::new();
    restored
        .register_definition(definition)
        .expect("restore definition registers");
    restored.restore_state(&state).expect("ordered state restores");
    assert_eq!(
        restored
            .object_snapshot(object)
            .expect("restored object exists")
            .component_order,
        ["ZZZZ", "AAAA", "AAAA"]
    );
}

#[test]
fn get_needed_mat_str_matches_cpp_component_ledger_order_and_builder_context() {
    // C4Object::GetNeededMatStr subtracts the target's invested Component
    // list, never its Contents. A global explicit-target call passes the
    // caller as GetCustomComponents' builder; an arrow call executes the
    // engine host in the receiver context and therefore passes the target.
    const BUILDER_SCRIPT: &str = r#"
#strict 2
func Probe(static_site, dynamic_site, leading_site, empty_site, nonarray_site)
{
  var own_default = GetNeededMatStr();
  var own_nil = GetNeededMatStr(0);
  var static_missing = GetNeededMatStr(static_site);
  var dynamic_from_builder = GetNeededMatStr(dynamic_site);
  var dynamic_from_arrow = dynamic_site->GetNeededMatStr();
  var dynamic_without_builder = DYNA->NeededWithoutBuilder(dynamic_site);
  var leading_invalid = GetNeededMatStr(leading_site);
  var empty_override = GetNeededMatStr(empty_site);
  var nonarray_fallback = GetNeededMatStr(nonarray_site);
  SetComponent(METL, 2, dynamic_site);
  var dynamic_after_metal = GetNeededMatStr(dynamic_site);
  SetComponent(WOOD, 1, dynamic_site);
  var dynamic_complete = GetNeededMatStr(dynamic_site);
  SetComponent(ROCK, 1, dynamic_site);
  var arrow_complete = dynamic_site->GetNeededMatStr();
  return [own_default, own_nil, static_missing,
          dynamic_from_builder, dynamic_from_arrow, dynamic_without_builder,
          leading_invalid, empty_override, nonarray_fallback,
          dynamic_after_metal, dynamic_complete, arrow_complete];
}
func ProbeLocalized(static_site)
{
  return [GetNeededMatStr(), GetNeededMatStr(static_site)];
}
"#;
    const EMPTY_CUSTOM_SCRIPT: &str = r#"
#strict 2
protected func GetCustomComponents(builder) { return []; }
"#;
    const NONARRAY_CUSTOM_SCRIPT: &str = r#"
#strict 2
protected func GetCustomComponents(builder) { return 17; }
"#;
    const DYNAMIC_SCRIPT: &str = r#"
#strict 2
public func NeededWithoutBuilder(target)
{
  return GetNeededMatStr(target);
}
protected func GetCustomComponents(builder)
{
  if (!builder) return [WOOD, WOOD, WOOD];
  if (GetID(builder) == BULD) return [METL, METL, WOOD];
  return [ROCK];
}
"#;
    const LEADING_INVALID_SCRIPT: &str = r#"
#strict 2
local missing;
protected func GetCustomComponents(builder)
{
  return [missing, WOOD];
}
"#;

    let mut engine = Engine::new();
    for (id, name) in [
        ("ROCK", "Stein"),
        ("WOOD", "Bauholz"),
        ("METL", "Metall"),
        ("ZERO", "Nullstoff"),
        ("EMPT", ""),
    ] {
        engine
            .register_definition(Definition::from_script(id, name, "").expect("component compiles"))
            .expect("component registers");
    }
    engine
        .register_definition(
            Definition::from_script("BULD", "Builder", BUILDER_SCRIPT).expect("builder compiles"),
        )
        .expect("builder registers");
    let mut static_definition =
        Definition::from_script("SITE", "Construction Site", "").expect("site compiles");
    static_definition.set_components(vec![
        DefinitionComponent {
            id: "ROCK".to_owned(),
            count: 1,
        },
        DefinitionComponent {
            id: "WOOD".to_owned(),
            count: 3,
        },
        DefinitionComponent {
            id: "METL".to_owned(),
            count: 2,
        },
        DefinitionComponent {
            id: "ZERO".to_owned(),
            count: 0,
        },
        DefinitionComponent {
            id: "EMPT".to_owned(),
            count: 1,
        },
        DefinitionComponent {
            id: "MISS".to_owned(),
            count: 1,
        },
    ]);
    engine
        .register_definition(static_definition)
        .expect("site registers");
    engine
        .register_definition(
            Definition::from_script("DYNA", "Dynamic", DYNAMIC_SCRIPT)
                .expect("dynamic site compiles"),
        )
        .expect("dynamic site registers");
    engine
        .register_definition(
            Definition::from_script("LEAD", "Leading", LEADING_INVALID_SCRIPT)
                .expect("leading-invalid site compiles"),
        )
        .expect("leading-invalid site registers");
    let mut empty_custom =
        Definition::from_script("CEMP", "Empty custom", EMPTY_CUSTOM_SCRIPT)
            .expect("empty-custom site compiles");
    empty_custom.set_components(vec![DefinitionComponent {
        id: "WOOD".to_owned(),
        count: 2,
    }]);
    engine
        .register_definition(empty_custom)
        .expect("empty-custom site registers");
    let mut nonarray_custom =
        Definition::from_script("NARR", "Non-array", NONARRAY_CUSTOM_SCRIPT)
            .expect("non-array site compiles");
    nonarray_custom.set_components(vec![DefinitionComponent {
        id: "ROCK".to_owned(),
        count: 2,
    }]);
    engine
        .register_definition(nonarray_custom)
        .expect("non-array site registers");

    let builder = engine
        .spawn_object(SpawnConfig::new("BULD"))
        .expect("builder spawns");
    let static_site = engine
        .spawn_object(
            SpawnConfig::new("SITE")
                .with_custom_name("Nordwerk")
                .with_ordered_components(vec![
                    ("ROCK".to_owned(), 0),
                    ("WOOD".to_owned(), 1),
                    ("METL".to_owned(), 2),
                    ("ZERO".to_owned(), 0),
                    ("EMPT".to_owned(), 0),
                    ("MISS".to_owned(), 0),
                ]),
        )
        .expect("static site spawns");
    // These are exactly the missing pieces, but C++ deliberately ignores
    // Contents here and still reports the Component-ledger deficits.
    for definition in ["ROCK", "WOOD", "WOOD"] {
        engine
            .spawn_object(SpawnConfig::new(definition).with_container(static_site))
            .expect("site content spawns");
    }
    assert_eq!(
        engine
            .object_snapshot(static_site)
            .expect("static site exists")
            .contents
            .len(),
        3
    );
    let dynamic_site = engine
        .spawn_object(SpawnConfig::new("DYNA").with_ordered_components(vec![
            ("METL".to_owned(), 0),
            ("WOOD".to_owned(), 0),
            ("ROCK".to_owned(), 0),
        ]))
        .expect("dynamic site spawns");
    let leading_site = engine
        .spawn_object(SpawnConfig::new("LEAD"))
        .expect("leading-invalid site spawns");
    let empty_site = engine
        .spawn_object(
            SpawnConfig::new("CEMP")
                .with_ordered_components(vec![("WOOD".to_owned(), 0)]),
        )
        .expect("empty-custom site spawns");
    let nonarray_site = engine
        .spawn_object(
            SpawnConfig::new("NARR")
                .with_ordered_components(vec![("ROCK".to_owned(), 0)]),
        )
        .expect("non-array site spawns");

    let builder_index = engine.find_object_index(builder).expect("builder index");
    assert_eq!(
        engine
            .call_object_function(
                builder_index,
                "Probe",
                vec![
                    Value::Object(static_site.as_u64()),
                    Value::Object(dynamic_site.as_u64()),
                    Value::Object(leading_site.as_u64()),
                    Value::Object(empty_site.as_u64()),
                    Value::Object(nonarray_site.as_u64()),
                ],
            )
            .expect("needed-material probe runs"),
        Value::Array(vec![
            Value::String("Builder needs|no more material.".to_owned().into()),
            Value::String("Builder needs|no more material.".to_owned().into()),
            Value::String("Nordwerk|needs|1x Stein|2x Bauholz|1x |1x MISS".to_owned().into()),
            Value::String("Dynamic|needs|2x Metall|1x Bauholz".to_owned().into()),
            Value::String("Dynamic|needs|1x Stein".to_owned().into()),
            Value::String("Dynamic|needs|3x Bauholz".to_owned().into()),
            Value::String("Leading needs|no more material.".to_owned().into()),
            Value::String("Empty custom needs|no more material.".to_owned().into()),
            Value::String("Non-array|needs|2x Stein".to_owned().into()),
            Value::String("Dynamic|needs|1x Bauholz".to_owned().into()),
            Value::String("Dynamic needs|no more material.".to_owned().into()),
            Value::String("Dynamic needs|no more material.".to_owned().into()),
        ])
    );

    engine.set_needed_material_resource_strings(
        "%s|braucht noch",
        "%s braucht kein|weiteres Baumaterial.",
    );
    assert_eq!(
        engine
            .call_object_function(
                builder_index,
                "ProbeLocalized",
                vec![Value::Object(static_site.as_u64())],
            )
            .expect("localized needed-material probe runs"),
        Value::Array(vec![
            Value::String("Builder braucht kein|weiteres Baumaterial.".to_owned().into()),
            Value::String(
                "Nordwerk|braucht noch|1x Stein|2x Bauholz|1x |1x MISS"
                    .to_owned()
                    .into(),
            ),
        ])
    );
}

#[test]
fn signed_component_counts_drive_get_needed_mat_str_and_survive_restore() {
    // C4IDList count storage is signed. SetComponent writes verbatim, and
    // GetNeededMatStr subtracts the live signed ledger from every nonzero
    // definition requirement (C4Script.cpp:2659-2663;
    // C4Object.cpp:6234-6265).
    const SIGNED_SCRIPT: &str = r#"
#strict 2
func SeedAndRead()
{
  var all = ComponentAll(this(), POSI);
  SetComponent(POSI, -2);
  SetComponent(NEGA, -5);
  SetComponent(ZERO, -7);
  return [all, GetComponent(POSI), GetComponent(NEGA), GetComponent(ZERO), GetNeededMatStr()];
}
func Read()
{
  return [GetComponent(POSI), GetComponent(NEGA), GetComponent(ZERO), GetNeededMatStr()];
}
"#;

    let positive = Definition::from_script("POSI", "Positive", "").expect("positive compiles");
    let negative = Definition::from_script("NEGA", "Negative", "").expect("negative compiles");
    let zero = Definition::from_script("ZERO", "Zero", "").expect("zero compiles");
    let mut signed =
        Definition::from_script("SIGN", "Signed", SIGNED_SCRIPT).expect("signed compiles");
    signed.set_components(vec![
        DefinitionComponent {
            id: "POSI".to_owned(),
            count: 1,
        },
        DefinitionComponent {
            id: "NEGA".to_owned(),
            count: -2,
        },
        DefinitionComponent {
            id: "ZERO".to_owned(),
            count: 0,
        },
    ]);

    let mut engine = Engine::new();
    for definition in [&positive, &negative, &zero, &signed] {
        engine
            .register_definition(definition.clone())
            .expect("definition registers");
    }
    let object = engine
        .spawn_object(SpawnConfig::new("SIGN"))
        .expect("signed object spawns");
    let index = engine.find_object_index(object).expect("signed index");
    assert_eq!(
        engine
            .call_object_function(index, "SeedAndRead", Vec::new())
            .expect("signed probe runs"),
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(-2),
            Value::Int(-5),
            Value::Int(-7),
            Value::String("Signed|needs|3x Positive|3x Negative".to_owned().into()),
        ])
    );
    let snapshot = engine.object_snapshot(object).expect("signed snapshot");
    assert_eq!(snapshot.components.get("POSI"), Some(&-2));
    assert_eq!(snapshot.components.get("NEGA"), Some(&-5));
    assert_eq!(snapshot.components.get("ZERO"), Some(&-7));

    let state: EngineState = serde_json::from_str(
        &serde_json::to_string(&engine.capture_state()).expect("signed state serializes"),
    )
    .expect("signed state deserializes");
    let mut restored = Engine::new();
    for definition in [positive, negative, zero, signed] {
        restored
            .register_definition(definition)
            .expect("restore definition registers");
    }
    restored
        .restore_state(&state)
        .expect("signed state restores");
    let index = restored.find_object_index(object).expect("restored index");
    assert_eq!(
        restored
            .call_object_function(index, "Read", Vec::new())
            .expect("restored signed probe runs"),
        Value::Array(vec![
            Value::Int(-2),
            Value::Int(-5),
            Value::Int(-7),
            Value::String("Signed|needs|3x Positive|3x Negative".to_owned().into()),
        ])
    );
}

#[test]
fn get_needed_mat_str_without_explicit_or_current_object_is_nil() {
    let mut script = clonk_script::Engine::new();
    clonk_engine::compat::register_host_functions(&mut script);
    script
        .load_script("#strict 2\nfunc Probe(target) { return GetNeededMatStr(target); }")
        .expect("probe compiles");
    assert_eq!(script.call("Probe", &[]).expect("probe runs"), Value::Nil);
    let error = script
        .call("Probe", &[Value::Proplist(Default::default())])
        .expect_err("a map cannot convert to C4Object");
    assert!(error.to_string().contains(
        "call to \"GetNeededMatStr\" parameter 1: got \"map\", but expected \"object\"!"
    ));
}

#[test]
fn get_needed_mat_str_rechecks_the_post_callback_object_name_like_cpp() {
    // AssignRemoval clears C4Object::Info but preserves CustomName/Def. A
    // GetCustomComponents callback can remove its builder while the native
    // GetNeededMatStr frame is still running; the final GetName must then
    // fall back from the retired crew name to the definition name.
    const SCRIPT: &str = r#"
#strict 2
func JoinRemoveAndRead()
{
  MakeCrewMember(this(), 0);
  return GetNeededMatStr();
}
protected func GetCustomComponents(builder)
{
  RemoveObject(builder);
  return [ROCK];
}
"#;
    let mut engine = Engine::new();
    engine
        .register_player(PlayerConfig::new(0, "Player"))
        .expect("player registers");
    engine.set_standard_names(Some("Roster Name\n".to_owned()));
    engine
        .register_definition(Definition::from_script("ROCK", "Stein", "").expect("rock compiles"))
        .expect("rock registers");
    let mut crew = Definition::from_script("CREW", "Crew", SCRIPT).expect("crew compiles");
    crew.set_crew_member(true);
    engine.register_definition(crew).expect("crew registers");
    let crew = engine
        .spawn_object(
            SpawnConfig::new("CREW")
                .with_owner(0)
                .with_crew_member(false),
        )
        .expect("crew spawns");
    let index = engine.find_object_index(crew).expect("crew index");
    assert_eq!(
        engine
            .call_object_function(index, "JoinRemoveAndRead", Vec::new())
            .expect("removal-time query runs"),
        Value::String("Crew|needs|1x Stein".to_owned().into())
    );
}
