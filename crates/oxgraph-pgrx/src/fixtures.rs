//! SQL/SPI fixture helpers for `#[pg_test]` and `#[pg_bench]`.

use oxgraph_postgres::{
    SnapshotRebuild,
    bench_fixture::{
        BENCH_AVG_DEGREE, BENCH_NODE_COUNT, BENCH_SEED, build_benchmark_fixture,
        build_oxgraph_bytes, chain_catalog, chain_edges, find_supernode,
    },
};
use pgrx::prelude::*;

/// Snapshot bytes for the 10k pgGraph-aligned benchmark fixture.
///
/// # Performance
///
/// `O(m)` for fixture edge count `m`.
pub fn benchmark_snapshot_bytes() -> Vec<u8> {
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    build_oxgraph_bytes(&fixture)
        .unwrap_or_else(|error| panic!("benchmark snapshot should build: {error}"))
}

/// Supernode seed in the 10k benchmark fixture.
pub fn benchmark_supernode() -> i32 {
    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    find_supernode(&fixture) as i32
}

/// Chain fixture snapshot bytes (three dense nodes).
pub fn chain_snapshot_bytes() -> Vec<u8> {
    let catalog = chain_catalog().unwrap_or_else(|error| panic!("chain catalog: {error}"));
    let edges = chain_edges();
    SnapshotRebuild::from_catalog_and_edges(&catalog, &edges, 1_700_000_000)
        .unwrap_or_else(|error| panic!("chain build: {error}"))
}

/// Runs arbitrary SQL in the current SPI connection.
pub fn run_sql(sql: &str) {
    Spi::run(sql).unwrap_or_else(|error| panic!("SQL failed: {error}: {sql}"));
}

/// Returns whether `to_regclass` resolves for `name`.
pub fn regclass_exists(name: &str) -> bool {
    let sql = format!("SELECT to_regclass('{name}') IS NOT NULL");
    Spi::get_one::<bool>(&sql).ok().flatten().unwrap_or(false)
}

/// Returns whether `to_regprocedure` resolves for `signature`.
pub fn regprocedure_exists(signature: &str) -> bool {
    let sql = format!("SELECT to_regprocedure('{signature}') IS NOT NULL");
    Spi::get_one::<bool>(&sql).ok().flatten().unwrap_or(false)
}

