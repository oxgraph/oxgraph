//! Catalog-validated snapshot rebuild orchestration.

use alloc::vec::Vec;

use crate::{
    build::{DualTopologySnapshot, EdgeRow},
    catalog::Catalog,
    error::PostgresGraphError,
};

/// Orchestrates relational snapshot rebuild from registration metadata and edge rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotRebuild;

impl SnapshotRebuild {
    /// Validates catalog registration, then exports dual-topology OXGTOPO bytes.
    ///
    /// Topology is derived from `edges` only; the catalog guards non-empty registration.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Catalog`] or [`PostgresGraphError::Build`] on failure.
    ///
    /// # Performance
    ///
    /// This function is `O(n log n + m)` where `n` is distinct nodes and `m` is edge rows.
    pub fn from_catalog_and_edges(
        catalog: &Catalog,
        edges: &[EdgeRow],
        built_at_unix: u64,
    ) -> Result<Vec<u8>, PostgresGraphError> {
        catalog.validate_for_build()?;
        DualTopologySnapshot::from_edge_rows(edges, built_at_unix)
    }
}
