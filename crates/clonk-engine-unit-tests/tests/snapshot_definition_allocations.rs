use clonk_engine::{Definition, Engine};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;
thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
// SAFETY: allocation, reallocation, and deallocation retain System's contract;
// the thread-local counter does not allocate or inspect allocated memory.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get().map(|n| n + 1)));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get().map(|n| n + 1)));
        unsafe { System.realloc(ptr, layout, size) }
    }
}

fn snapshot_allocations(count: usize, fog: bool) -> usize {
    let mut engine = Engine::new();
    for i in 0..count {
        let mut definition = Definition::from_script(format!("D{i:03}"), "test", "").unwrap();
        definition.set_line(1);
        definition.set_closed_container(1);
        engine.register_definition(definition).unwrap();
    }
    engine
        .register_player(clonk_engine::PlayerConfig::new(0, "viewer"))
        .unwrap();
    let _ = engine.player_mut(0).unwrap().set_fog_of_war(fog);
    let retained = engine.snapshot();
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let snapshot = engine.snapshot();
    let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(
        snapshot.definition_categories.len(),
        retained.definition_categories.len()
    );
    allocations
}

#[test]
fn warm_snapshot_allocations_do_not_scale_with_definition_count() {
    for fog in [false, true] {
        let small = snapshot_allocations(1, fog);
        let large = snapshot_allocations(256, fog);
        eprintln!(
            "fog={fog}: warm snapshot allocations: 1 definition={small}, 256 definitions={large}"
        );
        assert_eq!(
            large, small,
            "warm snapshot allocations must be independent of definition count"
        );
    }
}

#[test]
fn snapshot_definition_metadata_survives_reload_and_fog_changes() {
    use clonk_engine::{DefinitionLineMetadata, PlayerConfig};
    use std::sync::Arc;

    let root = tempfile::tempdir().unwrap();
    let group = root.path().join("Metadata.c4d");
    std::fs::create_dir(&group).unwrap();
    let write_core = |category, closed, line, intersect| {
        std::fs::write(
            group.join("DefCore.txt"),
            format!("[DefCore]\nid=META\nVersion=4,9,8\nName=Metadata\nCategory={category}\nClosedContainer={closed}\nLine={line}\nLineIntersect={intersect}\n"),
        ).unwrap();
    };
    write_core(1, 2, 1, 4);
    let mut engine = Engine::new();
    assert!(engine.load_definition_from_path(&group));
    engine
        .register_script_definition("ZERO", "zero", "")
        .unwrap();
    engine
        .register_player(PlayerConfig::new(0, "viewer"))
        .unwrap();
    let _ = engine.player_mut(0).unwrap().set_fog_of_war(false);
    let clear = engine.snapshot();
    assert!(clear.definition_closed_containers.is_empty());
    assert_eq!(clear.definition_categories.len(), 2);
    assert_eq!(clear.definition_categories.get("META"), Some(&1));
    assert_eq!(clear.definition_lines.len(), 1);
    assert_eq!(
        clear.definition_lines.get("META"),
        Some(&DefinitionLineMetadata {
            line: 1,
            line_intersect: 4
        })
    );

    let _ = engine.player_mut(0).unwrap().set_fog_of_war(true);
    let retained = engine.snapshot();
    let retained_json = serde_json::to_value(&retained).unwrap();
    assert_eq!(retained.definition_closed_containers.len(), 1);
    assert_eq!(retained.definition_closed_containers.get("META"), Some(&2));
    assert_eq!(clear.definition_categories, retained.definition_categories);
    assert_eq!(clear.definition_lines, retained.definition_lines);
    assert!(Arc::ptr_eq(
        &clear.definition_categories,
        &retained.definition_categories
    ));
    let next = engine.snapshot();
    assert!(Arc::ptr_eq(
        &next.definition_lines,
        &retained.definition_lines
    ));
    assert!(Arc::ptr_eq(
        &next.definition_closed_containers,
        &retained.definition_closed_containers
    ));

    // C4Game.cpp:2310-2355 reloads the definition and updates references;
    // retained Rust presentation frames must still own their original values.
    write_core(8, 1, 0, 2);
    assert!(engine.reload_definition("META", false));
    let reloaded = engine.tick().unwrap();
    assert_eq!(reloaded.definition_categories.get("META"), Some(&8));
    assert_eq!(reloaded.definition_closed_containers.get("META"), Some(&1));
    assert_eq!(
        reloaded.definition_lines.get("META"),
        Some(&DefinitionLineMetadata {
            line: 0,
            line_intersect: 2
        })
    );
    assert_eq!(serde_json::to_value(&retained).unwrap(), retained_json);

    // Failed C4Game::ReloadDef removes the definition (C4Game.cpp:2337-2346).
    std::fs::remove_file(group.join("DefCore.txt")).unwrap();
    assert!(!engine.reload_definition("META", false));
    let removed = engine.snapshot();
    assert!(!removed.definition_categories.contains_key("META"));
    assert!(removed.definition_closed_containers.is_empty());
    assert!(removed.definition_lines.is_empty());
    assert_eq!(serde_json::to_value(&retained).unwrap(), retained_json);
    assert_eq!(reloaded.definition_categories.get("META"), Some(&8));

    let _ = engine.player_mut(0).unwrap().set_fog_of_war(false);
    assert!(engine.snapshot().definition_closed_containers.is_empty());
    engine
        .register_script_definition("LATE", "late", "")
        .unwrap();
    assert!(engine.snapshot().definition_categories.contains_key("LATE"));
    assert!(!removed.definition_categories.contains_key("LATE"));

    let mut edited = retained.clone();
    Arc::make_mut(&mut edited.definition_categories).insert("META".into(), 16);
    Arc::make_mut(&mut edited.definition_closed_containers).clear();
    Arc::make_mut(&mut edited.definition_lines).clear();
    assert_eq!(serde_json::to_value(&retained).unwrap(), retained_json);
}
