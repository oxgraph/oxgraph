// In-process extension integration tests (`cargo pgrx test`).

use pgrx::prelude::*;

use crate::fixtures::{
    chain_snapshot_bytes, node_key_i64, regclass_exists, regprocedure_exists, run_sql,
    seed_chain_relational, sql_attach_sync_trigger, sql_build,
    sql_expect_exception_message, sql_load_persisted, sql_load_snapshot, latest_sync_log_row,
    sql_maintenance, sql_register_edge, sql_register_filter_column, sql_register_table,
    sql_rebuild, sql_reset, sql_search, sql_set_query_freshness, sql_snapshot_built_at,
    sql_status, sql_sync_health, sql_sync_log_count, sql_sync_reload, sql_traverse,
    sql_traverse_not_loaded, sql_traverse_seeds,
};

/// Bootstrap DDL from [`extension_sql_file`] is present after `CREATE EXTENSION`.
#[pg_test]
fn extension_bootstrap_smoke() {
    assert!(regclass_exists("graph._registered_tables"));
    assert!(regclass_exists("graph._registered_edges"));
    assert!(regclass_exists("graph._registered_filter_columns"));
    assert!(regclass_exists("graph._sync_log"));
    assert!(regclass_exists("graph._snapshot_store"));
    assert!(regprocedure_exists("graph._edge_change_sync_trigger()"));
}

/// SPI connectivity smoke test.
#[pg_test]
fn extension_sql_smoke() {
    let result = Spi::get_one::<i32>("SELECT 1");
    assert_eq!(result, Ok(Some(1)));
}

/// Load snapshot and query via `SELECT graph.*` (SQL/SPI path).
#[pg_test]
fn sql_load_traverse_search_roundtrip() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
    let matches = sql_search(1, 1, 10);
    assert_eq!(matches, vec![1]);
}

/// Build from registered catalog tables via SQL.
#[pg_test]
fn sql_build_from_catalog() {
    seed_chain_relational();
    assert!(sql_build(0));
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Persisted snapshot reload and status JSON.
#[pg_test]
fn sql_rebuild_and_load_persisted() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_load_persisted());
    let status = sql_status();
    assert!(status.contains("\"loaded\":true"));
}

/// Session GUC mirrors are visible to graph entrypoints after SQL `SET`.
#[pg_test]
fn guc_sql_set_visible_to_graph_queries() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    run_sql("SET oxgraph.search_limit = 2");
    run_sql("SET oxgraph.traverse_limit = 2");
    let matches = sql_search(0, 2, 100);
    assert_eq!(matches, vec![0, 1]);
    let out = sql_traverse(0, 100, "out", -1);
    assert_eq!(out.len(), 2);
}

/// Session GUC `oxgraph.search_limit` caps engine search output.
#[pg_test]
fn guc_search_limit_caps_results() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    run_sql("SET oxgraph.search_limit = 2");
    let matches = sql_search(0, 2, 100);
    assert_eq!(matches, vec![0, 1]);
}

/// Session GUC `oxgraph.traverse_limit` caps engine traverse output.
#[pg_test]
fn guc_traverse_limit_caps_results() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    run_sql("SET oxgraph.traverse_limit = 2");
    let out = sql_traverse(0, 100, "out", -1);
    assert_eq!(out.len(), 2);
}

/// Session GUC `oxgraph.traverse_limit` is registered and accepts `SET`.
#[pg_test]
fn guc_traverse_limit() {
    run_sql("SET oxgraph.traverse_limit = 2");
    let guc: i32 = Spi::get_one("SELECT current_setting('oxgraph.traverse_limit')::int")
        .ok()
        .flatten()
        .unwrap_or(0);
    assert_eq!(guc, 2);
}

