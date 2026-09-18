//! Allocation budget for warm script callbacks.
use clonk_engine::{Engine, SpawnConfig};
use clonk_script::Value;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// An odd array length separates its Value storage from VM identity tables.
const LOCAL_ARRAY_LEN: usize = 257;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static LOCAL_ARRAY_COPIES: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record_allocation(size: usize) {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
            if size == LOCAL_ARRAY_LEN * std::mem::size_of::<Value>() {
                LOCAL_ARRAY_COPIES.with(|copies| copies.set(copies.get() + 1));
            }
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        // SAFETY: forwards the caller's allocation contract unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        // SAFETY: forwards the caller's allocation contract unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation(size);
        // SAFETY: pointer and layout belong to the wrapped allocator.
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout belong to the wrapped allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[test]
fn warm_object_callback_avoids_discarded_world_defaults() {
    let mut engine = Engine::new();
    engine
        .register_script_definition("TEST", "Test", "#strict 2\nfunc Probe() { return 42; }")
        .unwrap();
    engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
    engine.call_object_function(0, "Probe", Vec::new()).unwrap();
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let result = engine.call_object_function(0, "Probe", Vec::new());
    let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(result.unwrap(), Value::Int(42));
    eprintln!("warm callback allocations: {allocations}");
    assert!(allocations <= 64, "warm callback allocated {allocations} times; shared engine resources must not get throwaway defaults");
}

#[test]
fn callback_that_ignores_global_effects_does_not_copy_their_values() {
    // C4AulExec::Exec retains the game/effect list; entering a callback does
    // not copy its C4Values (C4AulExec.cpp:330-359; C4Effect.cpp:450-455).
    let mut engine = Engine::new();
    engine
        .register_script_definition("TEST", "Test", "#strict 2\nfunc Probe() { return 42; }")
        .unwrap();
    engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
    let measure = |engine: &mut Engine| {
        engine.call_object_function(0, "Probe", Vec::new()).unwrap();
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = engine.call_object_function(0, "Probe", Vec::new());
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(result.unwrap(), Value::Int(42));
        allocations
    };
    let empty = measure(&mut engine);
    let mut state = engine.capture_state();
    state.global_effects = (0..64)
        .map(|index| {
            clonk_engine::effect::EffectState::new(format!("Untouched{index}")).with_vars(vec![
                clonk_engine::effect::EffectVarValue::Array(vec![
                    clonk_engine::effect::EffectVarValue::Int(index),
                ]),
            ])
        })
        .collect();
    engine.restore_state(&state).unwrap();
    let populated = measure(&mut engine);
    assert_eq!(engine.global_effects(), state.global_effects);
    assert!(
        populated <= empty + 2,
        "callback allocations grew from {empty} to {populated} for untouched global effects"
    );
}

#[test]
fn global_effect_timer_allocations_scale_with_callbacks_not_list_copies() {
    // C4Effect::Execute advances each node and invokes its callback without
    // copying the remaining list (C4Effect.cpp:319-363).
    let measure = |count: i32| {
        let mut engine = Engine::new();
        engine
            .register_script_definition(
                "TEST",
                "Test",
                "#strict 2\nglobal func FxUntouchedTimer(target, number, time) { return 1; }",
            )
            .unwrap();
        let mut state = engine.capture_state();
        state.global_effects = (0..count)
            .map(|index| {
                let mut effect = clonk_engine::EffectState::new("Untouched")
                    .with_interval(1)
                    .with_vars(vec![clonk_engine::EffectVarValue::Array(vec![
                        clonk_engine::EffectVarValue::Int(index),
                    ])]);
                effect.number = index + 1;
                effect
            })
            .collect();
        engine.restore_state(&state).unwrap();
        engine.tick_without_snapshot().unwrap();
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = engine.tick_without_snapshot();
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        result.unwrap();
        assert_eq!(engine.global_effects().len(), count as usize);
        assert!(engine
            .global_effects()
            .iter()
            .all(|effect| effect.timer == 2));
        allocations
    };
    let small = measure(8);
    let large = measure(64);
    eprintln!("global timer allocations: 8 effects={small}, 64 effects={large}");
    assert!(
        large <= small * 9,
        "effect timer allocation growth must stay linear"
    );
}

#[test]
fn effect_var_read_does_not_copy_unrelated_global_effects() {
    // FnEffectVar returns only the selected C4Value (C4Script.cpp:5571-5586).
    let measure = |count: i32| {
        let mut engine = Engine::new();
        engine
            .register_script_definition(
                "TEST",
                "Test",
                "#strict 2\nfunc Probe() { return EffectVar(0, 0, 1); }",
            )
            .unwrap();
        engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
        let mut state = engine.capture_state();
        state.global_effects = (0..count)
            .map(|index| {
                let mut effect = clonk_engine::EffectState::new(format!("Untouched{index}"))
                    .with_vars(vec![clonk_engine::EffectVarValue::Int(42)]);
                effect.number = index + 1;
                effect
            })
            .collect();
        engine.restore_state(&state).unwrap();
        engine.call_object_function(0, "Probe", Vec::new()).unwrap();
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = engine.call_object_function(0, "Probe", Vec::new());
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(result.unwrap(), Value::Int(42));
        assert_eq!(engine.global_effects(), state.global_effects);
        allocations
    };
    let small = measure(1);
    let large = measure(64);
    assert!(
        large <= small + 2,
        "EffectVar read allocations grew from {small} to {large}"
    );
}

#[test]
fn callback_reuses_object_effects_from_its_state_snapshot() {
    // Entering C4AulExec does not copy the carrier's effect list either
    // (C4AulExec.cpp:330-359). The Rust boundary already owns one snapshot.
    let mut engine = Engine::new();
    engine
        .register_script_definition("TEST", "Test", "func Probe() { return 42; }")
        .unwrap();
    engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
    let measure = |engine: &mut Engine| {
        engine.call_object_function(0, "Probe", Vec::new()).unwrap();
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = engine.call_object_function(0, "Probe", Vec::new());
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(result.unwrap(), Value::Int(42));
        allocations
    };
    let empty = measure(&mut engine);
    let mut state = engine.capture_state();
    state.objects[0].snapshot.effects = (0..64)
        .map(|index| {
            clonk_engine::EffectState::new(format!("Untouched{index}")).with_vars(vec![
                clonk_engine::EffectVarValue::Array(vec![clonk_engine::EffectVarValue::Int(index)]),
            ])
        })
        .collect();
    engine.restore_state(&state).unwrap();
    let populated = measure(&mut engine);
    assert_eq!(
        engine.capture_state().objects[0].snapshot.effects,
        state.objects[0].snapshot.effects
    );
    // One snapshot vector and three owned allocations per effect: name,
    // variable list, and nested array. No second callback-private copy.
    assert!(
        populated <= empty + 64 * 3 + 2,
        "object effect allocations grew from {empty} to {populated}"
    );
}

#[test]
fn callback_that_ignores_teams_does_not_copy_rosters() {
    // C4AulExec retains Game.Teams when entering a call; it does not copy
    // roster strings or member lists (C4AulExec.cpp:330-359).
    let mut engine = Engine::new();
    engine
        .register_script_definition("TEST", "Test", "func Probe() { return 42; }")
        .unwrap();
    engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
    let measure = |engine: &mut Engine| {
        engine.call_object_function(0, "Probe", Vec::new()).unwrap();
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = engine.call_object_function(0, "Probe", Vec::new());
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(result.unwrap(), Value::Int(42));
        allocations
    };
    let empty = measure(&mut engine);
    let teams = (1..=64)
        .map(|id| {
            clonk_engine::TeamInfo::new(id, format!("Team {id}"), 0).with_player_ids(vec![id])
        })
        .collect::<Vec<_>>();
    engine.set_teams(teams);
    let teams = engine.teams().to_vec();
    let populated = measure(&mut engine);
    assert_eq!(engine.teams(), teams);
    assert!(
        populated <= empty + 2,
        "callback allocations grew from {empty} to {populated} for untouched teams"
    );
}

#[test]
fn callback_that_ignores_players_does_not_build_player_tables() {
    let mut engine = Engine::new();
    engine
        .register_script_definition("TEST", "Test", "func Probe() { return 42; }")
        .unwrap();
    engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
    let measure = |engine: &mut Engine| {
        engine.call_object_function(0, "Probe", Vec::new()).unwrap();
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = engine.call_object_function(0, "Probe", Vec::new());
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(result.unwrap(), Value::Int(42));
        allocations
    };
    let empty = measure(&mut engine);
    for id in 0..64 {
        engine
            .register_player(clonk_engine::PlayerConfig::new(id, format!("Player {id}")))
            .unwrap();
    }
    let populated = measure(&mut engine);
    assert!(
        populated <= empty + 1,
        "callback allocations grew from {empty} to {populated} for unused player tables"
    );
}

#[test]
fn same_object_nested_calls_reuse_live_locals_without_snapshots() {
    // Re-entry keeps the same C4Object::Local cells (C4AulExec.cpp:343-352).
    // Count array-storage allocations separately from required VM name/identity
    // bindings, then subtract the outer call to isolate eight re-entries.
    let nested_copies = |local_count: usize| {
        let mut engine = Engine::new();
        let mut source = String::from("#strict 3\n");
        let mut locals = std::collections::HashMap::new();
        for index in 0..local_count {
            let name = format!("unused{index}");
            source.push_str(&format!("local {name};\n"));
            locals.insert(name, Value::Array(vec![Value::Int(7); LOCAL_ARRAY_LEN]));
        }
        source.push_str("func Inner() { return 42; }\nfunc Probe(n) { var sum = 0; for (var i = 0; i < n; ++i) sum += this()->Inner(); return sum; }");
        engine
            .register_script_definition("TEST", "Test", &source)
            .unwrap();
        engine
            .spawn_object(SpawnConfig::new("TEST").with_local_vars(locals))
            .unwrap();
        let mut measure = |calls: i32| {
            engine
                .call_object_function(0, "Probe", vec![Value::Int(calls)])
                .unwrap();
            LOCAL_ARRAY_COPIES.with(|copies| copies.set(0));
            ALLOCATIONS.with(|count| count.set(Some(0)));
            let result = engine.call_object_function(0, "Probe", vec![Value::Int(calls)]);
            let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
            assert_eq!(result.unwrap(), Value::Int(calls * 42));
            eprintln!(
                "{local_count} locals, {calls} nested calls: {allocations} total allocations"
            );
            LOCAL_ARRAY_COPIES.with(Cell::get)
        };
        let outer = measure(0);
        measure(8) - outer
    };
    let empty = nested_copies(0);
    let populated = nested_copies(64);
    assert!(
        populated <= empty,
        "nested callback array copies grew from {empty} to {populated} for untouched live locals"
    );
}

#[test]
fn warm_callbacks_reuse_temporary_object_storage() {
    let mut engine = Engine::new();
    engine
        .register_script_definition("TEST", "Test", "func Probe() { return 42; }")
        .unwrap();
    engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
    for _ in 0..2 {
        engine.call_object_function(0, "Probe", Vec::new()).unwrap();
    }
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let result = engine.call_object_function(0, "Probe", Vec::new());
    let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(result.unwrap(), Value::Int(42));
    assert!(
        allocations <= 42,
        "warm callback allocated {allocations} times; reuse object lookup and session storage"
    );
}
