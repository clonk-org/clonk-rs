//! Allocation regression for the newgfx execution loop.
use clonk_engine::particles::{ParticleDefCore, ParticleEnv, ParticleSystem};
use clonk_engine::ParticleLayer;
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
fn particle_heavy_execution_allocates_nothing() {
    // C4Particles.cpp:250-267 executes newest-first in place; fxStdExec
    // (614-697) reads the definition through its pointer, without copying it.
    let mut system = ParticleSystem::default();
    for index in 0..46 {
        let name = format!("Particle{index}");
        system
            .register_def(
                ParticleDefCore {
                    name: name.clone(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    max_count: 1000,
                    delay: 1,
                    repeats: 100,
                    ..ParticleDefCore::default()
                },
                8,
                1.0,
            )
            .unwrap();
        for _ in 0..24 {
            assert!(system.create(
                &name,
                100.0,
                100.0,
                1.0,
                0.5,
                10.0,
                0xffffff,
                ParticleLayer::Global,
                None
            ));
        }
    }
    let env = ParticleEnv {
        gravity: Default::default(),
        frame_counter: 1,
        back_wdt: 1000,
        back_hgt: 1000,
        solid: &|_, _| false,
        wind: &|_, _| 0,
    };
    ALLOCATIONS.with(|count| count.set(Some(0)));
    system.exec_layer(&ParticleLayer::Global, None, &env);
    let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(system.particles().len(), 1104);
    assert_eq!(
        allocations, 0,
        "one frame must not allocate per live particle"
    );
}

fn definition(name: &str, exec: &str) -> ParticleDefCore {
    ParticleDefCore {
        name: name.into(),
        init_fn: "StdInit".into(),
        exec_fn: exec.into(),
        draw_fn: "Std".into(),
        ..ParticleDefCore::default()
    }
}

fn tick(system: &mut ParticleSystem) {
    system.exec_layer(
        &ParticleLayer::Global,
        None,
        &ParticleEnv {
            gravity: Default::default(),
            frame_counter: 1,
            back_wdt: 1000,
            back_hgt: 1000,
            solid: &|_, _| false,
            wind: &|_, _| 0,
        },
    );
}

fn create(system: &mut ParticleSystem, name: &str) -> bool {
    system.create(
        name,
        100.0,
        100.0,
        1.0,
        2.0,
        10.0,
        1,
        ParticleLayer::Global,
        None,
    )
}

#[test]
fn definition_lookup_survives_removal_reordering_and_overload() {
    // GetDef returns the first name match (C4Particles.cpp:465-473), even
    // after overload moves a replacement to the tail (178-187).
    let mut system = ParticleSystem::default();
    for (name, exec) in [("First", "Stop"), ("Live", "BounceY"), ("Last", "Bounce")] {
        system.register_def(definition(name, exec), 1, 1.0).unwrap();
    }
    assert!(create(&mut system, "Live"));
    tick(&mut system);
    assert_eq!(system.particles()[0].ydir, -2.0);
    assert!(system.remove_def("First"));
    tick(&mut system);
    assert_eq!(system.particles()[0].xdir, 1.0);
    assert_eq!(system.particles()[0].ydir, 2.0);
    system.restore_def_order(0);
    tick(&mut system);
    assert_eq!(system.particles()[0].xdir, 1.0);
    assert_eq!(system.particles()[0].ydir, -2.0);
    system.clear_particles();
    system
        .register_def(definition("Live", "Stop"), 1, 1.0)
        .unwrap();
    assert!(create(&mut system, "Live"));
    tick(&mut system);
    assert_eq!(system.particles()[0].xdir, 0.0);
    assert_eq!(system.particles()[0].ydir, 0.0);
}

#[test]
fn renamed_definitions_preserve_first_match_and_missing_snapshot_particles() {
    use clonk_engine::particles::Particle;
    let mut system = ParticleSystem::default();
    system
        .register_def(definition("Old", "Bounce"), 1, 1.0)
        .unwrap();
    system
        .register_def(definition("Other", "Stop"), 1, 1.0)
        .unwrap();
    assert!(create(&mut system, "Old"));
    let snapshot = serde_json::to_string(&system.particles()[0]).unwrap();
    let restored: Particle = serde_json::from_str(&snapshot).unwrap();
    system.get_def_mut("Old").unwrap().core.name = "New".into();
    system.restore_particle(restored.clone());
    tick(&mut system);
    assert_eq!(system.particles(), &[restored.clone(), restored]);
    assert!(!create(&mut system, "Old"));
    // Public mutable access can produce duplicates: preserve GetDef's first
    // exact-case match (C4Particles.cpp:465-473), including after reordering.
    system.get_def_mut("Other").unwrap().core.name = "New".into();
    assert!(create(&mut system, "New"));
    tick(&mut system);
    assert_eq!(system.particles()[2].xdir, -1.0);
    system.restore_def_order(0);
    tick(&mut system);
    assert_eq!(system.particles()[2].xdir, 0.0);
    assert!(!create(&mut system, "new"));
}

#[test]
fn smoke_draws_follow_newest_first_order_across_deaths_and_layers() {
    use clonk_engine::particles::{Particle, SafeRng};
    use clonk_engine::ObjectId;
    // C4ParticleList::Exec (C4Particles.cpp:250-267) walks newest-first;
    // fxSmokeExec (540, 556-562) kills life=1 before drawing for building smoke.
    let mut system = ParticleSystem::default();
    system
        .register_def(definition("Smoke", "SmokeExec"), 1, 1.0)
        .unwrap();
    let particle = |life, layer| Particle {
        def_name: "Smoke".into(),
        x: 100.0,
        y: 100.0,
        xdir: 0.0,
        ydir: 0.0,
        life,
        a: 4.0,
        b: 0xff4b4b4bu32 as i32,
        layer,
    };
    for life in [0x20022, 1, 0x20022] {
        system.restore_particle(particle(life, ParticleLayer::Global));
    }
    let other = particle(0x20022, ParticleLayer::ObjectFront(ObjectId::new(7)));
    system.restore_particle(other.clone());
    system.safe_rng = SafeRng::new(7);
    let mut expected = SafeRng::new(7);
    let newest = 0.1f32 * expected.random(41) as f32 - 2.0;
    let oldest = 0.1f32 * expected.random(41) as f32 - 2.0;
    tick(&mut system);
    let particles = system.particles();
    assert_eq!(particles.len(), 3);
    assert_eq!(particles[0].xdir.to_bits(), oldest.to_bits());
    assert_eq!(particles[1].xdir.to_bits(), newest.to_bits());
    assert_eq!(particles[0].life, 0x10021);
    assert_eq!(particles[1].life, 0x10021);
    assert_eq!(particles[2], other);
    assert_eq!(system.get_def("Smoke").unwrap().count, 3);
    assert_eq!(system.safe_rng.random(1000), expected.random(1000));
}
