//! SQL admin and maintenance entrypoints.

use oxgraph_postgres::GraphRole;
use pgrx::prelude::*;

use crate::{
    acl, jobs,
    session::{SessionEngine, log_error},
    spi,
};

/// Clears the active engine (admin).
#[pg_extern(schema = "graph")]
fn graph_reset() -> bool {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return false;
    }
    SessionEngine::reset();
    true
}

/// Loads snapshot bytes into the active engine (admin).
#[pg_extern(schema = "graph")]
fn graph_load_snapshot(bytes: &[u8]) -> bool {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return false;
    }
    match SessionEngine::load(bytes) {
        Ok(()) => true,
        Err(error) => {
            log_error(&error);
            false
        }
    }
}

/// Loads the persisted snapshot from `graph._snapshot_store` (admin).
#[pg_extern(schema = "graph")]
fn graph_load_persisted() -> bool {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return false;
    }
    let bytes = match spi::load_persisted_snapshot_bytes() {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return false,
        Err(error) => {
            log_error(&error);
            return false;
        }
    };
    match SessionEngine::load(&bytes) {
        Ok(()) => true,
        Err(error) => {
            log_error(&error);
            false
        }
    }
}

/// Builds a snapshot from registered catalog tables and loads the engine (admin).
#[pg_extern(schema = "graph")]
fn graph_build(built_at_unix: default!(i64, 0)) -> bool {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return false;
    }
    let built_at = u64::try_from(built_at_unix).unwrap_or(0);
    let bytes = match spi::rebuild_and_persist_snapshot(built_at) {
        Ok(bytes) => bytes,
        Err(error) => {
            log_error(&error);
            return false;
        }
    };
    match SessionEngine::load(&bytes) {
        Ok(()) => true,
        Err(error) => {
            log_error(&error);
            false
        }
    }
}

/// Rebuilds the active engine artifact from catalog scans (admin).
#[pg_extern(schema = "graph")]
fn graph_rebuild(built_at_unix: default!(i64, 0)) -> bool {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return false;
    }
    let catalog = match spi::load_catalog_from_spi() {
        Ok(catalog) => catalog,
        Err(error) => {
            log_error(&error);
            return false;
        }
    };
    let edges = match spi::scan_edge_rows(&catalog) {
        Ok(edges) => edges,
        Err(error) => {
            log_error(&error);
            return false;
        }
    };
    let built_at = u64::try_from(built_at_unix).unwrap_or(0);
    match SessionEngine::try_with_mut(|engine| {
        engine.rebuild_from_catalog(&catalog, &edges, built_at)
    }) {
        Ok(()) => match SessionEngine::try_with(|engine| Ok(engine.snapshot_bytes().to_vec())) {
            Ok(bytes) => match spi::persist_snapshot_bytes(&bytes, built_at_unix) {
                Ok(()) => true,
                Err(error) => {
                    log_error(&error);
                    false
                }
            },
            Err(error) => {
                log_error(&error);
                false
            }
        },
        Err(error) => {
            log_error(&error);
            false
        }
    }
}

/// Replays durable sync rows into the active engine overlay.
#[pg_extern(schema = "graph")]
fn graph_sync_reload() -> i32 {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return 0;
    }
    let rows = match spi::load_sync_rows_from_spi() {
        Ok(rows) => rows,
        Err(error) => {
            log_error(&error);
            return 0;
        }
    };
    match SessionEngine::try_with_mut(|engine| engine.apply_sync_rows(&rows)) {
        Ok(count) => i32::try_from(count).unwrap_or(i32::MAX),
        Err(error) => {
            log_error(&error);
            0
        }
    }
}

/// Runs one maintenance pass synchronously (admin).
#[pg_extern(schema = "graph")]
fn graph_maintenance() -> bool {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return false;
    }
    jobs::schedule_maintenance();
    match spi::load_persisted_snapshot_bytes() {
        Ok(Some(bytes)) => match SessionEngine::load(&bytes) {
            Ok(()) => true,
            Err(error) => {
                log_error(&error);
                false
            }
        },
        Ok(None) => false,
        Err(error) => {
            log_error(&error);
            false
        }
    }
}

/// Attaches the generic edge sync trigger to a registered edge table (admin).
#[pg_extern(schema = "graph")]
fn graph_attach_sync_trigger(edge_id: i32) -> bool {
    if acl::ensure_role(GraphRole::Admin).is_err() {
        return false;
    }
    let catalog = match spi::load_catalog_from_spi() {
        Ok(catalog) => catalog,
        Err(error) => {
            log_error(&error);
            return false;
        }
    };
    let Some(edge) = catalog.edges.iter().find(|e| e.id.0 == edge_id as u32) else {
        return false;
    };
    if !spi::sql_ident_public(&edge.schema)
        || !spi::sql_ident_public(&edge.name)
        || !spi::sql_ident_public(&edge.source_column)
        || !spi::sql_ident_public(&edge.target_column)
    {
        return false;
    }
    let trigger_name = format!("oxgraph_sync_edge_{edge_id}");
    let sql = format!(
        "DROP TRIGGER IF EXISTS \"{trigger_name}\" ON \"{}\".\"{}\"; \
         CREATE TRIGGER \"{trigger_name}\" \
         AFTER INSERT OR DELETE ON \"{}\".\"{}\" \
         FOR EACH ROW EXECUTE FUNCTION graph._edge_change_sync_trigger('{}', '{}', '{}', '{}');",
        edge.schema,
        edge.name,
        edge.schema,
        edge.name,
        edge.source_table.0,
        edge.target_table.0,
        edge.source_column,
        edge.target_column
    );
    Spi::run(&sql).is_ok()
}
