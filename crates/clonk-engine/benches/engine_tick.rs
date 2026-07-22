use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use clonk_engine::{Definition, Engine, Landscape, SpawnConfig, Vector2};

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
        .register_definition(
            Definition::from_script("Bouncer", "Bouncer", BOUNCER_SCRIPT)
                .expect("benchmark definition compiles"),
        )
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

criterion_group!(benches, bench_engine_ticks);
criterion_main!(benches);
