use clonk_script::{Engine, Value};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

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

global func SumValues(values)
{
    var acc = 0;
    for (var value in values)
    {
        acc += value;
    }
    return acc;
}

global func Method(value) { return value + 1; }
global func CallMethod(target) { return target->Method(41); }
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

    let args = [Value::Array((0..8).map(Value::Int).collect())];
    assert_eq!(engine.call("SumValues", &args).unwrap(), Value::Int(28));
    c.bench_function("script_ast_sum_values", |b| {
        b.iter(|| {
            black_box(engine.call("SumValues", black_box(&args)).unwrap());
        });
    });

    let args = [Value::Object(1)];
    assert_eq!(engine.call("CallMethod", &args).unwrap(), Value::Int(42));
    c.bench_function("script_value_method_call", |b| {
        b.iter(|| black_box(engine.call("CallMethod", black_box(&args)).unwrap()));
    });
}

criterion_group!(benches, bench_script_execution);
criterion_main!(benches);
