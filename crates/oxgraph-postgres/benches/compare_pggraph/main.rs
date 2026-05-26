//! Engine benchmarks comparable to pgGraph `bfs_bench` (10k nodes, avg degree ~3).
//!
//! Uses the shared [`oxgraph_postgres::bench_fixture`] module. Compare:
//! - CSR/index build: oxgraph artifact export vs pgGraph `graph_construction/build`
//! - Engine open: oxgraph `EngineBuilder::build` (oxgraph-only; pgGraph keeps stores in memory)
//! - BFS (pgGraph-paired): [`traverse_core_out`] — collects visited node ids within depth (same
//!   class of work as pgGraph `bfs_execute`, which builds visited/parent/depth state). Count and
//!   collect share one kernel; there is no separate count-only bench lane.
//!
//! perf: fixture generation is `O(m)`; build/open/BFS are documented per benchmark name.

use core::num::{NonZeroU32, NonZeroUsize};
use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use oxgraph_postgres::{
    Config, EngineBuilder, QueryFreshness, TraversalDirection, TraverseLimits,
    bench_fixture::{
        BENCH_AVG_DEGREE, BENCH_LABEL_10K, BENCH_NODE_COUNT, BENCH_SEED, build_benchmark_fixture,
        build_oxgraph_bytes, find_supernode,
    },
};

/// Depth-limited outgoing BFS that materializes visited node ids (pgGraph `bfs_execute` class).
fn bfs_depth_limited_collect(
    engine: &mut oxgraph_postgres::Engine,
    seed: u32,
    max_depth: u32,
) -> usize {
    let cap = NonZeroUsize::new(engine.forward().node_count()).unwrap_or(NonZeroUsize::MIN);
    let limits = TraverseLimits {
        result_limit: cap,
        max_depth: NonZeroU32::new(max_depth),
    };
    engine
        .traverse(seed, limits, TraversalDirection::Out)
        .map_or(0, |nodes| nodes.len())
}

/// CSR/index build from fixture — comparable to pgGraph `graph_construction/build/10k`.
fn bench_graph_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_graph_construction");
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    group.bench_function(BenchmarkId::new("build", BENCH_LABEL_10K), |b| {
        b.iter_batched(
            || fixture.clone(),
            |fixture| {
                black_box(
                    build_oxgraph_bytes(&fixture)
                        .unwrap_or_else(|error| panic!("oxgraph indexes should build: {error}")),
                )
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

/// Engine open from pre-built artifact — oxgraph-only (validates + opens dual topology).
fn bench_engine_open(c: &mut Criterion) {
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    let bytes = build_oxgraph_bytes(&fixture)
        .unwrap_or_else(|error| panic!("fixture artifact should build: {error}"));

    c.bench_function("engine_oxgraph_engine_open/10k", |b| {
        b.iter(|| {
            black_box(
                EngineBuilder::new()
                    .snapshot_owned(bytes.clone())
                    .build()
                    .unwrap_or_else(|error| panic!("engine should open: {error}")),
            )
        });
    });
}

/// BFS traverse — criterion group `engine_bfs_traverse/*` for bench script pairing with pgGraph.
fn bench_bfs_traverse(c: &mut Criterion) {
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    let bytes = build_oxgraph_bytes(&fixture)
        .unwrap_or_else(|error| panic!("fixture artifact should build: {error}"));
    let mut engine = EngineBuilder::new()
        .snapshot_owned(bytes)
        .build()
        .unwrap_or_else(|error| panic!("engine should open: {error}"));
    engine
        .set_config(Config {
            query_freshness: QueryFreshness::BaseOnly,
            ..Config::default()
        })
        .unwrap_or_else(|error| panic!("engine config should apply: {error}"));
    let supernode = find_supernode(&fixture);

    let mut group = c.benchmark_group("engine_bfs_traverse");
    group.sample_size(50);

    group.bench_function(BenchmarkId::new("d1_supernode", BENCH_LABEL_10K), |b| {
        b.iter(|| {
            let seed = black_box(supernode);
            let depth = black_box(1_u32);
            black_box(bfs_depth_limited_collect(
                black_box(&mut engine),
                seed,
                depth,
            ))
        });
    });
    group.bench_function(BenchmarkId::new("d3_supernode", BENCH_LABEL_10K), |b| {
        b.iter(|| {
            let seed = black_box(supernode);
            let depth = black_box(3_u32);
            black_box(bfs_depth_limited_collect(
                black_box(&mut engine),
                seed,
                depth,
            ))
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_graph_construction,
    bench_engine_open,
    bench_bfs_traverse
);
criterion_main!(benches);
