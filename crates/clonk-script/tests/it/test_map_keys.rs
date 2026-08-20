//! C4Script maps use full `C4Value` keys. Computed literal keys are written
//! as `[expression]`, and C4ValueHash keeps the insertion position of the
//! first equal key when a later literal entry overwrites its value.

use clonk_script::{clear_active_object_references, Engine, Value, ValueMap};

fn run(source: &str, args: &[Value]) -> Value {
    crate::support::run(source, "Test", args)
}

#[test]
fn computed_map_keys_round_trip_through_literals_and_indexed_assignment() {
    let source = r#"
        #strict 3
        func Test(object obj) {
            var literal = {
                [42] = "literal-int",
                [CLNK] = "literal-id",
                [obj] = "literal-object",
                [true] = "literal-bool",
                ["name"] = "literal-string",
            };

            var assigned = {};
            assigned[42] = "assigned-int";
            assigned[CLNK] = "assigned-id";
            assigned[obj] = "assigned-object";
            assigned[true] = "assigned-bool";
            assigned["name"] = "assigned-string";

            return literal[42] == "literal-int"
                && literal[CLNK] == "literal-id"
                && literal[obj] == "literal-object"
                && literal[true] == "literal-bool"
                && literal["name"] == "literal-string"
                && assigned[42] == "assigned-int"
                && assigned[CLNK] == "assigned-id"
                && assigned[obj] == "assigned-object"
                && assigned[true] == "assigned-bool"
                && assigned["name"] == "assigned-string";
        }
    "#;

    assert_eq!(run(source, &[Value::Object(77)]), Value::Bool(true));
}

#[test]
fn host_returned_id_and_object_keys_round_trip_without_missing_read_insertion() {
    let mut engine = Engine::new();
    engine.register_host_function("GetID", |_| Ok(Value::C4Id("CLNK".into())));
    engine
        .load_script(
            r#"
            #strict 3
            func Test(object obj) {
                var entries = { [GetID()] = 1, [obj] = 2, ["5"] = 3, [5] = 4 };
                entries[GetID()] = 11;
                var missing = entries[999];
                var count = 0;
                for (var key, value in entries) count += 1;
                return [entries[GetID()], entries[obj], entries["5"], entries[5], missing, count];
            }
            "#,
        )
        .expect("script should load");

    assert_eq!(
        engine
            .call("Test", &[Value::Object(77)])
            .expect("typed map keys should run"),
        Value::Array(vec![
            Value::Int(11),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
            Value::Nil,
            Value::Int(4),
        ])
    );
}

#[test]
fn maxstrict_map_key_equality_keeps_value_types_distinct() {
    let source = r#"
        #strict 3
        func Test(object obj) {
            var entries = {
                [1] = 11,
                [true] = 12,
                ["1"] = 13,
                [CLNK] = 14,
                [obj] = 15,
                [nil] = 16,
                [false] = 17,
                [0] = 18,
            };
            var count = 0;
            for (var key, value in entries) count += 1;

            return count == 8
                && entries[1] == 11
                && entries[true] == 12
                && entries["1"] == 13
                && entries[CLNK] == 14
                && entries[obj] == 15
                && entries[nil] == 16
                && entries[false] == 17
                && entries[0] == 18;
        }
    "#;

    assert_eq!(run(source, &[Value::Object(77)]), Value::Bool(true));
}

#[test]
fn duplicate_computed_key_overwrites_without_moving_its_first_position() {
    let source = r#"
        #strict 3
        func Test() {
            var entries = { [1] = 1, [2] = 2, [1] = 3, [true] = 4 };
            var position = 0;
            var fingerprint = 0;

            for (var key, value in entries) {
                if (position == 0 && (key != 1 || value != 3)) return -1;
                if (position == 1 && (key != 2 || value != 2)) return -2;
                if (position == 2 && (key != true || value != 4)) return -3;
                fingerprint = fingerprint * 10 + value;
                position += 1;
            }
            if (position != 3) return -4;
            return fingerprint;
        }
    "#;

    assert_eq!(run(source, &[]), Value::Int(324));
}

#[test]
fn nil_overwrite_removes_an_existing_literal_entry_before_reinsertion() {
    let source = r#"
        #strict 3
        func Test() {
            var entries = { [1] = 1, [2] = 2, [1] = nil, [1] = 3, [4] = nil };
            var fingerprint = 0;
            for (var key, value in entries) {
                fingerprint = fingerprint * 100 + key * 10;
                if (value != nil) fingerprint += value;
            }
            return fingerprint;
        }
    "#;

    // The nonnil value at key 1 is erased, so its later reinsertion follows
    // key 2. A first insertion whose value is already nil remains present.
    assert_eq!(run(source, &[]), Value::Int(221_340));
}

#[test]
fn computed_literal_evaluates_key_then_value_in_source_order() {
    let source = r#"
        #strict 3
        func Next(&counter) {
            counter += 1;
            return counter;
        }

        func Test() {
            var counter = 0;
            var entries = {
                [Next(counter)] = Next(counter),
                [Next(counter)] = Next(counter),
            };
            var fingerprint = 0;
            for (var key, value in entries) {
                fingerprint = fingerprint * 100 + key * 10 + value;
            }
            return counter * 10000 + fingerprint;
        }
    "#;

    // Evaluation is key 1, value 2, key 3, value 4; foreach observes the
    // same insertion order and therefore produces the decimal suffix 1234.
    assert_eq!(run(source, &[]), Value::Int(41_234));
}

