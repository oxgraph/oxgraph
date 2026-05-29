//! SPI-agnostic relational ingest into OXGTOPO snapshots.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use oxgraph_csr::build::{GraphBuilder, GraphNodeId, export_csr_snapshot};
use oxgraph_graph::OutgoingNeighborsGraph;

use crate::{
    artifact::{PostgresMetadata, attach_postgres_sections},
    catalog::{NodeKey, RegisteredEdge},
    error::{BuildError, PostgresGraphError},
};

/// Builds the dense node-id assignment used by [`DualTopologySnapshot::from_edge_rows`].
///
/// Keys are sorted in ascending [`NodeKey`] order; dense ids are `0..key_count`.
///
/// # Errors
///
/// Returns [`BuildError::NodeCountOverflow`] when the distinct key count does not fit in `u32`.
///
/// # Performance
///
/// This function is `O(n log n + m)` for `n` distinct keys and `m` edges.
pub fn dense_node_map_from_edges(edges: &[EdgeRow]) -> Result<BTreeMap<NodeKey, u32>, BuildError> {
    let mut keys = BTreeSet::new();
    for edge in edges {
        keys.insert(edge.source);
        keys.insert(edge.target);
    }
    let mut map = BTreeMap::new();
    for (index, key) in keys.into_iter().enumerate() {
        let dense = u32::try_from(index).map_err(|_| BuildError::NodeCountOverflow)?;
        map.insert(key, dense);
    }
    Ok(map)
}

/// Maps scanned SQL primary-key values to an [`EdgeRow`] for one registered edge mapping.
///
/// # Performance
///
/// This function is `O(1)`.
#[must_use]
pub const fn edge_row_from_scan(edge: &RegisteredEdge, source_pk: u64, target_pk: u64) -> EdgeRow {
    EdgeRow {
        source: NodeKey::registered(edge.source_table, source_pk),
        target: NodeKey::registered(edge.target_table, target_pk),
    }
}

/// One edge endpoint pair supplied by a caller scanning source tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeRow {
    /// Source node key.
    pub source: NodeKey,
    /// Target node key.
    pub target: NodeKey,
}

/// Forward CSR plus inbound CSC snapshot bytes with Postgres metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DualTopologySnapshot;

