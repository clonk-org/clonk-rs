use crate::support::EngineTestExt;
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
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script("BAG_", "Bag", SCRIPT),
    ));
    let bag = engine.spawn_test_object(SpawnConfig::new("BAG_"));
    (engine, bag)
}

#[test]
fn dynamic_object_components_keep_cpp_insertion_order_and_zero_entries() {
    // C4IDList::SetIDCount(..., true) appends a missing ID even at count 0;
    // later count writes do not reorder it. FnGetComponent's indexed form
    // reads that runtime list, not DefCore order (C4IDList.cpp:38-45,85-103;
    // C4Script.cpp:2653-2709).
    let (mut engine, bag) = engine_with_bag();
    let bag_index = engine.test_object_index(bag);
    assert_eq!(
        engine.call_test_object_function(bag_index, "SeedAndRead", Vec::new()),
        Value::Array(vec![
            Value::C4Id("ZERO".to_string()),
            Value::C4Id("IROC".to_string()),
            Value::Int(2),
            Value::Int(3),
        ])
    );

    let snapshot = engine.test_object_snapshot(bag);
    assert_eq!(snapshot.component_order, ["ZERO", "IROC"]);
    assert_eq!(snapshot.components.get("ZERO"), Some(&2));
    assert_eq!(snapshot.components.get("IROC"), Some(&3));

    let state: EngineState = crate::support::TestValueExt::test_value(serde_json::from_str(
        &crate::support::TestValueExt::test_value(serde_json::to_string(&engine.capture_state())),
    ));
    let (mut restored, _) = engine_with_bag();
    crate::support::TestValueExt::test_value(restored.restore_state(&state));
    let restored = restored.test_object_snapshot(bag);
    assert_eq!(restored.component_order, ["ZERO", "IROC"]);
}