#[test]
fn assign_removal_keeps_a_nested_key_in_its_insertion_bucket() {
    // Nested object keys stay in their insertion hash after Set0. Native
    // C4ValueHash::operator== is an asymmetric lookup, so a later map keyed
    // by [nil] finds the stale node one way only (C4ValueHash.cpp:150-181).
    let mut engine = Engine::new();
    engine.register_host_function("Clear", |_| {
        clear_active_object_references(7);
        Ok(Value::Nil)
    });
    engine
        .load_script(
            r#"#strict 3
func Probe(object target) {
  var stale = { [[target]] = 1 };
  Clear();
  var fresh = { [[nil]] = 1 };
  return [fresh == stale, stale == fresh];
}
"#,
        )
        .expect("nested key bucket probe parses");

    assert_eq!(
        engine
            .call("Probe", &[Value::Object(7)])
            .expect("nested key bucket runs"),
        Value::Array(vec![Value::Bool(false), Value::Bool(true)])
    );
}

/// Host functions for the AssignRemoval sweep probes below. Each reports the
/// map's first key through a different accessor so a divergence between them
/// is visible from script.
fn sweep_probe_engine() -> Engine {
    let mut engine = Engine::new();
    engine.register_host_function("Clear7", |_| {
        clear_active_object_references(7);
        Ok(Value::Nil)
    });
    engine.register_host_function("Clear9", |_| {
        clear_active_object_references(9);
        Ok(Value::Nil)
    });
    engine.register_host_function("Nine", |_| Ok(Value::Object(9)));
    engine.register_host_function("Fresh7", |_| Ok(Value::Object(7)));
    engine.register_host_function("Len", |args| {
        let Some(Value::Proplist(map)) = args.first() else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(map.len() as i32))
    });
    engine.register_host_function("KeyViaIter", |args| {
        let Some(Value::Proplist(map)) = args.first() else {
            return Ok(Value::Nil);
        };
        Ok(map
            .iter()
            .next()
            .map(|(key, _)| key.clone())
            .unwrap_or(Value::Nil))
    });
    engine.register_host_function("KeyViaIntoIter", |args| {
        let Some(Value::Proplist(map)) = args.first().cloned() else {
            return Ok(Value::Nil);
        };
        Ok(map
            .into_iter()
            .next()
            .map(|(key, _)| key)
            .unwrap_or(Value::Nil))
    });
    engine.register_host_function("KeyAfterSaveLoad", |args| {
        let Some(Value::Proplist(map)) = args.first() else {
            return Ok(Value::Nil);
        };
        let encoded = serde_json::to_value(map).expect("a map serializes");
        let restored: ValueMap = serde_json::from_value(encoded).expect("a map deserializes");
        let first = restored.iter().next().map(|(key, _)| key.clone());
        Ok(first.unwrap_or(Value::Nil))
    });
    engine.register_host_function("HiddenValues", |args| {
        let Some(Value::Proplist(map)) = args.first() else {
            return Ok(Value::Nil);
        };
        Ok(Value::Array(map.hidden_values().cloned().collect()))
    });
    engine
}

fn run_sweep_probe(source: &str) -> Value {
    let mut engine = sweep_probe_engine();
    engine.load_script(source).expect("script should load");
    engine
        .call("Test", &[Value::Object(7)])
        .expect("script should run")
}

#[test]
fn assign_removal_shows_the_swept_key_through_every_accessor() {
    // Set0 rewrites the one key C4Value the node owns (C4Object.cpp:312), so
    // there is no second spelling of that key left for iteration, a move out
    // of the map, or a save/load round trip to disagree about.
    assert_eq!(
        run_sweep_probe(
            r#"#strict 3
func Test(object target) {
  var map = { [[target]] = 1 };
  Clear7();
  return [KeyViaIter(map), KeyViaIntoIter(map), KeyAfterSaveLoad(map)];
}
"#
        ),
        Value::Array(vec![
            Value::Array(vec![Value::Nil]),
            Value::Array(vec![Value::Nil]),
            Value::Array(vec![Value::Nil]),
        ])
    );
}

#[test]
fn a_later_unrelated_sweep_keeps_an_already_cleared_key_nil() {
    // The second sweep must mutate the key in place again, never rebuild it
    // from an insertion-time copy: object 7 is gone and cannot come back.
    assert_eq!(
        run_sweep_probe(
            r#"#strict 3
func Test(object target) {
  var map = { [[target]] = 1, other = Nine() };
  Clear7();
  Clear9();
  return KeyViaIter(map);
}
"#
        ),
        Value::Array(vec![Value::Nil])
    );
}

#[test]
fn reinserting_a_swept_keys_original_shape_adds_a_second_node() {
    // The stale node stays in the bucket its live object hashed to, so the
    // fresh key misses it and C4ValueHash allocates a second node
    // (C4ValueHash.cpp:49-136).
    assert_eq!(
        run_sweep_probe(
            r#"#strict 3
func Test(object target) {
  var map = { [[target]] = 1 };
  Clear7();
  map[[Fresh7()]] = 2;
  return Len(map);
}
"#
        ),
        Value::Int(2)
    );
}

#[test]
fn assign_removal_clears_a_retained_mapped_slot() {
    // A slot retained in emptyValues is still a registered C4Value
    // (C4Value.cpp:78-99), so the FirstRef sweep reaches it there too.
    assert_eq!(
        run_sweep_probe(
            r#"#strict 3
func Test(object target) {
  var map = {};
  map[target] = [Nine()];
  Clear7();
  Clear9();
  return HiddenValues(map);
}
"#
        ),
        Value::Array(vec![Value::Array(vec![Value::Nil])])
    );
}
