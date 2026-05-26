//! Typed status payloads serialized at the SQL boundary.

use oxgraph_postgres::{EngineStatusReport, SyncHealthReport};
use pgrx::prelude::*;

use crate::session::{SessionEngine, log_error};

/// Returns status JSON for the active in-memory engine.
#[pg_extern(schema = "graph")]
fn graph_status() -> String {
    match SessionEngine::try_with(|engine| Ok(EngineStatusReport::from_engine(engine))) {
        Ok(status) => status.to_json(),
        Err(error) => {
            log_error(&error);
            EngineStatusReport::unloaded().to_json()
        }
    }
}

/// Returns sync overlay health JSON for the active engine.
#[pg_extern(schema = "graph")]
fn graph_sync_health() -> String {
    match SessionEngine::try_with(|engine| Ok(SyncHealthReport::from(engine.sync_health()))) {
        Ok(health) => health.to_json(),
        Err(error) => {
            log_error(&error);
            SyncHealthReport {
                overlay_edges: 0,
                tombstoned_edges: 0,
                tombstoned_nodes: 0,
            }
            .to_json()
        }
    }
}
