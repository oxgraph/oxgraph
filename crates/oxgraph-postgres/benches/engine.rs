//! Engine open benchmarks for `oxgraph-postgres`.
//!
//! perf: open snapshot + dual topology attach is `O(n + m)` for `n <= 10k`.

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_postgres::{DualTopologySnapshot, EngineBuilder, PostgresGraphError};

/// Traverse limit used by the 10k-node benchmark.
const LIMIT_10K: core::num::NonZeroUsize = core::num::NonZeroUsize::new(10_000).unwrap();

/// Builds a chain snapshot fixture with inbound CSC sections.
fn build_sample_bytes(node_count: u32) -> Result<Vec<u8>, PostgresGraphError> {
    let edges: Vec<(u32, u32)> = (0..node_count.saturating_sub(1))
        .map(|source| (source, source + 1))
        .collect();
    DualTopologySnapshot::from_dense_u32_edges(&edges, 0)
}

/// Benchmarks [`EngineBuilder::build`] at 10k-node scale.
fn bench_open_snapshot(c: &mut Criterion) {
    let bytes = build_sample_bytes(10_000)
        .unwrap_or_else(|error| panic!("benchmark fixture should build: {error}"));
    c.bench_function("engine_builder_build_10k", |b| {
        b.iter(|| {
            EngineBuilder::new()
                .snapshot_owned(bytes.clone())
                .build()
                .unwrap_or_else(|error| panic!("engine should open: {error}"))
        });
    });
}

/// Benchmarks single-seed traverse at 10k-node scale.
fn bench_traverse_10k(c: &mut Criterion) {
    use oxgraph_postgres::{TraversalDirection, TraverseLimits};

    let bytes = build_sample_bytes(10_000)
        .unwrap_or_else(|error| panic!("benchmark fixture should build: {error}"));
    let mut engine = EngineBuilder::new()
        .snapshot_owned(bytes)
        .build()
        .unwrap_or_else(|error| panic!("engine should open: {error}"));
    let limits = TraverseLimits::bounded(LIMIT_10K);
    c.bench_function("engine_traverse_out_10k", |b| {
        b.iter(|| {
            engine
                .traverse(0, limits, TraversalDirection::Out)
                .unwrap_or_else(|error| panic!("traverse should succeed: {error}"))
        });
    });
}

criterion_group!(benches, bench_open_snapshot, bench_traverse_10k);
criterion_main!(benches);
