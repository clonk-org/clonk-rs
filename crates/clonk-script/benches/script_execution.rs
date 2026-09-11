use clonk_script::{new_global_variables, value_cell, Engine, Value};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
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

const SHARED_GLOBAL_SCRIPT: &str = r#"
global func Method(value) { return value + 1; }
global func NestedCalls(iterations)
{
    var total = 0;
    for (var index = 0; index < iterations; ++index)
        total += Method(index);
    return total;
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

    let mut group = c.benchmark_group("script_nested_calls_with_shared_globals");
    for global_count in [0, 128, 512] {
        let globals = new_global_variables();
        for index in 0..global_count {
            // Include live references among ordinary scalar globals so each
            // call discovers both kinds without weakening AssignRemoval.
            let value = if index % 16 == 0 {
                Value::Object(index as u64 + 1)
            } else {
                Value::Int(index)
            };
            globals
                .borrow_mut()
                .insert(format!("global{index}"), value_cell(value));
        }
        let mut engine = Engine::new();
        engine.set_global_variables(globals);
        engine.load_script(SHARED_GLOBAL_SCRIPT).unwrap();
        let args = [Value::Int(32)];
        assert_eq!(engine.call("NestedCalls", &args).unwrap(), Value::Int(528));
        group.bench_with_input(
            BenchmarkId::from_parameter(global_count),
            &args,
            |b, args| {
                b.iter(|| black_box(engine.call("NestedCalls", black_box(args)).unwrap()));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_script_execution);
criterion_main!(benches);
