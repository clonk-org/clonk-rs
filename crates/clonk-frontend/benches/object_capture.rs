use clonk_engine::{
    ActionState, DefinitionActionFacet, DefinitionActionGraphics, DefinitionRect, Direction,
    DrawTransform, ObjectId, ObjectSnapshot, Vector2,
};
use clonk_frontend::{CursorAtlas, DefinitionSprite, GraphicsSystem, HudGraphics, ImageData};
use clonk_graphics::{BitmapFont, GammaRamp, GpuCommand};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const OBJECTS: usize = 1_000;
const PHASES: usize = 20;
const FACE_SIZE: i32 = 15;
const EXTENT: [u32; 2] = [800, 600];

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_BYTES: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's allocation contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's allocation contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` and `layout` came from the wrapped allocator.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's reallocation contract.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AllocationStats {
    calls: usize,
    bytes: usize,
}

fn measure_allocations<T>(capture: impl FnOnce() -> T) -> (T, AllocationStats) {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATION_BYTES.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let result = capture();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    (
        result,
        AllocationStats {
            calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
            bytes: ALLOCATION_BYTES.load(Ordering::Relaxed),
        },
    )
}

fn object_template() -> ObjectSnapshot {
    serde_json::from_value(serde_json::json!({
        "id": 1,
        "definition_id": "ST5B",
        "position": { "x": 0, "y": 0 },
        "velocity": { "x": 0, "y": 0 },
        "energy": 100000
    }))
    .expect("minimal benchmark ObjectSnapshot")
}

fn objects() -> Vec<ObjectSnapshot> {
    let mut template = object_template();
    template.action = ActionState::new("Walk");
    template.crew_member = false;
    (0..OBJECTS)
        .map(|index| {
            let mut object = template.clone();
            object.id = ObjectId::new(index as u64 + 1);
            object.position = Vector2::new(
                8 + (index % 40) as i32 * FACE_SIZE,
                8 + (index / 40) as i32 * FACE_SIZE,
            );
            object.action.phase = (index % PHASES) as i32;
            object.direction = if index.is_multiple_of(2) {
                Direction::Left
            } else {
                Direction::Right
            };
            object.draw_transform = (!index.is_multiple_of(2))
                .then(|| DrawTransform::from_components(-1.0, 1.0, 0.0, 0.0));
            object.color_modulation = 0x0040_0000 | (index as u32 + 1);
            object
        })
        .collect()
}

fn graphics() -> GraphicsSystem {
    // This mirrors the shipped ST5B data in
    // `content/EkeReloaded.c4d/Creatures.c4d/Stippel.c4d`: DefCore is 15x15
    // with StretchGrowth=1; Walk has two directions, FlipDir=1 and 20 phases.
    let walk = DefinitionActionGraphics {
        facet: Some(DefinitionActionFacet {
            x: 0,
            y: 0,
            width: FACE_SIZE,
            height: FACE_SIZE,
            target_x: 0,
            target_y: 0,
        }),
        directions: 2,
        flip_dir: Some(1),
        length: Some(PHASES as i32),
        ..DefinitionActionGraphics::default()
    };
    let mut pixels = Vec::with_capacity(300 * 110 * 4);
    for y in 0..110_u32 {
        for x in 0..300_u32 {
            pixels.extend_from_slice(&[
                (32 + x.wrapping_mul(7)) as u8,
                (48 + y.wrapping_mul(11)) as u8,
                (64 + (x ^ y).wrapping_mul(3)) as u8,
                255,
            ]);
        }
    }
    let sprite = DefinitionSprite {
        image: ImageData::new(300, 110, pixels),
        actions: HashMap::from([("Walk".to_owned(), walk)]),
        color_mask: None,
        graphics_scale: 1.0,
        shape: Some(DefinitionRect::new(-7, -7, FACE_SIZE, FACE_SIZE)),
        fire_top: 0,
        rotateable: 0,
        line: 0,
        stretch_growth: true,
        top_face: None,
        picture: None,
    };
    GraphicsSystem::new(
        EXTENT[0],
        EXTENT[1],
        EXTENT[1] as i32,
        "ST5B object capture benchmark",
        Arc::new(BitmapFont::new()),
        Arc::new(HashMap::from([("ST5B".to_owned(), sprite)])),
        Arc::new(CursorAtlas::empty()),
        Arc::new(HudGraphics::default()),
    )
}

fn assert_compact_scene(scene: &clonk_graphics::GpuScene, fogged: bool) -> usize {
    assert_eq!(scene.commands.len(), 1, "one ordered ST5B resource run");
    assert_eq!(scene.textures.len(), 1);
    let [GpuCommand::ObjectBatch { sprites, .. }] = scene.commands.as_slice() else {
        panic!("representable ST5B faces entered the generic quad path");
    };
    if fogged {
        assert!(sprites.len() >= OBJECTS);
    } else {
        assert_eq!(sprites.len(), OBJECTS);
    }
    sprites.len()
}

fn allocation_probe(
    graphics: &mut GraphicsSystem,
    objects: &[ObjectSnapshot],
    render_order: &[ObjectId],
    gamma: &GammaRamp,
    fogged: bool,
) -> (AllocationStats, AllocationStats) {
    for _ in 0..2 {
        let scene =
            graphics.capture_st5b_objects_for_benchmark(objects, render_order, fogged, gamma);
        black_box(scene);
    }
    let (one, one_stats) = measure_allocations(|| {
        graphics.capture_st5b_objects_for_benchmark(
            &objects[..1],
            &render_order[render_order.len() - 1..],
            fogged,
            gamma,
        )
    });
    black_box(one);
    let (full, full_stats) = measure_allocations(|| {
        graphics.capture_st5b_objects_for_benchmark(objects, render_order, fogged, gamma)
    });
    assert_compact_scene(&full, fogged);
    assert_eq!(
        full_stats.calls, one_stats.calls,
        "allocations scaled with the number of representable objects"
    );
    assert_eq!(
        full_stats.bytes, one_stats.bytes,
        "allocated bytes scaled with the number of representable objects"
    );
    (one_stats, full_stats)
}

fn bench_object_capture(c: &mut Criterion) {
    let objects = objects();
    let render_order = objects
        .iter()
        .rev()
        .map(|object| object.id)
        .collect::<Vec<_>>();
    let gamma = GammaRamp::standard();
    let mut unfogged_graphics = graphics();
    let mut fogged_graphics = graphics();
    let (unfogged_one_allocation, unfogged_allocations) = allocation_probe(
        &mut unfogged_graphics,
        &objects,
        &render_order,
        &gamma,
        false,
    );
    let (fogged_one_allocation, fogged_allocations) =
        allocation_probe(&mut fogged_graphics, &objects, &render_order, &gamma, true);
    let unfogged = unfogged_graphics.capture_st5b_objects_for_benchmark(
        &objects,
        &render_order,
        false,
        &gamma,
    );
    let fogged =
        fogged_graphics.capture_st5b_objects_for_benchmark(&objects, &render_order, true, &gamma);
    let unfogged_instances = assert_compact_scene(&unfogged, false);
    let fogged_instances = assert_compact_scene(&fogged, true);
    let [GpuCommand::ObjectBatch { sprites, .. }] = unfogged.commands.as_slice() else {
        unreachable!("the compact-scene assertion already checked the command")
    };
    assert_eq!(
        sprites[0].modulation[0],
        objects.last().expect("1,000 objects").color_modulation,
        "capture ignored the explicit painter order"
    );
    let instance_bytes = std::mem::size_of::<clonk_graphics::GpuObjectSprite>();
    assert_eq!(
        objects
            .iter()
            .map(|object| object.color_modulation)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        OBJECTS,
        "each benchmark object needs a distinct color modulation"
    );
    println!(
        "object capture raw stats: objects={OBJECTS}, phases={PHASES}, face={FACE_SIZE}x{FACE_SIZE}, \
         unfogged_instances={unfogged_instances}, fogged_instances={fogged_instances}, \
         instance_bytes={instance_bytes}, unfogged_upload_bytes={}, fogged_upload_bytes={}, \
         commands=1, generic_quads=0, \
         unfogged_allocations_one={unfogged_one_allocation:?}, \
         unfogged_allocations_1000={unfogged_allocations:?}, \
         fogged_allocations_one={fogged_one_allocation:?}, \
         fogged_allocations_1000={fogged_allocations:?}",
        unfogged_instances * instance_bytes,
        fogged_instances * instance_bytes,
    );

    let mut group = c.benchmark_group("object_capture");
    group.throughput(Throughput::Elements(OBJECTS as u64));
    group.bench_function("unfogged_1000_st5b", |b| {
        b.iter(|| {
            black_box(unfogged_graphics.capture_st5b_objects_for_benchmark(
                &objects,
                &render_order,
                false,
                &gamma,
            ))
        });
    });
    group.bench_function("fogged_1000_st5b", |b| {
        b.iter(|| {
            black_box(fogged_graphics.capture_st5b_objects_for_benchmark(
                &objects,
                &render_order,
                true,
                &gamma,
            ))
        });
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .sample_size(20);
    targets = bench_object_capture
}
criterion_main!(benches);
