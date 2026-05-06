//! Criterion benchmarks for graph builder ingest, freeze, and snapshot export.

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_graph_build::{GraphBuildError, GraphBuilder};

/// Builds a chain graph with `nodes` nodes.
fn build_chain(nodes: usize) -> Result<GraphBuilder<(), ()>, GraphBuildError> {
    let mut builder = GraphBuilder::new((), ());
    let mut ids = Vec::with_capacity(nodes);
    for _ in 0..nodes {
        ids.push(builder.add_node()?);
    }
    for pair in ids.windows(2) {
        builder.add_edge(pair[0], pair[1])?;
    }
    Ok(builder)
}

/// Benchmarks graph builder operations at 10k-node scale.
fn graph_builder(c: &mut Criterion) {
    c.bench_function("graph_builder_ingest_freeze_10k", |b| {
        b.iter(|| {
            let builder = build_chain(10_000)
                .unwrap_or_else(|error| panic!("benchmark graph should build: {error}"));
            builder
                .freeze()
                .unwrap_or_else(|error| panic!("benchmark graph should freeze: {error}"))
        });
    });
    c.bench_function("graph_builder_snapshot_10k", |b| {
        let graph = build_chain(10_000)
            .unwrap_or_else(|error| panic!("benchmark graph should build: {error}"))
            .freeze()
            .unwrap_or_else(|error| panic!("benchmark graph should freeze: {error}"));
        b.iter(|| {
            graph
                .to_csr_snapshot()
                .unwrap_or_else(|error| panic!("benchmark graph should snapshot: {error}"))
        });
    });
}

criterion_group!(benches, graph_builder);
criterion_main!(benches);
