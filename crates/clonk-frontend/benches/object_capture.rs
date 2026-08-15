use clonk_engine::{
    ActionState, DefinitionActionFacet, DefinitionActionGraphics, DefinitionRect, Direction,
    DrawTransform, ObjectId, ObjectSnapshot, Vector2,
};
use clonk_frontend::{
    ColorByOwnerMask, CursorAtlas, DefinitionSprite, GraphicsSystem, HudGraphics, ImageData,
};
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
const OWNER_DEFINITION_ID: &str = "HZCK";
const OWNER_PHASES: usize = 15;
const OWNER_FACET_X: i32 = 4;
const OWNER_FACE_WIDTH: i32 = 16;
const OWNER_FACE_HEIGHT: i32 = 20;
const OWNER_SHEET_WIDTH: u32 = 256;
const OWNER_SHEET_HEIGHT: u32 = 420;
const OBJECT_INSTANCE_BYTES: usize = 88;
const OWNER_UNFOGGED_INSTANCE_BUDGET: usize = 176 * 1024;
const OWNER_FOGGED_INSTANCES: usize = 2_400;

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

fn object_template(definition_id: &str) -> ObjectSnapshot {
    serde_json::from_value(serde_json::json!({
        "id": 1,
        "definition_id": definition_id,
        "position": { "x": 0, "y": 0 },
        "velocity": { "x": 0, "y": 0 },
        "energy": 100000
    }))
    .expect("minimal benchmark ObjectSnapshot")
}

