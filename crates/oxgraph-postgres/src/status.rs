//! JSON status payloads for admin and discovery surfaces.

use crate::{Engine, SyncHealth};

/// Engine status JSON payload for admin surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineStatusReport {
    /// Whether an engine is loaded in this backend.
    pub loaded: bool,
    /// Node count from artifact metadata.
    pub node_count: u32,
    /// Edge count from artifact metadata.
    pub edge_count: u32,
    /// Whether the artifact is read-only.
    pub read_only: bool,
    /// Overlay edge insertions currently buffered.
    pub overlay_edge_count: usize,
    /// Tombstoned base edges.
    pub tombstoned_edges: usize,
    /// Sync overlay edge count.
    pub sync_overlay_edges: usize,
    /// Sync tombstoned edge count.
    pub sync_tombstoned_edges: usize,
    /// Sync tombstoned node count.
    pub sync_tombstoned_nodes: usize,
}

impl EngineStatusReport {
    /// Builds a report for an unloaded session.
    #[must_use]
    pub const fn unloaded() -> Self {
        Self {
            loaded: false,
            node_count: 0,
            edge_count: 0,
            read_only: false,
            overlay_edge_count: 0,
            tombstoned_edges: 0,
            sync_overlay_edges: 0,
            sync_tombstoned_edges: 0,
            sync_tombstoned_nodes: 0,
        }
    }

    /// Builds a report from a loaded engine.
    #[must_use]
    pub fn from_engine(engine: &Engine) -> Self {
        let status = engine.status();
        let health = engine.sync_health();
        Self {
            loaded: true,
            node_count: status.node_count,
            edge_count: status.edge_count,
            read_only: status.read_only,
            overlay_edge_count: status.overlay_edge_count,
            tombstoned_edges: status.tombstoned_edges,
            sync_overlay_edges: health.overlay_edges,
            sync_tombstoned_edges: health.tombstoned_edges,
            sync_tombstoned_nodes: health.tombstoned_nodes,
        }
    }

    /// Serializes this report as compact JSON.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn to_json(&self) -> alloc::string::String {
        if !self.loaded {
            return "{\"loaded\":false}".into();
        }
        alloc::format!(
            "{{\"loaded\":true,\"node_count\":{},\"edge_count\":{},\"read_only\":{},\
\"overlay_edge_count\":{},\"tombstoned_edges\":{},\"sync_overlay_edges\":{},\
\"sync_tombstoned_edges\":{},\"sync_tombstoned_nodes\":{}}}",
            self.node_count,
            self.edge_count,
            self.read_only,
            self.overlay_edge_count,
            self.tombstoned_edges,
            self.sync_overlay_edges,
            self.sync_tombstoned_edges,
            self.sync_tombstoned_nodes
        )
    }
}

/// Sync health JSON payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncHealthReport {
    /// Overlay edge insertions currently buffered.
    pub overlay_edges: usize,
    /// Tombstoned base edges.
    pub tombstoned_edges: usize,
    /// Tombstoned nodes.
    pub tombstoned_nodes: usize,
}

impl From<SyncHealth> for SyncHealthReport {
    fn from(health: SyncHealth) -> Self {
        Self {
            overlay_edges: health.overlay_edges,
            tombstoned_edges: health.tombstoned_edges,
            tombstoned_nodes: health.tombstoned_nodes,
        }
    }
}

impl SyncHealthReport {
    /// Serializes this report as compact JSON.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn to_json(&self) -> alloc::string::String {
        alloc::format!(
            "{{\"overlay_edges\":{},\"tombstoned_edges\":{},\"tombstoned_nodes\":{}}}",
            self.overlay_edges,
            self.tombstoned_edges,
            self.tombstoned_nodes
        )
    }
}
