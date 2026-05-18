//! Criterion benchmarks for hypergraph builder ingest, freeze, and snapshot export.

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_hyper_bcsr::build::{HyperBuildError, HypergraphBuilder, export_bcsr_snapshot};

/// Benchmark hypergraph builder type.
type BenchBuilder = HypergraphBuilder<u32, u32, u32>;

/// Benchmark hypergraph build error type.
type BenchError = HyperBuildError<u32, u32, u32>;

/// Benchmark build result type.
type BenchResult = Result<BenchBuilder, BenchError>;

/// Builds a transition-like hypergraph with `vertices` vertices.
fn build_transitions(vertices: usize) -> BenchResult {
    let mut builder = HypergraphBuilder::<u32, u32, u32>::new();
    let mut ids = Vec::with_capacity(vertices);
    for _ in 0..vertices {
        ids.push(builder.add_vertex()?);
    }
    for index in 0..vertices.saturating_sub(2) {
        builder.add_hyperedge(&[ids[index]], &[ids[index + 1], ids[index + 2]])?;
    }
    Ok(builder)
}

/// Benchmarks hypergraph builder operations at 10k-vertex scale.
fn hyper_builder(c: &mut Criterion) {
    c.bench_function("hyper_builder_ingest_freeze_10k", |b| {
        b.iter(|| {
            let builder = build_transitions(10_000)
                .unwrap_or_else(|error| panic!("benchmark hypergraph should build: {error}"));
            builder
                .freeze()
                .unwrap_or_else(|error| panic!("benchmark hypergraph should freeze: {error}"))
        });
    });
    c.bench_function("hyper_builder_snapshot_10k", |b| {
        let graph = build_transitions(10_000)
            .unwrap_or_else(|error| panic!("benchmark hypergraph should build: {error}"))
            .freeze()
            .unwrap_or_else(|error| panic!("benchmark hypergraph should freeze: {error}"));
        b.iter(|| {
            export_bcsr_snapshot(&graph)
                .unwrap_or_else(|error| panic!("benchmark hypergraph should snapshot: {error}"))
        });
    });
}

criterion_group!(benches, hyper_builder);
criterion_main!(benches);