fn objects() -> Vec<ObjectSnapshot> {
    let mut template = object_template("ST5B");
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

fn owner_objects() -> Vec<ObjectSnapshot> {
    let mut template = object_template(OWNER_DEFINITION_ID);
    template.action = ActionState::new("Walk");
    template.crew_member = true;
    (0..OBJECTS)
        .map(|index| {
            let mut object = template.clone();
            object.id = ObjectId::new(index as u64 + 1);
            object.position = Vector2::new(
                8 + (index % 40) as i32 * OWNER_FACE_WIDTH,
                8 + (index / 40) as i32 * OWNER_FACE_HEIGHT,
            );
            object.action.phase = (index % OWNER_PHASES) as i32;
            object.direction = if index.is_multiple_of(2) {
                Direction::Left
            } else {
                Direction::Right
            };
            object.draw_transform = (!index.is_multiple_of(2))
                .then(|| DrawTransform::from_components(-1.0, 1.0, 0.0, 0.0));
            object.color = 0x0001_0000 | (index as u32 + 1);
            object.color_modulation = 0x0020_0000 | (index as u32 + 1);
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

fn owner_graphics() -> GraphicsSystem {
    let walk = DefinitionActionGraphics {
        facet: Some(DefinitionActionFacet {
            // Deliberately offset the 16-pixel phases so phases 3, 7 and 11
            // cross a 64-pixel fog boundary. The warm allocation gate then
            // covers both object and compact-chunk growth.
            x: OWNER_FACET_X,
            y: 0,
            width: OWNER_FACE_WIDTH,
            height: OWNER_FACE_HEIGHT,
            target_x: 0,
            target_y: 0,
        }),
        directions: 2,
        flip_dir: Some(1),
        length: Some(OWNER_PHASES as i32),
        ..DefinitionActionGraphics::default()
    };
    let pixel_count = (OWNER_SHEET_WIDTH * OWNER_SHEET_HEIGHT) as usize;
    let mut base = Vec::with_capacity(pixel_count * 4);
    let mut overlay = Vec::with_capacity(pixel_count * 4);
    for y in 0..OWNER_SHEET_HEIGHT {
        for x in 0..OWNER_SHEET_WIDTH {
            base.extend_from_slice(&[
                (24 + x.wrapping_mul(5)) as u8,
                (40 + y.wrapping_mul(7)) as u8,
                (56 + (x ^ y).wrapping_mul(3)) as u8,
                255,
            ]);
            let alpha = if (x + 2 * y).is_multiple_of(5) {
                224
            } else {
                0
            };
            overlay.extend_from_slice(&[255, 255, 255, alpha]);
        }
    }
    let sprite = DefinitionSprite {
        image: ImageData::new(OWNER_SHEET_WIDTH, OWNER_SHEET_HEIGHT, base),
        actions: HashMap::from([("Walk".to_owned(), walk)]),
        color_mask: Some(ColorByOwnerMask::new(
            OWNER_SHEET_WIDTH,
            OWNER_SHEET_HEIGHT,
            Arc::from(overlay.into_boxed_slice()),
        )),
        graphics_scale: 1.0,
        shape: Some(DefinitionRect::new(
            -OWNER_FACE_WIDTH / 2,
            -OWNER_FACE_HEIGHT / 2,
            OWNER_FACE_WIDTH,
            OWNER_FACE_HEIGHT,
        )),
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
        "HZCK owner-color object capture benchmark",
        Arc::new(BitmapFont::new()),
        Arc::new(HashMap::from([(OWNER_DEFINITION_ID.to_owned(), sprite)])),
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

fn assert_owner_compact_scene(scene: &clonk_graphics::GpuScene, fogged: bool) -> usize {
    assert_eq!(scene.commands.len(), 1, "one ordered HZCK texture-pair run");
    assert_eq!(scene.textures.len(), 2, "base and owner textures only");
    let [GpuCommand::ObjectBatch {
        texture,
        owner_texture: Some(owner_texture),
        sprites,
        ..
    }] = scene.commands.as_slice()
    else {
        panic!("representable owner-colored faces entered the generic quad path");
    };
    assert_ne!(
        texture, owner_texture,
        "the owner layer needs its own resource"
    );
    let base_instances = sprites
        .iter()
        .filter(|sprite| !sprite.owner_layer())
        .count();
    let owner_instances = sprites.iter().filter(|sprite| sprite.owner_layer()).count();
    assert_eq!(base_instances, owner_instances);
    let mut cursor = 0;
    let mut faces = 0;
    while cursor < sprites.len() {
        let base_start = cursor;
        while cursor < sprites.len() && !sprites[cursor].owner_layer() {
            cursor += 1;
        }
        let owner_start = cursor;
        while cursor < sprites.len() && sprites[cursor].owner_layer() {
            cursor += 1;
        }
        let owner_end = cursor;
        assert!(owner_start > base_start, "owner face omitted its base pass");
        assert_eq!(
            owner_start - base_start,
            owner_end - owner_start,
            "owner face did not retain one owner chunk per base chunk"
        );
        assert!(
            sprites[base_start..owner_start]
                .iter()
                .zip(&sprites[owner_start..owner_end])
                .all(|(base, owner)| base.positions == owner.positions && base.uv == owner.uv),
            "an owner face changed chunk geometry between its ordered passes"
        );
        faces += 1;
    }
    assert_eq!(
        faces, OBJECTS,
        "adjacent owner faces were regrouped by layer"
    );
    if fogged {
        assert_eq!(sprites.len(), OWNER_FOGGED_INSTANCES);
    } else {
        assert_eq!(sprites.len(), OBJECTS * 2);
        assert_eq!(
            sprites.len() * std::mem::size_of::<clonk_graphics::GpuObjectSprite>(),
            OBJECTS * 2 * OBJECT_INSTANCE_BYTES
        );
        assert!(
            sprites.len() * std::mem::size_of::<clonk_graphics::GpuObjectSprite>()
                <= OWNER_UNFOGGED_INSTANCE_BUDGET
        );
    }
    sprites.len()
}

fn allocation_probe(
    graphics: &mut GraphicsSystem,
    objects: &[ObjectSnapshot],
    render_order: &[ObjectId],
    gamma: &GammaRamp,
    fogged: bool,
    assert_scene: fn(&clonk_graphics::GpuScene, bool) -> usize,
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
    assert_scene(&full, fogged);
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
        assert_compact_scene,
    );
    let (fogged_one_allocation, fogged_allocations) = allocation_probe(
        &mut fogged_graphics,
        &objects,
        &render_order,
        &gamma,
        true,
        assert_compact_scene,
    );
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

    let owner_objects = owner_objects();
    let owner_render_order = owner_objects
        .iter()
        .rev()
        .map(|object| object.id)
        .collect::<Vec<_>>();
    let mut owner_unfogged_graphics = owner_graphics();
    let mut owner_fogged_graphics = owner_graphics();
    let (owner_unfogged_one_allocation, owner_unfogged_allocations) = allocation_probe(
        &mut owner_unfogged_graphics,
        &owner_objects,
        &owner_render_order,
        &gamma,
        false,
        assert_owner_compact_scene,
    );
    let (owner_fogged_one_allocation, owner_fogged_allocations) = allocation_probe(
        &mut owner_fogged_graphics,
        &owner_objects,
        &owner_render_order,
        &gamma,
        true,
        assert_owner_compact_scene,
    );
    let owner_unfogged = owner_unfogged_graphics.capture_st5b_objects_for_benchmark(
        &owner_objects,
        &owner_render_order,
        false,
        &gamma,
    );
    let owner_fogged = owner_fogged_graphics.capture_st5b_objects_for_benchmark(
        &owner_objects,
        &owner_render_order,
        true,
        &gamma,
    );
    let owner_unfogged_instances = assert_owner_compact_scene(&owner_unfogged, false);
    let owner_fogged_instances = assert_owner_compact_scene(&owner_fogged, true);
    assert_eq!(std::mem::size_of::<clonk_graphics::GpuObjectSprite>(), 88);
    println!(
        "owner object capture raw stats: objects={OBJECTS}, definition={OWNER_DEFINITION_ID}, \
         phases={OWNER_PHASES}, face={OWNER_FACE_WIDTH}x{OWNER_FACE_HEIGHT}, \
         sheet={OWNER_SHEET_WIDTH}x{OWNER_SHEET_HEIGHT}, mask=full_rgba, \
         unfogged_instances={owner_unfogged_instances}, \
         fogged_instances={owner_fogged_instances}, instance_bytes={OBJECT_INSTANCE_BYTES}, \
         unfogged_upload_bytes={}, fogged_upload_bytes={}, commands=1, generic_quads=0, \
         unfogged_allocations_one={owner_unfogged_one_allocation:?}, \
         unfogged_allocations_1000={owner_unfogged_allocations:?}, \
         fogged_allocations_one={owner_fogged_one_allocation:?}, \
         fogged_allocations_1000={owner_fogged_allocations:?}",
        owner_unfogged_instances * OBJECT_INSTANCE_BYTES,
        owner_fogged_instances * OBJECT_INSTANCE_BYTES,
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
    group.bench_function("unfogged_1000_owner_colored_crew", |b| {
        b.iter(|| {
            black_box(owner_unfogged_graphics.capture_st5b_objects_for_benchmark(
                &owner_objects,
                &owner_render_order,
                false,
                &gamma,
            ))
        });
    });
    group.bench_function("fogged_1000_owner_colored_crew", |b| {
        b.iter(|| {
            black_box(owner_fogged_graphics.capture_st5b_objects_for_benchmark(
                &owner_objects,
                &owner_render_order,
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
