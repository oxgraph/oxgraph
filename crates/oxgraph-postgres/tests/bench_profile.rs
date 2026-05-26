//! Visit-count and latency probe for `compare_pggraph` fixture (release-only diagnostics).

use core::num::{NonZeroU32, NonZeroUsize};
use std::time::Instant;

use oxgraph_postgres::{
    EngineBuilder, TraversalDirection, TraverseLimits,
    bench_fixture::{
        BENCH_AVG_DEGREE, BENCH_NODE_COUNT, BENCH_SEED, build_benchmark_fixture,
        build_oxgraph_bytes, find_supernode, out_degrees,
    },
};

fn build_engine() -> Result<oxgraph_postgres::Engine, oxgraph_postgres::PostgresGraphError> {
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    let bytes = build_oxgraph_bytes(&fixture)?;
    EngineBuilder::new().snapshot_owned(bytes).build()
}

/// Prints parallel vs unique out-neighbors for the benchmark supernode (diagnostic).
#[test]
fn supernode_parallel_vs_unique_out_degree() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    use std::collections::HashSet;

    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    let supernode = find_supernode(&fixture);
    let parallel_raw = out_degrees(&fixture)[supernode as usize];
    let mut unique_raw = HashSet::new();
    for edge in &fixture.raw_edges {
        if edge.source == supernode {
            unique_raw.insert(edge.target);
        }
    }
    let engine = build_engine()?;
    let csr_parallel = engine.forward().successors(supernode).count();
    eprintln!(
        "supernode={supernode}: raw_parallel_out={parallel_raw} unique_raw_targets={} csr_out_slots={csr_parallel}",
        unique_raw.len()
    );
    Ok(())
}

#[test]
fn profile_supernode_depth_counts_and_latency() -> Result<(), oxgraph_postgres::PostgresGraphError>
{
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    let supernode = find_supernode(&fixture);
    let degree = out_degrees(&fixture)[supernode as usize];
    eprintln!(
        "fixture: nodes=10000 edges={} supernode={supernode} out_degree={degree}",
        fixture.raw_edges.len()
    );

    let mut engine = build_engine()?;
    let cap = NonZeroUsize::new(engine.forward().node_count()).unwrap_or(NonZeroUsize::MIN);

    let mut count_d1 = 0_usize;
    let mut count_d3 = 0_usize;
    let mut elapsed_d3_us = 0_u128;
    for depth in [1_u32, 3_u32] {
        let limits = TraverseLimits {
            result_limit: cap,
            max_depth: NonZeroU32::new(depth),
        };
        let start = Instant::now();
        let count = engine.visited_count(supernode, limits, TraversalDirection::Out)?;
        let elapsed = start.elapsed();
        eprintln!(
            "depth={depth}: visited_count={count} elapsed_us={}",
            elapsed.as_micros()
        );
        if depth == 1 {
            count_d1 = count;
        } else {
            count_d3 = count;
            elapsed_d3_us = elapsed.as_micros();
        }
    }
    assert_eq!(
        count_d1, count_d3,
        "hub fixture: depth-1 and depth-3 should visit the same component"
    );
    assert!(
        cfg!(debug_assertions) || elapsed_d3_us < 10_000,
        "depth-3 probe should stay well below pre-fix ~366ms (got {elapsed_d3_us}µs)"
    );

    // Warmed second iteration (scratch reuse)
    let limits = TraverseLimits {
        result_limit: cap,
        max_depth: NonZeroU32::new(1),
    };
    let start = Instant::now();
    let count = engine.visited_count(supernode, limits, TraversalDirection::Out)?;
    eprintln!(
        "depth=1 (2nd call): visited_count={count} elapsed_us={}",
        start.elapsed().as_micros()
    );

    Ok(())
}

/// Repeated d1 calls to measure scratch reuse overhead (ignored in default `cargo test`).
#[test]
#[ignore = "manual diagnostic: 500-iteration latency sweep"]
fn profile_supernode_depth_one_repeated() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    let supernode = find_supernode(&fixture);
    let mut engine = build_engine()?;
    let cap = NonZeroUsize::new(engine.forward().node_count()).unwrap_or(NonZeroUsize::MIN);
    let limits_d1 = TraverseLimits {
        result_limit: cap,
        max_depth: NonZeroU32::new(1),
    };
    let iters = 500_u32;
    let mut total_us = 0_u128;
    for _ in 0..iters {
        let start = Instant::now();
        let _ = engine.visited_count(supernode, limits_d1, TraversalDirection::Out)?;
        total_us += start.elapsed().as_micros();
    }
    eprintln!(
        "depth=1 x{iters}: total_us={total_us} avg_us={}",
        total_us / u128::from(iters)
    );
    Ok(())
}

/// Compares default `OverlayAware` vs `BaseOnly` d1 traverse (ignored in default CI).
#[test]
#[ignore = "manual diagnostic: freshness mode comparison"]
fn profile_freshness_mode_d1_latency() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    use oxgraph_postgres::{Config, QueryFreshness};

    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    let supernode = find_supernode(&fixture);
    let mut engine = build_engine()?;
    let cap = NonZeroUsize::new(engine.forward().node_count()).unwrap_or(NonZeroUsize::MIN);
    let limits_d1 = TraverseLimits {
        result_limit: cap,
        max_depth: NonZeroU32::new(1),
    };
    let iters = 300_u32;

    let start = Instant::now();
    for _ in 0..iters {
        let _ = engine.visited_count(supernode, limits_d1, TraversalDirection::Out)?;
    }
    let overlay_aware_avg = start.elapsed().as_micros() / u128::from(iters);

    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    let start = Instant::now();
    for _ in 0..iters {
        let _ = engine.visited_count(supernode, limits_d1, TraversalDirection::Out)?;
    }
    let base_only_avg = start.elapsed().as_micros() / u128::from(iters);

    eprintln!(
        "supernode out_degree={} d1_avg_us overlay_aware={overlay_aware_avg} base_only={base_only_avg}",
        out_degrees(&fixture)[supernode as usize]
    );
    Ok(())
}