/// Session GUC `oxgraph.search_limit` is registered and accepts `SET`.
#[pg_test]
fn guc_search_limit() {
    run_sql("SET oxgraph.search_limit = 3");
    let guc: i32 = Spi::get_one("SELECT current_setting('oxgraph.search_limit')::int")
        .ok()
        .flatten()
        .unwrap_or(0);
    assert_eq!(guc, 3);
}

/// SQL `limit` argument caps traverse output independently of GUC.
#[pg_test]
fn sql_traverse_limit_argument() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    let out = sql_traverse(0, 2, "out", -1);
    assert_eq!(out.len(), 2);
}

/// Non-superuser with reader GUC cannot build (admin returns `false`).
#[pg_test]
fn acl_reader_denied_build() {
    sql_reset();
    seed_chain_relational();
    run_sql("CREATE ROLE oxgraph_test_reader");
    run_sql("GRANT USAGE ON SCHEMA graph TO oxgraph_test_reader");
    run_sql("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA graph TO oxgraph_test_reader");
    run_sql("GRANT ALL ON ALL TABLES IN SCHEMA graph TO oxgraph_test_reader");
    run_sql("GRANT ALL ON ALL TABLES IN SCHEMA public TO oxgraph_test_reader");
    run_sql("SET ROLE oxgraph_test_reader");
    run_sql("SET oxgraph.graph_role = 0");
    assert!(!sql_build(0));
    run_sql("RESET ROLE");
}

/// Reader role cannot register tables (discovery raises SQL errors).
#[pg_test]
fn acl_reader_denied_register_table() {
    sql_reset();
    run_sql("CREATE ROLE oxgraph_test_reader");
    run_sql("GRANT USAGE ON SCHEMA graph TO oxgraph_test_reader");
    run_sql("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA graph TO oxgraph_test_reader");
    run_sql("SET ROLE oxgraph_test_reader");
    run_sql("SET oxgraph.graph_role = 0");
    let message = sql_expect_exception_message(
        "PERFORM graph.graph_register_table('public', 'nodes', 'id')",
    );
    assert!(message.is_some_and(|msg| msg.contains("access denied")));
    run_sql("RESET ROLE");
}

/// Edge sync trigger writes log rows; reload applies overlay shortcut.
#[pg_test]
fn sync_trigger_insert_and_reload() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    let (action_type, arg0, arg1) = latest_sync_log_row();
    assert_eq!(action_type, 1);
    assert_eq!(arg0, Some(node_key_i64(1, 1)));
    assert_eq!(arg1, Some(node_key_i64(1, 3)));
    let count: i64 = Spi::get_one("SELECT COUNT(*)::bigint FROM graph._sync_log")
        .ok()
        .flatten()
        .unwrap_or(0);
    assert!(count >= 1);
    let applied = sql_sync_reload();
    assert!(applied >= 1);
    let depth_one = sql_traverse(0, 10, "out", 1);
    assert!(depth_one.contains(&2), "overlay edge 1->3 should reach dense node 2");
    assert_eq!(sql_sync_health(), "{\"overlay_edges\":1,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}");
}

/// Delete sync removes a previously inserted overlay edge.
#[pg_test]
fn sync_trigger_delete_and_reload() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_reload() >= 1);
    run_sql("DELETE FROM public.edges WHERE src = 1 AND dst = 3");
    let (action_type, arg0, arg1) = latest_sync_log_row();
    assert_eq!(action_type, 5);
    assert_eq!(arg0, Some(node_key_i64(1, 1)));
    assert_eq!(arg1, Some(node_key_i64(1, 3)));
    assert!(sql_sync_reload() >= 1);
    let depth_one = sql_traverse(0, 10, "out", 1);
    assert_eq!(depth_one, vec![0, 1]);
    assert_eq!(sql_sync_health(), "{\"overlay_edges\":0,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}");
}

/// Inbound traverse reaches predecessors on the chain fixture.
#[pg_test]
fn sql_inbound_traverse() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    let inbound = sql_traverse(2, 10, "in", -1);
    assert_eq!(inbound, vec![2, 1, 0]);
}

