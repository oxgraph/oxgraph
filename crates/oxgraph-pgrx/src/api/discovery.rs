//! Discovery SQL for catalog registration.

use pgrx::prelude::*;

use crate::{report, spi};

/// Registers a node table and returns its assigned `table_id` (admin).
#[pg_extern(schema = "graph")]
fn graph_register_table(schema_name: &str, table_name: &str, primary_key_column: &str) -> i32 {
    report::ensure_admin_or_raise();
    match spi::register_table(schema_name, table_name, primary_key_column) {
        Ok(id) => i32::try_from(id.0).unwrap_or_else(|_| {
            report::raise(oxgraph_postgres::PostgresGraphError::Build(
                oxgraph_postgres::BuildError::Spi("table_id overflow".into()),
            ))
        }),
        Err(error) => report::raise(error),
    }
}

/// Registers an edge mapping table and returns its assigned `edge_id` (admin).
#[pg_extern(schema = "graph")]
fn graph_register_edge(
    source_table_id: i32,
    target_table_id: i32,
    source_column: &str,
    target_column: &str,
    schema_name: &str,
    table_name: &str,
) -> i32 {
    report::ensure_admin_or_raise();
    match spi::register_edge(
        source_table_id,
        target_table_id,
        source_column,
        target_column,
        schema_name,
        table_name,
    ) {
        Ok(id) => i32::try_from(id.0).unwrap_or_else(|_| {
            report::raise(oxgraph_postgres::PostgresGraphError::Build(
                oxgraph_postgres::BuildError::Spi("edge_id overflow".into()),
            ))
        }),
        Err(error) => report::raise(error),
    }
}

/// Registers a filter column on a node table (admin). Returns `true` on success.
#[pg_extern(schema = "graph")]
fn graph_register_filter_column(table_id: i32, column_name: &str) -> bool {
    report::ensure_admin_or_raise();
    match spi::register_filter_column(table_id, column_name) {
        Ok(()) => true,
        Err(error) => report::raise(error),
    }
}