#[test]
fn optional_integer_builtin_parameters_coerce_bool_to_zero_or_one() {
    let (mut engine, bag) = engine_with_bag();
    let bag_index = engine.test_object_index(bag);

    assert_eq!(
        engine.call_test_object_function(bag_index, "SeedAndReadBoolIndexes", Vec::new()),
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
    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "ORDR", "Ordered", script,
    ));
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
    engine.register_test_definition(definition.clone());
    let object =
        engine.spawn_test_object(SpawnConfig::new("ORDR").with_components(HashMap::from([
            ("AAAA".to_owned(), 1),
            ("ZZZZ".to_owned(), 2),
        ])));
    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "ReadOrder", Vec::new()),
        Value::Array(vec![
            Value::C4Id("ZZZZ".to_owned()),
            Value::C4Id("AAAA".to_owned()),
            Value::C4Id("AAAA".to_owned()),
        ])
    );

    let state: EngineState = crate::support::TestValueExt::test_value(serde_json::from_str(
        &crate::support::TestValueExt::test_value(serde_json::to_string(&engine.capture_state())),
    ));
    let mut restored = Engine::new();
    restored.register_test_definition(definition);
    crate::support::TestValueExt::test_value(restored.restore_state(&state));
    assert_eq!(
        restored.test_object_snapshot(object).component_order,
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
        engine.register_test_definition(crate::support::TestValueExt::test_value(
            Definition::from_script(id, name, ""),
        ));
    }
    engine.register_test_script_definition("BULD", "Builder", BUILDER_SCRIPT);
    let mut static_definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "SITE",
        "Construction Site",
        "",
    ));
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
    engine.register_test_definition(static_definition);
    engine.register_test_script_definition("DYNA", "Dynamic", DYNAMIC_SCRIPT);
    engine.register_test_script_definition("LEAD", "Leading", LEADING_INVALID_SCRIPT);
    let mut empty_custom = crate::support::TestValueExt::test_value(Definition::from_script(
        "CEMP",
        "Empty custom",
        EMPTY_CUSTOM_SCRIPT,
    ));
    empty_custom.set_components(vec![DefinitionComponent {
        id: "WOOD".to_owned(),
        count: 2,
    }]);
    engine.register_test_definition(empty_custom);
    let mut nonarray_custom = crate::support::TestValueExt::test_value(Definition::from_script(
        "NARR",
        "Non-array",
        NONARRAY_CUSTOM_SCRIPT,
    ));
    nonarray_custom.set_components(vec![DefinitionComponent {
        id: "ROCK".to_owned(),
        count: 2,
    }]);
    engine.register_test_definition(nonarray_custom);

    let builder = engine.spawn_test_object(SpawnConfig::new("BULD"));
    let static_site = engine.spawn_test_object(
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
    );
    // These are exactly the missing pieces, but C++ deliberately ignores
    // Contents here and still reports the Component-ledger deficits.
    for definition in ["ROCK", "WOOD", "WOOD"] {
        engine.spawn_test_object(SpawnConfig::new(definition).with_container(static_site));
    }
    assert_eq!(engine.test_object_snapshot(static_site).contents.len(), 3);
    let dynamic_site =
        engine.spawn_test_object(SpawnConfig::new("DYNA").with_ordered_components(vec![
            ("METL".to_owned(), 0),
            ("WOOD".to_owned(), 0),
            ("ROCK".to_owned(), 0),
        ]));
    let leading_site = engine.spawn_test_object(SpawnConfig::new("LEAD"));
    let empty_site = engine.spawn_test_object(
        SpawnConfig::new("CEMP").with_ordered_components(vec![("WOOD".to_owned(), 0)]),
    );
    let nonarray_site = engine.spawn_test_object(
        SpawnConfig::new("NARR").with_ordered_components(vec![("ROCK".to_owned(), 0)]),
    );

    let builder_index = engine.test_object_index(builder);
    assert_eq!(
        engine.call_test_object_function(
            builder_index,
            "Probe",
            vec![
                Value::Object(static_site.as_u64()),
                Value::Object(dynamic_site.as_u64()),
                Value::Object(leading_site.as_u64()),
                Value::Object(empty_site.as_u64()),
                Value::Object(nonarray_site.as_u64()),
            ],
        ),
        Value::Array(vec![
            Value::String("Builder needs|no more material.".to_owned().into()),
            Value::String("Builder needs|no more material.".to_owned().into()),
            Value::String(
                "Nordwerk|needs|1x Stein|2x Bauholz|1x |1x MISS"
                    .to_owned()
                    .into()
            ),
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
        engine.call_test_object_function(
            builder_index,
            "ProbeLocalized",
            vec![Value::Object(static_site.as_u64())],
        ),
        Value::Array(vec![
            Value::String(
                "Builder braucht kein|weiteres Baumaterial."
                    .to_owned()
                    .into()
            ),
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

    let positive =
        crate::support::TestValueExt::test_value(Definition::from_script("POSI", "Positive", ""));
    let negative =
        crate::support::TestValueExt::test_value(Definition::from_script("NEGA", "Negative", ""));
    let zero =
        crate::support::TestValueExt::test_value(Definition::from_script("ZERO", "Zero", ""));
    let mut signed = crate::support::TestValueExt::test_value(Definition::from_script(
        "SIGN",
        "Signed",
        SIGNED_SCRIPT,
    ));
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
        engine.register_test_definition(definition.clone());
    }
    let object = engine.spawn_test_object(SpawnConfig::new("SIGN"));
    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "SeedAndRead", Vec::new()),
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(-2),
            Value::Int(-5),
            Value::Int(-7),
            Value::String("Signed|needs|3x Positive|3x Negative".to_owned().into()),
        ])
    );
    let snapshot = engine.test_object_snapshot(object);
    assert_eq!(snapshot.components.get("POSI"), Some(&-2));
    assert_eq!(snapshot.components.get("NEGA"), Some(&-5));
    assert_eq!(snapshot.components.get("ZERO"), Some(&-7));

    let state: EngineState = crate::support::TestValueExt::test_value(serde_json::from_str(
        &crate::support::TestValueExt::test_value(serde_json::to_string(&engine.capture_state())),
    ));
    let mut restored = Engine::new();
    for definition in [positive, negative, zero, signed] {
        restored.register_test_definition(definition);
    }
    crate::support::TestValueExt::test_value(restored.restore_state(&state));
    let index = restored.test_object_index(object);
    assert_eq!(
        restored.call_test_object_function(index, "Read", Vec::new()),
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
    crate::support::TestValueExt::test_value(
        script.load_script("#strict 2\nfunc Probe(target) { return GetNeededMatStr(target); }"),
    );
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
    engine.register_test_player(PlayerConfig::new(0, "Player"));
    engine.set_standard_names(Some("Roster Name\n".to_owned()));
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script("ROCK", "Stein", ""),
    ));
    let mut crew =
        crate::support::TestValueExt::test_value(Definition::from_script("CREW", "Crew", SCRIPT));
    crew.set_crew_member(true);
    engine.register_test_definition(crew);
    let crew = engine.spawn_test_object(
        SpawnConfig::new("CREW")
            .with_owner(0)
            .with_crew_member(false),
    );
    let index = engine.test_object_index(crew);
    assert_eq!(
        engine.call_test_object_function(index, "JoinRemoveAndRead", Vec::new()),
        Value::String("Crew|needs|1x Stein".to_owned().into())
    );
}
