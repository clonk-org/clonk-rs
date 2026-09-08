//! A snapshot is a value: frame N's must not move when the engine advances.
//!
//! clonk-org/clonk-rs#294's stability criterion. The hazard is not the owned
//! `Vec`s — it is the copy-on-write backing. `PixelGrid.bytes` is an
//! `Arc<Vec<u8>>` and the 32-bit surface an `Arc<HashMap<..>>`, both shared
//! with the live engine at the moment the snapshot is taken. That sharing is
//! deliberate and is what keeps an unchanged landscape cheap, but it means a
//! later write has to clone out of the `Arc` rather than through it. If it ever
//! wrote through, a snapshot already handed to the renderer or the recorder
//! would change under its holder.

use super::*;

struct SnapshotAllocationCounter;

thread_local! {
    // Only count large allocations on the measuring test's thread.
    static LARGE_ALLOCATIONS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn count_allocation(size: usize) {
    if size >= 32 * 1024 {
        let _ = LARGE_ALLOCATIONS.try_with(|count| {
            count.set(count.get().map(|value| value + 1));
        });
    }
}

// SAFETY: every operation delegates unchanged pointers/layouts to System.
unsafe impl std::alloc::GlobalAlloc for SnapshotAllocationCounter {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_allocation(layout.size());
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_allocation(layout.size());
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        count_allocation(size);
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: SnapshotAllocationCounter = SnapshotAllocationCounter;

#[test]
fn populated_snapshot_allocates_only_one_large_object_buffer() {
    let mut engine = carving_engine();
    for index in 0..627 {
        crate::TestValueExt::test_value(engine.spawn_object(
            SpawnConfig::new("SNAP").with_position(Vector2::new(index % 64, index % 32)),
        ));
    }
    // Exercise a nontrivial physical order, independent of allocation order.
    for index in 0..engine.objects.len() {
        engine.objects.swap(index, (index * 37 + 11) % 627);
    }
    LARGE_ALLOCATIONS.with(|count| count.set(Some(0)));
    let snapshot = engine.snapshot();
    let allocations = LARGE_ALLOCATIONS.with(|count| count.replace(None));
    assert_eq!(
        allocations,
        Some(1),
        "only the returned object Vec needs a large allocation"
    );

    assert!(snapshot
        .objects
        .windows(2)
        .all(|pair| pair[0].id < pair[1].id));
    let mut stable: Vec<_> = engine
        .objects
        .iter()
        .map(|object| {
            object.snapshot(
                engine
                    .definitions
                    .get(&object.definition_id)
                    .map(|definition| definition.action_library()),
            )
        })
        .collect();
    stable.sort_by_key(|object| object.id);
    assert_eq!(
        crate::TestValueExt::test_value(serde_json::to_vec(&snapshot.objects)),
        crate::TestValueExt::test_value(serde_json::to_vec(&stable)),
    );
}

#[test]
fn restored_duplicate_ids_keep_stable_snapshot_order() {
    let mut engine = carving_engine();
    for index in 0..128 {
        crate::TestValueExt::test_value(
            engine.spawn_object(SpawnConfig::new("SNAP").with_position(Vector2::new(index, 4))),
        );
    }
    let mut state = engine.capture_state();
    let ids: Vec<_> = state
        .objects
        .iter()
        .take(8)
        .map(|object| object.snapshot.id)
        .collect();
    for (index, object) in state.objects.iter_mut().enumerate() {
        object.snapshot.id = ids[(index * 5 + 3) % ids.len()];
    }
    crate::TestValueExt::test_value(engine.restore_state(&state));
    assert!(!engine.object_ids_are_unique());

    let mut stable: Vec<_> = engine
        .objects
        .iter()
        .map(|object| {
            object.snapshot(
                engine
                    .definitions
                    .get(&object.definition_id)
                    .map(|definition| definition.action_library()),
            )
        })
        .collect();
    stable.sort_by_key(|object| object.id);
    assert_eq!(
        crate::TestValueExt::test_value(serde_json::to_vec(&engine.snapshot().objects)),
        crate::TestValueExt::test_value(serde_json::to_vec(&stable)),
    );
}

/// Serialising is what makes this test able to fail.
///
/// Comparing a retained snapshot against a `clone()` of itself would not:
/// `Arc::clone` shares the same allocation, so an in-place write moves both
/// copies together and the equality holds while the invariant is broken.
/// Materialising the bytes first is the only comparison that survives that.
fn serialized(snapshot: &SimulationSnapshot) -> String {
    crate::TestValueExt::test_value(serde_json::to_string(snapshot))
}

fn carving_engine() -> Engine {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition(
        "SNAP",
        "SNAP",
        // Writes one landscape pixel, which is the mutation that reaches the
        // shared backing rather than only the owned object vector.
        "func Carve() { return SetLandscapePixel(1, 0, 16711680); }",
    ));

    let mut landscape = Landscape::flat(64, 32);
    assert!(landscape.set_mode(LANDSCAPE_MODE_EXACT));
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        64,
        32,
        vec![1_u8; 64 * 32],
        {
            let mut densities = vec![0_i32; 256];
            densities[1] = 100;
            densities
        },
        vec![None; 256],
        vec![None; 256],
    ));
    engine.set_landscape(landscape);
    engine
}

#[test]
fn a_retained_snapshot_does_not_move_when_the_engine_does() {
    let mut engine = carving_engine();
    let first = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("SNAP").with_position(Vector2::new(8, 4))),
    );
    let second = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("SNAP").with_position(Vector2::new(20, 4))),
    );

    let retained = engine.snapshot();
    let before = serialized(&retained);
    assert!(
        retained.landscape.is_some(),
        "the landscape has to be in the snapshot for its backing to be shared at all"
    );

    // Frame N+1, in the order a real frame would do it: advance, script write,
    // advance again so the batched landscape write lands, then a deletion.
    crate::TestValueExt::test_value(engine.tick());
    let index = crate::TestValueExt::test_value(engine.find_object_index(first));
    crate::TestValueExt::test_value(engine.call_object_function(index, "Carve", Vec::new()));
    crate::TestValueExt::test_value(engine.tick());
    crate::TestValueExt::test_value(engine.assign_object_removal(second));
    crate::TestValueExt::test_value(engine.tick());

    // A reload, which the criterion names alongside the ordinary advance. A
    // scenario *section* change is not exercised here: it rebuilds the world
    // rather than mutating it, so it cannot write through a shared backing.
    let state = engine.capture_state();
    crate::TestValueExt::test_value(engine.restore_state(&state));

    assert_eq!(
        serialized(&retained),
        before,
        "the retained snapshot moved after the engine did"
    );
}

/// The other half of the same invariant: the *next* snapshot must show the
/// change. A projection that never updated would pass the test above trivially.
#[test]
fn the_next_snapshot_does_show_what_the_engine_did() {
    let mut engine = carving_engine();
    let object = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("SNAP").with_position(Vector2::new(8, 4))),
    );

    let before = serialized(&engine.snapshot());

    let index = crate::TestValueExt::test_value(engine.find_object_index(object));
    crate::TestValueExt::test_value(engine.call_object_function(index, "Carve", Vec::new()));
    crate::TestValueExt::test_value(engine.tick());

    assert_ne!(
        serialized(&engine.snapshot()),
        before,
        "a fresh snapshot has to reflect the write the retained one must not"
    );
}
