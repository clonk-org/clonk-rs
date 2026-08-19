use clonk_engine::{Engine, Landscape, SpawnConfig, Vector2};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::hint::black_box;

const BOUNCER_SCRIPT: &str = r#"
#strict 3
global func Initialize(state, random)
{
    return { energy = state.energy + (random % 3) };
}

global func Step(state, frame, random)
{
    var vx = state.velocity[0];
    var vy = state.velocity[1] + 1;
    var x = state.position[0] + vx;
    var y = state.position[1] + vy;

    if (y > 96)
    {
        y = 96;
        vy = -vy / 2;
    }
    if (x > 480)
    {
        x = 480;
        vx = -vx;
    }
    if (x < 0)
    {
        x = 0;
        vx = -vx;
    }

    return {
        position = [x, y],
        velocity = [vx, vy],
        energy = state.energy - 1,
    };
}
"#;

fn setup_engine(object_count: usize) -> Engine {
    let mut engine = Engine::with_seed(black_box(12345));
    engine
        .register_script_definition("Bouncer", "Bouncer", BOUNCER_SCRIPT)
        .expect("definition registers");
    engine.set_landscape(Landscape::flat(512, 96));

    for index in 0..object_count {
        let offset = (index % 16) as i32 * 30;
        let velocity = match index % 3 {
            0 => -2,
            1 => 0,
            _ => 2,
        };
        engine
            .spawn_object(
                SpawnConfig::new("Bouncer")
                    .with_position(Vector2::new(40 + offset, 20))
                    .with_velocity(Vector2::new(velocity, 0))
                    .with_energy(100),
            )
            .expect("object spawns");
    }

    engine
}

fn bench_engine_ticks(c: &mut Criterion) {
    c.bench_function("engine_tick_bouncers", |b| {
        b.iter_batched(
            || setup_engine(black_box(64)),
            |mut engine| {
                for _ in 0..32 {
                    let snapshot = engine.tick().expect("engine tick succeeds");
                    black_box(snapshot);
                }
            },
            BatchSize::SmallInput,
        );
    });
}

/// `Engine::snapshot` against the advance it follows, at two world sizes.
///
/// The question this exists to answer is whether snapshot *construction* is
/// worth optimising, and that is a ratio rather than a number: `tick` is
/// `tick_without_snapshot` plus the projection, so benching both separates
/// them without instrumenting the engine. Two sizes show how the projection
/// scales against the advance as the world grows — it is the growth, not the
/// small-world cost, that the hypothesis is about.
fn bench_snapshot_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_projection");
    for objects in [64_usize, 512] {
        group.bench_function(format!("advance_{objects}"), |b| {
            b.iter_batched(
                || setup_engine(black_box(objects)),
                |mut engine| {
                    for _ in 0..32 {
                        engine
                            .tick_without_snapshot()
                            .expect("engine advance succeeds");
                    }
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("advance_and_snapshot_{objects}"), |b| {
            b.iter_batched(
                || setup_engine(black_box(objects)),
                |mut engine| {
                    for _ in 0..32 {
                        let snapshot = engine.tick().expect("engine tick succeeds");
                        black_box(snapshot);
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_engine_ticks, bench_snapshot_projection);
criterion_main!(benches);
