//! HNSW performance benchmarks.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_basic(c: &mut Criterion) {
    c.bench_function("basic_bench", |b| {
        b.iter(|| {
            // Placeholder benchmark
            black_box(1 + 1)
        })
    });
}

criterion_group!(benches, bench_basic);
criterion_main!(benches);
