use criterion::{criterion_group, criterion_main, Criterion};

fn bench_collect(c: &mut Criterion) {
    let rows: Vec<Result<i32, String>> = (0..1000).map(|i| Ok(i)).collect();

    c.bench_function("loop_push", |b| {
        b.iter(|| {
            let mut vec = Vec::new();
            // Since `rows` is fixed size, this represents `for row in rows` as per code
            for row in rows.clone() {
                vec.push(row.unwrap());
            }
            vec
        })
    });

    c.bench_function("collect", |b| {
        b.iter(|| {
            let res: Result<Vec<i32>, String> = rows.clone().into_iter().collect();
            res.unwrap()
        })
    });
}

criterion_group!(benches, bench_collect);
criterion_main!(benches);
