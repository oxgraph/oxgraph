//! Postgres-backed [`OxGraph`] engine library.
//!
//! Owns catalog modeling, relational build into OXGTOPO snapshots, artifact I/O,
//! active engine state, overlays, sync replay, and query algorithms. SPI, triggers,
//! and SQL facades live in `oxgraph-pgrx`.
#![cfg(feature = "std")]

extern crate alloc;

mod artifact;
mod build;
mod builder;
mod catalog;
mod config;
mod engine;
mod error;
mod overlay;
mod rebuild;
mod role;
mod search;
mod status;
mod sync;
mod topology;
mod traverse;

pub use artifact::{
    PostgresMetadata, PostgresSectionError, SNAPSHOT_KIND_PG_CATALOG, SNAPSHOT_KIND_PG_METADATA,
    attach_metadata, attach_postgres_sections, load_snapshot_bytes, read_metadata,
    validate_snapshot_bytes, write_snapshot_bytes,
};
pub use build::{
    BuildEstimate, DualTopologySnapshot, EdgeRow, dense_node_map_from_edges, edge_row_from_scan,
    estimate_build,
};
pub use builder::EngineBuilder;
pub use catalog::{
    Catalog, CatalogError, EdgeId, FilterColumn, NodeKey, RegisteredEdge, RegisteredTable, TableId,
    edge_id_from_i32, table_id_from_i32, validate_primary_key, validate_sql_ident,
};
pub use config::{Config, QueryFreshness};
pub use engine::{Engine, EngineStatus};
pub use error::{BuildError, ConfigError, PostgresGraphError, QueryError, SyncError};
pub use overlay::{OverlayEdge, OverlayState};
pub use oxgraph_csc::{
    CscNodeId, CscSnapshotError, CscSnapshotGraph, SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
    SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
};
pub use rebuild::SnapshotRebuild;
pub use role::GraphRole;
pub use search::SearchPredicate;
pub use status::{EngineStatusReport, SyncHealthReport};
pub use sync::{
    RawSyncRow, SyncAction, SyncActionCodec, SyncActionWire, SyncHealth, SyncRow,
    dense_node_map_for_sync_resolution, resolve_sync_action, resolve_sync_rows,
};
pub use topology::{ForwardCsr, GraphTopology, InboundCsc};
pub use traverse::{TraversalDirection, TraverseLimits};

#[cfg(feature = "bench-fixture")]
pub mod bench_fixture;

#[cfg(kani)]
mod proofs;
