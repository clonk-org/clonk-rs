//! Allocation budget for warmed scalar script frames.
use clonk_script::{Engine, Value};
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
fn warm_scalar_call_reuses_parameter_and_local_storage() {
    // C++ keeps parameters and hoisted vars in C4AulExec::Values, with their
    // names resolved by the parser (C4AulExec.cpp:330-347; C4AulParse.cpp:2709-2729).
    let mut engine = Engine::new();
    engine
        .load_script(
            "#strict 2\nfunc Sum(a, b, c, d, e, f) {\n\
             var left = a + b, middle = c + d, right = e + f;\n\
             return left + middle + right;\n}",
        )
        .unwrap();
    let args = [1, 2, 3, 4, 5, 6].map(Value::Int);
    assert_eq!(engine.call("Sum", &args).unwrap(), Value::Int(21));
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let result = engine.call("Sum", &args);
    let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(result.unwrap(), Value::Int(21));
    eprintln!("warm scalar call allocations: {allocations}");
    assert!(
        allocations <= 16,
        "warm scalar call allocated {allocations} times"
    );
}