/// Multi-seed traverse merges distinct reachable nodes.
#[pg_test]
fn sql_multi_seed_traverse() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    let out = sql_traverse_seeds(&[0, 2], 10, "out");
    assert!(out.contains(&0));
    assert!(out.contains(&1));
    assert!(out.contains(&2));
}

/// Catalog registration APIs persist rows and support rebuild.
#[pg_test]
fn sql_register_catalog_and_rebuild() {
    sql_reset();
    run_sql("CREATE TABLE IF NOT EXISTS public.nodes (id bigint PRIMARY KEY)");
    run_sql("CREATE TABLE IF NOT EXISTS public.edges (src bigint NOT NULL, dst bigint NOT NULL)");
    run_sql("TRUNCATE public.nodes, public.edges");
    run_sql("INSERT INTO public.nodes (id) VALUES (1), (2), (3)");
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 2), (2, 3)");
    run_sql(
        "TRUNCATE graph._registered_filter_columns, graph._registered_edges, graph._registered_tables",
    );
    let table_id = sql_register_table("public", "nodes", "id");
    assert_eq!(table_id, 1);
    let edge_id = sql_register_edge(1, 1, "src", "dst", "public", "edges");
    assert_eq!(edge_id, 1);
    assert!(sql_register_filter_column(1, "id"));
    let count: i64 = Spi::get_one(
        "SELECT COUNT(*)::bigint FROM graph._registered_filter_columns WHERE table_id = 1",
    )
    .ok()
    .flatten()
    .unwrap_or(0);
    assert_eq!(count, 1);
    assert!(sql_build(0));
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Rebuild refreshes a loaded engine from catalog scans.
#[pg_test]
fn sql_rebuild_from_loaded_engine() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_rebuild(0));
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Duplicate table registration raises a SQL error.
#[pg_test]
fn sql_register_table_duplicate_errors() {
    sql_reset();
    run_sql(
        "TRUNCATE graph._registered_filter_columns, graph._registered_edges, graph._registered_tables",
    );
    assert_eq!(sql_register_table("public", "nodes", "id"), 1);
    let message = sql_expect_exception_message(
        "PERFORM graph.graph_register_table('public', 'nodes', 'id')",
    );
    assert!(message.is_some_and(|msg| msg.contains("duplicate table name")));
}

/// Sync trigger rejects primary keys outside the NodeKey payload range.
#[pg_test]
fn sync_trigger_rejects_out_of_range_primary_key() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    let message = sql_expect_exception_message(
        "INSERT INTO public.edges (src, dst) VALUES (4294967296, 1)",
    );
    assert!(message.is_some_and(|msg| msg.contains("primary key out of range")));
}

/// Build rejects relational rows whose primary keys do not fit NodeKey encoding.
#[pg_test]
fn sql_build_rejects_out_of_range_primary_key() {
    seed_chain_relational();
    run_sql("INSERT INTO public.nodes (id) VALUES (4294967296)");
    run_sql("INSERT INTO public.edges (src, dst) VALUES (4294967296, 1)");
    assert!(!sql_build(0));
}

