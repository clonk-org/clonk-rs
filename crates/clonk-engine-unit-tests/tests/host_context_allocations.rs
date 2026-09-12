//! Allocation budget for warm script callbacks.
use clonk_engine::{Engine, SpawnConfig};
use clonk_script::Value;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CountingAllocator;
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: forwards the caller's allocation contract unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: forwards the caller's allocation contract unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
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