/// Loads snapshot bytes via SQL (`graph.graph_load_snapshot`).
///
/// # Performance
///
/// `O(n)` for snapshot byte length `n` (SPI copy).
pub fn sql_load_snapshot(bytes: &[u8]) -> bool {
    Spi::get_one_with_args::<bool>(
        "SELECT graph.graph_load_snapshot($1::bytea)",
        &[pgrx::datum::DatumWithOid::from(bytes)],
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// Outgoing traverse via SQL.
pub fn sql_traverse(seed: i32, limit: i32, direction: &str, max_depth: i32) -> Vec<i32> {
    let sql = format!("SELECT graph.graph_traverse({seed}, {limit}, '{direction}', {max_depth})");
    Spi::get_one::<Vec<i32>>(&sql)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Search via SQL.
pub fn sql_search(start: i32, end: i32, limit: i32) -> Vec<i32> {
    let sql = format!("SELECT graph.graph_search({start}, {end}, {limit})");
    Spi::get_one::<Vec<i32>>(&sql)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Clears the in-memory engine via SQL.
pub fn sql_reset() {
    let _ = Spi::get_one::<bool>("SELECT graph.graph_reset()");
}

/// Build from registered catalog via SQL.
pub fn sql_build(built_at_unix: i64) -> bool {
    let sql = format!("SELECT graph.graph_build({built_at_unix})");
    Spi::get_one::<bool>(&sql).ok().flatten().unwrap_or(false)
}

/// Load persisted snapshot via SQL.
pub fn sql_load_persisted() -> bool {
    Spi::get_one::<bool>("SELECT graph.graph_load_persisted()")
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// Sync reload via SQL.
pub fn sql_sync_reload() -> i32 {
    Spi::get_one::<i32>("SELECT graph.graph_sync_reload()")
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// Status JSON via SQL.
pub fn sql_status() -> String {
    Spi::get_one::<String>("SELECT graph.graph_status()")
        .ok()
        .flatten()
        .unwrap_or_else(|| "{\"loaded\":false}".to_string())
}

/// Attach sync trigger via SQL.
pub fn sql_attach_sync_trigger(edge_id: i32) -> bool {
    let sql = format!("SELECT graph.graph_attach_sync_trigger({edge_id})");
    Spi::get_one::<bool>(&sql).ok().flatten().unwrap_or(false)
}

/// Rebuild active engine from catalog via SQL.
pub fn sql_rebuild(built_at_unix: i64) -> bool {
    let sql = format!("SELECT graph.graph_rebuild({built_at_unix})");
    Spi::get_one::<bool>(&sql).ok().flatten().unwrap_or(false)
}

/// Sync health JSON via SQL.
pub fn sql_sync_health() -> String {
    Spi::get_one::<String>("SELECT graph.graph_sync_health()")
        .ok()
        .flatten()
        .unwrap_or_else(|| "{\"overlay_edges\":0}".to_string())
}

/// Returns the number of rows in `graph._sync_log`.
pub fn sql_sync_log_count() -> i64 {
    Spi::get_one("SELECT COUNT(*)::bigint FROM graph._sync_log")
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// Returns `built_at_unix` from the persisted snapshot store (0 when absent).
pub fn sql_snapshot_built_at() -> i64 {
    Spi::get_one(
        "SELECT COALESCE(built_at_unix, 0)::bigint FROM graph._snapshot_store WHERE id = 1",
    )
    .ok()
    .flatten()
    .unwrap_or(0)
}

/// Sets the session query freshness GUC (`0` base-only, `1` overlay-aware).
pub fn sql_set_query_freshness(mode: i32) {
    run_sql(&format!("SET oxgraph.query_freshness = {mode}"));
}

/// Returns whether traverse yields no nodes when the engine is not loaded.
pub fn sql_traverse_not_loaded(seed: i32) -> bool {
    sql_traverse(seed, 10, "out", -1).is_empty()
}

/// Multi-seed traverse via SQL.
pub fn sql_traverse_seeds(seeds: &[i32], limit: i32, direction: &str) -> Vec<i32> {
    let seed_list = seeds
        .iter()
        .map(|seed| seed.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT graph.graph_traverse_seeds(ARRAY[{seed_list}]::int[], {limit}, '{direction}')"
    );
    Spi::get_one::<Vec<i32>>(&sql)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Maintenance pass via SQL.
pub fn sql_maintenance() -> bool {
    Spi::get_one::<bool>("SELECT graph.graph_maintenance()")
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// Registers a node table via SQL (admin; raises on error).
pub fn sql_register_table(schema: &str, table: &str, pk_column: &str) -> i32 {
    let sql = format!("SELECT graph.graph_register_table('{schema}', '{table}', '{pk_column}')");
    Spi::get_one::<i32>(&sql).ok().flatten().unwrap_or(-1)
}

/// Registers an edge mapping via SQL (admin; raises on error).
pub fn sql_register_edge(
    source_table_id: i32,
    target_table_id: i32,
    source_column: &str,
    target_column: &str,
    schema: &str,
    table: &str,
) -> i32 {
    let sql = format!(
        "SELECT graph.graph_register_edge({source_table_id}, {target_table_id}, \
         '{source_column}', '{target_column}', '{schema}', '{table}')"
    );
    Spi::get_one::<i32>(&sql).ok().flatten().unwrap_or(-1)
}

/// Registers a filter column via SQL (admin; raises on error).
pub fn sql_register_filter_column(table_id: i32, column_name: &str) -> bool {
    let sql = format!("SELECT graph.graph_register_filter_column({table_id}, '{column_name}')");
    Spi::get_one::<bool>(&sql).ok().flatten().unwrap_or(false)
}

/// Encodes `(table_id, primary_key)` the same way as the sync trigger.
pub fn node_key_i64(table_id: u32, primary_key: u64) -> i64 {
    let encoded = ((table_id as u64) << 32) | primary_key;
    i64::try_from(encoded).unwrap_or_else(|_| panic!("node key does not fit i64: {encoded}"))
}

/// Returns the latest sync-log row as `(action_type, arg0, arg1)`.
pub fn latest_sync_log_row() -> (i16, Option<i64>, Option<i64>) {
    let action: Option<i16> =
        Spi::get_one("SELECT action_type FROM graph._sync_log ORDER BY sequence DESC LIMIT 1")
            .ok()
            .flatten();
    let arg0: Option<i64> =
        Spi::get_one("SELECT arg0 FROM graph._sync_log ORDER BY sequence DESC LIMIT 1")
            .ok()
            .flatten();
    let arg1: Option<i64> =
        Spi::get_one("SELECT arg1 FROM graph._sync_log ORDER BY sequence DESC LIMIT 1")
            .ok()
            .flatten();
    (action.unwrap_or(-1), arg0, arg1)
}

/// Executes `statement` and returns the SQL error message when one is raised.
pub fn sql_expect_exception_message(statement: &str) -> Option<String> {
    run_sql(
        "CREATE TEMP TABLE IF NOT EXISTS oxgraph_expect_error (raised bool NOT NULL, message text)",
    );
    run_sql("DELETE FROM oxgraph_expect_error");
    let block = format!(
        r#"
DO $$
DECLARE msg text;
BEGIN
  BEGIN
    {statement};
    INSERT INTO oxgraph_expect_error VALUES (false, NULL);
  EXCEPTION WHEN OTHERS THEN
    GET STACKED DIAGNOSTICS msg = MESSAGE_TEXT;
    INSERT INTO oxgraph_expect_error VALUES (true, msg);
  END;
END $$;
"#
    );
    run_sql(&block);
    let raised: bool = Spi::get_one("SELECT raised FROM oxgraph_expect_error")
        .ok()
        .flatten()
        .unwrap_or(false);
    if !raised {
        return None;
    }
    Spi::get_one("SELECT message FROM oxgraph_expect_error")
        .ok()
        .flatten()
}

/// Returns whether `statement` raises a SQL exception when executed in a DO block.
pub fn sql_expect_exception(statement: &str) -> bool {
    sql_expect_exception_message(statement).is_some()
}

/// Returns whether `sql` fails at the SPI boundary.
pub fn sql_fails(sql: &str) -> bool {
    let _ = Spi::run("SAVEPOINT oxgraph_expect_fail");
    let failed = Spi::run(sql).is_err();
    let _ = Spi::run("ROLLBACK TO SAVEPOINT oxgraph_expect_fail");
    let _ = Spi::run("RELEASE SAVEPOINT oxgraph_expect_fail");
    failed
}

/// Seeds chain relational tables and catalog registration.
pub fn seed_chain_relational() {
    run_sql("CREATE TABLE IF NOT EXISTS public.nodes (id bigint PRIMARY KEY)");
    run_sql("CREATE TABLE IF NOT EXISTS public.edges (src bigint NOT NULL, dst bigint NOT NULL)");
    run_sql("TRUNCATE public.nodes, public.edges");
    run_sql("INSERT INTO public.nodes (id) VALUES (1), (2), (3)");
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 2), (2, 3)");
    run_sql(
        "TRUNCATE graph._registered_filter_columns, graph._registered_edges, graph._registered_tables",
    );
    run_sql(
        "INSERT INTO graph._registered_tables (table_id, schema_name, table_name, primary_key_column) \
         VALUES (1, 'public', 'nodes', 'id')",
    );
    run_sql(
        "INSERT INTO graph._registered_edges (
            edge_id, source_table_id, target_table_id,
            source_column, target_column, schema_name, table_name
         ) VALUES (1, 1, 1, 'src', 'dst', 'public', 'edges')",
    );
}

/// Seeds 10k relational tables from the benchmark fixture (for `graph_build` bench).
///
/// # Performance
///
/// `O(m)` inserts for `m` raw edges.
pub fn seed_benchmark_relational() {
    run_sql("CREATE TABLE IF NOT EXISTS public.bench_nodes (id bigint PRIMARY KEY)");
    run_sql(
        "CREATE TABLE IF NOT EXISTS public.bench_edges (src bigint NOT NULL, dst bigint NOT NULL)",
    );
    run_sql(
        "TRUNCATE graph._registered_filter_columns, graph._registered_edges, graph._registered_tables",
    );
    run_sql("TRUNCATE public.bench_nodes, public.bench_edges");

    run_sql(
        "INSERT INTO graph._registered_tables (table_id, schema_name, table_name, primary_key_column) \
         VALUES (1, 'public', 'bench_nodes', 'id')",
    );
    run_sql(
        "INSERT INTO graph._registered_edges (
            edge_id, source_table_id, target_table_id,
            source_column, target_column, schema_name, table_name
         ) VALUES (1, 1, 1, 'src', 'dst', 'public', 'bench_edges')",
    );

    let fixture = build_benchmark_fixture(BENCH_NODE_COUNT, BENCH_AVG_DEGREE, BENCH_SEED);
    for id in 0..fixture.node_count {
        let sql = format!("INSERT INTO public.bench_nodes (id) VALUES ({id})");
        run_sql(&sql);
    }
    for edge in &fixture.raw_edges {
        let sql = format!(
            "INSERT INTO public.bench_edges (src, dst) VALUES ({}, {})",
            edge.source, edge.target
        );
        run_sql(&sql);
    }
}

/// Setup for 10k SQL traverse benches: load snapshot into session engine.
pub fn setup_pg_10k_loaded() {
    let bytes = benchmark_snapshot_bytes();
    if !sql_load_snapshot(&bytes) {
        panic!("setup_pg_10k_loaded: graph_load_snapshot failed");
    }
}

/// Setup for 10k relational build bench.
pub fn setup_pg_relational_10k() {
    seed_benchmark_relational();
}