/// Deleting a base edge row logs remove-overlay but leaves the frozen artifact unchanged.
#[pg_test]
fn sync_base_edge_delete_does_not_tombstone_base() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("DELETE FROM public.edges WHERE src = 2 AND dst = 3");
    let (action_type, _, _) = latest_sync_log_row();
    assert_eq!(action_type, 5);
    assert!(sql_sync_reload() >= 1);
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Unknown node keys in the sync log cause reload to fail without applying rows.
#[pg_test]
fn sync_reload_rejects_unknown_node_key() {
    seed_chain_relational();
    assert!(sql_build(0));
    run_sql(
        "INSERT INTO graph._sync_log (sequence, action_type, arg0, arg1) \
         VALUES (1, 1, NULL, NULL)",
    );
    assert_eq!(sql_sync_reload(), 0);
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Loaded status JSON reports node and edge counts from the chain fixture.
#[pg_test]
fn sql_status_reports_chain_counts() {
    seed_chain_relational();
    assert!(sql_build(0));
    let status = sql_status();
    assert_eq!(
        status,
        "{\"loaded\":true,\"node_count\":3,\"edge_count\":2,\"read_only\":true,\
\"overlay_edge_count\":0,\"tombstoned_edges\":0,\"sync_overlay_edges\":0,\
\"sync_tombstoned_edges\":0,\"sync_tombstoned_nodes\":0}"
    );
}

/// Multi-seed inbound traverse reaches predecessors.
#[pg_test]
fn sql_multi_seed_inbound_traverse() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    let inbound = sql_traverse_seeds(&[2, 0], 10, "in");
    assert!(inbound.contains(&2));
    assert!(inbound.contains(&1));
    assert!(inbound.contains(&0));
}

/// Maintenance reloads persisted snapshot bytes when enabled.
#[pg_test]
fn sql_maintenance_reload() {
    seed_chain_relational();
    assert!(sql_build(0));
    run_sql("SET oxgraph.maintenance_enabled = true");
    assert!(sql_maintenance());
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Two SQL loads of the same snapshot yield identical traverse output.
#[pg_test]
fn sql_traverse_parity_after_reload() {
    sql_reset();
    let bytes = chain_snapshot_bytes();
    assert!(sql_load_snapshot(&bytes));
    let first = sql_traverse(0, 10, "out", -1);
    assert!(sql_load_snapshot(&bytes));
    let second = sql_traverse(0, 10, "out", -1);
    assert_eq!(first, second);
}

/// Session GUC `oxgraph.query_freshness` is registered and accepts `SET`.
#[pg_test]
fn guc_query_freshness() {
    run_sql("SET oxgraph.query_freshness = 0");
    let guc: i32 = Spi::get_one("SELECT current_setting('oxgraph.query_freshness')::int")
        .ok()
        .flatten()
        .unwrap_or(-1);
    assert_eq!(guc, 0);
}

/// Overlay edges are invisible until sync reload applies the log.
#[pg_test]
fn sync_insert_invisible_before_reload() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_log_count() >= 1);
    let before = sql_traverse(0, 10, "out", 1);
    assert_eq!(before, vec![0, 1], "overlay should not be visible before reload");
    assert!(sql_sync_reload() >= 1);
    let after = sql_traverse(0, 10, "out", 1);
    assert!(after.contains(&2), "overlay edge should be visible after reload");
}

/// Batch sync log rows apply in one reload.
#[pg_test]
fn sync_batch_reload_applies_all_rows() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    run_sql("INSERT INTO public.edges (src, dst) VALUES (3, 1)");
    assert!(sql_sync_log_count() >= 2);
    let applied = sql_sync_reload();
    assert!(applied >= 2);
    assert_eq!(
        sql_sync_health(),
        "{\"overlay_edges\":2,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}"
    );
}

/// Insert then delete the same overlay edge yields empty overlay after reload.
#[pg_test]
fn sync_reload_idempotent_net_zero() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    run_sql("DELETE FROM public.edges WHERE src = 1 AND dst = 3");
    assert!(sql_sync_reload() >= 2);
    let depth_one = sql_traverse(0, 10, "out", 1);
    assert_eq!(depth_one, vec![0, 1]);
    assert_eq!(sql_sync_health(), "{\"overlay_edges\":0,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}");
}

/// Overlay-aware freshness merges overlay edges into traverse output.
#[pg_test]
fn freshness_overlay_aware_sees_overlay() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_reload() >= 1);
    sql_set_query_freshness(1);
    let depth_one = sql_traverse(0, 10, "out", 1);
    assert!(depth_one.contains(&2));
}

