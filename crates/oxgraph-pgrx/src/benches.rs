// In-Postgres extension benchmarks (`cargo pgrx bench`).

use pgrx::prelude::*;
use pgrx_bench::{Bencher, black_box};

use crate::fixtures::{
    benchmark_supernode, run_sql, seed_benchmark_relational, setup_pg_10k_loaded,
    setup_pg_relational_10k, sql_build, sql_sync_reload, sql_traverse,
};
use crate::spi;

/// SQL traverse depth-1 on 10k supernode fixture.
///
/// perf: `O(degree)` at depth 1 for the loaded 10k graph.
#[pg_bench(setup = setup_pg_10k_loaded)]
fn bench_sql_traverse_d1_10k(b: &mut Bencher) {
    let seed = benchmark_supernode();
    b.iter(move || {
        black_box(sql_traverse(seed, 10_000, "out", 1));
    });
}

/// SQL traverse depth-3 on 10k supernode fixture.
///
/// perf: `O(visited)` with result cap for the loaded 10k graph.
#[pg_bench(setup = setup_pg_10k_loaded)]
fn bench_sql_traverse_d3_10k(b: &mut Bencher) {
    let seed = benchmark_supernode();
    b.iter(move || {
        black_box(sql_traverse(seed, 10_000, "out", 3));
    });
}

/// Full catalog scan + edge table scan + build via SQL.
///
/// perf: `O(m)` for `m` edges in the 10k relational seed.
#[pg_bench(setup = setup_pg_relational_10k)]
fn bench_sql_graph_build_10k(b: &mut Bencher) {
    b.iter(|| {
        black_box(sql_build(0));
    });
}

/// SPI catalog read only.
///
/// perf: `O(catalog rows)`.
#[pg_bench(setup = setup_pg_relational_10k)]
fn bench_spi_catalog_scan(b: &mut Bencher) {
    b.iter(|| {
        black_box(spi::load_catalog_from_spi());
    });
}

/// Sync log replay via SQL after trigger-fed inserts.
///
/// perf: `O(sync rows)` for rows appended in setup.
#[pg_bench(setup = setup_pg_sync_bench)]
fn bench_sql_sync_reload(b: &mut Bencher) {
    b.iter(|| {
        black_box(sql_sync_reload());
    });
}

/// Direct `graph_traverse` without SQL wrapper (overhead baseline).
///
/// perf: `O(degree)` at depth 1; compare to [`bench_sql_traverse_d1_10k`].
#[pg_bench(setup = setup_pg_10k_loaded)]
fn bench_direct_traverse_d1_10k(b: &mut Bencher) {
    use crate::graph_traverse;

    let seed = benchmark_supernode();
    b.iter(move || {
        black_box(graph_traverse(seed, 10_000, "out", 1));
    });
}

/// Seeds relational 10k tables, builds once, attaches trigger, inserts one edge.
fn setup_pg_sync_bench() {
    setup_pg_relational_10k();
    if !sql_build(0) {
        panic!("setup_pg_sync_bench: graph_build failed");
    }
    if !crate::fixtures::sql_attach_sync_trigger(1) {
        panic!("setup_pg_sync_bench: attach trigger failed");
    }
    run_sql("INSERT INTO public.bench_edges (src, dst) VALUES (0, 1)");
    let _ = sql_sync_reload();
}
