use criterion::{black_box, criterion_group, criterion_main, Criterion};
use clonk_script::{Engine, Value};

const SCRIPT: &str = r#"
global func SumLoop(iterations)
{
    var acc = 0;
    var index = 0;
    while (index < iterations)
    {
        acc = acc + (index % 7);
        index = index + 1;
    }
    return acc;
}
"#;

fn bench_script_execution(c: &mut Criterion) {
    let mut engine = Engine::new();
    engine
        .load_script(SCRIPT)
        .expect("benchmark script loads successfully");

    c.bench_function("script_sum_loop", |b| {
        b.iter(|| {
            let iterations = black_box(128);
            let args = [Value::Int(iterations)];
            let result = engine.call("SumLoop", &args).expect("script call succeeds");
            black_box(result);
        });
    });
}

criterion_group!(benches, bench_script_execution);
criterion_main!(benches);