/// Base-only freshness skips overlay edges while the engine is loaded.
#[pg_test]
fn freshness_base_only_hides_overlay() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_reload() >= 1);
    sql_set_query_freshness(0);
    let depth_one = sql_traverse(0, 10, "out", 1);
    assert_eq!(depth_one, vec![0, 1]);
}

/// Rebuild clears overlay state accumulated from sync reload.
#[pg_test]
fn rebuild_after_overlay_clears_overlay() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_reload() >= 1);
    assert_eq!(
        sql_sync_health(),
        "{\"overlay_edges\":1,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}"
    );
    run_sql("SET oxgraph.maintenance_enabled = true");
    assert!(sql_rebuild(0));
    let status = sql_status();
    assert!(status.contains("\"overlay_edge_count\":0"));
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Maintenance rebuilds persisted snapshot from relational rows and clears overlay.
#[pg_test]
fn maintenance_after_relational_change_updates_store() {
    seed_chain_relational();
    assert!(sql_build(0));
    let built_before = sql_snapshot_built_at();
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_reload() >= 1);
    assert_eq!(
        sql_sync_health(),
        "{\"overlay_edges\":1,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}"
    );
    run_sql("SET oxgraph.maintenance_enabled = true");
    assert!(sql_maintenance());
    let built_after = sql_snapshot_built_at();
    assert!(built_after >= built_before);
    assert_eq!(sql_sync_health(), "{\"overlay_edges\":0,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}");
    sql_set_query_freshness(0);
    let depth_one = sql_traverse(0, 10, "out", 1);
    assert!(
        depth_one.contains(&2),
        "maintenance should fold relational edge into base artifact"
    );
}

/// Rebuild is denied when maintenance is disabled.
#[pg_test]
fn maintenance_disabled_denies_rebuild() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_reload() >= 1);
    assert_eq!(
        sql_sync_health(),
        "{\"overlay_edges\":1,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}"
    );
    run_sql("SET oxgraph.maintenance_enabled = false");
    assert!(!sql_rebuild(0));
    assert_eq!(
        sql_sync_health(),
        "{\"overlay_edges\":1,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}"
    );
}

/// Reset clears the in-memory engine slot.
#[pg_test]
fn reset_clears_session_engine() {
    seed_chain_relational();
    assert!(sql_build(0));
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
    sql_reset();
    let status = sql_status();
    assert!(status.contains("\"loaded\":false"));
    assert!(sql_traverse_not_loaded(0));
}

/// Load persisted restores base graph after reset.
#[pg_test]
fn load_persisted_after_reset_restores_base() {
    seed_chain_relational();
    assert!(sql_build(0));
    sql_reset();
    assert!(sql_load_persisted());
    let status = sql_status();
    assert!(status.contains("\"loaded\":true"));
    assert_eq!(sql_sync_health(), "{\"overlay_edges\":0,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}");
    let out = sql_traverse(0, 10, "out", -1);
    assert_eq!(out, vec![0, 1, 2]);
}

/// Persisted snapshot does not carry overlay state from a prior session.
#[pg_test]
fn load_persisted_drops_overlay_from_prior_session() {
    seed_chain_relational();
    assert!(sql_build(0));
    assert!(sql_attach_sync_trigger(1));
    run_sql("INSERT INTO public.edges (src, dst) VALUES (1, 3)");
    assert!(sql_sync_reload() >= 1);
    sql_set_query_freshness(1);
    let with_overlay = sql_traverse(0, 10, "out", 1);
    assert!(with_overlay.contains(&2));
    sql_reset();
    assert!(sql_load_persisted());
    sql_set_query_freshness(1);
    let depth_one = sql_traverse(0, 10, "out", 1);
    assert_eq!(depth_one, vec![0, 1]);
    assert_eq!(sql_sync_health(), "{\"overlay_edges\":0,\"tombstoned_edges\":0,\"tombstoned_nodes\":0}");
}