impl DualTopologySnapshot {
    /// Builds OXGTOPO bytes from scanned edge rows.
    ///
    /// Node indices are assigned in ascending [`NodeKey`] order. Catalog registration is
    /// validated separately via [`crate::Catalog::validate_for_build`] or
    /// [`crate::SnapshotRebuild::from_catalog_and_edges`].
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Build`] when edge rows are empty or graph construction fails.
    ///
    /// # Performance
    ///
    /// This function is `O(n log n + m)` where `n` is distinct nodes and `m` is edges.
    pub fn from_edge_rows(
        edges: &[EdgeRow],
        built_at_unix: u64,
    ) -> Result<Vec<u8>, PostgresGraphError> {
        if edges.is_empty() {
            return Err(BuildError::EmptyEdges.into());
        }

        let mut keys: BTreeSet<NodeKey> = BTreeSet::new();
        for edge in edges {
            keys.insert(edge.source);
            keys.insert(edge.target);
        }

        let mut builder = GraphBuilder::<u32, u32>::new();
        let mut key_to_node: BTreeMap<NodeKey, GraphNodeId<u32>> = BTreeMap::new();
        for key in &keys {
            let node = builder.add_node()?;
            key_to_node.insert(*key, node);
        }
        for edge in edges {
            let source = *key_to_node
                .get(&edge.source)
                .ok_or(BuildError::MissingNodeKey)?;
            let target = *key_to_node
                .get(&edge.target)
                .ok_or(BuildError::MissingNodeKey)?;
            builder.add_edge(source, target)?;
        }

        let frozen = builder.freeze()?;
        let node_count = u32::try_from(keys.len()).map_err(|_| BuildError::NodeCountOverflow)?;
        let edge_count =
            u32::try_from(frozen.edge_ids().len()).map_err(|_| BuildError::EdgeCountOverflow)?;

        let mut inbound_builder = GraphBuilder::<u32, u32>::new();
        for _ in 0..keys.len() {
            inbound_builder.add_node()?;
        }
        for source in 0..keys.len() {
            let source_id =
                GraphNodeId(u32::try_from(source).map_err(|_| BuildError::NodeCountOverflow)?);
            for target in frozen.outgoing_neighbors(source_id) {
                inbound_builder.add_edge(target, source_id)?;
            }
        }
        let inbound_frozen = inbound_builder.freeze()?;
        if inbound_frozen.edge_ids().len() != frozen.edge_ids().len() {
            return Err(BuildError::EdgeCountMismatch.into());
        }

        let forward_bytes = export_csr_snapshot(&frozen)?;
        let inbound_bytes = export_csr_snapshot(&inbound_frozen)?;
        let metadata =
            PostgresMetadata::new(node_count, edge_count, built_at_unix, true).with_reverse_index();
        attach_postgres_sections(&forward_bytes, Some(&inbound_bytes), &metadata)
    }

    /// Builds OXGTOPO bytes from dense `0..node_count-1` node ids (tests and benches).
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Build`] when `edges` is empty or construction fails.
    ///
    /// # Performance
    ///
    /// This function is `O(n + m)` for `n` nodes and `m` edges.
    pub fn from_dense_u32_edges(
        edges: &[(u32, u32)],
        built_at_unix: u64,
    ) -> Result<Vec<u8>, PostgresGraphError> {
        if edges.is_empty() {
            return Err(BuildError::EmptyEdges.into());
        }
        let max_index = edges
            .iter()
            .flat_map(|(source, target)| [*source, *target])
            .max()
            .unwrap_or(0);
        let node_count = max_index
            .checked_add(1)
            .ok_or(BuildError::NodeCountOverflow)?;
        Self::from_dense_u32_edges_with_node_count(node_count, edges, built_at_unix)
    }

    /// Builds OXGTOPO bytes with an explicit node count (tests with isolated vertices).
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Build`] when `node_count` is zero, an edge endpoint is out of
    /// range, or construction fails.
    ///
    /// # Performance
    ///
    /// This function is `O(n + m)` for `n` nodes and `m` edges.
    pub fn from_dense_u32_edges_with_node_count(
        node_count: u32,
        edges: &[(u32, u32)],
        built_at_unix: u64,
    ) -> Result<Vec<u8>, PostgresGraphError> {
        if node_count == 0 {
            return Err(BuildError::EmptyEdges.into());
        }
        let node_count_usize = node_count as usize;
        for &(source, target) in edges {
            if source >= node_count || target >= node_count {
                return Err(BuildError::MissingNodeKey.into());
            }
        }

        let mut forward_builder = GraphBuilder::<u32, u32>::new();
        for _ in 0..node_count_usize {
            forward_builder.add_node()?;
        }
        for &(source, target) in edges {
            forward_builder.add_edge(GraphNodeId(source), GraphNodeId(target))?;
        }
        let forward_frozen = forward_builder.freeze()?;
        let edge_count = u32::try_from(forward_frozen.edge_ids().len())
            .map_err(|_| BuildError::EdgeCountOverflow)?;

        let mut inbound_builder = GraphBuilder::<u32, u32>::new();
        for _ in 0..node_count_usize {
            inbound_builder.add_node()?;
        }
        for &(source, target) in edges {
            inbound_builder.add_edge(GraphNodeId(target), GraphNodeId(source))?;
        }
        let inbound_frozen = inbound_builder.freeze()?;
        if inbound_frozen.edge_ids().len() != forward_frozen.edge_ids().len() {
            return Err(BuildError::EdgeCountMismatch.into());
        }

        let forward_bytes = export_csr_snapshot(&forward_frozen)?;
        let inbound_bytes = export_csr_snapshot(&inbound_frozen)?;
        let metadata =
            PostgresMetadata::new(node_count, edge_count, built_at_unix, true).with_reverse_index();
        attach_postgres_sections(&forward_bytes, Some(&inbound_bytes), &metadata)
    }
}

/// Estimates node and edge counts for a build without exporting a snapshot.
///
/// # Performance
///
/// This function is `O(m)`.
#[must_use]
pub fn estimate_build(edges: &[EdgeRow]) -> BuildEstimate {
    let mut keys = BTreeSet::new();
    for edge in edges {
        keys.insert(edge.source);
        keys.insert(edge.target);
    }
    BuildEstimate {
        node_count: keys.len(),
        edge_count: edges.len(),
    }
}

/// Estimated build sizes for registration UIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildEstimate {
    /// Distinct node keys observed in scanned edge rows.
    pub node_count: usize,
    /// Edge row count.
    pub edge_count: usize,
}
